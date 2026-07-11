#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/backend/internal/service" "$tmp/backend/internal/axon" "$tmp/backend/internal/pb/axon/v1" "$tmp/backend/internal/daemon_grpc"
  cat >"$tmp/backend/go.mod" <<'EOF'
module easynet-backend
EOF
  cat >"$tmp/backend/internal/service/allowed.go" <<'EOF'
package service

import sdk "easynet.run/cli/sdk/go"

var _ = sdk.ErrInvalidArgument
EOF
  "$0" "$tmp" >/dev/null
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
    _ = "unix:///tmp/easynet-control.sock"
}
EOF
  cat >"$tmp/backend/internal/axon/resolve_legacy.go" <<'EOF'
package axon

type legacyResolveInput struct {
    QueryName string `json:"queryName"`
}

var _ = `{"answerKind":"RESOLVE_ANSWER_KIND_FINAL_ROUTE"}`
EOF
  cat >"$tmp/backend/internal/pb/axon/v1/generated.go" <<'EOF'
package v1

type Invocation struct{}
EOF
  cat >"$tmp/backend/internal/daemon_grpc/client.go" <<'EOF'
package daemon_grpc

type Client struct{}
EOF
  cat >>"$tmp/backend/go.mod" <<'EOF'
require easynet.run/axon/sdk/go v0.0.0
EOF
  self_test_out="$tmp/backend-sdk-only-boundary-self-test.out"
  if "$0" "$tmp/backend" >"$self_test_out" 2>&1; then
    echo "self-test expected forbidden backend fixture to fail" >&2
    exit 1
  fi
  grep -Fq "raw_axon_module_dependency" "$self_test_out"
  grep -Fq "raw_axon_import" "$self_test_out"
  grep -Fq "generated_axon_pb_import" "$self_test_out"
  grep -Fq "generated_axon_pb_package" "$self_test_out"
  grep -Fq "direct_daemon_transport_import" "$self_test_out"
  grep -Fq "direct_daemon_transport_package" "$self_test_out"
  grep -Fq "raw_daemon_socket_marker" "$self_test_out"
  grep -Fq "runtime_subprocess" "$self_test_out"
  grep -Fq "retired_namespace_resolve_carrier_key" "$self_test_out"
  echo "check-backend-sdk-only-boundary self-test ok"
  exit 0
fi

BACKEND_ROOT="${1:-${EASYNET_BACKEND_ROOT:-$REPO_ROOT/../EasyNet/backend}}"

python3 - "$BACKEND_ROOT" <<'PY'
from __future__ import annotations

import re
import sys
from pathlib import Path


def resolve_backend_root(candidate: Path) -> Path:
    candidate = candidate.resolve()
    if (candidate / "go.mod").exists():
        return candidate
    nested = candidate / "backend"
    if (nested / "go.mod").exists():
        return nested
    return candidate


input_root = Path(sys.argv[1]).resolve()
backend = resolve_backend_root(input_root)
if not backend.exists():
    print(f"backend root does not exist: {input_root}", file=sys.stderr)
    sys.exit(2)
if not (backend / "go.mod").exists():
    print(f"backend root is missing go.mod: {input_root}", file=sys.stderr)
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
RAW_DAEMON_SOCKET_MARKERS = (
    "control.sock",
    "daemon.sock",
    "easynet-control.sock",
    "easynet-daemon.sock",
    "unix:///tmp/easynet",
)
RUNTIME_SUBPROCESS_TARGETS = {"easynet", "easynet-daemon"}
RETIRED_NAMESPACE_RESOLVE_CARRIER_KEYS = (
    "queryName",
    "abilityName",
    "realmHint",
    "callerUra",
    "subjectUra",
    "answerKind",
    "canonicalName",
    "ownerUra",
    "abilityUra",
    "routeUra",
    "nextHop",
    "selectedRoute",
    "routeCandidates",
    "routeEvidence",
    "releaseProfile",
    "cachePolicy",
    "recordType",
    "ttlMs",
    "expiresUnixMs",
    "localDeviceAbility",
    "hostedAgentViaDevice",
    "localHubAbility",
    "dispatchName",
    "deviceUra",
    "peerHub",
    "hubUra",
    "noRoute",
    "hostedBy",
    "hostedUra",
    "hostUra",
    "targetUra",
    "executeOn",
    "retryAfterUnixMs",
    "sharedCacheable",
)
NAMESPACE_RESOLVE_CARRIER_PREFIXES = (
    "internal/axon/",
    "internal/axontest/",
    "internal/federation/",
    "internal/catalog/",
    "internal/svc/",
    "internal/logic/ability/",
    "internal/logic/agent/",
    "internal/logic/skill/",
)


def rel(path: Path) -> str:
    return str(path.relative_to(backend))


def production_go_files(root: Path):
    for path in root.rglob("*.go"):
        parts = set(path.relative_to(root).parts)
        if parts & ignored_dirs:
            continue
        if path.name.endswith("_test.go"):
            continue
        yield path


def namespace_resolve_carrier_go_files(root: Path):
    for path in root.rglob("*.go"):
        parts = set(path.relative_to(root).parts)
        if parts & ignored_dirs:
            continue
        relative = str(path.relative_to(root))
        if relative.startswith(NAMESPACE_RESOLVE_CARRIER_PREFIXES):
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


