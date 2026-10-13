//! 作者侧脚手架表单流(spec #95 工单 #110 skill 含首套模板 / #111
//! extension pi-extension 契约)。
//!
//! 面板第一次**自己写文件**的消费方之一(ADR-0010):图形表单生成 `SKILL.md`
//! 骨架或 extension 的 manifest + 入口文件,落进数据仓 `skills/` 或
//! `extensions/`。落盘全程走 datarepo 写侧(#109)——**拟落账
//! 摊开(路径 + 内容)→ 图形确认 → 原子落盘(tmp + rename)**,确认前一个字节
//! 不落。
//!
//! # 可复用抽象(#111 extension 脚手架复用同一表单流,不复制)
//!
//! 表单流拆成两层,按「什么四张消费票共用 / 什么各自不同」切:
//!
//! - **共用的表单流**(本模块的[`ScaffoldFlow`] + [`ScaffoldSpec`] + 拟落账
//!   预览/结论渲染 + `App` 的 `on_key_scaffold` / `dispatch_scaffold_confirm`):
//!   阶段机(表单 → 拟落账预览 → 结论)、拟落账形状、确认闸、原子落盘调用。
//!   #111 的 extension 脚手架换一份表单与一份 [`ScaffoldSpec`] 实现即可,
//!   流程/预览/落盘一字不重写。
//! - **各自不同的规格**(本文件的 [`SkillForm`] / [`ExtensionForm`] 各一份
//!   `impl ScaffoldSpec` + `render_*_form` / `on_key_*_form`):字段形态、
//!   形状校验、骨架内容、落点目录。两份规格由 [`ScaffoldKind`] 指名,
//!   `App` 按它取哪份表单在场(入口门 / 确认闸 / 落盘仍只有一处)。
//!
//! 字段形状校验**分别对齐上游对相应文件的检查**:skill 侧
//! `SKILL.md` frontmatter 必填 `name` + `description`(见上游
//! `docs/external-sources.md` 注册验收第 2 条);skill 身份 = frontmatter 的
//! `name`,故名字按引擎 `validName` 同款口径校验(它同时是落点目录名
//! `skills/<名>/`)。extension 侧对齐 #21 的第六条验收:条目子树自带
//! `package.json` 且 `pi.extensions` 非空、清单里每条路径真实存在
//! (上游 `internal/source/scan.go` 的 `ExtensionManifestIssue`;validate 的
//! 形态面 = `catalog.extension_entries`),面板侧镜像见
//! [`ext_manifest_audit`]。产出内容是**面板自有资产**
//! (内置模板),不是引擎逻辑复刻(ADR-0010 §3)。
//!
//! 两份规格的落点同源:数据仓的组件家是 `skills/` 与 `extensions/`
//! (`datarepo` 读侧 `SKILLS_DIR` / `EXTENSIONS_DIR` 是路径知识的单源,
//! #114 预扫与 #116 引用完整性都按它认条目)。上游那半的**派内**落点是
//! `<agent-dir>/nw/extensions/<仓库>/<条目>/` 且必须登记
//! `settings.packages`(external-sources.md「扩展这半」)——那是 `up` 铺下
//! 去的事,面板脚手架只写数据仓里的源,不写派、也不写 `catalog.json`。

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::datarepo::write::{PlannedWrite, RepoRelPath, WritePlan, WriteReceipt};
use crate::input::{self, InputAction};
use crate::layout::popup_rect;
use crate::screens::recipes::clip;
use crate::screens::FormAction;

// ---- 共用的表单流:阶段机 + 拟落账/收据槽 ----

/// 脚手架表单流的阶段。与登记两段式(`RegPhase`)同构的精神:**拟落账段
/// 文件不落**,落盘段才在图形确认后发生;任一阶段可返航,返航不留半份骨架。
///
/// 与两段式登记的区别:登记落盘是**异步引擎命令**(入队等回音,故有在途态),
/// 脚手架落盘是**同步文件 IO**(`DataRepo::commit` 当场 tmp + rename),没有
/// 「在途」态——确认键按下即写完,直接进结论。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ScaffoldPhase {
    /// 未脚手架;配方屏常态。
    #[default]
    Idle,
    /// 填表(规格自己的表单形态,如 skill 的名称/描述/触发词/模板)。
    Form,
    /// 拟落账已定型并摊开(路径 + 内容预览),**一个字节都没落**;等图形确认。
    Review,
    /// 已确认并原子落盘,结论(实落清单)在此呈现。
    Done,
}

/// 脚手架表单流的运行时:阶段 + 拟落账 + 实落收据 + 落盘失败原因。
/// **与规格无关**——规格差异全在 [`ScaffoldSpec`] 与本模块各自的表单上,
/// 这一份骨架被 #110 / #111 共用(拟落账 / 收据都是 datarepo 写侧的形状)。
#[derive(Default)]
pub struct ScaffoldFlow {
    pub phase: ScaffoldPhase,
    /// 拟落账(表单 Enter 后由写侧 `plan` 定型;确认闸直接呈现它)。
    pub plan: Option<WritePlan>,
    /// 实落收据(确认放行后 `commit` 回报;结论屏呈现)。
    pub receipt: Option<WriteReceipt>,
    /// 落盘失败的人话(磁盘 / 权限问题,不是输入问题;留在确认态说清)。
    pub error: Option<String>,
}

impl ScaffoldFlow {
    /// 发起脚手架:进表单态,清掉上一轮(旧拟落账/收据不冒充本次)。
    pub fn begin(&mut self) {
        self.phase = ScaffoldPhase::Form;
        self.plan = None;
        self.receipt = None;
        self.error = None;
    }

    /// 表单 Enter 且字段合法:拟落账定型,进预览态(**仍不落盘**)。
    pub fn enter_review(&mut self, plan: WritePlan) {
        self.phase = ScaffoldPhase::Review;
        self.plan = Some(plan);
        self.receipt = None;
        self.error = None;
    }

    /// 确认放行后落盘成功:进结论态,收据在手。
    pub fn enter_done(&mut self, receipt: WriteReceipt) {
        self.phase = ScaffoldPhase::Done;
        self.receipt = Some(receipt);
        self.error = None;
    }

    /// 落盘失败:留在确认态把结论说清(不冒充成功、不悄悄回表单;
    /// 拟落账仍在,操作者可知刚才想落什么)。
    pub fn fail(&mut self, reason: String) {
        self.error = Some(reason);
    }

    /// 返航:任一态同义,清空不留半份(拟落账/收据一并丢)。
    pub fn abandon(&mut self) {
        self.phase = ScaffoldPhase::Idle;
        self.plan = None;
        self.receipt = None;
        self.error = None;
    }

    /// 表单流在场(HUD / 键位优先级 / 命中共用这一条)。
    pub fn in_flight(&self) -> bool {
        !matches!(self.phase, ScaffoldPhase::Idle)
    }
}

/// 脚手架规格契约:#110 skill 与 #111 extension 各实现一份,表单流本身不复制。
///
/// 只有一个必填方法——**字段形状校验 + 拟落账条目生成**(面板自有资产):
/// 合法字段 → 一组 [`PlannedWrite`](datarepo::write::PlannedWrite)(将写路径 +
/// 内容字节);不合法 → 人话原因清单(拦在表单里,留可改,不喂给写侧)。
/// 之后的「写侧 `plan` 校验 → 拟落账预览 → 图形确认 → `commit` 原子落盘」
/// 全走共用流程,规格不再参与。
pub trait ScaffoldSpec {
    /// 表单标题(呈现层用;如 "skill" / "extension")。
    fn title() -> &'static str
    where
        Self: Sized;

    /// 字段形状校验(对齐上游 validate 对相应文件的检查)+ 生成拟落账条目。
    /// `Err` = 拒绝原因(拦在表单里);`Ok` = 将写文件清单(路径 + 内容字节)。
    fn planned_writes(&self) -> Result<Vec<PlannedWrite>, Vec<String>>;
}

// ---- skill 规格(#110):名称 / 描述 / 触发词表单 + 首套 agent skill 模板 ----

/// skill 表单的字段(↑/↓ 移焦点;文本字段走通用单行输入态机)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SkillField {
    #[default]
    Name,
    Description,
    Triggers,
    /// 模板起步开关(空格切换;不是文本字段)。
    Template,
}

