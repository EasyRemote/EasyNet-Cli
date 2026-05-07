// EasyNet CLI — Agent Subcommand
// ===============================
//
// File: src/cli/agent.rs
// Description: `easynet agent` — register, manage, and invoke AI agents (Claude Code / Codex).
//
// Subcommands:
//   add <name>     — Register a new agent
//   list           — List registered agents
//   remove <name>  — Remove an agent
//   send <name>    — Send a prompt to an agent and print the response
//   doctor [name]  — Check agent CLI availability, version, auth
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};
use console::style;

use crate::core::agent_spec::{AgentSpec, RuntimeKind};
use crate::facade::cli::mission_runs::{self, MissionRunOpts};
use crate::persistence::config;
use crate::registry::agents::{self, AgentEntry, AgentType, CURRENT_REGISTRY_SCHEMA};
use crate::runtime::directory::{AgentDirectory, Location};
use crate::runtime::drivers::{claude_code, codex};
use crate::support::output;
use crate::support::timeouts;

#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub action: AgentAction,
}

#[derive(Debug, Subcommand)]
pub enum AgentAction {
    /// Register a new AI agent.
    Add(AddArgs),
    /// List registered agents.
    List,
    /// Remove a registered agent.
    Remove(RemoveArgs),
    /// Send a prompt to an agent and print the response.
    Send(SendArgs),
    /// Check agent CLI availability and authentication.
    Doctor(DoctorArgs),
    /// Remove registry rows whose on-disk root has gone missing.
    Prune(PruneArgs),
    /// List the abilities declared under `<agent-root>/abilities/`.
    Abilities(AbilitiesArgs),
    /// Update fields of a registered agent in place. Currently
    /// supports `--model`. Mutates `agent.toml` + the registry
    /// row atomically; preserves label, abilities, skills, runs.
    Set(SetArgs),
    /// Dry-run: show what `<agent>.<ability>` tools would be published,
    /// without touching Axon. Live publishing lands in a later PR.
    Publish(PublishArgs),
    /// Re-run runtime.register_local_tool for every daemon-owned
    /// ability against the live runtime. Use this after authoring a
    /// new `<agent>/abilities/<verb>.ability.toml` to make the new
    /// ability invokable from outside the daemon (the in-daemon
    /// dispatcher's fallback resolver picks up new TOMLs automatically
    /// for in-process invocation; this command propagates the same
    /// view to axon-runtime so cross-process Invokes route correctly).
    /// No daemon restart required.
    Refresh,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// Agent name (e.g. "claude", "codex", "my-agent")
    pub name: String,
    /// Agent type
    #[arg(long, value_name = "TYPE")]
    pub r#type: String,
    /// Model to use (e.g. "sonnet", "gpt-5.2")
    #[arg(long)]
    pub model: Option<String>,
    /// Human-readable label
    #[arg(long)]
    pub label: Option<String>,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Agent name to remove
    pub name: String,

    /// Also delete the on-disk agent root (agent.toml +
    /// abilities/ + skills/ + memory/ + runs/ + .env +
    /// projected runtime-native files). Without this flag only
    /// the registry row is removed; the directory is kept so
    /// the operator can re-register the same name later and
    /// pick up previous runs / skills. The flag is deliberately
    /// opt-in because `rm -rf` on a directory that carries an
    /// operator's `.env` credentials is a destructive action.
    #[arg(long)]
    pub purge: bool,
}

#[derive(Debug, Args)]
pub struct PruneArgs {
    /// Show what would be removed without mutating the
    /// registry. Pairs well with `agent list`'s "path missing"
    /// rows — an operator running `prune --dry-run` first sees
    /// exactly which rows will disappear.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct SendArgs {
    /// Registered agent name (from `easynet agent list`).
    pub name: String,
    /// Prompt body sent verbatim to the agent.
    pub prompt: String,
    /// Prior conversation to prepend under a `## Context (previous
    /// discussion)` section. Use this to carry state across separate
    /// `agent send` invocations when you want the agent to build on an
    /// earlier reply without re-pasting the history inline.
    #[arg(long, value_name = "TEXT")]
    pub context: Option<String>,
    /// Per-call deadline in seconds. LLM dispatches can legitimately
    /// take many minutes, so the default is 15 min
    /// (`support::timeouts::AGENT_SEND_DEFAULT_SECS`). `0` inherits the
    /// runtime default.
    #[arg(long, value_name = "SECS", default_value_t = timeouts::AGENT_SEND_DEFAULT_SECS)]
    pub timeout: u64,
    /// Write the raw stream-json trace (one event per line) to this file.
    /// The prompt is saved alongside it as `<file>.prompt.txt`.
    #[arg(long, value_name = "FILE")]
    pub trace: Option<std::path::PathBuf>,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Check a specific agent (or all if omitted)
    pub name: Option<String>,
}

#[derive(Debug, Args)]
pub struct AbilitiesArgs {
    /// Registered agent name (from `easynet agent list`).
    pub name: String,
}

#[derive(Debug, Args)]
pub struct SetArgs {
    /// Registered agent name (from `easynet agent list`).
    pub name: String,
    /// New model identifier. Pass any string the underlying CLI
    /// accepts for `--model` — `claude --model` and `codex
    /// --model` accept aliases (e.g. `sonnet`, `opus`) or full
    /// names (e.g. `claude-opus-4-7`). Neither CLI exposes an
    /// enumeration surface, so we deliberately do NOT validate
    /// the value — that would force EasyNet to ship a stale
    /// allow-list. Validation happens at invocation time when the
    /// underlying CLI sees the flag.
    ///
    /// Pass `--model ''` (empty string) to CLEAR the model and let
    /// the CLI fall back to its own default.
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Debug, Args)]
pub struct PublishArgs {
    /// Registered agent name (from `easynet agent list`).
    pub name: String,
    /// Currently required — the only supported mode in this PR is
    /// dry-run. Live publishing through Axon lands in a later PR;
    /// until then the flag is mandatory so scripts that assume
    /// "publish = dry-run" today can still say so explicitly and
    /// won't silently flip behaviour when the live path arrives.
    #[arg(long)]
    pub dry_run: bool,
}

pub fn run(args: AgentArgs) -> anyhow::Result<()> {
    match args.action {
        AgentAction::Add(a) => run_add(a),
        AgentAction::List => run_list(),
        AgentAction::Remove(a) => run_remove(a),
        AgentAction::Send(a) => run_send(a),
        AgentAction::Doctor(a) => run_doctor(a),
        AgentAction::Prune(a) => run_prune(a),
        AgentAction::Abilities(a) => run_abilities(a),
        AgentAction::Set(a) => run_set(a),
        AgentAction::Publish(a) => run_publish(a),
        AgentAction::Refresh => run_refresh(),
    }
}

fn run_add(args: AddArgs) -> anyhow::Result<()> {
    let agent_type: AgentType = args.r#type.parse()?;
    let mut registry = agents::load_agents()?;

    // Before we touch any disk state, reject an `agent add` that
    // would overwrite an existing v2 row whose root directory is
    // already present. The narrower `AgentDirectory::create`
    // would refuse the same case on its own, but catching it up
    // here lets us print a single clean CLI error instead of
    // letting it surface as the directory layer's internal
    // "agent.toml already exists" phrasing.
    let root = agents_root_for(&args.name);
    if root.join("agent.toml").exists() && !registry.agents.contains_key(&args.name) {
        anyhow::bail!(
            "agent root at {} already carries an `agent.toml` but no registry row. \
             Import it by hand (add a registry row pointing at this path) or remove \
             the directory before running `agent add`.",
            root.display()
        );
    }

    // ── Build the v2 spec + directory ──
    //
    // The source of truth for an agent's configuration is its
    // on-disk `agent.toml`. `run_add` materializes one from the
    // CLI flags, and the registry row becomes a thin pointer
    // (name → root path + runtime tag). Operator edits to the
    // spec afterwards do not require re-running `agent add`.
    let runtime = runtime_kind_from(agent_type);
    let mut spec = AgentSpec::new(&args.name, runtime);
    spec.model = args.model.clone();
    if let Some(label) = &args.label {
        spec.description = Some(label.clone());
    }

    // Only create the directory when the spec is brand new; an
    // update keeps the existing directory (and any operator
    // edits to the spec) intact, only touching the registry
    // pointer. The rationale: `agent add alice --model gpt-5`
    // the second time should change the registry-visible model
    // and leave the operator's hand-written description alone.
    let directory = if registry.agents.contains_key(&args.name) && root.join("agent.toml").exists()
    {
        AgentDirectory::open(&root)?
    } else {
        AgentDirectory::create(&Location::Local { root: root.clone() }, spec)?
    };

    // ── Build the v2 registry row ──
    //
    // We deliberately emit only the v2-shape fields on write
    // (name → {schema_version, root_path, runtime, model}).
    // The fat v1 fields are left at their default / empty value
    // so `save_agents`'s `skip_serializing_if` helpers omit them
    // from the JSON. A row read back from disk therefore carries
    // no fat data — PR-3b.5's dispatch refactor can then rely on
    // `AgentDirectory` being the only source of truth.
    let mut entry = AgentEntry::new(agent_type, args.model.clone());
    // Explicitly clear the fat fields that `AgentEntry::new`
    // populates for backwards compatibility.
    //
    // The `skip_serializing_if` helpers on those fields would
    // already omit them from JSON for their default values,
    // but we reset here too — two reasons:
    //
    //   1. Intent is visible at the write site. A reader of
    //      `run_add` sees "v2 write: every fat field explicitly
    //      blank" without having to cross-reference serde
    //      attributes in `registry::agents`.
    //   2. Protects against future drift. If `AgentEntry::new`
    //      ever starts returning a non-default `timeout_secs`
    //      or `max_output_bytes` (say, a future `agent add
    //      --timeout` flag is routed through `new`), the write
    //      path here keeps v2 rows free of that value so an
    //      operator who later edits `agent.toml` can't have
    //      their change shadowed by a stale registry value.
    //      The symmetry with `command.clear()` / `args.clear()`
    //      is the whole discipline.
    entry.command.clear();
    entry.args.clear();
    entry.label = None;
    entry.env.clear();
    entry.timeout_secs = agents::default_timeout_for_new_rows();
    entry.max_output_bytes = agents::default_max_output_for_new_rows();
    entry.schema_version = CURRENT_REGISTRY_SCHEMA;
    entry.root_path = Some(directory.root().to_path_buf());

    let is_update = registry.agents.contains_key(&args.name);
    registry.agents.insert(args.name.clone(), entry);
    agents::save_agents(&registry)?;

    if is_update {
        output::success(&format!("Updated agent '{}'", args.name));
    } else {
        output::success(&format!("Registered agent '{}'", args.name));
    }
    output::detail("type", &agent_type.to_string());
    if let Some(m) = &args.model {
        output::detail("model", m);
    }
    output::detail("root", &directory.root().display().to_string());

    // Eagerly project the workspace so the seeded skills (the
    // `easynet-collaborate` and `easynet-pages-author` SKILL.md
    // files) are on disk immediately after `agent add`, not
    // lazily on the first dispatch. Operators using a freshly-
    // added agent through `claude --resume` (or any non-easynet
    // entry path) get the briefing without an EasyNet round-trip
    // first. Best-effort: a projection failure logs but does not
    // fail the registration.
    if let Err(e) = crate::runtime::workspace::ensure_from_directory(&directory) {
        eprintln!(
            "[agent add warn] could not project workspace at {}: {e:#}; \
             skills will land on first dispatch",
            directory.root().display()
        );
    }

    // Publish the new agent's manifests to the local axon-runtime so
    // they appear in `ListMCPTools` (and therefore the EasyNet
    // frontend's Abilities catalog) immediately. Best-effort — see
    // `runtime::publish` doc for the failure model. The most common
    // miss is "operator hasn't started the runtime yet"; we surface a
    // hint so they know what to do, but don't fail `agent add`.
    publish_to_local_runtime_best_effort(&args.name, &directory);

    Ok(())
}

