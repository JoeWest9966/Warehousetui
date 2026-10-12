//! 五屏渲染快照 + 引擎结果路由的行为测试(TestBackend,零真实进程)。

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::app::{App, HealthMode, Message, Screen};
use crate::engine::{CommandRunner, EngineEvent, EngineOutput, ExitCode, FakeRunner};
use crate::screens::render;
use std::sync::Arc;

fn fake_app() -> App {
    let runner = Arc::new(FakeRunner::new(&[(
        "ls",
        0,
        r#"{"schema":"nw/ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}}"#,
    )]));
    App::with_runner(runner)
}

fn draw(app: &App, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| {
        let area = f.area();
        render(f, area, app);
    })
    .unwrap();
    term.backend().to_string()
}

fn feed(app: &mut App, out: EngineOutput) {
    app.update(Message::Engine(EngineEvent::Finished(Box::new(out))));
}

fn out(args: &[&str], rc: i32, json: &str) -> EngineOutput {
    EngineOutput {
        args: args.iter().map(|s| s.to_string()).collect(),
        exit_code: ExitCode::from_number(rc),
        ok: rc == 0,
        stderr_tail: vec![],
        envelope: serde_json::from_str(json).ok(),
        raw_stdout: json.to_string(),
        parse_error: None,
    }
}

const LS_FULL: &str = r#"{"domain":"all","recipes":[{"name":"research-desk","version":"0.2.0"}],"components":[{"name":"retention","depends":[]}],"sources":[],"profiles":[{"name":"desk","state":"drift","lockId":"9f8e","installedAt":"2026-10-01"},{"name":"lab","state":"ok","lockId":"a1b2"}],"locks":[{"lockId":"9f8e","recipe":"research-desk","bakedAt":"2026-10-01"}]}"#;

// ---- 五屏快照 ----

#[test]
fn 概览屏渲染派表与计数卡() {
    let mut app = fake_app();
    app.ls = Some(serde_json::from_str(LS_FULL).unwrap());
    let s = draw(&app, 110, 30);
    assert!(s.contains("desk"), "派名上表");
    assert!(s.contains("漂移"), "四态灯文字");
    assert!(s.contains("recipes"), "四域计数卡");
}

#[test]
fn 概览空态有入坑指南() {
    let mut app = fake_app();
    app.ls = Some(
        serde_json::from_str(
            r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#,
        )
        .unwrap(),
    );
    let s = draw(&app, 110, 30);
    assert!(s.contains("入坑三步"));
}

#[test]
fn 装配屏发射仪式清单() {
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.as_confirm = true;
    let s = draw(&app, 110, 30);
    assert!(s.contains("发射仪式"));
    assert!(s.contains("NW_HOME"));
}

#[test]
fn 装配屏运行中七步与日志() {
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.assembling = true;
    app.as_log = vec!["铺骨架…".into(), "落组件…".into()];
    let s = draw(&app, 110, 30);
    assert!(s.contains("七步"));
    assert!(
        s.contains("落组件"),
        "stderr 日志原样上屏(操作者唯一的人话面)"
    );
    assert!(
        !s.contains("✓ 3 落组件"),
        "七步不按 stderr 措辞打勾(#60):{s:?}"
    );
}

/// 装配在途两档都给中止键提示(#59):日志窗Pane 标题带着 `x`,操作者翻日志的
/// 视线不用离开窗Pane 去找键。
#[test]
fn 装配中_日志窗给中止键提示_两档都在() {
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.assembling = true;
    app.as_running = true;
    app.as_log = vec!["铺骨架…".into()];
    for (w, h) in [(110u16, 30u16), (80, 30), (66, 50)] {
        let s = draw(&app, w, h);
        assert!(s.contains("x 中止"), "{w}×{h} 在途给中止键提示:{s}");
    }
}

#[test]
fn 健康屏_doctor_七面与横幅() {
    let mut app = fake_app();
    app.screen = Screen::Health;
    feed(
        &mut app,
        out(
            &["doctor", "--profile", "desk"],
            11,
            r#"{"schema":"nw/doctor.v1","nw":"0.1.0","ok":false,"errors":[{"code":"DRIFT","exitCode":11,"message":"config 键被改","hint":"重跑装配命令收敛","details":{"key":"models.json.defaults"}}],"warnings":[],"data":{"results":[{"surface":"manifest","status":"pass","message":"在位"},{"surface":"config","status":"fail","message":"键被改"}],"exit":11,"converged":false}}"#,
        ),
    );
    let s = draw(&app, 110, 30);
    assert!(s.contains("漂移"), "rc11 横幅");
    assert!(s.contains("config 键被改"), "证据原文");
    assert!(s.contains("建议:重跑装配命令收敛"), "建议动作");
}

#[test]
fn 健康屏_validate_清单四态() {
    let mut app = fake_app();
    app.screen = Screen::Health;
    app.hl_mode = HealthMode::Validate;
    feed(
        &mut app,
        out(
            &["validate", "--recipe", "x"],
            3,
            r#"{"schema":"nw/validate.v1","nw":"0.1.0","ok":false,"errors":[],"warnings":[],"data":{"target":"recipe.yaml","results":[{"id":"1","status":"pass","message":"schema 认"},{"id":"15","status":"fail","message":"缺字段"}],"summary":{"total":27,"pass":20,"warn":5,"fail":1,"skip":1}}}"#,
        ),
    );
    let s = draw(&app, 110, 30);
    assert!(s.contains("27 项"));
    assert!(s.contains("缺字段"));
}

#[test]
fn 配方屏五域与详情() {
    let mut app = fake_app();
    app.screen = Screen::Recipes;
    app.ls = Some(serde_json::from_str(LS_FULL).unwrap());
    let s = draw(&app, 110, 30);
    assert!(s.contains("配方"));
    assert!(s.contains("research-desk"));
}

#[test]
fn 设置屏显示生效值与来源() {
    let mut app = fake_app();
    app.screen = Screen::Settings;
    app.engine_version = "0.1.0".into();
    let s = draw(&app, 110, 30);
    assert!(s.contains("来源"));
    assert!(s.contains("config.toml"));
}

#[test]
fn 指路屏列找过的地方() {
    let mut app = fake_app();
    app.screen = Screen::Settings;
    app.engine.found = false;
    app.engine.attempted = vec!["PATH(无 nw)".into(), "/x/nw(不存在)".into()];
    let s = draw(&app, 110, 30);
    assert!(s.contains("找不到 nw"));
    assert!(s.contains("PATH(无 nw)"));
    assert!(s.contains("--nw-bin"));
}

// ---- 引擎结果路由(票 #6 四条路径) ----

#[test]
fn up_成功关态并记收敛() {
    let mut app = fake_app();
    app.assembling = true;
    feed(
        &mut app,
        out(
            &["up", "--lock", "9f8e"],
            0,
            r#"{"schema":"nw/up.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"profile":"desk","components":["retention"],"envMissing":[],"lockId":"9f8e","convergeHash":"abc"}}"#,
        ),
    );
    assert!(!app.assembling);
    assert!(app.as_result.as_ref().unwrap().ok);
    assert!(app.log[0].contains("收敛"));
}

#[test]
fn up_失败停在装配屏并给证据() {
    let mut app = fake_app();
    app.assembling = true;
    feed(
        &mut app,
        out(
            &["up", "--lock", "9f8e"],
            4,
            r#"{"schema":"nw/up.v1","nw":"0.1.0","ok":false,"errors":[{"code":"APPLY_FAILED","exitCode":4,"message":"sha256 不符","hint":"重新 bake","details":{"file":"bin/pi"}}],"warnings":[],"data":null}"#,
        ),
    );
    assert!(!app.assembling);
    assert_eq!(app.screen, Screen::Assemble, "失败留在装配屏");
    assert!(app.as_result.as_ref().unwrap().evidence()[0]
        .message
        .contains("sha256"));
}

#[test]
fn up_并发锁明示() {
    let mut app = fake_app();
    app.assembling = true;
    feed(
        &mut app,
        out(
            &["up", "--lock", "9f8e"],
            5,
            r#"{"schema":"nw/up.v1","nw":"0.1.0","ok":false,"errors":[{"code":"LOCK_CONFLICT","exitCode":5,"message":"profile 锁占用","hint":"稍后重试"}],"warnings":[],"data":null}"#,
        ),
    );
    let r = app.as_result.as_ref().unwrap();
    assert_eq!(r.exit_code, ExitCode::LockConflict);
    assert!(r.headline().contains("锁"));
}

#[test]
fn up_env缺失是降级不算失败() {
    let mut app = fake_app();
    app.assembling = true;
    feed(
        &mut app,
        out(
            &["up", "--lock", "9f8e"],
            0,
            r#"{"schema":"nw/up.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"profile":"desk","envMissing":["OPENAI_API_KEY"],"lockId":"9f8e"}}"#,
        ),
    );
    assert!(app.as_result.as_ref().unwrap().ok);
    app.screen = Screen::Assemble;
    let s = draw(&app, 110, 30);
    assert!(s.contains("OPENAI_API_KEY"));
    assert!(s.contains("降级不算失败"));
}

#[test]
fn doctor_降级档位横幅() {
    let mut app = fake_app();
    app.screen = Screen::Health;
    feed(
        &mut app,
        out(
            &["doctor", "--profile", "desk"],
            10,
            r#"{"schema":"nw/doctor.v1","nw":"0.1.0","ok":false,"errors":[],"warnings":[{"exitCode":10,"message":"OPENAI_API_KEY 未设","hint":"导出后重跑"}],"data":{"results":[],"exit":10}}"#,
        ),
    );
    let s = draw(&app, 110, 30);
    assert!(s.contains("降级"));
}

#[test]
fn stdout_坏了不碎_给证据与原文() {
    let mut app = fake_app();
    app.screen = Screen::Health;
    feed(
        &mut app,
        EngineOutput {
            args: vec!["doctor".into()],
            exit_code: ExitCode::Ok,
            ok: false,
            stderr_tail: vec![],
            envelope: None,
            raw_stdout: "<html>502</html>".into(),
            parse_error: Some("expected value at line 1 column 1".into()),
        },
    );
    let s = draw(&app, 110, 30);
    assert!(s.contains("不可解析"));
}

// ---- 补档:警示带 / broken / skip / 设置交互 ----

#[test]
fn 概览警示带点名漂移派() {
    let mut app = fake_app();
    app.ls = Some(serde_json::from_str(LS_FULL).unwrap());
    let s = draw(&app, 110, 30);
    assert!(s.contains("警示带"));
    assert!(s.contains("desk"), "点名漂移的派");
}

#[test]
fn 健康屏_broken_档位() {
    let mut app = fake_app();
    app.screen = Screen::Health;
    feed(
        &mut app,
        out(
            &["doctor", "--profile", "desk"],
            12,
            r#"{"schema":"nw/doctor.v1","nw":"0.1.0","ok":false,"errors":[{"code":"MANIFEST_BROKEN","exitCode":12,"message":".nw-manifest 损坏","hint":"重新装配"}],"warnings":[],"data":{"results":[{"surface":"manifest","status":"fail","message":"损坏"}],"exit":12}}"#,
        ),
    );
    let s = draw(&app, 110, 30);
    assert!(s.contains("破损"));
}

