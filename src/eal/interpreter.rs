// EasyNet CLI — EAL Interpreter
// =============================
//
// File: src/eal/interpreter.rs
// Description: Client-side execution engine for Mission IR v2 (temporary — target: MissionControl v2).
//
// Execution Model:
//   Phases execute sequentially (data-flow barriers between them).
//   Steps within a phase execute in parallel via rayon work-stealing threadpool.
//   When parallel dispatch is unavailable (BorrowedBridgeDispatcher), falls back to sequential.
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
//   BorrowedBridgeDispatcher or AgentAwareDispatcher; tests inject MockDispatcher.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use console::style;
use crossbeam_queue::SegQueue;
use easynet_axon::dendrite_bridge::DendriteBridge;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::shared::bridge_pool::BridgePool;

/// Convert `Duration::as_millis()` (u128) to u64, saturating at u64::MAX.
#[inline]
fn millis_u64(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}
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
    /// Ability invoked. Mirrors `IrStep.ability`. See
    /// `docs/AGENT_IDENTITY.md` §10 — this is a method name, not an
    /// identity.
    pub ability: crate::shared::agent_id::AbilityName,
    /// Resolved dispatch target. Mirrors `IrStep.target`. The trace
    /// records the *resolved* target (Agent vs Device) so audit
    /// readers don't have to re-classify.
    pub target: crate::eal::ir::IrTarget,
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
    /// Captured outputs from steps with `output_binding`.
    /// Key = binding name, Value = JSON string of the step's result.
    pub outputs: HashMap<String, String>,
}

// ── Internal captured result ──

struct CapturedResult {
    value: Vec<u8>,
}

// ── Dispatch backend trait (enables test injection) ──

use crate::eal::ir::IrTarget;
use crate::shared::agent_id::AbilityName;

pub trait StepDispatcher {
    /// Dispatch one step. The runtime sees only the resolved
    /// `IrTarget` enum and the typed `AbilityName` — there is no
    /// string-based `is_agent` check here, by design (see
    /// `docs/AGENT_IDENTITY.md` invariant 2).
    fn dispatch(
        &self,
        tenant: &str,
        target: &IrTarget,
        ability: &AbilityName,
        arguments: &Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, String>;

    /// Create an independent clone for parallel dispatch.
    /// Each thread in a phase needs its own dispatcher.
    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, String>;
}

/// Dispatcher that borrows a `DendriteBridge`.
///
/// **Device-only**: this dispatcher does not load the agent registry
/// and cannot dispatch to agent targets. It is used by `mcp::handlers::run_mission`
/// and tests where only device dispatch is in scope. Agent targets
/// produce a hard error so the wrong dispatcher is never silently used.
///
/// This cannot be used for true parallel dispatch because
/// `DendriteBridge` is `!Send`/`!Sync`. The engine will automatically
/// fall back to sequential dispatch for phases when
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
        target: &IrTarget,
        ability: &AbilityName,
        arguments: &Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, String> {
        match target {
            IrTarget::Device { node_id } => self
                .bridge
                .call_mcp_tool_with_timeout(tenant, ability.as_str(), node_id, arguments, timeout_ms)
                .map_err(|e| format!("{e}")),
            IrTarget::Agent(_) => Err(
                "BorrowedBridgeDispatcher cannot dispatch to agent targets; \
                 use AgentAwareDispatcher (e.g. via run_mission_inproc)"
                    .to_string(),
            ),
        }
    }

    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, String> {
        Err("BorrowedBridgeDispatcher cannot be cloned for threads (bridge is !Send/!Sync)".into())
    }
}

// ── Agent-Aware Dispatcher ──
//
// Matches on `IrTarget` to choose between agent CLI dispatch (via
// `agent::dispatch::send_to_agent`) and bridge dispatch. There is no
// `is_agent` string check anywhere — the surface form already chose
// the variant at parse time, and the planner baked it into the IR.
// See `docs/AGENT_IDENTITY.md` invariants 1 and 2.

pub struct AgentAwareDispatcher {
    pool: Arc<BridgePool>,
    registry: Arc<crate::shared::agents::AgentRegistry>,
}

