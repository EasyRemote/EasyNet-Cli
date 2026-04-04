// EasyNet CLI — Shared Utilities
// ==============================
//
// File: src/shared/mod.rs
// Description: Cross-cutting infrastructure shared by all CLI subcommands.
//
// Protocol Responsibility:
// - Provides the DendriteBridge factory (connect_bridge / connect_bridge_to) that every
//   command needing Hub interaction calls. This is the single bottleneck between CLI
//   commands and the native FFI layer — all gRPC traffic flows through here.
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

pub mod config;
pub mod deploy;
pub mod net;
pub mod node;
pub mod output;
pub mod shutdown;
pub mod sysinfo;

use anyhow::Context;

/// Connect a `DendriteBridge` to a specific endpoint.
pub fn connect_bridge_to(
    endpoint: &str,
) -> anyhow::Result<easynet_axon::dendrite_bridge::DendriteBridge> {
    easynet_axon::dendrite_bridge::DendriteBridge::connect(endpoint, 5000)
        .with_context(|| format!("bridge connect to {endpoint}"))
}