impl SkillField {
    pub const ALL: [SkillField; 4] = [
        SkillField::Name,
        SkillField::Description,
        SkillField::Triggers,
        SkillField::Template,
    ];

    /// 字段标签(与工单「名称/描述/触发词」同词)。
    pub fn label(self) -> &'static str {
        match self {
            SkillField::Name => "名称",
            SkillField::Description => "描述",
            SkillField::Triggers => "触发词",
            SkillField::Template => "模板",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    /// 文本字段(有单行缓冲);模板开关没有。
    fn is_text(self) -> bool {
        !matches!(self, SkillField::Template)
    }
}

/// skill 脚手架表单:名称 / 描述 / 触发词 + 模板起步开关。
/// `name` = skill 身份(= SKILL.md frontmatter 的 name,也是落点目录名)。
#[derive(Clone, Default)]
pub struct SkillForm {
    pub name: String,
    pub description: String,
    /// 触发词:一行,逗号 / 顿号 / 分号分隔(面板侧便利,进骨架正文)。
    pub triggers: String,
    /// 从内置 agent skill 模板起步(true)还是空白骨架(false)。
    pub template: bool,
    pub field: SkillField,
}

impl SkillForm {
    fn focused_buf(&mut self) -> Option<&mut String> {
        match self.field {
            SkillField::Name => Some(&mut self.name),
            SkillField::Description => Some(&mut self.description),
            SkillField::Triggers => Some(&mut self.triggers),
            SkillField::Template => None,
        }
    }

    /// ↑/↓ 移字段(四字段轮转,到顶/到底不越界、不绕回)。
    fn move_field(&mut self, down: bool) {
        let i = self.field.index();
        let n = SkillField::ALL.len();
        self.field = SkillField::ALL[if down {
            (i + 1).min(n - 1)
        } else {
            i.saturating_sub(1)
        }];
    }
}

impl ScaffoldSpec for SkillForm {
    fn title() -> &'static str {
        "skill"
    }

    fn planned_writes(&self) -> Result<Vec<PlannedWrite>, Vec<String>> {
        let mut errs = Vec::new();
        // 名称:skill 身份 + 落点目录名,按引擎 validName 同款口径(上游惯例)。
        let name = self.name.trim();
        if name.is_empty() {
            errs.push("名称必填:skill 身份 = SKILL.md frontmatter 的 name".into());
        } else if let Some(reason) = invalid_entry_name(name) {
            errs.push(reason);
        }
        // 描述:SKILL.md frontmatter 的 description,validate 盯着这项(注册验收第 2 条)。
        let description = self.description.trim();
        if description.is_empty() {
            errs.push("描述必填:SKILL.md frontmatter 的 description(validate 盯着这项)".into());
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        let triggers = split_triggers(&self.triggers);
        let content = skill_md(name, description, &triggers, self.template);
        // 落点:数据仓 skills/<名>/SKILL.md。name 已校验为合法目录名,拼得出合法相对路径。
        let path = RepoRelPath::new(&format!("skills/{name}/SKILL.md"))
            .expect("name 已校验:拼得出的相对路径合法");
        Ok(vec![PlannedWrite::text(path, &content)])
    }
}

/// 名称形状校验(对齐上游 `internal/bake/scaffold.go` 的 `validName`:名字直接
/// 拼进文件路径,挡路径穿越与破坏路径的字符;字母数字按 Unicode 判,`-` `_` `.`
/// 放行——非 ASCII 名合法,同引擎口径)。
///
/// **两份规格共用这一条**:skill 名是 `skills/<名>/` 的目录名,extension
/// 条目名是 `extensions/<名>/` 的目录名(上游 #21:扩展没有 frontmatter,
/// 身份就是子树目录名)——两者都直接拼进落点路径,判据同源,不写两份。
fn invalid_entry_name(name: &str) -> Option<String> {
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Some("名称不能是路径:不含 `/` `\\` 分隔符,也不是 `.` / `..`".into());
    }
    if let Some(c) = name
        .chars()
        .find(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        return Some(format!(
            "名称含不支持的字符「{c}」:只允许字母 / 数字 / 连字符 / 下划线 / 点"
        ));
    }
    None
}

/// 触发词拆分:一行里逗号 / 顿号 / 分号 / 换行分隔,去空白、丢空段。
fn split_triggers(raw: &str) -> Vec<String> {
    raw.split([',', '，', '、', ';', '；', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// 生成 `SKILL.md` 字节。**frontmatter 必填 `name` + `description`**(上游
/// validate 对 skill 的检查项);正文按空白 / 模板两路给——模板是面板自有的
/// 「首套 agent skill 模板」静态资产,不是引擎逻辑复刻。
fn skill_md(name: &str, description: &str, triggers: &[String], template: bool) -> String {
    let mut s = String::new();
    s.push_str("---\n");
    s.push_str(&format!("name: {name}\n"));
    s.push_str(&format!("description: {description}\n"));
    s.push_str("---\n");
    s.push_str(&format!("\n# {name}\n\n{description}\n"));
    if template {
        // 首套 agent skill 模板:给「何时触发 / 用法 / 备注」的起步段落。
        s.push_str("\n## 何时触发\n\n");
        if triggers.is_empty() {
            s.push_str("- (补充:什么时候该启用这个 skill)\n");
        } else {
            for t in triggers {
                s.push_str(&format!("- {t}\n"));
            }
        }
        s.push_str("\n## 用法\n\n(说明这个 skill 做什么、怎么用)\n");
        s.push_str("\n## 备注\n\n- 由 nwt 脚手架生成(agent skill 模板起步);按需增删。\n");
    } else if !triggers.is_empty() {
        // 空白骨架:只在前置必填之外补一个触发词段(有触发词才补)。
        s.push_str("\n## 触发词\n\n");
        for t in triggers {
            s.push_str(&format!("- {t}\n"));
        }
    }
    s
}

/// 表单键位(规格自己的键入分发):↑/↓ 移字段;模板字段空格切换;文本字段走
/// 通用单行输入态机(Enter 提交 / Esc·q 返航 / 退格删 / 只收可打印字符)。
/// 后果形状与编目册表单同一只 [`FormAction`](crate::screens::FormAction)。
pub fn on_key_skill_form(form: &mut SkillForm, k: KeyEvent) -> FormAction {
    match k.code {
        KeyCode::Up => {
            form.move_field(false);
            return FormAction::Edited;
        }
        KeyCode::Down => {
            form.move_field(true);
            return FormAction::Edited;
        }
        // 模板开关:焦点在模板字段时,空格切换(空格在文本字段里仍是普通字符)。
        KeyCode::Char(' ') if form.field == SkillField::Template => {
            form.template = !form.template;
            return FormAction::Edited;
        }
        _ => {}
    }
    if !form.field.is_text() {
        // 模板字段没有文本缓冲:Enter 提交、Esc/q 返航,其余键不动作。
        return match k.code {
            KeyCode::Enter => FormAction::Submit,
            KeyCode::Esc | KeyCode::Char('q') => FormAction::Cancel,
            _ => FormAction::Edited,
        };
    }
    match input::on_key(form.focused_buf().expect("文本字段有缓冲"), k) {
        InputAction::Submit => FormAction::Submit,
        InputAction::Cancel => FormAction::Cancel,
        InputAction::Edited | InputAction::Ignored => FormAction::Edited,
    }
}

// ---- extension 规格(#111):pi-extension landing 契约 ----

/// 脚手架的目标规格:#110 skill 与 #111 extension 各一份表单、各一份
/// [`ScaffoldSpec`],**表单流本身不分叉**(阶段机 / 拟落账 / 确认闸 / 原子
/// 落盘全共用)。`App` 按它取哪份表单在场,入口门与落盘仍只有一处。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ScaffoldKind {
    #[default]
    Skill,
    Extension,
}

impl ScaffoldKind {
    /// 屏上 / 日志里的词(与上游 `nw source register --kind` 的槽位同词,
    /// 不另造;`internal/model` 的 `CatalogKindSkill` / `CatalogKindExtension`)。
    pub fn word(self) -> &'static str {
        match self {
            ScaffoldKind::Skill => "skill",
            ScaffoldKind::Extension => "extension",
        }
    }
}

/// 扩展表单字段(↑/↓ 移焦点;文本字段走通用单行输入态机)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ExtensionField {
    #[default]
    Name,
    Description,
    /// 入口文件名:manifest 的 `pi.extensions` 指着它。
    Entry,
    /// 模板起步开关(空格切换;不是文本字段)。
    Template,
}

