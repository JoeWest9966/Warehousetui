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
    assert!(s.contains("落组件"));
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

/// 66×50 紧凑态:装配七步栏收成一行进度——位置化标记 + 当前阶段名,
/// 随 stderr 日志推进打 ✓;日志独占余下全部行(不再被 14 列竖栏挤压)。
#[test]
fn 紧凑_装配收起一行进度_66x50() {
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.assembling = true;
    app.as_log = vec![
        "0 前置 …".into(),
        "1 引导 …".into(),
        "2 铺骨架 …".into(),
        "3 落组件 …".into(),
    ];
    let s = draw(&app, 66, 50);
    assert!(
        s.contains("七步 ✓✓✓✓▸·· 4 合配置"),
        "一行进度:4 步完成、当前第 5 步可辨识,原文拿到: {s:?}"
    );
    assert!(!s.contains("┌ 七步 "), "七步竖栏必须收起");
    assert!(
        s.contains("stderr 实时日志") && s.contains("0 前置 …"),
        "日志全宽可见,早先行也在(行数变多)"
    );
    // 全部完成:七格满✓,不再有未完成的「·」悬挂
    app.as_log = STEPS_LOG_ALL.iter().map(|s| s.to_string()).collect();
    let done = draw(&app, 66, 50);
    assert!(
        done.contains("七步 ✓✓✓✓✓✓✓ 全部完成"),
        "七步全完成:满格标记,原文拿到: {done:?}"
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
#[test]
fn 常畅_内层栏回归_80x30竖轨在场() {
    let mut app = fake_app();
    app.screen = Screen::Assemble;
    app.assembling = true;
    app.as_log = vec!["0 前置 …".into(), "1 引导 …".into()];
    let s = draw(&app, 80, 30);
    assert!(s.contains("┌ 七步 "), "常畅态七步竖栏原样在场");
    assert!(s.contains("✓ 0 前置"), "竖栏逐步打勾原样在场");

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
    // 越界/陈旧偏移:min 钳制取窗不 panic;七步判定读全量 as_log,不受取窗影响
    app.as_log.push("5 写清单 完成".into());
    app.as_log_offset = 100;
    let s3 = draw(&app, 80, 30);
    assert!(!s3.contains("行0"), "取窗钳在日志内,不越界");
    assert!(
        s3.contains("✓ 5 写清单"),
        "七步判定读全量日志:行被移出窗仍打勾"
    );
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

// ---- #29 能力探测门(设置屏能力清单渲染) ----

#[test]
fn 设置屏能力清单_缺能力禁用并指路_紧凑态不截断() {
    use crate::engine::{Capability, FakeRunner, Probe};
    let runner = FakeRunner::with_stderr(&[("category", 3, "", "nw: 未知子命令 \"category\"")]);
    let mut app = App::with_runner(std::sync::Arc::new(runner));
    app.capabilities
        .record(Capability::Category, Probe::Unavailable);
    app.capabilities.record(Capability::Vps, Probe::Unavailable);
    app.capabilities
        .record(Capability::SourceRegister, Probe::Available);
    app.capabilities
        .record(Capability::SourceRecheck, Probe::Available);
    app.screen = Screen::Settings;
    // 常畅态:禁用组 + 指路句都在
    let s = draw(&app, 110, 30);
    assert!(s.contains("入口禁用:类别、vps"), "禁用组可见:{s}");
    assert!(s.contains("缺的是引擎能力,不是面板坏了"), "指路句可见:{s}");
    assert!(s.contains("✓外部源登记"), "齐备能力可见:{s}");
    // 紧凑态 66×50(保证最小):能力行渲染不炸、禁用组可见
    let c = draw(&app, 66, 50);
    assert!(c.contains("入口禁用"), "紧凑态禁用组可见:{c}");
    assert!(c.contains("能力"), "紧凑态能力行在场:{c}");
}

#[test]
fn 设置屏能力_无响应回未决不冒充禁用() {
    use crate::engine::{Capability, FakeRunner, Probe};
    // found=true 但探针无回答:空 stdout、无「未知子命令」——归未决
    let runner = FakeRunner::with_stderr(&[("category", -1, "", "")]);
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
    let app = bake_app();
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
        s.contains(&format!("--repo {}", app.data_root.clone().unwrap())),
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
