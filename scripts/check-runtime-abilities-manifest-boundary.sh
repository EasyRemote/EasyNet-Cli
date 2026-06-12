#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

if rg -n 'default_chat_manifest|fn chat_ability\(|fn ensure_chat_manifest\(|agents_root\(\)\.join|falling back to synth|fallback.*synth|entry_without_root_path_falls_back|lazy migration|recreate chat\.ability' src/runtime/abilities.rs; then
  fail "runtime::abilities must be manifest-only; no synthetic chat or root fallback"
fi

if ! rg -n 'registry row is missing root_path' src/runtime/abilities.rs >/dev/null; then
  fail "runtime::abilities must explicitly handle missing root_path"
fi

if ! rg -n 'belongs to agent' src/runtime/abilities.rs >/dev/null; then
  fail "runtime::abilities must reject AgentDirectory name mismatches"
fi

echo "check-runtime-abilities-manifest-boundary: ok"
