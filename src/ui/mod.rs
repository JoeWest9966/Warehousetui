//! UI 外壳:顶栏 / 左竖导航 / HUD / 帮助浮层 / 过小提示。
//!
//! 壳负责「驾驶舱感」:常驻键位栏、全局装配态灯、NW_HOME 一目了然;
//! 屏幕内容由 [`crate::screens`] 各自渲染。

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crossterm::event::KeyCode;

use crate::app::{App, Domain, HealthMode, Screen};
use crate::layout::{popup_rect, size_class, SizeClass, HELP_H, HELP_W, NAV_WIDTH};
use crate::screens;
use crate::theme::Palette;

/// 壳三段:顶栏 1 行 / 主区 / HUD 1 行。
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let p = &app.palette;

    if app.splash {
        render_splash(frame, area, app);
        return;
    }
    if !app.fits(area.width, area.height) {
        render_too_small(frame, area, app);
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .split(area);

    render_top_bar(frame, rows[0], app);
    render_body(frame, rows[1], app);
    render_hud(frame, rows[2], app);

    if app.help {
        // 传 body 区(去掉顶栏 1 行)而非整帧:与 `layout::help_hits` 的
        // `area_pad(area,1)` 同源,浮层可见区与「点任意处关」命中区永不分歧。
        render_help(frame, rows[1], app);
    }
    let _ = p;
}

fn render_top_bar(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let lamp = if app.assembling {
        "◉ 装配中"
    } else {
        "○ 空闲"
    };
    let lamp_style = if app.assembling { p.accent() } else { p.dim() };
    // 紧凑态精简:去副品牌与版本细节,只留装配态灯与 NW_HOME
    // (ADR-0004「顶栏去副品牌/版本细节」,spec #15 ticket #17)。
    let line = if size_class(area) == SizeClass::Compact {
        Line::from(vec![
            Span::raw("NW_HOME: "),
            Span::styled(app.nw_home.clone(), p.dim()),
            Span::raw("  "),
            Span::styled(lamp, lamp_style),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Warehousetui", p.accent().bold()),
            Span::styled(" · Nous Warehouse 驾驶舱 ", p.dim()),
            Span::raw("  nwt v0.3 · "),
            Span::styled(
                format!("nw {} · {}", app.engine_version, app.engine.source),
                p.info(),
            ),
            Span::raw("  │  NW_HOME: "),
            Span::styled(app.nw_home.clone(), p.dim()),
            Span::raw("  "),
            Span::styled(lamp, lamp_style),
        ])
    };
    frame.render_widget(Paragraph::new(line).style(p.surface()), area);
}

fn render_body(frame: &mut Frame, area: Rect, app: &App) {
    // 分叉只读 size_class,不新造判定(ADR-0004)。
    if size_class(area) == SizeClass::Compact {
        // 紧凑态:竖左导航折成一行横向屏 Tab,省下的 12 列还给正文。
        let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).split(area);
        render_screen_tabs(frame, rows[0], app);
        screens::render(frame, rows[1], app);
    } else {
        let cols =
            Layout::horizontal([Constraint::Length(NAV_WIDTH), Constraint::Min(10)]).split(area);
        render_nav(frame, cols[0], app);
        screens::render(frame, cols[1], app);
    }
}

/// 紧凑态横向屏 Tab 条:一排五屏,顺序同 [`Screen::ALL`],当前屏带选中标记。
/// 单元文本取自 [`crate::layout::tab_cell`](布局模型)——渲染与命中区同源。
fn render_screen_tabs(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let spans: Vec<Span> = Screen::ALL
        .iter()
        .map(|screen| {
            let selected = *screen == app.screen;
            let style = if selected { p.sel().bold() } else { p.dim() };
            Span::styled(crate::layout::tab_cell(*screen, selected), style)
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)).style(p.surface()), area);
}

fn render_nav(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let block = Block::bordered()
        .border_style(p.dim())
        .title(Span::styled(" 导航 ", p.dim()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    for (i, screen) in Screen::ALL.iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }
        let row = Rect::new(inner.x, inner.y + i as u16, inner.width, 1);
        let selected = *screen == app.screen;
        let style = if selected { p.sel().bold() } else { p.dim() };
        let marker = if selected { "▸" } else { " " };
        let line = Line::from(vec![
            Span::raw(marker),
            Span::raw(" "),
            Span::styled(screen.icon(), style),
            Span::raw(" "),
            Span::styled(screen.title(), style),
        ]);
        frame.render_widget(Paragraph::new(line), row);
    }
}

