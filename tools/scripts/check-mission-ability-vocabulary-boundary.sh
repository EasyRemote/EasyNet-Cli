#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_MISSION_ABILITY_VOCABULARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-mission-ability-vocabulary-boundary: %s\n' "$1" >&2
  exit 1
}

MISSION="src/daemon/ability/builtins/automation/mission.rs"
CATALOG="src/daemon/ability/catalog/catalog_metadata.rs"
MANIFEST="src/daemon/ability/manifest.rs"
ORCHESTRATION="src/daemon/ability/builtins/automation/orchestration.rs"

for path in "$MISSION" "$CATALOG" "$MANIFEST" "$ORCHESTRATION"; do
  [[ -f "$path" ]] || fail "missing $path"
done

if rg -n 'easynet\.(run|track|cancel|invoke)' "$MISSION"; then
  fail "mission ability surface must use mission.run/mission.track/mission.cancel vocabulary"
fi

if rg -n 'fall back|fallback|legacy-carrier|thin shim|\bshim\b|Implicit-agent-fallback|compat' "$MISSION"; then
  fail "mission ability ingress must not describe canonical mission abilities as legacy, compat, shim, or fallback paths"
fi

if rg -n 'easynet\.run /|easynet\.cancel mutates|Same Operational class as easynet\.run' "$CATALOG"; then
  fail "catalog metadata must classify mission abilities by canonical mission.* names"
fi

if rg -n 'uses with `easynet\.run`|via `easynet\.run`|them via `easynet\.run`' "$MANIFEST"; then
  fail "ability manifest EAL executor docs must point at mission.run"
fi

if rg -n 'easynet\.run, easynet\.invoke|easynet\.run|easynet\.invoke' "$ORCHESTRATION"; then
  fail "mission discussion prompt must not teach retired easynet.* ability aliases"
fi

for required in \
  'mission.track` polls a long run' \
  'mission.cancel` aborts one' \
  'mission.run` invocation' \
  'mission.run` ability'; do
  if ! rg -n "$required" "$MISSION" "$MANIFEST" >/dev/null; then
    fail "missing canonical mission vocabulary: $required"
  fi
done

echo "check-mission-ability-vocabulary-boundary: ok"
