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
//   deploy <path> --node <ura>  Publish a new ability version              (-> cli::deploy)
//   uninstall <ability-ura>     Remove a deployed ability                  (NEW)
//   invoke <ability-ura>       Call a public ability by canonical URA      (-> cli::invoke)
//   stream <ability-ura>       Call a server-stream ability locally        (-> cli::ability_stream)
//   bidi <ability-ura>         Open a bidirectional ability locally        (-> cli::ability_bidi)
//   record <ability-ura>       Record from a resource-backed stream        (-> cli::ability_record)
//          [--node <ura>]      --node pins to a canonical remote Device URA
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
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::{Args, Subcommand};
use console::style;
use serde_json::Value;
use std::path::PathBuf;

use crate::cli::commands::{
    abilities, ability_bidi, ability_record, ability_scaffold, ability_stream, deploy, discover,
    exec, invoke, teach,
};
use crate::cli::daemon_client::ability_catalog::{AbilityCatalogueClient, AbilityCatalogueQuery};
use crate::support::platform::local_invoke::{
    LocalRemoteDesktopSessionControlBinding, LocalRemoteDesktopSessionIssuer,
    LocalRemoteTargetInventoryIssuer, LocalStreamFrame,
};
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
    /// Publish an ability version to a canonical Device URA.
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
    /// Refresh display/window/application targets for dedicated remote desktop surfaces.
    #[command(name = "refresh-remote-targets", hide = true)]
    RefreshRemoteTargets(RefreshRemoteTargetsArgs),
    /// Watch display/window/application target inventory for dedicated remote desktop surfaces.
    #[command(name = "watch-remote-targets", hide = true)]
    WatchRemoteTargets(WatchRemoteTargetsArgs),
    /// Create a remote desktop session from one selected display/window/application resource.
    #[command(name = "create-remote-desktop-session", hide = true)]
    CreateRemoteDesktopSession(CreateRemoteDesktopSessionArgs),
    /// Apply a remote desktop WebRTC signaling description for a created session.
    #[command(name = "set-remote-desktop-description", hide = true)]
    SetRemoteDesktopDescription(SetRemoteDesktopDescriptionArgs),
    /// Add a remote desktop WebRTC ICE candidate for a created session.
    #[command(name = "add-remote-desktop-ice-candidate", hide = true)]
    AddRemoteDesktopIceCandidate(AddRemoteDesktopIceCandidateArgs),
    /// Watch remote desktop session lifecycle/signaling events.
    #[command(name = "watch-remote-desktop-events", hide = true)]
    WatchRemoteDesktopEvents(WatchRemoteDesktopEventsArgs),
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
    /// Canonical remote Device URA. Omit, or pass `local`, to inspect
    /// this daemon; remote catalogue reads do not repair bare node ids.
    #[arg(long, short = 'n', value_name = "DEVICE_URA")]
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
    /// Target device. `local` resolves to this daemon's canonical Device URA;
    /// remote targets must be canonical Device URAs.
    #[arg(long, short = 'n', value_name = "DEVICE_URA")]
    pub node: Option<String>,
    /// Install id from the deploy receipt. Narrows uninstall to one
    /// deployed bundle when multiple rows share the same ability URA.
    #[arg(long, value_name = "INSTALL_ID")]
    pub install_id: Option<String>,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Debug, Args)]
