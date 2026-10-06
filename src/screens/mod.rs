//! 五屏内容区。每屏一个渲染函数,输入是帧区域与共享状态。

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, Screen};
use crate::ui::screen_block;

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let block = screen_block(
        p,
        app.screen.title(),
        Some(
            match app.screen {
                Screen::Overview => "目标侧日常",
                Screen::Assemble => "七步幂等装配",
                Screen::Health => "doctor · validate",
                Screen::Recipes => "五域只读浏览",
                Screen::Settings => "NW_HOME · 引擎发现",
            }
            .to_string(),
        ),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let placeholder = match app.screen {
        Screen::Overview => vec![
            Line::from(Span::styled("警示带", p.warn())),
            Line::from("(ls 五域接入后:漂移派、未过配方在此排队)"),
            Line::from(""),
            Line::from(Span::styled("派 (profiles)", p.accent())),
            Line::from("状态灯 / 派名 / lockId / 装配于 / 体检摘要"),
            Line::from(""),
            Line::from(Span::styled("指令栏 + 最近日志", p.info())),
        ],
        Screen::Assemble => vec![
            Line::from(Span::styled("装配向导", p.accent())),
            Line::from("选锁定集 → 发射仪式 → 七步步骤条 + stderr 实时日志"),
            Line::from("失败停在当前步,证据五字段展开;取消幂等安全"),
        ],
        Screen::Health => vec![
            Line::from(Span::styled("健康面板", p.accent())),
            Line::from("doctor 七面卡片 + validate 27 项清单"),
            Line::from("skip 属正常态,降级 ≠ 失败"),
        ],
        Screen::Recipes => vec![
            Line::from(Span::styled("配方浏览", p.accent())),
            Line::from("recipes / components / sources / locks / profiles"),
            Line::from("只读 + 复制路径/命令 + $EDITOR 逃逸口"),
        ],
        Screen::Settings => vec![
            Line::from(Span::styled("设置", p.accent())),
            Line::from("NW_HOME 切换、nw 发现四策略、主题、指路屏"),
            Line::from("配置永不存密钥"),
        ],
    };
    frame.render_widget(Paragraph::new(placeholder), inner);
}
