// EasyNet CLI — `easynet agent` inspection surface: doctor + abilities listing
// Split from cli/agent.rs (F-033 / T4.6); bodies are move-only.

use console::style;

use crate::cli::daemon_agent_view::AgentRuntimeKind;
use crate::runtime::directory::AgentDirectory;
use crate::runtime::drivers::{claude_code, codex};

use super::*;

pub(super) fn run_doctor(args: DoctorArgs) -> anyhow::Result<()> {
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

pub(super) fn run_abilities(args: AbilitiesArgs) -> anyhow::Result<()> {
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
