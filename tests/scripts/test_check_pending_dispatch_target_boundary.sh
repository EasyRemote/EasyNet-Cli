#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-pending-dispatch-target-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-pending-dispatch-target-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p \
        "$sandbox/src/daemon/invocation/bidi/state" \
        "$sandbox/src/daemon/invocation/dispatch" \
        "$sandbox/src/daemon/invocation/streams"
    cat >"$sandbox/src/daemon/invocation/bidi/state/pending_dispatch.rs" <<'RS'
pub struct PendingHandle;
pub struct PendingStreamHandle;
pub struct PendingDispatchMap;
pub struct PendingStreamDispatchMap;
pub enum StreamDeliveryPolicy {
    BoundedNoWait,
}

impl PendingDispatchMap {
    pub fn register_pending_for(&self, target_ura: &str) -> PendingHandle {
        let target_ura = require_pending_target_ura(target_ura);
        let _ = target_ura.to_string();
        PendingHandle
    }
}

impl PendingStreamDispatchMap {
    pub fn register_pending_for(&self, target_ura: &str) -> PendingStreamHandle {
        self.register_pending_for_policy(target_ura, StreamDeliveryPolicy::BoundedNoWait)
    }

    fn register_pending_for_policy(
        &self,
        target_ura: &str,
        _delivery_policy: StreamDeliveryPolicy,
    ) -> PendingStreamHandle {
        let target_ura = require_pending_target_ura(target_ura);
        let _ = target_ura.to_string();
        PendingStreamHandle
    }
}

fn require_pending_target_ura(target_ura: &str) -> &str {
    let target_ura = target_ura.trim();
    assert!(
        !target_ura.is_empty(),
        "pending dispatch target_ura is required"
    );
    target_ura
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "pending dispatch target_ura is required")]
    fn unary_pending_registration_rejects_empty_target_ura() {
        let map = PendingDispatchMap;
        let _ = map.register_pending_for(" ");
    }

    #[test]
    #[should_panic(expected = "pending dispatch target_ura is required")]
    fn stream_pending_registration_rejects_empty_target_ura() {
        let map = PendingStreamDispatchMap;
        let _ = map.register_pending_for(" ");
    }
}
RS
    cat >"$sandbox/src/daemon/invocation/dispatch/unary_dispatcher.rs" <<'RS'
fn dispatch(selected_route: SelectedInvokeRoute, pending: PendingDispatchMap) {
    let _ = pending.register_pending_for(&selected_route.execution_host_ura);
}
struct SelectedInvokeRoute {
    execution_host_ura: String,
}
struct PendingDispatchMap;
impl PendingDispatchMap {
    fn register_pending_for(&self, _: &str) {}
}
RS
    cat >"$sandbox/src/daemon/invocation/streams/stream_dispatcher.rs" <<'RS'
fn dispatch(selected_route: SelectedInvokeRoute, pending: PendingStreamDispatchMap) {
    let _ = pending.register_pending_for(&selected_route.execution_host_ura);
}
struct SelectedInvokeRoute {
    execution_host_ura: String,
}
struct PendingStreamDispatchMap;
impl PendingStreamDispatchMap {
    fn register_pending_for(&self, _: &str) {}
}
RS
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_PENDING_DISPATCH_TARGET_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: clean fixture should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
cat >>"$SB/src/daemon/invocation/bidi/state/pending_dispatch.rs" <<'RS'

impl PendingDispatchMap {
    pub fn register_pending(&self) -> PendingHandle {
        self.register_pending_for("")
    }
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "no-target register_pending method should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/fn require_pending_target_ura\(target_ura: &str\) -> &str/fn unchecked_pending_target_ura(target_ura: &str) -> &str/' \
    "$SB/src/daemon/invocation/bidi/state/pending_dispatch.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing target_ura guard should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/register_pending_for\(&selected_route\.execution_host_ura\)/register_pending()/' \
    "$SB/src/daemon/invocation/dispatch/unary_dispatcher.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "unary dispatcher missing execution-host URA binding should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/register_pending_for\(&selected_route\.execution_host_ura\)/register_pending()/' \
    "$SB/src/daemon/invocation/streams/stream_dispatcher.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "stream dispatcher missing execution-host URA binding should exit 1 (got $rc)"

echo "all check-pending-dispatch-target-boundary contract cases passed"
