// Step retry, output verification, argument resolution and
// result processing (split from interpreter.rs, T4.4 / F-021;
// bodies are move-only).

// EasyNet CLI — EAL Interpreter
// =============================
//
// File: src/eal/interpreter.rs
// Description: Daemon-owned execution engine for Mission IR v2.
//
// Execution Model:
//   Phases execute sequentially (data-flow barriers between them).
//   Steps within a phase execute in parallel via rayon work-stealing threadpool.
//   When a dispatcher cannot be cloned across worker threads, falls back to sequential.
//
// Core Capabilities:
//   1. True parallel dispatch — rayon::scope + clone_for_thread() per step.
//   2. Structured ExecutionTrace — per-step audit log with timestamps, result hashes, retry history.
//   3. Retry with exponential backoff — delay = min(base * 2^attempt, max) + deterministic jitter.
//   4. Cross-phase data flow — results captured in HashMap, substituted into downstream input_refs.
//   5. Lock-free result collection — crossbeam SegQueue eliminates collector contention.
//   6. Connection pool reuse — BridgePool with adaptive sizing based on CPU cores.
//
// Dispatch Abstraction:
//   `trait StepDispatcher` decouples execution from transport. Production uses
//   AgentAwareDispatcher; tests inject MockDispatcher or a non-cloneable dispatcher.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use console::style;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::trace::{CapturedResult, RetryRecord, StepOutcome, StepTrace};
use super::{millis_u64, IrStep, RunContext, StepDispatchOutcome, StepDispatcher, StepExecResult};
use crate::eal::diagnostics::EalError;
use crate::eal::runtime::ir::IrFailurePolicy;
use crate::support::platform::output;

pub(super) const RETRY_BASE_MS: u64 = 1000;
pub(super) const RETRY_MAX_MS: u64 = 30_000;

pub(super) enum VerifyDone {
    True,
    False,
    Malformed(String),
}

/// Peel the daemon shell-executor envelope (`{result, fulfilled_by,
/// ...}`, with `result` carrying the handler's stdout as a JSON
/// string) so `done` checks and downstream consumers see the
/// handler's own payload. Non-envelope values pass through untouched.
fn peel_shell_envelope(v: serde_json::Value) -> serde_json::Value {
    let Some(obj) = v.as_object() else { return v };
    if !(obj.contains_key("result") && obj.contains_key("fulfilled_by")) {
        return v;
    }
    match &obj["result"] {
        serde_json::Value::String(inner) => {
            serde_json::from_str(inner).unwrap_or_else(|_| serde_json::Value::String(inner.clone()))
        }
        inner => inner.clone(),
    }
}

pub(super) fn verify_output_done(bytes: &[u8]) -> VerifyDone {
    let v: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => {
            return VerifyDone::Malformed(format!("verify output is not JSON-decodable ({e})"));
        }
    };
    let v = peel_shell_envelope(v);
    let obj = match v.as_object() {
        Some(o) => o,
        None => {
            return VerifyDone::Malformed(format!(
                "verify output must be a JSON object with a top-level `done: bool` field, got {v}"
            ));
        }
    };
    match obj.get("done") {
        Some(serde_json::Value::Bool(true)) => VerifyDone::True,
        Some(serde_json::Value::Bool(false)) => VerifyDone::False,
        Some(other) => {
            VerifyDone::Malformed(format!("verify output `done` must be boolean, got {other}"))
        }
        None => VerifyDone::Malformed(
            "verify output has no top-level `done` field (RFC §4.4)".to_string(),
        ),
    }
}

