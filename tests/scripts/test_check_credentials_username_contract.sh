#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-credentials-username-contract.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-credentials-username-contract.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/daemon/persistence" "$sandbox/src/cli/commands"
    cp "$REPO_ROOT/src/daemon/persistence/config.rs" "$sandbox/src/daemon/persistence/config.rs"
    cp "$REPO_ROOT/src/cli/commands/join.rs" "$sandbox/src/cli/commands/join.rs"
    cp "$REPO_ROOT/src/cli/commands/start.rs" "$sandbox/src/cli/commands/start.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_CREDENTIALS_USERNAME_CONTRACT_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: username contract should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
cat >>"$SB/src/cli/commands/join.rs" <<'RS'

fn backfill_credentials_username_from_auth_session() {
    let _ = crate::cli::commands::auth::load_session();
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "auth-session username backfill should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/cli/commands/join.rs" <<'PY'
import pathlib
import sys
path = pathlib.Path(sys.argv[1])
path.write_text(path.read_text().replace("pairing response missing username", "pairing response accepted username fallback"))
PY
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing pairing username validation should exit 1 (got $rc)"

echo "test_check_credentials_username_contract.sh: all cases passed"
