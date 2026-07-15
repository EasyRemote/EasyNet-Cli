#!/usr/bin/env bash

set -euo pipefail

ROOT="${CHECK_VOICE_CALL_PRODUCT_CONTRACT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

if [[ -n "${VOICE_CONTRACT_VERIFIER_BIN:-}" ]]; then
    exec "$VOICE_CONTRACT_VERIFIER_BIN" --root "$ROOT"
fi

cd "$REPO_ROOT"
exec cargo run --quiet --no-default-features --bin verify-voice-contract -- --root "$ROOT"
