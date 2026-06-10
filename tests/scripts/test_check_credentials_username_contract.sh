#!/usr/bin/env bash
#
# Contract tests for scripts/check-credentials-username-contract.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/check-credentials-username-contract.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p "$sandbox/src/persistence" "$sandbox/src/facade/cli"
    cp "$REPO_ROOT/src/persistence/config.rs" "$sandbox/src/persistence/config.rs"
    cp "$REPO_ROOT/src/facade/cli/join.rs" "$sandbox/src/facade/cli/join.rs"
    cp "$REPO_ROOT/src/facade/cli/start.rs" "$sandbox/src/facade/cli/start.rs"
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
cat >>"$SB/src/facade/cli/join.rs" <<'RS'

fn backfill_credentials_username_from_auth_session() {
    let _ = crate::facade::cli::auth::load_session();
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "auth-session username backfill should exit 1 (got $rc)"

SB="$(make_sandbox)"
python3 - "$SB/src/facade/cli/join.rs" <<'PY'
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
