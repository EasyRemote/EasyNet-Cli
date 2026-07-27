// Phase partitioning and batch/loop execution (split from
// interpreter.rs, T4.4 / F-021; bodies are move-only).
// Guard rail: mission is a script — steps spawn child
// invocations, retry = re-invoke, no step-level resume (§0.1-5).

// EasyNet CLI — EAL Interpreter
// =============================
//
// File: src/eal/interpreter.rs
// Description: Daemon-owned execution engine for Mission IR v2.
//
// Execution Model:
//   Phases execute sequentially (data-flow barriers between them).
//   Steps within a phase execute under the dispatcher's declared concurrency policy.
//
// Core Capabilities:
//   1. Declared parallel dispatch — rayon::scope + clone_for_thread() per step.
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

use console::style;
use crossbeam_queue::SegQueue;
use serde_json::Value;

use super::retry::{
    execute_step_with_retry, now_unix_ms, process_step_result, resolve_arguments,
    verify_output_done, ResolveError, VerifyDone,
};
use super::trace::{CappedTraceBuffer, CapturedResult};
use super::*;
use crate::eal::runtime::ir::{IrCall, IrFailurePolicy, IrLoop, IrStep as RealIrStep};

pub(super) struct PhaseRunState<'a> {
    pub global_step: &'a mut usize,
    pub total: usize,
    pub captured: &'a mut HashMap<String, CapturedResult>,
    pub receipt_graph: &'a mut Vec<Value>,
    pub skipped_bindings: &'a mut std::collections::HashSet<String>,
    pub completed: &'a mut usize,
    pub failed: &'a mut usize,
    pub skipped: &'a mut usize,
    pub aborted: &'a mut bool,
    pub all_traces: &'a mut CappedTraceBuffer,
}

struct BatchDispatchRequest<'a> {
    dispatcher: &'a dyn StepDispatcher,
    run: RunContext<'a>,
    steps: &'a [IrCall],
    indices: &'a [usize],
    captured: &'a HashMap<String, CapturedResult>,
    receipt_graph: &'a [Value],
    skipped_bindings: &'a std::collections::HashSet<String>,
    parallel: bool,
}

struct BatchProcessRequest {
    phase_idx: usize,
    results: Vec<(usize, StepExecResult)>,
}

struct LoopBlockRequest<'a> {
    dispatcher: &'a dyn StepDispatcher,
    run: RunContext<'a>,
    steps: &'a [IrCall],
    phase_idx: usize,
}

#[derive(Debug)]
pub(super) enum PhasePartition<'a> {
    /// Contiguous run of `IrStep::Call` — dispatched via the
    /// existing parallel path when permitted by the dispatcher.
    Calls(&'a [RealIrStep]),
    /// A single Loop block — executed sequentially in-process via
    /// `execute_loop`.
    Loop(&'a IrLoop),
}

pub(super) fn split_phase_steps(steps: &[RealIrStep]) -> Vec<PhasePartition<'_>> {
    let mut out: Vec<PhasePartition<'_>> = Vec::new();
    let mut run_start: Option<usize> = None;
    for (i, step) in steps.iter().enumerate() {
        match step {
            RealIrStep::Call(_) => {
                if run_start.is_none() {
                    run_start = Some(i);
                }
            }
            RealIrStep::Loop(l) => {
                if let Some(s) = run_start.take() {
                    out.push(PhasePartition::Calls(&steps[s..i]));
                }
                out.push(PhasePartition::Loop(l));
            }
        }
    }
    if let Some(s) = run_start {
        out.push(PhasePartition::Calls(&steps[s..]));
    }
    out
}

/// Extract `IrCall`s from a run of `IrStep::Call` steps. The planner
/// guarantees every element of a `PhasePartition::Calls` slice is
/// `IrStep::Call`; `unreachable!` is unreachable on a well-formed IR.
pub(super) fn calls_from_partition(steps: &[RealIrStep]) -> Vec<IrCall> {
    steps
        .iter()
        .map(|s| match s {
            RealIrStep::Call(c) => c.clone(),
            RealIrStep::Loop(_) => {
                unreachable!("calls_from_partition invoked with non-Call step — planner bug")
            }
        })
        .collect()
}

