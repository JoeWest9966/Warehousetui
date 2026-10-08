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
