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

use crate::agent::{claude_code, codex, dispatch};
use crate::shared::agents::{self, AgentEntry, AgentType};
use crate::shared::output;

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
}

#[derive(Debug, Args)]
pub struct SendArgs {
    /// Agent name
    pub name: String,
    /// Prompt to send
    pub prompt: String,
    /// Optional context to include
    #[arg(long)]
    pub context: Option<String>,
    /// Timeout in seconds
    #[arg(long, default_value_t = 300)]
    pub timeout: u64,
}

#[derive(Debug, Args)]
pub struct DoctorArgs {
    /// Check a specific agent (or all if omitted)
    pub name: Option<String>,
}

pub fn run(args: AgentArgs) -> anyhow::Result<()> {
    match args.action {
        AgentAction::Add(a) => run_add(a),
        AgentAction::List => run_list(),
        AgentAction::Remove(a) => run_remove(a),
        AgentAction::Send(a) => run_send(a),
        AgentAction::Doctor(a) => run_doctor(a),
    }
}

fn run_add(args: AddArgs) -> anyhow::Result<()> {
    let agent_type: AgentType = args.r#type.parse()?;
    let mut registry = agents::load_agents()?;

    let mut entry = AgentEntry::new(agent_type, args.model.clone());
    if let Some(label) = args.label {
        entry.label = Some(label);
    }

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
    Ok(())
}

fn run_list() -> anyhow::Result<()> {
    let registry = agents::load_agents()?;

    if registry.agents.is_empty() {
        eprintln!("  No agents registered.");
        eprintln!("  Run {} to add one.", style("easynet agent add claude --type claude-code --model sonnet").cyan());
        return Ok(());
    }

    eprintln!();
    // Header
    eprintln!(
        "  {:<14} {:<18} {:<12} {}",
        style("NAME").dim(),
        style("TYPE").dim(),
        style("MODEL").dim(),
        style("TIMEOUT").dim(),
    );
    eprintln!("  {}", style("─".repeat(52)).dim());

    for (name, entry) in &registry.agents {
        let model = entry.model.as_deref().unwrap_or("-");
        let type_styled = match entry.agent_type {
            agents::AgentType::ClaudeCode => style("claude-code").magenta(),
            agents::AgentType::Codex => style("codex").yellow(),
            agents::AgentType::CodexAppServer => style("codex-app-server").yellow(),
        };
        eprintln!(
            "  {:<14} {:<18} {:<12} {}",
            style(name).white().bold(),
            type_styled,
            style(model).cyan(),
            style(format!("{}s", entry.timeout_secs)).dim(),
        );
    }
    eprintln!();
    Ok(())
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    let mut registry = agents::load_agents()?;

    if registry.agents.remove(&args.name).is_none() {
        anyhow::bail!("agent '{}' not found", args.name);
    }

    agents::save_agents(&registry)?;
    output::success(&format!("Removed agent '{}'", args.name));
    Ok(())
}

fn run_send(args: SendArgs) -> anyhow::Result<()> {
    let registry = agents::load_agents()?;

    let entry = registry.agents.get(&args.name)
        .ok_or_else(|| anyhow::anyhow!("agent '{}' not found. Run `easynet agent list`.", args.name))?;

    // Override timeout if specified.
    let mut entry = entry.clone();
    entry.timeout_secs = args.timeout;

    let spinner = indicatif::ProgressBar::new_spinner();
    spinner.set_style(
        indicatif::ProgressStyle::with_template("  {spinner:.dim} {msg:.dim}")
            .unwrap()
            .tick_strings(&["   ", ".  ", ".. ", "...", " ..", "  .", "   "]),
    );
    spinner.set_message(format!("sending to {}", args.name));
    spinner.enable_steady_tick(std::time::Duration::from_millis(120));

    let response = dispatch::send_to_agent(
        &args.name,
        &entry,
        &args.prompt,
        args.context.as_deref(),
        None,
    )?;

    spinner.finish_and_clear();
    eprintln!(
        "  {} {:.1}s",
        style(&args.name).white().bold(),
        response.duration_ms as f64 / 1000.0,
    );
    eprintln!();
    println!("{}", response.content);
    Ok(())
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
                registry.agents.iter()
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
        eprintln!("  Claude Code  {}", style("https://claude.ai/download").dim());
        eprintln!("  Codex        {}", style("npm install -g @openai/codex").dim());
        eprintln!();
    }

    Ok(())
}
