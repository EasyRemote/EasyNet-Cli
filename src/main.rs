// EasyNet CLI
// ===========
//
// File: src/main.rs
// Description: Binary entry point — parses subcommands and dispatches to handlers.
//
// Architectural Position:
// - Top-level orchestrator. Delegates all logic to `cli::run(Command)`.
// - No business logic lives here; this is purely argument parsing and dispatch.
//
// Subcommands:
//   start, stop, status     — runtime lifecycle
//   devices, abilities      — federation queries
//   exec, deploy, invoke    — remote operations
//   mission                 — EAL compilation and execution
//   mcp-server              — stdio MCP server for Claude Code / Codex
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

mod cli;
mod eal;
mod mcp;
mod shared;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "easynet", version, about = "EasyNet Hub CLI + MCP server")]
struct App {
    #[command(subcommand)]
    command: cli::Command,
}

fn main() -> anyhow::Result<()> {
    let app = App::parse();
    cli::run(app.command)
}

