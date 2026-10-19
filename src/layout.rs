//! 纯「布局模型」：尺寸档判定 + 鼠标命中区清单（spec #15 / ADR-0004、ADR-0005）。
//!
//! 无 I/O、无终端依赖、不需要真实终端即可单测。渲染与鼠标命中测试
//! 共用同一份几何：命中区由本模块纯函数产出，按与 [`crate::ui`] /
//! [`crate::screens`] 相同的排布约定独立算出——两者以同一套排布常量
//! 对齐（常畅态 `80×30` 快照是回归基线，排布一改即回归）。
//!
//! 常畅态：导航 / 各屏列表行 / 发射仪式 / 帮助浮层；紧凑态外壳（横向
//! 屏 Tab）由 ticket #17 落地并与渲染同源；命中分派见 `App::on_mouse`
//! （ticket #20），滚轮由 #21，紧凑态内层栏（七步 / 五域）随 #18 跟随。

use ratatui::layout::Rect;

use crate::app::{App, Screen};
use crate::engine::State;

/// 常畅态/紧凑态互换的宽度断点:单一常量,不散值。
/// 固定 80 = 原 `MIN_W`——旧常态一字不变,变窄才换装(ADR-0004)。
/// 不写成 `= MIN_W`:下限已降到 66,断点是独立决策(换装阈值 vs 开机下限)。
pub const BREAKPOINT: u16 = 80;

/// 竖左导航轨定宽(渲染侧同值,见 `ui::render_body`)。
pub const NAV_WIDTH: u16 = 12;

/// 命中区的目标:点到某矩形后要做的一件事(类型形状钉死在 spec #15)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// 点导航 / 屏 Tab 切屏。
    NavTo(Screen),
    /// 点列表第 `n` 行选中(只选中,不触发破坏性动作)。
    SelectRow(usize),
    /// 点确认按钮(发射仪式「发射」)。
    Activate,
    /// 滚轮在装配 stderr 日志区滚动(#21 接线)。
    ScrollLog(ScrollDir),
    /// 点帮助浮层任意处关闭。
    DismissPopup,
    /// 点配方域建议命令键位行 = 切换只读选中态(#39,与键盘 `c` 同路;
    /// 面板不执行该命令,也不碰剪贴板——名字里没有 "copy" 就是因为没有复制)。
    ToggleSuggest,
    /// 点正文键位行的「换册」段 = 按 `m` 轮转四本编目册(#67:鼠标只是叠加,
    /// 与键盘 `cycle_catalog_kind` 同一路;不绑定具体键——段上文案由渲染与
    /// 命中共用,点哪打哪)。
    SwitchBook,
    /// 点装配屏正文键位行的「中止」段 = 按 `x`(#71,补 #59 欠下的鼠标命中):
    /// 在途 `nw up` 子进程**进程级真止**(与键盘 `abort_assembling` 完全同一路)。
    /// 只在真有 up 在途(`as_running`)时由 `assemble_hits` 产出——不在场时不画
    /// 也不给命中;与发射前浮层里的「Esc 返航」(`DismissPopup`)是两个语义,
    /// 不共屏、不共话术。
    Abort,
    /// 点编目屏正文键位行的「Enter 反查」段 = 机器册选中行上按 `Enter`
    /// (#70:反查上浮的鼠标触发,与键盘 `App::vps_reverse_lookup` 完全同路)。
    /// **只机器册、且册体有条目(= 选中了一行)才由 `catalog_hits` 产出这一段**
    /// ——订阅 / 钥匙册的条目不在 `TAG_REF_KINDS` 里,放一个注定只返回「没有」
    /// 的动作是噪声(spec #65 §4:触发点只有一处)。
    VpsReverse,
    /// 点正文键位行的「过滤」段 = 按 `/` 进册内过滤输入态(#69:与键盘 `/`
    /// 同一路,鼠标只是叠加;段上文案由渲染与命中共用,点哪打哪)。
    CatalogFilter,
    /// 点编目屏正文键位行的「g 取回」段 = 按 `g`(#73:补 #65 §6 延期的那处
    /// 「屏上写着、鼠标点不到」)。与键盘 `g` **同一个函数、同一条后果链**——
    /// 当前选中订阅 / 钥匙的建议命令(`nw subscription get …` / `nw key get …`)
    /// 收进日志,
    /// 屏上「按 g 收进日志」翻成「已按 g 收进日志」;不入队、不下发、不碰
    /// 剪贴板(ADR-0009 §1:get 一律不代跑,上游才是唯一回显本体的出口)。
    ///
    /// **只订阅册 / 钥匙册的非空册体变体在场**(空册没有「取回哪一条」、在途
    /// 变体整行换成浮层提示、机器册 / 标签册的条目没有本体可取)——在场条件
    /// 由 [`catalog_key_parts`](crate::screens::catalog::catalog_key_parts)
    /// 按 `CatalogKind` 查表决定,命中区不另推一份。边缘态(册已读到、数据仓根
    /// 随后被撤)段仍在场,点下去复现键盘 `g` 的既有指路日志。
    ///
    /// **唯一与按键不同步的态**:册内过滤输入态(#69)里键入归输入态(敲 `g` 是
    /// 往过滤框里写字),而鼠标点段照旧走取回——这是键位行**所有**可点段的共同
    /// 边界(输入态只管键入,见 `catalog.rs` 反查段那条同口径测试),不是本段
    /// 特有的分叉。
    CatalogGet,
    /// 点编目屏正文键位行的「d 删除」段 = 按 `d`(#78,兑 #75 裁决 A:点段 ≡
    /// 按键)。与键盘 `d` **同一个函数、同一条后果链**——选中行 → ADR-0009
    /// 图形确认闸(上游四个 `rm` 都没有 `--yes`,面板自己加一道:删掉要重新
    /// 登记,代价不对称)→ 确认才 `rm`;标签册的删除嵌套在表单流里(摘引用 /
    /// 删整个标签两态),点它进同一条流。鼠标不造第二种行为,与 #71 的
    /// `x` 中止段同口径(后果最重的段,照样可点)。
    ///
    /// **四本册的非空、非在途键位行变体都在场**(空册没有「删哪一条」、在途
    /// 变体整行换成浮层提示)——在场条件由
    /// [`catalog_key_parts`](crate::screens::catalog::catalog_key_parts) 按
    /// `CatalogKind` 查表决定(`keys_body` 四行各带这个目标,段面按册:机器 /
    /// 订阅 / 钥匙三本「d 删除」,标签册「d 摘/删」),命中区不另推一份。无选中
    /// 行(如册内过滤筛到空)时点击复现键盘 `d` 的既有指路日志,不新造灰置态;
    /// 装配在途时复现写互斥那句。
    ///
    /// **唯一与按键不同步的态**:册内过滤输入态(#69)里键入归输入态(敲 `d` 是
    /// 往过滤框里写字),而鼠标点段照旧走删除——这是键位行**所有**可点段的共同
    /// 边界(输入态只管键入,见 `CatalogGet` 那条同口径注记),不是本段特有的
    /// 分叉。
    CatalogDelete,
    /// 点命令面板(#119)的结果行 = 选中那一行(与 ↑/↓ 同义)。**只选中、不
    /// 执行**:执行留给 Enter,免得误点就开火(与 #78 「点行只选中」同口径)。
    /// 面板开着时底层一律不产命中区——浮层吞点击不穿透(README 鼠标小节
    /// 既有纪律),所以这个目标只在面板自己那份命中区里出现。
    CmdRow(usize),
    /// 点反查跳转浮层里的第 `n` 条命中 = 在浮层里选中它并回车直跳(#120)。
    /// 与键盘 `↑/↓` + `Enter` 完全同路——矩形由
    /// [`catalog::tag_jump_row_rect`](crate::screens::catalog::tag_jump_row_rect)
    /// 与渲染同源产出(渲染把第 n 条画在同一个矩形里),点哪打哪(ADR-0005)。
    /// **只标签册、且反查跳转浮层在场时**由 `catalog_hits` 产出;浮层外的
    /// 正文是「点任意处关闭」(`DismissPopup`),与 Esc 同路。
    TagJump(usize),
    /// 点概览警示带上的**订阅到期条目** = 跳到订阅册并选中那一条(#94 工单
    /// #105,用户故事 12:「从发现到处理一步到位」)。载荷是这一条在**订阅册
    /// 投影**里的下标(`CatalogSheet::entries`,与册体同一排序)。
    ///
    /// 与 [`Target::SelectRow`] 的可见行下标**是两件事**(#69 册内过滤在场时
    /// 两者要换算),故单立一个变体而不是复用:命中侧拿投影下标经
    /// `catalog_visible()` 换成可见行再落选中槽,换算不出来(这一条被过滤词
    /// 挡在可见行外)走指路日志,不新造禁用态(与册体「无选中行时点下去复现
    /// 既有指路」同口径)。
    ///
    /// **零引擎调用**:回音是既有的 `app.sub_ls`,点它只换视图(定册内维度 →
    /// 切屏 → 落选中槽),与键盘 `v` 到编目屏同族。
    AlertSubEntry(usize),
}

/// 滚动方向(类型形状钉死在 spec #15)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScrollDir {
    Up,
    Down,
}

/// 概览警示带**一段**的输入形状(与 [`KeyPart`] 同族:一段文案 + 一个可选
/// 命中目标),多带一个 [`AlertTone`]。
///
/// 为什么不复用 [`KeyPart`]:警示带历来是**一行 `Paragraph` 的一段段 `Span`**
/// (段间无空隙、不是键位行的分段),且各段各有自己的既定配色——引擎缺失红 /
/// 数据仓根与漂移琥珀 / 装配进行中强调 / 提示暗,订阅条目还要按引擎下发的
/// 到期三态上四态色。配色归渲染侧,布局侧只量宽、只发目标,`tone` 原样透传。
#[derive(Clone, Debug)]
pub struct AlertPart {
    pub text: String,
    /// 命中目标;`None` = 只画不点(提示段与各档的警示头)。
    pub target: Option<Target>,
    pub tone: AlertTone,
}

/// 警示带一段的呈现角色(渲染侧据此取色;布局侧不解释它)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertTone {
    /// 红:引擎二进制找不到(既有话术一字不变)。
    Alarm,
    /// 琥珀:数据仓根三态 / 漂移派 / 订阅到期三态的警示面。
    Warn,
    /// 强调色:装配进行中(既有话术一字不变)。
    Busy,
    /// 暗:提示尾巴与「一切平静」。
    Hint,
    /// 订阅条目名:按引擎回音的 `status` 上四态色(expiring 警 / expired 败,
    /// 只标记不治理——不改退出码、不提醒)。
    Entry(State),
}

/// 警示带**一段**的产物(与 [`KeySegment`] 同族):文案 + 目标 + 角色 + 矩形。
#[derive(Debug)]
pub struct AlertSegment {
    pub text: String,
    pub target: Option<Target>,
    pub tone: AlertTone,
    /// 段在屏上的矩形(宽 = `unicode_width(文案)`,高 1 行)。
    pub rect: Rect,
}

/// 把一串 [`AlertPart`] 从 `anchor.x` 左起**紧贴**铺成一行(段间无空隙:
/// 警示带是一行 `Paragraph` 的一段段 `Span`,不是键位行的分段)。
///
/// 段宽 = `unicode_width(文案)`,与渲染逐字同源。放不下的收口分两种,
/// 判据是**这一段点不点得中**:
/// - **可点段**(`target.is_some()`):整段收口——这一段及后续既不画也不给命中
///   (与 [`key_line_segments`] 同一条纪律,#69)。屏上画得出却点不中的段比不画
///   更糟,订阅条目宁可少点名一条,也不画一截点不到的残名。
/// - **只画不点的提示段**:按剩余宽度**截断**后收口(与改动前 `Paragraph`
///   靠终端裁尾逐字同源)。数据仓根那串带路径的指路在 66×50 下限内本来就被裁过,
///   裁掉的是尾巴不是整句——若整段收口,窄终端下操作者连收到的路径都看不见。
pub fn alert_segments(anchor: Rect, parts: &[AlertPart]) -> Vec<AlertSegment> {
    use unicode_width::UnicodeWidthStr;
    let right = anchor.x.saturating_add(anchor.width);
    let mut out = Vec::new();
    let mut x = anchor.x;
    for part in parts {
        let w = u16::try_from(UnicodeWidthStr::width(part.text.as_str())).unwrap_or(0);
        let end = x.saturating_add(w);
        if w == 0 {
            break;
        }
        if end > right {
            match part.target {
                // 可点段放不下:这一段及后续都不画不命中。
                Some(_) => break,
                // 提示段放不下:裁到剩余宽度(终端宽字符不切半,
                // 与 `recipes::clip` 同一把尺),画完即收口。
                None => {
                    let room = usize::from(right.saturating_sub(x));
                    let text = crate::screens::recipes::clip(&part.text, room);
                    if !text.is_empty() {
                        out.push(AlertSegment {
                            text,
                            target: None,
                            tone: part.tone,
                            rect: Rect::new(x, anchor.y, right.saturating_sub(x), 1),
                        });
                    }
                    break;
                }
            }
        }
        out.push(AlertSegment {
            text: part.text.clone(),
            target: part.target,
            tone: part.tone,
            rect: Rect::new(x, anchor.y, w, 1),
        });
        x = end;
    }
    out
}

