//! 装配屏:选锁定集 → 发射仪式 → 七步步骤条 + stderr 实时日志。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListState, Paragraph};
use ratatui::Frame;
use serde_json::Value;

use crate::app::App;
use crate::datarepo::{archive_lamp, ArchiveState};
use crate::engine::{EngineOutput, ExitCode};
use crate::layout::{size_class, SizeClass};
use crate::screens::{body, data_root_signpost};

/// 七步:显示名 + `nw/up.v1` 的 data 里给这一步结论的**冻结字段名**(#60)。
///
/// 判据只取契约面:字段在场 = 这一步有结论;缺场 = 引擎没给(没跑到、
/// 或老引擎没这个字段)——既不打 ✓ 也不打 ✗,不猜。stderr 的人话措辞
/// 不是契约,引擎改个说法七步栏不该跟着变。
///
/// 步 6 自检的字段名留空:上游 `internal/up/up.go` 头注写明「由 CLI 层
/// 另起 nw doctor 复核,本包不做」,`up` 面永远不给这一步的结论;它的
/// 判据是 doctor 七面的 `surface` 冻结名(健康屏)。
const STEPS: [(&str, &str); 7] = [
    ("0 前置", "profile"),
    ("1 引导", "runtimeKey"),
    ("2 铺骨架", "skeletonFiles"),
    ("3 落组件", "components"),
    ("4 合配置", "configKeys"),
    ("5 写清单", "convergeHash"),
    ("6 自检", ""),
];

/// 装配中的明示语(紧凑态一行进度用):上游 `nw up` 七步执行期间没有任何
/// 增量 stdout/stderr 进度行(调研底稿 §5:chezmoi 的 stderr 被截断 400
/// 字节收进 details,跑完只在 stderr 打一行汇总)。这句话两档都摆,不让
/// 操作者把 ✓ 的缺席误读成「还没开始」。
const COARSE_NOTE: &str = "引擎无增量进度,跑完给结论";

/// 同上,常畅态七步栏底注用——栏内宽只有 12 列,放不下整句。
const COARSE_NOTE_SHORT: &str = "跑完给结论";

/// `nw/up.v1`:七步结论唯一的冻结 schema 判据(#60)。
const UP_SCHEMA: &str = "nw/up.v1";

pub fn render_assemble(frame: &mut Frame, area: Rect, app: &App) {
    if app.as_confirm {
        render_launch_ritual(frame, area, app);
        return;
    }
    // 右上注脚随次级视图换(#101):历史视图是只读档案,不是七步装配——
    // 不换的话屏上一直挂着「七步幂等装配」,讲一件此刻没发生的事。
    let note = if app.as_history.is_some() {
        "只读 · 清单存档"
    } else {
        "七步幂等装配"
    };
    let block = body(app, note);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // 装配历史次级视图(#101)排在 assembling 之后:在途屏面是七步+日志,
    // 历史是只读浏览,互不抢占(键位侧同样只在非在途开历史)。
    if app.assembling {
        render_running(frame, inner, app);
    } else if app.as_history.is_some() {
        render_history(frame, inner, app);
    } else if let Some(out) = app.as_result.as_ref() {
        render_result(frame, inner, app, out);
    } else {
        render_picker(frame, inner, app);
    }
    // 清单原文浮层(#101)最后画:盖住历史视图(与发射仪式同一款式)。
    if app.ah_raw.is_some() {
        render_manifest_raw(frame, inner, app);
    }
}

