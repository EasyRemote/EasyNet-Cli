// EasyNet CLI
// ===========
//
// File: src/cli/status.rs
// Description: `easynet status` — displays Hub connection info, node/ability summary.
//
// Data Sources:
// - Runtime state from ~/.easynet/runtime.json (Hub URL, tenant, label).
// - Live node list via DendriteBridge.list_nodes() — counts online/offline.
// - Ability count via DendriteBridge.list_mcp_tools() aggregated across all nodes.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use crate::shared::{self, config, output};

#[derive(Debug, Args)]
pub struct StatusArgs {}

pub fn run(_args: StatusArgs) -> anyhow::Result<()> {
    let state = config::load()?;
    let br = shared::connect_bridge()?;
    let tenant = state.tenant_or_default();

    let nodes = br
        .list_nodes(tenant, None)
        .map_err(|e| anyhow::anyhow!("list nodes: {e}"))?;

    let online = nodes.iter().filter(|n| is_online(n)).count();
    let offline = nodes.len() - online;

    let mut ability_count = 0usize;
    for node in &nodes {
        if let Some(node_id) = node.get("node_id").and_then(|v| v.as_str()) {
            if let Ok(tools) = br.list_mcp_tools(tenant, "", node_id) {
                ability_count += tools.len();
            }
        }
    }

    output::info(&format!("EasyNet CLI v{}", env!("CARGO_PKG_VERSION")));
    output::info(&format!(
        "Hub: {}",
        state.hub.as_deref().unwrap_or("-")
    ));
    output::info(&format!("Endpoint: {}", state.endpoint));
    output::info(&format!("Tenant: {tenant}"));
    output::info(&format!(
        "Label: {}",
        state.label.as_deref().unwrap_or("-")
    ));
    output::info(&format!("Nodes: {online} online, {offline} offline"));
    output::info(&format!("Abilities: {ability_count} active across network"));
    Ok(())
}

fn is_online(n: &serde_json::Value) -> bool {
    n.get("state")
        .and_then(|s| s.as_str())
        .map(|s| s == "HEALTHY" || s == "ONLINE")
        .unwrap_or(false)
}
