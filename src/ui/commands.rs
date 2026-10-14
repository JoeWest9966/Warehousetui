//! 全局命令面板(#119):`Ctrl+P` 呼出的叠加浮层。
//!
//! 命令清单就是帮助浮层那一份 [`HELP_ROWS`]——**一处登记、两处消费**:
//! 帮助浮层按它渲染「哪个键干什么」,命令面板按它搜索并执行。这治的是
//! 「帮助写的键与实现真有的键互相漂移」那笔历史账(#62 起有两条对齐测试
//! 双向钉着,#119 把第二个消费者接上来,清单从此只有一个出处)。
//!
//! 三条设计红线,全部来自规格 #96「实现决策 / Testing Decisions」:
//! - **执行不新造通道**:回车执行 = 把选中那一条的 `code` 当一次真实按键
//!   重新喂进 [`App::on_key`](crate::app::App::on_key)(等于「搜到并按下
//!   那个键」)。装配态 / 浮层态的互斥由分派入口的既有闸自然兜住,面板自己
//!   不设第二套门。
//! - **模糊匹配不引依赖**:简单的大小写不敏感「包含 + 子序列」匹配,零新
//!   crate;匹配只做输入→输出向量断言,不测算法内部。
//! - **面板是叠加层不是模式劫持**:面板只收自己的键(查询键入 / ↑/↓ /
//!   Enter / Esc / Ctrl+P),其余键一律不拦、照旧路由到当前屏。

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crossterm::event::KeyCode;

use crate::app::{App, Screen};
use crate::layout::{cmd_inner, cmd_popup_rect, cmd_row_rects, cmd_visible, cmd_window, CMD_HEAD};
use crate::screens::recipes::clip;
use crate::theme::Palette;
use crate::ui::{HelpRow, HELP_ROWS};

/// 面板结果行的屏名列:`screen: None` 的行是壳级键位,给它一个看得见的名
/// (规格故事 6:搜索结果要带屏名与键位原文,顺带把键位学了)。
pub(crate) fn screen_label(s: Option<Screen>) -> &'static str {
    match s {
        Some(s) => s.title(),
        None => "全局",
    }
}

/// 一条命令参与匹配的文本:**屏名 + 键位 + 动作名**(规格「命令清单 = 各屏
/// 已注册键位动作的静态登记(屏名+键位+动作名)」照字面收口,不另挑字段)。
pub(crate) fn haystack(row: &HelpRow) -> String {
    format!("{} {} {}", screen_label(row.screen), row.key, row.desc)
}

/// 大小写不敏感的子序列匹配(查询的每个字符按序出现在文本里即可)。
fn is_subseq(needle: &str, hay: &str) -> bool {
    let mut rest = hay.chars();
    needle.chars().all(|c| rest.any(|h| h == c))
}

/// 一条命令是否命中查询词:空查询 = 全中;否则「包含」或「子序列」二选一。
pub(crate) fn matches(row: &HelpRow, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    let hay = haystack(row).to_lowercase();
    hay.contains(&q) || is_subseq(&q, &hay)
}

/// **当前面板该呈示的清单**:先剔掉装配态下的写动作(规格故事 9:不许从面板
/// 绕过装配互斥),再按查询词过滤。渲染 / 命中 / 执行三处共用这一个函数——
/// 「屏上画得出的那一行」与「回车执行得了的那一条」永不分歧。
pub(crate) fn offered(app: &App) -> Vec<&'static HelpRow> {
    HELP_ROWS
        .iter()
        .filter(|r| !(app.assembling && r.write))
        .filter(|r| matches(r, &app.cmd_query))
        .collect()
}

/// 装配态下被面板收起、但帮助浮层仍列着的写动作条数(浮层里给人一句说明,
/// 免得操作者以为面板漏了键)。
pub(crate) fn hidden_write_count(app: &App) -> usize {
    if !app.assembling {
        return 0;
    }
    HELP_ROWS.iter().filter(|r| r.write).count()
}

