#!/usr/bin/env bash
#
# Contract tests for tools/scripts/check-canonical-hub-ura-boundary.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-canonical-hub-ura-boundary.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

copy_if_present() {
    local src="$1"
    local dst="$2"
    if [[ -f "$src" ]]; then
        mkdir -p "$(dirname "$dst")"
        cp "$src" "$dst"
    fi
}

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p \
        "$sandbox/src/core/ura" \
        "$sandbox/src/daemon/invocation/admission" \
        "$sandbox/src/daemon/invocation/dispatch" \
        "$sandbox/src/daemon/invocation/routing" \
        "$sandbox/tests" \
        "$sandbox/docs" \
        "$sandbox/tools/scripts" \
        "$sandbox/axon"
    cp "$REPO_ROOT/src/core/ura/mod.rs" "$sandbox/src/core/ura/mod.rs"
    cp "$REPO_ROOT/src/daemon/invocation/routing/remote_invoke.rs" "$sandbox/src/daemon/invocation/routing/remote_invoke.rs"
    cp "$REPO_ROOT/src/daemon/invocation/admission/admission_facade.rs" "$sandbox/src/daemon/invocation/admission/admission_facade.rs"
    cp "$REPO_ROOT/src/daemon/invocation/dispatch/daemon_invocation_service.rs" "$sandbox/src/daemon/invocation/dispatch/daemon_invocation_service.rs"
    cp "$REPO_ROOT/src/daemon/invocation/dispatch/daemon_invocation_service_tests.rs" "$sandbox/src/daemon/invocation/dispatch/daemon_invocation_service_tests.rs"
    cp "$REPO_ROOT/src/daemon/invocation/admission/register_device_pubkey.rs" "$sandbox/src/daemon/invocation/admission/register_device_pubkey.rs"
    copy_if_present "$REPO_ROOT/docs/spec/owner-truth-table/ability-owner-truth-table.tex" "$sandbox/docs/spec/owner-truth-table/ability-owner-truth-table.tex"
    cp "$REPO_ROOT/../EasyNet-Axon/core/ura-rs/src/lib.rs" "$sandbox/axon/lib.rs"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    (
        cd "$sandbox"
        CHECK_CANONICAL_HUB_URA_ROOT="$sandbox" \
        CHECK_CANONICAL_HUB_URA_AXON_URA_RS="$sandbox/axon/lib.rs" \
            bash "$SCRIPT"
    )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: canonical Hub URA boundary should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
cat >>"$SB/axon/lib.rs" <<'RS'

fn bad_hub_with_tail_generation(realm: &str) -> Ura {
    Ura(format!("{URA_SCHEME}{realm}/hub/{realm}"))
}
RS
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "Axon Hub generation should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's#whose canonical protocol identity is `easynet:///r/<realm>/authority`#whose canonical protocol identity is `easynet:///r/<realm>/hub`#' "$SB/src/daemon/invocation/routing/remote_invoke.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "stale parse_node_ura /hub docs should exit 1 (got $rc)"

SB="$(make_sandbox)"
perl -0pi -e 's#assert!\(!facade\.is_federated_caller\("easynet:///r/peer-realm/authority/extra"\)\);#assert!(facade.is_federated_caller("easynet:///r/peer-realm/authority/extra"));#' "$SB/src/daemon/invocation/admission/admission_facade.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "admission authority-with-tail acceptance should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/docs/stale"
cat >"$SB/docs/stale/00-intent.md" <<'MD'
The immediate target is the malformed hub identity shape `easynet:///r/<realm>/hub`.
MD
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "stale docs wording should exit 1 (got $rc)"

echo "test_check_canonical_hub_ura_boundary.sh: all cases passed"
