//! 装配屏:选锁定集 → 发射仪式 → 七步步骤条 + stderr 实时日志。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListState, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::engine::{EngineOutput, ExitCode};
use crate::screens::body;

const STEPS: [&str; 7] = [
    "0 前置",
    "1 引导",
    "2 铺骨架",
    "3 落组件",
    "4 合配置",
    "5 写清单",
    "6 自检",
];

pub fn render_assemble(frame: &mut Frame, area: Rect, app: &App) {
    if app.as_confirm {
        render_launch_ritual(frame, area, app);
        return;
    }
    let block = body(app, "七步幂等装配");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.assembling {
        render_running(frame, inner, app);
    } else if let Some(out) = app.as_result.as_ref() {
        render_result(frame, inner, app, out);
    } else {
        render_picker(frame, inner, app);
    }
}

fn render_picker(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let cols =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area);
    let Some(ls) = app.ls.as_ref() else {
        frame.render_widget(
            Paragraph::new("锁定集清单来自 ls;没有数据先回概览按 r 刷新。").style(p.dim()),
            area,
        );
        return;
    };
    let items: Vec<Line> = if ls.locks.is_empty() {
        vec![Line::from(Span::styled(
            "没有锁定集。找作者 bake,或把手里的 .nwlock 放到锁定集目录。",
            p.dim(),
        ))]
    } else {
        ls.locks
            .iter()
            .map(|l| {
                Line::from(vec![
                    Span::styled(
                        format!("{:<10}", l.lock_id.clone().unwrap_or_else(|| "—".into())),
                        p.info(),
                    ),
                    Span::raw(" "),
                    Span::styled(l.recipe.clone().unwrap_or_else(|| "—".into()), p.dim()),
                    Span::raw(" "),
                    Span::styled(l.baked_at.clone().unwrap_or_default(), p.dim()),
                ])
            })
            .collect()
    };
    let mut st = ListState::default();
    if !ls.locks.is_empty() {
        st.select(Some(app.as_sel.min(ls.locks.len() - 1)));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::bordered()
                    .border_style(p.dim())
                    .title(Span::styled(" 选锁定集 ", p.accent())),
            )
            .highlight_style(p.sel()),
        cols[0],
        &mut st,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("发射流程", p.accent())),
            Line::from(""),
            Line::from("  Enter   发射仪式(先看清单再上车)"),
            Line::from("  ↑/↓     选锁定集"),
            Line::from(""),
            Line::from(Span::styled(
                "up 幂等可重入;取消安全,重跑同一命令继续。",
                p.dim(),
            )),
        ])
        .block(
            Block::bordered()
                .border_style(p.dim())
                .title(Span::styled(" 仪式说明 ", p.dim())),
        ),
        cols[1],
    );
}

/// 发射前确认:展示将要发生什么。Enter 发射,Esc 返航。
fn render_launch_ritual(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let lock = app.ls.as_ref().and_then(|l| l.locks.get(app.as_sel));
    let w = 60u16.min(area.width.saturating_sub(4));
    let h = 14u16.min(area.height.saturating_sub(2));
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    );
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  锁定集   "),
            Span::styled(
                lock.and_then(|l| l.lock_id.clone())
                    .unwrap_or_else(|| "—".into()),
                p.info(),
            ),
        ]),
        Line::from(vec![
            Span::raw("  配方     "),
            Span::raw(
                lock.and_then(|l| l.recipe.clone())
                    .unwrap_or_else(|| "—".into()),
            ),
        ]),
        Line::from(vec![
            Span::raw("  目标派   "),
            Span::raw(
                lock.and_then(|l| l.name.clone())
                    .unwrap_or_else(|| "—".into()),
            ),
        ]),
        Line::from(vec![
            Span::raw("  NW_HOME  "),
            Span::raw(app.nw_home.clone()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  将写入该派目录并原子提交 .nw-manifest;",
            p.dim(),
        )),
        Line::from(Span::styled(
            "  用户自加的键保留,永不动 auth.json。",
            p.dim(),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Enter 发射 ", p.accent().bold()),
            Span::raw("   "),
            Span::styled("Esc 返航 ", p.dim()),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 发射仪式 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

fn render_running(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let cols = Layout::horizontal([Constraint::Length(14), Constraint::Min(20)]).split(area);
    let step_lines: Vec<Line> = STEPS
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (mark, style) = match (i, app.as_log.len()) {
                _ if app.as_log.iter().any(|l| l.contains(s)) => ("✓", p.ok()),
                _ => (" ", p.dim()),
            };
            Line::from(vec![
                Span::raw(mark),
                Span::raw(" "),
                Span::styled(*s, style),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(step_lines).block(
            Block::bordered()
                .border_style(p.dim())
                .title(Span::styled(" 七步 ", p.accent())),
        ),
        cols[0],
    );
    let log: Vec<Line> = app
        .as_log
        .iter()
        .rev()
        .take(area.height as usize)
        .map(|l| Line::from(l.clone()))
        .collect();
    frame.render_widget(
        Paragraph::new(log).block(
            Block::bordered()
                .border_style(p.dim())
                .title(Span::styled(" stderr 实时日志 ", p.dim())),
        ),
        cols[1],
    );
}

fn render_result(frame: &mut Frame, area: Rect, app: &App, out: &EngineOutput) {
    let p = &app.palette;
    let mut lines: Vec<Line> = Vec::new();
    let ok_style = if out.ok { p.ok() } else { p.fail() };
    lines.push(Line::from(Span::styled(
        if out.ok {
            "  ✦ 派已出炉 ✦"
        } else {
            "  ✗ 装配未成"
        },
        ok_style.bold(),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(format!("  命令   {}", out.command_string())));
    lines.push(Line::from(format!(
        "  结局   {} (rc {})",
        out.exit_code.human(),
        out.exit_code.number()
    )));
    lines.push(Line::from(format!("  一句话 {}", out.headline())));
    if let Some(e) = out
        .envelope
        .as_ref()
        .and_then(|e| e.data_as::<crate::engine::UpData>())
    {
        if let Some(h) = e.converge_hash {
            lines.push(Line::from(format!("  收敛   {h}")));
        }
        if !e.env_missing.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  env 缺 {} —— 降级不算失败", e.env_missing.join("、")),
                p.warn(),
            )));
        }
    }
    for ev in out.evidence().iter().take(3) {
        lines.push(Line::from(Span::styled(
            format!(
                "  [{}] {}",
                ev.code.clone().unwrap_or_else(|| "证据".into()),
                ev.message
            ),
            p.dim(),
        )));
        if let Some(h) = &ev.hint {
            lines.push(Line::from(Span::styled(format!("    建议:{h}"), p.info())));
        }
    }
    if out.exit_code == ExitCode::LockConflict {
        lines.push(Line::from(Span::styled(
            "  同一派已有装配在跑(profile 级锁),稍后重试。",
            p.warn(),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  j/k 离开本屏;装配幂等,重来一次也没事。",
        p.dim(),
    )));
    frame.render_widget(Paragraph::new(lines), area);
}
