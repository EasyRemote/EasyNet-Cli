#!/usr/bin/env bash
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

export PYTHONDONTWRITEBYTECODE="${PYTHONDONTWRITEBYTECODE:-1}"

fail() {
  echo "check-sdk-scaffold: $*" >&2
  exit 1
}

required=(
  PROJECT_STRUCTURE.md
  include/easynet_cli.h
  include/easynet_cli.exports.v7
  include/easynet_cli.exports.v8
  include/easynet_cli.exports.v9
  tools/sdk-conformance-runner/Cargo.toml
  tools/sdk-conformance-runner/src/main.rs
  sdk/README.md
  sdk/SDK_INTERFACE_SPEC.md
  sdk/SDK_PARITY.md
  sdk/CONFORMANCE_SUITE.md
  sdk/conformance/fixture-schema-bindings.json
  sdk/conformance/python_toolchain.sh
  sdk/conformance/refresh_conformance_report_evidence.py
  sdk/conformance/sdk-parity-matrix.json
  sdk/conformance/runner/README.md
  sdk/go/go.mod
  sdk/python/pyproject.toml
  sdk/node/package.json
  sdk/java/pom.xml
  sdk/swift/Package.swift
)

for path in "${required[@]}"; do
  [[ -f "$ROOT/$path" ]] || fail "missing required artifact: $path"
done

for path in \
  sdk/schemas \
  sdk/conformance/cases \
  sdk/conformance/fixtures \
  sdk/conformance/runner \
  sdk/go \
  sdk/python/easynet_sdk \
  sdk/node \
  sdk/java/src/main \
  sdk/swift/Sources
do
  [[ -d "$ROOT/$path" ]] || fail "missing required directory: $path"
done

python3 - "$ROOT" <<'PY'
from __future__ import annotations

import json
import re
import sys
from collections import Counter
from pathlib import Path

root = Path(sys.argv[1]).resolve()
schema_dir = root / "sdk/schemas"
fixture_dir = root / "sdk/conformance/fixtures"
case_dir = root / "sdk/conformance/cases"
runner_dir = root / "sdk/conformance/runner"


def fail(message: str) -> None:
    raise SystemExit(f"check-sdk-scaffold: {message}")


def load_json(path: Path) -> object:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:
        fail(f"invalid JSON {path.relative_to(root)}: {exc}")


schemas = sorted(schema_dir.glob("*.schema.json"))
fixtures = sorted(fixture_dir.glob("*.v*.json"))
cases = sorted(case_dir.glob("*.yaml"))
reports = sorted(runner_dir.glob("*-runtime-conformance-report.json"))
if not schemas or not fixtures or not cases or not reports:
    fail("schema, fixture, case, and conformance-report sets must be non-empty")

for path in schemas + fixtures + reports:
    load_json(path)

feature_schema = load_json(schema_dir / "feature-discovery.schema.json")
try:
    extension_required = feature_schema["properties"]["abi_extensions"]["required"]
    symbol_required = feature_schema["properties"]["symbols"]["required"]
except (KeyError, TypeError) as exc:
    fail(f"feature discovery schema has no additive extension boundary: {exc}")
if "v9" in extension_required:
    fail("feature discovery schema must accept pre-v9 discovery without abi_extensions.v9")
if "stream_buffer_lease_v9" in symbol_required:
    fail("feature discovery schema must accept pre-v9 discovery without stream_buffer_lease_v9")

bindings_path = root / "sdk/conformance/fixture-schema-bindings.json"
bindings = load_json(bindings_path)
if not isinstance(bindings, dict) or bindings.get("schema_version") != 1:
    fail("fixture-schema-bindings schema_version must be 1")
rows = bindings.get("bindings")
if not isinstance(rows, list) or not rows:
    fail("fixture-schema-bindings must contain bindings")

