//! 作者侧脚手架表单流(spec #95 工单 #110,含首套模板)。
//!
//! 面板第一次**自己写文件**的消费方之一(ADR-0010):图形表单生成 `SKILL.md`
//! 骨架,落进数据仓 `skills/`。落盘全程走 datarepo 写侧(#109)——**拟落账
//! 摊开(路径 + 内容)→ 图形确认 → 原子落盘(tmp + rename)**,确认前一个字节
//! 不落。
//!
//! # 可复用抽象(#111 extension 脚手架复用同一表单流,不复制)
//!
//! 表单流拆成两层,按「什么四张消费票共用 / 什么各自不同」切:
//!
//! - **共用的表单流**(本模块的[`ScaffoldFlow`] + [`ScaffoldSpec`] + 拟落账
//!   预览/结论渲染 + `App` 的 `on_key_scaffold` / `dispatch_scaffold_confirm`):
//!   阶段机(表单 → 拟落账预览 → 结论)、拟落账形状、确认闸、原子落盘调用。
//!   #111 的 extension 脚手架换一份表单与一份 [`ScaffoldSpec`] 实现即可,
//!   流程/预览/落盘一字不重写。
//! - **各自不同的规格**(本文件的 [`SkillForm`] / `impl ScaffoldSpec for
//!   SkillForm` / `render_skill_form` / `on_key_skill_form`):字段形态、
//!   形状校验、骨架内容、落点目录。
//!
//! 字段形状校验**对齐上游 validate 对 skill 的检查**:`SKILL.md` frontmatter
//! 必填 `name` + `description`(见上游 `docs/external-sources.md` 注册验收第
//! 2 条);skill 身份 = frontmatter 的 `name`,故名字按引擎 `validName` 同款
//! 口径校验(它同时是落点目录名 `skills/<名>/`)。产出内容是**面板自有资产**
//! (内置模板),不是引擎逻辑复刻(ADR-0010 §3)。

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::datarepo::write::{PlannedWrite, RepoRelPath, WritePlan, WriteReceipt};
use crate::input::{self, InputAction};
use crate::layout::popup_rect;
use crate::screens::recipes::clip;
use crate::screens::FormAction;

// ---- 共用的表单流:阶段机 + 拟落账/收据槽 ----

/// 脚手架表单流的阶段。与登记两段式(`RegPhase`)同构的精神:**拟落账段
/// 文件不落**,落盘段才在图形确认后发生;任一阶段可返航,返航不留半份骨架。
///
/// 与两段式登记的区别:登记落盘是**异步引擎命令**(入队等回音,故有在途态),
/// 脚手架落盘是**同步文件 IO**(`DataRepo::commit` 当场 tmp + rename),没有
/// 「在途」态——确认键按下即写完,直接进结论。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ScaffoldPhase {
    /// 未脚手架;配方屏常态。
    #[default]
    Idle,
    /// 填表(规格自己的表单形态,如 skill 的名称/描述/触发词/模板)。
    Form,
    /// 拟落账已定型并摊开(路径 + 内容预览),**一个字节都没落**;等图形确认。
    Review,
    /// 已确认并原子落盘,结论(实落清单)在此呈现。
    Done,
}

/// 脚手架表单流的运行时:阶段 + 拟落账 + 实落收据 + 落盘失败原因。
/// **与规格无关**——规格差异全在 [`ScaffoldSpec`] 与本模块各自的表单上,
/// 这一份骨架被 #110 / #111 共用(拟落账 / 收据都是 datarepo 写侧的形状)。
#[derive(Default)]
pub struct ScaffoldFlow {
    pub phase: ScaffoldPhase,
    /// 拟落账(表单 Enter 后由写侧 `plan` 定型;确认闸直接呈现它)。
    pub plan: Option<WritePlan>,
    /// 实落收据(确认放行后 `commit` 回报;结论屏呈现)。
    pub receipt: Option<WriteReceipt>,
    /// 落盘失败的人话(磁盘 / 权限问题,不是输入问题;留在确认态说清)。
    pub error: Option<String>,
}

