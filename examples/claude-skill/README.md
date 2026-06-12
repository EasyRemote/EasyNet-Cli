# EasyNet Claude Code Skill — Device Control

End-to-end example of Claude Code / Codex controlling remote edge devices through
EasyNet, packaged as an Agent Skill with pre-built abilities.

## Architecture

```
┌─────────────────────────────────┐
│  Claude Code / Codex / Cursor   │
│  (reads SKILL.md, calls tools)  │
└────────────┬────────────────────┘
             │ MCP (stdio) or Bash (skill script)
┌────────────▼────────────────────┐
│  easynet mcp-server             │
│  (Hub-level MCP — 11 tools)     │
│  OR                             │
│  invoke.sh (skill script)       │
└────────────┬────────────────────┘
             │ DendriteBridge (gRPC)
┌────────────▼────────────────────┐
│  Axon Runtime (Hub)             │
│  - node registry                │
│  - capability lifecycle         │
│  - invocation dispatch          │
└────────────┬────────────────────┘
             │ gRPC
┌────────────▼────────────────────┐
│  Edge Device(s)                 │
│  - registers node via start/    │
│    connect                      │
│  - receives & executes abilities│
└──────────────────────────────────┘
```

## What This Example Demonstrates

1. **Device discovery** — Find online edge devices in the federation
2. **Ability deployment** — Deploy pre-built abilities (sysinfo, disk-usage, health-check) to devices
3. **Inline ability creation** — Generate and deploy abilities on-the-fly from shell commands
4. **Ability invocation** — Execute deployed abilities and get structured results
5. **One-shot execution** — Run arbitrary commands without deploying
6. **Multi-agent setup** — Two agents (Claude + Codex) each controlling different devices
7. **MCP server installation** — One-command setup for Claude Code / Codex integration

## Quick Start

### 1. Start the Runtime

```bash
easynet runtime start
```

### 2. (Optional) Start a Second Device

On another machine or terminal:

```bash
easynet runtime start --hub <hub_endpoint>
# or if already paired:
easynet connect
```

### 3a. Use via MCP Server (Recommended)

Install the MCP server for your agent:

```bash
# For Claude Code
easynet mcp-install claude

# For Codex
easynet mcp-install codex
```

Restart your agent, then talk naturally:

```
You: "What devices are online?"
You: "Deploy the sysinfo ability to device-01"
You: "Run sysinfo on device-01"
You: "Check disk usage on device-01"
You: "Execute 'uname -a' on device-01"
```

### 3b. Use via Skill Script

```bash
SKILL=examples/claude-skill/scripts/invoke.sh

# Discover devices
$SKILL discover

# Deploy a pre-built ability
$SKILL deploy device-01 examples/claude-skill/abilities/sysinfo

# Invoke it
$SKILL invoke device-01 sysinfo

# Deploy an inline ability
$SKILL deploy-quick device-01 get_time "date -u +%Y-%m-%dT%H:%M:%SZ"

# One-shot command (no deploy needed)
$SKILL exec device-01 uname -a

# List all abilities
$SKILL abilities device-01
```

## Multi-Agent Setup (Two Devices, Two Agents)

```bash
# Terminal 1: Start runtime + Hub
easynet runtime start

# Terminal 2: Start second device
easynet runtime start --hub http://127.0.0.1:50051

# Install MCP servers for each agent, bound to specific devices
easynet mcp-install claude --name easynet-a --bound-node device-a --agent claude
easynet mcp-install codex  --name easynet-b --bound-node device-b --agent codex
```

Now Claude Code sees only `device-a` tools, and Codex sees only `device-b` tools.

## Pre-built Abilities

| File | Ability | What It Does |
|------|---------|--------------|
| `abilities/sysinfo/` | `sysinfo` | Hostname, OS, arch, uptime |
| `abilities/disk-usage/` | `disk-usage` | Root filesystem usage |
| `abilities/health-check/` | `health-check` | Load average, free memory, timestamp |

### Creating Custom Abilities

Create a directory with an `ability.json`:

```json
{
  "name": "my-custom-ability",
  "version": "1.0.0",
  "tool_name": "my-custom-ability",
  "description": "What this ability does",
  "command": "echo '{\"result\": \"hello\"}'"
}
```

Then deploy:

```bash
easynet deploy ./my-ability-dir --to device-01
```

Or use `deploy-quick` for inline abilities:

```bash
$SKILL deploy-quick device-01 check_memory "free -m | head -2"
```

## Skill Contract

The `skill/SKILL.md` file is the Agent Skill contract, discoverable by:
- Claude Code (via `~/.claude/settings.json`)
- Codex (via `~/.codex/mcp_servers.json`)
- Cursor, Gemini CLI, and other MCP-compatible agents

## Files

```
claude-skill/
├── README.md              ← This file
├── skill/
│   └── SKILL.md           ← Agent Skill contract
├── scripts/
│   └── invoke.sh          ← CLI wrapper for all operations
└── abilities/
    ├── sysinfo/ability.json       ← System info ability
    ├── disk-usage/ability.json    ← Disk usage ability
    └── health-check/ability.json  ← Health check ability
```
