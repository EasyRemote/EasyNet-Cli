#!/usr/bin/env bash
# standalone-hub-principal-lifecycle-e2e.sh — section 14.3 acceptance gate
# =============================================================================
#
# Section 14.3 requires both deployment shapes:
#   1. Backend-free standalone Hub PrincipalLifecycle.
#   2. Backend-present account flow mapped into the same daemon runtime.
#
# The focused scripts remain independently runnable for diagnosis. This gate is
# the canonical acceptance entrypoint for the pair.

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ "${1:-}" == "--self-test" ]]; then
  bash -n "$0"
  grep -q "standalone-hub-recovery-e2e.sh" "$0"
  grep -q "backend-live-principal-e2e.sh" "$0"
  grep -q "backend-live-http-daemon-e2e.sh" "$0"
  bash "$SELF_DIR/standalone-hub-recovery-e2e.sh" --self-test
  bash "$SELF_DIR/backend-live-principal-e2e.sh" --self-test
  bash "$SELF_DIR/backend-live-http-daemon-e2e.sh" --self-test
  echo "standalone-hub-principal-lifecycle-e2e self-test ok"
  exit 0
fi

echo "[standalone-hub-principal-lifecycle-e2e] running backend-free standalone Hub PrincipalLifecycle E2E..."
bash "$SELF_DIR/standalone-hub-recovery-e2e.sh"

echo "[standalone-hub-principal-lifecycle-e2e] running Backend-present PrincipalLifecycle E2E..."
bash "$SELF_DIR/backend-live-principal-e2e.sh"

echo "[standalone-hub-principal-lifecycle-e2e] running browser HTTP to live daemon E2E..."
bash "$SELF_DIR/backend-live-http-daemon-e2e.sh"

echo "[standalone-hub-principal-lifecycle-e2e] PASS"