/// 呈现态:宽 ≥ [`BREAKPOINT`] 为常畅态,否则紧凑态(仍不低于 66×50 下限)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SizeClass {
    Comfy,
    Compact,
}

/// 尺寸档判定：`width < BREAKPOINT` ⇒ 紧凑态，否则常畅态。
pub fn size_class(area: Rect) -> SizeClass {
    if area.width < BREAKPOINT {
        SizeClass::Compact
    } else {
        SizeClass::Comfy
    }
}

/// 紧凑态屏 Tab 的单元文本：标记 + 图标 + 标题 + 尾随分隔空格。
///
/// [`crate::ui::render_screen_tabs`]（渲染）与 [`compact_shell`]（命中）
/// 都以这一个字符串为准——单元宽 = 它的显示宽度，几何永不分歧。
pub fn tab_cell(screen: Screen, selected: bool) -> String {
    let marker = if selected { "▸" } else { " " };
    format!("{marker} {} {} ", screen.icon(), screen.title())
}

/// 命中区清单：渲染与鼠标命中测试共用的同一份几何。
///
/// `area` 是当前终端面（即壳 `ui::render` 收到的帧区域）。
/// 只给**当前屏**的列表行做命中区（点其他屏的空白处不命中）；
/// 命中区一律不越出 `area`。
///
/// 外壳几何按 `size_class` 分叉：常畅态竖左导航轨，紧凑态一行横向
/// 屏 Tab（#17 落地）；正文在紧凑态全宽（Tab 条之下）。紧凑态各屏
/// 内层栏（七步 / 五域，#18）收起后，对应 `*_hits` 在本函数内跟随——
/// 判据仍只读 `size_class`，调用方（#20）不硬编码矩形。
pub fn hitmap(area: Rect, app: &App, class: SizeClass) -> Vec<(Rect, Target)> {
    // 守卫屏同款门控：装不下主界面时面板只画「终端太小」，无可点区域
    // （与 `ui::render` 的 `fits` 判定同源；低于 66 宽不做新排布）。
    if !app.fits(area.width, area.height) {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let Some(body) = area_pad(area, 1) else {
        return hits;
    };
    // 浮层优先级对鼠标同样成立：弹层打开时只有弹层自己的命中区，
    // 底层导航/列表不产出（点弹层外不作用底层）。
    if app.help {
        help_hits(body, &mut hits);
        return hits;
    }
    // 自升级浮层(#121):与帮助浮层同级的吞点击纪律——底层一个命中区都不产。
    if app.upgrade.is_open() {
        upgrade_hits(body, &mut hits);
        return hits;
    }
    // 清单原文浮层(#101):与帮助同款「点任意处关」——弹层优先级,
    // 底层历史列表不产出命中(点浮层外不误选行)。
    if app.ah_raw.is_some() {
        hits.push((body, Target::DismissPopup));
        return hits;
    }
    if app.as_confirm {
        ritual_hits(body, class, &mut hits);
        return hits;
    }
    // 手填浮层(NW_HOME / 数据仓根 / 烘焙两槽位)对鼠标惰性:打字仍走键盘;
    // 浮层打开期间鼠标不分派任何命中区——点击只作用于浮层,不误触底层
    // (spec #15 故事 14/16;数据仓根手填沿用同一条,#93 两槽位同族)。
    if app.home_input || app.data_input || app.slot_input.is_some() {
        return hits;
    }
    // 全局命令面板(#119):叠加浮层,与帮助浮层同级的吞点击纪律——底层
    // 导航 / 列表一个命中区都不产。面板自己的命中区只有结果行(点行 = 选中)。
    if app.cmd_open {
        cmd_hits(body, app.cmd_results_len(), app.cmd_sel, &mut hits);
        return hits;
    }
    // 登记两段式(#30):输入/等回音态同手填浮层惰性;kind 选择与
    // 确认态只产浮层命中区(弹层优先级,底层不产出)。
    if app.reg_state != crate::app::RegState::Idle {
        register_hits(body, class, app, &mut hits);
        return hits;
    }
    match class {
        SizeClass::Comfy => {
            comfy_shell(body, &mut hits);
            if let Some((_nav, inner)) = body_cols(body) {
                screen_hits(inner, app, class, &mut hits);
            }
        }
        SizeClass::Compact => {
            compact_shell(body, app.screen, &mut hits);
            if let Some(inner) = body_below_tabs(body) {
                screen_hits(inner, app, class, &mut hits);
            }
        }
    }
    hits
}

/// 顶部/底部各留一行(顶栏/HUD),先于导航分栏,与壳渲染同序。
fn area_pad(area: Rect, rows: u16) -> Option<Rect> {
    if area.height <= rows * 2 {
        return None;
    }
    let body = Rect::new(area.x, area.y + rows, area.width, area.height - rows * 2);
    (body.height >= 3).then_some(body)
}

/// 常畅态外壳:顶栏 1 行 / 左导航轨 [`NAV_WIDTH`] 列 / HUD 1 行。
/// `body` 已去掉顶栏与 HUD(由 [`hitmap`] 切好);这里只铺导航命中区。
fn comfy_shell(body: Rect, hits: &mut Vec<(Rect, Target)>) {
    let nav_w = NAV_WIDTH.min(body.width);
    for (i, screen) in Screen::ALL.iter().enumerate() {
        // 导航块自带边框:首项在 body.y+2(边框占 body.y+1),放不下就截断(同渲染)。
        if i as u16 + 2 > body.height.saturating_sub(1) {
            break;
        }
        hits.push((
            Rect::new(body.x, body.y + 1 + i as u16, nav_w, 1),
            Target::NavTo(*screen),
        ));
    }
}

/// 紧凑态外壳：竖左导航折成 `body` 首行的横向屏 Tab（顺序同
/// [`Screen::ALL`]，当前屏高亮）。单元矩形按与渲染同一个
/// [`tab_cell`] 字符串量宽——渲染与命中同源，点哪打哪（ADR-0005）。
fn compact_shell(body: Rect, current: Screen, hits: &mut Vec<(Rect, Target)>) {
    use unicode_width::UnicodeWidthStr;
    let end = body.x.saturating_add(body.width);
    let mut x = body.x;
    for screen in Screen::ALL {
        if x >= end {
            break;
        }
        let cell = tab_cell(screen, screen == current);
        let w = (UnicodeWidthStr::width(cell.as_str()) as u16).min(end - x);
        if w == 0 {
            break;
        }
        hits.push((Rect::new(x, body.y, w, 1), Target::NavTo(screen)));
        x += w;
    }
}

/// 紧凑态正文区：Tab 条占 `body` 首行，其余全宽归正文
/// （省下的导航列还给内容，spec #15 用户故事 4）。
fn body_below_tabs(body: Rect) -> Option<Rect> {
    (body.height > 1).then(|| Rect::new(body.x, body.y + 1, body.width, body.height - 1))
}

/// 常畅态分出正文区：左侧导航轨 [`NAV_WIDTH`] 列，余下为屏幕内容。
fn body_cols(body: Rect) -> Option<(Rect, Rect)> {
    let nav_w = NAV_WIDTH.min(body.width);
    if body.width <= nav_w + 1 {
        return None;
    }
    Some((
        Rect::new(body.x, body.y, nav_w, body.height),
        Rect::new(body.x + nav_w, body.y, body.width - nav_w, body.height),
    ))
}

/// 当前屏正文内的命中区（列表行 / 浮层），`content` 是正文区（常畅态
/// 已去掉左导航轨、紧凑态已去掉 Tab 条），几何与各 `render_*` 同约定：
/// 先缩 `body()` 屏体边框，再叠各屏自己的分栏与部件边框。
fn screen_hits(content: Rect, app: &App, class: SizeClass, hits: &mut Vec<(Rect, Target)>) {
    // `screens::render_*` 第一步都是 `body(app, note)` 边框，先缩一圈。
    let inner = shrink_border(content);
    match app.screen {
        Screen::Overview => overview_hits(inner, app, hits),
        Screen::Assemble => assemble_hits(inner, app, class, hits),
        Screen::Health => health_hits(inner, app, hits),
        Screen::Recipes => recipes_hits(inner, app, class, hits),
        Screen::Vps => catalog_hits(inner, app, hits),
        Screen::Settings => {}
    }
}

/// 编目册屏命中区(#53 / #54 / #56 / #57):清单整块产行命中(点行 = 选中,
/// 同 ↑/↓);登记浮层与删除确认弹层只产键位行两段(渲染 `catalog::render_catalog`
/// 同一个 `popup_rect` 基准,弹层优先级高于册体)。屏虽叫 `Vps`,命中区
/// 服务屏内四本册(机器 / 标签 / 订阅 / 钥匙)。
///
/// **四本册走同一条代码路径**(工单 #66):按册差异全部收在
/// [`CatalogKind`](crate::screens::catalog::CatalogKind) 的查表成员上
/// (`flow` / `can_commit` / `rm_confirm` / `len` / `row_lines`),本函数
/// 没有第四份 `if kind == …`。加第五本册子只改查表,此处不动。
fn catalog_hits(inner: Rect, app: &App, hits: &mut Vec<(Rect, Target)>) {
    let kind = app.catalog_kind;
    // 反查上浮结果浮层(#70):**本屏弹层优先级最高的一层**——它在场时整块
    // 正文只有一个「点任意处返航」命中区,底层清单行与键位行(含「换册」
    // 「Enter 反查」两段)一段都不产:浮层吞点击、不穿透底层
    // (spec #65 故事 37 / 50)。
    //
    // 命中区取**整块正文**而不是浮层矩形:规格钉的是「`Esc` 或任意处点击
    // 关闭」(spec #65 §7),点浮层外的空白也算返航;只给浮层矩形会变成
    // 「点外面没反应」,与规格原文相反。整块正文即浮层自己的命中区,底层
    // 不另产——「只有浮层自己的命中」这条弹层纪律不变。
    if app.vps_reverse_open() {
        if inner.width > 0 && inner.height > 0 {
            hits.push((inner, Target::DismissPopup));
        }
        return;
    }
    // 反查跳转浮层(#120):标签册那一层薄浮层,弹层优先级与反查上浮同级
    // (本屏最高)。命中条目行给 `TagJump(i)`(点它 = 选中并直跳,与
    // `↑/↓` + `Enter` 同路);浮层外的正文整块是「点任意处关闭」,与 Esc 同路
    // ——规格钉的是「Esc 或点浮层外关闭」,只给浮层矩形会变成「点外面没
    // 反应」,与反查上浮那一层的口径相反。
    if app.tag_jump_open() {
        if let Some(popup) = crate::screens::catalog::tag_jump_popup_rect(inner, app) {
            for i in 0..app.tag_jump.hits().len() {
                if let Some(r) = crate::screens::catalog::tag_jump_row_rect(popup, i) {
                    hits.push((r, Target::TagJump(i)));
                }
            }
        }
        if inner.width > 0 && inner.height > 0 {
            hits.push((inner, Target::DismissPopup));
        }
        return;
    }
    // 登记浮层:只有确认态的键位行可点(其余态同手填浮层惰性)。
    // 标签册的确认态没有「Enter 落账」段(`CatalogKind::can_commit` 恒 false,
    // 上游 tag add 无确认闸),所以同一处条件对它自然不产段。
    if kind.flow(app).phase != crate::screens::catalog::RegPhase::Idle {
        if kind.flow(app).phase == crate::screens::catalog::RegPhase::Review && kind.can_commit(app)
        {
            let popup = popup_rect(
                inner,
                crate::screens::catalog::REG_W,
                crate::screens::catalog::REG_H,
            );
            if popup.width > 0 && popup.height > 0 {
                let key_y = popup.y + popup.height - 2;
                two_key_segments(popup, key_y, REG_KEY_REVIEW_COMMIT, RITUAL_KEY_RETURN, hits);
            }
        }
        return;
    }
    if kind.rm_confirm(app) {
        let popup = popup_rect(inner, REG_INPUT_W, REG_INPUT_H);
        if popup.width > 0 && popup.height > 0 {
            let key_y = popup.y + popup.height - 2;
            two_key_segments(popup, key_y, REG_KEY_RM_CONFIRM, RITUAL_KEY_RETURN, hits);
        }
        return;
    }
    // 取回本体的确认浮层(#97):与删除确认同一族两段命中——「Enter 确认揭示」
    // 给 `Activate`(点 = 确认,经 `App::confirm_get_reveal` 入队走唯一入口),
    // 「Esc/q 返航」给 `DismissPopup`(点 = 收回,一个字节都没下发)。
    // 在途态只有返航那一段(已下发的那一次收不回),所以这里按 `is_open` 判。
    if app.get_reveal.is_open() {
        let popup = popup_rect(
            inner,
            crate::screens::catalog::GET_REVEAL_W,
            crate::screens::catalog::GET_REVEAL_H,
        );
        if popup.width > 0 && popup.height > 0 {
            let key_y = popup.y + popup.height - 2;
            two_key_segments(
                popup,
                key_y,
                crate::screens::catalog::GET_REVEAL_KEY_CONFIRM,
                RITUAL_KEY_RETURN,
                hits,
            );
        }
        return;
    }
    // 册体:清单行命中(渲染每条目占 `row_lines` 行,命中按条给整块)
    // #69:只给「看得见的那一行」产命中——行数取可见行数(经过滤后的),
    // 不过滤时 = 册体条数,与过去逐字等价;点第 i 个可见行 = 选中第 i 个
    // 可见行(`on_mouse` 的 SelectRow 经 catalog_visible 映射回原始投影)。
    let n = app.catalog_visible().len();
    push_sheet_rows(inner, n, kind.row_lines(), hits);

    // 正文键位行的可点动作段(#67 接缝,#70/#69/#73 各加一段):段清单由
    // `catalog::catalog_key_parts` 一处产出——渲染画的与命中测的逐字同源
    // (ADR-0005)。
    //
    // **键位行钉在册体内层末行**(#73 补 #67 欠的那一半:几何真相只能有一处)。
    // 末行是布局事实,不数正文行数:渲染侧把册体内容截到末行之前、欠量留空,
    // 键位行恒在 `inner.y + inner.height - 1`——册体 1 条与 20 条,可点段的 y
    // 同一个数(规格故事 7 / 验收「键位行钉末行」)。早先这里按
    // `catalog_body_lines` 的行数推 y,行数一多键位行就被顶出可见区、命中区
    // 整段消失,而渲染侧那一行也跟着漂——同一处几何两套算法。今天两边都只认
    // 末行,不再有「第几行」的第二种推法。
    //
    // 行高必须真的有一行(内层被压成 0 行时什么都不产);段的列几何与裁剪仍由
    // `key_line_segments` 按 `inner` 右界收口(放不下的整段不画也不给命中)。
    let key_y = catalog_key_row(inner);
    if inner.height > 0 {
        let parts = crate::screens::catalog::catalog_key_parts(app);
        for seg in key_line_segments(
            inner,
            key_y,
            &parts,
            crate::screens::catalog::CATALOG_KEY_GAP,
        ) {
            // 只画不点的提示段(`None`)不进命中区;可点动作段才给矩形。
            if let Some(t) = seg.target {
                hits.push((seg.rect, t));
            }
        }
    }
}

/// 编目册屏**键位行**那一行的 y(#73:几何真相只能有一处)。
///
/// 键位行钉在册体内层**末行**,这一个数同时服务三处,谁都不自己算:
/// 命中区取它铺段矩形([`catalog_hits`])、册体条目行命中不越它
/// ([`push_sheet_rows`])、渲染把册体内容垫空 / 截到它之前
/// (`catalog::render_catalog`)。册体内容长短因此影响不到键位行的位置。
///
/// 内层被压成 0 行时它仍回一个数(回绕到 `inner.y - 1`),调用方各自看
/// `inner.height > 0`:没有行就没有键位行,也不产命中。
pub(crate) fn catalog_key_row(inner: Rect) -> u16 {
    inner.y + inner.height.saturating_sub(1)
}

/// 册体清单的行命中:首行是「当前是哪一本 + 编目路径」,末行是键位行,
/// 中间每条目占 `per` 行(`CatalogKind::row_lines`,渲染与命中共用)。
fn push_sheet_rows(inner: Rect, n: usize, per: u16, hits: &mut Vec<(Rect, Target)>) {
    let key_y = catalog_key_row(inner);
    let rows = key_y.saturating_sub(inner.y + 1);
    if rows == 0 || per == 0 {
        return;
    }
    let mut y = inner.y + 1;
    for i in 0..n as u16 {
        if y + per > key_y {
            break;
        }
        hits.push((
            Rect::new(inner.x, y, inner.width, per),
            Target::SelectRow(i as usize),
        ));
        y += per;
    }
}

/// 与 `render_*` 同款的二分分栏:直接用同一个 ratatui `Layout`,几何零分歧
/// (cassowary 的取整由渲染与命中共用,点哪打哪)。
fn h_split2(area: Rect, a: u16, b: u16) -> (Rect, Rect) {
    let cols = ratatui::layout::Layout::horizontal([
        ratatui::layout::Constraint::Percentage(a),
        ratatui::layout::Constraint::Percentage(b),
    ])
    .split(area);
    (cols[0], cols[1])
}

/// 概览:警示带 1 行 + 主区(68/32)+ 计数卡 4 行;派表在左栏内、带边框,
/// 表头 1 行后即数据行(与 `render_profiles` 同序,同一个 ratatui Layout)。
fn overview_hits(area: Rect, app: &App, hits: &mut Vec<(Rect, Target)>) {
    let rows = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(1),
        ratatui::layout::Constraint::Min(5),
        ratatui::layout::Constraint::Length(4),
    ])
    .split(area);
    let (cols, cmd) = h_split2(rows[1], 68, 32);
    let tbl = shrink_border(cols);
    if let Some(ls) = app.ls.as_ref() {
        row_hits(tbl, 1, ls.profiles.len(), hits, Target::SelectRow);
    }
    // #68 指令栏第五条「浏览编目」可点:命中区 = NavTo(Vps),点它 = 按 `v`。
    // 几何与 `render_overview`/`render_commands` 同一组 Layout(右栏 `Length(7)`
    // 指令框)+ 同一个边框内缩,第 5 条落在内层第 5 行——不另写一份「键位行长
    // 什么样」(ADR-0005 渲染与命中共用一份几何)。指令栏是动线不是能力表,
    // 故常驻可点、与 ls 有无数据无关;放不下(紧凑到裁掉第 5 行)就不产命中。
    let right = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(7),
        ratatui::layout::Constraint::Min(3),
    ])
    .split(cmd);
    let cmd_inner = shrink_border(right[0]);
    if cmd_inner.height >= 5 {
        let line = Rect::new(cmd_inner.x, cmd_inner.y + 4, cmd_inner.width, 1);
        hits.push((line, Target::NavTo(Screen::Vps)));
    }
    // #94 工单 #105:警示带上的订阅到期条目可点(点它 = 跳到订阅册并选中那
    // 一条)。几何与 `overview::render_alert` **同一份段清单**
    // (`screens::overview::alert_parts` → [`alert_segments`]):屏上画出的段才
    // 给命中,放不下的整段两厢都不出现——一处几何真相(ADR-0005)。
    // 只画不点的段(警示头 / 提示尾巴 / 分隔号)不进命中区。
    for seg in alert_segments(rows[0], &crate::screens::alert_parts(app)) {
        if let Some(t) = seg.target {
            hits.push((seg.rect, t));
        }
    }
}