/// Best-effort publish of a freshly-added (or re-added) agent's
/// manifests. Logs every outcome via `output::detail` / `output::warn`;
/// never propagates an error.
///
/// The dispatch_endpoint passed to register is the EasyNet-CLI
/// daemon's IPC socket. Step 3 of the cross-repo plan adds the
/// runtime-side dispatch hook that uses it; until then the runtime
/// stores it without acting on it. The frontend Abilities surface
/// only needs the registration to be visible in `ListMCPTools` —
/// dispatch is a follow-up.
fn publish_to_local_runtime_best_effort(agent_name: &str, directory: &AgentDirectory) {
    let creds = match crate::persistence::config::load_credentials() {
        Ok(c) => c,
        Err(e) => {
            output::warn(&format!(
                "could not load device credentials for publish: {e}"
            ));
            return;
        }
    };
    let username = crate::facade::cli::start::bootstrap_username_for(&creds);

    if let Ok((bridge, state)) = crate::persistence::config::load_and_connect() {
        let tenant_id = state.tenant.as_deref().unwrap_or(&creds.tenant_id);
        let plan = match crate::facade::cli::start::build_bootstrap_plan_from(
            tenant_id,
            &creds.node_id,
            &username,
        ) {
            Ok(p) => p,
            Err(e) => {
                output::warn(&format!("publish bootstrap plan: {e}"));
                return;
            }
        };
        // Pin the caller URI so the bridge's hub-shaped envelope synth
        // doesn't fall back to the literal-subject path
        // (`agents/easynet:prv:hub:<realm>`), which axon's membership
        // gate rejects with AXON_MEMBERSHIP_REQUIRED. Same shape `start.rs`
        // uses at boot — the difference there was the boot path always
        // calls `with_caller_uri`; this post-add path was missed.
        let invoker = crate::runtime::advertise::BridgeAbilityInvoker::with_caller_uri(
            &bridge,
            plan.host_device_uri.clone(),
        );
        let runtime_publish = publish_bootstrap_plan_via_invoker(tenant_id, &plan, &invoker);
        // Updating the local axon-runtime keeps the device's own
        // cross-process tool catalog fresh, but the realm directory
        // used by backend `/api/v1/agents` / `federation.resolve`
        // is maintained by the daemon↔hub advertise path. Drive
        // that second hop explicitly as well so post-boot `agent
        // add` mutates both surfaces instead of only the local
        // runtime view.
        match publish_to_hub_direct_best_effort_silent(
            tenant_id,
            &creds.hub_endpoint,
            &plan.host_device_uri,
            &plan,
        ) {
            Ok(()) => output::detail(
                "publish-path",
                "local runtime updated; realm directory advertised directly to the hub",
            ),
            Err(hub_msg) => match publish_via_local_daemon_best_effort_silent(
                tenant_id,
                &plan.host_device_uri,
                &plan,
            ) {
                Ok(()) => {
                    output::warn(
                        "direct hub directory sync failed; local daemon accepted only a local \
                         publish, so hub-visible agent rows may still be stale",
                    );
                    output::warn(&format!("direct hub directory sync failed: {hub_msg}"));
                    output::detail(
                        "publish-path",
                        "local runtime updated; local daemon accepted a non-authoritative local publish",
                    );
                }
                Err(local_msg) => {
                    output::warn("could not sync the realm directory after local runtime publish");
                    output::warn(&format!("direct hub directory sync failed: {hub_msg}"));
                    output::warn(&format!(
                        "local daemon gRPC directory sync also failed: {local_msg}"
                    ));
                    output::warn(
                        "  → peers and backend `/api/v1/agents` will stay stale until the next successful advertise round",
                    );
                }
            },
        }
        if let Err(msg) = runtime_publish {
            output::warn(&format!(
                "publish through local axon-runtime reported advertise failures: {msg}"
            ));
        }
        let _ = (agent_name, directory);
        return;
    }

    let tenant_id = creds.tenant_id.as_str();
    let plan = match crate::facade::cli::start::build_bootstrap_plan_from(
        tenant_id,
        &creds.node_id,
        &username,
    ) {
        Ok(p) => p,
        Err(e) => {
            output::warn(&format!("publish bootstrap plan: {e}"));
            return;
        }
    };
    match publish_to_hub_direct_best_effort(
        tenant_id,
        &creds.hub_endpoint,
        &plan.host_device_uri,
        &plan,
    ) {
        Ok(()) => output::detail(
            "publish-path",
            "local runtime state absent; advertised directly to the hub",
        ),
        Err(hub_msg) => {
            output::warn(
                "could not reach local axon-runtime to publish manifests: no runtime state",
            );
            output::warn(&format!("direct hub publish failed: {hub_msg}"));
            output::warn(
                "  → run `easynet runtime start` to start it; then `easynet agent add ...` \
                 again will publish, or restart the daemon to re-register every agent",
            );
        }
    }
    let _ = (agent_name, directory);
}

/// Best-effort unpublish for `easynet agent remove`. Mirror of
/// `publish_to_local_runtime_best_effort` — same connect-and-loop
/// pattern, calls unregister instead of register. Keeps the local
/// runtime's MCP tool catalog in sync with the registry.
fn unpublish_from_local_runtime_best_effort(agent_name: &str, directory: &AgentDirectory) {
    let creds = match crate::persistence::config::load_credentials() {
        Ok(c) => c,
        Err(_) => return,
    };
    // RFC-001 P4.8: revoke this agent's directory entry via
    // federation.revoke. Look up the URA from local-agents.json
    // (the bootstrap step persists every llm sub-agent under
    // `(profile=llm, name=<agent_name>)`); if absent, the agent
    // was never advertised, which makes the revoke a no-op.
    let file = match crate::persistence::local_agents::load() {
        Ok(f) => f,
        Err(e) => {
            output::warn(&format!("could not read local-agents.json for revoke: {e}"));
            return;
        }
    };
    let agent_uri =
        match crate::persistence::local_agents::lookup_hosted_uri(&file, "llm", agent_name) {
            Some(uri) => uri,
            None => return,
        };
    // Pin the caller URI so federation.revoke's signed envelope
    // carries the daemon's host URA, not the literal-subject
    // fallback the bridge would otherwise synthesise (which the
    // membership gate rejects). See the same fix in
    // `publish_to_local_runtime_best_effort`.
    let realm =
        crate::facade::cli::start::realm_from_agent_uri(&file.host_device_agent_uri).unwrap_or("");
    if let Ok((bridge, state)) = crate::persistence::config::load_and_connect() {
        let tenant_id = state.tenant.as_deref().unwrap_or(&creds.tenant_id);
        let invoker = crate::runtime::advertise::BridgeAbilityInvoker::with_caller_uri(
            &bridge,
            file.host_device_agent_uri.clone(),
        );
        if let Err(msg) = revoke_agent_via_invoker(
            &invoker,
            tenant_id,
            realm,
            &agent_uri,
            "operator removed agent",
        ) {
            output::warn(&format!(
                "revoke through local axon-runtime reported directory failure: {msg}"
            ));
        }
        let _ = (agent_name, directory);
        return;
    }

    let _ = revoke_from_hub_direct_best_effort(
        &creds.tenant_id,
        &creds.hub_endpoint,
        &file.host_device_agent_uri,
        realm,
        &agent_uri,
        "operator removed agent",
    );
    // Runtime not running and direct hub revoke unavailable →
    // leave the stale directory entry to the next boot's full
    // advertise sweep / revoke. Silent by contract; operator
    // already saw "Removed agent".
    let _ = (agent_name, directory);
}

fn publish_bootstrap_plan_via_invoker<I: crate::runtime::advertise::AbilityInvoker>(
    tenant_id: &str,
    plan: &crate::runtime::agents::profiles::bootstrap::BootstrapPlan,
    invoker: &I,
) -> Result<(), String> {
    let outcomes = publish_bootstrap_plan_outcomes_via_invoker(tenant_id, plan, invoker);
    render_publish_outcomes(&outcomes);
    publish_outcome_summary(&outcomes)
}

fn publish_bootstrap_plan_outcomes_via_invoker<I: crate::runtime::advertise::AbilityInvoker>(
    tenant_id: &str,
    plan: &crate::runtime::agents::profiles::bootstrap::BootstrapPlan,
    invoker: &I,
) -> Vec<crate::runtime::publish::PublishOutcome> {
    crate::runtime::publish::republish_abilities_via_advertise(invoker, tenant_id, plan)
}

#[cfg(feature = "axon-pb")]
fn publish_bootstrap_plan_via_invoker_silent<I: crate::runtime::advertise::AbilityInvoker>(
    tenant_id: &str,
    plan: &crate::runtime::agents::profiles::bootstrap::BootstrapPlan,
    invoker: &I,
) -> Result<(), String> {
    let outcomes = publish_bootstrap_plan_outcomes_via_invoker(tenant_id, plan, invoker);
    publish_outcome_summary(&outcomes)
}

fn render_publish_outcomes(outcomes: &[crate::runtime::publish::PublishOutcome]) {
    let mut ok = 0usize;
    let mut total = 0usize;
    for outcome in outcomes {
        if outcome.label == "skipped" || outcome.label == "local-agents.json" {
            continue;
        }
        total += 1;
        match &outcome.result {
            Ok(_) => {
                ok += 1;
                if outcome.label.starts_with("abilities/") {
                    output::detail(
                        "published",
                        &format!("{} {}", outcome.agent_uri, outcome.label),
                    );
                }
            }
            Err(msg) => output::warn(&format!("publish {} failed: {msg}", outcome.label)),
        }
    }
    if total > 0 {
        output::detail(
            "directory",
            &format!("{ok}/{total} federation.advertise_* calls — entries visible to peers"),
        );
    }
}

fn publish_outcome_summary(
    outcomes: &[crate::runtime::publish::PublishOutcome],
) -> Result<(), String> {
    let mut relevant = 0usize;
    let mut failures = Vec::new();
    for outcome in outcomes {
        if outcome.label == "skipped" || outcome.label == "local-agents.json" {
            continue;
        }
        relevant += 1;
        if let Err(msg) = &outcome.result {
            failures.push(format!("{} {}: {msg}", outcome.agent_uri, outcome.label));
        }
    }
    if relevant == 0 {
        return Err("no federation.advertise_* outcomes produced".into());
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn revoke_agent_via_invoker<I: crate::runtime::advertise::AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_uri: &str,
    reason: &str,
) -> Result<(), String> {
    let outcome = crate::runtime::publish::unpublish_abilities_via_revoke(
        invoker, tenant_id, realm, agent_uri, reason,
    );
    match outcome.result {
        Ok(_) => {
            output::detail("revoked", &outcome.agent_uri);
            Ok(())
        }
        Err(msg) => {
            output::warn(&format!("revoke {} failed: {msg}", outcome.agent_uri));
            Err(msg)
        }
    }
}

#[cfg(any(test, feature = "axon-pb"))]
fn hub_invoke_endpoint(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("hub endpoint is empty".into());
    }
    if let Some(rest) = trimmed.strip_prefix("axon://") {
        return Ok(format!("http://{rest}"));
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(trimmed.to_string());
    }
    if trimmed.contains("://") {
        return Err(format!("unsupported hub endpoint scheme in `{trimmed}`"));
    }
    Ok(format!("http://{trimmed}"))
}

#[cfg(any(test, feature = "axon-pb"))]
fn ability_name_from_resource_uri(resource_uri: &str) -> Option<String> {
    // Canonical v4.1.4 ability URAs encode the member-call target as
    // `<owner>.<agent>.<verb>`. Routing wants just `<agent>.<verb>`,
    // e.g. `federation.advertise_agent` for
    // `easynet:///r/acme/ability/hub.federation.advertise_agent`.
    crate::uri::qualified_ability_name(resource_uri)
}

