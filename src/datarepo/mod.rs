//! 数据仓/安装根访问层(spec #94 票 #101):与引擎层并列、**唯一拥有
//! NW_HOME 与数据仓根文件路径知识**的模块。
//!
//! 分工:引擎层(`crate::engine`)经 runner 的输出与 envelope 的类型看世界;
//! 本模块经文件系统看世界。两者都不下渗到 UI——屏幕层只消费意图级方法
//! (`archives()` / `manifest_raw()`),自己拼不出一个路径,「屏幕层零路径
//! 拼接」由 [`tests::屏幕层零路径_api`] 钉死。
//!
//! 读侧首面 = 装配历史(清单存档,本票)。写侧(spec #95:脚手架/组件清单/
//! 配方写回)随 #109 落地,方法清单在其 ADR 里随接缝契约一并钉死。
//! 已知消费方:#116 引用完整性扫数据仓的配方×组件目录(读侧)。

use std::path::PathBuf;

use serde::Deserialize;

use crate::engine::State;

/// `.nw-manifest` 的 schema 握手值:上游 `internal/manifest/manifest.go`
/// 的 `Schema` 常量,字段集已冻结;不认的 schema 当破损,不按旧形状猜。
const MANIFEST_SCHEMA: &str = "nw/manifest.v1";

/// 安装根(NW_HOME)的读侧句柄:枚举派、读清单存档、取清单原文。
///
/// 路径知识全关在这里:调用方给一句 NW_HOME(设置态的值,可带 `~`),
/// 拿回去的是意图级形状,不是路径。`profiles/` 布局与 `.nw-manifest`
/// 文件名是上游事实(见 [`MANIFEST_SCHEMA`] 注),模块外不再出现第二处。
#[derive(Clone, Debug)]
pub struct InstallRoot {
    root: PathBuf,
}

impl InstallRoot {
    pub fn new(nw_home: &str) -> Self {
        Self {
            root: crate::engine::discover::expand_tilde(nw_home),
        }
    }

    /// 安装根本身在吗(目录可读)。「未设/不存在」与「有根但没派」是两态,
    /// 屏幕层要能分开说(前者指路,后者空列表)。
    pub fn exists(&self) -> bool {
        self.root.is_dir()
    }

    /// 枚举安装根下的派目录名(排序确定:多派并存时列表不跳)。
    pub fn profiles(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.profiles_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        names
    }

    /// 一个派的清单存档(意图级:三态,不带路径)。
    pub fn archive(&self, profile: &str) -> ProfileArchive {
        let Some(dir) = self.profile_dir(profile) else {
            return ProfileArchive {
                profile: profile.to_string(),
                state: ArchiveState::Missing,
            };
        };
        let path = dir.join(MANIFEST_FILE);
        let Ok(bytes) = std::fs::read(&path) else {
            // 不存在 = 上一轮没提交完(上游 `manifest.Load` 的 ErrNotFound
            // 同义:未完成态),与「读得出但坏了」分开说。
            return ProfileArchive {
                profile: profile.to_string(),
                state: ArchiveState::Missing,
            };
        };
        let Ok(text) = String::from_utf8(bytes) else {
            return ProfileArchive {
                profile: profile.to_string(),
                state: ArchiveState::Broken("清单不是 UTF-8 文本".into()),
            };
        };
        match serde_json::from_str::<ManifestFile>(&text) {
            Ok(m) if m.schema == MANIFEST_SCHEMA => ProfileArchive {
                profile: profile.to_string(),
                state: ArchiveState::Ready(ManifestArchive {
                    lock_id: m.lock_id,
                    installed_at: m.installed_at,
                    converge_hash: m.converge_hash,
                    recipe_name: m.recipe.name,
                    recipe_version: m.recipe.version,
                    runtime_key: m.runtime.runtime_key,
                    engine_nw: m.engine.nw,
                    components: m.components.len(),
                }),
            },
            Ok(m) => ProfileArchive {
                profile: profile.to_string(),
                state: ArchiveState::Broken(format!(
                    "清单 schema 不认({}):需升级 nw",
                    if m.schema.is_empty() {
                        "缺 schema 字段"
                    } else {
                        &m.schema
                    }
                )),
            },
            Err(_) => ProfileArchive {
                profile: profile.to_string(),
                state: ArchiveState::Broken("清单不是合法 JSON".into()),
            },
        }
    }

