//! 数据仓写侧(spec #95 工单 #109):与读侧同层、面板**自有文件写**的
//! 唯一接缝。准许面 / 禁止面 / 方法清单由 ADR-0011 钉死,这里只实现。
//!
//! 三条铁律(全部有测试双证):
//! 1. **类型层不可达 NW_HOME**:句柄只收数据仓根,写目标只收
//!    [`RepoRelPath`](Self::RepoRelPath)——绝对路径 / `..` / 分隔符混用
//!    构造不出,调用方拼不出根之外的路径;
//! 2. **确认前零字节**:`plan` 只算拟落账(路径 + 内容 + 将写文件清单),
//!    不落盘;`commit` 仅确认闸放行后调;
//! 3. **原子落盘**:同目录兄弟 tmp + rename,失败清 tmp,目标处要么旧
//!    内容原样、要么没有,永不留半文件。

use std::path::{Path, PathBuf};

/// 数据仓根之下的相对路径:写侧方法唯一的指认形态。
///
/// 构造时挡掉:空值、绝对路径、任一 `..` 段、`/` 与 `\` 混用、NUL、
/// `~` 开头段(家目录简写,NW_HOME 的默认形状不许以相对路径还魂),
/// 以及末段是**编目册文件名**([`RESERVED_FILES`])。于是「写去 NW_HOME」
/// 「写去引擎仓」「写去根之外」「手写编目」在类型层就过不来——
/// 调用方连一个越界的值都造不出来,更别说递进写侧方法(ADR-0011 §4)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRelPath(String);

impl RepoRelPath {
    /// 从裸串构造;不合法返回人话原因(不带根路径:相对路径是调用方自己的输入)。
    pub fn new(raw: &str) -> Result<Self, RepoRelPathError> {
        if raw.is_empty() {
            return Err(RepoRelPathError::Empty);
        }
        if raw.contains('\0') {
            return Err(RepoRelPathError::Nul);
        }
        if raw.contains('\\') {
            return Err(RepoRelPathError::Backslash);
        }
        if Path::new(raw).is_absolute() {
            return Err(RepoRelPathError::Absolute);
        }
        for seg in raw.split('/') {
            match seg {
                "" => return Err(RepoRelPathError::EmptySegment),
                "." | ".." => return Err(RepoRelPathError::DotSegment(seg.to_string())),
                s if s.starts_with('~') => return Err(RepoRelPathError::TildeSegment),
                _ => {}
            }
        }
        // 末段是编目册文件名也不行(编目是信任闸,手写即开后门,ADR-0011 §2)。
        // 只挡**文件**:目录叫 `catalog.json` 不写这几个文件,不误伤。
        if let Some(last) = raw.rsplit('/').next() {
            if RESERVED_FILES.contains(&last) {
                return Err(RepoRelPathError::ReservedName(last.to_string()));
            }
        }
        Ok(Self(raw.to_string()))
    }

    /// 相对路径原文(确认闸摊开 / 日志 / receipt 都呈现它)。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 五本编目册的文件名(上游事实,模块外不出现第二处):编目是信任闸,
/// 面板写侧一个字节都不碰(ADR-0009 / ADR-0011 §2 第五类)。
const RESERVED_FILES: [&str; 5] = [
    "catalog.json",
    "keys.json",
    "subscriptions.json",
    "tags.json",
    "vps.json",
];

/// 拟落账的一条:相对路径 + 将写内容(字节级确定)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedWrite {
    pub path: RepoRelPath,
    pub content: Vec<u8>,
}

impl PlannedWrite {
    /// 文本内容的便捷构造(配方是文本;字节面仍在)。
    pub fn text(path: RepoRelPath, text: &str) -> Self {
        Self {
            path,
            content: text.as_bytes().to_vec(),
        }
    }
}

/// 拟落账:将写文件清单。确认闸直接呈现的形状——路径 + 内容 + 清单三样齐。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WritePlan {
    pub writes: Vec<PlannedWrite>,
}

