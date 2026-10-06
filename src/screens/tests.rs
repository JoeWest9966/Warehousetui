//! 五屏渲染快照 + 引擎结果路由的行为测试(TestBackend,零真实进程)。

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use crate::app::{App, HealthMode, Message, Screen};
use crate::engine::{EngineEvent, EngineOutput, ExitCode, FakeRunner};
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
