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
//   conversation    — Multi-agent round-robin `discuss` pattern
//                     (read by `facade::cli::discuss`). Lives at
//                     runtime-top rather than under drivers/
//                     because it is runtime-agnostic — it composes
//                     any two adapter-registered agents.
//   process_runner  — Shared subprocess helpers (spawn, line
//                     callbacks, byte caps).
//   run_store       — Per-run on-disk layout
//                     (<agent-root>/runs/<ts>/…).
//   stream_ui       — Terminal rendering of live JSONL events.
//   toml_escape     — Shell-safe TOML string escaping used when
//                     generating on-disk runtime config.
//   workspace       — Projects an agent's state onto its on-disk
//                     runtime-native layout (.mcp.json / .codex/ /
//                     CLAUDE.md / AGENTS.md / .git/). Today it
//                     writes the legacy `workspaces/<name>/`
//                     layout unchanged; full AgentDirectory-driven
//                     projection lands in a subsequent PR.

pub(crate) mod abilities;
pub(crate) mod adapter;
pub(crate) mod context;
pub(crate) mod conversation;
pub(crate) mod directory;
pub(crate) mod dispatch;
pub(crate) mod drivers;
pub(crate) mod process_runner;
pub(crate) mod run_store;
pub(crate) mod session;
pub(crate) mod stream_ui;
pub(crate) mod timeline;
pub(crate) mod toml_escape;
pub(crate) mod workspace;

// v10.5 R1 (PR-DAEMON) — Invocation as the system-level unit of
// execution + domain object model at the KernelApi boundary +
// Receipt subscriber v2 extension point. These modules are `pub`
// because:
//   * `domain` and `invocation` types appear in KernelApi method
//     signatures and must be reachable from Control-layer code
//     under `src/services/control/` (to be added in a follow-up
//     commit of PR-DAEMON).
//   * `kernel_api` is the trait the Control layer consumes; its
//     only legal import path from Control is via this crate root.
//   * `receipt_subscriber` exposes a v2 extension point; v1 code
//     never consumes it, but the trait needs to be reachable from
//     future out-of-tree consumers.
pub mod domain;
pub mod invocation;
pub mod kernel_api;
pub mod receipt_subscriber;

// Runtime→Network boundary: GatewayApi is the trait the Execution
// layer uses when it needs to touch federation (publish an ability,
// invoke remotely, enumerate peers, heartbeat). The implementation
// in `gateway.rs` holds the DendriteBridge; Execution code never
// imports `gateway` directly, only `gateway_api`. This split is the
// second hard boundary (the first being KernelApi above) enforced
// by scripts/check-kernel-boundary.sh.
pub mod gateway_api;

// Stage 1 of two-stage dispatch: InvocationPlan → InvocationTarget.
// The resolver decides `Local` vs `Remote { node_id }` in one place
// so handlers never re-implement `target_node == self.node_id` logic.
// Stage 2 (the executor) is a follow-up file `ability_dispatch.rs`;
// PR-SYS swaps the existing dispatch.rs call sites over to it.
pub mod invocation_target;

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
// other. scripts/check-subservice-isolation.sh grep-enforces the
// "no peer import" rule.
pub mod execution;

// PR-SYS: stage-2 dispatch executor + the `system.*` ability
// namespace. `ability_dispatch` consumes `InvocationTarget` (from
// stage-1 resolver in `invocation_target.rs`) and routes either to
// the in-process `LocalAbilityRegistry` or via `GatewayApi`.
// `agents::build_registry` populates the registry with every
// device-level ability the daemon publishes (today: `system.ping`;
// PR-ATTACH onwards extends this).
pub mod ability_dispatch;
pub mod publish;
pub mod agents;
