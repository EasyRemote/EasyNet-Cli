#!/usr/bin/env bash
# EasyNet CLI — Claude Code Skill Invoke Script
# ===============================================
#
# Wraps `easynet` CLI commands for agent-driven device control.
# Used by SKILL.md as the script interface for Claude Code / Codex / Cursor.
#
# Usage:
#   invoke.sh <action> [args...]
#
# Actions:
#   status                              — Show runtime status and online devices
#   discover                            — List online devices (JSON)
#   deploy <node_id> <ability_dir>      — Deploy ability from directory to device
#   deploy-quick <node_id> <name> <cmd> — Create and deploy an inline ability
#   invoke <node_id> <ability>          — Invoke a deployed ability
#   abilities [node_id]                 — List abilities (optionally for a specific node)
#   exec <node_id> <command>            — One-shot command execution on device
#   uninstall <node_id> <install_id>    — Uninstall ability from device
#   install-mcp <client> [options]      — Install MCP server config for claude/codex
#
# Environment:
#   EASYNET_ENDPOINT — Override runtime endpoint (auto-detected if omitted)
#   EASYNET_TENANT   — Override tenant ID
#
# Author: Silan Hu <silan.hu@u.nus.edu>
# Copyright (c) 2026 EasyNet. All rights reserved.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SKILL_DIR="$(dirname "$SCRIPT_DIR")"

# ── Ensure easynet is available ──────────────────────────────────────────────

if ! command -v easynet &>/dev/null; then
  echo '{"ok": false, "error": "easynet CLI not found. Install from: https://github.com/user/EasyNet-Cli"}' >&2
  exit 1
fi

# ── Optional overrides via env ───────────────────────────────────────────────

ENDPOINT_ARGS=()
if [ -n "${EASYNET_ENDPOINT:-}" ]; then
  # For mcp-server commands, pass endpoint; for CLI commands it auto-detects
  ENDPOINT_ARGS=(--endpoint "$EASYNET_ENDPOINT")
fi

TENANT_ARGS=()
if [ -n "${EASYNET_TENANT:-}" ]; then
  TENANT_ARGS=(--tenant "$EASYNET_TENANT")
fi

# ── Action dispatch ──────────────────────────────────────────────────────────

ACTION="${1:-help}"
shift || true

case "$ACTION" in

  status)
    easynet status --json 2>/dev/null || easynet status
    ;;

  discover)
    easynet devices --json 2>/dev/null || easynet devices
    ;;

  deploy)
    # deploy <node_id> <ability_dir>
    NODE_ID="${1:?usage: deploy <node_id> <ability_dir>}"
    ABILITY_DIR="${2:?usage: deploy <node_id> <ability_dir>}"
    shift 2

    if [ ! -d "$ABILITY_DIR" ]; then
      # If pointed at a file, use its parent directory
      if [ -f "$ABILITY_DIR" ]; then
        ABILITY_DIR="$(dirname "$ABILITY_DIR")"
      else
        echo "{\"ok\": false, \"error\": \"ability directory not found: $ABILITY_DIR\"}" >&2
        exit 1
      fi
    fi

    if [ ! -f "$ABILITY_DIR/ability.json" ]; then
      echo "{\"ok\": false, \"error\": \"no ability.json in $ABILITY_DIR\"}" >&2
      exit 1
    fi

    easynet deploy "$ABILITY_DIR" --to "$NODE_ID"
    ;;

  deploy-quick)
    # deploy-quick <node_id> <name> <command>
    # Creates a temporary ability.json and deploys it
    NODE_ID="${1:?usage: deploy-quick <node_id> <name> <command>}"
    ABILITY_NAME="${2:?usage: deploy-quick <node_id> <name> <command>}"
    COMMAND="${3:?usage: deploy-quick <node_id> <name> <command>}"
    shift 3

    TMPDIR_ABILITY="$(mktemp -d)"
    trap "rm -rf '$TMPDIR_ABILITY'" EXIT

    cat > "$TMPDIR_ABILITY/ability.json" <<EOF
{
  "name": "$ABILITY_NAME",
  "version": "1.0.0",
  "tool_name": "$ABILITY_NAME",
  "description": "Ability $ABILITY_NAME (deployed by agent)",
  "command": $(printf '%s' "$COMMAND" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))' 2>/dev/null || echo "\"$COMMAND\"")
}
EOF

    easynet deploy "$TMPDIR_ABILITY" --to "$NODE_ID"
    ;;

  invoke)
    # invoke <node_id> <ability> [--args JSON]
    NODE_ID="${1:?usage: invoke <node_id> <ability>}"
    ABILITY="${2:?usage: invoke <node_id> <ability>}"
    shift 2
    easynet invoke "$NODE_ID" "$ABILITY" "$@"
    ;;

  abilities)
    # abilities [node_id]
    if [ -n "${1:-}" ]; then
      easynet abilities --node "$1" --json 2>/dev/null || easynet abilities --node "$1"
    else
      easynet abilities --json 2>/dev/null || easynet abilities
    fi
    ;;

  exec)
    # exec <node_id> <command...>
    NODE_ID="${1:?usage: exec <node_id> <command>}"
    shift
    COMMAND="$*"
    easynet exec "$NODE_ID" -- $COMMAND
    ;;

  uninstall)
    # uninstall <node_id> <install_id>
    NODE_ID="${1:?usage: uninstall <node_id> <install_id>}"
    INSTALL_ID="${2:?usage: uninstall <node_id> <install_id>}"
    shift 2
    easynet invoke "$NODE_ID" __uninstall__ --args "{\"install_id\": \"$INSTALL_ID\"}"
    ;;

  install-mcp)
    # install-mcp <claude|codex> [extra args...]
    CLIENT="${1:?usage: install-mcp <claude|codex>}"
    shift
    easynet mcp-install "$CLIENT" "$@"
    ;;

  help|--help|-h)
    cat <<'HELP'
EasyNet Device Control — Agent Skill

Usage: invoke.sh <action> [args...]

Actions:
  status                                Show runtime status
  discover                              List online devices
  deploy <node_id> <ability_dir>        Deploy ability to device
  deploy-quick <node_id> <name> <cmd>   Deploy inline ability
  invoke <node_id> <ability>            Invoke deployed ability
  abilities [node_id]                   List abilities
  exec <node_id> <command>              One-shot remote command
  uninstall <node_id> <install_id>      Remove ability from device
  install-mcp <claude|codex> [opts]     Install MCP server config

Environment:
  EASYNET_ENDPOINT   Override runtime endpoint
  EASYNET_TENANT     Override tenant ID

Examples:
  invoke.sh discover
  invoke.sh deploy device-01 ./abilities/sysinfo
  invoke.sh deploy-quick device-01 get_time "date -u +%Y-%m-%dT%H:%M:%SZ"
  invoke.sh invoke device-01 sysinfo
  invoke.sh exec device-01 uname -a
  invoke.sh install-mcp claude --bound-node device-01 --agent claude
HELP
    ;;

  *)
    echo "{\"ok\": false, \"error\": \"unknown action: $ACTION. Run with 'help' for usage.\"}" >&2
    exit 1
    ;;
esac
