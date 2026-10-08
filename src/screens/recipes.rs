//! 配方屏:五域两栏只读浏览。h/l 切域,↑/↓ 选行;外部源域 `r` 发起
//! 登记(两段式预审段,spec #25 / 工单 #30——三个浮层见文末)。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListState, Paragraph};
use ratatui::Frame;

use crate::app::{App, Domain, RegKind, RegState};
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
    // 登记两段式的浮层(#30):盖在屏体之上(几何与 layout::register_hits
    // 共用 popup_rect 与同一组常量,点哪打哪)。
    render_register_overlay(frame, area, app);
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
            Domain::Sources => {
                let mut v: Vec<Line> = l
                    .sources
                    .iter()
                    .map(|s| {
                        Line::from(vec![
                            Span::raw(s.name.clone()),
                            Span::raw(" "),
                            Span::styled(s.repo.clone().unwrap_or_default(), p.dim()),
                        ])
                    })
                    .collect();
                // 登记入口提示(#30):可用给键位,缺能力给指路;
                // 根不可用时整个域已走上方 signpost,不进此分支。
                if app.reg_entry_enabled() {
                    v.push(Line::from(Span::styled("  按 r 登记新外部源…", p.info())));
                } else {
                    v.push(Line::from(Span::styled(
                        format!("  ⚠ 登记禁用:{}", crate::engine::capability::GATE_HINT),
                        p.warn(),
                    )));
                }
                v
            }
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

/// 详情栏内层行数(块边框各占一行):键位行钉末行按它算。
fn inner_h(area: Rect) -> usize {
    (area.height.saturating_sub(2)) as usize
}

/// 详情栏内层显示宽(块边框各占一列):折行/截断都按它算,
/// 紧凑态 66×50 与常畅态各自落到自己的宽度,不写死。
fn inner_w(area: Rect) -> usize {
    (area.width.saturating_sub(2)) as usize
}

/// 配方域建议命令区(spec #33 / 工单 #39,ADR-0003)。
///
/// 纯函数:选中配方行 + 数据仓根 + 能力结论进,展示行出——无 IO、不推导。
/// 三种缺前提各有可区分指路,**绝不渲染半条命令**;给得出时把既有那句
/// 「改它要走 bake」与命令合成一处(界面不说两遍话),并标明这是**建议命令**、
/// 执行权在操作者手里。
fn bake_suggest_lines(
    p: &crate::theme::Palette,
    app: &App,
    r: &crate::engine::RecipeRow,
    w: usize,
) -> Vec<Line<'static>> {
    let recipe = (r.name.as_str(), r.version.as_deref());
    let mut lines = vec![Line::from(Span::styled(
        "建议命令 · 执行权在你",
        p.accent(),
    ))];
    match crate::engine::commands::recipe_bake_suggest(
        Some(recipe),
        app.usable_data_root().as_deref(),
        app.capabilities
            .enabled(crate::engine::Capability::RecipeBake),
    ) {
        Ok(s) => {
            for (i, ln) in wrap(&s.display, w.saturating_sub(4)).iter().enumerate() {
                let indent = if i == 0 { "  " } else { "    " };
                let style = if app.rc_bake_sel {
                    p.sel().bold()
                } else {
                    p.text()
                };
                lines.push(Line::from(Span::styled(format!("{indent}{ln}"), style)));
            }
            // 退出码含义可查,且出处是引擎自身——面板不内置第二份真相。
            // 三行都按内层宽折行:66×50 的详情内层只有 29 列,裁掉尾部
            // 等于把「面板不代跑」与退出码表看不全(紧凑态不溢出铁律)。
            let notes = [
                "面板不代跑;不填 --out 引擎落 <配方目录>/../baked/nw-assemble.sh".to_string(),
                "退出码 0 成功 · 2 环境缺失 · 3 输入无效/校验失败 · 4 装配失败".to_string(),
                format!(
                    "—— 口径出自引擎自身(nw {}),面板不内置第二份真相。",
                    app.engine_version
                ),
            ];
            for n in &notes {
                for ln in wrap(n, w.saturating_sub(2)) {
                    lines.push(Line::from(Span::styled(format!("  {ln}"), p.dim())));
                }
            }
        }
        Err(g) => {
            lines.push(Line::from(Span::styled(
                format!("  ⚠ {}", g.title()),
                p.warn().bold(),
            )));
            for ln in wrap(&g.advice(), w.saturating_sub(4)).into_iter().take(3) {
                lines.push(Line::from(Span::styled(format!("  {ln}"), p.dim())));
            }
        }
    }
    lines
}