impl ExtensionField {
    pub const ALL: [ExtensionField; 4] = [
        ExtensionField::Name,
        ExtensionField::Description,
        ExtensionField::Entry,
        ExtensionField::Template,
    ];

    /// 字段标签。
    pub fn label(self) -> &'static str {
        match self {
            ExtensionField::Name => "名称",
            ExtensionField::Description => "描述",
            ExtensionField::Entry => "入口文件",
            ExtensionField::Template => "模板",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    /// 文本字段(有单行缓冲);模板开关没有。
    fn is_text(self) -> bool {
        !matches!(self, ExtensionField::Template)
    }
}

/// 入口文件名的缺省值:pi 的扩展发现规则是
/// `<agent-dir>/extensions/*.ts` + `<agent-dir>/extensions/*/index.ts`
///(**只一层**),条目根上的入口就叫 `index.ts`(上游 `docs/extensions.md`
/// 的发现位置表,转述于 `docs/external-sources.md`「扩展这半」)。
pub const DEFAULT_ENTRY: &str = "index.ts";

/// 条目 `package.json` 的起始版本:上游只读它、不校验它(本地组件的
/// `version` 才被 validate 查),给一个诚实的起步值,作者自己 bump。
const EXT_VERSION: &str = "0.1.0";

/// 条目清单的文件名:上游样本 `docs/samples/components/scout/package.json`。
const EXT_MANIFEST_FILE: &str = "package.json";

/// extension 脚手架表单:名称 / 描述 / 入口文件 + 模板起步开关。
/// `name` = 条目身份(= 条目子树目录名,上游 #21:扩展没有 frontmatter 可当
/// 身份),也是数据仓落点 `extensions/<名>/` 的目录名。
#[derive(Clone)]
pub struct ExtensionForm {
    pub name: String,
    pub description: String,
    /// 入口文件名(manifest 的 `pi.extensions` 指着它;缺省 [`DEFAULT_ENTRY`])。
    pub entry: String,
    /// 从内置 extension 模板起步(true)还是空白入口(false)。
    pub template: bool,
    pub field: ExtensionField,
}

impl ExtensionForm {
    fn focused_buf(&mut self) -> Option<&mut String> {
        match self.field {
            ExtensionField::Name => Some(&mut self.name),
            ExtensionField::Description => Some(&mut self.description),
            ExtensionField::Entry => Some(&mut self.entry),
            ExtensionField::Template => None,
        }
    }

    /// ↑/↓ 移字段(四字段轮转,到顶/到底不越界、不绕回)——与 [`SkillForm`]
    /// 同一只移字段习惯,不因规格不同改操作者的肌肉记忆。
    fn move_field(&mut self, down: bool) {
        let i = self.field.index();
        let n = ExtensionField::ALL.len();
        self.field = ExtensionField::ALL[if down {
            (i + 1).min(n - 1)
        } else {
            i.saturating_sub(1)
        }];
    }
}

/// `Default` 手工实现(derive 给不出非空初值):入口文件名预填
/// [`DEFAULT_ENTRY`] —— pi 的发现规则与上游样本都以 `index.ts` 为条目根
/// 入口,预填它 = 少一个要想的字段,清空了也有形状校验兜。
impl Default for ExtensionForm {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            entry: DEFAULT_ENTRY.to_string(),
            template: false,
            field: ExtensionField::Name,
        }
    }
}

impl ScaffoldSpec for ExtensionForm {
    fn title() -> &'static str {
        "extension"
    }

    fn planned_writes(&self) -> Result<Vec<PlannedWrite>, Vec<String>> {
        let mut errs = Vec::new();
        // 名称:条目身份 + 落点目录名(上游 #21:身份 = 子树目录名)。
        let name = self.name.trim();
        if name.is_empty() {
            errs.push("名称必填:扩展条目身份 = 条目子树目录名(上游 #21)".into());
        } else if let Some(reason) = invalid_entry_name(name) {
            errs.push(reason);
        }
        // 描述:进 manifest 的 description 与入口文件头(面板侧便利字段)。
        let description = self.description.trim();
        if description.is_empty() {
            errs.push("描述必填:进 package.json 的 description 与入口文件头".into());
        }
        // 入口文件:manifest 的 pi.extensions 指着它,形状不合 = 清单与入口对不上。
        let entry = self.entry.trim();
        if let Some(reason) = invalid_entry_file(entry) {
            errs.push(reason);
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        // 落点:数据仓 extensions/<名>/(作者侧脚手架的扩展家,与 skills/ 对称;
        // 见 datarepo::SKILLS_DIR / EXTENSIONS_DIR 的路径知识单源)。
        let dir = format!("extensions/{name}");
        let writes = vec![
            PlannedWrite::text(
                RepoRelPath::new(&format!("{dir}/{EXT_MANIFEST_FILE}"))
                    .expect("name 已校验:拼得出的相对路径合法"),
                &ext_package_json(name, description, entry),
            ),
            PlannedWrite::text(
                RepoRelPath::new(&format!("{dir}/{entry}"))
                    .expect("name/entry 已校验:拼得出的相对路径合法"),
                &ext_entry_ts(name, description, self.template),
            ),
        ];
        // 自审:上游 register 验收第六条(仅 --kind extension)的面板侧镜像。
        // 面板自己生成 manifest 与入口两边,这里保证两边对得上 —— 对不上就是
        // 「装上了不加载」的形状,上游 register 会拒。
        if let Some(reason) = ext_manifest_audit(&dir, &writes) {
            return Err(vec![reason]);
        }
        Ok(writes)
    }
}

/// 入口文件名形状:条目子树里的一个文件,manifest 的 `pi.extensions` 指着它。
/// 单段(不带 `/` `\`:落点路径由面板拼,入口文件不是子目录)、`.ts` / `.js`
/// 后缀(pi 的扩展只认这两种)、不是隐藏文件。
fn invalid_entry_file(entry: &str) -> Option<String> {
    if entry.is_empty() {
        return Some(format!(
            "入口文件名必填:manifest 的 pi.extensions 指着它(缺省 {DEFAULT_ENTRY})"
        ));
    }
    if entry.contains('/') || entry.contains('\\') {
        return Some("入口文件名是单段:不带 `/` `\\`(落点路径由面板拼)".into());
    }
    if entry.starts_with('.') {
        return Some("入口文件名不是隐藏文件(不以 `.` 开头)".into());
    }
    if !(entry.ends_with(".ts") || entry.ends_with(".js")) {
        return Some("入口文件名以 .ts 或 .js 结尾:pi 的扩展只认这两种".into());
    }
    None
}

