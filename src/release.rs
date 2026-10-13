//! 引擎版本对照的**数据面**(spec #94 / 工单 #108):本地 `nw` 版本 ↔ 引擎
//! release 仓最新 tag。
//!
//! 分工:`engine` 层是面板与 `nw` 进程之间的唯一缝(只跑 `nw`),本模块是
//! 面板与「引擎 release 仓」之间的唯一缝——只问一件事「最新 tag 是什么」,
//! 读答案、不写任何东西、不代跑任何升级动作(ADR-0003:建议命令只给文案)。
//!
//! spec 的实施决策把「引擎版本对照的数据面」划给 `datarepo` 接缝(工单 #101
//! 在建);本模块在 #101 落地前先自立,接缝形状(一个可注入的查询 + 一个纯
//! 判词函数)与 datarepo 要对齐,届时由 #101 收编。

use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 引擎 release 仓(`owner/repo`)。上游依据:`docs/research/nw-journeys-and-docs.md`
/// 第 14 行登记的本仓引擎仓,`docs/research/nw-cli-surface.md` 的调研对象
/// `/opt/Nous-Warehouse` 即它的工作副本。硬编码这一处坐标,与 `engine/commands.rs`
/// 硬编码上游 CLI 形状同属「跟随引擎」纪律的事实登记,不是面板自创的偏好。
pub const ENGINE_RELEASE_REPO: &str = "JoeWest9966/Nous-Warehouse";

/// 一次远端查询的结论。三态互不冒充:
/// - [`ReleaseProbe::Latest`]:拿到了 tag(可能不是 semver,原样上交);
/// - [`ReleaseProbe::Garbage`]:查询走得通、回话不是可用 tag(空 stdout / 无 tag 字段);
/// - [`ReleaseProbe::Unreachable`]:查询根本发不出去(`gh` 不在 PATH / 非零 rc / 没网)。
///
/// 后两态都通向同一句呈现——「本地版本 + 取不到远端」(spec 用户故事 25:
/// 离线机器上不报错、只少一块信息),但**理由分开说**:操作者要知道是该去装
/// `gh` / 联网,还是上游这次的发布形态不对劲。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReleaseProbe {
    Latest(String),
    Garbage,
    Unreachable,
}

/// 本地 ↔ 远端的对照判词。四态,**面板绝不无证据地说「已是最新」**:
/// 只有两边都解析成版本号且相等才判 [`Verdict::Same`]。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// 两边版本号相等。
    Same,
    /// 引擎 release 仓有比本地新的 tag。
    Behind,
    /// 本地比 release 仓最新 tag 还新(自编 / 预发布,不是问题)。
    Ahead,
    /// 至少一边不是可比的版本号(带前缀 / 带预发布后缀 / 本地还没探到)。
    /// 面板只说「比不了」,把两边原话并排摆出来,不猜。
    Incomparable,
}

/// 纯判词:本地版本串 ↔ 远端 tag(拿不到传 `None`)。
///
/// 可比 = 「可选的 `v` 前缀 + 一个以上点分数值段」;预发布后缀(`-rc1`)、
/// 构建元数据(`+`)、任何非数值段一律不可比——**不猜**是这里的纪律:
/// `bake/research-desk-1.0.0-360ae68` 这种上游真会发出来的 tag,拿它跟
/// `0.1.0` 比大小只会得到一句瞎话。
pub fn verdict(local: &str, remote: Option<&str>) -> Verdict {
    match (version_parts(local), remote.and_then(version_parts)) {
        (Some(l), Some(r)) => match cmp_parts(&l, &r) {
            std::cmp::Ordering::Equal => Verdict::Same,
            std::cmp::Ordering::Less => Verdict::Behind,
            std::cmp::Ordering::Greater => Verdict::Ahead,
        },
        _ => Verdict::Incomparable,
    }
}

/// 段级比较,**缺失段当 0**:`0.1` 与 `0.1.0` 因此相等(尾零隐式补齐),
/// 而 `0.2` 与 `0.10` 仍按数值判(字典序会把 10 排到 2 前面,那是错的)。
fn cmp_parts(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            o => return o,
        }
    }
    std::cmp::Ordering::Equal
}

