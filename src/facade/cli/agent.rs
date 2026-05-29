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

use clap::{Args, Subcommand, ValueEnum};
use console::style;
use serde_json::Value;
use std::time::Duration;

use crate::facade::cli::mission_runs::{self, MissionRunOpts};
use crate::facade::cli::{
    daemon_agent_view,
    daemon_agent_view::{AgentRuntimeKind, DaemonAgentRow},
};
use crate::persistence::config;
use crate::runtime::directory::AgentDirectory;
use crate::runtime::drivers::{claude_code, codex};
use crate::support::local_daemon_grpc::LocalDaemonAbilityClient;
use crate::support::output;
use crate::support::timeouts;

#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub action: AgentAction,
}

#[derive(Debug, Subcommand)]
pub enum AgentAction {
    /// Register a new daemon-owned AI agent. Requires a paired,
    /// running local runtime.
    Add(AddArgs),
    /// List daemon-owned registered agents. Requires a paired,
    /// running local runtime.
    List,
    /// Remove a daemon-owned registered agent. Requires a paired,
    /// running local runtime.
    Remove(RemoveArgs),
    /// Send a prompt to an agent and print the response.
    Send(SendArgs),
    /// Check agent CLI availability and authentication.
    Doctor(DoctorArgs),
    /// Remove registry rows whose on-disk root has gone missing.
    Prune(PruneArgs),
    /// List the abilities declared under `<agent-root>/abilities/`.
    Abilities(AbilitiesArgs),
    /// Bind configured upstream MCP tools into one agent.
    Mcp(McpArgs),
    /// Update fields of a daemon-owned registered agent in place.
    /// Currently supports `--model`. Requires a paired, running
    /// local runtime.
    Set(SetArgs),
    /// Dry-run: show what `<agent>.<ability>` tools would be published,
    /// without touching Axon. Live publishing lands in a later PR.
    Publish(PublishArgs),
    /// Ask the daemon to re-register one agent's, or every registered
    /// agent's, LocalRuntime handlers. Requires a paired, running
    /// local runtime. Use after authoring a new
    /// `<agent>/abilities/<verb>.ability.toml`; no daemon restart
    /// required.
    Refresh(RefreshArgs),
    /// Inspect this agent's persisted chat history (the JSONL log
    /// that --follow / --resume / --session-id on 'agent send' read).
    /// Distinct from 'agent session' (singular), which manages the
    /// per-caller memory dimension.
    #[command(name = "chat-history")]
    ChatHistory(ChatHistoryArgs),
}

#[derive(Debug, Args)]
pub struct RefreshArgs {
    /// Optional. Refresh only this agent's `<agent>.*` runtime
    /// handlers, including ability TOMLs. Omit to refresh every
    /// agent in `agents.json`.
    #[arg(long)]
    pub agent: Option<String>,
}

#[derive(Debug, Args)]
pub struct ChatHistoryArgs {
    /// Agent name whose sessions to inspect.
    pub name: String,
    #[command(subcommand)]
    pub action: ChatHistoryAction,
}

#[derive(Debug, Subcommand)]
pub enum ChatHistoryAction {
    /// List every recorded session for this agent, most-recent-first.
    List(ChatHistoryListArgs),
    /// Show one session's full transcript (every turn, in order).
    Show(ChatHistoryShowArgs),
}

#[derive(Debug, Args)]
pub struct ChatHistoryListArgs {
    /// Emit JSON instead of the human-readable table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ChatHistoryShowArgs {
    /// Session id to show (from `agent sessions list <name>`).
    pub session_id: String,
    /// Emit raw JSONL (the on-disk file verbatim) instead of the
    /// human-readable transcript.
    #[arg(long)]
    pub json: bool,
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
    /// opt-in because 'rm -rf' on a directory that carries an
    /// operator's '.env' credentials is a destructive action.
    #[arg(long)]
    pub purge: bool,
}

#[derive(Debug, Args)]
pub struct PruneArgs {
    /// Show what would be removed without mutating the
    /// registry. Pairs well with the "path missing" rows in
    /// 'agent list' — an operator running 'prune --dry-run'
    /// first sees exactly which rows will disappear.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
#[command(group = clap::ArgGroup::new("session_select").args(["follow", "session_id", "resume"]).multiple(false))]
pub struct SendArgs {
    /// Registered agent name (from `easynet agent list`).
    pub name: String,
    /// Prompt body sent verbatim to the agent.
    ///
    /// Optional only when --resume is the sole flag and no prompt
    /// follows: `easynet agent send <name> --resume` opens the
    /// picker, lets the operator select a prior session, marks
    /// that session as the new latest (so a later --follow lands
    /// on it), and exits without sending. Every other invocation
    /// requires a prompt.
    pub prompt: Option<String>,
    /// Prior conversation to prepend under a "## Context (previous
    /// discussion)" section. Use this to carry state across separate
    /// 'agent send' invocations when you want the agent to build on an
    /// earlier reply without re-pasting the history inline.
    #[arg(long, value_name = "TEXT")]
    pub context: Option<String>,
    /// Per-call deadline in seconds. LLM dispatches can legitimately
    /// take many minutes, so the default is 15 min
    /// ('support::timeouts::AGENT_SEND_DEFAULT_SECS'). '0' inherits the
    /// runtime default.
    #[arg(long, value_name = "SECS", default_value_t = timeouts::AGENT_SEND_DEFAULT_SECS)]
    pub timeout: u64,
    /// Write the raw stream-json trace (one event per line) to this file.
    /// The prompt is saved alongside it as '<file>.prompt.txt'.
    #[arg(long, value_name = "FILE")]
    pub trace: Option<std::path::PathBuf>,
    /// Continue the most-recent session for this agent. Reads the
    /// session_id from the agent's session log index. Mutually
    /// exclusive with --resume / --session-id; without any of the
    /// three a fresh session is minted.
    #[arg(long)]
    pub follow: bool,
    /// Pin the conversation to a specific session id (returned by
    /// 'easynet agent chat-history list <name>' or by an earlier
    /// 'agent send' reply).
    #[arg(long, value_name = "UUID")]
    pub session_id: Option<String>,
    /// Pick a prior session interactively from a numbered list. On
    /// non-TTY input the call fails with a clear message asking the
    /// caller to use --session-id instead.
    #[arg(long)]
    pub resume: bool,
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
pub struct McpArgs {
    #[command(subcommand)]
    pub action: McpAction,
}

#[derive(Debug, Subcommand)]
pub enum McpAction {
    /// Add upstream MCP tools as deterministic abilities on an agent.
    Add(McpAddArgs),
}

/// CLI adapter for [`crate::core::ability_spec::CostKind`].
///
/// **Why a separate enum.** `CostKind` lives in `core/`, which is the
/// zero-dependency ontology layer — it must not pull in `clap`. So we
/// mirror the four variants here as a clap-native `ValueEnum` and
/// convert with `into_core()`. The two enums must stay in lockstep;
/// `cost_kind_arg_round_trips_to_core_cost_kind` pins that invariant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CostKindArg {
    Free,
    ExternalMetered,
    LlmMetered,
    Unknown,
}

impl CostKindArg {
    fn into_core(self) -> crate::core::ability_spec::CostKind {
        use crate::core::ability_spec::CostKind;
        match self {
            CostKindArg::Free => CostKind::Free,
            CostKindArg::ExternalMetered => CostKind::ExternalMetered,
            CostKindArg::LlmMetered => CostKind::LlmMetered,
            CostKindArg::Unknown => CostKind::Unknown,
        }
    }
}

#[derive(Debug, Args)]
pub struct McpAddArgs {
    /// Registered agent name that will own the generated abilities.
    pub name: String,
    /// Optional upstream MCP server name from mcp_clients.json. Omit
    /// to bind tools from every configured server.
    #[arg(long)]
    pub server: Option<String>,
    /// Optional upstream tool name. Repeat to bind a subset. Omit to
    /// bind every tool reported by the selected server(s).
    #[arg(long = "tool")]
    pub tools: Vec<String>,
    /// Optional prefix for generated ability verbs. The default
    /// produces names like `mcp_wikipedia_search`.
    #[arg(long, default_value = "mcp")]
    pub prefix: String,
    /// Path to mcp_clients.json. Defaults to
    /// `$EASYNET_HOME/mcp_clients.json` or `~/.easynet/mcp_clients.json`.
    #[arg(long)]
    pub config: Option<std::path::PathBuf>,
    /// Print the manifests that would be written without touching
    /// the agent workspace.
    #[arg(long)]
    pub dry_run: bool,
    /// Replace an existing generated manifest when the target path
    /// already exists.
    #[arg(long)]
    pub overwrite: bool,
    /// Continue binding tools from other servers when one upstream
    /// fails tools/list.
    #[arg(long)]
    pub skip_unreachable: bool,
    /// Optional explicit cost bucket for every generated manifest.
    /// When omitted the manifest is written without a `[cost]` table,
    /// and the runtime's per-exec inference applies (`unknown` for
    /// MCP-backed tools, per the honesty rule). Pass this when the
    /// operator knows the upstream's real billing surface, so that
    /// discovery / MCP descriptions stamp the truth rather than the
    /// `cost not declared` placeholder.
    #[arg(long, value_enum)]
    pub cost_kind: Option<CostKindArg>,
    /// Free-form human label that accompanies `--cost-kind`. Only
    /// honoured when `--cost-kind` is also set; carrying a label
    /// without a kind would write a half-formed `[cost]` table that
    /// `AbilityManifest::validate` rejects. Example:
    /// `--cost-kind external-metered --cost-label "Google Maps API
    /// — $5 per 1000 requests"`.
    #[arg(long, requires = "cost_kind")]
    pub cost_label: Option<String>,
}

#[derive(Debug, Args)]
pub struct SetArgs {
    /// Registered agent name (from `easynet agent list`).
    pub name: String,
    /// New model identifier. Pass any string the underlying CLI
    /// accepts for --model. Both 'claude --model' and 'codex
    /// --model' accept aliases (e.g. sonnet, opus) or full names
    /// (e.g. claude-opus-4-7). Neither CLI exposes an enumeration
    /// surface, so we deliberately do NOT validate the value —
    /// that would force EasyNet to ship a stale allow-list.
    /// Validation happens at invocation time when the underlying
    /// CLI sees the flag.
    ///
    /// Pass an empty string (--model "") to CLEAR the model and
    /// let the CLI fall back to its own default.
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
        AgentAction::Mcp(a) => run_mcp(a),
        AgentAction::Set(a) => run_set(a),
        AgentAction::Publish(a) => run_publish(a),
        AgentAction::Refresh(args) => run_refresh(args),
        AgentAction::ChatHistory(a) => run_sessions(a),
    }
}

