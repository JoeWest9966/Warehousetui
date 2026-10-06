//! 健康屏:doctor 七面卡片 / validate 27 项清单 + 证据五字段。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, HealthMode};
use crate::engine::State;
use crate::screens::body;

const SURFACES: [&str; 7] = [
    "manifest", "consist", "converge", "config", "env", "runtime", "provider",
];

pub fn render_health(frame: &mut Frame, area: Rect, app: &App) {
    let block = body(app, "doctor · validate");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cols =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(inner);

    let Some(out) = app.hl_result.as_ref() else {
        let hint = if app.hl_loading {
            "复核进行中…".to_string()
        } else {
            "按 r 对选中的派跑 doctor;m 切 validate。".to_string()
        };
        frame.render_widget(Paragraph::new(hint).style(app.palette.dim()), inner);
        return;
    };

    render_results(frame, cols[0], app, out);
    render_evidence(frame, cols[1], app, out);
}

fn render_results(frame: &mut Frame, area: Rect, app: &App, out: &crate::engine::EngineOutput) {
    let p = &app.palette;
    match app.hl_mode {
        HealthMode::Doctor => {
            let results = out
                .envelope
                .as_ref()
                .and_then(|e| e.data_as::<crate::engine::DoctorData>())
                .map(|d| d.results)
                .unwrap_or_default();
            let lines: Vec<Line> = if results.is_empty() {
                vec![Line::from(Span::styled(
                    "doctor 未返回 results(证据在右边)",
                    p.dim(),
                ))]
            } else {
                results
                    .iter()
                    .map(|r| {
                        let st = State::from(r.status.as_str());
                        Line::from(vec![
                            Span::styled(format!("{} ", p.lamp(st)), p.state(st)),
                            Span::styled(format!("{:<10}", r.surface), p.dim()),
                            Span::raw(" "),
                            Span::raw(r.message.clone()),
                        ])
                    })
                    .collect()
            };
            let banner = match out.exit_code {
                crate::engine::ExitCode::Ok => ("✓ 全绿", p.ok()),
                crate::engine::ExitCode::Degraded => ("降级:没完全好,但不算坏", p.warn()),
                crate::engine::ExitCode::Drift => ("漂移:声明物被改", p.fail()),
                crate::engine::ExitCode::ManifestBroken => ("破损:清单没了", p.fail()),
                _ => ("看证据", p.dim()),
            };
            let mut all = vec![
                Line::from(Span::styled(
                    format!("  {} (rc {})", banner.0, out.exit_code.number()),
                    banner.1,
                )),
                Line::from(""),
            ];
            all.extend(lines);
            frame.render_widget(
                Paragraph::new(all).block(
                    Block::bordered()
                        .border_style(p.dim())
                        .title(Span::styled(" 七面 ", p.accent())),
                ),
                area,
            );
        }
        HealthMode::Validate => {
            let results = out
                .envelope
                .as_ref()
                .and_then(|e| e.data_as::<crate::engine::ValidateData>())
                .map(|d| d.results)
                .unwrap_or_default();
            let mut st = ListState::default();
            if !results.is_empty() {
                st.select(Some(app.hl_sel.min(results.len() - 1)));
            }
            let items: Vec<Line> = if results.is_empty() {
                vec![Line::from(Span::styled(
                    "validate 未返回 results(证据在右边)",
                    p.dim(),
                ))]
            } else {
                results
                    .iter()
                    .map(|r| {
                        let s = State::from(r.status.as_str());
                        Line::from(vec![
                            Span::styled(format!("{} ", p.lamp(s)), p.state(s)),
                            Span::styled(format!("{:<4}", r.id), p.dim()),
                            Span::raw(r.message.clone()),
                        ])
                    })
                    .collect()
            };
            let summary = out
                .envelope
                .as_ref()
                .and_then(|e| e.data_as::<crate::engine::ValidateData>())
                .map(|d| {
                    format!(
                        " 共 {}:过 {} 警 {} 败 {} 略 {} ",
                        d.summary.total,
                        d.summary.pass,
                        d.summary.warn,
                        d.summary.fail,
                        d.summary.skip
                    )
                })
                .unwrap_or_default();
            frame.render_stateful_widget(
                List::new(items)
                    .block(
                        Block::bordered()
                            .border_style(p.dim())
                            .title(Span::styled(" 27 项 ", p.accent()))
                            .title_top(Line::from(Span::raw(summary)).right_aligned()),
                    )
                    .highlight_style(p.sel()),
                area,
                &mut st,
            );
        }
    }
    let _ = SURFACES;
}

fn render_evidence(frame: &mut Frame, area: Rect, app: &App, out: &crate::engine::EngineOutput) {
    let p = &app.palette;
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "证据(五字段,原样呈现)",
        p.accent(),
    ))];
    if let Some(pe) = &out.parse_error {
        lines.push(Line::from(Span::styled(
            format!("stdout 不可解析:{pe}"),
            p.fail(),
        )));
        let raw: Vec<&str> = out.raw_stdout.lines().take(10).collect();
        for l in raw {
            lines.push(Line::from(Span::styled(l.to_string(), p.dim())));
        }
    }
    let evs = out.evidence();
    if evs.is_empty() && out.parse_error.is_none() {
        lines.push(Line::from(Span::styled("没有 errors/warnings。", p.dim())));
    }
    for ev in evs.iter().take(20) {
        lines.push(Line::from(vec![
            Span::styled(
                format!(
                    "[{} rc{}] ",
                    ev.code.clone().unwrap_or_else(|| "warn".into()),
                    ev.exit_code.unwrap_or_default()
                ),
                p.warn(),
            ),
            Span::raw(ev.message.clone()),
        ]));
        if let Some(h) = &ev.hint {
            lines.push(Line::from(Span::styled(format!("    建议:{h}"), p.info())));
        }
        if let Some(d) = &ev.details {
            let s = serde_json::to_string_pretty(d).unwrap_or_default();
            for l in s.lines().take(8) {
                lines.push(Line::from(Span::styled(format!("    {l}"), p.dim())));
            }
        }
    }
    for l in out.stderr_tail.iter().rev().take(6) {
        lines.push(Line::from(Span::styled(format!("    {l}"), p.dim())));
    }
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_style(p.dim())
                .title(Span::styled(" 证据 ", p.dim())),
        ),
        area,
    );
}
