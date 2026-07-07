#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
DEFAULT_MATRIX="$REPO_ROOT/sdk/conformance/sdk-parity-matrix.json"

run_validator() {
  local matrix="$1"
  python3 - "$REPO_ROOT" "$matrix" <<'PY'
from __future__ import annotations

import json
import os
import sys
from pathlib import Path


repo = Path(sys.argv[1]).resolve()
matrix_path = Path(sys.argv[2]).resolve()

CASE_ID = "sdk/go_python_parity_matrix"
SPEC_REF = "docs/spec/daemon-sdk-requirements-v1.md#5.7"
LANGUAGES = ("go", "python")
STATUS_ORDER = ("unsupported", "seam", "provider-backed", "cutover-ready")
ALLOWED_EVIDENCE_KINDS = {
    "go_test",
    "python_test",
    "sdk_conformance_case",
    "static_gate",
    "manifest",
    "doc",
}

REQUIRED_CAPABILITIES = (
    "abi_version_discovery",
    "daemon_lifecycle",
    "runtime_connection",
    "runtime_health",
    "runtime_companion_control",
    "typed_errors",
    "complete_invocation_draft",
    "prepare_sign_submit",
    "unary_invoke",
    "stream",
    "bidi",
    "directory_identity",
    "receipt",
    "publication",
    "host_binding",
    "mission",
    "admin_gateway",
    "events",
    "surface",
    "compatibility",
    "wrappers",
    "conformance_runner",
)

REQUIRED_PRODUCT_BOUNDARIES = ("easynet_product", "easyremote_product")
FORBIDDEN_CAPABILITY_TOKENS = ("backend", "easyremote", "product", "hub")
PROVIDER_BACKED_REQUIRED_EVIDENCE = {
    "surface": {
        "go": "sdk/go/surface_runtime_test.go",
        "python": "sdk/python/tests/test_cabi.py",
    },
    "compatibility": {
        "go": "sdk/go/compatibility_runtime_test.go",
        "python": "sdk/python/tests/test_cabi.py",
    },
    "wrappers": {
        "go": "sdk/go/wrappers_runtime_test.go",
        "python": "sdk/python/tests/test_wrappers.py",
    },
}
ACTION_ADAPTER_REPORTS = {
    "go": os.environ.get(
        "EASYNET_SDK_PARITY_GO_REPORT",
        "sdk/conformance/runner/go-action-adapter-report.json",
    ),
    "python": os.environ.get(
        "EASYNET_SDK_PARITY_PYTHON_REPORT",
        "sdk/conformance/runner/python-action-adapter-report.json",
    ),
}


def fail(message: str) -> None:
    print(f"sdk_parity_matrix: {message}", file=sys.stderr)
    raise SystemExit(1)


def require_string(value: object, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        fail(f"missing_or_empty_{field}")
    return value.strip()


def require_string_list(value: object, field: str) -> list[str]:
    if not isinstance(value, list) or not value:
        fail(f"missing_or_empty_{field}")
    result: list[str] = []
    for item in value:
        if not isinstance(item, str) or not item.strip():
            fail(f"invalid_{field}")
        result.append(item.strip())
    if len(set(result)) != len(result):
        fail(f"duplicate_{field}")
    return result


def repo_ref_exists(ref: str) -> bool:
    path = (repo / ref).resolve()
    try:
        path.relative_to(repo)
    except ValueError:
        return False
    return path.exists()


def load_json_repo_ref(ref: str, field: str) -> object:
    path = (repo / ref).resolve()
    try:
        path.relative_to(repo)
    except ValueError:
        fail(f"{field}_outside_repo:{ref}")
    if not path.exists():
        fail(f"missing_{field}:{ref}")
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        fail(f"invalid_{field}:{ref}:{exc}")


def load_shared_case_metadata(ref: str) -> tuple[str, set[str]]:
    path = (repo / ref).resolve()
    try:
        path.relative_to(repo)
    except ValueError:
        fail(f"shared_case_outside_repo:{ref}")
    if not path.exists():
        fail(f"missing_shared_case:{ref}")
    case_id = ""
    required_for: set[str] = set()
    in_required_for = False
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if line.startswith("id:"):
            case_id = require_string(line.split(":", 1)[1], f"{ref}.id")
            in_required_for = False
            continue
        if line.startswith("required_for:"):
            in_required_for = True
            continue
        if in_required_for:
            if line.startswith("  - "):
                required_for.add(require_string(line[4:], f"{ref}.required_for"))
                continue
            if stripped and not line.startswith(" "):
                in_required_for = False
    if not case_id:
        fail(f"missing_shared_case_id:{ref}")
    if not required_for:
        fail(f"missing_shared_case_required_for:{ref}")
    return case_id, required_for


def load_action_adapter_report(language: str) -> dict[str, dict[str, object]]:
    report_ref = ACTION_ADAPTER_REPORTS[language]
    report = load_json_repo_ref(report_ref, f"{language}_action_adapter_report")
    if not isinstance(report, dict):
        fail(f"invalid_{language}_action_adapter_report")
    if report.get("schema_version") != 1:
        fail(f"{language}_action_adapter_report_schema_version")
    if report.get("language") != language:
        fail(f"{language}_action_adapter_report_language")
    records = report.get("records")
    if not isinstance(records, list):
        fail(f"{language}_action_adapter_report_missing_records")
    by_case: dict[str, dict[str, object]] = {}
    for index, record in enumerate(records):
        if not isinstance(record, dict):
            fail(f"{language}_action_adapter_report_invalid_record:{index}")
        case_id = require_string(record.get("case_id"), f"{language}.report.case_id")
        if case_id in by_case:
            fail(f"{language}_action_adapter_report_duplicate_case:{case_id}")
        by_case[case_id] = record
    return by_case


def validate_evidence(owner: str, status: str, evidence: object) -> None:
    if status == "unsupported":
        if evidence not in ([], None):
            fail(f"unsupported_must_not_have_evidence:{owner}")
        return
    if not isinstance(evidence, list) or not evidence:
        fail(f"missing_evidence:{owner}")
    for index, item in enumerate(evidence):
        if not isinstance(item, dict):
            fail(f"invalid_evidence:{owner}:{index}")
        kind = require_string(item.get("kind"), f"{owner}.evidence.kind")
        ref = require_string(item.get("ref"), f"{owner}.evidence.ref")
        if kind not in ALLOWED_EVIDENCE_KINDS:
            fail(f"unknown_evidence_kind:{owner}:{kind}")
        if kind != "doc" and not repo_ref_exists(ref):
            fail(f"missing_evidence_ref:{owner}:{ref}")


def require_provider_backed_evidence(row_id: str, row: dict[str, object]) -> None:
    required = PROVIDER_BACKED_REQUIRED_EVIDENCE.get(row_id)
    if required is None:
        return
    for language, required_ref in required.items():
        state = row[language]
        if not isinstance(state, dict):
            fail(f"missing_language_state:{row_id}:{language}")
        if state.get("status") != "provider-backed":
            continue
        evidence = state.get("evidence")
        if not isinstance(evidence, list):
            fail(f"missing_evidence:{row_id}.{language}")
        refs = {item.get("ref") for item in evidence if isinstance(item, dict)}
        if required_ref not in refs:
            fail(f"provider_backed_without_runtime_evidence:{row_id}:{language}:{required_ref}")


def require_provider_backed_action_reports(
    row_id: str,
    row: dict[str, object],
    shared_cases: list[tuple[str, set[str]]],
    action_reports: dict[str, dict[str, dict[str, object]]],
) -> None:
    for language in LANGUAGES:
        state = row[language]
        if not isinstance(state, dict):
            fail(f"missing_language_state:{row_id}:{language}")
        if state.get("status") != "provider-backed":
            continue
        for case_id, required_for in shared_cases:
            if language not in required_for:
                continue
            record = action_reports[language].get(case_id)
            if record is None:
                fail(f"provider_backed_missing_action_report:{row_id}:{language}:{case_id}")
            status = require_string(record.get("status"), f"{language}.report.{case_id}.status")
            if status != "passed":
                fail(f"provider_backed_action_report_not_passed:{row_id}:{language}:{case_id}:{status}")


def validate_language_status(row_id: str, row: dict[str, object]) -> tuple[str, str]:
    statuses: list[str] = []
    for language in LANGUAGES:
        state = row.get(language)
        if not isinstance(state, dict):
            fail(f"missing_language_state:{row_id}:{language}")
        status = require_string(state.get("status"), f"{row_id}.{language}.status")
        if status not in STATUS_ORDER:
            fail(f"invalid_status:{row_id}:{language}:{status}")
        validate_evidence(f"{row_id}.{language}", status, state.get("evidence"))
        statuses.append(status)
    go_status, python_status = statuses
    if go_status != python_status and not require_string(row.get("parity_gap"), f"{row_id}.parity_gap"):
        fail(f"missing_parity_gap:{row_id}")
    if "cutover-ready" in statuses:
        remaining = str(row.get("remaining", "")).lower()
        if "incomplete" in remaining or "remain" in remaining:
            fail(f"false_cutover_ready:{row_id}")
        for language, status in zip(LANGUAGES, statuses):
            if status == "cutover-ready":
                evidence = row[language]["evidence"]  # type: ignore[index]
                kinds = {item.get("kind") for item in evidence}  # type: ignore[union-attr]
                if "static_gate" not in kinds and "manifest" not in kinds:
                    fail(f"cutover_ready_without_gate:{row_id}:{language}")
    else:
        require_string(row.get("remaining"), f"{row_id}.remaining")
    return go_status, python_status


def validate_product_boundary_evidence(row_id: str, evidence: object) -> None:
    if not isinstance(evidence, list) or not evidence:
        fail(f"missing_product_boundary_evidence:{row_id}")
    for index, item in enumerate(evidence):
        if not isinstance(item, dict):
            fail(f"invalid_product_boundary_evidence:{row_id}:{index}")
        kind = require_string(item.get("kind"), f"{row_id}.evidence.kind")
        ref = require_string(item.get("ref"), f"{row_id}.evidence.ref")
        if kind not in ALLOWED_EVIDENCE_KINDS:
            fail(f"unknown_evidence_kind:{row_id}:{kind}")
        if kind != "doc" and not repo_ref_exists(ref):
            fail(f"missing_product_boundary_evidence_ref:{row_id}:{ref}")


if not matrix_path.exists():
    fail(f"matrix_not_found:{matrix_path}")

try:
    matrix = json.loads(matrix_path.read_text(encoding="utf-8"))
except json.JSONDecodeError as exc:
    fail(f"invalid_json:{exc}")

if matrix.get("schema_version") != 1:
    fail("schema_version_must_be_1")
if matrix.get("case_id") != CASE_ID:
    fail("case_id_mismatch")
if matrix.get("source_spec") != SPEC_REF:
    fail("source_spec_mismatch")
if tuple(matrix.get("status_order", ())) != STATUS_ORDER:
    fail("status_order_mismatch")
if tuple(matrix.get("languages", ())) != LANGUAGES:
    fail("languages_mismatch")

definitions = matrix.get("status_definitions")
if not isinstance(definitions, dict):
    fail("missing_status_definitions")
for status in STATUS_ORDER:
    require_string(definitions.get(status), f"status_definitions.{status}")

capabilities = matrix.get("capabilities")
if not isinstance(capabilities, list):
    fail("missing_capabilities")
if len(capabilities) != len(REQUIRED_CAPABILITIES):
    fail(f"capability_count:{len(capabilities)}_want_{len(REQUIRED_CAPABILITIES)}")

seen: set[str] = set()
action_reports = {
    language: load_action_adapter_report(language)
    for language in LANGUAGES
}
for row in capabilities:
    if not isinstance(row, dict):
        fail("invalid_capability_row")
    row_id = require_string(row.get("capability_id"), "capability_id")
    if row_id in seen:
        fail(f"duplicate_capability:{row_id}")
    seen.add(row_id)
    if row_id not in REQUIRED_CAPABILITIES:
        fail(f"unknown_capability:{row_id}")
    profile = require_string(row.get("profile"), f"{row_id}.profile")
    product_surface = f"{row_id} {profile}".lower()
    for token in FORBIDDEN_CAPABILITY_TOKENS:
        if token in product_surface:
            fail(f"product_specific_capability:{row_id}:{token}")
    shared_case_metadata = [
        load_shared_case_metadata(ref)
        for ref in require_string_list(row.get("shared_cases"), f"{row_id}.shared_cases")
    ]
    validate_language_status(row_id, row)
    require_provider_backed_evidence(row_id, row)
    require_provider_backed_action_reports(row_id, row, shared_case_metadata, action_reports)

missing = sorted(set(REQUIRED_CAPABILITIES) - seen)
if missing:
    fail("missing_capability:" + ",".join(missing))

product_boundaries = matrix.get("product_boundary_rules")
if not isinstance(product_boundaries, list):
    fail("missing_product_boundary_rules")
if len(product_boundaries) != len(REQUIRED_PRODUCT_BOUNDARIES):
    fail(f"product_boundary_count:{len(product_boundaries)}_want_{len(REQUIRED_PRODUCT_BOUNDARIES)}")

seen_boundaries: set[str] = set()
for row in product_boundaries:
    if not isinstance(row, dict):
        fail("invalid_product_boundary_row")
    row_id = require_string(row.get("product_id"), "product_id")
    if row_id in seen_boundaries:
        fail(f"duplicate_product_boundary:{row_id}")
    seen_boundaries.add(row_id)
    if row_id not in REQUIRED_PRODUCT_BOUNDARIES:
        fail(f"unknown_product_boundary:{row_id}")
    primary = require_string(row.get("primary_sdk_language"), f"{row_id}.primary_sdk_language")
    if primary not in LANGUAGES:
        fail(f"invalid_primary_sdk_language:{row_id}:{primary}")
    if row.get("not_sdk_profile") is not True:
        fail(f"product_boundary_must_not_be_sdk_profile:{row_id}")
    require_string(row.get("rule"), f"{row_id}.rule")
    validate_product_boundary_evidence(row_id, row.get("evidence"))

missing_boundaries = sorted(set(REQUIRED_PRODUCT_BOUNDARIES) - seen_boundaries)
if missing_boundaries:
    fail("missing_product_boundary:" + ",".join(missing_boundaries))

print(f"sdk parity matrix ok: {matrix_path}")
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  mkdir -p "$REPO_ROOT/target"
  tmp="$(mktemp -d "$REPO_ROOT/target/sdk-parity-self-test.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT

  cp "$DEFAULT_MATRIX" "$tmp/good.json"
  run_validator "$tmp/good.json" >/dev/null

  python3 - "$DEFAULT_MATRIX" "$tmp/missing.json" "$tmp/status.json" "$tmp/cutover.json" "$tmp/product.json" "$tmp/provider.json" "$tmp/go-report-missing.json" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path


source = Path(sys.argv[1])
missing = Path(sys.argv[2])
status = Path(sys.argv[3])
cutover = Path(sys.argv[4])
product = Path(sys.argv[5])
provider = Path(sys.argv[6])
go_report_missing = Path(sys.argv[7])

matrix = json.loads(source.read_text(encoding="utf-8"))

without_capability = json.loads(json.dumps(matrix))
without_capability["capabilities"] = without_capability["capabilities"][:-1]
missing.write_text(json.dumps(without_capability), encoding="utf-8")

with_bad_status = json.loads(json.dumps(matrix))
with_bad_status["capabilities"][0]["go"]["status"] = "partial"
status.write_text(json.dumps(with_bad_status), encoding="utf-8")

with_false_cutover = json.loads(json.dumps(matrix))
with_false_cutover["capabilities"][0]["go"]["status"] = "cutover-ready"
with_false_cutover["capabilities"][0]["python"]["status"] = "cutover-ready"
with_false_cutover["capabilities"][0]["remaining"] = "lower-layer product cutover remains incomplete"
cutover.write_text(json.dumps(with_false_cutover), encoding="utf-8")

with_product_capability = json.loads(json.dumps(matrix))
with_product_capability["capabilities"][0]["capability_id"] = "easyremote_runtime"
product.write_text(json.dumps(with_product_capability), encoding="utf-8")

with_missing_provider_evidence = json.loads(json.dumps(matrix))
for row in with_missing_provider_evidence["capabilities"]:
    if row["capability_id"] == "surface":
        row["go"]["evidence"] = [
            item for item in row["go"]["evidence"]
            if item["ref"] != "sdk/go/surface_runtime_test.go"
        ]
        break
provider.write_text(json.dumps(with_missing_provider_evidence), encoding="utf-8")

repo_root = source.resolve().parents[2]
go_report = json.loads((repo_root / "sdk/conformance/runner/go-action-adapter-report.json").read_text(encoding="utf-8"))
go_report["records"] = [
    record for record in go_report["records"]
    if record["case_id"] != "receipt/axon_chain_verification"
]
go_report_missing.write_text(json.dumps(go_report), encoding="utf-8")
PY

  if run_validator "$tmp/missing.json" >"$tmp/missing.out" 2>&1; then
    echo "self-test expected missing capability fixture to fail" >&2
    exit 1
  fi
  grep -Eq "capability_count|missing_capability" "$tmp/missing.out"

  if run_validator "$tmp/status.json" >"$tmp/status.out" 2>&1; then
    echo "self-test expected invalid status fixture to fail" >&2
    exit 1
  fi
  grep -Fq "invalid_status" "$tmp/status.out"

  if run_validator "$tmp/cutover.json" >"$tmp/cutover.out" 2>&1; then
    echo "self-test expected false cutover fixture to fail" >&2
    exit 1
  fi
  grep -Fq "false_cutover_ready" "$tmp/cutover.out"

  if run_validator "$tmp/product.json" >"$tmp/product.out" 2>&1; then
    echo "self-test expected product capability fixture to fail" >&2
    exit 1
  fi
  grep -Eq "unknown_capability|product_specific_capability" "$tmp/product.out"

  if run_validator "$tmp/provider.json" >"$tmp/provider.out" 2>&1; then
    echo "self-test expected provider-backed runtime evidence fixture to fail" >&2
    exit 1
  fi
  grep -Fq "provider_backed_without_runtime_evidence" "$tmp/provider.out"

  if EASYNET_SDK_PARITY_GO_REPORT="$tmp/go-report-missing.json" run_validator "$tmp/good.json" >"$tmp/report.out" 2>&1; then
    echo "self-test expected missing action-adapter report fixture to fail" >&2
    exit 1
  fi
  grep -Fq "provider_backed_missing_action_report" "$tmp/report.out"

  echo "check-sdk-parity-matrix self-test ok"
  exit 0
fi

run_validator "${1:-$DEFAULT_MATRIX}"