fn run_add(args: AddArgs) -> anyhow::Result<()> {
    let agent_type: AgentRuntimeKind = args.r#type.parse()?;
    let daemon_client = required_local_daemon_agent_client()?;
    let name = args.name.clone();
    let daemon_response = invoke_daemon_agent_start_required(
        &daemon_client,
        serde_json::json!({
            "name": name,
            "agent_type": agent_type.to_string(),
            "model": args.model,
            "model_present": true,
            "label": args.label,
            "materialize_directory": true,
            "update_existing_spec": false,
            "project_workspace": true,
        }),
    )?;
    let is_update = daemon_response
        .get("replaced_prior")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    if is_update {
        output::success(&format!("Updated agent '{}'", args.name));
    } else {
        output::success(&format!("Registered agent '{}'", args.name));
    }
    output::detail("type", &agent_type.to_string());
    if let Some(m) = daemon_response
        .get("model")
        .and_then(serde_json::Value::as_str)
    {
        output::detail("model", m);
    }
    if let Some(root) = daemon_response
        .get("root_path")
        .and_then(serde_json::Value::as_str)
    {
        output::detail("root", root);
    }
    if let Some(err) = daemon_response
        .get("workspace_projection_error")
        .and_then(serde_json::Value::as_str)
    {
        eprintln!(
            "[agent add warn] could not project workspace: {err}; \
             skills will land on first dispatch",
        );
    }

    render_agent_start_runtime_outcome(&args.name, &daemon_response);

    Ok(())
}

/// Resolve the canonical caller URA the CLI should sign its
/// loopback gRPC calls under when invoking daemon-hosted management
/// abilities. Built from device credentials via the same bootstrap
/// helper used at daemon start, so admission on the daemon's gRPC
/// service sees a consistent shape across all CLI → daemon hops.
///
/// Returns `None` when the device hasn't been paired yet
/// (credentials not on disk). The caller silently skips the notify
/// in that case — there's no daemon state to mutate either.
#[cfg(feature = "axon-pb")]
fn resolve_local_daemon_caller_ura() -> Option<String> {
    let creds = crate::persistence::config::load_credentials().ok()?;
    let username = crate::facade::cli::start::bootstrap_username_for(&creds);
    let plan = crate::facade::cli::start::build_bootstrap_plan_from(
        &creds.tenant_id,
        &creds.node_id,
        &username,
    )
    .ok()?;
    Some(plan.host_device_ura)
}

fn required_local_daemon_agent_client() -> anyhow::Result<LocalDaemonAbilityClient> {
    #[cfg(feature = "axon-pb")]
    let caller_ura = resolve_local_daemon_caller_ura();
    #[cfg(not(feature = "axon-pb"))]
    let caller_ura = None;

    LocalDaemonAbilityClient::for_agent_management(caller_ura).map_err(|msg| {
        anyhow::anyhow!(
            "agent registry is daemon-owned, but the local daemon Axon ability surface is \
             unavailable: {msg}"
        )
    })
}

/// Shared CLI→daemon ability invocation with stable error
/// prefixing. The named wrappers below stay as 1-line readers so a
/// `git grep invoke_daemon_agent_start` still surfaces the call
/// site, but the error-format policy and the `.invoke(...)`
/// transport call live in ONE place. A future expansion to typed
/// `invoke::<R>()` per PR-D in
/// `docs/rfc/industrial-textbook-followups-2026-05-29.md` lands
/// here without touching the wrappers.
fn invoke_daemon_ability_required(
    client: &LocalDaemonAbilityClient,
    ability: &str,
    payload: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    client
        .invoke(ability, payload)
        .map_err(|err| anyhow::anyhow!("{ability} failed: {err}"))
}

fn invoke_daemon_agent_start_required(
    client: &LocalDaemonAbilityClient,
    payload: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    invoke_daemon_ability_required(client, "device.agent.start", payload)
}

fn invoke_daemon_agent_stop_required(
    client: &LocalDaemonAbilityClient,
    name: &str,
) -> anyhow::Result<serde_json::Value> {
    invoke_daemon_ability_required(
        client,
        "device.agent.stop",
        serde_json::json!({ "name": name }),
    )
}

fn invoke_daemon_agent_refresh_required(
    client: &LocalDaemonAbilityClient,
    payload: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    invoke_daemon_ability_required(client, "device.agent.refresh", payload)
}

fn render_agent_start_runtime_outcome(name: &str, resp: &serde_json::Value) {
    let registered = resp
        .get("runtime_registered")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let failed = resp
        .get("runtime_failed")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if registered > 0 {
        output::detail(
            "runtime",
            &format!(
                "daemon registered {registered} `<{name}>.*` ability rows into LocalRuntime \
                 (failed: {failed})"
            ),
        );
    } else {
        output::detail(
            "runtime",
            "daemon accepted device.agent.start but registered 0 rows \
             (already present or registrar pending)",
        );
    }
}

fn render_agent_stop_runtime_outcome(name: &str, resp: &serde_json::Value) {
    let removed = resp
        .get("runtime_removed")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if removed > 0 {
        output::detail(
            "runtime",
            &format!("daemon unregistered {removed} `{name}.*` rows from LocalRuntime"),
        );
    }
}

fn run_list() -> anyhow::Result<()> {
    let daemon_client = required_local_daemon_agent_client()?;
    let rows = invoke_daemon_agent_list_required(&daemon_client)?;

    if rows.is_empty() {
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

    for row in &rows {
        let status = render_daemon_agent_row_status(row);
        let type_styled = match row.runtime.as_str() {
            "claude-code" => style("claude-code").magenta(),
            "codex" => style("codex").yellow(),
            "codex-app-server" => style("codex-app-server").yellow(),
            other => style(other).yellow(),
        };
        eprintln!(
            "  {:<14} {:<18} {:<12} {:<10} {}",
            style(&row.name).white().bold(),
            type_styled,
            style(row.model.as_deref().unwrap_or("-")).cyan(),
            style(
                row.timeout_secs
                    .map(|secs| format!("{secs}s"))
                    .unwrap_or_else(|| "-".to_string()),
            )
            .dim(),
            status,
        );
    }
    eprintln!();
    Ok(())
}

fn invoke_daemon_agent_list_required(
    client: &LocalDaemonAbilityClient,
) -> anyhow::Result<Vec<DaemonAgentRow>> {
    daemon_agent_view::list_agents_with_client(client)
}

fn render_daemon_agent_row_status(row: &DaemonAgentRow) -> console::StyledObject<&'static str> {
    match row.root_exists {
        Some(true) => style("ok").green(),
        Some(false) => style("path missing").red(),
        None => style("unknown").yellow(),
    }
}

#[derive(Debug, serde::Deserialize)]
struct RemovedAgentPayload {
    #[serde(default)]
    root_path: Option<std::path::PathBuf>,
}

fn daemon_agent_row(
    client: &LocalDaemonAbilityClient,
    name: &str,
) -> anyhow::Result<DaemonAgentRow> {
    invoke_daemon_agent_list_required(client)?
        .into_iter()
        .find(|row| row.name == name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "agent '{name}' is not registered; run 'easynet agent list' to see registered \
                 names, or `easynet agent add {name} --type …` to register it"
            )
        })
}

fn daemon_row_agent_type(row: &DaemonAgentRow) -> anyhow::Result<AgentRuntimeKind> {
    daemon_agent_view::agent_kind(row)
}

