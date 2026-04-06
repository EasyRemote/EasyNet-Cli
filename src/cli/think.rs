// EasyNet CLI — Autonomous Agent Loop
// =====================================
//
// File: src/cli/think.rs
// Description: `easynet think` — autonomous think-act-observe loop.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::time::Duration;

use clap::Args;
use console::style;
use indicatif::{ProgressBar, ProgressStyle};

use crate::agent::dispatch;
use crate::shared::agents;

#[derive(Debug, Args)]
pub struct ThinkArgs {
    /// The orchestrator agent
    #[arg(long, default_value = "claude")]
    pub agent: String,
    /// The goal
    pub goal: String,
    /// Max cycles
    #[arg(long, default_value_t = 5)]
    pub max_cycles: usize,
    /// Timeout per call (seconds)
    #[arg(long, default_value_t = 120)]
    pub timeout: u64,
}

pub fn run(args: ThinkArgs) -> anyhow::Result<()> {
    let registry = agents::load_agents()?;
    let entry = registry.agents.get(&args.agent)
        .ok_or_else(|| anyhow::anyhow!("agent '{}' not found", args.agent))?
        .clone();

    // Header
    eprintln!();
    eprintln!("  {} {}", style("easynet think").cyan().bold(), style("autonomous agent loop").dim());
    eprintln!();
    eprintln!("  {}  {}", style("agent").dim(), style(&args.agent).white().bold());
    eprintln!("  {}   {}", style("goal").dim(), truncate(&args.goal, 64));
    eprintln!("  {} {}", style("cycles").dim(), args.max_cycles);
    eprintln!();

    let mut history: Vec<CycleRecord> = Vec::new();

    for cycle in 1..=args.max_cycles {
        eprintln!(
            "  {} {}",
            style(format!("cycle {cycle}")).white().bold(),
            style(format!("of {}", args.max_cycles)).dim()
        );

        // Think
        let spinner = make_spinner("thinking");
        let think_prompt = build_think_prompt(&args.goal, &history);
        let think_response = dispatch::send_to_agent(
            &args.agent, &entry, &think_prompt, None, None, None,
        )?;
        spinner.finish_and_clear();
        eprintln!(
            "  {}  thought  {}",
            style("+").green(),
            style(format!("{}s", think_response.duration_ms / 1000)).dim()
        );

        let action = parse_agent_action(think_response.content.trim());

        match action {
            AgentAction::Done { summary } => {
                eprintln!("  {}  done", style("+").green());
                eprintln!();
                println!("{summary}");
                return Ok(());
            }
            AgentAction::Eal { source, reasoning } => {
                eprintln!(
                    "  {}  plan  {}",
                    style("|").dim(),
                    style(truncate(&reasoning, 60)).dim()
                );

                let spinner = make_spinner("executing mission");
                let (observation, _outputs) = match execute_eal_source(&source) {
                    Ok((summary, outputs)) => {
                        spinner.finish_and_clear();
                        eprintln!("  {}  {}", style("+").green(), style(&summary).green());
                        for (binding, value) in &outputs {
                            eprintln!(
                                "  {}  {} {}",
                                style("|").dim(),
                                style(format!("${binding}")).cyan(),
                                style(truncate(value, 80)).dim()
                            );
                        }
                        let mut obs = format!("{summary}\n");
                        for (k, v) in &outputs {
                            obs.push_str(&format!("${k}: {}\n", truncate(v, 500)));
                        }
                        (obs, outputs)
                    }
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("  {}  {}", style("x").red(), e);
                        (format!("failed: {e}"), std::collections::HashMap::new())
                    }
                };
                history.push(CycleRecord { cycle, reasoning, action: source, observation });
            }
            AgentAction::Bash { command, reasoning } => {
                eprintln!(
                    "  {}  plan  {}",
                    style("|").dim(),
                    style(truncate(&reasoning, 60)).dim()
                );
                eprintln!(
                    "  {}  $ {}",
                    style("|").dim(),
                    style(truncate(&command, 60)).white()
                );

                let spinner = make_spinner("running");
                let observation = match execute_bash(&command) {
                    Ok(out) => {
                        spinner.finish_and_clear();
                        eprintln!("  {}  {}", style("+").green(), style(truncate(&out, 80)).dim());
                        truncate(&out, 2000)
                    }
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("  {}  {}", style("x").red(), e);
                        format!("failed: {e}")
                    }
                };
                history.push(CycleRecord { cycle, reasoning, action: command, observation });
            }
            AgentAction::Think { reasoning } => {
                eprintln!(
                    "  {}  {}",
                    style("|").dim(),
                    style(truncate(&reasoning, 70)).dim()
                );
                history.push(CycleRecord {
                    cycle, reasoning, action: "reflect".into(), observation: "no action".into(),
                });
            }
        }
        eprintln!();
    }

    eprintln!("  {}  max cycles reached", style("!").yellow());
    Ok(())
}

// ── Spinner ──────────────────────────────────────────────────────────────────

fn make_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("  {spinner:.dim} {msg:.dim}")
            .unwrap()
            .tick_strings(&["   ", ".  ", ".. ", "...", " ..", "  .", "   "]),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(120));
    pb
}

