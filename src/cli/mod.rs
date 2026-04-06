// EasyNet CLI
// ===========
//
// File: src/cli/mod.rs
// Description: Command routing hub. As of the layered-CLI refactor, this
//              module exposes a *noun-first* set of top-level subcommands
//              (`device`, `ability`, `runtime`, `mcp`, `mission`, `agent`)
//              alongside a small set of cross-cutting tools (`doctor`,
//              `logs`, `completion`).
//
// Layout:
//   groups/         — aggregated noun-first subcommand modules
//   doctor.rs       — `easynet doctor`
//   logs.rs         — `easynet logs`
//   completion.rs   — `easynet completion <shell>`
//   mission_runs.rs — on-disk EAL mission run history (used by groups::mission)
//   agent_sessions.rs — on-disk multi-turn agent session store (used by
//                       groups::agent)
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
pub mod logs;
pub mod mcp_install;
pub mod mcp_server;
pub mod mission_runs;
pub mod reset;
pub mod skill_install;
pub mod start;
pub mod status;
pub mod stop;
#[cfg(test)]
pub mod test_support;
pub mod think;

use clap::Subcommand;
use console::style;

#[derive(Debug, Subcommand)]
pub enum Command {
    // ── Layered, noun-first commands (the new public surface) ─────────────
    /// Manage federated devices (list/show/rename/tag/remove/join/reset/config).
    #[command(display_order = 1)]
    Device(groups::device::DeviceArgs),

    /// Manage abilities (list/show/deploy/update/uninstall/invoke/exec/logs).
    #[command(display_order = 2)]
    Ability(groups::ability::AbilityArgs),

    /// Manage the local Axon runtime (start/stop/status/connect/logs).
    #[command(display_order = 3)]
    Runtime(groups::runtime::RuntimeArgs),

    /// Compile, run, and inspect EAL missions.
    #[command(display_order = 4)]
    Mission(groups::mission::MissionArgs),

    /// Register and dispatch AI agents (Claude Code / Codex).
    #[command(display_order = 5)]
    Agent(groups::agent::AgentArgs),

    /// MCP server lifecycle and AI-client integration.
    #[command(display_order = 6)]
    Mcp(groups::mcp::McpArgs),

    // ── Cross-cutting tools ───────────────────────────────────────────────
    /// Aggregated health check across runtime / bridge / agents / MCP.
    #[command(display_order = 7)]
    Doctor(doctor::DoctorArgs),

    /// View logs across runtime / agent / mission subjects.
    #[command(display_order = 8)]
    Logs(logs::LogsArgs),

    /// Generate a shell completion script (bash/zsh/fish/powershell/elvish).
    #[command(display_order = 9)]
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
        Command::Device(args) => groups::device::run(args),
        Command::Ability(args) => groups::ability::run(args),
        Command::Runtime(args) => groups::runtime::run(args),
        Command::Mission(args) => groups::mission::run(args),
        Command::Agent(args) => groups::agent::run(args),
        Command::Mcp(args) => groups::mcp::run(args),

        // Cross-cutting
        Command::Doctor(args) => doctor::run(args),
        Command::Logs(args) => logs::run(args),
        Command::Completion(args) => completion::run(args),

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
            deprecated("think", "agent think");
            think::run(args)
        }
        Command::Discuss(args) => {
            deprecated("discuss", "agent discuss");
            discuss::run(args)
        }

        Command::HeartbeatDaemon => heartbeat::run_daemon(),
    }
}

#[cfg(test)]
mod tests {
    //! Clap parse-routing tests.
    //!
    //! These don't execute any handler — they just round-trip a CLI argv
    //! through `App::try_parse_from` and assert the resulting `Command`
    //! variant. This is the regression net for "did someone accidentally
    //! rename a subcommand" and the parallel-safe baseline for the layered
    //! command tree.

    use crate::App;
    use crate::cli::groups;
    use crate::cli::Command;
    use clap::Parser;

    fn parse(argv: &[&str]) -> Command {
        App::try_parse_from(argv).expect("parse").command
    }

    fn parse_err(argv: &[&str]) -> clap::Error {
        App::try_parse_from(argv).expect_err("expected parse error")
    }