/// 装配屏:装配运行中给 stderr 日志区产滚轮命中区(#21,几何与
/// `render_running` 同款分栏);闲置时选锁定集列表在 55% 左栏、
/// 带边框、首行即数据;结果态/无日志区,不产目标。
fn assemble_hits(area: Rect, app: &App, class: SizeClass, hits: &mut Vec<(Rect, Target)>) {
    if app.assembling {
        // 与 render_running_comfy/compact 同一个 Layout:七步栏/进度行 + 日志块
        // + 末行键位行(可点「中止」段),再缩一圈块边框(与 row_hits 取内层同约定)。
        // 渲染与命中共用这一组 Layout 常量——几何零分歧(ADR-0005)。
        let (log, key_row) = match class {
            SizeClass::Comfy => {
                let rows = ratatui::layout::Layout::vertical([
                    ratatui::layout::Constraint::Min(3),
                    ratatui::layout::Constraint::Length(1),
                ])
                .split(area);
                let log = ratatui::layout::Layout::horizontal([
                    ratatui::layout::Constraint::Length(14),
                    ratatui::layout::Constraint::Min(20),
                ])
                .split(rows[0])[1];
                (log, rows[1])
            }
            SizeClass::Compact => {
                let rows = ratatui::layout::Layout::vertical([
                    ratatui::layout::Constraint::Length(1),
                    ratatui::layout::Constraint::Min(3),
                    ratatui::layout::Constraint::Length(1),
                ])
                .split(area);
                (rows[1], rows[2])
            }
        };
        let log = shrink_border(log);
        if log.width > 0 && log.height > 0 {
            // 方向由滚轮事件在分派时携带;命中区是纯几何,此处的值
            // 仅为满足 Target 形状(spec #15),不参与语义。
            hits.push((log, Target::ScrollLog(ScrollDir::Down)));
        }
        // 正文键位行的可点「中止」段(#71):段清单由 `assemble::assemble_key_parts`
        // 一处产出——渲染画的与命中测的逐字同源(ADR-0005)。只在真有 up 在途
        // (as_running)时 parts 非空;不在场时不画也不给命中。放不下(越出 key_row
        // 右界)由 key_line_segments 收口——画不出的段不给点(#67 同款纪律)。
        let parts = crate::screens::assemble::assemble_key_parts(app);
        let gap = crate::screens::assemble::ASSEMBLE_KEY_GAP;
        for seg in key_line_segments(key_row, key_row.y, &parts, gap) {
            // 只画不点的提示段(None)不进命中区;可点动作段才给矩形。
            if let Some(t) = seg.target {
                hits.push((seg.rect, t));
            }
        }
        return;
    }
    if app.as_result.is_some() {
        return;
    }
    let (cols, _) = h_split2(area, 55, 45);
    let lst = shrink_border(cols);
    // 装配历史次级视图(#101):点行选中的是历史列表。几何与渲染侧
    // `assemble::render_history` 同一个 55/45 分栏 + 缩边框——渲染与
    // 命中共用一份(ADR-0005)。行数是**可见行**(过滤后),索引即
    // `on_mouse` 的 SelectRow 入参(可见下标,与册内过滤同族)。
    if let Some(view) = app.as_history.as_ref() {
        // 安装根不在时视图整块换成指路,不产命中(与渲染侧同一口径)。
        if !view.root_missing {
            row_hits(lst, 0, app.ah_visible().len(), hits, Target::SelectRow);
        }
        return;
    }
    let Some(ls) = app.ls.as_ref() else {
        return;
    };
    row_hits(lst, 0, ls.locks.len(), hits, Target::SelectRow);
}

/// recheck 屏四块几何(#40):渲染侧 `health::render_recheck` 与命中侧
/// [`health_hits`] 共用同一组常量,几何零分歧(不靠人工对齐)。
///
/// `recheck_panels` 返回 `[结论横幅, 已登记外部源清单, 漂移条目, 证据]`:
/// 前两块在 55% 左栏(横幅 3 行文案 + 2 行块边框 = `Length(5)`),后两块在
/// 45% 右栏(漂移条目 `Length(12)`),同栏的下一块都取 `Min(4)` 的余量。
pub(crate) fn recheck_panels(area: Rect) -> [Rect; 4] {
    let (left, right) = h_split2(area, 55, 45);
    let l = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(5),
        ratatui::layout::Constraint::Min(4),
    ])
    .split(left);
    let r = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(12),
        ratatui::layout::Constraint::Min(4),
    ])
    .split(right);
    [l[0], l[1], r[0], r[1]]
}

/// recheck 屏左右两栏(三道门的指路铺满整栏):由 [`recheck_panels`] 的同一组
/// 常量合回来,不再另写一份 55/45 + 纵向分栏。
pub(crate) fn recheck_cols(area: Rect) -> (Rect, Rect) {
    let p = recheck_panels(area);
    (
        Rect::new(p[0].x, p[0].y, p[0].width, p[0].height + p[1].height),
        Rect::new(p[2].x, p[2].y, p[2].width, p[2].height + p[3].height),
    )
}

/// 引用完整性(#116)三块几何:与 [`recheck_panels`] 同一组 55/45 分栏常量,
/// 左栏 = 结论横幅 + 四态检查行清单,右栏 = 面板自读事实(规模与选中行明细)。
/// 渲染(`screens::health::render_integrity`)与命中共用这一处,几何零分歧
/// (ADR-0005)。
pub(crate) fn integrity_panels(area: Rect) -> [Rect; 3] {
    let (left, right) = h_split2(area, 55, 45);
    let l = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(5),
        ratatui::layout::Constraint::Min(4),
    ])
    .split(left);
    [l[0], l[1], right]
}

