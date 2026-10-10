//! typed 命令构造。面板只发三类动作(ADR-0003)+ 版本探测 + 只读 `ls`。

/// 五域总览。数据仓根**显式携带**——签名强制带 repo,
/// 类型层面杜绝「不带 --repo 让引擎拿当前目录兜底」(spec #24,票 #27)。
/// 缺根时调用方不应构造命令(下发前有门控,见 `App::spawn_engine`)。
pub fn ls(repo: &str) -> Vec<String> {
    vec!["ls".to_string(), "--repo".into(), repo.to_string()]
}

/// 装配。`lock` 可以是 `.nwlock` 或装配命令文件。
pub fn up(lock: &str, nw_home: Option<&str>) -> Vec<String> {
    let mut a = vec!["up".to_string(), "--lock".into(), lock.to_string()];
    if let Some(h) = nw_home {
        a.push("--nw-home".into());
        a.push(h.into());
    }
    a
}

/// 健康体检:按派名(profile)。
pub fn doctor_profile(profile: &str, nw_home: Option<&str>) -> Vec<String> {
    let mut a = vec![
        "doctor".to_string(),
        "--profile".into(),
        profile.to_string(),
    ];
    if let Some(h) = nw_home {
        a.push("--nw-home".into());
        a.push(h.into());
    }
    a
}

/// 健康体检:按锁定集。
pub fn doctor_lock(lock: &str, nw_home: Option<&str>) -> Vec<String> {
    let mut a = vec!["doctor".to_string(), "--lock".into(), lock.to_string()];
    if let Some(h) = nw_home {
        a.push("--nw-home".into());
        a.push(h.into());
    }
    a
}

/// 校验:配方侧。
pub fn validate_recipe(recipe: &str) -> Vec<String> {
    vec!["validate".into(), "--recipe".into(), recipe.into()]
}

/// 校验:锁定集侧。
pub fn validate_lock(lock: &str) -> Vec<String> {
    vec!["validate".into(), "--lock".into(), lock.into()]
}

/// 版本探测。注意:`nw --version` 的裸版本号走 **stderr**。
pub fn version() -> Vec<String> {
    vec!["--version".to_string()]
}

/// 外部源登记·预审段(spec #25 ticket #30):扫描 + 验收 5(+1) 条,
/// 拟落账记录进 JSON,**文件不落**——铁律是不带 `--yes`(确认闸),
/// 本函数连 `--yes` 的位置都没有,从类型层面杜绝预审误落账。
/// `--root` 指数据仓根(与 `ls --repo` 同义的本地仓根槽位)。
pub fn source_register_preaudit(repo: &str, kind: &str, root: &str) -> Vec<String> {
    vec![
        "source".into(),
        "register".into(),
        "--repo".into(),
        repo.to_string(),
        "--kind".into(),
        kind.to_string(),
        "--root".into(),
        root.to_string(),
    ]
}

/// 外部源登记·落账段(spec #25 ticket #31):与预审同参,多一枚 `--yes`
/// 确认闸——引擎才把 `catalog.json` 写进数据仓根。**只能由图形确认后的
/// `App::dispatch_register_commit` 调用**;预审段用的
/// [`source_register_preaudit`] 连 `--yes` 的位置都没有,两段从构造函数
/// 形状上分开,误下发靠类型杜绝。
pub fn source_register_commit(repo: &str, kind: &str, root: &str) -> Vec<String> {
    let mut a = source_register_preaudit(repo, kind, root);
    a.push("--yes".into());
    a
}

/// 三条前提缺哪一条:两种配方域建议命令(bake / new)共用这一份判定,
/// 措辞各归各自的 gate enum——判定只有一处,说法不互相冒充。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Precondition {
    NoDataRoot,
    NotARecipe,
    NoCapability,
}

/// 前提判定(纯函数,无 IO、不读环境、不碰能力账):顺序即 bake 既有的
/// 信任顺序——先看选中行是不是配方,再看数据仓根,最后看引擎能力。
/// 任缺一条返回缺的那一条,**绝不返回半条命令**。
fn precondition<'a>(
    recipe: Option<&'a str>,
    data_root: Option<&'a str>,
    capable: bool,
) -> Result<(&'a str, &'a str), Precondition> {
    let Some(name) = recipe else {
        return Err(Precondition::NotARecipe);
    };
    let Some(root) = data_root else {
        return Err(Precondition::NoDataRoot);
    };
    if !capable {
        return Err(Precondition::NoCapability);
    }
    Ok((name, root))
}

