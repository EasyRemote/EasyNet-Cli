// EasyNet CLI — Device Group
// ==========================
//
// File: src/cli/groups/device.rs
// Description: `easynet device …` — every operation that takes a *device*
//              (federation node) as its primary noun.
//
// Verbs:
//   list                List federated devices                (-> cli::devices)
//   show <id>           Inspect one device + its abilities    (NEW)
//   rename <id> <name>  Set the device's display name         (NEW, best-effort)
//   tag <id> ...        Attach metadata tags                  (NEW, best-effort)
//   remove <id>         Drain + deregister a remote device    (NEW)
//   join <token>        Pair *this* host to the federation    (-> cli::join)
//   reset               Un-pair this host                     (-> cli::reset)
//   config [...]        Per-device runtime settings           (-> cli::config_cmd)
//
// Notes on rename/tag:
//   The Axon SDK currently has no first-class rename/tag RPC. Rather than
//   fail loudly, we attempt the operation through a generic
//   `easynet_admin` MCP tool on the target node and surface a clear error
//   if that ability is not deployed there. Users who wire a custom admin
//   ability get the feature for free; everyone else gets a precise
//   "ability not available on this device" message instead of a missing
//   command.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::{Args, Subcommand};
use console::style;
use serde_json::json;

use crate::cli::{config_cmd, devices, join, reset};
use crate::shared::{self, output};

#[derive(Debug, Args)]
pub struct DeviceArgs {
    #[command(subcommand)]
    pub action: DeviceAction,
}