// ── Retry constants ──

/// Sentinel an EAL author writes as an argument value to receive the
/// runner's receipt graph — the seven-tuple invocation records of all
/// bound steps completed so far. Substituted at argument-resolution
/// time (the runner owns receipt refs; `.receipt` is deliberately not
/// an EAL user value). Steps that don't ask don't pay.
const RECEIPT_GRAPH_SENTINEL: &str = "__runner_receipt_graph__";

/// Collect verified dependency receipt anchors for one step from the
/// producers named in its `input_refs`.
fn dependency_receipts_from_captured(
    step: &IrCall,
    captured: &HashMap<String, CapturedResult>,
) -> Vec<crate::daemon::execution::child_invocation::ChildInvocationReceiptAnchor> {
    let mut seen = std::collections::HashSet::new();
    let mut parents = Vec::new();
    for binding in step.input_refs.values() {
        if !seen.insert(binding.clone()) {
            continue;
        }
        let Some(produced) = captured.get(binding) else {
            continue;
        };
        parents.push(produced.invocation.terminal_receipt().clone());
    }
    parents
}

/// Replace any top-level [`RECEIPT_GRAPH_SENTINEL`] argument value
/// with the run-level receipt graph: every completed step's invocation
/// record so far, in execution order — including loop-internal steps,
/// whose bindings never reach the outer captured map. The graph is a
/// run-level fact, not a binding-scope projection.
fn substitute_receipt_graph(args: &mut Value, receipt_graph: &[Value]) {
    let Some(object) = args.as_object_mut() else {
        return;
    };
    for value in object.values_mut() {
        if value.as_str() == Some(RECEIPT_GRAPH_SENTINEL) {
            *value = Value::Array(receipt_graph.to_vec());
        }
    }
}

