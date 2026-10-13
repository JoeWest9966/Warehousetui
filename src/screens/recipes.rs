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
    // 作者侧脚手架浮层(#110):与登记/落点浮层同层覆盖(它在场时键位/命中都被
    // 吞,见 on_key 守卫与 layout::hitmap;三个浮层不会同时在场的语义由各自
    // 的入口门 + dismiss_popup 受理次序保证)。
    crate::screens::scaffold::render_scaffold_overlay(frame, area, app);
    // 落点溯源浮层(#115):与登记浮层同层覆盖(它在场时键位/命中都被吞,
    // 见 on_key 守卫与 layout::recipes_hits;两个浮层不会同时在场的语义由
    // dismiss_popup 的受理次序保证)。
    render_landing_overlay(frame, area, app);
    // 组件打包器浮层(#112):表单 → 拟落账摊开 → 确认 → 原子落盘。与另两个
    // 浮层同层覆盖、同一条弹层优先级(键位见 `App::on_key` 的守卫,鼠标见
    // `layout::recipes_hits`);它在场时五域键位一个都不受理。
    render_pack_overlay(frame, area, app);
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

/// 登记入口提示(#30):可用给键位、缺能力给指路 —— 外部源域的**列表尾行**
/// 与**详情空态指路**共用这一句,两种呈现里同一概念只有一个拼写。
/// 根不可用时不进此路(整域已走上方 signpost)。
fn register_entry_hint(app: &App) -> Line<'static> {
    let p = &app.palette;
    if app.reg_entry_enabled() {
        Line::from(Span::styled("  按 r 登记新外部源…", p.info()))
    } else {
        Line::from(Span::styled(
            format!("  ⚠ 登记禁用:{}", crate::engine::capability::GATE_HINT),
            p.warn(),
        ))
    }
}

/// 组件域新建入口提示(#110):与登记入口提示同级的**入口行**(不是数据行),
/// 始终留在列表尾行。skill 是组件(CONTEXT),脚手架落进数据仓 `skills/`;
/// 入口门(数据仓根就绪 / 不重入)在 `App::start_scaffold` 把守,根不可用时
/// 整域已走上方的 `data_root_signpost`,不进此路。
fn scaffold_entry_hint(app: &App) -> Line<'static> {
    let p = &app.palette;
    Line::from(Span::styled("  按 n 新建 skill 脚手架…", p.info()))
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
    // 过滤后的可见行(#61):列表、选中、命中区、详情都走这一份下标——
    // 「看得见的那一行」全屏只有一个口径。
    let vis = app.rc_visible();
    let filtering = !app.rc_filter.trim().is_empty();
    let mut items: Vec<Line> = match app.ls.as_ref() {
        None => vec![Line::from(Span::styled(
            "没有 ls 数据,回概览按 r。",
            p.dim(),
        ))],
        Some(l) => match app.rc_domain {
            Domain::Recipes => vis
                .iter()
                .copied()
                .filter_map(|i| l.recipes.get(i))
                .map(|r| {
                    Line::from(format!(
                        "{}  {}",
                        r.name,
                        r.version.clone().unwrap_or_default()
                    ))
                })
                .collect(),
            Domain::Components => vis
                .iter()
                .copied()
                .filter_map(|i| l.components.get(i))
                .map(|c| {
                    Line::from(vec![
                        Span::raw(c.name.clone()),
                        Span::raw(" "),
                        Span::styled(
                            if c.depends.is_empty() {
                                "无依赖".into()
                            } else {
                                format!(
                                    "依赖:{}",
                                    c.depends
                                        .iter()
                                        .map(|d| d.label())
                                        .collect::<Vec<_>>()
                                        .join(",")
                                )
                            },
                            p.dim(),
                        ),
                    ])
                })
                .collect(),
            Domain::Sources => vis
                .iter()
                .copied()
                .filter_map(|i| l.sources.get(i))
                .map(|s| {
                    Line::from(vec![
                        Span::raw(s.name.clone()),
                        Span::raw(" "),
                        // 链接缺失给「——」,不用空串冒充「没有这个字段」(#37)
                        Span::styled(
                            s.repo.as_deref().filter(|r| !r.is_empty()).unwrap_or("——"),
                            p.dim(),
                        ),
                    ])
                })
                .collect(),
            Domain::Profiles => vis
                .iter()
                .copied()
                .filter_map(|i| l.profiles.get(i))
                .map(|pr| {
                    Line::from(format!(
                        "{}  {}",
                        pr.name,
                        pr.lock_id.clone().unwrap_or_default()
                    ))
                })
                .collect(),
            Domain::Locks => vis
                .iter()
                .copied()
                .filter_map(|i| l.locks.get(i))
                .map(|k| {
                    Line::from(format!(
                        "{}  {}",
                        k.lock_id.clone().unwrap_or_default(),
                        k.recipe.as_ref().map(|r| r.label()).unwrap_or_default()
                    ))
                })
                .collect(),
        },
    };
    // 空列表的两态指路(#61):域真空 vs 被过滤空——占数据行位。
    if app.ls.is_some() && vis.is_empty() {
        items.extend(list_empty_lines(app, p));
    }
    // 登记入口提示(#30)是入口、不是数据行:始终留在列表**尾行**,
    // 过滤后也不例外(根不可用时整个域已走上方 signpost,不进此分支)。
    if app.rc_domain == Domain::Sources && app.ls.is_some() {
        items.push(register_entry_hint(app));
    }
    // 组件域新建入口(#110):与外部源域登记入口同级的尾行入口(根不可用时
    // 整域已走上方 signpost,不进此分支)。
    if app.rc_domain == Domain::Components && app.ls.is_some() {
        items.push(scaffold_entry_hint(app));
    }
    let mut st = ListState::default();
    let n = app.domain_len();
    if n > 0 {
        st.select(Some(app.rc_sel.min(n - 1)));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::bordered()
                    .border_style(p.dim())
                    .title(Span::styled(list_title(app, filtering), p.accent())),
            )
            .highlight_style(p.sel()),
        area,
        &mut st,
    );
}