fn render_hud(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    // 紧凑态 HUD 精简:少给提示(去「t 主题」)、去右侧主题标注,
    // 切屏/确认/帮助/退出四个核心提示保留(ADR-0004,spec #15 ticket #17)。
    let compact = size_class(area) == SizeClass::Compact;
    let mut spans = vec![
        Span::styled(" j/k", p.accent()),
        Span::raw(" 切屏 "),
        Span::styled("Enter", p.accent()),
        Span::raw(" 确认 "),
    ];
    if !compact {
        spans.push(Span::styled("t", p.accent()));
        spans.push(Span::raw(" 主题 "));
    }
    spans.push(Span::styled("?", p.accent()));
    spans.push(Span::raw(" 帮助 "));
    if app.assembling {
        spans.push(Span::raw(" "));
        // 中止键只在真有 up 在途时给(#59):登记落账也点这盏灯,但它的
        // 返航语义归登记流自己,不在这里混成一个键。
        let hint = if app.as_running {
            "装配中:x 中止 · 其余动作已锁定"
        } else {
            "装配中:动作已锁定"
        };
        spans.push(Span::styled(hint, p.dim()));
    } else if app.home_input || app.data_input || app.rc_filter_on {
        spans.push(Span::raw(" "));
        spans.push(Span::styled("输入中:Enter 确认 · Esc/q 放弃", p.accent()));
    } else if app.reg_state == crate::app::RegState::RepoInput {
        spans.push(Span::raw(" "));
        spans.push(Span::styled("登记填链:Enter 继续 · Esc/q 放弃", p.accent()));
    } else if app.reg_state != crate::app::RegState::Idle {
        spans.push(Span::raw(" "));
        spans.push(Span::styled("登记中:Esc 返航", p.accent()));
    } else {
        spans.push(Span::styled("  q", p.accent()));
        spans.push(Span::raw(" 退出"));
    }
    if !compact {
        let right = Span::styled(format!("主题:{} ", app.theme.name()), p.dim());
        let width = area.width as usize;
        let left_len: usize = spans.iter().map(|s| s.content.len()).sum();
        let right_len = right.content.len();
        let pad = width.saturating_sub(left_len + right_len);
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(right);
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(p.surface()), area);
}

/// 帮助浮层的派发前预设:帮助列出的键有的要靠浮层在场才触发
/// (如 Esc 关浮层)。测试据此布场,再派发 `code` 验证「实现确有这条分支」。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Prep {
    /// 壳级:把 app.screen / rc_domain / hl_mode 布好就直接派发。
    Shell,
    /// 先打开帮助浮层再派发(验证 Esc/q 关浮层)。
    HelpOpen,
}

/// 帮助浮层的一行键位:渲染与「帮助↔实现不脱节」测试共用这一份
/// (`app::tests` 的对照测试遍历它),是帮助内容的单一真相源。
/// - `key` = 浮层展示的键名(可列别名,如 `a / h / l`);`desc` = 一句说明。
/// - `screen` / `domain` / `mode` = 测试派发前把 App 布到的上下文
///   (`None` = 壳级 / 默认域);`prep` = 额外浮层预设;`code` = 代表性键码。
/// - **增删一个键位必须同步改这里与 `app.rs` 的 handler**:正向测试守
///   「帮助写的键实现里都有」,反向清点守「实现有的键帮助都写了」。
pub(crate) struct HelpRow {
    pub(crate) key: &'static str,
    pub(crate) desc: &'static str,
    pub(crate) screen: Option<Screen>,
    pub(crate) domain: Option<Domain>,
    pub(crate) mode: Option<HealthMode>,
    /// 编目册屏当前翻开哪一本(#54:`m` 换册)。`None` = 不限定册子
    /// (壳级 / 机器册默认态)。帮助↔实现对照测试据此布场。
    pub(crate) book: Option<crate::screens::CatalogKind>,
    pub(crate) prep: Prep,
    pub(crate) code: KeyCode,
    /// 派发前先把 App 布成「有 up 在途」(#59 中止键的受理面前提)。
    pub(crate) running: bool,
}

