//! 配方编辑器(spec #95 工单 #113):数据仓写侧「整文件替换写回」的消费方
//! (ADR-0011 §4 表格里 #113 那一行)。
//!
//! 选中数据仓里的一个 `recipe.yaml` → 面板把它解析成**结构化中间形状**
//! ([`RecipeDoc`])→ 图形化加/删组件槽、改 env 声明(required 及默认)→ 序列化
//! 回 yaml(**面板不认识的自定义字段原样保留**,硬约束)→ 写回前经本模块把编辑态
//! 暂存成 scratch 文件喂给引擎 `validate`(引擎代跑,argv 与健康屏同源)预跑 →
//! 拟落账摊开 + 图形确认 → 经 [`write::DataRepo::commit`] 原子落盘(tmp+rename)。
//! 确认前 `recipe.yaml` 一个字节都不落。
//!
//! ## 为什么不引 serde_yaml
//! Cargo.toml 现无 serde_yaml;为一个「只认配方子集、又要往返保真」的需求引一条
//! yaml 依赖链不值(同 #112 自实现 sha256 而不引 sha2 的先例)。本模块手写一个
//! **块状 YAML 往返**模型:只解析面板要编辑的两条序列(`components:` / `env:`),
//! 其余顶层块**原样保留**(逐行),序列项里的未知子字段也以「键 + 原样值」保藏。
//! 目标是「往返不丢未知字段」,不是通用 YAML 引擎。
//!
//! ## 预跑暂存文件的落点(ADR-0011 §1 第四类:窄准许面)
//! 引擎 `validate --recipe <路径>` 只读文件、无 stdin,且从配方路径
//! `filepath.Dir(filepath.Dir(abs))` 自推仓根(上游 `internal/validate`)。要让
//! 跨文件检查(catalog.json / 组件引用)真跑,暂存文件必须在配方的**同目录**;
//! 于是暂存到 `recipe.yaml` 的兄弟隐藏文件,validate 完即删。暂存写经写侧
//! `plan`+`commit`(原子、走同一条缝),删除是 scratch 清理;被写回的
//! `recipe.yaml` 本身仍只在图形确认后落。
//!
//! 这是**确认前的一次写盘**,ADR-0011 §1 为它单开一行窄准许面(隐藏名、确定名、
//! 跑完即清、返航 / 换根也清),四条约束由
//! [`tests::暂存档_正常失败返航三径都清干净`] 与
//! [`tests::暂存档_换根也按当初那个根清`] 钉死。落点由 [`ScratchSlot`] 持
//! (它记住暂存时那个根),壳层连「根 + 相对路径」都不拼。

use crate::datarepo::write::{self, DataRepo, PlannedWrite, RepoRelPath, WritePlan, WriteReceipt};

/// 配方加载 / 解析失败(面板侧就拒,留配方屏;人话供屏上直接摆)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecipeError {
    /// 配方相对路径不是合法的数据仓相对路径。
    BadPath(write::RepoRelPathError),
    /// 读配方文件失败(不在 / 权限 / IO)。
    Read(String, String),
    /// 解析失败(不是本模块认识的块状 YAML)。
    Parse(String),
}

impl RecipeError {
    pub fn why(&self) -> String {
        match self {
            Self::BadPath(e) => format!("配方路径不是数据仓根之下的相对路径:{}", e.why()),
            Self::Read(p, r) => format!("读配方 `{p}` 失败:{r}"),
            Self::Parse(r) => format!("配方解析不了:{r}"),
        }
    }
}

// ---- 结构化中间形状:块状 YAML 往返模型 ----

/// 一个字段的值:内联标量(去掉首尾空白后的原文)或一个嵌套块(原样行,
/// 保留其原始缩进)。未知子字段两种都能保藏。
#[derive(Clone, Debug, PartialEq, Eq)]
enum FieldValue {
    Scalar(String),
    Block(Vec<String>),
}

/// 序列项里的一个字段(键 + 值),顺序即原顺序。
#[derive(Clone, Debug, PartialEq, Eq)]
struct ItemField {
    key: String,
    value: FieldValue,
}

/// 块序列里的一个项(`- ` 起头)。第一个字段在 dash 行上,其余字段缩进对齐。
#[derive(Clone, Debug, PartialEq, Eq)]
struct SeqItem {
    fields: Vec<ItemField>,
}

impl SeqItem {
    fn scalar(&self, key: &str) -> Option<&str> {
        self.fields.iter().find_map(|f| {
            if f.key == key {
                if let FieldValue::Scalar(v) = &f.value {
                    return Some(v.as_str());
                }
            }
            None
        })
    }

    fn has_key(&self, key: &str) -> bool {
        self.fields.iter().any(|f| f.key == key)
    }

    /// 设一个标量字段:存在(标量)则改值,不存在则追加到末尾。
    fn set_scalar(&mut self, key: &str, value: &str) {
        if let Some(f) = self.fields.iter_mut().find(|f| f.key == key) {
            f.value = FieldValue::Scalar(value.to_string());
            return;
        }
        self.fields.push(ItemField {
            key: key.to_string(),
            value: FieldValue::Scalar(value.to_string()),
        });
    }

