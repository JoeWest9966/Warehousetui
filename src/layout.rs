//! 纯「布局模型」：尺寸档判定 + 鼠标命中区清单（spec #15 / ADR-0004、ADR-0005）。
//!
//! 无 I/O、无终端依赖、不需要真实终端即可单测。渲染与鼠标命中测试
//! 共用同一份几何：命中区由本模块纯函数产出，按与 [`crate::ui`] /
//! [`crate::screens`] 相同的排布约定独立算出——两者以同一套排布常量
//! 对齐（常畅态 `80×30` 快照是回归基线，排布一改即回归）。
//!
//! 常畅态：导航 / 各屏列表行 / 发射仪式 / 帮助浮层；紧凑态外壳（横向
//! 屏 Tab）由 ticket #17 落地并与渲染同源；命中分派见 `App::on_mouse`
//! （ticket #20），滚轮由 #21，紧凑态内层栏（七步 / 五域）随 #18 跟随。

use ratatui::layout::Rect;

use crate::app::{App, Screen};

/// 常畅态/紧凑态互换的宽度断点:单一常量,不散值。
/// 固定 80 = 原 `MIN_W`——旧常态一字不变,变窄才换装(ADR-0004)。
/// 不写成 `= MIN_W`:下限已降到 66,断点是独立决策(换装阈值 vs 开机下限)。
pub const BREAKPOINT: u16 = 80;

/// 竖左导航轨定宽(渲染侧同值,见 `ui::render_body`)。
pub const NAV_WIDTH: u16 = 12;

/// 命中区的目标:点到某矩形后要做的一件事(类型形状钉死在 spec #15)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// 点导航 / 屏 Tab 切屏。
    NavTo(Screen),
    /// 点列表第 `n` 行选中(只选中,不触发破坏性动作)。
    SelectRow(usize),
    /// 点确认按钮(发射仪式「发射」)。
    Activate,
    /// 滚轮在装配 stderr 日志区滚动(#21 接线)。
    ScrollLog(ScrollDir),
    /// 点帮助浮层任意处关闭。
    DismissPopup,
}

/// 滚动方向(类型形状钉死在 spec #15)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScrollDir {
    Up,
    Down,
}

/// 呈现态:宽 ≥ [`BREAKPOINT`] 为常畅态,否则紧凑态(仍不低于 66×50 下限)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SizeClass {
    Comfy,
    Compact,
}

/// 尺寸档判定：`width < BREAKPOINT` ⇒ 紧凑态，否则常畅态。
pub fn size_class(area: Rect) -> SizeClass {
    if area.width < BREAKPOINT {
        SizeClass::Compact
    } else {
        SizeClass::Comfy
    }
}

/// 紧凑态屏 Tab 的单元文本：标记 + 图标 + 标题 + 尾随分隔空格。
///
/// [`crate::ui::render_screen_tabs`]（渲染）与 [`compact_shell`]（命中）
/// 都以这一个字符串为准——单元宽 = 它的显示宽度，几何永不分歧。
pub fn tab_cell(screen: Screen, selected: bool) -> String {
    let marker = if selected { "▸" } else { " " };
    format!("{marker} {} {} ", screen.icon(), screen.title())
}

