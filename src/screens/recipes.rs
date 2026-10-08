//! 配方屏:五域两栏只读浏览。h/l 切域,↑/↓ 选行。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, Domain};
use crate::layout::{size_class, SizeClass};
use crate::screens::{body, data_root_signpost};

pub fn render_recipes(frame: &mut Frame, area: Rect, app: &App) {
    let block = body(app, "五域只读浏览");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match size_class(frame.area()) {
        SizeClass::Comfy => render_recipes_comfy(frame, inner, app),
        SizeClass::Compact => render_recipes_compact(frame, inner, app),
    }
}

/// 常畅态:五域轨(定宽 14)/ 列表 / 详情三栏,既有外观逐像素不变。
fn render_recipes_comfy(frame: &mut Frame, inner: Rect, app: &App) {
    let cols = Layout::horizontal([
        Constraint::Length(14),
        Constraint::Percentage(45),
        Constraint::Percentage(41),
    ])
    .split(inner);
    render_domain_rail(frame, cols[0], app);
    render_rows(frame, cols[1], app);
    render_detail(frame, cols[2], app);
}

/// 紧凑态:五域轨收起为顶部一行同级 Tab,竖轨让出的 14 列归内容
/// (列表/详情按 52:48 分余宽),h/l 切域与选中态经 Tab 行可见(ADR-0004)。
fn render_recipes_compact(frame: &mut Frame, inner: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(4)]).split(inner);
    render_domain_tabs(frame, rows[0], app);
    let cols =
        Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)]).split(rows[1]);
    render_rows(frame, cols[0], app);
    render_detail(frame, cols[1], app);
}

