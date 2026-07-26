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
MANIFEST_RS="src/daemon/ability/manifest.rs"

[[ -f "$CHAT_RS" ]] || fail "required file missing: $CHAT_RS"
[[ -f "$MANIFEST_RS" ]] || fail "required file missing: $MANIFEST_RS"

if rg -n 'parse_accepts_legacy_prompt|legacy_prompt_(only|and_context)|legacy[[:space:]_-]+prompt|prompt[[:space:]_-]+legacy|legacy[^[:cntrl:]]+chat[^[:cntrl:]]+input|chat[^[:cntrl:]]+input[^[:cntrl:]]+legacy' "$CHAT_RS" "$MANIFEST_RS"; then
  fail "agents.chat input boundary still describes canonical prompt/context payloads as legacy compatibility"
fi

if ! rg -q 'parse_accepts_canonical_minimal_prompt_args' "$CHAT_RS"; then
  fail "agents.chat parser tests must pin the canonical minimal prompt payload"
fi

if ! rg -Fq 'canonical minimal `{"prompt": "..."}` payload' "$MANIFEST_RS"; then
  fail "agents.chat manifest contract must describe prompt-only input as canonical minimal payload"
fi

if ! rg -q 'parse_accepts_canonical_prompt_and_context_args' "$CHAT_RS"; then
  fail "agents.chat parser tests must pin canonical prompt+context payloads"
fi

if ! rg -q 'enum ChatTurnSessionId' "$CHAT_RS"; then
  fail "agents.chat session id selection must use one explicit lifecycle selector"
fi

for token in \
  'ResumeRequested' \
  'DriverMinted' \
  'LocalResolved' \
  'chat_turn_session_id_prefers_resume_then_driver_then_local'
do
  if ! rg -q "$token" "$CHAT_RS"; then
    fail "agents.chat session id selector is missing $token"
  fi
done

if [[ "$(rg -c 'ChatTurnSessionId::select' "$CHAT_RS")" != "5" ]]; then
  fail "agents.chat RPC, stream, and precedence regression cases must share ChatTurnSessionId::select"
fi

if rg -n 'fall back to the local|locally-resolved id|session id we report|session-id fallback|session id fallback' "$CHAT_RS"; then
  fail "agents.chat session id lifecycle must not be described as fallback compatibility"
fi

echo "check-chat-ability-input-boundary: OK"
