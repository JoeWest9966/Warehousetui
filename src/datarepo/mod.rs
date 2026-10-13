//! 数据仓/安装根访问层(`datarepo`,spec #94 工单 #101 立)。
//!
//! 与 [`crate::engine`] 并列的**第二条访问层**:引擎层是「调 `nw` 子进程」的
//! 唯一接缝,本模块是「不调引擎、直接读文件」的唯一接缝。两者分工不混——
//! 读文件的事(清单存档 / 回收站 / 派枚举 / 日志存档 / 版本对照的数据面)
//! 一律问本模块,屏幕层与 app 层不自己拼 `$NW_HOME` 路径。
//!
//! 本模块只拥有**路径布局知识**(上游既有事实,面板只读不写):
//! `$NW_HOME/profiles/<派>/.nw-manifest`、`locks/`、`pruned/<时间戳>/`、
//! `logs/`;数据仓根的 `recipes/<名>/recipe.yaml`、`components/<名>/`。
//! 布局变更由「跟随引擎」纪律兜住(ADR-0007):上游改了,这里改,
//! 屏幕层一个字不动。
//!
//! **对外只暴露意图级方法**:调用方说「要什么」,不说「去哪个目录取哪个
//! 文件」。返回值是**分类**(读到了 / 不在 / 读不通 / 不认识),不是
//! `Err`——一条坏记录不该炸掉整屏(规格故事 4)。
//!
//! 接缝契约(方法清单与「谁可以调它」)钉在 `docs/adr/0010-*.md`。已落地
//! 的读侧意图:清单存档读取(#101)与数据仓引用图(#116);回收站 / 日志 /
//! 派总览 / 版本对照的方法由各自工单补——**不在这里留占位 stub**,各票
//! 落地时往 ADR §2 的表里填一行、往修订记录里记一笔。

use std::path::PathBuf;

use crate::engine::discover::expand_tilde;

/// 清单文件名(上游 `internal/manifest` 的原子提交点,永远最后写)。
pub const MANIFEST_FILE: &str = ".nw-manifest";

/// 面板唯一认得的清单 schema(上游既有事实;不认识的走契约守卫同款
/// 「需升级 nw」话术,不按旧形状降级解析)。
pub const MANIFEST_SCHEMA: &str = "nw/manifest.v1";

/// 安装根下的派目录(每个派一部,清单住在其根部)。
pub const PROFILES_DIR: &str = "profiles";

/// 安装根下的锁定集目录(引擎 `ls` 的 locks 域与装配命令交锁段的同一落点)。
pub const LOCKS_DIR: &str = "locks";

/// 数据仓根的配方目录(每本配方一部,`recipe.yaml` 住在其根部;
/// 引擎 `ls` 的 recipes 域给的就是 `<根>/recipes/<名>/recipe.yaml`)。
pub const RECIPES_DIR: &str = "recipes";

/// 配方文件名(上游既有事实;面板只读,永不写)。
pub const RECIPE_FILE: &str = "recipe.yaml";

/// 数据仓根的组件目录:skill / 扩展 / 模型配置 / agent 模板都住在这里
/// (CONTEXT.md 的「组件」词条),每部一个目录、`component.yaml` 住在其根部。
pub const COMPONENTS_DIR: &str = "components";

/// 安装根不可用的两种原因(与数据仓根的
/// [`crate::app::DataRootCause`] 同口径:各有各的说法、互不冒充)。
///
/// 为什么需要它:派目录枚举在安装根不在时天然返回空,而「空」在屏上
/// 与「一个派都还没装」长得一模一样——操作者会以为面板读了、只是没
/// 数据(规格故事 28:不出现拼错路径读空目录的假象)。所以可用性单独
/// 问一次,不可用时走指路,不走空列表。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HomeCause {
    /// 目录不在:还没装过派,或 NW_HOME 指错了。
    Missing,
    /// 目录在但不是可读的目录(是文件 / 权限不够)。
    Unreadable { reason: String },
}