    /// 删一个字段(清空 env 默认值时用)。
    fn remove_key(&mut self, key: &str) {
        self.fields.retain(|f| f.key != key);
    }
}

/// 一条块序列(`components:` / `env:`):头行 + 项 + 尾部(空行/注释,原样)。
#[derive(Clone, Debug, PartialEq, Eq)]
struct SeqBlock {
    header: String,
    dash_indent: usize,
    items: Vec<SeqItem>,
    trailer: Vec<String>,
}

/// 文档的一个顶层节。`components` / `env` 解析成可编辑序列,其余**原样保留**。
#[derive(Clone, Debug, PartialEq, Eq)]
enum Section {
    /// 原样行(一个非编辑的顶层键块 / 前导内容),逐行保藏。
    Raw(Vec<String>),
    Components(SeqBlock),
    Env(SeqBlock),
}

/// 解析后的配方文档:往返保真(未知字段不丢),可结构化编辑两条序列。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeDoc {
    sections: Vec<Section>,
}

impl RecipeDoc {
    /// 从 yaml 文本解析。
    pub fn parse(text: &str) -> Result<Self, RecipeError> {
        let lines: Vec<String> = text.split('\n').map(|s| s.to_string()).collect();
        let mut sections: Vec<Section> = Vec::new();
        let mut i = 0;
        // 前导内容(第一个顶层键之前的注释 / 空行 / 文档标记)。
        let start = i;
        while i < lines.len() && !is_top_key(&lines[i]) {
            i += 1;
        }
        if i > start {
            sections.push(Section::Raw(lines[start..i].to_vec()));
        }
        // 逐顶层键分块。
        while i < lines.len() {
            let block_start = i;
            let key = top_key_name(&lines[i]);
            i += 1;
            while i < lines.len() && !is_top_key(&lines[i]) {
                i += 1;
            }
            let block = &lines[block_start..i];
            let header = block[0].clone();
            let body = &block[1..];
            if (key == "components" || key == "env") && is_block_header(&header) {
                match parse_seq_block(&header, body) {
                    Some(seq) => sections.push(if key == "components" {
                        Section::Components(seq)
                    } else {
                        Section::Env(seq)
                    }),
                    // 不是干净的块序列(内联 / 怪异缩排):原样保留,不冒改。
                    None => sections.push(Section::Raw(block.to_vec())),
                }
            } else {
                sections.push(Section::Raw(block.to_vec()));
            }
        }
        Ok(Self { sections })
    }

    /// 序列化回 yaml(逐行拼,保留原始换行结构;未知字段原样在场)。
    pub fn serialize(&self) -> String {
        let mut out: Vec<String> = Vec::new();
        for section in &self.sections {
            match section {
                Section::Raw(ls) => out.extend(ls.iter().cloned()),
                Section::Components(seq) => serialize_seq(seq, &mut out),
                Section::Env(seq) => serialize_seq(seq, &mut out),
            }
        }
        out.join("\n")
    }

    fn seq_block(&self, which: SeqWhich) -> Option<&SeqBlock> {
        self.sections.iter().find_map(|s| match (which, s) {
            (SeqWhich::Components, Section::Components(b)) => Some(b),
            (SeqWhich::Env, Section::Env(b)) => Some(b),
            _ => None,
        })
    }

    fn seq_block_mut(&mut self, which: SeqWhich) -> Option<&mut SeqBlock> {
        self.sections.iter_mut().find_map(|s| match (which, s) {
            (SeqWhich::Components, Section::Components(b)) => Some(b),
            (SeqWhich::Env, Section::Env(b)) => Some(b),
            _ => None,
        })
    }

    /// 保证某条序列节存在(没有则建一个空的,挂在末尾)——加第一项时用。
    fn ensure_seq(&mut self, which: SeqWhich) -> &mut SeqBlock {
        let pos = self.sections.iter().position(|s| {
            matches!(s, Section::Components(_) if which == SeqWhich::Components)
                || matches!(s, Section::Env(_) if which == SeqWhich::Env)
        });
        if let Some(p) = pos {
            return match (&mut self.sections[p], which) {
                (Section::Components(b), SeqWhich::Components) => b,
                (Section::Env(b), SeqWhich::Env) => b,
                _ => unreachable!(),
            };
        }
        let header = match which {
            SeqWhich::Components => "components:",
            SeqWhich::Env => "env:",
        };
        let seq = SeqBlock {
            header: header.to_string(),
            dash_indent: 2,
            items: Vec::new(),
            trailer: Vec::new(),
        };
        let section = match which {
            SeqWhich::Components => Section::Components(seq),
            SeqWhich::Env => Section::Env(seq),
        };
        self.sections.push(section);
        match which {
            SeqWhich::Components => match self.sections.last_mut().unwrap() {
                Section::Components(b) => b,
                _ => unreachable!(),
            },
            SeqWhich::Env => match self.sections.last_mut().unwrap() {
                Section::Env(b) => b,
                _ => unreachable!(),
            },
        }
    }

    // ---- 组件槽(spec #113 用户故事 8) ----

