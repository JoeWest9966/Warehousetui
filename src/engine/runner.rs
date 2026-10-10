//! 运行器抽象:面板与 `nw` 进程之间的唯一通道。
//!
//! - [`RealRunner`]:fork 真实 `nw`,stdout 收 envelope,stderr 行人话流;
//! - [`MissingRunner`]:找不到二进制时的替身,让 UI 走指路屏而非 panic;
//! - [`FakeRunner`]:按规则回放 fixture,测试与截屏零真实进程。

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

use super::envelope::{Envelope, Evidence};
use super::ExitCode;

/// 流式事件:stderr 人话逐行来,Finished 是进程终结的完整结果。
#[derive(Clone, Debug)]
pub enum EngineEvent {
    Stderr(String),
    /// Box 住:EngineOutput ~240B,不塞会让 enum 虚胖(clippy::large_enum_variant)。
    Finished(Box<EngineOutput>),
}

/// 一次引擎调用的完整结果(无论成败都带证据)。
#[derive(Clone, Debug)]
pub struct EngineOutput {
    /// 展示用命令串,如 `nw up --lock x`。
    pub args: Vec<String>,
    pub exit_code: ExitCode,
    /// envelope 里的 ok(与 rc==0 等价,缺失时按 rc 推导)。
    pub ok: bool,
    /// stderr 行人话(截尾保留,防爆内存)。
    pub stderr_tail: Vec<String>,
    pub envelope: Option<Envelope>,
    pub raw_stdout: String,
    pub parse_error: Option<String>,
}

impl EngineOutput {
    pub fn command_string(&self) -> String {
        format!("nw {}", self.args.join(" "))
    }

    /// 「人话一句」:parse_error > 契约守卫「需升级」> 错误 message > rc 人话。
    pub fn headline(&self) -> String {
        if let Some(pe) = &self.parse_error {
            return format!("stdout 不是合法 envelope:{pe}");
        }
        if let Some(env) = &self.envelope {
            // 契约守卫(#28):schema 不认识时,面板自己的结论顶到最前,
            // 不引用引擎原话(原话的形状已经不可信)。
            if let Some(g) = &env.schema_guard {
                return g.message.clone();
            }
            return env.headline().to_string();
        }
        self.exit_code.human().to_string()
    }

    /// 证据条目:错误优先、警告兜底;用于结果屏折叠展示。
    /// schema 不认识时只给契约守卫的「需升级」证据——引擎对不认识的
    /// envelope 说的任何话都不采信(不降级解析)。
    pub fn evidence(&self) -> Vec<&Evidence> {
        match &self.envelope {
            Some(e) => {
                if let Some(g) = &e.schema_guard {
                    return vec![g];
                }
                let mut v: Vec<&Evidence> = e.errors.iter().collect();
                v.extend(e.warnings.iter());
                v
            }
            None => Vec::new(),
        }
    }

    fn from_parts(args: &[String], code: i32, stdout: String, stderr_tail: Vec<String>) -> Self {
        let trimmed = stdout.trim();
        let (envelope, parse_error, ok) = if trimmed.is_empty() {
            (None, Some("stdout 为空".into()), code == 0)
        } else {
            match serde_json::from_str::<Envelope>(trimmed) {
                Ok(mut e) => {
                    let ok = e.ok && code == 0;
                    // 契约守卫(#28):解析点认识性检查,不认识就带上
                    // 「需升级」证据(携引擎真实 rc),并禁止 data 拆型。
                    e.apply_schema_guard(code);
                    (Some(e), None, ok)
                }
                Err(e) => (None, Some(e.to_string()), code == 0),
            }
        };
        EngineOutput {
            args: args.to_vec(),
            exit_code: ExitCode::from_number(code),
            ok,
            stderr_tail,
            envelope,
            raw_stdout: stdout,
            parse_error,
        }
    }
}

