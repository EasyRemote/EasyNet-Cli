#!/usr/bin/env bash
# EasyNet Agent Discussion — Alive Topic
# =======================================
#
# Orchestrates a multi-agent discussion between Claude Code and Codex
# on the topic of agent-native networking and the Alive vision.
#
# Prerequisites:
#   - easynet CLI built and on PATH (or use cargo run --)
#   - claude CLI installed and authenticated
#   - codex CLI installed and authenticated
#
# Usage:
#   ./run.sh                          # 3 rounds, output to timestamped file
#   ./run.sh --rounds 2               # fewer rounds
#   ./run.sh --output my-article.md   # custom output path

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
EASYNET="${EASYNET_BIN:-easynet}"

# ── Register agents (idempotent) ─────────────────────────────────────────────

echo "Registering agents..."
$EASYNET agent add claude --type claude-code --model sonnet 2>/dev/null || true
$EASYNET agent add codex  --type codex --model gpt-5.2 2>/dev/null || true

echo ""
$EASYNET agent list
echo ""

# ── Doctor check ─────────────────────────────────────────────────────────────

echo "Checking agent availability..."
$EASYNET agent doctor || {
    echo "Warning: some agents may not be available."
    echo "Install missing CLIs and retry."
}
echo ""

# ── Parse arguments ──────────────────────────────────────────────────────────

ROUNDS=3
OUTPUT="alive-discussion-$(date +%Y%m%d-%H%M%S).md"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --rounds) ROUNDS="$2"; shift 2 ;;
        --output) OUTPUT="$2"; shift 2 ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# ── Run discussion ───────────────────────────────────────────────────────────

TOPIC="$(cat "$SCRIPT_DIR/topic-alive.txt")"

echo "Starting ${ROUNDS}-round discussion between Claude and Codex..."
echo "Output: $OUTPUT"
echo ""

$EASYNET discuss \
    --agents claude,codex \
    --rounds "$ROUNDS" \
    --topic "$TOPIC" \
    --output "$OUTPUT"

echo ""
echo "Article saved to: $OUTPUT"
echo "Run 'cat $OUTPUT' to view."
