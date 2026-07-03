#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-system-ability-resource-refs.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-system-ability-resource-refs.sh"

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

copy_descriptor() {
    local sandbox="$1"
    local ability="$2"
    local source rel
    source="$(descriptor_path "$REPO_ROOT" "$ability")"
    rel="${source#$REPO_ROOT/}"
    mkdir -p "$sandbox/$(dirname "$rel")"
    cp "$source" "$sandbox/$rel"
}

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/ability-descriptors/system"
    for ability in fs.read fs.write fs.list fs.stat fs.edit fs.transfer ability.deploy; do
        copy_descriptor "$sandbox" "$ability"
    done
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_SYSTEM_ABILITY_RESOURCE_REFS_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: manifests should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
DEPLOY_TOML="$(descriptor_path "$SB" ability.deploy)"
cat >>"$DEPLOY_TOML" <<'TOML'

[input_schema.properties.path]
type = "string"
TOML
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "raw path property should exit 1 (got $rc)"

SB="$(make_sandbox)"
FS_READ_TOML="$(descriptor_path "$SB" fs.read)"
perl -0pi -e 's/required = \["resource_ref"\]/required = ["path"]/' \
    "$FS_READ_TOML"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "raw required path should exit 1 (got $rc)"

SB="$(make_sandbox)"
FS_STAT_TOML="$(descriptor_path "$SB" fs.stat)"
perl -0pi -e 's/\[input_schema\.properties\.resource_ref\]/[input_schema.properties.bundle_ref]/' \
    "$FS_STAT_TOML"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing resource_ref schema should exit 1 (got $rc)"

echo "test_check_system_ability_resource_refs.sh: all cases passed"