impl AgentAwareDispatcher {
    pub fn new(endpoint: &str, timeout_ms: u64) -> Self {
        let registry = crate::shared::agents::load_agents()
            .unwrap_or_default();
        let pool = Arc::new(BridgePool::with_adaptive_size(endpoint, timeout_ms));
        Self {
            pool,
            registry: Arc::new(registry),
        }
    }

    /// Create a dispatcher with a pre-existing shared pool (for pool reuse across missions).
    #[allow(dead_code)]
    pub fn with_pool(pool: Arc<BridgePool>) -> Self {
        let registry = crate::shared::agents::load_agents()
            .unwrap_or_default();
        Self {
            pool,
            registry: Arc::new(registry),
        }
    }
}

impl StepDispatcher for AgentAwareDispatcher {
    fn dispatch(
        &self,
        tenant: &str,
        target: &IrTarget,
        ability: &AbilityName,
        arguments: &Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, String> {
        match target {
            IrTarget::Agent(agent_id) => {
                dispatch_to_agent(&self.registry, agent_id, ability, arguments)
            }
            IrTarget::Device { node_id } => {
                let guard = self.pool.checkout()?;
                guard
                    .bridge()
                    .call_mcp_tool_with_timeout(
                        tenant,
                        ability.as_str(),
                        node_id,
                        arguments,
                        timeout_ms,
                    )
                    .map_err(|e| format!("{e}"))
                // guard drops here → bridge returned to pool
            }
        }
    }

    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, String> {
        Ok(Box::new(AgentAwareDispatcher {
            pool: Arc::clone(&self.pool),
            registry: Arc::clone(&self.registry),
        }))
    }
}

// ── Pooled Bridge Dispatcher (for MCP server) ──
//
// Unlike BorrowedBridgeDispatcher which borrows a single bridge and
// cannot be cloned for threads, this dispatcher owns an Arc<BridgePool>
// and supports true parallel dispatch. Used by MCP server's run_mission
// handler to enable parallel phase execution.

pub struct PooledBridgeDispatcher {
    pool: Arc<BridgePool>,
}

impl PooledBridgeDispatcher {
    #[allow(dead_code)]
    pub fn new(endpoint: &str, timeout_ms: u64) -> Self {
        Self {
            pool: Arc::new(BridgePool::with_adaptive_size(endpoint, timeout_ms)),
        }
    }

