// EasyNet CLI — Agent Workspace Provisioning
// =============================================
//
// File: src/agent/workspace.rs
// Description: Creates per-agent workspace directories with project-level
//              configuration for Claude Code and Codex.
//
// Claude Code discovers MCP servers from `.mcp.json` in project root and
// knowledge from `CLAUDE.md`. `-p` mode respects both.
//
// Codex discovers skills from `.agents/skills/` and config from `.codex/`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::PathBuf;

use crate::shared::agents::{AgentEntry, AgentType};
use crate::shared::config;

/// Ensure a workspace exists for the given agent and return its path.
pub fn ensure_workspace(agent_name: &str, entry: &AgentEntry) -> anyhow::Result<PathBuf> {
    let ws = workspace_dir(agent_name);
    fs::create_dir_all(&ws)?;

    // Codex requires a git repo.
    if !ws.join(".git").exists() {
        let _ = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&ws)
            .output();
    }

    // Write knowledge doc (shared between both agent types).
    fs::write(ws.join("CLAUDE.md"), generate_knowledge_doc())?;
    fs::write(ws.join("AGENTS.md"), generate_knowledge_doc())?;

    // Write .mcp.json — project-level MCP discovery for Claude Code (-p mode).
    write_mcp_json(&ws, agent_name)?;

    match entry.agent_type {
        AgentType::ClaudeCode => {} // .mcp.json + CLAUDE.md is enough
        AgentType::Codex | AgentType::CodexAppServer => {
            write_codex_config(&ws, entry, agent_name)?;
            write_codex_skill(&ws)?;
        }
    }

    Ok(ws)
}

pub fn workspace_dir(agent_name: &str) -> PathBuf {
    config::state_dir().join("workspaces").join(agent_name)
}

// ── .mcp.json — Claude Code project-level MCP discovery ─────────────────────

fn write_mcp_json(ws: &std::path::Path, agent_name: &str) -> anyhow::Result<()> {
    let (cmd, args, env) = build_mcp_entry(agent_name);

    let mcp_json = serde_json::json!({
        "mcpServers": {
            "easynet": {
                "command": cmd,
                "args": args,
                "env": env,
            }
        }
    });

    let json = serde_json::to_string_pretty(&mcp_json)? + "\n";
    fs::write(ws.join(".mcp.json"), json)?;
    Ok(())
}

// ── Codex workspace ──────────────────────────────────────────────────────────

fn write_codex_config(ws: &std::path::Path, entry: &AgentEntry, agent_name: &str) -> anyhow::Result<()> {
    let codex_dir = ws.join(".codex");
    fs::create_dir_all(&codex_dir)?;

    let (cmd, args, env) = build_mcp_entry(agent_name);
    let args_toml = args.iter()
        .map(|a| format!("\"{}\"", a))
        .collect::<Vec<_>>()
        .join(", ");

    let mut env_lines = String::new();
    if let serde_json::Value::Object(map) = &env {
        if !map.is_empty() {
            env_lines.push_str("\n[mcp_servers.easynet.env]\n");
            for (k, v) in map {
                if let Some(s) = v.as_str() {
                    env_lines.push_str(&format!("{k} = \"{s}\"\n"));
                }
            }
        }
    }

    let model_line = entry.model.as_deref()
        .map(|m| format!("model = \"{m}\"\n"))
        .unwrap_or_default();

    let toml = format!(
        "{model}\n[mcp_servers.easynet]\ncommand = \"{cmd}\"\nargs = [{args}]\n{env}",
        model = model_line,
        cmd = cmd,
        args = args_toml,
        env = env_lines,
    );

    fs::write(codex_dir.join("config.toml"), toml)?;
    Ok(())
}

