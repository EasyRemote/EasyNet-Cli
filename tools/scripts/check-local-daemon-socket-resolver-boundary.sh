#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_LOCAL_DAEMON_SOCKET_RESOLVER_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-local-daemon-socket-resolver-boundary: %s\n' "$1" >&2
  exit 1
}

LOCAL_TRANSPORT="src/support/platform/local_daemon_grpc.rs"
DAEMON_CONFIG="src/daemon/persistence/daemon_config.rs"
[[ -f "$LOCAL_TRANSPORT" ]] || fail "missing $LOCAL_TRANSPORT"
[[ -f "$DAEMON_CONFIG" ]] || fail "missing $DAEMON_CONFIG"

if rg -n 'fn resolve_socket_path\s*\(' "$LOCAL_TRANSPORT"; then
  fail "local daemon socket resolver must live in daemon_config, not support transport"
fi

if rg -n 'local_daemon_grpc::resolve_socket_path|support/local_daemon_grpc::resolve_socket_path' src; then
  fail "production callers must import the daemon_config socket resolver directly"
fi

if ! rg -n 'pub fn resolved_local_uds_path_with_env_override\(\)' "$DAEMON_CONFIG" >/dev/null; then
  fail "daemon_config must own the CLI local daemon socket resolver"
fi

for target in \
  "src/daemon/boot/process.rs" \
  "src/daemon/invocation/routing/remote_invoke.rs" \
  "src/daemon/ability/builtins/integrations/a2a/client.rs" \
  "src/daemon/execution/mission/invocation_gateway.rs"; do
  [[ -f "$target" ]] || fail "missing $target"
  if ! rg -n 'daemon_config::resolved_local_uds_path_with_env_override' "$target" >/dev/null; then
    fail "$target must use daemon_config as the local daemon socket resolver owner"
  fi
done

echo "check-local-daemon-socket-resolver-boundary: ok"