    /// Create a dispatcher with a pre-existing shared pool (for pool reuse across missions).
    pub fn with_pool(pool: Arc<BridgePool>) -> Self {
        Self { pool }
    }
}

impl StepDispatcher for PooledBridgeDispatcher {
    fn dispatch(
        &self,
        tenant: &str,
        target: &IrTarget,
        ability: &AbilityName,
        arguments: &Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, String> {
        match target {
            IrTarget::Device { node_id } => {
                let guard = self.pool.checkout()?;
                guard
                    .bridge()
                    .call_mcp_tool_with_timeout(
                        tenant,
                        ability.as_str(),
                        node_id,
                        arguments,
                        timeout_ms,
                    )
                    .map_err(|e| format!("{e}"))
            }
            IrTarget::Agent(_) => Err(
                "PooledBridgeDispatcher cannot dispatch to agent targets; \
                 use AgentAwareDispatcher (e.g. via run_mission_inproc)"
                    .to_string(),
            ),
        }
    }

    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, String> {
        Ok(Box::new(PooledBridgeDispatcher {
            pool: Arc::clone(&self.pool),
        }))
    }
}

/// Shared agent dispatch logic used by AgentAwareDispatcher.
fn dispatch_to_agent(
    registry: &crate::shared::agents::AgentRegistry,
    agent_id: &crate::shared::agent_id::AgentId,
    ability: &AbilityName,
    arguments: &Value,
) -> Result<Value, String> {
    // Registry is keyed by string today (see Step 4
    // follow-up: registry will be keyed by AgentId itself).
    // For now, look up by the canonical Display form.
    let key = agent_id.to_string();
    let entry = registry
        .agents
        .get(&key)
        .or_else(|| {
            // Backwards-compat: registry files written
            // before the migration may use the bare name
            // form (`"claude"` instead of `"default/claude"`).
            // Fall back to the bare name when the agent
            // is in the default tenant.
            if agent_id.tenant == crate::shared::agent_id::DEFAULT_TENANT {
                registry.agents.get(&agent_id.name)
            } else {
                None
            }
        })
        .ok_or_else(|| format!("agent '{key}' not found in registry"))?;

    let prompt = build_agent_prompt(ability.as_str(), arguments);

    let response = crate::agent::dispatch::send_to_agent(
        &key,
        entry,
        &prompt,
        None,
        None,
        None,
    )
    .map_err(|e| format!("agent dispatch: {e}"))?;

    Ok(serde_json::json!({
        "ok": true,
        "agent": response.agent,
        "output": response.content,
        "model": response.model,
        "duration_ms": response.duration_ms,
    }))
}

/// Build a prompt for an agent from an EAL step's `function_name` and arguments.
///
/// The convention is: `function_name` becomes the task description,
/// arguments become context. The `prompt` argument, if present, is used directly.
fn build_agent_prompt(function_name: &str, arguments: &Value) -> String {
    // If there's a "prompt" key, use it directly.
    if let Some(prompt) = arguments.get("prompt").and_then(|v| v.as_str()) {
        return prompt.to_string();
    }

    // Otherwise, build from function name + all argument key-values.
    let mut parts = vec![format!("Task: {function_name}")];
    if let Some(obj) = arguments.as_object() {
        for (key, val) in obj {
            match val {
                Value::String(s) => parts.push(format!("{key}: {s}")),
                other => parts.push(format!("{key}: {other}")),
            }
        }
    }
    parts.join("\n\n")
}

// ── Execute with DendriteBridge (convenience) ──

/// Execute a mission using a borrowed bridge (sequential fallback).
///
/// This is the legacy path kept for callers that already hold a bridge.
/// For parallel execution, prefer `execute_pooled` or `execute_with_endpoint`.
/// Execute a mission with a pooled bridge dispatcher (parallel-capable, device-only).
///
/// Creates a new `PooledBridgeDispatcher` per call. For high-frequency callers
/// (MCP server), prefer `execute_pooled_shared` with a persistent pool.
#[allow(dead_code)]
pub fn execute_pooled(
    endpoint: &str,
    tenant: &str,
    ir: &MissionIr,
) -> anyhow::Result<ExecutionReport> {
    let dispatcher = PooledBridgeDispatcher::new(endpoint, crate::shared::BRIDGE_CONNECT_TIMEOUT_MS);
    execute_with_dispatcher(&dispatcher, tenant, ir)
}

#[allow(dead_code)]
pub fn execute(
    bridge: &DendriteBridge,
    tenant: &str,
    ir: &MissionIr,
) -> anyhow::Result<ExecutionReport> {
    let dispatcher = BorrowedBridgeDispatcher::new(bridge);
    execute_with_dispatcher(&dispatcher, tenant, ir)
}

/// Execute a mission reusing a shared BridgePool (amortizes connection cost across missions).
///
/// Preferred for high-frequency callers like the MCP server that execute many
/// missions within a single session.
pub fn execute_pooled_shared(
    pool: Arc<BridgePool>,
    tenant: &str,
    ir: &MissionIr,
) -> anyhow::Result<ExecutionReport> {
    let dispatcher = PooledBridgeDispatcher::with_pool(pool);
    execute_with_dispatcher(&dispatcher, tenant, ir)
}

/// Execute using a pooled, agent-aware dispatcher.
///
/// This enables true parallel dispatch within phases (each thread checks out
/// a bridge from the shared pool). **Agent-aware**: if a step's target is
/// an `IrTarget::Agent`, it is dispatched to the agent CLI instead of the Hub.
pub fn execute_with_endpoint(
    endpoint: &str,
    tenant: &str,
    ir: &MissionIr,
) -> anyhow::Result<ExecutionReport> {
    let dispatcher = AgentAwareDispatcher::new(endpoint, crate::shared::BRIDGE_CONNECT_TIMEOUT_MS);
    execute_with_dispatcher(&dispatcher, tenant, ir)
}

// ── Core execution engine ──

#[allow(clippy::too_many_lines, clippy::unnecessary_wraps)]
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
        let phase_label = if can_parallel {
            format!("phase {phase_idx}  parallel")
        } else {
            format!("phase {phase_idx}")
        };
        eprintln!("\n  {}", style(phase_label).cyan());