fn render_picker(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let cols =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area);
    // 锁定集属五域之一:根不对时入口给指路,不摆空清单(票 #27)。
    if app.data_root_cause().is_some() {
        frame.render_widget(
            Paragraph::new(data_root_signpost(app))
                .wrap(ratatui::widgets::Wrap { trim: true })
                .style(p.dim()),
            area,
        );
        return;
    }
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
                    Span::styled(
                        l.recipe
                            .as_ref()
                            .map(|r| r.label())
                            .unwrap_or_else(|| "—".into()),
                        p.dim(),
                    ),
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
            // 两个取消语义分开说(#59):返航在发射前(还没下发,什么都没发生),
            // 中止在发射后(在途子进程真止,可能已写了一半)——不共用一键,
            // 也不共用一句话;收敛路由指「重跑同一条命令」+ 健康屏 doctor。
            Line::from(Span::styled("  Esc 返航 = 还没下发,什么都没发生;", p.dim())),
            Line::from(Span::styled(
                "  x 中止 = 在途子进程真止,可能写了一半;",
                p.dim(),
            )),
            Line::from(Span::styled(
                "  up 幂等:重跑同一命令收敛,doctor 说了算。",
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

/// 装配历史次级视图(票 #101):按派列出清单存档(lockId / installedAt /
/// convergeHash),回车看清单原文。数据全部来自 `app.as_history`
/// (`datarepo::HistoryView`)——本函数零路径知识(接缝纪律,由
/// `datarepo::tests::屏幕层零路径_api` 钉死)。
///
/// 两档排布跟选锁定集同宗(百分比双栏,不按尺寸类另开一套):窄终端
/// 下双栏自然收窄,快照两档各守一次不溢出(工单 #101 验收)。
fn render_history(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let Some(view) = app.as_history.as_ref() else {
        return;
    };
    let cols =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area);

    // NW_HOME 安装根不存在:指路,不摆一个骗人的空列表(与数据仓根
    // 三态指路同族——「没在读」与「没有数据」不许互相冒充)。
    if view.root_missing {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(" ⚠ NW_HOME 下没有安装根 ", p.warn().bold())),
                Line::from(Span::styled(
                    format!("NW_HOME:{} 不存在——没有装配记录。", app.nw_home),
                    p.dim(),
                )),
                Line::from(Span::styled(
                    "先装配一个派再回来看;或去设置屏按 n 轮换 / i 手填 NW_HOME。",
                    p.dim(),
                )),
            ])
            .wrap(ratatui::widgets::Wrap { trim: true }),
            area,
        );
        return;
    }

    let vis = app.ah_visible();
    let items: Vec<Line> = if vis.is_empty() {
        vec![Line::from(Span::styled(
            if app.ah_filter.trim().is_empty() {
                "这个安装根下一个派都还没有——先装配一个派,历史就有第一行。"
            } else {
                "没有派命中过滤词(按 Esc 清词看全部)"
            },
            p.dim(),
        ))]
    } else {
        vis.iter()
            .map(|&i| history_row(view.rows[i].clone(), p))
            .collect()
    };
    let mut st = ListState::default();
    if !vis.is_empty() {
        st.select(Some(app.ah_sel.min(vis.len() - 1)));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::bordered()
                    .border_style(p.dim())
                    .title(Span::styled(" 装配历史(清单存档) ", p.accent())),
            )
            .highlight_style(p.sel()),
        cols[0],
        &mut st,
    );
    frame.render_widget(
        Paragraph::new(history_detail(app, view))
            .wrap(ratatui::widgets::Wrap { trim: true })
            .block(
                Block::bordered()
                    .border_style(p.dim())
                    .title(Span::styled(" 存档详情 ", p.dim())),
            ),
        cols[1],
    );
}

/// 历史列表一行:结果灯 + 派名 + 一行摘要(就绪给锁定集/时刻/收敛;
/// 缺失/破损给状态词)。灯用 `engine::State` 同一套(健康屏怎么画,
/// 这里怎么画),映射在 `datarepo::archive_lamp`(工单 #101:缺失=
/// 降级、损坏=破损)。
fn history_row(a: crate::datarepo::ProfileArchive, p: &crate::theme::Palette) -> Line<'static> {
    let st = archive_lamp(&a.state);
    let lamp = Span::styled(p.lamp(st), p.state(st));
    let name = Span::styled(format!("{:<12}", a.profile), p.info());
    let tail = match &a.state {
        ArchiveState::Ready(m) => Span::styled(
            format!(
                "{}  {}  {}",
                if m.lock_id.is_empty() {
                    "—"
                } else {
                    &m.lock_id
                },
                if m.installed_at.is_empty() {
                    "—"
                } else {
                    &m.installed_at
                },
                if m.converge_hash.is_empty() {
                    "—"
                } else {
                    &m.converge_hash
                }
            ),
            p.dim(),
        ),
        ArchiveState::Missing => Span::styled("清单缺失(上一轮没提交完)", p.warn()),
        ArchiveState::Broken(why) => Span::styled(format!("清单破损:{why}"), p.fail()),
    };
    Line::from(vec![lamp, Span::raw(" "), name, Span::raw(" "), tail])
}

