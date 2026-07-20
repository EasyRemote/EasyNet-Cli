// EasyNet CLI — `easynet federation discover` subcommand
// =========================================================
//
// File: src/cli/federation_discover.rs
//
// Operator surface for the cross-realm directory federation
// view. PR-N3 commit N3-4 landed `federation.discover` as an
// ability dispatched through the daemon's gRPC InvocationServer;
// the N3-N4 dispatch wire (PR-N4) added optional user-binding
// filtering. This subcommand is the human-friendly entrypoint
// to that surface — operators can read the cross-realm
// directory state directly without crafting the JSON request.
//
// What this command does
// ----------------------
// Dials the local daemon's gRPC UDS, sends a unary
// `federation.discover` request with optional `agent_ura` /
// `local_user_id` filters, and renders the returned
// `DirectoryEntry` list as a table (default) or JSON (--json).
//
// What this command does NOT do
// -----------------------------
// - Force a fresh poll. The directory cell reflects the most
//   recent streaming-supervisor + poll-task observations; if
//   a peer just changed state and its frame hasn't propagated
//   yet, this command shows the cached view. Operators waiting
//   for an update should re-run after the streaming supervisor's
//   next reconnect window (~5 s).
// - Filter by realm directly. Use `--user-id` for the
//   user-binding-driven privacy filter (INV-5 default), or
//   pass `--agent-ura` for a single-URA lookup. A "show entries
//   from realm X" surface lands when the operator-audit RFC
//   adds a multi-realm filter argument.
//
// Wire shape
// ----------
// Default plain output:
//
//   AGENT_URA                                  NODE_ID    STATUS  ORIGIN_REALM  HUB_ENDPOINT
//   easynet:///r/realm-a/agent/device-X        device-X   active  realm-a       https://authority-a.example:50443
//   ...
//
// `--json` emits a structured payload:
//
//   { "entries": [ { "agent_ura": "...", "node_id": "...",
//     "display_name": null, "status": "active", "origin_realm": "...",
//     "hub_endpoint": "...", "last_seen_unix_ms": null }, ... ] }
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;
use console::style;
use serde_json::{json, Value};

#[derive(Debug, Args)]
pub struct DiscoverArgs {
    /// Filter to a single agent URA. When set, the daemon
    /// returns at most one matching entry across all federated
    /// peers (lex tie-break on peer realm).
    #[arg(long)]
    pub agent_ura: Option<String>,

    /// Filter cross-realm entries by a user's federated bindings
    /// (PR-N4 INV-5 privacy default). Only entries whose URA is on the
    /// calling daemon's own realm or has a recorded binding for 'user-id'
    /// are returned. If absent, this command performs an explicit
    /// operator/audit read of the unfiltered directory.
    #[arg(long = "user-id")]
    pub local_user_id: Option<String>,

    /// Emit JSON for scripts instead of a plain-text table.
    #[arg(long)]
    pub json: bool,
}

/// Read the daemon-backed federated directory through the shared
/// service boundary. The facade owns argument mapping and rendering;
/// transport signing and gRPC details live below it.
pub fn run(args: DiscoverArgs) -> anyhow::Result<()> {
    let entries = if let Some(local_user_id) = args.local_user_id.as_deref() {
        crate::daemon::federation::directory_reader::read_federated_directory_for_user(
            args.agent_ura.as_deref(),
            local_user_id,
        )?
    } else {
        crate::daemon::federation::directory_reader::read_federated_directory_for_operator_audit(
            args.agent_ura.as_deref(),
        )?
    };

    if args.json {
        let out = json!({ "entries": entries });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    // Plain-text table.
    if entries.is_empty() {
        println!(
            "{}",
            style(
                "no federated directory entries — peers may not be reachable yet, \
                 or no devices are paired across hubs."
            )
            .dim()
        );
        return Ok(());
    }
    println!(
        "{:<58} {:<14} {:<10} {:<14} HUB_ENDPOINT",
        "AGENT_URA", "NODE_ID", "STATUS", "ORIGIN_REALM"
    );
    for entry in &entries {
        let agent_ura = entry
            .get("agent_ura")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let node_id = entry.get("node_id").and_then(Value::as_str).unwrap_or("-");
        let status = entry.get("status").and_then(Value::as_str).unwrap_or("-");
        let origin_realm = entry
            .get("origin_realm")
            .and_then(Value::as_str)
            .unwrap_or("(local)");
        let hub_endpoint = entry
            .get("hub_endpoint")
            .and_then(Value::as_str)
            .unwrap_or("-");
        println!(
            "{:<58} {:<14} {:<10} {:<14} {}",
            agent_ura, node_id, status, origin_realm, hub_endpoint
        );
    }
    Ok(())
}
