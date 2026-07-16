#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-system-ability-retired-aliases.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-system-ability-retired-aliases.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

descriptor_path() {
    local root="$1"
    local ability="$2"
    local paths count
    paths="$(find "$root/ability-descriptors/system" -type f -name "${ability}.ability.toml" -print | sort)"
    count="$(printf '%s\n' "$paths" | sed '/^$/d' | wc -l | tr -d ' ')"
    [[ "$count" == "1" ]] || fail "expected exactly one descriptor for $ability, found $count"
    printf '%s\n' "$paths"
}

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
A2A_TOML="$(descriptor_path "$SB" a2a.client.send_task)"
cat >>"$A2A_TOML" <<'TOML'

[input_schema.properties.target_node_uri]
type = "string"
description = "Deprecated non-URA alias for target_node_ura accepted during a retired migration window."
TOML
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "target_node_uri alias should exit 1 (got $rc)"

SB="$(make_sandbox)"
MCP_TOML="$(descriptor_path "$SB" mcp.bridge.call_tool)"
printf '\ndescription = "Legacy alias accepted by this manifest."\n' \
    >>"$MCP_TOML"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "legacy alias language should exit 1 (got $rc)"

echo "test_check_system_ability_retired_aliases.sh: all cases passed"
