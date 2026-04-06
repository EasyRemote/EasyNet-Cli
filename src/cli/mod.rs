// EasyNet CLI
// ===========
//
// File: src/cli/mod.rs
// Description: Command routing hub. As of the layered-CLI refactor, this
//              module exposes a *noun-first* set of top-level subcommands
//              (`device`, `ability`, `runtime`, `mcp`, `mission`, `agent`)
//              alongside a small set of cross-cutting tools (`doctor`,
//              `completion`).
//
// CLI Layering Intent — OOP view
// ===============================
//
// EasyNet's core abstraction is object-oriented: every Agent is an object
// on the network with public abilities (methods) and private skills
// (implementation). The CLI's top-level groups expose the public surface
// and deliberately hide the private one.
//
//   agent     → manage agent object instances (network actors)
//   ability   → manage public method endpoints (publish, list, invoke)
//   device    → manage hosting substrates (interpretation C: not
//                network first-class)
//   mission   → run External EAL (orchestration over public abilities)
//   runtime   → local Axon process lifecycle
//   mcp       → local MCP server process
//
// Two languages, two runtimes (see ontology spec §5)
// ---------------------------------------------------
//
// EasyNet has two independent language/runtime pairs, NOT a single
// compilation pipeline:
//
//   AAL  ──interpreted by──>  BCC      (producer side: define an agent class)
//   EAL  ──interpreted by──>  EasyNet  (consumer side: orchestrate abilities)
//
// This `easynet` binary IS the local CLI surface for the EAL runtime
// (= EasyNet itself). It does not yet contain BCC or any AAL machinery —
// BCC will arrive as a separate concern (likely a sibling binary). The
// bridge between the two runtimes is the gRPC ability endpoint: BCC
// exposes abilities, EasyNet calls them.
//
// Encapsulation invariant (load-bearing, see ontology spec §4)
// ------------------------------------------------------------
//
// No CLI command, no SDK call, and no EAL construct may reach across an
// agent boundary into a private skill. If a future need arises ("I want
// my agent to learn from another agent's skill"), the only valid answer
// is: the other agent must wrap that skill as an ability. Every CLI
// verb in this module is required to pass this encapsulation check
// before being added.
//
// Two top-level commands are deliberately ABSENT and must stay absent:
//
//   skill     → skills are private; exposing them breaks encapsulation.
//                Cross-agent reuse must go through wrapping a skill as
//                an ability.
//   capability → the cross-process invocation abstraction. When this
//                lands, it'll subsume `agent send` and `ability invoke`
//                into one verb. Until then, do NOT add a half-baked
//                `capability` command.
//
// Three time scales (do not conflate when adding commands):
//
//   - Schema/SLA changes: discrete, version-bumped, human-in-loop
//   - Graph evolution:    high-frequency, internal to the provider
//   - Per-call execution: realtime, generates Internal EAL each time
//
// See docs/easynet_ontology.pdf for the full ontology and the call
// walkthrough.
//
// Backwards compatibility:
//   Every old top-level verb (`devices`, `abilities`, `start`, `stop`,
//   `connect`, `status`, `join`, `reset`, `config`, `deploy`, `invoke`,
//   `exec`, `mcp-server`, `mcp-install`, `skill-install`, `think`,
//   `discuss`) is preserved as a *deprecated alias* — running it still
//   works, but a one-line stderr notice points the user at the new
//   layered command. The aliases will be removed in a future release.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod abilities;
pub mod agent;
pub mod agent_sessions;
pub mod completion;
pub mod config_cmd;
pub mod connect;
pub mod deploy;
pub mod devices;
pub mod discuss;
pub mod doctor;
pub mod exec;
pub mod groups;
pub mod heartbeat;
pub mod invoke;
pub mod join;
pub mod mcp_install;
pub mod mcp_server;
pub mod mission_runs;
pub mod reset;
pub mod skill_install;
pub mod start;
pub mod status;
pub mod stop;
pub mod think;

use clap::Subcommand;
use console::style;

#[derive(Debug, Subcommand)]
pub enum Command {
    // ── Layered, noun-first commands (the new public surface) ─────────────
    /// Manage agent instances (network actors).
    #[command(display_order = 1)]
    Agent(groups::agent::AgentArgs),

    /// Manage public ability endpoints.
    #[command(display_order = 2)]
    Ability(groups::ability::AbilityArgs),

    /// Manage hosting substrates (physical devices).
    #[command(display_order = 3)]
    Device(groups::device::DeviceArgs),

    /// Compile, run, and inspect EAL missions.
    #[command(display_order = 4)]
    Mission(groups::mission::MissionArgs),

    /// Manage the local Axon runtime.
    #[command(display_order = 5)]
    Runtime(groups::runtime::RuntimeArgs),

    /// Local MCP server process.
    #[command(display_order = 6)]
    Mcp(groups::mcp::McpArgs),

