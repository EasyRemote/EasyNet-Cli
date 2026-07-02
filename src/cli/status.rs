// EasyNet CLI — `easynet runtime status`
// =======================================
//
// File: src/cli/status.rs
// Description: Hub connection info + device summary. Joint-plan
//              unified path: cross-device enumeration goes through
//              `federation.discover` (the same surface
//              `easynet device list` uses); ability count goes
//              through `easynet.discover`. No more
//              `node.list` — that handler is on the phase 4
//              cull list.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;
use serde_json::{json, Value};

use crate::persistence::config;
use crate::runtime::join_connection_state;
use crate::support::local_invoke::invoke_local_ability;
use crate::support::net;
use crate::support::output;
use crate::ura;

#[derive(Debug, Args)]
pub struct StatusArgs {
    /// Emit JSON instead of the human-readable report.
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: StatusArgs) -> anyhow::Result<()> {
    if args.json {
        return run_json();
    }
    output::info(&format!("EasyNet CLI v{}", env!("CARGO_PKG_VERSION")));
    render_connection_state();

    // Pairing block — addressed by URA (the ontology-canonical
    // identity per RFC-001 §3.2). The transport URL
    // (creds.hub_endpoint) is intentionally NOT shown: it is an
    // implementation detail. Same rule the `easynet --help` banner
    // applies. `realm` is the v4.1.4 wire field name; the in-memory
    // field is still `tenant_id` for migration reasons but the
    // rendered label tracks the spec.
    if let Ok(creds) = config::load_credentials() {
        output::info("Device pairing:");
        let realm = creds.realm_str();
        let hub_ura = ura::hub_ura(realm);
        let device_ura = ura::device_ura(realm, &creds.node_id);
        // Per RFC-001 §3.2, hub / user / device are all first-class
        // agents; the user row must use the immutable product user id,
        // not the display username slug.
        let user_ura = creds.user_ura().ok();
        let mut rows: Vec<(&str, &str)> = vec![("Hub", hub_ura.as_str())];
        if let Some(ref u) = user_ura {
            rows.push(("Current user", u.as_str()));
        }
        rows.push(("Current device", device_ura.as_str()));
        rows.push(("Realm", realm));
        output::kv_section(&rows);
        eprintln!();
    } else {
        output::info("Device: not paired (run 'easynet device join <token>')");
        eprintln!();
    }

    let Ok(state) = config::load() else {
        output::info("Runtime: not running");
        output::info("Run 'easynet runtime start' to start.");
        return Ok(());
    };

    // Runtime block. We avoid duplicating the URA values printed
    // above; this block is the runtime's own knobs (mode, sockets,
    // pid) — not identity. The `Hub` / `Tenant` / `Label` fields
    // on `RuntimeState` are the runtime's own copy of the pairing
    // state used during boot — they may differ from credentials
    // (e.g. before a fresh pairing reaches the daemon). When
    // they're identical to creds (the common case) reprinting them
    // would be noise; when they differ they belong in a future
    // diagnostics command, not the status table. Keep the runtime
    // block scoped to mode + transport + pid.
    let mut rows: Vec<(&str, String)> = Vec::new();
    match state.runtime_kind {
        config::RuntimeKind::DaemonOnly => {
            rows.push(("Mode", "daemon-only".to_string()));
            rows.push(("gRPC socket", state.endpoint.clone()));
            rows.push((
                "Control socket",
                crate::daemon::control::transport::default_socket_path()
                    .display()
                    .to_string(),
            ));
        }
        config::RuntimeKind::AxonBridge => {
            // Legacy raw axon-runtime state. Both device and hub product
            // paths now record DaemonOnly; this arm is reached only by
            // pre-unification runtime.json or non-product axon-runtime use.
            rows.push(("Mode", "bridge (legacy)".to_string()));
            rows.push(("Bridge endpoint", state.endpoint.clone()));
            if let Some(pid) = state.pid {
                rows.push(("PID", pid.to_string()));
            }
        }
    }
    let kv: Vec<(&str, &str)> = rows.iter().map(|(k, v)| (*k, v.as_str())).collect();
    output::kv_section(&kv);
    if state.credential_verified == Some(false) {
        output::info("Credential: NOT VERIFIED (Hub was unreachable at startup)");
    }
    if state.uses_bridge() {
        let alive = state.pid.is_some_and(net::is_pid_alive)
            || net::discover_pid_from_endpoint(&state.endpoint).is_some();
        if alive {
            output::info(
                "Bridge-mode runtime is up. Local daemon-only device and ability probes are skipped in this mode.",
            );
        } else {
            output::warn(
                "Runtime metadata exists, but the recorded bridge process is not responding.",
            );
        }
        return Ok(());
    }

    match invoke_local_ability("observe.health", json!({"source": "runtime.status"})) {
        Ok(_) => {}
        Err(e) => {
            // The transport layer already converts the common case
            // (daemon.sock missing/refused because the daemon process
            // is gone) into an actionable daemon-offline error with a
            // recovery hint. Surface that one directly; wrapping it in
            // "despite runtime metadata: …" duplicated the diagnosis
            // and made the actionable line harder to read. For
            // genuinely-unexpected failures (permission, protocol
            // mismatch, etc.) keep the wrapping so the diagnosis
            // context is preserved.
            let inner = format!("{e}");
            if matches!(
                crate::support::local_invoke::classify_invoke_error(&e),
                crate::support::local_invoke::LocalInvokeErrorKind::DaemonOffline
            ) {
                output::warn(&inner);
            } else {
                output::warn(&format!(
                    "Local daemon is not responding to observe.health despite runtime metadata: {inner}"
                ));
            }
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
    match invoke_local_ability("meta.list_abilities", serde_json::json!({})) {
        Ok(v) => {
            let count = v
                .get("abilities")
                .and_then(Value::as_array)
                .map(|a| a.len())
                .unwrap_or(0);
            output::info(&format!(
                "Abilities: {count} active on this node (run 'easynet ability list' for the full catalogue)"
            ));
        }
        Err(e) => output::info(&format!("Abilities: cannot query ('{e}')")),
    }
    Ok(())
}

fn render_connection_state() {
    let snapshot = join_connection_state::latest_snapshot();
    output::info("Connection state:");
    let mut rows = vec![
        (
            "State",
            format!("{} [{}]", snapshot.state, snapshot.state_code),
        ),
        (
            "Transition",
            snapshot
                .interrupted_transition
                .clone()
                .or(snapshot.transition_id.clone())
                .unwrap_or_else(|| "-".to_string()),
        ),
    ];
    if let Some(failure) = snapshot.failure.as_ref() {
        rows.push(("Failure", failure.code.clone()));
        rows.push(("Reason", failure.message.clone()));
    }
    if !snapshot.device_ura.is_empty() {
        rows.push(("Device URA", snapshot.device_ura.clone()));
    }
    let kv: Vec<(&str, &str)> = rows.iter().map(|(k, v)| (*k, v.as_str())).collect();
    output::kv_section(&kv);
    eprintln!();
}

fn run_json() -> anyhow::Result<()> {
    let connection = join_connection_state::latest_snapshot();
    let runtime = config::load().ok();
    let payload = json!({
        "connection": connection,
        "runtime": runtime.as_ref().map(|state| json!({
            "endpoint": state.endpoint,
            "runtime_kind": match state.runtime_kind {
                config::RuntimeKind::DaemonOnly => "daemon_only",
                config::RuntimeKind::AxonBridge => "axon_bridge",
            },
            "pid": state.pid,
            "hub": state.hub,
            "tenant": state.tenant,
            "label": state.label,
            "started_at": state.started_at,
            "credential_verified": state.credential_verified,
        })),
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

/// Pull the federated directory snapshot from the local daemon.
/// Best-effort: a transport / parse failure surfaces an empty list
/// + an info line so the rest of the status output still renders
/// (the operator already saw "daemon up" via `observe.health`
/// above; whether any peers are advertised is a softer signal).
#[cfg(feature = "axon-pb")]
fn fetch_directory_entries() -> Vec<Value> {
    match crate::daemon::invocation::federation_invoke::invoke_federation_discover(None) {
        Ok(entries) => entries,
        Err(e) => {
            output::info(&format!("Fleet: cannot query federation.discover ('{e}')"));
            Vec::new()
        }
    }
}

#[cfg(not(feature = "axon-pb"))]
fn fetch_directory_entries() -> Vec<Value> {
    output::info("Fleet: federation.discover requires the 'axon-pb' feature");
    Vec::new()
}
