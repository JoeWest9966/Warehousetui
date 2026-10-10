//! 能力探测门(spec #24 / 工单 #29):面板问引擎「你有什么」,缺的能力
//! 入口禁用并指路,面板绝不自己造替代品、也绝不替引擎回答。
//!
//! 探测铁律(与 ADR-0007「跟随引擎」同源):
//! - **一次无害调用 + 读引擎自己的回答**:回 envelope = 子命令存在;
//!   stderr 人话「未知子命令」= 不存在;解析不出结论(引擎缺失/无响应)
//!   = 探测失败。四条探针实测于所配 `nw 0.1.0` 皆无副作用(只读,
//!   不落任何文件)。
//! - **不建本地能力常量表**:下面的映射只记「面板入口 → 探什么命令」,
//!   答案永远来自引擎当下这一跑,不记某版本有没有——上游一发版面板
//!   自动跟上。
//! - 探测失败走既有指路屏语义(见 [`GateHint`]),不瘫痪、不冒充。

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
    /// 类别(上游 master 有、所配 0.1.0 无):`nw category`。
    Category,
    /// vps(上游 master 有、所配 0.1.0 无):`nw vps`。
    Vps,
}

impl Capability {
    /// 面板入口清单(设置屏按此渲染,顺序即显示顺序)。
    pub const ALL: [Capability; 6] = [
        Capability::SourceRegister,
        Capability::SourceRecheck,
        Capability::RecipeBake,
        Capability::RecipeNew,
        Capability::Category,
        Capability::Vps,
    ];

    /// 入口名(操作者视角,禁用与指路文案用)。
    pub fn label(self) -> &'static str {
        match self {
            Capability::SourceRegister => "外部源登记",
            Capability::SourceRecheck => "外部源复核",
            Capability::RecipeBake => "配方烘焙",
            Capability::RecipeNew => "配方新建",
            Capability::Category => "类别",
            Capability::Vps => "vps",
        }
    }

    /// 探针 argv:一次无害调用(不带 --yes / --repo 等会动文件的旗标)。
    /// 实测六条探针在 0.1.0 上皆为纯读:register 无参在旗标校验处即拒,
    /// recheck 只读编目,recipe bake 无参在「恰好要一个配方」处即拒,
    /// recipe new 无参在「位置参数必须恰好一个」处即拒,
    /// 未知子命令直接回 usage。
    pub fn probe_args(self) -> Vec<String> {
        match self {
            Capability::SourceRegister => vec!["source".into(), "register".into()],
            Capability::SourceRecheck => vec!["source".into(), "recheck".into()],
            Capability::RecipeBake => vec!["recipe".into(), "bake".into()],
            Capability::RecipeNew => vec!["recipe".into(), "new".into()],
            Capability::Category => vec!["category".into()],
            Capability::Vps => vec!["vps".into()],
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
        // 所配 0.1.0 对 `nw category` 的真实回放:rc 3、stdout 空、
        // stderr 人话「未知子命令」—— 引擎没有这个能力。
        let out = out_for(&["category"], 3, "", &["nw: 未知子命令 \"category\""]).await;
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

    #[test]
    fn 未探测默认未决_只有明确可用才开门() {
        let gate = Gate::default();
        assert_eq!(gate.get(Capability::Category), Probe::Unknown);
        assert!(!gate.enabled(Capability::Category), "未决不放行");
        let mut g = Gate::default();
        g.record(Capability::Category, Probe::Unavailable);
        assert!(!g.enabled(Capability::Category), "禁用不放行");
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
        let cat: Vec<String> = vec!["category".into()];
        assert_eq!(Capability::from_probe(&cat), Some(Capability::Category));
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
