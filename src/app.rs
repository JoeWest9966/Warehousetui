//! 应用外壳:Elm 式集中状态 + update + 屏幕定义。
//!
//! V1 只有一棵状态树;所有交互先变成 [`Message`],由 [`App::update`]
//! 收口,屏幕渲染函数无副作用。引擎调用只能经 [`App::spawn_engine`]。

use std::sync::Arc;
use tokio::sync::mpsc;

use crate::engine::{
    capability::{self, Capability, Probe},
    commands, CommandRunner, EngineEvent, EngineOutput, EngineStatus, Gate, LsData, State,
    VpsLsData,
};
use crate::input::{self, InputAction};
use crate::layout::{hitmap, size_class, ScrollDir, Target};
use crate::screens::{catalog, RegisterSpec};
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
    /// 编目册屏(#53):vps 登记册端到端(登记两段式 + ls 投影 + rm 确认)。
    /// 第一本编目册;#54 起屏内可换册(机器 / 标签,`m` 键)——后两本
    /// (key / subscription)只换数据源与登记规格,不换屏与骨架
    /// (`screens::catalog`)。
    Vps,
    Settings,
}

impl Screen {
    pub const ALL: [Screen; 6] = [
        Screen::Overview,
        Screen::Assemble,
        Screen::Health,
        Screen::Recipes,
        Screen::Vps,
        Screen::Settings,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Screen::Overview => "概览",
            Screen::Assemble => "装配",
            Screen::Health => "健康",
            Screen::Recipes => "配方",
            Screen::Vps => "编目",
            Screen::Settings => "设置",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            Screen::Overview => "▸",
            Screen::Assemble => "⚙",
            Screen::Health => "✚",
            Screen::Recipes => "▤",
            Screen::Vps => "▩",
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

/// 健康屏三种复核模式:doctor 七面 / validate 27 项 / 外部源漂移(recheck,
/// spec #34 工单 #40)。三者并列,语义互不冒充。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HealthMode {
    Doctor,
    Validate,
    /// 外部源漂移复核:只读,结论按结果四态呈现,与 doctor 三档并列不合并。
    Recheck,
}

/// 数据仓根门控的三种不可用原因(spec #24:「没设 / 目录不可用 / 引擎缺能力」
/// 各有各的说法、互不冒充)。`Unset` / `Unavailable` 由 `App::data_root_cause`
/// 产出;`EngineCapability` 是能力探测门(#29)的接入点,本票只留位。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DataRootCause {
    /// 没设——不猜、不拿当前目录兜底。
    Unset,
    /// 设了,但目录不存在 / 不可读。
    Unavailable,
    /// 引擎缺能力(不是面板坏了)。
    EngineCapability,
}

impl DataRootCause {
    /// 指路标题:三态各说各话,互不冒充。
    pub fn title(self) -> &'static str {
        match self {
            DataRootCause::Unset => "数据仓根未设",
            DataRootCause::Unavailable => "数据仓根不可用",
            DataRootCause::EngineCapability => "引擎缺能力",
        }
    }

    /// 指路人话:每态一条,带各自的事实与下一步。
    pub fn advice(self, root: Option<&str>) -> String {
        match self {
            DataRootCause::Unset => {
                "五域读不到数据仓——不是数据丢了。去设置屏按 d 录入;面板不拿当前目录兜底。".into()
            }
            DataRootCause::Unavailable => format!(
                "目录 {} 不存在或不可读——不是数据丢了。到设置屏按 d 重设。",
                root.unwrap_or("—")
            ),
            DataRootCause::EngineCapability => {
                "引擎缺少所需能力——不是面板坏了。升级 nw 后能力探测门自然开启。".into()
            }
        }
    }
}

/// 复核门三(清单为空)的两种可区分原因(spec #34 / 工单 #40):
/// **五域还没读到** 与 **编目真为空** 是两件事,话不许互相冒充——
/// `ls` 还没回音时不能对操作者断言「没有记录」。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecheckEmpty {
    /// `ls` 还没回音:面板还没读到五域,谈不上有没有记录。
    NotLoaded,
    /// `ls` 回音了而外部源域确为空:编目里还没有登记记录(合法状态)。
    NoRecords,
}

impl RecheckEmpty {
    /// 指路标题:两态各说各话(与呈现层、日志同源)。
    pub fn title(self) -> &'static str {
        match self {
            RecheckEmpty::NotLoaded => "五域还没读到",
            RecheckEmpty::NoRecords => "没有已登记外部源",
        }
    }

    /// 指路人话:各自的下一步,互不冒充。
    pub fn advice(self) -> &'static str {
        match self {
            RecheckEmpty::NotLoaded => {
                "等 ls 回音(回概览屏按 r 重读五域)再查——现在断言有没有记录为时过早。"
            }
            RecheckEmpty::NoRecords => {
                "数据仓的编目里还没有记录;去配方屏外部源域按 r 登记(两段式:预审 → 图形确认后才落账)。"
            }
        }
    }
}

/// 外部源登记两段式(spec #25,ADR-0006)的状态机(#30 输入/预审/确认 +
/// #31 落账/结果)。任一态 Esc 返航(与既有返航同义),返航不留半条记录
/// ——面板从不写文件,账由引擎写;在途返航时互斥由 assembling 收敛。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegState {
    /// 未登记;外部源域常态。
    Idle,
    /// 填 repo 链接(通用单行输入态机)。
    RepoInput,
    /// 选 kind(skill / extension)。
    KindPick,
    /// 预审已下发(不带 `--yes`,文件不落),等回音。
    PreAudit,
    /// 预审回音已呈现(条目 + 验收 + 拟落账),可审阅、可落账、可返航。
    Review,
    /// 落账已下发(图形确认后才带 `--yes`),等写盘回音;
    /// 并入既有装配态(`assembling` 顶栏灯/HUD 同源)。
    Commit,
    /// 落账回音已呈现:写盘位置、记录内容、git 变更提示。
    Done,
}

/// 登记类别二选一(引擎 `--kind` 槽位;`--category` 是 0.1.0 就有的
/// `source register` 可重复旗标,不是能力缺口——要不要给类别输入是产品
/// 范围问题(ADR-0006),不是引擎有没有的问题;类别输入一律不给,spec #25)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegKind {
    Skill,
    Extension,
}

impl RegKind {
    pub const ALL: [RegKind; 2] = [RegKind::Skill, RegKind::Extension];

