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
//   Pre-layered aliases (`easynet runtime start`, `easynet devices`, `easynet
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
pub(crate) mod ability_catalog_row;
pub(crate) mod ability_scaffold;
pub(crate) mod agent;
pub(crate) mod agent_new_ability;
pub(crate) mod agent_sessions;
pub(crate) mod auth;
pub(crate) mod bridge_lib;
pub(crate) mod completion;
pub(crate) mod config_cmd;
pub(crate) mod connect;
pub(crate) mod daemon_agent_view;
pub(crate) mod deploy;
pub(crate) mod devices;
pub(crate) mod discuss;
pub(crate) mod docker;
pub(crate) mod doctor;
pub(crate) mod exec;
#[cfg(feature = "axon-pb")]
pub(crate) mod federation_discover;
pub(crate) mod federation_gen_cert;
pub(crate) mod federation_peers;
pub(crate) mod federation_wire;
pub(crate) mod groups;
pub(crate) mod heartbeat;
pub mod presentation;

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
pub(crate) mod context;
pub(crate) mod pages;
/// #185 — `easynet quota` owner verb to inspect/edit the per-consumer
/// invocation quota policy (`[daemon.quota]`).
pub(crate) mod quota_cmd;
pub(crate) mod reset;
pub(crate) mod skill;
pub(crate) mod skill_install;
pub(crate) mod start;
pub(crate) mod start_boot_watcher;
pub(crate) mod status;
pub(crate) mod stop;
#[cfg(test)]
pub(crate) mod test_support;
pub(crate) mod think;

use clap::builder::styling::{AnsiColor, Effects, Style, Styles};
use clap::{Parser, Subcommand};

// ── Help styling ─────────────────────────────────────────────────────
//
// We hand-render the Commands block (clap 4 derive can't group
// subcommands), so we also hand-paint the ANSI colours for that
// block. `anstyle` is already a transitive dep of clap; we don't
// pull anything new.
//
// Two layers of styling, both managed here:
//
// 1. The clap-owned segments — `-h`/`--help` argument rows — get a
//    `Styles` value (`HELP_STYLES`) wired through `#[command(styles=…)]`
//    so they render in the same palette as the hand-painted block.
// 2. The hand-rendered headers/Commands block embeds raw ANSI
//    escapes inline in `HELP_TEMPLATE`. Using `header()` from clap's
//    `Styles` would only paint clap-generated headers — our
//    `Commands:` / group labels are part of the template literal,
//    so they need the inline escapes.
//
// `anstream` (also a clap transitive) auto-strips these on a non-TTY
// stdout, so piping `easynet --help | cat` stays clean.

/// Palette wired into the clap-owned `--help` segments (option names,
/// placeholders). Three roles only:
///   - bold cyan      → headers + usage line (matches `banner::sgr::ACCENT`)
///   - bold default   → option literals (`-h`, `--help`)
///   - dim default    → placeholders (`<COMMAND>`)
/// Matches the banner module's palette so the whole `--help` reads
/// as one document with one accent colour.
const HELP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .literal(Style::new().effects(Effects::BOLD))
    .placeholder(Style::new().effects(Effects::DIMMED));

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
    about = "EasyNet — device management, remote execution, and real-time communication",
    // The struct doc comment above is internal context; suppress it from
    // `--help` so the user-facing summary stays the single `about` line.
    long_about = None,
    // Override the default `--help` template so the subcommand block
    // can be hand-grouped. Clap 4's derive macro does not support
    // per-variant `help_heading` on subcommands, so we replace the
    // auto-generated `{subcommands}` listing entirely with the static
    // grouped block in `HELP_TEMPLATE`.
    help_template = HELP_TEMPLATE,
    styles = HELP_STYLES,
    // `next_line_help = false` keeps each arg's description on the
    // same line as the flag (compact). Wrap behaviour itself
    // (term_width) is *not* set here — clap's derive macro
    // silently ignores `term_width` and `max_term_width`
    // attributes (they exist on `Command` but the derive parser
    // doesn't forward them). The width cap is therefore applied
    // via the builder API in `bin/easynet.rs::apply_help_layout`,
    // which walks the whole Command tree and calls `term_width()`
    // on every node. If clap_derive ever gains support for these
    // attributes, the apply_help_layout post-processing can move
    // back here.
    next_line_help = false,
)]
pub struct App {
    #[command(subcommand)]
    pub command: Command,
}

