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
python3 - "$SB/src/cli/commands/invoke.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
probe = '''
#[allow(dead_code)]
fn anonymous_remote_request_probe(target: &crate::daemon::invocation::routing::remote_invoke::RemoteAbilityInvocationTarget) {
    let _ = crate::daemon::invocation::routing::remote_invoke::RemoteInvocationRequest::new(
        target,
        "easynet:///r/acme/device/caller",
        "easynet:///r/acme/resource/r1",
        [1; 16],
        axon_sdk::invocation::CausalContext::None,
        serde_json::Value::Null,
        std::time::Duration::from_secs(1),
    );
}

'''
path.write_text(probe + text, encoding="utf-8")
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "public remote ingress direct request construction should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >"$SB/src/cli/commands/__remote_request_probe.rs" <<'RS'
#[allow(dead_code)]
fn anonymous_remote_request_probe(target: &crate::daemon::invocation::routing::remote_invoke::RemoteAbilityInvocationTarget) {
    let _ = crate::daemon::invocation::routing::remote_invoke::RemoteInvocationRequest::new(
        target,
        "easynet:///r/acme/device/caller",
        "easynet:///r/acme/resource/r1",
        [1; 16],
        axon_sdk::invocation::CausalContext::None,
        serde_json::Value::Null,
        std::time::Duration::from_secs(1),
    );
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "production remote request construction outside tuple plan should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/cli/daemon_client/remote_system_ability.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
probe = '''

#[allow(dead_code)]
fn anonymous_remote_system_nonce_probe() {
    let _ = axon_sdk::invocation::fresh_nonce();
}
'''
anchor = "#[cfg(all(test, feature = \"axon-pb\"))]"
if anchor not in text:
    raise SystemExit("remote system ability test anchor missing")
path.write_text(text.replace(anchor, probe + "\n" + anchor, 1), encoding="utf-8")
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "remote system anonymous nonce derivation should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >"$SB/src/__fresh_root_probe.rs" <<'RS'
pub fn probe() {
    let _ = axon_sdk::invocation::InvocationDerivationPolicy::FreshRoot;
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "anonymous FreshRoot policy should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >"$SB/src/__local_system_context_probe.rs" <<'RS'
pub fn probe() {
    let _ = crate::support::platform::local_invoke::LocalSystemInvocationContext::new(
        "easynet:///r/acme/resource/r1",
        [1; 16],
        &[],
        std::time::Duration::from_secs(1),
        None,
    );
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "anonymous local system context construction should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >>"$SB/src/daemon/invocation/routing/remote_invoke.rs" <<'RS'

#[cfg(test)]
mod rf8_gate_test_only_probe {
    fn allowed_test_only_name() {}
}

#[allow(dead_code)]
pub(crate) fn daemon_system_root() {}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "remote root constructor after cfg(test) should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/support/platform/local_daemon_grpc.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
probe = '''
#[cfg(feature = "axon-pb")]
#[allow(dead_code)]
fn anonymous_local_loopback_probe() -> anyhow::Result<()> {
    let _ = crate::daemon::invocation::dispatch::invocation_wire::LocalDaemonLoopbackInvocation::from_target(
        "job.run",
        serde_json::Value::Null,
        "easynet:///r/acme/device/local",
        "easynet:///r/acme/device/local",
        "easynet:///r/acme/device/local",
        crate::daemon::invocation::dispatch::invocation_wire::InvocationDerivationPolicy::FreshRoot,
        std::time::Duration::from_secs(1),
    )?;
    Ok(())
}

'''
marker = '#[cfg(all(test, feature = "axon-pb"))]'
path.write_text(text.replace(marker, probe + marker, 1), encoding="utf-8")
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "local loopback direct invocation construction should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/support/platform/local_daemon_grpc.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
probe = '''
#[cfg(feature = "axon-pb")]
#[allow(dead_code)]
enum LocalDaemonSubjectPolicy {
    SelfTarget,
}

'''
marker = '#[cfg(all(test, feature = "axon-pb"))]'
path.write_text(text.replace(marker, probe + marker, 1), encoding="utf-8")
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "obsolete local loopback subject policy should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/daemon/invocation/bidi/bidi_dispatcher.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
probe = '''
#[allow(dead_code)]
fn dispatch_checked_session_request() {}

'''
marker = '#[cfg(test)]'
path.write_text(text.replace(marker, probe + marker, 1), encoding="utf-8")
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "procedural JSON session request dispatcher should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >"$SB/src/__tuple_patch_probe.rs" <<'RS'
pub fn probe(mut target: crate::daemon::invocation::routing::target::InvocationTarget) {
    target = target.with_subject("easynet:///r/acme/resource/r1");
    let _ = target.with_causal_context(axon_sdk::invocation::CausalContext::None);
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "production InvocationTarget tuple patching should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/daemon/invocation/routing/target.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
needle = "    pub fn with_request_metadata("
insertion = '''
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = InvocationSubject::explicit(subject);
        self
    }

    pub fn with_causal_context(mut self, causal_context: CausalContext) -> Self {
        self.causal_context = InvocationCausalContext::explicit(causal_context);
        self
    }

'''
path.write_text(text.replace(needle, insertion + needle, 1), encoding="utf-8")
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "InvocationTarget retired tuple mutation API should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/daemon/invocation/routing/target.rs" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
text = text.replace("local_daemon_system_for_subject", "local_daemon_system_with_subject")
path.write_text(text, encoding="utf-8")
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "InvocationTarget retired local_daemon_system_with_subject name should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/support/platform/local_invoke.rs" "$SB/src/support/platform/local_daemon_grpc.rs" <<'PY'
from pathlib import Path
import sys

local_invoke = Path(sys.argv[1])
local_daemon = Path(sys.argv[2])

text = local_invoke.read_text(encoding="utf-8")
needle = "pub struct LocalDaemonSystemAbilityIssuer;"
insertion = '''
pub fn invoke_local_ability_with_subject(
    ability: &str,
    args: serde_json::Value,
    subject_ura: &str,
) -> anyhow::Result<serde_json::Value> {
    crate::support::platform::local_daemon_grpc::invoke_local_daemon_ability_with_subject(
        ability,
        args,
        subject_ura,
    )
}

'''
local_invoke.write_text(text.replace(needle, insertion + needle, 1), encoding="utf-8")

text = local_daemon.read_text(encoding="utf-8")
needle = "#[cfg(feature = \"axon-pb\")]\npub(crate) fn invoke_local_daemon_system_ability_root_for_subject_timeout("
insertion = '''
pub(crate) struct LocalDaemonAbilityClient;

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_with_subject(
    function_name: &str,
    payload_json: serde_json::Value,
    subject_ura: &str,
) -> anyhow::Result<serde_json::Value> {
    let tuple_plan = LocalDaemonLoopbackTuplePlan::local_root_for_subject(
        function_name,
        payload_json,
        subject_ura,
        std::time::Duration::from_secs(30),
    )?;
    invoke_local_daemon_ability_with_tuple_plan(tuple_plan)
}

'''
if needle not in text:
    raise SystemExit("subject-bound local daemon issuer anchor missing")
local_daemon.write_text(text.replace(needle, insertion + needle, 1), encoding="utf-8")
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "generic local invoke with_subject facade should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - \
    "$SB/tests/resolve_before_invoke_e2e.rs" \
    "$SB/src/daemon/ability/builtins/governance/teach.rs" \
    "$SB/src/daemon/keyring/abilities.rs" <<'PY'
from pathlib import Path
import sys

for path in map(Path, sys.argv[1:]):
    text = path.read_text(encoding="utf-8")
    text = text.replace("invoke_with_explicit_subject", "invoke_with_subject")
    text = text.replace("caller_env_for_explicit_subject", "caller_env_with_subject")
    text = text.replace(
        "handle_with_bound_subject_and_signing_key",
        "handle_with_subject_and_signing_key",
    )
    path.write_text(text, encoding="utf-8")
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "non-SDK subject helper vocabulary should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >"$SB/src/__target_constructor_probe.rs" <<'RS'
pub fn probe() {
    let _ = crate::daemon::invocation::routing::target::InvocationTarget::local_daemon_system(
        "observe.health",
        serde_json::Value::Null,
        crate::daemon::invocation::routing::target::CallMode::Rpc,
    );
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "production daemon-system target constructor should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >"$SB/src/__target_constructor_subject_probe.rs" <<'RS'
pub fn probe() {
    let _ = crate::daemon::invocation::routing::target::InvocationTarget::local_daemon_system_for_subject(
        "observe.health",
        serde_json::Value::Null,
        crate::daemon::invocation::routing::target::CallMode::Rpc,
        "easynet:///r/acme/device/dev-a",
    );
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "production daemon-system subject target constructor should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >"$SB/src/cli/commands/__subject_only_local_ingress_probe.rs" <<'RS'
#[allow(dead_code)]
fn probe(target: &crate::support::platform::local_invoke::LocalAbilityTarget) {
    let _ = crate::support::platform::local_invoke::invoke_local_ability_target_with_subject_timeout(
        target,
        serde_json::Value::Null,
        None,
        std::time::Duration::from_secs(1),
    );
    let _ = crate::support::platform::local_invoke::invoke_local_ability_target_stream_with_subject(
        target,
        serde_json::Value::Null,
        None,
        std::time::Duration::from_secs(1),
        None,
    );
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "subject-only local public ingress should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e \
    's/for route in DaemonBidiRoute::ALL\.iter\(\)\.copied\(\)/for route in [].iter().copied()/' \
    "$SB/src/daemon/invocation/dispatch/daemon_route_runtime.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "partial bidi exact-route registration should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/SystemInvocationIssuer::request_for_descriptor_ref/LocalRuntimeRequestFactory::request_for/' \
    "$SB/src/daemon/invocation/dispatch/local_runtime_invoker.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "local daemon-system calls must use SystemInvocationIssuer (got $rc)"

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
