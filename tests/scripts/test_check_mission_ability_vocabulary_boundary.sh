#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-mission-ability-vocabulary-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
OUT="$SB/check-mission-ability-vocabulary-boundary.out"
trap 'rm -rf "$SB"' EXIT

mkdir -p \
  "$SB/tools/scripts" \
  "$SB/src/daemon/ability/builtins/automation" \
  "$SB/src/daemon/ability/catalog" \
  "$SB/src/daemon/ability"
cp "$SCRIPT" "$SB/tools/scripts/check-mission-ability-vocabulary-boundary.sh"

write_happy_fixture() {
  cat >"$SB/src/daemon/ability/builtins/automation/mission.rs" <<'RS'
pub fn run_description() -> &'static str {
    "`mission.track` polls a long run, `mission.cancel` aborts one."
}

pub fn track_description() -> &'static str {
    "Read the persisted state of a prior `mission.run` invocation."
}
RS
  cat >"$SB/src/daemon/ability/catalog/catalog_metadata.rs" <<'RS'
// mission.discuss_round has the same Operational class as mission.run.
// mission.cancel mutates the run state of an in-flight mission.
RS
  cat >"$SB/src/daemon/ability/manifest.rs" <<'RS'
// EAL is exposed through the canonical `mission.run` ability.
RS
  cat >"$SB/src/daemon/ability/builtins/automation/orchestration.rs" <<'RS'
const HOUSE_RULES: &str = "mission.run, or direct invocation of state-mutating abilities";
RS
}

assert_fails_with() {
  local label="$1"
  local expected="$2"
  set +e
  (
    cd "$SB"
    bash tools/scripts/check-mission-ability-vocabulary-boundary.sh
  ) >"$OUT" 2>&1
  local rc=$?
  set -e
  [[ "$rc" == "1" ]] || fail "$label: expected gate failure exit 1, got $rc"
  grep -Fq "$expected" "$OUT" || fail "$label: expected failure to mention: $expected"
}

write_happy_fixture
(
  cd "$SB"
  bash tools/scripts/check-mission-ability-vocabulary-boundary.sh
) >/dev/null || fail "happy path should pass"

write_happy_fixture
printf '\n// `easynet.track` polls a long run.\n' >>"$SB/src/daemon/ability/builtins/automation/mission.rs"
assert_fails_with "mission-vocabulary-regression" "mission ability surface must use mission.run/mission.track/mission.cancel vocabulary"

write_happy_fixture
printf '\n// EAL orchestration. easynet.run / mission.run compile programs.\n' >>"$SB/src/daemon/ability/catalog/catalog_metadata.rs"
assert_fails_with "catalog-vocabulary-regression" "catalog metadata must classify mission abilities by canonical mission.* names"

write_happy_fixture
printf '\n// Let operators call them via `easynet.run` directly.\n' >>"$SB/src/daemon/ability/manifest.rs"
assert_fails_with "manifest-vocabulary-regression" "ability manifest EAL executor docs must point at mission.run"

write_happy_fixture
printf '\nconst BAD: &str = "easynet.run, easynet.invoke against state-mutating abilities";\n' >>"$SB/src/daemon/ability/builtins/automation/orchestration.rs"
assert_fails_with "orchestration-vocabulary-regression" "mission discussion prompt must not teach retired easynet.* ability aliases"

echo "test_check_mission_ability_vocabulary_boundary.sh: all cases passed"
