#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
MATRIX="$REPO_ROOT/sdk/conformance/sdk-parity-matrix.json"

validate_matrix_completion() {
  local matrix_path="$1"
  python3 - "$matrix_path" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path


matrix = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
status_order = matrix.get("status_order")
if status_order != ["unsupported", "seam", "provider-backed", "cutover-ready"]:
    raise SystemExit("completion_audit: invalid status_order")
rank = {name: index for index, name in enumerate(status_order)}
required_minimum = rank["provider-backed"]

capabilities = matrix.get("capabilities")
if not isinstance(capabilities, list) or not capabilities:
    raise SystemExit("completion_audit: missing capabilities")

failures: list[str] = []
for row in capabilities:
    capability = row.get("capability_id")
    if not isinstance(capability, str) or not capability:
        failures.append("invalid capability row")
        continue
    for language in ("go", "python"):
        state = row.get(language)
        if not isinstance(state, dict):
            failures.append(f"{capability}:{language}:missing_state")
            continue
        status = state.get("status")
        if status not in rank:
            failures.append(f"{capability}:{language}:invalid_status:{status}")
            continue
        if rank[status] < required_minimum:
            failures.append(f"{capability}:{language}:below_provider_backed:{status}")

boundaries = matrix.get("product_boundary_rules")
if not isinstance(boundaries, list):
    failures.append("missing_product_boundary_rules")
else:
    found = {row.get("product_id") for row in boundaries if isinstance(row, dict)}
    for product in ("easynet_product", "easyremote_product"):
        if product not in found:
            failures.append(f"missing_product_boundary:{product}")

if failures:
    raise SystemExit("completion_audit: " + "; ".join(failures))

print("sdk completion matrix ok")
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d "$REPO_ROOT/target/sdk-completion-audit.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT

  cp "$MATRIX" "$tmp/good.json"
  validate_matrix_completion "$tmp/good.json" >/dev/null

  python3 - "$MATRIX" "$tmp/bad-status.json" "$tmp/bad-boundary.json" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path


source = Path(sys.argv[1])
bad_status = Path(sys.argv[2])
bad_boundary = Path(sys.argv[3])
matrix = json.loads(source.read_text(encoding="utf-8"))

status_fixture = json.loads(json.dumps(matrix))
status_fixture["capabilities"][0]["python"]["status"] = "seam"
bad_status.write_text(json.dumps(status_fixture), encoding="utf-8")

boundary_fixture = json.loads(json.dumps(matrix))
boundary_fixture["product_boundary_rules"] = [
    row
    for row in boundary_fixture["product_boundary_rules"]
    if row.get("product_id") != "easyremote_product"
]
bad_boundary.write_text(json.dumps(boundary_fixture), encoding="utf-8")
PY

  if validate_matrix_completion "$tmp/bad-status.json" >"$tmp/status.out" 2>&1; then
    echo "self-test expected below-provider-backed fixture to fail" >&2
    exit 1
  fi
  grep -Fq "below_provider_backed" "$tmp/status.out"

  if validate_matrix_completion "$tmp/bad-boundary.json" >"$tmp/boundary.out" 2>&1; then
    echo "self-test expected missing product boundary fixture to fail" >&2
    exit 1
  fi
  grep -Fq "missing_product_boundary:easyremote_product" "$tmp/boundary.out"

  echo "check-sdk-completion-audit self-test ok"
  exit 0
fi

bash "$SELF_DIR/check-sdk-cutover-readiness.sh"
validate_matrix_completion "$MATRIX"

echo "SDK completion audit ok"
