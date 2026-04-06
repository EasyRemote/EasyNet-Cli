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
    write_mcp_json(&ws)?;

    match entry.agent_type {
        AgentType::ClaudeCode => {} // .mcp.json + CLAUDE.md is enough
        AgentType::Codex | AgentType::CodexAppServer => {
            write_codex_config(&ws, entry)?;
            write_codex_skill(&ws)?;
        }
    }

    Ok(ws)
}

pub fn workspace_dir(agent_name: &str) -> PathBuf {
    config::state_dir().join("workspaces").join(agent_name)
}

// ── .mcp.json — Claude Code project-level MCP discovery ─────────────────────

fn write_mcp_json(ws: &std::path::Path) -> anyhow::Result<()> {
    let (cmd, args, env) = build_mcp_entry();

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

fn write_codex_config(ws: &std::path::Path, entry: &AgentEntry) -> anyhow::Result<()> {
    let codex_dir = ws.join(".codex");
    fs::create_dir_all(&codex_dir)?;

    let (cmd, args, env) = build_mcp_entry();
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

pub(super) fn build_mcp_entry() -> (String, Vec<String>, serde_json::Value) {
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

    if let Ok(lib) = std::env::var("EASYNET_DENDRITE_BRIDGE_LIB") {
        env.insert("EASYNET_DENDRITE_BRIDGE_LIB".to_string(), serde_json::json!(lib));
    }

    (cmd, args, serde_json::Value::Object(env))
}

fn generate_knowledge_doc() -> String {
    r#"# EasyNet Agent Workspace

You have access to EasyNet via MCP tools. You can manage edge devices and create abilities.

## Available MCP Tools

- `hub_status` — Check Hub connectivity
- `list_devices` — Discover online devices
- `deploy_ability` — Deploy ability to device (args: node_id, tool_name, command, description)
- `invoke_ability` — Call a deployed ability (args: node_id, ability)
- `execute_command` — One-shot remote command (args: node_id, command)
- `list_all_abilities` — List abilities on devices
- `run_mission` — Compile and execute an EAL program (args: eal_source)
- `uninstall_ability` — Remove ability from device

## Creating Abilities

Use the `deploy_ability` tool. The `command` field runs on the target device and should output JSON:

```json
{
  "node_id": "device-01",
  "tool_name": "health-check",
  "command": "python3 -c \"import json,os; print(json.dumps({'load': os.getloadavg()[0]}))\"",
  "description": "Check device health"
}
```

## Writing EAL Programs

EAL orchestrates abilities across devices and agents. Use `run_mission` with `eal_source`:

```eal
mission "my-workflow" {
  let data = call "collect" on "device-01" with {
    key = "value"
  } timeout 30

  let result = call "process" on "device-02" with {
    input = data.output
  }
}
```

Rules:
- Dependencies inferred from `var.output` references — compiler builds DAG automatically
- Independent steps run in parallel (same phase)
- Options: `timeout <secs>`, `retries <n>`, `on_failure abort|skip|retry|continue`, `optional`
- Agent targets (`on "claude"`, `on "codex"`) dispatch to AI agents

## CLI (via Bash)

```bash
easynet devices                    # list online devices
easynet deploy <dir> --to <node>   # deploy ability
easynet invoke <node> <ability>    # invoke ability
easynet exec <node> -- <command>   # one-shot remote command
easynet mission run <file.eal>     # run EAL program
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
