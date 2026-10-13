//! 终端能力探测、初始化与恢复。
//!
//! 能力探测喂给主题做颜色降级;初始化/恢复保证退出与 panic 时
//! 终端不留残局(raw mode、备用屏幕)。

use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::style::Color;
use ratatui::Terminal;
use std::io::{stdout, Stdout};

pub type Term = Terminal<CrosstermBackend<Stdout>>;

/// 面板能用的最小终端尺寸(保证下限,支持到 66×50;spec #15)。
/// 宽度断点另有单一常量 [`crate::layout::BREAKPOINT`],别混:
/// 这里是「还能不能开」,那里是「换不换装」。
pub const MIN_W: u16 = 66;
pub const MIN_H: u16 = 24;

/// 终端颜色能力三档。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorCap {
    Truecolor,
    C256,
    C16,
}

/// 探测当前终端颜色能力:truecolor > 256 > 16。
pub fn detect_color_cap() -> ColorCap {
    if let Ok(ct) = std::env::var("COLORTERM") {
        let ct = ct.to_ascii_lowercase();
        if ct == "truecolor" || ct == "24bit" {
            return ColorCap::Truecolor;
        }
    }
    match std::env::var("TERM") {
        Ok(term) if term.contains("256color") => ColorCap::C256,
        _ => ColorCap::C16,
    }
}

/// RGB → xterm-256 索引(标准量化)。
pub fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    fn cube(v: u8) -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            (v - 35) / 40
        }
    }
    if r == g && g == b {
        if r < 8 {
            16
        } else if r > 248 {
            231
        } else {
            232 + ((r as u16 - 8) / 10) as u8
        }
    } else {
        16 + 36 * cube(r) + 6 * cube(g) + cube(b)
    }
}

/// 16 色 ANSI 标准 RGB 值,用于 256→16 的最近邻映射。
const ANSI16: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (128, 0, 0),
    (0, 128, 0),
    (128, 128, 0),
    (0, 0, 128),
    (128, 0, 128),
    (0, 128, 128),
    (192, 192, 192),
    (128, 128, 128),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (0, 0, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

fn ansi16_from_rgb(r: u8, g: u8, b: u8) -> Color {
    let mut best = (0usize, u32::MAX);
    for (i, (ar, ag, ab)) in ANSI16.iter().enumerate() {
        let dr = r as i32 - *ar as i32;
        let dg = g as i32 - *ag as i32;
        let db = b as i32 - *ab as i32;
        let d = (dr * dr + dg * dg + db * db) as u32;
        if d < best.1 {
            best = (i, d);
        }
    }
    match best.0 {
        0..=7 => Color::Indexed(best.0 as u8),
        _ => Color::Indexed(best.0 as u8),
    }
}

/// 把一个颜色按终端能力降到该终端能显示的最高保真档。
pub fn degrade(color: Color, cap: ColorCap) -> Color {
    match (color, cap) {
        (Color::Rgb(r, g, b), ColorCap::Truecolor) => Color::Rgb(r, g, b),
        (Color::Rgb(r, g, b), _) => {
            let idx = rgb_to_256(r, g, b);
            match cap {
                ColorCap::C256 => Color::Indexed(idx),
                ColorCap::C16 => ansi16_from_rgb(r, g, b),
                ColorCap::Truecolor => unreachable!(),
            }
        }
        (other, _) => other,
    }
}

/// 鼠标捕获开启序列(纯函数:`enabled=false` → 空串)。
///
/// init 与单测共用同一来源——关闭即一个字节都不发,终端保留原生
/// 「拖拽选中/右键复制」;这是逃逸两联(另一联 --no-mouse)的落点(ADR-0005)。
pub fn mouse_capture_ansi(enabled: bool) -> String {
    if !enabled {
        return String::new();
    }
    use crossterm::Command;
    let mut buf = String::new();
    let _ = crossterm::event::EnableMouseCapture.write_ansi(&mut buf);
    buf
}

// ---- 装配完成通知(#117):bell 与 OSC 9 桌面通知 ----

/// 终端 bell:一个 `\x07`,最低保底——任何终端都认,不支持桌面通知的
/// 环境也至少响一声(规格 Further Notes:bell 是最低保底)。
pub const BELL: &str = "\x07";

/// bell 序列(纯函数,与 [`mouse_capture_ansi`] 同族的「term 层输出捕获」
/// 先例:init 与单测共用同一来源)。
pub fn bell() -> &'static str {
    BELL
}

