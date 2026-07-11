#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
CARGO_BIN="${CARGO:-cargo}"

if ! command -v "$CARGO_BIN" >/dev/null 2>&1; then
  if [[ -x "$HOME/.cargo/bin/cargo" ]]; then
    CARGO_BIN="$HOME/.cargo/bin/cargo"
  fi
fi

run_report() {
  local language="$1"
  local report="$2"
  "$CARGO_BIN" run --quiet --bin sdk-conformance-runner -- \
    --root "$REPO_ROOT" \
    --language "$language" \
    --adapter-report "$report" \
    --format json
}

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d "$REPO_ROOT/target/sdk-conformance-report-gate.XXXXXX")"
  trap 'rm -rf "$tmp"' EXIT

  mkdir -p "$tmp/sdk/conformance/runner"
  cp "$REPO_ROOT/sdk/conformance/runner/go-action-adapter-report.json" \
    "$tmp/go-action-adapter-report.json"
  python3 - "$tmp/go-action-adapter-report.json" <<'PY'
from __future__ import annotations

import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
report = json.loads(path.read_text(encoding="utf-8"))
report["records"] = [
    record
    for record in report["records"]
    if record.get("case_id") != "invocation/complete_tuple"
]
path.write_text(json.dumps(report), encoding="utf-8")
PY

  if run_report go "$tmp/go-action-adapter-report.json" >"$tmp/out" 2>&1; then
    echo "self-test expected missing Go action-adapter record to fail" >&2
    exit 1
  fi
  grep -Fq "ACTION_ADAPTER_MISSING" "$tmp/out"

  echo "check-sdk-conformance-reports self-test ok"
  exit 0
fi

run_report rust sdk/conformance/runner/rust-action-adapter-report.json >/dev/null
run_report c_abi sdk/conformance/runner/c-abi-action-adapter-report.json >/dev/null
run_report go sdk/conformance/runner/go-action-adapter-report.json >/dev/null
run_report python sdk/conformance/runner/python-action-adapter-report.json >/dev/null
run_report node sdk/conformance/runner/node-action-adapter-report.json >/dev/null
run_report java sdk/conformance/runner/java-action-adapter-report.json >/dev/null
run_report swift sdk/conformance/runner/swift-action-adapter-report.json >/dev/null

echo "check-sdk-conformance-reports ok"
