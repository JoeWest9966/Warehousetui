//! UI 外壳:顶栏 / 左竖导航 / HUD / 帮助浮层 / 过小提示。
//!
//! 壳负责「驾驶舱感」:常驻键位栏、全局装配态灯、NW_HOME 一目了然;
//! 屏幕内容由 [`crate::screens`] 各自渲染。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, Screen};
use crate::screens;
use crate::theme::Palette;

/// 壳三段:顶栏 1 行 / 主区 / HUD 1 行。
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let p = &app.palette;

    if app.splash {
        render_splash(frame, area, app);
        return;
    }
    if !app.fits(area.width, area.height) {
        render_too_small(frame, area, app);
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    render_top_bar(frame, rows[0], app);
    render_body(frame, rows[1], app);
    render_hud(frame, rows[2], app);

    if app.help {
        render_help(frame, area, app);
    }
    let _ = p;
}

fn render_top_bar(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let lamp = if app.assembling {
        "◉ 装配中"
    } else {
        "○ 空闲"
    };
    let lamp_style = if app.assembling { p.accent() } else { p.dim() };
    let line = Line::from(vec![
        Span::styled(" Warehousetui", p.accent().bold()),
        Span::styled(" · Nous Warehouse 驾驶舱 ", p.dim()),
        Span::raw("  nwt v0.1 · "),
        Span::styled(
            format!("nw {} · {}", app.engine_version, app.engine.source),
            p.info(),
        ),
        Span::raw("  │  NW_HOME: "),
        Span::styled(app.nw_home.clone(), p.dim()),
        Span::raw("  "),
        Span::styled(lamp, lamp_style),
    ]);
    frame.render_widget(Paragraph::new(line).style(p.surface()), area);
}

fn render_body(frame: &mut Frame, area: Rect, app: &App) {
    let cols = Layout::horizontal([Constraint::Length(12), Constraint::Min(10)]).split(area);
    render_nav(frame, cols[0], app);
    screens::render(frame, cols[1], app);
}

fn render_nav(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let block = Block::bordered()
        .border_style(p.dim())
        .title(Span::styled(" 导航 ", p.dim()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    for (i, screen) in Screen::ALL.iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }
        let row = Rect::new(inner.x, inner.y + i as u16, inner.width, 1);
        let selected = *screen == app.screen;
        let style = if selected { p.sel().bold() } else { p.dim() };
        let marker = if selected { "▸" } else { " " };
        let line = Line::from(vec![
            Span::raw(marker),
            Span::raw(" "),
            Span::styled(screen.icon(), style),
            Span::raw(" "),
            Span::styled(screen.title(), style),
        ]);
        frame.render_widget(Paragraph::new(line), row);
    }
}

fn render_hud(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let mut spans = vec![
        Span::styled(" j/k", p.accent()),
        Span::raw(" 切屏 "),
        Span::styled("Enter", p.accent()),
        Span::raw(" 确认 "),
        Span::styled("t", p.accent()),
        Span::raw(" 主题 "),
        Span::styled("?", p.accent()),
        Span::raw(" 帮助 "),
    ];
    if app.assembling {
        spans.push(Span::raw(" "));
        spans.push(Span::styled("装配中:动作已锁定", p.dim()));
    } else if app.home_input {
        spans.push(Span::raw(" "));
        spans.push(Span::styled("输入中:Enter 确认 · Esc/q 放弃", p.accent()));
    } else {
        spans.push(Span::styled("  q", p.accent()));
        spans.push(Span::raw(" 退出"));
    }
    let right = Span::styled(format!("主题:{} ", app.theme.name()), p.dim());
    let width = area.width as usize;
    let left_len: usize = spans.iter().map(|s| s.content.len()).sum();
    let right_len = right.content.len();
    let pad = width.saturating_sub(left_len + right_len);
    let mut line = spans;
    line.push(Span::raw(" ".repeat(pad)));
    line.push(right);
    frame.render_widget(Paragraph::new(Line::from(line)).style(p.surface()), area);
}

fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let block = Block::bordered()
        .border_style(p.accent())
        .title(Span::styled(" 帮助 ", p.accent().bold()));
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("  移动 ", p.accent()).to_string()),
        Line::from("    j / ↓      下一屏"),
        Line::from("    k / ↑      上一屏"),
        Line::from("    h / l      面板内左右(数据屏内)"),
        Line::from(""),
        Line::from(Span::styled("  动作 ", p.accent()).to_string()),
        Line::from("    Enter      确认 / 进入"),
        Line::from("    Esc        返回 / 关闭浮层"),
        Line::from("    t          切换主题"),
        Line::from("    i          手填 NW_HOME(设置屏)"),
        Line::from("    ?          本帮助"),
        Line::from("    q          退出(装配态 / 输入中禁用)"),
        Line::from(""),
        Line::from(Span::styled(
            "  装配幂等可重入,取消安全;失败永远带证据五字段。",
            p.dim(),
        )),
    ];
    let w = 44u16.min(area.width.saturating_sub(4));
    let h = 18u16.min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block).style(p.surface()), popup);
}

