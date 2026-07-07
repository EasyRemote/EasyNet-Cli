#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
DEFAULT_COVERAGE="$REPO_ROOT/sdk/conformance/spec-section27-coverage.json"

run_validator() {
  local coverage="$1"
  python3 - "$REPO_ROOT" "$coverage" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path


repo = Path(sys.argv[1]).resolve()
coverage_path = Path(sys.argv[2]).resolve()

SOURCE_SPEC = "docs/spec/daemon-sdk-requirements-v1.md#27"
REQUIRED_SPEC_CASES = (
    "version/abi_compatible",
    "version/abi_incompatible",
    "daemon/control_only",
    "daemon/permission_denied",
    "invocation/complete_tuple",
    "invocation/canonical_material",
    "invocation/presigned_submit",
    "invocation/prepared_not_submittable",
    "invocation/local_daemon_signing_boundary",
    "invocation/terminal_monotonicity",
    "authority/mutual_exclusion",
    "stream/order_terminal",
    "stream/backpressure_bound",
    "bidi/frame0_required",
    "bidi/close_send_not_cancel",
    "directory/snapshot_then_live",
    "directory/list_pagination",
    "directory/no_default_fanout",
    "aggregate/partial_result",
    "error/retry_hint",
    "health/api_vs_runtime",
    "backend/import_ban",
    "backend/no_direct_daemon_transport",
    "backend/hub_route_family_coverage",
    "backend/events_profile",
    "backend/admin_pairing_session_profile",
    "backend/surface_profile",
    "backend/compatibility_profile",
    "backend/wrapper_profile",
    "backend/receipt_projection",
    "python/easyremote_no_raw_ffi",
    "python/easyremote_no_invocation_codec",
    "python/easyremote_publication_profile",
    "python/easyremote_mission_profile",
    "python/easyremote_context_causal",
    "python/easyremote_host_binding_profile",
    "python/easyremote_admin_gateway_profile",
    "python/easyremote_product_facade_only",
)


def fail(message: str) -> None:
    print(f"sdk_section27_coverage: {message}", file=sys.stderr)
    raise SystemExit(1)


def require_string(value: object, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        fail(f"missing_or_empty_{field}")
    return value.strip()


def load_case_ids() -> set[str]:
    ids: set[str] = set()
    for path in sorted((repo / "sdk/conformance/cases").glob("*.yaml")):
        case_id = ""
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.startswith("id:"):
                case_id = require_string(line.split(":", 1)[1], f"{path}.id")
                break
        if not case_id:
            fail(f"missing_case_id:{path.relative_to(repo)}")
        if case_id in ids:
            fail(f"duplicate_case_id:{case_id}")
        ids.add(case_id)
    return ids


if not coverage_path.exists():
    fail(f"coverage_not_found:{coverage_path}")

try:
    coverage = json.loads(coverage_path.read_text(encoding="utf-8"))
except json.JSONDecodeError as exc:
    fail(f"invalid_json:{exc}")

if coverage.get("schema_version") != 1:
    fail("schema_version_must_be_1")
if coverage.get("source_spec") != SOURCE_SPEC:
    fail("source_spec_mismatch")

case_ids = load_case_ids()
rows = coverage.get("spec_cases")
if not isinstance(rows, list):
    fail("missing_spec_cases")

seen: set[str] = set()
for index, row in enumerate(rows):
    if not isinstance(row, dict):
        fail(f"invalid_spec_case_row:{index}")
    spec_case_id = require_string(row.get("spec_case_id"), f"spec_cases.{index}.spec_case_id")
    if spec_case_id in seen:
        fail(f"duplicate_spec_case:{spec_case_id}")
    seen.add(spec_case_id)
    if spec_case_id not in REQUIRED_SPEC_CASES:
        fail(f"unknown_spec_case:{spec_case_id}")
    covered_by = row.get("covered_by")
    if not isinstance(covered_by, list) or not covered_by:
        fail(f"missing_covered_by:{spec_case_id}")
    for covered_index, case_id_value in enumerate(covered_by):
        case_id = require_string(case_id_value, f"{spec_case_id}.covered_by.{covered_index}")
        if case_id not in case_ids:
            fail(f"missing_covered_case:{spec_case_id}:{case_id}")

missing = sorted(set(REQUIRED_SPEC_CASES) - seen)
if missing:
    fail("missing_spec_case:" + ",".join(missing))

print(f"sdk section 27 coverage ok: {coverage_path}")
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  mkdir -p "$REPO_ROOT/target"
  tmp="$(mktemp -d "$REPO_ROOT/target/sdk-section27-coverage.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT

  cp "$DEFAULT_COVERAGE" "$tmp/good.json"
  run_validator "$tmp/good.json" >/dev/null

  python3 - "$DEFAULT_COVERAGE" "$tmp/missing.json" "$tmp/bad-case.json" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path


source = Path(sys.argv[1])
missing = Path(sys.argv[2])
bad_case = Path(sys.argv[3])
coverage = json.loads(source.read_text(encoding="utf-8"))

without_required = json.loads(json.dumps(coverage))
without_required["spec_cases"] = [
    row
    for row in without_required["spec_cases"]
    if row["spec_case_id"] != "daemon/permission_denied"
]
missing.write_text(json.dumps(without_required), encoding="utf-8")

with_bad_case = json.loads(json.dumps(coverage))
with_bad_case["spec_cases"][0]["covered_by"] = ["runtime/missing_case"]
bad_case.write_text(json.dumps(with_bad_case), encoding="utf-8")
PY

  if run_validator "$tmp/missing.json" >"$tmp/missing.out" 2>&1; then
    echo "self-test expected missing SPEC case fixture to fail" >&2
    exit 1
  fi
  grep -Fq "missing_spec_case:daemon/permission_denied" "$tmp/missing.out"

  if run_validator "$tmp/bad-case.json" >"$tmp/bad-case.out" 2>&1; then
    echo "self-test expected missing covered case fixture to fail" >&2
    exit 1
  fi
  grep -Fq "missing_covered_case:version/abi_compatible:runtime/missing_case" "$tmp/bad-case.out"

  echo "check-sdk-section27-coverage self-test ok"
  exit 0
fi

run_validator "${1:-$DEFAULT_COVERAGE}"