/// 帮助浮层键位对照表(单一真相源):顺序即浮层里的自上而下。
/// 别名不逐列——`↑/↓` 之 `n/p`、`h/l` 之 `←/→`、`j/k` 之 `Tab/BackTab`
/// 与主键同义,帮助只呈现主键(与 README 键位表同粒度)。
pub(crate) const HELP_ROWS: &[HelpRow] = &[
    HelpRow {
        key: "j / k",
        desc: "切屏(下一屏 / 上一屏)",
        screen: None,
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('j'),
    },
    HelpRow {
        key: "↑ / ↓",
        desc: "列表上下移(各屏可选列表)",
        screen: None,
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Down,
    },
    HelpRow {
        key: "a / h / l / v",
        desc: "概览:跳 装配 / 健康 / 配方 / 编目",
        screen: Some(Screen::Overview),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('a'),
    },
    HelpRow {
        key: "r",
        desc: "概览:刷新 ls",
        screen: Some(Screen::Overview),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('r'),
    },
    HelpRow {
        key: "Enter",
        desc: "装配:打开发射仪式(再按发射)",
        screen: Some(Screen::Assemble),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Enter,
    },
    HelpRow {
        key: "x",
        desc: "装配:中止在途子进程(进程真止,日志不清)",
        screen: Some(Screen::Assemble),
        domain: None,
        mode: None,
        book: None,
        running: true,
        prep: Prep::Shell,
        code: KeyCode::Char('x'),
    },
    HelpRow {
        key: "m",
        desc: "健康:切 体检 / 校验 / 复核",
        screen: Some(Screen::Health),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('m'),
    },
    HelpRow {
        key: "r",
        desc: "健康:复核(重跑 / 全查外部源)",
        screen: Some(Screen::Health),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('r'),
    },
    HelpRow {
        key: "Enter",
        desc: "健康复核屏:只查选中外部源",
        screen: Some(Screen::Health),
        domain: None,
        mode: Some(HealthMode::Recheck),
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Enter,
    },
    HelpRow {
        key: "h / l",
        desc: "配方:切换五域",
        screen: Some(Screen::Recipes),
        domain: Some(Domain::Recipes),
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('h'),
    },
    HelpRow {
        key: "c",
        desc: "配方:取用烘焙建议命令(只读)",
        screen: Some(Screen::Recipes),
        domain: Some(Domain::Recipes),
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('c'),
    },
    HelpRow {
        key: "r",
        desc: "配方:登记外部源(外部源域)",
        screen: Some(Screen::Recipes),
        domain: Some(Domain::Sources),
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('r'),
    },
    HelpRow {
        key: "/",
        desc: "配方:过滤五域(本地筛选,键入即筛)",
        screen: Some(Screen::Recipes),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('/'),
    },
    HelpRow {
        key: "r",
        desc: "编目:登记一台机器(两段式,预审不落文件)",
        screen: Some(Screen::Vps),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('r'),
    },
    HelpRow {
        key: "d",
        desc: "编目:删选中的机器(d 段可点,先确认)",
        screen: Some(Screen::Vps),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('d'),
    },
    HelpRow {
        key: "l",
        desc: "编目:重读 vps 编目(只读)",
        screen: Some(Screen::Vps),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('l'),
    },
    HelpRow {
        key: "m",
        desc: "编目:换一册(机器 / 标签 / 订阅 / 钥匙)",
        screen: Some(Screen::Vps),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('m'),
    },
    HelpRow {
        key: "/",
        desc: "编目:册内过滤(本地筛选,键入即筛;Esc 清词)",
        screen: Some(Screen::Vps),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('/'),
    },
    HelpRow {
        key: "g",
        desc: "编目:取回链接(订阅册;g 段可点,只给建议)",
        screen: Some(Screen::Vps),
        domain: None,
        mode: None,
        book: Some(crate::screens::CatalogKind::Subscription),
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('g'),
    },
    HelpRow {
        key: "g",
        desc: "编目:取回钥匙(钥匙册;g 段可点,只给建议)",
        screen: Some(Screen::Vps),
        domain: None,
        mode: None,
        book: Some(crate::screens::CatalogKind::Key),
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('g'),
    },
    HelpRow {
        key: "Enter",
        desc: "编目:反查什么标签引用了选中的这台机器(只读)",
        screen: Some(Screen::Vps),
        domain: None,
        mode: None,
        book: Some(crate::screens::CatalogKind::Vps),
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Enter,
    },
    HelpRow {
        key: "Enter",
        desc: "编目:反查选中的这一条标签(只读)",
        screen: Some(Screen::Vps),
        domain: None,
        mode: None,
        book: Some(crate::screens::CatalogKind::Tag),
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Enter,
    },
    HelpRow {
        key: "n",
        desc: "设置:轮换 NW_HOME",
        screen: Some(Screen::Settings),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('n'),
    },
    HelpRow {
        key: "i",
        desc: "设置:手填 NW_HOME",
        screen: Some(Screen::Settings),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('i'),
    },
    HelpRow {
        key: "d",
        desc: "设置:手填数据仓根",
        screen: Some(Screen::Settings),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('d'),
    },
    HelpRow {
        key: "e",
        desc: "设置:拉起 $EDITOR",
        screen: Some(Screen::Settings),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('e'),
    },
    HelpRow {
        key: "m",
        desc: "设置:切换鼠标捕获(重启生效)",
        screen: Some(Screen::Settings),
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('m'),
    },
    HelpRow {
        key: "Esc",
        desc: "返回 / 关闭浮层",
        screen: None,
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::HelpOpen,
        code: KeyCode::Esc,
    },
    HelpRow {
        key: "t",
        desc: "切换主题(任意屏)",
        screen: None,
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('t'),
    },
    HelpRow {
        key: "?",
        desc: "本帮助",
        screen: None,
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('?'),
    },
    HelpRow {
        key: "q",
        desc: "退出(装配态 / 输入中禁用)",
        screen: None,
        domain: None,
        mode: None,
        book: None,
        running: false,
        prep: Prep::Shell,
        code: KeyCode::Char('q'),
    },
];

