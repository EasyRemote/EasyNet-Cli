#!/usr/bin/env bash
#
# Contract tests for scripts/check-auth-exec-canonical-tools.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/engineering/scripts/check-auth-exec-canonical-tools.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/cli"
    cp "$REPO_ROOT/src/cli/auth.rs" "$sandbox/src/cli/auth.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_AUTH_EXEC_CANONICAL_TOOLS_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: canonical auth exec should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
cat >>"$SB/src/cli/auth.rs" <<'RS'

// Device owner-prefixed auth exec tools are accepted as legacy aliases.
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "legacy alias language should exit 1 (got $rc)"

SB="$(make_sandbox)"
cat >>"$SB/src/cli/auth.rs" <<'RS'

fn old_auth_exec_alias(tool: &str) -> &str {
    match tool {
        "shell.run" => "device.shell.run",
        "process.exec" => "device.process.exec",
        other => other,
    }
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired alias mapping should exit 1 (got $rc)"

echo "test_check_auth_exec_canonical_tools.sh: all cases passed"
