#!/usr/bin/env bash
#
# Contract tests for scripts/check-system-ability-retired-aliases.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/engineering/scripts/check-system-ability-retired-aliases.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/ability-descriptors"
    cp -R "$REPO_ROOT/ability-descriptors/system" "$sandbox/ability-descriptors/system"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_SYSTEM_ABILITY_RETIRED_ALIASES_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: manifests should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
cat >>"$SB/ability-descriptors/system/a2a.client.send_task.ability.toml" <<'TOML'

[input_schema.properties.target_node_uri]
type = "string"
description = "Deprecated alias for target_node_ura; accepted during the URI-to-URA migration window."
TOML
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "target_node_uri alias should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '\ndescription = "Legacy alias accepted by this manifest."\n' \
    >>"$SB/ability-descriptors/system/mcp.bridge.call_tool.ability.toml"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "legacy alias language should exit 1 (got $rc)"

echo "test_check_system_ability_retired_aliases.sh: all cases passed"
