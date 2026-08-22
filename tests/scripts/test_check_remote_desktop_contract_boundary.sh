#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remote-desktop-contract-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/plugins/remote-desktop/src/handlers" "$SB/plugins/remote-desktop/abilities"
cp "$SCRIPT" "$SB/tools/scripts/check-remote-desktop-contract-boundary.sh"

cat >"$SB/plugins/remote-desktop/src/contract.rs" <<'RS'
#[derive(serde::Serialize, serde::Deserialize)]
enum RemoteDesktopTransportKind {
    Unspecified,
    #[serde(rename = "webrtc")]
    WebRtc,
}

#[test]
fn media_backend_contract_accepts_only_canonical_webrtc_transport_name() {}

#[test]
fn media_backend_contract_rejects_retired_web_rtc_transport_alias() {}
RS

cat >"$SB/plugins/remote-desktop/src/permissions.rs" <<'RS'
const REMOTE_DESKTOP_HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA: &str =
    "easynet:///r/_system/resource/ability-contract.remote-desktop/host-local-permission-subject";

enum HostLocalPermissionProbeSubject {
    UserSelf,
    UserInvokeResource,
    LocalSystemLoopback,
}

fn host_local_subject_error() {
    RemoteDesktopError::InvalidArgument;
}

fn host_local_permission_subject_contract() {
    json!({
        "subject_contract_ura": REMOTE_DESKTOP_HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA,
        "allowed_subjects": [
            "caller_user_self",
            "descriptor_bound_invoke_resource",
            "local_system_loopback",
        ],
        "target_resource_subjects_allowed": false,
    });
}

fn screen_capture_permission_status() {
    json!({
        "subject_contract": host_local_permission_subject_contract(),
        "input_permission": {
            "permission": "accessibility",
        },
    });
}

fn request_screen_capture_permission() {
    request_input_injection_permission();
    json!({
        "subject_contract": host_local_permission_subject_contract(),
        "input_permission": {
            "permission": "accessibility",
        },
    });
}
RS

cat >"$SB/plugins/remote-desktop/src/schema.rs" <<'RS'
pub fn request_permission_description() -> &'static str {
    "Ask the operating system for host permissions. On macOS this requests \
     Accessibility for pointer/keyboard input injection."
}
RS

cat >"$SB/plugins/remote-desktop/src/registration.rs" <<'RS'
const PERMISSION_PROBE_SUBJECT_KINDS: &[&str] = &["agent", "resource", "user"];
RS

cat >"$SB/plugins/remote-desktop/src/handlers/permission_status.rs" <<'RS'
fn handle() {
    ensure_permission_probe_access();
}

#[test]
fn permission_probe_accepts_authenticated_user_self_subject() {}

#[test]
fn permission_probe_accepts_descriptor_bound_user_invoke_resource_subject() {}

#[test]
fn permission_probe_rejects_device_stream_resource_subject() {}

#[test]
fn permission_probe_rejects_non_caller_user_subject() {}

#[test]
fn permission_probe_rejects_device_subject_before_defaulting() {}

#[test]
fn permission_probe_accepts_local_system_loopback_subject() {}
RS

cat >"$SB/plugins/remote-desktop/src/handlers/request_permission.rs" <<'RS'
fn handle() {
    ensure_permission_probe_access();
}

#[test]
fn request_permission_rejects_device_stream_resource_subject_before_os_prompt() {}

#[test]
fn request_permission_rejects_target_subject_in_args_before_os_prompt() {}
RS

cat >"$SB/plugins/remote-desktop/abilities/remote_desktop.permission_status.ability.toml" <<'TOML'
subject_contract_ura = "easynet:///r/_system/resource/ability-contract.remote-desktop/host-local-permission-subject"
scope_subjects_uras = ["agent", "resource", "user"]
TOML

cat >"$SB/plugins/remote-desktop/abilities/remote_desktop.request_permission.ability.toml" <<'TOML'
description = "Ask the operating system for host permissions. On macOS this requests Accessibility for pointer/keyboard input injection."
subject_contract_ura = "easynet:///r/_system/resource/ability-contract.remote-desktop/host-local-permission-subject"
scope_subjects_uras = ["agent", "resource", "user"]
TOML

(
  cd "$SB"
  bash tools/scripts/check-remote-desktop-contract-boundary.sh
) >/dev/null || fail "happy path should pass"

perl -0pi -e 's/#\[serde\(rename = "webrtc"\)\]/#[serde(rename = "webrtc", alias = "web_rtc")]/' \
  "$SB/plugins/remote-desktop/src/contract.rs"

set +e
(
  cd "$SB"
  bash tools/scripts/check-remote-desktop-contract-boundary.sh
) >/tmp/check-remote-desktop-contract-boundary-alias.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "retired serde alias should exit 1 (got $rc)"

perl -0pi -e 's/, alias = "web_rtc"//' "$SB/plugins/remote-desktop/src/contract.rs"
perl -0pi -e 's/media_backend_contract_rejects_retired_web_rtc_transport_alias/media_backend_contract_rejects_missing_alias_regression/' \
  "$SB/plugins/remote-desktop/src/contract.rs"

set +e
(
  cd "$SB"
  bash tools/scripts/check-remote-desktop-contract-boundary.sh
) >/tmp/check-remote-desktop-contract-boundary-test.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing retired-alias rejection test should exit 1 (got $rc)"

