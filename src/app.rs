//! 应用外壳:Elm 式集中状态 + update + 屏幕定义。
//!
//! V1 只有一棵状态树;所有交互先变成 [`Message`],由 [`App::update`]
//! 收口,屏幕渲染函数无副作用。引擎调用只能经 [`App::spawn_engine`]。

use std::sync::Arc;
use tokio::sync::mpsc;

use crate::engine::{
    commands, CommandRunner, EngineEvent, EngineOutput, EngineStatus, LsData, State,
};
use crate::layout::{hitmap, size_class, ScrollDir, Target};
use crate::term::{self, ColorCap};
use crate::theme::{cached_degrade, Palette, ThemeKind};
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

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
    Mouse(MouseEvent),
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
    /// 装配 stderr 日志的滚轮偏移(#21):0=跟随最新,向上滚增、向下
    /// 滚减,钳在 [0, len)。只影响 log_block 取窗,七步判定仍读全量。
    pub as_log_offset: usize,
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
    /// 鼠标捕获是否开启(默认开;两联 = 配置 mouse_capture + --no-mouse,
    /// 见 ADR-0005)。term::init 按此门控捕获序列。
    pub mouse_enabled: bool,
    /// 最近一次渲染的终端面,供命中定标(主循环每帧刷新;
    /// 命中几何见 layout::hitmap)。
    pub size: Rect,
}

impl App {
    pub fn new() -> Self {
        Self::boot(None)
    }

