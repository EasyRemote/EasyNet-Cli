// EasyNet CLI
// ===========
//
// File: src/cli/mod.rs
// Description: Command routing hub — defines all 10 subcommands and dispatches to handlers.
//
// Architectural Position:
// - Single entry point from main.rs. Each subcommand is a module with its own `Args` struct
//   and `run()` function, keeping command logic isolated and independently testable.
// - The `Command` enum is the contract between CLI argument parsing and business logic.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod abilities;
pub mod deploy;
pub mod devices;
pub mod exec;
pub mod invoke;
pub mod mcp_server;
pub mod mission;
pub mod start;
pub mod status;
pub mod stop;

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start a local Axon runtime and (optionally) join a Hub.
    Start(start::StartArgs),
    /// Stop the locally started runtime (best-effort).
    Stop(stop::StopArgs),
    /// Show runtime / hub status and node summary.
    Status(status::StatusArgs),
    /// List devices (nodes) known to the runtime / hub.
    Devices(devices::DevicesArgs),
    /// List abilities (MCP tools) across nodes.
    Abilities(abilities::AbilitiesArgs),
    /// One-shot remote command execution (ephemeral ability).
    Exec(exec::ExecArgs),
    /// Deploy an ability package/descriptor to a node.
    Deploy(deploy::DeployArgs),
    /// Invoke an ability on a node.
    Invoke(invoke::InvokeArgs),
    /// Compile and run EAL missions.
    Mission(mission::MissionArgs),
    /// Run a Hub-level MCP server on stdio.
    McpServer(mcp_server::McpServerArgs),
}

pub fn run(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        Command::Start(args) => start::run(args),
        Command::Stop(args) => stop::run(args),
        Command::Status(args) => status::run(args),
        Command::Devices(args) => devices::run(args),
        Command::Abilities(args) => abilities::run(args),
        Command::Exec(args) => exec::run(args),
        Command::Deploy(args) => deploy::run(args),
        Command::Invoke(args) => invoke::run(args),
        Command::Mission(args) => mission::run(args),
        Command::McpServer(args) => mcp_server::run(args),
    }
}

