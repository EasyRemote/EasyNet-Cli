// EasyNet CLI
// ===========
//
// File: src/cli/mod.rs
// Description: Command routing hub — defines all subcommands and dispatches to handlers.
//
// Protocol Responsibility:
// - Owns the `Command` enum which is the exhaustive contract between CLI argument parsing
//   and business logic. Adding a subcommand means adding a variant here.
// - Dispatch is a pure match with no cross-command logic — each module is self-contained.
//
// Subcommand Groups:
//   Device lifecycle:   join, connect, config, reset
//   Runtime lifecycle:  start, stop, status
//   Federation queries: devices, abilities
//   Remote operations:  exec, deploy, invoke
//   Orchestration:      mission (EAL compiler + executor)
//   AI integration:     mcp-server (stdio MCP for Claude Code / Codex)
//
// Architectural Position:
// - Single fan-out point from main.rs. No business logic lives here.
// - Each subcommand module exports an `Args` struct (clap derive) and a `run()` function.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod abilities;
pub mod config_cmd;
pub mod connect;
pub mod deploy;
pub mod devices;
pub mod exec;
pub mod invoke;
pub mod join;
pub mod mcp_server;
pub mod mission;
pub mod reset;
pub mod start;
pub mod status;
pub mod stop;

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Pair this device with EasyNet using a pairing token.
    Join(join::JoinArgs),
    /// Connect to Hub as a paired device and optionally start MCP server.
    Connect(connect::ConnectArgs),
    /// Show or set device settings (session_bridge exec enable).
    Config(config_cmd::ConfigArgs),
    /// Remove device credentials and un-pair.
    Reset(reset::ResetArgs),
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
        Command::Join(args) => join::run(args),
        Command::Connect(args) => connect::run(args),
        Command::Config(args) => config_cmd::run(args),
        Command::Reset(args) => reset::run(args),
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

