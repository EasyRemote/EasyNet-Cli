// EasyNet CLI — Infrastructure Layer
// ==================================
//
// File: src/shared/mod.rs
// Description: Transport and I/O plumbing reused by every CLI
//              subcommand. Narrowly scoped after the persistence
//              and identity concerns were hoisted out.
//
// Scope (narrowed)
// ----------------
// This module now contains only infrastructure primitives:
//
//   net.rs         — host/port parsing and PID discovery
//   node.rs        — `nodes[]` interpretation helpers (online, state)
//   output.rs      — terminal formatting (tables, colors, JSON/plain)
//   shutdown.rs    — cooperative shutdown signalling
//   sysinfo.rs     — device fingerprint collection
//   timeouts.rs    — centralized timeout tower (user + infrastructure)
//
// What MOVED OUT
// --------------
//   config.rs           → `crate::persistence::config`
//   agents.rs           → `crate::registry::agents`
//   agent_id.rs         → `crate::core::agent_id`
//   a2a_labels.rs       → `crate::registry::a2a_labels`
//   connect_bridge()    → `crate::persistence::config` (needs RuntimeState)
//   BRIDGE_CONNECT_TIMEOUT_MS
//                       → `crate::support::timeouts` (centralised tower)
//
// Dependency direction invariant
// ------------------------------
// `shared` is the leaf layer: it has no dependency on `persistence`
// or `registry`. Earlier drafts had a `connect_bridge()` helper here
// that read `RuntimeState` from `persistence::config`, which made
// `shared` a *consumer* of `persistence` — quietly inverting the
// intended layering. Any function that needs to read on-disk state
// before opening a transport belongs in the module that owns that
// state (`persistence`), leaving `shared` as a pure-plumbing crate
// that takes values in, not paths. This module enforces that
// invariant: [`connect_bridge_to`] is the only bridge helper here,
// and it takes the endpoint as a parameter.
//
// Why the split
// -------------
// Before the split, `shared/` was a dumping ground: persistence,
// identity, and infrastructure all sat in one module because none
// of them had a better home. That made grep-to-intent harder than
// it needed to be — a reader looking for "where is the agent
// registry?" found it next to "where is the bridge pool?" as if
// they were peers. They are not. The split puts each concern in a
// module whose name answers "what is this?" from the use site
// alone:
//
//     use crate::persistence::config;          // on-disk state
//     use crate::registry::agents;             // agent registry
//     use crate::core::agent_id::NodeId;   // typed identity
//
// Architectural Position
// ----------------------
// Horizontal leaf layer below `cli/`, `persistence`, and `registry`
// (all of which may consume it) and above the `easynet-axon` SDK.
// No command-specific logic lives here; only reusable plumbing.
//
// Operator-log convention
// -----------------------
// This codebase does NOT depend on `tracing` or `log` at the
// declaration level — operator-visible runtime events are emitted to
// stderr. Call sites MUST go through the [`crate::op_event!`] macro
// rather than raw `eprintln!`. The macro enforces the shape at
// compile time (component / kind / field names are `ident` tokens,
// so spaces and dots are rejected by the parser) and renders the
// project's standard format:
//
//     [<component>] kind=<event> key1=<val> key2=<val> ...
//
//   * `<component>` is a stable hyphenated module tag — write it as
//     a Rust ident (`mcp_http_client`); the macro lowercases
//     underscores to hyphens for the `[bracket]` tag while leaving
//     field names verbatim so `grep kind=tls_insecure` is stable.
//   * `kind=<event>` is the stable enum-like event class
//     (`tls_insecure`, `auto_route`, `reflection_skipped`).
//   * Subsequent `key=value` pairs carry per-event detail. Values
//     containing whitespace are auto-quoted by the macro.
//
// Why a macro instead of raw `eprintln!`: a documented convention is
// not enforced by the compiler — the 2026-05-24 audit caught a fresh
// `eprintln!` violating the convention inside the same PR that
// codified it. The macro turns the convention into a type-level
// contract; the next maintainer cannot accidentally drift the shape.
//
// Why not `tracing`: pulling in `tracing` / `tracing-subscriber` at
// the daemon root would force every downstream binary (CLI, FFI
// surfaces, embedded callers) to take the same dependency for no
// behavioural gain — the operator-visible surface is already stderr.
// If/when a structured-logging migration is greenlit, the macro body
// is the single rewrite point; call sites already declare
// component / kind / field=value in the shape `tracing::event!`
// wants. No call-site churn.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

