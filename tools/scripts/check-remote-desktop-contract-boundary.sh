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

[[ -f "$CONTRACT" ]] || fail "missing $CONTRACT"
[[ -f "$PERMISSIONS" ]] || fail "missing $PERMISSIONS"
[[ -f "$PERMISSION_STATUS" ]] || fail "missing $PERMISSION_STATUS"

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

if ! rg -n 'UserSelf|LocalSystemLoopback' "$PERMISSIONS" >/dev/null; then
  fail "remote-desktop permission subject policy must name user-self and local-system loopback states"
fi

if rg -n 'default_user_subject|accepts_default|default subject|device default subject' "$PERMISSIONS" "$PERMISSION_STATUS"; then
  fail "remote-desktop permission probes must not preserve default-subject compatibility vocabulary"
fi

for test_name in \
  permission_probe_accepts_authenticated_user_self_subject \
  permission_probe_rejects_non_caller_user_subject \
  permission_probe_rejects_device_subject_before_defaulting \
  permission_probe_accepts_local_system_loopback_subject
do
  if ! rg -n "$test_name" "$PERMISSION_STATUS" >/dev/null; then
    fail "remote-desktop permission probe boundary test missing: $test_name"
  fi
done

echo "check-remote-desktop-contract-boundary: ok"