/// 配方域**建议命令**(spec #33 / 工单 #39,ADR-0003):选中一个配方时,
/// 面板给出可复制的 `nw recipe bake …` 命令文案,**执行权留给操作者**——
/// 本模块只算命令,永远不把它下发(`App` 没有任何路径会调它)。
///
/// 旗标与 [`recipe_bake`] 的拼装只有这一处真相:与 `ls` / `up` / `doctor`
/// 同一层构造,面板不内置第二份「命令长什么样」。
///
/// 三种缺前提各有可区分的指路,**绝不渲染半条命令**:没设数据仓根(spec #24:
/// 不猜、不拿当前目录兜底)、选中行不是配方、引擎没有 `recipe bake` 能力
/// (#29 能力探测门的结论,不是本地常量表)。
///
/// 缺前提的三种可区分原因(与 [`crate::app::DataRootCause`] 同形:
/// 标题 + 说法,两两不同、互不冒充)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BakeGate {
    /// 数据仓根未设——五域读不到数据仓,命令指哪儿都不对。
    NoDataRoot,
    /// 当前没有选中的配方(配方域为空,或选中行不是配方)。
    NotARecipe,
    /// 引擎没有 `recipe bake` 能力(缺的是引擎能力,不是面板坏了)。
    NoCapability,
}

impl BakeGate {
    /// 指路标题:三态各说各话。
    pub fn title(self) -> &'static str {
        match self {
            BakeGate::NoDataRoot => "数据仓根未设",
            BakeGate::NotARecipe => "没有选中的配方",
            BakeGate::NoCapability => "引擎缺 recipe bake 能力",
        }
    }

    /// 指路人话:每态一条,带各自的事实与下一步。
    pub fn advice(self) -> String {
        match self {
            BakeGate::NoDataRoot => {
                "五域读不到数据仓,bake 命令指不到本地仓。去设置屏按 d 录入数据仓根;面板不拿当前目录兜底。"
                    .into()
            }
            BakeGate::NotARecipe => {
                "配方域为空或未选中配方——先在配方域选中一条(↑/↓ 或鼠标点行)。".into()
            }
            BakeGate::NoCapability => {
                "缺的是引擎能力,不是面板坏了;升级所配 nw 后建议命令自动给出。".into()
            }
        }
    }

    /// 与 [`Precondition`] 的映射:判定只有一处(见 [`precondition`]),
    /// 说法仍是 bake 自己那一套。
    fn of(p: Precondition) -> Self {
        match p {
            Precondition::NoDataRoot => BakeGate::NoDataRoot,
            Precondition::NotARecipe => BakeGate::NotARecipe,
            Precondition::NoCapability => BakeGate::NoCapability,
        }
    }
}

/// 算出的建议命令:argv(下发形态,面板**不**下发)与给人看的展示串。
/// bake(#39)与 recipe new(#58)两条建议命令共用这一个形状——argv 与
/// 展示串同源,`display` 永远是 `"nw " + argv.join(" ")`。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BakeSuggest {
    /// 完整 argv(含二进制名之后的所有元素),与 [`recipe_bake`] / [`recipe_new`] 同源。
    pub argv: Vec<String>,
    /// 操作者可见的命令文案(`nw ` 起头,可直接复制)。
    pub display: String,
}

/// 位置参数 `<配方>[@版本]`:配方带版本时才拼 `@版本` 段(#39 验收第 3 条)。
/// 版本缺失时**不猜**——只给配方名,让引擎按配方自己的版本烤。
fn recipe_arg(recipe: &str, version: Option<&str>) -> String {
    match version {
        Some(v) if !v.is_empty() => format!("{recipe}@{v}"),
        _ => recipe.to_string(),
    }
}

/// 建议命令的 `--out` 默认值:`<配方>.sh`(操作者可改)。
/// 省了 `--out` 时引擎自己落 `<配方目录>/../baked/nw-assemble.sh`——
/// 那句话由呈现层照引擎原话说,不在这里替引擎承诺。
pub fn bake_out_default(recipe: &str) -> String {
    format!("{recipe}.sh")
}

