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

use crate::app::{App, DataRootCause};
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
pub trait RegisterSpec {
    /// 表单标题(呈现层用)。
    fn title() -> &'static str;

    /// 预审段 argv:跑闸、拟落账,**文件不落**(无 `--yes`)。
    fn preaudit(&self, root: &str) -> Vec<String>;

    /// 落账段 argv:只能由图形确认后的转换调用(带确认闸)。
    fn commit(&self, root: &str) -> Vec<String>;

    /// 回音拆成可摊开的预审结论;不可定型给 `None`。
    fn audit(out: &EngineOutput) -> Option<Audit>;

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
    fn confirm_gate() -> bool {
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

    // ---- 册级查表(工单 #66:按册差异只剩数据,不再有分支) ----
    //
    // 下面这一族把「这一本的登记流槽 / 删除确认旗 / 册体条数 / 选中槽 /
    // 落账闸 / 投影数据源 / 空册文案 / 附述行 / 边界表述 / 键位行 / 浮层 /
    // 册内键位表」收成 `CatalogKind` 的成员。**调用方不再写第四份
    // `if kind == …`**:命中区(`layout::catalog_hits`)、册内键位路由
    // (`App::key_catalog`)、渲染(`render_catalog` / `catalog_keys`)与
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
    pub fn keys_empty(self) -> &'static [&'static str] {
        match self {
            CatalogKind::Vps => &["  r 登记机器", "Esc/q 返航"],
            CatalogKind::Tag => &["  l 反查全部", "r 挂标签", "Esc/q 返航"],
            CatalogKind::Subscription => &["  r 登记订阅", "l 重读", "Esc/q 返航"],
            CatalogKind::Key => &["  r 登记钥匙", "l 重读", "Esc/q 返航"],
        }
    }

