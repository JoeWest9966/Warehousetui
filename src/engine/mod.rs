//! 引擎契约层(票 #3):与 `nw` 对话的类型、退出码、运行器抽象。
//!
//! 本模块是面板与引擎之间的**唯一**缝:Ui 永远经由 [`runner`] 的
//! 输出与 [`envelope`] 的类型看世界,凭据与环境细节不下渗到 UI。

pub mod capability;
pub mod commands;
pub mod config;
pub mod discover;
pub mod envelope;
pub mod runner;

use std::sync::Arc;

use serde::Deserialize;

pub use capability::{Capability, Gate, Probe, GATE_HINT};
#[allow(unused_imports)]
pub use envelope::{
    CatalogAddr, CatalogEntryView, CatalogGate, CatalogLogin, CatalogSheet, CheckResult,
    CheckedSource, ComponentRow, DoctorData, DoctorResult, Drift, Envelope, Evidence, LockRow,
    LsData, ManifestBrief, ProfileRow, RecheckData, RecipeRow, RegisterAcceptance, RegisterData,
    RegisterEntry, SourceRow, UpData, ValidateData, ValidateSummary, VpsEntry, VpsLsData,
    VpsRegisterData, VpsRmData,
};
#[allow(unused_imports)]
pub use runner::{CommandRunner, EngineEvent, EngineOutput, FakeRunner, MissingRunner, RealRunner};

/// 检查项通用四态:validate / doctor / source 三处同一套词。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Pass,
    Warn,
    Fail,
    Skip,
    /// `nw ls` 的 profiles[].state 与 doctor rc 一脉相承的两个健康档。
    Ok,
    Degraded,
    Drift,
    Broken,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Pass => "过",
            State::Warn => "警",
            State::Fail => "败",
            State::Skip => "略",
            State::Ok => "ok",
            State::Degraded => "降级",
            State::Drift => "漂移",
            State::Broken => "破损",
        }
    }

    /// 是否属于「装好了但没完全好」的降级家族(不算失败)。
    pub fn is_degraded_family(self) -> bool {
        matches!(self, State::Warn | State::Degraded | State::Skip)
    }
}

impl From<&str> for State {
    fn from(s: &str) -> Self {
        match s {
            "pass" => State::Pass,
            "warn" => State::Warn,
            "fail" => State::Fail,
            "skip" => State::Skip,
            "ok" => State::Ok,
            "degraded" => State::Degraded,
            "drift" => State::Drift,
            "broken" => State::Broken,
            _ => State::Skip,
        }
    }
}

/// `nw` 冻结退出码(9 个字符串码 ↔ 数字码,见调研底稿 §C)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExitCode {
    /// 0 成功。
    Ok,
    /// 2 环境缺失(NW_HOME/HOME 不可解析)。
    EnvMissing,
    /// 3 输入无效(参数/输入/validate fail/recheck 漂移)。
    InputInvalid,
    /// 3 校验失败(sha256 / 锁 schema 不认,与上面有意共用 3)。
    VerifyFailed,
    /// 4 装配失败。
    ApplyFailed,
    /// 5 锁冲突(同 profile 已有 up 在跑)。
    LockConflict,
    /// 10 doctor 降级(required env 缺 / provider 未就绪)。
    Degraded,
    /// 11 漂移(声明物被改或与锁定集不符)。
    Drift,
    /// 12 清单缺失或损坏。
    ManifestBroken,
}

impl ExitCode {
    pub fn number(self) -> i32 {
        match self {
            ExitCode::Ok => 0,
            ExitCode::EnvMissing => 2,
            ExitCode::InputInvalid | ExitCode::VerifyFailed => 3,
            ExitCode::ApplyFailed => 4,
            ExitCode::LockConflict => 5,
            ExitCode::Degraded => 10,
            ExitCode::Drift => 11,
            ExitCode::ManifestBroken => 12,
        }
    }