impl WritePlan {
    /// 将写文件清单(相对路径,按拟落账顺序)——确认闸的「将写这些文件」行。
    pub fn paths(&self) -> Vec<&str> {
        self.writes.iter().map(|w| w.path.as_str()).collect()
    }

    /// 全部将写字节数(确认闸的体量提示)。
    pub fn total_bytes(&self) -> usize {
        self.writes.iter().map(|w| w.content.len()).sum()
    }
}

/// 实落回报:确认闸放行后 `commit` 回报实落路径清单(日志 / 刷新用)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteReceipt {
    pub written: Vec<String>,
}

/// 数据仓写侧句柄:只收数据仓根,只写根之下(ADR-0011 §4)。
/// 与读侧 [`InstallRoot`](super::InstallRoot) 并列同层,两类路径知识
/// 互不窜:读侧句柄的世界是 NW_HOME,写侧句柄的世界是数据仓根。
#[derive(Clone, Debug)]
pub struct DataRepo {
    root: PathBuf,
}

impl DataRepo {
    /// 数据仓根(设置态同一来源;`~` 展开同读侧 `InstallRoot::new`)。
    /// 类型上没有 NW_HOME 形参——写侧从构造起就不知道安装根的存在。
    pub fn new(root: &str) -> Self {
        Self {
            root: crate::engine::discover::expand_tilde(root),
        }
    }

    /// 数据仓根的路径:**只给同模块族的 [`recipe`](super::recipe) 用**
    /// (读配方原文、预跑暂存落点)。`pub(super)` = 数据仓访问层内部的事,
    /// 屏幕层拿不到 ——它们只消费 `RepoRelPath` 与拟落账形状,「屏幕层零
    /// 路径拼接」(#101 已有测试先例)自然覆盖写侧。
    pub(super) fn root_path(&self) -> &Path {
        &self.root
    }

    /// 给引擎 argv 的绝对路径:**写侧句柄唯一的绝对路径出口**。
    ///
    /// 引擎是外部进程,只认路径(不收 stdin、不收相对当前工作目录的简称),
    /// 于是「根 + 相对路径」这一步也必须有个家——壳层不再自己拼
    /// `format!("{root}/{rel}")`(#113 预跑 argv 先例)。收 [`RepoRelPath`]
    /// 不收裸串:越界的值连这个出口都到不了。
    pub fn abs_path(&self, rel: &RepoRelPath) -> String {
        self.root.join(rel.as_str()).to_string_lossy().to_string()
    }

    /// 写前校验 + 拟落账:**一个字节都不落**。
    ///
    /// 校验:空拟落账拒(没东西可确认是调用方的 bug)、同一次拟落账内
    /// 重复目标拒(两个 PlannedWrite 写同一路径 = 哪份算数说不清)、
    /// 路径防线加深再挡一次(`RepoRelPath` 理论上已保证,这里双证)。
    pub fn plan(&self, writes: Vec<PlannedWrite>) -> Result<WritePlan, WritePlanError> {
        if writes.is_empty() {
            return Err(WritePlanError::EmptyPlan);
        }
        let mut seen: Vec<&str> = Vec::with_capacity(writes.len());
        for w in &writes {
            // 防线加深:newtype 已保证,plan 再判一次(双证,见 tests)。
            RepoRelPath::new(w.path.as_str()).map_err(WritePlanError::BadPath)?;
            if seen.contains(&w.path.as_str()) {
                return Err(WritePlanError::Duplicate(w.path.as_str().to_string()));
            }
            seen.push(w.path.as_str());
        }
        Ok(WritePlan { writes })
    }

