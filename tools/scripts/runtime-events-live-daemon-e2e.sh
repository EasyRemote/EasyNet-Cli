#!/usr/bin/env bash
# runtime-events-live-daemon-e2e.sh — cross-repo adapters plus live daemon proof
# =============================================================================
#
# Runtime events become cutover evidence only when two facts hold together:
# downstream products consume the SDK event adapters, and the SDK provider reads
# event pages from a real daemon rather than from fake transport fixtures. This
# gate composes the existing cross-repo adapter gate with Go/Python live daemon
# smokes that exercise RuntimeEventClient over C ABI handle-events.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"

if [[ "${1:-}" == "--self-test" ]]; then
  bash -n "$0"
  grep -q "RuntimeEventClient read live daemon handle events" "$REPO_ROOT/sdk/go/live_smoke_cabi_test.go"
  grep -q "RuntimeEventClient read live daemon handle events" "$SELF_DIR/python-sdk-live-smoke.sh"
  grep -q "runtime-events-cross-repo-e2e.sh" "$0"
  grep -q "go-sdk-live-smoke.sh" "$0"
  grep -q "python-sdk-live-smoke.sh" "$0"
  echo "runtime-events-live-daemon-e2e self-test ok"
  exit 0
fi

echo "[runtime-events-live-daemon-e2e] running cross-repo runtime-events adapter gate..."
bash "$SELF_DIR/runtime-events-cross-repo-e2e.sh"

echo "[runtime-events-live-daemon-e2e] running Go live daemon runtime-events proof..."
bash "$SELF_DIR/go-sdk-live-smoke.sh"

echo "[runtime-events-live-daemon-e2e] running Python live daemon runtime-events proof..."
bash "$SELF_DIR/python-sdk-live-smoke.sh"

echo "[runtime-events-live-daemon-e2e] PASS"