pub(super) fn execute_step_with_retry(
    dispatcher: &dyn StepDispatcher,
    run: RunContext<'_>,
    step: &IrStep,
    arguments: &Value,
    dependency_receipts: &[
        crate::daemon::execution::child_invocation::ChildInvocationReceiptAnchor
    ],
) -> StepExecResult {
    // Mission runtime semantics: `max_retries` is the number of retries AFTER the
    // first attempt, so total attempts = 1 + max_retries.
    #[allow(clippy::cast_sign_loss)] // max_retries is checked > 0 above
    let max_attempts = if matches!(step.on_failure, IrFailurePolicy::Retry) && step.max_retries > 0
    {
        1 + step.max_retries as u32
    } else {
        1
    };

    let mut retry_history = Vec::new();
    let started_at = now_unix_ms();
    let t0 = Instant::now();

    for attempt in 0..max_attempts {
        if attempt > 0 {
            // Exponential backoff with deterministic jitter.
            // Use rayon::yield_now() first to let other tasks run on this thread,
            // then sleep in small increments so rayon can still steal work between yields.
            let backoff = compute_backoff(attempt, &step.step_id);
            backoff_sleep(backoff);
        }

        let attempt_start = Instant::now();
        #[allow(clippy::cast_sign_loss)] // timeout_seconds is checked > 0 above
        let step_timeout_ms = if step.timeout_seconds > 0 {
            Some(step.timeout_seconds as u64 * 1000)
        } else {
            None
        };
        let dispatch_timeout_ms = match effective_dispatch_timeout_ms(step_timeout_ms, run.deadline)
        {
            Ok(timeout) => timeout,
            Err(error) => {
                let rendered = error.to_string();
                let elapsed_ms = millis_u64(t0.elapsed());
                return StepExecResult::Error {
                    message: rendered,
                    elapsed_ms,
                    started_at,
                    retry_count: attempt,
                    retry_history,
                };
            }
        };
        let res = dispatcher.dispatch(
            run,
            &step.target,
            &step.ability,
            arguments,
            dispatch_timeout_ms,
            dependency_receipts,
        );

        match res {
            Ok(outcome) => {
                let StepDispatchOutcome {
                    value: result,
                    invocation,
                } = outcome;
                // Serializing a `serde_json::Value` back to bytes can
                // only fail if the Value contains NaN / ±∞ numbers —
                // JSON has no representation for those. A dispatcher
                // that returns such a value is buggy (the producer
                // should map NaN to `null` or a sentinel), so we
                // surface it as a step-level `Internal` error rather
                // than silently capturing an empty `[]` and feeding
                // that to any downstream step that consumed the
                // binding. Either mode would be wrong; loud failure
                // gives operators a real error to chase.
                let result_bytes = match serde_json::to_vec(&result) {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        let elapsed_ms = millis_u64(t0.elapsed());
                        let rendered =
                            EalError::Internal(format!("step result not JSON-serializable: {e}"))
                                .to_string();
                        return StepExecResult::Error {
                            message: rendered,
                            elapsed_ms,
                            started_at,
                            retry_count: attempt,
                            retry_history,
                        };
                    }
                };
                let result_sha256 = sha256_hex(&result_bytes);
                let completed_at = now_unix_ms();
                let elapsed_ms = millis_u64(t0.elapsed());
                return StepExecResult::Ok {
                    result_bytes,
                    result_sha256,
                    elapsed_ms,
                    started_at,
                    completed_at,
                    retry_count: attempt,
                    retry_history,
                    invocation,
                };
            }
            Err(e) => {
                // Convert the typed `EalError` to its display form
                // (`error_code: message`) at the boundary into the
                // trace and retry history. The trace shape is owned
                // by `StepExecResult` / `RetryRecord` (both String
                // fields), so the typing migration ends here — but
                // because `Display` prefixes the error_code, the
                // category survives into on-disk traces and retry
                // logs without changing the schema.
                let rendered = e.to_string();
                let attempt_elapsed = millis_u64(attempt_start.elapsed());
                let backoff = if attempt + 1 < max_attempts {
                    compute_backoff(attempt + 1, &step.step_id)
                } else {
                    0
                };
                retry_history.push(RetryRecord {
                    attempt: attempt + 1,
                    elapsed_ms: attempt_elapsed,
                    backoff_ms: backoff,
                    error: rendered.clone(),
                });
                // Last attempt: return error with retry info
                if attempt + 1 >= max_attempts {
                    let elapsed_ms = millis_u64(t0.elapsed());
                    return StepExecResult::Error {
                        message: rendered,
                        elapsed_ms,
                        started_at,
                        retry_count: attempt,
                        retry_history,
                    };
                }
            }
        }
    }

    let elapsed_ms = millis_u64(t0.elapsed());
    StepExecResult::Error {
        message: "exhausted all retry attempts".into(),
        elapsed_ms,
        started_at,
        retry_count: max_attempts.saturating_sub(1),
        retry_history,
    }
}