/// 面板浮层矩形(与命中区同一个 `popup_rect` + 同一组常量,见 `layout::cmd_*`)。
pub(crate) fn render(frame: &mut Frame, area: Rect, app: &App) {
    let p: &Palette = &app.palette;
    let popup = cmd_popup_rect(area);
    let inner = cmd_inner(area);
    let inner_w = inner.width as usize;
    let rows = offered(app);
    let n = rows.len();
    let sel = app.cmd_sel.min(n.saturating_sub(1));
    let visible = cmd_visible(area);
    let (first, count) = cmd_window(n, sel, visible);

    let mut lines: Vec<Line> = Vec::with_capacity(count + CMD_HEAD as usize + 2);
    // 查询行:块光标让「键入即筛」看得见(与手填浮层同款光标)。
    lines.push(Line::from(vec![
        Span::styled("› ", p.accent().bold()),
        Span::styled(format!("{}█", app.cmd_query), p.text()),
    ]));
    // 提示行:命中计数 + 三个键的后果;空态换成「没有匹配」+ 下一步。
    let hidden = hidden_write_count(app);
    let hint = if n == 0 {
        if app.cmd_query.trim().is_empty() {
            "没有可执行的命令".to_string()
        } else {
            format!("没有匹配「{}」的命令", app.cmd_query)
        }
    } else {
        let mut h = format!("{}/{} · Enter 执行 · ↑/↓ 选 · Esc 关", sel + 1, n);
        if hidden > 0 {
            h.push_str(&format!(" · 装配中收起 {hidden} 个写动作"));
        }
        h
    };
    lines.push(Line::from(Span::styled(clip(&hint, inner_w), p.dim())));
    // 结果行:键位原文 + 屏名 + 动作名(规格故事 6)。
    for i in 0..count {
        let row = &rows[first + i];
        let on = first + i == sel;
        let style = if on { p.sel().bold() } else { p.text() };
        let label = format!(
            "{} {:<8}{} · {}",
            if on { "▸" } else { " " },
            row.key,
            screen_label(row.screen),
            row.desc
        );
        lines.push(Line::from(Span::styled(clip(&label, inner_w), style)));
    }
    // 空态给下一步,不把「没匹配」画成一根光秃秃的输入行。
    if n == 0 {
        lines.push(Line::from(Span::styled(
            clip("  换个词试试 · Esc 零成本退出", inner_w),
            p.dim(),
        )));
    }

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 命令面板 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// 面板结果行的命中区(渲染与命中共用同一份几何:渲染画第几行,命中就指第几行)。
/// 点行 = 选中(与 ↑/↓ 同义),**不执行**——执行留给 Enter,免得误点就开火
/// (#78 同口径:后果最重的段也不让单击直接触发)。
pub(crate) fn row_targets(area: Rect, n: usize, sel: usize) -> Vec<(usize, ratatui::layout::Rect)> {
    let visible = cmd_visible(area);
    let (first, count) = cmd_window(n, sel, visible);
    cmd_row_rects(area, first, count)
        .into_iter()
        .map(|(rect, idx)| (idx, rect))
        .collect()
}

/// 面板自己收的键;返回 `false` = 这个键**不归面板**,调用方照旧往下分派
/// (规格故事 7:面板打开时其它按键照旧路由到当前屏)。
pub(crate) fn on_key(app: &mut App, k: crossterm::event::KeyEvent) -> bool {
    match k.code {
        KeyCode::Esc => {
            app.close_cmd_panel();
            true
        }
        KeyCode::Enter => {
            app.run_cmd_selected();
            true
        }
        KeyCode::Up => {
            app.cmd_move(false);
            true
        }
        KeyCode::Down => {
            app.cmd_move(true);
            true
        }
        KeyCode::Backspace => {
            app.cmd_query.pop();
            app.cmd_sel = 0;
            true
        }
        // 可打印字符进查询词;带 CONTROL 的字符键(Ctrl+P 再按一次关面板)不归
        // 面板——它落进壳级那一支,与打开面板同一条路。
        KeyCode::Char(c)
            if !k
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
                && !c.is_control() =>
        {
            app.cmd_query.push(c);
            app.cmd_sel = 0;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn ctrl_p() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)
    }

    /// 就绪的面板夹具:越过 splash,免得首帧被 splash 挡住。
    fn panel_app() -> App {
        let mut app = App::new();
        app.splash = false;
        app
    }

    fn type_query(app: &mut App, q: &str) {
        for c in q.chars() {
            app.update(crate::app::Message::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::NONE,
            )));
        }
    }

    // ---- 登记清单完整性:每屏每键位恰有一条 ----

    /// 登记表是唯一真相源,所以「每屏每键位恰有一条」从它自身推:**同一屏 +
    /// 同一修饰键 + 同一键位字面量**不许出现两条。造第二份清单去对是重复
    /// 真相,这里只查表内部的自洽。
    #[test]
    fn 登记表_每屏每键位恰有一条() {
        for (i, a) in HELP_ROWS.iter().enumerate() {
            for b in HELP_ROWS.iter().skip(i + 1) {
                // 册子也是槽位的一部分:订阅册与钥匙册同在编目屏、同是 `g`,
                // 但翻的不是同一本册(`book` 不同)——那是两条,不是重复。
                // 健康屏的复核模式同理:doctor/validate 的 `r` 与引用完整性模态的
                // `r` 同在健康屏、同键,但 `mode` 不同(复核 vs 重扫)——两条,不是重复。
                // 配方屏的域也是槽位的一部分:组件域的 `e`(新建 extension 脚手架)
                // 与配方域的 `e`(编辑选中配方)同在配方屏、同键,但 `domain`
                // 不同——那是两条,不是重复(与 book/mode 同理)。
                let same_slot = a.screen == b.screen
                    && a.book == b.book
                    && a.mode == b.mode
                    && a.domain == b.domain
                    && a.modifiers == b.modifiers
                    && a.key == b.key;
                assert!(
                    !same_slot,
                    "登记表重复:{} 与 {} 同为 {:?} 屏的 `{}`",
                    a.desc, b.desc, a.screen, a.key
                );
            }
        }
    }

    /// 每一屏都至少有一条登记,壳级(全局)键位也有一份——面板按屏名呈现,
    /// 空屏等于那一屏在面板里凭空消失。
    #[test]
    fn 登记表_六屏与全局都至少有键位() {
        for s in Screen::ALL {
            let n = HELP_ROWS.iter().filter(|r| r.screen == Some(s)).count();
            assert!(n > 0, "屏 {s:?} 在登记表里一条都没有");
        }
        let global = HELP_ROWS.iter().filter(|r| r.screen.is_none()).count();
        assert!(global > 0, "壳级键位一条都没登记");
    }

    /// 面板与帮助浮层同源:面板每一行必然来自登记表(键位字面量 + 说明文案
    /// 两头都在表里),帮助浮层每一行也必然来自登记表。改一处两处同步——
    /// 这条钉的就是「没有第二份清单」。
    #[test]
    fn 面板与帮助浮层_都从同一张登记表取行() {
        let mut app = panel_app();
        app.update(crate::app::Message::Key(ctrl_p()));
        let offered = offered(&app);
        assert!(!offered.is_empty(), "空查询应该全中");
        // 判等按登记字段(键位 + 说明 + 屏 + 册 + 修饰键)逐项对,不用指针
        // 身份:`HELP_ROWS` 是 `const`,每个使用点各自物化一份,指针比不出
        // 「同一个出处」,而字段全等已经足够证明这一行来自那张表。
        let registered = |row: &HelpRow| {
            HELP_ROWS.iter().any(|r| {
                r.key == row.key
                    && r.desc == row.desc
                    && r.screen == row.screen
                    && r.book == row.book
                    && r.modifiers == row.modifiers
            })
        };
        for row in offered.iter() {
            assert!(
                registered(row),
                "面板行 `{}`({})不在登记表里(出现了第二份清单?)",
                row.key,
                row.desc
            );
        }
        // 帮助浮层那一侧:渲染出的每一行,键位与说明都该在登记表里找得到。
        let help: String = crate::ui::help_lines(&app.palette)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        for row in HELP_ROWS {
            assert!(help.contains(row.key), "帮助浮层缺键位 `{}`", row.key);
            assert!(help.contains(row.desc), "帮助浮层缺说明 `{}`", row.desc);
        }
    }

    // ---- 模糊匹配输入输出向量 ----

    /// 输入输出向量:期望值用**独立字面量**写死,不拿算法自身推。
    /// 覆盖:空查询全中 / 包含命中 / 子序列命中 / 大小写不敏感 / 查不到给空。
    #[test]
    fn 模糊匹配_输入输出向量() {
        let find = |q: &str| -> Vec<&'static str> {
            HELP_ROWS
                .iter()
                .filter(|r| matches(r, q))
                .map(|r| r.key)
                .collect()
        };
        // 空查询 = 全中(与登记表同序,确定性)
        assert_eq!(
            find(""),
            HELP_ROWS.iter().map(|r| r.key).collect::<Vec<_>>()
        );
        // 包含命中:「反查」是两条 Enter 行的说明词
        let hits = find("反查");
        assert_eq!(hits, vec!["Enter", "Enter"], "按说明词包含命中:{hits:?}");
        // 子序列命中:「jk」按序落在 `j / k` 那行的拼串里,但**不是子串**
        // (中间隔着 " / "),所以只能由子序列路径命中——这一条把两条匹配
        // 路都走到,不靠「包含」蒙混过关。
        let sub = find("jk");
        assert_eq!(sub, vec!["j / k"], "子序列命中:{sub:?}");
        // 大小写不敏感:小写查询照样命中大写键位
        assert_eq!(find("ctrl+p"), vec!["Ctrl+P"], "大小写不敏感");
        // 查不到给空(且不炸)
        assert!(find("不存在的词zzz").is_empty(), "查不到给空");
    }

    /// 匹配面就是「屏名 + 键位 + 动作名」三截拼串:查询屏名也能命中那一屏
    /// 的全部键位(规格「命令清单 = 屏名+键位+动作名」照字面收口)。
    #[test]
    fn 匹配面_屏名键位动作名三截都能命中() {
        let find = |q: &str| -> Vec<&'static str> {
            HELP_ROWS
                .iter()
                .filter(|r| matches(r, q))
                .map(|r| r.key)
                .collect()
        };
        // 屏名「设置」命中设置屏全部键位
        let by_screen = find("设置");
        assert!(by_screen.contains(&"n"), "屏名可搜:{by_screen:?}");
        assert!(by_screen.contains(&"i"), "屏名可搜:{by_screen:?}");
        // 键位「Ctrl+P」按字面命中自己
        assert_eq!(find("Ctrl+P"), vec!["Ctrl+P"], "键位可搜");
        // 动作名「刷新」命中概览 r;装配历史行说明词含「显式刷新」(#101),
        // 同词也命中——两条 r 都名副其实,不是重复登记。
        assert_eq!(find("刷新"), vec!["r", "r"], "动作名可搜");
    }

    // ---- 装配态下破坏性动作不透出 ----

    /// 装配态下 write=true 的条目不进 offered;且**不是静默消失**——面板
    /// 提示行给人一句「装配中收起 N 个写动作」(规格故事 9)。
    #[test]
    fn 装配态_破坏性动作不透出且屏上给得出说明() {
        let mut app = panel_app();
        app.update(crate::app::Message::Key(ctrl_p()));
        let all = offered(&app);
        let writes: Vec<&str> = all.iter().filter(|r| r.write).map(|r| r.key).collect();
        // Enter=打开发射仪式;r=外部源登记;r=编目册登记;d=删除。作者侧工具
        // (spec #95)再添四条写入口:n=skill 脚手架、e=extension 脚手架、
        // p=组件打包、e=配方编辑器写回——都是「拟落账+确认闸」的写流,
        // 装配态下一并收起,口径与上面四条一致。
        assert_eq!(
            writes,
            vec!["Enter", "r", "p", "e", "n", "e", "r", "d"],
            "登记表里的写动作:{writes:?}"
        );

        // 点亮装配互斥:写动作整批收起
        app.assembling = true;
        let gated = offered(&app);
        assert!(
            gated.iter().all(|r| !r.write),
            "装配态下写动作一条都不该透出:{:?}",
            gated
                .iter()
                .filter(|r| r.write)
                .map(|r| r.key)
                .collect::<Vec<_>>()
        );
        assert!(gated.len() < all.len(), "收起了写动作,条数该变少");
        // 4 条原有写入口 + 作者侧工具 4 条(n/e/p/e)= 8
        assert_eq!(hidden_write_count(&app), 8, "该说清收起了几条");

        // 屏上给得出说明,不是静默消失
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(
            s.contains("装配中收起 8 个写动作"),
            "提示行说明收起原因:{s}"
        );
        // 收敛后写动作回来(互斥是装配态专属,不是永久过滤)
        app.assembling = false;
        assert_eq!(offered(&app).len(), all.len(), "互斥收敛后全量回来");
    }

    // ---- 执行走既有分派入口 ----

    /// 面板回车执行的后果与「直接按那个键」**逐字节同后果**:两个 App 布成
    /// 同一状态,一个走「Ctrl+P → 键入 → Enter」,一个直接按那个键;入队 argv
    /// 与日志必须一致(这就是「注入假分派计数」的等价断言——执行确确实实
    /// 走了既有分派入口,没有第二套动作通道)。
    #[test]
    fn 面板回车执行_与直接按那个键同后果() {
        // 概览 r(刷新 ls):直接按会入队一条 `ls --repo <根>`
        let direct = {
            let mut app = panel_app();
            app.update(crate::app::Message::Key(KeyEvent::new(
                KeyCode::Char('r'),
                KeyModifiers::NONE,
            )));
            (app.queue.clone(), app.log.clone(), app.screen)
        };
        let via_panel = {
            let mut app = panel_app();
            app.update(crate::app::Message::Key(ctrl_p()));
            type_query(&mut app, "刷新");
            app.update(crate::app::Message::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )));
            (app.queue.clone(), app.log.clone(), app.screen)
        };
        assert_eq!(
            direct.0, via_panel.0,
            "入队 argv 逐字节同后果:{:?}",
            direct.0
        );
        assert_eq!(direct.2, via_panel.2, "屏不变");
        // 面板自己那一行日志是叠加的,直接按没有;剔掉它再比
        let mut a = direct.1.clone();
        let mut b = via_panel.1.clone();
        a.retain(|l| !l.starts_with("命令面板:"));
        b.retain(|l| !l.starts_with("命令面板:"));
        assert_eq!(a, b, "其余日志同后果");
    }

    /// 切屏类命令也走同一条路:面板里执行「j 切屏」与直接按 `j` 落到同一屏。
    #[test]
    fn 面板执行切屏命令_与直接按j同后果() {
        let mut direct = panel_app();
        direct.update(crate::app::Message::Key(KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
        )));
        let mut via = panel_app();
        via.update(crate::app::Message::Key(ctrl_p()));
        type_query(&mut via, "切屏");
        via.update(crate::app::Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(direct.screen, via.screen, "同一个屏");
        assert!(!via.cmd_open, "执行后面板收起");
        assert!(via.cmd_query.is_empty(), "执行后查询词清空");
    }

    /// 执行的是**选中那一行**,不是第一行:↓ 移选中后再回车,后果跟着选中走。
    #[test]
    fn 面板执行的是选中行_不是固定首行() {
        let mut app = panel_app();
        app.update(crate::app::Message::Key(ctrl_p()));
        type_query(&mut app, "切屏");
        let n = app.cmd_results_len();
        assert!(n >= 2, "该查询至少两条:{n}");
        // 选中第二行(↑/↓ 那条),执行它 → 移选中,不切屏
        app.update(crate::app::Message::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.cmd_sel, 1, "↓ 移到第二条");
        let before = app.screen;
        app.update(crate::app::Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.screen, before, "第二条是 ↑/↓ 移选中,不切屏");
        // 空态执行:有人话、不炸
        let mut empty = panel_app();
        empty.update(crate::app::Message::Key(ctrl_p()));
        type_query(&mut empty, "zzz不存在的词");
        assert_eq!(empty.cmd_results_len(), 0, "零匹配");
        empty.update(crate::app::Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(
            empty.log.iter().any(|l| l.contains("没有可执行的命令")),
            "空态执行给人话:{:?}",
            empty.log.first()
        );
    }

    // ---- 面板是叠加层不是模式劫持 ----

    /// 面板是叠加层不是模式劫持(规格故事 7):**不属于面板输入面的键照旧
    /// 路由到当前屏**。边界钉死在这条测试里,免得后人两头猜:
    /// - 面板的输入面 = 可打印字符(进查询词)+ 退格 + ↑/↓ + Enter + Esc +
    ///   Ctrl+P。可打印字符归查询词是搜索框的本分,所以 `j`/`k`/`?`/`t`/`q`
    ///   这些可打印壳键在面板开着时进的是查询词,不是屏键位。
    /// - 不可打印的壳键(Tab / BackTab 切屏)面板一个都不收,照旧落进壳级
    ///   分派——屏真的换了,面板还开着。
    #[test]
    fn 面板开着时_不属于面板输入面的键照旧路由到当前屏() {
        let mut app = panel_app();
        app.update(crate::app::Message::Key(ctrl_p()));
        assert!(app.cmd_open, "面板先开着");
        // Tab = 壳级切屏(不可打印,面板不收)
        app.update(crate::app::Message::Key(KeyEvent::new(
            KeyCode::Tab,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.screen, Screen::Assemble, "Tab 照旧切屏");
        assert!(app.cmd_open, "面板没收 Tab");
        // BackTab = 上一屏,同样穿透
        app.update(crate::app::Message::Key(KeyEvent::new(
            KeyCode::BackTab,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.screen, Screen::Overview, "BackTab 照旧回上一屏");
        assert!(app.cmd_open, "面板没收 BackTab");
        // 对照:可打印字符进查询词(搜索框的本分),不路由到屏键位
        app.update(crate::app::Message::Key(KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.cmd_query, "j", "可打印字符归查询词");
        assert_eq!(app.screen, Screen::Overview, "没有顺手切屏");
    }

    /// 键入归面板(查询词长出来),这是面板唯一收走的字符键。
    #[test]
    fn 面板里键入归查询词_退格删字符() {
        let mut app = panel_app();
        app.update(crate::app::Message::Key(ctrl_p()));
        type_query(&mut app, "反查");
        assert_eq!(app.cmd_query, "反查");
        app.update(crate::app::Message::Key(KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.cmd_query, "反", "退格删一个字符");
        assert_eq!(app.cmd_sel, 0, "查询一变,选中回首行");
    }

    // ---- Esc 关 / Ctrl+P 再按一次也关 / 空态不炸 ----

    /// Esc 关面板、Ctrl+P 再按一次也关,两条路都把查询词与选中行一并归零
    /// (重开总是从全清单首行看起,与帮助浮层 help_offset 归零同纪律)。
    #[test]
    fn esc与再按ctrl加p都关面板_且归零不留半句查询词() {
        for closer in ["Esc", "Ctrl+P"] {
            let mut app = panel_app();
            app.update(crate::app::Message::Key(ctrl_p()));
            type_query(&mut app, "反查");
            app.update(crate::app::Message::Key(KeyEvent::new(
                KeyCode::Down,
                KeyModifiers::NONE,
            )));
            app.update(crate::app::Message::Key(match closer {
                "Esc" => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                _ => ctrl_p(),
            }));
            assert!(!app.cmd_open, "{closer} 关面板");
            assert!(app.cmd_query.is_empty(), "{closer} 清查询词");
            assert_eq!(app.cmd_sel, 0, "{closer} 选中归零");
        }
    }

    /// 空态(零匹配)不炸:面板照常渲染、有人话、给下一步。
    #[test]
    fn 空态不炸_有人话有下一步() {
        let mut app = panel_app();
        app.update(crate::app::Message::Key(ctrl_p()));
        type_query(&mut app, "zzz不存在的词");
        assert_eq!(app.cmd_results_len(), 0);
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("没有匹配"), "空态说清没匹配:{s}");
        assert!(s.contains("换个词"), "空态给下一步:{s}");
    }

    // ---- 响应式双态快照 ----

    /// 常畅 / 紧凑两档快照(含空态):帧级断言钉「在场」,66×50 下限不溢宽。
    #[test]
    fn 命令面板_常畅与紧凑两档快照_含空态() {
        let mut app = panel_app();
        app.update(crate::app::Message::Key(ctrl_p()));
        // 搜索态:键位原文 + 屏名 + 动作名都在帧上(规格故事 6)
        type_query(&mut app, "反查");
        for (w, h) in [(80u16, 30u16), (66u16, 50u16), (110u16, 30u16)] {
            let s = crate::testutil::frame_display(&app, w, h);
            assert!(s.contains("命令面板"), "{w}×{h} 面板标题在场:{s}");
            assert!(s.contains("Enter"), "{w}×{h} 键位原文在场:{s}");
            assert!(s.contains("编目"), "{w}×{h} 屏名在场:{s}");
            assert!(s.contains("Esc 关"), "{w}×{h} 键位提示在场:{s}");
            assert!(
                s.lines()
                    .all(|l| unicode_width::UnicodeWidthStr::width(l) <= w as usize),
                "{w}×{h} 不溢宽"
            );
        }
        // 空态两档也渲染
        type_query(&mut app, "zzz");
        for (w, h) in [(80u16, 30u16), (66u16, 50u16)] {
            let s = crate::testutil::frame_display(&app, w, h);
            assert!(s.contains("没有匹配"), "{w}×{h} 空态有人话:{s}");
        }
    }

    /// 结果行超量时窗口跟着选中行走:↓ 到底不越界,选中行始终在可见区里
    /// (渲染与命中共用 `cmd_window`,窗口一偏就点不中)。
    #[test]
    fn 结果行超量_窗口跟着选中行走_到底不越界() {
        let mut app = panel_app();
        app.update(crate::app::Message::Key(ctrl_p()));
        let n = app.cmd_results_len();
        for _ in 0..(n + 10) {
            app.update(crate::app::Message::Key(KeyEvent::new(
                KeyCode::Down,
                KeyModifiers::NONE,
            )));
        }
        assert_eq!(app.cmd_sel, n - 1, "到底停底,不绕回");
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("▸"), "选中行有标记:{s}");
    }
}
