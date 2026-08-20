#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

PERMISSION="src/daemon/execution/permission/mod.rs"
KERNEL="src/daemon/boot/kernel/mod.rs"
CONSENT="src/daemon/ability/builtins/governance/consent.rs"
DAEMON_BIN="src/bin/easynet-daemon.rs"
TARGETS=("$PERMISSION" "$KERNEL" "$CONSENT" "$DAEMON_BIN")

for target in "${TARGETS[@]}"; do
  [[ -f "$target" ]] || fail "missing permission broker target: $target"
done

if rg -n \
  'AllowAllBroker|with_subscriber_broker|fallback[^[:cntrl:]]*Allow|Allow[^[:cntrl:]]*fallback|default broker auto-allows|default-allow|fail-open|pre-PR[^[:cntrl:]]*permission|compat[^[:cntrl:]]*permission' \
  "${TARGETS[@]}"; then
  fail "permission broker production path still describes headless operation as legacy/default fallback behavior"
fi

if ! rg -n 'struct HeadlessPermissionBroker' "$PERMISSION" >/dev/null; then
  fail "permission broker must expose an explicit HeadlessPermissionBroker"
fi

if ! rg -n 'enum UnobservedPermissionPolicy' "$PERMISSION" >/dev/null; then
  fail "permission broker must model no-observer behavior as UnobservedPermissionPolicy"
fi

if ! rg -n 'pub fn headless\(\) -> Self' "$PERMISSION" >/dev/null; then
  fail "PermissionService must name the headless constructor explicitly"
fi

if ! rg -n 'pub fn interactive\(\) -> Self' "$PERMISSION" >/dev/null; then
  fail "PermissionService must name the interactive constructor explicitly"
fi

if ! rg -n 'unobserved_policy: UnobservedPermissionPolicy' "$PERMISSION" >/dev/null; then
  fail "SubscriberBroker must own an explicit unobserved policy"
fi

if ! rg -n 'return self\.unobserved_policy\.decide\(\);' "$PERMISSION" >/dev/null; then
  fail "SubscriberBroker must route no-observer asks through UnobservedPermissionPolicy"
fi

if ! rg -n 'pub fn new_interactive\(\) -> Self' "$KERNEL" >/dev/null; then
  fail "Kernel must name the interactive boot state explicitly"
fi

if ! rg -n 'PermissionService::interactive\(\)' "$KERNEL" >/dev/null; then
  fail "Kernel interactive constructor must install PermissionService::interactive"
fi

if ! rg -n 'Kernel::new_interactive\(\)' "$DAEMON_BIN" >/dev/null; then
  fail "daemon boot must install the explicit interactive Kernel"
fi