/// 列表块标题(#61):过滤态显「命中/总数」——被过滤空在标题层就与域真空
/// (标题只有域名)可区分;无过滤词时与过滤前逐字节相同。
fn list_title(app: &App, filtering: bool) -> String {
    let label = app.rc_domain.label();
    if !filtering {
        return format!(" {label} ");
    }
    format!(
        " {label} {}/{} ",
        app.domain_len(),
        domain_count(app, app.rc_domain)
    )
}

/// 空列表的两种因由(#61):照仓内 `DataRootCause` / `BakeGate` 的既有范式——
/// 一个 enum 管「为什么空」,标题与说法两两不同。域真空说「这一域没有条目」,
/// 被过滤空说「词把条目筛掉了」并给改词/清空的路;两种空互不冒充。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ListEmptyCause {
    /// 数据仓里这一域就是没有条目。
    Vacant,
    /// 有关键词,把它全筛掉了。
    Filtered,
}

impl ListEmptyCause {
    /// 由当前过滤词定性:空串 = 域真空,非空 = 被过滤空。
    fn of_app(app: &App) -> Self {
        if app.rc_filter.trim().is_empty() {
            Self::Vacant
        } else {
            Self::Filtered
        }
    }

    /// 指路标题:两态各说各话。
    fn title(self, d: Domain) -> String {
        match self {
            Self::Vacant => format!("{}域为空", d.label()),
            Self::Filtered => format!("{}无命中", d.label()),
        }
    }

    /// 指路人话:每态一条,带各自的事实与下一步。
    fn advice(self, d: Domain, q: &str, total: usize) -> Vec<String> {
        match self {
            Self::Vacant => vec![format!("数据仓的{}里还没有条目。", d.label())],
            Self::Filtered => vec![
                format!("「{q}」在 {total} 条里没有命中。"),
                "按 / 改词 · Esc 清空过滤".to_string(),
            ],
        }
    }
}

/// 空态指路行(#61):标题一行 + 说法若干行,两态两套文案。列表块与详情块
/// **共用同一份**(一个概念只有一个拼写,不在两处各写一句)。
fn list_empty_lines(app: &App, p: &crate::theme::Palette) -> Vec<Line<'static>> {
    let cause = ListEmptyCause::of_app(app);
    let total = domain_count(app, app.rc_domain);
    let mut v = vec![Line::from(Span::styled(
        cause.title(app.rc_domain),
        p.warn(),
    ))];
    for a in cause.advice(app.rc_domain, app.rc_filter.trim(), total) {
        v.push(Line::from(Span::styled(a, p.dim())));
    }
    v
}

/// 字段缺失(引擎没给 / 空串)时给「——」——不猜、不编默认值(#37 验收)。
/// 回 owned 串:详情行整个是自持的,不把 `app.ls` 的借用拽进渲染签名。
fn field_or_dash(v: Option<&str>) -> String {
    v.filter(|x| !x.is_empty()).unwrap_or("——").to_string()
}

/// 外部源详情(#35/#37):类别(含多级)、仓库链接、一次扫描得出的条目清单。
/// 纯函数——行数据进、展示行出,无 IO、不推导、不改写引擎措辞:
/// 字段缺失显式「——」,空类别按编目约定兜底成「未分类」(引擎不落该字面量)。
/// 不折行:长值由 Paragraph 在块边截断,不在细节栏里折行(66×50 紧凑态铁律)。
fn source_detail(p: &crate::theme::Palette, s: &crate::engine::SourceRow) -> Vec<Line<'static>> {
    // 类别是多级数组:按层级拼(单层内禁 "/",拼出来无歧义);
    // 空数组 = 未分类 —— 消费方兜底,不是面板自造的值。
    let category = if s.category.is_empty() {
        "未分类".to_string()
    } else {
        s.category.join("/")
    };
    let mut lines = vec![
        Line::from(format!("名称  {}", field_or_dash(Some(s.name.as_str())))),
        Line::from(format!("kind  {}", field_or_dash(s.kind.as_deref()))),
        Line::from(format!("类别  {category}")),
        Line::from(format!("仓库  {}", field_or_dash(s.repo.as_deref()))),
        Line::from(""),
    ];
    if s.entries.is_empty() {
        lines.push(Line::from("条目  ——"));
        return lines;
    }
    lines.push(Line::from(format!("条目  {}", s.entries.len())));
    for e in s.entries.iter() {
        // 名字与路径各占一行:66×50 的详情内层只有 29 列,`名字  路径`
        // 单行排不下,截断等于把条目清单看不全;两行由渲染方逐行画,
        // 不是折行——行数是写死的,不随宽度变(不折行、不溢出)。
        lines.push(Line::from(Span::styled(
            format!("  {}", field_or_dash(Some(e.name.as_str()))),
            p.text(),
        )));
        lines.push(Line::from(Span::styled(
            format!("    {}", field_or_dash(Some(e.path.as_str()))),
            p.dim(),
        )));
    }
    lines
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