/// 引用完整性的左右两栏(门一只铺指路时用):由 [`integrity_panels`] 的同一组
/// 常量合回来,不再另写一份 55/45。
pub(crate) fn integrity_cols(area: Rect) -> (Rect, Rect) {
    let p = integrity_panels(area);
    (
        Rect::new(p[0].x, p[0].y, p[0].width, p[0].height + p[1].height),
        p[2],
    )
}

/// 健康屏 validate 档的正文分区(#98):顶部两行「校验范围」条(三档 + 两侧
/// 现状)+ 其余。渲染侧 `screens::health::render_health` 与命中侧
/// [`health_hits`] 共用这一处,几何零分歧(ADR-0005)——条子多高,27 项清单
/// 就下移多高,鼠标点行才不会错位。
pub(crate) fn health_validate_rows(area: Rect) -> (Rect, Rect) {
    let rows = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(2),
        ratatui::layout::Constraint::Min(1),
    ])
    .split(area);
    (rows[0], rows[1])
}

/// 健康屏:validate 模式下「27 项」列表在 55% 左栏、带边框、首行即数据
/// (doctor 模式是只读七面卡,无选中目标);外部源漂移模式(#40)的左栏是
/// 「结论横幅 + 已登记外部源清单」,清单行在横幅之下,几何见
/// [`recheck_panels`];引用完整性模式(#116)的左栏是「结论横幅 + 四态检查行」,
/// 几何见 [`integrity_panels`]。
fn health_hits(area: Rect, app: &App, hits: &mut Vec<(Rect, Target)>) {
    if matches!(app.hl_mode, crate::app::HealthMode::Recheck) {
        // 三道门任一不过:整栏是指路,没有可选中行
        if app.data_root_cause().is_some()
            || !app
                .capabilities
                .enabled(crate::engine::Capability::SourceRecheck)
            || app.recheck_empty_cause().is_some()
        {
            return;
        }
        // 与 render_recheck 同一个 recheck_panels,不抄第二份分栏常量
        row_hits(
            shrink_border(recheck_panels(area)[1]),
            0,
            app.recheck_source_len(),
            hits,
            Target::SelectRow,
        );
        return;
    }
    // 引用完整性(#116):检查行清单与渲染同一个 integrity_panels;数据仓根
    // 不就绪时整栏是指路,没有可选中行(与 recheck 门一同形)。
    if matches!(app.hl_mode, crate::app::HealthMode::Integrity) {
        if app.data_root_cause().is_some() {
            return;
        }
        row_hits(
            shrink_border(integrity_panels(area)[1]),
            0,
            app.hl_integrity_len(),
            hits,
            Target::SelectRow,
        );
        return;
    }
    if !matches!(app.hl_mode, crate::app::HealthMode::Validate) {
        return;
    }
    // 校验范围条(#98)占掉顶部两行:清单几何随之下移,与渲染同一个
    // `health_validate_rows`
    let (_, rest) = health_validate_rows(area);
    let (cols, _) = h_split2(rest, 55, 45);
    let lst = shrink_border(cols);
    let n = app
        .hl_result
        .as_ref()
        .map(|r| r.evidence().len())
        .unwrap_or(0);
    row_hits(lst, 0, n, hits, Target::SelectRow);
}

/// 配方屏:列表列几何与 `render_recipes` 同档——常畅态在 14 列五域轨右
/// (45% 栏),紧凑态在 Tab 行之下(52% 栏)。两支都用与渲染同一套
/// ratatui `Layout` 常量,几何零分歧(#18 收起内层栏后命中自动跟随)。
fn recipes_hits(area: Rect, app: &App, class: SizeClass, hits: &mut Vec<(Rect, Target)>) {
    // 配方编辑器浮层(#113):与落点溯源浮层同一条弹层纪律 —— 在场时整块正文
    // 只有浮层自己的命中区,底层列表行与键位行一段都不产。表单/预跑段鼠标惰性
    // (打字归键盘);确认段(预跑通过时)键位行两段(Enter 写回 / Esc 返航),
    // 预跑未过只产返航;结果段点任意处 = 返航。
    if app.recipe_edit_open() {
        recipe_edit_hits(area, app, class, hits);
        return;
    }
    // 落点溯源浮层(#115):与反查浮层同一条弹层纪律 —— 在场时整块正文只有
    // 一个「点任意处返航」命中区,底层列表行与键位行(含「Enter 落点」之外
    // 的一切)一段都不产:浮层吞点击、不穿透底层。
    //
    // 命中区取**整块正文**而不是浮层矩形:规格钉的是「Esc 或点任意处关闭」,
    // 点浮层外的空白也算返航;只给浮层矩形会变成「点外面没反应」,与规格
    // 原文相反(#70 反查浮层同款,渲染与命中共用 `landing_open()` 判据)。
    if app.landing_open() {
        if area.width > 0 && area.height > 0 {
            hits.push((area, Target::DismissPopup));
        }
        return;
    }
    let list = match class {
        SizeClass::Comfy => ratatui::layout::Layout::horizontal([
            ratatui::layout::Constraint::Length(14),
            ratatui::layout::Constraint::Percentage(45),
            ratatui::layout::Constraint::Percentage(41),
        ])
        .split(area)[1],
        SizeClass::Compact => {
            let rows = ratatui::layout::Layout::vertical([
                ratatui::layout::Constraint::Length(1),
                ratatui::layout::Constraint::Min(4),
            ])
            .split(area);
            ratatui::layout::Layout::horizontal([
                ratatui::layout::Constraint::Percentage(52),
                ratatui::layout::Constraint::Percentage(48),
            ])
            .split(rows[1])[0]
        }
    };
    row_hits(
        shrink_border(list),
        0,
        app.domain_len(),
        hits,
        Target::SelectRow,
    );
    // 配方域建议命令键位行(#39):与渲染同一套分栏常量算出详情栏,取其内层
    // 末行;三前提不齐时命中侧不放行(与登记落账闸同一种纪律)。
    if app.rc_domain == crate::app::Domain::Recipes
        && app.data_root_cause().is_none()
        && app.bake_suggest_gate().is_ok()
    {
        let detail = match class {
            SizeClass::Comfy => ratatui::layout::Layout::horizontal([
                ratatui::layout::Constraint::Length(14),
                ratatui::layout::Constraint::Percentage(45),
                ratatui::layout::Constraint::Percentage(41),
            ])
            .split(area)[2],
            SizeClass::Compact => {
                let rows = ratatui::layout::Layout::vertical([
                    ratatui::layout::Constraint::Length(1),
                    ratatui::layout::Constraint::Min(4),
                ])
                .split(area);
                ratatui::layout::Layout::horizontal([
                    ratatui::layout::Constraint::Percentage(52),
                    ratatui::layout::Constraint::Percentage(48),
                ])
                .split(rows[1])[1]
            }
        };
        let d = shrink_border(detail);
        if d.height > 0 && d.width > 0 {
            hits.push((
                Rect::new(d.x, d.y + d.height - 1, d.width, 1),
                Target::ToggleSuggest,
            ));
        }
    }
}

/// 配方编辑器浮层(#113)的命中区:几何与 `recipes::render_recipe_edit_overlay`
/// 同一组常量、同一个 `recipe_edit_popup_rect`(渲染与命中共用一份,ADR-0005)。
/// 表单 / 预跑段鼠标惰性(键入归键盘,同手填浮层先例);确认段预跑通过时键位行
/// 两段(Enter 写回 / Esc 返航),预跑未过整块归返航(点哪都是放弃,不误写回);
/// 结果段点任意处 = 返航。浮层外无任何命中区。
fn recipe_edit_hits(area: Rect, app: &App, class: SizeClass, hits: &mut Vec<(Rect, Target)>) {
    let Some(popup) = crate::screens::recipes::recipe_edit_popup_rect(area, app, class) else {
        return;
    };
    let flow = match app.rc_edit_flow.as_ref() {
        Some(f) => f,
        None => return,
    };
    use crate::screens::recipes::RecipeEditPhase;
    match flow.phase {
        // 表单 / 预跑:打字归键盘,鼠标不分派(同 NW_HOME / 数据仓根手填浮层)。
        RecipeEditPhase::Form | RecipeEditPhase::PreRun => {}
        // 确认闸:预跑通过才产「Enter 写回」段;未过整块归返航(不可写回)。
        RecipeEditPhase::Review => {
            if !flow.validate_pass {
                if popup.width > 0 && popup.height > 0 {
                    hits.push((popup, Target::DismissPopup));
                }
                return;
            }
            if popup.height < 4 {
                return; // 收窄后键位行不再可信,不产命中
            }
            let key_y = popup.y + popup.height - 2;
            two_key_segments(
                popup,
                key_y,
                crate::screens::recipes::RC_EDIT_KEY_COMMIT,
                RITUAL_KEY_RETURN,
                hits,
            );
        }
        // 结果屏:点任意处 = 返航(同登记结果屏先例)。
        RecipeEditPhase::Done => {
            if popup.width > 0 && popup.height > 0 {
                hits.push((popup, Target::DismissPopup));
            }
        }
    }
}

/// 发射仪式键位行的三段文案:渲染与命中共用(与 [`tab_cell`] 同法,
/// 点哪打哪)——`assemble::render_launch_ritual` 用同一组常量铺行,
/// [`ritual_hits`] 按同一组量宽分半。
pub const RITUAL_KEY_LAUNCH: &str = "  Enter 发射 ";
pub const RITUAL_KEY_GAP: &str = "   ";
pub const RITUAL_KEY_RETURN: &str = "Esc 返航 ";

/// 登记 kind 选择屏键位行的左段(与 RITUAL_KEY_* 同法,渲染与命中
/// 共用):右段直接复用 [`RITUAL_KEY_RETURN`],段间仍用 [`RITUAL_KEY_GAP`]。
pub const REG_KEY_KIND_LAUNCH: &str = "  Enter 预审 ";

/// 登记确认屏键位行的左段(#31 落账段,与 [`REG_KEY_KIND_LAUNCH`] 同法):
/// 只在落账闸放行时铺——不可落账时整屏仍归返航(命中侧同条件分叉)。
pub const REG_KEY_REVIEW_COMMIT: &str = "  Enter 落账 ";

/// 预扫选单键位行的左段(#114「Enter 继续」):渲染
/// (`recipes::render_reg_pick`)与命中(`register_hits`)同一份文案。
pub const REG_KEY_PICK_CONTINUE: &str = "  Enter 继续 ";

/// 编目册删除确认的键位行左段(#53,ADR-0009 §2 的图形确认):
/// 渲染(`catalog::render_rm_confirm`)与命中(`catalog_hits`)同一份文案。
pub const REG_KEY_RM_CONFIRM: &str = "  Enter 确认删除 ";

/// 登记四个浮层的尺寸(spec #25 / 工单 #30 #31):渲染与命中同一组常量,
/// 几何零分歧;`popup_rect` 的收窄规则两边同用。
pub const REG_INPUT_W: u16 = 64;
pub const REG_INPUT_H: u16 = 7;
/// 预扫选单浮层(#114):说明两行 + 分组勾选清单 + 键位行。
pub const REG_PICK_W: u16 = 64;
pub const REG_PICK_H: u16 = 20;
pub const REG_KIND_W: u16 = 56;
pub const REG_KIND_H: u16 = 9;
pub const REG_REVIEW_W: u16 = 76;
pub const REG_REVIEW_H: u16 = 24;
pub const REG_DONE_W: u16 = 76;
pub const REG_DONE_H: u16 = 20;

/// 发射仪式浮层:与 `render_launch_ritual` 同尺寸、在**正文区**内居中;
/// 底部键位行按渲染同一文案对半分——「Enter 发射」段给
/// [`Target::Activate`](点=入队 up),「Esc 返航」段给
/// [`Target::DismissPopup`](点=取消并留日志);段与段之间
/// 不重叠、不留歧义区。浮层外无任何命中区(浮层优先级对鼠标同样成立)。
///
/// 居中基准必须与渲染同源:`render_assemble` 的浮层画在 `render_body`
/// 分出的**正文列**里(常畅已去左导航轨、紧凑已去 Tab 行)。早先直接用
/// body 居中,常畅态恒差一个导航列宽(80 宽差 6 列——点渲染出来的「射」
/// 会落进返航段),紧凑态奇数高度还会差一行;两处偏差由列级测试钉住。
fn ritual_hits(body: Rect, class: SizeClass, hits: &mut Vec<(Rect, Target)>) {
    // 与 `render_body` 同款分出正文区,再在其内居中(布局模型只读 size_class)。
    let Some(content) = popup_content(body, class) else {
        return;
    };
    let popup = popup_rect(content, 60, 14);
    // 与渲染同一套行序:block 内 10 行(空/四字段/空/两行说明/空/键位行),
    // 键位行是第 9 行 ⇒ 绝对 y = popup.y + 1(边框) + 9;行在场需内高 ≥ 10。
    if popup.height < 12 {
        return;
    }
    let y = popup.y + 10;
    two_key_segments(popup, y, RITUAL_KEY_LAUNCH, RITUAL_KEY_RETURN, hits);
}