/// 把版本串解析成点分数值段;尾零隐式补齐(`0.1` ≡ `0.1.0`),非纯数值段即不可比。
fn version_parts(s: &str) -> Option<Vec<u64>> {
    let t = s.trim().trim_start_matches(['v', 'V']);
    if t.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for seg in t.split('.') {
        if seg.is_empty() || !seg.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        out.push(seg.parse::<u64>().ok()?);
    }
    Some(out)
}

/// 落后时的**升级指路**(ADR-0003 / ADR-0009:面板只给可复制的命令文案,
/// 执行权留给操作者,永远不代跑)。
///
/// 为什么不直接给「下载二进制」的命令:上游 release 的资产命名面板不知道,
/// 猜一个 `-p` 模式匹配不到只会得到一个更难懂失败。`gh release view` 打开
/// 的就是最新那条 release 本身(资产 + 说明),怎么升由操作者看着定。
pub fn upgrade_hint() -> &'static str {
    "gh release view --repo JoeWest9966/Nous-Warehouse"
}

/// 查询 argv(纯函数,与 `engine/commands.rs` 同纪律:只算命令,不做 IO)。
/// `gh api` 的 `--jq` 让 stdout 正好只有 tag 一个值——面板不自己解 JSON。
pub fn latest_tag_args() -> Vec<String> {
    vec![
        "api".into(),
        format!("repos/{ENGINE_RELEASE_REPO}/releases/latest"),
        "--jq".into(),
        ".tag_name".into(),
    ]
}

/// 远端查询缝:与 [`crate::engine::CommandRunner`] 同形的可注入接缝
/// (对象安全 + `Send + Sync`,测试换 [`FakeReleaseLookup`] 零网络)。
pub trait ReleaseLookup: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    fn latest_tag(&self) -> Pin<Box<dyn Future<Output = ReleaseProbe> + Send + '_>>;
}

/// 真实查询:fork `gh api`(凭据只从进程环境继承,`GH_TOKEN` / `GITHUB_TOKEN`,
/// 同 ADR-0009 的既有口径——面板不存、不写、不问)。
///
/// stderr 走 `Stdio::null()`:面板只用 rc + stdout 判态,不留错误正文,
/// 也免得没人读的管道写满把子进程挂住。
pub struct GhReleaseLookup;

impl ReleaseLookup for GhReleaseLookup {
    fn name(&self) -> &'static str {
        "gh api"
    }

    fn latest_tag(&self) -> Pin<Box<dyn Future<Output = ReleaseProbe> + Send + '_>> {
        Box::pin(async {
            let out = tokio::process::Command::new("gh")
                .args(latest_tag_args())
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output()
                .await;
            match out {
                Err(_) => ReleaseProbe::Unreachable, // gh 不在 PATH / fork 失败
                Ok(o) if !o.status.success() => ReleaseProbe::Unreachable, // 非零 rc:没网 / 没鉴权 / 404
                Ok(o) => {
                    let tag = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if tag.is_empty() {
                        // rc 0 但没有 tag:上游这条 release 没有 tag 字段
                        ReleaseProbe::Garbage
                    } else {
                        ReleaseProbe::Latest(tag)
                    }
                }
            }
        })
    }
}

/// 回放式查询(测试用):按脚本给结论,并记「被问过几次」——面板的「打开设置
/// 屏才对照一次、不轮询」由此可守。
pub struct FakeReleaseLookup {
    script: Vec<ReleaseProbe>,
    calls: AtomicUsize,
}

impl FakeReleaseLookup {
    pub fn new(script: Vec<ReleaseProbe>) -> Self {
        Self {
            script,
            calls: AtomicUsize::new(0),
        }
    }

    /// 测试探针:累计被问过几次(与 `FakeRunner::stdin_feed_count` 同形)。
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ReleaseLookup for FakeReleaseLookup {
    fn name(&self) -> &'static str {
        "回放"
    }

