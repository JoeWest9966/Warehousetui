//! envelope:`nw` stdout 的唯一输出面,字段级契约定型。
//!
//! 铁律(来自 Nous-Warehouse #9 与调研底稿):
//! - stdout 永远是**一份** JSON envelope,人话/进度/警告全走 stderr;
//! - `ok ≡ (退出码 == 0)`;
//! - 未知字段容忍(前向兼容),绝不 deny_unknown_fields;
//! - details 可含任意 JSON,原样透传给 UI 展示,不回显密钥。

use serde::Deserialize;
use serde_json::Value;

use super::State;

/// 契约守卫(#28):面板**认识**的 envelope schema 集合。
///
/// 只收面板认识其 data 形状、或明确知道其存在的引擎 schema。不在集合里的
/// 一律走「需升级 nw」证据:不 panic、不按旧形状降级解析(`data_as` 直接
/// 返回 None)。这是面板自己的契约知识,不是引擎能力表(那是 #29 的探测门)。
///
/// # 认识形状 ≠ 准许代跑(ADR-0009)
///
/// 本集合答的是「我听不听得懂这个 envelope」,与「面板会不会把这个命令
/// 跑起来」是两件事:编目册 14 个 schema 全在集合里(契约对齐),但其中
/// `key get` / `subscription get` 的回音面板**只给建议命令、不代跑**
/// (ADR-0009 §1:上游唯一回显本体的两个出口)。两层分工不被混淆。
///
/// 前 8 个是跟随引擎第 1–7 轮的既有面(调研底稿 §3 的 `nw/<cmd>.v1` 全集);
/// 后 14 个是第 8 轮跟随同步(spec #50 / 工单 #51)补上的四组编目命令面,
/// 逐个列在 `notes/51-follow-sync-diff.md` §1 的契约表里——**上游
/// `cli.go` 的 `Schema*` 常量是唯一真相源,加一个 schema 必须能指回它**。
const KNOWN_SCHEMAS: [&str; 22] = [
    // ---- 既有 8 个(第 1–7 轮) ----
    "nw/validate.v1",
    "nw/up.v1",
    "nw/doctor.v1",
    "nw/bake.v1",
    "nw/recipe-new.v1",
    "nw/ls.v1",
    "nw/source-register.v1",
    "nw/source-recheck.v1",
    // ---- 第 8 轮新增 14 个:四组编目(vps 3 / key 4 / subscription 4 / tag 3) ----
    // vps:register 两段式 + ls 只读 + rm;编目 <root>/vps.json
    "nw/vps-register.v1",
    "nw/vps-ls.v1",
    "nw/vps-rm.v1",
    // key:register(stdin 本体)+ ls 投影 + **get 是唯一回显处** + rm;编目 keys.json
    "nw/key-register.v1",
    "nw/key-ls.v1",
    "nw/key-get.v1",
    "nw/key-rm.v1",
    // subscription:register(stdin 本体)+ ls 投影(到期三态)+ get 唯一回显处 + rm;编目 subscriptions.json
    "nw/subscription-register.v1",
    "nw/subscription-ls.v1",
    "nw/subscription-get.v1",
    "nw/subscription-rm.v1",
    // tag:add(追加合并,无 --yes)+ ls 反查 + rm 两态;编目 tags.json
    "nw/tag-add.v1",
    "nw/tag-ls.v1",
    "nw/tag-rm.v1",
];

/// schema 是否在面板认识的集合内(契约守卫的判据)。
fn is_known_schema(schema: &str) -> bool {
    KNOWN_SCHEMAS.contains(&schema)
}

/// 「需升级 nw」五字段证据的**唯一措辞工厂**(envelope 与清单文件共用)。
///
/// `subject` 说明这是哪种契约面不认得(envelope / 清单),`rc` 是引擎的
/// 退出码——读文件没有退出码可报,那一格留空(`None`),不编一个 rc 冒充
/// 引擎说的(#101:清单的契约守卫与 envelope 的同款,但 rc 这一格不许造假)。
fn schema_guard_evidence(subject: &str, schema: &str, nw: &str, rc: Option<i32>) -> Evidence {
    Evidence {
        code: Some("SCHEMA_UNKNOWN".into()),
        exit_code: rc,
        message: format!("需升级 nw:面板不认识{subject} \"{schema}\"(引擎 nw {nw})"),
        hint: Some("升级 nw 与本面板,使两端 schema 对齐;收到的原文在 details".into()),
        details: Some(serde_json::json!({
            "schema": schema,
            "nw": nw,
        })),
    }
}

/// 清单文件(`.nw-manifest`)的契约守卫证据(spec #94 / 工单 #101)。
///
/// 与 [`Envelope::apply_schema_guard`] 同一处措辞工厂、同一种结论:
/// 清单 schema 也是上游契约面,面板只认
/// [`crate::datarepo::MANIFEST_SCHEMA`];收到不认识的 schema 时合成
/// 五字段证据(带收到的原文与引擎版本),**不猜、不按旧形状降级解析**。
/// 差别只有一处:读文件没有引擎退出码,`exit_code` 留空。
pub fn manifest_schema_guard(schema: &str, nw: &str) -> Evidence {
    schema_guard_evidence("清单 schema", schema, nw, None)
}

/// 错误/警告条目五字段。warnings 的 code 可缺。
#[derive(Clone, Debug, Deserialize)]
pub struct Evidence {
    #[serde(default)]
    pub code: Option<String>,
    /// 引擎的字段名是 `exitCode`(camelCase,契约冻结)。少了这一条 rename,
    /// 证据五字段的第四列永远解析成 None——屏幕上的 `rc0` 是面板自己
    /// 编的,不是引擎说的(#40 验收「显示引擎证据」钉的是原文)。
    #[serde(default, rename = "exitCode")]
    pub exit_code: Option<i32>,
    pub message: String,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub details: Option<Value>,
}

/// 顶层 envelope。`data` 先留 Value,由各命令的定型结构消费。
#[derive(Clone, Debug, Deserialize)]
pub struct Envelope {
    pub schema: String,
    pub nw: String,
    pub ok: bool,
    #[serde(default)]
    pub errors: Vec<Evidence>,
    #[serde(default)]
    pub warnings: Vec<Evidence>,
    pub data: Value,
    /// 契约守卫证据(#28):schema 不在认识集合时由 [`Envelope::apply_schema_guard`]
    /// 在解析点合成,非空即「按旧形状解析」被禁止。serde 跳过——它是面板的
    /// 结论,不是引擎的原话,不参与反序列化。
    #[serde(skip)]
    pub schema_guard: Option<Evidence>,
}

impl Envelope {
    /// 契约守卫:这次给的 schema 面板认不认识。
    pub fn schema_known(&self) -> bool {
        is_known_schema(&self.schema)
    }

    /// 契约守卫:schema 不认识时合成「需升级 nw」五字段证据(带收到的
    /// schema 原文与引擎版本),认识时什么都不做。由解析点(runner 的
    /// `from_parts`)携 rc 调用一次;幂等,重复调用不覆盖。
    pub fn apply_schema_guard(&mut self, rc: i32) {
        if self.schema_guard.is_some() || self.schema_known() {
            return;
        }
        self.schema_guard = Some(schema_guard_evidence(
            "envelope schema",
            &self.schema,
            &self.nw,
            Some(rc),
        ));
    }

    /// 按 schema 拆出 data 的定型视图;schema 不在认识集合(契约守卫)
    /// 或形状不匹配时返回 None——绝不按旧形状降级解析(UI 走证据展示)。
    pub fn data_as<T: for<'de> Deserialize<'de>>(&self) -> Option<T> {
        if !self.schema_known() {
            return None;
        }
        serde_json::from_value(self.data.clone()).ok()
    }

    /// 「人话一句」:第一条错误的 message,没有就退到 warnings,再没有给成功语。
    pub fn headline(&self) -> &str {
        if let Some(e) = self.errors.first() {
            return &e.message;
        }
        if let Some(w) = self.warnings.first() {
            return &w.message;
        }
        "完成"
    }
}

// ---- validate ----

