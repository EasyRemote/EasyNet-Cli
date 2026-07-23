#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-runtime-state-read-subject-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p \
  "$SB/tools/scripts" \
  "$SB/src/support/platform" \
  "$SB/src/cli/commands/groups" \
  "$SB/src/cli/daemon_client"
cp "$SCRIPT" "$SB/tools/scripts/check-runtime-state-read-subject-boundary.sh"

cat >"$SB/src/support/platform/local_invoke.rs" <<'RS'
pub struct LocalRuntimeStateReadIssuer;

impl LocalRuntimeStateReadIssuer {
    fn invoke(_ability: &str, _args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let _ = Self::subject_ura()?;
        Ok(serde_json::json!({}))
    }

    fn subject_ura() -> anyhow::Result<String> {
        LocalRuntimeStateReadSubject::from_credentials_file().map(|subject| subject.into_ura())
    }
}

struct LocalRuntimeStateReadSubject {
    ura: String,
}

impl LocalRuntimeStateReadSubject {
    fn from_credentials_file() -> anyhow::Result<Self> {
        let credentials = crate::daemon::persistence::config::load_credentials()?;
        let user_id = credentials.user_id()?;
        Ok(Self {
            ura: format!("easynet:///r/acme/resource/user.{user_id}/runtime-state/read"),
        })
    }

    fn into_ura(self) -> String {
        self.ura
    }
}

#[test]
fn runtime_state_read_subject_uses_user_owned_resource_not_daemon_identity() {}

#[test]
fn runtime_state_read_subject_rejects_missing_user_id_before_device_fallback() {}
RS

for target in \
  "$SB/src/cli/commands/ability_record.rs" \
  "$SB/src/cli/commands/discover.rs" \
  "$SB/src/cli/commands/doctor.rs" \
  "$SB/src/cli/commands/groups/mcp.rs" \
  "$SB/src/cli/commands/status.rs" \
  "$SB/src/cli/daemon_client/ability_catalog.rs" \
  "$SB/src/cli/commands/groups/invocation.rs" \
  "$SB/src/cli/commands/invocation_watch.rs"
do
  cat >"$target" <<'RS'
use crate::support::platform::local_invoke::LocalRuntimeStateReadIssuer;

fn read_runtime_state() {
    let _ = LocalRuntimeStateReadIssuer::invoke("meta.list_abilities", serde_json::json!({}));
}
RS
done

(
  cd "$SB"
  bash tools/scripts/check-runtime-state-read-subject-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >>"$SB/src/cli/commands/groups/invocation.rs" <<'RS'
fn legacy_read() {
    let _ = invoke_local_ability("invocation.history.list", serde_json::json!({}));
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-runtime-state-read-subject-boundary.sh
) >/tmp/check-runtime-state-read-subject-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "generic runtime-state read should exit 1 (got $rc)"

echo "test_check_runtime_state_read_subject_boundary.sh: all cases passed"
