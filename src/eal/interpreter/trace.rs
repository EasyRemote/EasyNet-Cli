// Mission execution trace/report read model (split from
// interpreter.rs, T4.4 / F-021; bodies are move-only).

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

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

fn current_trace_schema_version() -> u32 {
    EXECUTION_TRACE_SCHEMA_VERSION
}

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
pub(super) struct CappedTraceBuffer {
    head: Vec<StepTrace>,
    tail: std::collections::VecDeque<StepTrace>,
    dropped: usize,
}

impl CappedTraceBuffer {
    pub(super) fn new() -> Self {
        debug_assert_eq!(
            TRACE_CAP_TOTAL,
            TRACE_CAP_HEAD + TRACE_CAP_TAIL,
            "TRACE_CAP_TOTAL must remain the documented retained-trace ceiling"
        );
        Self {
            head: Vec::with_capacity(TRACE_CAP_HEAD.min(64)),
            tail: std::collections::VecDeque::with_capacity(TRACE_CAP_TAIL.min(64)),
            dropped: 0,
        }
    }

    pub(super) fn push(&mut self, t: StepTrace) {
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
    pub(super) fn into_parts(self) -> (Vec<StepTrace>, usize) {
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
    pub(super) fn len(&self) -> usize {
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
    /// Per-step seven-tuple invocation records (envelope echo +
    /// ledger receipt anchors) for steps lowered onto the daemon
    /// Invocation surface, in execution order. The receipt-level
    /// ability graph: nodes are the records' `ability` names, edges
    /// come from each record's `causal_context.parents`. Empty when
    /// no step produced an invocation record (offline run).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ability_graph: Vec<Value>,
    /// Ordered archival records produced by EAL `emit` statements.
    /// These are mission-local outputs for downstream answer/report
    /// stages. They are not child invocations and carry no receipts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emissions: Vec<EmissionRecord>,
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
    /// Seven-tuple Axon invocation record for this step when it was
    /// lowered onto the daemon Invocation surface: envelope echo
    /// (caller/callee/ability/subject/nonce/causal_context) plus the
    /// ledger-assigned invocation_ura, trace_id, and receipt anchors.
    /// None for receipt-less dispatch paths (in-process fallback,
    /// agent CLI) — absence is recorded, never fabricated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation: Option<Value>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmissionRecord {
    pub seq: usize,
    pub name: String,
    pub kind: String,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_binding: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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

pub(super) struct CapturedResult {
    pub(super) value: Vec<u8>,
    /// Seven-tuple invocation record for the step that produced this
    /// binding, when the step was lowered onto the daemon's Axon
    /// Invocation surface (None for in-process fallback dispatch).
    /// Downstream steps read this to name their causal parents.
    pub(super) invocation: Option<Value>,
}
