// EasyNet CLI — Device Group
// ==========================
//
// File: src/cli/groups/device.rs
// Description: `easynet device …` — manage *hosting substrates*.
//
// Ontology note (interpretation C, see ARCHITECTURE.md §6):
//
//   A device is NOT a network first-class entity. The only network
//   first-class objects are Agents (network actors) and Abilities (their
//   public methods). A device is the *physical substrate* on which agents
//   are hosted — analogous to an OS to a process. Devices are visible to
//   the CLI for diagnostic and lifecycle reasons (pairing, hardware
//   inventory, capacity), but they are NOT addressable as the "to" of an
//   ability call from outside.
//
// Verbs:
//   join <token>      Pair THIS host as a substrate                       (-> cli::join)
//   reset             Un-pair this host                                   (-> cli::reset)
//   config            Per-host runtime settings                           (-> cli::config_cmd)
//                     (semantically belongs to `runtime config`; physical
//                      command stays here for now — see ARCHITECTURE.md §8 #6)
//   list              List substrates known to the federation             (-> cli::devices)
//   show <id>         Inspect one substrate (hardware, hosted abilities)  (NEW)
//   remove <id>       Drain + deregister a remote substrate               (NEW)
//
// Verbs DELIBERATELY ABSENT:
//
//   rename / tag — would require an `easynet_admin` ability deployed on
//                  the target substrate. That ability does not exist in
//                  this PR, so the verbs would either silently fail or
//                  succeed half-way. Skipping them is the honest choice.
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
    /// Pair this host as a hosting substrate (registers credentials).
    Join(join::JoinArgs),
    /// Un-pair this host (delete local credentials and state).
    Reset(reset::ResetArgs),
    /// Show or update local runtime settings (will move to `runtime config` in a future release).
    Config(config_cmd::ConfigArgs),
    /// List hosting substrates known to the federation.
    List(devices::DevicesArgs),
    /// Show one substrate's hardware and hosted abilities.
    Show(ShowArgs),
    /// Drain and deregister a remote substrate from the federation.
    Remove(RemoveArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Target substrate node id.
    pub node_id: String,
    /// Emit raw JSON instead of the human-readable view.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Target substrate node id.
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
        DeviceAction::Join(a) => join::run(a),
        DeviceAction::Reset(a) => reset::run(a),
        DeviceAction::Config(a) => config_cmd::run(a),
        DeviceAction::List(a) => devices::run(a),
        DeviceAction::Show(a) => run_show(a),
        DeviceAction::Remove(a) => run_remove(a),
    }
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();

    let nodes = br.list_nodes(tenant, None).context("list nodes")?;
    let node = nodes
        .iter()
        .find(|n| n.get("node_id").and_then(|v| v.as_str()) == Some(args.node_id.as_str()))
        .ok_or_else(|| anyhow::anyhow!("substrate '{}' not found", args.node_id))?;

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

    // state and trust_level are returned as integers from Axon SDK.
    let state = match node.get("state").and_then(|v| v.as_i64()).unwrap_or(0) {
        0 => "UNKNOWN",
        1 => "JOINING",
        2 => "PROBATION",
        3 => "HEALTHY",
        4 => "SUSPECT",
        5 => "QUARANTINED",
        6 => "DRAINING",
        7 => "REMOVED",
        _ => "UNKNOWN",
    };
    let trust = match node.get("trust_level").and_then(|v| v.as_i64()).unwrap_or(0) {
        0 => "UNKNOWN",
        1 => "UNTRUSTED",
        2 => "PROBATION",
        3 => "STANDARD",
        4 => "TRUSTED",
        _ => "UNKNOWN",
    };
    let online = node.get("online").and_then(|v| v.as_bool()).unwrap_or(false);

    let dot = if online {
        style("●").green()
    } else {
        style("●").red()
    };

    eprintln!();
    eprintln!("  {} {}", dot, style(display_name).bold());
    output::detail("node_id", &args.node_id);
    output::detail("state", &format!("{state} (trust: {trust})"));
    output::detail("online", &format!("{online}"));
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
        style("hosted on this substrate").dim(),
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
    eprintln!(
        "  {}",
        style(
            "Reminder: this substrate is not network-addressable on its own. \
             Network calls always target an agent's published ability; the \
             substrate is the place where that ability happens to run."
        )
        .dim()
    );
    eprintln!();
    Ok(())
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    if !args.yes {
        let prompt = format!(
            "Drain and deregister substrate '{}' from the federation?",
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
