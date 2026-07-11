#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-sdk-parity-matrix.sh"

bash "$CHECK" >/dev/null
bash "$CHECK" --self-test >/dev/null

printf 'test_check_sdk_parity_matrix ok\n'
