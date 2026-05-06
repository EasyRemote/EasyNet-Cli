// EasyNet CLI — `easynet runtime status`
// =======================================
//
// File: src/facade/cli/status.rs
// Description: Hub connection info + fleet summary. Joint-plan
//              unified path: cross-device enumeration goes through
//              `federation.discover` (the same surface
//              `easynet device list` uses); ability count goes
//              through `easynet.discover`. No more
//              `fleet.list_nodes` — that handler is on the phase 4
//              cull list.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;
use serde_json::{json, Value};

use crate::persistence::config;
use crate::support::local_invoke::invoke_local_ability;
use crate::support::net;
use crate::support::output;

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
    output::detail("Tenant", state.tenant.as_deref().unwrap_or("default"));
    output::detail("Label", state.label.as_deref().unwrap_or("-"));
    match state.runtime_kind {
        config::RuntimeKind::DaemonOnly => {
            output::detail("Mode", "daemon-only");
            output::detail("gRPC socket", &state.endpoint);
            output::detail(
                "Control socket",
                &crate::services::control::transport::default_socket_path()
                    .display()
                    .to_string(),
            );
        }
        config::RuntimeKind::AxonBridge => {
            output::detail("Mode", "bridge/hub");
            output::detail("Bridge endpoint", &state.endpoint);
            if let Some(pid) = state.pid {
                output::detail("PID", &pid.to_string());
            }
        }
    }
    if state.credential_verified == Some(false) {
        output::info("Credential: NOT VERIFIED (Hub was unreachable at startup)");
    }
    if state.uses_bridge() {
        let alive = state.pid.is_some_and(net::is_pid_alive)
            || net::discover_pid_from_endpoint(&state.endpoint).is_some();
        if alive {
            output::info(
                "Bridge-mode runtime is up. Local daemon-only fleet and ability probes are skipped in this mode.",
            );
        } else {
            output::warn(
                "Runtime metadata exists, but the recorded bridge process is not responding.",
            );
        }
        return Ok(());
    }

    match invoke_local_ability("device.observe.health", json!({"source": "runtime.status"})) {
        Ok(_) => {}
        Err(e) => {
            output::warn(&format!(
                "Local daemon is not responding to observe.health despite runtime metadata: {e}"
            ));
            return Ok(());
        }
    }

    // Fleet view — go through `federation.discover` (the joint-plan
    // unified path the rest of the CLI uses). DirectoryEntries land
    // with a `status` field (`active` / `stale` / `draining`); we
    // count `active` as online so the summary line matches what
    // `easynet device list` shows.
    let entries = fetch_directory_entries();
    let total = entries.len();
    let online = entries
        .iter()
        .filter(|e| e.get("status").and_then(Value::as_str) == Some("active"))
        .count();
    let offline = total.saturating_sub(online);
    output::info(&format!("Nodes: {online} online, {offline} offline"));

    // Ability count — go through easynet.discover (one call,
    // returns the full local catalogue). Cheaper than the legacy
    // O(N) per-node fan-out and matches what `easynet ability list`
    // reports.
    match invoke_local_ability("device.meta.list_abilities", serde_json::json!({})) {
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

/// Pull the federated directory snapshot from the local daemon.
/// Best-effort: a transport / parse failure surfaces an empty list
/// + an info line so the rest of the status output still renders
/// (the operator already saw "daemon up" via `observe.health`
/// above; whether any peers are advertised is a softer signal).
#[cfg(feature = "axon-pb")]
fn fetch_directory_entries() -> Vec<Value> {
    match crate::support::federation_invoke::invoke_federation_discover(None, None) {
        Ok(entries) => entries,
        Err(e) => {
            output::info(&format!("Fleet: cannot query federation.discover (`{e}`)"));
            Vec::new()
        }
    }
}

#[cfg(not(feature = "axon-pb"))]
fn fetch_directory_entries() -> Vec<Value> {
    output::info("Fleet: federation.discover requires the `axon-pb` feature");
    Vec::new()
}
