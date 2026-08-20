#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-sdk-cutover-readiness.sh"

bash "$CHECK" --self-test >/dev/null

echo "test_check_sdk_cutover_readiness ok"
