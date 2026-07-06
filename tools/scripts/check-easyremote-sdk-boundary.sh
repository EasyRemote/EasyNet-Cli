#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

run_audit() {
  local easyremote_root="$1"
  python3 - "$REPO_ROOT" "$easyremote_root" <<'PY'
from __future__ import annotations

import importlib.util
import sys
from pathlib import Path


repo = Path(sys.argv[1]).resolve()
easyremote = Path(sys.argv[2]).resolve()
auditor_path = repo / "sdk/python/easynet_sdk/consumer_boundary.py"

if not easyremote.exists():
    print(f"easyremote root does not exist: {easyremote}", file=sys.stderr)
    raise SystemExit(2)
if not (easyremote / "pyproject.toml").exists():
    print(f"easyremote root is missing pyproject.toml: {easyremote}", file=sys.stderr)
    raise SystemExit(2)
if not (easyremote / "easyremote").is_dir():
    print(f"easyremote root is missing package directory: {easyremote}", file=sys.stderr)
    raise SystemExit(2)
if not auditor_path.exists():
    print(f"consumer boundary auditor not found: {auditor_path}", file=sys.stderr)
    raise SystemExit(2)

spec = importlib.util.spec_from_file_location(
    "easynet_sdk_consumer_boundary",
    auditor_path,
)
if spec is None or spec.loader is None:
    print(f"failed to load consumer boundary auditor: {auditor_path}", file=sys.stderr)
    raise SystemExit(2)
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)

result = module.audit_consumer_boundary(easyremote)
if not result.ok:
    print(f"EasyRemote SDK boundary violations in {easyremote}:")
    for violation in result.violations:
        line = violation.line or 1
        print(f"{violation.path}:{line}: {violation.rule}: {violation.detail}")
    raise SystemExit(1)

print(f"EasyRemote SDK boundary ok: {easyremote}")
PY
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT

  good="$tmp/EasyRemoteGood"
  mkdir -p "$good/easyremote"
  cat >"$good/pyproject.toml" <<'EOF'
[project]
name = "easyremote"
dependencies = ["easynet-sdk>=0.91.30"]
EOF
  cat >"$good/easyremote/client.py" <<'EOF'
from easynet_sdk import AbilityInvocationClient, InvocationDraft, ReceiptClient


def invoke(client: AbilityInvocationClient, draft: InvocationDraft):
    return client.invoke(draft)
EOF
  run_audit "$good" >/dev/null

  bad="$tmp/EasyRemoteBad"
  mkdir -p "$bad/easyremote/_transport"
  cat >"$bad/pyproject.toml" <<'EOF'
[project]
name = "easyremote"
dependencies = [
  "easynet-sdk>=0.91.30",
  "easynet-run-axon>=0.4",
]
EOF
  cat >"$bad/easyremote/invocation.py" <<'EOF'
import ctypes
import json
from easynet_axon import parse_ura

lib = ctypes.CDLL("libeasynet_cli.dylib")
symbol = "easynet_invocation_invoke"
raw = json.dumps({
    "caller_ura": "easynet:///r/example/agent/alice",
    "callee_ura": "easynet:///r/example/device/dev-a",
    "descriptor_ref": "easynet:///r/example/ability/device.dev-a.er.weather@1.0.0",
    "subject_ura": "easynet:///r/example/device/dev-a",
    "nonce_base64": "AQIDBAUGBwgJCgsMDQ4PEA==",
    "causal_context": {"form": "none"},
})
EOF
  cat >"$bad/easyremote/_transport/abi.py" <<'EOF'
class LegacyTransport:
    pass
EOF
  if run_audit "$bad" >"$tmp/bad.out" 2>&1; then
    echo "self-test expected forbidden EasyRemote fixture to fail" >&2
    exit 1
  fi
  grep -Fq "raw_lower_layer_dependency" "$tmp/bad.out"
  grep -Fq "raw_lower_layer_import" "$tmp/bad.out"
  grep -Fq "raw_ffi_loader" "$tmp/bad.out"
  grep -Fq "raw_c_abi_symbol" "$tmp/bad.out"
  grep -Fq "raw_invocation_json_codec" "$tmp/bad.out"
  grep -Fq "raw_transport_module" "$tmp/bad.out"

  echo "check-easyremote-sdk-boundary self-test ok"
  exit 0
fi

EASYREMOTE_ROOT="${1:-${EASYNET_EASYREMOTE_ROOT:-$REPO_ROOT/../EasyRemote}}"
run_audit "$EASYREMOTE_ROOT"