    /// 册体有条目时的册内键位行**段清单**(常态)。
    pub fn keys_body(self) -> &'static [&'static str] {
        match self {
            CatalogKind::Vps => &["  ↑/↓ 选机器", "r 登记", "d 删除", "Esc/q 返航"],
            CatalogKind::Tag => &["  ↑/↓ 选", "Enter 反查", "l 全部", "r 挂标签", "d 摘/删"],
            CatalogKind::Subscription => {
                &["  ↑/↓ 选订阅", "r 登记", "g 取回命令", "d 删除", "l 重读"]
            }
            CatalogKind::Key => &["  ↑/↓ 选钥匙", "r 登记", "g 取回命令", "d 删除", "l 重读"],
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
        use CatalogAction::{Act, Move};
        match (self, k) {
            // ↑/↓ 选行:四本同一口(不是按册差异,但同表驱动,四本走一条路)
            (_, KeyCode::Down) => Some(Move(true)),
            (_, KeyCode::Up) => Some(Move(false)),
            // r 登记 / 挂标签:两段式(带确认闸)与无确认闸两种形态,各本自己的方法
            (CatalogKind::Vps, KeyCode::Char('r')) if !app.assembling => {
                Some(Act(App::start_vps_register))
            }
            (CatalogKind::Tag, KeyCode::Char('r')) if !app.assembling => {
                Some(Act(App::start_tag_add))
            }
            (CatalogKind::Subscription, KeyCode::Char('r')) if !app.assembling => {
                Some(Act(App::start_sub_register))
            }
            (CatalogKind::Key, KeyCode::Char('r')) if !app.assembling => {
                Some(Act(App::start_key_register))
            }
            // d 删除:机器 / 订阅 / 钥匙三本进 rm 确认弹层;标签册的删除
            // 嵌套在表单流里(摘引用 / 删整个标签两态),是另一条路
            (CatalogKind::Vps, KeyCode::Char('d')) => Some(Act(App::start_vps_rm)),
            (CatalogKind::Tag, KeyCode::Char('d')) => Some(Act(App::start_tag_rm)),
            (CatalogKind::Subscription, KeyCode::Char('d')) => Some(Act(App::start_sub_rm)),
            (CatalogKind::Key, KeyCode::Char('d')) => Some(Act(App::start_key_rm)),
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

/// 册内浮层的两种形态(登记流 / rm 确认;弹层优先级高于册体)。
/// 渲染与命中区都只认这两个形态,不认「哪一本」。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OverlayForm {
    /// 两段式登记流(表单 / 预审在途 / 摊开闸结论 / 落账在途 / 结论)。
    Register,
    /// 删除前的图形确认(ADR-0009 §2:面板加一道,不改引擎语义)。
    RmConfirm,
}

/// 册内键位触发的动作(#66 查表表的取值)。`Act` 直接带这一本的落实方法
/// ——按册差异就是「哪个方法」,这是数据,不是调用方的一个分支。
pub enum CatalogAction {
    /// ↑/↓ 移选中(`true` = 向下;到界绕回,见 `App::catalog_move`)。
    Move(bool),
    /// 这一本对这一键的落实(登记 / 删除 / 重读 / 取回命令 / 反查)。
    Act(fn(&mut App)),
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
/// (ADR-0009 §3):它只以这一个类型存在——
/// - [`std::fmt::Debug`] **不打印内容**(只报字节数),所以它进不了日志行,
///   也进不了崩溃上报;
/// - 渲染只经 [`SecretBody::mask`] 拿「已喂入 N 字节」这一句,字符一个都不出现;
/// - 面板不把它写进 config / 任何持久化路径(也没有这样的 API);
/// - 流程结束(落账 / 返航 / 切屏 / 面板退出)随表单一起 drop,面板侧不留副本。
///
/// 它**只从 stdin 进**(`App` 经 `CommandRunner::spawn_with_stdin` 喂给子进程,
/// 写后即关、即弃):argv 面没有任何明文形参,误粘的 argv 会进 ps / history。
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
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
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
/// **册体行序是命中区的几何基准**(`layout::catalog_hits` 按 `row_lines()`
/// 逐条铺),所以「当前是哪一本」与编目路径合并在首行,不另起一行——
/// 多一行就整体错位,点哪打哪就断了。
pub fn render_catalog(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let kind = app.catalog_kind;
    let block = body(app, "机器写 · 永不手改");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let w = inner.width.saturating_sub(2) as usize;

    let mut lines = catalog_body_lines(app, w);
    let _ = cause_line(app);
    lines.push(key_line(&catalog_key_parts(app), p));
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
}

/// 标签册的边界表述(ADR-0009 §5,工单 #54 验收第 7 条):标签随组织演进,
/// 烤进锁定集等于每次改标签重烤一次派——面板不提供任何这样的入口。
const TAG_BOUNDARY: &str = "  边界:标签绝不进配方/锁定集/bundle/装配(不提供烤进去的入口)";

/// 数据仓根三态的一行说明(能力可用而根不可用时给人话;根就绪时给空)。
fn cause_line(app: &App) -> String {
    match app.data_root_cause() {
        Some(DataRootCause::Unset) => {
            "  数据仓根未设:去设置屏按 d 录入(面板不拿当前目录兜底)。".into()
        }
        Some(DataRootCause::Unavailable) => {
            "  数据仓根不可用:目录不存在或不可读,去设置屏按 d 重设。".into()
        }
        _ => String::new(),
    }
}

/// 编目册**正文行**(键位行之前的一切:标题 + 册体 + 各册附加行)。
/// #67 抽出供渲染与命中测试共用:键位行的行号 = `inner.y + 本函数行数`,
/// 一处真相——可点的「换册」段落在键位行上,渲染画的那一行与命中测的
/// 那一行由同一份行数算出,点哪打哪(ADR-0005 渲染与命中同源)。
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
        lines.push(Line::from(Span::styled(
            format!(
                "  当前册 {} · {} · 编目 {}",
                kind.label(),
                app.catalog_scope_label(),
                clip(
                    &app.catalog_sheet()
                        .map(|s| s.catalog_path)
                        .unwrap_or_else(|| "—".into()),
                    w.saturating_sub(20)
                )
            ),
            p.dim(),
        )));
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
                Some(s) => lines.extend(catalog_sheet_lines(
                    &s,
                    w,
                    app.catalog_sel(),
                    p,
                    kind.detail(),
                )),
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
pub(crate) const CATALOG_KEY_GAP: &str = " · ";

/// 键位行末段的「换册」可点文案(#67):点它 = 按 `m` 轮转四本册。渲染画这个
/// 串、命中测这个串的宽,同一份字符串;段可点(accent 高亮),与键盘 `m` 同路。
pub(crate) const CATALOG_SWITCH: &str = "m 换册";

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
pub(crate) fn catalog_key_parts(app: &App) -> Vec<(&'static str, Option<crate::layout::Target>)> {
    use crate::layout::Target;
    let kind = app.catalog_kind;
    if kind.flow(app).phase != RegPhase::Idle || kind.rm_confirm(app) {
        return vec![("  Enter 确认", None), ("Esc/q 返航", None)];
    }
    let hint: &'static [&'static str] = if kind.len(app) == 0 {
        kind.keys_empty()
    } else {
        kind.keys_body()
    };
    let mut parts: Vec<(&'static str, Option<Target>)> = hint.iter().map(|s| (*s, None)).collect();
    parts.push((CATALOG_SWITCH, Some(Target::SwitchBook)));
    parts
}

/// 把段清单画成一行 `Line`(渲染侧):可点段(`Some`)accent 高亮、提示段
/// (`None`)dim;段间用 [`CATALOG_KEY_GAP`] 连接。与命中侧
/// [`crate::layout::key_line_segments`] 同一份 `parts` + 同一个 gap——屏上画的
/// 和点得的逐字同源。
fn key_line(parts: &[(&str, Option<crate::layout::Target>)], p: &Palette) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, (text, target)) in parts.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(CATALOG_KEY_GAP, p.dim()));
        }
        let style = if target.is_some() {
            p.accent().bold()
        } else {
            p.dim()
        };
        spans.push(Span::styled((*text).to_string(), style));
    }
    Line::from(spans)
}