impl ScaffoldFlow {
    /// 发起脚手架:进表单态,清掉上一轮(旧拟落账/收据不冒充本次)。
    pub fn begin(&mut self) {
        self.phase = ScaffoldPhase::Form;
        self.plan = None;
        self.receipt = None;
        self.error = None;
    }

    /// 表单 Enter 且字段合法:拟落账定型,进预览态(**仍不落盘**)。
    pub fn enter_review(&mut self, plan: WritePlan) {
        self.phase = ScaffoldPhase::Review;
        self.plan = Some(plan);
        self.receipt = None;
        self.error = None;
    }

    /// 确认放行后落盘成功:进结论态,收据在手。
    pub fn enter_done(&mut self, receipt: WriteReceipt) {
        self.phase = ScaffoldPhase::Done;
        self.receipt = Some(receipt);
        self.error = None;
    }

    /// 落盘失败:留在确认态把结论说清(不冒充成功、不悄悄回表单;
    /// 拟落账仍在,操作者可知刚才想落什么)。
    pub fn fail(&mut self, reason: String) {
        self.error = Some(reason);
    }

    /// 返航:任一态同义,清空不留半份(拟落账/收据一并丢)。
    pub fn abandon(&mut self) {
        self.phase = ScaffoldPhase::Idle;
        self.plan = None;
        self.receipt = None;
        self.error = None;
    }

    /// 表单流在场(HUD / 键位优先级 / 命中共用这一条)。
    pub fn in_flight(&self) -> bool {
        !matches!(self.phase, ScaffoldPhase::Idle)
    }
}

/// 脚手架规格契约:#110 skill 与 #111 extension 各实现一份,表单流本身不复制。
///
/// 只有一个必填方法——**字段形状校验 + 拟落账条目生成**(面板自有资产):
/// 合法字段 → 一组 [`PlannedWrite`](datarepo::write::PlannedWrite)(将写路径 +
/// 内容字节);不合法 → 人话原因清单(拦在表单里,留可改,不喂给写侧)。
/// 之后的「写侧 `plan` 校验 → 拟落账预览 → 图形确认 → `commit` 原子落盘」
/// 全走共用流程,规格不再参与。
pub trait ScaffoldSpec {
    /// 表单标题(呈现层用;如 "skill" / "extension")。
    fn title() -> &'static str
    where
        Self: Sized;

    /// 字段形状校验(对齐上游 validate 对相应文件的检查)+ 生成拟落账条目。
    /// `Err` = 拒绝原因(拦在表单里);`Ok` = 将写文件清单(路径 + 内容字节)。
    fn planned_writes(&self) -> Result<Vec<PlannedWrite>, Vec<String>>;
}

// ---- skill 规格(#110):名称 / 描述 / 触发词表单 + 首套 agent skill 模板 ----

/// skill 表单的字段(↑/↓ 移焦点;文本字段走通用单行输入态机)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SkillField {
    #[default]
    Name,
    Description,
    Triggers,
    /// 模板起步开关(空格切换;不是文本字段)。
    Template,
}

impl SkillField {
    pub const ALL: [SkillField; 4] = [
        SkillField::Name,
        SkillField::Description,
        SkillField::Triggers,
        SkillField::Template,
    ];

