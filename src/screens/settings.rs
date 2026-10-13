//! 设置屏:引擎发现状态、NW_HOME、主题、配置入口、指路屏。

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::engine::config;
use crate::engine::{Capability, Probe, GATE_HINT};
use crate::screens::body;

pub fn render_settings(frame: &mut Frame, area: Rect, app: &App) {
    let block = body(app, "NW_HOME · 数据仓根 · 引擎发现");
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
    if app.data_input {
        render_data_input(frame, area, app);
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
    // #63:入口清单八项(去掉指向上游不存在子命令的「类别」,补 key/订阅/标签
    // 三条组级探针),常畅态 96 列仍是一行;紧凑态内层 64 列放不下时按显示
    // 宽度折行——竖向只多一行(紧凑态 66×50 内层 40+ 行,放得下),**不许
    // 截掉入口**:截掉等于面板对操作者隐瞒引擎有或没有的能力。
    let verdicts: Vec<(Capability, Probe)> = Capability::ALL
        .into_iter()
        .map(|c| (c, app.capabilities.get(c)))
        .collect();
    let mut rows: Vec<Line> = Vec::new();
    let mut row = vec![Span::raw("  "), Span::styled("能力 ", p.accent())];
    let mut used = UnicodeWidthStr::width("  能力 ");
    for (cap, v) in &verdicts {
        let (mark, style) = match v {
            Probe::Available => ("✓", p.ok()),
            Probe::Unavailable => ("✗", p.fail()),
            Probe::Unknown => ("?", p.warn()),
        };
        // 一格宽标记 + 入口名 + 一格间隔
        let w = 1 + UnicodeWidthStr::width(cap.label()) + 1;
        if used + w > inner.width as usize && row.len() > 2 {
            rows.push(Line::from(std::mem::take(&mut row)));
            row.push(Span::raw("  "));
            used = 2;
        }
        row.push(Span::styled(mark, style));
        row.push(Span::raw(cap.label()));
        row.push(Span::raw(" "));
        used += w;
    }
    rows.push(Line::from(row));
    lines.extend(rows);
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
        lines.push(Line::from(Span::styled(format!("  {GATE_HINT}"), p.dim())));
    }
    lines.extend(vec![
        Line::from(""),
        Line::from(Span::styled("环境", p.accent())),
        Line::from(format!("  NW_HOME  {}  (n 轮换 / i 手填)", app.nw_home)),
        Line::from(format!(
            "  数据仓根  {}  (d 手填)",
            app.data_root.as_deref().unwrap_or("未设置")
        )),
        // 三态指路与五域入口同源(标题+说法,互不冒充);健康时给一行安心
        match app.data_root_cause() {
            Some(cause) => {
                let style = if cause == crate::app::DataRootCause::Unset {
                    p.dim()
                } else {
                    p.warn()
                };
                Line::from(Span::styled(
                    format!(
                        "  {}: {}",
                        cause.title(),
                        cause.advice(app.data_root.as_deref())
                    ),
                    style,
                ))
            }
            None => Line::from(Span::styled("  数据仓根就绪:五域读的就是这个目录", p.dim())),
        },
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
        // 自升级入口(#121):`U` 复用 install.sh 同一道 sha256 闸,双出口
        // 共享同一套校验纪律(见 docs/release.md「双出口」)。**并进上一行**
        // 而不是新起一行:设置屏竖向预算钉死(见能力节头注「只许 +3」),
        // config 解析失败告警必须留在屏内,多起一行就把它挤出去。
        Line::from(Span::styled(
            "  只读展示;按 e 用 $EDITOR 打开 · 按 U 自升级 nwt(sha256 同款闸)",
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

/// 手填输入浮层(NW_HOME / 数据仓根共用):缓冲区 + 块光标 + 键位提示。
/// 居中矩形走 [`crate::layout::popup_rect`](与登记浮层同源),两个入口只差
/// 缓冲区、标题与末行提示——形状零分歧(Duplicated Code 收口)。
fn render_text_input(frame: &mut Frame, area: Rect, app: &App, buf: &str, title: &str, hint: &str) {
    let p = &app.palette;
    let popup = crate::layout::popup_rect(area, 64, 7);
    frame.render_widget(Clear, popup);
    let cursor = format!("{buf}█");
    let lines = vec![
        Line::from(Span::styled(cursor, p.text())),
        Line::from(Span::styled(
            "  Enter 确认 · Esc/q 放弃 · 退格删字符 · ~ 会展开",
            p.dim(),
        )),
        Line::from(Span::styled(format!("  {hint}"), p.dim())),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(title, p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// NW_HOME 手填浮层。
fn render_home_input(frame: &mut Frame, area: Rect, app: &App) {
    render_text_input(
        frame,
        area,
        app,
        &app.home_buf,
        " 手填 NW_HOME ",
        "  生效后与轮换同链:写环境、持久化、刷新 ls",
    );
}

/// 数据仓根手填浮层:与 NW_HOME 手填同构(同一渲染件)。
fn render_data_input(frame: &mut Frame, area: Rect, app: &App) {
    render_text_input(
        frame,
        area,
        app,
        &app.data_buf,
        " 手填数据仓根 ",
        "  生效后写配置并立即刷新五域;坏路径会得到指路",
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