    /// 全部派的存档(按派名排序,与 [`profiles`](Self::profiles) 同序)。
    pub fn archives(&self) -> Vec<ProfileArchive> {
        self.profiles().iter().map(|p| self.archive(p)).collect()
    }

    /// 清单原文(回车看原文用):读不到(缺失态)给 `None`——不开空浮层。
    /// 破损态也給原文:解析失败恰恰是操作者最该看原文的时候。
    pub fn manifest_raw(&self, profile: &str) -> Option<String> {
        let dir = self.profile_dir(profile)?;
        std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()
    }

    fn profiles_dir(&self) -> PathBuf {
        self.root.join(PROFILES_DIR)
    }

    /// 派目录的拼装(本模块唯一的路径构造)。派名来自目录枚举,仍挡
    /// 一层分隔符/`..`:调用方若从别处喂名字,构造不出越界路径。
    fn profile_dir(&self, profile: &str) -> Option<PathBuf> {
        if profile.is_empty() || profile.contains('/') || profile.contains('\\') || profile == ".."
        {
            return None;
        }
        Some(self.profiles_dir().join(profile))
    }
}

/// NW_HOME 布局里的派目录名(上游事实,模块外不出现第二处)。
const PROFILES_DIR: &str = "profiles";
/// 安装根下的锁定集目录(引擎 `ls` 的 locks 域与装配命令交锁段的同一落点)。
const LOCKS_DIR: &str = "locks";
/// 清单落点文件名(上游 `manifest.FileName`:写在 profile 根)。
const MANIFEST_FILE: &str = ".nw-manifest";

/// 一个派的清单存档:派名 + 三态(就绪/缺失/破损)。
/// (`PartialEq`/`Eq` 供测试快照比较,见 `KeySnap`。)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileArchive {
    pub profile: String,
    pub state: ArchiveState,
}

/// 存档三态(工单 #101:「清单缺失/损坏呈降级/破损态」):
/// - `Missing` 缺失 = 上一轮没提交完(未完成态)→ 屏上呈**降级**;
/// - `Broken` 破损 = 读得出但解析不了/schema 不认 → 屏上呈**破损**。
///
/// 人话原因不带路径(路径是接缝内部的事,屏上只摆结论)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArchiveState {
    Ready(ManifestArchive),
    Missing,
    Broken(String),
}

/// 就绪存档的字段(清单冻结字段集的读侧投影:lockId / installedAt /
/// convergeHash + 上下文几项)。组件只留条数——全貌在清单原文浮层。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestArchive {
    pub lock_id: String,
    pub installed_at: String,
    pub converge_hash: String,
    pub recipe_name: String,
    pub recipe_version: String,
    pub runtime_key: String,
    pub engine_nw: String,
    pub components: usize,
}

/// 装配历史次级视图的数据(票 #101):一次打开读一批,屏幕层只消费形状。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryView {
    /// 安装根本身不存在(未设态)——与「有根但一个派都没有」两态可区分:
    /// 前者走指路,后者摆空列表。
    pub root_missing: bool,
    pub rows: Vec<ProfileArchive>,
}

/// 存档三态 → 面板结果灯(工单 #101 的显式映射:缺失=降级、损坏=破损、
/// 就绪=ok)。词汇用 `engine::State` 的同一套灯,健康屏怎么画这里怎么画。
pub fn archive_lamp(state: &ArchiveState) -> State {
    match state {
        ArchiveState::Ready(_) => State::Ok,
        ArchiveState::Missing => State::Degraded,
        ArchiveState::Broken(_) => State::Broken,
    }
}

// ---- 清单文件的读侧投影(只取本票要的字段;形状对齐上游冻结字段集) ----

#[derive(Deserialize)]
struct ManifestFile {
    #[serde(default)]
    schema: String,
    #[serde(default, rename = "lockId")]
    lock_id: String,
    #[serde(default, rename = "installedAt")]
    installed_at: String,
    #[serde(default)]
    recipe: RecipeRefFile,
    #[serde(default)]
    runtime: RuntimeFile,
    /// 只取条数(条目全貌在清单原文浮层里看)。
    #[serde(default)]
    components: Vec<serde_json::Value>,
    #[serde(default, rename = "convergeHash")]
    converge_hash: String,
    #[serde(default)]
    engine: EngineFile,
}