#[test]
fn 健康屏_skip_是正常态() {
    let mut app = fake_app();
    app.screen = Screen::Health;
    feed(
        &mut app,
        out(
            &["doctor", "--profile", "desk"],
            0,
            r#"{"schema":"nw/doctor.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"results":[{"surface":"runtime","status":"skip","message":"sessions/ 缺席"},{"surface":"manifest","status":"pass","message":"在位"}],"exit":0}}"#,
        ),
    );
    let s = draw(&app, 110, 30);
    assert!(s.contains("○"), "skip 用空心灯");
    assert!(s.contains("缺席"));
    assert!(s.contains("全绿"));
}

#[test]
fn 设置屏_nw_home_轮换并持久化() {
    let mut app = fake_app();
    app.screen = Screen::Settings;
    app.nw_home = "/tmp/nwt-candidate-a".into();
    // 隔离 config 落点:轮换会持久化,不能写开发者真实配置
    let _cfg = crate::testutil::ConfigGuard::install();
    app.cycle_nw_home();
    assert_ne!(app.nw_home, "/tmp/nwt-candidate-a", "轮换应换值");
    assert!(
        app.queue
            .iter()
            .any(|a| a.first().map(String::as_str) == Some("ls")),
        "轮换后刷新 ls"
    );
    let s = draw(&app, 110, 30);
    assert!(s.contains(&app.nw_home));
}

#[test]
fn 设置屏_e_请求外部编辑器() {
    let mut app = fake_app();
    app.screen = Screen::Settings;
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('e'),
        crossterm::event::KeyModifiers::NONE,
    )));
    assert!(app.pending_editor);
}

// ---- #9 设置屏收尾:手填浮层 / config 告警 / 浮层优先级 ----

#[test]
fn 设置屏_手填浮层渲染() {
    let mut app = fake_app();
    app.screen = Screen::Settings;
    app.home_input = true;
    app.home_buf = "/data/pi-homes".into();
    let s = draw(&app, 110, 30);
    assert!(s.contains("手填 NW_HOME"));
    assert!(s.contains("/data/pi-homes"), "缓冲区原文回显");
    assert!(s.contains("█"), "块光标可见");
    assert!(s.contains("Enter 确认"));
}

#[test]
fn 设置屏_config解析失败告警() {
    let mut app = fake_app();
    app.screen = Screen::Settings;
    app.config_error =
        Some("/home/u/.config/warehousetui/config.toml 解析失败:expected `=`".into());
    let s = draw(&app, 110, 30);
    assert!(s.contains("解析失败"), "告警出现");
    assert!(s.contains("config.toml"), "告警带路径");
    assert!(s.contains("$EDITOR"), "给修复指引");
    assert!(s.contains("已降级到默认值"), "说明降级不影响使用");
}

#[test]
fn 设置屏_config健康时无告警() {
    let mut app = fake_app();
    app.screen = Screen::Settings;
    app.config_error = None;
    let s = draw(&app, 110, 30);
    assert!(!s.contains("解析失败"), "健康不该红");
    assert!(s.contains("解析正常"));
}

#[test]
fn 指路屏_不叠config告警() {
    let mut app = fake_app();
    app.screen = Screen::Settings;
    app.engine.found = false;
    app.config_error = Some("/x/config.toml 解析失败:坏".into());
    let s = draw(&app, 110, 30);
    assert!(s.contains("找不到 nw"), "指路屏在场");
    assert!(!s.contains("解析失败"), "指路屏优先,不叠告警");
}

#[test]
fn 输入浮层_盖在指路屏之上() {
    let mut app = fake_app();
    app.screen = Screen::Settings;
    app.engine.found = false;
    app.home_input = true;
    app.home_buf = "/srv/pi".into();
    let s = draw(&app, 110, 30);
    assert!(s.contains("找不到 nw"), "底层指路");
    assert!(s.contains("手填 NW_HOME"), "浮层盖顶");
    assert!(s.contains("/srv/pi"));
}

// ---- #27 数据仓根:五域入口指路 / 设置屏录入 / 三态互不冒充 ----

#[test]
fn 概览_未设数据仓根_五域走指路不摆空表() {
    let mut app = fake_app();
    app.data_root = None;
    app.screen = Screen::Overview;
    let s = draw(&app, 110, 30);
    assert!(s.contains("数据仓根未设"), "指路标题亮相");
    assert!(s.contains("按 d"), "说清去哪设");
    assert!(!s.contains("入坑三步"), "不把「没在读」摆成「没有数据」");
    assert!(!s.contains("还没有 ls 数据"), "旧空态文案退场");
}

#[test]
fn 概览_数据仓根不可用_指路带路径与事实() {
    let mut app = fake_app();
    app.data_root = Some("/nonexistent-nwt-xyz".into());
    app.screen = Screen::Overview;
    let s = draw(&app, 110, 30);
    assert!(s.contains("数据仓根不可用"), "可区分标题");
    assert!(s.contains("/nonexistent-nwt-xyz"), "带收到的路径");
    assert!(s.contains("不存在或不可读"), "事实原文");
}

#[test]
fn 配方屏_未设数据仓根_五域入口指路且计数不冒充0() {
    let mut app = fake_app();
    app.data_root = None;
    app.screen = Screen::Recipes;
    let s = draw(&app, 110, 30);
    assert!(s.contains("数据仓根未设"), "当域入口给指路");
    assert!(s.contains("(—)"), "域计数显未知而非 0");
    assert!(!s.contains("(recipes 0)"), "0 计数退场");
}

#[test]
fn 装配屏_未设数据仓根_锁定集入口走指路不摆空清单() {
    let mut app = fake_app();
    app.data_root = None;
    app.screen = Screen::Assemble;
    let s = draw(&app, 110, 30);
    assert!(s.contains("数据仓根未设"), "锁定集属五域,入口给指路");
    assert!(!s.contains("没有锁定集"), "不摆空清单");
    assert!(!s.contains("没有数据先回概览"), "旧空态文案退场");
}

#[test]
fn 设置屏_数据仓根行_未设给指路_已设给安心_坏根给指路() {
    let mut app = fake_app();
    app.screen = Screen::Settings;
    // fake_app 已注入可用临时根 → 健康行
    let ok = draw(&app, 110, 30);
    assert!(ok.contains("数据仓根"), "行在场");
    assert!(ok.contains("(d 手填)"), "键位提示可见");
    assert!(ok.contains("数据仓根就绪"), "健康给安心行");
    // 未设
    app.data_root = None;
    let unset = draw(&app, 110, 30);
    assert!(unset.contains("未设置"), "未设显式标出");
    assert!(unset.contains("数据仓根未设"), "指路与五域同源");
    // 坏根
    app.data_root = Some("/nonexistent-nwt-xyz".into());
    let bad = draw(&app, 110, 30);
    assert!(bad.contains("/nonexistent-nwt-xyz"), "坏根指路带路径");
    assert!(bad.contains("不存在或不可读"), "事实原文");
}

#[test]
fn 设置屏_数据仓根手填浮层渲染() {
    let mut app = fake_app();
    app.screen = Screen::Settings;
    app.data_input = true;
    app.data_buf = "/data/repo".into();
    let s = draw(&app, 110, 30);
    assert!(s.contains("手填数据仓根"), "浮层标题");
    assert!(s.contains("/data/repo"), "缓冲区回显");
    assert!(s.contains("█"), "块光标可见");
    assert!(s.contains("Enter 确认"), "键位提示");
}

// ---- #22 纵向密度:50 行富余用满(常畅/紧凑两档共用同一 render_*) ----

