#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

CONTRACT="plugins/remote-desktop/src/contract.rs"
PERMISSIONS="plugins/remote-desktop/src/permissions.rs"
PERMISSION_STATUS="plugins/remote-desktop/src/handlers/permission_status.rs"
REQUEST_PERMISSION="plugins/remote-desktop/src/handlers/request_permission.rs"
REGISTRATION="plugins/remote-desktop/src/registration.rs"
PERMISSION_STATUS_TOML="plugins/remote-desktop/abilities/remote_desktop.permission_status.ability.toml"
REQUEST_PERMISSION_TOML="plugins/remote-desktop/abilities/remote_desktop.request_permission.ability.toml"
HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA='easynet:///r/_system/resource/ability-contract.remote-desktop/host-local-permission-subject'

[[ -f "$CONTRACT" ]] || fail "missing $CONTRACT"
[[ -f "$PERMISSIONS" ]] || fail "missing $PERMISSIONS"
[[ -f "$PERMISSION_STATUS" ]] || fail "missing $PERMISSION_STATUS"
[[ -f "$REQUEST_PERMISSION" ]] || fail "missing $REQUEST_PERMISSION"
[[ -f "$REGISTRATION" ]] || fail "missing $REGISTRATION"
[[ -f "$PERMISSION_STATUS_TOML" ]] || fail "missing $PERMISSION_STATUS_TOML"
[[ -f "$REQUEST_PERMISSION_TOML" ]] || fail "missing $REQUEST_PERMISSION_TOML"

if rg -n '#\[serde\([^]]*\balias\b|alias\s*=\s*"web_rtc"' "$CONTRACT"; then
  fail "remote-desktop contract must not accept retired transport aliases"
fi

if ! rg -n '#\[serde\(rename = "webrtc"\)\]' "$CONTRACT" >/dev/null; then
  fail "remote-desktop contract must keep one canonical webrtc transport wire name"
fi

if ! rg -n 'media_backend_contract_accepts_only_canonical_webrtc_transport_name' "$CONTRACT" >/dev/null; then
  fail "remote-desktop contract must test canonical webrtc transport decoding"
fi

if ! rg -n 'media_backend_contract_rejects_retired_web_rtc_transport_alias' "$CONTRACT" >/dev/null; then
  fail "remote-desktop contract must test retired web_rtc transport rejection"
fi

if ! rg -n 'enum HostLocalPermissionProbeSubject' "$PERMISSIONS" >/dev/null; then
  fail "remote-desktop permission probes must use an explicit host-local subject policy"
fi

if ! rg -n 'UserSelf|UserInvokeResource|LocalSystemLoopback' "$PERMISSIONS" >/dev/null; then
  fail "remote-desktop permission subject policy must name user-self, descriptor-bound invoke resource, and local-system loopback states"
fi

if ! rg -n 'RemoteDesktopError::InvalidArgument' "$PERMISSIONS" >/dev/null; then
  fail "remote-desktop permission subject failures must use typed RemoteDesktopError::InvalidArgument"
fi

if ! rg -n 'const PERMISSION_PROBE_SUBJECT_KINDS: &\[&str\] = &\["agent", "resource", "user"\];' "$REGISTRATION" >/dev/null; then
  fail "remote-desktop permission descriptors must admit Agent/User plus descriptor-bound Resource subjects"
fi

for descriptor in "$PERMISSION_STATUS_TOML" "$REQUEST_PERMISSION_TOML"; do
  if ! rg -n 'scope_subjects_uras = \["agent", "resource", "user"\]' "$descriptor" >/dev/null; then
    fail "$descriptor must admit Agent/User plus descriptor-bound Resource subjects"
  fi
  if ! rg -n "subject_contract_ura = \"$HOST_LOCAL_PERMISSION_SUBJECT_CONTRACT_URA\"" "$descriptor" >/dev/null; then
    fail "$descriptor must publish the host-local permission subject policy URA"
  fi
done

for handler in "$PERMISSION_STATUS" "$REQUEST_PERMISSION"; do
  if ! rg -n 'ensure_permission_probe_access' "$handler" >/dev/null; then
    fail "$handler must use the shared host-local permission subject policy"
  fi
done

if rg -n 'default_user_subject|accepts_default|default subject|device default subject' "$PERMISSIONS" "$PERMISSION_STATUS"; then
  fail "remote-desktop permission probes must not preserve default-subject compatibility vocabulary"
fi

for test_name in \
  permission_probe_accepts_authenticated_user_self_subject \
  permission_probe_rejects_non_caller_user_subject \
  permission_probe_rejects_device_subject_before_defaulting \
  permission_probe_accepts_local_system_loopback_subject \
  permission_probe_accepts_descriptor_bound_user_invoke_resource_subject \
  permission_probe_rejects_device_stream_resource_subject
do
  if ! rg -n "$test_name" "$PERMISSION_STATUS" >/dev/null; then
    fail "remote-desktop permission probe boundary test missing: $test_name"
  fi
done

for test_name in \
  request_permission_rejects_device_stream_resource_subject_before_os_prompt \
  request_permission_rejects_target_subject_in_args_before_os_prompt
do
  if ! rg -n "$test_name" "$REQUEST_PERMISSION" >/dev/null; then
    fail "remote-desktop request_permission boundary test missing: $test_name"
  fi
done

echo "check-remote-desktop-contract-boundary: ok"
