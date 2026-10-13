//! nwt 自升级(工单 #121,规格 #96 第四项):设置屏 `U` 键。
//!
//! **双出口共享同一套校验纪律**:面板不是把「下载 → sha256 校验 → 落盘」
//! 在 Rust 里另写一遍,而是把 [`INSTALL_SH`] 那份原文写进临时文件、以 `sh`
//! 跑起来——操作者在终端里 `curl | sh` 与在面板里按 `U` 走的是**同一份字节、
//! 同一道闸**(规格 #96 Further Notes;纪律成文在 `docs/release.md`)。
//!
//! 面板自己只多两件 install.sh 刻意不做的事:
//! 1. **落盘用 tmp + rename**(install.sh 是 `cp`,覆盖不是原子的)——rename
//!    前新二进制已整个写进同目录的 `.new`,所以任何一步失败旧二进制都原样不动;
//! 2. **把过程呈现在日志区**(脚本原文逐行来,不翻译、不改写)。
//!
//! 铁律(与 install.sh 同序):**先校验后安装**。sha256 对不上 = 一个字节都不碰
//! 旧二进制。凭证(GH_TOKEN / GITHUB_TOKEN)只作为子进程环境变量透传,绝不进
//! 日志 / 配置 / 回音(既有铁律:配置永不存密钥)。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;

/// 嵌进来的 install.sh 原文:编译期从仓库根部取,**双出口同源的唯一保证**。
/// 改 install.sh 的行为契约(资产命名 / 校验顺序 / token 口径)这里自动跟上,
/// 不存在「面板一份、脚本一份」各自漂移的两套纪律。
pub const INSTALL_SH: &str = include_str!("../install.sh");

/// 升级请求:latest 或钉版本 + 要替换的那一个二进制 + 这一趟是干跑还是真跑。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpgradeRequest {
    /// `None` = latest;`Some(tag)` = 钉版本(带不带 `v` 前缀都收,脚本自己剥)。
    pub version: Option<String>,
    /// 要替换的可执行文件(面板升级它自己)。
    pub target: PathBuf,
    /// `true` = 只拿安装计划(`--dry-run`,零下载、零落盘);`false` = 真跑。
    pub dry_run: bool,
}

/// 升级子进程的环境:两个源覆盖口 + 一个凭证口。
///
/// **token 只从这里进子进程环境**。它不进 [`crate::app::App`] 任何字段、不进
/// 日志、不进配置、不进回音——所以这个结构体的 `Debug` 输出也不许把它印出来
/// (见下面手写的 `Debug`,以及「无 token 场景」测试对日志的断言)。
#[derive(Clone, Default, PartialEq, Eq)]
pub struct UpgradeEnv {
    pub download_base: Option<String>,
    pub api_base: Option<String>,
    pub token: Option<String>,
}

impl UpgradeEnv {
    /// 从进程环境取:面板自己是怎么被装上去的,升级就怎么走。
    /// `GH_TOKEN` / `GITHUB_TOKEN` 任选其一(install.sh 同款口径;都没有 = 匿名)。
    pub fn from_env() -> Self {
        let token = std::env::var("GH_TOKEN")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| std::env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty()));
        Self {
            download_base: std::env::var("NWT_DOWNLOAD_BASE")
                .ok()
                .filter(|s| !s.is_empty()),
            api_base: std::env::var("NWT_API_BASE").ok().filter(|s| !s.is_empty()),
            token,
        }
    }

    /// 把环境设进子进程。凭证**只在真有值时设**:显式设一个空串会让「本来
    /// 继承了真 token 的场合」被我们自己清成匿名;没有值时两个名字都摘掉,
    /// 免得测试或调用方的进程环境偷偷带进一个真 token。
    fn apply(&self, cmd: &mut Command) {
        if let Some(b) = &self.download_base {
            cmd.env("NWT_DOWNLOAD_BASE", b);
        }
        if let Some(b) = &self.api_base {
            cmd.env("NWT_API_BASE", b);
        }
        match &self.token {
            Some(t) => {
                cmd.env("GH_TOKEN", t);
                cmd.env_remove("GITHUB_TOKEN");
            }
            None => {
                cmd.env_remove("GH_TOKEN");
                cmd.env_remove("GITHUB_TOKEN");
            }
        }
    }
}