fn daemon_row_root(row: &DaemonAgentRow) -> std::path::PathBuf {
    daemon_agent_view::agent_root(row)
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    let daemon_client = required_local_daemon_agent_client()?;
    let daemon_response = invoke_daemon_agent_stop_required(&daemon_client, &args.name)?;
    let ack = daemon_response
        .get("ack")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !ack {
        anyhow::bail!("agent '{}' not found", args.name);
    }
    let removed: RemovedAgentPayload = serde_json::from_value(
        daemon_response
            .get("removed_entry")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(|err| anyhow::anyhow!("device.agent.stop returned invalid removed_entry: {err}"))?;
    output::success(&format!("Removed agent '{}'", args.name));
    render_agent_stop_runtime_outcome(&args.name, &daemon_response);

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
    let daemon_client = required_local_daemon_agent_client()?;

    // Identify rows whose root is missing. We check the
    // explicit `root_path` first, falling back to the
    // consumer-side default so a v2 row whose `root_path` was
    // never populated (still a real scenario today — `run_add`
    // before this PR did not set it) is classified the same as
    // any other.
    let rows = invoke_daemon_agent_list_required(&daemon_client)?;
    let orphans: Vec<DaemonAgentRow> = rows
        .into_iter()
        .filter(|row| row.root_exists == Some(false))
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
        let root = daemon_row_root(name);
        eprintln!("    • {}  (missing root: {})", name.name, root.display());
    }
    eprintln!();

    if args.dry_run {
        return Ok(());
    }

    for row in &orphans {
        let resp = invoke_daemon_agent_stop_required(&daemon_client, &row.name)?;
        if !resp
            .get("ack")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            output::warn(&format!(
                "daemon reported orphaned agent '{}' was already absent",
                row.name
            ));
        }
    }
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
    let daemon_client = required_local_daemon_agent_client()?;
    let row = daemon_agent_row(&daemon_client, &args.name)?;

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

    let name = args.name.clone();
    let runtime = row.runtime.clone();
    let root_path = row
        .root_path
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    let model_for_request = new_model.clone();
    let daemon_response = invoke_daemon_agent_start_required(
        &daemon_client,
        serde_json::json!({
            "name": name,
            "agent_type": runtime,
            "model": model_for_request,
            "model_present": true,
            "root_path": root_path,
            "materialize_directory": true,
            "update_existing_spec": true,
        }),
    )?;

    output::success(&format!("Updated agent '{}'", args.name));
    render_agent_start_runtime_outcome(&args.name, &daemon_response);
    match daemon_response
        .get("model")
        .and_then(serde_json::Value::as_str)
    {
        Some(m) => output::detail("model", m),
        None => output::detail("model", "(cleared — CLI default will be used)"),
    }
    if let Some(root) = daemon_response
        .get("root_path")
        .and_then(serde_json::Value::as_str)
    {
        output::detail("root", root);
    }

    Ok(())
}

/// Resolve the session_id the caller wants to attach this `agent
/// send` to, based on the mutually-exclusive flag set
/// (--follow / --session-id / --resume / none).
///
/// Return shapes:
///   * Ok(Some(id)) — caller pinned a specific session; the chat
///                    handler will resume it.
///   * Ok(None)     — caller wants a fresh session; the chat
///                    handler will mint a new id and we'll save
///                    the first turn under it.
///   * Err(...)     — flag combination invalid OR the requested
///                    session is unreachable (no prior session for
///                    --follow on a fresh agent, --resume on a
///                    non-TTY shell, picker rejected by user).
fn resolve_session_id(args: &SendArgs) -> anyhow::Result<Option<String>> {
    use crate::persistence::chat_sessions;

    // clap's ArgGroup already enforces "at most one of these" but
    // we double-check defensively in case future refactors break
    // the group decl. Cheaper than discovering the silent failure
    // mode in production.
    let n_flags =
        (args.follow as u8) + args.session_id.as_ref().map_or(0, |_| 1) + (args.resume as u8);
    if n_flags > 1 {
        anyhow::bail!(
            "--follow, --session-id, and --resume are mutually exclusive; pass at most one"
        );
    }

    if let Some(explicit) = args.session_id.as_deref() {
        let trimmed = explicit.trim();
        if trimmed.is_empty() {
            anyhow::bail!("--session-id is empty (shell expansion accident?)");
        }
        return Ok(Some(trimmed.to_string()));
    }

    if args.follow {
        match chat_sessions::latest_session(&args.name) {
            Some(sid) => return Ok(Some(sid)),
            None => anyhow::bail!(
                "agent '{}' has no recorded sessions yet — \
                 send a fresh prompt without --follow first",
                args.name
            ),
        }
    }

    if args.resume {
        let sessions = chat_sessions::list_sessions(&args.name);
        if sessions.is_empty() {
            anyhow::bail!(
                "agent '{}' has no recorded sessions yet — \
                 send a fresh prompt without --resume first",
                args.name
            );
        }
        if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
            anyhow::bail!(
                "--resume is interactive; stdin is not a terminal. \
                 Use --session-id <UUID> instead — list candidates with \
                 'easynet agent chat-history {} list'",
                args.name
            );
        }
        return prompt_session_picker(&args.name, &sessions).map(Some);
    }

    Ok(None)
}

/// Arrow-key TUI picker for `--resume`. Backed by `dialoguer`'s
/// `Select` (a thin wrapper over `console`, already a direct dep),
/// so the picker reuses the same terminal backend as the rest of
/// the CLI — no second TUI stack pulled in.
///
/// UX:
///   * ↑/↓ to move, Enter to confirm, Esc / q / Ctrl-C to abort.
///   * Cursor starts on the most-recent session (index 0; the
///     same id `--follow` would resume), so the common case
///     ("just continue the latest") is one Enter away.
///   * Each row renders `<short-id>  N turns  <since>  <preview>`.
///     Short id = first 8 chars of the UUID, enough to disambiguate
///     human-scale session counts without making the row 80 cols
///     wide.
///   * Cap at 50 most-recent sessions on screen — the picker grows
///     unwieldy past that and operators with hundreds of sessions
///     should pin via `--session-id` (which they already have to
///     copy from `agent chat-history list`).
///
/// Stdin is already verified to be a TTY by the caller. Aborts
/// (Esc / q / Ctrl-C / no choice) surface as a typed Err so
/// `agent send` doesn't silently hand back to the user with no
/// message.
fn prompt_session_picker(
    agent: &str,
    sessions: &[crate::persistence::chat_sessions::SessionDescriptor],
) -> anyhow::Result<String> {
    use dialoguer::theme::ColorfulTheme;
    use dialoguer::Select;

    const PICKER_CAP: usize = 50;
    let visible = &sessions[..sessions.len().min(PICKER_CAP)];

    let labels: Vec<String> = visible
        .iter()
        .map(|s| {
            let short_id: String = s.session_id.chars().take(8).collect();
            let preview = if s.prompt_preview.is_empty() {
                String::from("(no prompt yet)")
            } else {
                s.prompt_preview.clone()
            };
            format!(
                "{}  {} turns  {}  {}",
                short_id,
                s.turn_count,
                relative_age(&s.last_turn_at),
                preview,
            )
        })
        .collect();

    let header = if sessions.len() > PICKER_CAP {
        format!(
            "Pick a prior session for {agent} (showing latest {PICKER_CAP} of {}; \
             pin older ones via --session-id <UUID>)",
            sessions.len()
        )
    } else {
        format!(
            "Pick a prior session for {agent} ({} session{})",
            sessions.len(),
            if sessions.len() == 1 { "" } else { "s" }
        )
    };

    let chosen = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(header)
        .items(&labels)
        .default(0)
        .interact_opt()
        .map_err(|e| anyhow::anyhow!("session picker io error: {e}"))?;

    match chosen {
        Some(i) => Ok(visible[i].session_id.clone()),
        None => anyhow::bail!("session picker aborted by user"),
    }
}

