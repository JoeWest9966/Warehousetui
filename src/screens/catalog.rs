//! 编目册屏(spec #50 工单 #53 / #54,ADR-0009):第一本编目册(vps)端到端
//! 打通,同时立起**编目册骨架三件**;#54 加第二本(标签册),验证骨架在
//! 「无确认闸的写路径」上也站得住。
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
//! ([`CatalogKind`]):五本编目共用同一套骨架,拆五块屏等于把同一件东西
//! 复制五份;而「当前是哪一本」必须显式在场(五本都是数据仓根下机器写的
//! JSON)。键 `m` 换册。
//!
//! 机密纪律(ADR-0009 §3):`logins[].ref` 是**指针不是密钥**,原样呈现安全;
//! 摘要行不印 ref 与 addr(照引擎 stderr 摘要的口径);本屏没有任何
//! 「本体」槽(vps 组 argv 面无密码参数,误粘 PEM 由引擎第⑤道闸拦;tag 组
//! 连指针槽都没有——`--ref` 走 argv 不走 stdin)。

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::Frame;

use crossterm::event::KeyCode;

use crate::app::{App, DataRootCause};
use crate::engine::{
    capability, capability::Capability, CatalogEntryView, CatalogGate, CatalogRef, CatalogSheet,
    EngineOutput, State, TagAddData, VpsLsData, VpsRegisterData,
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
/// vps 是第一个实现(`VpsSpec`);key / subscription 两本只换这一个实现
/// (本体经 stdin 喂入,#55 铺的管子)。**tag 是例外**(#54):上游
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
/// **为什么是屏内的一个枚举而不是第二块屏**:五本编目共用同一套骨架
/// (投影三件 / 列表件 / 登记流),拆成五块屏等于把同一件东西复制五份;
/// 而「当前是哪一本」又必须显式在场——五本都是数据仓根下机器写的 JSON,
/// 操作者得知道自己正在改哪一本(ADR-0009:机器写、永不手改)。键 `m`
/// 换册(与健康屏 `m` 轮转模式同语义)。后两本(key / subscription)
/// 随票追加,只在这里多一个变体。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CatalogKind {
    /// 机器册:`root>/vps.json`(#53)。
    #[default]
    Vps,
    /// 标签册:`<root>/tags.json`(工单 #54;本票主价值是反查)。
    Tag,
}

impl CatalogKind {
    pub const ALL: [CatalogKind; 2] = [CatalogKind::Vps, CatalogKind::Tag];

    /// 册名(操作者视角;词有出处:vps / 标签都是上游命令组名,不自造)。
    pub fn label(self) -> &'static str {
        match self {
            CatalogKind::Vps => "机器",
            CatalogKind::Tag => "标签",
        }
    }

    /// 编目文件名(数据仓根下机器写的那本 JSON)。
    pub fn file(self) -> &'static str {
        match self {
            CatalogKind::Vps => "vps.json",
            CatalogKind::Tag => "tags.json",
        }
    }

    /// 这一本的能力门(#29 组级探针:`vps` / `tag`)。
    pub fn capability(self) -> Capability {
        match self {
            CatalogKind::Vps => Capability::Vps,
            CatalogKind::Tag => Capability::Tag,
        }
    }

    /// 册体里**每一条目占几行**(渲染与命中共用,同 `layout` 的法子:
    /// 几何只有一份真相,点哪打哪)。
    pub fn row_lines(self) -> u16 {
        match self {
            CatalogKind::Vps => 4, // 名字+端口 / 地址 / 登录 / 备注
            CatalogKind::Tag => 2, // 名字 / 引用
        }
    }

    /// 这一本的条目明细画法(列表件的可换部件,见 [`catalog_sheet_lines`])。
    pub fn detail(self) -> EntryDetail {
        match self {
            CatalogKind::Vps => vps_detail_rows,
            CatalogKind::Tag => tag_detail_rows,
        }
    }

    /// 换一册(`m` 键)。
    pub fn next(self) -> Self {
        match self {
            CatalogKind::Vps => CatalogKind::Tag,
            CatalogKind::Tag => CatalogKind::Vps,
        }
    }
}

/// 一本册子的条目明细画法:名字行右侧的弱化后缀(机器册给端口,标签册
/// 没有)+ 名字行之下的明细行(机器册给 地址/登录/备注,标签册给 引用)。
///
/// 列表件 [`catalog_sheet_lines`] 只按这一组画;选中标记 / 名字 / 折行
/// 截断 / 确定性排序都在它自己那一处。后两本册子只换这一个函数指针,
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

