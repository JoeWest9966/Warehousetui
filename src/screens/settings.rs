//! 设置屏:引擎发现状态、NW_HOME、主题、配置入口、指路屏。

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::engine::config;
use crate::engine::{Capability, Probe, GATE_HINT};
use crate::screens::body;

pub fn render_settings(frame: &mut Frame, area: Rect, app: &App) {
    let block = body(app, "NW_HOME · 引擎发现");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if !app.engine.found {
        render_signpost(frame, inner, app);
    } else {
        render_panel(frame, inner, app);
    }
    // 浮层优先级最低的常屏之上:手填输入浮层盖在指路屏上也成立
    if app.home_input {
        render_home_input(frame, area, app);
    }
}

/// 引擎就绪时的设置面板。
fn render_panel(frame: &mut Frame, inner: Rect, app: &App) {
    let p = &app.palette;
    let mut lines = vec![
        Line::from(Span::styled("引擎", p.accent())),
        Line::from(format!(
            "  二进制   {}",
            app.engine.bin.clone().unwrap_or_else(|| "—".into())
        )),
        Line::from(format!("  来源     {}", app.engine.source)),
        Line::from(format!("  版本     nw {}", app.engine_version)),
    ];
    // 能力节(#29):竖向预算钉死——面板在 110×30 内层 24 行,既有 21 行
    // 之外只许 +3(config 告警必须留在屏内,既有测试钉着);横向按紧凑态
    // 62 列排:总行只放标记+名,状态归组行,指路单独一行。
    let verdicts: Vec<(Capability, Probe)> = Capability::ALL
        .into_iter()
        .map(|c| (c, app.capabilities.get(c)))
        .collect();
    let mut row1 = vec![Span::raw("  "), Span::styled("能力 ", p.accent())];
    for (cap, v) in &verdicts {
        let (mark, style) = match v {
            Probe::Available => ("✓", p.ok()),
            Probe::Unavailable => ("✗", p.fail()),
            Probe::Unknown => ("?", p.warn()),
        };
        row1.push(Span::styled(mark, style));
        row1.push(Span::raw(cap.label()));
        row1.push(Span::raw(" "));
    }
    lines.push(Line::from(row1));
    let disabled: Vec<&str> = verdicts
        .iter()
        .filter(|(_, v)| *v == Probe::Unavailable)
        .map(|(c, _)| c.label())
        .collect();
    let pending: Vec<&str> = verdicts
        .iter()
        .filter(|(_, v)| *v == Probe::Unknown)
        .map(|(c, _)| c.label())
        .collect();
    // 状态归组行:只在有非「可用」时出现(全部可用不占行);
    // 入口禁用与未决各成一组,互不冒充。
    if !disabled.is_empty() || !pending.is_empty() {
        let mut seg: Vec<String> = Vec::new();
        if !disabled.is_empty() {
            seg.push(format!("入口禁用:{}", disabled.join("、")));
        }
        if !pending.is_empty() {
            seg.push(format!("未决:{}", pending.join("、")));
        }
        lines.push(Line::from(Span::styled(
            format!("      {}", seg.join("; ")),
            p.warn(),
        )));
    }
    // 指路:只在真有「引擎没这能力」时给(未决不冒充禁用,指路屏另管)。
    if !disabled.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", GATE_HINT),
            p.dim(),
        )));
    }
    lines.extend(vec![
        Line::from(""),
        Line::from(Span::styled("环境", p.accent())),
        Line::from(format!("  NW_HOME  {}  (n 轮换 / i 手填)", app.nw_home)),
        Line::from(""),
        Line::from(Span::styled("外观", p.accent())),
        Line::from(format!("  主题     {} (t 切换)", app.theme.name())),
        Line::from(format!(
            "  鼠标     {} (m 切换,重启生效)",
            if app.mouse_enabled {
                "捕获开"
            } else {
                "捕获关"
            }
        )),
        Line::from(format!("  终端     {:?}", app.cap)),
        Line::from(""),
        Line::from(Span::styled("配置", p.accent())),
        Line::from(format!("  文件     {}", config::config_path().display())),
        Line::from(Span::styled(
            "  只读展示;按 e 用 $EDITOR 打开(终端会让出来)",
            p.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  铁律:配置永不存密钥;env 只显示 present 布尔",
            p.dim(),
        )),
    ]);
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  发现优先级:--nw-bin > 配置 > PATH > $NW_HOME/nw",
        p.dim(),
    )));
    // config 解析失败告警:人话一句 + 路径 + e 修复指引;健康时给一行安心
    match app.config_error.as_deref() {
        Some(err) => {
            lines.push(Line::from(Span::styled(format!("  ⚠ {err}"), p.warn())));
            lines.push(Line::from(Span::styled(
                "    已降级到默认值,其余功能照常;按 e 用 $EDITOR 打开修复",
                p.dim(),
            )));
        }
        None => lines.push(Line::from(Span::styled("  config 状态  解析正常", p.dim()))),
    }
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().border_style(p.dim())),
        inner,
    );
}

/// 手填输入浮层:缓冲区 + 块光标 + 键位提示。
fn render_home_input(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let w = 64u16.min(area.width.saturating_sub(4));
    let h = 7u16.min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    );
    frame.render_widget(Clear, popup);
    let cursor = format!("{}█", app.home_buf);
    let lines = vec![
        Line::from(Span::styled(cursor, p.text())),
        Line::from(Span::styled(
            "  Enter 确认 · Esc/q 放弃 · 退格删字符 · ~ 会展开",
            p.dim(),
        )),
        Line::from(Span::styled(
            "  生效后与轮换同链:写环境、持久化、刷新 ls",
            p.dim(),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 手填 NW_HOME ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// 指路屏:说清楚要什么、去哪拿、怎么指。
fn render_signpost(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let mut lines = vec![
        Line::from(Span::styled("找不到 nw,驾驶舱无法点火。", p.fail().bold())),
        Line::from(""),
        Line::from("找过这些地方:"),
    ];
    for a in &app.engine.attempted {
        lines.push(Line::from(Span::styled(format!("  · {a}"), p.dim())));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("三条指路,任选其一:", p.accent())));
    lines.push(Line::from("  1. 装好 Nous Warehouse,把 nw 放进 PATH"));
    lines.push(Line::from("  2. 启动时指路:nwt --nw-bin /路径/nw"));
    lines.push(Line::from(format!(
        "  3. 或把二进制放到 {}/nw",
        app.nw_home
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  nw 是唯一的引擎与事实来源;本面板不做装配决策。",
        p.dim(),
    )));
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().border_style(p.fail())),
        area,
    );
}