    /// 字段标签(与工单「名称/描述/触发词」同词)。
    pub fn label(self) -> &'static str {
        match self {
            SkillField::Name => "名称",
            SkillField::Description => "描述",
            SkillField::Triggers => "触发词",
            SkillField::Template => "模板",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    /// 文本字段(有单行缓冲);模板开关没有。
    fn is_text(self) -> bool {
        !matches!(self, SkillField::Template)
    }
}

/// skill 脚手架表单:名称 / 描述 / 触发词 + 模板起步开关。
/// `name` = skill 身份(= SKILL.md frontmatter 的 name,也是落点目录名)。
#[derive(Clone, Default)]
pub struct SkillForm {
    pub name: String,
    pub description: String,
    /// 触发词:一行,逗号 / 顿号 / 分号分隔(面板侧便利,进骨架正文)。
    pub triggers: String,
    /// 从内置 agent skill 模板起步(true)还是空白骨架(false)。
    pub template: bool,
    pub field: SkillField,
}

impl SkillForm {
    fn focused_buf(&mut self) -> Option<&mut String> {
        match self.field {
            SkillField::Name => Some(&mut self.name),
            SkillField::Description => Some(&mut self.description),
            SkillField::Triggers => Some(&mut self.triggers),
            SkillField::Template => None,
        }
    }

    /// ↑/↓ 移字段(四字段轮转,到顶/到底不越界、不绕回)。
    fn move_field(&mut self, down: bool) {
        let i = self.field.index();
        let n = SkillField::ALL.len();
        self.field = SkillField::ALL[if down {
            (i + 1).min(n - 1)
        } else {
            i.saturating_sub(1)
        }];
    }
}

impl ScaffoldSpec for SkillForm {
    fn title() -> &'static str {
        "skill"
    }

    fn planned_writes(&self) -> Result<Vec<PlannedWrite>, Vec<String>> {
        let mut errs = Vec::new();
        // 名称:skill 身份 + 落点目录名,按引擎 validName 同款口径(上游惯例)。
        let name = self.name.trim();
        if name.is_empty() {
            errs.push("名称必填:skill 身份 = SKILL.md frontmatter 的 name".into());
        } else if let Some(reason) = invalid_skill_name(name) {
            errs.push(reason);
        }
        // 描述:SKILL.md frontmatter 的 description,validate 盯着这项(注册验收第 2 条)。
        let description = self.description.trim();
        if description.is_empty() {
            errs.push("描述必填:SKILL.md frontmatter 的 description(validate 盯着这项)".into());
        }
        if !errs.is_empty() {
            return Err(errs);
        }
        let triggers = split_triggers(&self.triggers);
        let content = skill_md(name, description, &triggers, self.template);
        // 落点:数据仓 skills/<名>/SKILL.md。name 已校验为合法目录名,拼得出合法相对路径。
        let path = RepoRelPath::new(&format!("skills/{name}/SKILL.md"))
            .expect("name 已校验:拼得出的相对路径合法");
        Ok(vec![PlannedWrite::text(path, &content)])
    }
}

/// 名称形状校验(对齐上游 `internal/bake/scaffold.go` 的 `validName`:名字直接
/// 拼进文件路径,挡路径穿越与破坏路径的字符;字母数字按 Unicode 判,`-` `_` `.`
/// 放行——非 ASCII 名合法,同引擎口径)。
fn invalid_skill_name(name: &str) -> Option<String> {
    if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
        return Some("名称不能是路径:不含 `/` `\\` 分隔符,也不是 `.` / `..`".into());
    }
    if let Some(c) = name
        .chars()
        .find(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        return Some(format!(
            "名称含不支持的字符「{c}」:只允许字母 / 数字 / 连字符 / 下划线 / 点"
        ));
    }
    None
}