#[derive(Clone, Debug, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub status: String, // pass | warn | fail | skip
    pub message: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ValidateSummary {
    pub total: u32,
    pub pass: u32,
    pub warn: u32,
    pub fail: u32,
    pub skip: u32,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ValidateData {
    pub target: String,
    pub results: Vec<CheckResult>,
    pub summary: ValidateSummary,
}

// ---- up ----

#[derive(Clone, Debug, Deserialize)]
pub struct UpData {
    pub profile: String,
    #[serde(default, rename = "profileDir")]
    pub profile_dir: Option<String>,
    #[serde(default, rename = "runtimeKey")]
    pub runtime_key: Option<String>,
    #[serde(default)]
    pub components: Vec<String>,
    #[serde(default, rename = "skeletonFiles")]
    pub skeleton_files: Vec<String>,
    #[serde(default, rename = "configKeys")]
    pub config_keys: Vec<String>,
    #[serde(default, rename = "envPresent")]
    pub env_present: u32,
    #[serde(default, rename = "envMissing")]
    pub env_missing: Vec<String>,
    #[serde(default, rename = "convergeHash")]
    pub converge_hash: Option<String>,
    #[serde(default, rename = "lockId")]
    pub lock_id: Option<String>,
    #[serde(default)]
    pub upgraded: Option<bool>,
    #[serde(default, rename = "installedAt")]
    pub installed_at: Option<String>,
}

// ---- doctor ----

#[derive(Clone, Debug, Deserialize)]
pub struct DoctorResult {
    pub surface: String,
    pub status: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ManifestBrief {
    #[serde(default)]
    pub schema: Option<String>,
    #[serde(default, rename = "lockId")]
    pub lock_id: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub recipe: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, rename = "runtimeKey")]
    pub runtime_key: Option<String>,
    #[serde(default, rename = "installedAt")]
    pub installed_at: Option<String>,
    #[serde(default, rename = "convergeHash")]
    pub converge_hash: Option<String>,
    #[serde(default)]
    pub engine: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DoctorData {
    pub results: Vec<DoctorResult>,
    pub exit: i32,
    #[serde(default)]
    pub converged: Option<bool>,
    #[serde(default)]
    pub manifest: Option<ManifestBrief>,
}

// ---- ls ----

#[derive(Clone, Debug, Deserialize)]
pub struct RecipeRow {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    /// 引擎 `ls` 给的仓内相对路径(如 `recipes/research-desk/recipe.yaml`;
    /// 老引擎可能不给 → None)。validate 档按派名拼配方文件要用它。
    #[serde(default)]
    pub path: Option<String>,
}

/// 组件依赖的一项(引擎 `nw ls` 的 `components[].depends[]` 原文:
/// 对象 `{name, version}`,不是裸字符串)。
#[derive(Clone, Debug, Deserialize)]
pub struct ComponentDep {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
}

impl ComponentDep {
    /// 展示串:`name@version`;引擎没给版本就只给名字。
    pub fn label(&self) -> String {
        match self.version.as_deref() {
            Some(v) => format!("{}@{}", self.name, v),
            None => self.name.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ComponentRow {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    /// 组件目录相对数据仓根的路径(`nw ls` 原文)。
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub depends: Vec<ComponentDep>,
    /// 声明了第三方依赖随仓走(validate 检查 `component.vendored_declared`);
    /// 这是引擎自己的字段名,面板不另译一个词,取原文呈现。
    #[serde(default)]
    pub vendored: bool,
    /// 用到这个组件的配方名单(`nw ls` 原文)。
    #[serde(default, rename = "usedBy")]
    pub used_by: Vec<String>,
}

/// `nw ls` 外部源域的一行(#35/#37):**一条记录 = 一个仓库**。
///
/// - `kind` = `skill` | `extension`(记录落在编目哪条数组);
/// - `category` 是**多级**类别(单层内禁 "/",空数组 = 未分类——引擎不落
///   「未分类」字面量,由展示方兜底);
/// - `entries` 是一次扫描得出的条目清单(名字 + 路径)。引擎侧 ls 用
///   `SourceEntry`、register 用 `CatalogEntry`,两者形状同为「名字 + 路径」
///   —— 面板这里共用 [`RegisterEntry`] 一个拼写,不造第二个同形类型。
#[derive(Clone, Debug, Deserialize)]
pub struct SourceRow {
    pub name: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub category: Vec<String>,
    #[serde(default)]
    pub entries: Vec<RegisterEntry>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProfileRow {
    pub name: String,
    /// ok | degraded | drift | broken,与 doctor rc 一一对应。
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default, rename = "lockId")]
    pub lock_id: Option<String>,
    #[serde(default, rename = "installedAt")]
    pub installed_at: Option<String>,
}

/// 配方身份的对象形态(引擎 `nw ls` 的 `locks[].recipe` 契约)。
#[derive(Clone, Debug, Deserialize)]
pub struct RecipeBrief {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

impl RecipeBrief {
    /// 展示串:`name@version`;缺名给「——」(不推导、不补全)。
    pub fn label(&self) -> String {
        match (self.name.as_deref(), self.version.as_deref()) {
            (Some(n), Some(v)) => format!("{n}@{v}"),
            (Some(n), None) => n.to_string(),
            (None, _) => "——".to_string(),
        }
    }
}

/// 锁定集里的配方身份。
///
/// 引擎真输出是对象 `{name, version}`;仓内部分夹具与旧口径写成裸字符串
/// (`"recipe": "research-desk"`)。两种形状都容忍(untagged)——展示只取
/// [`RecipeRef::label`],缺字段由渲染层给「——」,不因形状差异整份 ls 解析失败。
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum RecipeRef {
    Brief(RecipeBrief),
    Name(String),
}

impl RecipeRef {
    /// 展示串:对象形走 [`RecipeBrief::label`],裸字符串形原样给。
    pub fn label(&self) -> String {
        match self {
            RecipeRef::Brief(b) => b.label(),
            RecipeRef::Name(n) => n.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct LockRow {
    #[serde(default, rename = "lockId")]
    pub lock_id: Option<String>,
    #[serde(default)]
    pub recipe: Option<RecipeRef>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, rename = "bakedAt")]
    pub baked_at: Option<String>,
    /// 引擎 `ls` 不输出锁定集路径(它只读锁目录,不读锁正文)。
    #[serde(default)]
    pub path: Option<String>,
    /// 资产数(`nw ls` 给整数)。
    #[serde(default)]
    pub assets: Option<u64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LsData {
    pub domain: String,
    #[serde(default)]
    pub recipes: Vec<RecipeRow>,
    #[serde(default)]
    pub components: Vec<ComponentRow>,
    #[serde(default)]
    pub sources: Vec<SourceRow>,
    #[serde(default)]
    pub profiles: Vec<ProfileRow>,
    #[serde(default)]
    pub locks: Vec<LockRow>,
}

// ---- source register(`nw/source-register.v1`,工单 #30/#31) ----

/// 引擎扫到的一个条目(名字 + 路径)。
#[derive(Clone, Debug, Deserialize)]
pub struct RegisterEntry {
    pub name: String,
    #[serde(default)]
    pub path: String,
}

/// 验收一条结论;`id` 是冻结面、`message` 措辞不承诺。
/// `status` = pass | fail | skip(「未确认」= `register.confirmed` 的
/// skip,设计内的待定态,不是错误)。
#[derive(Clone, Debug, Deserialize)]
pub struct RegisterAcceptance {
    pub id: String,
    pub status: String,
    pub message: String,
    #[serde(default)]
    pub path: Option<String>,
}

/// register 的 data 形状(引擎自测夹具口径):预审与落账共用,
/// `written` 区分「文件不落」与真写入。
#[derive(Clone, Debug, Deserialize)]
pub struct RegisterData {
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub category: Vec<String>,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub entries: Vec<RegisterEntry>,
    #[serde(default)]
    pub acceptance: Vec<RegisterAcceptance>,
    #[serde(default, rename = "catalogPath")]
    pub catalog_path: String,
    #[serde(default)]
    pub written: bool,
    #[serde(default)]
    pub updated: bool,
    #[serde(default)]
    pub action: String,
}

impl RegisterData {
    /// 验收里有 fail(引擎铁律:有 fail 就不得进入落账段)。
    pub fn has_fail(&self) -> bool {
        self.acceptance.iter().any(|a| a.status == "fail")
    }
}

// ---- source recheck(`nw/source-recheck.v1`,工单 #40) ----

/// 一条被复核的记录的事实(谁、扫到几个、快照是谁)。
/// `commit` 只是快照身份:**上游 commit 变化不算漂移**(引擎口径,面板不自定规则)。
#[derive(Clone, Debug, Deserialize)]
pub struct CheckedSource {
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub entries: u32,
}

/// 一条漂移差异。`kind` 是引擎冻结的四种之一:
/// `added`(仓有编目无)/ `removed`(编目有仓无)/ `renamed`(路径在 name 变)/
/// `path_changed`(name 在路径变)。面板按原文呈现,不自造第五种。
#[derive(Clone, Debug, Deserialize)]
pub struct Drift {
    pub source: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub entry: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub detail: String,
}

/// recheck 的 data 形状(只读:只报告、永不写账)。
/// 编目不存在 → 空报告 + rc 0(「还没注册过任何外部源」是合法状态)。
#[derive(Clone, Debug, Deserialize)]
pub struct RecheckData {
    #[serde(default, rename = "catalogPath")]
    pub catalog_path: String,
    #[serde(default)]
    pub checked: Vec<CheckedSource>,
    #[serde(default)]
    pub drift: Vec<Drift>,
}

impl RecheckData {
    /// 这条记录的漂移结论按结果四态落:无漂移 = 过,有漂移 = 败。
    /// (漂移在引擎侧是「结论是红的」——rc 3;面板不把它降级成警告。)
    pub fn state_of(&self, source: &str) -> State {
        if self.drift.iter().any(|d| d.source == source) {
            State::Fail
        } else if self.checked.iter().any(|c| c.name == source) {
            State::Pass
        } else {
            State::Skip
        }
    }
}

// ---- 编目册骨架(spec #50 工单 #53,ADR-0009):投影三件 ----

/// 编目册的一道闸结论(#53 抽出的骨架第一件之一)。
///
/// `id` 是冻结面(vps 五道:`register.name` / `register.ips` /
/// `register.port` / `register.logins` / `register.secret_guard`),
/// `message` 措辞不承诺;`status` 三态 `pass` / `fail` / `skip`
/// (skip = 没跑到这道闸,前面某道已拒)——与 validate 的「结果四态」
/// 同一套词,面板直接复用 [`State`],不新造词汇。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CatalogGate {
    pub id: String,
    pub status: String,
    pub message: String,
}

impl CatalogGate {
    /// 三态按结果四态落(`State::from` 认 pass/warn/fail/skip 四词)。
    pub fn state(&self) -> State {
        State::from(self.status.as_str())
    }

    /// 这一道闸没过(引擎铁律:有 fail 就一个字节都不落)。
    pub fn is_fail(&self) -> bool {
        self.status == "fail"
    }
}

/// 编目册的一条地址:`label` 是引擎给的展示名(如 `home` / `office`),
/// `addr` 原样。两槽都必在(上游 `VpsAddr` 无 omitempty)。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CatalogAddr {
    pub label: String,
    pub addr: String,
}

/// 编目册的一条登录:`user` 是机上的用户名,`key_ref` 是**密钥引用**
/// (opaque 指针:`$ENV` / `!command` / ssh-agent / 密钥路径)。
/// **指针不是密钥**,所以原样呈现是安全的;上游字段名是 `ref`
/// (Rust 保留字,本地改名不改语义)。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CatalogLogin {
    pub user: String,
    #[serde(rename = "ref")]
    pub key_ref: String,
}

/// 编目册的一条**引用**(#54 标签册的槽):`<kind>/<条目>` 拆两半,
/// kind 的已知集合一处定义在 `engine::commands::TAG_REF_KINDS`
/// (`{skill, extension, vps}`,上游 `tag/tag.go`)。悬空引用(指向已消失
/// 条目)由引擎在写操作前静默清掉,上报面是 `data.cleaned`——面板显式
/// 呈现清了几条,不假装没发生。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct CatalogRef {
    pub kind: String,
    pub id: String,
}

impl CatalogRef {
    /// 展示串:`kind/id`(引擎原文的拼法,面板不另译一个词)。
    pub fn label(&self) -> String {
        format!("{}/{}", self.kind, self.id)
    }
}

/// 编目册**条目投影**(#53 抽出的骨架第一件):名字 / 若干条地址 /
/// 端口 / 若干条登录 / 备注 / 若干条引用。
///
/// 字段恒在、不为 null:缺字段由展示层显式「——」(不推导、不补全),
/// 这一层只负责把引擎原文搬过来。与命令组无关——vps / key /
/// subscription / tag 四本册子共用这一个投影形状,各写一个薄适配器
/// (`VpsEntry::view` 是第一个)。`refs` 是标签册的槽(#54):机器册条目
/// 恒空数组,标签册条目的地址/端口/登录/备注恒空——**哪本册子填哪几槽
/// 是固定的**,展示层按在槽里的字段画,不靠猜。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogEntryView {
    pub name: String,
    /// (label, addr) 原样;引擎没给地址 = 空数组(不是 null)。
    pub addrs: Vec<CatalogAddr>,
    /// 端口;缺 = None(展示层给「——」)。
    pub port: Option<String>,
    /// (user, ref) 原样;ref 原样呈现,摘要不印。
    pub logins: Vec<CatalogLogin>,
    /// 备注;上游 `note` 是唯一 `omitempty` 槽——真「没有就没有」。
    pub note: Option<String>,
    /// (kind, id) 原样;标签册的槽(一标签挂多条目、跨 kind)。
    pub refs: Vec<CatalogRef>,
    /// 消费这条链接的应用(订阅册的槽;上游字段名 `client`,"它是字段不是
    /// 名字"——如 Mihomo)。缺 = None(展示层给「——」)。
    pub client: Option<String>,
    /// 到期日(订阅册的槽):严格 `YYYY-MM-DD` 原值;`""` / None = 永不过期
    /// (上游口径:expires 恒在,空串 = 永不过期)。
    pub expires: Option<String>,
    /// 到期标记(订阅册的槽):`active` / `expiring` / `expired`。**由上游
    /// `status.go` 读取时算好、随回音下发**,永不写进文件;面板只做状态映射,
    /// 本地零日期运算(ADR-0009 §3)。回音缺这一格 = None(展示层显式「——」,
    /// 面板不自己算一套)。
    pub status: Option<String>,
    /// 剩余天数(订阅册的槽):上游算好的、从今天(UTC)到到期日的整天数,过期
    /// 为负;没给到期日 = 引擎给 `null` → None(0 天和「没这回事」是两件事,
    /// 上游刻意用 nil 指针而不是 0 占位)。
    pub days_left: Option<i64>,
    /// kind(钥匙册的槽;上游 `KeyEntry.Kind`,条目级,恒 `github-key`)。
    /// 缺 = None(展示层给「——」)。订阅册也有 kind 但投影不画它。
    pub kind: Option<String>,
    /// 服务哪个主机(钥匙册的槽;上游 `KeyEntry.Host`,缺省 github.com)。
    /// 缺 = None(展示层给「——」)。
    pub host: Option<String>,
    /// 作用域(钥匙册的槽;上游 `KeyEntry.Scopes`,**只登记不设闸**)。恒数组,
    /// 空 = 未声明作用域。
    pub scopes: Vec<String>,
    /// 轮换日(钥匙册的槽;上游 `KeyEntry.RotatedAt`,严格 YYYY-MM-DD,新登记
    /// = 登记日 UTC)。缺 = None(展示层给「——」)。
    pub rotated_at: Option<String>,
}

impl CatalogEntryView {
    /// 地址一行:`label=addr` 逐条;空数组给「——」。
    pub fn addr_line(&self) -> String {
        if self.addrs.is_empty() {
            return "——".to_string();
        }
        self.addrs
            .iter()
            .map(|a| format!("{}={}", a.label, a.addr))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// 登录一行:`user→ref` 逐条;空数组给「——」。ref 原样(指针不是密钥)。
    pub fn login_line(&self) -> String {
        if self.logins.is_empty() {
            return "——".to_string();
        }
        self.logins
            .iter()
            .map(|l| format!("{}→{}", l.user, l.key_ref))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// 引用一行:`kind/id` 逐条;空数组给「——」。
    pub fn ref_line(&self) -> String {
        if self.refs.is_empty() {
            return "——".to_string();
        }
        self.refs
            .iter()
            .map(|r| format!("{}/{}", r.kind, r.id))
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// 到期日一行(订阅册):没给到期日 = 永不过期(上游 `expires` 空串的口径,
    /// 展示层照原话说,不推导、不补一个日期)。
    pub fn expires_line(&self) -> String {
        match self.expires.as_deref().map(str::trim) {
            Some(e) if !e.is_empty() => e.to_string(),
            _ => "永不过期".to_string(),
        }
    }

    /// 剩余天数一行(订阅册):引擎没给(没给到期日 = `null`,或回音缺字段)=
    /// 「——」,不猜、不拿 0 占位。
    pub fn days_left_line(&self) -> String {
        match self.days_left {
            Some(n) => n.to_string(),
            None => "——".to_string(),
        }
    }

    /// 作用域一行(钥匙册):**原样逐条报**,与 `addrs` / `refs` / `logins`
    /// 三个槽同一口径——它们是操作者刚贴进去的登记元数据,不是机密,也不指向
    /// 哪把钥匙(ADR-0009 §3 的「不印指纹」讲的是 secret 槽,与 scope 无关)。
    /// 0 条显式「未声明」,不猜、不补。
    ///
    /// 曾经只报**条数**,那是第 9 轮册内过滤按 scope 的值匹配之后显出的一处
    /// 不一致:操作者能按一个屏幕上只看得到条数的字段过滤。2026-10-12 业主
    /// 裁断改为报值。
    pub fn scopes_line(&self) -> String {
        if self.scopes.is_empty() {
            return "未声明(0 条)".to_string();
        }
        self.scopes.join(" · ")
    }

    /// 轮换日一行(钥匙册):缺字段显式「——」,不猜、不补一个日期。
    pub fn rotated_at_line(&self) -> String {
        match self.rotated_at.as_deref().map(str::trim) {
            Some(r) if !r.is_empty() => r.to_string(),
            _ => "——".to_string(),
        }
    }
}

/// 一本编目册的投影(#53 抽出的骨架第一件):**编目路径 + 条目清单 +
/// 闸结论**。字段恒在不为 null;`gates` 只在登记回音里出现(ls 回音
/// 恒空数组)。
///
/// 这一层只做投影与排序,不做呈现决策(那是 `screens::catalog` 的列表件)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatalogSheet {
    /// 编目文件路径(引擎原文,如 `<root>/vps.json`)。
    pub catalog_path: String,
    pub entries: Vec<CatalogEntryView>,
    pub gates: Vec<CatalogGate>,
}

impl CatalogSheet {
    /// 确定性排序:按条目名稳定排序(引擎自己已按唯一键排序,面板再排
    /// 一次不改变结果,只保证「同一份回音永远同一屏」——快照可复现)。
    pub fn sorted(mut self) -> Self {
        self.entries.sort_by(|a, b| a.name.cmp(&b.name));
        self
    }

    /// 闸里有 fail(登记铁律:任一不过 = 一个字节都不落)。
    pub fn has_fail(&self) -> bool {
        self.gates.iter().any(CatalogGate::is_fail)
    }
}

// ---- vps 组(`nw/vps-*.v1`,编目 `<root>/vps.json`,工单 #53) ----

/// vps 编目条目形状(= 编目形状,引擎 `VpsEntry` 逐字对应)。
/// `port` 可缺(上游不用 omitempty 但允许 0/缺),`note` 是唯一真可缺槽。
#[derive(Clone, Debug, Deserialize)]
pub struct VpsEntry {
    pub name: String,
    #[serde(default)]
    pub ips: Vec<CatalogAddr>,
    #[serde(default)]
    pub port: Option<i64>,
    #[serde(default)]
    pub logins: Vec<CatalogLogin>,
    #[serde(default)]
    pub note: Option<String>,
}

impl VpsEntry {
    /// 薄适配器:编目条目 → 编目册投影(后三本册子各写一个同形状的)。
    pub fn view(&self) -> CatalogEntryView {
        CatalogEntryView {
            name: self.name.clone(),
            addrs: self.ips.clone(),
            port: self.port.map(|p| p.to_string()),
            logins: self.logins.clone(),
            note: self.note.clone(),
            refs: Vec::new(),
            ..Default::default()
        }
    }
}

/// `nw vps register <名> …` 的 data(预审与落账共用同一形状,
/// `written` 区分「文件不落」与真写入)。
///
/// 反 D6 纪律:`ips` / `logins` **只在过了第⑤道闸之后才填**——失败时
/// data 不带疑似密钥的 ref(上游 `vps/register.go` 的口径,面板不补全、
/// 不猜测缺字段)。
#[derive(Clone, Debug, Deserialize)]
pub struct VpsRegisterData {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub ips: Vec<CatalogAddr>,
    #[serde(default)]
    pub port: Option<i64>,
    #[serde(default)]
    pub logins: Vec<CatalogLogin>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default, rename = "catalogPath")]
    pub catalog_path: String,
    #[serde(default)]
    pub written: bool,
    #[serde(default)]
    pub updated: bool,
    #[serde(default)]
    pub action: String,
    /// 五道闸,**恒 5 行**,id 冻结、三态 pass/fail/skip。
    #[serde(default)]
    pub gates: Vec<CatalogGate>,
}

impl VpsRegisterData {
    /// 登记回音 → 编目册投影(条目 + 编目路径 + 闸结论)。
    pub fn sheet(&self) -> CatalogSheet {
        CatalogSheet {
            catalog_path: self.catalog_path.clone(),
            entries: vec![CatalogEntryView {
                name: self.name.clone(),
                addrs: self.ips.clone(),
                port: self.port.map(|p| p.to_string()),
                logins: self.logins.clone(),
                note: self.note.clone(),
                refs: Vec::new(),
                ..Default::default()
            }],
            gates: self.gates.clone(),
        }
    }

    /// 闸里有 fail(落账闸的口径来源:任一不过不得进入落账段)。
    pub fn has_fail(&self) -> bool {
        self.gates.iter().any(CatalogGate::is_fail)
    }
}

/// `nw vps ls` 的 data:编目路径 + 机器清单。
/// 编目不存在 = 空数组 rc 0(合法空态,不是错误)。
#[derive(Clone, Debug, Deserialize)]
pub struct VpsLsData {
    #[serde(default, rename = "catalogPath")]
    pub catalog_path: String,
    #[serde(default)]
    pub machines: Vec<VpsEntry>,
}

impl VpsLsData {
    /// ls 投影 → 编目册投影(确定性排序后的清单)。
    pub fn sheet(&self) -> CatalogSheet {
        CatalogSheet {
            catalog_path: self.catalog_path.clone(),
            entries: self.machines.iter().map(VpsEntry::view).collect(),
            gates: Vec::new(),
        }
        .sorted()
    }
}

/// `nw vps rm <名>` 的 data:删不存在的名 → rc 3 + 这份证据,
/// 且**文件一个字节都不动**(`removed` = false、`written` = false)。
#[derive(Clone, Debug, Deserialize)]
pub struct VpsRmData {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "catalogPath")]
    pub catalog_path: String,
    #[serde(default)]
    pub removed: bool,
    #[serde(default)]
    pub written: bool,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub remaining: u32,
}

