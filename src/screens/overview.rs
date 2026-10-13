//! 概览屏:「家」。警示带 + 派表 + 四域计数卡 + 指令栏 + 最近日志。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};
use ratatui::Frame;

use crate::app::App;
use crate::engine::{Capability, State};
use crate::screens::{body, data_root_signpost};

/// 警示带要名的**一条订阅**(#94 工单 #105):它在订阅册投影里的下标
/// (命中侧据此换算可见行)、引擎判的到期三态、带上的文案。
struct BandSubEntry {
    idx: usize,
    st: State,
    text: String,
}

/// 警示带要名的订阅条目:走**订阅册同一份投影、同一排序**
/// ([`crate::screens::CatalogKind::Subscription`] 的 `sheet`),只取引擎已判
/// expiring / expired 的那几条。
///
/// 三道门,缺一道就是空(一档不占,警示带照旧走「装配进行中」或「一切平静」):
/// 1. 引擎明确「有订阅这一组」(`Capability::Subscription` 探测为可用);
/// 2. 订阅 ls 回音在场(`app.sub_ls`);
/// 3. 这一条的 `status` 是 expiring / expired——active 与缺 `status` 都不进带
///    (缺 status 是第四态 skip,册体自己显式「——」,不进这一档)。
///
/// **面板零本地日期运算**:三态与 `daysLeft` 全是上游 `status.go` 在 ls 回音里
/// 按 UTC 现算好随回音下发的(ADR-0009 §3),这里只点名、只抄数,不算也不改。
fn band_sub_entries(app: &App) -> Vec<BandSubEntry> {
    if !app.capabilities.enabled(Capability::Subscription) {
        return Vec::new();
    }
    let Some(sheet) = crate::screens::CatalogKind::Subscription.sheet(app) else {
        return Vec::new();
    };
    sheet
        .entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let st = crate::screens::catalog::expiry_state(e.status.as_deref());
            match st {
                State::Warn | State::Fail => Some(BandSubEntry {
                    idx: i,
                    st,
                    text: sub_band_text(e, st),
                }),
                _ => None,
            }
        })
        .collect()
}

/// 带上的条目文案:名 + 引擎给的剩余天数。过期那条不说「剩 N 天」——已经过去
/// 了,剩余天数这套说法不成立;没给 `daysLeft` 的 expiring 说「将到期」,不拿
/// 0 天冒充。
fn sub_band_text(e: &crate::engine::CatalogEntryView, st: State) -> String {
    let name = e.name.trim();
    match st {
        State::Fail => format!("{name}(已过期)"),
        _ => match e.days_left {
            Some(n) => format!("{name}(剩 {n} 天)"),
            None => format!("{name}(将到期)"),
        },
    }
}

/// 警示带的**段清单**:渲染与鼠标命中共用同一份(ADR-0005 一处几何真相)。
///
/// ladder 一道不减、一档不抢:引擎缺失 → 数据仓根三态 → 漂移/破损派 →
/// **订阅到期(#94 工单 #105,最低一档)** → 装配进行中 → 一切平静。新档只顶掉
/// 「一切平静」,上面四道既有闸的优先序一字不变(工单 #105「不挤掉既有闸」)。
///
/// 只有订阅到期这一档带 `Some(Target::AlertSubEntry(..))` 的可点段(每条目一段,
/// 载荷 = 投影下标);其余段一律只画不点。
pub fn alert_parts(app: &App) -> Vec<crate::layout::AlertPart> {
    use crate::layout::{AlertPart, AlertTone, Target};
    let hint = |text: String| AlertPart {
        text,
        target: None,
        tone: AlertTone::Hint,
    };
    let mut parts: Vec<AlertPart> = Vec::new();
    if !app.engine.found {
        parts.push(AlertPart {
            text: " ⚠ 找不到 nw 二进制 ".into(),
            target: None,
            tone: AlertTone::Alarm,
        });
        parts.push(hint("——按 F1? 不,按 ? 看帮助;到「设置」屏看指路".into()));
        return parts;
    }
    // 数据仓根门控优先于陈旧派态:根不对时,五域本就没在读(spec #24)。
    if let Some(cause) = app.data_root_cause() {
        parts.push(AlertPart {
            text: format!(" ⚠ {} ", cause.title()),
            target: None,
            tone: AlertTone::Warn,
        });
        parts.push(hint(format!(
            "——{}",
            cause.advice(app.data_root.as_deref())
        )));
        return parts;
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
        parts.push(AlertPart {
            text: " ⚠ 警示带 ".into(),
            target: None,
            tone: AlertTone::Warn,
        });
        parts.push(hint(format!("派 {} 需要关注(漂移/破损)", drift.join("、"))));
        return parts;
    }
    // 订阅到期看板(#94 工单 #105):点名引擎已判 expiring / expired 的订阅,
    // active 不占带;每条一个可点段,点它 = 跳到订阅册并选中那一条
    // (`Target::AlertSubEntry`,载荷 = 投影下标)。
    let subs = band_sub_entries(app);
    if !subs.is_empty() {
        parts.push(AlertPart {
            text: format!(" ⚠ 订阅到期 {} 条 ", subs.len()),
            target: None,
            tone: AlertTone::Warn,
        });
        for (i, e) in subs.iter().enumerate() {
            if i > 0 {
                parts.push(hint("、".into()));
            }
            parts.push(AlertPart {
                text: e.text.clone(),
                target: Some(Target::AlertSubEntry(e.idx)),
                tone: AlertTone::Entry(e.st),
            });
        }
        parts.push(hint("——点条目直达订阅册".into()));
        return parts;
    }
    if app.assembling {
        let lamp = if app.blink { "◉" } else { "○" };
        parts.push(AlertPart {
            text: format!(" {lamp} 装配进行中 "),
            target: None,
            tone: AlertTone::Busy,
        });
        parts.push(hint("其他引擎动作已锁定".into()));
        return parts;
    }
    parts.push(hint(" ✓ 一切平静".into()));
    parts
}

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
    // 右栏:指令框固定 7 行(边框 2 + 5 条指令),余高全给「最近日志」——
    // 纵向富余由 Min 吸收、可视行随高度长(不引限高、不居中,票 #22)。
    // #68 加第五条「浏览编目 (v)」:框从 6 长到 7,只多这一行,其余部件
    // (警示带/派表/计数卡)高度不变,少的那一行由最近日志的 Min 吸收。
    let right = Layout::vertical([Constraint::Length(7), Constraint::Min(3)]).split(cols[1]);
    render_commands(frame, right[0], app);
    render_recent_log(frame, right[1], app);

    render_counts(frame, rows[2], app);
    let _ = p;
}

