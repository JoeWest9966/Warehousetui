//! `nwt` —— Nous Warehouse 驾驶舱。
#![allow(dead_code)] // 契约层/屏幕类型按票分期接线,#3–#7 施工期间整体放开;接线完成后收紧
//!
//! 进程职责只有三件事:恢复终端的兜底、把输入事件泵进状态机、
//! 把状态机渲染到终端。引擎调用与数据形状都在 [`engine`]。

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

#[tokio::main]
async fn main() -> Result<()> {
    let mut app = App::new();
    term::install_panic_hook();
    let mut terminal = term::init()?;

    // 输入线程:crossterm 阻塞读,汇入无界 channel(按键延迟与渲染解耦)。
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if tx.send(ev).is_err() {
                break;
            }
        }
    });

    let result = run_loop(&mut terminal, &mut app, &mut rx).await;

    term::restore();
    result
}

async fn run_loop(
    terminal: &mut term::Term,
    app: &mut App,
    rx: &mut mpsc::UnboundedReceiver<Event>,
) -> Result<()> {
    loop {
        terminal.draw(|frame| ui::render(frame, app))?;
        if app.quit {
            break;
        }
        match rx.recv().await {
            Some(Event::Key(k)) if k.kind == KeyEventKind::Press => app.update(Message::Key(k)),
            Some(_) => {}
            None => break,
        }
    }
    Ok(())
}