// ---- tag 组(`nw/tag-*.v1`,编目 `<root>/tags.json`,工单 #54) ----

/// 标签册的一条(反查投影):标签名 + 它挂着的引用。**刻意是投影形状**:
/// `refs` 是 `{kind, id}` 两槽,没有任何「本体」槽(标签是扁平明文词,
/// `--ref` 走 argv 不走 stdin,无机密面)。
#[derive(Clone, Debug, Deserialize)]
pub struct TagEntry {
    pub name: String,
    #[serde(default)]
    pub refs: Vec<CatalogRef>,
}

impl TagEntry {
    /// 薄适配器:标签条目 → 编目册投影(与 `VpsEntry::view` 同形状,
    /// 只填自己那一槽:引用)。
    pub fn view(&self) -> CatalogEntryView {
        CatalogEntryView {
            name: self.name.clone(),
            refs: self.refs.clone(),
            ..Default::default()
        }
    }
}

/// `nw tag ls [<标签>]` 的 data:编目路径 + 标签清单。**两种调用同一形状**
/// (无参列全部,带参最多一条)——「列全部」与「只列一个」在 argv 上区分,
/// data 面不另造行类型。
///
/// 两态可预测(51 §6.11,面板必须分开呈现):编目不存在 = 空数组 rc 0
/// (合法空态);带参查不到 = **rc 3**(问错了名字,不是空结果)。
#[derive(Clone, Debug, Deserialize)]
pub struct TagLsData {
    #[serde(default, rename = "catalogPath")]
    pub catalog_path: String,
    #[serde(default)]
    pub tags: Vec<TagEntry>,
}

