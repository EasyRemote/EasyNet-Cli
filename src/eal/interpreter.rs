// EasyNet CLI — EAL Interpreter
// =============================
//
// File: src/eal/interpreter.rs
// Description: Client-side execution engine for Mission IR v2 (temporary — target: MissionControl v2).
//
// Execution Model:
//   Phases execute sequentially (data-flow barriers between them).
//   Steps within a phase execute in parallel via std::thread::scope + per-thread bridge.
//   When parallel dispatch is unavailable (BorrowedBridgeDispatcher), falls back to sequential.
//
// Core Capabilities:
//   1. True parallel dispatch — std::thread::scope + clone_for_thread() per step.
//   2. Structured ExecutionTrace — per-step audit log with timestamps, result hashes, retry history.
//   3. Retry with exponential backoff — delay = min(base * 2^attempt, max) + deterministic jitter.
//   4. Cross-phase data flow — results captured in HashMap, substituted into downstream input_refs.
//
// Dispatch Abstraction:
//   `trait StepDispatcher` decouples execution from transport. Production uses BridgeDispatcher
//   (new DendriteBridge per call); tests inject MockDispatcher for deterministic verification.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use console::style;
use easynet_axon::dendrite_bridge::DendriteBridge;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::ir::{IrFailurePolicy, IrStep, MissionIr};
use crate::shared::output;

// ── Retry constants ──

const RETRY_BASE_MS: u64 = 1000;
const RETRY_MAX_MS: u64 = 30_000;

// ── Execution trace (structured audit log) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub mission_id: String,
    pub mission_name: String,
    pub started_at_unix_ms: u64,
    pub completed_at_unix_ms: u64,
    pub total_elapsed_ms: u64,
    pub phase_count: usize,
    pub steps_completed: usize,
    pub steps_failed: usize,
    pub steps_skipped: usize,
    pub outcome: MissionOutcome,
    pub step_traces: Vec<StepTrace>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepTrace {
    pub step_id: String,
    pub function_name: String,
    pub target_node_id: String,
    pub phase_index: usize,
    pub started_at_unix_ms: u64,
    pub completed_at_unix_ms: u64,
    pub elapsed_ms: u64,
    pub outcome: StepOutcome,
    pub retry_count: u32,
    pub retry_history: Vec<RetryRecord>,
    pub result_size_bytes: Option<usize>,
    pub result_sha256: Option<String>,
    pub error: Option<String>,
    pub input_refs: HashMap<String, String>,
    pub output_binding: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryRecord {
    pub attempt: u32,
    pub elapsed_ms: u64,
    pub backoff_ms: u64,
    pub error: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MissionOutcome {
    Completed,
    Partial,
    Aborted,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StepOutcome {
    Completed,
    Failed,
    Skipped,
}

// ── Public execution result ──

pub struct ExecutionReport {
    pub total_elapsed_ms: u64,
    pub steps_completed: usize,
    pub steps_failed: usize,
    pub trace: ExecutionTrace,
}

// ── Internal captured result ──

struct CapturedResult {
    value: Vec<u8>,
}

// ── Dispatch backend trait (enables test injection) ──

pub trait StepDispatcher {
    fn dispatch(
        &self,
        tenant: &str,
        function_name: &str,
        target_node_id: &str,
        arguments: &Value,
        timeout_seconds: Option<u64>,
    ) -> Result<Value, String>;

    /// Create an independent clone for parallel dispatch.
    /// Each thread in a phase needs its own dispatcher.
    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, String>;
}

/// Production dispatcher using DendriteBridge.
pub struct BridgeDispatcher {
    endpoint: String,
    timeout_ms: u64,
}

impl BridgeDispatcher {
    pub fn new(endpoint: &str, timeout_ms: u64) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            timeout_ms,
        }
    }
}

impl StepDispatcher for BridgeDispatcher {
    fn dispatch(
        &self,
        tenant: &str,
        function_name: &str,
        target_node_id: &str,
        arguments: &Value,
        timeout_seconds: Option<u64>,
    ) -> Result<Value, String> {
        let bridge = DendriteBridge::connect(&self.endpoint, self.timeout_ms)
            .map_err(|e| format!("bridge connect: {e}"))?;
        bridge
            .call_mcp_tool_with_timeout(tenant, function_name, target_node_id, arguments, timeout_seconds)
            .map_err(|e| format!("{e}"))
    }

    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, String> {
        Ok(Box::new(BridgeDispatcher {
            endpoint: self.endpoint.clone(),
            timeout_ms: self.timeout_ms,
        }))
    }
}

/// Dispatcher that borrows a `DendriteBridge`.
///
/// This cannot be used for true parallel dispatch because `DendriteBridge` is `!Send`/`!Sync`.
/// The engine will automatically fall back to sequential dispatch for phases when
/// `clone_for_thread()` returns an error.
pub struct BorrowedBridgeDispatcher<'a> {
    bridge: &'a DendriteBridge,
}

