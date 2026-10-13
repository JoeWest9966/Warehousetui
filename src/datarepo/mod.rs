//! 数据仓/安装根访问层(`datarepo`,spec #94 工单 #101 立)。
//!
//! 与 [`crate::engine`] 并列的**第二条访问层**:引擎层是「调 `nw` 子进程」的
//! 唯一接缝,本模块是「不调引擎、直接读文件」的唯一接缝。两者分工不混——
//! 读文件的事(清单存档 / 回收站 / 派枚举 / 日志存档 / 版本对照的数据面)
//! 一律问本模块,屏幕层与 app 层不自己拼 `$NW_HOME` 路径。
//!
//! 本模块只拥有**路径布局知识**(上游既有事实,面板只读不写):
//! `$NW_HOME/profiles/<派>/.nw-manifest`、`locks/`、`pruned/<时间戳>/`、
//! `logs/`。布局变更由「跟随引擎」纪律兜住(ADR-0007):上游改了,这里改,
//! 屏幕层一个字不动。
//!
//! **对外只暴露意图级方法**:调用方说「要什么」,不说「去哪个目录取哪个
//! 文件」。返回值是**分类**(读到了 / 不在 / 读不通 / 不认识),不是
//! `Err`——一条坏记录不该炸掉整屏(规格故事 4)。
//!
//! 接缝契约(方法清单与「谁可以调它」)钉在 `docs/adr/0010-*.md`;本票
//! (#101)只落地清单存档读取一侧,回收站 / 日志 / 派总览 / 版本对照的
//! 方法由各自工单补,不在这里留占位 stub。

use std::path::PathBuf;

use crate::engine::discover::expand_tilde;

/// 清单文件名(上游 `internal/manifest` 的原子提交点,永远最后写)。
pub const MANIFEST_FILE: &str = ".nw-manifest";

/// 面板唯一认得的清单 schema(上游既有事实;不认识的走契约守卫同款
/// 「需升级 nw」话术,不按旧形状降级解析)。
pub const MANIFEST_SCHEMA: &str = "nw/manifest.v1";

/// 内嵌锁定集 schema(上游 `nw/lock.v1`:bake 的 `data.lock` 与
/// `$NW_HOME/locks/*.nwlock` 同一个形状)。
pub const LOCK_SCHEMA: &str = "nw/lock.v1";

/// 安装根下的派目录(每个派一部,清单住在其根部)。
pub const PROFILES_DIR: &str = "profiles";

/// 安装根下的锁定集目录(引擎 `ls` 的 locks 域与装配命令交锁段的同一落点)。
pub const LOCKS_DIR: &str = "locks";

/// 安装根下的回收站目录(引擎把换下的旧组件**整目录移进来**,只移不删;
/// `--allow-prune` 才真删)。上游布局见 `docs/research/nw-cli-surface.md` §4:
/// `pruned/<时间戳>/` 住在**安装根根部**,不在某个派底下——面板不按派另造一层。
pub const PRUNED_DIR: &str = "pruned";

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

/// 一份锁定集存档(#104):面板从 `nw/lock.v1` 里只取**组件名**。
///
/// 为什么只取组件名:发射前 diff 要答的是「这次会改变什么」,而组件名集合
/// 是唯一两侧都有的可比面(见 [`read_lock`]。sha256 / landing / packages
/// 一概不取——面板不拿它们做任何判断,取了就是第二真相源。
///
/// 同 [`ManifestArchive`]:不派生相等性(内嵌 `Vec<String>` 虽可比,但
/// 与另一侧的读取结论并排时没有比较的意义,测试按字段断言)。
#[derive(Clone, Debug)]
pub struct LockArchive {
    /// 这份锁定集的目标派(`nw/lock.v1` 的 `profile`)。发射前 diff 靠它
    /// 指到「该看哪个派的清单」——`ls` 的锁行不一定带这个字段(真样本
    /// 只有 lockId / recipe / bakedAt / assets),所以以文件为准。
    pub profile: Option<String>,
    /// 组件名,**照文件里的顺序**(引擎写盘确定性:同输入重烤同锁,
    /// 上游 `nw/lock.v1` 文档明写「同 lock 连跑两次 manifest 逐字节
    /// 一致」)。集合差按名比,呈现顺序各侧保持原样——不重排,
    /// 免得把引擎给的顺序悄悄换成面板的口味。
    pub components: Vec<String>,
    /// 文件落点(只读呈现;面板不写)。
    pub path: String,
}