    fn latest_tag(&self) -> Pin<Box<dyn Future<Output = ReleaseProbe> + Send + '_>> {
        Box::pin(async {
            let i = self.calls.fetch_add(1, Ordering::SeqCst);
            self.script
                .get(i)
                .cloned()
                .unwrap_or(ReleaseProbe::Unreachable)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 判词:只信解析得出来的证据 ----

    #[test]
    fn 本地与远端同号判一致() {
        assert_eq!(verdict("0.1.0", Some("0.1.0")), Verdict::Same);
        assert_eq!(verdict("v0.1.0", Some("0.1.0")), Verdict::Same);
        // 尾零隐式补齐:0.1 与 0.1.0 是同一个版本
        assert_eq!(verdict("0.1", Some("0.1.0")), Verdict::Same);
        assert_eq!(verdict("0.1.0", Some("v0.1")), Verdict::Same);
    }

    #[test]
    fn 远端比本地新判落后_按数值不按字典() {
        assert_eq!(verdict("0.1.0", Some("0.2.0")), Verdict::Behind);
        // 0.10 > 0.2(字典序会把 10 排到 2 前面,那是错的)
        assert_eq!(verdict("0.2.0", Some("0.10.0")), Verdict::Behind);
        assert_eq!(verdict("0.1.0", Some("v1.0.0")), Verdict::Behind);
        // 段数不同也按补齐后比
        assert_eq!(verdict("0.1", Some("0.1.1")), Verdict::Behind);
    }

    #[test]
    fn 本地比远端新判领先_不是问题态() {
        assert_eq!(verdict("0.3.0", Some("0.2.0")), Verdict::Ahead);
        assert_eq!(verdict("1.0.0", Some("v0.9.9")), Verdict::Ahead);
    }

    #[test]
    fn 任一边不是版本号就判比不了_不猜() {
        // 上游真会发出来的 tag:带前缀 + 预发布段 + commit 短 hash
        assert_eq!(
            verdict("0.1.0", Some("bake/research-desk-1.0.0-360ae68")),
            Verdict::Incomparable
        );
        // 预发布后缀:与正式版比大小是判断题,面板不做
        assert_eq!(verdict("0.1.0", Some("0.2.0-rc1")), Verdict::Incomparable);
        assert_eq!(verdict("0.2.0-rc1", Some("0.1.0")), Verdict::Incomparable);
        // 本地还没探到版本
        assert_eq!(verdict("探测中", Some("0.1.0")), Verdict::Incomparable);
        assert_eq!(verdict("未知", Some("0.1.0")), Verdict::Incomparable);
        assert_eq!(verdict("", Some("0.1.0")), Verdict::Incomparable);
        // 非数值段
        assert_eq!(verdict("0.1.x", Some("0.1.0")), Verdict::Incomparable);
        assert_eq!(verdict("0..1", Some("0.1.0")), Verdict::Incomparable);
    }

    #[test]
    fn 远端没拿到就判比不了_呈现层另说取不到() {
        assert_eq!(verdict("0.1.0", None), Verdict::Incomparable);
        assert_eq!(verdict("0.1.0", Some("")), Verdict::Incomparable);
        assert_eq!(verdict("0.1.0", Some("   ")), Verdict::Incomparable);
    }

    // ---- 查询形状与回放 ----

    #[test]
    fn 查询argv指向引擎release仓的latest且只取tag() {
        let a = latest_tag_args();
        assert_eq!(
            a,
            vec![
                "api".to_string(),
                "repos/JoeWest9966/Nous-Warehouse/releases/latest".to_string(),
                "--jq".to_string(),
                ".tag_name".to_string(),
            ]
        );
        // 面板不猜资产命名,也不解 JSON:tag 由 gh 的 --jq 取
        assert!(a.iter().any(|s| s == "--jq"), "tag 交给 gh 取:{a:?}");
    }

    #[tokio::test]
    async fn 回放查询按脚本给结论并记次数() {
        let fake = FakeReleaseLookup::new(vec![
            ReleaseProbe::Latest("v0.2.0".into()),
            ReleaseProbe::Unreachable,
        ]);
        assert_eq!(fake.call_count(), 0, "没问过就是没问过");
        assert_eq!(
            fake.latest_tag().await,
            ReleaseProbe::Latest("v0.2.0".into())
        );
        assert_eq!(fake.latest_tag().await, ReleaseProbe::Unreachable);
        assert_eq!(fake.call_count(), 2, "问一次记一次");
        // 脚本用尽后不再编结论:一律取不到(保守,不猜)
        assert_eq!(fake.latest_tag().await, ReleaseProbe::Unreachable);
        assert_eq!(fake.call_count(), 3);
    }

    #[test]
    fn 升级指路只给命令文案_形状可复制() {
        let hint = upgrade_hint();
        assert!(hint.starts_with("gh release view"), "是可复制的命令:{hint}");
        assert!(
            hint.contains(ENGINE_RELEASE_REPO),
            "指着引擎 release 仓:{hint}"
        );
        // 不猜资产命名:没有 -p / --pattern 之类匹配不到就失败的模式
        assert!(!hint.contains("-p"), "不猜资产命名:{hint}");
    }
}

#[cfg(test)]
mod network_guard {
    //! 守「测试零网络」这条纪律本身:整棵测试树里,唯一会真连网的实现是
    //! [`super::GhReleaseLookup`],而它只该被 `App::assemble` 这一处生产路径
    //! 选上。这里用源码清点的方式键住——任何测试若直接编写那个名字,
    //! 本条红。
    //!
    //! 为什么值得单独立一条:注入缝(`ReleaseLookup` + `FakeReleaseLookup`)是
    //! 「测试可离线」的唯一保证,而保证本身没人守就会悄悄失效
    //! (下一个人加个集成测试就连了网)。注意本模块自身不被扫——它的断言字面量里写着那个名字,
    //! 扫自己会把自己撞红(那样的守卫没意义)。

    use super::*;

    /// 真实查询的类名(不写字面量:本模块的源码被别人清点,自己不扫自己。
    /// 字段名来自 `super`,不会把名字写进本文件的源码里。)
    const REAL: &str = "GhReleaseLookup";

    /// 把注释行换掉后的代码(注释里提到名字不算选用。
    /// 也不能把本模块自己扫进去——它的断言里写着这个名字。)
    ///
    /// 输入走编译期 `include_str!`(相对本文件),不靠进程 CWD——
    /// `cargo test` 的 CWD 是包根,而源文件在 `src/` 下,用 `fs` 读相对路径会读不到。
    fn code_of(src: &str, from: Option<usize>, to: Option<usize>) -> String {
        let lo = from.unwrap_or(0);
        let hi = to.unwrap_or(src.len());
        src[lo..hi]
            .lines()
            .filter(|l| {
                let l = l.trim_start();
                !(l.starts_with("//") || l.starts_with("/*") || l.starts_with('*'))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn 本文件的测试不直连真实查询() {
        let src = include_str!("release.rs");
        let at = src.find("mod tests").unwrap();
        let end = src.find("mod network_guard").unwrap();
        let tests = code_of(src, Some(at), Some(end));
        assert!(
            !tests.contains(REAL),
            "测试不许直接选真实查询(会连网);要测查询形状就测 latest_tag_args()"
        );
        // 查询 argv 是纯函数:连网与否与它无关,测它零网络
        assert_eq!(latest_tag_args().len(), 4, "查询形状四段");
    }

    #[test]
    fn 别处的测试也不直连真实查询() {
        // 真正的风险在别处:别人加的 app / 屏幕测试想要真查一次。
        // 测试要离线,得给注入缝换 `FakeReleaseLookup`。
        for (path, src) in [
            ("src/app.rs", include_str!("app.rs")),
            ("src/screens/tests.rs", include_str!("screens/tests.rs")),
        ] {
            // 注意:`screens/tests.rs` 本身就是测试文件(它里面没有 `mod tests` 声明),
            // 所以找不到时整份当测试代码扫——否则这个文件会被整直跳过。
            let at = src.find("mod tests");
            let tests = code_of(src, at, None);
            assert!(!tests.contains(REAL), "{path} 的测试不许直连网");
        }
    }

    #[test]
    fn 真实查询的选用点不在本模块() {
        let prod = code_of(
            include_str!("release.rs"),
            None,
            Some(include_str!("release.rs").find("mod tests").unwrap()),
        );
        assert!(
            !prod.contains(&format!("Arc::new({REAL})")),
            "选用(而非定义)应在 App::assemble;本模块只赁让缝形状"
        );
        let needle = concat!("tokio::process::Command::new(", '"', "gh", '"', ")");
        assert!(prod.contains(needle), "真实实现在场");
    }
}
