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
//     - [`INVOKE_DEFAULT_SECS`]      — short network-bound tool calls.
//                                      60 s is generous for any
//                                      well-behaved ability; a longer
//                                      budget belongs in the ability's
//                                      own execution window, not at
//                                      the CLI surface.
//     - [`AGENT_SEND_DEFAULT_SECS`]  — LLM-backed dispatches (Claude
//                                      Code / Codex). These can stream
//                                      for many minutes on a large
//                                      prompt + heavy thinking budget,
//                                      so the floor is 15 min and
//                                      users can raise it explicitly.
//     - [`THINK_DEFAULT_SECS`]       — per-cycle budget inside the
//                                      autonomous loop. Each cycle is
//                                      one think + one action; 120 s
//                                      is the legacy value kept for
//                                      back-compat.
//
// The number `0` is reserved across the CLI to mean "inherit the
// runtime default" — never hard-coded, but often plumbed through the
// bridge layer for per-operation budgets. [`effective_ms`] converts a
// user-facing seconds value to an `Option<u64>` in milliseconds so
// bridge callers can feed it straight into
// `call_mcp_tool_with_timeout`.
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

/// Convert a user-facing seconds value (from a `--timeout <N>` flag) to
/// the `Option<Duration>`-in-milliseconds shape used by the bridge API.
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
    fn absurdly_large_seconds_is_rejected_not_wrapped() {
        // u64::MAX seconds * 1000 would overflow. Surface it as an error
        // rather than silently wrapping or panicking in debug.
        assert!(effective_ms(u64::MAX).is_err());
    }
}
