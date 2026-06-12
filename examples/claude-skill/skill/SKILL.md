---
name: easynet-device-control
description: Control remote edge devices through EasyNet — discover devices, deploy abilities, invoke abilities, list abilities, uninstall abilities, execute one-shot commands, and manage device connections. Supports multi-agent setups where each agent controls its own device.
compatibility: Requires easynet CLI installed and a running Axon runtime
metadata:
  author: easynet
  version: "1.0.0"
  axon-resource-uri: "easynet:///r/org/device-control"
allowed-tools: Bash(*)
---

# EasyNet Device Control

Control remote edge devices through EasyNet Axon Runtime using the `easynet` CLI.

## Prerequisites

The `easynet` CLI must be installed and a runtime must be running. To start a local runtime:

```bash
easynet runtime start
```

To check status:

```bash
easynet status
```

## Install MCP Server (Recommended)

For the best experience, install the MCP server so tools are available directly:

```bash
# For Claude Code
easynet mcp-install claude

# For Codex
easynet mcp-install codex

# For a specific device (device-bound mode)
easynet mcp-install claude --name easynet-device-a --bound-node device-a --agent claude

# For a second device controlled by a different agent
easynet mcp-install claude --name easynet-device-b --bound-node device-b --agent codex
```

After installation, restart the agent to pick up the new MCP server.

## Device Operations (via CLI)

### Discover online devices

```bash
easynet devices
```

### Deploy an ability to a device

Deploy a pre-built ability from a directory containing `ability.json`:

```bash
easynet deploy ${CLAUDE_SKILL_DIR}/abilities/sysinfo --to <node_id>
```

Or deploy any ability by creating a directory with an `ability.json`:

```bash
# ability.json format:
# {
#   "name": "my-ability",
#   "version": "1.0.0",
#   "tool_name": "my-ability",
#   "description": "What this ability does",
#   "command": "shell command to execute"
# }
easynet deploy /path/to/ability-dir --to <node_id>
```

### Invoke a deployed ability

```bash
easynet invoke <node_id> <ability_name>
```

### List abilities on a device

```bash
easynet abilities --node <node_id>
```

### Execute a one-shot command (no deploy needed)

```bash
easynet exec <node_id> -- <command>
```

Examples:
```bash
easynet exec device-01 -- hostname
easynet exec device-01 -- df -h /
easynet exec device-01 -- uname -a
```

### Uninstall an ability

```bash
# First find the install_id
easynet abilities --node <node_id>
# Then uninstall
easynet invoke <node_id> __uninstall__ --args '{"install_id": "<install_id>"}'
```

## Script Interface

All operations are also available through the invoke script:

```bash
${CLAUDE_SKILL_DIR}/scripts/invoke.sh <action> [args...]
```

### Available Actions

| Action | Usage | Description |
|--------|-------|-------------|
| `status` | `invoke.sh status` | Show runtime and device status |
| `discover` | `invoke.sh discover` | List online devices |
| `deploy` | `invoke.sh deploy <node_id> <ability_dir>` | Deploy ability to device |
| `deploy-quick` | `invoke.sh deploy-quick <node_id> <name> <command>` | Deploy inline ability |
| `invoke` | `invoke.sh invoke <node_id> <ability>` | Call deployed ability |
| `abilities` | `invoke.sh abilities [node_id]` | List abilities |
| `exec` | `invoke.sh exec <node_id> <command>` | One-shot command execution |
| `uninstall` | `invoke.sh uninstall <node_id> <install_id>` | Remove ability from device |

## Pre-built Abilities

The `abilities/` directory contains ready-to-deploy abilities:

| Ability | Description |
|---------|-------------|
| `sysinfo` | Collect hostname, OS, architecture, uptime |
| `disk-usage` | Report disk usage for root filesystem |
| `health-check` | Load average, memory, connectivity check |

Deploy example:
```bash
${CLAUDE_SKILL_DIR}/scripts/invoke.sh deploy <node_id> ${CLAUDE_SKILL_DIR}/abilities/sysinfo
```

## Multi-Agent Setup

To have two agents (e.g. Claude and Codex) each controlling different devices:

```bash
# Agent A controls device-a
easynet mcp-install claude --name easynet-a --bound-node device-a --agent agent-a

# Agent B controls device-b
easynet mcp-install codex --name easynet-b --bound-node device-b --agent agent-b
```

Each agent sees only its bound device's tools, preventing cross-device interference.

## Parameters

| Name | Type | Required | Description |
|------|------|----------|-------------|
| action | string | Yes | One of: status, discover, deploy, deploy-quick, invoke, abilities, exec, uninstall |
| node_id | string | No | Target device node ID (required for device operations) |
| ability_name | string | No | Ability name (required for invoke) |
| command | string | No | Shell command (required for deploy-quick, exec) |

## Axon Resource

- **URI**: `easynet:///r/org/device-control`
- **Version**: 1.0.0
