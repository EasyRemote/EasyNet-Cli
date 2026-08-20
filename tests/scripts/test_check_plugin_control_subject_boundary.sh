#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-plugin-control-subject-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/cli/commands/groups"
cp "$SCRIPT" "$SB/tools/scripts/check-plugin-control-subject-boundary.sh"

cat > "$SB/src/cli/commands/groups/plugin.rs" <<'RS'
enum PluginControlSubject {
    Available(String),
    Unpaired,
}

fn resolve() -> anyhow::Result<PluginControlSubject> {
    let Some(creds) = crate::daemon::persistence::config::load_credentials_optional()? else {
        return Ok(PluginControlSubject::Unpaired);
    };
    Ok(PluginControlSubject::Available(creds.node_id))
}

#[test]
fn plugin_control_subject_rejects_malformed_credentials() {}

#[test]
fn plugin_control_subject_rejects_incomplete_credentials() {}
RS

(
  cd "$SB"
  bash tools/scripts/check-plugin-control-subject-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >> "$SB/src/cli/commands/groups/plugin.rs" <<'RS'
fn is_missing_or_incomplete_credentials(err: &anyhow::Error) -> bool {
    err.to_string().contains("credentials file is incomplete")
}
RS
set +e
(
  cd "$SB"
  bash tools/scripts/check-plugin-control-subject-boundary.sh
) >/tmp/check-plugin-control-subject.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "credential string sniffing should exit 1 (got $rc)"

cat > "$SB/src/cli/commands/groups/plugin.rs" <<'RS'
fn resolve() -> anyhow::Result<Option<String>> {
    crate::daemon::persistence::config::load_credentials().map(|creds| Some(creds.node_id))
}

#[test]
fn plugin_control_subject_rejects_malformed_credentials() {}

#[test]
fn plugin_control_subject_rejects_incomplete_credentials() {}
RS
set +e
(
  cd "$SB"
  bash tools/scripts/check-plugin-control-subject-boundary.sh
) >/tmp/check-plugin-control-subject.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "load_credentials collapse should exit 1 (got $rc)"

echo "test_check_plugin_control_subject_boundary.sh: all cases passed"
