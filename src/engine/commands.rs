//! typed 命令构造。面板只发三类动作(ADR-0003)+ 版本探测 + 只读 `ls`。

/// 五域总览。数据仓根**显式携带**——签名强制带 repo,
/// 类型层面杜绝「不带 --repo 让引擎拿当前目录兜底」(spec #24,票 #27)。
/// 缺根时调用方不应构造命令(下发前有门控,见 `App::spawn_engine`)。
pub fn ls(repo: &str) -> Vec<String> {
    vec!["ls".to_string(), "--repo".into(), repo.to_string()]
}

/// 装配。`lock` 可以是 `.nwlock` 或装配命令文件。
pub fn up(lock: &str, nw_home: Option<&str>) -> Vec<String> {
    let mut a = vec!["up".to_string(), "--lock".into(), lock.to_string()];
    if let Some(h) = nw_home {
        a.push("--nw-home".into());
        a.push(h.into());
    }
    a
}

/// 健康体检:按派名(profile)。
pub fn doctor_profile(profile: &str, nw_home: Option<&str>) -> Vec<String> {
    let mut a = vec![
        "doctor".to_string(),
        "--profile".into(),
        profile.to_string(),
    ];
    if let Some(h) = nw_home {
        a.push("--nw-home".into());
        a.push(h.into());
    }
    a
}

/// 健康体检:按锁定集。
pub fn doctor_lock(lock: &str, nw_home: Option<&str>) -> Vec<String> {
    let mut a = vec!["doctor".to_string(), "--lock".into(), lock.to_string()];
    if let Some(h) = nw_home {
        a.push("--nw-home".into());
        a.push(h.into());
    }
    a
}

/// 校验:配方侧。
pub fn validate_recipe(recipe: &str) -> Vec<String> {
    vec!["validate".into(), "--recipe".into(), recipe.into()]
}

/// 校验:锁定集侧。
pub fn validate_lock(lock: &str) -> Vec<String> {
    vec!["validate".into(), "--lock".into(), lock.into()]
}

/// 版本探测。注意:`nw --version` 的裸版本号走 **stderr**。
pub fn version() -> Vec<String> {
    vec!["--version".to_string()]
}

/// 外部源登记·预审段(spec #25 ticket #30):扫描 + 验收 5(+1) 条,
/// 拟落账记录进 JSON,**文件不落**——铁律是不带 `--yes`(确认闸),
/// 本函数连 `--yes` 的位置都没有,从类型层面杜绝预审误落账。
/// `--root` 指数据仓根(与 `ls --repo` 同义的本地仓根槽位)。
pub fn source_register_preaudit(repo: &str, kind: &str, root: &str) -> Vec<String> {
    vec![
        "source".into(),
        "register".into(),
        "--repo".into(),
        repo.to_string(),
        "--kind".into(),
        kind.to_string(),
        "--root".into(),
        root.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn up_带_nw_home() {
        assert_eq!(
            up("x.nwlock", Some("/data")),
            vec!["up", "--lock", "x.nwlock", "--nw-home", "/data"]
        );
    }

    #[test]
    fn doctor_二选一_flag_不混用() {
        assert_eq!(
            doctor_profile("desk", None),
            vec!["doctor", "--profile", "desk"]
        );
        assert_eq!(
            doctor_lock("x.nwlock", None),
            vec!["doctor", "--lock", "x.nwlock"]
        );
    }

    #[test]
    fn ls_必带_repo_不给引擎当前目录兜底() {
        assert_eq!(ls("/data/repo"), vec!["ls", "--repo", "/data/repo"]);
        // 签名里没有 Option:想不带 --repo 都写不出来
        let a = ls(".");
        assert_eq!(a[1], "--repo");
    }

    #[test]
    fn 登记预审_永不带_yes_根槽位指数据仓根() {
        let a = source_register_preaudit("owner/repo", "skill", "/data/root");
        assert_eq!(
            a,
            vec![
                "source",
                "register",
                "--repo",
                "owner/repo",
                "--kind",
                "skill",
                "--root",
                "/data/root"
            ]
        );
        assert!(
            !a.iter().any(|s| s == "--yes"),
            "预审段铁律:argv 里不许出现 --yes(文件不落)"
        );
    }
}