/// 整行剥掉边框字符后为空 ⇒ 空白行;返回最长连续空白行数。
fn longest_blank_run(rendered: &str) -> usize {
    let (mut best, mut cur) = (0usize, 0usize);
    for line in rendered.lines() {
        let stripped: String = line
            .chars()
            .map(|c| {
                if matches!(
                    c,
                    '│' | '─'
                        | '┌'
                        | '┐'
                        | '└'
                        | '┘'
                        | '├'
                        | '┤'
                        | '┬'
                        | '┴'
                        | '┼'
                        | ' '
                ) {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        if stripped.trim().is_empty() {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// 概览「最近日志」:可视行随高度增加(30 行 < 50 行)。
#[test]
fn 概览最近日志可视行随高度增加() {
    let mut app = fake_app();
    app.ls = Some(serde_json::from_str(LS_FULL).unwrap());
    for i in 0..60 {
        app.push_log(format!("动作{i:02}"));
    }
    let s30 = draw(&app, 110, 30);
    let s50 = draw(&app, 110, 50);
    let n30 = s30.matches("动作").count();
    let n50 = s50.matches("动作").count();
    assert!(n30 > 0, "30 行下也能看到最近日志");
    assert!(
        n50 > n30,
        "可视行随高度增加:30 行可见 {n30} 行,50 行可见 {n50} 行"
    );
    assert!(s30.contains("最近日志"), "日志区有标题");
}

/// 高终端无大片空白:派表/日志铺满可用高度(不引限高、不居中)。
/// 数据给足(40 派 + 60 条流水),断言最长连续空白行 ≤ 3。
#[test]
fn 纵向密度_50行高无大片空白() {
    let mut app = fake_app();
    let profiles: String = (0..40)
        .map(|i| format!(r#"{{"name":"p{i:02}","state":"ok","lockId":"l{i:02}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let rich = format!(
        r#"{{"domain":"all","recipes":[{{"name":"research-desk","version":"0.2.0"}}],"components":[],"sources":[],"profiles":[{profiles}],"locks":[{{"lockId":"l00","recipe":"research-desk","bakedAt":"2026-10-01"}}]}}"#
    );
    app.ls = Some(serde_json::from_str(&rich).unwrap());
    for i in 0..60 {
        app.push_log(format!("动作{i:02}"));
    }
    for (w, h) in [(100u16, 50u16), (66, 50)] {
        let s = draw(&app, w, h);
        assert!(s.contains("最近日志"), "{w}×{h} 日志区在场");
        let run = longest_blank_run(&s);
        assert!(run <= 3, "{w}×{h} 最长空白行 {run} > 3,富余没用满");
    }
}

// ---- #18 紧凑态内层栏收起(66×50 快照)+ 常畅态回归 ----

/// 66×50 紧凑态:装配七步栏收成一行进度——七格未决标记 + 明示粗粒度;
/// 日志独占余下全部行(不再被 14 列竖栏挤压)。
/// #60 起判定不再读 stderr 措辞:引擎七步执行期间不打增量进度,在途
/// 一律未决,结论等 envelope 落地后按 `nw/up.v1` 冻结字段判。
#[test]
fn 紧凑_装配收起一行进度_66x50() {
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.assembling = true;
    // 旧实现的毒丸:七个步骤名逐字进 stderr 日志,字符串匹配会把七步全打 ✓
    app.as_log = STEPS_LOG_ALL.iter().map(|s| s.to_string()).collect();
    let s = draw(&app, 66, 50);
    assert!(
        s.contains("七步 ······· 引擎无增量进度,跑完给结论"),
        "一行进度:七格未决 + 明示粗粒度,原文拿到: {s:?}"
    );
    assert!(!s.contains('✓'), "在途一个 ✓ 都不该有: {s:?}");
    assert!(!s.contains("┌ 七步 "), "七步竖栏必须收起");
    assert!(
        s.contains("stderr 实时日志") && s.contains("0 前置 …"),
        "日志全宽可见,早先行也在(行数变多)"
    );
}

const STEPS_LOG_ALL: [&str; 7] = [
    "0 前置 …",
    "1 引导 …",
    "2 铺骨架 …",
    "3 落组件 …",
    "4 合配置 …",
    "5 写清单 …",
    "6 自检 …",
];

/// 66×50 紧凑态:配方五域轨收起为顶部同级 Tab 行,选中态可见;
/// h/l 切域后 Tab 标记随之移动(可操作)。
#[test]
fn 紧凑_配方五域收起tab行_66x50() {
    let mut app = fake_app();
    app.screen = Screen::Recipes;
    app.ls = Some(serde_json::from_str(LS_FULL).unwrap());
    let s = draw(&app, 66, 50);
    for (label, n) in [
        ("配方", 1),
        ("组件", 1),
        ("外部源", 0),
        ("派", 2),
        ("锁定集", 1),
    ] {
        assert!(s.contains(&format!("{label}({n})")), "五域 Tab 齐:{label}");
    }
    assert!(s.contains("▸ 配方(1)"), "选中域 Tab 高亮可见");
    assert!(!s.contains("┌ 五域 "), "五域竖轨必须收起");
    assert!(s.contains("research-desk"), "列表与详情照常");
    // h/l 切域:Tab 标记移动(键盘可操作,与快照几何无关)
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('l'),
        crossterm::event::KeyModifiers::NONE,
    )));
    let s2 = draw(&app, 66, 50);
    assert!(!s2.contains("▸ 配方(1)"), "切域后旧 Tab 让位");
    assert!(s2.contains("▸ 组件(1)"), "l 切到组件,选中态可见");
}

/// ≥ 断点回归:80×30 常畅态两处内层栏保持既有竖栏(快照基线不回退)。
/// 七步竖栏仍在场,但 #60 起装配中一律未决(引擎无增量进度),不再按
/// stderr 措辞打 ✓。
#[test]
fn 常畅_内层栏回归_80x30竖轨在场() {
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.assembling = true;
    app.as_log = vec!["0 前置 …".into(), "1 引导 …".into()];
    let s = draw(&app, 80, 30);
    assert!(s.contains("┌ 七步 "), "常畅态七步竖栏原样在场");
    assert!(
        s.contains("· 0 前置"),
        "竖栏未决态原样在场(不按日志打勾):{s:?}"
    );
    assert!(s.contains("跑完给结论"), "明示粗粒度");

    let mut app2 = fake_app();
    app2.screen = Screen::Recipes;
    app2.ls = Some(serde_json::from_str(LS_FULL).unwrap());
    let s2 = draw(&app2, 80, 30);
    assert!(s2.contains("┌ 五域 "), "常畅态五域竖轨原样在场");
    assert!(!s2.contains("│ 派(2)"), "常畅态不落紧凑 Tab 行");
}

// ---- #21 滚轮:stderr 偏移进渲染取窗 ----

#[test]
fn 日志偏移取窗_跳过偏移行_七步判定不受影响() {
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.assembling = true;
    app.as_log = (0..10).map(|i| format!("行{i:02}")).collect();
    // 偏移 0:与既有观感一致,最新一行在窗内
    let s0 = draw(&app, 80, 30);
    assert!(s0.contains("行09"), "偏移 0 跟随最新行(既有观感不变)");
    // 偏移 2:最新两行移出窗,更早的行入窗
    app.as_log_offset = 2;
    let s2 = draw(&app, 80, 30);
    assert!(!s2.contains("行09") && !s2.contains("行08"), "偏移行移出窗");
    assert!(s2.contains("行07"), "更早的行入窗");
    assert!(s2.contains("行00"), "窗内仍有更早内容");
    // 越界/陈旧偏移:min 钳制取窗不 panic;#60 起七步判定不读 as_log,取窗照旧
    app.as_log.push("5 写清单 完成".into());
    app.as_log_offset = 100;
    let s3 = draw(&app, 80, 30);
    assert!(!s3.contains("行0"), "取窗钳在日志内,不越界");
    assert!(
        !s3.contains("✓ 5 写清单"),
        "日志里出现步骤名不再打勾(#60):{s3:?}"
    );
    assert!(
        s3.contains("跑完给结论"),
        "偏移把日志行推出窗也不影响粗粒度明示:{s3:?}"
    );
}

// ---- #60 七步进度改稳健判定:判据只取冻结面,不取 stderr 措辞 ----

/// 装配中:七个步骤名逐字进 stderr 日志(旧实现的字符串匹配会把七步全打
/// ✓),现在一律未决 + 明示「引擎无增量进度」。两档外观结构不变。
#[test]
fn 装配中七步一律未决_明示粗粒度_不按stderr措辞打勾() {
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.assembling = true;
    app.as_log = STEPS_LOG_ALL.iter().map(|s| s.to_string()).collect();
    for (w, h) in [(110u16, 30u16), (80, 30), (66, 50)] {
        let s = draw(&app, w, h);
        assert!(!s.contains('✓'), "{w}×{h} 在途一个 ✓ 都不该有:{s:?}");
        assert!(s.contains("跑完给结论"), "{w}×{h} 明示粗粒度");
    }
    // 常畅态:竖栏在场、七行未决、底注明示
    let comfy = draw(&app, 80, 30);
    assert!(comfy.contains("┌ 七步 "), "常畅态七步竖栏原样在场");
    for name in ["0 前置", "6 自检"] {
        assert!(comfy.contains(&format!("· {name}")), "未决态:{name}");
    }
    // 紧凑态:收成一行进度,那一行不带「当前步」名(没有增量信号可指)
    let compact = draw(&app, 66, 50);
    assert!(!compact.contains("┌ 七步 "), "七步竖栏必须收起");
    let line = compact
        .lines()
        .find(|l| l.contains("七步 ") && l.contains('·'))
        .expect("一行进度在场");
    assert!(
        line.contains("七步 ······· 引擎无增量进度,跑完给结论"),
        "一行进度原文:{line:?}"
    );
    for name in ["0 前置", "4 合配置", "6 自检"] {
        assert!(!line.contains(name), "一行进度不指认当前步:{name}:{line:?}");
    }
}

const UP_FULL: &str = r#"{"schema":"nw/up.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"profile":"desk","profileDir":"/d","runtimeKey":"rk-1","components":["retention"],"skeletonFiles":["settings.json"],"configKeys":["models.json"],"envPresent":[],"envMissing":[],"convergeHash":"abc123","lockId":"9f8e","upgraded":false,"installedAt":"2026-10-01"}}"#;

/// 跑完后:七步结论只认 `nw/up.v1` data 的冻结字段在场。六面有结论;
/// 自检在 up 面恒无字段(上游由 CLI 层另起 doctor 复核),明示归 doctor。
#[test]
fn 跑完后七步结论按up_v1冻结字段判() {
    let mut app = fake_app();
    app.assembling = true;
    feed(&mut app, out(&["up", "--lock", "9f8e"], 0, UP_FULL));
    assert!(!app.assembling);
    app.screen = Screen::Assemble;
    let s = draw(&app, 110, 30);
    assert!(s.contains("✓✓✓✓✓✓·"), "六面有结论、自检未决,原文拿到:{s:?}");
    assert!(s.contains("6 面有结论;自检归 doctor(up 不自检)"), "{s:?}");
}

/// 失败在早段(data 为 null):七步结论明说「引擎没给」,不用 ✓/· 假装。
#[test]
fn up失败早段_七步结论明说没给() {
    let mut app = fake_app();
    app.assembling = true;
    feed(
        &mut app,
        out(
            &["up", "--lock", "9f8e"],
            4,
            r#"{"schema":"nw/up.v1","nw":"0.1.0","ok":false,"errors":[{"code":"APPLY_FAILED","exitCode":4,"message":"sha256 不符","hint":"重新 bake"}],"warnings":[],"data":null}"#,
        ),
    );
    app.screen = Screen::Assemble;
    let s = draw(&app, 110, 30);
    assert!(s.contains("引擎没给七步结论(up 面 data 无字段)"), "{s:?}");
    assert!(!s.contains("✓"), "一个 ✓ 都不编:{s:?}");
}

/// 契约守卫同源:schema 不是 `nw/up.v1`(如未来的 up.v2)时,七步结论
/// 明说取不到,绝不按旧形状降级解析出一排 ✓。
#[test]
fn schema不是up_v1_七步结论不按旧形状猜() {
    let mut app = fake_app();
    app.assembling = true;
    feed(
        &mut app,
        out(
            &["up", "--lock", "9f8e"],
            0,
            r#"{"schema":"nw/up.v2","nw":"9.9.9","ok":true,"errors":[],"warnings":[],"data":{"profile":"desk","runtimeKey":"rk","skeletonFiles":[],"components":[],"configKeys":[],"convergeHash":"z"}}"#,
        ),
    );
    app.screen = Screen::Assemble;
    let s = draw(&app, 110, 30);
    assert!(s.contains("结论取不到:envelope 不是 nw/up.v1"), "{s:?}");
    assert!(!s.contains("✓✓"), "不按旧形状凑一排 ✓:{s:?}");
}

// ---- #28 契约守卫:不认识的 schema 走证据面板呈现 ----

#[tokio::test]
async fn 契约守卫_健康屏证据面板显示需升级且不渲染旧形状() {
    const UNKNOWN_DOCTOR: &str = r#"{"schema":"nw/doctor.v2","nw":"9.9.9","ok":false,"errors":[{"code":"MANIFEST_BROKEN","exitCode":12,"message":"引擎自己的话","hint":"h","details":{}}],"warnings":[],"data":{"results":[{"surface":"manifest","status":"fail","message":"旧形状"}],"exit":12}}"#;
    let runner = Arc::new(FakeRunner::new(&[("doctor", 12, UNKNOWN_DOCTOR)]));
    let mut app = App::with_runner(runner.clone());
    app.screen = Screen::Health;
    let args = vec!["doctor".to_string(), "--profile".into(), "desk".into()];
    let out = runner.run(&args).await;
    feed(&mut app, out);
    let s = draw(&app, 110, 30);
    assert!(s.contains("需升级 nw"), "证据面板顶着守卫结论");
    assert!(s.contains("nw/doctor.v2"), "收到的 schema 原文");
    assert!(s.contains("9.9.9"), "引擎版本");
    assert!(s.contains("SCHEMA_UNKNOWN"), "五字段 code 原样呈现");
    assert!(!s.contains("旧形状"), "data 不按旧形状降级解析");
    assert!(
        !s.contains("引擎自己的话"),
        "不认识的 envelope 引擎原话不采信"
    );
}

#[test]
fn 设置屏_引擎版本行常驻_复用既有版本调用() {
    let mut app = fake_app();
    app.engine_version = "0.1.0".into();
    app.screen = Screen::Settings;
    let s = draw(&app, 110, 30);
    assert!(s.contains("版本"), "设置屏有版本行");
    assert!(
        s.contains("nw 0.1.0"),
        "操作者在设置屏能看到面板在跟谁说话(不新增通道)"
    );
}

#[tokio::test]
async fn 喂入内容不经运行器进入日志或渲染或配置_机密本体不回显不落盘() {
    // #55:喂入是运行器能力,但本体绝不进日志行 / 渲染缓冲 / 配置。用 FakeRunner
    // 记录「收到过喂入」,再把 Finished 路由进 App:喂入的标记(模拟机密)不该出现在
    // 任何日志行、屏渲染或配置里 —— 守「永不回显密钥」铁律,不是靠注释守。
    let runner = Arc::new(FakeRunner::new(&[(
        "ls",
        0,
        r#"{"schema":"nw/ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}}"#,
    )]));
    let mut app = App::with_runner(runner.clone());
    let marker = "SUPER-SECRET-TOKEN-55-XYZ";
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    runner.spawn_with_stdin(
        vec!["ls".into(), "--repo".into(), "/x".into()],
        marker.as_bytes().to_vec(),
        tx,
    );
    assert_eq!(runner.stdin_feed_count(), 1, "喂入确有一次到达运行器");
    if let Some(EngineEvent::Finished(o)) = rx.recv().await {
        feed(&mut app, *o);
    }
    assert!(
        app.log.iter().all(|l| !l.contains(marker)),
        "最近日志不回显喂入的机密:{:?}",
        app.log
    );
    let s = draw(&app, 110, 30);
    assert!(!s.contains(marker), "渲染缓冲不回显喂入的机密");
    let (cfg, _) = crate::engine::config::load();
    assert!(!format!("{cfg:?}").contains(marker), "配置不落喂入的机密");
}

// ---- #29 能力探测门(设置屏能力清单渲染) ----

#[test]
fn 设置屏能力清单_缺能力禁用并指路_紧凑态不截断() {
    use crate::engine::{Capability, FakeRunner, Probe};
    // 老引擎(没有 tag 命令组)对 `nw tag` 的回放:stderr 人话「未知子命令」
    let runner = FakeRunner::with_stderr(&[("tag", 3, "", "nw: 未知子命令 \"tag\"")]);
    let mut app = App::with_runner(std::sync::Arc::new(runner));
    app.capabilities.record(Capability::Tag, Probe::Unavailable);
    app.capabilities.record(Capability::Vps, Probe::Unavailable);
    app.capabilities
        .record(Capability::SourceRegister, Probe::Available);
    app.capabilities
        .record(Capability::SourceRecheck, Probe::Available);
    app.screen = Screen::Settings;
    // 常畅态:禁用组 + 指路句都在
    let s = draw(&app, 110, 30);
    assert!(s.contains("入口禁用:vps、标签"), "禁用组可见:{s}");
    assert!(s.contains("缺的是引擎能力,不是面板坏了"), "指路句可见:{s}");
    assert!(s.contains("✓外部源登记"), "齐备能力可见:{s}");
    // 紧凑态 66×50(保证最小):能力行渲染不炸、禁用组可见
    let c = draw(&app, 66, 50);
    assert!(c.contains("入口禁用"), "紧凑态禁用组可见:{c}");
    assert!(c.contains("能力"), "紧凑态能力行在场:{c}");
}

#[test]
fn 设置屏能力清单_recipe_new_与烘焙同组不同项_紧凑态不截断() {
    use crate::engine::{Capability, Probe};
    let mut app = fake_app();
    app.splash = false;
    app.screen = Screen::Settings;
    for cap in Capability::ALL {
        app.capabilities.record(cap, Probe::Available);
    }
    // 常畅态:八项能力一排(顺序 = 入口清单),recipe new 与 bake 并列(同组不同项)
    let s = crate::testutil::frame_display(&app, 110, 30);
    assert!(s.contains("✓配方烘焙"), "烘焙能力在场:{s}");
    assert!(s.contains("✓配方新建"), "新建能力在场:{s}");
    assert!(
        !s.contains("入口禁用") && !s.contains("未决"),
        "全部可用时不占归组行:{s}"
    );
    // 紧凑态 66×50(保证最小):能力行不溢出(竖向预算不变——能力仍是一行)
    let c = crate::testutil::frame_display(&app, 66, 50);
    for l in c.lines() {
        assert!(
            unicode_width::UnicodeWidthStr::width(l) <= 66,
            "66×50 设置行不溢出:{l}"
        );
    }
    assert!(c.contains("配方新建"), "紧凑态新建能力可见:{c}");
    assert!(c.contains("标签"), "紧凑态标签能力可见:{c}");
    // 八项入口一个不少:紧凑态 64 列放不下一行时按显示宽度折行,但不许丢入口
    for cap in Capability::ALL {
        assert!(c.contains(cap.label()), "紧凑态 {cap:?} 入口不许被折掉:{c}");
    }
}

#[test]
fn 设置屏能力_无响应回未决不冒充禁用() {
    use crate::engine::{Capability, FakeRunner, Probe};
    // found=true 但探针无回答:空 stdout、无「未知子命令」——归未决
    let runner = FakeRunner::with_stderr(&[("tag", -1, "", "")]);
    let mut app = App::with_runner(std::sync::Arc::new(runner));
    for cap in Capability::ALL {
        app.capabilities.record(cap, Probe::Unknown);
    }
    app.screen = Screen::Settings;
    let s = draw(&app, 110, 30);
    assert!(s.contains("未决"), "无响应回未决:{s}");
    assert!(!s.contains("入口禁用"), "未决不冒充禁用(三态互不冒充):{s}");
    assert!(
        !s.contains("缺的是引擎能力"),
        "未决不给「缺能力」指路(那是禁用组的):{s}"
    );
}
// ---- #37 外部源详情:类别 / 仓库链接 / 扫出的条目(只读,以引擎输出为准) ----

/// 两条外部源记录:第一条字段齐全(多级类别 + 两条条目),
/// 第二条按引擎形状**缺 kind/repo 键**、类别空数组、条目空数组。
/// 夹具逐字对齐 `nw ls` 实跑输出(见 /opt/spec-notes/browse-suggest-recheck.md §2.1)。
const LS_SOURCES: &str = r#"{"domain":"all","recipes":[],"components":[],"sources":[{"name":"web-search","kind":"skill","category":["检索","网络"],"repo":"JoeWest9966/skills-web","entries":[{"name":"web-search","path":"skills/web-search"},{"name":"deep-research","path":"skills/deep-research"}]},{"name":"theme-pack","category":[],"entries":[]}],"profiles":[],"locks":[]}"#;

fn sources_app(ls: &str) -> App {
    let mut app = fake_app();
    app.splash = false;
    app.screen = Screen::Recipes;
    app.rc_domain = crate::app::Domain::Sources;
    app.ls = Some(serde_json::from_str(ls).unwrap());
    app
}

// ---- #38 组件与锁定集详情补全(spec #35):只读、以引擎 ls 输出为唯一来源 ----

/// 引擎实跑 `nw ls` 的真实形状(0.1.0 契约):components 带 path / depends
/// **对象数组** / vendored / usedBy;locks 的 recipe 是 **{name,version} 对象**、
/// assets 是**数字**。夹具照引擎原文写,不是面板自造。
const LS_COMPLOCKS: &str = r#"{"domain":"all","recipes":[],"components":[{"name":"scout","version":"1.0.0","path":"components/scout","depends":[{"name":"retention","version":"0.4.1"}],"vendored":true,"usedBy":["research-desk"]},{"name":"retention","version":"0.4.1","path":"components/retention","depends":[],"vendored":false,"usedBy":[]}],"sources":[],"profiles":[],"locks":[{"lockId":"9f8e","recipe":{"name":"research-desk","version":"1.0.0"},"bakedAt":"2026-10-01","assets":12}]}"#;

fn comp_locks_app() -> App {
    let mut app = fake_app();
    app.ls = Some(serde_json::from_str(LS_COMPLOCKS).unwrap());
    app.screen = Screen::Recipes;
    app
}

#[test]
fn 外部源详情_类别链接条目原样呈现_不新增引擎调用() {
    let app = sources_app(LS_SOURCES);
    let s = draw(&app, 110, 30);
    assert!(s.contains("名称  web-search"), "记录名可见:{s}");
    assert!(s.contains("kind  skill"), "kind 是引擎字段原样:{s}");
    assert!(s.contains("类别  检索/网络"), "多级类别按数组层级拼出:{s}");
    assert!(
        s.contains("仓库  JoeWest9966/skills-web"),
        "仓库链接可见:{s}"
    );
    assert!(s.contains("条目  2"), "条目条数可见:{s}");
    assert!(s.contains("  web-search"), "条目名字可见:{s}");
    assert!(s.contains("    skills/web-search"), "条目路径可见:{s}");
    // 只读铁律:浏览(含详情联动)不入队、不下发任何引擎命令
    assert!(app.queue.is_empty(), "浏览不入队:{:?}", app.queue);
}

#[test]
fn 外部源详情_缺字段给占位_空类别兜底未分类() {
    let mut app = sources_app(LS_SOURCES);
    app.rc_sel = 1; // theme-pack:kind/repo 键缺、类别空、条目空
    let s = draw(&app, 110, 30);
    assert!(s.contains("名称  theme-pack"), "第二条可见:{s}");
    assert!(s.contains("kind  ——"), "kind 缺失给——,不猜:{s}");
    assert!(s.contains("仓库  ——"), "链接缺失给——,不给空串:{s}");
    assert!(
        s.contains("类别  未分类"),
        "空类别按编目约定兜底(引擎不落该字面量,展示方兜):{s}"
    );
    assert!(s.contains("条目  ——"), "条目空给——,不是空框:{s}");
}

#[test]
fn 外部源详情_键盘切行即时联动() {
    let mut app = sources_app(LS_SOURCES);
    let first = draw(&app, 110, 30);
    assert!(
        first.contains("类别  检索/网络"),
        "选中第 1 条的详情:{first}"
    );
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    )));
    assert_eq!(app.rc_sel, 1, "↓ 移选中");
    let second = draw(&app, 110, 30);
    assert!(
        second.contains("类别  未分类"),
        "详情跟着选中行即时重算:{second}"
    );
    assert!(
        !second.contains("类别  检索/网络"),
        "旧记录不再占位:{second}"
    );
}

#[test]
fn 外部源域为空_给指向登记的指路_不摆空框() {
    use crate::engine::{Capability, Probe};
    let mut app = sources_app(
        r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#,
    );
    // 探测未回 / 缺能力:给能力指路,不冒充「可以登记」
    let s = draw(&app, 110, 30);
    assert!(s.contains("外部源域为空"), "空态有事实一句:{s}");
    assert!(s.contains("登记禁用"), "缺能力时不给键位:{s}");
    assert!(
        s.contains("缺的是引擎能力,不是面板坏了"),
        "指路说清缺的是引擎能力:{s}"
    );
    // 引擎确有 source register 能力:空态给登记键位
    app.capabilities
        .record(Capability::SourceRegister, Probe::Available);
    let s2 = draw(&app, 110, 30);
    assert!(s2.contains("按 r 登记新外部源"), "有能力时给键位指路:{s2}");
    assert!(!s2.contains("登记禁用"), "两态互不冒充:{s2}");
}

/// 66×50(保证最小)紧凑态:条目名字与路径**都**完整可见——逐行画,不是折行;
/// 详情块边框仍在自己的区内(内容不把边框挤出屏)。
#[test]
fn 紧凑_外部源详情_66x50_名字与路径都可见_不折行不溢出() {
    let app = sources_app(LS_SOURCES);
    let f = crate::testutil::frame_display(&app, 66, 50);
    assert!(f.contains("名称  web-search"), "详情首行可见:{f}");
    assert!(f.contains("类别  检索/网络"), "多级类别可见:{f}");
    assert!(
        f.contains("仓库  JoeWest9966/skills-web"),
        "仓库链接完整可见:{f}"
    );
    assert!(f.contains("条目  2"), "条目条数可见:{f}");
    assert!(f.contains("  deep-research"), "条目名字完整可见:{f}");
    assert!(
        f.contains("    skills/deep-research"),
        "条目路径完整可见:{f}"
    );
    assert_eq!(
        f.matches("skills/deep-research").count(),
        1,
        "不折行:路径只出现一次,没有被折成续行"
    );
    assert!(
        f.lines().any(|l| l.contains("└")),
        "详情块边框在场:内容没有溢出块区"
    );
}

// ---- #39 配方域建议命令:选中即给出 bake,执行权留给操作者(ADR-0003) ----

/// 一条带版本 + 一条不带版本的配方;能力探测门记「recipe bake 可用」。
fn bake_app() -> App {
    use crate::engine::{Capability, Probe};
    let mut app = fake_app();
    app.splash = false;
    app.screen = Screen::Recipes;
    app.rc_domain = crate::app::Domain::Recipes;
    app.ls = Some(
        serde_json::from_str(
            r#"{"domain":"all","recipes":[{"name":"research-desk","version":"1.0.0"},{"name":"desk"}],"components":[],"sources":[],"profiles":[],"locks":[]}"#,
        )
        .unwrap(),
    );
    app.capabilities
        .record(Capability::RecipeBake, Probe::Available);
    app
}

#[test]
fn 配方域建议命令_选中即给出bake_旗标与数据仓根与at版本() {
    let mut app = bake_app();
    // 数据仓根钉「定长短路径」:本断言钉的是「--repo 与根相邻整段在场」。
    // 命令按内层宽折行是真设计(紧凑态那条同款),但根一长(如 with_runner
    // 给的 OS 临时目录,macOS /var/folders/… 51+ 字符),折点就落进
    // 「--repo」与根之间,断言随之平台相关(实测 macOS/Windows 红、Linux 绿)。
    // 三平台各取一个必然存在且短的根:unix /tmp,windows 盘根。
    let short_root = if cfg!(windows) {
        "C:\\".to_string()
    } else {
        "/tmp".to_string()
    };
    app.data_root = Some(short_root.clone());
    // 常畅态:命令整段在场
    let s = draw(&app, 110, 30);
    assert!(s.contains("建议命令"), "标成建议命令:{s}");
    assert!(s.contains("执行权在你"), "执行权归属说清:{s}");
    assert!(
        s.contains("nw recipe bake research-desk@1.0.0"),
        "配方带版本时命令含 @版本 段:{s}"
    );
    assert!(s.contains("--out research-desk.sh"), "出参默认值:{s}");
    assert!(
        s.contains(&format!("--repo {short_root}")),
        "--repo 填数据仓根:{s}"
    );
    assert!(s.contains("c 取用(只读选中)"), "键位行可见:{s}");
    assert!(s.contains("退出码 0 成功"), "退出码表在场:{s}");
    assert!(s.contains("口径出自引擎自身"), "注明出处:{s}");
    assert!(s.contains("面板不代跑"), "不冒充面板代跑:{s}");
    // 紧凑态 66×50:命令按内层宽折行,旗标一段都不少
    let c = draw(&app, 66, 50);
    for needle in ["research-desk@1.0.0", "--out research-desk.sh", "--repo"] {
        assert!(c.contains(needle), "66×50 命令片段「{needle}」在场:{c}");
    }
    assert!(c.contains("c 取用(只读选中)"), "66×50 键位行可见:{c}");
    // 不带版本的配方:不猜,只给配方名
    let mut app = bake_app();
    app.rc_sel = 1;
    let s = draw(&app, 110, 30);
    assert!(s.contains("nw recipe bake desk "), "无版本不拼 @版本:{s}");
    assert!(!s.contains("desk@"), "不猜版本:{s}");
}

#[test]
fn 配方域建议命令_未设数据仓根给指路_不给半条命令() {
    let mut app = bake_app();
    app.data_root = None;
    let s = draw(&app, 110, 30);
    // 未设根时五域入口整体走 #27 的根指路(一处说,不说两遍):
    // 标题与「不拿当前目录兜底」可区分,且绝不出现半条命令。
    assert!(s.contains("数据仓根未设"), "指路标题:{s}");
    assert!(s.contains("不拿当前目录兜底"), "带下一步与纪律:{s}");
    assert!(!s.contains("nw recipe bake"), "缺前提绝不渲染半条命令:{s}");
    assert!(!s.contains("--repo"), "连旗标都不给:{s}");
    // 纯函数层三种缺前提两两不同(见 commands.rs 单测);这里只钉 UI 不放行
    assert!(
        app.bake_suggest_gate() == Err(crate::engine::commands::BakeGate::NoDataRoot),
        "未设根时门不给行"
    );
}

#[test]
fn 配方域建议命令_引擎无bake能力给指路_不冒充面板故障() {
    use crate::engine::{Capability, Probe};
    let mut app = bake_app();
    app.capabilities
        .record(Capability::RecipeBake, Probe::Unavailable);
    let s = draw(&app, 110, 30);
    assert!(s.contains("引擎缺 recipe bake 能力"), "缺能力指路:{s}");
    assert!(s.contains("不是面板坏了"), "不冒充面板故障:{s}");
    assert!(!s.contains("nw recipe bake"), "缺能力不给半条命令:{s}");
    // 未决(探测失败)同样不给命令,且说法与禁用可区分
    app.capabilities
        .record(Capability::RecipeBake, Probe::Unknown);
    let s2 = draw(&app, 110, 30);
    assert!(!s2.contains("nw recipe bake"), "未决不给命令:{s2}");
    assert!(
        s2.contains("引擎缺 recipe bake 能力"),
        "未决与禁用同走缺能力指路:{s2}"
    );
}

// ---- #58 配方域建议命令补 recipe new:与 bake 同源同层,执行权留给操作者 ----

#[test]
fn 配方域建议命令_选中即给出recipe_new_骨架名与out_dir同源() {
    use crate::engine::{Capability, Probe};
    let mut app = bake_app();
    app.capabilities
        .record(Capability::RecipeNew, Probe::Available);
    app.data_root = Some("/tmp".to_string());
    // 常畅态:新建骨架命令整段在场(详情内层窄,按宽折行是真设计;
    // 旗标一段都不少,拼回来的串与纯函数层逐字相同)
    let s = draw(&app, 110, 30);
    assert!(s.contains("nw recipe new research-desk"), "命令起头:{s}");
    assert!(s.contains("--out-dir"), "--out-dir 填数据仓根:{s}");
    assert!(
        crate::engine::commands::recipe_new_suggest(Some("research-desk"), Some("/tmp"), true)
            .expect("三前提齐备")
            .display
            == "nw recipe new research-desk --out-dir /tmp",
        "展示串由 argv 拼出(与屏上折行片段同一处真相)"
    );
    assert!(s.contains("新建配方骨架"), "标成新建骨架:{s}");
    assert!(s.contains("执行权在你"), "执行权归属说清:{s}");
    assert!(s.contains("面板不代跑"), "不冒充面板代跑:{s}");
    assert!(
        s.contains("schema nw/recipe-new.v1"),
        "schema 照引擎原话:{s}"
    );
    assert!(
        s.contains("不填 --profile") && s.contains("按配方名兜底"),
        "派名缺省照引擎原话:{s}"
    );
    // bake 仍在:两条建议命令同一块区域,谁也不顶掉谁
    assert!(
        s.contains("nw recipe bake research-desk@1.0.0"),
        "bake 建议命令仍在:{s}"
    );
    // 常畅态两档 + 紧凑态:命令按内层宽折行,旗标一段都不少,行不溢出
    for (w, h) in [(110u16, 30u16), (90, 30), (66, 50)] {
        let f = crate::testutil::frame_display(&app, w, h);
        assert!(f.contains("nw recipe new"), "{w}×{h} 新建命令在场:{f}");
        assert!(f.contains("--out-dir"), "{w}×{h} 旗标在场:{f}");
        for l in f.lines() {
            assert!(
                unicode_width::UnicodeWidthStr::width(l) <= usize::from(w),
                "{w}×{h} 行不溢出:{l}"
            );
        }
    }
    // 最挤的常畅态 80×30:详情内层只有 22 列,bake 的建议命令就把内层占满——
    // 放不下就**整块**不渲染,绝不在屏上留半条命令(标题在、旗标被裁)。
    let tight = crate::testutil::frame_display(&app, 80, 30);
    assert!(tight.contains("nw recipe bake"), "bake 不受影响:{tight}");
    assert!(
        !tight.contains("nw recipe new"),
        "放不下时整块不收,不给半条命令:{tight}"
    );
}

#[test]
fn 配方域建议命令_recipe_new_三种缺前提各有可区分指路_不给半条命令() {
    use crate::engine::{Capability, Probe};
    // 引擎没有 recipe new 能力:指路点名 recipe new,不拿 bake 的话冒充
    let mut app = bake_app();
    app.capabilities
        .record(Capability::RecipeNew, Probe::Unavailable);
    let s = draw(&app, 110, 30);
    assert!(s.contains("引擎缺 recipe new 能力"), "缺能力指路:{s}");
    assert!(s.contains("不是面板坏了"), "不冒充面板故障:{s}");
    assert!(!s.contains("nw recipe new"), "缺能力不给半条命令:{s}");
    // 未决(探测失败)同样不给命令,且与禁用同走缺能力指路
    app.capabilities
        .record(Capability::RecipeNew, Probe::Unknown);
    let s3 = draw(&app, 110, 30);
    assert!(!s3.contains("nw recipe new"), "未决不给命令:{s3}");
    assert!(
        s3.contains("引擎缺 recipe new 能力"),
        "未决与禁用同走缺能力指路:{s3}"
    );
    // 未设数据仓根:--out-dir 指不到本地仓,同样不给半条命令
    let mut app = bake_app();
    app.capabilities
        .record(Capability::RecipeNew, Probe::Available);
    app.data_root = None;
    let s2 = draw(&app, 110, 30);
    assert!(s2.contains("数据仓根未设"), "未设根指路:{s2}");
    assert!(!s2.contains("nw recipe new"), "未设根不给半条命令:{s2}");
    assert!(!s2.contains("--out-dir"), "连旗标都不给:{s2}");
    // 门不放行(纯函数层的三态可区分已在 commands.rs 单测钉过)
    assert_eq!(
        app.recipe_new_suggest_gate(),
        Err(crate::engine::commands::RecipeNewGate::NoDataRoot)
    );
}

#[test]
fn 组件详情_依赖列表来源与缺字段原样呈现() {
    let mut app = comp_locks_app();
    app.rc_domain = crate::app::Domain::Components;
    app.rc_sel = 0; // scout:带依赖、被配方引用、vendored
    let s = draw(&app, 110, 30);
    assert!(s.contains("scout"), "组件名在详情栏:{s}");
    assert!(s.contains("components/scout"), "路径取引擎原文:{s}");
    // 依赖 = 对象数组拼成 name@version(引擎原文的名与版本,不改写)
    assert!(s.contains("retention@0.4.1"), "依赖带上版本:{s}");
    assert!(s.contains("research-desk"), "被谁用取引擎 usedBy 原文:{s}");
    assert!(s.contains("vendored"), "引擎自己的字段名原样留着:{s}");
    assert!(s.contains("true"), "布尔值取引擎原文,不另译:{s}");
    // 来源(本地/外部):nw ls 不输出 → 显式「——」+ 出处,不是面板猜
    assert!(s.contains("(本地/外部)"), "来源栏点名是哪个二分:{s}");
    assert!(s.contains("——"), "缺字段显式「——」:{s}");
    assert!(s.contains("nw ls 不输出,面板不猜。"), "出处与纪律都在:{s}");
}

#[test]
fn 组件详情_空依赖给无依赖_空引用给无_不是白屏() {
    let mut app = comp_locks_app();
    app.rc_domain = crate::app::Domain::Components;
    app.rc_sel = 1; // retention:depends 空、usedBy 空
    let s = draw(&app, 110, 30);
    assert!(s.contains("retention"), "第二个组件也可见:{s}");
    assert!(s.contains("无依赖"), "空依赖给「无依赖」而不是空行:{s}");
    assert!(
        s.contains("依赖     无依赖"),
        "选中行(retention)详情给「无依赖」:{s}"
    );
    assert!(
        !s.contains("依赖     retention@0.4.1"),
        "选中行没有依赖就不冒充有(列表里 scout 的依赖不算详情):{s}"
    );
    assert!(s.contains("——"), "来源仍给「——」:{s}");
    // 详情栏不是白屏:字段行常在
    assert!(s.contains("版本"), "字段名常在:{s}");
}

#[test]
fn 锁定集详情_烤于资产数与ls不输出字段给占位与出处() {
    let mut app = comp_locks_app();
    app.rc_domain = crate::app::Domain::Locks;
    let s = draw(&app, 110, 30);
    assert!(s.contains("9f8e"), "lockId 可见:{s}");
    // recipe 是 {name,version} 对象 → 展示名@版本
    assert!(s.contains("research-desk@1.0.0"), "配方名@版本:{s}");
    assert!(s.contains("2026-10-01"), "烤于时间取引擎原文:{s}");
    assert!(s.contains("12"), "资产数取引擎整数:{s}");
    // 路径 / 外部条目来源:引擎 ls 不输出 → 显式「——」+ 一句出处
    assert!(s.contains("外部来源"), "外部条目来源栏在场:{s}");
    assert!(s.contains("(repo/commit/落地位置)"), "三个子字段点名:{s}");
    assert!(s.contains("——"), "缺字段显式「——」:{s}");
    assert!(
        s.contains("nw ls 不输出路径/外部来源;"),
        "说明是引擎没给,不是面板漏了:{s}"
    );
    assert!(
        s.contains("面板不猜,也不另开引擎调用。"),
        "纪律原话在场(不新增引擎调用):{s}"
    );
}

#[test]
fn 锁定集配方_夹具字符串与引擎对象两形状都呈现() {
    // 同一个渲染面吃两种 recipe 形状:对象形(引擎真输出)与裸字符串形
    // (仓内既有夹具)。形状差异只许体现在标签上,不许让整份 ls 解析失败。
    let mut app = comp_locks_app();
    app.rc_domain = crate::app::Domain::Locks;
    let object_shape = draw(&app, 110, 30);
    assert!(
        object_shape.contains("research-desk@1.0.0"),
        "对象形 → 名@版本:{object_shape}"
    );

    let mut app = fake_app();
    app.ls = Some(serde_json::from_str(LS_FULL).unwrap());
    app.screen = Screen::Recipes;
    app.rc_domain = crate::app::Domain::Locks;
    let string_shape = draw(&app, 110, 30);
    assert!(
        string_shape.contains("research-desk"),
        "字符串形照原样给,不因形状解析失败:{string_shape}"
    );
    assert!(
        !string_shape.contains("——") || string_shape.contains("lockId"),
        "两条路都渲染出字段行,不是白屏:{string_shape}"
    );
}

#[test]
fn 紧凑66x50_组件与锁定集详情_字段与出处都不被截() {
    // 紧凑态保证最小:详情栏不折行(Paragraph 无 wrap),字段名与出处整句可见。
    for (domain, needles) in [
        (
            crate::app::Domain::Components,
            vec![
                "scout",
                "retention@0.4.1",
                "vendored",
                "nw ls 不输出,面板不猜。",
            ],
        ),
        (
            crate::app::Domain::Locks,
            vec![
                "9f8e",
                "research-desk@1.0.0",
                "外部来源",
                "nw ls 不输出路径/外部来源;",
                "面板不猜,也不另开引擎调用。",
            ],
        ),
    ] {
        let mut app = comp_locks_app();
        app.rc_domain = domain;
        let s = draw(&app, 66, 50);
        for needle in needles {
            assert!(
                s.contains(needle),
                "66×50 {} 详情截断了「{needle}」:{s}",
                domain.label()
            );
        }
    }
}

#[test]
fn 配方域建议命令_配方域为空给人话指路_不摆空框() {
    let mut app = bake_app();
    app.ls = Some(serde_json::from_str(r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#).unwrap());
    let s = draw(&app, 110, 30);
    assert!(s.contains("配方域为空"), "空域给人话:{s}");
    assert!(!s.contains("nw recipe bake"), "空域不给半条命令:{s}");
}

#[test]
fn 配方域建议命令_紧凑66x50_键位行钉在详情末行_不溢出() {
    let app = bake_app();
    let s = crate::testutil::frame_display(&app, 66, 50);
    let key_rows: Vec<&str> = s.lines().filter(|l| l.contains("c 取用")).collect();
    assert_eq!(key_rows.len(), 1, "键位行只出现一次:{s}");
    // 键位行钉在详情块内层末行:它的下一行就是块边框
    let at = s.lines().position(|l| l.contains("c 取用")).unwrap();
    assert!(
        s.lines().nth(at + 1).unwrap().contains('└'),
        "键位行贴在详情块底边框之上:{s}"
    );
    for l in s.lines() {
        assert!(
            unicode_width::UnicodeWidthStr::width(l) <= 66,
            "66×50 详情行不溢出:{l}"
        );
    }
}

// ---- #40 外部源漂移:recheck 只读入健康屏,四态,口径以引擎为唯一真相 ----

/// 两条已登记外部源 + 一次 recheck 回放(1 处 added 漂移,rc 3)。
fn recheck_app() -> App {
    use crate::engine::{Capability, Probe};
    let ls = r#"{"domain":"all","recipes":[],"components":[],"sources":[{"name":"web-search","kind":"skill","entries":[{"name":"a","path":"skills/a"},{"name":"b","path":"skills/b"}]},{"name":"theme-pack","kind":"extension","entries":[{"name":"t","path":"."}]}],"profiles":[],"locks":[]}"#;
    let mut app = fake_app();
    app.splash = false;
    app.screen = Screen::Health;
    app.hl_mode = HealthMode::Recheck;
    app.ls = Some(serde_json::from_str(ls).unwrap());
    app.capabilities
        .record(Capability::SourceRecheck, Probe::Available);
    app
}

const RECHECK_DRIFT: &str = r#"{"schema":"nw/source-recheck.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"漂移 1 处:上游与编目不一致","hint":"重跑 nw source register --yes 覆盖写","details":{"catalogPath":"/data/repo/catalog.json"}}],"warnings":[],"data":{"catalogPath":"/data/repo/catalog.json","checked":[{"name":"web-search","kind":"skill","repo":"o/r","commit":"abc123","entries":2},{"name":"theme-pack","kind":"extension","repo":"o/t","commit":"def456","entries":1}],"drift":[{"source":"web-search","kind":"added","entry":"deep-research","path":"skills/deep-research"}]}}"#;

#[test]
fn 外部源漂移_四态呈现_与doctor三档并列不合并() {
    let mut app = recheck_app();
    feed(
        &mut app,
        out(&["source", "recheck", "--root", "/tmp"], 3, RECHECK_DRIFT),
    );
    let s = draw(&app, 110, 30);
    assert!(s.contains("外部源漂移"), "模式标题:{s}");
    // 四态:有漂移=败、查了没漂移=过
    assert!(s.contains("web-search"), "记录名在清单:{s}");
    assert!(s.contains("theme-pack"), "无漂移记录也在清单:{s}");
    // 漂移条目按引擎原文的 kind 呈现,不改写
    assert!(s.contains("added"), "漂移 kind 取引擎原文:{s}");
    assert!(
        s.contains("web-search/deep-research"),
        "条目点名到 source/entry:{s}"
    );
    assert!(s.contains("skills/deep-research"), "路径取引擎原文:{s}");
    // 结论与指路:批准的更新由操作者自己重跑 register
    assert!(s.contains("1 处漂移"), "漂移计数:{s}");
    assert!(
        s.contains("重跑 register 覆盖写"),
        "给可行动指路,执行权在操作者:{s}"
    );
    // 与 doctor 三档不合并:三种语义各说各话
    assert!(
        !s.contains("降级:没完全好"),
        "不把 doctor 的降级档搬来冒充:{s}"
    );
    assert!(
        !s.contains("破损:清单没了"),
        "不把 doctor 的破损档搬来冒充:{s}"
    );
    // 证据五字段原样(含引擎真实的 rc)
    assert!(s.contains("INPUT_INVALID rc3"), "证据带引擎 rc 原文:{s}");
    assert!(s.contains("上游与编目不一致"), "证据 message 原样:{s}");
    // 只读铁律与 commit 口径
    assert!(s.contains("只读"), "明示只读:{s}");
    assert!(s.contains("commit 变化不算漂移"), "口径以引擎为准:{s}");
}

#[test]
fn 外部源漂移_无漂移与未查可区分_不拿未查冒充没问题() {
    // 还没查:明说未查,不给四态结论
    let app = recheck_app();
    let s = draw(&app, 110, 30);
    assert!(s.contains("还没查"), "未查说清:{s}");
    assert!(!s.contains("无漂移"), "未查不冒充「没问题」:{s}");
    // 查了且无漂移:给「过」
    let ok = r#"{"schema":"nw/source-recheck.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/data/repo/catalog.json","checked":[{"name":"web-search","kind":"skill","repo":"o/r","commit":"abc123","entries":2},{"name":"theme-pack","kind":"extension","repo":"o/t","commit":"def456","entries":1}],"drift":[]}}"#;
    let mut app = recheck_app();
    feed(
        &mut app,
        out(&["source", "recheck", "--root", "/tmp"], 0, ok),
    );
    let s = draw(&app, 110, 30);
    assert!(s.contains("无漂移"), "查了没漂移给结论:{s}");
    assert!(s.contains("漂移条目:无"), "条目区说清无:{s}");
}

#[test]
fn 外部源漂移_三种缺前提各有可区分指路() {
    // 未设数据仓根
    let mut app = recheck_app();
    app.data_root = None;
    let s = draw(&app, 110, 30);
    assert!(s.contains("数据仓根未设"), "未设根指路:{s}");
    assert!(!s.contains("还没查"), "未设根不摆查询态:{s}");
    // 引擎无 source 能力
    use crate::engine::{Capability, Probe};
    let mut app = recheck_app();
    app.capabilities
        .record(Capability::SourceRecheck, Probe::Unavailable);
    let s = draw(&app, 110, 30);
    assert!(s.contains("引擎缺 source 能力"), "缺能力指路:{s}");
    assert!(s.contains("不是面板坏了"), "不冒充面板故障:{s}");
    // 编目为空
    let mut app = recheck_app();
    app.ls = Some(
        serde_json::from_str(
            r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#,
        )
        .unwrap(),
    );
    let s = draw(&app, 110, 30);
    assert!(s.contains("没有已登记外部源"), "空编目给人话:{s}");
    assert!(s.contains("按 r 登记"), "空态可行动:{s}");
}

#[test]
fn 外部源漂移_五域还没读到与编目为空两种指路_互不冒充() {
    // 第四种边缘态:ls 还没回音时清单也是 0 行,但不能对操作者断言
    // 「没有记录」——两种说法各有各的下一步,互不冒充。
    let mut app = recheck_app();
    app.ls = None;
    let s = draw(&app, 110, 30);
    assert!(s.contains("五域还没读到"), "没读到说没读到:{s}");
    assert!(s.contains("等 ls 回音"), "指路带下一步:{s}");
    assert!(!s.contains("没有已登记外部源"), "没读到不冒充空编目:{s}");
    // ls 回音了而外部源域确为空:另一个说法
    let mut app = recheck_app();
    app.ls = Some(
        serde_json::from_str(
            r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#,
        )
        .unwrap(),
    );
    let s = draw(&app, 110, 30);
    assert!(s.contains("没有已登记外部源"), "空编目说空编目:{s}");
    assert!(s.contains("按 r 登记"), "空编目给可行动指路:{s}");
    assert!(!s.contains("五域还没读到"), "空编目不冒充没读到:{s}");
}

#[tokio::test]
async fn 外部源漂移_不认识的schema走需升级_不按旧形状猜() {
    // 走 FakeRunner(经 runner 的解析点),契约守卫才合证——直接拼
    // EngineOutput 会绕过守卫,那不是引擎真实给面板的路径。
    let unknown = r#"{"schema":"nw/source-recheck.v2","nw":"9.9.9","ok":false,"errors":[],"warnings":[],"data":{"checked":[{"name":"x"}],"drift":[]}}"#;
    let runner = Arc::new(FakeRunner::new(&[("source recheck", 3, unknown)]));
    let mut app = App::with_runner(runner.clone());
    app.screen = Screen::Health;
    app.hl_mode = HealthMode::Recheck;
    app.capabilities.record(
        crate::engine::Capability::SourceRecheck,
        crate::engine::Probe::Available,
    );
    app.ls = Some(
        serde_json::from_str(
            r#"{"domain":"all","recipes":[],"components":[],"sources":[{"name":"web-search","kind":"skill","entries":[{"name":"a","path":"skills/a"}]}],"profiles":[],"locks":[]}"#,
        )
        .unwrap(),
    );
    let args = vec![
        "source".to_string(),
        "recheck".into(),
        "--root".into(),
        "/tmp".into(),
    ];
    let o = runner.run(&args).await;
    feed(&mut app, o);
    let s = draw(&app, 110, 30);
    assert!(s.contains("需升级 nw"), "契约守卫证据在屏:{s}");
    assert!(s.contains("nw/source-recheck.v2"), "带 schema 原文:{s}");
    assert!(!s.contains("无漂移"), "不认识的形状不按旧形状猜:{s}");
}

#[test]
fn 外部源漂移_紧凑66x50_结论与证据可读不溢出() {
    let mut app = recheck_app();
    feed(
        &mut app,
        out(&["source", "recheck", "--root", "/tmp"], 3, RECHECK_DRIFT),
    );
    let s = crate::testutil::frame_display(&app, 66, 50);
    for needle in ["外部源漂移", "已登记外部源", "漂移条目", "证据"] {
        assert!(s.contains(needle), "66×50 缺「{needle}」:{s}");
    }
    for l in s.lines() {
        assert!(
            unicode_width::UnicodeWidthStr::width(l) <= 66,
            "66×50 行不溢出:{l}"
        );
    }
}

// ---- #61 五域搜索过滤:纯本地筛选,空结果两态可区分指路 ----

/// 五域各两条的夹具(#61):每域第二条都带独特可检索词,过滤后应只剩它一条。
/// 形状照引擎 `nw ls` 原文(components 带 path/depends 对象数组/usedBy;
/// locks 的 recipe 是裸字符串形),不是面板自造。
const LS_FIVE: &str = r#"{"domain":"all","recipes":[{"name":"research-desk","version":"0.2.0","path":"recipes/research-desk/recipe.yaml"},{"name":"night-shift","version":"1.1.0","path":"recipes/night-shift/recipe.yaml"}],"components":[{"name":"retention","version":"0.4.1","path":"components/retention","depends":[],"usedBy":["research-desk"]},{"name":"sentry","version":"0.9.0","path":"components/sentry","depends":[{"name":"retention","version":"0.4.1"}],"usedBy":[]}],"sources":[{"name":"web-search","repo":"JoeWest9966/skills-web","kind":"skill","category":["检索","网络"],"entries":[{"name":"deep-research","path":"skills/deep-research"}]},{"name":"theme-pack","kind":"extension","category":[],"entries":[]}],"profiles":[{"name":"desk","state":"drift","lockId":"9f8e"},{"name":"lab","state":"ok","lockId":"a1b2"}],"locks":[{"lockId":"9f8e","recipe":"research-desk","bakedAt":"2026-10-01"},{"lockId":"c3d4","recipe":"night-shift","bakedAt":"2026-10-02"}]}"#;

fn five_app() -> App {
    let mut app = fake_app();
    app.splash = false;
    app.screen = Screen::Recipes;
    app.ls = Some(serde_json::from_str(LS_FIVE).unwrap());
    app
}

/// 派一个键(与既有测试同一条入口:`Message::Key` → `App::update`)。
fn key(app: &mut App, code: crossterm::event::KeyCode) {
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        code,
        crossterm::event::KeyModifiers::NONE,
    )));
}

