// EAL interpreter — assembly and entry points. The execution
// engine entries (execute_with_endpoint / execute_with_dispatcher),
// RunContext and shared helpers live here; trace/dispatch/phases/
// retry are sibling planes (T4.4 / F-021 split, move-only).

// EasyNet CLI — EAL Interpreter
// =============================
//
// File: src/eal/interpreter.rs
// Description: Client-side execution engine for Mission IR v2 (temporary — target: MissionControl v2).
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
use std::time::{Duration, Instant};

use serde_json::Value;

/// Convert `Duration::as_millis()` (u128) to u64, saturating at u64::MAX.
mod dispatch;
mod phases;
mod retry;
#[cfg(test)]
mod tests;
mod trace;

#[cfg(test)]
use dispatch::dispatch_to_agent;
pub use dispatch::AgentAwareDispatcher;
use phases::{
    calls_from_partition, execute_calls_phase_partition, execute_loop, split_phase_steps,
    PhasePartition, PhaseRunState,
};
use retry::now_unix_ms;
use trace::{
    CappedTraceBuffer, CapturedResult, EmissionRecord, EXECUTION_TRACE_SCHEMA_VERSION,
    TRACE_CAP_HEAD, TRACE_CAP_TAIL,
};
#[allow(unused_imports)] // public trace model re-export; external callers use this path.
pub use trace::{
    ExecutionReport, ExecutionTrace, MissionOutcome, RetryRecord, StepOutcome, StepTrace,
};

#[inline]
fn millis_u64(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}
use crate::core::agent::id::AbilityName;
use crate::eal::diagnostics::EalError;
use crate::eal::runtime::ir::IrTarget;
use crate::eal::runtime::ir::{IrCall, IrEmit, IrEmitValue, MissionIr};

/// Interpreter-local alias. The per-step execution machinery —
/// retries, dispatch, resolve_arguments, process_step_result — works
/// on flat `IrCall`s. Block variants of `RealIrStep` are expanded by
/// `execute_with_dispatcher` into batches of `IrCall`s (or iterated
/// sequentially in the `IrStep::Loop` case) before reaching any
/// per-step helper. The alias keeps those helpers signature-
/// compatible with the pre-PR-10 code without a churn-only rename.
type IrStep = IrCall;

// ── PR-10 Stage 3: per-variant IrStep dispatch ───────────────────────────
//
// The mission executor consumes a mixed `Vec<RealIrStep>`
// (`Call | Loop`). Chat and Handoff block forms were proposed in
// the Draft revision of the RFC but removed by the approved RFC
// (§10); the enum itself no longer carries those variants.
//
// `split_phase_steps` partitions a phase's steps into Call runs and
// Block singletons, preserving source order. The top-level phase
// walk dispatches each partition: Call runs go to the existing
// parallel/sequential path; a Loop singleton goes to `execute_loop`.
// The planner already collapses phases to one-step-per-phase when
// any top-level item is a Block, so in practice a phase containing
// a Loop will never mix Calls and Loops — but the partitioning is
// correct either way, so a future RFC relaxation (loops packed
// into parallel phases) does not silently break this layer.

#[derive(Debug, Clone, Copy)]
pub struct RunContext<'a> {
    pub tenant: &'a str,
    pub trace_id: &'a str,
}

pub trait StepDispatcher {
    /// Dispatch one step. The runtime sees only the resolved
    /// `IrTarget` enum and the typed `AbilityName` — there is no
    /// string-based `is_agent` check here, by design (see
    /// `docs/AGENT_IDENTITY.md` invariant 2).
    ///
    /// Errors are typed via `EalError` so the interpreter (and any
    /// future telemetry) can branch on category rather than parsing
    /// English strings. The error is converted to its display form
    /// when stored in `StepExecResult::Error.message`.
    /// `causal_parents` carries the producing steps' receipt anchors
    /// (`{node, invocation_ura, receipt_ura, receipt_hash}` objects)
    /// for this step's `input_refs`. Dispatchers that lower onto the
    /// Axon Invocation surface encode them as the envelope's
    /// `causal_context`; transports without an invocation surface
    /// ignore them.
    fn dispatch(
        &self,
        run: RunContext<'_>,
        target: &IrTarget,
        ability: &AbilityName,
        arguments: &Value,
        timeout_ms: Option<u64>,
        causal_parents: &[Value],
    ) -> Result<StepDispatchOutcome, EalError>;

    /// Create an independent clone for parallel dispatch.
    /// Each thread in a phase needs its own dispatcher.
    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError>;
}

/// Successful dispatch outcome: the step's result value plus, when
/// the step was lowered onto the daemon's Axon Invocation surface,
/// the seven-tuple invocation record (envelope echo + ledger receipt
/// anchors). `invocation: None` means the step executed through a
/// path that emits no Axon invocation (in-process fallback, agent
/// CLI dispatch) — the trace records that honestly rather than
/// fabricating a receipt.
#[derive(Debug)]
pub struct StepDispatchOutcome {
    pub value: Value,
    pub invocation: Option<Value>,
}

