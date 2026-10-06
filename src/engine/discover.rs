//! `nw` 二进制发现:显式 > 配置 > PATH > `$NW_HOME/nw`。
//!
//! 每一级来源都展示给用户(设置屏「生效值 + 来源」),找不到时给出
//! 指路证据而不是一句干巴巴的报错。

use std::path::PathBuf;

use super::config::Config;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// --nw-bin 显式指定。
    Flag,
    /// config.toml 的 nw_bin。
    Config,
    /// PATH 里找到。
    Path,
    /// $NW_HOME/nw。
    NwHome,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Flag => "--nw-bin 指定",
            Source::Config => "配置文件",
            Source::Path => "PATH",
            Source::NwHome => "$NW_HOME/nw",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Discovery {
    pub bin: PathBuf,
    pub source: Source,
}

/// 按优先级发现 `nw`。`attempted` 返回找过的地方(指路屏素材)。
pub fn discover(
    explicit: Option<&str>,
    cfg: &Config,
    nw_home: &str,
) -> Result<Discovery, Vec<String>> {
    let mut attempted = Vec::new();

    if let Some(p) = explicit.filter(|s| !s.is_empty()) {
        let path = PathBuf::from(p);
        if is_executable(&path) {
            return Ok(Discovery {
                bin: path,
                source: Source::Flag,
            });
        }
        attempted.push(format!("--nw-bin {p}(不可执行)"));
    }

    if let Some(p) = cfg.nw_bin.as_deref().filter(|s| !s.is_empty()) {
        let path = expand_tilde(p);
        if is_executable(&path) {
            return Ok(Discovery {
                bin: path,
                source: Source::Config,
            });
        }
        attempted.push(format!("配置 {p}(不可执行)"));
    }

    if let Some(path) = which("nw") {
        return Ok(Discovery {
            bin: path,
            source: Source::Path,
        });
    }
    attempted.push("PATH(无 nw)".into());

    let from_home = expand_tilde(&format!("{nw_home}/nw"));
    if is_executable(&from_home) {
        return Ok(Discovery {
            bin: from_home,
            source: Source::NwHome,
        });
    }
    attempted.push(format!("{}(不存在)", from_home.display()));

    Err(attempted)
}

fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(p)
}

fn is_executable(path: &std::path::Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// 极简 which:按 PATH 顺序找第一个可执行命中。
pub fn which(bin: &str) -> Option<PathBuf> {
    let paths = std::env::var("PATH").ok()?;
    let candidates: Vec<&str> = if cfg!(windows) {
        vec![bin, "nw.exe"]
    } else {
        vec![bin]
    };
    for dir in std::env::split_paths(&paths) {
        for c in &candidates {
            let p = dir.join(c);
            if is_executable(&p) {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 显式优先于一切() {
        let cfg = Config::default();
        let d = discover(Some("/bin/sh"), &cfg, "/nowhere").unwrap();
        assert_eq!(d.source, Source::Flag);
        assert_eq!(d.bin, PathBuf::from("/bin/sh"));
    }

    #[test]
    fn 都不中时给出找过清单() {
        let cfg = Config::default();
        let err = discover(None, &cfg, "/nowhere-xyz").unwrap_err();
        assert!(!err.is_empty());
        assert!(err.iter().any(|s| s.contains("PATH")));
    }

    #[test]
    fn 家目录兜底命中() {
        let cfg = Config::default();
        let home = std::env::var("HOME").unwrap();
        // 伪造一个可执行 $HOME/nw 来验证 NwHome 级(PATH 里恰好没有 nw 时)
        let fake = PathBuf::from(&home).join("nw");
        let original = std::fs::read(&fake).ok();
        let created = std::fs::write(&fake, "#!/bin/sh\n").is_ok();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if created {
                let _ = std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755));
            }
        }
        if which("nw").is_none() && created {
            let d = discover(None, &cfg, &home).unwrap();
            assert_eq!(d.source, Source::NwHome);
            assert_eq!(d.bin, fake);
        }
        let _ = std::fs::remove_file(&fake);
        if let Some(o) = original {
            let _ = std::fs::write(PathBuf::from(&home).join("nw"), o);
        }
    }
}