/// 配方域**新建骨架**建议命令区(spec #33 / 工单 #58,ADR-0003 + ADR-0006):
/// 紧随 bake 之后同一块地方给出 `nw recipe new …` 文案。与 bake 完全同源——
/// argv 与展示串都出自 `commands::recipe_new_suggest` 这一个纯函数层,
/// 旗标拼装只有一处真相;面板**永远不执行**(执行权留给操作者,
/// `App` 没有任何路径会把它下发)。
///
/// 三种缺前提各有可区分指路(标题 + 说法),**绝不渲染半条命令**;
/// 说法与 bake 的同名指路不同文——点名的旗标是 `--out-dir`,不冒充。
///
/// `room` 是详情内层还能给几行(末行已预留给 `c` 键位行)。竖向放不下时
/// 从尾部收口:先收引擎口径注(整条收,不截半句),头(标题 + 命令)
/// 放不下就整块不渲染——窗口再窄,屏上也不会出现半条命令。
fn recipe_new_suggest_lines(
    p: &crate::theme::Palette,
    app: &App,
    r: &crate::engine::RecipeRow,
    w: usize,
    room: usize,
) -> Vec<Line<'static>> {
    // 头:标题 + 命令本体(或指路标题 + 说法)。
    let mut head = vec![Line::from(Span::styled(
        "建议命令 · 新建配方骨架(执行权在你)",
        p.accent(),
    ))];
    let mut notes: Vec<String> = Vec::new();
    match crate::engine::commands::recipe_new_suggest(
        Some(r.name.as_str()),
        app.usable_data_root().as_deref(),
        app.capabilities
            .enabled(crate::engine::Capability::RecipeNew),
    ) {
        Ok(s) => {
            for (i, ln) in wrap(&s.display, w.saturating_sub(4)).iter().enumerate() {
                let indent = if i == 0 { "  " } else { "    " };
                head.push(Line::from(Span::styled(format!("{indent}{ln}"), p.text())));
            }
            // 引擎自己的缺省与落点照原话说:面板不内置第二份真相,
            // 也不替引擎承诺它没说的行为。
            notes.push("面板不代跑;不填 --profile 引擎按配方名兜底(#8 布局)".to_string());
            notes.push(
                "骨架落 <out-dir>/recipes/<名>/recipe.yaml —— schema nw/recipe-new.v1".to_string(),
            );
        }
        Err(g) => {
            head.push(Line::from(Span::styled(
                format!("  ⚠ {}", g.title()),
                p.warn().bold(),
            )));
            notes.push(g.advice());
        }
    }
    if head.len() > room {
        return Vec::new();
    }
    let mut lines = head;
    for n in &notes {
        let wrapped = wrap(n, w.saturating_sub(2));
        if lines.len() + wrapped.len() > room {
            break;
        }
        for ln in wrapped {
            lines.push(Line::from(Span::styled(format!("  {ln}"), p.dim())));
        }
    }
    lines
}

