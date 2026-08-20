#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-canonical-runtime-convergence-v2.sh"

bash "$CHECK" --self-test >/dev/null

echo "test_check_canonical_runtime_convergence_v2 ok"