/// 敲一串字符(过滤词键入)。
fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        key(app, crossterm::event::KeyCode::Char(c));
    }
}

/// 进过滤输入态并敲入 `q`(不做收词,保留输入态可断言)。
fn filter_with(app: &mut App, q: &str) {
    key(app, crossterm::event::KeyCode::Char('/'));
    type_str(app, q);
}

/// 走**回音路**的 ls 夹具(#61):与直接塞 `app.ls` 不同,这条经过 envelope
/// 解析(schema 认识才拆 data)——用来验证「刷新五域」的真路径,不是旁路设状态。
const LS_ONE_RECIPE: &str = r#"{"domain":"all","recipes":[{"name":"research-desk","version":"0.2.0"}],"components":[],"sources":[],"profiles":[],"locks":[]}"#;
const LS_EMPTY: &str =
    r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#;

fn ls_out(data: &str) -> EngineOutput {
    out(
        &["ls"],
        0,
        &format!(
            r#"{{"schema":"nw/ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{data}}}"#
        ),
    )
}

/// 每域一个只命中第二条的独特词(#61:五域都要能过滤)。
fn five_query(d: crate::app::Domain) -> &'static str {
    match d {
        crate::app::Domain::Recipes => "night",
        crate::app::Domain::Components => "sentry",
        crate::app::Domain::Sources => "theme",
        crate::app::Domain::Profiles => "lab",
        crate::app::Domain::Locks => "c3d4",
    }
}

