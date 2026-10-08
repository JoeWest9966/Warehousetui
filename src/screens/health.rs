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
    let block = body(app, "doctor · validate · 外部源漂移");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cols =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(inner);

    // 外部源漂移复核(#40):与 doctor / validate 并列的第三种模式,只读。
    // 结论按结果四态呈现,与 doctor 的降级/漂移/破损三档并列但不合并。
    if app.hl_mode == crate::app::HealthMode::Recheck {
        render_recheck(frame, inner, app);
        return;
    }

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
        // 漂移复核走 render_recheck(左栏清单 + 右栏证据),不在这条 match 里。
        crate::app::HealthMode::Recheck => {}
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

// ---- 外部源漂移复核(spec #34 / 工单 #40):只读,四态,口径以引擎为唯一真相 ----

/// 左栏:结论横幅 + 已登记外部源清单(四态);右栏:漂移条目 + 证据五字段。
/// 三道门(未设数据仓根 / 引擎无 source 能力 / 清单为空)各有可区分指路。
///
/// 四块几何与命中区共用 [`crate::layout::recheck_panels`],几何零分歧。
fn render_recheck(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    // 三道门铺满整栏(与命中侧同一组分栏常量)
    let (left, right) = crate::layout::recheck_cols(area);
    // 门一:数据仓根。未设/不可用与「引擎没能力」说法不同,互不冒充。
    if app.data_root_cause().is_some() {
        frame.render_widget(
            Paragraph::new(crate::screens::data_root_signpost(app))
                .wrap(ratatui::widgets::Wrap { trim: true })
                .block(
                    Block::bordered()
                        .border_style(p.dim())
                        .title(Span::styled(" 外部源漂移 ", p.accent())),
                ),
            left,
        );
        frame.render_widget(
            Paragraph::new("指路说清了缺的是什么;补上前提再回来查。").style(p.dim()),
            right,
        );
        return;
    }
    // 门二:引擎没有 source 能力(缺的是引擎能力,不是面板坏了)。
    if !app
        .capabilities
        .enabled(crate::engine::Capability::SourceRecheck)
    {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("  ⚠ 引擎缺 source 能力 ", p.warn().bold())),
                Line::from(Span::styled(
                    format!("  {}", crate::engine::capability::GATE_HINT),
                    p.dim(),
                )),
            ])
            .block(
                Block::bordered()
                    .border_style(p.dim())
                    .title(Span::styled(" 外部源漂移 ", p.accent())),
            ),
            left,
        );
        frame.render_widget(Paragraph::new("").style(p.dim()), right);
        return;
    }
    // 门三:清单为空——「还没读到五域」与「编目真为空」分开说,互不冒充。
    if let Some(cause) = app.recheck_empty_cause() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(format!("  {}", cause.title()), p.dim())),
                Line::from(""),
                Line::from(Span::styled(format!("  {}", cause.advice()), p.info())),
            ])
            .block(
                Block::bordered()
                    .border_style(p.dim())
                    .title(Span::styled(" 外部源漂移 ", p.accent())),
            ),
            left,
        );
        frame.render_widget(Paragraph::new("").style(p.dim()), right);
        return;
    }

    // 左栏:结论横幅(3 行文案 + 2 行块边框)+ 已登记外部源清单(四态灯,选中行可按名限定);
    // 右栏:漂移条目(引擎原文四种 kind)+ 证据五字段。四块几何共用同一处。
    let [banner_rect, list_rect, drift_rect, evidence_rect] = crate::layout::recheck_panels(area);
    let (banner, style) = recheck_banner(app);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(banner, style)),
            Line::from(Span::styled(
                "  ↑/↓ 选记录 · Enter 只查这条 · r 全查 · m 切回 doctor",
                p.dim(),
            )),
            Line::from(Span::styled(
                "  只读:本段不写任何文件;上游 commit 变化不算漂移(口径以引擎为准)。",
                p.dim(),
            )),
        ])
        .block(
            Block::bordered()
                .border_style(p.dim())
                .title(Span::styled(" 外部源漂移 ", p.accent())),
        ),
        banner_rect,
    );
    let mut st = ListState::default();
    st.select(Some(app.hl_sel.min(app.recheck_source_len() - 1)));
    let items: Vec<Line> = app
        .ls
        .as_ref()
        .map(|l| {
            l.sources
                .iter()
                .map(|s| {
                    let state = recheck_state_of(app, &s.name);
                    Line::from(vec![
                        Span::styled(format!("{} ", p.lamp(state)), p.state(state)),
                        Span::raw(s.name.clone()),
                        Span::raw(" "),
                        Span::styled(s.kind.clone().unwrap_or_default(), p.dim()),
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::bordered()
                    .border_style(p.dim())
                    .title(Span::styled(" 已登记外部源 ", p.accent())),
            )
            .highlight_style(p.sel()),
        list_rect,
        &mut st,
    );

    frame.render_widget(
        Paragraph::new(drift_lines(app)).block(
            Block::bordered()
                .border_style(p.dim())
                .title(Span::styled(" 漂移条目 ", p.accent())),
        ),
        drift_rect,
    );
    match app.hl_recheck.as_ref() {
        Some(out) => render_evidence(frame, evidence_rect, app, out),
        None => frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled("证据(五字段,原样呈现)", p.accent())),
                Line::from(Span::styled(
                    "本次还没查:按 r 全查,或 ↑/↓ + Enter 只查一条。",
                    p.dim(),
                )),
                Line::from(Span::styled(
                    "面板不缓存漂移结论冒充「上次结果」。",
                    p.dim(),
                )),
            ])
            .block(
                Block::bordered()
                    .border_style(p.dim())
                    .title(Span::styled(" 证据 ", p.dim())),
            ),
            evidence_rect,
        ),
    }
}