fn render_detail(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    // 过滤后的可见下标:详情取的是**看得见的那一行**(#61),不过滤时与过去
    // 的 `l.xxx.get(rc_sel)` 逐个等价。
    let vis = app.rc_visible();
    let row = |i: usize| vis.get(i).copied();
    let mut lines: Vec<Line> = if app.data_root_cause().is_some() {
        vec![Line::from(Span::styled(
            "根不对,详情未读取——先按指路设好数据仓根。",
            p.dim(),
        ))]
    // 被过滤空(#61):详情不给旧行残影,也不拿域真空的说法冒充——
    // 与列表块同一份因由,标题与说法两态两套。
    } else if app.ls.is_some() && !app.rc_filter.trim().is_empty() && vis.is_empty() {
        list_empty_lines(app, p)
    } else {
        match app.ls.as_ref() {
            None => vec![Line::from(Span::styled("—", p.dim()))],
            Some(l) => match app.rc_domain {
                Domain::Recipes => row(app.rc_sel)
                    .and_then(|i| l.recipes.get(i))
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
                        // 新建骨架建议命令(#58):紧随 bake 同一块区域,
                        // 与 bake 同源同层(同一个纯函数层算 argv 与展示串)。
                        // 末行预留给 `c` 键位行,放不下时该块自行收口。
                        let room = inner_h(area).saturating_sub(1).saturating_sub(v.len());
                        v.extend(recipe_new_suggest_lines(p, app, r, inner_w(area), room));
                        v
                    })
                    .unwrap_or_else(|| bake_empty_lines(p, app, inner_w(area))),
                Domain::Components => row(app.rc_sel)
                    .and_then(|i| l.components.get(i))
                    .map(|c| {
                        vec![
                            Line::from(format!("名称     {}", c.name)),
                            Line::from(format!(
                                "版本     {}",
                                c.version.clone().unwrap_or_else(|| "——".into())
                            )),
                            Line::from(format!(
                                "路径     {}",
                                c.path.clone().unwrap_or_else(|| "——".into())
                            )),
                            Line::from(format!(
                                "依赖     {}",
                                if c.depends.is_empty() {
                                    "无依赖".to_string()
                                } else {
                                    c.depends
                                        .iter()
                                        .map(|d| d.label())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                }
                            )),
                            Line::from(format!(
                                "被谁用   {}",
                                if c.used_by.is_empty() {
                                    "无".to_string()
                                } else {
                                    c.used_by.join(", ")
                                }
                            )),
                            // 引擎自己的字段名 + 原文布尔值:不另译一个词
                            Line::from(format!("vendored {}", c.vendored)),
                            Line::from(""),
                            Line::from("来源     ——  (本地/外部)"),
                            Line::from(Span::styled("nw ls 不输出,面板不猜。", p.dim())),
                        ]
                    })
                    .unwrap_or_default(),
                Domain::Locks => row(app.rc_sel)
                    .and_then(|i| l.locks.get(i))
                    .map(|k| {
                        vec![
                            Line::from(format!(
                                "lockId   {}",
                                k.lock_id.clone().unwrap_or_else(|| "——".into())
                            )),
                            Line::from(format!(
                                "配方     {}",
                                k.recipe
                                    .as_ref()
                                    .map(|r| r.label())
                                    .unwrap_or_else(|| "——".into())
                            )),
                            Line::from(format!(
                                "烤于     {}",
                                k.baked_at.clone().unwrap_or_else(|| "——".into())
                            )),
                            Line::from(format!(
                                "资产数   {}",
                                k.assets
                                    .map(|n| n.to_string())
                                    .unwrap_or_else(|| "——".into())
                            )),
                            // 路径与外部条目来源(repo/commit/落地位置)在锁定集
                            // 正文里,nw ls 只读锁目录、不读锁正文 —— 显式「——」
                            // + 一句出处:面板不猜,也不为它们另开引擎调用。
                            Line::from(format!(
                                "路径     {}",
                                k.path.clone().unwrap_or_else(|| "——".into())
                            )),
                            Line::from("外部来源 ——"),
                            Line::from(Span::styled("  (repo/commit/落地位置)", p.dim())),
                            Line::from(""),
                            Line::from(Span::styled("nw ls 不输出路径/外部来源;", p.dim())),
                            Line::from(Span::styled("面板不猜,也不另开引擎调用。", p.dim())),
                        ]
                    })
                    .unwrap_or_default(),
                Domain::Profiles => row(app.rc_sel)
                    .and_then(|i| l.profiles.get(i))
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
                // 外部源域(#37):选中一条记录 = 它的类别/链接/条目清单;
                // 编目为空时给指向登记的指路,不摆空框。
                Domain::Sources => match row(app.rc_sel).and_then(|i| l.sources.get(i)) {
                    Some(s) => source_detail(p, s),
                    None => vec![
                        Line::from(Span::styled(
                            "外部源域为空:数据仓的编目里还没有登记记录。",
                            p.dim(),
                        )),
                        Line::from(""),
                        register_entry_hint(app),
                    ],
                },
                // 五域(#37 #38)已全部有详情臂;将来上游加新域时这里会变成
                // 编译错误,提醒面板跟上引擎——不留 catch-all 把新域静默成一句占位。
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

// ---- #115 反向溯源:选中行的派落点(只读知识呈现) ----

/// 选中行「铺进派后落哪」的快照:打开浮层时从当次 ls 五域数据算定,之后
/// 不随刷新漂(呈现的是操作者选中那一刻的事实)。
///
/// 两类分列的来源(上游 docs/external-sources.md,只读知识、面板不另造规则):
/// - 本地组件住 `nw/components/<组件名>/`(「机制不同,家就不同」,2026-10-03 裁定);
/// - 外部源一条记录 = 一个仓库,条目按记录 kind 分派:skill 落
///   `skills/<仓库名>/<条目名>/`(仓库名即命名空间),extension 落
///   `nw/extensions/<仓库名>/<条目名>/`(与本地组件同一屋檐但不进 components/)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LandingTrace {
    /// 选中一个本地组件(ls 的 components 域:上游 `ls.go` 的 readComponents
    /// 只扫 `<root>/components/<名>/component.yaml`,这一域恒为本地组件)。
    Local { name: String },
    /// 选中一条外部源记录(ls 的 sources 域:一条记录 = 一个仓库 + 一次扫描的
    /// 条目清单)。`kind` 取引擎原文(skill/extension;缺 None 时呈现侧明说)。
    External {
        repo: String,
        kind: Option<String>,
        entries: Vec<String>,
    },
}

/// 本地组件的派落点(相对派目录):上游 external-sources.md「本地组件的 skill
/// 仍住 `nw/components/<组件>/` 里」。
pub fn local_component_landing(name: &str) -> String {
    format!("nw/components/{name}/")
}

/// 外部源条目的派落点(相对派目录):kind = `extension` 落
/// `nw/extensions/<仓库名>/<条目名>/`;其余(含 kind 缺省)按 skill 落
/// `skills/<仓库名>/<条目名>/` —— 上游编目两条数组必居其一(skills[] /
/// extensions[]),kind 缺省时呈现侧在标题明说,不静默冒充。
pub fn external_entry_landing(repo: &str, entry: &str, kind: Option<&str>) -> String {
    match kind {
        Some("extension") => format!("nw/extensions/{repo}/{entry}/"),
        _ => format!("skills/{repo}/{entry}/"),
    }
}

/// 当前选中行的落点快照(纯计算):只读,不碰文件系统、不发引擎命令。
/// 门与五域同源:数据仓根不就绪(`data_root_cause`)时不摆;无落点语义的域
/// (recipes/profiles/locks)不给;选中行取过滤后「看得见的那一行」(#61 同源)。
pub fn landing_trace(app: &App) -> Option<LandingTrace> {
    if app.data_root_cause().is_some() {
        return None;
    }
    let l = app.ls.as_ref()?;
    let i = *app.rc_visible().get(app.rc_sel)?;
    match app.rc_domain {
        Domain::Components => l.components.get(i).map(|c| LandingTrace::Local {
            name: c.name.clone(),
        }),
        Domain::Sources => l.sources.get(i).map(|s| LandingTrace::External {
            repo: s.name.clone(),
            kind: s.kind.clone(),
            entries: s.entries.iter().map(|e| e.name.clone()).collect(),
        }),
        _ => None,
    }
}

/// 浮层宽:要容得下 `nw/extensions/<仓库名>/<条目名>/` 的实指行(常畅/紧凑
/// 两档都放得下;`popup_rect` 再按终端收窄)。
const LANDING_W: u16 = 64;

/// 外部源条目的实指行上限:一仓扫出几十条时浮层不无限长,超出明说条数
///(与健康屏证据面板 20 条上限同一种「不收进无限列表」的纪律)。
const LANDING_MAX_ENTRIES: usize = 6;

/// 落点溯源浮层矩形:宽 [`LANDING_W`],高随内容(内容行 + 2 边框,与
/// `vps_reverse_popup_rect` 同法,钳在终端内)。渲染与命中共用这一几何。
pub(crate) fn landing_popup_rect(area: Rect, trace: &LandingTrace) -> Option<Rect> {
    let content = match trace {
        // 标签 + 实指 + 规则标签 + 两条规则 + 收口
        LandingTrace::Local { .. } => 6,
        // 标签 + 条目(含溢出说明)+ 规则标签 + 规则行 + 收口
        LandingTrace::External { entries, .. } => {
            4 + entries.len().min(LANDING_MAX_ENTRIES)
                + usize::from(entries.len() > LANDING_MAX_ENTRIES)
        }
    };
    let h = u16::try_from(content).unwrap_or(u16::MAX).saturating_add(2);
    let popup = crate::layout::popup_rect(area, LANDING_W, h);
    (popup.width > 0 && popup.height > 0).then_some(popup)
}

/// 落点溯源浮层(#115):纯只读呈现 —— 不读派目录、不发引擎命令、不落文件;
/// 两类分列:选中行所属那类摊实指,另一类给上游规则作对照(规格故事 19:
/// 两类组件的落点差异一目了然)。Esc 或点任意处关闭(键盘路由见
/// `App::on_key` 的浮层守卫,鼠标见 `layout::recipes_hits` 的弹层分支)。
fn render_landing_overlay(frame: &mut Frame, area: Rect, app: &App) {
    let Some(trace) = app.rc_landing.as_ref() else {
        return;
    };
    let Some(popup) = landing_popup_rect(area, trace) else {
        return;
    };
    let p = &app.palette;
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    match trace {
        LandingTrace::Local { name } => {
            lines.push(Line::from(Span::styled(
                " 本地组件(选中) ",
                p.text().bold(),
            )));
            lines.push(Line::from(vec![
                Span::raw("   ▸ "),
                Span::styled(local_component_landing(name), p.info()),
            ]));
            lines.push(Line::from(Span::styled(
                " 外部源组件 · 规则(上游落点) ",
                p.dim().bold(),
            )));
            lines.push(Line::from(Span::styled(
                "   · skill      →  skills/<仓库名>/<条目名>/",
                p.dim(),
            )));
            lines.push(Line::from(Span::styled(
                "   · extension →  nw/extensions/<仓库名>/<条目名>/",
                p.dim(),
            )));
        }
        LandingTrace::External {
            repo,
            kind,
            entries,
        } => {
            lines.push(Line::from(Span::styled(
                " 外部源组件(选中) ",
                p.text().bold(),
            )));
            if entries.is_empty() {
                lines.push(Line::from(Span::styled(
                    "   该仓库还没有扫出条目(register 预审时才有)。",
                    p.dim(),
                )));
            }
            for e in entries.iter().take(LANDING_MAX_ENTRIES) {
                lines.push(Line::from(vec![
                    Span::raw("   ▸ "),
                    Span::styled(clip(e, 18), p.info()),
                    Span::raw("  →  "),
                    Span::styled(
                        clip(
                            &external_entry_landing(repo, e, kind.as_deref()),
                            inner_w.saturating_sub(26),
                        ),
                        p.info(),
                    ),
                ]));
            }
            let rest = entries.len().saturating_sub(LANDING_MAX_ENTRIES);
            if rest > 0 {
                lines.push(Line::from(Span::styled(
                    format!("   …其余 {rest} 条按同一规则落 skills/<仓库名>/<条目名>/"),
                    p.dim(),
                )));
            }
            lines.push(Line::from(Span::styled(
                " 本地组件 · 规则(上游落点) ",
                p.dim().bold(),
            )));
            lines.push(Line::from(Span::styled(
                "   · 铺到 nw/components/<组件名>/",
                p.dim(),
            )));
        }
    }
    lines.push(Line::from(Span::styled(" Esc 或点击任意处关闭", p.dim())));
    let budget = popup.height.saturating_sub(2) as usize;
    lines.truncate(budget);
    // 标题带 kind 原文;缺 kind 明说按 skill 呈现,不静默冒充
    let title = match trace {
        LandingTrace::Local { name } => format!(" 落点溯源 · {name} "),
        LandingTrace::External { repo, kind, .. } => match kind.as_deref() {
            Some(k) => format!(" 落点溯源 · {repo} · kind: {k} "),
            None => format!(" 落点溯源 · {repo} · kind 未给出(按 skill) "),
        },
    };
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_style(p.accent())
                .title(Span::styled(title, p.accent().bold())),
        ),
        popup,
    );
}