/// `run_blocking()` + `try_run_blocking_in_tokio()` — the single
/// recipe for driving a future to completion from sync code,
/// with explicit fallback policy. Replaces three near-identical
/// `block_on_*` helpers that used to live in
/// `daemon/ability/dispatch.rs`,
/// the agent lifecycle system ability, and
/// `daemon/invocation/local_runtime_invoker.rs`.
pub mod async_bridge;

/// `append_cleanup_error()` — fold a best-effort cleanup outcome into a
/// primary error so transactional rollback paths report both what failed
/// and whether compensation completed. Shared by `daemon::ability::builtins`
/// and `persistence` rollback sites; see `errors.rs` for the rationale.
pub(crate) mod errors;

// federation_invoke moved under daemon::invocation, and the
// product read boundary lives in daemon::federation::directory_reader.
// Keeping both out of support prevents this module from reaching upward into
// services — the production back-edge this split removed.

/// Transport plumbing for the local daemon's Invocation gRPC
/// surface — socket resolution, UDS / named-pipe connect, tonic
/// channel construction, and the `LocalDaemonAbilityClient` value
/// object that carries a caller-override URA.
///
/// **Do not call into this module from CLI surfaces.** Use
/// [`local_invoke::invoke_local_ability`] (no caller override) or
/// the `LocalDaemonAbilityClient` constructors. The transport
/// helpers here are pub(crate) for that one shim; widening their
/// usage re-spawns the "one CLI subcommand opens its own IPC"
/// anti-pattern this module file was created to eliminate.
pub(crate) mod invocation_receipt_projection;
pub(crate) mod local_daemon_grpc;

/// **Canonical CLI ability-invocation surface.** One helper —
/// `invoke_local_ability(name, args)` — every CLI subcommand uses
/// to talk to the local daemon's Axon Invocation gRPC surface. Per
/// the AXON-RFC-001 ontology, every CLI action collapses to one
/// ability Invoke; centralising the daemon.sock bridge here keeps
/// CLI commands out of registry internals and legacy control-plane
/// dispatch. The day the transport evolves, this is the **one**
/// module to swap.
pub(crate) mod local_invoke;
#[cfg(windows)]
pub mod named_pipe;
pub(crate) mod net;
pub(crate) mod node;
/// Operator-log macro `op_event!`. See module header for the
/// convention; see `operator_log.rs` for the implementation
/// rationale and the migration-to-`tracing` story.
pub mod operator_log;
pub(crate) mod output;
/// `ProcessSingleton<T>` — typed "set at boot, read on the hot path"
/// handle. Replaces ad-hoc `OnceLock` / `RwLock<Option<_>>` statics
/// scattered across the agents layer; see `process_singleton.rs` for
/// the mode-choice rationale.
pub(crate) mod process_singleton;
#[cfg(feature = "axon-pb")]
pub(crate) mod remote_device;
/// AXIOM Tier 2.5 bash safety subsystem. Self-contained set
/// of helpers (destructive command list, hardened process
/// runner, AST + security pipeline added in later slices)
/// shared between `process.exec`, `shell.run`, and any
/// future ability that needs the same hardening surface.
/// See `shellguard/mod.rs` for the design rationale.
pub(crate) mod shellguard;
pub(crate) mod shutdown;
pub(crate) mod sysinfo;
pub(crate) mod timeouts;

use anyhow::Context;

/// Open a [`DendriteBridge`] to a given endpoint, using the shared
/// [`crate::support::timeouts::BRIDGE_CONNECT_TIMEOUT_MS`] budget.
///
/// This is the low-level, state-free entry point. Callers that also
/// want the runtime's on-disk [`RuntimeState`] (tenant, label, PID)
/// alongside the bridge should use
/// [`crate::persistence::config::RuntimeState::connect_bridge`] or
/// [`crate::persistence::config::load_and_connect`], both of which
/// wrap this function after a [`crate::persistence::config::load`].
///
/// [`RuntimeState`]: crate::persistence::config::RuntimeState
/// [`DendriteBridge`]: easynet_axon::dendrite_bridge::DendriteBridge
pub fn connect_bridge_to(
    endpoint: &str,
) -> anyhow::Result<easynet_axon::dendrite_bridge::DendriteBridge> {
    easynet_axon::dendrite_bridge::DendriteBridge::connect(
        endpoint,
        timeouts::BRIDGE_CONNECT_TIMEOUT_MS,
    )
    .with_context(|| format!("bridge connect to {endpoint}"))
}