/// 手写 `Debug`:token 一律印成 `<redacted>`。
/// 派生版会把凭证写进任何 `{:?}`——而 `{:?}` 会进 panic 上报(铁律:机密不进去)。
impl std::fmt::Debug for UpgradeEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpgradeEnv")
            .field("download_base", &self.download_base)
            .field("api_base", &self.api_base)
            .field(
                "token",
                &self
                    .token
                    .as_ref()
                    .map(|_| "<redacted>")
                    .unwrap_or_default(),
            )
            .finish()
    }
}

/// 升级过程的推进事件(子进程 → [`crate::app::App`])。照引擎事件的形状:
/// 逐行进度 + 一个终结结论,主循环在 select 里收。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpgradeEvent {
    /// install.sh 的一行**原文**输出(计划 / 进度 / 校验结论)。面板不翻译。
    Progress(String),
    /// 子进程终结。`exit_code == 0` = 已通过校验并落进暂存目录(面板随后才
    /// tmp+rename 替换自己);否则 = 失败,`reason` 是脚本自己的人话。
    Finished(UpgradeOutcome),
}

/// 一次升级跑完的结论。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpgradeOutcome {
    pub exit_code: i32,
    /// 失败时的人话(取 install.sh 的 `die` 文案;它带「私有仓请设 GH_TOKEN」
    /// 这一类指路,所以面板不另造一套说法)。
    pub reason: String,
}

/// 自升级过程的状态(工单 #121):`U` 只在设置屏在场,六态两两可区分。
///
/// 与装配态的关系:**在途态点 `assembling` 那盏全局写互斥灯**(规格「与装配态
/// 同款互斥思路」),所以其余写入口的既有闸自然兜住;顶栏 / HUD 另给一句
/// 「升级中」,不冒充装配。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpgradePhase {
    /// 不在升级。
    #[default]
    Idle,
    /// 输目标版本(预填 `latest`;Enter 才下发,Esc/q 放弃)。
    Version { buf: String },
    /// 干跑在途(只拿计划,零下载、零落盘)。
    Planning { version: String },
    /// 计划已摊开(版本 / 资产 / 校验文件 / 落点),等操作者确认。
    Plan { version: String },
    /// 真跑在途:下载 → sha256 校验 → 落暂存目录 → tmp+rename。
    Running { version: String },
    /// 完成:校验过、已换人。**要重启面板新版本才在场**。
    Done { version: String, sha256: String },
    /// 失败:下载失败 / 校验不过 / 换不进去。**旧二进制原样不动**。
    Failed { reason: String },
}

impl UpgradePhase {
    /// 升级浮层在不在场(六态里只有 `Idle` 不在)。
    pub fn is_open(&self) -> bool {
        !matches!(self, UpgradePhase::Idle)
    }

    /// 在途(干跑或真跑):此刻其余动作已锁,`Esc/q` 不给半途反悔的口子——
    /// 下载 / 校验在途中,面板不假装能收回。
    pub fn in_flight(&self) -> bool {
        matches!(
            self,
            UpgradePhase::Planning { .. } | UpgradePhase::Running { .. }
        )
    }

    /// 等一个「继续」确认的状态(计划摊开着)。
    pub fn awaits_confirm(&self) -> bool {
        matches!(self, UpgradePhase::Plan { .. })
    }

    /// 屏上该说的那一句(结论态的人话)。
    pub fn headline(&self) -> &'static str {
        match self {
            UpgradePhase::Idle => "",
            UpgradePhase::Version { .. } => "自升级:填目标版本(latest 或一个 tag)",
            UpgradePhase::Planning { .. } => "自升级:正在取安装计划(零下载、零落盘)",
            UpgradePhase::Plan { .. } => "自升级计划:确认后开始下载并校验",
            UpgradePhase::Running { .. } => "自升级中:下载 → sha256 校验 → 落盘",
            UpgradePhase::Done { .. } => "自升级完成:重启面板后新版本才在场",
            UpgradePhase::Failed { .. } => "自升级失败:旧二进制原样不动,未安装任何文件",
        }
    }
}

