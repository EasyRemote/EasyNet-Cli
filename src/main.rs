// EasyNet CLI
// ===========
//
// File: src/main.rs
// Description: Unified binary for EasyNet device management, ability
//              orchestration, and AI-agent integration (MCP + EAL).
//
// What this binary contains
// -------------------------
//
// This binary is the *consumer* side of the EasyNet platform — the
// runtime that reads EAL mission sources, orchestrates abilities
// across registered devices and agents, and exposes that surface over
// an MCP server for in-agent use. The producer side (Agent Ability
// Language compilation to the BCC runtime) is out of scope and lives
// in a sibling binary; we do NOT embed an AAL compiler here. The
// only compiler this binary ships is the EAL compiler under `eal/`.
//
// Protocol Responsibility:
// - Parses CLI arguments via clap and dispatches to subcommand handlers.
// - No business logic lives here; this is purely the shell entry point.
//
// Module Map:
//   cli/     — subcommands (device lifecycle, federation, remote ops,
//              orchestration, AI agents)
//   shared/  — config persistence, bridge factory, terminal output,
//              sysinfo
//   mcp/     — Hub-level MCP tool provider for Claude Code / Codex
//   eal/     — EasyNet Ability Language compiler
//              (lexer → parser → analyzer → IR → interpreter)
//   agent/   — local-agent adapters (Claude Code, Codex) used by EAL
//              member-call form and by the `send_to_agent` MCP tool
//
// Architectural Position:
// - Thinnest possible shell. All intelligence is in cli/mod.rs
//   dispatch and individual handlers.
// - The binary name "easynet" is the single user-facing entry point
//   for the EAL runtime and associated tooling.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

// Crate-level lint policy:
// - needless_pass_by_value: clap Args structs are consumed by value at dispatch boundaries.
// - struct_excessive_bools: CLI flag structs naturally have many bool fields.
// - doc_markdown: product names (EasyNet, EAL, etc.) don't need backticks.
// - module_name_repetitions: e.g. config::ConfigArgs — acceptable for clarity.
#![allow(
    clippy::needless_pass_by_value,
    clippy::struct_excessive_bools,
    clippy::doc_markdown,
    clippy::module_name_repetitions
)]

mod agent;
mod cli;
mod eal;
mod mcp;
mod persistence;
mod registry;
mod shared;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "easynet",
    version,
    about = "EasyNet — device management, remote execution, and real-time communication"
)]
pub struct App {
    #[command(subcommand)]
    pub command: cli::Command,
}

fn main() -> anyhow::Result<()> {
    let app = App::parse();
    cli::run(app.command)
}
