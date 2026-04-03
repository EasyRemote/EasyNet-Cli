// EasyNet CLI — Shared Utilities
// ==============================
//
// File: src/shared/mod.rs
// Description: Cross-cutting infrastructure shared by all CLI subcommands.
//
// Provides:
//   config.rs    — ~/.easynet/runtime.json persistence (endpoint, PID, Hub, tenant)
//   output.rs    — terminal formatting (tables, colors, status indicators)
//   connect_bridge() — DendriteBridge factory using persisted runtime endpoint
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod config;
pub mod output;

/// Connect a DendriteBridge to the locally running Axon runtime.
pub fn connect_bridge() -> anyhow::Result<easynet_axon::dendrite_bridge::DendriteBridge> {
    let state = config::load()?;
    connect_bridge_to(&state.endpoint)
}

/// Connect a DendriteBridge to a specific endpoint.
pub fn connect_bridge_to(
    endpoint: &str,
) -> anyhow::Result<easynet_axon::dendrite_bridge::DendriteBridge> {
    easynet_axon::dendrite_bridge::DendriteBridge::connect(endpoint, 5000)
        .map_err(|e| anyhow::anyhow!("bridge connect to {endpoint}: {e}"))
}