/// 存档详情(右栏):选中行的全字段 + 键位说明 + 后续票挂点占位。
/// 键位说明学选锁定集的「仪式说明」栏——屏上写着,帮助浮层同源。
fn history_detail(app: &App, view: &crate::datarepo::HistoryView) -> Vec<Line<'static>> {
    let p = &app.palette;
    let vis = app.ah_visible();
    let Some(&i) = vis.get(app.ah_sel.min(vis.len().saturating_sub(1))) else {
        return vec![Line::from(Span::styled("没有选中的派(↑/↓ 选行)", p.dim()))];
    };
    let a = &view.rows[i];
    let mut lines: Vec<Line> = Vec::new();
    match &a.state {
        ArchiveState::Ready(m) => {
            lines.push(Line::from(vec![
                Span::styled(" 派       ", p.dim()),
                Span::raw(a.profile.clone()),
            ]));
            lines.push(Line::from(vec![
                Span::styled(" 锁定集   ", p.dim()),
                Span::styled(
                    if m.lock_id.is_empty() {
                        "—".into()
                    } else {
                        m.lock_id.clone()
                    },
                    p.info(),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled(" 时刻     ", p.dim()),
                Span::raw(if m.installed_at.is_empty() {
                    "—".into()
                } else {
                    m.installed_at.clone()
                }),
            ]));
            lines.push(Line::from(vec![
                Span::styled(" 收敛     ", p.dim()),
                Span::raw(if m.converge_hash.is_empty() {
                    "—".into()
                } else {
                    m.converge_hash.clone()
                }),
            ]));
            lines.push(Line::from(vec![
                Span::styled(" 配方     ", p.dim()),
                Span::raw(format!(
                    "{} {}",
                    if m.recipe_name.is_empty() {
                        "—"
                    } else {
                        &m.recipe_name
                    },
                    m.recipe_version
                )),
            ]));
            lines.push(Line::from(vec![
                Span::styled(" 运行期   ", p.dim()),
                Span::raw(if m.runtime_key.is_empty() {
                    "—".into()
                } else {
                    m.runtime_key.clone()
                }),
            ]));
            lines.push(Line::from(vec![
                Span::styled(" 引擎     ", p.dim()),
                Span::raw(if m.engine_nw.is_empty() {
                    "—".into()
                } else {
                    m.engine_nw.clone()
                }),
                Span::raw(format!(" · 组件 {} 条", m.components)),
            ]));
            lines.push(Line::from(Span::styled(
                " Enter 看清单原文;组件全貌在原文里。",
                p.dim(),
            )));
            // 后续票挂点先占位(工单 #101):跳转入口在本票不实现,
            // 位置与话说清,免得操作者到处找。
            lines.push(Line::from(Span::styled(
                " 回收站浏览 / 存档日志 的跳转入口预留给 #102 / #103。",
                p.dim(),
            )));
        }
        ArchiveState::Missing => {
            lines.push(Line::from(Span::styled(
                format!(" {}:清单缺失——上一轮没提交完的未完成态。", a.profile),
                p.warn(),
            )));
            lines.push(Line::from(Span::styled(
                " 重跑同一命令收敛(up 幂等);没有原文可看。",
                p.dim(),
            )));
        }
        ArchiveState::Broken(why) => {
            lines.push(Line::from(Span::styled(
                format!(" {}:清单破损——{}", a.profile, why),
                p.fail(),
            )));
            lines.push(Line::from(Span::styled(
                " Enter 仍可看原文(解析失败恰恰最该看);修不好先重跑 up。",
                p.dim(),
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" ↑/↓ 选行 · / 过滤", p.dim())));
    lines.push(Line::from(Span::styled(
        " Enter 看清单原文 · Esc/h 返回选锁定集",
        p.dim(),
    )));
    lines
}

/// 清单原文浮层(工单 #101 故事 3):居中大浮层,原文逐行呈现;
/// Esc / 点任意处关(命中侧 `Target::DismissPopup` 同路)。原文即
/// 数据仓里那个文件的字节内容——面板不改写、不排版、不猜测。
fn render_manifest_raw(frame: &mut Frame, area: Rect, app: &App) {
    let Some(raw) = app.ah_raw.as_ref() else {
        return;
    };
    let p = &app.palette;
    let profile = app
        .as_history
        .as_ref()
        .and_then(|h| {
            app.ah_visible()
                .get(app.ah_sel)
                .map(|&i| h.rows[i].profile.clone())
        })
        .unwrap_or_default();
    let w = 72u16.min(area.width.saturating_sub(2));
    let h = area.height.saturating_sub(2).min(20);
    let popup = Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    );
    frame.render_widget(Clear, popup);
    let lines: Vec<Line> = raw.lines().map(Line::from).collect();
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(
                        format!(" 清单原文 · {profile} "),
                        p.accent().bold(),
                    ))
                    .title_bottom(Line::from(Span::styled(" Esc 关 ", p.dim()))),
            )
            .style(p.surface()),
        popup,
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
                lock.and_then(|l| l.recipe.as_ref().map(|r| r.label()))
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
            // 三段文案与命中分半同源(layout::RITUAL_KEY_*),点哪打哪
            Span::styled(crate::layout::RITUAL_KEY_LAUNCH, p.accent().bold()),
            Span::raw(crate::layout::RITUAL_KEY_GAP),
            Span::styled(crate::layout::RITUAL_KEY_RETURN, p.dim()),
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
    match size_class(frame.area()) {
        SizeClass::Comfy => render_running_comfy(frame, area, app),
        SizeClass::Compact => render_running_compact(frame, area, app),
    }
}

