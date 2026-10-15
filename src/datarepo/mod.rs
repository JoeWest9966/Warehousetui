//! 数据仓/安装根访问层(spec #94 票 #101):与引擎层并列、**唯一拥有
//! NW_HOME 与数据仓根文件路径知识**的模块。
//!
//! 分工:引擎层(`crate::engine`)经 runner 的输出与 envelope 的类型看世界;
//! 本模块经文件系统看世界。两者都不下渗到 UI——屏幕层只消费意图级方法
//! (`archives()` / `manifest_raw()` / `reference_scan()`),自己拼不出一个路径,
//! 「屏幕层零路径拼接」由 [`tests::屏幕层零路径_api`] 钉死。
//!
//! 读侧两面:装配历史(清单存档,#101)+ 配方×组件目录引用图(#116)。
//! 写侧(spec #95)只剩配方写回一条面:方法收在 [`write`] 模块,准许面 /
//! 禁止面 / 方法清单由 ADR-0011 钉死。
//! 已知消费方:#116 引用完整性扫数据仓的配方×组件目录(读侧);
//! #113 配方写回(写侧)。
//!
//! 只读铁律:读侧方法一个字节都不写(`reference_scan` 由
//! [`tests::引用扫描_零写动作_文件树一个字节都不动`] 钉死);写动作全部
//! 收在 [`write`](ADR-0011),面板永不写 NW_HOME。

pub mod recipe;
pub mod write;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::engine::State;

/// `.nw-manifest` 的 schema 握手值:上游 `internal/manifest/manifest.go`
/// 的 `Schema` 常量,字段集已冻结;不认的 schema 当破损,不按旧形状猜。
const MANIFEST_SCHEMA: &str = "nw/manifest.v1";

/// 安装根(NW_HOME)的读侧句柄:枚举派、读清单存档、取清单原文。
///
/// 路径知识全关在这里:调用方给一句 NW_HOME(设置态的值,可带 `~`),
/// 拿回去的是意图级形状,不是路径。`profiles/` 布局与 `.nw-manifest`
/// 文件名是上游事实(见 [`MANIFEST_SCHEMA`] 注),模块外不再出现第二处。
#[derive(Clone, Debug)]
pub struct InstallRoot {
    root: PathBuf,
}

impl InstallRoot {
    pub fn new(nw_home: &str) -> Self {
        Self {
            root: crate::engine::discover::expand_tilde(nw_home),
        }
    }

    /// 安装根本身在吗(目录可读)。「未设/不存在」与「有根但没派」是两态,
    /// 屏幕层要能分开说(前者指路,后者空列表)。
    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }

    /// 枚举安装根下的派目录名(排序确定:多派并存时列表不跳)。
    pub fn profiles(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.profiles_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        names
    }

    /// 一个派的清单存档(意图级:三态,不带路径)。
    pub fn archive(&self, profile: &str) -> ProfileArchive {
        let Some(dir) = self.profile_dir(profile) else {
            return ProfileArchive {
                profile: profile.to_string(),
                state: ArchiveState::Missing,
            };
        };
        let path = dir.join(MANIFEST_FILE);
        let Ok(bytes) = std::fs::read(&path) else {
            // 不存在 = 上一轮没提交完(上游 `manifest.Load` 的 ErrNotFound
            // 同义:未完成态),与「读得出但坏了」分开说。
            return ProfileArchive {
                profile: profile.to_string(),
                state: ArchiveState::Missing,
            };
        };
        let Ok(text) = String::from_utf8(bytes) else {
            return ProfileArchive {
                profile: profile.to_string(),
                state: ArchiveState::Broken("清单不是 UTF-8 文本".into()),
            };
        };
        match serde_json::from_str::<ManifestFile>(&text) {
            Ok(m) if m.schema == MANIFEST_SCHEMA => ProfileArchive {
                profile: profile.to_string(),
                state: ArchiveState::Ready(ManifestArchive {
                    lock_id: m.lock_id,
                    installed_at: m.installed_at,
                    converge_hash: m.converge_hash,
                    recipe_name: m.recipe.name,
                    recipe_version: m.recipe.version,
                    runtime_key: m.runtime.runtime_key,
                    engine_nw: m.engine.nw,
                    components: m.components.len(),
                }),
            },
            Ok(m) => ProfileArchive {
                profile: profile.to_string(),
                state: ArchiveState::Broken(format!(
                    "清单 schema 不认({}):需升级 nw",
                    if m.schema.is_empty() {
                        "缺 schema 字段"
                    } else {
                        &m.schema
                    }
                )),
            },
            Err(_) => ProfileArchive {
                profile: profile.to_string(),
                state: ArchiveState::Broken("清单不是合法 JSON".into()),
            },
        }
    }

    /// 全部派的存档(按派名排序,与 [`profiles`](Self::profiles) 同序)。
    pub fn archives(&self) -> Vec<ProfileArchive> {
        self.profiles().iter().map(|p| self.archive(p)).collect()
    }

    /// 清单原文(回车看原文用):读不到(缺失态)给 `None`——不开空浮层。
    /// 破损态也給原文:解析失败恰恰是操作者最该看原文的时候。
    pub fn manifest_raw(&self, profile: &str) -> Option<String> {
        let dir = self.profile_dir(profile)?;
        std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()
    }

    fn profiles_dir(&self) -> PathBuf {
        self.root.join(PROFILES_DIR)
    }

    /// 派目录的拼装(本模块唯一的路径构造)。派名来自目录枚举,仍挡
    /// 一层分隔符/`..`:调用方若从别处喂名字,构造不出越界路径。
    fn profile_dir(&self, profile: &str) -> Option<PathBuf> {
        if profile.is_empty() || profile.contains('/') || profile.contains('\\') || profile == ".."
        {
            return None;
        }
        Some(self.profiles_dir().join(profile))
    }
}

