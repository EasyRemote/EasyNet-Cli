#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

TARGET="src/cli/commands/status.rs"

[[ -f "$TARGET" ]] || fail "missing $TARGET"

if ! rg -n 'enum StatusPairingState' "$TARGET" >/dev/null; then
  fail "runtime status pairing must be represented as an explicit state enum"
fi

if ! rg -n 'load_credentials_optional\(\)' "$TARGET" >/dev/null; then
  fail "runtime status pairing must distinguish missing credentials with load_credentials_optional"
fi

if rg -n 'if let Ok\(creds\) = config::load_credentials\(\)|let Ok\(creds\) = config::load_credentials\(\)|config::load_credentials\(\)|load_credentials\(\)\.ok\(\)' "$TARGET"; then
  fail "runtime status must not collapse invalid credentials into not-paired state"
fi

for required_test in \
  status_pairing_state_reports_paired_credentials \
  status_pairing_state_reports_unpaired_only_for_missing_credentials \
  status_pairing_state_rejects_malformed_credentials_as_invalid
do
  if ! rg -n "$required_test" "$TARGET" >/dev/null; then
    fail "runtime status pairing state missing test: $required_test"
  fi
done

echo "check-status-pairing-state-boundary: ok"