/// Dispatch a batch of steps (identified by `indices` into `steps`) in parallel or sequentially.
/// Returns `Vec<(local_idx, StepExecResult)>` sorted by `local_idx`.
///
/// Parallel path uses rayon's work-stealing threadpool (amortizes thread creation
/// across phases) and crossbeam's lock-free SegQueue for result collection.
fn dispatch_batch(request: BatchDispatchRequest<'_>) -> Vec<(usize, StepExecResult)> {
    let BatchDispatchRequest {
        dispatcher,
        run,
        steps,
        indices,
        captured,
        receipt_graph,
        skipped_bindings,
        parallel,
    } = request;

    if indices.is_empty() {
        return Vec::new();
    }
    if parallel && indices.len() > 1 {
        // Pre-resolve arguments and pre-clone dispatchers on the main thread,
        // so the rayon closure only captures Send types (no &dyn StepDispatcher).
        type PreparedTask = (
            usize,
            Box<dyn StepDispatcher + Send>,
            Value,
            Vec<crate::daemon::execution::child_invocation::ChildInvocationReceiptAnchor>,
        );
        let mut tasks: Vec<PreparedTask> = Vec::new();
        // Lock-free result queue — each rayon task pushes without contention.
        let collector = SegQueue::new();
        for &local_idx in indices {
            let step = &steps[local_idx];
            let merged_args = match resolve_arguments(step, captured, skipped_bindings) {
                Ok(args) => args,
                Err(ResolveError::UpstreamSkipped { binding, arg }) => {
                    // Propagate skip: the upstream producer chose not to
                    // run, so this consumer must not run either.
                    // `SkippedByDependency` surfaces as `StepOutcome::Skipped`
                    // in `process_step_result` regardless of this step's
                    // own `optional` / `on_failure` policy.
                    collector.push((
                        local_idx,
                        StepExecResult::SkippedByDependency {
                            message: format!(
                                "skipped: input `{arg}` depends on `{binding}` which was skipped upstream"
                            ),
                            started_at: now_unix_ms(),
                        },
                    ));
                    continue;
                }
                Err(ResolveError::Other(e)) => {
                    collector.push((
                        local_idx,
                        StepExecResult::Error {
                            message: e,
                            elapsed_ms: 0,
                            started_at: now_unix_ms(),
                            retry_count: 0,
                            retry_history: Vec::new(),
                        },
                    ));
                    continue;
                }
            };
            let thread_dispatcher = match dispatcher.clone_for_thread() {
                Ok(d) => d,
                Err(e) => {
                    // Reaching this branch means a structural setup error: a
                    // dispatcher declared `Parallel` but could not produce a
                    // worker-local dispatcher. Render to display form
                    // (preserves error_code in the trace) and surface it as
                    // a step error.
                    collector.push((
                        local_idx,
                        StepExecResult::Error {
                            message: e.to_string(),
                            elapsed_ms: 0,
                            started_at: now_unix_ms(),
                            retry_count: 0,
                            retry_history: Vec::new(),
                        },
                    ));
                    continue;
                }
            };
            let mut merged_args = merged_args;
            substitute_receipt_graph(&mut merged_args, receipt_graph);
            let dependency_receipts = dependency_receipts_from_captured(step, captured);
            tasks.push((
                local_idx,
                thread_dispatcher,
                merged_args,
                dependency_receipts,
            ));
        }
        // Mission context handoff (F-028 / T5.4): rayon workers carry
        // their own thread-locals, so the orchestrating thread's
        // DispatchContext is captured ONCE here and re-installed
        // inside each worker for the duration of its step. This is
        // the in-process channel — the process-global env-var bridge
        // it replaces let concurrent missions stomp each other's id.
        let parent_ctx = crate::daemon::execution::mission::context::current();
        // Spawn rayon tasks — closure captures only Send types.
        rayon::scope(|scope| {
            for (local_idx, thread_dispatcher, merged_args, dependency_receipts) in tasks {
                let step = &steps[local_idx];
                let collector_ref = &collector;
                let parent_ctx = parent_ctx.clone();
                scope.spawn(move |_| {
                    let _mission_ctx =
                        parent_ctx.map(crate::daemon::execution::mission::context::enter);
                    let result = execute_step_with_retry(
                        thread_dispatcher.as_ref(),
                        run,
                        step,
                        &merged_args,
                        &dependency_receipts,
                    );
                    collector_ref.push((local_idx, result));
                });
            }
        });
        // Drain the lock-free queue into a sorted Vec.
        let mut results: Vec<_> = std::iter::from_fn(|| collector.pop()).collect();
        results.sort_by_key(|(idx, _)| *idx);
        results
    } else {
        let mut results = Vec::new();
        for &local_idx in indices {
            let step = &steps[local_idx];
            let merged_args = match resolve_arguments(step, captured, skipped_bindings) {
                Ok(args) => args,
                Err(ResolveError::UpstreamSkipped { binding, arg }) => {
                    // Mirror of the parallel branch above — see that
                    // site for the propagation rationale.
                    results.push((
                        local_idx,
                        StepExecResult::SkippedByDependency {
                            message: format!(
                                "skipped: input `{arg}` depends on `{binding}` which was skipped upstream"
                            ),
                            started_at: now_unix_ms(),
                        },
                    ));
                    continue;
                }
                Err(ResolveError::Other(e)) => {
                    results.push((
                        local_idx,
                        StepExecResult::Error {
                            message: e,
                            elapsed_ms: 0,
                            started_at: now_unix_ms(),
                            retry_count: 0,
                            retry_history: Vec::new(),
                        },
                    ));
                    continue;
                }
            };
            let mut merged_args = merged_args;
            substitute_receipt_graph(&mut merged_args, receipt_graph);
            let dependency_receipts = dependency_receipts_from_captured(step, captured);
            let result =
                execute_step_with_retry(dispatcher, run, step, &merged_args, &dependency_receipts);
            results.push((local_idx, result));
        }
        results
    }
}