impl From<Value> for StepDispatchOutcome {
    fn from(value: Value) -> Self {
        Self {
            value,
            invocation: None,
        }
    }
}

// ── Agent-Aware Dispatcher ──
//
// Matches on `IrTarget` to choose between agent CLI dispatch (via
// `runtime::dispatch::send_to_agent`) and bridge dispatch. There is no
// `is_agent` string check anywhere — the surface form already chose
// the variant at parse time, and the planner baked it into the IR.
// See `docs/AGENT_IDENTITY.md` invariants 1 and 2.

/// Execute a mission with a caller-owned trace id.
///
/// `mission_runs` uses this entry so the persisted run id, the
/// `MissionRunMeta.trace_id`, every child Invocation envelope, and
/// the on-disk `trace.json` all name the same run. That identity is
/// operational metadata, not an eighth Invocation tuple field.
pub fn execute_with_endpoint_for_trace(
    endpoint: &str,
    tenant: &str,
    ir: &MissionIr,
    trace_id: String,
) -> anyhow::Result<ExecutionReport> {
    let dispatcher = AgentAwareDispatcher::new(
        endpoint,
        crate::support::platform::timeouts::BRIDGE_CONNECT_TIMEOUT_MS,
    );
    execute_with_dispatcher_for_trace(&dispatcher, tenant, ir, trace_id)
}

// ── Core execution engine ──

#[allow(clippy::too_many_lines, clippy::unnecessary_wraps)]
#[cfg(test)]
pub fn execute_with_dispatcher(
    dispatcher: &dyn StepDispatcher,
    tenant: &str,
    ir: &MissionIr,
) -> anyhow::Result<ExecutionReport> {
    let trace_id = uuid::Uuid::new_v4().to_string();
    execute_with_dispatcher_for_trace(dispatcher, tenant, ir, trace_id)
}

/// Execute a mission through an injected dispatcher with a caller-owned
/// trace id.
///
/// Tests use this to pin the execution identity contract without
/// depending on `mission_runs` persistence. Production mission runs
/// call the endpoint variant above so the same trace id reaches the
/// daemon-lowered child Invocations.
pub fn execute_with_dispatcher_for_trace(
    dispatcher: &dyn StepDispatcher,
    tenant: &str,
    ir: &MissionIr,
    mission_id: String,
) -> anyhow::Result<ExecutionReport> {
    // One trace id per mission run, equal to the mission id: every
    // lowered invocation envelope carries it, so the daemon ledger can
    // group the run (`easynet invocation trace <mission_id>`).
    let run = RunContext {
        tenant,
        trace_id: &mission_id,
    };
    let mission_start = Instant::now();
    let started_at = now_unix_ms();

    let mut captured: HashMap<String, CapturedResult> = HashMap::new();
    // Run-level receipt graph: every completed step's invocation
    // record in execution order, loop-internal steps included. This
    // is what `__runner_receipt_graph__` substitutes to — the graph
    // is a run-level fact, independent of binding scopes.
    let mut receipt_graph: Vec<Value> = Vec::new();
    // `skipped_bindings` mirrors `captured`: when a step with an
    // `output_binding` is skipped (either directly or by dependency),
    // its binding name lands here instead of in `captured`. Downstream
    // `resolve_arguments` consults both so it can tell "skip me because
    // my producer was skipped" apart from "unresolved ref" (which is an
    // analyzer/planner bug). See `ResolveError::UpstreamSkipped`.
    let mut skipped_bindings: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut all_traces = CappedTraceBuffer::new();
    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let mut aborted = false;

    // `total` counts only top-level Call steps — and, for Loop
    // blocks, their max-case call budget (`max_iters * |body+verify
    // calls|`). The `[i/total]` progress display shows actual
    // dispatched calls as they happen, so Loop iterations push the
    // `i` value up while `total` is the worst case; on early
    // termination `i` lands below `total`, which is the correct
    // semantics for "upper bound, not ceiling".
    let total: usize = ir
        .steps
        .iter()
        .map(|s| usize::try_from(s.static_call_bound()).unwrap_or(usize::MAX))
        .sum();
    let mut global_step = 0usize;

    for (phase_idx, phase) in ir.phases.iter().enumerate() {
        if aborted {
            break;
        }
        let phase_steps = &ir.steps[phase.start..phase.end];
        if phase_steps.is_empty() {
            continue;
        }

        for partition in split_phase_steps(phase_steps) {
            if aborted {
                break;
            }
            match partition {
                PhasePartition::Calls(call_steps) => {
                    let calls = calls_from_partition(call_steps);
                    let mut phase_state = PhaseRunState {
                        global_step: &mut global_step,
                        total,
                        captured: &mut captured,
                        receipt_graph: &mut receipt_graph,
                        skipped_bindings: &mut skipped_bindings,
                        completed: &mut completed,
                        failed: &mut failed,
                        skipped: &mut skipped,
                        aborted: &mut aborted,
                        all_traces: &mut all_traces,
                    };
                    execute_calls_phase_partition(
                        dispatcher,
                        run,
                        &calls,
                        phase_idx,
                        &mut phase_state,
                    );
                }
                PhasePartition::Loop(lp) => {
                    let mut phase_state = PhaseRunState {
                        global_step: &mut global_step,
                        total,
                        captured: &mut captured,
                        receipt_graph: &mut receipt_graph,
                        skipped_bindings: &mut skipped_bindings,
                        completed: &mut completed,
                        failed: &mut failed,
                        skipped: &mut skipped,
                        aborted: &mut aborted,
                        all_traces: &mut all_traces,
                    };
                    execute_loop(dispatcher, run, lp, phase_idx, &mut phase_state);
                }
            }
        }
    }

    let total_elapsed = millis_u64(mission_start.elapsed());
    let completed_at = now_unix_ms();
    let outcome = if aborted {
        MissionOutcome::Aborted
    } else if failed > 0 {
        MissionOutcome::Partial
    } else {
        MissionOutcome::Completed
    };

    let (step_traces, traces_truncated) = all_traces.into_parts();
    if traces_truncated > 0 {
        // One diagnostic line on stderr so operators notice a truncated
        // trace without having to parse the JSON. Preserves the normal
        // exit status — truncation is graceful degradation, not an
        // error.
        eprintln!(
            "[easynet warn] mission trace truncated: {traces_truncated} middle step(s) \
             omitted to bound memory (head={TRACE_CAP_HEAD}, tail={TRACE_CAP_TAIL} \
             retained; see ExecutionTrace::traces_truncated)"
        );
    }
    let ability_graph: Vec<Value> = step_traces
        .iter()
        .filter_map(|step_trace| {
            step_trace.invocation.as_ref().map(|meta| {
                let mut entry = meta.clone();
                if let Some(object) = entry.as_object_mut() {
                    object.insert("step_id".into(), Value::String(step_trace.step_id.clone()));
                }
                entry
            })
        })
        .collect();
    let emissions = resolve_emissions(&ir.emits, &captured);
    let trace = ExecutionTrace {
        schema_version: EXECUTION_TRACE_SCHEMA_VERSION,
        mission_id,
        mission_name: ir.name.clone(),
        started_at_unix_ms: started_at,
        completed_at_unix_ms: completed_at,
        total_elapsed_ms: total_elapsed,
        phase_count: ir.phases.len(),
        steps_completed: completed,
        steps_failed: failed,
        steps_skipped: skipped,
        outcome,
        step_traces,
        ability_graph,
        emissions: emissions.clone(),
        traces_truncated,
    };

    // Convert captured results to readable strings for the report.
    let outputs: HashMap<String, String> = captured
        .into_iter()
        .map(|(k, v)| (k, String::from_utf8_lossy(&v.value).to_string()))
        .collect();

    Ok(ExecutionReport {
        total_elapsed_ms: total_elapsed,
        steps_completed: completed,
        steps_failed: failed,
        trace,
        outputs,
    })
}

