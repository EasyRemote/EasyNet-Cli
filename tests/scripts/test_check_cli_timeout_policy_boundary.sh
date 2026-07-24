#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-cli-timeout-policy-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/support/platform" "$SB/src/cli/commands"
cp "$SCRIPT" "$SB/tools/scripts/check-cli-timeout-policy-boundary.sh"

cat >"$SB/src/support/platform/timeouts.rs" <<'RS'
enum ZeroTimeoutPolicy {
    RuntimeDefault,
    DefaultTransportGuard,
}

pub struct TimeoutPolicy;

pub const INVOKE_DEFAULT_SECS: u64 = 3600;

pub fn effective_ms(_secs: u64) -> Result<Option<u64>, &'static str> {
    Ok(None)
}

pub fn invocation_transport_guard(_secs: u64) -> Result<std::time::Duration, &'static str> {
    Ok(std::time::Duration::from_secs(INVOKE_DEFAULT_SECS))
}

pub fn runtime_request_timeout_ms(_secs: u64) -> Result<Option<u64>, &'static str> {
    Ok(None)
}

#[test]
fn invocation_transport_guard_uses_default_guard_for_zero() {}

#[test]
fn runtime_request_timeout_preserves_zero_as_runtime_default() {}
RS

for file in invoke ability_stream ability_bidi ability_record; do
  cat >"$SB/src/cli/commands/${file}.rs" <<'RS'
fn run(args: Args) -> anyhow::Result<()> {
    let timeout = timeouts::invocation_transport_guard(args.timeout)?;
    Ok(())
}
RS
done

cat >"$SB/src/cli/commands/exec.rs" <<'RS'
fn run(args: Args) -> anyhow::Result<()> {
    let timeout_ms = timeouts::runtime_request_timeout_ms(args.timeout)?;
    Ok(())
}
RS

(
  cd "$SB"
  bash tools/scripts/check-cli-timeout-policy-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >>"$SB/src/cli/commands/invoke.rs" <<'RS'
fn legacy_timeout(args: Args) -> anyhow::Result<()> {
    let timeout_ms = timeouts::effective_ms(args.timeout)?.unwrap_or(timeouts::INVOKE_DEFAULT_SECS * 1000);
    Ok(())
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-cli-timeout-policy-boundary.sh
) >/tmp/check-cli-timeout-policy-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "manual timeout fallback should exit 1 (got $rc)"

echo "test_check_cli_timeout_policy_boundary.sh: all cases passed"