// ── Types ────────────────────────────────────────────────────────────────────

#[allow(dead_code)] // All fields populated for structured logging; not all read yet.
struct CycleRecord { cycle: usize, reasoning: String, action: String, observation: String }

enum AgentAction {
    Done { summary: String },
    Eal { source: String, reasoning: String },
    Bash { command: String, reasoning: String },
    Think { reasoning: String },
}

// ── Prompt ───────────────────────────────────────────────────────────────────

fn build_think_prompt(goal: &str, history: &[CycleRecord]) -> String {
    let mut prompt = format!(
r#"You are an autonomous orchestrator in the EasyNet agent network.

## Goal
{goal}

## Actions available

1. **EAL** — orchestrate agents/devices in parallel with data flow
2. **Bash** — run shell or easynet commands
3. **Done** — goal accomplished (include FULL deliverable in SUMMARY)

## Format (pick ONE)

EAL:
REASONING: <brief>
ACTION: EAL
```eal
mission "name" {{
  let x = call "task" on "claude" with {{ prompt = "..." }} timeout 120
  let y = call "task" on "codex" with {{ prompt = "..." }} timeout 120
  let z = call "synthesize" on "claude" with {{ a = x.output, b = y.output }} timeout 120
}}
```

Bash:
REASONING: <brief>
ACTION: BASH
COMMAND: <command>

Done:
REASONING: <brief>
ACTION: DONE
SUMMARY:
<full deliverable content>
"#);

    if !history.is_empty() {
        prompt.push_str("\n## History\n\n");
        for r in history {
            prompt.push_str(&format!(
                "Cycle {}: {}\nResult: {}\n\n",
                r.cycle, truncate(&r.reasoning, 150), truncate(&r.observation, 800),
            ));
        }
    }
    prompt
}

// ── Parse ────────────────────────────────────────────────────────────────────

fn parse_agent_action(response: &str) -> AgentAction {
    let response = response.trim();
    let reasoning = extract_after(response, "REASONING:")
        .unwrap_or_else(|| response.lines().next().unwrap_or("").to_string());

    if response.contains("ACTION: DONE") || response.contains("ACTION:DONE") {
        let summary = extract_section_after(response, "SUMMARY:")
            .unwrap_or_else(|| reasoning.clone());
        return AgentAction::Done { summary };
    }
    if response.contains("ACTION: EAL") || response.contains("```eal") {
        if let Some(eal) = extract_code_block(response, "eal") {
            return AgentAction::Eal { source: eal, reasoning };
        }
    }
    if response.contains("ACTION: BASH") {
        if let Some(cmd) = extract_after(response, "COMMAND:") {
            return AgentAction::Bash { command: cmd.trim().to_string(), reasoning };
        }
    }
    AgentAction::Think { reasoning }
}

fn extract_after(text: &str, prefix: &str) -> Option<String> {
    text.lines().find(|l| l.trim().starts_with(prefix))
        .map(|l| l.trim().trim_start_matches(prefix).trim().to_string())
}
fn extract_section_after(text: &str, prefix: &str) -> Option<String> {
    let mut found = false;
    let mut lines = Vec::new();
    for line in text.lines() {
        if line.trim().starts_with(prefix) {
            found = true;
            let rest = line.trim().trim_start_matches(prefix).trim();
            if !rest.is_empty() { lines.push(rest.to_string()); }
        } else if found { lines.push(line.to_string()); }
    }
    if found { Some(lines.join("\n").trim().to_string()) } else { None }
}
fn extract_code_block(text: &str, lang: &str) -> Option<String> {
    let marker = format!("```{lang}");
    let start = text.find(&marker)?;
    let after = &text[start + marker.len()..];
    let end = after.find("```")?;
    Some(after[..end].trim().to_string())
}

// ── Execute ──────────────────────────────────────────────────────────────────

fn execute_eal_source(source: &str) -> anyhow::Result<(String, std::collections::HashMap<String, String>)> {
    use crate::eal;
    use crate::shared::config;
    let program = eal::parser::parse(source).map_err(|e| anyhow::anyhow!("parse: {e}"))?;
    let ir = eal::planner::compile(&program).map_err(|e| anyhow::anyhow!("compile: {e}"))?;
    let state = config::load()?;
    let report = eal::interpreter::execute_with_endpoint(&state.endpoint, "default", &ir)?;
    Ok((
        format!("{} ok, {} failed, {:.1}s", report.steps_completed, report.steps_failed, report.total_elapsed_ms as f64 / 1000.0),
        report.outputs,
    ))
}

fn execute_bash(command: &str) -> anyhow::Result<String> {
    use crate::agent::process_runner::{self, ChildOptions};
    let result = process_runner::run_child("sh", &["-c", command], ChildOptions {
        timeout: Duration::from_secs(30), ..Default::default()
    })?;
    if result.exit_code != 0 { anyhow::bail!("exit {}: {}", result.exit_code, result.stderr.trim()); }
    Ok(result.stdout)
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ").replace('\r', "");
    if s.len() <= max { s } else { format!("{}...", &s[..max]) }
}
