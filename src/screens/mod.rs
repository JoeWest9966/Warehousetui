//! 五屏内容区。每屏一个渲染函数,输入是帧区域与共享状态。

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::App;
use crate::ui::screen_block;

pub(crate) mod assemble;
pub(crate) mod catalog;
mod health;
mod overview;
pub(crate) mod recipes;
mod settings;

pub use assemble::render_assemble;
#[allow(unused_imports)]
pub use catalog::{
    new_form, new_key_form, new_sub_form, new_tag_form, on_key_form, on_key_key_form,
    on_key_sub_form, on_key_tag_form, render_catalog, spec_from_form, spec_from_key_form,
    spec_from_sub_form, spec_from_tag_form, Audit, CatalogKind, FormAction, KeyField, KeyForm,
    KeySpec, RegPhase, RegisterFlow, RegisterSpec, SecretBody, StartMode, SubField, SubForm,
    SubscriptionSpec, TagField, TagForm, TagFormMode, TagSpec, VpsField, VpsForm, VpsSpec,
    WriteRoute,
};
pub use health::render_health;
pub use overview::render_overview;
pub use recipes::render_recipes;
pub use settings::render_settings;

#[cfg(test)]
mod tests;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    match app.screen {
        crate::app::Screen::Overview => render_overview(frame, area, app),
        crate::app::Screen::Assemble => render_assemble(frame, area, app),
        crate::app::Screen::Health => render_health(frame, area, app),
        crate::app::Screen::Recipes => render_recipes(frame, area, app),
        crate::app::Screen::Vps => render_catalog(frame, area, app),
        crate::app::Screen::Settings => render_settings(frame, area, app),
    }
}

/// 数据屏外框:标题 + 右上角注脚(各屏统一用)。
pub(crate) fn body<'a>(app: &App, note: &'a str) -> ratatui::widgets::Block<'a> {
    screen_block(&app.palette, app.screen.title(), Some(note.to_string()))
}

/// 数据仓根三态指路的共用渲染(五域入口同源,措辞取自
/// [`crate::app::DataRootCause`],三种情形互不冒充)。
pub(crate) fn data_root_signpost(app: &App) -> Vec<ratatui::text::Line<'static>> {
    use ratatui::text::{Line, Span};
    let Some(cause) = app.data_root_cause() else {
        return Vec::new();
    };
    let p = &app.palette;
    vec![
        Line::from(Span::styled(
            format!(" ⚠ {} ", cause.title()),
            p.warn().bold(),
        )),
        Line::from(Span::styled(
            cause.advice(app.data_root.as_deref()),
            p.dim(),
        )),
    ]
}