/// Write Codex-native skill in .agents/skills/ for project-level discovery.
fn write_codex_skill(ws: &std::path::Path) -> anyhow::Result<()> {
    let skill_dir = ws.join(".agents").join("skills").join("easynet-ability-author");
    let agents_dir = skill_dir.join("agents");
    fs::create_dir_all(&agents_dir)?;

    fs::write(skill_dir.join("SKILL.md"), CODEX_SKILL_MD)?;
    fs::write(agents_dir.join("openai.yaml"), CODEX_OPENAI_YAML)?;
    Ok(())
}

// ── Shared ───────────────────────────────────────────────────────────────────

fn resolve_easynet_binary() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "easynet".to_string())
}

/// Build the (command, args, env) tuple for launching the EasyNet MCP
/// server as a subprocess of an agent. The launching agent's name is
/// threaded through so the MCP server knows which agent it belongs to,
/// for two purposes:
///
/// 1. The `--agent <name>` flag becomes the `from=` label in the
///    per-call audit line emitted by `mcp::handlers::send_to_agent`.
/// 2. `--enable-agent-dispatch` is set so the MCP server is allowed to
///    spawn other agents through the mission runtime. This is what
///    makes ontology §6.2 derivation 3 ("there is no second path") hold
///    *inside* an agent, not just at the CLI surface.
///
/// Defaulting `--enable-agent-dispatch` is a behaviour-semantics
/// change, not pure plumbing. The scoping rules that make it safe live
/// in `mcp::handlers::send_to_agent` (audit log, recursion guard,
/// future tenant check). The MCP server itself prints a one-line
/// stderr banner on startup so the operator sees that the workspace
/// MCP can spawn other agents.
pub(super) fn build_mcp_entry(agent_name: &str) -> (String, Vec<String>, serde_json::Value) {
    let cmd = resolve_easynet_binary();
    let mut args = vec!["mcp-server".to_string()];
    let mut env = serde_json::Map::new();

    if let Ok(state) = config::load() {
        if !state.endpoint.is_empty() {
            args.push("--endpoint".to_string());
            args.push(state.endpoint);
        }
        if let Some(t) = &state.tenant {
            args.push("--tenant".to_string());
            args.push(t.clone());
        }
    }

    // Identify the launching agent so the MCP server can label its
    // audit lines with `from=<agent_name>`.
    args.push("--agent".to_string());
    args.push(agent_name.to_string());

    // Allow the MCP server to dispatch back to other agents via the
    // mission runtime. Without this flag the workspace MCP can only
    // expose local Hub tools — agent-to-agent calls would silently fail.
    args.push("--enable-agent-dispatch".to_string());

    if let Ok(lib) = std::env::var("EASYNET_DENDRITE_BRIDGE_LIB") {
        env.insert("EASYNET_DENDRITE_BRIDGE_LIB".to_string(), serde_json::json!(lib));
    }

    (cmd, args, serde_json::Value::Object(env))
}