    /// 原子落盘:**仅确认闸放行后调用**。
    ///
    /// 逐文件:同目录兄弟 tmp(`.name.nwt-tmp`,保证同文件系统 rename)→
    /// 写 tmp → rename 覆盖目标。任何一步失败:清 tmp、回报人话(带相对
    /// 路径),目标处要么旧内容原样、要么没有——**永不留半文件**。
    /// 多文件 plan 是逐文件原子,非跨文件事务(ADR-0011 Consequences)。
    pub fn commit(&self, plan: &WritePlan) -> Result<WriteReceipt, WriteError> {
        let mut written = Vec::with_capacity(plan.writes.len());
        for w in &plan.writes {
            self.commit_one(w).map_err(|reason| WriteError {
                path: w.path.as_str().to_string(),
                reason,
            })?;
            written.push(w.path.as_str().to_string());
        }
        Ok(WriteReceipt { written })
    }

    /// 单文件原子落盘(tmp + rename)。失败时清 tmp,半文件留不在目标处。
    fn commit_one(&self, w: &PlannedWrite) -> Result<(), String> {
        let target = self.root.join(w.path.as_str());
        let Some(parent) = target.parent() else {
            return Err("目标没有父目录".into());
        };
        std::fs::create_dir_all(parent).map_err(|e| io_reason(&e))?;
        let tmp = tmp_path(&target);
        if let Err(e) = std::fs::write(&tmp, &w.content) {
            // 写 tmp 就败:目标一个字节都没动(旧的原样或不在)。
            let _ = std::fs::remove_file(&tmp);
            return Err(io_reason(&e));
        }
        if let Err(e) = std::fs::rename(&tmp, &target) {
            // rename 败(如目标是目录):清 tmp,目标保持原样。
            let _ = std::fs::remove_file(&tmp);
            return Err(io_reason(&e));
        }
        Ok(())
    }
}

/// 兄弟临时文件名(与目标同目录 = 同文件系统,rename 才是原子的)。
/// 点号前缀 + `.nwt-tmp` 后缀:隐藏、不与目标撞名;固定名 = 上一轮崩溃
/// 留下的 stale tmp 被下一次 `fs::write` 截断复用,不积垃圾。
fn tmp_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|n| format!(".{}.nwt-tmp", n.to_string_lossy()))
        .unwrap_or_else(|| ".nwt-tmp".into());
    target.with_file_name(name)
}

/// IO 错误的人话(不带根路径:根是接缝内部的事,屏上只摆相对路径 + 原因)。
fn io_reason(e: &std::io::Error) -> String {
    format!("{e}")
}

/// `RepoRelPath` 构造被拒的原因(人话,供表单 / 闸直接呈现)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoRelPathError {
    Empty,
    Nul,
    Backslash,
    Absolute,
    EmptySegment,
    DotSegment(String),
    TildeSegment,
    /// 末段是编目册文件名(ADR-0011 §2 第五类):编目是信任闸,手写即开后门。
    ReservedName(String),
}

impl RepoRelPathError {
    pub fn why(&self) -> String {
        match self {
            Self::Empty => "相对路径为空".into(),
            Self::Nul => "路径含 NUL 字符".into(),
            Self::Backslash => "路径含反斜杠:数据仓路径一律用正斜杠".into(),
            Self::Absolute => "只收数据仓根之下的相对路径,不收绝对路径".into(),
            Self::EmptySegment => "路径有空段(连续斜杠或首尾斜杠)".into(),
            Self::DotSegment(s) => format!("路径含 `{s}` 段:相对路径不含 `.` / `..`"),
            Self::TildeSegment => "路径含 `~` 开头段:家目录简写不进数据仓相对路径".into(),
            Self::ReservedName(n) => {
                format!("`{n}` 是编目册文件:编目只走引擎命令(ADR-0009),面板写侧一个字节都不碰")
            }
        }
    }
}

/// 拟落账阶段的校验失败(写前校验,一个字节都没落)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WritePlanError {
    EmptyPlan,
    Duplicate(String),
    /// 防线加深才够得到的:newtype 理论上构造不出,双证留着。
    BadPath(RepoRelPathError),
}

