// EasyNet CLI — Top-Level Logs View
// =================================
//
// File: src/cli/logs.rs
// Description: `easynet logs` — cross-subject logs aggregator. Right now we
//              expose three log surfaces:
//                * runtime — `~/.easynet/axon.log`
//                * agent   — most recent agent run trace
//                * mission — most recent mission run trace
//
// This is intentionally lightweight: it does not multiplex live tails, it
// just gives the user one entry point that surfaces the file path and
// trailing lines for whichever subject they care about.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use clap::Args;
use console::style;

use crate::cli::groups::runtime::{self as runtime_group, LogsArgs as RuntimeLogsArgs, RuntimeAction, RuntimeArgs};
use crate::cli::mission_runs;
use crate::shared::{config, output};

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Subject to view: runtime | agent | mission
    #[arg(default_value = "runtime")]
    pub subject: String,
    /// Number of trailing lines to show.
    #[arg(long, default_value_t = 100)]
    pub tail: usize,
    /// Follow new lines (runtime only).
    #[arg(long, short = 'f')]
    pub follow: bool,
}

pub fn run(args: LogsArgs) -> anyhow::Result<()> {
    match args.subject.as_str() {
        "runtime" => runtime_group::run(RuntimeArgs {
            action: RuntimeAction::Logs(RuntimeLogsArgs {
                tail: args.tail,
                follow: args.follow,
            }),
        }),
        "agent" => show_latest_agent(args.tail),
        "mission" => show_latest_mission(args.tail),
        other => anyhow::bail!("unknown logs subject '{other}' (expected: runtime | agent | mission)"),
    }
}

fn show_latest_agent(tail: usize) -> anyhow::Result<()> {
    let workspaces = config::state_dir().join("workspaces");
    if !workspaces.exists() {
        output::info("no agent runs recorded");
        return Ok(());
    }
    let mut latest: Option<(String, PathBuf)> = None;
    for entry in std::fs::read_dir(&workspaces)? {
        let entry = entry?;
        let runs_dir = entry.path().join("runs");
        if !runs_dir.exists() {
            continue;
        }
        for run in std::fs::read_dir(&runs_dir)? {
            let run = run?;
            let id = run.file_name().to_string_lossy().to_string();
            if latest.as_ref().is_none_or(|(prev_id, _)| id > *prev_id) {
                latest = Some((id, run.path()));
            }
        }
    }
    let (id, path) = latest.ok_or_else(|| anyhow::anyhow!("no agent runs recorded"))?;
    eprintln!(
        "  {} {} {}",
        style("agent run").dim(),
        style(&id).bold(),
        style(path.display().to_string()).cyan()
    );
    let trace_path = path.join("trace.jsonl");
    if !trace_path.exists() {
        output::info("no trace.jsonl in this run");
        return Ok(());
    }
    print_tail(&trace_path, tail)
}

fn show_latest_mission(tail: usize) -> anyhow::Result<()> {
    let runs = mission_runs::list_runs()?;
    let run = runs
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no mission runs recorded"))?;
    eprintln!(
        "  {} {} {}",
        style("mission run").dim(),
        style(&run.id).bold(),
        style(run.path.display().to_string()).cyan()
    );
    let trace_path = run.path.join("trace.json");
    if !trace_path.exists() {
        output::info("no trace.json in this run");
        return Ok(());
    }
    print_tail(&trace_path, tail)
}

fn print_tail(path: &std::path::Path, tail: usize) -> anyhow::Result<()> {
    let file = std::fs::File::open(path)?;
    let lines: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
    let start = lines.len().saturating_sub(tail);
    for line in &lines[start..] {
        println!("{line}");
    }
    Ok(())
}
