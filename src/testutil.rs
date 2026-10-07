//! 测试工具:把 config 落点隔离到临时目录。
//!
//! 生产代码的写配置路径不变;只有测试经 [`ConfigGuard`] 把
//! `XDG_CONFIG_HOME` 指到一次性目录,免得单测污染开发者真实配置、
//! 也免得并行测试互相踩。env 是进程级全局,使用时必须持有调用方的
//! 串行锁(见 app::tests 的 ENV_LOCK)。

use std::path::PathBuf;

pub struct ConfigGuard {
    prev: Option<String>,
    dir: PathBuf,
}

impl ConfigGuard {
    pub fn install() -> Self {
        let prev = std::env::var("XDG_CONFIG_HOME").ok();
        let dir = std::env::temp_dir().join(format!("nwt-test-cfg-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        Self { prev, dir }
    }

    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }
}

impl Drop for ConfigGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(p) => std::env::set_var("XDG_CONFIG_HOME", p),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 整帧的真显示内容(逐格拚行;字节级 golden 与列级命中断言共用)。
/// `TestBackend::to_string` 带引号与隐藏格注记,不能当裸行用;宽字符
/// 占两格,第二格是隐藏延续格,跳过——拼出的行即肉眼所见(ADR-0005
/// 「渲染与命中同源」的测量基准)。
pub fn frame_display(app: &crate::app::App, w: u16, h: u16) -> String {
    use crate::ui::render;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use unicode_width::UnicodeWidthStr;
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render(f, app)).unwrap();
    let buf = term.backend().buffer();
    (0..h)
        .map(|y| {
            let mut line = String::new();
            let mut x: u16 = 0;
            while x < w {
                let sym = buf
                    .cell((x, y))
                    .map(|c| c.symbol().to_string())
                    .unwrap_or_default();
                let wd = UnicodeWidthStr::width(sym.as_str());
                line.push_str(&sym);
                x = x.saturating_add(1);
                if wd >= 2 {
                    x = x.saturating_add(u16::try_from(wd - 1).unwrap_or(0));
                }
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::config;

    #[test]
    fn 守卫把落点改到临时目录且用完恢复() {
        let before = std::env::var("XDG_CONFIG_HOME").ok();
        let inside;
        {
            let g = ConfigGuard::install();
            inside = g.dir().clone();
            let path = config::config_path();
            assert!(path.starts_with(&inside), "配置路径应落进临时目录");
        }
        let after = std::env::var("XDG_CONFIG_HOME").ok();
        assert_eq!(before, after, "守卫销毁后环境复原");
        assert!(!inside.exists(), "临时目录已清理");
    }
}