/// 命中区清单：渲染与鼠标命中测试共用的同一份几何。
///
/// `area` 是当前终端面（即壳 `ui::render` 收到的帧区域）。
/// 只给**当前屏**的列表行做命中区（点其他屏的空白处不命中）；
/// 命中区一律不越出 `area`。
///
/// 外壳几何按 `size_class` 分叉：常畅态竖左导航轨，紧凑态一行横向
/// 屏 Tab（#17 落地）；正文在紧凑态全宽（Tab 条之下）。紧凑态各屏
/// 内层栏（七步 / 五域，#18）收起后，对应 `*_hits` 在本函数内跟随——
/// 判据仍只读 `size_class`，调用方（#20）不硬编码矩形。
pub fn hitmap(area: Rect, app: &App, class: SizeClass) -> Vec<(Rect, Target)> {
    // 守卫屏同款门控：装不下主界面时面板只画「终端太小」，无可点区域
    // （与 `ui::render` 的 `fits` 判定同源；低于 66 宽不做新排布）。
    if !app.fits(area.width, area.height) {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let Some(body) = area_pad(area, 1) else {
        return hits;
    };
    // 浮层优先级对鼠标同样成立：弹层打开时只有弹层自己的命中区，
    // 底层导航/列表不产出（点弹层外不作用底层）。
    if app.help {
        help_hits(body, &mut hits);
        return hits;
    }
    if app.as_confirm {
        ritual_hits(body, class, &mut hits);
        return hits;
    }
    // 手填浮层(NW_HOME / 数据仓根)对鼠标惰性:打字仍走键盘;浮层打开
    // 期间鼠标不分派任何命中区——点击只作用于浮层,不误触底层
    // (spec #15 故事 14/16;数据仓根手填沿用同一条)。
    if app.home_input || app.data_input {
        return hits;
    }
    // 登记两段式(#30):输入/等回音态同手填浮层惰性;kind 选择与
    // 确认态只产浮层命中区(弹层优先级,底层不产出)。
    if app.reg_state != crate::app::RegState::Idle {
        register_hits(body, class, app, &mut hits);
        return hits;
    }
    match class {
        SizeClass::Comfy => {
            comfy_shell(body, &mut hits);
            if let Some((_nav, inner)) = body_cols(body) {
                screen_hits(inner, app, class, &mut hits);
            }
        }
        SizeClass::Compact => {
            compact_shell(body, app.screen, &mut hits);
            if let Some(inner) = body_below_tabs(body) {
                screen_hits(inner, app, class, &mut hits);
            }
        }
    }
    hits
}

/// 顶部/底部各留一行(顶栏/HUD),先于导航分栏,与壳渲染同序。
fn area_pad(area: Rect, rows: u16) -> Option<Rect> {
    if area.height <= rows * 2 {
        return None;
    }
    let body = Rect::new(area.x, area.y + rows, area.width, area.height - rows * 2);
    (body.height >= 3).then_some(body)
}

/// 常畅态外壳:顶栏 1 行 / 左导航轨 [`NAV_WIDTH`] 列 / HUD 1 行。
/// `body` 已去掉顶栏与 HUD(由 [`hitmap`] 切好);这里只铺导航命中区。
fn comfy_shell(body: Rect, hits: &mut Vec<(Rect, Target)>) {
    let nav_w = NAV_WIDTH.min(body.width);
    for (i, screen) in Screen::ALL.iter().enumerate() {
        // 导航块自带边框:首项在 body.y+2(边框占 body.y+1),放不下就截断(同渲染)。
        if i as u16 + 2 > body.height.saturating_sub(1) {
            break;
        }
        hits.push((
            Rect::new(body.x, body.y + 1 + i as u16, nav_w, 1),
            Target::NavTo(*screen),
        ));
    }
}

/// 紧凑态外壳：竖左导航折成 `body` 首行的横向屏 Tab（顺序同
/// [`Screen::ALL`]，当前屏高亮）。单元矩形按与渲染同一个
/// [`tab_cell`] 字符串量宽——渲染与命中同源，点哪打哪（ADR-0005）。
fn compact_shell(body: Rect, current: Screen, hits: &mut Vec<(Rect, Target)>) {
    use unicode_width::UnicodeWidthStr;
    let end = body.x.saturating_add(body.width);
    let mut x = body.x;
    for screen in Screen::ALL {
        if x >= end {
            break;
        }
        let cell = tab_cell(screen, screen == current);
        let w = (UnicodeWidthStr::width(cell.as_str()) as u16).min(end - x);
        if w == 0 {
            break;
        }
        hits.push((Rect::new(x, body.y, w, 1), Target::NavTo(screen)));
        x += w;
    }
}

/// 紧凑态正文区：Tab 条占 `body` 首行，其余全宽归正文
/// （省下的导航列还给内容，spec #15 用户故事 4）。
fn body_below_tabs(body: Rect) -> Option<Rect> {
    (body.height > 1).then(|| Rect::new(body.x, body.y + 1, body.width, body.height - 1))
}

/// 常畅态分出正文区：左侧导航轨 [`NAV_WIDTH`] 列，余下为屏幕内容。
fn body_cols(body: Rect) -> Option<(Rect, Rect)> {
    let nav_w = NAV_WIDTH.min(body.width);
    if body.width <= nav_w + 1 {
        return None;
    }
    Some((
        Rect::new(body.x, body.y, nav_w, body.height),
        Rect::new(body.x + nav_w, body.y, body.width - nav_w, body.height),
    ))
}

/// 当前屏正文内的命中区（列表行 / 浮层），`content` 是正文区（常畅态
/// 已去掉左导航轨、紧凑态已去掉 Tab 条），几何与各 `render_*` 同约定：
/// 先缩 `body()` 屏体边框，再叠各屏自己的分栏与部件边框。
fn screen_hits(content: Rect, app: &App, class: SizeClass, hits: &mut Vec<(Rect, Target)>) {
    // `screens::render_*` 第一步都是 `body(app, note)` 边框，先缩一圈。
    let inner = shrink_border(content);
    match app.screen {
        Screen::Overview => overview_hits(inner, app, hits),
        Screen::Assemble => assemble_hits(inner, app, class, hits),
        Screen::Health => health_hits(inner, app, hits),
        Screen::Recipes => recipes_hits(inner, app, class, hits),
        Screen::Settings => {}
    }
}

/// 与 `render_*` 同款的二分分栏:直接用同一个 ratatui `Layout`,几何零分歧
/// (cassowary 的取整由渲染与命中共用,点哪打哪)。
fn h_split2(area: Rect, a: u16, b: u16) -> (Rect, Rect) {
    let cols = ratatui::layout::Layout::horizontal([
        ratatui::layout::Constraint::Percentage(a),
        ratatui::layout::Constraint::Percentage(b),
    ])
    .split(area);
    (cols[0], cols[1])
}

/// 概览:警示带 1 行 + 主区(68/32)+ 计数卡 4 行;派表在左栏内、带边框,
/// 表头 1 行后即数据行(与 `render_profiles` 同序,同一个 ratatui Layout)。
fn overview_hits(area: Rect, app: &App, hits: &mut Vec<(Rect, Target)>) {
    let rows = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(1),
        ratatui::layout::Constraint::Min(5),
        ratatui::layout::Constraint::Length(4),
    ])
    .split(area);
    let (cols, _cmd) = h_split2(rows[1], 68, 32);
    let tbl = shrink_border(cols);
    let Some(ls) = app.ls.as_ref() else {
        return;
    };
    row_hits(tbl, 1, ls.profiles.len(), hits, Target::SelectRow);
}

