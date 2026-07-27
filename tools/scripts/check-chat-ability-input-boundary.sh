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

if rg -n 'driver\.resume_thread_id.*silently|silently.*driver\.resume_thread_id|tries to set `driver\.resume_thread_id` is silently' "$CHAT_RS"; then
  fail "agents.chat must reject driver.resume_thread_id instead of silently ignoring a second lifecycle surface"
fi

if ! rg -q 'parse_rejects_driver_resume_thread_id_as_second_lifecycle_surface' "$CHAT_RS"; then
  fail "agents.chat parser tests must reject driver.resume_thread_id as a second lifecycle surface"
fi

if ! rg -q 'parse_rejects_non_string_driver_model' "$CHAT_RS"; then
  fail "agents.chat parser tests must reject non-string driver.model instead of treating it as absent"
fi

for test_name in \
  parse_rejects_unknown_top_level_fields \
  parse_rejects_wrongly_typed_optional_string_fields \
  parse_rejects_wrongly_typed_stream_flag \
  parse_rejects_wrongly_typed_selection_mode \
  parse_rejects_unknown_selection_fields
do
  if ! rg -q "$test_name" "$CHAT_RS"; then
    fail "agents.chat parser tests must include $test_name"
  fi
done

if ! rg -q 'reject_unknown_fields' "$CHAT_RS"; then
  fail "agents.chat parser must reject unknown top-level fields in production code"
fi

if ! rg -q 'optional_selection_mode_field' "$CHAT_RS"; then
  fail "agents.chat selection parser must reject wrongly typed skills/context_loaders mode"
fi

for test_name in \
  parse_attachments_rejects_filename_on_path \
  parse_attachments_rejects_unknown_item_fields \
  parse_attachments_rejects_wrongly_typed_string_fields
do
  if ! rg -q "$test_name" "$CHAT_RS"; then
    fail "agents.chat attachment parser tests must include $test_name"
  fi
done

if ! rg -q 'optional_attachment_string_field' "$CHAT_RS"; then
  fail "agents.chat attachment parser must reject wrongly typed attachment string fields"
fi

echo "check-chat-ability-input-boundary: OK"