/// NW_HOME 布局里的派目录名(上游事实,模块外不出现第二处)。
const PROFILES_DIR: &str = "profiles";
/// 安装根下的锁定集目录(引擎 `ls` 的 locks 域与装配命令交锁段的同一落点)。
const LOCKS_DIR: &str = "locks";
/// 清单落点文件名(上游 `manifest.FileName`:写在 profile 根)。
const MANIFEST_FILE: &str = ".nw-manifest";

/// 一个派的清单存档:派名 + 三态(就绪/缺失/破损)。
/// (`PartialEq`/`Eq` 供测试快照比较,见 `KeySnap`。)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileArchive {
    pub profile: String,
    pub state: ArchiveState,
}

/// 存档三态(工单 #101:「清单缺失/损坏呈降级/破损态」):
/// - `Missing` 缺失 = 上一轮没提交完(未完成态)→ 屏上呈**降级**;
/// - `Broken` 破损 = 读得出但解析不了/schema 不认 → 屏上呈**破损**。
///
/// 人话原因不带路径(路径是接缝内部的事,屏上只摆结论)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchiveState {
    Ready(ManifestArchive),
    Missing,
    Broken(String),
}

/// 就绪存档的字段(清单冻结字段集的读侧投影:lockId / installedAt /
/// convergeHash + 上下文几项)。组件只留条数——全貌在清单原文浮层。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestArchive {
    pub lock_id: String,
    pub installed_at: String,
    pub converge_hash: String,
    pub recipe_name: String,
    pub recipe_version: String,
    pub runtime_key: String,
    pub engine_nw: String,
    pub components: usize,
}

/// 装配历史次级视图的数据(票 #101):一次打开读一批,屏幕层只消费形状。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryView {
    /// 安装根本身不存在(未设态)——与「有根但一个派都没有」两态可区分:
    /// 前者走指路,后者摆空列表。
    pub root_missing: bool,
    pub rows: Vec<ProfileArchive>,
}

/// 存档三态 → 面板结果灯(工单 #101 的显式映射:缺失=降级、损坏=破损、
/// 就绪=ok)。词汇用 `engine::State` 的同一套灯,健康屏怎么画这里怎么画。
pub fn archive_lamp(state: &ArchiveState) -> State {
    match state {
        ArchiveState::Ready(_) => State::Ok,
        ArchiveState::Missing => State::Degraded,
        ArchiveState::Broken(_) => State::Broken,
    }
}

// ---- 引用图(spec #95 工单 #116):配方×组件目录的只读扫描 ----

/// 数据仓根的三个组件家(上游事实):本地组件住 `components/`,参考源条目
/// 住 `skills/` 与 `extensions/`。路径知识只在本模块,模块外不出现第二处。
///
/// 读侧 `reference_scan` 认这三个家(同名解析见 [`resolves`])。
pub(crate) const SKILLS_DIR: &str = "skills";
pub(crate) const EXTENSIONS_DIR: &str = "extensions";
const COMPONENTS_DIR: &str = "components";
/// 配方目录与文件名(上游 `nw validate` 的发现规则:`recipes/<名>/recipe.yaml`,
/// 单配方仓回落 `recipe/recipe.yaml`)。
const RECIPES_DIR: &str = "recipes";
const RECIPE_SOLO_DIR: &str = "recipe";
const RECIPE_FILE: &str = "recipe.yaml";

/// 数据仓根(作者侧)的读侧句柄:配方×组件目录的引用图。
///
/// 与 [`InstallRoot`] 并列、同一条纪律:路径知识全关在这里,调用方给一句
/// 数据仓根(设置态的值,可带 `~`),拿回去的是意图级形状。
/// **只读**:`reference_scan` 一个字节都不写(由
/// [`tests::引用扫描_零写动作_文件树一个字节都不动`] 钉死)。
#[derive(Clone, Debug)]
pub struct DataRepo {
    root: PathBuf,
}

impl DataRepo {
    pub fn new(root: &str) -> Self {
        Self {
            root: crate::engine::discover::expand_tilde(root),
        }
    }

