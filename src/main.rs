//! `nwt` —— Nous Warehouse 驾驶舱。
//!
//! 进程职责:恢复终端的兜底、把输入事件泵进状态机、把状态机渲染到
//! 终端。引擎调用与数据形状都在 [`engine`]。
#![allow(dead_code)] // 契约层/屏幕类型按票分期接线,#3–#7 施工期间整体放开;V1 收口时收紧

mod app;
mod engine;
mod input;
pub mod layout;
mod screens;
mod term;
#[cfg(test)]
mod testutil;
mod theme;
mod ui;
mod upgrade;

use anyhow::Result;
use app::{App, Message};
use crossterm::event::{self, Event, KeyEventKind};
use tokio::sync::mpsc;
use tokio::time::{interval, Duration, MissedTickBehavior};

/// 挂起 TUI,拉起 $EDITOR 编辑 config,关掉后重新接管终端。
fn open_editor(terminal: &mut term::Term, app: &mut App) -> Result<()> {
    let Some(ed) = App::editor_command() else {
        app.push_log("EDITOR/VISUAL 都没设,先用 cat 看吧");
        return Ok(());
    };
    let path = engine::config::config_path();
    if !path.exists() {
        // 落一份默认配置,免得编辑器对着空气
        let _ = engine::config::save(&engine::config::Config::default());
    }
    term::restore();
    println!("\n用 {ed} 编辑 {}\n", path.display());
    let status = std::process::Command::new(&ed).arg(&path).status();
    match status {
        Ok(s) => app.push_log(format!("编辑器退出:{s}")),
        Err(e) => app.push_log(format!("拉起 {ed} 失败:{e}")),
    }
    *terminal = term::init(app.mouse_enabled)?;
    Ok(())
}

/// 解析 --nw-bin:支持 `--nw-bin <path>` 与 `--nw-bin=<path>`。
fn parse_nw_bin_flag() -> Option<String> {
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        if let Some(v) = a.strip_prefix("--nw-bin=") {
            return Some(v.to_string());
        }
        if a == "--nw-bin" {
            return it.next();
        }
    }
    None
}

/// 参数列表里有 --no-mouse 吗(纯函数,单测钉住;argv 由真实入口注入)。
fn no_mouse_in(args: &[String]) -> bool {
    args.iter().any(|a| a == "--no-mouse")
}

/// 解析 --no-mouse 旗标(逃逸两联之一,另一联是设置屏开关)。
fn parse_no_mouse_flag() -> bool {
    no_mouse_in(&std::env::args().skip(1).collect::<Vec<_>>())
}

#[tokio::main]
async fn main() -> Result<()> {
    // --nw-bin 显式指路(发现策略第一级)
    let explicit = parse_nw_bin_flag();

    let mut app = App::boot(explicit.as_deref());
    // --no-mouse 旗标并入(配置一联已在 boot 载入)
    app.apply_no_mouse_flag(parse_no_mouse_flag());
    term::install_panic_hook();
    let mut terminal = term::init(app.mouse_enabled)?;

    // 输入线程:crossterm 阻塞读,汇入无界 channel。
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if tx.send(ev).is_err() {
                break;
            }
        }
    });

    // 引擎事件通道 + 开机自举(版本探测 + 首次 ls)。
    let (etx, mut erx) = mpsc::unbounded_channel::<engine::EngineEvent>();
    app.bootstrap(&etx);

    // 自升级事件通道(#121):与引擎事件并列,主循环在 select 里一起收。
    let (utx, mut urx) = mpsc::unbounded_channel::<upgrade::UpgradeEvent>();

    let result = run_loop(
        &mut terminal,
        &mut app,
        &mut rx,
        &mut erx,
        &etx,
        &mut urx,
        &utx,
    )
    .await;

    term::restore();
    result
}

async fn run_loop(
    terminal: &mut term::Term,
    app: &mut App,
    rx: &mut mpsc::UnboundedReceiver<Event>,
    erx: &mut mpsc::UnboundedReceiver<engine::EngineEvent>,
    etx: &mpsc::UnboundedSender<engine::EngineEvent>,
    urx: &mut mpsc::UnboundedReceiver<upgrade::UpgradeEvent>,
    utx: &mpsc::UnboundedSender<upgrade::UpgradeEvent>,
) -> Result<()> {
    let mut tick = interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let splash_deadline = std::time::Instant::now() + Duration::from_millis(800);
    loop {
        terminal.draw(|frame| {
            // 每帧把最近终端面记进 app.size,供鼠标命中定标;
            // 渲染本身保持无副作用(只读 app)。
            app.size = frame.area();
            ui::render(frame, app);
        })?;
        if app.splash {
            // 任意键或 800ms 到点,交棒主界面(首帧已画出)
            tokio::select! {
                Some(Event::Key(_)) = rx.recv() => app.splash = false,
                _ = tokio::time::sleep_until(splash_deadline.into()) => app.splash = false,
            }
            continue;
        }
        if app.quit {
            break;
        }
        // 自升级队列(#121):与引擎命令同一个 drain 模式,但走 upgrade 模块
        // 自己那条 async fn(它把 install.sh 的逐行原文当进度推回来)。
        for req in std::mem::take(&mut app.upgrade_queue) {
            let utx = utx.clone();
            tokio::spawn(async move {
                upgrade::run(&req, &upgrade::UpgradeEnv::from_env(), &utx).await;
            });
        }
        // 键位处理器排队的引擎命令在这里统一执行(键位没有 tx)。
        for args in std::mem::take(&mut app.queue) {
            app.spawn_engine(args, etx);
        }
        // 喂入队列(#56):argv 同一个唯一入口,只多一份一次性 stdin 本体
        // (订阅链接;写后即关、即弃,不进回音/日志/配置)。
        for (args, body) in std::mem::take(&mut app.fed_queue) {
            app.spawn_engine_fed(args, body, etx);
        }
        // 外部编辑器逃逸口:挂起我们的终端,让 $EDITOR 用,完事再点火。
        if std::mem::take(&mut app.pending_editor) {
            open_editor(terminal, app)?;
        }
        tokio::select! {
            Some(ev) = rx.recv() => match ev {
                Event::Key(k) if k.kind == KeyEventKind::Press => app.update(Message::Key(k)),
                // 统一事件入口:鼠标与键盘并列(分派由 #20 落地)
                Event::Mouse(m) => app.update(Message::Mouse(m)),
                _ => {}
            },
            Some(ev) = erx.recv() => app.update(Message::Engine(ev)),
            // 自升级推进(#121):install.sh 的逐行进度 + 终结结论。
            Some(ev) = urx.recv() => app.update(Message::Upgrade(ev)),
            _ = tick.tick(), if app.assembling => app.update(Message::Tick),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_mouse_旗标解析() {
        assert!(no_mouse_in(&args(&["nwt", "--no-mouse"])), "出现即关");
        assert!(
            no_mouse_in(&args(&["--no-mouse", "--nw-bin", "/x/nw"])),
            "与其他旗标共存"
        );
        assert!(!no_mouse_in(&args(&["nwt"])), "不出现不拦");
        assert!(
            !no_mouse_in(&args(&["nwt", "--no-mouse=1"])),
            "只认裸旗标,不认带值变体"
        );
    }
}