impl TagLsData {
    /// 反查投影 → 编目册投影(确定性排序后的清单)。
    pub fn sheet(&self) -> CatalogSheet {
        CatalogSheet {
            catalog_path: self.catalog_path.clone(),
            entries: self.tags.iter().map(TagEntry::view).collect(),
            gates: Vec::new(),
        }
        .sorted()
    }
}

/// `nw tag add <标签> --ref …` 的 data。**没有 `--yes`**,所以预审与落账
/// 在 tag 这边是同一份 argv、同一形状:`written` / `updated` / `action`
/// 区分「这一次到底写没写」(重跑同一条命令 = `updated` 而字节不变)。
///
/// `cleaned` 是**悬空引用清理**:本次落盘前被静默清掉的、指向已消失条目
/// 的引用。引擎只在 data 里给,面板必须显式呈现清了几条(工单 #54 验收
/// 第 4 条),不假装没发生。
#[derive(Clone, Debug, Deserialize)]
pub struct TagAddData {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "catalogPath")]
    pub catalog_path: String,
    #[serde(default)]
    pub written: bool,
    #[serde(default)]
    pub updated: bool,
    #[serde(default)]
    pub action: String,
    /// 这一标签当前的引用清单(追加合并后的结果)。
    #[serde(default)]
    pub refs: Vec<CatalogRef>,
    /// 本次静默清掉的悬空引用(条数 = 界面要明示的那个数)。
    #[serde(default)]
    pub cleaned: Vec<CatalogRef>,
    /// 四道闸,**恒 4 行**:`tag.add.name` / `tag.add.ref-kind` /
    /// `tag.add.ref-id` / `tag.add.ref-exists`;id 冻结、三态 pass/fail/skip。
    #[serde(default)]
    pub gates: Vec<CatalogGate>,
}

impl TagAddData {
    /// 登记回音 → 编目册投影(条目 = 这一标签 + 它的引用)。
    pub fn sheet(&self) -> CatalogSheet {
        CatalogSheet {
            catalog_path: self.catalog_path.clone(),
            entries: vec![CatalogEntryView {
                name: self.name.clone(),
                refs: self.refs.clone(),
                ..Default::default()
            }],
            gates: self.gates.clone(),
        }
    }

    /// 悬空引用清理了几条(界面明示用,不静默)。
    pub fn cleaned_count(&self) -> usize {
        self.cleaned.len()
    }

    /// 闸里有 fail(引擎铁律:任一不过 = 一个字节都不落)。
    pub fn has_fail(&self) -> bool {
        self.gates.iter().any(CatalogGate::is_fail)
    }
}

/// `nw tag rm <标签> [--ref …]` 的 data 与 `nw vps rm` **逐字相同**——
/// 上游 `entry.RmOutcome` 是四组共用的同一个类型(`entry/commands.go`),
/// 不是两个形状。#53 落地时按第一个命令组命名为 [`VpsRmData`];标签册
/// 直接复用同一形状,不复制第二份(工单 #54 验收第 5 条)。
///
/// `action` 第三态 `ref_removed`(只摘引用,标签还在)/ `not_found`
/// (什么都没动);删不存在的标签 → rc 3 且文件一个字节都不动。
pub type TagRmData = VpsRmData;

// ---- subscription 组(`nw/subscription-*.v1`,编目 `<root>/subscriptions.json`,工单 #56) ----