fn generate_knowledge_doc() -> String {
    r#"# EasyNet Agent Workspace

## What you are

You are an agent in **EasyNet**, an agent-native distributed system. EasyNet's
network has only two first-class objects:

- **Agent** — the addressable actor (you).
- **Ability** — a public method endpoint exposed by an agent.

You expose abilities. You do **not** expose skills. Skills are private to you
and are unreachable from outside. Other agents can call your abilities; they
cannot read your skills, your memory, or your internal state. The same rule
applies in reverse: you can call other agents' abilities, but you cannot
reach into their skills.

This is the **encapsulation invariant**: no CLI command, no SDK call, and no
EAL construct may reach across an agent boundary into a private skill. If
you find yourself wanting to "use another agent's skill", the only valid
form is: that agent must wrap the skill as an ability. See
`docs/easynet_ontology.tex` §4 for the full rule.

## How you call other agents

Cross-agent calls go through the **mission runtime**. There is no second
path. The shortest possible call is:

```eal
mission "ask-claude" {
  let r = claude.chat(prompt: "hello")
}
```

`chat` is the default callable on every agent — analogous to `__call__` in
Python or `Object.toString()` in Java. Custom abilities are called the
same way:

```eal
mission "review-code" {
  let r = claude.review(file: "x.rs", strict: true)
}
```

You can run a mission via the `run_mission` MCP tool, or — when calling
just the default `chat` ability — via the `send_to_agent` MCP tool, which
is the wire-level form of `<agent>.chat(<prompt>)` and desugars to a
single-line External EAL mission internally.

### What this means for you

Calling another agent triggers a **network execution chain**. When you
write `<other_agent>.chat(...)` or use the `send_to_agent` MCP tool, the
Hub spawns the target agent through the mission runtime. The target agent
may itself call other agents, up to depth 2. This is the only path by
which agents talk to each other in EasyNet, and every call is logged in
the MCP server's stderr stream as `[easynet mcp dispatch] from=... to=...
depth=... mission=...`.

Use this awareness when planning multi-step tasks: a single sentence like
*"ask the reviewer agent to look at this"* is potentially a multi-process,
multi-second operation, not a local function call. If you don't actually
need cross-agent coordination, do the work yourself.

## MCP tools available to you

When the workspace MCP server is running, the following tools are exposed.
Use them for federation queries, ability lifecycle, remote execution,
orchestration, and agent-to-agent dispatch.

### Federation queries (read-only)

- `hub_status()` — Hub connectivity, node and ability counts.
- `list_devices()` — List federation devices.
- `get_device_detail(node_id)` — Device info plus installed abilities.
- `list_all_abilities(node_id?, name_pattern?)` — Abilities across nodes.
- `search_abilities(query)` — Search abilities by name or tags.
- `list_a2a_agents(tags?, owner_id?, limit?)` — Remote A2A agents.
- `get_a2a_agent_card(node_id)` — A2A agent card.

### Ability lifecycle (mutating)

- `deploy_ability(node_id, tool_name, command, description?)` — Publish,
  install, and activate an ability on a device.
- `uninstall_ability(node_id, install_id)` — Remove an ability.

### Remote execution

- `execute_command(node_id, command)` — One-shot remote command on a
  device.
- `invoke_ability(node_id, ability, arguments?)` — Invoke an ability on
  a federated node.
- `send_a2a_task(target_agent_id, skill_id, input_json?, ...)` — Send an
  A2A task to a remote agent.

### Orchestration

- `run_mission(eal_source, emit_ir_only?)` — Compile and execute an EAL
  program. This is the highest-leverage tool — every multi-step
  cross-agent or cross-device interaction is best expressed as a mission.

### Device management

- `manage_device(node_id, action)` — Drain or disconnect a device.

### Agent dispatch (this workspace only, see banner)

- `send_to_agent(agent, prompt, context?)` — Wire-level form of
  `<agent>.chat(<prompt>)`. Desugars to a single-line External EAL
  mission internally. Use this when you need to talk to another agent
  and don't want to write the EAL yourself.

## What you can not do

- You can **not** access another agent's skills. Skills are private.
- You can **not** call methods that are not declared as abilities. Only
  the public surface is callable.
- You can **not** introspect another agent's internal graph. The agent
  is a black box from the network's point of view.
- You can **not** rename, delete, or directly modify abilities deployed
  by other tenants. The CLI surface deliberately omits commands like
  `easynet ability update` because they conflate three distinct time
  scales (schema bumps, graph evolution, per-call execution).

If a future need arises that you can't satisfy with the above rules, the
only valid resolution is for the *other* agent to expose what you need
as a new ability. You don't get a back door.

## Patterns

### Ask another agent something

```eal
mission "ask-codex" {
  let r = codex.chat(prompt: "explain quicksort in 3 lines")
}
```

Or via the MCP tool:

```json
{
  "name": "send_to_agent",
  "arguments": { "agent": "codex", "prompt": "explain quicksort in 3 lines" }
}
```

Both produce identical execution: a single-line External EAL mission
through the mission runtime.

### Compose two agents in a pipeline

```eal
mission "draft-and-review" {
  let draft = claude.draft(topic: "rate limiter design")
  let review = codex.review(input: draft.output)
}
```

The dependency on `draft.output` is inferred — `review` runs after
`draft` automatically. Independent steps would run in parallel.

### Avoid recursive loops

The recursion guard caps cross-agent dispatch at depth 2. If you find
yourself wanting to call yourself recursively, restructure the mission
into explicit phases instead — the planner will handle parallelism for
you.

## Quick CLI reference

```bash
easynet agent send <agent> "<prompt>"   # sugar for one-line mission
easynet mission run <file.eal>          # run a multi-step mission
easynet mission list                    # show recorded mission runs
easynet ability list                    # discover available abilities
easynet device list                     # list hosting substrates
```
"#.to_string()
}

const CODEX_SKILL_MD: &str = r#"---
name: easynet-ability-author
description: Author, deploy, and orchestrate EasyNet abilities and EAL missions. Use when asked to create abilities for edge devices, write EAL programs, or build multi-agent workflows.
---

# EasyNet Ability Author

Create and deploy abilities to edge devices via EasyNet MCP tools.

## Deploy an ability

Use the `deploy_ability` MCP tool:
- `node_id`: target device
- `tool_name`: ability name
- `command`: shell command (must output JSON)
- `description`: human-readable description

## Write EAL programs

Use the `run_mission` MCP tool with EAL source:

```eal
mission "name" {
  let a = call "ability" on "device" with { key = "value" } timeout 30
  let b = call "process" on "device-2" with { input = a.output }
}
```

Dependencies are inferred. Independent steps run in parallel.
Agent targets (`on "claude"`) dispatch to AI agents instead of devices.

## Available MCP tools

`list_devices`, `deploy_ability`, `invoke_ability`, `execute_command`,
`list_all_abilities`, `run_mission`, `uninstall_ability`, `hub_status`
"#;

const CODEX_OPENAI_YAML: &str = r#"interface:
  display_name: "EasyNet Ability Author"
  short_description: "Create and deploy abilities to edge devices via EasyNet"

policy:
  allow_implicit_invocation: true

dependencies:
  tools:
    - type: "mcp"
      value: "easynet"
      description: "EasyNet Hub MCP server for device management and ability deployment"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the workspace MCP entry includes the agent dispatch
    /// flag and the launching agent name. This is the load-bearing
    /// behaviour from Step 5 of the implementation plan: every agent
    /// workspace must launch its MCP server with `--enable-agent-dispatch`
    /// and `--agent <name>` so cross-agent dispatch is always available
    /// from inside an agent.
    #[test]
    fn build_mcp_entry_enables_agent_dispatch_with_name() {
        let (cmd, args, _env) = build_mcp_entry("claude");
        assert!(!cmd.is_empty(), "command must be set");
        assert!(
            args.iter().any(|a| a == "--enable-agent-dispatch"),
            "args must contain --enable-agent-dispatch, got: {args:?}"
        );
        // The agent name must be passed as `--agent <name>` (two adjacent
        // args, not a single `--agent=name`).
        let agent_idx = args.iter().position(|a| a == "--agent")
            .expect("args must contain --agent");
        assert_eq!(
            args.get(agent_idx + 1).map(|s| s.as_str()),
            Some("claude"),
            "--agent must be followed by the agent name"
        );
    }

    #[test]
    fn build_mcp_entry_threads_different_agent_names() {
        let (_, args_a, _) = build_mcp_entry("alice");
        let (_, args_b, _) = build_mcp_entry("bob");
        let agent_a = args_a.iter().position(|a| a == "--agent").map(|i| &args_a[i + 1]);
        let agent_b = args_b.iter().position(|a| a == "--agent").map(|i| &args_b[i + 1]);
        assert_eq!(agent_a.map(|s| s.as_str()), Some("alice"));
        assert_eq!(agent_b.map(|s| s.as_str()), Some("bob"));
    }
}