/// 安装根可用吗;`None` = 可用。
///
/// `~` 仍走 [`expand_tilde`] 那一份展开器;判定只用 `fs::metadata`
/// 一次(与 `App::path_usable` 同级的轻量探问,不遍历、不读内容)。
pub fn home_cause(nw_home: &str) -> Option<HomeCause> {
    let home = expand_tilde(nw_home);
    match std::fs::metadata(&home) {
        Ok(md) if md.is_dir() => None,
        Ok(_) => Some(HomeCause::Unreadable {
            reason: "不是目录".into(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(HomeCause::Missing),
        Err(e) => Some(HomeCause::Unreadable {
            reason: e.to_string(),
        }),
    }
}

/// 选中锁定集的载体文件路径:`$NW_HOME/locks/<lockId>.nwlock`
/// (引擎 `ls` 的 locks 域与装配命令交锁段的同一落点)。
///
/// 从 `App::lock_carrier` 原样搬来(#101):这是面板原先唯一一处 app 层
/// 拼 `$NW_HOME` 路径的地方,`~` 展开仍走
/// [`crate::engine::discover::expand_tilde`] 那一份(全仓只有一个展开器)。
/// 行为逐字节不变——`app.rs` 既有两条载体落点测试原样绿。
pub fn lock_carrier(nw_home: &str, lock_id: &str) -> String {
    let home = expand_tilde(nw_home);
    format!(
        "{}/{LOCKS_DIR}/{lock_id}.nwlock",
        home.display().to_string().trim_end_matches('/')
    )
}

/// 枚举安装根下的派目录名(**确定性字典序**)。
///
/// `profiles/` 不在 / 读不通 → 空 `Vec`:调用方照「没有派」呈现。但
/// **安装根本身不在**要另问 [`home_cause`]——「一个派都还没装」与
/// 「NW_HOME 指错了」在屏上是两件事,不共用一个空列表(规格故事 28)。
/// 只收目录:落在 `profiles/` 里的散落文件不算一部派。
pub fn list_profiles(nw_home: &str) -> Vec<String> {
    let dir = expand_tilde(nw_home).join(PROFILES_DIR);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// 某派清单文件的落点(`$NW_HOME/profiles/<派>/.nw-manifest`)。
///
/// 全仓唯一知道这个形状的地方(#101 收口);屏幕层要显示路径时问这里,
/// 不自己拼。
pub fn manifest_path(nw_home: &str, profile: &str) -> PathBuf {
    expand_tilde(nw_home)
        .join(PROFILES_DIR)
        .join(profile)
        .join(MANIFEST_FILE)
}

/// 一份清单存档:字段形状沿用引擎侧的 [`crate::engine::ManifestBrief`]
/// (envelope.rs 早已有的同一份契约,不另立一套字段名),外加文件落点与
/// 原文——「回车看清单原文」(规格故事 3)看的就是 `raw`,不回头再读一次盘
/// (快照语义:要新的按 `r` 重读)。
///
/// 不派生 `PartialEq`/`Eq`:里面嵌的引擎类型(`ManifestBrief` / `Evidence`)
/// 天生不可比(它们是「容缺的解析视图」,不是一个值)。测试按字段断言,
/// 不需要整体相等。
#[derive(Clone, Debug)]
pub struct ManifestArchive {
    pub brief: crate::engine::ManifestBrief,
    /// 文件落点(只读呈现;面板不写)。
    pub path: String,
    /// 文件原文(回车呈现用)。
    pub raw: String,
}

/// 清单存档的读取结论:**四种分类,没有第五种**。
///
/// 分类而不是 `Err`:一条坏记录只该坏它自己那一行,不该炸掉整屏
/// (规格故事 4)。屏幕层把四种分类映射成四种呈现,datarepo 不决定颜色。
///
/// 同 [`ManifestArchive`]:不派生相等性——里面嵌的引擎解析视图不可比。
#[derive(Clone, Debug)]
pub enum ManifestStatus {
    /// 认得也读通了。
    Ok(ManifestArchive),
    /// 文件不在:该派尚未完成装配,或被清过(引擎侧同款事实是 doctor rc 12)。
    Missing,
    /// 文件在但读不通:不是合法 JSON,或缺 `schema` 字段。
    Broken { reason: String },
    /// JSON 合法、`schema` 不认得:契约守卫同款「需升级 nw」五字段证据
    /// (不猜、不按旧形状降级解析)。
    UnknownSchema { guard: crate::engine::Evidence },
}

/// 读某派的清单存档(只读;面板永不写 `.nw-manifest`——上游把它当唯一
/// 提交点,永远最后写)。
///
/// 判据只有两样:文件在不在、`schema` 认不认得。字段缺不缺**不**是分类
/// 依据——缺字段的清单仍是认得的那一份,屏上显「——」(与引擎 ls 投影的
/// 容缺口径一致)。
pub fn read_manifest(nw_home: &str, profile: &str) -> ManifestStatus {
    let path = manifest_path(nw_home, profile);
    let raw = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ManifestStatus::Missing,
        Err(e) => {
            return ManifestStatus::Broken {
                reason: format!("清单读不出:{e}"),
            }
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return ManifestStatus::Broken {
                reason: format!("清单不是合法 JSON:{e}"),
            }
        }
    };
    let Some(schema) = value.get("schema").and_then(|s| s.as_str()) else {
        return ManifestStatus::Broken {
            reason: "清单缺 schema 字段:不知道这是哪一版".into(),
        };
    };
    if schema.trim().is_empty() {
        return ManifestStatus::Broken {
            reason: "清单 schema 是空的:不知道这是哪一版".into(),
        };
    }
    if schema != MANIFEST_SCHEMA {
        let nw = value.get("engine").and_then(|n| n.as_str()).unwrap_or("——");
        return ManifestStatus::UnknownSchema {
            guard: crate::engine::manifest_schema_guard(schema, nw),
        };
    }
    match serde_json::from_str::<crate::engine::ManifestBrief>(&raw) {
        Ok(brief) => ManifestStatus::Ok(ManifestArchive {
            brief,
            path: path.display().to_string(),
            raw,
        }),
        Err(e) => ManifestStatus::Broken {
            reason: format!("清单 schema 认得、字段形状不对:{e}"),
        },
    }
}

// ---- 数据仓引用图(spec #94 工单 #116):配方 × 组件目录 ----

/// 引用图里的一处问题(**只报不修**:面板不改文件、不代跑引擎)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefIssue {
    /// 哪类问题。
    pub kind: RefKind,
    /// 出问题的那个组件名(孤儿与断链都指组件,不指配方)。
    pub name: String,
    /// 与 `kind` 配套的**事实**:孤儿给组件目录落点,断链给引用它的配方名。
    /// datarepo 只给事实、不组句——措辞归屏幕层(同 ADR-0010「接缝答读到
    /// 了什么,不答该画多红」)。
    pub fact: String,
}

/// 引用图问题的两类(工单 #116 点名要报的只有这两类)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RefKind {
    /// 组件目录在,却没有一本配方引用它——白养着一个组件。
    Orphan,
    /// 配方引用了一个组件目录不存在的名字——图是断的。
    BrokenLink,
}

/// 引用图扫描结论:**四种,没有第五种**(同 [`ManifestStatus`] 的口径:
/// 分类而不是 `Err`,一处坏不该炸掉整屏)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefScan {
    /// 扫完了,一处问题都没有。
    Clean,
    /// 扫完了,有问题。顺序确定:先断链后孤儿,各自按名字字典序——
    /// 渲染与测试都不靠 `read_dir` 的偶然顺序。
    Findings(Vec<RefIssue>),
    /// 扫不了:数据仓根不在 / 读不通 / 有本配方读不出。
    /// **绝不回 [`RefScan::Clean`]**——「读不到」与「没问题」在屏上是
    /// 两件事(规格故事 28 同款纪律:不拿空结果冒充好消息)。
    Unreadable { reason: String },
    /// 没什么可对照的:一本配方、一个组件都没有(四态之「略」——
    /// 「还没东西可查」不是「查过了没问题」)。
    Empty,
}