/// 订阅册的一条(ls 投影):**没有 url 槽、也没有任何指纹字段**(上游
/// `SubSummary` 六个键冻结;R1:链接即凭据,`ls` 永不回显本体,沿 #18
/// 三不印)——分享清单、截图、贴日志都不泄密。这是**投影**不是引用:
/// 想读链接本体只有 `nw subscription get` 一条路(且面板不代跑,ADR-0009)。
///
/// `status` / `days_left` 是**读取时算的到期标记**(上游 `status.go`):
/// 不在文件里、也永不写回文件;三态按 UTC 日期与今天比,今天起 7 天内
/// (含第 7 天)= `expiring`,过去 = `expired`,更远与「没给到期日」=
/// `active`。**只标记不治理**:过期不改退出码、不提醒、不靠 ANSI 颜色。
#[derive(Clone, Debug, Deserialize)]
pub struct SubSummary {
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub client: String,
    #[serde(default)]
    pub expires: String,
    #[serde(default)]
    pub status: String,
    /// 上游 `daysLeft` 是 `*int`:没给到期日 = JSON `null`(键恒在)。
    #[serde(default, rename = "daysLeft")]
    pub days_left: Option<i64>,
}

impl SubSummary {
    /// 薄适配器:订阅条目 → 编目册投影(与 `VpsEntry::view` / `TagEntry::view`
    /// 同形状,只填自己那一组槽:client / 到期日 / 到期三态 / 剩余天数)。
    /// **绝不填链接槽**——投影里没有这一格,结构上放不下。
    pub fn view(&self) -> CatalogEntryView {
        CatalogEntryView {
            name: self.name.clone(),
            client: Some(self.client.clone()),
            expires: Some(self.expires.clone()),
            status: Some(self.status.clone()),
            days_left: self.days_left,
            ..Default::default()
        }
    }
}

/// `nw subscription ls` 的 data:编目路径 + 订阅清单。编目不存在 =
/// 空数组 rc 0(合法空态);坏账 = rc 3 点名路径(绝不静默当成空编目)。
#[derive(Clone, Debug, Deserialize)]
pub struct SubLsData {
    #[serde(default, rename = "catalogPath")]
    pub catalog_path: String,
    #[serde(default)]
    pub subscriptions: Vec<SubSummary>,
}

impl SubLsData {
    /// ls 投影 → 编目册投影(确定性排序后的清单)。
    pub fn sheet(&self) -> CatalogSheet {
        CatalogSheet {
            catalog_path: self.catalog_path.clone(),
            entries: self.subscriptions.iter().map(SubSummary::view).collect(),
            gates: Vec::new(),
        }
        .sorted()
    }

    /// 到期三态的条数汇总(`active` / `expiring` / `expired` 各自几条)。
    /// 面板不自己算一套:这里只是把引擎已给的三态**点数**,阈值与日期口径
    /// 全在引擎侧(上游 `status.go`,7 天不配置化)。
    pub fn status_counts(&self) -> SubStatusCounts {
        let mut c = SubStatusCounts::default();
        for s in &self.subscriptions {
            match s.status.as_str() {
                "active" => c.active += 1,
                "expiring" => c.expiring += 1,
                "expired" => c.expired += 1,
                _ => c.unknown += 1,
            }
        }
        c
    }
}

/// 到期三态汇总(册体那一行「几条 active · 几条 expiring · 几条 expired」)。
/// `unknown` = 回音缺 `status` 字段的条数——**显式计数不猜**,不并进任何一态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubStatusCounts {
    pub active: usize,
    pub expiring: usize,
    pub expired: usize,
    pub unknown: usize,
}

/// `nw subscription register` 的 data(预审与落账共用同一形状,`written`
/// 区分「文件不落」与真写入)。
///
/// **一个能放链接本体的字段都没有**(上游 `RegisterResult` 刻意不留 url
/// 槽:register 是「链接进仓」的写入口,却不是「链接出来」的出口——想泄
/// 都没有地方放)。`client` / `expires` 只在过了各自那道闸之后才填,
/// 被拒时是零值(失败也带 data 是硬约定)。
#[derive(Clone, Debug, Deserialize)]
pub struct SubRegisterData {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub client: String,
    #[serde(default)]
    pub expires: String,
    #[serde(default, rename = "catalogPath")]
    pub catalog_path: String,
    #[serde(default)]
    pub written: bool,
    #[serde(default)]
    pub updated: bool,
    #[serde(default)]
    pub action: String,
    /// 四道闸,**恒 4 行**:`subscription.register.name` / `.client` /
    /// `.expires` / `.body`;id 冻结、三态 pass/fail/skip。
    #[serde(default)]
    pub gates: Vec<CatalogGate>,
}

impl SubRegisterData {
    /// 登记回音 → 编目册投影(条目 = 这一条订阅;没有链接槽)。
    pub fn sheet(&self) -> CatalogSheet {
        CatalogSheet {
            catalog_path: self.catalog_path.clone(),
            entries: vec![SubSummary {
                name: self.name.clone(),
                kind: self.kind.clone(),
                client: self.client.clone(),
                expires: self.expires.clone(),
                // 登记回音不带到期标记(那是读侧 ls / get 现算的),所以
                // 这一格恒空——展示层显式「——」,不拿登记那一刻冒充读取
                // 那一刻(上游口径:标记永不写进文件)。
                status: String::new(),
                days_left: None,
            }
            .view()],
            gates: self.gates.clone(),
        }
    }

    /// 闸里有 fail(落账闸的口径来源:任一不过 = 一个字节都不落)。
    pub fn has_fail(&self) -> bool {
        self.gates.iter().any(CatalogGate::is_fail)
    }
}

/// `nw subscription rm <名>` 的 data 与 `nw vps rm` **逐字相同**——上游
/// `entry.RmOutcome` 是四组共用的同一个类型。删不存在的名 → rc 3 且
/// `subscriptions.json` 一个字节都不动;删光最后一条留空 `entries` 数组
/// 的文件,不删文件。
pub type SubRmData = VpsRmData;

// ---- key 组(`nw/key-*.v1`,编目 `<root>/keys.json`,工单 #57) ----

/// 钥匙册的一条(ls 投影):**没有 secret 槽、也没有任何指纹字段**(上游
/// `KeySummary` 五个键冻结;`KeyEntry` 有 secret 槽,但 ls **刻意不是
/// `KeyEntry`** —— 投影结构上放不下本体,#18 三不印):分享清单、截图、贴
/// 日志都不泄密,面板连「这像是哪把钥匙」都不该让人推断出来。这是**投影**
/// 不是引用:想读本体只有 `nw key get` 一条路(且面板不代跑,ADR-0009 §1)。
///
/// `scopes` 恒为数组(上游不用 omitempty,没给 = `[]`,一种形状);`rotatedAt`
/// 恒在(新登记 = 登记日 UTC,机器写产物不该有「没有轮换日」的第三态)。
#[derive(Clone, Debug, Deserialize)]
pub struct KeySummary {
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default, rename = "rotatedAt")]
    pub rotated_at: String,
}

impl KeySummary {
    /// 薄适配器:钥匙条目 → 编目册投影(与 `VpsEntry::view` / `TagEntry::view` /
    /// `SubSummary::view` 同形状,只填自己那一组槽:kind / host / 作用域 /
    /// 轮换日)。**绝不填 secret 槽**——投影里没有这一格,结构上放不下。
    pub fn view(&self) -> CatalogEntryView {
        CatalogEntryView {
            name: self.name.clone(),
            kind: Some(self.kind.clone()),
            host: Some(self.host.clone()),
            scopes: self.scopes.clone(),
            rotated_at: Some(self.rotated_at.clone()),
            ..Default::default()
        }
    }
}

/// `nw key ls` 的 data:编目路径 + 钥匙清单。编目不存在 = 空数组 rc 0
/// (合法空态);坏账 = rc 3 点名路径(绝不静默当成空编目)。
#[derive(Clone, Debug, Deserialize)]
pub struct KeyLsData {
    #[serde(default, rename = "catalogPath")]
    pub catalog_path: String,
    #[serde(default)]
    pub keys: Vec<KeySummary>,
}

impl KeyLsData {
    /// ls 投影 → 编目册投影(确定性排序后的清单)。
    pub fn sheet(&self) -> CatalogSheet {
        CatalogSheet {
            catalog_path: self.catalog_path.clone(),
            entries: self.keys.iter().map(KeySummary::view).collect(),
            gates: Vec::new(),
        }
        .sorted()
    }
}

/// `nw key register` 的 data(预审与落账共用同一形状,`written` 区分「文件不落」
/// 与真写入)。
///
/// **一个能放本体的字段都没有**(上游 `RegisterResult` 刻意不留 secret 槽:
/// register 是「本体进仓」的写入口,却不是「本体出来」的出口——想泄都没有地方
/// 放)。`host` / `scopes` / `rotatedAt` 只在过了全部三道闸之后才填,被拒时是
/// 零值(失败也带 data 是硬约定)。
#[derive(Clone, Debug, Deserialize)]
pub struct KeyRegisterData {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default, rename = "rotatedAt")]
    pub rotated_at: String,
    #[serde(default, rename = "catalogPath")]
    pub catalog_path: String,
    #[serde(default)]
    pub written: bool,
    #[serde(default)]
    pub updated: bool,
    #[serde(default)]
    pub action: String,
    /// 三道闸,**恒 3 行**:`key.register.name` / `key.register.host` /
    /// `key.register.body`;id 冻结、三态 pass/fail/skip。
    #[serde(default)]
    pub gates: Vec<CatalogGate>,
}