#[test]
fn 五域搜索过滤_键入即筛_纯本地不新增引擎调用() {
    for d in crate::app::Domain::ALL {
        let mut app = five_app();
        app.rc_domain = d;
        let before = format!("{:?}", app.ls);
        filter_with(&mut app, five_query(d));
        assert_eq!(app.domain_len(), 1, "{d:?} 过滤后只剩命中那条");
        assert_eq!(app.rc_sel, 0, "{d:?} 选中回顶");
        let s = draw(&app, 110, 30);
        assert!(s.contains(five_query(d)), "{d:?} 命中行在场:{s}");
        assert!(!s.contains("research-desk"), "{d:?} 未命中行被筛掉:{s}");
        // 纯本地铁律:不入队、不下发任何引擎命令,ls 回音逐字节不动
        assert!(app.queue.is_empty(), "{d:?} 过滤不入队:{:?}", app.queue);
        assert_eq!(format!("{:?}", app.ls), before, "{d:?} 数据来源不动");
        assert!(!app.ls_loading, "{d:?} 不假装在加载");
    }
}

#[test]
fn 五域搜索过滤_大小写不敏感_可退格改词() {
    let mut app = five_app();
    filter_with(&mut app, "NIGHT");
    assert_eq!(app.domain_len(), 1, "大写同样命中");
    let mut app2 = five_app();
    filter_with(&mut app2, "Shift");
    assert_eq!(app2.domain_len(), 1, "小写片段同样命中");
    // 退格到空 = 不过滤,回到全量(与过滤前逐字节相同)
    for _ in 0..5 {
        key(&mut app, crossterm::event::KeyCode::Backspace);
    }
    assert!(app.rc_filter.is_empty(), "退格清空缓冲");
    assert_eq!(app.domain_len(), 2, "空词回到全量");
}