// ── Internals ──

fn resolve_emissions(
    emits: &[IrEmit],
    captured: &HashMap<String, CapturedResult>,
) -> Vec<EmissionRecord> {
    emits
        .iter()
        .enumerate()
        .map(|(idx, emit)| {
            let seq = idx + 1;
            match &emit.value {
                IrEmitValue::Literal { value } => EmissionRecord {
                    seq,
                    name: emit.name.clone(),
                    kind: emit.kind.clone(),
                    value: value.clone(),
                    source_binding: None,
                    error: None,
                },
                IrEmitValue::Binding { binding } => match captured.get(binding) {
                    Some(result) => EmissionRecord {
                        seq,
                        name: emit.name.clone(),
                        kind: emit.kind.clone(),
                        value: decode_emitted_value(&result.value),
                        source_binding: Some(binding.clone()),
                        error: None,
                    },
                    None => EmissionRecord {
                        seq,
                        name: emit.name.clone(),
                        kind: emit.kind.clone(),
                        value: Value::Null,
                        source_binding: Some(binding.clone()),
                        error: Some(format!(
                            "binding '{binding}' was not captured; producer may have failed or skipped"
                        )),
                    },
                },
            }
        })
        .collect()
}

fn decode_emitted_value(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).to_string()))
}

enum StepExecResult {
    Ok {
        result_bytes: Vec<u8>,
        result_sha256: String,
        elapsed_ms: u64,
        started_at: u64,
        completed_at: u64,
        retry_count: u32,
        retry_history: Vec<RetryRecord>,
        /// Seven-tuple invocation record from the daemon lowering
        /// path; None when the step ran through a receipt-less path.
        invocation: Option<Value>,
    },
    Error {
        message: String,
        elapsed_ms: u64,
        started_at: u64,
        retry_count: u32,
        retry_history: Vec<RetryRecord>,
    },
    /// Upstream binding was skipped, so this step cannot run. Emitted by
    /// `dispatch_batch` when `resolve_arguments` signals
    /// `ResolveError::UpstreamSkipped`. Classified as `StepOutcome::Skipped`
    /// in `process_step_result` regardless of this step's own
    /// `optional` / `on_failure` flags — propagating skip is the point.
    SkippedByDependency { message: String, started_at: u64 },
}
