#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-windows-main-crate-platform-boundaries.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

write_fixture() {
  rm -rf "$SANDBOX/src"
  mkdir -p "$SANDBOX/src/daemon/execution/mission/executors" \
    "$SANDBOX/src/daemon/ability/builtins/agents"
  cp "$REPO_ROOT/src/daemon/execution/mission/executors/host_stream.rs" \
    "$SANDBOX/src/daemon/execution/mission/executors/host_stream.rs"
  cp "$REPO_ROOT/src/daemon/ability/builtins/agents/lifecycle.rs" \
    "$SANDBOX/src/daemon/ability/builtins/agents/lifecycle.rs"
}

run_ok() {
  CHECK_WINDOWS_MAIN_CRATE_ROOT="$SANDBOX" "$SCRIPT" >/dev/null
}

run_fail() {
  local expected="$1"
  local output
  if output="$(CHECK_WINDOWS_MAIN_CRATE_ROOT="$SANDBOX" "$SCRIPT" 2>&1)"; then
    printf 'expected failure containing %s\n' "$expected" >&2
    exit 1
  fi
  [[ "$output" == *"$expected"* ]] || {
    printf 'expected failure containing %s, got:\n%s\n' "$expected" "$output" >&2
    exit 1
  }
}

write_fixture
run_ok

write_fixture
perl -0pi -e 's/#\[cfg\(unix\)\]\nuse tokio::net::UnixStream;/use tokio::net::UnixStream;/' \
  "$SANDBOX/src/daemon/execution/mission/executors/host_stream.rs"
run_fail 'UnixStream import must remain Unix-only'

write_fixture
perl -0pi -e 's/#\[cfg\(not\(unix\)\)\]\npub fn run_host_stream\(/pub fn run_host_stream_unbounded\(/' \
  "$SANDBOX/src/daemon/execution/mission/executors/host_stream.rs"
run_fail 'non-Unix host_stream must expose an explicit fail-closed boundary'

write_fixture
perl -0pi -e 's/#\[cfg\(unix\)\]\n    if let Some\(handle\) = open_root/if let Some(handle) = open_root/' \
  "$SANDBOX/src/daemon/ability/builtins/agents/lifecycle.rs"
run_fail 'open-handle Agent root identity validation must remain Unix-only'

printf 'test_check_windows_main_crate_platform_boundaries: all cases passed\n'
