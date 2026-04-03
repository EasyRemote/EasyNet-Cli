// EasyNet CLI — Bridge Helper (legacy)
// =====================================
//
// File: src/shared/bridge.rs
// Description: DendriteBridge connection helper — reads runtime.json, connects to endpoint.
//
// Note: This module is superseded by the inline connect_bridge() / connect_bridge_to()
// functions in shared/mod.rs. Kept for reference; may be removed in a future cleanup.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use easynet_axon::dendrite_bridge::DendriteBridge;

use super::config;

const CONNECT_TIMEOUT_MS: u64 = 5000;

/// Connect to the local Axon runtime using the persisted endpoint.
pub fn connect() -> Result<DendriteBridge, String> {
    let state = config::load()?;
    connect_to(&state.endpoint)
}

/// Connect to a specific endpoint.
pub fn connect_to(endpoint: &str) -> Result<DendriteBridge, String> {
    DendriteBridge::connect(endpoint, CONNECT_TIMEOUT_MS)
        .map_err(|e| format!("bridge connect to {endpoint}: {e}"))
}