/// 鼠标章节正文:逐条对应 README「鼠标(默认开捕获)」已写全部条目——
/// 点导航/Tab 切屏与点行选中、滚轮两用(移选中 / 翻装配 stderr 日志)、
/// 发射仪式两段可点、装配运行中「x 中止」段可点(#71,补 #59 欠账)、取回段
/// 可点(#73,补 #65 §6 延期的那处「屏上写着、鼠标点不到」)、删除段可点
/// (#78,兑 #75 裁决 A:点段 ≡ 按键,先过 ADR-0009 确认闸)、浮层吞底层
/// 点击、捕获默认开的 Shift 代价、以及 `--no-mouse` 与设置屏 `m` 两条逃逸。
/// 测试断言这些短语都在场。
pub(crate) const MOUSE_LINES: &[&str] = &[
    "点导航/Tab 切屏;点行 = 选中(同 ↑/↓)",
    "滚轮移选中;装配 stderr 日志可回看",
    "发射仪式「发射/返航」两段可点;浮层吞点击不穿透",
    "装配运行中「x 中止」段可点 = 按 x 进程级真止",
    "编目册键位行可点段:换册 / 过滤 / 反查(点它 = 按对应键)",
    "取回段只订阅/钥匙册:点它 = 按 g(只收建议命令,不代跑)",
    "删除段四本册可点 = 按 d(先确认;标签册是「d 摘/删」)",
    "捕获默认开的代价:复制要按 Shift",
    "逃逸:--no-mouse / 设置屏 m 关(重启生效)",
];

