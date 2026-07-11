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

use crate::cli::daemon_client::agent_view::{AgentRuntimeKind, DaemonAgentRow};
use crate::daemon::execution::mission::directory::AgentDirectory;
use crate::support::platform::local_daemon_grpc::LocalDaemonAbilityClient;
use crate::support::platform::timeouts;

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
    /// Agent-scoped object grammar:
    /// `easynet agent <agent-id-or-ura> new-ability ...`.
    #[command(external_subcommand)]
    Scoped(Vec<String>),
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
    /// Program to run for an external/custom agent.
    #[arg(long, value_name = "PROGRAM")]
    pub command: Option<String>,
    /// Argument passed to the external/custom agent program. Repeat for
    /// multiple argv entries.
    #[arg(long = "arg", value_name = "ARG")]
    pub args: Vec<String>,
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

/// CLI adapter for [`crate::daemon::ability::manifest::CostKind`].
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
    pub(crate) fn into_core(self) -> crate::daemon::ability::manifest::CostKind {
        use crate::daemon::ability::manifest::CostKind;
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
        AgentAction::Scoped(tokens) => run_agent_scoped(tokens),
    }
}

fn run_agent_scoped(tokens: Vec<String>) -> anyhow::Result<()> {
    let Some((selector, tail)) = tokens.split_first() else {
        anyhow::bail!("missing agent selector");
    };
    crate::cli::commands::agent_new_ability::run_scoped(selector, tail)
}

// ─── Split out of this file (F-033 / T4.6): args + dispatch only ───
//
// One concern per file; every external path (`agent::AgentArgs`,
// `agent::run`) is unchanged.
mod history;
mod inspect;
mod lifecycle;
mod mcp;
mod publish;
mod send;

#[cfg(test)]
mod tests;

use history::*;
use inspect::*;
use lifecycle::*;
pub(crate) use mcp::*;
use publish::*;
use send::*;

// ── Shared daemon-registry helpers ──────────────────────────────────
// Transplanted verbatim from the pre-split agent.rs (HEAD) when the
// module split left these glue functions behind — every sub-module
// reads agent rows through this one client/row path. pub(super) so
// the split stays the only consumer surface.

#[cfg(feature = "axon-pb")]
fn resolve_local_daemon_caller_ura() -> Option<String> {
    let creds = crate::daemon::persistence::config::load_credentials().ok()?;
    let user_id = creds.user_id().ok()?;
    let username = creds.username_slug().ok()?;
    let plan = crate::cli::commands::start::build_bootstrap_plan_from(
        &creds.realm,
        &creds.node_id,
        user_id,
        username,
    )
    .ok()?;
    Some(plan.host_device_ura)
}

pub(super) fn required_local_daemon_agent_client() -> anyhow::Result<LocalDaemonAbilityClient> {
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

pub(super) fn invoke_daemon_agent_list_required(
    client: &LocalDaemonAbilityClient,
) -> anyhow::Result<Vec<DaemonAgentRow>> {
    crate::cli::daemon_client::agent_view::list_agents_with_client(client)
}

pub(super) fn daemon_agent_row(
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

pub(super) fn daemon_row_agent_type(row: &DaemonAgentRow) -> anyhow::Result<AgentRuntimeKind> {
    crate::cli::daemon_client::agent_view::agent_kind(row)
}

pub(super) fn daemon_row_root(row: &DaemonAgentRow) -> std::path::PathBuf {
    crate::cli::daemon_client::agent_view::agent_root(row)
}

pub(super) fn open_registered_agent(name: &str) -> anyhow::Result<AgentDirectory> {
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
