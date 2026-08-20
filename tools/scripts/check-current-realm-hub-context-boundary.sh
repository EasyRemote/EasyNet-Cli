#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

TARGET="src/cli/daemon_client/remote_system_ability.rs"

[[ -f "$TARGET" ]] || fail "missing $TARGET"

if ! rg -n 'enum CurrentRealmHubInvocationContext' "$TARGET" >/dev/null; then
  fail "current-realm Hub dispatch must model credential state explicitly"
fi

if ! rg -n 'load_credentials_optional\(\)\?' "$TARGET" >/dev/null; then
  fail "current-realm Hub dispatch must distinguish missing credentials with load_credentials_optional"
fi

if rg -n 'load_credentials\(\)|let Ok\(creds\)|return Ok\(None\).*credentials|credentials.*return Ok\(None\)|unwrap_or_else\(\|\s*crate::support::platform::local_invoke::invoke_local_ability' "$TARGET"; then
  fail "current-realm Hub dispatch must not collapse credential errors into local fallback"
fi

if ! rg -n 'current_realm_hub_context_rejects_malformed_credentials' "$TARGET" >/dev/null; then
  fail "current-realm Hub dispatch must test malformed credentials as fail-closed"
fi

if ! rg -n 'current_realm_hub_context_rejects_incomplete_credentials' "$TARGET" >/dev/null; then
  fail "current-realm Hub dispatch must test incomplete credentials as fail-closed"
fi

echo "check-current-realm-hub-context-boundary: ok"