/// 运行器 trait:真实/缺失/回放三种实现共用。
///
/// `run` 一次性执行;`spawn` 流式执行(stderr 实时推送)。
/// 返回 `Pin<Box<dyn Future>>` 而非 `async fn`,保持 object-safe
/// (`Arc<dyn CommandRunner>` 可注入 App,测试可换 FakeRunner)。
///
/// # stdin 喂入通道(#55)
///
/// `run_with_stdin` / `spawn_with_stdin` 是**单独**的喂入路径:本体(密钥 /
/// 订阅链接)只从 stdin 进(argv 面无任何明文形参 —— 误粘的 argv 会进 ps /
/// history)。喂入内容是**一次性值**:运行器原样写进子进程 stdin 后立即 drop,
/// 绝不进 `EngineOutput` / `EngineEvent` / `App` 任何字段,也不进日志 / 配置 /
/// 崩溃上报。不喂入时走 `run` / `spawn`,行为与开此通道前逐字节一致。把「能喂
/// stdin」与「不能喂 stdin」分成两个方法,是让误用在类型层就难看得出。
///
/// 「stdin 是终端会一直挂住等人敲」由**引擎侧**拦(上游 `--token-stdin` 的
/// TTY 闸),面板只经管道喂入、永不复刻那道判定。
pub trait CommandRunner: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    fn run(&self, args: &[String]) -> Pin<Box<dyn Future<Output = EngineOutput> + Send + '_>>;

    fn spawn(&self, args: Vec<String>, tx: mpsc::UnboundedSender<EngineEvent>);

    /// stdin 喂入路径(一次性执行):把 `stdin` 原样写进子进程 stdin 后立即 drop。
    /// `stdin` 是本体字节(密钥 / 订阅链接),绝不进返回的 `EngineOutput`。
    fn run_with_stdin(
        &self,
        args: &[String],
        stdin: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = EngineOutput> + Send + '_>>;

    /// stdin 喂入路径(流式执行):同 `run_with_stdin`,stderr 实时推送。
    fn spawn_with_stdin(
        &self,
        args: Vec<String>,
        stdin: Vec<u8>,
        tx: mpsc::UnboundedSender<EngineEvent>,
    );
}

/// 真实 `nw` runner。
pub struct RealRunner {
    pub bin: PathBuf,
}

impl RealRunner {
    pub fn new(bin: PathBuf) -> Self {
        Self { bin }
    }

    /// 建命令。`feed_stdin = false` 时 stdin 关掉(`Stdio::null()`),与开 stdin
    /// 通道前逐字节一致;`true` 时开管道,供 `run_with_stdin` 喂入本体
    /// (机密只从 stdin 进,argv 面无任何明文形参)。
    fn command(&self, args: &[String], feed_stdin: bool) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(if feed_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        cmd
    }

    async fn run_inner(&self, args: Vec<String>) -> EngineOutput {
        self.run_inner_fed(args, None).await
    }