fn effective_dispatch_timeout_ms(
    step_timeout_ms: Option<u64>,
    run_deadline: Option<Instant>,
) -> Result<Option<u64>, EalError> {
    let Some(deadline) = run_deadline else {
        return Ok(step_timeout_ms);
    };
    let now = Instant::now();
    if now >= deadline {
        return Err(EalError::DeadlineExceeded(
            "mission run timeout expired before dispatch".to_string(),
        ));
    }
    let remaining_ms = millis_u64(deadline.duration_since(now)).max(1);
    Ok(match step_timeout_ms {
        Some(step_timeout) => Some(step_timeout.min(remaining_ms)),
        None => Some(remaining_ms),
    })
}

/// Typed outcome of argument resolution. Splits the "upstream was
/// skipped" case out from the generic error bucket so the caller can
/// surface it as `StepOutcome::Skipped` rather than `Failed`.
///
/// This is what fixes the "optional upstream → required downstream"
/// regression: without the distinction, a missing binding became a
/// generic "unresolved ref 'x'" error and the downstream step was
/// classified as `Failed`, even though the truth is "we couldn't run
/// you because your input was skipped upstream". The trace outcome
/// matters: a skipped mission leg should show up as a chain of
/// skipped steps, not as a cascade of spurious failures.
#[derive(Debug)]
pub(super) enum ResolveError {
    /// An input ref's upstream binding was skipped (upstream step
    /// chose not to run). Downstream auto-propagates as Skipped.
    UpstreamSkipped { binding: String, arg: String },
    /// Any other resolution problem: missing binding that wasn't
    /// tracked as skipped (implies the binding is truly undefined,
    /// which is an analyzer-time bug), malformed upstream JSON,
    /// etc. Renders as a normal step error.
    Other(String),
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::UpstreamSkipped { binding, arg } => write!(
                f,
                "input ref `{arg}` cannot be resolved: upstream binding `{binding}` was skipped"
            ),
            ResolveError::Other(s) => f.write_str(s),
        }
    }
}

pub(super) fn resolve_arguments(
    step: &IrStep,
    results: &HashMap<String, CapturedResult>,
    skipped_bindings: &std::collections::HashSet<String>,
) -> Result<Value, ResolveError> {
    let mut args = step
        .static_arguments
        .as_object()
        .cloned()
        .unwrap_or_default();
    for (key, src_binding) in &step.input_refs {
        // If the producer was skipped upstream, surface the typed
        // skip signal so the caller can propagate `Skipped` rather
        // than `Failed`.
        if skipped_bindings.contains(src_binding) {
            return Err(ResolveError::UpstreamSkipped {
                binding: src_binding.clone(),
                arg: key.clone(),
            });
        }
        let captured = results.get(src_binding).ok_or_else(|| {
            ResolveError::Other(format!(
                "unresolved ref `{src_binding}` (neither captured nor skipped — \
                 likely an analyzer/planner bug)"
            ))
        })?;
        // Propagate deserialization failure as a step-level error.
        //
        // The previous behaviour was `unwrap_or(Value::Null)`, which
        // silently fed a `null` to the consuming step when the
        // upstream result was malformed JSON (e.g. an ability that
        // returned a partial stream, a network corruption, or a buggy
        // wrapper). The downstream agent/ability would then treat the
        // null as legitimate input and either crash much later or —
        // worse — produce a plausible-but-wrong answer. A real
        // pipeline of 6+ steps would surface as "step F failed for an
        // inscrutable reason" while the actual culprit was step B's
        // unparseable payload.
        //
        // Surfacing the deser failure here means the corrupted step
        // is the one that fails, and the trace pinpoints which input
        // ref couldn't be parsed — which is exactly the diagnostic
        // an operator needs.
        let val: Value = serde_json::from_slice(&captured.value).map_err(|e| {
            ResolveError::Other(format!(
                "input ref `{key}` from binding `{src_binding}` is not valid JSON: {e}. \
                 Upstream step likely returned a malformed result."
            ))
        })?;
        args.insert(key.clone(), val);
    }
    Ok(Value::Object(args))
}

