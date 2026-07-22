#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-reset-credential-state-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/cli/commands"
cp "$SCRIPT" "$SB/tools/scripts/check-reset-credential-state-boundary.sh"

cat >"$SB/src/cli/commands/reset.rs" <<'RS'
enum ResetCredentialState {
    Paired(Credentials),
    Missing,
    Invalid { reason: String },
}

impl ResetCredentialState {
    fn load() -> Self {
        match config::load_credentials_optional() {
            Ok(Some(credentials)) => Self::Paired(credentials),
            Ok(None) => Self::Missing,
            Err(error) => Self::Invalid { reason: format!("{error:#}") },
        }
    }
}

#[test]
fn reset_credential_state_reports_paired_credentials() {}

#[test]
fn reset_credential_state_reports_missing_only_for_absent_credentials() {}

#[test]
fn reset_credential_state_reports_invalid_existing_credentials() {}

#[test]
fn reset_deletes_malformed_credentials_without_classifying_as_missing() {}
RS

(
  cd "$SB"
  bash tools/scripts/check-reset-credential-state-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >>"$SB/src/cli/commands/reset.rs" <<'RS'
fn collapsed_prompt() -> String {
    config::load_credentials()
        .ok()
        .map(|credentials| credentials.node_id)
        .unwrap_or_else(|| "<no credentials on disk>".to_string())
}

fn collapsed_revoke() {
    if let Ok(creds) = config::load_credentials() {
        let _ = creds;
    }
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-reset-credential-state-boundary.sh
) >/tmp/check-reset-credential-state-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "credential collapse should exit 1 (got $rc)"

echo "test_check_reset_credential_state_boundary.sh: all cases passed"
