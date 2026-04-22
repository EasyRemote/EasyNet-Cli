// EasyNet CLI
// ===========
//
// File: src/cli/devices.rs
// Description: `easynet devices` — lists all nodes across the federation.
//
// Output: colored, modern layout (no heavy table borders) or JSON.
// Filterable by state (online/offline).
// Data: DendriteBridge.list_nodes() returns federated peer nodes via Hub heartbeat sync.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;
use console::style;

use crate::persistence::config;
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
    // First-run UX: if we have no credentials on disk, the user simply
    // hasn't paired this machine yet. `load_and_connect` would raise a
    // low-level "no runtime endpoint" error that points the operator at
    // the wrong problem — the fix is `easynet device join <token>`,
    // not diagnosing a transport. Detect the not-paired state before
    // we reach the bridge and emit a direct, action-oriented message.
    if config::load_credentials().is_err() {
        anyhow::bail!(
            "this device has no credentials yet. Run `easynet device join <token>` to pair it \
             with the Hub (get a pairing token from the Hub dashboard), then retry."
        );
    }

    let (br, rt) = crate::persistence::config::load_and_connect()?;
    let tenant = rt.tenant_or_default();
    let current_node_id = config::load_credentials()
        .map(|c| c.node_id)
        .unwrap_or_default();

    let nodes = br.list_nodes(tenant, None).context("list nodes")?;

    let filtered: Vec<_> = nodes
        .iter()
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
        println!("{}", serde_json::to_string_pretty(&filtered)?);
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

    Ok(())
}

fn print_device(n: &serde_json::Value, current_node_id: &str) {
    let node_id = n.get("node_id").and_then(|v| v.as_str()).unwrap_or("-");
    let is_current = !current_node_id.is_empty() && node_id == current_node_id;
    let state_display = node::node_state_str(n);
    let online = node::is_online(n);
    let name = device_display_name(n, node_id);
    let (platform, os_detail, hardware_model) = device_platform_info(n);
    let last_active = device_last_active(n);

    // Status indicator
    let indicator = if online {
        format!("{}", style("●").green())
    } else {
        format!("{}", style("○").dim())
    };

    let state_styled = style_state(&state_display);

    // Line 1: indicator + name + state + current marker
    let current_tag = if is_current {
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

    // Line 2: details
    let mut details: Vec<String> = Vec::new();
    if !platform.is_empty() && platform != "—" {
        details.push(platform);
    }
    if !os_detail.is_empty() && hardware_model.is_empty() {
        details.push(os_detail);
    }
    // Surface federation topology: for nodes reached through a peer runtime,
    // the `list_nodes` handler stamps the originating runtime's label.
    if let Some(label) = node::federation_label(n) {
        details.push(format!("via {label}"));
    }
    details.push(format!("Active {last_active}"));
    println!("    {}", style(details.join("  ·  ")).dim());

    // Line 3: node ID (dimmed)
    println!("    {}", style(node_id).dim());
    println!();
}

fn device_display_name<'a>(n: &'a serde_json::Value, node_id: &'a str) -> &'a str {
    let display_name = n
        .get("display_name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty());
    let short_id = if node_id.starts_with("en-") && node_id.len() > SHORT_NODE_ID_LEN {
        // Safe: "en-" prefix guarantees first bytes are ASCII.
        node_id.get(..SHORT_NODE_ID_LEN).unwrap_or(node_id)
    } else {
        node_id
    };
    display_name.unwrap_or(short_id)
}

/// Returns (platform, `os_detail`, `hardware_model`) for display.
fn device_platform_info(n: &serde_json::Value) -> (String, String, String) {
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

fn device_last_active(n: &serde_json::Value) -> String {
    let last_seen = n
        .get("last_seen_unix_ms")
        .and_then(serde_json::Value::as_i64)
        .or_else(|| {
            n.get("last_heartbeat_unix_ms")
                .and_then(serde_json::Value::as_i64)
        });
    match last_seen {
        Some(ms) if ms > 0 => output::relative_time(ms),
        _ => "—".to_string(),
    }
}

fn style_state(state: &str) -> String {
    match state {
        "HEALTHY" => format!("{}", style("Online").green()),
        "JOINING" => format!("{}", style("Joining").cyan()),
        "PROBATION" => format!("{}", style("Probation").cyan()),
        "SUSPECT" => format!("{}", style("Suspect").yellow()),
        "QUARANTINED" => format!("{}", style("Quarantined").red()),
        "DRAINING" => format!("{}", style("Draining").dim()),
        "REMOVED" => format!("{}", style("Offline").dim()),
        _ => format!("{}", style(state).dim()),
    }
}
