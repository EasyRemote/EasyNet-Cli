#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BACKEND_ROOT="${EASYNET_BACKEND_ROOT:-$ROOT/../EasyNet/backend}"

bash "$ROOT/tools/scripts/check-backend-sdk-only-boundary.sh" --self-test >/dev/null
bash "$ROOT/tools/scripts/check-backend-sdk-only-boundary.sh" "$BACKEND_ROOT" >/dev/null

echo "test_check_backend_sdk_only_boundary ok"
