//! 应用外壳:Elm 式集中状态 + update + 屏幕定义。
//!
//! V1 只有一棵状态树;所有交互先变成 [`Message`],由 [`App::update`]
//! 收口,屏幕渲染函数无副作用。引擎调用只能经 [`App::spawn_engine`]。

use std::sync::Arc;
use tokio::sync::mpsc;

use crate::engine::{
    capability::{self, Capability, Probe},
    commands, CommandRunner, EngineEvent, EngineOutput, EngineStatus, Gate, LsData, State,
};
use crate::input::{self, InputAction};
use crate::layout::{hitmap, size_class, ScrollDir, Target};
use crate::release::{upgrade_hint, verdict, ReleaseLookup, ReleaseProbe};
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
    /// 第一本编目册;#54 起屏内可换册(机器 / 标签 / 订阅 / 钥匙,`m` 键)
    /// ——四本册只换数据源与登记规格,不换屏与骨架(`screens::catalog`)。
    /// 名字留在 `Vps` 是公开 API 形状(#53 已合入,改名收益不抵风险),
    /// 册内维度一律走 `catalog_kind`。
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

/// 设置屏的屏内页(spec #94 工单 #108,照 [`HealthMode`] 的屏内多模样板)。
///
/// 为什么不把版本对照并进常驻面板的那棵扁平行向量:设置屏的竖向预算已被
/// 110×30 内层 26 行钉死(`screens/settings.rs` 能力节注释记着「既有 21 行
/// 之外只许 +3」),而常驻面板在无 config 告警时正好用满 26 行——再加一行
/// 就会把末行的「config 状态」静默挤出手屏。新内容另起一页,常驻面板
/// **逐字节不动**。后续工单(多派总览)往这个枚举加页即可。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsPage {
    /// 常驻面板(引擎 / 能力 / 环境 / 外观 / 配置)。
    Panel,
    /// 引擎版本对照页:本地 `nw` 版本 ↔ 引擎 release 仓最新 tag。
    Version,
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
///
/// [`Message::Release`] 是引擎版本对照(#108)的回音:它**不借道**
/// [`Message::Engine`]——那不是 `nw` 的进程输出,塞进引擎通道会让「面板
/// 只跟 nw 说话」这条唯一缝说谎。同形(异步、不阻塞渲染)而不同路。
#[derive(Clone, Debug)]
pub enum Message {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Engine(EngineEvent),
    /// 引擎 release 仓最新 tag 的对照结论(#108)。
    Release(ReleaseProbe),
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
    /// 设置屏当前翻开哪一页(#108)。
    pub settings_page: SettingsPage,
    /// 引擎版本对照的**远端侧**结论(#108):`None` = 还没对照过,或正在对照中
    /// (见 `release_probing`)。判词不落状态——渲染时按本地版本 + 远端结论现算,
    /// 免得本地版本后到的时候屏幕上留着一句已经不算数的话。
    pub release_remote: Option<ReleaseProbe>,
    /// 有一次远端对照在途(#108):在途不重复发起(spec #94 实施决策:远端对照
    /// **不轮询**,定时轮询明确划在范围外)。
    pub release_probing: bool,
    /// 远端对照的注入缝(测试换 `release::FakeReleaseLookup`,零网络)。
    pub release_lookup: Arc<dyn ReleaseLookup>,
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
    /// 装配历史次级视图在场(#101):装配屏内看各派清单存档,只读。
    /// 装配在途时不受理(在途那屏归七步与中止键),up 下发即清。
    pub as_history: bool,
    /// 一次读盘的存档快照(#101):进视图 / 按 `r` 时经
    /// [`crate::datarepo`] 读,`None` = 还没读过。**快照语义**——不轮询、
    /// 不自动刷,要新的按 `r`。
    pub as_history_rows: Option<Vec<crate::screens::HistoryRow>>,
    pub as_history_sel: usize,
    /// 装配历史的本地过滤词(#101,与 #61 五域 / #69 册内过滤同一形态:
    /// 纯本地筛选,零引擎调用、不入队)。
    pub as_history_filter: String,
    pub as_history_filter_on: bool,
    /// 清单原文浮层(#101,规格故事 3):回车看选中派那份 `.nw-manifest`
    /// 的原文。`None` = 不在场。
    pub as_history_raw: Option<String>,
    /// 发射仪式翻到哪一页(#104):简报页(默认)/ 新旧对比页。仪式只有两页,
    /// `Enter` 发射 / `Esc` 返航 在两页同一语义(规格故事 10)。
    pub as_ritual_page: crate::screens::RitualPage,
    /// 新旧对比页的快照(#104):翻到该页时经 [`crate::datarepo`] 读一次,
    /// `None` = 还没读过。**快照语义**——与 #101 历史视图同口径,不轮询。
    pub as_ritual_diff: Option<crate::screens::RitualDiff>,

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
    /// 最近一次反查问的是哪一条(**由下发时记下**:`run_tag_ls` 的入参,
    /// `None` = 列全部)。只为把「列全部 / 只列一个」这档差别显式画出来——
    /// 操作者看得见自己问的是什么,不靠记性。回音路由
    /// ([`CatalogKind::absorb_ls`](crate::screens::CatalogKind::absorb_ls) 的标签册臂)
    /// 只把它读回来,不再从 argv 重新推导:argv 里
    /// `--root` 的值长得跟标签名一样,推导就会把「列全部」冒充成「反查
    /// 一个叫 <根> 的标签」。
    pub tag_lookup: Option<String>,
    /// 反查上浮状态(#70):机器册上一个键问「什么标签引用了这一条」。
    /// **独立小状态,不并入 `assembling` 写互斥**(反查只读);三态两两可
    /// 区分,见 [`crate::screens::catalog::VpsReverse`]。
    pub vps_reverse: crate::screens::catalog::VpsReverse,
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
    /// 编目册册内过滤词(#69),**按册各留一份**:`[String; 4]` 以
    /// [`CatalogKind::idx`](crate::screens::CatalogKind::idx) 为下标,换册时每本
    /// 各留各的词(规格故事 24:机器册筛「gate」、切到钥匙册筛「deploy」,回头
    /// 机器册还在「gate」上)。与 #61 五域 `rc_filter` 同源——**纯本地筛选**,
    /// 只作用于面板已拿到的投影,不新增引擎调用、不改数据来源、不把关键词
    /// 带进任何一条 argv。空串 = 不过滤(与过滤前逐字节相同)。
    pub catalog_filter: [String; 4],
    /// 编目册过滤输入态(#69,键 `/`):与 `rc_filter_on` 同一级顶层输入态
    /// (优先于册内键位,但只在编目屏在场时吞键)。键入即筛;`Enter` 收词退
    /// 输入态、`Esc` 清词。切屏即退输入态但留词(后果链与 #61 一字不差)。
    pub catalog_filter_on: bool,

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
            settings_page: SettingsPage::Panel,
            release_remote: None,
            release_probing: false,
            release_lookup: Arc::new(crate::release::GhReleaseLookup),
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
            as_history: false,
            as_history_rows: None,
            as_history_sel: 0,
            as_history_filter: String::new(),
            as_history_filter_on: false,
            as_history_raw: None,
            as_ritual_page: crate::screens::RitualPage::Brief,
            as_ritual_diff: None,
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
            vps_reverse: crate::screens::catalog::VpsReverse::Idle,
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
            catalog_filter: [String::new(), String::new(), String::new(), String::new()],
            catalog_filter_on: false,
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

    /// **打开设置屏才对照一次**(spec #94 实施决策:远端对照不轮询,定时轮询
    /// 明确划在范围外)。结论一到就锁 latch,不再发起;操作者要重查按设置屏
    /// `r`(清结论,下一帧自然再发起)。
    ///
    /// 发起走 `tokio::spawn` + 独立 channel,回音经 [`Message::Release`] 落账:
    /// 与引擎调用同形(异步、不阻塞渲染),但不借道 [`App::spawn_engine`]——
    /// 那不是 `nw` 命令,塞进引擎队列会让「面板只跟 nw 说话」这条缝说谎。
    /// 主循环每帧调它,闸内自行判断要不要发起(幂等)。
    pub fn ensure_release_probe(&mut self, tx: &mpsc::UnboundedSender<ReleaseProbe>) {
        if self.screen != Screen::Settings
            || self.release_probing
            || self.release_remote.is_some()
            || !self.engine.found
        {
            return;
        }
        self.start_release_probe(tx);
    }

    /// 设置屏 `r`:显式重查(与健康屏 `r` 复核同一语气)。
    ///
    /// 只清结论、不自己发起:键位处理器没有 channel(同 `queue` 的纪律),
    /// 下一帧的 [`App::ensure_release_probe`] 看到「暂无结论」自然会再问一次。
    pub fn reprobe_release(&mut self) {
        if self.release_probing {
            return; // 在途那次问完自然落账,不叠一次
        }
        self.release_remote = None;
        self.push_log("重查引擎 release 仓…");
    }

    fn start_release_probe(&mut self, tx: &mpsc::UnboundedSender<ReleaseProbe>) {
        self.release_probing = true;
        let lookup = Arc::clone(&self.release_lookup);
        let tx = tx.clone();
        tokio::spawn(async move {
            let probe = lookup.latest_tag().await;
            // 收端没了(面板正在退)就当没问过:不回写、不告警。
            let _ = tx.send(probe);
        });
    }

    /// 远端对照结论落账(#108):只写状态与日志,**绝不**据此下发任何命令
    /// (ADR-0003:建议命令只给文案,执行权留给操作者)。
    fn on_release_probe(&mut self, probe: ReleaseProbe) {
        self.release_probing = false;
        self.release_remote = Some(probe.clone());
        match &probe {
            ReleaseProbe::Latest(tag) => {
                self.push_log(format!("引擎 release 仓最新 tag:{tag}"));
            }
            // 两种「取不到」理由分开说:操作者要知道是该去装 gh / 联网,
            // 还是上游这次的发布形态不对劲。都不报错、都不猜(spec 用户故事 25)。
            ReleaseProbe::Garbage => {
                self.push_log("引擎 release 仓回话不是可用 tag:只显示本地版本,不猜".to_string())
            }
            ReleaseProbe::Unreachable => self.push_log(
                "取不到引擎 release 仓(gh 不在 PATH 或没网):只显示本地版本,不报错".to_string(),
            ),
        }
        self.note_release_verdict();
    }

