//! 外部源登记向导的预扫(工单 #114):面板侧拉取目标仓的条目选单。
//!
//! 边界(ADR-0003 的延长,spec #95  Implementation Decisions「外部源向导」):
//! 预扫是**只读**的 GitHub 查询(经 `gh api`),不落任何文件、不调引擎;
//! 登记本身仍 100% 走引擎 `nw source register` 两段式——这里只生成面板侧
//! 的选单。扫不出(私有仓/网络/非 GitHub 链接/树过大)一律由调用方回落
//! 既有单条登记流,不报死、不拦路。
//!
//! 发现规则对齐上游 `docs/external-sources.md`(只读知识,不造):
//! - skill:任何 `skills/**/SKILL.md` → 条目 = 该文件所在目录(递归发现);
//! - extension:`extensions/<名>.ts`,或 `extensions/<名>/` 带
//!   `index.ts` / `index.js` / `package.json` → 条目 = `extensions/<名>`。
//!
//! 面板的选单是 advisory:引擎 register 的扫描与验收仍为准。

use std::collections::BTreeSet;
use std::process::Stdio;
use tokio::process::Command;

/// 预扫发现的一条条目(仓内相对路径 + 面板侧分类)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScannedEntry {
    /// 仓内相对路径,如 `skills/web-scout`、`extensions/foo`。
    pub path: String,
    /// 面板侧分类(供选单分组;引擎扫描仍为准)。
    pub kind: ScanKind,
}

/// 条目面板侧分类(与引擎 `--kind` 槽位同词,不另造)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanKind {
    Skill,
    Extension,
}

impl ScanKind {
    /// 引擎 `--kind` 字面。
    pub fn word(self) -> &'static str {
        match self {
            ScanKind::Skill => "skill",
            ScanKind::Extension => "extension",
        }
    }
}

/// 预扫结论:条目选单(可能为空),或失败理由(调用方回落单条登记流)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanOutcome {
    Entries(Vec<ScannedEntry>),
    Failed(String),
}

/// 预扫回音(经主循环的扫描通道回流;`slug` 供对账,迟到回音按状态丢弃)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScanEvent {
    Finished { slug: String, outcome: ScanOutcome },
}