/// Format an RFC3339 timestamp as a short relative-age string
/// ("5m ago", "3h ago", "2d ago"). Used by the resume picker so
/// each row stays scannable. Falls back to the raw timestamp if
/// it can't be parsed — bad clock data is a banner-class problem
/// but we don't want it to break `--resume`.
fn relative_age(ts: &str) -> String {
    let parsed = match chrono::DateTime::parse_from_rfc3339(ts) {
        Ok(dt) => dt,
        Err(_) => return ts.to_string(),
    };
    let elapsed = chrono::Utc::now().signed_duration_since(parsed.with_timezone(&chrono::Utc));
    let secs = elapsed.num_seconds().max(0);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

fn run_send(args: SendArgs) -> anyhow::Result<()> {
    // Validate through the daemon's Axon ability surface so the CLI
    // does not own a parallel registry read path.
    let daemon_client = required_local_daemon_agent_client()?;
    let _row = daemon_agent_row(&daemon_client, &args.name)?;

    // `--resume` is picker-only — single job, no prompt allowed.
    // Validate this BEFORE resolving the session id so we don't
    // open the TTY picker just to throw the result away.
    if args.resume && args.prompt.is_some() {
        anyhow::bail!(
            "`--resume` does not take a PROMPT; it only sets the latest \
             session. Run `easynet agent send {} --resume` to pick a \
             session, then `agent send {0} --follow \"<msg>\"` to send.",
            args.name
        );
    }

    // Resolve the session_id BEFORE we kick off mission machinery.
    // The chat ability mints a fresh id when none is supplied; a
    // concrete id triggers the resume path on the daemon side.
    let resolved_session_id = resolve_session_id(&args)?;

    // Two prompt regimes after the early `--resume + prompt`
    // rejection above:
    //
    //   `--resume` (no prompt) → picker → set latest pointer → exit.
    //   anything else          → prompt required (send a new turn).
    let prompt = match args.prompt.as_deref() {
        Some(p) => p.to_string(),
        None => {
            if !args.resume {
                anyhow::bail!(
                    "PROMPT is required (omit only when `--resume` is the \
                     sole session flag, in which case the picker just sets \
                     the latest pointer)."
                );
            }
            let sid = resolved_session_id
                .clone()
                .expect("resume path always returns a session id");
            crate::persistence::chat_sessions::set_latest_session(&args.name, &sid)?;
            eprintln!(
                "  {} {} {}",
                style("[agent-send]").dim(),
                style("set latest session →").dim(),
                style(&sid).cyan(),
            );
            eprintln!(
                "  {}",
                style(format!(
                    "next `easynet agent send {} --follow \"...\"` will land on this session.",
                    args.name
                ))
                .dim(),
            );
            return Ok(());
        }
    };

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
    if let Some(sid) = resolved_session_id.as_deref() {
        eprintln!(
            "  {} {} {}",
            style("[agent-send]").dim(),
            style("resume session").dim(),
            style(sid).cyan(),
        );
    }

    // Compose the prompt: fold optional `--context` into the prompt body
    // BEFORE constructing the EAL source, so the prompt that ends up in
    // the EAL string literal is exactly the prompt the agent will see.
    let composed_prompt = match args.context.as_deref() {
        Some(ctx) if !ctx.trim().is_empty() => {
            format!("{prompt}\n\n## Context (previous discussion)\n\n{ctx}\n")
        }
        _ => prompt.clone(),
    };

    // Build the single-line EAL mission source. The mission name is
    // `agent-send`; the binding is `__reply` so the result can be
    // pulled out of `MissionRunResult.bound_vars`. `eal_string_literal`
    // can fail if the user's prompt contains an embedded NUL byte — we
    // surface that as a CLI error rather than silently truncating.
    let eal_source = match resolved_session_id.as_deref() {
        Some(sid) => format!(
            "mission \"agent-send\" {{\n    let __reply = {agent}.chat(prompt: {prompt}, session_id: {sid})\n}}\n",
            agent = args.name,
            prompt = eal_string_literal(&composed_prompt)?,
            sid = eal_string_literal(sid)?,
        ),
        None => format!(
            "mission \"agent-send\" {{\n    let __reply = {agent}.chat(prompt: {prompt})\n}}\n",
            agent = args.name,
            prompt = eal_string_literal(&composed_prompt)?,
        ),
    };

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
    // mission ability returns a JSON object. Two shapes can appear:
    //   * `<agent>.chat` (the invoke_direct_with_progress path):
    //     `{session_id, reply, tool_calls, usage, skills_loaded, ...}`
    //   * non-chat verbs (the send_to_agent shell-out path):
    //     `{ok, agent, output, model, duration_ms}`
    // The user-visible reply lives in `reply` for chat and `output`
    // for shell-out — try both.
    let reply_obj = match result.bound_vars.get("__reply") {
        Some(serde_json::Value::Object(obj)) => Some(obj.clone()),
        _ => None,
    };
    let reply_text: String = match &reply_obj {
        Some(obj) => obj
            .get("reply")
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("output").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
            .unwrap_or_else(|| serde_json::Value::Object(obj.clone()).to_string()),
        None => match result.bound_vars.get("__reply") {
            Some(other) => other.to_string(),
            None => String::new(),
        },
    };
    // Server-minted session id (when caller passed none) or echoed-back
    // (when the caller pinned one via --follow / --session-id). Used by
    // the persistence write below AND echoed to the user so they can
    // copy it for a later --session-id call.
    let response_session_id: Option<String> = reply_obj
        .as_ref()
        .and_then(|obj| obj.get("session_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let tool_calls: Vec<Value> = reply_obj
        .as_ref()
        .and_then(|obj| obj.get("tool_calls"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let usage_value: Value = reply_obj
        .as_ref()
        .and_then(|obj| obj.get("usage"))
        .cloned()
        .unwrap_or(Value::Null);

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
    if let Some(sid) = response_session_id.as_deref() {
        eprintln!(
            "  {} {}  {}",
            style("session").dim(),
            style(sid).cyan(),
            style("(use --follow to continue, --session-id <UUID> to pin)").dim(),
        );
    }
    eprintln!();

    // Persist the turn to the agent's per-session JSONL log so
    // future `--follow` / `--resume` / `agent sessions show` calls
    // can find it. Best-effort by contract — a disk-full or
    // permission failure must NOT abort the in-flight chat reply.
    if let Some(sid) = response_session_id.as_deref() {
        crate::persistence::chat_sessions::write_turn_best_effort(
            &args.name,
            sid,
            &prompt,
            &reply_text,
            &tool_calls,
            &usage_value,
        );
    }

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
    let daemon_client = required_local_daemon_agent_client()?;
    let rows = invoke_daemon_agent_list_required(&daemon_client)?;

    let agents_to_check: Vec<(String, AgentRuntimeKind)> = match args.name {
        Some(name) => {
            let row = rows
                .iter()
                .find(|row| row.name == name)
                .ok_or_else(|| anyhow::anyhow!("agent '{}' not found", name))?;
            vec![(name, daemon_row_agent_type(row)?)]
        }
        None => {
            if rows.is_empty() {
                // Check both CLIs even if no agents registered.
                vec![
                    ("claude-code".to_string(), AgentRuntimeKind::ClaudeCode),
                    ("codex".to_string(), AgentRuntimeKind::Codex),
                ]
            } else {
                rows.iter()
                    .map(|row| Ok((row.name.clone(), daemon_row_agent_type(row)?)))
                    .collect::<anyhow::Result<Vec<_>>>()?
            }
        }
    };

    let mut all_ok = true;
    eprintln!();

    for (name, agent_type) in &agents_to_check {
        let result = if agent_type.is_claude_code() {
            claude_code::doctor()
        } else {
            codex::doctor()
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
    let daemon_client = required_local_daemon_agent_client()?;
    let row = daemon_agent_row(&daemon_client, name)?;
    let root = daemon_row_root(&row);
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
            style("Drop a '<verb>.ability.toml' into that directory to declare one.").dim(),
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

fn run_mcp(args: McpArgs) -> anyhow::Result<()> {
    match args.action {
        McpAction::Add(a) => run_mcp_add(a),
    }
}

/// Top-level CLI entry. Each phase is delegated to a small,
/// independently testable helper so this function reads as the
/// product flow:
///
///   1. resolve the target agent + MCP config
///   2. plan the manifests that should exist (no filesystem writes)
///   3. validate the user's `--tool` selection against the plan
///   4. materialise / dry-run the plans + render the operator summary
fn run_mcp_add(args: McpAddArgs) -> anyhow::Result<()> {
    let dir = open_registered_agent(&args.name)?;
    let config_path = args.config.clone().unwrap_or_else(
        crate::runtime::execution::mcp_client::McpClientService::default_config_path,
    );
    let svc = crate::runtime::execution::mcp_client::McpClientService::from_path(&config_path)?;

    let declared_cost = build_cost_meta(args.cost_kind, args.cost_label.as_deref())?;
    let plan = plan_mcp_additions(
        &svc,
        &config_path,
        args.server.as_deref(),
        &args.tools,
        &args.prefix,
        args.skip_unreachable,
        declared_cost.as_ref(),
    )?;
    assert_tools_filter_satisfied(&args.tools, &plan.planned)?;

    if plan.planned.is_empty() {
        report_empty_plan(&plan.list_failures);
        return Ok(());
    }

    let outcome = write_mcp_additions(&dir, &plan.planned, args.overwrite, args.dry_run)?;
    report_write_outcome(&args.name, &dir, &plan, &outcome, args.dry_run);
    Ok(())
}

/// Result of phase (2): the manifests we'd write and the per-upstream
/// `tools/list` failures we tolerated via `--skip-unreachable`.
#[derive(Debug, Default)]
struct McpAdditionPlan {
    planned: Vec<McpAbilityPlan>,
    list_failures: Vec<String>,
}

/// Result of phase (4): per-plan disposition. `written` counts new
/// files created; `skipped` counts plans whose target already held
/// the same `(server, tool)` binding (idempotent re-runs).
#[derive(Debug, Default)]
struct McpAdditionOutcome {
    written: usize,
    skipped: usize,
}

/// Build the manifest plan for one `easynet agent mcp add` invocation.
///
/// Pure-ish: the only side effect is talking to `svc` (which itself
/// reads the operator's `mcp_clients.json` config). No filesystem
/// writes happen here — that's phase (4).
/// Build the `CostMeta` value the manifest writer will stamp on every
/// generated ability, or `None` when the operator did not pass
/// `--cost-kind`. Folds the two flags into one structure here so the
/// downstream pipeline only has to consider "declared / not declared",
/// not the cartesian product.
fn build_cost_meta(
    cost_kind: Option<CostKindArg>,
    cost_label: Option<&str>,
) -> anyhow::Result<Option<crate::core::ability_spec::CostMeta>> {
    use crate::core::ability_spec::CostMeta;
    let Some(kind) = cost_kind else {
        return Ok(None);
    };
    // Trimmed-empty labels are forbidden by `CostMeta::validate`, but
    // the CLI surface lets a user pass `--cost-label ""` (or just
    // whitespace) — translate that into "omitted" so it round-trips
    // through validation rather than failing at write time.
    let label = cost_label
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string);
    let meta = CostMeta {
        kind: kind.into_core(),
        label,
    };
    Ok(Some(meta))
}

fn plan_mcp_additions(
    svc: &crate::runtime::execution::mcp_client::McpClientService,
    config_path: &std::path::Path,
    server_filter: Option<&str>,
    tool_filter: &[String],
    prefix: &str,
    skip_unreachable: bool,
    declared_cost: Option<&crate::core::ability_spec::CostMeta>,
) -> anyhow::Result<McpAdditionPlan> {
    let selected_servers = select_mcp_servers(svc, server_filter)?;
    if selected_servers.is_empty() {
        anyhow::bail!(
            "no MCP servers configured in {}; populate the file with at least one server entry first",
            config_path.display()
        );
    }

    let mut plan = McpAdditionPlan::default();
    for server in selected_servers {
        let listing = match mcp_rpc_blocking_timeout(
            svc,
            &server,
            "tools/list",
            serde_json::json!({}),
            mcp_tools_list_timeout(),
        ) {
            Ok(v) => v,
            Err(e) if skip_unreachable => {
                plan.list_failures.push(format!("{server}: {e}"));
                continue;
            }
            Err(e) => anyhow::bail!("{server}: tools/list failed: {e}"),
        };
        let tools = listing
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                anyhow::anyhow!("{server}: tools/list response missing `tools` array")
            })?;
        for tool in tools {
            let Some(upstream_tool) = tool.get("name").and_then(Value::as_str) else {
                continue;
            };
            if !tool_filter.is_empty() && !tool_filter.iter().any(|t| t == upstream_tool) {
                continue;
            }
            let input_schema = normalize_mcp_input_schema(tool.get("inputSchema").cloned());
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("Call MCP tool `{upstream_tool}` on `{server}`."));
            let verb = generated_mcp_ability_name(prefix, &server, upstream_tool);
            plan.planned.push(McpAbilityPlan {
                server: server.clone(),
                tool: upstream_tool.to_string(),
                verb,
                description,
                input_schema,
                cost: declared_cost.cloned(),
            });
        }
    }
    Ok(plan)
}

/// Phase (3): every `--tool` the operator named must resolve into
/// the plan. Missing tools indicate a typo or a misaligned upstream
/// catalogue; failing loud here beats silently materialising fewer
/// abilities than the operator asked for.
fn assert_tools_filter_satisfied(
    tool_filter: &[String],
    planned: &[McpAbilityPlan],
) -> anyhow::Result<()> {
    if tool_filter.is_empty() {
        return Ok(());
    }
    let found: std::collections::BTreeSet<&str> = planned.iter().map(|p| p.tool.as_str()).collect();
    let missing: Vec<&str> = tool_filter
        .iter()
        .map(String::as_str)
        .filter(|tool| !found.contains(tool))
        .collect();
    if !missing.is_empty() {
        anyhow::bail!(
            "requested MCP tool(s) not found in selected server set: {}",
            missing.join(", ")
        );
    }
    Ok(())
}

/// Phase (4): turn each plan into a manifest TOML and either print
/// it (dry-run) or atomically write it to the agent's abilities
/// directory. Returns the materialisation outcome so the caller can
/// render the operator summary.
fn write_mcp_additions(
    dir: &AgentDirectory,
    planned: &[McpAbilityPlan],
    overwrite: bool,
    dry_run: bool,
) -> anyhow::Result<McpAdditionOutcome> {
    if !dry_run {
        std::fs::create_dir_all(dir.abilities_dir()).map_err(|e| {
            anyhow::anyhow!(
                "create abilities directory {}: {e}",
                dir.abilities_dir().display()
            )
        })?;
    }

    let mut outcome = McpAdditionOutcome::default();
    for plan in planned {
        let manifest = mcp_manifest_for(plan)?;
        let body = manifest.to_toml_string()?;
        let path = dir
            .abilities_dir()
            .join(format!("{}.ability.toml", manifest.name()));

        if path.exists() && !overwrite {
            let existing = std::fs::read_to_string(&path).ok();
            if existing.as_deref().and_then(existing_mcp_binding).as_ref()
                == Some(&(plan.server.clone(), plan.tool.clone()))
            {
                outcome.skipped += 1;
                continue;
            }
            anyhow::bail!(
                "refusing to overwrite existing ability manifest {}; pass --overwrite to replace it",
                path.display()
            );
        }

        if dry_run {
            println!("--- {}", path.display());
            print!("{body}");
        } else {
            config::atomic_write(&path, body.as_bytes())
                .map_err(|e| anyhow::anyhow!("write {}: {e}", path.display()))?;
            outcome.written += 1;
        }
    }
    Ok(outcome)
}

/// Operator summary when no plans were produced (either the filter
/// matched nothing or every upstream was unreachable under
/// `--skip-unreachable`).
fn report_empty_plan(list_failures: &[String]) {
    if list_failures.is_empty() {
        output::info("No MCP tools matched the requested selection.");
    } else {
        output::warn("No MCP tools were bound; every selected upstream failed tools/list.");
        for failure in list_failures {
            output::warn(failure);
        }
    }
}

/// Operator summary for a non-empty plan; mirrors the shape of the
/// other `easynet agent …` subcommands (success line + key/value
/// detail lines + trailing warnings for partial failures).
fn report_write_outcome(
    agent_name: &str,
    dir: &AgentDirectory,
    plan: &McpAdditionPlan,
    outcome: &McpAdditionOutcome,
    dry_run: bool,
) {
    if dry_run {
        output::success(&format!(
            "dry-run: {} MCP ability manifest(s) would be written for agent '{}'",
            plan.planned.len(),
            agent_name
        ));
    } else {
        output::success(&format!(
            "added {} MCP ability manifest(s) to agent '{}'",
            outcome.written, agent_name
        ));
        if outcome.skipped > 0 {
            output::detail(
                "skipped",
                &format!("{} existing identical binding(s)", outcome.skipped),
            );
        }
        output::detail("root", &dir.abilities_dir().display().to_string());
        output::info(
            "A running daemon can invoke these through the dynamic agent fallback immediately; restart or refresh catalogue surfaces if a UI needs to list them.",
        );
    }
    for failure in &plan.list_failures {
        output::warn(failure);
    }
}

#[derive(Debug, Clone)]
struct McpAbilityPlan {
    server: String,
    tool: String,
    verb: String,
    description: String,
    input_schema: Value,
    /// Operator-declared cost meta, forwarded verbatim from
    /// `--cost-kind`/`--cost-label`. `None` writes a manifest with no
    /// `[cost]` table; the runtime falls back to the per-exec
    /// inference at metadata-emit time.
    cost: Option<crate::core::ability_spec::CostMeta>,
}

fn select_mcp_servers(
    svc: &crate::runtime::execution::mcp_client::McpClientService,
    server: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    mcp_block_on(async {
        let names = svc.server_names().await;
        match server {
            Some(wanted) => {
                if names.iter().any(|n| n == wanted) {
                    Ok(vec![wanted.to_string()])
                } else {
                    anyhow::bail!(
                        "MCP server {wanted:?} not found in configured servers: {}",
                        names.join(", ")
                    )
                }
            }
            None => Ok(names),
        }
    })
}

fn mcp_rpc_blocking_timeout(
    svc: &crate::runtime::execution::mcp_client::McpClientService,
    server: &str,
    method: &str,
    params: Value,
    timeout: Duration,
) -> anyhow::Result<Value> {
    mcp_block_on(async move {
        match tokio::time::timeout(timeout, svc.rpc(server, method, params)).await {
            Ok(result) => result,
            Err(_) => anyhow::bail!("{method} timed out after {}s", timeout.as_secs()),
        }
    })
}

fn mcp_tools_list_timeout() -> Duration {
    let secs = std::env::var("EASYNET_MCP_TOOLS_LIST_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(20);
    Duration::from_secs(secs)
}

fn mcp_block_on<F, T>(fut: F) -> anyhow::Result<T>
where
    F: std::future::Future<Output = anyhow::Result<T>>,
{
    match tokio::runtime::Handle::try_current() {
        Ok(_handle) => Ok(tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(fut)
        })?),
        Err(_) => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| anyhow::anyhow!("build mcp cli runtime: {e}"))?;
            rt.block_on(fut)
        }
    }
}

fn normalize_mcp_input_schema(schema: Option<Value>) -> Value {
    match schema {
        Some(v @ Value::Object(_)) => toml_safe_json_value(v),
        Some(v) => serde_json::json!({
            "type": "object",
            "additionalProperties": true,
            "x-easynet-originalInputSchema": toml_safe_json_value(v),
        }),
        None => serde_json::json!({
            "type": "object",
            "additionalProperties": true,
        }),
    }
}

fn toml_safe_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let safe = map
                .into_iter()
                .filter_map(|(k, v)| {
                    if v.is_null() {
                        None
                    } else {
                        Some((k, toml_safe_json_value(v)))
                    }
                })
                .collect();
            Value::Object(safe)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|v| {
                    if v.is_null() {
                        Value::String("null".into())
                    } else {
                        toml_safe_json_value(v)
                    }
                })
                .collect(),
        ),
        other => other,
    }
}

