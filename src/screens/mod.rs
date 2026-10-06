//! 五屏内容区。每屏一个渲染函数,输入是帧区域与共享状态。

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::App;
use crate::ui::screen_block;

mod assemble;
mod health;
mod overview;
mod recipes;
mod settings;

pub use assemble::render_assemble;
pub use health::render_health;
pub use overview::render_overview;
pub use recipes::render_recipes;
pub use settings::render_settings;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    match app.screen {
        crate::app::Screen::Overview => render_overview(frame, area, app),
        crate::app::Screen::Assemble => render_assemble(frame, area, app),
        crate::app::Screen::Health => render_health(frame, area, app),
        crate::app::Screen::Recipes => render_recipes(frame, area, app),
        crate::app::Screen::Settings => render_settings(frame, area, app),
    }
}

/// 数据屏外框:标题 + 右上角注脚(各屏统一用)。
pub(crate) fn body<'a>(app: &App, note: &'a str) -> ratatui::widgets::Block<'a> {
    screen_block(&app.palette, app.screen.title(), Some(note.to_string()))
}
