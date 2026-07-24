// EasyNet CLI — Timeout Tower
// ===========================
//
// File: src/shared/timeouts.rs
// Description: Single source of truth for every timeout constant in
//              the CLI — both the user-visible `--timeout` defaults
//              and the internal plumbing deadlines (daemon connect,
//              …). Values are declared *here* and referenced by each
//              call site, so help text, compiled-in value, and
//              cross-command policy cannot drift from one another.
//
// Design — two layers, one file
// -----------------------------
//
// The CLI has two kinds of deadline, and both belong in the tower:
//
// 1. **User-surface** (`--timeout <N>` defaults). Commands fall into
//    three buckets chosen for a human's wall-clock expectation of the
//    operation:
//
//     - [`INVOKE_DEFAULT_SECS`]      — ability invocations over unary,
//                                      stream, bidi, exec, and recording
//                                      surfaces. The concrete transport guard
//                                      is one hour because real tool-using
//                                      abilities can legitimately run for
//                                      minutes-to-tens-of-minutes.
//     - [`AGENT_SEND_DEFAULT_SECS`]  — LLM-backed dispatches (Claude
//                                      Code / Codex). These can stream
//                                      for many minutes on a large
//                                      prompt + heavy thinking budget,
//                                      so the floor is 15 min and
//                                      users can raise it explicitly.
//     - [`THINK_DEFAULT_SECS`]       — per-cycle budget inside the
//                                      autonomous loop. Each cycle is
//                                      one think + one action; it shares the
//                                      same one-hour invocation budget.
//
// The number `0` is interpreted by a named [`TimeoutPolicy`]. Payload
// deadlines may preserve `0` as "inherit the runtime default". Transport
// guards cannot be absent, so they explicitly resolve `0` to the command's
// configured guard deadline instead of leaving each CLI command to hand-roll a
// fallback.
//
// Why the infrastructure constant lives here, not in `shared::mod`
// ----------------------------------------------------------------
//
// All timeout policy lives here so callers cannot silently drift.
//
// Unit convention
// ---------------
//
// User-surface constants are `_SECS` because `--timeout` is in
// seconds. Infrastructure constants are `_MS` because the bridge SDK
// takes milliseconds directly. The suffix is always part of the name
// so a call site cannot mis-unit a value.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

/// Default deadline for `easynet invoke` / `easynet ability invoke`,
/// in seconds. The runtime may impose a tighter ceiling; this is only
/// the CLI-level floor.
///
/// Set to 1 hour because the invoke surface routes to `<agent>.chat`
/// for agent abilities, and a real LLM with tool use can legitimately
/// run minutes-to-tens-of-minutes (mission.think cycles, multi-step
/// agent tool loops). A 60 s floor was forcing an unhelpful retry/
/// re-issue pattern from operators every time the model went into a
/// long sequence of tool calls. Operators who want a tighter ceiling
/// pass `--timeout <N>` explicitly.
pub const INVOKE_DEFAULT_SECS: u64 = 3600;

/// Default deadline for `easynet agent send` (LLM-backed dispatch), in
/// seconds. LLM responses can legitimately take many minutes when the
/// prompt is large or the model is reasoning, so the floor is 1 hour.
/// Same rationale as `INVOKE_DEFAULT_SECS`: tool-using agents can
/// legitimately run for tens of minutes without being stuck.
pub const AGENT_SEND_DEFAULT_SECS: u64 = 3600;

/// Default per-cycle deadline for `easynet think`, in seconds.
/// Test-only today — `easynet think` is not yet wired through this
/// constant in production code. Kept under `#[cfg(test)]` so a
/// future caller picks the same 1-hour budget when it lands.
#[cfg(test)]
pub const THINK_DEFAULT_SECS: u64 = 3600;

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ZeroTimeoutPolicy {
    RuntimeDefault,
    DefaultTransportGuard,
}

/// Canonical timeout policy for a CLI surface.
///
/// The policy owns the difference between an optional runtime request deadline
/// and a mandatory local transport guard. Callers must choose one of the named
/// methods rather than converting `0` and then applying ad-hoc fallbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutPolicy {
    default_secs: u64,
    zero: ZeroTimeoutPolicy,
}

