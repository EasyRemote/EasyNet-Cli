#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

bash "$ROOT/tools/scripts/check-sdk-product-smokes.sh" --self-test >/dev/null

echo "test_check_sdk_product_smokes ok"
