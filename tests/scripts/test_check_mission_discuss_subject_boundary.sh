#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-mission-discuss-subject-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" "$SB/src/cli/commands"
cp "$SCRIPT" "$SB/tools/scripts/check-mission-discuss-subject-boundary.sh"

cat > "$SB/src/cli/commands/discuss.rs" <<'RS'
struct DiscussCreateRequest;
struct DiscussHumanTurnRequest;
struct DiscussRoundRequest;
struct DiscussListTurnsRequest;

fn parse_discuss_roles(_entries: &[String]) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    Ok(serde_json::Map::new())
}

struct MissionDiscussIssuer;

impl MissionDiscussIssuer {
    fn invoke(ability: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let subject_ura = LocalDaemonSystemAbilityIssuer::local_daemon_identity_subject_ura()?;
        LocalDaemonSystemAbilityIssuer::invoke_root_for_subject(ability, args, &subject_ura)
    }
}

fn run() -> anyhow::Result<()> {
    MissionDiscussIssuer::invoke("discuss.create", serde_json::json!({}))?;
    MissionDiscussIssuer::invoke("discuss.post", serde_json::json!({}))?;
    MissionDiscussIssuer::invoke("mission.discuss_round", serde_json::json!({}))?;
    MissionDiscussIssuer::invoke("discuss.list_turns", serde_json::json!({}))?;
    Ok(())
}

#[test]
fn discuss_create_request_projects_payload() {}

#[test]
fn discuss_human_turn_request_projects_payload() {}

#[test]
fn discuss_round_request_projects_roles_when_present() {}

#[test]
fn discuss_list_turns_request_projects_payload() {}

#[test]
fn parse_discuss_roles_rejects_missing_separator_or_empty_values() {}
RS

(
  cd "$SB"
  bash tools/scripts/check-mission-discuss-subject-boundary.sh
) >/dev/null || fail "happy path should pass"

cat >> "$SB/src/cli/commands/discuss.rs" <<'RS'
fn legacy_discuss(args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    invoke_local_ability("mission.discuss_round", args)
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-mission-discuss-subject-boundary.sh
) >/tmp/check-mission-discuss-subject-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "generic invoke regression should exit 1 (got $rc)"

echo "test_check_mission_discuss_subject_boundary.sh: all cases passed"