/// 帮助浮层正文(键位对照表 + 鼠标章节 + 尾注):渲染开窗与 ↑/↓ 滚动的
/// 钳位都按它算,一处真相。样式在此一次上齐,开窗只切行、不重上色。
fn help_lines(p: &crate::theme::Palette) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::with_capacity(HELP_ROWS.len() + MOUSE_LINES.len() + 3);
    for row in HELP_ROWS {
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(format!("{:<10}", row.key), p.accent()),
            Span::raw(row.desc),
        ]));
    }
    lines.push(Line::from(Span::styled("  鼠标(默认开捕获)", p.accent())));
    for m in MOUSE_LINES {
        lines.push(Line::from(Span::styled(format!("    {m}"), p.dim())));
    }
    lines.push(Line::from(Span::styled(
        "  装配幂等可重入,取消安全;失败永远带证据五字段。",
        p.dim(),
    )));
    // 两个取消语义不许混(#59):一个还没下发,一个进程已跑了一半。
    lines.push(Line::from(Span::styled(
        "  发射前 Esc 返航 = 还没下发,什么都没发生;",
        p.dim(),
    )));
    lines.push(Line::from(Span::styled(
        "  发射后 x 中止 = 子进程真止,可能停在未收敛中间态 ——",
        p.dim(),
    )));
    lines.push(Line::from(Span::styled(
        "  面板不宣布成功也不假装失败:重跑同一装配命令收敛,doctor 说了算。",
        p.dim(),
    )));
    lines
}

fn render_help(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    // 浮层矩形与命中区(`layout::help_hits`)同一个 `popup_rect` + 同尺寸常量,
    // 渲染与「点任意处关」永不分歧。
    let popup = popup_rect(area, HELP_W, HELP_H);
    let block = Block::bordered()
        .border_style(p.accent())
        .title(Span::styled(" 帮助 ", p.accent().bold()));
    // 正文开窗(#53):帮助自 #62 补齐键位与鼠标章后,行数已超过最小常畅态
    // (80×30 内层 24 行)放得下的量。超量时 ↑/↓ 滚动(见 `on_key` 的帮助分支),
    // 末行给「还有内容」的可见提示——**不静默截断**:截断等于把键位藏起来,
    // 而本浮层的全部意义就是「按 ? 看到的每一行都在实现里存在」。
    let all = help_lines(p);
    let inner_h = popup.height.saturating_sub(2) as usize;
    let view = inner_h.saturating_sub(1); // 末行留给滚动提示
    let max_off = all.len().saturating_sub(view);
    let off = app.help_offset.min(max_off);
    let mut lines: Vec<Line> = all.iter().skip(off).take(view).cloned().collect();
    if max_off > 0 {
        let hint = match (off == 0, off >= max_off) {
            (true, _) => "  ↓ 还有下文",
            (false, true) => "  ↑ 还有上文",
            (false, false) => "  ↑↓ 还有上下文",
        };
        lines.push(Line::from(Span::styled(hint, p.dim())));
    }
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block).style(p.surface()), popup);
}

fn render_too_small(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let msg = format!(
        "终端太小:需要至少 {}×{},当前 {}×{}。调大窗口。",
        crate::term::MIN_W,
        crate::term::MIN_H,
        area.width,
        area.height
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(msg, p.fail())).centered())
            .block(Block::bordered().border_style(p.fail())),
        area,
    );
}

/// 启动 splash:复古货架 logo,任意键或 800ms 后交棒。
fn render_splash(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let art = [
        "   ┌───────────────────────────────┐   ",
        "   │  ▄▄   ▄▄   ▄▄   ▄▄   ▄▄   ▄▄  │   ",
        "   │  ██   ██   ██   ██   ██   ██  │   ",
        "   │  ▀▀   ▀▀   ▀▀   ▀▀   ▀▀   ▀▀  │   ",
        "   └───────────────────────────────┘   ",
        "         N O U S   W A R E H O U S E   ",
    ];
    let mut lines: Vec<Line> = vec![Line::from("")];
    for (i, row) in art.iter().enumerate() {
        let style = if i == art.len() - 1 {
            p.accent().bold()
        } else {
            p.dim()
        };
        lines.push(Line::from(Span::styled(*row, style)));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "        Warehousetui · Nous Warehouse 驾驶舱",
        p.text(),
    )));
    lines.push(Line::from(Span::styled(
        "               nwt v0.3 · 按任意键进入",
        p.dim(),
    )));
    frame.render_widget(Paragraph::new(lines).centered(), area);
}

