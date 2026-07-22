#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-call-create-participant-identity-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/cli/commands/groups"
cp "$SCRIPT" "$SB/tools/scripts/check-call-create-participant-identity-boundary.sh"

cat > "$SB/src/cli/commands/groups/call.rs" <<'RS'
enum CallCreateParticipantIdentity {
    DeviceNode(String),
    UnpairedHostname(String),
}

impl CallCreateParticipantIdentity {
    fn resolve() -> anyhow::Result<Self> {
        let Some(credentials) = crate::daemon::persistence::config::load_credentials_optional()? else {
            return Ok(Self::UnpairedHostname("host".to_string()));
        };
        Ok(Self::DeviceNode(credentials.node_id))
    }
}

#[test]
fn call_create_participant_rejects_malformed_credentials() {}

#[test]
fn call_create_participant_rejects_incomplete_credentials() {}
RS

(
  cd "$SB"
  bash tools/scripts/check-call-create-participant-identity-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >> "$SB/src/cli/commands/groups/call.rs" <<'RS'
fn collapsed() -> String {
    crate::daemon::persistence::config::load_credentials()
        .ok()
        .map(|creds| creds.node_id)
        .filter(|node_id| !node_id.trim().is_empty())
        .unwrap_or_else(|| gethostname::gethostname().to_string_lossy().to_string())
}
RS
set +e
(
  cd "$SB"
  bash tools/scripts/check-call-create-participant-identity-boundary.sh
) >/tmp/check-call-create-participant-identity.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "credential-to-hostname collapse should exit 1 (got $rc)"

echo "test_check_call_create_participant_identity_boundary.sh: all cases passed"
