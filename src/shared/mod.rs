// EasyNet CLI — Shared Utilities
// ==============================
//
// File: src/shared/mod.rs
// Description: Cross-cutting infrastructure shared by all CLI subcommands.
//
// Protocol Responsibility:
// - Provides the DendriteBridge factory (connect_bridge_to) that every command needing
//   Hub interaction calls. Single bottleneck between CLI and the native FFI layer.
//
// Provides:
//   config.rs    — ~/.easynet/ persistence (runtime state, credentials, device settings, constants)
//   output.rs    — terminal formatting (tables, colors, status indicators)
//   sysinfo.rs   — device fingerprint collection (hostname, OS, arch)
//   net.rs       — network/process utilities (port parsing, PID discovery)
//   node.rs      — node state interpretation (is_online, node_state_str)
//   connect_bridge_to() — DendriteBridge factory (caller provides endpoint)
//
// Architectural Position:
// - Horizontal layer below cli/ and above easynet-axon SDK.
// - No command-specific logic lives here; only reusable plumbing.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod agent_id;
pub mod agents;
pub mod bridge_pool;
pub mod config;
pub mod net;
pub mod node;
pub mod output;
pub mod shutdown;
pub mod sysinfo;

use anyhow::Context;

/// Default timeout (ms) for DendriteBridge connections.
pub const BRIDGE_CONNECT_TIMEOUT_MS: u64 = 5000;

/// Load runtime state and connect a `DendriteBridge` in one call.
/// Returns `(bridge, runtime_state)` so callers can access tenant, endpoint, etc.
pub fn connect_bridge() -> anyhow::Result<(easynet_axon::dendrite_bridge::DendriteBridge, config::RuntimeState)> {
    let state = config::load()?;
    let br = connect_bridge_to(&state.endpoint)?;
    Ok((br, state))
}

/// Connect a `DendriteBridge` to a specific endpoint.
pub fn connect_bridge_to(
    endpoint: &str,
) -> anyhow::Result<easynet_axon::dendrite_bridge::DendriteBridge> {
    easynet_axon::dendrite_bridge::DendriteBridge::connect(endpoint, BRIDGE_CONNECT_TIMEOUT_MS)
        .with_context(|| format!("bridge connect to {endpoint}"))
}
