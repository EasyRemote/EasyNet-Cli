// EasyNet CLI — Runtime Layer
// ===========================
//
// File: src/runtime/mod.rs
// Description: Reverse-dispatch layer that lets EasyNet invoke
//              external agent CLIs (Claude Code, Codex, …) as
//              programmable "edge agents".
//
// Submodules:
//   drivers/        — Per-runtime subprocess drivers (one file per
//                     runtime binary). Each driver owns its own
//                     process-spawn plumbing, JSONL/SSE stream
//                     parsing, and usage accounting.
//   dispatch        — Unified routing + per-run persistence +
//                     recursion guard. The single entry point for
//                     EAL member-call dispatch and the MCP
//                     `send_to_agent` tool.
//   context         — Thread-local dispatch context (mission id,
//                     depth, origin) with env-var fallback for
//                     subprocess children.
//   process_runner  — Shared subprocess helpers (spawn, line
//                     callbacks, byte caps).
//   run_store       — Per-run on-disk layout
//                     (<agent-root>/runs/<ts>/…).
//   stream_ui       — Terminal rendering of live JSONL events.
//   toml_escape     — Shell-safe TOML string escaping used when
//                     generating on-disk runtime config.
//   workspace       — Projects an agent's state onto its on-disk
//                     runtime-native layout (.mcp.json / .codex/ /
//                     CLAUDE.md / AGENTS.md / .git/) from the
//                     AgentDirectory source of truth.

pub(crate) mod adapter;
pub(crate) mod agent_ability_specs;
pub(crate) mod context;
pub(crate) mod directory;
pub(crate) mod dispatch;
pub(crate) mod drivers;
pub(crate) mod process_runner;
pub mod resources;
pub(crate) mod run_store;
pub(crate) mod session;
pub(crate) mod skill_store;
pub(crate) mod stream_ui;
pub(crate) mod timeline;
pub(crate) mod toml_escape;
pub(crate) mod workspace;

/// Install a mission dispatch context on the current thread and return the
/// guard that restores the previous context on drop.
///
/// This is the narrow public bridge used by crate binaries that must exercise
/// runtime dispatch directly instead of entering through
/// `cli::mission_runs::run_mission_inproc`. It intentionally does not
/// expose the full `context` module: production mission execution still owns
/// context construction, while subprocess propagation remains centralized in
/// `DispatchContext::serialize_to_env`.
#[must_use = "mission context only stays installed while the returned guard is alive"]
pub fn enter_mission_context_for_current_thread(
    mission_id: impl Into<String>,
    mission_run_dir: impl Into<std::path::PathBuf>,
) -> impl Drop {
    context::enter(context::DispatchContext::for_mission(
        mission_id,
        mission_run_dir.into(),
    ))
}

// v10.5 R1 (PR-DAEMON) — Invocation as the system-level unit of
// execution + domain object model at the KernelApi boundary +
// Receipt subscriber v2 extension point. These modules are `pub`
// because:
//   * `domain` and `invocation` types appear in KernelApi method
//     signatures and must be reachable from Control-layer code
//     under `src/daemon/control/` (to be added in a follow-up
//     commit of PR-DAEMON).
//   * `kernel_api` is the trait the Control layer consumes; its
//     only legal import path from Control is via this crate root.
//   * `receipt_subscriber` exposes a v2 extension point; v1 code
//     never consumes it, but the trait needs to be reachable from
//     future out-of-tree consumers.
pub mod failure_codes;
pub mod invocation;
pub mod join_connection_state;
pub mod kernel_api;
pub mod receipt_subscriber;

// Runtime→Network boundary: GatewayApi is the trait the Execution
// layer uses when it needs to touch federation (publish an ability,
// invoke remotely, enumerate peers, heartbeat). The implementation
// in `gateway.rs` holds the DendriteBridge; Execution code never
// imports `gateway` directly, only `gateway_api`. This split is the
// second hard boundary (the first being KernelApi above) enforced
// by engineering/scripts/check-kernel-boundary.sh.
pub mod gateway_api;

// Stage 1 of two-stage dispatch: InvocationPlan → InvocationTarget.
// The resolver decides `Local` vs `Remote { node_id }` in one place
// so handlers never re-implement `target_node == self.node_id` logic.
// Stage 2 (the executor) is a follow-up file `ability_dispatch.rs`;
// PR-SYS swaps the existing dispatch.rs call sites over to it.
pub mod invocation_target;
pub(crate) mod local_invocation_identity;
pub mod local_runtime_invoker;

// Kernel + Gateway implementations.
// - `kernel` provides the single execution entry Kernel::invoke that
//   schedule tick / loop controller / permission broker / Client FFI
//   all converge on (U1 invariant, v10.3 C*).
// - `gateway` is the AxonGateway impl that fronts federation calls;
//   in PR-DAEMON it is a thin skeleton and later PRs flesh it out.
pub mod gateway;
pub mod kernel;

// Execution sub-services (v10.2 isolation layer). Each sub-service
// owns its own state; the Kernel holds handles to all of them and
// routes inter-service calls so sub-services never import each
// other. engineering/scripts/check-subservice-isolation.sh grep-enforces the
// "no peer import" rule.
pub mod execution;
pub mod executors;

// Stage-2 dispatch executor for daemon-owned and agent-owned abilities.
// `ability_dispatch` consumes `InvocationTarget` (from
// stage-1 resolver in `invocation_target.rs`) and routes either to
// the in-process `AxonAbilityCatalog` or via `GatewayApi`.
// `system_ability_catalog::build_registry` populates the registry with every
// device-level ability the daemon publishes (today: `observe.health`;
// PR-ATTACH onwards extends this).
pub mod ability;
pub mod ability_descriptor;
pub mod ability_dispatch;
pub mod ability_names;
pub mod ability_wire;
pub mod advertise;
/// Bridge layer between CLI's existing services (RealmTrustAnchor,
/// InvocationLedger, etc.) and `easynet_axon::invocation`'s SDK
/// types (`KeyResolver`, `LedgerSink`, `LocalRuntime`). Phase 1–5
/// of the "use Axon SDK directly, stop reinventing" migration lives
/// here. Everything in this module is glue: it carries no
/// independent state of its own beyond holding `Arc` handles to
/// existing services + the constructed Axon objects.
///
/// Lives under `runtime/` (not `services/`) because it imports only
/// Axon SDK types and runtime ability dispatch, system abilities, and
/// invocation_target glue. Service-owned state is adapted in the
/// services layer and injected through traits.
pub mod axon_bridge;
pub mod dispatch_receipt;
pub mod federation_client;
pub(crate) mod owner_projection;
pub mod plugin_host;
pub mod provisional_ura;
pub mod publish;
pub mod system_abilities;
pub mod system_ability_catalog;
// RFC-002 keyring + KeyResolver. Local-first, zero axon dependency.
pub mod keyring;
// RFC-002 tenant suffix resolver: maps tenant_id to admission mode +
// URA scope + hub endpoints.
/// RFC-002.2 daemon-side federation initialisation. Pure decision
/// over (Credentials, KeyringHandle, Bridge?) → install + record
/// outcome. Daemon boot calls one function; the status probe
/// surfaces the result for operators.
pub mod federation_init;
/// RFC-006-B v0.6 — Hub module. v0 carries the in-daemon Pages
/// listener (HTTP boundary for the Pages reference system).
/// Production traffic enters via the Go backend; this listener
/// is the dev-mode existence proof.
pub mod hub;
pub mod resolver;