/// 把用户粘贴的仓库链接规范成 `owner/repo`(纯函数)。
///
/// 接受三种形态(与登记输入态的屏上提示同口径):`owner/repo` 裸形、
/// `https://github.com/owner/repo(.git)`、`git@github.com:owner/repo(.git)`。
/// 非 GitHub 主机、形状不完整、段内字符越界 → `None`(调用方拒绝/跳过)。
/// 这是 repo 链接形状的**唯一**出处(登记输入态的放行判据也用它)。
pub fn repo_slug(input: &str) -> Option<String> {
    let s = input.trim();
    let s = s.strip_suffix(".git").unwrap_or(s);
    let core = if let Some(rest) = s.strip_prefix("https://github.com/") {
        rest
    } else if let Some(rest) = s.strip_prefix("git@github.com:") {
        rest
    } else if s.starts_with("http") || s.starts_with("git@") || s.contains("://") {
        return None; // 别的主机/协议:不猜
    } else {
        s
    };
    let core = core.trim_matches('/');
    let mut parts = core.split('/');
    let owner = parts.next()?;
    let name = parts.next()?;
    if parts.next().is_some() {
        return None; // 只认 owner/repo,更深路径不猜
    }
    let seg_ok = |seg: &str| {
        !seg.is_empty()
            && seg
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    if !seg_ok(owner) || !seg_ok(name) {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

/// 预扫的 gh argv(纯函数):一次拉默认分支全树(递归)。
/// `HEAD` 由 GitHub 解析为默认分支(与上游 bake「现拉默认分支 HEAD」同源)。
pub fn gh_tree_args(slug: &str) -> Vec<String> {
    vec![
        "api".into(),
        format!("repos/{slug}/git/trees/HEAD?recursive=1"),
    ]
}

/// 解析 gh 的 git/trees 回音(纯函数)→ 条目选单。
///
/// 树被 GitHub 截断(超大仓)按失败处理:半份选单冒充全量会误导作者,
/// 回落单条登记流让引擎自己去扫。条目去重、组内排序,输出确定。
pub fn parse_tree(json: &str) -> Result<Vec<ScannedEntry>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("预扫回音不是 JSON:{e}"))?;
    if v.get("truncated").and_then(|t| t.as_bool()) == Some(true) {
        return Err("仓库树过大,预扫结果被 GitHub 截断".into());
    }
    let tree = v
        .get("tree")
        .and_then(|t| t.as_array())
        .ok_or("预扫回音缺 tree 数组")?;
    let mut skills: BTreeSet<String> = BTreeSet::new();
    let mut exts: BTreeSet<String> = BTreeSet::new();
    for item in tree {
        let path = item.get("path").and_then(|p| p.as_str()).unwrap_or("");
        // skill:skills/.../SKILL.md → 条目 = 所在目录(递归发现,深一层也算)
        if let Some(dir) = path
            .strip_prefix("skills/")
            .and_then(|r| r.strip_suffix("/SKILL.md"))
        {
            if !dir.is_empty() {
                skills.insert(format!("skills/{dir}"));
            }
        }
        // extension:extensions/<名>.ts,或 extensions/<名>/{index.ts,index.js,package.json}
        if let Some(rest) = path.strip_prefix("extensions/") {
            if let Some(name) = rest.strip_suffix(".ts") {
                if !name.is_empty() && !name.contains('/') {
                    exts.insert(format!("extensions/{name}"));
                }
            } else if let Some((name, file)) = rest.split_once('/') {
                if !name.is_empty() && matches!(file, "index.ts" | "index.js" | "package.json") {
                    exts.insert(format!("extensions/{name}"));
                }
            }
        }
    }
    Ok(skills
        .into_iter()
        .map(|p| ScannedEntry {
            path: p,
            kind: ScanKind::Skill,
        })
        .chain(exts.into_iter().map(|p| ScannedEntry {
            path: p,
            kind: ScanKind::Extension,
        }))
        .collect())
}

/// 生产预扫:跑 `gh api`(只读)。gh 不在 PATH / 网络 / 私有仓 / 回音读不懂
/// → [`ScanOutcome::Failed`](调用方回落单条登记流)。
pub async fn scan_repo(slug: &str) -> ScanOutcome {
    let out = match Command::new("gh")
        .args(gh_tree_args(slug))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(o) => o,
        Err(e) => return ScanOutcome::Failed(format!("预扫拉不起 gh({e});回落单条登记")),
    };
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let first = err
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim();
        return ScanOutcome::Failed(format!("预扫失败({slug}):{first}"));
    }
    match parse_tree(&String::from_utf8_lossy(&out.stdout)) {
        Ok(entries) => ScanOutcome::Entries(entries),
        Err(e) => ScanOutcome::Failed(format!("预扫回音读不懂({slug}):{e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(paths: &[&str]) -> String {
        let items: Vec<String> = paths
            .iter()
            .map(|p| format!(r#"{{"path":"{p}","mode":"100644","type":"blob","sha":"x"}}"#))
            .collect();
        format!(
            r#"{{"sha":"t","tree":[{}],"truncated":false}}"#,
            items.join(",")
        )
    }

    #[test]
    fn repo_slug_认三种形态_拒非github与残缺() {
        assert_eq!(repo_slug("owner/repo"), Some("owner/repo".into()));
        assert_eq!(
            repo_slug("https://github.com/owner/repo"),
            Some("owner/repo".into())
        );
        assert_eq!(
            repo_slug("https://github.com/owner/repo.git"),
            Some("owner/repo".into())
        );
        assert_eq!(
            repo_slug("https://github.com/owner/repo/"),
            Some("owner/repo".into())
        );
        assert_eq!(
            repo_slug("git@github.com:owner/repo.git"),
            Some("owner/repo".into())
        );
        assert_eq!(repo_slug("  owner/repo  "), Some("owner/repo".into()));
        // 拒:别的主机、残缺、深路径、空段、越界字符
        assert_eq!(repo_slug("https://gitlab.com/owner/repo"), None);
        assert_eq!(repo_slug("http://github.com/owner/repo"), None);
        assert_eq!(repo_slug("owner"), None);
        assert_eq!(repo_slug("owner/repo/tree/main"), None);
        assert_eq!(repo_slug("/repo"), None);
        assert_eq!(repo_slug("owner/"), None);
        assert_eq!(repo_slug("own er/repo"), None);
        assert_eq!(repo_slug(""), None);
    }

    #[test]
    fn gh_tree_args_一次拉全树() {
        assert_eq!(
            gh_tree_args("owner/repo"),
            vec![
                "api".to_string(),
                "repos/owner/repo/git/trees/HEAD?recursive=1".to_string()
            ]
        );
    }

    #[test]
    fn parse_tree_按上游发现规则取条目_去重排序确定() {
        let json = tree(&[
            "README.md",
            "skills/greeter/SKILL.md",
            "skills/scanner/SKILL.md",
            "skills/deep/nest/SKILL.md", // 递归发现:深一层也算
            "skills/SKILL.md",           // 根上的 SKILL.md 不是条目
            "skills/greeter/reference.md",
            "extensions/foo/index.ts",
            "extensions/foo/package.json", // 同一条目去重
            "extensions/bar.ts",           // 单文件扩展
            "extensions/baz/README.md",    // 无入口无清单:跳过
            "docs/x.md",
        ]);
        let entries = parse_tree(&json).expect("合法回音");
        assert_eq!(
            entries,
            vec![
                ScannedEntry {
                    path: "skills/deep/nest".into(),
                    kind: ScanKind::Skill
                },
                ScannedEntry {
                    path: "skills/greeter".into(),
                    kind: ScanKind::Skill
                },
                ScannedEntry {
                    path: "skills/scanner".into(),
                    kind: ScanKind::Skill
                },
                ScannedEntry {
                    path: "extensions/bar".into(),
                    kind: ScanKind::Extension
                },
                ScannedEntry {
                    path: "extensions/foo".into(),
                    kind: ScanKind::Extension
                },
            ]
        );
    }

    #[test]
    fn parse_tree_截断按失败_不呈半份选单() {
        let json =
            r#"{"sha":"t","tree":[{"path":"skills/a/SKILL.md","type":"blob"}],"truncated":true}"#;
        let err = parse_tree(json).expect_err("截断=失败");
        assert!(err.contains("截断"), "理由说清截断:{err}");
    }

    #[test]
    fn parse_tree_坏回音给理由不panic() {
        assert!(parse_tree("not json").is_err());
        assert!(parse_tree(r#"{"sha":"t"}"#).is_err());
    }
}