/// 烘焙:把配方烤成锁定集 + 一条 POSIX sh 装配命令。
///
/// `version` 非空时位置参数带 `@版本`(与配方版本不符由引擎拒,rc 3)。
/// 形状实测自引擎 `nw recipe bake --help` 的用法行(所配 0.1.0):
/// `nw recipe bake <配方>[@版本] --out <命令文件> [--assets-dir <目录>] [--repo <目录>]`。
pub fn recipe_bake(recipe: &str, version: Option<&str>, out: &str, repo: &str) -> Vec<String> {
    vec![
        "recipe".into(),
        "bake".into(),
        recipe_arg(recipe, version),
        "--out".into(),
        out.to_string(),
        "--repo".into(),
        repo.to_string(),
    ]
}

/// 由「选中配方 + 数据仓根 + 出参默认值 + 引擎能力」算出建议命令。
///
/// 纯函数:无 IO、不读环境、不碰能力账——`bake_capable` 由调用方从
/// 能力探测门(#29)取来。任一前提不成立就走对应的 [`BakeGate`],
/// **绝不返回半条命令**。
pub fn recipe_bake_suggest(
    recipe: Option<(&str, Option<&str>)>,
    data_root: Option<&str>,
    bake_capable: bool,
) -> Result<BakeSuggest, BakeGate> {
    let (name, repo) =
        precondition(recipe.map(|(n, _)| n), data_root, bake_capable).map_err(BakeGate::of)?;
    let version = recipe.and_then(|(_, v)| v);
    let out = bake_out_default(name);
    let argv = recipe_bake(name, version, &out, repo);
    Ok(BakeSuggest {
        display: format!("nw {}", argv.join(" ")),
        argv,
    })
}

/// 配方域**新建骨架**建议命令(spec #33 / 工单 #58,ADR-0003 + ADR-0006):
/// 选中一个配方时,面板在 bake 之外一并给出可复制的 `nw recipe new …`
/// 命令文案,**执行权留给操作者**——与 bake 同源同层(同一个纯函数层产出
/// argv 与展示串),本模块只算命令,永远不把它下发(`App` 没有任何路径会调它)。
///
/// 三种缺前提各有可区分的指路,**绝不渲染半条命令**:没设数据仓根、
/// 选中行不是配方、引擎没有 `recipe new` 能力(#29 能力探测门的结论)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecipeNewGate {
    /// 数据仓根未设——骨架要写进数据仓,`--out-dir` 指哪儿都不对。
    NoDataRoot,
    /// 当前没有选中的配方(配方域为空,或选中行不是配方):骨架名跟着它起。
    NotARecipe,
    /// 引擎没有 `recipe new` 能力(缺的是引擎能力,不是面板坏了)。
    NoCapability,
}

impl RecipeNewGate {
    /// 指路标题:三态各说各话。
    pub fn title(self) -> &'static str {
        match self {
            RecipeNewGate::NoDataRoot => "数据仓根未设",
            RecipeNewGate::NotARecipe => "没有选中的配方",
            RecipeNewGate::NoCapability => "引擎缺 recipe new 能力",
        }
    }

    /// 指路人话:每态一条,带各自的事实与下一步;与 [`BakeGate`] 同形
    /// 不同文(标题可能同题,说法绝不互相冒充)。
    pub fn advice(self) -> String {
        match self {
            RecipeNewGate::NoDataRoot => {
 "五域读不到数据仓,recipe new 的 --out-dir 指不到本地仓。去设置屏按 d 录入数据仓根;面板不拿当前目录兜底。"
                    .into()
            }
            RecipeNewGate::NotARecipe => {
                "配方域为空或未选中配方——先在配方域选中一条(↑/↓ 或鼠标点行),骨架名跟着它起。".into()
            }
            RecipeNewGate::NoCapability => {
                "缺的是引擎能力,不是面板坏了;升级所配 nw 后 recipe new 的建议命令自动给出。"
                    .into()
            }
        }
    }

    fn of(p: Precondition) -> Self {
        match p {
            Precondition::NoDataRoot => RecipeNewGate::NoDataRoot,
            Precondition::NotARecipe => RecipeNewGate::NotARecipe,
            Precondition::NoCapability => RecipeNewGate::NoCapability,
        }
    }
}

