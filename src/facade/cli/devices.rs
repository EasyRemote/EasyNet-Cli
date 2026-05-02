// EasyNet CLI — `easynet device list` / `easynet device show`
// =============================================================
//
// File: src/facade/cli/devices.rs
// Description: Read-only views over the fleet's device nodes.
//              `list` enumerates every node visible from this
//              daemon; `show <id>` describes one. Both go through
//              the canonical fleet.* ability surface
//              (`fleet.list_nodes`, `fleet.describe_node`)
//              registered on the local daemon, in line with the
//              ability-only ontology — the CLI never reaches for
//              a transport directly.
//
// Pre-rewrite this file called `bridge.list_nodes(...)` directly,
// the AXON-RFC-001 P1.5 victim. The replacement abilities live in
// `runtime::agents::fleet_ops_ability` and have a stable JSON
// envelope: `{nodes: [...], federation_view: "local_only" | ... }`.
// v1 returns the local device only; the federation_view field
// surfaces the limitation so an operator who expected peer entries
// sees why none appeared.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;
use console::style;
use serde_json::{json, Value};

use crate::persistence::config;
use crate::support::local_invoke::invoke_local_ability;
use crate::support::{
    node,
    output::{self, OutputFormat},
};

/// Display length for short node IDs: "en-" prefix (3) + 8 hex chars = 11.
const SHORT_NODE_ID_LEN: usize = 11;