perl -0pi -e 's/media_backend_contract_rejects_missing_alias_regression/media_backend_contract_rejects_retired_web_rtc_transport_alias/' \
  "$SB/plugins/remote-desktop/src/contract.rs"
perl -0pi -e 's/permission_probe_accepts_authenticated_user_self_subject/permission_probe_accepts_default_user_subject/' \
  "$SB/plugins/remote-desktop/src/handlers/permission_status.rs"

set +e
(
  cd "$SB"
  bash tools/scripts/check-remote-desktop-contract-boundary.sh
) >/tmp/check-remote-desktop-contract-boundary-subject.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "default-subject permission probe vocabulary should exit 1 (got $rc)"

perl -0pi -e 's/permission_probe_accepts_default_user_subject/permission_probe_accepts_authenticated_user_self_subject/' \
  "$SB/plugins/remote-desktop/src/handlers/permission_status.rs"
perl -0pi -e 's/scope_subjects_uras = \["agent", "resource", "user"\]/scope_subjects_uras = ["agent", "user"]/' \
  "$SB/plugins/remote-desktop/abilities/remote_desktop.request_permission.ability.toml"

set +e
(
  cd "$SB"
  bash tools/scripts/check-remote-desktop-contract-boundary.sh
) >/tmp/check-remote-desktop-contract-boundary-toml.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "permission descriptor scope drift should exit 1 (got $rc)"
grep -q "must admit Agent/User plus descriptor-bound Resource subjects" \
  /tmp/check-remote-desktop-contract-boundary-toml.out || fail "expected TOML scope failure message"

perl -0pi -e 's/scope_subjects_uras = \["agent", "user"\]/scope_subjects_uras = ["agent", "resource", "user"]/' \
  "$SB/plugins/remote-desktop/abilities/remote_desktop.request_permission.ability.toml"
perl -0pi -e 's#subject_contract_ura = "easynet:///r/_system/resource/ability-contract.remote-desktop/host-local-permission-subject"\R##' \
  "$SB/plugins/remote-desktop/abilities/remote_desktop.permission_status.ability.toml"

set +e
(
  cd "$SB"
  bash tools/scripts/check-remote-desktop-contract-boundary.sh
) >/tmp/check-remote-desktop-contract-boundary-subject-contract.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing host-local subject contract URA should exit 1 (got $rc)"
grep -q "host-local permission subject policy URA" \
  /tmp/check-remote-desktop-contract-boundary-subject-contract.out || fail "expected subject contract URA failure message"

cat >"$SB/plugins/remote-desktop/abilities/remote_desktop.permission_status.ability.toml" <<'TOML'
subject_contract_ura = "easynet:///r/_system/resource/ability-contract.remote-desktop/host-local-permission-subject"
scope_subjects_uras = ["agent", "resource", "user"]
TOML

perl -0pi -e 's/"subject_contract": host_local_permission_subject_contract\(\),//' \
  "$SB/plugins/remote-desktop/src/permissions.rs"

set +e
(
  cd "$SB"
  bash tools/scripts/check-remote-desktop-contract-boundary.sh
) >/tmp/check-remote-desktop-contract-boundary-response-contract.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing response subject contract should exit 1 (got $rc)"
grep -q "responses must include the host-local subject contract" \
  /tmp/check-remote-desktop-contract-boundary-response-contract.out || fail "expected response contract failure message"

cat >"$SB/plugins/remote-desktop/src/permissions.rs" <<'RS'
const REMOTE_DESKTOP_HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA: &str =
    "easynet:///r/_system/resource/ability-contract.remote-desktop/host-local-permission-subject";

enum HostLocalPermissionProbeSubject {
    UserSelf,
    UserInvokeResource,
    LocalSystemLoopback,
}

fn host_local_subject_error() {
    RemoteDesktopError::InvalidArgument;
}

fn host_local_permission_subject_contract() {
    json!({
        "subject_contract_ura": REMOTE_DESKTOP_HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA,
        "allowed_subjects": [
            "caller_user_self",
            "descriptor_bound_invoke_resource",
            "local_system_loopback",
        ],
        "target_resource_subjects_allowed": false,
    });
}

fn screen_capture_permission_status() {
    json!({
        "subject_contract": host_local_permission_subject_contract(),
        "input_permission": {
            "permission": "accessibility",
        },
    });
}

fn request_screen_capture_permission() {
    request_input_injection_permission();
    json!({
        "subject_contract": host_local_permission_subject_contract(),
        "input_permission": {
            "permission": "accessibility",
        },
    });
}
RS

perl -0pi -e 's/"target_resource_subjects_allowed": false/"target_resource_subjects_allowed": true/' \
  "$SB/plugins/remote-desktop/src/permissions.rs"

set +e
(
  cd "$SB"
  bash tools/scripts/check-remote-desktop-contract-boundary.sh
) >/tmp/check-remote-desktop-contract-boundary-target-resource.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "target-resource permission probe drift should exit 1 (got $rc)"
grep -q "responses must explicitly reject target resource subjects" \
  /tmp/check-remote-desktop-contract-boundary-target-resource.out || fail "expected target resource rejection failure message"

echo "test_check_remote_desktop_contract_boundary.sh: all cases passed"
