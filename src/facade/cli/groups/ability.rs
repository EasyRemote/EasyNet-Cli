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
//   search <intent>             Rank abilities across the discover ladder  (-> cli::discover,
//                               same impl as top-level `easynet discover`)
//   show <ability-ura>          Display one endpoint's contract surface    (NEW)
//   new <name> [--lang LANG]    Scaffold a new ability project             (-> cli::ability_scaffold)
//   validate <path>             Lint an ability manifest before deploy     (-> cli::ability_scaffold)
//   deploy <path> --node <id>   Publish a new ability version              (-> cli::deploy)
//   uninstall <ability-ura>     Remove a deployed ability                  (NEW)
//   invoke <ability-ura>       Call a public ability by canonical URA      (-> cli::invoke)
//          [--node <id>]       --node pins to a specific remote device
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

use crate::facade::cli::{abilities, ability_scaffold, deploy, discover, exec, invoke};
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
    /// Find abilities by intent — describe what you want done and
    /// get ranked candidates from the discover ladder (same
    /// implementation as top-level `easynet discover`).
    Search(discover::DiscoverArgs),
    /// Inspect a deployed ability: endpoint name, version, input
    /// schema, runtime state, and hosting device. Use `--format json`
    /// to pipe the raw registry record into other tools.
    Show(ShowArgs),
    /// Publish an ability version to a device (node id today; agent-id routing is planned).
    Deploy(deploy::DeployArgs),
    /// Uninstall a previously deployed ability.
    Uninstall(UninstallArgs),
    /// Invoke a public ability by canonical Ability URA.
    Invoke(invoke::InvokeArgs),
    /// Run a one-shot ad-hoc command on a device (ephemeral ability).
    Exec(exec::ExecArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Canonical Ability URA (e.g.
    /// `easynet:///r/localhost/ability/alice.claude.weather`).
    pub ability_ura: String,
    /// ⚠ Reserved for federation-tier resolution. Today this CLI
    /// pulls metadata from the local daemon's catalogue (post
    /// AXON-RFC-001 P1.5 there is no remote 'list_mcp_tools'
    /// surface). Passing '--node' returns a precise error rather
    /// than silently auto-resolving locally.
    #[arg(long, short = 'n', value_name = "NODE_ID")]
    pub node: Option<String>,
    /// Output format. 'table' emits the human-readable contract view;
    /// 'json' emits the raw underlying registry record. Aligned with
    /// every other list/show command — see 'support::output::OutputFormat'.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
    /// Canonical Ability URA to uninstall.
    pub ability_ura: String,
    /// Reserved for federation-tier uninstall. See '--node' on
    /// 'ability show'.
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
        AbilityAction::Search(a) => discover::run(a),
        AbilityAction::Show(a) => run_show(a),
        AbilityAction::Deploy(a) => deploy::run(a),
        AbilityAction::Uninstall(a) => run_uninstall(a),
        AbilityAction::Invoke(a) => invoke::run(a),
        AbilityAction::Exec(a) => exec::run(a),
    }
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    ensure_ability_ura(&args.ability_ura)?;
    // Joint-plan unified path: `--node` is now wired through
    // `federation.forward_invoke` against the target device URA;
    // `meta.list_abilities` runs on the peer daemon, the result
    // bridges back, we filter by ability name client-side. Match
    // the routing rules `ability list --node` and `device show`
    // settled on so a single `--node` flag means the same thing
    // across the whole CLI.
    let catalogue = match args.node.as_deref().map(str::trim) {
        None | Some("local") => invoke_local_ability("meta.list_abilities", serde_json::json!({}))
            .context("invoke meta.list_abilities")?,
        Some("") => {
            anyhow::bail!(
                "--node was given but empty; omit the flag to show abilities on the \
                 local daemon, or pass `easynet:///r/<realm>/device/<id>` to show \
                 an ability hosted on a peer device."
            );
        }
        Some(node) => invoke_remote_list_abilities(node)?,
    };
    let abilities = catalogue
        .get("abilities")
        .or_else(|| catalogue.get("tools"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let entry = abilities
        .into_iter()
        .find(|e| e.get("ability_ura").and_then(Value::as_str) == Some(args.ability_ura.as_str()))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "ability_ura '{}' not found in this node's catalogue. \
                 Run `easynet ability list` to see what is registered.",
                args.ability_ura
            )
        })?;

    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&entry)?);
        return Ok(());
    }

    // Human-readable contract surface. The fields below are
    // best-effort: `meta.list_abilities` doesn't yet surface every
    // historical `list_mcp_tools` field (version / state / hosted
    // node), so the renderer falls back to "-" when a field is
    // absent rather than failing — it's a *show* command, missing
    // metadata should still print the rest.
    let name = entry
        .get("name")
        .or_else(|| entry.get("tool_name"))
        .and_then(Value::as_str)
        .unwrap_or(&args.ability_ura);
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
        .get("owner_ura")
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
    ensure_ability_ura(&args.ability_ura)?;
    if !args.yes {
        let prompt = format!(
            "Uninstall ability '{}' from this device set?",
            args.ability_ura
        );
        if !output::confirm(&prompt)? {
            output::info("aborted");
            return Ok(());
        }
    }
    let mut body = serde_json::json!({ "ability_ura": args.ability_ura.clone() });
    if let Some(node) = args.node.as_deref().filter(|s| !s.trim().is_empty()) {
        body["node_id"] = serde_json::json!(node);
    }
    if let Some(iid) = args.install_id.as_deref().filter(|s| !s.trim().is_empty()) {
        body["install_id"] = serde_json::json!(iid);
    }
    let result =
        invoke_local_ability("ability.uninstall", body).context("invoke ability.uninstall")?;
    output::success(&format!("uninstalled {}", args.ability_ura));
    if !result.is_null() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

