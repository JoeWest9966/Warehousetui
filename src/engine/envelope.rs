//! envelope:`nw` stdout 的唯一输出面,字段级契约定型。
//!
//! 铁律(来自 Nous-Warehouse #9 与调研底稿):
//! - stdout 永远是**一份** JSON envelope,人话/进度/警告全走 stderr;
//! - `ok ≡ (退出码 == 0)`;
//! - 未知字段容忍(前向兼容),绝不 deny_unknown_fields;
//! - details 可含任意 JSON,原样透传给 UI 展示,不回显密钥。

use serde::Deserialize;
use serde_json::Value;

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
}

impl Envelope {
    /// 按 schema 拆出 data 的定型视图;形状不匹配时返回 None(UI 走证据展示)。
    pub fn data_as<T: for<'de> Deserialize<'de>>(&self) -> Option<T> {
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
    #[serde(default)]
    pub lock_id: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub recipe: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub runtime_key: Option<String>,
    #[serde(default)]
    pub installed_at: Option<String>,
    #[serde(default)]
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

#[derive(Clone, Debug, Deserialize)]
pub struct SourceRow {
    pub name: String,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub entries: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProfileRow {
    pub name: String,
    /// ok | degraded | drift | broken,与 doctor rc 一一对应。
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub lock_id: Option<String>,
    #[serde(default)]
    pub installed_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LockRow {
    #[serde(default)]
    pub lock_id: Option<String>,
    #[serde(default)]
    pub recipe: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
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
}
