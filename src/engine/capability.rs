//! 能力探测门(spec #24 / 工单 #29):面板问引擎「你有什么」,缺的能力
//! 入口禁用并指路,面板绝不自己造替代品、也绝不替引擎回答。
//!
//! 探测铁律(与 ADR-0007「跟随引擎」同源):
//! - **一次无害调用 + 读引擎自己的回答**:回 envelope = 子命令在;
//!   stderr 人话「未知子命令」= 不存在;解析不出结论(引擎缺失/无响应)
//!   = 探测失败。探针皆无副作用(只读,不落任何文件)。
//! - **不建本地能力常量表**:下面的映射只记「面板入口 → 探什么命令」,
//!   答案永远来自引擎当下这一跑,不记某版本有没有——上游一发版面板
//!   自动跟上。
//! - 探测失败走既有指路屏语义(见 [`GateHint`]),不瘫痪、不冒充。
//! - **证据是引擎原文,随进程生命周期**:每次探测的证据(见
//!   [`ProbeEvidence`])只从那一跑的回放取,原样入账、原样取回;不落盘、
//!   不进崩溃上报、不跨启动缓存 —— 重启重探即刷新,面板不建本地能力版本表。
//!
//! # 入口清单怎么长(#63 立的规矩)
//!
//! **入口清单由面板定义,可用性由引擎决定**:下面 [`Capability::ALL`] 的每一项
//! 都是面板自己要在设置屏摆出来的入口;它当下开不开,只由那一跑探针的回答
//! 决定。两条边界钉死:
//! 1. **探针必须指向上游真实存在的命令面**。`--category` 是
//!    `nw source register` 的旗标、不是子命令,所以「类别」不配占一个能力项
//!    (#63 移除)——问一个上游不回答的问题,门就一直关着,操作者还以为引擎旧。
//!    上游新增 kind 时同理:先在上游 `cli.go` 的 `Run` switch 里看见它,再谈探针。
//! 2. **新增入口必须落在 #51 差异表里点名过**(`notes/51-follow-sync-diff.md`
//!    §5),不许谁想加就加:入口清单是面板对操作者的承诺,每加一项就多一次
//!    无害调用、多一行设置屏。
//!
//! 组级探针(`["key"]` 这种,与既有 `["vps"]` 同粒度)无害:无子命令即在组分派处
//! 被拒(rc 3 + 对应 envelope),不落文件、不读 stdin。`get` 永远不进探针清单
//! (`nw key get` / `nw subscription get` 是上游唯一回显本体的两个出口)。

use std::collections::HashMap;

use super::runner::EngineOutput;

/// 面板入口依赖的引擎能力。映射只到「问引擎哪个命令」为止。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Capability {
    /// 外部源登记入口(#30 的前置门):`nw source register`。
    SourceRegister,
    /// 外部源复核(只读,漂移结论落健康屏):`nw source recheck`。
    SourceRecheck,
    /// 配方烘焙(作者侧,建议命令的来源):`nw recipe bake`。
    /// 探针无参即在「恰好要一个配方」处被拒(rc 3 + `nw/bake.v1` envelope),不写任何文件。
    RecipeBake,
    /// 配方新建·骨架(作者侧,建议命令的来源,#58):`nw recipe new`。
    /// 与 [`Capability::RecipeBake`] 同粒度(同属 `recipe` 命令组),单独立项
    /// 而不是复用烘焙的结论——判据仍只到「问引擎哪个命令」为止,不建本地
    /// 能力常量表;上游若只发其一,面板自动跟上。
    /// 探针无参即在「位置参数必须恰好一个」处被拒(rc 3 + `nw/recipe-new.v1`
    /// envelope),无害、不写任何文件。
    RecipeNew,
    /// vps 命令组(一台机器的连接信息编目,`vps.json`):`nw vps`。
    /// 组级探针,与下面三条同粒度。
    Vps,
    /// key 命令组(github key 编目,`keys.json`):`nw key`。
    /// 组级探针(与 [`Capability::Vps`] 同粒度):无子命令即在组分派处被拒
    /// (rc 3 + `nw/key-register.v1` envelope),不落文件、不读 stdin
    /// (本体只从 `key register --token-stdin` 进,探针不带那枚旗标)。
    Key,
    /// 订阅命令组(订阅链接编目,`subscriptions.json`):`nw subscription`。
    /// 组级探针:无子命令即被拒(rc 3 + `nw/subscription-register.v1`
    /// envelope),同上无害。
    Subscription,
    /// 标签命令组(标签编目,`tags.json`):`nw tag`。
    /// 组级探针:无子命令即被拒(rc 3 + `nw/tag-add.v1` envelope),同上无害。
    Tag,
}

impl Capability {
    /// 面板入口清单(设置屏按此渲染,顺序即显示顺序)。
    ///
    /// 这份清单由**面板**定义(#63 立的规矩):每一项都要能在上游 `cli.go`
    /// 的 `Run` switch 里指到一个真实命令面,且必须在 #51 差异表里点名过;
    /// 开不开由引擎当跑的探针回答决定,清单本身不预告任何版本有无。
    pub const ALL: [Capability; 8] = [
        Capability::SourceRegister,
        Capability::SourceRecheck,
        Capability::RecipeBake,
        Capability::RecipeNew,
        Capability::Vps,
        Capability::Key,
        Capability::Subscription,
        Capability::Tag,
    ];

    /// 入口名(操作者视角,禁用与指路文案用)。
    /// 词一律有出处:登记/复核/烘焙/新建/订阅/标签是面板领域词(CONTEXT.md),
    /// vps / key 是上游命令组名本身——不自造词。
    pub fn label(self) -> &'static str {
        match self {
            Capability::SourceRegister => "外部源登记",
            Capability::SourceRecheck => "外部源复核",
            Capability::RecipeBake => "配方烘焙",
            Capability::RecipeNew => "配方新建",
            Capability::Vps => "vps",
            Capability::Key => "key",
            Capability::Subscription => "订阅",
            Capability::Tag => "标签",
        }
    }

    /// 探针 argv:一次无害调用(不带 --yes / --repo 等会动文件的旗标)。
    /// 探针只指向上游真实存在的命令面:四段式子命令(register/recheck/bake/new)
    /// 无参即在各自参数校验处被拒,四条组级探针(vps/key/subscription/tag)
    /// 无子命令即在组分派处被拒——都不落文件、不读 stdin、不回显本体。
    pub fn probe_args(self) -> Vec<String> {
        match self {
            Capability::SourceRegister => vec!["source".into(), "register".into()],
            Capability::SourceRecheck => vec!["source".into(), "recheck".into()],
            Capability::RecipeBake => vec!["recipe".into(), "bake".into()],
            Capability::RecipeNew => vec!["recipe".into(), "new".into()],
            Capability::Vps => vec!["vps".into()],
            Capability::Key => vec!["key".into()],
            Capability::Subscription => vec!["subscription".into()],
            Capability::Tag => vec!["tag".into()],
        }
    }

    /// 恰为某能力的探针 argv 时返回该能力(精确匹配:真实的
    /// `source register --repo …` 不会被当成探针回放)。
    pub fn from_probe(args: &[String]) -> Option<Capability> {
        Capability::ALL.into_iter().find(|c| c.probe_args() == args)
    }
}