#[derive(Debug, Args)]
pub struct DevicesArgs {
    /// Filter by state (online, offline, all). Defaults to online.
    #[arg(long, default_value = "online")]
    pub state: String,
    /// Output format
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

pub fn run(args: DevicesArgs) -> anyhow::Result<()> {
    let resp =
        invoke_local_ability("fleet.list_nodes", json!({})).context("invoke fleet.list_nodes")?;
    let nodes = resp
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let current_node_id = config::load_credentials()
        .map(|c| c.node_id)
        .unwrap_or_default();

    let filtered: Vec<Value> = nodes
        .into_iter()
        .filter(|n| {
            let online = node::is_online(n);
            match args.state.as_str() {
                "all" => true,
                "online" => online,
                "offline" => !online,
                other => node::node_state_str(n).eq_ignore_ascii_case(other),
            }
        })
        .collect();

    if args.format == OutputFormat::Json {
        // Surface the full envelope (including `federation_view`) so
        // a script can detect "this is the local-only view" without
        // re-issuing the call. Keeps parity with the ability handler.
        let mut envelope = resp;
        envelope["nodes"] = Value::Array(filtered);
        println!("{}", serde_json::to_string_pretty(&envelope)?);
        return Ok(());
    }

    if filtered.is_empty() {
        if args.state == "all" {
            output::info("No devices found.");
        } else {
            output::info(&format!(
                "No {} devices found. Use `--state all` to include all states.",
                args.state
            ));
        }
        return Ok(());
    }

    // Header
    println!(
        "  {} {}",
        style(format!("{}", filtered.len())).bold(),
        if filtered.len() == 1 {
            "device"
        } else {
            "devices"
        }
    );
    println!();

    for n in &filtered {
        print_device(n, &current_node_id);
    }

    // Surface the federation view limitation as a footer when
    // active. A daemon that reports `local_only` is not a bug — it
    // just hasn't joined a federation yet (or the federation Invoke
    // replacement isn't published) — but the operator should know
    // why the list is short.
    if let Some(view) = resp_field(&resp, "federation_view") {
        if view == "local_only" {
            if let Some(reason) = resp_field(&resp, "federation_view_reason") {
                println!();
                output::info(&format!("federation view: local-only — {reason}"));
            }
        }
    }

    Ok(())
}

fn resp_field(_resp: &Value, _key: &str) -> Option<String> {
    // Re-fetch via direct lookup; kept as a helper so a future
    // envelope-shape change touches one place.
    _resp.get(_key).and_then(Value::as_str).map(str::to_string)
}

fn print_device(n: &Value, current_node_id: &str) {
    let node_id = n.get("node_id").and_then(|v| v.as_str()).unwrap_or("-");
    let is_current = !current_node_id.is_empty() && node_id == current_node_id;
    let is_self = n.get("is_self") == Some(&Value::Bool(true));
    let state_display = node::node_state_str(n);
    let online = node::is_online(n);
    let name = device_display_name(n, node_id);
    let (platform, os_detail, hardware_model) = device_platform_info(n);
    let last_active = device_last_active(n);

    let indicator = if online {
        format!("{}", style("●").green())
    } else {
        format!("{}", style("○").dim())
    };
    let state_styled = style_state(&state_display);
    let current_tag = if is_current || is_self {
        format!("  {}", style("← this device").cyan())
    } else {
        String::new()
    };
    println!(
        "  {} {}  {}{}",
        indicator,
        style(name).bold(),
        state_styled,
        current_tag
    );

    let mut details: Vec<String> = Vec::new();
    if !platform.is_empty() && platform != "—" {
        details.push(platform);
    }
    if !os_detail.is_empty() && hardware_model.is_empty() {
        details.push(os_detail);
    }
    if let Some(label) = node::federation_label(n) {
        details.push(format!("via {label}"));
    }
    if !last_active.is_empty() {
        details.push(format!("Active {last_active}"));
    }
    if !details.is_empty() {
        println!("    {}", style(details.join("  ·  ")).dim());
    }

    println!("    {}", style(node_id).dim());
    println!();
}

fn device_display_name<'a>(n: &'a Value, node_id: &'a str) -> &'a str {
    let display_name = n
        .get("display_name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let short_id = if node_id.starts_with("en-") && node_id.len() > SHORT_NODE_ID_LEN {
        node_id.get(..SHORT_NODE_ID_LEN).unwrap_or(node_id)
    } else {
        node_id
    };
    display_name.unwrap_or(short_id)
}

fn device_platform_info(n: &Value) -> (String, String, String) {
    let device_meta = n.get("device");
    let os = device_meta
        .and_then(|d| d.get("os"))
        .and_then(|v| v.as_str())
        .or_else(|| n.get("os").and_then(|v| v.as_str()))
        .unwrap_or("");
    let os_version = device_meta
        .and_then(|d| d.get("os_version"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let hardware_model = device_meta
        .and_then(|d| d.get("hardware_model"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let arch = device_meta
        .and_then(|d| d.get("architecture"))
        .and_then(|v| v.as_str())
        .or_else(|| n.get("arch").and_then(|v| v.as_str()))
        .unwrap_or("");

    let os_label = node::friendly_os(os);
    let platform = if !hardware_model.is_empty() {
        hardware_model.to_string()
    } else if !arch.is_empty() {
        format!("{os_label} ({arch})")
    } else if !os_label.is_empty() {
        os_label.to_string()
    } else {
        "—".to_string()
    };
    let os_detail = if !os_version.is_empty() && !os_label.is_empty() {
        format!("{os_label} {os_version}")
    } else if !os_label.is_empty() {
        os_label.to_string()
    } else {
        String::new()
    };
    (platform, os_detail, hardware_model.to_string())
}

fn device_last_active(n: &Value) -> String {
    let last_seen = n
        .get("last_seen_unix_ms")
        .and_then(Value::as_i64)
        .or_else(|| n.get("last_heartbeat_unix_ms").and_then(Value::as_i64));
    match last_seen {
        Some(ms) if ms > 0 => output::relative_time(ms),
        _ => String::new(),
    }
}

fn style_state(state: &str) -> String {
    match state {
        "HEALTHY" | "REGISTERED" => format!("{}", style("Online").green()),
        "STANDALONE" => format!("{}", style("Standalone").yellow()),
        "JOINING" => format!("{}", style("Joining").cyan()),
        "PROBATION" => format!("{}", style("Probation").cyan()),
        "SUSPECT" => format!("{}", style("Suspect").yellow()),
        "QUARANTINED" => format!("{}", style("Quarantined").red()),
        "DRAINING" => format!("{}", style("Draining").dim()),
        "REMOVED" => format!("{}", style("Offline").dim()),
        _ => format!("{}", style(state).dim()),
    }
}