    /// `stdin = Some(字节)` 时开管道并喂入本体(写后即关、即弃);`None` 时不喂入,
    /// stdin 仍关掉 —— 与开 stdin 通道前逐字节一致。喂入内容绝不进 EngineOutput。
    async fn run_inner_fed(&self, args: Vec<String>, stdin: Option<Vec<u8>>) -> EngineOutput {
        let spawned = if stdin.is_some() {
            self.command(&args, true).spawn()
        } else {
            self.command(&args, false).spawn()
        };
        match spawned {
            Ok(mut child) => {
                // 喂入(#55):原样写进子进程 stdin 后立即关闭;内容随进程结束即弃,
                // 不进 EngineOutput / 日志 / 配置任何一处。
                if let (Some(bytes), Some(mut sink)) = (stdin, child.stdin.take()) {
                    tokio::spawn(async move {
                        let _ = sink.write_all(&bytes).await;
                        // sink 关闭(子进程读到 EOF),bytes 在此作用域结束即弃
                    });
                }
                // stderr 收集(一次性执行场景给日志尾巴)
                let (stx, mut srx) = mpsc::unbounded_channel::<String>();
                if let Some(se) = child.stderr.take() {
                    tokio::spawn(async move {
                        let mut lines = BufReader::new(se).lines();
                        while let Ok(Some(l)) = lines.next_line().await {
                            if stx.send(l).is_err() {
                                break;
                            }
                        }
                    });
                }
                let output = match child.wait_with_output().await {
                    Ok(o) => o,
                    Err(e) => {
                        return EngineOutput::from_parts(
                            &args,
                            -1,
                            String::new(),
                            vec![format!("等待进程结束失败:{e}")],
                        );
                    }
                };
                let mut tail: Vec<String> = Vec::new();
                while let Ok(l) = srx.try_recv() {
                    tail.push(l);
                }
                const CAP: usize = 200;
                if tail.len() > CAP {
                    tail.drain(0..tail.len() - CAP);
                }
                EngineOutput::from_parts(
                    &args,
                    output.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&output.stdout).into_owned(),
                    tail,
                )
            }
            Err(e) => EngineOutput::from_parts(
                &args,
                -1,
                String::new(),
                vec![format!("启动 {} 失败:{e}", self.bin.display())],
            ),
        }
    }

    /// `spawn` 的喂入变体:`stdin = Some(字节)` 时开管道并喂入(写后即关、即弃),
    /// `None` 时不喂入(stdin 关掉,与改动前逐字节一致)。
    fn spawn_fed(
        &self,
        args: Vec<String>,
        stdin: Option<Vec<u8>>,
        tx: mpsc::UnboundedSender<EngineEvent>,
    ) {
        let bin = self.bin.clone();
        tokio::spawn(async move {
            let runner = RealRunner { bin };
            let spawned = if stdin.is_some() {
                runner.command(&args, true).spawn()
            } else {
                runner.command(&args, false).spawn()
            };
            let mut cmd = match spawned {
                Ok(c) => c,
                Err(e) => {
                    let out = EngineOutput::from_parts(
                        &args,
                        -1,
                        String::new(),
                        vec![format!("启动 {} 失败:{e}", runner.bin.display())],
                    );
                    let _ = tx.send(EngineEvent::Finished(Box::new(out)));
                    return;
                }
            };
            // 喂入(#55):原样写进子进程 stdin 后立即关闭;None = 不喂入,
            // 与开 stdin 通道前逐字节一致。内容随进程结束即弃,不进任何结构。
            if let (Some(bytes), Some(mut sink)) = (stdin, cmd.stdin.take()) {
                tokio::spawn(async move {
                    let _ = sink.write_all(&bytes).await;
                });
            }
            // stderr 实时逐行推 UI(装配日志窗格的数据源),同时收拢成
            // Finished 的尾巴 —— 版本探测等从 stderr 读真相的调用要用它
            // (nw --version 的裸版本号在 stderr;尾巴空 = 顶栏永远「未知」)。
            let (collect_tx, mut collect_rx) = mpsc::unbounded_channel::<String>();
            let tx_lines = tx.clone();
            let streamer = child_stderr_lines(cmd.stderr.take(), tx_lines, collect_tx);
            let output = cmd.wait_with_output().await;
            if let Some(h) = streamer {
                let _ = h.await;
            }
            let mut tail: Vec<String> = Vec::new();
            while let Ok(l) = collect_rx.try_recv() {
                tail.push(l);
            }
            const CAP: usize = 200;
            if tail.len() > CAP {
                tail.drain(0..tail.len() - CAP);
            }
            let out = match output {
                Ok(o) => EngineOutput::from_parts(
                    &args,
                    o.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&o.stdout).into_owned(),
                    tail,
                ),
                Err(e) => {
                    let out = EngineOutput::from_parts(
                        &args,
                        -1,
                        String::new(),
                        vec![format!("等待进程结束失败:{e}")],
                    );
                    out
                }
            };
            let _ = tx.send(EngineEvent::Finished(Box::new(out)));
        });
    }
}

