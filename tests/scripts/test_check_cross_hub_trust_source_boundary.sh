#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-cross-hub-trust-source-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-cross-hub-trust-source-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/daemon/federation/client"
    mkdir -p "$sandbox/src/daemon/boot"
    cp "$REPO_ROOT/src/daemon/federation/client/cross_hub_dial.rs" "$sandbox/src/daemon/federation/client/cross_hub_dial.rs"
    cp "$REPO_ROOT/src/daemon/boot/invocation.rs" "$sandbox/src/daemon/boot/invocation.rs" 2>/dev/null || true
    mkdir -p "$sandbox/src/daemon/boot/invocation"
    cp "$REPO_ROOT/src/daemon/boot/invocation/mod.rs" "$sandbox/src/daemon/boot/invocation/mod.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_CROSS_HUB_TRUST_SOURCE_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: cross-hub trust source boundary should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
cat >> "$SB/src/daemon/federation/client/cross_hub_dial.rs" <<'RS'
enum TrustSource {
    Snapshot(Arc<RealmTrustAnchor>),
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "TrustSource snapshot enum should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >> "$SB/src/daemon/federation/client/cross_hub_dial.rs" <<'RS'
impl CrossHubDialer {
    pub fn new(trust_anchor: Arc<RealmTrustAnchor>) -> Self {
        Self::from_trust_source(TrustSource::Snapshot(trust_anchor))
    }
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "snapshot constructor should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/CrossHubDialer::with_trust_anchor_cell/CrossHubDialer::new/' "$SB/src/daemon/boot/invocation/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "boot wiring without SharedTrustAnchor constructor should exit 1 (got $rc)"

echo "test_check_cross_hub_trust_source_boundary.sh: all cases passed"