/// 扫数据仓的配方 × 组件引用图(只读;面板永不写数据仓)。
///
/// 判据只有两条,都是上游既有事实:
/// ① 每本配方在 `recipes/<名>/recipe.yaml` 的 `components:` 段里点名组件;
/// ② 每个组件是 `components/<名>/` 一个目录。两边对不上就是问题。
///
/// **这是面板自有的检查,不是引擎的回答**:引擎 `ls` 的
/// `components[].usedBy` 是它自己算的同一件事,本方法不读它——面板自己
/// 读盘,才能在 ls 没跑 / 根刚换过时也有一句可说的话。上游对装配的裁定
/// 仍是唯一真相源,本检查只报、不改(工单 #116)。
///
/// `~` 不在这里展开:调用方(`App::data_root_cause`)用同一个原串判可用性,
/// 两边同一把尺才不打架。
pub fn scan_references(root: &str) -> RefScan {
    let base = std::path::Path::new(root);
    // 根本身不在 / 读不通 → 读不通。不往下扫:一个不存在的根会让
    // recipes/ 与 components/ 双双读空,那样的「Empty」是假象。
    if let Err(reason) = dir_readable(base) {
        return RefScan::Unreadable { reason };
    }
    let recipes = match recipe_refs(base) {
        Ok(r) => r,
        Err(reason) => return RefScan::Unreadable { reason },
    };
    let components = component_dirs(base);
    if recipes.is_empty() && components.is_empty() {
        return RefScan::Empty;
    }
    let mut findings = Vec::new();
    // 断链:配方点名的组件没有目录(按配方名 + 组件名字典序,确定)
    for (recipe, names) in &recipes {
        for name in names {
            if !components.iter().any(|c| c == name) {
                findings.push(RefIssue {
                    kind: RefKind::BrokenLink,
                    name: name.clone(),
                    fact: recipe.clone(),
                });
            }
        }
    }
    // 孤儿:组件目录没有进任何配方
    for comp in &components {
        let referenced = recipes
            .iter()
            .any(|(_, names)| names.iter().any(|n| n == comp));
        if !referenced {
            findings.push(RefIssue {
                kind: RefKind::Orphan,
                name: comp.clone(),
                fact: format!("{COMPONENTS_DIR}/{comp}"),
            });
        }
    }
    findings.sort_by(|a, b| {
        // 先断链后孤儿(断链是红的),再按名字
        let rank = |k: &RefKind| match k {
            RefKind::BrokenLink => 0,
            RefKind::Orphan => 1,
        };
        rank(&a.kind)
            .cmp(&rank(&b.kind))
            .then_with(|| a.name.cmp(&b.name))
    });
    if findings.is_empty() {
        RefScan::Clean
    } else {
        RefScan::Findings(findings)
    }
}

