// EasyNet CLI — Agent Group
// =========================
//
// File: src/cli/groups/agent.rs
// Description: `easynet agent …` — registration, single-shot dispatch,
//              multi-turn sessions, run-trace inspection, autonomous
//              think loops, and multi-agent discussions, all rooted on the
//              same noun.
//
// Verbs:
//   add / list / remove / doctor / send   (passthrough to cli::agent)
//   session new <id> --agent <a> [--initial <prompt>]    (NEW)
//   session resume <id> <prompt>                          (NEW)
//   session list                                          (NEW)
//   session show <id>                                     (NEW)
//   session end <id>                                      (NEW)
//   trace list [--agent <a>]                              (NEW)
//   trace show <run-dir>                                  (NEW)
//   think <goal>                                          (-> cli::think)
//   discuss [...]                                         (-> cli::discuss)
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};
use console::style;

use crate::agent::dispatch;
use crate::cli::agent as legacy_agent;
use crate::cli::agent_sessions::{self, Session};
use crate::cli::{discuss, think};
use crate::shared::{agents, config, output};

#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub action: AgentAction,
}

#[derive(Debug, Subcommand)]
pub enum AgentAction {
    /// Register a new agent.
    Add(legacy_agent::AddArgs),
    /// List registered agents.
    List,
    /// Remove a registered agent.
    Remove(legacy_agent::RemoveArgs),
    /// Send a one-shot prompt to an agent.
    Send(legacy_agent::SendArgs),
    /// Check whether the agent CLIs are installed and authenticated.
    Doctor(legacy_agent::DoctorArgs),
    /// Multi-turn agent sessions.
    Session(SessionArgs),
    /// Inspect persisted run traces.
    Trace(TraceArgs),
    /// Autonomous goal-directed agent loop.
    Think(think::ThinkArgs),
    /// Orchestrate a multi-agent discussion.
    Discuss(discuss::DiscussArgs),
}

#[derive(Debug, Args)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub action: SessionAction,
}

#[derive(Debug, Subcommand)]
pub enum SessionAction {
    /// Start a new session.
    New(SessionNewArgs),
    /// Continue an existing session with another prompt.
    Resume(SessionResumeArgs),
    /// List existing sessions.
    List,
    /// Show one session's full transcript.
    Show(SessionShowArgs),
    /// Delete a session.
    End(SessionEndArgs),
}

#[derive(Debug, Args)]
pub struct SessionNewArgs {
    /// Session id (any short label, must be unique).
    pub id: String,
    /// Registered agent name to bind to the session.
    #[arg(long)]
    pub agent: String,
    /// Optional first prompt to send immediately.
    #[arg(long)]
    pub initial: Option<String>,
    /// Per-call timeout in seconds (default: 900).
    #[arg(long, default_value_t = 900)]
    pub timeout: u64,
}

#[derive(Debug, Args)]
pub struct SessionResumeArgs {
    /// Session id.
    pub id: String,
    /// Next prompt to send.
    pub prompt: String,
    /// Override per-call timeout.
    #[arg(long, default_value_t = 900)]
    pub timeout: u64,
}

#[derive(Debug, Args)]
pub struct SessionShowArgs {
    /// Session id.
    pub id: String,
}

#[derive(Debug, Args)]
pub struct SessionEndArgs {
    /// Session id to delete.
    pub id: String,
}

#[derive(Debug, Args)]
pub struct TraceArgs {
    #[command(subcommand)]
    pub action: TraceAction,
}

#[derive(Debug, Subcommand)]
pub enum TraceAction {
    /// List persisted run directories.
    List(TraceListArgs),
    /// Show a single run's metadata, prompt, and response.
    Show(TraceShowArgs),
}

#[derive(Debug, Args)]
pub struct TraceListArgs {
    /// Filter by registered agent name.
    #[arg(long)]
    pub agent: Option<String>,
    /// Maximum number of runs to list.
    #[arg(long, default_value_t = 20)]
    pub limit: usize,
}

#[derive(Debug, Args)]
pub struct TraceShowArgs {
    /// Run directory id (timestamp), or unique prefix. May be qualified
    /// as `<agent>/<id>` if the same id exists across multiple agents.
    pub id: String,
    /// Print the raw trace.jsonl instead of the meta + response summary.
    #[arg(long)]
    pub raw: bool,
}

