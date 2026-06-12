#!/usr/bin/env bash
#
# Guard voice call signaling responses to the Axon voice contract.

set -euo pipefail

ROOT="${CHECK_VOICE_CALL_AXON_CONTRACT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-voice-call-axon-contract: $*" >&2
    exit 1
}

VOICE_RS="src/runtime/agents/voice_call_ability.rs"
REAL_TESTS_RS="src/runtime/agents/real_invoke_tests.rs"
JOIN_TOML="abilities/system/voice.join_call.ability.toml"

for file in "$VOICE_RS" "$REAL_TESTS_RS" "$JOIN_TOML"; do
    [[ -f "$file" ]] || fail "missing $file"
done

grep -q '"state_code"' "$VOICE_RS" \
    || fail "voice responses must expose numeric Axon state_code"

grep -q '"end_reason_code"' "$VOICE_RS" \
    || fail "voice end responses must expose numeric Axon end_reason_code"

grep -q 'struct VoiceCallService' "$VOICE_RS" \
    || fail "voice signaling state must be owned by VoiceCallService"

grep -q 'let service = Arc::new(VoiceCallService::default())' "$VOICE_RS" \
    || fail "voice registry must register closures over one explicit service instance"

grep -q 'VOICE_CALL_STATE_ACTIVE' "$REAL_TESTS_RS" \
    || fail "real-invoke tests must assert the Axon state enum name"

grep -q 'Response `state` carries the Axon' "$JOIN_TOML" \
    || fail "generated join_call descriptor must document Axon state semantics"

bad="$(
    grep -nE 'state_proto|end_reason_proto|legacy_label\(|legacy label|back-compat|wire compatibility|"reason_code"|OnceLock|fn store\(\)|fn (create|show|join|leave|end|watch|report_metrics|list_calls)_handler' \
        "$VOICE_RS" "$REAL_TESTS_RS" "$JOIN_TOML" 2>/dev/null || true
)"
if [[ -n "$bad" ]]; then
    fail "voice call surface still carries retired compatibility fields:
$bad"
fi

echo "check-voice-call-axon-contract: ok"
