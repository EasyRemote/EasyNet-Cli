// EasyNet CLI — Agent Group
// =========================
//
// File: src/cli/groups/agent.rs
// Description: `easynet agent …` — manage *agent instances*. Agents are
//              the only network first-class actors in EasyNet
//              (ARCHITECTURE.md §6, interpretation C). This group is the
//              CLI surface for working with them.
//
// Verbs:
//   add / list / remove / doctor   instance lifecycle      (-> cli::agent)
//   send <name> <prompt>           sugar for `agent.chat`  (-> cli::agent)
//   session new/list/show/append/end   memory dimension    (NEW)
//   discuss                        DEPRECATED → mission discuss
//   think                          DEPRECATED → mission think
//
// Why `agent send` is sugar:
//   `easynet agent send claude "hello"` desugars to a single-line
//   External EAL mission:
//
//       let r = claude.chat(prompt: "hello")
//       print(r)
//
//   `chat` is the agent's default callable (analogous to
//   `Object.toString()` in Java). See ARCHITECTURE.md §7.
//
// Why `session` lives here, not under `mission`:
//   Sessions are the simplest form of an agent's memory dimension
//   (per-caller conversation history). They are *private state* of an
//   agent instance from the network's point of view. The CLI exposes
//   them here because the calling client owns its own slice of that
//   memory and uses it across multiple `agent send` calls. See
//   ARCHITECTURE.md §10 (retention of agent_sessions.rs).
//
// Why `discuss` / `think` are deprecated aliases:
//   Both are *mission patterns*, not agent-instance methods:
//     - discuss = a multi-agent orchestration loop (calls many agents)
//     - think   = an iterative planning loop (calls one agent + executes)
//   Their primary location is now `easynet mission discuss` /
//   `easynet mission think`. The aliases here keep existing scripts
//   working but emit a deprecation notice.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};
use console::style;

use crate::facade::cli::{agent as agent_cmd, agent_sessions, discuss as discuss_cmd};
use crate::support::output;

#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub action: AgentAction,
}

#[derive(Debug, Subcommand)]
pub enum AgentAction {
    /// Register a new daemon-owned agent instance. Requires a paired,
    /// running local runtime because the daemon owns `agents.json`
    /// and LocalRuntime registration.
    Add(agent_cmd::AddArgs),
    /// Print daemon-owned registered agents: name, wrapper type, model,
    /// timeout. Requires a paired, running local runtime.
    List,
    /// Remove a daemon-owned registered agent instance. Requires a
    /// paired, running local runtime.
    Remove(agent_cmd::RemoveArgs),
    /// Remove registry rows whose on-disk root has gone missing.
    /// Pair with `easynet agent list`'s "path missing" column.
    Prune(agent_cmd::PruneArgs),
    /// Check whether an agent's underlying CLI is reachable.
    Doctor(agent_cmd::DoctorArgs),
    /// Sugar for a single-line EAL mission `agent.chat(prompt)`.
    Send(agent_cmd::SendArgs),
    /// Manage per-caller conversation sessions (memory dimension).
    Session(SessionArgs),
    /// List the abilities declared under `<agent-root>/abilities/`.
    Abilities(agent_cmd::AbilitiesArgs),
    /// Bind configured upstream MCP tools into this agent's
    /// `abilities/` directory as deterministic `[exec] kind="mcp"`
    /// abilities.
    Mcp(agent_cmd::McpArgs),
    /// Update fields of a daemon-owned registered agent in place.
    /// Currently supports `--model`. Requires a paired, running
    /// local runtime.
    Set(agent_cmd::SetArgs),
    /// Dry-run: show the `<agent>.<ability>` tools that a future live
    /// publish would register. No Axon calls, no mutation.
    Publish(agent_cmd::PublishArgs),
    /// Re-register every daemon-owned ability with axon-runtime.
    /// Requires a paired, running local runtime.
    /// Use after authoring a new `<agent>/abilities/<verb>.ability.toml`
    /// so the new ability is invokable cross-process without daemon
    /// restart. The in-daemon hot registrar materializes new TOMLs
    /// into the live runtime; this command propagates the same view
    /// to axon-runtime's `runtime_local_tools` registry. Pass
    /// `--agent <name>` to refresh only one row.
    Refresh(agent_cmd::RefreshArgs),
    /// Inspect this agent's persisted chat history (the JSONL log
    /// the --follow / --resume / --session-id flags on
    /// `agent send` read). Distinct from `agent session` (singular),
    /// which manages the per-caller memory dimension.
    #[command(name = "chat-history")]
    ChatHistory(agent_cmd::ChatHistoryArgs),
    /// DEPRECATED: use `easynet mission discuss`.
    Discuss(discuss_cmd::DiscussArgs),
    /// Agent-scoped object grammar:
    /// `easynet agent <agent-id-or-ura> new-ability ...`.
    #[command(external_subcommand)]
    Scoped(Vec<String>),
}

