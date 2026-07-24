#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_MISSION_THINK_SUBJECT_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-mission-think-subject-boundary: %s\n' "$1" >&2
  exit 1
}

TARGET="src/cli/commands/think.rs"

[[ -f "$TARGET" ]] || fail "missing $TARGET"

if ! rg -n 'struct MissionThinkRequest' "$TARGET" >/dev/null; then
  fail "mission.think CLI must own a request projection value object"
fi

if ! rg -n 'MissionThinkRequest::from_args\(&args\)' "$TARGET" >/dev/null; then
  fail "mission.think CLI must project args through MissionThinkRequest"
fi

if ! rg -n 'struct MissionThinkIssuer' "$TARGET" >/dev/null; then
  fail "mission.think CLI must use a named issuer"
fi

if ! rg -n 'LocalDaemonSystemAbilityIssuer::local_daemon_identity_subject_ura' "$TARGET" >/dev/null; then
  fail "mission.think issuer must resolve the local daemon identity subject"
fi

if ! rg -n -U 'LocalDaemonSystemAbilityIssuer::invoke_root_for_subject\(\s*"mission\.think",\s*args,\s*&subject_ura' "$TARGET" >/dev/null; then
  fail "mission.think issuer must bind explicit local daemon subject"
fi

if rg -n '\binvoke_local_ability\s*\(' "$TARGET"; then
  fail "mission.think CLI must not use generic invoke_local_ability"
fi

if ! rg -n 'mission_think_request_projects_cli_payload_without_judge' "$TARGET" >/dev/null; then
  fail "mission.think CLI must test payload projection without judge"
fi

if ! rg -n 'mission_think_request_projects_optional_judge' "$TARGET" >/dev/null; then
  fail "mission.think CLI must test payload projection with optional judge"
fi

echo "check-mission-think-subject-boundary: ok"