#[test]
fn 空结果两态指路_域真空与被过滤空_标题与说法两两不同() {
    // 域真空:组件域就是没有条目,且没有过滤词
    let mut vacant = five_app();
    vacant.rc_domain = crate::app::Domain::Components;
    vacant.ls = Some(
        serde_json::from_str(
            r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#,
        )
        .unwrap(),
    );
    let s = draw(&vacant, 110, 30);
    assert!(s.contains("组件域为空"), "域真空有指路:{s}");
    assert!(s.contains("还没有条目"), "域真空说清是域里没有:{s}");
    assert!(!s.contains("无命中"), "域真空不被说成过滤空:{s}");

    // 被过滤空:同一域两条,关键词把两条都筛掉
    let mut filtered = five_app();
    filtered.rc_domain = crate::app::Domain::Components;
    filter_with(&mut filtered, "zzz");
    assert_eq!(filtered.domain_len(), 0, "全被筛掉");
    let s2 = draw(&filtered, 110, 30);
    assert!(s2.contains("组件无命中"), "被过滤空有指路:{s2}");
    assert!(
        s2.contains("「zzz」在 2 条里没有命中"),
        "被过滤空带词与总数:{s2}"
    );
    assert!(s2.contains("按 / 改词"), "给改词的路:{s2}");
    assert!(s2.contains("Esc 清空过滤"), "给清空的路:{s2}");
    assert!(!s2.contains("组件域为空"), "被过滤空不冒充域真空:{s2}");
    assert!(!s2.contains("还没有条目"), "两态说法不混用:{s2}");
    // 详情栏同样不给旧行残影,也不换一套说法
    assert!(s2.contains("组件无命中"), "详情同因由:{s2}");
    assert!(!s2.contains("retention"), "旧行不残影:{s2}");
}