/// 装配屏:装配运行中给 stderr 日志区产滚轮命中区(#21,几何与
/// `render_running` 同款分栏);闲置时选锁定集列表在 55% 左栏、
/// 带边框、首行即数据;结果态/无日志区,不产目标。
fn assemble_hits(area: Rect, app: &App, class: SizeClass, hits: &mut Vec<(Rect, Target)>) {
    if app.assembling {
        // 与 render_running_comfy/compact 同一个 Layout:七步栏/进度行
        // 之外就是日志块,再缩一圈块边框(与 row_hits 的取内层同约定)。
        let log = match class {
            SizeClass::Comfy => ratatui::layout::Layout::horizontal([
                ratatui::layout::Constraint::Length(14),
                ratatui::layout::Constraint::Min(20),
            ])
            .split(area)[1],
            SizeClass::Compact => ratatui::layout::Layout::vertical([
                ratatui::layout::Constraint::Length(1),
                ratatui::layout::Constraint::Min(3),
            ])
            .split(area)[1],
        };
        let log = shrink_border(log);
        if log.width > 0 && log.height > 0 {
            // 方向由滚轮事件在分派时携带;命中区是纯几何,此处的值
            // 仅为满足 Target 形状(spec #15),不参与语义。
            hits.push((log, Target::ScrollLog(ScrollDir::Down)));
        }
        return;
    }
    if app.as_result.is_some() {
        return;
    }
    let (cols, _) = h_split2(area, 55, 45);
    let lst = shrink_border(cols);
    let Some(ls) = app.ls.as_ref() else {
        return;
    };
    row_hits(lst, 0, ls.locks.len(), hits, Target::SelectRow);
}

/// 健康屏:validate 模式下「27 项」列表在 55% 左栏、带边框、首行即数据
/// (doctor 模式是只读七面卡,无选中目标)。
fn health_hits(area: Rect, app: &App, hits: &mut Vec<(Rect, Target)>) {
    if !matches!(app.hl_mode, crate::app::HealthMode::Validate) {
        return;
    }
    let (cols, _) = h_split2(area, 55, 45);
    let lst = shrink_border(cols);
    let n = app
        .hl_result
        .as_ref()
        .map(|r| r.evidence().len())
        .unwrap_or(0);
    row_hits(lst, 0, n, hits, Target::SelectRow);
}

/// 配方屏:列表列几何与 `render_recipes` 同档——常畅态在 14 列五域轨右
/// (45% 栏),紧凑态在 Tab 行之下(52% 栏)。两支都用与渲染同一套
/// ratatui `Layout` 常量,几何零分歧(#18 收起内层栏后命中自动跟随)。
fn recipes_hits(area: Rect, app: &App, class: SizeClass, hits: &mut Vec<(Rect, Target)>) {
    let list = match class {
        SizeClass::Comfy => ratatui::layout::Layout::horizontal([
            ratatui::layout::Constraint::Length(14),
            ratatui::layout::Constraint::Percentage(45),
            ratatui::layout::Constraint::Percentage(41),
        ])
        .split(area)[1],
        SizeClass::Compact => {
            let rows = ratatui::layout::Layout::vertical([
                ratatui::layout::Constraint::Length(1),
                ratatui::layout::Constraint::Min(4),
            ])
            .split(area);
            ratatui::layout::Layout::horizontal([
                ratatui::layout::Constraint::Percentage(52),
                ratatui::layout::Constraint::Percentage(48),
            ])
            .split(rows[1])[0]
        }
    };
    row_hits(
        shrink_border(list),
        0,
        app.domain_len(),
        hits,
        Target::SelectRow,
    );
}

/// 发射仪式键位行的三段文案:渲染与命中共用(与 [`tab_cell`] 同法,
/// 点哪打哪)——`assemble::render_launch_ritual` 用同一组常量铺行,
/// [`ritual_hits`] 按同一组量宽分半。
pub const RITUAL_KEY_LAUNCH: &str = "  Enter 发射 ";
pub const RITUAL_KEY_GAP: &str = "   ";
pub const RITUAL_KEY_RETURN: &str = "Esc 返航 ";

/// 登记 kind 选择屏键位行的左段(与 RITUAL_KEY_* 同法,渲染与命中
/// 共用):右段直接复用 [`RITUAL_KEY_RETURN`],段间仍用 [`RITUAL_KEY_GAP`]。
pub const REG_KEY_KIND_LAUNCH: &str = "  Enter 预审 ";

/// 登记三个浮层的尺寸(spec #25 / 工单 #30):渲染与命中同一组常量,
/// 几何零分歧;`popup_rect` 的收窄规则两边同用。
pub const REG_INPUT_W: u16 = 64;
pub const REG_INPUT_H: u16 = 7;
pub const REG_KIND_W: u16 = 56;
pub const REG_KIND_H: u16 = 9;
pub const REG_REVIEW_W: u16 = 76;
pub const REG_REVIEW_H: u16 = 24;