/// JSON 字符串字面量(面板自写 manifest:字节要确定、键序按样本钉死,不引
/// serde 的序列化器)。控制字符按 `\u00XX` 逃,双引号与反斜杠转义。
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 单行化(描述进 `//` 注释与 JSON 串:换行会折断注释行,表单本就单行输入,
/// 这里是防线加深,不是新语义)。
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// 条目 `package.json` 字节(确定):上游样本
/// `docs/samples/components/scout/package.json` 的 pi manifest 形状 ——
/// `pi.extensions` **非空**是硬要求(上游 register 验收第六条;validate 的
/// `catalog.extension_entries` 审的就是编目里每条 entry 的 name/path 形态)。
///
/// 键序按样本钉死,`pi` 清单收成一行:拟落账预览屏(24 行)要能**把两份
/// 文件全摊开**给操作者看,多行 `pi` 会把入口文件挤出屏外。
///
/// 零依赖起步:**没有 `dependencies`** → bake 的 vendored 那套用不上
/// (#21「零依赖扩展零 npm」)。要引 pi 运行时包就写 `peerDependencies`
/// (宿主提供,别写 `dependencies`)—— 这句话写在入口文件头,不塞进清单。
fn ext_package_json(name: &str, description: &str, entry: &str) -> String {
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(&format!("  \"name\": {},\n", json_str(name)));
    s.push_str(&format!("  \"version\": {},\n", json_str(EXT_VERSION)));
    s.push_str("  \"private\": true,\n");
    s.push_str("  \"type\": \"module\",\n");
    s.push_str(&format!("  \"description\": {},\n", json_str(description)));
    s.push_str(&format!(
        "  \"pi\": {{ \"extensions\": [\"./{entry}\"] }}\n"
    ));
    s.push_str("}\n");
    s
}

/// 入口文件字节(确定):`export default` 工厂收 `ExtensionAPI`(上游样本
/// `components/scout/extensions/session-marker.ts` 同形)。空白路只给契约
/// 注释 + 空工厂;模板路多一条 `registerCommand` 起步。
fn ext_entry_ts(name: &str, description: &str, template: bool) -> String {
    let desc = one_line(description);
    let mut s = String::new();
    s.push_str(&format!(
        "// {name} —— pi 扩展入口(由 nwt extension 脚手架生成,spec #95 工单 #111)。\n"
    ));
    s.push_str("// 契约(上游 docs/external-sources.md「扩展这半」):pi 只扫一层\n");
    s.push_str("// extensions/*.ts 与 extensions/*/index.ts,条目必须自带\n");
    s.push_str("// package.json 且 pi.extensions 非空 —— 缺清单 = 装上了不加载。\n");
    s.push_str("// 本文件就是它指着的一个。零依赖起步(没有 dependencies → 不需要\n");
    s.push_str("// npm);要引 pi 运行时包就写 peerDependencies,别写 dependencies。\n");
    s.push_str("import type { ExtensionAPI } from \"@earendil-works/pi-coding-agent\";\n");
    s.push('\n');
    s.push_str("export default function (pi: ExtensionAPI) {\n");
    s.push_str(&format!("  // {desc}\n"));
    if template {
        s.push_str(&format!("  pi.registerCommand({}, {{\n", json_str(name)));
        s.push_str(&format!("    description: {},\n", json_str(&desc)));
        s.push_str("    handler: async (_arg, ctx) => {\n");
        s.push_str(&format!(
            "      ctx.ui.notify({}, \"info\");\n",
            json_str(&format!("{name}: 把活填在这里"))
        ));
        s.push_str("    },\n");
        s.push_str("  });\n");
    }
    s.push_str("}\n");
    s
}

/// 上游 register 验收第六条(仅 `--kind extension`,#21)的面板侧镜像:
/// 条目子树自带 `package.json`、`pi.extensions` 非空、清单里每条路径在条目
/// 子树里真实存在。判据逐条对齐上游 `internal/source/scan.go` 的
/// `ExtensionManifestIssue`(路径不许绝对、不许越级,匹配得到文件)。
///
/// 面板自己生成 manifest 与入口两边,这个自审把「两边对不上」挡在拟落账
/// 之前 —— 那种形状上游 register 会拒(装上了不加载),不该流到确认闸。
fn ext_manifest_audit(dir: &str, writes: &[PlannedWrite]) -> Option<String> {
    let manifest_rel = format!("{dir}/{EXT_MANIFEST_FILE}");
    let at = |rel: &str| writes.iter().find(|w| w.path.as_str() == rel);
    let Some(manifest) = at(&manifest_rel) else {
        return Some(format!(
            "条目子树没有 {EXT_MANIFEST_FILE}:pi 的包登记不兜底这种形状"
        ));
    };
    let text = String::from_utf8_lossy(&manifest.content);
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => return Some(format!("{EXT_MANIFEST_FILE} 解析失败:{e}")),
    };
    let Some(patterns) = v.pointer("/pi/extensions").and_then(|p| p.as_array()) else {
        return Some(format!(
            "{EXT_MANIFEST_FILE} 缺 pi.extensions:这不是一个 pi 扩展条目"
        ));
    };
    if patterns.is_empty() {
        return Some(format!(
            "{EXT_MANIFEST_FILE} 的 pi.extensions 为空:这不是一个 pi 扩展条目"
        ));
    }
    for p in patterns.iter().filter_map(|p| p.as_str()) {
        if p.starts_with('/') || p.contains("..") {
            return Some(format!("pi.extensions 里的 {p:?} 不许绝对路径或越级"));
        }
        let rel = p.trim_start_matches("./");
        if at(&format!("{dir}/{rel}")).is_none() {
            return Some(format!(
                "pi.extensions 里的 {p:?} 在条目子树里一个文件都没匹配上"
            ));
        }
    }
    None
}

/// 表单键位(规格自己的键入分发):与 [`on_key_skill_form`] 同一只形状 ——
/// ↑/↓ 移字段、模板字段空格切换、文本字段走通用单行输入态机。
pub fn on_key_ext_form(form: &mut ExtensionForm, k: KeyEvent) -> FormAction {
    match k.code {
        KeyCode::Up => {
            form.move_field(false);
            return FormAction::Edited;
        }
        KeyCode::Down => {
            form.move_field(true);
            return FormAction::Edited;
        }
        KeyCode::Char(' ') if form.field == ExtensionField::Template => {
            form.template = !form.template;
            return FormAction::Edited;
        }
        _ => {}
    }
    if !form.field.is_text() {
        return match k.code {
            KeyCode::Enter => FormAction::Submit,
            KeyCode::Esc | KeyCode::Char('q') => FormAction::Cancel,
            _ => FormAction::Edited,
        };
    }
    match input::on_key(form.focused_buf().expect("文本字段有缓冲"), k) {
        InputAction::Submit => FormAction::Submit,
        InputAction::Cancel => FormAction::Cancel,
        InputAction::Edited | InputAction::Ignored => FormAction::Edited,
    }
}

// ---- 渲染(共用预览/结论 + skill 表单)----

/// 脚手架浮层总入口:按阶段画一个;Idle 无浮层(几何与命中区同一 `popup_rect`)。
///
/// 表单段按 [`ScaffoldKind`] 分派到规格自己的表单件;#111 的 extension 表单
/// 与 #110 的 skill 表单**共用**预览 / 结论两屏与全部几何常量(一个字节的
/// 渲染逻辑都不复制)。
pub fn render_scaffold_overlay(frame: &mut Frame, area: Rect, app: &App) {
    match app.scaffold_flow.phase {
        ScaffoldPhase::Idle => {}
        ScaffoldPhase::Form => match app.scaffold_kind {
            ScaffoldKind::Skill => render_skill_form(frame, area, app),
            ScaffoldKind::Extension => render_ext_form(frame, area, app),
        },
        ScaffoldPhase::Review => render_scaffold_review(frame, area, app),
        ScaffoldPhase::Done => render_scaffold_done(frame, area, app),
    }
}

/// ① 表单:名称 / 描述 / 触发词 / 模板起步。当前字段带块光标,↑/↓ 移字段。
fn render_skill_form(frame: &mut Frame, area: Rect, app: &App) {
    use crate::layout::{SCAFFOLD_FORM_H, SCAFFOLD_FORM_W};
    let p = &app.palette;
    let popup = popup_rect(area, SCAFFOLD_FORM_W, SCAFFOLD_FORM_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let f = &app.scaffold_form;
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  skill 身份 = SKILL.md frontmatter 的 name;落进 skills/<名>/",
        p.dim(),
    ))];
    for field in SkillField::ALL {
        let on = field == f.field;
        if field == SkillField::Template {
            let mark = if f.template { "[x]" } else { "[ ]" };
            lines.push(Line::from(vec![
                Span::raw(format!("  {} ", field.label())),
                Span::styled(
                    format!("{mark} 从内置 agent skill 模板起步"),
                    if on { p.sel().bold() } else { p.text() },
                ),
            ]));
            continue;
        }
        let value = match field {
            SkillField::Name => &f.name,
            SkillField::Description => &f.description,
            SkillField::Triggers => &f.triggers,
            SkillField::Template => unreachable!("模板字段已在上支接走"),
        };
        let shown = if on {
            format!("{value}█")
        } else {
            value.clone()
        };
        lines.push(Line::from(vec![
            Span::raw(format!("  {} ", field.label())),
            Span::styled(
                clip(&shown, inner_w.saturating_sub(6)),
                if on { p.sel().bold() } else { p.text() },
            ),
        ]));
    }
    lines.push(Line::from(Span::styled(
        "  ↑/↓ 移字段 · 空格切模板 · Enter 拟落账预览 · Esc/q 放弃",
        p.dim(),
    )));
    let budget = popup.height.saturating_sub(3) as usize;
    lines.truncate(budget);
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    render_scaffold_block(frame, popup, lines, p, " 新建 skill · 表单 ");
}

