// EasyNet CLI
// ===========
//
// File: src/main.rs
// Description: Unified binary for EasyNet device management, ability orchestration,
//              and AI-agent integration (MCP + EAL).
//
// Protocol Responsibility:
// - Parses CLI arguments via clap and dispatches to subcommand handlers.
// - No business logic lives here; this is purely the shell entry point.
//
// Module Map:
//   cli/     — 14 subcommands (device lifecycle, federation, remote ops, orchestration)
//   shared/  — config persistence, bridge factory, terminal output, sysinfo
//   mcp/     — Hub-level MCP tool provider (11 tools for Claude Code / Codex)
//   eal/     — EasyNet Ability Language compiler (lexer → parser → analyzer → IR → interpreter)
//
// Architectural Position:
// - Thinnest possible shell. All intelligence is in cli/mod.rs dispatch and individual handlers.
// - The binary name "easynet" is the single user-facing entry point for the entire platform CLI.
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
    // Internal daemon entry point — intercept before clap to keep it out of the public API.
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("_heartbeat-daemon")) {
        return cli::start::run_heartbeat_daemon();
    }

    let app = App::parse();
    cli::run(app.command)
}