#[derive(Deserialize, Default)]
struct RecipeRefFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
}

#[derive(Deserialize, Default)]
struct RuntimeFile {
    #[serde(default, rename = "runtimeKey")]
    runtime_key: String,
}

#[derive(Deserialize, Default)]
struct EngineFile {
    #[serde(default)]
    nw: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// 每测试一个一次性目录(进程级唯一 + 测试名唯一,并行不互踩)。
    fn fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nwt-datarepo-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 就绪清单(字段集对齐上游 internal/manifest/manifest.go 样本)。
    const MANIFEST_OK: &str = r#"{"schema":"nw/manifest.v1","lockId":"desk-1.0.0-8f3a1c2","installedAt":"2026-10-01T12:00:00Z","recipe":{"name":"research-desk","version":"1.0.0"},"profile":"desk","runtime":{"runtimeKey":"node-24.21.0+pi-1.0.0","node":"24.21.0","pi":"1.0.0","chezmoi":"2.73.0","nodeSource":"bundled"},"components":[{"name":"retention","version":"0.4.1","landing":{"path":"nw/components/retention","kind":"pi-local-package"}},{"name":"scout","version":"0.2.0","landing":{"path":"nw/components/scout","kind":"pi-local-package"}}],"skeletonFiles":["AGENTS.md"],"configKeys":["/settings/theme"],"env":[{"name":"TAVILY_API_KEY","required":true,"present":true}],"convergeHash":"9f2c","engine":{"nw":"0.1.0","schema":"nw/manifest.v1"}}"#;