/// 编目册屏(机器册 / 标签册两本,`m` 换册)。常畅态 / 紧凑态同一份正文
/// (只随宽度折行,不引新断点、不缩放字号):当前是哪一本 + 编目路径 +
/// 清单 + 键位行;登记与删除各是一个浮层。
///
/// **册体行序是命中区的几何基准**(`layout::vps_hits` 按 `row_lines()`
/// 逐条铺),所以「当前是哪一本」与编目路径合并在首行,不另起一行——
/// 多一行就整体错位,点哪打哪就断了。
pub fn render_catalog(frame: &mut Frame, area: Rect, app: &App) {
    let p = &app.palette;
    let kind = app.catalog_kind;
    let block = body(app, "机器写 · 永不手改");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let w = inner.width.saturating_sub(2) as usize;

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
            match app.catalog_sheet() {
                None => lines.push(Line::from(Span::styled(
                    match kind {
                        CatalogKind::Vps => {
                            "  编目册还没读到:按 r 登记,或先刷新(回概览按 r 重读五域)。"
                        }
                        CatalogKind::Tag => {
                            "  还没反查过:按 l 列全部标签(只读),按 r 挂第一个标签。"
                        }
                    },
                    p.dim(),
                ))),
                Some(s) if s.entries.is_empty() => {
                    lines.push(Line::from(Span::styled(
                        match kind {
                            CatalogKind::Vps => {
                                "  编目里还没有机器(合法空态:vps.json 不存在 = 空册)。"
                            }
                            CatalogKind::Tag => {
                                "  编目里还没有标签(合法空态:tags.json 不存在 = 空册)。"
                            }
                        },
                        p.text(),
                    )));
                    lines.push(Line::from(Span::styled(
                        match kind {
                            CatalogKind::Vps => {
                                "  按 r 登记第一台(两段式:预审摊开五道闸 → 图形确认才落账)。"
                            }
                            CatalogKind::Tag => {
                                "  按 r 挂第一个标签(追加合并、幂等、无确认闸,Enter 即下发)。"
                            }
                        },
                        p.dim(),
                    )));
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
        // 标签册的边界表述(ADR-0009 §5):编目条目绝不进配方 / 锁定集 /
        // bundle / 装配命令,面板侧不提供任何把它烤进去的入口。
        if kind == CatalogKind::Tag {
            lines.push(Line::from(Span::styled(clip(TAG_BOUNDARY, w), p.dim())));
        }
    }
    let _ = cause_line(app);
    lines.push(Line::from(Span::styled(catalog_keys(app), p.dim())));
    frame.render_widget(Paragraph::new(lines).style(p.surface()), inner);

    // 浮层:登记 / 删除确认(弹层优先级高于册体;两本各一套,骨架同一件)
    if kind == CatalogKind::Tag {
        // rm 确认嵌套在表单流里(表单 Enter 才出现),弹层优先级高于表单
        // (同 `on_key` 的受理次序:更里层的浮层先收键)。
        if app.tag_rm_confirm {
            render_tag_rm_confirm(frame, area, app);
        } else if app.tag_flow.phase != RegPhase::Idle {
            render_tag_overlay(frame, area, app);
        }
        return;
    }
    if app.vps_flow.phase != RegPhase::Idle {
        render_register_overlay(frame, area, app);
    } else if app.vps_rm_confirm {
        render_rm_confirm(frame, area, app);
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

/// 编目册键位行(与鼠标命中区同文案同序)。
fn catalog_keys(app: &App) -> &'static str {
    if app.catalog_kind == CatalogKind::Tag {
        if app.tag_flow.phase != RegPhase::Idle || app.tag_rm_confirm {
            return "  Enter 确认 · Esc/q 返航 ";
        }
        if app.tag_len() == 0 {
            return "  l 反查全部 · r 挂标签 · m 换册 · Esc/q 返航 ";
        }
        return "  ↑/↓ 选 · Enter 反查 · l 全部 · r 挂标签 · d 摘/删 · m 换册 ";
    }
    if app.vps_flow.phase != RegPhase::Idle || app.vps_rm_confirm {
        return "  Enter 确认 · Esc/q 返航 ";
    }
    if app
        .vps_ls
        .as_ref()
        .map(|d| d.machines.is_empty())
        .unwrap_or(true)
    {
        return "  r 登记机器 · Esc/q 返航 ";
    }
    "  ↑/↓ 选机器 · r 登记 · d 删除 · Esc/q 返航 "
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
}