/// 触发词拆分:一行里逗号 / 顿号 / 分号 / 换行分隔,去空白、丢空段。
fn split_triggers(raw: &str) -> Vec<String> {
    raw.split([',', '，', '、', ';', '；', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// 生成 `SKILL.md` 字节。**frontmatter 必填 `name` + `description`**(上游
/// validate 对 skill 的检查项);正文按空白 / 模板两路给——模板是面板自有的
/// 「首套 agent skill 模板」静态资产,不是引擎逻辑复刻。
fn skill_md(name: &str, description: &str, triggers: &[String], template: bool) -> String {
    let mut s = String::new();
    s.push_str("---\n");
    s.push_str(&format!("name: {name}\n"));
    s.push_str(&format!("description: {description}\n"));
    s.push_str("---\n");
    s.push_str(&format!("\n# {name}\n\n{description}\n"));
    if template {
        // 首套 agent skill 模板:给「何时触发 / 用法 / 备注」的起步段落。
        s.push_str("\n## 何时触发\n\n");
        if triggers.is_empty() {
            s.push_str("- (补充:什么时候该启用这个 skill)\n");
        } else {
            for t in triggers {
                s.push_str(&format!("- {t}\n"));
            }
        }
        s.push_str("\n## 用法\n\n(说明这个 skill 做什么、怎么用)\n");
        s.push_str("\n## 备注\n\n- 由 nwt 脚手架生成(agent skill 模板起步);按需增删。\n");
    } else if !triggers.is_empty() {
        // 空白骨架:只在前置必填之外补一个触发词段(有触发词才补)。
        s.push_str("\n## 触发词\n\n");
        for t in triggers {
            s.push_str(&format!("- {t}\n"));
        }
    }
    s
}

/// 表单键位(规格自己的键入分发):↑/↓ 移字段;模板字段空格切换;文本字段走
/// 通用单行输入态机(Enter 提交 / Esc·q 返航 / 退格删 / 只收可打印字符)。
/// 后果形状与编目册表单同一只 [`FormAction`](crate::screens::FormAction)。
pub fn on_key_skill_form(form: &mut SkillForm, k: KeyEvent) -> FormAction {
    match k.code {
        KeyCode::Up => {
            form.move_field(false);
            return FormAction::Edited;
        }
        KeyCode::Down => {
            form.move_field(true);
            return FormAction::Edited;
        }
        // 模板开关:焦点在模板字段时,空格切换(空格在文本字段里仍是普通字符)。
        KeyCode::Char(' ') if form.field == SkillField::Template => {
            form.template = !form.template;
            return FormAction::Edited;
        }
        _ => {}
    }
    if !form.field.is_text() {
        // 模板字段没有文本缓冲:Enter 提交、Esc/q 返航,其余键不动作。
        return match k.code {
            KeyCode::Enter => FormAction::Submit,
            KeyCode::Esc | KeyCode::Char('q') => FormAction::Cancel,
            _ => FormAction::Edited,
        };
    }
    match input::on_key(form.focused_buf().expect("文本字段有缓冲"), k) {
        InputAction::Submit => FormAction::Submit,
        InputAction::Cancel => FormAction::Cancel,
        InputAction::Edited | InputAction::Ignored => FormAction::Edited,
    }
}

// ---- 渲染(共用预览/结论 + skill 表单)----

/// 脚手架浮层总入口:按阶段画一个;Idle 无浮层(几何与命中区同一 `popup_rect`)。
pub fn render_scaffold_overlay(frame: &mut Frame, area: Rect, app: &App) {
    match app.scaffold_flow.phase {
        ScaffoldPhase::Idle => {}
        ScaffoldPhase::Form => render_skill_form(frame, area, app),
        ScaffoldPhase::Review => render_scaffold_review(frame, area, app),
        ScaffoldPhase::Done => render_scaffold_done(frame, area, app),
    }
}

/// ① 表单:名称 / 描述 / 触发词 / 模板起步。当前字段带块光标,↑/↓ 移字段。
fn render_skill_form(frame: &mut Frame, area: Rect, app: &App) {
    use crate::layout::{SCAFFOLD_FORM_H, SCAFFOLD_FORM_W};
    let p = &app.palette;
    let popup = popup_rect(area, SCAFFOLD_FORM_W, SCAFFOLD_FORM_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let f = &app.scaffold_form;
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  skill 身份 = SKILL.md frontmatter 的 name;落进 skills/<名>/",
        p.dim(),
    ))];
    for field in SkillField::ALL {
        let on = field == f.field;
        if field == SkillField::Template {
            let mark = if f.template { "[x]" } else { "[ ]" };
            lines.push(Line::from(vec![
                Span::raw(format!("  {} ", field.label())),
                Span::styled(
                    format!("{mark} 从内置 agent skill 模板起步"),
                    if on { p.sel().bold() } else { p.text() },
                ),
            ]));
            continue;
        }
        let value = match field {
            SkillField::Name => &f.name,
            SkillField::Description => &f.description,
            SkillField::Triggers => &f.triggers,
            SkillField::Template => unreachable!("模板字段已在上支接走"),
        };
        let shown = if on {
            format!("{value}█")
        } else {
            value.clone()
        };
        lines.push(Line::from(vec![
            Span::raw(format!("  {} ", field.label())),
            Span::styled(
                clip(&shown, inner_w.saturating_sub(6)),
                if on { p.sel().bold() } else { p.text() },
            ),
        ]));
    }
    lines.push(Line::from(Span::styled(
        "  ↑/↓ 移字段 · 空格切模板 · Enter 拟落账预览 · Esc/q 放弃",
        p.dim(),
    )));
    let budget = popup.height.saturating_sub(3) as usize;
    lines.truncate(budget);
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    render_scaffold_block(frame, popup, lines, p, " 新建 skill · 表单 ");
}

/// ② 拟落账预览(**本段一个字节都不落**):摊开将写文件清单 + 逐条路径与内容
/// 预览 + 体量;Enter 才落盘,Esc 返航。与登记两段式确认屏同一闸语义。
fn render_scaffold_review(frame: &mut Frame, area: Rect, app: &App) {
    use crate::layout::{SCAFFOLD_KEY_COMMIT, SCAFFOLD_REVIEW_H, SCAFFOLD_REVIEW_W};
    let p = &app.palette;
    let popup = popup_rect(area, SCAFFOLD_REVIEW_W, SCAFFOLD_REVIEW_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let budget = popup.height.saturating_sub(3) as usize;
    let mut body: Vec<Line> = Vec::new();
    let Some(plan) = app.scaffold_flow.plan.as_ref() else {
        body.push(Line::from(Span::styled("  拟落账不在场", p.warn())));
        render_scaffold_block(frame, popup, body, p, " 新建脚手架 · 拟落账 ");
        return;
    };
    body.push(Line::from(Span::styled(
        format!(
            "  将写 {} 个文件 · 共 {} 字节(确认后才落盘)",
            plan.writes.len(),
            plan.total_bytes()
        ),
        p.info(),
    )));
    if let Some(err) = app.scaffold_flow.error.as_ref() {
        body.push(Line::from(Span::styled(
            format!("  ⚠ {err}"),
            p.warn().bold(),
        )));
    }
    for w in &plan.writes {
        body.push(Line::from(Span::styled(
            format!("  → {}", w.path.as_str()),
            p.accent(),
        )));
        let text = String::from_utf8_lossy(&w.content);
        for ln in text.lines() {
            body.push(Line::from(Span::styled(
                format!("    {}", clip(ln, inner_w.saturating_sub(4))),
                p.text(),
            )));
        }
    }
    // 收口到预算:超量截断 + 一句「未全显」,欠量垫空,键位行恒在内层最后一行。
    let mut lines: Vec<Line> = if body.len() > budget {
        let mut kept: Vec<Line> = body.into_iter().take(budget.saturating_sub(1)).collect();
        kept.push(Line::from(Span::styled("  …(屏内未全显)", p.dim())));
        kept
    } else {
        let mut kept = body;
        while kept.len() < budget {
            kept.push(Line::from(""));
        }
        kept
    };
    lines.push(Line::from(vec![
        Span::styled(SCAFFOLD_KEY_COMMIT, p.accent().bold()),
        Span::raw("  "),
        Span::styled("Esc 返航 ", p.dim()),
    ]));
    render_scaffold_block(frame, popup, lines, p, " 新建脚手架 · 拟落账 ");
}

/// ③ 结论屏:实落清单(相对路径)+ git 纪律一句;点任意处 / Esc/q 返航。
fn render_scaffold_done(frame: &mut Frame, area: Rect, app: &App) {
    use crate::layout::{SCAFFOLD_DONE_H, SCAFFOLD_DONE_W};
    let p = &app.palette;
    let popup = popup_rect(area, SCAFFOLD_DONE_W, SCAFFOLD_DONE_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  ✓ 骨架已写入数据仓(原子落盘:tmp + rename)",
        p.info(),
    ))];
    if let Some(receipt) = app.scaffold_flow.receipt.as_ref() {
        for path in &receipt.written {
            lines.push(Line::from(Span::styled(
                format!("  落盘 {}", clip(path, inner_w.saturating_sub(8))),
                p.text(),
            )));
        }
    }
    lines.push(Line::from(Span::styled(
        "  改动走 git PR + 家长审;面板不碰版本控制。",
        p.dim(),
    )));
    let budget = popup.height.saturating_sub(3) as usize;
    lines.truncate(budget);
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled("  Esc/q 返航 ", p.dim())));
    render_scaffold_block(frame, popup, lines, p, " 脚手架 · 已写入 ");
}

