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
enum HostLocalPermissionProbeSubject {
    UserSelf,
    UserInvokeResource,
    LocalSystemLoopback,
}

fn host_local_subject_error() {
    RemoteDesktopError::InvalidArgument;
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
scope_subjects_uras = ["agent", "resource", "user"]
TOML

cat >"$SB/plugins/remote-desktop/abilities/remote_desktop.request_permission.ability.toml" <<'TOML'
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

echo "test_check_remote_desktop_contract_boundary.sh: all cases passed"
