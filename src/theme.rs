//! 主题:调色板 + 按终端能力降级 + 主题切换。
//!
//! 颜色值来自第 3 轮决议(暗色仓库为默认,高对比备用)。

use ratatui::style::{Color, Style};
use std::sync::OnceLock;

use crate::term::{degrade, ColorCap};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeKind {
    /// 暗色「仓库」:钢灰底 + 琥珀强调。
    Warehouse,
    /// 高对比:纯黑底白字。
    HighContrast,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 2] = [ThemeKind::Warehouse, ThemeKind::HighContrast];

    pub fn name(self) -> &'static str {
        match self {
            ThemeKind::Warehouse => "暗色仓库",
            ThemeKind::HighContrast => "高对比",
        }
    }

    pub fn next(self) -> ThemeKind {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// 已按终端能力解析过的调色板。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Palette {
    pub bg: Color,
    pub surface: Color,
    pub border: Color,
    pub text: Color,
    pub dim: Color,
    pub accent: Color,
    pub info: Color,
    pub ok: Color,
    pub warn: Color,
    pub fail: Color,
    pub sel_bg: Color,
}

impl Palette {
    /// 暗色仓库主题的 RGB 原值。
    fn warehouse_rgb() -> Self {
        Palette {
            bg: Color::Rgb(0x14, 0x17, 0x1c),
            surface: Color::Rgb(0x1d, 0x21, 0x26),
            border: Color::Rgb(0x33, 0x3a, 0x42),
            text: Color::Rgb(0xc9, 0xd1, 0xd9),
            dim: Color::Rgb(0x6e, 0x76, 0x81),
            accent: Color::Rgb(0xd9, 0x9a, 0x2b),
            info: Color::Rgb(0x56, 0xb6, 0xc2),
            ok: Color::Rgb(0x3f, 0xb9, 0x50),
            warn: Color::Rgb(0xd2, 0x99, 0x22),
            fail: Color::Rgb(0xf8, 0x51, 0x49),
            sel_bg: Color::Rgb(0x2b, 0x31, 0x38),
        }
    }

    fn high_contrast_rgb() -> Self {
        Palette {
            bg: Color::Rgb(0x00, 0x00, 0x00),
            surface: Color::Rgb(0x0a, 0x0a, 0x0a),
            border: Color::Rgb(0x76, 0x76, 0x76),
            text: Color::Rgb(0xff, 0xff, 0xff),
            dim: Color::Rgb(0xb0, 0xb0, 0xb0),
            accent: Color::Rgb(0xff, 0xd1, 0x66),
            info: Color::Rgb(0x4d, 0xd0, 0xe1),
            ok: Color::Rgb(0x00, 0xe6, 0x76),
            warn: Color::Rgb(0xff, 0xea, 0x00),
            fail: Color::Rgb(0xff, 0x52, 0x52),
            sel_bg: Color::Rgb(0x2a, 0x2a, 0x2a),
        }
    }

    pub fn resolve(kind: ThemeKind, cap: ColorCap) -> Self {
        let base = match kind {
            ThemeKind::Warehouse => Self::warehouse_rgb(),
            ThemeKind::HighContrast => Self::high_contrast_rgb(),
        };
        base.degrade(cap)
    }

    fn degrade(self, cap: ColorCap) -> Self {
        Palette {
            bg: degrade(self.bg, cap),
            surface: degrade(self.surface, cap),
            border: degrade(self.border, cap),
            text: degrade(self.text, cap),
            dim: degrade(self.dim, cap),
            accent: degrade(self.accent, cap),
            info: degrade(self.info, cap),
            ok: degrade(self.ok, cap),
            warn: degrade(self.warn, cap),
            fail: degrade(self.fail, cap),
            sel_bg: degrade(self.sel_bg, cap),
        }
    }

    // ---- 常用样式快捷方式 ----

    pub fn base(&self) -> Style {
        Style::new().fg(self.text).bg(self.bg)
    }

    pub fn surface(&self) -> Style {
        Style::new().fg(self.text).bg(self.surface)
    }

    pub fn dim(&self) -> Style {
        Style::new().fg(self.dim)
    }

    pub fn info(&self) -> Style {
        Style::new().fg(self.info)
    }

    pub fn warn(&self) -> Style {
        Style::new().fg(self.warn)
    }

    pub fn fail(&self) -> Style {
        Style::new().fg(self.fail)
    }

    pub fn ok(&self) -> Style {
        Style::new().fg(self.ok)
    }

    pub fn accent(&self) -> Style {
        Style::new().fg(self.accent)
    }

    pub fn sel(&self) -> Style {
        Style::new().fg(self.text).bg(self.sel_bg)
    }

    pub fn state(&self, s: crate::engine::State) -> Style {
        use crate::engine::State;
        let color = match s {
            State::Pass | State::Ok => self.ok,
            State::Warn | State::Degraded => self.warn,
            State::Fail | State::Drift | State::Broken => self.fail,
            State::Skip => self.dim,
        };
        Style::new().fg(color)
    }

    /// 状态灯字符:结果四态 + doctor 三档降级共用一套视觉。
    pub fn lamp(&self, s: crate::engine::State) -> &'static str {
        use crate::engine::State;
        match s {
            State::Pass | State::Ok => "●",
            State::Warn | State::Degraded => "◐",
            State::Fail | State::Drift | State::Broken => "✗",
            State::Skip => "○",
        }
    }
}

/// 全局降级缓存:同一能力下 rgb→降级色只算一次。
pub fn cached_degrade(kind: ThemeKind, cap: ColorCap) -> Palette {
    static CACHE: OnceLock<std::sync::Mutex<Option<(ColorCap, Palette)>>> = OnceLock::new();
    let mutex = CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = mutex.lock().unwrap();
    if let Some((c, p)) = *guard {
        if c == cap {
            let _ = kind;
            return p;
        }
    }
    let p = Palette::resolve(kind, cap);
    *guard = Some((cap, p));
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 仓库主题关键色正确() {
        let p = Palette::warehouse_rgb();
        assert_eq!(p.accent, Color::Rgb(0xd9, 0x9a, 0x2b));
        assert_eq!(p.bg, Color::Rgb(0x14, 0x17, 0x1c));
    }

    #[test]
    fn 降级后真彩不变256变索引() {
        let tc = Palette::resolve(ThemeKind::Warehouse, ColorCap::Truecolor);
        let c256 = Palette::resolve(ThemeKind::Warehouse, ColorCap::C256);
        assert!(matches!(tc.accent, Color::Rgb(_, _, _)));
        assert!(matches!(c256.accent, Color::Indexed(_)));
    }

    #[test]
    fn 主题轮转不越界() {
        assert_eq!(ThemeKind::Warehouse.next(), ThemeKind::HighContrast);
        assert_eq!(ThemeKind::HighContrast.next(), ThemeKind::Warehouse);
    }
}