    /// 引擎的铁律:ok ≡ (退出码 == 0)。
    pub fn from_number(n: i32) -> Self {
        match n {
            0 => ExitCode::Ok,
            2 => ExitCode::EnvMissing,
            3 => ExitCode::InputInvalid,
            4 => ExitCode::ApplyFailed,
            5 => ExitCode::LockConflict,
            10 => ExitCode::Degraded,
            11 => ExitCode::Drift,
            12 => ExitCode::ManifestBroken,
            _ => ExitCode::InputInvalid, // 未登记码按 3 兜底,与引擎一致
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            ExitCode::Ok => "OK",
            ExitCode::EnvMissing => "ENV_MISSING",
            ExitCode::InputInvalid => "INPUT_INVALID",
            ExitCode::VerifyFailed => "VERIFY_FAILED",
            ExitCode::ApplyFailed => "APPLY_FAILED",
            ExitCode::LockConflict => "LOCK_CONFLICT",
            ExitCode::Degraded => "DEGRADED",
            ExitCode::Drift => "DRIFT",
            ExitCode::ManifestBroken => "MANIFEST_BROKEN",
        }
    }

    pub fn human(self) -> &'static str {
        match self {
            ExitCode::Ok => "成功",
            ExitCode::EnvMissing => "环境缺失:NW_HOME/HOME 不可解析",
            ExitCode::InputInvalid => "输入无效:参数或输入不合法",
            ExitCode::VerifyFailed => "校验失败:资产 sha256 不符或锁定集 schema 不认",
            ExitCode::ApplyFailed => "装配失败",
            ExitCode::LockConflict => "锁冲突:同一派已有装配在跑",
            ExitCode::Degraded => "降级:required env 缺失或 provider 未就绪",
            ExitCode::Drift => "漂移:声明物被改动或与锁定集不符",
            ExitCode::ManifestBroken => "清单缺失或损坏",
        }
    }

    pub fn is_ok(self) -> bool {
        matches!(self, ExitCode::Ok)
    }
}

/// 引擎就绪状态:顶栏与指路屏的共同数据源。
#[derive(Clone, Debug)]
pub struct EngineStatus {
    pub found: bool,
    pub bin: Option<String>,
    pub source: &'static str,
    /// 发现失败时找过的地方(指路素材)。
    pub attempted: Vec<String>,
}

impl Default for EngineStatus {
    fn default() -> Self {
        Self {
            found: false,
            bin: None,
            source: "未发现",
            attempted: Vec::new(),
        }
    }
}

/// 组装运行器 + 状态:配置 → 发现 → (真实 | 缺失替身)。
/// 返回(运行器, 引擎状态, config 解析错误证据)。
/// 第三值非空 = config.toml 坏了但已降级到默认,设置屏负责告知。
pub fn boot(explicit: Option<&str>) -> (Arc<dyn CommandRunner>, EngineStatus, Option<String>) {
    let (cfg, cfg_err) = config::load();
    let nw_home = std::env::var("NW_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.nw_home.clone())
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| format!("{h}/.nous-warehouse"))
        })
        .unwrap_or_else(|| "~/.nous-warehouse".into());
    let base = match discover::discover(explicit, &cfg, &nw_home) {
        Ok(d) => (
            Arc::new(RealRunner::new(d.bin.clone())) as Arc<dyn CommandRunner>,
            EngineStatus {
                found: true,
                bin: Some(d.bin.display().to_string()),
                source: d.source.label(),
                attempted: Vec::new(),
            },
        ),
        Err(attempted) => (
            Arc::new(MissingRunner {
                attempted: attempted.clone(),
            }) as Arc<dyn CommandRunner>,
            EngineStatus {
                found: false,
                bin: None,
                source: "未发现",
                attempted,
            },
        ),
    };
    (base.0, base.1, cfg_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 退出码表驱动() {
        for n in [0, 2, 3, 4, 5, 10, 11, 12] {
            assert_eq!(ExitCode::from_number(n).number(), n, "rc {n} 往返");
        }
        assert_eq!(ExitCode::from_number(99), ExitCode::InputInvalid);
        assert_eq!(ExitCode::LockConflict.code(), "LOCK_CONFLICT");
        assert!(ExitCode::Ok.is_ok());
        assert!(!ExitCode::Drift.is_ok());
    }

    #[test]
    fn 降级家族判定() {
        assert!(State::Warn.is_degraded_family());
        assert!(!State::Fail.is_degraded_family());
        assert!(State::Skip.is_degraded_family());
    }

    #[test]
    fn 状态串解析() {
        assert_eq!(State::from("drift"), State::Drift);
        assert_eq!(State::from("unknown"), State::Skip);
    }
}
