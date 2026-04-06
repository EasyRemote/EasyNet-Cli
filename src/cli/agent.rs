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
    /// Timeout in seconds (default: 900 = 15 min)
    #[arg(long, default_value_t = 900)]
    pub timeout: u64,
    /// Write the raw stream-json trace (one event per line) to this file.
    /// The prompt is saved alongside it as `<file>.prompt.txt`.
    #[arg(long)]
    pub trace: Option<std::path::PathBuf>,
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

    // Live trace events from the agent stream to stderr, so we can't use a
    // spinner here (it would fight with the scrolling output). Just print a
    // header and let the dispatch layer stream progress itself.
    eprintln!(
        "  {} {} {}",
        style("→").cyan(),
        style("sending to").dim(),
        style(&args.name).white().bold(),
    );

    let response = dispatch::send_to_agent(
        &args.name,
        &entry,
        &args.prompt,
        args.context.as_deref(),
        None,
        args.trace.as_deref(),
    )?;

    eprintln!();
    eprintln!(
        "  {} {} {}",
        style(&args.name).white().bold(),
        style("done in").dim(),
        style(format!("{:.1}s", response.duration_ms as f64 / 1000.0)).cyan(),
    );
    if let Some(u) = &response.usage {
        let total_in = u.input_tokens + u.cache_read_tokens + u.cache_creation_tokens;
        eprintln!(
            "  {} in={} out={} cache_read={} cache_write={} turns={} cost=${:.4}",
            style("tokens").dim(),
            style(total_in).cyan(),
            style(u.output_tokens).cyan(),
            style(u.cache_read_tokens).dim(),
            style(u.cache_creation_tokens).dim(),
            style(u.num_turns).dim(),
            u.total_cost_usd,
        );
    }
    if let Some(dir) = &response.run_dir {
        eprintln!(
            "  {} {}",
            style("saved").dim(),
            style(dir.display().to_string()).cyan(),
        );
    }
    eprintln!();

    // Render the agent's final reply as markdown when stdout is a TTY;
    // otherwise print raw text so piping into other tools stays clean.
    if console::Term::stdout().is_term() {
        let skin = build_markdown_skin();
        let compact = compact_markdown(&response.content);
        skin.print_text(&compact);
    } else {
        println!("{}", response.content);
    }
    Ok(())
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
        Color::Cyan,       // H1
        Color::Magenta,    // H2
        Color::Yellow,     // H3
        Color::Blue,       // H4
        Color::Green,      // H5
        Color::DarkCyan,   // H6
        Color::DarkMagenta,
        Color::DarkYellow,
    ];
    for (i, h) in skin.headers.iter_mut().enumerate() {
        *h = LineStyle::default();
        h.compound_style = CompoundStyle::with_fg(
            *header_colours.get(i).unwrap_or(&Color::White),
        );
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
        Color::Rgb { r: 220, g: 220, b: 220 },
        Color::Rgb { r: 30, g: 30, b: 40 },
    );

    // Bullets and other decorations.
    skin.bullet = StyledChar::from_fg_char(Color::Cyan, '▸');
    skin.quote_mark = StyledChar::from_fg_char(Color::Blue, '▌');
    skin.horizontal_rule =
        StyledChar::from_fg_char(Color::DarkGrey, '─');

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