        // ── Scheduling: required steps first, then optional ──────────────
        //
        // Within a phase, steps have no mutual data dependencies and can run
        // in parallel.  However, required steps have higher scheduling
        // priority than optional ones: they execute first so that they get
        // first access to shared resources (API quotas, failure budgets, etc.)
        // and their failures are observed before optional work begins.
        //
        // Execution order within a phase:
        //   1. Dispatch all REQUIRED steps (parallel or sequential).
        //   2. Barrier — process results, detect abort.
        //   3. Dispatch all OPTIONAL steps (parallel or sequential).
        //   4. Process results.
        //
        // This ensures "optional = low priority" is encoded in scheduling,
        // not just in post-hoc failure handling.

        let required_indices: Vec<usize> = steps.iter().enumerate()
            .filter(|(_, s)| !s.optional)
            .map(|(i, _)| i)
            .collect();
        let optional_indices: Vec<usize> = steps.iter().enumerate()
            .filter(|(_, s)| s.optional)
            .map(|(i, _)| i)
            .collect();

        // Batch 1: required steps
        let required_results = dispatch_batch(
            dispatcher, tenant, steps, &required_indices, &captured, can_parallel,
        );
        process_batch(
            steps, required_results, phase_idx, &mut global_step, total,
            &mut captured, &mut completed, &mut failed, &mut skipped,
            &mut aborted, &mut all_traces,
        );

