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

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use console::style;
use crossbeam_queue::SegQueue;
use easynet_axon::dendrite_bridge::DendriteBridge;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Convert `Duration::as_millis()` (u128) to u64, saturating at u64::MAX.
#[inline]
fn millis_u64(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}
use sha2::{Digest, Sha256};

use super::ir::{IrCall, IrFailurePolicy, IrLoop, IrStep as RealIrStep, MissionIr};
use crate::support::output;

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

#[derive(Debug)]
enum PhasePartition<'a> {
    /// Contiguous run of `IrStep::Call` — dispatched via the
    /// existing parallel path when permitted by the dispatcher.
    Calls(&'a [RealIrStep]),
    /// A single Loop block — executed sequentially in-process via
    /// `execute_loop`.
    Loop(&'a IrLoop),
}

fn split_phase_steps(steps: &[RealIrStep]) -> Vec<PhasePartition<'_>> {
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
fn calls_from_partition(steps: &[RealIrStep]) -> Vec<IrCall> {
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

const RETRY_BASE_MS: u64 = 1000;
const RETRY_MAX_MS: u64 = 30_000;

// ── Execution trace (structured audit log) ──

/// On-disk schema version for `ExecutionTrace` JSON. Bump on any wire
/// change that older readers cannot interpret (renamed fields, removed
/// variants, changed numeric ranges). Adding *optional* fields with
/// `#[serde(default)]` does NOT warrant a bump — old readers will
/// transparently ignore them, which is the entire point of `default`.
///
/// The version is stamped on every fresh trace so trace consumers can
/// branch on layout. Absent-version on a parsed trace means "pre-stamp";
/// tolerant readers should treat it as `1`. The golden test
/// `trace_schema_v1_is_stable` pins the exact serialized shape so a
/// regression here cannot land silently.
pub const EXECUTION_TRACE_SCHEMA_VERSION: u32 = 1;

fn current_trace_schema_version() -> u32 {
    EXECUTION_TRACE_SCHEMA_VERSION
}

/// Maximum number of `StepTrace` entries kept in memory per mission.
/// Beyond this, the interpreter retains only the *head* (first
/// `TRACE_CAP_HEAD`) and the *tail* (last `TRACE_CAP_TAIL`) and
/// records the omission on [`ExecutionTrace::traces_truncated`].
///
/// Why a cap exists:
/// For a realistic EAL mission (tens to hundreds of steps) this
/// value is never hit, so in-memory behaviour is unchanged. For a
/// pathological mission (thousands+ of steps with retry histories),
/// an unbounded `Vec<StepTrace>` would accumulate hundreds of MB of
/// live data through the `ExecutionReport` return path. The cap
/// bounds peak memory at ~500KB worst case regardless of step count,
/// while preserving the forensically useful head-and-tail view
/// (the first steps show the mission shape; the last steps show the
/// outcome). Operators who need every step can read the P1-P6
/// Timeline event log at `$AXON_INVOCATION_LOG_DIR/<id>.jsonl`,
/// which is unaffected by this cap.
///
/// These constants are deliberately `pub const` so downstream
/// tooling can branch on "was this trace truncated?" and size its
/// own buffers accordingly.
pub const TRACE_CAP_HEAD: usize = 500;
pub const TRACE_CAP_TAIL: usize = 500;
/// Sum of head + tail — exported for downstream tooling that wants to
/// pre-size its own buffers to the same ceiling. Not used directly by
/// the interpreter; the two slots are checked independently in
/// [`CappedTraceBuffer::push`].
#[allow(dead_code)]
pub const TRACE_CAP_TOTAL: usize = TRACE_CAP_HEAD + TRACE_CAP_TAIL;

/// In-memory bounded buffer for `StepTrace` entries.
///
/// Retains the first `TRACE_CAP_HEAD` entries verbatim, and a rolling
/// window of the most recent `TRACE_CAP_TAIL` entries. Entries that
/// fall off the tail (after both head and tail are full) are counted
/// in [`CappedTraceBuffer::dropped`]; the interpreter propagates that
/// count into `ExecutionTrace::traces_truncated` at finalization.
///
/// This shape is deliberately chosen for forensics: the head shows
/// the mission setup and first-phase behaviour; the tail shows how
/// the mission ended. The middle of a very-long-mission is typically
/// the least interesting part — retries look the same in aggregate
/// regardless of which attempt you see.
struct CappedTraceBuffer {
    head: Vec<StepTrace>,
    tail: std::collections::VecDeque<StepTrace>,
    dropped: usize,
}

impl CappedTraceBuffer {
    fn new() -> Self {
        Self {
            head: Vec::with_capacity(TRACE_CAP_HEAD.min(64)),
            tail: std::collections::VecDeque::with_capacity(TRACE_CAP_TAIL.min(64)),
            dropped: 0,
        }
    }

    fn push(&mut self, t: StepTrace) {
        if self.head.len() < TRACE_CAP_HEAD {
            self.head.push(t);
            return;
        }
        if self.tail.len() == TRACE_CAP_TAIL {
            // Roll the oldest tail entry out; count it as dropped.
            self.tail.pop_front();
            self.dropped += 1;
        }
        self.tail.push_back(t);
    }

    /// Consume the buffer and return (combined entries, dropped count).
    /// The ordering is head then tail — chronologically consistent with
    /// `push` order.
    fn into_parts(self) -> (Vec<StepTrace>, usize) {
        let Self {
            mut head,
            tail,
            dropped,
        } = self;
        head.reserve(tail.len());
        head.extend(tail);
        (head, dropped)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.head.len() + self.tail.len()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    /// Schema version of this trace document. See
    /// `EXECUTION_TRACE_SCHEMA_VERSION` for the contract on bumping.
    /// `#[serde(default)]` lets old on-disk traces (which lacked this
    /// field) deserialize as version 1 — matching the tolerant-read
    /// promise documented above.
    #[serde(default = "current_trace_schema_version")]
    pub schema_version: u32,
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
    /// Number of steps whose traces were dropped due to the
    /// in-memory cap (see `TRACE_CAP_TOTAL`). Zero means every step
    /// is present in `step_traces`. Nonzero means there are exactly
    /// this many consecutive steps *between the head and tail slices*
    /// whose trace entries are absent from `step_traces`. This field
    /// is `#[serde(default)]` so older on-disk traces (before the cap
    /// existed) parse as `0` — preserving the tolerant-read promise.
    #[serde(default)]
    pub traces_truncated: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepTrace {
    pub step_id: String,
    /// Ability invoked. Mirrors `IrStep.ability`. See
    /// `docs/AGENT_IDENTITY.md` §10 — this is a method name, not an
    /// identity.
    pub ability: crate::core::agent_id::AbilityName,
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
    /// Mirrors `IrStep::input_refs` — kept as a `BTreeMap` for the same
    /// reason: stable JSON output for trace files. A trace JSON whose
    /// key order shifted between runs would defeat any "diff two
    /// mission runs" workflow.
    pub input_refs: BTreeMap<String, String>,
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
    #[allow(dead_code)]
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

use crate::core::agent_id::AbilityName;
use crate::eal::error::EalError;
use crate::eal::ir::IrTarget;

/// Joint-plan unified path for EAL device-targeted dispatch
/// (海峰 + 凉冰, 2026-05-03). Every cross-device step routes
/// through `federation.forward_invoke` against the target device
/// URA — the same surface `easynet ability invoke --node` uses.
/// Pre-cut every `BorrowedBridgeDispatcher` / `AgentAwareDispatcher` /
/// `PooledBridgeDispatcher` returned `EalError::Unavailable` for
/// `IrTarget::Device`; this helper actually performs the call so
/// EAL programs can target peer devices uniformly with the rest
/// of the CLI.
///
/// `node_id` accepts:
///   * `local` / empty / this device's own node id — short-circuits to
///     `invoke_local_ability` against the local daemon's control
///     socket (skips the forward_invoke gRPC round-trip — there is
///     nothing to forward to that the local dispatcher does not
///     already see).
///   * a canonical URA `easynet:///r/<realm>/device/<id>` — forward.
///   * a bare uuid that does NOT match this device — wrapped in
///     `tenant`'s realm and forwarded.
///
/// Pre-fix the `local` arm wrapped the literal string `"local"` into
/// `easynet:///r/<tenant>/device/local`, which forward_invoke then
/// reported as `target_offline` (the PresenceRegistry has no such
/// entry — the daemon's self URI uses its real node uuid, not the
/// keyword). Mission programs that wrote `call "shell.run" on "local"`
/// therefore failed every step. The local short-circuit fixes that
/// without changing the EAL surface.
fn dispatch_remote_via_forward_invoke(
    tenant: &str,
    node_id: &str,
    ability_name: &str,
    arguments: &Value,
) -> Result<Value, EalError> {
    #[cfg(feature = "axon-pb")]
    {
        let trimmed = node_id.trim();

        // Local short-circuit: `local`, empty, or this device's own
        // node id all dispatch through the local daemon's control
        // socket, the same surface every other in-process invocation
        // uses. Skip the forward_invoke envelope entirely — the
        // self-target shortcut on the daemon side covers a different
        // case (canonical self URI), not the keyword `local`.
        let self_node = crate::persistence::config::load_credentials()
            .ok()
            .map(|c| c.node_id);
        let is_local = trimmed.is_empty()
            || trimmed.eq_ignore_ascii_case("local")
            || self_node
                .as_deref()
                .is_some_and(|n| !n.is_empty() && trimmed == n);
        if is_local {
            return crate::support::local_invoke::invoke_local_ability(
                ability_name,
                arguments.clone(),
            )
            .map_err(|e| {
                EalError::Unavailable(format!("invoke_local_ability {ability_name} (local): {e}"))
            });
        }

        let target_ura = if crate::ura::parse_ura(trimmed).is_ok() {
            crate::support::federation_invoke::parse_node_uri(trimmed)
                .map_err(|e| EalError::Validation(format!("parse target URI: {e}")))?
        } else if !tenant.is_empty() {
            crate::ura::device_ura(tenant, trimmed)
        } else {
            return Err(EalError::Validation(format!(
                "cannot resolve EAL device target {trimmed:?}: no tenant in scope; \
                 pass a canonical `easynet:///r/<realm>/device/<id>` URI"
            )));
        };

        let caller_ura = crate::persistence::config::load_credentials()
            .ok()
            .filter(|c| !c.tenant_id.trim().is_empty() && !c.node_id.trim().is_empty())
            .map(|c| crate::ura::device_ura(c.tenant_id.trim(), c.node_id.trim()));
        crate::support::federation_invoke::invoke_via_federation_forward(
            ability_name,
            arguments.clone(),
            &target_ura,
            caller_ura.as_deref(),
        )
        .map_err(|e| {
            EalError::Unavailable(format!("forward_invoke {ability_name} → {target_ura}: {e}"))
        })
    }
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = (tenant, node_id, ability_name, arguments);
        Err(EalError::Unavailable(
            "EAL device-targeted dispatch requires the `axon-pb` feature; \
             rebuild with `--features axon-pb` (production builds always do)."
                .to_string(),
        ))
    }
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
    fn dispatch(
        &self,
        tenant: &str,
        target: &IrTarget,
        ability: &AbilityName,
        arguments: &Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, EalError>;

    /// Create an independent clone for parallel dispatch.
    /// Each thread in a phase needs its own dispatcher.
    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError>;
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
        _timeout_ms: Option<u64>,
    ) -> Result<Value, EalError> {
        match target {
            IrTarget::Device { node_id } => {
                let _ = &self.bridge;
                dispatch_remote_via_forward_invoke(tenant, node_id, ability.as_str(), arguments)
            }
            // Agent target on a Device-only dispatcher is a planner /
            // call-site contract violation, not a transient failure —
            // categorise as Validation so retries don't fire and so
            // operators see "validation_error" in the trace, not the
            // misleading "unavailable".
            IrTarget::Agent(_) => Err(EalError::Validation(
                "BorrowedBridgeDispatcher cannot dispatch to agent targets; \
                 use AgentAwareDispatcher (e.g. via run_mission_inproc)"
                    .to_string(),
            )),
        }
    }

    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
        // The cannot-clone outcome is the *signal* to the parallel
        // dispatch path that it must fall back to sequential — see
        // `dispatch_batch`. `Internal` is the right category here:
        // this is not a caller bug, just a structural property of the
        // borrowed-bridge dispatcher being `!Send`.
        Err(EalError::Internal(
            "BorrowedBridgeDispatcher cannot be cloned for threads (bridge is !Send/!Sync)"
                .to_string(),
        ))
    }
}

// ── Agent-Aware Dispatcher ──
//
// Matches on `IrTarget` to choose between agent CLI dispatch (via
// `runtime::dispatch::send_to_agent`) and bridge dispatch. There is no
// `is_agent` string check anywhere — the surface form already chose
// the variant at parse time, and the planner baked it into the IR.
// See `docs/AGENT_IDENTITY.md` invariants 1 and 2.

pub struct AgentAwareDispatcher {
    registry: Arc<crate::registry::agents::AgentRegistry>,
}

impl AgentAwareDispatcher {
    pub fn new(_endpoint: &str, _timeout_ms: u64) -> Self {
        let registry = load_registry_or_warn();
        Self {
            registry: Arc::new(registry),
        }
    }
}

/// Load the agent registry, logging a visible warning if the load fails.
///
/// Previously this was `load_agents().unwrap_or_default()`, which turned
/// "registry file is corrupt / home dir missing / permission denied"
/// into "you have no registered agents", so an EAL member-call like
/// `claude.chat(...)` would fail downstream with `agent '…' not found
/// in registry` — a classic false-negative that sends operators hunting
/// for a mis-registered agent when the real problem is upstream.
///
/// We still want a usable dispatcher when no agents are registered
/// (that is a legitimate first-run state), so we return an empty
/// registry on failure *after* logging. The distinction between
/// "empty by design" and "empty by failure" is preserved in operator-
/// visible logs rather than hidden from the caller.
fn load_registry_or_warn() -> crate::registry::agents::AgentRegistry {
    match crate::registry::agents::load_agents() {
        Ok(r) => r,
        Err(e) => {
            eprintln!(
                "[easynet eal] warning: agent registry load failed ({e}); \
                 dispatching with an empty registry. Any agent-target call \
                 will fail with `not_found` until the registry is repaired."
            );
            crate::registry::agents::AgentRegistry::default()
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
    ) -> Result<Value, EalError> {
        match target {
            IrTarget::Agent(agent_id) => {
                dispatch_to_agent(&self.registry, agent_id, ability, arguments)
            }
            IrTarget::Device { node_id } => {
                let _ = timeout_ms;
                dispatch_remote_via_forward_invoke(tenant, node_id, ability.as_str(), arguments)
            }
        }
    }

    fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
        Ok(Box::new(AgentAwareDispatcher {
            registry: Arc::clone(&self.registry),
        }))
    }
}

/// Shared agent dispatch logic used by AgentAwareDispatcher.
fn dispatch_to_agent(
    registry: &crate::registry::agents::AgentRegistry,
    agent_id: &crate::core::agent_id::AgentId,
    ability: &AbilityName,
    arguments: &Value,
) -> Result<Value, EalError> {
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
            if agent_id.tenant == crate::core::agent_id::DEFAULT_TENANT {
                registry.agents.get(&agent_id.name)
            } else {
                None
            }
        })
        // Missing agent in registry is `not_found`, not `unavailable` —
        // the caller's identifier doesn't resolve and a retry of the
        // same id will not help.
        .ok_or_else(|| EalError::NotFound(format!("agent '{key}' not found in registry")))?;

    // Fast path: if the target ability has an `[exec]` binding in its
    // on-disk manifest, run the executor directly and skip spawning
    // the LLM. This is the EAL counterpart of the dispatcher's
    // shell-exec short-circuit in `chat_ability::build_agent_ability_handler`
    // — both paths converge on `manifests_for(...) → run_shell_exec(...)`
    // for a deterministic ability. Without this branch, EAL's
    // `agent.ability(...)` syntax would always go through the chat
    // CLI even when the manifest pinned a concrete argv, and a
    // weather lookup that should take 200 ms would burn 30 s of LLM
    // tool-search latency.
    let bare_ability = ability.as_str();
    let manifest_match = crate::runtime::abilities::manifests_for(&agent_id.name, entry)
        .into_iter()
        .find(|m| m.name() == bare_ability);
    if let Some(manifest) = manifest_match {
        if let Some(exec) = manifest.exec() {
            let timeout = manifest
                .timeout_seconds()
                .map(std::time::Duration::from_secs);
            return match exec {
                crate::core::ability_spec::AbilityExec::Shell(spec) => {
                    crate::runtime::agents::shell_executor::run_shell_exec(spec, arguments, timeout)
                        .map_err(|e| EalError::Unavailable(format!("shell exec: {e}")))
                }
                crate::core::ability_spec::AbilityExec::Http(spec) => {
                    crate::runtime::agents::http_executor::run_http_exec(spec, arguments, timeout)
                        .map_err(|e| EalError::Unavailable(format!("http exec: {e}")))
                }
                crate::core::ability_spec::AbilityExec::Eal(spec) => {
                    crate::runtime::agents::eal_executor::run_eal_exec(spec, arguments, timeout)
                        .map_err(|e| EalError::Unavailable(format!("eal exec: {e}")))
                }
                crate::core::ability_spec::AbilityExec::Mcp(spec) => {
                    let _ = timeout;
                    crate::runtime::agents::mcp_executor::run_mcp_exec(spec, arguments)
                        .map_err(|e| EalError::Unavailable(format!("mcp exec: {e}")))
                }
            };
        }
    }

    // `<agent>.chat` is special: when an EAL mission desugars
    // `easynet agent send` it wants the driver's live stderr
    // timeline in the *current* CLI process. Routing chat through
    // the daemon's unary Invoke RPC would hide that live output in
    // the daemon process and reduce the caller to a final snapshot.
    // Keep chat local by reusing the daemon handler's own parsing /
    // context / resume logic directly in-process.
    if bare_ability == crate::runtime::agents::chat_ability::ABILITY_VERB {
        return crate::runtime::agents::chat_ability::invoke_direct_with_progress(
            &agent_id.name,
            entry,
            &[],
            arguments.clone(),
            None,
        )
        .map_err(|e| EalError::Unavailable(format!("agent chat: {e}")));
    }

    // Second fast path: try the local daemon's ability registry over
    // the control socket. The daemon registers per-agent self-bundle
    // verbs (`<agent>.discover`, `<agent>.invoke`, …) plus any
    // workspace-declared `<agent>.<verb>` whose manifest does NOT
    // pin an `[exec]` block but DOES have a real registered handler
    // (Rust builtin or shell-via-handler). Without this branch EAL
    // would fall straight through to the chat-fulfils path even when
    // a deterministic handler existed in the daemon — a 30 s LLM
    // round-trip for what should be a sub-second registry call.
    //
    // We do the IPC round-trip here only when the manifest path
    // above did NOT short-circuit. Failure modes are explicit:
    //
    //   * daemon down → propagate as Unavailable (caller may retry).
    //   * daemon returned `ability_not_found` → fall through to the
    //     chat path (preserves the legacy "manifest declares an
    //     ability but only chat can fulfil it" behaviour).
    //   * daemon returned any other typed error → propagate.
    let qualified = format!("{}.{}", agent_id.name, ability.as_str());
    match try_dispatch_via_daemon(&qualified, arguments) {
        DaemonDispatch::Result(value) => return Ok(value),
        DaemonDispatch::AbilityNotFound => { /* fall through to chat */ }
        DaemonDispatch::DaemonDown(reason) => {
            return Err(EalError::Unavailable(format!("daemon: {reason}")));
        }
        DaemonDispatch::Error(reason) => {
            return Err(EalError::Unavailable(format!("daemon: {reason}")));
        }
    }

    let prompt = build_agent_prompt(ability.as_str(), arguments);

    // Agent CLI dispatch failures (process spawn, IO, model error) are
    // transport-class — `unavailable` is the right bucket so the
    // interpreter's retry policy can fire when configured.
    //
    // IMPORTANT: pass the BARE agent name to send_to_agent, not the
    // namespaced AgentId form (`default/claude`). Downstream
    // workspace::ensure_workspace runs the registry name validator
    // against this string, and the validator legitimately rejects
    // anything containing `/`. The namespaced form is the registry
    // *lookup* identity above; the *runtime* identity is the bare
    // name. Using the wrong one here was the root cause of every
    // EAL→agent dispatch failing with
    // "workspace provisioning failed: agent.toml: name = "default/claude"
    //  must contain only lowercase ASCII letters, …" before the
    // CLI could even spawn.
    let response =
        crate::runtime::dispatch::send_to_agent(&agent_id.name, entry, &prompt, None, None)
            .map_err(|e| EalError::Unavailable(format!("agent dispatch: {e}")))?;

    Ok(serde_json::json!({
        "ok": true,
        "agent": response.agent,
        "output": response.content,
        "model": response.model,
        "duration_ms": response.duration_ms,
    }))
}

/// Outcome of attempting to dispatch a `<agent>.<verb>` call through
/// the local daemon's control socket.
///
/// Why a custom enum (rather than `Result<Option<Value>, ...>`)
/// -----------------------------------------------------------
/// `dispatch_to_agent` needs to make three decisions on the result:
///   1. Got a value → return it directly.
///   2. Daemon told us "no such ability" → silently fall through to
///      the chat path. Legacy abilities that exist in name only (a
///      manifest declares the verb but the daemon never registered
///      a handler) STILL need the chat translation, and the caller
///      must not see a stack trace just because the daemon was
///      consulted first.
///   3. Daemon down / daemon errored → propagate as Unavailable and
///      stop. Continuing to chat in this case would mask transport
///      failures.
///
/// A flat `Result<Option<Value>, ...>` collapses (2) and (3) into the
/// "Err" axis, which the chat-fall-through code can't distinguish
/// without string-matching the error message — fragile.
enum DaemonDispatch {
    Result(Value),
    AbilityNotFound,
    DaemonDown(String),
    Error(String),
}

/// Dispatch a fully-qualified `<agent>.<verb>` against the local
/// daemon's ability registry over the control socket. Returns one of
/// the four outcome variants the caller branches on.
///
/// Spins a single-threaded current-thread tokio runtime per call.
/// The cost is one runtime construction (~500 µs) plus the UDS
/// round-trip (~1 ms). EAL's other CLI subcommands (mission run,
/// agent send) follow the same pattern; if EAL ever runs inside an
/// already-tokio context the construction will fail and we'll need
/// to detect that — but today every EAL entry point is sync.
fn try_dispatch_via_daemon(qualified_name: &str, arguments: &Value) -> DaemonDispatch {
    let control_json = crate::services::control::discovery::default_path();
    if !control_json.exists() {
        return DaemonDispatch::DaemonDown(format!(
            "no control.json at {} — start the daemon with `easynet runtime start`",
            control_json.display()
        ));
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            return DaemonDispatch::Error(format!("build tokio runtime: {e}"));
        }
    };

    let ability = qualified_name.to_string();
    let args = arguments.clone();
    let request_id = format!("eal-{}", uuid_like_hex());

    let outcome: Result<Result<Value, DaemonDispatch>, String> = runtime.block_on(async move {
        use crate::services::control::frames::{IncomingFrame, OutgoingFrame};

        let mut client = match crate::ffi::client::connect(&control_json).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(Err(DaemonDispatch::DaemonDown(format!(
                    "connect control socket: {e}"
                ))));
            }
        };
        let resp = match client
            .round_trip(IncomingFrame::Invoke {
                request_id: request_id.clone(),
                ability,
                args,
                subject: None,
            })
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return Ok(Err(DaemonDispatch::Error(format!("round_trip: {e}"))));
            }
        };
        match resp {
            OutgoingFrame::Result {
                request_id: rid,
                value,
                ..
            } => {
                if rid != request_id {
                    return Ok(Err(DaemonDispatch::Error(format!(
                        "daemon Result rid mismatch (got {rid:?}, sent {request_id:?})"
                    ))));
                }
                Ok(Ok(value))
            }
            OutgoingFrame::Error { code, message, .. } => {
                // Map the typed code so the caller can route on intent
                // rather than string-matching. `not_found` is the
                // documented daemon code for "no handler for this
                // ability"; anything else is propagated verbatim.
                let lower = code.to_ascii_lowercase();
                if lower.contains("not_found") || message.contains("no local handler registered") {
                    Ok(Err(DaemonDispatch::AbilityNotFound))
                } else {
                    Ok(Err(DaemonDispatch::Error(format!(
                        "code={code}: {message}"
                    ))))
                }
            }
            other => Ok(Err(DaemonDispatch::Error(format!(
                "unexpected frame: {other:?}"
            )))),
        }
    });

    match outcome {
        Ok(Ok(value)) => DaemonDispatch::Result(value),
        Ok(Err(d)) => d,
        Err(e) => DaemonDispatch::Error(e),
    }
}