/// 锁定集存档的读取结论:**四种分类,没有第五种**(与
/// [`ManifestStatus`] 同一套判词,屏幕层一处映射两种结论)。
#[derive(Clone, Debug)]
pub enum LockStatus {
    /// 认得也读通了。
    Ok(LockArchive),
    /// 文件不在:锁定集被清过,或压根没 bake 过。
    Missing,
    /// 文件在但读不通:不是合法 JSON(载体也可能是装配命令 sh 脚本),
    /// 或缺 `schema` 字段。
    Broken { reason: String },
    /// JSON 合法、`schema` 不认得:契约守卫同款「需升级 nw」五字段证据。
    UnknownSchema { guard: crate::engine::Evidence },
}

/// 读一份锁定集存档(`$NW_HOME/locks/<lockId>.nwlock`,只读)。
///
/// 落点复用 [`lock_carrier`](self::lock_carrier) 那一份(面板按
/// 引擎 `ls` 的 locks 域同一落点把 lockId 还原成文件);本方法不拼
/// 第二个路径形状。
///
/// #104 的两侧对比面
///
/// 上游契约:`nw/manifest.v1`(`.nw-manifest`)**不记组件**——它只有
/// `lockId` / `recipe` / `convergeHash` 等摘要字段(真样本见
/// `docs/real-first-run/envelopes/doctor-profile.json` 的 `data.manifest`);
/// 组件集合只住在 `nw/lock.v1` 里。所以「旧态装了哪些组件」这一问,
/// 文件面上唯一的答案是**清单自己记的那个 lockId 对应的锁定集文件**
/// (`locks/<manifest.lockId>.nwlock`)——本方法就是为读它而立的第二侧。
/// 那不是引擎的收敛逻辑,只是把两份文件各读一遍再按名做集合差。
pub fn read_lock(nw_home: &str, lock_id: &str) -> LockStatus {
    let path = std::path::PathBuf::from(lock_carrier(nw_home, lock_id));
    let raw = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LockStatus::Missing,
        Err(e) => {
            return LockStatus::Broken {
                reason: format!("锁定集读不出:{e}"),
            }
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            return LockStatus::Broken {
                reason: format!("锁定集不是合法 JSON:{e}"),
            }
        }
    };
    let Some(schema) = value.get("schema").and_then(|s| s.as_str()) else {
        return LockStatus::Broken {
            reason: "锁定集缺 schema 字段:不知道这是哪一版".into(),
        };
    };
    if schema.trim().is_empty() {
        return LockStatus::Broken {
            reason: "锁定集 schema 是空的:不知道这是哪一版".into(),
        };
    }
    if schema != LOCK_SCHEMA {
        let nw = value.get("engine").and_then(|n| n.as_str()).unwrap_or("——");
        return LockStatus::UnknownSchema {
            guard: crate::engine::manifest_schema_guard(schema, nw),
        };
    }
    // 只取组件名:缺 components 键按空数组(上游 omitempty 的老锁定集
    // 也可能没有它),形状坏了才报读不通——空与坏是两件事。
    let components = value
        .get("components")
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("name").and_then(|n| n.as_str()))
                .map(str::to_string)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    LockStatus::Ok(LockArchive {
        profile: value
            .get("profile")
            .and_then(|p| p.as_str())
            .map(str::to_string),
        components,
        path: path.display().to_string(),
    })
}
/// 回收站里的一条:一次退场的一座旧组件(#102)。
///
/// 上游事实(`docs/research/nw-cli-surface.md` §4、`nw-journeys-and-docs.md`):
/// `nw up` 收敛到新锁时,被换下的旧组件**整目录移入** `$NW_HOME/pruned/<时间戳>/`,
/// 只移不删(`--allow-prune` 才真删)。所以一条 = 一个「退场时刻 × 组件名」的
/// 二元组,外加面板唯一能负责任地给出的东西:**完整恢复路径**(选中即复制它,
/// 面板不代恢复——规格故事 6/7)。
///
/// 不派生相等性:与 [`ManifestArchive`] 同一理由(测试按字段断言)。
#[derive(Clone, Debug)]
pub struct PrunedEntry {
    /// 退场时刻目录名(引擎写的原样串,面板不改写、不解析成日期)。
    pub stamp: String,
    /// 组件名(该时刻目录下的目录名 / 文件名)。
    pub component: String,
    /// 完整恢复路径:操作者拿它去 shell 里手工找回的地址。
    pub path: String,
}