    /// 数据仓根本身在吗(目录可读)。「根不在」由调用方的指路语义接管
    /// (`DataRootCause` 三态),本模块不摆空列表冒充「扫过了,没问题」。
    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }

    /// 引用图一次快照(只读):三个组件家 × 全部配方的组件引用,分类成四态行。
    ///
    /// 两件事只报不修(spec #95「引用完整性」条目):
    /// - **孤儿**:`skills/` `extensions/` 下的目录没进任何配方;
    /// - **断链**:配方引用的组件名在三个组件家里都查不到。
    ///
    /// 外部源引用(`source: external`)不进本图——它们查 `catalog.json`,
    /// 口径以引擎为准,面板不拿目录扫描的结果冒充编目结论。
    pub fn reference_scan(&self) -> ReferenceScan {
        let skills = dir_names(&self.root.join(SKILLS_DIR));
        let extensions = dir_names(&self.root.join(EXTENSIONS_DIR));
        let components = dir_names(&self.root.join(COMPONENTS_DIR));
        let recipes = self.recipe_refs();
        let mut used_by: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut dangling: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut external_refs = 0usize;
        for r in &recipes {
            for c in &r.refs {
                if c.external {
                    external_refs += 1;
                    continue;
                }
                if resolves(&c.name, &skills, &extensions, &components) {
                    used_by
                        .entry(c.name.clone())
                        .or_default()
                        .insert(r.name.clone());
                } else {
                    dangling
                        .entry(c.name.clone())
                        .or_default()
                        .insert(r.name.clone());
                }
            }
        }
        let mut rows: Vec<ReferenceRow> = Vec::new();
        // 断链在前(败),孤儿次之(警),正常在后(过)——问题先看见。
        for (name, by) in &dangling {
            rows.push(ReferenceRow {
                kind: RefKind::Missing,
                name: name.clone(),
                lamp: State::Fail,
                detail: format!("被 {} 引用,本体不在", join_names(by)),
                used_by: by.iter().cloned().collect(),
            });
        }
        rows.extend(dir_rows(RefKind::Skill, &skills, &used_by));
        rows.extend(dir_rows(RefKind::Extension, &extensions, &used_by));
        rows.sort_by(|a, b| {
            lamp_rank(a.lamp)
                .cmp(&lamp_rank(b.lamp))
                .then_with(|| a.kind.label().cmp(b.kind.label()))
                .then_with(|| a.name.cmp(&b.name))
        });
        ReferenceScan {
            recipes: recipes.len(),
            skills: skills.len(),
            extensions: extensions.len(),
            components: components.len(),
            external_refs,
            nothing_to_scan: skills.is_empty() && extensions.is_empty() && components.is_empty(),
            unreadable_recipes: recipes
                .iter()
                .filter(|r| r.unreadable)
                .map(|r| r.name.clone())
                .collect(),
            rows,
        }
    }

    /// 全部配方的组件引用(只读):`recipes/<名>/recipe.yaml` 各一条,单配方仓
    /// (`recipe/recipe.yaml`)按文件内的顶层 `name:` 回落取名。读不出
    /// (IO 错 / 非 UTF-8)的配方记 `unreadable`——引用图因此不完整,
    /// 屏上要明说,否则会把「没读到」冒充「没人引」。
    fn recipe_refs(&self) -> Vec<RecipeRefs> {
        let mut out: Vec<RecipeRefs> = Vec::new();
        let multi = self.root.join(RECIPES_DIR);
        if multi.is_dir() {
            for name in dir_names(&multi) {
                let path = multi.join(&name).join(RECIPE_FILE);
                out.push(read_recipe(&path, name));
            }
        }
        let solo = self.root.join(RECIPE_SOLO_DIR).join(RECIPE_FILE);
        if solo.is_file() {
            let fallback = RECIPE_SOLO_DIR.to_string();
            out.push(read_recipe(&solo, fallback));
        }
        out
    }
}

/// 引用图的一行 = 面板自有检查的一项(结果四态,与健康屏既有读法一致)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceRow {
    /// 这一行的对象:skill / extension 目录,或「被引用而查无此组件」的断链。
    pub kind: RefKind,
    /// 名字:skill/extension 目录名;断链行 = 查无此件的组件名。
    pub name: String,
    /// 四态结论:过 = 进了配方;警 = 孤儿;败 = 断链。
    pub lamp: State,
    /// 人话结论(不带路径:路径是接缝内部的事)。
    pub detail: String,
    /// 引用它的配方名(断链行 = 哪些配方引用了它);孤儿行为空。
    pub used_by: Vec<String>,
}

/// 一行的对象种类(与上游 `--kind` / 组件家同词,不另造)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefKind {
    Skill,
    Extension,
    /// 断链:配方引用了而三个组件家都查不到的组件名。
    Missing,
}

impl RefKind {
    pub fn label(self) -> &'static str {
        match self {
            RefKind::Skill => "skill",
            RefKind::Extension => "extension",
            RefKind::Missing => "断链",
        }
    }
}

/// 引用图的一次快照(工单 #116):分类好的四态行 + 扫到的规模。
///
/// `Default` 供打开视图时先占位(与 [`HistoryView`] 同款:不在场与
/// 「扫了个空」两态,屏幕层自己分开说)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReferenceScan {
    /// 扫到的配方数(含读不出的)。
    pub recipes: usize,
    pub skills: usize,
    pub extensions: usize,
    /// 本地组件目录数(只参与引用解析:它们不是孤儿判定的对象)。
    pub components: usize,
    /// 外部源引用条数:查 `catalog.json`,不进本图(口径以引擎为准)。
    pub external_refs: usize,
    /// 一个组件目录都没扫到——与「扫到了且全部正常」是两态,
    /// 不摆空列表冒充「0 个问题」。
    pub nothing_to_scan: bool,
    /// 读不出的配方名:引用图因此不完整,屏上明说。
    pub unreadable_recipes: Vec<String>,
    /// 分类好的检查行(断链在前、孤儿次之、正常在后;同类按名字排序)。
    pub rows: Vec<ReferenceRow>,
}

impl ReferenceScan {
    /// 断链行(败)。
    pub fn broken(&self) -> Vec<&ReferenceRow> {
        self.rows.iter().filter(|r| r.lamp == State::Fail).collect()
    }

