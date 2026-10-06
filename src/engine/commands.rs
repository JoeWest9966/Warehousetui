//! typed 命令构造。面板只发三类动作(ADR-0003)+ 版本探测 + 只读 `ls`。

/// 五域总览。空 `repo` 时不带 `--repo`。
pub fn ls(repo: Option<&str>) -> Vec<String> {
    let mut a = vec!["ls".to_string()];
    if let Some(r) = repo {
        a.push("--repo".into());
        a.push(r.into());
    }
    a
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
    fn ls_可选_repo() {
        assert_eq!(ls(None), vec!["ls"]);
        assert_eq!(ls(Some(".")), vec!["ls", "--repo", "."]);
    }
}