pub fn run(args: AgentArgs) -> anyhow::Result<()> {
    match args.action {
        AgentAction::Add(a) => legacy_agent::run(legacy_agent::AgentArgs {
            action: legacy_agent::AgentAction::Add(a),
        }),
        AgentAction::List => legacy_agent::run(legacy_agent::AgentArgs {
            action: legacy_agent::AgentAction::List,
        }),
        AgentAction::Remove(a) => legacy_agent::run(legacy_agent::AgentArgs {
            action: legacy_agent::AgentAction::Remove(a),
        }),
        AgentAction::Send(a) => legacy_agent::run(legacy_agent::AgentArgs {
            action: legacy_agent::AgentAction::Send(a),
        }),
        AgentAction::Doctor(a) => legacy_agent::run(legacy_agent::AgentArgs {
            action: legacy_agent::AgentAction::Doctor(a),
        }),
        AgentAction::Session(a) => run_session(a),
        AgentAction::Trace(a) => run_trace(a),
        AgentAction::Think(a) => think::run(a),
        AgentAction::Discuss(a) => discuss::run(a),
    }
}

// ── sessions ────────────────────────────────────────────────────────────────

fn run_session(args: SessionArgs) -> anyhow::Result<()> {
    match args.action {
        SessionAction::New(a) => session_new(a),
        SessionAction::Resume(a) => session_resume(a),
        SessionAction::List => session_list(),
        SessionAction::Show(a) => session_show(a),
        SessionAction::End(a) => session_end(a),
    }
}

fn session_new(args: SessionNewArgs) -> anyhow::Result<()> {
    let registry = agents::load_agents()?;
    if !registry.agents.contains_key(&args.agent) {
        anyhow::bail!(
            "agent '{}' not found. Run `easynet agent list`.",
            args.agent
        );
    }
    if agent_sessions::session_path(&args.id).exists() {
        anyhow::bail!("session '{}' already exists", args.id);
    }

    let mut session = Session::new(args.id.clone(), args.agent.clone());
    session.save()?;
    output::success(&format!("created session '{}' (agent: {})", args.id, args.agent));

    if let Some(prompt) = args.initial {
        send_in_session(&mut session, &prompt, args.timeout)?;
    }
    Ok(())
}

fn session_resume(args: SessionResumeArgs) -> anyhow::Result<()> {
    let mut session = Session::load(&args.id)?;
    send_in_session(&mut session, &args.prompt, args.timeout)
}

fn session_list() -> anyhow::Result<()> {
    let sessions = agent_sessions::list_sessions()?;
    if sessions.is_empty() {
        output::info("No agent sessions yet. Start one with `easynet agent session new`.");
        return Ok(());
    }
    let mut table = output::table(&["ID", "Agent", "Turns", "Updated"]);
    for s in &sessions {
        let turns = s.turns.len().to_string();
        table.add_row(vec![&s.id, &s.agent, &turns, &s.updated_at]);
    }
    println!("{table}");
    Ok(())
}

fn session_show(args: SessionShowArgs) -> anyhow::Result<()> {
    let session = Session::load(&args.id)?;
    eprintln!();
    eprintln!(
        "  {} {}  {}",
        style("session").dim(),
        style(&session.id).bold(),
        style(format!("({} turns)", session.turns.len())).dim()
    );
    output::detail("agent", &session.agent);
    output::detail("created", &session.created_at);
    output::detail("updated", &session.updated_at);
    eprintln!();
    for (i, t) in session.turns.iter().enumerate() {
        let role = if t.role == "user" {
            style("user").cyan()
        } else {
            style("assistant").magenta()
        };
        eprintln!("  {} {}", style(format!("#{:02}", i + 1)).dim(), role);
        for line in t.content.lines() {
            eprintln!("    {line}");
        }
        eprintln!();
    }
    Ok(())
}

fn session_end(args: SessionEndArgs) -> anyhow::Result<()> {
    agent_sessions::delete_session(&args.id)?;
    output::success(&format!("ended session '{}'", args.id));
    Ok(())
}

fn send_in_session(session: &mut Session, prompt: &str, timeout: u64) -> anyhow::Result<()> {
    let registry = agents::load_agents()?;
    let entry = registry
        .agents
        .get(&session.agent)
        .ok_or_else(|| anyhow::anyhow!("agent '{}' not registered", session.agent))?;
    let mut entry = entry.clone();
    entry.timeout_secs = timeout;

    // Render prior turns into the context window.
    let context = if session.turns.is_empty() {
        None
    } else {
        Some(session.transcript())
    };

    eprintln!(
        "  {} {} {} {}",
        style("→").cyan(),
        style("session").dim(),
        style(&session.id).bold(),
        style(format!("(turn {})", session.turns.len() / 2 + 1)).dim(),
    );

    let response = dispatch::send_to_agent(
        &session.agent,
        &entry,
        prompt,
        context.as_deref(),
        None,
        None,
    )?;

    session.append("user", prompt);
    session.append("assistant", &response.content);
    session.save()?;

    eprintln!();
    eprintln!(
        "  {} {:.1}s",
        style(&session.agent).white().bold(),
        response.duration_ms as f64 / 1000.0
    );
    if let Some(dir) = &response.run_dir {
        eprintln!("  {} {}", style("saved").dim(), style(dir.display().to_string()).cyan());
    }
    eprintln!();
    println!("{}", response.content);
    Ok(())
}

