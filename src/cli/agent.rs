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

use crate::agent::{claude_code, codex};
use crate::cli::mission_runs::{self, MissionRunOpts};
use crate::registry::agents::{self, AgentEntry, AgentType};
use crate::shared::output;
use crate::shared::timeouts;

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
    /// (`shared::timeouts::AGENT_SEND_DEFAULT_SECS`). `0` inherits the
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
    if args.label.is_some() {
        // Routed through the typed builder so this CLI write path
        // doesn't depend on `AgentEntry`'s field layout — when the
        // struct grows new fields, only `with_label` needs to know.
        entry.with_label(args.label.clone());
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
        eprintln!(
            "  Run {} to add one.",
            style("easynet agent add claude --type claude-code --model sonnet").cyan()
        );
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

    let runs_root = crate::persistence::config::state_dir()
        .join("workspaces")
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
}