/// 每本配方点了哪些组件:`(配方名, 组件名[])`,按配方名字典序。
///
/// 有本配方读不通 → 整个扫描报 `Unreadable`:半张图会造出假孤儿,
/// 宁可说「读不通」也不给错结论。
fn recipe_refs(base: &std::path::Path) -> Result<Vec<(String, Vec<String>)>, String> {
    let dir = base.join(RECIPES_DIR);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new()); // 没有 recipes/ = 一本配方都没有(不是错误)
    };
    let mut entries: Vec<(String, std::path::PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            Some((name, e.path()))
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::new();
    for (name, path) in entries {
        let file = path.join(RECIPE_FILE);
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue, // 不是一本配方
            Err(e) => {
                return Err(format!("配方 {name} 的 {RECIPE_FILE} 读不出:{e}"));
            }
        };
        out.push((name, recipe_component_names(&text)));
    }
    Ok(out)
}

/// 目录能枚举吗(不在 / 不是目录 / 读不通都给出原因)。
fn dir_readable(dir: &std::path::Path) -> Result<(), String> {
    match std::fs::metadata(dir) {
        Ok(md) if md.is_dir() => match std::fs::read_dir(dir) {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("目录 {} 列不出内容:{e}", dir.display())),
        },
        Ok(_) => Err(format!("{} 不是目录", dir.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(format!("目录 {} 不在", dir.display()))
        }
        Err(e) => Err(format!("目录 {} 读不出:{e}", dir.display())),
    }
}