    // ── top-level routing ────────────────────────────────────────────────

    #[test]
    fn top_level_groups_route_correctly() {
        assert!(matches!(parse(&["easynet", "device", "list"]), Command::Device(_)));
        assert!(matches!(parse(&["easynet", "ability", "list"]), Command::Ability(_)));
        assert!(matches!(parse(&["easynet", "runtime", "status"]), Command::Runtime(_)));
        assert!(matches!(parse(&["easynet", "mission", "list"]), Command::Mission(_)));
        assert!(matches!(parse(&["easynet", "agent", "list"]), Command::Agent(_)));
        assert!(matches!(parse(&["easynet", "mcp", "list"]), Command::Mcp(_)));
        assert!(matches!(parse(&["easynet", "doctor"]), Command::Doctor(_)));
        assert!(matches!(parse(&["easynet", "logs"]), Command::Logs(_)));
        assert!(matches!(
            parse(&["easynet", "completion", "bash"]),
            Command::Completion(_)
        ));
    }

    // ── device group ─────────────────────────────────────────────────────

    #[test]
    fn device_actions_parse() {
        let cmd = parse(&["easynet", "device", "show", "node-1"]);
        let Command::Device(d) = cmd else { panic!() };
        assert!(matches!(d.action, groups::device::DeviceAction::Show(_)));

        let cmd = parse(&["easynet", "device", "rename", "n", "New Name"]);
        let Command::Device(d) = cmd else { panic!() };
        assert!(matches!(d.action, groups::device::DeviceAction::Rename(_)));

        let cmd = parse(&["easynet", "device", "tag", "n", "--set", "k=v", "--set", "x=y"]);
        let Command::Device(d) = cmd else { panic!() };
        if let groups::device::DeviceAction::Tag(t) = d.action {
            assert_eq!(t.set, vec!["k=v".to_string(), "x=y".to_string()]);
        } else {
            panic!("expected Tag");
        }

        let cmd = parse(&["easynet", "device", "remove", "n", "--yes"]);
        let Command::Device(d) = cmd else { panic!() };
        if let groups::device::DeviceAction::Remove(r) = d.action {
            assert!(r.yes);
        } else {
            panic!("expected Remove");
        }
    }

