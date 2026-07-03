#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

TARGET="src/daemon/execution/mission/agent_ability_specs.rs"

[[ -f "$TARGET" ]] || fail "missing $TARGET"

if rg -n 'fn chat_ability\(|fn ensure_chat_manifest\(|agents_root\(\)\.join|falling back to synth|fallback.*synth|entry_without_root_path_falls_back|lazy migration|recreate chat\.ability' "$TARGET"; then
  fail "mission::agent_ability_specs dispatch path must be manifest-only; no retired synthetic chat/root fallback"
fi

if ! rg -n 'registry row is missing root_path' "$TARGET" >/dev/null; then
  fail "mission::agent_ability_specs must explicitly handle missing root_path"
fi

if ! rg -n 'belongs to agent' "$TARGET" >/dev/null; then
  fail "mission::agent_ability_specs must reject AgentDirectory name mismatches"
fi

if ! rg -n 'abilities_for_returns_empty_when_root_path_missing|entry_without_root_path_publishes_no_abilities' "$TARGET" >/dev/null; then
  fail "mission::agent_ability_specs must test that dispatch discovery stays manifest-only"
fi

if ! rg -n 'abilities_for_publication_synthesizes_default_chat_without_root_path' "$TARGET" >/dev/null; then
  fail "mission::agent_ability_specs must explicitly pin the publication read-model default-chat rule"
fi

echo "check-runtime-abilities-manifest-boundary: ok"