/// 数据仓里的组件目录名(只收目录;散落的文件不算一部组件),字典序。
fn component_dirs(base: &std::path::Path) -> Vec<String> {
    let dir = base.join(COMPONENTS_DIR);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// 从一份 recipe.yaml 里取出 `components:` 段点名的组件名。
///
/// 为什么是手写行扫描:配方是上游契约面(`docs/samples` 的权威形状),面板
/// 不引 YAML 依赖(构建必须离线),也只读这一个键。认两种写法:
/// 块序列(`- component: scout`)与流式(`components: [{component: scout}]`);
/// 认不出的写法当「这本没声明组件」——不猜、不按旧形状解析。
fn recipe_component_names(text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut names: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        // `components:` 只认顶层键(缩进 0):别处的同名键不是配方组件段
        let Some(rest) = lines[i].strip_prefix("components:") else {
            i += 1;
            continue;
        };
        let inline = rest.trim();
        if !inline.is_empty() {
            collect_flow_names(inline, &mut names);
            i += 1;
            continue;
        }
        // 块序列:吃到下一个顶层键(缩进 0 的非空行)为止。段内每一行都过
        // 同一个抽取器:块序列(`- component: scout`)与流式
        // (`- {component: scout, version: 1.0.0}`)对它是一回事。
        i += 1;
        while i < lines.len() {
            let l = lines[i];
            if l.trim().is_empty() {
                i += 1;
                continue;
            }
            if l.len() - l.trim_start().len() == 0 {
                break;
            }
            collect_flow_names(l.trim_start(), &mut names);
            i += 1;
        }
    }
    names
}

