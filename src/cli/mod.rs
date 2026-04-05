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
pub mod agent;
pub mod config_cmd;
pub mod connect;
pub mod deploy;
pub mod devices;
pub mod discuss;
pub mod exec;
pub mod heartbeat;
pub mod invoke;
pub mod join;
pub mod mcp_server;
pub mod mission;
pub mod reset;
pub mod start;
pub mod status;
pub mod stop;
pub mod mcp_install;
pub mod skill_install;
pub mod think;

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum Command {
    // ── Runtime lifecycle (most common) ────────────────────────────────────
    /// Start a local Axon runtime and (optionally) join a Hub.
    #[command(display_order = 1)]
    Start(start::StartArgs),
    /// Stop the locally started runtime (best-effort).
    #[command(display_order = 2)]
    Stop(stop::StopArgs),
    /// Show runtime / hub status and node summary.
    #[command(display_order = 3)]
    Status(status::StatusArgs),
    /// Connect to Hub as a paired device and optionally start MCP server.
    #[command(display_order = 4)]
    Connect(connect::ConnectArgs),

    // ── Federation queries ─────────────────────────────────────────────────
    /// List devices (nodes) known to the runtime / hub.
    #[command(display_order = 10)]
    Devices(devices::DevicesArgs),
    /// List abilities (MCP tools) across nodes.
    #[command(display_order = 11)]
    Abilities(abilities::AbilitiesArgs),

    // ── Remote operations ──────────────────────────────────────────────────
    /// One-shot remote command execution (ephemeral ability).
    #[command(display_order = 20)]
    Exec(exec::ExecArgs),
    /// Deploy an ability package/descriptor to a node.
    #[command(display_order = 21)]
    Deploy(deploy::DeployArgs),
    /// Invoke an ability on a node.
    #[command(display_order = 22)]
    Invoke(invoke::InvokeArgs),
    /// Compile and run EAL missions.
    #[command(display_order = 23)]
    Mission(mission::MissionArgs),

    // ── Device lifecycle (one-time setup) ──────────────────────────────────
    /// Pair this device with EasyNet using a pairing token.
    #[command(display_order = 30)]
    Join(join::JoinArgs),
    /// Show or set device settings (session_bridge exec enable).
    #[command(display_order = 31)]
    Config(config_cmd::ConfigArgs),
    /// Remove device credentials and un-pair.
    #[command(display_order = 32)]
    Reset(reset::ResetArgs),

    // ── AI integration ─────────────────────────────────────────────────────
    /// Run a Hub-level MCP server on stdio.
    #[command(display_order = 40)]
    McpServer(mcp_server::McpServerArgs),
    /// Install MCP server config for Claude Code / Codex.
    #[command(display_order = 41)]
    McpInstall(mcp_install::McpInstallArgs),
    /// Register, manage, and invoke AI agents (Claude Code, Codex).
    #[command(display_order = 42)]
    Agent(agent::AgentArgs),
    /// Orchestrate multi-agent discussions.
    #[command(display_order = 43)]
    Discuss(discuss::DiscussArgs),
    /// Install EasyNet skill templates for Claude Code / Codex.
    #[command(display_order = 44)]
    SkillInstall(skill_install::SkillInstallArgs),
    /// Autonomous agent loop: goal → generate EAL → execute → observe → repeat.
    #[command(display_order = 45)]
    Think(think::ThinkArgs),

    // ── Internal ───────────────────────────────────────────────────────────
    /// Internal heartbeat daemon process (not for direct use).
    #[command(name = "_heartbeat-daemon", hide = true)]
    HeartbeatDaemon,
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
        Command::McpInstall(args) => mcp_install::run(args),
        Command::Agent(args) => agent::run(args),
        Command::Discuss(args) => discuss::run(args),
        Command::SkillInstall(args) => skill_install::run(args),
        Command::Think(args) => think::run(args),
        Command::HeartbeatDaemon => heartbeat::run_daemon(),
    }
}