fn mcp_manifest_for(
    plan: &McpAbilityPlan,
) -> anyhow::Result<crate::core::ability_spec::AbilityManifest> {
    use crate::core::ability_spec::{AbilityExec, AbilityManifest, McpExec};
    let mut manifest = AbilityManifest::new(
        plan.verb.clone(),
        plan.description.clone(),
        plan.input_schema.clone(),
    )?
    .with_exec(AbilityExec::Mcp(McpExec {
        server: plan.server.clone(),
        tool: plan.tool.clone(),
    }))?
    .with_output_schema(serde_json::json!({
        "type": "object",
        "properties": {
            "content": {"type": "array"},
            "isError": {"type": "boolean"}
        },
        "required": ["content"],
        "additionalProperties": true
    }))?;
    if let Some(cost) = &plan.cost {
        manifest = manifest.with_cost(cost.clone())?;
    }
    Ok(manifest)
}

fn existing_mcp_binding(body: &str) -> Option<(String, String)> {
    use crate::core::ability_spec::AbilityExec;
    let manifest = crate::core::ability_spec::AbilityManifest::from_toml_str(body).ok()?;
    match manifest.exec()? {
        AbilityExec::Mcp(exec) => Some((exec.server.clone(), exec.tool.clone())),
        _ => None,
    }
}