#[test]
fn 切域清空过滤词_选中回顶_旧词不跨域滞留() {
    let mut app = five_app();
    app.rc_domain = crate::app::Domain::Components;
    filter_with(&mut app, "sentry");
    key(&mut app, crossterm::event::KeyCode::Enter);
    app.rc_sel = 0;
    key(&mut app, crossterm::event::KeyCode::Char('l'));
    assert_eq!(app.rc_domain, crate::app::Domain::Sources, "l 切到外部源");
    assert!(app.rc_filter.is_empty(), "切域清空过滤词");
    assert!(!app.rc_filter_on, "输入态不跨域停留");
    assert_eq!(app.rc_sel, 0, "选中回顶");
    let s = draw(&app, 110, 30);
    assert!(s.contains("web-search"), "切域后回到全量:{s}");
    assert!(s.contains("theme-pack"), "全量两条都在:{s}");
}

#[test]
fn 切屏保留过滤词但退输入态_输入态不吞新屏的键() {
    let mut app = five_app();
    app.rc_domain = crate::app::Domain::Recipes;
    key(&mut app, crossterm::event::KeyCode::Char('/'));
    assert!(app.rc_filter_on, "/ 进过滤输入态");
    type_str(&mut app, "night");
    key(&mut app, crossterm::event::KeyCode::Enter);
    assert!(!app.rc_filter_on, "Enter 收词退输入态");
    assert_eq!(app.rc_filter, "night", "收词后过滤词留着");
    // 切屏(j):过滤词保留,输入态已退,新屏的键位照常
    // (#53 起屏序六块:概览/装配/健康/配方/编目/设置,配方之后是编目屏)
    key(&mut app, crossterm::event::KeyCode::Char('j'));
    assert_eq!(app.screen, Screen::Vps, "j 切到下一屏(编目册屏)");
    assert!(!app.rc_filter_on, "切屏后输入态不驻留");
    assert_eq!(app.rc_filter, "night", "过滤词跨屏保留");
    key(&mut app, crossterm::event::KeyCode::Char('j'));
    assert_eq!(app.screen, Screen::Settings, "新屏的键位照常");
    key(&mut app, crossterm::event::KeyCode::Char('j'));
    assert_eq!(app.screen, Screen::Overview, "再切绕回概览");
    // 回配方屏:列表仍是过滤态(标题显命中/总数,状态可见)
    key(&mut app, crossterm::event::KeyCode::Char('k'));
    key(&mut app, crossterm::event::KeyCode::Char('k'));
    key(&mut app, crossterm::event::KeyCode::Char('k'));
    assert_eq!(app.screen, Screen::Recipes, "k 回配方屏");
    let s = draw(&app, 110, 30);
    assert!(s.contains("night-shift"), "过滤态跨屏仍在:{s}");
    assert!(!s.contains("research-desk"), "未命中行仍被筛掉:{s}");
    assert!(s.contains("配方 1/2"), "标题显命中/总数:{s}");
    // 输入态里 j 是缓冲字符,不是切屏(输入态优先于屏键位,同两份手填)
    key(&mut app, crossterm::event::KeyCode::Char('/'));
    key(&mut app, crossterm::event::KeyCode::Char('j'));
    assert_eq!(app.screen, Screen::Recipes, "输入态里 j 不切屏");
    assert_eq!(app.rc_filter, "nightj", "输入态里 j 进缓冲");
}

