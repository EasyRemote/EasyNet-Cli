#!/usr/bin/env bash
#
# Contract tests for engineering/scripts/check-project-structure-v1.sh.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
SCRIPT="$REPO_ROOT/engineering/scripts/check-project-structure-v1.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }

make_sandbox() {
    local sandbox
    sandbox="$(mktemp -d)"
    mkdir -p \
        "$sandbox/src/runtime/system_abilities" \
        "$sandbox/src/runtime/system_ability_catalog" \
        "$sandbox/src/runtime/ability_names" \
        "$sandbox/src/runtime/executors" \
        "$sandbox/src/cli" \
        "$sandbox/src/daemon/control" \
        "$sandbox/src/daemon/federation/client" \
        "$sandbox/src/daemon/invocation" \
        "$sandbox/src/services" \
        "$sandbox/ability-descriptors/system" \
        "$sandbox/schemas" \
        "$sandbox/engineering/benches" \
        "$sandbox/engineering/docker" \
        "$sandbox/engineering/scripts" \
        "$sandbox/engineering/tests/scripts" \
        "$sandbox/platforms/macos" \
        "$sandbox/platforms/windows" \
        "$sandbox/src" \
        "$sandbox/benches" \
        "$sandbox/tests/scripts" \
        "$sandbox/scripts"
    printf '%s\n' '// cli root' > "$sandbox/src/cli/mod.rs"
    printf '%s\n' '// daemon root' > "$sandbox/src/daemon/mod.rs"
    printf '%s\n' '// daemon control root' > "$sandbox/src/daemon/control/mod.rs"
    printf '%s\n' '// daemon federation root' > "$sandbox/src/daemon/federation/mod.rs"
    printf '%s\n' '// daemon federation client root' > "$sandbox/src/daemon/federation/client/mod.rs"
    printf '%s\n' '// daemon federation directory' > "$sandbox/src/daemon/federation/directory.rs"
    printf '%s\n' '// daemon federation directory reader' > "$sandbox/src/daemon/federation/directory_reader.rs"
    printf '%s\n' '// daemon federation peers' > "$sandbox/src/daemon/federation/peers.rs"
    printf '%s\n' '// daemon invocation root' > "$sandbox/src/daemon/invocation/mod.rs"
    printf '%s\n' '// agent ability specs' > "$sandbox/src/runtime/agent_ability_specs.rs"
    printf '%s\n' '[package]' 'name = "fixture"' > "$sandbox/Cargo.toml"
    printf '%s\n' '// build fixture' > "$sandbox/build.rs"
    printf '%s\n' '// lib fixture' > "$sandbox/src/lib.rs"
    printf '%s\n' '// bench body' > "$sandbox/engineering/benches/example.rs"
    printf '%s\n' 'include!("../engineering/benches/example.rs");' > "$sandbox/benches/example.rs"
    printf '%s\n' '#!/usr/bin/env bash' 'echo ok' > "$sandbox/engineering/scripts/example.sh"
    printf '%s\n' '#!/usr/bin/env bash' 'exec "$PWD/engineering/scripts/example.sh" "$@"' > "$sandbox/scripts/example.sh"
    printf '%s\n' 'print("ok")' > "$sandbox/engineering/scripts/example.py"
    printf '%s\n' '#!/usr/bin/env python3' 'import runpy' 'target = "engineering/scripts/example.py"' 'runpy.run_path(target, run_name="__main__")' > "$sandbox/scripts/example.py"
    printf '%s\n' 'Write-Output ok' > "$sandbox/engineering/scripts/example.ps1"
    printf '%s\n' '$Target = "engineering/scripts/example.ps1"' '& $Target @args' > "$sandbox/scripts/example.ps1"
    printf '%s\n' '#!/usr/bin/env bash' 'echo ok' > "$sandbox/engineering/tests/scripts/example.sh"
    printf '%s\n' '#!/usr/bin/env bash' 'exec bash "$PWD/engineering/tests/scripts/example.sh" "$@"' > "$sandbox/tests/scripts/example.sh"
    echo "$sandbox"
}