impl TimeoutPolicy {
    pub const fn runtime_request_default(default_secs: u64) -> Self {
        Self {
            default_secs,
            zero: ZeroTimeoutPolicy::RuntimeDefault,
        }
    }

    pub const fn transport_guard_default(default_secs: u64) -> Self {
        Self {
            default_secs,
            zero: ZeroTimeoutPolicy::DefaultTransportGuard,
        }
    }

    pub fn request_timeout_ms(self, secs: u64) -> Result<Option<u64>, &'static str> {
        effective_ms(secs)
    }

    pub fn transport_guard(self, secs: u64) -> Result<Duration, &'static str> {
        let millis = match (self.zero, effective_ms(secs)?) {
            (_, Some(ms)) => ms,
            (ZeroTimeoutPolicy::DefaultTransportGuard, None) => self.default_ms()?,
            (ZeroTimeoutPolicy::RuntimeDefault, None) => {
                return Err("transport guard requires a concrete timeout")
            }
        };
        Ok(Duration::from_millis(millis))
    }

    fn default_ms(self) -> Result<u64, &'static str> {
        self.default_secs
            .checked_mul(1000)
            .ok_or("timeout default is too large (overflow converting seconds to milliseconds)")
    }
}

pub const INVOCATION_TRANSPORT_TIMEOUT: TimeoutPolicy =
    TimeoutPolicy::transport_guard_default(INVOKE_DEFAULT_SECS);
pub const RUNTIME_REQUEST_TIMEOUT: TimeoutPolicy =
    TimeoutPolicy::runtime_request_default(INVOKE_DEFAULT_SECS);

/// Convert a user-facing seconds value (from a `--timeout <N>` flag) to
/// the `Option<Duration>`-in-milliseconds shape used by request payloads.
///
/// `0` is the canonical "inherit the runtime default" sentinel: it maps
/// to `None`, which the bridge layer interprets as "use whatever the
/// called ability specified". Any positive value is converted to
/// milliseconds with overflow protection — a user passing `u64::MAX`
/// seconds would otherwise panic in debug and wrap in release, neither
/// of which we want at a CLI surface.
pub fn effective_ms(secs: u64) -> Result<Option<u64>, &'static str> {
    match secs {
        0 => Ok(None),
        s => s
            .checked_mul(1000)
            .map(Some)
            .ok_or("--timeout is too large (overflow converting seconds to milliseconds)"),
    }
}

pub fn invocation_transport_guard(secs: u64) -> Result<Duration, &'static str> {
    INVOCATION_TRANSPORT_TIMEOUT.transport_guard(secs)
}

pub fn runtime_request_timeout_ms(secs: u64) -> Result<Option<u64>, &'static str> {
    RUNTIME_REQUEST_TIMEOUT.request_timeout_ms(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_means_inherit_runtime_default() {
        assert_eq!(effective_ms(0), Ok(None));
    }

    #[test]
    fn positive_seconds_round_trip_to_milliseconds() {
        assert_eq!(effective_ms(1), Ok(Some(1_000)));
        // The three LLM-dispatch defaults all sit at 1 hour. A
        // regression that drops one to a sub-minute floor would
        // re-introduce the "timeout fires mid-cycle" UX bug; pin
        // the floor here.
        assert_eq!(effective_ms(INVOKE_DEFAULT_SECS), Ok(Some(3_600_000)));
        assert_eq!(effective_ms(AGENT_SEND_DEFAULT_SECS), Ok(Some(3_600_000)));
        assert_eq!(effective_ms(THINK_DEFAULT_SECS), Ok(Some(3_600_000)));
    }

    #[test]
    fn invocation_transport_guard_uses_default_guard_for_zero() {
        assert_eq!(
            invocation_transport_guard(0),
            Ok(Duration::from_secs(INVOKE_DEFAULT_SECS))
        );
    }

    #[test]
    fn runtime_request_timeout_preserves_zero_as_runtime_default() {
        assert_eq!(runtime_request_timeout_ms(0), Ok(None));
    }

    #[test]
    fn absurdly_large_seconds_is_rejected_not_wrapped() {
        // u64::MAX seconds * 1000 would overflow. Surface it as an error
        // rather than silently wrapping or panicking in debug.
        assert!(effective_ms(u64::MAX).is_err());
    }
}