/// 发射仪式浮层:与 `render_launch_ritual` 同尺寸、在**正文区**内居中;
/// 底部键位行按渲染同一文案对半分——「Enter 发射」段给
/// [`Target::Activate`](点=入队 up),「Esc 返航」段给
/// [`Target::DismissPopup`](点=取消并留日志);段与段之间
/// 不重叠、不留歧义区。浮层外无任何命中区(浮层优先级对鼠标同样成立)。
///
/// 居中基准必须与渲染同源:`render_assemble` 的浮层画在 `render_body`
/// 分出的**正文列**里(常畅已去左导航轨、紧凑已去 Tab 行)。早先直接用
/// body 居中,常畅态恒差一个导航列宽(80 宽差 6 列——点渲染出来的「射」
/// 会落进返航段),紧凑态奇数高度还会差一行;两处偏差由列级测试钉住。
fn ritual_hits(body: Rect, class: SizeClass, hits: &mut Vec<(Rect, Target)>) {
    // 与 `render_body` 同款分出正文区,再在其内居中(布局模型只读 size_class)。
    let Some(content) = popup_content(body, class) else {
        return;
    };
    let popup = popup_rect(content, 60, 14);
    // 与渲染同一套行序:block 内 10 行(空/四字段/空/两行说明/空/键位行),
    // 键位行是第 9 行 ⇒ 绝对 y = popup.y + 1(边框) + 9;行在场需内高 ≥ 10。
    if popup.height < 12 {
        return;
    }
    let y = popup.y + 10;
    two_key_segments(popup, y, RITUAL_KEY_LAUNCH, RITUAL_KEY_RETURN, hits);
}

/// 正文区(常畅去左导航轨 / 紧凑去 Tab 条):浮层命中的居中基准,
/// 与 `render_body` 分给各屏的区域几何零分歧。
fn popup_content(body: Rect, class: SizeClass) -> Option<Rect> {
    match class {
        SizeClass::Comfy => body_cols(body).map(|(_, inner)| inner),
        SizeClass::Compact => body_below_tabs(body),
    }
}

/// 键位行两段命中:左段(确认动作)→ [`RITUAL_KEY_GAP`] → 右段(返航);
/// 渲染按同一组文案同一顺序铺行,段间不重叠、不留歧义区。
/// 发射仪式与登记 kind 选择屏共用此分法(点哪打哪)。
fn two_key_segments(popup: Rect, y: u16, left: &str, right: &str, hits: &mut Vec<(Rect, Target)>) {
    use unicode_width::UnicodeWidthStr;
    let inner_x = popup.x.saturating_add(1);
    let left_w = u16::try_from(UnicodeWidthStr::width(left)).unwrap_or(0);
    let gap_w = u16::try_from(UnicodeWidthStr::width(RITUAL_KEY_GAP)).unwrap_or(0);
    let right_w = u16::try_from(UnicodeWidthStr::width(right)).unwrap_or(0);
    let left_end = (inner_x.saturating_add(left_w)).min(popup.x + popup.width);
    let left_w = left_end.saturating_sub(inner_x);
    if left_w > 0 {
        hits.push((Rect::new(inner_x, y, left_w, 1), Target::Activate));
    }
    let right_x = left_end.saturating_add(gap_w);
    let right_end = (right_x.saturating_add(right_w)).min(popup.x + popup.width);
    if right_x < right_end {
        hits.push((
            Rect::new(right_x, y, right_end - right_x, 1),
            Target::DismissPopup,
        ));
    }
}

/// 登记浮层(#30):几何与 `recipes::render_register_overlay` 同一组
/// 常量、同一 `popup_rect` 居中基准。输入/等回音态鼠标惰性(同手填
/// 浮层先例);kind 选项两行产 [`Target::SelectRow`] 选中 + 键位行两段
/// (Activate 预审 / Dismiss 返航);确认态整屏点任意处 = 返航
/// (同帮助浮层先例)。浮层外不产出任何命中区(弹层优先级)。
fn register_hits(
    body: Rect,
    class: SizeClass,
    app: &crate::app::App,
    hits: &mut Vec<(Rect, Target)>,
) {
    let Some(content) = popup_content(body, class) else {
        return;
    };
    match app.reg_state {
        crate::app::RegState::Idle | crate::app::RegState::RepoInput => {}
        crate::app::RegState::PreAudit => {} // 等回音:无处可点,同惰性
        crate::app::RegState::KindPick => {
            let popup = popup_rect(content, REG_KIND_W, REG_KIND_H);
            if popup.height < REG_KIND_H {
                return; // 收窄后行序不再可信,不产命中
            }
            let inner = shrink_border(popup);
            // 与渲染同序:内层首行是标题行,其后是 kind 选项两行,
            // 末行是键位行(渲染把键位行钉在内层最后一行)。
            for i in 0..crate::app::RegKind::ALL.len() as u16 {
                let y = inner.y.saturating_add(1 + i);
                if y < inner.y + inner.height {
                    hits.push((
                        Rect::new(inner.x, y, inner.width, 1),
                        Target::SelectRow(i as usize),
                    ));
                }
            }
            let key_y = popup.y + popup.height - 2;
            two_key_segments(popup, key_y, REG_KEY_KIND_LAUNCH, RITUAL_KEY_RETURN, hits);
        }
        crate::app::RegState::Review => {
            let popup = popup_rect(content, REG_REVIEW_W, REG_REVIEW_H);
            if popup.width > 0 && popup.height > 0 {
                hits.push((popup, Target::DismissPopup)); // 点任意处 = 返航
            }
        }
    }
}