/// Process a batch of dispatch results: update counters, capture outputs, build traces.
fn process_batch(steps: &[IrCall], request: BatchProcessRequest, state: &mut PhaseRunState<'_>) {
    let BatchProcessRequest { phase_idx, results } = request;

    for (local_idx, exec_result) in results {
        *state.global_step += 1;
        let step = &steps[local_idx];

        let (outcome, trace, result_bytes, invocation) = process_step_result(
            step,
            exec_result,
            *state.global_step,
            state.total,
            phase_idx,
        );

        match outcome {
            StepOutcome::Completed => {
                *state.completed += 1;
                let invocation = invocation
                    .expect("completed EAL step must carry its canonical child Invocation record");
                state.receipt_graph.push(invocation.projection());
                if let Some(ref binding) = step.output_binding {
                    if let Some(bytes) = result_bytes {
                        state.captured.insert(
                            binding.clone(),
                            CapturedResult {
                                value: bytes,
                                invocation,
                            },
                        );
                    }
                }
            }
            StepOutcome::Failed => {
                *state.failed += 1;
                if !step.optional && matches!(step.on_failure, IrFailurePolicy::Abort) {
                    *state.aborted = true;
                }
            }
            StepOutcome::Skipped => {
                *state.skipped += 1;
                // Register the (un-)produced binding so every future
                // `resolve_arguments` call on a step consuming it
                // returns `ResolveError::UpstreamSkipped` and the
                // downstream step is classified Skipped too. Without
                // this registration, the downstream step would hit
                // the `unresolved ref` branch and get classified as
                // Failed — miscategorising "your producer didn't run"
                // as "you ran and failed".
                if let Some(ref binding) = step.output_binding {
                    state.skipped_bindings.insert(binding.clone());
                }
            }
        }
        state.all_traces.push(trace);
    }
}

/// Dispatch a contiguous run of `IrStep::Call` in one phase partition.
/// Extracted verbatim from the pre-PR-10 phase body so the parallel-
/// when-independent scheduling behaviour is unchanged for pure-Call
/// missions.
pub(super) fn execute_calls_phase_partition(
    dispatcher: &dyn StepDispatcher,
    run: RunContext<'_>,
    steps: &[IrCall],
    phase_idx: usize,
    state: &mut PhaseRunState<'_>,
) {
    if steps.is_empty() {
        return;
    }
    let wants_parallel = steps.len() > 1;
    let can_parallel =
        wants_parallel && dispatcher.dispatch_concurrency() == StepDispatchConcurrency::Parallel;
    let phase_label = if can_parallel {
        format!("phase {phase_idx}  parallel")
    } else {
        format!("phase {phase_idx}")
    };
    eprintln!("\n  {}", style(phase_label).cyan());

    // Required first, then optional — same policy as pre-PR-10.
    let required_indices: Vec<usize> = steps
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.optional)
        .map(|(i, _)| i)
        .collect();
    let optional_indices: Vec<usize> = steps
        .iter()
        .enumerate()
        .filter(|(_, s)| s.optional)
        .map(|(i, _)| i)
        .collect();

    let required_results = dispatch_batch(BatchDispatchRequest {
        dispatcher,
        run,
        steps,
        indices: &required_indices,
        captured: &*state.captured,
        receipt_graph: &*state.receipt_graph,
        skipped_bindings: &*state.skipped_bindings,
        parallel: can_parallel,
    });
    process_batch(
        steps,
        BatchProcessRequest {
            phase_idx,
            results: required_results,
        },
        state,
    );

    if !*state.aborted && !optional_indices.is_empty() {
        let optional_results = dispatch_batch(BatchDispatchRequest {
            dispatcher,
            run,
            steps,
            indices: &optional_indices,
            captured: &*state.captured,
            receipt_graph: &*state.receipt_graph,
            skipped_bindings: &*state.skipped_bindings,
            parallel: can_parallel,
        });
        process_batch(
            steps,
            BatchProcessRequest {
                phase_idx,
                results: optional_results,
            },
            state,
        );
    }
}