fn render_too_small(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let msg = format!(
        "终端太小:需要至少 {}×{},当前 {}×{}。调大窗口。",
        crate::term::MIN_W,
        crate::term::MIN_H,
        area.width,
        area.height
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(msg, p.fail())).centered())
            .block(Block::bordered().border_style(p.fail())),
        area,
    );
}

/// 启动 splash:复古货架 logo,任意键或 800ms 后交棒。
fn render_splash(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let art = [
        "   ┌───────────────────────────────┐   ",
        "   │  ▄▄   ▄▄   ▄▄   ▄▄   ▄▄   ▄▄  │   ",
        "   │  ██   ██   ██   ██   ██   ██  │   ",
        "   │  ▀▀   ▀▀   ▀▀   ▀▀   ▀▀   ▀▀  │   ",
        "   └───────────────────────────────┘   ",
        "         N O U S   W A R E H O U S E   ",
    ];
    let mut lines: Vec<Line> = vec![Line::from("")];
    for (i, row) in art.iter().enumerate() {
        let style = if i == art.len() - 1 {
            p.accent().bold()
        } else {
            p.dim()
        };
        lines.push(Line::from(Span::styled(*row, style)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "        Warehousetui · Nous Warehouse 驾驶舱",
        p.text(),
    )));
    lines.push(Line::from(Span::styled(
        "               nwt v0.1 · 按任意键进入",
        p.dim(),
    )));
    frame.render_widget(Paragraph::new(lines).centered(), area);
}

/// 数据屏共用的外框:标题 + 可选右上角注脚。
pub fn screen_block<'a>(p: &Palette, title: &'a str, note: Option<String>) -> Block<'a> {
    let mut block = Block::bordered()
        .border_style(p.dim())
        .title(Span::styled(format!(" {title} "), p.accent().bold()));
    if let Some(n) = note {
        block = block.title_top(Line::from(Span::raw(n)).right_aligned());
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render_to_string(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render(f, app)).unwrap();
        term.backend().to_string()
    }

    /// 测试默认已经越过 splash(与真实启动流一致:先 splash 再主界面)。
    fn ui_app() -> App {
        let mut app = App::new();
        app.splash = false;
        app
    }

    #[test]
    fn splash_先亮相且可跳过() {
        let mut app = App::new();
        assert!(app.splash, "开机先 splash");
        let s = render_to_string(&app, 100, 30);
        assert!(s.contains("W A R E H O U S E"));
        assert!(s.contains("驾驶舱"));
        app.splash = false; // 任意键/800ms 后交棒
        assert!(!render_to_string(&app, 100, 30).contains("W A R E H O U S E"));
    }

    #[test]
    fn 壳三件套渲染() {
        let app = ui_app();
        let s = render_to_string(&app, 100, 30);
        assert!(s.contains("Warehousetui"));
        assert!(s.contains("NW_HOME"));
        assert!(s.contains("概览"));
    }

    #[test]
    fn 装配态_hud_提示锁定() {
        let mut app = ui_app();
        app.assembling = true;
        let s = render_to_string(&app, 100, 30);
        assert!(s.contains("装配中"));
    }

    #[test]
    fn 终端过小有提示() {
        let app = ui_app();
        let s = render_to_string(&app, 40, 10);
        assert!(s.contains("终端太小"));
    }

    /// 新下限:66×50 仍能进驾驶舱;65 宽守卫文案只提「调大窗口」
    /// (不再建议「减小字体」,那是破坏 CJK 混排的错误自救,spec #15)。
    #[test]
    fn 新下限_66宽进入而65宽守卫() {
        use crate::term::{MIN_H, MIN_W};
        assert_eq!((MIN_W, MIN_H), (66, 24), "下限 66×24 不回退");
        let app = ui_app();
        let ok = render_to_string(&app, 66, 50);
        assert!(!ok.contains("终端太小"), "66×50 是保证最小,不得弹守卫屏");
        assert!(ok.contains("概览"), "66×50 下主界面照常渲染");
        let s = render_to_string(&app, 65, 50);
        assert!(s.contains("终端太小"), "65 宽仍弹守卫");
        assert!(s.contains("调大窗口"), "文案提示调大窗口");
        assert!(!s.contains("减小字体"), "铁律:不再建议减小字体");
    }

    #[test]
    fn 输入态_hud_给键位提示() {
        let mut app = ui_app();
        app.home_input = true;
        let s = render_to_string(&app, 100, 30);
        assert!(s.contains("输入中"));
        assert!(s.contains("Enter 确认"));
    }

    #[test]
    fn 帮助浮层可开关() {
        let mut app = ui_app();
        app.help = true;
        let s = render_to_string(&app, 100, 30);
        assert!(s.contains("帮助"));
    }
}