/// 配方域为空(空数据仓)或选中行不是配方:给人话指路,不摆空框、
/// 不给半条命令(#39 用户故事 15)。
fn bake_empty_lines(p: &crate::theme::Palette, app: &App, w: usize) -> Vec<Line<'static>> {
    let gate = app.bake_suggest_gate().err();
    let mut lines = vec![Line::from(Span::styled(
        "配方域为空:数据仓里还没有配方。",
        p.dim(),
    ))];
    if let Some(g) = gate {
        lines.push(Line::from(Span::styled(
            format!("  ⚠ {}", g.title()),
            p.warn().bold(),
        )));
        for ln in wrap(&g.advice(), w.saturating_sub(4)).into_iter().take(3) {
            lines.push(Line::from(Span::styled(format!("  {ln}"), p.dim())));
        }
    }
    lines
}

fn render_detail(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let mut lines: Vec<Line> = if app.data_root_cause().is_some() {
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
                        // 建议命令区(#39)与选中行联动重算;键位行钉在内层
                        // 末行(命中侧同几何,见 layout::recipes_hits)。
                        let mut v = vec![
                            Line::from(format!("名称   {}", r.name)),
                            Line::from(format!(
                                "版本   {}",
                                r.version.clone().unwrap_or_else(|| "——".into())
                            )),
                            Line::from(""),
                        ];
                        v.extend(bake_suggest_lines(p, app, r, inner_w(area)));
                        v
                    })
                    .unwrap_or_else(|| bake_empty_lines(p, app, inner_w(area))),
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
    // 配方域建议命令的键位行(#39):钉在详情内层**末行**——与
    // `layout::recipes_hits` 的同一行几何对齐(点哪打哪)。缺前提时也铺
    // (文案说清要先补前提),但命中侧不放行,与登记落账闸同一种纪律。
    if app.rc_domain == Domain::Recipes && app.data_root_cause().is_none() {
        let budget = inner_h(area);
        if lines.len() >= budget {
            lines.truncate(budget.saturating_sub(1));
        }
        while lines.len() < budget.saturating_sub(1) {
            lines.push(Line::from(""));
        }
        let ready = app.bake_suggest_gate().is_ok();
        let (text, style) = if !ready {
            ("  c 取用(先补上前提)  ", p.dim())
        } else if app.rc_bake_sel {
            ("  c 已选中:可复制命令  ", p.accent().bold())
        } else {
            ("  c 取用(只读选中)  ", p.accent())
        };
        lines.push(Line::from(Span::styled(text, style)));
    }
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_style(p.dim())
                .title(Span::styled(" 详情 ", p.dim())),
        ),
        area,
    );
}

// ---- 登记两段式浮层(spec #25 / 工单 #30):输入 → kind → 预审回音 → 确认 ----

/// 按显示宽截断(不切宽字符,防 CJK 混排错位)。
fn clip(s: &str, w: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw > w {
            break;
        }
        out.push(ch);
        used += cw;
    }
    out
}

/// 按显示宽折行:优先在空格处断(schema、URL、人话短语不被拦腰切开),
/// 无处可断才按字符硬切。证据整句可见,不被弹层宽度截尾。
fn wrap(s: &str, w: usize) -> Vec<String> {
    let w = w.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut rest: Vec<char> = s.chars().collect();
    while !rest.is_empty() {
        let mut used = 0usize;
        let mut fit = 0usize;
        for (i, ch) in rest.iter().enumerate() {
            let cw = unicode_width::UnicodeWidthChar::width(*ch).unwrap_or(0);
            if used + cw > w {
                break;
            }
            used += cw;
            fit = i + 1;
        }
        let fit = fit.max(1);
        if fit >= rest.len() {
            out.push(rest.iter().collect());
            break;
        }
        // 断点:前缀内最后一个空格;没有就硬切。保证至少前进一个字符。
        let brk = (1..fit).rev().find(|&i| rest[i] == ' ').unwrap_or(fit);
        out.push(
            rest[..brk]
                .iter()
                .collect::<String>()
                .trim_end()
                .to_string(),
        );
        let start = if rest.get(brk) == Some(&' ') {
            brk + 1
        } else {
            brk.max(1)
        };
        rest = rest[start.min(rest.len())..].to_vec();
    }
    out
}

