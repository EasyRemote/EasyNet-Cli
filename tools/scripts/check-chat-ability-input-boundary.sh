#!/usr/bin/env bash
#
# Guard agents.chat input parsing against retired compatibility vocabulary.
#
# The public chat ability input in this repository is the flat manifest-backed
# `prompt`/`context` JSON object. It must not be documented or tested as a
# legacy alias for a second canonical request model.

set -euo pipefail

ROOT="${CHECK_CHAT_ABILITY_INPUT_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

CHAT_RS="src/daemon/ability/builtins/agents/chat.rs"

[[ -f "$CHAT_RS" ]] || fail "required file missing: $CHAT_RS"

if rg -n 'parse_accepts_legacy_prompt|legacy_prompt_(only|and_context)|legacy[[:space:]_-]+prompt|prompt[[:space:]_-]+legacy|legacy[^[:cntrl:]]+chat[^[:cntrl:]]+input|chat[^[:cntrl:]]+input[^[:cntrl:]]+legacy' "$CHAT_RS"; then
  fail "agents.chat input boundary still describes canonical prompt/context payloads as legacy compatibility"
fi

if ! rg -q 'parse_accepts_canonical_minimal_prompt_args' "$CHAT_RS"; then
  fail "agents.chat parser tests must pin the canonical minimal prompt payload"
fi

if ! rg -q 'parse_accepts_canonical_prompt_and_context_args' "$CHAT_RS"; then
  fail "agents.chat parser tests must pin canonical prompt+context payloads"
fi

echo "check-chat-ability-input-boundary: OK"