/// Returns (outcome, trace, `Option<result_bytes>`).
/// `result_bytes` is Some only on success — used for data flow capture.
#[allow(clippy::cast_precision_loss)] // elapsed_ms display — sub-ms precision not needed
pub(super) fn process_step_result(
    step: &IrStep,
    result: StepExecResult,
    global_step: usize,
    total: usize,
    phase_idx: usize,
) -> (
    StepOutcome,
    StepTrace,
    Option<Vec<u8>>,
    Option<crate::daemon::execution::child_invocation::ChildInvocationRecord>,
) {
    match result {
        StepExecResult::Ok {
            result_bytes,
            result_sha256,
            elapsed_ms,
            started_at,
            completed_at,
            retry_count,
            retry_history,
            invocation,
        } => {
            let bind_info = step
                .output_binding
                .as_ref()
                .map(|b| format!("  → ${b}"))
                .unwrap_or_default();
            let dep_info = if step.input_refs.is_empty() {
                String::new()
            } else {
                let refs: Vec<_> = step.input_refs.values().map(|v| format!("${v}")).collect();
                format!("  (← {})", refs.join(", "))
            };
            let retry_info = if retry_count > 0 {
                format!("  ({retry_count} retries)")
            } else {
                String::new()
            };

            output::step(&format!(
                "[{global_step}/{total}] {:<20} {:<14} {} {:.1}s{bind_info}{dep_info}{retry_info}",
                step.ability,
                step.target.display_string(),
                style("✓").green(),
                elapsed_ms as f64 / 1000.0,
            ));

            let size = result_bytes.len();
            let projection = invocation.projection();
            let trace = StepTrace {
                step_id: step.step_id.clone(),
                ability: step.ability.clone(),
                target: step.target.clone(),
                phase_index: phase_idx,
                started_at_unix_ms: started_at,
                completed_at_unix_ms: completed_at,
                elapsed_ms,
                outcome: StepOutcome::Completed,
                retry_count,
                retry_history,
                result_size_bytes: Some(size),
                result_sha256: Some(result_sha256),
                error: None,
                invocation: Some(projection),
                input_refs: step.input_refs.clone(),
                output_binding: step.output_binding.clone(),
            };

            (
                StepOutcome::Completed,
                trace,
                Some(result_bytes),
                Some(invocation),
            )
        }
        StepExecResult::Error {
            message,
            elapsed_ms,
            started_at,
            retry_count,
            retry_history,
        } => {
            let completed_at = now_unix_ms();
            let attempts = retry_history.len();
            let retry_info = if attempts > 1 {
                format!("  ({attempts} attempts)")
            } else {
                String::new()
            };
            output::step(&format!(
                "[{global_step}/{total}] {:<20} {:<14} {} {:.1}s{retry_info}  {message}",
                step.ability,
                step.target.display_string(),
                style("✗").red(),
                elapsed_ms as f64 / 1000.0,
            ));

            let outcome = if step.optional || matches!(step.on_failure, IrFailurePolicy::Skip) {
                StepOutcome::Skipped
            } else {
                StepOutcome::Failed
            };

            let trace = StepTrace {
                step_id: step.step_id.clone(),
                ability: step.ability.clone(),
                target: step.target.clone(),
                phase_index: phase_idx,
                started_at_unix_ms: started_at,
                completed_at_unix_ms: completed_at,
                elapsed_ms,
                outcome,
                retry_count,
                retry_history,
                result_size_bytes: None,
                result_sha256: None,
                error: Some(message),
                invocation: None,
                input_refs: step.input_refs.clone(),
                output_binding: step.output_binding.clone(),
            };

            (outcome, trace, None, None)
        }
        StepExecResult::SkippedByDependency {
            message,
            started_at,
        } => {
            // Print a distinct glyph for cascaded skips so operators
            // can tell "I chose not to run you" (— dim) from "your
            // input producer didn't run" (⟿ yellow). The trace
            // outcome is `Skipped` in both cases, but the message
            // carries the provenance.
            output::step(&format!(
                "[{global_step}/{total}] {:<20} {:<14} {} (dep skipped)  {message}",
                step.ability,
                step.target.display_string(),
                style("⟿").yellow(),
            ));
            let completed_at = now_unix_ms();
            let trace = StepTrace {
                step_id: step.step_id.clone(),
                ability: step.ability.clone(),
                target: step.target.clone(),
                phase_index: phase_idx,
                started_at_unix_ms: started_at,
                completed_at_unix_ms: completed_at,
                elapsed_ms: 0,
                outcome: StepOutcome::Skipped,
                retry_count: 0,
                retry_history: Vec::new(),
                result_size_bytes: None,
                result_sha256: None,
                error: Some(message),
                invocation: None,
                input_refs: step.input_refs.clone(),
                output_binding: step.output_binding.clone(),
            };
            (StepOutcome::Skipped, trace, None, None)
        }
    }
}

