#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

validate_root() {
  local root="$1"
  python3 - "$root" <<'PY'
from __future__ import annotations

import json
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # Python < 3.11 in local developer environments.
    import tomli as tomllib


root = Path(sys.argv[1])
failures: list[str] = []


def fail(message: str) -> None:
    failures.append(message)


def require_file(path: str) -> Path | None:
    candidate = root / path
    if not candidate.is_file():
        fail(f"missing:{path}")
        return None
    return candidate


def require_text(path: str, expected: str, label: str) -> str:
    candidate = require_file(path)
    if candidate is None:
        return ""
    text = candidate.read_text(encoding="utf-8")
    if expected not in text:
        fail(f"{label}:missing:{expected}")
    return text


go_mod = require_text("sdk/go/go.mod", "module easynet.run/cli/sdk/go", "go.mod")
if go_mod and "\ngo 1.22" not in go_mod:
    fail("go.mod:go_version_must_be_1.22")

pyproject_path = require_file("sdk/python/pyproject.toml")
if pyproject_path is not None:
    try:
        pyproject = tomllib.loads(pyproject_path.read_text(encoding="utf-8"))
    except tomllib.TOMLDecodeError as exc:
        fail(f"pyproject:invalid_toml:{exc}")
        pyproject = {}
    project = pyproject.get("project", {})
    if project.get("name") != "easynet-sdk":
        fail("pyproject:project.name_must_be_easynet-sdk")
    if not str(project.get("version", "")).strip():
        fail("pyproject:project.version_required")
    if project.get("requires-python") != ">=3.11":
        fail("pyproject:requires-python_must_be_>=3.11")
    package_data = pyproject.get("tool", {}).get("setuptools", {}).get("package-data", {})
    if "py.typed" not in package_data.get("easynet_sdk", []):
        fail("pyproject:py.typed_package_data_required")

node_path = require_file("sdk/node/package.json")
if node_path is not None:
    try:
        node = json.loads(node_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        fail(f"node:invalid_json:{exc}")
        node = {}
    if node.get("name") != "@easynet/daemon-sdk":
        fail("node:name_must_be_@easynet/daemon-sdk")
    if node.get("version") != "0.0.0-seam":
        fail("node:version_must_be_0.0.0-seam")
    if node.get("private") is not True:
        fail("node:p1_seam_package_must_remain_private")
    if node.get("type") != "module":
        fail("node:type_must_be_module")
    if node.get("main") != "./index.js":
        fail("node:main_must_be_index_js")
    if node.get("types") != "./index.d.ts":
        fail("node:types_must_be_index_d_ts")
    files = set(node.get("files", []))
    for required in {"index.js", "index.d.ts", "README.md"}:
        if required not in files:
            fail(f"node:files_missing:{required}")

pom_path = require_file("sdk/java/pom.xml")
if pom_path is not None:
    try:
        pom = ET.parse(pom_path).getroot()
    except ET.ParseError as exc:
        fail(f"java:invalid_pom:{exc}")
        pom = None
    if pom is not None:
        ns = {"m": "http://maven.apache.org/POM/4.0.0"}

        def text(name: str) -> str:
            found = pom.find(f"m:{name}", ns)
            return "" if found is None or found.text is None else found.text.strip()

        def prop(name: str) -> str:
            found = pom.find(f"m:properties/m:{name}", ns)
            return "" if found is None or found.text is None else found.text.strip()

        if text("groupId") != "run.runtime":
            fail("java:groupId_must_be_run.runtime")
        if text("artifactId") != "canonical-runtime-sdk":
            fail("java:artifactId_must_be_canonical-runtime-sdk")
        if text("version") != "0.0.0-seam":
            fail("java:version_must_be_0.0.0-seam")
        if text("packaging") != "jar":
            fail("java:packaging_must_be_jar")
        if prop("maven.compiler.release") != "17":
            fail("java:maven.compiler.release_must_be_17")

swift_text = require_text("sdk/swift/Package.swift", 'name: "RuntimeSDK"', "swift")
if swift_text:
    for required in [
        'name: "RuntimeSDK"',
        'targets: ["RuntimeSDK"]',
        'name: "RuntimeSDKTests"',
        'dependencies: ["RuntimeSDK"]',
    ]:
        if required not in swift_text:
            fail(f"swift:missing:{required}")

readme = require_text("sdk/README.md", "Package metadata", "sdk-readme")
if readme and "not stable release evidence" not in readme:
    fail("sdk-readme:must_distinguish_package_metadata_from_release_stability")

parity = require_text("sdk/SDK_PARITY.md", "Package metadata is machine-checked", "sdk-parity")
if parity and not re.search(r"publish/release stability.*remain incomplete", parity, re.S):
    fail("sdk-parity:must_keep_publish_release_stability_incomplete")

if failures:
    raise SystemExit("check-sdk-package-metadata failed:\n - " + "\n - ".join(failures))

print("check-sdk-package-metadata ok")
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d "$REPO_ROOT/target/sdk-package-metadata.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT

  mkdir -p \
    "$tmp/sdk/go" \
    "$tmp/sdk/python" \
    "$tmp/sdk/node" \
    "$tmp/sdk/java" \
    "$tmp/sdk/swift" \
    "$tmp/sdk/conformance"
  cp "$REPO_ROOT/sdk/go/go.mod" "$tmp/sdk/go/go.mod"
  cp "$REPO_ROOT/sdk/python/pyproject.toml" "$tmp/sdk/python/pyproject.toml"
  cp "$REPO_ROOT/sdk/node/package.json" "$tmp/sdk/node/package.json"
  cp "$REPO_ROOT/sdk/java/pom.xml" "$tmp/sdk/java/pom.xml"
  cp "$REPO_ROOT/sdk/swift/Package.swift" "$tmp/sdk/swift/Package.swift"
  cp "$REPO_ROOT/sdk/README.md" "$tmp/sdk/README.md"
  cp "$REPO_ROOT/sdk/SDK_PARITY.md" "$tmp/sdk/SDK_PARITY.md"

  validate_root "$tmp" >/dev/null

  python3 - "$tmp/sdk/node/package.json" "$tmp/sdk/python/pyproject.toml" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path

node_path = Path(sys.argv[1])
node = json.loads(node_path.read_text(encoding="utf-8"))
node["private"] = False
node_path.write_text(json.dumps(node), encoding="utf-8")

py_path = Path(sys.argv[2])
py_path.write_text(
    py_path.read_text(encoding="utf-8").replace('name = "easynet-sdk"', 'name = "easyremote"'),
    encoding="utf-8",
)
PY

  if validate_root "$tmp" >"$tmp/out" 2>&1; then
    echo "self-test expected broken package metadata to fail" >&2
    exit 1
  fi
  grep -Fq "node:p1_seam_package_must_remain_private" "$tmp/out"
  grep -Fq "pyproject:project.name_must_be_easynet-sdk" "$tmp/out"

  echo "check-sdk-package-metadata self-test ok"
  exit 0
fi

validate_root "$REPO_ROOT"