fixture_names = {path.name for path in fixtures}
retired_schemas = {
    "agent-record.schema.json",
    "ability-deploy-request.schema.json",
    "ability-deploy-result.schema.json",
    "ability-package-manifest.schema.json",
    "package-validation.schema.json",
    "published-ability.schema.json",
    "resource-ref.schema.json",
    "local-resource-ref-request.schema.json",
    "lifecycle-status.schema.json",
}
retired_fixtures = {
    "ability-deploy-request.v4.json",
    "ability-package-manifest.v4.json",
    "local-resource-ref-request.v4.json",
    "package-validation.v4.json",
    "resource-ref.local-fs.v4.json",
}
retired_schema_hits = sorted(path.name for path in schemas if path.name in retired_schemas)
retired_fixture_hits = sorted(path.name for path in fixtures if path.name in retired_fixtures)
if retired_schema_hits or retired_fixture_hits:
    fail(
        "retired product SDK schema fixtures remain: "
        f"schemas={retired_schema_hits}, fixtures={retired_fixture_hits}"
    )
bound_names: list[str] = []
for index, row in enumerate(rows):
    if not isinstance(row, dict):
        fail(f"invalid fixture binding at index {index}")
    fixture = row.get("fixture")
    schema = row.get("schema")
    if not isinstance(fixture, str) or not re.fullmatch(r".+\.v[1-9][0-9]*\.json", fixture):
        fail(f"invalid fixture binding name: {fixture!r}")
    if not isinstance(schema, str) or not schema.endswith(".schema.json"):
        fail(f"invalid schema binding for {fixture}")
    if fixture in retired_fixtures or schema in retired_schemas:
        fail(f"retired product fixture binding remains: {fixture} -> {schema}")
    if not (schema_dir / schema).is_file():
        fail(f"missing bound schema: {fixture} -> {schema}")
    bound_names.append(fixture)

duplicates = sorted(name for name, count in Counter(bound_names).items() if count > 1)
missing = sorted(fixture_names - set(bound_names))
extra = sorted(set(bound_names) - fixture_names)
if duplicates or missing or extra:
    fail(f"fixture binding closure failed: duplicates={duplicates}, missing={missing}, extra={extra}")

case_ids: list[str] = []
for path in cases:
    text = path.read_text(encoding="utf-8")
    case_id = re.search(r"(?m)^id:\s*(\S.*?)\s*$", text)
    profile = re.search(r"(?m)^profile:\s*(\S.*?)\s*$", text)
    required = re.search(r"(?m)^required_for:(?:\s*\[\])?\s*$", text)
    steps = re.search(r"(?m)^steps:\s*$", text)
    action = re.search(r"(?m)^\s+- action:\s*\S", text)
    expect = re.search(r"(?m)^expect:\s*$", text)
    if not all((case_id, profile, required, steps, action, expect)):
        fail(f"incomplete conformance case: {path.relative_to(root)}")
    case_ids.append(case_id.group(1).strip())
duplicates = sorted(name for name, count in Counter(case_ids).items() if count > 1)
if duplicates:
    fail("duplicate conformance case ids: " + ", ".join(duplicates))

matrix = load_json(root / "sdk/conformance/sdk-parity-matrix.json")
if not isinstance(matrix, dict) or matrix.get("schema_version") != 5:
    fail("SDK parity matrix schema_version must be 5")
languages = ["rust", "c_abi", "go", "python", "node", "java", "swift"]
if matrix.get("languages") != languages:
    fail("SDK parity matrix must declare all seven canonical languages")
capabilities = matrix.get("capability_ids")
cells = matrix.get("cells")
if not isinstance(capabilities, list) or not isinstance(cells, list):
    fail("SDK parity matrix is missing capabilities or cells")
keys = [(cell.get("capability_id"), cell.get("language")) for cell in cells]
expected = [(capability, language) for capability in capabilities for language in languages]
if keys != expected or len(keys) != len(set(keys)):
    fail("SDK parity matrix is not the complete ordered Cartesian product")
if "product_boundary_rules" in matrix:
    fail("product boundary rows do not belong in the runtime SDK matrix")

