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

if ! rg -n 'struct CallParticipantIdentity' "$TARGET" >/dev/null; then
  fail "call participant identity must use a named paired-device identity type"
fi

if ! rg -n 'load_credentials_optional\(\)\?' "$TARGET" >/dev/null; then
  fail "call create participant identity must distinguish missing credentials with load_credentials_optional"
fi

if ! rg -n 'resolve_paired_device' "$TARGET" >/dev/null; then
  fail "call participant identity must resolve from paired device credentials"
fi

if rg -n 'UnpairedHostname|from_unpaired_hostname|gethostname::gethostname|load_credentials\(\)\s*\.ok\(\)|map\(\|creds\|\s*creds\.node_id\)' "$TARGET"; then
  fail "call signaling must not collapse credential state into hostname participant identity"
fi

if ! rg -n 'requires paired device credentials' "$TARGET" >/dev/null; then
  fail "unpaired call signaling must fail closed with an explicit paired-device credential error"
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

if ! rg -n 'call_participant_rejects_unpaired_hostname_fallback' "$TARGET" >/dev/null; then
  fail "call participant identity must test that unpaired hostname fallback is retired"
fi

if ! rg -n 'call_participant_rejects_malformed_credentials' "$TARGET" >/dev/null; then
  fail "call create participant identity must test malformed credentials as fail-closed"
fi

if ! rg -n 'call_participant_rejects_incomplete_credentials' "$TARGET" >/dev/null; then
  fail "call create participant identity must test incomplete credentials as fail-closed"
fi

echo "check-call-create-participant-identity-boundary: ok"
