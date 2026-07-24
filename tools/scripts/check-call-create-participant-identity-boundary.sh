#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

TARGET="src/cli/commands/groups/call.rs"

[[ -f "$TARGET" ]] || fail "missing $TARGET"

if ! rg -n 'enum CallCreateParticipantIdentity' "$TARGET" >/dev/null; then
  fail "call create participant identity must use an explicit state enum"
fi

if ! rg -n 'load_credentials_optional\(\)\?' "$TARGET" >/dev/null; then
  fail "call create participant identity must distinguish missing credentials with load_credentials_optional"
fi

if rg -n 'load_credentials\(\)|load_credentials\(\)\s*\.ok\(\)|map\(\|creds\|\s*creds\.node_id\)|credentials.*unwrap_or_else\(\|\|\s*gethostname::gethostname' "$TARGET"; then
  fail "call create must not collapse credential errors into hostname participant identity"
fi

if ! rg -n 'struct CallSignalingIssuer' "$TARGET" >/dev/null; then
  fail "call signaling must use a named issuer"
fi

if ! rg -n 'invoke_current_realm_hub_system_ability\(ability, args\.clone\(\)\)' "$TARGET" >/dev/null; then
  fail "call signaling must preserve the current-realm Hub route before local signaling"
fi

if ! rg -n -U 'LocalDaemonSystemAbilityIssuer::invoke_root_for_subject\(\s*ability,\s*args,\s*&subject_ura' "$TARGET" >/dev/null; then
  fail "local call signaling must bind an explicit local daemon subject"
fi

if rg -n '\binvoke_local_ability\s*\(' "$TARGET"; then
  fail "call signaling must not use generic invoke_local_ability"
fi

if ! rg -n 'call_create_participant_rejects_malformed_credentials' "$TARGET" >/dev/null; then
  fail "call create participant identity must test malformed credentials as fail-closed"
fi

if ! rg -n 'call_create_participant_rejects_incomplete_credentials' "$TARGET" >/dev/null; then
  fail "call create participant identity must test incomplete credentials as fail-closed"
fi

echo "check-call-create-participant-identity-boundary: ok"