/// 探测三态:引擎自己的回答只分这三种,不存在第四种「面板猜」。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Probe {
    /// 引擎回了 envelope —— 子命令在,入口可用。
    Available,
    /// stderr 人话「未知子命令」—— 引擎没有,入口禁用。
    Unavailable,
    /// 无 envelope 也无「未知子命令」:引擎缺失/无响应/回答不可解 ——
    /// 探测失败,不冒充任何一种,走指路语义。
    Unknown,
}

impl Probe {
    pub fn label(self) -> &'static str {
        match self {
            Probe::Available => "可用",
            Probe::Unavailable => "入口禁用",
            Probe::Unknown => "未决",
        }
    }
}

/// 缺能力/探测失败的指路句:说清缺的是引擎能力、不是面板坏了,
/// 升级所配 nw 后入口自动开启;探测失败另有指路屏(找 nw 三步)。
pub const GATE_HINT: &str = "缺的是引擎能力,不是面板坏了;升级所配 nw 后入口自动开启。";

/// 读引擎自己的回答,判定能力有无。
///
/// 判定顺序即信任顺序:引擎亲口说「未知子命令」优先于 envelope
/// (老引擎若把未知子命令也包成 INPUT_INVALID envelope,先听人话);
/// 两者皆无 = 回答不可解,归 Unknown 而不是瞎猜。
pub fn interpret(out: &EngineOutput) -> Probe {
    if out.stderr_tail.iter().any(|l| l.contains("未知子命令")) {
        return Probe::Unavailable;
    }
    if out.envelope.is_some() {
        return Probe::Available;
    }
    Probe::Unknown
}

/// 一次能力探测的证据:面板问的是哪个引擎、引擎那一跑自己怎么答的。
///
/// # 不变量
///
/// - **证据是引擎原文**:字段只从那一跑的回放(`EngineOutput`)与开机已探明的
///   引擎身份取,不猜测、不改写、不美化,也不回显任何本体(探针无参,证据
///   只可能碰到 argv / stderr 人话 / envelope schema / rc)。
/// - **引擎无话可说时不造句**:那一跑没到过真引擎(二进制缺失)、或既无
///   stderr 人话也无 envelope 时,如实写「引擎未给出可解回答」并附 rc ——
///   面板绝不替引擎回答,也不把面板自己的指路素材冒充成引擎原话。
/// - **随进程生命周期**:只在内存里跟着 [`Gate`] 走,不落盘、不进崩溃上报、
///   不做跨启动持久化 —— 重启重探即刷新(spec #90:不建本地能力版本表)。
///
/// 后面的贵表面(编目册册体 #122、设置屏 #123、四处单行指路 #124)取用同一
/// 份字段与同一句结论,不各自再采集一遍。
#[derive(Clone, Debug)]
pub struct ProbeEvidence {
    /// 探针 argv(无旗标,一次无害调用):证明面板问的是哪条命令面。
    pub argv: Vec<String>,
    /// 生效引擎 bin。二进制缺失时如实记「未发现」——证据带要能分辨
    /// 「问错二进制」,也要能分辨「根本没找到二进制」。
    pub bin: String,
    /// 引擎来源,口径同 `discover::Source::label()`(--nw-bin 指定 / 配置文件 /
    /// PATH / $NW_HOME/nw)。
    pub source: &'static str,
    /// 引擎版本:复用开机版本探测结论,不新增引擎调用;「探测中」/「未知」
    /// 两态如实呈现,不当成确定性版本号。
    pub version: String,
    /// 引擎那一跑的原文:stderr 首条非空人话优先(判定顺序:人话优先于
    /// envelope,与 [`interpret`] 同源);没有则 envelope 的 schema 原文
    /// (契约守卫命中也如实记 schema 原文);两者皆无 = 「引擎未给出可解回答」。
    pub words: String,
    /// 那一跑的退出码(数字口径同 `ExitCode::number()`,未登记码按 3 兜底
    /// 与引擎一致)。
    pub exit_code: i32,
}

/// 引擎没给出可解回答时的如实写法。附在证据里,rc 由同一份证据带出 ——
/// 面板不编一句引擎没说过的话。
const NO_ANSWER: &str = "引擎未给出可解回答";

impl ProbeEvidence {
    /// 从那一跑的回放与开机已探明的引擎身份立一份证据(纯函数)。
    ///
    /// `bin` 为 `None` = 发现失败、那一跑没到过真引擎:此刻引擎无话可说,
    /// 证据不拿 `MissingRunner` 的指路素材冒充原话。`version` 取开机的
    /// `--version` 回音(已探明,不在这里再问引擎一次)。
    pub fn capture(
        out: &EngineOutput,
        bin: Option<&str>,
        source: &'static str,
        version: &str,
    ) -> Self {
        // 没到过真引擎就没有「引擎原话」可言;到过了才按判定顺序取原文。
        let reached_engine = bin.is_some() || out.envelope.is_some();
        let words = if reached_engine {
            out.stderr_tail
                .iter()
                .map(|l| l.trim())
                .find(|l| !l.is_empty())
                .map(str::to_string)
                .or_else(|| out.envelope.as_ref().map(|e| e.schema.clone()))
                .unwrap_or_else(|| NO_ANSWER.to_string())
        } else {
            NO_ANSWER.to_string()
        };
        Self {
            argv: out.args.clone(),
            bin: bin.unwrap_or("未发现").to_string(),
            source,
            version: version.to_string(),
            words,
            exit_code: out.exit_code.number(),
        }
    }

    /// 探针 argv 的展示形态(`nw vps`),与 `EngineOutput::command_string`
    /// 同口径。
    pub fn probe_display(&self) -> String {
        format!("nw {}", self.argv.join(" "))
    }
}

/// 探测结果账:入口可用性的唯一事实来源(默认全 Unknown = 未决)。
///
/// 每个能力在 verdict 之外按能力存一份 [`ProbeEvidence`]:结论说「开不开」,
/// 证据说「凭什么」——呈现表面(日志 / 册体 / 设置屏)只读取,不重新采集。
#[derive(Default)]
pub struct Gate {
    results: HashMap<Capability, (Probe, Option<ProbeEvidence>)>,
}

impl Gate {
    /// 收账:verdict 与证据一并写入。`evidence` 为 `None` = 这一跑没有证据
    /// (非探针回音驱动的结论),读回时如实给 `None`,不补造。
    pub fn record(&mut self, cap: Capability, verdict: Probe, evidence: Option<ProbeEvidence>) {
        self.results.insert(cap, (verdict, evidence));
    }

    /// 某能力的探测结论;没探测过/失败 = Unknown。
    pub fn get(&self, cap: Capability) -> Probe {
        self.results
            .get(&cap)
            .map(|(v, _)| *v)
            .unwrap_or(Probe::Unknown)
    }

