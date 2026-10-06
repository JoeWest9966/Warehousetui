//! 应用外壳:Elm 式集中状态 + update + 屏幕定义。
//!
//! V1 只有一棵状态树;所有交互先变成 [`Message`],由 [`App::update`]
//! 收口,屏幕渲染函数无副作用。引擎调用只能经 [`App::spawn_engine`]。

use std::sync::Arc;
use tokio::sync::mpsc;

use crate::engine::{
    commands, CommandRunner, EngineEvent, EngineOutput, EngineStatus, LsData, State,
};
use crate::term::{self, ColorCap};
use crate::theme::{cached_degrade, Palette, ThemeKind};
use crossterm::event::{KeyCode, KeyEvent};

/// 五块屏幕,左竖导航的顺序即数组顺序。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Overview,
    Assemble,
    Health,
    Recipes,
    Settings,
}

impl Screen {
    pub const ALL: [Screen; 5] = [
        Screen::Overview,
        Screen::Assemble,
        Screen::Health,
        Screen::Recipes,
        Screen::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Screen::Overview => "概览",
            Screen::Assemble => "装配",
            Screen::Health => "健康",
            Screen::Recipes => "配方",
            Screen::Settings => "设置",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Screen::Overview => "▸",
            Screen::Assemble => "⚙",
            Screen::Health => "✚",
            Screen::Recipes => "▤",
            Screen::Settings => "✦",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }
}

/// 配方屏的五个域(与 `nw ls` 一致)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Domain {
    Recipes,
    Components,
    Sources,
    Profiles,
    Locks,
}

impl Domain {
    pub const ALL: [Domain; 5] = [
        Domain::Recipes,
        Domain::Components,
        Domain::Sources,
        Domain::Profiles,
        Domain::Locks,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Domain::Recipes => "配方",
            Domain::Components => "组件",
            Domain::Sources => "外部源",
            Domain::Profiles => "派",
            Domain::Locks => "锁定集",
        }
    }
}

/// 健康屏两种复核模式。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HealthMode {
    Doctor,
    Validate,
}

/// 全局消息。引擎事件与动画心跳都在这里。
#[derive(Clone, Debug)]
pub enum Message {
    Key(KeyEvent),
    Engine(EngineEvent),
    Tick,
}

pub struct App {
    pub quit: bool,
    pub help: bool,
    pub screen: Screen,
    pub cap: ColorCap,
    pub theme: ThemeKind,
    pub palette: Palette,
    /// 全局装配态:点亮顶栏灯,并禁用 q。
    pub assembling: bool,
    /// 呼吸灯的相位(装配中每 500ms 翻转)。
    pub blink: bool,
    pub nw_home: String,
    pub engine: EngineStatus,
    pub engine_version: String,
    pub runner: Arc<dyn CommandRunner>,

    // ---- 概览 ----
    pub ls: Option<LsData>,
    pub ls_loading: bool,
    pub ov_sel: usize,

    // ---- 装配 ----
    pub as_sel: usize,
    pub as_confirm: bool,
    pub as_log: Vec<String>,
    pub as_result: Option<EngineOutput>,

    // ---- 健康 ----
    pub hl_mode: HealthMode,
    pub hl_sel: usize,
    pub hl_result: Option<EngineOutput>,
    pub hl_loading: bool,

    // ---- 配方 ----
    pub rc_domain: Domain,
    pub rc_sel: usize,

    /// 最近动作流水,概览屏右下「最近日志」。
    pub log: Vec<String>,
    /// 键位处理器排队的引擎命令,由主循环取出执行(键位无 tx)。
    pub queue: Vec<Vec<String>>,
    /// 设置屏按 e 请求外部编辑器;主循环挂起终端再拉起(见 main)。
    pub pending_editor: bool,
    /// 启动 splash:800ms 或任意键跳过(R3 决议)。
    pub splash: bool,
    /// NW_HOME 手填输入模式(设置屏 i 进入;浮层优先级见 on_key)。
    pub home_input: bool,
    /// 手填缓冲区(只收可打印字符;上限 HOME_BUF_CAP)。
    pub home_buf: String,
    /// config.toml 解析失败的人话证据;None = 配置健康。
    pub config_error: Option<String>,
}