impl CommandRunner for RealRunner {
    fn name(&self) -> &'static str {
        "real"
    }

    fn run(&self, args: &[String]) -> Pin<Box<dyn Future<Output = EngineOutput> + Send + '_>> {
        Box::pin(self.run_inner(args.to_vec()))
    }

    fn run_with_stdin(
        &self,
        args: &[String],
        stdin: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = EngineOutput> + Send + '_>> {
        Box::pin(self.run_inner_fed(args.to_vec(), Some(stdin)))
    }

    fn spawn(&self, args: Vec<String>, tx: mpsc::UnboundedSender<EngineEvent>) {
        self.spawn_fed(args, None, tx);
    }

    fn spawn_with_stdin(
        &self,
        args: Vec<String>,
        stdin: Vec<u8>,
        tx: mpsc::UnboundedSender<EngineEvent>,
    ) {
        self.spawn_fed(args, Some(stdin), tx);
    }
}

fn child_stderr_lines(
    stderr: Option<tokio::process::ChildStderr>,
    tx: mpsc::UnboundedSender<EngineEvent>,
    collect: mpsc::UnboundedSender<String>,
) -> Option<tokio::task::JoinHandle<()>> {
    stderr.map(|se| {
        tokio::spawn(async move {
            let mut lines = BufReader::new(se).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if tx.send(EngineEvent::Stderr(l.clone())).is_err() {
                    break;
                }
                let _ = collect.send(l);
            }
        })
    })
}

/// 找不到 `nw` 时的替身:任何调用都返回 EnvMissing + 指路证据。
pub struct MissingRunner {
    pub attempted: Vec<String>,
}

impl MissingRunner {
    fn output(&self, args: &[String]) -> EngineOutput {
        let hint = if self.attempted.is_empty() {
            "PATH、--nw-bin、配置文件、$NW_HOME/nw 都没找到 nw".to_string()
        } else {
            format!("找过:{}", self.attempted.join("、"))
        };
        EngineOutput {
            args: args.to_vec(),
            exit_code: ExitCode::EnvMissing,
            ok: false,
            stderr_tail: vec![hint],
            envelope: None,
            raw_stdout: String::new(),
            parse_error: Some("nw 二进制缺失".into()),
        }
    }
}

impl CommandRunner for MissingRunner {
    fn name(&self) -> &'static str {
        "missing"
    }

    fn run(&self, args: &[String]) -> Pin<Box<dyn Future<Output = EngineOutput> + Send + '_>> {
        let out = self.output(args);
        Box::pin(async move { out })
    }

    fn run_with_stdin(
        &self,
        args: &[String],
        stdin: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = EngineOutput> + Send + '_>> {
        // 没有进程可喂:本体即弃(EnvMissing 与是否喂入无关,绝不保留内容)。
        drop(stdin);
        let out = self.output(args);
        Box::pin(async move { out })
    }

    fn spawn(&self, args: Vec<String>, tx: mpsc::UnboundedSender<EngineEvent>) {
        let out = self.output(&args);
        let _ = tx.send(EngineEvent::Finished(Box::new(out)));
    }

    fn spawn_with_stdin(
        &self,
        args: Vec<String>,
        stdin: Vec<u8>,
        tx: mpsc::UnboundedSender<EngineEvent>,
    ) {
        drop(stdin); // 没有进程可喂:本体即弃
        let out = self.output(&args);
        let _ = tx.send(EngineEvent::Finished(Box::new(out)));
    }
}

/// 回放 runner:规则按「args 包含子串」匹配,stdout 给 fixture JSON。
pub struct FakeRunner {
    rules: Vec<(String, EngineOutput)>,
    /// 记录「收到过几次 stdin 喂入」——只记次数,**绝不存内容**(铁律:面板侧不留
    /// 副本)。测试靠它守「喂入确有一次到达运行器」,而内容一个字节都不进结构。
    stdin_feeds: AtomicUsize,
}