pub struct RefreshRemoteTargetsArgs {
    /// Restrict refresh output to one or more remote target types.
    #[arg(
        long = "type",
        value_name = "TYPE",
        value_parser = ["display", "application", "window"]
    )]
    pub types: Vec<String>,
    /// Output format. JSON is the frontend contract; table is an operator summary.
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct WatchRemoteTargetsArgs {
    /// Restrict watch output to one or more remote target types.
    #[arg(
        long = "type",
        value_name = "TYPE",
        value_parser = ["display", "application", "window"]
    )]
    pub types: Vec<String>,
    /// Poll interval for the daemon-side live inventory watcher.
    #[arg(long, value_name = "MS")]
    pub poll_interval_ms: Option<u64>,
    /// Maximum stream events to drain before the CLI exits.
    #[arg(long, default_value_t = 1)]
    pub max_events: usize,
    /// Output format. JSON preserves stream frame metadata for frontend tests.
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct CreateRemoteDesktopSessionArgs {
    /// Selected display/window/application Resource URA from refresh/watch remote targets.
    #[arg(long, value_name = "RESOURCE_URA")]
    pub subject: String,
    /// Remote desktop session mode.
    #[arg(long, value_parser = ["view_only", "interactive"])]
    pub mode: Option<String>,
    /// Preferred transport, in priority order.
    #[arg(long = "transport", value_name = "TRANSPORT")]
    pub transport_preferences: Vec<String>,
    /// Requested lease TTL in milliseconds.
    #[arg(long, value_name = "MS")]
    pub lease_ttl_ms: Option<u64>,
    /// Output format. JSON preserves receipt metadata for frontend tests.
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct SetRemoteDesktopDescriptionArgs {
    /// JSON response from create-remote-desktop-session; carries subject, token, and consent receipt.
    #[arg(long, value_name = "PATH")]
    pub session_json: PathBuf,
    /// Description side for remote_desktop.set_description.
    #[arg(long, value_parser = ["local", "remote"])]
    pub side: String,
    /// Inline WebRTC RTCSessionDescription JSON.
    #[arg(long, value_name = "JSON", conflicts_with = "description_json_file")]
    pub description_json: Option<String>,
    /// Path to WebRTC RTCSessionDescription JSON.
    #[arg(long, value_name = "PATH")]
    pub description_json_file: Option<PathBuf>,
    /// Output format. JSON is the host receiver contract.
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct AddRemoteDesktopIceCandidateArgs {
    /// JSON response from create-remote-desktop-session; carries subject, token, and consent receipt.
    #[arg(long, value_name = "PATH")]
    pub session_json: PathBuf,
    /// Inline RTCIceCandidateInit JSON.
    #[arg(long, value_name = "JSON", conflicts_with = "candidate_json_file")]
    pub candidate_json: Option<String>,
    /// Path to RTCIceCandidateInit JSON.
    #[arg(long, value_name = "PATH")]
    pub candidate_json_file: Option<PathBuf>,
    /// Output format. JSON is the host receiver contract.
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct WatchRemoteDesktopEventsArgs {
    /// JSON response from create-remote-desktop-session; carries subject, token, and consent receipt.
    #[arg(long, value_name = "PATH")]
    pub session_json: PathBuf,
    /// Return events strictly after this sequence number.
    #[arg(long, value_name = "SEQ")]
    pub from_sequence: Option<u64>,
    /// Maximum stream frames to drain before the CLI exits.
    #[arg(long, default_value_t = 1)]
    pub max_events: usize,
    /// Output format. JSON preserves stream frame metadata for host tests.
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    pub format: OutputFormat,
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
        AbilityAction::RefreshRemoteTargets(a) => run_refresh_remote_targets(a),
        AbilityAction::WatchRemoteTargets(a) => run_watch_remote_targets(a),
        AbilityAction::CreateRemoteDesktopSession(a) => run_create_remote_desktop_session(a),
        AbilityAction::SetRemoteDesktopDescription(a) => run_set_remote_desktop_description(a),
        AbilityAction::AddRemoteDesktopIceCandidate(a) => run_add_remote_desktop_ice_candidate(a),
        AbilityAction::WatchRemoteDesktopEvents(a) => run_watch_remote_desktop_events(a),
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

    let name = entry
        .get("name")
        .and_then(Value::as_str)
        .expect("schema-bound catalogue row carries name");
    let version = entry
        .get("version")
        .and_then(Value::as_str)
        .expect("schema-bound catalogue row carries version");
    let description = entry
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let state = entry.get("state").and_then(Value::as_str).unwrap_or("-");
    let owner = entry
        .get("owner_ura")
        .and_then(Value::as_str)
        .expect("schema-bound catalogue row carries owner_ura");
    let descriptor_ref = entry
        .get("descriptor_ref")
        .and_then(Value::as_str)
        .expect("schema-bound catalogue row carries descriptor_ref");

    eprintln!();
    eprintln!(
        "  {} {}  {}  {}",
        style("●").cyan(),
        style(name).bold(),
        style(version).dim(),
        style(format!("[{state}]")).dim(),
    );
    output::detail("owner", owner);
    output::detail("descriptor_ref", descriptor_ref);
    if !description.is_empty() {
        output::detail("description", description);
    }
    if let Some(schema) = entry.get("schema_summary").and_then(|s| s.get("input")) {
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

fn run_refresh_remote_targets(args: RefreshRemoteTargetsArgs) -> anyhow::Result<()> {
    let request = refresh_remote_targets_request(&args);
    let response = LocalRemoteTargetInventoryIssuer::refresh_remote_targets(request)
        .context("invoke resource.refresh_remote_targets")?;
    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }

    let resources = response
        .get("resources")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            anyhow::anyhow!("resource.refresh_remote_targets response missing resources array")
        })?;
    output::success(&format!("refreshed {} remote targets", resources.len()));
    if let Some(observed_at_ms) = response.get("observed_at_ms").and_then(Value::as_u64) {
        output::detail("observed_at_ms", &observed_at_ms.to_string());
    }
    if let Some(freshness_ttl_ms) = response.get("freshness_ttl_ms").and_then(Value::as_u64) {
        output::detail("freshness_ttl_ms", &freshness_ttl_ms.to_string());
    }
    for resource in resources {
        let kind = resource.get("type").and_then(Value::as_str).unwrap_or("-");
        let name = resource
            .get("display_name")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let ura = resource
            .get("resource_ura")
            .and_then(Value::as_str)
            .unwrap_or("-");
        println!("{kind}\t{name}\t{ura}");
    }
    Ok(())
}

fn refresh_remote_targets_request(args: &RefreshRemoteTargetsArgs) -> Value {
    if args.types.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::json!({ "types": args.types.clone() })
    }
}

fn run_watch_remote_targets(args: WatchRemoteTargetsArgs) -> anyhow::Result<()> {
    if args.max_events == 0 {
        anyhow::bail!("--max-events must be greater than zero");
    }
    let request = watch_remote_targets_request(&args);
    let frames =
        LocalRemoteTargetInventoryIssuer::watch_remote_targets(request, Some(args.max_events))
            .context("invoke resource.watch_remote_targets")?;
    if args.format == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&stream_frames_to_json(&frames))?
        );
        return Ok(());
    }

    output::success(&format!("received {} remote target event(s)", frames.len()));
    for frame in frames {
        let event_type = frame
            .payload
            .get("event_type")
            .and_then(Value::as_str)
            .unwrap_or("-");
        let resources = frame
            .payload
            .get("resources")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        println!(
            "{}\t{}\t{} resource(s)",
            frame.sequence, event_type, resources
        );
    }
    Ok(())
}