fn render_alert(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    // 渲染走与命中共用的一份段清单([`alert_parts`] → [`alert_segments`]):
    // 放不下的整段两厢都不出现(#69 同款纪律)——屏上没有的段,鼠标也点不到,
    // 点哪打哪(ADR-0005)。段间无空隙:警示带历来是一行 Paragraph 的一段段
    // Span,不是键位行的分段。各段按自己的角色上色,与改动前逐字同源。
    let spans: Vec<Span<'static>> = crate::layout::alert_segments(area, &alert_parts(app))
        .into_iter()
        .map(|seg| {
            let style = match seg.tone {
                crate::layout::AlertTone::Alarm => p.fail(),
                crate::layout::AlertTone::Warn => p.warn(),
                crate::layout::AlertTone::Busy => p.accent(),
                crate::layout::AlertTone::Hint => p.dim(),
                // 订阅条目:到期三态的四态色(expiring 警 / expired 败),
                // 只标记不治理——不改退出码、不提醒、不靠 ANSI 颜色。
                crate::layout::AlertTone::Entry(st) => p.state(st),
            };
            Span::styled(seg.text, style)
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_profiles(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let block = ratatui::widgets::Block::bordered()
        .border_style(p.dim())
        .title(Span::styled(" 派 (profiles) ", p.accent()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // 根不对 → 五域入口给指路,不把「没在读」呈现成「没有数据」(票 #27)。
    if app.data_root_cause().is_some() {
        frame.render_widget(
            Paragraph::new(data_root_signpost(app))
                .wrap(ratatui::widgets::Wrap { trim: true })
                .style(p.dim()),
            inner,
        );
        return;
    }

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
    // 指令栏是「家」的动线,不是能力表(#68):每条常驻可点、无禁用态。第五条
    // 「浏览编目 (v)」一键到第六屏(编目),键位 `v` 与**指令栏 / 帮助 / README**
    // 三处同写(HUD 是壳级提示栏,只写 j/k/Enter/t/?/q,不写这一屏的跳屏键);
    // 点它也等于按 `v`(命中区见 `layout::overview_hits`,与这里同一组 Layout)。
    // 四条既有跳屏键 `h/a/l` 之后接 `v`(屏枚举 `Screen::Vps` 首字母,全仓无冲突)。
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
        Line::from(vec![
            Span::styled("   浏览编目   ", p.accent()),
            Span::styled("  (v)", p.dim()),
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

/// 「最近日志」:动作流水新在前,可视行数 = 框内高度(高终端自动多显示,
/// 低终端截断旧尾巴;滚动不在本屏范围,见 spec #15 的 Out of Scope)。
fn render_recent_log(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let block = ratatui::widgets::Block::bordered()
        .border_style(p.dim())
        .title(Span::styled(" 最近日志 ", p.dim()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.log.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("还没有动作流水。", p.dim())),
            inner,
        );
        return;
    }
    let lines: Vec<Line> = app
        .log
        .iter()
        .take(inner.height as usize)
        .map(|l| Line::from(Span::styled(l.clone(), p.dim())))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_counts(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    // 根不对时不摆计数:0 计数会把「没在读」冒充成「没有数据」。
    let Some(ls) = app.ls.as_ref().filter(|_| app.data_root_cause().is_none()) else {
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