impl FakeRunner {
    /// 由 (args 子串, rc, fixture JSON) 列表构造。
    pub fn new(rules: &[(&str, i32, &str)]) -> Self {
        Self {
            rules: rules
                .iter()
                .map(|(needle, rc, json)| {
                    (
                        needle.to_string(),
                        EngineOutput::from_parts(
                            &[needle.to_string()],
                            *rc,
                            json.to_string(),
                            vec![],
                        ),
                    )
                })
                .collect(),
            stdin_feeds: AtomicUsize::new(0),
        }
    }

    /// 由 (args 子串, rc, fixture JSON, stderr 人话) 列表构造。
    /// stderr 是能力探测的判定面(#29:引擎亲口「未知子命令」),
    /// 三元构造给不出 stderr,故另开此路;既有三元构造不动。
    pub fn with_stderr(rules: &[(&str, i32, &str, &str)]) -> Self {
        Self {
            rules: rules
                .iter()
                .map(|(needle, rc, json, stderr)| {
                    let lines: Vec<String> = stderr.lines().map(String::from).collect();
                    (
                        needle.to_string(),
                        EngineOutput::from_parts(
                            &[needle.to_string()],
                            *rc,
                            json.to_string(),
                            lines,
                        ),
                    )
                })
                .collect(),
            stdin_feeds: AtomicUsize::new(0),
        }
    }

    /// 测试探针:这条 FakeRunner 累计收到过几次 stdin 喂入(只记次数,**绝不存内容**)。
    pub fn stdin_feed_count(&self) -> usize {
        self.stdin_feeds.load(Ordering::SeqCst)
    }

    fn match_for(&self, args: &[String]) -> EngineOutput {
        let joined = args.join(" ");
        self.rules
            .iter()
            .find(|(needle, _)| joined.contains(needle.as_str()))
            .map(|(_, o)| o.clone())
            .unwrap_or_else(|| {
                EngineOutput::from_parts(
                    args,
                    0,
                    r#"{"schema":"nw/ls.v1","nw":"fake","ok":true,"errors":[],"warnings":[],"data":{"domain":"all","recipes":[],"components":[],"sources":[],"profiles":[],"locks":[]}}"#.to_string(),
                    vec![],
                )
            })
    }
}