/// Execute an `IrStep::Loop` block in-process, sequentially, per RFC
/// §3.1 / §4.1–§4.4.
///
/// One iteration = run every `body` step in order (sequential within
/// the iteration — RFC §6 v1: no parallel body), then run every
/// `verify` step. The final `verify` call's output is inspected for a
/// top-level boolean field `done`:
///   - `done == true` → loop terminates successfully; the winning
///     verify output is captured as `<name>.result` if named.
///   - `done == false` → re-enter `body` unless `max_iters` reached.
///   - Missing output, missing `done`, or non-boolean `done` →
///     typed `VerifyMalformed` abort (hard mission abort per §5.2).
///
/// `max_iters` reached without `done: true` → typed `LoopExhausted`
/// abort. Both abort types propagate through the outer `aborted`
/// flag; the rest of the mission does not run.
///
/// Scope: the loop maintains its own per-iteration `captured` map.
/// Inner body/verify bindings live only for the iteration (RFC §3.1
/// hermetic scope). The outer `captured` receives only
/// `<name>.result` on a winning iteration, when the loop is named.
pub(super) fn execute_loop(
    dispatcher: &dyn StepDispatcher,
    run: RunContext<'_>,
    lp: &IrLoop,
    phase_idx: usize,
    state: &mut PhaseRunState<'_>,
) {
    let label = lp.name.as_deref().unwrap_or("<anonymous>");
    eprintln!(
        "\n  {}",
        style(format!(
            "phase {phase_idx}  loop '{label}' max_iters={n}",
            n = lp.max_iters
        ))
        .cyan()
    );

    // Body and verify at this layer are `Vec<IrStep>` (the IR enum).
    // The v1 planner rejects nested loops (RFC §4.2), and by design
    // loops do not contain further block variants, so every leaf is
    // `IrStep::Call`. Extract the flat call slice once per block
    // so the per-iteration executor can reuse them without walking
    // the enum every time.
    let body_calls = flatten_loop_leaves(&lp.body, label, "body");
    let verify_calls = flatten_loop_leaves(&lp.verify, label, "verify");

    let (body_calls, verify_calls) = match (body_calls, verify_calls) {
        (Ok(b), Ok(v)) => (b, v),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("  {}", style(format!("loop '{label}': {e}")).red());
            *state.failed += 1;
            *state.aborted = true;
            return;
        }
    };

    // `verify` must be non-empty (planner enforces) and its last leaf
    // call carries the termination predicate.
    let verify_last_idx = verify_calls.len() - 1;

    for iter in 1..=lp.max_iters {
        if *state.aborted {
            return;
        }

        // Fresh per-iteration scope — outer bindings are not visible
        // (hermetic per RFC §3.1 v1) and inner bindings do not leak
        // across iterations. A future RFC may relax the outbound
        // direction; the inner scope stays fresh regardless.
        let mut iter_captured: HashMap<String, CapturedResult> = HashMap::new();
        let mut iter_skipped: std::collections::HashSet<String> = std::collections::HashSet::new();

        eprintln!(
            "  {}",
            style(format!("  iter {iter}/{max} — body", max = lp.max_iters)).cyan()
        );
        let body_ok = run_loop_block_sequentially(
            LoopBlockRequest {
                dispatcher,
                run,
                steps: &body_calls,
                phase_idx,
            },
            state,
            &mut iter_captured,
            &mut iter_skipped,
        );
        if !body_ok || *state.aborted {
            // Body step failed with abort policy or surfaced a hard
            // error. The outer mission is already marked aborted by
            // `run_loop_block_sequentially`; stop here.
            return;
        }

        eprintln!(
            "  {}",
            style(format!("  iter {iter}/{max} — verify", max = lp.max_iters)).cyan()
        );
        // Track the verify final call's captured bytes so we can
        // inspect `done` and, on a winning iteration, export
        // `<name>.result`.
        let verify_ok = run_loop_block_sequentially(
            LoopBlockRequest {
                dispatcher,
                run,
                steps: &verify_calls,
                phase_idx,
            },
            state,
            &mut iter_captured,
            &mut iter_skipped,
        );
        if !verify_ok || *state.aborted {
            return;
        }

        // RFC §4.4: inspect the final verify call's output for
        // `done: bool`. The final call MUST have produced bytes — a
        // successful step always writes to `iter_captured` under its
        // `output_binding` (planner-assigned; never `None` for a
        // lowered `let`-call) OR, for an un-bound call, the
        // interpreter does not capture. To make this check robust
        // regardless of whether the verify last call had an explicit
        // `let`, we capture by synthetic binding below.
        let final_call = &verify_calls[verify_last_idx];
        let final_capture: Option<&CapturedResult> = final_call
            .output_binding
            .as_ref()
            .and_then(|b| iter_captured.get(b))
            .or_else(|| iter_captured.get(LOOP_VERIFY_SYNTHETIC_BINDING));

        let final_capture = match final_capture {
            Some(c) => c,
            None => {
                eprintln!(
                    "  {}",
                    style(format!(
                        "loop '{label}' iter {iter}: VerifyMalformed — verify final call produced no output"
                    ))
                    .red()
                );
                *state.failed += 1;
                *state.aborted = true;
                return;
            }
        };

        let done = match verify_output_done(&final_capture.value) {
            VerifyDone::True => true,
            VerifyDone::False => false,
            VerifyDone::Malformed(reason) => {
                eprintln!(
                    "  {}",
                    style(format!(
                        "loop '{label}' iter {iter}: VerifyMalformed — {reason}"
                    ))
                    .red()
                );
                *state.failed += 1;
                *state.aborted = true;
                return;
            }
        };

        if done {
            eprintln!(
                "  {}",
                style(format!(
                    "loop '{label}' terminated at iter {iter}/{max} (verify done=true)",
                    max = lp.max_iters
                ))
                .green()
            );
            if let Some(ref rb) = lp.result_binding {
                // The loop result IS the winning iteration's final
                // verify output, so that step's invocation record is
                // the binding's producer. Downstream joins retain its
                // verified terminal receipt across the loop boundary.
                state.captured.insert(
                    rb.clone(),
                    CapturedResult {
                        value: final_capture.value.clone(),
                        invocation: final_capture.invocation.clone(),
                    },
                );
            }
            return;
        }
    }

    // Exhausted max_iters without done: true → LoopExhausted hard
    // abort (RFC §5.2). The mission's outcome is Aborted; no
    // downstream steps run.
    eprintln!(
        "  {}",
        style(format!(
            "loop '{label}': LoopExhausted — reached max_iters={n} without done=true (RFC §5.2)",
            n = lp.max_iters
        ))
        .red()
    );
    *state.failed += 1;
    *state.aborted = true;
}

