//! 设置屏:引擎发现状态、NW_HOME、主题、配置入口、指路屏。

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::engine::config;
use crate::screens::body;

pub fn render_settings(frame: &mut Frame, area: Rect, app: &App) {
    let block = body(app, "NW_HOME · 引擎发现");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let p = &app.palette;
    if !app.engine.found {
        render_signpost(frame, inner, app);
        return;
    }

    let mut lines = vec![
        Line::from(Span::styled("引擎", p.accent())),
        Line::from(format!(
            "  二进制   {}",
            app.engine.bin.clone().unwrap_or_else(|| "—".into())
        )),
        Line::from(format!("  来源     {}", app.engine.source)),
        Line::from(format!("  版本     nw {}", app.engine_version)),
        Line::from(""),
        Line::from(Span::styled("环境", p.accent())),
        Line::from(format!("  NW_HOME  {}", app.nw_home)),
        Line::from(""),
        Line::from(Span::styled("外观", p.accent())),
        Line::from(format!("  主题     {} (t 切换)", app.theme.name())),
        Line::from(format!("  终端     {:?}", app.cap)),
        Line::from(""),
        Line::from(Span::styled("配置", p.accent())),
        Line::from(format!("  文件     {}", config::config_path().display())),
        Line::from(Span::styled(
            "  只读展示;V1 不自带编辑器,可用 $EDITOR 打开",
            p.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  铁律:配置永不存密钥;env 只显示 present 布尔",
            p.dim(),
        )),
    ];
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  发现优先级:--nw-bin > 配置 > PATH > $NW_HOME/nw",
        p.dim(),
    )));
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().border_style(p.dim())),
        inner,
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