/// 正文区(常畅去左导航轨 / 紧凑去 Tab 条):浮层命中的居中基准,
/// 与 `render_body` 分给各屏的区域几何零分歧。
fn popup_content(body: Rect, class: SizeClass) -> Option<Rect> {
    match class {
        SizeClass::Comfy => body_cols(body).map(|(_, inner)| inner),
        SizeClass::Compact => body_below_tabs(body),
    }
}

/// 键位行两段命中:左段(确认动作)→ [`RITUAL_KEY_GAP`] → 右段(返航);
/// 渲染按同一组文案同一顺序铺行,段间不重叠、不留歧义区。
/// 发射仪式与登记 kind 选择屏共用此分法(点哪打哪)。
fn two_key_segments(popup: Rect, y: u16, left: &str, right: &str, hits: &mut Vec<(Rect, Target)>) {
    use unicode_width::UnicodeWidthStr;
    let inner_x = popup.x.saturating_add(1);
    let left_w = u16::try_from(UnicodeWidthStr::width(left)).unwrap_or(0);
    let gap_w = u16::try_from(UnicodeWidthStr::width(RITUAL_KEY_GAP)).unwrap_or(0);
    let right_w = u16::try_from(UnicodeWidthStr::width(right)).unwrap_or(0);
    let left_end = (inner_x.saturating_add(left_w)).min(popup.x + popup.width);
    let left_w = left_end.saturating_sub(inner_x);
    if left_w > 0 {
        hits.push((Rect::new(inner_x, y, left_w, 1), Target::Activate));
    }
    let right_x = left_end.saturating_add(gap_w);
    let right_end = (right_x.saturating_add(right_w)).min(popup.x + popup.width);
    if right_x < right_end {
        hits.push((
            Rect::new(right_x, y, right_end - right_x, 1),
            Target::DismissPopup,
        ));
    }
}

/// 键位行**一段**的输入形状(#67 接缝的通用件,#70 加第三项):
/// 段文案 + 命中目标 + 「这一段此刻是活的吗」。`target` 为 `None` 表示
/// 「只画不点」的提示段(如动作说明),`Some` 表示可点动作段(点它 = 按对应
/// 键);`live` 为 `false` 表示**这一段在场但当前不可用**(入口禁用态,#70
/// 故事 39:禁用并指路)——仍然画、仍然给命中(点了照旧走同一条路、照旧
/// 收口给人话),只是画成 dim,让操作者在点之前就看出「这台引擎干不了」。
/// 提示段与可用动作段都传 `true`。
pub type KeyPart<'a> = (&'a str, Option<Target>, bool);

/// 正文键位行的**一段**(#67 通用接缝的第一件):一段文案 + 命中目标。
/// `target` 为 `None` 表示「只画不点」的提示段(如动作说明),`Some` 表示可点
/// 动作段(点它 = 按对应键)。`live` 同 [`KeyPart`] 的第三项(渲染据此选
/// 禁用态)。渲染与命中共用同一份 `KeySegment` 清单——段宽
/// 由 [`key_line_segments`] 按 `unicode_width` 量出,屏上画的和点得的逐字同源
/// (ADR-0005:一处几何真相,不出现第二份「键位行长什么样」)。
#[derive(Debug)]
pub struct KeySegment {
    /// 段文案(渲染照此画,命中也照此量宽——同一份字符串)。
    pub text: String,
    /// 命中目标;`None` = 只画不点。
    pub target: Option<Target>,
    /// 这一段此刻是活的吗;`false` = 在场但禁用(画 dim,仍给命中,点下去
    /// 由动作自己收口给人话——#70 故事 39「入口禁用并指路」)。
    pub live: bool,
    /// 段在屏上的矩形(宽 = `unicode_width(text)`,高 1 行,贴 anchor 右界裁)。
    pub rect: Rect,
}

/// 把一串 [`KeyPart`] 按 `gap` 从 `anchor.x` 左起铺成一行键位段
/// (#67):generalise [`two_key_segments`](弹层两段)与 [`tab_cell`](紧凑屏 Tab)
/// 的先例——段宽 = `unicode_width(文案)`(与渲染逐字同源),段间不重叠、不留
/// 歧义区。**这是纯布局模型里的通用件:不绑定任何屏、不绑定任何键**,调用方
/// 给什么 `(text, target, live)` 就铺什么;渲染按同一份 `parts` 画 `Span`,
/// 命中按同一份取 `Some(target)` 段——一处真相。
///
/// 放不下就整段收口:任一段完整越出 `anchor` 右界(或空段)即到此为止,这一段
/// 及后续**既不画也不给命中**(#67:66×50 下限内放不下就不画这段、也不给命中——
/// 画出去点不中的段比不画更糟)。
pub fn key_line_segments(
    anchor: Rect,
    y: u16,
    parts: &[KeyPart<'_>],
    gap: &str,
) -> Vec<KeySegment> {
    use unicode_width::UnicodeWidthStr;
    let right = anchor.x.saturating_add(anchor.width);
    let gap_w = u16::try_from(UnicodeWidthStr::width(gap)).unwrap_or(0);
    let mut out = Vec::new();
    let mut x = anchor.x;
    for (i, (text, target, live)) in parts.iter().enumerate() {
        if i > 0 {
            x = x.saturating_add(gap_w);
        }
        let w = u16::try_from(UnicodeWidthStr::width(*text)).unwrap_or(0);
        let end = x.saturating_add(w);
        // 整段放不下(越出 anchor 右界 / 空段):这一段及后续都不画不命中。
        if w == 0 || x >= right || end > right {
            break;
        }
        out.push(KeySegment {
            text: (*text).to_string(),
            target: *target,
            live: *live,
            rect: Rect::new(x, y, w, 1),
        });
        x = end;
    }
    out
}

/// 登记浮层(#30 #31 #114):几何与 `recipes::render_register_overlay` 同一组
/// 常量、同一 `popup_rect` 居中基准。输入/等回音态鼠标惰性(同手填
/// 浮层先例);kind 选项两行产 [`Target::SelectRow`] 选中 + 键位行两段
/// (Activate 预审 / Dismiss 返航);预扫选单(#114)条目行产 SelectRow
/// (app 侧按 reg_state 分派成勾选/取消)+ 键位行两段(Activate 继续 /
/// Dismiss 返航);确认态按落账闸分叉——放行时
/// 键位行两段(Activate 落账 / Dismiss 返航),不放行时整屏点任意处
/// = 返航(不可落账不可达);结果屏整屏点任意处 = 返航。浮层外不产出
/// 任何命中区(弹层优先级)。
fn register_hits(
    body: Rect,
    class: SizeClass,
    app: &crate::app::App,
    hits: &mut Vec<(Rect, Target)>,
) {
    let Some(content) = popup_content(body, class) else {
        return;
    };
    match app.reg_state {
        crate::app::RegState::Idle | crate::app::RegState::RepoInput => {}
        crate::app::RegState::PreScan => {} // 等回音:无处可点,同惰性
        crate::app::RegState::PreScanPick => {
            // 预扫选单(#114):条目行 = 勾选/取消(与空格同路,经
            // Target::SelectRow 在 app 侧按 reg_state 分派);键位行两段
            // (Enter 继续 / Esc 返航)。只给「键位行以上真的画出来了」
            // 的行产命中——与渲染同序:内层 0/1 行说明、2 行空,其后按
            // kind 分组(组头一行 + 条目行),超量截断。
            let popup = popup_rect(content, REG_PICK_W, REG_PICK_H);
            if popup.height < REG_PICK_H || popup.width == 0 {
                return; // 收窄后行序不再可信,不产命中
            }
            let inner = shrink_border(popup);
            let key_y = popup.y + popup.height - 2;
            let mut y = inner.y + 3;
            'outer: for kind in [
                crate::scan::ScanKind::Skill,
                crate::scan::ScanKind::Extension,
            ] {
                let mut any = false;
                for (i, e) in app.reg_scan_entries.iter().enumerate() {
                    if e.kind != kind {
                        continue;
                    }
                    if !any {
                        y += 1; // 组头占一行
                        any = true;
                    }
                    if y >= key_y {
                        break 'outer; // 截断线以下没画,不给命中
                    }
                    hits.push((Rect::new(inner.x, y, inner.width, 1), Target::SelectRow(i)));
                    y += 1;
                }
            }
            two_key_segments(popup, key_y, REG_KEY_PICK_CONTINUE, RITUAL_KEY_RETURN, hits);
        }
        crate::app::RegState::PreAudit => {} // 等回音:无处可点,同惰性
        crate::app::RegState::KindPick => {
            let popup = popup_rect(content, REG_KIND_W, REG_KIND_H);
            if popup.height < REG_KIND_H {
                return; // 收窄后行序不再可信,不产命中
            }
            let inner = shrink_border(popup);
            // 与渲染同序:内层首行是标题行,其后是 kind 选项两行,
            // 末行是键位行(渲染把键位行钉在内层最后一行)。
            for i in 0..crate::app::RegKind::ALL.len() as u16 {
                let y = inner.y.saturating_add(1 + i);
                if y < inner.y + inner.height {
                    hits.push((
                        Rect::new(inner.x, y, inner.width, 1),
                        Target::SelectRow(i as usize),
                    ));
                }
            }
            let key_y = popup.y + popup.height - 2;
            two_key_segments(popup, key_y, REG_KEY_KIND_LAUNCH, RITUAL_KEY_RETURN, hits);
        }
        crate::app::RegState::Review => {
            let popup = popup_rect(content, REG_REVIEW_W, REG_REVIEW_H);
            if popup.width == 0 || popup.height == 0 {
                return;
            }
            if app.reg_can_commit() {
                // 落账闸放行(#31):键位行两段——「Enter 落账」→ Activate
                // (与键盘 Enter 同路,仍过闸),「Esc 返航」→ Dismiss;
                // 几何与渲染同文案同序(key_y = 内层末行)。
                let key_y = popup.y + popup.height - 2;
                two_key_segments(popup, key_y, REG_KEY_REVIEW_COMMIT, RITUAL_KEY_RETURN, hits);
            } else {
                // 闸不放行:没绑 Enter(不可落账不可达),整屏点任意处 = 返航
                hits.push((popup, Target::DismissPopup));
            }
        }
        crate::app::RegState::Commit => {}
        // 落账在途:同预审等回音态鼠标惰性(Esc 返航归键盘);
        // 写盘互斥由 assembling 顶栏灯/HUD 同源呈现。
        crate::app::RegState::Done => {
            // 结果屏:点任意处 = 返航(同确认态不可落账分支的先例)
            let popup = popup_rect(content, REG_DONE_W, REG_DONE_H);
            if popup.width > 0 && popup.height > 0 {
                hits.push((popup, Target::DismissPopup));
            }
        }
    }
}

/// 帮助浮层:与 `render_help` 同尺寸居中,点任意处关闭(不区分内外)。
/// 尺寸用共享常量 `HELP_W` / `HELP_H`,渲染侧同一个 `popup_rect`——
/// 可见浮层与「点任意处关」的命中区几何永不分歧。
fn help_hits(area: Rect, hits: &mut Vec<(Rect, Target)>) {
    hits.push((popup_rect(area, HELP_W, HELP_H), Target::DismissPopup));
}

/// 帮助浮层尺寸:渲染(`ui::render_help`)与命中(`help_hits`)共用。
pub(crate) const HELP_W: u16 = 60;
pub(crate) const HELP_H: u16 = 30;

/// 命令面板(#119)尺寸:渲染(`ui::commands::render`)与命中(`cmd_hits`)共用。
/// 与 `HELP_W`/`HELP_H` 同族——一处几何真相,渲染与命中永不分歧。
pub(crate) const CMD_W: u16 = 64;
pub(crate) const CMD_H: u16 = 20;

/// 命令面板头部占的行数:查询行 + 提示行(结果行从内层第 `CMD_HEAD` 行起)。
pub(crate) const CMD_HEAD: u16 = 2;

/// 命令面板浮层矩形(与渲染同一个 `popup_rect` + 同一组常量)。
pub(crate) fn cmd_popup_rect(area: Rect) -> Rect {
    popup_rect(area, CMD_W, CMD_H)
}