#[derive(Debug, Args)]
pub struct SessionArgs {
    #[command(subcommand)]
    pub action: SessionAction,
}

#[derive(Debug, Subcommand)]
pub enum SessionAction {
    /// Create a new session bound to an agent.
    New(SessionNewArgs),
    /// List sessions on this host.
    List,
    /// Show a session's full transcript.
    Show(SessionIdArgs),
    /// Append a turn to a session.
    Append(SessionAppendArgs),
    /// Delete a session.
    End(SessionIdArgs),
}

#[derive(Debug, Args)]
pub struct SessionNewArgs {
    /// Session id (a short user-chosen label).
    pub id: String,
    /// Target agent instance name.
    #[arg(long)]
    pub agent: String,
}

#[derive(Debug, Args)]
pub struct SessionIdArgs {
    /// Session id.
    pub id: String,
}

#[derive(Debug, Args)]
pub struct SessionAppendArgs {
    /// Session id.
    pub id: String,
    /// Role of the turn ("user" or "assistant").
    #[arg(long, default_value = "user")]
    pub role: String,
    /// Turn content (free text).
    pub content: String,
}

pub fn run(args: AgentArgs) -> anyhow::Result<()> {
    match args.action {
        AgentAction::Add(a) => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::Add(a),
        }),
        AgentAction::List => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::List,
        }),
        AgentAction::Remove(a) => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::Remove(a),
        }),
        AgentAction::Prune(a) => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::Prune(a),
        }),
        AgentAction::Doctor(a) => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::Doctor(a),
        }),
        AgentAction::Send(a) => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::Send(a),
        }),
        AgentAction::Abilities(a) => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::Abilities(a),
        }),
        AgentAction::Mcp(a) => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::Mcp(a),
        }),
        AgentAction::Set(a) => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::Set(a),
        }),
        AgentAction::Refresh(a) => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::Refresh(a),
        }),
        AgentAction::Publish(a) => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::Publish(a),
        }),
        AgentAction::Session(s) => run_session(s),
        AgentAction::ChatHistory(a) => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::ChatHistory(a),
        }),
        AgentAction::Discuss(a) => {
            eprintln!(
                "  {} {}",
                style("deprecated:").yellow(),
                style("`easynet agent discuss` → use `easynet mission discuss`").dim()
            );
            discuss_cmd::run(a)
        } // The pre-rewrite `easynet agent think` deprecated alias was
        // removed alongside `easynet mission think` and the
        // `mission.think` ability: modern agent runtimes already do
        // think-act-observe inside `<agent>.chat`, so the outer loop
        // was redundant.
        AgentAction::Scoped(tokens) => agent_cmd::run(agent_cmd::AgentArgs {
            action: agent_cmd::AgentAction::Scoped(tokens),
        }),
    }
}

fn run_session(args: SessionArgs) -> anyhow::Result<()> {
    match args.action {
        SessionAction::New(a) => {
            let session = agent_sessions::Session::new(a.id.clone(), a.agent.clone())?;
            session.save()?;
            output::success(&format!(
                "created session '{}' for agent '{}'",
                a.id, a.agent
            ));
            Ok(())
        }
        SessionAction::List => {
            let sessions = agent_sessions::list_sessions()?;
            if sessions.is_empty() {
                output::info("No sessions on this host.");
                return Ok(());
            }
            let mut table = output::table(&["ID", "Agent", "Turns", "Updated"]);
            for s in &sessions {
                let turns = s.turns.len().to_string();
                table.add_row(vec![
                    s.id.as_str(),
                    s.agent.as_str(),
                    turns.as_str(),
                    s.updated_at.as_str(),
                ]);
            }
            println!("{table}");
            Ok(())
        }
        SessionAction::Show(a) => {
            let session = agent_sessions::Session::load(&a.id)?;
            eprintln!();
            eprintln!(
                "  {} {}  {}",
                style("session").dim(),
                style(&session.id).bold(),
                style(format!("→ {}", session.agent)).dim()
            );
            output::detail("created", &session.created_at);
            output::detail("updated", &session.updated_at);
            output::detail("turns", &session.turns.len().to_string());
            eprintln!();
            print!("{}", session.transcript());
            Ok(())
        }
        SessionAction::Append(a) => {
            let mut session = agent_sessions::Session::load(&a.id)?;
            session.append(&a.role, &a.content);
            session.save()?;
            output::success(&format!("appended turn to '{}'", a.id));
            Ok(())
        }
        SessionAction::End(a) => {
            agent_sessions::delete_session(&a.id)?;
            output::success(&format!("ended session '{}'", a.id));
            Ok(())
        }
    }
}
