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
//   bridge_pool.rs — DendriteBridge connection pool (lock-free)
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
//   agent_id.rs         → `crate::registry::agent_id`
//   a2a_labels.rs       → `crate::registry::a2a_labels`
//   connect_bridge()    → `crate::persistence::config` (needs RuntimeState)
//   BRIDGE_CONNECT_TIMEOUT_MS
//                       → `crate::shared::timeouts` (centralised tower)
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
//     use crate::registry::agent_id::NodeId;   // typed identity
//     use crate::shared::bridge_pool;          // transport
//
// Architectural Position
// ----------------------
// Horizontal leaf layer below `cli/`, `persistence`, and `registry`
// (all of which may consume it) and above the `easynet-axon` SDK.
// No command-specific logic lives here; only reusable plumbing.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub(crate) mod bridge_pool;
pub(crate) mod net;
pub(crate) mod node;
pub(crate) mod output;
pub(crate) mod shutdown;
pub(crate) mod sysinfo;
pub(crate) mod timeouts;

use anyhow::Context;

/// Open a [`DendriteBridge`] to a given endpoint, using the shared
/// [`crate::shared::timeouts::BRIDGE_CONNECT_TIMEOUT_MS`] budget.
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
