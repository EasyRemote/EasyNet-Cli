#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

ISSUER="src/support/platform/local_invoke.rs"
TARGETS=(
  "src/cli/commands/ability_record.rs"
  "src/cli/commands/discover.rs"
  "src/cli/commands/doctor.rs"
  "src/cli/commands/groups/mcp.rs"
  "src/cli/commands/status.rs"
  "src/cli/daemon_client/ability_catalog.rs"
  "src/cli/commands/groups/invocation.rs"
  "src/cli/commands/invocation_watch.rs"
)

[[ -f "$ISSUER" ]] || fail "missing $ISSUER"

if ! rg -n 'struct LocalRuntimeStateReadIssuer' "$ISSUER" >/dev/null; then
  fail "runtime-state reads must use a named issuer"
fi

if ! rg -n 'struct LocalRuntimeStateReadSubject' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must own an explicit read-subject value object"
fi

if ! rg -n 'runtime-state/read' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must bind a dedicated runtime-state resource subject"
fi

if ! rg -n 'persistence::config::load_credentials\(\)' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must derive subject ownership from paired credentials"
fi

if ! rg -n '\.user_id\(\)' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must require a paired user id"
fi

if rg -n 'local_invocation::local_daemon_ura\(\)|local_invocation::local_device_ura\(\)|UNPAIRED_LOCAL_REALM|UNPAIRED_LOCAL_DEVICE_ID' "$ISSUER"; then
  fail "runtime-state read issuer must not fall back to daemon/device/default subjects"
fi

if ! rg -n 'runtime_state_read_subject_uses_user_owned_resource_not_daemon_identity' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must test user-owned resource subject projection"
fi

if ! rg -n 'runtime_state_read_subject_rejects_missing_user_id_before_device_fallback' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must test missing user id as fail-closed"
fi

for target in "${TARGETS[@]}"; do
  [[ -f "$target" ]] || fail "missing $target"
  if ! rg -n 'LocalRuntimeStateReadIssuer::invoke' "$target" >/dev/null; then
    fail "$target must enter local runtime through LocalRuntimeStateReadIssuer"
  fi
  if rg -n '\binvoke_local_ability\s*\(' "$target"; then
    fail "$target must not use generic invoke_local_ability for runtime-state reads"
  fi
done

echo "check-runtime-state-read-subject-boundary: ok"