/// 五字段证据落行:`[code/rc] message` 折行呈现,指路与详情随后缩进——
/// 错误/警告取前几条原样给人话,不静默降级成一句「出错了」。
fn push_evidence(
    lines: &mut Vec<Line>,
    label: &str,
    head_style: ratatui::style::Style,
    e: &crate::engine::Evidence,
    w: usize,
) {
    let head = match (&e.code, e.exit_code) {
        (Some(c), Some(rc)) => format!("[{c}/rc{rc}] {}", e.message),
        (Some(c), None) => format!("[{c}] {}", e.message),
        (None, Some(rc)) => format!("[rc{rc}] {}", e.message),
        (None, None) => e.message.clone(),
    };
    for (i, ln) in wrap(&head, w.saturating_sub(8)).iter().enumerate() {
        if i == 0 {
            lines.push(Line::from(Span::styled(
                format!("  {label}  {ln}"),
                head_style,
            )));
        } else {
            lines.push(Line::from(Span::styled(
                format!("        {ln}"),
                head_style,
            )));
        }
    }
    let dim = ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM);
    if let Some(h) = &e.hint {
        for (i, ln) in wrap(&format!("指路 {h}"), w.saturating_sub(8))
            .iter()
            .enumerate()
        {
            lines.push(Line::from(Span::styled(
                if i == 0 {
                    format!("        {ln}")
                } else {
                    format!("          {ln}")
                },
                dim,
            )));
        }
    }
    // 详情:五字段里的第五列,截到两行防止单条吃光屏高。
    if let Some(d) = &e.details {
        for ln in wrap(&format!("详情 {d}"), w.saturating_sub(8))
            .iter()
            .take(2)
        {
            lines.push(Line::from(Span::styled(format!("        {ln}"), dim)));
        }
    }
}

/// 四态标签(按冻结措辞渲染,不承诺文案);「未确认」是
/// `register.confirmed` 的 skip——设计内的待定态,显示「待定」而不是错误。
fn acc_label(a: &crate::engine::RegisterAcceptance) -> &'static str {
    if a.id == "register.confirmed" && a.status == "skip" {
        return "待定";
    }
    match a.status.as_str() {
        "pass" => "过",
        "fail" => "败",
        "warn" => "警",
        _ => "略",
    }
}

/// 登记浮层总入口:按状态机画一个;Idle 无浮层。
/// 居中基准与 `layout::register_hits` 共用 `popup_rect`,几何零分歧。
fn render_register_overlay(frame: &mut Frame, area: Rect, app: &App) {
    match app.reg_state {
        RegState::Idle => {}
        RegState::RepoInput => render_reg_input(frame, area, app),
        RegState::KindPick => render_reg_kind(frame, area, app),
        RegState::PreAudit => render_reg_preaudit(frame, area, app),
        RegState::Review => render_reg_review(frame, area, app),
        RegState::Commit => render_reg_commit(frame, area, app),
        RegState::Done => render_reg_done(frame, area, app),
    }
}