/// Custom `--help` template. clap interpolates `{usage}` and
/// `{options}` only here. `{about}` is intentionally omitted
/// because the banner module already prints the verbatim
/// homepage tagline; rendering `about` again would duplicate
/// the "what is easynet" line. `{subcommands}` is omitted
/// because the hand-grouped block below replaces it.
///
/// Group headers use the form `[GroupName]:` rather than a bare word
/// so neither human readers nor agents misread them as runnable
/// subcommand names. The ANSI escapes embedded here are stripped
/// automatically by `anstream` when stdout isn't a TTY (e.g. when
/// piping to `cat` or `less`).
///
/// Maintenance contract
/// --------------------
/// This block is hand-written. Adding, renaming, or removing a
/// top-level subcommand requires updating BOTH the `Command` enum
/// below AND this template. There is no auto-sync — that's the cost
/// of grouping in clap 4 derive. The unit test
/// `help_template_lists_every_command_in_enum` (bottom of this file)
/// parses this template and asserts every command listed here exists
/// in the `Command` enum (and vice versa) so a missing-update fails
/// CI rather than ships.
///
/// ANSI escape legend (embedded literally):
///   \x1b[1;36m  — bold cyan  (group headers, matches clap's header colour)
///   \x1b[1m     — bold       (command names)
///   \x1b[2m     — dim        (maintenance hint at bottom)
///   \x1b[0m     — reset
// `{about}` is intentionally NOT interpolated. The top-level
// banner (rendered by `bin/easynet.rs` before clap takes over)
// already prints the wordmark + verbatim homepage tagline +
// signature; printing `about` here too would be a duplicate
// "what is easynet" line. The string lives on the App as
// metadata for completion scripts and crates.io, but it does
// not appear in `--help`.
const HELP_TEMPLATE: &str = "\
\x1b[1;36mUsage:\x1b[0m {usage}