/// **编目册列表件**(#53 骨架第二件):按投影字段渲染,遵循确定性排序。
/// 缺字段显式「——」,不自造、不补全;ref 原样呈现(指针不是密钥),
/// 摘要不印 ref 与 addr。后三本册子只换 `CatalogSheet` 的数据源与
/// [`EntryDetail`](条目明细画法),不换列表件。
pub fn catalog_sheet_lines(
    sheet: &CatalogSheet,
    width: usize,
    sel: usize,
    p: &Palette,
    detail: EntryDetail,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for (i, e) in sheet.entries.iter().enumerate() {
        let on = i == sel;
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

/// 反查上浮结果浮层的矩形(#70):**渲染与命中区共用这一处几何**(ADR-0005:
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
                        clip(
                            &crate::engine::CatalogRef::line(&t.refs),
                            inner_w.saturating_sub(24),
                        ),
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

/// 钥匙册的条目明细:名字行后缀给 kind(条目级,恒 `github-key`),之下给
/// **主机 / 作用域条数 / 轮换日**(缺字段显式「——」)。**没有本体槽,也不印
/// 指纹**——投影里就没有这两格(上游 `KeySummary` 五键冻结),面板连「这像是
/// 哪把钥匙」都不该让人推断出来。作用域只报**条数**(只登记不设闸,面板不代管、
/// 不提醒、不评审)。
fn key_detail_rows(e: &CatalogEntryView, width: usize, p: &Palette) -> EntryDetailOut {
    let kind = e
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .unwrap_or("——");
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
                Span::styled(
                    clip(&e.rotated_at_line(), width.saturating_sub(12)),
                    p.text(),
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
            TagFormMode::Add => "  Enter 挂上去(无确认闸) · Esc/q 返航",
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
            "  Esc/q 返航(无确认闸:改好表单重下同一条 argv)",
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
    use crossterm::event::{KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
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
            // 键位行钉在册体末行(两档同文案)
            assert!(s.contains("选机器"), "{w}×{h} 键位行应在场:{s}");
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
        let lines = catalog_sheet_lines(&sheet, 60, 0, &p, vps_detail_rows);
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
            catalog_sheet_lines(&sheet, 60, 0, &p, detail)
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
            catalog_sheet_lines(&sheet, 60, 0, &p, detail)
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
    /// 钥匙」都不该让人推断出来;作用域只报**条数**、轮换日原样(只登记不设闸)。
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
            "2 条",
            "作用域只报条数(不代管、不评审)"
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
        assert!(s.contains("2 条"), "{s}");
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
            catalog_sheet_lines(&sheet, 60, 0, &p, detail)
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
        assert!(key.contains("2 条"), "作用域只报条数:{key}");
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

    // ---- 工单 #66:四本册整帧 golden(逐字节) ----
    //
    // 本票是**纯重构**:操作者感知不到任何变化。上面四条
    // `*_屏_常畅与紧凑两档都渲染不溢出` 只钉「在场」(contains),钉不住
    // 「没变」;这一族把整帧显示内容逐字节钉住,按册各一份、常畅(80×30)与
    // 紧凑(66×50)两档各一份——按册分叉收敛成查表成员之后,任何一本的
    // 文案 / 行序 / 行数 / 折列漂了一个字节,这里就红。
    //
    // 期望值取自**本票之前**的第 8 轮 tip `bb19d70`(同一夹具、同一尺寸),
    // 并经人工逐行核对。修订纪律同 `ui::tests::golden_常畅态_80x30_整帧逐字节`:
    // 若既有渲染面发生真实回归而改期望,必须与该回归同一次提交并说明——
    // 不许为过测试回填。

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
            app.data_root = Some(std::env::temp_dir().to_string_lossy().to_string());
            app.nw_home = "/nw".into(); // 顶栏 NW_HOME 也钉死(别的测试会写进程环境)
            app
        };
        let mut tag = tag_app(&[("tag ls", 0, TAG_LS_ALL)]);
        tag.data_root = Some(std::env::temp_dir().to_string_lossy().to_string());
        tag.nw_home = "/nw".into();
        tap(&mut tag, KeyCode::Char('l'));
        feed(&mut tag);
        let (mut sub, _r) = sub_app(&[("subscription ls", 0, SUB_LS_ALL)]);
        sub.data_root = Some(std::env::temp_dir().to_string_lossy().to_string());
        sub.nw_home = "/nw".into();
        tap(&mut sub, KeyCode::Char('l'));
        feed(&mut sub);
        let (mut key, _r) = key_app(&[("key ls", 0, KEY_LS_ALL)]);
        key.data_root = Some(std::env::temp_dir().to_string_lossy().to_string());
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
    /// 机器册 80×30 整帧 golden(本票之前逐字节)。
    const VPS_80X30: &str = r####" Warehousetui · Nous Warehouse 驾驶舱   nwt v0.2 · nw 探测中 · 测试注入  │  NW_H
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
│          ││  ↑/↓ 选机器 · r 登记 · d 删除 · Esc/q 返航 · m 换册              │
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
└──────────┘└──────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 t 主题 ? 帮助   q 退出       主题:暗色仓库                 "####;
    /// 机器册 66×50 整帧 golden(本票之前逐字节)。
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
│  ↑/↓ 选机器 · r 登记 · d 删除 · Esc/q 返航 · m 换册            │
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
└────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 ? 帮助   q 退出                              "####;
    /// 标签册 80×30 整帧 golden(本票之前逐字节)。
    const TAG_80X30: &str = r####" Warehousetui · Nous Warehouse 驾驶舱   nwt v0.2 · nw 探测中 · 测试注入  │  NW_H
┌ 导航 ────┐┌ 编目 ───────────────────────────────────────────机器写 · 永不手改┐
│  ▸ 概览  ││  当前册 标签 · 反查 /tmp · 编目 /r/tags.json                     │
│  ⚙ 装配  ││  ▸ bare                                                          │
│  ✚ 健康  ││    引用 ——                                                       │
│  ▤ 配方  ││    pi                                                            │
│▸ ▩ 编目  ││    引用 skill/o/r/deep-research · vps/gate                       │
│  ✦ 设置  ││  边界:标签绝不进配方/锁定集/bundle/装配(不提供烤进去的入口)      │
│          ││  ↑/↓ 选 · Enter 反查 · l 全部 · r 挂标签 · d 摘/删 · m 换册      │
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
└──────────┘└──────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 t 主题 ? 帮助   q 退出       主题:暗色仓库                 "####;
    /// 标签册 66×50 整帧 golden(本票之前逐字节)。
    const TAG_66X50: &str = r####"NW_HOME: /nw  ○ 空闲                                              
  ▸ 概览   ⚙ 装配   ✚ 健康   ▤ 配方 ▸ ▩ 编目   ✦ 设置             
┌ 编目 ─────────────────────────────────────────机器写 · 永不手改┐
│  当前册 标签 · 反查 /tmp · 编目 /r/tags.json                   │
│  ▸ bare                                                        │
│    引用 ——                                                     │
│    pi                                                          │
│    引用 skill/o/r/deep-research · vps/gate                     │
│  边界:标签绝不进配方/锁定集/bundle/装配(不提供烤进去的入口)    │
│  ↑/↓ 选 · Enter 反查 · l 全部 · r 挂标签 · d 摘/删 · m 换册    │
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
└────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 ? 帮助   q 退出                              "####;
    /// 订阅册 80×30 整帧 golden(本票之前逐字节)。
    const SUB_80X30: &str = r####" Warehousetui · Nous Warehouse 驾驶舱   nwt v0.2 · nw 探测中 · 测试注入  │  NW_H
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
│          ││  ↑/↓ 选订阅 · r 登记 · g 取回命令 · d 删除 · l 重读 · m 换册     │
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
└──────────┘└──────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 t 主题 ? 帮助   q 退出       主题:暗色仓库                 "####;
    /// 订阅册 66×50 整帧 golden(本票之前逐字节)。
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
│  ↑/↓ 选订阅 · r 登记 · g 取回命令 · d 删除 · l 重读 · m 换册   │
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
└────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 ? 帮助   q 退出                              "####;
    /// 钥匙册 80×30 整帧 golden(本票之前逐字节)。
    const KEY_80X30: &str = r####" Warehousetui · Nous Warehouse 驾驶舱   nwt v0.2 · nw 探测中 · 测试注入  │  NW_H
┌ 导航 ────┐┌ 编目 ───────────────────────────────────────────机器写 · 永不手改┐
│  ▸ 概览  ││  当前册 钥匙 · 全部钥匙 · 编目 /r/keys.json                      │
│  ⚙ 装配  ││  ▸ ci  kind github-key                                           │
│  ✚ 健康  ││    主机 ghe.internal                                             │
│  ▤ 配方  ││    作用域 未声明(0 条)                                           │
│▸ ▩ 编目  ││    轮换日 2025-11-02                                             │
│  ✦ 设置  ││    deploy  kind github-key                                       │
│          ││    主机 github.com                                               │
│          ││    作用域 2 条                                                   │
│          ││    轮换日 2026-01-15                                             │
│          ││  取回钥匙(面板不代跑,按 g 收进日志):                             │
│          ││nw key get --name ci --root /tmp | jq -r .data.key                │
│          ││  get 是上游唯一回显本体的出口:铺在屏显上会进滚动历史/截图/快照   │
│          ││  边界:钥匙绝不进配方/锁定集/bundle/装配(凭据不随派分发,不提供烤  │
│          ││  ↑/↓ 选钥匙 · r 登记 · g 取回命令 · d 删除 · l 重读 · m 换册     │
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
└──────────┘└──────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 t 主题 ? 帮助   q 退出       主题:暗色仓库                 "####;
    /// 钥匙册 66×50 整帧 golden(本票之前逐字节)。
    const KEY_66X50: &str = r####"NW_HOME: /nw  ○ 空闲                                              
  ▸ 概览   ⚙ 装配   ✚ 健康   ▤ 配方 ▸ ▩ 编目   ✦ 设置             
┌ 编目 ─────────────────────────────────────────机器写 · 永不手改┐
│  当前册 钥匙 · 全部钥匙 · 编目 /r/keys.json                    │
│  ▸ ci  kind github-key                                         │
│    主机 ghe.internal                                           │
│    作用域 未声明(0 条)                                         │
│    轮换日 2025-11-02                                           │
│    deploy  kind github-key                                     │
│    主机 github.com                                             │
│    作用域 2 条                                                 │
│    轮换日 2026-01-15                                           │
│  取回钥匙(面板不代跑,按 g 收进日志):                           │
│nw key get --name ci --root /tmp | jq -r .data.key              │
│  get 是上游唯一回显本体的出口:铺在屏显上会进滚动历史/截图/快   │
│  边界:钥匙绝不进配方/锁定集/bundle/装配(凭据不随派分发,不提供  │
│  ↑/↓ 选钥匙 · r 登记 · g 取回命令 · d 删除 · l 重读 · m 换册   │
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
└────────────────────────────────────────────────────────────────┘
 j/k 切屏 Enter 确认 ? 帮助   q 退出                              "####;

    /// 机器册整帧 golden:常畅 80×30 与紧凑 66×50 两档都逐字节不变
    /// (#66 验收第 1 条:纯重构,零用户可见变化)。
    #[test]
    fn 机器册整帧golden_两档逐字节不变() {
        let app = book("机器");
        assert_book_frame(&app, 80, 30, VPS_80X30);
        assert_book_frame(&app, 66, 50, VPS_66X50);
    }

    /// 标签册整帧 golden:常畅 80×30 与紧凑 66×50 两档都逐字节不变
    /// (#66 验收第 1 条:纯重构,零用户可见变化)。
    #[test]
    fn 标签册整帧golden_两档逐字节不变() {
        let app = book("标签");
        assert_book_frame(&app, 80, 30, TAG_80X30);
        assert_book_frame(&app, 66, 50, TAG_66X50);
    }

    /// 订阅册整帧 golden:常畅 80×30 与紧凑 66×50 两档都逐字节不变
    /// (#66 验收第 1 条:纯重构,零用户可见变化)。
    #[test]
    fn 订阅册整帧golden_两档逐字节不变() {
        let app = book("订阅");
        assert_book_frame(&app, 80, 30, SUB_80X30);
        assert_book_frame(&app, 66, 50, SUB_66X50);
    }

    /// 钥匙册整帧 golden:常畅 80×30 与紧凑 66×50 两档都逐字节不变
    /// (#66 验收第 1 条:纯重构,零用户可见变化)。
    #[test]
    fn 钥匙册整帧golden_两档逐字节不变() {
        let app = book("钥匙");
        assert_book_frame(&app, 80, 30, KEY_80X30);
        assert_book_frame(&app, 66, 50, KEY_66X50);
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
}
