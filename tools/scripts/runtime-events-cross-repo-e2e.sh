#!/usr/bin/env bash
# runtime-events-cross-repo-e2e.sh — cross-repo Runtime Events adapter evidence

set -euo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../.." && pwd)"
BACKEND_ROOT="${EASYNET_BACKEND_ROOT:-$REPO_ROOT/../EasyNet/backend}"
EASYREMOTE_ROOT="${EASYREMOTE_ROOT:-$REPO_ROOT/../EasyRemote}"

PYTHON_BIN="${PYTHON_BIN:-}"
if [[ -z "$PYTHON_BIN" ]]; then
  if [[ -x "$EASYREMOTE_ROOT/.venv/bin/python" ]]; then
    PYTHON_BIN="$EASYREMOTE_ROOT/.venv/bin/python"
  else
    PYTHON_BIN="python3"
  fi
fi

if [[ "${1:-}" == "--self-test" ]]; then
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  backend_fixture="$tmp/backend"
  easyremote_fixture="$tmp/easyremote"
  mkdir -p "$backend_fixture/internal/sdkevents" \
    "$backend_fixture/internal/svc" \
    "$easyremote_fixture/tests"
  cat >"$backend_fixture/internal/sdkevents/events_test.go" <<'EOF'
package sdkevents

func TestClientBuildsDeviceSubscriptionThroughBackendEventsAdapter() {}
EOF
  cat >"$backend_fixture/internal/svc/sdk_events_test.go" <<'EOF'
package svc

func TestSDKEventsAdapterOwnsProductStreamLowering() {}
EOF
  cat >"$easyremote_fixture/tests/test_mission.py" <<'EOF'
def test_event_tailer_fails_closed_on_dropped_events():
    pass
EOF
  bash -n "$0"
  test -f "$REPO_ROOT/sdk/go/runtime_events_test.go"
  test -f "$REPO_ROOT/sdk/python/tests/test_runtime_events.py"
  test -f "$backend_fixture/internal/sdkevents/events_test.go"
  test -f "$backend_fixture/internal/svc/sdk_events_test.go"
  test -f "$easyremote_fixture/tests/test_mission.py"
  grep -q "TestClientBuildsDeviceSubscriptionThroughBackendEventsAdapter" "$backend_fixture/internal/sdkevents/events_test.go"
  grep -q "TestSDKEventsAdapterOwnsProductStreamLowering" "$backend_fixture/internal/svc/sdk_events_test.go"
  grep -q "test_event_tailer_fails_closed_on_dropped_events" "$easyremote_fixture/tests/test_mission.py"
  echo "runtime-events-cross-repo-e2e self-test ok"
  exit 0
fi

if [[ ! -f "$BACKEND_ROOT/go.mod" ]]; then
  echo "[runtime-events-cross-repo-e2e] backend go.mod not found at $BACKEND_ROOT" >&2
  exit 2
fi
if [[ ! -f "$EASYREMOTE_ROOT/pyproject.toml" ]]; then
  echo "[runtime-events-cross-repo-e2e] EasyRemote pyproject.toml not found at $EASYREMOTE_ROOT" >&2
  exit 2
fi

echo "[runtime-events-cross-repo-e2e] Go SDK Runtime Events tests..."
(
  cd "$REPO_ROOT/sdk/go"
  go test . -run 'TestRuntimeEvent' -count=1
)

echo "[runtime-events-cross-repo-e2e] Python SDK Runtime Events tests..."
(
  cd "$REPO_ROOT"
  PYTHONPATH="$REPO_ROOT/sdk/python" "$PYTHON_BIN" -m pytest -q sdk/python/tests/test_runtime_events.py
)

echo "[runtime-events-cross-repo-e2e] Backend SDK event adapter tests..."
(
  cd "$BACKEND_ROOT"
  go test ./internal/sdkevents ./internal/svc ./internal/sdkboundary \
    -run 'Test(ClientBuildsDeviceSubscriptionThroughBackendEventsAdapter|ClientBuildsSessionSubscriptionWithSinceSequence|SubscriptionAbilityRejectsUnsupportedStream|SDKEventsAdapterOwnsProductStreamLowering|SDKEventsAdapterDoesNotUseCanonicalSDKRouteCatalog)$' \
    -count=1
)

echo "[runtime-events-cross-repo-e2e] EasyRemote product event consumer tests..."
(
  cd "$EASYREMOTE_ROOT"
  PYTHONPATH="$EASYREMOTE_ROOT:$REPO_ROOT/sdk/python" "$PYTHON_BIN" -m pytest -q \
    tests/test_mission.py::test_events_fetches_mission_event_page \
    tests/test_mission.py::test_event_tailer_fails_closed_on_dropped_events \
    tests/test_mission.py::test_event_tailer_rejects_has_more_without_cursor_progress \
    tests/test_mission.py::test_event_tailer_rejects_nonempty_page_without_cursor_progress \
    tests/test_mission.py::test_event_controls_enforce_bounds_before_invocation
)

echo "[runtime-events-cross-repo-e2e] PASS"
