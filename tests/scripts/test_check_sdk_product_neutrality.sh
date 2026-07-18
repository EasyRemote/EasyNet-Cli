#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-sdk-product-neutrality.sh"
POLICY="$ROOT/sdk/conformance/edge_adapter_policy.py"
source "$ROOT/sdk/conformance/python_toolchain.sh"
source "$ROOT/sdk/conformance/toolchain_path.sh"
resolve_sdk_toolchain_path "$ROOT"
resolve_sdk_python_toolchain "$ROOT"
PYTHON_BIN="$SDK_CONFORMANCE_PYTHON"

"$PYTHON_BIN" "$POLICY" --self-test >/dev/null
bash "$CHECK" --self-test >/dev/null
bash "$CHECK" >/dev/null

echo "test_check_sdk_product_neutrality ok"