/// ① extension 表单(#111):名称 / 描述 / 入口文件 / 模板起步。与 skill 表单
/// 同一只几何(`SCAFFOLD_FORM_W` / `SCAFFOLD_FORM_H`)、同一只字段光标与
/// ↑/↓ 习惯,只换字段与首行契约说明。
fn render_ext_form(frame: &mut Frame, area: Rect, app: &App) {
    use crate::layout::{SCAFFOLD_FORM_H, SCAFFOLD_FORM_W};
    let p = &app.palette;
    let popup = popup_rect(area, SCAFFOLD_FORM_W, SCAFFOLD_FORM_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let f = &app.ext_form;
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  pi 只扫一层 extensions/*/index.ts:条目必须自带 package.json",
        p.dim(),
    ))];
    for field in ExtensionField::ALL {
        let on = field == f.field;
        if field == ExtensionField::Template {
            let mark = if f.template { "[x]" } else { "[ ]" };
            lines.push(Line::from(vec![
                Span::raw(format!("  {} ", field.label())),
                Span::styled(
                    format!("{mark} 从内置 extension 模板起步"),
                    if on { p.sel().bold() } else { p.text() },
                ),
            ]));
            continue;
        }
        let value = match field {
            ExtensionField::Name => &f.name,
            ExtensionField::Description => &f.description,
            ExtensionField::Entry => &f.entry,
            ExtensionField::Template => unreachable!("模板字段已在上支接走"),
        };
        let shown = if on {
            format!("{value}█")
        } else {
            value.clone()
        };
        lines.push(Line::from(vec![
            Span::raw(format!("  {} ", field.label())),
            Span::styled(
                clip(&shown, inner_w.saturating_sub(6)),
                if on { p.sel().bold() } else { p.text() },
            ),
        ]));
    }
    lines.push(Line::from(Span::styled(
        "  ↑/↓ 移字段 · 空格切模板 · Enter 拟落账预览 · Esc/q 放弃",
        p.dim(),
    )));
    let budget = popup.height.saturating_sub(3) as usize;
    lines.truncate(budget);
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    render_scaffold_block(frame, popup, lines, p, " 新建 extension · 表单 ");
}

/// ② 拟落账预览(**本段一个字节都不落**):摊开将写文件清单 + 逐条路径与内容
/// 预览 + 体量;Enter 才落盘,Esc 返航。与登记两段式确认屏同一闸语义。
fn render_scaffold_review(frame: &mut Frame, area: Rect, app: &App) {
    use crate::layout::{SCAFFOLD_KEY_COMMIT, SCAFFOLD_REVIEW_H, SCAFFOLD_REVIEW_W};
    let p = &app.palette;
    let popup = popup_rect(area, SCAFFOLD_REVIEW_W, SCAFFOLD_REVIEW_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let budget = popup.height.saturating_sub(3) as usize;
    let mut body: Vec<Line> = Vec::new();
    let Some(plan) = app.scaffold_flow.plan.as_ref() else {
        body.push(Line::from(Span::styled("  拟落账不在场", p.warn())));
        render_scaffold_block(frame, popup, body, p, " 新建脚手架 · 拟落账 ");
        return;
    };
    body.push(Line::from(Span::styled(
        format!(
            "  将写 {} 个文件 · 共 {} 字节(确认后才落盘)",
            plan.writes.len(),
            plan.total_bytes()
        ),
        p.info(),
    )));
    if let Some(err) = app.scaffold_flow.error.as_ref() {
        body.push(Line::from(Span::styled(
            format!("  ⚠ {err}"),
            p.warn().bold(),
        )));
    }
    for w in &plan.writes {
        body.push(Line::from(Span::styled(
            format!("  → {}", w.path.as_str()),
            p.accent(),
        )));
        let text = String::from_utf8_lossy(&w.content);
        for ln in text.lines() {
            body.push(Line::from(Span::styled(
                format!("    {}", clip(ln, inner_w.saturating_sub(4))),
                p.text(),
            )));
        }
    }
    // 收口到预算:超量截断 + 一句「未全显」,欠量垫空,键位行恒在内层最后一行。
    let mut lines: Vec<Line> = if body.len() > budget {
        let mut kept: Vec<Line> = body.into_iter().take(budget.saturating_sub(1)).collect();
        kept.push(Line::from(Span::styled("  …(屏内未全显)", p.dim())));
        kept
    } else {
        let mut kept = body;
        while kept.len() < budget {
            kept.push(Line::from(""));
        }
        kept
    };
    lines.push(Line::from(vec![
        Span::styled(SCAFFOLD_KEY_COMMIT, p.accent().bold()),
        Span::raw("  "),
        Span::styled("Esc 返航 ", p.dim()),
    ]));
    render_scaffold_block(frame, popup, lines, p, " 新建脚手架 · 拟落账 ");
}

/// ③ 结论屏:实落清单(相对路径)+ git 纪律一句;点任意处 / Esc/q 返航。
fn render_scaffold_done(frame: &mut Frame, area: Rect, app: &App) {
    use crate::layout::{SCAFFOLD_DONE_H, SCAFFOLD_DONE_W};
    let p = &app.palette;
    let popup = popup_rect(area, SCAFFOLD_DONE_W, SCAFFOLD_DONE_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  ✓ 骨架已写入数据仓(原子落盘:tmp + rename)",
        p.info(),
    ))];
    if let Some(receipt) = app.scaffold_flow.receipt.as_ref() {
        for path in &receipt.written {
            lines.push(Line::from(Span::styled(
                format!("  落盘 {}", clip(path, inner_w.saturating_sub(8))),
                p.text(),
            )));
        }
    }
    lines.push(Line::from(Span::styled(
        "  改动走 git PR + 家长审;面板不碰版本控制。",
        p.dim(),
    )));
    let budget = popup.height.saturating_sub(3) as usize;
    lines.truncate(budget);
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled("  Esc/q 返航 ", p.dim())));
    render_scaffold_block(frame, popup, lines, p, " 脚手架 · 已写入 ");
}

/// 脚手架弹层正文收口(与册体浮层同一款式:边框 + 标题 + 底色)。
fn render_scaffold_block(
    frame: &mut Frame,
    popup: Rect,
    lines: Vec<Line<'static>>,
    p: &crate::theme::Palette,
    title: &'static str,
) {
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(title, p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Message};
    use crate::engine::FakeRunner;
    use crate::testutil::frame_display;
    use crossterm::event::KeyModifiers;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::sync::Arc;

    fn press(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }
    fn down() -> KeyEvent {
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
    }

    fn form_app() -> App {
        let runner = Arc::new(FakeRunner::new(&[("ls", 0, "{}")]));
        App::with_runner(runner)
    }

    /// 每测试一个一次性数据仓目录(进程级唯一 + 标签唯一,并行不互踩)。
    fn repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nwt-scaffold-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ---- 规格:字段形状校验 + 骨架内容 ----

    #[test]
    fn 表单_字段导航与模板切换() {
        let mut f = SkillForm::default();
        assert_eq!(f.field, SkillField::Name);
        on_key_skill_form(&mut f, down());
        assert_eq!(f.field, SkillField::Description);
        on_key_skill_form(&mut f, down());
        assert_eq!(f.field, SkillField::Triggers);
        on_key_skill_form(&mut f, down());
        assert_eq!(f.field, SkillField::Template);
        // 到底不绕回
        on_key_skill_form(&mut f, down());
        assert_eq!(f.field, SkillField::Template);
        // 模板字段:空格切换起步方式
        assert!(!f.template);
        on_key_skill_form(&mut f, press(' '));
        assert!(f.template);
        // 键入进当前文本字段
        f.field = SkillField::Name;
        on_key_skill_form(&mut f, press('a'));
        on_key_skill_form(&mut f, press('b'));
        assert_eq!(f.name, "ab");
    }

    #[test]
    fn planned_writes_空名称与空描述被拦() {
        let mut f = SkillForm {
            description: "会检索".into(),
            ..SkillForm::default()
        };
        let errs = f.planned_writes().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("名称")), "{errs:?}");
        // 补上名称,描述又空 → 换一条拦
        f.name = "web-scout".into();
        f.description = "  ".into();
        let errs = f.planned_writes().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("描述")), "{errs:?}");
    }

    #[test]
    fn planned_writes_非法名称被拦_与引擎同款口径() {
        for bad in ["a/b", "..", "a b", "na:me", "skill@1"] {
            let f = SkillForm {
                name: bad.into(),
                description: "d".into(),
                ..SkillForm::default()
            };
            assert!(f.planned_writes().is_err(), "{bad:?} 应被拦");
        }
        // 合法名(含非 ASCII、连字符、下划线、点)照常收
        let f = SkillForm {
            name: "web-scout_v2.0".into(),
            description: "d".into(),
            ..SkillForm::default()
        };
        assert!(f.planned_writes().is_ok());
    }

    #[test]
    fn 合法产出_frontmatter必填name与description_过validate相应检查() {
        let f = SkillForm {
            name: "web-scout".into(),
            description: "从网页检索资料".into(),
            ..SkillForm::default()
        };
        let writes = f.planned_writes().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].path.as_str(), "skills/web-scout/SKILL.md");
        let md = String::from_utf8(writes[0].content.clone()).unwrap();
        // 上游 validate 对 skill 的检查:frontmatter 有 name + description。
        assert!(md.starts_with("---\n"), "YAML frontmatter 开头:{md}");
        assert!(md.contains("name: web-scout"), "frontmatter 有 name:{md}");
        assert!(
            md.contains("description: 从网页检索资料"),
            "frontmatter 有 description:{md}"
        );
    }

    #[test]
    fn 空白与模板两路_产出不同骨架() {
        let base = SkillForm {
            name: "web-scout".into(),
            description: "从网页检索资料".into(),
            triggers: "查网页, 找资料".into(),
            ..SkillForm::default()
        };
        let blank = String::from_utf8(base.planned_writes().unwrap()[0].content.clone()).unwrap();
        let mut tmpl = base.clone();
        tmpl.template = true;
        let tmpl_md = String::from_utf8(tmpl.planned_writes().unwrap()[0].content.clone()).unwrap();
        assert_ne!(blank, tmpl_md, "两路骨架不同");
        assert!(blank.contains("## 触发词"), "空白路给触发词段:{blank}");
        assert!(!blank.contains("## 用法"), "空白路无用法段:{blank}");
        assert!(
            tmpl_md.contains("## 何时触发"),
            "模板路给何时触发:{tmpl_md}"
        );
        assert!(tmpl_md.contains("## 用法"), "模板路给用法:{tmpl_md}");
        // 触发词按分隔符拆成两条
        assert!(
            tmpl_md.contains("- 查网页") && tmpl_md.contains("- 找资料"),
            "{tmpl_md}"
        );
    }

    // ---- 表单流:确认前零字节、确认后确定字节(两段式先例) ----

    #[test]
    fn 表单流_确认前零字节_确认后确定字节落进skills() {
        let root = repo("twostage");
        let mut app = form_app();
        app.data_root = Some(root.to_str().unwrap().to_string());
        // 数据仓根不可用时入口指路、不开流程
        let bad = std::env::temp_dir().join(format!("nwt-scaffold-nope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bad);
        app.data_root = Some(bad.to_str().unwrap().to_string());
        app.start_scaffold();
        assert!(!app.scaffold_flow.in_flight(), "根不可用不开流程");
        assert!(
            std::fs::read_dir(&bad).is_err() || std::fs::read_dir(&bad).unwrap().next().is_none()
        );

        // 根可用:开流程填表
        app.data_root = Some(root.to_str().unwrap().to_string());
        app.start_scaffold();
        assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Form);
        app.scaffold_form.name = "web-scout".into();
        app.scaffold_form.description = "从网页检索资料".into();
        // Enter 提交 → 拟落账预览(仍不落盘)
        app.update(Message::Key(enter()));
        assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Review);
        assert!(
            std::fs::read_dir(&root).unwrap().next().is_none(),
            "确认前一个字节都不落(两段式先例)"
        );
        // Enter 确认 → 原子落盘
        app.update(Message::Key(enter()));
        assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Done);
        let written = std::fs::read_to_string(root.join("skills/web-scout/SKILL.md")).unwrap();
        assert!(
            written.starts_with("---\nname: web-scout\n"),
            "确定字节:{written}"
        );
        assert!(written.contains("description: 从网页检索资料"), "{written}");
        // 返航清状态
        app.update(Message::Key(press('q')));
        assert!(!app.scaffold_flow.in_flight(), "Done 后 Esc/q 返航");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 表单流_非法字段拦在表单里_不进预览更不落盘() {
        let root = repo("reject");
        let mut app = form_app();
        app.data_root = Some(root.to_str().unwrap().to_string());
        app.start_scaffold();
        app.scaffold_form.name = "a/b".into(); // 非法:路径分隔符
        app.scaffold_form.description = "d".into();
        app.update(Message::Key(enter()));
        assert_eq!(
            app.scaffold_flow.phase,
            ScaffoldPhase::Form,
            "拦在表单里,留可改"
        );
        assert!(
            std::fs::read_dir(&root).unwrap().next().is_none(),
            "被拒即不落盘"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 组件域n键_发起脚手架_其他域不触发() {
        use crate::app::{Domain, Screen};
        let root = repo("entry");
        let mut app = form_app();
        app.data_root = Some(root.to_str().unwrap().to_string());
        app.screen = Screen::Recipes;
        // 非组件域按 n 无 scaffolding 义(不开流程)
        app.rc_domain = Domain::Recipes;
        app.update(Message::Key(press('n')));
        assert!(!app.scaffold_flow.in_flight(), "非组件域 n 不触发脚手架");
        // 组件域按 n 开流程(键 → start_scaffold → 表单态)
        app.rc_domain = Domain::Components;
        app.update(Message::Key(press('n')));
        assert_eq!(
            app.scaffold_flow.phase,
            ScaffoldPhase::Form,
            "组件域 n 发起脚手架"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    // ---- 快照:常畅 / 紧凑两档 ----

    fn snap(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| {
            let area = f.area();
            crate::screens::render(f, area, app);
        })
        .unwrap();
        term.backend().to_string()
    }

    #[test]
    fn 脚手架浮层_表单拟落账结论_常畅紧凑两档不溢出() {
        for (w, h) in [(110u16, 30u16), (80, 30), (66, 50)] {
            let mut app = form_app();
            app.screen = crate::app::Screen::Recipes;
            // 表单态
            app.start_scaffold();
            app.scaffold_form.name = "web-scout".into();
            app.scaffold_form.description = "从网页检索资料".into();
            app.scaffold_form.triggers = "查网页".into();
            let f = snap(&app, w, h);
            assert!(f.contains("新建 skill"), "{w}×{h} 表单标题:{f}");
            assert!(f.contains("web-scout"), "{w}×{h} 名称回显:{f}");
            for ln in frame_display(&app, w, h).lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 表单有行超宽:{ln}"
                );
            }
            // 拟落账预览态
            app.update(Message::Key(enter()));
            assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Review);
            let f = snap(&app, w, h);
            assert!(f.contains("拟落账"), "{w}×{h} 预览标题:{f}");
            assert!(
                f.contains("skills/web-scout/SKILL.md"),
                "{w}×{h} 落点路径:{f}"
            );
            // 拟落账预览要摊开**内容**(frontmatter 必填两项),不只是路径。
            assert!(f.contains("name: web-scout"), "{w}×{h} 内容预览 name:{f}");
            assert!(
                f.contains("description: 从网页检索资料"),
                "{w}×{h} 内容预览 description:{f}"
            );
            assert!(f.contains("Enter 落盘"), "{w}×{h} 确认键位:{f}");
            for ln in frame_display(&app, w, h).lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 预览有行超宽:{ln}"
                );
            }
        }
    }

    // ---- extension 规格(#111):pi-extension landing 契约 ----

    /// 一份填好的 extension 表单(名称 / 描述 / 入口 / 模板全默认)。
    fn ext_form(name: &str, description: &str) -> ExtensionForm {
        ExtensionForm {
            name: name.into(),
            description: description.into(),
            ..ExtensionForm::default()
        }
    }

    #[test]
    fn extension表单_字段导航与入口默认值与模板切换() {
        let mut f = ExtensionForm::default();
        // 入口文件名预填 pi 的发现缺省(index.ts):少一个要想的字段
        assert_eq!(f.entry, DEFAULT_ENTRY);
        assert_eq!(f.field, ExtensionField::Name);
        on_key_ext_form(&mut f, down());
        assert_eq!(f.field, ExtensionField::Description);
        on_key_ext_form(&mut f, down());
        assert_eq!(f.field, ExtensionField::Entry);
        on_key_ext_form(&mut f, down());
        assert_eq!(f.field, ExtensionField::Template);
        // 到底不绕回
        on_key_ext_form(&mut f, down());
        assert_eq!(f.field, ExtensionField::Template);
        // 模板字段:空格切换起步方式
        assert!(!f.template);
        on_key_ext_form(&mut f, press(' '));
        assert!(f.template);
        // 键入进当前文本字段
        f.field = ExtensionField::Entry;
        on_key_ext_form(&mut f, press('m'));
        on_key_ext_form(&mut f, press('.'));
        assert_eq!(f.entry, "index.tsm.");
    }

    #[test]
    fn extension_合法产出_manifest与入口两件_字节确定() {
        let f = ext_form("ribbon", "把会话结论钉成一行");
        let writes = f.planned_writes().unwrap();
        assert_eq!(writes.len(), 2, "manifest + 入口文件");
        assert_eq!(writes[0].path.as_str(), "extensions/ribbon/package.json");
        assert_eq!(writes[1].path.as_str(), "extensions/ribbon/index.ts");
        // manifest 字节:pi.extensions 指着入口文件(上游样本同形)
        assert_eq!(
            String::from_utf8(writes[0].content.clone()).unwrap(),
            "{\n  \"name\": \"ribbon\",\n  \"version\": \"0.1.0\",\n  \"private\": true,\n  \"type\": \"module\",\n  \"description\": \"把会话结论钉成一行\",\n  \"pi\": { \"extensions\": [\"./index.ts\"] }\n}\n"
        );
        // 入口字节:默认工厂收 ExtensionAPI,与上游样本同形
        let ts = String::from_utf8(writes[1].content.clone()).unwrap();
        assert!(
            ts.contains("import type { ExtensionAPI } from \"@earendil-works/pi-coding-agent\";")
        );
        assert!(ts.contains("export default function (pi: ExtensionAPI) {"));
        assert!(ts.contains("  // 把会话结论钉成一行\n"));
        // 空白路没有 registerCommand 起步
        assert!(!ts.contains("registerCommand"), "空白路只给空工厂:{ts}");
    }

    #[test]
    fn extension_入口文件名与描述进manifest_换名即换落点与清单() {
        let mut f = ext_form("ribbon", "d");
        f.entry = "main.js".into();
        let writes = f.planned_writes().unwrap();
        assert_eq!(writes[1].path.as_str(), "extensions/ribbon/main.js");
        let manifest = String::from_utf8(writes[0].content.clone()).unwrap();
        assert!(
            manifest.contains("\"extensions\": [\"./main.js\"]"),
            "manifest 的 pi.extensions 跟着入口文件走:{manifest}"
        );
        // 自定义入口名也过自审(manifest 与入口两边对得上)
        assert_eq!(ext_manifest_audit("extensions/ribbon", &writes), None);
    }

    #[test]
    fn extension_空白与模板两路_产出不同入口() {
        let base = ext_form("ribbon", "把会话结论钉成一行");
        let blank = String::from_utf8(base.planned_writes().unwrap()[1].content.clone()).unwrap();
        let mut tmpl = base.clone();
        tmpl.template = true;
        let tmpl_ts = String::from_utf8(tmpl.planned_writes().unwrap()[1].content.clone()).unwrap();
        assert_ne!(blank, tmpl_ts, "两路入口不同");
        assert!(!blank.contains("registerCommand"), "空白路:{blank}");
        assert!(
            tmpl_ts.contains("pi.registerCommand(\"ribbon\", {"),
            "模板路给 registerCommand 起步:{tmpl_ts}"
        );
        assert!(
            tmpl_ts.contains("ctx.ui.notify(\"ribbon: 把活填在这里\", \"info\");"),
            "模板路的命令体:{tmpl_ts}"
        );
        // 两路的 manifest 逐字节相同(差别只在入口文件)
        assert_eq!(
            base.planned_writes().unwrap()[0],
            tmpl.planned_writes().unwrap()[0]
        );
    }

    #[test]
    fn extension_描述含引号与反斜杠_json与注释都不破() {
        let f = ext_form("ribbon", "他说\"你好\" \\ 走人");
        let writes = f.planned_writes().unwrap();
        let manifest = String::from_utf8(writes[0].content.clone()).unwrap();
        // 仍是合法 JSON(自审要能解析),且转义落在字节里
        assert!(manifest.contains("\\\"你好\\\""), "双引号转义:{manifest}");
        assert_eq!(ext_manifest_audit("extensions/ribbon", &writes), None);
        let ts = String::from_utf8(writes[1].content.clone()).unwrap();
        assert!(ts.contains("// 他说\"你好\" \\ 走人"), "注释单行化:{ts}");
    }

    #[test]
    fn extension_自审_对齐上游验收第六条_对不上就拦() {
        // 合法形状:manifest + 入口都在,pi.extensions 指着入口
        let ok = ext_form("ribbon", "d").planned_writes().unwrap();
        assert_eq!(ext_manifest_audit("extensions/ribbon", &ok), None);
        // ① 缺 manifest
        let no_manifest = vec![PlannedWrite::text(
            RepoRelPath::new("extensions/ribbon/index.ts").unwrap(),
            "x",
        )];
        assert!(ext_manifest_audit("extensions/ribbon", &no_manifest)
            .unwrap()
            .contains("package.json"));
        // ② pi.extensions 为空
        let empty_pi = vec![PlannedWrite::text(
            RepoRelPath::new("extensions/ribbon/package.json").unwrap(),
            "{\n  \"pi\": {\n    \"extensions\": []\n  }\n}\n",
        )];
        assert!(ext_manifest_audit("extensions/ribbon", &empty_pi)
            .unwrap()
            .contains("pi.extensions"));
        // ③ 清单指着一个不存在的文件(= 装上了不加载)
        let dangling = vec![
            PlannedWrite::text(
                RepoRelPath::new("extensions/ribbon/package.json").unwrap(),
                "{\n  \"pi\": {\n    \"extensions\": [\"./gone.ts\"]\n  }\n}\n",
            ),
            PlannedWrite::text(RepoRelPath::new("extensions/ribbon/index.ts").unwrap(), "x"),
        ];
        assert!(ext_manifest_audit("extensions/ribbon", &dangling)
            .unwrap()
            .contains("一个文件都没匹配上"));
        // ④ 绝对路径 / 越级
        for bad in ["/etc/passwd", "../outside.ts"] {
            let evil = vec![PlannedWrite::text(
                RepoRelPath::new("extensions/ribbon/package.json").unwrap(),
                &format!("{{\n  \"pi\": {{\n    \"extensions\": [\"{bad}\"]\n  }}\n}}\n"),
            )];
            assert!(ext_manifest_audit("extensions/ribbon", &evil)
                .unwrap()
                .contains("不许绝对路径或越级"));
        }
    }

    #[test]
    fn extension_字段不合法被拦_不进预览更不落盘() {
        // 名称:与 skill 同一条目录名判据(共享 invalid_entry_name)
        for bad in ["a/b", "..", "a b", "ribbon@1"] {
            let f = ext_form(bad, "d");
            let errs = f.planned_writes().unwrap_err();
            assert!(errs.iter().any(|e| e.contains("名称")), "{bad}:{errs:?}");
        }
        // 空名称 / 空描述各有一条拦
        let errs = ExtensionForm::default().planned_writes().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("名称")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("描述")), "{errs:?}");
        // 入口文件名:必填、单段、.ts/.js、不隐藏
        for bad in ["", "sub/index.ts", "index.txt", ".hidden.ts", "index"] {
            let mut f = ext_form("ribbon", "d");
            f.entry = bad.into();
            let errs = f.planned_writes().unwrap_err();
            assert!(
                errs.iter().any(|e| e.contains("入口文件")),
                "{bad}:{errs:?}"
            );
        }
        // 合法入口名照常收
        for ok in ["index.ts", "main.js", "ribbon.ts"] {
            let mut f = ext_form("ribbon", "d");
            f.entry = ok.into();
            assert!(f.planned_writes().is_ok(), "{ok} 应放行");
        }
    }

    // ---- 表单流(#110 的共用件被 extension 消费,不复制)----

    #[test]
    fn extension表单流_确认前零字节_确认后确定字节落进extensions() {
        let root = repo("ext-twostage");
        let mut app = form_app();
        app.data_root = Some(root.to_str().unwrap().to_string());
        // 数据仓根不可用:入口指路、不开流程
        let bad = std::env::temp_dir().join(format!("nwt-scaffold-extnope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bad);
        app.data_root = Some(bad.to_str().unwrap().to_string());
        app.start_ext_scaffold();
        assert!(!app.scaffold_flow.in_flight(), "根不可用不开流程");

        // 根可用:开流程填表
        app.data_root = Some(root.to_str().unwrap().to_string());
        app.start_ext_scaffold();
        assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Form);
        assert_eq!(app.scaffold_kind, ScaffoldKind::Extension);
        assert_eq!(app.ext_form.entry, DEFAULT_ENTRY, "入口文件名有缺省");
        app.ext_form.name = "ribbon".into();
        app.ext_form.description = "把会话结论钉成一行".into();
        // Enter 提交 → 拟落账预览(**仍不落盘**,两段式先例)
        app.update(Message::Key(enter()));
        assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Review);
        assert!(
            std::fs::read_dir(&root).unwrap().next().is_none(),
            "确认前一个字节都不落"
        );
        // Enter 确认 → 原子落盘
        app.update(Message::Key(enter()));
        assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Done);
        // 落盘后按上游 register 验收第六条(仅 --kind extension)读回判:
        // 条目子树自带 package.json、pi.extensions 非空、每条路径真实存在。
        let dir = root.join("extensions/ribbon");
        let manifest = std::fs::read_to_string(dir.join("package.json")).unwrap();
        assert!(
            manifest.contains("\"extensions\": [\"./index.ts\"]"),
            "{manifest}"
        );
        assert!(
            dir.join("index.ts").is_file(),
            "pi.extensions 指着的那条路径在条目子树里真实存在"
        );
        // 确定字节:与拟落账逐字节相同
        let plan = app.scaffold_flow.receipt.as_ref().unwrap();
        assert_eq!(
            plan.written,
            vec![
                "extensions/ribbon/package.json".to_string(),
                "extensions/ribbon/index.ts".to_string()
            ]
        );
        assert!(std::fs::read_to_string(dir.join("index.ts"))
            .unwrap()
            .contains("export default function (pi: ExtensionAPI) {"));
        // 返航:两份表单与 kind 一并复位(旧字段不冒充下一轮)
        app.update(Message::Key(press('q')));
        assert!(!app.scaffold_flow.in_flight(), "Done 后 Esc/q 返航");
        assert_eq!(app.ext_form.name, "", "返航清表单");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn extension_填错入口文件名_拦在表单里一个字节都不落() {
        let root = repo("ext-reject");
        let mut app = form_app();
        app.data_root = Some(root.to_str().unwrap().to_string());
        app.start_ext_scaffold();
        app.ext_form.name = "ribbon".into();
        app.ext_form.description = "d".into();
        app.ext_form.entry = "index.txt".into();
        app.update(Message::Key(enter()));
        assert_eq!(
            app.scaffold_flow.phase,
            ScaffoldPhase::Form,
            "入口文件名不合法拦在表单里,留可改"
        );
        assert!(
            std::fs::read_dir(&root).unwrap().next().is_none(),
            "被拒即不落盘"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 组件域e键_发起extension脚手架_n键仍是skill_两流程不互窜() {
        let root = repo("ext-entry");
        let mut app = form_app();
        app.data_root = Some(root.to_str().unwrap().to_string());
        app.screen = crate::app::Screen::Recipes;
        app.rc_domain = crate::app::Domain::Recipes;
        app.update(Message::Key(press('e')));
        assert!(!app.scaffold_flow.in_flight(), "非组件域 e 不触发");
        // 组件域 e → extension 表单
        app.rc_domain = crate::app::Domain::Components;
        app.update(Message::Key(press('e')));
        assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Form);
        assert_eq!(app.scaffold_kind, ScaffoldKind::Extension);
        app.update(Message::Key(press('q')));
        assert!(!app.scaffold_flow.in_flight());
        // 组件域 n → skill 表单(kind 跟着复位,不留在 extension)
        app.update(Message::Key(press('n')));
        assert_eq!(app.scaffold_kind, ScaffoldKind::Skill);
        assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Form);
        std::fs::remove_dir_all(&root).ok();
    }

    // ---- 快照:常畅 / 紧凑两档 ----

    #[test]
    fn extension_拟落账预览_点落盘段_与键盘同路落确定字节() {
        // 图形确认闸对鼠标同样成立(ADR-0005:渲染与命中共用同一份几何,
        // #110 的 scaffold_hits 按阶段产段,与规格无关)。
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        let root = repo("ext-mouse");
        let mut app = form_app();
        app.data_root = Some(root.to_str().unwrap().to_string());
        app.screen = crate::app::Screen::Recipes;
        app.size = ratatui::layout::Rect::new(0, 0, 110, 30);
        app.start_ext_scaffold();
        app.ext_form.name = "ribbon".into();
        app.ext_form.description = "把会话结论钉成一行".into();
        app.update(Message::Key(enter()));
        assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Review);
        // 键位行上「Enter 落盘」段的命中区(几何由 layout 与渲染同源算出)
        let hits = crate::layout::hitmap(app.size, &app, crate::layout::size_class(app.size));
        let seg = hits
            .iter()
            .find(|(_, t)| *t == crate::layout::Target::Activate)
            .expect("预览态应产「Enter 落盘」命中段");
        let at = (seg.0.x + 1, seg.0.y);
        app.update(Message::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: at.0,
            row: at.1,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(
            app.scaffold_flow.phase,
            ScaffoldPhase::Done,
            "点确认段即落盘"
        );
        assert!(root.join("extensions/ribbon/package.json").is_file());
        assert!(root.join("extensions/ribbon/index.ts").is_file());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn extension浮层_表单拟落账_常畅紧凑两档不溢出() {
        for (w, h) in [(110u16, 30u16), (80, 30), (66, 50)] {
            let mut app = form_app();
            app.screen = crate::app::Screen::Recipes;
            app.start_ext_scaffold();
            app.ext_form.name = "ribbon".into();
            app.ext_form.description = "把会话结论钉成一行".into();
            let f = snap(&app, w, h);
            assert!(f.contains("新建 extension"), "{w}×{h} 表单标题:{f}");
            assert!(f.contains("ribbon"), "{w}×{h} 名称回显:{f}");
            assert!(f.contains("index.ts"), "{w}×{h} 入口文件名缺省:{f}");
            for ln in frame_display(&app, w, h).lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 表单有行超宽:{ln}"
                );
            }
            // 拟落账预览态:摊开 manifest 与入口两件的路径 + 内容
            app.update(Message::Key(enter()));
            assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Review);
            let f = snap(&app, w, h);
            assert!(f.contains("将写 2 个文件"), "{w}×{h} 将写清单:{f}");
            assert!(
                f.contains("extensions/ribbon/package.json"),
                "{w}×{h} manifest 落点:{f}"
            );
            assert!(
                f.contains("extensions/ribbon/index.ts"),
                "{w}×{h} 入口落点:{f}"
            );
            // 预览摊开的是**内容**:pi.extensions 指着入口文件(上游硬要求)
            assert!(f.contains("\"pi\""), "{w}×{h} manifest 内容:{f}");
            assert!(
                f.contains("\"./index.ts\""),
                "{w}×{h} pi.extensions 指着入口:{f}"
            );
            assert!(
                f.contains("export default function"),
                "{w}×{h} 入口内容:{f}"
            );
            assert!(f.contains("Enter 落盘"), "{w}×{h} 确认键位:{f}");
            for ln in frame_display(&app, w, h).lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 预览有行超宽:{ln}"
                );
            }
        }
    }
}