fn ensure_ability_ura(value: &str) -> anyhow::Result<()> {
    let parsed = crate::ura::parse_ura(value)
        .map_err(|e| anyhow::anyhow!("expected canonical Ability URA, got {value:?}: {e}"))?;
    if parsed.kind != crate::ura::URAKind::Ability {
        anyhow::bail!("expected canonical Ability URA, got {value:?}");
    }
    Ok(())
}

/// Joint-plan unified path: `easynet ability show --node <URA>`
/// forwards `meta.list_abilities` to the target device through
/// `federation.forward_invoke`. Mirrors the same helper in
/// `cli/abilities.rs::fetch_remote_catalogue` so a future audit
/// "every CLI surface that asks a peer device for its catalogue"
/// finds one routing pattern in two call sites. Bare UUID targets
/// go through the shared cross-hub directory lookup helper before
/// local-realm fallback.
#[cfg(feature = "axon-pb")]
fn invoke_remote_list_abilities(node: &str) -> anyhow::Result<Value> {
    let target_ura = crate::support::remote_device::resolve_target_device_ura(node)?;
    let caller_ura = crate::support::remote_device::caller_device_ura_from_credentials();
    let ability_ura = crate::services::invocation_transport::federation_invoke::TargetOwnedAbilityUra::from_selector(
        &target_ura,
        "meta.list_abilities",
    )?;
    crate::services::invocation_transport::federation_invoke::invoke_via_federation_forward_ability_ura(
        ability_ura.as_str(),
        serde_json::json!({}),
        &target_ura,
        caller_ura.as_deref(),
    )
    .with_context(|| format!("forward meta.list_abilities to target={target_ura}"))
}

#[cfg(not(feature = "axon-pb"))]
fn invoke_remote_list_abilities(node: &str) -> anyhow::Result<Value> {
    Err(crate::support::local_invoke::federation_not_wired_error(
        &format!("showing an ability on remote node {node:?}"),
    ))
}
