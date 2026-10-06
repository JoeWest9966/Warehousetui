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
