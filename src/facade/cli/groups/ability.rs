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
//   list                        List published abilities                   (-> cli::abilities)
//   show <node> <name>          Display one endpoint's contract surface    (NEW)
//   new <name> [--lang LANG]    Scaffold a new ability project             (-> cli::ability_scaffold)
//   validate <path>             Lint an ability manifest before deploy     (-> cli::ability_scaffold)
//   deploy <path> --node <id>   Publish a new ability version              (-> cli::deploy)
//   uninstall <node> <id>       Remove a deployed ability                  (NEW)
//   invoke <name> [--node <id>] Call a public ability (auto-routes by      (-> cli::invoke)
//                               default; --node pins to a specific device)
//   exec <node> -- <cmd>        One-shot remote shell (ad-hoc ability)     (-> cli::exec)
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
//   `deploy --node <id>` currently takes a *device node id*, not an
//   agent logical id. Under interpretation C this is a known leak —
//   the public API should resolve `<tenant>/<agent-name>` to its
//   hosting device. The migration is tracked as a deferred item; the
//   CLI shape stays as-is until the SDK exposes agent-id-routed
//   deploy.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::{Args, Subcommand};
use console::style;
use serde_json::Value;

use crate::facade::cli::{abilities, ability_scaffold, deploy, exec, invoke};
use crate::support::local_invoke::invoke_local_ability;
use crate::support::output::{self, OutputFormat};

#[derive(Debug, Args)]
pub struct AbilityArgs {
    #[command(subcommand)]
    pub action: AbilityAction,
}

