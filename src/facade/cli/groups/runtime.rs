// EasyNet CLI — Runtime Group
// ===========================
//
// File: src/cli/groups/runtime.rs
// Description: `easynet runtime …` — every operation that affects the
//              *local* Axon runtime process on this host.
//
// Verbs:
//   start    Spawn (or attach to) a local runtime  (-> cli::start)
//   stop     Shut down the local runtime           (-> cli::stop)
//   status   Show local runtime + federation info  (-> cli::status)
//   connect  Foreground "paired device" mode       (-> cli::connect)
//   logs     Tail the runtime log file             (NEW)
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::io::{BufRead, BufReader, Seek, SeekFrom};

use clap::{Args, Subcommand};
use console::style;

use crate::facade::cli::{connect, start, status, stop};
use crate::persistence::config;

#[derive(Debug, Args)]
pub struct RuntimeArgs {
    #[command(subcommand)]
    pub action: RuntimeAction,
}

#[derive(Debug, Subcommand)]
pub enum RuntimeAction {
    /// Start a local Axon runtime as a background daemon (records pid
    /// and endpoint in `~/.easynet/runtime.json`).
    Start(start::StartArgs),
    /// Signal the running runtime to shut down cleanly, then remove
    /// `~/.easynet/runtime.json`.
    Stop(stop::StopArgs),
    /// Report runtime process liveness, bridge endpoint, Hub
    /// reachability, and online/offline node counts.
    Status(status::StatusArgs),
    /// Run the paired device in the foreground (no background daemon).
    /// Blocks until Ctrl-C; useful for `systemd` / container PID 1.
    Connect(connect::ConnectArgs),
    /// Tail (and optionally follow) the local runtime log file.
    Logs(LogsArgs),
}

#[derive(Debug, Args)]
pub struct LogsArgs {
    /// Number of trailing lines to print before streaming.
    #[arg(long, default_value_t = 100)]
    pub tail: usize,
    /// Follow the file as new lines are appended.
    #[arg(long, short = 'f')]
    pub follow: bool,
}

pub fn run(args: RuntimeArgs) -> anyhow::Result<()> {
    match args.action {
        RuntimeAction::Start(a) => start::run(a),
        RuntimeAction::Stop(a) => stop::run(a),
        RuntimeAction::Status(a) => status::run(a),
        RuntimeAction::Connect(a) => connect::run(a),
        RuntimeAction::Logs(a) => run_logs(a),
    }
}

fn run_logs(args: LogsArgs) -> anyhow::Result<()> {
    let log_path = config::state_dir().join("axon.log");
    if !log_path.exists() {
        anyhow::bail!(
            "no runtime log file at {}\n\
             Start the runtime with `easynet runtime start` first.",
            log_path.display()
        );
    }

    eprintln!(
        "  {} {}",
        style("logs").dim(),
        style(log_path.display().to_string()).cyan()
    );

    let file = std::fs::File::open(&log_path)?;
    let mut reader = BufReader::new(file);

    // Print the last `tail` lines.
    let lines: Vec<String> = BufReader::new(std::fs::File::open(&log_path)?)
        .lines()
        .map_while(Result::ok)
        .collect();
    let start = lines.len().saturating_sub(args.tail);
    for line in &lines[start..] {
        println!("{line}");
    }

    if !args.follow {
        return Ok(());
    }

    // Naive follow loop: re-open + seek to EOF and poll for new lines.
    reader.seek(SeekFrom::End(0))?;
    let mut buf = String::new();
    loop {
        buf.clear();
        match reader.read_line(&mut buf) {
            Ok(0) => std::thread::sleep(std::time::Duration::from_millis(250)),
            Ok(_) => print!("{buf}"),
            Err(e) => {
                eprintln!("  {}: {e}", style("log read error").red());
                break;
            }
        }
    }
    Ok(())
}