    /// 孤儿行(警)。
    pub fn orphans(&self) -> Vec<&ReferenceRow> {
        self.rows.iter().filter(|r| r.lamp == State::Warn).collect()
    }

    /// 干净 = 没有断链、没有孤儿、也确实扫到了东西。
    /// 「什么都没扫到」不算干净(那是什么都没查)。
    pub fn clean(&self) -> bool {
        !self.nothing_to_scan && self.broken().is_empty() && self.orphans().is_empty()
    }
}

/// 一条配方的组件引用(扫图的中间形状,不出模块)。
struct RecipeRefs {
    name: String,
    refs: Vec<ComponentRef>,
    /// 文件读不出(IO 错 / 非 UTF-8):引用图缺这一条,不静默跳过。
    unreadable: bool,
}

/// 一个组件引用:名字 + 是不是外部源引用。
struct ComponentRef {
    name: String,
    /// `source: external`(上游配方句法):查编目,不进目录引用图。
    external: bool,
}

/// 读一条配方:读不出 → `unreadable`(不猜内容);读到了 → 取组件引用。
/// 单配方仓没有目录名可依,顶层 `name:` 兜底,再兜底 `recipe`。
fn read_recipe(path: &Path, dir_name: String) -> RecipeRefs {
    let Some(text) = std::fs::read_to_string(path).ok() else {
        return RecipeRefs {
            name: dir_name,
            refs: Vec::new(),
            unreadable: true,
        };
    };
    let name = top_value(&text, "name").unwrap_or(dir_name);
    RecipeRefs {
        name,
        refs: parse_components(&text),
        unreadable: false,
    }
}

/// 组件家目录的一批行:进了配方 = 过,没进 = 孤儿(警)。
fn dir_rows(
    kind: RefKind,
    names: &[String],
    used_by: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<ReferenceRow> {
    names
        .iter()
        .map(|name| match used_by.get(name) {
            Some(by) => ReferenceRow {
                kind,
                name: name.clone(),
                lamp: State::Pass,
                detail: format!("进了配方 {}", join_names(by)),
                used_by: by.iter().cloned().collect(),
            },
            None => ReferenceRow {
                kind,
                name: name.clone(),
                lamp: State::Warn,
                detail: format!("没进任何配方(孤儿 {})", kind.label()),
                used_by: Vec::new(),
            },
        })
        .collect()
}

/// 引用名在三个组件家里有没有落点(外部源引用不进本函数:它们查编目)。
fn resolves(name: &str, skills: &[String], extensions: &[String], components: &[String]) -> bool {
    skills
        .iter()
        .chain(extensions)
        .chain(components)
        .any(|n| n == name)
}

/// 排序权重:败(0)→ 警(1)→ 其余(2)。问题先看见。
fn lamp_rank(s: State) -> u8 {
    match s {
        State::Fail => 0,
        State::Warn => 1,
        _ => 2,
    }
}

/// 配方名列表的人话拼接(排序确定,集合来自 `BTreeSet`)。
fn join_names(names: &BTreeSet<String>) -> String {
    names.iter().cloned().collect::<Vec<_>>().join("、")
}

/// 一个目录下的子目录名(排序确定:多目录并存时列表不跳)。
/// 点号开头的目录不算组件——上游注册验收第 4 条同款宽容忽略
/// (`.bak-*` / `.gitkeep` / `.github` 一类元数据与脏文件)。
fn dir_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !n.starts_with('.'))
        .collect();
    names.sort();
    names
}

/// 从 `recipe.yaml` 文本取组件引用(块列表形态,上游样本钉死):
/// `components:` 块下每个 `- component: <名>` 条目;同条目里的
/// `source: external` 标记外部源引用。只认这一种形态——流式
/// `components: [{...}]` 与多文档/锚点形态不当配方样本,不猜。
fn parse_components(text: &str) -> Vec<ComponentRef> {
    let mut out: Vec<ComponentRef> = Vec::new();
    let mut in_block = false;
    let mut name: Option<String> = None;
    let mut external = false;
    for raw in text.lines() {
        let line = strip_comment(raw);
        if !in_block {
            if top_key(line) == Some("components") {
                in_block = true;
            }
            continue;
        }
        // 块内见到下一个顶层键即出块(components 到此为止)。
        if is_top_level(line) {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let item = trimmed.starts_with('-');
        let kv_line = if item { trimmed[1..].trim() } else { trimmed };
        if item {
            // 新条目:上一条结算(组件条目里的嵌套列表不会造出假条目:
            // 那些行不带 `component:` 键,结算时 name 已取走)。
            if let Some(n) = name.take() {
                out.push(ComponentRef { name: n, external });
            }
            external = false;
        }
        if let Some((k, v)) = kv(kv_line) {
            match k {
                "component" if !v.is_empty() => name = Some(v),
                "source" => external = v.eq_ignore_ascii_case("external"),
                _ => {}
            }
        }
    }
    if let Some(n) = name.take() {
        out.push(ComponentRef { name: n, external });
    }
    out
}

/// 行首顶层键名(无缩进、非列表项、`key:` 形态);不是顶层键给 `None`。
fn top_key(line: &str) -> Option<&str> {
    if line.starts_with(char::is_whitespace) || line.starts_with('-') {
        return None;
    }
    let (k, _) = line.split_once(':')?;
    let k = k.trim();
    (!k.is_empty()).then_some(k)
}

/// 顶层 `key: value` 的值(去引号);找不到给 `None`。
fn top_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .map(strip_comment)
        .find(|l| top_key(l) == Some(key))
        .and_then(|l| kv(l))
        .map(|(_, v)| v)
}