#[cfg(any(test, feature = "axon-pb"))]
fn direct_hub_trust_match(endpoint_url: &str) -> Option<Option<std::path::PathBuf>> {
    for path in direct_hub_trust_path_candidates() {
        let anchor =
            match crate::services::realm_trust_anchor::RealmTrustAnchor::load_or_empty(&path) {
                Ok(anchor) => anchor,
                Err(_) => continue,
            };
        if let Some(entry) = anchor.lookup_peer_hub(endpoint_url) {
            return Some(entry.tls_ca_pem_path.clone());
        }
    }
    None
}

#[cfg(any(test, feature = "axon-pb"))]
fn direct_hub_trust_path_candidates() -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Some(override_path) = std::env::var_os("EASYNET_REALM_TRUST_PATH") {
        candidates.push(std::path::PathBuf::from(override_path));
    }
    candidates.push(std::path::PathBuf::from(
        crate::services::realm_trust_anchor::DEFAULT_REALM_TRUST_PATH,
    ));
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        candidates.push(home.join("realm-trust.toml"));
        candidates.push(home.join(".easynet/realm-trust.toml"));
    }
    let mut deduped = Vec::with_capacity(candidates.len());
    for path in candidates {
        if !deduped.iter().any(|seen| seen == &path) {
            deduped.push(path);
        }
    }
    deduped
}

#[cfg(feature = "axon-pb")]
struct DirectLocalDaemonAbilityInvoker {
    caller_uri: String,
    socket_path: std::path::PathBuf,
}

#[cfg(feature = "axon-pb")]
impl DirectLocalDaemonAbilityInvoker {
    fn new(caller_uri: &str) -> Result<Self, String> {
        if caller_uri.trim().is_empty() {
            return Err("host device URI is empty; cannot publish via local daemon".into());
        }
        let socket_path = local_daemon_grpc_socket_path();
        if !socket_path.exists() {
            return Err(format!(
                "daemon gRPC socket not found at {}",
                socket_path.display()
            ));
        }
        Ok(Self {
            caller_uri: caller_uri.to_string(),
            socket_path,
        })
    }

    fn invoke_function(
        &self,
        function_name: &str,
        payload_json: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let caller_uri = self.caller_uri.clone();
        let socket_path = self.socket_path.clone();
        let arguments = serde_json::to_vec(&payload_json)
            .map_err(|e| format!("encode {function_name} payload: {e}"))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("build tokio runtime for local daemon publish: {e}"))?;

        runtime.block_on(async move {
            let endpoint = tonic::transport::Endpoint::try_from("http://[::1]:50051")
                .map_err(|e| format!("build tonic endpoint: {e}"))?
                .timeout(std::time::Duration::from_secs(10))
                .connect_timeout(std::time::Duration::from_secs(5));
            let channel = endpoint
                .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                    let path = socket_path.clone();
                    async move {
                        let stream = tokio::net::UnixStream::connect(path).await?;
                        Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                    }
                }))
                .await
                .map_err(|e| format!("connect to local daemon gRPC socket: {e}"))?;
            let mut client = crate::pb::axon::v1::invocation_client::InvocationClient::new(channel);
            let request = crate::pb::axon::v1::InvokeRequest {
                envelope: Some(crate::pb::axon::v1::Envelope {
                    caller: Some(crate::pb::axon::v1::AgentIdentity {
                        uri: caller_uri.clone(),
                        ..Default::default()
                    }),
                    callee: Some(crate::pb::axon::v1::AgentIdentity {
                        uri: caller_uri.clone(),
                        ..Default::default()
                    }),
                    subject: Some(crate::pb::axon::v1::SubjectIdentity {
                        uri: caller_uri,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                function_name: function_name.to_string(),
                arguments,
                ..Default::default()
            };
            let response = client.invoke(request).await.map_err(|status| {
                format!(
                    "local daemon rejected {function_name}: code={:?} message={}",
                    status.code(),
                    status.message()
                )
            })?;
            let body = response.into_inner();
            if body.result.is_empty() {
                return Ok(serde_json::Value::Null);
            }
            serde_json::from_slice(&body.result)
                .map_err(|e| format!("decode {function_name} response: {e}"))
        })
    }
}

#[cfg(feature = "axon-pb")]
impl crate::runtime::advertise::AbilityInvoker for DirectLocalDaemonAbilityInvoker {
    fn invoke_ability(
        &self,
        _tenant_id: &str,
        resource_uri: &str,
        payload_json: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let function_name = ability_name_from_resource_uri(resource_uri).ok_or_else(|| {
            format!("cannot derive ability name from resource URI `{resource_uri}`")
        })?;
        self.invoke_function(&function_name, payload_json)
    }
}

#[cfg(feature = "axon-pb")]
fn local_daemon_grpc_socket_path() -> std::path::PathBuf {
    let raw = std::env::var("EASYNET_DAEMON_GRPC_UDS")
        .unwrap_or_else(|_| crate::persistence::daemon_config::DEFAULT_DAEMON_UDS_PATH.to_string());
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::PathBuf::from(home).join(rest);
        }
    }
    std::path::PathBuf::from(raw)
}

#[cfg(feature = "axon-pb")]
fn publish_via_local_daemon_best_effort_silent(
    tenant_id: &str,
    caller_uri: &str,
    plan: &crate::runtime::agents::profiles::bootstrap::BootstrapPlan,
) -> Result<(), String> {
    let invoker = DirectLocalDaemonAbilityInvoker::new(caller_uri)?;
    publish_bootstrap_plan_via_invoker_silent(tenant_id, plan, &invoker)
}

#[cfg(not(feature = "axon-pb"))]
fn publish_via_local_daemon_best_effort_silent(
    _tenant_id: &str,
    _caller_uri: &str,
    _plan: &crate::runtime::agents::profiles::bootstrap::BootstrapPlan,
) -> Result<(), String> {
    Err("local daemon publish requires the `axon-pb` feature".into())
}

#[cfg(feature = "axon-pb")]
struct DirectHubAbilityInvoker {
    endpoint: String,
    caller_uri: String,
    hub_uri: String,
}

#[cfg(feature = "axon-pb")]
impl DirectHubAbilityInvoker {
    fn new(raw_endpoint: &str, caller_uri: &str, realm: &str) -> Result<Self, String> {
        if caller_uri.trim().is_empty() {
            return Err("host device URI is empty; cannot publish to the hub".into());
        }
        Ok(Self {
            endpoint: hub_invoke_endpoint(raw_endpoint)?,
            caller_uri: caller_uri.to_string(),
            hub_uri: crate::uri::hub_uri(realm),
        })
    }

    fn invoke_function(
        &self,
        function_name: &str,
        payload_json: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let endpoint_url = self.endpoint.clone();
        let caller_uri = self.caller_uri.clone();
        let hub_uri = self.hub_uri.clone();
        let arguments = serde_json::to_vec(&payload_json)
            .map_err(|e| format!("encode {function_name} payload: {e}"))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("build tokio runtime for hub publish: {e}"))?;