    /// 引擎 `--kind` 字面。
    pub fn word(self) -> &'static str {
        match self {
            RegKind::Skill => "skill",
            RegKind::Extension => "extension",
        }
    }

    /// 选项行文案。
    pub fn label(self) -> &'static str {
        self.word()
    }

    pub fn index(self) -> usize {
        match self {
            RegKind::Skill => 0,
            RegKind::Extension => 1,
        }
    }

    pub fn from_index(i: usize) -> Self {
        if i == 0 {
            RegKind::Skill
        } else {
            RegKind::Extension
        }
    }
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
    /// 数据仓根(spec #24,票 #27):与 NW_HOME 并列的一等设置,
    /// 持久化进 config;None = 未设——五域入口走指路,不兑当前目录。
    pub data_root: Option<String>,
    pub engine: EngineStatus,
    pub engine_version: String,
    /// 能力探测门(#29):问引擎自己所得的三态结论,入口可用性唯一事实源。
    pub capabilities: Gate,
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
    /// 有一程 `up` 在途(#59):中止键的受理面(只有它在场才受理 `x`),
    /// 回音到达或中止后即灭。与 `assembling` 分开:后者是全局写互斥,
    /// 登记落账 / 编目落账也点它,那些流程有自己的返航语义,不归装配中止管。
    pub as_running: bool,
    /// 在途那一程是被**中止**掉的(#59):进程可能已写了一半,迟到的回音
    /// 按它丢弃——不替引擎宣布成功,也不假装失败。下一次 up 下发时清零。
    pub as_aborted: bool,

    // ---- 健康 ----
    pub hl_mode: HealthMode,
    pub hl_sel: usize,
    pub hl_result: Option<EngineOutput>,
    pub hl_loading: bool,
    /// 外部源漂移复核的**本次**结论(#40):只由刚下发的那一次 recheck 的回音
    /// 写入,面板不缓存、不轮询、不冒充「上次结果」。换数据仓根即清空。
    pub hl_recheck: Option<EngineOutput>,

    // ---- 配方 ----
    pub rc_domain: Domain,
    pub rc_sel: usize,
    /// 外部源登记·预审段状态机(spec #25 / 工单 #30)。
    pub reg_state: RegState,
    /// repo 链接缓冲(通用单行输入态机,[`input::BUF_CAP`] 封顶)。
    pub reg_repo_buf: String,
    /// 已选 kind;默认 skill(选完 Enter 才下发)。
    pub reg_kind: RegKind,
    /// kind 选项选中行(↑/↓ 与鼠标点行同源)。
    pub reg_kind_sel: usize,
    /// 预审回音(确认态呈现;为 None 时 Review 不成立)。
    pub reg_result: Option<EngineOutput>,
    /// 编目册屏(#53):vps 两段式登记流(与命令组无关的骨架件)。
    pub vps_flow: crate::screens::RegisterFlow,
    /// vps 登记表单(名字 / 若干条地址 / 端口 / 若干条登录 / 备注)。
    pub vps_form: crate::screens::VpsForm,
    /// vps 编目册投影(`nw vps ls` 回音定型后落在这里;None = 还没读到)。
    pub vps_ls: Option<crate::engine::VpsLsData>,
    /// ls 在途(不拿上一轮投影冒充本次)。
    pub vps_ls_loading: bool,
    /// 编目册选中行(↑/↓ 与鼠标点行同源)。
    pub vps_sel: usize,
    /// 删除确认浮层(ADR-0009 §2:面板加的一道图形确认)。
    pub vps_rm_confirm: bool,
    /// 最近一次 `vps rm` 回音(证据五字段原样呈现)。
    pub vps_rm_result: Option<EngineOutput>,
    /// 编目册屏当前翻开哪一本(#54:`m` 换册)。屏只有一块,册子是屏内
    /// 维度——理由见 `screens::catalog::CatalogKind`。
    pub catalog_kind: crate::screens::CatalogKind,
    /// 标签册的登记流(#54):复用骨架件 [`crate::screens::RegisterFlow`],
    /// 但 tag add 没有 `--yes`,只走 Form → Commit → Done/Review
    /// (没有「文件不落」的预审段,见 `screens::catalog::TagSpec`)。
    pub tag_flow: crate::screens::RegisterFlow,
    /// 标签册表单(标签名 + 若干条 `kind/条目` 引用);挂标签与摘引用/删
    /// 标签共用,用途由 `mode` 分。
    pub tag_form: crate::screens::TagForm,
    /// 最近一次 `nw tag ls` 回音(#54 反查,本票主价值)。只由刚下发那一次
    /// 写入,面板不缓存、不轮询、不冒充「上次结果」。
    pub tag_ls_echo: Option<EngineOutput>,
    /// 反查在途(不拿上一轮结论冒充本次)。
    pub tag_ls_loading: bool,
    /// 最近一次反查问的是哪一条(由回音 argv 推导:`None` = 列全部)。
    /// 只为把「列全部 / 只列一个」这档差别显式画出来——操作者看得见自己
    /// 问的是什么,不靠记性。
    pub tag_lookup: Option<String>,
    /// 标签册选中行(↑/↓ 与鼠标点行同源)。
    pub tag_sel: usize,
    /// 摘引用 / 删标签确认浮层(ADR-0009 §2:上游 rm 没有 `--yes`,面板加
    /// 一道图形确认)。
    pub tag_rm_confirm: bool,
    /// 最近一次 `nw tag rm` 回音(证据五字段原样呈现)。
    pub tag_rm_result: Option<EngineOutput>,
    /// 订阅册(#56):两段式登记流(复用骨架件 `crate::screens::RegisterFlow`;
    /// 链接本体只经 stdin 进,预审与落账各喂一次)。
    pub sub_flow: crate::screens::RegisterFlow,
    /// 订阅登记表单:名字 / 应用(client)/ 到期日 + **链接本体喂入框(掩码)**。
    /// `body` 是 `SecretBody`——在途内存、Debug 不打印内容、随流程 drop。
    pub sub_form: crate::screens::SubForm,
    /// 订阅编目册投影(`nw subscription ls` 回音定型后落在这里;None = 还没读到)。
    pub sub_ls: Option<crate::engine::SubLsData>,
    /// ls 在途(不拿上一轮投影冒充本次)。
    pub sub_ls_loading: bool,
    /// 订阅册选中行(↑/↓ 与鼠标点行同源)。
    pub sub_sel: usize,
    /// 删除确认浮层(ADR-0009 §2:上游 rm 没有 `--yes`,面板加一道图形确认)。
    pub sub_rm_confirm: bool,
    /// 最近一次 `subscription rm` 回音(证据五字段原样呈现)。
    pub sub_rm_result: Option<EngineOutput>,
    /// 取回链接本体的建议命令是否已收进日志(#56,键 `g`):命令可见可取用,
    /// 但面板**不执行**它(ADR-0009 §1:get 一律只给建议命令)。
    pub sub_get_logged: bool,
    /// 钥匙册(#57,ADR-0009):两段式登记流(复用骨架件 `crate::screens::RegisterFlow`;
    /// 本体只经 stdin 进,预审与落账各喂一次)。机密纪律最严的一本:投影无本体
    /// 槽也不印指纹,get 一律只给建议命令。
    pub key_flow: crate::screens::RegisterFlow,
    /// 钥匙登记表单:名字 / 主机 / 若干条作用域 / 轮换日 + **钥匙本体喂入框(掩码)**。
    /// `body` 是 `SecretBody`——在途内存、Debug 不打印内容、随流程 drop。
    pub key_form: crate::screens::KeyForm,
    /// 钥匙编目册投影(`nw key ls` 回音定型后落在这里;None = 还没读到)。
    pub key_ls: Option<crate::engine::KeyLsData>,
    /// ls 在途(不拿上一轮投影冒充本次)。
    pub key_ls_loading: bool,
    /// 钥匙册选中行(↑/↓ 与鼠标点行同源)。
    pub key_sel: usize,
    /// 删除确认浮层(ADR-0009 §2:上游 rm 没有 `--yes`,面板加一道图形确认)。
    pub key_rm_confirm: bool,
    /// 最近一次 `key rm` 回音(证据五字段原样呈现)。
    pub key_rm_result: Option<EngineOutput>,
    /// 取回钥匙本体的建议命令是否已收进日志(#57,键 `g`):命令可见可取用,
    /// 但面板**不执行**它(ADR-0009 §1:get 一律只给建议命令)。
    pub key_get_logged: bool,
    /// 配方域建议命令的只读选中态(#39,键 `c`):命令可见可取用,
    /// 但面板**不执行**它(ADR-0003:执行权留给操作者)。选中行/切域即复位。
    pub rc_bake_sel: bool,
    /// 配方屏五域搜索过滤词(#61):**纯本地筛选**——只作用于面板已拿到的
    /// `ls` 回音,不新增引擎调用、不改数据来源、不把关键词带进任何命令。
    /// 空串 = 不过滤(与过滤前的渲染逐字节相同)。
    pub rc_filter: String,
    /// 过滤词输入态(#61,键 `/`):与 NW_HOME / 数据仓根手填同一级输入态,
    /// 键入即筛。只在配方屏在场时吞键(切屏即退输入态,过滤词保留)。
    pub rc_filter_on: bool,

    /// 最近动作流水,概览屏右下「最近日志」。
    pub log: Vec<String>,
    /// 键位处理器排队的引擎命令,由主循环取出执行(键位无 tx)。
    pub queue: Vec<Vec<String>>,
    /// **喂入队列**(#56): argv + 一次性 stdin 本体。与 [`App::queue`] 分开而
    /// 不是把 `Vec<String>` 换成 enum:绝大多数命令不喂 stdin,分开让「这条命令
    /// 带本体」在队列层面就显眼(也不会让既有入队点全部改写)。本体是
    /// [`crate::screens::SecretBody`](不打印内容的外包类型),在此只做在途中转,
    /// 喂给子进程后即弃。
    pub fed_queue: Vec<(Vec<String>, crate::screens::SecretBody)>,
    /// 设置屏按 e 请求外部编辑器;主循环挂起终端再拉起(见 main)。
    pub pending_editor: bool,
    /// 启动 splash:800ms 或任意键跳过(R3 决议)。
    pub splash: bool,
    /// 帮助浮层的正文滚动偏移(#53):键位与鼠标章补齐后正文超过最小常畅态
    /// (80×30 内层 24 行)放得下的量,超量时 ↑/↓ 在浮层内滚动,末行给
    /// 「还有内容」提示——不静默截断(截断等于把键位藏起来)。开关浮层即归零。
    pub help_offset: usize,
    /// NW_HOME 手填输入模式(设置屏 i 进入;浮层优先级见 on_key)。
    pub home_input: bool,
    /// 手填缓冲区(只收可打印字符;上限见 [`input::BUF_CAP`])。
    pub home_buf: String,
    /// 数据仓根手填输入模式(设置屏 d 进入,形态与 NW_HOME 手填同构)。
    pub data_input: bool,
    /// 数据仓根手填缓冲区(同款上限与拒绝语义)。
    pub data_buf: String,
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
        let mut app = Self::assemble(runner, status, None);
        // 测试注入给定一个可用的数据仓根(临时目录,三平台都在),
        // 让既有行为测试聚焦各自语义;「未设 / 不可用」由 #27 专属
        // 测试显式置 None / 坏路径验证,不靠环境带过。
        app.data_root = Some(std::env::temp_dir().display().to_string());
        app
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
            data_root: crate::engine::config::load().0.data_root,
            engine,
            engine_version: "探测中".into(),
            capabilities: Gate::default(),
            runner,
            ls: None,
            ls_loading: false,
            ov_sel: 0,
            as_sel: 0,
            as_confirm: false,
            as_log: Vec::new(),
            as_log_offset: 0,
            as_result: None,
            as_running: false,
            as_aborted: false,
            hl_mode: HealthMode::Doctor,
            hl_sel: 0,
            hl_result: None,
            hl_loading: false,
            hl_recheck: None,
            rc_domain: Domain::Recipes,
            rc_sel: 0,
            reg_state: RegState::Idle,
            reg_repo_buf: String::new(),
            reg_kind: RegKind::Skill,
            reg_kind_sel: 0,
            reg_result: None,
            vps_flow: crate::screens::RegisterFlow::new(),
            vps_form: crate::screens::new_form(),
            vps_ls: None,
            vps_ls_loading: false,
            vps_sel: 0,
            vps_rm_confirm: false,
            vps_rm_result: None,
            catalog_kind: crate::screens::CatalogKind::Vps,
            tag_flow: crate::screens::RegisterFlow::new(),
            tag_form: crate::screens::new_tag_form(crate::screens::TagFormMode::Add),
            tag_ls_echo: None,
            tag_ls_loading: false,
            tag_lookup: None,
            tag_sel: 0,
            tag_rm_confirm: false,
            tag_rm_result: None,
            sub_flow: crate::screens::RegisterFlow::new(),
            sub_form: crate::screens::new_sub_form(),
            sub_ls: None,
            sub_ls_loading: false,
            sub_sel: 0,
            sub_rm_confirm: false,
            sub_rm_result: None,
            sub_get_logged: false,
            key_flow: crate::screens::RegisterFlow::new(),
            key_form: crate::screens::new_key_form(),
            key_ls: None,
            key_ls_loading: false,
            key_sel: 0,
            key_rm_confirm: false,
            key_rm_result: None,
            key_get_logged: false,
            fed_queue: Vec::new(),
            rc_bake_sel: false,
            rc_filter: String::new(),
            rc_filter_on: false,
            log: vec!["驾驶舱已就绪".into()],
            queue: Vec::new(),
            pending_editor: false,
            splash: true,
            help_offset: 0,
            home_input: false,
            home_buf: String::new(),
            data_input: false,
            data_buf: String::new(),
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

    /// 开机自举:版本探测 + 能力探测(#29)+ 首次 ls(#27:引擎缺失时
    /// MissingRunner 自然给出证据;数据仓根缺/坏时不下发 ls,指路在五域入口)。
    pub fn bootstrap(&mut self, tx: &mpsc::UnboundedSender<EngineEvent>) {
        self.spawn_engine(commands::version(), tx);
        // 能力探测:每能力一次无害调用,经同一唯一入口下发;
        // 结论在 on_engine_finished 按精确探针 argv 收账。
        for cap in Capability::ALL {
            self.spawn_engine(cap.probe_args(), tx);
        }
        if let Some(root) = self.usable_data_root() {
            self.spawn_engine(commands::ls(&root), tx);
        } else if let Some(cause) = self.data_root_cause() {
            self.push_log(format!("ls 未下发:{}", cause.title()));
        }
    }

    /// 唯一的引擎调用入口:登记进行态,再交给 runner。
    pub fn spawn_engine(&mut self, args: Vec<String>, tx: &mpsc::UnboundedSender<EngineEvent>) {
        self.note_engine_dispatch(&args);
        self.runner.spawn(args, tx.clone());
    }

    /// **喂入版**引擎调用入口(#56): argv 同一个入口、同一套进行态登记,差别
    /// 只在多一份一次性 stdin 本体(订阅链接 / 将来的密钥本体)。喂入内容是
    /// 一次性值:原样写进子进程 stdin 后立即 drop,**绝不进 `EngineOutput` /
    /// 日志 / 配置 / 崩溃上报**(ADR-0009 §3,`engine::runner` 的喂入通道)。
    ///
    /// 与 [`App::spawn_engine`] 分成两个方法(而不是一个 `Option<Vec<u8>>` 参数)
    /// 是让「这条命令带机密本体」在调用处就难看得出。
    pub fn spawn_engine_fed(
        &mut self,
        args: Vec<String>,
        body: crate::screens::SecretBody,
        tx: &mpsc::UnboundedSender<EngineEvent>,
    ) {
        self.note_engine_dispatch(&args);
        self.runner
            .spawn_with_stdin(args, body.into_bytes(), tx.clone());
    }

    /// 下发前的进行态登记(喂入与非喂入两条路共用这一处:同一条 argv 的互斥 /
    /// 在途灯不该因为「带不带 stdin」而分叉)。
    fn note_engine_dispatch(&mut self, args: &[String]) {
        match args.first().map(String::as_str) {
            Some("up") => {
                self.assembling = true;
                self.as_confirm = false;
                self.as_log.clear();
                self.as_log_offset = 0; // 新一轮装配从最新一行跟起
                self.as_result = None;
                self.as_running = true; // 中止键从此可受理(#59)
                self.as_aborted = false; // 新一程:旧程的中止标记不赖到它头上
                self.push_log(format!("装配启动:{}", args.join(" ")));
            }
            Some("ls") => {
                // 缺根不下发命令的唯一水闸:任何路径想跑 ls 都过这里。
                if self.data_root_cause().is_some() {
                    self.push_log(format!("ls 未下发:{}", self.data_root_gate_msg()));
                    return;
                }
                self.ls_loading = true;
            }
            Some("doctor") | Some("validate") => {
                self.hl_loading = true;
                self.hl_result = None;
            }
            // vps 编目册(#53):ls 只读在途;register 落账段点亮写互斥
            // (预审段不点亮——它不写任何文件)。
            Some("vps") => match args.get(1).map(String::as_str) {
                Some("ls") => {
                    self.vps_ls_loading = true;
                }
                Some("register") if args.iter().any(|a| a == "--yes") => {
                    self.assembling = true;
                }
                _ => {}
            },
            // 标签册(#54):ls 反查只读在途;add 没有 `--yes`,那一条 argv
            // 本身就是写命令,点亮写互斥;rm 同 vps rm(面板确认后才到这儿)。
            Some("tag") => match args.get(1).map(String::as_str) {
                Some("ls") => {
                    self.tag_ls_loading = true;
                }
                Some("add") => {
                    self.assembling = true;
                }
                _ => {}
            },
            // 订阅册(#56):ls 只读在途;register 落账段点亮写互斥(预审段不
            // 点亮——它不写任何文件;本体经 stdin 喂入,见 `spawn_engine_fed`)。
            Some("subscription") => match args.get(1).map(String::as_str) {
                Some("ls") => {
                    self.sub_ls_loading = true;
                }
                Some("register") if args.iter().any(|a| a == "--yes") => {
                    self.assembling = true;
                }
                _ => {}
            },
            // 钥匙册(#57):ls 只读在途;register 落账段点亮写互斥(预审段不点亮;
            // 本体经 stdin 喂入,见 `spawn_engine_fed`)。
            Some("key") => match args.get(1).map(String::as_str) {
                Some("ls") => {
                    self.key_ls_loading = true;
                }
                Some("register") if args.iter().any(|a| a == "--yes") => {
                    self.assembling = true;
                }
                _ => {}
            },
            // recheck(#40)在途:清掉上一轮结论,不拿旧结论冒充本次
            Some("source") if args.get(1).map(String::as_str) == Some("recheck") => {
                self.hl_loading = true;
                self.hl_recheck = None;
            }
            _ => {}
        }
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
            Target::NavTo(s) => {
                self.screen = s;
                self.rc_filter_on = false; // 点导航切屏同样退过滤输入态(#61)
            }
            // 点行=选中(与 ↑/↓ 同语义),只选中、不触发破坏性动作;
            // 命中区只产自当前屏,按 self.screen 路由对应选中态。
            // 登记 kind 选项行在场时优先于配方列表(弹层优先级)。
            Target::SelectRow(i) => {
                if self.reg_state == RegState::KindPick {
                    self.reg_kind_sel = i.min(RegKind::ALL.len() - 1);
                    self.reg_kind = RegKind::from_index(self.reg_kind_sel);
                    return;
                }
                match self.screen {
                    Screen::Overview => self.ov_sel = i,
                    Screen::Assemble => self.as_sel = i,
                    Screen::Health => self.hl_sel = i,
                    Screen::Recipes => self.rc_sel = i,
                    // 编目册:点行选中按当前是哪一本走(#54)
                    Screen::Vps if self.catalog_kind == catalog::CatalogKind::Tag => {
                        self.tag_sel = i.min(self.tag_len().saturating_sub(1));
                    }
                    Screen::Vps if self.catalog_kind == catalog::CatalogKind::Key => {
                        self.key_sel = i.min(self.key_len().saturating_sub(1));
                    }
                    Screen::Vps => self.vps_sel = i.min(self.vps_len().saturating_sub(1)),
                    Screen::Settings => {}
                }
            }
            // 仪式「Enter 发射」段:与键盘 Enter 同一路(入队 up、关浮层);
            // kind 选择屏的「Enter 预审」段同款(入队不带 --yes 的 register);
            // 确认屏「Enter 落账」段(#31,闸放行时才产此目标)同路入队
            // 带 --yes 的落账——与键盘 Enter 一个后果,语义不许漂移。
            Target::Activate => match self.reg_state {
                RegState::KindPick => self.dispatch_register(),
                RegState::Review if self.reg_can_commit() => self.dispatch_register_commit(),
                _ => self.launch_selected_lock(),
            },
            // 帮助「任意处关」与仪式「Esc 返航」:与键盘 Esc 同一路
            // (日志区左键无动作,翻看归上面的滚轮分支)。
            Target::DismissPopup => self.dismiss_popup(),
            // 左键点日志区不动作;翻看由上面的滚轮分支受理(#21)。
            Target::ScrollLog(_) => {}
            // 配方域建议命令取用(#39):与键盘 `c` 完全同路——只读选中态,
            // 面板不执行这条命令(ADR-0003:执行权留给操作者)。
            Target::ToggleSuggest => self.toggle_bake_suggest(),
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
                // kind 选项行的滚轮移选项(与 ↑/↓ 同步进,弹层优先级)
                if self.reg_state == RegState::KindPick {
                    let n = RegKind::ALL.len();
                    let cur = self.reg_kind_sel.min(n - 1);
                    self.reg_kind_sel = match dir {
                        ScrollDir::Down => (cur + 1).min(n - 1),
                        ScrollDir::Up => cur.saturating_sub(1),
                    };
                    self.reg_kind = RegKind::from_index(self.reg_kind_sel);
                    return;
                }
                let n = self.domain_len();
                (&mut self.rc_sel, n)
            }
            Screen::Vps => {
                // 编目册选中行按当前是哪一本走(#54:两本各有自己的投影
                // 与选中态,换册不串味)。
                if self.catalog_kind == catalog::CatalogKind::Tag {
                    let n = self.tag_len();
                    (&mut self.tag_sel, n)
                } else if self.catalog_kind == catalog::CatalogKind::Key {
                    let n = self.key_len();
                    (&mut self.key_sel, n)
                } else {
                    let n = self.vps_len();
                    (&mut self.vps_sel, n)
                }
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

    /// 中止在途装配(#59):在途 `nw up` 子进程**真止**(进程级 kill,不等它
    /// 自己跑完),面板回锁定集选择器态;stderr 日志一行不清(`as_log` /
    /// `as_log_offset` 原样)——那是操作者判断「停在哪一步」的唯一依据。
    ///
    /// 中止之后状态由谁裁决:引擎铁律「清单是唯一提交点、永远最后写」,
    /// 所以中止可能落在一个**未收敛的中间态**。面板不替引擎宣布成功,也不
    /// 假装失败,只把两条路指给操作者:① 重跑同一条装配命令即可收敛
    /// (`up` 幂等);② 收敛与否以健康屏 `doctor` 复核为准。
    ///
    /// 与发射前的「返航」是两个语义:返航时命令还没下发,什么都没发生;
    /// 中止时进程可能已经写了一半。
    pub fn abort_assembling(&mut self) {
        self.runner.abort(); // 进程级:真止在途子进程
        self.assembling = false;
        self.as_confirm = false;
        self.as_running = false;
        self.as_aborted = true; // 迟到的 up 回音按此丢弃
        self.push_log("装配已中止:在途 nw up 子进程已止(进程级,不等它跑完)");
        self.push_log("中止可能停在未收敛中间态:面板不宣布成功也不假装失败");
        self.push_log("重跑同一条装配命令即可收敛(up 幂等);收敛与否以健康屏 doctor 复核为准");
    }

    /// 关闭当前浮层,与键盘 Esc 完全同路:帮助只关灯,发射仪式
    /// 「返航」取消并留日志(鼠标/键盘共用,语义不许漂移)。
    fn dismiss_popup(&mut self) {
        if self.help {
            self.help = false;
            self.help_offset = 0; // 关浮层即归零:重开总是从首行看起
        } else if self.as_confirm {
            self.as_confirm = false;
            self.push_log("发射取消");
        } else if self.reg_state != RegState::Idle {
            // 登记返航与 Esc 完全同路(鼠标点确认屏与键盘 Esc 一个后果)
            self.cancel_register();
        } else if self.vps_rm_confirm {
            self.vps_rm_confirm = false;
            self.push_log("删除已取消:vps.json 一个字节都没动");
        } else if self.tag_rm_confirm {
            self.tag_rm_confirm = false;
            self.push_log("删除已取消:tags.json 一个字节都没动");
        } else if self.vps_flow.phase != catalog::RegPhase::Idle {
            // 编目册登记返航与 Esc 完全同路(骨架件同一语义)
            self.cancel_vps_register();
        } else if self.tag_flow.phase != catalog::RegPhase::Idle {
            self.cancel_tag_flow();
        } else if self.sub_rm_confirm {
            self.sub_rm_confirm = false;
            self.push_log("删除已取消:subscriptions.json 一个字节都没动");
        } else if self.sub_flow.phase != catalog::RegPhase::Idle {
            // 编目册登记返航与 Esc 完全同路(骨架件同一语义);在途链接本体
            // 随表单 drop(ADR-0009 §3:面板侧不留副本)。
            self.cancel_sub_register();
        } else if self.key_rm_confirm {
            self.key_rm_confirm = false;
            self.push_log("删除已取消:keys.json 一个字节都没动");
        } else if self.key_flow.phase != catalog::RegPhase::Idle {
            // 编目册登记返航与 Esc 完全同路(骨架件同一语义);在途钥匙本体
            // 随表单 drop(ADR-0009 §3:面板侧不留副本)。
            self.cancel_key_register();
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
                // 换根竞态(#30 评审 P2):回音携带下发时的 --repo,与当前
                // 数据仓根不一致 = 旧根的迟到结果,不入账(自愈刷新已带
                // 新根在途);不带 --repo 的裸回放维持旧行为照收。
                let echo_repo = out
                    .args
                    .windows(2)
                    .find(|w| w[0] == "--repo")
                    .map(|w| w[1].as_str());
                if let Some(repo) = echo_repo {
                    let current = self.usable_data_root();
                    if current.as_deref() != Some(repo) {
                        self.push_log(format!(
                            "ls 回音已丢弃:数据仓根已换(旧根 {repo} 结果不入账)"
                        ));
                        return;
                    }
                }
                match out.envelope.as_ref().and_then(|e| e.data_as::<LsData>()) {
                    Some(d) => {
                        self.ls = Some(d);
                        // 过滤词在刷新后仍然生效(#61):新回音照样过一遍筛,
                        // 选中钳回新的可见行数内,不越界、不指向另一条。
                        self.clamp_rc_sel();
                        self.push_log("ls 五域已刷新".to_string());
                    }
                    None => {
                        self.push_log(format!("ls 结果不可用:{}", out.headline()));
                    }
                }
            }
            "up" => {
                if self.as_aborted {
                    // 中止后的迟到回音(#59):这一程是被杀掉的,半途的 stdout
                    // 不是可信 envelope —— 丢弃,不拿半个结果冒充结论。
                    // (RealRunner 中止后本就不推回音,这里兜底其它实现。)
                    // 只裁决这一条回音:assembling / as_running 归中止键自己
                    // 收敛,这里再动一次可能踩到另一笔在途写。
                    self.as_aborted = false;
                    self.push_log("装配回音已丢弃(已中止:半个结果不算结论)");
                    return;
                }
                self.assembling = false;
                self.as_running = false;
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
            // 登记回音(#30 预审 / #31 落账):只认真实登记 argv(带
            // --repo/--kind 等旗标)——精确的无参探针仍归 `_` 的能力
            // 回音路;`--yes` 在不在 argv 里区分两段;流程已返航的回音
            // 按状态丢弃(落账路的丢弃同时收敛写互斥),不幽灵重开。
            "source"
                if out.args.get(1).map(String::as_str) == Some("register")
                    && Capability::from_probe(&out.args).is_none() =>
            {
                let is_commit = out.args.iter().any(|a| a == "--yes");
                if is_commit {
                    if self.reg_state == RegState::Commit {
                        self.assembling = false;
                        let ok = out.ok;
                        let headline = out.headline().to_string();
                        self.reg_result = Some(out);
                        if ok {
                            self.reg_state = RegState::Done;
                            self.push_log(
                                "登记落账完成:catalog.json 已写入数据仓根(一次 git 变更)"
                                    .to_string(),
                            );
                            // 外部源域立即刷新,无需重启(#31);缺根时
                            // refresh_ls 自然走缺根门控留证据。
                            self.refresh_ls();
                        } else {
                            // 失败回确认态:闸随失败结论关闭(不可原地重发
                            // 同一个写),证据在确认屏呈现,Esc 可返航。
                            self.reg_state = RegState::Review;
                            self.push_log(format!("登记落账失败:{headline}"));
                        }
                    } else {
                        self.assembling = false;
                        if out.ok {
                            // 在途返航收不回引擎的写(进程边界):回音报成功则
                            // 五域必须刷新,并明示「写可能已落盘」——catalog.json
                            // 的最终对错交给 PR + 家长审复核(#31 评审 P1)。
                            self.refresh_ls();
                            self.push_log(
                                "登记写盘可能已完成:五域已刷新,catalog.json 走 PR + 家长审复核"
                                    .to_string(),
                            );
                        } else {
                            self.push_log("登记落账回音已丢弃(流程已返航,写未成功)");
                        }
                    }
                } else if self.reg_state == RegState::PreAudit {
                    let headline = out.headline().to_string();
                    self.reg_result = Some(out);
                    self.reg_state = RegState::Review;
                    self.push_log(format!("登记预审完成:{headline}"));
                } else {
                    self.push_log("登记预审回音已丢弃(流程已返航)");
                }
            }
            // 编目册回音(#53):ls 只读 / register 两段式 / rm 带面板确认。
            // 只认真实 argv——精确的无参组级探针仍归下面的能力回音路。
            "vps" if Capability::from_probe(&out.args).is_none() => self.on_vps_echo(out),
            // 标签册回音(#54):ls 反查两态 / add 无确认闸 / rm 两态。
            // 只认真实 argv(精确的无参 `tag` 探针仍归能力回音路)。
            "tag" if Capability::from_probe(&out.args).is_none() => self.on_tag_echo(out),
            // 订阅册回音(#56):ls 只读 / register 两段式(本体经 stdin)/ rm。
            // 只认真实 argv(精确的无参 `subscription` 探针仍归能力回音路)。
            "subscription" if Capability::from_probe(&out.args).is_none() => self.on_sub_echo(out),
            // 钥匙册回音(#57):ls 只读 / register 两段式(本体经 stdin)/ rm。
            // 只认真实 argv(精确的无参 `key` 探针仍归能力回音路)。
            "key" if Capability::from_probe(&out.args).is_none() => self.on_key_echo(out),
            // 外部源漂移复核回音(#40):只读,只认真实 argv(带 --root 或
            // --name);精确的无参探针仍归下面的能力回音路。
            "source"
                if out.args.get(1).map(String::as_str) == Some("recheck")
                    && Capability::from_probe(&out.args).is_none() =>
            {
                self.hl_loading = false;
                let headline = out.headline();
                self.push_log(format!("外部源漂移复核完成(只读,未写任何文件):{headline}"));
                self.hl_recheck = Some(out);
            }
            // 能力探测回音(#29):只认精确探针 argv——真实的
            // `source register --repo …`(#30)不落此路,其结果留给 #30 路由。
            _ => {
                if let Some(cap) = Capability::from_probe(&out.args) {
                    let verdict = capability::interpret(&out);
                    if verdict != Probe::Available {
                        // 日志只记一句人话;指路全文归设置屏能力清单。
                        self.push_log(format!("能力探测:{} {}", cap.label(), verdict.label()));
                    }
                    self.capabilities.record(cap, verdict);
                }
            }
        }
    }

    /// 入口门(#29):引擎明确「有」才放行;未决与禁用一律不给。
    /// #30 的外部源登记入口按此禁用并指路。
    pub fn entry_enabled(&self, cap: Capability) -> bool {
        self.capabilities.enabled(cap)
    }

    fn on_key(&mut self, k: KeyEvent) {
        if self.help {
            match k.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => self.dismiss_popup(),
                // 正文滚动(#53):复用 ↑/↓(不新开键),钳在 [0, 行数-可见行数]。
                // 滚动手感与装配日志窗格同源:向上 = 看更早的内容。
                KeyCode::Up => {
                    self.help_offset = self.help_offset.saturating_sub(1);
                }
                KeyCode::Down => {
                    self.help_offset = self.help_offset.saturating_add(1);
                }
                _ => {}
            }
            return;
        }
        if self.as_confirm {
            self.on_key_confirm(k);
            return;
        }
        // 浮层优先级:帮助浮层 > 发射确认 > 手填输入(NW_HOME / 数据仓根) > 指路屏/常屏(见 spec #9)
        if self.home_input {
            self.on_key_home_input(k);
            return;
        }
        if self.data_input {
            self.on_key_data_input(k);
            return;
        }
        // 配方屏搜索过滤输入态(#61):与两份手填同一级(输入态优先于屏键位),
        // 但只在本屏在场时吞键——切屏即退输入态,过滤词保留可预期。
        if self.rc_filter_on && self.screen == Screen::Recipes {
            self.on_key_rc_filter(k);
            return;
        }
        // 编目册登记两段式(#53):与外部源登记同级的浮层优先级,
        // 骨架件同构(`screens::catalog::RegisterFlow`)。
        if self.vps_flow.phase != catalog::RegPhase::Idle {
            self.on_key_vps_flow(k);
            return;
        }
        if self.vps_rm_confirm {
            self.on_key_vps_rm_confirm(k);
            return;
        }
        // 标签册浮层(#54):骨架件同一个 `RegisterFlow`。**rm 确认是嵌套在
        // 表单流里的一道浮层**(表单 Enter 才出现),所以它比流程态更里层,
        // 优先受理——否则 Enter 会被表单再收一次,确认框永远点不着。
        if self.tag_rm_confirm {
            self.on_key_tag_rm_confirm(k);
            return;
        }
        if self.tag_flow.phase != catalog::RegPhase::Idle {
            self.on_key_tag_flow(k);
            return;
        }
        // 订阅册浮层(#56):同一个骨架件。删除确认是独立一道图形确认
        // (ADR-0009 §2),登记流里没有 rm 那一步,所以两道浮层平级即可。
        if self.sub_rm_confirm {
            self.on_key_sub_rm_confirm(k);
            return;
        }
        if self.sub_flow.phase != catalog::RegPhase::Idle {
            self.on_key_sub_flow(k);
            return;
        }
        // 钥匙册浮层(#57):同一个骨架件。删除确认是独立一道图形确认
        // (ADR-0009 §2),登记流里没有 rm 那一步,所以两道浮层平级即可。
        if self.key_rm_confirm {
            self.on_key_key_rm_confirm(k);
            return;
        }
        if self.key_flow.phase != catalog::RegPhase::Idle {
            self.on_key_key_flow(k);
            return;
        }
        // 登记两段式(#30 #31):输入 → 预审 → 确认 → 落账 → 结果,
        // 浮层优先级接在手填之后
        match self.reg_state {
            RegState::RepoInput => {
                self.on_key_reg_repo(k);
                return;
            }
            RegState::KindPick => {
                self.on_key_reg_kind(k);
                return;
            }
            RegState::PreAudit => {
                self.on_key_reg_preaudit(k);
                return;
            }
            RegState::Review => {
                self.on_key_reg_review(k);
                return;
            }
            RegState::Commit => {
                self.on_key_reg_commit(k);
                return;
            }
            RegState::Done => {
                self.on_key_reg_done(k);
                return;
            }
            RegState::Idle => {}
        }
        match k.code {
            KeyCode::Char('q') if !self.assembling => self.quit = true,
            KeyCode::Char('?') => {
                self.help = true;
                self.help_offset = 0; // 重开从首行看起
            }
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
            Screen::Vps => self.key_catalog(k),
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
            KeyCode::Enter if !self.assembling && self.reg_state == RegState::Idle && n > 0 => {
                self.as_confirm = true
            }
            // 装配中止(#59):在途才受理,键位取本屏未占用的 `x`。Esc 是
            // 发射前「返航」的既有语义(还没下发,什么都没发生),两个取消
            // 不许共用一键——按下去之前操作者就该知道会停的是什么。
            KeyCode::Char('x') if self.as_running => self.abort_assembling(),
            _ => {}
        }
    }

    fn key_health(&mut self, k: KeyEvent) {
        // m 在三模式间轮转:doctor → validate → 外部源漂移 → doctor。
        if let KeyCode::Char('m') = k.code {
            self.hl_mode = match self.hl_mode {
                HealthMode::Doctor => HealthMode::Validate,
                HealthMode::Validate => HealthMode::Recheck,
                HealthMode::Recheck => HealthMode::Doctor,
            };
            return;
        }
        match self.hl_mode {
            HealthMode::Recheck => {
                let n = self.recheck_source_len();
                match k.code {
                    // r = 全查已登记外部源;Enter = 只查选中这一条。
                    KeyCode::Char('r') => self.run_recheck(None),
                    KeyCode::Enter if n > 0 => {
                        let name = self
                            .ls
                            .as_ref()
                            .and_then(|l| l.sources.get(self.hl_sel.min(n - 1)))
                            .map(|s| s.name.clone());
                        self.run_recheck(name);
                    }
                    KeyCode::Down if n > 0 => self.hl_sel = (self.hl_sel + 1) % n,
                    KeyCode::Up if n > 0 => self.hl_sel = (self.hl_sel + n - 1) % n,
                    _ => {}
                }
            }
            _ => {
                let n = self
                    .hl_result
                    .as_ref()
                    .map(|r| r.evidence().len())
                    .unwrap_or(0);
                match k.code {
                    KeyCode::Char('r') => self.run_health(),
                    KeyCode::Down if n > 0 => self.hl_sel = (self.hl_sel + 1) % n,
                    KeyCode::Up if n > 0 => self.hl_sel = (self.hl_sel + n - 1) % n,
                    _ => {}
                }
            }
        }
    }

    /// 外部源漂移复核的可选行数(#40):已登记的外部源记录数。
    pub fn recheck_source_len(&self) -> usize {
        self.ls.as_ref().map(|l| l.sources.len()).unwrap_or(0)
    }

    /// 门三(清单为空)的原因(#40):`None` = 清单非空,门放行。
    /// 「五域还没读到」与「编目真为空」分开说,两者互不冒充。
    pub fn recheck_empty_cause(&self) -> Option<RecheckEmpty> {
        (self.recheck_source_len() == 0).then(|| {
            if self.ls.is_none() {
                RecheckEmpty::NotLoaded
            } else {
                RecheckEmpty::NoRecords
            }
        })
    }

    /// 本次 recheck 回音里可定型的 data(#40):`None` = 还没查,或回音不可
    /// 定型(含契约守卫「需升级 nw」)。三处呈现共用这一条链,不再各写一遍
    /// (`hl_recheck → envelope → data_as::<RecheckData>`)。
    pub fn recheck_data(&self) -> Option<crate::engine::RecheckData> {
        self.hl_recheck
            .as_ref()
            .and_then(|o| o.envelope.as_ref())
            .and_then(|e| e.data_as::<crate::engine::RecheckData>())
    }

    /// 下发 recheck(#40):**只读**。三道门——数据仓根已设、引擎有 `source`
    /// 能力、清单非空——各有可区分指路;任一门不过都不下发命令。
    pub fn run_recheck(&mut self, name: Option<String>) {
        if let Some(cause) = self.data_root_cause() {
            self.push_log(format!("recheck 未下发:{}", cause.title()));
            return;
        }
        if !self.capabilities.enabled(Capability::SourceRecheck) {
            self.push_log(format!(
                "recheck 未下发:{}",
                crate::engine::capability::GATE_HINT
            ));
            return;
        }
        if let Some(cause) = self.recheck_empty_cause() {
            self.push_log(format!(
                "recheck 未下发:{}:{}",
                cause.title(),
                cause.advice()
            ));
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log("recheck 未下发:数据仓根不可用");
            return;
        };
        let scope = match name.as_deref() {
            Some(n) => format!("只查 {n}"),
            None => "全查已登记外部源".to_string(),
        };
        self.hl_recheck = None; // 在途:不拿上一轮结论冒充本次
        self.queue
            .push(commands::source_recheck(name.as_deref(), &root));
        self.push_log(format!("recheck 已入队({scope},只读不写账)"));
    }

    fn key_recipes(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Char('h') | KeyCode::Left => {
                self.rc_domain = prev_domain(self.rc_domain);
                self.rc_sel = 0;
                self.rc_bake_sel = false;
                self.clear_rc_filter();
            }
            KeyCode::Char('l') | KeyCode::Right => {
                self.rc_domain = next_domain(self.rc_domain);
                self.rc_sel = 0;
                self.rc_bake_sel = false;
                self.clear_rc_filter();
            }
            // 五域搜索过滤(#61):`/` 进过滤输入态,缓冲以当前过滤词起手
            // (可改可退格),键入即筛。键位取配方屏未占用的 `/`——
            // h/l/←/→ 切域、c 取用建议命令、r 登记、↑/↓ 选行都已占用,
            // 不为搜索新开一个会撞车的键。
            KeyCode::Char('/') => {
                self.rc_filter_on = true;
            }
            // Esc 一键回到全量(#61):过滤词与选中都复位,输入态不退
            // (本来就不在)。无过滤词时 Esc 仍是什么都不做。
            KeyCode::Esc if !self.rc_filter.is_empty() => {
                self.clear_rc_filter();
                self.push_log("已清空过滤词:回到五域全量".to_string());
            }
            // 配方域建议命令取用(#39):与鼠标点键位行完全同一条路——
            // 只读选中态,面板不执行(ADR-0003)。三前提不齐时只留指路。
            KeyCode::Char('c') => self.toggle_bake_suggest(),
            // 外部源域登记入口(#30):两道门在 start_register 里把守,
            // 不可用时只留一句指路,不进流程。
            KeyCode::Char('r') if self.rc_domain == Domain::Sources => self.start_register(),
            KeyCode::Down => {
                let n = self.domain_len();
                if n > 0 {
                    self.rc_sel = (self.rc_sel + 1) % n;
                    self.rc_bake_sel = false;
                }
            }
            KeyCode::Up => {
                let n = self.domain_len();
                if n > 0 {
                    self.rc_sel = (self.rc_sel + n - 1) % n;
                    self.rc_bake_sel = false;
                }
            }
            _ => {}
        }
    }

    /// 配方域建议命令是否给得出(#39):三前提——选中行是配方、数据仓根
    /// 已设、引擎有 `recipe bake` 能力(能力结论来自 #29 的探测门,不是常量表)。
    pub fn bake_suggest_gate(&self) -> Result<(), crate::engine::commands::BakeGate> {
        let recipe = self
            .selected_recipe()
            .map(|r| (r.name.as_str(), r.version.as_deref()));
        crate::engine::commands::recipe_bake_suggest(
            recipe,
            self.usable_data_root().as_deref(),
            self.capabilities.enabled(Capability::RecipeBake),
        )
        .map(|_| ())
    }

    /// 配方域**新建骨架**建议命令是否给得出(#58):与 bake 同三条前提——
    /// 选中行是配方(骨架名跟着它起)、数据仓根已设(`--out-dir` 指它,
    /// 不拿当前目录兜底)、引擎有 `recipe new` 能力(#29 探测门的结论)。
    pub fn recipe_new_suggest_gate(&self) -> Result<(), crate::engine::commands::RecipeNewGate> {
        let name = self.selected_recipe().map(|r| r.name.as_str());
        crate::engine::commands::recipe_new_suggest(
            name,
            self.usable_data_root().as_deref(),
            self.capabilities.enabled(Capability::RecipeNew),
        )
        .map(|_| ())
    }

    /// 配方域建议命令切换只读选中态(#39):键盘 `c` 与鼠标点键位行「取用」段
    /// 走同一条路。三前提不齐时**不翻状态、不入队**,只留一句与呈现层
    /// 同源的指路(门不放行时命中区同样不产出,键盘/鼠标同源)。
    fn toggle_bake_suggest(&mut self) {
        if self.rc_domain != Domain::Recipes {
            return;
        }
        match self.bake_suggest_gate() {
            Ok(()) => {
                self.rc_bake_sel = !self.rc_bake_sel;
                self.push_log(if self.rc_bake_sel {
                    "建议命令已选中(只读):可复制,执行权在你".to_string()
                } else {
                    "建议命令已取消选中".to_string()
                });
            }
            Err(gate) => {
                self.push_log(format!("建议命令取用:{}:{}", gate.title(), gate.advice()));
            }
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
            KeyCode::Char('d') => {
                self.data_input = true;
                self.data_buf = self.data_root.clone().unwrap_or_default();
                self.push_log("手填数据仓根:Enter 确认,Esc/q 放弃");
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

    /// 手填输入模式的键位:键入分发归通用态机 [`input::on_key`],
    /// 本函数只接 NW_HOME 的后果链(拒绝留驻、取消留日志)。
    fn on_key_home_input(&mut self, k: KeyEvent) {
        match input::on_key(&mut self.home_buf, k) {
            InputAction::Submit => self.apply_home_input(),
            InputAction::Cancel => {
                self.home_input = false;
                self.home_buf.clear();
                self.push_log("取消手填 NW_HOME");
            }
            InputAction::Edited | InputAction::Ignored => {}
        }
    }

    /// 手填生效:规范化(去空白 + `~` 展开)→ 与轮换同一条后果链。
    /// 空值/纯空格拒绝并留证据,不写任何状态、不退输入模式。
    pub fn apply_home_input(&mut self) {
        let Some(trimmed) = input::normalize(&self.home_buf) else {
            self.push_log("NW_HOME 手填被拒:空值,原值保持不变");
            return;
        };
        let expanded = crate::engine::discover::expand_tilde(&trimmed)
            .display()
            .to_string();
        self.home_input = false;
        self.home_buf.clear();
        self.apply_nw_home(expanded);
    }

    /// 配方屏搜索过滤输入态(#61):键入分发归通用态机 [`input::on_key`]
    /// (与两份手填同一条路,缓冲同样 [`input::BUF_CAP`] 封顶),本函数只接
    /// 过滤这一条的后果链。**纯本地**:改的只有 `rc_filter` 一个字符串,
    /// 不入队、不下发任何引擎命令,也不把关键词写进别处。
    /// Enter 收词退输入态;Esc/q 取消并清空过滤词(两种退出都可预期)。
    fn on_key_rc_filter(&mut self, k: KeyEvent) {
        match input::on_key(&mut self.rc_filter, k) {
            InputAction::Submit => {
                self.rc_filter_on = false;
                self.rc_sel = 0;
                self.rc_bake_sel = false;
                let n = self.domain_len();
                self.push_log(format!(
                    "过滤词「{}」:{} 域 {} 条命中(本地筛选,未入队)",
                    self.rc_filter.trim(),
                    self.rc_domain.label(),
                    n
                ));
            }
            InputAction::Cancel => {
                self.rc_filter_on = false;
                self.rc_filter.clear();
                self.rc_sel = 0;
                self.rc_bake_sel = false;
                self.push_log("已清空过滤词:回到五域全量".to_string());
            }
            // 每改一个词,选中回顶:行数变了,旧下标可能指向另一条
            InputAction::Edited => {
                self.rc_sel = 0;
                self.rc_bake_sel = false;
            }
            InputAction::Ignored => {}
        }
    }

    /// 数据仓根手填键位:与 NW_HOME 手填同一条通用态机
    /// ([`input::on_key`]),本函数只接数据仓根的后果链。
    fn on_key_data_input(&mut self, k: KeyEvent) {
        match input::on_key(&mut self.data_buf, k) {
            InputAction::Submit => self.apply_data_input(),
            InputAction::Cancel => {
                self.data_input = false;
                self.data_buf.clear();
                self.push_log("取消手填数据仓根");
            }
            InputAction::Edited | InputAction::Ignored => {}
        }
    }

    /// 数据仓根手填生效:去空白 + `~` 展开;空值拒绝并留驻可改。
    /// 路径不存在/不可读**接受入状态**(由指路给可区分说法),不在输入层拦。
    pub fn apply_data_input(&mut self) {
        let Some(trimmed) = input::normalize(&self.data_buf) else {
            self.push_log("数据仓根手填被拒:空值,原值保持不变");
            return;
        };
        let expanded = crate::engine::discover::expand_tilde(&trimmed)
            .display()
            .to_string();
        self.data_input = false;
        self.data_buf.clear();
        self.apply_data_root(expanded);
    }

    /// 数据仓根生效链(手填与将来其他入口共用):写状态 → 持久化 config →
    /// 根可用则立即刷新五域(入队 ls,无需重启);根不可用则清掉陈旧五域、
    /// 不下发命令,指路由 `data_root_cause` 的说法接管。
    pub fn apply_data_root(&mut self, next: String) {
        self.data_root = Some(next.clone());
        let mut cfg = crate::engine::config::load().0;
        cfg.data_root = Some(next.clone());
        if let Err(e) = crate::engine::config::save(&cfg) {
            self.push_log(format!("数据仓根持久化失败:{e}"));
        }
        self.push_log(format!("数据仓根切换到 {next}"));
        if self.data_root_cause().is_none() {
            self.refresh_ls();
        } else {
            self.ls = None; // 陈旧五域不冒充新根的内容
            self.clamp_rc_sel();
            self.push_log(format!("ls 未下发:{}", self.data_root_gate_msg()));
        }
        // 换根即作废上一轮的漂移结论(#40 无缓存语义):旧根的结论不冒充新根的。
        if self.hl_recheck.take().is_some() {
            self.push_log("漂移结论已作废:数据仓根已换(旧根结论不入账)");
        }
    }

    // ---- 外部源登记·预审段(spec #25 / 工单 #30) ----

    /// 登记入口可用性(#27 数据仓根 + #29 能力门两道,互不冒充):
    /// 根不可用时五域入口本身已走根指路;引擎无 source 能力时入口禁用
    /// 并给能力指路。两态都不会启动登记流程。
    pub fn reg_entry_enabled(&self) -> bool {
        self.data_root_cause().is_none() && self.capabilities.enabled(Capability::SourceRegister)
    }

    /// 入口不可用时的一句指路(与入口行提示同源)。
    pub fn reg_gate_hint(&self) -> String {
        match self.data_root_cause() {
            Some(_) => self.data_root_gate_msg(),
            None => capability::GATE_HINT.to_string(),
        }
    }

    /// 配方屏外部源域发起登记(键 `r`):先过两道门,再进输入态。
    pub fn start_register(&mut self) {
        if self.assembling {
            // 写互斥(spec #25):装配进行中不接登记,反之亦然。
            self.push_log("装配进行中:登记与装配互斥,稍后再试");
            return;
        }
        if !self.reg_entry_enabled() {
            self.push_log(format!("登记入口不可用:{}", self.reg_gate_hint()));
            return;
        }
        if self.reg_state != RegState::Idle {
            return; // 流程进行中不重入
        }
        self.reg_state = RegState::RepoInput;
        self.reg_repo_buf.clear();
        self.reg_kind = RegKind::Skill;
        self.reg_kind_sel = 0;
        self.reg_result = None;
        self.push_log("登记外部源:填 repo 链接,Enter 确认,Esc/q 放弃");
    }

    /// 输入态键位:键入分发归通用态机 [`input::on_key`](与 NW_HOME /
    /// 数据仓根手填同一条),本函数只接登记的后果链。
    fn on_key_reg_repo(&mut self, k: KeyEvent) {
        match input::on_key(&mut self.reg_repo_buf, k) {
            InputAction::Submit => self.apply_reg_repo(),
            InputAction::Cancel => self.cancel_register(),
            InputAction::Edited | InputAction::Ignored => {}
        }
    }

    /// repo 链接提交:格式非法**当场拒绝并留驻可改**(不等引擎跑一趟);
    /// 合法则进 kind 选择。
    fn apply_reg_repo(&mut self) {
        let Some(trimmed) = input::normalize(&self.reg_repo_buf) else {
            self.push_log("登记被拒:repo 链接为空,留输入态可改");
            return;
        };
        if !valid_repo_link(&trimmed) {
            self.push_log(format!(
                "登记被拒:\"{trimmed}\" 不是合法 repo 链接(owner/repo、https://github.com/owner/repo 或 git@github.com:owner/repo.git),留输入态可改"
            ));
            return;
        }
        self.reg_repo_buf = trimmed;
        self.reg_state = RegState::KindPick;
        self.reg_kind = RegKind::Skill;
        self.reg_kind_sel = 0;
        self.push_log("选 kind:↑/↓ 或 s/e 选择,Enter 预审,Esc/q 返航");
    }

    /// kind 选择态键位:↑/↓ 移选中、s/e 直选、Enter 预审、Esc/q 返航
    /// (q 与 Esc 同义,与输入态机一致)。
    fn on_key_reg_kind(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.reg_kind_sel = self.reg_kind_sel.saturating_sub(1);
                self.reg_kind = RegKind::from_index(self.reg_kind_sel);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.reg_kind_sel = (self.reg_kind_sel + 1).min(RegKind::ALL.len() - 1);
                self.reg_kind = RegKind::from_index(self.reg_kind_sel);
            }
            KeyCode::Char('s') => {
                self.reg_kind_sel = 0;
                self.reg_kind = RegKind::Skill;
            }
            KeyCode::Char('e') => {
                self.reg_kind_sel = 1;
                self.reg_kind = RegKind::Extension;
            }
            KeyCode::Enter => self.dispatch_register(),
            KeyCode::Esc | KeyCode::Char('q') => self.cancel_register(),
            _ => {}
        }
    }

    /// 预审回音态:等引擎跑完;Esc 仍可返航(回音到达时按状态丢弃,
    /// 不会幽灵重开),其余键不动作。
    fn on_key_reg_preaudit(&mut self, k: KeyEvent) {
        if matches!(k.code, KeyCode::Esc) {
            self.cancel_register();
        }
    }

    /// 确认态键位(#31):审阅;Enter = 图形确认 → 落账(——`--yes` 全流程
    /// 只可能由这一次转换下发,闸不过时不下发任何写命令);
    /// Esc 返航。与鼠标两段命中同路(`layout::register_hits` 同文案)。
    fn on_key_reg_review(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Enter => self.dispatch_register_commit(),
            KeyCode::Esc => self.cancel_register(),
            _ => {}
        }
    }

    /// 落账在途键位:只收 Esc(任一态可返航的 spec 约定)——视图返航、
    /// 缓冲清空,但写盘互斥(`assembling`)留到回音收敛;其余键不动作。
    fn on_key_reg_commit(&mut self, k: KeyEvent) {
        if matches!(k.code, KeyCode::Esc) {
            self.cancel_register();
        }
    }

    /// 结果屏键位:Esc(或 q)返航清状态;其余键不动作。
    fn on_key_reg_done(&mut self, k: KeyEvent) {
        if matches!(k.code, KeyCode::Esc | KeyCode::Char('q')) {
            self.cancel_register();
        }
    }

    /// 下发预审:不带 `--yes`、不带 `--category`(类别输入不给),
    /// `--root` 指数据仓根;下发前重过两道门(入口状态可能已变)。
    pub fn dispatch_register(&mut self) {
        if !self.reg_entry_enabled() {
            self.push_log(format!("登记不下发:{}", self.reg_gate_hint()));
            self.cancel_register();
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("登记不下发:{}", self.data_root_gate_msg()));
            self.cancel_register();
            return;
        };
        let repo = self.reg_repo_buf.clone();
        let kind = self.reg_kind.word().to_string();
        self.reg_state = RegState::PreAudit;
        self.reg_result = None;
        self.queue
            .push(commands::source_register_preaudit(&repo, &kind, &root));
        self.push_log(format!(
            "登记预审已入队:source register --repo {repo} --kind {kind}(不带 --yes,文件不落)"
        ));
    }

    /// 落账(#31):图形确认(键盘 Enter / 鼠标点「Enter 落账」段)后才可能
    /// 到达这里——全流程唯一能产生 `--yes` 的转换。重过三道闸(入口能力 /
    /// 数据仓根 / 预审结论),全过才下发并点亮写互斥(`assembling`,顶栏灯
    /// 与 HUD 同源);任何一道不过都不下发写命令,留在确认态把结论说清。
    pub fn dispatch_register_commit(&mut self) {
        if self.reg_state != RegState::Review || !self.reg_can_commit() {
            self.push_log("落账未下发:预审未通过(验收有 fail 或退出码非 0)");
            return;
        }
        if !self.reg_entry_enabled() {
            self.push_log(format!("落账未下发:{}", self.reg_gate_hint()));
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("落账未下发:{}", self.data_root_gate_msg()));
            return;
        };
        let repo = self.reg_repo_buf.clone();
        let kind = self.reg_kind.word().to_string();
        self.reg_state = RegState::Commit;
        // 写互斥(spec #25 铁律):登记与装配不同时进行;回音到达时收敛
        // (含在途返航后的丢弃路,见 on_engine_finished)。
        self.assembling = true;
        self.queue
            .push(commands::source_register_commit(&repo, &kind, &root));
        self.push_log(format!(
            "登记落账已入队:source register --repo {repo} --kind {kind} --yes(--root {root})"
        ));
    }

    /// 返航:任一态 Esc 同义,状态清空不留半条记录(结果丢弃);
    /// 键盘 Esc 与鼠标点「Esc 返航」段同路同语(经 [`App::dismiss_popup`])。
    pub fn cancel_register(&mut self) {
        self.reg_state = RegState::Idle;
        self.reg_repo_buf.clear();
        self.reg_result = None;
        self.push_log("登记返航");
    }

    /// 落账闸(呈现口径):预审通过 = rc 0 且验收无 fail 且 data 可定型。
    /// #31 的 `--yes` 下发必先过此闸;预审失败态在确认屏明示不可落账。
    pub fn reg_can_commit(&self) -> bool {
        let Some(out) = self.reg_result.as_ref() else {
            return false;
        };
        // 落账闸口径与编目册登记共用一处(#53 抽出的骨架件):退出码 0 +
        // 回音可定型 + 闸无 fail。外部源登记的闸面是 `acceptance[]`,
        // 折算成同一件 `catalog::Audit` 后走同一个谓词——同一个布尔,
        // 不换说法。
        let audit = out
            .envelope
            .as_ref()
            .and_then(|e| e.data_as::<crate::engine::RegisterData>())
            .map(|d| catalog::Audit {
                entry: None,
                catalog_path: d.catalog_path.clone(),
                gates: d
                    .acceptance
                    .iter()
                    .map(|a| crate::engine::CatalogGate {
                        id: a.id.clone(),
                        status: a.status.clone(),
                        message: a.message.clone(),
                    })
                    .collect(),
            });
        catalog::preaudit_passed(out, audit.as_ref())
    }

    // ---- 编目册屏·vps(spec #50 工单 #53,ADR-0009) ----

    /// 编目册屏的机器行数(`vps ls` 投影条数;没读到 = 0)。
    pub fn vps_len(&self) -> usize {
        self.vps_ls.as_ref().map(|d| d.machines.len()).unwrap_or(0)
    }

    /// 选中机器的名字(删除确认与键位行同源)。
    pub fn selected_vps_name(&self) -> Option<String> {
        let d = self.vps_ls.as_ref()?;
        d.machines
            .get(self.vps_sel.min(d.machines.len().saturating_sub(1)))
            .map(|m| m.name.clone())
    }

    /// vps 入口门(#29 能力门 + #27 数据仓根,与外部源登记同两道、互不冒充):
    /// 引擎明确「有 vps 组」且数据仓根可用才放行;否则入口禁用并指路。
    pub fn vps_entry_enabled(&self) -> bool {
        self.data_root_cause().is_none() && self.capabilities.enabled(Capability::Vps)
    }

    /// 入口不可用时的一句指路(与册体提示同源)。
    pub fn vps_gate_hint(&self) -> String {
        match self.data_root_cause() {
            Some(_) => self.data_root_gate_msg(),
            None => capability::GATE_HINT.to_string(),
        }
    }

    /// 编目册屏键位:`m` 换册(机器 / 标签);机器册 r 登记 / ↑/↓ 选 /
    /// d 删除 / l 重读;标签册 r 挂标签 / d 摘引用删标签 / Enter 反查这一条 /
    /// l 反查全部(#54)。
    fn key_catalog(&mut self, k: KeyEvent) {
        if let KeyCode::Char('m') = k.code {
            self.cycle_catalog_kind();
            return;
        }
        if self.catalog_kind == catalog::CatalogKind::Subscription {
            match k.code {
                KeyCode::Char('r') if !self.assembling => self.start_sub_register(),
                // 取回链接本体:只给建议命令,面板不代跑 get(ADR-0009 §1)
                KeyCode::Char('g') => self.log_sub_get_suggest(),
                KeyCode::Char('d') => self.start_sub_rm(),
                KeyCode::Down => {
                    let n = self.sub_len();
                    if n > 0 {
                        self.sub_sel = (self.sub_sel + 1) % n;
                    }
                }
                KeyCode::Up => {
                    let n = self.sub_len();
                    if n > 0 {
                        self.sub_sel = (self.sub_sel + n - 1) % n;
                    }
                }
                KeyCode::Char('l') => self.run_sub_ls(),
                _ => {}
            }
            return;
        }
        if self.catalog_kind == catalog::CatalogKind::Key {
            match k.code {
                KeyCode::Char('r') if !self.assembling => self.start_key_register(),
                // 取回钥匙本体:只给建议命令,面板不代跑 get(ADR-0009 §1)
                KeyCode::Char('g') => self.log_key_get_suggest(),
                KeyCode::Char('d') => self.start_key_rm(),
                KeyCode::Down => {
                    let n = self.key_len();
                    if n > 0 {
                        self.key_sel = (self.key_sel + 1) % n;
                    }
                }
                KeyCode::Up => {
                    let n = self.key_len();
                    if n > 0 {
                        self.key_sel = (self.key_sel + n - 1) % n;
                    }
                }
                KeyCode::Char('l') => self.run_key_ls(),
                _ => {}
            }
            return;
        }
        if self.catalog_kind == catalog::CatalogKind::Tag {
            match k.code {
                KeyCode::Char('r') if !self.assembling => self.start_tag_add(),
                KeyCode::Char('d') => self.start_tag_rm(),
                // 反查这一条(带参 `tag ls <标签>`):本票主价值的第二个态
                KeyCode::Enter => {
                    let Some(name) = self.selected_tag_name() else {
                        self.push_log("反查未下发:没有选中的标签(↑/↓ 先选一行)");
                        return;
                    };
                    self.run_tag_ls(Some(&name));
                }
                KeyCode::Down => {
                    let n = self.tag_len();
                    if n > 0 {
                        self.tag_sel = (self.tag_sel + 1) % n;
                    }
                }
                KeyCode::Up => {
                    let n = self.tag_len();
                    if n > 0 {
                        self.tag_sel = (self.tag_sel + n - 1) % n;
                    }
                }
                KeyCode::Char('l') => self.run_tag_ls(None),
                _ => {}
            }
            return;
        }
        match k.code {
            KeyCode::Char('r') if !self.assembling => self.start_vps_register(),
            KeyCode::Down => {
                let n = self.vps_len();
                if n > 0 {
                    self.vps_sel = (self.vps_sel + 1) % n;
                }
            }
            KeyCode::Up => {
                let n = self.vps_len();
                if n > 0 {
                    self.vps_sel = (self.vps_sel + n - 1) % n;
                }
            }
            KeyCode::Char('d') => self.start_vps_rm(),
            KeyCode::Char('l') => self.run_vps_ls(),
            _ => {}
        }
    }

    /// 发起 vps 登记:先过两道门 + 写互斥,再进表单态。
    pub fn start_vps_register(&mut self) {
        if self.assembling {
            self.push_log("装配进行中:登记与装配互斥,稍后再试");
            return;
        }
        if !self.vps_entry_enabled() {
            self.push_log(format!("登记入口不可用:{}", self.vps_gate_hint()));
            return;
        }
        self.vps_rm_confirm = false;
        self.vps_flow.begin();
        self.vps_form = catalog::new_form();
        self.push_log("登记机器:填名字/地址/端口/登录/备注,Enter 预审,Esc/q 返航");
    }

    /// 登记流程键位(表单 / 等回音 / 确认 / 落账在途 / 结果)。
    fn on_key_vps_flow(&mut self, k: KeyEvent) {
        match self.vps_flow.phase {
            catalog::RegPhase::Form => match catalog::on_key_form(&mut self.vps_form, k) {
                catalog::FormAction::Submit => self.dispatch_vps_register(),
                catalog::FormAction::Cancel => self.cancel_vps_register(),
                catalog::FormAction::Edited => {}
            },
            catalog::RegPhase::PreAudit | catalog::RegPhase::Commit => {
                if matches!(k.code, KeyCode::Esc) {
                    self.cancel_vps_register();
                }
            }
            catalog::RegPhase::Review => match k.code {
                KeyCode::Enter => self.dispatch_vps_commit(),
                KeyCode::Esc => self.cancel_vps_register(),
                _ => {}
            },
            catalog::RegPhase::Done => {
                if matches!(k.code, KeyCode::Esc | KeyCode::Char('q')) {
                    self.cancel_vps_register();
                }
            }
            catalog::RegPhase::Idle => {}
        }
    }

    /// 下发预审(两段式第一段):不带 `--yes`,文件不落;下发前重过两道门。
    pub fn dispatch_vps_register(&mut self) {
        if !self.vps_entry_enabled() {
            self.push_log(format!("登记不下发:{}", self.vps_gate_hint()));
            self.cancel_vps_register();
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("登记不下发:{}", self.data_root_gate_msg()));
            self.cancel_vps_register();
            return;
        };
        let spec = match catalog::spec_from_form(&self.vps_form) {
            Ok(s) => s,
            Err(bad) => {
                // 面板侧就拒(空名字 / label=addr 拆不出两侧),留表单可改,
                // 不喂给引擎跑一趟。
                self.push_log(format!("登记被拒:{},留表单可改", bad.join(";")));
                return;
            }
        };
        self.vps_flow.preaudit_sent();
        self.queue.push(spec.preaudit(&root));
        self.push_log(format!(
            "登记预审已入队:vps register {}(不带 --yes,文件不落)",
            spec.name
        ));
    }

    /// 落账(两段式第二段):图形确认后才可能到达——全流程唯一能产生
    /// `--yes` 的转换。重过三道闸(入口 / 数据仓根 / 预审结论),全过才
    /// 下发并点亮写互斥;任何一道不过都不下发写命令。
    pub fn dispatch_vps_commit(&mut self) {
        if self.vps_flow.phase != catalog::RegPhase::Review || !self.vps_can_commit() {
            self.push_log("落账未下发:预审未通过(五道闸有 fail 或退出码非 0)");
            return;
        }
        if !self.vps_entry_enabled() {
            self.push_log(format!("落账未下发:{}", self.vps_gate_hint()));
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("落账未下发:{}", self.data_root_gate_msg()));
            return;
        };
        let Ok(spec) = catalog::spec_from_form(&self.vps_form) else {
            self.push_log("落账未下发:表单已不可定型(返航后从表单重来)");
            return;
        };
        self.vps_flow.commit_sent();
        self.assembling = true;
        self.queue.push(spec.commit(&root));
        self.push_log(format!(
            "登记落账已入队:vps register {} --yes(--root {})",
            spec.name, root
        ));
    }

    /// vps 落账闸(呈现口径):预审通过 = rc 0 且五道闸无 fail 且 data 可定型。
    pub fn vps_can_commit(&self) -> bool {
        let Some(out) = self.vps_flow.result.as_ref() else {
            return false;
        };
        catalog::preaudit_passed(out, catalog::VpsSpec::audit(out).as_ref())
    }

    /// 返航:任一态 Esc 同义,状态清空不留半条记录。
    pub fn cancel_vps_register(&mut self) {
        self.vps_flow.abandon();
        self.vps_form = catalog::new_form();
        self.push_log("登记返航");
    }

    /// 删除确认浮层(ADR-0009 §2:面板加一道图形确认,不改引擎语义)。
    fn start_vps_rm(&mut self) {
        if self.assembling {
            self.push_log("装配进行中:删除与装配互斥,稍后再试");
            return;
        }
        if !self.vps_entry_enabled() {
            self.push_log(format!("删除未下发:{}", self.vps_gate_hint()));
            return;
        }
        let Some(name) = self.selected_vps_name() else {
            self.push_log("删除未下发:没有选中的机器(↑/↓ 先选一行)");
            return;
        };
        self.vps_rm_confirm = true;
        self.push_log(format!("删除确认:{name}(Enter 确认,Esc/q 取消)"));
    }

    fn on_key_vps_rm_confirm(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Enter => self.dispatch_vps_rm(),
            KeyCode::Esc | KeyCode::Char('q') => self.dismiss_popup(),
            _ => {}
        }
    }

    /// 下发删除:argv 与在 shell 里敲的逐字相同(面板只决定何时递出去)。
    fn dispatch_vps_rm(&mut self) {
        let Some(name) = self.selected_vps_name() else {
            self.vps_rm_confirm = false;
            self.push_log("删除未下发:没有选中的机器");
            return;
        };
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("删除未下发:{}", self.data_root_gate_msg()));
            return;
        };
        self.vps_rm_confirm = false;
        self.vps_rm_result = None;
        self.queue.push(commands::vps_rm(&name, &root));
        self.push_log(format!("删除已入队:vps rm {name} --root {root}"));
    }

    /// 读一次编目册(只读;编目不存在 = 空数组 rc 0,合法空态)。
    pub fn run_vps_ls(&mut self) {
        if !self.vps_entry_enabled() {
            self.push_log(format!("编目册未读:{}", self.vps_gate_hint()));
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("编目册未读:{}", self.data_root_gate_msg()));
            return;
        };
        self.vps_ls_loading = true;
        self.queue.push(commands::vps_ls(&root));
        self.push_log("vps ls 已入队(只读,不写任何文件)");
    }

    /// 编目册回音路由(#53):ls 只读 / register 两段式 / rm。
    fn on_vps_echo(&mut self, out: EngineOutput) {
        match out.args.get(1).map(String::as_str) {
            Some("ls") => {
                self.vps_ls_loading = false;
                match out.envelope.as_ref().and_then(|e| e.data_as::<VpsLsData>()) {
                    Some(d) => {
                        let n = d.machines.len();
                        self.vps_ls = Some(d);
                        self.clamp_vps_sel();
                        self.push_log(format!("vps 编目册已刷新:{n} 台机器(只读)"));
                    }
                    None => self.push_log(format!("vps ls 结果不可用:{}", out.headline())),
                }
            }
            Some("register") => {
                let is_commit = out.args.iter().any(|a| a == "--yes");
                if is_commit {
                    let echo_ok = out.ok;
                    match self.vps_flow.commit_back(out) {
                        Some(true) => {
                            self.assembling = false;
                            self.push_log("登记落账完成:vps.json 已写入数据仓根(一次 git 变更)");
                            self.run_vps_ls();
                        }
                        Some(false) => {
                            self.assembling = false;
                            let h = self
                                .vps_flow
                                .result
                                .as_ref()
                                .map(|o| o.headline())
                                .unwrap_or_default();
                            self.push_log(format!("登记落账失败:{h}"));
                        }
                        None => {
                            // 在途返航收不回引擎的写:回音报成功则刷新并明示
                            // 「写可能已落盘」(与外部源登记同一收敛行为)。
                            self.assembling = false;
                            if echo_ok {
                                self.run_vps_ls();
                                self.push_log(
                                    "登记写盘可能已完成:编目册已刷新,vps.json 走 PR + 家长审复核"
                                        .to_string(),
                                );
                            } else {
                                self.push_log("登记落账回音已丢弃(流程已返航,写未成功)");
                            }
                        }
                    }
                } else if self.vps_flow.preaudit_back(out) {
                    let h = self
                        .vps_flow
                        .result
                        .as_ref()
                        .map(|o| o.headline())
                        .unwrap_or_default();
                    self.push_log(format!("登记预审完成:{h}"));
                } else {
                    self.push_log("登记预审回音已丢弃(流程已返航)");
                }
            }
            Some("rm") => {
                let typed = out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::VpsRmData>());
                let rm_ok = out.ok;
                let rm_headline = out.headline();
                self.vps_rm_result = Some(out);
                match typed {
                    Some(d) if d.removed => {
                        self.push_log(format!(
                            "已删除 {}:剩余 {} 台(一次 git 变更,走 PR + 家长审)",
                            d.name, d.remaining
                        ));
                        self.run_vps_ls();
                    }
                    Some(d) => {
                        // 删不存在的名:rc 3 + 证据五字段 + 文件一个字节不动
                        self.push_log(format!(
                            "没删成:{} 不在编目里(action {}),vps.json 一个字节都没动",
                            d.name, d.action
                        ));
                    }
                    None => {
                        let _ = rm_ok;
                        self.push_log(format!("删除结果不可用:{rm_headline}"))
                    }
                }
            }
            _ => {}
        }
    }

    /// 选中行钳回可见范围内(刷新后旧下标可能越界)。
    fn clamp_vps_sel(&mut self) {
        let n = self.vps_len();
        if n == 0 {
            self.vps_sel = 0;
        } else {
            self.vps_sel = self.vps_sel.min(n - 1);
        }
    }

    // ---- 编目册屏·标签册(spec #50 工单 #54,ADR-0009) ----

    /// 当前是哪一本(`m` 换册)。换册只改维度、不清另一本的投影——
    /// 两本各有自己的账,回来还在(不重跑、不轮询)。
    pub fn cycle_catalog_kind(&mut self) {
        self.catalog_kind = self.catalog_kind.next();
        let k = self.catalog_kind;
        self.push_log(format!(
            "换册:{}册({});{}",
            k.label(),
            k.file(),
            match k {
                catalog::CatalogKind::Tag => "按 l 反查全部标签(只读)",
                catalog::CatalogKind::Key => "按 l 重读钥匙编目(只读,无本体槽、不印指纹)",
                catalog::CatalogKind::Subscription => "按 l 重读订阅编目(只读,含到期三态)",
                catalog::CatalogKind::Vps => "按 l 重读机器编目(只读)",
            }
        ));
    }

    /// 编目册屏当前这一本的投影(两本各自的数据源汇到同一个
    /// `CatalogSheet`——列表件只有一份,见 `screens::catalog`)。
    pub fn catalog_sheet(&self) -> Option<crate::engine::CatalogSheet> {
        match self.catalog_kind {
            catalog::CatalogKind::Vps => self.vps_ls.as_ref().map(|d| d.sheet()),
            catalog::CatalogKind::Tag => self
                .tag_ls_echo
                .as_ref()
                .filter(|o| o.ok)
                .and_then(|o| o.envelope.as_ref())
                .and_then(|e| e.data_as::<crate::engine::TagLsData>())
                .map(|d| d.sheet()),
            catalog::CatalogKind::Subscription => self.sub_ls.as_ref().map(|d| d.sheet()),
            catalog::CatalogKind::Key => self.key_ls.as_ref().map(|d| d.sheet()),
        }
    }

    /// 反查回音的失败态(带参问错了名字 = rc 3 / 回音不可定型)。
    /// `None` = 没有失败态。**只标签册有这一态**:机器册的 `vps ls` 没有
    /// 位置参数,失败时只留日志(51 §6.11:两种语义不都画成空列表)。
    pub fn catalog_ls_error(&self) -> Option<&EngineOutput> {
        if self.catalog_kind != catalog::CatalogKind::Tag {
            return None;
        }
        let out = self.tag_ls_echo.as_ref()?;
        let typed = out
            .envelope
            .as_ref()
            .and_then(|e| e.data_as::<crate::engine::TagLsData>());
        (typed.is_none() || !out.ok).then_some(out)
    }

    /// 册体首行的口径词:机器册给册名,标签册把「列全部 / 只列一个」
    /// 这档差别显式画出来(51 §6.11:两种语义不都画成空列表)。
    pub fn catalog_scope_label(&self) -> String {
        match self.catalog_kind {
            catalog::CatalogKind::Vps => "机器".to_string(),
            catalog::CatalogKind::Tag => match self.tag_lookup.as_deref() {
                Some(name) => format!("反查 {name}"),
                None => "全部标签".to_string(),
            },
            catalog::CatalogKind::Subscription => "全部订阅".to_string(),
            catalog::CatalogKind::Key => "全部钥匙".to_string(),
        }
    }

    /// 编目册当前这一本的选中行(↑/↓ 与鼠标点行同源)。
    pub fn catalog_sel(&self) -> usize {
        match self.catalog_kind {
            catalog::CatalogKind::Vps => self.vps_sel,
            catalog::CatalogKind::Tag => self.tag_sel,
            catalog::CatalogKind::Subscription => self.sub_sel,
            catalog::CatalogKind::Key => self.key_sel,
        }
    }

    /// 标签册的行数(反查投影条数;没查到 = 0)。
    pub fn tag_len(&self) -> usize {
        self.catalog_sheet().map(|s| s.entries.len()).unwrap_or(0)
    }

    /// 选中标签的名字(反查这一条 / 摘引用删标签同源)。
    pub fn selected_tag_name(&self) -> Option<String> {
        let sheet = self.catalog_sheet()?;
        sheet
            .entries
            .get(self.tag_sel.min(sheet.entries.len().saturating_sub(1)))
            .map(|e| e.name.clone())
    }

    /// 标签册入口门(#29 能力门 + #27 数据仓根,与 vps 册同两道、互不冒充)。
    pub fn tag_entry_enabled(&self) -> bool {
        self.data_root_cause().is_none() && self.capabilities.enabled(Capability::Tag)
    }

    /// 入口不可用时的一句指路(与册体提示同源)。
    pub fn tag_gate_hint(&self) -> String {
        match self.data_root_cause() {
            Some(_) => self.data_root_gate_msg(),
            None => capability::GATE_HINT.to_string(),
        }
    }

    /// 标签册选中行钳回可见范围内(反查刷新后旧下标可能越界)。
    fn clamp_tag_sel(&mut self) {
        let n = self.tag_len();
        if n == 0 {
            self.tag_sel = 0;
        } else {
            self.tag_sel = self.tag_sel.min(n - 1);
        }
    }

    /// 反查(#54 主价值,只读):不带参列全部标签及各引用,带参只列那一条。
    /// 编目不存在 = 空数组 rc 0(合法空态);带参查不到 = rc 3(问错了名字)。
    pub fn run_tag_ls(&mut self, name: Option<&str>) {
        if !self.tag_entry_enabled() {
            self.push_log(format!("反查未下发:{}", self.tag_gate_hint()));
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("反查未下发:{}", self.data_root_gate_msg()));
            return;
        };
        self.tag_ls_loading = true;
        self.tag_ls_echo = None; // 在途:不拿上一轮结论冒充本次
        self.tag_lookup = name.map(str::to_string);
        let argv = match name {
            Some(n) => commands::tag_ls_one(n, &root),
            None => commands::tag_ls(&root),
        };
        let scope = match name {
            Some(n) => format!("只列 {n}"),
            None => "列全部标签".to_string(),
        };
        self.queue.push(argv);
        self.push_log(format!("tag ls 已入队({scope},只读不写任何文件)"));
    }

    /// 发起挂标签:进表单态(Enter 即下发——上游没有 `--yes`,面板不自己
    /// 造一道确认,ADR-0009 §2)。
    pub fn start_tag_add(&mut self) {
        if self.assembling {
            self.push_log("装配进行中:挂标签与装配互斥,稍后再试");
            return;
        }
        if !self.tag_entry_enabled() {
            self.push_log(format!("挂标签入口不可用:{}", self.tag_gate_hint()));
            return;
        }
        self.tag_rm_confirm = false;
        self.tag_flow.begin();
        self.tag_form = catalog::new_tag_form(catalog::TagFormMode::Add);
        self.push_log("挂标签:填标签名与若干条 kind/条目 引用,Enter 挂上去,Esc/q 返航");
    }

    /// 发起摘引用 / 删标签:表单预填选中的标签名(引用留空 = 删整个标签)。
    pub fn start_tag_rm(&mut self) {
        if self.assembling {
            self.push_log("装配进行中:删除与装配互斥,稍后再试");
            return;
        }
        if !self.tag_entry_enabled() {
            self.push_log(format!("删除未下发:{}", self.tag_gate_hint()));
            return;
        }
        let Some(name) = self.selected_tag_name() else {
            self.push_log("删除未下发:没有选中的标签(↑/↓ 先选一行)");
            return;
        };
        self.tag_rm_confirm = false;
        self.tag_flow.begin();
        self.tag_form = catalog::new_tag_form(catalog::TagFormMode::Rm);
        self.tag_form.name = name.clone();
        // 光标落在引用字段:名字已预填,摘引用/删标签的动作全在引用行
        // (留空 = 删整个标签)。不预填该标签现有的引用——摘哪几条由操作者
        // 明确点出来,面板不替他决定(全预填 + 一路 Enter = 悄悄摘光)。
        self.tag_form.field = catalog::TagField::Refs;
        self.push_log(format!(
            "摘引用 / 删标签 {name}:引用留空 = 删整个标签,填 kind/条目 = 只摘那几条"
        ));
    }

    /// 标签册流程键位(表单 / 写在途 / 结论)。
    fn on_key_tag_flow(&mut self, k: KeyEvent) {
        match self.tag_flow.phase {
            catalog::RegPhase::Form => match catalog::on_key_tag_form(&mut self.tag_form, k) {
                catalog::FormAction::Submit => self.dispatch_tag_write(),
                catalog::FormAction::Cancel => self.cancel_tag_flow(),
                catalog::FormAction::Edited => {}
            },
            catalog::RegPhase::Commit => {
                if matches!(k.code, KeyCode::Esc) {
                    self.cancel_tag_flow();
                }
            }
            // tag add 没有引擎确认闸,面板也不自己造一道(ADR-0009 §2):
            // 写已随那一次下发完成,结论屏只有返航,没有「Enter 落账」段。
            catalog::RegPhase::Review | catalog::RegPhase::Done => {
                if matches!(k.code, KeyCode::Esc | KeyCode::Char('q')) {
                    self.cancel_tag_flow();
                }
            }
            catalog::RegPhase::PreAudit | catalog::RegPhase::Idle => {}
        }
    }

    /// 下发标签册的写命令:`tag add`(无确认闸,表单 Enter 即下发)或
    /// `tag rm`(面板加一道图形确认后才到这里)。幂等由**复用同一份 argv**
    /// 保证——同一条命令重跑,tags.json 一个字节不变。
    pub fn dispatch_tag_write(&mut self) {
        if !self.tag_entry_enabled() {
            self.push_log(format!("未下发:{}", self.tag_gate_hint()));
            self.cancel_tag_flow();
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("未下发:{}", self.data_root_gate_msg()));
            self.cancel_tag_flow();
            return;
        };
        let spec = match catalog::spec_from_tag_form(&self.tag_form) {
            Ok(s) => s,
            Err(bad) => {
                // 面板侧就拒(空标签名 / kind 不在已知集合),留表单可改,
                // 不喂给引擎跑一趟。
                self.push_log(format!("被拒:{},留表单可改", bad.join(";")));
                return;
            }
        };
        match self.tag_form.mode {
            catalog::TagFormMode::Add => {
                self.tag_flow.commit_sent();
                self.assembling = true;
                self.queue.push(spec.commit(&root));
                self.push_log(format!(
                    "挂标签已下发:tag add {}(追加合并、幂等,无确认闸;--root {})",
                    spec.name, root
                ));
            }
            catalog::TagFormMode::Rm => {
                // 图形确认(ADR-0009 §2):面板只决定「什么时候递出去」,
                // 下发的 argv 与在 shell 里敲的逐字相同。
                self.tag_rm_confirm = true;
                self.push_log(format!("删除确认:{}(Enter 确认,Esc/q 取消)", spec.name));
            }
        }
    }

    /// 摘引用 / 删标签确认键位。
    fn on_key_tag_rm_confirm(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Enter => self.dispatch_tag_rm(),
            KeyCode::Esc | KeyCode::Char('q') => self.dismiss_popup(),
            _ => {}
        }
    }

    /// 下发 `tag rm`:引用行为空 = 删整个标签,带引用 = 只摘那几条。
    fn dispatch_tag_rm(&mut self) {
        let Ok(spec) = catalog::spec_from_tag_form(&self.tag_form) else {
            self.tag_rm_confirm = false;
            self.push_log("删除未下发:表单已不可定型(返航后从表单重来)");
            return;
        };
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("删除未下发:{}", self.data_root_gate_msg()));
            return;
        };
        self.tag_rm_confirm = false;
        self.tag_rm_result = None;
        let refs = spec.refs.len();
        self.queue
            .push(commands::tag_rm(&spec.name, &spec.refs, &root));
        // 表单的使命到此结束(确认即消费):收流并复位,下一趟 `d` 开的是一张
        // 新表单——否则上一张引用行还摊在那儿,键会继续往里灌。
        self.tag_flow.abandon();
        self.tag_form = catalog::new_tag_form(catalog::TagFormMode::Add);
        let scope = if refs == 0 {
            "删整个标签".to_string()
        } else {
            format!("只摘 {refs} 条引用")
        };
        self.push_log(format!(
            "删除已入队:tag rm {}({scope};--root {})",
            spec.name, root
        ));
    }

    /// 返航:任一态 Esc 同义,状态清空不留半条记录。
    pub fn cancel_tag_flow(&mut self) {
        self.tag_flow.abandon();
        self.tag_rm_confirm = false;
        self.tag_form = catalog::new_tag_form(catalog::TagFormMode::Add);
        self.push_log("标签册返航");
    }

    /// 标签册回音路由(#54):ls 反查两态 / add 无确认闸 / rm 两态。
    fn on_tag_echo(&mut self, out: EngineOutput) {
        match out.args.get(1).map(String::as_str) {
            Some("ls") => {
                self.tag_ls_loading = false;
                let asked = out
                    .args
                    .iter()
                    .skip(2)
                    .find(|a| !a.starts_with("--"))
                    .cloned();
                match out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::TagLsData>())
                {
                    Some(d) => {
                        let n = d.tags.len();
                        if asked.is_some() {
                            self.tag_lookup = asked.clone();
                        }
                        self.tag_ls_echo = Some(out);
                        self.clamp_tag_sel();
                        let scope = match asked.as_deref() {
                            Some(name) => format!("标签 {name}:{n} 条"),
                            None => format!("全部标签:{n} 条"),
                        };
                        self.push_log(format!("反查完成({scope},只读)"));
                    }
                    None => {
                        // 带参问错了名字(rc 3)/ 回音不可定型:留证据,册体
                        // 不画空列表(51 §6.11)
                        let h = out.headline().to_string();
                        self.tag_ls_echo = Some(out);
                        self.push_log(format!("反查没结果:{h}"));
                    }
                }
            }
            Some("add") => {
                self.assembling = false;
                let echo_ok = out.ok;
                let typed = out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::TagAddData>());
                match self.tag_flow.commit_back(out) {
                    Some(true) => {
                        let (name, cleaned) = match typed {
                            Some(ref d) => (d.name.clone(), d.cleaned_count()),
                            None => (String::new(), 0),
                        };
                        let cleaned_line = if cleaned > 0 {
                            format!(";悬空引用已清 {cleaned} 条(已明示)")
                        } else {
                            String::new()
                        };
                        self.push_log(format!(
                            "挂标签完成:{name} 已写入 tags.json(一次 git 变更){cleaned_line}"
                        ));
                        self.run_tag_ls(None);
                    }
                    Some(false) => {
                        let h = self
                            .tag_flow
                            .result
                            .as_ref()
                            .map(|o| o.headline())
                            .unwrap_or_default();
                        self.push_log(format!("没挂成:{h}(四道闸有 fail,一个字节都没落)"));
                    }
                    None => {
                        // 在途返航收不回引擎的写:回音报成功则反查刷新并明示
                        // 「写可能已落盘」(与 vps 登记同一收敛行为)
                        if echo_ok {
                            self.run_tag_ls(None);
                            self.push_log(
                                "挂标签写盘可能已完成:标签册已刷新,tags.json 走 PR + 家长审复核"
                                    .to_string(),
                            );
                        } else {
                            self.push_log("挂标签回音已丢弃(流程已返航,写未成功)");
                        }
                    }
                }
            }
            Some("rm") => {
                let typed = out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::TagRmData>());
                self.tag_rm_result = Some(out);
                match typed {
                    Some(d) if d.removed => {
                        let what = if d.action == "ref_removed" {
                            format!("已从 {} 摘掉引用(标签还在)", d.name)
                        } else {
                            format!("已删除标签 {}", d.name)
                        };
                        self.push_log(format!(
                            "{what}:剩余 {} 个标签(一次 git 变更,走 PR + 家长审)",
                            d.remaining
                        ));
                        self.run_tag_ls(None);
                    }
                    Some(d) => {
                        // 删不存在的标签:rc 3 + 证据五字段 + 文件一个字节不动
                        self.push_log(format!(
                            "没删成:{} 不在编目里(action {}),tags.json 一个字节都没动",
                            d.name, d.action
                        ));
                    }
                    None => {
                        let h = self
                            .tag_rm_result
                            .as_ref()
                            .map(|o| o.headline())
                            .unwrap_or_default();
                        self.push_log(format!("删除结果不可用:{h}"));
                    }
                }
            }
            _ => {}
        }
    }

    // ---- 编目册屏·订阅册(spec #50 工单 #56,ADR-0009) ----

    /// 订阅册的行数(ls 投影条数;没读到 = 0)。
    pub fn sub_len(&self) -> usize {
        self.sub_ls
            .as_ref()
            .map(|d| d.subscriptions.len())
            .unwrap_or(0)
    }

    /// 选中订阅的名字(删除确认 / 取回建议命令同源)。
    pub fn selected_sub_name(&self) -> Option<String> {
        let d = self.sub_ls.as_ref()?;
        d.subscriptions
            .get(self.sub_sel.min(d.subscriptions.len().saturating_sub(1)))
            .map(|s| s.name.clone())
    }

    /// 订阅册入口门(#29 能力门 + #27 数据仓根,与 vps / tag 册同两道、互不冒充)。
    pub fn sub_entry_enabled(&self) -> bool {
        self.data_root_cause().is_none() && self.capabilities.enabled(Capability::Subscription)
    }

    /// 入口不可用时的一句指路(与册体提示同源)。
    pub fn sub_gate_hint(&self) -> String {
        match self.data_root_cause() {
            Some(_) => self.data_root_gate_msg(),
            None => capability::GATE_HINT.to_string(),
        }
    }

    /// 取回链接本体的**建议命令**(ADR-0009 §1:面板不代跑 `subscription get`)。
    /// 纯函数层产 argv + 展示串(`engine::commands::subscription_get_suggest`,
    /// 与 bake / recipe new 同源同层);`App` 只在这里读它,**没有任何路径会
    /// 把它下发**——队列里永远不会出现 `subscription get`。
    pub fn sub_get_suggest(&self) -> Option<crate::engine::commands::Suggest> {
        let name = self.selected_sub_name()?;
        let root = self.usable_data_root()?;
        Some(crate::engine::commands::subscription_get_suggest(
            &name, &root,
        ))
    }

    /// 把取回链接的建议命令收进日志(键 `g`):命令文案可取用,执行权留给操作者。
    /// **只收命令不收本体**——这条命令本身不含链接,面板也从不代跑它。
    pub fn log_sub_get_suggest(&mut self) {
        match self.sub_get_suggest() {
            Some(s) => {
                self.push_log(format!("取回链接(面板不代跑,自己执行):{}", s.display));
                self.push_log("get 是上游唯一回显本体的出口:别把它贴回面板(不进日志/配置/快照)。");
                self.sub_get_logged = true;
            }
            None => self.push_log(format!("取回链接未给出:{}", self.sub_gate_hint())),
        }
    }

    /// 读一次订阅编目册(只读;编目不存在 = 空数组 rc 0,合法空态)。
    /// 投影**没有链接槽**:到期三态与剩余天数由引擎在读取时按 UTC 现算。
    pub fn run_sub_ls(&mut self) {
        if !self.sub_entry_enabled() {
            self.push_log(format!("订阅册未读:{}", self.sub_gate_hint()));
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("订阅册未读:{}", self.data_root_gate_msg()));
            return;
        };
        self.sub_ls_loading = true;
        self.queue.push(commands::subscription_ls(&root));
        self.push_log("subscription ls 已入队(只读,不写任何文件)");
    }

    /// 发起订阅登记:先过两道门 + 写互斥,再进表单态(含掩码喂入框)。
    pub fn start_sub_register(&mut self) {
        if self.assembling {
            self.push_log("装配进行中:登记与装配互斥,稍后再试");
            return;
        }
        if !self.sub_entry_enabled() {
            self.push_log(format!("登记入口不可用:{}", self.sub_gate_hint()));
            return;
        }
        self.sub_rm_confirm = false;
        self.sub_flow.begin();
        self.sub_form = crate::screens::new_sub_form();
        self.push_log("登记订阅:填名字/应用/到期日,在链接框贴一次本体,Enter 预审,Esc/q 返航");
    }

    /// 订阅登记流程键位(表单 / 等回音 / 确认 / 落账在途 / 结果)。
    fn on_key_sub_flow(&mut self, k: KeyEvent) {
        match self.sub_flow.phase {
            catalog::RegPhase::Form => match catalog::on_key_sub_form(&mut self.sub_form, k) {
                catalog::FormAction::Submit => self.dispatch_sub_register(),
                catalog::FormAction::Cancel => self.cancel_sub_register(),
                catalog::FormAction::Edited => {}
            },
            catalog::RegPhase::PreAudit | catalog::RegPhase::Commit => {
                if matches!(k.code, KeyCode::Esc) {
                    self.cancel_sub_register();
                }
            }
            catalog::RegPhase::Review => match k.code {
                KeyCode::Enter => self.dispatch_sub_commit(),
                KeyCode::Esc => self.cancel_sub_register(),
                _ => {}
            },
            catalog::RegPhase::Done => {
                if matches!(k.code, KeyCode::Esc | KeyCode::Char('q')) {
                    self.cancel_sub_register();
                }
            }
            catalog::RegPhase::Idle => {}
        }
    }

    /// 下发预审(两段式第一段):不带 `--yes`,文件不落;**链接本体经 stdin
    /// 喂入这一次**(喂入是一次性的,落账那一下要再喂一次)。
    pub fn dispatch_sub_register(&mut self) {
        if !self.sub_entry_enabled() {
            self.push_log(format!("登记不下发:{}", self.sub_gate_hint()));
            self.cancel_sub_register();
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("登记不下发:{}", self.data_root_gate_msg()));
            self.cancel_sub_register();
            return;
        };
        let spec = match catalog::spec_from_sub_form(&self.sub_form) {
            Ok(s) => s,
            Err(bad) => {
                // 面板侧就拒(空名字 / 空 client / 到期日形状不对 / 本体没喂入),
                // 留表单可改,不喂给引擎跑一趟。
                self.push_log(format!("登记被拒:{},留表单可改", bad.join(";")));
                return;
            }
        };
        self.sub_flow.preaudit_sent();
        let fed = spec.preaudit(&root);
        let n = self.sub_form.body.len();
        // 本体在面板侧的唯一副本在表单里(在途内存):预审这一下**克隆**一份喂
        // 出去,表单里那份留到落账那一下再喂一次(两段式各喂一次,喂入是一次性
        // 的)。喂出的副本随进程结束即弃;表单里那份随返航 / 落账完成即 drop。
        self.fed_queue.push((fed, self.sub_form.body.clone()));
        self.push_log(format!(
            "登记预审已入队:subscription register --name {}(不带 --yes,文件不落;已喂入 {n} 字节)",
            spec.name
        ));
    }

    /// 落账(两段式第二段):图形确认后才可能到达——全流程唯一能产生 `--yes`
    /// 的转换。重过三道闸(入口 / 数据仓根 / 预审结论),全过才下发并点亮写
    /// 互斥;任何一道不过都不下发写命令。**本体再喂一次 stdin**。
    pub fn dispatch_sub_commit(&mut self) {
        if self.sub_flow.phase != catalog::RegPhase::Review || !self.sub_can_commit() {
            self.push_log("落账未下发:预审未通过(四道闸有 fail 或退出码非 0)");
            return;
        }
        if !self.sub_entry_enabled() {
            self.push_log(format!("落账未下发:{}", self.sub_gate_hint()));
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("落账未下发:{}", self.data_root_gate_msg()));
            return;
        };
        let Ok(spec) = catalog::spec_from_sub_form(&self.sub_form) else {
            self.push_log("落账未下发:表单已不可定型(返航后从表单重来)");
            return;
        };
        self.sub_flow.commit_sent();
        self.assembling = true;
        let fed = spec.commit(&root);
        let n = self.sub_form.body.len();
        // 本体在面板侧的唯一副本在表单里(在途内存):预审这一下**克隆**一份喂
        // 出去,表单里那份留到落账那一下再喂一次(两段式各喂一次,喂入是一次性
        // 的)。喂出的副本随进程结束即弃;表单里那份随返航 / 落账完成即 drop。
        self.fed_queue.push((fed, self.sub_form.body.clone()));
        self.push_log(format!(
            "登记落账已入队:subscription register --name {} --yes(--root {};已再喂入 {n} 字节)",
            spec.name, root
        ));
    }

    /// 订阅落账闸(呈现口径):预审通过 = rc 0 且四道闸无 fail 且 data 可定型。
    pub fn sub_can_commit(&self) -> bool {
        let Some(out) = self.sub_flow.result.as_ref() else {
            return false;
        };
        catalog::preaudit_passed(out, catalog::SubscriptionSpec::audit(out).as_ref())
    }

    /// 返航:任一态 Esc 同义,状态清空不留半条记录(在途本体随表单一起 drop)。
    pub fn cancel_sub_register(&mut self) {
        self.sub_flow.abandon();
        self.sub_form = crate::screens::new_sub_form();
        self.push_log("登记返航(在途链接本体已 drop,面板侧不留副本)");
    }

    /// 删除确认浮层(ADR-0009 §2:面板加一道图形确认,不改引擎语义)。
    fn start_sub_rm(&mut self) {
        if self.assembling {
            self.push_log("装配进行中:删除与装配互斥,稍后再试");
            return;
        }
        if !self.sub_entry_enabled() {
            self.push_log(format!("删除未下发:{}", self.sub_gate_hint()));
            return;
        }
        let Some(name) = self.selected_sub_name() else {
            self.push_log("删除未下发:没有选中的订阅(↑/↓ 先选一行)");
            return;
        };
        self.sub_rm_confirm = true;
        self.push_log(format!("删除确认:{name}(Enter 确认,Esc/q 取消)"));
    }

    fn on_key_sub_rm_confirm(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Enter => self.dispatch_sub_rm(),
            KeyCode::Esc | KeyCode::Char('q') => self.dismiss_popup(),
            _ => {}
        }
    }

    /// 下发删除:argv 与在 shell 里敲的逐字相同(面板只决定何时递出去)。
    fn dispatch_sub_rm(&mut self) {
        let Some(name) = self.selected_sub_name() else {
            self.sub_rm_confirm = false;
            self.push_log("删除未下发:没有选中的订阅");
            return;
        };
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("删除未下发:{}", self.data_root_gate_msg()));
            return;
        };
        self.sub_rm_confirm = false;
        self.sub_rm_result = None;
        self.queue.push(commands::subscription_rm(&name, &root));
        self.push_log(format!("删除已入队:subscription rm {name} --root {root}"));
    }

    /// 订阅册选中行钳回可见范围内(刷新后旧下标可能越界)。
    fn clamp_sub_sel(&mut self) {
        let n = self.sub_len();
        if n == 0 {
            self.sub_sel = 0;
        } else {
            self.sub_sel = self.sub_sel.min(n - 1);
        }
    }

    /// 订阅册回音路由(#56):ls 只读 / register 两段式(本体经 stdin)/ rm。
    fn on_sub_echo(&mut self, out: EngineOutput) {
        match out.args.get(1).map(String::as_str) {
            Some("ls") => {
                self.sub_ls_loading = false;
                match out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::SubLsData>())
                {
                    Some(d) => {
                        let n = d.subscriptions.len();
                        let counts = d.status_counts();
                        self.sub_ls = Some(d);
                        self.clamp_sub_sel();
                        self.sub_get_logged = false;
                        self.push_log(format!(
                            "订阅册已刷新:{n} 条(active {} · expiring {} · expired {},只读)",
                            counts.active, counts.expiring, counts.expired
                        ));
                    }
                    None => self.push_log(format!("subscription ls 结果不可用:{}", out.headline())),
                }
            }
            Some("register") => {
                let is_commit = out.args.iter().any(|a| a == "--yes");
                if is_commit {
                    let echo_ok = out.ok;
                    match self.sub_flow.commit_back(out) {
                        Some(true) => {
                            self.assembling = false;
                            self.push_log(
                                "登记落账完成:subscriptions.json 已写入数据仓根(一次 git 变更)"
                                    .to_string(),
                            );
                            self.run_sub_ls();
                        }
                        Some(false) => {
                            self.assembling = false;
                            let h = self
                                .sub_flow
                                .result
                                .as_ref()
                                .map(|o| o.headline())
                                .unwrap_or_default();
                            self.push_log(format!("登记落账失败:{h}"));
                        }
                        None => {
                            // 在途返航收不回引擎的写(与 vps / 外部源登记同一收敛行为)
                            self.assembling = false;
                            if echo_ok {
                                self.run_sub_ls();
                                self.push_log(
                                    "登记写盘可能已完成:订阅册已刷新,subscriptions.json 走 PR + 家长审复核"
                                        .to_string(),
                                );
                            } else {
                                self.push_log("登记落账回音已丢弃(流程已返航,写未成功)");
                            }
                        }
                    }
                } else if self.sub_flow.preaudit_back(out) {
                    let h = self
                        .sub_flow
                        .result
                        .as_ref()
                        .map(|o| o.headline())
                        .unwrap_or_default();
                    self.push_log(format!("登记预审完成:{h}"));
                } else {
                    self.push_log("登记预审回音已丢弃(流程已返航)");
                }
            }
            Some("rm") => {
                let typed = out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::SubRmData>());
                self.sub_rm_result = Some(out);
                match typed {
                    Some(d) if d.removed => {
                        self.push_log(format!(
                            "已删除订阅 {}:剩余 {} 条(一次 git 变更,走 PR + 家长审)",
                            d.name, d.remaining
                        ));
                        self.run_sub_ls();
                    }
                    Some(d) => {
                        // 删不存在的名:rc 3 + 证据五字段 + 文件一个字节都不动
                        self.push_log(format!(
                            "没删成:{} 不在编目里(action {}),subscriptions.json 一个字节都没动",
                            d.name, d.action
                        ));
                    }
                    None => {
                        let h = self
                            .sub_rm_result
                            .as_ref()
                            .map(|o| o.headline())
                            .unwrap_or_default();
                        self.push_log(format!("删除结果不可用:{h}"));
                    }
                }
            }
            _ => {}
        }
    }

    // ---- 编目册屏·钥匙册(spec #50 工单 #57,ADR-0009;机密纪律最严的一本) ----

    /// 钥匙册的行数(ls 投影条数;没读到 = 0)。
    pub fn key_len(&self) -> usize {
        self.key_ls.as_ref().map(|d| d.keys.len()).unwrap_or(0)
    }

    /// 选中钥匙的名字(删除确认 / 取回建议命令同源)。
    pub fn selected_key_name(&self) -> Option<String> {
        let d = self.key_ls.as_ref()?;
        d.keys
            .get(self.key_sel.min(d.keys.len().saturating_sub(1)))
            .map(|k| k.name.clone())
    }

    /// 钥匙册入口门(#29 能力门 + #27 数据仓根,与 vps / tag / subscription 册同两道)。
    pub fn key_entry_enabled(&self) -> bool {
        self.data_root_cause().is_none() && self.capabilities.enabled(Capability::Key)
    }

    /// 入口不可用时的一句指路(与册体提示同源)。
    pub fn key_gate_hint(&self) -> String {
        match self.data_root_cause() {
            Some(_) => self.data_root_gate_msg(),
            None => capability::GATE_HINT.to_string(),
        }
    }

    /// 取回钥匙本体的**建议命令**(ADR-0009 §1:面板不代跑 `key get`)。纯函数层产
    /// argv + 展示串(`engine::commands::key_get_suggest`,与 bake / recipe new /
    /// subscription get 同源同层);`App` 只在这里读它,**没有任何路径会把它下发**
    /// ——队列里永远不会出现 `key get`。
    pub fn key_get_suggest(&self) -> Option<crate::engine::commands::Suggest> {
        let name = self.selected_key_name()?;
        let root = self.usable_data_root()?;
        Some(crate::engine::commands::key_get_suggest(&name, &root))
    }

    /// 把取回钥匙的建议命令收进日志(键 `g`):命令文案可取用,执行权留给操作者。
    /// **只收命令不收本体**——这条命令本身不含钥匙,面板也从不代跑它。
    pub fn log_key_get_suggest(&mut self) {
        match self.key_get_suggest() {
            Some(s) => {
                self.push_log(format!("取回钥匙(面板不代跑,自己执行):{}", s.display));
                self.push_log("get 是上游唯一回显本体的出口:别把它贴回面板(不进日志/配置/快照)。");
                self.key_get_logged = true;
            }
            None => self.push_log(format!("取回钥匙未给出:{}", self.key_gate_hint())),
        }
    }

    /// 读一次钥匙编目册(只读;编目不存在 = 空数组 rc 0,合法空态)。投影**没有
    /// 本体槽、也不印指纹**。
    pub fn run_key_ls(&mut self) {
        if !self.key_entry_enabled() {
            self.push_log(format!("钥匙册未读:{}", self.key_gate_hint()));
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("钥匙册未读:{}", self.data_root_gate_msg()));
            return;
        };
        self.key_ls_loading = true;
        self.queue.push(commands::key_ls(&root));
        self.push_log("key ls 已入队(只读,不写任何文件)");
    }

    /// 发起钥匙登记:先过两道门 + 写互斥,再进表单态(含掩码喂入框)。
    pub fn start_key_register(&mut self) {
        if self.assembling {
            self.push_log("装配进行中:登记与装配互斥,稍后再试");
            return;
        }
        if !self.key_entry_enabled() {
            self.push_log(format!("登记入口不可用:{}", self.key_gate_hint()));
            return;
        }
        self.key_rm_confirm = false;
        self.key_flow.begin();
        self.key_form = crate::screens::new_key_form();
        self.push_log(
            "登记钥匙:填名字/主机/作用域/轮换日,在钥匙框贴一次本体,Enter 预审,Esc/q 返航",
        );
    }

    /// 钥匙登记流程键位(表单 / 等回音 / 确认 / 落账在途 / 结果)。
    fn on_key_key_flow(&mut self, k: KeyEvent) {
        match self.key_flow.phase {
            catalog::RegPhase::Form => match catalog::on_key_key_form(&mut self.key_form, k) {
                catalog::FormAction::Submit => self.dispatch_key_register(),
                catalog::FormAction::Cancel => self.cancel_key_register(),
                catalog::FormAction::Edited => {}
            },
            catalog::RegPhase::PreAudit | catalog::RegPhase::Commit => {
                if matches!(k.code, KeyCode::Esc) {
                    self.cancel_key_register();
                }
            }
            catalog::RegPhase::Review => match k.code {
                KeyCode::Enter => self.dispatch_key_commit(),
                KeyCode::Esc => self.cancel_key_register(),
                _ => {}
            },
            catalog::RegPhase::Done => {
                if matches!(k.code, KeyCode::Esc | KeyCode::Char('q')) {
                    self.cancel_key_register();
                }
            }
            catalog::RegPhase::Idle => {}
        }
    }

    /// 下发预审(两段式第一段):不带 `--yes`,文件不落;**钥匙本体经 stdin 喂入
    /// 这一次**(喂入是一次性的,落账那一下要再喂一次)。
    pub fn dispatch_key_register(&mut self) {
        if !self.key_entry_enabled() {
            self.push_log(format!("登记不下发:{}", self.key_gate_hint()));
            self.cancel_key_register();
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("登记不下发:{}", self.data_root_gate_msg()));
            self.cancel_key_register();
            return;
        };
        let spec = match catalog::spec_from_key_form(&self.key_form) {
            Ok(s) => s,
            Err(bad) => {
                // 面板侧就拒(空名字 / 轮换日形状不对 / 本体没喂入),留表单可改,
                // 不喂给引擎跑一趟。
                self.push_log(format!("登记被拒:{},留表单可改", bad.join(";")));
                return;
            }
        };
        self.key_flow.preaudit_sent();
        let fed = spec.preaudit(&root);
        let n = self.key_form.body.len();
        // 本体在面板侧的唯一副本在表单里(在途内存):预审这一下**克隆**一份喂出去,
        // 表单里那份留到落账那一下再喂一次(两段式各喂一次,喂入是一次性的)。
        self.fed_queue.push((fed, self.key_form.body.clone()));
        self.push_log(format!(
            "登记预审已入队:key register --name {}(不带 --yes,文件不落;已喂入 {n} 字节)",
            spec.name
        ));
    }

    /// 落账(两段式第二段):图形确认后才可能到达——全流程唯一能产生 `--yes` 的
    /// 转换。重过三道闸(入口 / 数据仓根 / 预审结论),全过才下发并点亮写互斥;
    /// 任何一道不过都不下发写命令。**本体再喂一次 stdin**。
    pub fn dispatch_key_commit(&mut self) {
        if self.key_flow.phase != catalog::RegPhase::Review || !self.key_can_commit() {
            self.push_log("落账未下发:预审未通过(三道闸有 fail 或退出码非 0)");
            return;
        }
        if !self.key_entry_enabled() {
            self.push_log(format!("落账未下发:{}", self.key_gate_hint()));
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("落账未下发:{}", self.data_root_gate_msg()));
            return;
        };
        let Ok(spec) = catalog::spec_from_key_form(&self.key_form) else {
            self.push_log("落账未下发:表单已不可定型(返航后从表单重来)");
            return;
        };
        self.key_flow.commit_sent();
        self.assembling = true;
        let fed = spec.commit(&root);
        let n = self.key_form.body.len();
        self.fed_queue.push((fed, self.key_form.body.clone()));
        self.push_log(format!(
            "登记落账已入队:key register --name {} --yes(--root {};已再喂入 {n} 字节)",
            spec.name, root
        ));
    }

    /// 钥匙落账闸(呈现口径):预审通过 = rc 0 且三道闸无 fail 且 data 可定型。
    pub fn key_can_commit(&self) -> bool {
        let Some(out) = self.key_flow.result.as_ref() else {
            return false;
        };
        catalog::preaudit_passed(out, catalog::KeySpec::audit(out).as_ref())
    }

    /// 返航:任一态 Esc 同义,状态清空不留半条记录(在途本体随表单一起 drop)。
    pub fn cancel_key_register(&mut self) {
        self.key_flow.abandon();
        self.key_form = crate::screens::new_key_form();
        self.push_log("登记返航(在途钥匙本体已 drop,面板侧不留副本)");
    }

    /// 删除确认浮层(ADR-0009 §2:面板加一道图形确认,不改引擎语义)。
    fn start_key_rm(&mut self) {
        if self.assembling {
            self.push_log("装配进行中:删除与装配互斥,稍后再试");
            return;
        }
        if !self.key_entry_enabled() {
            self.push_log(format!("删除未下发:{}", self.key_gate_hint()));
            return;
        }
        let Some(name) = self.selected_key_name() else {
            self.push_log("删除未下发:没有选中的钥匙(↑/↓ 先选一行)");
            return;
        };
        self.key_rm_confirm = true;
        self.push_log(format!("删除确认:{name}(Enter 确认,Esc/q 取消)"));
    }

    fn on_key_key_rm_confirm(&mut self, k: KeyEvent) {
        match k.code {
            KeyCode::Enter => self.dispatch_key_rm(),
            KeyCode::Esc | KeyCode::Char('q') => self.dismiss_popup(),
            _ => {}
        }
    }

    /// 下发删除:argv 与在 shell 里敲的逐字相同(面板只决定何时递出去)。
    fn dispatch_key_rm(&mut self) {
        let Some(name) = self.selected_key_name() else {
            self.key_rm_confirm = false;
            self.push_log("删除未下发:没有选中的钥匙");
            return;
        };
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("删除未下发:{}", self.data_root_gate_msg()));
            return;
        };
        self.key_rm_confirm = false;
        self.key_rm_result = None;
        self.queue.push(commands::key_rm(&name, &root));
        self.push_log(format!("删除已入队:key rm {name} --root {root}"));
    }

    /// 钥匙册选中行钳回可见范围内(刷新后旧下标可能越界)。
    fn clamp_key_sel(&mut self) {
        let n = self.key_len();
        if n == 0 {
            self.key_sel = 0;
        } else {
            self.key_sel = self.key_sel.min(n - 1);
        }
    }

    /// 钥匙册回音路由(#57):ls 只读 / register 两段式(本体经 stdin)/ rm。
    fn on_key_echo(&mut self, out: EngineOutput) {
        match out.args.get(1).map(String::as_str) {
            Some("ls") => {
                self.key_ls_loading = false;
                match out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::KeyLsData>())
                {
                    Some(d) => {
                        let n = d.keys.len();
                        self.key_ls = Some(d);
                        self.clamp_key_sel();
                        self.key_get_logged = false;
                        self.push_log(format!("钥匙册已刷新:{n} 条(只读,无本体槽、不印指纹)"));
                    }
                    None => self.push_log(format!("key ls 结果不可用:{}", out.headline())),
                }
            }
            Some("register") => {
                let is_commit = out.args.iter().any(|a| a == "--yes");
                if is_commit {
                    let echo_ok = out.ok;
                    match self.key_flow.commit_back(out) {
                        Some(true) => {
                            self.assembling = false;
                            self.push_log(
                                "登记落账完成:keys.json 已写入数据仓根(一次 git 变更)".to_string(),
                            );
                            self.run_key_ls();
                        }
                        Some(false) => {
                            self.assembling = false;
                            let h = self
                                .key_flow
                                .result
                                .as_ref()
                                .map(|o| o.headline())
                                .unwrap_or_default();
                            self.push_log(format!("登记落账失败:{h}"));
                        }
                        None => {
                            // 在途返航收不回引擎的写(与 vps / subscription 同一收敛行为)
                            self.assembling = false;
                            if echo_ok {
                                self.run_key_ls();
                                self.push_log(
                                    "登记写盘可能已完成:钥匙册已刷新,keys.json 走 PR + 家长审复核"
                                        .to_string(),
                                );
                            } else {
                                self.push_log("登记落账回音已丢弃(流程已返航,写未成功)");
                            }
                        }
                    }
                } else if self.key_flow.preaudit_back(out) {
                    let h = self
                        .key_flow
                        .result
                        .as_ref()
                        .map(|o| o.headline())
                        .unwrap_or_default();
                    self.push_log(format!("登记预审完成:{h}"));
                } else {
                    self.push_log("登记预审回音已丢弃(流程已返航)");
                }
            }
            Some("rm") => {
                let typed = out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::KeyRmData>());
                self.key_rm_result = Some(out);
                match typed {
                    Some(d) if d.removed => {
                        self.push_log(format!(
                            "已删除钥匙 {}:剩余 {} 条(一次 git 变更,走 PR + 家长审)",
                            d.name, d.remaining
                        ));
                        self.run_key_ls();
                    }
                    Some(d) => {
                        // 删不存在的名:rc 3 + 证据五字段 + 文件一个字节都不动
                        self.push_log(format!(
                            "没删成:{} 不在编目里(action {}),keys.json 一个字节都没动",
                            d.name, d.action
                        ));
                    }
                    None => {
                        let h = self
                            .key_rm_result
                            .as_ref()
                            .map(|o| o.headline())
                            .unwrap_or_default();
                        self.push_log(format!("删除结果不可用:{h}"));
                    }
                }
            }
            _ => {}
        }
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

    /// 数据仓根门控:已设且目录存在可读才放行;否则给可区分的原因。
    /// 未设本身就是原因 `Unset`——「None = 没问题」在这里不成立(spec #24)。
    /// (文件系统检查是面板侧呈现门控,不替代引擎自己的判断。)
    pub fn data_root_cause(&self) -> Option<DataRootCause> {
        let Some(root) = self.data_root.as_deref() else {
            return Some(DataRootCause::Unset);
        };
        if path_usable(root) {
            None
        } else {
            Some(DataRootCause::Unavailable)
        }
    }

    /// 可用的数据仓根:缺 / 坏一律不给——调用方拿不到就不得下发 ls。
    pub fn usable_data_root(&self) -> Option<String> {
        self.data_root_cause()
            .is_none()
            .then(|| self.data_root.clone())
            .flatten()
    }

    /// 下发拒绝的人话(带各自的下一步,与五域指路同源)。
    pub fn data_root_gate_msg(&self) -> String {
        match self.data_root_cause() {
            Some(cause) => cause.advice(self.data_root.as_deref()),
            None => "数据仓根就绪".into(),
        }
    }

    /// 外部编辑器逃逸口(设置屏 e)。返回拉起命令;EDITOR/VISUAL 都缺则 None。
    pub fn editor_command() -> Option<String> {
        std::env::var("EDITOR")
            .ok()
            .or_else(|| std::env::var("VISUAL").ok())
            .filter(|s| !s.is_empty())
    }

    // ---- 需要 tx 的动作:由主循环调 ----

    /// 刷新 ls(概览 'r' 与设置屏共用)。缺根 / 坏根不入队(缺根不下发命令)。
    pub fn refresh_ls(&mut self) {
        let Some(root) = self.usable_data_root() else {
            if self.data_root_cause().is_some() {
                self.push_log(format!("ls 未入队:{}", self.data_root_gate_msg()));
            }
            return;
        };
        self.queue.push(commands::ls(&root));
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
            HealthMode::Validate => self.queue_validate(&profile),
            // 漂移复核走 run_recheck(三道门 + 按名/全查),不在这条派体检路上。
            HealthMode::Recheck => self.run_recheck(None),
        }
    }

    /// 健康屏 validate 档:27 项冻结检查按**配方文件**跑。引擎没有
    /// 「按派名 validate」这个形状(`--recipe` 收配方路径),而派名 = 配方名
    /// 是 #5 命名决策(引擎 up 的缺省同款)—— 据此在 ls 的配方域找行,
    /// 用行自带的 path 拼数据仓根下的配方文件。引擎从配方路径自己推导
    /// 仓根(`<root>/recipes/<名>/recipe.yaml` → root),不必另带 --repo。
    /// 缺前提(根没设 / 配方不在仓里 / 引擎没给 path)→ 日志说清、不下发
    /// 一条必败的命令(与 bake 建议同纪律:不给半条命令)。
    fn queue_validate(&mut self, profile: &str) {
        let Some(root) = self.usable_data_root() else {
            self.push_log("validate 要读配方文件:数据仓根未设,去设置屏按 d 录入");
            return;
        };
        let rel = self.ls.as_ref().and_then(|l| {
            l.recipes
                .iter()
                .find(|r| r.name == profile)
                .and_then(|r| r.path.clone())
        });
        match rel {
            Some(rel) => {
                let abs = format!("{}/{rel}", root.trim_end_matches('/'));
                self.queue.push(commands::validate_recipe(&abs));
            }
            None => self.push_log(format!(
                "派 {profile} 的配方不在当前数据仓(或引擎 ls 没给配方路径):validate 无从拼装,先 ls 刷新或换数据仓根"
            )),
        }
    }

    /// 发射当前选中的锁定集。
    ///
    /// 引擎 `up --lock` 只收**载体文件**(装配命令或独立 `.nwlock`),`ls` 的
    /// 锁行又不带路径 —— 但落点不另立真相:引擎 `ls` 的 locks 域读
    /// `$NW_HOME/locks/*.nwlock`,装配命令的「交锁段」写的也是
    /// `$NW_HOME/locks/<lockId>.nwlock`。面板按同一落点把 lockId 还原成
    /// 引擎认的文件。裸 lockId 下发实测 rc 3 `INPUT_INVALID:读锁定集 …`
    /// (FakeRunner 回放发现不了,真机首跑暴露,见 docs/real-first-run.md)。
    /// NW_HOME 与子进程环境同源:apply / cycle 都写进程环境供引擎继承。
    pub fn launch_selected_lock(&mut self) {
        let lock = self
            .ls
            .as_ref()
            .and_then(|l| l.locks.get(self.as_sel))
            .and_then(|l| l.lock_id.clone());
        if let Some(lock) = lock {
            self.queue
                .push(commands::up(&self.lock_carrier(&lock), None));
            self.as_confirm = false;
        } else {
            self.push_log("没有选中的锁定集");
        }
    }

    /// 选中锁定集的载体文件路径:`$NW_HOME/locks/<lockId>.nwlock`
    /// (引擎 ls 的 locks 域与装配命令交锁段的同一落点;tilde 展开同输入链)。
    fn lock_carrier(&self, lock_id: &str) -> String {
        let home = crate::engine::discover::expand_tilde(&self.nw_home);
        format!(
            "{}/locks/{lock_id}.nwlock",
            home.display().to_string().trim_end_matches('/')
        )
    }

    // ---- 通用 ----

    pub fn next_screen(&mut self) {
        let i = (self.screen.index() + 1) % Screen::ALL.len();
        self.screen = Screen::ALL[i];
        self.rc_filter_on = false; // 切屏退过滤输入态(#61),过滤词留着回屏还在
    }

    pub fn prev_screen(&mut self) {
        let i = (self.screen.index() + Screen::ALL.len() - 1) % Screen::ALL.len();
        self.screen = Screen::ALL[i];
        self.rc_filter_on = false;
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

    /// 过滤后选中行对应的配方行(#61):↑/↓、鼠标点行与 `c` 取用建议命令
    /// 都走这一口——口径是「**看得见**的那一行」,下标经过滤后的可见列表
    /// 映射回原始数组。不过滤时与过去的 `l.recipes.get(rc_sel)` 逐个等价。
    pub fn selected_recipe(&self) -> Option<&crate::engine::RecipeRow> {
        if self.rc_domain != Domain::Recipes {
            return None;
        }
        let l = self.ls.as_ref()?;
        let i = *self.rc_visible().get(self.rc_sel)?;
        l.recipes.get(i)
    }

    /// 清空过滤词并让选中回顶(#61):切域 / Esc / 取消输入共用一条后果链。
    fn clear_rc_filter(&mut self) {
        self.rc_filter.clear();
        self.rc_filter_on = false;
        self.rc_sel = 0;
        self.rc_bake_sel = false;
    }

    /// 选中钳回当前域可见行数内(#61):过滤/刷新后行数变了,`rc_sel` 不越界——
    /// ↑/↓ 的 `% n`、鼠标命中区与详情取值都建立在 `rc_sel ∈ [0, domain_len())` 上。
    pub fn clamp_rc_sel(&mut self) {
        let n = self.domain_len();
        self.rc_sel = if n == 0 { 0 } else { self.rc_sel.min(n - 1) };
    }

    /// 五域当前域**可见行**在原始数组里的下标(#61 纯本地过滤):
    /// 无关键词时就是 `0..n` 全量顺序(与过滤前逐字节同序);有关键词时按域
    /// 在该行字段上做大小写不敏感包含匹配。只筛面板已拿到的 `ls` 回音——
    /// 不新增引擎调用、不改数据来源、不把关键词带进任何命令。
    pub fn rc_visible(&self) -> Vec<usize> {
        let Some(l) = self.ls.as_ref() else {
            return Vec::new();
        };
        let q = self.rc_filter.trim().to_lowercase();
        (0..domain_total(l, self.rc_domain))
            .filter(|&i| {
                q.is_empty()
                    || domain_row_fields(self.rc_domain, l, i)
                        .iter()
                        .any(|f| f.to_lowercase().contains(&q))
            })
            .collect()
    }

    pub fn domain_len(&self) -> usize {
        self.rc_visible().len()
    }
}