/// 数据屏共用的外框:标题 + 可选右上角注脚。
pub fn screen_block<'a>(p: &Palette, title: &'a str, note: Option<String>) -> Block<'a> {
    let mut block = Block::bordered()
        .border_style(p.dim())
        .title(Span::styled(format!(" {title} "), p.accent().bold()));
    if let Some(n) = note {
        block = block.title_top(Line::from(Span::raw(n)).right_aligned());
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render_to_string(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| render(f, app)).unwrap();
        term.backend().to_string()
    }

    /// 常畅态 80×30 整帧逐字节 golden(评审 finding-3):仓内既有「快照」
    /// 全是 contains 断言,只钉「在场」、不钉「没变」;这条把整帧显示内容
    /// 逐字节钉住——≥80 宽常畅态渲染一变就红(spec #15 的回归基线铁律)。
    /// 期望值从当前正确渲染生成并经人工核对(顶栏/导航/警示带/派表/指令栏/
    /// 最近日志/计数卡/HUD 逐行过目);若既有渲染面发生真实回归而改期望,
    /// 必须与该回归同一次提交并说明——不许为过测试回填。
    /// 场景全部钉死:with_runner(引擎状态固定「测试注入」)+ splash 关 +
    /// NW_HOME 置 /nw + ls 固定夹具——CI 三平台 HOME/配置不同也不漂。
    #[test]
    fn golden_常畅态_80x30_整帧逐字节() {
        use crate::engine::FakeRunner;
        use std::sync::Arc;
        let ls = r#"{"domain":"all","recipes":[{"name":"research-desk","version":"0.2.0"}],"components":[{"name":"retention","depends":[]}],"sources":[],"profiles":[{"name":"desk","state":"drift","lockId":"9f8e","installedAt":"2026-10-01"},{"name":"lab","state":"ok","lockId":"a1b2"}],"locks":[{"lockId":"9f8e","recipe":"research-desk","bakedAt":"2026-10-01"}]}"#;
        let mut app = App::with_runner(Arc::new(FakeRunner::new(&[("ls", 0, ls)])));
        app.splash = false;
        app.nw_home = "/nw".into();
        app.ls = Some(serde_json::from_str(ls).unwrap());
        let got = crate::testutil::frame_display(&app, 80, 30);
        let want = r#" Warehousetui · Nous Warehouse 驾驶舱   nwt v0.3 · nw 探测中 · 测试注入  │  NW_H
┌ 导航 ────┐┌ 概览 ──────────────────────────────────────────────────目标侧日常┐
│▸ ▸ 概览  ││ ⚠ 警示带 派 desk 需要关注(漂移/破损)                             │
│  ⚙ 装配  ││┌ 派 (profiles) ────────────────────────────┐┌ 指令 ─────────────┐│
│  ✚ 健康  │││   派         lockId     装配于      状态  ││ ▸ 跑一次体检   (h)││
│  ▤ 配方  │││✗  desk       9f8e       2026-10-01  漂移  ││   发起装配…   (a) ││
│  ▩ 编目  │││●  lab        a1b2       —           ok    ││   浏览配方     (l)││
│  ✦ 设置  │││                                           ││   刷新 ls      (r)││
│          │││                                           ││   浏览编目     (v)││
│          │││                                           │└───────────────────┘│
│          │││                                           │┌ 最近日志 ─────────┐│
│          │││                                           ││驾驶舱已就绪       ││
│          │││                                           ││                   ││
│          │││                                           ││                   ││
│          │││                                           ││                   ││
│          │││                                           ││                   ││
│          │││                                           ││                   ││
│          │││                                           ││                   ││
│          │││                                           ││                   ││
│          │││                                           ││                   ││
│          │││                                           ││                   ││
│          │││                                           ││                   ││
│          │││                                           ││                   ││
│          ││└───────────────────────────────────────────┘└───────────────────┘│
│          ││┌───────────┐┌───────────┐┌────────────┐┌───────────┐┌───────────┐│
│          │││recipes 1  ││components ││sources 0   ││profiles 2 ││locks 1    ││
│          │││           ││           ││            ││           ││           ││
│          ││└───────────┘└───────────┘└────────────┘└───────────┘└───────────┘│
└──────────┘└──────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 t 主题 ? 帮助   q 退出       主题:暗色仓库                 "#;
        assert_eq!(
            got, want,
            "常畅态 80×30 整帧渲染发生变化(上=got 下=want;见 golden 测试头注的修订纪律)"
        );
    }

    /// 测试默认已经越过 splash(与真实启动流一致:先 splash 再主界面)。
    fn ui_app() -> App {
        let mut app = App::new();
        app.splash = false;
        app
    }

    #[test]
    fn splash_先亮相且可跳过() {
        let mut app = App::new();
        assert!(app.splash, "开机先 splash");
        let s = render_to_string(&app, 100, 30);
        assert!(s.contains("W A R E H O U S E"));
        assert!(s.contains("驾驶舱"));
        app.splash = false; // 任意键/800ms 后交棒
        assert!(!render_to_string(&app, 100, 30).contains("W A R E H O U S E"));
    }

    #[test]
    fn 壳三件套渲染() {
        let app = ui_app();
        let s = render_to_string(&app, 100, 30);
        assert!(s.contains("Warehousetui"));
        assert!(s.contains("NW_HOME"));
        assert!(s.contains("概览"));
    }

    #[test]
    fn 装配态_hud_提示锁定() {
        let mut app = ui_app();
        app.assembling = true;
        let s = render_to_string(&app, 100, 30);
        assert!(s.contains("装配中"));
    }

    #[test]
    fn 装配态_hud_真有up在途才给中止键提示() {
        // 中止键只在 up 在途时给(#59):登记落账也点这盏灯,但它的返航语义
        // 归登记流自己,HUD 不在这里混成一个键。
        let mut app = ui_app();
        app.assembling = true;
        app.as_running = true;
        let s = render_to_string(&app, 100, 30);
        assert!(s.contains("x 中止"), "在途要给中止键提示:{s}");
        app.as_running = false;
        let s2 = render_to_string(&app, 100, 30);
        assert!(!s2.contains("x 中止"), "非 up 在途不给中止键提示:{s2}");
    }

    #[test]
    fn 终端过小有提示() {
        let app = ui_app();
        let s = render_to_string(&app, 40, 10);
        assert!(s.contains("终端太小"));
    }

    /// 新下限:66×50 仍能进驾驶舱;65 宽守卫文案只提「调大窗口」
    /// (不再建议「减小字体」,那是破坏 CJK 混排的错误自救,spec #15)。
    #[test]
    fn 新下限_66宽进入而65宽守卫() {
        use crate::term::{MIN_H, MIN_W};
        assert_eq!((MIN_W, MIN_H), (66, 24), "下限 66×24 不回退");
        let app = ui_app();
        let ok = render_to_string(&app, 66, 50);
        assert!(!ok.contains("终端太小"), "66×50 是保证最小,不得弹守卫屏");
        assert!(ok.contains("概览"), "66×50 下主界面照常渲染");
        let s = render_to_string(&app, 65, 50);
        assert!(s.contains("终端太小"), "65 宽仍弹守卫");
        assert!(s.contains("调大窗口"), "文案提示调大窗口");
        assert!(!s.contains("减小字体"), "铁律:不再建议减小字体");
    }

    #[test]
    fn 输入态_hud_给键位提示() {
        let mut app = ui_app();
        app.home_input = true;
        let s = render_to_string(&app, 100, 30);
        assert!(s.contains("输入中"));
        assert!(s.contains("Enter 确认"));
    }

    /// 数据仓根手填与 NW_HOME 手填同一条 HUD 提示(票 #27)。
    #[test]
    fn 数据仓根输入态_hud_给键位提示() {
        let mut app = ui_app();
        app.data_input = true;
        let s = render_to_string(&app, 100, 30);
        assert!(s.contains("输入中"), "数据仓根手填共用输入态提示");
        assert!(s.contains("Enter 确认"));
    }

    #[test]
    fn 帮助浮层可开关() {
        let mut app = ui_app();
        app.help = true;
        let s = render_to_string(&app, 100, 30);
        assert!(s.contains("帮助"));
    }

    /// 紧凑态外壳快照(66×50 保证最小):横向屏 Tab 与正文不重叠不溢出,
    /// 顶栏/HUD 精简但保留核心提示(spec #15 ticket #17,ADR-0004)。
    #[test]
    fn 紧凑态外壳_屏tab条_不重叠不溢出() {
        let mut app = ui_app();
        app.nw_home = "/root/.nous-warehouse".into(); // 钉住路径,顶栏宽度确定
        app.screen = Screen::Health; // 换一屏验证高亮不依赖当前屏
        let s = render_to_string(&app, 66, 50);
        let lines: Vec<&str> = s.lines().collect();
        assert!(lines.len() >= 3, "顶栏 + Tab 条 + 正文至少三行");

        // 行 0 顶栏:精简后只留装配态灯与 NW_HOME,无副品牌/版本细节
        let top = lines[0];
        assert!(top.contains("NW_HOME"), "NW_HOME 保留");
        assert!(top.contains("○ 空闲"), "装配态灯保留");
        assert!(!top.contains("驾驶舱"), "副品牌去掉了");
        assert!(!top.contains("nwt v0.3"), "版本细节去掉了");

        // 行 1:横向屏 Tab,五屏齐全、顺序同 Screen::ALL、当前屏带选中标记
        let tabs = lines[1];
        let mut last = 0usize;
        for screen in Screen::ALL {
            let pos = tabs
                .find(screen.title())
                .unwrap_or_else(|| panic!("Tab 条缺屏 {}", screen.title()));
            assert!(pos > last, "Tab 顺序同 Screen::ALL");
            last = pos;
        }
        assert!(tabs.contains("▸ ✚ 健康"), "当前屏(健康)在 Tab 条带选中标记");
        // buffer_view 把每行包成 `"…"` 并可能附 `Hidden by…` 调试注记,
        // 只量行内容本身的显示宽(截掉引号与注记)。
        let row = tabs.strip_prefix('"').unwrap_or(tabs);
        let row = row.split('"').next().unwrap_or(row);
        let w = unicode_width::UnicodeWidthStr::width(row);
        assert!(w <= 66, "Tab 条不溢出 66 列:width={w} row={row:?}");

        // 正文从行 2 开始:与 Tab 条不重叠
        let body_start = &lines[2..];
        assert!(
            body_start.iter().any(|l| l.contains("健康")),
            "正文屏块在 Tab 条之下"
        );

        // HUD(最后一行):保留切屏/确认/帮助/退出,去掉主题提示与右侧主题标注
        let hud = *lines.last().unwrap();
        assert!(hud.contains("切屏") && hud.contains("确认"));
        assert!(hud.contains("帮助") && hud.contains("退出"));
        assert!(!hud.contains("主题"), "紧凑态 HUD 少给提示、无主题标注");
    }

    /// j/k 切屏与 Tab 指示同步(紧凑态):选中标记跟到当前屏。
    #[test]
    fn 紧凑态_jk切屏与屏tab指示同步() {
        use crate::app::Message;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = ui_app();
        let tabs_of = |app: &App| -> String {
            let s = render_to_string(app, 66, 50);
            s.lines().nth(1).unwrap().to_string()
        };
        assert!(tabs_of(&app).contains("▸ ▸ 概览"), "初始在概览");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
        )));
        let tabs = tabs_of(&app);
        assert!(tabs.contains("▸ ⚙ 装配"), "j 下一屏,标记跟到装配");
        assert!(!tabs.contains("▸ ▸ 概览"), "概览不再是当前");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::NONE,
        )));
        assert!(tabs_of(&app).contains("▸ ▸ 概览"), "k 回概览");
    }

    /// 五屏 × 紧凑三档压力:任何一屏在 66×50(保证最小)/ 66×24(MIN_H
    /// 下限)/ 79×24(断点前一列)下都渲染不炸、Tab 条都在(小终端下
    /// 五屏均可达,spec #15 用户故事 27)。
    #[test]
    fn 紧凑态_五屏渲染不炸_且屏tab条常在() {
        for (w, h) in [(66, 50), (66, 24), (79, 24)] {
            for screen in Screen::ALL {
                let mut app = ui_app();
                app.screen = screen;
                let s = render_to_string(&app, w, h);
                let tabs = s.lines().nth(1).unwrap();
                for t in Screen::ALL {
                    assert!(
                        tabs.contains(t.title()),
                        "{w}×{h} {} 屏的 Tab 条缺 {}",
                        screen.title(),
                        t.title()
                    );
                }
            }
        }
    }
}