/// 枚举回收站里被换下的旧组件(只读;#102)。
///
/// **不按派过滤**:上游 `pruned/<时间戳>/` 住在安装根根部,不在某个派底下
/// (见 [`PRUNED_DIR`]);面板不按派另造一层布局。「从哪一派的视角翻进来」
/// 是视图层的事(`App` 记下来历派),枚举这一层只答「回收站里有什么」。
///
/// 可用性另问 [`home_cause`]:安装根本身不在时走指路,不拿一个空列表冒充
/// 「从没换下过组件」(规格故事 28)。而 `pruned/` 不在 / 读不通 / 是文件
/// 都归**空**:那确实是「没有可恢复的东西」,不是错误(规格验收:空/缺失
/// 目录呈空态不报错)。
///
/// 顺序确定(字典序,stamp 先、组件名后):与 [`list_profiles`] 同一口径,
/// 渲染与测试都不靠 `read_dir` 的偶然顺序。
pub fn list_pruned(nw_home: &str) -> Vec<PrunedEntry> {
    let root = expand_tilde(nw_home).join(PRUNED_DIR);
    let Ok(rd) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut stamps: Vec<String> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    stamps.sort();
    let mut out: Vec<PrunedEntry> = Vec::new();
    for stamp in stamps {
        let dir = root.join(&stamp);
        let Ok(inner) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut comps: Vec<String> = inner
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        comps.sort();
        for component in comps {
            out.push(PrunedEntry {
                stamp: stamp.clone(),
                path: dir.join(&component).display().to_string(),
                component,
            });
        }
    }
    out
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

    /// 往 `locks/` 里写一份 `.nwlock`(字段照引擎 bake 的 `data.lock`
    /// 真样本:见 `docs/real-first-run/envelopes/bake.json`)。
    fn write_lock(home: &std::path::Path, lock_id: &str, text: &str) {
        let dir = home.join(LOCKS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{lock_id}.nwlock")), text).unwrap();
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

    // ---- 回收站枚举:被换下的旧组件 ----

    /// 造一座 fixture 回收站(#102):两个退场时刻目录,组件名照上游事实取
    /// (整目录移入,所以组件也是目录)。
    fn fixture_pruned(tag: &str) -> std::path::PathBuf {
        let home = fixture_home(tag);
        let stamps: Vec<(&str, Vec<&str>)> = vec![
            (
                "20261009T030007Z",
                vec!["retention", "satoshi-localeselector"],
            ),
            ("20261012T101500Z", vec!["hyva-ai-tools"]),
        ];
        for (stamp, comps) in stamps {
            for c in comps {
                std::fs::create_dir_all(home.join(PRUNED_DIR).join(stamp).join(c)).unwrap();
            }
        }
        home
    }

    #[test]
    fn 回收站枚举_时间戳目录与组件名齐_顺序确定() {
        let home = fixture_pruned("pruned");
        let got = list_pruned(home.to_str().unwrap());
        let pairs: Vec<(&str, &str)> = got
            .iter()
            .map(|e| (e.stamp.as_str(), e.component.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("20261009T030007Z", "retention"),
                ("20261009T030007Z", "satoshi-localeselector"),
                ("20261012T101500Z", "hyva-ai-tools"),
            ],
            "顺序确定(字典序):渲染与测试都不靠 read_dir 的偶然顺序"
        );
        assert!(
            got[0].path.ends_with("pruned/20261009T030007Z/retention"),
            "完整恢复路径由 datarepo 给出,屏幕层不拼:{}",
            got[0].path
        );
    }

    #[test]
    fn 回收站_散落文件不算一条_只收时间戳目录下的组件() {
        let home = fixture_pruned("pruned-stray");
        // 落在 pruned/ 根部的散落文件不是一次退场,更不是组件
        std::fs::write(home.join(PRUNED_DIR).join("stray.txt"), "x").unwrap();
        let got = list_pruned(home.to_str().unwrap());
        assert_eq!(got.len(), 3, "散落文件不该混进回收站:{got:?}");
    }

    #[test]
    fn 回收站_pruned不在时是空_不报错() {
        let home = fixture_home("no-pruned");
        assert!(
            list_pruned(home.to_str().unwrap()).is_empty(),
            "从没换下过组件:空是事实,不是错误"
        );
    }

    #[test]
    fn 回收站_pruned是个文件时也是空_不炸() {
        let home = fixture_home("pruned-file");
        std::fs::write(home.join(PRUNED_DIR), "x").unwrap();
        assert!(list_pruned(home.to_str().unwrap()).is_empty());
    }

    #[test]
    fn 回收站_空的退场目录不产条目() {
        let home = fixture_home("pruned-empty-stamp");
        std::fs::create_dir_all(home.join(PRUNED_DIR).join("20261013T000000Z")).unwrap();
        assert!(
            list_pruned(home.to_str().unwrap()).is_empty(),
            "一次退场目录里没有组件:没有可恢复的地址可给"
        );
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

    // ---- #104 发射前 diff 的锁定集一侧 ----

    /// `nw/lock.v1` 夹具(字段与取值照 bake.json 的 `data.lock` 真样本;
    /// 两个组件 + env 三键,env 只用于证明「读组件名不碰 env 值」)。
    const LOCK_OK: &str = r#"{"schema":"nw/lock.v1","lockId":"research-desk-1.0.0-360ae68","bakedAt":"2026-10-09T02:52:51Z","recipe":{"name":"research-desk","version":"1.0.0"},"profile":"research-desk","runtime":{"node":"22.19.0","pi":"1.0.2","chezmoi":"2.73.0"},"components":[{"name":"scout","version":"1.0.0","landing":{"path":"<agent-dir>/nw/components/scout","kind":"pi-local-package"},"packages":[],"sha256":"d34c"},{"name":"retention","version":"0.4.1","landing":{"path":"<agent-dir>/nw/components/retention","kind":"pi-local-package"},"packages":[],"sha256":"b01b"}],"env":[{"name":"STEPFUN_API_KEY","required":false,"usedBy":["models.providers.stepfun-sb"],"purpose":""},{"name":"TAVILY_API_KEY","required":true,"usedBy":["components/scout"],"purpose":""}],"assets":[]}"#;

    #[test]
    fn 读锁定集_认得schema时给组件名与落点() {
        let home = fixture_home("lock-ok");
        write_lock(&home, "research-desk-1.0.0-360ae68", LOCK_OK);
        match read_lock(home.to_str().unwrap(), "research-desk-1.0.0-360ae68") {
            LockStatus::Ok(l) => {
                // 组件名照文件顺序(引擎写盘确定性:同输入重烤同锁),不重排
                assert_eq!(l.components, vec!["scout", "retention"]);
                assert_eq!(l.profile.as_deref(), Some("research-desk"));
                assert!(l.path.ends_with("locks/research-desk-1.0.0-360ae68.nwlock"));
            }
            other => panic!("认得该读到,实得 {other:?}"),
        }
    }

    #[test]
    fn 读锁定集_文件不在报缺失() {
        let home = fixture_home("lock-gone");
        assert!(
            matches!(
                read_lock(home.to_str().unwrap(), "9f8e"),
                LockStatus::Missing
            ),
            "旧锁定集文件被清过:与「一个组件都没有」不是一件事"
        );
    }

    #[test]
    fn 读锁定集_不是合法json报读不通() {
        let home = fixture_home("lock-bad");
        // 载体也可能是装配命令(sh 脚本):那不是 nw/lock.v1,报读不通不猜
        write_lock(&home, "cmd", "#!/bin/sh\nnw up --profile desk\n");
        match read_lock(home.to_str().unwrap(), "cmd") {
            LockStatus::Broken { reason } => assert!(reason.contains("JSON"), "{reason}"),
            other => panic!("装配命令不是锁定集 JSON,该报读不通,实得 {other:?}"),
        }
    }

    #[test]
    fn 读锁定集_不认识的schema走契约守卫同款话术() {
        let home = fixture_home("lock-v2");
        write_lock(
            &home,
            "future",
            r#"{"schema":"nw/lock.v2","lockId":"x","engine":"9.9.9","components":[]}"#,
        );
        match read_lock(home.to_str().unwrap(), "future") {
            LockStatus::UnknownSchema { guard } => {
                assert!(guard.message.contains("需升级"), "{}", guard.message)
            }
            other => panic!("不认识的 schema 该走契约守卫,实得 {other:?}"),
        }
    }

    #[test]
    fn 读锁定集_缺schema字段报读不通() {
        let home = fixture_home("lock-noschema");
        write_lock(&home, "noschema", r#"{"lockId":"x","components":[]}"#);
        match read_lock(home.to_str().unwrap(), "noschema") {
            LockStatus::Broken { reason } => assert!(reason.contains("schema"), "{reason}"),
            other => panic!("缺 schema 不知道是哪一版,实得 {other:?}"),
        }
    }
}