    /// 开机装配:发现引擎(显式 --nw-bin 优先),建状态树。
    pub fn boot(explicit_nw_bin: Option<&str>) -> Self {
        let (runner, status, cfg_err) = crate::engine::boot(explicit_nw_bin);
        let mut app = Self::assemble(runner, status, cfg_err);
        // 鼠标捕获两联之配置一联(缺省/坏配置降级 → 开);
        // --no-mouse 旗标由 main 另行并入(apply_no_mouse_flag)。
        app.mouse_enabled = crate::engine::config::load().0.mouse_capture_enabled();
        app
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
            as_log_offset: 0,
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
            mouse_enabled: true,
            size: Rect::default(),
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
                self.as_log_offset = 0; // 新一轮装配从最新一行跟起
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
            Message::Mouse(ev) => self.on_mouse(ev),
            Message::Engine(ev) => self.on_engine(ev),
            Message::Tick => self.blink = !self.blink,
        }
    }

    /// 鼠标事件分派（spec #15 ticket #20/#21）：命中区取自与渲染同源的
    /// [`hitmap`]——先借出目标（owned [`Target`]，随即释放对 `self` 的
    /// 不可变借用）再改状态，规避自反借用；只读命中区、不硬编码矩形
    /// （#17/#18 细化紧凑几何后自动跟随）。左键按下做点击分派，滚轮做
    /// #21 的移选中/日志翻看；释放是配对尾巴不二次触发，右键/拖拽不在
    /// V1 范围（ADR-0005）。
    fn on_mouse(&mut self, ev: MouseEvent) {
        let scroll = match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => None,
            MouseEventKind::ScrollUp => Some(ScrollDir::Up),
            MouseEventKind::ScrollDown => Some(ScrollDir::Down),
            _ => return,
        };
        let size = self.size;
        let hits = hitmap(size, self, size_class(size));
        let pos = Position::new(ev.column, ev.row);
        let Some(target) = hits
            .into_iter()
            .find(|(r, _)| r.contains(pos))
            .map(|(_, t)| t)
        else {
            return; // 点/滚空白:不改任何状态
        };
        // 滚轮(#21):只受理可选列表与装配 stderr 日志区——滚到导航/
        // 仪式按钮/浮层目标上不动作(滚轮不切屏、不点火、不关浮层);
        // 方向取自事件,ScrollLog 内的方向不参与分派(命中区纯几何)。
        if let Some(dir) = scroll {
            match target {
                Target::SelectRow(_) => self.wheel_sel(dir),
                Target::ScrollLog(_) => self.wheel_log(dir),
                _ => {}
            }
            return;
        }
        match target {
            // 点导航(常畅)/屏 Tab(紧凑)切屏,与 j/k 同语义。
            Target::NavTo(s) => self.screen = s,
            // 点行=选中(与 ↑/↓ 同语义),只选中、不触发破坏性动作;
            // 命中区只产自当前屏,按 self.screen 路由对应选中态。
            Target::SelectRow(i) => match self.screen {
                Screen::Overview => self.ov_sel = i,
                Screen::Assemble => self.as_sel = i,
                Screen::Health => self.hl_sel = i,
                Screen::Recipes => self.rc_sel = i,
                Screen::Settings => {}
            },
            // 仪式「Enter 发射」段:与键盘 Enter 同一路(入队 up、关浮层)。
            Target::Activate => self.launch_selected_lock(),
            // 帮助「任意处关」与仪式「Esc 返航」:与键盘 Esc 同一路
            // (日志区左键无动作,翻看归上面的滚轮分支)。
            Target::DismissPopup => self.dismiss_popup(),
            // 左键点日志区不动作;翻看由上面的滚轮分支受理(#21)。
            Target::ScrollLog(_) => {}
        }
    }

    /// 滚轮在可选列表上移选中(#21):上下各一档,与键盘同步进;
    /// 边界按票面钉死「到顶/到底不越界、不绕回」——到顶停顶、到底停底,
    /// 落在 [0, n) (既有键盘代码走 % n 会绕回,那是键盘行为,
    /// 键盘零回归不动它)。
    fn wheel_sel(&mut self, dir: ScrollDir) {
        let (sel, n) = match self.screen {
            Screen::Overview => {
                let n = self.ls.as_ref().map(|l| l.profiles.len()).unwrap_or(0);
                (&mut self.ov_sel, n)
            }
            Screen::Assemble => {
                let n = self.ls.as_ref().map(|l| l.locks.len()).unwrap_or(0);
                (&mut self.as_sel, n)
            }
            Screen::Health => {
                let n = self
                    .hl_result
                    .as_ref()
                    .map(|r| r.evidence().len())
                    .unwrap_or(0);
                (&mut self.hl_sel, n)
            }
            Screen::Recipes => {
                let n = self.domain_len();
                (&mut self.rc_sel, n)
            }
            Screen::Settings => return, // 设置屏无可滚列表
        };
        if n == 0 {
            return;
        }
        match dir {
            ScrollDir::Down => *sel = (*sel + 1).min(n - 1),
            ScrollDir::Up => *sel = sel.saturating_sub(1),
        }
    }

    /// 滚轮在装配 stderr 实时日志区翻看(#21):偏移 0=跟随最新,
    /// 向上增、向下减,钳在 [0, len)(空日志不动,drain 后的陈旧偏移
    /// 自愈),与 render log_block 的取窗衔接。
    fn wheel_log(&mut self, dir: ScrollDir) {
        let max = self.as_log.len().saturating_sub(1);
        let cur = self.as_log_offset.min(max);
        self.as_log_offset = match dir {
            ScrollDir::Up => (cur + 1).min(max),
            ScrollDir::Down => cur.saturating_sub(1),
        };
    }

    /// 关闭当前浮层,与键盘 Esc 完全同路:帮助只关灯,发射仪式
    /// 「返航」取消并留日志(鼠标/键盘共用,语义不许漂移)。
    fn dismiss_popup(&mut self) {
        if self.help {
            self.help = false;
        } else if self.as_confirm {
            self.as_confirm = false;
            self.push_log("发射取消");
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
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => self.dismiss_popup(),
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
            KeyCode::Esc | KeyCode::Char('q') => self.dismiss_popup(),
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
            KeyCode::Char('m') => self.toggle_mouse_capture(),
            _ => {}
        }
    }

    /// 逃逸两联之一:设置屏 m 开关。翻状态 + 持久化 config
    /// (重启生效);会话内不重配活动终端——运行时热键已被砍掉(ADR-0005)。
    pub fn toggle_mouse_capture(&mut self) {
        self.mouse_enabled = !self.mouse_enabled;
        let mut cfg = crate::engine::config::load().0;
        cfg.mouse_capture = Some(self.mouse_enabled);
        if let Err(e) = crate::engine::config::save(&cfg) {
            self.push_log(format!("鼠标捕获偏好持久化失败:{e}"));
        }
        self.push_log(format!(
            "鼠标捕获偏好:{}(重启生效)",
            if self.mouse_enabled { "开" } else { "关" }
        ));
    }

    /// 逃逸两联之一:--no-mouse 旗标并入(只收紧不放松,已在
    /// 运行中开启的捕获不会被旗标回开)。
    pub fn apply_no_mouse_flag(&mut self, no_mouse: bool) {
        if no_mouse {
            self.mouse_enabled = false;
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
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEventKind};
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

    // ---- #19 鼠标接线:统一事件入口 + 逃逸两联 ----

    fn mouse_at(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn 鼠标消息灌入_状态不动_不panic() {
        let mut app = fake_app();
        app.size = Rect::new(0, 0, 66, 50);
        let before = (
            app.screen,
            app.ov_sel,
            app.as_sel,
            app.rc_sel,
            app.hl_sel,
            app.queue.len(),
            app.home_input,
        );
        // 点击/滚轮/释放都灌进统一入口。坐标取顶栏行(#16 钉住的
        // 「顶栏无命中区」):#20 分派上线后,点无命中区仍零变化;
        // 分派语义见下方 #20 测试。
        for ev in [
            mouse_at(MouseEventKind::Down(MouseButton::Left), 3, 0),
            mouse_at(MouseEventKind::ScrollDown, 5, 0),
            mouse_at(MouseEventKind::Up(MouseButton::Left), 3, 0),
        ] {
            app.update(Message::Mouse(ev));
        }
        assert_eq!(
            (
                app.screen,
                app.ov_sel,
                app.as_sel,
                app.rc_sel,
                app.hl_sel,
                app.queue.len(),
                app.home_input,
            ),
            before,
            "点无命中区不分派:状态与入队原样(键盘零回归)"
        );
        assert_eq!(
            app.size,
            Rect::new(0, 0, 66, 50),
            "size 是定标面,不被事件改写"
        );
    }

    #[test]
    fn no_mouse_旗标改变_mouse_enabled() {
        let mut app = fake_app();
        assert!(app.mouse_enabled, "默认开");
        app.apply_no_mouse_flag(false);
        assert!(app.mouse_enabled, "无旗标不动");
        app.apply_no_mouse_flag(true);
        assert!(!app.mouse_enabled, "--no-mouse 关掉捕获");
    }

    #[test]
    fn 设置屏_m_开关改变_mouse_enabled_并持久化() {
        let (_g, _cfg) = env_locked();
        let mut app = settings_app();
        assert!(app.mouse_enabled, "默认开");
        app.update(Message::Key(key('m')));
        assert!(!app.mouse_enabled, "设置开关改变 mouse_enabled");
        let (cfg, _) = crate::engine::config::load();
        assert_eq!(cfg.mouse_capture, Some(false), "写入 config,重启仍生效");
        assert!(app.log[0].contains("鼠标捕获"), "日志留证据");
        app.update(Message::Key(key('m')));
        assert!(app.mouse_enabled, "再按翻回开");
        let (cfg2, _) = crate::engine::config::load();
        assert_eq!(cfg2.mouse_capture, Some(true));
    }

    // ---- #20 命中分派:点哪打哪(坐标取自布局模型,不硬编码矩形) ----

    /// 同一夹具(3 派 + 2 锁定集):概览/装配/配方/健康四屏都有可点行,
    /// 且概览第 3 行(y=6)落在帮助浮层内——浮层点击优先的测试前置。
    fn mouse_ls_app() -> App {
        let mut app = fake_app();
        app.ls = Some(
            serde_json::from_str::<LsData>(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[{"name":"desk","state":"drift","lockId":"9f8e"},{"name":"lab","state":"ok","lockId":"a1b2"},{"name":"zen","state":"ok","lockId":"c3d4"}],"locks":[{"lockId":"9f8e","recipe":"research-desk","bakedAt":"2026-10-01"},{"lockId":"a1b2","recipe":"research-desk","bakedAt":"2026-10-02"}]}"#,
            )
            .unwrap(),
        );
        app
    }

    /// 取某目标的命中矩形(与渲染同源的布局模型,点哪拿哪)。
    fn hit_target(app: &App, pred: impl Fn(&Target) -> bool) -> Rect {
        let class = size_class(app.size);
        hitmap(app.size, app, class)
            .into_iter()
            .find(|(_, t)| pred(t))
            .map(|(r, _)| r)
            .expect("该目标必须有命中区")
    }

    /// 左键单击矩形中心(Down;释放不参与分派)。
    fn click(app: &mut App, r: Rect) {
        app.update(Message::Mouse(mouse_at(
            MouseEventKind::Down(MouseButton::Left),
            r.x + r.width / 2,
            r.y + r.height / 2,
        )));
    }

    #[test]
    fn 点导航与屏tab切屏_两档同语义() {
        let mut app = mouse_ls_app();
        app.screen = Screen::Overview;
        // 常畅态:点竖左导航
        app.size = Rect::new(0, 0, 80, 30);
        let nav = hit_target(&app, |t| matches!(t, Target::NavTo(Screen::Health)));
        click(&mut app, nav);
        assert_eq!(app.screen, Screen::Health, "点常畅态竖导航切屏");
        // 紧凑态:点顶部横向屏 Tab
        app.size = Rect::new(0, 0, 66, 50);
        let tab = hit_target(&app, |t| matches!(t, Target::NavTo(Screen::Recipes)));
        click(&mut app, tab);
        assert_eq!(app.screen, Screen::Recipes, "点紧凑态屏 Tab 切屏");
        // 释放(Up)不参与分派,不二次触发
        app.update(Message::Mouse(mouse_at(
            MouseEventKind::Up(MouseButton::Left),
            tab.x,
            tab.y,
        )));
        assert_eq!(app.screen, Screen::Recipes, "Up 不分派");
        // 非左键不在 V1 范围,不改状态(ADR-0005 右键/拖拽未纳入)
        app.update(Message::Mouse(mouse_at(
            MouseEventKind::Down(MouseButton::Right),
            tab.x,
            tab.y,
        )));
        assert_eq!(app.screen, Screen::Recipes, "右键不分派");
    }

    #[test]
    fn 点列表行选中_当前屏对应选中态() {
        let mut app = mouse_ls_app();
        app.size = Rect::new(0, 0, 80, 30);
        // 概览派表
        app.screen = Screen::Overview;
        let row = hit_target(&app, |t| matches!(t, Target::SelectRow(1)));
        click(&mut app, row);
        assert_eq!(app.ov_sel, 1, "概览派表点行选中");
        // 装配锁定集
        app.screen = Screen::Assemble;
        let row = hit_target(&app, |t| matches!(t, Target::SelectRow(1)));
        click(&mut app, row);
        assert_eq!(app.as_sel, 1, "装配锁定集点行选中");
        // 配方列表(切到有数据的域)
        app.screen = Screen::Recipes;
        app.rc_domain = Domain::Profiles;
        let row = hit_target(&app, |t| matches!(t, Target::SelectRow(1)));
        click(&mut app, row);
        assert_eq!(app.rc_sel, 1, "配方列表点行选中");
        // 健康 validate 项(证据两条)
        app.screen = Screen::Health;
        app.hl_mode = HealthMode::Validate;
        app.update(Message::Engine(EngineEvent::Finished(Box::new(
            EngineOutput {
                args: vec!["validate".into(), "--recipe".into(), "x".into()],
                exit_code: crate::engine::ExitCode::from_number(3),
                ok: false,
                stderr_tail: vec![],
                envelope: Some(
                    serde_json::from_str(
                        r#"{"schema":"nw/validate.v1","nw":"0.1.0","ok":false,"errors":[{"code":"A","exitCode":3,"message":"甲","hint":"h","details":{}},{"code":"B","exitCode":3,"message":"乙","hint":"h","details":{}}],"warnings":[],"data":{"results":[],"summary":{"total":2,"pass":0,"warn":0,"fail":2,"skip":0}}}"#,
                    )
                    .unwrap(),
                ),
                raw_stdout: "".into(),
                parse_error: None,
            },
        ))));
        let row = hit_target(&app, |t| matches!(t, Target::SelectRow(1)));
        click(&mut app, row);
        assert_eq!(app.hl_sel, 1, "健康 validate 项点行选中");
        // 点行只是选中:不入队、不开仪式(安全语义与键盘一致,故事 31)
        assert!(app.queue.is_empty(), "点行不入队");
        assert!(!app.as_confirm, "点行不触发破坏性动作");
    }

    #[test]
    fn 点空白不改状态_顶栏与hud行() {
        let mut app = mouse_ls_app();
        app.screen = Screen::Overview;
        app.size = Rect::new(0, 0, 80, 30);
        let before = (
            app.screen,
            app.ov_sel,
            app.as_sel,
            app.rc_sel,
            app.hl_sel,
            app.queue.len(),
            app.as_confirm,
            app.help,
        );
        // 顶栏(y=0)与 HUD(y=29)都不在命中区内(#16 钉住)
        app.update(Message::Mouse(mouse_at(
            MouseEventKind::Down(MouseButton::Left),
            0,
            0,
        )));
        app.update(Message::Mouse(mouse_at(
            MouseEventKind::Down(MouseButton::Left),
            79,
            29,
        )));
        assert_eq!(
            (
                app.screen,
                app.ov_sel,
                app.as_sel,
                app.rc_sel,
                app.hl_sel,
                app.queue.len(),
                app.as_confirm,
                app.help,
            ),
            before,
            "点空白零变化"
        );
    }

    #[test]
    fn 点发射入队up_点返航取消留日志() {
        let mut app = mouse_ls_app();
        app.screen = Screen::Assemble;
        app.size = Rect::new(0, 0, 80, 30);
        // 发射:与键盘 Enter 同路
        app.as_confirm = true;
        let act = hit_target(&app, |t| *t == Target::Activate);
        click(&mut app, act);
        assert_eq!(
            app.queue.last().unwrap(),
            &vec!["up".to_string(), "--lock".into(), "9f8e".into()],
            "点发射与 Enter 同路入队 up"
        );
        assert!(!app.as_confirm, "发射后仪式关闭");
        // 返航:与键盘 Esc 同路,取消并留日志、不入队
        app.as_confirm = true;
        let back = hit_target(&app, |t| *t == Target::DismissPopup);
        let qlen = app.queue.len();
        click(&mut app, back);
        assert!(!app.as_confirm, "点返航关闭仪式");
        assert_eq!(app.queue.len(), qlen, "返航不入队");
        assert!(
            app.log.iter().any(|l| l.contains("发射取消")),
            "返航留日志:发射取消"
        );
    }

    #[test]
    fn 点帮助任意处关_浮层外点击不作用底层() {
        let mut app = mouse_ls_app();
        app.screen = Screen::Overview;
        app.size = Rect::new(0, 0, 80, 30);
        // 先拿到帮助浮层下压着的导航矩形(浮层打开时命中区只剩浮层)
        let nav = hit_target(&app, |t| matches!(t, Target::NavTo(Screen::Health)));
        // 浮层外(导航上)点击:浮层吞掉,底层不作用、浮层不关
        app.help = true;
        click(&mut app, nav);
        assert!(app.help, "浮层外点击不关闭浮层");
        assert_eq!(app.screen, Screen::Overview, "浮层外点击不切屏(不误触底层)");
        // 浮层内点击:关闭(点任意处,与 Esc/?/q 同语义)
        let dm = hit_target(&app, |t| *t == Target::DismissPopup);
        click(&mut app, dm);
        assert!(!app.help, "点帮助浮层任意处关闭");
        // 帮助浮层盖着的派表行(行心落在浮层内,行心定位用上一段拿到的
        // 浮层矩形 dm):点那里只关浮层,选中不动——浮层内点击优先、不穿透到底层
        let class = size_class(app.size);
        let row = hitmap(app.size, &app, class)
            .iter()
            .find(|(r, t)| {
                matches!(t, Target::SelectRow(_))
                    && dm.contains(Position::new(r.x + r.width / 2, r.y + r.height / 2))
            })
            .map(|(r, _)| *r)
            .expect("浮层内有被盖住的派表行");
        app.help = true;
        click(&mut app, row);
        assert!(!app.help, "浮层内点击生效:帮助关闭");
        assert_eq!(app.ov_sel, 0, "浮层内点击不穿透到列表选中");
    }

    #[test]
    fn 装配态下点击_无点火与选中目标() {
        let mut app = mouse_ls_app();
        app.screen = Screen::Assemble;
        app.size = Rect::new(0, 0, 80, 30);
        app.assembling = true;
        // 运行视图替换选集列表(assemble_hits 随渲染同档短路):命中区里
        // 既无 Activate 也无 SelectRow——鼠标无从点火/改选,与键盘 Enter 的
        // !assembling 门控、引擎侧 profile 级并发锁同一约束(故事 17)
        let class = size_class(app.size);
        let hits = hitmap(app.size, &app, class);
        assert!(
            hits.iter().all(|(_, t)| *t != Target::Activate),
            "装配态无 Activate 命中区"
        );
        assert!(
            hits.iter().all(|(_, t)| !matches!(t, Target::SelectRow(_))),
            "装配态无列表选中目标(运行视图)"
        );
        // 点原本是列表的位置:零变化
        let before = (app.as_sel, app.queue.len(), app.assembling);
        app.update(Message::Mouse(mouse_at(
            MouseEventKind::Down(MouseButton::Left),
            30,
            10,
        )));
        assert_eq!(
            (app.as_sel, app.queue.len(), app.assembling),
            before,
            "装配态点击零变化"
        );
    }

    #[test]
    fn 手填浮层下点击_不改任何状态() {
        let mut app = mouse_ls_app();
        app.size = Rect::new(0, 0, 80, 30);
        app.screen = Screen::Settings;
        // 拿到浮层下压着的导航矩形(浮层未开时)
        let nav = hit_target(&app, |t| matches!(t, Target::NavTo(Screen::Health)));
        app.home_input = true;
        app.home_buf = "/x".into();
        click(&mut app, nav);
        assert_eq!(app.screen, Screen::Settings, "手填浮层下鼠标惰性:不切屏");
        assert_eq!(app.home_buf, "/x", "输入缓冲不被鼠标扰动(故事 16)");
    }

    // ---- #21 滚轮滚动:列表移选中 + 装配 stderr 偏移 ----

    /// 在命中矩形中心滚一格。
    fn wheel(app: &mut App, r: Rect, kind: MouseEventKind) {
        app.update(Message::Mouse(mouse_at(
            kind,
            r.x + r.width / 2,
            r.y + r.height / 2,
        )));
    }

    #[test]
    fn 滚轮移选中_四屏各一档_边界到顶停顶到底停底() {
        let mut app = mouse_ls_app();
        app.size = Rect::new(0, 0, 80, 30);
        // 概览派表(3 派):与键盘同步进;边界按票面不绕回
        app.screen = Screen::Overview;
        let row = hit_target(&app, |t| matches!(t, Target::SelectRow(_)));
        wheel(&mut app, row, MouseEventKind::ScrollDown);
        assert_eq!(app.ov_sel, 1, "滚一格 = 键盘 Down 一档");
        wheel(&mut app, row, MouseEventKind::ScrollDown);
        wheel(&mut app, row, MouseEventKind::ScrollDown);
        assert_eq!(app.ov_sel, 2, "到底停底(3 派),不绕回");
        wheel(&mut app, row, MouseEventKind::ScrollUp);
        assert_eq!(app.ov_sel, 1, "向上一档");
        wheel(&mut app, row, MouseEventKind::ScrollUp);
        wheel(&mut app, row, MouseEventKind::ScrollUp);
        assert_eq!(app.ov_sel, 0, "到顶停顶,不绕回到 2");
        // 装配锁定集(2 锁)
        app.screen = Screen::Assemble;
        let row = hit_target(&app, |t| matches!(t, Target::SelectRow(_)));
        wheel(&mut app, row, MouseEventKind::ScrollDown);
        assert_eq!(app.as_sel, 1, "装配列表滚一档");
        wheel(&mut app, row, MouseEventKind::ScrollDown);
        assert_eq!(app.as_sel, 1, "2 锁到底停底");
        wheel(&mut app, row, MouseEventKind::ScrollUp);
        assert_eq!(app.as_sel, 0, "装配列表回顶");
        // 配方列表(Profiles 域 3 条)
        app.screen = Screen::Recipes;
        app.rc_domain = Domain::Profiles;
        let row = hit_target(&app, |t| matches!(t, Target::SelectRow(_)));
        wheel(&mut app, row, MouseEventKind::ScrollDown);
        assert_eq!(app.rc_sel, 1, "配方列表滚一档");
        wheel(&mut app, row, MouseEventKind::ScrollUp);
        assert_eq!(app.rc_sel, 0, "配方列表回顶");
        // 健康 validate 项(证据两条)
        app.screen = Screen::Health;
        app.hl_mode = HealthMode::Validate;
        app.update(Message::Engine(EngineEvent::Finished(Box::new(
            EngineOutput {
                args: vec!["validate".into(), "--recipe".into(), "x".into()],
                exit_code: crate::engine::ExitCode::from_number(3),
                ok: false,
                stderr_tail: vec![],
                envelope: Some(
                    serde_json::from_str(
                        r#"{"schema":"nw/validate.v1","nw":"0.1.0","ok":false,"errors":[{"code":"A","exitCode":3,"message":"甲","hint":"h","details":{}},{"code":"B","exitCode":3,"message":"乙","hint":"h","details":{}}],"warnings":[],"data":{"results":[],"summary":{"total":2,"pass":0,"warn":0,"fail":2,"skip":0}}}"#,
                    )
                    .unwrap(),
                ),
                raw_stdout: "".into(),
                parse_error: None,
            },
        ))));
        let row = hit_target(&app, |t| matches!(t, Target::SelectRow(_)));
        wheel(&mut app, row, MouseEventKind::ScrollDown);
        assert_eq!(app.hl_sel, 1, "健康列表滚一档");
        wheel(&mut app, row, MouseEventKind::ScrollDown);
        assert_eq!(app.hl_sel, 1, "健康列表到底停底");
        // 滚轮只是选中:不入队、不开仪式(安全语义与键盘一致,故事 31)
        assert!(app.queue.is_empty(), "滚轮不入队");
        assert!(!app.as_confirm, "滚轮不触发破坏性动作");
    }

    #[test]
    fn 滚轮只受理列表与日志区_导航与浮层上滚动不动状态() {
        let mut app = mouse_ls_app();
        app.size = Rect::new(0, 0, 80, 30);
        app.screen = Screen::Overview;
        // 滚到竖导航上:不切屏(切屏只归点击/键位)
        let nav = hit_target(&app, |t| matches!(t, Target::NavTo(Screen::Health)));
        wheel(&mut app, nav, MouseEventKind::ScrollDown);
        assert_eq!(app.screen, Screen::Overview, "滚导航不切屏");
        // 概览「最近日志」等纯文本区不入滚动范围:命中清单里无 ScrollLog
        assert!(
            !hitmap(app.size, &app, size_class(app.size))
                .iter()
                .any(|(_, t)| matches!(t, Target::ScrollLog(_))),
            "概览屏不产日志滚动目标(最近日志维持截断)"
        );
        wheel(&mut app, Rect::new(60, 20, 8, 3), MouseEventKind::ScrollUp);
        assert_eq!(app.as_log_offset, 0, "非目标区滚动不产偏移");
        // 帮助浮层:滚到浮层上不关浮层(浮层优先级对滚轮同样成立)
        app.help = true;
        let popup = hit_target(&app, |t| matches!(t, Target::DismissPopup));
        wheel(&mut app, popup, MouseEventKind::ScrollDown);
        assert!(app.help, "滚轮不关帮助浮层(点才关)");
        assert_eq!(app.ov_sel, 0, "浮层下滚动不作用底层列表");
    }

    #[test]
    fn 滚轮在stderr日志区_偏移增减_边界不得越界() {
        // 两档尺寸都覆盖:ScrollLog 区随 render_running 同款分栏产出
        for (w, h) in [(80, 30), (66, 50)] {
            let mut app = mouse_ls_app();
            app.size = Rect::new(0, 0, w, h);
            app.screen = Screen::Assemble;
            app.assembling = true;
            app.as_log = (0..8).map(|i| format!("行{i:02}")).collect();
            let log = hit_target(&app, |t| matches!(t, Target::ScrollLog(_)));
            wheel(&mut app, log, MouseEventKind::ScrollUp);
            assert_eq!(app.as_log_offset, 1, "{w}×{h} 向上翻一格");
            wheel(&mut app, log, MouseEventKind::ScrollUp);
            assert_eq!(app.as_log_offset, 2, "{w}×{h} 再翻一格");
            wheel(&mut app, log, MouseEventKind::ScrollDown);
            wheel(&mut app, log, MouseEventKind::ScrollDown);
            wheel(&mut app, log, MouseEventKind::ScrollDown);
            assert_eq!(app.as_log_offset, 0, "{w}×{h} 回到底部停住,不越界");
            for _ in 0..20 {
                wheel(&mut app, log, MouseEventKind::ScrollUp);
            }
            assert_eq!(app.as_log_offset, 7, "{w}×{h} 偏移停在 len-1,不越界");
            assert_eq!(app.as_sel, 0, "滚日志不动选中");
            assert!(app.queue.is_empty(), "滚日志不入队");
        }
        // 空日志:无偏移可言
        let mut app = mouse_ls_app();
        app.size = Rect::new(0, 0, 80, 30);
        app.screen = Screen::Assemble;
        app.assembling = true;
        let log = hit_target(&app, |t| matches!(t, Target::ScrollLog(_)));
        wheel(&mut app, log, MouseEventKind::ScrollUp);
        assert_eq!(app.as_log_offset, 0, "空日志滚动不产偏移");
    }

    #[test]
    fn 新一轮装配_日志偏移归零() {
        let mut app = mouse_ls_app();
        app.as_log = (0..5).map(|i| format!("行{i:02}")).collect();
        app.as_log_offset = 4;
        app.spawn_engine(
            vec!["up".into(), "--lock".into(), "9f8e".into()],
            &mpsc::unbounded_channel().0,
        );
        assert_eq!(app.as_log_offset, 0, "新装配从最新一行跟起");
    }

    // ---- #28 契约守卫:不认识的 schema 不入状态树、日志给结论、面板照常 ----

    #[tokio::test]
    async fn 未知_schema_ls_不入状态树_日志给需升级_面板其余照常() {
        const UNKNOWN_LS: &str = r#"{"schema":"nw/ls.v2","nw":"9.9.9","ok":true,"errors":[],"warnings":[],"data":{"domain":"all","recipes":[{"name":"x"}]}}"#;
        let runner = Arc::new(FakeRunner::new(&[("ls", 0, UNKNOWN_LS)]));
        let mut app = App::with_runner(runner.clone());
        let args = vec!["ls".to_string()];
        let out = runner.run(&args).await;
        app.update(Message::Engine(EngineEvent::Finished(Box::new(out))));
        assert!(app.ls.is_none(), "不认识的 schema 不按旧形状入状态树");
        let log0 = app.log[0].clone();
        assert!(
            log0.contains("需升级 nw") && log0.contains("nw/ls.v2") && log0.contains("9.9.9"),
            "概览最近日志一句含需升级 + schema 原文 + 引擎版本:{log0}"
        );
        assert!(
            !log0.contains("数据仓根") && !log0.contains("能力"),
            "schema 指路不冒充另两种指路"
        );
        // 其余部分仍可操作:切屏、开帮助都不受阻
        app.update(Message::Key(key('j')));
        assert_eq!(app.screen, Screen::Assemble, "契约守卫不冻结面板");
        app.update(Message::Key(key('?')));
        assert!(app.help, "帮助浮层照常");
    }
}
