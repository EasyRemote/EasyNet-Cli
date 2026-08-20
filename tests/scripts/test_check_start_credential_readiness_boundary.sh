#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-start-credential-readiness-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/cli/commands"
cp "$SCRIPT" "$SB/tools/scripts/check-start-credential-readiness-boundary.sh"

cat >"$SB/src/cli/commands/start.rs" <<'RS'
enum StartCredentialReadiness {
    Ready(Credentials),
    Missing,
    Invalid { reason: String },
}

impl StartCredentialReadiness {
    fn load() -> Self {
        match config::load_credentials_optional() {
            Ok(Some(credentials)) => Self::Ready(credentials),
            Ok(None) => Self::Missing,
            Err(error) => Self::Invalid { reason: format!("{error:#}") },
        }
    }
}

#[test]
fn start_credential_readiness_reports_ready_credentials() {}

#[test]
fn start_credential_readiness_reports_missing_only_for_absent_credentials() {}

#[test]
fn start_credential_readiness_reports_invalid_existing_credentials() {}

#[test]
fn start_after_local_state_purge_fails_without_runtime_projection_side_effect() {}

#[test]
fn load_and_verify_credentials_rejects_invalid_credentials_before_verify() {}
RS

(
  cd "$SB"
  bash tools/scripts/check-start-credential-readiness-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >>"$SB/src/cli/commands/start.rs" <<'RS'
fn collapsed_start_preflight() -> anyhow::Result<()> {
    let Ok(creds) = config::load_credentials() else {
        anyhow::bail!("no credentials")
    };
    let _ = creds;
    Ok(())
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-start-credential-readiness-boundary.sh
) >/tmp/check-start-credential-readiness-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "credential readiness collapse should exit 1 (got $rc)"

echo "test_check_start_credential_readiness_boundary.sh: all cases passed"