\x1b[1;36mCommands:\x1b[0m
  \x1b[1;36m[Identity]\x1b[0m
    \x1b[1mauth\x1b[0m                 Log in / out, mint device-pairing tokens

  \x1b[1;36m[Network]\x1b[0m
    \x1b[1mjoin\x1b[0m                 Pair THIS host with a Hub via a one-time token
    \x1b[1mdevice\x1b[0m               Manage remote devices — pair, list, exec, terminal
    \x1b[1magent\x1b[0m                Manage agents — network actors that expose abilities
    \x1b[1mability\x1b[0m              Manage abilities — deploy, invoke, list public endpoints
    \x1b[1mcall\x1b[0m                 Voice/video calls — create, join, leave conferences
    \x1b[1mmission\x1b[0m              Compile, run, and inspect EAL orchestration missions

  \x1b[1;36m[Content]\x1b[0m
    \x1b[1mskill\x1b[0m                Manage agent-owned skills (install, list, upgrade, remove)
    \x1b[1mpages\x1b[0m                Publish a folder of static bytes as a website
    \x1b[1mapi-key\x1b[0m              Mint / list / revoke OpenAI-compat API keys
    \x1b[1mllm-api\x1b[0m              One-shot OpenAI-compat chat completion
    \x1b[1mcontext\x1b[0m              Track clipboard history and map project folders

  \x1b[1;36m[Runtime]\x1b[0m
    \x1b[1mruntime\x1b[0m              Manage the local Axon runtime (start, stop, status)
    \x1b[1mstart\x1b[0m                Start the local Axon runtime as a background daemon
    \x1b[1mstop\x1b[0m                 Stop the local Axon runtime
    \x1b[1mplugin\x1b[0m               Manage daemon ability-extension plugin packages
    \x1b[1mmcp\x1b[0m                  MCP server — expose device abilities to AI assistants
    \x1b[1mfederation\x1b[0m           Inspect cross-hub peers and trusted hubs
    \x1b[1minvocation\x1b[0m           Audit invocation records, show one record, inspect traces

  \x1b[1;36m[Maintenance]\x1b[0m
    \x1b[1mself\x1b[0m                 Update, check version, or uninstall EasyNet CLI
    \x1b[1mdoctor\x1b[0m               Health check — runtime, bridge, agents, MCP connectivity
    \x1b[1mdocker\x1b[0m               Docker/container operator diagnostics
    \x1b[1mquota\x1b[0m                Inspect or edit the per-consumer invocation quota policy
    \x1b[1mcompletion\x1b[0m           Emit a shell completion script (bash/zsh/fish/powershell)
    \x1b[1mhelp\x1b[0m                 Print this message or the help of the given subcommand

\x1b[1;36mOptions:\x1b[0m
{options}

\x1b[2mRun 'easynet <command> --help' for command-specific help.\x1b[0m
";

// Subcommand ordering uses `display_order` to bucket commands by role
// (Identity 10–19, Network 20–29, Content 30–39, Runtime 40–49,
// Maintenance 50–59). Clap 4 derive does not support per-variant
// help_heading on subcommands, so `HELP_TEMPLATE` owns the grouped
// command listing and the sync test below keeps it aligned with this enum.
#[derive(Debug, Subcommand)]
pub enum Command {
    // ── Identity (10-19) ─────────────────────────────────────────────────
    /// Log in / out, mint device-pairing tokens.
    #[command(display_order = 10)]
    Auth(groups::auth::AuthArgs),

    // ── Network (20-29) ──────────────────────────────────────────────────
    // Top-level lifecycle shortcuts (join / start / stop). The layered
    // forms (`device join`, `runtime start`, `runtime stop`) remain the
    // canonical homes; these aliases forward to the same `JoinArgs` /
    // `StartArgs` / `StopArgs` types and the same `run` functions, so
    // there is no behavioural drift — clap parses one struct, the
    // dispatcher hands off to the same impl regardless of spelling.
    // `start` / `stop` live in the Runtime bucket below alongside
    // `runtime`.
    /// Pair THIS host with a Hub via a one-time token (alias of `device join`).
    #[command(display_order = 20)]
    Join(join::JoinArgs),

    /// Manage remote devices — pair, list, exec, terminal.
    #[command(display_order = 21)]
    Device(groups::device::DeviceArgs),

    /// Manage agents — network actors that expose abilities.
    #[command(display_order = 22)]
    Agent(groups::agent::AgentArgs),

    /// Manage abilities — deploy, invoke, list public endpoints.
    #[command(display_order = 23)]
    Ability(groups::ability::AbilityArgs),

    /// Voice/video calls — create, join, leave multi-party conferences.
    #[command(display_order = 24)]
    Call(groups::call::CallArgs),

    /// Compile, run, and inspect EAL orchestration missions.
    #[command(display_order = 25)]
    Mission(groups::mission::MissionArgs),

    // ── Content (30-39) ──────────────────────────────────────────────────
    /// Manage agent-owned skills (install, list, upgrade, remove).
    #[command(display_order = 30)]
    Skill(skill::SkillArgs),

    /// Publish a folder of static bytes as a website.
    #[command(display_order = 31)]
    Pages(pages::PagesArgs),
    /// Context surface: clipboard tracking + mapped project folders.
    #[command(display_order = 34)]
    Context(context::ContextArgs),

    /// Mint / list / revoke OpenAI-compat API keys.
    #[command(name = "api-key", display_order = 32)]
    ApiKey(api_key_cli::ApiKeyArgs),

    /// One-shot OpenAI-compat chat completion against any chat-base ability.
    // No backticks in `about`: some terminals (iTerm2, Warp)
    // auto-highlight backtick-fenced text with an inverted
    // background, which produces a visual banner across the
    // about line in subcommand `--help`. See the same fix on
    // Join / Start / Stop above.
    #[command(
        name = "llm-api",
        display_order = 33,
        about = "One-shot OpenAI-compat chat completion against any chat-base ability."
    )]
    LlmApi(llm_api::LlmApiArgs),

    // ── Runtime (40-49) ──────────────────────────────────────────────────
    /// Manage the local Axon runtime (start, stop, status).
    #[command(display_order = 40)]
    Runtime(groups::runtime::RuntimeArgs),

    /// Start the local Axon runtime as a background daemon (alias of `runtime start`).
    #[command(display_order = 41)]
    Start(start::StartArgs),

    /// Stop the local Axon runtime (alias of `runtime stop`).
    #[command(display_order = 42)]
    Stop(stop::StopArgs),

    /// Manage daemon ability-extension plugin packages.
    #[command(display_order = 43)]
    Plugin(groups::plugin::PluginArgs),

    /// MCP server — expose device abilities to AI assistants.
    #[command(display_order = 44)]
    Mcp(groups::mcp::McpArgs),

    /// Federation — inspect cross-hub peers and trusted hubs.
    #[command(display_order = 45)]
    Federation(groups::federation::FederationArgs),

    /// Invocation audit — list records, show one record, inspect traces.
    #[command(display_order = 46)]
    Invocation(groups::invocation::InvocationArgs),

    // ── Maintenance (50-59) ──────────────────────────────────────────────
    /// Update, check version, or uninstall EasyNet CLI.
    #[command(name = "self", display_order = 50)]
    SelfCmd(groups::selfcmd::SelfArgs),

    /// Health check — runtime, bridge, agents, MCP connectivity.
    #[command(display_order = 51)]
    Doctor(doctor::DoctorArgs),

    /// Docker/container operator diagnostics.
    #[command(display_order = 52)]
    Docker(docker::DockerArgs),

    /// Inspect or edit the per-consumer invocation quota policy.
    #[command(display_order = 53)]
    Quota(quota_cmd::QuotaArgs),

    /// Emit a shell completion script (bash/zsh/fish/powershell).
    #[command(display_order = 54)]
    Completion(completion::CompletionArgs),

    // ── Internal ─────────────────────────────────────────────────────────
    /// Internal heartbeat daemon process (not for direct use).
    #[command(name = "_heartbeat-daemon", hide = true)]
    HeartbeatDaemon,
}

pub fn run(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        // Top-level lifecycle shortcuts — forward to the same impls the
        // layered forms (`device join`, `runtime start`, `runtime stop`)
        // call. `start` mirrors the runtime group's banner render so the
        // two spellings produce identical output.
        Command::Join(args) => join::run(args),
        Command::Start(args) => {
            eprint!("{}", presentation::banner::render_logo());
            start::run(args)
        }
        Command::Stop(args) => stop::run(args),

        // Layered groups
        Command::Auth(args) => groups::auth::dispatch(args),
        Command::Agent(args) => groups::agent::run(args),
        Command::Ability(args) => groups::ability::run(args),
        Command::Device(args) => groups::device::run(args),
        Command::Mission(args) => groups::mission::run(args),
        Command::Skill(args) => skill::run(args),
        Command::Pages(args) => pages::run(args),
        Command::Context(args) => context::run(args),
        Command::ApiKey(args) => api_key_cli::run(args),
        Command::LlmApi(args) => llm_api::run(args),
        Command::Runtime(args) => groups::runtime::run(args),
        Command::Plugin(args) => groups::plugin::run(args),
        Command::Mcp(args) => groups::mcp::run(args),
        Command::Federation(args) => groups::federation::run(args),
        Command::Invocation(args) => groups::invocation::run(args),
        Command::Call(args) => groups::call::run(args),

        // Cross-cutting
        Command::SelfCmd(args) => groups::selfcmd::run(args),
        Command::Doctor(args) => doctor::run(args),
        Command::Docker(args) => docker::run(args),
        Command::Quota(args) => quota_cmd::run(args),
        Command::Completion(args) => completion::run::<App>(args),

        // Internal
        Command::HeartbeatDaemon => heartbeat::run_daemon(),
    }
}

#[cfg(test)]
mod help_template_sync_tests {
    //! Pin the contract that `HELP_TEMPLATE`'s hand-written grouped
    //! command list stays in sync with the `Command` enum. clap 4
    //! derive does not auto-generate the grouped block, so a missing
    //! entry here ships silently — except for this test.
    //!
    //! The test extracts every command name from the template by
    //! scanning lines whose leading-whitespace prefix matches a
    //! command row (4-space indent under a 2-space group heading)
    //! and compares it to the set of canonical names emitted by the
    //! `Command` enum. Mismatch in either direction = fail.
    use super::*;
    use clap::CommandFactory;
    use std::collections::BTreeSet;

    /// Strip ANSI escape sequences (`\x1b[...m`) from a line so the
    /// parser below can see the plain layout. We only need to handle
    /// the SGR escapes embedded in `HELP_TEMPLATE`; a tiny hand-rolled
    /// scanner avoids a regex dep.
    fn strip_ansi(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                // Skip ESC '[' …terminator (final byte 0x40..=0x7e).
                i += 2;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1; // skip the final byte too
                }
            } else {
                out.push(bytes[i] as char);
                i += 1;
            }
        }
        out
    }

    /// Names baked into the hand-written template under `Commands:`.
    /// Command rows are 4-space indented; group headers under
    /// `Commands:` are 2-space indented and wrapped in `[...]:` so
    /// they cannot be mistaken for command names.
    fn template_command_names() -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        let mut in_commands_block = false;
        for raw in HELP_TEMPLATE.lines() {
            let plain = strip_ansi(raw);
            let trimmed_end = plain.trim_end();
            if trimmed_end == "Commands:" {
                in_commands_block = true;
                continue;
            }
            if !in_commands_block {
                continue;
            }
            if trimmed_end == "Options:" {
                break;
            }
            // Command rows: exactly 4 leading spaces, first non-space
            // char is the command name. Skip group headers (2 leading
            // spaces, start with `[`) and blank lines.
            if !plain.starts_with("    ") {
                continue;
            }
            let after_indent = &plain[4..];
            if after_indent.starts_with(' ') {
                // 5+ spaces = continuation/wrap line; ignore.
                continue;
            }
            if let Some(first_word) = after_indent.split_whitespace().next() {
                if first_word.starts_with('[') {
                    continue;
                }
                names.insert(first_word.to_string());
            }
        }
        names
    }

    /// Names clap actually derives from the `Command` enum, minus
    /// hidden variants (HeartbeatDaemon) and minus the `help` row
    /// (which clap auto-injects into `--help`; the template lists
    /// it manually for visual consistency).
    fn enum_command_names() -> BTreeSet<String> {
        let app = App::command();
        app.get_subcommands()
            .filter(|c| !c.is_hide_set())
            .map(|c| c.get_name().to_string())
            .collect()
    }

    #[test]
    fn help_template_lists_every_command_in_enum() {
        let template = template_command_names();
        let enum_names = enum_command_names();

        // `help` is auto-injected by clap and not declared in the
        // enum, so it appears in the template only. Strip it before
        // comparing.
        let mut template_no_help = template.clone();
        template_no_help.remove("help");

        let missing_from_template: Vec<&String> =
            enum_names.difference(&template_no_help).collect();
        let extra_in_template: Vec<&String> = template_no_help.difference(&enum_names).collect();

        assert!(
            missing_from_template.is_empty() && extra_in_template.is_empty(),
            "HELP_TEMPLATE drifted from `Command` enum.\n  \
             missing from template: {:?}\n  \
             extra in template: {:?}\n\
             update HELP_TEMPLATE in src/facade/cli/mod.rs to match.",
            missing_from_template,
            extra_in_template,
        );
    }
}
