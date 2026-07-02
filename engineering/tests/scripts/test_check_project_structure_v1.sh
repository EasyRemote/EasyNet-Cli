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
        "$sandbox/src/daemon/ability/builtins" \
        "$sandbox/src/daemon/ability/catalog" \
        "$sandbox/src/daemon/ability/authority" \
        "$sandbox/src/daemon/ability/descriptors" \
        "$sandbox/src/daemon/ability/impl_bindings" \
        "$sandbox/src/runtime/executors" \
        "$sandbox/src/cli" \
        "$sandbox/src/daemon/ability" \
        "$sandbox/src/daemon/ability/names" \
        "$sandbox/src/daemon/ability/wire" \
        "$sandbox/src/daemon/axon_bridge" \
        "$sandbox/src/daemon/context" \
        "$sandbox/src/daemon/control" \
        "$sandbox/src/daemon/execution" \
        "$sandbox/src/daemon/federation/client" \
        "$sandbox/src/daemon/federation/read_model" \
        "$sandbox/src/daemon/identity" \
        "$sandbox/src/daemon/invocation" \
        "$sandbox/src/daemon/invocation/state" \
        "$sandbox/src/daemon/keyring" \
        "$sandbox/src/daemon/plugins" \
        "$sandbox/src/daemon/resources" \
        "$sandbox/src/daemon/trust" \
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
    printf '%s\n' '// daemon ability root' > "$sandbox/src/daemon/ability/mod.rs"
    printf '%s\n' '// daemon ability authority root' > "$sandbox/src/daemon/ability/authority/mod.rs"
    printf '%s\n' '// daemon ability builtins root' > "$sandbox/src/daemon/ability/builtins/mod.rs"
    printf '%s\n' '// daemon ability catalog root' > "$sandbox/src/daemon/ability/catalog/mod.rs"
    printf '%s\n' '// daemon ability conformance' > "$sandbox/src/daemon/ability/conformance.rs"
    printf '%s\n' '// daemon ability control plane' > "$sandbox/src/daemon/ability/control_plane.rs"
    printf '%s\n' '// daemon ability control plane error' > "$sandbox/src/daemon/ability/control_plane_error.rs"
    printf '%s\n' '// daemon ability descriptors root' > "$sandbox/src/daemon/ability/descriptors/mod.rs"
    printf '%s\n' '// daemon ability descriptor surface' > "$sandbox/src/daemon/ability/descriptors/surface.rs"
    printf '%s\n' '// daemon ability dispatch' > "$sandbox/src/daemon/ability/dispatch.rs"
    printf '%s\n' '// daemon ability health' > "$sandbox/src/daemon/ability/health.rs"
    printf '%s\n' '// daemon ability impl bindings root' > "$sandbox/src/daemon/ability/impl_bindings/mod.rs"
    printf '%s\n' '// daemon ability names root' > "$sandbox/src/daemon/ability/names/mod.rs"
    printf '%s\n' '// daemon ability wire root' > "$sandbox/src/daemon/ability/wire/mod.rs"
    printf '%s\n' '// daemon axon bridge root' > "$sandbox/src/daemon/axon_bridge/mod.rs"
    printf '%s\n' '// daemon context root' > "$sandbox/src/daemon/context/mod.rs"
    printf '%s\n' '// daemon clipboard tracker' > "$sandbox/src/daemon/context/clipboard_tracker.rs"
    printf '%s\n' '// daemon control root' > "$sandbox/src/daemon/control/mod.rs"
    printf '%s\n' '// daemon execution root' > "$sandbox/src/daemon/execution/mod.rs"
    printf '%s\n' '// daemon federation root' > "$sandbox/src/daemon/federation/mod.rs"
    printf '%s\n' '// daemon federation client root' > "$sandbox/src/daemon/federation/client/mod.rs"
    printf '%s\n' '// daemon federation directory' > "$sandbox/src/daemon/federation/directory.rs"
    printf '%s\n' '// daemon federation directory reader' > "$sandbox/src/daemon/federation/directory_reader.rs"
    printf '%s\n' '// daemon federation peers' > "$sandbox/src/daemon/federation/peers.rs"
    printf '%s\n' '// daemon federation read model root' > "$sandbox/src/daemon/federation/read_model/mod.rs"
    printf '%s\n' '// ability catalog read model' > "$sandbox/src/daemon/federation/read_model/ability_catalog.rs"
    printf '%s\n' '// advertised agents read model' > "$sandbox/src/daemon/federation/read_model/advertised_agents.rs"
    printf '%s\n' '// hub published abilities read model' > "$sandbox/src/daemon/federation/read_model/hub_published_abilities.rs"
    printf '%s\n' '// daemon identity root' > "$sandbox/src/daemon/identity/mod.rs"
    printf '%s\n' '// daemon local invocation identity' > "$sandbox/src/daemon/identity/local_invocation.rs"
    printf '%s\n' '// daemon self identity' > "$sandbox/src/daemon/identity/self_identity.rs"
    printf '%s\n' '// daemon invocation root' > "$sandbox/src/daemon/invocation/mod.rs"
    printf '%s\n' '// daemon local runtime invoker' > "$sandbox/src/daemon/invocation/local_runtime_invoker.rs"
    printf '%s\n' '// daemon invocation state root' > "$sandbox/src/daemon/invocation/state/mod.rs"
    printf '%s\n' '// nonce replay' > "$sandbox/src/daemon/invocation/state/nonce_replay.rs"
    printf '%s\n' '// pending dispatch' > "$sandbox/src/daemon/invocation/state/pending_dispatch.rs"
    printf '%s\n' '// presence' > "$sandbox/src/daemon/invocation/state/presence.rs"
    printf '%s\n' '// session failure' > "$sandbox/src/daemon/invocation/state/session_failure.rs"
    printf '%s\n' '// usage quota' > "$sandbox/src/daemon/invocation/state/usage_quota.rs"
    printf '%s\n' '// daemon keyring root' > "$sandbox/src/daemon/keyring/mod.rs"
    printf '%s\n' '// daemon plugins root' > "$sandbox/src/daemon/plugins/mod.rs"
    printf '%s\n' '// daemon resources root' > "$sandbox/src/daemon/resources/mod.rs"
    printf '%s\n' '// daemon trust root' > "$sandbox/src/daemon/trust/mod.rs"
    printf '%s\n' '// daemon trust anchor' > "$sandbox/src/daemon/trust/anchor.rs"
    printf '%s\n' '// daemon trust cell' > "$sandbox/src/daemon/trust/cell.rs"
    printf '%s\n' '// daemon trust key resolver' > "$sandbox/src/daemon/trust/key_resolver.rs"
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
mkdir -p "$SB/src/services"
printf '%s\n' '// retired services root' > "$SB/src/services/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services root should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/runtime/agents"
printf '%s\n' '// handler leak' > "$SB/src/runtime/agents/ping.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/agents directory should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/runtime/ability"
printf '%s\n' '// retired ability control plane root' > "$SB/src/runtime/ability/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/ability directory should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// retired ability descriptor facade' > "$SB/src/runtime/ability_descriptor.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/ability_descriptor.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// retired ability dispatch facade' > "$SB/src/runtime/ability_dispatch.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/ability_dispatch.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/runtime/ability_names"
printf '%s\n' '// retired ability names root' > "$SB/src/runtime/ability_names/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/ability_names directory should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// retired ability wire module' > "$SB/src/runtime/ability_wire.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/ability_wire.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/runtime/axon_bridge"
printf '%s\n' '// retired axon bridge root' > "$SB/src/runtime/axon_bridge/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/axon_bridge directory should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/runtime/execution"
printf '%s\n' '// retired execution root' > "$SB/src/runtime/execution/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/execution directory should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// retired local invocation identity' > "$SB/src/runtime/local_invocation_identity.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/local_invocation_identity.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// retired local runtime invoker' > "$SB/src/runtime/local_runtime_invoker.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/local_runtime_invoker.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/runtime/plugin_host"
printf '%s\n' '// retired plugin host root' > "$SB/src/runtime/plugin_host/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/plugin_host directory should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/runtime/resources"
printf '%s\n' '// retired resources root' > "$SB/src/runtime/resources/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/resources directory should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/runtime/system_abilities"
printf '%s\n' '// retired system abilities root' > "$SB/src/runtime/system_abilities/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/system_abilities directory should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/runtime/system_ability_catalog"
printf '%s\n' '// retired system ability catalog root' > "$SB/src/runtime/system_ability_catalog/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired runtime/system_ability_catalog directory should exit 1 (got $rc)"

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
printf '%s\n' 'fn f() { let _ = crate::runtime::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::ability import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::ability_descriptor::AbilityDescriptor::new; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::ability_descriptor import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::ability_dispatch::AxonAbilityCatalog::new; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::ability_dispatch import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::ability_names::agents::CHAT; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::ability_names import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::ability_wire::AbilityWireRegistry::core; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::ability_wire import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::axon_bridge::runtime_factory::build_local_runtime; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::axon_bridge import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::execution::schedule::ScheduleService::new; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::execution import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::local_invocation_identity import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::local_runtime_invoker::invoke_local_rpc_sync; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::local_runtime_invoker import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::plugin_host::PluginRuntimeManager::new; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::plugin_host import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::resources::filesystem::resource_ref_schema; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::resources import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::system_abilities::agents::chat::register; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::system_abilities import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::runtime::system_ability_catalog::build_registry; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "runtime::system_ability_catalog import should exit 1 (got $rc)"

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
printf '%s\n' 'fn f() { let _ = crate::services::realm_trust_anchor::RealmTrustAnchor; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::realm_trust_anchor import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::trust_anchor_cell::SharedTrustAnchor; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::trust_anchor_cell import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::trust_anchor_key_resolver::RealmTrustAnchorKeyResolver; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::trust_anchor_key_resolver import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::keyring::Vault; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::keyring import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::self_identity::SelfIdentityError; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::self_identity import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::presence_registry::PresenceRegistry; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::presence_registry import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::pending_dispatch::PendingDispatchMap; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::pending_dispatch import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::nonce_replay_store::SharedNonceReplayStore; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::nonce_replay_store import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::usage_quota_store::SharedUsageQuotaGate; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::usage_quota_store import should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' 'fn f() { let _ = crate::services::session_failure::SessionFailure; }' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "services::session_failure import should exit 1 (got $rc)"

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
rm -f "$SB/src/daemon/invocation/local_runtime_invoker.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/invocation/local_runtime_invoker.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/identity/self_identity.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/identity/self_identity.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/keyring/mod.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/keyring/mod.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/invocation/state/nonce_replay.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/invocation/state/nonce_replay.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/invocation/state/pending_dispatch.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/invocation/state/pending_dispatch.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/invocation/state/presence.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/invocation/state/presence.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/invocation/state/session_failure.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/invocation/state/session_failure.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/invocation/state/usage_quota.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/invocation/state/usage_quota.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/trust/anchor.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/trust/anchor.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/trust/cell.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/trust/cell.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
rm -f "$SB/src/daemon/trust/key_resolver.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "missing src/daemon/trust/key_resolver.rs should exit 1 (got $rc)"

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
mkdir -p "$SB/src/services"
printf '%s\n' '// retired federation directory path' > "$SB/src/services/federation_directory.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/federation_directory.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services"
printf '%s\n' '// retired federated peers path' > "$SB/src/services/federated_peers_cell.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/federated_peers_cell.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services"
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
mkdir -p "$SB/src/services"
printf '%s\n' '// retired realm trust anchor path' > "$SB/src/services/realm_trust_anchor.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/realm_trust_anchor.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services"
printf '%s\n' '// retired trust anchor cell path' > "$SB/src/services/trust_anchor_cell.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/trust_anchor_cell.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services"
printf '%s\n' '// retired trust anchor key resolver path' > "$SB/src/services/trust_anchor_key_resolver.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/trust_anchor_key_resolver.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services"
printf '%s\n' '// retired keyring path' > "$SB/src/services/keyring.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/keyring.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services"
printf '%s\n' '// retired self identity path' > "$SB/src/services/self_identity.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/self_identity.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services"
printf '%s\n' '// retired presence registry path' > "$SB/src/services/presence_registry.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/presence_registry.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services"
printf '%s\n' '// retired pending dispatch path' > "$SB/src/services/pending_dispatch.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/pending_dispatch.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services"
printf '%s\n' '// retired nonce replay store path' > "$SB/src/services/nonce_replay_store.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/nonce_replay_store.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services"
printf '%s\n' '// retired usage quota store path' > "$SB/src/services/usage_quota_store.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/usage_quota_store.rs should exit 1 (got $rc)"

SB="$(make_sandbox)"
mkdir -p "$SB/src/services"
printf '%s\n' '// retired session failure path' > "$SB/src/services/session_failure.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "retired src/services/session_failure.rs should exit 1 (got $rc)"

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
printf '%s\n' '// doc mentions src/services/realm_trust_anchor.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/realm_trust_anchor.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/trust_anchor_cell.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/trust_anchor_cell.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/trust_anchor_key_resolver.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/trust_anchor_key_resolver.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/keyring.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/keyring.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/self_identity.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/self_identity.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/presence_registry.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/presence_registry.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/pending_dispatch.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/pending_dispatch.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/nonce_replay_store.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/nonce_replay_store.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/usage_quota_store.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/usage_quota_store.rs reference should exit 1 (got $rc)"

SB="$(make_sandbox)"
printf '%s\n' '// doc mentions src/services/session_failure.rs' > "$SB/src/lib.rs"
rc=0
run_check "$SB" >/dev/null 2>&1 || rc=$?
rm -rf "$SB"
[[ "$rc" == "1" ]] || fail "src/services/session_failure.rs reference should exit 1 (got $rc)"

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