/// 紧凑态的五域 Tab 行:五域同级一排,选中域 `▸` + 加粗高亮,计数随 tab。
/// 宽度:五域全名 + 计数 ≈ 48 列,66×50 下内宽 ≥ 48 不截断(窄则尾部截,Tab 不折行)。
fn render_domain_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let gated = app.data_root_cause().is_some();
    let mut spans: Vec<Span> = Vec::new();
    for (i, d) in Domain::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", p.dim()));
        }
        // 根不对时计数显「—」:0 会把「没在读」冒充成「没有数据」。
        let n = if gated {
            "—".to_string()
        } else {
            domain_count(app, *d).to_string()
        };
        let sel = *d == app.rc_domain;
        let label = format!("{}({n})", d.label());
        if sel {
            spans.push(Span::styled(format!("▸ {label}"), p.sel().bold()));
        } else {
            spans.push(Span::styled(label, p.dim()));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_domain_rail(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let gated = app.data_root_cause().is_some();
    let items: Vec<Line> = Domain::ALL
        .iter()
        .map(|d| {
            let count = if gated {
                "(—)".to_string()
            } else {
                format!("({})", domain_count(app, *d))
            };
            let sel = *d == app.rc_domain;
            let style = if sel { p.sel().bold() } else { p.dim() };
            Line::from(vec![
                Span::raw(if sel { "▸" } else { " " }),
                Span::raw(" "),
                Span::styled(d.label(), style),
                Span::raw(" "),
                Span::styled(count, p.dim()),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(items).block(
            Block::bordered()
                .border_style(p.dim())
                .title(Span::styled(" 五域 ", p.accent())),
        ),
        area,
    );
}

fn domain_count(app: &App, d: Domain) -> usize {
    match app.ls.as_ref() {
        None => 0,
        Some(l) => match d {
            Domain::Recipes => l.recipes.len(),
            Domain::Components => l.components.len(),
            Domain::Sources => l.sources.len(),
            Domain::Profiles => l.profiles.len(),
            Domain::Locks => l.locks.len(),
        },
    }
}

fn render_rows(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    // 根不对 → 当前域的入口给指路,不摆空列表(票 #27)。
    if app.data_root_cause().is_some() {
        frame.render_widget(
            Paragraph::new(data_root_signpost(app))
                .wrap(ratatui::widgets::Wrap { trim: true })
                .block(Block::bordered().border_style(p.dim()).title(Span::styled(
                    format!(" {} ", app.rc_domain.label()),
                    p.accent(),
                ))),
            area,
        );
        return;
    }
    let items: Vec<Line> = match app.ls.as_ref() {
        None => vec![Line::from(Span::styled(
            "没有 ls 数据,回概览按 r。",
            p.dim(),
        ))],
        Some(l) => match app.rc_domain {
            Domain::Recipes => l
                .recipes
                .iter()
                .map(|r| {
                    Line::from(format!(
                        "{}  {}",
                        r.name,
                        r.version.clone().unwrap_or_default()
                    ))
                })
                .collect(),
            Domain::Components => l
                .components
                .iter()
                .map(|c| {
                    Line::from(vec![
                        Span::raw(c.name.clone()),
                        Span::raw(" "),
                        Span::styled(
                            if c.depends.is_empty() {
                                "无依赖".into()
                            } else {
                                format!("依赖:{}", c.depends.join(","))
                            },
                            p.dim(),
                        ),
                    ])
                })
                .collect(),
            Domain::Sources => l
                .sources
                .iter()
                .map(|s| {
                    Line::from(vec![
                        Span::raw(s.name.clone()),
                        Span::raw(" "),
                        Span::styled(s.repo.clone().unwrap_or_default(), p.dim()),
                    ])
                })
                .collect(),
            Domain::Profiles => l
                .profiles
                .iter()
                .map(|pr| {
                    Line::from(format!(
                        "{}  {}",
                        pr.name,
                        pr.lock_id.clone().unwrap_or_default()
                    ))
                })
                .collect(),
            Domain::Locks => l
                .locks
                .iter()
                .map(|k| {
                    Line::from(format!(
                        "{}  {}",
                        k.lock_id.clone().unwrap_or_default(),
                        k.recipe.clone().unwrap_or_default()
                    ))
                })
                .collect(),
        },
    };
    let mut st = ListState::default();
    let n = app.domain_len();
    if n > 0 {
        st.select(Some(app.rc_sel.min(n - 1)));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::bordered().border_style(p.dim()).title(Span::styled(
                format!(" {} ", app.rc_domain.label()),
                p.accent(),
            )))
            .highlight_style(p.sel()),
        area,
        &mut st,
    );
}

fn render_detail(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let lines: Vec<Line> = if app.data_root_cause().is_some() {
        vec![Line::from(Span::styled(
            "根不对,详情未读取——先按指路设好数据仓根。",
            p.dim(),
        ))]
    } else {
        match app.ls.as_ref() {
            None => vec![Line::from(Span::styled("—", p.dim()))],
            Some(l) => match app.rc_domain {
                Domain::Recipes => l
                    .recipes
                    .get(app.rc_sel)
                    .map(|r| {
                        vec![
                            Line::from(format!("名称   {}", r.name)),
                            Line::from(format!(
                                "版本   {}",
                                r.version.clone().unwrap_or_else(|| "—".into())
                            )),
                            Line::from(""),
                            Line::from(Span::styled(
                                "配方是仓库里的一等公民;改它要走 bake。",
                                p.dim(),
                            )),
                        ]
                    })
                    .unwrap_or_default(),
                Domain::Locks => l
                    .locks
                    .get(app.rc_sel)
                    .map(|k| {
                        vec![
                            Line::from(format!(
                                "lockId  {}",
                                k.lock_id.clone().unwrap_or_else(|| "—".into())
                            )),
                            Line::from(format!(
                                "配方    {}",
                                k.recipe.clone().unwrap_or_else(|| "—".into())
                            )),
                            Line::from(format!(
                                "烤于    {}",
                                k.baked_at.clone().unwrap_or_else(|| "—".into())
                            )),
                            Line::from(format!(
                                "路径    {}",
                                k.path.clone().unwrap_or_else(|| "—".into())
                            )),
                        ]
                    })
                    .unwrap_or_default(),
                Domain::Profiles => l
                    .profiles
                    .get(app.rc_sel)
                    .map(|pr| {
                        vec![
                            Line::from(format!("派      {}", pr.name)),
                            Line::from(format!(
                                "状态    {}",
                                pr.state.clone().unwrap_or_else(|| "—".into())
                            )),
                            Line::from(format!(
                                "lockId  {}",
                                pr.lock_id.clone().unwrap_or_else(|| "—".into())
                            )),
                            Line::from(format!(
                                "装配于  {}",
                                pr.installed_at.clone().unwrap_or_else(|| "—".into())
                            )),
                        ]
                    })
                    .unwrap_or_default(),
                _ => vec![Line::from(Span::styled("详情按域展开(只读)。", p.dim()))],
            },
        }
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_style(p.dim())
                .title(Span::styled(" 详情 ", p.dim())),
        ),
        area,
    );
}
