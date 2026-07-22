#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-current-realm-hub-context-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/cli/daemon_client"
cp "$SCRIPT" "$SB/tools/scripts/check-current-realm-hub-context-boundary.sh"

cat > "$SB/src/cli/daemon_client/remote_system_ability.rs" <<'RS'
enum CurrentRealmHubInvocationContext {
    Ready,
    Unpaired,
}

fn resolve() -> anyhow::Result<CurrentRealmHubInvocationContext> {
    let Some(_creds) = crate::daemon::persistence::config::load_credentials_optional()? else {
        return Ok(CurrentRealmHubInvocationContext::Unpaired);
    };
    Ok(CurrentRealmHubInvocationContext::Ready)
}

#[test]
fn current_realm_hub_context_rejects_malformed_credentials() {}

#[test]
fn current_realm_hub_context_rejects_incomplete_credentials() {}
RS

(
  cd "$SB"
  bash tools/scripts/check-current-realm-hub-context-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >> "$SB/src/cli/daemon_client/remote_system_ability.rs" <<'RS'
fn collapsed() -> anyhow::Result<Option<()>> {
    let Ok(creds) = crate::daemon::persistence::config::load_credentials() else {
        return Ok(None);
    };
    let _ = creds;
    Ok(Some(()))
}
RS
set +e
(
  cd "$SB"
  bash tools/scripts/check-current-realm-hub-context-boundary.sh
) >/tmp/check-current-realm-hub-context.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "credential collapse should exit 1 (got $rc)"

echo "test_check_current_realm_hub_context_boundary.sh: all cases passed"
