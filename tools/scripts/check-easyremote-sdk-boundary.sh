#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

run_audit() {
  local easyremote_root="$1"
  python3 - "$easyremote_root" <<'PY'
from __future__ import annotations

import ast
import sys
from pathlib import Path

root = Path(sys.argv[1]).resolve()
package = root / "easyremote"
violations: list[str] = []
if not (root / "pyproject.toml").is_file():
    violations.append("missing_pyproject")
if not package.is_dir():
    violations.append("missing_package")
else:
    if (package / "_sdk_profiles.py").exists():
        violations.append("retired_profile_bridge")
    forbidden_imports = {"easynet_axon", "grpc", "ctypes", "subprocess"}
    forbidden_attrs = ("MissionClient", "AdminClient", "DirectoryClient", "ReceiptClient", "DaemonProfileBridge")
    for path in sorted(package.rglob("*.py")):
        try:
            tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        except SyntaxError as exc:
            violations.append(f"syntax_error:{path}:{exc.lineno}")
            continue
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                for alias in node.names:
                    if alias.name.split(".", 1)[0] in forbidden_imports:
                        violations.append(f"raw_lower_layer_import:{path}:{alias.name}")
            elif isinstance(node, ast.ImportFrom) and node.module:
                if node.module.split(".", 1)[0] in forbidden_imports:
                    violations.append(f"raw_lower_layer_import:{path}:{node.module}")
            elif isinstance(node, ast.Attribute) and node.attr in forbidden_attrs:
                violations.append(f"product_sdk_type:{path}:{node.attr}")
            elif isinstance(node, ast.Constant) and isinstance(node.value, str):
                if node.value.startswith("easynet_abi_"):
                    violations.append(f"raw_c_abi_symbol:{path}")
if violations:
    print(f"EasyRemote SDK boundary violations in {root}:")
    print("\n".join(violations))
    raise SystemExit(1)
print(f"EasyRemote SDK boundary ok: {root}")
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  good="$tmp/good"; mkdir -p "$good/easyremote"
  printf '%s\n' '[project]' 'name = "easyremote"' >"$good/pyproject.toml"
  printf '%s\n' 'from easynet_sdk import RuntimeClient' >"$good/easyremote/client.py"
  run_audit "$good" >/dev/null
  bad="$tmp/bad"; mkdir -p "$bad/easyremote"
  printf '%s\n' '[project]' 'name = "easyremote"' >"$bad/pyproject.toml"
  printf '%s\n' 'import ctypes' >"$bad/easyremote/client.py"
  if run_audit "$bad" >"$tmp/bad.out" 2>&1; then
    echo "self-test expected forbidden fixture to fail" >&2; exit 1
  fi
  grep -Fq "raw_lower_layer_import" "$tmp/bad.out"
  echo "check-easyremote-sdk-boundary self-test ok"
  exit 0
fi

run_audit "${1:-$REPO_ROOT/../EasyRemote}"
