#!/usr/bin/env bash
#
# Contract tests for scripts/check-system-ability-resource-refs.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/engineering/scripts/check-system-ability-resource-refs.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/ability-descriptors/system"
    cp "$REPO_ROOT"/ability-descriptors/system/fs.{read,write,list,stat,edit,transfer}.ability.toml \
        "$sandbox/ability-descriptors/system/"
    cp "$REPO_ROOT/ability-descriptors/system/ability.deploy.ability.toml" \
        "$sandbox/ability-descriptors/system/"
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
cat >>"$SB/ability-descriptors/system/ability.deploy.ability.toml" <<'TOML'

[input_schema.properties.path]
type = "string"
TOML
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "raw path property should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/required = \["resource_ref"\]/required = ["path"]/' \
    "$SB/ability-descriptors/system/fs.read.ability.toml"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "raw required path should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/\[input_schema\.properties\.resource_ref\]/[input_schema.properties.bundle_ref]/' \
    "$SB/ability-descriptors/system/fs.stat.ability.toml"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing resource_ref schema should exit 1 (got $rc)"

echo "test_check_system_ability_resource_refs.sh: all cases passed"