#[derive(Debug, Subcommand)]
pub enum AbilityAction {
    /// Scaffold a new ability project (ability.json + SKILL.md + handler).
    New(ability_scaffold::NewArgs),
    /// Validate an ability directory's manifest before deploy.
    Validate(ability_scaffold::ValidateArgs),
    /// List published abilities across the federation.
    List(abilities::AbilitiesArgs),
    /// Inspect a deployed ability: endpoint name, version, input
    /// schema, runtime state, and hosting device. Use `--format json`
    /// to pipe the raw registry record into other tools.
    Show(ShowArgs),
    /// Publish an ability version to a device (node id today; agent-id routing is planned).
    Deploy(deploy::DeployArgs),
    /// Uninstall a previously deployed ability.
    Uninstall(UninstallArgs),
    /// Invoke a public ability. Auto-routes across the federation unless `--node` pins it.
    Invoke(invoke::InvokeArgs),
    /// Run a one-shot ad-hoc command on a device (ephemeral ability).
    Exec(exec::ExecArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Fully-qualified ability name (e.g. `claude.weather`,
    /// `easynet.discover`, `observe.health`). The bare-verb form
    /// is accepted for system abilities; agent-owned abilities
    /// MUST carry their `<owner>.` prefix.
    pub name: String,
    /// ⚠ Reserved for federation-tier resolution. Today this CLI
    /// pulls metadata from the local daemon's catalogue (post
    /// AXON-RFC-001 P1.5 there is no remote `list_mcp_tools`
    /// surface). Passing `--node` returns a precise error rather
    /// than silently auto-resolving locally.
    #[arg(long, short = 'n', value_name = "NODE_ID")]
    pub node: Option<String>,
    /// Output format. `table` emits the human-readable contract view;
    /// `json` emits the raw underlying registry record. Aligned with
    /// every other list/show command — see `support::output::OutputFormat`.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
    /// Target ability name. Post-P1.5 the only addressable form is
    /// the qualified `<owner>.<verb>`; the historical
    /// `<node_id> <install_id>` shape is preserved as `--node` and
    /// `--install-id` so existing scripts keep parsing while the
    /// federation Invoke replacement is wired.
    pub name: String,
    /// Reserved for federation-tier uninstall. See `--node` on
    /// `ability show`.
    #[arg(long, short = 'n', value_name = "NODE_ID")]
    pub node: Option<String>,
    /// Install id from the deploy receipt. Reserved for the
    /// federation-tier uninstall surface; not consumed today.
    #[arg(long, value_name = "INSTALL_ID")]
    pub install_id: Option<String>,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

pub fn run(args: AbilityArgs) -> anyhow::Result<()> {
    match args.action {
        AbilityAction::New(a) => ability_scaffold::run_new(a),
        AbilityAction::Validate(a) => ability_scaffold::run_validate(a),
        AbilityAction::List(a) => abilities::run(a),
        AbilityAction::Show(a) => run_show(a),
        AbilityAction::Deploy(a) => deploy::run(a),
        AbilityAction::Uninstall(a) => run_uninstall(a),
        AbilityAction::Invoke(a) => invoke::run(a),
        AbilityAction::Exec(a) => exec::run(a),
    }
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    // Joint-plan unified path: `--node` is now wired through
    // `federation.forward_invoke` against the target device URA;
    // `easynet.discover` runs on the peer daemon, the result
    // bridges back, we filter by ability name client-side. Match
    // the routing rules `ability list --node` and `device show`
    // settled on so a single `--node` flag means the same thing
    // across the whole CLI.
    let catalogue = match args.node.as_deref().map(str::trim) {
        None | Some("local") => {
            invoke_local_ability("device.meta.list_abilities", serde_json::json!({}))
                .context("invoke easynet.discover")?
        }
        Some("") => {
            anyhow::bail!(
                "--node was given but empty; omit the flag to show abilities on the \
                 local daemon, or pass `easynet:///r/<realm>/device/<id>` to show \
                 an ability hosted on a peer device."
            );
        }
        Some(node) => invoke_remote_easynet_discover(node)?,
    };
    let abilities = catalogue
        .get("abilities")
        .or_else(|| catalogue.get("tools"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let entry = abilities
        .into_iter()
        .find(|e| {
            e.get("name")
                .or_else(|| e.get("tool_name"))
                .or_else(|| e.get("ability_name"))
                .and_then(Value::as_str)
                == Some(args.name.as_str())
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ability '{}' not found in this node's catalogue. \
                 Run `easynet ability list` to see what is registered.",
                args.name
            )
        })?;

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&entry)?);
        return Ok(());
    }

    // Human-readable contract surface. The fields below are
    // best-effort: `easynet.discover` doesn't yet surface every
    // historical `list_mcp_tools` field (version / state / hosted
    // node), so the renderer falls back to "-" when a field is
    // absent rather than failing — it's a *show* command, missing
    // metadata should still print the rest.
    let name = entry
        .get("name")
        .or_else(|| entry.get("tool_name"))
        .or_else(|| entry.get("ability_name"))
        .and_then(Value::as_str)
        .unwrap_or(&args.name);
    let version = entry
        .get("ability_version")
        .and_then(Value::as_str)
        .unwrap_or("-");
    let description = entry
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let state = entry
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("ACTIVE");
    let owner = entry
        .get("owner_agent_uri")
        .and_then(Value::as_str)
        .or_else(|| {
            // Fall back to deriving owner from the qualified name
            // (`<owner>.<verb>`); aligns with `ability list`'s
            // owner column when discover doesn't surface a URI.
            name.split_once('.').map(|(o, _)| o)
        })
        .unwrap_or("-");

    eprintln!();
    eprintln!(
        "  {} {}  {}  {}",
        style("●").cyan(),
        style(name).bold(),
        style(version).dim(),
        style(format!("[{state}]")).dim(),
    );
    output::detail("owner", owner);
    if !description.is_empty() {
        output::detail("description", description);
    }
    if let Some(schema) = entry
        .get("input_schema")
        .or_else(|| entry.get("schema_summary").and_then(|s| s.get("input")))
    {
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
        let prompt = format!("Uninstall ability '{}' from this fleet?", args.name);
        if !output::confirm(&prompt)? {
            output::info("aborted");
            return Ok(());
        }
    }
    let mut body = serde_json::json!({ "ability_name": args.name });
    if let Some(node) = args.node.as_deref().filter(|s| !s.trim().is_empty()) {
        body["node_id"] = serde_json::json!(node);
    }
    if let Some(iid) = args.install_id.as_deref().filter(|s| !s.trim().is_empty()) {
        body["install_id"] = serde_json::json!(iid);
    }
    let result = invoke_local_ability("device.fleet.uninstall_ability", body)
        .context("invoke fleet.uninstall_ability")?;
    output::success(&format!("uninstalled {}", args.name));
    if !result.is_null() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

/// Joint-plan unified path: `easynet ability show --node <URA>`
/// forwards `easynet.discover` to the target device through
/// `federation.forward_invoke`. Mirrors the same helper in
/// `cli/abilities.rs::fetch_remote_catalogue` so a future audit
/// "every CLI surface that asks a peer device for its catalogue"
/// finds one routing pattern in two call sites. Bare UUID targets
/// go through the shared cross-hub directory lookup helper before
/// local-realm fallback.
#[cfg(feature = "axon-pb")]
fn invoke_remote_easynet_discover(node: &str) -> anyhow::Result<Value> {
    let target_uri = crate::support::remote_device::resolve_target_device_uri(node)?;
    let caller_uri = crate::support::remote_device::caller_device_uri_from_credentials();
    crate::support::federation_invoke::invoke_via_federation_forward(
        "easynet.discover",
        serde_json::json!({}),
        &target_uri,
        caller_uri.as_deref(),
    )
    .with_context(|| format!("forward easynet.discover to target={target_uri}"))
}

#[cfg(not(feature = "axon-pb"))]
fn invoke_remote_easynet_discover(node: &str) -> anyhow::Result<Value> {
    Err(crate::support::local_invoke::federation_not_wired_error(
        &format!("showing an ability on remote node {node:?}"),
    ))
}