/// 脚手架弹层正文收口(与册体浮层同一款式:边框 + 标题 + 底色)。
fn render_scaffold_block(
    frame: &mut Frame,
    popup: Rect,
    lines: Vec<Line<'static>>,
    p: &crate::theme::Palette,
    title: &'static str,
) {
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(title, p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Message};
    use crate::engine::FakeRunner;
    use crate::testutil::frame_display;
    use crossterm::event::KeyModifiers;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::sync::Arc;

    fn press(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }
    fn enter() -> KeyEvent {
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
    }
    fn down() -> KeyEvent {
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)
    }

    fn form_app() -> App {
        let runner = Arc::new(FakeRunner::new(&[("ls", 0, "{}")]));
        App::with_runner(runner)
    }

    /// 每测试一个一次性数据仓目录(进程级唯一 + 标签唯一,并行不互踩)。
    fn repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nwt-scaffold-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ---- 规格:字段形状校验 + 骨架内容 ----

    #[test]
    fn 表单_字段导航与模板切换() {
        let mut f = SkillForm::default();
        assert_eq!(f.field, SkillField::Name);
        on_key_skill_form(&mut f, down());
        assert_eq!(f.field, SkillField::Description);
        on_key_skill_form(&mut f, down());
        assert_eq!(f.field, SkillField::Triggers);
        on_key_skill_form(&mut f, down());
        assert_eq!(f.field, SkillField::Template);
        // 到底不绕回
        on_key_skill_form(&mut f, down());
        assert_eq!(f.field, SkillField::Template);
        // 模板字段:空格切换起步方式
        assert!(!f.template);
        on_key_skill_form(&mut f, press(' '));
        assert!(f.template);
        // 键入进当前文本字段
        f.field = SkillField::Name;
        on_key_skill_form(&mut f, press('a'));
        on_key_skill_form(&mut f, press('b'));
        assert_eq!(f.name, "ab");
    }

    #[test]
    fn planned_writes_空名称与空描述被拦() {
        let mut f = SkillForm {
            description: "会检索".into(),
            ..SkillForm::default()
        };
        let errs = f.planned_writes().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("名称")), "{errs:?}");
        // 补上名称,描述又空 → 换一条拦
        f.name = "web-scout".into();
        f.description = "  ".into();
        let errs = f.planned_writes().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("描述")), "{errs:?}");
    }

    #[test]
    fn planned_writes_非法名称被拦_与引擎同款口径() {
        for bad in ["a/b", "..", "a b", "na:me", "skill@1"] {
            let f = SkillForm {
                name: bad.into(),
                description: "d".into(),
                ..SkillForm::default()
            };
            assert!(f.planned_writes().is_err(), "{bad:?} 应被拦");
        }
        // 合法名(含非 ASCII、连字符、下划线、点)照常收
        let f = SkillForm {
            name: "web-scout_v2.0".into(),
            description: "d".into(),
            ..SkillForm::default()
        };
        assert!(f.planned_writes().is_ok());
    }

    #[test]
    fn 合法产出_frontmatter必填name与description_过validate相应检查() {
        let f = SkillForm {
            name: "web-scout".into(),
            description: "从网页检索资料".into(),
            ..SkillForm::default()
        };
        let writes = f.planned_writes().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].path.as_str(), "skills/web-scout/SKILL.md");
        let md = String::from_utf8(writes[0].content.clone()).unwrap();
        // 上游 validate 对 skill 的检查:frontmatter 有 name + description。
        assert!(md.starts_with("---\n"), "YAML frontmatter 开头:{md}");
        assert!(md.contains("name: web-scout"), "frontmatter 有 name:{md}");
        assert!(
            md.contains("description: 从网页检索资料"),
            "frontmatter 有 description:{md}"
        );
    }

    #[test]
    fn 空白与模板两路_产出不同骨架() {
        let base = SkillForm {
            name: "web-scout".into(),
            description: "从网页检索资料".into(),
            triggers: "查网页, 找资料".into(),
            ..SkillForm::default()
        };
        let blank = String::from_utf8(base.planned_writes().unwrap()[0].content.clone()).unwrap();
        let mut tmpl = base.clone();
        tmpl.template = true;
        let tmpl_md = String::from_utf8(tmpl.planned_writes().unwrap()[0].content.clone()).unwrap();
        assert_ne!(blank, tmpl_md, "两路骨架不同");
        assert!(blank.contains("## 触发词"), "空白路给触发词段:{blank}");
        assert!(!blank.contains("## 用法"), "空白路无用法段:{blank}");
        assert!(
            tmpl_md.contains("## 何时触发"),
            "模板路给何时触发:{tmpl_md}"
        );
        assert!(tmpl_md.contains("## 用法"), "模板路给用法:{tmpl_md}");
        // 触发词按分隔符拆成两条
        assert!(
            tmpl_md.contains("- 查网页") && tmpl_md.contains("- 找资料"),
            "{tmpl_md}"
        );
    }

    // ---- 表单流:确认前零字节、确认后确定字节(两段式先例) ----

    #[test]
    fn 表单流_确认前零字节_确认后确定字节落进skills() {
        let root = repo("twostage");
        let mut app = form_app();
        app.data_root = Some(root.to_str().unwrap().to_string());
        // 数据仓根不可用时入口指路、不开流程
        let bad = std::env::temp_dir().join(format!("nwt-scaffold-nope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&bad);
        app.data_root = Some(bad.to_str().unwrap().to_string());
        app.start_scaffold();
        assert!(!app.scaffold_flow.in_flight(), "根不可用不开流程");
        assert!(
            std::fs::read_dir(&bad).is_err() || std::fs::read_dir(&bad).unwrap().next().is_none()
        );

        // 根可用:开流程填表
        app.data_root = Some(root.to_str().unwrap().to_string());
        app.start_scaffold();
        assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Form);
        app.scaffold_form.name = "web-scout".into();
        app.scaffold_form.description = "从网页检索资料".into();
        // Enter 提交 → 拟落账预览(仍不落盘)
        app.update(Message::Key(enter()));
        assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Review);
        assert!(
            std::fs::read_dir(&root).unwrap().next().is_none(),
            "确认前一个字节都不落(两段式先例)"
        );
        // Enter 确认 → 原子落盘
        app.update(Message::Key(enter()));
        assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Done);
        let written = std::fs::read_to_string(root.join("skills/web-scout/SKILL.md")).unwrap();
        assert!(
            written.starts_with("---\nname: web-scout\n"),
            "确定字节:{written}"
        );
        assert!(written.contains("description: 从网页检索资料"), "{written}");
        // 返航清状态
        app.update(Message::Key(press('q')));
        assert!(!app.scaffold_flow.in_flight(), "Done 后 Esc/q 返航");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 表单流_非法字段拦在表单里_不进预览更不落盘() {
        let root = repo("reject");
        let mut app = form_app();
        app.data_root = Some(root.to_str().unwrap().to_string());
        app.start_scaffold();
        app.scaffold_form.name = "a/b".into(); // 非法:路径分隔符
        app.scaffold_form.description = "d".into();
        app.update(Message::Key(enter()));
        assert_eq!(
            app.scaffold_flow.phase,
            ScaffoldPhase::Form,
            "拦在表单里,留可改"
        );
        assert!(
            std::fs::read_dir(&root).unwrap().next().is_none(),
            "被拒即不落盘"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 组件域n键_发起脚手架_其他域不触发() {
        use crate::app::{Domain, Screen};
        let root = repo("entry");
        let mut app = form_app();
        app.data_root = Some(root.to_str().unwrap().to_string());
        app.screen = Screen::Recipes;
        // 非组件域按 n 无 scaffolding 义(不开流程)
        app.rc_domain = Domain::Recipes;
        app.update(Message::Key(press('n')));
        assert!(!app.scaffold_flow.in_flight(), "非组件域 n 不触发脚手架");
        // 组件域按 n 开流程(键 → start_scaffold → 表单态)
        app.rc_domain = Domain::Components;
        app.update(Message::Key(press('n')));
        assert_eq!(
            app.scaffold_flow.phase,
            ScaffoldPhase::Form,
            "组件域 n 发起脚手架"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    // ---- 快照:常畅 / 紧凑两档 ----

    fn snap(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| {
            let area = f.area();
            crate::screens::render(f, area, app);
        })
        .unwrap();
        term.backend().to_string()
    }

    #[test]
    fn 脚手架浮层_表单拟落账结论_常畅紧凑两档不溢出() {
        for (w, h) in [(110u16, 30u16), (80, 30), (66, 50)] {
            let mut app = form_app();
            app.screen = crate::app::Screen::Recipes;
            // 表单态
            app.start_scaffold();
            app.scaffold_form.name = "web-scout".into();
            app.scaffold_form.description = "从网页检索资料".into();
            app.scaffold_form.triggers = "查网页".into();
            let f = snap(&app, w, h);
            assert!(f.contains("新建 skill"), "{w}×{h} 表单标题:{f}");
            assert!(f.contains("web-scout"), "{w}×{h} 名称回显:{f}");
            for ln in frame_display(&app, w, h).lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 表单有行超宽:{ln}"
                );
            }
            // 拟落账预览态
            app.update(Message::Key(enter()));
            assert_eq!(app.scaffold_flow.phase, ScaffoldPhase::Review);
            let f = snap(&app, w, h);
            assert!(f.contains("拟落账"), "{w}×{h} 预览标题:{f}");
            assert!(
                f.contains("skills/web-scout/SKILL.md"),
                "{w}×{h} 落点路径:{f}"
            );
            // 拟落账预览要摊开**内容**(frontmatter 必填两项),不只是路径。
            assert!(f.contains("name: web-scout"), "{w}×{h} 内容预览 name:{f}");
            assert!(
                f.contains("description: 从网页检索资料"),
                "{w}×{h} 内容预览 description:{f}"
            );
            assert!(f.contains("Enter 落盘"), "{w}×{h} 确认键位:{f}");
            for ln in frame_display(&app, w, h).lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 预览有行超宽:{ln}"
                );
            }
        }
    }
}
