// EasyNet CLI — Ability Group
// ===========================
//
// File: src/cli/groups/ability.rs
// Description: `easynet ability …` — manage *public ability endpoints* on
//              EasyNet. Abilities are the only thing that flows on the
//              network (see ARCHITECTURE.md §4 — OOP visibility rules:
//              `ability` is `public`, `skill` is `private`).
//
// Verbs:
//   list                       List published abilities                    (-> cli::abilities)
//   show <node> <name>         Display one endpoint's contract surface     (NEW)
//   deploy <path> --to <node>  Publish a new ability version               (-> cli::deploy)
//   uninstall <node> <id>      Remove a deployed ability                   (NEW)
//   invoke <node> <name>       Call a public ability                       (-> cli::invoke)
//   exec <node> -- <cmd>       One-shot remote shell (ad-hoc ability)      (-> cli::exec)
//
// Verbs DELIBERATELY ABSENT:
//
//   update    — would conflate three time scales:
//                 (1) signature/SLA bump   (discrete, version event)
//                 (2) graph evolution      (low-frequency, internal)
//                 (3) per-call execution   (realtime)
//               See ARCHITECTURE.md §11 (Three time scales) for why
//               compressing them is a mental-model corruption. Use
//               `ability deploy` to publish a new version.
//
//   logs      — the real artefact for "what happened inside one call" is
//               the ability graph trace, which lives at a layer this PR
//               does not yet model. A naive stdout tail would teach the
//               wrong thing.
//
// Routing note (transitional misalignment):
//   `deploy --to <node>` currently takes a *device node id*, not an agent
//   logical id. Under interpretation C this is a known leak — the public
//   API should resolve `<tenant>/<agent-name>` to its hosting device. The
//   migration is tracked as a deferred item; the CLI shape stays as-is
//   until the SDK exposes agent-id-routed deploy.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::{Args, Subcommand};
use console::style;

use crate::cli::{abilities, deploy, exec, invoke};
use crate::shared::{self, output};

#[derive(Debug, Args)]
pub struct AbilityArgs {
    #[command(subcommand)]
    pub action: AbilityAction,
}

#[derive(Debug, Subcommand)]
pub enum AbilityAction {
    /// List published abilities across the federation.
    List(abilities::AbilitiesArgs),
    /// Show one ability's contract surface (schema, version, description).
    Show(ShowArgs),
    /// Publish an ability version to a hosting device.
    Deploy(deploy::DeployArgs),
    /// Uninstall a previously deployed ability.
    Uninstall(UninstallArgs),
    /// Invoke a public ability on its hosting device.
    Invoke(invoke::InvokeArgs),
    /// Run a one-shot ad-hoc command on a device (ephemeral ability).
    Exec(exec::ExecArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Hosting device node id.
    pub node_id: String,
    /// Ability tool name.
    pub name: String,
    /// Emit raw JSON (the underlying registry record) instead of the
    /// human-readable contract view.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
    /// Hosting device node id.
    pub node_id: String,
    /// Install id (from `ability list` or the deploy receipt).
    pub install_id: String,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

pub fn run(args: AbilityArgs) -> anyhow::Result<()> {
    match args.action {
        AbilityAction::List(a) => abilities::run(a),
        AbilityAction::Show(a) => run_show(a),
        AbilityAction::Deploy(a) => deploy::run(a),
        AbilityAction::Uninstall(a) => run_uninstall(a),
        AbilityAction::Invoke(a) => invoke::run(a),
        AbilityAction::Exec(a) => exec::run(a),
    }
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();
    let tools = br
        .list_mcp_tools(tenant, "", &args.node_id)
        .with_context(|| format!("list_mcp_tools {}", args.node_id))?;
    let tool = tools
        .iter()
        .find(|t| {
            t.get("tool_name")
                .or_else(|| t.get("ability_name"))
                .and_then(|v| v.as_str())
                == Some(args.name.as_str())
        })
        .ok_or_else(|| {
            anyhow::anyhow!("ability '{}' not found on '{}'", args.name, args.node_id)
        })?;

    if args.json {
        println!("{}", serde_json::to_string_pretty(tool)?);
        return Ok(());
    }

    // Human-readable contract surface. We deliberately speak the OOP /
    // "published service endpoint" vocabulary here so the CLI itself
    // teaches the ontology.
    let name = tool
        .get("tool_name")
        .or_else(|| tool.get("ability_name"))
        .and_then(|v| v.as_str())
        .unwrap_or(&args.name);
    let version = tool
        .get("ability_version")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let description = tool
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let state = tool.get("state").and_then(|v| v.as_str()).unwrap_or("ACTIVE");

    eprintln!();
    eprintln!(
        "  {} {}  {}  {}",
        style("●").cyan(),
        style(name).bold(),
        style(version).dim(),
        style(format!("[{state}]")).dim(),
    );
    output::detail("hosted on", &args.node_id);
    if !description.is_empty() {
        output::detail("description", description);
    }
    if let Some(schema) = tool.get("input_schema") {
        eprintln!();
        eprintln!("  {}", style("input schema").dim());
        println!("{}", serde_json::to_string_pretty(schema)?);
    }
    eprintln!();
    eprintln!(
        "  {}",
        style(
            "This is a public method on a network actor. Its internal \
             implementation is a self-evolving graph (memory + workflow) \
             that you cannot inspect from outside — by design. To see what \
             happened during a specific call, look at the corresponding \
             mission run's ability_graph_traces."
        )
        .dim()
    );
    eprintln!();
    Ok(())
}

fn run_uninstall(args: UninstallArgs) -> anyhow::Result<()> {
    if !args.yes {
        let prompt = format!(
            "Uninstall ability install '{}' from device '{}'?",
            args.install_id, args.node_id
        );
        if !output::confirm(&prompt)? {
            output::info("aborted");
            return Ok(());
        }
    }

    let (br, rt) = shared::connect_bridge()?;
    let tenant = rt.tenant_or_default();
    let result = br
        .uninstall_capability_with_reason(
            tenant,
            &args.node_id,
            &args.install_id,
            "removed via easynet ability uninstall",
        )
        .with_context(|| format!("uninstall {} on {}", args.install_id, args.node_id))?;
    output::success(&format!(
        "uninstalled {} on {}",
        args.install_id, args.node_id
    ));
    if !result.is_null() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}