/// 一域的原始条数(#61:不过滤的总数,域轨/Tab 计数与「命中/总数」的分母)。
fn domain_total(l: &LsData, d: Domain) -> usize {
    match d {
        Domain::Recipes => l.recipes.len(),
        Domain::Components => l.components.len(),
        Domain::Sources => l.sources.len(),
        Domain::Profiles => l.profiles.len(),
        Domain::Locks => l.locks.len(),
    }
}

/// 一行里参与过滤的字段(#61):按域取该 Row 的文本(列表与详情里出现的那些)。
/// 引擎没给的字段不参与匹配——不猜、不拿占位符「——」冒充可检索内容。
/// `profile`(引擎 `ls` 确有该键)不在面板的 RecipeRow 契约里,故不入匹配面。
fn domain_row_fields(d: Domain, l: &LsData, i: usize) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    match d {
        Domain::Recipes => {
            if let Some(r) = l.recipes.get(i) {
                v.push(r.name.clone());
                v.extend(r.version.clone());
                v.extend(r.path.clone());
            }
        }
        Domain::Components => {
            if let Some(c) = l.components.get(i) {
                v.push(c.name.clone());
                v.extend(c.version.clone());
                v.extend(c.path.clone());
                v.extend(c.depends.iter().map(|d| d.label()));
                v.extend(c.used_by.iter().cloned());
            }
        }
        Domain::Sources => {
            if let Some(s) = l.sources.get(i) {
                v.push(s.name.clone());
                v.extend(s.repo.clone());
                v.extend(s.kind.clone());
                v.extend(s.category.iter().cloned());
                v.extend(s.entries.iter().map(|e| e.name.clone()));
                v.extend(s.entries.iter().map(|e| e.path.clone()));
            }
        }
        Domain::Profiles => {
            if let Some(p) = l.profiles.get(i) {
                v.push(p.name.clone());
                v.extend(p.lock_id.clone());
            }
        }
        Domain::Locks => {
            if let Some(k) = l.locks.get(i) {
                v.extend(k.lock_id.clone());
                v.extend(k.recipe.as_ref().map(|r| r.label()));
            }
        }
    }
    v
}

