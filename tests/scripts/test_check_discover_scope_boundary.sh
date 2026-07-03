#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-discover-scope-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-discover-scope-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/daemon/ability/builtins/agents"
    cp "$REPO_ROOT/src/daemon/ability/builtins/agents/discover.rs" "$sandbox/src/daemon/ability/builtins/agents/discover.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_DISCOVER_SCOPE_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: discover scope boundary should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's/&\["self", "device", "user", "public"\]/&["self", "device", "user", "public", "easynet"]/' "$SB/src/daemon/ability/builtins/agents/discover.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "scope enum with easynet alias should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's/"user" => Ok\(Scope::User\),/"easynet" | "user" => Ok(Scope::User),/' "$SB/src/daemon/ability/builtins/agents/discover.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "parser accepting easynet alias should exit 1 (got $rc)"

SB="$(make_sandbox)"
echo '// `easynet` is retained as a back-compat alias for `user`.' >> "$SB/src/daemon/ability/builtins/agents/discover.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "schema description advertising alias should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -ni -e 'print unless /parse_scope\(&json!\(\{"scope": "easynet"\}\)\)\.is_err\(\)/' "$SB/src/daemon/ability/builtins/agents/discover.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing rejection test should exit 1 (got $rc)"

echo "test_check_discover_scope_boundary.sh: all cases passed"
