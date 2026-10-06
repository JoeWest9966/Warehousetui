//! `nwt` —— Nous Warehouse 驾驶舱。
//!
//! 进程职责:恢复终端的兜底、把输入事件泵进状态机、把状态机渲染到
//! 终端。引擎调用与数据形状都在 [`engine`]。
#![allow(dead_code)] // 契约层/屏幕类型按票分期接线,#3–#7 施工期间整体放开;V1 收口时收紧

mod app;
mod engine;
mod screens;
mod term;
mod theme;
mod ui;

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
    *terminal = term::init()?;
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

#[tokio::main]
async fn main() -> Result<()> {
    // --nw-bin 显式指路(发现策略第一级)
    let explicit = parse_nw_bin_flag();

    let mut app = App::boot(explicit.as_deref());
    term::install_panic_hook();
    let mut terminal = term::init()?;

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

    let result = run_loop(&mut terminal, &mut app, &mut rx, &mut erx, &etx).await;

    term::restore();
    result
}

async fn run_loop(
    terminal: &mut term::Term,
    app: &mut App,
    rx: &mut mpsc::UnboundedReceiver<Event>,
    erx: &mut mpsc::UnboundedReceiver<engine::EngineEvent>,
    etx: &mpsc::UnboundedSender<engine::EngineEvent>,
) -> Result<()> {
    let mut tick = interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;
        if app.quit {
            break;
        }
        // 键位处理器排队的引擎命令在这里统一执行(键位没有 tx)。
        for args in std::mem::take(&mut app.queue) {
            app.spawn_engine(args, etx);
        }
        // 外部编辑器逃逸口:挂起我们的终端,让 $EDITOR 用,完事再点火。
        if std::mem::take(&mut app.pending_editor) {
            open_editor(terminal, app)?;
        }
        tokio::select! {
            Some(ev) = rx.recv() => {
                if let Event::Key(k) = ev {
                    if k.kind == KeyEventKind::Press {
                        app.update(Message::Key(k));
                    }
                }
            }
            Some(ev) = erx.recv() => app.update(Message::Engine(ev)),
            _ = tick.tick(), if app.assembling => app.update(Message::Tick),
        }
    }
    Ok(())
}