    // ── Cross-cutting tools ───────────────────────────────────────────────
    /// Aggregated health check across runtime / bridge / agents / MCP.
    #[command(display_order = 7)]
    Doctor(doctor::DoctorArgs),

    /// Generate a shell completion script (bash/zsh/fish/powershell/elvish).
    #[command(display_order = 8)]
    Completion(completion::CompletionArgs),

    // ── Deprecated flat aliases (kept until next release) ─────────────────
    #[command(hide = true)]
    Start(start::StartArgs),
    #[command(hide = true)]
    Stop(stop::StopArgs),
    #[command(hide = true)]
    Status(status::StatusArgs),
    #[command(hide = true)]
    Connect(connect::ConnectArgs),
    #[command(hide = true)]
    Devices(devices::DevicesArgs),
    #[command(hide = true)]
    Abilities(abilities::AbilitiesArgs),
    #[command(hide = true)]
    Exec(exec::ExecArgs),
    #[command(hide = true)]
    Deploy(deploy::DeployArgs),
    #[command(hide = true)]
    Invoke(invoke::InvokeArgs),
    #[command(hide = true)]
    Join(join::JoinArgs),
    #[command(hide = true, name = "config")]
    Config(config_cmd::ConfigArgs),
    #[command(hide = true)]
    Reset(reset::ResetArgs),
    #[command(hide = true, name = "mcp-server")]
    McpServer(mcp_server::McpServerArgs),
    #[command(hide = true, name = "mcp-install")]
    McpInstall(mcp_install::McpInstallArgs),
    #[command(hide = true, name = "skill-install")]
    SkillInstall(skill_install::SkillInstallArgs),
    #[command(hide = true)]
    Think(think::ThinkArgs),
    #[command(hide = true)]
    Discuss(discuss::DiscussArgs),

    // ── Internal ──────────────────────────────────────────────────────────
    /// Internal heartbeat daemon process (not for direct use).
    #[command(name = "_heartbeat-daemon", hide = true)]
    HeartbeatDaemon,
}

/// Print a one-line deprecation hint when an old flat alias is invoked.
fn deprecated(old: &str, new: &str) {
    eprintln!(
        "  {} `easynet {}` is deprecated — use `easynet {}` instead.",
        style("warning:").yellow().bold(),
        old,
        new,
    );
}

pub fn run(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        // Layered groups
        Command::Agent(args) => groups::agent::run(args),
        Command::Ability(args) => groups::ability::run(args),
        Command::Device(args) => groups::device::run(args),
        Command::Mission(args) => groups::mission::run(args),
        Command::Runtime(args) => groups::runtime::run(args),
        Command::Mcp(args) => groups::mcp::run(args),

        // Cross-cutting
        Command::Doctor(args) => doctor::run(args),
        Command::Completion(args) => completion::run::<crate::App>(args),

        // Deprecated flat aliases — print hint and forward.
        Command::Start(args) => {
            deprecated("start", "runtime start");
            start::run(args)
        }
        Command::Stop(args) => {
            deprecated("stop", "runtime stop");
            stop::run(args)
        }
        Command::Status(args) => {
            deprecated("status", "runtime status");
            status::run(args)
        }
        Command::Connect(args) => {
            deprecated("connect", "runtime connect");
            connect::run(args)
        }
        Command::Devices(args) => {
            deprecated("devices", "device list");
            devices::run(args)
        }
        Command::Abilities(args) => {
            deprecated("abilities", "ability list");
            abilities::run(args)
        }
        Command::Exec(args) => {
            deprecated("exec", "ability exec");
            exec::run(args)
        }
        Command::Deploy(args) => {
            deprecated("deploy", "ability deploy");
            deploy::run(args)
        }
        Command::Invoke(args) => {
            deprecated("invoke", "ability invoke");
            invoke::run(args)
        }
        Command::Join(args) => {
            deprecated("join", "device join");
            join::run(args)
        }
        Command::Config(args) => {
            deprecated("config", "device config");
            config_cmd::run(args)
        }
        Command::Reset(args) => {
            deprecated("reset", "device reset");
            reset::run(args)
        }
        Command::McpServer(args) => {
            deprecated("mcp-server", "mcp serve");
            mcp_server::run(args)
        }
        Command::McpInstall(args) => {
            deprecated("mcp-install", "mcp install");
            mcp_install::run(args)
        }
        Command::SkillInstall(args) => {
            deprecated("skill-install", "mcp skill-install");
            skill_install::run(args)
        }
        Command::Think(args) => {
            deprecated("think", "mission think");
            think::run(args)
        }
        Command::Discuss(args) => {
            deprecated("discuss", "mission discuss");
            discuss::run(args)
        }
        Command::HeartbeatDaemon => heartbeat::run_daemon(),
    }
}
