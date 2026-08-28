#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-status-pairing-state-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/cli/commands"
cp "$SCRIPT" "$SB/tools/scripts/check-status-pairing-state-boundary.sh"

cat >"$SB/src/cli/commands/status.rs" <<'RS'
enum StatusPairingState {
    Paired(Credentials),
    Unpaired,
    Invalid { reason: String },
}

impl StatusPairingState {
    fn load() -> Self {
        match config::load_credentials_optional() {
            Ok(Some(credentials)) => Self::Paired(credentials),
            Ok(None) => Self::Unpaired,
            Err(error) => Self::Invalid { reason: format!("{error:#}") },
        }
    }

    fn to_json(&self) -> Value {
        json!({"state": "paired", "current_user": {"state": "bound", "ura": "easynet:///r/localhost/user/alice"}})
    }
}

fn run_json() {
    let pairing = StatusPairingState::load().to_json();
    let mut payload = json!({});
    payload["pairing"] = pairing;
}

#[test]
fn status_pairing_state_reports_paired_credentials() {}

#[test]
fn status_pairing_json_projects_bound_current_user_identity() {}

#[test]
fn status_pairing_state_reports_unpaired_only_for_missing_credentials() {}

#[test]
fn status_pairing_state_rejects_malformed_credentials_as_invalid() {}
RS

(
  cd "$SB"
  bash tools/scripts/check-status-pairing-state-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >>"$SB/src/cli/commands/status.rs" <<'RS'
fn collapsed_status_pairing() {
    if let Ok(creds) = config::load_credentials() {
        let _ = creds;
    } else {
        eprintln!("Device: not paired");
    }
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-status-pairing-state-boundary.sh
) >/tmp/check-status-pairing-state-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "credential error collapse should exit 1 (got $rc)"

echo "test_check_status_pairing_state_boundary.sh: all cases passed"
