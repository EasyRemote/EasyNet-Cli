#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

bash "$ROOT/tools/scripts/check-canonical-runtime-convergence-v2.sh" --self-test-ura >/dev/null
bash "$ROOT/tools/scripts/check-canonical-runtime-convergence-v2.sh" --ura-only >/dev/null

echo "test_check_ura_vocabulary_contract ok"
