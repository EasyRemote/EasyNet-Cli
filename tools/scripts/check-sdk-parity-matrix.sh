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
SPEC_REF = "docs/spec/daemon-sdk-requirements-v1.md#10-capability-state-matrix"
LANGUAGES = ("go", "python")
STATUS_ORDER = ("unsupported", "seam", "provider-backed", "cutover-ready")
PROFILES = {
    "runtime_core", "addressing", "authority", "managed_signing", "access_control",
    "principal", "directory", "receipts", "runtime_events",
    "runtime_administration", "conformance",
}
EVIDENCE_KINDS = {"go_test", "python_test", "sdk_conformance_case", "static_gate", "manifest", "doc"}
REQUIRED_CAPABILITIES = (
    "abi_version_discovery",
    "daemon_lifecycle",
    "runtime_connection",
    "canonical_addressing",
    "runtime_health",
    "typed_errors",
    "ability_descriptor_projection",
    "authority_metadata",
    "complete_invocation_draft",
    "prepare_sign_submit",
    "managed_signing",
    "access_control",
    "principal_lifecycle",
    "principal_enrollment",
    "principal_public_key_bindings",
    "principal_recovery",
    "principal_authorization_grants",
    "directory_resolution",
    "receipt_history",
    "runtime_events",
    "runtime_administration",
    "unary_invoke",
    "stream",
    "bidi",
    "terminal_receipt_facts",
    "conformance_runner",
)
FORBIDDEN_TOKENS = (
    "gateway", "mission", "publication", "host_binding", "surface",
    "compatibility", "wrapper", "companion", "easyremote", "backend",
    "product",
)
ACTION_REPORTS = {
    "go": os.environ.get("EASYNET_SDK_PARITY_GO_REPORT", "sdk/conformance/runner/go-action-adapter-report.json"),
    "python": os.environ.get("EASYNET_SDK_PARITY_PYTHON_REPORT", "sdk/conformance/runner/python-action-adapter-report.json"),
}


def fail(message: str) -> None:
    print(f"sdk_parity_matrix: {message}", file=sys.stderr)
    raise SystemExit(1)