/// Short hex correlation id for the IPC `request_id`. Mirrors the
/// helper in `facade::cli::invoke` — kept local to avoid a cross-crate
/// dep just for a 5-line function.
fn uuid_like_hex() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}", nanos)
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
#[allow(dead_code)]
pub fn execute(
    bridge: &DendriteBridge,
    tenant: &str,
    ir: &MissionIr,
) -> anyhow::Result<ExecutionReport> {
    let dispatcher = BorrowedBridgeDispatcher::new(bridge);
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
    let dispatcher = AgentAwareDispatcher::new(
        endpoint,
        crate::support::timeouts::BRIDGE_CONNECT_TIMEOUT_MS,
    );
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
                    execute_calls_phase_partition(
                        dispatcher,
                        tenant,
                        &calls,
                        phase_idx,
                        &mut global_step,
                        total,
                        &mut captured,
                        &mut skipped_bindings,
                        &mut completed,
                        &mut failed,
                        &mut skipped,
                        &mut aborted,
                        &mut all_traces,
                    );
                }
                PhasePartition::Loop(lp) => {
                    execute_loop(
                        dispatcher,
                        tenant,
                        lp,
                        phase_idx,
                        &mut global_step,
                        total,
                        &mut captured,
                        &mut completed,
                        &mut failed,
                        &mut aborted,
                        &mut all_traces,
                    );
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
    /// Upstream binding was skipped, so this step cannot run. Emitted by
    /// `dispatch_batch` when `resolve_arguments` signals
    /// `ResolveError::UpstreamSkipped`. Classified as `StepOutcome::Skipped`
    /// in `process_step_result` regardless of this step's own
    /// `optional` / `on_failure` flags — propagating skip is the point.
    SkippedByDependency { message: String, started_at: u64 },
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
    skipped_bindings: &std::collections::HashSet<String>,
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
                    // `BorrowedBridgeDispatcher::clone_for_thread`
                    // returns `EalError::Internal` to *signal* "fall
                    // back to sequential" — but here, in the parallel
                    // path, hitting it means a structural setup error
                    // (a !Send dispatcher reached the parallel path).
                    // Render to display form (preserves error_code in
                    // the trace) and surface it as a step error.
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
            tasks.push((local_idx, thread_dispatcher, merged_args));
        }
        // Spawn rayon tasks — closure captures only Send types.
        rayon::scope(|scope| {
            for (local_idx, thread_dispatcher, merged_args) in tasks {
                let step = &steps[local_idx];
                let collector_ref = &collector;
                scope.spawn(move |_| {
                    let result = execute_step_with_retry(
                        thread_dispatcher.as_ref(),
                        tenant,
                        step,
                        &merged_args,
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
    skipped_bindings: &mut std::collections::HashSet<String>,
    completed: &mut usize,
    failed: &mut usize,
    skipped: &mut usize,
    aborted: &mut bool,
    all_traces: &mut CappedTraceBuffer,
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
                        captured.insert(binding.clone(), CapturedResult { value: bytes });
                    }
                }
            }
            StepOutcome::Failed => {
                *failed += 1;
                if !step.optional && matches!(step.on_failure, IrFailurePolicy::Abort) {
                    *aborted = true;
                }
            }
            StepOutcome::Skipped => {
                *skipped += 1;
                // Register the (un-)produced binding so every future
                // `resolve_arguments` call on a step consuming it
                // returns `ResolveError::UpstreamSkipped` and the
                // downstream step is classified Skipped too. Without
                // this registration, the downstream step would hit
                // the `unresolved ref` branch and get classified as
                // Failed — miscategorising "your producer didn't run"
                // as "you ran and failed".
                if let Some(ref binding) = step.output_binding {
                    skipped_bindings.insert(binding.clone());
                }
            }
        }
        all_traces.push(trace);
    }
}