/// 常畅态:七步栏(定宽 14)+ stderr 日志 + 末行键位行(可点「中止」段,#71)。
/// 键位行占末行 1 行,余高全归七步栏/日志;键位行的命中区由
/// `layout::assemble_hits` 用同一组 Layout 算出——渲染与命中共用一份几何
/// (ADR-0005),不另写一套「键位行在第几行」。
fn render_running_comfy(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let rows = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(area);
    let cols = Layout::horizontal([Constraint::Length(14), Constraint::Min(20)]).split(rows[0]);
    frame.render_widget(step_rail(p), cols[0]);
    frame.render_widget(log_block(app, p, cols[1].height), cols[1]);
    frame.render_widget(running_key_line(app, p), rows[1]);
}

/// 紧凑态:七步栏收成一行进度,日志独占中段,末行是键位行(可点「中止」段,#71)。
/// 判定走 `layout::size_class`(终端面每帧现算),不在此处散判宽窄;键位行命中区
/// 由 `layout::assemble_hits` 用同一组 Layout 算出(渲染与命中共用一份几何)。
fn render_running_compact(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(progress_line(p), rows[0]);
    frame.render_widget(log_block(app, p, rows[1].height), rows[1]);
    frame.render_widget(running_key_line(app, p), rows[2]);
}

