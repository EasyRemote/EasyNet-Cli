// EasyNet CLI — Agent Adapter Trait
// =================================
//
// File: src/runtime/adapter.rs
// Description: The `AgentAdapter` trait. A single synchronous surface
//              every runtime driver exposes to `dispatch::send_to_agent`.
//
// Why sync?
//   The three call sites that eventually reach `invoke` — EAL
//   interpreter, MCP `send_to_agent` handler, `easynet agent send`
//   CLI — are all synchronous. Introducing `async_trait` just to
//   please one subsystem would ripple a `.await` boundary through
//   every caller and force a tokio runtime into CLI start-up paths
//   that do nothing but list agents or compile IR. The daemon + WS
//   tokio landing is planned for a subsequent PR; until then, sync
//   signatures keep the dispatch hot-path flat and the compiled
//   binary small.
//
// Why concrete request/response types instead of an associated
// type per driver?
//   Every driver already funnels its result through `AgentResponse`
//   (the same shape the MCP tool returns). A driver-specific
//   response would need a collapse to `AgentResponse` at the
//   dispatch seam anyway, so we stage that collapse inside the
//   adapter itself and keep the seam narrow.
//
// What this trait does NOT yet model:
//   - Streaming events (`AgentStreamEvent`)
//   - Tool-call normalization (`ToolCallDetail`)
//   - Long-lived sessions + timeline
//   - Permission round-trip
//   Each of the above lands in its own PR and has its own
//   consumer. Introducing them here prematurely would bloat the
//   trait surface for no caller benefit today.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::dispatch::{AgentResponse, AgentUsage};
use super::timeline::TimelineWriter;
use crate::registry::agents::AgentEntry;

/// Per-invocation knobs the dispatch layer has already resolved
/// from the `AgentEntry` + request. An adapter receives these
/// verbatim; it does not re-read registry state.
pub struct InvokeOpts {
    pub timeout: Duration,
    pub max_output_bytes: usize,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    /// PR-7 Commit 2: Timeline writer for this invocation. When
    /// `Some`, the driver's stdout-line callback emits a
    /// `progress` event per stream chunk, fsynced to disk and
    /// broadcast to any live subscribers. `None` in tests that
    /// do not need timeline observation; absence is not an error.
    ///
    /// Plumbing Timeline through `InvokeOpts` keeps the adapter
    /// trait signature unchanged (still sync, no async_trait) —
    /// the driver emits synchronously and the P2 discipline is
    /// upheld by `TimelineWriter::emit` holding the fsync
    /// barrier before broadcast wake (see `runtime::timeline`).
    pub timeline: Option<Arc<TimelineWriter>>,
    /// Optional live-progress callback. When Some, the driver
    /// invokes it once per stdout line emitted by the spawned
    /// LLM CLI — i.e. once per token-batch in stream-json mode.
    /// The callback receives a typed `progress` frame with the
    /// raw driver chunk inside; the chat ability's stream
    /// handler uses this to forward per-token progress to the
    /// InvokeBidi/Stream subscriber's broadcast channel.
    ///
    /// Distinct from `timeline`: timeline is durable
    /// (fsynced-to-disk) audit log; progress_tx is the
    /// volatile in-memory wire to a live subscriber. Both can
    /// be set at once — every driver line goes to BOTH, in
    /// that order (timeline first so disk durability holds the
    /// barrier before subscribers wake).
    ///
    /// Pre-fix the chat stream emitted only {session, loaded?,
    /// done|error} — three frames total per call regardless of
    /// LLM response length. The ability's wire shape claimed
    /// to be \"streaming\" but in practice was a snapshot +
    /// terminal frame. Real-world audit caught it.
    pub progress_tx: Option<Arc<dyn Fn(serde_json::Value) + Send + Sync>>,
    /// Binary to spawn for this agent's runtime. Resolved at
    /// dispatch time from `AgentEntry::command`, with an empty
    /// string signalling "take the driver default" (claude /
    /// codex / …). Load-bearing: without this field the drivers
    /// hardcoded the binary name, which silently ignored any
    /// operator override and made test-mode fake commands
    /// impossible. See `runtime/drivers/claude_code.rs` and
    /// `runtime/drivers/codex.rs` for the empty-string fallback
    /// rule.
    pub command: String,
    /// When `Some(<id>)`, the driver should resume an existing
    /// conversation (codex: `codex exec resume <id>`) instead of
    /// starting a fresh one. Drivers that do not support resume
    /// (claude-code today) ignore this field and treat each call
    /// as fresh. The chat ability sets this from the caller's
    /// `session_id` argument when it looks like a driver-issued
    /// thread id; on a fresh conversation it is `None`.
    pub resume_thread_id: Option<String>,
}

