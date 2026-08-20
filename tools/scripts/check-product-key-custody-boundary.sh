#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

run_audit() {
  local backend_root="$1"
  local easyremote_root="$2"
  python3 - "$backend_root" "$easyremote_root" <<'PY'
from __future__ import annotations

import ast
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


backend = resolve_backend_root(Path(sys.argv[1]))
easyremote = Path(sys.argv[2]).resolve()
violations: list[str] = []


def strip_go_comments(text: str) -> str:
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
                continue
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


def backend_production_go_files(root: Path):
    roots = [root / "easynet.go", root / "internal"]
    ignored_parts = {
        ".git",
        "vendor",
        "testdata",
        "axontest",
        "testsigner",
        "sdktest",
    }
    for scan_root in roots:
        if scan_root.is_file():
            yield scan_root
            continue
        if not scan_root.exists():
            continue
        for path in scan_root.rglob("*.go"):
            rel_parts = set(path.relative_to(root).parts)
            if rel_parts & ignored_parts:
                continue
            if path.name.endswith("_test.go"):
                continue
            yield path


def check_backend() -> None:
    if not backend.exists() or not (backend / "go.mod").exists():
        violations.append(f"backend:missing_go_mod:{backend}")
        return
    forbidden_regexes = [
        ("runtime_private_key_type", r"\bed25519\.PrivateKey\b"),
        ("runtime_private_key_generation", r"\bed25519\.GenerateKey\s*\("),
        ("runtime_seed_key_constructor", r"\bNewKeyFromSeed\s*\("),
        ("runtime_private_seed_field", r"\b(privateKeySeed|private_key_seed|SigningSeed|signing_seed|seed_hex)\b"),
        ("key_service_passphrase_escape", r"\bEASYNET_KEYRING_PASSPHRASE\b"),
        ("key_service_vault_escape", r"\bEASYNET_KEYRING_VAULT_PATH\b"),
        ("raw_c_abi_marker", r"\b(libeasynet_cli|dlopen)\b"),
        ("runtime_process_spawn", r"\bexec\.Command\s*\(\s*\"(?:easynet|easynet-daemon)\""),
    ]
    for source in backend_production_go_files(backend):
        rel = source.relative_to(backend).as_posix()
        text = source.read_text(encoding="utf-8", errors="replace")
        code = strip_go_comments(text)
        for line, imported in go_imports(code):
            if imported == "C":
                violations.append(f"backend:{rel}:{line}:cgo_ffi_import")
            if imported == "os/exec":
                violations.append(f"backend:{rel}:{line}:process_exec_import")
        for rule, pattern in forbidden_regexes:
            match = re.search(pattern, code)
            if match:
                line = code[: match.start()].count("\n") + 1
                violations.append(f"backend:{rel}:{line}:{rule}")


def easyremote_python_files(root: Path):
    package = root / "easyremote"
    if not package.is_dir():
        violations.append(f"easyremote:missing_package:{root}")
        return
    for path in sorted(package.rglob("*.py")):
        if "__pycache__" in path.parts:
            continue
        yield path


def attribute_name(node: ast.AST) -> str:
    if isinstance(node, ast.Name):
        return node.id
    if isinstance(node, ast.Attribute):
        parent = attribute_name(node.value)
        return f"{parent}.{node.attr}" if parent else node.attr
    return ""


def check_easyremote() -> None:
    if not easyremote.exists() or not (easyremote / "pyproject.toml").exists():
        violations.append(f"easyremote:missing_pyproject:{easyremote}")
        return
    forbidden_import_roots = {"ctypes", "subprocess", "multiprocessing"}
    forbidden_name_re = re.compile(
        r"(Ed25519PrivateKey|from_private_bytes|private_key_seed|signing_seed|seed_hex)"
    )
    for source in easyremote_python_files(easyremote):
        rel = source.relative_to(easyremote).as_posix()
        try:
            tree = ast.parse(source.read_text(encoding="utf-8"), filename=str(source))
        except SyntaxError as exc:
            violations.append(f"easyremote:{rel}:{exc.lineno}:syntax_error")
            continue
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                for alias in node.names:
                    if alias.name.split(".", 1)[0] in forbidden_import_roots:
                        violations.append(f"easyremote:{rel}:{node.lineno}:raw_process_or_ffi_import:{alias.name}")
            elif isinstance(node, ast.ImportFrom) and node.module:
                if node.module.split(".", 1)[0] in forbidden_import_roots:
                    violations.append(f"easyremote:{rel}:{node.lineno}:raw_process_or_ffi_import:{node.module}")
            elif isinstance(node, ast.Call):
                called = attribute_name(node.func)
                if called in {"os.system", "subprocess.run", "subprocess.Popen", "subprocess.call"}:
                    violations.append(f"easyremote:{rel}:{node.lineno}:runtime_process_spawn:{called}")
                if called.endswith(".generate_private_key") and rel != "easyremote/gateway.py":
                    violations.append(f"easyremote:{rel}:{node.lineno}:runtime_private_key_generation:{called}")
                if called.endswith(".from_private_bytes"):
                    violations.append(f"easyremote:{rel}:{node.lineno}:runtime_private_key_material:{called}")
            elif isinstance(node, ast.Attribute):
                if forbidden_name_re.search(node.attr):
                    violations.append(f"easyremote:{rel}:{node.lineno}:runtime_private_key_material:{node.attr}")
            elif isinstance(node, ast.Name):
                if forbidden_name_re.search(node.id):
                    violations.append(f"easyremote:{rel}:{node.lineno}:runtime_private_key_material:{node.id}")
            elif isinstance(node, ast.Constant) and isinstance(node.value, str):
                value = node.value
                if value.startswith("easynet_abi_") or "libeasynet_cli" in value or "dlopen" in value:
                    violations.append(f"easyremote:{rel}:{node.lineno}:raw_c_abi_marker")
                if "EASYNET_KEYRING_PASSPHRASE" in value or "EASYNET_KEYRING_VAULT_PATH" in value:
                    violations.append(f"easyremote:{rel}:{node.lineno}:key_service_secret_escape")


check_backend()
check_easyremote()

if violations:
    print("product key-custody boundary violations:")
    for violation in violations:
        print(violation)
    raise SystemExit(1)

print("product key-custody boundary ok")
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  good_backend="$tmp/EasyNet/backend"
  good_remote="$tmp/EasyRemote"
  mkdir -p "$good_backend/internal/service" "$good_remote/easyremote"
  printf '%s\n' 'module easynet-backend' >"$good_backend/go.mod"
  cat >"$good_backend/internal/service/service.go" <<'EOF'
package service

type PublicProjection struct {
	PublicKeyBase64 string
}
EOF
  printf '%s\n' '[project]' 'name = "easyremote"' >"$good_remote/pyproject.toml"
  cat >"$good_remote/easyremote/client.py" <<'EOF'
import easynet_sdk

RuntimeClient = easynet_sdk.RuntimeClient
EOF
  run_audit "$good_backend" "$good_remote" >/dev/null

  bad_backend="$tmp/BadEasyNet/backend"
  bad_remote="$tmp/BadEasyRemote"
  mkdir -p "$bad_backend/internal/service" "$bad_remote/easyremote"
  printf '%s\n' 'module easynet-backend' >"$bad_backend/go.mod"
  cat >"$bad_backend/internal/service/service.go" <<'EOF'
package service

import (
    "crypto/ed25519"
    "os/exec"
)

func boot(privateKey ed25519.PrivateKey) {
    _ = exec.Command("easynet-daemon")
    _ = privateKey
}
EOF
  printf '%s\n' '[project]' 'name = "easyremote"' >"$bad_remote/pyproject.toml"
  cat >"$bad_remote/easyremote/client.py" <<'EOF'
import subprocess

def boot():
    subprocess.Popen(["easynet-daemon"])
EOF
  if run_audit "$bad_backend" "$bad_remote" >"$tmp/bad.out" 2>&1; then
    echo "self-test expected forbidden product custody fixture to fail" >&2
    exit 1
  fi
  grep -Fq "runtime_private_key_type" "$tmp/bad.out"
  grep -Fq "runtime_process_spawn" "$tmp/bad.out"
  grep -Fq "raw_process_or_ffi_import" "$tmp/bad.out"
  echo "check-product-key-custody-boundary self-test ok"
  exit 0
fi

BACKEND_ROOT="${EASYNET_BACKEND_ROOT:-$REPO_ROOT/../EasyNet/backend}"
EASYREMOTE_ROOT="${EASYNET_EASYREMOTE_ROOT:-$REPO_ROOT/../EasyRemote}"
run_audit "${1:-$BACKEND_ROOT}" "${2:-$EASYREMOTE_ROOT}"
