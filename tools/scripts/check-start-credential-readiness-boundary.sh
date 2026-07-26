#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

TARGET="src/cli/commands/start.rs"

[[ -f "$TARGET" ]] || fail "missing $TARGET"

if ! rg -n 'enum StartCredentialReadiness' "$TARGET" >/dev/null; then
  fail "start credential preflight must be represented as an explicit readiness enum"
fi

if ! rg -n 'load_credentials_optional\(\)' "$TARGET" >/dev/null; then
  fail "start credential preflight must distinguish missing credentials with load_credentials_optional"
fi

if rg -n 'let Ok\(creds\) = config::load_credentials\(\)|if let Ok\(creds\) = config::load_credentials\(\)|config::load_credentials\(\)\.ok\(\)' "$TARGET"; then
  fail "start credential preflight must not collapse invalid credentials into missing state"
fi

for required_test in \
  start_credential_readiness_reports_ready_credentials \
  start_credential_readiness_reports_missing_only_for_absent_credentials \
  start_credential_readiness_reports_invalid_existing_credentials \
  start_after_local_state_purge_fails_without_runtime_projection_side_effect \
  load_and_verify_credentials_rejects_invalid_credentials_before_verify
do
  if ! rg -n "$required_test" "$TARGET" >/dev/null; then
    fail "start credential readiness missing test: $required_test"
  fi
done

echo "check-start-credential-readiness-boundary: ok"
