//! envelope:`nw` stdout 的唯一输出面,字段级契约定型。
//!
//! 铁律(来自 Nous-Warehouse #9 与调研底稿):
//! - stdout 永远是**一份** JSON envelope,人话/进度/警告全走 stderr;
//! - `ok ≡ (退出码 == 0)`;
//! - 未知字段容忍(前向兼容),绝不 deny_unknown_fields;
//! - details 可含任意 JSON,原样透传给 UI 展示,不回显密钥。

use serde::Deserialize;
use serde_json::Value;

/// 契约守卫(#28):面板**认识**的 envelope schema 集合。
///
/// 只收面板认识其 data 形状、或明确知道其存在的引擎 schema(调研底稿
/// §3 的 `nw/<cmd>.v1` 全集)。不在集合里的一律走「需升级 nw」证据:
/// 不 panic、不按旧形状降级解析(`data_as` 直接返回 None)。
/// 这是面板自己的契约知识,不是引擎能力表(那是 #29 的探测门)。
const KNOWN_SCHEMAS: [&str; 8] = [
    "nw/validate.v1",
    "nw/up.v1",
    "nw/doctor.v1",
    "nw/bake.v1",
    "nw/recipe-new.v1",
    "nw/ls.v1",
    "nw/source-register.v1",
    "nw/source-recheck.v1",
];

/// schema 是否在面板认识的集合内(契约守卫的判据)。
fn is_known_schema(schema: &str) -> bool {
    KNOWN_SCHEMAS.contains(&schema)
}

/// 错误/警告条目五字段。warnings 的 code 可缺。
#[derive(Clone, Debug, Deserialize)]
pub struct Evidence {
    #[serde(default)]
    pub code: Option<String>,
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
        self.schema_guard = Some(Evidence {
            code: Some("SCHEMA_UNKNOWN".into()),
            exit_code: Some(rc),
            message: format!(
                "需升级 nw:面板不认识 envelope schema \"{}\"(引擎 nw {})",
                self.schema, self.nw
            ),
            hint: Some("升级 nw 与本面板,使两端 schema 对齐;收到的原文在 details".into()),
            details: Some(serde_json::json!({
                "schema": self.schema,
                "nw": self.nw,
            })),
        });
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
    pub env_present: Vec<String>,
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
}

#[derive(Clone, Debug, Deserialize)]
pub struct ComponentRow {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub depends: Vec<String>,
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

#[derive(Clone, Debug, Deserialize)]
pub struct LockRow {
    #[serde(default, rename = "lockId")]
    pub lock_id: Option<String>,
    #[serde(default)]
    pub recipe: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, rename = "bakedAt")]
    pub baked_at: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub assets: Option<Value>,
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
        assert!(d.env_present.is_empty());
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
    fn 契约守卫_重复合成幂等_不覆盖已有证据() {
        let mut e = envelope_with_schema("nw/doctor.v2", "1.0.0", json!({}));
        e.apply_schema_guard(11);
        let first = e.schema_guard.clone().unwrap();
        e.apply_schema_guard(0);
        assert_eq!(e.schema_guard.unwrap().exit_code, first.exit_code, "幂等");
    }
}
