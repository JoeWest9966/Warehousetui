//! 概览屏:「家」。警示带 + 派表 + 四域计数卡 + 指令栏 + 最近日志。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use crate::app::App;
use crate::engine::State;
use crate::screens::body;

pub fn render_overview(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let block = body(app, "目标侧日常");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Length(1), // 警示带
        Constraint::Min(5),    // 主区
        Constraint::Length(4), // 计数卡
    ])
    .split(inner);

    render_alert(frame, rows[0], app);

    let cols =
        Layout::horizontal([Constraint::Percentage(68), Constraint::Percentage(32)]).split(rows[1]);
    render_profiles(frame, cols[0], app);
    render_commands(frame, cols[1], app);

    render_counts(frame, rows[2], app);
    let _ = p;
}

fn render_alert(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    if !app.engine.found {
        let line = Line::from(vec![
            Span::styled(" ⚠ 找不到 nw 二进制 ", p.fail()),
            Span::styled("——按 F1? 不,按 ? 看帮助;到「设置」屏看指路", p.dim()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let drift: Vec<String> = app
        .ls
        .as_ref()
        .map(|l| {
            l.profiles
                .iter()
                .filter(|pr| matches!(pr.state.as_deref(), Some("drift") | Some("broken")))
                .map(|pr| pr.name.clone())
                .collect()
        })
        .unwrap_or_default();
    if !drift.is_empty() {
        let line = Line::from(vec![
            Span::styled(" ⚠ 警示带 ", p.warn()),
            Span::styled(
                format!("派 {} 需要关注(漂移/破损)", drift.join("、")),
                p.dim(),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), area);
    } else if app.assembling {
        let lamp = if app.blink { "◉" } else { "○" };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {lamp} 装配进行中 "), p.accent()),
                Span::styled("其他引擎动作已锁定", p.dim()),
            ])),
            area,
        );
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(" ✓ 一切平静", p.dim()))),
            area,
        );
    }
}

fn render_profiles(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let block = ratatui::widgets::Block::bordered()
        .border_style(p.dim())
        .title(Span::styled(" 派 (profiles) ", p.accent()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(ls) = app.ls.as_ref() else {
        if app.ls_loading {
            frame.render_widget(Paragraph::new("正在 ls…").style(p.dim()), inner);
        } else {
            frame.render_widget(
                Paragraph::new("还没有 ls 数据。按 r 刷新;若引擎缺失,去设置屏看指路。")
                    .style(p.dim()),
                inner,
            );
        }
        return;
    };

    if ls.profiles.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("一个派都还没有。", p.dim())),
                Line::from(""),
                Line::from("入坑三步:"),
                Line::from("  1. 找作者要装配命令(bake 产出)"),
                Line::from("  2. 到「装配」屏选锁定集,发射"),
                Line::from("  3. 回来看它出现在这张表里"),
            ])
            .style(p.dim()),
            inner,
        );
        return;
    }

    let header = Row::new(vec!["", "派", "lockId", "装配于", "状态"])
        .style(p.dim())
        .bottom_margin(0);
    let rows = ls.profiles.iter().map(|pr| {
        let st = pr.state.as_deref().map(State::from).unwrap_or(State::Skip);
        Row::new(vec![
            Cell::from(p.lamp(st)).style(p.state(st)),
            Cell::from(pr.name.clone()),
            Cell::from(pr.lock_id.clone().unwrap_or_else(|| "—".into())),
            Cell::from(pr.installed_at.clone().unwrap_or_else(|| "—".into())),
            Cell::from(st.label()).style(p.state(st)),
        ])
    });
    let widths = [
        Constraint::Length(2),
        Constraint::Percentage(28),
        Constraint::Percentage(22),
        Constraint::Percentage(26),
        Constraint::Percentage(14),
    ];
    let mut state = TableState::default();
    state.select(Some(app.ov_sel.min(ls.profiles.len().saturating_sub(1))));
    frame.render_stateful_widget(
        Table::new(rows, widths)
            .header(header)
            .row_highlight_style(p.sel()),
        inner,
        &mut state,
    );
}

fn render_commands(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let items = vec![
        Line::from(vec![
            Span::styled(" ▸ 跑一次体检 ", p.accent()),
            Span::styled("  (h)", p.dim()),
        ]),
        Line::from(vec![
            Span::styled("   发起装配… ", p.accent()),
            Span::styled("  (a)", p.dim()),
        ]),
        Line::from(vec![
            Span::styled("   浏览配方   ", p.accent()),
            Span::styled("  (l)", p.dim()),
        ]),
        Line::from(vec![
            Span::styled("   刷新 ls    ", p.accent()),
            Span::styled("  (r)", p.dim()),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(items).block(
            ratatui::widgets::Block::bordered()
                .border_style(p.dim())
                .title(Span::styled(" 指令 ", p.dim())),
        ),
        area,
    );
}

fn render_counts(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let Some(ls) = app.ls.as_ref() else {
        frame.render_widget(Paragraph::new(""), area);
        return;
    };
    let cells = Layout::horizontal([
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
    ])
    .split(area);
    let card = |title: &str, n: usize| -> Paragraph<'_> {
        let style = if n == 0 { p.dim() } else { p.surface() };
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{title} "), p.dim()),
            Span::styled(format!("{n}"), p.accent()),
        ]))
        .block(ratatui::widgets::Block::bordered().border_style(p.dim()))
        .style(style)
    };
    let titles = ["recipes", "components", "sources", "profiles", "locks"];
    let counts = [
        ls.recipes.len(),
        ls.components.len(),
        ls.sources.len(),
        ls.profiles.len(),
        ls.locks.len(),
    ];
    for (i, (t, n)) in titles.iter().zip(counts).enumerate() {
        frame.render_widget(card(t, n), cells[i]);
    }
}
