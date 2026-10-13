//! 作者侧脚手架的**落点计算**(spec #95 code-review H1):组件家目录名
//! (`skills/` / `extensions/`)与骨架文件名的路径知识只有一个家——本模块,
//! 与读侧同模块的 [`SKILLS_DIR`](super::SKILLS_DIR) /
//! [`EXTENSIONS_DIR`](super::EXTENSIONS_DIR) 同源。
//!
//! 屏幕层的脚手架表单交进来的是**意图级形状**(「条目名 + 内容」),拿回去的是
//! 拟落账条目([`write::PlannedWrite`]);落点落在哪个家、文件名是什么,屏幕层
//! 不知道。于是「屏幕层零路径拼接」这条接缝纪律(#101 立、#116 覆盖读侧,
//! 由 [`super::tests::写侧两屏零路径_api`] 钉死)在写侧也闭合:读侧
//! `reference_scan` 与写侧 [`skill_write`] / [`ext_writes`] 认同一批组件家,
//! 脚手架落下的条目因此**进得了引用图**,不会因为屏幕层另拼一套路径而断链。
//!
//! 产出内容仍是**面板自有资产**(内置模板 + 表单字段,ADR-0011 §1):本模块只算
//! 落点、不生成正文——正文在屏幕层的表单规格里。

use crate::datarepo::write::{self, PlannedWrite, RepoRelPath};

use super::{EXTENSIONS_DIR, SKILLS_DIR};

/// skill 骨架的文件名(上游 skill 契约:`skills/<名>/SKILL.md`,frontmatter 的
/// `name` 就是身份)。
const SKILL_FILE: &str = "SKILL.md";

/// extension 条目清单的文件名:上游样本
/// `docs/samples/components/scout/package.json`。
const EXT_MANIFEST_FILE: &str = "package.json";

/// 脚手架落点阶段的拒件(面板侧就拒,留表单可改;人话供屏上直接摆)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScaffoldError {
    /// 条目名 / 文件名不是单段:它们直接拼进落点,带分隔符会爬到别的条目
    /// 目录里去(表单已校验过一层,这里是防线加深)。
    NotOneSegment(String),
    /// 拼得出的串不是合法的数据仓相对路径(同上,防线加深)。
    BadPath(write::RepoRelPathError),
    /// 清单与入口文件对不上(上游 register 验收第六条的面板侧镜像)。
    Audit(String),
}

impl ScaffoldError {
    /// 人话原因(不带数据仓根:根是接缝内部的事,屏上只摆条目名 + 原因)。
    pub fn why(&self) -> String {
        match self {
            Self::NotOneSegment(s) => format!(
                "`{s}` 不是单段:落点按 `<组件家>/<条目名>/<文件>` 拼,条目名与文件名都不带分隔符"
            ),
            Self::BadPath(e) => format!("落点拼不出来:{}", e.why()),
            Self::Audit(r) => r.clone(),
        }
    }
}

/// skill 骨架的拟落账条目(意图级:交「名 + 正文」,拿回一条将写)。
///
/// 落点 = 数据仓 `skills/<名>/SKILL.md`。名同时是 skill 身份(frontmatter 的
/// `name`)与落点目录名,故按 [`RepoRelPath`] 再挡一次——表单已校验过一层,
/// 这是防线加深:名字直接进路径,穿越字符不该到了这里还存在。
pub fn skill_write(name: &str, md: &str) -> Result<PlannedWrite, ScaffoldError> {
    let rel = entry_rel(SKILLS_DIR, name, SKILL_FILE)?;
    Ok(PlannedWrite::text(rel, md))
}

/// extension 条目骨架的拟落账条目(意图级:交「条目名 + 清单正文 + 入口文件名
/// + 入口正文」,拿回两条将写)。
///
/// 落点 = 数据仓 `extensions/<名>/` 下的 `package.json` 与入口文件(上游 #21:
/// 扩展没有 frontmatter 可当身份,条目身份 = 子树目录名)。两条一并自审
/// ——清单指着入口、入口在树里,「装上了不加载」的形状过不去。
pub fn ext_writes(
    name: &str,
    manifest: &str,
    entry: &str,
    entry_body: &str,
) -> Result<Vec<PlannedWrite>, ScaffoldError> {
    let manifest_rel = entry_rel(EXTENSIONS_DIR, name, EXT_MANIFEST_FILE)?;
    let entry_file_rel = entry_rel(EXTENSIONS_DIR, name, entry)?;
    let writes = vec![
        PlannedWrite::text(manifest_rel, manifest),
        PlannedWrite::text(entry_file_rel, entry_body),
    ];
    if let Some(reason) = ext_manifest_audit(name, &writes) {
        return Err(ScaffoldError::Audit(reason));
    }
    Ok(writes)
}