fn watch_remote_targets_request(args: &WatchRemoteTargetsArgs) -> Value {
    let mut object = serde_json::Map::new();
    if !args.types.is_empty() {
        object.insert("types".to_string(), serde_json::json!(args.types));
    }
    if let Some(poll_interval_ms) = args.poll_interval_ms {
        object.insert(
            "poll_interval_ms".to_string(),
            serde_json::json!(poll_interval_ms),
        );
    }
    object.insert("max_events".to_string(), serde_json::json!(args.max_events));
    Value::Object(object)
}

fn stream_frames_to_json(frames: &[LocalStreamFrame]) -> Value {
    Value::Array(
        frames
            .iter()
            .map(|frame| {
                serde_json::json!({
                    "sequence": frame.sequence,
                    "content_type": frame.content_type,
                    "terminal": frame.terminal,
                    "payload": frame.payload,
                })
            })
            .collect(),
    )
}

fn run_create_remote_desktop_session(args: CreateRemoteDesktopSessionArgs) -> anyhow::Result<()> {
    let request = create_remote_desktop_session_request(&args);
    let (session, invocation) =
        LocalRemoteDesktopSessionIssuer::create_session(&args.subject, request)
            .context("invoke remote_desktop.grant_consent -> remote_desktop.create_session")?;
    let response = serde_json::json!({
        "session": session,
        "invocation": invocation.as_value(),
    });
    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }

    output::success("created remote desktop session");
    if let Some(session_id) = response
        .get("session")
        .and_then(|session| session.get("session_id"))
        .and_then(Value::as_str)
    {
        output::detail("session_id", session_id);
    }
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn create_remote_desktop_session_request(args: &CreateRemoteDesktopSessionArgs) -> Value {
    let mut object = serde_json::Map::new();
    if let Some(mode) = args.mode.as_ref() {
        object.insert("mode".to_string(), serde_json::json!(mode));
    }
    if !args.transport_preferences.is_empty() {
        object.insert(
            "transport_preferences".to_string(),
            serde_json::json!(args.transport_preferences),
        );
    }
    if let Some(lease_ttl_ms) = args.lease_ttl_ms {
        object.insert("lease_ttl_ms".to_string(), serde_json::json!(lease_ttl_ms));
    }
    Value::Object(object)
}

