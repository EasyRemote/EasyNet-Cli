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
  "src/cli/commands/status.rs"
  "src/cli/daemon_client/ability_catalog.rs"
  "src/cli/commands/groups/invocation.rs"
  "src/cli/commands/invocation_watch.rs"
)

[[ -f "$ISSUER" ]] || fail "missing $ISSUER"

if ! rg -n 'struct LocalRuntimeStateReadIssuer' "$ISSUER" >/dev/null; then
  fail "runtime-state reads must use a named issuer"
fi

if ! rg -n 'local_invocation::local_daemon_ura\(\)' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must bind subject from control-discovery daemon identity"
fi

if ! rg -n 'runtime_state_read_subject_requires_control_discovery_identity' "$ISSUER" >/dev/null; then
  fail "runtime-state read issuer must test missing control discovery as fail-closed"
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
