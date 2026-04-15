// EasyNet CLI
// ===========
//
// File: src/cli/status.rs
// Description: `easynet status` — displays Hub connection info, node/ability summary.
//
// Data Sources:
// - Runtime state from ~/.easynet/runtime.json (Hub URL, tenant, label).
// - Credentials from ~/.easynet/credentials.json (node_id, hub_endpoint, tenant_id).
// - Live node list via DendriteBridge.list_nodes() — counts online/offline.
// - Ability count via DendriteBridge.list_mcp_tools() aggregated across online nodes.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use crate::persistence::config;
use crate::shared::{self, node, output};

/// Maximum number of online nodes to query for ability counts.
/// Prevents O(N) gRPC calls in large federations.
const MAX_NODES_TO_QUERY_ABILITIES: usize = 50;

#[derive(Debug, Args)]
pub struct StatusArgs {}

pub fn run(_args: StatusArgs) -> anyhow::Result<()> {
    output::info(&format!("EasyNet CLI v{}", env!("CARGO_PKG_VERSION")));

    // Show device pairing info if available.
    if let Ok(creds) = config::load_credentials() {
        output::info("Device pairing:");
        output::detail("node_id", &creds.node_id);
        output::detail("hub_endpoint", &creds.hub_endpoint);
        output::detail("tenant_id", &creds.tenant_id);
        eprintln!();
    } else {
        output::info("Device: not paired (run `easynet join <token>`)");
        eprintln!();
    }

    // Runtime info — may not be running.
    let Ok(state) = config::load() else {
        output::info("Runtime: not running");
        output::info("Run `easynet start` or `easynet connect` to start.");
        return Ok(());
    };

    output::detail("Hub", state.hub.as_deref().unwrap_or("-"));
    output::detail("Endpoint", &state.endpoint);
    output::detail("Tenant", state.tenant.as_deref().unwrap_or("default"));
    output::detail("Label", state.label.as_deref().unwrap_or("-"));
    if state.credential_verified == Some(false) {
        output::info("Credential: NOT VERIFIED (Hub was unreachable at startup)");
    }

    // Live federation stats — best effort.
    let Ok(br) = shared::connect_bridge_to(&state.endpoint) else {
        output::info("Bridge: cannot connect (runtime may have crashed)");
        return Ok(());
    };
    let tenant = state.tenant.as_deref().unwrap_or("default");

    let nodes = match br.list_nodes(tenant, None) {
        Ok(n) => n,
        Err(e) => {
            output::info(&format!("Federation: cannot query nodes ({e})"));
            return Ok(());
        }
    };

    let online = nodes.iter().filter(|n| node::is_online(n)).count();
    let offline = nodes.len() - online;

    // Count abilities across online nodes only (avoid O(N) calls for large federations).
    let mut ability_count = 0usize;
    let mut nodes_queried = 0usize;
    for n in &nodes {
        if !node::is_online(n) {
            continue;
        }
        if nodes_queried >= MAX_NODES_TO_QUERY_ABILITIES {
            break;
        }
        if let Some(node_id) = n.get("node_id").and_then(|v| v.as_str()) {
            if let Ok(tools) = br.list_mcp_tools(tenant, "", node_id) {
                ability_count += tools.len();
            }
            nodes_queried += 1;
        }
    }
    let ability_suffix = if nodes_queried >= MAX_NODES_TO_QUERY_ABILITIES {
        format!(" (sampled from first {MAX_NODES_TO_QUERY_ABILITIES} of {online} online nodes)")
    } else {
        format!(" (across {nodes_queried} online nodes)")
    };

    output::info(&format!("Nodes: {online} online, {offline} offline"));
    output::info(&format!(
        "Abilities: {ability_count} active{ability_suffix}"
    ));
    Ok(())
}