    /// 某能力的探测证据;未探测过/未收证据 = None。原样取回。
    pub fn evidence(&self, cap: Capability) -> Option<&ProbeEvidence> {
        self.results.get(&cap).and_then(|(_, ev)| ev.as_ref())
    }

    /// 入口门:只有引擎明确「有」才放行;未决与禁用一律不给。
    pub fn enabled(&self, cap: Capability) -> bool {
        self.get(cap) == Probe::Available
    }
}

/// 引擎发现事实:开机已探明,证据带只**读取**、不重新采集。
///
/// 能力账([`Gate`])只存探针那一跑;引擎身份三元组与「找 nw 试过哪」在开机
/// 发现结论里,这里原样引用——版本复用开机 `--version` 回音(已探明,不在这里
/// 再问引擎一次),`attempted` 是未决态唯一现成的指路素材(指路屏同源)。
pub struct EngineFacts<'a> {
    /// 生效引擎 bin(`App.engine.bin`);`None` = 发现失败,那一跑没到过真引擎。
    pub bin: Option<&'a str>,
    /// 引擎来源,口径同 `discover::Source::label()`(--nw-bin 指定 / 配置文件 /
    /// PATH / $NW_HOME/nw)。
    pub source: &'static str,
    /// 引擎版本:复用开机版本探测结论;「探测中」/「未知」两态如实呈现,
    /// **不当成确定性版本号**——版本号推不出能力结论。
    pub version: &'a str,
    /// 引擎发现试过的地方(`App.engine.attempted`):未决态「修引擎发现」的
    /// 指路素材,操作者按图索骥。
    pub attempted: &'a [String],
}

/// 能力门关门的规范指路:三态措辞 + 证据带。
///
/// # 不变量
///
/// - **三态互不冒充**:`Unavailable` 才出现 [`GATE_HINT`](那句「缺的是引擎
///   能力」的唯一出处就是那个常量,这里只引用、不复制字符串);`Unknown`
///   走未决话术 + 引擎发现素材 + 重启重探,绝不出现「入口禁用」「缺的是引擎
///   能力」「升级所配 nw」任何一样;`Available` 不产指路。
/// - **证据是引擎原文**:字段全部取自 [`Gate::evidence`] 的 [`ProbeEvidence`],
///   原样呈现、不猜测、不改写、不回显任何本体。**没收到证据时证据带为空**
///   ——不编造一份证据(未探测过 / 记账时没带证据)。
pub struct GateSignpost {
    /// 真相行:这一态开不开、这一态最短的下一步(1-2 行)。
    pub verdict: Vec<String>,
    /// 证据带:探的是哪个引擎、引擎那一跑怎么答的;空 = 没收到证据。
    pub evidence: Vec<String>,
}

impl GateSignpost {
    /// 单行形态:日志结论句。三态与证据压成一行,与册体多行形态共用同一份
    /// 证据字段——措辞一处真相(spec #90 故事 12:事后翻日志能复现当时的判断)。
    pub fn line(&self, cap: Capability, verdict: Probe) -> String {
        let mut s = format!("能力探测:{} {}", cap.label(), verdict.label());
        for e in &self.evidence {
            s.push_str(" · ");
            s.push_str(e);
        }
        s
    }

    /// 单行形态·**指路**(工单 #124 的收口):机器册反查未下发、配方烘焙 /
    /// 配方新建建议闸、外部源复核闸四处单行指路共用这一句。
    ///
    /// 与 [`Self::line`](日志结论句)、册体真相行(#122)同一处措辞真相:三态
    /// 标签取自 [`Probe::label`],下一步取自 [`next_steps`](与册体同一句),
    /// 证据带就是 [`Self::evidence`] 原字段——不另排一遍、不改写。
    ///
    /// 为什么不复用 [`Self::line`]:那句的主语是「能力探测」本身(日志结论
    /// 形态);指路要的是「⚠ 哪一项 什么态:这一步该做什么」,操作者照着做。
    /// 两者共用同一份证据字段,只头部形态不同。
    pub fn hint(&self, cap: Capability, verdict: Probe, facts: &EngineFacts) -> String {
        let mut s = format!(
            "{} {} {}:{}",
            verdict_mark(verdict),
            cap.label(),
            verdict.label(),
            next_steps(verdict, facts).concat()
        );
        for e in &self.evidence {
            s.push_str(" · ");
            s.push_str(e);
        }
        s
    }
}

/// 能力门关门的规范指路(spec #90 / 工单 #122 的**唯一新缝针**):册体(#122)、
/// 设置屏能力节(#123)、四处单行指路(#124)都从这里取「三态措辞 + 证据带」,
/// 不各自再排一遍。
///
/// 输入 = 册或入口标识 + 能力账 + 开机已探明的引擎发现事实;`Available`
/// 返回 `None`(可用不占指路行,健康时册体无噪音)。
///
/// `subject` 是册体传的册名(「机器」/「钥匙」);日志结论句只取
/// [`GateSignpost::line`],传入口名即可——真相行不归单行形态用。
pub fn gate_signpost(
    subject: &str,
    cap: Capability,
    gate: &Gate,
    facts: &EngineFacts,
) -> Option<GateSignpost> {
    let verdict = gate.get(cap);
    if verdict == Probe::Available {
        return None;
    }
    Some(GateSignpost {
        verdict: verdict_lines(subject, verdict, facts),
        evidence: evidence_band(cap, gate),
    })
}

/// 三态各自的真相行。`Unavailable` 的禁用话术唯一出处是 [`GATE_HINT`];
/// `Unknown` 只说「引擎这一跑没答上」,不碰禁用那三样字样。
///
/// # 为什么按句读断行、不按列硬切
///
/// 册体正文行宽预算只有 64 列(80×30 常畅态;66×50 紧凑态 62 列),而
/// 「⚠ 机器 册入口禁用:缺的是引擎能力,不是面板坏了;升级所配 nw 后入口自动开启。」
/// 整行 74 列——硬塞会被 [`crate::screens::recipes::clip`] 从中间吃掉,操作者
/// 正好看不到「自动开启。」那句最关键的下一步(spec #90 故事 18:常畅态 /
/// 紧凑态两档都完整可读、不截断)。所以断行点取在句读上,每行都短于预算。
///
/// 竖向预算红线因此仍然没破:册体关门时本来就不画条目,键位行照旧钉在
/// 内层末行(`catalog_key_row`),这几行远在 27 行预算内——多两行不等于
/// 几何失真,截断才是。
///
/// 每态 2-3 行:第一行说这一态开不开,后面补事实与下一步。证据带另两行,
/// 合起来仍落在既有关门行位置上。
fn verdict_lines(subject: &str, verdict: Probe, facts: &EngineFacts) -> Vec<String> {
    match verdict {
        // 真禁用:升级所配 nw(入口下次启动自动开启)+ 册体不呈现。
        // 中间一行整句引用 [`GATE_HINT`]——那句话的唯一出处就是那个常量,
        // 这里只引用、不复制字符串(同一句话两个出处正是本票要收掉的重复)。
        Probe::Unavailable => vec![
            format!("{} {subject} 册入口禁用", verdict_mark(verdict)),
            // 下一步与单行指路(#124)同一句:一处措辞真相。
            next_steps(verdict, facts).concat(),
            "编目册内容此时不可信也不呈现。".to_string(),
        ],
        // 未决:引擎这一跑没答上,不是缺能力。修引擎发现(attempted 素材)
        // 后重启重探——升级 nw 对未决未必管用,所以这里绝不出现那句禁用
        // 话术(spec #90 故事 2 / 22 / 23:两态互不冒充)。
        Probe::Unknown => {
            // 问的是哪个引擎(故事 3/4/5):与证据带同源,这里说「谁没答上」。
            let mut v = vec![format!(
                "{} {subject} 册入口未决:引擎这一跑没答上,册体不可信也不呈现。",
                verdict_mark(verdict)
            )];
            // 下一步(问的是哪个引擎 / 没找到二进制 / 引擎发现素材)与单行
            // 指路同一句——册体按行铺,单行形态拼成一行。
            v.extend(next_steps(verdict, facts));
            v
        }
        // 可用不产指路(`gate_signpost` 已在上面拦掉)。
        Probe::Available => Vec::new(),
    }
}

