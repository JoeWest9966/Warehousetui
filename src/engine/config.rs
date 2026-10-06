//! config.toml:只读为主、最小写入。
//!
//! 位置:`$XDG_CONFIG_HOME/warehousetui/config.toml`,缺省 `~/.config/...`。
//! 铁律:永不存密钥;用户自加的键原样保留(进 extra 再写回)。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const APP_DIR: &str = "warehousetui";
const FILE: &str = "config.toml";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// `nw` 二进制路径(发现策略第二级)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nw_bin: Option<String>,
    /// 默认 NW_HOME。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nw_home: Option<String>,
    /// 主题 warehouse | high_contrast。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// 最近查看的派。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_profile: Option<String>,
    /// 用户自加的键:原样保留,写回时不丢。
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

pub fn config_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join(APP_DIR);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config").join(APP_DIR)
}

pub fn config_path() -> PathBuf {
    config_dir().join(FILE)
}

/// 从文本装载:解析失败退默认 + 人话证据(load 与测试共用的纯函数)。
pub fn parse(text: &str) -> (Config, Option<String>) {
    match toml::from_str::<Config>(text) {
        Ok(c) => (c, None),
        Err(e) => (Config::default(), Some(format!("解析失败:{e}"))),
    }
}

/// 加载配置:不存在或解析失败都退回默认(解析失败时调用方应提示)。
pub fn load() -> (Config, Option<String>) {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let (c, err) = parse(&text);
            (c, err.map(|e| format!("{} {e}", path.display())))
        }
        Err(_) => (Config::default(), None),
    }
}

/// 最小写入:toml 序列化整文件。extra 保证陌生键不丢(注释会丢,V1 可接受)。
pub fn save(cfg: &Config) -> anyhow::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let text = toml::to_string_pretty(cfg)?;
    std::fs::write(config_path(), text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 空配置兜底() {
        let cfg = Config::default();
        assert!(cfg.nw_bin.is_none());
    }

    #[test]
    fn 坏_toml_第二返回值非空_好_toml_为_none() {
        let (_c, err) = parse("nw_home = ");
        assert!(err.is_some(), "坏 TOML 必须给证据");
        assert!(err.unwrap().contains("解析失败"));
        let (_c2, err2) = parse("nw_home = \"/data\"");
        assert!(err2.is_none(), "好 TOML 不制造噪音");
    }

    #[test]
    fn toml_往返保留陌生键() {
        let text = r#"
nw_bin = "/usr/local/bin/nw"
someUnknownKey = { a = 1 }
"#;
        let cfg: Config = toml::from_str(text).unwrap();
        assert_eq!(cfg.nw_bin.as_deref(), Some("/usr/local/bin/nw"));
        assert!(cfg.extra.contains_key("someUnknownKey"));
        let out = toml::to_string_pretty(&cfg).unwrap();
        assert!(out.contains("someUnknownKey"), "陌生键写回不丢");
    }
}
