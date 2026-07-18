#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
MATRIX="$REPO_ROOT/sdk/conformance/sdk-parity-matrix.json"

validate_matrix_completion() {
  python3 - "$1" <<'PY'
import json, sys
from pathlib import Path
matrix = json.loads(Path(sys.argv[1]).read_text())
if matrix.get("schema_version") != 5:
    raise SystemExit("completion_audit: invalid_schema")
if matrix.get("status_order") != ["unsupported", "seam", "provider-backed", "cutover-ready"]:
    raise SystemExit("completion_audit: invalid_status_order")
if "product_boundary_rules" in matrix:
    raise SystemExit("completion_audit: product_boundary_rows_forbidden")
languages = ["rust", "c_abi", "go", "python", "node", "java", "swift"]
capabilities = matrix.get("capability_ids")
cells = matrix.get("cells")
if matrix.get("languages") != languages or not isinstance(capabilities, list) or not capabilities:
    raise SystemExit("completion_audit: invalid_universe")
keys = [(cell.get("capability_id"), cell.get("language")) for cell in cells or []]
expected = [(capability, language) for capability in capabilities for language in languages]
if keys != expected or len(keys) != len(set(keys)):
    raise SystemExit("completion_audit: missing_or_duplicate_cell")
for cell in cells:
    status = cell.get("status")
    evidence = cell.get("evidence_case_ids")
    shapes = cell.get("shape_evidence")
    step_shapes = cell.get("step_shape_evidence")
    proof = cell.get("provider_proof_ref")
    if not isinstance(evidence, list) or not isinstance(shapes, list) or not isinstance(step_shapes, list):
        raise SystemExit("completion_audit: invalid_cell_evidence")
    if status not in {"unsupported", "seam", "provider-backed", "cutover-ready"}:
        raise SystemExit("completion_audit: invalid_cell_status")
    if status == "unsupported" and (evidence or shapes or step_shapes or proof is not None):
        raise SystemExit("completion_audit: unsupported_claims_evidence")
    if status == "seam" and (
        proof is not None
        or (not evidence and not shapes)
        or (shapes and not step_shapes)
        or (step_shapes and not shapes)
    ):
        raise SystemExit("completion_audit: invalid_seam_evidence")
    if status in {"provider-backed", "cutover-ready"} and (not evidence or not shapes or not step_shapes or not proof):
        raise SystemExit("completion_audit: provider_proof_missing")
print("sdk completion matrix ok")
PY
}

if [[ "${1:-}" == "--matrix-only" ]]; then
  validate_matrix_completion "$MATRIX"
  exit 0
fi

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d "$REPO_ROOT/target/sdk-completion-audit.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  validate_matrix_completion "$MATRIX" >/dev/null
  python3 - "$MATRIX" "$tmp/missing.json" "$tmp/provider.json" "$tmp/product.json" "$tmp/seam.json" "$tmp/status.json" <<'PY'
import copy, json, sys
from pathlib import Path
source = json.loads(Path(sys.argv[1]).read_text())
missing = copy.deepcopy(source)
missing["cells"].pop()
Path(sys.argv[2]).write_text(json.dumps(missing))
provider = copy.deepcopy(source)
cell = next(cell for cell in provider["cells"] if cell["status"] == "seam")
cell["status"] = "provider-backed"
Path(sys.argv[3]).write_text(json.dumps(provider))
product = copy.deepcopy(source)
product["product_boundary_rules"] = []
Path(sys.argv[4]).write_text(json.dumps(product))
seam = copy.deepcopy(source)
cell = next(cell for cell in seam["cells"] if cell["status"] == "seam")
cell["evidence_case_ids"] = []
cell["shape_evidence"] = []
cell["step_shape_evidence"] = []
Path(sys.argv[5]).write_text(json.dumps(seam))
status = copy.deepcopy(source)
status["cells"][0]["status"] = "half-ready"
Path(sys.argv[6]).write_text(json.dumps(status))
PY
  for fixture in missing provider product seam status; do
    if validate_matrix_completion "$tmp/$fixture.json" >"$tmp/$fixture.out" 2>&1; then
      echo "self-test expected $fixture fixture to fail" >&2
      exit 1
    fi
  done
  grep -Fq missing_or_duplicate_cell "$tmp/missing.out"
  grep -Fq provider_proof_missing "$tmp/provider.out"
  grep -Fq product_boundary_rows_forbidden "$tmp/product.out"
  grep -Fq invalid_seam_evidence "$tmp/seam.out"
  grep -Fq invalid_cell_status "$tmp/status.out"
  echo "check-sdk-completion-audit self-test ok"
  exit 0
fi

bash "$SELF_DIR/check-sdk-cutover-readiness.sh"
validate_matrix_completion "$MATRIX"
echo "SDK completion audit ok"