impl App {
    pub fn new() -> Self {
        Self::boot(None)
    }

    /// 开机装配:发现引擎(显式 --nw-bin 优先),建状态树。
    pub fn boot(explicit_nw_bin: Option<&str>) -> Self {
        let (runner, status, cfg_err) = crate::engine::boot(explicit_nw_bin);
        Self::assemble(runner, status, cfg_err)
    }

    /// 测试注入:完全可控的 runner(零真实进程)。
    pub fn with_runner(runner: Arc<dyn CommandRunner>) -> Self {
        let status = EngineStatus {
            found: true,
            bin: Some("<fake>".into()),
            source: "测试注入",
            attempted: Vec::new(),
        };
        Self::assemble(runner, status, None)
    }

    fn assemble(
        runner: Arc<dyn CommandRunner>,
        engine: EngineStatus,
        config_error: Option<String>,
    ) -> Self {
        let cap = term::detect_color_cap();
        let theme = ThemeKind::Warehouse;
        let palette = cached_degrade(theme, cap);
        let nw_home = std::env::var("NW_HOME")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| format!("{h}/.nous-warehouse"))
            })
            .unwrap_or_else(|| "~/.nous-warehouse".into());
        Self {
            quit: false,
            help: false,
            screen: Screen::Overview,
            cap,
            theme,
            palette,
            assembling: false,
            blink: false,
            nw_home,
            engine,
            engine_version: "探测中".into(),
            runner,
            ls: None,
            ls_loading: false,
            ov_sel: 0,
            as_sel: 0,
            as_confirm: false,
            as_log: Vec::new(),
            as_result: None,
            hl_mode: HealthMode::Doctor,
            hl_sel: 0,
            hl_result: None,
            hl_loading: false,
            rc_domain: Domain::Recipes,
            rc_sel: 0,
            log: vec!["驾驶舱已就绪".into()],
            queue: Vec::new(),
            pending_editor: false,
            splash: true,
            home_input: false,
            home_buf: String::new(),
            config_error,
        }
    }

    pub fn push_log(&mut self, line: impl Into<String>) {
        self.log.insert(0, line.into());
        if self.log.len() > 100 {
            self.log.truncate(100);
        }
    }

    /// 开机自举:版本探测 + 首次 ls(引擎缺失时 MissingRunner 自然给出证据)。
    pub fn bootstrap(&mut self, tx: &mpsc::UnboundedSender<EngineEvent>) {
        self.spawn_engine(commands::version(), tx);
        self.spawn_engine(commands::ls(None), tx);
    }

    /// 唯一的引擎调用入口:登记进行态,再交给 runner。
    pub fn spawn_engine(&mut self, args: Vec<String>, tx: &mpsc::UnboundedSender<EngineEvent>) {
        match args.first().map(String::as_str) {
            Some("up") => {
                self.assembling = true;
                self.as_confirm = false;
                self.as_log.clear();
                self.as_result = None;
                self.push_log(format!("装配启动:{}", args.join(" ")));
            }
            Some("ls") => {
                self.ls_loading = true;
            }
            Some("doctor") | Some("validate") => {
                self.hl_loading = true;
                self.hl_result = None;
            }
            _ => {}
        }
        self.runner.spawn(args, tx.clone());
    }

    pub fn update(&mut self, msg: Message) {
        match msg {
            Message::Key(k) => self.on_key(k),
            Message::Engine(ev) => self.on_engine(ev),
            Message::Tick => self.blink = !self.blink,
        }
    }

    fn on_engine(&mut self, ev: EngineEvent) {
        match ev {
            EngineEvent::Stderr(line) => {
                self.as_log.push(line);
                if self.as_log.len() > 500 {
                    self.as_log.drain(0..self.as_log.len() - 500);
                }
            }
            EngineEvent::Finished(out) => self.on_engine_finished(*out),
        }
    }

    fn on_engine_finished(&mut self, out: EngineOutput) {
        let cmd = out.args.first().cloned().unwrap_or_default();
        match cmd.as_str() {
            "--version" => {
                // nw --version 的裸版本号在 stderr
                self.engine_version = out
                    .stderr_tail
                    .iter()
                    .find_map(|l| {
                        let t = l.trim();
                        if t.is_empty() || t.starts_with("nw") {
                            None
                        } else {
                            Some(t.trim_start_matches("nw version ").to_string())
                        }
                    })
                    .unwrap_or_else(|| "未知".into());
                self.push_log(format!("引擎就绪:nw {}", self.engine_version));
            }
            "ls" => {
                self.ls_loading = false;
                match out.envelope.as_ref().and_then(|e| e.data_as::<LsData>()) {
                    Some(d) => {
                        self.ls = Some(d);
                        self.push_log("ls 五域已刷新".to_string());
                    }
                    None => {
                        self.push_log(format!("ls 结果不可用:{}", out.headline()));
                    }
                }
            }
            "up" => {
                self.assembling = false;
                let ok = out.ok;
                self.push_log(format!(
                    "装配结束:{} (rc {})",
                    if ok { "收敛" } else { "失败" },
                    out.exit_code.number()
                ));
                self.as_result = Some(out);
                if !ok {
                    self.screen = Screen::Assemble;
                }
            }
            "doctor" | "validate" => {
                self.hl_loading = false;
                self.push_log(format!("{cmd} 复核完成"));
                self.hl_result = Some(out);
            }
            _ => {}
        }
    }

    fn on_key(&mut self, k: KeyEvent) {
        if self.help {
            match k.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => self.help = false,
                _ => {}
            }
            return;
        }
        if self.as_confirm {
            self.on_key_confirm(k);
            return;
        }
        // 浮层优先级:帮助浮层 > 发射确认 > 手填输入 > 指路屏/常屏(见 spec #9)
        if self.home_input {
            self.on_key_home_input(k);
            return;
        }
        match k.code {
            KeyCode::Char('q') if !self.assembling => self.quit = true,
            KeyCode::Char('?') => self.help = true,
            // j/k 是壳级切屏;↑/↓ 留给各屏自己的列表(见 HUD)
            KeyCode::Char('j') | KeyCode::Tab => self.next_screen(),
            KeyCode::Char('k') | KeyCode::BackTab => self.prev_screen(),
            KeyCode::Char('t') => self.cycle_theme(),
            _ => self.screen_key(k),
        }
    }

    /// 发射仪式确认浮层:Enter 发射,Esc 返航。
    fn on_key_confirm(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Enter => self.launch_selected_lock(),
            KeyCode::Esc | KeyCode::Char('q') => {
                self.as_confirm = false;
                self.push_log("发射取消");
            }
            _ => {}
        }
    }

    fn screen_key(&mut self, k: KeyEvent) {
        match self.screen {
            Screen::Overview => self.key_overview(k),
            Screen::Assemble => self.key_assemble(k),
            Screen::Health => self.key_health(k),
            Screen::Recipes => self.key_recipes(k),
            Screen::Settings => self.key_settings(k),
        }
    }

    // ---- 各屏键位(纯状态变更,引擎调用经 spawn_engine) ----

    fn key_overview(&mut self, k: KeyEvent) {
        let n = self.ls.as_ref().map(|l| l.profiles.len()).unwrap_or(0);
        match k.code {
            KeyCode::Char('r') => self.refresh_ls(),
            KeyCode::Char('a') => self.screen = Screen::Assemble,
            KeyCode::Char('h') => self.screen = Screen::Health,
            KeyCode::Char('l') => self.screen = Screen::Recipes,
            KeyCode::Down | KeyCode::Char('n') => {
                if n > 0 {
                    self.ov_sel = (self.ov_sel + 1) % n;
                }
            }
            KeyCode::Up | KeyCode::Char('p') if n > 0 => {
                self.ov_sel = (self.ov_sel + n - 1) % n;
            }
            _ => {}
        }
    }

    fn key_assemble(&mut self, k: KeyEvent) {
        let n = self.ls.as_ref().map(|l| l.locks.len()).unwrap_or(0);
        match k.code {
            KeyCode::Down => {
                if n > 0 {
                    self.as_sel = (self.as_sel + 1) % n;
                }
            }
            KeyCode::Up => {
                if n > 0 {
                    self.as_sel = (self.as_sel + n - 1) % n;
                }
            }
            KeyCode::Enter if !self.assembling && n > 0 => self.as_confirm = true,
            _ => {}
        }
    }

    fn key_health(&mut self, k: KeyEvent) {
        let n = self
            .hl_result
            .as_ref()
            .map(|r| r.evidence().len())
            .unwrap_or(0);
        match k.code {
            KeyCode::Char('m') => {
                self.hl_mode = match self.hl_mode {
                    HealthMode::Doctor => HealthMode::Validate,
                    HealthMode::Validate => HealthMode::Doctor,
                };
            }
            KeyCode::Char('r') => self.run_health(),
            KeyCode::Down if n > 0 => self.hl_sel = (self.hl_sel + 1) % n,
            KeyCode::Up if n > 0 => self.hl_sel = (self.hl_sel + n - 1) % n,
            _ => {}
        }
    }

    fn key_recipes(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('h') | KeyCode::Left => {
                self.rc_domain = prev_domain(self.rc_domain);
                self.rc_sel = 0;
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.rc_domain = next_domain(self.rc_domain);
                self.rc_sel = 0;
            }
            KeyCode::Down => {
                let n = self.domain_len();
                if n > 0 {
                    self.rc_sel = (self.rc_sel + 1) % n;
                }
            }
            KeyCode::Up => {
                let n = self.domain_len();
                if n > 0 {
                    self.rc_sel = (self.rc_sel + n - 1) % n;
                }
            }
            _ => {}
        }
    }

    fn key_settings(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('t') => self.cycle_theme(),
            // NW_HOME 轮换候选:当前值 > $HOME/.nous-warehouse > 配置里那个
            KeyCode::Char('n') => self.cycle_nw_home(),
            KeyCode::Char('i') => {
                self.home_input = true;
                self.home_buf = self.nw_home.clone();
                self.push_log("手填 NW_HOME:Enter 确认,Esc/q 放弃");
            }
            KeyCode::Char('e') => self.pending_editor = true,
            _ => {}
        }
    }

    /// 手填输入模式的键位:Enter 生效、Esc/q 放弃、Backspace 删、
    /// 可打印字符入栈(控制字符与带 CONTROL 修饰的键一律忽略)。
    fn on_key_home_input(&mut self, k: KeyEvent) {
        const CAP: usize = 512;
        match k.code {
            KeyCode::Enter => self.apply_home_input(),
            KeyCode::Esc | KeyCode::Char('q') => {
                self.home_input = false;
                self.home_buf.clear();
                self.push_log("取消手填 NW_HOME");
            }
            KeyCode::Backspace => {
                self.home_buf.pop();
            }
            KeyCode::Char(c) => {
                let ctrl = k
                    .modifiers
                    .contains(crossterm::event::KeyModifiers::CONTROL);
                if !ctrl && !c.is_control() && self.home_buf.chars().count() < CAP {
                    self.home_buf.push(c);
                }
            }
            _ => {}
        }
    }

    /// 手填生效:规范化(去空白 + `~` 展开)→ 与轮换同一条后果链。
    /// 空值/纯空格拒绝并留证据,不写任何状态、不退输入模式。
    pub fn apply_home_input(&mut self) {
        let trimmed = self.home_buf.trim().to_string();
        if trimmed.is_empty() {
            self.push_log("NW_HOME 手填被拒:空值,原值保持不变");
            return;
        }
        let expanded = crate::engine::discover::expand_tilde(&trimmed)
            .display()
            .to_string();
        self.home_input = false;
        self.home_buf.clear();
        self.apply_nw_home(expanded);
    }

    /// 轮换 NW_HOME:写进程环境(子进程继承)、持久化、顺手刷新 ls。
    pub fn cycle_nw_home(&mut self) {
        let home = std::env::var("HOME").unwrap_or_else(|_| "~".into());
        let from_cfg = crate::engine::config::load().0.nw_home;
        let mut candidates: Vec<String> = vec![self.nw_home.clone()];
        if let Ok(env) = std::env::var("NW_HOME") {
            if !env.is_empty() {
                candidates.push(env);
            }
        }
        candidates.push(format!("{home}/.nous-warehouse"));
        if let Some(c) = from_cfg {
            candidates.push(c);
        }
        // 保序去重:候选永远至少两个(当前值 + 家目录默认)
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|c| seen.insert(c.clone()));
        let cur = candidates
            .iter()
            .position(|c| *c == self.nw_home)
            .unwrap_or(0);
        let next = candidates[(cur + 1) % candidates.len()].clone();
        self.apply_nw_home(next);
    }

    /// NW_HOME 生效链(手填与轮换唯一入口):写状态 → 写进程环境供子进程
    /// 继承 → 持久化 config → 入队一次 ls。
    pub fn apply_nw_home(&mut self, next: String) {
        self.nw_home = next.clone();
        // 子进程经 std::env 继承 NW_HOME(edition 2021 下 set_var 可用)
        std::env::set_var("NW_HOME", &next);
        let mut cfg = crate::engine::config::load().0;
        cfg.nw_home = Some(next.clone());
        if let Err(e) = crate::engine::config::save(&cfg) {
            self.push_log(format!("NW_HOME 持久化失败:{e}"));
        }
        self.push_log(format!("NW_HOME 切换到 {next}"));
        self.refresh_ls();
    }

    /// 外部编辑器逃逸口(设置屏 e)。返回拉起命令;EDITOR/VISUAL 都缺则 None。
    pub fn editor_command() -> Option<String> {
        std::env::var("EDITOR")
            .ok()
            .or_else(|| std::env::var("VISUAL").ok())
            .filter(|s| !s.is_empty())
    }

    // ---- 需要 tx 的动作:由主循环调 ----

    /// 刷新 ls(概览 'r' 与设置屏共用)。
    pub fn refresh_ls(&mut self) {
        self.queue.push(commands::ls(None));
        self.push_log("ls 已入队");
    }

    /// 对当前选中派跑 doctor / validate。
    pub fn run_health(&mut self) {
        let profile = self
            .ls
            .as_ref()
            .and_then(|l| {
                l.profiles
                    .get(self.ov_sel.min(l.profiles.len().saturating_sub(1)))
            })
            .map(|p| p.name.clone())
            .unwrap_or_default();
        if profile.is_empty() {
            self.push_log("没有选中的派,先 ls");
            return;
        }
        match self.hl_mode {
            HealthMode::Doctor => self.queue.push(commands::doctor_profile(&profile, None)),
            HealthMode::Validate => self.queue.push(commands::validate_recipe(&profile)),
        }
    }

    /// 发射当前选中的锁定集。
    pub fn launch_selected_lock(&mut self) {
        let lock = self
            .ls
            .as_ref()
            .and_then(|l| l.locks.get(self.as_sel))
            .and_then(|l| l.lock_id.clone());
        if let Some(lock) = lock {
            self.queue.push(commands::up(&lock, None));
            self.as_confirm = false;
        } else {
            self.push_log("没有选中的锁定集");
        }
    }

    // ---- 通用 ----

    pub fn next_screen(&mut self) {
        let i = (self.screen.index() + 1) % Screen::ALL.len();
        self.screen = Screen::ALL[i];
    }

    pub fn prev_screen(&mut self) {
        let i = (self.screen.index() + Screen::ALL.len() - 1) % Screen::ALL.len();
        self.screen = Screen::ALL[i];
    }

    pub fn cycle_theme(&mut self) {
        self.theme = self.theme.next();
        self.palette = cached_degrade(self.theme, self.cap);
        self.push_log(format!("主题切换:{}", self.theme.name()));
    }

    /// 终端是否够大(概览与各屏共同的门槛)。
    pub fn fits(&self, w: u16, h: u16) -> bool {
        w >= term::MIN_W && h >= term::MIN_H
    }

    /// 选中派的状态(警示带与表格共用)。
    pub fn selected_profile_state(&self) -> Option<State> {
        let ls = self.ls.as_ref()?;
        let p = ls.profiles.get(self.ov_sel)?;
        p.state.as_deref().map(State::from)
    }

    pub fn domain_len(&self) -> usize {
        match self.ls.as_ref() {
            None => 0,
            Some(l) => match self.rc_domain {
                Domain::Recipes => l.recipes.len(),
                Domain::Components => l.components.len(),
                Domain::Sources => l.sources.len(),
                Domain::Profiles => l.profiles.len(),
                Domain::Locks => l.locks.len(),
            },
        }
    }
}

