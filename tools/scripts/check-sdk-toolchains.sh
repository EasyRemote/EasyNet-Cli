#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CONTRACT="${SDK_TOOLCHAIN_CONTRACT:-$ROOT/sdk/conformance/toolchains.json}"

validate_contract() {
  python3 - "$1" <<'PY'
import json, sys
from pathlib import Path
contract = json.loads(Path(sys.argv[1]).read_text())
if contract.get("schema_version") != 1:
    raise SystemExit("toolchains: invalid_schema")
expected = {"cargo", "go", "java", "maven", "node", "python", "rust", "rustdoc_nightly", "swift", "typescript"}
if set(contract.get("toolchains", {})) != expected:
    raise SystemExit("toolchains: incomplete_toolchain_set")
if set(contract.get("python_tools", {})) != {"mypy", "pytest", "ruff"}:
    raise SystemExit("toolchains: incomplete_python_tool_set")
for group in ("toolchains", "python_tools"):
    for name, version in contract[group].items():
        if not isinstance(version, str) or not version or version == "latest" or version.startswith(">="):
            raise SystemExit(f"toolchains: unpinned:{name}")
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d "${TMPDIR:-/tmp}/sdk-toolchains.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  validate_contract "$CONTRACT"
  python3 - "$CONTRACT" "$tmp/unpinned.json" <<'PY'
import json, sys
from pathlib import Path
value = json.loads(Path(sys.argv[1]).read_text())
value["toolchains"]["node"] = "latest"
Path(sys.argv[2]).write_text(json.dumps(value))
PY
  if validate_contract "$tmp/unpinned.json" >"$tmp/out" 2>&1; then
    echo "toolchains self-test accepted an unpinned version" >&2
    exit 1
  fi
  grep -Fq 'unpinned:node' "$tmp/out"
  echo "check-sdk-toolchains self-test ok"
  exit 0
fi

validate_contract "$CONTRACT"
eval "$(python3 - "$CONTRACT" <<'PY'
import json, shlex, sys
value = json.load(open(sys.argv[1]))
for group in ("toolchains", "python_tools"):
    for name, version in value[group].items():
        print(f"expected_{name}={shlex.quote(version)}")
PY
)"

actual_rust="$(rustc --version | awk '{print $2}')"
actual_cargo="$(cargo --version | awk '{print $2}')"
actual_go="$(go version | sed -E 's/.* go([0-9.]+) .*/\1/')"
actual_python="$(python -c 'import platform; print(platform.python_version())')"
actual_node="$(node --version | sed 's/^v//')"
actual_java="$(java -version 2>&1 | sed -nE '1s/.*version "([0-9.]+)".*/\1/p')"
actual_maven="$(mvn -version | sed -nE '1s/^Apache Maven ([0-9.]+).*/\1/p')"
actual_swift="$(swift --version | sed -nE '1s/.*Apple Swift version ([0-9.]+).*/\1/p')"
actual_pytest="$(pytest --version | awk '{print $2}')"
actual_ruff="$(ruff --version | awk '{print $2}')"
actual_mypy="$(mypy --version | awk '{print $2}')"
actual_rustdoc_nightly="nightly-$(rustc +nightly --version --verbose | sed -nE 's/^commit-date: //p')"
actual_typescript="$(sdk/node/node_modules/.bin/tsc --version | awk '{print $2}')"

for name in rust rustdoc_nightly cargo go python node java maven swift typescript pytest ruff mypy; do
  eval "actual=\$actual_$name"
  eval "expected=\$expected_$name"
  [[ "$actual" == "$expected" ]] || {
    echo "toolchains: version_mismatch:$name:expected=$expected:actual=$actual" >&2
    exit 1
  }
done

echo "check-sdk-toolchains ok"