/// 组件家之下的一个条目文件——**落点拼装的唯一一处**:家目录 + 条目名 + 文件名。
///
/// 条目名与文件名都必须是**单段**(带 `/` 会爬到别的条目目录下去,带 `\` 在
/// 数据仓路径里一律不合法),再交给 [`RepoRelPath`] 挡绝对路径 / `..` / `~`
/// ——表单已校验过一层,这里是防线加深。
fn entry_rel(home: &str, name: &str, file: &str) -> Result<RepoRelPath, ScaffoldError> {
    for seg in [name, file] {
        if seg.is_empty() || seg.contains('/') || seg.contains('\\') {
            return Err(ScaffoldError::NotOneSegment(seg.to_string()));
        }
    }
    RepoRelPath::new(&format!("{home}/{name}/{file}")).map_err(ScaffoldError::BadPath)
}

/// 上游 register 验收第六条(仅 `--kind extension`,#21)的面板侧镜像:
/// 条目子树自带 `package.json`、`pi.extensions` 非空、清单里每条路径在条目
/// 子树里真实存在。判据逐条对齐上游 `internal/source/scan.go` 的
/// `ExtensionManifestIssue`(路径不许绝对、不许越级,匹配得到文件)。
///
/// 面板自己生成 manifest 与入口两边,这个自审把「两边对不上」挡在拟落账之前
/// ——那种形状上游 register 会拒(装上了不加载),不该流到确认闸。
pub fn ext_manifest_audit(name: &str, writes: &[PlannedWrite]) -> Option<String> {
    let Ok(manifest_rel) = entry_rel(EXTENSIONS_DIR, name, EXT_MANIFEST_FILE) else {
        return Some(format!(
            "条目名 `{name}` 形状不合:拼不出 {EXTENSIONS_DIR}/<名>/ 落点"
        ));
    };
    let at = |rel: &str| writes.iter().find(|w| w.path.as_str() == rel);
    let Some(manifest) = at(manifest_rel.as_str()) else {
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
        let want = entry_rel(EXTENSIONS_DIR, name, rel)
            .map(|r| r.as_str().to_string())
            .unwrap_or_default();
        if want.is_empty() || at(&want).is_none() {
            return Some(format!(
                "pi.extensions 里的 {p:?} 在条目子树里一个文件都没匹配上"
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rel: &str, body: &str) -> PlannedWrite {
        PlannedWrite::text(RepoRelPath::new(rel).unwrap(), body)
    }

    fn paths(writes: &[PlannedWrite]) -> Vec<&str> {
        writes.iter().map(|w| w.path.as_str()).collect()
    }

    // ---- 落点:组件家知识只在本模块,读写同源 ----

    #[test]
    fn skill落点_落在技能家_文件名是上游契约的skill_md() {
        let w = skill_write("web-scout", "---\nname: web-scout\n---\n").unwrap();
        assert_eq!(w.path.as_str(), "skills/web-scout/SKILL.md");
        assert_eq!(
            String::from_utf8(w.content).unwrap(),
            "---\nname: web-scout\n---\n"
        );
    }

    #[test]
    fn extension落点_清单与入口两件_都落在扩展家() {
        let writes = ext_writes(
            "ribbon",
            "{\n  \"pi\": { \"extensions\": [\"./index.ts\"] }\n}\n",
            "index.ts",
            "export default {}\n",
        )
        .unwrap();
        assert_eq!(
            paths(&writes),
            vec![
                "extensions/ribbon/package.json",
                "extensions/ribbon/index.ts"
            ]
        );
    }

    #[test]
    fn 条目名带分隔符或穿越段_拼不出落点_防线加深() {
        // 表单已校验过一层(invalid_entry_name);这里是双证——名字直接进路径。
        for bad in ["a/b", "..", "a\\b"] {
            assert!(
                skill_write(bad, "x").is_err(),
                "{bad:?} 应被拒:拼不出 skills/<名>/ 落点"
            );
            assert!(
                ext_writes(bad, "{}", "index.ts", "x").is_err(),
                "{bad:?} 应被拒:拼不出 extensions/<名>/ 落点"
            );
        }
        // 入口文件名带分隔符同样爬不出去
        assert!(ext_writes("ribbon", "{}", "sub/index.ts", "x").is_err());
        let err = skill_write("a/b", "x").unwrap_err();
        assert!(
            matches!(err, ScaffoldError::NotOneSegment(_)),
            "带分隔符的名字是单段拒件:{err:?}"
        );
        assert!(err.why().contains("不是单段"), "人话点题:{}", err.why());
        let err2 = skill_write("..", "x").unwrap_err();
        assert!(
            matches!(err2, ScaffoldError::BadPath(_)),
            "`..` 段由 RepoRelPath 挡:{err2:?}"
        );
    }

    // ---- 自审:上游 register 验收第六条的面板侧镜像 ----

    #[test]
    fn 自审_清单与入口对得上才放行() {
        let ok = ext_writes(
            "ribbon",
            "{\n  \"pi\": { \"extensions\": [\"./index.ts\"] }\n}\n",
            "index.ts",
            "x",
        )
        .unwrap();
        assert_eq!(ext_manifest_audit("ribbon", &ok), None);
        // 自定义入口名也放行(清单跟着入口走)
        let custom = ext_writes(
            "ribbon",
            "{\n  \"pi\": { \"extensions\": [\"./main.js\"] }\n}\n",
            "main.js",
            "x",
        )
        .unwrap();
        assert_eq!(ext_manifest_audit("ribbon", &custom), None);
    }

    #[test]
    fn 自审_四种对不上一律拦_且过ext_writes时是audit拒件() {
        // ① 缺 manifest
        let no_manifest = vec![text("extensions/ribbon/index.ts", "x")];
        assert!(ext_manifest_audit("ribbon", &no_manifest)
            .unwrap()
            .contains("package.json"));
        // ② pi.extensions 为空
        let empty_pi = vec![text(
            "extensions/ribbon/package.json",
            "{\n  \"pi\": {\n    \"extensions\": []\n  }\n}\n",
        )];
        assert!(ext_manifest_audit("ribbon", &empty_pi)
            .unwrap()
            .contains("pi.extensions"));
        // ③ 清单指着一个不存在的文件(= 装上了不加载)
        let dangling = vec![
            text(
                "extensions/ribbon/package.json",
                "{\n  \"pi\": {\n    \"extensions\": [\"./gone.ts\"]\n  }\n}\n",
            ),
            text("extensions/ribbon/index.ts", "x"),
        ];
        assert!(ext_manifest_audit("ribbon", &dangling)
            .unwrap()
            .contains("一个文件都没匹配上"));
        // ④ 绝对路径 / 越级
        for bad in ["/etc/passwd", "../outside.ts"] {
            let evil = vec![text(
                "extensions/ribbon/package.json",
                &format!("{{\n  \"pi\": {{\n    \"extensions\": [\"{bad}\"]\n  }}\n}}\n"),
            )];
            assert!(ext_manifest_audit("ribbon", &evil)
                .unwrap()
                .contains("不许绝对路径或越级"));
        }
        // 经 ext_writes 进来时,自审不过 = Audit 拒件(不流到确认闸)
        let err = ext_writes(
            "ribbon",
            "{\n  \"pi\": { \"extensions\": [\"./gone.ts\"] }\n}\n",
            "index.ts",
            "x",
        )
        .unwrap_err();
        assert!(
            matches!(err, ScaffoldError::Audit(_)),
            "清单与入口对不上是 audit 拒件:{err:?}"
        );
    }

    #[test]
    fn 自审_引号与反斜杠的manifest仍是合法json() {
        // 描述进 manifest 的 JSON 串:转义后仍要解析得动(自审按 JSON 读)
        let manifest = "{\n  \"name\": \"ribbon\",\n  \"description\": \"他说\\\"你好\\\"\",\n  \"pi\": { \"extensions\": [\"./index.ts\"] }\n}\n";
        let writes = ext_writes("ribbon", manifest, "index.ts", "x").unwrap();
        assert_eq!(ext_manifest_audit("ribbon", &writes), None);
    }
}