impl<'a> BorrowedBridgeDispatcher<'a> {
    pub fn new(bridge: &'a DendriteBridge) -> Self {
        Self { bridge }
    }
}

impl StepDispatcher for BorrowedBridgeDispatcher<'_> {
    fn dispatch(
        &self,
        tenant: &str,
        function_name: &str,
        target_node_id: &str,
        arguments: &Value,
        timeout_seconds: Option<u64>,
    ) -> Result<Value, String> {
        self.bridge
            .call_mcp_tool_with_timeout(tenant, function_name, target_node_id, arguments, timeout_seconds)
            .map_err(|e| format!("{e}"))
    }

    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, String> {
        Err("BorrowedBridgeDispatcher cannot be cloned for threads (bridge is !Send/!Sync)".into())
    }
}

// ── Execute with DendriteBridge (convenience) ──

pub fn execute(
    bridge: &DendriteBridge,
    tenant: &str,
    ir: &MissionIr,
) -> anyhow::Result<ExecutionReport> {
    let dispatcher = BorrowedBridgeDispatcher::new(bridge);
    execute_with_dispatcher(&dispatcher, tenant, ir)
}

/// Execute using a dispatcher that connects to `endpoint` for each step.
///
/// This enables true parallel dispatch within phases (each thread uses its own
/// dispatcher instance).
pub fn execute_with_endpoint(
    endpoint: &str,
    tenant: &str,
    ir: &MissionIr,
) -> anyhow::Result<ExecutionReport> {
    let dispatcher = BridgeDispatcher::new(endpoint, 5000);
    execute_with_dispatcher(&dispatcher, tenant, ir)
}

// ── Core execution engine ──