fn prev_domain(d: Domain) -> Domain {
    let i = Domain::ALL.iter().position(|x| *x == d).unwrap_or(0);
    Domain::ALL[(i + Domain::ALL.len() - 1) % Domain::ALL.len()]
}

fn next_domain(d: Domain) -> Domain {
    let i = Domain::ALL.iter().position(|x| *x == d).unwrap_or(0);
    Domain::ALL[(i + 1) % Domain::ALL.len()]
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::FakeRunner;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::sync::Arc;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn fake_app() -> App {
        let runner = Arc::new(FakeRunner::new(&[(
            "ls",
            0,
            r#"{"schema":"nw/ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[{"name":"desk","state":"drift","lockId":"9f8e"},{"name":"lab","state":"ok","lockId":"a1b2"}],"locks":[{"lockId":"9f8e","recipe":"research-desk","bakedAt":"2026-10-01"}]}}"#,
        )]));
        App::with_runner(runner)
    }

    #[tokio::test]
    async fn bootstrap_喂版本与_ls() {
        let mut app = fake_app();
        let (tx, mut rx) = mpsc::unbounded_channel();
        // 版本探测:先手动回一个 stderr 形态的 --version 完成事件
        app.bootstrap(&tx);
        let mut saw_ls = false;
        while let Ok(ev) = rx.try_recv() {
            let EngineEvent::Finished(o) = ev else {
                continue;
            };
            match o.args.first().map(String::as_str) {
                Some("--version") => {
                    let mut v = *o;
                    v.stderr_tail = vec!["0.1.0".into()];
                    app.update(Message::Engine(EngineEvent::Finished(Box::new(v))));
                }
                Some("ls") => {
                    app.update(Message::Engine(EngineEvent::Finished(o)));
                    saw_ls = true;
                }
                _ => {}
            }
        }
        assert!(saw_ls, "bootstrap 必须打 ls");
        assert!(app.ls.is_some(), "ls 数据应入状态树");
        assert_eq!(app.ls.as_ref().unwrap().profiles.len(), 2);
    }

    #[test]
    fn q_退出但装配态禁用() {
        let mut app = fake_app();
        app.update(Message::Key(key('q')));
        assert!(app.quit);
        let mut app = fake_app();
        app.assembling = true;
        app.update(Message::Key(key('q')));
        assert!(!app.quit);
    }

    #[test]
    fn 导航环绕() {
        let mut app = fake_app();
        app.update(Message::Key(key('k')));
        assert_eq!(app.screen, Screen::Settings);
        app.update(Message::Key(key('j')));
        assert_eq!(app.screen, Screen::Overview);
    }

    #[test]
    fn 概览选择派并跑复核() {
        let mut app = fake_app();
        app.ls = Some(
            serde_json::from_str::<LsData>(r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[{"name":"desk"},{"name":"lab"}],"locks":[]}"#)
                .unwrap(),
        );
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.ov_sel, 1);
        app.run_health();
        assert_eq!(
            app.queue.last().unwrap(),
            &vec!["doctor".to_string(), "--profile".into(), "lab".into()]
        );
    }

    #[test]
    fn 发射锁定集入队() {
        let mut app = fake_app();
        app.ls = Some(
            serde_json::from_str::<LsData>(r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[{"lockId":"9f8e"}]}"#)
                .unwrap(),
        );
        app.launch_selected_lock();
        assert_eq!(
            app.queue.last().unwrap(),
            &vec!["up".to_string(), "--lock".into(), "9f8e".into()]
        );
    }

    #[test]
    fn up_完成关闭装配态并留结果() {
        let mut app = fake_app();
        app.assembling = true;
        let out = EngineOutput {
            args: vec!["up".into()],
            exit_code: crate::engine::ExitCode::Ok,
            ok: true,
            stderr_tail: vec![],
            envelope: None,
            raw_stdout: "{}".into(),
            parse_error: None,
        };
        app.update(Message::Engine(EngineEvent::Finished(Box::new(out))));
        assert!(!app.assembling);
        assert!(app.as_result.is_some());
    }

    // ---- #9 NW_HOME 手填输入(App 行为缝) ----

    /// 摸进程环境/配置文件的测试串行化,免得并行互相踩。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// 持锁 + 隔离 config 落点:会触发持久化的测试都必须先拿这个。
    fn env_locked() -> (
        std::sync::MutexGuard<'static, ()>,
        crate::testutil::ConfigGuard,
    ) {
        let g = ENV_LOCK.lock().unwrap();
        let cfg = crate::testutil::ConfigGuard::install();
        (g, cfg)
    }

    fn settings_app() -> App {
        let mut app = fake_app();
        app.screen = Screen::Settings;
        app
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            app.update(Message::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::NONE,
            )));
        }
    }

    #[test]
    fn 手填_键入回显并生效同链() {
        let (_g, _cfg) = env_locked();
        let mut app = settings_app();
        app.update(Message::Key(key('i')));
        assert!(app.home_input);
        type_str(&mut app, "/data/pi-homes");
        assert_eq!(app.home_buf, app.nw_home.clone() + "/data/pi-homes");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let target = app.nw_home.clone();
        assert!(target.ends_with("/data/pi-homes"), "新值落进状态树");
        assert_eq!(std::env::var("NW_HOME").unwrap(), target, "进程环境同步");
        assert!(
            app.queue
                .iter()
                .any(|a| a.first().map(String::as_str) == Some("ls")),
            "同链入队 ls"
        );
        assert!(!app.home_input && app.home_buf.is_empty(), "用完即退");
    }

    #[test]
    fn 手填_q与_esc_放弃且不退出() {
        let _g = ENV_LOCK.lock().unwrap();
        let before = settings_app().nw_home;
        let mut app = settings_app();
        app.update(Message::Key(key('i')));
        type_str(&mut app, "/tmp/whatever");
        app.update(Message::Key(key('q')));
        assert!(!app.quit, "输入态 q = 放弃,不是退出");
        assert!(!app.home_input);
        assert_eq!(app.nw_home, before, "放弃不碰状态");
        assert!(app.home_buf.is_empty());
        // Esc 同理
        let mut app = settings_app();
        app.update(Message::Key(key('i')));
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(!app.home_input);
        assert!(!app.quit);
    }

    #[test]
    fn 手填_空值与纯空格拒绝且留证据() {
        let _g = ENV_LOCK.lock().unwrap();
        let mut app = settings_app();
        let before = app.nw_home.clone();
        let queued = app.queue.len();
        app.update(Message::Key(key('i')));
        app.home_buf = "   ".into();
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.nw_home, before, "拒绝不改状态");
        assert_eq!(app.queue.len(), queued, "拒绝不入队 ls");
        assert!(app.home_input, "拒绝后留在输入态可改");
        assert!(app.log[0].contains("被拒"), "日志留证据");
    }

    #[cfg(unix)]
    #[test]
    fn 手填_tilde展开成绝对路径() {
        let (_g, _cfg) = env_locked();
        let home = std::env::var("HOME").unwrap();
        let mut app = settings_app();
        app.update(Message::Key(key('i')));
        app.home_buf = "~/pi-space".into();
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.nw_home, format!("{home}/pi-space"));
    }

    #[test]
    fn 输入模式_只收可打印字符() {
        let mut app = settings_app();
        app.update(Message::Key(key('i')));
        app.home_buf.clear();
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('\u{7}'),
            KeyModifiers::NONE,
        )));
        assert!(app.home_buf.is_empty(), "控制字符不入栈");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert!(app.home_buf.is_empty(), "带 CONTROL 的键不入栈");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('x'),
            KeyModifiers::SHIFT,
        )));
        assert_eq!(app.home_buf, "x", "普通可打印字符正常入栈");
    }

    #[test]
    fn 手填缓冲区_512上限() {
        let mut app = settings_app();
        app.update(Message::Key(key('i')));
        app.home_buf.clear();
        type_str(&mut app, &"a".repeat(600));
        assert_eq!(app.home_buf.chars().count(), 512, "上限防误粘贴");
    }

    #[test]
    fn 手填与轮换共用生效链() {
        let (_g, _cfg) = env_locked();
        let mut app = settings_app();
        app.apply_nw_home("/data/shared".into());
        let a = app.nw_home.clone();
        let qa = app.queue.len();
        let mut app2 = settings_app();
        app2.apply_nw_home("/data/shared".into());
        assert_eq!(a, app2.nw_home);
        assert_eq!(app.queue.len(), qa, "两个入口的入队行为一致");
    }
}