run_check() {
    local sandbox="$1"
    ( cd "$sandbox" && CHECK_PROJECT_STRUCTURE_V1_ROOT="$sandbox" bash "$SCRIPT" )
}

SB="$(make_sandbox)"
run_check "$SB" >/dev/null 2>&1 || { rm -rf "$SB"; fail "happy: project structure guard should pass"; }
rm -rf "$SB"

SB="$(make_sandbox)"
mkdir -p "$SB/src/runtime/agents"
printf '%s\n' '// handler leak' > "$SB/src/runtime/agents/ping.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/agents directory should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/abilities/system"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired descriptor root should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::agents::ping::x; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::agents import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::abilities::abilities_for; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::abilities import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::facade::cli::run; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "facade::cli import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::facade::mcp::x; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "facade::mcp import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::control::server::x; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::control import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::federation_client::FederationClient; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::federation_client import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::federation_directory::DirectoryEntry; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::federation_directory import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::federated_peers_cell::SharedFederatedPeers; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::federated_peers_cell import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::federated_directory_reader::read_federated_directory; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::federated_directory_reader import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::invocation_transport::DaemonInvocationService; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::invocation_transport import should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/facade/cli"
printf '%s\n' '// retired cli path' > "$SB/src/facade/cli/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/facade should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// retired ability specs facade' > "$SB/src/runtime/abilities.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/runtime/abilities.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/facade/mcp"
printf '%s\n' '// retired mcp path' > "$SB/src/facade/mcp/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/facade/mcp should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -rf "$SB/src/cli"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/cli should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -rf "$SB/src/daemon"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// retired daemon root' > "$SB/src/daemon.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/daemon.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -rf "$SB/src/daemon/control"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/control should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -rf "$SB/src/daemon/federation/client"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/federation/client should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/federation/directory.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/federation/directory.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/federation/directory_reader.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/federation/directory_reader.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/federation/peers.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/federation/peers.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -rf "$SB/src/daemon/invocation"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/invocation should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services/control"
printf '%s\n' '// retired control path' > "$SB/src/services/control/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/control should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services/federation_client"
printf '%s\n' '// retired federation client path' > "$SB/src/services/federation_client/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/federation_client should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// retired federation directory path' > "$SB/src/services/federation_directory.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/federation_directory.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// retired federated peers path' > "$SB/src/services/federated_peers_cell.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/federated_peers_cell.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// retired federated directory reader path' > "$SB/src/services/federated_directory_reader.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/federated_directory_reader.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services/invocation_transport"
printf '%s\n' '// retired invocation transport path' > "$SB/src/services/invocation_transport/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/invocation_transport should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/daemon.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/daemon.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/runtime/abilities.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/runtime/abilities.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/facade/mcp' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/facade/mcp reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/control' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/control reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/federation_client' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/federation_client reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/federation_directory.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/federation_directory.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/federated_peers_cell.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/federated_peers_cell.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/federated_directory_reader.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/federated_directory_reader.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/invocation_transport' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/invocation_transport reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/docker"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "root docker directory should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -rf "$SB/schemas"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing root schemas directory should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/engineering/schemas"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "engineering/schemas should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -rf "$SB/platforms/windows"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing platforms/windows should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -rf "$SB/engineering/tests/scripts"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing engineering/tests/scripts should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '#!/usr/bin/env bash' 'echo not a wrapper' > "$SB/scripts/example.sh"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "non-wrapper root script should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '#!/usr/bin/env bash' 'echo not a wrapper' > "$SB/tests/scripts/example.sh"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "non-wrapper root test script should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/engineering/tests/scripts/example.sh"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "root test wrapper without engineering target should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn bench_body() {}' > "$SB/benches/example.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "non-wrapper root bench should exit 1 (got $rc)"

echo "test_check_project_structure_v1.sh: all cases passed"