    #[test]
    fn device_show_requires_node_id() {
        let err = parse_err(&["easynet", "device", "show"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    // ── ability group ────────────────────────────────────────────────────

    #[test]
    fn ability_actions_parse() {
        let cmd = parse(&["easynet", "ability", "show", "n", "tool"]);
        let Command::Ability(a) = cmd else { panic!() };
        assert!(matches!(a.action, groups::ability::AbilityAction::Show(_)));

        let cmd = parse(&["easynet", "ability", "uninstall", "n", "iid", "--yes"]);
        let Command::Ability(a) = cmd else { panic!() };
        if let groups::ability::AbilityAction::Uninstall(u) = a.action {
            assert!(u.yes);
            assert_eq!(u.install_id, "iid");
        } else {
            panic!("expected Uninstall");
        }

        let cmd = parse(&["easynet", "ability", "logs", "n", "tool", "--tail", "50"]);
        let Command::Ability(a) = cmd else { panic!() };
        if let groups::ability::AbilityAction::Logs(l) = a.action {
            assert_eq!(l.tail, 50);
        } else {
            panic!("expected Logs");
        }
    }

    // ── mission group ────────────────────────────────────────────────────

    #[test]
    fn mission_actions_parse() {
        let cmd = parse(&["easynet", "mission", "compile", "f.eal", "--emit-ir"]);
        let Command::Mission(m) = cmd else { panic!() };
        if let groups::mission::MissionAction::Compile(c) = m.action {
            assert!(c.emit_ir);
            assert_eq!(c.file, "f.eal");
        } else {
            panic!("expected Compile");
        }

        let cmd = parse(&["easynet", "mission", "list", "--limit", "5", "--json"]);
        let Command::Mission(m) = cmd else { panic!() };
        if let groups::mission::MissionAction::List(l) = m.action {
            assert_eq!(l.limit, 5);
            assert!(l.json);
        } else {
            panic!("expected List");
        }

        let cmd = parse(&["easynet", "mission", "show", "id-1", "--trace"]);
        let Command::Mission(m) = cmd else { panic!() };
        assert!(matches!(m.action, groups::mission::MissionAction::Show(_)));

        let cmd = parse(&["easynet", "mission", "cancel", "id-1"]);
        let Command::Mission(m) = cmd else { panic!() };
        assert!(matches!(m.action, groups::mission::MissionAction::Cancel(_)));
    }

    // ── agent group ──────────────────────────────────────────────────────

    #[test]
    fn agent_session_actions_parse() {
        let cmd = parse(&["easynet", "agent", "session", "new", "s1", "--agent", "claude"]);
        let Command::Agent(a) = cmd else { panic!() };
        if let groups::agent::AgentAction::Session(s) = a.action {
            if let groups::agent::SessionAction::New(n) = s.action {
                assert_eq!(n.id, "s1");
                assert_eq!(n.agent, "claude");
            } else {
                panic!("expected SessionAction::New");
            }
        } else {
            panic!("expected Session");
        }
    }

    #[test]
    fn agent_trace_actions_parse() {
        let cmd = parse(&["easynet", "agent", "trace", "list", "--agent", "claude", "--limit", "10"]);
        let Command::Agent(a) = cmd else { panic!() };
        if let groups::agent::AgentAction::Trace(t) = a.action {
            if let groups::agent::TraceAction::List(l) = t.action {
                assert_eq!(l.agent.as_deref(), Some("claude"));
                assert_eq!(l.limit, 10);
            } else {
                panic!("expected TraceAction::List");
            }
        } else {
            panic!("expected Trace");
        }

        let cmd = parse(&["easynet", "agent", "trace", "show", "claude/2026-04-06_141856", "--raw"]);
        let Command::Agent(a) = cmd else { panic!() };
        if let groups::agent::AgentAction::Trace(t) = a.action {
            if let groups::agent::TraceAction::Show(s) = t.action {
                assert_eq!(s.id, "claude/2026-04-06_141856");
                assert!(s.raw);
            } else {
                panic!("expected TraceAction::Show");
            }
        } else {
            panic!("expected Trace");
        }
    }

    // ── runtime group ────────────────────────────────────────────────────

    #[test]
    fn runtime_logs_parses_follow_and_tail() {
        let cmd = parse(&["easynet", "runtime", "logs", "--tail", "20", "--follow"]);
        let Command::Runtime(r) = cmd else { panic!() };
        if let groups::runtime::RuntimeAction::Logs(l) = r.action {
            assert_eq!(l.tail, 20);
            assert!(l.follow);
        } else {
            panic!("expected Logs");
        }
    }

    // ── deprecated aliases still parse ───────────────────────────────────

    #[test]
    fn deprecated_aliases_still_parse() {
        // Old flat names must keep working until they're removed in a
        // future release. We don't run the handler — just confirm the
        // parser still routes them.
        assert!(matches!(parse(&["easynet", "devices"]), Command::Devices(_)));
        assert!(matches!(parse(&["easynet", "abilities"]), Command::Abilities(_)));
        assert!(matches!(parse(&["easynet", "start"]), Command::Start(_)));
        assert!(matches!(parse(&["easynet", "stop"]), Command::Stop(_)));
        assert!(matches!(parse(&["easynet", "status"]), Command::Status(_)));
        assert!(matches!(parse(&["easynet", "join", "tok"]), Command::Join(_)));
        assert!(matches!(parse(&["easynet", "reset"]), Command::Reset(_)));
        assert!(matches!(parse(&["easynet", "mcp-server"]), Command::McpServer(_)));
        assert!(matches!(
            parse(&["easynet", "mcp-install", "claude"]),
            Command::McpInstall(_)
        ));
        assert!(matches!(parse(&["easynet", "skill-install"]), Command::SkillInstall(_)));
    }

    // ── unknown command surfaces a clap error ────────────────────────────

    #[test]
    fn unknown_subcommand_errors() {
        let err = parse_err(&["easynet", "absolutely-not-a-command"]);
        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }
}