def require_string(value: object, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        fail(f"missing_or_empty:{field}")
    return value.strip()


def repo_path(ref: str, field: str) -> Path:
    path = (repo / ref).resolve()
    try:
        path.relative_to(repo)
    except ValueError:
        fail(f"outside_repo:{field}:{ref}")
    if not path.exists():
        fail(f"missing_ref:{field}:{ref}")
    return path


def string_list(value: object, field: str) -> list[str]:
    if not isinstance(value, list) or not value:
        fail(f"missing_or_empty:{field}")
    result = [require_string(item, field) for item in value]
    if len(result) != len(set(result)):
        fail(f"duplicate:{field}")
    return result


def case_metadata(ref: str) -> tuple[str, set[str]]:
    path = repo_path(ref, "shared_case")
    case_id = ""
    required_for: set[str] = set()
    in_required = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("id:"):
            case_id = require_string(line.split(":", 1)[1], f"{ref}.id")
            in_required = False
        elif line.startswith("required_for:"):
            in_required = True
        elif in_required and line.startswith("  - "):
            required_for.add(require_string(line[4:], f"{ref}.required_for"))
        elif in_required and line and not line.startswith(" "):
            in_required = False
    if not case_id or not required_for:
        fail(f"invalid_shared_case:{ref}")
    missing = set(LANGUAGES) - required_for
    if missing:
        fail(f"shared_case_not_symmetric:{ref}:{','.join(sorted(missing))}")
    return case_id, required_for


def load_report(language: str) -> dict[str, dict[str, object]]:
    ref = ACTION_REPORTS[language]
    try:
        report = json.loads(repo_path(ref, f"{language}_report").read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        fail(f"invalid_report_json:{language}:{exc}")
    if report.get("schema_version") != 1 or report.get("language") != language:
        fail(f"invalid_report_header:{language}")
    records = report.get("records")
    if not isinstance(records, list):
        fail(f"missing_report_records:{language}")
    indexed: dict[str, dict[str, object]] = {}
    for record in records:
        if not isinstance(record, dict):
            fail(f"invalid_report_record:{language}")
        case_id = require_string(record.get("case_id"), f"{language}.case_id")
        if case_id in indexed:
            fail(f"duplicate_report_record:{language}:{case_id}")
        indexed[case_id] = record
    return indexed


def validate_evidence(capability: str, language: str, status: str, raw: object) -> None:
    if status == "unsupported":
        if raw not in (None, []):
            fail(f"unsupported_with_evidence:{capability}:{language}")
        return
    if not isinstance(raw, list) or not raw:
        fail(f"missing_evidence:{capability}:{language}")
    kinds: set[str] = set()
    for item in raw:
        if not isinstance(item, dict):
            fail(f"invalid_evidence:{capability}:{language}")
        kind = require_string(item.get("kind"), f"{capability}.{language}.kind")
        ref = require_string(item.get("ref"), f"{capability}.{language}.ref")
        if kind not in EVIDENCE_KINDS:
            fail(f"unknown_evidence_kind:{capability}:{language}:{kind}")
        if kind != "doc":
            repo_path(ref, f"{capability}.{language}.evidence")
        kinds.add(kind)
    if status == "cutover-ready" and not ({"static_gate", "manifest"} & kinds):
        fail(f"cutover_without_gate:{capability}:{language}")


try:
    matrix = json.loads(matrix_path.read_text(encoding="utf-8"))
except (OSError, json.JSONDecodeError) as exc:
    fail(f"invalid_matrix:{exc}")

if matrix.get("schema_version") != 1:
    fail("schema_version")
if matrix.get("case_id") != CASE_ID:
    fail("case_id")
if matrix.get("source_spec") != SPEC_REF:
    fail("source_spec")
if tuple(matrix.get("status_order", ())) != STATUS_ORDER:
    fail("status_order")
if tuple(matrix.get("languages", ())) != LANGUAGES:
    fail("languages")
if "product_boundary_rules" in matrix:
    fail("product_boundary_rows_forbidden")

definitions = matrix.get("status_definitions")
if not isinstance(definitions, dict):
    fail("status_definitions")
for status in STATUS_ORDER:
    require_string(definitions.get(status), f"status_definitions.{status}")

rows = matrix.get("capabilities")
if not isinstance(rows, list):
    fail("capabilities")
if len(rows) != len(REQUIRED_CAPABILITIES):
    fail(f"capability_count:{len(rows)}_want_{len(REQUIRED_CAPABILITIES)}")

reports = {language: load_report(language) for language in LANGUAGES}
seen: set[str] = set()
for row in rows:
    if not isinstance(row, dict):
        fail("invalid_capability_row")
    capability = require_string(row.get("capability_id"), "capability_id")
    if capability in seen:
        fail(f"duplicate_capability:{capability}")
    seen.add(capability)
    if capability not in REQUIRED_CAPABILITIES:
        fail(f"unknown_capability:{capability}")
    profile = require_string(row.get("profile"), f"{capability}.profile")
    if profile not in PROFILES:
        fail(f"product_or_unknown_profile:{capability}:{profile}")
    surface = f"{capability} {profile}".lower()
    for token in FORBIDDEN_TOKENS:
        if token in surface:
            fail(f"product_specific_capability:{capability}:{token}")

    cases = [case_metadata(ref) for ref in string_list(row.get("shared_cases"), f"{capability}.shared_cases")]
    states: dict[str, str] = {}
    for language in LANGUAGES:
        state = row.get(language)
        if not isinstance(state, dict):
            fail(f"missing_language_state:{capability}:{language}")
        status = require_string(state.get("status"), f"{capability}.{language}.status")
        if status not in STATUS_ORDER:
            fail(f"invalid_status:{capability}:{language}:{status}")
        states[language] = status
        validate_evidence(capability, language, status, state.get("evidence"))
        if status == "provider-backed" or status == "cutover-ready":
            for case_id, _required_for in cases:
                record = reports[language].get(case_id)
                if record is None:
                    fail(f"provider_backed_missing_action_report:{capability}:{language}:{case_id}")
                if record.get("status") != "passed":
                    fail(f"provider_backed_action_report_not_passed:{capability}:{language}:{case_id}")
    if states["go"] != states["python"]:
        fail(f"language_state_mismatch:{capability}:{states['go']}:{states['python']}")
    if states["go"] != "cutover-ready":
        require_string(row.get("remaining"), f"{capability}.remaining")
    elif row.get("remaining") not in (None, ""):
        fail(f"cutover_ready_has_remaining:{capability}")

missing = sorted(set(REQUIRED_CAPABILITIES) - seen)
if missing:
    fail("missing_capability:" + ",".join(missing))

print(f"sdk parity matrix ok: {matrix_path}")
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  mkdir -p "$REPO_ROOT/target"
  tmp="$(mktemp -d "$REPO_ROOT/target/sdk-parity-self-test.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT

  cp "$DEFAULT_MATRIX" "$tmp/good.json"
  run_validator "$tmp/good.json" >/dev/null

  python3 - "$DEFAULT_MATRIX" "$tmp" "$REPO_ROOT/sdk/conformance/runner/go-action-adapter-report.json" <<'PY'
import json, sys
from pathlib import Path
source, out, go_report = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
matrix = json.loads(source.read_text())

def write(name, value):
    (out / name).write_text(json.dumps(value))

value = json.loads(json.dumps(matrix)); value["capabilities"] = value["capabilities"][:-1]; write("missing.json", value)
value = json.loads(json.dumps(matrix)); value["capabilities"][0]["go"]["status"] = "partial"; write("status.json", value)
value = json.loads(json.dumps(matrix)); value["capabilities"][0]["python"]["status"] = "seam"; write("mismatch.json", value)
value = json.loads(json.dumps(matrix)); value["capabilities"][0]["capability_id"] = "mission_runtime"; write("product.json", value)
value = json.loads(json.dumps(matrix)); value["product_boundary_rules"] = []; write("boundary.json", value)
value = json.loads(json.dumps(matrix)); value["capabilities"][0]["go"]["status"] = "cutover-ready"; value["capabilities"][0]["python"]["status"] = "cutover-ready"; write("cutover.json", value)

report = json.loads(go_report.read_text())
report["records"] = [r for r in report["records"] if r["case_id"] != "invocation/complete_tuple"]
write("go-report-missing.json", report)
PY

  expect_failure() {
    local fixture="$1" pattern="$2"
    if run_validator "$tmp/$fixture" >"$tmp/$fixture.out" 2>&1; then
      echo "self-test expected $fixture to fail" >&2
      exit 1
    fi
    grep -Eq "$pattern" "$tmp/$fixture.out"
  }

  expect_failure missing.json 'capability_count|missing_capability'
  expect_failure status.json 'invalid_status'
  expect_failure mismatch.json 'language_state_mismatch'
  expect_failure product.json 'unknown_capability|product_specific_capability'
  expect_failure boundary.json 'product_boundary_rows_forbidden'
  expect_failure cutover.json 'cutover_without_gate|cutover_ready_has_remaining'
  if EASYNET_SDK_PARITY_GO_REPORT="$tmp/go-report-missing.json" run_validator "$tmp/good.json" >"$tmp/report.out" 2>&1; then
    echo "self-test expected missing report case to fail" >&2
    exit 1
  fi
  grep -Fq 'provider_backed_missing_action_report' "$tmp/report.out"

  echo "check-sdk-parity-matrix self-test ok"
  exit 0
fi

run_validator "${1:-$DEFAULT_MATRIX}"