/// 七步栏(常畅态用):装配中七步一律**未决**,底注明示粗粒度(#60)。
///
/// 旧实现按「stderr 日志里出现过步骤名」打 ✓——拿人话措辞当契约。引擎
/// 一旦改说法、或某一步恰好不打日志,七步栏就显示错,而操作者看不出那是
/// 错的。现在:在途时一个 ✓ 都不打(引擎确实没有增量信号),结论等 envelope
/// 落地后按冻结字段判(见 [`step_verdict_line`])。
fn step_rail(p: &crate::theme::Palette) -> Paragraph<'static> {
    let mut lines: Vec<Line> = STEPS
        .iter()
        .map(|(name, _)| {
            Line::from(vec![
                Span::raw("·"),
                Span::raw(" "),
                Span::styled(*name, p.dim()),
            ])
        })
        .collect();
    // 明示粗粒度:不让操作者把 ✓ 的缺席误读成「还没开始」。
    lines.push(Line::from(Span::styled(COARSE_NOTE_SHORT, p.dim())));
    Paragraph::new(lines).block(
        Block::bordered()
            .border_style(p.dim())
            .title(Span::styled(" 七步 ", p.accent())),
    )
}

/// 一行进度(紧凑态用):七格未决标记 + 明示粗粒度(#60)。
/// 在途没有可判决的增量信号,所以没有「当前步」可指——指了就是猜。
fn progress_line(p: &crate::theme::Palette) -> Paragraph<'static> {
    let mut spans = vec![Span::styled("七步 ", p.accent())];
    for _ in STEPS.iter() {
        spans.push(Span::styled("·", p.dim()));
    }
    spans.push(Span::raw(" "));
    spans.push(Span::styled(COARSE_NOTE, p.dim()));
    Paragraph::new(Line::from(spans))
}

/// stderr 实时日志块,两档共用(只随分栏几何变化);`height` 是日志块自身高度,
/// 只取可见行数(比旧实现按整屏高多取的部分会被 Paragraph 裁掉,渲染等价)。
/// `as_log_offset`(#21 滚轮偏移,0=跟随最新)先跳过偏移再取窗——偏移为 0
/// 时与逐字节旧观感一致;drain 陈旧偏移用 min 钳在窗内。
fn log_block(app: &App, p: &crate::theme::Palette, height: u16) -> Paragraph<'static> {
    let offset = app.as_log_offset.min(app.as_log.len());
    let log: Vec<Line> = app
        .as_log
        .iter()
        .rev()
        .skip(offset)
        .take(height as usize)
        .map(|l| Line::from(l.clone()))
        .collect();
    Paragraph::new(log).block(
        Block::bordered()
            .border_style(p.dim())
            // 中止提示(#71)从块标题挪到正文末行的可点键位行「x 中止」——块标题
            // 只描述这块是什么(stderr 实时日志)。话术单源在键位行上:渲染画的与
            // 命中测的同一份(`assemble::assemble_key_parts`),且只在真有 up 在途
            // (as_running)时在场;不在场时连提示都不给(点不到一个当前不存在的动作,
            // 比点得到更糟)。
            .title(Span::styled(" stderr 实时日志 ", p.dim())),
    )
}

/// 装配运行中正文键位行可点段之间的分隔串(#71):与命中侧
/// [`crate::layout::assemble_hits`] 共用同一份,段宽按 `unicode_width` 量——
/// 一处真相,点哪打哪(目前键位行只有一个可点段,此 gap 留作同款纪律,#67)。
pub(crate) const ASSEMBLE_KEY_GAP: &str = "  ";

/// 装配运行中正文键位行的「中止」可点文案(#71):点它 = 按 `x` 进程级真止在途
/// `nw up` 子进程。渲染(`render_running_*`)与命中(`layout::assemble_hits`)共用
/// 这一份字符串量宽——点哪打哪(ADR-0005,#67 接缝的同一个用户形状)。段文案带
/// 键 hint `x`(同 `m 换册` / `Esc 返航` 的先例):鼠标用户点它,键盘用户照按。
///
/// 与发射前浮层的「Esc 返航」是两个取消、两套话术,不许混:返航是命令还没下发
/// (什么都没发生),中止是在途子进程真止(可能停在未收敛中间态)。
pub(crate) const ASSEMBLE_ABORT: &str = "x 中止";

