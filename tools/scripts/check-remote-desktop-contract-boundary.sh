#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

CONTRACT="plugins/remote-desktop/src/contract.rs"

[[ -f "$CONTRACT" ]] || fail "missing $CONTRACT"

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

echo "check-remote-desktop-contract-boundary: ok"