    fn write_manifest(root: &Path, profile: &str, body: &str) {
        let dir = root.join("profiles").join(profile);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".nw-manifest"), body).unwrap();
    }

    #[test]
    fn 安装根枚举派_按名排序() {
        let root = fixture("profiles");
        write_manifest(&root, "zeta", MANIFEST_OK);
        write_manifest(&root, "alpha", MANIFEST_OK);
        fs::write(root.join("profiles/stray-file"), "x").ok();
        let ir = InstallRoot::new(root.to_str().unwrap());
        assert_eq!(ir.profiles(), vec!["alpha", "zeta"], "只收目录、按名排序");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 清单存档_就绪态字段齐全() {
        let root = fixture("ready");
        write_manifest(&root, "desk", MANIFEST_OK);
        let ir = InstallRoot::new(root.to_str().unwrap());
        let a = ir.archive("desk");
        assert_eq!(a.profile, "desk");
        match &a.state {
            ArchiveState::Ready(m) => {
                assert_eq!(m.lock_id, "desk-1.0.0-8f3a1c2");
                assert_eq!(m.installed_at, "2026-10-01T12:00:00Z");
                assert_eq!(m.converge_hash, "9f2c");
                assert_eq!(m.recipe_name, "research-desk");
                assert_eq!(m.recipe_version, "1.0.0");
                assert_eq!(m.runtime_key, "node-24.21.0+pi-1.0.0");
                assert_eq!(m.engine_nw, "0.1.0");
                assert_eq!(m.components, 2, "组件条数");
            }
            other => panic!("应就绪,实得 {other:?}"),
        }
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 清单缺失_降级态() {
        let root = fixture("missing");
        fs::create_dir_all(root.join("profiles/desk")).unwrap(); // 派目录在,清单不在
        let ir = InstallRoot::new(root.to_str().unwrap());
        assert!(
            matches!(ir.archive("desk").state, ArchiveState::Missing),
            "缺清单 = 上一轮没提交完的未完成态"
        );
        assert_eq!(archive_lamp(&ArchiveState::Missing), State::Degraded);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 清单坏json_破损态() {
        let root = fixture("broken-json");
        write_manifest(&root, "desk", "{not json");
        let ir = InstallRoot::new(root.to_str().unwrap());
        match &ir.archive("desk").state {
            ArchiveState::Broken(why) => assert!(why.contains("JSON"), "人话点题:{why}"),
            other => panic!("应破损,实得 {other:?}"),
        }
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 清单schema不认_破损态() {
        let root = fixture("broken-schema");
        let m = MANIFEST_OK.replace("nw/manifest.v1", "nw/manifest.v2");
        write_manifest(&root, "desk", &m);
        let ir = InstallRoot::new(root.to_str().unwrap());
        match &ir.archive("desk").state {
            ArchiveState::Broken(why) => {
                assert!(why.contains("schema"), "点题:{why}");
                assert!(why.contains("nw/manifest.v2"), "带上收到的 schema:{why}");
                assert!(!why.contains("profiles"), "破损原因不带路径:{why}");
            }
            other => panic!("应破损,实得 {other:?}"),
        }
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 多派三态并存_逐个不互窜() {
        let root = fixture("mixed");
        write_manifest(&root, "desk", MANIFEST_OK);
        fs::create_dir_all(root.join("profiles/lab")).unwrap(); // 缺失
        write_manifest(&root, "broken", "{oops"); // 破损
        let ir = InstallRoot::new(root.to_str().unwrap());
        let got: Vec<(String, bool, bool)> = ir
            .archives()
            .into_iter()
            .map(|a| {
                (
                    a.profile,
                    matches!(a.state, ArchiveState::Ready(_)),
                    matches!(a.state, ArchiveState::Broken(_)),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("broken".into(), false, true),
                ("desk".into(), true, false),
                ("lab".into(), false, false),
            ],
            "三态各归各,按派名排序"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 清单原文_读得到与读不到() {
        let root = fixture("raw");
        write_manifest(&root, "desk", MANIFEST_OK);
        fs::create_dir_all(root.join("profiles/lab")).unwrap();
        let ir = InstallRoot::new(root.to_str().unwrap());
        assert_eq!(ir.manifest_raw("desk").as_deref(), Some(MANIFEST_OK));
        assert_eq!(ir.manifest_raw("lab"), None, "缺失态没有原文,不开空浮层");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 安装根不存在_与有根没派两态可区分() {
        let root = fixture("no-root");
        let ir = InstallRoot::new(root.join("nope").to_str().unwrap());
        assert!(!ir.exists());
        assert!(ir.profiles().is_empty());
        assert!(ir.archives().is_empty());
        // 有根没派:根在、派表空——两态在 exists() 上分开
        fs::create_dir_all(&root).unwrap();
        let ir2 = InstallRoot::new(root.to_str().unwrap());
        assert!(ir2.exists());
        assert!(ir2.profiles().is_empty());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 派名带分隔符_构造不出越界路径() {
        let root = fixture("traversal");
        let ir = InstallRoot::new(root.to_str().unwrap());
        for bad in ["../etc", "a/b", "a\\b", ".."] {
            assert!(
                matches!(ir.archive(bad).state, ArchiveState::Missing),
                "{bad} 读不到任何东西"
            );
            assert_eq!(ir.manifest_raw(bad), None);
        }
        fs::remove_dir_all(&root).ok();
    }

    /// 接缝纪律(工单 #101 验收):屏幕层零路径 API——装配屏渲染只消费
    /// `HistoryView` 形状,不知道派目录怎么拼、清单文件叫什么。这里扫
    /// 屏幕源码钉死:路径知识只有 datarepo 一个家。
    #[test]
    fn 屏幕层零路径_api() {
        let src = include_str!("../screens/assemble.rs");
        for needle in ["PathBuf", "Path::new", PROFILES_DIR] {
            assert!(
                !src.contains(needle),
                "装配屏不得出现 `{needle}`:NW_HOME 路径拼接归 datarepo(接缝纪律)"
            );
        }
    }
}

/// 选中锁定集的载体文件路径:`$NW_HOME/locks/<lockId>.nwlock`(引擎 `ls` 的
/// locks 域与装配命令交锁段的同一落点)。`~` 展开走全仓唯一那份展开器。
pub fn lock_carrier(nw_home: &str, lock_id: &str) -> String {
    let home = crate::engine::discover::expand_tilde(nw_home);
    format!(
        "{}/{LOCKS_DIR}/{lock_id}.nwlock",
        home.display().to_string().trim_end_matches('/')
    )
}