fn run_set_remote_desktop_description(args: SetRemoteDesktopDescriptionArgs) -> anyhow::Result<()> {
    let binding = remote_desktop_session_control_binding_from_file(&args.session_json)?;
    let request = set_remote_desktop_description_request(&args)?;
    let response = LocalRemoteDesktopSessionIssuer::set_description(&binding, request)
        .context("invoke remote_desktop.set_description")?;
    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }

    output::success("set remote desktop description");
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn set_remote_desktop_description_request(
    args: &SetRemoteDesktopDescriptionArgs,
) -> anyhow::Result<Value> {
    Ok(serde_json::json!({
        "side": args.side,
        "description": json_value_from_inline_or_file(
            "remote_desktop.set_description description",
            args.description_json.as_deref(),
            args.description_json_file.as_ref(),
        )?,
    }))
}

fn run_add_remote_desktop_ice_candidate(
    args: AddRemoteDesktopIceCandidateArgs,
) -> anyhow::Result<()> {
    let binding = remote_desktop_session_control_binding_from_file(&args.session_json)?;
    let request = add_remote_desktop_ice_candidate_request(&args)?;
    let response = LocalRemoteDesktopSessionIssuer::add_ice_candidate(&binding, request)
        .context("invoke remote_desktop.add_ice_candidate")?;
    if args.format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&response)?);
        return Ok(());
    }

    output::success("added remote desktop ICE candidate");
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

fn add_remote_desktop_ice_candidate_request(
    args: &AddRemoteDesktopIceCandidateArgs,
) -> anyhow::Result<Value> {
    Ok(serde_json::json!({
        "candidate": json_value_from_inline_or_file(
            "remote_desktop.add_ice_candidate candidate",
            args.candidate_json.as_deref(),
            args.candidate_json_file.as_ref(),
        )?,
    }))
}

