#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-local-daemon-socket-resolver-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" \
  "$SB/src/support/platform" \
  "$SB/src/daemon/persistence" \
  "$SB/src/daemon/boot" \
  "$SB/src/daemon/invocation/routing" \
  "$SB/src/daemon/ability/builtins/integrations/a2a" \
  "$SB/src/daemon/execution/mission"
cp "$SCRIPT" "$SB/tools/scripts/check-local-daemon-socket-resolver-boundary.sh"

cat >"$SB/src/support/platform/local_daemon_grpc.rs" <<'RS'
pub(crate) fn probe_accepting() -> bool {
    true
}
RS

cat >"$SB/src/daemon/persistence/daemon_config.rs" <<'RS'
pub fn resolved_local_uds_path_with_env_override() -> std::path::PathBuf {
    std::path::PathBuf::from("daemon.sock")
}
RS

for target in \
  "$SB/src/daemon/boot/process.rs" \
  "$SB/src/daemon/invocation/routing/remote_invoke.rs" \
  "$SB/src/daemon/ability/builtins/integrations/a2a/client.rs" \
  "$SB/src/daemon/execution/mission/invocation_gateway.rs"; do
  cat >"$target" <<'RS'
fn socket() {
    let _ = crate::daemon::persistence::daemon_config::resolved_local_uds_path_with_env_override();
}
RS
done

(
  cd "$SB"
  bash tools/scripts/check-local-daemon-socket-resolver-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >>"$SB/src/support/platform/local_daemon_grpc.rs" <<'RS'
pub(crate) fn resolve_socket_path() -> std::path::PathBuf {
    crate::daemon::persistence::daemon_config::resolved_local_uds_path_with_env_override()
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-local-daemon-socket-resolver-boundary.sh
) >/tmp/check-local-daemon-socket-resolver-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "support resolver shim should exit 1 (got $rc)"
grep -Fq "must live in daemon_config" /tmp/check-local-daemon-socket-resolver-boundary.out \
  || fail "support shim failure should name daemon_config ownership"

cat >"$SB/src/support/platform/local_daemon_grpc.rs" <<'RS'
pub(crate) fn probe_accepting() -> bool {
    true
}
RS
cat >>"$SB/src/daemon/boot/process.rs" <<'RS'
fn retired_socket() {
    let _ = crate::support::platform::local_daemon_grpc::resolve_socket_path();
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-local-daemon-socket-resolver-boundary.sh
) >/tmp/check-local-daemon-socket-resolver-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "retired caller shim should exit 1 (got $rc)"
grep -Fq "production callers must import the daemon_config socket resolver directly" \
  /tmp/check-local-daemon-socket-resolver-boundary.out \
  || fail "caller shim failure should name direct daemon_config import"

echo "test_check_local_daemon_socket_resolver_boundary.sh: all cases passed"
