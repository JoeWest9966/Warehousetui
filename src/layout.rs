//! 纯「布局模型」:尺寸档判定 + 鼠标命中区清单(spec #15 / ADR-0004、ADR-0005)。
//!
//! 无 I/O、无终端依赖、不需要真实终端即可单测。渲染与鼠标命中测试
//! 共用同一份几何:命中区由本模块纯函数产出,按与 [`crate::ui`] /
//! [`crate::screens`] 相同的排布约定独立算出——两者以同一套排布常量
//! 对齐(常畅态 `80×30` 快照是回归基线,排布一改即回归)。
//!
//! 本片(ticket #16)只落骨骼:`size_class` 判定 + 常畅态真实命中区
//! (导航 / 各屏列表行 / 发射仪式 / 帮助浮层)+ 紧凑态占位;命中分派
//! 由 ticket #20 接线,滚轮由 #21。

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

/// 尺寸档判定:`width < BREAKPOINT` ⇒ 紧凑态,否则常畅态。
pub fn size_class(area: Rect) -> SizeClass {
    if area.width < BREAKPOINT {
        SizeClass::Compact
    } else {
        SizeClass::Comfy
    }
}

/// 命中区清单:渲染与鼠标命中测试共用的同一份几何。
///
/// `area` 是当前终端面(即壳 `ui::render` 收到的帧区域)。
/// 只给**当前屏**的列表行做命中区(点其他屏的空白处不命中);
/// 命中区一律不越出 `area`。
///
/// 紧凑态命中区几何是**占位**:此片只定骨骼,紧凑态排布(#17/#18 收起
/// 内层栏)落地后按同一排布约定在本函数内跟随——判据仍只读 `size_class`,
/// 调用方(#20)不硬编码矩形。
pub fn hitmap(area: Rect, app: &App, class: SizeClass) -> Vec<(Rect, Target)> {
    // 守卫屏同款门控:装不下主界面时面板只画「终端太小」,无可点区域
    // (与 `ui::render` 的 `fits` 判定同源;低于 66 宽不做新排布)。
    if !app.fits(area.width, area.height) {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let Some(body) = area_pad(area, 1) else {
        return hits;
    };
    // 浮层优先级对鼠标同样成立:弹层打开时只有弹层自己的命中区,
    // 底层导航/列表不产出(点弹层外不作用底层)。
    if app.help {
        help_hits(body, &mut hits);
        return hits;
    }
    if app.as_confirm {
        ritual_hits(body, &mut hits);
        return hits;
    }
    // 手填 NW_HOME 浮层对鼠标惰性:不给它专属命中区(打字仍走键盘)。
    match class {
        SizeClass::Comfy => comfy_shell(body, &mut hits),
        SizeClass::Compact => compact_shell(body, &mut hits),
    }
    screen_hits(body, app, &mut hits);
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

/// 紧凑态外壳占位:竖左导航轨折成横向屏 Tab 是 #17 的事,本片先按现行
/// 几何铺出导航命中区,保证 #20 接线时两档都有落点、几何后续同源跟随。
fn compact_shell(body: Rect, hits: &mut Vec<(Rect, Target)>) {
    comfy_shell(body, hits)
}

/// 分出正文区:左侧导航轨 [`NAV_WIDTH`] 列,余下为屏幕内容。
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

/// 当前屏正文内的命中区(列表行 / 浮层),几何与各 `render_*` 同约定:
/// 先缩 `body()` 屏体边框,再叠各屏自己的分栏与部件边框。
fn screen_hits(body: Rect, app: &App, hits: &mut Vec<(Rect, Target)>) {
    let Some((_nav, inner)) = body_cols(body) else {
        return;
    };
    // `screens::render_*` 第一步都是 `body(app, note)` 边框,先缩一圈。
    let inner = shrink_border(inner);
    match app.screen {
        Screen::Overview => overview_hits(inner, app, hits),
        Screen::Assemble => assemble_hits(inner, app, hits),
        Screen::Health => health_hits(inner, app, hits),
        Screen::Recipes => recipes_hits(inner, app, hits),
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

/// 装配屏:发射仪式浮层短路后,选锁定集列表在 55% 左栏、带边框、首行即数据。
fn assemble_hits(area: Rect, app: &App, hits: &mut Vec<(Rect, Target)>) {
    if app.assembling || app.as_result.is_some() {
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

/// 配方屏:五域轨(14 列)/ 列表(45%)/ 详情(41%)三栏同 `render_recipes`,
/// 列表在中间栏、带边框、首行即数据。
fn recipes_hits(area: Rect, app: &App, hits: &mut Vec<(Rect, Target)>) {
    let cols = ratatui::layout::Layout::horizontal([
        ratatui::layout::Constraint::Length(14),
        ratatui::layout::Constraint::Percentage(45),
        ratatui::layout::Constraint::Percentage(41),
    ])
    .split(area);
    let lst = shrink_border(cols[1]);
    row_hits(lst, 0, app.domain_len(), hits, Target::SelectRow);
}

/// 发射仪式浮层:与 `render_launch_ritual` 同尺寸居中;「Enter 发射」行给
/// 整宽命中区(Activate),浮层外无任何命中区(浮层优先级对鼠标同样成立)。
fn ritual_hits(area: Rect, hits: &mut Vec<(Rect, Target)>) {
    let popup = popup_rect(area, 60, 14);
    // 与渲染同一套行序:12 个内容行之后是「Enter 发射 / Esc 返航」行。
    if popup.height > 13 {
        hits.push((
            Rect::new(popup.x, popup.y + 12, popup.width, 1),
            Target::Activate,
        ));
    }
}

/// 帮助浮层:与 `render_help` 同尺寸居中,点任意处关闭(不区分内外)。
fn help_hits(area: Rect, hits: &mut Vec<(Rect, Target)>) {
    hits.push((popup_rect(area, 44, 18), Target::DismissPopup));
}

/// 与壳/浮层渲染同款的居中弹层矩形。
fn popup_rect(area: Rect, w: u16, h: u16) -> Rect {
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
            "紧凑态占位:导航命中区先铺好(#17 折成 Tab 后几何同源跟随)"
        );
        app.screen = Screen::Overview;
        let hits = hitmap(a, &app, class);
        assert_eq!(
            first(&hits, |t| matches!(t, Target::SelectRow(0))),
            Some(Rect::new(14, 5, 33, 1)),
            "紧凑态 66 宽:屏体边框 → 警示带 → 68% 分栏在窄屏的整数落点"
        );
    }

    #[test]
    fn hitmap_弹层优先级_浮层外无底层命中区() {
        let mut app = test_app();
        let a = area(80, 30);
        // 发射仪式:只有仪式内的 Activate,导航/列表全部让位
        app.as_confirm = true;
        app.screen = Screen::Assemble;
        let hits = hitmap(a, &app, size_class(a));
        assert!(
            hits.iter().any(|(_, t)| *t == Target::Activate),
            "仪式「Enter 发射」行可点"
        );
        assert!(
            hits.iter().all(|(_, t)| matches!(t, Target::Activate)),
            "浮层打开:底层点击不命中"
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
            hits.iter().all(|(_, t)| matches!(t, Target::NavTo(_))),
            "手填浮层对鼠标惰性(只剩导航)"
        );
    }

    #[test]
    fn hitmap_过窄终端_不产出命中区() {
        let app = test_app();
        let a = area(30, 10); // 守卫屏状态:无导航、无列表
        let hits = hitmap(a, &app, size_class(a));
        assert!(hits.is_empty(), "正文不足时不铺命中区");
    }
}
