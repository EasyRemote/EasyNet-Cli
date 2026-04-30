// EasyNet CLI — `easynet runtime status`
// =======================================
//
// File: src/facade/cli/status.rs
// Description: Hub connection info + fleet summary. Per the
//              ability-only ontology this command sources its
//              data from `fleet.list_nodes` and `easynet.discover`
//              — the same ability surfaces every other CLI
//              command uses. No direct bridge calls.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;
use serde_json::{json, Value};

use crate::persistence::config;
use crate::support::local_invoke::invoke_local_ability;
use crate::support::{node, output};

#[derive(Debug, Args)]
pub struct StatusArgs {}

pub fn run(_args: StatusArgs) -> anyhow::Result<()> {
    output::info(&format!("EasyNet CLI v{}", env!("CARGO_PKG_VERSION")));

    if let Ok(creds) = config::load_credentials() {
        output::info("Device pairing:");
        output::detail("node_id", &creds.node_id);
        output::detail("hub_endpoint", &creds.hub_endpoint);
        output::detail("tenant_id", &creds.tenant_id);
        eprintln!();
    } else {
        output::info("Device: not paired (run `easynet device join <token>`)");
        eprintln!();
    }

    let Ok(state) = config::load() else {
        output::info("Runtime: not running");
        output::info("Run `easynet runtime start` to start.");
        return Ok(());
    };

    output::detail("Hub", state.hub.as_deref().unwrap_or("-"));
    output::detail("Endpoint", &state.endpoint);
    output::detail("Tenant", state.tenant.as_deref().unwrap_or("default"));
    output::detail("Label", state.label.as_deref().unwrap_or("-"));
    if state.credential_verified == Some(false) {
        output::info("Credential: NOT VERIFIED (Hub was unreachable at startup)");
    }
    match invoke_local_ability("observe.health", json!({"source": "runtime.status"})) {
        Ok(_) => {}
        Err(e) => {
            output::warn(&format!(
                "Local daemon is not responding to observe.health despite runtime metadata: {e}"
            ));
            return Ok(());
        }
    }

    // Fleet view — go through fleet.list_nodes (the canonical
    // ability surface) so the daemon's federation_view metadata
    // tells us whether the count is local-only or full.
    let nodes_envelope = match invoke_local_ability("fleet.list_nodes", serde_json::json!({})) {
        Ok(v) => v,
        Err(e) => {
            output::info(&format!("Fleet: cannot query (`{e}`)"));
            return Ok(());
        }
    };
    let nodes = nodes_envelope
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let online = nodes.iter().filter(|n| node::is_online(n)).count();
    let offline = nodes.len() - online;
    output::info(&format!("Nodes: {online} online, {offline} offline"));
    if let Some(view) = nodes_envelope
        .get("federation_view")
        .and_then(Value::as_str)
    {
        if view == "local_only" {
            let reason = nodes_envelope
                .get("federation_view_reason")
                .and_then(Value::as_str)
                .unwrap_or("only the local daemon is visible from this runtime");
            output::info(&format!("Federation: local-only view ({reason})"));
        }
    }

    // Ability count — go through easynet.discover (one call,
    // returns the full local catalogue). Cheaper than the legacy
    // O(N) per-node fan-out and matches what `easynet ability list`
    // reports.
    match invoke_local_ability("easynet.discover", serde_json::json!({})) {
        Ok(v) => {
            let count = v
                .get("abilities")
                .and_then(Value::as_array)
                .map(|a| a.len())
                .unwrap_or(0);
            output::info(&format!(
                "Abilities: {count} active on this node (run `easynet ability list` for the full catalogue)"
            ));
        }
        Err(e) => output::info(&format!("Abilities: cannot query (`{e}`)")),
    }
    Ok(())
}