def go_string_literals(text: str) -> list[tuple[int, str]]:
    strings: list[tuple[int, str]] = []
    index = 0
    line = 1
    in_block_comment = False
    while index < len(text):
        char = text[index]
        next_char = text[index + 1] if index + 1 < len(text) else ""
        if char == "\n":
            line += 1
            index += 1
            continue
        if in_block_comment:
            if char == "*" and next_char == "/":
                in_block_comment = False
                index += 2
            else:
                index += 1
            continue
        if char == "/" and next_char == "/":
            newline = text.find("\n", index + 2)
            if newline == -1:
                break
            index = newline
            continue
        if char == "/" and next_char == "*":
            in_block_comment = True
            index += 2
            continue
        if char == '"':
            start_line = line
            index += 1
            value: list[str] = []
            while index < len(text):
                char = text[index]
                if char == "\n":
                    line += 1
                if char == "\\" and index + 1 < len(text):
                    value.append(text[index + 1])
                    index += 2
                    continue
                if char == '"':
                    index += 1
                    break
                value.append(char)
                index += 1
            strings.append((start_line, "".join(value)))
            continue
        if char == "`":
            start_line = line
            index += 1
            value = []
            while index < len(text):
                char = text[index]
                if char == "`":
                    index += 1
                    break
                if char == "\n":
                    line += 1
                value.append(char)
                index += 1
            strings.append((start_line, "".join(value)))
            continue
        index += 1
    return strings


def go_code_without_comments(text: str) -> str:
    output: list[str] = []
    index = 0
    in_block_comment = False
    while index < len(text):
        char = text[index]
        next_char = text[index + 1] if index + 1 < len(text) else ""
        if in_block_comment:
            if char == "*" and next_char == "/":
                in_block_comment = False
                index += 2
            else:
                output.append("\n" if char == "\n" else " ")
                index += 1
            continue
        if char == "/" and next_char == "/":
            newline = text.find("\n", index + 2)
            if newline == -1:
                output.extend(" " for _ in text[index:])
                break
            output.extend(" " for _ in text[index:newline])
            index = newline
            continue
        if char == "/" and next_char == "*":
            output.extend("  ")
            in_block_comment = True
            index += 2
            continue
        output.append(char)
        index += 1
    return "".join(output)


def scan_go_mod(path: Path) -> None:
    in_require_block = False
    for index, raw_line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), start=1):
        stripped = raw_line.strip()
        if not stripped or stripped.startswith("//"):
            continue
        if in_require_block:
            if stripped == ")":
                in_require_block = False
                continue
            module = stripped.split()[0] if stripped.split() else ""
            if module.startswith("easynet.run/axon") and "// indirect" not in raw_line:
                violations.append(("go.mod", index, "raw_axon_module_dependency", module))
            continue
        if stripped == "require (":
            in_require_block = True
            continue
        if stripped.startswith("require "):
            fields = stripped.split()
            if len(fields) >= 2:
                module = fields[1]
                if module.startswith("easynet.run/axon") and "// indirect" not in raw_line:
                    violations.append(("go.mod", index, "raw_axon_module_dependency", module))


def retired_namespace_resolve_keys(literal: str) -> list[str]:
    retired: list[str] = []
    for key in RETIRED_NAMESPACE_RESOLVE_CARRIER_KEYS:
        if literal == key:
            retired.append(key)
            continue
        if re.search(r'\bjson:"' + re.escape(key) + r'(?:,|")', literal):
            retired.append(key)
            continue
        if re.search(r'"' + re.escape(key) + r'"\s*:', literal):
            retired.append(key)
    return retired


scan_go_mod(backend / "go.mod")

for source in production_go_files(backend):
    text = source.read_text(encoding="utf-8", errors="replace")
    relative = rel(source)
    normalized = "/" + relative
    imports = go_imports(text)
    string_literals = go_string_literals(text)
    code_without_comments = go_code_without_comments(text)
    if "/internal/pb/axon/v1/" in normalized:
        violations.append((relative, 1, "generated_axon_pb_package", "internal/pb/axon/v1"))
    if "/internal/daemon_grpc/" in normalized:
        violations.append((relative, 1, "direct_daemon_transport_package", "internal/daemon_grpc"))
    for line, imported in imports:
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
    for line, literal in string_literals:
        if "libeasynet_cli" in literal or "dlopen" in literal:
            violations.append((relative, line, "raw_c_abi_marker", "libeasynet_cli/dlopen"))
        for marker in RAW_DAEMON_SOCKET_MARKERS:
            if marker in literal:
                violations.append((relative, line, "raw_daemon_socket_marker", marker))
    if "exec.Command(" in code_without_comments and any(
        literal in RUNTIME_SUBPROCESS_TARGETS for _, literal in string_literals
    ):
        violations.append((relative, 1, "runtime_subprocess", "exec.Command easynet/easynet-daemon"))
    if relative == "cmd/seed-dev/main.go" and (
        "NewKeyFromSeed" in code_without_comments
        or "ed25519.PrivateKey" in code_without_comments
    ):
        violations.append((relative, 1, "local_key_custody", "seed-dev must use the daemon key-service SDK"))

for source in namespace_resolve_carrier_go_files(backend):
    text = source.read_text(encoding="utf-8", errors="replace")
    relative = rel(source)
    for line, literal in go_string_literals(text):
        for key in retired_namespace_resolve_keys(literal):
            violations.append((relative, line, "retired_namespace_resolve_carrier_key", key))

if violations:
    print(f"backend SDK-only boundary violations in {backend}:")
    for path, line, rule, detail in sorted(violations):
        print(f"{path}:{line}: {rule}: {detail}")
    sys.exit(1)

print(f"backend SDK-only boundary ok: {backend}")
PY