/// 新建配方骨架:写 `<out-dir>/recipes/<名>/recipe.yaml`,不碰 git。
///
/// `profile` 非空时才带 `--profile`——上游缺省 "" = 按配方名兜底
/// (#8 布局),面板不替引擎猜一个派名。`--out-dir` 与 bake 的 `--repo`
/// 同一纪律:显式指数据仓根,不让引擎拿当前目录兜底。
/// 形状实测自引擎 usage 行(所配 0.1.0):
/// `nw recipe new <名> [--profile <名>] [--out-dir <目录>]`。
pub fn recipe_new(name: &str, profile: Option<&str>, out_dir: &str) -> Vec<String> {
    let mut a = vec!["recipe".into(), "new".into(), name.to_string()];
    if let Some(p) = profile.filter(|p| !p.is_empty()) {
        a.push("--profile".into());
        a.push(p.to_string());
    }
    a.push("--out-dir".into());
    a.push(out_dir.to_string());
    a
}

/// 由「选中配方 + 数据仓根 + 引擎能力」算出新建骨架的建议命令。
///
/// 纯函数:无 IO、不读环境、不碰能力账——`new_capable` 由调用方从
/// 能力探测门(#29)取来。任一前提不成立就走对应的 [`RecipeNewGate`],
/// **绝不返回半条命令**。
pub fn recipe_new_suggest(
    recipe: Option<&str>,
    data_root: Option<&str>,
    new_capable: bool,
) -> Result<BakeSuggest, RecipeNewGate> {
    let (name, out_dir) =
        precondition(recipe, data_root, new_capable).map_err(RecipeNewGate::of)?;
    // 派名面板不知道,也不猜:不填 --profile,由引擎按配方名兜底(#8 布局)
    let argv = recipe_new(name, None, out_dir);
    Ok(BakeSuggest {
        display: format!("nw {}", argv.join(" ")),
        argv,
    })
}

