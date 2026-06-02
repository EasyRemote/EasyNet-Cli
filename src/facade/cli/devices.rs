// EasyNet CLI — `easynet device list`
// =====================================
//
// File: src/facade/cli/devices.rs
// Description: Read-only enumeration of every device the
//              federation surfaces. Routes through the daemon's
//              `federation.discover` ability — the same surface
//              `easynet federation discover` exposes — and
//              filters the returned `DirectoryEntry` set down to
//              URA-kind = `device` projections.
//
// Why the cut over from `device.node.list`
// -----------------------------------------
// `device.node.list` was the AXON-RFC-001 P1.5 placeholder that
// fanned out only the local probe view + a same-realm
// `federation.resolve` fallback. The joint plan
// (海峰 + 凉冰, 2026-05-03) collapses every cross-device dispatch
// onto `federation.forward_invoke`; for read-only directory
// queries the canonical surface is `federation.discover`. One
// helper, one path; the legacy `device.node.list` arm gets
// removed in the cull phase.
//
// Wire shape (post-cut)
// ---------------------
// `federation.discover` returns `{ entries: [DirectoryEntry] }`
// per `services::federation_directory::DirectoryEntry`:
//
//   {
//     agent_ura: "easynet:///r/<realm>/device/<id>",
//     node_id: "<id>",
//     display_name: <Option<String>>,
//     status: "active" | "stale" | "draining",
//     origin_realm: <Option<String>>,
//     hub_endpoint: <Option<String>>,
//     last_seen_unix_ms: <Option<i64>>,
//   }
//
// We project this into the row shape the existing renderer
// expects (`node_id`, `state`, `online`, `last_seen_unix_ms`,
// `is_self`, `agent_ura`, …) so the print path is unchanged.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#[cfg(feature = "axon-pb")]
use anyhow::Context;
use clap::Args;
use console::style;
use serde_json::{json, Value};

use crate::persistence::config;
use crate::support::{
    node,
    output::{self, OutputFormat},
};

/// Display length for short device IDs in table output. URA v4.1.4
/// device-ids are bare UUIDs (8-4-4-4-12, total 36 chars); we trim
/// to the leading 8 hex chars + a trailing ellipsis so the table
/// stays scannable.
const SHORT_NODE_ID_LEN: usize = 8;

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
    let creds = config::load_credentials().ok();
    let current_node_id = creds
        .as_ref()
        .map(|c| c.node_id.clone())
        .unwrap_or_default();
    let self_ura = creds
        .as_ref()
        .map(|c| crate::ura::device_ura(&c.tenant_id, &c.node_id));

    let entries = fetch_directory_entries(self_ura.as_deref())?;
    let nodes: Vec<Value> = entries
        .into_iter()
        .filter(is_device_entry)
        .map(|e| project_directory_entry(e, self_ura.as_deref()))
        .collect();

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
        // Wrap in an envelope so a JSON consumer can detect the
        // origin (`federation_view: "federated"`) without re-
        // running the call. The shape is intentionally
        // forward-compatible with the v1 envelope so existing
        // scripts that read `nodes` keep working.
        let envelope = json!({
            "nodes": filtered,
            "federation_view": "federated",
        });
        println!("{}", serde_json::to_string_pretty(&envelope)?);
        return Ok(());
    }

    if filtered.is_empty() {
        if args.state == "all" {
            output::info("No devices found.");
        } else {
            output::info(&format!(
                "No {} devices found. Use '--state all' to include all states.",
                args.state
            ));
        }
        return Ok(());
    }

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

#[cfg(feature = "axon-pb")]
fn fetch_directory_entries(self_ura: Option<&str>) -> anyhow::Result<Vec<Value>> {
    crate::support::federation_invoke::invoke_federation_discover(None, self_ura)
        .context("invoke federation.discover for device list")
}

#[cfg(not(feature = "axon-pb"))]
fn fetch_directory_entries(_self_ura: Option<&str>) -> anyhow::Result<Vec<Value>> {
    Err(crate::support::local_invoke::federation_not_wired_error(
        "listing devices via federation.discover",
    ))
}

