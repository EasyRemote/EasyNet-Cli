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
    raise SystemExit("completion_audit: invalid_status_order")
if "product_boundary_rules" in matrix:
    raise SystemExit("completion_audit: product_boundary_rows_forbidden")

capabilities = matrix.get("capabilities")
if not isinstance(capabilities, list) or not capabilities:
    raise SystemExit("completion_audit: missing_capabilities")

failures: list[str] = []
for row in capabilities:
    capability = row.get("capability_id")
    if not isinstance(capability, str) or not capability:
        failures.append("invalid_capability")
        continue
    go = row.get("go")
    python = row.get("python")
    if not isinstance(go, dict) or not isinstance(python, dict):
        failures.append(f"{capability}:missing_language_state")
        continue
    go_status = go.get("status")
    python_status = python.get("status")
    if go_status not in status_order or python_status not in status_order:
        failures.append(f"{capability}:invalid_status")
        continue
    if go_status != python_status:
        failures.append(f"{capability}:language_state_mismatch")
    if go_status == "unsupported":
        failures.append(f"{capability}:unsupported_required_capability")

if failures:
    raise SystemExit("completion_audit: " + "; ".join(failures))
print("sdk completion matrix ok")
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  mkdir -p "$REPO_ROOT/target"
  tmp="$(mktemp -d "$REPO_ROOT/target/sdk-completion-audit.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  cp "$MATRIX" "$tmp/good.json"
  validate_matrix_completion "$tmp/good.json" >/dev/null

  python3 - "$MATRIX" "$tmp/unsupported.json" "$tmp/mismatch.json" "$tmp/product.json" <<'PY'
import json, sys
from pathlib import Path
source = json.loads(Path(sys.argv[1]).read_text())

value = json.loads(json.dumps(source))
value["capabilities"][0]["go"]["status"] = "unsupported"
value["capabilities"][0]["go"]["evidence"] = []
value["capabilities"][0]["python"]["status"] = "unsupported"
value["capabilities"][0]["python"]["evidence"] = []
Path(sys.argv[2]).write_text(json.dumps(value))

value = json.loads(json.dumps(source))
value["capabilities"][0]["python"]["status"] = "seam"
Path(sys.argv[3]).write_text(json.dumps(value))

value = json.loads(json.dumps(source))
value["product_boundary_rules"] = []
Path(sys.argv[4]).write_text(json.dumps(value))
PY

  for fixture in unsupported mismatch product; do
    if validate_matrix_completion "$tmp/$fixture.json" >"$tmp/$fixture.out" 2>&1; then
      echo "self-test expected $fixture fixture to fail" >&2
      exit 1
    fi
  done
  grep -Fq unsupported_required_capability "$tmp/unsupported.out"
  grep -Fq language_state_mismatch "$tmp/mismatch.out"
  grep -Fq product_boundary_rows_forbidden "$tmp/product.out"
  echo "check-sdk-completion-audit self-test ok"
  exit 0
fi

bash "$SELF_DIR/check-sdk-cutover-readiness.sh"
validate_matrix_completion "$MATRIX"
echo "SDK completion audit ok"