        runtime.block_on(async move {
            let mut endpoint = tonic::transport::Endpoint::from_shared(endpoint_url.clone())
                .map_err(|e| format!("invalid hub endpoint `{endpoint_url}`: {e}"))?
                .timeout(std::time::Duration::from_secs(10))
                .connect_timeout(std::time::Duration::from_secs(5));

            if endpoint_url.starts_with("https://") {
                let tls = match direct_hub_trust_match(&endpoint_url) {
                    Some(Some(path)) => {
                        crate::services::federation_client::pinned_tls_config(path.as_path())
                            .map_err(|e| format!("load pinned CA for `{endpoint_url}`: {e}"))?
                    }
                    Some(None) | None => {
                        tonic::transport::ClientTlsConfig::new().with_native_roots()
                    }
                };
                endpoint = endpoint
                    .tls_config(tls)
                    .map_err(|e| format!("configure TLS for `{endpoint_url}`: {e}"))?;
            }

            let channel = endpoint
                .connect()
                .await
                .map_err(|e| format!("connect to hub `{endpoint_url}`: {e}"))?;
            let mut client = crate::pb::axon::v1::invocation_client::InvocationClient::new(channel);
            let request = crate::pb::axon::v1::InvokeRequest {
                envelope: Some(crate::pb::axon::v1::Envelope {
                    caller: Some(crate::pb::axon::v1::AgentIdentity {
                        uri: caller_uri,
                        ..Default::default()
                    }),
                    callee: Some(crate::pb::axon::v1::AgentIdentity {
                        uri: hub_uri.clone(),
                        ..Default::default()
                    }),
                    subject: Some(crate::pb::axon::v1::SubjectIdentity {
                        uri: hub_uri,
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                function_name: function_name.to_string(),
                arguments,
                ..Default::default()
            };
            let response = client.invoke(request).await.map_err(|status| {
                format!(
                    "hub rejected {function_name}: code={:?} message={}",
                    status.code(),
                    status.message()
                )
            })?;
            let body = response.into_inner();
            if body.result.is_empty() {
                return Ok(serde_json::Value::Null);
            }
            serde_json::from_slice(&body.result)
                .map_err(|e| format!("decode {function_name} response: {e}"))
        })
    }
}

#[cfg(feature = "axon-pb")]
impl crate::runtime::advertise::AbilityInvoker for DirectHubAbilityInvoker {
    fn invoke_ability(
        &self,
        _tenant_id: &str,
        resource_uri: &str,
        payload_json: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let function_name = ability_name_from_resource_uri(resource_uri).ok_or_else(|| {
            format!("cannot derive ability name from resource URI `{resource_uri}`")
        })?;
        self.invoke_function(&function_name, payload_json)
    }
}

#[cfg(feature = "axon-pb")]
fn publish_to_hub_direct_best_effort(
    tenant_id: &str,
    hub_endpoint: &str,
    caller_uri: &str,
    plan: &crate::runtime::agents::profiles::bootstrap::BootstrapPlan,
) -> Result<(), String> {
    let invoker = DirectHubAbilityInvoker::new(hub_endpoint, caller_uri, tenant_id)?;
    publish_bootstrap_plan_via_invoker(tenant_id, plan, &invoker)
}

#[cfg(feature = "axon-pb")]
fn publish_to_hub_direct_best_effort_silent(
    tenant_id: &str,
    hub_endpoint: &str,
    caller_uri: &str,
    plan: &crate::runtime::agents::profiles::bootstrap::BootstrapPlan,
) -> Result<(), String> {
    let invoker = DirectHubAbilityInvoker::new(hub_endpoint, caller_uri, tenant_id)?;
    publish_bootstrap_plan_via_invoker_silent(tenant_id, plan, &invoker)
}

#[cfg(not(feature = "axon-pb"))]
fn publish_to_hub_direct_best_effort(
    _tenant_id: &str,
    _hub_endpoint: &str,
    _caller_uri: &str,
    _plan: &crate::runtime::agents::profiles::bootstrap::BootstrapPlan,
) -> Result<(), String> {
    Err("direct hub publish requires the `axon-pb` feature".into())
}

#[cfg(not(feature = "axon-pb"))]
fn publish_to_hub_direct_best_effort_silent(
    _tenant_id: &str,
    _hub_endpoint: &str,
    _caller_uri: &str,
    _plan: &crate::runtime::agents::profiles::bootstrap::BootstrapPlan,
) -> Result<(), String> {
    Err("direct hub publish requires the `axon-pb` feature".into())
}

#[cfg(feature = "axon-pb")]
fn revoke_from_hub_direct_best_effort(
    tenant_id: &str,
    hub_endpoint: &str,
    caller_uri: &str,
    realm: &str,
    agent_uri: &str,
    reason: &str,
) -> Result<(), String> {
    let invoker = DirectHubAbilityInvoker::new(hub_endpoint, caller_uri, tenant_id)?;
    revoke_agent_via_invoker(&invoker, tenant_id, realm, agent_uri, reason)
}

#[cfg(not(feature = "axon-pb"))]
fn revoke_from_hub_direct_best_effort(
    _tenant_id: &str,
    _hub_endpoint: &str,
    _caller_uri: &str,
    _realm: &str,
    _agent_uri: &str,
    _reason: &str,
) -> Result<(), String> {
    Err("direct hub revoke requires the `axon-pb` feature".into())
}

/// Map the legacy `AgentType` tag onto the `RuntimeKind` used
/// by `AgentSpec`. Kept as a free function because three call
/// sites share this mapping (`run_add`, `registry::migrate_one_entry`,
/// `runtime::workspace::spec_from_entry`). A drift between any
/// two of them would produce inconsistent specs for the same
/// agent — the parity tests pin two of the three to each
/// other; this function is the common implementation.
fn runtime_kind_from(t: AgentType) -> RuntimeKind {
    match t {
        AgentType::ClaudeCode => RuntimeKind::ClaudeCode,
        AgentType::Codex => RuntimeKind::Codex,
        AgentType::CodexAppServer => RuntimeKind::CodexAppServer,
    }
}

/// Resolve the on-disk root an `agent add` should materialize
/// for the given name. Folds over the `agents_root()` fallback
/// introduced in PR-0b so an operator with only the legacy
/// `workspaces/` tree keeps working without a flag day.
fn agents_root_for(agent_name: &str) -> std::path::PathBuf {
    config::agents_root().join(agent_name)
}

fn run_list() -> anyhow::Result<()> {
    let registry = agents::load_agents()?;

    if registry.agents.is_empty() {
        eprintln!("  No agents registered.");
        eprintln!(
            "  Run {} to add one.",
            style("easynet agent add claude --type claude-code --model sonnet").cyan()
        );
        return Ok(());
    }

    eprintln!();
    // Header. The new `STATUS` column calls out rows whose root
    // directory has disappeared — an operator who sees "path
    // missing" can either `agent prune` or re-materialize the
    // directory by hand. Adding it here rather than post-hoc
    // turning out "zombie rows" into dispatch errors at
    // `agent send` time is the more humane shape.
    eprintln!(
        "  {:<14} {:<18} {:<12} {:<10} {}",
        style("NAME").dim(),
        style("TYPE").dim(),
        style("MODEL").dim(),
        style("TIMEOUT").dim(),
        style("STATUS").dim(),
    );
    eprintln!("  {}", style("─".repeat(68)).dim());

    for (name, entry) in &registry.agents {
        // Prefer the spec-derived values when available: an
        // operator who edited `agent.toml` after
        // `agent add` sees their changes reflected here
        // without re-registering. Fall back to the legacy
        // fat-row fields so pre-migration rows still render.
        let (model, timeout_secs, status) = render_row_status(name, entry);

        let type_styled = match entry.agent_type {
            agents::AgentType::ClaudeCode => style("claude-code").magenta(),
            agents::AgentType::Codex => style("codex").yellow(),
            agents::AgentType::CodexAppServer => style("codex-app-server").yellow(),
        };
        eprintln!(
            "  {:<14} {:<18} {:<12} {:<10} {}",
            style(name).white().bold(),
            type_styled,
            style(model.as_deref().unwrap_or("-")).cyan(),
            style(format!("{timeout_secs}s")).dim(),
            status,
        );
    }
    eprintln!();
    Ok(())
}

/// Resolve the rendered columns for one `agent list` row.
///
/// Returns `(model, timeout_secs, status)` where `status` is a
/// pre-styled console string. Pulling this logic out of
/// `run_list` keeps the loop body readable and lets the status
/// rules be unit-tested if they grow (today "ok" vs "path
/// missing" is the whole rule set; tomorrow we might add "v1
/// un-migrated", "orphaned handle", etc.).
fn render_row_status(
    name: &str,
    entry: &AgentEntry,
) -> (Option<String>, u64, console::StyledObject<&'static str>) {
    // Resolve the agent root: explicit `root_path` wins; fall
    // back to the consumer-side default that `ensure_workspace`
    // uses. This keeps display consistent with dispatch
    // behaviour — any agent that `agent send` would find at
    // path P must also be listed at that P.
    let root = entry
        .root_path
        .clone()
        .unwrap_or_else(|| config::agents_root().join(name));

    // Try to open the agent.toml for spec-derived values. If
    // that fails (root missing, file missing, parse error) we
    // still render the row using fat-field fallbacks so an
    // operator can see the row at all and decide what to do.
    let (spec_model, spec_timeout) = match AgentDirectory::open(&root) {
        Ok(dir) => (dir.spec().model.clone(), dir.spec().timeout_secs),
        Err(_) => (None, None),
    };

    let model = spec_model.or_else(|| entry.model.clone());
    let timeout_secs = spec_timeout.unwrap_or(entry.timeout_secs);

    let status = if root.exists() {
        style("ok").green()
    } else {
        // Red on purpose: a missing root is an actionable
        // signal, not noise. Operators who see this are
        // expected to run `agent prune` or re-materialize by
        // hand; the column gives them the cue.
        style("path missing").red()
    };

    (model, timeout_secs, status)
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    let mut registry = agents::load_agents()?;

    let removed = registry
        .agents
        .remove(&args.name)
        .ok_or_else(|| anyhow::anyhow!("agent '{}' not found", args.name))?;

    agents::save_agents(&registry)?;
    output::success(&format!("Removed agent '{}'", args.name));

    // Unpublish from the local axon-runtime so this agent's tools
    // disappear from `ListMCPTools` (and the EasyNet frontend's
    // Abilities catalog). Done BEFORE purge so we can still read the
    // abilities directory; best-effort — see runtime::publish doc.
    // The unpublish must happen before --purge wipes the abilities
    // dir, otherwise we'd have nothing to enumerate from.
    if let Some(root) = removed.root_path.as_ref() {
        if let Ok(directory) = AgentDirectory::open(root) {
            unpublish_from_local_runtime_best_effort(&args.name, &directory);
        }
    }

    // Root deletion is opt-in. This is the purge branch: we
    // only reach it when the operator explicitly asked for it.
    // `removed.root_path` is the authoritative location on v2
    // rows; a v1 row that somehow survived migration would
    // fall back to the legacy computation.
    if args.purge {
        let root = removed
            .root_path
            .clone()
            .unwrap_or_else(|| config::agents_root().join(&args.name));
        if root.exists() {
            std::fs::remove_dir_all(&root)
                .map_err(|e| anyhow::anyhow!("remove {}: {e}", root.display()))?;
            output::detail("purged", &root.display().to_string());
        } else {
            output::detail("purge", &format!("{} already absent", root.display()));
        }
    } else if let Some(root) = removed.root_path.as_ref() {
        // Not a warning, just a hint for operators who wanted
        // the directory gone too. The directory still holds
        // per-agent credentials (`.env`) and run history, so
        // defaulting to "keep" is the safe choice.
        output::detail(
            "kept",
            &format!(
                "{} (pass --purge to delete credentials + runs)",
                root.display()
            ),
        );
    }

    Ok(())
}

/// `easynet agent prune` removes registry rows whose on-disk
/// agent root has gone missing. The target shape is the
/// "orphan" row the `agent list` command flags as `path
/// missing` — when a project-local agent directory has been
/// deleted (e.g. the enclosing repo was removed), the row that
/// used to point at it becomes dead weight and its
/// `agent send` calls fail loudly at dispatch time. `prune` is
/// the sanctioned cleanup.
///
/// A `--dry-run` flag prints what would be removed without
/// mutating the registry. Operators scanning a large registry
/// can preview the cleanup safely before committing.
///
/// No explicit backup is written on the non-dry path: pruned
/// rows are by definition unreachable (their root is gone, so
/// dispatch would fail anyway), and `save_agents` is atomic, so
/// a crash mid-prune leaves the registry either unchanged or
/// fully rewritten. The one rollback path is manual — an
/// operator can restore from `~/.easynet/agents.json.v1.bak` if
/// they never completed a v2 save.
fn run_prune(args: PruneArgs) -> anyhow::Result<()> {
    let mut registry = agents::load_agents()?;

    // Identify rows whose root is missing. We check the
    // explicit `root_path` first, falling back to the
    // consumer-side default so a v2 row whose `root_path` was
    // never populated (still a real scenario today — `run_add`
    // before this PR did not set it) is classified the same as
    // any other.
    let orphans: Vec<String> = registry
        .agents
        .iter()
        .filter_map(|(name, entry)| {
            let root = entry
                .root_path
                .clone()
                .unwrap_or_else(|| config::agents_root().join(name));
            if root.exists() {
                None
            } else {
                Some(name.clone())
            }
        })
        .collect();

    if orphans.is_empty() {
        output::info("No orphaned agents to prune.");
        return Ok(());
    }

    eprintln!();
    eprintln!(
        "  {} {} orphan(s):",
        if args.dry_run {
            style("would prune").yellow()
        } else {
            style("pruning").green()
        },
        orphans.len()
    );
    for name in &orphans {
        let root = registry.agents[name]
            .root_path
            .clone()
            .unwrap_or_else(|| config::agents_root().join(name));
        eprintln!("    • {}  (missing root: {})", name, root.display());
    }
    eprintln!();

    if args.dry_run {
        return Ok(());
    }

    for name in &orphans {
        registry.agents.remove(name);
    }
    agents::save_agents(&registry)?;
    output::success(&format!("Pruned {} orphaned agent(s)", orphans.len()));
    Ok(())
}

/// `easynet agent send <name> "<prompt>"` is sugar for a single-line
/// External EAL mission invoking the target agent's default `chat`
/// ability. Quoting ontology §6.2 Decision 4:
///
/// > **agent send semantics**: Sugar for a single-line External EAL
/// > mission invoking the target's default `chat` ability. Unifies all
/// > cross-agent interaction under the mission execution model.
///
/// Concretely, `easynet agent send claude "hi"` desugars to:
///
/// ```eal
/// mission "agent-send" {
///   let __reply = claude.chat(prompt: "hi")
/// }
/// ```
///
/// and is then handed to `mission_runs::run_mission_inproc` — the single
/// in-process mission entry point. There is no second path: every
/// cross-agent call in EasyNet (CLI surface, EAL programs, MCP handlers)
/// goes through this function. See `cli/mission_runs.rs` for the load-
/// bearing single-entry invariant.
/// In-place mutation of a registered agent. Currently the only
/// supported field is `model`; future fields plug in as additional
/// `Option<...>` flags on `SetArgs` without changing this routing.
///
/// The two on-disk artifacts (`agent.toml` + `agents.json` row) are
/// updated in two writes — atomic per file, but not jointly atomic.
/// We update `agent.toml` first because it is the source of truth
/// for the agent's runtime contract; if the registry write later
/// fails, the next read goes through `AgentDirectory::open` and
/// sees the new model. The opposite ordering would risk a window
/// where the registry advertises a model the on-disk agent has
/// not yet adopted.
///
/// We deliberately do NOT validate the model string against any
/// hardcoded list. `claude --model` and `codex --model` accept
/// any string transparently and resolve aliases at the upstream
/// CLI's discretion; shipping our own allow-list would force
/// EasyNet to chase upstream releases. Validation belongs at
/// invocation time. See `SetArgs::model` doc.
fn run_set(args: SetArgs) -> anyhow::Result<()> {
    let registry_before = agents::load_agents()?;
    if !registry_before.agents.contains_key(&args.name) {
        anyhow::bail!(
            "agent '{}' is not registered; run `easynet agent list` to see registered \
             names, or `easynet agent add {} --type …` to register it first",
            args.name,
            args.name,
        );
    }

    // The clap surface lets us tell "flag absent" from "flag empty
    // string" via Option<String>. An empty string is the explicit
    // CLEAR signal (operator wants the agent to fall back to the
    // CLI's default model); absence means "no change".
    let new_model: Option<Option<String>> = match args.model.as_deref() {
        Some("") => Some(None),
        Some(m) => Some(Some(m.to_string())),
        None => None,
    };

    if new_model.is_none() {
        anyhow::bail!(
            "agent set: nothing to change. Pass --model <name> to change the model, or \
             --model '' to clear it"
        );
    }
    let new_model = new_model.unwrap();

    // Step 1: rewrite agent.toml.
    let mut directory = open_registered_agent(&args.name)?;
    directory.set_model(new_model.clone())?;

    // Step 2: rewrite registry row.
    let mut registry = agents::load_agents()?;
    if let Some(entry) = registry.agents.get_mut(&args.name) {
        entry.with_model(new_model.clone());
    }
    agents::save_agents(&registry)?;

    output::success(&format!("Updated agent '{}'", args.name));
    match &new_model {
        Some(m) => output::detail("model", m),
        None => output::detail("model", "(cleared — CLI default will be used)"),
    }
    output::detail("root", &directory.root().display().to_string());

    Ok(())
}

fn run_send(args: SendArgs) -> anyhow::Result<()> {
    // Validate the agent exists in the registry up-front so the user gets
    // a clear error before we go through the mission machinery.
    let registry = agents::load_agents()?;
    let _entry = registry.agents.get(&args.name).ok_or_else(|| {
        anyhow::anyhow!("agent '{}' not found. Run `easynet agent list`.", args.name)
    })?;

    // User-visible counterpart to the doc-comment ontology reference.
    // Tells the user exactly what path their command is taking, so they
    // can reason about why a mission run dir appears, why MCP audit
    // lines may show up, and why the dispatch invariant assertion may
    // fire if anything is misconfigured.
    eprintln!(
        "  {} {}",
        style("[agent-send]").dim(),
        style("dispatching via mission runtime").dim(),
    );

    // Compose the prompt: fold optional `--context` into the prompt body
    // BEFORE constructing the EAL source, so the prompt that ends up in
    // the EAL string literal is exactly the prompt the agent will see.
    let composed_prompt = match args.context.as_deref() {
        Some(ctx) if !ctx.trim().is_empty() => {
            format!(
                "{}\n\n## Context (previous discussion)\n\n{}\n",
                args.prompt, ctx
            )
        }
        _ => args.prompt.clone(),
    };

    // Build the single-line EAL mission source. The mission name is
    // `agent-send`; the binding is `__reply` so the result can be
    // pulled out of `MissionRunResult.bound_vars`. `eal_string_literal`
    // can fail if the user's prompt contains an embedded NUL byte — we
    // surface that as a CLI error rather than silently truncating.
    let eal_source = format!(
        "mission \"agent-send\" {{\n    let __reply = {agent}.chat(prompt: {prompt})\n}}\n",
        agent = args.name,
        prompt = eal_string_literal(&composed_prompt)?,
    );

    // Hand the source to THE single in-process mission entry point.
    // The runner sets `EASYNET_MISSION_ID` for the duration of execution
    // (see `mission_runs::MissionContextGuard`), so any nested
    // `dispatch::send_to_agent` calls satisfy Step 9's invariant.
    let result = mission_runs::run_mission_inproc(
        &eal_source,
        MissionRunOpts {
            source_label: Some(format!("agent send {}", args.name)),
            // `--trace <path>` plumbing — currently unused; the mission
            // runner always writes the full trace into the run dir.
            // TODO: thread this through if/when `MissionRunOpts` grows
            // a real trace export.
            trace_path: args.trace.clone(),
        },
    )?;

    // Pull the agent's reply out of the mission's bound vars. The
    // dispatcher returns a JSON object with shape
    // `{"ok": true, "agent": "...", "output": "...", ...}` (see
    // `eal::interpreter::AgentAwareDispatcher::dispatch`); the
    // user-visible reply is the `output` field.
    let reply_text: String = match result.bound_vars.get("__reply") {
        Some(serde_json::Value::Object(obj)) => obj
            .get("output")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| serde_json::Value::Object(obj.clone()).to_string()),
        Some(other) => other.to_string(),
        None => String::new(),
    };

    eprintln!();
    eprintln!(
        "  {} {} {}",
        style(&args.name).white().bold(),
        style("done in").dim(),
        style(format!("{:.1}s", result.meta.duration_ms as f64 / 1000.0)).cyan(),
    );

    // Token line: read the nested agent run dir's meta.json. The mission
    // run dir contains the agent run dir as a sibling artefact (the
    // dispatch layer creates it independently under
    // ~/.easynet/workspaces/<agent>/runs/). We surface the most recent
    // one for this agent — for a one-step `agent send` it is unambiguous.
    //
    // TODO(token-meta-aggregation): this token aggregation logic is a
    // presentation-layer leak into execution detail. The right home is
    // `MissionRunMeta` itself — after every step, the mission runner
    // should sum the agent run stats into a `MissionRunMeta.token_usage`
    // field. Defer to a follow-up PR.
    if let Some(usage) = read_latest_agent_usage(&args.name) {
        let total_in = usage.input_tokens + usage.cache_read_tokens + usage.cache_creation_tokens;
        eprintln!(
            "  {} in={} out={} cache_read={} cache_write={} turns={} cost=${:.4}",
            style("tokens").dim(),
            style(total_in).cyan(),
            style(usage.output_tokens).cyan(),
            style(usage.cache_read_tokens).dim(),
            style(usage.cache_creation_tokens).dim(),
            style(usage.num_turns).dim(),
            usage.total_cost_usd,
        );
    }

    // "saved" path is the **mission** run dir, not the nested agent run
    // dir. The mission run dir is the artefact users should reference —
    // it contains source.eal, ir.json, trace.json, meta.json, and is
    // where the (currently None) `ability_graph_traces` field will land
    // when v2 ships.
    eprintln!(
        "  {} {}",
        style("saved").dim(),
        style(result.run_dir.display().to_string()).cyan(),
    );
    eprintln!();

    // Render the agent's final reply as markdown when stdout is a TTY;
    // otherwise print raw text so piping into other tools stays clean.
    if console::Term::stdout().is_term() {
        let skin = build_markdown_skin();
        let compact = compact_markdown(&reply_text);
        skin.print_text(&compact);
    } else {
        println!("{}", reply_text);
    }
    Ok(())
}

/// Quote a string as a valid EAL string literal: wrap in double quotes
/// and escape every character the EAL lexer would otherwise consume or
/// that downstream consumers (the deployed ability runtime, agent
/// prompts) might mis-handle.
///
/// EAL's lexer (`src/eal/lexer.rs::read_string`) treats `\\<char>` as
/// "skip one byte after the backslash" rather than performing real
/// escape decoding (locked contract — see iter-4 audit notes), so we
/// only need to defang the characters that would terminate the literal
/// (`"`) or change how the lexer counts bytes (`\\`). The remaining
/// escapes (`\n`, `\r`, `\t`) keep the generated EAL source readable
/// when the user pastes a multi-line prompt.
///
/// We additionally reject ASCII NUL: while EAL's lexer would store it
/// happily, downstream consumers — agent CLIs that treat the prompt as
/// a C string, ability runtimes that exec via shell — silently
/// truncate at the first `\0`. Better to fail loud at the call site
/// (`run_send`) than to deliver a half-prompt.
fn eal_string_literal(s: &str) -> anyhow::Result<String> {
    if s.contains('\0') {
        anyhow::bail!(
            "prompt contains an embedded NUL byte (U+0000); strip it before sending — \
             downstream agent CLIs treat NUL as end-of-string and would silently truncate"
        );
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

/// Read the most recent agent run dir for `agent_name` and extract the
/// usage stats from its `meta.json`. Returns `None` if no run dir
/// exists or the meta is unreadable. This is the temporary glue that
/// powers the token line in `run_send` — see the
/// `TODO(token-meta-aggregation)` comment in `run_send` for the
/// long-term plan.
fn read_latest_agent_usage(agent_name: &str) -> Option<AgentUsageReader> {
    use std::fs;

    // Source of truth for the per-agent root directory is
    // `agents_root()`: it returns the new `~/.easynet/agents/`
    // layout when present and falls back to the legacy
    // `workspaces/` path otherwise. A direct join on `state_dir()`
    // here would break reads against agents created under the new
    // layout.
    let runs_root = crate::persistence::config::agents_root()
        .join(agent_name)
        .join("runs");
    if !runs_root.exists() {
        return None;
    }

    let mut latest: Option<(String, std::path::PathBuf)> = None;
    for entry in fs::read_dir(&runs_root).ok()? {
        let entry = entry.ok()?;
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if latest
            .as_ref()
            .map(|(n, _)| name.as_str() > n.as_str())
            .unwrap_or(true)
        {
            latest = Some((name, entry.path()));
        }
    }
    let (_, path) = latest?;
    let meta_path = path.join("meta.json");
    let raw = fs::read_to_string(&meta_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Some(AgentUsageReader {
        input_tokens: v.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        output_tokens: v.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        cache_read_tokens: v
            .get("cache_read_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        cache_creation_tokens: v
            .get("cache_creation_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        num_turns: v.get("num_turns").and_then(|x| x.as_u64()).unwrap_or(0),
        total_cost_usd: v
            .get("total_cost_usd")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0),
    })
}

struct AgentUsageReader {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    num_turns: u64,
    total_cost_usd: f64,
}

/// Build a custom termimad skin: compact spacing, colourful header levels,
/// high-contrast bold, bright code highlighting.
fn build_markdown_skin() -> termimad::MadSkin {
    use termimad::crossterm::style::{Attribute, Color};
    use termimad::{CompoundStyle, LineStyle, MadSkin, StyledChar};

    let mut skin = MadSkin::default();

    // Headers — each level gets a distinct bright colour. No underline, no
    // centring, no background — just bold colour so the hierarchy pops
    // without wasting vertical space.
    let header_colours = [
        Color::Cyan,     // H1
        Color::Magenta,  // H2
        Color::Yellow,   // H3
        Color::Blue,     // H4
        Color::Green,    // H5
        Color::DarkCyan, // H6
        Color::DarkMagenta,
        Color::DarkYellow,
    ];
    for (i, h) in skin.headers.iter_mut().enumerate() {
        *h = LineStyle::default();
        h.compound_style = CompoundStyle::with_fg(*header_colours.get(i).unwrap_or(&Color::White));
        h.compound_style.add_attr(Attribute::Bold);
    }

    // Bold / italic / inline code: bright colours for contrast.
    skin.bold = CompoundStyle::with_fg(Color::White);
    skin.bold.add_attr(Attribute::Bold);

    skin.italic = CompoundStyle::with_fg(Color::Cyan);
    skin.italic.add_attr(Attribute::Italic);

    skin.inline_code = CompoundStyle::with_fgbg(Color::Yellow, Color::Reset);
    skin.inline_code.add_attr(Attribute::Bold);

    // Code block: subtle grey background + bright foreground.
    skin.code_block.compound_style = CompoundStyle::with_fgbg(
        Color::Rgb {
            r: 220,
            g: 220,
            b: 220,
        },
        Color::Rgb {
            r: 30,
            g: 30,
            b: 40,
        },
    );

    // Bullets and other decorations.
    skin.bullet = StyledChar::from_fg_char(Color::Cyan, '▸');
    skin.quote_mark = StyledChar::from_fg_char(Color::Blue, '▌');
    skin.horizontal_rule = StyledChar::from_fg_char(Color::DarkGrey, '─');

    // Table borders — termimad's default `STANDARD_TABLE_BORDER_CHARS`
    // already uses the same Unicode box-drawing characters as
    // `comfy_table::presets::UTF8_FULL_CONDENSED` (│ ─ ┌┐└┘ ┬┴├┤┼), which
    // is the style used by every other `easynet` table (`output::table`).
    // We can't match the `╞═╪╡` header separator or `┆` dashed inside
    // rulers because `TableBorderChars` is uniform, but the overall look
    // is consistent. We only need to colour the border to match the dim
    // grey the EasyNet CLI uses for table chrome.
    skin.table.compound_style.set_fg(Color::DarkGrey);

    skin
}

/// Collapse consecutive blank lines down to a single blank line and strip
/// leading/trailing blanks, so the rendered output stays compact without
/// fighting termimad's layout engine.
fn compact_markdown(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut prev_blank = true; // treat start-of-doc as already-blank
    for line in src.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank && prev_blank {
            continue; // skip duplicate blank lines
        }
        out.push_str(line);
        out.push('\n');
        prev_blank = is_blank;
    }
    // Trim trailing blank line.
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

fn run_doctor(args: DoctorArgs) -> anyhow::Result<()> {
    let registry = agents::load_agents()?;

    let agents_to_check: Vec<(String, AgentType)> = match args.name {
        Some(name) => {
            if let Some(entry) = registry.agents.get(&name) {
                vec![(name, entry.agent_type)]
            } else {
                anyhow::bail!("agent '{}' not found", name);
            }
        }
        None => {
            if registry.agents.is_empty() {
                // Check both CLIs even if no agents registered.
                vec![
                    ("claude-code".to_string(), AgentType::ClaudeCode),
                    ("codex".to_string(), AgentType::Codex),
                ]
            } else {
                registry
                    .agents
                    .iter()
                    .map(|(n, e)| (n.clone(), e.agent_type))
                    .collect()
            }
        }
    };

    let mut all_ok = true;
    eprintln!();

    for (name, agent_type) in &agents_to_check {
        let result = match agent_type {
            AgentType::ClaudeCode => claude_code::doctor(),
            AgentType::Codex | AgentType::CodexAppServer => codex::doctor(),
        };
        match result {
            Ok(version) => {
                eprintln!(
                    "  {:<14} {}",
                    style(name).white().bold(),
                    style(version).dim(),
                );
            }
            Err(e) => {
                eprintln!(
                    "  {:<14} {}",
                    style(name).white().bold(),
                    style(format!("unavailable: {e}")).red(),
                );
                all_ok = false;
            }
        }
    }

    eprintln!();
    if !all_ok {
        eprintln!("  Install missing CLIs:");
        eprintln!(
            "  Claude Code  {}",
            style("https://claude.ai/download").dim()
        );
        eprintln!(
            "  Codex        {}",
            style("npm install -g @openai/codex").dim()
        );
        eprintln!();
    }

    Ok(())
}

/// Look up the on-disk root for a registered agent. Returns a
/// typed error if the registry has no row for that name, or if
/// the row's root is missing / unparseable. Shared between
/// `agent abilities` and `agent publish`: both need to open
/// `<agent-root>/abilities/*` and both must fail with the same
/// phrasing when the agent is unknown.
fn open_registered_agent(name: &str) -> anyhow::Result<AgentDirectory> {
    let registry = agents::load_agents()?;
    let entry = registry.agents.get(name).ok_or_else(|| {
        anyhow::anyhow!(
            "agent '{name}' is not registered; run `easynet agent list` to see \
             registered names, or `easynet agent add {name} --type …` to register it"
        )
    })?;
    let root = entry
        .root_path
        .clone()
        .unwrap_or_else(|| config::agents_root().join(name));
    if !root.exists() {
        anyhow::bail!(
            "agent '{name}' has no on-disk root at {}. Either the directory was \
             removed (run `agent prune` to clear the row) or the path is stale. \
             Re-register with `agent add {name} --type …` to re-materialize.",
            root.display()
        );
    }
    AgentDirectory::open(&root)
}

fn run_abilities(args: AbilitiesArgs) -> anyhow::Result<()> {
    let dir = open_registered_agent(&args.name)?;
    let manifests = dir.list_ability_manifests()?;

    eprintln!();
    if manifests.is_empty() {
        // "No abilities" is a legitimate — if unusual — shape on
        // disk. An operator can manually empty `abilities/` to
        // temporarily hide the agent from network discovery. We
        // print the empty-list message explicitly so it's
        // observable without the operator having to guess
        // whether parsing silently failed.
        eprintln!(
            "  {} {}",
            style("No abilities declared under").dim(),
            style(dir.abilities_dir().display().to_string()).cyan(),
        );
        eprintln!(
            "  {}",
            style("Drop a `<verb>.ability.toml` into that directory to declare one.").dim(),
        );
        eprintln!();
        return Ok(());
    }

    eprintln!(
        "  {} {}",
        style("agent").dim(),
        style(&args.name).white().bold(),
    );
    eprintln!();
    eprintln!(
        "  {:<28} {:<12} {}",
        style("ABILITY").dim(),
        style("TIMEOUT").dim(),
        style("DESCRIPTION").dim(),
    );
    eprintln!("  {}", style("─".repeat(72)).dim());
    for m in &manifests {
        let qualified = m.qualified_name(&args.name);
        let timeout = m
            .timeout_seconds()
            .map(|s| format!("{s}s"))
            .unwrap_or_else(|| "-".to_string());
        // One-line description; truncate overlong blurbs to keep
        // the table readable. The full text is always on disk.
        let desc: String = m.description().chars().take(60).collect();
        let ellipsis = if m.description().chars().count() > 60 {
            "…"
        } else {
            ""
        };
        eprintln!(
            "  {:<28} {:<12} {}{}",
            style(qualified).cyan(),
            style(timeout).dim(),
            desc,
            ellipsis,
        );
    }
    eprintln!();
    Ok(())
}

fn run_publish(args: PublishArgs) -> anyhow::Result<()> {
    if !args.dry_run {
        // Live publishing is gated until the cross-repo publish
        // spec + implementation lands. Returning a clear error
        // here — rather than silently calling through to a
        // future Axon path — keeps the flag's contract honest:
        // today every successful `agent publish` is a dry-run.
        anyhow::bail!(
            "only `--dry-run` is supported in this release. Live publishing through \
             Axon lands in a later PR. Re-run with `--dry-run` to preview the \
             `<agent>.<ability>` tools that would be registered."
        );
    }

    let dir = open_registered_agent(&args.name)?;
    let manifests = dir.list_ability_manifests()?;

    eprintln!();
    eprintln!(
        "  {} {}  {}",
        style("dry-run:").yellow(),
        style(format!("agent publish {}", args.name)).white().bold(),
        style(format!("root={}", dir.root().display())).dim(),
    );
    eprintln!();

    if manifests.is_empty() {
        eprintln!(
            "  {}",
            style("Nothing to advertise: abilities/ is empty or missing.").dim(),
        );
        eprintln!();
        return Ok(());
    }

    // Emit one line per planned ToolSpec registration. The
    // lines are `<qualified>\t<input_schema_shape>\t<output>` so
    // a downstream consumer (`diff`, an ops script) can parse
    // them with awk. The decorative styling only affects TTY
    // output; `console::style` degrades to plain ASCII when the
    // sink is not a terminal.
    eprintln!(
        "  {:<28} {:<18} {}",
        style("QUALIFIED NAME").dim(),
        style("INPUT SHAPE").dim(),
        style("OUTPUT SHAPE").dim(),
    );
    eprintln!("  {}", style("─".repeat(72)).dim());

    for m in &manifests {
        let qualified = m.qualified_name(&args.name);
        // Render a one-line shape summary for each schema. A
        // full JSON Schema tree would flood the terminal; the
        // summary is "object(keys=prompt,context)" style. That
        // line is enough to spot a schema regression at a
        // glance; full content lives on disk for anyone who
        // wants to inspect it.
        let input_shape = summarize_schema(m.input_schema());
        let output_shape = m
            .output_schema()
            .map(summarize_schema)
            .unwrap_or_else(|| "-".to_string());
        eprintln!(
            "  {:<28} {:<18} {}",
            style(qualified).cyan(),
            style(input_shape).white(),
            style(output_shape).dim(),
        );
    }

    eprintln!();
    eprintln!(
        "  {} {}",
        style("would advertise").green(),
        style(format!(
            "{} ability{} in the node roster label",
            manifests.len(),
            if manifests.len() == 1 { "" } else { "s" }
        ))
        .white()
        .bold(),
    );
    eprintln!(
        "  {}",
        style("(dry-run — no Axon calls, no registry mutation)").dim(),
    );
    eprintln!();
    Ok(())
}

/// One-line shape summary for a JSON Schema root — used by the
/// publish dry-run table. Deliberately coarse: the intent is "spot
/// a regression at a glance", not "fully re-render the schema".
fn summarize_schema(schema: &serde_json::Value) -> String {
    let obj = match schema.as_object() {
        Some(o) => o,
        None => return format!("{:?}", schema),
    };
    let ty = obj.get("type").and_then(|v| v.as_str()).unwrap_or("?");
    // Dead-by-contract: AbilityManifest::validate() rejects any
    // input_schema or output_schema whose top-level is not an
    // object, so both schemas reaching this helper are objects.
    // Kept as a belt-and-braces fallback so a future API widening
    // ("accept a top-level $ref") doesn't panic the dry-run table;
    // the render degrades to a single type word instead.
    if ty != "object" {
        return ty.to_string();
    }
    let mut keys: Vec<&str> = obj
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default();
    keys.sort();
    let required: std::collections::HashSet<&str> = obj
        .get("required")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    // Mark required keys with a trailing `!` so the summary
    // distinguishes "prompt (required)" from "context (optional)"
    // without expanding the column width.
    let rendered: Vec<String> = keys
        .iter()
        .map(|k| {
            if required.contains(k) {
                format!("{k}!")
            } else {
                (*k).to_string()
            }
        })
        .collect();
    if rendered.is_empty() {
        "object".to_string()
    } else {
        format!("object({})", rendered.join(","))
    }
}

/// `easynet agent refresh` — re-issue `runtime.register_local_tool`
/// for every daemon-owned ability the workspace currently declares.
///
/// Use this after authoring a new `<agent>/abilities/<verb>.ability.toml`
/// (or after running `easynet agent add <name>` while the daemon is
/// alive) to make the new ability invokable cross-process without
/// restarting the daemon.
///
/// In-process invocations (the agent calling its own ability via the
/// daemon's local MCP bridge) work the moment the TOML lands —
/// `chat_ability::register_dynamic_agent_fallback` consults the
/// workspace at lookup time. This command exists so the same view
/// reaches axon-runtime's `runtime_local_tools` registry, which is
/// what cross-process Invokes (frontend Abilities page,
/// `bridge.ability_call_raw` from another process, etc.) consult.
///
/// Best-effort: bridge connect / register failures are reported but
/// the command's exit code only reflects whether the bridge connect
/// succeeded — partial registration is the same shape this same path
/// already takes during boot.
fn run_refresh() -> anyhow::Result<()> {
    let (bridge, _state) = crate::persistence::config::load_and_connect().map_err(|e| {
        anyhow::anyhow!(
            "could not reach local axon-runtime: {e}; run `easynet runtime start` first"
        )
    })?;
    let creds = crate::persistence::config::load_credentials()
        .map_err(|e| anyhow::anyhow!("load credentials: {e}"))?;
    let username = crate::facade::cli::start::bootstrap_username_for(&creds);
    let plan = crate::facade::cli::start::build_bootstrap_plan_from(
        &creds.tenant_id,
        &creds.node_id,
        &username,
    )?;
    if plan.realm.is_empty() {
        anyhow::bail!(
            "daemon is not joined to a realm yet; run `easynet join <token>` before refresh"
        );
    }
    // Pin the caller URI for the refresh sweep — same reason as
    // boot's start.rs and post-add publish_to_local_runtime: the
    // bridge needs an explicit URA so hub-shaped envelopes carry
    // a proper caller, not the literal-subject fallback that fails
    // the membership gate.
    let invoker = crate::runtime::advertise::BridgeAbilityInvoker::with_caller_uri(
        &bridge,
        plan.host_device_uri.clone(),
    );
    let dispatch_endpoint = crate::services::control::runtime_dispatch::dispatch_endpoint_uri();
    let outcomes = crate::runtime::publish::register_local_tools_via_runtime(
        &invoker,
        &creds.tenant_id,
        &plan.realm,
        &creds.node_id,
        &dispatch_endpoint,
    );
    let mut ok = 0usize;
    let mut total = 0usize;
    for o in &outcomes {
        total += 1;
        match &o.result {
            Ok(_) => ok += 1,
            Err(msg) => {
                output::warn(&format!("refresh {} failed: {msg}", o.label));
            }
        }
    }
    output::success(&format!(
        "{ok}/{total} runtime.register_local_tool calls succeeded; \
         daemon-owned abilities are now invokable cross-process."
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eal_string_literal_quotes_and_escapes_metachars() {
        // Round-trip property: every char that would either terminate
        // the literal or change the lexer's byte-skipping behaviour
        // must be escaped. The EAL lexer is intentionally non-decoding
        // (it only uses `\` to skip the next byte), so we don't need
        // numeric `\uXXXX` escapes — just the quote/backslash pair plus
        // the readability escapes for newline/tab.
        assert_eq!(eal_string_literal("hello").unwrap(), "\"hello\"");
        assert_eq!(eal_string_literal(r#"a"b"#).unwrap(), r#""a\"b""#);
        assert_eq!(eal_string_literal(r"a\b").unwrap(), r#""a\\b""#);
        assert_eq!(eal_string_literal("a\nb").unwrap(), r#""a\nb""#);
    }

    #[test]
    fn eal_string_literal_rejects_embedded_nul() {
        // Downstream agent CLIs treat the prompt as a C string and
        // silently truncate at the first NUL. Surface the bad input as
        // an error at the CLI layer rather than delivering a corrupt
        // half-prompt to the model.
        let err = eal_string_literal("good\0bad").unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("NUL"),
            "expected NUL-rejection error, got: {msg}"
        );
    }

    // ── v2 CLI verbs ────────────────────────────────────────────────────

    use crate::facade::cli::test_support::HomeGuard;
    use std::fs;

    /// Build the AddArgs shape the CLI surface would construct
    /// for `easynet agent add <name> --type <t> --model <m>`.
    /// We don't drive clap here — we exercise the `run_add`
    /// body directly, which is the contract-bearing surface.
    fn add_args(name: &str, r#type: &str, model: Option<&str>) -> AddArgs {
        AddArgs {
            name: name.into(),
            r#type: r#type.into(),
            model: model.map(str::to_string),
            label: None,
        }
    }

    #[test]
    fn hub_invoke_endpoint_rewrites_axon_scheme_to_http() {
        assert_eq!(
            hub_invoke_endpoint("axon://hub.example:50051").unwrap(),
            "http://hub.example:50051"
        );
        assert_eq!(
            hub_invoke_endpoint("https://hub.example:50443").unwrap(),
            "https://hub.example:50443"
        );
        assert_eq!(
            hub_invoke_endpoint("hub.example:50051").unwrap(),
            "http://hub.example:50051"
        );
    }

    #[test]
    fn ability_name_from_resource_uri_extracts_canonical_hub_ability_id() {
        // Canonical hub-owned ability: reserved owner `hub`, agent
        // namespace `federation`, verb `advertise_agent`.
        assert_eq!(
            ability_name_from_resource_uri(
                "easynet:///r/acme/ability/hub.federation.advertise_agent"
            )
            .as_deref(),
            Some("federation.advertise_agent")
        );
        assert_eq!(ability_name_from_resource_uri("not-a-resource-uri"), None);
    }

    #[test]
    fn ability_name_from_resource_uri_rejects_retired_legacy_shape() {
        assert_eq!(
            ability_name_from_resource_uri(
                "easynet:///r/prv/hub/acme/abilities/federation.advertise_agent@1?tenant_id=acme"
            ),
            None
        );
    }

    #[test]
    fn direct_hub_trust_match_prefers_env_override() {
        let _home = HomeGuard::new();
        let tmp = tempfile::tempdir().expect("tempdir");
        let override_path = tmp.path().join("override.toml");
        fs::write(
            &override_path,
            r#"
[[trusted_agent]]
agent_uri = "easynet:///r/test-realm/hub"
public_key_b64 = "AA=="
role = "hub"
added_at_unix_ms = 1
origin_tenant_id = "test-realm"
hub_uri = "https://hub.example:50443"
tls_ca_pem_path = "/tmp/env-ca.pem"
"#,
        )
        .expect("write env trust anchor");

        let prev = std::env::var_os("EASYNET_REALM_TRUST_PATH");
        std::env::set_var("EASYNET_REALM_TRUST_PATH", &override_path);
        struct EnvGuard(Option<std::ffi::OsString>);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(v) => std::env::set_var("EASYNET_REALM_TRUST_PATH", v),
                    None => std::env::remove_var("EASYNET_REALM_TRUST_PATH"),
                }
            }
        }
        let _guard = EnvGuard(prev);

        assert_eq!(
            direct_hub_trust_match("https://hub.example:50443"),
            Some(Some(std::path::PathBuf::from("/tmp/env-ca.pem")))
        );
    }

    #[test]
    fn direct_hub_trust_match_falls_back_to_home_paths() {
        let _home = HomeGuard::new();
        let prev = std::env::var_os("EASYNET_REALM_TRUST_PATH");
        std::env::remove_var("EASYNET_REALM_TRUST_PATH");
        struct EnvGuard(Option<std::ffi::OsString>);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(v) => std::env::set_var("EASYNET_REALM_TRUST_PATH", v),
                    None => std::env::remove_var("EASYNET_REALM_TRUST_PATH"),
                }
            }
        }
        let _guard = EnvGuard(prev);

        let home = std::path::PathBuf::from(std::env::var_os("HOME").expect("home"));
        fs::write(
            home.join("realm-trust.toml"),
            r#"
[[trusted_agent]]
agent_uri = "easynet:///r/test-realm/hub"
public_key_b64 = "AA=="
role = "hub"
added_at_unix_ms = 1
origin_tenant_id = "test-realm"
hub_uri = "https://hub.example:50443"
tls_ca_pem_path = "/tmp/home-ca.pem"
"#,
        )
        .expect("write home trust anchor");

        assert_eq!(
            direct_hub_trust_match("https://hub.example:50443"),
            Some(Some(std::path::PathBuf::from("/tmp/home-ca.pem")))
        );
    }

    #[test]
    fn publish_outcome_summary_requires_all_directory_rows_to_succeed() {
        let outcomes = vec![
            crate::runtime::publish::PublishOutcome {
                agent_uri: String::new(),
                label: "local-agents.json".into(),
                result: Err("save failed".into()),
            },
            crate::runtime::publish::PublishOutcome {
                agent_uri: "device:1".into(),
                label: "device".into(),
                result: Ok(()),
            },
            crate::runtime::publish::PublishOutcome {
                agent_uri: "agent:alice".into(),
                label: "abilities/3".into(),
                result: Err("membership required".into()),
            },
        ];

        let err = publish_outcome_summary(&outcomes).unwrap_err();
        assert!(err.contains("agent:alice abilities/3: membership required"));
        assert!(!err.contains("local-agents.json"));
    }

    #[test]
    fn publish_outcome_summary_accepts_clean_directory_rows() {
        let outcomes = vec![
            crate::runtime::publish::PublishOutcome {
                agent_uri: String::new(),
                label: "local-agents.json".into(),
                result: Err("save failed".into()),
            },
            crate::runtime::publish::PublishOutcome {
                agent_uri: "device:1".into(),
                label: "device".into(),
                result: Ok(()),
            },
            crate::runtime::publish::PublishOutcome {
                agent_uri: "agent:alice".into(),
                label: "abilities/3".into(),
                result: Ok(()),
            },
        ];

        publish_outcome_summary(&outcomes).expect("directory rows succeeded");
    }

    #[test]
    fn run_add_writes_v2_row_and_materializes_agent_directory() {
        // Fresh add must: (a) insert a v2 registry row
        // carrying `root_path` + `schema_version=2`; (b)
        // create the agent directory on disk with an
        // `agent.toml` that reflects the CLI flags; (c) leave
        // the fat fields (`command`, `args`) empty so they
        // omit on serialize — the whole point of the CLI
        // rewrite is that v2 rows do not carry vestigial v1
        // data.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", Some("claude-opus-4-7"))).unwrap();

        let registry = agents::load_agents().unwrap();
        let alice = registry.agents.get("alice").expect("alice registered");
        assert_eq!(alice.schema_version, CURRENT_REGISTRY_SCHEMA);
        assert!(alice.root_path.is_some());
        // Fat-field cleanliness: fresh v2 row must not carry
        // command / args from `AgentEntry::new`.
        assert!(alice.command.is_empty());
        assert!(alice.args.is_empty());

        // Directory materialized with a real agent.toml that
        // reflects the CLI flags.
        let root = alice.root_path.as_ref().unwrap();
        let toml = fs::read_to_string(root.join("agent.toml")).unwrap();
        let spec = AgentSpec::from_toml_str(&toml).unwrap();
        assert_eq!(spec.name, "alice");
        assert_eq!(spec.runtime, RuntimeKind::ClaudeCode);
        assert_eq!(spec.model.as_deref(), Some("claude-opus-4-7"));
    }

    #[test]
    fn run_add_update_preserves_operator_edits_to_agent_toml() {
        // Repeat `agent add` with a different model must
        // update the registry row (new model reflected in
        // `AgentEntry.model`) but NOT clobber a hand-written
        // `description` in agent.toml. The contract: CLI
        // flags update the registry-visible subset; operator
        // edits to agent.toml survive.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", Some("old-model"))).unwrap();

        let registry = agents::load_agents().unwrap();
        let root = registry.agents["alice"].root_path.as_ref().unwrap().clone();

        // Hand-edit agent.toml to add a description.
        let mut spec =
            AgentSpec::from_toml_str(&fs::read_to_string(root.join("agent.toml")).unwrap())
                .unwrap();
        spec.description = Some("user-edited".into());
        fs::write(root.join("agent.toml"), spec.to_toml_string().unwrap()).unwrap();

        // Re-run agent add with a different model.
        run_add(add_args("alice", "claude-code", Some("new-model"))).unwrap();

        // The operator's description must survive; we do not
        // rewrite agent.toml on update, we only update the
        // registry row.
        let spec2 = AgentSpec::from_toml_str(&fs::read_to_string(root.join("agent.toml")).unwrap())
            .unwrap();
        assert_eq!(spec2.description.as_deref(), Some("user-edited"));
    }

    #[test]
    fn run_remove_default_keeps_the_on_disk_root() {
        // `agent remove` without --purge must strip the
        // registry row but leave the directory (and its
        // `.env`, `runs/`, operator edits) intact. The rule
        // is "default to non-destructive"; credentials are at
        // stake and a second `agent add` on the same name can
        // legitimately want the old history back.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        let root = agents::load_agents().unwrap().agents["alice"]
            .root_path
            .clone()
            .unwrap();
        assert!(root.join("agent.toml").exists());

        run_remove(RemoveArgs {
            name: "alice".into(),
            purge: false,
        })
        .unwrap();

        // Registry row gone.
        assert!(!agents::load_agents().unwrap().agents.contains_key("alice"));
        // Directory still present.
        assert!(
            root.join("agent.toml").exists(),
            "--purge not passed: dir must stay"
        );
    }

    #[test]
    fn run_remove_with_purge_deletes_the_on_disk_root() {
        // `agent remove --purge` deletes the directory too.
        // This is the explicit destructive path.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        let root = agents::load_agents().unwrap().agents["alice"]
            .root_path
            .clone()
            .unwrap();

        run_remove(RemoveArgs {
            name: "alice".into(),
            purge: true,
        })
        .unwrap();

        assert!(
            !root.exists(),
            "--purge must delete the directory, but {} still exists",
            root.display()
        );
    }

    fn set_args(name: &str, model: Option<&str>) -> SetArgs {
        SetArgs {
            name: name.into(),
            model: model.map(str::to_string),
        }
    }

    #[test]
    fn run_set_changes_model_in_both_agent_toml_and_registry_row() {
        // The on-disk `agent.toml` and the registry row must agree
        // after `agent set --model X`. Earlier versions only
        // updated one; the discrepancy showed up later as
        // "claude reports sonnet, but `agent list` shows opus" —
        // the contract here pins both.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", Some("sonnet"))).unwrap();

        run_set(set_args("alice", Some("opus"))).unwrap();

        // Registry row updated.
        let entry = agents::load_agents().unwrap().agents["alice"].clone();
        assert_eq!(entry.model.as_deref(), Some("opus"));

        // agent.toml on disk updated.
        let root = entry.root_path.clone().unwrap();
        let spec = AgentSpec::from_toml_str(&fs::read_to_string(root.join("agent.toml")).unwrap())
            .unwrap();
        assert_eq!(spec.model.as_deref(), Some("opus"));
    }

    #[test]
    fn run_set_with_empty_model_string_clears_the_field() {
        // Passing `--model ''` is the explicit CLEAR signal:
        // the agent should fall back to the underlying CLI's
        // default model. This is the load-bearing distinction
        // between "no flag passed" (no change) and "flag with
        // empty value" (clear).
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", Some("sonnet"))).unwrap();

        run_set(set_args("alice", Some(""))).unwrap();

        let entry = agents::load_agents().unwrap().agents["alice"].clone();
        assert!(
            entry.model.is_none(),
            "empty-string --model must clear; got {:?}",
            entry.model
        );
        // agent.toml round-trips with no `model` field.
        let root = entry.root_path.clone().unwrap();
        let body = fs::read_to_string(root.join("agent.toml")).unwrap();
        assert!(
            !body.contains("model ="),
            "cleared model must not be persisted; got:\n{body}"
        );
    }

    #[test]
    fn run_set_rejects_unknown_agent_with_actionable_message() {
        // No false positives — `agent set nonexistent --model X`
        // must fail with a clear message pointing at `agent list`,
        // not silently create a row (which would be a footgun:
        // operator typos a name and gets a phantom agent).
        let _g = HomeGuard::new();
        let err = run_set(set_args("nonexistent", Some("sonnet"))).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("not registered"), "msg: {msg}");
        assert!(msg.contains("agent list"), "msg should hint list: {msg}");
    }

    #[test]
    fn run_set_with_no_flags_errors_explicitly() {
        // `agent set alice` (no --model) is meaningless today.
        // We could silently no-op, but that risks operators
        // believing they changed something when they didn't.
        // Explicit error is friendlier.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", Some("sonnet"))).unwrap();
        let err = run_set(set_args("alice", None)).unwrap_err();
        assert!(format!("{err}").contains("nothing to change"));
    }

    #[test]
    fn run_set_does_not_validate_model_string_against_any_allow_list() {
        // Per the SetArgs::model doc: claude/codex CLIs accept any
        // string and resolve aliases at their own discretion. Even
        // a deliberately-wrong-looking name must round-trip — the
        // validation belongs at invocation time, not at
        // configuration time. This pins the no-allow-list policy.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        run_set(set_args("alice", Some("definitely-not-a-real-model-xyz"))).unwrap();
        let entry = agents::load_agents().unwrap().agents["alice"].clone();
        assert_eq!(
            entry.model.as_deref(),
            Some("definitely-not-a-real-model-xyz")
        );
    }

    #[test]
    fn run_prune_removes_orphaned_rows_only() {
        // With two agents — one whose root exists, one whose
        // root has been deleted — `prune` must remove only
        // the orphan. The surviving one must stay, and both
        // its directory and its row must be intact.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        run_add(add_args("bob", "codex", None)).unwrap();

        // Orphan bob by deleting its root.
        let bob_root = agents::load_agents().unwrap().agents["bob"]
            .root_path
            .clone()
            .unwrap();
        fs::remove_dir_all(&bob_root).unwrap();

        run_prune(PruneArgs { dry_run: false }).unwrap();

        let registry = agents::load_agents().unwrap();
        assert!(registry.agents.contains_key("alice"), "alice must survive");
        assert!(!registry.agents.contains_key("bob"), "bob must be pruned");
    }

    #[test]
    fn run_prune_dry_run_leaves_registry_unchanged() {
        // The `--dry-run` contract is "no mutations". Rows
        // reported as "would prune" must still be present in
        // the registry after the command returns.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        let root = agents::load_agents().unwrap().agents["alice"]
            .root_path
            .clone()
            .unwrap();
        fs::remove_dir_all(&root).unwrap();

        run_prune(PruneArgs { dry_run: true }).unwrap();

        // Row must still be present — dry-run MUST NOT
        // mutate the registry. This is the load-bearing
        // property that makes `prune --dry-run` safe to run
        // as a recon step.
        assert!(agents::load_agents().unwrap().agents.contains_key("alice"));
    }

    // ── abilities / publish dry-run ─────────────────────────────────────

    #[test]
    fn run_abilities_lists_the_seeded_chat_manifest_for_a_fresh_agent() {
        // Fresh `agent add` always ships a default chat manifest.
        // `agent abilities` must surface it exactly once with its
        // fully-qualified `<agent>.chat` name.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        // Happy path is "no error"; we leave the eprintln-based
        // output un-asserted (tested at helper level via
        // list_ability_manifests).
        run_abilities(AbilitiesArgs {
            name: "alice".into(),
        })
        .expect("fresh agent must list its seeded chat manifest");
    }

    #[test]
    fn run_abilities_reports_the_unknown_agent_as_an_error() {
        // `agent abilities <unknown>` must fail loud — we do not
        // want the empty-list path to mask a typo'd agent name.
        let _g = HomeGuard::new();
        let err = run_abilities(AbilitiesArgs {
            name: "nobody".into(),
        })
        .expect_err("unknown agent must error");
        let msg = format!("{err}");
        assert!(
            msg.contains("nobody"),
            "error must name the missing agent: {msg}"
        );
        assert!(
            msg.contains("not registered") || msg.contains("add"),
            "error must hint at remediation: {msg}"
        );
    }

    #[test]
    fn run_abilities_reports_missing_root_as_an_error() {
        // A row whose root was `rm -rf`d must not silently fall
        // through to "empty abilities list" — the operator needs
        // to see the true cause.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        let root = agents::load_agents().unwrap().agents["alice"]
            .root_path
            .clone()
            .unwrap();
        fs::remove_dir_all(&root).unwrap();
        let err = run_abilities(AbilitiesArgs {
            name: "alice".into(),
        })
        .expect_err("orphan row must error on `agent abilities`");
        assert!(format!("{err}").contains("no on-disk root"));
    }

    #[test]
    fn run_abilities_handles_empty_abilities_directory_without_error() {
        // An operator can legitimately remove every manifest to
        // hide the agent from discovery. That must succeed with
        // no panic, no error — just an empty-list signal.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        let root = agents::load_agents().unwrap().agents["alice"]
            .root_path
            .clone()
            .unwrap();
        // Wipe the seeded default.
        fs::remove_dir_all(root.join("abilities")).unwrap();
        fs::create_dir_all(root.join("abilities")).unwrap();
        run_abilities(AbilitiesArgs {
            name: "alice".into(),
        })
        .expect("empty abilities dir must be non-fatal");
    }

    #[test]
    fn run_abilities_surfaces_manifest_parse_errors() {
        // A malformed manifest must surface as an error — silent
        // skip would hide it from the operator reviewing their
        // ability set before a publish.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        let root = agents::load_agents().unwrap().agents["alice"]
            .root_path
            .clone()
            .unwrap();
        fs::write(
            root.join("abilities").join("bad.ability.toml"),
            "not = valid = toml",
        )
        .unwrap();
        let err = run_abilities(AbilitiesArgs {
            name: "alice".into(),
        })
        .expect_err("malformed manifest must surface");
        assert!(format!("{err}").contains("bad"));
    }

    #[test]
    fn run_publish_dry_run_succeeds_on_a_fresh_agent() {
        // The whole point of PR-4: dry-run shows what a future
        // publish would register without calling Axon. It must
        // succeed on a freshly-added agent (which has exactly
        // one seeded chat manifest).
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        run_publish(PublishArgs {
            name: "alice".into(),
            dry_run: true,
        })
        .expect("dry-run must succeed on a fresh agent");
    }

    #[test]
    fn run_publish_requires_dry_run_flag_for_now() {
        // Until live publishing lands, we refuse to let scripts
        // call `agent publish <name>` without `--dry-run`. This
        // prevents a silent behaviour change when the live path
        // arrives.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        let err = run_publish(PublishArgs {
            name: "alice".into(),
            dry_run: false,
        })
        .expect_err("non-dry-run must be refused in this release");
        let msg = format!("{err}");
        assert!(msg.contains("dry-run"), "error must name the flag: {msg}");
    }

    #[test]
    fn run_publish_reports_unknown_agent_before_checking_flags() {
        // An unknown agent name is a different error than
        // "flag not set". The unknown-agent check happens even
        // when --dry-run is passed, so the operator sees the
        // most-specific error first.
        let _g = HomeGuard::new();
        let err = run_publish(PublishArgs {
            name: "nobody".into(),
            dry_run: true,
        })
        .expect_err("unknown agent must error");
        assert!(format!("{err}").contains("nobody"));
    }

    #[test]
    fn run_publish_dry_run_works_even_with_empty_abilities() {
        // An agent with zero manifests is a legitimate state;
        // dry-run prints "Nothing to advertise" rather than
        // erroring. The rationale: `agent publish --dry-run` is
        // a read-only diagnostic; forcing it to fail on an
        // empty set would make it unusable as a preflight check
        // during partial setup.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        let root = agents::load_agents().unwrap().agents["alice"]
            .root_path
            .clone()
            .unwrap();
        fs::remove_dir_all(root.join("abilities")).unwrap();
        fs::create_dir_all(root.join("abilities")).unwrap();
        run_publish(PublishArgs {
            name: "alice".into(),
            dry_run: true,
        })
        .expect("dry-run over empty abilities must not error");
    }

    #[test]
    fn run_publish_dry_run_does_not_mutate_registry_or_filesystem() {
        // Pinning the "no mutation" contract. If a future
        // refactor accidentally made dry-run touch state, this
        // test would catch it — compare registry bytes and the
        // abilities directory modtime before/after.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", None)).unwrap();
        let root = agents::load_agents().unwrap().agents["alice"]
            .root_path
            .clone()
            .unwrap();

        let registry_path = config::state_dir().join("agents.json");
        let before_registry = fs::read(&registry_path).unwrap();
        let before_ability = fs::read(root.join("abilities").join("chat.ability.toml")).unwrap();

        run_publish(PublishArgs {
            name: "alice".into(),
            dry_run: true,
        })
        .unwrap();

        let after_registry = fs::read(&registry_path).unwrap();
        let after_ability = fs::read(root.join("abilities").join("chat.ability.toml")).unwrap();
        assert_eq!(
            before_registry, after_registry,
            "dry-run must not touch the registry"
        );
        assert_eq!(
            before_ability, after_ability,
            "dry-run must not touch manifests"
        );
    }

    // ── summarize_schema helper ──────────────────────────────────────────

    #[test]
    fn summarize_schema_emits_object_keys_with_required_marker() {
        // The one-line shape summary is what the dry-run table
        // shows; the test pins the format so a reader of the
        // summary can tell "required" from "optional" at a glance.
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": {"type": "string"},
                "context": {"type": "string"}
            },
            "required": ["prompt"]
        });
        assert_eq!(summarize_schema(&schema), "object(context,prompt!)");
    }

    #[test]
    fn summarize_schema_handles_non_object_type() {
        let schema = serde_json::json!({"type": "string"});
        assert_eq!(summarize_schema(&schema), "string");
    }

    #[test]
    fn summarize_schema_handles_object_with_no_properties() {
        let schema = serde_json::json!({"type": "object"});
        assert_eq!(summarize_schema(&schema), "object");
    }

    #[test]
    fn run_add_refuses_when_root_carries_agent_toml_but_registry_empty() {
        // Defensive: someone has `agent.toml` at
        // `<agents_root>/alice/` (maybe copied from another
        // machine, maybe a prior install) but the registry
        // doesn't know about it. We must not silently adopt
        // it — the operator should import it explicitly so
        // they see what runtime / model / description
        // travelled with the file.
        let _g = HomeGuard::new();
        // Materialize the directory by hand.
        let root = config::agents_root().join("alice");
        let spec = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        AgentDirectory::create(&Location::Local { root: root.clone() }, spec).unwrap();
        assert!(root.join("agent.toml").exists());

        let err = run_add(add_args("alice", "claude-code", None))
            .expect_err("must refuse to adopt pre-existing agent.toml");
        let msg = format!("{err}");
        assert!(
            msg.contains("agent.toml") || msg.contains("already"),
            "error must name the conflict; got {msg}"
        );
    }
}