pub fn execute_with_dispatcher(
    dispatcher: &dyn StepDispatcher,
    tenant: &str,
    ir: &MissionIr,
) -> anyhow::Result<ExecutionReport> {
    let mission_id = uuid::Uuid::new_v4().to_string();
    let mission_start = Instant::now();
    let started_at = now_unix_ms();

    let mut captured: HashMap<String, CapturedResult> = HashMap::new();
    let mut all_traces: Vec<StepTrace> = Vec::new();
    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;
    let mut aborted = false;

    let total = ir.steps.len();
    let mut global_step = 0usize;

    for (phase_idx, phase) in ir.phases.iter().enumerate() {
        if aborted {
            break;
        }
        let steps = &ir.steps[phase.start..phase.end];
        if steps.is_empty() {
            continue;
        }
        let wants_parallel = steps.len() > 1;
        let can_parallel = wants_parallel && dispatcher.clone_for_thread().is_ok();
        output::info(&format!(
            "\nphase {phase_idx}{}:",
            if can_parallel { " (parallel)" } else { "" }
        ));

        // Collect results from this phase: Vec<(local_idx, StepExecResult)>
        let phase_exec_results: Vec<(usize, StepExecResult)> = if can_parallel {
            // ── True parallel dispatch via std::thread::scope ──
            let collector: Mutex<Vec<(usize, StepExecResult)>> = Mutex::new(Vec::new());

            std::thread::scope(|scope| {
                for (local_idx, step) in steps.iter().enumerate() {
                    let collector_ref = &collector;

                    // Resolve arguments before spawning (read from prior phases)
                    let merged_args = resolve_arguments(step, &captured);
                    let merged_args = match merged_args {
                        Ok(args) => args,
                        Err(e) => {
                            collector_ref.lock().unwrap().push((local_idx, StepExecResult::Error {
                                message: e, elapsed_ms: 0, started_at: now_unix_ms(),
                                retry_count: 0, retry_history: Vec::new(),
                            }));
                            continue;
                        }
                    };

                    let thread_dispatcher = match dispatcher.clone_for_thread() {
                        Ok(d) => d,
                        Err(e) => {
                            collector_ref.lock().unwrap().push((local_idx, StepExecResult::Error {
                                message: e, elapsed_ms: 0, started_at: now_unix_ms(),
                                retry_count: 0, retry_history: Vec::new(),
                            }));
                            continue;
                        }
                    };

                    scope.spawn(move || {
                        let result = execute_step_with_retry(
                            thread_dispatcher.as_ref(), tenant, step, &merged_args,
                        );
                        collector_ref.lock().unwrap().push((local_idx, result));
                    });
                }
            });

            let mut results = collector.into_inner().unwrap();
            results.sort_by_key(|(idx, _)| *idx);
            results
        } else {
            // ── Sequential dispatch (fallback when the dispatcher can't be cloned for threads) ──
            let mut results: Vec<(usize, StepExecResult)> = Vec::new();
            for (local_idx, step) in steps.iter().enumerate() {
                let merged_args = resolve_arguments(step, &captured);
                let merged_args = match merged_args {
                    Ok(args) => args,
                    Err(e) => {
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
                let result = execute_step_with_retry(dispatcher, tenant, step, &merged_args);
                results.push((local_idx, result));
            }
            results
        };

        // Process results: update counters, capture outputs, build traces
        for (local_idx, exec_result) in phase_exec_results {
            global_step += 1;
            let step = &steps[local_idx];

            let (outcome, trace, result_bytes) =
                process_step_result(step, exec_result, global_step, total, phase_idx);

            match outcome {
                StepOutcome::Completed => {
                    completed += 1;
                    // Capture output for data flow to subsequent phases
                    if let Some(ref binding) = step.output_binding {
                        if let Some(bytes) = result_bytes {
                            captured.insert(
                                binding.clone(),
                                CapturedResult {
                                    value: bytes,
                                },
                            );
                        }
                    }
                }
                StepOutcome::Failed => {
                    failed += 1;
                    if !step.optional && matches!(step.on_failure, IrFailurePolicy::Abort) {
                        aborted = true;
                    }
                }
                StepOutcome::Skipped => skipped += 1,
            }
            all_traces.push(trace);
        }
    }

    let total_elapsed = mission_start.elapsed().as_millis() as u64;
    let completed_at = now_unix_ms();
    let outcome = if aborted {
        MissionOutcome::Aborted
    } else if failed > 0 {
        MissionOutcome::Partial
    } else {
        MissionOutcome::Completed
    };

    let trace = ExecutionTrace {
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
        step_traces: all_traces,
    };

    Ok(ExecutionReport {
        total_elapsed_ms: total_elapsed,
        steps_completed: completed,
        steps_failed: failed,
        trace,
    })
}

// ── Internals ──

enum StepExecResult {
    Ok {
        result_bytes: Vec<u8>,
        result_sha256: String,
        elapsed_ms: u64,
        started_at: u64,
        completed_at: u64,
        retry_count: u32,
        retry_history: Vec<RetryRecord>,
    },
    Error {
        message: String,
        elapsed_ms: u64,
        started_at: u64,
        retry_count: u32,
        retry_history: Vec<RetryRecord>,
    },
}

fn execute_step_with_retry(
    dispatcher: &dyn StepDispatcher,
    tenant: &str,
    step: &IrStep,
    arguments: &Value,
) -> StepExecResult {
    // MissionControl semantics: `max_retries` is the number of retries AFTER the
    // first attempt, so total attempts = 1 + max_retries.
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
            // Exponential backoff with deterministic jitter
            let backoff = compute_backoff(attempt, &step.step_id);
            std::thread::sleep(Duration::from_millis(backoff));
        }

        let attempt_start = Instant::now();
        let step_timeout = if step.timeout_seconds > 0 {
            Some(step.timeout_seconds as u64)
        } else {
            None
        };
        let res = dispatcher.dispatch(
            tenant,
            &step.function_name,
            &step.target_node_id,
            arguments,
            step_timeout,
        );

        match res {
            Ok(result) => {
                let result_bytes = serde_json::to_vec(&result).unwrap_or_default();
                let result_sha256 = sha256_hex(&result_bytes);
                let completed_at = now_unix_ms();
                let elapsed_ms = t0.elapsed().as_millis() as u64;
                return StepExecResult::Ok {
                    result_bytes,
                    result_sha256,
                    elapsed_ms,
                    started_at,
                    completed_at,
                    retry_count: attempt,
                    retry_history,
                };
            }
            Err(e) => {
                let attempt_elapsed = attempt_start.elapsed().as_millis() as u64;
                let backoff = if attempt + 1 < max_attempts {
                    compute_backoff(attempt + 1, &step.step_id)
                } else {
                    0
                };
                retry_history.push(RetryRecord {
                    attempt: attempt + 1,
                    elapsed_ms: attempt_elapsed,
                    backoff_ms: backoff,
                    error: e.clone(),
                });
                // Last attempt: return error with retry info
                if attempt + 1 >= max_attempts {
                    let elapsed_ms = t0.elapsed().as_millis() as u64;
                    return StepExecResult::Error {
                        message: e,
                        elapsed_ms,
                        started_at,
                        retry_count: attempt,
                        retry_history,
                    };
                }
            }
        }
    }

    let elapsed_ms = t0.elapsed().as_millis() as u64;
    StepExecResult::Error {
        message: "exhausted all retry attempts".into(),
        elapsed_ms,
        started_at,
        retry_count: max_attempts.saturating_sub(1),
        retry_history,
    }
}