/// 外部源漂移复核(spec #34 / 工单 #40):**只读**——只报告漂移、永不写账。
/// `name` 非空 = 只查这一条记录;None = 全查已登记源。
/// `--root` 指数据仓根(与 register 同槽位;别与 ls 的 `--repo` 混)。
/// 铁律:本命令不产生、也不允许产生任何写旗标——两段式登记的写入口只有
/// `source_register_commit` 一个,那里才有 `--yes`。
pub fn source_recheck(name: Option<&str>, root: &str) -> Vec<String> {
    let mut a = vec!["source".into(), "recheck".into()];
    if let Some(n) = name {
        a.push("--name".into());
        a.push(n.to_string());
    }
    a.push("--root".into());
    a.push(root.to_string());
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn up_带_nw_home() {
        assert_eq!(
            up("x.nwlock", Some("/data")),
            vec!["up", "--lock", "x.nwlock", "--nw-home", "/data"]
        );
    }

    #[test]
    fn doctor_二选一_flag_不混用() {
        assert_eq!(
            doctor_profile("desk", None),
            vec!["doctor", "--profile", "desk"]
        );
        assert_eq!(
            doctor_lock("x.nwlock", None),
            vec!["doctor", "--lock", "x.nwlock"]
        );
    }

    #[test]
    fn ls_必带_repo_不给引擎当前目录兜底() {
        assert_eq!(ls("/data/repo"), vec!["ls", "--repo", "/data/repo"]);
        // 签名里没有 Option:想不带 --repo 都写不出来
        let a = ls(".");
        assert_eq!(a[1], "--repo");
    }

    #[test]
    fn recheck_只读_按名限定与全查都不带任何写旗标() {
        let all = source_recheck(None, "/data/root");
        assert_eq!(
            all,
            vec!["source", "recheck", "--root", "/data/root"],
            "全查:不带 --name"
        );
        let one = source_recheck(Some("web-search"), "/data/root");
        assert_eq!(one[2], "--name");
        assert_eq!(one[3], "web-search");
        assert_eq!(one[4], "--root");
        for a in [&all, &one] {
            assert!(
                !a.iter().any(|s| s == "--yes"),
                "只读铁律:recheck 的 argv 里不许出现 --yes(写入口只有 register 一个)"
            );
        }
    }

    #[test]
    fn 登记预审_永不带_yes_根槽位指数据仓根() {
        let a = source_register_preaudit("owner/repo", "skill", "/data/root");
        assert_eq!(
            a,
            vec![
                "source",
                "register",
                "--repo",
                "owner/repo",
                "--kind",
                "skill",
                "--root",
                "/data/root"
            ]
        );
        assert!(
            !a.iter().any(|s| s == "--yes"),
            "预审段铁律:argv 里不许出现 --yes(文件不落)"
        );
    }

    #[test]
    fn 建议命令_带版本与出参默认值_旗标只有一份真相() {
        // 与 ls/up/doctor 同一处拼装:形状变了只改 recipe_bake 一处
        let a = recipe_bake(
            "research-desk",
            Some("1.0.0"),
            "research-desk.sh",
            "/data/repo",
        );
        assert_eq!(
            a,
            vec![
                "recipe",
                "bake",
                "research-desk@1.0.0",
                "--out",
                "research-desk.sh",
                "--repo",
                "/data/repo"
            ]
        );
        // 版本缺失:不猜,只给配方名
        assert_eq!(
            recipe_bake("desk", None, "desk.sh", "/data/repo")[2],
            "desk"
        );
        assert_eq!(bake_out_default("desk"), "desk.sh");
    }

    #[test]
    fn 建议命令_三前提齐备才算得出_且可复制() {
        let ok = recipe_bake_suggest(
            Some(("research-desk", Some("1.0.0"))),
            Some("/data/repo"),
            true,
        )
        .expect("三前提齐备必须给得出命令");
        assert_eq!(
            ok.display,
            "nw recipe bake research-desk@1.0.0 --out research-desk.sh --repo /data/repo"
        );
        assert!(
            ok.argv.iter().all(|a| !a.contains("--yes")),
            "建议命令永不带确认闸"
        );
        assert_eq!(ok.display, format!("nw {}", ok.argv.join(" ")));
    }

    #[test]
    fn 建议命令_三种缺前提各有可区分指路_绝不给半条命令() {
        // 选中行不是配方
        assert_eq!(
            recipe_bake_suggest(None, Some("/data/repo"), true),
            Err(BakeGate::NotARecipe)
        );
        // 未设数据仓根(不拿当前目录兜底)
        assert_eq!(
            recipe_bake_suggest(Some(("desk", None)), None, true),
            Err(BakeGate::NoDataRoot)
        );
        // 引擎没有 recipe bake 能力
        assert_eq!(
            recipe_bake_suggest(Some(("desk", None)), Some("/data/repo"), false),
            Err(BakeGate::NoCapability)
        );
        // 三种指路两两不同,互不冒充
        let gates = [
            BakeGate::NoDataRoot,
            BakeGate::NotARecipe,
            BakeGate::NoCapability,
        ];
        for (i, a) in gates.iter().enumerate() {
            for b in &gates[i + 1..] {
                assert_ne!(a.title(), b.title(), "标题两两不同");
                assert_ne!(a.advice(), b.advice(), "说法两两不同");
            }
        }
        assert!(BakeGate::NoCapability.advice().contains("不是面板坏了"));
        assert!(BakeGate::NoDataRoot.advice().contains("不拿当前目录兜底"));
    }

    #[test]
    fn 建议命令_指路人话不带连续空格_换行残留不入屏() {
        // NoDataRoot 的说法曾是一行拼接残留,中段混进 17 个空格,
        // 操作者在屏上会看到怪异空档——指路人话是一句正常句子。
        for gate in [
            BakeGate::NoDataRoot,
            BakeGate::NotARecipe,
            BakeGate::NoCapability,
        ] {
            let advice = gate.advice();
            assert!(
                !advice.contains("  "),
                "{} 的说法中段不许有一长串空格:{advice:?}",
                gate.title()
            );
        }
    }

    #[test]
    fn 建议命令_recipe_new_位置参数恰好一个_旗标与数据仓根同源() {
        // 形状实测自引擎 usage 行(所配 0.1.0):
        // nw recipe new <名> [--profile <名>] [--out-dir <目录>]
        assert_eq!(
            recipe_new("research-desk", None, "/data/repo"),
            vec!["recipe", "new", "research-desk", "--out-dir", "/data/repo"]
        );
        // 派名非空才给 --profile:上游缺省 = 按配方名兜底(#8 布局),面板不替引擎猜
        assert_eq!(
            recipe_new("desk", Some("desk"), "/data/repo"),
            vec![
                "recipe",
                "new",
                "desk",
                "--profile",
                "desk",
                "--out-dir",
                "/data/repo"
            ]
        );
        assert_eq!(
            recipe_new("desk", Some(""), "/data/repo"),
            recipe_new("desk", None, "/data/repo"),
            "空派名等于没给"
        );
    }

    #[test]
    fn 建议命令_recipe_new_三前提齐备_argv与展示串同源且无写旗标() {
        let ok = recipe_new_suggest(Some("research-desk"), Some("/data/repo"), true)
            .expect("三前提齐备必须给得出命令");
        assert_eq!(
            ok.display,
            "nw recipe new research-desk --out-dir /data/repo"
        );
        assert_eq!(
            ok.display,
            format!("nw {}", ok.argv.join(" ")),
            "展示串由 argv 拼出"
        );
        for a in &ok.argv {
            assert!(
                !a.contains("--yes") && !a.contains("--json"),
                "建议命令永不带写旗标/no-op 旗标:{a}"
            );
        }
    }

    #[test]
    fn 建议命令_recipe_new_三种缺前提各有可区分指路_不给半条命令() {
        assert_eq!(
            recipe_new_suggest(None, Some("/data/repo"), true),
            Err(RecipeNewGate::NotARecipe)
        );
        assert_eq!(
            recipe_new_suggest(Some("desk"), None, true),
            Err(RecipeNewGate::NoDataRoot)
        );
        assert_eq!(
            recipe_new_suggest(Some("desk"), Some("/data/repo"), false),
            Err(RecipeNewGate::NoCapability)
        );
        // 三种指路两两不同,互不冒充;且与 bake 的同名指路不同文(不冒充)
        let gates = [
            RecipeNewGate::NoDataRoot,
            RecipeNewGate::NotARecipe,
            RecipeNewGate::NoCapability,
        ];
        for (i, a) in gates.iter().enumerate() {
            for b in &gates[i + 1..] {
                assert_ne!(a.title(), b.title(), "标题两两不同");
                assert_ne!(a.advice(), b.advice(), "说法两两不同");
            }
        }
        assert_ne!(
            RecipeNewGate::NoDataRoot.advice(),
            BakeGate::NoDataRoot.advice(),
            "同为未设根,new 的说法要点名 --out-dir,不拿 bake 的话冒充"
        );
        assert_ne!(
            RecipeNewGate::NoCapability.advice(),
            BakeGate::NoCapability.advice(),
            "缺能力要说是缺哪一个"
        );
        assert!(RecipeNewGate::NoCapability.advice().contains("recipe new"));
        assert!(RecipeNewGate::NoDataRoot.advice().contains("--out-dir"));
        assert!(RecipeNewGate::NoDataRoot
            .advice()
            .contains("不拿当前目录兜底"));
    }

    #[test]
    fn 建议命令_recipe_new_指路人话不带连续空格_换行残留不入屏() {
        for gate in [
            RecipeNewGate::NoDataRoot,
            RecipeNewGate::NotARecipe,
            RecipeNewGate::NoCapability,
        ] {
            let advice = gate.advice();
            assert!(
                !advice.contains("  "),
                "{} 的说法中段不许有一长串空格:{advice:?}",
                gate.title()
            );
        }
    }

    #[test]
    fn 登记落账_带_yes_带_root_不带_category() {
        let a = source_register_commit("owner/repo", "skill", "/data/root");
        assert_eq!(
            a,
            vec![
                "source",
                "register",
                "--repo",
                "owner/repo",
                "--kind",
                "skill",
                "--root",
                "/data/root",
                "--yes"
            ]
        );
        assert!(
            a.iter().any(|s| s == "--yes"),
            "落账段铁律:确认闸必须在 argv 里(否则文件不落)"
        );
        assert!(
            !a.iter().any(|s| s == "--category"),
            "类别输入不给:--category 是 0.1.0 就有的 source register 可重复旗标,\
             不是引擎能力缺口;要不要给类别输入是产品范围问题(ADR-0006),\
             理由不能写成「引擎没这个能力」"
        );
        // 预审与落账只差确认闸一枚棋:同根、同链、不同后果
        let p = source_register_preaudit("owner/repo", "skill", "/data/root");
        let mut pc = p.clone();
        pc.push("--yes".into());
        assert_eq!(a, pc, "落账 = 预审 + --yes,不另改槽位");
    }
}