/// True when a `DirectoryEntry` projects an URA-kind = `device`
/// agent. Anything else (`agent/<user>.<agent>`, hub URAs,
/// resource URAs) is a non-device row and gets dropped from the
/// device-list view.
fn is_device_entry(entry: &Value) -> bool {
    let ura = entry.get("agent_ura").and_then(Value::as_str).unwrap_or("");
    if ura.is_empty() {
        return false;
    }
    crate::ura::parse_ura(ura)
        .map(|p| !p.device_id.is_empty() && p.agent_id.is_empty())
        .unwrap_or(false)
}

/// Project a `DirectoryEntry` into the row shape `print_device`
/// + `node::is_online` / `node::node_state_str` already consume.
/// The mapping is straight-line: `status: "active"` → `state:
/// "HEALTHY"`, `status: "stale"` → `state: "SUSPECT"` (sweep-
/// candidate), `status: "draining"` → `state: "DRAINING"`. Any
/// other value lands as `state: "UNKNOWN"` rather than crashing
/// the renderer.
fn project_directory_entry(entry: Value, self_ura: Option<&str>) -> Value {
    let agent_ura = entry
        .get("agent_ura")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let node_id = entry
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let display_name = entry
        .get("display_name")
        .and_then(Value::as_str)
        .map(str::to_string);
    let status = entry
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("active");
    let state = match status {
        "active" => "HEALTHY",
        "stale" => "SUSPECT",
        "draining" => "DRAINING",
        _ => "UNKNOWN",
    };
    let online = state == "HEALTHY" || state == "REGISTERED";
    let last_seen_unix_ms = entry.get("last_seen_unix_ms").cloned();
    let origin_realm = entry
        .get("origin_realm")
        .and_then(Value::as_str)
        .map(str::to_string);
    let hub_endpoint = entry
        .get("hub_endpoint")
        .and_then(Value::as_str)
        .map(str::to_string);
    let is_self = self_ura.map(|u| u == agent_ura).unwrap_or(false);

    let mut row = json!({
        "node_id": node_id,
        "agent_ura": agent_ura,
        "display_name": display_name,
        "state": state,
        "online": online,
        "is_self": is_self,
        "paired": true,
    });
    if let Some(v) = last_seen_unix_ms {
        row["last_seen_unix_ms"] = v;
    }
    if let Some(realm) = origin_realm {
        row["tenant_id"] = Value::String(realm);
    }
    if let Some(endpoint) = hub_endpoint {
        row["hub_endpoint"] = Value::String(endpoint);
    }
    row
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
    let short_id = if node_id.len() > SHORT_NODE_ID_LEN {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_device_entry_accepts_canonical_device_ura() {
        let entry = json!({
            "agent_ura": "easynet:///r/easynet.run/device/abc-123",
        });
        assert!(is_device_entry(&entry));
    }

    #[test]
    fn is_device_entry_rejects_agent_ura() {
        let entry = json!({
            "agent_ura": "easynet:///r/easynet.run/agent/alice.claude",
        });
        assert!(!is_device_entry(&entry));
    }

    #[test]
    fn project_maps_active_to_healthy_and_marks_self() {
        let entry = json!({
            "agent_ura": "easynet:///r/r1/device/n1",
            "node_id": "n1",
            "status": "active",
            "origin_realm": "r1",
        });
        let row = project_directory_entry(entry, Some("easynet:///r/r1/device/n1"));
        assert_eq!(row["state"], "HEALTHY");
        assert_eq!(row["online"], true);
        assert_eq!(row["is_self"], true);
        assert_eq!(row["tenant_id"], "r1");
    }

    #[test]
    fn project_maps_stale_to_suspect_offline() {
        let entry = json!({
            "agent_ura": "easynet:///r/r1/device/n2",
            "node_id": "n2",
            "status": "stale",
        });
        let row = project_directory_entry(entry, None);
        assert_eq!(row["state"], "SUSPECT");
        assert_eq!(row["online"], false);
        assert_eq!(row["is_self"], false);
    }
}
