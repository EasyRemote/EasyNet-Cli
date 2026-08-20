#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-voice-call-product-contract.sh"
BIN="${VOICE_CONTRACT_VERIFIER_BIN:-$REPO_ROOT/target/debug/verify-voice-contract}"

fail() { echo "FAIL: $*" >&2; exit 1; }

if [[ -z "${VOICE_CONTRACT_VERIFIER_BIN:-}" ]]; then
    cargo build --quiet --no-default-features --bin verify-voice-contract
fi
"$BIN" --root "$REPO_ROOT" --self-test >/dev/null \
    || fail "structured verifier self-tests failed"
VOICE_CONTRACT_VERIFIER_BIN="$BIN" bash "$SCRIPT" >/dev/null \
    || fail "repository Voice contract failed structured verification"

echo "test_check_voice_call_product_contract.sh: all cases passed"
