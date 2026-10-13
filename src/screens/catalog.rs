//! 编目册屏(spec #50 工单 #53 / #54 / #56 / #57,ADR-0009):第一本编目册(vps)
//! 端到端打通,同时立起**编目册骨架三件**;#54 加第二本(标签册),验证骨架在
//! 「无确认闸的写路径」上也站得住;#56 加第三本(订阅册),验证骨架在
//! 「本体只经 stdin 进」的机密路径上也站得住(到期三态按 UTC 现算);#57 加第四本
//! (钥匙册),机密纪律最严的一本——本体只经 stdin 进、投影无本体槽也不印指纹、
//! get 一律只给建议命令(连「这像是哪把钥匙」都不该让人推断出来)。
//!
//! 三件骨架(ADR-0009 Consequences):
//! 1. envelope 投影类型(条目 + 编目路径 + 闸结论,字段恒在不为 null)——
//!    在 [`crate::engine::envelope`](`CatalogSheet` / `CatalogEntryView` /
//!    `CatalogGate` / `CatalogRef`),与 `RegisterData` / `RecheckData` 同层;
//! 2. **按投影字段渲染且遵循确定性排序的列表件**——本模块的
//!    [`catalog_sheet_lines`](后几本册子只换数据源与 [`EntryDetail`],
//!    不换列表件);
//! 3. **与命令组无关的两段式登记流**(预审 → 摊开闸结论 → 图形确认 →
//!    落账 → 刷新)——本模块的 [`RegisterFlow`] / [`RegisterSpec`] /
//!    [`preaudit_passed`],从外部源登记两段式(`app.rs` 的 `RegState` 一族)
//!    里抽出同构件。外部源登记**行为逐字节不变**(既有快照全绿):抽的是
//!    复用件,不是改语义——vps 先走这件,外部源登记的落账闸口径也并到
//!    [`preaudit_passed`] 一处(同一个布尔,不换说法)。
//!
//! # 册子与屏的关系(#54)
//!
//! 屏只有一块(`Screen::Vps`,标题「编目」),**册子是屏内的一个维度**
//! ([`CatalogKind`]):四本编目共用同一套骨架,拆四块屏等于把同一件东西
//! 复制四份;而「当前是哪一本」必须显式在场(四本都是数据仓根下机器写的
//! JSON)。键 `m` 换册。
//!
//! 机密纪律(ADR-0009 §3):`logins[].ref` 是**指针不是密钥**,原样呈现安全;
//! 摘要行不印 ref 与 addr(照引擎 stderr 摘要的口径)。订阅册与钥匙册有「本体」:
//! 链接 / 钥匙本体都**只经 stdin 进**,面板侧只以 [`SecretBody`] 这一
//! 「在途内存、不打印内容的外包类型」存在——喂入框渲染成掩码、日志只记
//! 字节数、Debug 不打印内容、不进 config,流程结束即 drop。钥匙册更严:投影
//! 无本体槽也不印指纹,`get` 一律只给建议命令。vps 组 argv 面无密码参数
//! (误粘 PEM 由引擎第⑤道闸拦);tag 组连指针槽都没有(`--ref` 走 argv 不走 stdin)。

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crossterm::event::KeyCode;

use crate::app::App;
use crate::engine::{
    capability, capability::Capability, CatalogEntryView, CatalogGate, CatalogRef, CatalogSheet,
    EngineOutput, KeyRegisterData, State, SubRegisterData, SubStatusCounts, TagAddData, TagLsData,
    VpsLsData, VpsRegisterData,
};
use crate::input::{self, InputAction};
use crate::layout::{popup_rect, REG_INPUT_H, REG_INPUT_W};
use crate::screens::recipes::{clip, wrap};
use crate::screens::{body, data_root_signpost};
use crate::theme::Palette;

// ---- 骨架第三件:与命令组无关的两段式登记流 ----

/// 两段式登记的阶段。与外部源登记的 `RegState` 同构(那个多两步表单:
/// repo 链接 → kind 选择),阶段语义逐字对齐:预审段**文件不落**,
/// 落账段才带确认闸;任一阶段可返航,返航不留半条记录。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RegPhase {
    /// 未登记;编目册常态。
    #[default]
    Idle,
    /// 填表(命令组自己的表单形态)。
    Form,
    /// 预审已下发(不带 `--yes`,文件不落),等回音。
    PreAudit,
    /// 预审回音已呈现(条目 + 闸结论 + 拟落账),可审阅、可落账、可返航。
    Review,
    /// 落账已下发(图形确认后才带 `--yes`),等写盘回音;并入既有装配态
    /// (`assembling` 顶栏灯 / HUD 同源)。
    Commit,
    /// 落账回音已呈现。
    Done,
}

/// 两段式登记流的运行时:阶段 + 最近一次回音(只由刚下发那一次写入,
/// 面板不缓存、不轮询、不冒充「上次结果」)。与命令组无关——命令组的
/// 差异全在 [`RegisterSpec`] 里。
#[derive(Default)]
pub struct RegisterFlow {
    pub phase: RegPhase,
    pub result: Option<EngineOutput>,
}

impl RegisterFlow {
    pub fn new() -> Self {
        Self::default()
    }

    /// 发起登记:进表单态,清掉上一轮回音(旧结论不冒充本次)。
    pub fn begin(&mut self) {
        self.phase = RegPhase::Form;
        self.result = None;
    }

    /// 预审已下发(入队不带 `--yes` 的 argv):进等回音态。
    pub fn preaudit_sent(&mut self) {
        self.phase = RegPhase::PreAudit;
        self.result = None;
    }

    /// 预审回音到达:只在等回音态收账,进确认态(闸结论摊开给看)。
    /// 已返航的迟到回音返回 false(调用方丢弃,不幽灵重开)。
    pub fn preaudit_back(&mut self, out: EngineOutput) -> bool {
        if self.phase != RegPhase::PreAudit {
            return false;
        }
        self.result = Some(out);
        self.phase = RegPhase::Review;
        true
    }

    /// 落账已下发(图形确认后才可能到达):进等写盘回音态。
    pub fn commit_sent(&mut self) {
        self.phase = RegPhase::Commit;
    }

    /// 落账回音收敛。返回 `true` = 写盘成功(调用方刷新编目册),
    /// `false` = 失败(留在确认态把结论说清,闸随失败结论关闭)。
    /// 已返航的迟到回音不收敛任何状态,只回报「不是本次」。
    pub fn commit_back(&mut self, out: EngineOutput) -> Option<bool> {
        if self.phase != RegPhase::Commit {
            return None;
        }
        let ok = out.ok;
        self.result = Some(out);
        self.phase = if ok { RegPhase::Done } else { RegPhase::Review };
        Some(ok)
    }

    /// 返航:任一态同义,状态清空不留半条记录(回音丢弃)。
    pub fn abandon(&mut self) {
        self.phase = RegPhase::Idle;
        self.result = None;
    }

    /// 在途(等回音)才算「登记中」——HUD 与互斥判定共用这一条。
    pub fn in_flight(&self) -> bool {
        !matches!(self.phase, RegPhase::Idle)
    }
}

/// 一次可摊开的预审结论:条目投影 + 编目路径 + 闸结论。
/// `None` = 回音不可定型(含契约守卫「需升级 nw」)——面板**不按旧形状
/// 降级解析**,落账闸因此不放行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Audit {
    pub entry: Option<CatalogEntryView>,
    pub catalog_path: String,
    pub gates: Vec<CatalogGate>,
}

impl Audit {
    /// 闸里有 fail(引擎铁律:任一不过 = 一个字节都不落)。
    pub fn has_fail(&self) -> bool {
        self.gates.iter().any(CatalogGate::is_fail)
    }
}

/// 两段式登记的命令组契约:预审段与落账段的 argv 构造**在类型层面分开**
/// (两个方法,预审那个连 `--yes` 的位置都没有),误下发靠形状杜绝。
///
/// vps 是第一个实现(`VpsSpec`);key / subscription 两本(与 vps 同形状)
/// 只换这一个实现(本体经 stdin 喂入,#55 铺的管子)。**tag 是例外**(#54):上游
/// `tag add` 没有 `--yes`(追加合并、幂等,没有「覆盖」可确认),所以
/// 它的 `preaudit` 与 `commit` 是**同一份 argv**,且那一份本身就是写命令
/// ——tag 没有「拟落账不落盘」的那一段。别为了套这个 trait 伪造一枚
/// `--yes`:多一枚未知 flag 引擎直接 rc 3,那是改引擎语义。
///
/// 没有 `self` 接收者的三个方法(`title` / `audit` / `confirm_gate`)带
/// `where Self: Sized`:它们是**命令组自己的静态属性**,按类型问、不按实例问
/// (调用形状一字不变),加这一条只是为了把本 trait 变成 object safe——
/// #76 起 `CatalogKind::spec_from_form` 要把四本册的规格收成同一份
/// `Box<dyn RegisterSpec>` 交给调用方那一份骨架,四本册才不必各写一份分发。
pub trait RegisterSpec {
    /// 表单标题(呈现层用)。
    fn title() -> &'static str
    where
        Self: Sized;

    /// 预审段 argv:跑闸、拟落账,**文件不落**(无 `--yes`)。
    fn preaudit(&self, root: &str) -> Vec<String>;

    /// 落账段 argv:只能由图形确认后的转换调用(带确认闸)。
    fn commit(&self, root: &str) -> Vec<String>;

    /// 这一趟登记的**条目名**(日志口径:只报名字,不报本体)。
    fn name(&self) -> &str;

    /// 回音拆成可摊开的预审结论;不可定型给 `None`。
    fn audit(out: &EngineOutput) -> Option<Audit>
    where
        Self: Sized;

    /// 引擎这一组**有没有自己的确认闸**(`--yes`)。`true` = 有(vps /
    /// 外部源):预审段文件不落,面板的图形确认落在「预审通过 → 落账」
    /// 之间。`false` = 没有(`tag add`):面板**不自己造一道**
    /// (ADR-0009 §2)——表单 Enter 直接下发写命令,回音摊开闸结论即终态。
    ///
    /// 为什么做成 trait 上的方法而不是面板侧的一个 `if`:「有没有 `--yes`」
    /// 是**命令组自己的属性**,面板不替引擎回答(同 #29 探测铁律)。默认
    /// `true` 让 vps 与外部源登记两个既有实现逐字节不变。
    ///
    /// 谁在消费它:`render_tag_review` 的键位行按它决定画不画「Enter 落账」
    /// 段(tag add 无闸 → 只画返航段),测试钉两种实现的取值。不是死代码。
    fn confirm_gate() -> bool
    where
        Self: Sized,
    {
        true
    }
}

/// 落账闸(两段式共用口径,外部源登记与 vps 登记同一处):
/// 退出码 0 + 回音可定型 + 闸无 fail。任一不过 = 不放行落账,
/// 且面板**不下发任何写命令**。
pub fn preaudit_passed(out: &EngineOutput, audit: Option<&Audit>) -> bool {
    out.ok && audit.map(|a| !a.has_fail()).unwrap_or(false)
}

// ---- 当前是哪一本册子(工单 #54:编目册屏从「只有 vps」长成「多本」) ----

/// 编目册屏当前翻开的那一本。
///
/// **为什么是屏内的一个枚举而不是第二块屏**:四本编目共用同一套骨架
/// (投影三件 / 列表件 / 登记流),拆成四块屏等于把同一件东西复制四份;
/// 而「当前是哪一本」又必须显式在场——四本都是数据仓根下机器写的 JSON,
/// 操作者得知道自己正在改哪一本(ADR-0009:机器写、永不手改)。键 `m`
/// 换册(与健康屏 `m` 轮转模式同语义)。后三本(tag / subscription / key)
/// 随票追加,只在这里多一个变体。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CatalogKind {
    /// 机器册:`root>/vps.json`(#53)。
    #[default]
    Vps,
    /// 标签册:`<root>/tags.json`(工单 #54;本票主价值是反查)。
    Tag,
    /// 订阅册:`<root>/subscriptions.json`(工单 #56;本票主价值是到期三态)。
    Subscription,
    /// 钥匙册:`<root>/keys.json`(工单 #57;机密纪律最严的一本:本体只经 stdin
    /// 进,投影无本体槽也不印指纹,get 只给建议命令)。
    Key,
}

impl CatalogKind {
    /// 册名(操作者视角;词有出处:vps / 标签 / 订阅 / 钥匙都是上游命令组名,不自造)。
    pub fn label(self) -> &'static str {
        match self {
            CatalogKind::Vps => "机器",
            CatalogKind::Tag => "标签",
            CatalogKind::Subscription => "订阅",
            CatalogKind::Key => "钥匙",
        }
    }

    /// 编目文件名(数据仓根下机器写的那本 JSON)。
    pub fn file(self) -> &'static str {
        match self {
            CatalogKind::Vps => "vps.json",
            CatalogKind::Tag => "tags.json",
            CatalogKind::Subscription => "subscriptions.json",
            CatalogKind::Key => "keys.json",
        }
    }

    /// 这一本的能力门(#29 组级探针:`vps` / `tag` / `subscription` / `key`)。
    pub fn capability(self) -> Capability {
        match self {
            CatalogKind::Vps => Capability::Vps,
            CatalogKind::Tag => Capability::Tag,
            CatalogKind::Subscription => Capability::Subscription,
            CatalogKind::Key => Capability::Key,
        }
    }

    /// 册体里**每一条目占几行**(渲染与命中共用,同 `layout` 的法子:
    /// 几何只有一份真相,点哪打哪)。
    pub fn row_lines(self) -> u16 {
        match self {
            CatalogKind::Vps => 4,          // 名字+端口 / 地址 / 登录 / 备注
            CatalogKind::Tag => 2,          // 名字 / 引用
            CatalogKind::Subscription => 2, // 名字+client / 到期日+三态+剩余天数
            CatalogKind::Key => 4,          // 名字+kind / 主机 / 作用域条数 / 轮换日
        }
    }

    /// 这一本的条目明细画法(列表件的可换部件,见 [`catalog_sheet_lines`])。
    pub fn detail(self) -> EntryDetail {
        match self {
            CatalogKind::Vps => vps_detail_rows,
            CatalogKind::Tag => tag_detail_rows,
            CatalogKind::Subscription => sub_detail_rows,
            CatalogKind::Key => key_detail_rows,
        }
    }

    /// 换一册(`m` 键)。
    /// 换册:四本册按 Vps → Tag → Subscription → Key → Vps 轮转(屏内维度,
    /// 不换屏;`m` 键)。
    pub fn next(self) -> Self {
        match self {
            CatalogKind::Vps => CatalogKind::Tag,
            CatalogKind::Tag => CatalogKind::Subscription,
            CatalogKind::Subscription => CatalogKind::Key,
            CatalogKind::Key => CatalogKind::Vps,
        }
    }

    /// 这一本在「按册各留一份」状态数组里的下标(#69 册内过滤:编目过滤词是
    /// `[String; 4]`,换册时每本各留各的词,靠这个下标取自己那一份;
    /// 规格故事 24:机器册筛「gate」、切到钥匙册筛「deploy」,回头机器册
    /// 还在「gate」上)。四本固定 0/1/2/3,与枚举声明序一致。
    pub fn idx(self) -> usize {
        match self {
            CatalogKind::Vps => 0,
            CatalogKind::Tag => 1,
            CatalogKind::Subscription => 2,
            CatalogKind::Key => 3,
        }
    }

    // ---- 册级查表(工单 #66:按册差异只剩数据,不再有分支) ----
    //
    // 下面这一族把「这一本的登记流槽 / 删除确认旗 / 册体条数 / 选中槽 /
    // 落账闸 / 投影数据源 / 空册文案 / 附述行 / 边界表述 / 键位行 / 浮层 /
    // 册内键位表」收成 `CatalogKind` 的成员。**调用方不再写第四份
    // `if kind == …`**:命中区(`layout::catalog_hits`)、册内键位路由
    // (`App::key_catalog`)、渲染(`render_catalog` / `catalog_key_parts`)与
    // `App` 的册级访问器各只剩一处公共代码路径,四本册走同一条。
    //
    // 加一条只在一本册上成立的关切 = 在这里加一个成员(那一本给真值,
    // 其余三本给 `false` / 空 / `None`),不是往调用方再插一个分支——
    // 第五本册子来时同样只改这一处。测试
    // `只属一两本的关切_按册各按一次_表外一本都不多发` 钉住这条口径。

    /// 这一本的登记流槽(两段式 / 无确认闸两种形态,四本各一份)。
    pub fn flow(self, app: &App) -> &RegisterFlow {
        match self {
            CatalogKind::Vps => &app.vps_flow,
            CatalogKind::Tag => &app.tag_flow,
            CatalogKind::Subscription => &app.sub_flow,
            CatalogKind::Key => &app.key_flow,
        }
    }

    // ---- 册级回音命令族查表(工单 #76:`on_*_echo` 四份收成一份) ----
    //
    // 下面这一族把「ls / register / rm / add 回音」按册的差异收成
    // `CatalogKind` 的成员。调用方(`App::on_echo` 及其子路由)只写
    // `kind.xxx(app, out)`,**不再有第二处 `match kind` / `if kind ==`**:
    // 登记段(vps / 订阅 / 钥匙三本同形)在 `App::on_echo_register` 里收成一份
    // 实现;ls / rm 的按册差异(数据型 / 存储槽 / 日志口径,以及标签册独有的
    // 反查收敛)落在 `absorb_ls` / `absorb_rm` 两个查表成员里——与 #66
    // `sheet` 同一形状:一处匹配长在 [`CatalogKind`] 上,调用方不分册。
    //
    // 加一条按册的回音关切 = 在这里加 / 改一个成员,不是往调用方再插一个分支。

    /// 回音 argv 的命令组名 → 这一本(`App::on_engine_finished` 的回音路由:
    /// 四条 `"vps" => …` / `"tag" => …` … 收成一条 `from_command` 查表)。
    pub fn from_command(s: &str) -> Option<Self> {
        match s {
            "vps" => Some(CatalogKind::Vps),
            "tag" => Some(CatalogKind::Tag),
            "subscription" => Some(CatalogKind::Subscription),
            "key" => Some(CatalogKind::Key),
            _ => None,
        }
    }

    /// 这一本登记流槽的可写侧(登记回音两段式收敛走这一口;与 [`Self::flow`]
    /// 同一张表)。
    pub fn flow_mut(self, app: &mut App) -> &mut RegisterFlow {
        match self {
            CatalogKind::Vps => &mut app.vps_flow,
            CatalogKind::Tag => &mut app.tag_flow,
            CatalogKind::Subscription => &mut app.sub_flow,
            CatalogKind::Key => &mut app.key_flow,
        }
    }

    /// 这一本的只读刷新(登记 / 删除回音成功后重读编目册;四本各调自己的
    /// `run_*_ls`,标签册的反查刷新带 `None` = 列全部)。
    pub fn run_ls(self, app: &mut App) {
        match self {
            CatalogKind::Vps => app.run_vps_ls(),
            CatalogKind::Tag => app.run_tag_ls(None),
            CatalogKind::Subscription => app.run_sub_ls(),
            CatalogKind::Key => app.run_key_ls(),
        }
    }

    /// 这一本 ls 回音的「在读」旗(可写侧;回音落地即灭,与 `App` 各册自己的
    /// `*_ls_loading` 同一张表)。
    pub fn ls_loading_mut(self, app: &mut App) -> &mut bool {
        match self {
            CatalogKind::Vps => &mut app.vps_ls_loading,
            CatalogKind::Tag => &mut app.tag_ls_loading,
            CatalogKind::Subscription => &mut app.sub_ls_loading,
            CatalogKind::Key => &mut app.key_ls_loading,
        }
    }

    /// 登记写盘「可能已完成」那句里的册名词(按册:机器册沿用量「编目册」,
    /// 后三本给各自册名)。**只登记回音段用**(vps / 订阅 / 钥匙;标签册无登记段)。
    pub fn write_refreshed_noun(self) -> &'static str {
        match self {
            CatalogKind::Vps => "编目册",
            CatalogKind::Tag => "标签册",
            CatalogKind::Subscription => "订阅册",
            CatalogKind::Key => "钥匙册",
        }
    }

    /// 这一本 ls 回音的落地(只读刷新:#53 / #54 / #56 / #57)。按册差异全在
    /// 这里:数据型(`VpsLsData` / `TagLsData` / `SubLsData` / `KeyLsData`)、
    /// 存储槽、日志口径,以及**标签册独有的反查收敛**(#70:机器册问「什么标签
    /// 引用了这一条」,回音落地时从 fresh 标签册就地推导 `vps_reverse`)。
    /// 调用方(`App::on_echo_ls`)只负责灭「在读」旗,不再分册。
    pub fn absorb_ls(self, app: &mut App, out: EngineOutput) {
        match self {
            CatalogKind::Vps => {
                match out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::VpsLsData>())
                {
                    Some(d) => {
                        let n = d.machines.len();
                        app.vps_ls = Some(d);
                        app.catalog_clamp();
                        // 反查跳转的待落地选中位(#120):vps 投影读到了,
                        // 按名字把选中位落下去(见 `App::land_jump_pending`)。
                        app.land_jump_pending();
                        app.push_log(format!("vps 编目册已刷新:{n} 台机器(只读)"));
                    }
                    None => app.push_log(format!("vps ls 结果不可用:{}", out.headline())),
                }
            }
            CatalogKind::Tag => {
                // 问的是哪一条,读下发时记下的 tag_lookup(None = 列全部),
                // 不从回音 argv 重新推导(见 App::run_tag_ls 的入参)。
                let asked = app.tag_lookup.clone();
                // 反查上浮在途(#70):按在途记住的那台机器,从这次 fresh 标签册
                // 就地按 `<kind>/<条目>` 精确匹配推导。只在在途时收敛。
                let machine = match &app.vps_reverse {
                    VpsReverse::InFlight { machine } => Some(machine.clone()),
                    _ => None,
                };
                // 「读到了」的判据:rc 0 且拆得出 TagLsData(两种语义不互相冒充)。
                match out
                    .ok
                    .then_some(out.envelope.as_ref())
                    .flatten()
                    .and_then(|e| e.data_as::<crate::engine::TagLsData>())
                {
                    Some(d) => {
                        let n = d.tags.len();
                        // 先算收敛结论(借用 d),再 move out 进 tag_ls_echo。
                        let converged = machine.as_ref().map(|m| {
                            let hits = tags_referring(&d.sheet(), "vps", m);
                            (m.clone(), hits)
                        });
                        // 反查跳转浮层的命中条目(#120):只在这一本、且问的
                        // 是某一条(`tag_lookup` = Some)时才开——列全部与
                        // 机器册反查上浮都走 `run_tag_ls(None)`,那一态没有
                        // 「这一条的命中」可言。一条引用都没有的标签不开
                        // (没有可跳的东西,册体的「引用 ——」已经说清)。
                        // 在 move out 之前算完:借用 d,不借用 app。
                        let jumped = asked.as_deref().and_then(|name| {
                            d.tags
                                .iter()
                                .find(|t| t.name == name)
                                .map(|t| (name.to_string(), t.refs.clone()))
                                .filter(|(_, refs)| !refs.is_empty())
                        });
                        app.tag_ls_echo = Some(out);
                        app.catalog_clamp();
                        if let Some((m, hits)) = converged {
                            app.vps_reverse = if hits.is_empty() {
                                VpsReverse::Empty { machine: m }
                            } else {
                                VpsReverse::Hits {
                                    machine: m,
                                    tags: hits,
                                }
                            };
                        }
                        app.tag_jump = match jumped {
                            Some((tag, hits)) => TagJump::Hits { tag, hits, sel: 0 },
                            None => TagJump::Idle,
                        };
                        let scope = match asked.as_deref() {
                            Some(name) => format!("标签 {name}:{n} 条"),
                            None => format!("全部标签:{n} 条"),
                        };
                        app.push_log(format!("反查完成({scope},只读)"));
                    }
                    None => {
                        let h = out.headline().to_string();
                        app.tag_ls_echo = Some(out);
                        // 读不到的一律不收跳转浮层(#120):「这次没读到」
                        // 与「有 N 条命中」是两件事,话不许互相冒充——浮层
                        // 不开,既有指路(日志 + 册体那行)说这次的实话。
                        app.tag_jump = TagJump::Idle;
                        if let Some(m) = machine {
                            app.vps_reverse = VpsReverse::Unreadable {
                                machine: m,
                                reason: h.clone(),
                            };
                        }
                        app.push_log(format!("反查没结果:{h}"));
                    }
                }
            }
            CatalogKind::Subscription => {
                match out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::SubLsData>())
                {
                    Some(d) => {
                        let n = d.subscriptions.len();
                        let counts = d.status_counts();
                        app.sub_ls = Some(d);
                        app.catalog_clamp();
                        app.sub_get_logged = false;
                        app.push_log(format!(
                            "订阅册已刷新:{n} 条(active {} · expiring {} · expired {},只读)",
                            counts.active, counts.expiring, counts.expired
                        ));
                    }
                    None => app.push_log(format!("subscription ls 结果不可用:{}", out.headline())),
                }
            }
            CatalogKind::Key => {
                match out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::KeyLsData>())
                {
                    Some(d) => {
                        let n = d.keys.len();
                        app.key_ls = Some(d);
                        app.catalog_clamp();
                        app.key_get_logged = false;
                        app.push_log(format!("钥匙册已刷新:{n} 条(只读,无本体槽、不印指纹)"));
                    }
                    None => app.push_log(format!("key ls 结果不可用:{}", out.headline())),
                }
            }
        }
    }

    /// 这一本 rm 回音的落地(#53 / #54 / #56 / #57;写互斥的灭由调用方
    /// `App::on_echo_rm` 统一做)。按册差异(rm 数据型 / 结果槽 / 成功口径,
    /// 以及标签册的「摘引用 vs 删标签」两态)全在这里。
    pub fn absorb_rm(self, app: &mut App, out: EngineOutput) {
        match self {
            CatalogKind::Vps => {
                let typed = out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::VpsRmData>());
                let rm_ok = out.ok;
                let rm_headline = out.headline();
                app.vps_rm_result = Some(out);
                match typed {
                    Some(d) if d.removed => {
                        app.push_log(format!(
                            "已删除 {}:剩余 {} 台(一次 git 变更,走 PR + 家长审)",
                            d.name, d.remaining
                        ));
                        app.run_vps_ls();
                    }
                    Some(d) => {
                        app.push_log(format!(
                            "没删成:{} 不在编目里(action {}),vps.json 一个字节都没动",
                            d.name, d.action
                        ));
                    }
                    None => {
                        let _ = rm_ok;
                        app.push_log(format!("删除结果不可用:{rm_headline}"))
                    }
                }
            }
            CatalogKind::Tag => {
                let typed = out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::TagRmData>());
                app.tag_rm_result = Some(out);
                match typed {
                    Some(d) if d.removed => {
                        let what = if d.action == "ref_removed" {
                            format!("已从 {} 摘掉引用(标签还在)", d.name)
                        } else {
                            format!("已删除标签 {}", d.name)
                        };
                        app.push_log(format!(
                            "{what}:剩余 {} 个标签(一次 git 变更,走 PR + 家长审)",
                            d.remaining
                        ));
                        app.run_tag_ls(None);
                    }
                    Some(d) => {
                        app.push_log(format!(
                            "没删成:{} 不在编目里(action {}),tags.json 一个字节都没动",
                            d.name, d.action
                        ));
                    }
                    None => {
                        let h = app
                            .tag_rm_result
                            .as_ref()
                            .map(|o| o.headline())
                            .unwrap_or_default();
                        app.push_log(format!("删除结果不可用:{h}"));
                    }
                }
            }
            CatalogKind::Subscription => {
                let typed = out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::SubRmData>());
                app.sub_rm_result = Some(out);
                match typed {
                    Some(d) if d.removed => {
                        app.push_log(format!(
                            "已删除订阅 {}:剩余 {} 条(一次 git 变更,走 PR + 家长审)",
                            d.name, d.remaining
                        ));
                        app.run_sub_ls();
                    }
                    Some(d) => {
                        app.push_log(format!(
                            "没删成:{} 不在编目里(action {}),subscriptions.json 一个字节都没动",
                            d.name, d.action
                        ));
                    }
                    None => {
                        let h = app
                            .sub_rm_result
                            .as_ref()
                            .map(|o| o.headline())
                            .unwrap_or_default();
                        app.push_log(format!("删除结果不可用:{h}"));
                    }
                }
            }
            CatalogKind::Key => {
                let typed = out
                    .envelope
                    .as_ref()
                    .and_then(|e| e.data_as::<crate::engine::KeyRmData>());
                app.key_rm_result = Some(out);
                match typed {
                    Some(d) if d.removed => {
                        app.push_log(format!(
                            "已删除钥匙 {}:剩余 {} 条(一次 git 变更,走 PR + 家长审)",
                            d.name, d.remaining
                        ));
                        app.run_key_ls();
                    }
                    Some(d) => {
                        app.push_log(format!(
                            "没删成:{} 不在编目里(action {}),keys.json 一个字节都没动",
                            d.name, d.action
                        ));
                    }
                    None => {
                        let h = app
                            .key_rm_result
                            .as_ref()
                            .map(|o| o.headline())
                            .unwrap_or_default();
                        app.push_log(format!("删除结果不可用:{h}"));
                    }
                }
            }
        }
    }

    /// 这一本的删除确认弹层是否打开(ADR-0009 §2:面板加一道图形确认,
    /// 不改引擎语义;四本各一面旗)。
    pub fn rm_confirm(self, app: &App) -> bool {
        match self {
            CatalogKind::Vps => app.vps_rm_confirm,
            CatalogKind::Tag => app.tag_rm_confirm,
            CatalogKind::Subscription => app.sub_rm_confirm,
            CatalogKind::Key => app.key_rm_confirm,
        }
    }

    // ---- 册级登记 / 删除入口查表(工单 #76:`start_*` 八份收成一份) ----
    //
    // 下面这一族把「这一本的登记入口开不开 / 入口指路 / 复位表单 / 选中名 /
    // 起删除确认 / 登记提示口径」收成 `CatalogKind` 的成员。调用方
    // (`App::start`)只写 `kind.xxx(app)`,**不再有第二处 `match kind` /
    // `if kind ==`**:登记 / 删除八份 `start_*` 收成一份 `App::start(kind, mode)`,
    // 按册差异(入口门、表单形态、删除是进确认弹层还是嵌套进表单流、提示口径)
    // 全落在这一族成员上。
    //
    // 加一条按册的登记 / 删除关切 = 在这里加 / 改一个成员,不是往调用方再插
    // 一个分支——第五本册子来时同样只改这一处。

    /// 这一本的删除确认旗(可写侧;登记入口先清它,删除入口按册置位——
    /// 标签册的删除嵌套在表单流里,confirm 旗恒 false)。
    pub fn rm_confirm_set(self, app: &mut App, val: bool) {
        match self {
            CatalogKind::Vps => app.vps_rm_confirm = val,
            CatalogKind::Tag => app.tag_rm_confirm = val,
            CatalogKind::Subscription => app.sub_rm_confirm = val,
            CatalogKind::Key => app.key_rm_confirm = val,
        }
    }

    /// 这一本的登记入口开不开(#29 能力门 + #27 数据仓根;四本各问自己的
    /// `*_entry_enabled`,调用方不再分册)。
    pub fn entry_enabled(self, app: &App) -> bool {
        match self {
            CatalogKind::Vps => app.vps_entry_enabled(),
            CatalogKind::Tag => app.tag_entry_enabled(),
            CatalogKind::Subscription => app.sub_entry_enabled(),
            CatalogKind::Key => app.key_entry_enabled(),
        }
    }

    /// 入口不可用时的一句指路(与册体提示同源;四本各问自己的 `*_gate_hint`)。
    pub fn gate_hint(self, app: &App) -> String {
        match self {
            CatalogKind::Vps => app.vps_gate_hint(),
            CatalogKind::Tag => app.tag_gate_hint(),
            CatalogKind::Subscription => app.sub_gate_hint(),
            CatalogKind::Key => app.key_gate_hint(),
        }
    }

    /// 复位这一本的登记表单(登记 / 挂标签入口进表单态前清干净:机器 /
    /// 订阅 / 钥匙三本各新建一张空表单,标签册新建 Add 态空表单)。
    pub fn form_reset(self, app: &mut App) {
        match self {
            CatalogKind::Vps => app.vps_form = new_form(),
            CatalogKind::Tag => app.tag_form = new_tag_form(TagFormMode::Add),
            CatalogKind::Subscription => app.sub_form = new_sub_form(),
            CatalogKind::Key => app.key_form = new_key_form(),
        }
    }

    /// 这一本此刻选中的条目名(删除确认 / 反查同源;经过滤后的可见列表映射
    /// 回原始投影,四本各问自己的 `selected_*_name`)。
    pub fn selected_name(self, app: &App) -> Option<String> {
        match self {
            CatalogKind::Vps => app.selected_vps_name(),
            CatalogKind::Tag => app.selected_tag_name(),
            CatalogKind::Subscription => app.selected_sub_name(),
            CatalogKind::Key => app.selected_key_name(),
        }
    }

    /// 登记 / 删除入口那句提示里的动词(按册:机器 / 订阅 / 钥匙三本「登记」,
    /// 标签册「挂标签」)。装配冲突句与入口门句都拿它拼,四本各一句。
    pub fn register_verb(self) -> &'static str {
        match self {
            CatalogKind::Vps | CatalogKind::Subscription | CatalogKind::Key => "登记",
            CatalogKind::Tag => "挂标签",
        }
    }

    /// 登记 / 挂标签入口进表单态后那句提示(四本各一句,逐字节不同)。
    pub fn register_prompt(self) -> &'static str {
        match self {
            CatalogKind::Vps => "登记机器:填名字/地址/端口/登录/备注,Enter 预审,Esc/q 返航",
            CatalogKind::Tag => "挂标签:填标签名与若干条 kind/条目 引用,Enter 挂上去,Esc/q 返航",
            CatalogKind::Subscription => {
                "登记订阅:填名字/应用/到期日,在链接框贴一次本体,Enter 预审,Esc/q 返航"
            }
            CatalogKind::Key => {
                "登记钥匙:填名字/主机/作用域/轮换日,在钥匙框贴一次本体,Enter 预审,Esc/q 返航"
            }
        }
    }

    /// 起这一本的删除(删除键 `d` 的按册分叉下沉处):机器 / 订阅 / 钥匙三本
    /// 置 rm 确认旗 + 打确认提示;标签册的删除嵌套在表单流里(预填选中标签名,
    /// 光标落引用字段,引用留空 = 删整个标签)。
    pub fn start_rm(self, app: &mut App, name: String) {
        match self {
            CatalogKind::Vps | CatalogKind::Subscription | CatalogKind::Key => {
                self.raise_rm_confirm(app, &name);
            }
            CatalogKind::Tag => {
                app.tag_rm_confirm = false;
                app.tag_flow.begin();
                app.tag_form = new_tag_form(TagFormMode::Rm);
                app.tag_form.name = name.clone();
                // 光标落在引用字段:名字已预填,摘引用/删标签的动作全在引用行
                // (留空 = 删整个标签)。不预填该标签现有的引用——摘哪几条由操作者
                // 明确点出来,面板不替他决定(全预填 + 一路 Enter = 悄悄摘光)。
                app.tag_form.field = TagField::Refs;
                app.push_log(format!(
                    "摘引用 / 删标签 {name}:引用留空 = 删整个标签,填 kind/条目 = 只摘那几条"
                ));
            }
        }
    }

    // ---- 册级命令下发查表(工单 #76:`dispatch_*` 十一份 + 八个键位处理器 +
    //      四份 cancel 收成一份) ----
    //
    // 二十三个按册复制收成六个调用方入口(`App::dispatch_preaudit` /
    // `dispatch_commit` / `dispatch_rm` / `on_key_flow` / `on_key_rm_confirm` /
    // `App::cancel`,外加同形状的 `App::dismiss_rm`)。四本册各留一个 `cancel_*`
    // 转口是给测试直接驱动用的(不改写测试),实现仍只有一份。
    //
    // 下面这一族把「表单怎么定型 / 表单键位 / 写命令递去哪条队列 / 有没有确认闸 /
    // 表单 Enter 之后走哪条路 / 起删除确认 / 预审与落账两句日志 / 删除取谁 /
    // 删除 argv / 删除下发后的收尾 / 返航口径」收成 `CatalogKind` 的成员。调用方
    // (`App::dispatch_preaudit` / `dispatch_commit` / `dispatch_rm` /
    // `on_key_flow` / `on_key_rm_confirm` / `App::cancel`)只写 `kind.xxx(app)`,
    // **不再有第二处 `match kind` / `if kind ==`**:四本册同形的骨架(两道守卫 →
    // 面板侧定型 → 按这一本的路由递出去)只有一份,按册差异(标签册没有确认闸且
    // 删除嵌套在表单流里、订阅 / 钥匙两本本体经 stdin 喂入、各自的日志口径)
    // 全落在这一族成员上。
    //
    // 加一条按册的下发关切 = 在这里加 / 改一个成员,不是往调用方再插一个分支。

    /// 这一本的表单 → 登记规格(面板侧解析,不喂给引擎猜)。四本各问自己的
    /// `spec_from_*`;统一收成 `Box<dyn RegisterSpec>` 是为了让调用方那一份
    /// 骨架不分册(问 [`RegisterSpec::name`] 与两段式两个 argv 方法就够)。
    pub fn spec_from_form(self, app: &App) -> Result<Box<dyn RegisterSpec>, Vec<String>> {
        match self {
            CatalogKind::Vps => {
                spec_from_form(&app.vps_form).map(|s| Box::new(s) as Box<dyn RegisterSpec>)
            }
            CatalogKind::Tag => {
                spec_from_tag_form(&app.tag_form).map(|s| Box::new(s) as Box<dyn RegisterSpec>)
            }
            CatalogKind::Subscription => {
                spec_from_sub_form(&app.sub_form).map(|s| Box::new(s) as Box<dyn RegisterSpec>)
            }
            CatalogKind::Key => {
                spec_from_key_form(&app.key_form).map(|s| Box::new(s) as Box<dyn RegisterSpec>)
            }
        }
    }

    /// 这一本登记表单的一个键(四本各问自己的 `on_key_*_form`;调用方
    /// `App::on_key_flow` 的表单段只写这一句,不再分册)。
    pub fn form_key(self, app: &mut App, k: crossterm::event::KeyEvent) -> FormAction {
        match self {
            CatalogKind::Vps => on_key_form(&mut app.vps_form, k),
            CatalogKind::Tag => on_key_tag_form(&mut app.tag_form, k),
            CatalogKind::Subscription => on_key_sub_form(&mut app.sub_form, k),
            CatalogKind::Key => on_key_key_form(&mut app.key_form, k),
        }
    }

    /// 引擎这一组**有没有自己的确认闸**(`--yes`;[`RegisterSpec::confirm_gate`]
    /// 的册级转发,不在这里重述一遍答案)。`false` = 只有标签册:上游
    /// `tag add` 追加合并、幂等,没有「覆盖」可确认,ADR-0009 §2 不许面板自己
    /// 造一道。消费方两处:`App::on_key_flow` 的确认段认不认「Enter 落账」,
    /// `App::dispatch_commit` 的落账闸认不认(标签册从表单 Enter 直接进写段)。
    pub fn confirm_gate(self) -> bool {
        match self {
            CatalogKind::Vps => VpsSpec::confirm_gate(),
            CatalogKind::Tag => TagSpec::confirm_gate(),
            CatalogKind::Subscription => SubscriptionSpec::confirm_gate(),
            CatalogKind::Key => KeySpec::confirm_gate(),
        }
    }

    /// 表单 Enter 之后,这一本的写命令走哪条路。**按册差异是控制流**,所以它
    /// 作为查表数据落在这一族成员上,不落进调用方的 if:机器 / 订阅 / 钥匙三本
    /// 有两段式(第一段预审),标签册没有——`Add` 直接进写段,`Rm` 只起面板侧
    /// 那道图形确认(它的删除嵌套在表单流里)。
    pub fn write_route(self, app: &App) -> WriteRoute {
        match self {
            CatalogKind::Vps | CatalogKind::Subscription | CatalogKind::Key => WriteRoute::Preaudit,
            CatalogKind::Tag => match app.tag_form.mode {
                TagFormMode::Add => WriteRoute::Commit,
                TagFormMode::Rm => WriteRoute::RmConfirm,
            },
        }
    }

    /// 起这一本的删除确认弹层(ADR-0009 §2:面板只决定「什么时候递出去」,下发的
    /// argv 与在 shell 里敲的逐字相同)。**四本册同一句**:机器 / 订阅 / 钥匙三本
    /// 由 `d` 键经 [`Self::start_rm`] 起到;标签册由表单 Enter 经
    /// [`App::dispatch_preaudit`](crate::app::App::dispatch_preaudit) 起到——
    /// 它的删除嵌套在表单流里,旗起时表单已在 Rm 态,所以这里**不碰表单**
    /// (碰了就把操作者刚填的引用行抹了)。
    pub fn raise_rm_confirm(self, app: &mut App, name: &str) {
        self.rm_confirm_set(app, true);
        app.push_log(format!("删除确认:{name}(Enter 确认,Esc/q 取消)"));
    }

    /// 把这一趟写命令递出去,并按这一本的机密纪律决定进哪条队列:订阅 / 钥匙
    /// 两本的本体**只经 stdin 进**(ADR-0009 §3),argv 与本体一起进
    /// `fed_queue`(喂入是一次性的,两段式各喂一次);机器 / 标签两本只进
    /// `queue`。返回喂入的字节数(没喂 = `None`)——日志只报字节数,内容不进
    /// 任何字符串。
    pub fn dispatch_cmd(self, app: &mut App, argv: Vec<String>) -> Option<usize> {
        let stdin = match self {
            CatalogKind::Subscription => Some((app.sub_form.body.len(), app.sub_form.body.clone())),
            CatalogKind::Key => Some((app.key_form.body.len(), app.key_form.body.clone())),
            CatalogKind::Vps | CatalogKind::Tag => None,
        };
        match stdin {
            Some((n, body)) => {
                app.fed_queue.push((argv, body));
                Some(n)
            }
            None => {
                app.queue.push(argv);
                None
            }
        }
    }

    /// 入口门 / 数据仓根不过时那句的开头(按册:机器 / 订阅 / 钥匙三本
    /// 「登记不下发:」,标签册「未下发:」——它的写命令没有两段式,「登记」
    /// 这个词不贴)。
    pub fn dispatch_gate_prefix(self) -> &'static str {
        match self {
            CatalogKind::Vps | CatalogKind::Subscription | CatalogKind::Key => "登记不下发:",
            CatalogKind::Tag => "未下发:",
        }
    }

    /// 面板侧拒下(空名字 / 形状不合)那句的开头(按册:机器 / 订阅 / 钥匙三本
    /// 「登记被拒:」,标签册「被拒:」)。
    pub fn dispatch_reject_prefix(self) -> &'static str {
        match self {
            CatalogKind::Vps | CatalogKind::Subscription | CatalogKind::Key => "登记被拒:",
            CatalogKind::Tag => "被拒:",
        }
    }

    /// 预审 / 落账两句日志里的命令组名(按册:vps register /
    /// subscription register / key register;标签册没有这两段,给它的写命令名
    /// `tag add`)。
    pub fn register_cmd(self) -> &'static str {
        match self {
            CatalogKind::Vps => "vps register",
            CatalogKind::Tag => "tag add",
            CatalogKind::Subscription => "subscription register",
            CatalogKind::Key => "key register",
        }
    }

    /// 预审段入队那句(按册:订阅 / 钥匙两本 argv 带 `--name`、且报本体喂入
    /// 字节数,机器册两样都不带)。**标签册没有预审段**(`confirm_gate` 为
    /// `false`,见 [`Self::write_route`]),这一臂对它永不成立。
    pub fn preaudit_log(self, name: &str, fed: Option<usize>) -> String {
        match self {
            CatalogKind::Vps => format!("登记预审已入队:vps register {name}(不带 --yes,文件不落)"),
            CatalogKind::Tag => format!("登记预审已入队:tag add {name}(追加合并、幂等,无确认闸)"),
            CatalogKind::Subscription | CatalogKind::Key => {
                let fed = fed.map_or_else(String::new, |n| format!(";已喂入 {n} 字节"));
                format!(
                    "登记预审已入队:{} --name {name}(不带 --yes,文件不落{fed})",
                    self.register_cmd()
                )
            }
        }
    }

    /// 落账段第一道闸不过那句(按册:预审摊开几道闸——机器五道 / 标签与订阅
    /// 四道 / 钥匙三道)。**标签册不经落账段**(见 [`Self::confirm_gate`]),
    /// 这一臂对它永不成立。
    pub fn commit_gate_msg(self) -> &'static str {
        match self {
            CatalogKind::Vps => "落账未下发:预审未通过(五道闸有 fail 或退出码非 0)",
            CatalogKind::Tag => "落账未下发:预审未通过(四道闸有 fail 或退出码非 0)",
            CatalogKind::Subscription => "落账未下发:预审未通过(四道闸有 fail 或退出码非 0)",
            CatalogKind::Key => "落账未下发:预审未通过(三道闸有 fail 或退出码非 0)",
        }
    }

    /// 落账段入队那句(按册:机器册裸位置参数,订阅 / 钥匙两本 argv 带
    /// `--name` 且报再喂入的字节数;标签册的那一句就是它的 `tag add` 写段——
    /// 它没有确认闸,这一句由 [`Self::write_route`] 的 Commit 臂直接到达)。
    pub fn commit_log(self, name: &str, root: &str, fed: Option<usize>) -> String {
        match self {
            CatalogKind::Vps => {
                format!("登记落账已入队:vps register {name} --yes(--root {root})")
            }
            CatalogKind::Tag => {
                format!("挂标签已下发:tag add {name}(追加合并、幂等,无确认闸;--root {root})")
            }
            CatalogKind::Subscription | CatalogKind::Key => {
                let fed = fed.map_or_else(String::new, |n| format!(";已再喂入 {n} 字节"));
                format!(
                    "登记落账已入队:{} --name {name} --yes(--root {root}{fed})",
                    self.register_cmd()
                )
            }
        }
    }

    /// 这一次删除删谁(按册:机器 / 订阅 / 钥匙三本取当前选中行;#69 起经过滤后
    /// 的可见列表映射回原始投影)。**标签册的删除嵌套在表单流里**,目标由表单
    /// 定型——取不到即表单已不可定型(与另三本「没有选中行」同一形状:
    /// 调用方收旗 + 按册留一句,不下发)。
    pub fn rm_target(self, app: &App) -> Option<String> {
        match self {
            CatalogKind::Vps | CatalogKind::Subscription | CatalogKind::Key => {
                self.selected_name(app)
            }
            CatalogKind::Tag => spec_from_tag_form(&app.tag_form).ok().map(|s| s.name),
        }
    }

    /// 取不到删除目标那句(按册:三本「没有选中的<册名>」,标签册「表单已不可
    /// 定型」)。
    pub fn rm_missing_log(self) -> &'static str {
        match self {
            CatalogKind::Vps => "删除未下发:没有选中的机器",
            CatalogKind::Tag => "删除未下发:表单已不可定型(返航后从表单重来)",
            CatalogKind::Subscription => "删除未下发:没有选中的订阅",
            CatalogKind::Key => "删除未下发:没有选中的钥匙",
        }
    }

    /// 清这一本上一轮的 rm 回音(与 `b_rm_result` 同一张表;新一趟删除下发前
    /// 清干净,旧结论不冒充本次)。
    pub fn rm_result_clear(self, app: &mut App) {
        match self {
            CatalogKind::Vps => app.vps_rm_result = None,
            CatalogKind::Tag => app.tag_rm_result = None,
            CatalogKind::Subscription => app.sub_rm_result = None,
            CatalogKind::Key => app.key_rm_result = None,
        }
    }

    /// 这一本删除命令的 argv(与在 shell 里敲的逐字相同,面板只决定何时递出去)。
    /// 标签册多一层:引用行为空 = 删整个标签,带引用 = 只摘那几条——引用从
    /// 表单现取(纯函数,与 [`Self::rm_target`] 同一次定型)。
    pub fn rm_cmd(self, app: &App, root: &str, name: &str) -> Vec<String> {
        match self {
            CatalogKind::Vps => crate::engine::commands::vps_rm(name, root),
            CatalogKind::Tag => {
                let refs = spec_from_tag_form(&app.tag_form)
                    .map(|s| s.refs)
                    .unwrap_or_default();
                crate::engine::commands::tag_rm(name, &refs, root)
            }
            CatalogKind::Subscription => crate::engine::commands::subscription_rm(name, root),
            CatalogKind::Key => crate::engine::commands::key_rm(name, root),
        }
    }

    /// 删除下发后的收尾与那句日志。**标签册多两步**(确认即消费):收流并复位
    /// 表单——否则上一张引用行还摊在那儿,键会继续往里灌;引用条数也在这里现取
    /// (纯函数,表单到这一步之前没有被改过)。其余三本只打日志。
    pub fn rm_consumed(self, app: &mut App, name: &str, root: &str) {
        match self {
            CatalogKind::Vps => app.push_log(format!("删除已入队:vps rm {name} --root {root}")),
            CatalogKind::Tag => {
                let refs = spec_from_tag_form(&app.tag_form)
                    .map(|s| s.refs.len())
                    .unwrap_or(0);
                // 表单的使命到此结束(确认即消费)
                app.tag_flow.abandon();
                app.tag_form = new_tag_form(TagFormMode::Add);
                let scope = if refs == 0 {
                    "删整个标签".to_string()
                } else {
                    format!("只摘 {refs} 条引用")
                };
                app.push_log(format!("删除已入队:tag rm {name}({scope};--root {root})"));
            }
            CatalogKind::Subscription => {
                app.push_log(format!("删除已入队:subscription rm {name} --root {root}"))
            }
            CatalogKind::Key => app.push_log(format!("删除已入队:key rm {name} --root {root}")),
        }
    }

    /// 返航那句日志(按册:机器册「登记返航」;订阅 / 钥匙两本多说一句在途本体
    /// 已 drop;标签册是「标签册返航」)。清状态那一半不走查表——四本同一条
    /// (`App::cancel` 里 `abandon` + [`Self::form_reset`])。
    pub fn cancel_log(self) -> &'static str {
        match self {
            CatalogKind::Vps => "登记返航",
            CatalogKind::Tag => "标签册返航",
            CatalogKind::Subscription => "登记返航(在途链接本体已 drop,面板侧不留副本)",
            CatalogKind::Key => "登记返航(在途钥匙本体已 drop,面板侧不留副本)",
        }
    }

    /// 这一本册体的条数(没读到 = 0;与 [`Self::sel`] 同一张表)。
    pub fn len(self, app: &App) -> usize {
        match self {
            CatalogKind::Vps => app.vps_len(),
            CatalogKind::Tag => app.tag_len(),
            CatalogKind::Subscription => app.sub_len(),
            CatalogKind::Key => app.key_len(),
        }
    }

    /// 这一本的选中行(↑/↓ 与鼠标点行同源;四本各一个槽,换册不串味)。
    pub fn sel(self, app: &App) -> usize {
        match self {
            CatalogKind::Vps => app.vps_sel,
            CatalogKind::Tag => app.tag_sel,
            CatalogKind::Subscription => app.sub_sel,
            CatalogKind::Key => app.key_sel,
        }
    }

    /// 这一本的选中槽(可写侧;键位 ↑/↓、鼠标点行、滚轮移选中都走这一口)。
    pub fn sel_mut(self, app: &mut App) -> &mut usize {
        match self {
            CatalogKind::Vps => &mut app.vps_sel,
            CatalogKind::Tag => &mut app.tag_sel,
            CatalogKind::Subscription => &mut app.sub_sel,
            CatalogKind::Key => &mut app.key_sel,
        }
    }

    /// 这一本的登记确认态是否放行落账。**标签册恒 `false`**:上游
    /// `tag add` 没有确认闸(追加合并、幂等),ADR-0009 §2 不许面板自己造
    /// 一道——所以标签册的确认屏没有「Enter 落账」段可点,这一条就是那道闸。
    pub fn can_commit(self, app: &App) -> bool {
        match self {
            CatalogKind::Vps => app.vps_can_commit(),
            CatalogKind::Tag => false,
            CatalogKind::Subscription => app.sub_can_commit(),
            CatalogKind::Key => app.key_can_commit(),
        }
    }

    /// 反查回音的失败态(带参问错了名字 = rc 3 / 回音不可定型)。
    /// `None` = 没有失败态。**只标签册有这一态**:机器册的 `vps ls` 没有
    /// 位置参数,失败时只留日志(51 §6.11:两种语义不都画成空列表)。
    /// 「只有一本册成立」是一条按册关切,所以它落在查表成员上(#66):
    /// 其余三本恒 `None`,调用方不再写 `if kind != Tag`。
    pub fn ls_error(self, app: &App) -> Option<&EngineOutput> {
        if self != CatalogKind::Tag {
            return None;
        }
        let out = app.tag_ls_echo.as_ref()?;
        let typed = out.envelope.as_ref().and_then(|e| e.data_as::<TagLsData>());
        (typed.is_none() || !out.ok).then_some(out)
    }

    /// 册体首行的口径词:机器册给册名,标签册把「列全部 / 只列一个」
    /// 这档差别显式画出来(51 §6.11:两种语义不都画成空列表)。
    pub fn scope_label(self, app: &App) -> String {
        match self {
            CatalogKind::Vps => "机器".to_string(),
            CatalogKind::Tag => match app.tag_lookup.as_deref() {
                Some(name) => format!("反查 {name}"),
                None => "全部标签".to_string(),
            },
            CatalogKind::Subscription => "全部订阅".to_string(),
            CatalogKind::Key => "全部钥匙".to_string(),
        }
    }

    /// 换到这一本时,日志里那一句「接下来按 l 做什么」(各本只读调用的
    /// 口径不同:标签册是反查、钥匙册明示无本体槽、订阅册含到期三态)。
    pub fn next_hint(self) -> &'static str {
        match self {
            CatalogKind::Vps => "按 l 重读机器编目(只读)",
            CatalogKind::Tag => "按 l 反查全部标签(只读)",
            CatalogKind::Subscription => "按 l 重读订阅编目(只读,含到期三态)",
            CatalogKind::Key => "按 l 重读钥匙编目(只读,无本体槽、不印指纹)",
        }
    }

    /// 这一本的投影(四本各自的数据源汇到同一个 [`CatalogSheet`])。
    ///
    /// **这是全模块唯一一处「四本各读哪个状态」的匹配**:四本的数据源
    /// 形状本就不同(`Option<VpsLsData>` / 回音 envelope / `Option<SubLsData>`
    /// / `Option<KeyLsData>`),没法再拆成更小的查表——那就让这一处匹配
    /// 长在 [`CatalogKind`] 上,调用方(`App::catalog_sheet` 及渲染)只写
    /// `kind.sheet(app)`,不再各自 `match self.catalog_kind`。
    pub fn sheet(self, app: &App) -> Option<CatalogSheet> {
        match self {
            CatalogKind::Vps => app.vps_ls.as_ref().map(|d| d.sheet()),
            CatalogKind::Tag => app
                .tag_ls_echo
                .as_ref()
                .filter(|o| o.ok)
                .and_then(|o| o.envelope.as_ref())
                .and_then(|e| e.data_as::<TagLsData>())
                .map(|d| d.sheet()),
            CatalogKind::Subscription => app.sub_ls.as_ref().map(|d| d.sheet()),
            CatalogKind::Key => app.key_ls.as_ref().map(|d| d.sheet()),
        }
    }

    /// 这一本的一条条目**参与过滤的字段**(#69,与 #61 五域 `domain_row_fields`
    /// 同一族口径):按册取该行在列表与明细里出现过的文本。**只收引擎真给了的
    /// 字段**,缺字段 / 占位符「——」一律不入匹配面——不猜、不拿「——」冒充
    /// 可检索内容(规格 §3「每册一份可匹配字段」)。机器 = 名/地址/登录/备注;
    /// 标签 = 名/引用;订阅 = 名/应用/到期日;钥匙 = 名/主机/作用域/轮换日。
    /// 落在查表成员上(#66 模式):调用方(`App::catalog_visible`)只写
    /// `kind.match_fields(e)`,不再有四份按册匹配分支。
    pub fn match_fields(self, e: &CatalogEntryView) -> Vec<String> {
        let mut v = vec![e.name.clone()];
        match self {
            CatalogKind::Vps => {
                for a in &e.addrs {
                    v.push(a.label.clone());
                    v.push(a.addr.clone());
                }
                for l in &e.logins {
                    v.push(l.user.clone());
                    v.push(l.key_ref.clone());
                }
                if let Some(n) = e.note.as_deref().filter(|s| !s.is_empty()) {
                    v.push(n.to_string());
                }
            }
            CatalogKind::Tag => {
                for r in &e.refs {
                    v.push(r.label());
                }
            }
            CatalogKind::Subscription => {
                if let Some(c) = e.client.as_deref().filter(|s| !s.is_empty()) {
                    v.push(c.to_string());
                }
                if let Some(x) = e
                    .expires
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    v.push(x.to_string());
                }
            }
            CatalogKind::Key => {
                if let Some(h) = e.host.as_deref().filter(|s| !s.is_empty()) {
                    v.push(h.to_string());
                }
                v.extend(e.scopes.iter().cloned());
                if let Some(r) = e
                    .rotated_at
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    v.push(r.to_string());
                }
            }
        }
        v
    }

    /// 册体「还没读到」的一句话(合法空态之前的第三态:一次 ls 都没成功过)。
    /// 四本各一句——按册查表,渲染不再 `match kind` 四回。
    pub fn unread_line(self) -> &'static str {
        match self {
            CatalogKind::Vps => "  编目册还没读到:按 r 登记,或先刷新(回概览按 r 重读五域)。",
            CatalogKind::Tag => "  还没反查过:按 l 列全部标签(只读),按 r 挂第一个标签。",
            CatalogKind::Subscription => "  订阅册还没读到:按 l 重读(只读),按 r 登记第一条订阅。",
            CatalogKind::Key => "  钥匙册还没读到:按 l 重读(只读),按 r 登记第一把钥匙。",
        }
    }

    /// 册体「合法空态」的两行:`(标题, 下一步)`。引擎给了空数组 = 册子
    /// 存在而里面没有条目(文件不存在也是这一态),与「还没读到」分开说
    /// (三态不共用一个空列表)。
    pub fn empty_lines(self) -> [&'static str; 2] {
        match self {
            CatalogKind::Vps => [
                "  编目里还没有机器(合法空态:vps.json 不存在 = 空册)。",
                "  按 r 登记第一台(两段式:预审摊开五道闸 → 图形确认才落账)。",
            ],
            CatalogKind::Tag => [
                "  编目里还没有标签(合法空态:tags.json 不存在 = 空册)。",
                "  按 r 挂第一个标签(追加合并、幂等、无确认闸,Enter 即下发)。",
            ],
            CatalogKind::Subscription => [
                "  编目里还没有订阅(合法空态:subscriptions.json 不存在 = 空册)。",
                "  按 r 登记第一条(两段式:链接本体只经 stdin 进,Enter 预审摊开四道闸)。",
            ],
            CatalogKind::Key => [
                "  编目里还没有钥匙(合法空态:keys.json 不存在 = 空册)。",
                "  按 r 登记第一把(两段式:钥匙本体只经 stdin 进,Enter 预审摊开三道闸)。",
            ],
        }
    }

    /// 册体清单之外、**只属于某一本**的附述行(订阅册的到期三态汇总与
    /// 取回链接建议;钥匙册的取回钥匙建议)。机器册与标签册没有这一族
    /// 关切,给空——所以渲染侧不再有 `if kind == Subscription` /
    /// `if kind == Key` 两段。
    pub fn extra_lines(self, app: &App, w: usize, p: &Palette) -> Vec<Line<'static>> {
        match self {
            CatalogKind::Subscription => {
                let mut out = Vec::new();
                if let Some(s) = self.sheet(app) {
                    if !s.entries.is_empty() {
                        out.extend(sub_summary_lines(&sub_status_counts(&s), w, p));
                    }
                }
                out.extend(sub_get_lines(app, w, p));
                out
            }
            CatalogKind::Key => key_get_lines(app, w, p),
            CatalogKind::Vps | CatalogKind::Tag => Vec::new(),
        }
    }

    /// 这一本的边界表述(ADR-0009 §5:编目条目绝不进配方 / 锁定集 /
    /// bundle / 装配命令)。**机器册没有这一句**(vps 条目不进派这条纪律
    /// 由装配屏那边守),给 `None` = 不画行——渲染侧不再 `match kind`。
    pub fn boundary(self) -> Option<&'static str> {
        match self {
            CatalogKind::Vps => None,
            CatalogKind::Tag => Some(TAG_BOUNDARY),
            CatalogKind::Subscription => Some(SUB_BOUNDARY),
            CatalogKind::Key => Some(KEY_BOUNDARY),
        }
    }

    /// 空册时的册内键位行**段清单**(与鼠标命中区同文案同序;四本各一组)。
    /// 给段而不是给一整行字符串,好让 #67 的可点「换册」段接在末段——渲染
    /// 与命中共用同一份段清单,一处真相。新增第五本册只加一行。
    ///
    /// 机器册空册态与有条目态同一口径(#65 评审第 8 条):`q` 在编目册册体上
    /// 是壳级「退出」(`on_key` 的 `Char('q') => quit`),**不是返航**——`keys_body`
    /// 已按这条收窄成裸 `Esc`,空册态却还写着 `Esc/q 返航`,同一块屏上两个态
    /// 对同一个键说两样话。这里跟上:`Esc`(语义同册体,清当前册过滤词;
    /// 空册没词时它不出声,但至少不虚指一个 quit 键当返航)。
    ///
    /// 另三本的空册态仍写 `Esc/q 返航`,是另一件既有事:它们的 `keys_body`
    /// 根本不给 Esc 提示段(整行没有它),所以本册内并不自相矛盾;要跨四本
    /// 统一口径得连册体一起动,那是另一条工单,不夹在本评审里。
    ///
    /// 段的第三项是 `KeyPart` 的命中目标:`None` = 只画不点的提示段,
    /// `Some(..)` = 可点动作段。**空册变体一个可点段都没有**——册里没有条目,
    /// 「取回哪一条」无从问起(#73 验收:空册变体没有取回命中目标)。
    pub fn keys_empty(self) -> &'static [crate::layout::KeyPart<'static>] {
        match self {
            CatalogKind::Vps => &[("  r 登记机器", None, true), ("Esc", None, true)],
            CatalogKind::Tag => &[
                ("  l 反查全部", None, true),
                ("r 挂标签", None, true),
                ("Esc/q 返航", None, true),
            ],
            CatalogKind::Subscription => &[
                ("  r 登记订阅", None, true),
                ("l 重读", None, true),
                ("Esc/q 返航", None, true),
            ],
            CatalogKind::Key => &[
                ("  r 登记钥匙", None, true),
                ("l 重读", None, true),
                ("Esc/q 返航", None, true),
            ],
        }
    }

    /// 册体有条目时的册内键位行**段清单**(常态)。
    ///
    /// 机器册这一段在 #70 收窄过两处,好让「Enter 反查」可点段在 66×50 的
    /// 紧凑态也放得下(#67 的纪律:放不下就不画也不给命中,而规格故事 52 要求
    /// 新命中区两档都算得对):`↑/↓ 选机器` → `↑/↓ 选`(选哪一本 HUD 已说),
    /// `Esc/q 返航` → `Esc 返航`(**`q` 在册体上是退出面板,不是返航**——旧文案
    /// 把两个语义混成一句;浮层里的 `Esc/q 返航` 仍两键都认,那句不动)。
    ///
    /// 「Enter 反查」**不写在这里**:它是可点动作段,由 `catalog_key_parts`
    /// 按「只机器册 + 有选中行」现拼(提示段与可点段在一处汇合,渲染与命中
    /// 共用同一份清单);写进这张静态表就会在订阅 / 钥匙册上也画出一个点不动
    /// 的段,也会在空册上问一台不存在的机器。
    ///
    /// #69 合入后再收窄第三处:`Esc 返航` → `Esc`。#69 的 `/ 过滤`、#70 的
    /// `Enter 反查`、#67 的 `m 换册` 三个可点段在机器册键位行同场,且三段的
    /// 点击命中都在 80×30 / 66×50 两档钉死;不把这段提示再收一档,紧凑态下
    /// `/ 过滤` 会被整段裁掉、点不到(#69 的过滤行为就不完整)。册体上 `Esc`
    /// 只做「清当前册过滤词」(见 `key_catalog`),裸 `Esc` 足够提示这个键、语义
    /// 也不虚,故只留键名、去掉「返航」二字。
    ///
    /// **`g 取回` 段挂在本表里**(#73):段文案与命中目标
    /// (`Some(Target::CatalogGet)`)都由这一行查表数据带着——订阅 / 钥匙册两行
    /// 各一个,机器册 / 标签册两行没有(它们的条目没有本体可取,
    /// `CatalogKind::action` 里也没有 `g` 这一行)。
    /// 于是「这一段可不可点」与「这个键在这一本上有没有动作」是同一张表的同一
    /// 行,`catalog_key_parts` 不为本票加 `if kind ==` 分叉(#66 查表纪律);点下去
    /// 走 [`App::log_sub_get_suggest`] / [`App::log_key_get_suggest`],与按 `g`
    /// 逐字节同一条路。
    ///
    /// **`d 删除` 段也挂在本表里**(#78,兑 #75 裁决 A):四本册各一行,命中目标
    /// `Some(Target::CatalogDelete)`——点它 = 按 `d`,汇到 [`App::start`] 的
    /// `StartMode::Rm` 一个入口(与 `key_catalog` 受理 `d` 同一个函数、同一条
    /// 后果链:选中行 → ADR-0009 图形确认闸 → 确认才 `rm`)。段面按册取文案:
    /// 机器 / 订阅 / 钥匙三本 [`CATALOG_DELETE`],标签册
    /// [`CATALOG_DELETE_TAG`](它的删除是摘引用 / 删整个标签两态,嵌套在表单流
    /// 里,不是确认弹层)。无选中行时点下去复现键盘 `d` 的既有指路
    /// (「删除未下发:没有选中的…」),不新造灰置态;#71 的 `x` 已经证明可点的
    /// 重段不吓人,本段与它同口径。
    pub fn keys_body(self) -> &'static [crate::layout::KeyPart<'static>] {
        use crate::layout::Target;
        match self {
            CatalogKind::Vps => &[
                ("  ↑/↓ 选", None, true),
                ("r 登记", None, true),
                (CATALOG_DELETE, Some(Target::CatalogDelete), true),
                ("Esc", None, true),
            ],
            CatalogKind::Tag => &[
                ("  ↑/↓ 选", None, true),
                ("Enter 反查", None, true),
                ("l 全部", None, true),
                ("r 挂标签", None, true),
                (CATALOG_DELETE_TAG, Some(Target::CatalogDelete), true),
            ],
            // #69 后续:订阅 / 钥匙册的 `g 取回命令` 收窄成 `g 取回`(#69 的
            // 鼠标过滤入口要在 66×50 下限内也放得下,两本册的键位行本来就最长;
            // 「取回」二字已够提示这个键,全句建议命令就在册体下一行摊着)。
            // #73:这一段现在挂 Target::CatalogGet——点它 = 按 `g`。
            CatalogKind::Subscription => &[
                ("  ↑/↓ 选订阅", None, true),
                ("r 登记", None, true),
                (CATALOG_GET, Some(Target::CatalogGet), true),
                (CATALOG_DELETE, Some(Target::CatalogDelete), true),
                ("l 重读", None, true),
            ],
            CatalogKind::Key => &[
                ("  ↑/↓ 选钥匙", None, true),
                ("r 登记", None, true),
                (CATALOG_GET, Some(Target::CatalogGet), true),
                (CATALOG_DELETE, Some(Target::CatalogDelete), true),
                ("l 重读", None, true),
            ],
        }
    }

    /// 这一本此刻该画哪个浮层;`None` = 画册体。
    ///
    /// **两种浮层同时在场时谁盖谁,也是查表数据不是调用方的 if**:机器册
    /// 让登记表单在上,其余三本让 rm 确认在上——因为标签册的 rm 确认
    /// 嵌套在表单流里(表单 Enter 才出现),`on_key` 的受理次序是更里层
    /// 的先收;订阅 / 钥匙册同形。这一条的次序与 `on_key` 逐字对齐,
    /// 否则会出现「键被 A 收、屏上画的是 B」。
    pub fn overlay(self, app: &App) -> Option<OverlayForm> {
        let order = match self {
            CatalogKind::Vps => [OverlayForm::Register, OverlayForm::RmConfirm],
            CatalogKind::Tag | CatalogKind::Subscription | CatalogKind::Key => {
                [OverlayForm::RmConfirm, OverlayForm::Register]
            }
        };
        order.into_iter().find(|f| self.overlay_on(*f, app))
    }

    /// 这个形态的浮层,这一本此刻在不在场(与形态无关,四本同一口径)。
    fn overlay_on(self, form: OverlayForm, app: &App) -> bool {
        match form {
            OverlayForm::Register => self.flow(app).phase != RegPhase::Idle,
            OverlayForm::RmConfirm => self.rm_confirm(app),
        }
    }

    /// 画这个形态的浮层(四本各一套画法,骨架同一件:同一个
    /// [`popup_rect`] 基准、同一套阶段,差别全在呈现)。
    pub fn draw_overlay(self, form: OverlayForm, frame: &mut Frame, area: Rect, app: &App) {
        match (self, form) {
            (CatalogKind::Vps, OverlayForm::Register) => render_register_overlay(frame, area, app),
            (CatalogKind::Vps, OverlayForm::RmConfirm) => render_rm_confirm(frame, area, app),
            (CatalogKind::Tag, OverlayForm::Register) => render_tag_overlay(frame, area, app),
            (CatalogKind::Tag, OverlayForm::RmConfirm) => render_tag_rm_confirm(frame, area, app),
            (CatalogKind::Subscription, OverlayForm::Register) => {
                render_sub_overlay(frame, area, app)
            }
            (CatalogKind::Subscription, OverlayForm::RmConfirm) => {
                render_sub_rm_confirm(frame, area, app)
            }
            (CatalogKind::Key, OverlayForm::Register) => render_key_overlay(frame, area, app),
            (CatalogKind::Key, OverlayForm::RmConfirm) => render_key_rm_confirm(frame, area, app),
        }
    }

    /// 册内一个键落在这一本上触发什么(#66:按册差异查表化)。
    ///
    /// **四本册走同一条键位路径**:`key_catalog` 只有一处查表 + 一处
    /// 落实,不再有四份逐字重复的 `match k.code`。按册差异全在这张表里
    /// ——同一个键在不同册上可以落到不同动作(`l` = 重读 / 列全部;
    /// `g` = 订阅/钥匙册的取回命令建议;`Enter` = 只有标签册反查选中的
    /// 这一条),也可以这一本根本没有这个键(机器册没有 `g`、没有
    /// `Enter` 反查),表里就不写那一行 = 这个键在这一本上无事可做。
    ///
    /// `r` 的写互斥也在这张表上:装配在途时机器册 / 订阅册 / 钥匙册的
    /// `r` 行不成立(与既有的 `if !self.assembling` 守卫同一口径),
    /// 落空即「这个键此刻无事可做」。第五本册子来时只加行,不改调用方。
    pub fn action(self, app: &App, k: KeyCode) -> Option<CatalogAction> {
        use CatalogAction::{Act, Move, Start};
        match (self, k) {
            // ↑/↓ 选行:四本同一口(不是按册差异,但同表驱动,四本走一条路)
            (_, KeyCode::Down) => Some(Move(true)),
            (_, KeyCode::Up) => Some(Move(false)),
            // r 登记 / 挂标签(#76:`start_*` 八份收成一份后,四本册同一个动作值)。
            // 真正的按册差异——两段式带确认闸 vs 无确认闸、各自的表单与提示——
            // 下沉到 `App::start` + `CatalogKind` 查表成员,表里不再分册。写互斥
            // 守卫不变:装配在途时 `r` 行不成立,落空即「这个键此刻无事可做」。
            (_, KeyCode::Char('r')) if !app.assembling => Some(Start(StartMode::Register)),
            // d 删除:四本册同一个动作值。机器 / 订阅 / 钥匙三本进 rm 确认弹层,
            // 标签册的删除嵌套在表单流里(摘引用 / 删整个标签两态)——这条按册
            // 分叉也下沉到 `App::start` → `CatalogKind::start_rm`,表里不分册。
            (_, KeyCode::Char('d')) => Some(Start(StartMode::Rm)),
            // l 重读册体 / 列全部标签:四本各一次只读调用,argv 同形
            (CatalogKind::Vps, KeyCode::Char('l')) => Some(Act(App::run_vps_ls)),
            (CatalogKind::Tag, KeyCode::Char('l')) => Some(Act(App::run_tag_ls_all)),
            (CatalogKind::Subscription, KeyCode::Char('l')) => Some(Act(App::run_sub_ls)),
            (CatalogKind::Key, KeyCode::Char('l')) => Some(Act(App::run_key_ls)),
            // g 取回本体的建议命令:只订阅册与钥匙册有(投影里有本体槽的
            // 两本;ADR-0009 §1 面板不代跑 get)。机器册 / 标签册没有这一行
            (CatalogKind::Subscription, KeyCode::Char('g')) => Some(Act(App::log_sub_get_suggest)),
            (CatalogKind::Key, KeyCode::Char('g')) => Some(Act(App::log_key_get_suggest)),
            // Enter 反查选中的这一条:标签册反查选中的那条标签(#54 旧态);
            // 机器册反查「什么标签引用了这一条」(#70 上浮——只机器册的条目可打
            // 标签,见 TAG_REF_KINDS;订阅 / 钥匙册的条目不在其内,不提供)。
            (CatalogKind::Vps, KeyCode::Enter) => Some(Act(App::vps_reverse_lookup)),
            (CatalogKind::Tag, KeyCode::Enter) => Some(Act(App::tag_lookup_selected)),
            _ => None,
        }
    }
}

/// 反查上浮**命中一条标签**(#70):标签名 + 它引用的 `<kind>/<条目>` 清单
/// (原样,指针不是密钥;结果呈现只印这些,绝不印机密本体)。
///
/// 只由纯函数 [`tags_referring`] 产出——匹配口径集中在这一处,不散进
/// 渲染 / 回音路由里,好被单测钉住。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TagHit {
    pub name: String,
    pub refs: Vec<crate::engine::CatalogRef>,
}

/// 反查上浮的**匹配纯函数**(#70:本轮最该被钉住的口径,spec #65 故事 41 /
/// 验收第 5 条)。一台机器被哪些标签引用?按 `<kind>/<条目>` **精确匹配**
/// (kind 与条目两侧都原样比对),**绝不做子串包含**——否则一台叫 `gate` 的
/// 机器会和一条引用 `skill/gate` 的标签互相串味。
///
/// 输入是**标签册的投影**(`nw tag ls` 无参全列落进 `tag_ls_echo` 后的
/// [`CatalogSheet`]:每条目是一个标签,`refs` 是它引用的 `<kind>/<条目>`
/// 清单)。`kind` 取自 [`crate::engine::commands::TAG_REF_KINDS`],机器册
/// 条目恒 `vps`。输出保持引擎的确定性排序(面板不重排)。
pub fn tags_referring(sheet: &CatalogSheet, kind: &str, id: &str) -> Vec<TagHit> {
    sheet
        .entries
        .iter()
        .filter(|e| e.refs.iter().any(|r| r.kind == kind && r.id == id))
        .map(|e| TagHit {
            name: e.name.clone(),
            refs: e.refs.clone(),
        })
        .collect()
}

/// 反查上浮状态(#70):机器册上一个键(Enter)或点键位段,问「什么标签引用了
/// 这一条」。**独立小状态,不并入 `assembling` 全局写互斥**(反查是只读,
/// 不占写闸);在途不拿上一轮结论冒充本次,迟到回音不收敛任何状态(操作者
/// 已返航 = Idle 时,回音落地不重开浮层)。
///
/// **三态两两可区分**(spec #65 故事 33 / 铁律 4:不共用一个空列表):
/// [`Hits`](VpsReverse::Hits) 有 N 条 / [`Empty`](VpsReverse::Empty) 一条都
/// 没有 / [`Unreadable`](VpsReverse::Unreadable) 读不到(rc≠0 / 契约守卫 /
/// 数据仓根 / 能力)。「读不到」不进结果浮层——它走既有指路(日志),只
/// [`Hits`](VpsReverse::Hits) / [`Empty`](VpsReverse::Empty) 画薄浮层。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum VpsReverse {
    /// 闲置:没有反查在场。
    #[default]
    Idle,
    /// 在途:已下发只读 `nw tag ls`(无参全列),等回音。此刻**不呈现任何
    /// 结论**(`tag_ls_echo` 已被清空),屏上只有机器册本体。
    InFlight { machine: String },
    /// 有 N 条标签命中(列标签名 + 各自引用)。
    Hits { machine: String, tags: Vec<TagHit> },
    /// 一条标签引用都没有(明说 + 给「去挂第一个标签」的下一步)。
    Empty { machine: String },
    /// 读不到(rc≠0 / 契约守卫合成「需升级 nw」):带一句人话理由,走既有
    /// 指路(日志),**不画成空列表**。
    Unreadable { machine: String, reason: String },
}

impl VpsReverse {
    /// 结果浮层在不在场(只有 有 N 条 / 一条都没有 两态画浮层;在途 / 闲置 /
    /// 读不到 都不画)。渲染与命中区共用这一判据——点哪打哪(ADR-0005)。
    pub fn is_open(&self) -> bool {
        matches!(self, VpsReverse::Hits { .. } | VpsReverse::Empty { .. })
    }
}

/// 反查跳转浮层里一条**命中条目**的去处(#120):按 kind 钉死在现有屏结构上。
///
/// - `vps` → 编目册屏的**机器册**(条目就是一台机器);
/// - `skill` / `extension` → **配方屏的外部源域**(技能与扩展住在源仓里,
///   `nw ls` 的 sources 域是面板唯一看得到它们的地方——编目册屏四本册都
///   不是它们的家,硬安一个去处等于造第六本册)。
///
/// `None` = 这个 kind 在现有屏结构里没有去处。**不是死键**:条目照画在
/// 浮层里,Enter 给人话(见 [`App::tag_jump_to`](crate::app::App::tag_jump_to))。
/// 已知 kind 集合是 [`crate::engine::commands::TAG_REF_KINDS`];本表与它的
/// 对齐由测试钉住(`TAG_REF_KINDS_每个kind都有去处`)——上游加 kind 而这里
/// 没跟上,当场红,不许静默少一个去处。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JumpDest {
    /// 编目册屏的机器册。
    Machine,
    /// 配方屏的外部源域。
    SourceEntry,
}

impl JumpDest {
    /// 去处的中文名(浮层里画在命中条目右侧,操作者先看见去处再跳)。
    pub fn label(self) -> &'static str {
        match self {
            JumpDest::Machine => "机器册",
            JumpDest::SourceEntry => "外部源域",
        }
    }
}

/// 一条 `<kind>/<条目>` 引用的去处。查表同 [`CatalogKind::action`] 的法子:
/// 按 kind 的差异是**数据**,不是调用方的一串分支。
pub fn jump_dest(kind: &str) -> Option<JumpDest> {
    match kind {
        "vps" => Some(JumpDest::Machine),
        "skill" | "extension" => Some(JumpDest::SourceEntry),
        _ => None,
    }
}

/// skill / extension 命中条目在**外部源域**里的下标(#120)。
///
/// 引用的 id 形状是引擎自己的口径(`nw tag add` 第③道闸的 hint:
/// `skill/<仓库>/<条目>`),所以 id 按 `/` 拆:**最后一段是条目名**,前面的
/// 是仓库提示。匹配两步走,都不猜:
/// ① 条目名对上——源的 `entries[]` 里有一条同名条目;
/// ② 仓库提示也对得上——优先取 `name` 或 `repo` 与提示相符的那一条
///    (同一条目名可能出现在多个源里;都相符时取引擎给的确定性顺序里的
///    第一个,面板不重排)。
/// 两条都落空给 `None` = 这个条目在当前 `ls` 回音里没有去处,调用方明示。
pub fn source_hit_index(ls: &crate::engine::LsData, id: &str) -> Option<usize> {
    let (entry, repo_hint) = split_ref_id(id)?;
    let mut first: Option<usize> = None;
    for (i, s) in ls.sources.iter().enumerate() {
        if !s.entries.iter().any(|e| e.name == entry) {
            continue;
        }
        if repo_hint.is_some_and(|h| source_namesake(s, h)) {
            return Some(i);
        }
        if first.is_none() {
            first = Some(i);
        }
    }
    first
}

/// `o/r/deep-research` → `("deep-research", Some("o/r"))`。
/// 只有一段(没有仓库提示)也认:条目名就是它自己,仓库提示 None——匹配退
/// 化成「只看条目名」,不因此把这条引用判成无去处。
fn split_ref_id(id: &str) -> Option<(&str, Option<&str>)> {
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    match id.rsplit_once('/') {
        Some((head, entry)) if !entry.is_empty() && !head.is_empty() => Some((entry, Some(head))),
        _ => Some((id, None)),
    }
}

/// 源的哪一个字段算「仓库提示相符」:`name` 直接相等,或 `repo` 链接以提示
/// 收尾(`https://github.com/o/r` ↔ `o/r`)。两者都不判空——空串不配对。
fn source_namesake(s: &crate::engine::SourceRow, hint: &str) -> bool {
    s.name == hint
        || s.repo
            .as_deref()
            .is_some_and(|r| r.trim_end_matches('/').ends_with(hint))
}

/// 标签册反查浮层的**命中条目跳转**(#120):问「这一条标签引用了什么」之后,
/// 浮层把命中条目列出来,↑/↓ 选、Enter 直跳它所在的册 / 视图。
///
/// 与 [`VpsReverse`](机器册反查上浮)是两件事,别混:那边问「什么标签引用
/// 了这一条」,命中是**标签**、没有 kind 维度;这边问「这一条标签引用了
/// 什么」,命中是**条目**、每条带 kind,所以才谈得上「跳到哪个册」。
///
/// **独立小状态,不并入 `assembling` 写互斥**:跳转只是改面板自己的状态树,
/// 零引擎调用、零写路径。开浮层的条件只有一条——标签册 + 正在反查某一条
/// (`App::tag_lookup` = Some)+ 回音可定型;列全部(`l`)与机器册反查上浮都走
/// `run_tag_ls(None)`,`tag_lookup` 是 None,浮层不开。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TagJump {
    /// 闲置:没有反查跳转浮层在场。
    #[default]
    Idle,
    /// 在场:这一条标签的命中条目清单 + 浮层内选中行。
    Hits {
        tag: String,
        hits: Vec<crate::engine::CatalogRef>,
        sel: usize,
    },
}

impl TagJump {
    /// 浮层在不在场(渲染 / 键位吞键 / 命中区共用这一判据)。
    pub fn is_open(&self) -> bool {
        matches!(self, TagJump::Hits { .. })
    }

    /// 问的是哪一条标签(浮层标题用)。
    pub fn tag(&self) -> Option<&str> {
        match self {
            TagJump::Hits { tag, .. } => Some(tag.as_str()),
            TagJump::Idle => None,
        }
    }

    /// 命中条目清单(空 = 没开场)。
    pub fn hits(&self) -> &[crate::engine::CatalogRef] {
        match self {
            TagJump::Hits { hits, .. } => hits.as_slice(),
            TagJump::Idle => &[],
        }
    }

    /// 浮层内选中行(越界钳在最后一条;没开场给 0)。
    pub fn sel(&self) -> usize {
        match self {
            TagJump::Hits { sel, hits, .. } => (*sel).min(hits.len().saturating_sub(1)),
            TagJump::Idle => 0,
        }
    }

    /// 选中那一条命中(越界 / 未开场给 None)。
    pub fn hit(&self, sel: usize) -> Option<&crate::engine::CatalogRef> {
        match self {
            TagJump::Hits { hits, .. } => hits.get(sel),
            TagJump::Idle => None,
        }
    }

    /// ↑/↓ 移选中:上下各一档,钳在 `[0, len)`(空清单不动)。
    pub fn move_sel(&mut self, down: bool) {
        if let TagJump::Hits { sel, hits, .. } = self {
            if hits.is_empty() {
                return;
            }
            *sel = if down {
                (*sel + 1).min(hits.len() - 1)
            } else {
                (*sel).saturating_sub(1)
            };
        }
    }

    /// 鼠标点第 `i` 条:先把选中落到那一行(越界钳住),再由调用方按
    /// [`Self::sel`] 直跳——点哪跳哪,与 `↑/↓` + `Enter` 同一个后果。
    pub fn move_sel_to(&mut self, i: usize) {
        if let TagJump::Hits { sel, hits, .. } = self {
            if !hits.is_empty() {
                *sel = i.min(hits.len() - 1);
            }
        }
    }
}

/// 册内浮层的两种形态(登记流 / rm 确认;弹层优先级高于册体)。
/// 渲染与命中区都只认这两个形态,不认「哪一本」。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverlayForm {
    /// 两段式登记流(表单 / 预审在途 / 摊开闸结论 / 落账在途 / 结论)。
    Register,
    /// 删除前的图形确认(ADR-0009 §2:面板加一道,不改引擎语义)。
    RmConfirm,
}

/// 册内登记 / 删除入口的模式(#76:四本册的 `start_*` 八份收成一份后,由
/// [`CatalogAction::Start`] 带着)。只留「登记 / 挂标签」与「删除」两种模式,
/// 按册差异(入口门、表单形态、删除是进确认弹层还是嵌套进表单流)全部下沉到
/// [`CatalogKind`] 查表成员 + [`App::start`](crate::app::App::start),这里不分册。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StartMode {
    /// 登记 / 挂标签:进登记流表单(vps / 订阅 / 钥匙两段式带确认闸,标签册无确认闸)。
    Register,
    /// 删除:机器 / 订阅 / 钥匙三本进 rm 确认弹层;标签册的删除嵌套在表单流里。
    Rm,
}

/// 表单 Enter 之后,这一本的写命令走哪条路(#76:按册差异是**控制流**,所以它
/// 作为查表数据由 [`CatalogKind::write_route`] 给出,调用方那一份分发因此
/// 不分册)。
pub enum WriteRoute {
    /// 两段式第一段(预审,不带 `--yes`,文件不落):机器 / 订阅 / 钥匙三本。
    Preaudit,
    /// 直接进写段:标签册的 `tag add`(上游没有 `--yes`,ADR-0009 §2 面板不
    /// 自己造一道确认)。
    Commit,
    /// 面板侧的图形确认:标签册的 `tag rm`(删除嵌套在表单流里,表单 Enter 只
    /// 起确认、不下发)。
    RmConfirm,
}

/// 册内键位触发的动作(#66 查表表的取值)。`Act` 直接带这一本的落实方法
/// ——按册差异就是「哪个方法」,这是数据,不是调用方的一个分支。
/// [`Start`](CatalogAction::Start) 是 #76 收敛后的登记 / 删除入口:它**不带**
/// 方法指针(那会把按册分叉重新搬回表里),只带模式,真正的按册差异走
/// [`App::start`](crate::app::App::start) + [`CatalogKind`] 查表成员。
pub enum CatalogAction {
    /// ↑/↓ 移选中(`true` = 向下;到界绕回,见 `App::catalog_move`)。
    Move(bool),
    /// 这一本对这一键的落实(登记 / 删除 / 重读 / 取回命令 / 反查)。
    Act(fn(&mut App)),
    /// 登记 / 删除入口(#76:`r` / `d` 两键,四本册同一个动作值;按册差异在
    /// [`App::start`](crate::app::App::start) 里查表落地)。
    Start(StartMode),
}

/// 一本册子的条目明细画法:名字行右侧的弱化后缀(机器册给端口,标签册
/// 没有)+ 名字行之下的明细行(机器册给 地址/登录/备注,标签册给 引用)。
///
/// 列表件 [`catalog_sheet_lines`] 只按这一组画;选中标记 / 名字 / 折行
/// 截断 / 确定性排序都在它自己那一处。后三本册子只换这一个函数指针,
/// **不换列表件**(工单 #54 验收第 5 条:不复制第二份列表逻辑)。
pub type EntryDetail =
    fn(&CatalogEntryView, usize, &Palette) -> (Option<String>, Vec<Line<'static>>);

/// [`EntryDetail`] 的返回形状(名字行后缀 + 明细行)。
type EntryDetailOut = (Option<String>, Vec<Line<'static>>);

// ---- vps 命令组的登记规格(ADR-0009 §1 获准两段式代跑) ----

/// vps 登记的命令组规格。`--root` 指数据仓根;**不喂 `--repo`**
/// (那是 `source register` 的外部源链接槽,喂给 vps 组 = 未知 flag → rc 3)。
pub struct VpsSpec {
    pub name: String,
    /// `(label, addr)` 逐对(可重复 `--ip`,顺序即入参数组顺序)。
    pub ips: Vec<(String, String)>,
    pub port: Option<u16>,
    /// `(user, ref)` 逐对(可重复 `--login`;ref 是指针不是密钥)。
    pub logins: Vec<(String, String)>,
    pub note: Option<String>,
}

impl VpsSpec {
    /// 表单 → 规格(面板侧解析 `label=addr` / `user=ref`,不喂给引擎猜)。
    /// 格式非法的行被剔除并回报(调用方留证据、留驻可改)。
    pub fn from_form(form: &VpsForm) -> (Self, Vec<String>) {
        let mut bad = Vec::new();
        let mut ips = Vec::new();
        for row in &form.ips {
            match split_pair(row) {
                Some(p) => ips.push(p),
                None => bad.push(format!("地址「{row}」不是 label=addr")),
            }
        }
        let mut logins = Vec::new();
        for row in &form.logins {
            match split_pair(row) {
                Some(p) => logins.push(p),
                None => bad.push(format!("登录「{row}」不是 user=ref")),
            }
        }
        let port = form.port.trim().parse::<u16>().ok();
        (
            Self {
                name: form.name.trim().to_string(),
                ips,
                port,
                logins,
                note: form
                    .note
                    .trim()
                    .is_empty()
                    .then_some(form.note.trim().to_string()),
            },
            bad,
        )
    }
}

impl RegisterSpec for VpsSpec {
    fn title() -> &'static str {
        "登记机器 · vps"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn preaudit(&self, root: &str) -> Vec<String> {
        crate::engine::commands::vps_register_preaudit(
            &self.name,
            &self.ips,
            self.port,
            &self.logins,
            self.note.as_deref(),
            root,
        )
    }

    fn commit(&self, root: &str) -> Vec<String> {
        crate::engine::commands::vps_register_commit(
            &self.name,
            &self.ips,
            self.port,
            &self.logins,
            self.note.as_deref(),
            root,
        )
    }

    fn audit(out: &EngineOutput) -> Option<Audit> {
        out.envelope
            .as_ref()
            .and_then(|e| e.data_as::<VpsRegisterData>())
            .map(|d| Audit {
                entry: d.sheet().entries.into_iter().next(),
                catalog_path: d.catalog_path.clone(),
                gates: d.gates.clone(),
            })
    }
}

/// `label=addr` 拆一对:两侧都非空才认(空一侧 = 面板侧就拒,不喂给引擎)。
fn split_pair(row: &str) -> Option<(String, String)> {
    let (a, b) = row.trim().split_once('=')?;
    let (a, b) = (a.trim(), b.trim());
    (!a.is_empty() && !b.is_empty()).then(|| (a.to_string(), b.to_string()))
}

// ---- tag 命令组的登记规格(工单 #54,ADR-0009 §1:add 代跑、**无确认闸**) ----

/// 标签册的登记规格:一个标签名 + 若干条 `<kind>/<条目>` 引用。
///
/// # 为什么 tag 的 `preaudit` 与 `commit` 是同一份 argv
///
/// 上游 `tag add` **没有 `--yes`**:追加合并、幂等,没有「覆盖」可确认,
/// 所以也没有「拟落账不落盘」的那一段——预审段一跑就是在写。这里
/// **不伪造一枚 `--yes`**(多一枚未知 flag 引擎直接 rc 3,那是改引擎
/// 语义):两个方法都返回 [`crate::engine::commands::tag_add`] 的同一份
/// argv,差别只在面板**什么时候**把它递出去(见 [`RegisterSpec::confirm_gate`]
/// 返回 `false`:面板不自己造一道图形确认,表单 Enter 即下发)。
pub struct TagSpec {
    /// 标签名(归一后小写是引擎的事,面板只 Trim、不改写)。
    pub name: String,
    /// `(kind, id)` 逐对(可重复 `--ref`,顺序即入参数组顺序)。
    pub refs: Vec<(String, String)>,
}

impl TagSpec {
    /// 表单 → 规格(面板侧解析 `kind/条目`,不喂给引擎猜)。格式非法的行
    /// 被剔除并回报(调用方留证据、留驻可改)。
    pub fn from_form(form: &TagForm) -> (Self, Vec<String>) {
        let mut bad = Vec::new();
        let mut refs = Vec::new();
        for row in &form.refs {
            if row.trim().is_empty() {
                continue; // 空行 = 没填这一条,不是坏行
            }
            match crate::engine::commands::parse_tag_ref(row) {
                Some(p) => refs.push(p),
                None => bad.push(format!(
                    "引用「{row}」不是 <kind>/<条目>(kind 已知集合:skill / extension / vps)"
                )),
            }
        }
        (
            Self {
                name: form.name.trim().to_string(),
                refs,
            },
            bad,
        )
    }
}

impl RegisterSpec for TagSpec {
    fn title() -> &'static str {
        "挂标签 · tag"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn preaudit(&self, root: &str) -> Vec<String> {
        // 同一份 argv(见类型文档):tag 没有「文件不落」的那一段
        crate::engine::commands::tag_add(&self.name, &self.refs, root)
    }

    fn commit(&self, root: &str) -> Vec<String> {
        crate::engine::commands::tag_add(&self.name, &self.refs, root)
    }

    fn audit(out: &EngineOutput) -> Option<Audit> {
        let env = out.envelope.as_ref()?;
        // 形状不认不猜:`TagAddData` 的字段全带 default,任何一个**已认识**
        // 的 schema 都能被解析成「空标签 + 零道闸」——那正是「看起来正常
        // 其实什么都没解析」。所以除了契约守卫的 `data_as`,再钉一道
        // schema 相符(回音不是 tag add 的 = 不可定型,不落账、不画结论)。
        if env.schema != "nw/tag-add.v1" {
            return None;
        }
        env.data_as::<TagAddData>().map(|d| Audit {
            entry: Some(CatalogEntryView {
                name: d.name.clone(),
                refs: d.refs.clone(),
                ..Default::default()
            }),
            catalog_path: d.catalog_path.clone(),
            gates: d.gates.clone(),
        })
    }

    /// tag add 没有引擎确认闸 → 面板也不自己造一道(ADR-0009 §2)。
    fn confirm_gate() -> bool {
        false
    }
}

// ---- subscription 命令组的登记规格(工单 #56,ADR-0009 §1:两段式 + 本体经 stdin) ----

/// 在途的**链接本体**(ADR-0009 §3):「在途内存 + 一个不打印内容的外包类型」。
///
/// 两段式要求预审与落账**各喂一次**(喂入是一次性的、随进程结束即弃,不能复用
/// 同一次管道),所以本体在面板侧必须在登记流程的在途状态里停留一会儿。裁断
/// (ADR-0009 §3):它只以这一个类型存在——「面板侧不留副本」由此拆成两句,
/// 各自可判定:
/// - **不留引用**:[`std::fmt::Debug`] **不打印内容**(只报字节数),所以它进不了
///   日志行,也进不了崩溃上报;渲染只经 [`SecretBody::mask`] 拿「已喂入 N 字节」
///   这一句,字符一个都不出现;面板不把它写进 config / 任何持久化路径(也没有这样
///   的 API);`fed_queue` 只是交给 runner 的在途中转,每个 tick 被 `std::mem::take`
///   抽空,不是缓存。
/// - **流程结束即擦字节**:见 [`impl Drop`]——`Vec<u8>` 的默认 drop 只释放堆、
///   不擦字节,core dump 或内存复用都能还原明文,所以 `Drop` 里 `fill(0)`。
///
/// 它**只从 stdin 进**(`App` 经 `CommandRunner::spawn_with_stdin` 喂给子进程,
/// 写后即关、即弃):argv 面没有任何明文形参,误粘的 argv 会进 ps / history。
/// [`impl Drop`]: SecretBody#impl-Drop-for-SecretBody
#[derive(Clone, Default, PartialEq, Eq)]
pub struct SecretBody(Vec<u8>);

impl SecretBody {
    /// 已喂入的字节数(面板侧唯一被允许说出来的关于本体的事实)。
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 喂入框的**掩码**文案:只报字节数,不报内容。
    pub fn mask(&self) -> String {
        if self.is_empty() {
            "未喂入(链接本体只经 stdin 进,界面不显示字符)".to_string()
        } else {
            format!(
                "已喂入 {} 字节(掩码:本体字符不进渲染/日志/配置)",
                self.len()
            )
        }
    }

    /// 交出字节(喂给子进程 stdin 的那一刻;调用方用完即弃)。
    ///
    /// 注意这之后明文就出了本类型的管辖:接手的 `Vec<u8>` 必须在喂完
    /// (`write_all`) 之后自己 `fill(0)`——`CommandRunner::spawn_with_stdin`
    /// 那条路已经这么做,ADR-0009 §3 的「流程结束即擦字节」才不说谎。
    pub fn into_bytes(mut self) -> Vec<u8> {
        std::mem::take(&mut self.0)
    }
}

impl Drop for SecretBody {
    /// **流程结束即擦字节**(ADR-0009 §3):`Vec<u8>` 的默认 drop 只把堆还给
    /// 分配器、不擦内容——进程被 core dump、那些页被后续分配复用,明文都还
    /// 还原得出来。这里补上默认 drop 缺的那一步。
    ///
    /// 不加 `zeroize` crate:一行 `fill(0)` 就够,且不引 `unsafe`。
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

impl std::fmt::Debug for SecretBody {
    /// **刻意不打印内容**:只报「在途 N 字节」。这样 `{:?}` 进日志 / 崩溃上报
    /// 也不会漏本体(ADR-0009 §3 的「不打印内容的外包类型」)。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretBody(<在途 {} 字节,不打印内容>)", self.0.len())
    }
}

/// 到期日的**形状**校验(面板侧就拒、不喂给引擎猜):严格 `YYYY-MM-DD`
/// (10 字符、第 5/第 8 字符是 `-`、其余全是 ASCII 数字)。空 = 永不过期
/// (上游 `expires` 空串的口径,合法)。
///
/// 面板只校**形状**:「02-30 是不是真日期」由引擎的第③道闸按 `time.Parse`
/// 往返判(上游 `status.go` `validExpires`),面板不复刻那道判定,也不自定一套
/// 更宽/更严的口径。
pub fn valid_expires(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return true;
    }
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && [0, 1, 2, 3, 5, 6, 8, 9]
            .iter()
            .all(|&i| b[i].is_ascii_digit())
}

/// 订阅登记的命令组规格:名字 / client / 到期日。**没有链接本体槽**——
/// 本体只经 stdin 进(见 [`SecretBody`]),规格里放不下它,所以也从规格上
/// 杜绝「误把链接拼进 argv」。
///
/// 两段式与 vps 同构(预审无 `--yes`、落账多一枚 `--yes`),差别只在 argv
/// 形状与「喂一次 stdin」:`App` 在下发这两段时各喂一次本体。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubscriptionSpec {
    pub name: String,
    /// 消费这条链接的应用(如 Mihomo;上游口径:它是**字段不是名字**)。
    pub client: String,
    /// 到期日;`None` / 空 = 永不过期(不下发 `--expires`)。
    pub expires: Option<String>,
}

impl RegisterSpec for SubscriptionSpec {
    fn title() -> &'static str {
        "登记订阅 · subscription"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn preaudit(&self, root: &str) -> Vec<String> {
        crate::engine::commands::subscription_register_preaudit(
            &self.name,
            &self.client,
            self.expires.as_deref(),
            root,
        )
    }

    fn commit(&self, root: &str) -> Vec<String> {
        crate::engine::commands::subscription_register_commit(
            &self.name,
            &self.client,
            self.expires.as_deref(),
            root,
        )
    }

    fn audit(out: &EngineOutput) -> Option<Audit> {
        let env = out.envelope.as_ref()?;
        // 同 tag:形状不认不猜。`SubRegisterData` 字段全带 default,任何一个
        // 已认识的 schema 都能被解析成「空订阅 + 零道闸」——那正是「看起来
        // 正常其实什么都没解析」。所以再钉一道 schema 相符。
        if env.schema != "nw/subscription-register.v1" {
            return None;
        }
        env.data_as::<SubRegisterData>().map(|d| Audit {
            entry: d.sheet().entries.into_iter().next(),
            catalog_path: d.catalog_path.clone(),
            gates: d.gates.clone(),
        })
    }
}

// ---- key 命令组的登记规格(工单 #57,ADR-0009 §1:两段式 + 本体经 stdin) ----

/// 钥匙登记的命令组规格:名字 / 服务哪个主机 / 若干条作用域 / 轮换日。
/// **没有钥匙本体槽**——本体只经 stdin 进(见 [`SecretBody`]),规格里放不下它,
/// 所以也从规格上杜绝「误把钥匙拼进 argv」。
///
/// 两段式与 vps / subscription 同构(预审无 `--yes`、落账多一枚 `--yes`),差别
/// 只在 argv 形状与「喂一次 stdin」:`App` 在下发这两段时各喂一次本体。
///
/// `scopes` / `rotated_at` **只登记、不设闸**(ADR-0009 §3:多宽、何时轮换归
/// 所有者自裁,面板不代管、不提醒、不评审);形状错(空 scope / 非 YYYY-MM-DD)
/// 由面板侧就拒,不喂给引擎猜。`host` 空 = 缺省 github.com(不下发 `--host`)。
/// `rotated_at` `None` = 缺省登记日 UTC(不下发 `--rotated-at`)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySpec {
    pub name: String,
    /// 服务哪个主机;空 = 缺省 github.com(上游 CLI flag 缺省)。
    pub host: String,
    /// 若干条作用域(只登记不设闸;顺序即入参数组顺序)。
    pub scopes: Vec<String>,
    /// 轮换日(严格 YYYY-MM-DD);`None` / 空 = 缺省登记日 UTC(不下发 `--rotated-at`)。
    pub rotated_at: Option<String>,
}

impl RegisterSpec for KeySpec {
    fn title() -> &'static str {
        "登记钥匙 · key"
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn preaudit(&self, root: &str) -> Vec<String> {
        crate::engine::commands::key_register_preaudit(
            &self.name,
            &self.host,
            &self.scopes,
            self.rotated_at.as_deref(),
            root,
        )
    }

    fn commit(&self, root: &str) -> Vec<String> {
        crate::engine::commands::key_register_commit(
            &self.name,
            &self.host,
            &self.scopes,
            self.rotated_at.as_deref(),
            root,
        )
    }

    fn audit(out: &EngineOutput) -> Option<Audit> {
        let env = out.envelope.as_ref()?;
        // 同 tag / subscription:形状不认不猜。`KeyRegisterData` 字段全带 default,
        // 任何一个已认识的 schema 都能被解析成「空钥匙 + 零道闸」——那正是「看起来
        // 正常其实什么都没解析」。所以再钉一道 schema 相符。
        if env.schema != "nw/key-register.v1" {
            return None;
        }
        env.data_as::<KeyRegisterData>().map(|d| Audit {
            entry: d.sheet().entries.into_iter().next(),
            catalog_path: d.catalog_path.clone(),
            gates: d.gates.clone(),
        })
    }
}

/// 订阅册表单的字段(顺序即 ↑/↓ 轮转顺序)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SubField {
    #[default]
    Name,
    Client,
    Expires,
    /// 链接本体喂入框(掩码;只经 stdin 进)。
    Body,
}

impl SubField {
    pub const ALL: [SubField; 4] = [
        SubField::Name,
        SubField::Client,
        SubField::Expires,
        SubField::Body,
    ];

    /// 字段标签(呈现与键位提示同源)。
    pub fn label(self) -> &'static str {
        match self {
            SubField::Name => "名字",
            SubField::Client => "应用",
            SubField::Expires => "到期",
            SubField::Body => "链接",
        }
    }

    /// 喂入框(掩码字段)——键入走 [`on_key_sub_form`] 的掩码分支,
    /// 不进任何字符串缓冲。
    pub fn is_body(self) -> bool {
        matches!(self, SubField::Body)
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// 订阅登记表单:名字 / client / 到期日 + **链接本体喂入框**(掩码)。
/// 与 [`VpsForm`] / [`TagForm`] 同构(单行字段,可测),但多一个掩码字段:
/// 它的缓冲是 [`SecretBody`](不打印内容的外包类型),不是 `String`。
#[derive(Clone, Default)]
pub struct SubForm {
    pub name: String,
    pub client: String,
    /// 到期日(严格 YYYY-MM-DD;空 = 永不过期)。
    pub expires: String,
    /// 链接本体(在途内存;掩码渲染、Debug 不打印内容、随流程 drop)。
    pub body: SecretBody,
    pub field: SubField,
}

impl SubForm {
    /// 当前单行字段的缓冲(喂入框借 [`SecretBody`],不走这里)。
    fn buf(&mut self) -> &mut String {
        match self.field {
            SubField::Name => &mut self.name,
            SubField::Client => &mut self.client,
            SubField::Expires => &mut self.expires,
            // 喂入框的键入在 on_key_sub_form 里就被接走,走不到这里;
            // 给 name 只是为让 match 完备(不新增第二条真路径)。
            SubField::Body => &mut self.name,
        }
    }

    /// ↑/↓ 移字段(四行轮转,到顶/到底不越界、不绕回)。
    fn move_field(&mut self, down: bool) {
        let i = self.field.index();
        let n = SubField::ALL.len();
        self.field = SubField::ALL[if down {
            (i + 1).min(n - 1)
        } else {
            i.saturating_sub(1)
        }];
    }
}

/// 订阅册表单段键位:↑/↓ 移字段、Enter 预审、Esc/q 返航。喂入框的键入走
/// 掩码分支(字节数可用于日志,字符不进任何字符串缓冲)。
pub fn on_key_sub_form(form: &mut SubForm, k: crossterm::event::KeyEvent) -> FormAction {
    match k.code {
        KeyCode::Up => {
            form.move_field(false);
            return FormAction::Edited;
        }
        KeyCode::Down => {
            form.move_field(true);
            return FormAction::Edited;
        }
        _ => {}
    }
    if form.field.is_body() {
        return on_key_secret(&mut form.body, k);
    }
    match input::on_key(form.buf(), k) {
        InputAction::Submit => FormAction::Submit,
        InputAction::Cancel => FormAction::Cancel,
        InputAction::Edited | InputAction::Ignored => FormAction::Edited,
    }
}

/// 喂入框的键入分发:语义与 [`input::on_key`] 逐条对齐(Enter 提交、Esc/q
/// 取消、退格删一个字符、只收可打印字符、缓冲封顶),差别只在**落在
/// [`SecretBody`] 上**——字节数可报,内容不进任何字符串。
fn on_key_secret(body: &mut SecretBody, k: crossterm::event::KeyEvent) -> FormAction {
    match k.code {
        KeyCode::Enter => FormAction::Submit,
        KeyCode::Esc | KeyCode::Char('q') => FormAction::Cancel,
        KeyCode::Backspace => {
            body.0.pop();
            FormAction::Edited
        }
        KeyCode::Char(c) => {
            let ctrl = k
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL);
            if !ctrl && !c.is_control() && body.0.len() < input::BUF_CAP {
                let mut buf = [0u8; 4];
                body.0.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            FormAction::Edited
        }
        _ => FormAction::Edited,
    }
}

/// 订阅登记的发起规格由表单算出(供 `App` 下发两段用)。面板侧就拒(空名字 /
/// 空 client / 到期日形状不对 / 本体没喂入),留表单可改,不喂给引擎跑一趟。
pub fn spec_from_sub_form(form: &SubForm) -> Result<SubscriptionSpec, Vec<String>> {
    let name = form.name.trim().to_string();
    let client = form.client.trim().to_string();
    let expires = form.expires.trim().to_string();
    let mut bad = Vec::new();
    if name.is_empty() {
        bad.push("名字为空".into());
    }
    if client.is_empty() {
        bad.push("应用(client)为空:它是字段不是名字,如 Mihomo".into());
    }
    if !valid_expires(&expires) {
        bad.push(format!(
            "到期日「{expires}」不是严格 YYYY-MM-DD(可留空 = 永不过期)"
        ));
    }
    if form.body.is_empty() {
        bad.push("链接本体还没喂入(只经 stdin 进:在链接框贴一次)".into());
    }
    if !bad.is_empty() {
        return Err(bad);
    }
    Ok(SubscriptionSpec {
        name,
        client,
        expires: (!expires.is_empty()).then_some(expires),
    })
}

/// 新建一张空订阅表单(供 `App` 发起登记用)。
pub fn new_sub_form() -> SubForm {
    SubForm::default()
}

// ---- 钥匙册表单(工单 #57:名字 / 主机 / 作用域(多行)/ 轮换日 + 本体喂入框) ----

/// 钥匙册表单的字段(顺序即 ↑/↓ 轮转顺序)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum KeyField {
    #[default]
    Name,
    Host,
    Scopes,
    RotatedAt,
    /// 钥匙本体喂入框(掩码;只经 stdin 进)。
    Body,
}

impl KeyField {
    pub const ALL: [KeyField; 5] = [
        KeyField::Name,
        KeyField::Host,
        KeyField::Scopes,
        KeyField::RotatedAt,
        KeyField::Body,
    ];

    /// 字段标签(呈现与键位提示同源)。
    pub fn label(self) -> &'static str {
        match self {
            KeyField::Name => "名字",
            KeyField::Host => "主机",
            KeyField::Scopes => "作用域",
            KeyField::RotatedAt => "轮换日",
            KeyField::Body => "钥匙",
        }
    }

    /// 多行字段(一行一条)——单行字段给 false。
    pub fn multi(self) -> bool {
        matches!(self, KeyField::Scopes)
    }

    /// 喂入框(掩码字段)——键入走 [`on_key_key_form`] 的掩码分支,不进任何字符串缓冲。
    pub fn is_body(self) -> bool {
        matches!(self, KeyField::Body)
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// 钥匙登记表单:名字 / 主机 / 若干条作用域(一行一条)/ 轮换日 + **钥匙本体
/// 喂入框**(掩码)。与 [`VpsForm`](多行作用域)和 [`SubForm`](掩码本体)同构,
/// 是两者的合并:单行字段 + 一个多行字段 + 一个掩码字段。**不复制第二份表单
/// 机制**——字段各归其槽,只借 [`input::on_key`] 与 [`on_key_secret`] 这两条
/// 既有的键入分发。
#[derive(Clone, Default)]
pub struct KeyForm {
    pub name: String,
    /// 服务哪个主机;空 = 缺省 github.com(不下发 `--host`)。
    pub host: String,
    /// 若干条作用域(一行一条;空行 = 没填这条,不是坏行)。
    pub scopes: Vec<String>,
    /// 轮换日(严格 YYYY-MM-DD;空 = 缺省登记日 UTC)。
    pub rotated_at: String,
    /// 钥匙本体(在途内存;掩码渲染、Debug 不打印内容、随流程 drop)。
    pub body: SecretBody,
    pub field: KeyField,
    /// 多行字段里的当前行(单行字段恒 0)。
    pub row: usize,
}

impl KeyForm {
    fn new() -> Self {
        Self {
            scopes: vec![String::new()],
            ..Self::default()
        }
    }

    /// 当前单行/多行字段的缓冲(喂入框借 [`SecretBody`],不走这里)。
    fn buf(&mut self) -> &mut String {
        match self.field {
            KeyField::Name => &mut self.name,
            KeyField::Host => &mut self.host,
            KeyField::RotatedAt => &mut self.rotated_at,
            KeyField::Scopes => {
                while self.row >= self.scopes.len() {
                    self.scopes.push(String::new());
                }
                &mut self.scopes[self.row]
            }
            // 喂入框的键入在 on_key_key_form 里就被接走,走不到这里;
            // 给 name 只是为让 match 完备(不新增第二条真路径)。
            KeyField::Body => &mut self.name,
        }
    }

    fn rows(&self) -> usize {
        if self.field.multi() {
            self.scopes.len()
        } else {
            1
        }
    }

    /// ↑/↓ 移字段(五行轮转,到顶/到底不越界、不绕回)。
    fn move_field(&mut self, down: bool) {
        let i = self.field.index();
        let n = KeyField::ALL.len();
        self.field = KeyField::ALL[if down {
            (i + 1).min(n - 1)
        } else {
            i.saturating_sub(1)
        }];
        self.row = 0;
    }

    /// 作用域字段新行(追加一条空行,行光标跟上)。
    fn add_row(&mut self) {
        if self.field.multi() {
            self.scopes.push(String::new());
            self.row = self.scopes.len() - 1;
        }
    }

    /// 删当前行(至少留一行——表单不塌成空)。
    fn drop_row(&mut self) {
        if self.field.multi() && self.scopes.len() > 1 {
            self.scopes.remove(self.row);
            self.row = self.row.min(self.scopes.len() - 1);
        }
    }
}

/// 钥匙册表单段键位:↑/↓ 移字段、n 新行、d 删行、Enter 预审、Esc/q 返航。
/// 喂入框的键入走掩码分支(字节数可用于日志,字符不进任何字符串)。
pub fn on_key_key_form(form: &mut KeyForm, k: crossterm::event::KeyEvent) -> FormAction {
    match k.code {
        KeyCode::Up => {
            form.move_field(false);
            return FormAction::Edited;
        }
        KeyCode::Down => {
            form.move_field(true);
            return FormAction::Edited;
        }
        KeyCode::Char('n') if form.field.multi() => {
            form.add_row();
            return FormAction::Edited;
        }
        KeyCode::Char('d') if form.field.multi() && form.rows() > 1 => {
            form.drop_row();
            return FormAction::Edited;
        }
        _ => {}
    }
    if form.field.is_body() {
        return on_key_secret(&mut form.body, k);
    }
    match input::on_key(form.buf(), k) {
        InputAction::Submit => FormAction::Submit,
        InputAction::Cancel => FormAction::Cancel,
        InputAction::Edited | InputAction::Ignored => FormAction::Edited,
    }
}

/// 钥匙登记的发起规格由表单算出(供 `App` 下发两段用)。面板侧就拒(空名字 /
/// 轮换日形状不对 / 本体没喂入),留表单可改,不喂给引擎跑一趟。作用域多宽、
/// 何时轮换**只登记不设闸**——面板不代管、不提醒、不评审。
pub fn spec_from_key_form(form: &KeyForm) -> Result<KeySpec, Vec<String>> {
    let name = form.name.trim().to_string();
    let host = form.host.trim().to_string();
    let rotated_at = form.rotated_at.trim().to_string();
    let mut bad = Vec::new();
    if name.is_empty() {
        bad.push("名字为空".into());
    }
    // 轮换日形状错(非严格 YYYY-MM-DD)面板侧就拒(与到期日同一条 strict 形状校验;
    // 空 = 缺省登记日 UTC,合法)。只校形状,不设闸、不评审。
    if !valid_expires(&rotated_at) {
        bad.push(format!(
            "轮换日「{rotated_at}」不是严格 YYYY-MM-DD(可留空 = 登记日 UTC)"
        ));
    }
    if form.body.is_empty() {
        bad.push("钥匙本体还没喂入(只经 stdin 进:在钥匙框贴一次)".into());
    }
    if !bad.is_empty() {
        return Err(bad);
    }
    // 作用域:一行一条,空行 = 没填这条(不是坏行);非空行原样收(只登记不设闸)。
    let scopes = form
        .scopes
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Ok(KeySpec {
        name,
        host,
        scopes,
        rotated_at: (!rotated_at.is_empty()).then_some(rotated_at),
    })
}

/// 新建一张空钥匙表单(供 `App` 发起登记用)。
pub fn new_key_form() -> KeyForm {
    KeyForm::new()
}

// ---- vps 登记表单(多组 --ip / --login 用「一行一条」的多行缓冲) ----

/// 表单字段(顺序即 ↑/↓ 轮转顺序)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum VpsField {
    #[default]
    Name,
    Ips,
    Port,
    Logins,
    Note,
}

impl VpsField {
    pub const ALL: [VpsField; 5] = [
        VpsField::Name,
        VpsField::Ips,
        VpsField::Port,
        VpsField::Logins,
        VpsField::Note,
    ];

    /// 字段标签(呈现与键位提示同源)。
    pub fn label(self) -> &'static str {
        match self {
            VpsField::Name => "名字",
            VpsField::Ips => "地址",
            VpsField::Port => "端口",
            VpsField::Logins => "登录",
            VpsField::Note => "备注",
        }
    }

    /// 多行字段(一行一条)——单行字段给 false。
    pub fn multi(self) -> bool {
        matches!(self, VpsField::Ips | VpsField::Logins)
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// vps 登记表单:名字 / 若干条地址 / 端口 / 若干条登录 / 备注。
/// 多组 `--ip` / `--login` 收成「一行一条」的多行缓冲——比 N 个单行框
/// 简单,且可测(每行就是一个字符串,断言直接)。
#[derive(Clone, Default)]
pub struct VpsForm {
    pub name: String,
    pub ips: Vec<String>,
    pub port: String,
    pub logins: Vec<String>,
    pub note: String,
    pub field: VpsField,
    /// 多行字段里的当前行(单行字段恒 0)。
    pub row: usize,
}

impl VpsForm {
    fn new() -> Self {
        Self {
            ips: vec![String::new()],
            logins: vec![String::new()],
            ..Self::default()
        }
    }

    /// 当前字段的缓冲(单行字段借字段本身,多行字段借当前行)。
    fn buf(&mut self) -> &mut String {
        match self.field {
            VpsField::Name => &mut self.name,
            VpsField::Port => &mut self.port,
            VpsField::Note => &mut self.note,
            VpsField::Ips => {
                while self.row >= self.ips.len() {
                    self.ips.push(String::new());
                }
                &mut self.ips[self.row]
            }
            VpsField::Logins => {
                while self.row >= self.logins.len() {
                    self.logins.push(String::new());
                }
                &mut self.logins[self.row]
            }
        }
    }

    fn rows(&self) -> usize {
        match self.field {
            VpsField::Ips => self.ips.len(),
            VpsField::Logins => self.logins.len(),
            _ => 1,
        }
    }

    /// ↑/↓ 移字段(五行轮转,到顶/到底不越界、不绕回)。
    fn move_field(&mut self, down: bool) {
        let i = self.field.index();
        let n = VpsField::ALL.len();
        self.field = VpsField::ALL[if down {
            (i + 1).min(n - 1)
        } else {
            i.saturating_sub(1)
        }];
        self.row = 0;
    }

    /// 多行字段新行(追加一条空行,行光标跟上)。
    fn add_row(&mut self) {
        match self.field {
            VpsField::Ips => {
                self.ips.push(String::new());
                self.row = self.ips.len() - 1;
            }
            VpsField::Logins => {
                self.logins.push(String::new());
                self.row = self.logins.len() - 1;
            }
            _ => {}
        }
    }

    /// 删当前行(至少留一行——表单不塌成空)。
    fn drop_row(&mut self) {
        match self.field {
            VpsField::Ips if self.ips.len() > 1 => {
                self.ips.remove(self.row);
                self.row = self.row.min(self.ips.len() - 1);
            }
            VpsField::Logins if self.logins.len() > 1 => {
                self.logins.remove(self.row);
                self.row = self.row.min(self.logins.len() - 1);
            }
            _ => {}
        }
    }
}

// ---- 编目册屏渲染 ----

/// 编目册屏(机器册 / 标签册 / 订阅册 / 钥匙册四本,`m` 换册)。常畅态 /
/// 紧凑态同一份正文
/// (只随宽度折行,不引新断点、不缩放字号):当前是哪一本 + 编目路径 +
/// 清单 + 键位行;登记与删除各是一个浮层。
///
/// **册体行序是命中区的几何基准**(`layout::catalog_hits` 按 `row_lines()
/// 逐条铺),所以「当前是哪一本」与编目路径合并在首行,不另起一行——
/// 多一行就整体错位,点哪打哪就断了。**末行归键位行**(#73):册体条目
/// 永远铺不到末行,键位行也永远钉在末行。
pub fn render_catalog(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let kind = app.catalog_kind;
    let block = body(app, "机器写 · 永不手改");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let w = body_text_width(inner);

    let mut lines = catalog_body_lines(app, w);
    // 键位行钉在册体内层**末行**(#73 补 #67 欠的那一半):册体内容超量就截到
    // 末行之前(与册体行同一个 `clip` 纪律——截断,不折行;折行会多占一行,
    // 末行几何即失真),欠量垫空行补到末行之前。于是键位行的 y 只由 `inner`
    // 的几何决定,册体 1 条与 20 条它都在同一行(规格故事 7)。命中侧
    // (`layout::catalog_hits`)按同一个末行算矩形——一处几何真相。
    // 行数取「到键位行为止」的那一个数:与命中侧同一个 `catalog_key_row`。
    lines.resize(
        crate::layout::catalog_key_row(inner).saturating_sub(inner.y) as usize,
        Line::from(String::new()),
    );
    lines.push(key_line(inner, &catalog_key_parts(app), p));
    frame.render_widget(Paragraph::new(lines).style(p.surface()), inner);

    // 浮层:登记 / 删除确认(弹层优先级高于册体;骨架同一件)。
    // 画哪个、谁盖谁都在 `CatalogKind::overlay` 这张表上(#66),本函数不再分册。
    if let Some(form) = kind.overlay(app) {
        kind.draw_overlay(form, frame, area, app);
    }
    // 反查上浮结果浮层(#70):机器册上一个键问「什么标签引用了这一条」的结论
    // (有 N 条 / 一条都没有两态)。薄浮层,不是新屏、不是新册;几何与命中区
    // 共用 `vps_reverse_popup_rect`。换册已把状态清成 Idle,所以这本就画在
    // 机器册上。
    if app.vps_reverse_open() {
        render_vps_reverse_popup(frame, area, app);
    }
    // 反查跳转浮层(#120):标签册「这一条引用了什么」的命中条目清单。与上面
    // 那一层同族(册体不动、薄浮层叠加),但**只标签册开**——换册已把状态清
    // 成 Idle(见 `App::cycle_catalog_kind`),所以这本就画在标签册上。
    //
    // 基准矩形取 `inner`(册体内层)而不是 `area`:命中区(`layout::catalog_hits`
    // 收的也是 shrink 过的 inner)按同一个矩形算命中行的 y——渲染与命中
    // 共用一处几何,点哪打哪(ADR-0005)。上面那一层只用「整块关闭」命中区,
    // 基准取哪边都看不出分歧;这一层有逐行命中区,基准必须钉死在同一处。
    if app.tag_jump_open() {
        render_tag_jump_popup(frame, inner, app);
    }
}

/// 标签册的边界表述(ADR-0009 §5,工单 #54 验收第 7 条):标签随组织演进,
/// 烤进锁定集等于每次改标签重烤一次派——面板不提供任何这样的入口。
const TAG_BOUNDARY: &str = "  边界:标签绝不进配方/锁定集/bundle/装配(不提供烤进去的入口)";

/// 编目册正文行的**宽度预算**(#65 评审第 6 条)。`render_catalog` 与
/// 册体行命中区(`layout::catalog_hits` 里 `push_sheet_rows` 的列宽)都按这一个
/// 函数问「正文行按多宽算」——早先两边各写一遍(渲染 `inner.width - 2`、命中区
/// `inner.width`),同一块屏上两个数。收成一处:哪天真要改预算,两边一起变。
///
/// #73 起它**不再参与键位行的行号**:键位行钉在册体内层末行(见
/// [`render_catalog`]),行号只由 `inner` 的高度决定,与正文有多少行无关。早先
/// 命中区要靠「数正文行数」推键位行的 y,而这个行数又依赖这里的宽度预算
/// (`clip` 截断、从不折行,所以两边的行数才一致)——一条为几何服务的隐性
/// 依赖,如今随末行钉住一起消失。
pub(crate) fn body_text_width(inner: Rect) -> usize {
    inner.width.saturating_sub(2) as usize
}

/// 编目册**正文行**(键位行之前的一切:标题 + 册体 + 各册附加行)。
/// #67 抽出供渲染与命中测试共用。
///
/// #73 起它的行数**只服务渲染**(册体内容截到末行之前,见
/// [`render_catalog`]):命中区不再数它——键位行的 y 是末行,一处布局事实。
pub(crate) fn catalog_body_lines(app: &App, w: usize) -> Vec<Line<'static>> {
    let p = &app.palette;
    let kind = app.catalog_kind;
    let mut lines: Vec<Line> = Vec::new();
    // 引擎没有这一本的能力:入口禁用并指路(既有门控语义不变,面板不自造替代品)
    if !app.capabilities.enabled(kind.capability()) {
        lines.push(Line::from(Span::styled(
            format!(" ⚠ {} 册入口禁用:{}", kind.label(), capability::GATE_HINT),
            p.warn().bold(),
        )));
        lines.push(Line::from(Span::styled(
            "  缺的是引擎能力,不是面板坏了;编目册内容此时不可信也不呈现。",
            p.dim(),
        )));
    } else if app.data_root_cause().is_some() {
        lines.extend(data_root_signpost(app));
    } else {
        // 册内过滤态(#69):标题显「命中/总数」——被过滤空在标题层就与册体
        // 真空 / 还没读到可区分(规格故事 19)。不过滤时 count 段为空,与过滤前
        // 逐字节相同(面板永不重排引擎输出,故事 18/30)。
        let filtering = !app.catalog_filter().trim().is_empty();
        let count = if filtering {
            format!(" · 命中 {}/{}", app.catalog_visible().len(), kind.len(app))
        } else {
            String::new()
        };
        // 过滤态多出 count 段,路径按同一总预算再让一截,整行不因过滤而溢宽。
        let path_budget =
            w.saturating_sub(20 + unicode_width::UnicodeWidthStr::width(count.as_str()));
        lines.push(Line::from(Span::styled(
            format!(
                "  当前册 {} · {} · 编目 {}{}",
                kind.label(),
                app.catalog_scope_label(),
                clip(
                    &app.catalog_sheet()
                        .map(|s| s.catalog_path)
                        .unwrap_or_else(|| "—".into()),
                    path_budget
                ),
                count
            ),
            p.dim(),
        )));
        // 反查上浮的第三态「读不到」(#70 规格验收 3 / 故事 34):rc≠0 或契约
        // 守卫合成「需升级 nw」。机器册这一本没有带参反查态(`catalog_ls_error`
        // 恒 None),若不在这里给一句人话,操作者按了 Enter 屏上一个字都不变,
        // 只会当成「面板没反应」。走既有指路的同一套措辞(不新造一套话),
        // 并且**绝不画成空列表**——「这次没读到」与「一条标签引用都没有」是
        // 两件事,话不许互相冒充(spec #65 铁律 4)。册体与选中行原样在场:
        // 反查从头到尾不写 `vps_sel`,读不到也不该把册体收走。
        //
        // **只在机器册画**(`kind == CatalogKind::Vps`):反查上浮是机器册独有
        // 的动作,换册(`cycle_catalog_kind`)会把 `vps_reverse` 清回 Idle,所以
        // 今天别三本册上这条指路本来就画不出来。但把不变量写在**用它的这一处**
        // 本地,而不是靠调用方在远处复位:将来哪条路径换了册却忘了清
        // `vps_reverse`,画出来的会是机器册的指路摊在标签 / 订阅 / 钥匙册上——
        // 一态冒充另一态。
        if kind == CatalogKind::Vps {
            if let VpsReverse::Unreadable { reason, .. } = &app.vps_reverse {
                lines.push(Line::from(Span::styled(
                    format!("  ⚠ 反查没结果:{}", clip(reason, w.saturating_sub(12))),
                    p.warn().bold(),
                )));
                lines.push(Line::from(Span::styled(
                    "  标签册这次没读到,不是「一条标签引用都没有」;册体与选中行原样不动。",
                    p.dim(),
                )));
            }
        }
        // 反查回音的终态(#54):问错了名字(rc 3)/ 合法空态 / 有内容,
        // 三者互不冒充——都画成「空列表」会丢掉「这个名字不存在」的信息
        // (51 §6.11)。机器册没有带参反查这一态,`catalog_ls_error` 恒 None。
        if let Some(err) = app.catalog_ls_error() {
            lines.push(Line::from(Span::styled(
                format!("  ⚠ 反查没结果:{}", err.headline()),
                p.warn().bold(),
            )));
            if let Some(env) = err.envelope.as_ref() {
                for e in env.errors.iter().take(2) {
                    crate::screens::recipes::push_evidence(&mut lines, "出错", p.fail(), e, w);
                }
            }
            lines.push(Line::from(Span::styled(
                "  标签不在编目里 = 问错了名字(rc 3),不是空结果;册体此时不呈现。",
                p.dim(),
            )));
        } else {
            match kind.sheet(app) {
                None => lines.push(Line::from(Span::styled(kind.unread_line(), p.dim()))),
                Some(s) if s.entries.is_empty() => {
                    // 合法空态两态可区分:标题(正文色)+ 下一步(弱化)
                    let [title, next] = kind.empty_lines();
                    lines.push(Line::from(Span::styled(title, p.text())));
                    lines.push(Line::from(Span::styled(next, p.dim())));
                }
                Some(s) => {
                    // 册内过滤(#69):只画「看得见的那一行」——下标经
                    // `catalog_visible` 映射回原始投影。无关键词时 vis = 0..n
                    // 全量同序,与过滤前逐字节相同。
                    let vis = app.catalog_visible();
                    if vis.is_empty() {
                        // 关键词把条目全筛掉了:与「册体本来是空的」两态可
                        // 区分,给改词 / 清过滤的下一步,绝不共用一个空列表。
                        lines.extend(filter_empty_lines(app, s.entries.len(), p));
                    } else {
                        lines.extend(catalog_sheet_lines(
                            &s,
                            &vis,
                            w,
                            app.catalog_sel(),
                            p,
                            kind.detail(),
                        ));
                    }
                }
            }
        }
        // 只属于某一本的附述行(订阅册的到期三态汇总 + 取回链接建议;
        // 钥匙册的取回钥匙建议)。机器册 / 标签册没有这一族关切,
        // `CatalogKind::extra_lines` 给空——渲染侧不再有两段 `if kind ==`。
        // ADR-0009 §1:面板不代跑 get;两本的投影都没有本体槽。
        lines.extend(kind.extra_lines(app, w, p));
        // 边界表述(ADR-0009 §5):编目条目绝不进配方 / 锁定集 / bundle /
        // 装配命令,面板侧不提供任何把它烤进去的入口(各本一句,同一条
        // 纪律;机器册没有这一句)。
        if let Some(b) = kind.boundary() {
            lines.push(Line::from(Span::styled(clip(b, w), p.dim())));
        }
    }
    lines
}

/// 键位行可点段之间(以及段内各项之间)的分隔串:渲染与命中共用同一份,
/// 段宽按 `unicode_width` 量——一处真相,点哪打哪。
///
/// #69 收窄到 **2 列**(`" ·"`,原为 3 列的 `" · "`):订阅 / 钥匙两本册的
/// 键位行在 66×50 下限(正文 64 列)内放不下 `m 换册` + `/ 过滤` 两段,于是
/// 鼠标过滤入口只在机器册有、键盘 `/` 却四本册都能按(故事 29 要的是两边
/// 都能用)。gap 是这一个常量、渲染与命中共用,收窄它不丢任何一段文案,
/// 只把分隔从「空格·空格」改成「·空格」。收窄后四本册两档都放得下,还剩
/// ≥2 列余量(见 `过滤段四本册两档都放得下_渲染与命中共源`)。
pub(crate) const CATALOG_KEY_GAP: &str = " ·";

/// 键位行末段的「换册」可点文案(#67):点它 = 按 `m` 轮转四本册。渲染画这个
/// 串、命中测这个串的宽,同一份字符串;段可点(accent 高亮),与键盘 `m` 同路。
pub(crate) const CATALOG_SWITCH: &str = "m 换册";

/// 键位行「Enter 反查」可点文案(#70):点它 = 机器册选中行上按 `Enter`
/// (`App::vps_reverse_lookup`)。渲染画这个串、命中测这个串的宽,同一份
/// 字符串——点哪打哪(ADR-0005)。
///
/// **只机器册给这一段**:`TAG_REF_KINDS` 的已知集合是 `skill` / `extension`
/// / `vps`,四本册里只有机器册的条目可打标签;订阅 / 钥匙册放这个动作注定
/// 只返回「没有」,是噪声(spec #65 §4:触发点只有一处)。
pub(crate) const CATALOG_REVERSE: &str = "Enter 反查";

/// 键位行「过滤」可点文案(#69):点它 = 按 `/` 进册内过滤输入态。与
/// [`CATALOG_SWITCH`] 同一份接缝——渲染画这个串、命中测这个串的宽,
/// 段可点(accent 高亮),与键盘 `/` 同路(点哪打哪)。
pub(crate) const CATALOG_FILTER: &str = "/ 过滤";

/// 键位行「g 取回」可点文案(#73):点它 = 按 `g`——当前选中订阅 / 钥匙的
/// **建议命令**收进日志,执行权留给操作者(ADR-0009 §1:面板不代跑 get,
/// 上游才是唯一回显本体的出口)。渲染画这个串、命中测这个串的宽,同一份
/// 字符串——点哪打哪(ADR-0005)。
///
/// 与 [`CATALOG_SWITCH`] / [`CATALOG_REVERSE`] / [`CATALOG_FILTER`] 不同的
/// 是产地:那三段由 [`catalog_key_parts`] 现拼(它们的在场条件跨册推得到),
/// 本段的文案与命中目标都写在 [`CatalogKind::keys_body`] 的订阅 / 钥匙两行里
/// ——「这一段可不可点」与「`g` 这个键在这一本上有没有动作」是同一张表的
/// 同一行(#66 查表纪律,#73 不为它加 `if kind ==` 分叉)。
pub(crate) const CATALOG_GET: &str = "g 取回";

/// 键位行「d 删除」可点文案(#78,兑 #75 裁决 A:点 `d` 段 ≡ 按 `d`):点它 =
/// 按 `d`——选中行 → ADR-0009 图形确认闸 → 确认才 `rm`,与键盘完全同一路。
/// 渲染画这个串、命中测这个串的宽,同一份字符串——点哪打哪(ADR-0005)。
///
/// 机器 / 订阅 / 钥匙三本共用这一个文案;标签册的删除是「摘引用 / 删整个标签」
/// 两态,用 [`CATALOG_DELETE_TAG`]。产地与 [`CATALOG_GET`] 同:文案与命中目标
/// 都写在 [`CatalogKind::keys_body`] 的四行里——「这一段可不可点」与「`d` 这个
/// 键在这一本上有没有动作」是同一张表的同一行(#66 查表纪律,#78 不为它加
/// `if kind ==` 分叉)。
pub(crate) const CATALOG_DELETE: &str = "d 删除";

/// 标签册键位行删除段的文案(#78):它的 `d` 是**两态**——引用留空 = 删整个标签,
/// 填 `kind/条目` = 只摘那几条(嵌套在表单流里,不是rm 确认弹层),故段面写成
/// 「d 摘/删」而不是「d 删除」。点它 = 按 `d`,与键盘进同一条表单流
/// ([`App::start`](crate::app::App::start) 的 `StartMode::Rm` 一个入口)。
pub(crate) const CATALOG_DELETE_TAG: &str = "d 摘/删";

/// 编目册键位行的**段清单**(#67 通用接缝的第一个用户):提示段只画不点
/// (`None`),末段「换册」可点(`Some(SwitchBook)`)。渲染与命中测试共用这
/// 同一份清单——段文案与段宽一处真相。
///
/// 册级键位行按 kind 查表(#66):「在途」「空册」「有条目」三个判定与三组段
/// 清单全在 `CatalogKind` 成员上(`flow` / `rm_confirm` / `len` /
/// `keys_empty` / `keys_body`),本函数不再有第四份 `match app.catalog_kind`
/// ——键位行画的和命中区算的走同一口,不可能各说各话。
///
/// **规格订正(#67 承载)**:可点动作段画在**正文里**,不画在 HUD——命中区模型
/// 历来把顶栏与 HUD 整行切在外面(spec #15/#16)。在途浮层/删除确认态换册键被
/// 浮层受理(`m` 到不了 `key_catalog`),键位行只留浮层提示、不画可点段。
///
/// **浮层态的 `Esc/q 返航` 是有意两键都写**(#65 评审第 8 条:这一处与册体
/// 态口径不同,理由键在这儿,别再当成遗漏):登记流 / 删除确认浮层在场时,
/// `on_key` 先把键交给浮层那层(`RegState` 非 Idle 走 `on_key_reg_flow`、
/// `as_confirm` 走确认分支),`Esc` 与 `q` **都**收敛浮层
/// (`cancel_register` / `dismiss_popup`),壳级的 `q => quit` 那一次判定排
/// 在更后面,根本轮不到。所以这一态写两键不是虚指。册体态反过来:
/// `q` 直接 quit、`Esc` 只清过滤词,故 `keys_body` 收窄成裸 `Esc`(机器册)、
/// 另三本干脆不给 Esc 段。
///
/// 第三项 `live`(#70 故事 39):「这一段此刻是活的吗」。**只有反查段会传
/// `false`**——门禁是 `App::tag_entry_enabled`(数据仓根优先、再看标签组
/// 能力),与 `App::vps_reverse_lookup` 进门那一个判据同一个,不另推一份;
/// 过不了时这一条**照画、照给命中**(禁用并指路:点下去仍走
/// `vps_reverse_lookup`,仍给人话、仍零引擎调用),只是画成 dim。其余段
/// (提示段与其余可点段)一律 `true`。
///
/// `g 取回` 段(#73)**不在这里现拼**:它的文案与命中目标跟着
/// [`CatalogKind::keys_body`] 的订阅 / 钥匙两行走(见 [`CATALOG_GET`]),本函数
/// 只把查表结果原样搬进段清单——「这一段在不在场」= 「`g` 这个键在这一本上
/// 有没有动作」,一处真相,不为鼠标复制第二份在场条件。
pub(crate) fn catalog_key_parts(app: &App) -> Vec<crate::layout::KeyPart<'static>> {
    use crate::layout::Target;
    let kind = app.catalog_kind;
    if kind.flow(app).phase != RegPhase::Idle || kind.rm_confirm(app) {
        return vec![("  Enter 确认", None, true), ("Esc/q 返航", None, true)];
    }
    // 查表即段清单:提示段(`None`)与可点动作段(`Some(..)`)都在表里,
    // 本函数不再把整行拆成「只画不点」的一等公民。#66 收敛成的查表纪律
    // 不为任何一票回退成 `if kind ==`。
    let mut parts: Vec<crate::layout::KeyPart<'static>> = if kind.len(app) == 0 {
        kind.keys_empty().to_vec()
    } else {
        kind.keys_body().to_vec()
    };
    // 反查上浮触发段(#70):只机器册、且册体有条目(= 选中了一行)才给。
    // 判据与 `keys_body` / `keys_empty` 的分岔同一个——空册没有「选中的这一
    // 条」可问,面板不空手问,也不画一个点下去只得到一行指路的段。
    if kind == CatalogKind::Vps && kind.len(app) > 0 {
        // 门禁过不了:段仍在场、仍可点(指路照旧发生),画成 dim(#70 故事 39
        // 「入口禁用并指路」)。判据与 `App::vps_reverse_lookup` 进门那一个
        // 同一个 `tag_entry_enabled`,不在这儿另推一份。
        parts.push((
            CATALOG_REVERSE,
            Some(Target::VpsReverse),
            app.tag_entry_enabled(),
        ));
    }
    parts.push((CATALOG_SWITCH, Some(Target::SwitchBook), true));
    // 册内过滤入口(#69):有条目可筛时才给这一段(空册没东西可筛,给了反而
    // 误导)。点它 = 按 `/`,与键盘同一路;空册键位行(`keys_empty`)不带它。
    if kind.len(app) > 0 {
        parts.push((CATALOG_FILTER, Some(Target::CatalogFilter), true));
    }
    parts
}

/// 把段清单画成一行 `Line`(渲染侧):**可用**可点段 accent 加粗,提示段与
/// **禁用态**可点段(#70 故事 39)都 dim;段间用 [`CATALOG_KEY_GAP`] 连接。
/// 与命中侧
/// 编目册键位行的**渲染**(#67/#69/#70):段清单 `parts` 由
/// [`catalog_key_parts`] 一处产出,渲染与命中测试共用。
///
/// #69 修一处「渲染与命中不同源」的隐患:键位行若直接铺全部段、靠终端
/// 裁尾,会在「段放不下」时画出一截点不到的残段(如紧凑态标签册的 `· / `),
/// 而命中侧 [`crate::layout::key_line_segments`] 早就是「放不下整段收口、
/// 这一段及后续既不画也不给命中」。两边口径一旦分叉,就破了 ADR-0005「渲染
/// 与命中同源」。故渲染也走同一份 [`crate::layout::key_line_segments`]:
/// 放得下的段才画、放不下的段两厢都不出现——屏上没有的段,鼠标也点不到,
/// 点哪打哪。
///
/// #69 后续(本工单)把「放不下」这件事在四本册 × 两档内**做没了**:键位行
/// 分隔收窄到 2 列 + 订阅 / 钥匙册 `g 取回命令` → `g 取回` 之后,`m 换册`
/// 与 `/ 过滤` 在 66×50 下限内都放得下(见
/// `过滤段四本册两档都放得下_渲染与命中共源`)。收口纪律本身留着——它管的是
/// 「更窄的终端 / 将来更长的键位行」,不是现在这四行。
///
/// #70 故事 39 的**禁用态**:`live == false` 的可点段画成 dim(与提示段同色,
/// 不再 accent 加粗),段本身照画、命中区照给——操作者在点之前就看出
///「这台引擎干不了」,点下去仍由动作自己收口给人话(`App::vps_reverse_lookup`
/// 的门禁指路)。不按能力把这一段收掉:那会把机器册的键位行耦合到另一本册
/// 的能力上(#66 刚刚拆掉的耦合),规格要的也是「禁用并指路」而不是「消失」。
fn key_line(inner: Rect, parts: &[crate::layout::KeyPart<'_>], p: &Palette) -> Line<'static> {
    // 与命中侧同一 anchor(`inner`)、同一 gap:逐字同源。y 只进命中矩形,
    // 渲染不取矩形,传 0 即可。
    let segs = crate::layout::key_line_segments(inner, 0, parts, CATALOG_KEY_GAP);
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, seg) in segs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(CATALOG_KEY_GAP, p.dim()));
        }
        let style = match (seg.target.is_some(), seg.live) {
            (true, true) => p.accent().bold(), // 可用动作段
            (true, false) => p.dim(),          // #70 禁用态:在场、可点,但不加强调
            _ => p.dim(),                      // 提示段
        };
        spans.push(Span::styled(seg.text.clone(), style));
    }
    Line::from(spans)
}

/// **编目册列表件**(#53 骨架第二件):按投影字段渲染,遵循确定性排序。
/// 缺字段显式「——」,不自造、不补全;ref 原样呈现(指针不是密钥),
/// 摘要不印 ref 与 addr。后三本册子只换 `CatalogSheet` 的数据源与
/// [`EntryDetail`](条目明细画法),不换列表件。
///
/// #69 册内过滤:`vis` 是「看得见的那一行」在原始投影里的下标(
/// [`App::catalog_visible`] 的产出),本件只画 `vis` 指到的那些条目、按
/// `vis` 的顺序;选中高亮按**可见位置** `pos == sel`。无关键词时 `vis` =
/// `0..n` 全量同序,画出来的与过滤前逐字节相同(面板永不重排引擎输出)。
pub fn catalog_sheet_lines(
    sheet: &CatalogSheet,
    vis: &[usize],
    width: usize,
    sel: usize,
    p: &Palette,
    detail: EntryDetail,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for (pos, &i) in vis.iter().enumerate() {
        let Some(e) = sheet.entries.get(i) else {
            continue;
        };
        let on = pos == sel;
        let (suffix, rows) = detail(e, width.saturating_sub(12), p);
        let mut head = vec![Span::raw(format!(
            "  {} {}",
            if on { "▸" } else { " " },
            clip(&e.name, 18)
        ))];
        if let Some(s) = suffix {
            head.push(Span::raw("  "));
            head.push(Span::styled(s, p.dim()));
        }
        out.push(Line::from(head));
        out.extend(rows);
    }
    out
}

/// 册内过滤把条目全筛掉了的指路(#69,照 `recipes::ListEmptyCause` 的范式:
/// 一个概念一个拼写,与「册体本来是空的」「还没读到」三态两两可区分,绝不
/// 共用一个空列表)。标题(warn 色,与合法空态的正文色区分)+ 两句下一步:
/// 事实(「q」在 total 条里没命中)+ 出路(改词 / 清过滤)。`total` 是册体
/// 原始条数(分母),让操作者知道是「被筛掉了」而不是「本来就没有」。
fn filter_empty_lines(app: &App, total: usize, p: &Palette) -> Vec<Line<'static>> {
    let q = app.catalog_filter().trim();
    vec![
        Line::from(Span::styled(
            format!("  {}册无命中", app.catalog_kind.label()),
            p.warn(),
        )),
        Line::from(Span::styled(
            format!("  「{q}」在 {total} 条里没有命中。"),
            p.dim(),
        )),
        Line::from(Span::styled("  按 / 改词 · Esc 清空过滤", p.dim())),
    ]
}

/// 机器册的条目明细:名字行后缀给端口,之下给 地址 / 登录 / 备注
/// (缺字段显式「——」;ref 原样,摘要不印 ref 与 addr)。
fn vps_detail_rows(e: &CatalogEntryView, width: usize, p: &Palette) -> EntryDetailOut {
    (
        Some(format!("端口 {}", e.port.as_deref().unwrap_or("——"))),
        vec![
            Line::from(vec![
                Span::raw("    地址 "),
                Span::styled(clip(&e.addr_line(), width), p.text()),
            ]),
            Line::from(vec![
                Span::raw("    登录 "),
                Span::styled(clip(&e.login_line(), width), p.text()),
            ]),
            Line::from(vec![
                Span::raw("    备注 "),
                Span::styled(clip(e.note.as_deref().unwrap_or("——"), width), p.dim()),
            ]),
        ],
    )
}

/// 标签册的条目明细:没有名字行后缀(标签没有端口这一槽),之下给
/// **引用清单**(`kind/id` 逐条;一标签挂多条目、跨 kind)。超宽按宽度
/// 截断——与机器册的地址/登录行同一口径,不另造折行规则。
fn tag_detail_rows(e: &CatalogEntryView, width: usize, p: &Palette) -> EntryDetailOut {
    (
        None,
        vec![Line::from(vec![
            Span::raw("    引用 "),
            Span::styled(clip(&e.ref_line(), width), p.text()),
        ])],
    )
}

/// 登记浮层总入口:按阶段画一个;Idle 无浮层(几何与命中区同一 `popup_rect`)。
fn render_register_overlay(frame: &mut Frame, area: Rect, app: &App) {
    match app.vps_flow.phase {
        RegPhase::Idle => {}
        RegPhase::Form => render_reg_form(frame, area, app),
        RegPhase::PreAudit => render_reg_preaudit(frame, area, app),
        RegPhase::Review => render_reg_review(frame, area, app),
        RegPhase::Commit => render_reg_commit(frame, area, app),
        RegPhase::Done => render_reg_done(frame, area, app),
    }
}

/// ① 表单:名字 / 地址(一行一条 label=addr)/ 端口 / 登录(一行一条
/// user=ref)/ 备注。当前字段带块光标,↑/↓ 移字段。
fn render_reg_form(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let f = &app.vps_form;
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  一行一条:地址 label=addr,登录 user=ref(ref 是指针不是密钥)",
        p.dim(),
    ))];
    for field in VpsField::ALL {
        let on = field == f.field;
        let rows: Vec<String> = match field {
            VpsField::Name => vec![f.name.clone()],
            VpsField::Port => vec![f.port.clone()],
            VpsField::Note => vec![f.note.clone()],
            VpsField::Ips => f.ips.clone(),
            VpsField::Logins => f.logins.clone(),
        };
        for (r, row) in rows.iter().enumerate() {
            let cur = on && r == f.row.min(rows.len().saturating_sub(1));
            let text = if cur {
                format!("{row}█")
            } else {
                row.clone()
            };
            lines.push(Line::from(vec![
                Span::raw(if r == 0 {
                    format!("  {} ", field.label())
                } else {
                    "      ".to_string()
                }),
                Span::styled(text, if cur { p.sel().bold() } else { p.text() }),
            ]));
        }
    }
    lines.push(Line::from(Span::styled(
        "  ↑/↓ 移字段 · n 新行 · d 删行 · Enter 预审 · Esc/q 返航",
        p.dim(),
    )));
    let budget = popup.height.saturating_sub(3) as usize;
    lines.truncate(budget);
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记机器 · vps 表单 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
    let _ = inner_w;
}

/// ② 预审进行中:五道闸在跑,文件不落。
fn render_reg_preaudit(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_INPUT_W, REG_INPUT_H);
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from(Span::styled("  正在预审:跑引擎的五道闸…", p.text())),
        Line::from(Span::styled(
            "  不带 --yes:文件不落,拟落账记录只进回音。",
            p.dim(),
        )),
        Line::from(Span::styled("  Esc 可返航(回音将丢弃)。", p.dim())),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记机器 · 预审 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ③ 确认态:摊开五道闸结论(按冻结 id,三态复用结果四态)+ 拟落账条目 +
/// 落账闸结论。**本段不下发任何写命令**——Enter 落账是下一段。
fn render_reg_review(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let out = app.vps_flow.result.as_ref();
    let audit = out.and_then(VpsSpec::audit);
    let mut lines: Vec<Line> = Vec::new();
    match audit.as_ref() {
        Some(a) => {
            lines.push(Line::from(Span::styled(
                format!(
                    "  编目  {}",
                    clip(&a.catalog_path, inner_w.saturating_sub(9))
                ),
                p.dim(),
            )));
            if let Some(e) = a.entry.as_ref() {
                lines.push(Line::from(format!(
                    "  拟落账  {} · 端口 {} · {}",
                    clip(&e.name, 16),
                    e.port.as_deref().unwrap_or("——"),
                    clip(&e.addr_line(), inner_w.saturating_sub(24))
                )));
                lines.push(Line::from(Span::styled(
                    format!(
                        "  登录    {}",
                        clip(&e.login_line(), inner_w.saturating_sub(11))
                    ),
                    p.dim(),
                )));
            }
            lines.push(Line::from(Span::styled("  闸(恒 5 行):", p.accent())));
            for g in &a.gates {
                let st = g.state();
                lines.push(Line::from(vec![
                    Span::raw("   "),
                    Span::styled(gate_label(st), p.state(st)),
                    Span::raw(" "),
                    Span::styled(g.id.clone(), p.dim()),
                    Span::raw(" "),
                    Span::styled(
                        clip(&g.message, inner_w.saturating_sub(10 + g.id.len())),
                        p.text(),
                    ),
                ]));
            }
            if let Some(env) = out.and_then(|o| o.envelope.as_ref()) {
                for e in env.errors.iter().take(2) {
                    crate::screens::recipes::push_evidence(
                        &mut lines,
                        "出错",
                        p.fail(),
                        e,
                        inner_w,
                    );
                }
            }
            if preaudit_passed(out.unwrap(), Some(a)) {
                lines.push(Line::from(Span::styled(
                    "  预审通过:五道闸无 fail、退出码 0(本段不写任何文件)",
                    p.info(),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    "  ⚠ 不可落账:闸有 fail 或退出码非 0——见上方结论",
                    p.warn().bold(),
                )));
            }
        }
        None => {
            let headline = out.map(|o| o.headline().to_string()).unwrap_or_default();
            for ln in wrap(
                &format!("  预审结果不可定型:{headline}"),
                inner_w.saturating_sub(2),
            )
            .into_iter()
            .take(3)
            {
                lines.push(Line::from(Span::styled(ln, p.warn())));
            }
            lines.push(Line::from(Span::styled(
                "  ⚠ 不可落账:拿不到可定型的预审结论(不按旧形状解析)",
                p.warn().bold(),
            )));
        }
    }
    let budget = popup.height.saturating_sub(3) as usize;
    if lines.len() > budget {
        lines.truncate(budget.saturating_sub(1));
        lines.push(Line::from(Span::styled("  …(屏内未全显)", p.dim())));
    }
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    let can = out
        .map(|o| preaudit_passed(o, audit.as_ref()))
        .unwrap_or(false);
    lines.push(if can {
        Line::from(vec![
            Span::styled("  Enter 落账 ", p.accent().bold()),
            Span::raw("  "),
            Span::styled("Esc 返航 ", p.dim()),
        ])
    } else {
        Line::from(Span::styled("  Esc 返航 ", p.dim()))
    });
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记机器 · 预审确认 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ④ 落账在途:带 `--yes` 的写命令已下发;登记与装配互斥。
fn render_reg_commit(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let root = app.usable_data_root().unwrap_or_else(|| "—".into());
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "  正在落账:引擎把 vps.json 写进数据仓根…",
            p.text(),
        )),
        Line::from(Span::styled(
            format!(
                "  写入目标  {}",
                clip(&format!("{root}/vps.json"), inner_w.saturating_sub(13))
            ),
            p.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  登记与装配互斥:写盘期间不接新装配。",
            p.dim(),
        )),
        Line::from(Span::styled(
            "  Esc 返航(回音将丢弃,互斥随回音收敛)",
            p.dim(),
        )),
    ];
    while lines.len() < popup.height.saturating_sub(3) as usize {
        lines.push(Line::from(""));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记机器 · 落账 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ⑤ 结果屏:写盘位置 + 记录五字段 + git 变更提示;失败路给证据五字段。
fn render_reg_done(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let out = app.vps_flow.result.as_ref();
    let mut lines: Vec<Line> = Vec::new();
    match out.and_then(VpsSpec::audit) {
        Some(a) if out.map(|o| o.ok).unwrap_or(false) => {
            lines.push(Line::from(Span::styled(
                format!(
                    "  已写入  {}",
                    clip(&a.catalog_path, inner_w.saturating_sub(11))
                ),
                p.info().bold(),
            )));
            if let Some(e) = a.entry.as_ref() {
                lines.push(Line::from(format!(
                    "  机器    {} · 端口 {}",
                    clip(&e.name, 18),
                    e.port.as_deref().unwrap_or("——")
                )));
                lines.push(Line::from(format!(
                    "  地址    {}",
                    clip(&e.addr_line(), inner_w.saturating_sub(11))
                )));
                lines.push(Line::from(format!(
                    "  登录    {}",
                    clip(&e.login_line(), inner_w.saturating_sub(11))
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ⚠ vps.json 是一次 git 变更:走 PR + 家长审,",
                p.warn().bold(),
            )));
            lines.push(Line::from(Span::styled(
                "    别把它当本地小改;编目册已即时刷新。",
                p.warn(),
            )));
        }
        _ => {
            let headline = out.map(|o| o.headline().to_string()).unwrap_or_default();
            for ln in wrap(
                &format!("  落账未完成:{headline}"),
                inner_w.saturating_sub(2),
            )
            .into_iter()
            .take(2)
            {
                lines.push(Line::from(Span::styled(ln, p.fail().bold())));
            }
            lines.push(Line::from(Span::styled(
                "  未写入任何文件;Esc 返航后可从表单重来。",
                p.dim(),
            )));
        }
    }
    while lines.len() < popup.height.saturating_sub(3) as usize {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled("  Esc/q 返航 ", p.dim())));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记机器 · 落账结果 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// 删除确认(ADR-0009 §2:上游 rm 没有 `--yes`,面板加一道图形确认——
/// 面板只决定「什么时候递出去」,下发的 argv 与 shell 里敲的逐字相同)。
fn render_rm_confirm(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_INPUT_W, REG_INPUT_H);
    frame.render_widget(Clear, popup);
    let name = app.selected_vps_name().unwrap_or_else(|| "—".into());
    let root = app.usable_data_root().unwrap_or_else(|| "—".into());
    let lines = vec![
        Line::from(Span::styled(format!("  删除机器 {name}?"), p.text().bold())),
        Line::from(Span::styled(
            format!("  下发  nw vps rm {name} --root {root}"),
            p.dim(),
        )),
        Line::from(Span::styled(
            "  引擎没有确认闸:点名即删。要恢复得重新登记一遍。",
            p.warn(),
        )),
        Line::from(Span::styled(
            "  删不存在的名:引擎 rc 3 + 文件一个字节不动。",
            p.dim(),
        )),
        Line::from(Span::styled(
            "  Enter 确认删除 · Esc/q 返航",
            p.accent().bold(),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 删除机器 · 确认 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// 反查命中标签的**引用一行**(`kind/id` 逐条,空数组 = 「——」):与标签册
/// 明细的 [`CatalogEntryView::ref_line`](crate::engine::CatalogEntryView::ref_line)
/// 同一拼法——面板不为浮层另造一种引用文案(一处真相)。
///
/// **只印指针,不印机密本体**:标签引用恒是 `<kind>/<条目>` 形式的指针,
/// 密钥本体只经 stdin 进、投影里根本没有(ADR-0009 §1)。
fn tag_hit_ref_line(refs: &[crate::engine::CatalogRef]) -> String {
    if refs.is_empty() {
        return "——".to_string();
    }
    refs.iter()
        .map(|r| r.label())
        .collect::<Vec<_>>()
        .join(" · ")
}

/// 反查上浮结果浮层的矩形(#70):**渲染与命中区共用这一处几何**(ADR-0005):
/// 一处真相,点哪打哪)。高按命中数现算——标题 1 行 + 命中标签 N 行 + 收口
/// 1 行 + 上下边框 2 行;「一条都没有」是固定 3 行内容。`None` = 没有可画
/// 的浮层(在途 / 闲置 / 读不到都不画薄浮层;读不到走既有指路)。
pub(crate) fn vps_reverse_popup_rect(area: Rect, app: &App) -> Option<Rect> {
    let content = match &app.vps_reverse {
        VpsReverse::Hits { tags, .. } => tags.len() + 2, // 标题 + N 标签 + 收口
        VpsReverse::Empty { .. } => 3,                   // 标题 + 下一步 + 收口
        _ => return None,
    };
    let h = u16::try_from(content).unwrap_or(u16::MAX).saturating_add(2); // +上下边框
    let popup = popup_rect(area, REVERSE_W, h);
    (popup.width > 0 && popup.height > 0).then_some(popup)
}

/// 反查上浮结果浮层(#70):一层薄浮层(不是新屏、不是新册),几何与命中区
/// 同一个 [`vps_reverse_popup_rect`]。三态里画得出的两态在这里:
/// [`Hits`](VpsReverse::Hits) 列标签名 + 各自引用;
/// [`Empty`](VpsReverse::Empty) 明说「没有标签引用这一条」+ 给「去挂第一个
/// 标签」的下一步。第三态「读不到」不进这里——它走既有指路(日志),**绝不
/// 画成空列表**。
///
/// **只印引用(`kind/id` 指针),不印任何机密本体**——与面板其余部分同纪律
/// (ADR-0009 §1:指针不是密钥;反查看不到、也不该看到密钥本体)。`Esc` 或
/// 任意点击关闭(命中区见 `layout::catalog_hits`),回到机器册原选中行。
fn render_vps_reverse_popup(frame: &mut Frame, area: Rect, app: &App) {
    let Some(popup) = vps_reverse_popup_rect(area, app) else {
        return;
    };
    let p = &app.palette;
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();
    match &app.vps_reverse {
        VpsReverse::Hits { machine, tags } => {
            lines.push(Line::from(Span::styled(
                format!("  {} 个标签引用了 {machine}", tags.len()),
                p.text().bold(),
            )));
            for t in tags {
                lines.push(Line::from(vec![
                    Span::raw("  ▸ "),
                    Span::styled(clip(&t.name, 18), p.accent().bold()),
                    Span::raw("  "),
                    Span::styled(
                        clip(&tag_hit_ref_line(&t.refs), inner_w.saturating_sub(24)),
                        p.dim(),
                    ),
                ]));
            }
        }
        VpsReverse::Empty { machine } => {
            lines.push(Line::from(Span::styled(
                format!("  没有标签引用 {machine}"),
                p.text().bold(),
            )));
            lines.push(Line::from(Span::styled(
                "  下一步:在机器册按 r 挂第一个标签(追加合并、幂等、无确认闸)。",
                p.dim(),
            )));
        }
        _ => return,
    }
    lines.push(Line::from(Span::styled("  Esc 或点击任意处关闭", p.dim())));
    let budget = popup.height.saturating_sub(2) as usize;
    lines.truncate(budget);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(
                        " 反查 · 什么标签引用了这一条 ",
                        p.accent().bold(),
                    )),
            )
            .style(p.surface()),
        popup,
    );
}

/// 反查跳转浮层的宽(#120):与反查上浮([`REVERSE_W`])同一个量级——一层
/// **薄浮层**,不是新屏、不是新册;同一个 [`popup_rect`] 基准。
pub const JUMP_W: u16 = 56;

/// 反查跳转浮层的矩形(#120):**渲染与命中共用这一处几何**(ADR-0005,同
/// [`vps_reverse_popup_rect`] 的法子)。高按命中数现算:标题 1 行 + N 条
/// 命中 + 收口 1 行 + 上下边框 2 行。`None` = 浮层没开场。
pub(crate) fn tag_jump_popup_rect(area: Rect, app: &App) -> Option<Rect> {
    if !app.tag_jump_open() {
        return None;
    }
    let content = app.tag_jump.hits().len().saturating_add(2);
    let h = u16::try_from(content).unwrap_or(u16::MAX).saturating_add(2);
    let popup = popup_rect(area, JUMP_W, h);
    (popup.width > 0 && popup.height > 0).then_some(popup)
}

/// 反查跳转浮层里第 `i` 条命中的矩形:渲染把第 i 条画在 `popup.y + 1 + i`,
/// 命中区按同一个矩形发 [`Target::TagJump`](crate::layout::Target::TagJump)
/// ——一处几何,点哪打哪。越出浮层可见区的行不给(末行是收口提示,
/// 与渲染的 `truncate(budget)` 同一个收口)。
pub(crate) fn tag_jump_row_rect(popup: Rect, i: usize) -> Option<Rect> {
    let budget = popup.height.saturating_sub(2) as usize; // 去掉上下边框
    let content_rows = budget.saturating_sub(1); // 末行留给收口提示
    if i + 1 > content_rows {
        return None;
    }
    let w = popup.width.saturating_sub(2);
    (w > 0).then(|| Rect::new(popup.x + 1, popup.y + 1 + i as u16, w, 1))
}

/// 反查跳转浮层(#120):标签册问「这一条标签引用了什么」之后,把命中条目
/// 列成一层薄浮层(册体原样在场,与 [`render_vps_reverse_popup`] 同族的
/// 叠加形态)。每行右侧写**去处**(机器册 / 外部源域),`→ 无去处` 的
/// warn 行明说这个 kind 在现有屏结构里落不到地——不是死键,Enter 给人话。
///
/// 只印 `kind/id` 指针,不印任何机密本体(与面板其余部分同纪律,ADR-0009
/// §1:指针不是密钥)。`Enter` 直跳所选、`↑/↓` 选、`Esc/q` 关闭。
fn render_tag_jump_popup(frame: &mut Frame, area: Rect, app: &App) {
    let Some(popup) = tag_jump_popup_rect(area, app) else {
        return;
    };
    let Some(tag) = app.tag_jump.tag() else {
        return;
    };
    let p = &app.palette;
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    // 右侧去处列的宽度预算:`→ 外部源域` 是去处里最长的一条
    let dest_w = 14;
    let label_budget = inner_w.saturating_sub(dest_w + 6).max(8);
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        format!("  标签 {tag} 引用了 {} 条", app.tag_jump.hits().len()),
        p.text().bold(),
    ))];
    let sel = app.tag_jump.sel();
    for (i, r) in app.tag_jump.hits().iter().enumerate() {
        let chosen = i == sel;
        let mark = if chosen { "▸" } else { " " };
        let (dest_text, dest_style) = match jump_dest(&r.kind) {
            Some(d) => (format!("→ {}", d.label()), p.info()),
            None => ("→ 无去处".to_string(), p.warn()),
        };
        lines.push(Line::from(vec![
            Span::raw(format!("  {mark} ")),
            Span::styled(
                clip(&r.label(), label_budget),
                if chosen { p.accent().bold() } else { p.text() },
            ),
            Span::raw("  "),
            Span::styled(dest_text, dest_style),
        ]));
    }
    lines.push(Line::from(Span::styled(
        "  Enter 直跳所选条目 · ↑/↓ 选 · Esc/q 关闭",
        p.dim(),
    )));
    let budget = popup.height.saturating_sub(2) as usize;
    lines.truncate(budget);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 反查 · 这一条引用了什么 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// `expiring` = Warn、`expired` = Fail;回音缺 `status` 字段 = 第四态 Skip
/// (「没这道标记」,显式「——」不猜)。三态本身仍是引擎的原词
/// (active / expiring / expired,上游 `status.go` 的三个取值),面板只做
/// 视觉映射,不改写、不翻译。
pub fn expiry_state(status: Option<&str>) -> State {
    match status.map(str::trim) {
        Some("active") => State::Pass,
        Some("expiring") => State::Warn,
        Some("expired") => State::Fail,
        _ => State::Skip,
    }
}

/// 订阅册的条目明细:名字行后缀给 client(消费这条链接的应用),之下给
/// **到期日 + 三态 + 剩余天数**(按 UTC、与引擎同一口径:引擎在 ls 回音里
/// 给足,面板不自己算一套;缺字段显式「——」)。
/// **没有链接槽**——投影里就没有这一格(上游 `SubSummary` 六个键冻结)。
fn sub_detail_rows(e: &CatalogEntryView, width: usize, p: &Palette) -> EntryDetailOut {
    let client = e
        .client
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or("——");
    let st = expiry_state(e.status.as_deref());
    let status_word = e
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("——");
    let days = match e.days_left {
        Some(n) => format!("剩 {n} 天"),
        None => "剩余 ——".to_string(),
    };
    (
        Some(format!("应用 {client}")), // client 是应用名(短),超宽由 clip 收口
        vec![Line::from(vec![
            Span::raw("    到期 "),
            Span::styled(clip(&e.expires_line(), width.saturating_sub(30)), p.text()),
            Span::raw("  "),
            Span::styled(gate_label(st), p.state(st)),
            Span::raw(" "),
            Span::styled(status_word.to_string(), p.state(st)),
            Span::raw("  "),
            Span::styled(days, p.state(st)),
        ])],
    )
}

/// 钥匙轮换账龄的 amber 阈值(天,spec #94 第 5 项)。**照订阅到期 7 天的
/// 先例,不配置化**;订阅那 7 天是上游 `status.go` 算好随回音下发的,这一条
/// 是面板自己的展示阈值——两处不同源,不许互相「对齐」成一套。只标记不治理:
/// 不碰退出码、不提醒、不自动轮换。
const KEY_AGE_WARN_DAYS: u32 = 7;

/// UTC 日历日——账龄运算的唯一时间粒度(整天,不涉时刻)。
///
/// 面板不引 chrono:账龄是**纯展示运算**(`rotated-at` 是静态登记字段,引擎
/// 不下发账龄;订阅三态「引擎在回音里算好、面板零日期运算」那条口径讲的是
/// 订阅,不禁这一处——spec #94 实现决策点名开了这个口子)。要的只是两个日历
/// 日之间的整天数,整数运算就够,不值得为一个差引一整个时区库。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct UtcDay {
    y: i32,
    m: u32,
    d: u32,
}

impl UtcDay {
    /// 严格 `YYYY-MM-DD`(引擎 `rotatedAt` 的契约形状)**且真是日历日**;
    /// 空串 / 形状不认 / `2026-02-30` 这种不存在的日子 = None。
    ///
    /// 形状那一层与登记表单共用同一条 strict 校验([`valid_expires`]);这里
    /// 多要一层「真是日历日」,是因为账龄要把日期换成天数——`2026-02-30`
    /// 会被静默滚成 3 月 2 日,印一个看着精确、其实无依据的天数。算不出就
    /// 说算不出,不猜(与 `days_left_line` 同一口径)。
    fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() || !valid_expires(s) {
            return None;
        }
        let y: i32 = s.get(0..4)?.parse().ok()?;
        let m: u32 = s.get(5..7)?.parse().ok()?;
        let d: u32 = s.get(8..10)?.parse().ok()?;
        if !(1..=12).contains(&m) || d == 0 || d > days_in_month(y, m) {
            return None;
        }
        Some(Self { y, m, d })
    }

    /// 日历日 → Unix 纪元天数(Howard Hinnant `days_from_civil`,纯整数,
    /// 无时区无库)。
    fn epoch_days(self) -> i64 {
        let y = i64::from(self.y) - i64::from(self.m <= 2);
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400; // [0, 399]
        let m = i64::from(self.m);
        let d = i64::from(self.d);
        let mp = if m > 2 { m - 3 } else { m + 9 }; // [0, 11]
        let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
        era * 146_097 + doe - 719_468
    }

    /// Unix 纪元天数 → 日历日(Hinnant `civil_from_days`)。
    fn from_epoch_days(z: i64) -> Self {
        let z = z + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097; // [0, 146096]
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11]
        let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1); // [1, 31]
        let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1); // [1, 12]
        Self {
            y: i32::try_from(if m <= 2 { y + 1 } else { y }).unwrap_or(1970),
            m,
            d,
        }
    }

    /// 今天(UTC):本模块唯一读时钟的地方。
    ///
    /// 每帧每条目读一次(廉价,`clock_gettime` 而已);要收成「一帧一次」得把
    /// today 传进 [`EntryDetail`] 签名,那是动骨架,本票不做。时钟早于 1970
    /// (不存在的机器)按纪元日算,账龄会大得离谱 → 亮 amber,方向安全。
    fn today() -> Self {
        #[cfg(test)]
        if let Some(d) = pinned_today() {
            return d;
        }
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self::from_epoch_days(i64::try_from(secs).unwrap_or(i64::MAX) / 86_400)
    }
}

// 测试钉死的「今天」。账龄随墙钟走,而 golden 是逐字节的字面量——不钉的话
// 同一帧每天都漂一次,钥匙册两档 golden 会天天红。thread_local 让并行测试
// 各钉各的,不必像 env 那样上串行锁(`testutil::ConfigGuard` 的取舍)。
#[cfg(test)]
thread_local! {
    static PINNED_TODAY: std::cell::Cell<Option<UtcDay>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn pinned_today() -> Option<UtcDay> {
    PINNED_TODAY.with(|c| c.get())
}

/// 钉死「今天」,见 `PINNED_TODAY`。
#[cfg(test)]
fn pin_today_for_test(y: i32, m: u32, d: u32) {
    PINNED_TODAY.with(|c| c.set(Some(UtcDay { y, m, d })));
}

/// 某年某月有几天(含闰年)。月越界返回 0——调用方据此判「不是真日历日」。
fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// 钥匙册的轮换账龄(spec #94 第 5 项):`rotated-at` 至今的 UTC 整天数。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RotationAge {
    /// 算得出:0 = 今天刚轮换。
    Days(u32),
    /// 算不出:没给轮换日 / 形状或日历日不认 / 轮换日在未来。**不拿 0 冒充**
    /// (规格用户故事 16:0 天是「今天刚换」,与「不知道」是两件事)。
    Unknown,
}

impl RotationAge {
    fn of(e: &CatalogEntryView, today: UtcDay) -> Self {
        let Some(r) = e.rotated_at.as_deref().and_then(UtcDay::parse) else {
            return Self::Unknown;
        };
        let days = today.epoch_days() - r.epoch_days();
        if days < 0 {
            // 轮换日在未来(时钟漂移或预登记):印负数没有意义,拿 0 冒充
            // 「今天刚换」更是假话——两类都归未知。
            return Self::Unknown;
        }
        // 现实中到不了(天数溢出 u32 等于一千多万年);真到了也读成 amber,
        // 比 panic 或悄悄截断成小数好。
        Self::Days(u32::try_from(days).unwrap_or(u32::MAX))
    }

    /// 「账龄 N 天」/「账龄 未知」——算不出就说算不出,不拿 0 占位。
    fn line(self) -> String {
        match self {
            Self::Days(n) => format!("账龄 {n} 天"),
            Self::Unknown => "账龄 未知".to_string(),
        }
    }

    /// 超阈(见 [`KEY_AGE_WARN_DAYS`]);「超」是严格大于,7 天本身不算超。
    /// 未知不算超:没有依据就不亮。
    fn over_threshold(self) -> bool {
        matches!(self, Self::Days(n) if n > KEY_AGE_WARN_DAYS)
    }
}

/// 钥匙册的条目明细:名字行后缀给 kind(条目级,恒 `github-key`),之下给
/// **主机 / 作用域 / 轮换日 + 账龄**(缺字段显式「——」)。**没有本体槽,也不印
/// 指纹**——投影里就没有这两格(上游 `KeySummary` 五键冻结),面板连「这像是
/// 哪把钥匙」都不该让人推断出来。作用域**原样逐条报**(只登记不设闸,面板不代管、
/// 不提醒、不评审;它们是登记元数据,与 secret 槽的「不印指纹」是两回事)。
///
/// 账龄那一列(#106):轮换日之后跟「账龄 N 天」,超 [`KEY_AGE_WARN_DAYS`] 天
/// 亮 amber、其余 dim。**只标记不治理**——不动退出码、不动 doctor 档位、不
/// 提醒、不自动轮换;册面其余读法逐字节不变。宽度上给它留位:超宽先砍日期
/// (截断,不折行),账龄文案一个字节不砍——砍了等于这一列白加。
///
/// 两档 golden 里的 345 / 271 天是**钉出来的**:测试经 `pin_today_for_test`
/// 把「今天」钉在 2026-10-13(见 `PINNED_TODAY`),所以这两帧不随墙钟漂。
fn key_detail_rows(e: &CatalogEntryView, width: usize, p: &Palette) -> EntryDetailOut {
    let kind = e
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .unwrap_or("——");
    let age = RotationAge::of(e, UtcDay::today());
    let age_line = age.line();
    let date_w = width
        .saturating_sub(12) // 字段标签的既有预算(与另外两行同一口径)
        .saturating_sub(unicode_width::UnicodeWidthStr::width(age_line.as_str()) + 2); // 两个空格 + 账龄文案
    (
        Some(format!("kind {kind}")),
        vec![
            Line::from(vec![
                Span::raw("    主机 "),
                Span::styled(
                    clip(e.host.as_deref().unwrap_or("——"), width.saturating_sub(12)),
                    p.text(),
                ),
            ]),
            Line::from(vec![
                Span::raw("    作用域 "),
                Span::styled(clip(&e.scopes_line(), width.saturating_sub(12)), p.text()),
            ]),
            Line::from(vec![
                Span::raw("    轮换日 "),
                Span::styled(clip(&e.rotated_at_line(), date_w), p.text()),
                Span::raw("  "),
                Span::styled(
                    age_line,
                    if age.over_threshold() {
                        p.warn()
                    } else {
                        p.dim()
                    },
                ),
            ]),
        ],
    )
}

/// 到期三态汇总(册体那一行「几条 active · 几条 expiring · 几条 expired」)。
/// 面板不自己算一套:三态与剩余天数由引擎在 ls 回音里按 UTC 现算给足,
/// 这里只**点数**;回音缺 `status` 的条数单独计数,不并进任何一态、不猜。
fn sub_status_counts(sheet: &CatalogSheet) -> SubStatusCounts {
    let mut c = SubStatusCounts::default();
    for e in &sheet.entries {
        match expiry_state(e.status.as_deref()) {
            State::Pass => c.active += 1,
            State::Warn => c.expiring += 1,
            State::Fail => c.expired += 1,
            _ => c.unknown += 1,
        }
    }
    c
}

/// 汇总行一句人话(视觉仍走结果四态:过 / 警 / 败)。`w` = 册体宽,说明行
/// 超宽显式截断(不靠终端隐式截断)。
fn sub_summary_lines(c: &SubStatusCounts, w: usize, p: &Palette) -> Vec<Line<'static>> {
    let seg = |n: usize, st: State| Span::styled(format!("{n} "), p.state(st).bold());
    let mut head = vec![
        Span::raw("  汇总 "),
        seg(c.active, State::Pass),
        Span::raw("active · "),
        seg(c.expiring, State::Warn),
        Span::raw("expiring · "),
        seg(c.expired, State::Fail),
        Span::raw("expired"),
    ];
    if c.unknown > 0 {
        head.push(Span::raw(" · "));
        head.push(Span::styled(
            format!("{} 条回音缺 status(显式「——」不猜)", c.unknown),
            p.warn(),
        ));
    }
    vec![
        Line::from(head),
        Line::from(Span::styled(
            clip(
                "  三态按 UTC 在读取时算、永不写进文件;只标记不治理(过期不改退出码)。",
                w,
            ),
            p.dim(),
        )),
    ]
}

/// 取回链接本体的**建议命令**那一行(ADR-0009 §1 裁定:面板不代跑
/// `subscription get`——`data.url` 是上游唯一回显本体的出口,屏显铺一条活
/// 订阅链接的扩散面面板兜不住)。执行权留给操作者,面板只给命令文案。
fn sub_get_lines(app: &App, w: usize, p: &Palette) -> Vec<Line<'static>> {
    match app.sub_get_suggest() {
        Some(s) => vec![
            Line::from(Span::styled(
                if app.sub_get_logged {
                    "  取回链接(面板不代跑,已按 g 收进日志):"
                } else {
                    "  取回链接(面板不代跑,按 g 收进日志):"
                },
                p.dim(),
            )),
            Line::from(Span::styled(clip(&s.display, w), p.accent().bold())),
            Line::from(Span::styled(
                clip(
                    "  get 是上游唯一回显本体的出口:铺在屏显上会进滚动历史/截图/快照帧,执行权留给操作者。",
                    w,
                ),
                p.dim(),
            )),
        ],
        None => vec![Line::from(Span::styled(
            "  取回链接:先选中一条订阅(↑/↓ 或鼠标点行);面板只给建议命令,不代跑 get。",
            p.dim(),
        ))],
    }
}

/// 订阅册的边界表述(ADR-0009 §5,工单 #56 验收第 7 条):订阅链接随续费轮换,
/// 烤进锁定集等于每次换链接重烤一次派——面板不提供任何这样的入口。
pub const SUB_BOUNDARY: &str =
    "  边界:订阅链接绝不进配方/锁定集/bundle/装配(续费换链接不触发重烤,不提供烤进去的入口)";

/// 钥匙册的边界表述(ADR-0009 §5,工单 #57 验收第 7 条):与订阅同一条纪律——
/// 钥匙是凭据,烤进锁定集 / bundle 等于把机密摊进派,面板不提供任何这样的入口。
pub const KEY_BOUNDARY: &str =
    "  边界:钥匙绝不进配方/锁定集/bundle/装配(凭据不随派分发,不提供烤进去的入口)";

/// 取回钥匙本体的**建议命令**那一行(ADR-0009 §1 裁定:面板不代跑 `key get`——
/// `data.key` 是上游唯一回显本体的出口,屏显铺一把活钥匙的扩散面面板兜不住)。
/// 执行权留给操作者,面板只给命令文案(与 bake / recipe new / subscription get
/// 同源同层,纯函数层产、永不带写旗标)。
fn key_get_lines(app: &App, w: usize, p: &Palette) -> Vec<Line<'static>> {
    match app.key_get_suggest() {
        Some(s) => vec![
            Line::from(Span::styled(
                if app.key_get_logged {
                    "  取回钥匙(面板不代跑,已按 g 收进日志):"
                } else {
                    "  取回钥匙(面板不代跑,按 g 收进日志):"
                },
                p.dim(),
            )),
            Line::from(Span::styled(clip(&s.display, w), p.accent().bold())),
            Line::from(Span::styled(
                clip(
                    "  get 是上游唯一回显本体的出口:铺在屏显上会进滚动历史/截图/快照帧,执行权留给操作者。",
                    w,
                ),
                p.dim(),
            )),
        ],
        None => vec![Line::from(Span::styled(
            "  取回钥匙:先选中一把(↑/↓ 或鼠标点行);面板只给建议命令,不代跑 get。",
            p.dim(),
        ))],
    }
}

// ---- 订阅册浮层(工单 #56:两段式登记 + rm 图形确认;本体只经 stdin) ----

/// 订阅册浮层总入口:按阶段画一个;Idle 无浮层(几何与命中共用同一
/// `popup_rect`)。与 vps 册同一条骨架([`RegPhase`]),差别全在呈现:
/// 表单多一个掩码喂入框、预审/落账两段各喂一次本体、结论屏没有链接槽。
fn render_sub_overlay(frame: &mut Frame, area: Rect, app: &App) {
    match app.sub_flow.phase {
        RegPhase::Idle => {}
        RegPhase::Form => render_sub_form(frame, area, app),
        RegPhase::PreAudit => render_sub_preaudit(frame, area, app),
        RegPhase::Review => render_sub_review(frame, area, app),
        RegPhase::Commit => render_sub_commit(frame, area, app),
        RegPhase::Done => render_sub_done(frame, area, app),
    }
}

/// ① 表单:名字 / 应用(client)/ 到期日 / **链接本体喂入框(掩码)**。
/// 当前字段带块光标;喂入框只显示「已喂入 N 字节」,字符一个都不渲染。
fn render_sub_form(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let f = &app.sub_form;
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  链接本体只经 stdin 进:在「链接」框贴一次,界面只显示已喂入字节数",
        p.dim(),
    ))];
    for field in SubField::ALL {
        let on = field == f.field;
        let text = if field.is_body() {
            f.body.mask()
        } else {
            let v = match field {
                SubField::Name => &f.name,
                SubField::Client => &f.client,
                SubField::Expires => &f.expires,
                SubField::Body => unreachable!("is_body 分支已接走"),
            };
            if on {
                format!("{v}█")
            } else {
                v.clone()
            }
        };
        let hint = match field {
            SubField::Expires => "  (YYYY-MM-DD,留空 = 永不过期)",
            SubField::Body => "  (掩码:本体不进渲染/日志/配置)",
            _ => "",
        };
        lines.push(Line::from(vec![
            Span::raw(format!("  {} ", field.label())),
            Span::styled(text, if on { p.sel().bold() } else { p.text() }),
            Span::styled(hint, p.dim()),
        ]));
    }
    lines.push(Line::from(Span::styled(
        "  ↑/↓ 移字段 · Enter 预审(四道闸,文件不落) · Esc/q 返航",
        p.dim(),
    )));
    let budget = popup.height.saturating_sub(3) as usize;
    lines.truncate(budget);
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记订阅 · 表单 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ② 预审进行中:四道闸在跑,文件不落;本体这一次性喂入即将随进程结束即弃。
fn render_sub_preaudit(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_INPUT_W, REG_INPUT_H);
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from(Span::styled("  正在预审:跑引擎的四道闸…", p.text())),
        Line::from(Span::styled(
            format!(
                "  链接本体已喂入 {} 字节(stdin 一次性,喂后即关即弃)。",
                app.sub_form.body.len()
            ),
            p.dim(),
        )),
        Line::from(Span::styled(
            "  不带 --yes:文件不落,拟落账记录只进回音。",
            p.dim(),
        )),
        Line::from(Span::styled("  Esc 可返航(回音将丢弃)。", p.dim())),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记订阅 · 预审 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ③ 确认态:摊开四道闸结论(按冻结 id,三态复用结果四态)+ 拟落账条目 +
/// 落账闸结论。**本段不下发任何写命令**——Enter 落账是下一段(且会再喂一次
/// stdin:喂入是一次性的)。投影里没有链接槽,这一屏也不出现本体。
fn render_sub_review(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let out = app.sub_flow.result.as_ref();
    let audit = out.and_then(SubscriptionSpec::audit);
    let mut lines: Vec<Line> = Vec::new();
    match audit.as_ref() {
        Some(a) => {
            lines.push(Line::from(Span::styled(
                format!(
                    "  编目  {}",
                    clip(&a.catalog_path, inner_w.saturating_sub(9))
                ),
                p.dim(),
            )));
            if let Some(e) = a.entry.as_ref() {
                lines.push(Line::from(format!(
                    "  拟落账  {} · 应用 {} · 到期 {}",
                    clip(&e.name, 16),
                    e.client.as_deref().unwrap_or("——"),
                    e.expires_line(),
                )));
            }
            lines.push(Line::from(Span::styled("  闸(恒 4 行):", p.accent())));
            for g in &a.gates {
                let st = g.state();
                lines.push(Line::from(vec![
                    Span::raw("   "),
                    Span::styled(gate_label(st), p.state(st)),
                    Span::raw(" "),
                    Span::styled(g.id.clone(), p.dim()),
                    Span::raw(" "),
                    Span::styled(
                        clip(&g.message, inner_w.saturating_sub(10 + g.id.len())),
                        p.text(),
                    ),
                ]));
            }
            if let Some(env) = out.and_then(|o| o.envelope.as_ref()) {
                for e in env.errors.iter().take(2) {
                    crate::screens::recipes::push_evidence(
                        &mut lines,
                        "出错",
                        p.fail(),
                        e,
                        inner_w,
                    );
                }
            }
            if preaudit_passed(out.unwrap(), Some(a)) {
                lines.push(Line::from(Span::styled(
                    "  预审通过:四道闸无 fail、退出码 0(本段不写任何文件)",
                    p.info(),
                )));
                lines.push(Line::from(Span::styled(
                    format!(
                        "  链接本体仍在途内存({} 字节,掩码):落账那一下会再喂一次 stdin。",
                        app.sub_form.body.len()
                    ),
                    p.dim(),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    "  ⚠ 不可落账:闸有 fail 或退出码非 0——见上方结论",
                    p.warn().bold(),
                )));
            }
        }
        None => {
            let headline = out.map(|o| o.headline().to_string()).unwrap_or_default();
            for ln in wrap(
                &format!("  预审结果不可定型:{headline}"),
                inner_w.saturating_sub(2),
            )
            .into_iter()
            .take(3)
            {
                lines.push(Line::from(Span::styled(ln, p.warn())));
            }
            lines.push(Line::from(Span::styled(
                "  ⚠ 不可落账:拿不到可定型的预审结论(不按旧形状解析)",
                p.warn().bold(),
            )));
        }
    }
    let budget = popup.height.saturating_sub(3) as usize;
    if lines.len() > budget {
        lines.truncate(budget.saturating_sub(1));
        lines.push(Line::from(Span::styled("  …(屏内未全显)", p.dim())));
    }
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    let can = out
        .map(|o| preaudit_passed(o, audit.as_ref()))
        .unwrap_or(false);
    lines.push(if can {
        Line::from(vec![
            Span::styled("  Enter 落账(再喂一次 stdin) ", p.accent().bold()),
            Span::raw("  "),
            Span::styled("Esc 返航 ", p.dim()),
        ])
    } else {
        Line::from(Span::styled("  Esc 返航 ", p.dim()))
    });
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记订阅 · 预审确认 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ④ 落账在途:带 `--yes` 的写命令已下发;登记与装配互斥。
fn render_sub_commit(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let root = app.usable_data_root().unwrap_or_else(|| "—".into());
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "  正在落账:引擎把 subscriptions.json 写进数据仓根…",
            p.text(),
        )),
        Line::from(Span::styled(
            format!(
                "  写入目标  {}",
                clip(
                    &format!("{root}/subscriptions.json"),
                    inner_w.saturating_sub(13)
                )
            ),
            p.dim(),
        )),
        Line::from(Span::styled(
            format!(
                "  链接本体已再喂入 {} 字节(stdin 一次性;界面不显示字符)。",
                app.sub_form.body.len()
            ),
            p.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  登记与装配互斥:写盘期间不接新装配。",
            p.dim(),
        )),
        Line::from(Span::styled(
            "  Esc 返航(回音将丢弃,互斥随回音收敛)",
            p.dim(),
        )),
    ];
    while lines.len() < popup.height.saturating_sub(3) as usize {
        lines.push(Line::from(""));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记订阅 · 落账 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ⑤ 结果屏:写盘位置 + 记录五字段(名字/应用/到期日,**没有链接槽**)+
/// git 变更提示 + 边界表述;失败路给证据五字段。
fn render_sub_done(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let out = app.sub_flow.result.as_ref();
    let mut lines: Vec<Line> = Vec::new();
    match out.and_then(SubscriptionSpec::audit) {
        Some(a) if out.map(|o| o.ok).unwrap_or(false) => {
            lines.push(Line::from(Span::styled(
                format!(
                    "  已写入  {}",
                    clip(&a.catalog_path, inner_w.saturating_sub(11))
                ),
                p.info().bold(),
            )));
            if let Some(e) = a.entry.as_ref() {
                lines.push(Line::from(format!(
                    "  订阅    {} · 应用 {} · 到期 {}",
                    clip(&e.name, 18),
                    e.client.as_deref().unwrap_or("——"),
                    e.expires_line(),
                )));
            }
            lines.push(Line::from(Span::styled(
                "  链接本体已从 stdin 收进 subscriptions.json(回显它只有 subscription get)。",
                p.dim(),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ⚠ subscriptions.json 是一次 git 变更:走 PR + 家长审,",
                p.warn().bold(),
            )));
            lines.push(Line::from(Span::styled(
                "    别把它当本地小改;订阅册已即时刷新。",
                p.warn(),
            )));
            lines.push(Line::from(Span::styled(
                clip(SUB_BOUNDARY, inner_w),
                p.dim(),
            )));
        }
        _ => {
            let headline = out.map(|o| o.headline().to_string()).unwrap_or_default();
            for ln in wrap(
                &format!("  落账未完成:{headline}"),
                inner_w.saturating_sub(2),
            )
            .into_iter()
            .take(2)
            {
                lines.push(Line::from(Span::styled(ln, p.fail().bold())));
            }
            lines.push(Line::from(Span::styled(
                "  未写入任何文件;Esc 返航后可从表单重来(链接要再喂一次)。",
                p.dim(),
            )));
        }
    }
    while lines.len() < popup.height.saturating_sub(3) as usize {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled("  Esc/q 返航 ", p.dim())));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记订阅 · 落账结果 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// 删除确认(ADR-0009 §2:上游 rm 没有 `--yes`,面板加一道图形确认——
/// 面板只决定「什么时候递出去」,下发的 argv 与 shell 里敲的逐字相同)。
/// 删不存在的名:引擎 rc 3 + 文件一个字节都不动。
fn render_sub_rm_confirm(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_INPUT_W, REG_INPUT_H);
    frame.render_widget(Clear, popup);
    let name = app.selected_sub_name().unwrap_or_else(|| "—".into());
    let root = app.usable_data_root().unwrap_or_else(|| "—".into());
    let lines = vec![
        Line::from(Span::styled(format!("  删除订阅 {name}?"), p.text().bold())),
        Line::from(Span::styled(
            format!("  下发  nw subscription rm {name} --root {root}"),
            p.dim(),
        )),
        Line::from(Span::styled(
            "  引擎没有确认闸:点名即删。要恢复得重新登记一遍(链接得再喂一次)。",
            p.warn(),
        )),
        Line::from(Span::styled(
            "  删不存在的名:引擎 rc 3 + subscriptions.json 一个字节都不动。",
            p.dim(),
        )),
        Line::from(Span::styled(
            "  Enter 确认删除 · Esc/q 返航",
            p.accent().bold(),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 删除订阅 · 确认 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

// ---- 钥匙册浮层(工单 #57:两段式登记 + rm 图形确认;本体只经 stdin) ----

/// 钥匙册浮层总入口:按阶段画一个;Idle 无浮层(几何与命中共用同一
/// `popup_rect`)。与订阅册同一条骨架([`RegPhase`]),差别全在呈现:表单多一个
/// 多行作用域字段与一个掩码本体喂入框、预审/落账两段各喂一次本体、结论屏没有
/// 本体槽也不印指纹。
fn render_key_overlay(frame: &mut Frame, area: Rect, app: &App) {
    match app.key_flow.phase {
        RegPhase::Idle => {}
        RegPhase::Form => render_key_form(frame, area, app),
        RegPhase::PreAudit => render_key_preaudit(frame, area, app),
        RegPhase::Review => render_key_review(frame, area, app),
        RegPhase::Commit => render_key_commit(frame, area, app),
        RegPhase::Done => render_key_done(frame, area, app),
    }
}

/// ① 表单:名字 / 主机 / 作用域(一行一条)/ 轮换日 / **钥匙本体喂入框(掩码)**。
/// 当前字段带块光标;喂入框只显示「已喂入 N 字节」,字符一个都不渲染。
fn render_key_form(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let f = &app.key_form;
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  钥匙本体只经 stdin 进:在「钥匙」框贴一次,界面只显示已喂入字节数;作用域/轮换日只登记不设闸",
        p.dim(),
    ))];
    for field in KeyField::ALL {
        let on = field == f.field;
        if field.is_body() {
            lines.push(Line::from(vec![
                Span::raw(format!("  {} ", field.label())),
                Span::styled(f.body.mask(), if on { p.sel().bold() } else { p.text() }),
                Span::styled("  (掩码:本体不进渲染/日志/配置)", p.dim()),
            ]));
            continue;
        }
        let rows: Vec<String> = match field {
            KeyField::Name => vec![f.name.clone()],
            KeyField::Host => vec![f.host.clone()],
            KeyField::RotatedAt => vec![f.rotated_at.clone()],
            KeyField::Scopes => f.scopes.clone(),
            KeyField::Body => unreachable!("is_body 分支已接走"),
        };
        let hint = match field {
            KeyField::Host => "  (留空 = github.com)",
            KeyField::RotatedAt => "  (YYYY-MM-DD,留空 = 登记日 UTC)",
            KeyField::Scopes => "  (一行一条,只登记不设闸)",
            _ => "",
        };
        for (r, row) in rows.iter().enumerate() {
            let cur = on && r == f.row.min(rows.len().saturating_sub(1));
            let text = if cur {
                format!("{row}█")
            } else {
                row.clone()
            };
            lines.push(Line::from(vec![
                Span::raw(if r == 0 {
                    format!("  {} ", field.label())
                } else {
                    "      ".to_string()
                }),
                Span::styled(text, if cur { p.sel().bold() } else { p.text() }),
                if r == 0 {
                    Span::styled(hint, p.dim())
                } else {
                    Span::raw("")
                },
            ]));
        }
    }
    lines.push(Line::from(Span::styled(
        "  ↑/↓ 移字段 · n 新行 · d 删行 · Enter 预审 · Esc/q 返航",
        p.dim(),
    )));
    let budget = popup.height.saturating_sub(3) as usize;
    lines.truncate(budget);
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记钥匙 · key 表单 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ② 预审进行中:三道闸在跑,文件不落;本体这一次性喂入即将随进程结束即弃。
fn render_key_preaudit(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_INPUT_W, REG_INPUT_H);
    frame.render_widget(Clear, popup);
    let lines = vec![
        Line::from(Span::styled("  正在预审:跑引擎的三道闸…", p.text())),
        Line::from(Span::styled(
            format!(
                "  钥匙本体已喂入 {} 字节(stdin 一次性,喂后即关即弃)。",
                app.key_form.body.len()
            ),
            p.dim(),
        )),
        Line::from(Span::styled(
            "  不带 --yes:文件不落,拟落账记录只进回音。",
            p.dim(),
        )),
        Line::from(Span::styled("  Esc 可返航(回音将丢弃)。", p.dim())),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记钥匙 · 预审 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ③ 确认态:摊开三道闸结论(按冻结 id,三态复用结果四态)+ 拟落账条目 +
/// 落账闸结论。**本段不下发任何写命令**——Enter 落账是下一段(且会再喂一次
/// stdin:喂入是一次性的)。投影里没有本体槽、不印指纹,这一屏也不出现本体。
fn render_key_review(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let out = app.key_flow.result.as_ref();
    let audit = out.and_then(KeySpec::audit);
    let mut lines: Vec<Line> = Vec::new();
    match audit.as_ref() {
        Some(a) => {
            lines.push(Line::from(Span::styled(
                format!(
                    "  编目  {}",
                    clip(&a.catalog_path, inner_w.saturating_sub(9))
                ),
                p.dim(),
            )));
            if let Some(e) = a.entry.as_ref() {
                lines.push(Line::from(format!(
                    "  拟落账  {} · 主机 {} · 轮换日 {}",
                    clip(&e.name, 16),
                    e.host.as_deref().unwrap_or("——"),
                    e.rotated_at_line()
                )));
                lines.push(Line::from(Span::styled(
                    format!("  作用域  {}(只登记,不设闸)", e.scopes_line()),
                    p.dim(),
                )));
            }
            lines.push(Line::from(Span::styled("  闸(恒 3 行):", p.accent())));
            for g in &a.gates {
                let st = g.state();
                lines.push(Line::from(vec![
                    Span::raw("   "),
                    Span::styled(gate_label(st), p.state(st)),
                    Span::raw(" "),
                    Span::styled(g.id.clone(), p.dim()),
                    Span::raw(" "),
                    Span::styled(
                        clip(&g.message, inner_w.saturating_sub(10 + g.id.len())),
                        p.text(),
                    ),
                ]));
            }
            if let Some(env) = out.and_then(|o| o.envelope.as_ref()) {
                for e in env.errors.iter().take(2) {
                    crate::screens::recipes::push_evidence(
                        &mut lines,
                        "出错",
                        p.fail(),
                        e,
                        inner_w,
                    );
                }
            }
            if preaudit_passed(out.unwrap(), Some(a)) {
                lines.push(Line::from(Span::styled(
                    "  预审通过:三道闸无 fail、退出码 0(本段不写任何文件)",
                    p.info(),
                )));
                lines.push(Line::from(Span::styled(
                    format!(
                        "  钥匙本体仍在途内存({} 字节,掩码):落账那一下会再喂一次 stdin。",
                        app.key_form.body.len()
                    ),
                    p.dim(),
                )));
            } else {
                lines.push(Line::from(Span::styled(
                    "  ⚠ 不可落账:闸有 fail 或退出码非 0——见上方结论",
                    p.warn().bold(),
                )));
            }
        }
        None => {
            let headline = out.map(|o| o.headline().to_string()).unwrap_or_default();
            for ln in wrap(
                &format!("  预审结果不可定型:{headline}"),
                inner_w.saturating_sub(2),
            )
            .into_iter()
            .take(3)
            {
                lines.push(Line::from(Span::styled(ln, p.warn())));
            }
            lines.push(Line::from(Span::styled(
                "  ⚠ 不可落账:拿不到可定型的预审结论(不按旧形状解析)",
                p.warn().bold(),
            )));
        }
    }
    let budget = popup.height.saturating_sub(3) as usize;
    if lines.len() > budget {
        lines.truncate(budget.saturating_sub(1));
        lines.push(Line::from(Span::styled("  …(屏内未全显)", p.dim())));
    }
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    let can = out
        .map(|o| preaudit_passed(o, audit.as_ref()))
        .unwrap_or(false);
    lines.push(if can {
        Line::from(vec![
            Span::styled("  Enter 落账(再喂一次 stdin) ", p.accent().bold()),
            Span::raw("  "),
            Span::styled("Esc 返航 ", p.dim()),
        ])
    } else {
        Line::from(Span::styled("  Esc 返航 ", p.dim()))
    });
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记钥匙 · 预审确认 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ④ 落账在途:带 `--yes` 的写命令已下发;登记与装配互斥。
fn render_key_commit(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let root = app.usable_data_root().unwrap_or_else(|| "—".into());
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "  正在落账:引擎把 keys.json 写进数据仓根…",
            p.text(),
        )),
        Line::from(Span::styled(
            format!(
                "  写入目标  {}",
                clip(&format!("{root}/keys.json"), inner_w.saturating_sub(13))
            ),
            p.dim(),
        )),
        Line::from(Span::styled(
            format!(
                "  钥匙本体已再喂入 {} 字节(stdin 一次性;界面不显示字符)。",
                app.key_form.body.len()
            ),
            p.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  登记与装配互斥:写盘期间不接新装配。",
            p.dim(),
        )),
        Line::from(Span::styled(
            "  Esc 返航(回音将丢弃,互斥随回音收敛)",
            p.dim(),
        )),
    ];
    while lines.len() < popup.height.saturating_sub(3) as usize {
        lines.push(Line::from(""));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记钥匙 · 落账 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ⑤ 结果屏:写盘位置 + 记录五字段(名字/主机/作用域条数/轮换日,**没有本体槽、
/// 不印指纹**)+ git 变更提示 + 边界表述;失败路给证据五字段。
fn render_key_done(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let out = app.key_flow.result.as_ref();
    let mut lines: Vec<Line> = Vec::new();
    match out.and_then(KeySpec::audit) {
        Some(a) if out.map(|o| o.ok).unwrap_or(false) => {
            lines.push(Line::from(Span::styled(
                format!(
                    "  已写入  {}",
                    clip(&a.catalog_path, inner_w.saturating_sub(11))
                ),
                p.info().bold(),
            )));
            if let Some(e) = a.entry.as_ref() {
                lines.push(Line::from(format!(
                    "  钥匙    {} · 主机 {} · 轮换日 {}",
                    clip(&e.name, 18),
                    e.host.as_deref().unwrap_or("——"),
                    e.rotated_at_line()
                )));
                lines.push(Line::from(Span::styled(
                    format!("  作用域  {}(只登记,不设闸)", e.scopes_line()),
                    p.dim(),
                )));
            }
            lines.push(Line::from(Span::styled(
                "  钥匙本体已从 stdin 收进 keys.json(回显它只有 key get)。",
                p.dim(),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ⚠ keys.json 是一次 git 变更:走 PR + 家长审,",
                p.warn().bold(),
            )));
            lines.push(Line::from(Span::styled(
                "    别把它当本地小改;钥匙册已即时刷新。",
                p.warn(),
            )));
            lines.push(Line::from(Span::styled(
                clip(KEY_BOUNDARY, inner_w),
                p.dim(),
            )));
        }
        _ => {
            let headline = out.map(|o| o.headline().to_string()).unwrap_or_default();
            for ln in wrap(
                &format!("  落账未完成:{headline}"),
                inner_w.saturating_sub(2),
            )
            .into_iter()
            .take(2)
            {
                lines.push(Line::from(Span::styled(ln, p.fail().bold())));
            }
            lines.push(Line::from(Span::styled(
                "  未写入任何文件;Esc 返航后可从表单重来(钥匙要再喂一次)。",
                p.dim(),
            )));
        }
    }
    while lines.len() < popup.height.saturating_sub(3) as usize {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled("  Esc/q 返航 ", p.dim())));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 登记钥匙 · 落账结果 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// 删除确认(ADR-0009 §2:面板加一道图形确认)。删不存在的名:引擎 rc 3 +
/// 文件一个字节都不动。
fn render_key_rm_confirm(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_INPUT_W, REG_INPUT_H);
    frame.render_widget(Clear, popup);
    let name = app.selected_key_name().unwrap_or_else(|| "—".into());
    let root = app.usable_data_root().unwrap_or_else(|| "—".into());
    let lines = vec![
        Line::from(Span::styled(format!("  删除钥匙 {name}?"), p.text().bold())),
        Line::from(Span::styled(
            format!("  下发  nw key rm {name} --root {root}"),
            p.dim(),
        )),
        Line::from(Span::styled(
            "  引擎没有确认闸:点名即删。要恢复得重新登记一遍(钥匙得再喂一次)。",
            p.warn(),
        )),
        Line::from(Span::styled(
            "  删不存在的名:引擎 rc 3 + keys.json 一个字节都不动。",
            p.dim(),
        )),
        Line::from(Span::styled(
            "  Enter 确认删除 · Esc/q 返航",
            p.accent().bold(),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 删除钥匙 · 确认 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// 闸三态的呈现词(复用「结果四态」的同一套词,不新造)。
fn gate_label(st: State) -> &'static str {
    match st {
        State::Fail => "✗",
        State::Warn => "警",
        State::Pass => "✓",
        _ => "·",
    }
}

// ---- 标签册浮层(工单 #54:挂标签 add / 摘引用删标签 rm) ----

/// 标签册浮层总入口:按阶段画一个;Idle 无浮层(几何与命中区同一
/// `popup_rect`)。
///
/// 阶段用的是骨架那一个 [`RegPhase`],但 tag 只走其中三条:`Form` →
/// `Commit`(写命令已下发)→ `Done`(回音摊开闸结论)/ `Review`(闸有 fail,
/// 一个字节都没落)。**没有 `PreAudit`**——上游 `tag add` 没有 `--yes`,
/// 不存在「文件不落」的那一段(ADR-0009 §2,详见 [`TagSpec`] 文档)。
fn render_tag_overlay(frame: &mut Frame, area: Rect, app: &App) {
    match app.tag_flow.phase {
        RegPhase::Idle | RegPhase::PreAudit => {}
        RegPhase::Form => render_tag_form(frame, area, app),
        RegPhase::Commit => render_tag_commit(frame, area, app),
        RegPhase::Review => render_tag_review(frame, area, app),
        RegPhase::Done => render_tag_done(frame, area, app),
    }
}

/// ① 标签册表单:标签名 + 若干条 `kind/条目` 引用(一行一条)。
/// 挂标签与摘引用/删标签共用,标题按用途分。
fn render_tag_form(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let f = &app.tag_form;
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        "  一行一条:<kind>/<条目>(skill 的 id = <仓库>/<条目>)",
        p.dim(),
    ))];
    for field in TagField::ALL {
        let on = field == f.field;
        let rows: Vec<String> = match field {
            TagField::Name => vec![f.name.clone()],
            TagField::Refs => f.refs.clone(),
        };
        for (r, row) in rows.iter().enumerate() {
            let cur = on && r == f.row.min(rows.len().saturating_sub(1));
            let text = if cur {
                format!("{row}█")
            } else {
                row.clone()
            };
            lines.push(Line::from(vec![
                Span::raw(if r == 0 {
                    format!("  {} ", field.label())
                } else {
                    "      ".to_string()
                }),
                Span::styled(text, if cur { p.sel().bold() } else { p.text() }),
            ]));
        }
    }
    lines.push(Line::from(Span::styled(
        "  ↑/↓ 移字段 · n 新行 · d 删行",
        p.dim(),
    )));
    lines.push(Line::from(Span::styled(
        match f.mode {
            // add 无确认闸(ADR-0009 §2):Enter 就是下发写命令那一下
            TagFormMode::Add => "  Enter 挂上去(无确认闸:追加合并可重放) · Esc/q 返航",
            // rm 上游也没有 --yes,面板加一道图形确认
            TagFormMode::Rm => "  Enter 去确认 · Esc/q 返航",
        },
        p.dim(),
    )));
    let budget = popup.height.saturating_sub(3) as usize;
    lines.truncate(budget);
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    let title = match f.mode {
        TagFormMode::Add => " 挂标签 · tag 表单 ",
        TagFormMode::Rm => " 摘引用 / 删标签 · tag 表单 ",
    };
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

/// ② 写在途:`tag add` / `tag rm` 已下发(它就是写命令,没有第二段)。
fn render_tag_commit(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let root = app.usable_data_root().unwrap_or_else(|| "—".into());
    let (verb, what) = match app.tag_form.mode {
        TagFormMode::Add => ("挂标签", "tag add"),
        TagFormMode::Rm => ("摘引用 / 删标签", "tag rm"),
    };
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            format!("  正在{verb}:引擎把 tags.json 写进数据仓根…"),
            p.text(),
        )),
        Line::from(Span::styled(
            format!(
                "  下发  {} --root {}",
                what,
                clip(&root, inner_w.saturating_sub(13))
            ),
            p.dim(),
        )),
        Line::from(Span::styled(
            "  上游没有 --yes:这一条 argv 本身就是写命令(追加合并、幂等)。",
            p.dim(),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "  登记与装配互斥:写盘期间不接新装配。",
            p.dim(),
        )),
        Line::from(Span::styled(
            "  Esc 返航(回音将丢弃,互斥随回音收敛)",
            p.dim(),
        )),
    ];
    while lines.len() < popup.height.saturating_sub(3) as usize {
        lines.push(Line::from(""));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(format!(" {verb} · 下发 "), p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ③ 结论屏:四道闸摊开 + 悬空引用清理明示 + 记录五字段。
/// tag add **没有落账段**(无 `--yes`、无面板确认),所以这一屏即终态。
fn render_tag_done(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let out = app.tag_flow.result.as_ref();
    let mut lines: Vec<Line> = Vec::new();
    if out.map(|o| o.ok).unwrap_or(false) {
        lines.push(Line::from(Span::styled(
            "  已写入  tags.json(追加合并;同一条命令重跑,字节不变)",
            p.info().bold(),
        )));
        lines.extend(tag_audit_lines(out, inner_w, p));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  ⚠ tags.json 是一次 git 变更:走 PR + 家长审,",
            p.warn().bold(),
        )));
        lines.push(Line::from(Span::styled(
            "    别把它当本地小改;标签册已即时刷新。",
            p.warn(),
        )));
        lines.push(Line::from(Span::styled(
            clip(TAG_BOUNDARY, inner_w),
            p.dim(),
        )));
    } else {
        // 落账段失败路:留在结论屏把话说清(闸随失败结论关闭)
        lines.extend(tag_audit_lines(out, inner_w, p));
    }
    let budget = popup.height.saturating_sub(3) as usize;
    if lines.len() > budget {
        lines.truncate(budget.saturating_sub(1));
        lines.push(Line::from(Span::styled("  …(屏内未全显)", p.dim())));
    }
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    lines.push(Line::from(Span::styled("  Esc/q 返航 ", p.dim())));
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 挂标签 · 结论 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// ④ 闸有 fail 的结论屏:一个字节都没落 + 四道闸原文 + 证据五字段。
/// **不给「Enter 落账」段**:tag add 没有引擎确认闸,面板也不自己造一道
/// (ADR-0009 §2;`TagSpec::confirm_gate()` = false)。
fn render_tag_review(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_W, REG_H);
    frame.render_widget(Clear, popup);
    let inner_w = popup.width.saturating_sub(2) as usize;
    let out = app.tag_flow.result.as_ref();
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            "  ⚠ 没挂成:tags.json 一个字节都没落",
            p.warn().bold(),
        )),
        Line::from(Span::styled(
            "  四道闸有 fail 或退出码非 0;改好表单可重下同一条 argv。",
            p.dim(),
        )),
    ];
    lines.extend(tag_audit_lines(out, inner_w, p));
    let budget = popup.height.saturating_sub(3) as usize;
    if lines.len() > budget {
        lines.truncate(budget.saturating_sub(1));
        lines.push(Line::from(Span::styled("  …(屏内未全显)", p.dim())));
    }
    while lines.len() < budget {
        lines.push(Line::from(""));
    }
    lines.push(if TagSpec::confirm_gate() {
        Line::from(vec![
            Span::styled("  Enter 落账 ", p.accent().bold()),
            Span::raw("  "),
            Span::styled("Esc 返航 ", p.dim()),
        ])
    } else {
        Line::from(Span::styled(
            "  Esc/q 返航(无确认闸:追加合并、幂等可重放,点错重跑同一 argv 即回到同一态)",
            p.dim(),
        ))
    });
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 挂标签 · 闸结论 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

/// 摊开 tag add 的回音:编目路径 + 标签与引用 + **四道闸**(恒 4 行,id
/// 冻结、三态复用结果四态)+ **悬空引用清理条数**(不静默)+ 证据五字段。
/// 结论屏与失败屏共用这一处,不写第二份。
fn tag_audit_lines(out: Option<&EngineOutput>, inner_w: usize, p: &Palette) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    let data = out
        .and_then(|o| o.envelope.as_ref())
        .and_then(|e| e.data_as::<TagAddData>());
    match out.and_then(TagSpec::audit) {
        Some(a) => {
            lines.push(Line::from(Span::styled(
                format!(
                    "  编目  {}",
                    clip(&a.catalog_path, inner_w.saturating_sub(9))
                ),
                p.dim(),
            )));
            if let Some(e) = a.entry.as_ref() {
                lines.push(Line::from(format!("  标签    {}", clip(&e.name, 18))));
                lines.push(Line::from(vec![
                    Span::raw("  引用    "),
                    Span::styled(clip(&e.ref_line(), inner_w.saturating_sub(11)), p.text()),
                ]));
            }
            lines.push(Line::from(Span::styled("  闸(恒 4 行):", p.accent())));
            for g in &a.gates {
                let st = g.state();
                lines.push(Line::from(vec![
                    Span::raw("   "),
                    Span::styled(gate_label(st), p.state(st)),
                    Span::raw(" "),
                    Span::styled(g.id.clone(), p.dim()),
                    Span::raw(" "),
                    Span::styled(
                        clip(&g.message, inner_w.saturating_sub(10 + g.id.len())),
                        p.text(),
                    ),
                ]));
            }
            // 悬空引用清理:引擎静默清的,界面明示清了几条(工单 #54 验收第 4 条)
            match data.as_ref() {
                Some(d) if d.cleaned_count() > 0 => {
                    lines.push(Line::from(Span::styled(
                        format!("  ⚠ 悬空引用已清 {} 条(指向已消失条目):", d.cleaned_count()),
                        p.warn().bold(),
                    )));
                    lines.push(Line::from(Span::styled(
                        format!(
                            "    {}",
                            clip(
                                &d.cleaned
                                    .iter()
                                    .map(CatalogRef::label)
                                    .collect::<Vec<_>>()
                                    .join(" · "),
                                inner_w.saturating_sub(6)
                            )
                        ),
                        p.warn(),
                    )));
                }
                Some(_) => lines.push(Line::from(Span::styled(
                    "  悬空引用清理:0 条(没有指向已消失条目的引用)",
                    p.dim(),
                ))),
                None => {}
            }
            if let Some(env) = out.and_then(|o| o.envelope.as_ref()) {
                for e in env.errors.iter().take(2) {
                    crate::screens::recipes::push_evidence(&mut lines, "出错", p.fail(), e, inner_w)
                }
            }
        }
        None => {
            let headline = out.map(|o| o.headline().to_string()).unwrap_or_default();
            for ln in wrap(
                &format!("  回音不可定型:{headline}"),
                inner_w.saturating_sub(2),
            )
            .into_iter()
            .take(3)
            {
                lines.push(Line::from(Span::styled(ln, p.warn())));
            }
            lines.push(Line::from(Span::styled(
                "  ⚠ 不可定型:拿不到闸结论(不按旧形状解析,evidence 见上方)",
                p.warn().bold(),
            )));
        }
    }
    lines
}

/// 摘引用 / 删标签确认(ADR-0009 §2:上游 rm 没有 `--yes`,面板加一道图形
/// 确认——面板只决定「什么时候递出去」,下发的 argv 与 shell 里敲的逐字
/// 相同)。两态在确认框里说清:带引用只摘那几条,不带删整个标签。
fn render_tag_rm_confirm(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let popup = popup_rect(area, REG_INPUT_W, REG_INPUT_H);
    frame.render_widget(Clear, popup);
    let name = app.selected_tag_name().unwrap_or_else(|| "—".into());
    let root = app.usable_data_root().unwrap_or_else(|| "—".into());
    let refs = app
        .tag_form
        .refs
        .iter()
        .filter(|r| !r.trim().is_empty())
        .count();
    let (what, cmd) = if refs == 0 {
        (
            format!("删整个标签 {name}?"),
            format!("nw tag rm {name} --root {root}"),
        )
    } else {
        (
            format!("从 {name} 摘掉 {refs} 条引用?"),
            format!("nw tag rm {name} --ref …(--root {root})"),
        )
    };
    let lines = vec![
        Line::from(Span::styled(format!("  {what}"), p.text().bold())),
        Line::from(Span::styled(format!("  下发  {cmd}"), p.dim())),
        Line::from(Span::styled(
            "  带引用只摘那几条(action ref_removed,标签还在);不带删整个标签。",
            p.dim(),
        )),
        Line::from(Span::styled(
            "  删不存在的标签:引擎 rc 3 + tags.json 一个字节不动。",
            p.warn(),
        )),
        Line::from(Span::styled(
            "  Enter 确认删除 · Esc/q 返航",
            p.accent().bold(),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .border_style(p.accent())
                    .title(Span::styled(" 摘引用 / 删标签 · 确认 ", p.accent().bold())),
            )
            .style(p.surface()),
        popup,
    );
}

// ---- 登记流的键位(命令组无关的段 + vps 表单段) ----

/// 登记表单段键位:↑/↓ 移字段、n 新行、d 删行、Enter 预审、Esc/q 返航。
/// 键入分发归通用态机 [`input::on_key`](与 NW_HOME / 数据仓根手填同一条)。
pub fn on_key_form(form: &mut VpsForm, k: crossterm::event::KeyEvent) -> FormAction {
    match k.code {
        KeyCode::Up => {
            form.move_field(false);
            return FormAction::Edited;
        }
        KeyCode::Down => {
            form.move_field(true);
            return FormAction::Edited;
        }
        KeyCode::Char('n') if form.field.multi() => {
            form.add_row();
            return FormAction::Edited;
        }
        KeyCode::Char('d') if form.field.multi() && form.rows() > 1 => {
            form.drop_row();
            return FormAction::Edited;
        }
        _ => {}
    }
    match input::on_key(form.buf(), k) {
        InputAction::Submit => FormAction::Submit,
        InputAction::Cancel => FormAction::Cancel,
        InputAction::Edited | InputAction::Ignored => FormAction::Edited,
    }
}

/// 表单按键的后果(与 [`input::InputAction`] 同形,多两个表单动作)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormAction {
    Submit,
    Cancel,
    Edited,
}

/// vps 登记的发起规格由表单算出(供 `App` 下发两段用)。
pub fn spec_from_form(form: &VpsForm) -> Result<VpsSpec, Vec<String>> {
    let (spec, bad) = VpsSpec::from_form(form);
    if spec.name.is_empty() {
        return Err(vec!["名字为空".into()]);
    }
    if !bad.is_empty() {
        return Err(bad);
    }
    Ok(spec)
}

/// 新建一张空表单(供 `App` 发起登记用)。
pub fn new_form() -> VpsForm {
    VpsForm::new()
}

// ---- 标签册表单(工单 #54:标签名 + 若干条 <kind>/<条目> 引用) ----

/// 标签册表单的两个字段(顺序即 ↑/↓ 轮转顺序)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TagField {
    #[default]
    Name,
    Refs,
}

impl TagField {
    pub const ALL: [TagField; 2] = [TagField::Name, TagField::Refs];

    /// 字段标签(呈现与键位提示同源)。
    pub fn label(self) -> &'static str {
        match self {
            TagField::Name => "标签",
            TagField::Refs => "引用",
        }
    }

    /// 多行字段(一行一条)——单行字段给 false。
    pub fn multi(self) -> bool {
        matches!(self, TagField::Refs)
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }
}

/// 标签册表单的两种用途:**挂标签**(add,无确认闸)与**摘引用 / 删标签**
/// (rm,面板加一道图形确认)。两者共用同一个表单形状——差别只在
/// 「引用行为空 = 删整个标签」,以及 rm 多那道面板侧的确认(ADR-0009 §2)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TagFormMode {
    /// 挂标签:表单 Enter 直接下发写命令(上游没有 `--yes`,面板不造一道)。
    #[default]
    Add,
    /// 摘引用 / 删标签:表单 Enter 进图形确认,确认后才下发。
    Rm,
}

/// 标签册表单:标签名 + 若干条 `<kind>/<条目>` 引用(一行一条)。
/// 与 [`VpsForm`] 同构(多行字段收成「一行一条」的字符串行,可测),但
/// **不复制第二份表单机制**——字段更少(两个),只借 [`input::on_key`]
/// 这一条通用键入分发。
#[derive(Clone, Default)]
pub struct TagForm {
    pub name: String,
    /// `kind/条目` 一行一条(空行 = 没填这条,不是坏行)。
    pub refs: Vec<String>,
    pub field: TagField,
    /// 多行字段里的当前行(单行字段恒 0)。
    pub row: usize,
    pub mode: TagFormMode,
}

impl TagForm {
    fn new() -> Self {
        Self {
            refs: vec![String::new()],
            ..Self::default()
        }
    }

    /// 当前字段的缓冲(单行字段借字段本身,多行字段借当前行)。
    fn buf(&mut self) -> &mut String {
        match self.field {
            TagField::Name => &mut self.name,
            TagField::Refs => {
                while self.row >= self.refs.len() {
                    self.refs.push(String::new());
                }
                &mut self.refs[self.row]
            }
        }
    }

    fn rows(&self) -> usize {
        if self.field.multi() {
            self.refs.len()
        } else {
            1
        }
    }

    /// ↑/↓ 移字段(两行轮转,到顶/到底不越界、不绕回)。
    fn move_field(&mut self, down: bool) {
        let i = self.field.index();
        let n = TagField::ALL.len();
        self.field = TagField::ALL[if down {
            (i + 1).min(n - 1)
        } else {
            i.saturating_sub(1)
        }];
        self.row = 0;
    }

    /// 引用字段新行(追加一条空行,行光标跟上)。
    fn add_row(&mut self) {
        if self.field.multi() {
            self.refs.push(String::new());
            self.row = self.refs.len() - 1;
        }
    }

    /// 删当前行(至少留一行——表单不塌成空)。
    fn drop_row(&mut self) {
        if self.field.multi() && self.refs.len() > 1 {
            self.refs.remove(self.row);
            self.row = self.row.min(self.refs.len() - 1);
        }
    }
}

/// 标签册表单段键位:↑/↓ 移字段、n 新行、d 删行、Enter 下发、Esc/q 返航。
/// 键入分发归通用态机 [`input::on_key`](与 vps 表单 / 两份手填同一条)。
pub fn on_key_tag_form(form: &mut TagForm, k: crossterm::event::KeyEvent) -> FormAction {
    match k.code {
        KeyCode::Up => {
            form.move_field(false);
            return FormAction::Edited;
        }
        KeyCode::Down => {
            form.move_field(true);
            return FormAction::Edited;
        }
        KeyCode::Char('n') if form.field.multi() => {
            form.add_row();
            return FormAction::Edited;
        }
        KeyCode::Char('d') if form.field.multi() && form.rows() > 1 => {
            form.drop_row();
            return FormAction::Edited;
        }
        _ => {}
    }
    match input::on_key(form.buf(), k) {
        InputAction::Submit => FormAction::Submit,
        InputAction::Cancel => FormAction::Cancel,
        InputAction::Edited | InputAction::Ignored => FormAction::Edited,
    }
}

/// 标签册的发起规格由表单算出(供 `App` 下发用)。
pub fn spec_from_tag_form(form: &TagForm) -> Result<TagSpec, Vec<String>> {
    let (spec, bad) = TagSpec::from_form(form);
    if spec.name.is_empty() {
        return Err(vec!["标签名为空".into()]);
    }
    if !bad.is_empty() {
        return Err(bad);
    }
    Ok(spec)
}

/// 新建一张空标签表单(供 `App` 发起挂标签 / 摘引用用)。
pub fn new_tag_form(mode: TagFormMode) -> TagForm {
    TagForm {
        mode,
        ..TagForm::new()
    }
}

/// 编目册登记浮层的宽高(渲染与命中共用,同 `layout::REG_*` 的法子):
/// 表单/确认/结果三态同一基准不跳变,预审与删除确认用输入档尺寸。
pub const REG_W: u16 = 80;
pub const REG_H: u16 = 17;

/// 反查上浮结果浮层的宽(#70):一层**薄浮层**(不是新屏、不是新册),与
/// 登记浮层同一个 [`popup_rect`] 基准;高按命中数现算(见
/// [`vps_reverse_popup_rect`]),渲染与命中共用这一处几何——点哪打哪
/// (ADR-0005)。
pub const REVERSE_W: u16 = 56;

/// 编目册的删除确认键位行几何(渲染与命中共用)。
pub fn rm_confirm_rect(area: Rect) -> Rect {
    popup_rect(area, REG_INPUT_W, REG_INPUT_H)
}

/// 选中机器的名字(供删除确认与键位行用);没有选中给 None。
pub fn selected_name(sheet: &VpsLsData, sel: usize) -> Option<String> {
    sheet.machines.get(sel).map(|m| m.name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Screen;
    use crossterm::event::{KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    /// 常畅态 / 紧凑态两档都成立(#53 验收):不引新断点、不缩放字号,
    /// 66×50 紧凑态下也不溢出、不 panic,键位行常在。
    #[test]
    fn 编目册屏_常畅与紧凑两档都渲染不溢出() {
        // 夹具是 **data 面**(VpsLsData 的形状),不是整份 envelope——
        // 整份 envelope 喂进来会被 serde 静默忽略成空册,那正是「看起来
        // 正常其实什么都没解析」的坑。
        let ls = r#"{"catalogPath":"/r/vps.json","machines":[{"name":"gate","ips":[{"label":"home","addr":"10.0.0.1"}],"port":22,"logins":[{"user":"root","ref":"$DEPLOY_KEY"}],"note":"门口"},{"name":"bare","ips":[],"port":0,"logins":[]}]}"#;
        let mut app = crate::app::App::with_runner(std::sync::Arc::new(
            crate::engine::FakeRunner::new(&[("vps ls", 0, ls)]),
        ));
        app.splash = false;
        app.screen = crate::app::Screen::Vps;
        app.capabilities.record(
            crate::engine::Capability::Vps,
            crate::engine::Probe::Available,
        );
        app.vps_ls = Some(serde_json::from_str(ls).unwrap());
        for (w, h) in [(80u16, 30u16), (66, 50), (100, 40)] {
            let s = crate::testutil::frame_display(&app, w, h);
            assert!(s.contains("编目"), "{w}×{h} 册体应在场");
            assert!(s.contains("gate"), "{w}×{h} 机器名应在场");
            // 键位行钉在册体末行(两档同文案);#70 起机器册这一行还带可点的
            // 「Enter 反查」段(只机器册有,见 `catalog_key_parts`)
            assert!(s.contains("Enter 反查"), "{w}×{h} 键位行应在场:{s}");
            // 不溢出:每行显示宽不超终端宽
            for ln in s.lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 有行超宽:{ln}"
                );
            }
        }
    }

    fn press(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    const VPS_REG_OK: &str = r#"{"schema":"nw/vps-register.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"name":"gate","ips":[{"label":"home","addr":"10.0.0.1"}],"port":22,"logins":[{"user":"root","ref":"$DEPLOY_KEY"}],"catalogPath":"/r/vps.json","written":false,"updated":false,"action":"registered","gates":[{"id":"register.name","status":"pass","message":"名字认"},{"id":"register.ips","status":"pass","message":"地址认"},{"id":"register.port","status":"pass","message":"端口认"},{"id":"register.logins","status":"pass","message":"登录认"},{"id":"register.secret_guard","status":"pass","message":"ref 不像密钥本体"}]}}"#;

    async fn vps_register_out(json: &str) -> EngineOutput {
        let fake = crate::engine::FakeRunner::new(&[("vps register", 0, json)]);
        crate::engine::CommandRunner::run(&fake, &["vps".into(), "register".into()]).await
    }

    #[tokio::test]
    async fn 两段式登记流_预审落账返航的状态迁移() {
        let mut f = RegisterFlow::new();
        assert_eq!(f.phase, RegPhase::Idle);
        f.begin();
        assert_eq!(f.phase, RegPhase::Form);
        f.preaudit_sent();
        assert_eq!(f.phase, RegPhase::PreAudit);
        // 等回音态的预审回音进确认态
        assert!(f.preaudit_back(vps_register_out(VPS_REG_OK).await));
        assert_eq!(f.phase, RegPhase::Review);
        // 已返航的迟到回音不收敛(不幽灵重开)
        f.abandon();
        assert_eq!(f.phase, RegPhase::Idle);
        assert!(f.result.is_none(), "返航不留半条回音");
        assert!(f.commit_back(vps_register_out(VPS_REG_OK).await).is_none());
    }

    #[test]
    fn 表单_多行地址与登录_一行一条且可增删() {
        let mut form = VpsForm::new();
        on_key_form(&mut form, press('g'));
        on_key_form(&mut form, press('a'));
        assert_eq!(form.name, "ga");
        // 地址字段:一行一条
        form.field = VpsField::Ips;
        on_key_form(&mut form, press('h'));
        on_key_form(&mut form, press('o'));
        on_key_form(&mut form, press('m'));
        on_key_form(&mut form, press('e'));
        on_key_form(&mut form, press('='));
        on_key_form(&mut form, press('1'));
        assert_eq!(form.ips, vec!["home=1".to_string()]);
        on_key_form(&mut form, press('n')); // 新行
        assert_eq!(form.ips.len(), 2);
        on_key_form(&mut form, press('d')); // 删当前行(至少留一行)
        assert_eq!(form.ips.len(), 1);
        // ↑/↓ 移字段:到顶停顶、到底停底,不绕回
        form.field = VpsField::Name;
        on_key_form(&mut form, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(form.field, VpsField::Name, "到顶不越界");
        on_key_form(&mut form, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(form.field, VpsField::Ips);
    }

    #[test]
    fn 表单解析_空一侧的行被拒_不喂给引擎猜() {
        let mut form = VpsForm::new();
        form.name = "gate".into();
        form.ips = vec!["home=10.0.0.1".into(), "=坏行".into()];
        form.logins = vec!["root=$DEPLOY_KEY".into()];
        form.port = "22".into();
        let (spec, bad) = VpsSpec::from_form(&form);
        assert_eq!(spec.ips.len(), 1, "坏行剔除");
        assert_eq!(bad.len(), 1, "坏行给证据");
        assert_eq!(spec.port, Some(22));
        assert_eq!(
            spec.logins[0],
            ("root".to_string(), "$DEPLOY_KEY".to_string())
        );
        // 规格的两段 argv:预审无 --yes,落账恰好一枚
        let pre = spec.preaudit("/r");
        let com = spec.commit("/r");
        assert!(!pre.iter().any(|s| s == "--yes"));
        assert_eq!(com, [pre.clone(), vec!["--yes".into()]].concat());
        assert!(pre.iter().any(|s| s == "--ip"), "多组 --ip 按序落槽");
        assert!(!pre.iter().any(|s| s == "--repo"), "vps 组只认 --root");
    }

    #[tokio::test]
    async fn 落账闸_退出码0且闸无fail才放行() {
        let out = vps_register_out(VPS_REG_OK).await;
        let audit = VpsSpec::audit(&out);
        assert!(audit.is_some(), "回音可定型");
        assert!(preaudit_passed(&out, audit.as_ref()));
        // 不可定型(契约守卫需升级)= 不放行,不按旧形状降级
        let blind = vps_register_out(
            r#"{"schema":"nw/vps-register.v2","nw":"9.9.9","ok":true,"errors":[],"warnings":[],"data":{}}"#,
        )
        .await;
        assert!(VpsSpec::audit(&blind).is_none());
        assert!(!preaudit_passed(&blind, None));
    }

    #[test]
    fn 列表件_缺字段给破折号_摘要不印ref与addr() {
        let sheet = CatalogSheet {
            catalog_path: "/r/vps.json".into(),
            entries: vec![CatalogEntryView {
                name: "bare".into(),
                refs: Vec::new(),
                ..Default::default()
            }],
            gates: Vec::new(),
        };
        let mut term = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let p = crate::theme::cached_degrade(
            crate::theme::ThemeKind::Warehouse,
            crate::term::ColorCap::Truecolor,
        );
        let lines = catalog_sheet_lines(
            &sheet,
            &(0..sheet.entries.len()).collect::<Vec<_>>(),
            60,
            0,
            &p,
            vps_detail_rows,
        );
        let text: String = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("bare"), "名字在场:{text}");
        assert!(text.contains("——"), "缺字段显式「——」:{text}");
        assert!(!text.contains('$'), "ref 不进摘要:{text}");
        assert!(!text.contains("10.0"), "addr 不进摘要:{text}");
        let _ = &mut term;
    }

    // ---- 工单 #54:标签册(反查两态 / add 无确认闸 / rm 两态 / 悬空清理明示) ----

    const TAG_LS_ALL: &str = r#"{"schema":"nw/tag-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/tags.json","tags":[{"name":"pi","refs":[{"kind":"skill","id":"o/r/deep-research"},{"kind":"vps","id":"gate"}]},{"name":"bare","refs":[]}]}}"#;

    const TAG_ADD_OK: &str = r#"{"schema":"nw/tag-add.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"name":"pi","catalogPath":"/r/tags.json","written":true,"updated":false,"action":"added","refs":[{"kind":"skill","id":"o/r/deep-research"}],"cleaned":[],"gates":[{"id":"tag.add.name","status":"pass","message":"标签认"},{"id":"tag.add.ref-kind","status":"pass","message":"kind 认"},{"id":"tag.add.ref-id","status":"pass","message":"条目 id 认"},{"id":"tag.add.ref-exists","status":"pass","message":"对端编目里有"}]}}"#;

    /// 标签册失败回音:第③道闸(ref-id)不过 → 引擎不写文件。
    const TAG_ADD_FAIL: &str = r#"{"schema":"nw/tag-add.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"第③道闸没过:--ref 的 id 是空的","hint":"id 要指向具体条目:--ref vps/<机器名> / --ref skill/<仓库>/<条目>","details":{"gate":"tag.add.ref-id"}}],"warnings":[],"data":{"name":"pi","catalogPath":"/r/tags.json","written":false,"updated":false,"action":"rejected","refs":[],"cleaned":[],"gates":[{"id":"tag.add.name","status":"pass","message":"标签认"},{"id":"tag.add.ref-kind","status":"pass","message":"kind 认"},{"id":"tag.add.ref-id","status":"fail","message":"id 是空的"},{"id":"tag.add.ref-exists","status":"skip","message":"前面已拒"}]}}"#;

    /// 标签册成功回音,带 2 条悬空引用清理。
    const TAG_ADD_CLEANED: &str = r#"{"schema":"nw/tag-add.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"name":"pi","catalogPath":"/r/tags.json","written":true,"updated":false,"action":"added","refs":[{"kind":"skill","id":"o/r/deep-research"}],"cleaned":[{"kind":"extension","id":"gone-pack"},{"kind":"vps","id":"ghost"}],"gates":[{"id":"tag.add.name","status":"pass","message":"标签认"},{"id":"tag.add.ref-kind","status":"pass","message":"kind 认"},{"id":"tag.add.ref-id","status":"pass","message":"条目 id 认"},{"id":"tag.add.ref-exists","status":"pass","message":"对端编目里有"}]}}"#;

    /// 反查一条不存在的标签:rc 3 + 证据(上游口径:问错了名字,不是空结果)。
    const TAG_LS_MISS: &str = r#"{"schema":"nw/tag-ls.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"标签 \"ghost\" 不在 tags.json 里","hint":"nw tag ls 看全部标签;名字归一(Pi = pi);新标签先 nw tag add <标签> --ref <kind>/<条目>","details":{"catalogPath":"/r/tags.json"}}],"warnings":[],"data":{"catalogPath":"/r/tags.json","tags":[]}}"#;

    /// 摘引用回音(action 第三态)。
    const TAG_RM_REF: &str = r#"{"schema":"nw/tag-rm.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"name":"pi","catalogPath":"/r/tags.json","removed":true,"written":true,"action":"ref_removed","remaining":1}}"#;

    /// 删不存在的标签回音:rc 3 + 文件一个字节都不动。
    const TAG_RM_MISS: &str = r#"{"schema":"nw/tag-rm.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"标签 \"ghost\" 不在 tags.json 里","hint":"tags.json 是机器写的产物,永不手改","details":{"catalogPath":"/r/tags.json"}}],"warnings":[],"data":{"name":"ghost","catalogPath":"/r/tags.json","removed":false,"written":false,"action":"not_found","remaining":2}}"#;

    /// 删整个标签成功回音(引用留空 = 删标签本身)。
    const TAG_RM_LAST: &str = r#"{"schema":"nw/tag-rm.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"name":"pi","catalogPath":"/r/tags.json","removed":true,"written":true,"action":"removed","remaining":1}}"#;

    /// 机器册 3 条投影(点行 / 滚轮 / rm 互斥测试共用)。
    const CAT_VPS_LS: &str = r#"{"catalogPath":"/r/vps.json","machines":[{"name":"gate","ips":[],"port":22,"logins":[]},{"name":"bare","ips":[],"port":0,"logins":[]},{"name":"zen","ips":[],"port":0,"logins":[]}]}"#;

    /// 机器册删一台成功回音(剩 2 台)。
    const VPS_RM_LAST: &str = r#"{"schema":"nw/vps-rm.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"name":"gate","catalogPath":"/r/vps.json","removed":true,"written":true,"action":"removed","remaining":2}}"#;

    /// 删后即时重读的机器册投影(合法空态也成立,这里留 1 台)。
    const VPS_LS_GATE_ONLY: &str = r#"{"catalogPath":"/r/vps.json","machines":[{"name":"gate","ips":[],"port":22,"logins":[]}]}"#;

    /// 一台能力齐备的标签册夹具:引擎有 tag 组、数据仓根可用。
    fn tag_app(rules: &[(&str, i32, &str)]) -> crate::app::App {
        let mut app = crate::app::App::with_runner(std::sync::Arc::new(
            crate::engine::FakeRunner::new(rules),
        ));
        app.splash = false;
        app.screen = crate::app::Screen::Vps;
        app.catalog_kind = CatalogKind::Tag;
        app.capabilities.record(
            crate::engine::Capability::Tag,
            crate::engine::Probe::Available,
        );
        app
    }

    /// 把队列里的命令经唯一入口下发,并把回音灌回状态树(生产同路:
    /// 主循环收 tx)。FakeRunner 同步回音,所以 drain 一次即收敛。
    fn feed(app: &mut crate::app::App) {
        use crate::app::Message;
        use crate::engine::EngineEvent;
        use tokio::sync::mpsc;
        let (tx, mut rx) = mpsc::unbounded_channel::<EngineEvent>();
        for argv in std::mem::take(&mut app.queue) {
            app.spawn_engine(argv, &tx);
        }
        // 喂入队列(#56):带 stdin 本体的命令走同一条唯一入口(多一份一次性本体)
        for (argv, body) in std::mem::take(&mut app.fed_queue) {
            app.spawn_engine_fed(argv, body, &tx);
        }
        while let Ok(ev) = rx.try_recv() {
            app.update(Message::Engine(ev));
        }
    }

    fn tap(app: &mut crate::app::App, code: KeyCode) {
        use crate::app::Message;
        app.update(Message::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    /// 标签册屏常畅态 / 紧凑态两档都成立(#54 验收):不引新断点、不缩放
    /// 字号,66×50 紧凑态下也不溢出、不 panic,键位行与边界表述常在。
    #[test]
    fn 标签册屏_常畅与紧凑两档都渲染不溢出() {
        let mut app = tag_app(&[("tag ls", 0, TAG_LS_ALL)]);
        app.run_tag_ls(None); // 反查一次,册体才有内容可画
        feed(&mut app);
        for (w, h) in [(80u16, 30u16), (66, 50), (100, 40)] {
            let s = crate::testutil::frame_display(&app, w, h);
            assert!(s.contains("当前册 标签"), "{w}×{h} 当前是哪一本应在场:{s}");
            assert!(s.contains("pi"), "{w}×{h} 标签名应在场");
            assert!(s.contains("引用"), "{w}×{h} 引用行应在场");
            assert!(
                s.contains("绝不进配方"),
                "{w}×{h} 边界表述应在场(标签不进配方/锁定集/bundle/装配)"
            );
            assert!(s.contains("m 换册"), "{w}×{h} 键位行应在场:{s}");
            for ln in s.lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 有行超宽:{ln}"
                );
            }
        }
    }

    /// 反查两态(本票主交付)都经面板完成:l = 列全部,Enter = 只列选中的
    /// 那一条。两种 argv 都由面板下发,形状与 `nw tag ls` 契约一致。
    #[test]
    fn 反查两态_列全部与只列一个_都经面板完成() {
        let mut app = tag_app(&[("tag ls", 0, TAG_LS_ALL)]);
        // ① 不带参:列全部标签及各引用
        let root = app.usable_data_root().unwrap_or_default();
        tap(&mut app, KeyCode::Char('l'));
        assert_eq!(
            app.queue.last().map(|a| a.join(" ")).as_deref(),
            Some(format!("tag ls --root {root}").as_str())
        );
        assert!(app.tag_ls_loading, "反查在途不拿上一轮结论冒充本次");
        feed(&mut app);
        assert_eq!(app.tag_len(), 2, "列全部:两个标签");
        assert_eq!(
            app.selected_tag_name().as_deref(),
            Some("bare"),
            "排序后首行"
        );
        // ② 带参:只列那一条(选中第二行 = pi)
        tap(&mut app, KeyCode::Down);
        tap(&mut app, KeyCode::Enter);
        let argv = app.queue.last().cloned().unwrap_or_default();
        assert_eq!(argv[0], "tag");
        assert_eq!(argv[1], "ls");
        assert_eq!(argv[2], "pi", "带参反查只问这一条");
        assert!(argv.contains(&"--root".to_string()), "只认 --root");
        feed(&mut app);
        assert_eq!(app.tag_len(), 2);
        assert_eq!(
            app.catalog_scope_label(),
            "反查 pi",
            "只列一个也说清问的是谁"
        );
        // ③ 问错了名字:rc 3,册体不画空列表(两种语义不混)
        let mut miss = tag_app(&[("tag ls", 3, TAG_LS_MISS)]);
        miss.run_tag_ls(Some("ghost")); // 带参反查一个不在编目里的名字
        feed(&mut miss);
        assert!(miss.catalog_ls_error().is_some(), "rc 3 留失败态");
        assert_eq!(miss.tag_len(), 0, "失败态不呈现册体清单");
        let s = crate::testutil::frame_display(&miss, 80, 30);
        assert!(s.contains("反查没结果"), "rc 3 给人话:{s}");
        assert!(s.contains("问错了名字"), "说清不是空结果:{s}");
    }

    // ---- #120 反查结果跳转:标签册命中条目回车直跳对应册 ----

    /// 标签册**带参反查**回音(#120):只问 pi 一条,引擎给这一条 + 它的引用。
    /// 引用跨三个 kind——两个有去处(`skill` / `vps`),一个**没有**
    /// (`mystery`)。引擎今天不会给出处表外的 kind(第②道闸就拒),但 ls 回音
    /// 是数据,面板不能假设它永远只给已知 kind:无去处那条专门用来钉
    /// 「明示而非死键」。
    const TAG_LS_ONE: &str = r#"{"schema":"nw/tag-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/tags.json","tags":[{"name":"pi","refs":[{"kind":"skill","id":"o/r/deep-research"},{"kind":"vps","id":"gate"},{"kind":"mystery","id":"x"}]}]}}"#;

    /// 五域回音的**外部源域**(#120):两条源都带一条叫 `deep-research` 的
    /// 条目——条目名撞车时按仓库提示择优(标签引用 `skill/o/r/deep-research`
    /// 该落到 `o/r`,不该落到排在前面的 `x/y`)。第三条没有条目清单。
    const LS_SOURCES: &str = r#"{"domain":"all","recipes":[],"components":[],"sources":[{"name":"other","repo":"https://github.com/x/y","kind":"skill","entries":[{"name":"deep-research","path":"skills/deep-research/SKILL.md"}]},{"name":"o/r","repo":"https://github.com/o/r","kind":"skill","entries":[{"name":"deep-research","path":"skills/deep-research/SKILL.md"}]},{"name":"empty-pack","repo":"https://github.com/e/f","kind":"extension","entries":[]}],"profiles":[],"locks":[]}"#;

    /// 机器册投影(#120 跳转夹具):**已按名排好序**的三台(引擎的编目册输出
    /// 是确定性排序的,生产路径上「册体行序」恒等于「数组序」)。`gate` 是
    /// 标签引用 `vps/gate` 的落点。
    ///
    /// 两份形状:裸 data(直接 `serde_json::from_str::<VpsLsData>()` 灌
    /// `app.vps_ls`)与整信封(FakeRunner 回放用——回音要过契约守卫那一关,
    /// 少 `schema` 就是「stdout 不是合法 envelope」)。
    const JUMP_VPS_LS: &str = r#"{"catalogPath":"/r/vps.json","machines":[{"name":"bare","ips":[],"port":0,"logins":[]},{"name":"gate","ips":[],"port":22,"logins":[]},{"name":"zen","ips":[],"port":0,"logins":[]}]}"#;

    /// [`JUMP_VPS_LS`] 的整信封形状(FakeRunner 的 `vps ls` 规则用)。
    fn jump_vps_ls_echo() -> String {
        format!(
            r#"{{"schema":"nw/vps-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{JUMP_VPS_LS}}}"#
        )
    }

    /// 一路走到「标签册反查某一条、命中浮层在场」(#120 的共同就绪态)。
    ///
    /// 一并把 **vps 组能力**记成可用:第 2 条命中的去处是机器册,
    /// `run_vps_ls` 的门(先根后能力)要过能力这一道才放行刷新——夹具只记
    /// Tag 一门,跳转那条入队刷新的路就走不到(那是「引擎没这能力」的合法
    /// 结论,由 `反查跳转_目标册缺能力时不入队只指路` 单独钉)。
    fn jump_ready() -> crate::app::App {
        let echo = jump_vps_ls_echo();
        let mut app = tag_app(&[("tag ls", 0, TAG_LS_ONE), ("vps ls", 0, echo.as_str())]);
        app.capabilities.record(
            crate::engine::Capability::Vps,
            crate::engine::Probe::Available,
        );
        app.run_tag_ls(Some("pi"));
        feed(&mut app);
        assert!(app.tag_jump_open(), "就绪态:反查跳转浮层应在场");
        app
    }

    /// 去处表与引擎的已知 kind 集合**互为镜像**(#120):上游 `TAG_REF_KINDS`
    /// 里每个 kind 都有去处(不多不少),去处表也不替上游发明 kind。上游加
    /// kind 而这里没跟上,这条当场红——少一个去处 = 那个 kind 的命中成死键。
    #[test]
    fn tag_ref_kinds_每个kind都有去处_去处表也不发明kind() {
        for k in crate::engine::commands::TAG_REF_KINDS {
            assert!(
                jump_dest(k).is_some(),
                "引擎已知 kind `{k}` 在去处表里没有落点(会变成死键)"
            );
        }
        for d in [JumpDest::Machine, JumpDest::SourceEntry] {
            let known = match d {
                JumpDest::Machine => "vps",
                JumpDest::SourceEntry => "skill",
            };
            assert!(
                crate::engine::commands::TAG_REF_KINDS.contains(&known),
                "去处 {d:?} 对应的 kind 不在引擎已知集合里(面板在替上游发明 kind)"
            );
        }
        assert!(
            jump_dest("subscription-url").is_none(),
            "订阅/钥匙条目不在 TAG_REF_KINDS 里,没有去处(明说,不造)"
        );
    }

    /// skill / extension 命中的落点纯函数(#120):条目名 + 仓库提示两步匹配,
    /// 同条目名多源时按仓库提示择优,没有提示时退化成「只看条目名」。
    #[test]
    fn source_hit_index_条目名加仓库提示两步匹配() {
        let ls: crate::engine::LsData = serde_json::from_str(LS_SOURCES).unwrap();
        assert_eq!(
            source_hit_index(&ls, "o/r/deep-research"),
            Some(1),
            "仓库提示 o/r 择优:不落到排在前面的 x/y"
        );
        assert_eq!(
            source_hit_index(&ls, "deep-research"),
            Some(0),
            "没有仓库提示 = 只看条目名,取引擎顺序里的第一个"
        );
        assert_eq!(
            source_hit_index(&ls, "x/y/deep-research"),
            Some(0),
            "仓库提示 x/y 落到另一条"
        );
        assert_eq!(
            source_hit_index(&ls, "o/r/nope"),
            None,
            "条目名对不上 = 这一条在当前五域回音里没有去处"
        );
        assert_eq!(source_hit_index(&ls, ""), None, "空 id 不配对");
        assert_eq!(source_hit_index(&ls, "o/r/"), None, "条目名为空不配对");
    }

    /// 反查跳转浮层在场:册体原样在场(叠加层,不是模式劫持),命中条目逐条
    /// 列出并**带去处**;无去处那条画成 warn 的「→ 无去处」——明示,不藏。
    /// 常畅 / 紧凑两档都不溢宽(66×50 下限)。
    #[test]
    fn 反查跳转浮层_命中条目列出且带去处_两档不溢宽() {
        let app = jump_ready();
        for (w, h) in [(80u16, 30u16), (66, 50), (110, 30)] {
            let s = crate::testutil::frame_display(&app, w, h);
            assert!(s.contains("这一条引用了什么"), "{w}×{h} 浮层标题:{s}");
            assert!(s.contains("标签 pi 引用了 3 条"), "{w}×{h} 命中数:{s}");
            assert!(
                s.contains("skill/o/r/deep-research"),
                "{w}×{h} 命中条目:{s}"
            );
            assert!(s.contains("vps/gate"), "{w}×{h} vps 命中:{s}");
            assert!(s.contains("→ 外部源域"), "{w}×{h} 去处写明:{s}");
            assert!(s.contains("→ 机器册"), "{w}×{h} 去处写明:{s}");
            assert!(s.contains("→ 无去处"), "{w}×{h} 无去处明示:{s}");
            assert!(s.contains("Enter 直跳所选条目"), "{w}×{h} 收口键位:{s}");
            // 册体仍在场:浮层是叠加层,把编目册本体盖掉就不是叠加了
            assert!(s.contains("当前册 标签"), "{w}×{h} 册体原样在场:{s}");
            for ln in s.lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 不溢宽:{ln:?}"
                );
            }
        }
    }

    /// vps 命中 → 机器册,选中位落在命中那一台上(#120 验收第 1 条)。
    ///
    /// 夹具刻意用**已按名排好序**的投影:引擎的编目册输出是确定性排序的
    /// (「机器写、确定性排序」),所以生产路径上「册体行序」与「引擎给的
    /// 数组序」恒同。选中位断言同时看两处——帧上 `▸` 落在 gate 那一行
    /// (操作者眼里的事实)与 `selected_vps_name()`(面板自己的取值口,
    /// `d 删除` 用的就是它)——两边必须同时成立,跳转才算真的落对。
    #[test]
    fn 反查跳转_vps命中落到机器册并选中那一台() {
        let mut app = jump_ready();
        app.vps_ls = Some(serde_json::from_str(JUMP_VPS_LS).unwrap());
        tap(&mut app, KeyCode::Down); // 浮层内选中第 2 条 = vps/gate
        tap(&mut app, KeyCode::Enter);
        assert!(!app.tag_jump_open(), "跳转与浮层关闭同帧完成");
        assert_eq!(app.screen, crate::app::Screen::Vps, "落到编目册屏");
        assert_eq!(app.catalog_kind, CatalogKind::Vps, "vps 条目落到机器册");
        assert_eq!(
            app.selected_vps_name().as_deref(),
            Some("gate"),
            "选中位落在命中那一台上"
        );
        let s = crate::testutil::frame_display(&app, 110, 30);
        // 帧上高亮行就是命中那一台。竖导航轨也有 `▸`(当前屏标记),所以按
        // 机器名找行,再断言那一行带高亮标记——按 `▸` 找行会先命中导航轨。
        let marked = s.lines().find(|l| l.contains("gate")).unwrap_or_default();
        assert!(marked.contains("▸"), "帧上高亮行就是命中那一台:{marked:?}");
        assert!(!app.assembling, "跳转是纯面板内改状态:不点写互斥灯");
    }

    /// skill 命中 → 配方屏外部源域,选中位落在命中条目**所在的那条源**上
    /// (#120 验收第 1 条 + 规格「按现有屏结构钉死」)。
    #[test]
    fn 反查跳转_skill命中落到外部源域并选中所在源() {
        let mut app = jump_ready();
        app.ls = Some(serde_json::from_str(LS_SOURCES).unwrap());
        tap(&mut app, KeyCode::Enter); // 第 1 条 = skill/o/r/deep-research
        assert!(!app.tag_jump_open(), "跳转与浮层关闭同帧完成");
        assert_eq!(app.screen, crate::app::Screen::Recipes, "落到配方屏");
        assert_eq!(app.rc_domain, crate::app::Domain::Sources, "落到外部源域");
        assert_eq!(app.rc_sel, 1, "选中位落在 o/r 那条源上(不是排在前面的 x/y)");
        let s = crate::testutil::frame_display(&app, 110, 30);
        assert!(s.contains("外部源"), "屏上就是外部源域:{s}");
    }

    /// 跳转后无残留(#120 验收第 2 条):浮层收了、反查态清了、册头口径回到
    /// 「全部标签」——不把数据仓根当标签名(#70 既有口径,别撞)。
    ///
    /// 先给足 `ls`:第 1 条命中是 skill,去处是外部源域,五域已读 = **同步
    /// 落地**,这才谈得上「不欠待落地选中位」。五域没读时走的是异步那半
    /// (入队刷新 + `jump_pending`),由下面那条专门钉。
    #[test]
    fn 反查跳转_同帧收浮层并无残留_册头回到全部标签() {
        let mut app = jump_ready();
        app.ls = Some(serde_json::from_str(LS_SOURCES).unwrap());
        tap(&mut app, KeyCode::Enter);
        assert!(!app.tag_jump_open(), "浮层关闭");
        assert!(app.tag_lookup.is_none(), "反查态清空");
        assert!(app.tag_ls_echo.is_none(), "不拿上一轮结论冒充本次");
        assert_eq!(
            app.catalog_scope_label(),
            "全部标签",
            "册头口径回到全部标签:不把数据仓根当标签名"
        );
        assert!(app.jump_pending.is_none(), "同步落地不欠待落地选中位");
        // 回到标签册:册体说的是「还没反查过」,不是上一轮的命中清单
        app.screen = crate::app::Screen::Vps;
        app.catalog_kind = CatalogKind::Tag;
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(
            s.contains("还没反查过"),
            "回来看到的是未反查态,不是陈 conclusion:{s}"
        );
    }

    /// kind 无去处 / 条目找不到 → 明示而非死键(#120 验收第 3 条):给人话、
    /// 留日志、不入队任何命令、选中位一个字节都没动。
    #[test]
    fn 反查跳转_kind无去处时明示而非死键() {
        let mut app = jump_ready();
        let qlen = app.queue.len();
        let sel0 = app.catalog_sel();
        tap(&mut app, KeyCode::Down);
        tap(&mut app, KeyCode::Down); // 第 3 条 = mystery/x,无去处
        tap(&mut app, KeyCode::Enter);
        assert!(!app.tag_jump_open(), "浮层照收");
        assert_eq!(app.queue.len(), qlen, "明示路零引擎调用");
        assert_eq!(app.catalog_sel(), sel0, "选中位没动");
        assert_eq!(
            app.screen,
            crate::app::Screen::Vps,
            "没有去处就不切屏(不把操作者扔去一个错的屏)"
        );
        assert!(
            app.log[0].contains("没有去处") && app.log[0].contains("mystery/x"),
            "日志说清是哪个 kind 的哪一条:{:?}",
            app.log.first()
        );
        // 条目在目标数据里找不到:另一句明示(引用可能已悬空)
        let mut miss = jump_ready();
        miss.vps_ls = Some(serde_json::from_str(CAT_VPS_LS).unwrap());
        // 把机器册投影换成没有 gate 的一份
        miss.vps_ls = Some(
            serde_json::from_str(r#"{"catalogPath":"/r/vps.json","machines":[{"name":"bare","ips":[],"port":0,"logins":[]}]}"#)
                .unwrap(),
        );
        tap(&mut miss, KeyCode::Down);
        tap(&mut miss, KeyCode::Enter);
        assert!(
            miss.log[0].contains("册里没有 gate"),
            "条目找不到给人话,不静默:{:?}",
            miss.log.first()
        );
    }

    /// 目标册还没读过(#120 验收第 1 条的异步半边):入队刷新 + 记待落地选中位,
    /// 回音到达才按名字落选中——不假装选中。
    #[test]
    fn 反查跳转_目标册未读时入队刷新_回音落地后选中() {
        let mut app = jump_ready();
        let qlen = app.queue.len();
        assert!(app.vps_ls.is_none(), "前提:机器册投影还没读过");
        tap(&mut app, KeyCode::Down); // vps/gate
        tap(&mut app, KeyCode::Enter);
        assert_eq!(app.screen, crate::app::Screen::Vps, "先切到机器册");
        assert_eq!(app.catalog_kind, CatalogKind::Vps);
        assert_eq!(app.queue.len(), qlen + 1, "入队一次 vps ls 刷新");
        assert_eq!(
            app.queue.last().map(|a| a.join(" ")).as_deref(),
            Some(format!("vps ls --root {}", app.usable_data_root().unwrap()).as_str()),
            "刷新走既有只读 argv,不新造调用形状"
        );
        assert_eq!(
            app.jump_pending,
            Some(crate::app::JumpPending::Machine("gate".to_string())),
            "待落地选中位记下要落哪一台"
        );
        assert_eq!(
            app.selected_vps_name().as_deref(),
            None,
            "投影没读到就不假装选中"
        );
        // 回音落地:选中位落在 gate 上
        feed(&mut app);
        assert!(app.jump_pending.is_none(), "落地后清待落位");
        assert_eq!(
            app.selected_vps_name().as_deref(),
            Some("gate"),
            "回音到达后选中位落在命中那一台上"
        );
    }

    /// 目标册**缺能力**时不入队、只指路(#120):刷新走的是 `run_vps_ls` 既有
    /// 那道门(先根后能力),门不过 = 一条必败的命令都不发。与「投影没读到」
    /// 是两回事,话不许互相冒充。
    #[test]
    fn 反查跳转_目标册缺能力时不入队只指路() {
        // tag_app 只记 Tag 一门:vps 组能力未决 → 门不过
        let echo = jump_vps_ls_echo();
        let mut app = tag_app(&[("tag ls", 0, TAG_LS_ONE), ("vps ls", 0, echo.as_str())]);
        app.run_tag_ls(Some("pi"));
        feed(&mut app);
        assert!(app.tag_jump_open(), "就绪态");
        let qlen = app.queue.len();
        tap(&mut app, KeyCode::Down); // vps/gate
        tap(&mut app, KeyCode::Enter);
        assert_eq!(app.queue.len(), qlen, "缺能力不入队任何命令");
        assert!(app.jump_pending.is_none(), "没入队就不欠待落地选中位");
        assert!(
            app.log[0].contains("刷新也没入队"),
            "日志说清为什么没刷新:{:?}",
            app.log.first()
        );
    }

    /// 鼠标与键盘同路(#120):点浮层里的命中条目 = 选中它并直跳,与
    /// `↑/↓` + `Enter` 同一个函数、同一条后果链。命中区矩形与渲染同源。
    #[test]
    fn 反查跳转_点命中条目等于上下加回车() {
        let mut app = jump_ready();
        app.vps_ls = Some(serde_json::from_str(JUMP_VPS_LS).unwrap());
        app.size = ratatui::layout::Rect::new(0, 0, 110, 30);
        let hits = crate::layout::hitmap(app.size, &app, crate::layout::size_class(app.size));
        let row = hits
            .iter()
            .find(|(_, t)| matches!(t, crate::layout::Target::TagJump(1)))
            .map(|(r, _)| *r)
            .expect("第 2 条命中应有可点矩形");
        // 点它 = 选中第 2 条(vps/gate)并直跳
        let mut app2 = jump_ready();
        app2.vps_ls = Some(serde_json::from_str(JUMP_VPS_LS).unwrap());
        app2.size = app.size;
        crate::app::App::update(
            &mut app2,
            crate::app::Message::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: row.x + 1,
                row: row.y,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert!(!app2.tag_jump_open(), "点了就跳,浮层同帧收");
        assert_eq!(app2.screen, crate::app::Screen::Vps);
        assert_eq!(
            app2.selected_vps_name().as_deref(),
            Some("gate"),
            "点第 2 条 = 键盘选中第 2 条再回车"
        );
    }

    /// 反查跳转浮层对鼠标的收口(#120):浮层外的正文是「点任意处关闭」,与
    /// Esc 同路;浮层里的命中行才是动作。
    #[test]
    fn 反查跳转浮层_点浮层外关闭_命中行才是动作() {
        let mut app = jump_ready();
        app.size = ratatui::layout::Rect::new(0, 0, 110, 30);
        let hits = crate::layout::hitmap(app.size, &app, crate::layout::size_class(app.size));
        assert!(
            hits.iter()
                .any(|(_, t)| matches!(t, crate::layout::Target::DismissPopup)),
            "浮层外点任意处 = 关闭(Esc 同路)"
        );
        assert!(
            hits.iter()
                .all(|(_, t)| !matches!(t, crate::layout::Target::SelectRow(_))),
            "浮层吞点击:底层清单行不产命中"
        );
        // 点浮层外 = 关
        let outside = hits
            .iter()
            .find(|(r, t)| {
                matches!(t, crate::layout::Target::DismissPopup) && r.y < app.size.height - 1
            })
            .map(|(r, _)| *r)
            .expect("有关闭命中区");
        crate::app::App::update(
            &mut app,
            crate::app::Message::Mouse(crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: outside.x,
                row: outside.y,
                modifiers: KeyModifiers::NONE,
            }),
        );
        assert!(!app.tag_jump_open(), "点浮层外 = Esc 同路关闭");
    }

    /// 只有「标签册 + 正在反查某一条」才开跳转浮层(#120):列全部(`l`)与
    /// 机器册反查上浮都走 `run_tag_ls(None)`,`tag_lookup` 是 None——那一态
    /// 没有「这一条的命中」可言,不开浮层(开了就是噪声)。
    #[test]
    fn 列全部与机器册反查上浮_都不开跳转浮层() {
        // ① 标签册列全部
        let mut all = tag_app(&[("tag ls", 0, TAG_LS_ALL)]);
        tap(&mut all, KeyCode::Char('l'));
        feed(&mut all);
        assert!(!all.tag_jump_open(), "列全部不开跳转浮层");
        // ② 机器册反查上浮(#70):回音同样落 tag_ls_echo,但不该开跳转浮层
        let (_r, mut rev) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        反查这台(&mut rev);
        feed(&mut rev);
        assert!(rev.vps_reverse_open(), "机器册反查上浮在场");
        assert!(
            !rev.tag_jump_open(),
            "反查上浮不开跳转浮层(问的不是同一件事)"
        );
        // ③ 换册即收:标签册的命中清单不顺带到别的册上
        // ③ 浮层在场 = 顶层浮层吞键:`m` 换册不被受理(与反查上浮同口径——
        //    弹层优先级高于册体键位)。要先 Esc 收浮层,再换册;换册本身也清
        //    状态(防御:哪条路径换了册却忘了清,画出来的就是标签册的命中
        //    清单摊在别的册上)。
        let mut j = jump_ready();
        tap(&mut j, KeyCode::Char('m'));
        assert!(j.tag_jump_open(), "浮层在场时吞册体键:换册不被受理");
        assert_eq!(j.catalog_kind, CatalogKind::Tag, "册没换");
        tap(&mut j, KeyCode::Esc);
        assert!(!j.tag_jump_open(), "Esc 收浮层");
        tap(&mut j, KeyCode::Char('m'));
        assert!(!j.tag_jump_open(), "换册后不收着浮层");
        assert_eq!(
            j.catalog_kind,
            CatalogKind::Subscription,
            "标签册之后是订阅册"
        );
    }

    /// 反查跳转是只读(#120):不点亮 `assembling` 写互斥、不新增任何写路径。
    /// 写互斥点亮意味着别的写动作会被挡住——一次跳转不该有那个代价。
    #[test]
    fn 反查跳转_是只读_不点亮assembling写互斥() {
        let mut app = jump_ready();
        app.vps_ls = Some(serde_json::from_str(JUMP_VPS_LS).unwrap());
        let qlen = app.queue.len();
        tap(&mut app, KeyCode::Down);
        tap(&mut app, KeyCode::Enter);
        assert!(!app.assembling, "跳转不占写互斥");
        assert_eq!(app.queue.len(), qlen, "投影已读过:零引擎调用");
    }

    /// add 幂等由面板复用同一 argv 保证;四道闸摊开给看;无确认闸
    /// (ADR-0009 §2:面板不自己造一道,画面没有「Enter 落账」段)。
    #[test]
    fn 挂标签_无确认闸_同一argv重跑字节不变_四道闸摊开() {
        let mut app = tag_app(&[("tag add", 0, TAG_ADD_OK)]);
        tap(&mut app, KeyCode::Char('r'));
        assert_eq!(app.tag_flow.phase, RegPhase::Form);
        // 表单:标签名 + 一条引用
        for c in "pi".chars() {
            tap(&mut app, KeyCode::Char(c));
        }
        tap(&mut app, KeyCode::Down); // 移到引用字段
        for c in "skill/o/r/deep-research".chars() {
            tap(&mut app, KeyCode::Char(c));
        }
        tap(&mut app, KeyCode::Enter); // 直接下发(无确认闸)
        let root = app.usable_data_root().unwrap_or_default();
        let first = app.queue.last().cloned().unwrap_or_default();
        assert_eq!(
            first,
            vec![
                "tag".to_string(),
                "add".to_string(),
                "pi".to_string(),
                "--ref".to_string(),
                "skill/o/r/deep-research".to_string(),
                "--root".to_string(),
                root.clone(),
            ],
            "argv 与 nw tag add 契约一致"
        );
        assert!(
            !first.iter().any(|a| a == "--yes"),
            "上游没有 --yes,面板不自己造一枚(改了引擎语义)"
        );
        assert!(app.assembling, "这一条 argv 就是写命令:点亮写互斥");
        feed(&mut app);
        assert_eq!(app.tag_flow.phase, RegPhase::Done, "回音摊开闸结论即终态");
        // 幂等:同一张表单再下发,argv 逐字节相同(引擎侧字节不变)
        app.tag_flow.abandon();
        tap(&mut app, KeyCode::Char('r'));
        for c in "pi".chars() {
            tap(&mut app, KeyCode::Char(c));
        }
        tap(&mut app, KeyCode::Down);
        for c in "skill/o/r/deep-research".chars() {
            tap(&mut app, KeyCode::Char(c));
        }
        tap(&mut app, KeyCode::Enter);
        let second = app
            .queue
            .iter()
            .rev()
            .find(|a| a.get(1).map(String::as_str) == Some("add"))
            .cloned()
            .unwrap_or_default();
        assert_eq!(second, first, "幂等由面板复用同一份 argv 保证");
        // 四道闸摊开给看(恒 4 行,id 冻结)
        feed(&mut app);
        let s = crate::testutil::frame_display(&app, 80, 30);
        for id in [
            "tag.add.name",
            "tag.add.ref-kind",
            "tag.add.ref-id",
            "tag.add.ref-exists",
        ] {
            assert!(s.contains(id), "闸 {id} 应摊开:{s}");
        }
        assert!(!s.contains("Enter 落账"), "无确认闸:不给落账段:{s}");
    }

    /// 任一闸不过 = 一个字节都不落:回音给失败态 + 四道闸原文,且面板没有
    /// 第二条写命令可发。
    #[test]
    fn 闸有fail_一个字节都不落_且没有落账段() {
        let mut app = tag_app(&[("tag add", 3, TAG_ADD_FAIL)]);
        tap(&mut app, KeyCode::Char('r'));
        tap(&mut app, KeyCode::Char('p'));
        tap(&mut app, KeyCode::Char('i'));
        tap(&mut app, KeyCode::Enter);
        assert_eq!(app.queue.len(), 1, "只有一条写命令");
        feed(&mut app);
        assert_eq!(app.tag_flow.phase, RegPhase::Review);
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("一个字节都没落"), "{s}");
        assert!(s.contains("tag.add.ref-id"), "失败的那道闸在场:{s}");
        assert!(s.contains("第③道闸没过"), "证据五字段原样:{s}");
        assert!(!s.contains("Enter 落账"), "闸不过不放行,也没有落账段:{s}");
        assert!(!app.assembling, "写互斥随回音收敛");
    }

    /// 悬空引用清理在界面明示条数,不静默(工单 #54 验收第 4 条)。
    #[test]
    fn 悬空引用清理_界面明示条数() {
        let mut app = tag_app(&[("tag add", 0, TAG_ADD_CLEANED)]);
        app.tag_flow.begin();
        app.tag_form.name = "pi".into();
        app.tag_form.refs = vec!["skill/o/r/deep-research".into()];
        tap(&mut app, KeyCode::Enter);
        feed(&mut app);
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("悬空引用已清 2 条"), "条数明示:{s}");
        assert!(s.contains("extension/gone-pack"), "清掉哪几条也在场:{s}");
        assert!(s.contains("vps/ghost"), "第二条也在场:{s}");
    }

    /// rm 两态都可完成:带引用只摘那几条,不带删整个标签;删不存在的标签
    /// 给证据且文件一个字节都不动(不刷新册体)。
    #[test]
    fn 摘引用与删标签两态_删不存在的标签给证据且不动文件() {
        // ① 不带引用 = 删整个标签(带面板图形确认)
        let mut app = tag_app(&[("tag ls", 0, TAG_LS_ALL), ("tag rm", 0, TAG_RM_REF)]);
        app.run_tag_ls(None); // 先反查到,才能选中
        feed(&mut app);
        tap(&mut app, KeyCode::Down); // 排序后首行是 bare,第二行才是 pi
        tap(&mut app, KeyCode::Char('d'));
        assert_eq!(
            app.tag_flow.phase,
            RegPhase::Form,
            "先进表单(名字预填、光标在引用行)"
        );
        assert_eq!(app.tag_form.name, "pi", "预填选中的标签名");
        assert_eq!(app.tag_form.field, TagField::Refs, "光标落在引用字段");
        assert!(app.queue.is_empty(), "表单没确认前不下发任何写命令");
        tap(&mut app, KeyCode::Enter); // 引用留空 → 进图形确认
        assert!(app.tag_rm_confirm, "面板加一道图形确认(ADR-0009 §2)");
        assert!(app.queue.is_empty(), "确认前不下发任何写命令");
        tap(&mut app, KeyCode::Enter);
        let root = app.usable_data_root().unwrap_or_default();
        let argv = app.queue.last().cloned().unwrap_or_default();
        assert_eq!(
            argv,
            vec![
                "tag".to_string(),
                "rm".to_string(),
                "pi".to_string(),
                "--root".to_string(),
                root.clone(),
            ],
            "不带 --ref = 删整个标签"
        );
        assert!(!argv.iter().any(|a| a == "--yes"), "上游 rm 没有确认闸");
        feed(&mut app);
        assert!(
            app.queue
                .iter()
                .any(|a| a.get(1).map(String::as_str) == Some("ls")),
            "删后即时反查刷新册体(只读)"
        );
        feed(&mut app);
        assert_eq!(app.tag_len(), 2, "册体已刷新");
        // ② 带引用 = 只摘那几条
        tap(&mut app, KeyCode::Char('d'));
        for c in "vps/gate".chars() {
            tap(&mut app, KeyCode::Char(c));
        }
        tap(&mut app, KeyCode::Enter); // 表单 → 图形确认
        assert!(app.tag_rm_confirm);
        assert!(app
            .queue
            .iter()
            .all(|a| a.get(1).map(String::as_str) != Some("rm")));
        tap(&mut app, KeyCode::Enter); // 确认 → 下发
        let root = app.usable_data_root().unwrap_or_default();
        let argv = app.queue.last().cloned().unwrap_or_default();
        assert_eq!(
            argv,
            vec![
                "tag".to_string(),
                "rm".to_string(),
                "pi".to_string(),
                "--ref".to_string(),
                "vps/gate".to_string(),
                "--root".to_string(),
                root,
            ],
            "带 --ref = 只摘这一条引用"
        );
        feed(&mut app);
        assert!(
            app.log.iter().any(|l| l.contains("摘掉引用")),
            "action ref_removed 的说法在场:{:?}",
            app.log.first()
        );
        // ③ 删不存在的标签:rc 3 + 证据 + 文件一个字节都不动
        let mut miss = tag_app(&[("tag rm", 3, TAG_RM_MISS)]);
        miss.tag_flow.abandon();
        miss.tag_form = new_tag_form(TagFormMode::Rm);
        miss.tag_form.name = "ghost".into();
        miss.tag_rm_confirm = true;
        tap(&mut miss, KeyCode::Enter);
        feed(&mut miss);
        assert!(miss.queue.is_empty(), "没有后续写命令");
        assert!(
            miss.log.iter().any(|l| l.contains("一个字节都没动")),
            "删不存在的标签给证据:{:?}",
            miss.log.first()
        );
    }

    /// 复用 #53 的骨架:标签册走同一个列表件(`catalog_sheet_lines`),
    /// 只换 [`EntryDetail`](明细画法)——不复制第二份投影 / 列表 / 落盘逻辑。
    #[test]
    fn 列表件两本共用_标签册只换明细画法() {
        let sheet = CatalogSheet {
            catalog_path: "/r/tags.json".into(),
            entries: vec![CatalogEntryView {
                name: "pi".into(),
                refs: vec![
                    CatalogRef {
                        kind: "skill".into(),
                        id: "o/r/deep-research".into(),
                    },
                    CatalogRef {
                        kind: "vps".into(),
                        id: "gate".into(),
                    },
                ],
                ..Default::default()
            }],
            gates: Vec::new(),
        };
        let p = crate::theme::cached_degrade(
            crate::theme::ThemeKind::Warehouse,
            crate::term::ColorCap::Truecolor,
        );
        let text = |detail: EntryDetail| -> String {
            catalog_sheet_lines(
                &sheet,
                &(0..sheet.entries.len()).collect::<Vec<_>>(),
                60,
                0,
                &p,
                detail,
            )
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
        };
        // 同一份投影,两个画法:机器册给端口/地址/登录/备注,标签册给引用
        let vps = text(vps_detail_rows);
        let tag = text(tag_detail_rows);
        assert!(vps.contains("端口 ——"), "机器册:{vps}");
        assert!(!vps.contains("引用"), "机器册不画引用槽:{vps}");
        assert!(
            tag.contains("引用 skill/o/r/deep-research · vps/gate"),
            "标签册:{tag}"
        );
        assert!(!tag.contains("端口"), "标签册不画端口槽(没有这一槽):{tag}");
        assert_eq!(
            sheet.entries[0].ref_line(),
            "skill/o/r/deep-research · vps/gate"
        );
    }

    /// 标签册表单:kind 不在已知集合的行被面板侧拒下,不喂给引擎猜;
    /// 标签名为空同理。坏行留证据、留驻可改。
    #[test]
    fn 表单解析_坏引用行被拒_不喂给引擎猜() {
        let mut form = new_tag_form(TagFormMode::Add);
        form.name = "pi".into();
        form.refs = vec![
            "skill/o/r/deep-research".into(),
            "recipe/x".into(), // kind 不在已知集合
            "vps/".into(),     // 空条目
            String::new(),     // 空行 = 没填,不是坏行
        ];
        let (spec, bad) = TagSpec::from_form(&form);
        assert_eq!(spec.refs.len(), 1, "只有好行进 argv");
        assert_eq!(bad.len(), 2, "两个坏行各留一条证据");
        assert!(spec_from_tag_form(&form).is_err(), "有坏行就不下发");
        // 规格的两段 argv 对 tag 是同一份(上游没有 --yes,详见 TagSpec 文档)
        let root = "/r";
        assert_eq!(spec.preaudit(root), spec.commit(root));
        assert!(!spec.preaudit(root).iter().any(|a| a == "--yes"));
        assert!(!TagSpec::confirm_gate(), "tag add 无确认闸");
        assert!(VpsSpec::confirm_gate(), "vps 有 --yes 确认闸(默认不变)");
    }
    // ---- 工单 #56:订阅册(本体只经 stdin / 到期三态 / get 只给建议命令) ----

    /// 订阅 ls 回音:四条订阅覆盖四个态——远未来 active、7 天内 expiring、
    /// 过去 expired(过期不改退出码:整份回音 ok:true)、以及没给到期日的
    /// 永不过期(daysLeft = null)。
    const SUB_LS_ALL: &str = r#"{"schema":"nw/subscription-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/subscriptions.json","subscriptions":[{"name":"airport","kind":"subscription-url","client":"Mihomo","expires":"2099-03-01","status":"active","daysLeft":26000},{"name":"soon","kind":"subscription-url","client":"Clash","expires":"2099-03-04","status":"expiring","daysLeft":3},{"name":"gone","kind":"subscription-url","client":"Mihomo","expires":"2020-01-01","status":"expired","daysLeft":-2000},{"name":"forever","kind":"subscription-url","client":"Surge","expires":"","status":"active","daysLeft":null}]}}"#;

    /// 登记预审回音:四道闸全 pass、`written:false`(不带 `--yes`,文件不落)。
    /// **data 里一个能放链接的字段都没有**(上游 RegisterResult 刻意不留 url 槽)。
    const SUB_REG_OK: &str = r#"{"schema":"nw/subscription-register.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"name":"airport","kind":"subscription-url","client":"Mihomo","expires":"2099-03-01","catalogPath":"/r/subscriptions.json","written":false,"updated":false,"action":"registered","gates":[{"id":"subscription.register.name","status":"pass","message":"订阅名合法"},{"id":"subscription.register.client","status":"pass","message":"应用认"},{"id":"subscription.register.expires","status":"pass","message":"到期日认"},{"id":"subscription.register.body","status":"pass","message":"本体非空、单行"}]}}"#;

    /// 登记预审失败回音:第③道闸(expires)不过 → 一个字节都不落,后面一道 skip。
    /// 主角是「2099-02-30」:面板只校 YYYY-MM-DD **形状**(放行),引擎按
    /// `time.Parse` 往返校**真日历**(拒)——面板不复刻那道判定。
    const SUB_REG_FAIL: &str = r#"{"schema":"nw/subscription-register.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"第③道闸没过:--expires 不是真实日期(2099-02-30)","hint":"到期日严格 YYYY-MM-DD(可省 = 永不过期);02-30 这种日历上不存在的日子一律拒","details":{"gate":"subscription.register.expires"}}],"warnings":[],"data":{"name":"airport","kind":"subscription-url","client":"Mihomo","expires":"2099-02-30","catalogPath":"/r/subscriptions.json","written":false,"updated":false,"action":"rejected","gates":[{"id":"subscription.register.name","status":"pass","message":"订阅名合法"},{"id":"subscription.register.client","status":"pass","message":"应用认"},{"id":"subscription.register.expires","status":"fail","message":"不是真实日期"},{"id":"subscription.register.body","status":"skip","message":"前面已拒"}]}}"#;

    /// 删一条订阅成功:删光最后一条,`remaining: 0`(文件留空 entries 数组)。
    const SUB_RM_LAST: &str = r#"{"schema":"nw/subscription-rm.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"name":"forever","catalogPath":"/r/subscriptions.json","removed":true,"written":true,"action":"removed","remaining":0}}"#;

    /// 删不存在的订阅:rc 3 + 证据五字段 + 文件一个字节都不动。
    const SUB_RM_MISS: &str = r#"{"schema":"nw/subscription-rm.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"订阅 \"ghost\" 不在 subscriptions.json 里","hint":"subscriptions.json 是机器写的产物,永不手改","details":{"catalogPath":"/r/subscriptions.json"}}],"warnings":[],"data":{"name":"ghost","catalogPath":"/r/subscriptions.json","removed":false,"written":false,"action":"not_found","remaining":2}}"#;

    /// 订阅 ls 的 **data 面**(直接灌 `sub_ls` 状态用):只剩一条、永不过期。
    const SUB_LS_ONE: &str = r#"{"catalogPath":"/r/subscriptions.json","subscriptions":[{"name":"forever","kind":"subscription-url","client":"Surge","expires":"","status":"active","daysLeft":null}]}"#;

    /// 删光最后一条之后的 ls 回音:空数组(合法空态,文件不删)。
    const SUB_LS_EMPTY_ENV: &str = r#"{"schema":"nw/subscription-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/subscriptions.json","subscriptions":[]}}"#;

    /// 一台能力齐备的订阅册夹具:引擎有 subscription 组、数据仓根可用。
    /// 连同 runner 一起交还(要数「收到过几次 stdin 喂入」)。
    fn sub_app(
        rules: &[(&str, i32, &str)],
    ) -> (crate::app::App, std::sync::Arc<crate::engine::FakeRunner>) {
        let runner = std::sync::Arc::new(crate::engine::FakeRunner::new(rules));
        let mut app = crate::app::App::with_runner(runner.clone());
        app.splash = false;
        app.screen = crate::app::Screen::Vps;
        app.catalog_kind = CatalogKind::Subscription;
        app.capabilities.record(
            crate::engine::Capability::Subscription,
            crate::engine::Probe::Available,
        );
        (app, runner)
    }

    /// 在订阅册表单里填名字/应用/到期日,并在链接框喂入本体(掩码字段)。
    fn fill_sub_form(
        app: &mut crate::app::App,
        name: &str,
        client: &str,
        expires: &str,
        link: &str,
    ) {
        for c in name.chars() {
            tap(app, KeyCode::Char(c));
        }
        tap(app, KeyCode::Down);
        for c in client.chars() {
            tap(app, KeyCode::Char(c));
        }
        tap(app, KeyCode::Down);
        for c in expires.chars() {
            tap(app, KeyCode::Char(c));
        }
        tap(app, KeyCode::Down); // 光标落到链接(掩码)框
        for c in link.chars() {
            tap(app, KeyCode::Char(c));
        }
    }

    /// 登记走两段式:链接本体只经 stdin 进(argv 面无明文形参),四道闸摊开,
    /// 任一不过 = 一个字节都不落、面板不放行落账。
    #[test]
    fn 订阅登记_两段式_本体只经stdin进_四道闸摊开且不过不放行落账() {
        let (mut app, runner) = sub_app(&[
            ("subscription register", 0, SUB_REG_OK),
            ("subscription register", 0, SUB_REG_OK),
            ("subscription ls", 0, SUB_LS_ALL),
        ]);
        let link = "https://airport.example/subscribe?token=SUBSECRET-56";
        tap(&mut app, KeyCode::Char('r'));
        assert_eq!(app.sub_flow.phase, RegPhase::Form);
        fill_sub_form(&mut app, "airport", "Mihomo", "2099-03-01", link);
        tap(&mut app, KeyCode::Enter); // 预审(不带 --yes,文件不落)
        let root = app.usable_data_root().unwrap_or_default();
        let pre = app.fed_queue.last().cloned().unwrap_or_default();
        assert_eq!(
            pre.0,
            vec![
                "subscription".to_string(),
                "register".to_string(),
                "--name".to_string(),
                "airport".to_string(),
                "--client".to_string(),
                "Mihomo".to_string(),
                "--token-stdin".to_string(),
                "--expires".to_string(),
                "2099-03-01".to_string(),
                "--root".to_string(),
                root.clone(),
            ],
            "argv 与 nw subscription register 契约一致"
        );
        assert!(
            !pre.0.iter().any(|a| a == "--yes"),
            "预审段连 --yes 的位置都没有(文件不落)"
        );
        assert!(
            !pre.0.iter().any(|a| a.contains("https://")),
            "argv 面无明文链接:本体只从 stdin 进:{:?}",
            pre.0
        );
        assert!(
            pre.0.iter().any(|a| a == "--token-stdin"),
            "--token-stdin 是「本体从 stdin 进」的声明,不是载荷"
        );
        // 本体确有一次到达运行器(喂入队列里那份与表单里那份等长)
        assert_eq!(app.fed_queue.len(), 1, "预审这一次喂入");
        feed(&mut app);
        assert_eq!(runner.stdin_feed_count(), 1, "本体经 stdin 喂过一次");
        assert_eq!(app.sub_flow.phase, RegPhase::Review, "闸结论摊开供审阅");
        // 落账段 = 预审 + 恰好一枚 --yes,并**再喂一次** stdin
        let s = crate::testutil::frame_display(&app, 80, 30);
        for id in [
            "subscription.register.name",
            "subscription.register.client",
            "subscription.register.expires",
            "subscription.register.body",
        ] {
            assert!(s.contains(id), "闸 {id} 应摊开:{s}");
        }
        assert!(s.contains("预审通过"), "{s}");
        assert!(!s.contains("SUBSECRET"), "屏显任何位置都不出现本体:{s}");
        assert!(
            !app.log.iter().any(|l| l.contains("SUBSECRET")),
            "日志不回显本体"
        );
        tap(&mut app, KeyCode::Enter);
        let com = app.fed_queue.last().cloned().unwrap_or_default();
        assert_eq!(
            com.0,
            [pre.0.clone(), vec!["--yes".into()]].concat(),
            "落账 = 预审 + --yes"
        );
        assert_eq!(com.0.iter().filter(|a| *a == "--yes").count(), 1);
        assert!(app.assembling, "落账段点亮写互斥");
        feed(&mut app);
        assert_eq!(
            runner.stdin_feed_count(),
            2,
            "两段式各喂一次(喂入是一次性的)"
        );
        assert_eq!(app.sub_flow.phase, RegPhase::Done, "落账回音呈现");
        assert!(!app.assembling, "写互斥随回音收敛");
        feed(&mut app); // 落账成功后即时重读订阅册(只读)
        assert_eq!(app.sub_len(), 4, "订阅册已刷新");
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("已写入"), "{s}");
        assert!(!s.contains("SUBSECRET"), "结果屏也没有本体:{s}");
    }

    /// 任一闸不过 = 一个字节都不落:预审失败态呈现四道闸原文,且面板**没有**
    /// 第二条写命令可发(确认屏不给「Enter 落账」)。
    #[test]
    fn 订阅登记_闸有fail_一个字节都不落_且没有落账段() {
        let (mut app, runner) = sub_app(&[("subscription register", 3, SUB_REG_FAIL)]);
        tap(&mut app, KeyCode::Char('r'));
        // client 填满(面板侧不拒),到期日给形状对但日历上不存在的一天:
        // 面板只校形状,真日期由引擎第③道闸判——这一趟就是要看引擎拒。
        fill_sub_form(
            &mut app,
            "airport",
            "Mihomo",
            "2099-02-30",
            "https://x/sub?token=T",
        );
        tap(&mut app, KeyCode::Enter);
        assert_eq!(app.fed_queue.len(), 1, "只有一次喂入");
        feed(&mut app);
        assert_eq!(runner.stdin_feed_count(), 1);
        assert_eq!(app.sub_flow.phase, RegPhase::Review, "闸不过留在确认态");
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(
            s.contains("一个字节都没落") || s.contains("不可落账"),
            "{s}"
        );
        assert!(
            s.contains("subscription.register.expires"),
            "失败的那道闸在场:{s}"
        );
        assert!(s.contains("第③道闸没过"), "证据五字段原样:{s}");
        assert!(!s.contains("Enter 落账"), "闸不过不放行,也没有落账段:{s}");
        assert!(!app.assembling, "写互斥随回音收敛");
        assert!(app.fed_queue.is_empty(), "没有第二条写命令可发");
    }

    /// 机密纪律**真的守**(不是注释守):喂入的链接标记不进最近日志、不进渲染
    /// 缓冲、不进 config、不进 `EngineOutput` 的 Debug 串,`SecretBody` 自己的
    /// `Debug` 也不打印内容(照 #55 已立的那条测试的形状)。
    #[test]
    fn 机密纪律_喂入的链接本体不进日志渲染配置与回音() {
        let marker = "https://airport.example/sub?token=SUPERSECRET-56-XYZ";
        let (mut app, _runner) = sub_app(&[("subscription register", 0, SUB_REG_OK)]);
        // ① 外包类型自己的 Debug 不打印内容
        let mut body = SecretBody::default();
        for c in marker.chars() {
            on_key_secret(
                &mut body,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            );
        }
        assert_eq!(body.len(), marker.len(), "字节数可报(唯一可说出的事实)");
        assert!(
            !format!("{body:?}").contains(marker),
            "SecretBody 的 Debug 不打印内容:{body:?}"
        );
        assert!(format!("{body:?}").contains("字节"), "只报字节数:{body:?}");
        // ② 走完整条登记流程,再查三面 + 回音 Debug
        app.sub_flow.begin();
        app.sub_form.name = "airport".into();
        app.sub_form.client = "Mihomo".into();
        app.sub_form.body = body;
        tap(&mut app, KeyCode::Enter);
        feed(&mut app);
        assert!(
            app.log.iter().all(|l| !l.contains(marker)),
            "最近日志不回显本体:{:?}",
            app.log.first()
        );
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(!s.contains(marker), "渲染缓冲不回显本体:{s}");
        let (cfg, _) = crate::engine::config::load();
        assert!(!format!("{cfg:?}").contains(marker), "配置不落本体");
        let out = app.sub_flow.result.as_ref().unwrap();
        assert!(
            !format!("{out:?}").contains(marker),
            "EngineOutput 的 Debug 串不回显本体"
        );
        // ③ 流程结束即 drop:返航后表单里不留副本
        app.cancel_sub_register();
        assert!(app.sub_form.body.is_empty(), "返航即 drop,面板侧不留副本");
    }

    /// **流程结束即擦字节**(ADR-0009 §3,2026-10-12 业主裁断)的交接一头:
    /// `into_bytes` 把字节**全部**交给 runner,之后明文只存在于接手的那个
    /// `Vec<u8>` 里——由 runner 在 `write_all` 之后 `fill(0)` 收口(另一头)。
    /// 表单里那份已被 move 进来,不会同时留两份。
    ///
    /// `Drop` 里那句 `fill(0)` 本身没有单元测试:要观测它就得在 `drop` 之后读
    /// 已释放的堆,那是未定义行为。它的存在由 `impl Drop for SecretBody` 与那段
    /// 注释锚着,后人要删会先撞上注释。
    #[test]
    fn into_bytes_交出全部字节_之后明文只在接手的那一份里() {
        let mut body = SecretBody::default();
        for b in [0x41u8, 0x42, 0x43] {
            on_key_secret(
                &mut body,
                KeyEvent::new(KeyCode::Char(b as char), KeyModifiers::NONE),
            );
        }
        assert_eq!(body.len(), 3, "先喂三个字节");
        let handed = body.into_bytes();
        assert_eq!(handed, vec![0x41u8, 0x42, 0x43], "字节原样交出,一个不少");
        // `body` 到此已被 move:面板侧不再有第二份。交给 runner 的那份喂完即擦。
    }

    /// ls 投影**无链接槽**:三态与剩余天数按 UTC、由引擎在回音里给足,面板不
    /// 自己算一套;过期不改退出码(整份回音 ok:true)。
    #[test]
    fn ls投影无链接槽_三态与剩余天数按引擎回音_过期不改退出码() {
        let (mut app, _r) = sub_app(&[("subscription ls", 0, SUB_LS_ALL)]);
        tap(&mut app, KeyCode::Char('l'));
        assert_eq!(
            app.queue.last().map(|a| a.join(" ")).as_deref(),
            Some(
                format!(
                    "subscription ls --root {}",
                    app.usable_data_root().unwrap_or_default()
                )
                .as_str()
            )
        );
        feed(&mut app);
        let d = app.sub_ls.as_ref().expect("回音可定型");
        // 结构上没有链接槽:投影的 Debug 串里不可能出现链接
        let sheet = d.sheet();
        assert!(
            !format!("{sheet:?}").contains("https://"),
            "ls 投影无 url 槽、不印指纹:{sheet:?}"
        );
        // 过期条目也在册体里(只标记不治理),退出码没被它改动
        let gone = sheet.entries.iter().find(|e| e.name == "gone").unwrap();
        assert_eq!(gone.status.as_deref(), Some("expired"));
        assert_eq!(
            gone.days_left,
            Some(-2000),
            "剩余天数可为负(过去 = expired)"
        );
        let forever = sheet.entries.iter().find(|e| e.name == "forever").unwrap();
        assert_eq!(forever.expires_line(), "永不过期", "没给到期日 = 永不过期");
        assert_eq!(forever.days_left, None, "daysLeft = null(键恒在)");
        assert_eq!(forever.days_left_line(), "——", "缺字段显式「——」不猜");
        // 汇总一行:几条 active · 几条 expiring · 几条 expired
        let c = d.status_counts();
        assert_eq!((c.active, c.expiring, c.expired, c.unknown), (2, 1, 1, 0));
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("2 active"), "汇总 active:{s}");
        assert!(s.contains("1 expiring"), "汇总 expiring:{s}");
        assert!(s.contains("1 expired"), "汇总 expired:{s}");
        assert!(s.contains("gone"), "过期条目在场(只标记不治理):{s}");
        assert!(s.contains("剩 -2000 天"), "剩余天数原样:{s}");
        assert!(!s.contains("https://"), "册体不出现链接:{s}");
    }

    /// rm 两态可预测:删不存在的名给证据且文件一个字节不动(不刷新册体);
    /// 删光最后一条留空数组(册体回到合法空态,不删文件)。
    #[test]
    fn 删订阅_删不存在的名给证据且不动文件_删光留空数组() {
        // ① 删不存在的名:rc 3 + 证据 + 文件一个字节都不动
        let (mut miss, _r) = sub_app(&[
            ("subscription ls", 0, SUB_LS_ALL),
            ("subscription rm", 3, SUB_RM_MISS),
        ]);
        tap(&mut miss, KeyCode::Char('l'));
        feed(&mut miss);
        assert_eq!(miss.sub_len(), 4);
        tap(&mut miss, KeyCode::Down); // 选中一条(排序后首行是 airport)
        tap(&mut miss, KeyCode::Char('d'));
        assert!(miss.sub_rm_confirm, "面板加一道图形确认(ADR-0009 §2)");
        assert!(
            miss.queue
                .iter()
                .all(|a| a.get(1).map(String::as_str) != Some("rm")),
            "确认前不下发"
        );
        tap(&mut miss, KeyCode::Enter);
        let argv = miss.queue.last().cloned().unwrap_or_default();
        assert_eq!(argv[1], "rm", "下发 rm");
        assert!(
            !argv.iter().any(|a| a == "--yes"),
            "上游 rm 没有确认闸,面板不造一枚"
        );
        feed(&mut miss);
        assert!(
            miss.log.iter().any(|l| l.contains("一个字节都没动")),
            "删不存在的名给证据:{:?}",
            miss.log.first()
        );
        assert!(
            miss.queue
                .iter()
                .all(|a| a.get(1).map(String::as_str) != Some("ls")),
            "没删成:不刷新册体"
        );
        assert_eq!(miss.sub_len(), 4, "册体未被刷新覆盖");
        // ② 删光最后一条:remaining 0 → 面板即时重读(只读),册体回到合法空态
        // (文件留空 entries 数组,不删文件;两种态都可预测)
        let (mut app, _r) = sub_app(&[
            ("subscription rm", 0, SUB_RM_LAST),
            ("subscription ls", 0, SUB_LS_EMPTY_ENV),
        ]);
        app.sub_ls = Some(serde_json::from_str(SUB_LS_ONE).unwrap());
        assert_eq!(app.sub_len(), 1, "删前册里有一条");
        app.sub_sel = 0;
        app.sub_rm_confirm = true;
        tap(&mut app, KeyCode::Enter);
        feed(&mut app); // rm 回音
        feed(&mut app); // 删后即时重读(只读)
        assert_eq!(app.sub_len(), 0, "删光后册体是合法空态(不是错误)");
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("合法空态"), "{s}");
        assert!(s.contains("subscriptions.json"), "说清是哪一本:{s}");
    }

    /// rm 是写动作:下发即点亮写互斥(ADR-0009 §4,与 register 落账段同一处
    /// 点灯)。在途窗口里两个写不并发(装配仪式打不开)、`q` 不禁用前不退;
    /// 回音到达(删成 / 删不存在的名 rc 3 都算到达)互斥即释放。
    #[test]
    fn 删订阅_下发即点亮写互斥_在途不接装配q禁用_回音后释放() {
        let (mut app, _r) = sub_app(&[
            ("subscription ls", 0, SUB_LS_ALL),
            ("subscription rm", 0, SUB_RM_LAST),
        ]);
        tap(&mut app, KeyCode::Char('l'));
        feed(&mut app);
        assert_eq!(app.sub_len(), 4);
        app.sub_sel = 0;
        tap(&mut app, KeyCode::Char('d'));
        assert!(app.sub_rm_confirm, "面板加一道图形确认(ADR-0009 §2)");
        assert!(!app.assembling, "确认前不点灯:命令还没递出去");
        tap(&mut app, KeyCode::Enter);
        assert!(app.assembling, "rm 下发即点亮写互斥(ADR-0009 §4)");
        assert!(
            app.queue
                .iter()
                .any(|a| a.get(1).map(String::as_str) == Some("rm")),
            "rm 已入队"
        );
        // 在途互斥:装配屏 Enter 打不开发射仪式(rm 在途时 up 不并发)
        app.ls = Some(
            serde_json::from_str(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[{"lockId":"9f8e","recipe":"research-desk","bakedAt":"2026-10-01"}]}"#,
            )
            .unwrap(),
        );
        app.screen = crate::app::Screen::Assemble;
        tap(&mut app, KeyCode::Enter);
        assert!(!app.as_confirm, "rm 在途:发射仪式打不开(写互斥)");
        assert!(
            app.queue
                .iter()
                .all(|a| a.first().map(String::as_str) != Some("up")),
            "rm 在途:up 不入队(两个写不并发)"
        );
        // 写互斥期间 q 禁用
        app.screen = crate::app::Screen::Vps;
        tap(&mut app, KeyCode::Char('q'));
        assert!(!app.quit, "写互斥期间 q 不禁用");
        // 回音到达:互斥释放,q 恢复受理
        feed(&mut app);
        assert!(!app.assembling, "rm 回音到达后互斥释放");
        feed(&mut app); // 删后即时重读(只读)
        tap(&mut app, KeyCode::Char('q'));
        assert!(app.quit, "互斥释放后 q 恢复受理");
    }

    /// 四本册的 rm 下发都点亮写互斥、回音都灭(ADR-0009 §4 对四本册一视
    /// 同仁:册级状态复制了四份,互斥语义不许漏掉任何一本——评审发现 2
    /// 正是四本册的 dispatch 全都没点灯)。
    #[test]
    fn 四本册rm下发都点亮写互斥_回音到达都释放() {
        // 机器册
        let mut app =
            crate::app::App::with_runner(std::sync::Arc::new(crate::engine::FakeRunner::new(&[
                ("vps rm", 0, VPS_RM_LAST),
                ("vps ls", 0, VPS_LS_GATE_ONLY),
            ])));
        app.splash = false;
        app.screen = crate::app::Screen::Vps;
        app.capabilities.record(
            crate::engine::Capability::Vps,
            crate::engine::Probe::Available,
        );
        app.vps_ls = Some(serde_json::from_str(CAT_VPS_LS).unwrap());
        tap(&mut app, KeyCode::Char('d'));
        tap(&mut app, KeyCode::Enter);
        assert!(app.assembling, "机器册 rm 下发点灯");
        feed(&mut app);
        assert!(!app.assembling, "机器册 rm 回音灭灯");
        // 标签册(删标签要走一遍表单:Enter 提交 → 确认框 → Enter 下发)
        let mut app = tag_app(&[("tag rm", 0, TAG_RM_LAST), ("tag ls", 0, TAG_LS_ALL)]);
        app.tag_ls_echo = Some(crate::engine::EngineOutput {
            args: vec!["tag".into(), "ls".into()],
            exit_code: crate::engine::ExitCode::Ok,
            ok: true,
            stderr_tail: vec![],
            envelope: Some(serde_json::from_str(TAG_LS_ALL).unwrap()),
            raw_stdout: TAG_LS_ALL.into(),
            parse_error: None,
        });
        tap(&mut app, KeyCode::Char('d'));
        tap(&mut app, KeyCode::Enter); // 表单提交(引用留空 = 删整个标签)
        assert!(app.tag_rm_confirm, "面板加一道图形确认");
        tap(&mut app, KeyCode::Enter); // 确认下发
        assert!(app.assembling, "标签册 rm 下发点灯");
        feed(&mut app);
        assert!(!app.assembling, "标签册 rm 回音灭灯");
        // 订阅册
        let (mut app, _r) = sub_app(&[
            ("subscription rm", 0, SUB_RM_LAST),
            ("subscription ls", 0, SUB_LS_EMPTY_ENV),
        ]);
        app.sub_ls = Some(serde_json::from_str(SUB_LS_ONE).unwrap());
        tap(&mut app, KeyCode::Char('d'));
        tap(&mut app, KeyCode::Enter);
        assert!(app.assembling, "订阅册 rm 下发点灯");
        feed(&mut app);
        assert!(!app.assembling, "订阅册 rm 回音灭灯");
        // 钥匙册
        let (mut app, _r) = key_app(&[("key rm", 0, KEY_RM_LAST), ("key ls", 0, KEY_LS_ONE)]);
        app.key_ls = Some(serde_json::from_str(KEY_LS_ONE).unwrap());
        tap(&mut app, KeyCode::Char('d'));
        tap(&mut app, KeyCode::Enter);
        assert!(app.assembling, "钥匙册 rm 下发点灯");
        feed(&mut app);
        assert!(!app.assembling, "钥匙册 rm 回音灭灯");
    }

    /// 取回链接本体:**只给建议命令**。面板不代跑 `subscription get`——队列里
    /// 永远不会出现它;建议命令与 bake / recipe new 同源同层(commands.rs 纯函数层),
    /// 永不带写旗标。
    #[test]
    fn 取回链接_只给建议命令_面板无任何路径会下发它() {
        let (mut app, runner) = sub_app(&[("subscription ls", 0, SUB_LS_ALL)]);
        tap(&mut app, KeyCode::Char('l'));
        feed(&mut app);
        let suggest = app.sub_get_suggest().expect("选中一条就能算出建议命令");
        assert_eq!(
            suggest.display,
            format!(
                "nw subscription get --name airport --root {} | jq -r .data.url",
                app.usable_data_root().unwrap_or_default()
            ),
            "形态照上游 usage:要裸链接自己拼管道"
        );
        assert!(
            !suggest.display.contains("--yes"),
            "get 是只读出口,永不带写旗标"
        );
        assert!(
            !suggest.display.contains("--raw"),
            "上游不开 --raw,面板也不造"
        );
        // 册体把建议命令与理由都摊开给人看(取回链接那一行给足宽度)
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("面板不代跑"), "{s}");
        assert!(
            s.contains("jq -r .data.url"),
            "建议命令完整在场(不被标签挤掉):{s}"
        );
        assert!(!s.contains("https://"), "册体不出现链接:{s}");
        // 按 g 收进日志(可取用),但队列里一个字节都不多
        assert!(!app.sub_get_logged);
        tap(&mut app, KeyCode::Char('g'));
        assert!(app.sub_get_logged, "g 把建议命令收进日志");
        assert!(
            app.log.iter().any(|l| l.contains("jq -r .data.url")),
            "建议命令收进日志可取用:{:?}",
            app.log.first()
        );
        assert!(app.queue.is_empty(), "面板不代跑 get:队列里没有它");
        assert_eq!(runner.stdin_feed_count(), 0, "没有真实子进程被拉起喂入");
        // 键位扫一遍:没有任何一条路径会把 subscription get 入队
        for c in ['g', 'l', 'm', 'k', 'j'] {
            tap(&mut app, KeyCode::Char(c));
        }
        for code in [KeyCode::Up, KeyCode::Down, KeyCode::Esc] {
            tap(&mut app, code);
        }
        assert!(
            app.queue
                .iter()
                .chain(app.fed_queue.iter().map(|(a, _)| a))
                .all(|a| a.get(1).map(String::as_str) != Some("get")),
            "任何路径都不会下发 subscription get:{:?}",
            app.queue
        );
    }

    /// 复用 #53 的骨架:三本册共用同一个列表件(`catalog_sheet_lines`),订阅册
    /// 只换 [`EntryDetail`](明细画法)——不复制第二份投影 / 列表 / 落盘逻辑。
    #[test]
    fn 四本册共用列表件_订阅册只换明细画法_且m四轮转() {
        let sheet = CatalogSheet {
            catalog_path: "/r/subscriptions.json".into(),
            entries: vec![CatalogEntryView {
                name: "airport".into(),
                client: Some("Mihomo".into()),
                expires: Some("2099-03-01".into()),
                status: Some("expiring".into()),
                days_left: Some(3),
                ..Default::default()
            }],
            gates: Vec::new(),
        };
        let p = crate::theme::cached_degrade(
            crate::theme::ThemeKind::Warehouse,
            crate::term::ColorCap::Truecolor,
        );
        let text = |detail: EntryDetail| -> String {
            catalog_sheet_lines(
                &sheet,
                &(0..sheet.entries.len()).collect::<Vec<_>>(),
                60,
                0,
                &p,
                detail,
            )
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
        };
        let sub = text(sub_detail_rows);
        assert!(sub.contains("应用 Mihomo"), "订阅册:{sub}");
        assert!(sub.contains("2099-03-01"), "到期日在场:{sub}");
        assert!(sub.contains("expiring"), "三态用引擎原词:{sub}");
        assert!(sub.contains("剩 3 天"), "剩余天数在场:{sub}");
        assert!(
            sub.contains("警"),
            "视觉映射到结果四态(expiring = 警):{sub}"
        );
        assert!(!sub.contains("端口"), "订阅册不画端口槽:{sub}");
        assert!(!sub.contains("引用"), "订阅册不画引用槽:{sub}");
        // 同一份投影,机器册与标签册的画法不画订阅槽
        assert!(
            !text(vps_detail_rows).contains("应用"),
            "机器册不画 client 槽"
        );
        assert!(!text(tag_detail_rows).contains("到期"), "标签册不画到期槽");
        // m 四轮转(屏只有一块,册子是屏内维度)
        let (mut app, _r) = sub_app(&[]);
        let mut kinds = vec![app.catalog_kind];
        for _ in 0..3 {
            app.cycle_catalog_kind();
            kinds.push(app.catalog_kind);
        }
        assert_eq!(
            kinds,
            vec![
                CatalogKind::Subscription,
                CatalogKind::Key,
                CatalogKind::Vps,
                CatalogKind::Tag
            ],
            "m 换册四轮转(订阅 → 钥匙 → 机器 → 标签),不复制第二块屏"
        );
    }

    /// 表单面板侧就拒:空名字 / 空 client / 到期日不是严格 YYYY-MM-DD /
    /// 本体没喂入——都不喂给引擎跑一趟,留表单可改。
    #[test]
    fn 表单解析_坏到期日与未喂入_面板侧就拒() {
        let mut f = new_sub_form();
        f.name = "airport".into();
        f.client = "Mihomo".into();
        f.body = SecretBody(vec![b'h', b't', b't', b'p']);
        let spec = spec_from_sub_form(&f);
        assert!(spec.is_ok(), "到期日留空 = 永不过期:{spec:?}");
        assert_eq!(
            spec.unwrap().expires,
            None,
            "留空 = 永不过期,不下发 --expires"
        );
        // 形状不对(非严格 YYYY-MM-DD)
        for bad_date in ["2026-3-1", "01/02/2026", "2026-02-30x", "20260301"] {
            assert!(!valid_expires(bad_date), "{bad_date} 应被拒");
        }
        for ok_date in ["2026-03-01", "", " 2099-12-31 "] {
            assert!(valid_expires(ok_date), "{ok_date} 应认");
        }
        f.expires = "2026-3-1".into();
        assert!(spec_from_sub_form(&f).is_err(), "坏到期日不下发");
        // 本体没喂入
        let mut g = new_sub_form();
        g.name = "airport".into();
        g.client = "Mihomo".into();
        let errs = spec_from_sub_form(&g).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("还没喂入")), "{errs:?}");
    }

    /// 订阅册屏常畅态 / 紧凑态两档都成立(#56 验收):不引新断点、不缩放字号,
    /// 66×50 紧凑态下也不溢出、不 panic,键位行 / 汇总 / 边界表述常在。
    #[test]
    fn 订阅册屏_常畅与紧凑两档都渲染不溢出() {
        let (mut app, _r) = sub_app(&[("subscription ls", 0, SUB_LS_ALL)]);
        tap(&mut app, KeyCode::Char('l'));
        feed(&mut app);
        for (w, h) in [(80u16, 30u16), (66, 50), (100, 40)] {
            let s = crate::testutil::frame_display(&app, w, h);
            assert!(s.contains("当前册 订阅"), "{w}×{h} 当前是哪一本应在场:{s}");
            assert!(s.contains("airport"), "{w}×{h} 订阅名应在场");
            assert!(s.contains("应用 Mihomo"), "{w}×{h} client 行应在场");
            assert!(s.contains("active"), "{w}×{h} 三态应在场");
            assert!(s.contains("绝不进配方"), "{w}×{h} 边界表述应在场:{s}");
            assert!(s.contains("m 换册"), "{w}×{h} 键位行应在场:{s}");
            for ln in s.lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 有行超宽:{ln}"
                );
            }
        }
    }

    /// 能力门缺 subscription 时:入口禁用并指路,册体不呈现(不自造替代品)。
    #[test]
    fn 能力门缺subscription_入口禁用并指路() {
        let (mut app, _r) = sub_app(&[]);
        app.capabilities.record(
            crate::engine::Capability::Subscription,
            crate::engine::Probe::Unavailable,
        );
        assert!(!app.sub_entry_enabled());
        tap(&mut app, KeyCode::Char('r'));
        assert!(app.fed_queue.is_empty(), "门不过不下发任何命令");
        assert!(app.sub_flow.phase == RegPhase::Idle, "门不过不进表单");
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("入口禁用"), "{s}");
        assert!(s.contains("缺的是引擎能力"), "{s}");
    }

    // ---- 工单 #57:钥匙册(本体只经 stdin / 投影无本体槽不印指纹 / get 只给建议) ----

    /// 钥匙 ls 回音:两把钥匙,一把声明了作用域、一把没有;`kind` / `host` /
    /// `scopes[]` / `rotatedAt` 五键冻结——**没有 secret 槽、也没有指纹字段**。
    const KEY_LS_ALL: &str = r#"{"schema":"nw/key-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/keys.json","keys":[{"name":"ci","kind":"github-key","host":"ghe.internal","scopes":[],"rotatedAt":"2025-11-02"},{"name":"deploy","kind":"github-key","host":"github.com","scopes":["repo","read:org"],"rotatedAt":"2026-01-15"}]}}"#;

    /// 登记预审回音:三道闸全 pass、`written:false`(不带 `--yes`,文件不落)。
    /// **data 里一个能放本体的字段都没有**(上游 RegisterResult 刻意不留 secret 槽)。
    const KEY_REG_OK: &str = r#"{"schema":"nw/key-register.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"name":"deploy","kind":"github-key","host":"github.com","scopes":["repo"],"rotatedAt":"2026-03-01","catalogPath":"/r/keys.json","written":false,"updated":false,"action":"registered","gates":[{"id":"key.register.name","status":"pass","message":"钥匙名合法"},{"id":"key.register.host","status":"pass","message":"host 是合法主机名/IP"},{"id":"key.register.body","status":"pass","message":"本体非空、单行"}]}}"#;

    /// 登记预审失败回音:第②道闸(host)不过 → 一个字节都不落,第③道 skip。
    /// 主角是「github.com:22」:端口不进 host 字段(面板不校 host 形状,交给引擎)。
    const KEY_REG_FAIL: &str = r#"{"schema":"nw/key-register.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"第②道闸没过:host 带端口","hint":"host 只登记主机名/IP,端口由使用这把钥匙的地方自己带","details":{"gate":"key.register.host"}}],"warnings":[],"data":{"name":"deploy","kind":"github-key","host":"","scopes":[],"rotatedAt":"","catalogPath":"/r/keys.json","written":false,"updated":false,"action":"rejected","gates":[{"id":"key.register.name","status":"pass","message":"钥匙名合法"},{"id":"key.register.host","status":"fail","message":"host 带端口"},{"id":"key.register.body","status":"skip","message":"前面已拒"}]}}"#;

    /// 删一条钥匙成功:删光最后一条,`remaining: 0`(文件留空 entries 数组)。
    const KEY_RM_LAST: &str = r#"{"schema":"nw/key-rm.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"name":"ci","catalogPath":"/r/keys.json","removed":true,"written":true,"action":"removed","remaining":0}}"#;

    /// 删不存在的钥匙:rc 3 + 证据五字段 + 文件一个字节都不动。
    const KEY_RM_MISS: &str = r#"{"schema":"nw/key-rm.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"key \"ghost\" 不在 keys.json 里","hint":"keys.json 是机器写的产物,永不手改","details":{"catalogPath":"/r/keys.json"}}],"warnings":[],"data":{"name":"ghost","catalogPath":"/r/keys.json","removed":false,"written":false,"action":"not_found","remaining":2}}"#;

    /// key ls 的 **data 面**(直接灌 `key_ls` 状态用):只剩一条。
    const KEY_LS_ONE: &str = r#"{"catalogPath":"/r/keys.json","keys":[{"name":"ci","kind":"github-key","host":"ghe.internal","scopes":[],"rotatedAt":"2025-11-02"}]}"#;

    /// 删光最后一条之后的 ls 回音:空数组(合法空态,文件不删)。
    const KEY_LS_EMPTY_ENV: &str = r#"{"schema":"nw/key-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/keys.json","keys":[]}}"#;

    /// 一台能力齐备的钥匙册夹具:引擎有 key 组、数据仓根可用。连同 runner
    /// 一起交还(要数「收到过几次 stdin 喂入」)。
    fn key_app(
        rules: &[(&str, i32, &str)],
    ) -> (crate::app::App, std::sync::Arc<crate::engine::FakeRunner>) {
        let runner = std::sync::Arc::new(crate::engine::FakeRunner::new(rules));
        let mut app = crate::app::App::with_runner(runner.clone());
        app.splash = false;
        app.screen = crate::app::Screen::Vps;
        app.catalog_kind = CatalogKind::Key;
        app.capabilities.record(
            crate::engine::Capability::Key,
            crate::engine::Probe::Available,
        );
        (app, runner)
    }

    /// 在钥匙册表单里填名字/主机,跳过作用域(留空 = 未声明),填轮换日,在钥匙框
    /// 喂入本体(掩码字段)。
    fn fill_key_form(
        app: &mut crate::app::App,
        name: &str,
        host: &str,
        rotated: &str,
        token: &str,
    ) {
        for c in name.chars() {
            tap(app, KeyCode::Char(c));
        }
        tap(app, KeyCode::Down); // 主机
        for c in host.chars() {
            tap(app, KeyCode::Char(c));
        }
        tap(app, KeyCode::Down); // 作用域(留空 = 未声明,只登记不设闸)
        tap(app, KeyCode::Down); // 轮换日
        for c in rotated.chars() {
            tap(app, KeyCode::Char(c));
        }
        tap(app, KeyCode::Down); // 钥匙(掩码)框
        for c in token.chars() {
            tap(app, KeyCode::Char(c));
        }
    }

    /// 登记走两段式:钥匙本体只经 stdin 进(argv 面无明文形参),三道闸摊开,
    /// 任一不过 = 一个字节都不落、面板不放行落账。
    #[tokio::test]
    async fn 钥匙登记_两段式_本体只经stdin进_三道闸摊开() {
        let (mut app, runner) = key_app(&[
            ("key register", 0, KEY_REG_OK),
            ("key register", 0, KEY_REG_OK),
            ("key ls", 0, KEY_LS_ALL),
        ]);
        let token = "ghp_SuperSecretToken-57-XYZ";
        tap(&mut app, KeyCode::Char('r'));
        assert_eq!(app.key_flow.phase, RegPhase::Form);
        fill_key_form(&mut app, "deploy", "github.com", "2026-03-01", token);
        tap(&mut app, KeyCode::Enter); // 预审(不带 --yes,文件不落)
        let root = app.usable_data_root().unwrap_or_default();
        let pre = app.fed_queue.last().cloned().unwrap_or_default();
        assert_eq!(
            pre.0,
            vec![
                "key".to_string(),
                "register".to_string(),
                "--name".to_string(),
                "deploy".to_string(),
                "--token-stdin".to_string(),
                "--host".to_string(),
                "github.com".to_string(),
                "--rotated-at".to_string(),
                "2026-03-01".to_string(),
                "--root".to_string(),
                root.clone(),
            ],
            "argv 与 nw key register 契约一致"
        );
        assert!(
            !pre.0.iter().any(|a| a == "--yes"),
            "预审段连 --yes 的位置都没有"
        );
        assert!(
            !pre.0.iter().any(|a| a.contains("ghp_")),
            "argv 面无明文钥匙:本体只从 stdin 进:{:?}",
            pre.0
        );
        assert!(
            pre.0.iter().any(|a| a == "--token-stdin"),
            "--token-stdin 是声明不是载荷"
        );
        assert_eq!(app.fed_queue.len(), 1, "预审这一次喂入");
        feed(&mut app);
        assert_eq!(runner.stdin_feed_count(), 1, "本体经 stdin 喂过一次");
        assert_eq!(app.key_flow.phase, RegPhase::Review, "闸结论摊开供审阅");
        let s = crate::testutil::frame_display(&app, 80, 30);
        for id in [
            "key.register.name",
            "key.register.host",
            "key.register.body",
        ] {
            assert!(s.contains(id), "闸 {id} 应摊开:{s}");
        }
        assert!(s.contains("预审通过"), "{s}");
        assert!(!s.contains(token), "屏显任何位置都不出现本体:{s}");
        assert!(!app.log.iter().any(|l| l.contains(token)), "日志不回显本体");
        // 落账段 = 预审 + 恰好一枚 --yes,并**再喂一次** stdin
        tap(&mut app, KeyCode::Enter);
        let com = app.fed_queue.last().cloned().unwrap_or_default();
        assert_eq!(
            com.0,
            [pre.0.clone(), vec!["--yes".into()]].concat(),
            "落账 = 预审 + --yes"
        );
        assert_eq!(com.0.iter().filter(|a| *a == "--yes").count(), 1);
        assert!(app.assembling, "落账段点亮写互斥");
        feed(&mut app);
        assert_eq!(
            runner.stdin_feed_count(),
            2,
            "两段式各喂一次(喂入是一次性的)"
        );
        assert_eq!(app.key_flow.phase, RegPhase::Done, "落账回音呈现");
        assert!(!app.assembling, "写互斥随回音收敛");
        feed(&mut app); // 落账成功后即时重读钥匙册(只读)
        assert_eq!(app.key_len(), 2, "钥匙册已刷新");
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("已写入"), "{s}");
        assert!(!s.contains(token), "结果屏也没有本体:{s}");
    }

    /// 任一闸不过 = 一个字节都不落:预审失败态呈现三道闸原文,且面板**没有**
    /// 第二条写命令可发(确认屏不给「Enter 落账」)。
    #[tokio::test]
    async fn 钥匙登记_闸有fail_一个字节都不落_且没有落账段() {
        let (mut app, runner) = key_app(&[("key register", 3, KEY_REG_FAIL)]);
        tap(&mut app, KeyCode::Char('r'));
        // host 填「github.com:22」(面板不校 host 形状,交给引擎第②道闸按端口拒)
        fill_key_form(&mut app, "deploy", "github.com:22", "2026-03-01", "ghp_x");
        tap(&mut app, KeyCode::Enter);
        assert_eq!(app.fed_queue.len(), 1, "只有一次喂入");
        feed(&mut app);
        assert_eq!(runner.stdin_feed_count(), 1);
        assert_eq!(app.key_flow.phase, RegPhase::Review, "闸不过留在确认态");
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(
            s.contains("一个字节都没落") || s.contains("不可落账"),
            "{s}"
        );
        assert!(s.contains("key.register.host"), "失败的那道闸在场:{s}");
        assert!(s.contains("第②道闸没过"), "证据五字段原样:{s}");
        assert!(!s.contains("Enter 落账"), "闸不过不放行,也没有落账段:{s}");
        assert!(!app.assembling, "写互斥随回音收敛");
        assert!(app.fed_queue.is_empty(), "没有第二条写命令可发");
    }

    /// 机密纪律**真的守**(不是注释守):喂入的钥匙标记不进最近日志、不进渲染
    /// 缓冲、不进 config、不进 `EngineOutput` 的 Debug 串,`SecretBody` 自己的
    /// `Debug` 也不打印内容(照 #56 已立的那条测试的形状)。
    #[test]
    fn 机密纪律_喂入的钥匙本体不进日志渲染配置与回音() {
        let marker = "ghp_SuperSecretToken-57-XYZ-marker";
        let (mut app, _runner) = key_app(&[("key register", 0, KEY_REG_OK)]);
        // ① 外包类型自己的 Debug 不打印内容
        let mut body = SecretBody::default();
        for c in marker.chars() {
            on_key_secret(
                &mut body,
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            );
        }
        assert_eq!(body.len(), marker.len(), "字节数可报(唯一可说出的事实)");
        assert!(
            !format!("{body:?}").contains(marker),
            "SecretBody 的 Debug 不打印内容:{body:?}"
        );
        assert!(format!("{body:?}").contains("字节"), "只报字节数:{body:?}");
        // ② 走完整条登记流程,再查三面 + 回音 Debug
        app.key_flow.begin();
        app.key_form.name = "deploy".into();
        app.key_form.host = "github.com".into();
        app.key_form.body = body;
        tap(&mut app, KeyCode::Enter);
        feed(&mut app);
        assert!(
            app.log.iter().all(|l| !l.contains(marker)),
            "最近日志不回显本体:{:?}",
            app.log.first()
        );
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(!s.contains(marker), "渲染缓冲不回显本体:{s}");
        let (cfg, _) = crate::engine::config::load();
        assert!(!format!("{cfg:?}").contains(marker), "配置不落本体");
        let out = app.key_flow.result.as_ref().unwrap();
        assert!(
            !format!("{out:?}").contains(marker),
            "EngineOutput 的 Debug 串不回显本体"
        );
        // ③ 流程结束即 drop:返航后表单里不留副本
        app.cancel_key_register();
        assert!(app.key_form.body.is_empty(), "返航即 drop,面板侧不留副本");
    }

    /// ls 投影**无本体槽、不印指纹**:结构上放不下 secret,面板连「这像是哪把
    /// 钥匙」都不该让人推断出来;作用域原样逐条报、轮换日原样(只登记不设闸)。
    #[test]
    fn ls投影无本体槽不印指纹_作用域轮换日只登记不设闸() {
        let (mut app, _r) = key_app(&[("key ls", 0, KEY_LS_ALL)]);
        tap(&mut app, KeyCode::Char('l'));
        assert_eq!(
            app.queue.last().map(|a| a.join(" ")).as_deref(),
            Some(
                format!(
                    "key ls --root {}",
                    app.usable_data_root().unwrap_or_default()
                )
                .as_str()
            )
        );
        feed(&mut app);
        let d = app.key_ls.as_ref().expect("回音可定型");
        let sheet = d.sheet();
        // 结构上没有本体槽:投影的 Debug 串里不可能出现令牌/指纹
        assert!(
            !format!("{sheet:?}").contains("ghp_"),
            "ls 投影无本体、不印指纹:{sheet:?}"
        );
        let deploy = sheet.entries.iter().find(|e| e.name == "deploy").unwrap();
        assert_eq!(deploy.kind.as_deref(), Some("github-key"));
        assert_eq!(deploy.host.as_deref(), Some("github.com"));
        assert_eq!(
            deploy.scopes_line(),
            "repo · read:org",
            "作用域原样逐条报(登记元数据,与 secret 槽的不印指纹是两回事)"
        );
        assert_eq!(deploy.rotated_at_line(), "2026-01-15", "轮换日原样呈现");
        let ci = sheet.entries.iter().find(|e| e.name == "ci").unwrap();
        assert_eq!(
            ci.scopes_line(),
            "未声明(0 条)",
            "没给作用域显式未声明,不猜"
        );
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("deploy"), "{s}");
        assert!(s.contains("github.com"), "{s}");
        assert!(s.contains("read:org"), "作用域报值:{s}");
        assert!(!s.contains("ghp_"), "册体不出现本体/指纹:{s}");
    }

    /// rm 两态可预测:删不存在的名给证据且文件一个字节不动(不刷新册体);删光
    /// 最后一条留空数组(册体回到合法空态,不删文件)。
    #[test]
    fn 删钥匙_删不存在的名给证据且不动文件_删光留空数组() {
        // ① 删不存在的名:rc 3 + 证据 + 文件一个字节都不动
        let (mut miss, _r) = key_app(&[("key ls", 0, KEY_LS_ALL), ("key rm", 3, KEY_RM_MISS)]);
        tap(&mut miss, KeyCode::Char('l'));
        feed(&mut miss);
        assert_eq!(miss.key_len(), 2);
        tap(&mut miss, KeyCode::Down); // 选中一把
        tap(&mut miss, KeyCode::Char('d'));
        assert!(miss.key_rm_confirm, "面板加一道图形确认(ADR-0009 §2)");
        assert!(
            miss.queue
                .iter()
                .all(|a| a.get(1).map(String::as_str) != Some("rm")),
            "确认前不下发"
        );
        tap(&mut miss, KeyCode::Enter);
        let argv = miss.queue.last().cloned().unwrap_or_default();
        assert_eq!(argv[1], "rm", "下发 rm");
        assert!(
            !argv.iter().any(|a| a == "--yes"),
            "上游 rm 没有确认闸,面板不造一枚"
        );
        feed(&mut miss);
        assert!(
            miss.log.iter().any(|l| l.contains("一个字节都没动")),
            "删不存在的名给证据:{:?}",
            miss.log.first()
        );
        assert!(
            miss.queue
                .iter()
                .all(|a| a.get(1).map(String::as_str) != Some("ls")),
            "没删成:不刷新册体"
        );
        assert_eq!(miss.key_len(), 2, "册体未被刷新覆盖");
        // ② 删光最后一条:remaining 0 → 面板即时重读(只读),册体回到合法空态
        let (mut app, _r) = key_app(&[("key rm", 0, KEY_RM_LAST), ("key ls", 0, KEY_LS_EMPTY_ENV)]);
        app.key_ls = Some(serde_json::from_str(KEY_LS_ONE).unwrap());
        assert_eq!(app.key_len(), 1, "删前册里有一条");
        app.key_sel = 0;
        app.key_rm_confirm = true;
        tap(&mut app, KeyCode::Enter);
        feed(&mut app); // rm 回音
        feed(&mut app); // 删后即时重读(只读)
        assert_eq!(app.key_len(), 0, "删光后册体是合法空态(不是错误)");
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("合法空态"), "{s}");
        assert!(s.contains("keys.json"), "说清是哪一本:{s}");
    }

    /// 取回钥匙本体:**只给建议命令**。面板不代跑 `key get`——队列里永远不会
    /// 出现它;建议命令与 bake / recipe new / subscription get 同源同层
    /// (commands.rs 纯函数层),永不带写旗标。
    #[test]
    fn 取回钥匙_只给建议命令_面板无任何路径会下发它() {
        let (mut app, runner) = key_app(&[("key ls", 0, KEY_LS_ALL)]);
        tap(&mut app, KeyCode::Char('l'));
        feed(&mut app);
        let suggest = app.key_get_suggest().expect("选中一条就能算出建议命令");
        assert_eq!(
            suggest.display,
            format!(
                "nw key get --name ci --root {} | jq -r .data.key",
                app.usable_data_root().unwrap_or_default()
            ),
            "形态照上游 usage:要裸钥匙自己拼管道"
        );
        assert!(
            !suggest.display.contains("--yes"),
            "get 是只读出口,永不带写旗标"
        );
        assert!(
            !suggest.display.contains("--raw"),
            "上游不开 --raw,面板也不造"
        );
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("面板不代跑"), "{s}");
        assert!(s.contains("jq -r .data.key"), "建议命令完整在场:{s}");
        assert!(!s.contains("ghp_"), "册体不出现本体:{s}");
        assert!(!app.key_get_logged);
        tap(&mut app, KeyCode::Char('g'));
        assert!(app.key_get_logged, "g 把建议命令收进日志");
        assert!(
            app.log.iter().any(|l| l.contains("jq -r .data.key")),
            "建议命令收进日志可取用:{:?}",
            app.log.first()
        );
        assert!(app.queue.is_empty(), "面板不代跑 get:队列里没有它");
        assert_eq!(runner.stdin_feed_count(), 0, "没有真实子进程被拉起喂入");
        // 键位扫一遍:没有任何一条路径会把 key get 入队
        for c in ['g', 'l', 'm', 'k', 'j'] {
            tap(&mut app, KeyCode::Char(c));
        }
        for code in [KeyCode::Up, KeyCode::Down, KeyCode::Esc] {
            tap(&mut app, code);
        }
        assert!(
            app.queue
                .iter()
                .chain(app.fed_queue.iter().map(|(a, _)| a))
                .all(|a| a.get(1).map(String::as_str) != Some("get")),
            "任何路径都不会下发 key get:{:?}",
            app.queue
        );
    }

    /// 复用 #53 的骨架:四本册共用同一个列表件(`catalog_sheet_lines`),钥匙册
    /// 只换 [`EntryDetail`](明细画法)——不复制第二份投影 / 列表 / 落盘逻辑;m 四轮转。
    #[test]
    fn 四本册共用列表件_钥匙册只换明细画法_且m四轮转() {
        let sheet = CatalogSheet {
            catalog_path: "/r/keys.json".into(),
            entries: vec![CatalogEntryView {
                name: "deploy".into(),
                kind: Some("github-key".into()),
                host: Some("github.com".into()),
                scopes: vec!["repo".into(), "read:org".into()],
                rotated_at: Some("2026-01-15".into()),
                ..Default::default()
            }],
            gates: Vec::new(),
        };
        let p = crate::theme::cached_degrade(
            crate::theme::ThemeKind::Warehouse,
            crate::term::ColorCap::Truecolor,
        );
        let text = |detail: EntryDetail| -> String {
            catalog_sheet_lines(
                &sheet,
                &(0..sheet.entries.len()).collect::<Vec<_>>(),
                60,
                0,
                &p,
                detail,
            )
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
        };
        let key = text(key_detail_rows);
        assert!(key.contains("kind github-key"), "钥匙册:{key}");
        assert!(key.contains("github.com"), "钥匙册:{key}");
        assert!(key.contains("read:org"), "作用域报值:{key}");
        assert!(key.contains("2026-01-15"), "轮换日在场:{key}");
        assert!(!key.contains("端口"), "钥匙册不画端口槽:{key}");
        assert!(!key.contains("引用"), "钥匙册不画引用槽:{key}");
        // 同一份投影,机器册的画法不画 kind 槽
        assert!(
            !text(vps_detail_rows).contains("kind"),
            "机器册不画 kind 槽"
        );
        // m 四轮转(屏只有一块,册子是屏内维度)
        let (mut app, _r) = key_app(&[]);
        let mut kinds = vec![app.catalog_kind];
        for _ in 0..4 {
            app.cycle_catalog_kind();
            kinds.push(app.catalog_kind);
        }
        assert_eq!(
            kinds,
            vec![
                CatalogKind::Key,
                CatalogKind::Vps,
                CatalogKind::Tag,
                CatalogKind::Subscription,
                CatalogKind::Key
            ],
            "m 换册四轮转,不复制第二块屏"
        );
    }

    /// 表单面板侧就拒:空名字 / 轮换日不是严格 YYYY-MM-DD / 本体没喂入——都不喂给
    /// 引擎跑一趟,留表单可改。作用域空行滤掉(只登记不设闸)。
    #[test]
    fn 表单解析_坏轮换日与未喂入_面板侧就拒() {
        let mut f = new_key_form();
        f.name = "deploy".into();
        f.host = "github.com".into();
        f.body = SecretBody(vec![b'g', b'h', b'p']);
        // 轮换日留空 = 缺省登记日 UTC,合法
        let spec = spec_from_key_form(&f);
        assert!(spec.is_ok(), "轮换日留空 = 缺省登记日 UTC:{spec:?}");
        assert_eq!(spec.unwrap().rotated_at, None, "留空不下发 --rotated-at");
        // 形状不对(非严格 YYYY-MM-DD)
        for bad_date in ["2026-3-1", "01/02/2026", "2026-02-30x", "20260301"] {
            f.rotated_at = bad_date.into();
            assert!(spec_from_key_form(&f).is_err(), "{bad_date} 应被拒");
        }
        f.rotated_at = "2026-03-01".into();
        assert!(spec_from_key_form(&f).is_ok(), "好轮换日应认");
        // 本体没喂入
        let mut g = new_key_form();
        g.name = "deploy".into();
        let errs = spec_from_key_form(&g).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("还没喂入")), "{errs:?}");
        // 作用域:一行一条,空行 = 没填这条(不是坏行);非空行原样收(只登记不设闸)
        f.scopes = vec!["repo".into(), "  ".into(), "read:org".into()];
        let spec = spec_from_key_form(&f).unwrap();
        assert_eq!(
            spec.scopes,
            vec!["repo".to_string(), "read:org".to_string()],
            "空行滤掉,好行原样收"
        );
    }

    /// 钥匙册屏常畅态 / 紧凑态两档都成立(#57 验收):不引新断点、不缩放字号,
    /// 66×50 紧凑态下也不溢出、不 panic,键位行 / 边界表述常在。
    #[test]
    fn 钥匙册屏_常畅与紧凑两档都渲染不溢出() {
        let (mut app, _r) = key_app(&[("key ls", 0, KEY_LS_ALL)]);
        tap(&mut app, KeyCode::Char('l'));
        feed(&mut app);
        for (w, h) in [(80u16, 30u16), (66, 50), (100, 40)] {
            let s = crate::testutil::frame_display(&app, w, h);
            assert!(s.contains("当前册 钥匙"), "{w}×{h} 当前是哪一本应在场:{s}");
            assert!(s.contains("deploy"), "{w}×{h} 钥匙名应在场");
            assert!(s.contains("github.com"), "{w}×{h} 主机行应在场");
            assert!(s.contains("绝不进配方"), "{w}×{h} 边界表述应在场:{s}");
            assert!(s.contains("m 换册"), "{w}×{h} 键位行应在场:{s}");
            for ln in s.lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 有行超宽:{ln}"
                );
            }
        }
    }

    /// 能力门缺 key 时:入口禁用并指路,册体不呈现(不自造替代品)。
    #[test]
    fn 能力门缺key_入口禁用并指路() {
        let (mut app, _r) = key_app(&[]);
        app.capabilities.record(
            crate::engine::Capability::Key,
            crate::engine::Probe::Unavailable,
        );
        assert!(!app.key_entry_enabled());
        tap(&mut app, KeyCode::Char('r'));
        assert!(app.fed_queue.is_empty(), "门不过不下发任何命令");
        assert!(app.key_flow.phase == RegPhase::Idle, "门不过不进表单");
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("入口禁用"), "{s}");
        assert!(s.contains("缺的是引擎能力"), "{s}");
    }

    // ---- 工单 #70:反查上浮(机器册上一个键问「什么标签引用了这一条」) ----

    /// 反查匹配**纯函数**的口径(#70 验收第 5 条,spec #65 故事 41):同时存在
    /// 一台叫 `gate` 的机器与一条引用 `skill/gate` 的标签(名字也叫 gate)时,
    /// 反查机器只返回 `vps/gate` 那条引用——kind 与条目两侧都**精确比对**,
    /// 一条 `skill/gate` 不会因为条目名撞车被算进来;`vps/gateway` 也不会因
    /// 子串包含被算进来。这一个函数就是本轮最该被钉住的口径。
    #[test]
    fn 反查匹配纯函数_gate机器不串skill_gate_只认vps_gate精确() {
        // 标签册投影:web 引用 vps/gate(真命中);gate 引用 skill/gate(条目撞车,
        // kind 不同,不命中);edge 引用 vps/gateway(子串,不命中)。
        let data: crate::engine::TagLsData = serde_json::from_str(
            r#"{"catalogPath":"/r/tags.json","tags":[
                {"name":"web","refs":[{"kind":"vps","id":"gate"}]},
                {"name":"gate","refs":[{"kind":"skill","id":"gate"}]},
                {"name":"edge","refs":[{"kind":"vps","id":"gateway"}]}
            ]}"#,
        )
        .unwrap();
        let sheet = data.sheet();
        let hits = crate::screens::catalog::tags_referring(&sheet, "vps", "gate");
        let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["web"],
            "只有真正引用 vps/gate 的 web 命中;skill/gate 与 vps/gateway 都不算"
        );
        assert_eq!(
            hits[0].refs,
            vec![crate::engine::CatalogRef {
                kind: "vps".into(),
                id: "gate".into()
            }],
            "命中标签带上各自引用(原样,指针不是密钥)"
        );
    }

    /// 反查匹配纯函数:一台机器可被多个标签引用,全部返回且保持引擎的确定性
    /// 排序(面板不重排);一个都不引时给空(由调用方画成「一条都没有」)。
    #[test]
    fn 反查匹配纯函数_多标签命中保序_零命中给空() {
        let data: crate::engine::TagLsData = serde_json::from_str(
            r#"{"catalogPath":"/r/tags.json","tags":[
                {"name":"prod","refs":[{"kind":"vps","id":"gate"}]},
                {"name":"bare","refs":[]},
                {"name":"edge","refs":[{"kind":"vps","id":"gate"},{"kind":"vps","id":"bare"}]}
            ]}"#,
        )
        .unwrap();
        let sheet = data.sheet();
        let hits = crate::screens::catalog::tags_referring(&sheet, "vps", "gate");
        assert_eq!(
            hits.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(),
            vec!["edge", "prod"],
            "两个引用 vps/gate 的标签都命中,保持投影(确定性排序后)的顺序"
        );
        assert!(
            crate::screens::catalog::tags_referring(&sheet, "vps", "ghost").is_empty(),
            "没有标签引用 ghost:零命中给空(调用方画成「一条都没有」)"
        );
    }

    // ---- 工单 #70:反查上浮的行为测试(公开缝:update(Key/Mouse) 驱动) ----
    //
    // 全部走在公开缝上:`App::update(Message::Key / Message::Mouse)` 驱动一个
    // `FakeRunner`,断言落在 `frame_display` 帧、`app.log` 人话行、选中下标、
    // `hitmap()` 命中目标与 `FakeRunner::call_argvs()` / `abort_count()` 探针上
    // ——不断言私有函数签名、不断言内部字段名。上面的 `反查匹配纯函数_*` 钉纯
    // 函数口径,这一族钉「一个键问下去之后操作者看见什么、引擎收到什么」。

    /// 反查上浮的**三标签夹具**(#70 规格验收 1):prod / edge / pi 三条都
    /// 引用 `vps/gate`(各自引用原样在场);另放一条**名字也叫 gate** 的标签,
    /// 它引用的是 `skill/gate`——条目名撞车,反查机器时不该被算进来(纯函数
    /// 那一族已钉口径,这里在帧上复核)。prod 多带一条 `skill/dr` 引用,用来
    /// 钉「命中标签的**各自**引用原样上场,不只印名字」。
    const TAG_LS_GATE3: &str = r#"{"schema":"nw/tag-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/tags.json","tags":[{"name":"prod","refs":[{"kind":"vps","id":"gate"},{"kind":"skill","id":"dr"}]},{"name":"edge","refs":[{"kind":"vps","id":"gate"}]},{"name":"pi","refs":[{"kind":"vps","id":"gate"}]},{"name":"gate","refs":[{"kind":"skill","id":"gate"}]}]}}"#;

    /// 反查上浮的**机器册夹具**(#70):gate 带一条登录指针 `$DEPLOY_KEY`
    /// (用来钉「结果不印机密本体」——投影里就只有指针,面板也不该多印),
    /// 另留一台 bare(零标签,给「一条都没有」那一态)。
    const REVERSE_MACHINES: &str = r#"[{"name":"gate","ips":[{"label":"home","addr":"10.0.0.1"}],"port":22,"logins":[{"user":"root","ref":"$DEPLOY_KEY"}],"note":"门口"},{"name":"bare","ips":[],"port":0,"logins":[]}]"#;

    /// 标签册 **rc 3** 回音(#70 规格验收 3 / 故事 34:读不到,不是空结果)。
    /// 刻意让 `data` 仍是**可定型**的形状(`TagLsData` 拆得出来)——这正是
    /// 最容易蒙混过关的那一种:只凭「拆不拆得出型」判定,就会把「这次没读到」
    /// 画成「一条标签引用都没有」,两态互相冒充(spec #65 铁律 4)。
    const TAG_LS_RC3: &str = r#"{"schema":"nw/tag-ls.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"tags.json 读不到:数据仓根下没有这个文件","hint":"先 nw tag add <标签> --ref <kind>/<条目> 挂一个","details":{"catalogPath":"/r/tags.json"}}],"warnings":[],"data":{"catalogPath":"/r/tags.json","tags":[]}}"#;

    /// 契约守卫合成「需升级 nw」的回音(#70 读不到第三态):schema 面板不认识,
    /// `data_as` 拆不出型——绝不按旧形状降级解析,也不画成空列表。
    const TAG_LS_GUARD: &str = r#"{"schema":"nw/tag-ls.v9","nw":"9.9.9","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/tags.json","tags":[]}}"#;

    /// 建一本就绪的机器册(#70 反查上浮的行为夹具)。`machines` 是 VpsLsData
    /// 的 machines JSON 数组;`rules` 是 FakeRunner 的回放规则——反查只读走
    /// `tag ls` 无参全列,把标签册夹具喂在这里;`root` = 数据仓根(`None` =
    /// 未设);`tag_cap` = 标签组能力可不可用。后两个参数让**门禁两态**可分别
    /// 自造:门禁顺序钉死「先根、再能力、最后才发那一次只读调用」,任缺一条
    /// 给各自指路且**一次引擎调用都不发**。
    ///
    /// 返回 runner 是为了探针:`call_argvs()` 是「发没发命令」唯一可观察的
    /// 证据(FakeRunner 只记 argv、绝不存 stdin 本体,守「面板侧不留副本」铁律)。
    fn reverse_app(
        machines: &str,
        rules: &[(&str, i32, &str)],
        root: Option<&str>,
        tag_cap: bool,
    ) -> (std::sync::Arc<crate::engine::FakeRunner>, crate::app::App) {
        let ls = format!(r#"{{"catalogPath":"/r/vps.json","machines":{machines}}}"#);
        let mut all: Vec<(&str, i32, &str)> = vec![("vps ls", 0, ls.as_str())];
        all.extend_from_slice(rules);
        let runner = std::sync::Arc::new(crate::engine::FakeRunner::new(&all));
        let mut app = crate::app::App::with_runner(runner.clone());
        app.splash = false;
        app.screen = crate::app::Screen::Vps;
        app.capabilities.record(
            crate::engine::Capability::Vps,
            crate::engine::Probe::Available,
        );
        if tag_cap {
            app.capabilities.record(
                crate::engine::Capability::Tag,
                crate::engine::Probe::Available,
            );
        }
        app.vps_ls = Some(serde_json::from_str(ls.as_str()).unwrap());
        app.data_root = root.map(str::to_string);
        (runner, app)
    }

    /// 反查上浮两门俱开的就绪机器册(上面各族的行为夹具都走这一档)。
    fn reverse_ready(
        machines: &str,
        rules: &[(&str, i32, &str)],
    ) -> (std::sync::Arc<crate::engine::FakeRunner>, crate::app::App) {
        // 数据仓根钉成临时目录:门禁第一道(根)因此确确过得去,第二道(能力)
        // 由 `tag_cap` 另造。不钉就会随跑测试那台机器的配置漂,门禁测不准。
        let root = crate::testutil::stable_root();
        reverse_app(machines, rules, Some(&root), true)
    }

    /// 反查上浮在机器册选中行上的键盘触发(规格故事 31):Enter 问「什么标签
    /// 引用了这一条」。抽出来是因为四族测试都要走这一下,而这一下必须与
    /// 鼠标触发完全同路(见 `反查触发段_有鼠标命中_点它等于按键`)。
    fn 反查这台(app: &mut crate::app::App) {
        tap(app, KeyCode::Enter);
    }

    /// 直接灌一条 `tag ls` 回音(不经 FakeRunner):用来造「迟到的第二条回音」
    /// ——它确确实实是一次引擎回音(带 argv、带回音信封),只是落在操作者
    /// 已返航之后。`data` 是 TagLsData 的 data 面 JSON,`rc` 是退出码。
    fn feed_tag_echo(app: &mut crate::app::App, data: &str, rc: i32) {
        let env = format!(
            r#"{{"schema":"nw/tag-ls.v1","nw":"0.1.0","ok":{ok},"errors":[],"warnings":[],"data":{data}}}"#,
            ok = rc == 0
        );
        let out = crate::engine::EngineOutput {
            args: vec![
                "tag".into(),
                "ls".into(),
                "--root".into(),
                crate::testutil::stable_root(),
            ],
            exit_code: crate::engine::ExitCode::from_number(rc),
            ok: rc == 0,
            stderr_tail: vec![],
            envelope: serde_json::from_str(&env).ok(),
            raw_stdout: env,
            parse_error: None,
        };
        app.update(crate::app::Message::Engine(
            crate::engine::EngineEvent::Finished(Box::new(out)),
        ));
    }

    /// 规格验收 1 / 故事 32:一台挂了 3 个标签的机器,三个标签名与各自引用
    /// **都在帧上**;条目名撞车的 `skill/gate` 不混进来。
    #[test]
    fn 反查_三个标签与各自引用都在帧上() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app, KeyCode::Down); // 投影按名排序:首行 bare,第二行才是 gate
        assert_eq!(app.catalog_sel(), 1, "选中 gate");
        反查这台(&mut app);
        feed(&mut app);
        let s = crate::testutil::frame_display(&app, 80, 30);
        for name in ["prod", "edge", "pi"] {
            assert!(s.contains(name), "标签 {name} 应在浮层里:{s}");
        }
        assert!(s.contains("3 个标签引用了 gate"), "命中条数明说:{s}");
        assert!(s.contains("skill/dr"), "prod 的另一条引用原样在场:{s}");
        assert!(s.contains("Esc 或点击任意处关闭"), "收口给返航路:{s}");
        // 在途的那一次只读调用确有一次到达运行器,形状与 nw tag ls 契约一致
        assert_eq!(
            _r.call_argvs(),
            vec![vec![
                "tag".to_string(),
                "ls".to_string(),
                "--root".to_string(),
                crate::testutil::stable_root()
            ]],
            "反查复用既有的 tag ls 无参全列(不新增调用形状)"
        );
    }

    /// 规格验收 2 / 故事 33:一台没有标签的机器,**明说**「没有标签引用这一
    /// 条」并给「去挂第一个标签」的下一步——与「有 N 条」两态可区分,绝不
    /// 共用一个空列表。
    #[test]
    fn 反查_一条标签都没有_明说并给下一步() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        assert_eq!(app.catalog_sel(), 0, "投影按名排序:首行 bare");
        反查这台(&mut app);
        feed(&mut app);
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("没有标签引用 bare"), "明说一条都没有:{s}");
        assert!(s.contains("挂第一个标签"), "给下一步:{s}");
        assert!(!s.contains("个标签引用了"), "不是有 N 条那一态:{s}");
    }

    /// 规格验收 5 / 故事 41 的**帧上层**:同时存在一台叫 `gate` 的机器与一条
    /// 叫 `gate` 的技能标签时,反查机器只摊开引用 `vps/gate` 的那几条——帧上
    /// 出现 `skill/gate` 就是匹配退化成了子串包含(纯函数那一族钉口径,这一
    /// 条钉「口径真的走到了屏上」)。
    #[test]
    fn 反查_机器与技能同名_帧上只有vps_gate的引用() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app, KeyCode::Down); // 投影按名排序:首行 bare,第二行才是 gate
        反查这台(&mut app);
        feed(&mut app);
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("3 个标签引用了 gate"), "只数出三条:{s}");
        assert!(
            !s.contains("skill/gate"),
            "引用 skill/gate 的那条标签不上屏(同名不串味):{s}"
        );
        assert!(!s.contains("4 个标签"), "不是四条:同名的那条没被算进来:{s}");
    }

    /// 规格验收 3 / 故事 34:标签册回 rc 3,**走指路、不画成空列表**。
    /// rc 3 的 `data` 仍是可定型形状——只看「拆不拆得出型」就会把它当成
    /// 合法空册,于是「这次没读到」冒充「一条标签引用都没有」。帧上因此既
    /// 没有「没有标签引用」那套话,也没有「挂第一个标签」的下一步;指路
    /// 落在日志行里(人话原文 + 说清不是空结果)。
    #[test]
    fn 反查_标签册回rc3_走指路不画成空列表() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 3, TAG_LS_RC3)]);
        反查这台(&mut app);
        feed(&mut app);
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(
            s.contains("反查没结果"),
            "帧上给指路,不是一片空白(按了 Enter 屏上没动静 = 当成面板没反应):{s}"
        );
        assert!(
            s.contains("不是「一条标签引用都没有」"),
            "说清读不到与没打过标签是两件事:{s}"
        );
        assert!(
            !s.contains("没有标签引用"),
            "rc 3 是读不到,绝不是「一条都没有」:{s}"
        );
        assert!(
            !s.contains("挂第一个标签"),
            "读不到不给挂标签的下一步(那会把没读到骗成没打过标签):{s}"
        );
        assert!(
            app.log.iter().any(|l| l.contains("反查没结果")),
            "指路同时落在日志人话里:{:?}",
            app.log
        );
        assert!(
            app.log.iter().any(|l| l.contains("tags.json 读不到")),
            "引擎的原文证据在场,不替它改写:{:?}",
            app.log
        );
    }

    /// 规格验收 3 的另一半:契约守卫合成「需升级 nw」(schema 面板不认识)
    /// 同样走指路,不画成空列表。这一路 `data_as` 拆不出型,与 rc 3 那一
    /// 路形状不同,但落到操作者眼里必须是同一件事:**没读到**。
    #[test]
    fn 反查_契约守卫需升级nw_走指路不画成空列表() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GUARD)]);
        反查这台(&mut app);
        feed(&mut app);
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("反查没结果"), "帧上给指路,不画成空列表:{s}");
        assert!(!s.contains("没有标签引用"), "读不到不是空列表:{s}");
        assert!(
            app.log.iter().any(|l| l.contains("反查没结果")),
            "指路落在日志人话里:{:?}",
            app.log
        );
        assert!(
            app.log.iter().any(|l| l.contains("需升级 nw")),
            "说清是两端 schema 没对齐,不是数据没了:{:?}",
            app.log
        );
    }

    /// 规格验收 4 / 故事 40:数据仓根**未设**时给数据仓根指路(帧上 + 日志),
    /// 且**一次引擎调用都没发**。夹具里明明摆着能答对的 `tag ls` 回放规则,
    /// 所以「没发」不是夹具答不上,是面板自己没收口。
    #[test]
    fn 反查门禁_数据仓根未设_指路且零引擎调用() {
        let (runner, mut app) =
            reverse_app(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)], None, true);
        反查这台(&mut app);
        assert!(
            runner.call_argvs().is_empty(),
            "根未设:一条 argv 都不发(探针是空的):{:?}",
            runner.call_argvs()
        );
        assert!(
            app.log.iter().any(|l| l.contains("反查未下发")),
            "日志给指路:{:?}",
            app.log
        );
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("数据仓根未设"), "帧上也给人话:{s}");
        assert!(!s.contains("没有标签引用"), "门禁不过不进结果浮层:{s}");
    }

    /// 规格验收 4 / 故事 39:标签组能力不可用时给能力指路(缺的是引擎能力,
    /// 不是面板坏了),同样**零引擎调用**。数据仓根这一道是过的——所以这一
    /// 条钉的是第二道门真的在管事,不是被第一道顺手挡掉的。
    #[test]
    fn 反查门禁_标签组能力不可用_指路且零引擎调用() {
        let root = crate::testutil::stable_root();
        let (runner, mut app) = reverse_app(
            REVERSE_MACHINES,
            &[("tag ls", 0, TAG_LS_GATE3)],
            Some(&root),
            false,
        );
        反查这台(&mut app);
        assert!(
            runner.call_argvs().is_empty(),
            "能力不可用:一条 argv 都不发:{:?}",
            runner.call_argvs()
        );
        assert!(
            app.log.iter().any(|l| l.contains("缺的是引擎能力")),
            "说清缺的是引擎能力,不是面板坏了:{:?}",
            app.log
        );
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(!s.contains("没有标签引用"), "门禁不过不进结果浮层:{s}");
    }

    /// 故事 39 的**鼠标那一侧**(「反查要求标签组能力可用,缺能力时入口禁用并
    /// 指路」):标签组能力缺席时「Enter 反查」段**照画、照给命中**,画成 dim
    /// (禁用态);点它仍走 `App::vps_reverse_lookup` 的同一条门禁——给人话、
    /// 一次引擎调用都不发。
    ///
    /// 与 [`反查门禁_标签组能力不可用_指路且零引擎调用`] 同一夹具、同一结论,
    /// 这一条只补键盘那一侧没覆盖的三件事:① 帧上真的有这一段(不是收掉了);
    /// ② 命中区真的有矩形,且矩形正是帧上这一段所在的那几列(点哪打哪);
    /// ③ 点下去仍然零调用、仍然给指路。禁用态的颜色不在这条里断言
    /// (`frame_display` 只拚符号、不带样式),由渲染侧 `key_line` 的分叉承载。
    #[test]
    fn 反查段_标签组能力缺席_段照画点得中_指路且零调用() {
        let root = crate::testutil::stable_root();
        let (runner, mut app) = reverse_app(
            REVERSE_MACHINES,
            &[("tag ls", 0, TAG_LS_GATE3)],
            Some(&root),
            false, // 标签组能力缺席(数据仓根这一道是过的)
        );
        tap(&mut app, KeyCode::Down); // 选中 gate:这一段只在册体有条目时给
        assert!(
            !app.tag_entry_enabled(),
            "夹具确确落在门禁不过那一侧(缺的是标签组能力)"
        );
        // ① 帧上在场:禁用 ≠ 消失(规格要的是「禁用并指路」)
        let f = crate::testutil::frame_display(&app, 80, 30);
        assert!(
            f.contains("Enter 反查"),
            "能力缺席也画这一段(画成禁用态):{f}"
        );
        // ② 命中区在场,且矩形就是帧上这一段所在的几列(同一份段清单,ADR-0005)
        let size = Rect::new(0, 0, 80, 30);
        app.size = size;
        let hit = crate::layout::hitmap(size, &app, crate::layout::size_class(size))
            .into_iter()
            .find(|(_, t)| matches!(t, crate::layout::Target::VpsReverse))
            .map(|(r, _)| r)
            .expect("能力缺席:反查段仍应有命中区(禁用并指路)");
        let row = f.lines().nth(hit.y as usize).expect("命中行在渲染中在场");
        let byte = row.find("Enter 反查").expect("帧上找得着这一段");
        let col: u16 = row[..byte]
            .chars()
            .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u16)
            .sum();
        assert!(
            hit.x <= col && col < hit.x + hit.width,
            "段起始列 {col} 落在命中矩形 {}..{} 内(点哪打哪)",
            hit.x,
            hit.x + hit.width
        );
        // ③ 点下去:同一条门禁、同一条指路、零引擎调用
        点中(&mut app, 80, 30, |t| {
            matches!(t, crate::layout::Target::VpsReverse)
        });
        assert!(
            runner.call_argvs().is_empty(),
            "点禁用态段也一条 argv 都不发:{:?}",
            runner.call_argvs()
        );
        assert!(
            app.log.iter().any(|l| l.contains("反查未下发")),
            "点了仍给指路:{:?}",
            app.log
        );
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(!s.contains("没有标签引用"), "门禁不过不进结果浮层:{s}");
    }

    /// 规格验收 4 的**门禁顺序**:根未设与能力不可用同时在场时,**根那句赢**
    /// (与面板其余入口同一信任序:先根、再能力),并且仍然是零引擎调用。
    /// 两条指路措辞互不冒充——操作者该先去设根,而不是先被告知去升级 nw。
    #[test]
    fn 反查门禁_根与能力双缺_根那句赢且零调用() {
        let (runner, mut app) = reverse_app(
            REVERSE_MACHINES,
            &[("tag ls", 0, TAG_LS_GATE3)],
            None,
            false,
        );
        反查这台(&mut app);
        assert!(
            runner.call_argvs().is_empty(),
            "两道门都不过:更是一条都不发:{:?}",
            runner.call_argvs()
        );
        let line = app
            .log
            .iter()
            .find(|l| l.contains("反查未下发"))
            .cloned()
            .unwrap_or_default();
        assert!(line.contains("五域读不到数据仓"), "根那句赢:{line}");
        assert!(
            !line.contains("缺的是引擎能力"),
            "不让能力那句抢在前面(两条指路互不冒充):{line}"
        );
        assert!(
            !line.contains("升级所配 nw"),
            "能力那句的下一步也不出现:{line}"
        );
    }

    /// 规格验收 6 / 故事 38:反查**只在机器册**提供。订阅册与钥匙册的条目
    /// 不在 `TAG_REF_KINDS` 里,在那里放这个键注定只返回「没有」——所以同一
    /// 个 `Enter` 在这两本上**无事可做**:不开浮层、不入在途、不排队一条
    /// `tag ls`。这里把两道门都开到过得去(根设了、tag 能力记成可用), 钉的
    /// 就是「没有这一行」而不是「门挡掉了」。
    #[test]
    fn 反查边界_订阅册与钥匙册上同一个键不做反查() {
        let root = crate::testutil::stable_root();
        for (name, mk) in [
            (
                "订阅",
                sub_app
                    as fn(
                        &[(&str, i32, &str)],
                    )
                        -> (crate::app::App, std::sync::Arc<crate::engine::FakeRunner>),
            ),
            ("钥匙", key_app),
        ] {
            let (mut app, runner) = mk(&[]);
            app.data_root = Some(root.clone());
            app.capabilities.record(
                crate::engine::Capability::Tag,
                crate::engine::Probe::Available,
            );
            tap(&mut app, KeyCode::Char('l')); // 先把册体读出来(有行可选)
            feed(&mut app);
            let before = runner.call_argvs();
            tap(&mut app, KeyCode::Down);
            tap(&mut app, KeyCode::Enter);
            assert!(!app.vps_reverse_open(), "{name}册上 Enter 不开反查浮层");
            assert_eq!(
                runner.call_argvs(),
                before,
                "{name}册上 Enter 不下发任何命令(连 tag ls 都不排)"
            );
        }
    }

    /// 规格验收 8 / 故事 42:反查**复用既有的只读调用形状**——队里那一条
    /// argv 逐字节就是 `nw tag ls --root <根>`(无参全列)。不新增调用形状、
    /// 不新增写路径,`tags.json` 因此一个字节都不会被碰。
    #[test]
    fn 反查_复用既有只读调用_队里argv逐字节是tag_ls_root() {
        let root = crate::testutil::stable_root();
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app, KeyCode::Down);
        反查这台(&mut app);
        assert_eq!(
            app.queue.last().cloned(),
            Some(vec![
                "tag".to_string(),
                "ls".to_string(),
                "--root".to_string(),
                root.clone()
            ]),
            "无参全列 + 只认 --root:与既有的 nw tag ls 逐字节同形"
        );
        assert!(
            app.queue
                .iter()
                .all(|a| a.first().map(String::as_str) != Some("tag")
                    || a.get(1) == Some(&"ls".to_string())),
            "反查这一路只有只读的 ls,没有任何写路径:{:?}",
            app.queue
        );
    }

    /// 规格验收 9 / 故事 43:反查结果**不印密钥引用、不印机密本体**。机器册
    /// 帧上本来就有 `$DEPLOY_KEY` 这个指针(那是登录槽的 ref,指针不是密钥),
    /// 所以断言要更严:浮层里只准出现 `<kind>/<条目>` 形式的标签引用,不准
    /// 出现任何 `$` 引用、任何形如地址/端口的本体。
    #[test]
    fn 反查结果_不印密钥引用不印机密本体() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app, KeyCode::Down);
        反查这台(&mut app);
        feed(&mut app);
        let s = crate::testutil::frame_display(&app, 80, 30);
        // 浮层那一块(取「反查 · 什么标签引用了这一条」到收口行之间)
        let popup: String = s
            .lines()
            .skip_while(|l| !l.contains("反查 · 什么标签引用了这一条"))
            .take_while(|l| !l.contains("Esc 或点击任意处关闭"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(popup.contains("prod"), "浮层确凿在场:{popup}");
        assert!(!popup.contains('$'), "浮层不印任何 $ 引用:{popup}");
        assert!(!popup.contains("10.0.0.1"), "浮层不印地址本体:{popup}");
        assert!(!popup.contains(":22"), "浮层不印端口:{popup}");
        assert!(!popup.contains("门口"), "浮层不印备注:{popup}");
        assert!(!popup.contains("root"), "浮层不印登录名:{popup}");
    }

    /// 规格验收 11:反查是**只读**,不并入 `assembling` 全局写互斥。按下去
    /// 之后 `assembling` 必须还是灭的——否则在途的一次反查会把登记 / 装配
    /// 一路全挡住(那是写闸,不是读闸)。
    #[test]
    fn 反查_是只读_不点亮assembling写互斥() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        assert!(!app.assembling, "起初没有写在途");
        tap(&mut app, KeyCode::Down);
        反查这台(&mut app);
        assert!(!app.assembling, "反查在途不点亮写互斥(只读,不占写闸)");
        assert_eq!(app.queue.len(), 1, "确有一次只读调用排队");
        feed(&mut app);
        assert!(!app.assembling, "反查落地也不点亮写互斥");
        // 反证:写互斥亮着(装配在途)时反查照样问得动。若把反查并进
        // `assembling`,这一下就会被互斥语挡掉,队里一条都不多。
        tap(&mut app, KeyCode::Esc); // 先返航,否则结果浮层吞键
        app.assembling = true;
        反查这台(&mut app);
        assert_eq!(app.queue.len(), 1, "装配在途也问得动(只读不占写闸)");
    }

    /// 规格验收 7 / 故事 36:反查**在途期间不拿上一轮结论冒充本次**。第一轮
    /// 问 bare 得到「一条都没有」,返航后换一台机器再问——在途那一刻屏上既
    /// 没有第一轮的结论,也没有本轮的回音(还没到)。`run_tag_ls` 因此把上一
    /// 轮的回音清空:留着它,「在途」就会长得像「有结果」。
    #[test]
    fn 反查_在途不拿上一轮结论冒充本次() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        反查这台(&mut app); // 第一轮:bare
        feed(&mut app);
        let a = crate::testutil::frame_display(&app, 80, 30);
        assert!(a.contains("没有标签引用 bare"), "第一轮落地:{a}");
        tap(&mut app, KeyCode::Esc); // 返航回机器册(选中行不动)
        tap(&mut app, KeyCode::Down); // 换一台机器问
        反查这台(&mut app); // 第二轮:在途
        let b = crate::testutil::frame_display(&app, 80, 30);
        assert!(!b.contains("没有标签引用 bare"), "在途不摆上一轮的结论:{b}");
        assert!(
            !b.contains("个标签引用了"),
            "在途也不摆本轮(回音还没到):{b}"
        );
    }

    /// 规格验收 7 的另一半:操作者已返航(Idle)之后,**迟到的回音不收敛任何
    /// 状态、不重开浮层**。这一条回音确确实实是一次引擎回音,只是落在操作
    /// 者已经看完走了之后——幽灵重开一个旧结果比不开更糟。
    #[test]
    fn 反查_迟到回音不重开浮层() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app, KeyCode::Down);
        反查这台(&mut app);
        feed(&mut app);
        assert!(app.vps_reverse_open(), "先落地");
        tap(&mut app, KeyCode::Esc);
        assert!(!app.vps_reverse_open(), "返航");
        // 迟到的第二条 tag ls 回音:同一台机器、另一份标签册
        feed_tag_echo(
            &mut app,
            r#"{"catalogPath":"/r/tags.json","tags":[{"name":"late","refs":[{"kind":"vps","id":"gate"}]}]}"#,
            0,
        );
        assert!(
            !app.vps_reverse_open(),
            "迟到回音不重开浮层(已返航 = 这一轮已经结束)"
        );
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(!s.contains("个标签引用了"), "迟到回音不上屏:{s}");
        assert!(!s.contains("late"), "迟到回音里的标签名也不上屏:{s}");
    }

    /// 规格验收 10 / 故事 44:反查触发段**有鼠标命中**,且点它 = 在机器册
    /// 选中行上按 `Enter`(两条路给同一件事:同一条 argv、同一次在途)。
    /// 段文案由渲染与命中共用(`catalog_key_parts` 一处产出),点哪打哪。
    #[test]
    fn 反查触发段_有鼠标命中_点它等于按键() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app, KeyCode::Down); // 选中 gate
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("Enter 反查"), "段在键位行上画着:{s}");
        点中(&mut app, 80, 30, |t| {
            matches!(t, crate::layout::Target::VpsReverse)
        });
        assert_eq!(
            app.queue.last().cloned(),
            Some(vec![
                "tag".to_string(),
                "ls".to_string(),
                "--root".to_string(),
                crate::testutil::stable_root()
            ]),
            "点段与按 Enter 下同一条只读 argv"
        );
        // 与键盘那条路逐字节同路
        let (_r2, mut app2) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app2, KeyCode::Down);
        反查这台(&mut app2);
        assert_eq!(app.queue, app2.queue, "鼠标与键盘两条路一条 argv");
    }

    /// 规格验收 10 / 故事 37:结果浮层 `Esc` 或**任意处点击**都关,且关掉
    /// 之后回到机器册**原选中行**。反查从头到尾不写 `vps_sel`,所以「回到
    /// 原选中行」不需要一次恢复动作,只需要不在返航时顺手改它——这条钉的
    /// 正是「没顺手改」。
    #[test]
    fn 反查浮层_键盘与任意点击都关_回到机器册原选中行() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app, KeyCode::Down);
        let sel = app.catalog_sel();
        assert_eq!(sel, 1, "选中 gate");
        反查这台(&mut app);
        feed(&mut app);
        assert!(app.vps_reverse_open(), "浮层在场");
        tap(&mut app, KeyCode::Esc);
        assert!(!app.vps_reverse_open(), "Esc 返航");
        assert_eq!(app.catalog_sel(), sel, "Esc 后选中行原样");
        // 再开一次:点浮层外的空白也算返航(规格 §7:任意处点击关闭)
        反查这台(&mut app);
        feed(&mut app);
        assert!(app.vps_reverse_open(), "再开一次");
        点中(&mut app, 80, 30, |t| {
            matches!(t, crate::layout::Target::DismissPopup)
        });
        assert!(!app.vps_reverse_open(), "点任意处返航");
        assert_eq!(app.catalog_sel(), sel, "点任意处后选中行仍原样");
    }

    /// 规格验收 10 的弹层纪律 / 故事 50:浮层打开时**底层不产命中**——清单
    /// 行与键位行三个可点段(反查 / 换册 / 过滤)一段都不给,点击不穿透。
    /// 命中的整个正文只有一个「点任意处返航」。
    #[test]
    fn 反查浮层_弹层优先级_底层清单行与键位段都不产命中() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app, KeyCode::Down);
        反查这台(&mut app);
        feed(&mut app);
        assert!(app.vps_reverse_open());
        app.size = Rect::new(0, 0, 80, 30);
        let hits = crate::layout::hitmap(app.size, &app, crate::layout::size_class(app.size));
        assert!(
            hits.iter()
                .any(|(_, t)| matches!(t, crate::layout::Target::DismissPopup)),
            "浮层自己给一个「点任意处返航」:{hits:?}"
        );
        assert!(
            hits.iter()
                .all(|(_, t)| !matches!(t, crate::layout::Target::SelectRow(_))),
            "底层清单行不产命中(点不到被浮层盖住的那一行)"
        );
        assert!(
            hits.iter().all(|(_, t)| !matches!(
                t,
                crate::layout::Target::VpsReverse
                    | crate::layout::Target::SwitchBook
                    | crate::layout::Target::CatalogFilter
            )),
            "键位行三个可点段都不产命中"
        );
    }

    /// #69 与 #70 合用(规格故事 27):过滤收窄到一台机器之后,反查点的是
    /// **看得见的那一台**——被滤掉的 bare 不该被问到,浮层点名的必须是 gate。
    /// 选中 / 鼠标 / 反查全部经过可见下标映射回原始投影,同一口径。
    #[test]
    fn 反查与册内过滤合用_浮层点名看得见的那一台() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        过滤敲词(&mut app, "gate"); // 只留 gate(bare 被滤掉)
        assert_eq!(app.catalog_visible().len(), 1, "收窄到一台");
        反查这台(&mut app);
        feed(&mut app);
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("3 个标签引用了 gate"), "点名看得见的那一台:{s}");
        assert!(!s.contains("没有标签引用"), "滤掉的 bare 不被问到:{s}");
    }

    /// 两档响应式(#70 验收的常畅 / 紧凑):80×30 与 66×50(外加 100×40)下,
    /// 触发段与结果浮层都画得出来,且**没有一行超宽**。紧凑态放不下整段时
    /// 渲染与命中同源收口(屏上没有的段,鼠标也点不到)。浮层盖住键位行是
    /// 弹层纪律,所以「关着看段、开着看浮层」分两次量。
    #[test]
    fn 反查浮层与触发段_常畅与紧凑两档都渲染不溢出() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app, KeyCode::Down);
        for (w, h) in [(80u16, 30u16), (66, 50), (100, 40)] {
            // 关着:触发段在键位行上,且没有浮层
            let closed = crate::testutil::frame_display(&app, w, h);
            assert!(
                closed.contains("Enter 反查"),
                "{w}×{h} 触发段应在场:{closed}"
            );
            assert!(
                !closed.contains("什么标签引用了这一条"),
                "{w}×{h} 没问就不起浮层:{closed}"
            );
            for ln in closed.lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 关着也有行超宽:{ln}"
                );
            }
            // 开着:结果浮层在场(它盖住键位行是弹层纪律,不是漏画)
            反查这台(&mut app);
            feed(&mut app);
            let open = crate::testutil::frame_display(&app, w, h);
            assert!(
                open.contains("3 个标签引用了 gate"),
                "{w}×{h} 结果浮层应在场:{open}"
            );
            assert!(
                open.contains("Esc 或点击任意处关闭"),
                "{w}×{h} 收口行应在场:{open}"
            );
            for ln in open.lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 开着也有行超宽:{ln}"
                );
            }
            tap(&mut app, KeyCode::Esc);
        }
    }

    // ---- 工单 #70 的两处「要当场钉住」的边界 ----

    /// 规格验收 7 的第三面:在途时**换册**(`m`)把在途清成 Idle,迟到的
    /// `tag ls` 回音于是不收敛任何状态。它问的那台机器已经不在场,面板不替
    /// 一个走了的问题留一个幽灵结果——「迟到回音不收敛」这条纪律因此不只
    /// 返航一条路,换册 / 切屏同样算。
    #[test]
    fn 反查_在途时换册_迟到回音不收敛任何状态() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app, KeyCode::Down);
        反查这台(&mut app); // 在途
        assert_eq!(app.queue.len(), 1, "确有一次只读调用在队里");
        tap(&mut app, KeyCode::Char('m')); // 翻到标签册
        assert_eq!(
            app.catalog_kind,
            CatalogKind::Tag,
            "换册成功(在途不是写闸,挡不住换册)"
        );
        assert!(!app.vps_reverse_open(), "换册即收在途");
        // 迟到的 tag ls 回音落地:标签册照常读进来,但反查浮层不开
        feed(&mut app);
        assert!(!app.vps_reverse_open(), "迟到回音不重开浮层");
        assert_eq!(app.tag_len(), 4, "标签册照常呈现(只读回音没被丢)");
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(!s.contains("什么标签引用了这一条"), "屏上没有反查浮层:{s}");
    }

    /// 规格验收 12 / #62 立过的双向对齐(帮助不写幽灵键、README 不漏键):
    /// 机器册的 `Enter`(反查上浮)在 `HELP_ROWS` 里挂在机器册那一行、说明写
    /// 的是「什么标签引用了这一条」,在 README 的编目键位列里也在场。这一条
    /// 不新造对照机制,只是把 #70 这一个键接进既有的双向对齐——免得反查上浮
    /// 落成「屏上有个键,两处文档都没有」。顺带把那一行派发下去:实现里真有
    /// 一条真实分支,不是幽灵键。
    #[test]
    fn 反查键_帮助浮层与readme都对齐_派发确有一次只读调用() {
        use crate::ui::HELP_ROWS;
        let row = HELP_ROWS
            .iter()
            .find(|r| r.book == Some(CatalogKind::Vps) && r.code == KeyCode::Enter)
            .expect("机器册的 Enter(反查上浮)应在帮助对照表里");
        assert!(
            row.desc.contains("什么标签引用了"),
            "说明写的是反查上浮这一件事:{}",
            row.desc
        );
        let readme = include_str!("../../README.md");
        let line = readme
            .lines()
            .find(|l| l.starts_with("| 编目 "))
            .expect("README 六屏表应有编目一行");
        assert!(line.contains("`Enter`"), "编目键位列应有 Enter:{line}");
        assert!(line.contains("反查"), "并说清它是反查:{line}");
        // 帮助那一行派发下去:实现里真有一条只读分支
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app, KeyCode::Down);
        app.update(crate::app::Message::Key(KeyEvent::new(
            row.code,
            KeyModifiers::NONE,
        )));
        assert_eq!(
            app.queue.len(),
            1,
            "帮助列的 Enter 派发下去确有一次只读 tag ls"
        );
    }

    /// 反查触发段**只机器册有命中**。`TAG_REF_KINDS` 的已知集合是
    /// `skill` / `extension` / `vps`,四本册里只有机器册的条目可打标签;
    /// 订阅册与钥匙册的条目不在其内,放一个注定只返回「没有」的动作是噪声
    /// (spec #65 §4:触发点只有一处)。标签册的 `Enter` 是另一件事(#54 反查
    /// 选中的那条标签),它的键位行也写着「Enter 反查」四个字,但那是**只画
    /// 不点的提示段**,不给命中——所以这一条只认命中区,不认帧上有没有那四
    /// 个字(认字就会把标签册的提示段误判成触发段)。
    #[test]
    fn 反查触发段_只机器册给命中_另三本连段都不给() {
        for (name, kind) in [
            ("机器", CatalogKind::Vps),
            ("标签", CatalogKind::Tag),
            ("订阅", CatalogKind::Subscription),
            ("钥匙", CatalogKind::Key),
        ] {
            let mut app = book(name);
            assert_eq!(app.catalog_kind, kind, "{name}册");
            app.size = Rect::new(0, 0, 80, 30);
            let hits = crate::layout::hitmap(app.size, &app, crate::layout::size_class(app.size));
            let has = hits
                .iter()
                .any(|(_, t)| matches!(t, crate::layout::Target::VpsReverse));
            assert_eq!(
                has,
                kind == CatalogKind::Vps,
                "{name}册:反查触发段的有无={has}"
            );
        }
    }

    /// 反查结果浮层与编目过滤输入态**同时在场**时,`Esc` 归谁?答案:浮层是
    /// 更上一层(与帮助 / 发射确认同级的顶层浮层),所以 **Esc 先收浮层**,
    /// 过滤输入态与刚敲的过滤词原样留着。反过来会让操作者卡在一个二选一里:
    /// 要么还被浮层蒙着就丢了刚敲的词,要么为了清词先手动关浮层。
    /// 这条把次序钉住,免得以后有人把浮层守卫挪到过滤守卫后面去。
    #[test]
    fn 反查浮层与过滤输入态同时在场_先收浮层_过滤词不丢() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        tap(&mut app, KeyCode::Down); // 选中 gate
        tap(&mut app, KeyCode::Char('/')); // 进过滤输入态
        tap(&mut app, KeyCode::Char('g'));
        tap(&mut app, KeyCode::Char('a')); // 过滤词「ga」(只留 gate)
        assert!(app.catalog_filter_on, "输入态在场");
        // 输入态下点键位行的反查段:可点段不收输入态管(输入态只管键入)
        点中(&mut app, 80, 30, |t| {
            matches!(t, crate::layout::Target::VpsReverse)
        });
        feed(&mut app);
        assert!(app.vps_reverse_open(), "浮层开起来了");
        assert!(app.catalog_filter_on, "过滤输入态还在");
        tap(&mut app, KeyCode::Esc);
        assert!(!app.vps_reverse_open(), "Esc 先收浮层(更上一层先收)");
        assert!(app.catalog_filter_on, "过滤输入态不退:它不在这一层");
        assert_eq!(app.catalog_filter(), "ga", "刚敲的过滤词不丢");
        // 再按一次 Esc 才归输入态那一层(清词 + 退输入态)
        tap(&mut app, KeyCode::Esc);
        assert!(!app.catalog_filter_on, "第二次 Esc 才退输入态");
        assert_eq!(app.catalog_filter(), "", "第二次 Esc 清词");
    }

    /// 回归(#65 评审第 3 条):机器册反查(#70)发的是**不带参**的
    /// `tag ls --root <根>`,所以切到标签册时册头必须说「列全部标签」。
    /// 早先回音路由从 argv 重新推导「问的是哪一条」
    /// (`skip(2).find(非 -- 开头)`),它把 `--root` 的**值**当成了标签名:
    /// 册头写「当前册 标签 · 反查 /tmp」、日志写「反查完成(标签 /tmp:4 条,
    /// 只读)」——「列全部标签」被冒充成「反查一个叫 /tmp 的标签」,一种
    /// 状态冒充另一种。下面两条把册头与日志两处都钉住:带的只能是
    /// `run_tag_ls` 下发时记下的那个入参(`None` = 列全部)。
    #[test]
    fn 反查后切到标签册_册头与日志都说全部标签_不把数据仓根当标签名() {
        let (_r, mut app) = reverse_ready(REVERSE_MACHINES, &[("tag ls", 0, TAG_LS_GATE3)]);
        反查这台(&mut app);
        feed(&mut app);
        assert!(app.vps_reverse_open(), "反查结论先在场");
        tap(&mut app, KeyCode::Esc); // 先收浮层(它在场时吞册体键)
        tap(&mut app, KeyCode::Char('m')); // 机器册 → 标签册
        let root = app.usable_data_root().unwrap_or_default();
        assert_eq!(
            app.catalog_scope_label(),
            "全部标签",
            "不带参反查 = 列全部:册头别把数据仓根 {root} 当成标签名"
        );
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(
            s.contains("当前册 标签 · 全部标签 ·"),
            "帧上册头同口径(下面两帧 golden 就钉在这一行):{s}"
        );
        assert!(
            !s.contains(&format!("反查 {root}")),
            "册头不出现「反查 <数据仓根>」:{s}"
        );
        assert!(
            app.log.iter().any(|l| l.contains("反查完成(全部标签:")),
            "日志同口径:列全部,不是一个叫 {root} 的标签:{:?}",
            app.log.first()
        );
        assert!(
            app.log
                .iter()
                .all(|l| !l.contains(&format!("标签 {root}:"))),
            "日志不把数据仓根 {root} 当成标签名:{:?}",
            app.log.first()
        );
    }

    /// 机器册键位行三个态的 `Esc` 口径一致(#65 评审第 8 条):`q` 在编目册册体上
    /// 是壳级 quit(`on_key` 的 `Char('q') => quit`),**不是返航**——`keys_body`
    /// 已为此收窄成裸 `Esc`,空册态却还写着 `Esc/q 返航`,同一块屏两个态对同一
    /// 个键说两样话。这条把「册体态 / 空册态都不许把 `q` 当返航、浮层态两键
    /// 都认」钉住:谁再把 `Esc/q 返航` 写回册体或空册的段清单,这里就红。
    ///
    /// 浮层态是**有意**两键都写:登记流 / 删除确认在场时 `on_key` 先把键交给浮层
    /// 那层,`Esc` 与 `q` 都收敛浮层,壳级 quit 那次判定排在更后面、根本轮不到
    /// (详见 `catalog_key_parts` 的注释)。
    #[test]
    fn 机器册键位行_esc口径三态一致_册体与空册都不把q当返航() {
        // ① 有条目(册体态):`Esc`,且不出现 `q 返航`
        let mut app = book("机器");
        let body = crate::testutil::frame_display(&app, 110, 30);
        let body_line = body
            .lines()
            .find(|l| l.contains("↑/↓ 选"))
            .expect("册体态键位行在场")
            .to_string();
        assert!(body_line.contains("Esc"), "册体态给 Esc 提示:{body_line}");
        assert!(
            !body_line.contains("q 返航"),
            "册体态不把 q 当返航(q 是壳级退出):{body_line}"
        );
        // ② 空册态:同一个口径
        let mut empty =
            crate::app::App::with_runner(std::sync::Arc::new(crate::engine::FakeRunner::new(&[(
                "vps ls",
                0,
                r#"{"catalogPath":"/r/vps.json","machines":[]}"#,
            )])));
        empty.splash = false;
        empty.screen = crate::app::Screen::Vps;
        empty.catalog_kind = CatalogKind::Vps;
        empty.capabilities.record(
            crate::engine::Capability::Vps,
            crate::engine::Probe::Available,
        );
        empty.vps_ls =
            Some(serde_json::from_str(r#"{"catalogPath":"/r/vps.json","machines":[]}"#).unwrap());
        empty.data_root = Some(crate::testutil::stable_root());
        assert_eq!(empty.catalog_len(), 0, "确实是空册");
        let ef = crate::testutil::frame_display(&empty, 110, 30);
        let empty_line = ef
            .lines()
            .find(|l| l.contains("r 登记机器"))
            .expect("空册态键位行在场")
            .to_string();
        assert!(empty_line.contains("Esc"), "空册态给 Esc 提示:{empty_line}");
        assert!(
            !empty_line.contains("q 返航"),
            "空册态与册体态同一口径,不把 q 当返航:{empty_line}"
        );
        // ③ 浮层态:两键都认(这一处与册体态口径不同,理由见 catalog_key_parts)
        tap(&mut app, KeyCode::Char('r'));
        let of = crate::testutil::frame_display(&app, 110, 30);
        assert!(
            of.contains("Esc/q 返航"),
            "浮层态 Esc 与 q 都收敛浮层,两句都写:{of}"
        );
    }

    /// 键位行的**行号**与它的**命中矩形**钉在同一处(#65 评审第 6 条):命中区用
    /// `catalog_body_lines` 的行数推出键位行的 y,渲染用同一份行数把键位行画到
    /// 那一行。这一条从行为上再钉一道——命中矩形落在的那一行,渲染出来的行里
    /// **真的带着键位段**。谁再把两边的宽度预算拆开(渲染一个数、命中另一个数),
    /// 键位行就会整体错一行,这条就红。
    ///
    /// 顺带钉三档:常畅 80×30、紧凑 66×50、宽 110×30(三段可点段都放得下的那一档)。
    #[test]
    fn 键位行命中矩形那一行_渲染出来真的带着键位段() {
        for (w, h) in [(80u16, 30u16), (66, 50), (110, 30)] {
            let mut app = book("机器");
            let size = Rect::new(0, 0, w, h);
            app.size = size;
            let hits = crate::layout::hitmap(size, &app, crate::layout::size_class(size));
            let rows: Vec<u16> = hits
                .iter()
                .filter(|(_, t)| {
                    matches!(
                        t,
                        crate::layout::Target::SwitchBook
                            | crate::layout::Target::CatalogFilter
                            | crate::layout::Target::VpsReverse
                    )
                })
                .map(|(r, _)| r.y)
                .collect();
            assert!(!rows.is_empty(), "{w}×{h} 机器册键位行应有可点段:{hits:?}");
            assert!(
                rows.iter().all(|&y| y == rows[0]),
                "{w}×{h} 可点段应同一行:{rows:?}"
            );
            let f = crate::testutil::frame_display(&app, w, h);
            let line = f.lines().nth(rows[0] as usize).unwrap_or_default();
            assert!(
                line.contains(CATALOG_SWITCH),
                "{w}×{h} 命中矩形那一行渲染出来真的带着键位段:{line}"
            );
        }
    }

    // ---- 工单 #66:四本册整帧 golden(逐字节) ----
    //
    // 本票是**纯重构**:操作者感知不到任何变化。上面四条
    // `*_屏_常畅与紧凑两档都渲染不溢出` 只钉「在场」(contains),钉不住
    // 「没变」;这一族把整帧显示内容逐字节钉住,按册各一份、常畅(80×30)与
    // 紧凑(66×50)两档各一份——按册分叉收敛成查表成员之后,任何一本的
    // 文案 / 行序 / 行数 / 折列漂了一个字节,这里就红。
    //
    // **期望值的来历(不止一句「本票之前」,每一次刷新都记在这儿)**:
    // 基线 = 第 8 轮 tip `bb19d70` + 工单 #66(同一夹具、同一尺寸,人工逐行
    // 核对;#66 确是纯重构,基线帧与 `bb19d70` 逐字节相同)。基线之后有过四
    // 次**真实的用户可见变化**,每次都刷新过期望,差都记在各自工单里:
    //   · #67(合入点 `42eb955`):机器册键位行末尾多出可点的 `m 换册` 段;
    //   · #69(`8f676fc`):机器册键位行再多 `/ 过滤` 段(另三本放不下,段就
    //     不画也不给命中——那一版的取舍由本工单推翻,见下一条);
    //   · #70(`31f069d`):机器册键位行 `↑/↓ 选机器`→`↑/↓ 选`、
    //     `Esc/q 返航`→`Esc`,并多出 `Enter 反查` 段;
    //   · 本票 #65 评审第 3 条:标签册册头把 `反查 /tmp` 改回 `全部标签`
    //     (那一行原是把 `--root` 的值错当成标签名,见 `CatalogKind::absorb_ls` 的标签册臂);
    //   · **本工单(#69 后续 / #70 评审第 5 条)**:键位行分隔
    //     `CATALOG_KEY_GAP` 由 3 列收窄到 2 列(`" · "` → `" ·"`),订阅 /
    //     钥匙册 `g 取回命令` → `g 取回`,于是 `/ 过滤` 在四本册 × 两档内都
    //     放得下(此前只有机器册画得出、点得着)。八帧全部重刷,每帧差的就是
    //     键位行这一行,逐字节对过;帧上其余部分与上一次刷新逐字节相同。
    //   · **#73**:键位行钉在册体内层**末行**(此前随册体内容长短漂:内容一多
    //     就被顶出可见区,键位行整行消失、命中区也跟着不产)。八帧全部重刷,
    //     每帧差的**只是键位行这一行的位置**——段文案一个字节都没动(订阅 /
    //     钥匙册的 `g 取回` 段本轮才挂上命中目标,屏上文字与刷新前逐字相同),
    //     逐字节对过;帧上其余部分与上一次刷新逐字节相同。
    //   · **ADR-0009 Q5(scopes 报值)随第 8 轮分支并入**:`949643b` 把钥匙册
    //     作用域行从「只报条数」改成「原样逐条报」(第 9 轮册内过滤按 scope 的
    //     值匹配之后显出的一处不一致:能按一个只看得到条数的字段过滤)。该提交
    //     落在第 8 轮分支上,本分支(第 9 轮)的钥匙册两帧期望还是旧句,合并后
    //     **只有这一行红**。两帧重刷,差的就只有作用域这一行(`2 条` →
    //     `repo · read:org`),逐字节对过;帧上其余部分与上一次刷新逐字节相同。
    // 除上面记下的几处,其余六帧自 #66 基线以来一个字节都没动过——所以
    // 「基线 + 六次有记账的刷新」就是这一族期望值的全部来历,没有第七次,
    // 也没有哪一次是悄悄回填的。
    //
    // 修订纪律同 `ui::tests::golden_常畅态_80x30_整帧逐字节`:若既有渲染面
    // 发生真实回归而改期望,必须与该回归同一次提交并说明——不许为过测试回填。

    /// 四本册各就绪一个 app(夹具 / splash / 只读 ls 回音全部钉死),
    /// 整帧 golden 与「四本册走同一条键位路径」共用。
    ///
    /// 场景钉死到不随机器与测试次序漂:数据仓根显式钉成临时目录(否则
    /// golden 字节与键位测试跟着跑测试那台机器的配置漂,建议命令里带
    /// `--root <根>`)、NW_HOME 显式钉成 `/nw`(顶栏那一行;别的测试会经
    /// `apply_nw_home` 写进程环境,不钉就随执行次序漂)。理由同
    /// `ui::tests::golden_常畅态_80x30_整帧逐字节` 的头注:环境噪声不算回归。
    fn book_apps() -> Vec<(&'static str, crate::app::App)> {
        let vps = {
            let ls = r#"{"catalogPath":"/r/vps.json","machines":[{"name":"gate","ips":[{"label":"home","addr":"10.0.0.1"}],"port":22,"logins":[{"user":"root","ref":"$DEPLOY_KEY"}],"note":"门口"},{"name":"bare","ips":[],"port":0,"logins":[]}]}"#;
            let mut app = crate::app::App::with_runner(std::sync::Arc::new(
                crate::engine::FakeRunner::new(&[("vps ls", 0, ls)]),
            ));
            app.splash = false;
            app.screen = crate::app::Screen::Vps;
            app.capabilities.record(
                crate::engine::Capability::Vps,
                crate::engine::Probe::Available,
            );
            app.vps_ls = Some(serde_json::from_str(ls).unwrap());
            app.data_root = Some(crate::testutil::stable_root());
            app.nw_home = "/nw".into(); // 顶栏 NW_HOME 也钉死(别的测试会写进程环境)
            app
        };
        let mut tag = tag_app(&[("tag ls", 0, TAG_LS_ALL)]);
        tag.data_root = Some(crate::testutil::stable_root());
        tag.nw_home = "/nw".into();
        tap(&mut tag, KeyCode::Char('l'));
        feed(&mut tag);
        let (mut sub, _r) = sub_app(&[("subscription ls", 0, SUB_LS_ALL)]);
        sub.data_root = Some(crate::testutil::stable_root());
        sub.nw_home = "/nw".into();
        tap(&mut sub, KeyCode::Char('l'));
        feed(&mut sub);
        let (mut key, _r) = key_app(&[("key ls", 0, KEY_LS_ALL)]);
        key.data_root = Some(crate::testutil::stable_root());
        key.nw_home = "/nw".into();
        tap(&mut key, KeyCode::Char('l'));
        feed(&mut key);
        vec![("机器", vps), ("标签", tag), ("订阅", sub), ("钥匙", key)]
    }

    /// 按册名取一本就绪的册(四本册同一套就绪口径)。
    fn book(name: &str) -> crate::app::App {
        book_apps()
            .into_iter()
            .find(|(n, _)| *n == name)
            .map(|(_, a)| a)
            .expect("四本册之一")
    }

    /// 整帧逐字节比对(got 与 want 一上一下,漂一个字节就红)。
    fn assert_book_frame(app: &crate::app::App, w: u16, h: u16, want: &str) {
        assert_eq!(
            crate::testutil::frame_display(app, w, h),
            want,
            "{w}×{h} 整帧渲染发生变化(见本族测试头注的修订纪律)"
        );
    }
    /// 机器册 80×30 整帧 golden(基线 `bb19d70` + #66;此后 #67 加 `m 换册`、#69 加 `/ 过滤`、#70 改 `Esc` 并加 `Enter 反查`;#119 常畅态 HUD 加 `Ctrl+P 命令` 段)。
    const VPS_80X30: &str = r####" Warehousetui · Nous Warehouse 驾驶舱   nwt v0.3 · nw 探测中 · 测试注入  │  NW_H
┌ 导航 ────┐┌ 编目 ───────────────────────────────────────────机器写 · 永不手改┐
│  ▸ 概览  ││  当前册 机器 · 机器 · 编目 /r/vps.json                           │
│  ⚙ 装配  ││  ▸ bare  端口 0                                                  │
│  ✚ 健康  ││    地址 ——                                                       │
│  ▤ 配方  ││    登录 ——                                                       │
│▸ ▩ 编目  ││    备注 ——                                                       │
│  ✦ 设置  ││    gate  端口 22                                                 │
│          ││    地址 home=10.0.0.1                                            │
│          ││    登录 root→$DEPLOY_KEY                                         │
│          ││    备注 门口                                                     │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││  ↑/↓ 选 ·r 登记 ·d 删除 ·Esc ·Enter 反查 ·m 换册 ·/ 过滤         │
└──────────┘└──────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 t 主题 ? 帮助 Ctrl+P 命令   q 退出主题:暗色仓库            "####;
    /// 机器册 66×50 整帧 golden(同 80×30 的来历,紧凑档)。
    const VPS_66X50: &str = r####"NW_HOME: /nw  ○ 空闲                                              
  ▸ 概览   ⚙ 装配   ✚ 健康   ▤ 配方 ▸ ▩ 编目   ✦ 设置             
┌ 编目 ─────────────────────────────────────────机器写 · 永不手改┐
│  当前册 机器 · 机器 · 编目 /r/vps.json                         │
│  ▸ bare  端口 0                                                │
│    地址 ——                                                     │
│    登录 ——                                                     │
│    备注 ——                                                     │
│    gate  端口 22                                               │
│    地址 home=10.0.0.1                                          │
│    登录 root→$DEPLOY_KEY                                       │
│    备注 门口                                                   │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│  ↑/↓ 选 ·r 登记 ·d 删除 ·Esc ·Enter 反查 ·m 换册 ·/ 过滤       │
└────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 ? 帮助   q 退出                              "####;
    /// 标签册 80×30 整帧 golden(基线 `bb19d70` + #66;#65 评审第 3 条把册头 `反查 /tmp` 改回 `全部标签`)。
    const TAG_80X30: &str = r####" Warehousetui · Nous Warehouse 驾驶舱   nwt v0.3 · nw 探测中 · 测试注入  │  NW_H
┌ 导航 ────┐┌ 编目 ───────────────────────────────────────────机器写 · 永不手改┐
│  ▸ 概览  ││  当前册 标签 · 全部标签 · 编目 /r/tags.json                      │
│  ⚙ 装配  ││  ▸ bare                                                          │
│  ✚ 健康  ││    引用 ——                                                       │
│  ▤ 配方  ││    pi                                                            │
│▸ ▩ 编目  ││    引用 skill/o/r/deep-research · vps/gate                       │
│  ✦ 设置  ││  边界:标签绝不进配方/锁定集/bundle/装配(不提供烤进去的入口)      │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││  ↑/↓ 选 ·Enter 反查 ·l 全部 ·r 挂标签 ·d 摘/删 ·m 换册 ·/ 过滤   │
└──────────┘└──────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 t 主题 ? 帮助 Ctrl+P 命令   q 退出主题:暗色仓库            "####;
    /// 标签册 66×50 整帧 golden(同 80×30 的来历,紧凑档)。
    const TAG_66X50: &str = r####"NW_HOME: /nw  ○ 空闲                                              
  ▸ 概览   ⚙ 装配   ✚ 健康   ▤ 配方 ▸ ▩ 编目   ✦ 设置             
┌ 编目 ─────────────────────────────────────────机器写 · 永不手改┐
│  当前册 标签 · 全部标签 · 编目 /r/tags.json                    │
│  ▸ bare                                                        │
│    引用 ——                                                     │
│    pi                                                          │
│    引用 skill/o/r/deep-research · vps/gate                     │
│  边界:标签绝不进配方/锁定集/bundle/装配(不提供烤进去的入口)    │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│  ↑/↓ 选 ·Enter 反查 ·l 全部 ·r 挂标签 ·d 摘/删 ·m 换册 ·/ 过滤 │
└────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 ? 帮助   q 退出                              "####;
    /// 订阅册 80×30 整帧 golden(基线 `bb19d70` + #66,此后一个字节未动)。
    const SUB_80X30: &str = r####" Warehousetui · Nous Warehouse 驾驶舱   nwt v0.3 · nw 探测中 · 测试注入  │  NW_H
┌ 导航 ────┐┌ 编目 ───────────────────────────────────────────机器写 · 永不手改┐
│  ▸ 概览  ││  当前册 订阅 · 全部订阅 · 编目 /r/subscriptions.json             │
│  ⚙ 装配  ││  ▸ airport  应用 Mihomo                                          │
│  ✚ 健康  ││    到期 2099-03-01  ✓ active  剩 26000 天                        │
│  ▤ 配方  ││    forever  应用 Surge                                           │
│▸ ▩ 编目  ││    到期 永不过期  ✓ active  剩余 ——                              │
│  ✦ 设置  ││    gone  应用 Mihomo                                             │
│          ││    到期 2020-01-01  ✗ expired  剩 -2000 天                       │
│          ││    soon  应用 Clash                                              │
│          ││    到期 2099-03-04  警 expiring  剩 3 天                         │
│          ││  汇总 2 active · 1 expiring · 1 expired                          │
│          ││  三态按 UTC 在读取时算、永不写进文件;只标记不治理(过期不改退出   │
│          ││  取回链接(面板不代跑,按 g 收进日志):                             │
│          ││nw subscription get --name airport --root /tmp | jq -r .data.url  │
│          ││  get 是上游唯一回显本体的出口:铺在屏显上会进滚动历史/截图/快照   │
│          ││  边界:订阅链接绝不进配方/锁定集/bundle/装配(续费换链接不触发重   │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││  ↑/↓ 选订阅 ·r 登记 ·g 取回 ·d 删除 ·l 重读 ·m 换册 ·/ 过滤      │
└──────────┘└──────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 t 主题 ? 帮助 Ctrl+P 命令   q 退出主题:暗色仓库            "####;
    /// 订阅册 66×50 整帧 golden(同 80×30 的来历,紧凑档)。
    const SUB_66X50: &str = r####"NW_HOME: /nw  ○ 空闲                                              
  ▸ 概览   ⚙ 装配   ✚ 健康   ▤ 配方 ▸ ▩ 编目   ✦ 设置             
┌ 编目 ─────────────────────────────────────────机器写 · 永不手改┐
│  当前册 订阅 · 全部订阅 · 编目 /r/subscriptions.json           │
│  ▸ airport  应用 Mihomo                                        │
│    到期 2099-03-01  ✓ active  剩 26000 天                      │
│    forever  应用 Surge                                         │
│    到期 永不过期  ✓ active  剩余 ——                            │
│    gone  应用 Mihomo                                           │
│    到期 2020-01-01  ✗ expired  剩 -2000 天                     │
│    soon  应用 Clash                                            │
│    到期 2099-03-04  警 expiring  剩 3 天                       │
│  汇总 2 active · 1 expiring · 1 expired                        │
│  三态按 UTC 在读取时算、永不写进文件;只标记不治理(过期不改退   │
│  取回链接(面板不代跑,按 g 收进日志):                           │
│nw subscription get --name airport --root /tmp | jq -r .data.u  │
│  get 是上游唯一回显本体的出口:铺在屏显上会进滚动历史/截图/快   │
│  边界:订阅链接绝不进配方/锁定集/bundle/装配(续费换链接不触发   │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│  ↑/↓ 选订阅 ·r 登记 ·g 取回 ·d 删除 ·l 重读 ·m 换册 ·/ 过滤    │
└────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 ? 帮助   q 退出                              "####;
    /// 钥匙册 80×30 整帧 golden(基线 `bb19d70` + #66,此后一个字节未动;
    /// #106 轮换日那一行加了账龄列——本轮唯二变动,来历记在这里)。
    const KEY_80X30: &str = r####" Warehousetui · Nous Warehouse 驾驶舱   nwt v0.3 · nw 探测中 · 测试注入  │  NW_H
┌ 导航 ────┐┌ 编目 ───────────────────────────────────────────机器写 · 永不手改┐
│  ▸ 概览  ││  当前册 钥匙 · 全部钥匙 · 编目 /r/keys.json                      │
│  ⚙ 装配  ││  ▸ ci  kind github-key                                           │
│  ✚ 健康  ││    主机 ghe.internal                                             │
│  ▤ 配方  ││    作用域 未声明(0 条)                                           │
│▸ ▩ 编目  ││    轮换日 2025-11-02  账龄 345 天                                │
│  ✦ 设置  ││    deploy  kind github-key                                       │
│          ││    主机 github.com                                               │
│          ││    作用域 repo · read:org                                        │
│          ││    轮换日 2026-01-15  账龄 271 天                                │
│          ││  取回钥匙(面板不代跑,按 g 收进日志):                             │
│          ││nw key get --name ci --root /tmp | jq -r .data.key                │
│          ││  get 是上游唯一回显本体的出口:铺在屏显上会进滚动历史/截图/快照   │
│          ││  边界:钥匙绝不进配方/锁定集/bundle/装配(凭据不随派分发,不提供烤  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││                                                                  │
│          ││  ↑/↓ 选钥匙 ·r 登记 ·g 取回 ·d 删除 ·l 重读 ·m 换册 ·/ 过滤      │
└──────────┘└──────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 t 主题 ? 帮助 Ctrl+P 命令   q 退出主题:暗色仓库            "####;
    /// 钥匙册 66×50 整帧 golden(同 80×30 的来历,紧凑档;#106 同因加账龄列)。
    const KEY_66X50: &str = r####"NW_HOME: /nw  ○ 空闲                                              
  ▸ 概览   ⚙ 装配   ✚ 健康   ▤ 配方 ▸ ▩ 编目   ✦ 设置             
┌ 编目 ─────────────────────────────────────────机器写 · 永不手改┐
│  当前册 钥匙 · 全部钥匙 · 编目 /r/keys.json                    │
│  ▸ ci  kind github-key                                         │
│    主机 ghe.internal                                           │
│    作用域 未声明(0 条)                                         │
│    轮换日 2025-11-02  账龄 345 天                              │
│    deploy  kind github-key                                     │
│    主机 github.com                                             │
│    作用域 repo · read:org                                      │
│    轮换日 2026-01-15  账龄 271 天                              │
│  取回钥匙(面板不代跑,按 g 收进日志):                           │
│nw key get --name ci --root /tmp | jq -r .data.key              │
│  get 是上游唯一回显本体的出口:铺在屏显上会进滚动历史/截图/快   │
│  边界:钥匙绝不进配方/锁定集/bundle/装配(凭据不随派分发,不提供  │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│                                                                │
│  ↑/↓ 选钥匙 ·r 登记 ·g 取回 ·d 删除 ·l 重读 ·m 换册 ·/ 过滤    │
└────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 ? 帮助   q 退出                              "####;

    /// 机器册整帧 golden:常畅 80×30 与紧凑 66×50 两档各一份,逐字节钉住
    /// (#66 验收第 1 条:纯重构,零用户可见变化——基线 `bb19d70` + #66;
    /// 此后的每次刷新都记在本族头注的来历里,不再一句「本票之前」带过)。
    #[test]
    fn 机器册整帧golden_两档golden() {
        let app = book("机器");
        assert_book_frame(&app, 80, 30, VPS_80X30);
        assert_book_frame(&app, 66, 50, VPS_66X50);
    }

    /// 标签册整帧 golden:常畅 80×30 与紧凑 66×50 两档各一份,逐字节钉住
    /// (#66 验收第 1 条:纯重构,零用户可见变化——基线 `bb19d70` + #66;
    /// 此后的每次刷新都记在本族头注的来历里,不再一句「本票之前」带过)。
    #[test]
    fn 标签册整帧golden_两档golden() {
        let app = book("标签");
        assert_book_frame(&app, 80, 30, TAG_80X30);
        assert_book_frame(&app, 66, 50, TAG_66X50);
    }

    /// 订阅册整帧 golden:常畅 80×30 与紧凑 66×50 两档各一份,逐字节钉住
    /// (#66 验收第 1 条:纯重构,零用户可见变化——基线 `bb19d70` + #66;
    /// 此后的每次刷新都记在本族头注的来历里,不再一句「本票之前」带过)。
    #[test]
    fn 订阅册整帧golden_两档golden() {
        let app = book("订阅");
        assert_book_frame(&app, 80, 30, SUB_80X30);
        assert_book_frame(&app, 66, 50, SUB_66X50);
    }

    /// 钥匙册整帧 golden:常畅 80×30 与紧凑 66×50 两档各一份,逐字节钉住
    /// (#66 验收第 1 条:纯重构,零用户可见变化——基线 `bb19d70` + #66;
    /// 此后的每次刷新都记在本族头注的来历里,不再一句「本票之前」带过)。
    #[test]
    fn 钥匙册整帧golden_两档golden() {
        pin_today_for_test(2026, 10, 13);
        let app = book("钥匙");
        assert_book_frame(&app, 80, 30, KEY_80X30);
        assert_book_frame(&app, 66, 50, KEY_66X50);
    }

    // ---- 钥匙轮换账龄(spec #94 第 5 项 / 工单 #106) ----

    /// 造一条只填了轮换日的钥匙条目——账龄只依赖这一格(其余槽与本票无关)。
    fn key_entry(rotated_at: Option<&str>) -> CatalogEntryView {
        CatalogEntryView {
            name: "ci".into(),
            rotated_at: rotated_at.map(str::to_string),
            ..CatalogEntryView::default()
        }
    }

    fn age_of(rotated_at: Option<&str>, today: UtcDay) -> RotationAge {
        RotationAge::of(&key_entry(rotated_at), today)
    }

    /// 账龄是 UTC **日历日**差(不是 30 天月、不是 365 天年)。期望值取自独立
    /// 算法(python3 `datetime.date` 减法),不按实现里的同一套公式反推。
    #[test]
    fn 轮换账龄_按日历日差算_同一天为0() {
        let today = UtcDay {
            y: 2026,
            m: 10,
            d: 13,
        };
        assert_eq!(age_of(Some("2026-10-13"), today), RotationAge::Days(0));
        assert_eq!(age_of(Some("2026-10-12"), today), RotationAge::Days(1));
        assert_eq!(age_of(Some("2026-10-06"), today), RotationAge::Days(7));
        assert_eq!(age_of(Some("2026-01-15"), today), RotationAge::Days(271));
        assert_eq!(age_of(Some("2025-11-02"), today), RotationAge::Days(345));
        // 跨闰年与闰日本身:2024-02-29 是真日子,2025-02-29 不是
        assert_eq!(age_of(Some("2024-02-29"), today), RotationAge::Days(957));
        assert_eq!(age_of(Some("2025-02-29"), today), RotationAge::Unknown);
    }

    /// 算不出的五类(没这一格 / 空 / 形状不认 / 不是真日历日 / 未来日)全归
    /// 未知,**没有一类拿 0 冒充**——0 天只有「今天刚轮换」一种来路。
    #[test]
    fn 轮换账龄_算不出就说未知_不拿0冒充() {
        let today = UtcDay {
            y: 2026,
            m: 10,
            d: 13,
        };
        for r in [
            None,
            Some(""),
            Some("   "),
            Some("2026-1-5"),
            Some("2026-13-01"),
            Some("2026-02-30"),
            Some("2025-02-29"),
            Some("昨天"),
        ] {
            assert_eq!(age_of(r, today), RotationAge::Unknown, "{r:?} 应算不出");
        }
        assert_eq!(
            age_of(Some("2026-10-14"), today),
            RotationAge::Unknown,
            "轮换日在未来:不印负数,也不拿 0 冒充今天刚换"
        );
    }

    #[test]
    fn 账龄一行_算得出报天数_算不出说未知() {
        assert_eq!(RotationAge::Days(0).line(), "账龄 0 天");
        assert_eq!(RotationAge::Days(345).line(), "账龄 345 天");
        assert_eq!(RotationAge::Unknown.line(), "账龄 未知");
    }

    /// 阈值为 7 天、「超」是严格大于:7 天本身不亮,8 天亮;未知不亮
    /// (没有依据就不标记)。
    #[test]
    fn 账龄超7天才亮_未知与7天本身不亮() {
        assert!(!RotationAge::Days(0).over_threshold());
        assert!(!RotationAge::Days(7).over_threshold(), "7 天本身不算超");
        assert!(RotationAge::Days(8).over_threshold());
        assert!(
            !RotationAge::Unknown.over_threshold(),
            "算不出不亮:没有依据就不标记"
        );
    }

    /// 一把老钥匙 + 一把新钥匙:老的那把亮 amber,新的那把不亮(今天钉在
    /// 2026-10-13,账龄 345 天 / 1 天)。
    const KEY_LS_AGE_MIX: &str = r#"{"schema":"nw/key-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/keys.json","keys":[{"name":"old","kind":"github-key","host":"github.com","scopes":[],"rotatedAt":"2025-11-02"},{"name":"fresh","kind":"github-key","host":"github.com","scopes":[],"rotatedAt":"2026-10-12"}]}}"#;

    /// 册面真画出了账龄,且两档都不溢宽(66×50 是响应式下限)。
    #[test]
    fn 钥匙册_轮换日行带账龄_两档都在且不溢宽() {
        pin_today_for_test(2026, 10, 13);
        let (mut app, _r) = key_app(&[("key ls", 0, KEY_LS_ALL)]);
        app.nw_home = "/nw".into();
        tap(&mut app, KeyCode::Char('l'));
        feed(&mut app);
        for (w, h) in [(80u16, 30u16), (66, 50)] {
            let s = crate::testutil::frame_display(&app, w, h);
            assert!(s.contains("账龄 345 天"), "{w}×{h} 老钥匙的账龄应在场:{s}");
            assert!(s.contains("账龄 271 天"), "{w}×{h} 另一把的账龄应在场:{s}");
            assert!(
                s.contains("轮换日 2025-11-02"),
                "{w}×{h} 轮换日原文应在场:{s}"
            );
            for ln in s.lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 有行超宽:{ln}"
                );
            }
        }
    }

    /// 账龄列只标记不治理:超阈的那条亮 amber,未超的那条不亮,册面其余读法
    /// (主机 / 作用域 / 轮换日原文)逐字节不变。
    #[test]
    fn 钥匙册_账龄超阈亮amber_未超不亮() {
        pin_today_for_test(2026, 10, 13);
        let (mut app, _r) = key_app(&[("key ls", 0, KEY_LS_AGE_MIX)]);
        app.nw_home = "/nw".into();
        tap(&mut app, KeyCode::Char('l'));
        feed(&mut app);
        let p = app.palette;
        assert_ne!(
            p.warn().fg,
            p.dim().fg,
            "两种样式得能分辨,否则这条什么都没测"
        );
        let old = crate::testutil::frame_span_fg(&app, 80, 30, "账龄 345 天");
        let fresh = crate::testutil::frame_span_fg(&app, 80, 30, "账龄 1 天");
        assert_eq!(old, p.warn().fg, "超阈账龄应亮 amber");
        assert_eq!(fresh, p.dim().fg, "未超阈账龄不应亮");
        assert_ne!(fresh, p.warn().fg);
        // 轮换日原文不跟着变色(只标记账龄,不动册面其余读法)
        assert_eq!(
            crate::testutil::frame_span_fg(&app, 80, 30, "2025-11-02"),
            p.text().fg,
            "轮换日原文仍是正文色"
        );
    }

    /// 没给轮换日 = 「账龄 未知」,不是「账龄 0 天」;也不该把整册带炸。
    #[test]
    fn 钥匙册_没给轮换日_账龄未知且不顶替0天() {
        pin_today_for_test(2026, 10, 13);
        let ls = r#"{"schema":"nw/key-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/keys.json","keys":[{"name":"ci","kind":"github-key","host":"ghe.internal","scopes":[]}]}}"#;
        let (mut app, _r) = key_app(&[("key ls", 0, ls)]);
        app.nw_home = "/nw".into();
        tap(&mut app, KeyCode::Char('l'));
        feed(&mut app);
        let s = crate::testutil::frame_display(&app, 80, 30);
        assert!(s.contains("账龄 未知"), "没给轮换日应说未知:{s}");
        assert!(!s.contains("账龄 0 天"), "不许拿 0 天冒充不知道:{s}");
    }

    /// 四本册走同一条键位路径(#66 验收第 4 条):对四本册各驱动一次**相同**
    /// 的键序列(`l` 重读 → `↓` → `↑`),断言回音形状同形——
    /// ① 每本都只下发一条只读 argv,形状都是 `<组> ls --root <根>`;
    /// ② 每本都只多一行日志,都带「已入队」+「只读」+「不写任何文件」;
    /// ③ 选中行增量相同(`↓` = +1、`↑` = −1,与行数无关);
    /// ④ 帧上「当前是哪一本」与册内键位行都在,两档都不溢宽。
    ///
    /// 收敛之前这四段是四份逐字重复的分叉(每本一条 `if kind ==`),改一本
    /// 漏三本是常态;收敛之后四本册同一条代码路径。谁再把某一本单独拆出去
    /// (在调用方插一个 `if kind ==`),这条就红。
    #[test]
    fn 四本册同一键序列_回音与日志同形() {
        for (label, mut app) in book_apps() {
            let root = app.usable_data_root().unwrap_or_default();
            let group = match label {
                "机器" => "vps",
                "标签" => "tag",
                "订阅" => "subscription",
                _ => "key",
            };
            let log_before = app.log.len();
            // ① l:一条只读 argv,形状同形
            tap(&mut app, KeyCode::Char('l'));
            let argv = app.queue.last().cloned().unwrap_or_default();
            assert_eq!(argv.len(), 4, "{label}册 ls 的 argv 四段");
            assert_eq!(argv[0], group, "{label}册 ls 打到自己的组");
            assert_eq!(argv[1], "ls", "{label}册 ls 是只读子命令");
            assert_eq!(argv[2], "--root", "{label}册 ls 只认 --root(不猜目录)");
            assert_eq!(argv[3], root, "{label}册 ls 的根是就绪的数据仓根");
            // ② 一行日志,同一形状(四本措辞各有各的限定词,形状不变)
            assert_eq!(app.log.len(), log_before + 1, "{label}册 l 只留一行日志");
            let line = app.log.first().cloned().unwrap_or_default(); // 最新一行在前
            assert!(line.contains("ls 已入队"), "{label}册日志说清入队:{line}");
            assert!(line.contains("只读"), "{label}册日志说清只读:{line}");
            assert!(
                line.contains("不写任何文件"),
                "{label}册日志说清不写:{line}"
            );
            feed(&mut app);
            // ③ ↑/↓ 选中增量与行数无关(到界绕回是另一条口径,这里只测增量)
            let sel0 = app.catalog_sel();
            tap(&mut app, KeyCode::Down);
            assert_eq!(app.catalog_sel(), sel0 + 1, "{label}册 ↓ 下移一行");
            tap(&mut app, KeyCode::Up);
            assert_eq!(app.catalog_sel(), sel0, "{label}册 ↑ 上移一行");
            // ④ 帧上两样都在:当前是哪一本 + 册内键位行
            for (w, h) in [(80u16, 30u16), (66, 50)] {
                let f = crate::testutil::frame_display(&app, w, h);
                assert!(
                    f.contains(&format!("当前册 {label}")),
                    "{label}册 {w}×{h} 报出当前册"
                );
                assert!(
                    f.lines()
                        .any(|ln| ln.contains("Esc/q 返航") || ln.contains("m 换册")),
                    "{label}册 {w}×{h} 键位行在场:{f}"
                );
                for ln in f.lines() {
                    assert!(
                        unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                        "{label}册 {w}×{h} 有行超宽:{ln}"
                    );
                }
            }
        }
    }

    /// 「一条只在一本册上成立的关切,落在查表成员上即生效」(#66 验收第 5 条)。
    ///
    /// 钉两件真事,不靠口头约定:`g`(取回本体的建议命令)只属于订阅册与
    /// 钥匙册——投影里有本体槽的两本(ADR-0009 §1 面板不代跑 get);
    /// `Enter`(反查)属于标签册与机器册——标签册带参反查选中的那条(#54)、
    /// 机器册无参列全部再本地精确匹配「什么标签引用了这一条」(#70;`vps`
    /// 在 `TAG_REF_KINDS` 里,机器条目可打标签)。订阅 / 钥匙册的条目不在
    /// `TAG_REF_KINDS` 里,没有这一关切。对四本册各按一次:有这一关切的给
    /// 各人话的一行(标签册入队 / 机器册门禁指路),没有的这一关切的两本
    /// **一个字节都不多打**。谁把这一条挪出查表成员、或往调用方插一个
    /// `if kind ==`,这条测试就红。
    #[test]
    fn 只属一两本的关切_按册各按一次_表外一本都不多发() {
        for (label, mut app) in book_apps() {
            let log_before = app.log.len();
            let sheet_before = app.catalog_len();
            let sel_before = app.catalog_sel();
            // g:只有订阅册 / 钥匙册有这一条关切
            tap(&mut app, KeyCode::Char('g'));
            let has_g = matches!(label, "订阅" | "钥匙");
            if has_g {
                // 两行:建议命令 + 「get 是上游唯一回显本体的出口」那句纪律注
                assert_eq!(app.log.len(), log_before + 2, "{label}册 g 留两行");
                // 日志最新在前:先入队的是建议命令那一行,纪律注压在它上面
                let line = app.log.get(1).cloned().unwrap_or_default();
                assert!(
                    line.starts_with(if label == "订阅" {
                        "取回链接"
                    } else {
                        "取回钥匙"
                    }),
                    "{label}册 g 给的是自己那一本的取回建议:{line}"
                );
                assert!(
                    app.log[0].contains("上游唯一回显本体的出口"),
                    "{label}册 g 同时留一句纪律注:{s}",
                    s = app.log[0]
                );
            } else {
                assert_eq!(
                    app.log.len(),
                    log_before,
                    "{label}册没有 g 这一关切:一字不多打"
                );
            }
            assert!(app.queue.is_empty(), "{label}册 g 只给命令文案,不下发");
            // g 是纯读关切:册体与选中一个字节都不动(四本同一口径)
            assert_eq!(app.catalog_len(), sheet_before, "{label}册 g 不改册体");
            assert_eq!(app.catalog_sel(), sel_before, "{label}册 g 不改选中");
            // Enter 反查:标签册(#54 旧态)与机器册(#70 上浮)都有这一关切,
            // 但两本读法不同——标签册带参反查选中的那条,机器册无参列全部再
            // 本地按 `<kind>/<条目>` 精确匹配。订阅 / 钥匙册的条目不在
            // TAG_REF_KINDS 里,没有这一关切(表外一本都不多发)。
            tap(&mut app, KeyCode::Enter);
            if label == "标签" {
                let argv = app.queue.last().cloned().unwrap_or_default();
                assert_eq!(argv[0], "tag", "标签册 Enter 走 tag 组");
                assert_eq!(argv[1], "ls", "标签册 Enter 是只读反查");
                assert_eq!(argv[2], "bare", "反查选中的这一条(排序后首行)");
                assert_eq!(argv[3], "--root", "反查只认 --root");
                assert_eq!(
                    app.log.len(),
                    log_before + 1,
                    "标签册 Enter 只留一行入队日志"
                );
                // 在途清空上一轮结论是 #54 既定语义(不拿旧结论冒充本次),
                // 不是这两键「改了册体内容」——回音到达才重新摊开。
                assert_eq!(app.catalog_len(), 0, "反查在途不拿上一轮结论冒充本次");
            } else if label == "机器" {
                // 机器册反查(#70 上浮):本夹具只记了 vps 能力、没记 tag 能力,
                // 所以门禁在「标签组能力」这道上指路——**一次引擎调用都不发**,
                // 只留一行人话。门禁顺序与 zero-call 由 #70 专测钉,这里只证
                // 「这一本确有 Enter 关切」(不再是 #66 时的「Enter 无声」)。
                assert!(app.queue.is_empty(), "机器册反查门不过:零引擎调用");
                assert_eq!(
                    app.log.len(),
                    log_before + 1,
                    "机器册 Enter 反查门不过留一行指路"
                );
                assert_eq!(app.catalog_len(), sheet_before, "机器册反查门不过不改册体");
                assert_eq!(app.catalog_sel(), sel_before, "机器册反查门不过不改选中");
            } else {
                assert!(
                    app.queue.is_empty(),
                    "{label}册没有 Enter 反查:一个字节都不多打"
                );
                assert_eq!(
                    app.log.len(),
                    log_before + 2 * usize::from(has_g),
                    "{label}册 Enter 无声"
                );
                assert_eq!(app.catalog_len(), sheet_before, "{label}册册体原样");
                assert_eq!(app.catalog_sel(), sel_before, "{label}册选中原样");
            }
        }
    }

    /// `catalog_filter: [String; 4]` 与 `CatalogKind::idx()` 的耦合钉在这儿
    /// (#65 评审第 9 条):第五本册子若只给 `idx()` 加了第 5 个下标、却忘了把
    /// 数组放宽成 5,`catalog_filter()[idx]` 就是一次**运行期索引 panic**,不是
    /// 编译错误——编译期一处都不红,单测跑到那一本才炸。这条按换册环(`next()`)
    /// 把每一本都走一遍,断言每本的 `idx()` 都落在那只数组的长度里;顺带钉各本
    /// 占的槽两两不同(重槽 = 两本册子共用一个过滤词,规格故事 24「按册各留一份」
    /// 就破了)。走环而不写死四本清单,是为了第五本真来时这条自己会长。
    #[test]
    fn 每本册的idx都落进过滤词数组_且各本不重槽() {
        let slots =
            crate::app::App::with_runner(std::sync::Arc::new(crate::engine::FakeRunner::new(&[])))
                .catalog_filter
                .len();
        let mut seen: Vec<usize> = Vec::new();
        let mut k = CatalogKind::Vps;
        for _ in 0..slots {
            let i = k.idx();
            assert!(
                i < slots,
                "{} 册的 idx()={i} 超出过滤词数组长度 {slots}:加第五本册子时要把 `catalog_filter: [String; N]` 一起放宽",
                k.label()
            );
            assert!(!seen.contains(&i), "{} 册与别的册重槽(idx={i})", k.label());
            seen.push(i);
            k = k.next();
        }
        assert_eq!(seen.len(), slots, "换册环一圈走过的册数 = 数组长度");
    }

    // ---- #69 册内过滤:四本册都能 `/` 过滤,纯本地筛选(与 #61 五域同源同形) ----
    //
    // 全部走在公开缝上:`App::update(Message::Key/Mouse)` 驱动、经 FakeRunner
    // 回放,断言落在 frame_display 帧、可观察状态后果(catalog_visible /
    // catalog_sel / selected_*_name / catalog_filter)、命中区(FakeRunner 的
    // call_argvs 探针)上——不断言私有签名或字段名。每本册用 book_apps() 的
    // 就绪态,vps 另用直接灌 vps_ls 的 vps_book 便于自造条数。

    /// 建一本就绪的机器册:machines 是 VpsLsData 的 machines JSON 数组。
    /// 直接灌 vps_ls(不下发命令),FakeRunner 只为 `/ 过滤` 后的 `l` 刷新备用。
    fn vps_book(machines: &str) -> crate::app::App {
        let ls = format!(r#"{{"catalogPath":"/r/vps.json","machines":{machines}}}"#);
        let mut app = crate::app::App::with_runner(std::sync::Arc::new(
            crate::engine::FakeRunner::new(&[("vps ls", 0, ls.as_str())]),
        ));
        app.splash = false;
        app.screen = crate::app::Screen::Vps;
        app.capabilities.record(
            crate::engine::Capability::Vps,
            crate::engine::Probe::Available,
        );
        app.vps_ls = Some(serde_json::from_str(ls.as_str()).unwrap());
        app
    }

    /// 进过滤输入态、敲词、收词(走真实键路,与 #61 filter_type 同一条)。连敲多
    /// 词先 Esc 清掉上一轮(过滤词按册各留一份,这里只清当前这本;不在输入态时
    /// Esc 才走清词路,与 #61 口径一致)。
    fn 过滤敲词(app: &mut crate::app::App, q: &str) {
        if !app.catalog_filter().is_empty() {
            tap(app, KeyCode::Esc);
        }
        tap(app, KeyCode::Char('/'));
        for c in q.chars() {
            tap(app, KeyCode::Char(c));
        }
        tap(app, KeyCode::Enter);
    }

    /// 左键点中某命中目标(坐标取自布局模型,点哪拿哪;与渲染同源)。
    fn 点中(
        app: &mut crate::app::App,
        w: u16,
        h: u16,
        pred: impl Fn(&crate::layout::Target) -> bool,
    ) {
        app.size = Rect::new(0, 0, w, h);
        let r = crate::layout::hitmap(app.size, app, crate::layout::size_class(app.size))
            .into_iter()
            .find(|(_, t)| pred(t))
            .map(|(r, _)| r)
            .expect("该目标应有命中区");
        app.update(crate::app::Message::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: r.x + r.width / 2,
            row: r.y + r.height / 2,
            modifiers: KeyModifiers::NONE,
        }));
    }

    /// 喂一条 vps ls 回音(走 `on_echo_ls` → `CatalogKind::absorb_ls`,即 `l` 刷新的收敛路径;
    /// FakeRunner 每次回放同一 fixture,故自造回音面来验「刷新后选中钳回可见行数」)。
    fn feed_vps_echo(app: &mut crate::app::App, machines: &str) {
        let data = format!(r#"{{"catalogPath":"/r/vps.json","machines":{machines}}}"#);
        let env = format!(
            r#"{{"schema":"nw/vps-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{data}}}"#
        );
        let out = crate::engine::EngineOutput {
            args: vec!["vps".into(), "ls".into(), "--root".into(), "/tmp".into()],
            exit_code: crate::engine::ExitCode::from_number(0),
            ok: true,
            stderr_tail: vec![],
            envelope: serde_json::from_str(&env).ok(),
            raw_stdout: env,
            parse_error: None,
        };
        app.update(crate::app::Message::Engine(
            crate::engine::EngineEvent::Finished(Box::new(out)),
        ));
    }

    /// 规格验收 1 / 故事 18:过滤收窄后再放宽,册体回到与过滤前**逐字节同序**
    /// (无关键词时 vis = 0..n 全量同序,面板永不重排引擎输出)。
    #[test]
    fn 过滤收窄再放宽_册体回到逐字节同序() {
        let mut app = book("机器");
        let before = crate::testutil::frame_display(&app, 80, 30);
        过滤敲词(&mut app, "gate");
        let narrowed = crate::testutil::frame_display(&app, 80, 30);
        assert_ne!(
            narrowed, before,
            "收窄后册体确实变了(bare 被滤掉、标题多命中数)"
        );
        assert!(narrowed.contains("命中 1/2"), "收窄显命中/总数:{narrowed}");
        assert!(!narrowed.contains("bare"), "收窄后 bare 不在场:{narrowed}");
        // Esc 清词放宽:册体回到与过滤前逐字节同序
        tap(&mut app, KeyCode::Esc);
        let after = crate::testutil::frame_display(&app, 80, 30);
        assert_eq!(after, before, "清空过滤后册体与过滤前逐字节同序");
    }

    /// 规格验收 5 / 故事 20-21:过滤空与「册体本来是空的」两态可区分,各有不同
    /// 文案与下一步,互不冒充(绝不共用一个空列表)。
    #[test]
    fn 过滤空与册体本来空_两态可区分_互不冒充() {
        // 册体本来空:0 台机器
        let empty = vps_book("[]");
        let f_empty = crate::testutil::frame_display(&empty, 80, 30);
        assert!(
            f_empty.contains("编目里还没有机器"),
            "册体真空的措辞:{f_empty}"
        );
        assert!(
            !f_empty.contains("无命中"),
            "册体真空不冒充过滤空:{f_empty}"
        );
        // 过滤空:有条目但被全筛掉
        let mut filtered = book("机器");
        过滤敲词(&mut filtered, "不存在的关键词");
        let f_filtered = crate::testutil::frame_display(&filtered, 80, 30);
        assert!(f_filtered.contains("无命中"), "过滤空的措辞:{f_filtered}");
        assert!(f_filtered.contains("没有命中"), "过滤空给事实:{f_filtered}");
        assert!(
            f_filtered.contains("按 / 改词"),
            "过滤空给改词出路:{f_filtered}"
        );
        assert!(
            f_filtered.contains("Esc 清空过滤"),
            "过滤空给清滤出路:{f_filtered}"
        );
        assert!(
            !f_filtered.contains("编目里还没有"),
            "过滤空不冒充册体真空:{f_filtered}"
        );
    }

    /// 规格验收 3 / 故事 16:大小写不敏感匹配。
    #[test]
    fn 过滤大小写不敏感() {
        let mut app = book("机器"); // gate / bare
        过滤敲词(&mut app, "GATE");
        assert_eq!(app.catalog_visible().len(), 1, "全大写关键词命中小写名");
        assert_eq!(app.selected_vps_name().as_deref(), Some("gate"));
        过滤敲词(&mut app, "GaTe");
        assert_eq!(app.catalog_visible().len(), 1, "混合大小写同样命中");
    }

    /// 规格验收 3 / 故事 17:占位符「——」与引擎未给出的字段不参与匹配——bare
    /// 没有 ips/logins/note,明细画「——」,但按「——」过滤不该命中它。
    #[test]
    fn 占位符与缺字段不参与匹配() {
        let mut app = book("机器"); // gate(有字段) / bare(全缺 → 明细画 ——)
        过滤敲词(&mut app, "——");
        assert_eq!(
            app.catalog_visible().len(),
            0,
            "「——」不冒充可检索内容:bare 的缺字段不入匹配面"
        );
        assert!(app.selected_vps_name().is_none(), "无可见行");
    }

    /// 规格验收 4 / 故事 15(硬验收):过滤全程 FakeRunner 收到的 argv 里**不含
    /// 关键词**。过滤是纯本地筛选,不打新命令;唯一那条 `l` 只读 argv 也不含词。
    #[test]
    fn 过滤全程_下发argv不含过滤词() {
        let ls = r#"{"catalogPath":"/r/vps.json","machines":[{"name":"gate","ips":[],"port":0,"logins":[]},{"name":"bare","ips":[],"port":0,"logins":[]}]}"#;
        let runner = std::sync::Arc::new(crate::engine::FakeRunner::new(&[("vps ls", 0, ls)]));
        let mut app = crate::app::App::with_runner(runner.clone());
        app.splash = false;
        app.screen = crate::app::Screen::Vps;
        app.capabilities.record(
            crate::engine::Capability::Vps,
            crate::engine::Probe::Available,
        );
        app.data_root = Some(crate::testutil::stable_root());
        // 真刷新一次:入队并回放,让 FakeRunner 记到一条 argv
        tap(&mut app, KeyCode::Char('l'));
        feed(&mut app);
        let before = runner.call_argvs().len();
        assert_eq!(before, 1, "刷新只打过一条只读 argv");
        // 过滤敲词(纯本地):不应再打任何命令
        过滤敲词(&mut app, "gate");
        let argvs = runner.call_argvs();
        assert_eq!(argvs.len(), before, "过滤是纯本地,不打新命令");
        for a in &argvs {
            assert!(
                !a.iter().any(|s| s.contains("gate")),
                "没有任何一条下发 argv 含过滤词:{a:?}"
            );
        }
    }

    /// 规格验收 6 / 故事 22:Esc 清词并让选中回顶。
    #[test]
    fn 逃逸键清词并让选中回顶() {
        let machines = r#"[{"name":"gate-a","ips":[],"port":0,"logins":[]},{"name":"gate-b","ips":[],"port":0,"logins":[]},{"name":"gate-c","ips":[],"port":0,"logins":[]}]"#;
        let mut app = vps_book(machines);
        过滤敲词(&mut app, "gate");
        assert_eq!(app.catalog_visible().len(), 3, "三条都含 gate");
        tap(&mut app, KeyCode::Down);
        tap(&mut app, KeyCode::Down);
        assert_eq!(app.catalog_sel(), 2, "选中挪到第 3 个可见行");
        tap(&mut app, KeyCode::Esc); // 清词 + 回顶
        assert_eq!(app.catalog_filter(), "", "Esc 清空过滤词");
        assert_eq!(app.catalog_sel(), 0, "Esc 让选中回顶");
        assert_eq!(app.catalog_visible().len(), 3, "清词后回到全量");
    }

    /// 规格验收 6 / 故事 23:切屏退输入态但留词。输入态里唯一能绕开吞键守卫的
    /// 是鼠标点导航——点它切屏并退输入态,过滤词留着;切回编目屏词还在、仍生效。
    #[test]
    fn 输入态里点导航切屏_退输入态留词() {
        let mut app = book("机器");
        tap(&mut app, KeyCode::Char('/'));
        for c in "gate".chars() {
            tap(&mut app, KeyCode::Char(c));
        }
        assert!(app.catalog_filter_on, "在输入态");
        点中(&mut app, 80, 30, |t| {
            matches!(t, crate::layout::Target::NavTo(Screen::Overview))
        });
        assert_eq!(app.screen, Screen::Overview, "点导航切到概览");
        assert!(!app.catalog_filter_on, "切屏退输入态");
        assert_eq!(app.catalog_filter(), "gate", "过滤词留着");
        // 切回编目屏:词还在、仍生效(gate 命中 1)
        点中(&mut app, 80, 30, |t| {
            matches!(t, crate::layout::Target::NavTo(Screen::Vps))
        });
        assert_eq!(app.screen, Screen::Vps);
        assert_eq!(app.catalog_filter(), "gate", "切回编目屏过滤词还在");
        assert_eq!(app.catalog_visible().len(), 1, "过滤词仍生效");
    }

    /// 规格验收 6 / 故事 23(键路):收词退输入态后,壳级切屏(j/k)不碰过滤词,
    /// 绕回编目屏词仍生效——与 #61 五域同一条后果链。
    #[test]
    fn 切屏留词_绕回编目仍生效() {
        let mut app = book("机器");
        过滤敲词(&mut app, "gate");
        assert!(!app.catalog_filter_on, "收词后已退输入态");
        tap(&mut app, KeyCode::Char('j')); // Vps → Settings
        assert_ne!(app.screen, Screen::Vps, "切走");
        assert!(!app.catalog_filter_on, "切屏后退输入态");
        tap(&mut app, KeyCode::Char('k')); // Settings → Vps
        assert_eq!(app.screen, Screen::Vps, "绕回编目屏");
        assert_eq!(app.catalog_filter(), "gate", "过滤词没被切屏清掉");
        assert_eq!(app.catalog_visible().len(), 1, "过滤词仍生效");
    }

    /// 规格验收 6 / 故事 24:换册(m)过滤词按册各留各的——机器册筛「gate」、切到
    /// 钥匙册筛「deploy」、回头机器册还在「gate」上。
    #[test]
    fn 换册各留各的词() {
        let mut app = book("机器"); // [gate, bare]
                                    // 同一本 app 再装钥匙册数据(机器册已在场)
        let key_ls = r#"{"schema":"nw/key-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/keys.json","keys":[{"name":"ci","kind":"github-key","host":"ghe.internal","scopes":[],"rotatedAt":"2025-11-02"},{"name":"deploy","kind":"github-key","host":"github.com","scopes":["repo"],"rotatedAt":"2026-01-15"}]}}"#;
        let key_echo = crate::engine::EngineOutput {
            args: vec!["key".into(), "ls".into(), "--root".into(), "/tmp".into()],
            exit_code: crate::engine::ExitCode::from_number(0),
            ok: true,
            stderr_tail: vec![],
            envelope: serde_json::from_str(key_ls).ok(),
            raw_stdout: key_ls.to_string(),
            parse_error: None,
        };
        app.update(crate::app::Message::Engine(
            crate::engine::EngineEvent::Finished(Box::new(key_echo)),
        ));
        app.capabilities.record(
            crate::engine::Capability::Key,
            crate::engine::Probe::Available,
        );
        // 机器册筛 gate
        过滤敲词(&mut app, "gate");
        assert_eq!(app.catalog_filter(), "gate");
        assert_eq!(app.catalog_visible().len(), 1);
        // 切到钥匙册(Vps→Tag→Sub→Key)
        for _ in 0..3 {
            tap(&mut app, KeyCode::Char('m'));
        }
        assert_eq!(app.catalog_kind, CatalogKind::Key, "切到钥匙册");
        assert_eq!(app.catalog_filter(), "", "钥匙册各留各的词:起初空");
        // 钥匙册筛 deploy
        过滤敲词(&mut app, "deploy");
        assert_eq!(app.catalog_filter(), "deploy");
        assert_eq!(app.catalog_visible().len(), 1, "deploy 命中 1 把钥匙");
        // 切回机器册:`next()` 的环在 Key → Vps 处合上,所以从钥匙册回机器册
        // 只要**一次** `m`(Vps→Tag→Sub→Key 三步、Key→Vps 一步,环不是对称的)。
        tap(&mut app, KeyCode::Char('m'));
        assert_eq!(app.catalog_kind, CatalogKind::Vps, "切回机器册");
        assert_eq!(app.catalog_filter(), "gate", "回头机器册还在 gate 上");
        assert_eq!(app.catalog_visible().len(), 1, "gate 过滤仍生效");
    }

    /// 规格验收 6 / 故事 25:刷新(l)后选中钳回可见行数内(不越界、不指向看不见的行)。
    #[test]
    fn 刷新后_选中钳回可见行数内() {
        let machines = r#"[{"name":"gate-a","ips":[],"port":0,"logins":[]},{"name":"gate-b","ips":[],"port":0,"logins":[]},{"name":"gate-c","ips":[],"port":0,"logins":[]}]"#;
        let mut app = vps_book(machines);
        过滤敲词(&mut app, "gate");
        assert_eq!(app.catalog_visible().len(), 3);
        tap(&mut app, KeyCode::Down);
        tap(&mut app, KeyCode::Down);
        assert_eq!(app.catalog_sel(), 2, "选中在可见位 2");
        // 刷新回音只剩 gate-a 含 gate(其余换名):可见数塌到 1,选中必须钳回 0
        feed_vps_echo(
            &mut app,
            r#"[{"name":"gate-a","ips":[],"port":0,"logins":[]},{"name":"solo","ips":[],"port":0,"logins":[]}]"#,
        );
        assert_eq!(app.catalog_visible().len(), 1, "刷新后只剩 gate-a 含 gate");
        assert_eq!(app.catalog_sel(), 0, "选中钳回可见行数内");
    }

    /// 规格验收 7 / 故事 27:过滤态下删除作用于「看得见的那一行」——滤到只剩
    /// gate(原始下标 1),按 d 起的删除确认点名 gate,不是被滤掉的 bare。
    #[test]
    fn 过滤态下_删除作用于看得见的那一行() {
        let mut app = book("机器"); // 排序后 [bare(0), gate(1)]
        过滤敲词(&mut app, "gate");
        assert_eq!(app.catalog_visible(), vec![1], "只剩 gate(原始下标 1)可见");
        tap(&mut app, KeyCode::Char('d'));
        assert!(app.vps_rm_confirm, "d 起删除确认");
        assert_eq!(
            app.selected_vps_name().as_deref(),
            Some("gate"),
            "删除确认点名看得见的 gate,不是被滤掉的 bare"
        );
        let f = crate::testutil::frame_display(&app, 80, 30);
        assert!(f.contains("gate"), "删除确认框点名 gate:{f}");
    }

    /// 规格验收 7 / 故事 26:过滤态下鼠标点行与键盘同一本账——点可见第 i 行 =
    /// 选中可见第 i 行(下标经 catalog_visible 映射回原始数组)。
    #[test]
    fn 过滤态下_鼠标点行作用于看得见的那一行() {
        let machines = r#"[{"name":"aaa","ips":[],"port":0,"logins":[]},{"name":"gate-x","ips":[],"port":0,"logins":[]},{"name":"gate-y","ips":[],"port":0,"logins":[]}]"#;
        let mut app = vps_book(machines); // 排序后 [aaa(0), gate-x(1), gate-y(2)]
        过滤敲词(&mut app, "gate"); // 可见 [1, 2]
        assert_eq!(app.catalog_visible(), vec![1, 2]);
        // 点可见第 1 行 = 原始下标 1 的 gate-x(不是原始下标 0 的 aaa)
        点中(&mut app, 80, 30, |t| {
            matches!(t, crate::layout::Target::SelectRow(0))
        });
        assert_eq!(
            app.selected_vps_name().as_deref(),
            Some("gate-x"),
            "点可见第 1 行"
        );
        // 点可见第 2 行 = 原始下标 2 的 gate-y
        点中(&mut app, 80, 30, |t| {
            matches!(t, crate::layout::Target::SelectRow(1))
        });
        assert_eq!(
            app.selected_vps_name().as_deref(),
            Some("gate-y"),
            "点可见第 2 行"
        );
        // 键盘从第 2 行按 ↑ 也回可见第 1 行(键鼠同一本账)
        tap(&mut app, KeyCode::Up);
        assert_eq!(
            app.selected_vps_name().as_deref(),
            Some("gate-x"),
            "↑ 与点行同一本账"
        );
    }

    /// 规格验收 8 / 故事 28:过滤态是输入态——期间动作键(m 换册 / d 删除 /
    /// r 登记 / j 切屏)不生效,只当字符进过滤缓冲;与 #61 五域同一形态。
    #[test]
    fn 过滤态是输入态_期间动作键不生效() {
        let mut app = book("机器");
        tap(&mut app, KeyCode::Char('/')); // 进输入态
        assert!(app.catalog_filter_on);
        for c in ['m', 'd', 'r', 'j'] {
            tap(&mut app, KeyCode::Char(c));
        }
        assert_eq!(app.catalog_kind, CatalogKind::Vps, "输入态里 m 不换册");
        assert!(!app.vps_rm_confirm, "输入态里 d 不起删除确认");
        assert_eq!(app.screen, Screen::Vps, "输入态里 j 不切屏(壳级键也被吞)");
        assert_eq!(app.catalog_filter(), "mdrj", "动作键在输入态只进过滤缓冲");
    }

    /// 规格验收 9 / 故事 29:过滤入口有鼠标命中,点中 = 按 `/`。机器册键位行够短,
    /// `/ 过滤` 段在常畅(80×30)与紧凑(66×50)两档都放得下、都点得中。
    #[test]
    fn 过滤入口段_点中等于按斜杠_机器册两档可点() {
        for (w, h) in [(80u16, 30u16), (66, 50)] {
            let mut app = book("机器");
            app.size = Rect::new(0, 0, w, h);
            let hits = crate::layout::hitmap(app.size, &app, crate::layout::size_class(app.size));
            assert!(
                hits.iter()
                    .any(|(_, t)| *t == crate::layout::Target::CatalogFilter),
                "{w}×{h} 过滤段应有命中区"
            );
            assert!(
                crate::testutil::frame_display(&app, w, h).contains("/ 过滤"),
                "{w}×{h} 屏上也该画出这一段(点哪打哪)"
            );
            点中(&mut app, w, h, |t| {
                *t == crate::layout::Target::CatalogFilter
            });
            assert!(app.catalog_filter_on, "{w}×{h} 点过滤段 = 进输入态(同按 /)");
        }
    }

    /// 帧上某一行的**显示列**(列按显示宽累计——`frame_display` 已跳过宽字
    /// 续格,字节偏移 ≠ 终端列,与命中区同一把尺)。
    fn display_col(row: &str, byte: usize) -> u16 {
        use unicode_width::UnicodeWidthChar;
        row[..byte]
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0) as u16)
            .sum()
    }

    /// 规格验收 9 / 故事 29 + ADR-0005「渲染与命中同源」:鼠标过滤入口
    /// **四本册、两档都在场**。#69 只给机器册做成了这件事——`key_line_segments`
    /// 的「放不下整段收口」把标签 / 订阅 / 钥匙三本册的 `/ 过滤` 整段收掉
    /// (键位行太长),于是键盘 `/` 四本册都能按、鼠标却只有机器册点得着;
    /// 旧版这条测试把那个取舍当既成事实钉着(#69 后续:#70 评审把它判成
    /// 「该补齐的 parity」,工单已批准)。
    ///
    /// 现在键位行在 66×50 下限(正文 64 列)内放得下全部段:分隔
    /// [`CATALOG_KEY_GAP`] 由 3 列收窄到 2 列(` · ` → ` ·`,一个常量、渲染与
    /// 命中共用,不丢任何一段文案),订阅 / 钥匙册的 `g 取回命令` 再收窄成
    /// `g 取回`(全句建议命令就在册体下一行摊着)。
    ///
    /// 这一条钉四件事:① 四本册 × 两档都画得出 `/ 过滤`、都有命中矩形;
    /// ② 渲染行与命中矩形逐字同源(段起始列 == 矩形 x,段右界 == 矩形右界);
    /// ③ 键位行右侧**至少留 1 列余量**(不是恰好贴边框赌出来的);
    /// ④ 一段提示段 / 动作段都没被收掉(#67 的收口纪律仍在,只是够宽到不用收)。
    #[test]
    fn 过滤段四本册两档都放得下_渲染与命中共源() {
        for name in ["机器", "标签", "订阅", "钥匙"] {
            for (w, h) in [(80u16, 30u16), (66, 50)] {
                let mut app = book(name);
                app.size = Rect::new(0, 0, w, h);
                let size = Rect::new(0, 0, w, h);
                let f = crate::testutil::frame_display(&app, w, h);
                let hits = crate::layout::hitmap(size, &app, crate::layout::size_class(size));
                // ① 过滤段与换册段都在场、都有命中矩形,且同一行
                let filt = hits
                    .iter()
                    .find(|(_, t)| *t == crate::layout::Target::CatalogFilter)
                    .map(|(r, _)| *r)
                    .unwrap_or_else(|| panic!("{name} {w}×{h} 过滤段应有命中区"));
                let sw = hits
                    .iter()
                    .find(|(_, t)| *t == crate::layout::Target::SwitchBook)
                    .map(|(r, _)| *r)
                    .unwrap_or_else(|| panic!("{name} {w}×{h} 换册段应有命中区"));
                assert_eq!(filt.y, sw.y, "{name} {w}×{h} 两段同一行(键位行)");
                let row = f.lines().nth(filt.y as usize).expect("命中行在渲染中在场");
                assert!(
                    row.contains(CATALOG_FILTER),
                    "{name} {w}×{h} 屏上应画出过滤段:{row}"
                );
                assert!(
                    row.contains(CATALOG_SWITCH),
                    "{name} {w}×{h} 屏上应画出换册段:{row}"
                );
                // ② 渲染与命中共源:段起始列 == 矩形 x,段右界 == 矩形右界
                let byte = row.find(CATALOG_FILTER).expect("帧上有过滤段");
                let col = display_col(row, byte);
                assert_eq!(
                    col, filt.x,
                    "{name} {w}×{h} 渲染段起始列 == 命中矩形 x(点哪打哪)"
                );
                assert_eq!(
                    col + u16::try_from(unicode_width::UnicodeWidthStr::width(CATALOG_FILTER))
                        .unwrap(),
                    filt.x + filt.width,
                    "{name} {w}×{h} 渲染段右界 == 命中矩形右界(同为 unicode_width 量出的宽)"
                );
                // ③ 至少 1 列余量:过滤段右侧第一个格子是空白,不是边框
                let after = &row[byte + CATALOG_FILTER.len()..];
                assert!(
                    after.starts_with(' '),
                    "{name} {w}×{h} 键位行右侧至少留 1 列余量:{row}"
                );
                // ④ 一段都没被收掉:段清单里每一段(提示段 + 动作段)都在这一行上
                for (text, _t, _live) in catalog_key_parts(&app) {
                    assert!(
                        row.contains(text),
                        "{name} {w}×{h} 段「{text}」被收掉了(放不下就整段收口,#67):{row}"
                    );
                }
                // 点中 = 按 /(键盘那条路见输入态测试)
                点中(&mut app, w, h, |t| {
                    *t == crate::layout::Target::CatalogFilter
                });
                assert!(
                    app.catalog_filter_on,
                    "{name} {w}×{h} 点过滤段 = 进输入态(同按 /)"
                );
            }
        }
    }

    /// 规格验收 10:过滤态在两档(80×30 / 66×50)整帧渲染不溢宽;过滤空态同。
    #[test]
    fn 过滤态两档渲染不溢宽() {
        let mut app = book("机器");
        过滤敲词(&mut app, "gate");
        for (w, h) in [(80u16, 30u16), (66, 50)] {
            let f = crate::testutil::frame_display(&app, w, h);
            assert!(f.contains("命中 1/2"), "{w}×{h} 标题显命中/总数:{f}");
            for ln in f.lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 有行超宽:{ln}"
                );
            }
        }
        // 过滤空态两档也不溢宽
        过滤敲词(&mut app, "zzz-不存在");
        for (w, h) in [(80u16, 30u16), (66, 50)] {
            let f = crate::testutil::frame_display(&app, w, h);
            assert!(f.contains("无命中"), "{w}×{h} 过滤空态在场:{f}");
            for ln in f.lines() {
                assert!(
                    unicode_width::UnicodeWidthStr::width(ln) <= w as usize,
                    "{w}×{h} 过滤空态有行超宽:{ln}"
                );
            }
        }
    }

    /// 规格验收 1 / 故事 10-13:每册按各自的可匹配字段过滤(机器 名/地址/登录/
    /// 备注;标签 名/引用;订阅 名/应用/到期日;钥匙 名/主机/作用域/轮换日)。
    #[test]
    fn 四本册各自按可匹配字段过滤() {
        // 机器册:地址 / 登录 / 备注
        let mut vps = book("机器"); // gate(10.0.0.1, root→$DEPLOY_KEY, 门口) / bare
        过滤敲词(&mut vps, "10.0.0.1"); // 地址
        assert_eq!(vps.catalog_visible().len(), 1, "机器册按地址过滤");
        过滤敲词(&mut vps, "$DEPLOY_KEY"); // 登录 key_ref
        assert_eq!(vps.catalog_visible().len(), 1, "机器册按登录过滤");
        过滤敲词(&mut vps, "门口"); // 备注
        assert_eq!(vps.catalog_visible().len(), 1, "机器册按备注过滤");
        assert_eq!(vps.selected_vps_name().as_deref(), Some("gate"));
        // 标签册:引用
        let mut tag = book("标签"); // pi(引用 deep-research / vps/gate) / bare
        过滤敲词(&mut tag, "deep-research");
        assert_eq!(tag.catalog_visible().len(), 1, "标签册按引用 id 过滤");
        过滤敲词(&mut tag, "vps/gate");
        assert_eq!(tag.catalog_visible().len(), 1, "标签册按引用 kind/id 过滤");
        // 订阅册:应用 / 到期日
        let mut sub = book("订阅"); // airport/soon(Mihomo/Clash, 2099) ...
        过滤敲词(&mut sub, "Mihomo");
        assert_eq!(
            sub.catalog_visible().len(),
            2,
            "订阅册按应用过滤(airport+gone)"
        );
        过滤敲词(&mut sub, "2099");
        assert_eq!(sub.catalog_visible().len(), 2, "订阅册按到期日过滤");
        // 钥匙册:主机 / 作用域 / 轮换日
        let mut key = book("钥匙"); // ci(ghe.internal) / deploy(github.com, [repo], 2026-01-15)
        过滤敲词(&mut key, "github.com");
        assert_eq!(key.catalog_visible().len(), 1, "钥匙册按主机过滤");
        过滤敲词(&mut key, "repo");
        assert_eq!(key.catalog_visible().len(), 1, "钥匙册按作用域过滤");
        过滤敲词(&mut key, "2026-01");
        assert_eq!(key.catalog_visible().len(), 1, "钥匙册按轮换日过滤");
    }

    // ---- #73 取回键 g 的正文动作段:点「g 取回」段 = 按 g ----
    //
    // 全部走在公开缝上:`App::update(Message::Key / Message::Mouse)` 双路驱动,
    // 断言落在渲染帧、日志行、屏上「已按 g 收进日志」、命中区清单、FakeRunner
    // 收到的 argv 与调用计数上——不断言私有字段名、不断言内部方法名。「点 = 按」
    // 在本族里是**逐条比对**的:两条路各跑一遍,日志行一上一下对,不是各说各话。

    /// 就绪一本订阅册:册体有条目、数据仓根可用(临时目录)、订阅组能力记上。
    /// FakeRunner 随 App 一起返回——「零调用」那条验收要对着它数 argv。
    fn sub_book() -> (crate::app::App, std::sync::Arc<crate::engine::FakeRunner>) {
        let (mut app, runner) = sub_app(&[("subscription ls", 0, SUB_LS_ALL)]);
        tap(&mut app, KeyCode::Char('l'));
        feed(&mut app);
        (app, runner)
    }

    /// 自造一本订阅册(`entries` 是 subscriptions JSON 数组):「键位行钉末行」要
    /// 册体 1 条与册体多条两种内容量各来一次,夹具自造条数最直白。
    fn sub_book_entries(entries: &str) -> crate::app::App {
        let ls = format!(r#"{{"catalogPath":"/r/subscriptions.json","subscriptions":{entries}}}"#);
        let mut app = crate::app::App::with_runner(std::sync::Arc::new(
            crate::engine::FakeRunner::new(&[("subscription ls", 0, ls.as_str())]),
        ));
        app.splash = false;
        app.screen = crate::app::Screen::Vps;
        app.catalog_kind = CatalogKind::Subscription;
        app.capabilities.record(
            crate::engine::Capability::Subscription,
            crate::engine::Probe::Available,
        );
        app.sub_ls = Some(serde_json::from_str(ls.as_str()).unwrap());
        app
    }

    /// 「g 取回」段的命中矩形(从布局模型取,点哪拿哪;`app.size` 须先钉好)。
    fn get_rect(app: &crate::app::App) -> Rect {
        crate::layout::hitmap(app.size, app, crate::layout::size_class(app.size))
            .into_iter()
            .find(|(_, t)| *t == crate::layout::Target::CatalogGet)
            .map(|(r, _)| r)
            .expect("该册此刻应有 g 取回命中段")
    }

    /// 帧上某一行从第 `col` **显示列**起的那一小截(按显示列切,不按字符下标——
    /// 行里有中文,一字两列;命中矩形量的是显示列,两边必须同一把尺)。
    fn 帧行截取(line: &str, col: u16, n: usize) -> String {
        use unicode_width::UnicodeWidthChar;
        let mut out = String::new();
        let mut w = 0usize;
        for ch in line.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
            if w >= col as usize + n {
                break;
            }
            if w >= col as usize {
                out.push(ch);
            }
            w += cw;
        }
        out
    }

    /// 日志里**最新**一条「取回链接」行(按一次 `g` 收两行:命令 + 一句纪律;
    /// `push_log` 是头插,最新在前,所以取第一个命中)。
    fn 最后一条取回日志(app: &crate::app::App) -> String {
        app.log
            .iter()
            .find(|l| l.contains("取回链接"))
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn 点g取回段_等于按g_日志逐条相同且零调用() {
        for (w, h) in [(80u16, 30u16), (66, 50u16)] {
            let (mut clicked, r1) = sub_book();
            let (mut keyed, _r2) = sub_book();
            clicked.size = Rect::new(0, 0, w, h);
            let before = r1.call_argvs();
            点中(&mut clicked, w, h, |t| {
                *t == crate::layout::Target::CatalogGet
            });
            tap(&mut keyed, KeyCode::Char('g'));
            assert_eq!(clicked.log, keyed.log, "{w}×{h} 点 g 段与按 g 日志逐条相同");
            assert!(clicked.sub_get_logged, "{w}×{h} 屏上该翻成已收进日志");
            let f = crate::testutil::frame_display(&clicked, w, h);
            assert!(f.contains("已按 g 收进日志"), "{w}×{h} 屏上文案翻转:{f}");
            assert!(
                clicked.queue.is_empty() && clicked.fed_queue.is_empty(),
                "{w}×{h} 不入队、不下发"
            );
            assert_eq!(
                r1.call_argvs(),
                before,
                "{w}×{h} 点击路径零引擎调用(argv 逐条相同)"
            );
            assert_eq!(r1.stdin_feed_count(), 0, "{w}×{h} 没有真实子进程被拉起喂入");
        }
    }

    /// 规格验收 2 / 故事 17:再点一次 = 再按一次——日志再收,不报错、不静默。
    #[test]
    fn 再点一次g段_等于再按一次g() {
        let (mut clicked, _r) = sub_book();
        let (mut keyed, _r) = sub_book();
        clicked.size = Rect::new(0, 0, 80, 30);
        点中(&mut clicked, 80, 30, |t| {
            *t == crate::layout::Target::CatalogGet
        });
        tap(&mut keyed, KeyCode::Char('g'));
        let once = clicked.log.len();
        点中(&mut clicked, 80, 30, |t| {
            *t == crate::layout::Target::CatalogGet
        });
        tap(&mut keyed, KeyCode::Char('g'));
        assert_eq!(
            clicked.log.len(),
            once + 2,
            "再点一次再收两行(命令 + 一句纪律),不是静默无后果"
        );
        assert_eq!(clicked.log, keyed.log, "连点与连按逐条相同");
    }

    /// 规格验收 3 / 故事 9-11 / 31:段的在场条件 = 键位行变体条件,由
    /// `CatalogKind` 查表一处决定——只有订阅 / 钥匙册的**非空、非在途**变体带
    /// `g 取回` 可点段;空册变体、在途变体(登记浮层 / 删除确认)、机器册、标签册
    /// 的任何变体都没有取回命中目标。
    #[test]
    fn g取回段_只在订阅与钥匙册的非空非在途变体在场() {
        // 非空册体:两本各恰好一段,且没有禁用态(边缘态也照点、照给指路)
        for (name, mut app) in [("订阅", book("订阅")), ("钥匙", book("钥匙"))] {
            app.size = Rect::new(0, 0, 80, 30);
            let segs: Vec<_> = catalog_key_parts(&app)
                .into_iter()
                .filter(|(_, t, _)| *t == Some(crate::layout::Target::CatalogGet))
                .collect();
            assert_eq!(segs.len(), 1, "{name}册应恰好一个 g 取回段:{segs:?}");
            assert_eq!(segs[0].0, CATALOG_GET, "{name}册段文案与屏上同源");
            assert!(segs[0].2, "{name}册 g 段没有禁用态");
        }
        // 机器册 / 标签册:有条目也没有(它们的条目没有本体可取)
        for (name, app) in [("机器", book("机器")), ("标签", book("标签"))] {
            assert!(
                catalog_key_parts(&app)
                    .iter()
                    .all(|(_, t, _)| *t != Some(crate::layout::Target::CatalogGet)),
                "{name}册任何变体都不该有取回命中目标"
            );
        }
        // 空册变体(合法空态):段清单退成 keys_empty
        for (name, mut app) in [("订阅", book("订阅")), ("钥匙", book("钥匙"))] {
            let kind = app.catalog_kind;
            置空册(&mut app, kind);
            let f = crate::testutil::frame_display(&app, 80, 30);
            assert!(f.contains("编目里还没有"), "{name}册空态在场:{f}");
            assert!(
                catalog_key_parts(&app)
                    .iter()
                    .all(|(_, t, _)| *t != Some(crate::layout::Target::CatalogGet)),
                "{name}册空册变体没有取回命中目标"
            );
        }
        // 在途变体:登记浮层 / 删除确认浮层在场,整行换成浮层提示、无可点段
        for (name, mut app) in [("订阅", book("订阅")), ("钥匙", book("钥匙"))] {
            tap(&mut app, KeyCode::Char('r'));
            assert!(
                catalog_key_parts(&app)
                    .iter()
                    .all(|(_, t, _)| *t != Some(crate::layout::Target::CatalogGet)),
                "{name}册登记浮层态没有取回命中目标"
            );
            let f = crate::testutil::frame_display(&app, 80, 30);
            assert!(f.contains("Enter 确认"), "{name}册在途键位行:{f}");
            tap(&mut app, KeyCode::Esc);
            tap(&mut app, KeyCode::Char('d'));
            assert!(
                catalog_key_parts(&app)
                    .iter()
                    .all(|(_, t, _)| *t != Some(crate::layout::Target::CatalogGet)),
                "{name}册删除确认态没有取回命中目标"
            );
        }
    }

    /// 规格验收 4 / 故事 3:点段与段之间的空白不触发任何动作——日志不新增、状态不翻。
    #[test]
    fn 点g段之间的空白_无任何后果() {
        let (mut app, _r) = sub_book();
        app.size = Rect::new(0, 0, 80, 30);
        let g = get_rect(&app);
        let before_log = app.log.len();
        // g 段左侧那一列正是段间分隔(`CATALOG_KEY_GAP` = " ·")所在的空白
        app.update(crate::app::Message::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: g.x - 1,
            row: g.y,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.log.len(), before_log, "点段间空白不新增日志");
        assert!(!app.sub_get_logged, "点段间空白不翻状态");
    }

    /// 规格验收 5 / 故事 25:边缘态(册已读到、数据仓根随后被撤)——段仍在场,点击
    /// 复现键盘 `g` 的既有指路日志(「取回链接未给出:…」):不画成死人、也不给半条命令。
    #[test]
    fn 边缘态_数据仓根被撤_段仍在场_点击复现键盘g的指路() {
        let (mut clicked, _r) = sub_book();
        let (mut keyed, _r) = sub_book();
        clicked.data_root = None;
        keyed.data_root = None;
        clicked.size = Rect::new(0, 0, 80, 30);
        let g = get_rect(&clicked);
        assert!(
            crate::testutil::frame_display(&clicked, 80, 30).contains(CATALOG_GET),
            "数据仓根被撤后段仍在场"
        );
        点中(&mut clicked, 80, 30, |t| {
            *t == crate::layout::Target::CatalogGet
        });
        tap(&mut keyed, KeyCode::Char('g'));
        assert_eq!(clicked.log, keyed.log, "边缘态点击与按键逐条相同");
        assert!(
            clicked.log.iter().any(|l| l.contains("取回链接未给出")),
            "既有指路复现:{:?}",
            clicked.log.last()
        );
        assert!(!clicked.sub_get_logged, "命令没给出就不标记已收");
        assert!(clicked.queue.is_empty(), "仍是一个字节都不下发");
        assert!(g.width > 0, "段有宽度(不是空壳)");
    }

    /// 规格验收 6 / 故事 18:换选中行(`↑/↓` 与鼠标点行两条路各一次)后点 g——
    /// 命令跟着新选中行变,点击拿到的永远是屏上那一条。
    #[test]
    fn 换选中行后点g_命令跟着新选中行变() {
        // 路一:键盘 ↑/↓
        let (mut app, _r) = sub_book();
        app.size = Rect::new(0, 0, 80, 30);
        点中(&mut app, 80, 30, |t| {
            *t == crate::layout::Target::CatalogGet
        });
        assert!(
            最后一条取回日志(&app).contains("--name airport"),
            "默认选中第一条:{:?}",
            最后一条取回日志(&app)
        );
        tap(&mut app, KeyCode::Down);
        点中(&mut app, 80, 30, |t| {
            *t == crate::layout::Target::CatalogGet
        });
        assert!(
            最后一条取回日志(&app).contains("--name forever"),
            "↓ 后拿新选中那条:{:?}",
            最后一条取回日志(&app)
        );
        // 路二:鼠标点行
        let (mut app2, _r) = sub_book();
        app2.size = Rect::new(0, 0, 80, 30);
        点中(&mut app2, 80, 30, |t| {
            matches!(t, crate::layout::Target::SelectRow(2))
        });
        点中(&mut app2, 80, 30, |t| {
            *t == crate::layout::Target::CatalogGet
        });
        assert!(
            最后一条取回日志(&app2).contains("--name gone"),
            "鼠标点行后拿那一行:{:?}",
            最后一条取回日志(&app2)
        );
    }

    /// 故事 29:册内过滤(#69)不悄悄改动这一段的在场条件——过滤筛掉的只是册体
    /// 行、不是这个动作。筛到一格不剩时 g 段仍在场,点它与按 `g` 同后果(同一条
    /// 既有指路:没有可见的选中行,命令给不出)。
    #[test]
    fn 册内过滤到空_g段仍在场_点击与按键同后果() {
        let (mut clicked, _r) = sub_book();
        let (mut keyed, _r) = sub_book();
        过滤敲词(&mut clicked, "zzz-不存在");
        过滤敲词(&mut keyed, "zzz-不存在");
        clicked.size = Rect::new(0, 0, 80, 30);
        let g = get_rect(&clicked);
        let f = crate::testutil::frame_display(&clicked, 80, 30);
        assert!(f.contains("册无命中"), "册体确实被筛空:{f}");
        assert!(f.contains(CATALOG_GET), "筛到空,g 段仍在场:{f}");
        点中(&mut clicked, 80, 30, |t| {
            *t == crate::layout::Target::CatalogGet
        });
        tap(&mut keyed, KeyCode::Char('g'));
        assert_eq!(clicked.log, keyed.log, "筛到空:点击与按键逐条相同");
        assert!(
            clicked.log.iter().any(|l| l.contains("取回链接未给出")),
            "给不出命令时走既有指路:{:?}",
            clicked.log.first()
        );
        assert_eq!(g.height, 1, "g 段是 1 行可点段");
    }

    /// 规格验收 7 / 故事 7:键位行钉在册体内层**末行**——册体 1 条与册体多条两种
    /// 内容量下,g 段命中矩形的 y 相等,且那一行渲染出来真的带着 g 段、紧贴册体
    /// 下边框(不再随内容漂,也不再被内容顶出可见区)。
    #[test]
    fn 键位行钉末行_册体一条与多条_g段同一行() {
        let one = r#"[{"name":"airport","kind":"subscription-url","client":"Mihomo","expires":"2099-03-01","status":"active","daysLeft":26000}]"#;
        let many = format!(
            "[{}]",
            (0..20)
                .map(|i| format!(
                    r#"{{"name":"s{i:02}","kind":"subscription-url","client":"Mihomo","expires":"2099-03-01","status":"active","daysLeft":26000}}"#
                ))
                .collect::<Vec<_>>()
                .join(",")
        );
        let mut ys: Vec<(String, u16)> = Vec::new();
        for (label, mut app) in [
            ("1 条", sub_book_entries(one)),
            ("20 条", sub_book_entries(&many)),
        ] {
            app.size = Rect::new(0, 0, 80, 30);
            let g = get_rect(&app);
            ys.push((label.to_string(), g.y));
            let f = crate::testutil::frame_display(&app, 80, 30);
            let row = f.lines().nth(g.y as usize).unwrap_or_default();
            assert_eq!(
                帧行截取(row, g.x, unicode_width::UnicodeWidthStr::width(CATALOG_GET)),
                CATALOG_GET,
                "{label} 命中矩形那一行、那一列渲染出来就是 g 段:{row}"
            );
            let below = f.lines().nth(g.y as usize + 1).unwrap_or_default();
            assert!(
                below.contains('└'),
                "{label} 键位行紧贴册体下边框(= 内层末行):{below}"
            );
        }
        assert_eq!(
            ys[0].1, ys[1].1,
            "册体内容长短不影响键位行的 y(1 条与 20 条同一行):{ys:?}"
        );
    }

    /// 规格验收 8 / 故事 5-6 / 33:两档(80×30 / 66×50)都点得中;段在屏上的起始
    /// 列 = 命中矩形 x(渲染与命中共用同一份段表量宽),整段在场才可点。
    #[test]
    fn g段_两档都点得中_屏上那一列就是命中矩形() {
        for (w, h) in [(80u16, 30u16), (66, 50u16)] {
            let (mut app, _r) = sub_book();
            app.size = Rect::new(0, 0, w, h);
            let g = get_rect(&app);
            let f = crate::testutil::frame_display(&app, w, h);
            let row = f.lines().nth(g.y as usize).unwrap_or_default();
            assert_eq!(
                帧行截取(row, g.x, unicode_width::UnicodeWidthStr::width(CATALOG_GET)),
                CATALOG_GET,
                "{w}×{h} 段整段在场(不是半个命中):{row}"
            );
            点中(&mut app, w, h, |t| *t == crate::layout::Target::CatalogGet);
            assert!(app.sub_get_logged, "{w}×{h} 点得中");
            assert!(
                最后一条取回日志(&app).contains("nw subscription get --name airport"),
                "{w}×{h} 拿到当前选中那条的建议命令:{:?}",
                最后一条取回日志(&app)
            );
        }
    }

    /// 规格验收 9 / 故事 14:装配态下点 g 段与按 `g` 同后果——`g` 是只读,不受
    /// 装配互斥约束(键盘侧本就无 `assembling` 守卫,鼠标同口径)。
    #[test]
    fn 装配态下点g段_与按g同后果() {
        let (mut clicked, r1) = sub_book();
        let (mut keyed, _r) = sub_book();
        clicked.assembling = true;
        keyed.assembling = true;
        clicked.size = Rect::new(0, 0, 80, 30);
        let before = r1.call_argvs();
        点中(&mut clicked, 80, 30, |t| {
            *t == crate::layout::Target::CatalogGet
        });
        tap(&mut keyed, KeyCode::Char('g'));
        assert_eq!(clicked.log, keyed.log, "装配态下两条路逐条相同");
        assert!(clicked.sub_get_logged, "装配态下照旧收进日志");
        assert!(
            clicked.queue.is_empty(),
            "装配态也不入队(只读,不碰装配互斥)"
        );
        assert_eq!(r1.call_argvs(), before, "装配态点击仍然零调用");
    }

    /// 故事 19:钥匙册各在各的键位行、各取各的命令——点「g 取回」拿到的是
    /// `nw key get …`,与按 `g` 同路;屏显上永不出现钥匙本体。
    #[test]
    fn 钥匙册点g段_拿到key_get建议命令且不现本体() {
        let (mut app, runner) = key_app(&[("key ls", 0, KEY_LS_ALL)]);
        tap(&mut app, KeyCode::Char('l'));
        feed(&mut app);
        app.size = Rect::new(0, 0, 80, 30);
        let before = runner.call_argvs();
        点中(&mut app, 80, 30, |t| {
            *t == crate::layout::Target::CatalogGet
        });
        assert!(app.key_get_logged, "g 把钥匙建议命令收进日志");
        assert!(
            app.log
                .iter()
                .any(|l| l.contains("nw key get --name ci --root")),
            "建议命令收进日志可取用:{:?}",
            app.log.last()
        );
        assert!(app.queue.is_empty(), "队列里没有 key get");
        assert_eq!(runner.call_argvs(), before, "点击路径零引擎调用");
        let f = crate::testutil::frame_display(&app, 80, 30);
        assert!(!f.contains("ghp_"), "屏显永不出现钥匙本体:{f}");
    }

    // ---- #78 删除键 d 的正文动作段:点「d 删除」段 = 按 d ----
    //
    // 承接 #75 裁决 A(点段 ≡ 按键,完整复用键盘既有后果链,与 #71 的 `x` 中止段
    // 同口径)。与 #73 的 g 族同款双路驱动:点击与按键各跑一遍,日志行一上一下
    // 对;断言落在渲染帧、日志行、屏上状态、命中区清单、FakeRunner 收到的 argv
    // 与调用计数上——不断言私有字段名、不断言内部方法名。差别在后果链的终点:
    // g 的终点是日志里多一条建议命令,d 的终点是删除发起(机器 / 订阅 / 钥匙三本
    // 进 rm 确认弹层,标签册进摘引用 / 删标签表单流),确认前一个字节都不下发。

    /// 四本册的 (册名, kind, 就绪 app):非空册体、数据仓根可用、组能力记上
    /// (夹具口径见 `book_apps`)。
    fn four_books() -> Vec<(&'static str, CatalogKind, crate::app::App)> {
        [
            ("机器", CatalogKind::Vps),
            ("标签", CatalogKind::Tag),
            ("订阅", CatalogKind::Subscription),
            ("钥匙", CatalogKind::Key),
        ]
        .into_iter()
        .map(|(name, kind)| (name, kind, book(name)))
        .collect()
    }

    /// 这一本册键位行上删除段的文案:机器 / 订阅 / 钥匙三本「d 删除」,标签册是
    /// 两态的「d 摘/删」(引用留空 = 删整个标签,填 kind/条目 = 只摘那几条)。
    fn delete_text(kind: CatalogKind) -> &'static str {
        match kind {
            CatalogKind::Tag => CATALOG_DELETE_TAG,
            _ => CATALOG_DELETE,
        }
    }

    /// 「d 删除」段的命中矩形(从布局模型取,点哪拿哪;`app.size` 须先钉好)。
    fn delete_rect(app: &crate::app::App) -> Rect {
        crate::layout::hitmap(app.size, app, crate::layout::size_class(app.size))
            .into_iter()
            .find(|(_, t)| *t == crate::layout::Target::CatalogDelete)
            .map(|(r, _)| r)
            .expect("该册此刻应有 d 删除命中段")
    }

    /// 点击 / 按键之后「删除已发起」的跨册可观测量:机器 / 订阅 / 钥匙三本进
    /// rm 确认弹层(`CatalogKind::rm_confirm`,与 `catalog_key_parts` 的在途判定
    /// 同一个查表成员),标签册进表单流(`flow` 非 Idle)。四本册一个口径,
    /// 不断言私有字段名。
    fn 删除已发起(kind: CatalogKind, app: &crate::app::App) -> bool {
        match kind {
            CatalogKind::Tag => app.catalog_kind.flow(app).phase != RegPhase::Idle,
            _ => kind.rm_confirm(app),
        }
    }

    /// 把一本就绪的册换成「册体一条都没有」的**合法空态**(段清单该退成
    /// `keys_empty`,一个可点段都没有)。四本各换各的数据源——标签册的回音面是
    /// `tag_ls_echo`(EngineOutput),其余三本是解析后的数据。#73 的 g 族原先只覆盖
    /// 订阅 / 钥匙两本(各带一段 JSON),本族按册参数化后两处共用这一份。
    fn 置空册(app: &mut crate::app::App, kind: CatalogKind) {
        match kind {
            CatalogKind::Vps => {
                app.vps_ls = Some(
                    serde_json::from_str(r#"{"catalogPath":"/r/vps.json","machines":[]}"#)
                        .unwrap(),
                )
            }
            CatalogKind::Tag => {
                app.tag_ls_echo = Some(crate::engine::EngineOutput {
                    args: vec!["tag".into(), "ls".into()],
                    exit_code: crate::engine::ExitCode::Ok,
                    ok: true,
                    stderr_tail: vec![],
                    envelope: Some(
                        serde_json::from_str(
                            r#"{"schema":"nw/tag-ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"catalogPath":"/r/tags.json","tags":[]}}"#,
                        )
                        .unwrap(),
                    ),
                    raw_stdout: String::new(),
                    parse_error: None,
                })
            }
            CatalogKind::Subscription => {
                app.sub_ls = Some(
                    serde_json::from_str(
                        r#"{"catalogPath":"/r/subscriptions.json","subscriptions":[]}"#,
                    )
                    .unwrap(),
                )
            }
            CatalogKind::Key => {
                app.key_ls = Some(
                    serde_json::from_str(r#"{"catalogPath":"/r/keys.json","keys":[]}"#).unwrap(),
                )
            }
        }
    }

    /// 规格验收 1:点「d 删除」段 = 按 `d`——四本册 × 两档,两条路各跑一遍,
    /// 日志行一上一下对,删除以同一形态发起,确认闸前一个字节都不下发。屏上进
    /// 同一条流:三本画 rm 确认弹层(「确认删除」在场),标签册画表单。
    #[test]
    fn 点d删除段_等于按d_四本册两档同一后果链() {
        for (name, kind, _) in four_books() {
            for (w, h) in [(80u16, 30u16), (66, 50)] {
                let mut clicked = book(name);
                let mut keyed = book(name);
                clicked.size = Rect::new(0, 0, w, h);
                keyed.size = Rect::new(0, 0, w, h);
                assert!(!删除已发起(kind, &clicked), "{name} {w}×{h} 点前删除未发起");
                点中(&mut clicked, w, h, |t| {
                    *t == crate::layout::Target::CatalogDelete
                });
                tap(&mut keyed, KeyCode::Char('d'));
                assert_eq!(
                    clicked.log, keyed.log,
                    "{name} {w}×{h} 点 d 段与按 d 日志逐条相同"
                );
                assert!(删除已发起(kind, &clicked), "{name} {w}×{h} 点击发起删除");
                assert!(删除已发起(kind, &keyed), "{name} {w}×{h} 按键发起删除");
                assert!(
                    clicked.queue.is_empty() && clicked.fed_queue.is_empty(),
                    "{name} {w}×{h} 确认闸前一个字节都不下发"
                );
                let f = crate::testutil::frame_display(&clicked, w, h);
                let needle = match kind {
                    CatalogKind::Tag => "一行一条",
                    _ => "确认删除",
                };
                assert!(
                    f.contains(needle),
                    "{name} {w}×{h} 屏上进同一条流(浮层带着 {needle}):{f}"
                );
                let picked = kind.selected_name(&clicked).expect("删之前册里确有选中行");
                assert!(
                    clicked.log.iter().any(|l| l.contains(&picked)),
                    "{name} {w}×{h} 后果链冲着选中那一条去:{:?}",
                    clicked.log.first()
                );
            }
        }
    }

    /// 规格验收 2:删除确认弹层在场时,键位行整行换成浮层提示——删除段不在命中
    /// 区,点它原来的位置什么都不发生(日志不新增、弹层不被打扫);Esc 收回后再点
    /// 一次 = 再按一次 `d`:日志逐条相同,删除再次发起,不是静默无后果。(三本同一
    /// 口径,取订阅册为代表;标签册的表单流见标签册专属那条。)
    #[test]
    fn 弹层在场时无删除段_收回后再点等于再按() {
        let (mut clicked, _r) = sub_book();
        let (mut keyed, _r) = sub_book();
        clicked.size = Rect::new(0, 0, 80, 30);
        let rect = delete_rect(&clicked);
        点中(&mut clicked, 80, 30, |t| {
            *t == crate::layout::Target::CatalogDelete
        });
        tap(&mut keyed, KeyCode::Char('d'));
        assert!(
            clicked.sub_rm_confirm && keyed.sub_rm_confirm,
            "第一次都发起"
        );
        // 弹层态:命中区里没有删除段(整行换成浮层提示)
        assert!(
            crate::layout::hitmap(
                clicked.size,
                &clicked,
                crate::layout::size_class(clicked.size)
            )
            .iter()
            .all(|(_, t)| *t != crate::layout::Target::CatalogDelete),
            "删除确认弹层在场时没有删除命中目标"
        );
        // 点原来的位置:确认弹层够矮(7 行、居中),盖不到键位行,那一格此刻谁也不
        // 是——点击无后果(日志不新增、弹层不被打扫)
        let log_after_first = clicked.log.len();
        clicked.update(crate::app::Message::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + rect.width / 2,
            row: rect.y + rect.height / 2,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(clicked.log.len(), log_after_first, "点原位置不新增日志");
        assert!(clicked.sub_rm_confirm, "点原位置不打扫弹层");
        // Esc 收回,再点一次 = 再按一次
        tap(&mut clicked, KeyCode::Esc);
        tap(&mut keyed, KeyCode::Esc);
        点中(&mut clicked, 80, 30, |t| {
            *t == crate::layout::Target::CatalogDelete
        });
        tap(&mut keyed, KeyCode::Char('d'));
        assert_eq!(clicked.log, keyed.log, "收回后再点与再按逐条相同");
        assert!(clicked.sub_rm_confirm, "删除再次发起");
    }

    /// 规格验收 3:段的在场条件 = 键位行变体条件,由 `CatalogKind` 查表一处决定
    /// ——四本册的**非空、非在途**变体各恰好一个删除段(文案按册、无禁用态);
    /// 空册变体(合法空态)一个都没有;在途变体(登记浮层 / 删除确认浮层)整行换成
    /// 浮层提示,也一个都没有。
    #[test]
    fn d删除段_四本册非空非在途都在_空册与浮层态都不在() {
        for (name, kind, mut app) in four_books() {
            app.size = Rect::new(0, 0, 80, 30);
            let segs: Vec<_> = catalog_key_parts(&app)
                .into_iter()
                .filter(|(_, t, _)| *t == Some(crate::layout::Target::CatalogDelete))
                .collect();
            assert_eq!(segs.len(), 1, "{name}册应恰好一个删除段:{segs:?}");
            assert_eq!(segs[0].0, delete_text(kind), "{name}册段文案与屏上同源");
            assert!(
                segs[0].2,
                "{name}册删除段没有禁用态(点下去复现键盘指路,不画灰)"
            );
        }
        // 空册变体:段清单退成 keys_empty(一个可点段都没有)
        for (name, kind, mut app) in four_books() {
            置空册(&mut app, kind);
            let f = crate::testutil::frame_display(&app, 80, 30);
            assert!(f.contains("编目里还没有"), "{name}册空态在场:{f}");
            assert!(
                catalog_key_parts(&app)
                    .iter()
                    .all(|(_, t, _)| *t != Some(crate::layout::Target::CatalogDelete)),
                "{name}册空册变体没有删除命中目标"
            );
        }
        // 在途变体:登记浮层 / 删除确认浮层
        for (name, _kind, mut app) in four_books() {
            app.size = Rect::new(0, 0, 80, 30);
            tap(&mut app, KeyCode::Char('r'));
            assert!(
                catalog_key_parts(&app)
                    .iter()
                    .all(|(_, t, _)| *t != Some(crate::layout::Target::CatalogDelete)),
                "{name}册登记浮层态没有删除命中目标"
            );
            tap(&mut app, KeyCode::Esc);
            tap(&mut app, KeyCode::Char('d'));
            assert!(
                catalog_key_parts(&app)
                    .iter()
                    .all(|(_, t, _)| *t != Some(crate::layout::Target::CatalogDelete)),
                "{name}册删除确认 / 删除表单态没有删除命中目标"
            );
            let f = crate::testutil::frame_display(&app, 80, 30);
            assert!(f.contains("Enter 确认"), "{name}册在途键位行:{f}");
        }
    }

    /// 规格验收 4:点段与段之间的空白不触发任何动作——日志不新增、删除不发起。
    /// 四本册各一遍(段面文案不同,几何同构)。
    #[test]
    fn 点d段之间的空白_无任何后果() {
        for (name, kind, mut app) in four_books() {
            app.size = Rect::new(0, 0, 80, 30);
            let d = delete_rect(&app);
            let before_log = app.log.len();
            assert!(!删除已发起(kind, &app), "{name}册点前删除未发起");
            // d 段左侧那一列正是段间分隔(`CATALOG_KEY_GAP` = " ·")所在的空白:
            // 段间不重叠、不留歧义区(#67),命中矩形只覆盖段文案本身
            app.update(crate::app::Message::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: d.x - 1,
                row: d.y,
                modifiers: KeyModifiers::NONE,
            }));
            assert_eq!(app.log.len(), before_log, "{name}册点段间空白不新增日志");
            assert!(!删除已发起(kind, &app), "{name}册点段间空白不发起删除");
        }
    }

    /// 规格验收 5 / #69 口径:册内过滤筛到空时,可见列表里没有选中的那一行——
    /// 删除段仍在场(键位行按册体全长走,不随过滤收),点它与按 `d` 同后果:同一条
    /// 既有指路(「删除未下发:没有选中的…」),不弹确认、不下发、不新造行为。
    #[test]
    fn 册内过滤筛到空_点d段与按d同走既有指路() {
        let (mut clicked, _r) = sub_book();
        let (mut keyed, _r) = sub_book();
        过滤敲词(&mut clicked, "zzz-不存在");
        过滤敲词(&mut keyed, "zzz-不存在");
        clicked.size = Rect::new(0, 0, 80, 30);
        let d = delete_rect(&clicked);
        let f = crate::testutil::frame_display(&clicked, 80, 30);
        assert!(f.contains("册无命中"), "册体确实被筛空:{f}");
        assert!(f.contains(CATALOG_DELETE), "筛到空,删除段仍在场:{f}");
        点中(&mut clicked, 80, 30, |t| {
            *t == crate::layout::Target::CatalogDelete
        });
        tap(&mut keyed, KeyCode::Char('d'));
        assert_eq!(clicked.log, keyed.log, "筛到空:点击与按键逐条相同");
        assert!(
            clicked
                .log
                .iter()
                .any(|l| l.contains("删除未下发:没有选中的")),
            "给不出选中行时走既有指路:{:?}",
            clicked.log.first()
        );
        assert!(!clicked.sub_rm_confirm, "没有选中行就不弹确认");
        assert!(clicked.queue.is_empty(), "指路路径零下发");
        assert_eq!(d.height, 1, "删除段是 1 行可点段");
    }

    /// 规格验收 6:两档(80×30 / 66×50)四本册都点得中;段在屏上的起始列 = 命中
    /// 矩形 x(渲染与命中共用同一份段表量宽),整段在场才可点;删除段与换册段同一
    /// 行——键位行钉在册体内层末行(#73 的几何,删除段继承,不另推一份)。
    #[test]
    fn d段_四本册两档都点得中_屏上那一列就是命中矩形() {
        for (name, kind, _) in four_books() {
            for (w, h) in [(80u16, 30u16), (66, 50)] {
                // 每档一本新册:点中之后册子进删除发起态(键位行整行换成浮层提示),
                // 复用同一本会把下一档的 delete_rect 找空
                let mut app = book(name);
                app.size = Rect::new(0, 0, w, h);
                let d = delete_rect(&app);
                let sw = crate::layout::hitmap(app.size, &app, crate::layout::size_class(app.size))
                    .into_iter()
                    .find(|(_, t)| *t == crate::layout::Target::SwitchBook)
                    .map(|(r, _)| r)
                    .expect("换册段应有命中区");
                assert_eq!(
                    d.y, sw.y,
                    "{name} {w}×{h} 删除段与换册段同一行(键位行钉末行)"
                );
                let f = crate::testutil::frame_display(&app, w, h);
                let row = f.lines().nth(d.y as usize).expect("命中行在渲染中在场");
                assert_eq!(
                    帧行截取(
                        row,
                        d.x,
                        unicode_width::UnicodeWidthStr::width(delete_text(kind))
                    ),
                    delete_text(kind),
                    "{name} {w}×{h} 段整段在场(不是半个命中):{row}"
                );
                点中(&mut app, w, h, |t| {
                    *t == crate::layout::Target::CatalogDelete
                });
                assert!(删除已发起(kind, &app), "{name} {w}×{h} 点得中");
            }
        }
    }

    /// 规格验收 7 / ADR-0009 §4:装配态下点删除段与按 `d` 同后果——`d` 是写动作,
    /// 与装配互斥(键盘侧本就复现这句,鼠标同口径):日志逐条相同、不弹确认、
    /// 不入队、零引擎调用。
    #[test]
    fn 装配态下点d段_与按d同后果() {
        let (mut clicked, r1) = sub_book();
        let (mut keyed, _r) = sub_book();
        clicked.assembling = true;
        keyed.assembling = true;
        clicked.size = Rect::new(0, 0, 80, 30);
        let before = r1.call_argvs();
        点中(&mut clicked, 80, 30, |t| {
            *t == crate::layout::Target::CatalogDelete
        });
        tap(&mut keyed, KeyCode::Char('d'));
        assert_eq!(clicked.log, keyed.log, "装配态下两条路逐条相同");
        assert!(
            clicked
                .log
                .iter()
                .any(|l| l.contains("装配进行中:删除与装配互斥")),
            "写互斥那句复现:{:?}",
            clicked.log.first()
        );
        assert!(!clicked.sub_rm_confirm, "装配态不弹确认");
        assert!(clicked.queue.is_empty(), "装配态零下发");
        assert_eq!(r1.call_argvs(), before, "装配态点击零引擎调用");
    }

    /// #75 裁决 A 的标签册形状:点「d 摘/删」段进的不是确认弹层,是同一条表单
    /// 流——名字预填选中的那条、光标落引用字段(留空 = 删整个标签,填 kind/条目
    /// = 只摘那几条),确认前零下发。与按 `d` 逐条相同。
    #[test]
    fn 标签册点d段_进同一表单流_预填选中标签名() {
        let mut clicked = book("标签");
        let mut keyed = book("标签");
        clicked.size = Rect::new(0, 0, 80, 30);
        keyed.size = Rect::new(0, 0, 80, 30);
        点中(&mut clicked, 80, 30, |t| {
            *t == crate::layout::Target::CatalogDelete
        });
        tap(&mut keyed, KeyCode::Char('d'));
        assert_eq!(clicked.log, keyed.log, "点 d 摘/删段与按 d 日志逐条相同");
        assert_eq!(
            clicked.tag_flow.phase,
            RegPhase::Form,
            "进表单流,不是确认弹层"
        );
        assert_eq!(
            clicked.tag_form.name, "bare",
            "预填选中的标签名(排序后首行)"
        );
        assert_eq!(clicked.tag_form.field, TagField::Refs, "光标落引用字段");
        assert!(
            !clicked.tag_rm_confirm,
            "表单阶段的确认旗未亮(Enter 提交后才亮)"
        );
        assert!(clicked.queue.is_empty(), "表单没确认前不下发任何写命令");
    }
}