/// 把一行里的每个 `component: <名>` 抠出来(块序列与流式同一套)。
///
/// 只认 `component:` 这个键:续行(`version: 1.0.0` / `params: { dir: notes }`)
/// 里没有它,就一个名字都不产——不会把旋钮值当成组件名。
fn collect_flow_names(s: &str, out: &mut Vec<String>) {
    const KEY: &str = "component:";
    let mut rest = s;
    while let Some(pos) = rest.find(KEY) {
        let after = &rest[pos + KEY.len()..];
        let v = after
            .split([',', '}', ']', '#'])
            .next()
            .unwrap_or("")
            .trim();
        let v = v.trim_matches('"').trim_matches('\'').trim();
        if !v.is_empty() {
            out.push(v.to_string());
        }
        rest = after;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一座 fixture 安装根(真目录;按进程 id + 用例名分,免得并行互踩)。
    fn fixture_home(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("nwt-datarepo-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 锁载体落点_绝对安装根拼在locks下() {
        assert_eq!(
            lock_carrier("/nw-home", "9f8e"),
            "/nw-home/locks/9f8e.nwlock"
        );
    }

    #[test]
    fn 锁载体落点_安装根带斜杠尾巴不双写() {
        assert_eq!(
            lock_carrier("/nw-home/", "9f8e"),
            "/nw-home/locks/9f8e.nwlock",
            "尾巴斜杠吃掉一个,不拼出 //"
        );
    }

    #[test]
    fn 锁载体落点_波浪号走同一个展开器() {
        // 展开器是 engine::discover 那一份:本模块不写第二个 ~ 展开。
        let home = expand_tilde("~/.nous-warehouse");
        assert_eq!(
            lock_carrier("~/.nous-warehouse", "9f8e"),
            format!("{}/locks/9f8e.nwlock", home.display())
        );
    }

    #[test]
    fn 安装根在_可用() {
        let home = fixture_home("usable");
        assert_eq!(home_cause(home.to_str().unwrap()), None);
    }

    #[test]
    fn 安装根不在_报缺失而不是空列表() {
        let gone = std::env::temp_dir().join(format!("nwt-datarepo-gone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&gone);
        assert_eq!(
            home_cause(gone.to_str().unwrap()),
            Some(HomeCause::Missing),
            "目录不在与「一个派都还没装」是两件事,不许共用一个空列表"
        );
    }

    #[test]
    fn 安装根是个文件_报读不了() {
        let dir = fixture_home("file");
        let f = dir.join("not-a-dir");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(
            home_cause(f.to_str().unwrap()),
            Some(HomeCause::Unreadable {
                reason: "不是目录".into()
            })
        );
    }

    // ---- 派枚举 ----

    #[test]
    fn 枚举派_只收目录_按字典序() {
        let home = fixture_home("profiles");
        for p in ["zeta", "alpha", "mid"] {
            std::fs::create_dir_all(home.join(PROFILES_DIR).join(p)).unwrap();
        }
        // 散落在 profiles/ 里的文件不算一部派
        std::fs::write(home.join(PROFILES_DIR).join("stray.txt"), "x").unwrap();
        assert_eq!(
            list_profiles(home.to_str().unwrap()),
            vec!["alpha", "mid", "zeta"],
            "顺序确定(字典序),渲染与测试都不靠 read_dir 的偶然顺序"
        );
    }

    #[test]
    fn 枚举派_没有profiles目录时是空() {
        let home = fixture_home("no-profiles");
        assert!(list_profiles(home.to_str().unwrap()).is_empty());
    }

    // ---- 引用图(spec #94 工单 #116):配方 × 组件目录 ----

    /// 造一座 fixture 数据仓(真目录;按进程 id + 用例名分,免得并行互踩)。
    fn fixture_repo(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nwt-datarepo-repo-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 一份配方 fixture:块序列写法,字段与取值照上游权威样本
    /// (`docs/research/nw-journeys-and-docs.md` §3.1 的 `recipes/research-desk/recipe.yaml`)。
    const RECIPE_BLOCK: &str = "apiVersion: nw/v1\nkind: recipe\nname: research-desk\nversion: 1.0.0\nprofile: research-desk\nruntime: { node: \"22.19.0\", pi: \"1.0.2\", chezmoi: \"2.73.0\" }\ncomponents:\n  - component: scout\n    version: 1.0.0\n  - component: retention\n    version: 0.4.1\n    params: { dir: notes }\nmodels:\n  defaults: { defaultProvider: stepfun-sb }\nenv:\n  - { name: TAVILY_API_KEY, required: true, usedBy: [components/scout] }\n";

    fn write_recipe(root: &std::path::Path, name: &str, text: &str) {
        let dir = root.join(RECIPES_DIR).join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(RECIPE_FILE), text).unwrap();
    }

    fn write_component(root: &std::path::Path, name: &str) {
        let dir = root.join(COMPONENTS_DIR).join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("component.yaml"),
            format!("apiVersion: nw/v1\nkind: component\nname: {name}\nversion: 1.0.0\n"),
        )
        .unwrap();
    }

    #[test]
    fn 引用图_组件没进任何配方_报孤儿() {
        let root = fixture_repo("orphan");
        write_recipe(&root, "research-desk", RECIPE_BLOCK);
        write_component(&root, "scout");
        write_component(&root, "retention");
        write_component(&root, "ghost-tools"); // 白养着:没有一本配方点名它
        assert_eq!(
            scan_references(root.to_str().unwrap()),
            RefScan::Findings(vec![RefIssue {
                kind: RefKind::Orphan,
                name: "ghost-tools".into(),
                fact: "components/ghost-tools".into(),
            }]),
            "孤儿给组件名与目录落点,屏层据此组句"
        );
    }

    #[test]
    fn 引用图_配方引用的组件没有目录_报断链() {
        let root = fixture_repo("broken");
        write_recipe(&root, "research-desk", RECIPE_BLOCK);
        write_component(&root, "scout");
        // retention 被配方点名,目录却不在——图是断的
        assert_eq!(
            scan_references(root.to_str().unwrap()),
            RefScan::Findings(vec![RefIssue {
                kind: RefKind::BrokenLink,
                name: "retention".into(),
                fact: "research-desk".into(),
            }]),
            "断链给组件名与引用它的配方名"
        );
    }

    #[test]
    fn 引用图_两种写法都认_流式与块序列同一结论() {
        let root = fixture_repo("flow");
        // 流式写法(上游契约允许的同一形状的另一种排版)
        write_recipe(
            &root,
            "night-shift",
            "components: [{component: scout}, {component: retention}]\n",
        );
        write_component(&root, "scout");
        write_component(&root, "retention");
        assert_eq!(
            scan_references(root.to_str().unwrap()),
            RefScan::Clean,
            "排版不该改变结论:流式与块序列是同一份配方"
        );
    }

    #[test]
    fn 引用图_一本配方一个组件都没有_是没什么可对照() {
        let root = fixture_repo("empty");
        assert_eq!(
            scan_references(root.to_str().unwrap()),
            RefScan::Empty,
            "空仓不是「查过了没问题」:四态之略,与过可区分"
        );
    }

    #[test]
    fn 引用图_有本配方读不出_整扫报读不通_不造假孤儿() {
        let root = fixture_repo("unreadable");
        write_recipe(&root, "research-desk", RECIPE_BLOCK);
        write_component(&root, "scout");
        write_component(&root, "retention");
        // 一本读不通的配方:它点名了什么无从知道,若照常扫就会把两个组件
        // 误报成孤儿——宁可说读不通。用「recipe.yaml 是个目录」造读不通
        // (不靠权限位:测试可能在 root 下跑,chmod 000 拦不住 root)。
        std::fs::create_dir_all(root.join(RECIPES_DIR).join("half-broken").join(RECIPE_FILE))
            .unwrap();
        match scan_references(root.to_str().unwrap()) {
            RefScan::Unreadable { reason } => {
                assert!(
                    reason.contains("half-broken"),
                    "点名读不通的是哪一本:{reason}"
                )
            }
            other => panic!("读不通该报读不通,实得 {other:?}"),
        }
    }

    #[test]
    fn 引用图_数据仓根不在_报读不通_不拿空仓冒充() {
        let gone = std::env::temp_dir().join(format!("nwt-datarepo-nope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&gone);
        match scan_references(gone.to_str().unwrap()) {
            RefScan::Unreadable { .. } => {}
            other => panic!("根不在该报读不通,实得 {other:?}"),
        }
    }

    #[test]
    fn 引用图_配方引用的组件都在_一处问题都没有() {
        let root = fixture_repo("clean");
        write_recipe(&root, "research-desk", RECIPE_BLOCK);
        write_component(&root, "scout");
        write_component(&root, "retention");
        assert_eq!(
            scan_references(root.to_str().unwrap()),
            RefScan::Clean,
            "两头都对得上:没有孤儿、没有断链"
        );
    }

    // ---- 清单存档读取:四种分类 ----

    /// 清单 fixture:字段与取值照引擎真样本
    /// (`docs/real-first-run/envelopes/doctor-profile.json` 的 `data.manifest`),
    /// 不是面板自造的形状。
    const MANIFEST_OK: &str = r#"{"schema":"nw/manifest.v1","lockId":"research-desk-1.0.0-360ae68","profile":"research-desk","recipe":"research-desk","version":"1.0.0","runtimeKey":"node-22.19.0+pi-1.0.2","installedAt":"2026-10-09T03:00:07Z","convergeHash":"3cf6a584a523b35ec42630766a9eb6a84d3dcf11d2347d5aa3f3ae879b420502","engine":"0.1.0"}"#;

    fn write_manifest(home: &std::path::Path, profile: &str, text: &str) {
        let dir = home.join(PROFILES_DIR).join(profile);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), text).unwrap();
    }

    #[test]
    fn 读清单_认得schema时给全字段与落点() {
        let home = fixture_home("manifest-ok");
        write_manifest(&home, "research-desk", MANIFEST_OK);
        match read_manifest(home.to_str().unwrap(), "research-desk") {
            ManifestStatus::Ok(a) => {
                assert_eq!(
                    a.brief.lock_id.as_deref(),
                    Some("research-desk-1.0.0-360ae68")
                );
                assert_eq!(
                    a.brief.installed_at.as_deref(),
                    Some("2026-10-09T03:00:07Z")
                );
                assert_eq!(
                    a.brief.converge_hash.as_deref(),
                    Some("3cf6a584a523b35ec42630766a9eb6a84d3dcf11d2347d5aa3f3ae879b420502")
                );
                assert_eq!(a.brief.engine.as_deref(), Some("0.1.0"));
                assert_eq!(a.brief.recipe.as_deref(), Some("research-desk"));
                assert_eq!(
                    a.brief.runtime_key.as_deref(),
                    Some("node-22.19.0+pi-1.0.2")
                );
                assert!(a.raw.contains("convergeHash"), "原文留着,回车看的就是它");
                assert!(
                    a.path.ends_with("profiles/research-desk/.nw-manifest"),
                    "落点路径由 datarepo 给出,屏幕层不拼:{}",
                    a.path
                );
            }
            other => panic!("认得 schema 该读通,实得 {other:?}"),
        }
    }

    #[test]
    fn 读清单_文件不在报缺失() {
        let home = fixture_home("manifest-missing");
        std::fs::create_dir_all(home.join(PROFILES_DIR).join("fresh")).unwrap();
        assert!(
            matches!(
                read_manifest(home.to_str().unwrap(), "fresh"),
                ManifestStatus::Missing
            ),
            "还没装过的派:缺失是事实,不是错误"
        );
    }

    #[test]
    fn 读清单_坏json报读不通_且带上原因() {
        let home = fixture_home("manifest-bad-json");
        write_manifest(&home, "half", "{\"schema\":\"nw/manifest.v1\",\"lockId\"");
        match read_manifest(home.to_str().unwrap(), "half") {
            ManifestStatus::Broken { reason } => {
                assert!(reason.contains("JSON"), "原因说明读不通在哪:{reason}")
            }
            other => panic!("坏 JSON 该报读不通,实得 {other:?}"),
        }
    }

    #[test]
    fn 读清单_缺schema字段报读不通() {
        let home = fixture_home("manifest-no-schema");
        write_manifest(&home, "noschema", r#"{"lockId":"x-1.0.0-aaa"}"#);
        match read_manifest(home.to_str().unwrap(), "noschema") {
            ManifestStatus::Broken { reason } => {
                assert!(reason.contains("schema"), "原因点名缺的是 schema:{reason}")
            }
            other => panic!("缺 schema 该报读不通,实得 {other:?}"),
        }
    }

    #[test]
    fn 读清单_不认识的schema走契约守卫同款话术() {
        let home = fixture_home("manifest-unknown");
        write_manifest(
            &home,
            "future",
            r#"{"schema":"nw/manifest.v2","lockId":"x-1.0.0-bbb","engine":"9.9.9"}"#,
        );
        match read_manifest(home.to_str().unwrap(), "future") {
            ManifestStatus::UnknownSchema { guard } => {
                assert_eq!(guard.code.as_deref(), Some("SCHEMA_UNKNOWN"));
                assert!(
                    guard.message.contains("需升级 nw"),
                    "与 envelope 契约守卫同款话术:{}",
                    guard.message
                );
                assert!(
                    guard.message.contains("nw/manifest.v2"),
                    "带上收到的 schema 原文:{}",
                    guard.message
                );
                assert_eq!(
                    guard.exit_code, None,
                    "读文件没有引擎退出码可报:这一格留空,不编一个 rc 冒充引擎说的"
                );
                assert!(
                    guard.hint.is_some(),
                    "五字段齐:code/exitCode/message/hint/details"
                );
                assert_eq!(
                    guard
                        .details
                        .as_ref()
                        .and_then(|d| d.get("schema"))
                        .and_then(|s| s.as_str()),
                    Some("nw/manifest.v2"),
                    "details 带原文"
                );
            }
            other => panic!("不认识的 schema 该走契约守卫,实得 {other:?}"),
        }
    }

    #[test]
    fn 读清单_认得schema但字段形状不对也报读不通() {
        let home = fixture_home("manifest-shape");
        // schema 认得,installedAt 却是个对象:形状坏了,但这不是「新版 schema」
        write_manifest(
            &home,
            "weird",
            r#"{"schema":"nw/manifest.v1","installedAt":{"at":1}}"#,
        );
        match read_manifest(home.to_str().unwrap(), "weird") {
            ManifestStatus::Broken { reason } => assert!(reason.contains("字段形状"), "{reason}"),
            other => panic!("形状坏该报读不通,实得 {other:?}"),
        }
    }
}