fn generated_mcp_ability_name(prefix: &str, server: &str, tool: &str) -> String {
    let prefix_slug = slug_segment(prefix);
    let server_slug = slug_segment(server);
    let tool_slug = slug_segment(tool);
    // Flat single-underscore form: `{prefix}_{server}_{tool}`. The
    // earlier double-underscore at the server↔tool seam advertised
    // the boundary visually but cost readability across the whole
    // catalogue; user calls this trade-off in favour of a uniform
    // separator. Two distinct server↔tool pairs that slugify to the
    // same flat string would collide; the hash fallback below
    // covers that case for empty / separator-only slugs.
    let base = if prefix_slug.is_empty() {
        format!("{server_slug}_{tool_slug}")
    } else {
        format!("{prefix_slug}_{server_slug}_{tool_slug}")
    };
    // "Empty after slugify" means either the formatted string is
    // literally empty OR it slugifies to nothing but separators
    // (e.g. `"__"` from server=tool="…"). The hash fallback
    // guarantees a deterministic, distinct ability name in both
    // cases. We hash the RAW upstream identifiers (not the slugs)
    // so that two upstream pairs that slugify to the same empty
    // shape still receive distinct hashes — without this the test
    // pair `("...", "///")` vs `("***", "===")` would collide on
    // the empty-slug `":"` hash input.
    let is_only_separators = !base.is_empty() && base.chars().all(|c| c == '_' || c == '-');
    if base.is_empty() || is_only_separators {
        format!("mcp_{}", short_hex(format!("{server}:{tool}").as_bytes()))
    } else {
        base
    }
}

fn slug_segment(raw: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in raw.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch == '_' || ch == '-' {
            Some(ch)
        } else {
            Some('_')
        };
        if let Some(c) = mapped {
            if c == '_' || c == '-' {
                if !last_was_sep && !out.is_empty() {
                    out.push('_');
                    last_was_sep = true;
                }
            } else {
                out.push(c);
                last_was_sep = false;
            }
        }
    }
    while out.ends_with('_') || out.ends_with('-') {
        out.pop();
    }
    out
}

fn short_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest[..4].iter().map(|b| format!("{b:02x}")).collect()
}

