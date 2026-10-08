//! 运行器抽象:面板与 `nw` 进程之间的唯一通道。
//!
//! - [`RealRunner`]:fork 真实 `nw`,stdout 收 envelope,stderr 行人话流;
//! - [`MissingRunner`]:找不到二进制时的替身,让 UI 走指路屏而非 panic;
//! - [`FakeRunner`]:按规则回放 fixture,测试与截屏零真实进程。

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
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
pub trait CommandRunner: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    fn run(&self, args: &[String]) -> Pin<Box<dyn Future<Output = EngineOutput> + Send + '_>>;

    fn spawn(&self, args: Vec<String>, tx: mpsc::UnboundedSender<EngineEvent>);
}

/// 真实 `nw` runner。
pub struct RealRunner {
    pub bin: PathBuf,
}

impl RealRunner {
    pub fn new(bin: PathBuf) -> Self {
        Self { bin }
    }

    fn command(&self, args: &[String]) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        cmd
    }

    async fn run_inner(&self, args: Vec<String>) -> EngineOutput {
        match self.command(&args).spawn() {
            Ok(mut child) => {
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
}

impl CommandRunner for RealRunner {
    fn name(&self) -> &'static str {
        "real"
    }

    fn run(&self, args: &[String]) -> Pin<Box<dyn Future<Output = EngineOutput> + Send + '_>> {
        Box::pin(self.run_inner(args.to_vec()))
    }

    fn spawn(&self, args: Vec<String>, tx: mpsc::UnboundedSender<EngineEvent>) {
        let bin = self.bin.clone();
        tokio::spawn(async move {
            let runner = RealRunner { bin };
            let mut cmd = match runner.command(&args).spawn() {
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
            // stderr 实时逐行推 UI(装配日志窗格的数据源)
            let tx_lines = tx.clone();
            let streamer = child_stderr_lines(cmd.stderr.take(), tx_lines);
            let output = cmd.wait_with_output().await;
            if let Some(h) = streamer {
                let _ = h.await;
            }
            let out = match output {
                Ok(o) => EngineOutput::from_parts(
                    &args,
                    o.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&o.stdout).into_owned(),
                    vec![],
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

fn child_stderr_lines(
    stderr: Option<tokio::process::ChildStderr>,
    tx: mpsc::UnboundedSender<EngineEvent>,
) -> Option<tokio::task::JoinHandle<()>> {
    stderr.map(|se| {
        tokio::spawn(async move {
            let mut lines = BufReader::new(se).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if tx.send(EngineEvent::Stderr(l)).is_err() {
                    break;
                }
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

    fn spawn(&self, args: Vec<String>, tx: mpsc::UnboundedSender<EngineEvent>) {
        let out = self.output(&args);
        let _ = tx.send(EngineEvent::Finished(Box::new(out)));
    }
}

/// 回放 runner:规则按「args 包含子串」匹配,stdout 给 fixture JSON。
pub struct FakeRunner {
    rules: Vec<(String, EngineOutput)>,
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
        }
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

    fn spawn(&self, args: Vec<String>, tx: mpsc::UnboundedSender<EngineEvent>) {
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
}
