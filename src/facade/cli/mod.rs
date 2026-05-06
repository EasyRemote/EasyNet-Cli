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
pub(crate) mod auth;
pub(crate) mod completion;
pub(crate) mod config_cmd;
pub(crate) mod connect;
pub(crate) mod deploy;
pub(crate) mod devices;
pub(crate) mod discuss;
pub(crate) mod doctor;
pub(crate) mod exec;
#[cfg(feature = "axon-pb")]
pub(crate) mod federation_discover;
pub(crate) mod federation_gen_cert;
pub(crate) mod federation_peers;
pub(crate) mod federation_wire;
pub(crate) mod groups;
pub(crate) mod heartbeat;

/// Public re-export of the daemon entry point so the `easynet-daemon`
/// bin (in `src/bin/easynet-daemon.rs`) can call it without widening
/// the `heartbeat` module to `pub`. Keeping the module `pub(crate)`
/// is the correct default — only the daemon's main needs the entry
/// point outside the crate.
pub use heartbeat::run_daemon;
/// RFC-006-C v0.1 — `easynet api-key` for OpenAI-compat bearer
/// tokens, and `easynet llm-api` for chat-completion calls.
pub(crate) mod api_key_cli;
pub(crate) mod invoke;
pub(crate) mod join;
pub(crate) mod llm_api;
pub(crate) mod mcp_install;
pub(crate) mod mcp_server;
pub(crate) mod mission_runs;
/// RFC-006-B v0.6 — `easynet pages` ergonomic wrapper around
/// `<user>.pages.{publish,unpublish,list,get}` and the
/// `<user>.<project_id>.page.fetch` family.
pub(crate) mod pages;
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
    /// Log in to / out of an EasyNet backend, mint device-pairing
    /// tokens. Same model as `gh auth login` / `kubectl login` —
    /// JWT cached at ~/.easynet/auth.json (mode 0600), every later
    /// auth-aware command picks it up automatically.
    #[command(display_order = 0)]
    Auth(groups::auth::AuthArgs),

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

    /// Publish a folder of static bytes as a website (RFC-006-B v0.6).
    /// `easynet pages create papers --folder <path>` mints a project
    /// resource and serves it at `papers.<user>.pages.localhost:<port>/`.
    #[command(display_order = 6)]
    Pages(pages::PagesArgs),

    /// Mint / list / revoke OpenAI-compat API keys (RFC-006-C v0.1).
    #[command(name = "api-key", display_order = 6)]
    ApiKey(api_key_cli::ApiKeyArgs),

    /// One-shot OpenAI-compat chat completion against any chat-base
    /// ability registered on this daemon. `easynet llm-api "<prompt>"`
    /// (RFC-006-C v0.1).
    #[command(name = "llm-api", display_order = 6)]
    LlmApi(llm_api::LlmApiArgs),

    // ── Infrastructure ───────────────────────────────────────────────────
    /// Manage the local Axon runtime (start, stop, status).
    #[command(display_order = 6)]
    Runtime(groups::runtime::RuntimeArgs),

    /// MCP server — expose device abilities to AI assistants.
    #[command(display_order = 7)]
    Mcp(groups::mcp::McpArgs),

    /// Federation — inspect cross-hub federation peers and trusted
    /// hubs. Use `easynet federation peers` to list the URIs an
    /// operator can pass to `--node` on `easynet ability invoke`.
    #[command(display_order = 7)]
    Federation(groups::federation::FederationArgs),

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

    // ── Top-level shortcuts ──────────────────────────────────────────────
    //
    // The three most-frequent verbs (`join`, `start`, `stop`) get
    // top-level shortcuts so a freshly-installed user can run
    // `easynet join <token>` / `easynet start` / `easynet stop`
    // without remembering the layered form. They are *not* renames or
    // hidden aliases: the layered forms (`easynet device join`,
    // `easynet runtime start`, `easynet runtime stop`) keep working
    // unchanged. The shortcut variants forward to the same `JoinArgs`/
    // `StartArgs`/`StopArgs` types and the same `run` functions, so
    // there is no behavioural drift surface — Clap parses one struct,
    // the dispatcher hands off to the same impl regardless of which
    // path the user typed.
    /// Pair this device with a hub. Shortcut for `easynet device join`.
    #[command(display_order = 0)]
    Join(join::JoinArgs),

    /// Start the local Axon runtime as a background daemon.
    /// Shortcut for `easynet runtime start`.
    #[command(display_order = 0)]
    Start(start::StartArgs),

    /// Stop the local Axon runtime. Shortcut for `easynet runtime stop`.
    #[command(display_order = 0)]
    Stop(stop::StopArgs),

    // ── Internal ──────────────────────────────────────────────────────────
    /// Internal heartbeat daemon process (not for direct use).
    #[command(name = "_heartbeat-daemon", hide = true)]
    HeartbeatDaemon,
}

pub fn run(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        // Layered groups
        Command::Auth(args) => groups::auth::dispatch(args),
        Command::Agent(args) => groups::agent::run(args),
        Command::Ability(args) => groups::ability::run(args),
        Command::Device(args) => groups::device::run(args),
        Command::Mission(args) => groups::mission::run(args),
        Command::Skill(args) => skill::run(args),
        Command::Pages(args) => pages::run(args),
        Command::ApiKey(args) => api_key_cli::run(args),
        Command::LlmApi(args) => llm_api::run(args),
        Command::Runtime(args) => groups::runtime::run(args),
        Command::Mcp(args) => groups::mcp::run(args),
        Command::Federation(args) => groups::federation::run(args),
        Command::Call(args) => groups::call::run(args),

        // Cross-cutting
        Command::SelfCmd(args) => groups::selfcmd::run(args),
        Command::Doctor(args) => doctor::run(args),
        Command::Completion(args) => completion::run::<App>(args),

        // Top-level shortcuts — forward to the same impl the layered
        // forms call. No behaviour difference; only spelling.
        Command::Join(args) => join::run(args),
        Command::Start(args) => start::run(args),
        Command::Stop(args) => stop::run(args),

        // Internal
        Command::HeartbeatDaemon => heartbeat::run_daemon(),
    }
}
