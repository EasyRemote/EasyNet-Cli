// EasyNet CLI — `easynet agent` lifecycle surface: add/list/remove/prune/set/refresh + daemon client plumbing
// Split from cli/agent.rs (F-033 / T4.6); bodies are move-only.

use console::style;

use crate::cli::daemon_client::agent_view::{AgentRuntimeKind, DaemonAgentRow};
use crate::support::platform::output;

use super::*;

pub(super) fn run_add(args: AddArgs) -> anyhow::Result<()> {
    let agent_type: AgentRuntimeKind = args.r#type.parse()?;
    let gateway = agent_command_gateway();
    let name = args.name.clone();
    let daemon_response = invoke_daemon_agent_start_required(
        gateway.as_ref(),
        serde_json::json!({
            "name": name,
            "agent_type": agent_type.to_string(),
            "model": args.model,
            "model_present": true,
            "label": args.label,
            "command": args.command,
            "command_args": args.args,
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

/// Shared CLI→daemon ability invocation with stable error
/// prefixing. The named wrappers below stay as 1-line readers so a
/// `git grep invoke_daemon_agent_start` still surfaces the call
/// site, but the error-format policy and the `.invoke(...)`
/// transport call live in ONE place. A future expansion to typed
/// `invoke::<R>()` per PR-D in
/// `docs/rfc/industrial-textbook-followups-2026-05-29.md` lands
/// here without touching the wrappers.
fn invoke_daemon_ability_required(
    gateway: &dyn AgentCommandGateway,
    ability: &str,
    payload: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    gateway.invoke(ability, payload)
}

fn invoke_daemon_agent_start_required(
    gateway: &dyn AgentCommandGateway,
    payload: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    invoke_daemon_ability_required(gateway, "agent.start", payload)
}

fn invoke_daemon_agent_stop_required(
    gateway: &dyn AgentCommandGateway,
    name: &str,
) -> anyhow::Result<serde_json::Value> {
    invoke_daemon_ability_required(gateway, "agent.stop", serde_json::json!({ "name": name }))
}

fn invoke_daemon_agent_purge_required(
    gateway: &dyn AgentCommandGateway,
    name: &str,
) -> anyhow::Result<serde_json::Value> {
    invoke_daemon_ability_required(gateway, "agent.purge", serde_json::json!({ "name": name }))
}

fn invoke_daemon_agent_refresh_required(
    gateway: &dyn AgentCommandGateway,
    payload: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    invoke_daemon_ability_required(gateway, "agent.refresh", payload)
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
            "daemon accepted agent.start but registered 0 rows \
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

pub(super) fn run_list() -> anyhow::Result<()> {
    let gateway = agent_command_gateway();
    let rows = invoke_daemon_agent_list_required(gateway.as_ref())?;

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

fn render_daemon_agent_row_status(row: &DaemonAgentRow) -> console::StyledObject<&'static str> {
    match row.root_exists {
        Some(true) => style("ok").green(),
        Some(false) => style("path missing").red(),
        None => style("unknown").yellow(),
    }
}

pub(super) fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    let gateway = agent_command_gateway();
    let daemon_response = if args.purge {
        invoke_daemon_agent_purge_required(gateway.as_ref(), &args.name)?
    } else {
        invoke_daemon_agent_stop_required(gateway.as_ref(), &args.name)?
    };
    let ack = daemon_response
        .get("ack")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !ack {
        anyhow::bail!("agent '{}' not found", args.name);
    }
    output::success(&format!("Removed agent '{}'", args.name));
    render_agent_stop_runtime_outcome(&args.name, &daemon_response);

    if args.purge {
        let state = daemon_response
            .get("purge_state")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("agent.purge omitted purge_state"))?;
        let root = daemon_response
            .get("purged_path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("agent.purge omitted purged_path for state={state}"))?;
        match state {
            "purged" => output::detail("purged", root),
            "already_absent" => output::detail("purge", &format!("{root} already absent")),
            other => anyhow::bail!("agent.purge returned unexpected purge_state `{other}`"),
        }
    } else if let Some(root) = daemon_response
        .get("removed_entry")
        .and_then(|entry| entry.get("root_path"))
        .and_then(serde_json::Value::as_str)
    {
        output::detail(
            "kept",
            &format!("{root} (pass --purge to delete credentials + runs)"),
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
pub(super) fn run_prune(args: PruneArgs) -> anyhow::Result<()> {
    let gateway = agent_command_gateway();

    // Identify rows whose daemon-projected registered root is missing.
    // A row without root_path is rejected by daemon_row_root below; the CLI
    // does not derive a persistence path from the agent name.
    let rows = invoke_daemon_agent_list_required(gateway.as_ref())?;
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
        let root = daemon_row_root(name)?;
        eprintln!("    • {}  (missing root: {})", name.name, root.display());
    }
    eprintln!();

    if args.dry_run {
        return Ok(());
    }

    for row in &orphans {
        let resp = invoke_daemon_agent_stop_required(gateway.as_ref(), &row.name)?;
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
pub(super) fn run_set(args: SetArgs) -> anyhow::Result<()> {
    let gateway = agent_command_gateway();
    let row = daemon_agent_row(gateway.as_ref(), &args.name)?;

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
    let root_path = daemon_row_root(&row)?.to_string_lossy().to_string();
    let model_for_request = new_model.clone();
    let daemon_response = invoke_daemon_agent_start_required(
        gateway.as_ref(),
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
/// `agent.refresh`.
pub(super) fn run_refresh(args: RefreshArgs) -> anyhow::Result<()> {
    let gateway = agent_command_gateway();
    let payload = match args
        .agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(name) => serde_json::json!({ "name": name }),
        None => serde_json::json!({}),
    };
    let response = invoke_daemon_agent_refresh_required(gateway.as_ref(), payload)?;
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
    let removed = response
        .get("runtime_removed")
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
            let row_removed = row
                .get("runtime_removed")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            output::detail(
                "refreshed",
                &format!(
                    "{name}: registered {row_registered}, failed {row_failed}, \
                     removed {row_removed}"
                ),
            );
        }
    }
    output::success(&format!(
        "daemon refreshed {scanned} agent(s): registered {registered}, \
         failed {failed}, removed {removed}"
    ));
    Ok(())
}
