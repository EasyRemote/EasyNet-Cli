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
// Two top-level commands have a restricted shape (not absent):
//
//   skill     → PACKAGE MANAGEMENT ONLY. The `easynet skill
//                {install,list,upgrade,remove}` verbs install a
//                skill bundle from a marketplace (GitHub v1) into
//                a NAMED agent's `<agent-root>/skills/` directory.
//                They NEVER invoke a skill — invocation still goes
//                through the agent's public abilities, and
//                cross-agent skill reuse still requires the owning
//                agent to wrap the skill as an ability. The
//                encapsulation invariant in ontology §4 is about
//                invocation across an agent boundary; installing
//                your own agent's private dependencies does not
//                cross a boundary (the agent owns the skills/
//                directory; the operator owns the agent). An
//                earlier version of this comment forbade the
//                command outright — the restriction was over-
//                scoped and has been narrowed.
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
// No flat-command compatibility layer:
//   Pre-layered aliases (`easynet start`, `easynet devices`, `easynet
//   exec`, …) were removed before 1.0. The product never shipped under
//   those spellings, so there is no installed base to pay the carrying
//   cost of two parallel command surfaces. The individual modules
//   (`start.rs`, `stop.rs`, `connect.rs`, …) remain as *implementation
//   details*; they are called by `groups/runtime.rs`, `groups/device.rs`,
//   and their siblings.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub(crate) mod abilities;
pub(crate) mod ability_scaffold;
pub(crate) mod agent;
pub(crate) mod agent_sessions;
pub(crate) mod completion;
pub(crate) mod config_cmd;
pub(crate) mod connect;
pub(crate) mod deploy;
pub(crate) mod devices;
pub(crate) mod discuss;
pub(crate) mod doctor;
pub(crate) mod exec;
pub(crate) mod groups;
pub(crate) mod heartbeat;

/// Public re-export of the daemon entry point so the `easynet-daemon`
/// bin (in `src/bin/easynet-daemon.rs`) can call it without widening
/// the `heartbeat` module to `pub`. Keeping the module `pub(crate)`
/// is the correct default — only the daemon's main needs the entry
/// point outside the crate.
pub use heartbeat::run_daemon;
pub(crate) mod invoke;
pub(crate) mod join;
pub(crate) mod mcp_install;
pub(crate) mod mcp_server;
pub(crate) mod mission_runs;
pub(crate) mod reset;
pub(crate) mod skill;
pub(crate) mod skill_install;
pub(crate) mod start;
pub(crate) mod status;
pub(crate) mod stop;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod think;

use clap::{Parser, Subcommand};

/// Top-level clap App parser for the `easynet` user-facing CLI.
///
/// Lives in the library so that both `src/bin/easynet.rs` (user CLI)
/// and `facade::cli::completion::run::<App>(...)` can reference it
/// without the bin having to re-export it upward. v10.5 R1 moved it
/// here when the crate was split into library + multiple bins.
#[derive(Debug, Parser)]
#[command(
    name = "easynet",
    version,
    about = "EasyNet — device management, remote execution, and real-time communication"
)]
pub struct App {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    // ── Core commands ─────────────────────────────────────────────────────
    /// Manage remote devices — pair, list, exec, terminal.
    #[command(display_order = 1)]
    Device(groups::device::DeviceArgs),

    /// Manage agents — network actors that expose abilities.
    #[command(display_order = 2)]
    Agent(groups::agent::AgentArgs),

    /// Manage abilities — deploy, invoke, list public endpoints.
    #[command(display_order = 3)]
    Ability(groups::ability::AbilityArgs),

    /// Voice/video calls — create, join, leave multi-party conferences.
    #[command(display_order = 4)]
    Call(groups::call::CallArgs),

    /// Compile, run, and inspect EAL orchestration missions.
    #[command(display_order = 5)]
    Mission(groups::mission::MissionArgs),

    /// Manage agent-owned skills — install, list, upgrade, remove from a
    /// marketplace source (v1: GitHub). Skills are private packages the
    /// owning agent can wrap as an ability; never directly invocable.
    #[command(display_order = 6)]
    Skill(skill::SkillArgs),

    // ── Infrastructure ───────────────────────────────────────────────────
    /// Manage the local Axon runtime (start, stop, status).
    #[command(display_order = 6)]
    Runtime(groups::runtime::RuntimeArgs),

    /// MCP server — expose device abilities to AI assistants.
    #[command(display_order = 7)]
    Mcp(groups::mcp::McpArgs),

    // ── Tools ────────────────────────────────────────────────────────────
    /// Update, check version, or uninstall EasyNet CLI.
    #[command(display_order = 8, name = "self")]
    SelfCmd(groups::selfcmd::SelfArgs),

    /// Health check — runtime, bridge, agents, MCP connectivity.
    #[command(display_order = 9)]
    Doctor(doctor::DoctorArgs),

    /// Emit a shell completion script (bash/zsh/fish/powershell) to
    /// stdout. Pipe it into your shell's completion dir, e.g.
    /// `easynet completion zsh > ~/.zsh/completions/_easynet`.
    #[command(display_order = 10)]
    Completion(completion::CompletionArgs),

    // ── Internal ──────────────────────────────────────────────────────────
    //
    // Flat deprecated aliases (`easynet start`, `easynet devices`, …) were
    // removed before 1.0. The product never shipped under those spellings,
    // so there is no installed base to pay the carrying cost of dual
    // command surfaces: parallel help text, parallel completion, parallel
    // test coverage, and the live hazard of introducing a behavioural
    // drift between a layered command and its flat alias. Users who find
    // old docs pointing at `easynet start` will get a clean "command not
    // found" and the `--help` listing will direct them to the layered
    // form (`easynet runtime start`). The individual modules (`start.rs`,
    // `stop.rs`, …) remain as *implementation* — `groups/runtime.rs` and
    // its siblings call into them.
    /// Internal heartbeat daemon process (not for direct use).
    #[command(name = "_heartbeat-daemon", hide = true)]
    HeartbeatDaemon,
}

pub fn run(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        // Layered groups
        Command::Agent(args) => groups::agent::run(args),
        Command::Ability(args) => groups::ability::run(args),
        Command::Device(args) => groups::device::run(args),
        Command::Mission(args) => groups::mission::run(args),
        Command::Skill(args) => skill::run(args),
        Command::Runtime(args) => groups::runtime::run(args),
        Command::Mcp(args) => groups::mcp::run(args),
        Command::Call(args) => groups::call::run(args),

        // Cross-cutting
        Command::SelfCmd(args) => groups::selfcmd::run(args),
        Command::Doctor(args) => doctor::run(args),
        Command::Completion(args) => completion::run::<App>(args),

        // Internal
        Command::HeartbeatDaemon => heartbeat::run_daemon(),
    }
}