/// 是不是一个顶层键行(无缩进、非列表项、带冒号)——块内见到它即出块。
fn is_top_level(line: &str) -> bool {
    !line.starts_with(char::is_whitespace) && line.contains(':') && !line.starts_with('-')
}

/// `key: value` 拆一对(值去首尾空白与引号);没有冒号给 `None`。
fn kv(line: &str) -> Option<(&str, String)> {
    let (k, v) = line.split_once(':')?;
    let k = k.trim();
    if k.is_empty() {
        return None;
    }
    let v = v.trim().trim_matches('"').trim_matches('\'').trim();
    Some((k, v.to_string()))
}

/// 去掉行注释(YAML 规矩:`#` 在行首或其前是空白)。值里的 `#` 不动。
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' && (i == 0 || bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
            return &line[..i];
        }
    }
    line
}

// ---- 清单文件的读侧投影(只取本票要的字段;形状对齐上游冻结字段集) ----

#[derive(Deserialize)]
struct ManifestFile {
    #[serde(default)]
    schema: String,
    #[serde(default, rename = "lockId")]
    lock_id: String,
    #[serde(default, rename = "installedAt")]
    installed_at: String,
    #[serde(default)]
    recipe: RecipeRefFile,
    #[serde(default)]
    runtime: RuntimeFile,
    /// 只取条数(条目全貌在清单原文浮层里看)。
    #[serde(default)]
    components: Vec<serde_json::Value>,
    #[serde(default, rename = "convergeHash")]
    converge_hash: String,
    #[serde(default)]
    engine: EngineFile,
}

#[derive(Deserialize, Default)]
struct RecipeRefFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
}

#[derive(Deserialize, Default)]
struct RuntimeFile {
    #[serde(default, rename = "runtimeKey")]
    runtime_key: String,
}

#[derive(Deserialize, Default)]
struct EngineFile {
    #[serde(default)]
    nw: String,
}