/// Dispatch a contiguous run of `IrStep::Call` in one phase partition.
/// Extracted verbatim from the pre-PR-10 phase body so the parallel-
/// when-independent scheduling behaviour is unchanged for pure-Call
/// missions.
#[allow(clippy::too_many_arguments)]
fn execute_calls_phase_partition(
    dispatcher: &dyn StepDispatcher,
    tenant: &str,
    steps: &[IrCall],
    phase_idx: usize,
    global_step: &mut usize,
    total: usize,
    captured: &mut HashMap<String, CapturedResult>,
    skipped_bindings: &mut std::collections::HashSet<String>,
    completed: &mut usize,
    failed: &mut usize,
    skipped: &mut usize,
    aborted: &mut bool,
    all_traces: &mut CappedTraceBuffer,
) {
    if steps.is_empty() {
        return;
    }
    let wants_parallel = steps.len() > 1;
    let can_parallel = wants_parallel && dispatcher.clone_for_thread().is_ok();
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

    let required_results = dispatch_batch(
        dispatcher,
        tenant,
        steps,
        &required_indices,
        captured,
        skipped_bindings,
        can_parallel,
    );
    process_batch(
        steps,
        required_results,
        phase_idx,
        global_step,
        total,
        captured,
        skipped_bindings,
        completed,
        failed,
        skipped,
        aborted,
        all_traces,
    );

    if !*aborted && !optional_indices.is_empty() {
        let optional_results = dispatch_batch(
            dispatcher,
            tenant,
            steps,
            &optional_indices,
            captured,
            skipped_bindings,
            can_parallel,
        );
        process_batch(
            steps,
            optional_results,
            phase_idx,
            global_step,
            total,
            captured,
            skipped_bindings,
            completed,
            failed,
            skipped,
            aborted,
            all_traces,
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
#[allow(clippy::too_many_arguments)]
fn execute_loop(
    dispatcher: &dyn StepDispatcher,
    tenant: &str,
    lp: &IrLoop,
    phase_idx: usize,
    global_step: &mut usize,
    total: usize,
    outer_captured: &mut HashMap<String, CapturedResult>,
    completed: &mut usize,
    failed: &mut usize,
    aborted: &mut bool,
    all_traces: &mut CappedTraceBuffer,
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
            *failed += 1;
            *aborted = true;
            return;
        }
    };

    // `verify` must be non-empty (planner enforces) and its last leaf
    // call carries the termination predicate.
    let verify_last_idx = verify_calls.len() - 1;

    for iter in 1..=lp.max_iters {
        if *aborted {
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
            dispatcher,
            tenant,
            &body_calls,
            phase_idx,
            global_step,
            total,
            &mut iter_captured,
            &mut iter_skipped,
            completed,
            failed,
            aborted,
            all_traces,
        );
        if !body_ok || *aborted {
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
            dispatcher,
            tenant,
            &verify_calls,
            phase_idx,
            global_step,
            total,
            &mut iter_captured,
            &mut iter_skipped,
            completed,
            failed,
            aborted,
            all_traces,
        );
        if !verify_ok || *aborted {
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
        let verify_bytes: Option<&Vec<u8>> = final_call
            .output_binding
            .as_ref()
            .and_then(|b| iter_captured.get(b).map(|c| &c.value))
            .or_else(|| {
                iter_captured
                    .get(LOOP_VERIFY_SYNTHETIC_BINDING)
                    .map(|c| &c.value)
            });

        let verify_bytes = match verify_bytes {
            Some(b) => b,
            None => {
                eprintln!(
                    "  {}",
                    style(format!(
                        "loop '{label}' iter {iter}: VerifyMalformed — verify final call produced no output"
                    ))
                    .red()
                );
                *failed += 1;
                *aborted = true;
                return;
            }
        };

        let done = match verify_output_done(verify_bytes) {
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
                *failed += 1;
                *aborted = true;
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
                outer_captured.insert(
                    rb.clone(),
                    CapturedResult {
                        value: verify_bytes.clone(),
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
    *failed += 1;
    *aborted = true;
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
#[allow(clippy::too_many_arguments)]
fn run_loop_block_sequentially(
    dispatcher: &dyn StepDispatcher,
    tenant: &str,
    steps: &[IrCall],
    phase_idx: usize,
    global_step: &mut usize,
    total: usize,
    iter_captured: &mut HashMap<String, CapturedResult>,
    iter_skipped: &mut std::collections::HashSet<String>,
    completed: &mut usize,
    failed: &mut usize,
    aborted: &mut bool,
    all_traces: &mut CappedTraceBuffer,
) -> bool {
    let last_idx = steps.len().saturating_sub(1);
    for (i, step) in steps.iter().enumerate() {
        if *aborted {
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
                process_batch(
                    steps,
                    vec![(i, result)],
                    phase_idx,
                    global_step,
                    total,
                    iter_captured,
                    iter_skipped,
                    completed,
                    failed,
                    &mut local_skipped,
                    aborted,
                    all_traces,
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
                process_batch(
                    steps,
                    vec![(i, result)],
                    phase_idx,
                    global_step,
                    total,
                    iter_captured,
                    iter_skipped,
                    completed,
                    failed,
                    &mut local_skipped,
                    aborted,
                    all_traces,
                );
                return !*aborted;
            }
        };

        let result = execute_step_with_retry(dispatcher, tenant, step, &merged_args);

        // Mirror the "capture under synthetic binding for last step"
        // side-effect by copying result_bytes into iter_captured
        // before handing to process_batch (which would only capture
        // if output_binding is Some).
        if i == last_idx {
            if let StepExecResult::Ok { result_bytes, .. } = &result {
                iter_captured.insert(
                    LOOP_VERIFY_SYNTHETIC_BINDING.to_string(),
                    CapturedResult {
                        value: result_bytes.clone(),
                    },
                );
            }
        }

        let mut local_skipped = 0usize;
        process_batch(
            steps,
            vec![(i, result)],
            phase_idx,
            global_step,
            total,
            iter_captured,
            iter_skipped,
            completed,
            failed,
            &mut local_skipped,
            aborted,
            all_traces,
        );
    }
    !*aborted
}

/// Typed outcome of inspecting the verify block's final call output
/// for RFC §4.4's `done: bool` predicate. `Malformed` carries a
/// reason string used as the `VerifyMalformed` abort message.
enum VerifyDone {
    True,
    False,
    Malformed(String),
}

fn verify_output_done(bytes: &[u8]) -> VerifyDone {
    let v: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => {
            return VerifyDone::Malformed(format!("verify output is not JSON-decodable ({e})"));
        }
    };
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
enum ResolveError {
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

fn resolve_arguments(
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
                input_refs: step.input_refs.clone(),
                output_binding: step.output_binding.clone(),
            };
            (StepOutcome::Skipped, trace, None)
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
    use crate::registry::agents::{AgentEntry, AgentRegistry, AgentType};
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

    fn dummy_agent_entry() -> AgentEntry {
        let mut entry = AgentEntry::new(AgentType::ClaudeCode, None);
        entry.command = "easynet-test-nonexistent-agent-binary".to_string();
        entry.timeout_secs = 1;
        entry
    }

    impl StepDispatcher for MockDispatcher {
        fn dispatch(
            &self,
            _tenant: &str,
            _target: &IrTarget,
            ability: &AbilityName,
            _arguments: &Value,
            _timeout_ms: Option<u64>,
        ) -> Result<Value, EalError> {
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
                return Err(EalError::Unavailable(format!(
                    "simulated failure for {ability_str}"
                )));
            }

            // Fail first N calls (order-dependent — use only in sequential phases)
            let fail_n = self.fail_first_n.load(Ordering::SeqCst);
            if call_num < fail_n {
                return Err(EalError::Unavailable(format!(
                    "simulated failure #{call_num}"
                )));
            }

            Ok(serde_json::json!({
                "ok": true,
                "call_num": call_num,
                "function": ability_str,
            }))
        }

        fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
            Ok(Box::new(MockDispatcher {
                delay_ms: self.delay_ms,
                call_count: Arc::clone(&self.call_count),
                fail_first_n: Arc::clone(&self.fail_first_n),
                fail_functions: Arc::clone(&self.fail_functions),
                calls: Arc::clone(&self.calls),
            }))
        }
    }

    // ── CappedTraceBuffer: bounded memory invariant ──

    /// Small helper: build a synthetic `StepTrace` with the given id,
    /// so buffer tests don't depend on executing a real mission.
    fn synth_trace(id: &str) -> StepTrace {
        StepTrace {
            step_id: id.to_string(),
            ability: crate::core::agent_id::AbilityName::parse("t").expect("valid ability name"),
            target: crate::eal::ir::IrTarget::Device {
                node_id: "n".to_string(),
            },
            phase_index: 0,
            started_at_unix_ms: 0,
            completed_at_unix_ms: 0,
            elapsed_ms: 0,
            outcome: StepOutcome::Completed,
            retry_count: 0,
            retry_history: vec![],
            result_size_bytes: None,
            result_sha256: None,
            error: None,
            input_refs: BTreeMap::new(),
            output_binding: None,
        }
    }

    #[test]
    fn capped_trace_buffer_under_head_cap_keeps_everything() {
        let mut buf = CappedTraceBuffer::new();
        for i in 0..10 {
            buf.push(synth_trace(&format!("s{i}")));
        }
        assert_eq!(buf.len(), 10);
        let (entries, dropped) = buf.into_parts();
        assert_eq!(dropped, 0);
        assert_eq!(entries.len(), 10);
        // Order preserved — this is the forensics contract.
        for (i, e) in entries.iter().enumerate() {
            assert_eq!(e.step_id, format!("s{i}"));
        }
    }

    #[test]
    fn capped_trace_buffer_head_boundary_saturates_exactly() {
        // Pushing exactly TRACE_CAP_HEAD entries must fill the head
        // and leave the tail empty — no entries dropped, no tail use.
        let mut buf = CappedTraceBuffer::new();
        for i in 0..TRACE_CAP_HEAD {
            buf.push(synth_trace(&format!("s{i}")));
        }
        let (entries, dropped) = buf.into_parts();
        assert_eq!(dropped, 0);
        assert_eq!(entries.len(), TRACE_CAP_HEAD);
    }

    #[test]
    fn capped_trace_buffer_between_head_and_cap_uses_tail() {
        // Head + part of tail, but within TRACE_CAP_TOTAL — nothing
        // should be dropped.
        let n = TRACE_CAP_HEAD + 50;
        let mut buf = CappedTraceBuffer::new();
        for i in 0..n {
            buf.push(synth_trace(&format!("s{i}")));
        }
        let (entries, dropped) = buf.into_parts();
        assert_eq!(dropped, 0);
        assert_eq!(entries.len(), n);
    }

    #[test]
    fn capped_trace_buffer_over_cap_drops_middle_with_count() {
        // Push twice TRACE_CAP_TOTAL. Expect: head preserved, tail
        // holds the most recent TRACE_CAP_TAIL entries, middle slab
        // counted as dropped.
        let n = TRACE_CAP_HEAD + TRACE_CAP_TAIL + 250;
        let expected_dropped = n - (TRACE_CAP_HEAD + TRACE_CAP_TAIL);
        let mut buf = CappedTraceBuffer::new();
        for i in 0..n {
            buf.push(synth_trace(&format!("s{i}")));
        }
        let (entries, dropped) = buf.into_parts();
        assert_eq!(dropped, expected_dropped);
        assert_eq!(entries.len(), TRACE_CAP_HEAD + TRACE_CAP_TAIL);

        // Head first TRACE_CAP_HEAD entries are s0..s{HEAD-1}.
        for (i, e) in entries.iter().take(TRACE_CAP_HEAD).enumerate() {
            assert_eq!(e.step_id, format!("s{i}"));
        }
        // Tail last TRACE_CAP_TAIL entries are s{n-TAIL}..s{n-1}.
        let tail_slice = &entries[TRACE_CAP_HEAD..];
        for (offset, e) in tail_slice.iter().enumerate() {
            let expected_idx = n - TRACE_CAP_TAIL + offset;
            assert_eq!(e.step_id, format!("s{expected_idx}"));
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
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();
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
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

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
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();
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
            IrTarget::Device {
                node_id: "gpu".to_string()
            }
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

    #[test]
    fn dispatch_to_agent_chat_stays_local_when_daemon_is_absent() {
        // Regression pin for `easynet agent send`: chat must not go
        // through the daemon's unary control-socket invoke, because
        // that hides the driver's live stderr timeline inside the
        // daemon process. The local path fails here on the bogus
        // binary name, not on missing control.json.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut registry = AgentRegistry::default();
        registry
            .agents
            .insert("alice".to_string(), dummy_agent_entry());

        let agent_id = crate::core::agent_id::AgentId::parse("alice").expect("valid agent id");
        let ability = AbilityName::parse(crate::runtime::agents::chat_ability::ABILITY_VERB)
            .expect("valid chat ability");
        let err = dispatch_to_agent(
            &registry,
            &agent_id,
            &ability,
            &serde_json::json!({"prompt": "hi"}),
        )
        .expect_err("bogus binary must fail on local chat dispatch");
        let msg = format!("{err}");

        assert!(
            msg.contains("easynet-test-nonexistent-agent-binary"),
            "expected local driver spawn failure, got: {msg}"
        );
        assert!(
            !msg.contains("control.json") && !msg.contains("daemon:"),
            "chat dispatch must not depend on daemon unary invoke, got: {msg}"
        );
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
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

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
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

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
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

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
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(report.steps_completed, 1, "must-run should succeed");
        assert_eq!(
            report.trace.steps_skipped, 1,
            "optional failure should be skipped"
        );
        assert_eq!(report.trace.outcome, MissionOutcome::Completed);
    }

    /// Regression: when an optional step is skipped and a downstream
    /// step consumes its output, the downstream must propagate as
    /// `Skipped` (not `Failed`). Previously the missing binding hit
    /// the `unresolved ref` branch and the consumer was classified
    /// `Failed`, which miscategorised "my producer didn't run" as
    /// "I ran and failed" in the trace — confusing operators reading
    /// the audit log.
    #[test]
    fn downstream_is_auto_skipped_when_its_producer_is_skipped() {
        let src = r#"
            mission "cascade-skip" {
                let p = call "producer" on "n" optional
                let c = call "consumer" on "n" with { input = p.output }
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();

        // `producer` fails (and is optional → Skipped), so `consumer`
        // has no `p` binding to read. With the cascade-skip fix, the
        // consumer sees `ResolveError::UpstreamSkipped` and is
        // classified as Skipped too.
        let dispatcher = MockDispatcher::new(0).with_fail_functions(&["producer"]);
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();

        assert_eq!(
            report.trace.steps_skipped, 2,
            "both the optional producer and the dependent consumer must skip; got trace: {:?}",
            report.trace.step_traces
        );
        assert_eq!(report.steps_completed, 0);
        assert_eq!(report.steps_failed, 0);

        // Consumer trace carries the provenance in its error message.
        let consumer = report
            .trace
            .step_traces
            .iter()
            .find(|t| t.ability.as_str() == "consumer")
            .expect("consumer trace must be present");
        assert_eq!(consumer.outcome, StepOutcome::Skipped);
        let err = consumer
            .error
            .as_deref()
            .expect("cascaded skip must carry a provenance message");
        assert!(
            err.contains("`p`"),
            "cascaded-skip message must name the missing upstream binding; got: {err}"
        );
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
        let report = execute_with_dispatcher(&dispatcher, "test", &ir).unwrap();
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
    //
    // These tests pin three properties the retry scheduler relies on:
    //
    //   1. **Exponential growth** (attempt N doubles attempt N-1's base).
    //   2. **Upper bound** (capped base + bounded jitter; never unbounded).
    //   3. **Determinism** (same `(attempt, step_id)` → same delay across
    //      runs, threads, and processes). Determinism lets replay-based
    //      trace comparison (two runs of the same mission) line up
    //      exactly, which is the whole point of the deterministic jitter.
    //
    // Jitter bound: `jitter_seed % (RETRY_BASE_MS / 2 + 1)` → jitter is
    // in `0..=500` ms. Asserting the *strict* upper bound (not just
    // "capped + BASE") is what turns "works today" into "contract".

    #[test]
    fn backoff_is_exponential_and_deterministic() {
        let b1 = compute_backoff(1, "step-a");
        let b2 = compute_backoff(2, "step-a");
        let b3 = compute_backoff(3, "step-a");

        // Base: 1000 ms. Attempt N adds `BASE * 2^(N-1)` (capped at MAX)
        // plus a jitter of `0..=BASE/2`. The lower bound is the pure
        // exponential; the upper bound is cap + jitter_max.
        let jitter_max = RETRY_BASE_MS / 2;
        assert!(b1 >= RETRY_BASE_MS && b1 <= RETRY_BASE_MS + jitter_max);
        assert!(b2 >= RETRY_BASE_MS * 2 && b2 <= RETRY_BASE_MS * 2 + jitter_max);
        assert!(b3 >= RETRY_BASE_MS * 4 && b3 <= RETRY_BASE_MS * 4 + jitter_max);

        // Determinism: same inputs → same output, same process OR fresh.
        assert_eq!(b1, compute_backoff(1, "step-a"));
        assert_eq!(b2, compute_backoff(2, "step-a"));
        assert_eq!(b3, compute_backoff(3, "step-a"));

        // Different step_id → independent jitter (but still in range).
        let b1_other = compute_backoff(1, "step-b");
        assert!(b1_other >= RETRY_BASE_MS && b1_other <= RETRY_BASE_MS + jitter_max);
    }

    /// Capping behaviour: beyond the saturation attempt, `base` is
    /// clamped at `RETRY_MAX_MS` and the only variation comes from
    /// jitter. A future refactor that removed the `min(MAX)` would
    /// send delays into the stratosphere and fail this test.
    #[test]
    fn backoff_caps_base_at_retry_max_ms() {
        let jitter_max = RETRY_BASE_MS / 2;
        // attempt=10 → raw base = 1000 * 2^9 = 512_000, well past MAX=30_000
        let capped = compute_backoff(10, "saturating-step");
        assert!(
            capped >= RETRY_MAX_MS && capped <= RETRY_MAX_MS + jitter_max,
            "attempt=10 must saturate at RETRY_MAX_MS (~{RETRY_MAX_MS}); got {capped}"
        );
    }

    /// Cross-step independence: two different step ids at the same
    /// attempt number must (with overwhelming probability) yield
    /// different jitter values. Asserting *any difference* across a
    /// small corpus is a cheap way to catch a regression that silently
    /// collapsed jitter to a constant (e.g. forgot to mix in step_id).
    #[test]
    fn backoff_jitter_varies_across_step_ids() {
        let values: std::collections::HashSet<u64> = (0..8)
            .map(|i| compute_backoff(1, &format!("s{i}")))
            .collect();
        assert!(
            values.len() > 1,
            "jitter collapsed to a constant across step ids — SHA256 seed broken?"
        );
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
            ) -> Result<Value, EalError> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Ok(serde_json::json!({"ok": true, "function": ability.as_str()}))
            }
            fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
                Err(EalError::Internal("not cloneable".into()))
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
            Self {
                seen: Arc::new(Mutex::new(Vec::new())),
            }
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
        ) -> Result<Value, EalError> {
            self.seen
                .lock()
                .unwrap()
                .push((target.clone(), ability.clone(), arguments.clone()));
            Ok(serde_json::json!({"ok": true}))
        }

        fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
            Ok(Box::new(ShapeRecordingDispatcher {
                seen: Arc::clone(&self.seen),
            }))
        }
    }

    /// Regression: when a dispatcher returns a categorised `EalError`,
    /// the `error_code:` prefix must survive the boundary into the
    /// trace and retry log. Operators reading a trace file should be
    /// able to grep for `validation_error:` / `not_found:` /
    /// `unavailable:` / `internal_error:` without needing the typed
    /// error available — that is the whole point of using `Display`
    /// (rather than just `.message()`) at the boundary.
    #[test]
    fn dispatcher_error_code_is_preserved_in_trace_message() {
        struct CategorisedDispatcher;
        impl StepDispatcher for CategorisedDispatcher {
            fn dispatch(
                &self,
                _tenant: &str,
                _target: &IrTarget,
                _ability: &AbilityName,
                _arguments: &Value,
                _timeout_ms: Option<u64>,
            ) -> Result<Value, EalError> {
                Err(EalError::NotFound("device 'node-x' not registered".into()))
            }
            fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
                Ok(Box::new(CategorisedDispatcher))
            }
        }

        let src = r#"mission "t" { let r = call "ping" on "node-x" }"#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        let report = execute_with_dispatcher(&CategorisedDispatcher, "tenant", &ir).unwrap();

        let trace = &report.trace.step_traces[0];
        let err_msg = trace.error.as_deref().expect("step must have an error");
        assert!(
            err_msg.starts_with("not_found:"),
            "trace error must start with the EalError code prefix; got: {err_msg}"
        );
        assert!(
            err_msg.contains("device 'node-x' not registered"),
            "trace error must include the human message; got: {err_msg}"
        );
    }

    /// Golden test: pin the on-disk JSON shape of `ExecutionTrace` v1.
    ///
    /// This test is the contract between this module and any external
    /// consumer that reads trace files (CI scrapers, external auditors,
    /// the future trace-replay UI). Adding a field with
    /// `#[serde(default)]` keeps this test green and is a backwards-
    /// compatible change. *Renaming* a field, removing one, or changing
    /// a numeric type fails this test — at which point the codebase is
    /// telling you to bump `EXECUTION_TRACE_SCHEMA_VERSION` and write a
    /// reader-side migration.
    ///
    /// We assert two properties:
    ///   1. A freshly constructed trace serializes with
    ///      `schema_version = EXECUTION_TRACE_SCHEMA_VERSION` and the
    ///      full set of expected top-level keys.
    ///   2. A *legacy* JSON payload (no `schema_version` field) round-
    ///      trips through deserialization and lands at version 1 — the
    ///      tolerant-read promise documented on the constant.
    #[test]
    fn trace_schema_v1_is_stable() {
        // Use a hand-built trace rather than running a real mission so
        // the expected JSON is fully deterministic — no timestamps to
        // freeze, no SHA digests to mock.
        let trace = ExecutionTrace {
            schema_version: EXECUTION_TRACE_SCHEMA_VERSION,
            mission_id: "m-test".to_string(),
            mission_name: "test-mission".to_string(),
            started_at_unix_ms: 1_000,
            completed_at_unix_ms: 2_000,
            total_elapsed_ms: 1_000,
            phase_count: 1,
            steps_completed: 0,
            steps_failed: 0,
            steps_skipped: 0,
            outcome: MissionOutcome::Completed,
            step_traces: vec![],
            traces_truncated: 0,
        };

        let json: serde_json::Value =
            serde_json::to_value(&trace).expect("trace must serialize cleanly");

        // Property 1: version is stamped and the key set is fixed.
        assert_eq!(json["schema_version"], serde_json::json!(1));
        let expected_keys: std::collections::BTreeSet<&str> = [
            "schema_version",
            "mission_id",
            "mission_name",
            "started_at_unix_ms",
            "completed_at_unix_ms",
            "total_elapsed_ms",
            "phase_count",
            "steps_completed",
            "steps_failed",
            "steps_skipped",
            "outcome",
            "step_traces",
            // `traces_truncated` is a v1-compatible additive field: it
            // serializes as `0` for missions under the cap and older
            // readers ignore unknown keys. Its presence here pins that
            // the on-the-wire shape includes it for every fresh trace;
            // the legacy-deserialize property below confirms old
            // payloads without this key still parse.
            "traces_truncated",
        ]
        .into_iter()
        .collect();
        let actual_keys: std::collections::BTreeSet<&str> = json
            .as_object()
            .expect("trace serializes to an object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            actual_keys, expected_keys,
            "ExecutionTrace key set drift detected. \
             If this is intentional, bump EXECUTION_TRACE_SCHEMA_VERSION \
             and update this test."
        );

        // Property 2: legacy payloads (no `schema_version`) read back
        // as version 1 — the tolerant-read promise. A reader who pulls
        // a pre-stamp trace file off disk must not get a deser error.
        let legacy_json = serde_json::json!({
            "mission_id": "legacy",
            "mission_name": "legacy",
            "started_at_unix_ms": 0,
            "completed_at_unix_ms": 0,
            "total_elapsed_ms": 0,
            "phase_count": 0,
            "steps_completed": 0,
            "steps_failed": 0,
            "steps_skipped": 0,
            "outcome": "completed",
            "step_traces": [],
        });
        let legacy: ExecutionTrace = serde_json::from_value(legacy_json)
            .expect("pre-stamp trace JSON must deserialize via #[serde(default)]");
        assert_eq!(
            legacy.schema_version, 1,
            "pre-stamp traces must read back as v1 — bump current_trace_schema_version() if v2+ landed"
        );
    }

    /// Regression: a malformed upstream payload must fail the
    /// consuming step at `resolve_arguments`, not silently inject
    /// `null` and let the downstream step run with corrupt input. The
    /// previous `unwrap_or(Value::Null)` made this class of bug a
    /// debugging black hole — see commit history for the original
    /// motivation.
    #[test]
    fn resolve_arguments_fails_loud_on_malformed_upstream_payload() {
        use crate::core::agent_id::{AbilityName, AgentId};
        use std::collections::BTreeMap;

        let mut input_refs = BTreeMap::new();
        input_refs.insert("input".to_string(), "upstream".to_string());

        let step = IrStep {
            step_id: "consumer".to_string(),
            step_name: "consumer".to_string(),
            ability: AbilityName::parse("review").unwrap(),
            target: IrTarget::Agent(AgentId::parse("claude").unwrap()),
            static_arguments: serde_json::json!({}),
            input_refs,
            output_binding: None,
            timeout_seconds: 0,
            max_retries: 0,
            on_failure: IrFailurePolicy::Continue,
            optional: false,
            content_type: "application/json".to_string(),
        };

        // "{not json" is exactly the sort of partial / corrupted output
        // that motivated this guard — a streaming ability that died
        // mid-flush can leave bytes like this in the captured slot.
        let mut results: HashMap<String, CapturedResult> = HashMap::new();
        results.insert(
            "upstream".to_string(),
            CapturedResult {
                value: b"{not json".to_vec(),
            },
        );

        let skipped: std::collections::HashSet<String> = std::collections::HashSet::new();
        let err = resolve_arguments(&step, &results, &skipped)
            .expect_err("malformed upstream payload must surface as step error");
        let msg = err.to_string();
        // The malformed-payload path is `ResolveError::Other` (not
        // UpstreamSkipped) — the binding *was* produced, we just
        // couldn't parse the bytes.
        assert!(
            matches!(err, ResolveError::Other(_)),
            "malformed payload is a generic resolve error, not UpstreamSkipped; got: {err:?}"
        );
        assert!(
            msg.contains("input ref `input`"),
            "error must name the consuming arg name; got: {msg}"
        );
        assert!(
            msg.contains("binding `upstream`"),
            "error must name the upstream binding; got: {msg}"
        );
        assert!(
            msg.to_lowercase().contains("not valid json"),
            "error must explain the failure category; got: {msg}"
        );
    }

    /// Regression: when an upstream step is skipped, a consumer's
    /// `resolve_arguments` must return the typed `UpstreamSkipped`
    /// variant (not the generic `Other`), so the caller can propagate
    /// `Skipped` instead of miscategorising as `Failed`. This is the
    /// contract that prevents the "optional producer → required
    /// consumer" trace from looking like a cascade of failures.
    #[test]
    fn resolve_arguments_returns_upstream_skipped_for_skipped_binding() {
        use crate::core::agent_id::{AbilityName, AgentId};
        use std::collections::BTreeMap;

        let mut input_refs = BTreeMap::new();
        input_refs.insert("input".to_string(), "producer".to_string());

        let step = IrStep {
            step_id: "consumer".to_string(),
            step_name: "consumer".to_string(),
            ability: AbilityName::parse("review").unwrap(),
            target: IrTarget::Agent(AgentId::parse("claude").unwrap()),
            static_arguments: serde_json::json!({}),
            input_refs,
            output_binding: None,
            timeout_seconds: 0,
            max_retries: 0,
            on_failure: IrFailurePolicy::Continue,
            optional: false,
            content_type: "application/json".to_string(),
        };

        let results: HashMap<String, CapturedResult> = HashMap::new();
        let mut skipped = std::collections::HashSet::new();
        skipped.insert("producer".to_string());

        let err = resolve_arguments(&step, &results, &skipped)
            .expect_err("skipped upstream must surface as typed skip");
        match err {
            ResolveError::UpstreamSkipped { binding, arg } => {
                assert_eq!(binding, "producer");
                assert_eq!(arg, "input");
            }
            other => panic!("expected UpstreamSkipped, got: {other:?}"),
        }
    }

    /// Counterpart: well-formed payloads still flow through cleanly.
    /// Pinned alongside the failure case so a future refactor that
    /// over-tightens the parser (e.g. requires top-level objects) is
    /// caught immediately.
    #[test]
    fn resolve_arguments_threads_well_formed_payload() {
        use crate::core::agent_id::{AbilityName, AgentId};
        use std::collections::BTreeMap;

        let mut input_refs = BTreeMap::new();
        input_refs.insert("input".to_string(), "upstream".to_string());

        let step = IrStep {
            step_id: "consumer".to_string(),
            step_name: "consumer".to_string(),
            ability: AbilityName::parse("review").unwrap(),
            target: IrTarget::Agent(AgentId::parse("claude").unwrap()),
            static_arguments: serde_json::json!({"k": "static"}),
            input_refs,
            output_binding: None,
            timeout_seconds: 0,
            max_retries: 0,
            on_failure: IrFailurePolicy::Continue,
            optional: false,
            content_type: "application/json".to_string(),
        };

        let mut results: HashMap<String, CapturedResult> = HashMap::new();
        results.insert(
            "upstream".to_string(),
            CapturedResult {
                value: b"{\"answer\": 42}".to_vec(),
            },
        );

        let skipped: std::collections::HashSet<String> = std::collections::HashSet::new();
        let resolved =
            resolve_arguments(&step, &results, &skipped).expect("well-formed payload must parse");
        let obj = resolved.as_object().expect("resolved args are an object");
        assert_eq!(obj.get("k"), Some(&serde_json::json!("static")));
        assert_eq!(obj.get("input"), Some(&serde_json::json!({"answer": 42})));
    }

    #[test]
    fn member_call_lowers_to_agent_target() {
        use crate::core::agent_id::AgentId;

        let src = r#"
            mission "member-call" {
                let r = claude.chat(prompt: "hi")
            }
        "#;
        let ir = planner::compile(&parser::parse(src).unwrap()).unwrap();
        assert_eq!(ir.steps.len(), 1);
        let call = ir.steps[0].as_call().expect("flat call step");
        assert_eq!(call.ability.as_str(), "chat");
        assert_eq!(
            call.target,
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
        let call = ir.steps[0].as_call().expect("flat call step");
        assert_eq!(call.ability.as_str(), "chat");
        assert_eq!(
            call.target,
            IrTarget::Device {
                node_id: "node-1".to_string()
            },
            "traditional `call ... on ...` must lower to IrTarget::Device"
        );
    }

    #[test]
    fn member_call_dispatches_to_agent_via_recorder() {
        // The interpreter dispatch path receives the resolved
        // IrTarget::Agent — no string-based classification along the way.
        use crate::core::agent_id::AgentId;

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
        assert_eq!(seen[0].2.get("prompt").and_then(|v| v.as_str()), Some("hi"));
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
            IrTarget::Device {
                node_id: "node-1".to_string()
            }
        );
        assert_eq!(seen[0].1.as_str(), "chat");
    }

    // ── PR-10 Stage 3: loop executor audit hooks ──────────────────────────
    //
    // These cover the RFC §4 / §5 behaviours the planner tests
    // cannot: runtime termination, typed error surfaces, depth
    // non-nesting, and the `<name>.result` export contract.

    /// Programmable dispatcher: returns canned JSON Values per call,
    /// indexed by a per-ability counter so tests can script the
    /// sequence of verify outputs across iterations.
    struct ScriptedDispatcher {
        outputs: Arc<Mutex<std::collections::HashMap<String, Vec<Value>>>>,
        cursor: Arc<Mutex<std::collections::HashMap<String, usize>>>,
        default: Value,
        calls: Arc<Mutex<Vec<String>>>,
        /// If set, each dispatch reads `EASYNET_AGENT_DEPTH` and
        /// records the observed depth value. Used by the
        /// non-nesting-depth test.
        depth_observations: Arc<Mutex<Vec<Option<String>>>>,
    }

    impl ScriptedDispatcher {
        fn new(default: Value) -> Self {
            Self {
                outputs: Arc::new(Mutex::new(std::collections::HashMap::new())),
                cursor: Arc::new(Mutex::new(std::collections::HashMap::new())),
                default,
                calls: Arc::new(Mutex::new(Vec::new())),
                depth_observations: Arc::new(Mutex::new(Vec::new())),
            }
        }
        fn with_script(self, ability: &str, script: Vec<Value>) -> Self {
            self.outputs
                .lock()
                .unwrap()
                .insert(ability.to_string(), script);
            self
        }
    }

    impl StepDispatcher for ScriptedDispatcher {
        fn dispatch(
            &self,
            _tenant: &str,
            _target: &IrTarget,
            ability: &AbilityName,
            _arguments: &Value,
            _timeout_ms: Option<u64>,
        ) -> Result<Value, EalError> {
            let k = ability.as_str().to_string();
            self.calls.lock().unwrap().push(k.clone());
            self.depth_observations
                .lock()
                .unwrap()
                .push(std::env::var("EASYNET_AGENT_DEPTH").ok());
            let mut cursors = self.cursor.lock().unwrap();
            let cur = cursors.entry(k.clone()).or_insert(0);
            let outs = self.outputs.lock().unwrap();
            if let Some(script) = outs.get(&k) {
                if *cur < script.len() {
                    let v = script[*cur].clone();
                    *cur += 1;
                    return Ok(v);
                }
            }
            Ok(self.default.clone())
        }
        fn clone_for_thread(&self) -> Result<Box<dyn StepDispatcher + Send>, EalError> {
            // Loops are sequential by design — no thread cloning needed
            // for these tests. Signal "fall back to sequential" via Err.
            Err(EalError::Internal(
                "scripted dispatcher is single-thread".into(),
            ))
        }
    }

    /// RFC §4.4 happy path: verify returns `done: true` on iter K;
    /// the loop terminates successfully, and `<name>.result` binds
    /// the verify final call's output.
    #[test]
    fn loop_terminates_on_done_true_and_binds_result() {
        let src = r#"
            mission "t" {
                loop "review" max_iters: 4 {
                    body { a.step(p: "x") }
                    verify { a.ok(p: "x") }
                }
            }"#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        // Verify script: iter 1 → done:false; iter 2 → done:true.
        let d = ScriptedDispatcher::new(serde_json::json!({"done": false})).with_script(
            "ok",
            vec![
                serde_json::json!({"done": false}),
                serde_json::json!({"done": true, "payload": "winner"}),
            ],
        );
        let report = execute_with_dispatcher(&d, "test", &ir).unwrap();
        assert_eq!(report.steps_failed, 0);
        // Body + verify × 2 iterations = 4 calls total.
        assert_eq!(d.calls.lock().unwrap().len(), 4);
        // `<name>.result` export: captured as review.result, verify
        // final output on the winning iteration.
        let winner = report
            .outputs
            .get("review.result")
            .expect("review.result must be exported on winning iter");
        assert!(
            winner.contains("winner"),
            "result must carry verify final output; got: {winner}"
        );
    }

    /// RFC §5.2: `LoopExhausted` — max_iters reached without
    /// done:true. Mission outcome is Aborted; error surface cites
    /// "LoopExhausted" and "max_iters".
    #[test]
    fn loop_exhausts_with_typed_error() {
        let src = r#"
            mission "t" {
                loop "x" max_iters: 3 {
                    body { a.step(p: "x") }
                    verify { a.ok(p: "x") }
                }
            }"#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        // Never done.
        let d = ScriptedDispatcher::new(serde_json::json!({"done": false}));
        let report = execute_with_dispatcher(&d, "test", &ir).unwrap();
        assert_eq!(report.trace.outcome, MissionOutcome::Aborted);
        assert!(report.steps_failed >= 1);
        // 3 iters × (body + verify) = 6 calls.
        assert_eq!(d.calls.lock().unwrap().len(), 6);
    }

    /// RFC §4.4 / §5.2: `VerifyMalformed` — verify output missing
    /// `done` field. Mission aborts at iter 1.
    #[test]
    fn verify_without_done_field_aborts() {
        let src = r#"
            mission "t" {
                loop max_iters: 3 {
                    body { a.step(p: "x") }
                    verify { a.ok(p: "x") }
                }
            }"#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        // Verify returns an object with NO `done` field.
        let d = ScriptedDispatcher::new(serde_json::json!({"ok": true}));
        let report = execute_with_dispatcher(&d, "test", &ir).unwrap();
        assert_eq!(report.trace.outcome, MissionOutcome::Aborted);
        // Stopped at iter 1 (body + verify).
        assert_eq!(d.calls.lock().unwrap().len(), 2);
    }

    /// RFC §4.4 / §5.2: non-boolean `done` is also VerifyMalformed.
    /// Pins the strict-bool contract — a string "true" does NOT
    /// count, to stop authors from shipping prose-predicate verify.
    #[test]
    fn verify_with_non_bool_done_aborts() {
        let src = r#"
            mission "t" {
                loop max_iters: 2 {
                    body { a.step(p: "x") }
                    verify { a.ok(p: "x") }
                }
            }"#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        let d = ScriptedDispatcher::new(serde_json::json!({"done": "yes"}));
        let report = execute_with_dispatcher(&d, "test", &ir).unwrap();
        assert_eq!(report.trace.outcome, MissionOutcome::Aborted);
    }

    /// RFC §4.2 / §4.3: loop iterations do NOT stack agent-dispatch
    /// depth. A 4-iter loop dispatching once per body and once per
    /// verify runs 8 total dispatches, but each stays at the same
    /// `EASYNET_AGENT_DEPTH` value the mission was invoked at — it
    /// does not climb with iteration count. The dispatcher records
    /// the env var each call; all values must be equal.
    #[test]
    fn loop_with_body_dispatch_does_not_nest_depth() {
        let src = r#"
            mission "t" {
                loop max_iters: 4 {
                    body { a.step(p: "x") }
                    verify { a.ok(p: "x") }
                }
            }"#;
        let prog = parser::parse(src).unwrap();
        let ir = planner::compile(&prog).unwrap();
        let d = ScriptedDispatcher::new(serde_json::json!({"done": false}));
        let _ = execute_with_dispatcher(&d, "test", &ir).unwrap();
        let obs = d.depth_observations.lock().unwrap();
        // 4 iters * (body + verify) = 8 dispatches.
        assert_eq!(obs.len(), 8);
        // Every observation is identical — no climbing depth.
        let first = obs.first().cloned().flatten();
        for (i, v) in obs.iter().enumerate() {
            assert_eq!(
                v.clone(),
                first.clone(),
                "iter-dispatch {i} observed depth {v:?}, differs from first {first:?}"
            );
        }
    }
}