/// repo 链接形状检查(spec #25:owner/repo · https://github.com/… ·
/// git@github.com:…,非 GitHub 拒)。面板侧快一拍拒绝,免得操作者
/// 等引擎跑一趟才发现写错(#30 故事 14);真正的守门人仍是引擎。
pub fn valid_repo_link(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.contains(char::is_whitespace) {
        return false;
    }
    if let Some(rest) = t.strip_prefix("https://github.com/") {
        return is_owner_repo(rest.trim_end_matches(".git"));
    }
    if let Some(rest) = t.strip_prefix("git@github.com:") {
        return is_owner_repo(rest.trim_end_matches(".git"));
    }
    // 其余带协议/其它主机的写法一律不在面板放行(非 GitHub 拒)
    if t.starts_with("http") || t.starts_with("git@") || t.contains("://") {
        return false;
    }
    is_owner_repo(t)
}

fn is_owner_repo(t: &str) -> bool {
    let mut parts = t.split('/');
    let (Some(owner), Some(repo), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let seg = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    seg(owner) && seg(repo)
}

fn prev_domain(d: Domain) -> Domain {
    let i = Domain::ALL.iter().position(|x| *x == d).unwrap_or(0);
    Domain::ALL[(i + Domain::ALL.len() - 1) % Domain::ALL.len()]
}

/// 目录可用性:是目录且能枚举(不存在 / 不是目录 / 不可读都算不可用)。
fn path_usable(p: &str) -> bool {
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).is_ok(),
        _ => false,
    }
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
    fn 健康validate档_按派名拼配方文件入队() {
        let mut app = fake_app();
        // 根须真存在(下发前的可用性门控);期望与实现同构拼路径,不钉平台形状
        let root = std::env::temp_dir().join("nwt-validate-ok");
        let _ = std::fs::create_dir_all(&root);
        app.data_root = Some(root.display().to_string());
        app.ls = Some(
            serde_json::from_str::<LsData>(r#"{"domain":"all","recipes":[{"name":"research-desk","version":"1.0.0","path":"recipes/research-desk/recipe.yaml"}],"components":[],"sources":[],"profiles":[{"name":"research-desk","state":"ok","lockId":"9f8e"}],"locks":[]}"#)
                .unwrap(),
        );
        app.hl_mode = HealthMode::Validate;
        app.run_health();
        assert_eq!(
            app.queue.last().unwrap(),
            &vec![
                "validate".to_string(),
                "--recipe".into(),
                // 引擎 --recipe 收配方路径(裸派名实测 rc 3 INPUT_INVALID);
                // 派名 = 配方名(#5),path 取自 ls 行,仓根由引擎从路径自推导
                format!("{}/recipes/research-desk/recipe.yaml", root.display())
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn 健康validate档_配方不在仓_不入队只留话() {
        let mut app = fake_app();
        let root = std::env::temp_dir().join("nwt-validate-miss");
        let _ = std::fs::create_dir_all(&root);
        app.data_root = Some(root.display().to_string());
        app.ls = Some(
            serde_json::from_str::<LsData>(r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[{"name":"desk","state":"ok"}],"locks":[]}"#)
                .unwrap(),
        );
        app.hl_mode = HealthMode::Validate;
        let before = app.queue.len();
        app.run_health();
        assert_eq!(app.queue.len(), before, "找不到配方:不下发一条必败的命令");
        assert!(
            app.log.iter().any(|l| l.contains("配方不在当前数据仓")),
            "日志说清为什么没跑:{:?}",
            app.log
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn 发射锁定集入队() {
        let mut app = fake_app();
        app.nw_home = "/nw-home".into(); // 钉定:落点拼装不随进程环境漂
        app.ls = Some(
            serde_json::from_str::<LsData>(r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[{"lockId":"9f8e"}]}"#)
                .unwrap(),
        );
        app.launch_selected_lock();
        assert_eq!(
            app.queue.last().unwrap(),
            &vec![
                "up".to_string(),
                "--lock".into(),
                // 引擎 up --lock 只收载体文件(裸 lockId 实测 rc 3 INPUT_INVALID);
                // 落点 = ls 的 locks 域读处、装配命令交锁段写处
                "/nw-home/locks/9f8e.nwlock".into()
            ]
        );
    }

    #[test]
    fn 发射锁定集_载体落点跟随当前安装根() {
        let mut app = fake_app();
        app.ls = Some(
            serde_json::from_str::<LsData>(r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[{"lockId":"9f8e"}]}"#)
                .unwrap(),
        );
        app.nw_home = "/first-home".into();
        app.launch_selected_lock();
        app.nw_home = "/second-home".into();
        app.launch_selected_lock();
        assert_eq!(
            app.queue.last().unwrap()[2],
            "/second-home/locks/9f8e.nwlock",
            "换 NW_HOME 后再发射,锁落点跟新家走(与子进程继承的 NW_HOME 同源)"
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

    // ---- #59 up 中途取消:进程级中止,日志不清空,收敛路由指给健康屏 ----

    /// 在途 up 的夹具(键位派发前把面板布成「装配跑着」)。
    fn up_inflight_app() -> (Arc<FakeRunner>, App) {
        let runner = Arc::new(FakeRunner::new(&[]));
        let mut app = App::with_runner(runner.clone());
        app.screen = Screen::Assemble;
        app.assembling = true;
        app.as_running = true;
        app.as_log = vec!["铺骨架…".into(), "落组件…".into()];
        app.as_log_offset = 1;
        app.ls = Some(
            serde_json::from_str::<LsData>(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[{"lockId":"9f8e"}]}"#,
            )
            .unwrap(),
        );
        (runner, app)
    }

    #[test]
    fn 中止键_在途子进程真止_面板回选择器态_日志一行不清() {
        let (runner, mut app) = up_inflight_app();
        // 在途互斥不变:再按 Enter 不开发射仪式(给忙,不排队)
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(!app.as_confirm, "在途时不接第二次发射(互斥语义不变)");
        app.update(Message::Key(key('x')));
        assert_eq!(
            runner.abort_count(),
            1,
            "中止确有一次到达运行器(进程级那一枪)"
        );
        assert!(!app.assembling, "装配态灯灭");
        assert!(!app.as_running, "在途标记收敛");
        assert!(!app.as_confirm, "回选择器态(不是确认态)");
        assert_eq!(
            app.as_log,
            vec!["铺骨架…".to_string(), "落组件…".to_string()],
            "中止不清空既有 stderr 日志:那是判断停在哪一步的唯一依据"
        );
        assert_eq!(app.as_log_offset, 1, "滚轮偏移也不动");
        assert!(app.as_result.is_none(), "不拿半个结果冒充结论");
        // 人话:不宣布成功也不假装失败,只指路
        let said = app.log.join("\n");
        assert!(said.contains("装配已中止"), "留证据:{said}");
        assert!(
            said.contains("不宣布成功也不假装失败"),
            "中止后状态不归面板裁决:{said}"
        );
        assert!(
            said.contains("重跑同一条装配命令即可收敛") && said.contains("doctor"),
            "收敛路由:重跑同一条命令 + 健康屏 doctor 复核:{said}"
        );
        // 面板真回到选择器态:能重新打开发射仪式(互斥语义不变——忙时不排队,
        // 中止后不忙了才接)
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(app.as_confirm, "回选择器态后可重新发射");
    }

    #[test]
    fn 中止后_迟到的up回音被丢弃_下一程照常入账() {
        let (_runner, mut app) = up_inflight_app();
        app.update(Message::Key(key('x')));
        // 迟到的回音(进程被杀前已产出的半个 envelope):丢弃,不入账
        let half = EngineOutput {
            args: vec!["up".into()],
            exit_code: crate::engine::ExitCode::Ok,
            ok: true,
            stderr_tail: vec![],
            envelope: None,
            raw_stdout: "{}".into(),
            parse_error: None,
        };
        app.update(Message::Engine(EngineEvent::Finished(Box::new(half))));
        assert!(app.as_result.is_none(), "半个结果不入账");
        assert!(!app.assembling, "互斥收敛");
        assert!(
            app.log.iter().any(|l| l.contains("已中止")),
            "丢弃也留证据:{:?}",
            app.log
        );
        // 新一程:经唯一入口下发,旧程的中止标记不赖到它头上
        let (tx, _rx) = mpsc::unbounded_channel();
        app.spawn_engine(vec!["up".into(), "--lock".into(), "/x.nwlock".into()], &tx);
        assert!(app.as_running && !app.as_aborted, "新一程重新在途");
        let done = EngineOutput {
            args: vec!["up".into(), "--lock".into(), "/x.nwlock".into()],
            exit_code: crate::engine::ExitCode::Ok,
            ok: true,
            stderr_tail: vec![],
            envelope: None,
            raw_stdout: "{}".into(),
            parse_error: None,
        };
        app.update(Message::Engine(EngineEvent::Finished(Box::new(done))));
        assert!(app.as_result.is_some(), "新一程的回音照常入账");
    }

    #[test]
    fn 发射前返航_与_发射后中止_两个语义不混() {
        let runner = Arc::new(FakeRunner::new(&[]));
        let mut app = App::with_runner(runner.clone());
        app.screen = Screen::Assemble;
        // 发射前返航:命令还没下发,什么都没发生 —— 不碰运行器
        app.as_confirm = true;
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(!app.as_confirm, "返航关仪式");
        assert_eq!(runner.abort_count(), 0, "返航不中止任何进程(还没下发)");
        assert!(
            app.log.iter().any(|l| l.contains("发射取消")),
            "返航留自己的话:{:?}",
            app.log
        );
        // 发射后中止:在途子进程真止
        app.assembling = true;
        app.as_running = true;
        app.update(Message::Key(key('x')));
        assert_eq!(runner.abort_count(), 1, "中止才动进程");
        assert!(
            app.log.iter().any(|l| l.contains("装配已中止")),
            "中止留自己的话:{:?}",
            app.log
        );
    }

    #[test]
    fn 中止键_只认up在途_不误杀登记落账() {
        // 登记落账也点 assembling 灯,但它的返航语义归登记流自己:x 不越界
        let runner = Arc::new(FakeRunner::new(&[]));
        let mut app = App::with_runner(runner.clone());
        app.assembling = true;
        app.reg_state = RegState::Commit;
        app.update(Message::Key(key('x')));
        assert_eq!(runner.abort_count(), 0, "没有 up 在途:一枪都不放");
        assert!(app.assembling, "登记落账的互斥不被装配中止键关掉");
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

    // ---- #27 数据仓根:一等设置 / 缺根不下发 / 三态指路 ----

    #[test]
    fn 数据仓根_手填生效同链_持久化并入队带_repo的_ls() {
        let (_g, _cfg) = env_locked();
        let mut app = settings_app();
        app.data_root = None; // 从未设开始
        app.update(Message::Key(key('d')));
        assert!(app.data_input, "d 进入数据仓根手填");
        let target = std::env::temp_dir().display().to_string();
        type_str(&mut app, &target);
        assert_eq!(app.data_buf, target, "缓冲区回显");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(
            app.data_root.as_deref(),
            Some(target.as_str()),
            "新值落状态树"
        );
        assert!(!app.data_input && app.data_buf.is_empty(), "用完即退");
        let (cfg, err) = crate::engine::config::load();
        assert!(err.is_none());
        assert_eq!(
            cfg.data_root.as_deref(),
            Some(target.as_str()),
            "持久化进 config,重启(重读)仍生效"
        );
        assert!(
            app.queue
                .iter()
                .any(|a| { a == &vec!["ls".to_string(), "--repo".into(), target.clone()] }),
            "同链入队显式携带数据仓根的 ls"
        );
        assert!(
            app.log.iter().any(|l| l.contains("数据仓根切换")),
            "日志留证据"
        );
    }

    #[test]
    fn 数据仓根_空值拒绝留驻_esc_放弃不改状态() {
        let mut app = settings_app();
        let before = app.data_root.clone();
        app.update(Message::Key(key('d')));
        app.data_buf = "   ".into();
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.data_root, before, "拒绝不改状态");
        assert!(app.data_input, "拒绝后留在输入态可改");
        assert!(app.log[0].contains("被拒"), "日志留证据");
        // Esc 放弃:状态不动、不退出
        app.data_buf = "/x/y".into();
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(!app.data_input);
        assert_eq!(app.data_root, before, "放弃不碰状态");
        assert!(!app.quit, "输入态 Esc 不是退出");
    }

    #[test]
    fn 未设数据仓根_入队与下发与自举三处都不发_ls() {
        let mut app = settings_app();
        app.data_root = None;
        let queued = app.queue.len();
        app.refresh_ls();
        assert_eq!(app.queue.len(), queued, "缺根不入队");
        // 水闸在 spawn_engine:argv 带不带 --repo 都拦
        app.spawn_engine(
            vec!["ls".into(), "--repo".into(), "/tmp".into()],
            &mpsc::unbounded_channel().0,
        );
        assert!(!app.ls_loading, "缺根不设加载态(命令未下发)");
        assert!(app.log[0].contains("未下发"), "日志留证据");
        // bootstrap:版本探测 + 能力探针(#29),但不含 ls——缺根不下发命令
        let mut app2 = settings_app();
        app2.data_root = None;
        let (tx, mut rx) = mpsc::unbounded_channel();
        app2.bootstrap(&tx);
        let mut cmds: Vec<String> = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let EngineEvent::Finished(o) = ev {
                cmds.push(o.args.first().cloned().unwrap_or_default());
            }
        }
        assert!(
            cmds.contains(&"--version".to_string()),
            "bootstrap 仍下版本探测:{cmds:?}"
        );
        assert!(
            !cmds.contains(&"ls".to_string()),
            "bootstrap 缺根不发 ls(能力探针不受数据仓根门控):{cmds:?}"
        );
        assert!(!app2.ls_loading);
        assert_eq!(app2.data_root_cause(), Some(DataRootCause::Unset));
    }

    #[test]
    fn 换数据仓根_立即再入队_ls_带新根_无需重启() {
        let (_g, _cfg) = env_locked();
        let mut app = settings_app();
        let first_root = std::env::temp_dir().display().to_string();
        app.apply_data_root(first_root.clone());
        let n1 = app.queue.len();
        assert!(n1 > 0, "首个根入队一次");
        let second_root = _cfg.dir().display().to_string(); // ConfigGuard 建的一次性目录
        app.apply_data_root(second_root.clone());
        assert_eq!(app.queue.len(), n1 + 1, "换根再入队一次");
        assert_eq!(
            app.queue.last().unwrap(),
            &vec!["ls".to_string(), "--repo".into(), second_root.clone()],
            "新根的 argv 立即生效"
        );
        assert_eq!(app.data_root.as_deref(), Some(second_root.as_str()));
    }

    #[test]
    fn 数据仓根不可用_可区分指路_不崩不下发() {
        let mut app = settings_app();
        app.data_root = Some("/nonexistent-nwt-data-root-xyz".into());
        assert_eq!(
            app.data_root_cause(),
            Some(DataRootCause::Unavailable),
            "坏根与未设可区分"
        );
        assert!(app.usable_data_root().is_none());
        let queued = app.queue.len();
        app.refresh_ls();
        assert_eq!(app.queue.len(), queued, "坏根不入队");
        app.spawn_engine(vec!["ls".into()], &mpsc::unbounded_channel().0);
        assert!(!app.ls_loading, "坏根不下发");
        let msg = app.data_root_gate_msg();
        assert!(msg.contains("/nonexistent-nwt-data-root-xyz"), "指路带路径");
        assert!(msg.contains("不存在或不可读"), "事实原文");
        assert!(msg.contains("按 d 重设"), "带下一步,不冒充未设或缺能力");
    }

    #[test]
    fn 三种指路_标题与说法两两不同_互不冒充() {
        let causes = [
            DataRootCause::Unset,
            DataRootCause::Unavailable,
            DataRootCause::EngineCapability,
        ];
        for (i, a) in causes.iter().enumerate() {
            for b in &causes[i + 1..] {
                assert_ne!(a.title(), b.title(), "标题两两不同");
                assert_ne!(a.advice(Some("/x")), b.advice(Some("/x")), "说法两两不同");
            }
        }
        assert!(
            DataRootCause::Unset.advice(None).contains("按 d 录入"),
            "未设指路说清去哪设"
        );
        assert!(
            DataRootCause::EngineCapability
                .advice(None)
                .contains("不是面板坏了"),
            "缺能力指路不冒充面板故障"
        );
    }

    #[test]
    fn 数据仓根手填浮层下_鼠标惰性_缓冲不被扰动() {
        let mut app = mouse_ls_app();
        app.size = Rect::new(0, 0, 80, 30);
        app.screen = Screen::Settings;
        let nav = hit_target(&app, |t| matches!(t, Target::NavTo(Screen::Health)));
        app.data_input = true;
        app.data_buf = "/x".into();
        click(&mut app, nav);
        assert_eq!(app.screen, Screen::Settings, "输入浮层下鼠标不分派");
        assert_eq!(app.data_buf, "/x", "缓冲区不被鼠标扰动");
    }

    // ---- #39 配方域建议命令:选中即给出 bake,执行权留给操作者 ----

    fn bake_app() -> App {
        let mut app = fake_app();
        app.screen = Screen::Recipes;
        app.rc_domain = Domain::Recipes;
        app.ls = Some(
            serde_json::from_str::<LsData>(
                r#"{"domain":"all","recipes":[{"name":"research-desk","version":"1.0.0"},{"name":"desk"}],"components":[],"sources":[],"profiles":[],"locks":[]}"#,
            )
            .unwrap(),
        );
        app.capabilities
            .record(Capability::RecipeBake, capability::Probe::Available);
        app
    }

    #[test]
    fn 建议命令_三种缺前提各有可区分门_互不冒充() {
        // 齐备:门放行
        let app = bake_app();
        assert!(app.bake_suggest_gate().is_ok(), "三前提齐备门放行");
        // 选中行不是配方(配方域为空)
        let mut app = bake_app();
        app.ls = Some(serde_json::from_str::<LsData>(r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#).unwrap());
        assert_eq!(app.bake_suggest_gate(), Err(commands::BakeGate::NotARecipe));
        // 未设数据仓根
        let mut app = bake_app();
        app.data_root = None;
        assert_eq!(app.bake_suggest_gate(), Err(commands::BakeGate::NoDataRoot));
        // 引擎没有 recipe bake 能力
        let mut app = bake_app();
        app.capabilities
            .record(Capability::RecipeBake, capability::Probe::Unavailable);
        assert_eq!(
            app.bake_suggest_gate(),
            Err(commands::BakeGate::NoCapability)
        );
    }

    #[test]
    fn 建议命令_c键取用只读选中_面板不执行不入队() {
        let mut app = bake_app();
        assert!(!app.rc_bake_sel, "初始未选中");
        let queued = app.queue.len();
        app.update(Message::Key(key('c')));
        assert!(app.rc_bake_sel, "c 进入只读选中态");
        assert_eq!(app.queue.len(), queued, "取用不入队:面板不执行 bake");
        assert!(
            app.log.iter().any(|l| l.contains("执行权在你")),
            "日志说清执行权归属:{:?}",
            app.log
        );
        app.update(Message::Key(key('c')));
        assert!(!app.rc_bake_sel, "再按取消选中");
        // 别的域按 c 不动作(建议命令只属于配方域)
        let mut app = bake_app();
        app.rc_domain = Domain::Locks;
        app.update(Message::Key(key('c')));
        assert!(!app.rc_bake_sel, "非配方域不给建议命令选中态");
    }

    #[test]
    fn 建议命令_闸不过时c只给指路_不翻状态不入队() {
        // 键盘与鼠标同源:命中侧在闸不放行时不产出目标,键盘同样不得
        // 翻转只读选中态——只留一句与呈现层同源的指路。
        let mut app = bake_app();
        app.capabilities
            .record(Capability::RecipeBake, capability::Probe::Unavailable);
        let n = app.queue.len();
        app.update(Message::Key(key('c')));
        assert!(!app.rc_bake_sel, "引擎无能力时 c 不翻只读选中态");
        assert_eq!(app.queue.len(), n, "闸不过不入队(面板不执行)");
        assert!(
            app.log[0].contains("引擎缺 recipe bake 能力"),
            "缺能力的指路在日志:{:?}",
            app.log[0]
        );
        assert!(
            app.log[0].contains("升级所配 nw"),
            "指路带下一步:{:?}",
            app.log[0]
        );
        // 未设数据仓根:另一种指路,与「缺能力」互不冒充
        let mut app = bake_app();
        app.data_root = None;
        app.update(Message::Key(key('c')));
        assert!(!app.rc_bake_sel, "未设根时 c 不翻只读选中态");
        assert!(
            app.log[0].contains("数据仓根未设"),
            "未设根的指路在日志:{:?}",
            app.log[0]
        );
        assert!(
            !app.log[0].contains("recipe bake 能力"),
            "两种缺前提各说各话:{:?}",
            app.log[0]
        );
        // 三前提齐备:照旧翻转,键盘与鼠标同一后果
        let mut app = bake_app();
        app.update(Message::Key(key('c')));
        assert!(app.rc_bake_sel, "门放行时 c 取用只读选中态");
        assert!(
            app.log[0].contains("执行权在你"),
            "取用说清执行权归属:{:?}",
            app.log[0]
        );
    }

    #[test]
    fn 建议命令_recipe_new_三前提各有可区分门_互不冒充() {
        // 齐备:门放行(面板仍只读呈现,不执行)
        let mut app = bake_app();
        app.capabilities
            .record(Capability::RecipeNew, capability::Probe::Available);
        app.data_root = Some("/tmp".into());
        assert!(app.recipe_new_suggest_gate().is_ok(), "三前提齐备门放行");
        // 选中行不是配方(配方域为空)
        let mut app = bake_app();
        app.ls = Some(serde_json::from_str::<LsData>(r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#).unwrap());
        assert_eq!(
            app.recipe_new_suggest_gate(),
            Err(commands::RecipeNewGate::NotARecipe)
        );
        // 未设数据仓根
        let mut app = bake_app();
        app.data_root = None;
        assert_eq!(
            app.recipe_new_suggest_gate(),
            Err(commands::RecipeNewGate::NoDataRoot)
        );
        // 引擎没有 recipe new 能力
        let mut app = bake_app();
        app.capabilities
            .record(Capability::RecipeNew, capability::Probe::Unavailable);
        assert_eq!(
            app.recipe_new_suggest_gate(),
            Err(commands::RecipeNewGate::NoCapability)
        );
    }

    #[test]
    fn 建议命令_recipe_new_面板没有任何入队路径_只读呈现() {
        // ADR-0003:面板只呈现与编排,执行权留给操作者。recipe new 与 bake
        // 同源同层——同一个纯函数层算 argv 与展示串,但 App 没有任何路径
        // 会把它下发(没有键位、没有命中区、没有队列入口)。
        let mut app = bake_app();
        app.capabilities
            .record(Capability::RecipeNew, capability::Probe::Available);
        app.data_root = Some("/tmp".into());
        // 配方域键面全按一遍(含 bake 的 c 与五域导航/搜索)
        for code in ['h', 'l', 'j', 'k', '/', 'c', 'r', 'n', 'y'] {
            app.update(Message::Key(key(code)));
        }
        for code in [KeyCode::Up, KeyCode::Down, KeyCode::Enter, KeyCode::Esc] {
            app.update(Message::Key(KeyEvent::new(code, KeyModifiers::NONE)));
        }
        assert!(
            app.queue.is_empty(),
            "recipe new 永不下发:队列一个都不该有:{:?}",
            app.queue
        );
        // 铁律的另一半:建议命令本身不带任何写旗标
        let s =
            crate::engine::commands::recipe_new_suggest(Some("research-desk"), Some("/tmp"), true)
                .expect("三前提齐备");
        for a in &s.argv {
            assert!(!a.contains("--yes"), "建议命令永不带确认闸:{a}");
        }
    }

    #[test]
    fn 建议命令_移选中与切域都复位只读选中态() {
        let mut app = bake_app();
        app.update(Message::Key(key('c')));
        assert!(app.rc_bake_sel);
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert!(!app.rc_bake_sel, "选中行变了,命令跟着变,选中态复位");
        app.update(Message::Key(key('c')));
        assert!(app.rc_bake_sel);
        app.update(Message::Key(key('l')));
        assert!(!app.rc_bake_sel, "切域复位");
    }

    // ---- #61 五域搜索过滤:纯本地筛选,选中语义不因过滤漂移 ----

    /// 配方域两条(bake_app 的夹具即两条配方)+ 数据仓根,便于验 `c` 取用。
    fn filter_app() -> App {
        let mut app = bake_app();
        app.data_root = Some("/tmp".into());
        app
    }

    /// 进过滤输入态、敲词、收词(走真实键路,与 tests.rs 的 type_str 同一条)。
    fn filter_type(app: &mut App, q: &str) {
        app.update(Message::Key(key('/')));
        for c in q.chars() {
            app.update(Message::Key(key(c)));
        }
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
    }

    #[test]
    fn 过滤后_上下键与鼠标点行仍同义_命中区随过滤后条数收缩() {
        let mut app = filter_app();
        filter_type(&mut app, "research");
        assert_eq!(app.domain_len(), 1, "过滤后只剩命中那条");
        // 命中区只产过滤后的条数:点第 0 行 = ↑/↓ 落在同一行
        for (w, h) in [(80u16, 30u16), (66, 50)] {
            app.size = Rect::new(0, 0, w, h);
            let hits = hitmap(app.size, &app, size_class(app.size));
            let rows: Vec<Rect> = hits
                .iter()
                .filter(|(_, t)| matches!(t, Target::SelectRow(_)))
                .map(|(r, _)| *r)
                .collect();
            assert_eq!(rows.len(), 1, "{w}×{h} 命中区随过滤后条数收缩:{rows:?}");
            click(&mut app, rows[0]);
            assert_eq!(app.rc_sel, 0, "{w}×{h} 点行 = 选中");
        }
        // Esc 清空:两条都回来,命中区回到全量条数
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.domain_len(), 2, "Esc 清空过滤词");
        app.size = Rect::new(0, 0, 80, 30);
        let hits = hitmap(app.size, &app, size_class(app.size));
        assert_eq!(
            hits.iter()
                .filter(|(_, t)| matches!(t, Target::SelectRow(_)))
                .count(),
            2,
            "清空后命中区回到全量"
        );
        let r = hits
            .iter()
            .find(|(_, t)| matches!(t, Target::SelectRow(1)))
            .map(|(r, _)| *r)
            .expect("第 2 行有命中区");
        click(&mut app, r);
        assert_eq!(app.rc_sel, 1, "点第 2 行");
        app.update(Message::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        assert_eq!(app.rc_sel, 0, "↑ 与点行同一本账");
    }

    #[test]
    fn 过滤后选中行不错位_建议命令作用于看得见的那一行() {
        let mut app = filter_app();
        filter_type(&mut app, "research");
        assert_eq!(app.rc_sel, 0, "键入后选中回顶");
        app.update(Message::Key(key('c')));
        assert!(app.rc_bake_sel, "过滤后仍可取用(只读)");
        assert_eq!(
            app.selected_recipe().map(|r| r.name.as_str()),
            Some("research-desk"),
            "取用的是看得见的那一行,不是原始数组同下标的另一条"
        );
        assert!(app.queue.is_empty(), "取用不入队、不执行:{:?}", app.queue);
        // 换成一个没有命中的词:可见列表空,门不放行、不给半条命令
        filter_type(&mut app, "desk");
        assert_eq!(app.selected_recipe().map(|r| r.name.as_str()), None);
        assert!(
            app.bake_suggest_gate().is_err(),
            "无可见配方行时门不放行(不给半条命令)"
        );
    }

    #[test]
    fn 建议命令_鼠标点键位行与c同路_命中区只在门放行时产出() {
        let mut app = bake_app();
        app.size = Rect::new(0, 0, 80, 30);
        for (w, h) in [(80, 30), (66, 50)] {
            app.size = Rect::new(0, 0, w, h);
            let row = hit_target(&app, |t| *t == Target::ToggleSuggest);
            click(&mut app, row);
            assert!(app.rc_bake_sel, "{w}×{h} 点键位行 = 键盘 c");
            assert!(app.queue.is_empty(), "{w}×{h} 点取用不入队(不执行)");
            click(&mut app, row);
            assert!(!app.rc_bake_sel, "{w}×{h} 再点取消");
        }
        // 门不放行(引擎无能力):命中区不产出,鼠标点哪儿都不取用
        let mut app = bake_app();
        app.capabilities
            .record(Capability::RecipeBake, capability::Probe::Unavailable);
        app.size = Rect::new(0, 0, 80, 30);
        let hits = hitmap(app.size, &app, size_class(app.size));
        assert!(
            hits.iter().all(|(_, t)| *t != Target::ToggleSuggest),
            "闸不放行时无取用命中区(与登记落账闸同纪律)"
        );
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
            &vec![
                "up".to_string(),
                "--lock".into(),
                // 载体文件路径(落点同 launch_selected_lock:NW_HOME/locks/<id>.nwlock)
                format!("{}/locks/9f8e.nwlock", app.nw_home)
            ],
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

    // ---- #29 能力探测门(App 行为缝:探针下发 / 三路回音 / 入口门 / 指路) ----

    /// 缺能力夹具:tag/vps 按 0.1.0 实证形态回放(空 stdout + rc 3 +
    /// stderr 人话「未知子命令」),source register/recheck 回 envelope(存在)。
    fn missing_caps_runner() -> FakeRunner {
        FakeRunner::with_stderr(&[
            ("tag", 3, "", "nw: 未知子命令 \"tag\"\n用法见 stderr"),
            ("vps", 3, "", "nw: 未知子命令 \"vps\""),
            (
                "source register",
                0,
                r#"{"schema":"nw/source-register.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{}}"#,
                "",
            ),
            (
                "source recheck",
                0,
                r#"{"schema":"nw/source-recheck.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"catalog.json","checked":[],"drift":[]}}"#,
                "",
            ),
            (
                "ls",
                0,
                r#"{"schema":"nw/ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}}"#,
                "",
            ),
        ])
    }

    /// 齐备夹具:四条探针都回 envelope(等价于具备能力的引擎回放)。
    fn capable_runner() -> FakeRunner {
        FakeRunner::with_stderr(&[
            (
                "tag",
                0,
                r#"{"schema":"nw/tag-add.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{}}"#,
                "",
            ),
            (
                "vps",
                0,
                r#"{"schema":"nw/vps.v1","nw":"0.2.0","ok":true,"errors":[],"warnings":[],"data":{}}"#,
                "",
            ),
            (
                "source register",
                0,
                r#"{"schema":"nw/source-register.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{}}"#,
                "",
            ),
            (
                "source recheck",
                0,
                r#"{"schema":"nw/source-recheck.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"catalog.json","checked":[],"drift":[]}}"#,
                "",
            ),
            (
                "ls",
                0,
                r#"{"schema":"nw/ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}}"#,
                "",
            ),
        ])
    }

    /// 把 bootstrap 下发的全部回音灌回状态树(生产同路:主循环收 tx)。
    fn feed_bootstrap(
        app: &mut App,
        tx: &mpsc::UnboundedSender<EngineEvent>,
        rx: &mut mpsc::UnboundedReceiver<EngineEvent>,
    ) {
        app.bootstrap(tx);
        while let Ok(ev) = rx.try_recv() {
            app.update(Message::Engine(ev));
        }
    }

    #[tokio::test]
    async fn 能力探针经唯一入口下发_无害无写旗标() {
        let mut app = fake_app();
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.bootstrap(&tx);
        let mut argvs: Vec<Vec<String>> = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let EngineEvent::Finished(o) = ev {
                argvs.push(o.args);
            }
        }
        for cap in Capability::ALL {
            assert!(
                argvs.contains(&cap.probe_args()),
                "{:?} 探针必须经 spawn_engine 下发",
                cap
            );
        }
        // 无害性:探针不携带任何会动文件的旗标
        for a in &argvs {
            assert!(
                !a.iter()
                    .any(|f| f == "--yes" || f == "--root" || f == "--lock"),
                "探针必须无害:{a:?}"
            );
        }
    }

    #[tokio::test]
    async fn 能力门_缺能力入口禁用并指路_齐备正常呈现() {
        let mut app = App::with_runner(Arc::new(missing_caps_runner()));
        let (tx, mut rx) = mpsc::unbounded_channel();
        feed_bootstrap(&mut app, &tx, &mut rx);
        // 实证锚点:0.1.0 无 tag/vps 两个命令组,这两个入口一律不呈现为可用
        assert!(!app.entry_enabled(Capability::Tag));
        assert!(!app.entry_enabled(Capability::Vps));
        // 能力齐备的两个:入口正常呈现
        assert!(app.entry_enabled(Capability::SourceRegister));
        assert!(app.entry_enabled(Capability::SourceRecheck));
        // 操作者可见:禁用组 + 指路(说清缺的是引擎能力、不是面板坏了)
        app.splash = false;
        app.screen = Screen::Settings;
        let s = crate::testutil::frame_display(&app, 110, 30);
        assert!(s.contains("入口禁用:vps、标签"), "缺能力入口禁用可见:{s}");
        assert!(
            s.contains(crate::engine::GATE_HINT),
            "指路文案可见(缺的是引擎能力,不是面板坏了)"
        );
        assert!(s.contains("✓外部源登记"), "齐备能力正常呈现:{s}");

        // 等价回放另一侧:齐备引擎全部放行,不出现禁用组
        let mut app2 = App::with_runner(Arc::new(capable_runner()));
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        feed_bootstrap(&mut app2, &tx2, &mut rx2);
        for cap in Capability::ALL {
            assert!(app2.entry_enabled(cap), "齐备引擎下 {:?} 应放行", cap);
        }
        app2.splash = false;
        app2.screen = Screen::Settings;
        let s2 = crate::testutil::frame_display(&app2, 110, 30);
        assert!(!s2.contains("入口禁用"), "齐备时无禁用组:{s2}");
        assert!(s2.contains("✓标签"), "齐备时标签入口呈现为可用");
    }

    #[tokio::test]
    async fn 能力探测失败_走既有指路屏_面板不瘫痪() {
        let mut app = App::with_runner(Arc::new(crate::engine::MissingRunner {
            attempted: vec!["PATH".into()],
        }));
        app.engine.found = false;
        let (tx, mut rx) = mpsc::unbounded_channel();
        feed_bootstrap(&mut app, &tx, &mut rx);
        // 探测全部未决:入口一律不放行(缺省不猜)
        for cap in Capability::ALL {
            assert!(!app.entry_enabled(cap), "未决不放行:{cap:?}");
        }
        // 引擎缺失:走既有指路屏,不新造失败面
        app.splash = false;
        app.screen = Screen::Settings;
        let s = crate::testutil::frame_display(&app, 110, 30);
        assert!(s.contains("找不到 nw"), "探测失败走既有指路屏:{s}");
        // 不瘫痪:探测回音后其余部分照常可操作
        app.update(Message::Key(key('?')));
        assert!(app.help, "探测失败后帮助浮层仍可开(面板不瘫痪)");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(!app.help, "帮助浮层可关");
        app.update(Message::Key(key('j')));
        assert_ne!(app.screen, Screen::Settings, "探测失败后仍可切屏");
    }

    // ---- #30 外部源登记·预审段(五路回放 + 入口门 + 两段命中) ----

    /// 预审成功夹具:所配 0.1.0 的 data 形状——条目(名字+路径)、
    /// 验收 5(+1) 冻结 id、拟落账记录;written=false 即「文件不落」。
    const REG_OK: &str = r#"{"schema":"nw/source-register.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"repo":"https://github.com/owner/tiny-skills","kind":"skill","name":"tiny-skills","category":[],"commit":"abc1234","entries":[{"name":"greeter","path":"skills/greeter/SKILL.md"},{"name":"scanner","path":"skills/scanner/SKILL.md"}],"acceptance":[{"id":"register.repo_structure","status":"pass","message":"扫到 2 个 skill 条目"},{"id":"register.frontmatter","status":"pass","message":"2 个 SKILL.md frontmatter 合法"},{"id":"register.name_unique","status":"pass","message":"2 个条目名互不重复"},{"id":"register.metadata_ignored","status":"pass","message":"按规则跳过 1 项元数据"},{"id":"register.extension_manifest","status":"skip","message":"仅 --kind extension 适用"},{"id":"register.confirmed","status":"skip","message":"未给 --yes:拟落账进这份 JSON,文件不落(#19 确认闸)"}],"catalogPath":"/data/root/catalog.json","written":false,"updated":false,"action":""}}"#;

    /// 验收有 fail 的预审:rc 3、ok false,第 2 条 frontmatter 败。
    const REG_FAIL: &str = r#"{"schema":"nw/source-register.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"验收有 1 条 fail,拒绝落账","hint":"修好条目再登记"}],"warnings":[],"data":{"repo":"owner/broken","kind":"skill","name":"broken","category":[],"commit":"","entries":[{"name":"bad","path":"skills/bad/SKILL.md"}],"acceptance":[{"id":"register.repo_structure","status":"pass","message":"扫到 1 个条目"},{"id":"register.frontmatter","status":"fail","message":"SKILL.md frontmatter 缺 name","path":"skills/bad/SKILL.md"},{"id":"register.confirmed","status":"skip","message":"未给 --yes:拟落账进这份 JSON,文件不落(#19 确认闸)"}],"catalogPath":"/data/root/catalog.json","written":false,"updated":false,"action":""}}"#;

    /// rc≠0 但验收区空(取回失败等):errors 带人话,ok false。
    const REG_RC3: &str = r#"{"schema":"nw/source-register.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"取回 https://github.com/owner/nope 失败(git clone)","hint":"认证: gh auth status;或仓库名写错"}],"warnings":[],"data":{"repo":"owner/nope","kind":"skill","name":"nope","category":[],"commit":"","entries":[],"acceptance":[],"catalogPath":"/data/root/catalog.json","written":false,"updated":false,"action":""}}"#;

    /// 登记夹具:能力齐备 + 直达外部源域(入口三前提就位)。
    fn reg_app(rules: &[(&str, i32, &str)]) -> App {
        let mut app = App::with_runner(Arc::new(FakeRunner::new(rules)));
        app.capabilities
            .record(Capability::SourceRegister, Probe::Available);
        app.splash = false;
        app.screen = Screen::Recipes;
        app.rc_domain = Domain::Sources;
        app
    }

    /// r 发起 → 填链 → Enter:走到 kind 选择态(合法链接才走得到)。
    fn reg_to_kindpick(app: &mut App, link: &str) {
        app.update(Message::Key(key('r')));
        assert_eq!(app.reg_state, RegState::RepoInput, "入口键进输入态");
        type_str(app, link);
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.reg_state, RegState::KindPick, "合法链接进 kind 选择");
    }

    /// 按当前队尾 argv 回放回音并灌进 update(与 #28 的既有先例同款)。
    async fn reg_feed(app: &mut App, json: &str, rc: i32) {
        let argv = app.queue.last().expect("队尾有预审 argv").clone();
        let runner = FakeRunner::new(&[("source register", rc, json)]);
        let out = runner.run(&argv).await;
        app.update(Message::Engine(EngineEvent::Finished(Box::new(out))));
    }

    #[test]
    fn 登记入口_看得见_指路键位与列表行() {
        let mut app = reg_app(&[]);
        app.ls = Some(
            serde_json::from_str(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#,
            )
            .unwrap(),
        );
        // 输入态 HUD 指路
        app.update(Message::Key(key('r')));
        let s = crate::testutil::frame_display(&app, 110, 30);
        assert!(s.contains("登记填链"), "输入态 HUD 指路可见:{s}");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        // 列表行(能力齐备时)给发起键位
        let s2 = crate::testutil::frame_display(&app, 110, 30);
        assert!(s2.contains("按 r 登记新外部源"), "列表行可见键位:{s2}");
        // 帮助浮层列键位
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('?'),
            KeyModifiers::NONE,
        )));
        let h = crate::testutil::frame_display(&app, 110, 30);
        assert!(h.contains("登记外部源"), "帮助浮层列键位:{h}");
    }

    #[tokio::test]
    async fn 登记预审_成功路_argv不带_yes_回音进确认态() {
        let mut app = reg_app(&[("source register", 0, REG_OK)]);
        reg_to_kindpick(&mut app, "owner/tiny-skills");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.reg_state, RegState::PreAudit, "Enter 下发预审");
        let argv = app.queue.last().expect("预审入队").clone();
        assert_eq!(argv[..2], ["source", "register"]);
        assert!(
            !argv.iter().any(|s| s == "--yes"),
            "预审铁律:argv 不带 --yes(文件不落):{argv:?}"
        );
        assert!(
            argv.windows(2).any(|w| w[0] == "--root"),
            "--root 指数据仓根:{argv:?}"
        );
        assert!(
            !argv.iter().any(|s| s == "--category"),
            "类别输入不给(0.1.0 无该能力)"
        );
        reg_feed(&mut app, REG_OK, 0).await;
        assert_eq!(app.reg_state, RegState::Review, "回音进确认态");
        assert!(app.reg_can_commit(), "无 fail 且 rc 0:闸口径放行");
    }

    #[tokio::test]
    async fn 登记预审_验收fail_不进下一步_屏上见败因() {
        let mut app = reg_app(&[("source register", 3, REG_FAIL)]);
        reg_to_kindpick(&mut app, "owner/broken");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        reg_feed(&mut app, REG_FAIL, 3).await;
        assert_eq!(app.reg_state, RegState::Review, "失败也要呈现");
        assert!(!app.reg_can_commit(), "验收有 fail 不可落账");
        let s = crate::testutil::frame_display(&app, 110, 30);
        assert!(s.contains("不可落账"), "闸结论可见:{s}");
        assert!(s.contains("register.frontmatter"), "按冻结 id 呈现:{s}");
        assert!(s.contains("败"), "fail 逐条可见:{s}");
        // 确认态没有绑定 Enter:本票根本不给「进入下一步」的键路
        let qlen = app.queue.len();
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.queue.len(), qlen, "fail 态按 Enter 不下发任何命令");
    }

    #[tokio::test]
    async fn 登记预审_rc非0_不进下一步_人话可见() {
        let mut app = reg_app(&[("source register", 3, REG_RC3)]);
        reg_to_kindpick(&mut app, "owner/nope");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        reg_feed(&mut app, REG_RC3, 3).await;
        assert_eq!(app.reg_state, RegState::Review);
        assert!(!app.reg_can_commit(), "退出码非 0 不可落账");
        let s = crate::testutil::frame_display(&app, 110, 30);
        assert!(s.contains("不可落账"), "闸结论可见:{s}");
        assert!(s.contains("取回"), "引擎人话原样呈现:{s}");
    }

    #[test]
    fn 未设数据仓根_登记入口不给_指路与缺能力分说() {
        let mut app = reg_app(&[("source register", 0, REG_OK)]);
        app.data_root = None;
        app.update(Message::Key(key('r')));
        assert_eq!(app.reg_state, RegState::Idle, "缺根不进流程");
        assert!(
            app.log[0].contains("数据仓"),
            "指路说数据仓:{:?}",
            app.log[0]
        );
        assert!(
            !app.log[0].contains("引擎能力"),
            "缺根指路不冒充缺能力:{:?}",
            app.log[0]
        );
        assert!(app.queue.is_empty(), "缺根不下发命令");
    }

    #[test]
    fn 引擎无source能力_入口禁用并指路_列表行可见禁用() {
        let mut app = reg_app(&[]);
        app.capabilities
            .record(Capability::SourceRegister, Probe::Unavailable);
        app.ls = Some(
            serde_json::from_str(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#,
            )
            .unwrap(),
        );
        app.update(Message::Key(key('r')));
        assert_eq!(app.reg_state, RegState::Idle, "缺能力不进流程");
        assert!(
            app.log[0].contains("引擎能力") && app.log[0].contains("面板坏了"),
            "指路说缺的是引擎能力:{:?}",
            app.log[0]
        );
        let s = crate::testutil::frame_display(&app, 110, 30);
        assert!(s.contains("登记禁用"), "外部源域列表行可见禁用指路:{s}");
        assert!(app.queue.is_empty(), "缺能力不下发命令");
    }

    #[test]
    fn repo链接_当场拒绝留改_合法才进下一步() {
        let mut app = reg_app(&[]);
        app.update(Message::Key(key('r')));
        type_str(&mut app, "not a repo");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.reg_state, RegState::RepoInput, "非法格式当场拒绝留驻");
        assert!(app.log[0].contains("被拒"), "拒绝留证据:{:?}", app.log[0]);
        assert!(!app.reg_repo_buf.is_empty(), "缓冲不丢,可改");
        // 清掉重填(退格是通用态机行为,这里只造夹具)
        while !app.reg_repo_buf.is_empty() {
            app.update(Message::Key(KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::NONE,
            )));
        }
        type_str(&mut app, "owner/tiny-skills");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.reg_state, RegState::KindPick, "改对了就放行");
    }

    #[test]
    fn repo链接形状_三收三拒() {
        for ok in [
            "owner/repo",
            "https://github.com/owner/repo",
            "git@github.com:owner/repo.git",
            "https://github.com/a/b.git",
        ] {
            assert!(valid_repo_link(ok), "应放行:{ok}");
        }
        for bad in [
            "owner",
            "owner/repo/extra",
            "http://github.com/o/r",
            "https://gitlab.com/o/r",
            "owner/repo with space",
            "",
        ] {
            assert!(!valid_repo_link(bad), "应拒绝:{bad:?}");
        }
    }

    #[test]
    fn kind二选一_s与e键与方向键_argv带所选kind() {
        let mut app = reg_app(&[("source register", 0, REG_OK)]);
        reg_to_kindpick(&mut app, "owner/ext");
        app.update(Message::Key(key('e')));
        assert_eq!(app.reg_kind, RegKind::Extension, "e 选 extension");
        app.update(Message::Key(key('s')));
        assert_eq!(app.reg_kind, RegKind::Skill, "s 选 skill");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let argv = app.queue.last().expect("入队").clone();
        assert!(
            argv.windows(2).any(|w| w[0] == "--kind" && w[1] == "skill"),
            "--kind 随所选项:{argv:?}"
        );
        assert!(!argv.iter().any(|s| s == "--yes"), "仍不带 --yes");
    }

    #[tokio::test]
    async fn 各态返航_不留半条记录_and_迟到回音不重开() {
        let mut app = reg_app(&[("source register", 0, REG_OK)]);
        // 输入态
        app.update(Message::Key(key('r')));
        type_str(&mut app, "owner/x");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.reg_state, RegState::Idle, "输入态 Esc 返航");
        assert!(app.reg_repo_buf.is_empty(), "缓冲清空不留痕");
        // kind 态
        reg_to_kindpick(&mut app, "owner/x");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.reg_state, RegState::Idle, "kind 态 Esc 返航");
        // 确认态
        reg_to_kindpick(&mut app, "owner/x");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        reg_feed(&mut app, REG_OK, 0).await;
        assert_eq!(app.reg_state, RegState::Review);
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.reg_state, RegState::Idle, "确认态 Esc 返航");
        assert!(app.reg_result.is_none(), "返航不留半条回音");
        assert!(app.log[0].contains("登记返航"), "返航留日志");
        // 迟到的回音不幽灵重开
        let argv = vec!["source".into(), "register".into()];
        let out = FakeRunner::new(&[("source register", 0, REG_OK)])
            .run(&argv)
            .await;
        app.update(Message::Engine(EngineEvent::Finished(Box::new(out))));
        assert_eq!(app.reg_state, RegState::Idle, "已返航的回音丢弃");
    }

    #[tokio::test]
    async fn 确认屏_条目验收拟落账与待定原样呈现_两档不炸() {
        let mut app = reg_app(&[("source register", 0, REG_OK)]);
        reg_to_kindpick(&mut app, "owner/tiny-skills");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        reg_feed(&mut app, REG_OK, 0).await;
        assert_eq!(app.reg_state, RegState::Review);
        let s = crate::testutil::frame_display(&app, 110, 30);
        assert!(s.contains("greeter"), "条目名呈现:{s}");
        assert!(s.contains("skills/greeter/SKILL.md"), "条目路径呈现:{s}");
        assert!(s.contains("register.confirmed"), "冻结 id 原样:{s}");
        assert!(s.contains("待定"), "未确认显示待定不是错误:{s}");
        assert!(s.contains("拟落账"), "拟落账记录呈现:{s}");
        assert!(s.contains("文件不落"), "预审不落文件可见:{s}");
        assert!(s.contains("预审通过"), "闸结论可见:{s}");
        // 紧凑档(80×50)不炸且仍见确认屏
        let c = crate::testutil::frame_display(&app, 80, 50);
        assert!(c.contains("预审确认"), "紧凑档确认屏可见:{c}");
    }

    #[tokio::test]
    async fn 确认屏_不认识的schema_需升级且不可落账() {
        const REG_V2: &str = r#"{"schema":"nw/source-register.v2","nw":"9.9.9","ok":true,"errors":[],"warnings":[],"data":{"whatever":1}}"#;
        let mut app = reg_app(&[("source register", 0, REG_V2)]);
        reg_to_kindpick(&mut app, "owner/x");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        reg_feed(&mut app, REG_V2, 0).await;
        assert_eq!(app.reg_state, RegState::Review);
        assert!(!app.reg_can_commit(), "需升级的回音不可落账");
        let s = crate::testutil::frame_display(&app, 110, 30);
        assert!(s.contains("不可定型"), "不可定型可见:{s}");
        assert!(s.contains("nw/source-register.v2"), "schema 原文可见:{s}");
        assert!(s.contains("9.9.9"), "nw 版本可见:{s}");
    }

    #[tokio::test]
    async fn 登记两段命中_点选项与预审与返航_与键盘同路() {
        let mut app = reg_app(&[("source register", 0, REG_OK)]);
        app.size = Rect::new(0, 0, 110, 30);
        reg_to_kindpick(&mut app, "owner/tiny-skills");
        // 输入态:浮层未就位(命中零散件,后续态机自己收)
        // kind 屏:点第二选项行 = 选中 extension
        let row = hit_target(&app, |t| matches!(t, Target::SelectRow(1)));
        click(&mut app, row);
        assert_eq!(app.reg_kind, RegKind::Extension, "点行选中");
        // 点 Enter 预审段 = 下发(与 Enter 同路);argv 仍不带 --yes
        let act = hit_target(&app, |t| *t == Target::Activate);
        click(&mut app, act);
        assert_eq!(app.reg_state, RegState::PreAudit, "点预审段同路");
        let argv = app.queue.last().expect("入队").clone();
        assert!(
            argv.windows(2)
                .any(|w| w[0] == "--kind" && w[1] == "extension"),
            "点选中与键选同路:{argv:?}"
        );
        assert!(!argv.iter().any(|s| s == "--yes"), "点选也绝不带 --yes");
        // 回音 → 确认态;点屏任意处 = 返航
        let runner = FakeRunner::new(&[("source register", 0, REG_OK)]);
        let out = runner.run(&argv).await;
        app.update(Message::Engine(EngineEvent::Finished(Box::new(out))));
        assert_eq!(app.reg_state, RegState::Review);
        let dm = hit_target(&app, |t| *t == Target::DismissPopup);
        click(&mut app, dm);
        assert_eq!(app.reg_state, RegState::Idle, "确认态点屏 = 返航");
    }

    #[test]
    fn 登记kind屏_滚轮只移选项_不下发不越界() {
        let mut app = reg_app(&[("source register", 0, REG_OK)]);
        reg_to_kindpick(&mut app, "owner/x");
        let qlen = app.queue.len();
        app.wheel_sel(ScrollDir::Down);
        assert_eq!(app.reg_kind, RegKind::Extension, "滚轮下移选中 extension");
        app.wheel_sel(ScrollDir::Down);
        assert_eq!(app.reg_kind, RegKind::Extension, "到末项不越界(两行选项)");
        app.wheel_sel(ScrollDir::Up);
        assert_eq!(app.reg_kind, RegKind::Skill, "滚轮上移回 skill");
        assert_eq!(app.queue.len(), qlen, "滚轮不下发命令");
    }

    // ---- #31 落账段:图形确认后才 --yes;结果/互斥/返航 ----

    /// 落账成功回音:written=true、rc 0、ok true(引擎把 catalog.json
    /// 写进数据仓根后的形状)。
    const REG_WRITTEN: &str = r#"{"schema":"nw/source-register.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"repo":"https://github.com/owner/tiny-skills","kind":"skill","name":"tiny-skills","category":[],"commit":"abc1234","entries":[{"name":"greeter","path":"skills/greeter/SKILL.md"},{"name":"scanner","path":"skills/scanner/SKILL.md"}],"acceptance":[{"id":"register.repo_structure","status":"pass","message":"扫到 2 个 skill 条目"},{"id":"register.confirmed","status":"pass","message":"已确认:编目落账"}],"catalogPath":"/data/root/catalog.json","written":true,"updated":false,"action":"registered"}}"#;

    /// 一路走到确认态(预审成功回音已灌入)。
    async fn reg_to_review(app: &mut App, link: &str) {
        reg_to_kindpick(app, link);
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.reg_state, RegState::PreAudit);
        reg_feed(app, REG_OK, 0).await;
        assert_eq!(app.reg_state, RegState::Review);
        assert!(app.reg_can_commit());
    }

    #[tokio::test]
    async fn 登记落账_仅图形确认后带_yes_成功路结果与即时刷新() {
        let mut app = reg_app(&[("--yes", 0, REG_WRITTEN), ("source register", 0, REG_OK)]);
        let root = app.usable_data_root().expect("注入的临时根可用");
        reg_to_review(&mut app, "owner/tiny-skills").await;
        let qlen = app.queue.len();
        // 图形确认:Enter —— 全流程唯一能产生 --yes 的转换
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.reg_state, RegState::Commit, "确认后才进落账在途");
        assert!(app.assembling, "写互斥点亮(登记中不接装配)");
        let argv = app.queue.last().expect("落账入队").clone();
        assert!(
            argv.iter().any(|s| s == "--yes"),
            "落账 argv 带 --yes:{argv:?}"
        );
        assert!(
            argv.windows(2).any(|w| w[0] == "--root" && w[1] == root),
            "--root 指数据仓根:{argv:?}"
        );
        // 落账回音(成功):结果态 + 互斥收敛 + 外部源域即时刷新
        let runner = FakeRunner::new(&[("source register", 0, REG_WRITTEN)]);
        let out = runner.run(&argv).await;
        app.update(Message::Engine(EngineEvent::Finished(Box::new(out))));
        assert_eq!(app.reg_state, RegState::Done, "成功进结果态");
        assert!(!app.assembling, "回音收敛互斥");
        assert!(
            app.log.iter().any(|l| l.contains("git 变更")),
            "日志说明这是一次 git 变更:{:?}",
            app.log.iter().take(3).collect::<Vec<_>>()
        );
        assert!(
            app.queue.len() > qlen
                && app.queue.last().map(|a| a.first().map(String::as_str)) == Some(Some("ls")),
            "落账后入队 ls:外部源域即时刷新,无需重启"
        );
        // 结果屏:写盘位置 + git 提示 + 可复制命令都在屏上
        let s = crate::testutil::frame_display(&app, 110, 30);
        assert!(s.contains("catalog.json"), "写到哪个文件:{s}");
        assert!(s.contains("git 变更"), "git 变更提示:{s}");
        assert!(s.contains("nw source register"), "可复制命令:{s}");
        assert!(s.contains("--yes"), "命令带确认闸:{s}");
    }

    #[tokio::test]
    async fn 登记落账_预审未通过_enter不下发_yes() {
        let mut app = reg_app(&[("source register", 3, REG_FAIL)]);
        reg_to_kindpick(&mut app, "owner/broken");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        reg_feed(&mut app, REG_FAIL, 3).await;
        assert_eq!(app.reg_state, RegState::Review);
        assert!(!app.reg_can_commit(), "验收有 fail:闸关");
        let qlen = app.queue.len();
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.queue.len(), qlen, "闸关时 Enter 不入队任何命令");
        assert!(
            !app.queue.iter().any(|a| a.iter().any(|s| s == "--yes")),
            "预审未通过:任何路径都不出现 --yes"
        );
        assert!(
            app.log[0].contains("落账未下发"),
            "日志留证据:{}",
            app.log[0]
        );
        assert_eq!(app.reg_state, RegState::Review, "留在确认态可审阅");
    }

    #[tokio::test]
    async fn 登记落账_失败路_回确认态_闸关闭_互斥收敛() {
        let mut app = reg_app(&[("--yes", 3, REG_RC3), ("source register", 0, REG_OK)]);
        reg_to_review(&mut app, "owner/tiny-skills").await;
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(app.assembling);
        let argv = app.queue.last().expect("落账入队").clone();
        let runner = FakeRunner::new(&[("source register", 3, REG_RC3)]);
        let out = runner.run(&argv).await;
        app.update(Message::Engine(EngineEvent::Finished(Box::new(out))));
        assert_eq!(app.reg_state, RegState::Review, "失败回确认态");
        assert!(!app.assembling, "互斥收敛");
        assert!(
            !app.reg_can_commit(),
            "失败结论关闭落账闸(不可原地重发同一个写)"
        );
        let s = crate::testutil::frame_display(&app, 110, 30);
        assert!(s.contains("不可落账"), "闸结论在屏:{s}");
    }

    #[test]
    fn 登记与装配互斥_双向都点不着火() {
        let mut app = reg_app(&[]);
        // 登记进行中:装配屏 Enter 不开发射仪式
        app.update(Message::Key(key('r')));
        assert_eq!(app.reg_state, RegState::RepoInput);
        app.screen = Screen::Assemble;
        app.ls = Some(
            serde_json::from_str::<LsData>(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[{"lockId":"9f8e"}]}"#,
            )
            .unwrap(),
        );
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(!app.as_confirm, "登记进行中不接装配(反之亦然)");
        // 装配进行中:登记入口拒绝并留证据(先回配方屏,r 归它)
        app.cancel_register();
        app.assembling = true;
        app.screen = Screen::Recipes;
        app.update(Message::Key(key('r')));
        assert_eq!(app.reg_state, RegState::Idle, "装配中不给开登记");
        assert!(app.log[0].contains("互斥"), "日志留证据:{}", app.log[0]);
    }

    #[tokio::test]
    async fn 登记落账_在途返航_回音丢弃_互斥仍收敛() {
        let mut app = reg_app(&[("--yes", 0, REG_WRITTEN), ("source register", 0, REG_OK)]);
        reg_to_review(&mut app, "owner/tiny-skills").await;
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let argv = app.queue.last().expect("落账入队").clone();
        // 在途 Esc 返航:视图清空(引擎的写已在途中,面板不假装能收回)
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.reg_state, RegState::Idle, "返航清状态");
        assert!(app.reg_repo_buf.is_empty(), "不留半条记录");
        // 回音到达:按状态丢弃,但写互斥必须收敛(否则面板假装还在写)
        let qlen = app.queue.len();
        let runner = FakeRunner::new(&[("source register", 0, REG_WRITTEN)]);
        let out = runner.run(&argv).await;
        app.update(Message::Engine(EngineEvent::Finished(Box::new(out))));
        assert_eq!(app.reg_state, RegState::Idle, "丢弃不回填状态");
        assert!(app.reg_result.is_none(), "丢弃不幽灵重开");
        assert!(!app.assembling, "丢弃路也收敛互斥");
        // #31 评审 P1:回音报写盘成功 → 五域刷新必须入队(不因返航而丢),
        // 且日志明示「写可能已落盘」并指路 PR + 家长审复核。
        assert!(
            app.queue.len() > qlen
                && app.queue.last().map(|a| a.first().map(String::as_str)) == Some(Some("ls")),
            "成功回音触发五域刷新入队"
        );
        assert!(
            app.log.iter().any(|l| l.contains("登记写盘可能已完成")
                && l.contains("catalog.json 走 PR + 家长审复核")),
            "日志明示写可能已落盘并指路复核:{:?}",
            app.log.iter().take(3).collect::<Vec<_>>()
        );
    }

    // ---- PR #32 评审修复:q 返航 / 旧根回音 / 最小终端登记浮层 ----

    #[test]
    fn 登记kind态_q与小写esc同义返航() {
        let mut app = reg_app(&[]);
        reg_to_kindpick(&mut app, "owner/x");
        app.update(Message::Key(key('q')));
        assert_eq!(app.reg_state, RegState::Idle, "q 返航与 Esc 同义");
        assert!(app.reg_repo_buf.is_empty(), "不留半条记录");
        // 再进一次,Esc 仍同义(两个键一个后果)
        reg_to_kindpick(&mut app, "owner/x");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.reg_state, RegState::Idle, "Esc 返航不漂移");
    }

    #[test]
    fn 换根后旧根ls回音_不入账_新根回音照常() {
        let mut app = fake_app();
        let cur = app.usable_data_root().expect("注入的临时根可用");
        const LS_JSON: &str = r#"{"schema":"nw/ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"domain":"all","recipes":[{"name":"r1"}],"components":[],"sources":[],"profiles":[],"locks":[]}}"#;
        let echo = |repo: &str| EngineOutput {
            args: vec!["ls".into(), "--repo".into(), repo.into()],
            exit_code: crate::engine::ExitCode::Ok,
            ok: true,
            stderr_tail: vec![],
            envelope: serde_json::from_str(LS_JSON).ok(),
            raw_stdout: LS_JSON.into(),
            parse_error: None,
        };
        // 旧根的迟到回音:--repo 与当前数据仓根不一致 → 丢弃并留证据
        app.update(Message::Engine(EngineEvent::Finished(Box::new(echo(
            "/old-root-abc",
        )))));
        assert!(app.ls.is_none(), "旧根结果不入状态树");
        assert!(
            app.log[0].contains("数据仓根已换") && app.log[0].contains("/old-root-abc"),
            "日志留证据:{}",
            app.log[0]
        );
        // 当前根的回音照常入账(自愈刷新不受影响)
        app.update(Message::Engine(EngineEvent::Finished(Box::new(echo(&cur)))));
        assert!(app.ls.is_some(), "当前根的回音入账");
        assert!(
            app.log[0].contains("五域已刷新"),
            "日志留证据:{}",
            app.log[0]
        );
    }

    #[tokio::test]
    async fn 登记浮层_紧凑66x50_键位行可见_两段命中与落账同路() {
        let mut app = reg_app(&[("--yes", 0, REG_WRITTEN), ("source register", 0, REG_OK)]);
        app.size = Rect::new(0, 0, 66, 50);
        reg_to_review(&mut app, "owner/tiny-skills").await;
        // 确认屏在最小支持终端(66×50)下键位行两段可见
        let s = crate::testutil::frame_display(&app, 66, 50);
        assert!(s.contains("Enter 落账"), "左段键位行可见:{s}");
        assert!(s.contains("Esc 返航"), "右段键位行可见:{s}");
        // 右段返航:点与键盘 Esc 同路(紧凑态两段命中不炸)
        let back = hit_target(&app, |t| *t == Target::DismissPopup);
        click(&mut app, back);
        assert_eq!(app.reg_state, RegState::Idle, "点返航段同路");
        // 重进确认态:左段落账 = 图形确认(唯一 --yes 出口,紧凑态同路)
        reg_to_review(&mut app, "owner/tiny-skills").await;
        let act = hit_target(&app, |t| *t == Target::Activate);
        click(&mut app, act);
        assert_eq!(app.reg_state, RegState::Commit, "点落账段进在途");
        let argv = app.queue.last().expect("落账入队").clone();
        assert!(
            argv.iter().any(|a| a == "--yes"),
            "段命中与键盘同路也带 --yes:{argv:?}"
        );
        // 结果屏在 66×50 下渲染不炸:写盘位置与记录链接可见
        let runner = FakeRunner::new(&[("source register", 0, REG_WRITTEN)]);
        let out = runner.run(&argv).await;
        app.update(Message::Engine(EngineEvent::Finished(Box::new(out))));
        assert_eq!(app.reg_state, RegState::Done, "回音进结果态");
        let s2 = crate::testutil::frame_display(&app, 66, 50);
        assert!(s2.contains("catalog.json"), "写盘位置可见:{s2}");
        assert!(
            s2.contains("https://github.com/owner/tiny-skills"),
            "记录链接行可见:{s2}"
        );
    }

    // ---- #40 外部源漂移:recheck 只读入健康屏 ----

    fn recheck_app() -> App {
        let mut app = fake_app();
        app.screen = Screen::Health;
        app.hl_mode = HealthMode::Recheck;
        app.ls = Some(
            serde_json::from_str::<LsData>(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[{"name":"web-search","kind":"skill","entries":[{"name":"a","path":"skills/a"}]},{"name":"theme-pack","kind":"extension","entries":[{"name":"t","path":"."}]}],"profiles":[],"locks":[]}"#,
            )
            .unwrap(),
        );
        app.capabilities
            .record(Capability::SourceRecheck, capability::Probe::Available);
        app
    }

    #[test]
    fn m_在三模式间轮转_漂移与doctor不互冒充() {
        let mut app = recheck_app();
        assert_eq!(app.hl_mode, HealthMode::Recheck);
        app.update(Message::Key(key('m')));
        assert_eq!(app.hl_mode, HealthMode::Doctor);
        app.update(Message::Key(key('m')));
        assert_eq!(app.hl_mode, HealthMode::Validate);
        app.update(Message::Key(key('m')));
        assert_eq!(app.hl_mode, HealthMode::Recheck, "轮转回漂移模式");
    }

    #[test]
    fn recheck_全查与按名限定_argv都不带任何写旗标() {
        let mut app = recheck_app();
        app.run_recheck(None);
        assert_eq!(
            app.queue.last().unwrap(),
            &vec![
                "source".to_string(),
                "recheck".into(),
                "--root".into(),
                std::env::temp_dir().display().to_string()
            ],
            "全查:不带 --name"
        );
        app.run_recheck(Some("web-search".into()));
        let a = app.queue.last().unwrap();
        assert_eq!(a[2], "--name");
        assert_eq!(a[3], "web-search");
        assert!(
            !a.iter().any(|x| x == "--yes"),
            "只读铁律:recheck 永不带 --yes(写入口只有 register 一个)"
        );
    }

    #[test]
    fn recheck_三种缺前提都不下发命令_各有可区分指路() {
        // 未设数据仓根
        let mut app = recheck_app();
        app.data_root = None;
        let n = app.queue.len();
        app.run_recheck(None);
        assert_eq!(app.queue.len(), n, "未设根不下发");
        assert!(app.log[0].contains("数据仓根未设"), "{:?}", app.log);
        // 引擎无 source 能力
        let mut app = recheck_app();
        app.capabilities
            .record(Capability::SourceRecheck, capability::Probe::Unavailable);
        let n = app.queue.len();
        app.run_recheck(None);
        assert_eq!(app.queue.len(), n, "缺能力不下发");
        assert!(app.log[0].contains("不是面板坏了"), "{:?}", app.log);
        // 编目为空
        let mut app = recheck_app();
        app.ls = Some(
            serde_json::from_str::<LsData>(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#,
            )
            .unwrap(),
        );
        let n = app.queue.len();
        app.run_recheck(None);
        assert_eq!(app.queue.len(), n, "空编目不下发");
        assert!(app.log[0].contains("没有已登记外部源"), "{:?}", app.log);
    }

    #[test]
    fn recheck_清单空的两种原因各说各话_互不冒充() {
        // 四种缺前提里的第四种边缘态:ls 还没回音 ≠ 编目真为空。
        let mut app = recheck_app();
        app.ls = None;
        assert_eq!(app.recheck_empty_cause(), Some(RecheckEmpty::NotLoaded));
        let n = app.queue.len();
        app.run_recheck(None);
        assert_eq!(app.queue.len(), n, "还没读到不下发");
        assert!(
            app.log[0].contains("五域还没读到"),
            "没读到说没读到:{:?}",
            app.log[0]
        );
        assert!(
            !app.log[0].contains("没有已登记外部源"),
            "没读到不冒充空编目:{:?}",
            app.log[0]
        );
        // ls 回音了而外部源域确为空:另一个说法,可行动指路不变
        let mut app = recheck_app();
        app.ls = Some(
            serde_json::from_str::<LsData>(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}"#,
            )
            .unwrap(),
        );
        assert_eq!(app.recheck_empty_cause(), Some(RecheckEmpty::NoRecords));
        app.run_recheck(None);
        assert!(
            app.log[0].contains("没有已登记外部源"),
            "空编目说空编目:{:?}",
            app.log[0]
        );
        assert!(
            app.log[0].contains("按 r 登记"),
            "空编目给可行动指路:{:?}",
            app.log[0]
        );
        assert!(
            !app.log[0].contains("五域还没读到"),
            "空编目不冒充没读到:{:?}",
            app.log[0]
        );
        // 清单非空:门放行,不产原因
        let app = recheck_app();
        assert_eq!(app.recheck_empty_cause(), None, "有记录时门放行");
    }

    #[tokio::test]
    async fn recheck_定型链只有一处_查过给数据没查给空() {
        // recheck_state_of / recheck_banner / drift_lines 三处共用
        // App::recheck_data 这一条链, 行为不变:没查 / 不可定型
        // (含契约守卫「需升级 nw」)一律 None,不冒充上次结论。
        let mut app = recheck_app();
        assert!(app.recheck_data().is_none(), "还没查 = None");
        app.run_recheck(None);
        let json = r#"{"schema":"nw/source-recheck.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/data/repo/catalog.json","checked":[{"name":"web-search","kind":"skill","repo":"o/r","commit":"abc","entries":1}],"drift":[]}}"#;
        recheck_feed(&mut app, json, 0).await;
        let d = app.recheck_data().expect("回音可定型 = Some");
        assert_eq!(d.checked.len(), 1, "三处呈现看到的是同一份数据");
        // 换数据仓根作废陈旧结论：链随之归 None,不拿旧根的结论冒充
        let (_g, _cfg) = env_locked();
        app.apply_data_root(_cfg.dir().display().to_string());
        assert!(app.recheck_data().is_none(), "换根后链归 None");
    }

    /// 经 FakeRunner 喂一次 recheck 回音(与既有 reg_feed 同款:走解析点,
    /// 契约守卫才会合证;直接拼 EngineOutput 会绕过它)。
    async fn recheck_feed(app: &mut App, json: &str, rc: i32) {
        let argv = app.queue.last().expect("队尾有 recheck argv").clone();
        let runner = FakeRunner::new(&[("source recheck", rc, json)]);
        let o = runner.run(&argv).await;
        app.update(Message::Engine(EngineEvent::Finished(Box::new(o))));
    }

    #[tokio::test]
    async fn recheck_回音入状态树_只读不占装配互斥() {
        let mut app = recheck_app();
        app.run_recheck(None);
        let json = r#"{"schema":"nw/source-recheck.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"漂移 1 处","hint":"重跑 register","details":{}}],"warnings":[],"data":{"catalogPath":"/data/repo/catalog.json","checked":[{"name":"web-search","kind":"skill","repo":"o/r","commit":"abc","entries":1},{"name":"theme-pack","kind":"extension","repo":"o/t","commit":"def","entries":1}],"drift":[{"source":"web-search","kind":"added","entry":"x","path":"p/x"}]}}"#;
        recheck_feed(&mut app, json, 3).await;
        assert!(app.hl_recheck.is_some(), "本次结论入状态树");
        assert!(!app.assembling, "只读不点装配互斥灯");
        assert!(
            app.log.iter().any(|l| l.contains("只读,未写任何文件")),
            "日志明示只读:{:?}",
            app.log
        );
    }

    #[tokio::test]
    async fn recheck_换数据仓根即作废陈旧结论_不冒充新根的() {
        let mut app = recheck_app();
        app.run_recheck(None);
        let json = r#"{"schema":"nw/source-recheck.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/data/repo/catalog.json","checked":[{"name":"web-search","kind":"skill","repo":"o/r","commit":"abc","entries":1}],"drift":[]}}"#;
        recheck_feed(&mut app, json, 0).await;
        assert!(app.hl_recheck.is_some());
        let (_g, _cfg) = env_locked();
        app.apply_data_root(_cfg.dir().display().to_string());
        assert!(app.hl_recheck.is_none(), "换根作废旧结论(无缓存语义)");
        assert!(
            app.log.iter().any(|l| l.contains("漂移结论已作废")),
            "作废留证据:{:?}",
            app.log
        );
    }

    #[test]
    fn recheck_键位_r全查_enter只查选中_record条目不误发() {
        let mut app = recheck_app();
        // Enter 只查选中这一条
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Down,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.hl_sel, 1, "↓ 移到第二条记录");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let a = app.queue.last().unwrap();
        assert_eq!(a[3], "theme-pack", "Enter 只查选中记录:{a:?}");
        // r 全查
        app.update(Message::Key(key('r')));
        let a = app.queue.last().unwrap();
        assert!(!a.iter().any(|x| x == "--name"), "r 全查:{a:?}");
    }

    #[test]
    fn recheck_鼠标点清单行选中_与上下键同路_点空白不误发() {
        let mut app = recheck_app();
        app.size = Rect::new(0, 0, 80, 30);
        let row = hit_target(&app, |t| matches!(t, Target::SelectRow(1)));
        let qlen = app.queue.len();
        click(&mut app, row);
        assert_eq!(app.hl_sel, 1, "点清单行选中");
        assert_eq!(app.queue.len(), qlen, "点行只选中,不下发命令");
        // 三道门任一不过:清单不产命中区(整栏是指路)
        let mut gated = recheck_app();
        gated.size = Rect::new(0, 0, 80, 30);
        gated.data_root = None;
        let hits = hitmap(gated.size, &gated, size_class(gated.size));
        assert!(
            hits.iter().all(|(_, t)| *t != Target::SelectRow(1)),
            "未设根时没有可选中行(指路屏)"
        );
    }

    // ---- #62 帮助浮层↔实现 双向对齐(纯文档,零行为变更) ----

    /// 可观测状态快照:帮助对照测试据此判断「这个键按下去到底动没动」。
    /// 覆盖任何键位可能改到的字段(屏 / 各屏选中 / 模式 / 域 / 浮层 / 退出 /
    /// 手填 / 编辑器 / 鼠标 / NW_HOME / 日志长度 / 队列长度)。
    #[derive(PartialEq, Debug)]
    struct KeySnap {
        screen: Screen,
        catalog_kind: crate::screens::CatalogKind,
        ov_sel: usize,
        as_sel: usize,
        as_confirm: bool,
        hl_mode: HealthMode,
        hl_sel: usize,
        rc_domain: Domain,
        rc_sel: usize,
        rc_bake_sel: bool,
        reg_state: RegState,
        help: bool,
        quit: bool,
        home_input: bool,
        data_input: bool,
        pending_editor: bool,
        mouse_enabled: bool,
        nw_home: String,
        log_len: usize,
        queue_len: usize,
    }

    fn key_snap(a: &App) -> KeySnap {
        KeySnap {
            screen: a.screen,
            ov_sel: a.ov_sel,
            as_sel: a.as_sel,
            as_confirm: a.as_confirm,
            hl_mode: a.hl_mode,
            hl_sel: a.hl_sel,
            rc_domain: a.rc_domain,
            rc_sel: a.rc_sel,
            rc_bake_sel: a.rc_bake_sel,
            reg_state: a.reg_state,
            catalog_kind: a.catalog_kind,
            help: a.help,
            quit: a.quit,
            home_input: a.home_input,
            data_input: a.data_input,
            pending_editor: a.pending_editor,
            mouse_enabled: a.mouse_enabled,
            nw_home: a.nw_home.clone(),
            log_len: a.log.len(),
            queue_len: a.queue.len(),
        }
    }

    /// 一次性临时目录(数据仓根要真实可读,闸才放行;进程级唯一,建好即用)。
    fn help_env_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nwt-help-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    /// 帮助对照测试的就绪态:数据仓根指向真实可读临时目录(三道闸放行)、
    /// 五项能力全 Available、五域夹具各备 ≥2 行——让每个键位都能触发
    /// 可观测变化(派发只入队,不真跑子进程)。`dir` 由调用方建好并持有。
    fn help_ready_app(dir: &str) -> App {
        let mut app = fake_app();
        app.splash = false;
        app.data_root = Some(dir.to_string());
        for cap in Capability::ALL {
            app.capabilities.record(cap, Probe::Available);
        }
        app.ls = Some(
            serde_json::from_str(
                r#"{"domain":"all","recipes":[{"name":"research-desk","version":"0.2.0"},{"name":"lab-desk","version":"0.1.0"}],"components":[],"sources":[{"name":"tiny-skills"},{"name":"more-skills"}],"profiles":[{"name":"desk","state":"drift"},{"name":"lab","state":"ok"}],"locks":[{"lockId":"9f8e","recipe":"research-desk"},{"lockId":"a1b2","recipe":"lab-desk"}]}"#,
            )
            .unwrap(),
        );
        app
    }

    /// 正向守「帮助列出的每个键,实现里都有真实分支」:遍历 `HELP_ROWS`,
    /// 按行把 App 布到对应上下文,派发代表性键码,断言可观测状态确曾改变
    /// (帮助不写幽灵键)。增删键位只改 `HELP_ROWS` 一处即双向生效。
    #[test]
    fn 帮助列出的每个键_实现里都有真实分支() {
        use crate::ui::{Prep, HELP_ROWS};
        let (_g, _cfg) = env_locked();
        let dir = help_env_dir();
        let dir = dir.to_str().unwrap().to_string();
        for row in HELP_ROWS {
            let mut app = help_ready_app(&dir);
            if let Some(s) = row.screen {
                app.screen = s;
            }
            if let Some(d) = row.domain {
                app.rc_domain = d;
            }
            if let Some(m) = row.mode {
                app.hl_mode = m;
            }
            if let Some(b) = row.book {
                app.catalog_kind = b;
            }
            if row.running {
                // 中止键的受理面前提:有 up 在途
                app.assembling = true;
                app.as_running = true;
            }
            if row.prep == Prep::HelpOpen {
                app.help = true;
            }
            let before = key_snap(&app);
            app.update(Message::Key(KeyEvent::new(row.code, KeyModifiers::NONE)));
            assert_ne!(
                key_snap(&app),
                before,
                "帮助列的键 `{}`({})在实现里应有真实分支",
                row.key,
                row.desc
            );
        }
    }

    /// 守「HELP_ROWS 单一真相源真的渲染进了浮层」:逐行断言键名与说明都在
    /// 渲染文本里,鼠标章节每一行也都在——渲染与对照表不脱节。
    #[test]
    fn 帮助浮层渲染出对照表每一行() {
        use crate::ui::{HELP_ROWS, MOUSE_LINES};
        let mut app = fake_app();
        app.splash = false;
        app.help = true;
        // 正文开窗(#53):80×30 最小常畅态内层只有 24 行, HELP_ROWS + 鼠标章
        // + 尾注已超过它——所以这条测试**滚完整个浮层**再断言,顺带守「没有
        // 一行是因为放不下被静默丢掉的」。滚到底仍看不到的行 = 真丢了。
        let mut seen = String::new();
        for _ in 0..HELP_ROWS.len() + MOUSE_LINES.len() + 4 {
            seen.push_str(&crate::testutil::frame_display(&app, 80, 44));
            seen.push('\n');
            app.update(Message::Key(KeyEvent::new(
                KeyCode::Down,
                KeyModifiers::NONE,
            )));
        }
        for row in HELP_ROWS {
            assert!(seen.contains(row.key), "帮助应显示键位 `{}`", row.key);
            assert!(seen.contains(row.desc), "帮助应显示说明 `{}`", row.desc);
        }
        for m in MOUSE_LINES {
            assert!(seen.contains(m), "鼠标章节应含 `{m}`");
        }
    }

    /// 帮助正文开窗(#53):超量时 ↑/↓ 在浮层内滚动,钳在 [0, 行数-可见行数],
    /// 且给出「还有下文/上文」的可见提示——不静默截断。
    #[test]
    fn 帮助浮层_超量时上下滚动不越界_末行给还有内容的提示() {
        let mut app = fake_app();
        app.splash = false;
        app.help = true;
        // 80×30:内层 24 行,可见 23 行(末行留给提示),正文 30 行 → 需滚动
        let first = crate::testutil::frame_display(&app, 80, 30);
        assert!(first.contains("↓ 还有下文"), "首屏应提示还有下文:{first}");
        assert!(!first.contains("↑ 还有上文"), "首屏不该提示还有上文");
        // 一路向下:不越界(偏移钳住),且提示翻面
        for _ in 0..40 {
            app.update(Message::Key(KeyEvent::new(
                KeyCode::Down,
                KeyModifiers::NONE,
            )));
        }
        let last = crate::testutil::frame_display(&app, 80, 30);
        assert!(last.contains("↑ 还有上文"), "滚到底应提示还有上文:{last}");
        assert!(!last.contains("↓ 还有下文"), "滚到底不该再提示还有下文");
        // 回到顶部:提示翻回来
        for _ in 0..40 {
            app.update(Message::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)));
        }
        let back = crate::testutil::frame_display(&app, 80, 30);
        assert!(back.contains("↓ 还有下文"), "回顶应再次提示还有下文:{back}");
        // 关浮层即归零:重开总是从首行看起
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(!app.help);
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('?'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.help_offset, 0, "重开归零");
    }

    /// 反向守「实现里每个屏的分支键,帮助都写了」:对每屏盲按 a-z + ↑/↓/Enter/Esc,
    /// 凡使状态改变者即「实现里有分支」,断言帮助对照表必有兼容行覆盖它。
    /// 概览 `n/p` 是 ↑/↓ 的别名(帮助只呈现 ↑/↓,同 README 粒度),显式排除免误报。
    /// 新增实现键若忘加进 HELP_ROWS,此测试当场红。
    #[test]
    fn 实现每个屏的按键_帮助都写了() {
        use crate::ui::HELP_ROWS;
        let (_g, _cfg) = env_locked();
        let dir = help_env_dir();
        let dir = dir.to_str().unwrap().to_string();
        let aliases = [
            (Screen::Overview, KeyCode::Char('n')),
            (Screen::Overview, KeyCode::Char('p')),
        ];
        let mut candidates: Vec<KeyCode> =
            (b'a'..=b'z').map(|b| KeyCode::Char(b as char)).collect();
        candidates.extend([KeyCode::Up, KeyCode::Down, KeyCode::Enter, KeyCode::Esc]);
        for screen in Screen::ALL {
            for code in &candidates {
                if aliases.contains(&(screen, *code)) {
                    continue;
                }
                let mut app = help_ready_app(&dir);
                app.screen = screen;
                let before = key_snap(&app);
                app.update(Message::Key(KeyEvent::new(*code, KeyModifiers::NONE)));
                if key_snap(&app) == before {
                    continue; // 该屏此键无分支
                }
                let token = match code {
                    KeyCode::Char(c) => c.to_string(),
                    KeyCode::Up => "↑".to_string(),
                    KeyCode::Down => "↓".to_string(),
                    KeyCode::Enter => "Enter".to_string(),
                    KeyCode::Esc => "Esc".to_string(),
                    _ => unreachable!(),
                };
                let documented = HELP_ROWS.iter().any(|r| {
                    (r.screen == Some(screen) || r.screen.is_none())
                        && r.key
                            .split(['/', ' '])
                            .filter(|t| !t.is_empty())
                            .any(|t| t == token)
                });
                assert!(
                    documented,
                    "屏 {screen:?} 的键 `{token}` 在实现里有分支,但帮助对照表没写(补 HELP_ROWS)"
                );
            }
        }
    }

    /// 鼠标章节覆盖 README「鼠标(默认开捕获)」已写全部条目:点导航/Tab 切屏与
    /// 点行选中、滚轮两用、发射仪式两段、浮层吞点击、Shift 放行复制、
    /// `--no-mouse` 与设置屏 `m` 两逃逸——每条都有短语在场。
    #[test]
    fn 帮助鼠标章节_覆盖_readme_全部条目() {
        use crate::ui::MOUSE_LINES;
        let joined = MOUSE_LINES.join("\n");
        for phrase in [
            "切屏",
            "选中",
            "滚轮",
            "装配",
            "发射",
            "返航",
            "浮层",
            "不穿透",
            "Shift",
            "--no-mouse",
            "设置屏 m",
        ] {
            assert!(joined.contains(phrase), "鼠标章节缺 README 条目 `{phrase}`");
        }
    }
}