// ---- 登记两段式浮层(spec #25 / 工单 #30):输入 → kind → 预审回音 → 确认 ----

/// 按显示宽截断(不切宽字符,防 CJK 混排错位)。
pub(crate) fn clip(s: &str, w: usize) -> String {
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
pub(crate) fn wrap(s: &str, w: usize) -> Vec<String> {
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
pub(crate) fn push_evidence(
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
        RegState::PreScan => render_reg_prescan(frame, area, app),
        RegState::PreScanPick => render_reg_pick(frame, area, app),
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

/// ②·预扫在途(#114):只读拉目标仓条目清单(`gh api`),说清这段在干什么;
/// 鼠标惰性(命中侧同款,见 `layout::register_hits` 的 `PreScan` 臂)。
fn render_reg_prescan(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup =
        crate::layout::popup_rect(area, crate::layout::REG_INPUT_W, crate::layout::REG_INPUT_H);
    frame.render_widget(Clear, popup);
    let slug = app.reg_scan_slug.as_deref().unwrap_or("—");
    let lines = vec![
        Line::from(Span::styled(format!("  正在预扫 {slug} 的条目…"), p.text())),
        Line::from(Span::styled(
            "  只读 GitHub(gh api):不落文件、不调引擎。",
            p.dim(),
        )),
        Line::from(Span::styled("  Esc 可返航(回音将丢弃)。", p.dim())),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记外部源 · 预扫 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ③·预扫选单(#114):勾选要登记的条目。按 kind 分组列条,每条一个
/// 勾选框(默认全勾——菜单答「这仓有什么」,作者改「我要登记哪些」);
/// 键位行钉内层末行(与 `layout::register_hits` 同几何同文案)。
/// **本段只改面板侧勾选态**:不落文件、不下发任何命令;勾选结论随
/// 确认态呈现,登记仍按仓整体走引擎两段式。
fn render_reg_pick(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup =
        crate::layout::popup_rect(area, crate::layout::REG_PICK_W, crate::layout::REG_PICK_H);
    frame.render_widget(Clear, popup);
    let inner_h = popup.height.saturating_sub(2) as usize;
    let inner_w = popup.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    let slug = app.reg_scan_slug.as_deref().unwrap_or("—");
    let total = app.reg_scan_entries.len();
    let checked = app.reg_scan_checks.iter().filter(|c| **c).count();
    lines.push(Line::from(Span::styled(
        format!("  预扫 {slug}:{total} 条,已勾选 {checked}"),
        p.text(),
    )));
    lines.push(Line::from(Span::styled(
        "  空格 勾选/取消 · ↑/↓ 移选中(面板侧选单,引擎扫描仍为准)",
        p.dim(),
    )));
    lines.push(Line::from(""));
    // 分组列条(skill 组在前,与 scan::parse_tree 的确定序一致)
    for kind in [
        crate::scan::ScanKind::Skill,
        crate::scan::ScanKind::Extension,
    ] {
        let idxs: Vec<usize> = app
            .reg_scan_entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == kind)
            .map(|(i, _)| i)
            .collect();
        if idxs.is_empty() {
            continue;
        }
        lines.push(Line::from(Span::styled(
            format!("  {}({}):", kind.word(), idxs.len()),
            p.accent(),
        )));
        for i in idxs {
            let on = app.reg_scan_checks.get(i).copied().unwrap_or(false);
            let cur = i == app.reg_scan_sel;
            lines.push(Line::from(vec![
                Span::raw(if cur { "  ▸ " } else { "    " }),
                Span::styled(
                    if on { "[x] " } else { "[ ] " },
                    if on { p.info() } else { p.dim() },
                ),
                Span::styled(
                    clip(&app.reg_scan_entries[i].path, inner_w.saturating_sub(8)),
                    if cur { p.sel() } else { p.text() },
                ),
            ]));
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
    lines.push(Line::from(vec![
        Span::styled(crate::layout::REG_KEY_PICK_CONTINUE, p.accent().bold()),
        Span::raw(crate::layout::RITUAL_KEY_GAP),
        Span::styled(crate::layout::RITUAL_KEY_RETURN, p.dim()),
    ]));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记外部源 · 预扫选单 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ③ kind 二选一:两行选项(选中标记 + 高亮)钉在内层第 1/2 行,
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
            // #114 预扫勾选(面板侧选单结论)随确认态摊开:登记按仓整体
            // 进行、引擎扫描为准,这里只是把作者在选单上勾的结论给人看。
            if !app.reg_scan_entries.is_empty() {
                let picked: Vec<&str> = app
                    .reg_scan_entries
                    .iter()
                    .zip(&app.reg_scan_checks)
                    .filter(|(_, on)| **on)
                    .map(|(e, _)| e.path.as_str())
                    .collect();
                lines.push(Line::from(Span::styled(
                    format!(
                        "  预扫勾选(面板侧) {}/{}:{}",
                        picked.len(),
                        app.reg_scan_entries.len(),
                        if picked.is_empty() {
                            " 未勾选".to_string()
                        } else {
                            format!(" {}", clip(&picked.join(" · "), inner_w.saturating_sub(24)))
                        }
                    ),
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
                &format!("  预审结果不可定型:{headline}"),
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

// ---- 组件打包器浮层(spec #95 工单 #112):表单 → 拟落账 → 确认 → 落盘 ----

/// 表单的三个字段(顺序即 ↑/↓ 轮转顺序)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PackField {
    #[default]
    Source,
    Name,
    Version,
}

impl PackField {
    pub const ALL: [PackField; 3] = [PackField::Source, PackField::Name, PackField::Version];

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Self {
        Self::ALL.get(i).copied().unwrap_or_default()
    }

    /// 字段标签(渲染与命中共用同一份词)。
    pub fn label(self) -> &'static str {
        match self {
            PackField::Source => "源目录",
            PackField::Name => "组件名",
            PackField::Version => "版  本",
        }
    }

    /// 一句字段说明(表单里的 dim 行)。
    pub fn hint(self) -> &'static str {
        match self {
            PackField::Source => "数据仓根之下的相对目录(要打包的那个)",
            PackField::Name => "落进清单 name;只用字母/数字/-/_/.",
            PackField::Version => "形如 1.0.0;本地组件的 version 必填",
        }
    }
}

/// 打包流程三态:表单 → 确认闸(校验和 + 拟生成清单摊开)→ 落盘结果。
/// 与外部源登记的 [`RegState`] 同级、不同族:那条走引擎两段式,这条走
/// 数据仓写侧(ADR-0010 §4 表格里 #112 那一行)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PackPhase {
    Form,
    Review,
    Done,
}

/// 打包浮层的全部状态(屏幕层持意图级字段 + 拟落账,零路径拼接)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackFlow {
    pub phase: PackPhase,
    pub field: PackField,
    pub source: String,
    pub name: String,
    pub version: String,
    /// 拟落账:确认闸摊开的三样(校验和 / 拟生成清单 / 将写文件)都从它读。
    pub plan: Option<crate::datarepo::pack::PackPlan>,
    /// 面板侧就拒的人话(留在表单可改,不喂给写侧跑一趟)。
    pub err: Option<String>,
    /// 落盘结论(Done 态呈现)。
    pub receipt: Option<String>,
}

impl PackFlow {
    pub fn new() -> Self {
        Self {
            phase: PackPhase::Form,
            field: PackField::Source,
            source: String::new(),
            name: String::new(),
            version: String::new(),
            plan: None,
            err: None,
            receipt: None,
        }
    }

    /// 表单 → 打包草稿(意图级;路径与校验和都在接缝里算)。
    pub fn draft(&self) -> crate::datarepo::pack::PackDraft {
        crate::datarepo::pack::PackDraft {
            source: self.source.trim().to_string(),
            name: self.name.trim().to_string(),
            version: self.version.trim().to_string(),
        }
    }

    /// 当前字段的缓冲(↑/↓ 移字段,键入归当前字段)。
    pub fn buf_mut(&mut self) -> &mut String {
        match self.field {
            PackField::Source => &mut self.source,
            PackField::Name => &mut self.name,
            PackField::Version => &mut self.version,
        }
    }

    /// ↑/↓ 移字段(三字段轮转,到顶/到底不越界、不绕回)。
    pub fn move_field(&mut self, down: bool) {
        let i = self.field.index();
        let n = PackField::ALL.len();
        self.field = PackField::ALL[if down {
            (i + 1).min(n - 1)
        } else {
            i.saturating_sub(1)
        }];
    }
}

impl Default for PackFlow {
    fn default() -> Self {
        Self::new()
    }
}

/// 打包浮层在不在场(渲染 / 键位守卫 / 命中弹层优先级共用这一句)。
pub fn pack_open(app: &App) -> bool {
    app.pk_flow.is_some()
}

/// 表单浮层尺寸(渲染与命中同一组常量)。
pub const PACK_FORM_W: u16 = 64;
pub const PACK_FORM_H: u16 = 10;
/// 确认闸浮层尺寸:**两档各一份**——常畅摊得开整份清单,紧凑只预览前几行
/// (ADR-0004:同一内容两档呈现,不收窄成同一副面孔)。
pub const PACK_REVIEW_W: u16 = 76;
pub const PACK_REVIEW_H_COMFY: u16 = 22;
pub const PACK_REVIEW_H_COMPACT: u16 = 17;
pub const PACK_MANIFEST_LINES_COMFY: usize = 12;
pub const PACK_MANIFEST_LINES_COMPACT: usize = 6;
/// 结果浮层尺寸。
pub const PACK_DONE_W: u16 = 64;
pub const PACK_DONE_H: u16 = 9;

/// 确认闸键位行左段(渲染与命中共用,与 [`crate::layout::RITUAL_KEY_RETURN`] 同法)。
pub const PACK_KEY_COMMIT: &str = "  Enter 落盘 ";
/// 表单键位行(算校验和 + 出拟落账,仍不落一个字节)。表单段鼠标惰性
/// (键入归键盘),故没有命中区——文案仍是单一真相源,别在渲染里另抄一份。
pub const PACK_KEY_FORM: &str = "  ↑/↓ 移字段 · Enter 算校验和 · Esc/q 返航";

/// 打包浮层矩形:按态取尺寸,`popup_rect` 再按终端收窄(渲染与命中共用)。
///
/// **尺寸档由调用方给**(渲染侧传 `size_class(frame.area())`、命中侧传
/// `recipes_hits` 的 `class`):本屏的常畅/紧凑判据看**整帧**宽(与
/// `render_recipes` 同源),不是看内容列宽——80 宽下去掉左导航还有 68 列,
/// 按内容列判会把常畅屏上的确认闸画成紧凑尺寸。
pub(crate) fn pack_popup_rect(
    area: ratatui::layout::Rect,
    app: &App,
    class: crate::layout::SizeClass,
) -> Option<Rect> {
    let f = app.pk_flow.as_ref()?;
    let (w, h) = match f.phase {
        PackPhase::Form => (PACK_FORM_W, PACK_FORM_H),
        PackPhase::Review => (
            PACK_REVIEW_W,
            match class {
                crate::layout::SizeClass::Comfy => PACK_REVIEW_H_COMFY,
                crate::layout::SizeClass::Compact => PACK_REVIEW_H_COMPACT,
            },
        ),
        PackPhase::Done => (PACK_DONE_W, PACK_DONE_H),
    };
    let popup = crate::layout::popup_rect(area, w, h);
    (popup.width > 0 && popup.height > 0).then_some(popup)
}

/// 确认闸的内容行(渲染与命中共用:键位行恒为末行,两边同一条算式)。
fn pack_review_lines(app: &App, inner_w: usize, manifest_cap: usize) -> Vec<Line<'static>> {
    let p = &app.palette;
    let plan = match app.pk_flow.as_ref().and_then(|f| f.plan.as_ref()) {
        Some(pl) => pl,
        None => return Vec::new(),
    };
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(vec![
        Span::raw("  源目录  "),
        Span::styled(plan.source.clone(), p.text()),
    ]));
    // 校验和:64 hex 一律**两行各 32** —— 不按终端宽折行(那会让同一份
    // 摘要在两档里断成不同的两截),也不截断(截半等于没显示)。10 + 32 = 42
    // 列,66 宽下限内也放得下。
    lines.push(Line::from(vec![
        Span::raw("  校验和  "),
        Span::styled(plan.digest[..32].to_string(), p.info().bold()),
    ]));
    lines.push(Line::from(Span::styled(
        format!("          {}", &plan.digest[32..]),
        p.info().bold(),
    )));
    lines.push(Line::from(vec![
        Span::raw("  体量    "),
        Span::styled(
            format!("{} 个文件 · {} 字节", plan.files, plan.bytes),
            p.dim(),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  将写    "),
        Span::styled(clip(&plan.target, inner_w.saturating_sub(10)), p.text()),
    ]));
    lines.push(Line::from(Span::styled(
        "  拟生成清单(字节确定,与数据仓现有组件样本同键序):",
        p.dim().bold(),
    )));
    let manifest_lines: Vec<&str> = plan.manifest.lines().collect();
    for l in manifest_lines.iter().take(manifest_cap) {
        lines.push(Line::from(Span::styled(
            format!("    {}", clip(l, inner_w.saturating_sub(4))),
            p.text(),
        )));
    }
    let rest = manifest_lines.len().saturating_sub(manifest_cap);
    if rest > 0 {
        lines.push(Line::from(Span::styled(
            format!("    …其余 {rest} 行见拟落账(确认闸已全量拿到,落盘逐字节相同)"),
            p.dim(),
        )));
    }
    lines
}

/// 打包浮层渲染:三态一份(表单 / 确认闸 / 结果),尺寸档在
/// [`pack_popup_rect`] 里分(常畅摊全清单,紧凑只预览前几行)。
fn render_pack_overlay(frame: &mut Frame, area: Rect, app: &App) {
    let Some(flow) = app.pk_flow.as_ref() else {
        return;
    };
    let Some(popup) = pack_popup_rect(area, app, size_class(frame.area())) else {
        return;
    };
    let p = &app.palette;
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    // 键位行恒为浮层内层末行(渲染与命中同一条算式:`key_y = popup.y +
    // popup.height - 2`),正文行放不下就让位给它——键位提示不许被挤掉。
    let content_h = popup.height.saturating_sub(2) as usize;
    let key_line = match flow.phase {
        PackPhase::Form => PACK_KEY_FORM,
        PackPhase::Review => "  Enter 落盘(原子写 tmp+rename)· Esc 返航(不落一个字节)",
        PackPhase::Done => "  Esc/q/Enter 返航 ",
    };
    let mut lines: Vec<Line<'static>> = Vec::new();
    match flow.phase {
        PackPhase::Form => {
            for f in PackField::ALL {
                let cur = f == flow.field;
                let text = match f {
                    PackField::Source => flow.source.clone(),
                    PackField::Name => flow.name.clone(),
                    PackField::Version => flow.version.clone(),
                };
                lines.push(Line::from(vec![
                    Span::raw(format!("  {}  ", f.label())),
                    Span::styled(
                        if cur { format!("{text}█") } else { text },
                        if cur { p.sel().bold() } else { p.text() },
                    ),
                ]));
            }
            lines.push(Line::from(Span::styled(
                format!("  {}", PackField::Source.hint()),
                p.dim(),
            )));
            if let Some(err) = flow.err.as_ref() {
                lines.push(Line::from(Span::styled(
                    format!("  ⚠ {}", clip(err, inner_w.saturating_sub(4))),
                    p.fail(),
                )));
            }
        }
        PackPhase::Review => {
            let cap = match size_class(frame.area()) {
                crate::layout::SizeClass::Comfy => PACK_MANIFEST_LINES_COMFY,
                crate::layout::SizeClass::Compact => PACK_MANIFEST_LINES_COMPACT,
            };
            lines = pack_review_lines(app, inner_w, cap);
        }
        PackPhase::Done => {
            lines.push(Line::from(Span::styled(
                format!(
                    "  {}",
                    flow.receipt.clone().unwrap_or_else(|| "已落盘".into())
                ),
                p.info().bold(),
            )));
            if let Some(plan) = flow.plan.as_ref() {
                lines.push(Line::from(vec![
                    Span::raw("  将写    "),
                    Span::styled(clip(&plan.target, inner_w.saturating_sub(10)), p.dim()),
                ]));
                lines.push(Line::from(vec![
                    Span::raw("  校验和  "),
                    Span::styled(clip(&plan.digest, inner_w.saturating_sub(10)), p.dim()),
                ]));
            }
            lines.push(Line::from(Span::styled(
                "  ⚠ component.yaml 是一次 git 变更:走 PR + 家长审,",
                p.warn(),
            )));
            lines.push(Line::from(Span::styled(
                "    面板只负责把确认过的字节写对地方(ADR-0010 §6)。",
                p.warn(),
            )));
        }
    }
    lines.truncate(content_h.saturating_sub(1));
    while lines.len() < content_h.saturating_sub(1) {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled(key_line, p.dim())));
    let title = match flow.phase {
        PackPhase::Form => " 组件打包器 · 表单 ",
        PackPhase::Review => " 组件打包器 · 确认闸 ",
        PackPhase::Done => " 组件打包器 · 落盘结果 ",
    };
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
