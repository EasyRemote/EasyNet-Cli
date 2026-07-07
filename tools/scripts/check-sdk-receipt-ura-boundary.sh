#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

fail() {
  echo "check-sdk-receipt-ura-boundary: $*" >&2
  exit 1
}

collect_files() {
  local root
  for root in \
    "include" \
    "src/ffi" \
    "sdk/go" \
    "sdk/python/easynet_sdk" \
    "sdk/node" \
    "sdk/java/src/main/java" \
    "sdk/swift/Sources"; do
    [[ -e "$ROOT/$root" ]] || continue
    if [[ -f "$ROOT/$root" ]]; then
      printf '%s\0' "$ROOT/$root"
      continue
    fi
    find "$ROOT/$root" \
      \( -path '*/target/*' \
        -o -path '*/__pycache__/*' \
        -o -path '*/sdk/go/internal/axonpb/*' \
        -o -path '*/sdk/python/easynet_sdk/_axon_pb/*' \) -prune \
      -o -type f \
        \( -name '*.h' -o -name '*.rs' -o -name '*.go' -o -name '*.py' \
           -o -name '*.js' -o -name '*.ts' -o -name '*.d.ts' \
           -o -name '*.java' -o -name '*.swift' \) \
        ! -name '*_test.go' \
        ! -name 'test_*.py' \
        ! -name '*.test.js' \
        -print0
  done
}

collect_literal_files() {
  local root
  for root in \
    "include" \
    "src" \
    "sdk" \
    "tests"; do
    [[ -e "$ROOT/$root" ]] || continue
    find "$ROOT/$root" \
      \( -path '*/target/*' \
        -o -path '*/__pycache__/*' \
        -o -path '*/node_modules/*' \
        -o -path '*/sdk/go/internal/axonpb/*' \
        -o -path '*/sdk/python/easynet_sdk/_axon_pb/*' \) -prune \
      -o -type f \
        \( -name '*.h' -o -name '*.rs' -o -name '*.go' -o -name '*.py' \
           -o -name '*.js' -o -name '*.ts' -o -name '*.d.ts' \
           -o -name '*.java' -o -name '*.swift' -o -name '*.json' \
           -o -name '*.yaml' -o -name '*.yml' \) \
        -print0
  done
}

scan_files() {
  local files_list="$1"
  python3 - "$ROOT" "$files_list" <<'PY'
from __future__ import annotations

import re
import sys
from pathlib import Path


root = Path(sys.argv[1])
files_list = Path(sys.argv[2])

identifier = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
builder_words = ("build", "builder", "construct", "constructor", "make", "from")
receipt_ura_pattern = re.compile(r"receipt.*ura")

violations: list[tuple[str, int, str]] = []


def strip_comments(text: str, suffix: str) -> str:
    if suffix in {".py"}:
        return "\n".join(line.split("#", 1)[0] for line in text.splitlines())
    text = re.sub(r"/\*.*?\*/", lambda m: "\n" * m.group(0).count("\n"), text, flags=re.S)
    return "\n".join(line.split("//", 1)[0] for line in text.splitlines())


def display_path(path: Path) -> str:
    try:
        return str(path.relative_to(root))
    except ValueError:
        return str(path)


def suspicious(name: str) -> bool:
    normalized = name.replace("_", "").lower()
    if receipt_ura_pattern.search(normalized) is None:
        return False
    return any(word in normalized for word in builder_words)


raw = files_list.read_bytes()
for item in raw.split(b"\0"):
    if not item:
        continue
    path = Path(item.decode())
    try:
        text = strip_comments(path.read_text(encoding="utf-8", errors="replace"), path.suffix)
    except OSError:
        continue
    for line_no, line in enumerate(text.splitlines(), start=1):
        for match in identifier.finditer(line):
            name = match.group(0)
            if suspicious(name):
                violations.append((display_path(path), line_no, name))

if violations:
    for path, line, name in violations:
        print(f"{path}:{line}: local_receipt_ura_builder: {name}", file=sys.stderr)
    raise SystemExit(1)
PY
}

scan_receipt_ura_literals() {
  local files_list="$1"
  python3 - "$ROOT" "$files_list" <<'PY'
from __future__ import annotations

import re
import sys
from pathlib import Path


root = Path(sys.argv[1])
files_list = Path(sys.argv[2])
ura_literal = re.compile(r"easynet:///r/[^\"'`\s,)\\}]+")
canonical_receipt = re.compile(
    r"^easynet:///r/[^/]+/resource/[^/]+/invocation/[^/]+/receipt$"
)
violations: list[tuple[str, int, str]] = []


def display_path(path: Path) -> str:
    try:
        return str(path.relative_to(root))
    except ValueError:
        return str(path)


def legacy_receipt_ura(ura: str, line: str) -> bool:
    if canonical_receipt.match(ura):
        return False
    if re.match(r"^easynet:///r/[^/]+/receipt(?:/|$)", ura):
        return True
    if "/resource/receipt." in ura:
        return True
    if "/resource/invocations" in ura:
        return True
    return False


raw = files_list.read_bytes()
for item in raw.split(b"\0"):
    if not item:
        continue
    path = Path(item.decode())
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        continue
    for line_no, line in enumerate(lines, start=1):
        for match in ura_literal.finditer(line):
            ura = match.group(0)
            if legacy_receipt_ura(ura, line):
                violations.append((display_path(path), line_no, ura))

if violations:
    for path, line, ura in violations:
        print(f"{path}:{line}: legacy_receipt_ura_literal: {ura}", file=sys.stderr)
    raise SystemExit(1)
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  good="$tmp/good.go"
  bad="$tmp/bad.go"
  legacy="$tmp/legacy.go"
  files="$tmp/files.list"
  cat >"$good" <<'EOF'
package good

type ReceiptRef struct {
	ReceiptURA string
}

func NewReceiptRefFromJSON() ReceiptRef {
	return ReceiptRef{}
}
EOF
  cat >"$bad" <<'EOF'
package bad

func BuildReceiptURA(owner string, invocationID string) string {
	return owner + invocationID
}
EOF
  printf '%s\0' "$good" >"$files"
  scan_files "$files"
  printf '%s\0' "$bad" >"$files"
  if scan_files "$files" >"$tmp/out" 2>&1; then
    echo "self-test expected local receipt URA builder to fail" >&2
    exit 1
  fi
  grep -Fq "local_receipt_ura_builder" "$tmp/out"
  cat >"$legacy" <<'EOF'
package legacy

var bad = map[string]string{"receipt_ura": "easynet:///r/acme/receipt/r1"}
EOF
  printf '%s\0' "$legacy" >"$files"
  if scan_receipt_ura_literals "$files" >"$tmp/out" 2>&1; then
    echo "self-test expected legacy receipt URA literal to fail" >&2
    exit 1
  fi
  grep -Fq "legacy_receipt_ura_literal" "$tmp/out"
  echo "check-sdk-receipt-ura-boundary self-test ok"
  exit 0
fi

tmp_files="$(mktemp)"
tmp_literal_files="$(mktemp)"
trap 'rm -f "$tmp_files" "$tmp_literal_files"' EXIT
collect_files >"$tmp_files"
if [[ -s "$tmp_files" ]]; then
  scan_files "$tmp_files"
fi
collect_literal_files >"$tmp_literal_files"
if [[ -s "$tmp_literal_files" ]]; then
  scan_receipt_ura_literals "$tmp_literal_files"
fi
echo "check-sdk-receipt-ura-boundary: ok"