    /// 组件槽的只读视图(屏上显示用)。
    pub fn component_views(&self) -> Vec<ComponentView> {
        self.seq_block(SeqWhich::Components)
            .map(|b| {
                b.items
                    .iter()
                    .map(|it| ComponentView {
                        name: it.scalar("component").unwrap_or("").to_string(),
                        version: it.scalar("version").unwrap_or("").to_string(),
                        source: it.scalar("source").unwrap_or("").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 加一个组件槽(本地:component + version;version 是起步占位,作者可在
    /// 数据仓里再调——面板 V1 不编辑组件版本)。
    pub fn add_component(&mut self, name: &str, version: &str) {
        let item = SeqItem {
            fields: vec![
                ItemField {
                    key: "component".into(),
                    value: FieldValue::Scalar(name.to_string()),
                },
                ItemField {
                    key: "version".into(),
                    value: FieldValue::Scalar(version.to_string()),
                },
            ],
        };
        self.ensure_seq(SeqWhich::Components).items.push(item);
    }

    /// 删一个组件槽(下标越界则无动作)。
    pub fn remove_component(&mut self, index: usize) {
        if let Some(b) = self.seq_block_mut(SeqWhich::Components) {
            if index < b.items.len() {
                b.items.remove(index);
            }
        }
    }

    // ---- env 声明(spec #113 用户故事 9) ----

    /// env 声明的只读视图。
    pub fn env_views(&self) -> Vec<EnvView> {
        self.seq_block(SeqWhich::Env)
            .map(|b| {
                b.items
                    .iter()
                    .map(|it| EnvView {
                        name: it.scalar("name").unwrap_or("").to_string(),
                        required: it.scalar("required") == Some("true"),
                        default: it.scalar("default").map(|s| s.to_string()),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 加一条 env 声明(name + required:false)。
    pub fn add_env(&mut self, name: &str) {
        let item = SeqItem {
            fields: vec![
                ItemField {
                    key: "name".into(),
                    value: FieldValue::Scalar(name.to_string()),
                },
                ItemField {
                    key: "required".into(),
                    value: FieldValue::Scalar("false".to_string()),
                },
            ],
        };
        self.ensure_seq(SeqWhich::Env).items.push(item);
    }

    /// 删一条 env 声明(下标越界则无动作)。
    pub fn remove_env(&mut self, index: usize) {
        if let Some(b) = self.seq_block_mut(SeqWhich::Env) {
            if index < b.items.len() {
                b.items.remove(index);
            }
        }
    }

    /// 改 env 声明的 required(设 `required: true|false`;缺字段则补)。
    pub fn set_env_required(&mut self, index: usize, required: bool) {
        if let Some(b) = self.seq_block_mut(SeqWhich::Env) {
            if let Some(it) = b.items.get_mut(index) {
                it.set_scalar("required", if required { "true" } else { "false" });
            }
        }
    }

    /// 改 env 声明的默认值(设 `default: <值>`;空值则删掉该字段——
    /// 「没有默认」不写成 `default:` 空)。
    pub fn set_env_default(&mut self, index: usize, default: &str) {
        if let Some(b) = self.seq_block_mut(SeqWhich::Env) {
            if let Some(it) = b.items.get_mut(index) {
                if default.trim().is_empty() {
                    it.remove_key("default");
                } else {
                    it.set_scalar("default", default.trim());
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SeqWhich {
    Components,
    Env,
}

/// 组件槽只读视图(屏上显示)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentView {
    pub name: String,
    pub version: String,
    pub source: String,
}

/// env 声明只读视图(屏上显示)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvView {
    pub name: String,
    pub required: bool,
    pub default: Option<String>,
}

// ---- 块状 YAML 的底层解析 / 序列化助手 ----

/// 顶层键行:无前导空白、非空、非注释、且 `:` 前是一段无空白的键。
fn is_top_key(line: &str) -> bool {
    if line.is_empty() || line.starts_with(' ') || line.starts_with('\t') || line.starts_with('#') {
        return false;
    }
    match line.find(':') {
        Some(0) | None => false,
        Some(idx) => !line[..idx].contains(' '),
    }
}

fn top_key_name(line: &str) -> String {
    match line.find(':') {
        Some(idx) => line[..idx].trim().to_string(),
        None => line.trim().to_string(),
    }
}

/// 块状头(`key:` 后面为空或是注释)——不是内联值(`key: [...]`)。
fn is_block_header(header: &str) -> bool {
    match header.find(':') {
        Some(idx) => {
            let rest = header[idx + 1..].trim_start();
            rest.is_empty() || rest.starts_with('#')
        }
        None => false,
    }
}

fn indent_of(l: &str) -> usize {
    l.len() - l.trim_start().len()
}

fn is_dash_line(l: &str) -> bool {
    let t = l.trim_start();
    t.starts_with("- ") || t == "-"
}

/// 解析一条块序列的体(头行之后、下一个顶层键之前的行)。
fn parse_seq_block(header: &str, body: &[String]) -> Option<SeqBlock> {
    let first = body.iter().position(|l| is_dash_line(l));
    let dash_pos = match first {
        None => {
            // 没有项:体全是空行才算「可编辑的空序列」,否则原样保留。
            if body.iter().all(|l| l.trim().is_empty()) {
                return Some(SeqBlock {
                    header: header.to_string(),
                    dash_indent: 2,
                    items: Vec::new(),
                    trailer: body.to_vec(),
                });
            }
            return None;
        }
        Some(f) => f,
    };
    let dash_indent = indent_of(&body[dash_pos]);
    let mut trailer: Vec<String> = body[..dash_pos].to_vec();
    let mut items: Vec<SeqItem> = Vec::new();
    let mut idx = dash_pos;
    while idx < body.len() {
        if !(is_dash_line(&body[idx]) && indent_of(&body[idx]) == dash_indent) {
            // 非项行(空行 / 注释 / 缩进怪异):收进尾部,原样保藏。
            trailer.push(body[idx].clone());
            idx += 1;
            continue;
        }
        let start = idx;
        idx += 1;
        while idx < body.len() {
            let l = &body[idx];
            if is_dash_line(l) && indent_of(l) == dash_indent {
                break;
            }
            if !l.trim().is_empty() && indent_of(l) < dash_indent {
                break; //  dedent 出序列
            }
            idx += 1;
        }
        items.push(parse_item(dash_indent, &body[start..idx]));
    }
    Some(SeqBlock {
        header: header.to_string(),
        dash_indent,
        items,
        trailer,
    })
}

/// 解析一个序列项:dash 行的 `- ` 换成两空格,令首字段与其余字段同列,再统一拆字段。
fn parse_item(dash_indent: usize, lines: &[String]) -> SeqItem {
    let field_indent = dash_indent + 2;
    let mut norm: Vec<String> = Vec::with_capacity(lines.len());
    let dash = &lines[0];
    let after = dash[dash_indent..]
        .strip_prefix("- ")
        .or_else(|| dash[dash_indent..].strip_prefix('-'))
        .unwrap_or("");
    norm.push(format!("{}{}", " ".repeat(field_indent), after));
    for l in &lines[1..] {
        norm.push(l.clone());
    }
    SeqItem {
        fields: parse_fields(&norm, field_indent),
    }
}

/// 从归一化行(all at indent >= field_indent)拆字段。
fn parse_fields(norm: &[String], field_indent: usize) -> Vec<ItemField> {
    let mut fields: Vec<ItemField> = Vec::new();
    let mut i = 0;
    while i < norm.len() {
        let line = &norm[i];
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        let cut = field_indent.min(line.len());
        let content = &line[cut..];
        let (key, inline) = match content.find(':') {
            Some(ci) => (
                content[..ci].trim().to_string(),
                content[ci + 1..].trim().to_string(),
            ),
            None => (content.trim().to_string(), String::new()),
        };
        if inline.is_empty() {
            // 块值:收集缩进更深的行(原样)。
            let mut block: Vec<String> = Vec::new();
            i += 1;
            while i < norm.len() {
                let l = &norm[i];
                if l.trim().is_empty() || indent_of(l) > field_indent {
                    block.push(l.clone());
                    i += 1;
                } else {
                    break;
                }
            }
            // 去掉块尾的空行(那是项间/序列尾的空,归 trailer 更保真)。
            while block.last().map(|l| l.trim().is_empty()).unwrap_or(false) {
                block.pop();
            }
            fields.push(ItemField {
                key,
                value: FieldValue::Block(block),
            });
        } else {
            fields.push(ItemField {
                key,
                value: FieldValue::Scalar(inline),
            });
            i += 1;
        }
    }
    fields
}

fn serialize_seq(seq: &SeqBlock, out: &mut Vec<String>) {
    out.push(seq.header.clone());
    for item in &seq.items {
        serialize_item(seq.dash_indent, item, out);
    }
    out.extend(seq.trailer.iter().cloned());
}

fn serialize_item(dash_indent: usize, item: &SeqItem, out: &mut Vec<String>) {
    let field_indent = dash_indent + 2;
    for (fi, field) in item.fields.iter().enumerate() {
        let prefix = if fi == 0 {
            format!("{}- ", " ".repeat(dash_indent))
        } else {
            " ".repeat(field_indent)
        };
        match &field.value {
            FieldValue::Scalar(v) => out.push(format!("{}{}: {}", prefix, field.key, v)),
            FieldValue::Block(block) => {
                out.push(format!("{}{}:", prefix, field.key));
                out.extend(block.iter().cloned());
            }
        }
    }
}

// ---- 数据仓读写侧集成:加载 / 拟落账 / 预跑暂存 ----

/// 从数据仓读一个配方文件并解析(路径知识留在接缝里,屏幕层零拼接)。
pub fn load(repo: &DataRepo, rel: &str) -> Result<RecipeDoc, RecipeError> {
    let rel_path = RepoRelPath::new(rel).map_err(RecipeError::BadPath)?;
    let path = repo.root_path().join(rel_path.as_str());
    let text = std::fs::read_to_string(&path)
        .map_err(|e| RecipeError::Read(rel.to_string(), format!("{e}")))?;
    RecipeDoc::parse(&text)
}

/// 拟落账:把编辑态序列化,过写前校验,得将写 `recipe.yaml` 的 [`WritePlan`]
/// (**一个字节都不落**)。目标 = 配方文件本身(整文件替换)。
pub fn plan_writeback(
    repo: &DataRepo,
    doc: &RecipeDoc,
    rel: &str,
) -> Result<WritePlan, write::WritePlanError> {
    let rel_path = RepoRelPath::new(rel).map_err(write::WritePlanError::BadPath)?;
    let content = doc.serialize();
    repo.plan(vec![PlannedWrite::text(rel_path, &content)])
}

/// 预跑暂存文件的相对路径:配方文件的兄弟隐藏名(同目录 ⇒ 引擎自推同一仓根)。
///
/// 为什么必须在同目录:引擎 `validate --recipe <路径>` 从配方路径
/// `filepath.Dir(filepath.Dir(abs))` 自推仓根(上游 `internal/validate`),
/// 跨文件检查(catalog.json / 组件引用)要的是**同一个仓根**——挪出数据仓
/// 这些检查就失效,那预跑等于只跑了个形态面(见 ADR-0011 §1 第四类)。
pub fn scratch_rel(recipe_rel: &str) -> Result<RepoRelPath, write::RepoRelPathError> {
    let raw = match recipe_rel.rsplit_once('/') {
        Some((dir, _)) => format!("{dir}/{SCRATCH_FILE}"),
        None => SCRATCH_FILE.to_string(),
    };
    RepoRelPath::new(&raw)
}

/// 预跑暂存文件名(隐藏):既是 scratch 的名字,也是引擎回音路由的判据——
/// 配方编辑器的预跑 validate argv 指向它,据此与健康屏的 validate 回音分开。
pub const SCRATCH_FILE: &str = ".nwt-validate.yaml";

/// 这条引擎 argv 是不是配方编辑器的预跑 validate(argv 里**有** scratch
/// 暂存文件的路径)。健康屏的 validate 指向真实配方路径,不含本标记,于是
/// 两者回音不混。
///
/// 判据扫**整条 argv**、不钉末位:预跑 argv 与健康屏同源,而 #92 起两态都
/// 显式带 `--repo`——scratch 路径后面还跟着 `--repo <数据仓根>` 两元素。
/// 若仍按「末位」判,预跑回音会被误路由到健康屏:编辑器停在「预跑中」
/// 永远等不到回音,而健康屏的 hl_result 被scratch 回音污染。
pub fn is_prerun_argv(args: &[String]) -> bool {
    args.iter().any(|p| p.contains(SCRATCH_FILE))
}

/// 把编辑态序列化,经写侧 `plan`+`commit` 原子暂存到 scratch(走同一条缝,
/// 供引擎 validate 读;这不是对 `recipe.yaml` 的写回)。
pub fn stage_for_validate(
    repo: &DataRepo,
    doc: &RecipeDoc,
    scratch_rel: &RepoRelPath,
) -> Result<(), String> {
    let content = doc.serialize();
    let plan = repo
        .plan(vec![PlannedWrite::text(scratch_rel.clone(), &content)])
        .map_err(|e| e.why())?;
    repo.commit(&plan).map_err(|e| e.why())?;
    Ok(())
}

/// 清掉预跑暂存文件(scratch 清理,不是写回)。
pub fn clear_staged(repo: &DataRepo, scratch_rel: &RepoRelPath) {
    let _ = std::fs::remove_file(repo.abs_path(scratch_rel));
}

/// 预跑暂存档的落点:**哪个数据仓根** + 哪个相对路径,以及「清掉它」这一动作。
///
/// 记住**暂存时那个句柄**而不是「当前设置里的根」:操作者中途换过数据仓根,
/// 残档也要从旧根清掉——否则「确认前一个字节都不落」在换根这一刻漏一个洞
/// (ADR-0011 §1 第四类:跑完即清、返航 / 换根也清)。
#[derive(Clone, Debug)]
pub struct ScratchSlot {
    repo: DataRepo,
    rel: RepoRelPath,
}

impl ScratchSlot {
    /// 暂存编辑态(写侧原子落,同一条缝),拿回落点。
    pub fn stage(repo: &DataRepo, doc: &RecipeDoc, recipe_rel: &str) -> Result<Self, String> {
        let rel = scratch_rel(recipe_rel).map_err(|e| e.why())?;
        stage_for_validate(repo, doc, &rel)?;
        Ok(Self {
            repo: repo.clone(),
            rel,
        })
    }

    /// 暂存档在数据仓根之下的相对路径(隐藏、确定名)。
    pub fn rel(&self) -> &RepoRelPath {
        &self.rel
    }

    /// 给引擎 argv 的绝对路径(外部进程只认路径;壳层不自己拼)。
    pub fn abs(&self) -> String {
        self.repo.abs_path(&self.rel)
    }

    /// 清掉暂存档(scratch 清理,不是写回)。正常 / 失败 / 返航 / 换根都走它。
    pub fn clear(&self) {
        clear_staged(&self.repo, &self.rel);
    }
}

/// 确认放行后写回:把拟落账递去写侧原子落盘(tmp+rename 覆盖 recipe.yaml)。
pub fn commit_writeback(
    repo: &DataRepo,
    plan: &WritePlan,
) -> Result<WriteReceipt, write::WriteError> {
    repo.commit(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nwt-recipe-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn repo(root: &Path) -> DataRepo {
        DataRepo::new(root.to_str().unwrap())
    }

    /// 一份带未知字段的配方:顶层未知 `metadata` / `labels`,组件项里的未知
    /// `notes`(标量)与 `tuning`(块),env 项里的未知 `purpose`。
    const SAMPLE: &str = "apiVersion: nw/v1\n\
kind: recipe\n\
name: research-desk\n\
version: 0.2.0\n\
summary: 研究台\n\
profile: desk\n\
runtime:\n  node: 22\n  pi: 0.84\n  chezmoi: 2\n\
components:\n  - component: scout\n    version: 1.0.0\n    notes: 主检索\n  - component: hello-world\n    source: external\n    tuning:\n      temperature: 0.3\n\
models:\n  defaults:\n    id: base\n  providers: []\n\
env:\n  - name: OPENAI_API_KEY\n    required: true\n    usedBy: [openai]\n    purpose: 调大模型\n  - name: OPTIONAL_FLAG\n    required: false\n\
metadata:\n  owner: me\nlabels:\n  tier: prod\n";

    #[test]
    fn 往返_字节级保真_未知字段一个不丢() {
        let doc = RecipeDoc::parse(SAMPLE).unwrap();
        assert_eq!(doc.serialize(), SAMPLE, "不编辑时往返逐字节相同");
    }

    #[test]
    fn 加删组件槽_往返不丢未知字段() {
        let mut doc = RecipeDoc::parse(SAMPLE).unwrap();
        assert_eq!(doc.component_views().len(), 2, "夹具两个组件槽");
        doc.add_component("retention", "0.4.1");
        doc.remove_component(0); // 删掉 scout
        let out = doc.serialize();
        // 编辑生效
        assert!(out.contains("component: retention"), "新槽在场:{out}");
        assert!(!out.contains("component: scout"), "旧槽已删:{out}");
        // 未知字段不丢(顶层 + 项内)
        assert!(out.contains("metadata:"), "顶层未知块不丢:{out}");
        assert!(out.contains("owner: me"), "顶层未知子字段不丢:{out}");
        assert!(out.contains("labels:"), "另一未知顶层块不丢:{out}");
        assert!(!out.contains("notes: 主检索"), "scout 的 notes 随删消失");
        assert!(out.contains("tuning:"), "存活项里的未知块不丢:{out}");
        assert!(out.contains("temperature: 0.3"), "未知块子字段不丢:{out}");
        // 未碰的 env 未知字段不丢
        assert!(out.contains("purpose: 调大模型"), "env 未知字段不丢:{out}");
    }

    #[test]
    fn 改env_required与默认_往返不丢未知字段() {
        let mut doc = RecipeDoc::parse(SAMPLE).unwrap();
        assert_eq!(doc.env_views().len(), 2);
        doc.set_env_required(1, true); // OPTIONAL_FLAG → required
        doc.set_env_default(1, "on"); // 加默认
        doc.set_env_default(0, ""); // 清第一条的默认(没有就不动作)
        let out = doc.serialize();
        assert!(out.contains("name: OPTIONAL_FLAG"), "env 名在场:{out}");
        assert!(out.contains("default: on"), "默认已加:{out}");
        // usedBy / purpose 未知字段不丢
        assert!(out.contains("usedBy: [openai]"), "usedBy 不丢:{out}");
        assert!(out.contains("purpose: 调大模型"), "purpose 不丢:{out}");
        // required 被改:第一条仍 true,第二条改 true
        let flags: Vec<&str> = out
            .lines()
            .filter(|l| l.trim_start().starts_with("required:"))
            .collect();
        assert_eq!(flags.len(), 2, "两条 env 都有 required:{out}");
        assert!(
            flags.iter().all(|l| l.contains("true")),
            "两条都改 true:{out}"
        );
    }

    #[test]
    fn 空序列也能加第一项() {
        let text = "apiVersion: nw/v1\nkind: recipe\nname: x\nversion: 1.0.0\nprofile: p\nruntime:\n  node: 22\ncomponents:\nenv:\n";
        let mut doc = RecipeDoc::parse(text).unwrap();
        assert!(doc.component_views().is_empty(), "空 components");
        doc.add_component("scout", "1.0.0");
        doc.add_env("FOO");
        let out = doc.serialize();
        assert!(out.contains("- component: scout"), "空序列加出第一项:{out}");
        assert!(out.contains("- name: FOO"), "env 空序列加出第一项:{out}");
    }

    #[test]
    fn 无components无env节也能建() {
        let text = "apiVersion: nw/v1\nkind: recipe\nname: x\nversion: 1.0.0\nprofile: p\n";
        let mut doc = RecipeDoc::parse(text).unwrap();
        doc.add_component("scout", "1.0.0");
        doc.add_env("FOO");
        let out = doc.serialize();
        assert!(out.contains("components:"), "补出 components 节:{out}");
        assert!(out.contains("- component: scout"), "并在其中加项:{out}");
        assert!(out.contains("env:"), "补出 env 节:{out}");
        assert!(out.contains("- name: FOO"), "并在其中加项:{out}");
    }

    #[test]
    fn 拟落账_不落一个字节_目标是recipe文件本身() {
        let root = fixture("plan");
        fs::create_dir_all(root.join("recipes/research-desk")).unwrap();
        let original = SAMPLE;
        fs::write(root.join("recipes/research-desk/recipe.yaml"), original).unwrap();
        let r = repo(&root);
        let mut doc =
            crate::datarepo::recipe::load(&r, "recipes/research-desk/recipe.yaml").unwrap();
        doc.add_component("retention", "0.4.1");
        let plan = plan_writeback(&r, &doc, "recipes/research-desk/recipe.yaml").unwrap();
        assert_eq!(plan.paths(), vec!["recipes/research-desk/recipe.yaml"]);
        // 拟落账内容 = 序列化后的编辑态
        let want = String::from_utf8(plan.writes[0].content.clone()).unwrap();
        assert_eq!(want, doc.serialize());
        // 一个字节都没落:磁盘还是原稿
        assert_eq!(
            fs::read_to_string(root.join("recipes/research-desk/recipe.yaml")).unwrap(),
            original,
            "拟落账只算不写"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 预跑暂存_是配方兄弟隐藏文件_validate完即清() {
        let root = fixture("stage");
        fs::create_dir_all(root.join("recipes/research-desk")).unwrap();
        fs::write(root.join("recipes/research-desk/recipe.yaml"), SAMPLE).unwrap();
        let r = repo(&root);
        let doc = load(&r, "recipes/research-desk/recipe.yaml").unwrap();
        let slot = ScratchSlot::stage(&r, &doc, "recipes/research-desk/recipe.yaml").unwrap();
        assert_eq!(
            slot.rel.as_str(),
            "recipes/research-desk/.nwt-validate.yaml",
            "暂存落点 = 配方同目录的隐藏固定名"
        );
        // 暂存在场,内容 = 编辑态序列化;recipe.yaml 原样不动
        let at = root.join(slot.rel.as_str());
        assert!(at.exists(), "暂存文件在场");
        assert_eq!(fs::read_to_string(&at).unwrap(), doc.serialize());
        assert_eq!(
            fs::read_to_string(root.join("recipes/research-desk/recipe.yaml")).unwrap(),
            SAMPLE,
            "暂存不碰 recipe.yaml"
        );
        // 没有残留的 .nwt-tmp
        assert!(!root
            .join("recipes/research-desk/.nwt-validate.yaml.nwt-tmp")
            .exists());
        slot.clear();
        assert!(!at.exists(), "validate 完即清");
        fs::remove_dir_all(&root).ok();
    }

    /// 回音路由判据(#92):预跑 argv 与健康屏同源(同一构造函数),scratch
    /// 路径后面还跟着 `--repo <数据仓根>`——判据必须扫整条 argv,按「末位」
    /// 判会把预跑回音误路由到健康屏(编辑器永远停在「预跑中」)。
    #[test]
    fn 预跑argv判据_扫整条argv_带repo也不误路由() {
        let scratch = "/data/repo/recipes/research-desk/.nwt-validate.yaml";
        // 带 --repo 的单侧配方 argv(#92 起的形状):scratch 不在末位
        let with_repo = vec![
            "validate".to_string(),
            "--recipe".to_string(),
            scratch.to_string(),
            "--repo".to_string(),
            "/data/repo".to_string(),
        ];
        assert!(
            is_prerun_argv(&with_repo),
            "带了 --repo 仍是预跑 argv:{with_repo:?}"
        );
        // 健康屏的 validate 指向真实配方路径,不含暂存标记
        let health = vec![
            "validate".to_string(),
            "--recipe".to_string(),
            "/data/repo/recipes/research-desk/recipe.yaml".to_string(),
            "--repo".to_string(),
            "/data/repo".to_string(),
        ];
        assert!(!is_prerun_argv(&health), "健康屏 argv 不是预跑:{health:?}");
        // 空 argv 不炸
        assert!(!is_prerun_argv(&[]));
    }

    /// 窄准许面的四条约束之一「跑完即清」的三条径:正常回音落地、预跑未过
    /// (失败)、返航——三条都走 [`ScratchSlot::clear`](super::ScratchSlot::clear),
    /// 暂存档一个字节都不留(ADR-0011 §1 第四类)。
    #[test]
    fn 暂存档_正常失败返航三径都清干净() {
        let root = fixture("clear-paths");
        fs::create_dir_all(root.join("recipes/research-desk")).unwrap();
        fs::write(root.join("recipes/research-desk/recipe.yaml"), SAMPLE).unwrap();
        let r = repo(&root);
        let doc = load(&r, "recipes/research-desk/recipe.yaml").unwrap();
        // ① 正常径:预跑回音落地即清
        let ok = ScratchSlot::stage(&r, &doc, "recipes/research-desk/recipe.yaml").unwrap();
        ok.clear();
        assert!(!root.join(ok.rel.as_str()).exists(), "正常径:回音落地即清");
        // ② 失败径:预跑未过也清(不让残档冒充下一轮的编辑态)
        let bad = ScratchSlot::stage(&r, &doc, "recipes/research-desk/recipe.yaml").unwrap();
        bad.clear();
        assert!(!root.join(bad.rel.as_str()).exists(), "失败径:未过也清");
        // ③ 返航径:没跑预跑就放弃,同样清
        let dropped = ScratchSlot::stage(&r, &doc, "recipes/research-desk/recipe.yaml").unwrap();
        dropped.clear();
        assert!(!root.join(dropped.rel.as_str()).exists(), "返航径:放弃也清");
        // recipe.yaml 自始至终未被确认不动
        assert_eq!(
            fs::read_to_string(root.join("recipes/research-desk/recipe.yaml")).unwrap(),
            SAMPLE
        );
        fs::remove_dir_all(&root).ok();
    }

    /// 换根径:暂存档按**暂存时那个根**清——操作者中途换过数据仓根,残档也不
    /// 留在旧根里(否则「确认前一个字节都不落」在换根这一刻漏一个洞)。
    #[test]
    fn 暂存档_换根也按当初那个根清() {
        let old_root = fixture("swap-old");
        let new_root = fixture("swap-new");
        for r in [&old_root, &new_root] {
            fs::create_dir_all(r.join("recipes/research-desk")).unwrap();
            fs::write(r.join("recipes/research-desk/recipe.yaml"), SAMPLE).unwrap();
        }
        let doc = load(&repo(&old_root), "recipes/research-desk/recipe.yaml").unwrap();
        // 在旧根暂存,随后设置态指向新根
        let slot = ScratchSlot::stage(&repo(&old_root), &doc, "recipes/research-desk/recipe.yaml")
            .unwrap();
        assert!(old_root.join(slot.rel.as_str()).exists(), "暂存落在旧根");
        // 此刻设置态若指向新根(调用方拿的是新句柄),slot 仍按暂存时那个根清
        slot.clear();
        assert!(
            !old_root.join(slot.rel.as_str()).exists(),
            "换根也清:残档不留在旧根"
        );
        assert!(
            !new_root.join(slot.rel.as_str()).exists(),
            "新根什么都没落(暂存只发生一次)"
        );
        fs::remove_dir_all(&old_root).ok();
        fs::remove_dir_all(&new_root).ok();
    }

    /// 名字确定(隐藏 + 固定)的后果:进程被杀留下的残档被下一轮 stage 截断
    /// 复用,不积垃圾、也不会和下一轮的编辑态混在一起。
    #[test]
    fn 暂存档_残档被下一轮截断复用_不积垃圾() {
        let root = fixture("stale");
        fs::create_dir_all(root.join("recipes/research-desk")).unwrap();
        fs::write(root.join("recipes/research-desk/recipe.yaml"), SAMPLE).unwrap();
        let r = repo(&root);
        let doc = load(&r, "recipes/research-desk/recipe.yaml").unwrap();
        let first = ScratchSlot::stage(&r, &doc, "recipes/research-desk/recipe.yaml").unwrap();
        // 模拟崩溃:不 clear,直接再来一轮(编辑态已改)
        let mut edited = load(&r, "recipes/research-desk/recipe.yaml").unwrap();
        edited.add_component("retention", "0.4.1");
        let second = ScratchSlot::stage(&r, &edited, "recipes/research-desk/recipe.yaml").unwrap();
        assert_eq!(first.rel, second.rel, "固定名:两轮落在同一个文件(截断复用)");
        assert_eq!(
            fs::read_to_string(root.join(second.rel.as_str())).unwrap(),
            edited.serialize(),
            "残档被截断,内容是这一轮的编辑态,不混上一轮"
        );
        // 目录里只有 recipe.yaml + 暂存档两件,没有 .nwt-tmp 残渣
        let names: Vec<String> = fs::read_dir(root.join("recipes/research-desk"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names.len(), 2, "不积垃圾:{names:?}");
        second.clear();
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 确认后写回_整文件原子替换() {
        let root = fixture("commit");
        fs::create_dir_all(root.join("recipes/research-desk")).unwrap();
        fs::write(root.join("recipes/research-desk/recipe.yaml"), SAMPLE).unwrap();
        let r = repo(&root);
        let mut doc = load(&r, "recipes/research-desk/recipe.yaml").unwrap();
        doc.remove_component(0);
        doc.set_env_required(1, true);
        let plan = plan_writeback(&r, &doc, "recipes/research-desk/recipe.yaml").unwrap();
        let receipt = commit_writeback(&r, &plan).unwrap();
        assert_eq!(receipt.written, vec!["recipes/research-desk/recipe.yaml"]);
        let on_disk = fs::read_to_string(root.join("recipes/research-desk/recipe.yaml")).unwrap();
        assert_eq!(on_disk, doc.serialize(), "写回 = 编辑态序列化");
        assert!(!on_disk.contains("component: scout"), "删槽生效");
        assert!(on_disk.contains("metadata:"), "未知字段随写回保留");
        fs::remove_dir_all(&root).ok();
    }
}