impl CommandRunner for FakeRunner {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn run(&self, args: &[String]) -> Pin<Box<dyn Future<Output = EngineOutput> + Send + '_>> {
        let mut out = self.match_for(args);
        out.args = args.to_vec();
        Box::pin(async move { out })
    }

    fn run_with_stdin(
        &self,
        args: &[String],
        stdin: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = EngineOutput> + Send + '_>> {
        // 记「收到过」即弃:内容绝不进任何结构(只计数,铁律面板侧不留副本)。
        self.stdin_feeds.fetch_add(1, Ordering::SeqCst);
        drop(stdin);
        let mut out = self.match_for(args);
        out.args = args.to_vec();
        Box::pin(async move { out })
    }

    fn spawn(&self, args: Vec<String>, tx: mpsc::UnboundedSender<EngineEvent>) {
        let mut out = self.match_for(&args);
        out.args = args;
        let _ = tx.send(EngineEvent::Finished(Box::new(out)));
    }

    fn spawn_with_stdin(
        &self,
        args: Vec<String>,
        stdin: Vec<u8>,
        tx: mpsc::UnboundedSender<EngineEvent>,
    ) {
        self.stdin_feeds.fetch_add(1, Ordering::SeqCst);
        drop(stdin); // 记「收到过」即弃:内容绝不进任何结构
        let mut out = self.match_for(&args);
        out.args = args;
        let _ = tx.send(EngineEvent::Finished(Box::new(out)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LS_FIXTURE: &str = r#"{"schema":"nw/ls.v1","nw":"0.1.0","ok":true,"errors":[],"warnings":[],"data":{"domain":"all","recipes":[{"name":"research-desk","version":"0.2.0"}],"components":[],"sources":[],"profiles":[{"name":"desk","state":"drift","lockId":"9f8e"}],"locks":[]}}"#;

    #[tokio::test]
    async fn fake_runner_按子串回放() {
        let r = FakeRunner::new(&[("ls", 0, LS_FIXTURE)]);
        let out = r.run(&["ls".into(), "--json".into()]).await;
        assert!(out.ok);
        let data: super::super::LsData = out.envelope.as_ref().unwrap().data_as().unwrap();
        assert_eq!(data.profiles[0].name, "desk");
        assert_eq!(out.command_string(), "nw ls --json");
    }

    #[tokio::test]
    async fn fake_runner_失败码与证据() {
        let r = FakeRunner::new(&[(
            "doctor",
            11,
            r#"{"schema":"nw/doctor.v1","nw":"0.1.0","ok":false,"errors":[{"code":"DRIFT","exitCode":11,"message":"漂了","hint":"重跑装配命令","details":{"surface":"config"}}],"warnings":[],"data":{"results":[],"exit":11}}"#,
        )]);
        let out = r
            .run(&["doctor".into(), "--profile".into(), "desk".into()])
            .await;
        assert!(!out.ok);
        assert_eq!(out.exit_code, ExitCode::Drift);
        assert_eq!(out.headline(), "漂了");
        assert_eq!(out.evidence().len(), 1);
    }

    #[tokio::test]
    async fn missing_runner_给指路证据() {
        let r = MissingRunner {
            attempted: vec!["PATH".into(), "$NW_HOME/nw".into()],
        };
        let out = r.run(&["ls".into()]).await;
        assert_eq!(out.exit_code, ExitCode::EnvMissing);
        assert!(out.parse_error.is_some());
        assert!(out.stderr_tail[0].contains("PATH"));
    }

    #[tokio::test]
    async fn spawn_推送_finished() {
        let r = FakeRunner::new(&[("ls", 0, LS_FIXTURE)]);
        let (tx, mut rx) = mpsc::unbounded_channel();
        r.spawn(vec!["ls".into()], tx);
        match rx.recv().await.unwrap() {
            EngineEvent::Finished(o) => assert!(o.ok),
            _ => panic!("应立即 Finished"),
        }
    }

    #[tokio::test]
    async fn spawn_真实进程_stderr收进尾巴() {
        // 回归:spawn 路径曾把 Finished 的 stderr_tail 落成空 Vec ——
        // 版本探测(nw --version 的裸版本号在 stderr)永远解析出「未知」,
        // 顶栏从此报错引擎版本。真进程:拿测试二进制自己认不动的旗标,
        // libtest 会把 "Unrecognized option" 写到 stderr 并非零退出。
        let exe = std::env::current_exe().expect("当前测试二进制路径");
        let r = RealRunner::new(exe);
        let (tx, mut rx) = mpsc::unbounded_channel();
        r.spawn(vec!["--nonexistent-flag-xyz".into()], tx);
        let mut lines = 0usize;
        let mut finished = None;
        while let Some(ev) = rx.recv().await {
            match ev {
                EngineEvent::Stderr(_) => lines += 1,
                EngineEvent::Finished(o) => {
                    finished = Some(o);
                    break;
                }
            }
        }
        let out = finished.expect("必须收到 Finished");
        assert!(lines > 0, "stderr 应实时逐行推 UI");
        assert!(
            !out.stderr_tail.is_empty(),
            "尾巴必须收拢 stderr(不再空 Vec)"
        );
        assert!(
            out.stderr_tail
                .iter()
                .any(|l| l.contains("Unrecognized option")),
            "尾巴带 stderr 原文:{:?}",
            out.stderr_tail
        );
    }

    // ---- #28 契约守卫:解析点合证 + 输出面只采信守卫 ----

    const UNKNOWN_SCHEMA_LS: &str = r#"{"schema":"nw/ls.v2","nw":"9.9.9","ok":true,"errors":[{"code":"X","exitCode":0,"message":"引擎自己的话","hint":null,"details":null}],"warnings":[],"data":{"domain":"all","recipes":[{"name":"x"}]}}"#;

    #[tokio::test]
    async fn 未知_schema_headline_与_evidence_只给需升级证据() {
        let r = FakeRunner::new(&[("ls", 0, UNKNOWN_SCHEMA_LS)]);
        let out = r.run(&["ls".into()]).await;
        assert!(out.ok, "守卫不改写引擎的 ok≡rc 判定");
        let h = out.headline();
        assert!(
            h.contains("需升级 nw") && h.contains("nw/ls.v2") && h.contains("9.9.9"),
            "一句话含需升级 + schema 原文 + 引擎版本:{h}"
        );
        let evs = out.evidence();
        assert_eq!(evs.len(), 1, "不认识的 envelope 话不采信,只给守卫证据");
        assert_eq!(evs[0].code.as_deref(), Some("SCHEMA_UNKNOWN"));
        let d = evs[0].details.as_ref().unwrap();
        assert_eq!(d["schema"], "nw/ls.v2");
        assert_eq!(d["nw"], "9.9.9");
        // 降级解析被禁止:形状再像也不拆
        assert!(out
            .envelope
            .as_ref()
            .unwrap()
            .data_as::<super::super::LsData>()
            .is_none());
    }

    #[tokio::test]
    async fn 未知_schema_rc11_守卫证据带引擎真实rc() {
        let r = FakeRunner::new(&[(
            "doctor",
            11,
            r#"{"schema":"nw/doctor.v2","nw":"2.0.0","ok":false,"errors":[],"warnings":[],"data":{"exit":11}}"#,
        )]);
        let out = r
            .run(&["doctor".into(), "--profile".into(), "desk".into()])
            .await;
        assert_eq!(out.exit_code, ExitCode::Drift);
        let evs = out.evidence();
        assert_eq!(evs[0].exit_code, Some(11), "守卫证据携引擎真实 rc");
        assert!(evs[0].message.contains("需升级 nw"));
    }

    #[tokio::test]
    async fn 认识的_schema_不产生守卫_旧样照常() {
        let r = FakeRunner::new(&[("ls", 0, LS_FIXTURE)]);
        let out = r.run(&["ls".into()]).await;
        assert!(out.envelope.as_ref().unwrap().schema_guard.is_none());
        let d: super::super::LsData = out.envelope.as_ref().unwrap().data_as().unwrap();
        assert_eq!(d.profiles[0].name, "desk");
        assert_eq!(out.headline(), "完成");
    }

    // ---- #55 stdin 喂入通道:只铺管子,不开放任何命令 ----

    #[tokio::test]
    async fn 真实运行器_把一段文本喂进子进程stdin() {
        // 用只回显 stdin 的 helper(`cat`)当子进程:喂进去的标记应从 stdout 回来,
        // 证明运行器确能把本体写进子进程 stdin(机密本体的喂入前提)。
        let r = RealRunner::new(PathBuf::from("cat"));
        let marker = "nw-body-feed-55";
        let out = r
            .run_with_stdin(&[], format!("{marker}\n").into_bytes())
            .await;
        assert_eq!(out.exit_code, ExitCode::Ok, "cat 读到 EOF 后退出 0");
        assert!(
            out.raw_stdout.contains(marker),
            "标记经 cat 回显,证明喂进了子进程 stdin:{:?}",
            out.raw_stdout
        );
    }

    #[tokio::test]
    async fn 真实运行器_spawn_也把一段文本喂进子进程stdin() {
        let r = RealRunner::new(PathBuf::from("cat"));
        let marker = "nw-body-feed-spawn-55";
        let (tx, mut rx) = mpsc::unbounded_channel();
        r.spawn_with_stdin(vec![], format!("{marker}\n").into_bytes(), tx);
        let mut finished = None;
        while let Some(ev) = rx.recv().await {
            if let EngineEvent::Finished(o) = ev {
                finished = Some(o);
                break;
            }
        }
        let out = finished.expect("必须收到 Finished");
        assert_eq!(out.exit_code, ExitCode::Ok);
        assert!(
            out.raw_stdout.contains(marker),
            "spawn 路径同样把标记喂进 cat stdin:{:?}",
            out.raw_stdout
        );
    }

    #[tokio::test]
    async fn 喂入内容不落入_engine_output_任何字段_也不进argv() {
        // 用一个不读 stdin 的子进程(测试二进制匹配不到测试即退),喂入唯一标记;
        // 子进程退出后,返回的 EngineOutput 的 Debug(含所有字段)都不该含这个标记,
        // argv / command_string 也不含 —— 本体只从 stdin 进,绝不落到结果结构。
        let exe = std::env::current_exe().expect("当前测试二进制路径");
        let r = RealRunner::new(exe);
        let marker = "FEED-SECRET-55-DO-NOT-LEAK";
        let out = r
            .run_with_stdin(
                &["--exact".into(), "this_test_nonexistent_xyz".into()],
                marker.as_bytes().to_vec(),
            )
            .await;
        let dbg = format!("{out:?}");
        assert!(
            !dbg.contains(marker),
            "标记不该出现在 EngineOutput 的任何字段(喂入即弃):{dbg}"
        );
        assert!(
            !out.args.iter().any(|a| a.contains(marker)),
            "标记不进 argv:{:?}",
            out.args
        );
        assert!(!out.command_string().contains(marker));
    }

    #[tokio::test]
    async fn 未喂入时_stdin仍是关掉的_子进程立刻读到eof() {
        // 回归:未喂入路径必须保持 Stdio::null()(与改动前逐字节一致)。
        // cat 读 stdin 到 EOF 才退出:stdin 若是 null,cat 立即 EOF、输出为空、退 0;
        // 若误开了空管道且不关,cat 会挂住等数据 —— 超时即视为回归。
        let r = RealRunner::new(PathBuf::from("cat"));
        let out = tokio::time::timeout(std::time::Duration::from_secs(5), r.run(&[]))
            .await
            .expect("未喂入路径不该挂住:stdin 必须关掉");
        assert!(
            out.raw_stdout.is_empty(),
            "stdin 关掉,cat 读不到字节:{:?}",
            out.raw_stdout
        );
        assert_eq!(out.exit_code, ExitCode::Ok, "cat 应立即 EOF 并退出 0");
    }

    #[tokio::test]
    async fn fake_runner_记录收到stdin喂入_但不存内容() {
        // FakeRunner 能表达「收到 / 没收到 stdin 喂入」(计数),否则测试守不住;
        // 且只记次数、绝不存内容。未喂入的 run/spawn 不增加计数。
        let r = FakeRunner::new(&[("ls", 0, LS_FIXTURE)]);
        assert_eq!(r.stdin_feed_count(), 0, "起初没收到喂入");
        let _ = r.run(&["ls".into()]).await;
        assert_eq!(r.stdin_feed_count(), 0, "未喂入的 run 不计数");
        let _ = r.run_with_stdin(&["ls".into()], b"secret".to_vec()).await;
        assert_eq!(r.stdin_feed_count(), 1, "喂入一次计一次");
        // 回放内容与未喂入一致,且喂入内容绝不进结构
        let out = r
            .run_with_stdin(&["ls".into()], b"another-secret".to_vec())
            .await;
        assert_eq!(r.stdin_feed_count(), 2);
        assert!(
            !format!("{out:?}").contains("another-secret"),
            "FakeRunner 只记次数、不存喂入内容"
        );
    }

    #[tokio::test]
    async fn missing_runner_喂入路径即弃_仍给env_missing() {
        // MissingRunner 没有进程可喂:喂入方法即弃本体,结论与未喂入一致(EnvMissing)。
        let r = MissingRunner {
            attempted: vec!["PATH".into()],
        };
        let out = r.run_with_stdin(&["key".into()], b"secret".to_vec()).await;
        assert_eq!(out.exit_code, ExitCode::EnvMissing);
        assert!(
            !format!("{out:?}").contains("secret"),
            "MissingRunner 即弃喂入内容"
        );
    }
}