/// 帮助浮层:与 `render_help` 同尺寸居中,点任意处关闭(不区分内外)。
fn help_hits(area: Rect, hits: &mut Vec<(Rect, Target)>) {
    hits.push((popup_rect(area, 44, 18), Target::DismissPopup));
}

/// 与壳/浮层渲染同款的居中弹层矩形(`recipes` 登记浮层渲染直接调用,
/// 与命中侧同一个函数——居中与收窄规则永不分歧)。
pub(crate) fn popup_rect(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(4));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    )
}

/// `Block::bordered().inner(r)`:四周各缩 1 列(空间不足时返回空区)。
fn shrink_border(r: Rect) -> Rect {
    if r.width < 2 || r.height < 2 {
        return Rect::new(r.x, r.y, 0, 0);
    }
    Rect::new(r.x + 1, r.y + 1, r.width - 2, r.height - 2)
}

/// 在列表区内铺 `n` 个数据行(跳过前 `skip` 行,如表头)。
/// 列表宽度整块命中——点行即选中,不越出列表矩形。
fn row_hits(
    inner: Rect,
    skip: usize,
    n: usize,
    hits: &mut Vec<(Rect, Target)>,
    mk: fn(usize) -> Target,
) {
    let rows = match inner.height.checked_sub(skip as u16) {
        Some(r) => r,
        None => return,
    };
    let vis = (n as u16).min(rows);
    for i in 0..vis {
        hits.push((
            Rect::new(inner.x, inner.y + skip as u16 + i, inner.width, 1),
            mk(usize::from(i)),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::FakeRunner;
    use std::sync::Arc;

    fn area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    fn test_app() -> App {
        let mut app = App::with_runner(Arc::new(FakeRunner::new(&[(
            "ls",
            0,
            r#"{"schema":"nw/ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[{"name":"desk","state":"drift","lockId":"9f8e"},{"name":"lab","state":"ok","lockId":"a1b2"}],"locks":[{"lockId":"9f8e","recipe":"research-desk","bakedAt":"2026-10-01"}]}}"#,
        )])));
        // with_runner 不跑引擎;列表数据直接入状态树(与 app 单测同套路)。
        app.ls = Some(
            serde_json::from_str::<crate::engine::LsData>(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[{"name":"desk","state":"drift","lockId":"9f8e"},{"name":"lab","state":"ok","lockId":"a1b2"}],"locks":[{"lockId":"9f8e","recipe":"research-desk","bakedAt":"2026-10-01"}]}"#,
            )
            .unwrap(),
        );
        app.splash = false;
        app
    }

    // ---- size_class:断点单一常量,阈值四点 ----

    #[test]
    fn 断点与下限是两个独立常量() {
        // 断点=换装阈值(原 MIN_W 80),下限=还能不能开(66×50);
        // 两者语义不同,不许链成一个值(spec #15)。
        assert_eq!(BREAKPOINT, 80);
        assert_eq!(crate::term::MIN_W, 66);
        assert_eq!(crate::term::MIN_H, 24);
    }

    #[test]
    fn size_class_断点阈值() {
        for w in [0, 1, 65, 66, 67, 79] {
            assert_eq!(
                size_class(area(w, 50)),
                SizeClass::Compact,
                "宽 {w} < 断点 ⇒ 紧凑态"
            );
        }
        for w in [80, 81, 100, 200] {
            assert_eq!(
                size_class(area(w, 30)),
                SizeClass::Comfy,
                "宽 {w} ≥ 断点 ⇒ 常畅态"
            );
        }
    }

    // ---- 命中区:66×50(紧凑)与 80×30(常畅)两档 ----

    /// 所有命中区不越出 area、且都是非空矩形。
    fn check(hits: &[(Rect, Target)], area: Rect, label: &str) {
        for (r, _) in hits {
            assert!(
                r.x >= area.x
                    && r.y >= area.y
                    && r.x + r.width <= area.x + area.width
                    && r.y + r.height <= area.y + area.height,
                "{label}:命中区 {r:?} 越出 area {area:?}"
            );
            assert!(r.width > 0 && r.height > 0, "{label}:空命中区 {r:?}");
        }
    }

    fn first(hits: &[(Rect, Target)], pred: impl Fn(&Target) -> bool) -> Option<Rect> {
        hits.iter().find(|(_, t)| pred(t)).map(|(r, _)| *r)
    }

    #[test]
    fn hitmap_常畅态_80x30_导航五屏齐() {
        let app = test_app();
        let a = area(80, 30);
        let hits = hitmap(a, &app, size_class(a));
        check(&hits, a, "80×30");
        let nav = hits
            .iter()
            .filter(|(_, t)| matches!(t, Target::NavTo(_)))
            .count();
        assert_eq!(nav, 5, "导航五屏齐全");
        // 顺序同 Screen::ALL:顶栏占 y=0,导航块边框占 y=1,首项自 y=2
        for (i, screen) in Screen::ALL.iter().enumerate() {
            let r = first(&hits, |t| matches!(t, Target::NavTo(s) if s == screen)).unwrap();
            assert_eq!(r.y, 2 + i as u16, "导航行序同 Screen::ALL(边框之后)");
            assert_eq!(r.width, NAV_WIDTH, "导航命中区贴齐导航轨");
        }
    }

    #[test]
    fn hitmap_常畅态_点列表行得选中目标() {
        let mut app = test_app();
        let a = area(80, 30);
        app.screen = Screen::Overview;
        let hits = hitmap(a, &app, size_class(a));
        let rows: Vec<_> = hits
            .iter()
            .filter(|(_, t)| matches!(t, Target::SelectRow(_)))
            .collect();
        assert_eq!(rows.len(), 2, "概览两个派 = 两行命中区(按序)");
        assert_eq!(
            first(&hits, |t| matches!(t, Target::SelectRow(1))),
            Some(rows[1].0),
            "SelectRow(1) 指第二行"
        );
        // 派表首行:屏体边框 → 警示带 1 行 → 主区 → 派表边框 + 表头 1 行 ⇒ y=5
        assert_eq!(
            rows[0].0,
            Rect::new(14, 5, 43, 1),
            "派表首行与渲染同序(嵌套边框后数据自 y=5)"
        );

        app.screen = Screen::Assemble;
        let hits = hitmap(a, &app, size_class(a));
        assert_eq!(
            first(&hits, |t| matches!(t, Target::SelectRow(0))),
            Some(Rect::new(14, 3, 34, 1)),
            "选锁定集首行:屏体边框 → 55% 左栏 → 列表边框,首行即数据"
        );
    }

    #[test]
    fn hitmap_空白不得命中() {
        let mut app = test_app();
        let a = area(80, 30);
        app.screen = Screen::Overview;
        let hits = hitmap(a, &app, size_class(a));
        assert!(
            hits.iter().all(|(r, _)| r.y != 0),
            "顶栏无命中区:点顶栏不得切屏/选中"
        );
        // 设置屏无列表:除导航外无命中区
        app.screen = Screen::Settings;
        let hits = hitmap(a, &app, size_class(a));
        assert!(
            hits.iter().all(|(_, t)| matches!(t, Target::NavTo(_))),
            "设置屏只有导航命中区,点正文空白不命中"
        );
        // 无 ls 数据:正文无列表行(没有可选中的行)
        let mut bare = test_app();
        bare.ls = None;
        bare.screen = Screen::Overview;
        let hits = hitmap(a, &bare, size_class(a));
        assert!(
            hits.iter().all(|(_, t)| matches!(t, Target::NavTo(_))),
            "没有 ls 数据就没有列表行可命中"
        );
    }

    #[test]
    fn hitmap_紧凑态_66x50_也有导航与列表() {
        let mut app = test_app();
        let a = area(66, 50);
        let class = size_class(a);
        assert_eq!(class, SizeClass::Compact);
        let hits = hitmap(a, &app, class);
        check(&hits, a, "66×50");
        assert!(
            hits.iter().any(|(_, t)| matches!(t, Target::NavTo(_))),
            "紧凑态:导航命中区 = 横向屏 Tab(#17)"
        );
        app.screen = Screen::Overview;
        let hits = hitmap(a, &app, class);
        assert_eq!(
            first(&hits, |t| matches!(t, Target::SelectRow(0))),
            Some(Rect::new(2, 6, 42, 1)),
            "紧凑态 66 宽:Tab 条让出首行,导航列还给正文后——屏体边框 → 警示带 → 68% 分栏的整数落点"
        );
    }

    /// #17 落地后紧凑态外壳的真实几何:一行横向屏 Tab,
    /// 顺序同 Screen::ALL、单元宽与渲染同一个 `tab_cell`。
    #[test]
    fn hitmap_紧凑态_屏tab条几何与渲染同源() {
        let mut app = test_app();
        let a = area(66, 50);
        let hits = hitmap(a, &app, size_class(a));
        let nav: Vec<_> = hits
            .iter()
            .filter(|(_, t)| matches!(t, Target::NavTo(_)))
            .collect();
        assert_eq!(nav.len(), 5, "五屏 Tab 齐全");
        let mut x = 0u16;
        for (i, screen) in Screen::ALL.iter().enumerate() {
            let (r, t) = nav[i];
            assert_eq!(*t, Target::NavTo(*screen), "Tab 顺序同 Screen::ALL");
            assert_eq!(r.y, 1, "Tab 条在 body 首行(顶栏之下)");
            assert_eq!(r.x, x, "Tab 单元首尾相接不重叠");
            let cell = tab_cell(*screen, *screen == app.screen);
            assert_eq!(
                r.width,
                unicode_width::UnicodeWidthStr::width(cell.as_str()) as u16,
                "命中区宽 = 渲染同一个 tab_cell 的显示宽(点哪打哪)"
            );
            x += r.width;
        }
        assert!(x <= a.width, "Tab 条不溢出终端");
        // 切屏后选中标记跟着走:重算命中区,单元宽不因高亮改变
        app.screen = Screen::Recipes;
        let hits2 = hitmap(a, &app, size_class(a));
        let x_after: u16 = hits2
            .iter()
            .filter(|(_, t)| matches!(t, Target::NavTo(_)))
            .map(|(r, _)| r.width)
            .sum();
        assert_eq!(x, x_after, "j/k 切屏不改变 Tab 条几何");
    }

    #[test]
    fn hitmap_弹层优先级_浮层外无底层命中区() {
        let mut app = test_app();
        let a = area(80, 30);
        // 发射仪式:只有键位行两段(发射/返航),导航/列表全部让位
        app.as_confirm = true;
        app.screen = Screen::Assemble;
        let hits = hitmap(a, &app, size_class(a));
        assert!(
            hits.iter().any(|(_, t)| *t == Target::Activate),
            "仪式「Enter 发射」行可点"
        );
        assert!(
            hits.iter()
                .all(|(_, t)| matches!(t, Target::Activate | Target::DismissPopup)),
            "浮层打开:只有仪式两段(发射/返航),底层点击不命中(#20 返航亦可点)"
        );
        // 帮助浮层:任意处关闭
        app.as_confirm = false;
        app.help = true;
        let hits = hitmap(a, &app, size_class(a));
        assert_eq!(
            first(&hits, |t| *t == Target::DismissPopup),
            Some(Rect::new(18, 6, 44, 18)),
            "帮助浮层与渲染同尺寸居中,点任意处关闭"
        );
        assert!(
            hits.iter().all(|(_, t)| matches!(t, Target::DismissPopup)),
            "帮助浮层吞掉所有底层点击"
        );
        // 手填 NW_HOME 浮层:对鼠标惰性,不产出任何命中区
        app.help = false;
        app.home_input = true;
        app.screen = Screen::Settings;
        let hits = hitmap(a, &app, size_class(a));
        assert!(
            hits.is_empty(),
            "手填浮层对鼠标惰性:零命中区(#20 不分派,故事 16)"
        );
    }

    #[test]
    fn hitmap_仪式键位行_发射返航分半_与渲染同一行() {
        let mut app = test_app();
        app.screen = Screen::Assemble;
        app.as_confirm = true;
        let a = area(80, 30);
        let hits = hitmap(a, &app, size_class(a));
        // popup 在正文列内居中(常畅去左导航轨):x=12+(68-60)/2=16,
        // y=1+(28-14)/2=8;键位行是 block 内第 9 行 ⇒ y=8+1(边框)+9=18;
        // 两段起止贴渲染文案(内层首列 17 起,宽 = RITUAL_KEY_* 同一串量宽)。
        // y 修正自 #16 期的 +12(错位两行);x 修正自评审 finding-1 的
        // 恒偏导航列宽(旧期望 11/27 → 17/33,点射落进返航段的错位)。
        let act = first(&hits, |t| *t == Target::Activate).expect("发射段可点");
        assert_eq!(
            act,
            Rect::new(17, 18, 13, 1),
            "发射段贴渲染「Enter 发射」文案"
        );
        let back = first(&hits, |t| *t == Target::DismissPopup).expect("返航段可点");
        assert_eq!(
            back,
            Rect::new(33, 18, 9, 1),
            "返航段贴渲染「Esc 返航」文案"
        );
        assert!(act.x + act.width < back.x, "两段不重叠,段间空隙不歧义");
        assert!(hits.iter().all(|(r, _)| r.y == 18), "仪式只产键位行");
        // 渲染同源:键位文案就在命中同一行(点哪打哪)
        let mut term =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(a.width, a.height)).unwrap();
        term.draw(|f| crate::ui::render(f, &app)).unwrap();
        let s = term.backend().to_string();
        let row = s.split('\n').nth(18).expect("第 18 行在场");
        assert!(
            row.contains("Enter 发射") && row.contains("Esc 返航"),
            "键位文案与命中同处 y=18"
        );
    }

    /// 仪式键位行的列级同源断言:渲染整段 RITUAL_KEY_LAUNCH 的起始列
    /// == 命中 Activate 矩形的 x,且渲染「射」字所在列落在 Activate 内
    /// (评审 finding-1 的 6 列错位正是行级 contains 断言漏掉的)。
    fn assert_ritual_column_aligned(app: &App, w: u16, h: u16) {
        use unicode_width::UnicodeWidthChar;
        let a = area(w, h);
        let hits = hitmap(a, app, size_class(a));
        let act = first(&hits, |t| *t == Target::Activate).expect("发射段可点");
        // 与 golden 同一把尺:整帧真显示串(testutil::frame_display),
        // to_string 的引号/隐藏格注记不能当裸行。
        let frame = crate::testutil::frame_display(app, w, h);
        let row = frame
            .split('\n')
            .nth(act.y as usize)
            .expect("命中行在渲染中在场");
        let idx = row
            .find(RITUAL_KEY_LAUNCH)
            .expect("渲染行铺着整段发射键位文案");
        let col: u16 = row[..idx]
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0) as u16)
            .sum();
        assert_eq!(
            col, act.x,
            "渲染整段 RITUAL_KEY_LAUNCH 起始列 {col} == 命中 Activate.x {}({w}×{h})",
            act.x
        );
        // 「点射即发射」:可见的「射」字列必须落在 Activate 矩形内
        // (修复前恒偏导航列宽 6 列,「射」落进返航段)。
        let idx = row.find('射').expect("渲染含「射」字");
        let glyph: u16 = row[..idx]
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0) as u16)
            .sum();
        assert!(
            act.x <= glyph && glyph < act.x + act.width,
            "「射」列 {glyph} 落在 Activate {}..{} 内({w}×{h})",
            act.x,
            act.x + act.width
        );
    }

    #[test]
    fn hitmap_仪式列级同源_常畅80x30() {
        let mut app = test_app();
        app.screen = Screen::Assemble;
        app.as_confirm = true;
        assert_ritual_column_aligned(&app, 80, 30);
    }

    #[test]
    fn hitmap_仪式列级同源_紧凑66x50() {
        let mut app = test_app();
        app.screen = Screen::Assemble;
        app.as_confirm = true;
        assert_ritual_column_aligned(&app, 66, 50);
    }

    #[test]
    fn hitmap_仪式列级同源_紧凑66x31奇数高() {
        // 评审 finding-1 附带:奇数高度下旧代码居中在 body(未去 Tab 行),
        // 命中行比渲染行高 1;同根因,同一把尺钉住。
        let mut app = test_app();
        app.screen = Screen::Assemble;
        app.as_confirm = true;
        assert_ritual_column_aligned(&app, 66, 31);
    }

    #[test]
    fn hitmap_配方行随尺寸档分栏_66与80两态() {
        // #18:常畅态配方列表仍在 14 列竖轨右侧(45% 栏),紧凑态随 Tab 行
        // 收起后落在全宽 52% 栏——几何两支都由 render_recipes 同款
        // Layout 算出,命中与渲染同源(点哪打哪)。
        let mut app = test_app();
        app.screen = Screen::Recipes;
        app.rc_domain = crate::app::Domain::Profiles; // 测试数据里派有 2 条

        let a80 = area(80, 30);
        let hits = hitmap(a80, &app, size_class(a80));
        let comfy_first =
            first(&hits, |t| matches!(t, Target::SelectRow(0))).expect("常畅态配方行有命中");
        let a66 = area(66, 50);
        let hits = hitmap(a66, &app, size_class(a66));
        let compact_first =
            first(&hits, |t| matches!(t, Target::SelectRow(0))).expect("紧凑态配方行有命中");
        assert_eq!(
            compact_first,
            // #17 落地真实紧凑外壳后:竖导航列还给正文(x 14→2)、Tab 条让出
            // 首行(y 4→5);栏内 52% 分栏由 render_recipes 同款 Layout 算出。
            Rect::new(2, 5, 31, 1),
            "紧凑态:真实 Tab 行 + 全宽正文 → 列表在 52% 左栏,首行数据自 y=5"
        );
        assert_ne!(
            compact_first, comfy_first,
            "两档分栏不同:紧凑态不得沿用常畅态几何"
        );
        // 常畅态首行仍是竖轨右侧 45% 栏(既有几何,不回退)
        assert_eq!(
            comfy_first,
            Rect::new(28, 3, 24, 1),
            "常畅态:14 列竖轨 + 45% 栏,首行数据自 y=3"
        );
        // 两档所有行命中都落在各自列表区内、不越出 area
        for (r, _) in hits.iter() {
            assert!(r.x + r.width <= a66.x + a66.width, "命中区不越界");
        }
    }

    #[test]
    fn hitmap_过窄终端_不产出命中区() {
        let app = test_app();
        let a = area(30, 10); // 守卫屏状态:无导航、无列表
        let hits = hitmap(a, &app, size_class(a));
        assert!(hits.is_empty(), "正文不足时不铺命中区");
    }

    // ---- #21 stderr 日志滚动区:装配运行中才有,与 render_running 同款分栏 ----

    #[test]
    fn hitmap_stderr日志区_装配运行中产出_闲置不产() {
        let mut app = test_app();
        app.screen = Screen::Assemble;
        app.assembling = true;
        for (w, h) in [(80, 30), (66, 50)] {
            let a = area(w, h);
            let r = first(&hitmap(a, &app, size_class(a)), |t| {
                matches!(t, Target::ScrollLog(_))
            })
            .unwrap_or_else(|| panic!("{w}×{h} 装配运行中应有日志滚动区"));
            assert!(r.width > 0 && r.height > 0, "{w}×{h} 日志区非空");
            assert!(
                r.x + r.width <= a.x + a.width && r.y + r.height <= a.y + a.height,
                "日志滚动区不越出终端面"
            );
        }
        // 闲置选锁定集:无日志区,滚轮无处可滚(几何来自同一个 assemble_hits)
        app.assembling = false;
        let a = area(80, 30);
        assert!(
            !hitmap(a, &app, size_class(a))
                .iter()
                .any(|(_, t)| matches!(t, Target::ScrollLog(_))),
            "闲置装配屏不产日志滚动区"
        );
    }
}