    /// 落后时把升级指路写进日志区(#108,ADR-0003):建议命令只给文案,面板不代跑。
    ///
    /// 两个落账点都调它——本地版本与远端 tag 谁后到,这句指路都不该漏
    /// (版本常比 tag 先到,也可能反过来)。
    fn note_release_verdict(&mut self) {
        let Some(ReleaseProbe::Latest(tag)) = self.release_remote.clone() else {
            return; // 没拿到远端就无从判落后
        };
        if verdict(&self.engine_version, Some(&tag)) == crate::release::Verdict::Behind {
            self.push_log(format!(
                "引擎落后:升级指路 `{}`(不代跑,升不升你定)",
                upgrade_hint()
            ));
        }
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
                                         // 装配历史是闲时的只读浏览视图(#101):一启动就让位给七步与
                                         // stderr 日志,回音落地时屏幕已经在运行态,不搞「装配中还在
                                         // 翻存档」这种半屏两义。
                self.as_history = false;
                self.as_history_raw = None;
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
            // 本身就是写命令,点亮写互斥;rm 同 vps rm(面板确认后在下发处
            // 点灯,回音到达时灭,ADR-0009 §4)。
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
            Message::Release(p) => self.on_release_probe(p),
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
                self.catalog_filter_on = false; // 编目册过滤同理(#69)
                self.as_history_filter_on = false; // 装配历史过滤同理(#101)
                self.as_history_raw = None; // 原文浮层不跨屏(#101)
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
                    // 装配屏两副列表(#101):锁定集选择器与装配历史各一个
                    // 选中槽,点行按当前在场的那一个走——不共用一个槽,
                    // 两个视图的选中互不串味。
                    Screen::Assemble => {
                        if self.as_history {
                            let n = self.history_visible().len();
                            self.as_history_sel = i.min(n.saturating_sub(1));
                        } else {
                            self.as_sel = i;
                        }
                    }
                    Screen::Health => self.hl_sel = i,
                    Screen::Recipes => self.rc_sel = i,
                    // 编目册:点行选中按当前是哪一本走(#54 起四本册各有
                    // 自己的选中态,catalog_sel_mut 单一查表,换册不串味)
                    Screen::Vps => {
                        let (sel, n) = self.catalog_sel_mut();
                        *sel = i.min(n.saturating_sub(1));
                    }
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
            // 点正文键位行的「换册」段(#67):与键盘 `m` 完全同路——四本编目册
            // 按 CatalogKind::next() 轮转。鼠标只是叠加,不新增语义。
            Target::SwitchBook => self.cycle_catalog_kind(),
            // 点正文键位行的「中止」段(#71):与键盘 `x` 完全同路——进程级真止在途
            // nw up 子进程,回选择器态,stderr 日志一行不清(不宣布成功也不假装失败)。
            // 命中区只在真有 up 在途(as_running)时由 assemble_hits 产出,故此臂
            // 到达即确有进程可止;滚轮分支不受理它(滚过中止段不点火)。
            Target::Abort => self.abort_assembling(),
            // 点正文键位行的「Enter 反查」段(#70):与机器册选中行上按 `Enter`
            // 完全同路——只读地问「什么标签引用了这一条」。段只在机器册、且册体
            // 有条目(= 选中了一行)时由 catalog_hits 产出,故到达即确有一台机器
            // 可问;滚轮分支不受理它(滚过反查段不发起调用)。
            Target::VpsReverse => self.vps_reverse_lookup(),
            // 点正文键位行的「过滤」段(#69):与键盘 `/` 完全同路——进册内过滤
            // 输入态。鼠标只是叠加,不新增语义。
            Target::CatalogFilter => self.catalog_filter_on = true,
            // 点正文键位行的「g 取回」段(#73):与键盘 `g` 同一个函数、同一条
            // 后果链——当前册选中订阅 / 钥匙的建议命令收进日志,面板不代跑 get、
            // 不入队、不碰剪贴板(ADR-0009 §1)。路由只有这一处,且分叉的是「哪个
            // 册」而不是「哪条路径」:查 `CatalogKind::action` 同一张表(与
            // `key_catalog` 同一个),所以鼠标与键盘不可能给出两种后果。(过滤输入态
            // 里键入归输入态、鼠标照旧点得动,那是键位行所有可点段的共同边界。)
            Target::CatalogGet => self.catalog_get(),
            // 点正文键位行的「d 删除」段(#78,兑 #75 裁决 A):与键盘 `d` 同一个
            // 函数、同一条后果链——选中行 → ADR-0009 图形确认闸 → 确认才 `rm`
            // (标签册的删除嵌套在表单流里,点它进同一条流)。路由查
            // `CatalogKind::action` 同一张表(与 `key_catalog` 受理 `d` 同一个
            // 入口),落到 `App::start` 的 `StartMode::Rm`(与 #76 收敛后的键盘
            // 路径逐字同一处),所以鼠标与键盘不可能给出两种后果;无选中行 /
            // 入口门不过 / 装配在途时,复现键盘 `d` 的既有日志,不新造行为。
            Target::CatalogDelete => self.catalog_delete(),
            // 点概览警示带上的订阅到期条目(#94 工单 #105,用户故事 12):跳到
            // 订阅册并选中那一条。与键盘 `v` 到编目屏同族的三步(定册内维度 →
            // 切屏 → 落册级选中槽),选中落在**可见行**上(#69 册内过滤);载荷是
            // 投影下标,与 `SelectRow` 的可见行下标是两回事,故单立变体。
            // 零引擎调用:回音是既有的 `sub_ls`,面板只换视图。
            Target::AlertSubEntry(idx) => self.open_subscription_book(idx),
        }
    }

    /// 滚轮在可选列表上移选中(#21):上下各一档,与键盘同步进;
    /// 边界按票面钉死「到顶/到底不越界、不绕回」——到顶停顶、到底停底,
    /// 落在 [0, n) (既有键盘代码走 % n 会绕回,那是键盘行为,
    /// 键盘零回归不动它)。
    fn wheel_sel(&mut self, dir: ScrollDir) {
        let screen = self.screen;
        let (sel, n) = match screen {
            Screen::Overview => {
                let n = self.ls.as_ref().map(|l| l.profiles.len()).unwrap_or(0);
                (&mut self.ov_sel, n)
            }
            Screen::Assemble => {
                // 装配历史次级视图在场时(#101),滚轮移的是历史视图的选中
                // (可见行数按过滤后的算);否则照旧移锁定集选择器。
                if self.as_history {
                    let n = self.history_visible().len();
                    (&mut self.as_history_sel, n)
                } else {
                    let n = self.ls.as_ref().map(|l| l.locks.len()).unwrap_or(0);
                    (&mut self.as_sel, n)
                }
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
            // 编目册选中行按当前是哪一本走(#54 起四本册各有自己的投影
            // 与选中态,catalog_sel_mut 单一查表,换册不串味)。
            Screen::Vps => self.catalog_sel_mut(),
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
            self.as_ritual_diff = None;
            self.push_log("发射取消");
        } else if self.as_history_raw.is_some() {
            // 清单原文浮层(#101):只收状态,历史视图与选中行原样不动——
            // 看原文不写任何选中态(规格故事 3 是「看」,不是「挑」)。
            self.as_history_raw = None;
        } else if self.vps_reverse_open() {
            // 反查上浮结果浮层返航(#70):只收状态,册体一行不改、机器册原选中行
            // 原样在场——反查从头到尾不写 `vps_sel`(它在途时册体键位被吞、回音
            // 落地只按当前册钳位),所以「回到原选中行」不需要一次恢复动作,只
            // 需要不在返航时顺手改它。Esc 与点任意处走这一条同路(语义不漂移)。
            self.vps_reverse = crate::screens::catalog::VpsReverse::Idle;
        } else if self.reg_state != RegState::Idle {
            // 登记返航与 Esc 完全同路(鼠标点确认屏与键盘 Esc 一个后果)
            self.cancel_register();
        } else if self.vps_rm_confirm {
            self.dismiss_rm(crate::screens::CatalogKind::Vps);
        } else if self.tag_rm_confirm {
            self.dismiss_rm(crate::screens::CatalogKind::Tag);
        } else if self.vps_flow.phase != catalog::RegPhase::Idle {
            // 编目册登记返航与 Esc 完全同路(骨架件同一语义)
            self.cancel(crate::screens::CatalogKind::Vps);
        } else if self.tag_flow.phase != catalog::RegPhase::Idle {
            self.cancel(crate::screens::CatalogKind::Tag);
        } else if self.sub_rm_confirm {
            self.dismiss_rm(crate::screens::CatalogKind::Subscription);
        } else if self.sub_flow.phase != catalog::RegPhase::Idle {
            // 编目册登记返航与 Esc 完全同路(骨架件同一语义);在途链接本体
            // 随表单 drop(ADR-0009 §3:面板侧不留副本)。
            self.cancel(crate::screens::CatalogKind::Subscription);
        } else if self.key_rm_confirm {
            self.dismiss_rm(crate::screens::CatalogKind::Key);
        } else if self.key_flow.phase != catalog::RegPhase::Idle {
            // 编目册登记返航与 Esc 完全同路(骨架件同一语义);在途钥匙本体
            // 随表单 drop(ADR-0009 §3:面板侧不留副本)。
            self.cancel(crate::screens::CatalogKind::Key);
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
                // 版本后到也要补判一次:远端 tag 可能已经等在桌上了(#108)
                self.note_release_verdict();
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
            // 编目册回音(#53 / #54 / #56 / #57;#76 起四本收成一条):ls 只读 /
            // register 两段式 / add 无确认闸 / rm。命令组名查表落到
            // `CatalogKind`,`on_echo` 一份实现按子命令分发——只认真实 argv
            // (精确的无参组级探针仍归下面的能力回音路)。
            cmd if crate::screens::CatalogKind::from_command(cmd).is_some()
                && Capability::from_probe(&out.args).is_none() =>
            {
                let kind = crate::screens::CatalogKind::from_command(cmd)
                    .expect("guard ensured a catalog command group");
                self.on_echo(kind, out);
            }
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
        // 清单原文浮层(#101):与发射仪式同级的顶层浮层——它在场时吞键,
        // 只有 Enter / Esc / q 关(与键盘同一条后果链,鼠标点任意处也走
        // `dismiss_popup`)。关掉后历史视图原样在场:看原文从头到尾不写
        // 选中行。
        if self.as_history_raw.is_some() {
            if matches!(k.code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q')) {
                self.dismiss_popup();
            }
            return;
        }
        // 反查上浮结果浮层(#70):与帮助浮层同级的**顶层浮层**——它在场时吞键,
        // 只有 Esc 返航;册体键位(↑/↓ / r / d / l / Enter / m)一个都不受理,
        // 机器册原选中行因此原样不动(规格故事 37:返航后回到原选中行)。
        // 与 `dismiss_popup` 同路:鼠标点任意处与键盘 Esc 一个后果。
        if self.vps_reverse_open() {
            if matches!(k.code, KeyCode::Esc) {
                self.dismiss_popup();
            }
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
        // 编目册册内过滤输入态(#69):与 `rc_filter_on` 同一级顶层输入态
        // (优先于册内键位,但只在编目屏在场时吞键——切屏即退输入态,过滤词
        // 按册保留)。插在 vps_flow / rm_confirm 守卫**之前**:它是顶层浮层,
        // 与 `rc_filter` 同级,不与登记两段式混。
        if self.catalog_filter_on && self.screen == Screen::Vps {
            self.on_key_catalog_filter(k);
            return;
        }
        // 装配历史过滤输入态(#101):与 `rc_filter_on` / `catalog_filter_on`
        // 同一级顶层输入态(优先于屏键位,但只在本屏在场时吞键——切屏即退
        // 输入态,过滤词留着回屏还在)。
        if self.as_history_filter_on && self.screen == Screen::Assemble {
            self.on_key_history_filter(k);
            return;
        }
        // 编目册登记两段式(#53):与外部源登记同级的浮层优先级,
        // 骨架件同构(`screens::catalog::RegisterFlow`)。四本册各一道守卫,
        // 受理次序固定(与 `CatalogKind::overlay` 逐字对齐:哪一本的哪道浮层
        // 更里层)。**只转口不重排**(#76):每个叶子现在都走收敛后的那一份
        // `on_key_flow` / `on_key_rm_confirm`,按册差异在查表成员上,这里不再有
        // 四份逐字重复的处理器——但守卫本身仍按册的旗 / 槽判(不是按
        // `catalog_kind` 判),次序与优先级一字节不动。
        if self.vps_flow.phase != catalog::RegPhase::Idle {
            self.on_key_flow(crate::screens::CatalogKind::Vps, k);
            return;
        }
        if self.vps_rm_confirm {
            self.on_key_rm_confirm(crate::screens::CatalogKind::Vps, k);
            return;
        }
        // 标签册浮层(#54):骨架件同一个 `RegisterFlow`。**rm 确认是嵌套在
        // 表单流里的一道浮层**(表单 Enter 才出现),所以它比流程态更里层,
        // 优先受理——否则 Enter 会被表单再收一次,确认框永远点不着。
        if self.tag_rm_confirm {
            self.on_key_rm_confirm(crate::screens::CatalogKind::Tag, k);
            return;
        }
        if self.tag_flow.phase != catalog::RegPhase::Idle {
            self.on_key_flow(crate::screens::CatalogKind::Tag, k);
            return;
        }
        // 订阅册浮层(#56):同一个骨架件。删除确认是独立一道图形确认
        // (ADR-0009 §2),登记流里没有 rm 那一步,所以两道浮层平级即可。
        if self.sub_rm_confirm {
            self.on_key_rm_confirm(crate::screens::CatalogKind::Subscription, k);
            return;
        }
        if self.sub_flow.phase != catalog::RegPhase::Idle {
            self.on_key_flow(crate::screens::CatalogKind::Subscription, k);
            return;
        }
        // 钥匙册浮层(#57):同一个骨架件。删除确认是独立一道图形确认
        // (ADR-0009 §2),登记流里没有 rm 那一步,所以两道浮层平级即可。
        if self.key_rm_confirm {
            self.on_key_rm_confirm(crate::screens::CatalogKind::Key, k);
            return;
        }
        if self.key_flow.phase != catalog::RegPhase::Idle {
            self.on_key_flow(crate::screens::CatalogKind::Key, k);
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
            // 红线(#104 规格故事 10):两页同一语义,一个字节不改。
            KeyCode::Enter => self.launch_selected_lock(),
            KeyCode::Esc | KeyCode::Char('q') => self.dismiss_popup(),
            // 翻到新旧对比页 / 翻回来(#104):进出一个键,与 #101 历史视图的
            // `h` 同口径。Esc 仍一路返航——不因为站在对比页就变成「回上一页」。
            KeyCode::Char('d') => {
                self.as_ritual_page = match self.as_ritual_page {
                    crate::screens::RitualPage::Brief => crate::screens::RitualPage::Diff,
                    crate::screens::RitualPage::Diff => crate::screens::RitualPage::Brief,
                };
                if matches!(self.as_ritual_page, crate::screens::RitualPage::Diff) {
                    self.load_ritual_diff();
                }
            }
            _ => {}
        }
    }

    /// 读一遍新旧对比(#104,规格故事 8/9)。
    ///
    /// 走 [`crate::datarepo`] 唯一接缝:选中锁定集的载体文件 + 当前清单 +
    /// 清单指名的上一轮锁定集,屏幕层与 app 层都不碰文件系统(规格故事
    /// 27)。**快照语义**:只在翻到该页时读一次,不轮询、不自动刷——与
    /// #101 历史视图同口径。
    ///
    /// 判词全部来自文件,面板不重算引擎的任何结论:增 / 删是两侧组件名
    /// 的集合差(按名比);指纹「会变 / 不变」的判据只有「清单记的 lockId
    /// 与在途这一份是否相同」(上游铁律:同 lock 连跑两次 `up`,manifest
    /// 逐字节一致)。没有旧指纹可比时两个都不说。
    pub fn load_ritual_diff(&mut self) {
        let nw_home = self.nw_home.clone();
        let new_lock_id = self
            .ls
            .as_ref()
            .and_then(|l| l.locks.get(self.as_sel))
            .and_then(|l| l.lock_id.clone());
        // ls 锁行的 `name`(目标派)是老口径,但真样本的锁行不一定带它;
        // 权威答案在锁定集文件自己的 `profile` 字段里(见下一段)。
        let ls_profile = self
            .ls
            .as_ref()
            .and_then(|l| l.locks.get(self.as_sel))
            .and_then(|l| l.name.clone())
            .unwrap_or_default();
        let lock = new_lock_id
            .as_deref()
            .map(|id| crate::datarepo::read_lock(&nw_home, id));
        // 目标派以锁定集文件里的 `profile` 为准(ls 的锁行不一定带它),
        // 读不到才退到 ls 锁行的 `name`。
        let profile = match &lock {
            Some(crate::datarepo::LockStatus::Ok(l)) => {
                l.profile.clone().unwrap_or_else(|| ls_profile.clone())
            }
            _ => ls_profile.clone(),
        };
        let manifest = crate::datarepo::read_manifest(&nw_home, &profile);
        // 旧态组件面:清单自己记的那个 lock 的锁定集文件。清单缺失 = 首次
        // 装配,没有旧态可比(规格故事 9)——不摆一个吓人的空对比。
        let (old_lock_id, old_hash, first_assembly) = match &manifest {
            crate::datarepo::ManifestStatus::Ok(a) => (
                a.brief.lock_id.clone(),
                a.brief.converge_hash.clone(),
                false,
            ),
            crate::datarepo::ManifestStatus::Missing => (None, None, true),
            // 读不通 / 不认识的 schema:不当成「没有旧态」,也不当成就着它算差
            // —— 屏上走契约守卫同款话术,增删两列一并收起(见 render_diff)。
            _ => (None, None, false),
        };
        let old_lock = old_lock_id
            .as_deref()
            .map(|id| crate::datarepo::read_lock(&nw_home, id));
        let new_names = match &lock {
            Some(crate::datarepo::LockStatus::Ok(l)) => l.components.clone(),
            _ => Vec::new(),
        };
        let old_names = match &old_lock {
            Some(crate::datarepo::LockStatus::Ok(l)) => Some(l.components.clone()),
            // 上一轮锁定集文件不在 / 读不通:旧态组件面无从得知。这不是
            // 「一个组件都没有」,屏上明说无从对比,不拿空集冒充。
            _ => None,
        };
        let added = match &old_names {
            Some(old) => new_names
                .iter()
                .filter(|n| !old.contains(n))
                .cloned()
                .collect(),
            // 首次装配:没有旧态,在途这一份全部是将铺入
            None if first_assembly => new_names.clone(),
            None => Vec::new(),
        };
        let removed = match &old_names {
            Some(old) => old
                .iter()
                .filter(|n| !new_names.contains(n))
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        let hash_changes = match (&old_lock_id, &new_lock_id) {
            (Some(old), Some(new)) => old != new,
            _ => false,
        };
        self.as_ritual_diff = Some(crate::screens::RitualDiff {
            manifest: Some(manifest),
            lock,
            old_lock,
            old_lock_id,
            old_hash,
            added,
            removed,
            first_assembly,
            hash_changes,
        });
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
            // #68:「家」直达第六屏(编目)。键取 `v`(屏枚举 `Screen::Vps` 首字母),
            // 与既有 `a/h/l` 跳屏同构;屏上指令栏、帮助、README 三处同写 `v`
            // (屏上看到的与我按下的是一回事,故事 5)。HUD 不写它——HUD 是壳级
            // 提示栏,只有 j/k 切屏 / Enter 确认 / t 主题 / ? 帮助 / q 退出。
            // 纯导航零引擎调用——册体到编目屏按 `l` 才读,门禁也按册各自给,
            // 不在指令栏做禁用态。
            KeyCode::Char('v') => self.screen = Screen::Vps,
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
        // 装配历史次级视图在场(#101):键位整个归它——↑/↓ 选派、回车看清单
        // 原文、`/` 过滤、`r` 重读快照、Esc 退视图。锁定集选择器那些键
        // (Enter 开发射仪式 / x 中止)在此期间一律不受理,两个语义不串味。
        // **装配在途时整屏归运行态**:历史旗子可能还挂着(渲染画的是七步与
        // 日志),但它的键位一个都不该有分支。
        if self.as_history && !self.assembling {
            self.key_history(k);
            return;
        }
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
                self.as_confirm = true;
                // #104:每次开仪式都从简报页起——上一轮停在对比页不带给下一轮
                self.as_ritual_page = crate::screens::RitualPage::Brief;
            }
            // 装配中止(#59):在途才受理,键位取本屏未占用的 `x`。Esc 是
            // 发射前「返航」的既有语义(还没下发,什么都没发生),两个取消
            // 不许共用一键——按下去之前操作者就该知道会停的是什么。
            KeyCode::Char('x') if self.as_running => self.abort_assembling(),
            // 装配历史次级视图(#101):`h` 进 / 退。装配在途不受理——在途
            // 那屏归七步与 stderr 日志,历史是闲时浏览的只读视图;up 一下发
            // 即清场(`note_engine_dispatch`),迟到回音落地时屏幕已经回去了。
            KeyCode::Char('h') if !self.assembling => {
                self.as_history = !self.as_history;
                self.as_history_raw = None;
                if self.as_history {
                    self.load_history();
                    let n = self.history_total();
                    self.push_log(format!("装配历史:读到 {n} 个派的清单存档(只读,按 r 重读)"));
                } else {
                    self.as_history_filter_on = false;
                }
            }
            _ => {}
        }
    }

    /// 装配历史次级视图的键位(#101)。
    ///
    /// 全部只读:选中、看原文、本地过滤、重读快照。没有任何写动作,
    /// 也没有任何引擎调用(`app.queue` 一个字节都不进)。
    fn key_history(&mut self, k: KeyEvent) {
        let n = self.history_visible().len();
        match k.code {
            KeyCode::Down => {
                if n > 0 {
                    self.as_history_sel = (self.as_history_sel + 1) % n;
                }
            }
            KeyCode::Up => {
                if n > 0 {
                    self.as_history_sel = (self.as_history_sel + n - 1) % n;
                }
            }
            KeyCode::Enter => self.show_history_raw(),
            // 重读快照(#101):显式刷新,不自动跑——文件可能在面板之外变过
            // (引擎自己装了一版),要新的就按一下。
            KeyCode::Char('r') => {
                self.load_history();
                let n = self.history_total();
                self.push_log(format!("装配历史已重读:{n} 个派(只读快照)"));
            }
            KeyCode::Char('/') => self.as_history_filter_on = true,
            // `h` 再按一次即退(进出一个键,不必记两个)。与 Esc 的分工:
            // Esc 第一级清过滤词、第二级退视图;`h` 直接退,过滤词留着
            // (与切屏「退输入态、留词」同口径)。
            KeyCode::Char('h') => {
                self.as_history = false;
                self.as_history_raw = None;
                self.as_history_filter_on = false;
            }
            // Esc 两级(#61 / #69 同口径):有过滤词先清词,再按才退视图。
            KeyCode::Esc => {
                if !self.as_history_filter.is_empty() {
                    self.clear_history_filter();
                    self.push_log("已清空过滤词:回到装配历史全量".to_string());
                } else {
                    self.as_history = false;
                    self.as_history_raw = None;
                }
            }
            _ => {}
        }
    }

    /// 装配历史过滤输入态(#101,与 `on_key_rc_filter` / `on_key_catalog_filter`
    /// 同一形态、同一后果链):键入归通用态机,本函数只接过滤这一条的后果。
    /// **纯本地**:改的只有 `as_history_filter` 一个字符串,零引擎调用。
    fn on_key_history_filter(&mut self, k: KeyEvent) {
        match input::on_key(&mut self.as_history_filter, k) {
            InputAction::Submit => {
                self.as_history_filter_on = false;
                self.as_history_sel = 0;
                let n = self.history_visible().len();
                self.push_log(format!(
                    "过滤词「{}」:{} 个派命中(本地筛选,未入队)",
                    self.as_history_filter.trim(),
                    n
                ));
            }
            InputAction::Cancel => {
                self.clear_history_filter();
                self.push_log("已清空过滤词:回到装配历史全量".to_string());
            }
            // 每改一个词选中回顶:行数变了,旧下标可能指向另一派
            InputAction::Edited => self.as_history_sel = 0,
            InputAction::Ignored => {}
        }
    }

    /// 清空装配历史过滤词并让选中回顶(#101):输入态 Cancel / Esc 共用
    /// 一条后果链(与 `clear_rc_filter` 同形状)。
    fn clear_history_filter(&mut self) {
        self.as_history_filter.clear();
        self.as_history_filter_on = false;
        self.as_history_sel = 0;
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
            // 引擎版本对照页(#108):v 翻开,再按 v 或 Esc 回常驻面板。
            // 引擎没找到时不给翻——没有本地版本可比,指路屏的话已经说清了。
            KeyCode::Char('v') if self.engine.found => {
                self.settings_page = match self.settings_page {
                    SettingsPage::Panel => SettingsPage::Version,
                    SettingsPage::Version => SettingsPage::Panel,
                };
                self.push_log(match self.settings_page {
                    SettingsPage::Panel => "版本对照收起:回常驻面板".to_string(),
                    SettingsPage::Version => {
                        "版本对照:本地 nw 版本 ↔ 引擎 release 仓最新 tag".to_string()
                    }
                });
            }
            // 版本对照页内 Esc 返航(壳级 Esc 的既有语义:「返回」)。常驻面板上
            // Esc 仍是无分支——那里没有可返回的上一层。
            KeyCode::Esc if self.settings_page == SettingsPage::Version => {
                self.settings_page = SettingsPage::Panel;
            }
            // 版本对照页内 r 重查(显式刷新,与健康屏 r 复核同语气)
            KeyCode::Char('r') if self.settings_page == SettingsPage::Version => {
                self.reprobe_release();
            }
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

    /// 编目册册内过滤输入态(#69,与 `on_key_rc_filter` 同一形态、同一后果链):
    /// 键入分发归通用态机 [`input::on_key`](与两份手填 / 五域过滤同一条路),
    /// 本函数只接过滤这一条的后果链。**纯本地**:改的只有当前册的过滤词一个
    /// 字符串,不入队、不下发任何引擎命令、不把关键词写进别处。`Enter` 收词
    /// 退输入态;`Esc/q` 取消并清空过滤词(两种退出都可预期);每改一个词选中
    /// 回顶。按册各留一份:清的只是当前这一本的词。
    fn on_key_catalog_filter(&mut self, k: KeyEvent) {
        let action = input::on_key(self.catalog_filter_mut(), k);
        match action {
            InputAction::Submit => {
                self.catalog_filter_on = false;
                *self.catalog_kind.sel_mut(self) = 0;
                let n = self.catalog_visible().len();
                self.push_log(format!(
                    "过滤词「{}」:{}册 {} 条命中(本地筛选,未入队)",
                    self.catalog_filter().trim(),
                    self.catalog_kind.label(),
                    n
                ));
            }
            InputAction::Cancel => {
                self.clear_catalog_filter();
                self.push_log("已清空过滤词:回到编目全量".to_string());
            }
            InputAction::Edited => {
                *self.catalog_kind.sel_mut(self) = 0;
            }
            InputAction::Ignored => {}
        }
    }

    /// 清空当前册过滤词并让选中回顶(#69):输入态 Cancel / Esc 共用一条后果链
    /// (与 `clear_rc_filter` 同形状)。**只清当前这一本**——换册各留各的词。
    fn clear_catalog_filter(&mut self) {
        self.catalog_filter_mut().clear();
        self.catalog_filter_on = false;
        *self.catalog_kind.sel_mut(self) = 0;
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
        // #69:登记 / 删除 / 反查都作用于「看得见的那一行」——下标经过滤后的
        // 可见列表映射回原始投影。不过滤时 vis = 0..n,与过去的 `machines[sel]`
        // 等价(投影按名排序,册体与选中同一口径)。
        let sheet = self.catalog_sheet()?;
        let i = *self.catalog_visible().get(self.vps_sel)?;
        sheet.entries.get(i).map(|e| e.name.clone())
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

    /// 编目册屏键位:`m` 换册(机器 / 标签 / 订阅 / 钥匙);各册 r 登记 /
    /// ↑/↓ 选 / d 删除 / l 重读;订阅与钥匙册 `g` 只给取回建议命令、
    /// 标签册 `Enter` 反查选中的这一条(#54 / #56 / #57);`/` 册内过滤(#69)。
    fn key_catalog(&mut self, k: KeyEvent) {
        if let KeyCode::Char('m') = k.code {
            self.cycle_catalog_kind();
            return;
        }
        // 册内过滤入口(#69):`/` 进过滤输入态(点正文「过滤」段同路,见
        // `on_mouse` 的 `Target::CatalogFilter`)。与 #61 五域 `/` 同形:输入态
        // 优先于册内动作键,键入即筛。
        if let KeyCode::Char('/') = k.code {
            self.catalog_filter_on = true;
            return;
        }
        // Esc 一键回全量(#69):不在输入态时,按 Esc 清当前册过滤词并让选中回顶
        // (输入态下 Esc 归 `on_key_catalog_filter` 的 Cancel,同一后果链)。
        // 无过滤词时 Esc 什么都不做(与 #61 五域同口径)。
        if let KeyCode::Esc = k.code {
            if !self.catalog_filter().is_empty() {
                self.clear_catalog_filter();
                self.push_log("已清空过滤词:回到编目全量".to_string());
            }
            return;
        }
        // 册内动作键(#66):**四本册同一处查表 + 同一处落实**。按册差异全在
        // `CatalogKind::action` 那张表上(同一个键在不同册上可以落到不同
        // 方法,也可以这一本没有这个键),这里不再有四份逐字重复的
        // `match k.code`。加第五本册子 = 表里加行,本函数不动。
        let Some(act) = self.catalog_kind.action(self, k.code) else {
            return;
        };
        match act {
            catalog::CatalogAction::Move(down) => self.catalog_move(down),
            catalog::CatalogAction::Act(f) => f(self),
            catalog::CatalogAction::Start(mode) => self.start(self.catalog_kind, mode),
        }
    }

    /// 册级登记 / 删除入口(#76:四本册的 `start_*` 八份收成一份)。
    ///
    /// 操作者感知不到任何变化——四本册各自的入口门、写互斥、表单复位与提示
    /// 逐字节不变;这一票买的是「加一条按册的登记 / 删除关切只改一处」(
    /// [`crate::screens::CatalogKind`] 查表成员),不再改四处。按册差异全走查表,
    /// 这里不再有 `match kind` / `if kind ==`:装配在途与入口门两道守卫按 mode
    /// 取各自的日志口径(登记 vs 删除),过了守卫后 Register/Add 复位表单进登记
    /// 流、Rm 先取选中名再走各自的删除发起(机器 / 订阅 / 钥匙三本进 rm 确认
    /// 弹层;标签册的删除嵌套在表单流里)。
    fn start(&mut self, kind: crate::screens::CatalogKind, mode: crate::screens::StartMode) {
        if self.assembling {
            match mode {
                crate::screens::StartMode::Register => self.push_log(format!(
                    "装配进行中:{}与装配互斥,稍后再试",
                    kind.register_verb()
                )),
                crate::screens::StartMode::Rm => {
                    self.push_log("装配进行中:删除与装配互斥,稍后再试")
                }
            }
            return;
        }
        if !kind.entry_enabled(self) {
            match mode {
                crate::screens::StartMode::Register => self.push_log(format!(
                    "{}入口不可用:{}",
                    kind.register_verb(),
                    kind.gate_hint(self)
                )),
                crate::screens::StartMode::Rm => {
                    self.push_log(format!("删除未下发:{}", kind.gate_hint(self)))
                }
            }
            return;
        }
        match mode {
            crate::screens::StartMode::Register => {
                kind.rm_confirm_set(self, false);
                kind.flow_mut(self).begin();
                kind.form_reset(self);
                self.push_log(kind.register_prompt());
            }
            crate::screens::StartMode::Rm => {
                let Some(name) = kind.selected_name(self) else {
                    self.push_log(format!(
                        "删除未下发:没有选中的{}(↑/↓ 先选一行)",
                        kind.label()
                    ));
                    return;
                };
                kind.start_rm(self, name);
            }
        }
    }

    /// 册级登记流的键位(表单 / 等回音 / 确认 / 落账在途 / 结果;#76:四本册的
    /// `on_key_*_flow` 四份收成一份)。
    ///
    /// 操作者感知不到任何变化——四本册各自的表单键位、等回音段、确认段、结论段
    /// 逐字节不变;这一票买的是「加一条按册的登记流键位关切只改一处」
    /// ([`crate::screens::CatalogKind`] 查表成员),不再改四处。按册差异全走查表,
    /// 这里不再有 `match kind` / `if kind ==`:表单段问这一本的 `form_key`,确认段
    /// 按这一本**有没有引擎确认闸**(`confirm_gate`)决定认不认「Enter 落账」——
    /// 标签册没有 `--yes`(ADR-0009 §2 面板不自己造一道),它的结论屏只有返航。
    fn on_key_flow(&mut self, kind: crate::screens::CatalogKind, k: KeyEvent) {
        let phase = kind.flow(self).phase;
        match phase {
            catalog::RegPhase::Form => match kind.form_key(self, k) {
                catalog::FormAction::Submit => self.dispatch_preaudit(kind),
                catalog::FormAction::Cancel => self.cancel(kind),
                catalog::FormAction::Edited => {}
            },
            // 等回音段:Esc 返航。标签册没有预审段(它的写命令是 `tag add`,见
            // `dispatch_preaudit` 的 `write_route`),这一条对它恒不成立。
            catalog::RegPhase::PreAudit | catalog::RegPhase::Commit => {
                if matches!(k.code, KeyCode::Esc) {
                    self.cancel(kind);
                }
            }
            catalog::RegPhase::Review => {
                if kind.confirm_gate() {
                    match k.code {
                        KeyCode::Enter => self.dispatch_commit(kind),
                        KeyCode::Esc => self.cancel(kind),
                        _ => {}
                    }
                } else if matches!(k.code, KeyCode::Esc | KeyCode::Char('q')) {
                    // 标签册:没有引擎确认闸,面板也不自己造一道(ADR-0009 §2)——
                    // 写已随那一次下发完成,结论屏只有返航,没有「Enter 落账」段。
                    self.cancel(kind);
                }
            }
            catalog::RegPhase::Done => {
                if matches!(k.code, KeyCode::Esc | KeyCode::Char('q')) {
                    self.cancel(kind);
                }
            }
            catalog::RegPhase::Idle => {}
        }
    }

    /// 删除确认弹层的键位(四本册逐字节相同:Enter 下发,Esc/q 收回;#76:四份
    /// `on_key_*_rm_confirm` 收成一份)。
    fn on_key_rm_confirm(&mut self, kind: crate::screens::CatalogKind, k: KeyEvent) {
        match k.code {
            KeyCode::Enter => self.dispatch_rm(kind),
            KeyCode::Esc | KeyCode::Char('q') => self.dismiss_popup(),
            _ => {}
        }
    }

    /// 下发登记 / 挂标签(#76:三本册的 `dispatch_*_register` 与标签册的
    /// `dispatch_tag_write` 收成一份)。
    ///
    /// 四本册同一副骨架:两道守卫(入口门 / 数据仓根,任一不过即返航,不留半条
    /// 记录)→ 面板侧定型(空名字 / 形状不合就拒,留表单可改,不喂给引擎跑一趟)→
    /// 按这一本的 [`crate::screens::WriteRoute`] 递出去。按册差异(入口门与提示
    /// 口径、表单形态、写命令进哪条队列、日志口径)全走查表,这里不再有
    /// `match kind` / `if kind ==`。
    ///
    /// 名字不与外部源登记的 [`Self::dispatch_register`] 抢:那一族是另一副骨架
    /// (`RegState` 六态),本函数只服务编目册四本。
    fn dispatch_preaudit(&mut self, kind: crate::screens::CatalogKind) {
        if !kind.entry_enabled(self) {
            self.push_log(format!(
                "{}{}",
                kind.dispatch_gate_prefix(),
                kind.gate_hint(self)
            ));
            self.cancel(kind);
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!(
                "{}{}",
                kind.dispatch_gate_prefix(),
                self.data_root_gate_msg()
            ));
            self.cancel(kind);
            return;
        };
        let spec = match kind.spec_from_form(self) {
            Ok(s) => s,
            Err(bad) => {
                // 面板侧就拒,留表单可改,不喂给引擎跑一趟。
                self.push_log(format!(
                    "{}{},留表单可改",
                    kind.dispatch_reject_prefix(),
                    bad.join(";")
                ));
                return;
            }
        };
        match kind.write_route(self) {
            // 两段式第一段(预审):不带 `--yes`,文件不落。
            crate::screens::WriteRoute::Preaudit => {
                kind.flow_mut(self).preaudit_sent();
                let fed = kind.dispatch_cmd(self, spec.preaudit(&root));
                self.push_log(kind.preaudit_log(spec.name(), fed));
            }
            // 标签册的 `tag add`:没有确认闸,表单 Enter 直接进写段。
            crate::screens::WriteRoute::Commit => self.dispatch_commit(kind),
            // 标签册的 `tag rm`:只起面板侧那道图形确认,不下发。
            crate::screens::WriteRoute::RmConfirm => kind.raise_rm_confirm(self, spec.name()),
        }
    }

    /// 落账(两段式第二段):图形确认后才可能到达——全流程唯一能产生 `--yes`
    /// 的转换。重过三道闸(入口 / 数据仓根 / 预审结论),全过才下发并点亮写
    /// 互斥;任何一道不过都不下发写命令。#76:三本册的 `dispatch_*_commit` 收成
    /// 一份,标签册的 `tag add`(没有确认闸)也从表单 Enter 直接到这一条。
    fn dispatch_commit(&mut self, kind: crate::screens::CatalogKind) {
        // 第一道闸只问有确认闸的三本:标签册从表单 Enter 直接进写段
        // (`write_route` 的 Commit 臂),没有「预审通过」这一说。
        if kind.confirm_gate()
            && (kind.flow(self).phase != catalog::RegPhase::Review || !kind.can_commit(self))
        {
            self.push_log(kind.commit_gate_msg());
            return;
        }
        if !kind.entry_enabled(self) {
            self.push_log(format!("落账未下发:{}", kind.gate_hint(self)));
            return;
        }
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("落账未下发:{}", self.data_root_gate_msg()));
            return;
        };
        let Ok(spec) = kind.spec_from_form(self) else {
            self.push_log("落账未下发:表单已不可定型(返航后从表单重来)");
            return;
        };
        kind.flow_mut(self).commit_sent();
        self.assembling = true;
        let fed = kind.dispatch_cmd(self, spec.commit(&root));
        self.push_log(kind.commit_log(spec.name(), &root, fed));
    }

    /// 下发删除:argv 与在 shell 里敲的逐字相同(面板只决定何时递出去;
    /// #76:四本册的 `dispatch_*_rm` 收成一份)。
    fn dispatch_rm(&mut self, kind: crate::screens::CatalogKind) {
        // 删谁:机器 / 订阅 / 钥匙三本取当前选中行;标签册的删除嵌套在表单流里,
        // 目标由表单定型。取不到即收确认旗、按册留一句,不下发。
        let Some(name) = kind.rm_target(self) else {
            kind.rm_confirm_set(self, false);
            self.push_log(kind.rm_missing_log());
            return;
        };
        let Some(root) = self.usable_data_root() else {
            self.push_log(format!("删除未下发:{}", self.data_root_gate_msg()));
            return;
        };
        // 写互斥(ADR-0009 §4:rm 是写动作,并入既有 assembling 语义;
        // 与 register 落账段同一处点灯,回音到达时灭,见 `on_echo_rm` → `CatalogKind::absorb_rm`)。
        kind.rm_confirm_set(self, false);
        kind.rm_result_clear(self);
        self.assembling = true;
        let argv = kind.rm_cmd(self, &root, &name);
        self.queue.push(argv);
        kind.rm_consumed(self, &name, &root);
    }

    /// 返航:任一态 Esc 同义,状态清空不留半条记录(#76:四份 `cancel_*` 收成一份)。
    /// 清状态那一半四本同一条(收流 + 复位表单 + 收删除确认旗),按册只剩那句
    /// 日志的口径(`CatalogKind::cancel_log`)。
    fn cancel(&mut self, kind: crate::screens::CatalogKind) {
        kind.flow_mut(self).abandon();
        kind.form_reset(self);
        // 标签册的删除确认嵌套在表单流里(#54),返航连它一起收;其余三本在登记
        // 流里那面旗恒 false,这一句对它们是空操作。
        kind.rm_confirm_set(self, false);
        self.push_log(kind.cancel_log());
    }

    /// 删除确认被收回(鼠标点任意处与键盘 Esc/q 同路):收旗,留一句「一个字节
    /// 都没动」。四本册同一形状,差别只在数据仓根下那个文件名
    /// (`CatalogKind::file`)。
    fn dismiss_rm(&mut self, kind: crate::screens::CatalogKind) {
        kind.rm_confirm_set(self, false);
        self.push_log(format!("删除已取消:{} 一个字节都没动", kind.file()));
    }

    /// vps 落账闸(呈现口径):预审通过 = rc 0 且五道闸无 fail 且 data 可定型。
    pub fn vps_can_commit(&self) -> bool {
        let Some(out) = self.vps_flow.result.as_ref() else {
            return false;
        };
        catalog::preaudit_passed(out, catalog::VpsSpec::audit(out).as_ref())
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

    /// 编目册回音路由(#76:`on_*_echo` 四份收成一份)。按子命令分发,按册差异
    /// 走 [`crate::screens::CatalogKind`] 查表成员,这里不再有 `match kind`。
    fn on_echo(&mut self, kind: crate::screens::CatalogKind, out: EngineOutput) {
        match out.args.get(1).map(String::as_str) {
            Some("ls") => self.on_echo_ls(kind, out),
            Some("register") => self.on_echo_register(kind, out),
            Some("add") => self.on_echo_add(out),
            Some("rm") => self.on_echo_rm(kind, out),
            _ => {}
        }
    }

    /// ls 回音(只读刷新):灭「在读」旗后,按册落地(`kind.absorb_ls`)。
    fn on_echo_ls(&mut self, kind: crate::screens::CatalogKind, out: EngineOutput) {
        *kind.ls_loading_mut(self) = false;
        kind.absorb_ls(self, out);
    }

    /// 登记回音两段式(#53 / #56 / #57;vps / 订阅 / 钥匙三本同形,收成一份):
    /// 预审段摊开闸结论,落账段(图形确认后才可能到达)收敛写互斥并刷新册。
    /// 标签册没有这一段(它的写命令是 `add`,见 [`Self::on_echo_add`])。
    fn on_echo_register(&mut self, kind: crate::screens::CatalogKind, out: EngineOutput) {
        let is_commit = out.args.iter().any(|a| a == "--yes");
        if is_commit {
            let echo_ok = out.ok;
            let verdict = kind.flow_mut(self).commit_back(out);
            match verdict {
                Some(true) => {
                    self.assembling = false;
                    self.push_log(format!(
                        "登记落账完成:{} 已写入数据仓根(一次 git 变更)",
                        kind.file()
                    ));
                    kind.run_ls(self);
                }
                Some(false) => {
                    self.assembling = false;
                    let h = kind
                        .flow(self)
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
                        kind.run_ls(self);
                        self.push_log(format!(
                            "登记写盘可能已完成:{}已刷新,{} 走 PR + 家长审复核",
                            kind.write_refreshed_noun(),
                            kind.file()
                        ));
                    } else {
                        self.push_log("登记落账回音已丢弃(流程已返航,写未成功)");
                    }
                }
            }
        } else if kind.flow_mut(self).preaudit_back(out) {
            let h = kind
                .flow(self)
                .result
                .as_ref()
                .map(|o| o.headline())
                .unwrap_or_default();
            self.push_log(format!("登记预审完成:{h}"));
        } else {
            self.push_log("登记预审回音已丢弃(流程已返航)");
        }
    }

    /// 挂标签回音(#54;标签册独有的写命令,无确认闸):落账即终态,回音摊开
    /// 闸结论。只有标签册走这一段(`add` 是上游 `tag add` 的动词,别本没有)。
    fn on_echo_add(&mut self, out: EngineOutput) {
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

    /// rm 回音(#53 / #54 / #56 / #57):写互斥随 rm 回音收敛(删不存在的名 rc 3
    /// 同样到达这里——互斥释放,文件一个字节都没动)。按册落地 `kind.absorb_rm`。
    fn on_echo_rm(&mut self, kind: crate::screens::CatalogKind, out: EngineOutput) {
        self.assembling = false;
        kind.absorb_rm(self, out);
    }

    // ---- 编目册屏·标签册(spec #50 工单 #54,ADR-0009) ----

    /// 点键位行「g 取回」段 = 按 `g`(#73)。
    ///
    /// 走 [`catalog::CatalogKind::action`] **同一张表**(与 `key_catalog` 受理
    /// `g` 的那一次查表同一个入口),由当前是哪一本决定落实方法:订阅册 →
    /// [`App::log_sub_get_suggest`],钥匙册 → [`App::log_key_get_suggest`]。鼠标
    /// 与键盘因此不可能给出两种后果,也不会有第二份「点下去发生什么」。
    ///
    /// 段只在订阅 / 钥匙册的非空册体变体在场(命中区的产地
    /// [`catalog::catalog_key_parts`]),所以这一处到达即确有命令可取;表里查不到
    /// (机器册 / 标签册没有 `g`)就什么都不做——不为鼠标补一条键盘也没有的路。
    fn catalog_get(&mut self) {
        if let Some(catalog::CatalogAction::Act(f)) = self
            .catalog_kind
            .action(self, crossterm::event::KeyCode::Char('g'))
        {
            f(self);
        }
    }

    /// 点键位行「d 删除」段 = 按 `d`(#78,兑 #75 裁决 A:点段 ≡ 按键)。
    ///
    /// 走 [`catalog::CatalogKind::action`] **同一张表**(与 `key_catalog` 受理
    /// `d` 的那一次查表同一个入口):四本册同一个动作值
    /// `Start(StartMode::Rm)`,由 [`App::start`] **一个函数**落实——机器 /
    /// 订阅 / 钥匙三本进 rm 确认弹层(ADR-0009 §2:上游 `rm` 都没有 `--yes`,
    /// 面板加一道,删掉要重新登记、代价不对称),标签册的删除嵌套在表单流里
    /// (摘引用 / 删整个标签两态)。按册差异全在查表成员里,鼠标与键盘因此不可能
    /// 给出两种后果,也不会有第二份「点下去发生什么」。
    ///
    /// 段在四本册的非空、非在途键位行变体在场(命中区的产地
    /// [`catalog::catalog_key_parts`]),所以这一处到达即确有册可删;无选中行
    /// (如册内过滤筛到空)时 `selected_name` 落空,复现键盘 `d` 的既有指路
    /// (「删除未下发:没有选中的…」),不新造灰置态;装配在途 / 入口门不过时
    /// 复现各自的既有日志。过滤输入态里键入归输入态、鼠标照旧点得动,那是
    /// 键位行所有可点段的共同边界(见 [`catalog_get`] 的同口径注记)。
    fn catalog_delete(&mut self) {
        if let Some(catalog::CatalogAction::Start(mode)) = self
            .catalog_kind
            .action(self, crossterm::event::KeyCode::Char('d'))
        {
            self.start(self.catalog_kind, mode);
        }
    }

    /// 当前是哪一本(`m` 换册)。换册只改维度、不清另一本的投影——
    /// 四本各有自己的账,回来还在(不重跑、不轮询)。
    pub fn cycle_catalog_kind(&mut self) {
        self.catalog_kind = self.catalog_kind.next();
        // 换册即离开机器册语境(#70):反查上浮是机器册独有的只读动作,换册
        // 取消任何在途 / 在场的反查——不把机器册的结论顺带到别的册上,也不
        // 在换册这种「别的动作」后顺手收敛一次回音。
        self.vps_reverse = crate::screens::catalog::VpsReverse::Idle;
        let k = self.catalog_kind;
        self.push_log(format!(
            "换册:{}册({});{}",
            k.label(),
            k.file(),
            k.next_hint()
        ));
    }

    /// 点概览警示带的订阅到期条目 = 跳到订阅册并选中那一条(#94 工单 #105,
    /// 用户故事 12:「从发现到处理一步到位」)。三步与键盘 `v` 到编目屏同族:
    /// 先定屏内维度(订阅册)、再切屏、最后把册级选中槽落到那一条上。
    ///
    /// 选中槽是**可见行**下标(#69 册内过滤),故载荷给的投影下标要经
    /// `catalog_visible()` 换算;换算不出来(这一条被过滤词挡在可见行外)只留
    /// 一行指路日志,不新造禁用态——与册体「无选中行时点下去复现既有指路」
    /// 同一口径。
    ///
    /// **零引擎调用**:回音是既有的 `app.sub_ls`,这里只换视图。
    pub(crate) fn open_subscription_book(&mut self, idx: usize) {
        self.catalog_kind = crate::screens::CatalogKind::Subscription;
        // 换册即离开机器册语境(#70):反查上浮是机器册独有的只读动作,跳册同样
        // 收回任何在场 / 在途的反查——与 `cycle_catalog_kind` 同一条纪律。
        self.vps_reverse = crate::screens::catalog::VpsReverse::Idle;
        self.screen = Screen::Vps;
        self.catalog_filter_on = false; // 进册不顺手进过滤输入态(同 NavTo)
        let name = self
            .catalog_sheet()
            .and_then(|s| s.entries.get(idx).map(|e| e.name.clone()));
        let pos = self.catalog_visible().iter().position(|&i| i == idx);
        match (pos, name) {
            (Some(p), Some(name)) => {
                let (sel, _) = self.catalog_sel_mut();
                *sel = p;
                self.push_log(format!("跳到订阅册并选中 {name}"));
            }
            (None, Some(name)) => self.push_log(format!(
                "跳到订阅册:{name} 不在当前过滤后的可见行里——册内按 / 改过滤词"
            )),
            _ => self.push_log("跳到订阅册:这一条不在回音里了(可能刚被删)"),
        }
        self.catalog_clamp();
    }

    /// 编目册屏当前这一本的投影(四本各自的数据源汇到同一个
    /// `CatalogSheet`——列表件只有一份,见 `screens::catalog`)。
    ///
    /// 「四本各读哪个状态」那处匹配长在 [`catalog::CatalogKind::sheet`] 上
    /// (#66:数据源形状本就不同,这一处匹配是全模块唯一 unavoidable 的
    /// 一处,那就让它只存在于查表成员里,调用方不再各自分册)。
    pub fn catalog_sheet(&self) -> Option<crate::engine::CatalogSheet> {
        self.catalog_kind.sheet(self)
    }

    /// 反查回音的失败态(带参问错了名字 = rc 3 / 回音不可定型)。
    /// `None` = 没有失败态。**只标签册有这一态**:机器册的 `vps ls` 没有
    /// 位置参数,失败时只留日志(51 §6.11:两种语义不都画成空列表)。
    /// 「只有一本册有」这条关切落在查表成员上(#66)。
    pub fn catalog_ls_error(&self) -> Option<&EngineOutput> {
        self.catalog_kind.ls_error(self)
    }

    /// 册体首行的口径词:机器册给册名,标签册把「列全部 / 只列一个」
    /// 这档差别显式画出来(51 §6.11:两种语义不都画成空列表)。
    /// 查表成员 [`catalog::CatalogKind::scope_label`] 的一层转发(#66)。
    pub fn catalog_scope_label(&self) -> String {
        self.catalog_kind.scope_label(self)
    }

    /// 编目册当前这一本的选中行(↑/↓ 与鼠标点行同源)。
    /// 查表成员 [`catalog::CatalogKind::sel`] 的一层转发(#66)。
    pub fn catalog_sel(&self) -> usize {
        self.catalog_kind.sel(self)
    }

    /// 当前这一本的过滤词(#69,按册各留一份):取 `catalog_filter[idx]`。
    /// 换册时每本各留各的词,靠 [`CatalogKind::idx`](crate::screens::CatalogKind::idx)
    /// 取自己那一份——调用方不写 `match kind`。
    pub fn catalog_filter(&self) -> &str {
        &self.catalog_filter[self.catalog_kind.idx()]
    }

    /// 当前这一本过滤词的可写侧(输入态 [`Self::on_key_catalog_filter`] 键入用)。
    fn catalog_filter_mut(&mut self) -> &mut String {
        &mut self.catalog_filter[self.catalog_kind.idx()]
    }

    /// 当前这一本**可见行**在原始投影里的下标(#69 册内过滤,与 #61 五域
    /// `rc_visible` 同一族口径,全册单一「看得见的那一行」真相源):无关键词时
    /// 就是 `0..n` 全量同序(与过滤前逐字节相同,面板永不重排引擎输出);有关键词时
    /// 按册在该行字段上做大小写不敏感包含匹配。只筛面板已拿到的投影——不新增
    /// 引擎调用、不改数据来源、不把关键词带进任何命令。
    pub fn catalog_visible(&self) -> Vec<usize> {
        let Some(sheet) = self.catalog_sheet() else {
            return Vec::new();
        };
        let q = self.catalog_filter().trim().to_lowercase();
        (0..sheet.entries.len())
            .filter(|&i| {
                q.is_empty()
                    || self
                        .catalog_kind
                        .match_fields(&sheet.entries[i])
                        .iter()
                        .any(|f| f.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// 册级选中态的**单一查表**:当前是哪一本,就借出它自己的选中槽与行数
    /// (键位 ↑/↓、鼠标点行、滚轮移选中都走这一口,换册不串味)。
    /// 行数取**可见行数**(#69:`catalog_sel` 是「看得见的那一行」在可见列表里
    /// 的位置,↑/↓ / 点行 / 钳位都建立在 `catalog_sel ∈ [0, 可见行数)` 上;
    /// 不过滤时可见行数 = 册体条数,与过去逐字等价)。
    /// 只读侧的同源口径是 [`App::catalog_sel`] / [`App::catalog_len`]。
    fn catalog_sel_mut(&mut self) -> (&mut usize, usize) {
        let n = self.catalog_visible().len();
        (self.catalog_kind.sel_mut(self), n)
    }

    /// 当前这一本的行数(只读侧;与 `catalog_sel_mut` 同一张表)。
    /// 查表成员 [`catalog::CatalogKind::len`] 的一层转发(#66)。
    pub fn catalog_len(&self) -> usize {
        self.catalog_kind.len(self)
    }

    /// 册级选中行钳回可见范围内(刷新后旧下标可能越界;四本册同一口径)。
    pub(crate) fn catalog_clamp(&mut self) {
        let (sel, n) = self.catalog_sel_mut();
        *sel = if n == 0 { 0 } else { (*sel).min(n - 1) };
    }

    /// 册级键盘 ↑/↓ 移选中:四本册同一口径。与鼠标/滚轮的唯一差别是
    /// **到边界绕回**(既有键盘行为,零回归);鼠标/滚轮到界停界
    /// (`on_mouse` / `wheel_sel` 走 `catalog_sel_mut` 的钳位口径)。
    fn catalog_move(&mut self, down: bool) {
        let (sel, n) = self.catalog_sel_mut();
        if n > 0 {
            *sel = if down {
                (*sel + 1) % n
            } else {
                (*sel + n - 1) % n
            };
        }
    }

    /// 标签册的行数(反查投影条数;没查到 = 0)。
    pub fn tag_len(&self) -> usize {
        self.catalog_sheet().map(|s| s.entries.len()).unwrap_or(0)
    }

    /// 选中标签的名字(反查这一条 / 摘引用删标签同源)。
    pub fn selected_tag_name(&self) -> Option<String> {
        // #69:同 `selected_vps_name`——经过滤后的可见列表映射回原始投影。
        let sheet = self.catalog_sheet()?;
        let i = *self.catalog_visible().get(self.tag_sel)?;
        sheet.entries.get(i).map(|e| e.name.clone())
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

    /// 标签册 `l`:列全部标签(不带参的只读反查)。单独一个方法名,是为了让
    /// [`catalog::CatalogKind::action`] 的键位表能把它当**数据**指过去
    /// (#66:表里每一项都是「这个键在这一本上落到哪个方法」,不塞闭包、
    /// 不在调用方留分支)。
    pub fn run_tag_ls_all(&mut self) {
        self.run_tag_ls(None);
    }

    /// 标签册 `Enter`:反查选中的这一条(带参 `tag ls <标签>`,本票主价值
    /// 的第二个态)。没选中行 = 一行日志、不下发命令——与 `run_tag_ls`
    /// 的门禁同一口径:面板不空手发命令,也不把「没选」画成一次反查。
    pub fn tag_lookup_selected(&mut self) {
        let Some(name) = self.selected_tag_name() else {
            self.push_log("反查未下发:没有选中的标签(↑/↓ 先选一行)");
            return;
        };
        self.run_tag_ls(Some(&name));
    }

    /// 反查上浮(#70 主入口,只读):机器册选中行上一个键(Enter)或点键位段,
    /// 问「什么标签引用了这一条」。**门禁顺序与面板信任序一致**:先看
    /// 数据仓根设没设、再看标签组能力可不可用,最后才发那一次只读
    /// `nw tag ls`(无参全列);任缺一条给对应指路,**不发命令、不发在途**
    /// (`tag_entry_enabled` 内部就是「先根后能力」,`tag_gate_hint` 按同序
    /// 给各自的指路,互不冒充)。
    ///
    /// 反查是只读,**不并入 `assembling` 全局写互斥**;在途是独立小状态
    /// (`vps_reverse`),不拿上一轮结论冒充本次。读法复用既有
    /// [`run_tag_ls`](App::run_tag_ls) 落进标签册既有的 `tag_ls_echo`,再
    /// 本地按 `<kind>/<条目>` 精确匹配推导——**不新增调用形状、不新增写
    /// 路径、不动 `tags.json` 一个字节**。
    pub fn vps_reverse_lookup(&mut self) {
        // 门禁①+②:先数据仓根、再标签组能力(任一缺给各自指路,零引擎调用)。
        if !self.tag_entry_enabled() {
            self.push_log(format!("反查未下发:{}", self.tag_gate_hint()));
            return;
        }
        let Some(_root) = self.usable_data_root() else {
            self.push_log(format!("反查未下发:{}", self.data_root_gate_msg()));
            return;
        };
        // 选中行:机器册没选中(空册)不下发——面板不空手问,也不把「没选」
        // 画成一次反查。
        let Some(machine) = self.selected_vps_name() else {
            self.push_log("反查未下发:没有选中的机器(↑/↓ 先选一行)");
            return;
        };
        // 在途:清空上一轮结论(`run_tag_ls` 会把 `tag_ls_echo` 置 None),独立
        // 小状态记住这次问的是哪台机器;回音落地时按它推导。
        self.vps_reverse = crate::screens::catalog::VpsReverse::InFlight { machine };
        self.run_tag_ls(None);
    }

    /// 反查结果浮层在不在场(只有 有 N 条 / 一条都没有 两态画;渲染 / 命中 /
    /// 键位吞键共用这一判据)。
    pub fn vps_reverse_open(&self) -> bool {
        self.vps_reverse.is_open()
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
        // #69:同 `selected_vps_name`——经过滤后的可见列表映射回原始投影。
        let sheet = self.catalog_sheet()?;
        let i = *self.catalog_visible().get(self.sub_sel)?;
        sheet.entries.get(i).map(|e| e.name.clone())
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

    /// 订阅落账闸(呈现口径):预审通过 = rc 0 且四道闸无 fail 且 data 可定型。
    pub fn sub_can_commit(&self) -> bool {
        let Some(out) = self.sub_flow.result.as_ref() else {
            return false;
        };
        catalog::preaudit_passed(out, catalog::SubscriptionSpec::audit(out).as_ref())
    }

    /// 订阅册返航(#76 收敛后的转口:实现只有一份 [`Self::cancel`]——四本册同一句
    /// 「状态清空不留半条记录」,按册那点差别(在途本体随表单一起 drop)在
    /// [`crate::screens::CatalogKind`] 查表成员上)。测试直接驱动它。
    #[cfg(test)]
    pub fn cancel_sub_register(&mut self) {
        self.cancel(crate::screens::CatalogKind::Subscription);
    }

    // ---- 编目册屏·钥匙册(spec #50 工单 #57,ADR-0009;机密纪律最严的一本) ----

    /// 钥匙册的行数(ls 投影条数;没读到 = 0)。
    pub fn key_len(&self) -> usize {
        self.key_ls.as_ref().map(|d| d.keys.len()).unwrap_or(0)
    }

    /// 选中钥匙的名字(删除确认 / 取回建议命令同源)。
    pub fn selected_key_name(&self) -> Option<String> {
        // #69:同 `selected_vps_name`——经过滤后的可见列表映射回原始投影。
        let sheet = self.catalog_sheet()?;
        let i = *self.catalog_visible().get(self.key_sel)?;
        sheet.entries.get(i).map(|e| e.name.clone())
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

    /// 钥匙落账闸(呈现口径):预审通过 = rc 0 且三道闸无 fail 且 data 可定型。
    pub fn key_can_commit(&self) -> bool {
        let Some(out) = self.key_flow.result.as_ref() else {
            return false;
        };
        catalog::preaudit_passed(out, catalog::KeySpec::audit(out).as_ref())
    }

    /// 钥匙册返航(#76 收敛后的转口:实现只有一份 [`Self::cancel`]——四本册同一句
    /// 「状态清空不留半条记录」,按册那点差别(在途本体随表单一起 drop)在
    /// [`crate::screens::CatalogKind`] 查表成员上)。测试直接驱动它。
    #[cfg(test)]
    pub fn cancel_key_register(&mut self) {
        self.cancel(crate::screens::CatalogKind::Key);
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

    // ---- #101 装配历史(只读;文件面一律经 datarepo 接缝) ----

    /// 安装根(NW_HOME)可用吗;`None` = 可用。
    ///
    /// 与 [`App::data_root_cause`] 并列的**第二条**三态门:数据仓根管
    /// 「五域读不读得到」,安装根管「清单存档读不读得到」。两者各有各的
    /// 说法,不共用一个空列表——「NW_HOME 指错了」与「一个派都还没装」
    /// 在屏上是两件事(规格故事 28)。
    pub fn home_cause(&self) -> Option<crate::datarepo::HomeCause> {
        crate::datarepo::home_cause(&self.nw_home)
    }

    /// 读一遍各派清单存档(#101)。
    ///
    /// 走 [`crate::datarepo`] 唯一接缝:枚举派 + 逐派读清单,屏幕层与
    /// app 层都不碰文件系统(规格故事 27)。**快照语义**:只在进视图 /
    /// 按 `r` 时读一次,不轮询、不自动刷。
    pub fn load_history(&mut self) {
        let rows = crate::datarepo::list_profiles(&self.nw_home)
            .into_iter()
            .map(|profile| {
                let status = crate::datarepo::read_manifest(&self.nw_home, &profile);
                crate::screens::HistoryRow { profile, status }
            })
            .collect();
        self.as_history_rows = Some(rows);
        self.as_history_sel = 0;
    }

    /// 历史视图的可见行(过滤后的下标,#101:纯本地筛选,与 #61 / #69
    /// 同一形态)。匹配面只收屏上真看得到的字段——引擎没给的字段不参与
    /// 匹配,不拿占位符「——」冒充可检索内容。
    pub fn history_visible(&self) -> Vec<usize> {
        let Some(rows) = self.as_history_rows.as_ref() else {
            return Vec::new();
        };
        let q = self.as_history_filter.trim().to_lowercase();
        (0..rows.len())
            .filter(|&i| {
                q.is_empty()
                    || history_row_fields(&rows[i])
                        .iter()
                        .any(|f| f.to_lowercase().contains(&q))
            })
            .collect()
    }

    /// 装配历史的总行数(过滤前的分母,与 #69 的「命中/总数」同口径)。
    pub fn history_total(&self) -> usize {
        self.as_history_rows.as_ref().map(|r| r.len()).unwrap_or(0)
    }

    /// 回车看清单原文(#101,规格故事 3)。
    ///
    /// 只有「认得也读通了」的那一派有原文可看;其余三种分类各给一句
    /// 人话(缺失 / 读不通 / 需升级 nw),不画一个空浮层假装有原文。
    fn show_history_raw(&mut self) {
        let Some(rows) = self.as_history_rows.as_ref() else {
            return;
        };
        let Some(&i) = self.history_visible().get(self.as_history_sel) else {
            return;
        };
        match &rows[i].status {
            crate::datarepo::ManifestStatus::Ok(a) => {
                self.as_history_raw = Some(a.raw.clone());
            }
            crate::datarepo::ManifestStatus::Missing => {
                self.push_log("这一派还没有清单(首次装配前),没有原文可看")
            }
            crate::datarepo::ManifestStatus::Broken { reason } => {
                self.push_log(format!("清单读不通,没有原文可看:{reason}"))
            }
            // 不认识的 schema 一律走契约守卫那句话:不猜、不按旧形状解析
            // (与引擎侧 `EngineOutput::evidence()` 对未知 envelope 的裁法同口径)
            crate::datarepo::ManifestStatus::UnknownSchema { guard } => {
                self.push_log(guard.message.clone())
            }
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
            // #104:发射即收仪式,对比快照不跨程(下一程的旧态是新的清单)
            self.as_ritual_diff = None;
        } else {
            self.push_log("没有选中的锁定集");
        }
    }

    /// 选中锁定集的载体文件路径:`$NW_HOME/locks/<lockId>.nwlock`
    /// (引擎 ls 的 locks 域与装配命令交锁段的同一落点;tilde 展开同输入链)。
    ///
    /// #101 起路径知识收进 [`crate::datarepo`]:本方法只剩转口,
    /// app 层不再自己拼 `$NW_HOME`(规格故事 27)。
    fn lock_carrier(&self, lock_id: &str) -> String {
        crate::datarepo::lock_carrier(&self.nw_home, lock_id)
    }

    // ---- 通用 ----

    pub fn next_screen(&mut self) {
        let i = (self.screen.index() + 1) % Screen::ALL.len();
        self.screen = Screen::ALL[i];
        self.rc_filter_on = false; // 切屏退过滤输入态(#61),过滤词留着回屏还在
        self.catalog_filter_on = false; // 编目册过滤同理(#69):退输入态、留词
        self.as_history_filter_on = false; // 装配历史过滤同理(#101)
        self.as_history_raw = None; // 原文浮层不跨屏(#101)
    }

    pub fn prev_screen(&mut self) {
        let i = (self.screen.index() + Screen::ALL.len() - 1) % Screen::ALL.len();
        self.screen = Screen::ALL[i];
        self.rc_filter_on = false;
        self.catalog_filter_on = false;
        self.as_history_filter_on = false;
        self.as_history_raw = None;
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

/// 装配历史一行里参与过滤的字段(#101):派名 + 屏上真看得见的那几个清单
/// 字段。引擎没给的字段不入匹配面——不猜、不拿占位符「——」冒充可检索
/// 内容(与 [`domain_row_fields`] 同一条纪律)。
fn history_row_fields(row: &crate::screens::HistoryRow) -> Vec<String> {
    let mut v = vec![row.profile.clone()];
    if let crate::datarepo::ManifestStatus::Ok(a) = &row.status {
        v.extend(a.brief.lock_id.clone());
        v.extend(a.brief.recipe.clone());
        v.extend(a.brief.installed_at.clone());
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

    // ---- #71 中止键 x 的正文动作段:点「中止」段 = 按 x 进程级真止 ----

    /// 点装配屏正文键位行的「中止」段 = 按 `x`(#71,补 #59 欠下的鼠标命中):
    /// 在途 `nw up` 子进程进程级真止(`FakeRunner::abort_count()` +1),面板回
    /// 锁定集选择器态;stderr 日志一行不清——那是操作者判断「停在哪一步」的
    /// 唯一依据,面板不宣布成功也不假装失败。与键盘 `x` 完全同一路
    /// (`abort_assembling`),鼠标只是叠加、零键盘回归,不新增语义。
    #[test]
    fn 点中止段_等于按x_进程级真止_日志一行不清() {
        let (runner, mut app) = up_inflight_app();
        app.size = Rect::new(0, 0, 80, 30);
        // 在途互斥不变:点中止前再按 Enter 不开发射仪式(忙时不排队)
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(!app.as_confirm, "在途时不接第二次发射(互斥语义不变)");
        let abort = hit_target(&app, |t| *t == Target::Abort);
        click(&mut app, abort);
        assert_eq!(
            runner.abort_count(),
            1,
            "点中止段确有一次到达运行器(进程级那一枪,与按 x 同路)"
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
    }

    /// 「中止」段常畅(80×30)与紧凑(66×50)两档都点得中;段文案由渲染与命中
    /// 共用同一份字符串(`assemble::assemble_key_parts`),点哪打哪——命中行在
    /// 整帧里必能找着「x 中止」(ADR-0005 渲染与命中同源,#67 接缝的同一形状;
    /// 规格故事 52:换终端不断命中)。
    #[test]
    fn 中止段_常畅与紧凑两档都点得中_且段文案在屏上在场() {
        for (w, h) in [(80u16, 30u16), (66, 50u16)] {
            let (runner, mut app) = up_inflight_app();
            app.size = Rect::new(0, 0, w, h);
            app.splash = false; // 命中区不看 splash、渲染看:留着它 frame_display 只画 logo(同既有鼠标测试纪律)
            let abort = hit_target(&app, |t| *t == Target::Abort);
            let frame = crate::testutil::frame_display(&app, w, h);
            let row = frame
                .split('\n')
                .nth(abort.y as usize)
                .expect("命中行在渲染中在场");
            assert!(row.contains("x 中止"), "{w}×{h} 键位行应画着中止段:{row}");
            click(&mut app, abort);
            assert_eq!(runner.abort_count(), 1, "{w}×{h} 点中止段进程级真止");
            assert!(!app.as_running, "{w}×{h} 在途标记收敛");
        }
    }

    /// 「中止」段只在真有 `up` 在途(`as_running`)时画得出、点得中(#71 票面 +
    /// 规格故事 47):不在场时不画也不给命中——点到一个当前不存在的动作,比点不到
    /// 更糟。在途标记一收敛,命中区与整帧的中止提示一并消失(块标题也不再带 `x`,
    /// 话术单源在键位行上)。
    #[test]
    fn 中止段_只在up在途在场_闲置不画不命中() {
        let (_r, mut app) = up_inflight_app();
        app.size = Rect::new(0, 0, 80, 30);
        app.splash = false; // 命中区不看 splash、渲染看:留着它 frame_display 只画 logo
                            // 真有 up 在途:画得出、点得中
        let abort = hit_target(&app, |t| *t == Target::Abort);
        let frame = crate::testutil::frame_display(&app, 80, 30);
        let row = frame
            .split('\n')
            .nth(abort.y as usize)
            .expect("命中行在渲染中在场");
        assert!(row.contains("x 中止"), "up 在途:中止段画在键位行:{row}");
        // 在途标记收敛(仍 assembling = 运行视图在场,但没有 up 在途):段不画、
        // 命中区也没有、整帧再无中止提示
        app.as_running = false;
        assert!(
            hitmap(app.size, &app, size_class(app.size))
                .iter()
                .all(|(_, t)| *t != Target::Abort),
            "非 up 在途:中止段不产命中区(点不到一个当前不存在的动作)"
        );
        let frame = crate::testutil::frame_display(&app, 80, 30);
        assert!(
            !frame.contains("x 中止"),
            "非 up 在途:整帧无中止提示(块标题不再带 x,话术单源在键位行):{frame}"
        );
    }

    /// 「中止」与「Esc 返航」是两个可点段、话术不互冒充(#71 票面 + 规格故事
    /// 46/48):发射前(仪式浮层)可点的是「Esc 返航」——命令还没下发,点什么
    /// 都什么都没发生(不碰运行器);发射后(运行视图)可点的是「x 中止」——在途
    /// 子进程进程级真止。两个状态各产各的命中区,不共屏、不共话术。
    #[test]
    fn 中止与返航两段可点_话术不互冒充() {
        // 发射前:仪式浮层只产「Enter 发射 / Esc 返航」两段,没有「中止」段
        let (runner, mut app) = up_inflight_app();
        app.size = Rect::new(0, 0, 80, 30);
        app.assembling = false; // 仪式在发射前:命令还没下发
        app.as_running = false;
        app.as_confirm = true;
        let hits = hitmap(app.size, &app, size_class(app.size));
        assert!(
            hits.iter().any(|(_, t)| *t == Target::DismissPopup),
            "发射前:「Esc 返航」段可点"
        );
        assert!(
            hits.iter().all(|(_, t)| *t != Target::Abort),
            "发射前:没有「中止」段(命令还没下发,没有进程可止)"
        );
        let back = hit_target(&app, |t| *t == Target::DismissPopup);
        click(&mut app, back);
        assert_eq!(runner.abort_count(), 0, "点返航 = 取消仪式,什么都没发生");
        assert!(!app.as_confirm, "返航关仪式");
        // 发射后:运行视图只产「x 中止」段,没有「返航」段(浮层没开)
        let (runner2, mut app2) = up_inflight_app();
        app2.size = Rect::new(0, 0, 80, 30);
        let hits2 = hitmap(app2.size, &app2, size_class(app2.size));
        assert!(
            hits2.iter().any(|(_, t)| *t == Target::Abort),
            "发射后:「x 中止」段可点"
        );
        assert!(
            hits2.iter().all(|(_, t)| *t != Target::DismissPopup),
            "发射后:没有「返航」段(浮层没开,返航归发射前)"
        );
        let abort = hit_target(&app2, |t| *t == Target::Abort);
        click(&mut app2, abort);
        assert_eq!(runner2.abort_count(), 1, "点中止 = 进程级真止");
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

    // ---- #108 引擎版本对照(数据面在 release 模块,这里只钉 App 行为缝) ----

    /// 对照用的就绪态:注入回放式查询(零网络),本地版本给定。
    fn release_app(script: Vec<crate::release::ReleaseProbe>) -> App {
        let mut app = settings_app();
        app.engine_version = "0.1.0".into();
        app.release_lookup = Arc::new(crate::release::FakeReleaseLookup::new(script));
        app
    }

    #[tokio::test]
    async fn 版本对照_打开设置屏才发起一次查询_结论一到就锁latch() {
        let mut app = release_app(vec![crate::release::ReleaseProbe::Latest("v0.2.0".into())]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        // 打开设置屏:主循环每帧调这个,这里连调三次模拟三帧
        app.ensure_release_probe(&tx);
        app.ensure_release_probe(&tx);
        app.ensure_release_probe(&tx);
        let probe = rx.recv().await.expect("应有一次查询回音");
        assert_eq!(
            probe,
            crate::release::ReleaseProbe::Latest("v0.2.0".into()),
            "回音是拿到的 tag"
        );
        assert!(rx.try_recv().is_err(), "只发起一次,不轮询");
        // 结论落账后 latch 锁上:再调也不该发起第二次
        app.update(Message::Release(probe));
        app.ensure_release_probe(&tx);
        app.ensure_release_probe(&tx);
        assert!(rx.try_recv().is_err(), "结论一到就锁 latch,不轮询");
    }

    #[tokio::test]
    async fn 版本对照_不在设置屏不发起查询() {
        let mut app = release_app(vec![crate::release::ReleaseProbe::Latest("v0.2.0".into())]);
        app.screen = Screen::Overview;
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.ensure_release_probe(&tx);
        // 给在途任务一个调度机会,再断言什么都没发生
        tokio::task::yield_now().await;
        assert!(rx.try_recv().is_err(), "概览屏不该对照 release 仓");
    }

    #[tokio::test]
    async fn 版本对照_按r重查才清结论再问一次() {
        let mut app = release_app(vec![
            crate::release::ReleaseProbe::Latest("v0.2.0".into()),
            crate::release::ReleaseProbe::Unreachable,
        ]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.ensure_release_probe(&tx);
        app.update(Message::Release(rx.recv().await.unwrap()));
        assert!(app.release_remote.is_some(), "首次结论已入账");
        // 先按 v 翻开对照页(r 只在该页上有分支),再按 r 清结论
        app.update(Message::Key(key('v')));
        app.update(Message::Key(key('r')));
        assert!(app.release_remote.is_none(), "r 清掉旧结论");
        app.ensure_release_probe(&tx);
        assert_eq!(
            rx.recv().await.unwrap(),
            crate::release::ReleaseProbe::Unreachable,
            "重查走的是同一条缝"
        );
    }

    #[test]
    fn 版本对照_落后时建议命令只进日志区_不代跑() {
        let mut app = release_app(vec![]);
        app.update(Message::Release(crate::release::ReleaseProbe::Latest(
            "v0.2.0".into(),
        )));
        let hint = crate::release::upgrade_hint();
        assert!(
            app.log.iter().any(|l| l.contains(hint)),
            "升级指路进日志区:{:?}",
            app.log
        );
        assert!(
            app.log.iter().any(|l| l.contains("不代跑")),
            "指路带「不代跑」的口径:{:?}",
            app.log
        );
        // 只给文案:没有任何东西被排进引擎队列
        assert!(app.queue.is_empty(), "不排引擎命令:{:?}", app.queue);
        assert!(app.fed_queue.is_empty(), "不排喂入命令:{:?}", app.fed_queue);
    }

    #[test]
    fn 版本对照_本地版本后到也补判落后() {
        // 真实时序:release 常比 `--version` 先到,本地还是「探测中」
        let mut app = release_app(vec![]);
        app.engine_version = "探测中".into();
        app.update(Message::Release(crate::release::ReleaseProbe::Latest(
            "v0.2.0".into(),
        )));
        let hint = crate::release::upgrade_hint();
        assert!(
            !app.log.iter().any(|l| l.contains(hint)),
            "本地还没探到版本时不瞎判落后:{:?}",
            app.log
        );
        // 版本回音到达:补判一次,指路不遗漏
        let out = EngineOutput {
            args: vec!["--version".into()],
            exit_code: crate::engine::ExitCode::Ok,
            ok: true,
            stderr_tail: vec!["0.1.0".into()],
            envelope: None,
            raw_stdout: String::new(),
            parse_error: None,
        };
        app.update(Message::Engine(EngineEvent::Finished(Box::new(out))));
        assert_eq!(app.engine_version, "0.1.0", "版本入账");
        assert!(
            app.log.iter().any(|l| l.contains(hint)),
            "版本后到也要补指路:{:?}",
            app.log
        );
    }

    #[test]
    fn 版本对照_取不到远端时降级为只显示本地_两种理由分开说() {
        for (probe, needle) in [
            (
                crate::release::ReleaseProbe::Unreachable,
                "取不到引擎 release 仓",
            ),
            (crate::release::ReleaseProbe::Garbage, "回话不是可用 tag"),
        ] {
            let mut app = release_app(vec![]);
            app.update(Message::Release(probe));
            assert!(
                app.log.iter().any(|l| l.contains(needle)),
                "{needle} 该出现在日志里:{:?}",
                app.log
            );
            assert!(
                !app.log
                    .iter()
                    .any(|l| l.contains(crate::release::upgrade_hint())),
                "取不到远端就不该给升级指路:{:?}",
                app.log
            );
        }
    }

    #[test]
    fn 版本对照_v键翻开对照页_esc返回_且不排引擎命令() {
        let mut app = release_app(vec![]);
        app.update(Message::Key(key('v')));
        assert_eq!(
            app.settings_page,
            crate::app::SettingsPage::Version,
            "v 翻开版本对照页"
        );
        assert!(app.queue.is_empty(), "翻页不排引擎命令:{:?}", app.queue);
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert_eq!(
            app.settings_page,
            crate::app::SettingsPage::Panel,
            "Esc 从对照页返航"
        );
        app.update(Message::Key(key('v')));
        app.update(Message::Key(key('v')));
        assert_eq!(
            app.settings_page,
            crate::app::SettingsPage::Panel,
            "再按 v 也回常驻面板"
        );
    }

    #[test]
    fn 版本对照_引擎没找到时v键不翻页() {
        let mut app = release_app(vec![]);
        app.engine.found = false;
        app.update(Message::Key(key('v')));
        assert_eq!(
            app.settings_page,
            crate::app::SettingsPage::Panel,
            "没有本地版本可比,对照页不登场"
        );
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

    // ---- #68 概览指令栏第五条「浏览编目」:一键到第六屏,点它也等于按键 ----

    /// 在整帧真显示里定位某子串的终端 (列, 行)(列按显示宽累计,与命中区
    /// 同一把尺;frame_display 已跳过宽字续格,故字节偏移≠列,须累宽)。
    fn locate_col_row(app: &App, w: u16, h: u16, needle: &str) -> (u16, u16) {
        use unicode_width::UnicodeWidthChar;
        let frame = crate::testutil::frame_display(app, w, h);
        for (y, line) in frame.split('\n').enumerate() {
            if let Some(byte) = line.find(needle) {
                let col: u16 = line[..byte]
                    .chars()
                    .map(|c| UnicodeWidthChar::width(c).unwrap_or(0) as u16)
                    .sum();
                return (col, y as u16);
            }
        }
        panic!("整帧里找不到「{needle}」({w}×{h})");
    }

    /// 键盘路:概览按 `v` 一键到第六屏(编目),且纯导航零引擎调用、不写流水。
    /// 键位选 `v`(屏枚举 `Screen::Vps` 首字母)与既有 `a`/`h`/`l` 跳屏同构,
    /// 全仓无 `Char('v')` 冲突;指令栏 / 帮助 / README 三处同写 `v`
    /// (屏上=按下,#68 故事 5)。HUD 不写它:壳级提示栏只写 j/k、Enter、t、?、q。
    #[test]
    fn 概览_按v跳到编目屏_零引擎调用() {
        let mut app = fake_app();
        app.screen = Screen::Overview;
        let (q, log) = (app.queue.len(), app.log.len());
        app.update(Message::Key(key('v')));
        assert_eq!(app.screen, Screen::Vps, "v 一键到第六屏(编目,屏内 m 换册)");
        assert_eq!(app.queue.len(), q, "切屏零引擎调用:队列不长");
        assert_eq!(app.log.len(), log, "纯导航不写流水");
    }

    /// 点路 == 键路:点帧上「浏览编目」所在行列 = 按 `v`,两档都中、都零调用。
    /// 坐标取自整帧真显示(点哪打哪),不硬编码矩形——紧凑态 Tab 行让首行、
    /// 竖导航列还正文,几何两支都由渲染同源算出(ADR-0005)。
    #[test]
    fn 概览_点浏览编目等于按v_常畅与紧凑两档都中_且不发调用() {
        for (w, h) in [(80u16, 30u16), (66, 50)] {
            let mut by_key = mouse_ls_app();
            by_key.screen = Screen::Overview;
            by_key.update(Message::Key(key('v')));

            let mut by_click = mouse_ls_app();
            by_click.screen = Screen::Overview;
            by_click.splash = false;
            by_click.size = Rect::new(0, 0, w, h);
            let (col, row) = locate_col_row(&by_click, w, h, "浏览编目");
            let q = by_click.queue.len();
            by_click.update(Message::Mouse(mouse_at(
                MouseEventKind::Down(MouseButton::Left),
                col,
                row,
            )));
            assert_eq!(
                by_click.screen,
                Screen::Vps,
                "{w}×{h} 点指令栏第五条到编目屏"
            );
            assert_eq!(
                by_click.screen, by_key.screen,
                "{w}×{h} 点它 = 按 v(同一后果)"
            );
            assert_eq!(by_click.queue.len(), q, "{w}×{h} 点它零引擎调用");
        }
    }

    /// 入口常驻可点:指令栏是动线不是能力表——数据仓根未设也不在指令栏做
    /// 禁用态(#68 明确不做 gate);按册的能力门在编目屏内各自指路,语义不变。
    #[test]
    fn 概览_浏览编目入口常驻可点_不设数据仓根也到第六屏() {
        let mut app = mouse_ls_app();
        app.screen = Screen::Overview;
        app.data_root = None; // 未设数据仓根
        app.splash = false;
        app.size = Rect::new(0, 0, 66, 50);
        let (col, row) = locate_col_row(&app, 66, 50, "浏览编目");
        app.update(Message::Mouse(mouse_at(
            MouseEventKind::Down(MouseButton::Left),
            col,
            row,
        )));
        assert_eq!(app.screen, Screen::Vps, "入口常驻可点:坏根也不禁用指令栏");
        assert!(app.queue.is_empty(), "切屏零引擎调用(门禁在编目屏内按册给)");
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

    // ---- 编目册屏四本册:鼠标点行 / 滚轮移选中走册级查表(#54 起) ----

    /// 四本册的 ls 投影(数据面形状,与 `screens/catalog.rs` 测试同源):
    /// 每本 3 条,才好断言「点第 3 行 = 该册 sel 2」。
    const CAT_VPS_LS: &str = r#"{"catalogPath":"/r/vps.json","machines":[{"name":"gate","ips":[],"port":22,"logins":[]},{"name":"bare","ips":[],"port":0,"logins":[]},{"name":"zen","ips":[],"port":0,"logins":[]}]}"#;
    const CAT_TAG_LS: &str = r#"{"schema":"nw/tag-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/tags.json","tags":[{"name":"pi","refs":[{"kind":"skill","id":"o/r/deep-research"}]},{"name":"bare","refs":[]},{"name":"ops","refs":[]}]}}"#;
    const CAT_SUB_LS: &str = r#"{"catalogPath":"/r/subscriptions.json","subscriptions":[{"name":"airport","kind":"subscription-url","client":"Mihomo","expires":"2099-03-01","status":"active","daysLeft":26000},{"name":"soon","kind":"subscription-url","client":"Clash","expires":"2099-03-04","status":"expiring","daysLeft":3},{"name":"forever","kind":"subscription-url","client":"Surge","expires":"","status":"active","daysLeft":null}]}"#;
    const CAT_KEY_LS: &str = r#"{"catalogPath":"/r/keys.json","keys":[{"name":"ci","kind":"github-key","host":"ghe.internal","scopes":[],"rotatedAt":"2025-11-02"},{"name":"deploy","kind":"github-key","host":"github.com","scopes":["repo"],"rotatedAt":"2026-01-15"},{"name":"ops","kind":"github-key","host":"gitlab.com","scopes":[],"rotatedAt":"2026-02-01"}]}"#;

    /// 编目册屏四本册各灌 3 条投影(点行/滚轮才有命中区可点)。
    fn catalog_four_books_app() -> App {
        let mut app = fake_app();
        app.size = Rect::new(0, 0, 80, 30);
        // splash 必须先跳过:命中区不看 splash、渲染看——留着它会「点得到、
        // 屏上没有」,正是 ADR-0005「渲染与命中同源」要防的那类分裂(#67)。
        app.splash = false;
        app.screen = Screen::Vps;
        app.vps_ls = Some(serde_json::from_str(CAT_VPS_LS).unwrap());
        app.tag_ls_echo = Some(EngineOutput {
            args: vec!["tag".into(), "ls".into()],
            exit_code: crate::engine::ExitCode::Ok,
            ok: true,
            stderr_tail: vec![],
            envelope: Some(serde_json::from_str(CAT_TAG_LS).unwrap()),
            raw_stdout: CAT_TAG_LS.into(),
            parse_error: None,
        });
        app.sub_ls = Some(serde_json::from_str(CAT_SUB_LS).unwrap());
        app.key_ls = Some(serde_json::from_str(CAT_KEY_LS).unwrap());
        // 四本册的能力都记上:命中区与渲染用同一判据(`CatalogKind::capability`),
        // 不记就会「册体一行不画、行命中区照产」——点得到、屏上没有,正是
        // ADR-0005「渲染与命中同源」要防的那类分裂。
        for cap in [
            crate::engine::Capability::Vps,
            crate::engine::Capability::Tag,
            crate::engine::Capability::Subscription,
            crate::engine::Capability::Key,
        ] {
            app.capabilities
                .record(cap, crate::engine::Probe::Available);
        }
        app
    }

    /// 册体文案承诺「↑/↓ 或鼠标点行」(ADR-0005 V1 交互范围):鼠标点行与
    /// 滚轮移选中对**四本册**都落在各自的 `*_sel` 上——订阅册曾落进
    /// `Screen::Vps` 兜底写 `vps_sel`,册体选中永不随鼠标动(双轴评审
    /// 发现 1),这条守四本册各自不串味。
    #[test]
    fn 编目册四本册_鼠标点行与滚轮移选中_各动各的选中槽() {
        let mut app = catalog_four_books_app();
        for kind in [
            catalog::CatalogKind::Vps,
            catalog::CatalogKind::Tag,
            catalog::CatalogKind::Subscription,
            catalog::CatalogKind::Key,
        ] {
            app.catalog_kind = kind;
            // 每本册都从全零选中态起步(上一本册的滚轮余量不许带进来)
            app.vps_sel = 0;
            app.tag_sel = 0;
            app.sub_sel = 0;
            app.key_sel = 0;
            assert_eq!(app.catalog_len(), 3, "{kind:?} 册应有 3 条投影");
            // 鼠标点第 3 行 = 该册 sel 2
            let row = hit_target(&app, |t| matches!(t, Target::SelectRow(2)));
            click(&mut app, row);
            assert_eq!(app.catalog_sel(), 2, "{kind:?} 册点行选中");
            assert_eq!(
                (app.vps_sel, app.tag_sel, app.sub_sel, app.key_sel),
                match kind {
                    catalog::CatalogKind::Vps => (2, 0, 0, 0),
                    catalog::CatalogKind::Tag => (0, 2, 0, 0),
                    catalog::CatalogKind::Subscription => (0, 0, 2, 0),
                    catalog::CatalogKind::Key => (0, 0, 0, 2),
                },
                "{kind:?} 册点行只动自己的选中槽,四本册不串味"
            );
            // 滚轮在册体行上移一档(与 ↑/↓ 同语义、钳位不绕回)
            let row = hit_target(&app, |t| matches!(t, Target::SelectRow(_)));
            wheel(&mut app, row, MouseEventKind::ScrollUp);
            assert_eq!(app.catalog_sel(), 1, "{kind:?} 册滚轮上移一档");
        }
        // 订阅册专项:点行/滚轮都不再误写 vps_sel(发现 1 的回归形状)
        app.catalog_kind = catalog::CatalogKind::Subscription;
        let row = hit_target(&app, |t| matches!(t, Target::SelectRow(1)));
        click(&mut app, row);
        assert_eq!(app.sub_sel, 1, "订阅册点行动 sub_sel");
        assert_eq!(app.vps_sel, 0, "订阅册点行不误写 vps_sel");
        wheel(&mut app, row, MouseEventKind::ScrollDown);
        assert_eq!(app.sub_sel, 2, "订阅册滚轮动 sub_sel");
        assert_eq!(app.vps_sel, 0, "订阅册滚轮不误写 vps_sel");
    }

    // ---- #94 工单 #105:点概览警示带的订阅到期条目直跳订阅册 ----

    /// 点警示带上的到期订阅 = 跳到订阅册并选中那一条(#105 用户故事 12)。
    /// 三步与键盘 `v` 到编目屏同族(定册内维度 → 切屏 → 落册级选中槽),选中落在
    /// **可见行**上(#69 过滤的口径);`CAT_SUB_LS` 按名排序后 soon 在投影第 3 条
    /// (下标 2),无过滤时可见位置就是 2。两档都点得中,全程零引擎调用——回音是
    /// 既有的 `sub_ls`,面板只换视图。
    #[test]
    fn 概览_点警示带到期订阅_跳到订阅册并选中那一条_两档零调用() {
        for (w, h) in [(80u16, 30u16), (66, 50)] {
            let mut app = catalog_four_books_app();
            app.screen = Screen::Overview;
            app.size = Rect::new(0, 0, w, h);
            let entry = hit_target(&app, |t| matches!(t, Target::AlertSubEntry(2)));
            click(&mut app, entry);
            assert_eq!(app.screen, Screen::Vps, "{w}×{h} 跳到编目屏");
            assert_eq!(
                app.catalog_kind,
                catalog::CatalogKind::Subscription,
                "{w}×{h} 屏内维度落到订阅册"
            );
            assert_eq!(app.sub_sel, 2, "{w}×{h} 选中 soon 那一条");
            assert_eq!(
                app.catalog_sel(),
                2,
                "{w}×{h} 册级查表读到的就是订阅册的选中槽"
            );
            assert!(app.queue.is_empty(), "{w}×{h} 零引擎调用:只换视图");
            assert!(
                app.log.iter().any(|l| l.contains("soon")),
                "{w}×{h} 日志记下选中了哪一条:{:?}",
                app.log
            );
        }
    }

    /// 册内过滤把这一条筛掉时(#69):点击照旧发生,但不落到一个看不见的选中位上
    /// ——复现册体「无选中行时点下去给指路」的既有口径,不新造禁用态。
    #[test]
    fn 概览_点警示带到期订阅_被册内过滤筛掉时给指路不硬选中() {
        let mut app = catalog_four_books_app();
        app.screen = Screen::Overview;
        app.size = Rect::new(0, 0, 80, 30);
        // 订阅册的过滤词筛掉 soon(只剩 airport / forever 两条 active)
        app.catalog_filter[catalog::CatalogKind::Subscription.idx()] = "airport".into();
        let entry = hit_target(&app, |t| matches!(t, Target::AlertSubEntry(2)));
        click(&mut app, entry);
        assert_eq!(app.screen, Screen::Vps, "仍然跳到订阅册");
        assert_eq!(app.catalog_kind, catalog::CatalogKind::Subscription);
        assert_eq!(
            app.catalog_sel(),
            0,
            "可见行里没有它:选中钳在可见范围内(2 条可见 → 0),不落在看不见的行上"
        );
        assert!(
            app.log
                .iter()
                .any(|l| l.contains("soon") && l.contains("过滤")),
            "指路说清它被过滤词挡住了:{:?}",
            app.log
        );
        assert!(app.queue.is_empty(), "零引擎调用");
    }

    /// 点哪打哪(ADR-0005):命中区不是另推一份几何——坐标取自整帧真显示里
    /// `soon` 所在的那一列(警示带第一行),点下去落在订阅册并选中它。
    #[test]
    fn 概览_点警示带上soon所在列_整帧定位也中_落到订阅册选中它() {
        let mut app = catalog_four_books_app();
        app.screen = Screen::Overview;
        app.splash = false;
        app.size = Rect::new(0, 0, 80, 30);
        let (col, row) = locate_col_row(&app, 80, 30, "soon");
        app.update(Message::Mouse(mouse_at(
            MouseEventKind::Down(MouseButton::Left),
            col,
            row,
        )));
        assert_eq!(app.screen, Screen::Vps, "整帧定位点下去也跳订阅册");
        assert_eq!(app.catalog_kind, catalog::CatalogKind::Subscription);
        assert_eq!(app.sub_sel, 2, "选中 soon 那一条");
        assert!(app.queue.is_empty(), "零引擎调用");
    }

    // ---- #67 正文键位行命中接缝:点「换册」段 = 按 m ----

    /// 点正文键位行的「换册」段 = 按 `m`:四本册按 `CatalogKind::next()` 既有
    /// 顺序轮转(鼠标只是叠加,不新增语义,#67 承载的规格订正:可点动作段画在
    /// **正文里**,不画在 HUD)。段文案由渲染与命中共用同一份字符串——点哪打哪。
    #[test]
    fn 点换册段_等于按m_四本册按既有顺序轮转() {
        let mut app = catalog_four_books_app();
        app.size = Rect::new(0, 0, 80, 30);
        let mut by_click = vec![app.catalog_kind];
        for _ in 0..4 {
            let sw = hit_target(&app, |t| *t == Target::SwitchBook);
            click(&mut app, sw);
            by_click.push(app.catalog_kind);
        }
        // 与连按四次 m 同一条轮转链(Vps → Tag → Subscription → Key → Vps)
        let mut keyed = catalog_four_books_app();
        keyed.size = Rect::new(0, 0, 80, 30);
        let mut by_key = vec![keyed.catalog_kind];
        for _ in 0..4 {
            keyed.update(Message::Key(key('m')));
            by_key.push(keyed.catalog_kind);
        }
        assert_eq!(by_click, by_key, "点换册段与按 m 逐帧同源");
        assert_eq!(
            by_click,
            vec![
                catalog::CatalogKind::Vps,
                catalog::CatalogKind::Tag,
                catalog::CatalogKind::Subscription,
                catalog::CatalogKind::Key,
                catalog::CatalogKind::Vps,
            ],
            "四本册按 next() 既有顺序轮转,不另造一套"
        );
    }

    /// 「换册」段常畅(80×30)与紧凑(66×50)两档都点得中;段文案在屏上逐字在场
    /// (渲染与命中共用同一份字符串量宽,点哪打哪)。
    #[test]
    fn 换册段_常畅与紧凑两档都点得中_且段文案在屏上在场() {
        for (w, h) in [(80u16, 30u16), (66, 50u16)] {
            let mut app = catalog_four_books_app();
            app.size = Rect::new(0, 0, w, h);
            let sw = hit_target(&app, |t| *t == Target::SwitchBook);
            // 段文案与命中共用:渲染行里必能找着这段字符串,且它的起始列 = 命中矩形 x
            let frame = crate::testutil::frame_display(&app, w, h);
            let row = frame
                .split('\n')
                .nth(sw.y as usize)
                .expect("命中行在渲染中在场");
            assert!(row.contains("m 换册"), "{w}×{h} 键位行应画着换册段:{row}");
            click(&mut app, sw);
            assert_eq!(
                app.catalog_kind,
                catalog::CatalogKind::Tag,
                "{w}×{h} 点换册段切到下一本"
            );
        }
    }

    /// 浮层打开时点底层不换册(spec #65 故事 50:浮层吞底层点击,弹层优先级);
    /// 删除确认弹层在场时「换册」段不产命中区。
    #[test]
    fn 换册段_删除确认弹层打开时_点底层不换册() {
        let mut app = catalog_four_books_app();
        app.size = Rect::new(0, 0, 80, 30);
        app.vps_rm_confirm = true;
        let hits = hitmap(app.size, &app, size_class(app.size));
        assert!(
            hits.iter().all(|(_, t)| *t != Target::SwitchBook),
            "删除确认弹层打开:换册段不产命中区(弹层优先级,底层不穿透)"
        );
    }

    /// 点顶栏 / HUD 行零状态变化(#16 钉住:HUD 是提示栏,永远不可点)。
    ///
    /// 快照字段刻意带上**编目册本轮新增的三个状态**(#65 评审第 10 条):
    /// `catalog_kind` / `catalog_filter_on` / `vps_reverse`。它们在结构上今天
    /// 不可能被这一下改到(`area_pad` 先把 HUD 整行从命中区切掉,任何 `*_hits`
    /// 都在切过的区域上跑),但正因为切得干净,旧的快照元组**结构上就看不见
    /// 编目屏上的回归**——而这正是本轮长出三个可点正文段的地方。带上这三个
    /// 字段,将来看不见的回归才会真的红。
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
            app.catalog_kind,
            app.catalog_filter_on,
            app.vps_reverse.clone(),
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
                app.catalog_kind,
                app.catalog_filter_on,
                app.vps_reverse.clone(),
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
                "{cap:?} 探针必须经 spawn_engine 下发"
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
            assert!(app2.entry_enabled(cap), "齐备引擎下 {cap:?} 应放行");
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
    /// 手填 / 配方过滤输入态 / 编辑器 / 鼠标 / NW_HOME / 日志长度 / 队列长度)。
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
        rc_filter_on: bool,
        rc_filter: String,
        catalog_filter_on: bool,
        catalog_filter: [String; 4],
        as_history: bool,
        as_history_filter_on: bool,
        as_history_raw_on: bool,
        pending_editor: bool,
        mouse_enabled: bool,
        settings_page: SettingsPage,
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
            rc_filter_on: a.rc_filter_on,
            rc_filter: a.rc_filter.clone(),
            catalog_filter_on: a.catalog_filter_on,
            catalog_filter: a.catalog_filter.clone(),
            as_history: a.as_history,
            as_history_filter_on: a.as_history_filter_on,
            as_history_raw_on: a.as_history_raw.is_some(),
            pending_editor: a.pending_editor,
            mouse_enabled: a.mouse_enabled,
            settings_page: a.settings_page,
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
            if row.prep == Prep::HistoryView {
                // 装配历史次级视图(#101):`/` 过滤与 `r` 重读只在这个上下文
                // 里有分支,帮助对照表因此要能把它布出来(与 `running` 旗同级)。
                app.screen = Screen::Assemble;
                app.as_history = true;
                app.load_history();
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

    /// README 六屏表的中文屏名 ↔ `Screen`(键位对齐测试的地砖)。
    fn readme_screen_cn(s: Screen) -> &'static str {
        match s {
            Screen::Overview => "概览",
            Screen::Assemble => "装配",
            Screen::Health => "健康",
            Screen::Recipes => "配方",
            Screen::Vps => "编目",
            Screen::Settings => "设置",
        }
    }

    /// 键位字面量切碎:「a / h / l」「↑/↓」→ 单个键;`/` 保持原样。
    fn key_tokens(key: &str) -> Vec<String> {
        key.split('/')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect()
    }

    /// 取 README 六屏表里某一屏的那一行(行首是屏名)。
    fn readme_row_for(readme: &str, cn: &str) -> String {
        readme
            .lines()
            .find(|l| l.trim_start().starts_with('|') && l.contains(cn))
            .unwrap_or_else(|| panic!("README 六屏表应有「{cn}」行"))
            .to_string()
    }

    /// 守「README 键位表与帮助浮层自此一致,不再互相漏」(#62 验收钉,
    /// 双轴评审发现 3):方向一,`HELP_ROWS` 每一行的键都写进 README 对应屏
    /// 那一行(帮助不写 README 没有的键);方向二,README 六屏表每一行的键列
    /// 列出的键都在 `HELP_ROWS` 里(README 不写帮助里没有的键)。
    ///
    /// 有意不追的边界(注释钉住,别静默扩大):方向一只对**带屏**的行按屏
    /// 比对,全局行(j/k、↑/↓、Esc、t、?)在「通用行 + 六屏键列」里找;
    /// 方向二只扫六屏表的键列——README「通用」行里的 `q`(退出)与鼠标章节
    /// 不在帮助对照表里,那是既有取舍(浮层不教退出),不进豁免清单。
    #[test]
    fn readme_键位表与帮助浮层互不漏() {
        use crate::ui::HELP_ROWS;
        let readme = include_str!("../README.md");
        let help_keys: Vec<String> = HELP_ROWS.iter().flat_map(|r| key_tokens(r.key)).collect();
        // 方向一:帮助列出的每个键,README 键位表都找得到
        for row in HELP_ROWS {
            let hay = match row.screen {
                Some(s) => readme_row_for(readme, readme_screen_cn(s)).replace('`', ""),
                // 全局键:通用行 + 六屏键列(不放大到整篇,免得单字母被
                // 英文正文里的同名字母「凑中」)
                None => {
                    let mut h = readme
                        .lines()
                        .find(|l| l.starts_with("通用:"))
                        .expect("README 应有通用键位行")
                        .replace('`', "");
                    for s in Screen::ALL {
                        h.push_str(&readme_row_for(readme, readme_screen_cn(s)).replace('`', ""));
                    }
                    h
                }
            };
            for t in key_tokens(row.key) {
                assert!(
                    hay.contains(&t),
                    "帮助列的键 `{}`({})应写进 README 键位表",
                    row.key,
                    row.desc
                );
            }
        }
        // 方向二:README 六屏表每行键列列出的键,帮助对照表里都有
        for s in Screen::ALL {
            let line = readme_row_for(readme, readme_screen_cn(s));
            let cells: Vec<&str> = line.split('|').collect();
            let key_cell = cells.get(3).expect("六屏表第三列是键列");
            // 反引号包裹的才是键位字面量(`` `r` `` / `` `↑/↓` ``)
            for tok in key_cell.split('`').skip(1).step_by(2) {
                for t in key_tokens(tok) {
                    assert!(
                        help_keys.iter().any(|k| k == &t),
                        "README {s:?} 屏键列的 `{tok}` 应在 HELP_ROWS 里"
                    );
                }
            }
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
    ///
    /// **反方向也钉(#65 评审第 4 条)**:册体画得出来的每一个**可点段**,章节
    /// 与 README 鼠标小节两边都点名。旧版只查了「README 有的短语在不在章节
    /// 里」这一个方向,于是 #69 给机器册加了 `/ 过滤` 段、#70 加了
    /// `Enter 反查` 段,README 还停在「换册 / Enter 反查」两段,两边悄悄不
    /// 一致也没人红。段名词从**画段的那几份常量**取
    /// ([`crate::screens::catalog::CATALOG_SWITCH`] / `CATALOG_FILTER` /
    /// `CATALOG_REVERSE` / `CATALOG_GET` / `CATALOG_DELETE` /
    /// `CATALOG_DELETE_TAG`——`catalog_key_parts` 里全部 `Some(Target)` 的段都
    /// 是它们),不另抄一份清单:将来加新的可点段而文档没跟上,这条就红。
    #[test]
    fn 帮助鼠标章节_覆盖_readme_全部条目() {
        use crate::screens::catalog::{
            CATALOG_DELETE, CATALOG_DELETE_TAG, CATALOG_FILTER, CATALOG_GET, CATALOG_REVERSE,
            CATALOG_SWITCH,
        };
        use crate::ui::MOUSE_LINES;
        let readme = include_str!("../README.md");
        // README 鼠标小节:从 `### 鼠标(默认开捕获)` 到下一个 `##` 标题之间
        let section = readme
            .split("### 鼠标(默认开捕获)")
            .nth(1)
            .and_then(|rest| rest.split("\n## ").next())
            .expect("README 应有「鼠标(默认开捕获)」小节");
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
            "编目",
            "过滤",
        ] {
            assert!(joined.contains(phrase), "鼠标章节缺 README 条目 `{phrase}`");
        }
        // 反方向:画在屏上的每一个可点段,章节与 README 两边都点名
        for seg in [
            CATALOG_SWITCH,
            CATALOG_FILTER,
            CATALOG_REVERSE,
            CATALOG_GET,
            CATALOG_DELETE,
            CATALOG_DELETE_TAG,
        ] {
            // 段文案是 `m 换册` / `/ 过滤` / `Enter 反查`:名词取最后一段
            let noun = seg.split_whitespace().last().unwrap_or(seg);
            assert!(
                joined.contains(noun),
                "鼠标章节漏了可点段 `{seg}`(册体画得出来,章节却没写)"
            );
            assert!(
                section.contains(noun),
                "README 鼠标小节漏了可点段 `{seg}`(屏上点得中,README 却没写)"
            );
        }
    }

    // ---- #101 装配历史次级视图(只读) ----

    /// 清单 fixture:字段与取值照引擎真样本
    /// (`docs/real-first-run/envelopes/doctor-profile.json` 的 `data.manifest`)。
    const HISTORY_MANIFEST_OK: &str = r#"{"schema":"nw/manifest.v1","lockId":"research-desk-1.0.0-360ae68","profile":"research-desk","recipe":"research-desk","version":"1.0.0","runtimeKey":"node-22.19.0+pi-1.0.2","installedAt":"2026-10-09T03:00:07Z","convergeHash":"3cf6a584a523b35ec42630766a9eb6a84d3dcf11d2347d5aa3f3ae879b420502","engine":"0.1.0"}"#;

    /// fixture 安装根:一部装好的、一部清单写坏的、一部还没装的。
    fn history_home(tag: &str) -> std::path::PathBuf {
        // 每个用例一座目录:测试并行跑,共用一个路径会互相 remove_dir_all。
        let dir = std::env::temp_dir().join(format!("nwt-hist-app-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (p, text) in [
            ("research-desk", HISTORY_MANIFEST_OK),
            ("half", "{\"schema\":\"nw/manifest.v1\",\"lockId\""),
            ("fresh", ""),
        ] {
            let d = dir.join("profiles").join(p);
            std::fs::create_dir_all(&d).unwrap();
            if !text.is_empty() {
                std::fs::write(d.join(".nw-manifest"), text).unwrap();
            }
        }
        dir
    }

    fn history_app(tag: &str) -> App {
        let mut app = fake_app();
        app.screen = Screen::Assemble;
        app.nw_home = history_home(tag).display().to_string();
        app.as_history = true;
        app.load_history();
        app
    }

    /// 进视图即读盘,且**零引擎调用**(#101:这是读文件的接缝,不是新命令面)。
    #[test]
    fn 装配历史_按h进出_读各派存档_零引擎调用() {
        let mut app = fake_app();
        app.screen = Screen::Assemble;
        app.nw_home = history_home("hkey").display().to_string();
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )));
        assert!(app.as_history, "h 进视图");
        assert_eq!(app.history_total(), 3, "三个派目录都读到");
        assert!(app.queue.is_empty(), "一个引擎命令都不入队");
        assert!(
            app.log.iter().any(|l| l.contains("读到 3 个派")),
            "读到的条数进日志:{:?}",
            app.log
        );
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )));
        assert!(!app.as_history, "再按 h 退视图");
    }

    /// 一条坏记录只坏它自己(规格故事 4):分类各归各位,不连坐。
    #[test]
    fn 装配历史_坏清单读不通_其余派照常() {
        let app = history_app("classify");
        let rows = app.as_history_rows.as_ref().expect("读过盘");
        // 字典序 fresh / half / research-desk
        assert!(
            matches!(rows[0].status, crate::datarepo::ManifestStatus::Missing),
            "第一部还没装:缺失"
        );
        assert!(
            matches!(
                rows[1].status,
                crate::datarepo::ManifestStatus::Broken { .. }
            ),
            "第二部清单写坏:读不通"
        );
        match &rows[2].status {
            crate::datarepo::ManifestStatus::Ok(a) => assert_eq!(
                a.brief.lock_id.as_deref(),
                Some("research-desk-1.0.0-360ae68")
            ),
            other => panic!("第三部该读通,实得 {other:?}"),
        }
    }

    /// 回车只在「认得也读通了」那一派上给原文;三种不认得的分类各给一句
    /// 人话,不画一个空浮层假装有原文。
    #[test]
    fn 装配历史_回车在坏记录上只给指路_不画空浮层() {
        let mut app = history_app("enter");
        app.as_history_sel = 1; // 写坏清单的 half
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(app.as_history_raw.is_none(), "读不通就没有原文可看");
        assert!(
            app.log.iter().any(|l| l.contains("清单读不通")),
            "日志给指路:{:?}",
            app.log
        );
    }

    /// 本地过滤(#101):纯本地、零引擎调用;Esc 先清词再退视图(两级)。
    #[test]
    fn 装配历史_过滤纯本地_esc两级() {
        let mut app = history_app("filter");
        // `/` 进输入态,键入即筛
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('/'),
            KeyModifiers::NONE,
        )));
        for c in "research".chars() {
            app.update(Message::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::NONE,
            )));
        }
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(!app.as_history_filter_on, "Enter 收词退输入态");
        assert_eq!(app.history_visible().len(), 1, "只命中 research-desk");
        assert!(app.queue.is_empty(), "纯本地筛选,零引擎调用");
        // 第一级 Esc:清词,视图还在
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(app.as_history, "清词不退视图");
        assert_eq!(app.history_visible().len(), 3, "清词回全量");
        // 第二级 Esc:退视图
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(!app.as_history, "再按 Esc 退视图");
    }

    /// 显式刷新(#101):面板之外新装了一个派,按 r 重读快照就进来——不轮询、
    /// 不自动跑。
    #[test]
    fn 装配历史_r重读快照() {
        let mut app = history_app("refresh");
        let home = std::path::PathBuf::from(app.nw_home.clone());
        std::fs::create_dir_all(home.join("profiles").join("later")).unwrap();
        std::fs::write(
            home.join("profiles").join("later").join(".nw-manifest"),
            HISTORY_MANIFEST_OK,
        )
        .unwrap();
        assert_eq!(app.history_total(), 3, "快照还是旧的");
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('r'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.history_total(), 4, "按 r 重读后新派在场");
        assert!(app.queue.is_empty(), "重读也不下发引擎命令");
    }

    /// 安装根不在(#101 验收):走指路语义——`home_cause` 给结论,界面不画空列表。
    #[test]
    fn 装配历史_安装根不在走指路() {
        let mut app = fake_app();
        app.screen = Screen::Assemble;
        app.nw_home = std::env::temp_dir()
            .join(format!("nwt-hist-gone-app-{}", std::process::id()))
            .display()
            .to_string();
        app.as_history = true;
        app.load_history();
        assert!(
            matches!(app.home_cause(), Some(crate::datarepo::HomeCause::Missing)),
            "安装根不在:缺失结论"
        );
        assert!(
            app.as_history_rows.as_ref().unwrap().is_empty(),
            "没有派可列"
        );
    }

    /// 鼠标点行 = 键盘 ↑/↓ 同一路(#101):点历史视图的行选中的是那一派,
    /// 不串到锁定集选择器。
    #[test]
    fn 装配历史_点行选中与键盘同一路() {
        let mut app = history_app("click");
        app.size = Rect::new(0, 0, 110, 30);
        app.splash = false;
        let row = hit_target(&app, |t| matches!(t, Target::SelectRow(_)));
        click(&mut app, row);
        assert_eq!(app.as_history_sel, 0, "点第一行选中第一派");
        assert_eq!(app.as_sel, 0, "锁定集选择器一个字节不动");
    }

    /// 装配在途不受理 `h`(#101:在途那屏归七步与中止键);`up` 一下发即清场。
    #[test]
    fn 装配历史_装配在途不受理_up下发即清场() {
        let mut app = history_app("running");
        app.as_running = true;
        app.assembling = true;
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )));
        assert!(
            app.as_history,
            "在途不踢出视图(渲染画的是运行态),但也不新开"
        );
        // 在途时历史视图的键位一律无分支:`h` 不退、Esc 不关原文浮层
        app.as_history_raw = Some("{}".into());
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )));
        assert!(app.as_history, "在途时 h 无分支");
        // 原文浮层是顶层浮层(与帮助同级),在途也关得掉——不然在途时它会在
        // 屏上关不掉,那是把操作者关住。
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(app.as_history_raw.is_none(), "在途时原文浮层仍可关");
        app.as_history = false;
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )));
        assert!(!app.as_history, "在途时进不了视图");
        // up 下发即清场:回音落地时屏幕已经在运行态
        app.as_history = true;
        app.ls = Some(
            serde_json::from_str::<LsData>(r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[{"lockId":"9f8e"}]}"#)
                .unwrap(),
        );
        app.launch_selected_lock();
        assert!(!app.queue.is_empty(), "up 已入队");
        // 入队只是排队;清场发生在真正下发那一刻(note_engine_dispatch),
        // 与主循环排空 queue 的时序一致。
        let (tx, _rx) = mpsc::unbounded_channel();
        for args in std::mem::take(&mut app.queue) {
            app.spawn_engine(args, &tx);
        }
        assert!(!app.as_history, "up 一下发,历史视图清场");
        assert!(app.as_running, "装配态接管");
    }

    // ---- #104 发射前 diff 预览:仪式多一页,Enter/Esc 语义不动 ----

    /// 一座 fixture 安装根 + 一份选中锁定集(#104):清单记着上一轮
    /// `old-lock`,在途这一份是 `new-lock`,两边组件集合有增有删。
    fn ritual_diff_home(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nwt-ritual-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pd = dir.join("profiles").join("desk");
        std::fs::create_dir_all(&pd).unwrap();
        std::fs::write(
            pd.join(".nw-manifest"),
            r#"{"schema":"nw/manifest.v1","lockId":"old-lock","profile":"desk","recipe":"research-desk","version":"1.0.0","runtimeKey":"rk","installedAt":"2026-10-01T00:00:00Z","convergeHash":"3cf6a584a523b35ec42630766a9eb6a84d3dcf11d2347d5aa3f3ae879b420502","engine":"0.1.0"}"#,
        )
        .unwrap();
        let ld = dir.join("locks");
        std::fs::create_dir_all(&ld).unwrap();
        // 上一轮锁定集:scout + retention(与清单 lockId 对得上)
        std::fs::write(
            ld.join("old-lock.nwlock"),
            r#"{"schema":"nw/lock.v1","lockId":"old-lock","profile":"desk","components":[{"name":"scout"},{"name":"retention"}]}"#,
        )
        .unwrap();
        // 在途这一份:retention 留着,scout 走了,新来 mcp-server
        std::fs::write(
            ld.join("new-lock.nwlock"),
            r#"{"schema":"nw/lock.v1","lockId":"new-lock","profile":"desk","components":[{"name":"mcp-server"},{"name":"retention"}]}"#,
        )
        .unwrap();
        dir
    }

    /// 一台「选中锁定集 = new-lock、安装根是 fixture」的台子(#104)。
    fn ritual_app(tag: &str) -> App {
        let mut app = App::with_runner(Arc::new(FakeRunner::new(&[])));
        app.screen = Screen::Assemble;
        app.nw_home = ritual_diff_home(tag).display().to_string();
        app.ls = Some(
            serde_json::from_str(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[{"lockId":"new-lock","recipe":"research-desk"}]}"#,
            )
            .unwrap(),
        );
        app.as_sel = 0;
        app.as_confirm = true;
        app
    }

    #[test]
    fn 发射仪式_d键翻新旧对比页_再按回简报页() {
        let mut app = ritual_app("toggle");
        assert!(
            matches!(app.as_ritual_page, crate::screens::RitualPage::Brief),
            "开场是既有的简报页"
        );
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
        )));
        assert!(
            matches!(app.as_ritual_page, crate::screens::RitualPage::Diff),
            "d 翻到新旧对比页"
        );
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::NONE,
        )));
        assert!(
            matches!(app.as_ritual_page, crate::screens::RitualPage::Brief),
            "再按 d 回简报页:进出一个键,不必记两个"
        );
    }

    #[test]
    fn 对比页_esc仍返航_enter仍发射_肌肉记忆不作废() {
        // 规格故事 10:预览只是仪式中间一页,两个既有键的语义一个字节不改。
        let (runner, mut app) = up_inflight_app();
        app.nw_home = ritual_diff_home("keys").display().to_string();
        app.ls = Some(
            serde_json::from_str(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[{"lockId":"new-lock","recipe":"research-desk"}]}"#,
            )
            .unwrap(),
        );
        app.as_sel = 0;
        app.as_confirm = true;
        app.as_ritual_page = crate::screens::RitualPage::Diff;
        app.load_ritual_diff();
        // Esc:仍是返航(关仪式 + 留自己的话),不因为站在对比页就变成「回上一页」
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        assert!(!app.as_confirm, "对比页上 Esc 照样返航");
        assert!(
            app.log.iter().any(|l| l.contains("发射取消")),
            "返航留自己的话:{:?}",
            app.log
        );
        assert_eq!(runner.abort_count(), 0, "返航不碰运行器(还没下发)");
        // Enter:仍是发射(入队 up),不因为站在对比页就要多按一次
        app.as_confirm = true;
        app.as_ritual_page = crate::screens::RitualPage::Diff;
        app.load_ritual_diff();
        let qlen = app.queue.len();
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.queue.len(), qlen + 1, "对比页上 Enter 照样发射");
        assert!(
            app.queue.last().unwrap().iter().any(|a| a == "up"),
            "入队的还是 up:{:?}",
            app.queue.last()
        );
    }

    #[test]
    fn 重开仪式回到简报页_不把上一轮的对比页带给下一次() {
        let mut app = ritual_app("reopen");
        app.as_ritual_page = crate::screens::RitualPage::Diff;
        app.load_ritual_diff();
        // 返航( Esc )再重开:页码归位,不然操作者按 Enter 后莫名站在对比页
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(app.as_confirm, "重开仪式");
        assert!(
            matches!(app.as_ritual_page, crate::screens::RitualPage::Brief),
            "重开必回简报页"
        );
    }

    #[test]
    fn 对比页_组件增删按名做集合差_不猜旧态() {
        let mut app = ritual_app("diff");
        app.as_ritual_page = crate::screens::RitualPage::Diff;
        app.load_ritual_diff();
        let d = app.as_ritual_diff.as_ref().expect("对比快照在场");
        // 增:在途有、上一轮没有
        assert_eq!(d.added, vec!["mcp-server"], "只有 mcp-server 是新增");
        // 删:上一轮有、在途没有
        assert_eq!(d.removed, vec!["scout"], "只有 scout 被换下");
        // retention 两侧都在:既不进增也不进删
        assert!(!d.added.contains(&"retention".to_string()));
        assert!(!d.removed.contains(&"retention".to_string()));
        // 指纹侧:清单现值 + 判定,面板不重算任何哈希
        assert_eq!(
            d.old_hash.as_deref(),
            Some("3cf6a584a523b35ec42630766a9eb6a84d3dcf11d2347d5aa3f3ae879b420502")
        );
        assert!(d.hash_changes, "换了锁定集:指纹会变(判据写在屏上)");
        assert_eq!(d.old_lock_id.as_deref(), Some("old-lock"));
    }

    #[test]
    fn 对比页_清单缺失时明示首次装配_不摆空对比() {
        let mut app = ritual_app("first");
        // 把清单挪走:新派第一次装配,没有旧态可比
        let home = std::path::PathBuf::from(app.nw_home.clone());
        std::fs::remove_file(home.join("profiles").join("desk").join(".nw-manifest")).unwrap();
        app.as_ritual_page = crate::screens::RitualPage::Diff;
        app.load_ritual_diff();
        let d = app.as_ritual_diff.as_ref().expect("对比快照在场");
        assert_eq!(
            d.added,
            vec!["mcp-server", "retention"],
            "首次装配:全是将铺入"
        );
        assert!(d.removed.is_empty(), "没有旧态,删除列无可比内容");
        assert!(d.first_assembly, "明示首次装配、无旧态可比");
        assert!(
            !d.hash_changes,
            "没有旧指纹可对比:不说「会变」也不说「不变」"
        );
    }
}