/// 从 install.sh 的输出行里取 sha256 校验结论:`sha256 校验通过(<hash>)`。
/// 取不到(那一行还没来 / 格式变了)→ `None`,面板不猜、不补一个哈希。
pub fn sha256_from_line(line: &str) -> Option<String> {
    let key = "sha256 校验通过(";
    let i = line.find(key)? + key.len();
    let rest = &line[i..];
    Some(rest[..rest.find(')')?].to_string())
}

/// 从 install.sh 的输出行里取**解析后的**版本号:`安装 nwt v<版本> …`。
/// 操作者填 `latest` 时,屏上该说清最终装的是哪一版(故事 14:可复现)。
pub fn installed_version_from_line(line: &str) -> Option<String> {
    let key = "安装 nwt v";
    let i = line.find(key)? + key.len();
    let rest = &line[i..];
    Some(rest[..rest.find(|c: char| c.is_whitespace())?].to_string())
}

/// 干跑 argv:只拿计划,不下载、不落盘。`--prefix` 指**真正的落点父目录**,
/// 这样计划里「装到 <落点>/nwt」说的是实话,不是暂存目录。
fn dry_run_args(req: &UpgradeRequest, prefix: &Path) -> Vec<String> {
    let mut args = vec![
        "--dry-run".to_string(),
        "--prefix".to_string(),
        prefix.display().to_string(),
    ];
    if let Some(v) = &req.version {
        args.push("--version".to_string());
        args.push(v.clone());
    }
    args
}

/// 真跑 argv:`--prefix` 指暂存目录——install.sh 把验证过的那一份装到那儿,
/// 面板随后自己 tmp+rename 换掉当前可执行文件。
fn install_args(stage: &Path, version: Option<&str>) -> Vec<String> {
    let mut args = vec!["--prefix".to_string(), stage.display().to_string()];
    if let Some(v) = version {
        args.push("--version".to_string());
        args.push(v.to_string());
    }
    args
}

/// 把嵌进来的脚本写进临时文件(POSIX 权限 0700)。每次调用一个独立路径,
/// 免得并发的两趟升级互相踩;用完由调用方删。
fn write_script() -> std::io::Result<PathBuf> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("nwt-upgrade-{}-{}", std::process::id(), n));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("install.sh");
    std::fs::write(&path, INSTALL_SH)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

/// 跑一趟 install.sh(干跑或真跑),把逐行输出当进度推出去。
/// 返回 `(退出码, stderr 尾迹)`;stderr 尾迹只在失败时当 `reason` 用。
async fn run_script(
    script: &Path,
    args: &[String],
    env: &UpgradeEnv,
    tx: &mpsc::UnboundedSender<UpgradeEvent>,
) -> (i32, Vec<String>) {
    let mut cmd = Command::new("sh");
    cmd.arg(script)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    env.apply(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(UpgradeEvent::Progress(format!(
                "nwt-install: 起不了 install.sh({e})"
            )));
            return (127, vec![format!("起不了 install.sh({e})")]);
        }
    };
    // 两路都先 take 出来:stderr 另起一路收(install.sh 的 `die` 人话全在
    // stderr,失败时它是 reason),stdout 就地逐行当进度推出去。
    let out = child.stdout.take();
    let err = child.stderr.take();
    let err_lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let stderr_sink = err_lines.clone();
    let stderr_task = tokio::spawn(async move {
        if let Some(s) = err {
            let mut lines = tokio::io::BufReader::new(s).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                stderr_sink.lock().unwrap().push(l);
            }
        }
    });
    if let Some(s) = out {
        let mut lines = tokio::io::BufReader::new(s).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            let _ = tx.send(UpgradeEvent::Progress(l));
        }
    }
    let status = child.wait().await;
    let _ = stderr_task.await;
    let code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
    let tail = err_lines.lock().unwrap().clone();
    (code, tail)
}

