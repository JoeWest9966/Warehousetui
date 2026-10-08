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

/// 外部源登记·落账段(spec #25 ticket #31):与预审同参,多一枚 `--yes`
/// 确认闸——引擎才把 `catalog.json` 写进数据仓根。**只能由图形确认后的
/// `App::dispatch_register_commit` 调用**;预审段用的
/// [`source_register_preaudit`] 连 `--yes` 的位置都没有,两段从构造函数
/// 形状上分开,误下发靠类型杜绝。
pub fn source_register_commit(repo: &str, kind: &str, root: &str) -> Vec<String> {
    let mut a = source_register_preaudit(repo, kind, root);
    a.push("--yes".into());
    a
}

/// 外部源漂移复核(spec #34 / 工单 #40):**只读**——只报告漂移、永不写账。
/// `name` 非空 = 只查这一条记录;None = 全查已登记源。
/// `--root` 指数据仓根(与 register 同槽位;别与 ls 的 `--repo` 混)。
/// 铁律:本命令不产生、也不允许产生任何写旗标——两段式登记的写入口只有
/// `source_register_commit` 一个,那里才有 `--yes`。
pub fn source_recheck(name: Option<&str>, root: &str) -> Vec<String> {
    let mut a = vec!["source".into(), "recheck".into()];
    if let Some(n) = name {
        a.push("--name".into());
        a.push(n.to_string());
    }
    a.push("--root".into());
    a.push(root.to_string());
    a
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
    fn recheck_只读_按名限定与全查都不带任何写旗标() {
        let all = source_recheck(None, "/data/root");
        assert_eq!(
            all,
            vec!["source", "recheck", "--root", "/data/root"],
            "全查:不带 --name"
        );
        let one = source_recheck(Some("web-search"), "/data/root");
        assert_eq!(one[2], "--name");
        assert_eq!(one[3], "web-search");
        assert_eq!(one[4], "--root");
        for a in [&all, &one] {
            assert!(
                !a.iter().any(|s| s == "--yes"),
                "只读铁律:recheck 的 argv 里不许出现 --yes(写入口只有 register 一个)"
            );
        }
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

    #[test]
    fn 登记落账_带_yes_带_root_不带_category() {
        let a = source_register_commit("owner/repo", "skill", "/data/root");
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
                "/data/root",
                "--yes"
            ]
        );
        assert!(
            a.iter().any(|s| s == "--yes"),
            "落账段铁律:确认闸必须在 argv 里(否则文件不落)"
        );
        assert!(
            !a.iter().any(|s| s == "--category"),
            "类别输入不给(所配引擎无该能力,记录落未分类)"
        );
        // 预审与落账只差确认闸一枚棋:同根、同链、不同后果
        let p = source_register_preaudit("owner/repo", "skill", "/data/root");
        let mut pc = p.clone();
        pc.push("--yes".into());
        assert_eq!(a, pc, "落账 = 预审 + --yes,不另改槽位");
    }
}