/// Synthetic output-binding used internally when a verify final call
/// has no user-declared `let`. Never collides with a user binding
/// because `.` is not a legal identifier char in the grammar.
const LOOP_VERIFY_SYNTHETIC_BINDING: &str = "__loop_verify.output__";

/// Flatten a loop block's `Vec<IrStep>` into `Vec<IrCall>`. Planner
/// guarantees only `IrStep::Call` leaves inside loop bodies in v1
/// (no nested blocks — RFC §4.2). An unexpected non-Call variant
/// signals a planner bug and becomes an anyhow error surfaced up.
fn flatten_loop_leaves(
    block: &[RealIrStep],
    label: &str,
    which: &str,
) -> anyhow::Result<Vec<IrCall>> {
    let mut out = Vec::with_capacity(block.len());
    for step in block {
        match step {
            RealIrStep::Call(c) => out.push(c.clone()),
            RealIrStep::Loop(_) => {
                anyhow::bail!(
                    "loop '{label}' {which}: nested loops are rejected at compile time \
                     (RFC §4.2 v1) — reaching the executor with one is a planner bug"
                )
            }
        }
    }
    Ok(out)
}

/// Run a loop block (body or verify) sequentially in the caller-
/// provided scope. Returns `false` if any step failed with abort
/// policy (outer `aborted` is also set by `process_batch`).
///
/// Loop iterations are sequential within each block (RFC §6 v1: no
/// parallel body). Each step goes through the same dispatch path as
/// a flat Call, so `MAX_AGENT_DEPTH` and
/// `check_mission_context_invariant` are honoured unchanged (RFC
/// §4.3 single-path invariant).
///
/// The last step of the block is captured under
/// `LOOP_VERIFY_SYNTHETIC_BINDING` as a fallback, so the verify
/// `done: bool` check can read the final output even when the user
/// did not write an explicit `let` on that statement.
fn run_loop_block_sequentially(
    request: LoopBlockRequest<'_>,
    outer_state: &mut PhaseRunState<'_>,
    iter_captured: &mut HashMap<String, CapturedResult>,
    iter_skipped: &mut std::collections::HashSet<String>,
) -> bool {
    let LoopBlockRequest {
        dispatcher,
        run,
        steps,
        phase_idx,
    } = request;
    let last_idx = steps.len().saturating_sub(1);
    for (i, step) in steps.iter().enumerate() {
        if *outer_state.aborted {
            return false;
        }
        let merged_args = match resolve_arguments(step, iter_captured, iter_skipped) {
            Ok(args) => args,
            Err(ResolveError::UpstreamSkipped { binding, arg }) => {
                let result = StepExecResult::SkippedByDependency {
                    message: format!(
                        "skipped: input `{arg}` depends on `{binding}` which was skipped upstream"
                    ),
                    started_at: now_unix_ms(),
                };
                let mut local_skipped = 0usize;
                let mut block_state = PhaseRunState {
                    global_step: &mut *outer_state.global_step,
                    total: outer_state.total,
                    captured: iter_captured,
                    receipt_graph: &mut *outer_state.receipt_graph,
                    skipped_bindings: iter_skipped,
                    completed: &mut *outer_state.completed,
                    failed: &mut *outer_state.failed,
                    skipped: &mut local_skipped,
                    aborted: &mut *outer_state.aborted,
                    all_traces: &mut *outer_state.all_traces,
                };
                process_batch(
                    steps,
                    BatchProcessRequest {
                        phase_idx,
                        results: vec![(i, result)],
                    },
                    &mut block_state,
                );
                continue;
            }
            Err(ResolveError::Other(e)) => {
                let result = StepExecResult::Error {
                    message: e,
                    elapsed_ms: 0,
                    started_at: now_unix_ms(),
                    retry_count: 0,
                    retry_history: Vec::new(),
                };
                let mut local_skipped = 0usize;
                let mut block_state = PhaseRunState {
                    global_step: &mut *outer_state.global_step,
                    total: outer_state.total,
                    captured: iter_captured,
                    receipt_graph: &mut *outer_state.receipt_graph,
                    skipped_bindings: iter_skipped,
                    completed: &mut *outer_state.completed,
                    failed: &mut *outer_state.failed,
                    skipped: &mut local_skipped,
                    aborted: &mut *outer_state.aborted,
                    all_traces: &mut *outer_state.all_traces,
                };
                process_batch(
                    steps,
                    BatchProcessRequest {
                        phase_idx,
                        results: vec![(i, result)],
                    },
                    &mut block_state,
                );
                return !*outer_state.aborted;
            }
        };

        let mut merged_args = merged_args;
        substitute_receipt_graph(&mut merged_args, outer_state.receipt_graph);

        // Within an iteration, dependency receipts follow `input_refs`
        // exactly as in flat phases. Cross-iteration receipt
        // joins are out of scope until the loop-semantics RFC pins
        // how iteration receipts should join (iteration scopes are
        // hermetic per RFC §3.1 v1, so no binding can reference a
        // prior iteration today anyway).
        let dependency_receipts = dependency_receipts_from_captured(step, iter_captured);
        let result =
            execute_step_with_retry(dispatcher, run, step, &merged_args, &dependency_receipts);

        // Mirror the "capture under synthetic binding for last step"
        // side-effect by copying result_bytes into iter_captured
        // before handing to process_batch (which would only capture
        // if output_binding is Some). The invocation record rides
        // along so the loop's result binding can name its producer.
        if i == last_idx {
            if let StepExecResult::Ok {
                result_bytes,
                invocation,
                ..
            } = &result
            {
                iter_captured.insert(
                    LOOP_VERIFY_SYNTHETIC_BINDING.to_string(),
                    CapturedResult {
                        value: result_bytes.clone(),
                        invocation: invocation.clone(),
                    },
                );
            }
        }

        let mut local_skipped = 0usize;
        let mut block_state = PhaseRunState {
            global_step: &mut *outer_state.global_step,
            total: outer_state.total,
            captured: iter_captured,
            receipt_graph: &mut *outer_state.receipt_graph,
            skipped_bindings: iter_skipped,
            completed: &mut *outer_state.completed,
            failed: &mut *outer_state.failed,
            skipped: &mut local_skipped,
            aborted: &mut *outer_state.aborted,
            all_traces: &mut *outer_state.all_traces,
        };
        process_batch(
            steps,
            BatchProcessRequest {
                phase_idx,
                results: vec![(i, result)],
            },
            &mut block_state,
        );
    }
    !*outer_state.aborted
}