/// 三态各自的醒目标记:册体真相行(#122)与单行指路(#124)共用——
/// 禁用 = ⚠(要动手),未决 = ?(要先修发现),一眼可辨且不互冒充。
fn verdict_mark(verdict: Probe) -> &'static str {
    match verdict {
        Probe::Unavailable => "⚠",
        Probe::Unknown => "?",
        // 可用不产指路,标记用不上;给空串而不是 panic,调用方自己已经拦掉。
        Probe::Available => "",
    }
}

/// 这一态最短的下一步:册体真相行(#122)与单行指路(#124)共用的一句/一组。
///
/// 为什么单独抽出来:四处单行指路(反查未下发 / 烘焙 / 新建建议 / 外部源复核)
/// 与册体说的是同一件事,只密度不同——「升级所配 nw」只在真禁用态出现,
/// 未决只说「修引擎发现后重启重探」(spec #90 故事 22 / 23:两态互不冒充)。
/// 写成两份就是两处真相,迟早说岔。
///
/// 返回 `Vec` 是因为未决且没找到二进制时要多铺一行「找 nw 试过:…」
/// (引擎发现素材,故事 15):册体按行铺,单行形态直接concat 成一句。
fn next_steps(verdict: Probe, facts: &EngineFacts) -> Vec<String> {
    match verdict {
        // 真禁用:升级所配 nw,入口下次启动自动开启。整句引用 [`GATE_HINT`]
        // ——那句话的唯一出处就是那个常量,这里只引用、不复制字符串。
        Probe::Unavailable => vec![GATE_HINT.to_string()],
        // 未决:引擎这一跑没答上,不是缺能力——升级 nw 对未决未必管用,
        // 所以这里绝不出现那句禁用话术。
        Probe::Unknown => match facts.bin {
            // 问的是哪个引擎(故事 3/4/5):与证据带同源,这里说「谁没答上」。
            Some(b) => vec![format!(
                "问的是 {b} ({}, {});修引擎发现后重启重探。",
                facts.source, facts.version
            )],
            None => {
                // 没找到二进制:如实说没找到,不拿一个路径冒充(故事 3)。
                let mut v = vec!["没找到 nw 二进制;修引擎发现后重启重探。".to_string()];
                // 引擎发现素材(故事 15):把试过的地方摊开,操作者按图索骥。
                // 这一行可能长,超预算时由册体的 clip 收尾——素材本身
                // 宁可被截,也不为省一行把它并进上一句。
                if !facts.attempted.is_empty() {
                    v.push(format!("找 nw 试过:{}", facts.attempted.join("、")));
                }
                v
            }
        },
        // 可用不产指路。
        Probe::Available => Vec::new(),
    }
}

