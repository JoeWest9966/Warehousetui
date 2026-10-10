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

/// 探测结果账:入口可用性的唯一事实来源(默认全 Unknown = 未决)。
#[derive(Default)]
pub struct Gate {
    results: HashMap<Capability, Probe>,
}

impl Gate {
    pub fn record(&mut self, cap: Capability, verdict: Probe) {
        self.results.insert(cap, verdict);
    }

    /// 某能力的探测结论;没探测过/失败 = Unknown。
    pub fn get(&self, cap: Capability) -> Probe {
        self.results.get(&cap).copied().unwrap_or(Probe::Unknown)
    }

    /// 入口门:只有引擎明确「有」才放行;未决与禁用一律不给。
    pub fn enabled(&self, cap: Capability) -> bool {
        self.get(cap) == Probe::Available
    }
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
        assert!(!gate.enabled(Capability::Tag), "未决不放行");
        let mut g = Gate::default();
        g.record(Capability::Tag, Probe::Unavailable);
        assert!(!g.enabled(Capability::Tag), "禁用不放行");
        g.record(Capability::SourceRegister, Probe::Available);
        assert!(g.enabled(Capability::SourceRegister), "明确可用才放行");
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
}
