#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_MISSION_DISCUSS_SUBJECT_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-mission-discuss-subject-boundary: %s\n' "$1" >&2
  exit 1
}

TARGET="src/cli/commands/discuss.rs"

[[ -f "$TARGET" ]] || fail "missing $TARGET"

for object in \
  "struct DiscussCreateRequest" \
  "struct DiscussHumanTurnRequest" \
  "struct DiscussRoundRequest" \
  "struct DiscussListTurnsRequest"; do
  if ! rg -n "$object" "$TARGET" >/dev/null; then
    fail "mission discuss CLI must own request projection value object: $object"
  fi
done

if ! rg -n 'fn parse_discuss_roles' "$TARGET" >/dev/null; then
  fail "mission discuss CLI must parse role projection outside the run flow"
fi

if ! rg -n 'struct MissionDiscussIssuer' "$TARGET" >/dev/null; then
  fail "mission discuss CLI must use a named subject-bound issuer"
fi

if ! rg -n 'LocalDaemonSystemAbilityIssuer::local_daemon_identity_subject_ura' "$TARGET" >/dev/null; then
  fail "mission discuss issuer must resolve the local daemon identity subject"
fi

if ! rg -n -U 'LocalDaemonSystemAbilityIssuer::invoke_root_for_subject\(\s*ability,\s*args,\s*&subject_ura' "$TARGET" >/dev/null; then
  fail "mission discuss issuer must bind explicit local daemon subject"
fi

if rg -n '\binvoke_local_ability\s*\(' "$TARGET"; then
  fail "mission discuss CLI must not use generic invoke_local_ability"
fi

for call in \
  'MissionDiscussIssuer::invoke\(\s*"discuss\.create"' \
  'MissionDiscussIssuer::invoke\(\s*"discuss\.post"' \
  'MissionDiscussIssuer::invoke\(\s*"mission\.discuss_round"' \
  'MissionDiscussIssuer::invoke\(\s*"discuss\.list_turns"'; do
  if ! rg -n -U "$call" "$TARGET" >/dev/null; then
    fail "mission discuss CLI must route through MissionDiscussIssuer for $call"
  fi
done

for test_name in \
  "discuss_create_request_projects_payload" \
  "discuss_human_turn_request_projects_payload" \
  "discuss_round_request_projects_roles_when_present" \
  "discuss_list_turns_request_projects_payload" \
  "parse_discuss_roles_rejects_missing_separator_or_empty_values"; do
  if ! rg -n "$test_name" "$TARGET" >/dev/null; then
    fail "mission discuss request boundary must be covered by test: $test_name"
  fi
done

echo "check-mission-discuss-subject-boundary: ok"
