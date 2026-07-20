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
//   stream <ability-ura>       Call a server-stream ability locally        (-> cli::ability_stream)
//   bidi <ability-ura>         Open a bidirectional ability locally        (-> cli::ability_bidi)
//   record <ability-ura>       Record from a resource-backed stream        (-> cli::ability_record)
//          [--node <id>]       --node pins to a specific remote device
//   exec <node> -- <cmd>        One-shot remote shell (ad-hoc ability)     (-> cli::exec)
//   teach <agent.name> --to U   Grant descriptor import to ONE agent       (-> cli::teach)
//   learn <ability-ura> --as A  Import a granted descriptor into A         (-> cli::teach)
//   forget <name> --agent A     Remove an imported descriptor              (-> cli::teach)
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
//   pull      — descriptor import is owner-initiated, never consumer-
//               initiated: an owner grants a declaration-only descriptor
//               (`teach`/`learn`, with `allow_transferred_code = false` by
//               default), while executable capability remains at the owner.
//               A `pull` verb would imply code installation; the default GET
//               route is remote `invoke`. See
//               docs/spec/seven-axes-p0-landing-v1.md §2.5 / §0.1-6.
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

use crate::cli::commands::{
    abilities, ability_bidi, ability_record, ability_scaffold, ability_stream, deploy, discover,
    exec, invoke, teach,
};
use crate::cli::daemon_client::ability_catalog::{AbilityCatalogueClient, AbilityCatalogueQuery};
use crate::support::platform::local_invoke::invoke_local_ability;
use crate::support::platform::output::{self, OutputFormat};

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
    /// Invoke a public server-stream ability by canonical Ability URA.
    Stream(ability_stream::StreamArgs),
    /// Open a public bidirectional ability by canonical Ability URA.
    Bidi(ability_bidi::BidiArgs),
    /// Ergonomic wrapper for resource-backed recording streams.
    Record(ability_record::RecordArgs),
    /// Run a one-shot ad-hoc command on a device (ephemeral ability).
    Exec(exec::ExecArgs),
    /// Grant one agent permission to import a declaration-only descriptor.
    Teach(teach::TeachArgs),
    /// Import a granted descriptor; this does not install executable code.
    Learn(teach::LearnArgs),
    /// Drop an imported descriptor (native abilities are not forgettable).
    Forget(teach::ForgetArgs),
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Canonical Ability URA (e.g.
    /// `easynet:///r/localhost/ability/alice.claude.weather`).
    pub ability_ura: String,
    /// Target node id or canonical device URA. Omit, or pass `local`,
    /// to inspect this daemon; any other value is resolved through the
    /// canonical remote catalogue path.
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
    /// Target node id. `local` or this device id uninstall locally;
    /// remote uninstall keeps this CLI shape but currently returns the
    /// daemon's typed `federation_not_wired` error.
    #[arg(long, short = 'n', value_name = "NODE_ID")]
    pub node: Option<String>,
    /// Install id from the deploy receipt. Narrows uninstall to one
    /// deployed bundle when multiple rows share the same ability URA.
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
        AbilityAction::Stream(a) => ability_stream::run(a),
        AbilityAction::Bidi(a) => ability_bidi::run(a),
        AbilityAction::Record(a) => ability_record::run(a),
        AbilityAction::Exec(a) => exec::run(a),
        AbilityAction::Teach(a) => teach::run_teach(a),
        AbilityAction::Learn(a) => teach::run_learn(a),
        AbilityAction::Forget(a) => teach::run_forget(a),
    }
}

fn run_show(args: ShowArgs) -> anyhow::Result<()> {
    ensure_ability_ura(&args.ability_ura)?;
    let client = AbilityCatalogueClient::new(AbilityCatalogueQuery::default());
    // Joint-plan unified path: `--node` is now wired through
    // the canonical `Invocation::Invoke` RPC against the target device URA;
    // `meta.list_abilities` runs on the peer daemon, the result
    // bridges back, we filter by ability name client-side. Match
    // the routing rules `ability list --node` and `device show`
    // settled on so a single `--node` flag means the same thing
    // across the whole CLI.
    let catalogue = match args.node.as_deref().map(str::trim) {
        None | Some("local") => client
            .fetch_local_value()
            .context("invoke meta.list_abilities")?,
        Some("") => {
            anyhow::bail!(
                "--node was given but empty; omit the flag to show abilities on the \
                 local daemon, or pass `easynet:///r/<realm>/device/<id>` to show \
                an ability hosted on a peer device."
            );
        }
        Some(node) => {
            let action = format!("showing an ability on remote node {node:?}");
            client.fetch_remote_value(node, &action)?
        }
    };
    let abilities = AbilityCatalogueClient::abilities_from_value(&catalogue)?;
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
        .unwrap_or("UNKNOWN");
    let owner = entry
        .get("owner_ura")
        .and_then(Value::as_str)
        .or_else(|| {
            // Fall back to deriving owner from the qualified name
            // (`<owner>.<verb>`); aligns with `ability list`'s
            // owner column when discover doesn't surface a URA.
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
        let target = args.node.as_deref().unwrap_or("local");
        let prompt = format!("Uninstall ability '{}' from {target}?", args.ability_ura);
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
    let parsed = crate::core::ura::parse_ura(value)
        .map_err(|e| anyhow::anyhow!("expected canonical Ability URA, got {value:?}: {e}"))?;
    if parsed.kind != crate::core::ura::URAKind::Ability {
        anyhow::bail!("expected canonical Ability URA, got {value:?}");
    }
    Ok(())
}