#[derive(Debug, Subcommand)]
pub enum DeviceAction {
    /// List all federated devices.
    List(devices::DevicesArgs),
    /// Show one device's full detail (status, OS, abilities).
    Show(ShowArgs),
    /// Set a device's display name.
    Rename(RenameArgs),
    /// Attach a metadata tag (`key=value`) to a device.
    Tag(TagArgs),
    /// Drain and deregister a remote device from the federation.
    Remove(RemoveArgs),
    /// Pair this host with EasyNet using a join token.
    Join(join::JoinArgs),
    /// Un-pair this host (delete local credentials + state).
    Reset(reset::ResetArgs),
    /// Show or update local device runtime settings.
    Config(config_cmd::ConfigArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Target device node id.
    pub node_id: String,
    /// Emit raw JSON instead of the human-readable view.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RenameArgs {
    /// Target device node id.
    pub node_id: String,
    /// New display name.
    pub name: String,
}

#[derive(Debug, Args)]
pub struct TagArgs {
    /// Target device node id.
    pub node_id: String,
    /// Tag, in `key=value` form. Repeat the flag to set multiple tags.
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub set: Vec<String>,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Target device node id.
    pub node_id: String,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
    /// Reason recorded on the deregister event.
    #[arg(long, default_value = "removed via easynet device remove")]
    pub reason: String,
}

pub fn run(args: DeviceArgs) -> anyhow::Result<()> {
    match args.action {
        DeviceAction::List(a) => devices::run(a),
        DeviceAction::Show(a) => run_show(a),
        DeviceAction::Rename(a) => run_rename(a),
        DeviceAction::Tag(a) => run_tag(a),
        DeviceAction::Remove(a) => run_remove(a),
        DeviceAction::Join(a) => join::run(a),
        DeviceAction::Reset(a) => reset::run(a),
        DeviceAction::Config(a) => config_cmd::run(a),
    }
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();

    let nodes = br.list_nodes(tenant, None).context("list nodes")?;
    let node = nodes
        .iter()
        .find(|n| n.get("node_id").and_then(|v| v.as_str()) == Some(args.node_id.as_str()))
        .ok_or_else(|| anyhow::anyhow!("device '{}' not found", args.node_id))?;

    let abilities = br.list_mcp_tools(tenant, "", &args.node_id).unwrap_or_default();

    if args.json {
        let payload = json!({"node": node, "abilities": abilities});
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    let display_name = node
        .get("display_name")
        .and_then(|v| v.as_str())
        .unwrap_or(&args.node_id);
    let state = node
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN");

    eprintln!();
    eprintln!("  {} {}", style("●").cyan(), style(display_name).bold());
    output::detail("node_id", &args.node_id);
    output::detail("state", state);
    if let Some(os) = node.pointer("/device/os").and_then(|v| v.as_str()) {
        output::detail("os", os);
    }
    if let Some(ver) = node.pointer("/device/os_version").and_then(|v| v.as_str()) {
        output::detail("os_version", ver);
    }
    if let Some(arch) = node.pointer("/device/architecture").and_then(|v| v.as_str()) {
        output::detail("arch", arch);
    }
    if let Some(model) = node.pointer("/device/hardware_model").and_then(|v| v.as_str()) {
        output::detail("model", model);
    }
    if let Some(ms) = node
        .get("last_seen_unix_ms")
        .and_then(serde_json::Value::as_i64)
    {
        output::detail("last_seen", &output::relative_time(ms));
    }

    eprintln!();
    eprintln!(
        "  {} {} {}",
        style(format!("{}", abilities.len())).bold(),
        if abilities.len() == 1 { "ability" } else { "abilities" },
        style("on this device").dim(),
    );
    for a in &abilities {
        let name = a
            .get("tool_name")
            .or_else(|| a.get("ability_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let ver = a
            .get("ability_version")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        eprintln!(
            "    {} {}  {}",
            style("·").dim(),
            style(name).cyan(),
            style(ver).dim(),
        );
    }
    eprintln!();
    Ok(())
}

fn run_rename(args: RenameArgs) -> anyhow::Result<()> {
    invoke_admin(&args.node_id, "rename", json!({"display_name": args.name}))
}

fn run_tag(args: TagArgs) -> anyhow::Result<()> {
    if args.set.is_empty() {
        anyhow::bail!("no --set KEY=VALUE provided");
    }
    let mut tags = serde_json::Map::new();
    for entry in &args.set {
        let (k, v) = entry
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("tag '{entry}' must be KEY=VALUE"))?;
        tags.insert(k.to_string(), json!(v));
    }
    invoke_admin(
        &args.node_id,
        "set_tags",
        json!({"tags": serde_json::Value::Object(tags)}),
    )
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    if !args.yes {
        let prompt = format!(
            "Drain + deregister device '{}' from the federation?",
            args.node_id
        );
        if !output::confirm(&prompt)? {
            output::info("aborted");
            return Ok(());
        }
    }

    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();

    // Drain first so any in-flight calls finish, then deregister.
    let _ = br.drain_node(tenant, &args.node_id, &args.reason);
    br.deregister_node(tenant, &args.node_id, &args.reason)
        .with_context(|| format!("deregister {}", args.node_id))?;

    output::success(&format!("removed {}", args.node_id));
    Ok(())
}

/// Best-effort call into a custom `easynet_admin` ability on the target
/// device. Returns a clear error if the ability isn't deployed there, so
/// users understand the operation requires opt-in tooling on the device side.
fn invoke_admin(node_id: &str, action: &str, payload: serde_json::Value) -> anyhow::Result<()> {
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();
    let mut args = serde_json::Map::new();
    args.insert("action".into(), json!(action));
    if let serde_json::Value::Object(map) = payload {
        for (k, v) in map {
            args.insert(k, v);
        }
    }
    let arguments = serde_json::Value::Object(args);

    match br.call_mcp_tool_with_timeout(
        tenant,
        "easynet_admin",
        node_id,
        &arguments,
        Some(15_000),
    ) {
        Ok(v) => {
            output::success(&format!("{action} on {node_id}"));
            if !v.is_null() {
                println!("{}", serde_json::to_string_pretty(&v)?);
            }
            Ok(())
        }
        Err(e) => {
            anyhow::bail!(
                "device '{node_id}' does not expose 'easynet_admin' (or call failed): {e}\n\
                 Deploy an admin ability with `easynet ability deploy <path> --to {node_id}` \
                 to enable rename/tag operations."
            );
        }
    }
}