impl WritePlanError {
    pub fn why(&self) -> String {
        match self {
            Self::EmptyPlan => "拟落账为空:没有要写的东西".into(),
            Self::Duplicate(p) => format!("同一次拟落账里 `{p}` 出现两次"),
            Self::BadPath(e) => e.why(),
        }
    }
}

/// 落盘失败:磁盘 / 权限问题,**不是用户输入问题**(引擎 rc 4 同款教训,
/// ADR-0011 §5:别渲染成「输入无效」)。`path` 是数据仓根之下的相对路径。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteError {
    pub path: String,
    pub reason: String,
}

impl WriteError {
    pub fn why(&self) -> String {
        format!("落盘失败 `{}`:{}", self.path, self.reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 每测试一个一次性目录(进程级唯一 + 测试名唯一,并行不互踩;
    /// 同读侧 `tests::fixture` 模式)。
    fn fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nwt-write-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn repo(root: &Path) -> DataRepo {
        DataRepo::new(root.to_str().unwrap())
    }

    // ---- NW_HOME 不可达:类型 + 测试双证 ----

    #[test]
    fn 相对路径_newtype_拒绝对径与越界段() {
        for bad in [
            "",
            "/etc/passwd",
            "~/.nous-warehouse/x",
            "../../../etc/passwd",
            "..",
            "recipes/../../nw-home",
            "a//b",
            "a/./b",
            "a\\b",
            "a\0b",
        ] {
            assert!(
                RepoRelPath::new(bad).is_err(),
                "{bad:?} 应被拒:写侧只收根之下相对路径"
            );
        }
        // 合法相对路径照常收
        assert_eq!(
            RepoRelPath::new("recipes/research-desk/recipe.yaml")
                .unwrap()
                .as_str(),
            "recipes/research-desk/recipe.yaml"
        );
    }

    /// ADR-0011 §2 第五类「机密编目文件」在类型层闭合:五本编目册的名字
    /// 直接构造不出相对路径——手写编目即开后门(编目是信任闸)。
    #[test]
    fn 编目册文件名_类型层就拒() {
        for bad in [
            "catalog.json",
            "keys.json",
            "subscriptions.json",
            "tags.json",
            "vps.json",
            "recipes/catalog.json",
            "a/b/c/keys.json",
        ] {
            let err = RepoRelPath::new(bad).unwrap_err();
            assert!(
                matches!(err, RepoRelPathError::ReservedName(_)),
                "{bad} 应被拒:{err:?}"
            );
            assert!(
                err.why().contains("编目册"),
                "{bad} 的人话点题:{}",
                err.why()
            );
        }
        // 同名的**目录**不挡:编目册是文件,目录叫什么都不写这几个文件
        assert!(RepoRelPath::new("catalog.json/notes.md").is_ok());
        // 数据仓里合法可写的相对路径照常收(现消费方只写配方一脉,#113)
        for ok in [
            "recipes/research-desk/recipe.yaml",
            "recipes/research-desk/.nwt-validate.yaml",
            "recipes/night-shift/recipe.yaml",
        ] {
            assert!(RepoRelPath::new(ok).is_ok(), "{ok} 照常收");
        }
    }

    #[test]
    fn 写侧句柄只碰根之下_根外零痕迹() {
        let root = fixture("scope");
        let outside = root.join("outside-sentinel");
        fs::write(&outside, "keep").unwrap();
        let r = repo(&root);
        let plan = r
            .plan(vec![PlannedWrite::text(
                RepoRelPath::new("recipes/research-desk/recipe.yaml").unwrap(),
                "apiVersion: nw/v1\n",
            )])
            .unwrap();
        r.commit(&plan).unwrap();
        // 落的精确落在根之下
        assert_eq!(
            fs::read_to_string(root.join("recipes/research-desk/recipe.yaml")).unwrap(),
            "apiVersion: nw/v1\n"
        );
        // 根外哨兵原样;根之外没有任何新文件(除测试自己建的)
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
        assert!(!root.join("nw-home-ish").exists());
        fs::remove_dir_all(&root).ok();
    }

    // ---- 拟落账:确认前零字节,形状可供确认闸呈现 ----

    #[test]
    fn 拟落账_不落一个字节_形状齐_paths与内容可呈现() {
        let root = fixture("plan-only");
        let r = repo(&root);
        let plan = r
            .plan(vec![
                PlannedWrite::text(
                    RepoRelPath::new("recipes/research-desk/recipe.yaml").unwrap(),
                    "apiVersion: nw/v1\n",
                ),
                PlannedWrite::text(
                    RepoRelPath::new("recipes/research-desk/.nwt-validate.yaml").unwrap(),
                    "apiVersion: nw/v1\n",
                ),
            ])
            .unwrap();
        // 确认闸直接呈现的三样:清单、逐条内容、体量
        assert_eq!(
            plan.paths(),
            vec![
                "recipes/research-desk/recipe.yaml",
                "recipes/research-desk/.nwt-validate.yaml"
            ]
        );
        assert_eq!(plan.writes[0].content, b"apiVersion: nw/v1\n".to_vec());
        assert_eq!(plan.total_bytes(), 2 * "apiVersion: nw/v1\n".len());
        // 一个字节都没落:根还是空的
        assert!(fs::read_dir(&root).unwrap().next().is_none(), "plan 不落盘");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 写前校验_空拟落账与重复目标被拒() {
        let root = fixture("plan-reject");
        let r = repo(&root);
        assert_eq!(r.plan(vec![]).unwrap_err(), WritePlanError::EmptyPlan);
        let dup = r
            .plan(vec![
                PlannedWrite::text(RepoRelPath::new("a.md").unwrap(), "1"),
                PlannedWrite::text(RepoRelPath::new("a.md").unwrap(), "2"),
            ])
            .unwrap_err();
        assert_eq!(dup, WritePlanError::Duplicate("a.md".into()));
        assert!(fs::read_dir(&root).unwrap().next().is_none(), "拒了更不落");
        fs::remove_dir_all(&root).ok();
    }

    // ---- 落盘:确定字节 + 单文件 / 多文件两种形状 ----

    #[test]
    fn 确认后落盘_确定字节() {
        let root = fixture("commit-bytes");
        let r = repo(&root);
        let plan = r
            .plan(vec![PlannedWrite::text(
                RepoRelPath::new("recipes/research-desk/recipe.yaml").unwrap(),
                "apiVersion: nw/v1\nkind: recipe\n",
            )])
            .unwrap();
        let receipt = r.commit(&plan).unwrap();
        assert_eq!(receipt.written, vec!["recipes/research-desk/recipe.yaml"]);
        assert_eq!(
            fs::read(root.join("recipes/research-desk/recipe.yaml")).unwrap(),
            plan.writes[0].content,
            "fixture 进、确定字节出"
        );
        fs::remove_dir_all(&root).ok();
    }

    /// 多文件 plan 的接缝能力(§4:逐文件原子,非跨文件事务)。工单 #118 后
    /// 没有消费方一次写多文件,能力留在接缝上、测试钉住,不是现行义务。
    #[test]
    fn 多文件拟落账_逐文件原子落() {
        let root = fixture("multi-file");
        let r = repo(&root);
        let plan = r
            .plan(vec![
                PlannedWrite::text(
                    RepoRelPath::new("recipes/research-desk/recipe.yaml").unwrap(),
                    "apiVersion: nw/v1\n",
                ),
                PlannedWrite::text(
                    RepoRelPath::new("recipes/night-shift/recipe.yaml").unwrap(),
                    "apiVersion: nw/v1\n",
                ),
            ])
            .unwrap();
        r.commit(&plan).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("recipes/research-desk/recipe.yaml")).unwrap(),
            "apiVersion: nw/v1\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("recipes/night-shift/recipe.yaml")).unwrap(),
            "apiVersion: nw/v1\n"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 整文件替换_rename覆盖即原子替换() {
        let root = fixture("replace");
        fs::create_dir_all(root.join("recipes")).unwrap();
        fs::write(
            root.join("recipes/research.yaml"),
            "apiVersion: nw/v1\nold: true\n",
        )
        .unwrap();
        let r = repo(&root);
        let plan = r
            .plan(vec![PlannedWrite::text(
                RepoRelPath::new("recipes/research.yaml").unwrap(),
                "apiVersion: nw/v1\nnew: true\n",
            )])
            .unwrap();
        r.commit(&plan).unwrap();
        assert_eq!(
            fs::read_to_string(root.join("recipes/research.yaml")).unwrap(),
            "apiVersion: nw/v1\nnew: true\n",
            "整文件替换,旧内容一个字节不留"
        );
        fs::remove_dir_all(&root).ok();
    }

    // ---- 原子写:中断不留半文件(模拟失败的场景) ----

    #[test]
    fn 写tmp阶段中断_目标不留半文件() {
        let root = fixture("half-tmp");
        fs::create_dir_all(root.join("recipes/research-desk")).unwrap();
        // 抢占(tmp 名确定:`.recipe.yaml.nwt-tmp`),让 fs::write 落在目录上必败
        fs::create_dir_all(root.join("recipes/research-desk/.recipe.yaml.nwt-tmp")).unwrap();
        let r = repo(&root);
        let plan = r
            .plan(vec![PlannedWrite::text(
                RepoRelPath::new("recipes/research-desk/recipe.yaml").unwrap(),
                "# 半个文件?\n",
            )])
            .unwrap();
        let err = r.commit(&plan).unwrap_err();
        assert_eq!(err.path, "recipes/research-desk/recipe.yaml");
        assert!(err.why().contains("落盘失败"), "人话点题:{}", err.why());
        assert!(
            !root.join("recipes/research-desk/recipe.yaml").exists(),
            "写 tmp 就败 = 目标不存在,没有半文件"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rename阶段中断_目标保持原样() {
        let root = fixture("half-rename");
        fs::create_dir_all(root.join("recipes")).unwrap();
        fs::write(root.join("recipes/research.yaml"), "old: true\n").unwrap();
        // 目标是目录时 rename 必败;tmp 是普通文件能写成
        fs::create_dir_all(root.join("blocked.yaml")).unwrap();
        let r = repo(&root);
        let plan = r
            .plan(vec![
                PlannedWrite::text(
                    RepoRelPath::new("recipes/research.yaml").unwrap(),
                    "new: true\n",
                ),
                PlannedWrite::text(RepoRelPath::new("blocked.yaml").unwrap(), "x"),
            ])
            .unwrap();
        let err = r.commit(&plan).unwrap_err();
        assert_eq!(err.path, "blocked.yaml");
        // 前一个文件已实落(逐文件原子,非跨文件事务,ADR-0011 明示)
        assert_eq!(
            fs::read_to_string(root.join("recipes/research.yaml")).unwrap(),
            "new: true\n"
        );
        // 失败的目标保持原样(目录还在),且 tmp 已清
        assert!(root.join("blocked.yaml").is_dir());
        assert!(
            !root.join(".blocked.yaml.nwt-tmp").exists(),
            "失败清 tmp,不留残渣"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn 父路径被文件占_写不进去也不留残渣() {
        let root = fixture("blocked-parent");
        fs::write(root.join("blocker"), "i am a file").unwrap();
        let r = repo(&root);
        let plan = r
            .plan(vec![PlannedWrite::text(
                RepoRelPath::new("blocker/child.md").unwrap(),
                "x",
            )])
            .unwrap();
        let err = r.commit(&plan).unwrap_err();
        assert_eq!(err.path, "blocker/child.md");
        assert_eq!(
            fs::read_to_string(root.join("blocker")).unwrap(),
            "i am a file"
        );
        fs::remove_dir_all(&root).ok();
    }
}
