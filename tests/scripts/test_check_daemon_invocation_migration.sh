#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-daemon-invocation-migration.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-daemon-invocation-migration.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox"
    cp -R "$REPO_ROOT/src" "$sandbox/src"
    cp -R "$REPO_ROOT/tests" "$sandbox/tests"
    cp "$REPO_ROOT/README.md" "$sandbox/README.md"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_DAEMON_INVOCATION_MIGRATION_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: clean tree should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/pub enum IncomingFrame \{/pub enum IncomingFrame {\n    Invoke { request_id: String },/' \
    "$SB/src/daemon/control/frames.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired IncomingFrame variant should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >"$SB/src/__retired_control_probe.rs" <<'RS'
pub fn probe() {
    let _ = crate::daemon::control::frames::IncomingFrame::OpenBidi;
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired control constructor should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >"$SB/src/__daemon_invocation_probe.rs" <<'RS'
pub fn probe() {
    let _ = crate::daemon::DaemonInvocation {
        caller_ura: String::new(),
        callee_ura: String::new(),
        ability: String::new(),
        subject_ura: String::new(),
        nonce: [0; 16],
        causal_context: Default::default(),
        args: Vec::new(),
        content_type: String::new(),
    };
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "direct DaemonInvocation construction should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/args_state: PhantomData<ArgsState>,/args_set: bool,/' \
    "$SB/src/daemon/invocation/dispatch/request.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime args-set boolean should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e \
    's/derivation_policy: axon_sdk::invocation::InvocationDerivationPolicy,/derivation_policy: (),/g' \
    "$SB/src/daemon/invocation/dispatch/request.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing explicit derivation policy should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e \
    's/impl<ArgsState> DaemonInvocationBuilder<ArgsState> \{/impl<ArgsState> DaemonInvocationBuilder<ArgsState> {\n    pub fn nonce(mut self, nonce: [u8; 16]) -> Self { self.nonce = nonce; self }/' \
    "$SB/src/daemon/invocation/dispatch/request.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "public nonce override should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e \
    's/impl DaemonInvocationBuilder<InvocationArgsSet> \{/impl<ArgsState> DaemonInvocationBuilder<ArgsState> {/' \
    "$SB/src/daemon/invocation/dispatch/request.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "generic builder completion path should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '\nNote: Requires a local or remote Axon runtime. The easynet runtime start command auto-spawns one.\n' \
    >>"$SB/README.md"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "stale README product runtime text should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/daemon/invocation/receipts"
cat >"$SB/src/daemon/invocation/receipts/runtime_record.rs" <<'RS'
pub struct RuntimeInvocation;
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "obsolete daemon runtime-record authority should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >>"$SB/src/daemon/invocation/dispatch/daemon_invocation_service.rs" <<'RS'

#[test]
fn remote_bidi_target_ura_does_not_repair_bare_device_agent_alias() {
    panic!("remote bidi target extraction must preserve legacy aliases");
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "remote bidi legacy-alias language should exit 1 (got $rc)"

echo "test_check_daemon_invocation_migration.sh: all cases passed"