/// A runtime driver that can invoke an external agent binary once
/// and return a single, fully-realized `AgentResponse`.
///
/// Thread-safety: implementations are trait objects held in a
/// `'static` table (see `drivers::adapter_for`), so they must be
/// `Send + Sync`. Every current driver is stateless — the `&self`
/// receiver is there for future drivers that want to cache
/// binary-path discovery, not for mutable state.
///
/// The `runtime_id` / `is_available` accessors currently have no
/// call sites inside the runtime layer — they are part of the
/// trait's public contract and will be consumed by
/// `easynet agent doctor` and the `registry/agents` v2 schema
/// once those PRs land. Keeping them here prevents the trait from
/// churning in the next PR when those sites come online.
#[allow(dead_code)]
pub trait AgentAdapter: Send + Sync {
    /// Stable runtime identifier. Matches the `agent_type` string
    /// stored in `~/.easynet/agents.json` (e.g. `"claude-code"`,
    /// `"codex"`, `"codex-app-server"`).
    fn runtime_id(&self) -> &'static str;

    /// Quick health-check: is the underlying binary on `$PATH`?
    /// Used by `easynet agent doctor`. Defaults to `true` for
    /// drivers whose binary is bundled or always discovered via
    /// some other mechanism; most drivers will want to override.
    fn is_available(&self) -> bool {
        true
    }

    /// Run one prompt. Blocking. The adapter owns its own process
    /// spawn, stream parse, and usage accounting; it returns a
    /// partial `AgentResponse` (content + usage + tool_calls) and
    /// the dispatch layer fills in bookkeeping fields (agent name,
    /// duration, run_dir path, model).
    ///
    /// Returns an `AdapterOutput` rather than a tuple so future
    /// adapters can populate new fields (truncation flag, retry
    /// count, …) without breaking the trait signature.
    fn invoke(
        &self,
        entry: &AgentEntry,
        prompt: &str,
        opts: InvokeOpts,
    ) -> anyhow::Result<AdapterOutput>;
}

/// What an adapter returns from `invoke`. Grew from a `(String,
/// Option<AgentUsage>)` tuple when tool-call observability landed —
/// using a struct keeps future field additions backward-compatible.
#[derive(Debug, Clone, Default)]
pub struct AdapterOutput {
    pub content: String,
    pub usage: Option<AgentUsage>,
    /// Tool invocations captured from the driver's stream. Empty for
    /// drivers that do not surface tool-use events (codex today).
    pub tool_calls: Vec<crate::runtime::dispatch::ToolCall>,
    /// Driver-assigned conversation id parsed from the run (codex's
    /// `thread.started` event populates this; claude-code leaves it
    /// `None` until that driver gains an equivalent surface). The
    /// dispatch layer copies this into `AgentResponse::thread_id`
    /// without inspection — the chat ability is the layer that
    /// decides what to do with it (currently: echo as `session_id`).
    pub thread_id: Option<String>,
}

/// Bridge the dispatcher's already-filled `AgentResponse` fields to
/// an adapter's partial result. Only dispatch itself should call
/// this — it centralizes the "where do truncation and duration
/// come from?" answer in one place.
///
/// Currently unused — `dispatch::send_to_agent_with_depth` builds
/// the `AgentResponse` inline to keep its existing structure
/// stable. We keep the helper because a future PR that grows the
/// number of response fields (streaming token account, permission
/// resolution status) will want a single build site instead of
/// two.
#[allow(dead_code)]
pub(super) fn finalize_response(
    agent: String,
    model: Option<String>,
    duration_ms: u64,
    content: String,
    truncated: bool,
    usage: Option<AgentUsage>,
    run_dir_path: Option<PathBuf>,
) -> AgentResponse {
    AgentResponse {
        agent,
        content,
        model,
        duration_ms,
        truncated,
        usage,
        run_dir: run_dir_path,
        tool_calls: Vec::new(),
        thread_id: None,
    }
}