/// ① 填 repo 链接(风格照 settings::render_home_input:块光标 + 键位提示)。
fn render_reg_input(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup =
        crate::layout::popup_rect(area, crate::layout::REG_INPUT_W, crate::layout::REG_INPUT_H);
    frame.render_widget(Clear, popup);
    let cursor = format!("{}█", app.reg_repo_buf);
    let lines = vec![
        Line::from(Span::styled(cursor, p.text())),
        Line::from(Span::styled(
            "  Enter 继续 · Esc/q 放弃 · 非法格式当场拒绝留改",
            p.dim(),
        )),
        Line::from(Span::styled(
            "  接受 owner/repo、https://github.com/… 或 git@github.com:…",
            p.dim(),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记外部源 · 仓库链接 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ② kind 二选一:两行选项(选中标记 + 高亮)钉在内层第 1/2 行,
/// 键位行钉在内层末行——与 `layout::register_hits` 的行序同约定。
fn render_reg_kind(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup =
        crate::layout::popup_rect(area, crate::layout::REG_KIND_W, crate::layout::REG_KIND_H);
    frame.render_widget(Clear, popup);
    let inner_h = popup.height.saturating_sub(2);
    let sel = app.reg_kind.index();
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  kind 二选一(类别输入不给,记录落「未分类」)",
        p.dim(),
    ))];
    for (i, kind) in RegKind::ALL.iter().enumerate() {
        let on = i == sel;
        lines.push(Line::from(vec![
            Span::raw(if on { "  ▸ " } else { "    " }),
            Span::styled(kind.label(), if on { p.sel().bold() } else { p.dim() }),
        ]));
    }
    lines.push(Line::from(Span::styled("  ↑/↓ 或 s/e 选择", p.dim())));
    // 垫到「键位行 = 内层最后一行」;收窄到放不下时由 hits 侧同款守卫兜底。
    while lines.len() < inner_h.saturating_sub(1) as usize {
        lines.push(Line::from(""));
    }
    lines.truncate(inner_h.saturating_sub(1) as usize);
    lines.push(Line::from(vec![
        Span::styled(crate::layout::REG_KEY_KIND_LAUNCH, p.accent().bold()),
        Span::raw(crate::layout::RITUAL_KEY_GAP),
        Span::styled(crate::layout::RITUAL_KEY_RETURN, p.dim()),
    ]));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记外部源 · 选 kind ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ③ 预审进行中:说清这段在干什么(扫描 + 验收,文件不落);鼠标惰性。
fn render_reg_preaudit(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup =
        crate::layout::popup_rect(area, crate::layout::REG_INPUT_W, crate::layout::REG_INPUT_H);
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from(Span::styled(
            "  正在预审:扫描条目 + 验收 5(+1) 条…",
            p.text(),
        )),
        Line::from(Span::styled(
            "  不带 --yes:文件不落,拟落账记录只进回音。",
            p.dim(),
        )),
        Line::from(Span::styled("  Esc 可返航(回音将丢弃)。", p.dim())),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记外部源 · 预审 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ④ 确认态:条目清单 + 验收逐条 + 拟落账记录 + 落账闸结论;
/// 键位行钉内层末行(点整屏 = 返航,命中侧同几何)。**本段不下发任何
/// 写命令**——Enter 落账是 #31 的下一段,这里只呈现与把闸。
fn render_reg_review(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = crate::layout::popup_rect(
        area,
        crate::layout::REG_REVIEW_W,
        crate::layout::REG_REVIEW_H,
    );
    frame.render_widget(Clear, popup);
    let inner_h = popup.height.saturating_sub(2) as usize;
    let inner_w = popup.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();

    let out = app.reg_result.as_ref();
    let typed = out
        .and_then(|o| o.envelope.as_ref())
        .and_then(|e| e.data_as::<crate::engine::RegisterData>());
    match typed {
        Some(d) => {
            lines.push(Line::from(format!(
                "  仓库  {}",
                clip(
                    &format!("{} · kind {} · 提交 {}", d.repo, d.kind, d.commit),
                    inner_w.saturating_sub(5)
                )
            )));
            lines.push(Line::from(format!(
                "  名称  {}",
                clip(&d.name, inner_w.saturating_sub(5))
            )));
            let written = if d.written {
                "(已写入)"
            } else {
                "(预审:文件不落)"
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "  编目  {} {}",
                    clip(&d.catalog_path, inner_w.saturating_sub(13)),
                    written
                ),
                p.dim(),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  条目({}):", d.entries.len()),
                p.accent(),
            )));
            for e in d.entries.iter().take(4) {
                lines.push(Line::from(Span::styled(
                    format!(
                        "   {}  {}",
                        clip(&e.name, 18),
                        clip(&e.path, inner_w.saturating_sub(21))
                    ),
                    p.text(),
                )));
            }
            if d.entries.len() > 4 {
                lines.push(Line::from(Span::styled(
                    format!("   … 共 {} 条,屏内未全显", d.entries.len()),
                    p.dim(),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("  验收 5(+1):", p.accent())));
            for a in d.acceptance.iter().take(6) {
                let st = crate::engine::State::from(a.status.as_str());
                lines.push(Line::from(vec![
                    Span::raw("   "),
                    Span::styled(acc_label(a), p.state(st)),
                    Span::raw(" "),
                    Span::styled(a.id.clone(), p.dim()),
                    Span::raw(" "),
                    Span::styled(
                        clip(&a.message, inner_w.saturating_sub(10 + a.id.len())),
                        p.text(),
                    ),
                ]));
            }
            if d.acceptance.len() > 6 {
                lines.push(Line::from(Span::styled(
                    format!("   … 共 {} 条,屏内未全显", d.acceptance.len()),
                    p.dim(),
                )));
            }
            lines.push(Line::from(""));
            let cat = if d.category.is_empty() {
                "未分类".to_string()
            } else {
                d.category.join("/")
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "  拟落账  {} · {} · category {} · entries {}",
                    d.name,
                    d.kind,
                    cat,
                    d.entries.len()
                ),
                p.dim(),
            )));
            // 五字段证据呈现(错误/警告带人话 + 指路,取多不静默):
            // code · rc · message 折行,指路、详情跟随其后。
            if let Some(env) = out.and_then(|o| o.envelope.as_ref()) {
                for e in env.errors.iter().take(3) {
                    push_evidence(&mut lines, "出错", p.fail(), e, inner_w);
                }
                for e in env.warnings.iter().take(2) {
                    push_evidence(&mut lines, "警告", p.warn(), e, inner_w);
                }
            }
            // 落账闸(呈现口径):有 fail 或 rc≠0 明示不可落账——
            // 「不带操作者进入下一步」= 闸结论 + 本段根本没绑 Enter。
            if app.reg_can_commit() {
                lines.push(Line::from(Span::styled(
                    "  预审通过:验收无 fail、退出码 0(本段不写任何文件)",
                    p.info(),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    "  ⚠ 不可落账:验收有 fail 或退出码非 0——见上方结论",
                    p.warn().bold(),
                )));
            }
        }
        None => {
            // 不可定型:走证据呈现(契约守卫的「需升级」也从这里出),
            // 不按旧形状解析、不 panic(spec #28/#30 交汇)。
            let headline = out.map(|o| o.headline().to_string()).unwrap_or_default();
            for ln in wrap(
                &format!("  预审结果不可定型:{}", headline),
                inner_w.saturating_sub(2),
            )
            .into_iter()
            .take(3)
            {
                lines.push(Line::from(Span::styled(ln, p.warn())));
            }
            if let Some(sg) = out
                .and_then(|o| o.envelope.as_ref())
                .and_then(|e| e.schema_guard.as_ref())
            {
                // 正文已在 headline(守卫合成进 errors[0]),这里只补指路;
                // 折行而非裁短:schema 原文与 nw 版本必须整句可见。
                if let Some(h) = sg.hint.as_deref() {
                    for ln in wrap(h, inner_w.saturating_sub(4)).into_iter().take(3) {
                        lines.push(Line::from(Span::styled(ln, p.dim())));
                    }
                }
            }
            lines.push(Line::from(Span::styled(
                "  不按旧形状解析;面板其余部分照常可操作。",
                p.dim(),
            )));
            lines.push(Line::from(Span::styled(
                "  ⚠ 不可落账:拿不到可定型的预审结论",
                p.warn().bold(),
            )));
        }
    }

    // 内容钉进内层:超量截断、欠量垫空,键位行恒在内层最后一行
    // (与 layout::register_hits 的 key_y = popup.y + height - 2 同约定)。
    let budget = inner_h.saturating_sub(1);
    if lines.len() > budget {
        lines.truncate(budget.saturating_sub(1));
        lines.push(Line::from(Span::styled("  …(屏内未全显)", p.dim())));
    }
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    lines.push(if app.reg_can_commit() {
        // 落账闸放行(#31):键位行与 layout::register_hits 的
        // two_key_segments(同文案同序)对齐——左「Enter 落账」
        // 右「Esc 返航」,几何都在内层末行。
        Line::from(vec![
            Span::styled(crate::layout::REG_KEY_REVIEW_COMMIT, p.accent().bold()),
            Span::raw(crate::layout::RITUAL_KEY_GAP),
            Span::styled(crate::layout::RITUAL_KEY_RETURN, p.dim()),
        ])
    } else {
        Line::from(Span::styled("  Esc 返航 ", p.dim()))
    });
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记外部源 · 预审确认 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ⑤ 落账在途(#31):带 `--yes` 的写命令已下发,等引擎回音;
/// 鼠标惰性(命中侧同款,见 layout::register_hits 的 Commit 臂)。
/// 尺寸与确认屏一致(同基准不跳变)。
fn render_reg_commit(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = crate::layout::popup_rect(
        area,
        crate::layout::REG_REVIEW_W,
        crate::layout::REG_REVIEW_H,
    );
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let root = app.usable_data_root().unwrap_or_else(|| "—".to_string());
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled("  正在落账:引擎把编目写进数据仓根…", p.text())),
        Line::from(Span::styled(
            format!(
                "  写入目标  {}",
                clip(&format!("{root}/catalog.json"), inner_w.saturating_sub(13))
            ),
            p.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  登记与装配互斥:写盘期间不接新装配。",
            p.dim(),
        )),
        Line::from(Span::styled(
            "  凭据只从进程环境继承:不回显、不落盘、不进日志。",
            p.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  Esc 返航(回音将丢弃,互斥随回音收敛)",
            p.dim(),
        )),
    ];
    while lines.len() < popup.height.saturating_sub(3) as usize {
        lines.push(Line::from(""));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记外部源 · 落账 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ⑥ 结果屏(#31):写盘位置 + 记录内容 + git 变更提示 + 可复制命令;
/// 失败路给证据五字段。整屏点任意处 = 返航(命中侧同几何)。
fn render_reg_done(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup =
        crate::layout::popup_rect(area, crate::layout::REG_DONE_W, crate::layout::REG_DONE_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();

    let out = app.reg_result.as_ref();
    let typed = out
        .and_then(|o| o.envelope.as_ref())
        .and_then(|e| e.data_as::<crate::engine::RegisterData>());
    match typed {
        Some(d) if out.map(|o| o.ok).unwrap_or(false) => {
            lines.push(Line::from(Span::styled(
                format!(
                    "  已写入  {}",
                    clip(&d.catalog_path, inner_w.saturating_sub(11))
                ),
                p.info().bold(),
            )));
            let cat = if d.category.is_empty() {
                "未分类".to_string()
            } else {
                d.category.join("/")
            };
            lines.push(Line::from(format!(
                "  记录    {} · kind {} · category {} · entries {}",
                clip(&d.name, 16),
                d.kind,
                cat,
                d.entries.len()
            )));
            // repo 链接(#31 验收点名 名字/类别/链接/条目):链接是记录
            // 五要素之一,不藏在可复制命令里才看得到。
            lines.push(Line::from(format!(
                "  链接    {}",
                clip(&d.repo, inner_w.saturating_sub(8))
            )));
            for e in d.entries.iter().take(3) {
                lines.push(Line::from(Span::styled(
                    format!(
                        "    {}  {}",
                        clip(&e.name, 18),
                        clip(&e.path, inner_w.saturating_sub(23))
                    ),
                    p.dim(),
                )));
            }
            if d.entries.len() > 3 {
                lines.push(Line::from(Span::styled(
                    format!("    … 共 {} 条", d.entries.len()),
                    p.dim(),
                )));
            }
            lines.push(Line::from(""));
            // git 变更提示(spec #25 用户故事 23):铁律位置,不折叠进小字。
            lines.push(Line::from(Span::styled(
                "  ⚠ catalog.json 是一次 git 变更:走 PR + 家长审,",
                p.warn().bold(),
            )));
            lines.push(Line::from(Span::styled(
                "    别把它当本地小改;外部源域已即时刷新。",
                p.warn(),
            )));
            lines.push(Line::from(""));
            // 可复制文案(用户故事 18):贴进 PR / CI 复核。取不到可用
            // 数据仓根时明说不可复制,绝不产出 `--root .` 的假命令。
            lines.push(Line::from(Span::styled("  可复制命令", p.accent())));
            match app.usable_data_root() {
                Some(root) => {
                    let copy = format!(
                        "nw source register --repo {} --kind {} --yes --root {}",
                        app.reg_repo_buf,
                        app.reg_kind.word(),
                        root
                    );
                    for ln in wrap(&copy, inner_w.saturating_sub(4)).into_iter().take(2) {
                        lines.push(Line::from(Span::styled(format!("    {ln}"), p.text())));
                    }
                }
                None => {
                    lines.push(Line::from(Span::styled(
                        "    —(数据仓根不可用,命令不可复制)",
                        p.dim(),
                    )));
                }
            }
        }
        _ => {
            // 失败路:引擎没写盘(或写盘失败)——证据五字段原样,
            // 说法可区分(不是「装好了但没完全好」那种降级)。
            let headline = out.map(|o| o.headline().to_string()).unwrap_or_default();
            for ln in wrap(
                &format!("  落账未完成:{headline}"),
                inner_w.saturating_sub(2),
            )
            .into_iter()
            .take(2)
            {
                lines.push(Line::from(Span::styled(ln, p.fail().bold())));
            }
            if let Some(env) = out.and_then(|o| o.envelope.as_ref()) {
                for e in env.errors.iter().take(3) {
                    push_evidence(&mut lines, "出错", p.fail(), e, inner_w);
                }
                for e in env.warnings.iter().take(2) {
                    push_evidence(&mut lines, "警告", p.warn(), e, inner_w);
                }
            }
            lines.push(Line::from(Span::styled(
                "  未写入任何文件;Esc 返航后可从预审重来。",
                p.dim(),
            )));
        }
    }
    while lines.len() < popup.height.saturating_sub(3) as usize {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled("  Esc/q 返航 ", p.dim())));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记外部源 · 落账结果 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}
