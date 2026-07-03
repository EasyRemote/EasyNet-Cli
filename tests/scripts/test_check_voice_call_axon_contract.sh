#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-voice-call-axon-contract.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-voice-call-axon-contract.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

descriptor_path() {
    local root="$1"
    local ability="$2"
    local paths count
    paths="$(find "$root/ability-descriptors/system" -type f -name "${ability}.ability.toml" -print | sort)"
    count="$(printf '%s\n' "$paths" | sed '/^$/d' | wc -l | tr -d ' ')"
    [[ "$count" == "1" ]] || fail "expected exactly one descriptor for $ability, found $count"
    printf '%s\n' "$paths"
}

copy_descriptor() {
    local sandbox="$1"
    local ability="$2"
    local source rel
    source="$(descriptor_path "$REPO_ROOT" "$ability")"
    rel="${source#$REPO_ROOT/}"
    mkdir -p "$sandbox/$(dirname "$rel")"
    cp "$source" "$sandbox/$rel"
}

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/daemon/ability/builtins/resources" "$sandbox/src/daemon/ability/builtins" "$sandbox/ability-descriptors/system"
    cp "$REPO_ROOT/src/daemon/ability/builtins/resources/voice.rs" "$sandbox/src/daemon/ability/builtins/resources/voice.rs"
    cp "$REPO_ROOT/src/daemon/ability/builtins/real_invoke_tests.rs" "$sandbox/src/daemon/ability/builtins/real_invoke_tests.rs"
    copy_descriptor "$sandbox" voice.join_call
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_VOICE_CALL_AXON_CONTRACT_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: voice Axon contract should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/"state_code": call\.state\.to_wire_i32\(\),/"state_proto": call.state.as_proto_name(),/' "$SB/src/daemon/ability/builtins/resources/voice.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "state_proto compatibility field should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/"end_reason_code": end_reason\.to_wire_i32\(\),/"end_reason_proto": end_reason.as_proto_name(),/' "$SB/src/daemon/ability/builtins/resources/voice.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "end_reason_proto compatibility field should exit 1 (got $rc)"

SB="$(make_sandbox)"
JOIN_TOML="$(descriptor_path "$SB" voice.join_call)"
perl -0pi -e 's/Response `state` carries the Axon/response `state` carries the legacy label; `state_proto` carries the Axon/' "$JOIN_TOML"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "legacy descriptor wording should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/struct VoiceCallService/struct RetiredVoiceCallService/' "$SB/src/daemon/ability/builtins/resources/voice.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing VoiceCallService owner should exit 1 (got $rc)"

SB="$(make_sandbox)"
{
    echo 'use std::sync::OnceLock;'
    echo 'fn store() {}'
	} >> "$SB/src/daemon/ability/builtins/resources/voice.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "global store compatibility path should exit 1 (got $rc)"

echo "test_check_voice_call_axon_contract.sh: all cases passed"
