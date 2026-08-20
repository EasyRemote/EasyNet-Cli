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
struct CallParticipantIdentity {
    node_id: String,
}

impl CallParticipantIdentity {
    fn resolve_paired_device() -> anyhow::Result<Self> {
        let credentials = crate::daemon::persistence::config::load_credentials_optional()?
            .ok_or_else(|| anyhow::anyhow!("call participant identity requires paired device credentials"))?;
        Ok(Self { node_id: credentials.node_id })
    }
}

struct CallSignalingIssuer;

impl CallSignalingIssuer {
    fn invoke(ability: &str, args: Value) -> anyhow::Result<Value> {
        if let Some(value) = invoke_current_realm_hub_system_ability(ability, args.clone())? {
            return Ok(value);
        }
        Self::invoke_local(ability, args)
    }

    fn invoke_local(ability: &str, args: Value) -> anyhow::Result<Value> {
        LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity(ability, args)
    }
}

#[test]
fn call_participant_rejects_unpaired_hostname_fallback() {}

#[test]
fn call_participant_rejects_malformed_credentials() {}

#[test]
fn call_participant_rejects_incomplete_credentials() {}
RS

(
  cd "$SB"
  bash tools/scripts/check-call-create-participant-identity-boundary.sh
) >/dev/null || fail "paired-device happy path should pass"

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

cp "$SCRIPT" "$SB/tools/scripts/check-call-create-participant-identity-boundary.sh"
cat > "$SB/src/cli/commands/groups/call.rs" <<'RS'
struct CallParticipantIdentity {
    node_id: String,
}

impl CallParticipantIdentity {
    fn resolve_paired_device() -> anyhow::Result<Self> {
        let credentials = crate::daemon::persistence::config::load_credentials_optional()?
            .ok_or_else(|| anyhow::anyhow!("call participant identity requires paired device credentials"))?;
        Ok(Self { node_id: credentials.node_id })
    }
}

struct CallSignalingIssuer;

impl CallSignalingIssuer {
    fn invoke(ability: &str, args: Value) -> anyhow::Result<Value> {
        invoke_local_ability(ability, args)
    }
}

#[test]
fn call_participant_rejects_unpaired_hostname_fallback() {}

#[test]
fn call_participant_rejects_malformed_credentials() {}

#[test]
fn call_participant_rejects_incomplete_credentials() {}
RS
set +e
(
  cd "$SB"
  bash tools/scripts/check-call-create-participant-identity-boundary.sh
) >/tmp/check-call-create-participant-identity.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "generic call signaling invoke should exit 1 (got $rc)"

cp "$SCRIPT" "$SB/tools/scripts/check-call-create-participant-identity-boundary.sh"
cat > "$SB/src/cli/commands/groups/call.rs" <<'RS'
struct CallParticipantIdentity {
    node_id: String,
}

impl CallParticipantIdentity {
    fn resolve_paired_device() -> anyhow::Result<Self> {
        let Some(credentials) = crate::daemon::persistence::config::load_credentials_optional()? else {
            return Ok(Self { node_id: gethostname::gethostname().to_string_lossy().to_string() });
        };
        Ok(Self { node_id: credentials.node_id })
    }
}

struct CallSignalingIssuer;

impl CallSignalingIssuer {
    fn invoke(ability: &str, args: Value) -> anyhow::Result<Value> {
        if let Some(value) = invoke_current_realm_hub_system_ability(ability, args.clone())? {
            return Ok(value);
        }
        Self::invoke_local(ability, args)
    }

    fn invoke_local(ability: &str, args: Value) -> anyhow::Result<Value> {
        LocalDaemonSystemAbilityIssuer::invoke_root_for_local_daemon_identity(ability, args)
    }
}

#[test]
fn call_participant_rejects_unpaired_hostname_fallback() {}

#[test]
fn call_participant_rejects_malformed_credentials() {}

#[test]
fn call_participant_rejects_incomplete_credentials() {}
RS
set +e
(
  cd "$SB"
  bash tools/scripts/check-call-create-participant-identity-boundary.sh
) >/tmp/check-call-create-participant-identity.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "unpaired hostname fallback should exit 1 (got $rc)"

echo "test_check_call_create_participant_identity_boundary.sh: all cases passed"