fn resolve_arguments(
    step: &IrStep,
    results: &HashMap<String, CapturedResult>,
) -> Result<Value, String> {
    let mut args = step
        .static_arguments
        .as_object()
        .cloned()
        .unwrap_or_default();
    for (key, src_binding) in &step.input_refs {
        let captured = results
            .get(src_binding)
            .ok_or_else(|| format!("unresolved ref '{src_binding}'"))?;
        let val: Value =
            serde_json::from_slice(&captured.value).unwrap_or(Value::Null);
        args.insert(key.clone(), val);
    }
    Ok(Value::Object(args))
}

/// Returns (outcome, trace, Option<result_bytes>).
/// result_bytes is Some only on success — used for data flow capture.
fn process_step_result(
    step: &IrStep,
    result: StepExecResult,
    global_step: usize,
    total: usize,
    phase_idx: usize,
) -> (StepOutcome, StepTrace, Option<Vec<u8>>) {
    match result {
        StepExecResult::Ok {
            result_bytes,
            result_sha256,
            elapsed_ms,
            started_at,
            completed_at,
            retry_count,
            retry_history,
        } => {
            let bind_info = step
                .output_binding
                .as_ref()
                .map(|b| format!("  → ${b}"))
                .unwrap_or_default();
            let dep_info = if step.input_refs.is_empty() {
                String::new()
            } else {
                let refs: Vec<_> =
                    step.input_refs.values().map(|v| format!("${v}")).collect();
                format!("  (← {})", refs.join(", "))
            };
            let retry_info = if retry_count > 0 {
                format!("  ({retry_count} retries)")
            } else {
                String::new()
            };

            output::step(&format!(
                "[{global_step}/{total}] {:<20} {:<14} {} {:.1}s{bind_info}{dep_info}{retry_info}",
                step.function_name,
                step.target_node_id,
                style("✓").green(),
                elapsed_ms as f64 / 1000.0,
            ));

            let size = result_bytes.len();
            let trace = StepTrace {
                step_id: step.step_id.clone(),
                function_name: step.function_name.clone(),
                target_node_id: step.target_node_id.clone(),
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
                input_refs: step.input_refs.clone(),
                output_binding: step.output_binding.clone(),
            };

            (StepOutcome::Completed, trace, Some(result_bytes))
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
                step.function_name,
                step.target_node_id,
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
                function_name: step.function_name.clone(),
                target_node_id: step.target_node_id.clone(),
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
                input_refs: step.input_refs.clone(),
                output_binding: step.output_binding.clone(),
            };

            (outcome, trace, None)
        }
    }
}

fn compute_backoff(attempt: u32, step_id: &str) -> u64 {
    let base = RETRY_BASE_MS * 2u64.pow(attempt.saturating_sub(1));
    let capped = base.min(RETRY_MAX_MS);
    // Deterministic jitter based on step_id + attempt
    let mut hasher = Sha256::new();
    hasher.update(step_id.as_bytes());
    hasher.update(attempt.to_le_bytes());
    let hash = hasher.finalize();
    let jitter_seed = u64::from_le_bytes(hash[..8].try_into().unwrap());
    let jitter = jitter_seed % (RETRY_BASE_MS / 2 + 1);
    capped + jitter
}

