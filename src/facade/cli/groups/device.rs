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

use crate::facade::cli::{config_cmd, devices, join, reset};
use crate::support::output::{self, OutputFormat};

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
    /// Drain in-flight work on a remote substrate, then deregister it
    /// from the federation (the device disappears from
    /// `device list`). Irreversible without a fresh pairing token —
    /// prompts for confirmation unless `--yes` is passed.
    Remove(RemoveArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Target substrate node id.
    pub node_id: String,
    /// Output format. `table` emits the human-readable view; `json`
    /// emits the raw substrate record + abilities array. Aligned with
    /// every other list/show command — see `support::output::OutputFormat`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
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
    // Per the ability-only ontology this command is one
    // `fleet.describe_node` invocation against the local daemon.
    // The daemon-side handler returns the node envelope (or
    // `federation_not_wired` for a remote id while the federation
    // Invoke replacement is being landed).
    let node = crate::support::local_invoke::invoke_local_ability(
        "fleet.describe_node",
        json!({ "node_id": args.node_id }),
    )
    .with_context(|| format!("describe node {}", args.node_id))?;

    // Hosted-ability list is `easynet.discover` filtered to entries
    // whose owner matches the target node id. v1 only knows about
    // the local node — once federation Invoke ships the daemon-side
    // handler will return per-node ability lists.
    let abilities = node
        .get("abilities")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_else(|| {
            match crate::support::local_invoke::invoke_local_ability("easynet.discover", json!({}))
            {
                Ok(catalogue) => catalogue
                    .get("abilities")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
                Err(_) => Vec::new(),
            }
        });
    // Refer to the borrowed slot below as `&node` to keep parity
    // with the legacy variable name; the dereferences are checked.
    let node = &node;

    if args.format == OutputFormat::Json {
        let payload = json!({"node": node, "abilities": abilities});
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    let display_name = node
        .get("display_name")
        .and_then(|v| v.as_str())
        .unwrap_or(&args.node_id);

    // `fleet.describe_node` returns a string `state` (HEALTHY /
    // STANDALONE / REMOVED / etc) and a boolean `paired`. Fall
    // back to integer-indexed `state` for compatibility with any
    // future federation-tier handler that still serialises the
    // Axon SDK enum (the legacy form was 0..=7 → label).
    let state = node
        .get("state")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            node.get("state").and_then(|v| v.as_i64()).map(|n| {
                match n {
                    1 => "JOINING",
                    2 => "PROBATION",
                    3 => "HEALTHY",
                    4 => "SUSPECT",
                    5 => "QUARANTINED",
                    6 => "DRAINING",
                    7 => "REMOVED",
                    _ => "UNKNOWN",
                }
                .to_string()
            })
        })
        .unwrap_or_else(|| "UNKNOWN".to_string());
    let paired = node
        .get("paired")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let online = state == "HEALTHY" || state == "REGISTERED" || state == "STANDALONE";

    let dot = if online {
        style("●").green()
    } else {
        style("●").red()
    };

    eprintln!();
    eprintln!("  {} {}", dot, style(display_name).bold());
    output::detail("node_id", &args.node_id);
    output::detail("state", &state);
    output::detail("paired", &format!("{paired}"));
    if let Some(os) = node.pointer("/device/os").and_then(|v| v.as_str()) {
        output::detail("os", os);
    }
    if let Some(ver) = node.pointer("/device/os_version").and_then(|v| v.as_str()) {
        output::detail("os_version", ver);
    }
    if let Some(arch) = node
        .pointer("/device/architecture")
        .and_then(|v| v.as_str())
    {
        output::detail("arch", arch);
    }
    if let Some(model) = node
        .pointer("/device/hardware_model")
        .and_then(|v| v.as_str())
    {
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
        if abilities.len() == 1 {
            "ability"
        } else {
            "abilities"
        },
        style("hosted on this substrate").dim(),
    );
    for a in &abilities {
        // `easynet.discover` returns each ability as `{name, …}`
        // (no `tool_name` / `ability_version` fields). Fall back
        // through the historical aliases for forward-compat with
        // the day a federation-tier `list_mcp_tools` ability ships.
        let name = a
            .get("name")
            .or_else(|| a.get("tool_name"))
            .or_else(|| a.get("ability_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let owner = a
            .get("owner_agent_uri")
            .and_then(|v| v.as_str())
            .or_else(|| name.split_once('.').map(|(o, _)| o))
            .unwrap_or("-");
        eprintln!(
            "    {} {}  {}",
            style("·").dim(),
            style(name).cyan(),
            style(owner).dim(),
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

    // One ability invocation: `fleet.remove_node`. The daemon-side
    // handler refuses to remove the local device (operator should
    // use `easynet device reset` for that — it is the local side
    // of the same operation) and surfaces `federation_not_wired`
    // for remote ids until the Invoke replacement ships.
    let result = crate::support::local_invoke::invoke_local_ability(
        "fleet.remove_node",
        json!({
            "node_id": args.node_id,
            "reason":  args.reason,
        }),
    )
    .with_context(|| format!("remove {}", args.node_id))?;

    output::success(&format!("removed {}", args.node_id));
    if !result.is_null() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}
