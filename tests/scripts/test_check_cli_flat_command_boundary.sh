#!/usr/bin/env bash
#
# Contract tests for scripts/check-cli-flat-command-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/check-cli-flat-command-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/facade" "$sandbox/scripts" "$sandbox/tests"
    cp -R "$REPO_ROOT/src/facade/cli" "$sandbox/src/facade/"
    cp -R "$REPO_ROOT/scripts" "$sandbox/"
    cp -R "$REPO_ROOT/tests" "$sandbox/"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_CLI_FLAT_COMMAND_BOUNDARY_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: layered CLI boundary should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
perl -0pi -e 's#Command::Auth\(args\) => groups::auth::dispatch\(args\),#Command::Start(args) => start::run(args),\n        Command::Auth(args) => groups::auth::dispatch(args),#' "$SB/src/facade/cli/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "top-level Command::Start arm should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >>"$SB/src/facade/cli/mod.rs" <<'RS'

const BAD_HELP_ROW: &str = "start                Start the local Axon runtime (alias of 'runtime start')";
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "help-template alias wording should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >>"$SB/src/facade/cli/presentation/banner.rs" <<'RS'

fn bad_flat_hint() -> &'static str {
    "run easynet start"
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "user-facing flat command hint should exit 1 (got $rc)"

echo "test_check_cli_flat_command_boundary.sh: all cases passed"