fn sha256_hex(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    hex_encode(&hash)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eal::{ir::*, parser, planner};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use std::time::Instant;

    // ── Mock dispatcher for testing ──

    struct MockDispatcher {
        /// Per-call delay to simulate real work
        delay_ms: u64,
        /// Counter to track how many dispatch calls happened
        call_count: Arc<AtomicU32>,
        /// If set, fail the first N calls (for retry testing)
        fail_first_n: Arc<AtomicU32>,
        /// Record of function names called (for ordering verification)
        calls: Arc<Mutex<Vec<(String, Instant)>>>,
    }

    impl MockDispatcher {
        fn new(delay_ms: u64) -> Self {
            Self {
                delay_ms,
                call_count: Arc::new(AtomicU32::new(0)),
                fail_first_n: Arc::new(AtomicU32::new(0)),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_fail_first_n(mut self, n: u32) -> Self {
            self.fail_first_n = Arc::new(AtomicU32::new(n));
            self
        }
    }

    impl StepDispatcher for MockDispatcher {
        fn dispatch(
            &self,
            _tenant: &str,
            function_name: &str,
            _target_node_id: &str,
            _arguments: &Value,
            _timeout_seconds: Option<u64>,
        ) -> Result<Value, String> {
            let call_num = self.call_count.fetch_add(1, Ordering::SeqCst);
            self.calls
                .lock()
                .unwrap()
                .push((function_name.to_string(), Instant::now()));

            // Simulate work
            if self.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(self.delay_ms));
            }

            // Fail first N calls
            let fail_n = self.fail_first_n.load(Ordering::SeqCst);
            if call_num < fail_n {
                return Err(format!("simulated failure #{call_num}"));
            }

            Ok(serde_json::json!({
                "ok": true,
                "call_num": call_num,
                "function": function_name,
            }))
        }

        fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, String> {
            Ok(Box::new(MockDispatcher {
                delay_ms: self.delay_ms,
                call_count: Arc::clone(&self.call_count),
                fail_first_n: Arc::clone(&self.fail_first_n),
                calls: Arc::clone(&self.calls),
            }))
        }
    }

    // ── Test 1: Parallel dispatch actually runs concurrently ──

    #[test]
    fn parallel_dispatch_is_concurrent() {
        // 3 independent steps, each takes 100ms.
        // If sequential: ≥300ms. If parallel: ~100ms.
        let src = r#"
            mission "parallel-test" {
                let a = call "slow.op" on "n1"
                let b = call "slow.op" on "n2"
                let c = call "slow.op" on "n3"
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();

        // All 3 steps should be in phase 0 (independent)
        assert_eq!(ir.phases.len(), 1);

        let dispatcher = MockDispatcher::new(100);
        let t0 = Instant::now();
        let report =
            execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();
        let elapsed = t0.elapsed();

        assert_eq!(report.steps_completed, 3);
        assert_eq!(report.steps_failed, 0);
        assert_eq!(dispatcher.call_count.load(Ordering::SeqCst), 3);

        // Must finish in under 250ms (3×100ms serial would be ≥300ms)
        assert!(
            elapsed < Duration::from_millis(250),
            "parallel dispatch took {elapsed:?} — expected <250ms for 3×100ms steps"
        );
    }

    // ── Test 2: Sequential phases respect data dependencies ──

    #[test]
    fn sequential_phases_respect_order() {
        let src = r#"
            mission "chain" {
                let a = call "step1" on "n1"
                let b = call "step2" on "n1" with { input = a.output }
                let c = call "step3" on "n1" with { input = b.output }
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        assert_eq!(ir.phases.len(), 3);

        let dispatcher = MockDispatcher::new(10);
        let report =
            execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(report.steps_completed, 3);

        // Verify call ordering
        let calls = dispatcher.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].0, "step1");
        assert_eq!(calls[1].0, "step2");
        assert_eq!(calls[2].0, "step3");
        // Each call started after the previous one finished
        assert!(calls[1].1 > calls[0].1);
        assert!(calls[2].1 > calls[1].1);
    }

    // ── Test 3: Execution trace captures correct fields ──

    #[test]
    fn trace_captures_fields() {
        let src = r#"
            mission "traced" {
                let x = call "compute" on "gpu"
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();

        let dispatcher = MockDispatcher::new(0);
        let report =
            execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();
        let trace = &report.trace;

        assert_eq!(trace.mission_name, "traced");
        assert!(!trace.mission_id.is_empty());
        assert_eq!(trace.phase_count, 1);
        assert_eq!(trace.steps_completed, 1);
        assert_eq!(trace.steps_failed, 0);
        assert_eq!(trace.outcome, MissionOutcome::Completed);
        assert!(trace.started_at_unix_ms > 0);
        assert!(trace.completed_at_unix_ms >= trace.started_at_unix_ms);

        let st = &trace.step_traces[0];
        assert_eq!(st.step_id, "x");
        assert_eq!(st.function_name, "compute");
        assert_eq!(st.target_node_id, "gpu");
        assert_eq!(st.phase_index, 0);
        assert_eq!(st.outcome, StepOutcome::Completed);
        assert!(st.result_size_bytes.unwrap() > 0);
        assert!(st.result_sha256.is_some());
        assert!(st.error.is_none());
    }

    // ── Test 4: Trace is serializable to JSON ──

    #[test]
    fn trace_is_serializable() {
        let src = r#"mission "s" { let a = call "x" on "n" }"#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        let dispatcher = MockDispatcher::new(0);
        let report = execute_with_dispatcher(&dispatcher, "t", &ir).unwrap();
        let json = serde_json::to_string_pretty(&report.trace).unwrap();
        assert!(json.contains("\"mission_name\": \"s\""));
        assert!(json.contains("\"result_sha256\""));
        // Roundtrip
        let _: ExecutionTrace = serde_json::from_str(&json).unwrap();
    }

    // ── Test 5: Retry with exponential backoff ──

    #[test]
    fn retry_fires_correct_attempts() {
        let src = r#"
            mission "retry-test" {
                let x = call "flaky" on "n" retries 3 on_failure retry
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();

        // Fail first 2 calls, succeed on 3rd
        let dispatcher = MockDispatcher::new(0).with_fail_first_n(2);
        let report =
            execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(report.steps_completed, 1);
        assert_eq!(report.steps_failed, 0);
        // Total calls: 2 failures + 1 success = 3
        assert_eq!(dispatcher.call_count.load(Ordering::SeqCst), 3);

        let st = &report.trace.step_traces[0];
        assert_eq!(st.outcome, StepOutcome::Completed);
        assert_eq!(st.retry_count, 2); // 2 retries before success
        assert_eq!(st.retry_history.len(), 2);
        assert!(st.retry_history[0].error.contains("simulated failure"));
    }

    // ── Test 6: Retry exhaustion results in failure ──

    #[test]
    fn retry_exhaustion_fails() {
        let src = r#"
            mission "exhaust" {
                let x = call "always-fail" on "n" retries 2 on_failure retry
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();

        // Fail all calls
        let dispatcher = MockDispatcher::new(0).with_fail_first_n(100);
        let report =
            execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(report.steps_completed, 0);
        assert_eq!(report.steps_failed, 1);
        assert_eq!(report.trace.outcome, MissionOutcome::Partial);
        // max_retries=2 means 3 total attempts (1 + 2 retries)
        assert_eq!(dispatcher.call_count.load(Ordering::SeqCst), 3);

        let st = &report.trace.step_traces[0];
        assert_eq!(st.outcome, StepOutcome::Failed);
        assert!(st.error.is_some());
        assert_eq!(st.retry_count, 2);
        assert_eq!(st.retry_history.len(), 3); // all attempts failed
    }

    // ── Test 7: Abort policy stops execution ──

    #[test]
    fn abort_stops_subsequent_phases() {
        let src = r#"
            mission "abort-test" {
                let a = call "will-fail" on "n" on_failure abort
                let b = call "should-not-run" on "n" with { input = a.output }
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        assert_eq!(ir.phases.len(), 2);

        let dispatcher = MockDispatcher::new(0).with_fail_first_n(1);
        let report =
            execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(report.steps_failed, 1);
        assert_eq!(report.trace.outcome, MissionOutcome::Aborted);
        // Only 1 call made (step b never dispatched)
        assert_eq!(dispatcher.call_count.load(Ordering::SeqCst), 1);
        assert_eq!(report.trace.step_traces.len(), 1);
    }

    // ── Test 8: Optional step failure doesn't abort ──

    #[test]
    fn optional_step_skipped() {
        let src = r#"
            mission "opt" {
                call "maybe" on "n" optional
                let b = call "must-run" on "n"
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();

        // Fail first call (the optional one)
        let dispatcher = MockDispatcher::new(0).with_fail_first_n(1);
        let report =
            execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(report.steps_completed, 1);
        assert_eq!(report.trace.steps_skipped, 1);
        assert_eq!(report.trace.outcome, MissionOutcome::Completed);
    }

    // ── Test 9: Diamond graph phases + data flow ──

    #[test]
    fn diamond_parallel_phases() {
        let src = r#"
            mission "diamond" {
                let a = call "root" on "n1"
                let b = call "left" on "n2" with { input = a.output }
                let c = call "right" on "n3" with { input = a.output }
                let d = call "merge" on "n4" with { l = b.output, r = c.output }
            }
        "#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        assert_eq!(ir.phases.len(), 3);

        // Phase 1 (b,c) should run in parallel with 50ms delay each
        let dispatcher = MockDispatcher::new(50);
        let t0 = Instant::now();
        let report =
            execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();
        let elapsed = t0.elapsed();

        assert_eq!(report.steps_completed, 4);
        // Phase 0: 50ms, Phase 1: 50ms (parallel b+c), Phase 2: 50ms = ~150ms
        // If b,c were serial: 50+50+50+50 = 200ms
        assert!(
            elapsed < Duration::from_millis(200),
            "diamond took {elapsed:?} — parallel phase should save time"
        );
    }

    // ── Test 10: Backoff calculation is deterministic and exponential ──

    #[test]
    fn backoff_is_exponential_and_deterministic() {
        let b1 = compute_backoff(1, "step-a");
        let b2 = compute_backoff(2, "step-a");
        let b3 = compute_backoff(3, "step-a");

        // Base: 1000ms. Attempt 1: 1000 + jitter, attempt 2: 2000 + jitter, attempt 3: 4000 + jitter
        assert!(b1 >= RETRY_BASE_MS);
        assert!(b2 >= RETRY_BASE_MS * 2);
        assert!(b3 >= RETRY_BASE_MS * 4);
        assert!(b3 <= RETRY_MAX_MS + RETRY_BASE_MS); // capped

        // Deterministic: same inputs → same output
        assert_eq!(b1, compute_backoff(1, "step-a"));
        assert_eq!(b2, compute_backoff(2, "step-a"));

        // Different step_id → different jitter
        let b1_other = compute_backoff(1, "step-b");
        // Could be same by chance, but very unlikely with sha256
        // Just check it's valid range
        assert!(b1_other >= RETRY_BASE_MS);
    }

    // ── Test 11: Graceful fallback to sequential when clone_for_thread fails ──

    #[test]
    fn fallback_to_sequential_when_not_cloneable() {
        // Non-cloneable dispatcher simulates BorrowedBridgeDispatcher
        struct SeqOnlyDispatcher(Arc<AtomicU32>);
        impl StepDispatcher for SeqOnlyDispatcher {
            fn dispatch(&self, _: &str, f: &str, _: &str, _: &Value, _: Option<u64>) -> Result<Value, String> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(serde_json::json!({"ok": true, "function": f}))
            }
            fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, String> {
                Err("not cloneable".into())
            }
        }

        // 3 independent steps — normally would run in parallel
        let src = r#"mission "f" { let a = call "x" on "n1" let b = call "y" on "n2" let c = call "z" on "n3" }"#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        assert_eq!(ir.phases.len(), 1);

        let count = Arc::new(AtomicU32::new(0));
        let dispatcher = SeqOnlyDispatcher(Arc::clone(&count));
        let report = execute_with_dispatcher(&dispatcher, "t", &ir).unwrap();

        // All 3 steps succeed despite no parallel support
        assert_eq!(report.steps_completed, 3);
        assert_eq!(report.steps_failed, 0);
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    // ── Test 12: Cross-phase data flow propagates results correctly ──

    #[test]
    fn cross_phase_data_flow() {
        let src = r#"
            mission "flow" {
                let a = call "produce" on "n1"
                let b = call "consume" on "n2" with { input = a.output }
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        assert_eq!(ir.phases.len(), 2);

        let dispatcher = MockDispatcher::new(0);
        let report = execute_with_dispatcher(&dispatcher, "t", &ir).unwrap();

        assert_eq!(report.steps_completed, 2);
        // Verify trace shows data flow connections
        let traces = &report.trace.step_traces;
        assert_eq!(traces[0].output_binding, Some("a".into()));
        assert!(traces[0].result_sha256.is_some());
        assert_eq!(traces[1].input_refs.get("input"), Some(&"a".into()));
        // Both steps have result hashes (proving they executed and returned data)
        assert!(traces[1].result_sha256.is_some());
    }
}
