//! 数据仓/安装根访问层(`datarepo`,spec #94 工单 #101 立)。
//!
//! 与 [`crate::engine`] 并列的**第二条访问层**:引擎层是「调 `nw` 子进程」的
//! 唯一接缝,本模块是「不调引擎、直接读文件」的唯一接缝。两者分工不混——
//! 读文件的事(清单存档 / 回收站 / 派枚举 / 日志存档 / 版本对照的数据面)
//! 一律问本模块,屏幕层与 app 层不自己拼 `$NW_HOME` 路径。
//!
//! **唯一的写例外**(ADR-0010 §6,spec #94 工单 #103):装配 stderr 的旁路
//! 副本落盘在 `$NW_HOME/logs/<lockId>/`。那是日志,不是状态——面板永不写
//! `.nw-manifest`(上游把它当唯一提交点、永远最后写),作者侧 spec #95 也永不
//! 写 NW_HOME。例外只有这一处,且必须在 ADR 里记账。
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

/// 安装根下的派目录(每个派一部,清单住在其根部)。
pub const PROFILES_DIR: &str = "profiles";

/// 安装根下的锁定集目录(引擎 `ls` 的 locks 域与装配命令交锁段的同一落点)。
pub const LOCKS_DIR: &str = "locks";

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

/// 装配日志目录名(安装根下;ADR-0010 §6 的写例外落点)。
pub const LOGS_DIR: &str = "logs";

/// 存档日志文件的扩展名(确定命名的一部分,不是可配置项)。
pub const LOG_SUFFIX: &str = ".log";

/// 某次装配的存档日志目录:`$NW_HOME/logs/<lockId>/`
/// (ADR-0010 §6:按 lockId 组织,一台安装根下每次装配一个目录)。
///
/// `~` 走 [`expand_tilde`] 那一份展开器;尾巴斜杠吃不掉一个,与
/// [`lock_carrier`] 同一条纪律(不拼出 `//`)。
pub fn log_dir(nw_home: &str, lock_id: &str) -> PathBuf {
    expand_tilde(nw_home).join(LOGS_DIR).join(lock_id)
}

/// 某次装配的存档日志文件:`$NW_HOME/logs/<lockId>/<lockId>.log`
///
/// **确定命名**:目录与文件名都由 lockId 决定,一个时钟都不看——所以
/// ① 同一条装配命令重跑,落点是同一个文件(追加,不另起一个);
/// ② 操作者在 shell 里 `cat $NW_HOME/logs/<lockId>/<lockId>.log` 直接找得到,
///  glob `logs/*/*.log` 也认得出每一份属于哪一条锁(规格故事 19)。
/// 时间戳方案(规格给的另一个选项)被否:它让「哪一份是刚才那次」要靠
/// 排序猜,而重跑同一条锁时会攒下一串分不清先后的文件。
pub fn log_path(nw_home: &str, lock_id: &str) -> PathBuf {
    let mut p = log_dir(nw_home, lock_id);
    p.push(format!("{lock_id}{LOG_SUFFIX}"));
    p
}

/// 装配 stderr 的落盘 tee(#103):开一次文件、逐行追加、显式收尾。
///
/// **追加语义,不是「结束时原子写」**:中止(进程级真止)时文件里必须留着
/// 已经收到的那几行——操作者判断「上次装配停在哪一步」的唯一依据就是它
/// (与内存里 `as_log` 中止不清空同一条纪律,ADR-0010 §6)。原子写只在
/// **收尾**时发生:写坏一行不留半个文件(见 [`LogTee::line`])。
///
/// 目录不存在就建(`create_dir_all`):这是面板唯一被准许写 NW_HOME 的地方,
/// 建的是 `logs/<lockId>/` 这一棵,不碰 `profiles/` / `locks/` / `pruned/`。
pub struct LogTee {
    file: std::fs::File,
    /// 落点(只为收尾报告与测试断言;不参与写)。
    path: PathBuf,
}