/// 装配运行中正文键位行的**段清单**(#71,通用接缝 `key_line_segments` 的用户):
/// 只在真有 `up` 在途(`as_running`)时给一个可点的「中止」段(`Some(Abort)`);
/// 不在场时给空——既不画也不给命中(规格故事 47:点不到一个当前不存在的动作)。
/// 渲染与命中共用这同一份清单,段文案与段宽一处真相(ADR-0005)。第三项
/// `live` 恒 `true`(#70 的禁用态位,装配中止段没有门禁——有 up 在途就止得了)。
pub(crate) fn assemble_key_parts(app: &App) -> Vec<crate::layout::KeyPart<'static>> {
    use crate::layout::Target;
    if app.as_running {
        vec![(ASSEMBLE_ABORT, Some(Target::Abort), true)]
    } else {
        Vec::new()
    }
}

/// 把段清单画成一行键位行(渲染侧,#71):**可用**可点段 accent 加粗、提示段
/// 与**禁用态**可点段(#70)都 dim,段间用 [`ASSEMBLE_KEY_GAP`] 连接——与命中侧
/// [`crate::layout::key_line_segments`] 同一份 `parts` + 同一个 gap,屏上画的
/// 和点得的逐字同源。没有段(非 up 在途)时画空行,占位不显示任何东西。
fn running_key_line(app: &App, p: &crate::theme::Palette) -> Paragraph<'static> {
    let parts = assemble_key_parts(app);
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, (text, target, live)) in parts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(ASSEMBLE_KEY_GAP, p.dim()));
        }
        let style = match (target.is_some(), live) {
            (true, true) => p.accent().bold(), // 可用动作段
            (true, false) => p.dim(),          // #70 禁用态:在场、可点,但不加强调
            _ => p.dim(),                      // 提示段
        };
        spans.push(Span::styled((*text).to_string(), style));
    }
    Paragraph::new(Line::from(spans))
}

/// 跑完后的七步结论行(#60):判据只有冻结面。
///
/// `nw/up.v1` 的 data 字段**在场** = 这一步有结论;缺场 = 引擎没给
/// (没跑到 / 老引擎没这个字段),既不打 ✓ 也不打 ✗。schema 不是
/// `nw/up.v1` 时明说取得到不到——不按旧形状降级解析(与 #28 契约守卫
/// 同一立场)。步 6 自检在 up 面恒无字段,它的判据是 doctor 七面的
/// `surface` 冻结名(健康屏),这里不替它猜。
fn step_verdict_line(out: &EngineOutput, p: &crate::theme::Palette) -> Line<'static> {
    let mut spans = vec![Span::raw("  七步   ")];
    let up_data = out
        .envelope
        .as_ref()
        .filter(|e| e.schema == UP_SCHEMA)
        .map(|e| &e.data);
    match up_data {
        Some(data) => {
            let settled = settled_steps(data);
            let n = settled.iter().filter(|s| **s).count();
            let marks: String = settled.iter().map(|s| if *s { "✓" } else { "·" }).collect();
            spans.push(Span::styled(marks, if n == 0 { p.dim() } else { p.ok() }));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                if n == 0 {
                    "引擎没给七步结论(up 面 data 无字段)".to_string()
                } else {
                    format!("{n} 面有结论;自检归 doctor(up 不自检)")
                },
                p.dim(),
            ));
        }
        None => spans.push(Span::styled(
            format!("结论取不到:envelope 不是 {UP_SCHEMA}(契约守卫)"),
            p.dim(),
        )),
    }
    Line::from(spans)
}

/// 按 `nw/up.v1` data 的冻结字段在场判七步结论(#60)。
fn settled_steps(data: &Value) -> [bool; 7] {
    let obj = data.as_object();
    STEPS.map(|(_, field)| !field.is_empty() && obj.map(|o| o.contains_key(field)).unwrap_or(false))
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
    // 七步结论:跑完了才按冻结面判(#60),不拿 stderr 措辞凑。
    lines.push(step_verdict_line(out, p));
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