/// 某条记录在**本次**结论里的四态:有漂移 = 败,查了没漂移 = 过,没查 = 略。
/// 拿不到可定型的回音(含契约守卫「需升级 nw」)时一律「略」——不猜。
fn recheck_state_of(app: &App, source: &str) -> crate::engine::State {
    app.recheck_data()
        .map(|d| d.state_of(source))
        .unwrap_or(crate::engine::State::Skip)
}

/// 本次结论的一句话:无漂移 / 有漂移 / 还没查 —— 三者可区分,
/// 「还没查」绝不冒充「没问题」(无缓存语义)。
fn recheck_banner(app: &App) -> (String, ratatui::style::Style) {
    let p = &app.palette;
    let Some(out) = app.hl_recheck.as_ref() else {
        return (
            "  还没查:按 r 全查,或 ↑/↓ + Enter 只查一条。".to_string(),
            p.dim(),
        );
    };
    let Some(d) = app.recheck_data() else {
        // 不可定型(含契约守卫的「需升级 nw」):走证据,不按旧形状猜
        return (format!("  {}", out.headline()), p.warn());
    };
    if d.drift.is_empty() {
        (
            format!("  ✓ 无漂移:{} 条记录,口径以引擎为准", d.checked.len()),
            p.ok(),
        )
    } else {
        (
            format!(
                "  ✗ {} 处漂移(rc {}):批准的更新由你重跑 register 覆盖写",
                d.drift.len(),
                out.exit_code.number()
            ),
            p.fail(),
        )
    }
}

/// 漂移条目按引擎原文呈现:四种 kind 一个都不改写,条目不猜、不补全。
fn drift_lines(app: &App) -> Vec<Line<'static>> {
    let p = &app.palette;
    let Some(d) = app.recheck_data() else {
        return vec![Line::from(Span::styled(
            "  本次查询后呈现(上游 commit 变化不算漂移,口径以引擎为准)。",
            p.dim(),
        ))];
    };
    if d.drift.is_empty() {
        return vec![
            Line::from(Span::styled("  漂移条目:无。", p.dim())),
            Line::from(Span::styled(
                "  上游 commit 变化不算漂移——引擎口径,面板不加码。",
                p.dim(),
            )),
        ];
    }
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        format!("  {} 处(engine rc 3 = 结论是红的):", d.drift.len()),
        p.accent(),
    ))];
    for it in d.drift.iter().take(5) {
        // kind 与 source/entry 一行、路径一行:右栏内层只有 ~43 列,
        // 单行排下就会被裁掉半边——路径是「该动哪条」的关键,不能看不见。
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} ", p.lamp(crate::engine::State::Fail)),
                p.fail(),
            ),
            Span::styled(it.kind.clone(), p.fail()),
            Span::raw(" "),
            Span::raw(format!("{}/{}", it.source, it.entry)),
        ]));
        lines.push(Line::from(Span::styled(
            format!("      {}", it.path),
            p.dim(),
        )));
    }
    if d.drift.len() > 5 {
        lines.push(Line::from(Span::styled(
            format!("  … 共 {} 处,屏内未全显", d.drift.len()),
            p.dim(),
        )));
    }
    lines
}
