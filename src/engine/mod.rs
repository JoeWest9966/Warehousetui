//! 引擎契约层(票 #3):与 `nw` 对话的类型、退出码、运行器抽象。
//!
//! 本模块是面板与引擎之间的**唯一**缝:Ui 永远经由 [`runner`] 的
//! 输出与 [`envelope`] 的类型看世界,凭据与环境细节不下渗到 UI。

pub mod envelope;

use serde::Deserialize;

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
            other => {
                let _ = other;
                ExitCode::InputInvalid // 未登记码按 3 兜底,与引擎一致
            }
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
}
