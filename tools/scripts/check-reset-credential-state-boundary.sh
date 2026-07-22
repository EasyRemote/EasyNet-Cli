#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

TARGET="src/cli/commands/reset.rs"

[[ -f "$TARGET" ]] || fail "missing $TARGET"

if ! rg -n 'enum ResetCredentialState' "$TARGET" >/dev/null; then
  fail "reset credentials must be represented as an explicit state enum"
fi

if ! rg -n 'load_credentials_optional\(\)' "$TARGET" >/dev/null; then
  fail "reset credential state must distinguish missing credentials with load_credentials_optional"
fi

if rg -n 'load_credentials\(\)\s*\.ok\(\)|if let Ok\(creds\) = config::load_credentials\(\)|let Ok\(creds\) = config::load_credentials\(\)' "$TARGET"; then
  fail "reset must not collapse invalid credentials into missing/no-credentials state"
fi

for required_test in \
  reset_credential_state_reports_paired_credentials \
  reset_credential_state_reports_missing_only_for_absent_credentials \
  reset_credential_state_reports_invalid_existing_credentials \
  reset_deletes_malformed_credentials_without_classifying_as_missing
do
  if ! rg -n "$required_test" "$TARGET" >/dev/null; then
    fail "reset credential state missing test: $required_test"
  fi
done

echo "check-reset-credential-state-boundary: ok"
