#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/backend/internal/service" "$tmp/backend/internal/pb/axon/v1" "$tmp/backend/internal/daemon_grpc"
  cat >"$tmp/backend/go.mod" <<'EOF'
module easynet-backend
EOF
  cat >"$tmp/backend/internal/service/allowed.go" <<'EOF'
package service

import sdk "easynet.run/cli/sdk/go"

var _ = sdk.ErrInvalidArgument
EOF
  "$0" "$tmp/backend" >/dev/null
  cat >"$tmp/backend/internal/service/forbidden.go" <<'EOF'
package service

import (
    "os/exec"

    daemon "easynet-backend/internal/daemon_grpc"
    pb "easynet-backend/internal/pb/axon/v1"
    axonsdk "easynet.run/axon/sdk/go/easynet"
)

var _ = daemon.Client{}
var _ = pb.InvokeRequest{}
var _ = axonsdk.ErrInvalidArgument

func boot() {
    _ = exec.Command("easynet-daemon")
}
EOF
  if "$0" "$tmp/backend" >/tmp/backend-sdk-only-boundary-self-test.out 2>&1; then
    echo "self-test expected forbidden backend fixture to fail" >&2
    exit 1
  fi
  grep -Fq "raw_axon_import" /tmp/backend-sdk-only-boundary-self-test.out
  grep -Fq "generated_axon_pb_import" /tmp/backend-sdk-only-boundary-self-test.out
  grep -Fq "direct_daemon_transport_import" /tmp/backend-sdk-only-boundary-self-test.out
  grep -Fq "runtime_subprocess" /tmp/backend-sdk-only-boundary-self-test.out
  rm -f /tmp/backend-sdk-only-boundary-self-test.out
  echo "check-backend-sdk-only-boundary self-test ok"
  exit 0
fi

BACKEND_ROOT="${1:-${EASYNET_BACKEND_ROOT:-$REPO_ROOT/../EasyNet/backend}}"

python3 - "$BACKEND_ROOT" <<'PY'
from __future__ import annotations

import re
import sys
from pathlib import Path


backend = Path(sys.argv[1]).resolve()
if not backend.exists():
    print(f"backend root does not exist: {backend}", file=sys.stderr)
    sys.exit(2)
if not (backend / "go.mod").exists():
    print(f"backend root is missing go.mod: {backend}", file=sys.stderr)
    sys.exit(2)

ignored_dirs = {
    ".git",
    "node_modules",
    "vendor",
    "tmp",
    "dist",
    "build",
}

violations: list[tuple[str, int, str, str]] = []


def rel(path: Path) -> str:
    return str(path.relative_to(backend))


def production_go_files(root: Path):
    for path in root.rglob("*.go"):
        parts = set(path.relative_to(root).parts)
        if parts & ignored_dirs:
            continue
        if path.name.endswith("_test.go"):
            continue
        if "/internal/pb/axon/v1/" in "/" + rel(path):
            continue
        yield path


def go_imports(text: str) -> list[tuple[int, str]]:
    imports: list[tuple[int, str]] = []
    lines = text.splitlines()
    in_block = False
    for index, line in enumerate(lines, start=1):
        stripped = line.strip()
        if not in_block and stripped.startswith("import "):
            rest = stripped[len("import ") :].strip()
            if rest == "(":
                in_block = True
                continue
            imports.extend((index, item) for item in re.findall(r'"([^"]+)"', rest))
            continue
        if in_block:
            if stripped == ")":
                in_block = False
                continue
            imports.extend((index, item) for item in re.findall(r'"([^"]+)"', stripped))
    return imports


for source in production_go_files(backend):
    text = source.read_text(encoding="utf-8", errors="replace")
    relative = rel(source)
    normalized = "/" + relative
    if "/internal/daemon_grpc/" in normalized:
        violations.append((relative, 1, "direct_daemon_transport_package", "internal/daemon_grpc"))
    for line, imported in go_imports(text):
        if imported == "C":
            violations.append((relative, line, "cgo_ffi_import", imported))
        if imported.startswith("easynet.run/axon"):
            violations.append((relative, line, "raw_axon_import", imported))
        if imported.endswith("/internal/pb/axon/v1") or "/internal/pb/axon/v1" in imported:
            violations.append((relative, line, "generated_axon_pb_import", imported))
        if imported.endswith("/internal/daemon_grpc") or "/internal/daemon_grpc" in imported:
            violations.append((relative, line, "direct_daemon_transport_import", imported))
        if "EasyRemote" in imported or "easyremote" in imported:
            violations.append((relative, line, "easyremote_runtime_dependency", imported))
    if "libeasynet_cli" in text or "dlopen" in text:
        violations.append((relative, 1, "raw_c_abi_marker", "libeasynet_cli/dlopen"))
    if "exec.Command(" in text and ("easynet-daemon" in text or '"easynet"' in text):
        violations.append((relative, 1, "runtime_subprocess", "exec.Command easynet/easynet-daemon"))

if violations:
    print(f"backend SDK-only boundary violations in {backend}:")
    for path, line, rule, detail in sorted(violations):
        print(f"{path}:{line}: {rule}: {detail}")
    sys.exit(1)

print(f"backend SDK-only boundary ok: {backend}")
PY