// ── traces ──────────────────────────────────────────────────────────────────

fn run_trace(args: TraceArgs) -> anyhow::Result<()> {
    match args.action {
        TraceAction::List(a) => trace_list(a),
        TraceAction::Show(a) => trace_show(a),
    }
}

#[derive(Debug, Clone)]
struct TraceEntry {
    agent: String,
    id: String,
    path: std::path::PathBuf,
    meta: serde_json::Value,
}

fn collect_traces(filter_agent: Option<&str>) -> anyhow::Result<Vec<TraceEntry>> {
    let workspaces = config::state_dir().join("workspaces");
    if !workspaces.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&workspaces)? {
        let entry = entry?;
        let agent_name = entry.file_name().to_string_lossy().to_string();
        if let Some(f) = filter_agent {
            if f != agent_name {
                continue;
            }
        }
        let runs_dir = entry.path().join("runs");
        if !runs_dir.exists() {
            continue;
        }
        for run in std::fs::read_dir(&runs_dir)? {
            let run = run?;
            let path = run.path();
            if !path.is_dir() {
                continue;
            }
            let id = run.file_name().to_string_lossy().to_string();
            let meta_path = path.join("meta.json");
            let meta: serde_json::Value = std::fs::read_to_string(&meta_path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or(serde_json::json!({}));
            out.push(TraceEntry {
                agent: agent_name.clone(),
                id,
                path,
                meta,
            });
        }
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}

fn trace_list(args: TraceListArgs) -> anyhow::Result<()> {
    let traces = collect_traces(args.agent.as_deref())?;
    if traces.is_empty() {
        output::info("No agent run traces yet. Run `easynet agent send …` first.");
        return Ok(());
    }
    let mut table = output::table(&["ID", "Agent", "Status", "Tokens", "Duration"]);
    for t in traces.iter().take(args.limit) {
        let status = t
            .meta
            .get("exit_status")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let in_t = t.meta.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let out_t = t.meta.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
        let tokens = format!("{in_t}/{out_t}");
        let dur_ms = t.meta.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
        let dur = format!("{:.1}s", dur_ms as f64 / 1000.0);
        table.add_row(vec![&t.id, &t.agent, status, &tokens, &dur]);
    }
    println!("{table}");
    Ok(())
}

fn trace_show(args: TraceShowArgs) -> anyhow::Result<()> {
    let traces = collect_traces(None)?;
    let (agent_filter, id_part) = match args.id.split_once('/') {
        Some((a, i)) => (Some(a.to_string()), i.to_string()),
        None => (None, args.id.clone()),
    };
    let matches: Vec<&TraceEntry> = traces
        .iter()
        .filter(|t| {
            agent_filter.as_deref().is_none_or(|a| a == t.agent)
                && (t.id == id_part || t.id.starts_with(&id_part))
        })
        .collect();
    if matches.is_empty() {
        anyhow::bail!("no trace matching '{}'", args.id);
    }
    if matches.len() > 1 {
        let names: Vec<String> = matches.iter().map(|t| format!("{}/{}", t.agent, t.id)).collect();
        anyhow::bail!("ambiguous '{}' — matches: {}", args.id, names.join(", "));
    }
    let entry = matches[0];

    eprintln!();
    eprintln!(
        "  {} {}  {}",
        style("trace").dim(),
        style(&entry.id).bold(),
        style(format!("({})", entry.agent)).dim()
    );
    output::detail("path", &entry.path.display().to_string());
    if let Some(model) = entry.meta.get("model").and_then(|v| v.as_str()) {
        output::detail("model", model);
    }
    if let Some(status) = entry.meta.get("exit_status").and_then(|v| v.as_str()) {
        output::detail("status", status);
    }
    if let Some(dur) = entry.meta.get("duration_ms").and_then(|v| v.as_u64()) {
        output::detail("duration", &format!("{:.1}s", dur as f64 / 1000.0));
    }
    eprintln!();

    if args.raw {
        let trace_path = entry.path.join("trace.jsonl");
        if trace_path.exists() {
            let raw = std::fs::read_to_string(trace_path)?;
            print!("{raw}");
        } else {
            output::info("no trace.jsonl recorded");
        }
        return Ok(());
    }

    let prompt_path = entry.path.join("prompt.txt");
    if prompt_path.exists() {
        eprintln!("  {}", style("prompt").dim());
        let prompt = std::fs::read_to_string(prompt_path)?;
        for line in prompt.lines() {
            eprintln!("    {line}");
        }
        eprintln!();
    }
    let resp_path = entry.path.join("response.md");
    if resp_path.exists() {
        eprintln!("  {}", style("response").dim());
        let resp = std::fs::read_to_string(resp_path)?;
        println!("{resp}");
    }
    Ok(())
}