/// 证据带:全部取自 [`Gate::evidence`] 的 [`ProbeEvidence`],原样呈现。
/// 没收到证据 = 空带,不编造。
fn evidence_band(cap: Capability, gate: &Gate) -> Vec<String> {
    let Some(ev) = gate.evidence(cap) else {
        return Vec::new();
    };
    vec![
        format!(
            "引擎 {} ({}, {}) · 探针 {}",
            ev.bin,
            ev.source,
            ev.version,
            ev.probe_display()
        ),
        format!("引擎原话:{} (rc {})", ev.words, ev.exit_code),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{CommandRunner, FakeRunner, MissingRunner};

    async fn out_for(args: &[&str], rc: i32, stdout: &str, stderr: &[&str]) -> EngineOutput {
        let fake =
            FakeRunner::with_stderr(&[(args.join(" ").as_str(), rc, stdout, &stderr.join("\n"))]);
        let argv: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        fake.run(&argv).await
    }

    #[tokio::test]
    async fn 探针回_envelope_判可用() {
        // 所配 0.1.0 对 `nw source register`(无参)的真实回放:
        // rc 3 + nw/source-register.v1 envelope —— 子命令存在。
        let out = out_for(
            &["source", "register"],
            3,
            r#"{"schema":"nw/source-register.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"--kind \"\" 不认"}],"warnings":[],"data":{}}"#,
            &[],
        )
        .await;
        assert_eq!(interpret(&out), Probe::Available);
    }

    #[tokio::test]
    async fn 探针回未知子命令_stderr_判缺能力() {
        // 老引擎(没有 key 命令组)对 `nw key` 的回放形态:rc 3、stdout 空、
        // stderr 人话「未知子命令」—— 引擎没有这个能力,入口禁用。
        // (#63 删掉的「类别」探针正是这一形态:上游 `cli.go` 的 Run switch
        //  里没有 category 分支,走 default 回同一句话,所以那一行永远关着。)
        let out = out_for(&["key"], 3, "", &["nw: 未知子命令 \"key\""]).await;
        assert_eq!(interpret(&out), Probe::Unavailable);
    }

    #[tokio::test]
    async fn 引擎缺失或无回答_判未决_不冒充() {
        let out = MissingRunner {
            attempted: vec!["PATH".into()],
        }
        .run(&["vps".to_string()])
        .await;
        assert_eq!(interpret(&out), Probe::Unknown);
        // 无 envelope 也无「未知子命令」的空回答同样归未决
        let blank = out_for(&["vps"], 0, "", &[]).await;
        assert_eq!(interpret(&blank), Probe::Unknown);
    }

    #[tokio::test]
    async fn 组级探针无子命令即拒_回_envelope_判可用() {
        // 所配 0.1.0 对 `nw key`(无子命令)的真实回放:rc 3 +
        // nw/key-register.v1 envelope —— 命令组在,且不落文件、不读 stdin
        // (entry.RunGroup 只把分派结果收进内存缓冲再出口)。
        let out = out_for(
            &["key"],
            3,
            r#"{"schema":"nw/key-register.v1","nw":"0.1.0","ok":false,"errors":[{"code":"INPUT_INVALID","exitCode":3,"message":"key 下需要子命令 register / ls / get / rm"}],"warnings":[],"data":{}}"#,
            &[],
        )
        .await;
        assert_eq!(interpret(&out), Probe::Available);
        // 三条组级探针各问各的,互不复用结论
        for cap in [Capability::Key, Capability::Subscription, Capability::Tag] {
            assert_eq!(cap.probe_args().len(), 1, "{cap:?} 是组级探针");
            assert_eq!(
                Capability::from_probe(&cap.probe_args()),
                Some(cap),
                "{cap:?} 探针认得出自己"
            );
        }
    }

    #[tokio::test]
    async fn 组内未知子命令_不误判成缺能力() {
        // 上游各组自己的未知子命令话术是「未知 key 子命令 %q」,**不含**
        // 「未知子命令」这个连续子串 —— 「组在、子命令拼错」不会被 interpret()
        // 误判成缺能力(判据不用为组级探针放宽)。
        let out = out_for(
            &["key"],
            3,
            r#"{"schema":"nw/key-register.v1","nw":"0.1.0","ok":false,"errors":[],"warnings":[],"data":{}}"#,
            &["nw key: 未知 key 子命令 \"regsiter\""],
        )
        .await;
        assert_eq!(interpret(&out), Probe::Available);
    }

    #[test]
    fn 未探测默认未决_只有明确可用才开门() {
        let gate = Gate::default();
        assert_eq!(gate.get(Capability::Tag), Probe::Unknown);
        assert!(gate.evidence(Capability::Tag).is_none(), "未探测过没有证据");
        assert!(!gate.enabled(Capability::Tag), "未决不放行");
        let mut g = Gate::default();
        g.record(Capability::Tag, Probe::Unavailable, None);
        assert!(!g.enabled(Capability::Tag), "禁用不放行");
        assert!(g.evidence(Capability::Tag).is_none(), "没收证据就是 None");
        g.record(Capability::SourceRegister, Probe::Available, None);
        assert!(g.enabled(Capability::SourceRegister), "明确可用才放行");
    }

    #[tokio::test]
    async fn 探针回未知子命令_禁用且证据含引擎人话原文() {
        // 「问错二进制」的实证形态:同机另一个自称 0.1.0 的构建对 `nw vps`
        // 回「未知子命令」。判禁用的同时,证据要留住引擎那一跑的原话。
        let out = out_for(&["vps"], 3, "", &["nw: 未知子命令 \"vps\""]).await;
        let ev = ProbeEvidence::capture(&out, Some("/tmp/nw"), "PATH", "0.1.0");
        let mut gate = Gate::default();
        gate.record(Capability::Vps, interpret(&out), Some(ev));
        assert_eq!(gate.get(Capability::Vps), Probe::Unavailable);
        assert!(!gate.enabled(Capability::Vps), "禁用不放行");
        let ev = gate.evidence(Capability::Vps).expect("禁用也要收证据");
        assert_eq!(ev.argv, vec!["vps".to_string()], "探针 argv 原样入账");
        assert_eq!(ev.bin, "/tmp/nw");
        assert_eq!(ev.source, "PATH");
        assert_eq!(ev.version, "0.1.0");
        assert_eq!(ev.words, "nw: 未知子命令 \"vps\"", "引擎原话不改写");
        assert_eq!(ev.exit_code, 3);
    }

    #[tokio::test]
    async fn 探针回_envelope_可用且证据记_schema_原文() {
        // 可用态同样收证据:册体证据带(#122)与日志都要能说出「引擎答的是
        // 哪句 schema」——只有原文可信,版本号推不出能力结论。
        let out = out_for(
            &["vps"],
            3,
            r#"{"schema":"nw/vps-register.v1","nw":"0.1.0","ok":false,"errors":[],"warnings":[],"data":{}}"#,
            &[],
        )
        .await;
        let ev = ProbeEvidence::capture(&out, Some("/usr/local/bin/nw"), "PATH", "0.1.0");
        let mut gate = Gate::default();
        gate.record(Capability::Vps, interpret(&out), Some(ev));
        assert_eq!(gate.get(Capability::Vps), Probe::Available);
        assert!(gate.enabled(Capability::Vps), "明确可用才放行");
        let ev = gate.evidence(Capability::Vps).expect("可用也收证据");
        assert_eq!(ev.words, "nw/vps-register.v1", "证据记 schema 原文");
        assert_eq!(ev.exit_code, 3);
    }

    #[tokio::test]
    async fn 引擎无话可说_未决且证据如实记未给出可解回答_不编造() {
        // 三种「引擎没答上」:二进制缺失(那一跑没到过真引擎)、空回、
        // stdout 不可解。一律未决,证据一律如实写「引擎未给出可解回答」
        // 并附 rc——面板不替引擎造句。
        let missing = MissingRunner {
            attempted: vec!["PATH".into()],
        }
        .run(&["vps".to_string()])
        .await;
        assert_eq!(interpret(&missing), Probe::Unknown);
        let ev = ProbeEvidence::capture(&missing, None, "未发现", "未知");
        assert_eq!(ev.words, "引擎未给出可解回答");
        assert_eq!(ev.exit_code, 2);
        assert_eq!(ev.bin, "未发现", "没找到二进制也如实记");
        let blank = out_for(&["vps"], 0, "", &[]).await;
        let ev = ProbeEvidence::capture(&blank, Some("/tmp/nw"), "PATH", "0.1.0");
        assert_eq!(interpret(&blank), Probe::Unknown);
        assert_eq!(ev.words, "引擎未给出可解回答");
        assert_eq!(ev.exit_code, 0);
        let unsolvable = out_for(&["vps"], 3, "这不是 envelope", &[]).await;
        let ev = ProbeEvidence::capture(&unsolvable, Some("/tmp/nw"), "PATH", "0.1.0");
        assert_eq!(interpret(&unsolvable), Probe::Unknown);
        assert_eq!(ev.words, "引擎未给出可解回答");
        assert_eq!(ev.exit_code, 3);
    }

    #[test]
    fn 探针_argv_只认精确匹配() {
        // 精确探针:命中
        let probe: Vec<String> = vec!["source".into(), "register".into()];
        assert_eq!(
            Capability::from_probe(&probe),
            Some(Capability::SourceRegister)
        );
        // 真实登记(带 --repo/--kind/--yes)不是探针,不误吞 #30 的结果
        let real: Vec<String> = vec![
            "source".into(),
            "register".into(),
            "--repo".into(),
            "o/r".into(),
            "--kind".into(),
            "skill".into(),
            "--yes".into(),
        ];
        assert_eq!(Capability::from_probe(&real), None);
        // 别的能力的探针也不互认
        let tag: Vec<String> = vec!["tag".into()];
        assert_eq!(Capability::from_probe(&tag), Some(Capability::Tag));
        // 组级探针不带子命令:真实的 `nw tag add <标签>` 不是探针
        let real_tag: Vec<String> = vec!["tag".into(), "add".into(), "pi".into()];
        assert_eq!(Capability::from_probe(&real_tag), None);
    }

    #[test]
    fn recipe_new_探针无参即在位置参数处被拒_无害() {
        // 所配 0.1.0 对 `nw recipe new`(无参)的真实回放:rc 3 +
        // nw/recipe-new.v1 envelope —— 子命令存在,且不写任何文件。
        assert_eq!(Capability::RecipeNew.probe_args(), vec!["recipe", "new"]);
        assert_eq!(
            Capability::from_probe(&Capability::RecipeNew.probe_args()),
            Some(Capability::RecipeNew),
            "探针认得出自己"
        );
        // 真实的 `nw recipe new <名>` 不是探针,不误吞建议命令的结论
        let real: Vec<String> = vec!["recipe".into(), "new".into(), "desk".into()];
        assert_eq!(Capability::from_probe(&real), None);
        // 与烘焙同组不同项:两个探针各问各的
        assert_ne!(
            Capability::RecipeNew.probe_args(),
            Capability::RecipeBake.probe_args()
        );
    }

    /// 按一条证据立一份规范指路(测试专用):把证据记进一个一次性账,再走与
    /// 生产同一个入口 [`gate_signpost`]——断言的是操作者可见的那行。
    /// `line()` / `hint()` 只取「入口 + 三态 + (下一步 + )证据带」,不碰册体
    /// 专属的真相行措辞,所以引擎发现事实给空值即可(真相行另有测试盯着)。
    fn signpost_of(ev: &ProbeEvidence, cap: Capability, verdict: Probe) -> GateSignpost {
        let mut gate = Gate::default();
        gate.record(cap, verdict, Some(ev.clone()));
        gate_signpost(
            cap.label(),
            cap,
            &gate,
            &EngineFacts {
                bin: None,
                source: "测试注入",
                version: "0.1.0",
                attempted: &[],
            },
        )
        .expect("非可用态必有指路")
    }

    #[tokio::test]
    async fn 结论句_禁用态与未决态各带齐证据字段_互不冒充() {
        // 这一行同时是日志结论句与册体证据带(#122 起共用同一措辞与字段),
        // 所以禁用态与未决态都要带齐证据,且各说各话、不互相冒充。
        let bad = out_for(&["vps"], 3, "", &["nw: 未知子命令 \"vps\""]).await;
        let ev = ProbeEvidence::capture(&bad, Some("/tmp/nw"), "PATH", "0.1.0");
        let line = signpost_of(&ev, Capability::Vps, Probe::Unavailable)
            .line(Capability::Vps, Probe::Unavailable);
        assert!(line.contains("能力探测:vps 入口禁用"), "{line}");
        assert!(line.contains("/tmp/nw"), "bin 在场:{line}");
        assert!(line.contains("(PATH, 0.1.0)"), "来源与版本在场:{line}");
        assert!(line.contains("探针 nw vps"), "探针 argv 在场:{line}");
        assert!(
            line.contains("引擎原话:nw: 未知子命令 \"vps\" (rc 3)"),
            "引擎原话与 rc 在场:{line}"
        );
        // 未决:同构字段一个不少,但不出现「入口禁用」字样
        let missing = MissingRunner {
            attempted: vec!["PATH".into()],
        }
        .run(&["vps".to_string()])
        .await;
        let ev = ProbeEvidence::capture(&missing, None, "未发现", "未知");
        let line =
            signpost_of(&ev, Capability::Vps, Probe::Unknown).line(Capability::Vps, Probe::Unknown);
        assert!(line.contains("能力探测:vps 未决"), "{line}");
        assert!(!line.contains("入口禁用"), "未决不冒充禁用:{line}");
        assert!(line.contains("探针 nw vps"), "未决也带探针:{line}");
        assert!(
            line.contains("引擎原话:引擎未给出可解回答 (rc 2)"),
            "没答上就如实说没答上:{line}"
        );
    }

    #[tokio::test]
    async fn 同版本号双构建_证据分辨问错二进制() {
        // 立项实证:两个都自称 0.1.0 的构建,`nw vps` 一个回 envelope、一个回
        // 「未知子命令」。版本号推不出能力结论,证据要能分辨问错了二进制。
        let good = out_for(
            &["vps"],
            3,
            r#"{"schema":"nw/vps-register.v1","nw":"0.1.0","ok":false,"errors":[],"warnings":[],"data":{}}"#,
            &[],
        )
        .await;
        let bad = out_for(&["vps"], 3, "", &["nw: 未知子命令 \"vps\""]).await;
        let mut gate = Gate::default();
        gate.record(
            Capability::Vps,
            interpret(&good),
            Some(ProbeEvidence::capture(
                &good,
                Some("/usr/local/bin/nw"),
                "PATH",
                "0.1.0",
            )),
        );
        assert!(gate.enabled(Capability::Vps), "回 envelope 判可用");
        // 换成指着 /tmp/nw 再探一次:同一能力重新收账,证据随之刷新
        // (证据随进程生命周期,不跨启动缓存)。
        gate.record(
            Capability::Vps,
            interpret(&bad),
            Some(ProbeEvidence::capture(
                &bad,
                Some("/tmp/nw"),
                "PATH",
                "0.1.0",
            )),
        );
        assert!(!gate.enabled(Capability::Vps), "回未知子命令判禁用");
        let ev = gate.evidence(Capability::Vps).expect("禁用也要收证据");
        assert_eq!(ev.version, "0.1.0", "两个构建同版本号");
        assert_eq!(ev.bin, "/tmp/nw", "bin 可分辨");
        assert_eq!(ev.words, "nw: 未知子命令 \"vps\"", "引擎原话可分辨");
        let line = signpost_of(ev, Capability::Vps, Probe::Unavailable)
            .line(Capability::Vps, Probe::Unavailable);
        assert!(line.contains("/tmp/nw"), "结论句点名生效二进制:{line}");
        assert!(
            line.contains("0.1.0"),
            "版本号照实呈现,但不作为能力依据:{line}"
        );
    }

    #[tokio::test]
    async fn 证据只取引擎原话与_schema_不回显本体() {
        // 探针无参(argv 面无任何载荷形参),证据只可能碰到 argv / stderr 人话 /
        // envelope schema / rc;envelope 的 data(可能含本体)一个字节都不进
        // 证据带(ADR-0009:永不回显本体)。
        let out = out_for(
            &["key"],
            3,
            r#"{"schema":"nw/key-register.v1","nw":"0.1.0","ok":false,"errors":[],"warnings":[],"data":{"key":"sk-live-绝密本体"}}"#,
            &[],
        )
        .await;
        let ev = ProbeEvidence::capture(&out, Some("/tmp/nw"), "PATH", "0.1.0");
        // 可用态不产指路(`gate_signpost` 返回 None),所以这一条直接钉证据
        // 载体本身:证据只可能碰到 argv / stderr 人话 / envelope schema / rc。
        assert_eq!(ev.words, "nw/key-register.v1", "只取 schema 原文:{ev:?}");
        assert!(!ev.words.contains("sk-live"), "本体不进证据:{ev:?}");
        assert!(
            !ev.words.contains("绝密"),
            "本体字样一个字节都不出现:{ev:?}"
        );
        assert!(
            !format!("{ev:?}").contains("sk-live"),
            "Debug 输出同样不回显本体:{ev:?}"
        );
        assert!(
            ev.argv.iter().all(|a| !a.starts_with('-')),
            "探针 argv 无旗标:{:?}",
            ev.argv
        );
    }

    #[test]
    fn 入口清单八项_顺序即设置屏显示顺序() {
        // #63:入口清单由面板定义、可用性由引擎决定。清单形态钉死:去掉指向上游
        // 不存在子命令的「类别」,补 key / 订阅 / 标签 三条组级探针,共八项。
        let labels: Vec<&str> = Capability::ALL.into_iter().map(|c| c.label()).collect();
        assert_eq!(
            labels,
            vec![
                "外部源登记",
                "外部源复核",
                "配方烘焙",
                "配方新建",
                "vps",
                "key",
                "订阅",
                "标签",
            ]
        );
        // 每一项的探针都指着上游真实存在的命令面,且两两不重复
        let mut probes: Vec<Vec<String>> = Capability::ALL
            .into_iter()
            .map(|c| c.probe_args())
            .collect();
        for p in &probes {
            assert!(!p.is_empty(), "探针不许空");
            assert!(
                p.iter().all(|a| !a.starts_with('-')),
                "探针只给命令面,不带旗标:{p:?}"
            );
        }
        probes.sort();
        probes.dedup();
        assert_eq!(probes.len(), Capability::ALL.len(), "探针两两不重复");
    }

    #[test]
    fn 映射表只到探什么命令为止() {
        // 无害性:探针不携带会动文件的旗标(--yes/--repo/--root 全无)
        for cap in Capability::ALL {
            let args = cap.probe_args();
            assert!(
                !args.iter().any(|a| a == "--yes" || a == "--root"),
                "{cap:?} 探针必须无害"
            );
            assert!(!args.is_empty());
        }
        assert_eq!(
            Capability::SourceRecheck.probe_args(),
            vec!["source", "recheck"]
        );
    }

    // ---- 共享纯行构造器(#122 的唯一新缝针):三态措辞 + 证据带 ----

    /// 未决态的事实:引擎缺失(那一跑没到过真引擎),attempted 是指路素材。
    fn facts_missing<'a>(attempted: &'a [String]) -> EngineFacts<'a> {
        EngineFacts {
            bin: None,
            source: "未发现",
            version: "未知",
            attempted,
        }
    }

    /// 三态判据只认 Available 才放行,所以可用态构造器直接不产指路——
    /// 健康时册体一个字节都不多(规格故事 28:可用入口无噪音)。
    #[test]
    fn 构造器_可用态不产指路() {
        let mut gate = Gate::default();
        gate.record(Capability::Vps, Probe::Available, None);
        let facts = EngineFacts {
            bin: Some("/usr/local/bin/nw"),
            source: "PATH",
            version: "0.1.0",
            attempted: &[],
        };
        assert!(
            gate_signpost("机器", Capability::Vps, &gate, &facts).is_none(),
            "可用态不产指路"
        );
    }

    #[tokio::test]
    async fn 构造器_禁用态_升级指路与证据带四要素都在() {
        // 真禁用(引擎没有 vps 命令组):禁用话术 + 「不可信也不呈现」 +
        // 证据带(引擎 bin/来源/版本 · 探针 argv · 引擎原话 + rc)。
        let out = out_for(&["vps"], 3, "", &["nw: 未知子命令 \"vps\""]).await;
        let ev = ProbeEvidence::capture(&out, Some("/tmp/nw"), "PATH", "0.1.0");
        let mut gate = Gate::default();
        gate.record(Capability::Vps, interpret(&out), Some(ev));
        let facts = EngineFacts {
            bin: Some("/tmp/nw"),
            source: "PATH",
            version: "0.1.0",
            attempted: &[],
        };
        let s = gate_signpost("机器", Capability::Vps, &gate, &facts).expect("禁用要产指路");
        let all = s.verdict.join("\n");
        assert!(all.contains("入口禁用"), "禁用话术在场:{all}");
        assert!(
            all.contains(GATE_HINT),
            "禁用话术唯一出处是 GATE_HINT 常量:{all}"
        );
        assert!(all.contains("不可信也不呈现"), "册体不呈现:{all}");
        assert!(!all.contains("重启重探"), "禁用态不给未决话术:{all}");
        // 证据带四要素
        let evl = s.evidence.join("\n");
        assert!(evl.contains("/tmp/nw"), "bin 在场:{evl}");
        assert!(evl.contains("(PATH, 0.1.0)"), "来源与版本在场:{evl}");
        assert!(evl.contains("探针 nw vps"), "探针 argv 在场:{evl}");
        assert!(
            evl.contains("引擎原话:nw: 未知子命令 \"vps\" (rc 3)"),
            "引擎原话与 rc 在场:{evl}"
        );
        // 单行形态(日志结论句):三态 + 同一份证据
        let line = s.line(Capability::Vps, Probe::Unavailable);
        assert!(line.contains("能力探测:vps 入口禁用"), "{line}");
        assert!(
            line.contains("引擎原话:nw: 未知子命令 \"vps\" (rc 3)"),
            "{line}"
        );
    }

    #[tokio::test]
    async fn 构造器_未决态_不冒充禁用_带attempted素材与重启重探() {
        // 未决(引擎缺失):绝不出「入口禁用」「缺的是引擎能力」「升级所配 nw」
        // 任何一样;改走未决话术 + 引擎发现素材 + 重启重探,证据带同构在场。
        let out = MissingRunner {
            attempted: vec!["PATH".into()],
        }
        .run(&["vps".to_string()])
        .await;
        let ev = ProbeEvidence::capture(&out, None, "未发现", "未知");
        let mut gate = Gate::default();
        gate.record(Capability::Vps, interpret(&out), Some(ev));
        let attempted = vec!["PATH".to_string(), "$NW_HOME/nw".to_string()];
        let facts = facts_missing(&attempted);
        let s = gate_signpost("机器", Capability::Vps, &gate, &facts).expect("未决要产指路");
        let all = s.verdict.join("\n");
        assert!(all.contains("入口未决"), "未决话术在场:{all}");
        assert!(!all.contains("入口禁用"), "未决不冒充禁用:{all}");
        assert!(!all.contains("缺的是引擎能力"), "未决不冒充缺能力:{all}");
        assert!(!all.contains("升级所配 nw"), "未决不给误导性下一步:{all}");
        assert!(all.contains("重启重探"), "重启重探在场:{all}");
        assert!(
            all.contains("PATH、$NW_HOME/nw"),
            "attempted 素材在场:{all}"
        );
        assert!(all.contains("不可信也不呈现"), "册体同样不呈现:{all}");
        // 证据带同构(字段一个不少)
        let evl = s.evidence.join("\n");
        assert!(evl.contains("探针 nw vps"), "未决也带探针:{evl}");
        assert!(
            evl.contains("引擎原话:引擎未给出可解回答 (rc 2)"),
            "没答上就如实说没答上:{evl}"
        );
        let line = s.line(Capability::Vps, Probe::Unknown);
        assert!(line.contains("能力探测:vps 未决"), "{line}");
        assert!(!line.contains("入口禁用"), "单行形态也不冒充:{line}");
    }

    /// 单行形态·**指路**(工单 #124):四处单行指路(机器册反查未下发 / 烘焙 /
    /// 新建建议 / 外部源复核)共用这一句——与册体真相行同一处措辞真相
    /// (三态标签 + 下一步),证据带同一份字段,只密度不同。
    #[tokio::test]
    async fn 单行指路_禁用态给升级下一步_未决态只给重启重探_互不冒充() {
        // 真禁用:下一步是升级所配 nw(整句引用 GATE_HINT),四要素证据在场。
        let bad = out_for(&["vps"], 3, "", &["nw: 未知子命令 \"vps\""]).await;
        let ev = ProbeEvidence::capture(&bad, Some("/tmp/nw"), "PATH", "0.1.0");
        let mut gate = Gate::default();
        gate.record(Capability::Vps, interpret(&bad), Some(ev));
        let facts = EngineFacts {
            bin: Some("/tmp/nw"),
            source: "PATH",
            version: "0.1.0",
            attempted: &[],
        };
        let s = gate_signpost("机器", Capability::Vps, &gate, &facts).expect("禁用要产指路");
        let hint = s.hint(Capability::Vps, Probe::Unavailable, &facts);
        assert!(hint.starts_with("⚠ vps 入口禁用:"), "禁用态头部形态:{hint}");
        assert!(hint.contains(GATE_HINT), "下一步整句引用 GATE_HINT:{hint}");
        assert!(!hint.contains("重启重探"), "禁用态不给未决话术:{hint}");
        assert!(hint.contains("/tmp/nw"), "bin 在场:{hint}");
        assert!(hint.contains("(PATH, 0.1.0)"), "来源与版本在场:{hint}");
        assert!(hint.contains("探针 nw vps"), "探针 argv 在场:{hint}");
        assert!(
            hint.contains("引擎原话:nw: 未知子命令 \"vps\" (rc 3)"),
            "引擎原话与 rc 在场:{hint}"
        );
        // 未决:换成「修引擎发现后重启重探」,那三样字样一个都不出现。
        let missing = MissingRunner {
            attempted: vec!["PATH".into()],
        }
        .run(&["vps".to_string()])
        .await;
        let ev = ProbeEvidence::capture(&missing, None, "未发现", "未知");
        let mut gate = Gate::default();
        gate.record(Capability::Vps, interpret(&missing), Some(ev));
        let attempted = ["PATH".to_string(), "$NW_HOME/nw".to_string()];
        let facts = facts_missing(&attempted);
        let s = gate_signpost("机器", Capability::Vps, &gate, &facts).expect("未决要产指路");
        let hint = s.hint(Capability::Vps, Probe::Unknown, &facts);
        assert!(hint.starts_with("? vps 未决:"), "未决态头部形态:{hint}");
        assert!(hint.contains("修引擎发现后重启重探"), "未决的下一步:{hint}");
        assert!(
            hint.contains("找 nw 试过:PATH、$NW_HOME/nw"),
            "引擎发现素材并入同一行:{hint}"
        );
        assert!(!hint.contains("入口禁用"), "未决不冒充禁用:{hint}");
        assert!(!hint.contains("缺的是引擎能力"), "未决不冒充缺能力:{hint}");
        assert!(!hint.contains("升级所配 nw"), "未决不给误导性下一步:{hint}");
        assert!(hint.contains("探针 nw vps"), "未决也带探针:{hint}");
        assert!(
            hint.contains("引擎原话:引擎未给出可解回答 (rc 2)"),
            "没答上就如实说没答上:{hint}"
        );
    }

    /// 没收到证据时(测试里 `record(cap, verdict, None)` / 未探测过):
    /// 真相行照常给,证据带**不出现**——不编造一份证据。
    #[test]
    fn 构造器_没收证据_真相行照常给_证据带不出现() {
        let mut gate = Gate::default();
        gate.record(Capability::Vps, Probe::Unavailable, None);
        let facts = EngineFacts {
            bin: Some("/tmp/nw"),
            source: "PATH",
            version: "0.1.0",
            attempted: &[],
        };
        let s = gate_signpost("机器", Capability::Vps, &gate, &facts).expect("禁用要产指路");
        assert!(
            s.verdict.join("\n").contains("入口禁用"),
            "真相行照常:{:?}",
            s.verdict
        );
        assert!(s.evidence.is_empty(), "没证据就不编造:{:?}", s.evidence);
        // 未探测过同理
        let empty = Gate::default();
        let s2 = gate_signpost("机器", Capability::Vps, &empty, &facts).expect("未探测也要产指路");
        assert!(s2.verdict.join("\n").contains("入口未决"), "未探测归未决");
        assert!(s2.evidence.is_empty(), "未探测没有证据");
    }

    #[tokio::test]
    async fn 构造器_同版本号双构建_证据可分辨问错二进制与引擎旧() {
        // 立项实证:两个都自称 0.1.0 的构建,`nw vps` 一个回 envelope、一个回
        // 「未知子命令」。版本号推不出能力结论,证据要能分辨问错了二进制。
        let good = out_for(
            &["vps"],
            3,
            r#"{"schema":"nw/vps-register.v1","nw":"0.1.0","ok":false,"errors":[],"warnings":[],"data":{}}"#,
            &[],
        )
        .await;
        let bad = out_for(&["vps"], 3, "", &["nw: 未知子命令 \"vps\""]).await;
        let facts_good = EngineFacts {
            bin: Some("/usr/local/bin/nw"),
            source: "PATH",
            version: "0.1.0",
            attempted: &[],
        };
        let mut good_gate = Gate::default();
        good_gate.record(
            Capability::Vps,
            interpret(&good),
            Some(ProbeEvidence::capture(
                &good,
                Some("/usr/local/bin/nw"),
                "PATH",
                "0.1.0",
            )),
        );
        assert!(
            gate_signpost("机器", Capability::Vps, &good_gate, &facts_good).is_none(),
            "回 envelope 判可用,不产指路"
        );
        let mut bad_gate = Gate::default();
        bad_gate.record(
            Capability::Vps,
            interpret(&bad),
            Some(ProbeEvidence::capture(
                &bad,
                Some("/tmp/nw"),
                "PATH",
                "0.1.0",
            )),
        );
        let facts_bad = EngineFacts {
            bin: Some("/tmp/nw"),
            source: "PATH",
            version: "0.1.0",
            attempted: &[],
        };
        let s =
            gate_signpost("机器", Capability::Vps, &bad_gate, &facts_bad).expect("禁用要产指路");
        let evl = s.evidence.join("\n");
        assert!(evl.contains("/tmp/nw"), "问错二进制可分辨:{evl}");
        assert!(
            evl.contains("引擎原话:nw: 未知子命令 \"vps\""),
            "引擎旧/没这命令组可分辨:{evl}"
        );
        assert!(evl.contains("0.1.0"), "同版本号照实呈现但不作依据:{evl}");
    }
}