/// Cooperative backoff: yields to rayon's work-stealing scheduler between
/// short sleep intervals. This prevents a retrying step from monopolizing
/// a rayon worker thread for the entire backoff duration.
///
/// - For delays ≤ 50ms: single yield + sleep (not worth splitting).
/// - For longer delays: sleep in 50ms chunks with yields between them.
fn backoff_sleep(total_ms: u64) {
    const CHUNK_MS: u64 = 50;
    if total_ms <= CHUNK_MS {
        rayon::yield_now();
        std::thread::sleep(Duration::from_millis(total_ms));
        return;
    }
    let mut remaining = total_ms;
    while remaining > 0 {
        rayon::yield_now();
        let chunk = remaining.min(CHUNK_MS);
        std::thread::sleep(Duration::from_millis(chunk));
        remaining = remaining.saturating_sub(chunk);
    }
}

pub(super) fn compute_backoff(attempt: u32, step_id: &str) -> u64 {
    let base = RETRY_BASE_MS * 2u64.pow(attempt.saturating_sub(1));
    let capped = base.min(RETRY_MAX_MS);
    // Deterministic jitter based on step_id + attempt.
    let mut hasher = Sha256::new();
    hasher.update(step_id.as_bytes());
    hasher.update(attempt.to_le_bytes());
    let hash = hasher.finalize();
    // SHA-256 always returns 32 bytes, so `hash[..8]` is always exactly 8
    // bytes and the conversion to `[u8; 8]` is infallible. The `expect`
    // (rather than `unwrap`) documents the invariant for the next reader
    // and would surface a clear cause if the digest algorithm were ever
    // swapped for one with a smaller output.
    let jitter_seed = u64::from_le_bytes(
        hash[..8]
            .try_into()
            .expect("SHA-256 produces 32 bytes; first-8-byte slice is always [u8; 8]"),
    );
    let jitter = jitter_seed % (RETRY_BASE_MS / 2 + 1);
    capped + jitter
}

fn sha256_hex(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    hex_encode(&hash)
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

pub(super) fn now_unix_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

// ── Tests ──
