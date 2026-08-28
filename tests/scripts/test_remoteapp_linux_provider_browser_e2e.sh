#!/usr/bin/env bash
set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
RUNNER="$REPO_ROOT/tools/scripts/remoteapp-linux-provider-browser-e2e.sh"

bash -n "$RUNNER"
output="$(bash "$RUNNER" --self-test)"
grep -q 'self-test ok' <<<"$output"
grep -q 'current_build_linux_x11_provider_browser' "$RUNNER"
grep -q -- '--expected-input-mode view_only' "$RUNNER"
if grep -Eq '^[[:space:]]*EASYNET_REMOTEAPP_BROWSER_REQUIRE_HOST_INPUT_EFFECTS=1' "$RUNNER"; then
  echo 'Linux Window/Application runner must remain view-only' >&2
  exit 1
fi
grep -q 'product_complete_claim:false' "$RUNNER"
echo 'test_remoteapp_linux_provider_browser_e2e ok'