/// **tmp + rename 原子替换**:把暂存目录里那一份(已通过 sha256 校验)写进
/// 目标同目录的 `.new`,再 rename 盖上去。
///
/// 顺序即安全保证:新二进制整个写完才 rename,而 rename 之前旧的一个字节都没
/// 被碰过——所以下载失败 / 校验不过 / 写盘失败 / rename 失败,旧二进制统统
/// 原样在场(故事 15:升级动作本身不会把面板弄坏)。
pub fn apply_staged(staged: &Path, target: &Path) -> std::io::Result<()> {
    let bytes = std::fs::read(staged)?;
    let tmp = target.with_file_name(format!(
        "{}.new",
        target
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "nwt".into())
    ));
    if let Err(e) = std::fs::write(&tmp, &bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    #[cfg(unix)]
    {
        // 暂存那份的权限照搬(install.sh chmod 0755);设不上不算失败——
        // rename 本身才是换人的那一步。
        if let Ok(m) = std::fs::metadata(staged) {
            let _ = std::fs::set_permissions(&tmp, m.permissions());
        }
    }
    if let Err(e) = std::fs::rename(&tmp, target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// 跑一趟升级:干跑只出计划;真跑在 rc 0 后由本函数做 tmp+rename。
///
/// 全程**零网络逃逸口**:下载面由 [`UpgradeEnv`] 的两个 base 决定,测试把它们
/// 指到 fixture 目录(curl 吃 `file://`),所以这条链在测试里不连 github.com。
pub async fn run(
    req: &UpgradeRequest,
    env: &UpgradeEnv,
    tx: &mpsc::UnboundedSender<UpgradeEvent>,
) -> UpgradeOutcome {
    let script = match write_script() {
        Ok(p) => p,
        Err(e) => {
            return UpgradeOutcome {
                exit_code: 127,
                reason: format!("写不了临时安装脚本({e})"),
            };
        }
    };
    let (code, tail) = if req.dry_run {
        // 干跑:prefix 指真正的落点父目录,计划里「装到哪」说的是实话。
        let prefix = req
            .target
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| req.target.clone());
        run_script(&script, &dry_run_args(req, &prefix), env, tx).await
    } else {
        // 真跑:先落暂存目录,rc 0 且暂存里真有那一份,才轮到面板换人。
        let stage = script
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(std::env::temp_dir)
            .join("stage");
        let _ = std::fs::create_dir_all(&stage);
        let args = install_args(&stage, req.version.as_deref());
        let (code, tail) = run_script(&script, &args, env, tx).await;
        if code == 0 {
            let staged = stage.join("nwt");
            if !staged.is_file() {
                return UpgradeOutcome {
                    exit_code: 1,
                    reason: format!("暂存目录里找不到 {}", staged.display()),
                };
            }
            // 换人这一步失败:旧二进制原样不动,人话说清是「换不进去」。
            if let Err(e) = apply_staged(&staged, &req.target) {
                return UpgradeOutcome {
                    exit_code: 1,
                    reason: format!(
                        "校验过了,但暂存那一份换不进 {}({e})——旧二进制原样不动",
                        req.target.display()
                    ),
                };
            }
            let _ = std::fs::remove_dir_all(&stage);
        }
        (code, tail)
    };
    let _ = std::fs::remove_file(&script);
    let outcome = if code == 0 {
        UpgradeOutcome {
            exit_code: 0,
            reason: String::new(),
        }
    } else {
        // install.sh 的 die 文案自带指路(私有仓请设 GH_TOKEN …),取最后一行
        // 非空的人话;一行都没有就按退出码说,不编。
        UpgradeOutcome {
            exit_code: code,
            reason: tail
                .iter()
                .rev()
                .find(|l| !l.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| format!("install.sh 退出码 {code}")),
        }
    };
    // 终结事件由本函数自己发:调用方(主循环)只负责 spawn,不负责拼结论——
    // 否则「Finished 由谁发」会变成第二个真相源,漏发就让 App 永远停在在途态。
    let _ = tx.send(UpgradeEvent::Finished(outcome.clone()));
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一次性临时目录(按测试名分开,免得并行用例互相踩)。
    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nwt-upg-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("建临时目录");
        d
    }

    fn sha256_of(path: &Path) -> std::io::Result<String> {
        let out = std::process::Command::new("sha256sum").arg(path).output()?;
        let text = String::from_utf8_lossy(&out.stdout);
        Ok(text.split_whitespace().next().unwrap_or("").to_string())
    }

    /// 当前机器的 arch 字面(install.sh 的 detect_arch 同口径:x86_64 / aarch64)。
    fn arch() -> &'static str {
        std::env::consts::ARCH
    }

    /// 造假 release 基座:web 下载面 + API 资产面 + checksums.txt + latest/tags JSON。
    ///
    /// `good = false` 时 checksums.txt 里那一行的哈希是错的——闸必须拒。
    /// 全部落在临时目录,`NWT_DOWNLOAD_BASE` / `NWT_API_BASE` 指过去(curl 吃
    /// `file://`),所以驱动这条链的是 fixture 而不是 github.com。
    fn fake_release(base: &Path, ver: &str, good: bool) -> std::io::Result<PathBuf> {
        let a = arch();
        let asset = format!("nwt_{ver}_linux_{a}.tar.gz");
        // 包里只有一个二进制本体(与发布包同构:包内只有 nwt)
        let src = base.join("stage-src");
        std::fs::create_dir_all(&src)?;
        std::fs::write(src.join("nwt"), format!("FAKE-NWT-v{ver}\n"))?;
        // web 下载面的路径形状是 `<download_base>/v<VER>/<asset>`
        // —— 目录名 literal 是 `v0.9.9`,不是 `v/0.9.9`(install.sh 的
        // ASSET_URL 就那么拼,fixture 必须照它的形状摆)。
        let tar = base.join(format!("v{ver}")).join(&asset);
        std::fs::create_dir_all(tar.parent().expect("父目录"))?;
        let st = std::process::Command::new("tar")
            .arg("-C")
            .arg(&src)
            .arg("-czf")
            .arg(&tar)
            .arg("nwt")
            .status()?;
        assert!(st.success(), "打包 fixture 资产失败");
        let wrote = if good {
            sha256_of(&tar)?
        } else {
            "0".repeat(64)
        };
        std::fs::write(
            tar.parent().expect("父目录").join("checksums.txt"),
            format!("{wrote}  {asset}\n"),
        )?;
        // API 面:latest / tags/<v> / assets/<id>(install.sh 的 asset_id 抠 id 用)
        let api = base.join("repos/JoeWest9966/Warehousetui/releases");
        std::fs::create_dir_all(api.join("tags"))?;
        std::fs::create_dir_all(api.join("assets"))?;
        std::fs::write(
            api.join("latest"),
            format!("{{\n  \"tag_name\": \"v{ver}\"\n}}\n"),
        )?;
        std::fs::write(
            api.join("tags").join(format!("v{ver}")),
            format!(
                "{{\n  \"tag_name\": \"v{ver}\",\n  \"assets\": [\n    {{\n      \"id\": 4242,\n      \"name\": \"{asset}\"\n    }},\n    {{\n      \"id\": 4243,\n      \"name\": \"checksums.txt\"\n    }}\n  ]\n}}\n"
            ),
        )?;
        std::fs::copy(&tar, api.join("assets/4242"))?;
        std::fs::copy(
            tar.parent().expect("父目录").join("checksums.txt"),
            api.join("assets/4243"),
        )?;
        Ok(tar)
    }

    /// 指向 fixture 的环境:两个 base 同一个目录(web 面在 `v<ver>/`、API 面在
    /// `repos/…`,都在它下面)。`token = None` = 匿名走 web 下载面。
    fn fixture_env(base: &Path, token: Option<&str>) -> UpgradeEnv {
        let root = format!("file://{}", base.display());
        UpgradeEnv {
            download_base: Some(root.clone()),
            api_base: Some(root),
            token: token.map(str::to_string),
        }
    }

    /// 收齐一趟升级的所有进度行(测试断言用)。
    async fn drain(
        rx: &mut mpsc::UnboundedReceiver<UpgradeEvent>,
    ) -> (Vec<String>, UpgradeOutcome) {
        let mut lines = Vec::new();
        let mut done = None;
        while let Some(ev) = rx.recv().await {
            match ev {
                UpgradeEvent::Progress(l) => lines.push(l),
                UpgradeEvent::Finished(o) => {
                    done = Some(o);
                    break;
                }
            }
        }
        (lines, done.expect("必须有终结事件"))
    }

    // ---- 双出口同源:嵌进来的就是仓库那一份 ----

    #[test]
    fn 嵌进来的脚本_就是仓库那一份_先校验后安装的次序在场() {
        // 纪律的同源不是口号:sha256 闸与资产命名都在这份字节里,且
        // **校验排在拷贝之前**——先校验后安装,与 install.sh 同序。
        assert!(
            INSTALL_SH.contains("verify_checksum"),
            "嵌进来的脚本必须带 sha256 闸"
        );
        assert!(
            INSTALL_SH.contains("nwt_${VER}_${SYS_OS}_${SYS_ARCH}.tar.gz"),
            "资产命名契约同源"
        );
        assert!(
            INSTALL_SH.contains("GH_TOKEN:-${GITHUB_TOKEN:-}"),
            "私有仓凭证口径同源"
        );
        let verify = INSTALL_SH.find("verify_checksum").expect("闸在场");
        let cp = INSTALL_SH
            .find("cp \"$WORK/$BIN_NAME\"")
            .expect("安装动作在场");
        assert!(verify < cp, "先校验后安装:闸必须排在拷贝之前");
    }

    // ---- 脚本输出面的两个纯函数(呈现用,不重算) ----

    #[test]
    fn sha256校验结论行_取出哈希_格式不对给none() {
        assert_eq!(
            sha256_from_line("nwt-install: sha256 校验通过(9f8ea1b2c3)"),
            Some("9f8ea1b2c3".into()),
            "校验结论原样取哈希"
        );
        assert_eq!(sha256_from_line("nwt-install: 安装 nwt v0.9.9"), None);
        assert_eq!(sha256_from_line(""), None, "没那一行不猜");
    }

    #[test]
    fn 安装行_取出解析后的版本_填latest也知道最终装了哪版() {
        assert_eq!(
            installed_version_from_line(
                "nwt-install: 安装 nwt v0.9.9 (linux/x86_64) 到 /usr/local/bin"
            ),
            Some("0.9.9".into()),
            "填 latest 也说清最终装了哪一版"
        );
        assert_eq!(installed_version_from_line("nwt-install: 安装计划"), None);
    }

    // ---- argv 形状 ----

    #[test]
    fn 干跑argv_指真正的落点_钉版本才带version旗标() {
        let req = |version: Option<&str>| UpgradeRequest {
            version: version.map(str::to_string),
            target: PathBuf::from("/usr/local/bin/nwt"),
            dry_run: true,
        };
        let args = dry_run_args(&req(Some("v0.4.0")), Path::new("/usr/local/bin"));
        assert_eq!(
            args,
            vec![
                "--dry-run".to_string(),
                "--prefix".to_string(),
                "/usr/local/bin".to_string(),
                "--version".to_string(),
                "v0.4.0".to_string()
            ],
            "干跑:零下载零落盘,prefix 说真正的落点"
        );
        let latest = dry_run_args(&req(None), Path::new("/usr/local/bin"));
        assert!(
            !latest.iter().any(|a| a == "--version"),
            "latest 不带 --version(脚本自己解析):{latest:?}"
        );
    }

    #[test]
    fn 真跑argv_指暂存目录_钉版本带上() {
        let args = install_args(Path::new("/tmp/stage"), Some("0.4.0"));
        assert_eq!(
            args,
            vec![
                "--prefix".to_string(),
                "/tmp/stage".to_string(),
                "--version".to_string(),
                "0.4.0".to_string()
            ],
            "真跑落暂存目录,随后面板自己 tmp+rename"
        );
    }

    // ---- tmp+rename:顺序即安全保证 ----

    #[test]
    fn 换人_新二进制整个写完才rename_失败则旧的一个字节都没被碰() {
        let d = tmp("staged");
        let staged = d.join("stage/nwt");
        std::fs::create_dir_all(staged.parent().expect("父目录")).unwrap();
        std::fs::write(&staged, b"NEW-BINARY").unwrap();
        let target = d.join("bin/nwt");
        std::fs::create_dir_all(target.parent().expect("父目录")).unwrap();
        std::fs::write(&target, b"OLD-BINARY").unwrap();

        apply_staged(&staged, &target).expect("换人");
        assert_eq!(std::fs::read(&target).unwrap(), b"NEW-BINARY", "换成功");
        assert!(!target.with_file_name("nwt.new").exists(), "不留 .new 残壳");

        // 暂存那份不在(下载或校验已经失败):一个字节都不碰旧的
        let before = std::fs::read(&target).unwrap();
        assert!(
            apply_staged(&d.join("nope"), &target).is_err(),
            "读不到暂存档即失败"
        );
        assert_eq!(std::fs::read(&target).unwrap(), before, "旧二进制原样不动");
    }

    // ---- fixture 假 release 驱动下载-校验-落盘三路 ----

    #[tokio::test]
    async fn fixture假release_对sha256_三路都过_旧二进制换成新的() -> std::io::Result<()> {
        let d = tmp("good");
        let base = d.join("base");
        fake_release(&base, "0.9.9", true)?;
        let target = d.join("bin/nwt");
        std::fs::create_dir_all(target.parent().expect("父目录"))?;
        std::fs::write(&target, b"OLD-BINARY\n")?;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let out = run(
            &UpgradeRequest {
                version: Some("0.9.9".into()),
                target: target.clone(),
                dry_run: false,
            },
            &fixture_env(&base, None),
            &tx,
        )
        .await;
        let (lines, _) = drain(&mut rx).await;
        assert_eq!(out.exit_code, 0, "对 sha256:三路全过:{out:?}");
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"FAKE-NWT-v0.9.9\n",
            "落盘的那一份就是资产里验证过的那一份"
        );
        assert!(
            lines.iter().any(|l| l.contains("sha256 校验通过(")),
            "校验结论在进度里原样呈现:{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("安装 nwt v0.9.9")),
            "解析后的版本在进度里:{lines:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn fixture假release_错sha256_闸不过旧文件字节不变() -> std::io::Result<()> {
        let d = tmp("bad");
        let base = d.join("base");
        fake_release(&base, "0.9.9", false)?;
        let target = d.join("bin/nwt");
        std::fs::create_dir_all(target.parent().expect("父目录"))?;
        std::fs::write(&target, b"OLD-BINARY\n")?;
        let before = std::fs::read(&target).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let out = run(
            &UpgradeRequest {
                version: Some("0.9.9".into()),
                target: target.clone(),
                dry_run: false,
            },
            &fixture_env(&base, None),
            &tx,
        )
        .await;
        let (_lines, done) = drain(&mut rx).await;
        assert_eq!(done.exit_code, out.exit_code);
        assert_ne!(out.exit_code, 0, "错 sha256:闸必须拒:{out:?}");
        assert!(
            out.reason.contains("sha256 校验失败"),
            "人话说清是校验不过:{:?}",
            out.reason
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            before,
            "闸不过:旧文件**逐字节**不变(不是长度不变)"
        );
        assert!(!d.join("bin/nwt.new").exists(), "连 .new 都不该留下");
        Ok(())
    }

    #[tokio::test]
    async fn 带token走api资产面_同一份fixture也装得上() -> std::io::Result<()> {
        // 私有仓的唯一可用面:带 token 的 API 资产面(install.sh 口径)。
        let d = tmp("token");
        let base = d.join("base");
        fake_release(&base, "0.9.9", true)?;
        let target = d.join("bin/nwt");
        std::fs::create_dir_all(target.parent().expect("父目录"))?;
        std::fs::write(&target, b"OLD-BINARY\n")?;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let out = run(
            &UpgradeRequest {
                version: Some("0.9.9".into()),
                target: target.clone(),
                dry_run: false,
            },
            &fixture_env(&base, Some("ghp_fixture")),
            &tx,
        )
        .await;
        let (lines, _) = drain(&mut rx).await;
        assert_eq!(out.exit_code, 0, "token 透传后 API 资产面可用:{out:?}");
        assert_eq!(std::fs::read(&target).unwrap(), b"FAKE-NWT-v0.9.9\n");
        assert!(
            !lines.iter().any(|l| l.contains("ghp_fixture")),
            "token 绝不进日志/进度:{lines:?}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn 无token且资产不存在_给人话指路_不报死旧二进制不动() -> std::io::Result<()> {
        let d = tmp("anon");
        let base = d.join("empty-base");
        std::fs::create_dir_all(&base)?;
        let target = d.join("bin/nwt");
        std::fs::create_dir_all(target.parent().expect("父目录"))?;
        std::fs::write(&target, b"OLD-BINARY\n")?;
        let before = std::fs::read(&target).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let out = run(
            &UpgradeRequest {
                version: Some("0.9.9".into()),
                target: target.clone(),
                dry_run: false,
            },
            &fixture_env(&base, None),
            &tx,
        )
        .await;
        let (_lines, _) = drain(&mut rx).await;
        assert_ne!(out.exit_code, 0, "匿名取不到资产:失败但不崩");
        assert!(
            out.reason.contains("私有仓请设 GH_TOKEN") || out.reason.contains("下载失败"),
            "人话带指路:{:?}",
            out.reason
        );
        assert_eq!(std::fs::read(&target).unwrap(), before, "旧二进制原样不动");
        Ok(())
    }

    #[tokio::test]
    async fn 钉版本与latest两路_干跑都拿得到计划() -> std::io::Result<()> {
        let d = tmp("plan");
        let base = d.join("base");
        fake_release(&base, "0.9.9", true)?;
        let target = d.join("bin/nwt");
        std::fs::create_dir_all(target.parent().expect("父目录"))?;
        std::fs::write(&target, b"OLD-BINARY\n")?;
        for (label, version) in [("钉版本", Some("0.9.9".to_string())), ("latest", None)] {
            let (tx, mut rx) = mpsc::unbounded_channel();
            let out = run(
                &UpgradeRequest {
                    version,
                    target: target.clone(),
                    dry_run: true,
                },
                &fixture_env(&base, None),
                &tx,
            )
            .await;
            let (lines, _) = drain(&mut rx).await;
            assert_eq!(out.exit_code, 0, "{label}:干跑零下载也该 rc 0:{out:?}");
            let joined = lines.join("\n");
            assert!(joined.contains("安装计划"), "{label}:计划在进度里:{joined}");
            assert!(
                joined.contains("nwt_0.9.9_linux_") && joined.contains("checksums.txt"),
                "{label}:计划摊开资产与校验文件:{joined}"
            );
            assert_eq!(
                std::fs::read(&target).unwrap(),
                b"OLD-BINARY\n",
                "{label}:干跑一个字节都不落"
            );
        }
        Ok(())
    }

    #[test]
    fn 升级环境的debug_不印token() {
        let env = UpgradeEnv {
            download_base: None,
            api_base: None,
            token: Some("ghp_super_secret".into()),
        };
        let text = format!("{env:?}");
        assert!(
            !text.contains("ghp_super_secret"),
            "token 绝不进 Debug 输出(那会进 panic 上报):{text}"
        );
        assert!(text.contains("<redacted>"), "凭证位只印占位:{text}");
    }
}
