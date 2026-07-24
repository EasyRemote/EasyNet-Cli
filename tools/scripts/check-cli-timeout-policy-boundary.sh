#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_CLI_TIMEOUT_POLICY_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-cli-timeout-policy-boundary: %s\n' "$1" >&2
  exit 1
}

TIMEOUTS="src/support/platform/timeouts.rs"
[[ -f "$TIMEOUTS" ]] || fail "missing $TIMEOUTS"

for token in \
  "pub struct TimeoutPolicy" \
  "enum ZeroTimeoutPolicy" \
  "pub fn invocation_transport_guard" \
  "pub fn runtime_request_timeout_ms" \
  "invocation_transport_guard_uses_default_guard_for_zero" \
  "runtime_request_timeout_preserves_zero_as_runtime_default"; do
  if ! rg -n "$token" "$TIMEOUTS" >/dev/null; then
    fail "timeout tower must expose canonical policy token: $token"
  fi
done

if rg -n 'legacy value kept for|back-compat' "$TIMEOUTS"; then
  fail "timeout tower must not preserve legacy/back-compat timeout policy language"
fi

if rg -n 'Default:\s*60\s*s|default is 15 min' src/cli/commands src/support/platform/timeouts.rs; then
  fail "CLI timeout docs must not advertise retired 60s/15min defaults"
fi

if rg -n 'timeouts::effective_ms\(' src/cli/commands; then
  fail "CLI commands must use named TimeoutPolicy helpers instead of raw effective_ms"
fi

if rg -n 'unwrap_or\s*\(\s*timeouts::INVOKE_DEFAULT_SECS\s*\*\s*1000\s*\)' src/cli/commands; then
  fail "CLI commands must not hand-roll invoke timeout fallback"
fi

for target in \
  "src/cli/commands/invoke.rs" \
  "src/cli/commands/ability_stream.rs" \
  "src/cli/commands/ability_bidi.rs" \
  "src/cli/commands/ability_record.rs"; do
  [[ -f "$target" ]] || fail "missing $target"
  if ! rg -n 'timeouts::invocation_transport_guard' "$target" >/dev/null; then
    fail "$target must resolve invocation transport guard through TimeoutPolicy"
  fi
done

if ! rg -n 'timeouts::runtime_request_timeout_ms' src/cli/commands/exec.rs >/dev/null; then
  fail "process exec must preserve runtime request timeout semantics through TimeoutPolicy"
fi

echo "check-cli-timeout-policy-boundary: ok"