fn run_publish(args: PublishArgs) -> anyhow::Result<()> {
    if !args.dry_run {
        // Live publishing is gated until the cross-repo publish
        // spec + implementation lands. Returning a clear error
        // here — rather than silently calling through to a
        // future Axon path — keeps the flag's contract honest:
        // today every successful `agent publish` is a dry-run.
        anyhow::bail!(
            "only '--dry-run' is supported in this release. Live publishing through \
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

/// `easynet agent refresh` — ask the daemon to re-register every
/// agent ability the workspace currently declares into LocalRuntime.
///
/// Use this after authoring a new `<agent>/abilities/<verb>.ability.toml`
/// (or after running `easynet agent add <name>` while the daemon is
/// alive) to make the new ability invokable without restarting the
/// daemon.
///
/// The CLI deliberately does not connect to daemon storage, Axon runtime,
/// or hub transport here. Runtime sync is daemon-owned and exposed as
/// `device.agent.refresh`.
fn run_refresh(args: RefreshArgs) -> anyhow::Result<()> {
    let daemon_client = required_local_daemon_agent_client()?;
    let payload = match args
        .agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(name) => serde_json::json!({ "name": name }),
        None => serde_json::json!({}),
    };
    let response = invoke_daemon_agent_refresh_required(&daemon_client, payload)?;
    let scanned = response
        .get("agents_scanned")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let registered = response
        .get("runtime_registered")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let failed = response
        .get("runtime_failed")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if let Some(rows) = response.get("agents").and_then(serde_json::Value::as_array) {
        for row in rows {
            let name = row
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("<unknown>");
            let row_registered = row
                .get("runtime_registered")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let row_failed = row
                .get("runtime_failed")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            output::detail(
                "refreshed",
                &format!("{name}: registered {row_registered}, failed {row_failed}"),
            );
        }
    }
    output::success(&format!(
        "daemon refreshed {scanned} agent(s): registered {registered}, failed {failed}"
    ));
    Ok(())
}

// ── Sessions inspection ────────────────────────────────────────────

fn run_sessions(args: ChatHistoryArgs) -> anyhow::Result<()> {
    // Validate the agent exists. Lets us emit "no such agent"
    // rather than "no sessions" for a typo'd name.
    let daemon_client = required_local_daemon_agent_client()?;
    let _row = daemon_agent_row(&daemon_client, &args.name)?;
    match args.action {
        ChatHistoryAction::List(a) => run_sessions_list(&args.name, a),
        ChatHistoryAction::Show(a) => run_sessions_show(&args.name, a),
    }
}

fn run_sessions_list(agent: &str, args: ChatHistoryListArgs) -> anyhow::Result<()> {
    use crate::persistence::chat_sessions;
    let sessions = chat_sessions::list_sessions(agent);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }
    if sessions.is_empty() {
        println!(
            "No sessions recorded for agent '{agent}'. \
             Run 'easynet agent send {agent} \"...\"' to start one."
        );
        return Ok(());
    }
    let latest = chat_sessions::latest_session(agent).unwrap_or_default();
    println!(
        "{:<38} {:<22} {:>6}  {}",
        "SESSION_ID", "LAST_TURN_AT", "TURNS", "PROMPT"
    );
    for s in &sessions {
        let marker = if s.session_id == latest { "*" } else { " " };
        println!(
            "{}{:<37} {:<22} {:>6}  {}",
            marker, s.session_id, s.last_turn_at, s.turn_count, s.prompt_preview,
        );
    }
    println!();
    println!("  '*' marks the most-recent session ('agent send {agent} --follow' resumes it).");
    Ok(())
}

fn run_sessions_show(agent: &str, args: ChatHistoryShowArgs) -> anyhow::Result<()> {
    use crate::persistence::chat_sessions;
    let lines = chat_sessions::load_session(agent, &args.session_id)?;
    if args.json {
        for v in &lines {
            println!("{}", serde_json::to_string(v)?);
        }
        return Ok(());
    }
    // Human-readable transcript: meta header, then one block per turn.
    if lines.is_empty() {
        println!("Session '{}' is empty.", args.session_id);
        return Ok(());
    }
    let mut turn_no = 0usize;
    for v in &lines {
        match v.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                println!(
                    "Session: {}",
                    v.get("session_id").and_then(Value::as_str).unwrap_or("?")
                );
                println!(
                    "  agent:       {}",
                    v.get("agent").and_then(Value::as_str).unwrap_or("?")
                );
                println!(
                    "  started_at:  {}",
                    v.get("timestamp").and_then(Value::as_str).unwrap_or("?")
                );
                println!(
                    "  cwd:         {}",
                    v.get("cwd").and_then(Value::as_str).unwrap_or("?")
                );
                println!(
                    "  cli_version: {}",
                    v.get("cli_version").and_then(Value::as_str).unwrap_or("?")
                );
                println!();
            }
            Some("turn") => {
                turn_no += 1;
                println!(
                    "── Turn {turn_no} ── {}",
                    v.get("timestamp").and_then(Value::as_str).unwrap_or("?")
                );
                if let Some(p) = v.get("prompt").and_then(Value::as_str) {
                    println!("  user:");
                    for line in p.lines() {
                        println!("    {line}");
                    }
                }
                if let Some(r) = v.get("reply").and_then(Value::as_str) {
                    println!("  agent:");
                    for line in r.lines() {
                        println!("    {line}");
                    }
                }
                if let Some(usage) = v.get("usage") {
                    if !usage.is_null() {
                        println!("  usage: {}", usage);
                    }
                }
                println!();
            }
            _ => {
                // Unknown line type — surface verbatim so future
                // log additions don't disappear from `show`.
                println!("(unknown line type) {v}");
            }
        }
    }
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

    use crate::core::agent_spec::{AgentSpec, RuntimeKind};
    use crate::facade::cli::test_support::HomeGuard;
    use crate::registry::agents::{self, CURRENT_REGISTRY_SCHEMA};
    use crate::runtime::directory::Location;
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

    #[cfg(unix)]
    fn write_cli_mcp_echo_server(dir: &std::path::Path) -> std::path::PathBuf {
        let script = dir.join("echo_mcp.sh");
        fs::write(
            &script,
            r#"#!/bin/sh
exec python3 -u -c '
import sys, json
def read_msg():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.decode().strip()
        if not line:
            break
        name, value = line.split(":", 1)
        headers[name.lower()] = value.strip()
    body = sys.stdin.buffer.read(int(headers["content-length"]))
    return json.loads(body)
def write_msg(resp):
    body = json.dumps(resp).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()
while True:
    req = read_msg()
    if req is None:
        break
    rid = req.get("id")
    method = req.get("method")
    if method == "initialize":
        result = {"protocolVersion": "2024-11-05", "capabilities": {}, "serverInfo": {"name": "echo", "version": "0"}}
    elif method == "tools/list":
        result = {"tools": [
            {"name": "echo-text", "description": "Echo text through MCP", "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}}}
        ]}
    else:
        result = {"content": [{"type": "text", "text": "ok"}], "isError": False}
    write_msg({"jsonrpc": "2.0", "id": rid, "result": result})
'
"#,
        )
        .expect("write echo mcp");
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
        script
    }

    #[cfg(unix)]
    #[test]
    fn run_mcp_add_writes_mcp_exec_manifest_for_agent() {
        let _home = HomeGuard::new();
        run_add(add_args("codex", "codex", None)).expect("agent add");
        let tmp = tempfile::tempdir().expect("tempdir");
        let server = write_cli_mcp_echo_server(tmp.path());
        let mcp_dir = crate::persistence::config::state_dir();
        fs::create_dir_all(&mcp_dir).expect("state dir");
        let config_path = mcp_dir.join("mcp_clients.json");
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "servers": [{
                    "name": "Echo Server",
                    "command": server.display().to_string(),
                    "args": [],
                    "stdio_framing": "content-length"
                }]
            }))
            .unwrap(),
        )
        .expect("write mcp config");

        run_mcp_add(McpAddArgs {
            name: "codex".into(),
            server: Some("Echo Server".into()),
            tools: vec![],
            prefix: "mcp".into(),
            config: Some(config_path),
            dry_run: false,
            overwrite: false,
            skip_unreachable: false,
            cost_kind: None,
            cost_label: None,
        })
        .expect("mcp add");

        let manifest_path = crate::persistence::config::agents_root()
            .join("codex")
            .join("abilities")
            .join("mcp_echo_server_echo_text.ability.toml");
        let body = fs::read_to_string(&manifest_path).expect("manifest written");
        let manifest =
            crate::core::ability_spec::AbilityManifest::from_toml_str(&body).expect("parse");
        assert_eq!(manifest.name(), "mcp_echo_server_echo_text");
        match manifest.exec().expect("exec") {
            crate::core::ability_spec::AbilityExec::Mcp(exec) => {
                assert_eq!(exec.server, "Echo Server");
                assert_eq!(exec.tool, "echo-text");
            }
            other => panic!("expected mcp exec, got {other:?}"),
        }
        assert_eq!(
            manifest.input_schema()["properties"]["text"]["type"],
            serde_json::Value::String("string".into())
        );
    }

    #[test]
    fn normalize_mcp_input_schema_removes_toml_unsupported_nulls() {
        let normalized = normalize_mcp_input_schema(Some(serde_json::json!({
            "type": "object",
            "properties": {
                "q": {"type": ["string", null], "default": null}
            }
        })));
        let manifest = crate::core::ability_spec::AbilityManifest::new(
            "mcp_null_schema",
            "schema with upstream nulls",
            normalized,
        )
        .expect("schema should validate");
        manifest
            .to_toml_string()
            .expect("normalized schema should serialize to TOML");
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
    fn run_set_preserves_project_local_root_path() {
        // `agent set` is now a daemon ability invoke. The CLI must
        // still preserve an existing registry row's custom root_path;
        // otherwise project-local agents get silently rewritten into
        // the global agents root during a model update.
        let _g = HomeGuard::new();
        run_add(add_args("alice", "claude-code", Some("sonnet"))).unwrap();

        let custom_root = crate::persistence::config::home_dir()
            .join("project")
            .join("agents")
            .join("alice");
        let mut spec = AgentSpec::new("alice", RuntimeKind::ClaudeCode);
        spec.model = Some("sonnet".to_string());
        AgentDirectory::create(
            &Location::Local {
                root: custom_root.clone(),
            },
            spec,
        )
        .unwrap();

        let mut registry = agents::load_agents().unwrap();
        registry.agents.get_mut("alice").unwrap().root_path = Some(custom_root.clone());
        agents::save_agents(&registry).unwrap();

        run_set(set_args("alice", Some("opus"))).unwrap();

        let entry = agents::load_agents().unwrap().agents["alice"].clone();
        assert_eq!(entry.root_path.as_deref(), Some(custom_root.as_path()));
        let spec =
            AgentSpec::from_toml_str(&fs::read_to_string(custom_root.join("agent.toml")).unwrap())
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
        .expect_err("orphan row must error on 'agent abilities'");
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

    // ── mcp add helpers ────────────────────────────────────────────────

    #[test]
    fn generated_mcp_ability_name_is_slug_safe_and_deterministic() {
        // Prefix + server + tool slugify independently and join with
        // the dotted-verb convention (single `_` between prefix and
        // server, double `__` between server and tool — operators
        // grep the double underscore to identify the tool half).
        // Note: `slug_segment` collapses `-` to `_` along with other
        // non-alnum punctuation, so `geocode-address` lands as
        // `geocode_address` rather than retaining the hyphen.
        let name = generated_mcp_ability_name("mcp", "Google Maps", "geocode-address");
        assert_eq!(name, "mcp_google_maps_geocode_address");
    }

    #[test]
    fn generated_mcp_ability_name_collapses_runs_of_punctuation() {
        // Internal slug runs collapse to a single separator so the
        // emitted ability name remains a legal verb (no `__` runs
        // sneaking in from messy upstream names).
        let name = generated_mcp_ability_name("MCP", "google//maps", "geo  code");
        assert_eq!(name, "mcp_google_maps_geo_code");
    }

    #[test]
    fn generated_mcp_ability_name_falls_back_to_hash_when_slug_empty() {
        // Upstream pair that slugifies to nothing (e.g. all
        // non-alphanumeric) must still produce a stable, unique
        // ability name so collisions surface as different bindings.
        let a = generated_mcp_ability_name("", "...", "///");
        let b = generated_mcp_ability_name("", "***", "===");
        assert!(a.starts_with("mcp_"), "fallback prefix: {a}");
        assert!(b.starts_with("mcp_"), "fallback prefix: {b}");
        assert_ne!(a, b, "distinct upstream pairs must hash to distinct names");
        // Determinism: same input → same output.
        let a2 = generated_mcp_ability_name("", "...", "///");
        assert_eq!(a, a2);
    }

    #[test]
    fn generated_mcp_ability_name_empty_prefix_drops_leading_separator() {
        // `--prefix=""` should produce `<server>_<tool>` without a
        // leading underscore — operators use the empty prefix when
        // they manage their own naming scheme.
        let name = generated_mcp_ability_name("", "echo", "ping");
        assert_eq!(name, "echo_ping");
    }

    #[test]
    fn existing_mcp_binding_extracts_server_and_tool() {
        // A round-tripped manifest must let an idempotent re-run
        // recognise the prior binding so we skip rewriting it.
        let plan = McpAbilityPlan {
            server: "echo".into(),
            tool: "ping".into(),
            verb: "mcp_echo__ping".into(),
            description: "Echo ping.".into(),
            input_schema: serde_json::json!({"type": "object"}),
            cost: None,
        };
        let body = mcp_manifest_for(&plan).unwrap().to_toml_string().unwrap();
        let binding = existing_mcp_binding(&body).expect("manifest declares an mcp binding");
        assert_eq!(binding, ("echo".to_string(), "ping".to_string()));
    }

    #[test]
    fn existing_mcp_binding_returns_none_for_non_mcp_exec() {
        // A manifest without an `mcp` exec block is the operator's
        // own file — must NOT be treated as "matching binding" and
        // overwritten by the idempotent skip path.
        let manifest_toml = r#"
schema_version = "1"
name = "ping"
description = "Operator-authored manifest."

[input_schema]
type = "object"
"#;
        assert_eq!(existing_mcp_binding(manifest_toml), None);
    }

    #[test]
    fn existing_mcp_binding_returns_none_for_malformed_toml() {
        // Don't panic on garbage on disk; the caller will then fall
        // through to the "refuse to overwrite without --overwrite"
        // branch, which is the safer disposition.
        assert_eq!(existing_mcp_binding("this is not valid toml @@@"), None);
    }

    #[test]
    fn mcp_manifest_for_emits_mcp_exec_with_pinned_server_tool() {
        use crate::core::ability_spec::{AbilityExec, AbilityManifest};
        let plan = McpAbilityPlan {
            server: "echo".into(),
            tool: "ping".into(),
            verb: "mcp_echo__ping".into(),
            description: "Echo ping.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"text": {"type": "string"}}
            }),
            cost: None,
        };
        let manifest = mcp_manifest_for(&plan).unwrap();
        assert_eq!(manifest.name(), "mcp_echo__ping");
        assert_eq!(manifest.description(), "Echo ping.");
        match manifest.exec() {
            Some(AbilityExec::Mcp(exec)) => {
                assert_eq!(exec.server, "echo");
                assert_eq!(exec.tool, "ping");
            }
            other => panic!("expected Mcp exec, got {other:?}"),
        }
        // Round-trip through TOML must preserve the binding so
        // existing_mcp_binding can read it back.
        let body = manifest.to_toml_string().unwrap();
        let reparsed = AbilityManifest::from_toml_str(&body).unwrap();
        assert_eq!(reparsed.name(), "mcp_echo__ping");
    }

    #[test]
    fn assert_tools_filter_satisfied_passes_when_every_request_resolved() {
        let planned = vec![
            McpAbilityPlan {
                server: "s".into(),
                tool: "a".into(),
                verb: "_".into(),
                description: String::new(),
                input_schema: serde_json::json!({}),
                cost: None,
            },
            McpAbilityPlan {
                server: "s".into(),
                tool: "b".into(),
                verb: "_".into(),
                description: String::new(),
                input_schema: serde_json::json!({}),
                cost: None,
            },
        ];
        assert_tools_filter_satisfied(&["a".into(), "b".into()], &planned).unwrap();
    }

    #[test]
    fn assert_tools_filter_satisfied_empty_filter_is_unconditional_ok() {
        assert_tools_filter_satisfied(&[], &[]).unwrap();
    }

    #[test]
    fn assert_tools_filter_satisfied_lists_every_missing_tool() {
        let planned = vec![McpAbilityPlan {
            server: "s".into(),
            tool: "a".into(),
            verb: "_".into(),
            description: String::new(),
            input_schema: serde_json::json!({}),
            cost: None,
        }];
        let err = assert_tools_filter_satisfied(
            &["a".into(), "missing-1".into(), "missing-2".into()],
            &planned,
        )
        .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("missing-1"),
            "msg should name missing-1: {msg}"
        );
        assert!(
            msg.contains("missing-2"),
            "msg should name missing-2: {msg}"
        );
        assert!(
            !msg.contains(" a,") && !msg.ends_with(" a"),
            "msg should not list resolved tools as missing: {msg}"
        );
    }

    // ── cost flags ──────────────────────────────────────────────────────

    #[test]
    fn cost_kind_arg_round_trips_to_core_cost_kind() {
        // Pins the CLI ↔ core enum lockstep documented on
        // `CostKindArg`. If a future variant lands on
        // `core::ability_spec::CostKind` and someone forgets the
        // mirror, this test fails loud instead of leaving operators
        // with an unreachable flag.
        use crate::core::ability_spec::CostKind;
        assert_eq!(CostKindArg::Free.into_core(), CostKind::Free);
        assert_eq!(
            CostKindArg::ExternalMetered.into_core(),
            CostKind::ExternalMetered
        );
        assert_eq!(CostKindArg::LlmMetered.into_core(), CostKind::LlmMetered);
        assert_eq!(CostKindArg::Unknown.into_core(), CostKind::Unknown);
    }

    #[test]
    fn build_cost_meta_returns_none_when_kind_absent() {
        // No `--cost-kind` → no `[cost]` table on disk. A bare
        // `--cost-label` without a kind is rejected at clap parse time
        // by `requires = "cost_kind"`, so the helper does not need to
        // re-defend against that case; we just confirm the None path.
        assert!(build_cost_meta(None, None).unwrap().is_none());
        assert!(build_cost_meta(None, Some("ignored")).unwrap().is_none());
    }

    #[test]
    fn build_cost_meta_normalises_blank_label_to_none() {
        // CLI users can technically pass `--cost-label ""` or just
        // whitespace; the manifest validator would reject an empty
        // label outright. Treat empty-ish input as "label omitted" so
        // the kind still lands without dragging a useless blank
        // string onto disk.
        use crate::core::ability_spec::CostKind;
        let meta = build_cost_meta(Some(CostKindArg::ExternalMetered), Some("   "))
            .unwrap()
            .expect("kind set => meta present");
        assert_eq!(meta.kind, CostKind::ExternalMetered);
        assert!(meta.label.is_none());
    }

    #[test]
    fn build_cost_meta_carries_kind_and_trimmed_label() {
        use crate::core::ability_spec::CostKind;
        let meta = build_cost_meta(
            Some(CostKindArg::ExternalMetered),
            Some("  Google Maps API — $5 per 1000 requests  "),
        )
        .unwrap()
        .expect("kind set => meta present");
        assert_eq!(meta.kind, CostKind::ExternalMetered);
        // We keep the inner spacing verbatim; only outer whitespace
        // is normalised so a label written with deliberate alignment
        // (rare, but plausible) survives.
        assert_eq!(
            meta.label.as_deref(),
            Some("Google Maps API — $5 per 1000 requests")
        );
    }

    #[test]
    fn mcp_manifest_for_stamps_declared_cost_on_disk() {
        // Operator passed `--cost-kind external-metered --cost-label
        // "Google Maps geocoding — $5/1000"`. The generated TOML must
        // carry that `[cost]` table verbatim, and re-parsing it must
        // surface the same `CostMeta` — that is what `profiles::mcp`
        // reads to stop reporting `cost: unknown` on this row.
        use crate::core::ability_spec::{AbilityManifest, CostKind, CostMeta};
        let plan = McpAbilityPlan {
            server: "Google Maps MCP".into(),
            tool: "geocode-address".into(),
            verb: "mcp_google_maps_mcp__geocode_address".into(),
            description: "Geocode an address via Google Maps.".into(),
            input_schema: serde_json::json!({"type": "object"}),
            cost: Some(CostMeta {
                kind: CostKind::ExternalMetered,
                label: Some("Google Maps geocoding — $5/1000 requests".into()),
            }),
        };
        let manifest = mcp_manifest_for(&plan).unwrap();
        let cost = manifest
            .cost()
            .expect("declared cost must survive manifest build");
        assert_eq!(cost.kind, CostKind::ExternalMetered);
        assert_eq!(
            cost.label.as_deref(),
            Some("Google Maps geocoding — $5/1000 requests")
        );
        // Round-trip through TOML — `agent mcp add` writes via
        // `to_toml_string`; reading happens via `from_toml_str` at
        // the next daemon boot. Any drift between the two surfaces
        // here as a deserialise failure or label mismatch.
        let body = manifest.to_toml_string().unwrap();
        assert!(
            body.contains("[cost]") && body.contains("external_metered"),
            "manifest TOML must contain a [cost] table with kind = external_metered, got:\n{body}"
        );
        let reparsed = AbilityManifest::from_toml_str(&body).unwrap();
        let reparsed_cost = reparsed.cost().expect("cost survives round-trip");
        assert_eq!(reparsed_cost.kind, CostKind::ExternalMetered);
        assert_eq!(
            reparsed_cost.label.as_deref(),
            Some("Google Maps geocoding — $5/1000 requests")
        );
    }

    #[test]
    fn mcp_manifest_for_without_cost_writes_no_cost_table() {
        // Default — no `--cost-kind` — keeps the on-disk manifest free
        // of any `[cost]` section so the runtime applies its
        // honesty-rule inference (`unknown` for MCP-backed tools).
        // We pin this to prevent a future regression where someone
        // "helpfully" stamps a default cost into every generated file.
        let plan = McpAbilityPlan {
            server: "echo".into(),
            tool: "ping".into(),
            verb: "mcp_echo__ping".into(),
            description: "Echo ping.".into(),
            input_schema: serde_json::json!({"type": "object"}),
            cost: None,
        };
        let body = mcp_manifest_for(&plan).unwrap().to_toml_string().unwrap();
        assert!(
            !body.contains("[cost]"),
            "expected no [cost] table when --cost-kind omitted; got:\n{body}"
        );
    }
}
