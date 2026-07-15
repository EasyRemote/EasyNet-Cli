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
//   net.rs         — explicit process lifecycle helpers
//   node.rs        — `nodes[]` interpretation helpers (online, state)
//   output.rs      — terminal formatting (tables, colors, JSON/plain)
//   shutdown.rs    — cooperative shutdown signalling
//   sysinfo.rs     — device fingerprint collection
//   timeouts.rs    — centralized timeout tower (user + infrastructure)
//
// What MOVED OUT
// --------------
//   config.rs           → `crate::daemon::persistence::config`
//   agents.rs           → `crate::daemon::persistence::agent_registry`
//   agent_id.rs         → `crate::core::agent::id`
//   a2a_labels.rs       → `crate::daemon::federation::read_model::a2a_labels`
//
// Dependency direction invariant
// ------------------------------
// `support` is a leaf layer with no dependency on persistence or
// product registries. Transport ownership stays in daemon Invocation.
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
//     use crate::daemon::persistence::config;          // on-disk state
//     use crate::daemon::persistence::agent_registry as agents;             // agent registry
//     use crate::core::agent::id::NodeId;   // typed identity
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

pub mod async_bridge;
pub mod platform;
pub(crate) mod shellguard;
