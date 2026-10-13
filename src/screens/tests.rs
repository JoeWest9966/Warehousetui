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

/// #115 落点浮层夹具:两个本地组件 + 一条 skill 外部源 + 一条 extension 外部源。
const LOCAL_LS: &str = r#"{"domain":"all","recipes":[{"name":"research-desk","version":"0.2.0"}],"components":[{"name":"retention"},{"name":"scout"}],"sources":[{"name":"hello-world","repo":"https://github.com/o/hello-world","kind":"skill","entries":[{"name":"web-scout","path":"skills/web-scout"},{"name":"doc-lookup","path":"skills/doc-lookup"}]},{"name":"theme-pack","kind":"extension","entries":[{"name":"ribbon","path":"extensions/ribbon"}]}],"profiles":[{"name":"desk","state":"ok","lockId":"9f8e"}],"locks":[]}"#;

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

/// #68 概览指令栏加第五条「浏览编目 (v)」的整帧 golden:指令框从 6 长到 7
/// (边框 2 + 5 条),余高由「最近日志」的 `Min` 吸收——其余部件(警示带/派表/
/// 计数卡)排布与高度不变,只多这一行。常畅(110×30 / 80×30)与紧凑(66×50)
/// 两档都放得下这条入口且无行超宽;键位 `v` 与指令栏 / 帮助 / README 同写
/// (故事 5/6/7;HUD 是壳级提示栏,不写这一屏的跳屏键)。
/// 紧凑态内层仅 18 列,19 宽的指令行沿用 #22 既定裁法把 `)` 裁掉(同 `h/l/r`),
/// 不放宽指令框、不引限高、不居中。
#[test]
fn 概览指令栏第五条_浏览编目带v键_两档不溢出() {
    let mut app = fake_app();
    app.ls = Some(serde_json::from_str(LS_FULL).unwrap());
    app.splash = false; // frame_display 走整帧壳:splash 在场会画 logo,先交棒
    for (w, h) in [(110u16, 30u16), (80, 30), (66, 50)] {
        let f = crate::testutil::frame_display(&app, w, h);
        assert!(
            f.contains("浏览编目"),
            "{w}×{h} 指令栏应有第五条「浏览编目」:{f}"
        );
        // 四条原有指令不丢(顺序在第五条之前)
        for label in ["跑一次体检", "发起装配", "浏览配方", "刷新 ls"] {
            assert!(f.contains(label), "{w}×{h} 原指令 {label} 不丢:{f}");
        }
        // 常畅态键位提示 (v) 完整;紧凑态 (v) 被裁到「(v」,与 h/l/r 同裁法
        if h == 30 {
            assert!(f.contains("(v)"), "{w}×{h} 常畅态键位 (v) 完整可见:{f}");
        }
        // 其余部件排布与高度不变:警示带 / 派表 / 计数卡 / 最近日志都在场
        assert!(f.contains("警示带"), "{w}×{h} 警示带在场:{f}");
        assert!(
            f.contains("desk") && f.contains("漂移"),
            "{w}×{h} 派表在场:{f}"
        );
        assert!(f.contains("recipes"), "{w}×{h} 计数卡在场:{f}");
        assert!(
            f.contains("最近日志"),
            "{w}×{h} 最近日志区在场(未被挤掉):{f}"
        );
        // 两档纪律:无行超宽
        for ln in f.lines() {
            assert!(
                unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                "{w}×{h} 有行超宽:{ln}"
            );
        }
    }
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

/// 装配在途两档都给中止键提示(#59 / #71):`x 中止` 在**正文末行的可点键位行**
/// 上——操作者翻日志的视线不用离开日志区去找键。
///
/// #71 特意把它从日志窗 Pane 的**块标题**挪了出来(块标题只描述这块
/// 是什么:stderr 实时日志;话术单源在键位行 `assemble::assemble_key_parts`,
/// 渲染画的与命中测的同一份)。所以这条钉两个方向:键位行那一行在场、块标题
/// 那一行不带它——早先只断言「帧里出现过 `x 中止`」,那个串被挪回块标题也
/// 照样绿,测试就不再钉它声称钉的位置。
#[test]
fn 装配中_日志窗给中止键提示_两档都在() {
    use crate::screens::assemble::ASSEMBLE_ABORT;
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.assembling = true;
    app.as_running = true;
    app.as_log = vec!["铺骨架…".into()];
    app.splash = false; // 整帧壳渲染走 splash 优先,先交棒(屏幕本体的 draw 不看它)
    for (w, h) in [(110u16, 30u16), (80, 30), (66, 50)] {
        let s = draw(&app, w, h);
        assert!(s.contains(ASSEMBLE_ABORT), "{w}×{h} 在途给中止键提示:{s}");
        // 位置:正文键位行那一行在场(HUD 那句之外另有一行),且日志窗 Pane 的
        // 块标题那一行不带它
        let f = crate::testutil::frame_display(&app, w, h);
        assert!(
            f.lines()
                .any(|l| l.contains(ASSEMBLE_ABORT) && !l.contains("装配中")),
            "{w}×{h} 中止键在正文键位行上:{f}"
        );
        assert!(
            f.lines()
                .all(|l| !(l.contains("stderr 实时日志") && l.contains(ASSEMBLE_ABORT))),
            "{w}×{h} 块标题只描述这块是什么,不夹带键位提示:{f}"
        );
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

// ---- #116 引用完整性:面板自有检查,四态、来源标注、两档快照 ----

/// 夹具数据仓:孤儿 / 断链 / 正常三态俱全(与 `app::tests` 的夹具同构,
/// 屏层自己建一份——测试夹具不跨模块共享)。
fn integrity_repo(tag: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("nwt-integrity-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    let put = |rel: &str, body: &str| {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    };
    put("skills/web-scout/SKILL.md", "---\nname: web-scout\n---\n");
    put("skills/lonely/SKILL.md", "---\nname: lonely\n---\n");
    put("extensions/ribbon/package.json", "{}");
    put("extensions/dropped/package.json", "{}");
    put(
        "components/scout/component.yaml",
        "kind: pi-local-package\n",
    );
    put(
        "recipes/research-desk/recipe.yaml",
        "kind: recipe\nname: research-desk\ncomponents:\n  - component: scout\n  - component: web-scout\n  - component: ribbon\n  - component: ghost\n",
    );
    put(
        "recipes/lab/recipe.yaml",
        "kind: recipe\nname: lab\ncomponents:\n  - component: ghost\n  - component: hello-world\n    source: external\n",
    );
    root
}

/// 夹具 App:数据仓根指向夹具仓、切到引用完整性并现扫一次。
fn integrity_app(tag: &str) -> (App, std::path::PathBuf) {
    let root = integrity_repo(tag);
    let mut app = fake_app();
    app.splash = false;
    app.screen = Screen::Health;
    app.hl_mode = HealthMode::Integrity;
    app.data_root = Some(root.to_str().unwrap().to_string());
    app.run_integrity();
    (app, root)
}

#[test]
fn 引用完整性_四态呈现_与引擎读法一致_来源标注可区分() {
    let (app, root) = integrity_app("four");
    let s = draw(&app, 110, 30);
    // 三态分类:孤儿 / 断链 / 正常各就各位
    assert!(s.contains("ghost"), "断链点名组件:{s}");
    assert!(s.contains("lonely"), "孤儿 skill 在列:{s}");
    assert!(s.contains("dropped"), "孤儿 extension 在列:{s}");
    assert!(s.contains("web-scout"), "正常 skill 也在列:{s}");
    assert!(s.contains("ribbon"), "正常 extension 也在列:{s}");
    // 结果四态:与 validate 同一套灯字符(✗ 败 / ◐ 警 / ● 过)
    assert!(s.contains("✗"), "断链是败:{s}");
    assert!(s.contains("◐"), "孤儿是警:{s}");
    assert!(s.contains("●"), "进了配方是过:{s}");
    // 人话点题
    assert!(s.contains("没进任何配方"), "孤儿点题:{s}");
    assert!(s.contains("本体不在"), "断链点题:{s}");
    assert!(
        s.contains("被 lab、research-desk 引用"),
        "断链点名是哪几条配方:{s}"
    );
    assert!(s.contains("进了配方"), "正常态点题:{s}");
    // 来源标注:面板自有检查可区分于引擎检查(引擎三项的块标题带「· 引擎」)
    assert!(
        s.contains(crate::screens::health::CheckSource::Panel.badge()),
        "来源徽标在屏:{s}"
    );
    assert!(
        !s.contains(&format!(
            "· {}",
            crate::screens::health::CheckSource::Engine.badge()
        )),
        "本模式的块标题不标引擎来源:{s}"
    );
    // 只读口径
    assert!(s.contains("只读"), "明示只读:{s}");
    assert!(s.contains("只报不修"), "只报不修:{s}");
    // 自读事实:规模与外部源口径
    assert!(s.contains("扫到 2 个配方"), "规模在右栏:{s}");
    assert!(s.contains("外部源引用 1 条"), "外部源引用不进本图:{s}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn 引用完整性_引擎三项的块标题带来源徽标_面板项也带() {
    // 来源标注是**机制**:四个模式各自声明,不是只给面板那一项贴花。
    let mut app = fake_app();
    app.splash = false;
    app.screen = Screen::Health;
    let doctor = r#"{"schema":"nw/doctor.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"results":[{"surface":"manifest","status":"pass","message":"在位"}],"exit":0,"converged":true}}"#;
    feed(&mut app, out(&["doctor", "--profile", "desk"], 0, doctor));
    let s = draw(&app, 110, 30);
    assert!(
        s.contains(&format!(
            "七面 {}",
            crate::screens::health::CheckSource::Engine.title_suffix()
        )),
        "doctor 块标题标来源:{s}"
    );
    assert!(
        !s.contains(crate::screens::health::CheckSource::Panel.badge()),
        "引擎模式不出现面板徽标:{s}"
    );
    // 引擎项的灯与文案没被改花:四态灯仍在、原文消息仍在
    assert!(s.contains("manifest"), "引擎项原文在场:{s}");
    assert!(s.contains("在位"), "引擎项消息原文在场:{s}");
    assert!(s.contains("●"), "四态灯读法不变:{s}");
}

#[test]
fn 引用完整性_数据仓根未设走指路_不摆空列表不冒充0() {
    let (mut app, root) = integrity_app("noroot");
    app.data_root = None;
    app.run_integrity();
    let s = draw(&app, 110, 30);
    assert!(s.contains("数据仓根未设"), "未设根走既有指路:{s}");
    assert!(
        s.contains(crate::screens::health::CheckSource::Panel.badge()),
        "来源标注仍在:{s}"
    );
    assert!(!s.contains("没进任何配方"), "不摆空列表冒充没问题:{s}");
    assert!(!s.contains("本体不在"), "不摆断链行:{s}");
    assert!(!s.contains("ghost"), "没有半个检查行:{s}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn 引用完整性_无可扫对象与全正常两态不互冒充() {
    // 扫了个空:一个组件家都没有——说「没扫到」,不说「没问题」
    let empty = std::env::temp_dir().join(format!("nwt-integrity-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&empty);
    std::fs::create_dir_all(&empty).unwrap();
    let mut app = fake_app();
    app.splash = false;
    app.screen = Screen::Health;
    app.hl_mode = HealthMode::Integrity;
    app.data_root = Some(empty.to_str().unwrap().to_string());
    app.run_integrity();
    let s = draw(&app, 110, 30);
    assert!(s.contains("没扫到组件目录"), "空仓说空仓:{s}");
    assert!(!s.contains("全进配方"), "空仓不冒充全绿:{s}");
    std::fs::remove_dir_all(&empty).ok();
    // 全正常:扫到了且全部进配方
    let root = integrity_repo("clean");
    std::fs::remove_dir_all(root.join("skills/lonely")).unwrap();
    std::fs::remove_dir_all(root.join("extensions/dropped")).unwrap();
    let desk = root.join("recipes/research-desk/recipe.yaml");
    std::fs::write(&desk, "kind: recipe\nname: research-desk\ncomponents:\n  - component: scout\n  - component: web-scout\n  - component: ribbon\n").unwrap();
    let lab = root.join("recipes/lab/recipe.yaml");
    std::fs::write(
        &lab,
        "kind: recipe\nname: lab\ncomponents:\n  - component: hello-world\n    source: external\n",
    )
    .unwrap();
    let mut app = fake_app();
    app.splash = false;
    app.screen = Screen::Health;
    app.hl_mode = HealthMode::Integrity;
    app.data_root = Some(root.to_str().unwrap().to_string());
    app.run_integrity();
    let s = draw(&app, 110, 30);
    assert!(s.contains("全进配方"), "全绿给结论:{s}");
    assert!(!s.contains("没进任何配方"), "没有孤儿行:{s}");
    assert!(!s.contains("引用了不存在的组件"), "没有断链行:{s}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn 引用完整性_选中行明细列引用方_上下键与点行同路() {
    let (mut app, root) = integrity_app("sel");
    let s = draw(&app, 110, 30);
    assert!(s.contains("选中:断链 ghost"), "首行(断链)明细在右栏:{s}");
    assert!(s.contains("引用方:lab、research-desk"), "断链的引用方:{s}");
    // ↓ 到第一个孤儿行:右栏跟着选中行走
    app.update(Message::Key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    )));
    let s = draw(&app, 110, 30);
    assert!(s.contains("选中:extension dropped"), "选中行走动:{s}");
    assert!(s.contains("引用方:无"), "孤儿没有引用方:{s}");
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn 引用完整性_常畅与紧凑两档快照_结论可读不溢出() {
    let (app, root) = integrity_app("snap");
    let comfy = crate::testutil::frame_display(&app, 110, 30);
    for needle in [
        "引用完整性",
        "检查行",
        "自读事实",
        "ghost",
        crate::screens::health::CheckSource::Panel.badge(),
    ] {
        assert!(comfy.contains(needle), "常畅 110×30 缺「{needle}」:{comfy}");
    }
    let compact = crate::testutil::frame_display(&app, 66, 50);
    for needle in [
        "引用完整性",
        "检查行",
        "ghost",
        crate::screens::health::CheckSource::Panel.badge(),
    ] {
        assert!(
            compact.contains(needle),
            "紧凑 66×50 缺「{needle}」:{compact}"
        );
    }
    for (w, frame) in [(110, comfy.as_str()), (66, compact.as_str())] {
        for l in frame.lines() {
            assert!(
                unicode_width::UnicodeWidthStr::width(l) <= w,
                "{w} 宽行不溢出:{l}"
            );
        }
    }
    std::fs::remove_dir_all(&root).ok();
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

/// 在整帧真显示里定位某子串的 (行, 列)(列按显示宽累计——`frame_display`
/// 已跳过宽字续格,字节偏移≠终端列;与命中区同一把尺,同 `layout::tests` 的
/// `locate`)。鼠标测试据此把点击落到屏上看得见的那几个字上。
fn locate(frame: &str, needle: &str) -> (u16, u16) {
    use unicode_width::UnicodeWidthChar;
    for (y, line) in frame.split('\n').enumerate() {
        if let Some(byte) = line.find(needle) {
            let col: u16 = line[..byte]
                .chars()
                .map(|c| UnicodeWidthChar::width(c).unwrap_or(0) as u16)
                .sum();
            return (y as u16, col);
        }
    }
    panic!("整帧里找不到「{needle}」");
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

// ---- 装配历史次级视图(#101,datarepo 读侧)----

/// 就绪派的一份清单(字段集对齐上游 internal/manifest/manifest.go 样本)。
const MANIFEST_FIXTURE: &str = r#"{"schema":"nw/manifest.v1","lockId":"desk-1.0.0-8f3a1c2","installedAt":"2026-10-01T12:00:00Z","recipe":{"name":"research-desk","version":"1.0.0"},"profile":"desk","runtime":{"runtimeKey":"node-24.21.0+pi-1.0.0","node":"24.21.0","pi":"1.0.0","chezmoi":"2.73.0","nodeSource":"bundled"},"components":[{"name":"retention","version":"0.4.1","landing":{"path":"nw/components/retention","kind":"pi-local-package"}},{"name":"scout","version":"0.2.0","landing":{"path":"nw/components/scout","kind":"pi-local-package"}}],"skeletonFiles":["AGENTS.md"],"configKeys":["/settings/theme"],"env":[{"name":"TAVILY_API_KEY","required":true,"present":true}],"convergeHash":"9f2c","engine":{"nw":"0.1.0","schema":"nw/manifest.v1"}}"#;

/// 一次性 NW_HOME fixture:三个派三种态——desk 就绪、lab 缺失(派目录在、
/// 清单不在=上一轮没提交完)、broken 破损(清单不是合法 JSON)。
fn history_fixture(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("nwt-hist-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    let profiles = dir.join("profiles");
    std::fs::create_dir_all(profiles.join("desk")).unwrap();
    std::fs::write(profiles.join("desk/.nw-manifest"), MANIFEST_FIXTURE).unwrap();
    std::fs::create_dir_all(profiles.join("lab")).unwrap();
    std::fs::create_dir_all(profiles.join("broken")).unwrap();
    std::fs::write(profiles.join("broken/.nw-manifest"), "{not json").unwrap();
    dir
}

/// 打开装配历史的就绪态 App(NW_HOME 指向 fixture)。
fn history_app(tag: &str) -> App {
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.splash = false;
    app.nw_home = history_fixture(tag).to_str().unwrap().to_string();
    key(&mut app, crossterm::event::KeyCode::Char('h'));
    app
}

#[test]
fn 装配历史_三态两档渲染不炸且锚点在阵() {
    let app = history_app("snap");
    for (w, h) in [(80u16, 30u16), (66, 50)] {
        let f = crate::testutil::frame_display(&app, w, h);
        assert!(f.contains("装配历史"), "{w}×{h} 应有次级视图标题:{f}");
        for name in ["broken", "desk", "lab"] {
            assert!(f.contains(name), "{w}×{h} 派 {name} 上表:{f}");
        }
        assert!(
            f.contains("desk-1.0.0-8f3a1c2"),
            "{w}×{h} 就绪派的锁定集上表:{f}"
        );
        assert!(f.contains("清单缺失"), "{w}×{h} 缺失态措辞:{f}");
        assert!(f.contains("清单破损"), "{w}×{h} 破损态措辞:{f}");
        assert!(!f.contains("panic"), "{w}×{h} 不炸:{f}");
    }
}

#[test]
fn 装配历史_就绪行详情给全字段_含后续票占位() {
    let mut app = history_app("detail");
    // 排序 broken → desk → lab:按一次到 desk(就绪态)看详情
    key(&mut app, crossterm::event::KeyCode::Down);
    let f = draw(&app, 110, 30);
    for anchor in [
        "desk-1.0.0-8f3a1c2",
        "2026-10-01T12:00:00Z",
        "9f2c",
        "research-desk",
        "node-24.21.0+pi-1.0.0",
        "0.1.0",
    ] {
        assert!(f.contains(anchor), "详情应有 {anchor}:{f}");
    }
    assert!(f.contains("组件 2 条"), "组件条数:{f}");
    assert!(f.contains("Enter 看清单原文"), "键位说明:{f}");
    assert!(
        f.contains("#102") && f.contains("#103"),
        "回收站/存档日志挂点占位(#101):{f}"
    );
}

#[test]
fn 装配历史_回车看清单原文_esc关() {
    let mut app = history_app("raw");
    // 排序 broken → desk → lab:按一次到 desk(就绪态,有原文)
    key(&mut app, crossterm::event::KeyCode::Down);
    key(&mut app, crossterm::event::KeyCode::Enter);
    let raw = app.ah_raw.clone().expect("回车应开原文浮层");
    assert_eq!(raw, MANIFEST_FIXTURE, "原文即数据仓里那个文件的字节");
    let f = draw(&app, 110, 30);
    assert!(f.contains("清单原文"), "浮层标题:{f}");
    assert!(f.contains("nw/manifest.v1"), "原文内容上屏:{f}");
    key(&mut app, crossterm::event::KeyCode::Esc);
    assert!(app.ah_raw.is_none(), "Esc 关原文");
}

#[test]
fn 装配历史_缺失态回车不开空浮层只留指路() {
    let mut app = history_app("raw-missing");
    // 排序 broken → desk → lab:按两次到 lab(缺失态)
    key(&mut app, crossterm::event::KeyCode::Down);
    key(&mut app, crossterm::event::KeyCode::Down);
    key(&mut app, crossterm::event::KeyCode::Enter);
    assert!(app.ah_raw.is_none(), "缺失态没有原文,不开空浮层");
    assert!(
        app.log.iter().any(|l| l.contains("没有清单原文")),
        "留一句指路:{:?}",
        app.log.iter().take(3).collect::<Vec<_>>()
    );
}

#[test]
fn 装配历史_nwhome未设走指路不摆空列表() {
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.splash = false;
    // 一个不存在的安装根:未设/不存在态
    app.nw_home = std::env::temp_dir()
        .join(format!("nwt-hist-nope-{}", std::process::id()))
        .to_str()
        .unwrap()
        .to_string();
    key(&mut app, crossterm::event::KeyCode::Char('h'));
    assert!(app.as_history.as_ref().unwrap().root_missing, "根缺失标记");
    let f = draw(&app, 110, 30);
    assert!(f.contains("没有安装根"), "指路标题:{f}");
    assert!(f.contains("不存在"), "指路带事实:{f}");
    assert!(
        !f.contains("一个派都还没有"),
        "不摆「空列表」那种骗人话:{f}"
    );
}

#[test]
fn 装配历史_有根没派_与未设两态可区分() {
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.splash = false;
    let dir = std::env::temp_dir().join(format!("nwt-hist-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    app.nw_home = dir.to_str().unwrap().to_string();
    key(&mut app, crossterm::event::KeyCode::Char('h'));
    assert!(!app.as_history.as_ref().unwrap().root_missing, "根在");
    let f = draw(&app, 110, 30);
    assert!(f.contains("一个派都还没有"), "有根没派说空列表:{f}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn 装配历史_开关与选中_键位自成一套() {
    let mut app = history_app("keys");
    assert!(app.as_history.is_some(), "h 开");
    key(&mut app, crossterm::event::KeyCode::Down);
    assert_eq!(app.ah_sel, 1, "↓ 选下一行");
    key(&mut app, crossterm::event::KeyCode::Up);
    assert_eq!(app.ah_sel, 0, "↑ 选上一行");
    // 视图内 Enter 是看原文,不是发射仪式
    key(&mut app, crossterm::event::KeyCode::Enter);
    assert!(app.ah_raw.is_some(), "Enter 看原文(选中行=broken,也有原文)");
    assert!(!app.as_confirm, "不误打开发射仪式");
    key(&mut app, crossterm::event::KeyCode::Esc); // 关原文
    key(&mut app, crossterm::event::KeyCode::Esc); // 关视图
    assert!(app.as_history.is_none(), "Esc 返航");
    // 关后键位回归选锁定集:h 再开
    key(&mut app, crossterm::event::KeyCode::Char('h'));
    assert!(app.as_history.is_some(), "h 再开");
}

#[test]
fn 装配历史_过滤本地筛_esc清词() {
    let mut app = history_app("filter");
    key(&mut app, crossterm::event::KeyCode::Char('/'));
    assert!(app.ah_filter_on, "/ 进过滤输入态");
    for c in ['d', 'e', 's'] {
        key(&mut app, crossterm::event::KeyCode::Char(c));
    }
    assert_eq!(app.ah_visible().len(), 1, "只命中 desk");
    key(&mut app, crossterm::event::KeyCode::Enter); // 收词退输入态
    assert!(!app.ah_filter_on, "Enter 收词");
    assert_eq!(app.ah_visible().len(), 1, "词仍生效");
    key(&mut app, crossterm::event::KeyCode::Esc); // 视图内 Esc = 返航(不过滤输入态)
    assert!(app.as_history.is_none(), "无过滤输入态时 Esc 返航");
}

#[test]
fn 装配历史_过滤无命中给可区分空态() {
    let mut app = history_app("filter-empty");
    key(&mut app, crossterm::event::KeyCode::Char('/'));
    for c in ['z', 'z', 'z'] {
        key(&mut app, crossterm::event::KeyCode::Char(c));
    }
    key(&mut app, crossterm::event::KeyCode::Enter);
    assert!(app.ah_visible().is_empty(), "无命中");
    let f = draw(&app, 110, 30);
    assert!(f.contains("没有派命中过滤词"), "空态与真空册可区分:{f}");
}

// ---- #115 反向溯源:派落点(纯函数 + 选中行快照) ----

/// 本地组件的落点 = `nw/components/<组件名>/`:上游 external-sources.md
/// 「本地组件的 skill 仍住 nw/components/<组件>/ 里 —— 机制不同,家就不同」
/// (2026-10-03 家长裁定)。纯函数,零 IO;规则只读知识,面板不另造。
#[test]
fn 落点纯函数_本地组件落nw_components() {
    assert_eq!(
        super::recipes::local_component_landing("retention"),
        "nw/components/retention/"
    );
}

/// 外部 skill 的落点 = `skills/<仓库名>/<条目名>/`:上游 external-sources.md
/// 「选中的 skill 铺到 <派目录>/skills/<仓库名>/<skill 名>/ —— 仓库名就是
/// 命名空间」;锁定集 landing.path 同形(`<agent-dir>/skills/hello-world/
/// web-scout`,kind `pi-skill`)。
#[test]
fn 落点纯函数_外部skill落skills_仓库名是命名空间() {
    assert_eq!(
        super::recipes::external_entry_landing("hello-world", "web-scout", Some("skill")),
        "skills/hello-world/web-scout/"
    );
}

/// 外部 extension 的落点 = `nw/extensions/<仓库名>/<条目名>/`:上游
/// external-sources.md「扩展这半」§1 —— 与本地组件同一屋檐(`nw/`)但
/// **不进 nw/components/**(外部扩展本体没有 component.yaml,塞进去会让
/// up 步 3 的拓扑与 pruneRemoved 的语义变味)。
#[test]
fn 落点纯函数_外部extension落nw_extensions() {
    assert_eq!(
        super::recipes::external_entry_landing("theme-pack", "ribbon", Some("extension")),
        "nw/extensions/theme-pack/ribbon/"
    );
}

/// 选中行 → 落点快照(#115):components 域恒为本地组件(上游 ls.go 的
/// readComponents 只扫 <root>/components/<名>/component.yaml,外部条目住在
/// sources 域);sources 域一条记录 = 一个仓库 + 条目清单,按 kind 分派落点。
/// 无落点语义的域(recipes/profiles/locks)不给快照。
#[test]
fn 落点快照_组件域取本地_源域取外部条目_其它域无() {
    let ls: crate::engine::LsData = serde_json::from_str(
        r#"{"domain":"all","recipes":[{"name":"r1"}],"components":[{"name":"retention"},{"name":"scout"}],"sources":[{"name":"hello-world","repo":"https://github.com/o/hello-world","kind":"skill","entries":[{"name":"web-scout","path":"skills/web-scout"}]},{"name":"theme-pack","kind":"extension","entries":[{"name":"ribbon","path":"extensions/ribbon"}]}],"profiles":[],"locks":[]}"#,
    )
    .unwrap();
    let mut app = fake_app();
    app.ls = Some(ls);
    // components 域:选中第二行(scout)
    app.rc_domain = crate::app::Domain::Components;
    app.rc_sel = 1;
    assert!(matches!(
        super::recipes::landing_trace(&app),
        Some(super::recipes::LandingTrace::Local { name }) if name == "scout"
    ));
    // sources 域:第一条记录(hello-world / skill)
    app.rc_domain = crate::app::Domain::Sources;
    app.rc_sel = 0;
    match super::recipes::landing_trace(&app) {
        Some(super::recipes::LandingTrace::External {
            repo,
            kind,
            entries,
        }) => {
            assert_eq!(repo, "hello-world");
            assert_eq!(kind.as_deref(), Some("skill"));
            assert_eq!(entries, vec!["web-scout".to_string()]);
        }
        other => panic!("sources 域应给外部快照,得到 {other:?}"),
    }
    // 无落点语义的域
    app.rc_domain = crate::app::Domain::Profiles;
    assert!(
        super::recipes::landing_trace(&app).is_none(),
        "profiles 域无落点语义"
    );
    app.rc_domain = crate::app::Domain::Locks;
    assert!(
        super::recipes::landing_trace(&app).is_none(),
        "locks 域无落点语义"
    );
    app.rc_domain = crate::app::Domain::Recipes;
    assert!(
        super::recipes::landing_trace(&app).is_none(),
        "recipes 域无落点语义"
    );
}

/// 过滤后选中行跟随「看得见的那一行」(#61 同源):过滤词把 scout 筛掉后,
/// 原下标 1 不再有落点快照。
#[test]
fn 落点快照_过滤后取看得见的那一行() {
    let mut app = fake_app();
    app.ls = Some(
        serde_json::from_str(
            r#"{"domain":"all","recipes":[],"components":[{"name":"retention"},{"name":"scout"}],"sources":[],"profiles":[],"locks":[]}"#,
        )
        .unwrap(),
    );
    app.rc_domain = crate::app::Domain::Components;
    app.rc_sel = 1;
    app.rc_filter = "retention".into();
    assert!(
        super::recipes::landing_trace(&app).is_none(),
        " scout 被过滤掉:原下标不再有快照(与详情取同一行)"
    );
    app.rc_sel = 0;
    assert!(matches!(
        super::recipes::landing_trace(&app),
        Some(super::recipes::LandingTrace::Local { name }) if name == "retention"
    ));
}

// ---- #115 落点浮层:两类分列的内容与两档快照(纯呈现) ----

/// 规格验收 1 / 故事 19:选中本地组件,浮层两类分列 —— 本地类摊实指
/// (`nw/components/<名>/`,上游 external-sources.md「本地组件的 skill 仍住
/// nw/components/<组件>/ 里」),外部类给上游规则作对照:两类落点差异一目了然。
#[test]
fn 落点浮层_本地组件_实指与外部规则分列() {
    let mut app = fake_app();
    app.screen = Screen::Recipes;
    app.ls = Some(serde_json::from_str(LOCAL_LS).unwrap());
    app.rc_domain = crate::app::Domain::Components;
    app.rc_landing = Some(super::recipes::LandingTrace::Local {
        name: "retention".into(),
    });
    let s = draw(&app, 110, 30);
    assert!(
        s.contains("nw/components/retention/"),
        "本地组件实指落点:{s}"
    );
    assert!(
        s.contains("skills/<仓库名>/<条目名>/"),
        "外部 skill 规则作对照:{s}"
    );
    assert!(
        s.contains("nw/extensions/<仓库名>/<条目名>/"),
        "外部 extension 规则作对照:{s}"
    );
    assert!(s.contains("Esc 或点击任意处关闭"), "收口给返航路:{s}");
}

/// 规格验收 1 / 故事 18:选中外部 skill 源,条目逐个落
/// `skills/<仓库名>/<条目名>/`(仓库名即命名空间,上游 2026-10-03 裁定);
/// 本地类规则作对照。一条记录 = 一个仓库 + 一次扫描的条目清单。
#[test]
fn 落点浮层_外部源skill_条目逐个落点() {
    let mut app = fake_app();
    app.screen = Screen::Recipes;
    app.rc_landing = Some(super::recipes::LandingTrace::External {
        repo: "hello-world".into(),
        kind: Some("skill".into()),
        entries: vec!["web-scout".into(), "doc-lookup".into()],
    });
    let s = draw(&app, 110, 30);
    assert!(
        s.contains("skills/hello-world/web-scout/"),
        "skill 条目落点:{s}"
    );
    assert!(
        s.contains("skills/hello-world/doc-lookup/"),
        "同仓第二条条目同规则:{s}"
    );
    assert!(
        s.contains("nw/components/<组件名>/"),
        "本地类规则作对照:{s}"
    );
}

/// 规格验收 1:外部 extension 落 `nw/extensions/<仓库名>/<条目名>/` —— 上游
/// external-sources.md「扩展这半」§1:与本地组件同一屋檐但不进 components/。
#[test]
fn 落点浮层_外部源extension_落nw_extensions() {
    let mut app = fake_app();
    app.screen = Screen::Recipes;
    app.rc_landing = Some(super::recipes::LandingTrace::External {
        repo: "theme-pack".into(),
        kind: Some("extension".into()),
        entries: vec!["ribbon".into()],
    });
    let s = draw(&app, 110, 30);
    assert!(
        s.contains("nw/extensions/theme-pack/ribbon/"),
        "extension 条目落点:{s}"
    );
}

/// kind 缺省不静默冒充:标题明说「kind 未给出」,落点按 skill 呈现
/// (上游编目两条数组必居其一;面板不猜,只把「按什么呈现」摊开)。
#[test]
fn 落点浮层_外部源kind缺失_明说按skill落点() {
    let mut app = fake_app();
    app.screen = Screen::Recipes;
    app.rc_landing = Some(super::recipes::LandingTrace::External {
        repo: "bare".into(),
        kind: None,
        entries: vec!["x".into()],
    });
    let s = draw(&app, 110, 30);
    assert!(s.contains("kind 未给出"), "kind 缺失明说:{s}");
    assert!(s.contains("skills/bare/x/"), "按 skill 落点呈现:{s}");
}

/// 规格验收 4:常畅(110×30)与紧凑(66×50)两档浮层都在帧上、不炸。
#[test]
fn 落点浮层_两档快照() {
    let mut app = fake_app();
    app.screen = Screen::Recipes;
    app.ls = Some(serde_json::from_str(LOCAL_LS).unwrap());
    app.rc_domain = crate::app::Domain::Components;
    app.rc_landing = Some(super::recipes::LandingTrace::Local {
        name: "retention".into(),
    });
    for (w, h) in [(110u16, 30u16), (66, 50)] {
        app.splash = false;
        let f = crate::testutil::frame_display(&app, w, h);
        assert!(
            f.contains("nw/components/retention/"),
            "{w}×{h} 本地落点:{f}"
        );
        assert!(f.contains("Esc 或点击任意处关闭"), "{w}×{h} 收口在帧:{f}");
    }
}

// ---- #115 落点浮层:键位 / 鼠标行为(纯呈现可证) ----

/// 落点浮层行为夹具:数据仓根指向真实可读临时目录(三道闸放行)、五域有料、
/// FakeRunner 可见 —— 这些测试要能证明「零引擎调用」。
fn landing_app() -> (Arc<FakeRunner>, App) {
    let dir = std::env::temp_dir().join(format!("nwt-landing-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let runner = Arc::new(FakeRunner::new(&[(
        "ls",
        0,
        r#"{"schema":"nw/ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}}"#,
    )]));
    let mut app = App::with_runner(runner.clone());
    app.screen = Screen::Recipes;
    app.data_root = Some(dir.to_string_lossy().to_string());
    app.ls = Some(serde_json::from_str(LOCAL_LS).unwrap());
    (runner, app)
}

/// 规格验收 2:Enter 打开浮层 = 纯呈现 —— 一个引擎调用都不发、一个字节都
/// 不入队(落点规则是面板侧只读知识,不读派目录、不代跑)。
#[test]
fn 落点浮层_enter打开_零引擎调用零入队() {
    let (runner, mut app) = landing_app();
    app.rc_domain = crate::app::Domain::Components;
    key(&mut app, crossterm::event::KeyCode::Enter);
    assert!(app.landing_open(), "Enter 打开落点浮层");
    assert!(
        matches!(&app.rc_landing, Some(super::recipes::LandingTrace::Local { name }) if name == "retention"),
        "快照取选中行(第一行 retention)"
    );
    assert!(
        runner.call_argvs().is_empty(),
        "零引擎调用:{:?}",
        runner.call_argvs()
    );
    assert!(app.queue.is_empty(), "零入队");
}

/// 外部源域 Enter 给外部快照(一条记录 = 一个仓库 + 条目清单,按 kind 分派)。
#[test]
fn 落点浮层_外部源域enter给外部快照() {
    let (_r, mut app) = landing_app();
    app.rc_domain = crate::app::Domain::Sources;
    app.rc_sel = 1; // theme-pack(extension)
    key(&mut app, crossterm::event::KeyCode::Enter);
    match &app.rc_landing {
        Some(super::recipes::LandingTrace::External {
            repo,
            kind,
            entries,
        }) => {
            assert_eq!(repo, "theme-pack");
            assert_eq!(kind.as_deref(), Some("extension"));
            assert_eq!(entries, &vec!["ribbon".to_string()]);
        }
        other => panic!("应给外部快照,得到 {other:?}"),
    }
    assert!(_r.call_argvs().is_empty(), "零引擎调用");
}

/// 三道门各说各话:无落点语义的域(recipes/profiles/locks)、没有选中行、
/// 数据仓根未设 —— Enter 静默不摆浮层(不缺能力、不发调用、不摆空浮层)。
#[test]
fn 落点浮层_三道门_不满足不摆() {
    // 无落点语义的域
    for d in [
        crate::app::Domain::Recipes,
        crate::app::Domain::Profiles,
        crate::app::Domain::Locks,
    ] {
        let (_r, mut app) = landing_app();
        app.rc_domain = d;
        key(&mut app, crossterm::event::KeyCode::Enter);
        assert!(!app.landing_open(), "{d:?} 域无落点语义,不摆浮层");
    }
    // 没有选中行(components 域被过滤空)
    let (_r, mut app) = landing_app();
    app.rc_domain = crate::app::Domain::Components;
    app.rc_filter = "查无此件".into();
    key(&mut app, crossterm::event::KeyCode::Enter);
    assert!(!app.landing_open(), "无选中行不摆浮层");
    // 数据仓根未设:与五域同门,不摆(也不拿落点冒充)
    let dir = std::env::temp_dir().join(format!("nwt-landing-noroot-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let runner = Arc::new(FakeRunner::new(&[]));
    let mut app = App::with_runner(runner);
    app.screen = Screen::Recipes;
    app.data_root = None;
    app.ls = Some(serde_json::from_str(LOCAL_LS).unwrap());
    app.rc_domain = crate::app::Domain::Components;
    key(&mut app, crossterm::event::KeyCode::Enter);
    assert!(!app.landing_open(), "数据仓根未设不摆浮层");
}

/// 规格验收 3:Esc 返航只收浮层状态 —— 配方屏选中行原样在场(浮层读的是
/// 打开那一刻的快照,返航不顺手改 rc_sel/rc_domain)。
#[test]
fn 落点浮层_esc返航_选中行原样在场() {
    let (_r, mut app) = landing_app();
    app.rc_domain = crate::app::Domain::Components;
    app.rc_sel = 1;
    key(&mut app, crossterm::event::KeyCode::Enter);
    assert!(app.landing_open());
    key(&mut app, crossterm::event::KeyCode::Esc);
    assert!(!app.landing_open(), "Esc 关浮层");
    assert_eq!(app.rc_sel, 1, "选中行原样在场");
    assert_eq!(app.rc_domain, crate::app::Domain::Components, "域原样在场");
}

/// 规格验收 3:浮层在场时吞键 —— 五域键位(h 切域 / ↑↓ 选行 / c 建议命令)
/// 一个都不受理,与反查浮层同一条弹层纪律;再按 Esc 才返航。
#[test]
fn 落点浮层_在场吞键_五域键位不受理() {
    let (_r, mut app) = landing_app();
    app.rc_domain = crate::app::Domain::Components;
    key(&mut app, crossterm::event::KeyCode::Enter);
    let sel = app.rc_sel;
    key(&mut app, crossterm::event::KeyCode::Char('h'));
    key(&mut app, crossterm::event::KeyCode::Down);
    key(&mut app, crossterm::event::KeyCode::Char('c'));
    assert_eq!(app.rc_sel, sel, "浮层吞键:选中行不动");
    assert_eq!(
        app.rc_domain,
        crate::app::Domain::Components,
        "浮层吞键:不切域"
    );
    assert!(!app.rc_bake_sel, "浮层吞键:c 不受理");
    assert!(app.landing_open(), "非 Esc 键不返航");
}

/// 规格验收 3:鼠标点任意处关闭(与键盘 Esc 一个后果)—— 命中区取整块正文,
/// 点浮层外的空白也算返航(与 #70 反查浮层同一条弹层纪律)。
#[test]
fn 落点浮层_点任意处关闭_整块正文都是返航命中() {
    let (_r, mut app) = landing_app();
    app.rc_domain = crate::app::Domain::Components;
    key(&mut app, crossterm::event::KeyCode::Enter);
    app.size = ratatui::layout::Rect::new(0, 0, 110, 30);
    let class = crate::layout::size_class(app.size);
    let hits = crate::layout::hitmap(app.size, &app, class);
    assert!(
        hits.iter()
            .any(|(_, t)| *t == crate::layout::Target::DismissPopup),
        "浮层在场:整块正文产「点任意处返航」命中"
    );
    assert!(
        hits.iter()
            .all(|(_, t)| !matches!(t, crate::layout::Target::SelectRow(_))),
        "浮层吞点击:底层列表行不产命中(不穿透)"
    );
    // 点命中区中心(浮层外空白)= 返航(坐标取自布局模型,不硬编码)
    let dismiss = hits
        .iter()
        .find(|(_, t)| *t == crate::layout::Target::DismissPopup)
        .map(|(r, _)| *r)
        .expect("上面已断言有 DismissPopup 命中");
    app.update(Message::Mouse(crossterm::event::MouseEvent {
        kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
        column: dismiss.x + dismiss.width / 2,
        row: dismiss.y + dismiss.height / 2,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }));
    assert!(!app.landing_open(), "点任意处关闭");
}

/// 外部源记录还没有扫出条目(register 预审时才有):明说,不画成空列表也不
/// 拿本地落点冒充;本地类规则仍在(两类分列的读法不因条目空而缺一半)。
#[test]
fn 落点浮层_外部源无条目_明说与规则对照() {
    let mut app = fake_app();
    app.screen = Screen::Recipes;
    app.rc_landing = Some(super::recipes::LandingTrace::External {
        repo: "tiny-skills".into(),
        kind: Some("skill".into()),
        entries: vec![],
    });
    let s = draw(&app, 110, 30);
    assert!(s.contains("该仓库还没有扫出条目"), "无条明说:{s}");
    assert!(
        s.contains("nw/components/<组件名>/"),
        "本地类规则仍作对照:{s}"
    );
}

/// 一仓扫出超过上限的条目:浮层不无限长,摊前 6 条 + 明说其余条数
/// (与健康屏证据面板上限同一种纪律)。
#[test]
fn 落点浮层_外部源条目超上限_明说其余条数() {
    let mut app = fake_app();
    app.screen = Screen::Recipes;
    let entries: Vec<String> = (0..9).map(|i| format!("e{i}")).collect();
    app.rc_landing = Some(super::recipes::LandingTrace::External {
        repo: "many".into(),
        kind: Some("skill".into()),
        entries: entries.clone(),
    });
    let s = draw(&app, 110, 30);
    assert!(s.contains("skills/many/e0/"), "首条在:{s}");
    assert!(s.contains("skills/many/e5/"), "第 6 条在:{s}");
    assert!(!s.contains("skills/many/e6/"), "第 7 条不进浮层:{s}");
    assert!(s.contains("…其余 3 条"), "超出明说条数:{s}");
}

// ---- 组件打包器(spec #95 工单 #112):fixture 目录进、组件清单出 ----

/// 打包夹具数据仓:一个本地 skill 目录 + 一个 extension 目录(带 node_modules
/// 与 .git 脏树,验证排除表)。
fn pack_fixture(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("nwt-pack-fix-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    let skill = dir.join("payloads/web-scout");
    std::fs::create_dir_all(skill.join("references")).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: web-scout\ndescription: 检索\n---\n",
    )
    .unwrap();
    std::fs::write(skill.join("references/triggers.md"), "查一下\n").unwrap();
    std::fs::create_dir_all(skill.join("node_modules/zod")).unwrap();
    std::fs::write(skill.join("node_modules/zod/index.js"), "x\n").unwrap();
    std::fs::create_dir_all(skill.join(".git")).unwrap();
    std::fs::write(skill.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    dir
}

/// 打包 App:数据仓根指夹具(写侧只认根,不认 NW_HOME)。
fn pack_app(tag: &str) -> App {
    let mut app = fake_app();
    app.screen = Screen::Recipes;
    app.splash = false;
    app.data_root = Some(pack_fixture(tag).to_string_lossy().to_string());
    app
}

/// 填三字段并 Enter(走到确认闸)。
fn pack_fill(app: &mut App, source: &str, name: &str, version: &str) {
    app.rc_domain = crate::app::Domain::Components;
    if app.pack_open() {
        key(app, crossterm::event::KeyCode::Esc); // 不重入:先返航再开新的
    }
    key(app, crossterm::event::KeyCode::Char('p'));
    assert!(app.pack_open(), "组件域按 p 开打包表单");
    type_str(app, source);
    key(app, crossterm::event::KeyCode::Down);
    type_str(app, name);
    key(app, crossterm::event::KeyCode::Down);
    type_str(app, version);
    key(app, crossterm::event::KeyCode::Enter);
}

/// 夹具里那棵 skill 树的真校验和(与 `sha256sum` 同算法的目录摘要,含拟落盘的
/// 清单)——由测试侧按上游 DirHash 同形独立算一遍,不当「面板自己说对」的证据。
fn expected_digest(root: &std::path::Path) -> String {
    let payload = root.join("payloads/web-scout");
    let mut lines: Vec<String> = vec![
        "SKILL.md".to_string(),
        "references/triggers.md".to_string(),
        "component.yaml".to_string(),
    ]
    .into_iter()
    .map(|rel| {
        let bytes = if rel == "component.yaml" {
            crate::datarepo::pack::component_manifest("web-scout", "1.0.0").into_bytes()
        } else {
            std::fs::read(payload.join(&rel)).unwrap()
        };
        format!("{rel} {}", crate::datarepo::pack::sha256_hex(&bytes))
    })
    .collect();
    lines.sort();
    let mut text = String::new();
    for l in &lines {
        text.push_str(l);
        text.push('\n');
    }
    crate::datarepo::pack::sha256_hex(text.as_bytes())
}

/// 规格验收 1 + 4:fixture 目录进、组件清单出,字节确定;清单键序与数据仓现有
/// 组件样本一致(样本键一个不少)。
#[test]
fn 打包_表单进确认闸_字节确定且清单形状对齐样本() {
    let mut app = pack_app("bytes");
    let root = std::path::PathBuf::from(app.data_root.clone().unwrap());
    pack_fill(&mut app, "payloads/web-scout", "web-scout", "1.0.0");
    let plan = app
        .pk_flow
        .as_ref()
        .and_then(|f| f.plan.clone())
        .expect("Enter 后进确认闸,拟落账在场");
    // 确认前一个字节都不落:清单还没写
    assert!(
        !root.join("payloads/web-scout/component.yaml").exists(),
        "确认前不落盘(拟落账只算不写)"
    );
    // 清单形状:样本键序逐行对齐
    let keys: Vec<&str> = plan
        .manifest
        .lines()
        .map(|l| l.split(':').next().unwrap())
        .collect();
    assert_eq!(
        keys,
        vec![
            "apiVersion",
            "kind",
            "name",
            "version",
            "pi",
            "packages",
            "vendored",
            "depends",
            "params",
            "config",
            "env",
            "requires"
        ],
        "清单键序与样本(nw-journeys-and-docs component.yaml)一致"
    );
    assert!(plan.manifest.contains("apiVersion: nw/v1"));
    assert!(plan.manifest.contains("kind: component"));
    assert!(plan.manifest.contains("name: web-scout"));
    assert!(plan.manifest.contains("version: 1.0.0"));
    // 校验和:与测试侧按上游同算法独立算的一致(排除表生效:.git/node_modules
    // 不进,3 个文件 = SKILL.md + references/triggers.md + 拟落盘的清单)
    assert_eq!(plan.files, 2, "排除表生效:.git/ 与 node_modules/ 不算");
    assert_eq!(plan.digest, expected_digest(&root), "校验和与独立重算一致");
    assert_eq!(plan.target, "payloads/web-scout/component.yaml");
}

/// 规格验收 2:确认前不落盘 —— 表单 Enter 只出拟落账;Esc 返航连拟落账一起丢,
/// 磁盘上仍然什么都没有。确认(Enter)之后才落,且只落那一个文件。
#[test]
fn 打包_确认前一个字节都不落_确认后原子落盘() {
    let mut app = pack_app("gate");
    let root = std::path::PathBuf::from(app.data_root.clone().unwrap());
    pack_fill(&mut app, "payloads/web-scout", "web-scout", "1.0.0");
    // 拟落账在手,磁盘干净
    assert_eq!(
        std::fs::read_dir(root.join("payloads/web-scout"))
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name() != "node_modules")
            .filter(|e| e.as_ref().unwrap().file_name() != ".git")
            .count(),
        2,
        "确认前:源目录原样(SKILL.md + references),没有 component.yaml"
    );
    // Esc 返航:不落
    key(&mut app, crossterm::event::KeyCode::Esc);
    assert!(!app.pack_open(), "Esc 收起浮层");
    assert!(
        !root.join("payloads/web-scout/component.yaml").exists(),
        "返航不落盘(拟落账随浮层一起丢)"
    );
    // 再来一次,这次确认
    pack_fill(&mut app, "payloads/web-scout", "web-scout", "1.0.0");
    key(&mut app, crossterm::event::KeyCode::Enter);
    let written = std::fs::read_to_string(root.join("payloads/web-scout/component.yaml")).unwrap();
    let plan = app.pk_flow.as_ref().unwrap().plan.clone().unwrap();
    assert_eq!(
        written, plan.manifest,
        "落盘字节 = 确认闸摊开过的拟生成清单(逐字节相同)"
    );
    // 幂等:再打一次,清单与校验和都不漂
    let before = plan.digest.clone();
    pack_fill(&mut app, "payloads/web-scout", "web-scout", "1.0.0");
    let again = app.pk_flow.as_ref().unwrap().plan.clone().unwrap();
    assert_eq!(again.digest, before, "同输入同校验和(可重复)");
    assert_eq!(again.manifest, plan.manifest, "同输入同清单(可重复)");
    // 源目录里没被顺手多写东西
    assert!(!root.join("payloads/web-scout/SKILL.md.nwt-tmp").exists());
    std::fs::remove_dir_all(&root).ok();
}

/// 规格验收 2 的鼠标一侧:点「Enter 落盘」段 = 落盘(与键盘同一条路),
/// 点「Esc 返航」段 = 收起不落。
#[test]
fn 打包_确认闸键位行两段_点哪打哪() {
    for (w, h) in [(80u16, 30u16), (66, 50)] {
        let mut app = pack_app(&format!("mouse-{w}x{h}"));
        let root = std::path::PathBuf::from(app.data_root.clone().unwrap());
        pack_fill(&mut app, "payloads/web-scout", "web-scout", "1.0.0");
        let f = crate::testutil::frame_display(&app, w, h);
        assert!(f.contains("Enter 落盘"), "{w}×{h} 确认段左段在场:{f}");
        assert!(f.contains("Esc 返航"), "{w}×{h} 确认段右段在场:{f}");
        // 点左段:坐标取自布局模型的 Enter 命中区,并先断言屏上「Enter 落盘」
        // 那几个字确落进该区(渲染与命中共源,ADR-0005)。
        app.size = ratatui::layout::Rect::new(0, 0, w, h);
        let hits = crate::layout::hitmap(app.size, &app, crate::layout::size_class(app.size));
        let act = hits
            .iter()
            .find(|(_, t)| *t == crate::layout::Target::Activate)
            .map(|(r, _)| *r)
            .expect("确认段产「Enter 落盘」命中区");
        let (row, col) = locate(&f, "Enter 落盘");
        assert!(
            act.contains(ratatui::layout::Position::new(col, row)),
            "{w}×{h} 渲染与命中共源:屏上「Enter 落盘」落进 Enter 命中区"
        );
        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: col,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }));
        assert!(
            root.join("payloads/web-scout/component.yaml").exists(),
            "{w}×{h} 点左段 = 落盘(与键盘 Enter 同路)"
        );
        assert!(matches!(
            app.pk_flow.as_ref().map(|f| f.phase),
            Some(super::recipes::PackPhase::Done)
        ));
        // 结果屏点浮层内任意处 = 返航(坐标取自命中区)
        let dismiss = crate::layout::hitmap(app.size, &app, crate::layout::size_class(app.size))
            .into_iter()
            .find(|(_, t)| *t == crate::layout::Target::DismissPopup)
            .map(|(r, _)| r)
            .expect("结果屏整块是返航命中");
        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: dismiss.x + dismiss.width / 2,
            row: dismiss.y + dismiss.height / 2,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }));
        assert!(!app.pack_open(), "{w}×{h} 结果屏点任意处返航");
        std::fs::remove_dir_all(&root).ok();
    }
}

/// 规格验收 5:常畅 / 紧凑两档快照 —— 校验和(64 hex 不截断)、拟生成清单、
/// 将写文件、键位行都在阵;紧凑档明说只预览前几行(两档不同副面孔)。
#[test]
fn 打包_确认闸两档快照() {
    let mut app = pack_app("snap");
    pack_fill(&mut app, "payloads/web-scout", "web-scout", "1.0.0");
    let digest = app
        .pk_flow
        .as_ref()
        .unwrap()
        .plan
        .as_ref()
        .unwrap()
        .digest
        .clone();
    let comfy = crate::testutil::frame_display(&app, 80, 30);
    assert!(comfy.contains("组件打包器"), "常畅标题:{comfy}");
    assert!(comfy.contains("payloads/web-scout"), "常畅源目录:{comfy}");
    assert!(
        comfy.contains(&digest[..32]) && comfy.contains(&digest[32..]),
        "常畅校验和完整 64 hex 不截断(放不下就折行):{comfy}"
    );
    assert!(
        comfy.contains("apiVersion: nw/v1"),
        "常畅摊开拟生成清单:{comfy}"
    );
    assert!(comfy.contains("requires: []"), "常畅清单末行在:{comfy}");
    assert!(comfy.contains("将写"), "常畅将写文件行:{comfy}");
    assert!(comfy.contains("2 个文件"), "常畅体量行:{comfy}");
    let compact = crate::testutil::frame_display(&app, 66, 50);
    assert!(compact.contains("组件打包器"), "紧凑标题:{compact}");
    assert!(
        compact.contains(&digest[..32]) && compact.contains(&digest[32..]),
        "紧凑校验和也完整(折行不截断):{compact}"
    );
    assert!(
        compact.contains("apiVersion: nw/v1"),
        "紧凑也摊清单:{compact}"
    );
    assert!(compact.contains("…其余"), "紧凑明说只预览前几行:{compact}");
    assert!(compact.contains("Enter 落盘"), "紧凑键位行:{compact}");
    assert!(!compact.contains("panic"), "两档不炸:{compact}");
    std::fs::remove_dir_all(std::path::PathBuf::from(app.data_root.clone().unwrap())).ok();
}

/// 表单两档快照:三字段 + 当前字段块光标 + 键位提示。
#[test]
fn 打包_表单两档快照() {
    let mut app = pack_app("form-snap");
    app.rc_domain = crate::app::Domain::Components;
    key(&mut app, crossterm::event::KeyCode::Char('p'));
    type_str(&mut app, "payloads/web-scout");
    for (w, h) in [(80u16, 30u16), (66, 50)] {
        let f = crate::testutil::frame_display(&app, w, h);
        assert!(f.contains("源目录"), "{w}×{h} 源目录字段:{f}");
        assert!(f.contains("组件名"), "{w}×{h} 组件名字段:{f}");
        assert!(f.contains("payloads/web-scout"), "{w}×{h} 键入内容:{f}");
        assert!(f.contains("Enter 算校验和"), "{w}×{h} 表单键位行:{f}");
        assert!(!f.contains("panic"), "{w}×{h} 不炸:{f}");
    }
    std::fs::remove_dir_all(std::path::PathBuf::from(app.data_root.clone().unwrap())).ok();
}

/// 面板侧就拒:坏形状的组件名 / 版本 / 不在的目录,留表单可改,一个字节都不落,
/// 也不喂给写侧跑一趟(拒绝理由在表单上摊开)。
#[test]
fn 打包_形状不合_留表单可改且零字节() {
    let mut app = pack_app("reject");
    let root = std::path::PathBuf::from(app.data_root.clone().unwrap());
    pack_fill(&mut app, "payloads/web-scout", "web/scout", "1.0.0");
    assert!(
        matches!(
            app.pk_flow.as_ref().map(|f| f.phase),
            Some(super::recipes::PackPhase::Form)
        ),
        "被拒留在表单,不进确认闸"
    );
    assert!(
        app.pk_flow.as_ref().unwrap().err.is_some(),
        "拒件人话摊在表单上"
    );
    assert!(
        !root.join("payloads/web-scout/component.yaml").exists(),
        "拒了不落"
    );
    // 版本形状也不合(同一处表单,改完就能再试)
    pack_fill(&mut app, "payloads/web-scout", "web-scout", "latest");
    assert!(app.pk_flow.as_ref().unwrap().err.is_some(), "版本也拒");
    // 目录不在
    pack_fill(&mut app, "payloads/nope", "web-scout", "1.0.0");
    assert!(app.pk_flow.as_ref().unwrap().err.is_some(), "目录不在也拒");
    assert!(
        !root.join("payloads/web-scout/component.yaml").exists(),
        "三种拒件都不落盘"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// 三道门:数据仓根不就绪不摆(只留指路,不猜当前目录)、装配互斥、不重入。
#[test]
fn 打包_三道门_根不就绪与装配互斥都不开浮层() {
    let mut app = pack_app("gates");
    app.data_root = None;
    app.rc_domain = crate::app::Domain::Components;
    key(&mut app, crossterm::event::KeyCode::Char('p'));
    assert!(!app.pack_open(), "数据仓根未设:指路,不开表单");
    assert!(
        app.log[0].contains("数据仓"),
        "未设根只留指路,不开表单:{:?}",
        app.log
    );

    let mut app2 = pack_app("gates2");
    app2.assembling = true;
    app2.rc_domain = crate::app::Domain::Components;
    key(&mut app2, crossterm::event::KeyCode::Char('p'));
    assert!(!app2.pack_open(), "装配互斥(写互斥,ADR-0010 §5)");
    // 非组件域静默不摆(与 `r` 只外部源域同构)
    let mut app3 = pack_app("gates3");
    app3.rc_domain = crate::app::Domain::Recipes;
    key(&mut app3, crossterm::event::KeyCode::Char('p'));
    assert!(!app3.pack_open(), "非组件域不给打包入口");
    for a in [&app, &app2, &app3] {
        if let Some(root) = a.data_root.as_ref() {
            std::fs::remove_dir_all(root).ok();
        }
    }
}

/// 浮层在场吞键:五域键位(切域 / 选行 / 过滤 / 落点 / 建议命令)一个都不受理,
/// 配方屏原选中行原样不动。
#[test]
fn 打包_浮层吞键_五域键位不受理() {
    let mut app = pack_app("swallow");
    app.ls = Some(serde_json::from_str(LOCAL_LS).unwrap());
    app.rc_domain = crate::app::Domain::Components;
    key(&mut app, crossterm::event::KeyCode::Char('p'));
    let before = (app.rc_domain, app.rc_sel, app.rc_filter.clone());
    for code in [
        crossterm::event::KeyCode::Char('h'),
        crossterm::event::KeyCode::Char('l'),
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyCode::Up,
        crossterm::event::KeyCode::Char('/'),
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyCode::Enter,
    ] {
        key(&mut app, code);
    }
    assert_eq!(
        (app.rc_domain, app.rc_sel, app.rc_filter.clone()),
        before,
        "浮层吞键:五域状态一个都不改(Enter 归表单段)"
    );
    std::fs::remove_dir_all(std::path::PathBuf::from(app.data_root.clone().unwrap())).ok();
}

// ---- 配方编辑器(spec #95 工单 #113)----

/// 夹具配方:带未知字段(顶层 metadata/labels、组件项 notes/tuning、env 项
/// purpose)——往返保真的断言基准。
const RECIPE_SAMPLE: &str = "apiVersion: nw/v1\n\
kind: recipe\n\
name: research-desk\n\
version: 0.2.0\n\
profile: desk\n\
runtime:\n  node: 22\n  pi: 0.84\n  chezmoi: 2\n\
components:\n  - component: scout\n    version: 1.0.0\n    notes: 主检索\n  - component: hello-world\n    source: external\n    tuning:\n      temperature: 0.3\n\
models:\n  defaults:\n    id: base\n  providers: []\n\
env:\n  - name: OPENAI_API_KEY\n    required: true\n    usedBy: [openai]\n    purpose: 调大模型\n  - name: OPTIONAL_FLAG\n    required: false\n\
metadata:\n  owner: me\nlabels:\n  tier: prod\n";

/// ls 回音的 **data 对象**(直接塞 `app.ls`,同 `LS_FULL` 先例):一条带 path
/// 的配方行(配方编辑器要从数据仓读它)。
const LS_RECIPE_EDIT: &str = r#"{"domain":"all","recipes":[{"name":"research-desk","version":"0.2.0","path":"recipes/research-desk/recipe.yaml"}],"components":[],"sources":[],"profiles":[],"locks":[]}"#;

/// 预跑 validate 的两态回音(四态里 fail=0 才放行写回)。
const VALIDATE_PASS: &str = r#"{"schema":"nw/validate.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"target":"recipe.yaml","results":[{"id":"recipe.schema","status":"pass","message":"形状合"}],"summary":{"total":1,"pass":1,"warn":0,"fail":0,"skip":0}}}"#;
const VALIDATE_FAIL: &str = r#"{"schema":"nw/validate.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"组件引用不存在","hint":"补上组件或改引用","details":{}}],"warnings":[],"data":{"target":"recipe.yaml","results":[{"id":"recipe.components_exist","status":"fail","message":"组件不存在"}],"summary":{"total":1,"pass":0,"warn":0,"fail":1,"skip":0}}}"#;

/// 夹具数据仓:recipes/research-desk/recipe.yaml(带未知字段)。
fn recipe_fixture(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("nwt-recipe-fix-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    let recipe_dir = dir.join("recipes/research-desk");
    std::fs::create_dir_all(&recipe_dir).unwrap();
    std::fs::write(recipe_dir.join("recipe.yaml"), RECIPE_SAMPLE).unwrap();
    dir
}

/// 编辑器 App:数据仓根指夹具、ls 带配方行、假引擎认 ls + validate 两态。
fn recipe_app(tag: &str, validate_rc: i32, validate_json: &str) -> (App, Arc<FakeRunner>) {
    let ls_env = format!(
        r#"{{"schema":"nw/ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{LS_RECIPE_EDIT}}}"#
    );
    let runner = Arc::new(FakeRunner::new(&[
        ("ls", 0, ls_env.as_str()),
        ("validate", validate_rc, validate_json),
    ]));
    let mut app = App::with_runner(runner.clone());
    app.screen = Screen::Recipes;
    app.splash = false;
    app.rc_domain = crate::app::Domain::Recipes;
    app.rc_sel = 0;
    app.data_root = Some(recipe_fixture(tag).to_string_lossy().to_string());
    app.ls = Some(serde_json::from_str(LS_RECIPE_EDIT).unwrap());
    (app, runner)
}

/// 开编辑器(配方域按 e)。
fn open_editor(app: &mut App) {
    key(app, crossterm::event::KeyCode::Char('e'));
    assert!(app.recipe_edit_open(), "配方域按 e 开编辑器");
}

/// 驱动一组编辑:加组件 retention、删第一个组件 scout、把 OPTIONAL_FLAG 的
/// required 打开并设默认 on。
fn drive_edits(app: &mut App) {
    // 加组件 retention(组件区,a → 键入 → Enter)
    key(app, crossterm::event::KeyCode::Char('a'));
    type_str(app, "retention");
    key(app, crossterm::event::KeyCode::Enter);
    // 光标回到第一个组件(scout)删掉
    key(app, crossterm::event::KeyCode::Up);
    key(app, crossterm::event::KeyCode::Up);
    key(app, crossterm::event::KeyCode::Char('x'));
    // 切到 env 区,选中 OPTIONAL_FLAG,开 required、设默认
    key(app, crossterm::event::KeyCode::Tab);
    key(app, crossterm::event::KeyCode::Down);
    key(app, crossterm::event::KeyCode::Char(' '));
    key(app, crossterm::event::KeyCode::Enter);
    type_str(app, "on");
    key(app, crossterm::event::KeyCode::Enter);
}

/// 喂一次预跑 validate 回音(走 app 的假引擎解析点,与生产同一条路)。
async fn feed_validate(app: &mut App, runner: &FakeRunner) {
    let argv = app.queue.last().expect("队尾有 validate argv").clone();
    let o = runner.run(&argv).await;
    app.update(Message::Engine(EngineEvent::Finished(Box::new(o))));
}

/// 规格验收 1:加/删组件槽、改 env 的编辑往返不丢未知字段(fixture 断言)。
/// 全流程:编辑 → 拟落账+预跑(通过)→ 确认写回,落盘的 recipe.yaml 保留全部
/// 未知字段(metadata/labels/notes/tuning/purpose),编辑确已生效。
#[tokio::test]
async fn 配方编辑_加删槽改env_往返不丢未知字段() {
    let (mut app, runner) = recipe_app("roundtrip", 0, VALIDATE_PASS);
    let root = std::path::PathBuf::from(app.data_root.clone().unwrap());
    let recipe = root.join("recipes/research-desk/recipe.yaml");
    let original = std::fs::read_to_string(&recipe).unwrap();
    open_editor(&mut app);
    drive_edits(&mut app);
    key(&mut app, crossterm::event::KeyCode::Char('s'));
    assert!(
        matches!(
            app.rc_edit_flow.as_ref().map(|f| f.phase),
            Some(crate::screens::RecipeEditPhase::PreRun)
        ),
        "s 后进预跑在途"
    );
    // 预跑在途:recipe.yaml 仍未动,scratch 已暂存
    assert_eq!(
        std::fs::read_to_string(&recipe).unwrap(),
        original,
        "预跑不碰 recipe.yaml"
    );
    feed_validate(&mut app, &runner).await;
    assert!(
        matches!(
            app.rc_edit_flow.as_ref().map(|f| f.phase),
            Some(crate::screens::RecipeEditPhase::Review)
        ),
        "回音落地进确认态"
    );
    // 确认前仍不落盘
    assert_eq!(
        std::fs::read_to_string(&recipe).unwrap(),
        original,
        "确认前不落盘"
    );
    // Enter 写回
    key(&mut app, crossterm::event::KeyCode::Enter);
    assert!(
        matches!(
            app.rc_edit_flow.as_ref().map(|f| f.phase),
            Some(crate::screens::RecipeEditPhase::Done)
        ),
        "确认后写回进结论态"
    );
    let written = std::fs::read_to_string(&recipe).unwrap();
    // 编辑生效
    assert!(
        written.contains("component: retention"),
        "新组件槽在场:{written}"
    );
    assert!(!written.contains("component: scout"), "旧槽已删:{written}");
    assert!(
        written.contains("component: hello-world"),
        "存活槽还在:{written}"
    );
    assert!(written.contains("default: on"), "env 默认已设:{written}");
    // 未知字段一个不丢
    assert!(written.contains("metadata:"), "顶层未知块不丢:{written}");
    assert!(
        written.contains("owner: me"),
        "顶层未知子字段不丢:{written}"
    );
    assert!(written.contains("labels:"), "另一未知顶层块不丢:{written}");
    assert!(
        written.contains("tuning:"),
        "存活项里的未知块不丢:{written}"
    );
    assert!(
        written.contains("temperature: 0.3"),
        "未知块子字段不丢:{written}"
    );
    assert!(
        written.contains("purpose: 调大模型"),
        "env 未知字段不丢:{written}"
    );
    // scratch 已清
    assert!(
        !root
            .join("recipes/research-desk/.nwt-validate.yaml")
            .exists(),
        "预跑暂存已清"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// 规格验收 2:写回前预跑 argv 与健康屏 validate 同源(同一 `commands::validate_recipe`
/// 构造函数,即 `["validate","--recipe",<路径>]`;路径指向 scratch 编辑态,因 recipe.yaml
/// 确认前不动)。
#[tokio::test]
async fn 配方编辑_预跑argv与健康屏validate同源() {
    let (mut app, _runner) = recipe_app("argv", 0, VALIDATE_PASS);
    let root = app.data_root.clone().unwrap();
    open_editor(&mut app);
    drive_edits(&mut app);
    key(&mut app, crossterm::event::KeyCode::Char('s'));
    let argv = app.queue.last().expect("s 入队了 validate").clone();
    // 与健康屏同一构造函数:commands::validate_recipe(abs)
    let scratch_abs = format!(
        "{}/{}",
        root.trim_end_matches('/'),
        crate::datarepo::recipe::scratch_rel("recipes/research-desk/recipe.yaml")
    );
    assert_eq!(
        argv,
        crate::engine::commands::validate_recipe(&scratch_abs),
        "预跑 argv = commands::validate_recipe(scratch),与健康屏 validate 同源(同一构造函数)"
    );
    assert_eq!(argv[0], "validate", "命令族是 validate");
    assert_eq!(
        argv[1], "--recipe",
        "子形状是 --recipe(非 --lock / 非面板自造检查)"
    );
}

/// 规格验收 3:预跑失败可放弃写回,数据仓状态不变(recipe.yaml 逐字节原样,scratch 已清)。
#[tokio::test]
async fn 配方编辑_预跑失败可放弃_数据仓不变() {
    let (mut app, runner) = recipe_app("fail", 3, VALIDATE_FAIL);
    let root = std::path::PathBuf::from(app.data_root.clone().unwrap());
    let recipe = root.join("recipes/research-desk/recipe.yaml");
    let original = std::fs::read_to_string(&recipe).unwrap();
    open_editor(&mut app);
    drive_edits(&mut app);
    key(&mut app, crossterm::event::KeyCode::Char('s'));
    feed_validate(&mut app, &runner).await;
    // 预跑未过:写回闸关着
    assert!(
        matches!(
            app.rc_edit_flow.as_ref().map(|f| f.phase),
            Some(crate::screens::RecipeEditPhase::Review)
        ),
        "回音落地进确认态"
    );
    assert!(
        !app.rc_edit_flow.as_ref().unwrap().validate_pass,
        "预跑未过:validate_pass=false"
    );
    // Enter 不被受理(闸关着),Esc 放弃
    key(&mut app, crossterm::event::KeyCode::Enter);
    assert_eq!(
        std::fs::read_to_string(&recipe).unwrap(),
        original,
        "预跑未过时 Enter 不写回"
    );
    key(&mut app, crossterm::event::KeyCode::Esc);
    assert!(!app.recipe_edit_open(), "Esc 收起浮层");
    assert_eq!(
        std::fs::read_to_string(&recipe).unwrap(),
        original,
        "放弃写回:recipe.yaml 逐字节原样"
    );
    assert!(
        !root
            .join("recipes/research-desk/.nwt-validate.yaml")
            .exists(),
        "放弃也清 scratch"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// 规格验收 4:确认前不落盘 —— 拟落账 + 预跑通过都不写;只有 Enter 确认后才写。
#[tokio::test]
async fn 配方编辑_确认前一个字节都不落() {
    let (mut app, runner) = recipe_app("gate", 0, VALIDATE_PASS);
    let root = std::path::PathBuf::from(app.data_root.clone().unwrap());
    let recipe = root.join("recipes/research-desk/recipe.yaml");
    let original = std::fs::read_to_string(&recipe).unwrap();
    open_editor(&mut app);
    drive_edits(&mut app);
    key(&mut app, crossterm::event::KeyCode::Char('s'));
    feed_validate(&mut app, &runner).await;
    // 预跑通过、拟落账在手,但确认前一个字节都不落
    assert!(
        app.rc_edit_flow.as_ref().unwrap().plan.is_some(),
        "拟落账在手"
    );
    assert_eq!(
        std::fs::read_to_string(&recipe).unwrap(),
        original,
        "拟落账+预跑通过仍不落盘"
    );
    // Esc 返航(未确认):连拟落账一起丢,磁盘干净
    key(&mut app, crossterm::event::KeyCode::Esc);
    assert!(!app.recipe_edit_open(), "Esc 收起");
    assert_eq!(
        std::fs::read_to_string(&recipe).unwrap(),
        original,
        "返航不落盘"
    );
    std::fs::remove_dir_all(&root).ok();
}

/// 规格验收 5:常畅 / 紧凑两档快照 —— 表单(组件槽/env 清单 + 键位行)与确认闸
/// (预跑四态 + 拟落账 + 写回闸)两档都在阵、不炸。
#[tokio::test]
async fn 配方编辑_两档快照() {
    let (mut app, runner) = recipe_app("snap", 0, VALIDATE_PASS);
    open_editor(&mut app);
    drive_edits(&mut app);
    // 表单两档
    for (w, h) in [(80u16, 30u16), (66, 50)] {
        let f = crate::testutil::frame_display(&app, w, h);
        assert!(f.contains("配方编辑器"), "{w}×{h} 标题:{f}");
        assert!(f.contains("组件槽"), "{w}×{h} 组件槽区:{f}");
        assert!(f.contains("env 声明"), "{w}×{h} env 区:{f}");
        assert!(f.contains("拟落账+预跑"), "{w}×{h} 表单键位行:{f}");
        assert!(!f.contains("panic"), "{w}×{h} 不炸:{f}");
    }
    // 进确认闸(预跑通过)
    key(&mut app, crossterm::event::KeyCode::Char('s'));
    feed_validate(&mut app, &runner).await;
    for (w, h) in [(80u16, 30u16), (66, 50)] {
        let f = crate::testutil::frame_display(&app, w, h);
        assert!(f.contains("配方编辑器"), "{w}×{h} 确认闸标题:{f}");
        assert!(f.contains("预跑"), "{w}×{h} 预跑四态在场:{f}");
        assert!(f.contains("recipe.yaml"), "{w}×{h} 将写文件在场:{f}");
        assert!(f.contains("Enter 写回"), "{w}×{h} 写回键位行:{f}");
        assert!(!f.contains("panic"), "{w}×{h} 不炸:{f}");
    }
    std::fs::remove_dir_all(std::path::PathBuf::from(app.data_root.clone().unwrap())).ok();
}

/// 三道门:数据仓根不就绪 / 装配互斥 / 非配方域都不开浮层(只留指路)。
#[test]
fn 配方编辑_三道门_根不就绪装配互斥非配方域() {
    // 数据仓根未设
    let (mut app, _r) = recipe_app("gate-root", 0, VALIDATE_PASS);
    app.data_root = None;
    key(&mut app, crossterm::event::KeyCode::Char('e'));
    assert!(!app.recipe_edit_open(), "根未设:指路,不开编辑器");
    assert!(
        app.log.iter().any(|l| l.contains("数据仓")),
        "根未设只留指路:{:?}",
        app.log
    );
    // 装配互斥
    let (mut app2, _r2) = recipe_app("gate-asm", 0, VALIDATE_PASS);
    app2.assembling = true;
    key(&mut app2, crossterm::event::KeyCode::Char('e'));
    assert!(!app2.recipe_edit_open(), "装配互斥");
    // 非配方域静默不摆(与 r 只外部源域、p 只组件域同构)
    let (mut app3, _r3) = recipe_app("gate-dom", 0, VALIDATE_PASS);
    app3.rc_domain = crate::app::Domain::Components;
    key(&mut app3, crossterm::event::KeyCode::Char('e'));
    assert!(!app3.recipe_edit_open(), "非配方域不给编辑器入口");
    for a in [&app, &app2, &app3] {
        if let Some(root) = a.data_root.as_ref() {
            std::fs::remove_dir_all(root).ok();
        }
    }
}

/// 浮层在场吞键:五域键位(切域/选行/过滤/建议命令/落点/打包)一个都不受理,
/// 配方屏原选中行原样不动。
#[test]
fn 配方编辑_浮层吞键_五域键位不受理() {
    let (mut app, _r) = recipe_app("swallow", 0, VALIDATE_PASS);
    open_editor(&mut app);
    let before = (app.rc_domain, app.rc_sel, app.rc_filter.clone());
    for code in [
        crossterm::event::KeyCode::Char('h'),
        crossterm::event::KeyCode::Char('l'),
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyCode::Up,
        crossterm::event::KeyCode::Char('/'),
        crossterm::event::KeyCode::Char('c'),
        crossterm::event::KeyCode::Char('p'),
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyCode::Char('j'),
        crossterm::event::KeyCode::Char('t'),
    ] {
        key(&mut app, code);
    }
    assert_eq!(
        (app.rc_domain, app.rc_sel, app.rc_filter.clone()),
        before,
        "浮层吞键:五域与壳级切屏/主题状态一个都不改"
    );
    std::fs::remove_dir_all(std::path::PathBuf::from(app.data_root.clone().unwrap())).ok();
}

/// 预跑在途直接返航:迟到的 validate 回音应丢弃,不污染健康屏 hl_result
/// (按 argv 暂存标记分流,不是靠浮层在不在场)。
#[tokio::test]
async fn 配方编辑_预跑在途返航_迟到回音不污染健康屏() {
    let (mut app, runner) = recipe_app("abandon-inflight", 0, VALIDATE_PASS);
    open_editor(&mut app);
    drive_edits(&mut app);
    key(&mut app, crossterm::event::KeyCode::Char('s'));
    // 预跑在途直接返航(scratch 随返航清)
    key(&mut app, crossterm::event::KeyCode::Esc);
    assert!(!app.recipe_edit_open(), "返航收起编辑器");
    assert!(app.hl_result.is_none(), "前置:健康屏无结论");
    // 迟到回音到达:指向 scratch 暂存,应丢弃
    feed_validate(&mut app, &runner).await;
    assert!(
        app.hl_result.is_none(),
        "编辑器的迟到预跑回音不污染健康屏 hl_result"
    );
    std::fs::remove_dir_all(std::path::PathBuf::from(app.data_root.clone().unwrap())).ok();
}

/// 规格验收的鼠标一侧:确认段点「Enter 写回」段 = 写回(与键盘 Enter 同一条路),
/// 渲染与命中共源(屏上「Enter 写回」那几个字落进 Activate 命中区,ADR-0005)。
#[tokio::test]
async fn 配方编辑_确认闸写回段_点哪打哪() {
    use ratatui::layout::{Position, Rect};
    for (w, h) in [(80u16, 30u16), (66, 50)] {
        let (mut app, runner) = recipe_app(&format!("mouse-{w}x{h}"), 0, VALIDATE_PASS);
        let root = std::path::PathBuf::from(app.data_root.clone().unwrap());
        let recipe = root.join("recipes/research-desk/recipe.yaml");
        let original = std::fs::read_to_string(&recipe).unwrap();
        open_editor(&mut app);
        drive_edits(&mut app);
        key(&mut app, crossterm::event::KeyCode::Char('s'));
        feed_validate(&mut app, &runner).await;
        let f = crate::testutil::frame_display(&app, w, h);
        assert!(f.contains("Enter 写回"), "{w}×{h} 写回段在场:{f}");
        app.size = Rect::new(0, 0, w, h);
        let hits = crate::layout::hitmap(app.size, &app, crate::layout::size_class(app.size));
        let act = hits
            .iter()
            .find(|(_, t)| *t == crate::layout::Target::Activate)
            .map(|(r, _)| *r)
            .expect("确认段产「Enter 写回」命中区");
        let (row, col) = locate(&f, "Enter 写回");
        assert!(
            act.contains(Position::new(col, row)),
            "{w}×{h} 渲染与命中共源:屏上「Enter 写回」落进 Activate 命中区:{f}"
        );
        app.update(Message::Mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: col,
            row,
            modifiers: crossterm::event::KeyModifiers::NONE,
        }));
        assert_ne!(
            std::fs::read_to_string(&recipe).unwrap(),
            original,
            "{w}×{h} 点写回段 = 落盘(与键盘 Enter 同路)"
        );
        assert!(matches!(
            app.rc_edit_flow.as_ref().map(|f| f.phase),
            Some(crate::screens::RecipeEditPhase::Done)
        ));
        std::fs::remove_dir_all(&root).ok();
    }
}
