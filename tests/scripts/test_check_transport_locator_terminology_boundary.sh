#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-transport-locator-terminology-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/support/platform" "$SB/tests"
cp "$SCRIPT" "$SB/tools/scripts/check-transport-locator-terminology-boundary.sh"

cat >"$SB/src/support/platform/local_daemon_grpc.rs" <<'RS'
use tonic::transport::{Channel, Endpoint, Uri as GrpcEndpointLocator};

fn connect(endpoint: Endpoint) {
    let _ = endpoint.connect_with_connector(tower::service_fn(move |_: GrpcEndpointLocator| async {
        Ok::<_, std::io::Error>(())
    }));
}
RS

(
  cd "$SB"
  bash tools/scripts/check-transport-locator-terminology-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >"$SB/tests/leaked_uri.rs" <<'RS'
use tonic::transport::{Channel, Endpoint, Server, Uri};

fn connect(endpoint: Endpoint) {
    let _ = endpoint.connect_with_connector(tower::service_fn(move |_: Uri| async {
        Ok::<_, std::io::Error>(())
    }));
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-transport-locator-terminology-boundary.sh
) >/tmp/check-transport-locator-terminology-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "bare transport Uri should exit 1 (got $rc)"

rm "$SB/tests/leaked_uri.rs"
cat >"$SB/src/support/platform/semantic_uri.rs" <<'RS'
const CALLER_URI: &str = "easynet:///r/example/agent/alice";
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-transport-locator-terminology-boundary.sh
) >/tmp/check-transport-locator-terminology-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "semantic URI should exit 1 (got $rc)"

echo "test_check_transport_locator_terminology_boundary.sh: all cases passed"