        // Batch 2: optional steps (skip if mission already aborted)
        if !aborted && !optional_indices.is_empty() {
            let optional_results = dispatch_batch(
                dispatcher, tenant, steps, &optional_indices, &captured, can_parallel,
            );
            process_batch(
                steps, optional_results, phase_idx, &mut global_step, total,
                &mut captured, &mut completed, &mut failed, &mut skipped,
                &mut aborted, &mut all_traces,
            );
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

    // Convert captured results to readable strings for the report.
    let outputs: HashMap<String, String> = captured.into_iter()
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

/// Dispatch a batch of steps (identified by `indices` into `steps`) in parallel or sequentially.
/// Returns `Vec<(local_idx, StepExecResult)>` sorted by `local_idx`.
///
/// Parallel path uses rayon's work-stealing threadpool (amortizes thread creation
/// across phases) and crossbeam's lock-free SegQueue for result collection.
fn dispatch_batch(
    dispatcher: &dyn StepDispatcher,
    tenant: &str,
    steps: &[IrStep],
    indices: &[usize],
    captured: &HashMap<String, CapturedResult>,
    parallel: bool,
) -> Vec<(usize, StepExecResult)> {
    if indices.is_empty() {
        return Vec::new();
    }
    if parallel && indices.len() > 1 {
        // Pre-resolve arguments and pre-clone dispatchers on the main thread,
        // so the rayon closure only captures Send types (no &dyn StepDispatcher).
        let mut tasks: Vec<(usize, Box<dyn StepDispatcher + Send>, Value)> = Vec::new();
        // Lock-free result queue — each rayon task pushes without contention.
        let collector = SegQueue::new();
        for &local_idx in indices {
            let step = &steps[local_idx];
            let merged_args = match resolve_arguments(step, captured) {
                Ok(args) => args,
                Err(e) => {
                    collector.push((local_idx, StepExecResult::Error {
                        message: e, elapsed_ms: 0, started_at: now_unix_ms(),
                        retry_count: 0, retry_history: Vec::new(),
                    }));
                    continue;
                }
            };
            let thread_dispatcher = match dispatcher.clone_for_thread() {
                Ok(d) => d,
                Err(e) => {
                    collector.push((local_idx, StepExecResult::Error {
                        message: e, elapsed_ms: 0, started_at: now_unix_ms(),
                        retry_count: 0, retry_history: Vec::new(),
                    }));
                    continue;
                }
            };
            tasks.push((local_idx, thread_dispatcher, merged_args));
        }
        // Spawn rayon tasks — closure captures only Send types.
        rayon::scope(|scope| {
            for (local_idx, thread_dispatcher, merged_args) in tasks {
                let step = &steps[local_idx];
                let collector_ref = &collector;
                scope.spawn(move |_| {
                    let result = execute_step_with_retry(
                        thread_dispatcher.as_ref(), tenant, step, &merged_args,
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
            let merged_args = match resolve_arguments(step, captured) {
                Ok(args) => args,
                Err(e) => {
                    results.push((local_idx, StepExecResult::Error {
                        message: e, elapsed_ms: 0, started_at: now_unix_ms(),
                        retry_count: 0, retry_history: Vec::new(),
                    }));
                    continue;
                }
            };
            let result = execute_step_with_retry(dispatcher, tenant, step, &merged_args);
            results.push((local_idx, result));
        }
        results
    }
}

/// Process a batch of dispatch results: update counters, capture outputs, build traces.
#[allow(clippy::too_many_arguments)]
fn process_batch(
    steps: &[IrStep],
    results: Vec<(usize, StepExecResult)>,
    phase_idx: usize,
    global_step: &mut usize,
    total: usize,
    captured: &mut HashMap<String, CapturedResult>,
    completed: &mut usize,
    failed: &mut usize,
    skipped: &mut usize,
    aborted: &mut bool,
    all_traces: &mut Vec<StepTrace>,
) {
    for (local_idx, exec_result) in results {
        *global_step += 1;
        let step = &steps[local_idx];

        let (outcome, trace, result_bytes) =
            process_step_result(step, exec_result, *global_step, total, phase_idx);

        match outcome {
            StepOutcome::Completed => {
                *completed += 1;
                if let Some(ref binding) = step.output_binding {
                    if let Some(bytes) = result_bytes {
                        captured.insert(
                            binding.clone(),
                            CapturedResult { value: bytes },
                        );
                    }
                }
            }
            StepOutcome::Failed => {
                *failed += 1;
                if !step.optional && matches!(step.on_failure, IrFailurePolicy::Abort) {
                    *aborted = true;
                }
            }
            StepOutcome::Skipped => *skipped += 1,
        }
        all_traces.push(trace);
    }
}

fn execute_step_with_retry(
    dispatcher: &dyn StepDispatcher,
    tenant: &str,
    step: &IrStep,
    arguments: &Value,
) -> StepExecResult {
    // MissionControl semantics: `max_retries` is the number of retries AFTER the
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
        let res = dispatcher.dispatch(
            tenant,
            &step.target,
            &step.ability,
            arguments,
            step_timeout_ms,
        );

        match res {
            Ok(result) => {
                let result_bytes = serde_json::to_vec(&result).unwrap_or_default();
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
                };
            }
            Err(e) => {
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
                    error: e.clone(),
                });
                // Last attempt: return error with retry info
                if attempt + 1 >= max_attempts {
                    let elapsed_ms = millis_u64(t0.elapsed());
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

    let elapsed_ms = millis_u64(t0.elapsed());
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

/// Returns (outcome, trace, `Option<result_bytes>`).
/// `result_bytes` is Some only on success — used for data flow capture.
#[allow(clippy::cast_precision_loss)] // elapsed_ms display — sub-ms precision not needed
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
                step.ability,
                step.target.display_string(),
                style("✓").green(),
                elapsed_ms as f64 / 1000.0,
            ));

            let size = result_bytes.len();
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
                input_refs: step.input_refs.clone(),
                output_binding: step.output_binding.clone(),
            };

            (outcome, trace, None)
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
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn now_unix_ms() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eal::{parser, planner};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    // ── Mock dispatcher for testing ──

    struct MockDispatcher {
        /// Per-call delay to simulate real work
        delay_ms: u64,
        /// Counter to track how many dispatch calls happened
        call_count: Arc<AtomicU32>,
        /// If set, fail the first N calls (for retry testing)
        fail_first_n: Arc<AtomicU32>,
        /// If set, fail calls whose function name is in this set
        fail_functions: Arc<std::collections::HashSet<String>>,
        /// Record of function names called (for ordering verification)
        calls: Arc<Mutex<Vec<(String, Instant)>>>,
    }

    impl MockDispatcher {
        fn new(delay_ms: u64) -> Self {
            Self {
                delay_ms,
                call_count: Arc::new(AtomicU32::new(0)),
                fail_first_n: Arc::new(AtomicU32::new(0)),
                fail_functions: Arc::new(std::collections::HashSet::new()),
                calls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_fail_first_n(mut self, n: u32) -> Self {
            self.fail_first_n = Arc::new(AtomicU32::new(n));
            self
        }

        fn with_fail_functions(mut self, names: &[&str]) -> Self {
            self.fail_functions = Arc::new(names.iter().map(|s| (*s).to_string()).collect());
            self
        }
    }

    impl StepDispatcher for MockDispatcher {
        fn dispatch(
            &self,
            _tenant: &str,
            _target: &IrTarget,
            ability: &AbilityName,
            _arguments: &Value,
            _timeout_ms: Option<u64>,
        ) -> Result<Value, String> {
            let ability_str = ability.as_str().to_string();
            let call_num = self.call_count.fetch_add(1, Ordering::SeqCst);
            self.calls
                .lock()
                .unwrap()
                .push((ability_str.clone(), Instant::now()));

            // Simulate work
            if self.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(self.delay_ms));
            }

            // Fail by ability name (deterministic — safe for parallel tests)
            if self.fail_functions.contains(&ability_str) {
                return Err(format!("simulated failure for {ability_str}"));
            }

            // Fail first N calls (order-dependent — use only in sequential phases)
            let fail_n = self.fail_first_n.load(Ordering::SeqCst);
            if call_num < fail_n {
                return Err(format!("simulated failure #{call_num}"));
            }

            Ok(serde_json::json!({
                "ok": true,
                "call_num": call_num,
                "function": ability_str,
            }))
        }

        fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, String> {
            Ok(Box::new(MockDispatcher {
                delay_ms: self.delay_ms,
                call_count: Arc::clone(&self.call_count),
                fail_first_n: Arc::clone(&self.fail_first_n),
                fail_functions: Arc::clone(&self.fail_functions),
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
        assert_eq!(st.ability.as_str(), "compute");
        assert_eq!(
            st.target,
            IrTarget::Device { node_id: "gpu".to_string() }
        );
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

        // Both steps land in the same phase (no data dependency).
        // Scheduling priority: required ("must-run") executes first → call #0.
        // Optional ("maybe") executes second → call #1.
        // fail_first_n(1) fails call #0 ("must-run"), which is not optional → Failed.
        //
        // But the test expects the *optional* step to fail and be skipped.
        // We need fail_first_n(2) so both fail, then:
        //   must-run (required, call #0) → Failed → abort? No: default on_failure is Continue.
        //   maybe (optional, call #1) → Skipped.
        //
        // Actually: the correct test is to fail only the optional step.
        // With priority scheduling, optional runs second (call #1).
        // fail_first_n only fails call #0, which hits must-run.
        // To fail only the optional step, use with_fail_functions.
        let dispatcher = MockDispatcher::new(0).with_fail_functions(&["maybe"]);
        let report =
            execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(report.steps_completed, 1, "must-run should succeed");
        assert_eq!(report.trace.steps_skipped, 1, "optional failure should be skipped");
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
            fn dispatch(
                &self,
                _: &str,
                _target: &IrTarget,
                ability: &AbilityName,
                _: &Value,
                _: Option<u64>,
            ) -> Result<Value, String> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(serde_json::json!({"ok": true, "function": ability.as_str()}))
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

    // ── Surface form → IR target asymmetry (anti-regression) ───────────────
    //
    // The two EAL surface forms intentionally lower to DIFFERENT IR
    // target variants:
    //
    //   member-call form  `claude.chat(prompt: "hi")`
    //     → IrTarget::Agent(AgentId { tenant: "default", name: "claude" })
    //
    //   traditional form  `call "chat" on "claude" with { prompt = "hi" }`
    //     → IrTarget::Device { node_id: "claude" }
    //
    // The asymmetry is the design (ontology §5: device is hosting
    // substrate, §6.4: agent is logical actor; surface forms encode
    // the distinction). The runtime dispatcher matches `IrTarget`
    // and never re-classifies. See AGENT_IDENTITY.md invariant 2.

    /// Dispatcher that records every `(target, ability, args)` tuple
    /// it receives. Used to verify the resolved dispatch shapes.
    struct ShapeRecordingDispatcher {
        seen: Arc<Mutex<Vec<(IrTarget, AbilityName, Value)>>>,
    }

    impl ShapeRecordingDispatcher {
        fn new() -> Self {
            Self { seen: Arc::new(Mutex::new(Vec::new())) }
        }
    }

    impl StepDispatcher for ShapeRecordingDispatcher {
        fn dispatch(
            &self,
            _tenant: &str,
            target: &IrTarget,
            ability: &AbilityName,
            arguments: &Value,
            _timeout_ms: Option<u64>,
        ) -> Result<Value, String> {
            self.seen.lock().unwrap().push((
                target.clone(),
                ability.clone(),
                arguments.clone(),
            ));
            Ok(serde_json::json!({"ok": true}))
        }

        fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, String> {
            Ok(Box::new(ShapeRecordingDispatcher {
                seen: Arc::clone(&self.seen),
            }))
        }
    }

    #[test]
    fn member_call_lowers_to_agent_target() {
        use crate::shared::agent_id::AgentId;

        let src = r#"
            mission "member-call" {
                let r = claude.chat(prompt: "hi")
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        assert_eq!(ir.steps.len(), 1);
        let step = &ir.steps[0];
        assert_eq!(step.ability.as_str(), "chat");
        assert_eq!(
            step.target,
            IrTarget::Agent(AgentId::parse("claude").unwrap()),
            "member-call must lower to IrTarget::Agent"
        );
    }

    #[test]
    fn traditional_call_lowers_to_device_target() {
        let src = r#"
            mission "traditional" {
                let r = call "chat" on "node-1" with { prompt = "hi" }
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        assert_eq!(ir.steps.len(), 1);
        let step = &ir.steps[0];
        assert_eq!(step.ability.as_str(), "chat");
        assert_eq!(
            step.target,
            IrTarget::Device { node_id: "node-1".to_string() },
            "traditional `call ... on ...` must lower to IrTarget::Device"
        );
    }

    #[test]
    fn member_call_dispatches_to_agent_via_recorder() {
        // The interpreter dispatch path receives the resolved
        // IrTarget::Agent — no string-based classification along the way.
        use crate::shared::agent_id::AgentId;

        let src = r#"
            mission "member-call" {
                let r = claude.chat(prompt: "hi")
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        let dispatcher = ShapeRecordingDispatcher::new();
        execute_with_dispatcher(&dispatcher, "tenant", &ir).unwrap();

        let seen = dispatcher.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].0,
            IrTarget::Agent(AgentId::parse("claude").unwrap())
        );
        assert_eq!(seen[0].1.as_str(), "chat");
        assert_eq!(
            seen[0].2.get("prompt").and_then(|v| v.as_str()),
            Some("hi")
        );
    }

    #[test]
    fn traditional_call_dispatches_to_device_via_recorder() {
        let src = r#"
            mission "traditional" {
                let r = call "chat" on "node-1" with { prompt = "hi" }
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        let dispatcher = ShapeRecordingDispatcher::new();
        execute_with_dispatcher(&dispatcher, "tenant", &ir).unwrap();

        let seen = dispatcher.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].0,
            IrTarget::Device { node_id: "node-1".to_string() }
        );
        assert_eq!(seen[0].1.as_str(), "chat");
    }
}