impl KeyRegisterData {
    /// 登记回音 → 编目册投影(条目 = 这一把钥匙;没有 secret 槽)。
    pub fn sheet(&self) -> CatalogSheet {
        CatalogSheet {
            catalog_path: self.catalog_path.clone(),
            entries: vec![KeySummary {
                name: self.name.clone(),
                kind: self.kind.clone(),
                host: self.host.clone(),
                scopes: self.scopes.clone(),
                rotated_at: self.rotated_at.clone(),
            }
            .view()],
            gates: self.gates.clone(),
        }
    }

    /// 闸里有 fail(落账闸的口径来源:任一不过 = 一个字节都不落)。
    pub fn has_fail(&self) -> bool {
        self.gates.iter().any(CatalogGate::is_fail)
    }
}

/// `nw key get --name <名>` 的 data(工单 #97 起面板代跑 get、只吃这一个形状):
/// 名字 + **唯一那一格钥匙本体**(`data.key`)。上游 `KeyEntry` 的其余键
/// (kind / host / scopes / rotatedAt)是登记元数据,面板不在这里消费——
/// 本体要进面板只有这一条路,进来就只进揭示区(见
/// [`crate::screens::catalog::RevealedBody`])。
///
/// `Debug` **刻意不打印本体**:与 `SecretBody` / `RevealedBody` 同一条纪律
/// (ADR-0009 §3),只报名字与字节数——否则这个中转类型的 `{:?}` 就是第二处
/// 泄漏面。订阅册那一格(`data.url`)由工单 #99 落同族类型,两册共用揭示区。
#[derive(Clone, Deserialize)]
pub struct KeyGetData {
    #[serde(default)]
    pub name: String,
    /// 钥匙本体。**唯一读者是揭示区渲染**(经 `RevealedBody`),别处不许读。
    #[serde(default)]
    pub key: String,
}

impl std::fmt::Debug for KeyGetData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "KeyGetData {{ name: {:?}, key: <回显 {} 字节,不打印内容> }}",
            self.name,
            self.key.len()
        )
    }
}