/// 命令面板浮层内层(去掉边框,与 `Block::bordered().inner()` 同结果)。
pub(crate) fn cmd_inner(area: Rect) -> Rect {
    shrink_border(cmd_popup_rect(area))
}

/// 命令面板可见结果行数(内层高度去掉头部两行)。
pub(crate) fn cmd_visible(area: Rect) -> usize {
    cmd_inner(area).height.saturating_sub(CMD_HEAD) as usize
}

/// 结果行窗口:把选中行带进可见区(到下边界不越界、到上边界不回绕)。
/// 渲染与命中共用——窗口一偏,「画得出的那行」就点不中。
pub(crate) fn cmd_window(n: usize, sel: usize, visible: usize) -> (usize, usize) {
    if visible == 0 || n == 0 {
        return (0, 0);
    }
    let first = sel
        .saturating_sub(visible.saturating_sub(1))
        .min(n.saturating_sub(1));
    let count = visible.min(n - first);
    (first, count)
}

/// 结果行矩形清单:`(矩形, 命中下标)`。渲染按同一份几何画行——点哪打哪。
pub(crate) fn cmd_row_rects(area: Rect, first: usize, count: usize) -> Vec<(Rect, usize)> {
    let inner = cmd_inner(area);
    let top = inner.y.saturating_add(CMD_HEAD);
    (0..count)
        .filter_map(|i| {
            let y = top.checked_add(i as u16)?;
            (y < inner.y.saturating_add(inner.height))
                .then(|| (Rect::new(inner.x, y, inner.width, 1), first + i))
        })
        .collect()
}

/// 命令面板的命中区:只产结果行(点行 = 选中),浮层其余部分吞点击、不穿透
/// 底层。与 `ui::commands::render` 的画行几何同一个 `cmd_row_rects`。
pub(crate) fn cmd_hits(area: Rect, n: usize, sel: usize, hits: &mut Vec<(Rect, Target)>) {
    let visible = cmd_visible(area);
    let (first, count) = cmd_window(n, sel, visible);
    for (rect, idx) in cmd_row_rects(area, first, count) {
        hits.push((rect, Target::CmdRow(idx)));
    }
}

/// 自升级浮层(#121)尺寸:渲染(`ui::render_upgrade`)与命中(`upgrade_hits`)
/// 共用,与 `HELP_W` / `HELP_H` 同族——一处几何真相。
pub(crate) const UPGRADE_W: u16 = 66;
pub(crate) const UPGRADE_H: u16 = 18;

/// 自升级浮层矩形(与渲染同一个 `popup_rect` + 同一组常量)。
pub(crate) fn upgrade_popup_rect(area: Rect) -> Rect {
    popup_rect(area, UPGRADE_W, UPGRADE_H)
}

/// 自升级浮层内层(去掉边框,与 `Block::bordered().inner(r)` 同结果)。
pub(crate) fn upgrade_inner(area: Rect) -> Rect {
    shrink_border(upgrade_popup_rect(area))
}

/// 自升级浮层的命中区:整块浮层一个 `DismissPopup`——点哪儿都是「放弃 /
/// 关」,与键盘 Esc 同路(在途态点不动,见 `on_key_upgrade`)。浮层外不产
/// 命中区(吞点击不穿透,与帮助浮层同纪律)。
pub(crate) fn upgrade_hits(area: Rect, hits: &mut Vec<(Rect, Target)>) {
    hits.push((upgrade_popup_rect(area), Target::DismissPopup));
}

/// 与壳/浮层渲染同款的居中弹层矩形(`recipes` 登记浮层渲染直接调用,
/// 与命中侧同一个函数——居中与收窄规则永不分歧)。
pub(crate) fn popup_rect(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(4));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + (area.width.saturating_sub(w)) / 2,
        area.y + (area.height.saturating_sub(h)) / 2,
        w,
        h,
    )
}

/// `Block::bordered().inner(r)`:四周各缩 1 列(空间不足时返回空区)。
fn shrink_border(r: Rect) -> Rect {
    if r.width < 2 || r.height < 2 {
        return Rect::new(r.x, r.y, 0, 0);
    }
    Rect::new(r.x + 1, r.y + 1, r.width - 2, r.height - 2)
}

