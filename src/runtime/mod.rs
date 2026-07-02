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

pub mod failure_codes;
pub mod join_connection_state;

pub mod executors;

// Stage-2 dispatch executor for daemon-owned and agent-owned abilities.
// `ability_dispatch` consumes `InvocationTarget` (from the daemon
// stage-1 resolver in `daemon::invocation::target`) and routes either to
// the in-process `AxonAbilityCatalog` or via daemon federation `GatewayApi`.
// `daemon::ability::catalog::build_registry` populates the registry with every
// device-level ability the daemon publishes (today: `observe.health`;
// PR-ATTACH onwards extends this).
pub mod advertise;
pub mod dispatch_receipt;
pub mod federation_client;
pub mod provisional_ura;
pub mod publish;
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