/// 选中锁定集的载体文件路径:`$NW_HOME/locks/<lockId>.nwlock`(引擎 `ls` 的
/// locks 域与装配命令交锁段的同一落点)。`~` 展开走全仓唯一那份展开器。
pub fn lock_carrier(nw_home: &str, lock_id: &str) -> String {
    let home = crate::engine::discover::expand_tilde(nw_home);
    format!(
        "{}/{LOCKS_DIR}/{lock_id}.nwlock",
        home.display().to_string().trim_end_matches('/')
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// 每测试一个一次性目录(进程级唯一 + 测试名唯一,并行不互踩)。
    fn fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nwt-datarepo-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 就绪清单(字段集对齐上游 internal/manifest/manifest.go 样本)。
    const MANIFEST_OK: &str = r#"{"schema":"nw/manifest.v1","lockId":"desk-1.0.0-8f3a1c2","installedAt":"2026-10-01T12:00:00Z","recipe":{"name":"research-desk","version":"1.0.0"},"profile":"desk","runtime":{"runtimeKey":"node-24.21.0+pi-1.0.0","node":"24.21.0","pi":"1.0.0","chezmoi":"2.73.0","nodeSource":"bundled"},"components":[{"name":"retention","version":"0.4.1","landing":{"path":"nw/components/retention","kind":"pi-local-package"}},{"name":"scout","version":"0.2.0","landing":{"path":"nw/components/scout","kind":"pi-local-package"}}],"skeletonFiles":["AGENTS.md"],"configKeys":["/settings/theme"],"env":[{"name":"TAVILY_API_KEY","required":true,"present":true}],"convergeHash":"9f2c","engine":{"nw":"0.1.0","schema":"nw/manifest.v1"}}"#;

    fn write_manifest(root: &Path, profile: &str, body: &str) {
        let dir = root.join("profiles").join(profile);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".nw-manifest"), body).unwrap();
    }

    #[test]
    fn 安装根枚举派_按名排序() {
        let root = fixture("profiles");
        write_manifest(&root, "zeta", MANIFEST_OK);
        write_manifest(&root, "alpha", MANIFEST_OK);
        fs::write(root.join("profiles/stray-file"), "x").ok();
        let ir = InstallRoot::new(root.to_str().unwrap());
        assert_eq!(ir.profiles(), vec!["alpha", "zeta"], "只收目录、按名排序");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 清单存档_就绪态字段齐全() {
        let root = fixture("ready");
        write_manifest(&root, "desk", MANIFEST_OK);
        let ir = InstallRoot::new(root.to_str().unwrap());
        let a = ir.archive("desk");
        assert_eq!(a.profile, "desk");
        match &a.state {
            ArchiveState::Ready(m) => {
                assert_eq!(m.lock_id, "desk-1.0.0-8f3a1c2");
                assert_eq!(m.installed_at, "2026-10-01T12:00:00Z");
                assert_eq!(m.converge_hash, "9f2c");
                assert_eq!(m.recipe_name, "research-desk");
                assert_eq!(m.recipe_version, "1.0.0");
                assert_eq!(m.runtime_key, "node-24.21.0+pi-1.0.0");
                assert_eq!(m.engine_nw, "0.1.0");
                assert_eq!(m.components, 2, "组件条数");
            }
            other => panic!("应就绪,实得 {other:?}"),
        }
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 清单缺失_降级态() {
        let root = fixture("missing");
        fs::create_dir_all(root.join("profiles/desk")).unwrap(); // 派目录在,清单不在
        let ir = InstallRoot::new(root.to_str().unwrap());
        assert!(
            matches!(ir.archive("desk").state, ArchiveState::Missing),
            "缺清单 = 上一轮没提交完的未完成态"
        );
        assert_eq!(archive_lamp(&ArchiveState::Missing), State::Degraded);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 清单坏json_破损态() {
        let root = fixture("broken-json");
        write_manifest(&root, "desk", "{not json");
        let ir = InstallRoot::new(root.to_str().unwrap());
        match &ir.archive("desk").state {
            ArchiveState::Broken(why) => assert!(why.contains("JSON"), "人话点题:{why}"),
            other => panic!("应破损,实得 {other:?}"),
        }
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 清单schema不认_破损态() {
        let root = fixture("broken-schema");
        let m = MANIFEST_OK.replace("nw/manifest.v1", "nw/manifest.v2");
        write_manifest(&root, "desk", &m);
        let ir = InstallRoot::new(root.to_str().unwrap());
        match &ir.archive("desk").state {
            ArchiveState::Broken(why) => {
                assert!(why.contains("schema"), "点题:{why}");
                assert!(why.contains("nw/manifest.v2"), "带上收到的 schema:{why}");
                assert!(!why.contains("profiles"), "破损原因不带路径:{why}");
            }
            other => panic!("应破损,实得 {other:?}"),
        }
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 多派三态并存_逐个不互窜() {
        let root = fixture("mixed");
        write_manifest(&root, "desk", MANIFEST_OK);
        fs::create_dir_all(root.join("profiles/lab")).unwrap(); // 缺失
        write_manifest(&root, "broken", "{oops"); // 破损
        let ir = InstallRoot::new(root.to_str().unwrap());
        let got: Vec<(String, bool, bool)> = ir
            .archives()
            .into_iter()
            .map(|a| {
                (
                    a.profile,
                    matches!(a.state, ArchiveState::Ready(_)),
                    matches!(a.state, ArchiveState::Broken(_)),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("broken".into(), false, true),
                ("desk".into(), true, false),
                ("lab".into(), false, false),
            ],
            "三态各归各,按派名排序"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 清单原文_读得到与读不到() {
        let root = fixture("raw");
        write_manifest(&root, "desk", MANIFEST_OK);
        fs::create_dir_all(root.join("profiles/lab")).unwrap();
        let ir = InstallRoot::new(root.to_str().unwrap());
        assert_eq!(ir.manifest_raw("desk").as_deref(), Some(MANIFEST_OK));
        assert_eq!(ir.manifest_raw("lab"), None, "缺失态没有原文,不开空浮层");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 安装根不存在_与有根没派两态可区分() {
        let root = fixture("no-root");
        let ir = InstallRoot::new(root.join("nope").to_str().unwrap());
        assert!(!ir.exists());
        assert!(ir.profiles().is_empty());
        assert!(ir.archives().is_empty());
        // 有根没派:根在、派表空——两态在 exists() 上分开
        fs::create_dir_all(&root).unwrap();
        let ir2 = InstallRoot::new(root.to_str().unwrap());
        assert!(ir2.exists());
        assert!(ir2.profiles().is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 派名带分隔符_构造不出越界路径() {
        let root = fixture("traversal");
        let ir = InstallRoot::new(root.to_str().unwrap());
        for bad in ["../etc", "a/b", "a\\b", ".."] {
            assert!(
                matches!(ir.archive(bad).state, ArchiveState::Missing),
                "{bad} 读不到任何东西"
            );
            assert_eq!(ir.manifest_raw(bad), None);
        }
        fs::remove_dir_all(&root).ok();
    }

    /// 引用图夹具(#116):一个数据仓里同时放**孤儿 / 断链 / 正常**三态。
    /// 组件家照上游事实摆:本地组件 `components/`、作者侧 skill/extension
    /// 落 `skills/` `extensions/`;配方照 Data-001 样本的块列表形态写。
    fn repo_fixture(tag: &str) -> PathBuf {
        let root = fixture(tag);
        write_file(
            &root,
            "skills/web-scout/SKILL.md",
            "---\nname: web-scout\n---\n",
        );
        write_file(&root, "skills/lonely/SKILL.md", "---\nname: lonely\n---\n");
        write_file(
            &root,
            "extensions/ribbon/package.json",
            "{\n  \"pi\": {}\n}\n",
        );
        write_file(
            &root,
            "extensions/dropped/package.json",
            "{\n  \"pi\": {}\n}\n",
        );
        write_file(
            &root,
            "components/scout/component.yaml",
            "kind: pi-local-package\n",
        );
        write_file(
            &root,
            "components/retention/component.yaml",
            "kind: pi-local-package\n",
        );
        write_file(&root, "recipes/research-desk/recipe.yaml", RECIPE_DESK);
        write_file(&root, "recipes/lab/recipe.yaml", RECIPE_LAB);
        root
    }

    fn write_file(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    /// 递归快照(相对路径, 长度, mtime):写动作(新建 / 改内容 / 碰时间)都会被抓到。
    fn tree_snapshot(root: &Path) -> Vec<(String, u64, std::time::SystemTime)> {
        let mut out = Vec::new();
        walk_tree(root, root, &mut out);
        out.sort();
        out
    }

    fn walk_tree(root: &Path, dir: &Path, out: &mut Vec<(String, u64, std::time::SystemTime)>) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().to_string();
            let md = e.metadata().unwrap();
            out.push((rel.clone(), md.len(), md.modified().unwrap()));
            if md.is_dir() {
                walk_tree(root, &p, out);
            }
        }
    }

    /// 夹具配方 A:正常 skill/extension/本地组件 + 一个断链(`ghost`)。
    const RECIPE_DESK: &str = r#"apiVersion: nw/v1
kind: recipe
name: research-desk
version: 1.0.0
components:
  - component: scout          # 本地组件:components/scout
    version: 1.0.0
  - component: web-scout      # 作者侧 skill 目录
    version: 0.1.0
  - component: ribbon
    params:
      dirs:
        - notes
  - component: ghost          # 断链:三个组件家都查不到
    version: 9.9.9
models:
  defaults:
    theme: dark
"#;

    /// 夹具配方 B:正常本地组件 + 与 A 同名的断链(按名合并)+ 一条外部源引用。
    const RECIPE_LAB: &str = r#"kind: recipe
name: lab
components:
  - component: retention
    version: 0.4.1
  - component: ghost
    version: 1.0.0
  - component: hello-world
    source: external
"#;

    #[test]
    fn 引用图_孤儿断链正常三态分类正确() {
        let root = repo_fixture("refs");
        let scan = DataRepo::new(root.to_str().unwrap()).reference_scan();
        assert_eq!(scan.recipes, 2, "扫到两条配方");
        assert_eq!((scan.skills, scan.extensions, scan.components), (2, 2, 2));
        assert_eq!(
            scan.external_refs, 1,
            "外部源引用不进本图(查编目,口径以引擎为准)"
        );
        assert!(scan.unreadable_recipes.is_empty(), "两条配方都读得出");
        assert!(!scan.clean(), "有断链有孤儿:不干净");
        // 断链:一个组件名一行,两个配方都点名它 → 合并成一行、双方都列
        let broken = scan.broken();
        assert_eq!(broken.len(), 1, "断链按组件名合并:{broken:?}");
        assert_eq!(broken[0].name, "ghost");
        assert_eq!(broken[0].kind, RefKind::Missing);
        assert_eq!(broken[0].lamp, State::Fail);
        assert_eq!(broken[0].used_by, vec!["lab", "research-desk"]);
        assert!(
            broken[0].detail.contains("lab、research-desk"),
            "人话点名是哪几条配方:{}",
            broken[0].detail
        );
        // 孤儿:skill 一个、extension 一个,按名排序
        let orphans: Vec<&str> = scan.orphans().iter().map(|r| r.name.as_str()).collect();
        assert_eq!(orphans, vec!["dropped", "lonely"], "两个孤儿各归各");
        for o in scan.orphans() {
            assert_eq!(o.lamp, State::Warn);
            assert!(o.used_by.is_empty(), "孤儿没有引用方");
            assert!(o.detail.contains("没进任何配方"), "人话点题:{}", o.detail);
        }
        // 正常态:进了配方的 skill/extension 呈过
        let pass: Vec<&str> = scan
            .rows
            .iter()
            .filter(|r| r.lamp == State::Pass)
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(pass, vec!["ribbon", "web-scout"], "进了配方的是过");
        // 本地组件不当断链、也不当孤儿(它们只参与引用解析)
        for n in ["scout", "retention"] {
            assert!(
                !scan.rows.iter().any(|r| r.name == n),
                "{n} 是本地组件:不进孤儿/断链判定"
            );
        }
        // 顺序:败在前、警居中、过在后(问题先看见)
        assert_eq!(
            scan.rows.iter().map(|r| r.lamp).collect::<Vec<_>>(),
            vec![
                State::Fail,
                State::Warn,
                State::Warn,
                State::Pass,
                State::Pass
            ]
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 引用扫描_零写动作_文件树一个字节都不动() {
        let root = repo_fixture("refs-ro");
        let before = tree_snapshot(&root);
        let scan = DataRepo::new(root.to_str().unwrap()).reference_scan();
        assert!(!scan.rows.is_empty(), "确实扫到了东西(不是空转)");
        assert_eq!(
            tree_snapshot(&root),
            before,
            "扫完文件树逐字节不动(零写动作)"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 引用图_读不出的配方记下来_不把没读到冒充没人引() {
        let root = repo_fixture("refs-unreadable");
        // 一条读不出的配方(非 UTF-8):引用图缺它,屏上要明说
        let dir = root.join("recipes/broken");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(RECIPE_FILE), [0xff, 0xfe, 0x00]).unwrap();
        let scan = DataRepo::new(root.to_str().unwrap()).reference_scan();
        assert_eq!(scan.unreadable_recipes, vec!["broken"], "读不出的配方记名");
        assert_eq!(scan.recipes, 3, "读不出的也计入规模");
        assert_eq!(scan.broken().len(), 1, "其余分类不被打乱:{scan:?}");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 引用图_无可扫对象与全正常是两态() {
        // 扫了个空:一个组件家都没有——不摆空列表冒充「0 个问题」
        let empty = fixture("refs-empty");
        let scan = DataRepo::new(empty.to_str().unwrap()).reference_scan();
        assert!(scan.nothing_to_scan, "三个组件家都不在");
        assert!(scan.rows.is_empty());
        assert!(!scan.clean(), "什么都没扫到不算干净");
        fs::remove_dir_all(&empty).ok();
        // 全正常:扫到了且全部进配方
        let root = fixture("refs-clean");
        write_file(&root, "skills/a/SKILL.md", "x");
        write_file(
            &root,
            "recipes/only/recipe.yaml",
            "kind: recipe\nname: only\ncomponents:\n  - component: a\n",
        );
        let scan = DataRepo::new(root.to_str().unwrap()).reference_scan();
        assert!(!scan.nothing_to_scan);
        assert!(scan.clean(), "全部进配方 = 干净");
        assert_eq!(scan.rows.len(), 1);
        assert_eq!(scan.rows[0].lamp, State::Pass);
        assert_eq!(scan.rows[0].used_by, vec!["only"]);
        assert!(
            scan.rows[0].detail.contains("only"),
            "人话点名配方:{}",
            scan.rows[0].detail
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 引用图_单配方仓_注释与引号都不啃错() {
        let root = fixture("refs-solo");
        write_file(
            &root,
            "components/scout/component.yaml",
            "kind: pi-local-package\n",
        );
        write_file(
            &root,
            "recipe/recipe.yaml",
            "kind: recipe\nname: solo-recipe\ncomponents:\n  - component: \"scout\"   # 带引号与行注释\n    version: 1.0.0\n  # 注释里带冒号:不该把 components 块结束\n  - component: ghost\nmodels:\n  defaults:\n    theme: dark\n",
        );
        let scan = DataRepo::new(root.to_str().unwrap()).reference_scan();
        assert_eq!(scan.recipes, 1, "单配方仓(`recipe/recipe.yaml`)也扫");
        let broken = scan.broken();
        assert_eq!(broken.len(), 1, "块在 models: 处正确结束:{broken:?}");
        assert_eq!(broken[0].name, "ghost");
        assert_eq!(broken[0].used_by, vec!["solo-recipe"], "配方名取顶层 name:");
        assert!(
            !scan.rows.iter().any(|r| r.name == "scout"),
            "引号剥掉了:scout 解析得出来"
        );
        fs::remove_dir_all(&root).ok();
    }

    /// 接缝纪律(工单 #101 验收):屏幕层零路径 API——装配屏渲染只消费
    /// `HistoryView` 形状,不知道派目录怎么拼、清单文件叫什么。这里扫
    /// 屏幕源码钉死:路径知识只有 datarepo 一个家。
    #[test]
    fn 屏幕层零路径_api() {
        let src = include_str!("../screens/assemble.rs");
        for needle in ["PathBuf", "Path::new", PROFILES_DIR] {
            assert!(
                !src.contains(needle),
                "装配屏不得出现 `{needle}`:NW_HOME 路径拼接归 datarepo(接缝纪律)"
            );
        }
    }

    /// 同一条接缝纪律的健康屏半边(#116):引用完整性屏只消费
    /// `ReferenceScan` 形状——不做路径拼接(无 `PathBuf` / `join` / `std::fs`),
    /// 也不出现组件家与配方文件的字面目录名(那些知识只有 datarepo 有)。
    #[test]
    fn 健康屏零路径_api() {
        let src = include_str!("../screens/health.rs");
        for needle in [
            "PathBuf",
            "Path::new",
            "std::fs",
            "read_dir",
            "read_to_string",
            "\"skills\"",
            "\"extensions\"",
            "\"components\"",
            "\"recipes\"",
            "\"recipe.yaml\"",
        ] {
            assert!(
                !src.contains(needle),
                "健康屏不得出现 `{needle}`:数据仓路径拼接归 datarepo(接缝纪律)"
            );
        }
    }

    /// 禁字面量的小帮手(与 [`tests::健康屏零路径_api`] 同一套判据,屏名进
    /// 失败信息:哪一屏哪一条要说清)。
    fn 禁字面量(screen: &str, src: &str, needles: &[&str]) {
        for needle in needles {
            assert!(
                !src.contains(needle),
                "{screen}不得出现 `{needle}`:数据仓路径知识只有 datarepo 一个家(接缝纪律)"
            );
        }
    }

    /// 同一条接缝纪律的**写侧**半边(code-review H1):配方屏只消费意图级
    /// 形状——数据仓的组件家与配方文件名是 datarepo 才有的路径知识,屏幕层
    /// 自己拼不出一个路径。禁字面量与 [`tests::健康屏零路径_api`] 同款。
    ///
    /// 只扫生产码(`#[cfg(test)]` 之前):测试夹具要造 fixture 数据仓,免不了
    /// `std::fs` 与落点字面量——那是测试自己的事,不是屏幕层的事。
    ///
    /// 配方屏不禁两条**路径形状**的家目录字面量(`\"skills/` / `\"extensions/`):
    /// 它身上的 `skills/<仓库>/<条目>/` 是 #115 **派内**落点的只读呈现
    /// (上游 external-sources.md 的口径,与数据仓组件家不是一回事,也不归
    /// datarepo 管);禁它会逼着把只读知识也搬进接缝,那是扩范围。
    #[test]
    fn 写侧零路径_api() {
        let common: &[&str] = &[
            "PathBuf",
            "Path::new",
            "std::fs",
            "read_dir",
            "read_to_string",
            "\"skills\"",
            "\"extensions\"",
            "\"components\"",
            "\"recipes\"",
            "\"recipe.yaml\"",
        ];
        let recipes_src = include_str!("../screens/recipes.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        禁字面量("配方屏", recipes_src, common);
    }
}