impl LogTee {
    /// 为某条锁开一个 tee(追加模式;文件不在则建)。
    ///
    /// **安装根本身不在/读不了时不开**:写例外只许写 `logs/<lockId>/` 这一棵,
    /// 不许 `create_dir_all` 顺手造出一座安装根(规格故事 28 的写侧对应:
    /// 不出现拼错路径、也不凭空造目录的假象)。调用方据此走既有指路语义。
    pub fn open(nw_home: &str, lock_id: &str) -> std::io::Result<Self> {
        if let Some(cause) = home_cause(nw_home) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("安装根不可用({cause:?}):不凭空造 logs/ 树"),
            ));
        }
        let path = log_path(nw_home, lock_id);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        Ok(Self { file, path })
    }

    /// 追加一行 stderr。
    ///
    /// **一行一写、写完即 flush**:进程被 kill 的瞬间,已 flush 的行在盘上,
    /// 没写完的那半行不在——不会留下一个「看起来是完整一行、其实被腰斩」的
    /// 文件。测试据此断言「中止不留半个文件」。
    pub fn line(&mut self, line: &str) -> std::io::Result<()> {
        use std::io::Write;
        // 只写这一行 + 换行:一行一次 write_all,不跨行缓冲。
        self.file.write_all(line.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        Ok(())
    }

    /// 落点(回读与报告用)。
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

/// 存档日志的读取结论(**三种分类,没有第四种**):与
/// [`ManifestStatus`] 同口径——一条读不通的存档只坏它自己,不炸回看浮层。
#[derive(Clone, Debug)]
pub enum LogArchive {
    /// 读到了:正文 + 落点。
    Ok { path: String, body: String },
    /// 没有这一份(那条锁从没在面板里跑过装配,或日志被清过)。
    Missing,
    /// 文件在但读不通(权限 / IO)。
    Broken { reason: String },
}

/// 读某条锁的存档日志(只读;回看浮层的数据面,#103)。
///
/// 空文件是 `Ok { body: "" }` 而不是 `Missing`:「有一份、里面没有行」与
/// 「没有这一份」在屏上是两件事(与清单的 Missing / Ok 分野同口径)。
pub fn read_log(nw_home: &str, lock_id: &str) -> LogArchive {
    let path = log_path(nw_home, lock_id);
    match std::fs::read_to_string(&path) {
        Ok(body) => LogArchive::Ok {
            path: path.display().to_string(),
            body,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => LogArchive::Missing,
        Err(e) => LogArchive::Broken {
            reason: format!("存档日志读不出:{e}"),
        },
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

    // ---- #103 装配日志落盘与回看 ----

    /// 装配 stderr 的夹具:形状照引擎真样本的口径——七步人话 + env **布尔**
    /// (引擎纪律:env 只报 present,本体只从 stdin 进,stderr 不含机密)。
    const UP_STDERR: &str =
        "铺骨架…\n落组件 retention\nenv present: GH_TOKEN\n装配完成:convergeHash 3cf6a584";

    #[test]
    fn 落盘_按lock_id确定命名_shell里直接找得到() {
        let home = fixture_home("log-path");
        assert_eq!(
            log_path(home.to_str().unwrap(), "9f8e"),
            home.join(LOGS_DIR).join("9f8e").join("9f8e.log"),
            "目录与文件名都由 lockId 决定,一个时钟都不看"
        );
        assert_eq!(
            log_path("/nw-home/", "9f8e"),
            std::path::PathBuf::from("/nw-home/logs/9f8e/9f8e.log"),
            "尾巴斜杠吃不掉一个,不拼出 //"
        );
    }

    #[test]
    fn 落盘_逐行追加_每行写完即落盘() {
        let home = fixture_home("log-write");
        let mut tee = LogTee::open(home.to_str().unwrap(), "9f8e").unwrap();
        for l in UP_STDERR.lines() {
            tee.line(l).unwrap();
        }
        // 不调 finish 就读:tear-down 之前盘上必须已经是全部四行
        // (「写完即落盘」是中止场景的依据,不能等到收尾)。
        let body = std::fs::read_to_string(tee.path()).unwrap();
        assert_eq!(
            body,
            format!("{UP_STDERR}\n"),
            "逐行原样落盘,一行不多一行不少"
        );
    }

    #[test]
    fn 落盘_同一条锁重跑是追加_不另起一个文件() {
        let home = fixture_home("log-append");
        {
            let mut tee = LogTee::open(home.to_str().unwrap(), "9f8e").unwrap();
            tee.line("第一程:铺骨架…").unwrap();
        }
        {
            let mut tee = LogTee::open(home.to_str().unwrap(), "9f8e").unwrap();
            tee.line("第二程:铺骨架…").unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(log_path(home.to_str().unwrap(), "9f8e")).unwrap(),
            "第一程:铺骨架…\n第二程:铺骨架…\n",
            "确定命名 ⇒ 重跑同一条锁落在同一个文件上,两程都在(日志不丢)"
        );
    }

    #[test]
    fn 读回_读到了给正文与落点() {
        let home = fixture_home("log-read");
        {
            let mut tee = LogTee::open(home.to_str().unwrap(), "9f8e").unwrap();
            for l in UP_STDERR.lines() {
                tee.line(l).unwrap();
            }
        }
        match read_log(home.to_str().unwrap(), "9f8e") {
            LogArchive::Ok { path, body } => {
                assert!(
                    body.contains("env present: GH_TOKEN"),
                    "stderr 流原样在:{body}"
                );
                assert!(
                    path.ends_with("logs/9f8e/9f8e.log"),
                    "落点由 datarepo 给出,屏幕层不拼:{path}"
                );
            }
            other => panic!("写过的存档该读得到,实得 {other:?}"),
        }
    }

    #[test]
    fn 读回_从没跑过这条锁报缺失_与空文件是两件事() {
        let home = fixture_home("log-missing");
        assert!(
            matches!(
                read_log(home.to_str().unwrap(), "9f8e"),
                LogArchive::Missing
            ),
            "没有这一份:不是错误,是事实"
        );
        // 建了目录但文件不在:同样是缺失(建目录不等于有日志)
        std::fs::create_dir_all(log_dir(home.to_str().unwrap(), "9f8e")).unwrap();
        assert!(matches!(
            read_log(home.to_str().unwrap(), "9f8e"),
            LogArchive::Missing
        ));
        // 空文件:Ok + 空正文——「有一份、里面没有行」不与「没有这一份」混淆
        std::fs::write(log_path(home.to_str().unwrap(), "9f8e"), "").unwrap();
        match read_log(home.to_str().unwrap(), "9f8e") {
            LogArchive::Ok { body, .. } => assert!(body.is_empty(), "空文件是 Ok,不是 Missing"),
            other => panic!("空文件该是 Ok,实得 {other:?}"),
        }
    }

    #[test]
    fn 落盘_安装根不在时不建目录_不凭空造一棵树() {
        // 写例外的边界:NW_HOME 不在时面板不 create_dir_all 出一座安装根
        // (规格故事 28:不出现拼错路径读空目录的假象;写也一样不许凭空造)。
        let gone = std::env::temp_dir().join(format!("nwt-log-gone-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&gone);
        assert!(
            LogTee::open(gone.to_str().unwrap(), "9f8e").is_err(),
            "安装根不在:开不出 tee(调用方据此走指路,不写盘)"
        );
        assert!(!gone.exists(), "更不许顺手把安装根建出来");
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
