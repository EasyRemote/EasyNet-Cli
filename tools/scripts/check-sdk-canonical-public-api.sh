#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEFAULT_MANIFEST="$ROOT/sdk/conformance/canonical-public-api.json"
MANIFEST="${1:-$DEFAULT_MANIFEST}"
PYTHON_BIN="${PYTHON:-}"
GO_BIN="${GO:-}"

if [[ -z "$PYTHON_BIN" ]]; then
  if [[ -x "$ROOT/sdk/python/.venv/bin/python" ]]; then
    PYTHON_BIN="$ROOT/sdk/python/.venv/bin/python"
  else
    PYTHON_BIN="$(command -v python3)"
  fi
fi

if [[ -z "$GO_BIN" ]]; then
  if command -v go >/dev/null 2>&1; then
    GO_BIN="$(command -v go)"
  elif [[ -x /opt/homebrew/bin/go ]]; then
    GO_BIN=/opt/homebrew/bin/go
  elif [[ -x /usr/local/go/bin/go ]]; then
    GO_BIN=/usr/local/go/bin/go
  else
    echo "canonical-public-api: go tool not found" >&2
    exit 1
  fi
fi

run_check() {
local manifest="$1"

"$PYTHON_BIN" - "$manifest" <<'PY' || return 1
import json
import sys
from pathlib import Path

manifest = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
if manifest.get("schema_version") != 1:
    raise SystemExit("canonical-public-api: unsupported schema version")
languages = manifest.get("languages")
if not isinstance(languages, dict) or set(languages) != {"go", "python"}:
    raise SystemExit("canonical-public-api: expected exactly go and python")
for language, symbols in languages.items():
    if not isinstance(symbols, list) or not symbols or symbols != sorted(set(symbols)):
        raise SystemExit(f"canonical-public-api: {language} symbols must be non-empty, sorted and unique")
members = manifest.get("members", {})
if members:
    if not isinstance(members, dict) or set(members) != {"go", "python"}:
        raise SystemExit("canonical-public-api: members must be keyed by go and python")
    for language, values in members.items():
        if not isinstance(values, list) or values != sorted(set(values)):
            raise SystemExit(f"canonical-public-api: {language} members must be sorted and unique")
PY

while IFS= read -r symbol; do
  (cd "$ROOT/sdk/go" && "$GO_BIN" doc ".$symbol" >/dev/null) || {
    echo "canonical-public-api: missing Go symbol $symbol" >&2
    return 1
  }
done < <(jq -r '.languages.go[]' "$manifest")

PYTHONPATH="$ROOT/sdk/python:$ROOT/../EasyNet-Axon/sdk/python${PYTHONPATH:+:$PYTHONPATH}" "$PYTHON_BIN" - "$manifest" "$ROOT" <<'PY' || return 1
import json
import re
import sys
from pathlib import Path

import easynet_sdk

manifest = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
root = Path(sys.argv[2])
exports = set(getattr(easynet_sdk, "__all__", ()))
missing = [symbol for symbol in manifest["languages"]["python"] if symbol not in exports]
if missing:
    raise SystemExit("canonical-public-api: missing Python symbols: " + ", ".join(missing))
if manifest.get("complete_inventory"):
    extra = sorted(exports - set(manifest["languages"]["python"]))
    if extra:
        raise SystemExit("canonical-public-api: untracked Python exports: " + ", ".join(extra))

    go_symbols: set[str] = set()
    go_members: set[str] = set()
    ident = r"[A-Za-z_][A-Za-z0-9_]*"
    for path in sorted((root / "sdk/go").glob("*.go")):
        if path.name.endswith("_test.go"):
            continue
        text = path.read_text(encoding="utf-8")
        text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
        for match in re.finditer(rf"^func\s+({ident})\s*\(", text, flags=re.M):
            name = match.group(1)
            if name[:1].isupper():
                go_symbols.add(name)
        for match in re.finditer(rf"^func\s*\(\s*(?:{ident}\s+)?\*?({ident})\s*\)\s*({ident})\s*\(", text, flags=re.M):
            receiver, name = match.groups()
            if receiver[:1].isupper() and name[:1].isupper():
                go_members.add(f"{receiver}.{name}")
        for match in re.finditer(rf"^type\s+({ident})\b", text, flags=re.M):
            name = match.group(1)
            if name[:1].isupper():
                go_symbols.add(name)
        for kind in ("const", "var"):
            for block in re.finditer(rf"^{kind}\s*\((.*?)^\)", text, flags=re.M | re.S):
                for line in block.group(1).splitlines():
                    line = line.strip()
                    if not line or line.startswith("//"):
                        continue
                    match = re.match(rf"({ident})\b", line)
                    if match and match.group(1)[:1].isupper():
                        go_symbols.add(match.group(1))
            for match in re.finditer(rf"^{kind}\s+({ident})\b", text, flags=re.M):
                name = match.group(1)
                if name[:1].isupper():
                    go_symbols.add(name)
    manifest_go = set(manifest["languages"]["go"])
    if go_symbols != manifest_go:
        missing_go = sorted(go_symbols - manifest_go)
        stale_go = sorted(manifest_go - go_symbols)
        raise SystemExit(
            "canonical-public-api: Go inventory mismatch"
            + (": missing " + ", ".join(missing_go) if missing_go else "")
            + (": stale " + ", ".join(stale_go) if stale_go else "")
        )
    manifest_members = manifest.get("members", {})
    if manifest_members:
        expected_go_members = set(manifest_members.get("go", []))
        if go_members != expected_go_members:
            missing_members = sorted(go_members - expected_go_members)
            stale_members = sorted(expected_go_members - go_members)
            raise SystemExit(
                "canonical-public-api: Go member inventory mismatch"
                + (": missing " + ", ".join(missing_members) if missing_members else "")
                + (": stale " + ", ".join(stale_members) if stale_members else "")
            )
        expected_python_members = set(manifest_members.get("python", []))
        python_members = set()
        for symbol in exports:
            value = getattr(easynet_sdk, symbol, None)
            if isinstance(value, type):
                for member in vars(value):
                    if not member.startswith("_") and callable(getattr(value, member, None)):
                        python_members.add(f"{symbol}.{member}")
        if python_members != expected_python_members:
            missing_members = sorted(python_members - expected_python_members)
            stale_members = sorted(expected_python_members - python_members)
            raise SystemExit(
                "canonical-public-api: Python member inventory mismatch"
                + (": missing " + ", ".join(missing_members) if missing_members else "")
                + (": stale " + ", ".join(stale_members) if stale_members else "")
            )
PY

echo "canonical-public-api: OK"
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d "$ROOT/target/canonical-public-api.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT
  cp "$DEFAULT_MANIFEST" "$tmp/good.json"
  run_check "$tmp/good.json" >/dev/null
  "$PYTHON_BIN" - "$DEFAULT_MANIFEST" "$tmp/missing.json" "$tmp/extra.json" <<'PY'
import json
import sys
from pathlib import Path

source = json.loads(Path(sys.argv[1]).read_text())

missing = json.loads(json.dumps(source))
missing["languages"]["python"].remove("RuntimeClient")
Path(sys.argv[2]).write_text(json.dumps(missing))

extra = json.loads(json.dumps(source))
extra["languages"]["python"].append("NotARealExport")
extra["languages"]["python"] = sorted(set(extra["languages"]["python"]))
Path(sys.argv[3]).write_text(json.dumps(extra))
PY
  if run_check "$tmp/missing.json" >"$tmp/missing.out" 2>&1; then
    echo "canonical-public-api self-test expected missing inventory fixture to fail" >&2
    exit 1
  fi
  grep -Fq "untracked Python exports" "$tmp/missing.out"
  if run_check "$tmp/extra.json" >"$tmp/extra.out" 2>&1; then
    echo "canonical-public-api self-test expected stale export fixture to fail" >&2
    exit 1
  fi
  grep -Fq "missing Python symbols" "$tmp/extra.out"
  echo "check-sdk-canonical-public-api self-test ok"
  exit 0
fi

run_check "$MANIFEST"
