#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-sdk-product-neutrality.sh"

bash "$CHECK" --self-test >/dev/null
bash "$CHECK" >/dev/null

echo "test_check_sdk_product_neutrality ok"