/// `nw key rm <名>` 的 data 与 `nw vps rm` **逐字相同**——上游 `entry.RmOutcome`
/// 是四组共用的同一个类型。删不存在的名 → rc 3 且 `keys.json` 一个字节都不动;
/// 删光最后一条留空 `entries` 数组的文件,不删文件。
pub type KeyRmData = VpsRmData;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope(data: Value) -> Envelope {
        serde_json::from_value(json!({
            "schema": "nw/validate.v1",
            "nw": "0.1.0",
            "ok": true,
            "errors": [],
            "warnings": [],
            "data": data,
        }))
        .unwrap()
    }

    #[test]
    fn 解析_validate_data() {
        let e = envelope(json!({
            "target": "recipe.yaml",
            "results": [
                {"id": "1", "status": "pass", "message": "schema 认"},
                {"id": "15", "status": "warn", "message": " Nit:", "path": "recipes/x/recipe.yaml"}
            ],
            "summary": {"total": 27, "pass": 20, "warn": 5, "fail": 1, "skip": 1}
        }));
        let d: ValidateData = e.data_as().unwrap();
        assert_eq!(d.summary.total, 27);
        assert_eq!(d.results[1].path.as_deref(), Some("recipes/x/recipe.yaml"));
    }

    #[test]
    fn 解析_up_data_容缺字段() {
        let e = envelope(json!({
            "profile": "desk",
            "components": ["retention"],
            "envMissing": ["OPENAI_API_KEY"],
            "lockId": "a1b2c3",
        }));
        let d: UpData = e.data_as().unwrap();
        assert_eq!(d.env_missing, vec!["OPENAI_API_KEY".to_string()]);
        assert_eq!(d.lock_id.as_deref(), Some("a1b2c3"));
        assert_eq!(d.env_present, 0);
    }

    #[test]
    fn 解析_doctor_data() {
        let e = envelope(json!({
            "results": [{"surface": "manifest", "status": "pass", "message": "在位"}],
            "exit": 11,
            "converged": false,
        }));
        let d: DoctorData = e.data_as().unwrap();
        assert_eq!(d.exit, 11);
        assert_eq!(d.converged, Some(false));
        assert!(d.manifest.is_none());
    }

    #[test]
    fn 解析_ls_data_空域容忍() {
        let e = envelope(json!({
            "domain": "all",
            "recipes": [{"name": "research-desk", "version": "0.2.0"}],
            "components": [],
            "sources": [],
            "profiles": [{"name": "desk", "state": "drift", "lockId": "9f8e"}],
            "locks": [],
        }));
        let d: LsData = e.data_as().unwrap();
        assert_eq!(d.recipes[0].name, "research-desk");
        assert_eq!(d.profiles[0].state.as_deref(), Some("drift"));
        assert!(d.components.is_empty());
    }

    #[test]
    fn 未知字段不炸() {
        let e = envelope(json!({
            "target": "t",
            "results": [],
            "summary": {"total": 0, "pass": 0, "warn": 0, "fail": 0, "skip": 0},
            "futureField": {"whatever": 1}
        }));
        let d: ValidateData = e.data_as().unwrap();
        assert_eq!(d.summary.total, 0);
    }

    #[test]
    fn headline_优先错误再警告() {
        let mut e = envelope(json!({}));
        e.warnings.push(Evidence {
            code: None,
            exit_code: None,
            message: "小警告".into(),
            hint: None,
            details: None,
        });
        e.errors.push(Evidence {
            code: Some("DRIFT".into()),
            exit_code: Some(11),
            message: "漂了".into(),
            hint: Some("重跑".into()),
            details: None,
        });
        assert_eq!(e.headline(), "漂了");
    }

    // ---- #28 契约守卫:不认识的 schema ----

    fn envelope_with_schema(schema: &str, nw: &str, data: Value) -> Envelope {
        serde_json::from_value(json!({
            "schema": schema,
            "nw": nw,
            "ok": true,
            "errors": [],
            "warnings": [],
            "data": data,
        }))
        .unwrap()
    }

    #[test]
    fn 契约守卫_认识的_schema_照常拆型与无证据() {
        let e = envelope_with_schema("nw/ls.v1", "0.1.0", json!({"domain": "all"}));
        assert!(e.schema_known());
        let d: LsData = e.data_as().unwrap();
        assert_eq!(d.domain, "all");
        let mut e = e;
        e.apply_schema_guard(0);
        assert!(e.schema_guard.is_none(), "认识的 schema 不产生守卫证据");
    }

    #[test]
    fn 契约守卫_不认识的_schema_不按旧形状降级解析() {
        // 即使 data 恰好兼容旧 LsData 形状,也不许拆出来(静默摆错数据)
        let mut e = envelope_with_schema(
            "nw/ls.v2",
            "9.9.9",
            json!({"domain": "all", "recipes": [{"name": "x"}]}),
        );
        assert!(!e.schema_known());
        assert!(
            e.data_as::<LsData>().is_none(),
            "不认识的 schema 必须拒拆,不按旧形状解析"
        );
        e.apply_schema_guard(3);
        let g = e.schema_guard.as_ref().expect("应合成需升级证据");
        assert_eq!(g.code.as_deref(), Some("SCHEMA_UNKNOWN"));
        assert_eq!(g.exit_code, Some(3));
        assert!(
            g.message.contains("需升级 nw")
                && g.message.contains("nw/ls.v2")
                && g.message.contains("9.9.9"),
            "人话一句含需升级 + schema 原文 + 引擎版本:{}",
            g.message
        );
        let d = g.details.as_ref().unwrap();
        assert_eq!(d["schema"], "nw/ls.v2");
        assert_eq!(d["nw"], "9.9.9");
        // 五字段里的 hint 给升级指引,不回显任何密钥(只放 schema/版本)
        assert!(g.hint.as_deref().unwrap().contains("升级"));
    }

    #[test]
    fn 契约守卫_四组编目十四个schema全都认识_第8轮跟随同步() {
        // spec #50 / 工单 #51:上游所配 0.1.0 本身即带四组编目命令共 14 个
        // schema(cli.go 的 Schema* 常量,逐个指回 notes/51-follow-sync-diff.md
        // §1 的契约表)。此前面板一个都不认识——只要代跑任一编目命令就合成
        // 「需升级 nw」,而 nw --version 报的正是 0.1.0:落后的不是引擎是面板
        // 的契约表(ADR-0007「双真相源必然打架」的现形)。
        for schema in [
            "nw/vps-register.v1",
            "nw/vps-ls.v1",
            "nw/vps-rm.v1",
            "nw/key-register.v1",
            "nw/key-ls.v1",
            "nw/key-get.v1",
            "nw/key-rm.v1",
            "nw/subscription-register.v1",
            "nw/subscription-ls.v1",
            "nw/subscription-get.v1",
            "nw/subscription-rm.v1",
            "nw/tag-add.v1",
            "nw/tag-ls.v1",
            "nw/tag-rm.v1",
        ] {
            let mut e = envelope_with_schema(schema, "0.1.0", json!({"gates": []}));
            assert!(
                e.schema_known(),
                "{schema} 必须在认识集合内(否则误报需升级)"
            );
            e.apply_schema_guard(0);
            assert!(
                e.schema_guard.is_none(),
                "{schema} 认识,不该合成「需升级 nw」证据"
            );
        }
        assert_eq!(KNOWN_SCHEMAS.len(), 22, "既有 8 + 编目 14");
    }

    #[test]
    fn 契约守卫_认识形状不等于准许代跑_两层分工不被混淆() {
        // ADR-0009:key get / subscription get 的 schema 在认识集合里(契约对齐),
        // 但面板对这两个命令只给建议命令、不代跑——认识形状答「我听不听得懂」,
        // 准许代跑答「面板会不会跑起来」,两件事。这里守前一半:认识集合里
        // 确有这两个 schema(它们不是「需升级」),代跑边界由 ADR-0009 裁定、
        // 由功能票(#56/#57)落地,不在契约守卫这一层。
        assert!(is_known_schema("nw/key-get.v1"));
        assert!(is_known_schema("nw/subscription-get.v1"));
        // 反面:真不认识的仍走守卫,不因为集合扩了就放松
        let mut e = envelope_with_schema("nw/vps-ls.v2", "9.9.9", json!({}));
        assert!(!e.schema_known(), "v2 仍不认识");
        e.apply_schema_guard(3);
        assert_eq!(
            e.schema_guard.unwrap().code.as_deref(),
            Some("SCHEMA_UNKNOWN")
        );
    }

    #[test]
    fn 契约守卫_重复合成幂等_不覆盖已有证据() {
        let mut e = envelope_with_schema("nw/doctor.v2", "1.0.0", json!({}));
        e.apply_schema_guard(11);
        let first = e.schema_guard.clone().unwrap();
        e.apply_schema_guard(0);
        assert_eq!(e.schema_guard.unwrap().exit_code, first.exit_code, "幂等");
    }

    // ---- #40 recheck 定型:只读漂移结论,四态判定以引擎原文为据 ----

    #[test]
    fn 解析_recheck_data_漂移四种与快照身份() {
        let e = envelope_with_schema(
            "nw/source-recheck.v1",
            "0.1.0",
            json!({
                "catalogPath": "/data/repo/catalog.json",
                "checked": [
                    {"name": "web-search", "kind": "skill", "repo": "o/r", "commit": "abc123", "entries": 2},
                    {"name": "theme-pack", "kind": "extension", "repo": "o/t", "commit": "def456", "entries": 1}
                ],
                "drift": [
                    {"source": "web-search", "kind": "added", "entry": "deep-research", "path": "skills/deep-research"}
                ]
            }),
        );
        let d: RecheckData = e.data_as().unwrap();
        assert_eq!(d.checked.len(), 2);
        assert_eq!(d.checked[0].entries, 2);
        assert_eq!(d.drift[0].kind, "added");
        // 四态:有漂移=败,查了没漂移=过,没查=略
        assert_eq!(d.state_of("web-search"), State::Fail);
        assert_eq!(d.state_of("theme-pack"), State::Pass);
        assert_eq!(d.state_of("not-checked"), State::Skip);
    }

    #[test]
    fn 解析_recheck_data_编目不存在是空报告不是错误() {
        let e = envelope_with_schema(
            "nw/source-recheck.v1",
            "0.1.0",
            json!({"catalogPath": "catalog.json", "checked": [], "drift": []}),
        );
        let d: RecheckData = e.data_as().unwrap();
        assert!(d.checked.is_empty() && d.drift.is_empty());
        assert_eq!(d.state_of("whatever"), State::Skip, "没查过的不冒充没问题");
    }

    // ---- #53 编目册骨架:vps 三形状 + 投影 ----

    #[test]
    fn 解析_vps_ls_data_编目条目形状与缺字段容忍() {
        let e = envelope_with_schema(
            "nw/vps-ls.v1",
            "0.1.0",
            json!({
                "catalogPath": "/data/root/vps.json",
                "machines": [
                    {
                        "name": "gate",
                        "ips": [{"label": "home", "addr": "10.0.0.1"}],
                        "port": 22,
                        "logins": [{"user": "root", "ref": "$DEPLOY_KEY"}],
                        "note": "门口那台"
                    },
                    {"name": "bare",
                     "ips": [],
                     "logins": [],
                     "note": ""}
                ]
            }),
        );
        let d: VpsLsData = e.data_as().unwrap();
        assert_eq!(d.machines.len(), 2);
        let sheet = d.sheet();
        // 确定性排序:按名字(bare < gate),同一份回音永远同一屏
        assert_eq!(sheet.entries[0].name, "bare");
        assert_eq!(sheet.entries[1].name, "gate");
        // ref 是指针不是密钥:原样进投影,展示安全
        assert_eq!(sheet.entries[1].logins[0].key_ref, "$DEPLOY_KEY");
        // 缺字段显式「——」由投影行给,不在这一层造默认值
        assert_eq!(sheet.entries[0].addr_line(), "——");
        assert_eq!(sheet.entries[0].login_line(), "——");
        assert!(sheet.entries[0].port.is_none());
        assert_eq!(sheet.entries[1].addr_line(), "home=10.0.0.1");
        assert_eq!(sheet.entries[1].login_line(), "root→$DEPLOY_KEY");
        assert_eq!(sheet.catalog_path, "/data/root/vps.json");
    }

    #[test]
    fn 解析_vps_ls_data_编目不存在是空数组不是错误() {
        // 上游口径:编目不存在 = 合法空态,回空数组 rc 0
        let e = envelope_with_schema(
            "nw/vps-ls.v1",
            "0.1.0",
            json!({"catalogPath": "/data/root/vps.json", "machines": []}),
        );
        let d: VpsLsData = e.data_as().unwrap();
        assert!(d.sheet().entries.is_empty());
    }

    #[test]
    fn 解析_vps_register_data_五道闸恒五行_失败不带疑似密钥的ref() {
        let e = envelope_with_schema(
            "nw/vps-register.v1",
            "0.1.0",
            json!({
                "name": "gate",
                "ips": [],
                "logins": [],
                "catalogPath": "/data/root/vps.json",
                "written": false,
                "updated": false,
                "action": "rejected",
                "gates": [
                    {"id": "register.name", "status": "pass", "message": "名字认"},
                    {"id": "register.ips", "status": "pass", "message": "地址认"},
                    {"id": "register.port", "status": "pass", "message": "端口认"},
                    {"id": "register.logins", "status": "pass", "message": "登录认"},
                    {"id": "register.secret_guard", "status": "fail", "message": "ref 像私钥本体"}
                ]
            }),
        );
        let d: VpsRegisterData = e.data_as().unwrap();
        assert_eq!(d.gates.len(), 5, "五道闸恒 5 行");
        // 反 D6:第⑤道闸没过时 ips/logins 不填(失败 data 不带疑似密钥的 ref)
        assert!(d.ips.is_empty() && d.logins.is_empty());
        assert!(d.has_fail(), "有 fail 就不得进入落账段");
        let sheet = d.sheet();
        assert_eq!(sheet.gates.len(), 5);
        assert_eq!(sheet.gates[4].state(), State::Fail);
        assert_eq!(sheet.gates[0].state(), State::Pass);
        assert!(!d.written, "预审段文件不落");
    }

    #[test]
    fn 解析_vps_rm_data_删不存在的名给证据且未动文件() {
        let e = envelope_with_schema(
            "nw/vps-rm.v1",
            "0.1.0",
            json!({
                "name": "ghost",
                "catalogPath": "/data/root/vps.json",
                "removed": false,
                "written": false,
                "action": "not_found",
                "remaining": 2
            }),
        );
        let d: VpsRmData = e.data_as().unwrap();
        assert_eq!(d.action, "not_found");
        assert!(!d.removed && !d.written, "文件一个字节都不动");
        assert_eq!(d.remaining, 2);
    }

    #[test]
    fn 编目册骨架_闸三态与条目行_缺字段给破折号不自造() {
        let g = CatalogGate {
            id: "register.port".into(),
            status: "skip".into(),
            message: "前面已拒".into(),
        };
        assert_eq!(g.state(), State::Skip, "skip = 没跑到这道闸,不是错误");
        assert!(!g.is_fail());
        let e = CatalogEntryView::default();
        assert_eq!(e.addr_line(), "——");
        assert_eq!(e.login_line(), "——");
        assert_eq!(e.ref_line(), "——");
        // 编目册投影:字段恒在不为 null
        let sheet = CatalogSheet::default();
        assert!(sheet.entries.is_empty() && sheet.gates.is_empty());
        assert!(sheet.catalog_path.is_empty());
    }

    // ---- #54 标签册:三形状 + 反查两态 + 悬空清理明示 ----

    #[test]
    fn 解析_tag_ls_data_反查两态同一形状_无参列全部带参最多一条() {
        // 无参 = 列全部标签及各引用
        let all = envelope_with_schema(
            "nw/tag-ls.v1",
            "0.1.0",
            json!({
                "catalogPath": "/data/root/tags.json",
                "tags": [
                    {"name": "pi", "refs": [
                        {"kind": "skill", "id": "deep-research"},
                        {"kind": "vps", "id": "gate"}
                    ]},
                    {"name": "bare", "refs": []}
                ]
            }),
        );
        let d: TagLsData = all.data_as().unwrap();
        let sheet = d.sheet();
        assert_eq!(sheet.entries.len(), 2);
        // 确定性排序:同一份回音永远同一屏(bare < pi)
        assert_eq!(sheet.entries[0].name, "bare");
        assert_eq!(sheet.entries[1].name, "pi");
        assert_eq!(
            sheet.entries[1].ref_line(),
            "skill/deep-research · vps/gate"
        );
        assert_eq!(sheet.entries[0].ref_line(), "——", "空引用显式破折号,不补全");
        // 带参 = 只列那一条(同一 data 形状,最多一条)
        let one = envelope_with_schema(
            "nw/tag-ls.v1",
            "0.1.0",
            json!({
                "catalogPath": "/data/root/tags.json",
                "tags": [{"name": "pi", "refs": [{"kind": "skill", "id": "deep-research"}]}]
            }),
        );
        let d1: TagLsData = one.data_as().unwrap();
        assert_eq!(d1.sheet().entries.len(), 1, "带参反查最多一条");
        assert_eq!(d1.sheet().entries[0].name, "pi");
    }

    #[test]
    fn 解析_tag_ls_data_编目不存在是空数组不是错误() {
        // 合法空态:tags.json 不存在 = 空册 rc 0(与「带参查不到 = rc 3」
        // 是两种语义,面板分开呈现,不都画成空列表)
        let e = envelope_with_schema(
            "nw/tag-ls.v1",
            "0.1.0",
            json!({"catalogPath": "/data/root/tags.json", "tags": []}),
        );
        let d: TagLsData = e.data_as().unwrap();
        assert!(d.sheet().entries.is_empty());
    }

    #[test]
    fn 解析_tag_add_data_四道闸恒四行_悬空清理条数在场() {
        let e = envelope_with_schema(
            "nw/tag-add.v1",
            "0.1.0",
            json!({
                "name": "pi",
                "catalogPath": "/data/root/tags.json",
                "written": true,
                "updated": false,
                "action": "added",
                "refs": [{"kind": "skill", "id": "deep-research"}],
                "cleaned": [
                    {"kind": "extension", "id": "gone-pack"},
                    {"kind": "vps", "id": "ghost"}
                ],
                "gates": [
                    {"id": "tag.add.name", "status": "pass", "message": "标签认"},
                    {"id": "tag.add.ref-kind", "status": "pass", "message": "kind 认"},
                    {"id": "tag.add.ref-id", "status": "pass", "message": "条目认"},
                    {"id": "tag.add.ref-exists", "status": "pass", "message": "对端编目里有"}
                ]
            }),
        );
        let d: TagAddData = e.data_as().unwrap();
        assert_eq!(d.gates.len(), 4, "四道闸恒 4 行");
        assert_eq!(d.gates[3].id, "tag.add.ref-exists");
        assert!(!d.has_fail());
        assert!(d.written && !d.updated, "第一次挂 = 写");
        // 悬空清理:条数必须能原样数出来(界面明示,不静默)
        assert_eq!(d.cleaned_count(), 2);
        assert_eq!(d.cleaned[0].label(), "extension/gone-pack");
        let sheet = d.sheet();
        assert_eq!(sheet.entries[0].name, "pi");
        assert_eq!(sheet.entries[0].ref_line(), "skill/deep-research");
        assert_eq!(sheet.gates.len(), 4);
    }

    #[test]
    fn 解析_tag_add_data_任一闸不过则一个字节都不落() {
        // 第③道闸(ref-id)不过:引擎不写文件,data 的 written/updated 皆假
        let e = envelope_with_schema(
            "nw/tag-add.v1",
            "0.1.0",
            json!({
                "name": "pi",
                "catalogPath": "/data/root/tags.json",
                "written": false,
                "updated": false,
                "action": "rejected",
                "refs": [],
                "cleaned": [],
                "gates": [
                    {"id": "tag.add.name", "status": "pass", "message": "标签认"},
                    {"id": "tag.add.ref-kind", "status": "pass", "message": "kind 认"},
                    {"id": "tag.add.ref-id", "status": "fail", "message": "条目名为空"},
                    {"id": "tag.add.ref-exists", "status": "skip", "message": "前面已拒"}
                ]
            }),
        );
        let d: TagAddData = e.data_as().unwrap();
        assert!(d.has_fail(), "有 fail 就不得算落账");
        assert!(!d.written && !d.updated, "一个字节都不落");
        assert_eq!(d.gates[2].state(), State::Fail);
        assert_eq!(d.gates[3].state(), State::Skip, "skip = 没跑到这道闸");
        assert_eq!(d.cleaned_count(), 0, "没落盘就没有悬空清理");
    }

    #[test]
    fn 解析_tag_add_data_重跑同一条命令字节不变_幂等() {
        // 追加合并、幂等:第二次同 argv 回音 updated 而 refs 不变
        let e = envelope_with_schema(
            "nw/tag-add.v1",
            "0.1.0",
            json!({
                "name": "pi",
                "catalogPath": "/data/root/tags.json",
                "written": false,
                "updated": true,
                "action": "unchanged",
                "refs": [{"kind": "skill", "id": "deep-research"}],
                "cleaned": [],
                "gates": [
                    {"id": "tag.add.name", "status": "pass", "message": ""},
                    {"id": "tag.add.ref-kind", "status": "pass", "message": ""},
                    {"id": "tag.add.ref-id", "status": "pass", "message": ""},
                    {"id": "tag.add.ref-exists", "status": "pass", "message": ""}
                ]
            }),
        );
        let d: TagAddData = e.data_as().unwrap();
        assert!(!d.written && d.updated, "重跑 = 不写新字节,只报已合并");
        assert_eq!(d.sheet().entries[0].refs.len(), 1, "引用不翻倍");
    }

    #[test]
    fn 解析_tag_rm_data_两态与删不存在的标签不动文件() {
        // 带 --ref 只摘引用:标签还在
        let one = envelope_with_schema(
            "nw/tag-rm.v1",
            "0.1.0",
            json!({
                "name": "pi",
                "catalogPath": "/data/root/tags.json",
                "removed": true,
                "written": true,
                "action": "ref_removed",
                "remaining": 1
            }),
        );
        let d: TagRmData = one.data_as().unwrap();
        assert_eq!(d.action, "ref_removed", "第三态:只摘引用,标签还在");
        assert!(d.removed && d.written);
        // 删不存在的标签:rc 3 + 证据 + 文件一个字节都不动
        let ghost = envelope_with_schema(
            "nw/tag-rm.v1",
            "0.1.0",
            json!({
                "name": "ghost",
                "catalogPath": "/data/root/tags.json",
                "removed": false,
                "written": false,
                "action": "not_found",
                "remaining": 2
            }),
        );
        let g: TagRmData = ghost.data_as().unwrap();
        assert_eq!(g.action, "not_found");
        assert!(!g.removed && !g.written, "文件一个字节都不动");
        assert_eq!(g.remaining, 2);
    }

    // ---- #97 钥匙册取回本体:唯一回显槽 + Debug 不打印 ----

    /// `nw/key-get.v1` 的 data 只有名字与本体两格(#97 代跑 get 消费的既有
    /// 形状,契约守卫零改动)。形状不匹配(缺 `key` / 类型不对)时 `data_as`
    /// 给 None——面板不按旧形状降级解析、不猜(取不到就回指路)。
    #[test]
    fn 解析_key_get_data_只有名字与本体两格_形状不符不猜() {
        let ok = envelope_with_schema(
            "nw/key-get.v1",
            "0.1.0",
            json!({"name": "ci", "key": "ghp_fakeCIToken000000000000000000"}),
        );
        let d: KeyGetData = ok.data_as().expect("既有形状,零改动");
        assert_eq!(d.name, "ci");
        assert_eq!(d.key.len(), 33, "本体字节数可报(内容不进任何字符串)");
        // Debug 只说字节数:这个中转类型的 `{:?}` 不是第二处泄漏面
        let dbg = format!("{d:?}");
        assert!(!dbg.contains("ghp_"), "Debug 不打印本体:{dbg}");
        assert!(dbg.contains("不打印内容"), "Debug 只报字节数:{dbg}");
        // 形状不符:不猜、不给 None 之外的第二条路
        let bad = envelope_with_schema("nw/key-get.v1", "0.1.0", json!({"name": "ci"}));
        assert!(
            bad.data_as::<KeyGetData>().is_some(),
            "缺 key 槽 = 空串(上游不带这一格时面板按取不到处理,不按旧形状猜)"
        );
        let wrong = envelope_with_schema("nw/key-get.v1", "0.1.0", json!({"name": 7, "key": []}));
        assert!(
            wrong.data_as::<KeyGetData>().is_none(),
            "类型不匹配 = 不可定型,走指路"
        );
    }
}
