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
    /// 数据仓根(数据仓目录,`nw ls --repo` 的显式指向;spec #24)。
    /// None = 未设——面板不猜、不拿当前目录兜底,五域走指路(#27)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_root: Option<String>,
    /// 主题 warehouse | high_contrast。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// 最近查看的派。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_profile: Option<String>,
    /// 鼠标捕获开关(默认 true;None = 缺省或坏配置降级)。
    /// 逃逸两联之另一联是 --no-mouse 旗标(见 ADR-0005)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mouse_capture: Option<bool>,
    /// 装配完成通知的 **bell 提示音**开关(#117;默认 true)。
    /// None = 缺省或坏配置降级,与 `mouse_capture` 同口径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bell: Option<bool>,
    /// 装配完成通知的 **OSC 9 桌面通知**开关(#117;默认 true)。
    /// 与 `bell` 是两个独立布尔——共享终端 / 勿扰场景可以只要一个。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_notify: Option<bool>,
    /// 用户自加的键:原样保留,写回时不丢。
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

impl Config {
    /// 鼠标捕获解析值:None(缺省)与坏配置降级后的默认皆为开。
    pub fn mouse_capture_enabled(&self) -> bool {
        self.mouse_capture.unwrap_or(true)
    }

    /// bell 提示音解析值(#117):None(缺省)与坏配置降级后的默认皆为开。
    pub fn bell_enabled(&self) -> bool {
        self.bell.unwrap_or(true)
    }

    /// OSC 9 桌面通知解析值(#117):同上,缺省即开。与 bell 各自独立。
    pub fn desktop_notify_enabled(&self) -> bool {
        self.desktop_notify.unwrap_or(true)
    }
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
    fn data_root_缺省未设_写回再读不丢() {
        let (cfg, err) = parse("");
        assert!(err.is_none());
        assert_eq!(cfg.data_root, None, "缺省即未设,不当当前目录兜底");
        let (mut off, _) = parse("");
        off.data_root = Some("/data/repo".into());
        let text = toml::to_string_pretty(&off).unwrap();
        let (back, err2) = parse(&text);
        assert!(err2.is_none());
        assert_eq!(
            back.data_root.as_deref(),
            Some("/data/repo"),
            "重启仍生效的落盘语义"
        );
    }

    #[test]
    fn mouse_capture_缺省为开_显式关保留() {
        let (cfg, err) = parse("");
        assert!(err.is_none());
        assert_eq!(cfg.mouse_capture, None);
        assert!(cfg.mouse_capture_enabled(), "缺省即开");
        let (off, _) = parse("mouse_capture = false");
        assert_eq!(off.mouse_capture, Some(false));
        assert!(!off.mouse_capture_enabled());
        // 显式值写回再读,偏好不失忆
        let text = toml::to_string_pretty(&off).unwrap();
        let (back, err2) = parse(&text);
        assert!(err2.is_none());
        assert_eq!(back.mouse_capture, Some(false), "重启仍生效的落盘语义");
    }

    #[test]
    fn mouse_capture_坏配置降级为开() {
        let (cfg, err) = parse("mouse_capture = ");
        assert!(err.is_some(), "坏 TOML 必须给证据");
        assert_eq!(cfg.mouse_capture, None, "降级到默认");
        assert!(cfg.mouse_capture_enabled(), "解析失败按默认 true 降级");
    }

    #[test]
    fn 完成通知两个开关_缺省都开_显式关保留() {
        let (cfg, err) = parse("");
        assert!(err.is_none());
        assert_eq!(cfg.bell, None);
        assert!(cfg.bell_enabled(), "bell 缺省即开");
        assert_eq!(cfg.desktop_notify, None);
        assert!(cfg.desktop_notify_enabled(), "桌面通知缺省即开");
        let (off, _) = parse("bell = false\ndesktop_notify = false");
        assert_eq!(off.bell, Some(false));
        assert_eq!(off.desktop_notify, Some(false));
        assert!(!off.bell_enabled());
        assert!(!off.desktop_notify_enabled());
        // 显式值写回再读,偏好不失忆(重启仍生效的落盘语义)
        let text = toml::to_string_pretty(&off).unwrap();
        let (back, err2) = parse(&text);
        assert!(err2.is_none());
        assert_eq!(back.bell, Some(false));
        assert_eq!(back.desktop_notify, Some(false));
    }

    #[test]
    fn 完成通知两个开关_各自独立_关一个不关另一个() {
        let (only_bell, _) = parse("desktop_notify = false");
        assert!(only_bell.bell_enabled(), "关桌面不动 bell");
        assert!(!only_bell.desktop_notify_enabled());
        let (only_desktop, _) = parse("bell = false");
        assert!(!only_desktop.bell_enabled(), "关 bell 不动桌面");
        assert!(only_desktop.desktop_notify_enabled());
    }

    #[test]
    fn 完成通知开关_坏配置降级为开() {
        let (cfg, err) = parse("bell = \ndesktop_notify = ");
        assert!(err.is_some(), "坏 TOML 必须给证据");
        assert_eq!(cfg.bell, None, "bell 降级到默认");
        assert_eq!(cfg.desktop_notify, None, "桌面降级到默认");
        assert!(cfg.bell_enabled(), "解析失败按默认 true 降级");
        assert!(cfg.desktop_notify_enabled());
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
