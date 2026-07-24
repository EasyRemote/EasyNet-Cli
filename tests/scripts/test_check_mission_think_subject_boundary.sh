#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-mission-think-subject-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/cli/commands"
cp "$SCRIPT" "$SB/tools/scripts/check-mission-think-subject-boundary.sh"

cat > "$SB/src/cli/commands/think.rs" <<'RS'
struct ThinkArgs;

fn run(args: ThinkArgs) -> anyhow::Result<serde_json::Value> {
    let request = MissionThinkRequest::from_args(&args);
    let payload = request.to_payload();
    MissionThinkIssuer::invoke(payload)
}

struct MissionThinkRequest;

impl MissionThinkRequest {
    fn from_args(_args: &ThinkArgs) -> Self {
        Self
    }

    fn to_payload(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

struct MissionThinkIssuer;

impl MissionThinkIssuer {
    fn invoke(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let subject_ura = LocalDaemonSystemAbilityIssuer::local_daemon_identity_subject_ura()?;
        LocalDaemonSystemAbilityIssuer::invoke_root_for_subject("mission.think", args, &subject_ura)
    }
}

#[test]
fn mission_think_request_projects_cli_payload_without_judge() {}

#[test]
fn mission_think_request_projects_optional_judge() {}
RS

(
  cd "$SB"
  bash tools/scripts/check-mission-think-subject-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >> "$SB/src/cli/commands/think.rs" <<'RS'
fn legacy_think(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    invoke_local_ability("mission.think", args)
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-mission-think-subject-boundary.sh
) >/tmp/check-mission-think-subject-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "generic invoke regression should exit 1 (got $rc)"

echo "test_check_mission_think_subject_boundary.sh: all cases passed"