for path in reports:
    report = load_json(path)
    if not isinstance(report, dict) or report.get("schema_version") != 2:
        fail(f"invalid conformance report header: {path.relative_to(root)}")
    if not isinstance(report.get("records"), list):
        fail(f"missing conformance records: {path.relative_to(root)}")
PY

header_symbols="$(python3 - "$ROOT/include/easynet_cli.h" <<'PY'
import re, sys
text = open(sys.argv[1], encoding='utf-8').read()
print('\n'.join(sorted(set(re.findall(r'\b(runtime_[A-Za-z0-9_]+)\s*\(', text)))))
PY
)"
v7_symbols="$(LC_ALL=C sort -u "$ROOT/include/easynet_cli.exports.v7")"
v8_symbols="$(LC_ALL=C sort -u "$ROOT/include/easynet_cli.exports.v8")"
v9_symbols="$(LC_ALL=C sort -u "$ROOT/include/easynet_cli.exports.v9")"
[[ "$header_symbols" == "$v9_symbols" ]] || fail "C header and latest v9 export allowlist differ"
[[ "$(printf '%s\n' "$v7_symbols" | grep -c '^runtime_')" -eq 56 ]] || fail "generic C ABI v7 must contain exactly 56 runtime symbols"
[[ "$(printf '%s\n' "$v8_symbols" | grep -c '^runtime_')" -eq 57 ]] || fail "generic C ABI v8 must contain exactly 57 runtime symbols"
[[ "$(printf '%s\n' "$v9_symbols" | grep -c '^runtime_')" -eq 60 ]] || fail "generic C ABI v9 must contain exactly 60 runtime symbols"
[[ "$(comm -23 "$ROOT/include/easynet_cli.exports.v7" "$ROOT/include/easynet_cli.exports.v8")" == "" ]] || fail "generic C ABI v8 must include every v7 symbol"
[[ "$(comm -13 "$ROOT/include/easynet_cli.exports.v7" "$ROOT/include/easynet_cli.exports.v8")" == "runtime_invocation_stream_open_v8" ]] || fail "generic C ABI v8 must add only runtime_invocation_stream_open_v8"
[[ "$(comm -23 "$ROOT/include/easynet_cli.exports.v8" "$ROOT/include/easynet_cli.exports.v9")" == "" ]] || fail "generic C ABI v9 must include every v8 symbol"
[[ "$(comm -13 "$ROOT/include/easynet_cli.exports.v8" "$ROOT/include/easynet_cli.exports.v9")" == $'runtime_buffer_lease_release_v9\nruntime_buffer_lease_retain_v9\nruntime_invocation_stream_open_v9' ]] || fail "generic C ABI v9 has an invalid additive set"
[[ "$(printf '%s\n' "$v7_symbols" | grep -c '^easynet_')" -eq 0 ]] || fail "generic C ABI v7 must not contain easynet-prefixed symbols"
[[ "$(printf '%s\n' "$v8_symbols" | grep -c '^easynet_')" -eq 0 ]] || fail "generic C ABI v8 must not contain easynet-prefixed symbols"
[[ "$(printf '%s\n' "$v9_symbols" | grep -c '^easynet_')" -eq 0 ]] || fail "generic C ABI v9 must not contain easynet-prefixed symbols"
if printf '%s\n' "$v9_symbols" | rg -q '_(admin|directory|identity|mission|publication|receipt|surface|compatibility|host_binding|events|wrapper|companion)_'; then
  fail "product-domain symbol leaked into C ABI v7"
fi

if command -v cc >/dev/null 2>&1; then
  tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/easynet-sdk-header.XXXXXX")"
  tmp="$tmp_dir/header.c"
  trap 'rm -rf "$tmp_dir"' EXIT
  printf '#include "include/easynet_cli.h"\n' >"$tmp"
  cc -fsyntax-only -I"$ROOT" "$tmp" >/dev/null 2>&1 || fail "include/easynet_cli.h does not compile as C"
fi

echo "check-sdk-scaffold ok"
