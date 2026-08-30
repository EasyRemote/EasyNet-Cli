#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_WINDOWS_MAIN_CRATE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
HOST_STREAM="$ROOT/src/daemon/execution/mission/executors/host_stream.rs"
AGENT_LIFECYCLE="$ROOT/src/daemon/ability/builtins/agents/lifecycle.rs"

fail() {
  printf 'check-windows-main-crate-platform-boundaries: %s\n' "$1" >&2
  exit 1
}

require() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  rg -Uq -- "$pattern" "$path" || fail "$message"
}

for path in "$HOST_STREAM" "$AGENT_LIFECYCLE"; do
  [[ -f "$path" ]] || fail "missing required source ${path#"$ROOT/"}"
done

require '#\[cfg\(unix\)\][[:space:]]*use tokio::net::UnixStream;' "$HOST_STREAM" \
  'UnixStream import must remain Unix-only'
require '#\[cfg\(unix\)\][[:space:]]*pub fn run_host_stream\(' "$HOST_STREAM" \
  'host_stream Unix implementation must remain explicitly platform-bounded'
require '#\[cfg\(not\(unix\)\)\][[:space:]]*pub fn run_host_stream\(' "$HOST_STREAM" \
  'non-Unix host_stream must expose an explicit fail-closed boundary'
require 'manifest transport requires a Unix domain socket' "$HOST_STREAM" \
  'non-Unix host_stream failure must explain the transport boundary'
require '#\[cfg\(unix\)\][[:space:]]*if let Some\(handle\) = open_root' "$AGENT_LIFECYCLE" \
  'open-handle Agent root identity validation must remain Unix-only'
require '#\[cfg\(not\(unix\)\)\][[:space:]]*let _ = open_root;' "$AGENT_LIFECYCLE" \
  'non-Unix Agent purge validation must consume the structurally absent handle explicitly'

printf 'check-windows-main-crate-platform-boundaries: ok\n'