#[test]
fn 刷新五域_过滤词仍生效_选中钳到新的可见行数() {
    let mut app = five_app();
    app.rc_domain = crate::app::Domain::Recipes;
    filter_with(&mut app, "desk");
    key(&mut app, crossterm::event::KeyCode::Enter);
    assert_eq!(app.domain_len(), 1, "desk 只命中 research-desk");
    // 刷新:新回音仍过同一道筛(纯本地),选中不越界
    feed(&mut app, ls_out(LS_ONE_RECIPE));
    assert_eq!(app.domain_len(), 1, "刷新后过滤词仍生效");
    assert_eq!(app.rc_sel, 0, "选中钳在可见行数内");
    let s = draw(&app, 110, 30);
    assert!(s.contains("research-desk"), "新回音过筛后在场:{s}");
    // 再刷新成空域:可见行数 0,选中钳回 0(不越界、不指向另一条)
    feed(&mut app, ls_out(LS_EMPTY));
    assert_eq!(app.domain_len(), 0, "空域可见 0 条");
    assert_eq!(app.rc_sel, 0, "选中钳回 0");
    let s2 = draw(&app, 110, 30);
    assert!(s2.contains("配方无命中"), "过滤词把新回音也筛空了:{s2}");
    assert!(
        app.queue.is_empty(),
        "过滤与回音处理都不入队:{:?}",
        app.queue
    );
}

#[test]
fn 过滤后选中行不错位_c取用建议命令作用于看得见的那一行() {
    use crate::engine::{Capability, Probe};
    let mut app = five_app();
    app.rc_domain = crate::app::Domain::Recipes;
    app.capabilities
        .record(Capability::RecipeBake, Probe::Available);
    app.data_root = Some("/tmp".into());
    // 不过滤时第 0 条是 research-desk:先钉住过滤前的口径
    let plain = draw(&app, 110, 30);
    assert!(
        plain.contains("nw recipe bake research-desk@0.2.0"),
        "{plain}"
    );
    filter_with(&mut app, "night");
    key(&mut app, crossterm::event::KeyCode::Enter);
    assert_eq!(app.domain_len(), 1, "只剩 night-shift");
    key(&mut app, crossterm::event::KeyCode::Char('c'));
    assert!(app.rc_bake_sel, "过滤后仍可取用(只读)");
    let s = draw(&app, 110, 30);
    assert!(
        s.contains("nw recipe bake night-shift@1.1.0"),
        "取用的是看得见的那一行:{s}"
    );
    assert!(
        !s.contains("research-desk@0.2.0"),
        "不错位到原始数组第 0 条:{s}"
    );
    // 选中态复位:过滤词一动,建议命令的只读选中态跟着清(不冒充旧行的)
    key(&mut app, crossterm::event::KeyCode::Char('/'));
    type_str(&mut app, "x");
    assert!(!app.rc_bake_sel, "改词即复位只读选中态");
    assert!(app.queue.is_empty(), "全程不入队、不执行:{:?}", app.queue);
}

#[test]
fn 常畅与紧凑两档_过滤态布局不溢出不删内容() {
    let mut app = five_app();
    app.rc_domain = crate::app::Domain::Components;
    filter_with(&mut app, "sentry");
    for (w, h) in [(110u16, 30u16), (80, 30), (66, 50)] {
        let f = crate::testutil::frame_display(&app, w, h);
        for l in f.lines() {
            assert!(
                unicode_width::UnicodeWidthStr::width(l) <= usize::from(w),
                "{w}×{h} 行不溢出:{l}"
            );
        }
        assert!(f.lines().any(|l| l.contains("└")), "{w}×{h} 块边框在场");
        assert!(f.contains("sentry"), "{w}×{h} 命中行可见:{f}");
    }
    // 紧凑态:五域 Tab 行照旧一排,当前域计数显命中/总数
    let c = crate::testutil::frame_display(&app, 66, 50);
    for label in ["配方", "组件", "外部源", "派", "锁定集"] {
        assert!(c.contains(label), "66×50 五域 Tab 齐:{label}");
    }
    assert!(
        c.contains("▸ 组件(2)"),
        "过滤态 Tab 计数仍是原总数(不改导航布局):{c}"
    );
    assert!(c.contains("组件 1/2"), "命中/总数由列表块标题说:{c}");
    // 常畅态:五域竖轨照旧
    let m = crate::testutil::frame_display(&app, 80, 30);
    assert!(m.contains("┌ 五域 "), "常畅态五域竖轨原样在场:{m}");
}

#[test]
fn 不过滤时渲染与过滤前逐字节相同() {
    let mut app = five_app();
    let plain = draw(&app, 110, 30);
    // 无过滤词:标题只有域名、计数是原总数,没有任何过滤痕迹
    assert!(plain.contains(" 配方 "), "标题只有域名:{plain}");
    assert!(!plain.contains('/'), "无过滤痕迹:{plain}");
    assert!(!plain.contains("无命中"), "无空态指路:{plain}");
    assert!(!plain.contains("域为空"), "无空态指路:{plain}");
    // Esc 在无过滤词时什么都不做(不新增键位语义)
    let before = format!("{:?}", app.rc_filter);
    key(&mut app, crossterm::event::KeyCode::Esc);
    assert_eq!(
        format!("{:?}", app.rc_filter),
        before,
        "无过滤词时 Esc 空动作"
    );
}
