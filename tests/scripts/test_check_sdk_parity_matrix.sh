#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-sdk-parity-matrix.sh"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

if bash "$CHECK" >"$tmp/no-results.out" 2>"$tmp/no-results.err"; then
    echo "check-sdk-parity-matrix accepted missing live results" >&2
    exit 1
fi
grep -Fq "live_results_required" "$tmp/no-results.err"

bash "$CHECK" --self-test >/dev/null

printf 'test_check_sdk_parity_matrix ok\n'