/// 在列表区内铺 `n` 个数据行(跳过前 `skip` 行,如表头)。
/// 列表宽度整块命中——点行即选中,不越出列表矩形。
fn row_hits(
    inner: Rect,
    skip: usize,
    n: usize,
    hits: &mut Vec<(Rect, Target)>,
    mk: fn(usize) -> Target,
) {
    let rows = match inner.height.checked_sub(skip as u16) {
        Some(r) => r,
        None => return,
    };
    let vis = (n as u16).min(rows);
    for i in 0..vis {
        hits.push((
            Rect::new(inner.x, inner.y + skip as u16 + i, inner.width, 1),
            mk(usize::from(i)),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::FakeRunner;
    use std::sync::Arc;

    fn area(w: u16, h: u16) -> Rect {
        Rect::new(0, 0, w, h)
    }

    fn test_app() -> App {
        let mut app = App::with_runner(Arc::new(FakeRunner::new(&[(
            "ls",
            0,
            r#"{"schema":"nw/ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[{"name":"desk","state":"drift","lockId":"9f8e"},{"name":"lab","state":"ok","lockId":"a1b2"}],"locks":[{"lockId":"9f8e","recipe":"research-desk","bakedAt":"2026-10-01"}]}}"#,
        )])));
        // with_runner 不跑引擎;列表数据直接入状态树(与 app 单测同套路)。
        app.ls = Some(
            serde_json::from_str::<crate::engine::LsData>(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[{"name":"desk","state":"drift","lockId":"9f8e"},{"name":"lab","state":"ok","lockId":"a1b2"}],"locks":[{"lockId":"9f8e","recipe":"research-desk","bakedAt":"2026-10-01"}]}"#,
            )
            .unwrap(),
        );
        app.splash = false;
        app
    }

    // ---- size_class:断点单一常量,阈值四点 ----

    #[test]
    fn 断点与下限是两个独立常量() {
        // 断点=换装阈值(原 MIN_W 80),下限=还能不能开(66×50);
        // 两者语义不同,不许链成一个值(spec #15)。
        assert_eq!(BREAKPOINT, 80);
        assert_eq!(crate::term::MIN_W, 66);
        assert_eq!(crate::term::MIN_H, 24);
    }

    #[test]
    fn size_class_断点阈值() {
        for w in [0, 1, 65, 66, 67, 79] {
            assert_eq!(
                size_class(area(w, 50)),
                SizeClass::Compact,
                "宽 {w} < 断点 ⇒ 紧凑态"
            );
        }
        for w in [80, 81, 100, 200] {
            assert_eq!(
                size_class(area(w, 30)),
                SizeClass::Comfy,
                "宽 {w} ≥ 断点 ⇒ 常畅态"
            );
        }
    }

    // ---- 命中区:66×50(紧凑)与 80×30(常畅)两档 ----

    /// 所有命中区不越出 area、且都是非空矩形。
    fn check(hits: &[(Rect, Target)], area: Rect, label: &str) {
        for (r, _) in hits {
            assert!(
                r.x >= area.x
                    && r.y >= area.y
                    && r.x + r.width <= area.x + area.width
                    && r.y + r.height <= area.y + area.height,
                "{label}:命中区 {r:?} 越出 area {area:?}"
            );
            assert!(r.width > 0 && r.height > 0, "{label}:空命中区 {r:?}");
        }
    }

    fn first(hits: &[(Rect, Target)], pred: impl Fn(&Target) -> bool) -> Option<Rect> {
        hits.iter().find(|(_, t)| pred(t)).map(|(r, _)| *r)
    }

    #[test]
    fn hitmap_常畅态_80x30_导航六屏齐() {
        let app = test_app();
        let a = area(80, 30);
        let hits = hitmap(a, &app, size_class(a));
        check(&hits, a, "80×30");
        // 只数导航轨里的 NavTo(宽 = NAV_WIDTH);概览指令栏第五条「浏览编目」也是
        // NavTo(Vps)(#68),但它在正文右栏、宽 ≠ 导航轨,不计入「导航六屏」。
        let nav = hits
            .iter()
            .filter(|(r, t)| matches!(t, Target::NavTo(_)) && r.width == NAV_WIDTH)
            .count();
        assert_eq!(
            nav,
            Screen::ALL.len(),
            "导航屏数同 Screen::ALL(#53 起六屏:概览/装配/健康/配方/编目/设置)"
        );
        // 顺序同 Screen::ALL:顶栏占 y=0,导航块边框占 y=1,首项自 y=2
        for (i, screen) in Screen::ALL.iter().enumerate() {
            let r = first(&hits, |t| matches!(t, Target::NavTo(s) if s == screen)).unwrap();
            assert_eq!(r.y, 2 + i as u16, "导航行序同 Screen::ALL(边框之后)");
            assert_eq!(r.width, NAV_WIDTH, "导航命中区贴齐导航轨");
        }
    }

    #[test]
    fn hitmap_常畅态_点列表行得选中目标() {
        let mut app = test_app();
        let a = area(80, 30);
        app.screen = Screen::Overview;
        let hits = hitmap(a, &app, size_class(a));
        let rows: Vec<_> = hits
            .iter()
            .filter(|(_, t)| matches!(t, Target::SelectRow(_)))
            .collect();
        assert_eq!(rows.len(), 2, "概览两个派 = 两行命中区(按序)");
        assert_eq!(
            first(&hits, |t| matches!(t, Target::SelectRow(1))),
            Some(rows[1].0),
            "SelectRow(1) 指第二行"
        );
        // 派表首行:屏体边框 → 警示带 1 行 → 主区 → 派表边框 + 表头 1 行 ⇒ y=5
        assert_eq!(
            rows[0].0,
            Rect::new(14, 5, 43, 1),
            "派表首行与渲染同序(嵌套边框后数据自 y=5)"
        );

        app.screen = Screen::Assemble;
        let hits = hitmap(a, &app, size_class(a));
        assert_eq!(
            first(&hits, |t| matches!(t, Target::SelectRow(0))),
            Some(Rect::new(14, 3, 34, 1)),
            "选锁定集首行:屏体边框 → 55% 左栏 → 列表边框,首行即数据"
        );
    }

    #[test]
    fn hitmap_空白不得命中() {
        let mut app = test_app();
        let a = area(80, 30);
        app.screen = Screen::Overview;
        let hits = hitmap(a, &app, size_class(a));
        assert!(
            hits.iter().all(|(r, _)| r.y != 0),
            "顶栏无命中区:点顶栏不得切屏/选中"
        );
        // 设置屏无列表:除导航外无命中区
        app.screen = Screen::Settings;
        let hits = hitmap(a, &app, size_class(a));
        assert!(
            hits.iter().all(|(_, t)| matches!(t, Target::NavTo(_))),
            "设置屏只有导航命中区,点正文空白不命中"
        );
        // 无 ls 数据:正文无列表行(没有可选中的行)
        let mut bare = test_app();
        bare.ls = None;
        bare.screen = Screen::Overview;
        let hits = hitmap(a, &bare, size_class(a));
        assert!(
            hits.iter().all(|(_, t)| matches!(t, Target::NavTo(_))),
            "没有 ls 数据就没有列表行可命中"
        );
    }

    /// #68:在整帧真显示里定位某子串的 (行, 列)(列按显示宽累计——frame_display
    /// 已跳过宽字续格,字节偏移≠终端列,与命中区同一把尺)。
    fn locate(frame: &str, needle: &str) -> (u16, u16) {
        use unicode_width::UnicodeWidthChar;
        for (y, line) in frame.split('\n').enumerate() {
            if let Some(byte) = line.find(needle) {
                let col: u16 = line[..byte]
                    .chars()
                    .map(|c| UnicodeWidthChar::width(c).unwrap_or(0) as u16)
                    .sum();
                return (y as u16, col);
            }
        }
        panic!("整帧里找不到「{needle}」");
    }

    /// #68 概览指令栏第五条「浏览编目」可点:命中区是 `NavTo(Vps)`,几何与渲染
    /// 同源——点在帧上「浏览编目」所在行列必落进该命中区(点哪打哪,ADR-0005);
    /// 常畅/紧凑两档都产、都不越界。指令栏是动线不是能力表,故与 ls 有无数据无关
    /// (复用渲染同一组 `Layout::vertical([Length(7), Min(3)])` + 边框,不另写几何)。
    #[test]
    fn hitmap_概览指令栏第五条_浏览编目可点_两档同源() {
        for (w, h) in [(80u16, 30u16), (66, 50)] {
            let mut app = test_app();
            app.screen = Screen::Overview;
            let a = area(w, h);
            let hits = hitmap(a, &app, size_class(a));
            check(&hits, a, "{w}×{h}");
            let frame = crate::testutil::frame_display(&app, w, h);
            let (row, col) = locate(&frame, "浏览编目");
            let hit = hits
                .iter()
                .find(|(r, _)| r.y == row && col >= r.x && col < r.x + r.width)
                .map(|(_, t)| *t)
                .unwrap_or_else(|| panic!("{w}×{h} 「浏览编目」行应有命中区"));
            assert_eq!(
                hit,
                Target::NavTo(Screen::Vps),
                "{w}×{h} 指令栏第五条 = 到编目屏"
            );
        }
    }

    #[test]
    fn hitmap_紧凑态_66x50_也有导航与列表() {
        let mut app = test_app();
        let a = area(66, 50);
        let class = size_class(a);
        assert_eq!(class, SizeClass::Compact);
        let hits = hitmap(a, &app, class);
        check(&hits, a, "66×50");
        assert!(
            hits.iter().any(|(_, t)| matches!(t, Target::NavTo(_))),
            "紧凑态:导航命中区 = 横向屏 Tab(#17)"
        );
        app.screen = Screen::Overview;
        let hits = hitmap(a, &app, class);
        assert_eq!(
            first(&hits, |t| matches!(t, Target::SelectRow(0))),
            Some(Rect::new(2, 6, 42, 1)),
            "紧凑态 66 宽:Tab 条让出首行,导航列还给正文后——屏体边框 → 警示带 → 68% 分栏的整数落点"
        );
    }

    /// #17 落地后紧凑态外壳的真实几何:一行横向屏 Tab,
    /// 顺序同 Screen::ALL、单元宽与渲染同一个 `tab_cell`。
    #[test]
    fn hitmap_紧凑态_屏tab条几何与渲染同源() {
        let mut app = test_app();
        let a = area(66, 50);
        let hits = hitmap(a, &app, size_class(a));
        // 只取屏 Tab 条(y=1)的 NavTo;概览指令栏第五条「浏览编目」也是 NavTo(Vps)
        // (#68),但它在正文右栏更低处(y≠1),不计入 Tab 条几何。
        let nav: Vec<_> = hits
            .iter()
            .filter(|(r, t)| matches!(t, Target::NavTo(_)) && r.y == 1)
            .collect();
        assert_eq!(
            nav.len(),
            Screen::ALL.len(),
            "屏 Tab 数同 Screen::ALL(#53 起六屏)"
        );
        let mut x = 0u16;
        for (i, screen) in Screen::ALL.iter().enumerate() {
            let (r, t) = nav[i];
            assert_eq!(*t, Target::NavTo(*screen), "Tab 顺序同 Screen::ALL");
            assert_eq!(r.y, 1, "Tab 条在 body 首行(顶栏之下)");
            assert_eq!(r.x, x, "Tab 单元首尾相接不重叠");
            let cell = tab_cell(*screen, *screen == app.screen);
            assert_eq!(
                r.width,
                unicode_width::UnicodeWidthStr::width(cell.as_str()) as u16,
                "命中区宽 = 渲染同一个 tab_cell 的显示宽(点哪打哪)"
            );
            x += r.width;
        }
        assert!(x <= a.width, "Tab 条不溢出终端");
        // 切屏后选中标记跟着走:重算命中区,单元宽不因高亮改变
        app.screen = Screen::Recipes;
        let hits2 = hitmap(a, &app, size_class(a));
        let x_after: u16 = hits2
            .iter()
            .filter(|(r, t)| matches!(t, Target::NavTo(_)) && r.y == 1)
            .map(|(r, _)| r.width)
            .sum();
        assert_eq!(x, x_after, "j/k 切屏不改变 Tab 条几何");
    }

    #[test]
    fn hitmap_弹层优先级_浮层外无底层命中区() {
        let mut app = test_app();
        let a = area(80, 30);
        // 发射仪式:只有键位行两段(发射/返航),导航/列表全部让位
        app.as_confirm = true;
        app.screen = Screen::Assemble;
        let hits = hitmap(a, &app, size_class(a));
        assert!(
            hits.iter().any(|(_, t)| *t == Target::Activate),
            "仪式「Enter 发射」行可点"
        );
        assert!(
            hits.iter()
                .all(|(_, t)| matches!(t, Target::Activate | Target::DismissPopup)),
            "浮层打开:只有仪式两段(发射/返航),底层点击不命中(#20 返航亦可点)"
        );
        // 帮助浮层:任意处关闭
        app.as_confirm = false;
        app.help = true;
        let hits = hitmap(a, &app, size_class(a));
        assert_eq!(
            first(&hits, |t| *t == Target::DismissPopup),
            Some(Rect::new(10, 2, 60, 26)),
            "帮助浮层与渲染同尺寸居中(body 区 60×30 钳到 26),点任意处关闭"
        );
        assert!(
            hits.iter().all(|(_, t)| matches!(t, Target::DismissPopup)),
            "帮助浮层吞掉所有底层点击"
        );
        // 手填 NW_HOME 浮层:对鼠标惰性,不产出任何命中区
        app.help = false;
        app.home_input = true;
        app.screen = Screen::Settings;
        let hits = hitmap(a, &app, size_class(a));
        assert!(
            hits.is_empty(),
            "手填浮层对鼠标惰性:零命中区(#20 不分派,故事 16)"
        );
        // 烘焙两槽位手填(#93):同一级浮层,同一条惰性纪律
        app.home_input = false;
        app.slot_input = Some(crate::app::BakeSlot::AssetsDir);
        let hits = hitmap(a, &app, size_class(a));
        assert!(hits.is_empty(), "槽位手填浮层同样对鼠标惰性:零命中区");
    }

    #[test]
    fn hitmap_仪式键位行_发射返航分半_与渲染同一行() {
        let mut app = test_app();
        app.screen = Screen::Assemble;
        app.as_confirm = true;
        let a = area(80, 30);
        let hits = hitmap(a, &app, size_class(a));
        // popup 在正文列内居中(常畅去左导航轨):x=12+(68-60)/2=16,
        // y=1+(28-14)/2=8;键位行是 block 内第 9 行 ⇒ y=8+1(边框)+9=18;
        // 两段起止贴渲染文案(内层首列 17 起,宽 = RITUAL_KEY_* 同一串量宽)。
        // y 修正自 #16 期的 +12(错位两行);x 修正自评审 finding-1 的
        // 恒偏导航列宽(旧期望 11/27 → 17/33,点射落进返航段的错位)。
        let act = first(&hits, |t| *t == Target::Activate).expect("发射段可点");
        assert_eq!(
            act,
            Rect::new(17, 18, 13, 1),
            "发射段贴渲染「Enter 发射」文案"
        );
        let back = first(&hits, |t| *t == Target::DismissPopup).expect("返航段可点");
        assert_eq!(
            back,
            Rect::new(33, 18, 9, 1),
            "返航段贴渲染「Esc 返航」文案"
        );
        assert!(act.x + act.width < back.x, "两段不重叠,段间空隙不歧义");
        assert!(hits.iter().all(|(r, _)| r.y == 18), "仪式只产键位行");
        // 渲染同源:键位文案就在命中同一行(点哪打哪)
        let mut term =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(a.width, a.height)).unwrap();
        term.draw(|f| crate::ui::render(f, &app)).unwrap();
        let s = term.backend().to_string();
        let row = s.split('\n').nth(18).expect("第 18 行在场");
        assert!(
            row.contains("Enter 发射") && row.contains("Esc 返航"),
            "键位文案与命中同处 y=18"
        );
    }

    /// 仪式键位行的列级同源断言:渲染整段 RITUAL_KEY_LAUNCH 的起始列
    /// == 命中 Activate 矩形的 x,且渲染「射」字所在列落在 Activate 内
    /// (评审 finding-1 的 6 列错位正是行级 contains 断言漏掉的)。
    fn assert_ritual_column_aligned(app: &App, w: u16, h: u16) {
        use unicode_width::UnicodeWidthChar;
        let a = area(w, h);
        let hits = hitmap(a, app, size_class(a));
        let act = first(&hits, |t| *t == Target::Activate).expect("发射段可点");
        // 与 golden 同一把尺:整帧真显示串(testutil::frame_display),
        // to_string 的引号/隐藏格注记不能当裸行。
        let frame = crate::testutil::frame_display(app, w, h);
        let row = frame
            .split('\n')
            .nth(act.y as usize)
            .expect("命中行在渲染中在场");
        let idx = row
            .find(RITUAL_KEY_LAUNCH)
            .expect("渲染行铺着整段发射键位文案");
        let col: u16 = row[..idx]
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0) as u16)
            .sum();
        assert_eq!(
            col, act.x,
            "渲染整段 RITUAL_KEY_LAUNCH 起始列 {col} == 命中 Activate.x {}({w}×{h})",
            act.x
        );
        // 「点射即发射」:可见的「射」字列必须落在 Activate 矩形内
        // (修复前恒偏导航列宽 6 列,「射」落进返航段)。
        let idx = row.find('射').expect("渲染含「射」字");
        let glyph: u16 = row[..idx]
            .chars()
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0) as u16)
            .sum();
        assert!(
            act.x <= glyph && glyph < act.x + act.width,
            "「射」列 {glyph} 落在 Activate {}..{} 内({w}×{h})",
            act.x,
            act.x + act.width
        );
    }

    #[test]
    fn hitmap_仪式列级同源_常畅80x30() {
        let mut app = test_app();
        app.screen = Screen::Assemble;
        app.as_confirm = true;
        assert_ritual_column_aligned(&app, 80, 30);
    }

    #[test]
    fn hitmap_仪式列级同源_紧凑66x50() {
        let mut app = test_app();
        app.screen = Screen::Assemble;
        app.as_confirm = true;
        assert_ritual_column_aligned(&app, 66, 50);
    }

    #[test]
    fn hitmap_仪式列级同源_紧凑66x31奇数高() {
        // 评审 finding-1 附带:奇数高度下旧代码居中在 body(未去 Tab 行),
        // 命中行比渲染行高 1;同根因,同一把尺钉住。
        let mut app = test_app();
        app.screen = Screen::Assemble;
        app.as_confirm = true;
        assert_ritual_column_aligned(&app, 66, 31);
    }

    /// 引用完整性(#116)三块几何:横幅占 5 行,命中侧的检查行首行就是渲染侧的
    /// 首行(与 recheck 同一条纪律:几何收在一处,渲染与命中不可能各写一套);
    /// 数据仓根不就绪时整栏是指路,不产可选中行。
    #[test]
    fn hitmap_引用完整性_横幅占五行_检查行与渲染同一行() {
        let mut app = test_app();
        app.screen = Screen::Health;
        app.hl_mode = crate::app::HealthMode::Integrity;
        let dir = std::env::temp_dir().join(format!("nwt-layout-refs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let put = |rel: &str, body: &str| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        put("skills/lonely/SKILL.md", "x");
        put(
            "recipes/only/recipe.yaml",
            "kind: recipe\nname: only\ncomponents:\n  - component: ghost\n",
        );
        app.data_root = Some(dir.display().to_string());
        app.run_integrity();
        let a = area(80, 30);
        assert_eq!(
            integrity_panels(a)[0].height,
            5,
            "横幅 = 3 行文案 + 2 行边框(与 recheck_panels 同组常量)"
        );
        let hits = hitmap(a, &app, size_class(a));
        let row0 = first(&hits, |t| *t == Target::SelectRow(0)).expect("检查行首行可点");
        let frame = crate::testutil::frame_display(&app, a.width, a.height);
        let rendered = frame
            .split('\n')
            .nth(row0.y as usize)
            .expect("命中行在渲染中在场");
        assert!(
            rendered.contains("ghost"),
            "命中行 = 渲染检查行首行(断链在前):{rendered}"
        );
        // 数据仓根不就绪:整栏是指路,没有可选中行(与 recheck 门一同形)
        let mut gated = test_app();
        gated.screen = Screen::Health;
        gated.hl_mode = crate::app::HealthMode::Integrity;
        gated.data_root = None;
        assert!(
            hitmap(a, &gated, size_class(a))
                .iter()
                .all(|(_, t)| !matches!(t, Target::SelectRow(_))),
            "根不就绪时没有可选中行(指路屏)"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// recheck 屏(#40)四块几何:横幅确实占 5 行(3 行文案 + 2 行块边框,
    /// 与 [`recheck_panels`] 注释同源),且命中侧的清单首行就是渲染侧的
    /// 清单首行——几何收在一处后,渲染与命中不可能再各写一套。
    #[test]
    fn hitmap_recheck_横幅占五行_清单行与渲染同一行() {
        use crate::engine::{Capability, Probe};
        let mut app = test_app();
        app.screen = Screen::Health;
        app.hl_mode = crate::app::HealthMode::Recheck;
        app.capabilities
            .record(Capability::SourceRecheck, Probe::Available, None);
        app.ls = Some(
            serde_json::from_str::<crate::engine::LsData>(
                r#"{"domain":"all","recipes":[],"components":[],"sources":[{"name":"web-search","kind":"skill","entries":[]},{"name":"theme-pack","kind":"extension","entries":[]}],"profiles":[],"locks":[]}"#,
            )
            .unwrap(),
        );
        let a = area(80, 30);
        assert_eq!(recheck_panels(a)[0].height, 5, "横幅 = 3 行文案 + 2 行边框");
        let hits = hitmap(a, &app, size_class(a));
        let row0 = first(&hits, |t| *t == Target::SelectRow(0)).expect("清单首行可点");
        let frame = crate::testutil::frame_display(&app, a.width, a.height);
        let rendered = frame
            .split('\n')
            .nth(row0.y as usize)
            .expect("命中行在渲染中在场");
        assert!(
            rendered.contains("web-search"),
            "命中行 = 渲染清单首行:{rendered}"
        );
        assert!(
            hits.iter()
                .all(|(_, t)| !matches!(t, Target::ToggleSuggest)),
            "配方域建议命令的键位行不归健康屏"
        );
    }

    #[test]
    fn hitmap_配方行随尺寸档分栏_66与80两态() {
        // #18:常畅态配方列表仍在 14 列竖轨右侧(45% 栏),紧凑态随 Tab 行
        // 收起后落在全宽 52% 栏——几何两支都由 render_recipes 同款
        // Layout 算出,命中与渲染同源(点哪打哪)。
        let mut app = test_app();
        app.screen = Screen::Recipes;
        app.rc_domain = crate::app::Domain::Profiles; // 测试数据里派有 2 条

        let a80 = area(80, 30);
        let hits = hitmap(a80, &app, size_class(a80));
        let comfy_first =
            first(&hits, |t| matches!(t, Target::SelectRow(0))).expect("常畅态配方行有命中");
        let a66 = area(66, 50);
        let hits = hitmap(a66, &app, size_class(a66));
        let compact_first =
            first(&hits, |t| matches!(t, Target::SelectRow(0))).expect("紧凑态配方行有命中");
        assert_eq!(
            compact_first,
            // #17 落地真实紧凑外壳后:竖导航列还给正文(x 14→2)、Tab 条让出
            // 首行(y 4→5);栏内 52% 分栏由 render_recipes 同款 Layout 算出。
            Rect::new(2, 5, 31, 1),
            "紧凑态:真实 Tab 行 + 全宽正文 → 列表在 52% 左栏,首行数据自 y=5"
        );
        assert_ne!(
            compact_first, comfy_first,
            "两档分栏不同:紧凑态不得沿用常畅态几何"
        );
        // 常畅态首行仍是竖轨右侧 45% 栏(既有几何,不回退)
        assert_eq!(
            comfy_first,
            Rect::new(28, 3, 24, 1),
            "常畅态:14 列竖轨 + 45% 栏,首行数据自 y=3"
        );
        // 两档所有行命中都落在各自列表区内、不越出 area
        for (r, _) in hits.iter() {
            assert!(r.x + r.width <= a66.x + a66.width, "命中区不越界");
        }
    }

    #[test]
    fn hitmap_过窄终端_不产出命中区() {
        let app = test_app();
        let a = area(30, 10); // 守卫屏状态:无导航、无列表
        let hits = hitmap(a, &app, size_class(a));
        assert!(hits.is_empty(), "正文不足时不铺命中区");
    }

    // ---- #21 stderr 日志滚动区:装配运行中才有,与 render_running 同款分栏 ----

    #[test]
    fn hitmap_stderr日志区_装配运行中产出_闲置不产() {
        let mut app = test_app();
        app.screen = Screen::Assemble;
        app.assembling = true;
        for (w, h) in [(80, 30), (66, 50)] {
            let a = area(w, h);
            let r = first(&hitmap(a, &app, size_class(a)), |t| {
                matches!(t, Target::ScrollLog(_))
            })
            .unwrap_or_else(|| panic!("{w}×{h} 装配运行中应有日志滚动区"));
            assert!(r.width > 0 && r.height > 0, "{w}×{h} 日志区非空");
            assert!(
                r.x + r.width <= a.x + a.width && r.y + r.height <= a.y + a.height,
                "日志滚动区不越出终端面"
            );
        }
        // 闲置选锁定集:无日志区,滚轮无处可滚(几何来自同一个 assemble_hits)
        app.assembling = false;
        let a = area(80, 30);
        assert!(
            !hitmap(a, &app, size_class(a))
                .iter()
                .any(|(_, t)| matches!(t, Target::ScrollLog(_))),
            "闲置装配屏不产日志滚动区"
        );
    }

    /// #71 装配中止段命中区:只在真有 up 在途(`as_running`)时产出,与 stderr 日志
    /// 滚动区同源于 `assemble_hits`(同一组 `Layout`——渲染与命中共用一份几何,
    /// ADR-0005)。常畅(80×30)与紧凑(66×50)两档都在场、是 1 行可点段、不越界;
    /// 在途标记一收敛(仍 assembling = 运行视图在场)即不产——点不到一个当前不
    /// 存在的动作(规格故事 47)。
    #[test]
    fn hitmap_装配中止段_在途产出_闲置不产() {
        let mut app = test_app();
        app.screen = Screen::Assemble;
        app.assembling = true;
        app.as_running = true;
        for (w, h) in [(80u16, 30u16), (66, 50u16)] {
            let a = area(w, h);
            let hits = hitmap(a, &app, size_class(a));
            check(&hits, a, "{w}×{h}");
            let r = first(&hits, |t| *t == Target::Abort)
                .unwrap_or_else(|| panic!("{w}×{h} up 在途应有中止段命中区"));
            assert!(r.height == 1, "{w}×{h} 中止段是 1 行可点段");
        }
        // 在途标记收敛(仍 assembling,运行视图在场):中止段不产命中区
        app.as_running = false;
        let a = area(80, 30);
        assert!(
            hitmap(a, &app, size_class(a))
                .iter()
                .all(|(_, t)| *t != Target::Abort),
            "非 up 在途:中止段不产命中区"
        );
    }

    // ---- #67 接缝的纯函数:`key_line_segments` 的段表分解与收口 ----
    //
    // 段表分解是被钉住的几何口径(规格故事 31):一行有哪些段、各多宽、哪段可点、
    // 放不放得下——纯函数、可单测,不依赖任何屏状态。

    /// 段按序铺开:段宽 = `unicode_width(文案)`,段 x = 行内累计显示宽,段间不
    /// 重叠、不留歧义区;`target` 原样带着(`None` = 只画不点)。
    #[test]
    fn key_line_segments_按序铺开_段宽按unicode宽度量() {
        let anchor = Rect::new(4, 9, 40, 1);
        let parts: [KeyPart; 3] = [
            ("  ↑/↓ 选", None, true),
            ("g 取回", Some(Target::CatalogGet), true),
            ("m 换册", Some(Target::SwitchBook), true),
        ];
        let segs = key_line_segments(anchor, 9, &parts, " ·");
        assert_eq!(segs.len(), 3, "三段都放得下:{segs:?}");
        // 第一段贴在 anchor 左界(含段内前导空格),后续段 = 上一段末 + gap 宽
        assert_eq!(segs[0].rect.x, 4);
        let gap = unicode_width::UnicodeWidthStr::width(" ·");
        assert_eq!(
            segs[1].rect.x,
            segs[0].rect.x + segs[0].rect.width + gap as u16,
            "段 x = 行内累计显示宽"
        );
        for s in &segs {
            assert_eq!(
                s.rect.width as usize,
                unicode_width::UnicodeWidthStr::width(s.text.as_str()),
                "段宽 = 文案显示宽({})",
                s.text
            );
            assert_eq!(s.rect.y, 9, "段高 1 行,落在给定 y 上");
        }
        assert_eq!(segs[1].target, Some(Target::CatalogGet));
        assert_eq!(segs[0].target, None, "提示段不带命中目标");
    }

    /// **整段可见才可点**(#67 纪律,#73 验收 8):任一段完整越出 anchor 右界,
    /// 这一段及后续既不画也不给命中——放不下的段一个矩形都不产,不会出现
    /// 「屏上半截、点得到另外半截」的半个命中。
    ///
    /// 为什么在纯函数上钉:四本册的键位行今天在 66×50 下限内都放得下(#69 收窄
    /// 分隔后还剩余量,见 `过滤段四本册两档都放得下_渲染与命中共源`),所以在真
    /// 尺寸上构造不出「g 段被裁」——这条纪律只能就着 anchor 宽度直接钉,否则
    /// 将来键位行再长一截、或下限再降,裁切行为悄悄退化也没人红。
    #[test]
    fn key_line_segments_放不下整段收口_不给半个命中() {
        // 「ab」(2)+ gap(1)+ 「cd」(2)+ gap(1)+ 「ef」(2)= 8 列,anchor 只给 6 列
        let anchor = Rect::new(0, 0, 6, 1);
        let parts: [KeyPart; 3] = [
            ("ab", None, true),
            ("cd", Some(Target::CatalogGet), true),
            ("ef", None, true),
        ];
        let segs = key_line_segments(anchor, 0, &parts, " ");
        assert_eq!(
            segs.len(),
            2,
            "第三段越界:这一段及后续都不画不命中:{segs:?}"
        );
        assert_eq!(segs[1].rect.x, 3, "第二段仍整段在场");
        assert_eq!(segs[1].rect.width, 2);
        // 产出的矩形一个都不越界;第 5、6 两列(被收掉的第三段)没有任何命中
        for s in &segs {
            assert!(
                s.rect.x + s.rect.width <= anchor.x + anchor.width,
                "段不越出 anchor 右界:{s:?}"
            );
        }
        assert!(
            segs.iter().all(|s| s.rect.x + s.rect.width <= 5),
            "被裁掉的那一列起无命中(不是半个命中):{segs:?}"
        );
        // 空段也到此为止(画不出东西的段不给命中)
        let blank: [KeyPart; 2] = [("ab", None, true), ("", Some(Target::CatalogGet), true)];
        assert_eq!(
            key_line_segments(Rect::new(0, 0, 10, 1), 0, &blank, " ").len(),
            1,
            "空段不画不给命中"
        );
    }

    /// 编目册键位行的段表(#73):订阅 / 钥匙册的非空册体变体带一个
    /// `CatalogGet` 段,空册 / 在途 / 机器册 / 标签册一个都没有——段表是
    /// 「这一段在不在场」的唯一真相,命中区只照它取矩形。
    #[test]
    fn 编目册段表_取回段按册按变体在场() {
        let mut app = test_app();
        app.screen = Screen::Vps;
        app.catalog_kind = crate::screens::CatalogKind::Subscription;
        let gets = |app: &App| -> usize {
            crate::screens::catalog::catalog_key_parts(app)
                .iter()
                .filter(|(_, t, _)| *t == Some(Target::CatalogGet))
                .count()
        };
        // 空册:没有条目就没有「取回哪一条」
        assert_eq!(gets(&app), 0, "空册变体无取回段");
        // 非空册体:恰好一段
        app.sub_ls = Some(
            serde_json::from_str(
                r#"{"catalogPath":"/r/subscriptions.json","subscriptions":[{"name":"airport","kind":"subscription-url","client":"Mihomo","expires":"2099-03-01","status":"active","daysLeft":1}]}"#,
            )
            .unwrap(),
        );
        assert_eq!(gets(&app), 1, "订阅册非空册体恰一个取回段");
        // 钥匙册同款
        app.catalog_kind = crate::screens::CatalogKind::Key;
        assert_eq!(gets(&app), 0, "换到钥匙册:订阅册的投影不跟过来");
        app.key_ls = Some(
            serde_json::from_str(
                r#"{"catalogPath":"/r/keys.json","keys":[{"name":"ci","kind":"github-key","host":"h","scopes":[],"rotatedAt":"2025-11-02"}]}"#,
            )
            .unwrap(),
        );
        assert_eq!(gets(&app), 1, "钥匙册非空册体恰一个取回段");
        // 机器册 / 标签册:任何变体都没有
        for kind in [
            crate::screens::CatalogKind::Vps,
            crate::screens::CatalogKind::Tag,
        ] {
            app.catalog_kind = kind;
            assert_eq!(gets(&app), 0, "{kind:?} 册没有取回段");
        }
        // 在途变体:整行换成浮层提示
        app.catalog_kind = crate::screens::CatalogKind::Subscription;
        app.sub_rm_confirm = true;
        assert_eq!(gets(&app), 0, "删除确认态无取回段");
    }
}