fn run_watch_remote_desktop_events(args: WatchRemoteDesktopEventsArgs) -> anyhow::Result<()> {
    if args.max_events == 0 {
        anyhow::bail!("--max-events must be greater than zero");
    }
    let binding = remote_desktop_session_control_binding_from_file(&args.session_json)?;
    let request = watch_remote_desktop_events_request(&args);
    let frames =
        LocalRemoteDesktopSessionIssuer::watch_events(&binding, request, Some(args.max_events))
            .context("invoke remote_desktop.watch_events")?;
    if args.format == OutputFormat::Json {
        println!(
            "{}",
            serde_json::to_string_pretty(&stream_frames_to_json(&frames))?
        );
        return Ok(());
    }

    output::success(&format!(
        "received {} remote desktop event frame(s)",
        frames.len()
    ));
    for frame in frames {
        let event_type = frame
            .payload
            .get("event_type")
            .or_else(|| frame.payload.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("-");
        println!("{}\t{}", frame.sequence, event_type);
    }
    Ok(())
}

fn watch_remote_desktop_events_request(args: &WatchRemoteDesktopEventsArgs) -> Value {
    let mut object = serde_json::Map::new();
    if let Some(from_sequence) = args.from_sequence {
        object.insert(
            "from_sequence".to_string(),
            serde_json::json!(from_sequence),
        );
    }
    Value::Object(object)
}

fn remote_desktop_session_control_binding_from_file(
    path: &PathBuf,
) -> anyhow::Result<LocalRemoteDesktopSessionControlBinding> {
    let value = json_file(path)
        .with_context(|| format!("read remote desktop session JSON from {}", path.display()))?;
    LocalRemoteDesktopSessionControlBinding::from_create_session_response(&value)
}

fn json_value_from_inline_or_file(
    label: &'static str,
    inline: Option<&str>,
    file: Option<&PathBuf>,
) -> anyhow::Result<Value> {
    match (
        inline.map(str::trim).filter(|value| !value.is_empty()),
        file,
    ) {
        (Some(raw), None) => {
            serde_json::from_str(raw).with_context(|| format!("parse inline {label} JSON"))
        }
        (None, Some(path)) => {
            json_file(path).with_context(|| format!("read {label} JSON from {}", path.display()))
        }
        (Some(_), Some(_)) => {
            anyhow::bail!("{label} accepts either inline JSON or file JSON, not both")
        }
        (None, None) => anyhow::bail!("{label} requires inline JSON or file JSON"),
    }
}

fn json_file(path: &PathBuf) -> anyhow::Result<Value> {
    let raw = std::fs::read_to_string(path)?;
    serde_json::from_str(&raw).with_context(|| format!("parse JSON from {}", path.display()))
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
    let target_ura = ability_uninstall_target_ura(&args)?;
    let identity = crate::support::platform::remote_device::PairedInvocationIdentity::load(
        "ability uninstall",
    )?;
    let result = crate::cli::daemon_client::remote_system_ability::invoke_target_ability_uninstall(
        &target_ura,
        identity.caller_user_ura(),
        &args.ability_ura,
        args.install_id.as_deref(),
    )
    .context("invoke ability.uninstall")?;
    output::success(&format!("uninstalled {}", args.ability_ura));
    if !result.is_null() {
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    Ok(())
}

fn ability_uninstall_target_ura(args: &UninstallArgs) -> anyhow::Result<String> {
    crate::support::platform::remote_device::resolve_cli_device_target_ura(
        args.node.as_deref(),
        "ability uninstall",
    )
}

fn ensure_ability_ura(value: &str) -> anyhow::Result<()> {
    let parsed = crate::core::ura::parse_ura(value)
        .map_err(|e| anyhow::anyhow!("expected canonical Ability URA, got {value:?}: {e}"))?;
    if parsed.kind != crate::core::ura::URAKind::Ability {
        anyhow::bail!("expected canonical Ability URA, got {value:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ability_uninstall_uses_canonical_target_ura() {
        let target = ability_uninstall_target_ura(&UninstallArgs {
            ability_ura:
                "easynet:///r/test/ability/system-agent.dev.ability-management.example.run"
                    .to_string(),
            node: Some("easynet:///r/test/device/dev-node".to_string()),
            install_id: Some("install-123".to_string()),
            yes: true,
        })
        .expect("canonical uninstall target");

        assert_eq!(target, "easynet:///r/test/device/dev-node");
    }

    #[test]
    fn ability_uninstall_payload_rejects_blank_target_before_daemon_payload() {
        let err = ability_uninstall_target_ura(&UninstallArgs {
            ability_ura:
                "easynet:///r/test/ability/system-agent.dev.ability-management.example.run"
                    .to_string(),
            node: Some("  ".to_string()),
            install_id: Some("".to_string()),
            yes: true,
        })
        .expect_err("blank target must fail before daemon payload");
        assert!(err.to_string().contains("target must not be empty"));
    }

    #[test]
    fn refresh_remote_targets_request_omits_empty_type_filter() {
        let request = refresh_remote_targets_request(&RefreshRemoteTargetsArgs {
            types: Vec::new(),
            format: OutputFormat::Json,
        });

        assert_eq!(request, serde_json::json!({}));
    }

    #[test]
    fn refresh_remote_targets_request_preserves_picker_type_filter() {
        let request = refresh_remote_targets_request(&RefreshRemoteTargetsArgs {
            types: vec!["window".to_string(), "application".to_string()],
            format: OutputFormat::Json,
        });

        assert_eq!(
            request,
            serde_json::json!({"types": ["window", "application"]})
        );
    }

    #[test]
    fn watch_remote_targets_request_preserves_stream_picker_controls() {
        let request = watch_remote_targets_request(&WatchRemoteTargetsArgs {
            types: vec!["window".to_string()],
            poll_interval_ms: Some(250),
            max_events: 3,
            format: OutputFormat::Json,
        });

        assert_eq!(
            request,
            serde_json::json!({
                "types": ["window"],
                "poll_interval_ms": 250,
                "max_events": 3,
            })
        );
    }

    #[test]
    fn stream_frames_to_json_preserves_transport_metadata() {
        let frames = vec![LocalStreamFrame {
            sequence: 7,
            content_type: "application/json".to_string(),
            terminal: false,
            payload: serde_json::json!({
                "event_type": "target_inventory_snapshot",
                "resources": [],
            }),
        }];

        assert_eq!(
            stream_frames_to_json(&frames),
            serde_json::json!([{
                "sequence": 7,
                "content_type": "application/json",
                "terminal": false,
                "payload": {
                    "event_type": "target_inventory_snapshot",
                    "resources": [],
                },
            }])
        );
    }

    #[test]
    fn create_remote_desktop_session_request_keeps_selected_subject_out_of_args() {
        let request = create_remote_desktop_session_request(&CreateRemoteDesktopSessionArgs {
            subject: "easynet:///r/test/resource/device.dev/streams/window.7".to_string(),
            mode: Some("view_only".to_string()),
            transport_preferences: vec!["webrtc".to_string()],
            lease_ttl_ms: Some(30_000),
            format: OutputFormat::Json,
        });

        assert_eq!(
            request,
            serde_json::json!({
                "mode": "view_only",
                "transport_preferences": ["webrtc"],
                "lease_ttl_ms": 30000,
            })
        );
        assert!(request.get("subject").is_none());
        assert!(request.get("resource_ura").is_none());
    }

    #[test]
    fn set_remote_desktop_description_request_keeps_session_fields_out_of_args() {
        let request = set_remote_desktop_description_request(&SetRemoteDesktopDescriptionArgs {
            session_json: PathBuf::from("session.json"),
            side: "remote".to_string(),
            description_json: Some(r#"{"type":"offer","sdp":"v=0\r\n"}"#.to_string()),
            description_json_file: None,
            format: OutputFormat::Json,
        })
        .expect("description request");

        assert_eq!(request["side"], serde_json::json!("remote"));
        assert_eq!(request["description"]["type"], serde_json::json!("offer"));
        assert!(request.get("subject").is_none());
        assert!(request.get("resource_ura").is_none());
        assert!(request.get("session_id").is_none());
        assert!(request.get("session_token").is_none());
    }

    #[test]
    fn add_remote_desktop_ice_candidate_request_keeps_session_fields_out_of_args() {
        let request = add_remote_desktop_ice_candidate_request(&AddRemoteDesktopIceCandidateArgs {
            session_json: PathBuf::from("session.json"),
            candidate_json: Some(
                r#"{"candidate":"candidate:1 1 UDP 1 127.0.0.1 9 typ host"}"#.to_string(),
            ),
            candidate_json_file: None,
            format: OutputFormat::Json,
        })
        .expect("candidate request");

        assert!(request["candidate"]["candidate"]
            .as_str()
            .unwrap()
            .starts_with("candidate:"));
        assert!(request.get("subject").is_none());
        assert!(request.get("resource_ura").is_none());
        assert!(request.get("session_id").is_none());
        assert!(request.get("session_token").is_none());
    }

    #[test]
    fn watch_remote_desktop_events_request_preserves_resume_without_session_fields() {
        let request = watch_remote_desktop_events_request(&WatchRemoteDesktopEventsArgs {
            session_json: PathBuf::from("session.json"),
            from_sequence: Some(41),
            max_events: 2,
            format: OutputFormat::Json,
        });

        assert_eq!(
            request,
            serde_json::json!({
                "from_sequence": 41,
            })
        );
        assert!(request.get("subject").is_none());
        assert!(request.get("resource_ura").is_none());
        assert!(request.get("session_id").is_none());
        assert!(request.get("session_token").is_none());
    }
}
