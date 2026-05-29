// EasyNet CLI — `easynet federation discover` subcommand
// =========================================================
//
// File: src/facade/cli/federation_discover.rs
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
//   pass `--agent-uri` for a single-URI lookup. A "show entries
//   from realm X" surface lands when the operator-audit RFC
//   adds a multi-realm filter argument.
//
// Wire shape
// ----------
// Default plain output:
//
//   AGENT_URI                                  NODE_ID    STATUS  ORIGIN_REALM  HUB_ENDPOINT
//   easynet:///r/realm-a/agent/device-X        device-X   active  realm-a       https://hub-a.example:50443
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

use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use clap::Args;
use console::style;
use serde_json::{json, Value};

use crate::pb::axon::v1::invocation_client::InvocationClient;
use crate::services::axon_serve::ProtoEnvelope;

#[derive(Debug, Args)]
pub struct DiscoverArgs {
    /// Filter to a single agent URI. When set, the daemon
    /// returns at most one matching entry across all federated
    /// peers (lex tie-break on peer realm).
    #[arg(long)]
    pub agent_ura: Option<String>,

    /// Filter cross-realm entries by the calling user's
    /// federated bindings (PR-N4 INV-5 privacy default). Only
    /// entries whose URI is on the calling daemon's own realm
    /// or has a recorded binding for 'user-id' are returned.
    /// Absent ⇒ unfiltered (operator / audit query path).
    #[arg(long = "user-id")]
    pub local_user_id: Option<String>,

    /// Emit JSON for scripts instead of a plain-text table.
    #[arg(long)]
    pub json: bool,
}

/// Resolve the UDS socket path the daemon's gRPC server binds.
/// Mirrors the same env-override + tilde-expansion the
/// federation_invoke bridge uses so the two CLI surfaces stay
/// aligned across test deployments.
pub fn run(args: DiscoverArgs) -> anyhow::Result<()> {
    let socket_path = crate::support::local_daemon_grpc::resolve_socket_path();
    if !crate::support::local_daemon_grpc::probe_accepting(&socket_path) {
        bail!(
            "daemon gRPC listener not reachable at {} — start the daemon first \
             (`easynet runtime start`) and confirm `~/.easynet/daemon-config.toml` \
             enables the transport plane.",
            socket_path.display()
        );
    }

    // Build the request. Use the daemon's own loopback URI as
    // caller — the daemon's admission gate has a loopback
    // bypass so operator-side calls bypass the strict signature
    // pipeline. PR-N5 will replace this with a real signed
    // operator envelope when the user-as-subject surface lands.
    let mut req_args = json!({});
    if let Some(uri) = args.agent_ura.as_deref() {
        req_args["agent_ura"] = Value::String(uri.to_string());
    }
    if let Some(user) = args.local_user_id.as_deref() {
        req_args["local_user_id"] = Value::String(user.to_string());
    }
    let arg_bytes = serde_json::to_vec(&req_args).context("encode discover args")?;

    // Caller URI: derive from credentials.json so the daemon's
    // loopback bypass admits us. If credentials are missing,
    // fall back to a generic operator URI; the daemon will
    // reject if its trust set hasn't been wired for that URI.
    let caller_ura = crate::persistence::config::load_credentials()
        .ok()
        .map(|c| crate::ura::device_ura(&c.tenant_id, &c.node_id))
        .unwrap_or_else(|| crate::ura::device_ura("cli", "local"));

    let request =
        ProtoEnvelope::loopback(caller_ura)?.invoke_request("federation.discover", arg_bytes)?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for federation discover")?;

    let response: crate::pb::axon::v1::InvokeResponse = {
        runtime.block_on(async move {
            let channel = crate::support::local_daemon_grpc::connect_channel(
                socket_path.clone(),
                Duration::from_secs(10),
                Duration::from_secs(5),
            )
            .await
            .context("connect to local daemon gRPC endpoint")?;
            let mut client = InvocationClient::new(channel);
            let resp = client.invoke(request).await.map_err(|status| {
                anyhow!(
                    "daemon rejected federation.discover: code={:?} message={}",
                    status.code(),
                    status.message()
                )
            })?;
            Ok::<_, anyhow::Error>(resp.into_inner())
        })?
    };

    let body: serde_json::Value =
        serde_json::from_slice(&response.result).context("decode discover response body")?;
    let entries = body
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

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
        "AGENT_URI", "NODE_ID", "STATUS", "ORIGIN_REALM"
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