/// OSC 9 桌面通知序列:`ESC ] 9 ; <文本> BEL`。
///
/// **不探测、不报错**:OSC 9 是「支持就弹、不支持就静默丢弃」的序列
/// (规格 Further Notes:OSC 9 在不支持的环境是静默 no-op),所以这里既
/// 不做能力探测也不做降级——照发即是覆盖面最大化(bell 兜底 + OSC 9 加成)。
pub fn osc9(text: &str) -> String {
    format!("\x1b]9;{text}\x07")
}

/// 完成通知出口:两个**独立**开关各自门控(bell / desktop),都关则返回
/// 空串——调用方因此不必自己判空,「共享终端 / 勿扰场景不扰人」在这一处
/// 收口。文本由调用方给(装配收尾话术,不另造)。
pub fn notify_ansi(bell_on: bool, desktop_on: bool, text: &str) -> String {
    let mut s = String::new();
    if bell_on {
        s.push_str(BELL);
    }
    if desktop_on {
        s.push_str(&osc9(text));
    }
    s
}

/// 初始化终端:raw mode + 备用屏幕 + 鼠标捕获(由 mouse_enabled 门控)。
/// 失败时调用方负责 restore。
pub fn init(mouse_enabled: bool) -> anyhow::Result<Term> {
    enable_raw_mode()?;
    let mut out = stdout();
    crossterm::execute!(out, EnterAlternateScreen)?;
    let capture = mouse_capture_ansi(mouse_enabled);
    if !capture.is_empty() {
        use std::io::Write;
        out.write_all(capture.as_bytes())?;
    }
    Ok(Terminal::new(CrosstermBackend::new(out))?)
}

/// 恢复终端到常态。幂等,可重复调用;退出路径和 panic hook 共用。
/// 释放鼠标捕获无条件发(未开启时该序列是无害的模式复位),保证
/// 任何退出路径都不把捕获状态遗留给用户的 shell。
pub fn restore() {
    let mut out = stdout();
    let _ = crossterm::execute!(out, crossterm::event::DisableMouseCapture);
    let _ = crossterm::execute!(out, LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

/// panic 时先恢复终端再打印,避免用户面对一个坏掉的 shell。
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        default(info);
    }));
}

/// 当前终端尺寸,探测失败按最小尺寸兜底。
pub fn terminal_size() -> (u16, u16) {
    crossterm::terminal::size().unwrap_or((MIN_W, MIN_H))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 灰度上rgb_256往返合理() {
        assert_eq!(rgb_to_256(0, 0, 0), 16);
        assert_eq!(rgb_to_256(255, 255, 255), 231);
        assert_eq!(rgb_to_256(20, 142, 118), 30);
    }

    #[test]
    fn 降级链保留可显示性() {
        let c = Color::Rgb(0xd9, 0x9a, 0x2b);
        assert!(matches!(
            degrade(c, ColorCap::Truecolor),
            Color::Rgb(_, _, _)
        ));
        assert!(matches!(degrade(c, ColorCap::C256), Color::Indexed(_)));
        assert!(matches!(degrade(c, ColorCap::C16), Color::Indexed(_)));
    }

    #[test]
    fn 索引色原样透传() {
        assert_eq!(degrade(Color::Indexed(7), ColorCap::C16), Color::Indexed(7));
    }

    #[test]
    fn 鼠标捕获序列_由开关门控() {
        let on = mouse_capture_ansi(true);
        assert!(!on.is_empty(), "开必须发捕获序列");
        assert!(on.contains("?1000h"), "点击追踪到位");
        assert!(on.contains("?1006h"), "SGR 坐标到位");
        let off = mouse_capture_ansi(false);
        assert!(off.is_empty(), "关 = 一个字节都不发,原生选中/复制不被接管");
    }

    #[test]
    fn 通知bell序列_就是一声x07() {
        assert_eq!(BELL, "\x07", "bell 是一个字节,不带任何包装");
        assert_eq!(bell(), "\x07");
    }

    #[test]
    fn osc9序列_桌面通知形状正确() {
        assert_eq!(
            osc9("装配结束:收敛 (rc 0)"),
            "\x1b]9;装配结束:收敛 (rc 0)\x07"
        );
    }

    #[test]
    fn 通知出口_两个开关各自门控_都关则一个字节都不发() {
        let both = notify_ansi(true, true, "x");
        assert_eq!(
            both,
            format!("{BELL}\u{1b}]9;x\u{7}"),
            "都开 = bell 在前,OSC 9 在后"
        );
        assert_eq!(
            notify_ansi(false, true, "x"),
            "\u{1b}]9;x\u{7}",
            "bell 关 = 只剩桌面通知"
        );
        assert_eq!(notify_ansi(true, false, "x"), BELL, "桌面关 = 只剩一声响");
        assert_eq!(
            notify_ansi(false, false, "x"),
            "",
            "都关 = 一个字节都不发,共享终端不扰人"
        );
    }
}
