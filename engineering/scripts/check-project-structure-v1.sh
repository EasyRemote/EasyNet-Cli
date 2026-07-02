#!/usr/bin/env bash
#
# Guard the Project Structure v1 migration boundary.

set -euo pipefail

ROOT="${CHECK_PROJECT_STRUCTURE_V1_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
    echo "check-project-structure-v1: $*" >&2
    exit 1
}

require_rg() {
    command -v rg >/dev/null 2>&1 || fail "ripgrep is required"
}

require_path() {
    local path="$1"
    [[ -e "$path" ]] || fail "missing required path: $path"
}

reject_path() {
    local path="$1"
    [[ ! -e "$path" ]] || fail "retired path still exists: $path"
}

scan_must_be_empty() {
    local label="$1"
    local pattern="$2"
    shift 2

    local hits
    hits="$(rg -n "$pattern" "$@" \
        --glob '!scripts/check-project-structure-v1.sh' \
        --glob '!engineering/scripts/check-project-structure-v1.sh' \
        --glob '!engineering/scripts/check-rfc-001-conformance-self-test.sh' \
        --glob '!engineering/tests/scripts/test_check_project_structure_v1.sh' \
        --glob '!tests/scripts/test_check_project_structure_v1.sh' \
        2>/dev/null || true)"
    [[ -z "$hits" ]] || fail "$label:
$hits"
}

require_bash_wrapper() {
    local wrapper="$1"
    local target="$2"
    local label="$3"

    [[ -f "$target" ]] || fail "$label wrapper has no engineering target: $wrapper -> $target"
    grep -qF "exec " "$wrapper" \
        && grep -qF "$target" "$wrapper" \
        || fail "$label wrapper must exec its engineering target: $wrapper"
}

require_tool_wrapper() {
    local wrapper="$1"
    local target="$2"
    local label="$3"

    [[ -f "$target" ]] || fail "$label wrapper has no engineering target: $wrapper -> $target"
    case "$wrapper" in
        *.sh)
            grep -qF "exec " "$wrapper" \
                && grep -qF "$target" "$wrapper" \
                || fail "$label wrapper must exec its engineering target: $wrapper"
            ;;
        *.ps1)
            grep -qF "& " "$wrapper" \
                && grep -qF "$target" "$wrapper" \
                || fail "$label wrapper must invoke its engineering target: $wrapper"
            ;;
        *.py)
            grep -qF "runpy.run_path" "$wrapper" \
                && grep -qF "engineering" "$wrapper" \
                && grep -qF "scripts" "$wrapper" \
                && grep -qF "$(basename "$target")" "$wrapper" \
                || fail "$label wrapper must run its engineering target: $wrapper"
            ;;
        *)
            fail "$label wrapper has unsupported file type: $wrapper"
            ;;
    esac
}

require_rg

SCAN_ROOTS=(Cargo.toml build.rs src tests scripts engineering/scripts engineering/tests/scripts)

require_path "src/runtime/system_abilities"
require_path "src/runtime/system_ability_catalog"
require_path "src/runtime/ability_names"
require_path "src/runtime/executors"
require_path "src/runtime/agent_ability_specs.rs"
require_path "src/cli/mod.rs"
require_path "src/daemon/mod.rs"
require_path "src/daemon/control"
require_path "src/daemon/federation/client"
require_path "src/daemon/federation/directory.rs"
require_path "src/daemon/federation/directory_reader.rs"
require_path "src/daemon/federation/peers.rs"
require_path "src/daemon/identity/self_identity.rs"
require_path "src/daemon/invocation"
require_path "src/daemon/keyring/mod.rs"
require_path "src/daemon/trust/anchor.rs"
require_path "src/daemon/trust/cell.rs"
require_path "src/daemon/trust/key_resolver.rs"
require_path "ability-descriptors/system"
require_path "schemas"
require_path "engineering/benches"
require_path "engineering/docker"
require_path "engineering/scripts"
require_path "engineering/tests/scripts"
require_path "platforms/macos"
require_path "platforms/windows"
require_path "scripts"
require_path "tests/scripts"

reject_path "src/runtime/ability_runtime"
reject_path "src/runtime/agents"
reject_path "src/runtime/abilities"
reject_path "src/runtime/abilities.rs"
reject_path "src/daemon.rs"
reject_path "src/facade"
reject_path "src/services/control"
reject_path "src/services/federated_directory_reader.rs"
reject_path "src/services/federated_peers_cell.rs"
reject_path "src/services/federation_client"
reject_path "src/services/federation_directory.rs"
reject_path "src/services/invocation_transport"
reject_path "src/services/keyring.rs"
reject_path "src/services/realm_trust_anchor.rs"
reject_path "src/services/self_identity.rs"
reject_path "src/services/trust_anchor_cell.rs"
reject_path "src/services/trust_anchor_key_resolver.rs"
reject_path "abilities/system"
reject_path "docker"
reject_path "macos"
reject_path "windows"
reject_path "engineering/schemas"

for wrapper in scripts/*; do
    [[ -f "$wrapper" ]] || continue
    target="engineering/scripts/$(basename "$wrapper")"
    require_tool_wrapper "$wrapper" "$target" "root script"
done

for wrapper in tests/scripts/*.sh; do
    [[ -f "$wrapper" ]] || continue
    target="engineering/tests/scripts/$(basename "$wrapper")"
    require_bash_wrapper "$wrapper" "$target" "root test script"
done

for wrapper in benches/*.rs; do
    [[ -f "$wrapper" ]] || continue
    target="engineering/benches/$(basename "$wrapper")"
    [[ -f "$target" ]] || fail "bench wrapper has no engineering target: $wrapper -> $target"
    grep -q "engineering/benches/$(basename "$wrapper")" "$wrapper" \
        || fail "root bench must be a thin engineering/benches wrapper: $wrapper"
done

scan_must_be_empty \
    "active code must not import through retired runtime::agents paths" \
    '(^|[^[:alnum:]_])(crate::runtime::agents::|runtime::agents::)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not import through retired runtime::abilities paths" \
    '(^|[^[:alnum:]_])(crate::runtime::abilities::|runtime::abilities::)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not reference retired src/runtime/agents physical paths" \
    'src/runtime/agents/' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not reference retired src/runtime/abilities.rs physical path" \
    'src/runtime/abilities\.rs|runtime/abilities\.rs' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not hard-code retired abilities/system descriptor root" \
    'abilities/system' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not import through facade::cli compatibility paths" \
    '(^|[^[:alnum:]_])(crate::facade::cli::|easynet_cli::facade::cli::|facade::cli::)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not import through retired facade::mcp paths" \
    '(^|[^[:alnum:]_])(crate::facade::mcp::|easynet_cli::facade::mcp::|facade::mcp::)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not import through retired services::control paths" \
    '(^|[^[:alnum:]_])(crate::services::control::|easynet_cli::services::control::|services::control::)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not import through retired services::federation_client paths" \
    '(^|[^[:alnum:]_])(crate::services::federation_client::|easynet_cli::services::federation_client::|services::federation_client::)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not import through retired services federation directory paths" \
    '(^|[^[:alnum:]_])(crate::services::federation_directory::|easynet_cli::services::federation_directory::|services::federation_directory::|crate::services::federated_directory_reader::|easynet_cli::services::federated_directory_reader::|services::federated_directory_reader::|crate::services::federated_peers_cell::|easynet_cli::services::federated_peers_cell::|services::federated_peers_cell::)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not import through retired services::invocation_transport paths" \
    '(^|[^[:alnum:]_])(crate::services::invocation_transport::|easynet_cli::services::invocation_transport::|services::invocation_transport::)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not import through retired services trust paths" \
    '(^|[^[:alnum:]_])(crate::services::realm_trust_anchor::|easynet_cli::services::realm_trust_anchor::|services::realm_trust_anchor::|crate::services::trust_anchor_cell::|easynet_cli::services::trust_anchor_cell::|services::trust_anchor_cell::|crate::services::trust_anchor_key_resolver::|easynet_cli::services::trust_anchor_key_resolver::|services::trust_anchor_key_resolver::)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not import through retired services identity/keyring paths" \
    '(^|[^[:alnum:]_])(crate::services::keyring::|easynet_cli::services::keyring::|services::keyring::|crate::services::self_identity::|easynet_cli::services::self_identity::|services::self_identity::)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not reference retired src/facade physical paths" \
    'src/facade(/|$)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not reference retired src/services/control physical path" \
    'src/services/control(/|$)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not reference retired src/services/federation_client physical path" \
    'src/services/federation_client(/|$)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not reference retired src/services federation directory physical paths" \
    'src/services/(federation_directory\.rs|federated_directory_reader\.rs|federated_peers_cell\.rs)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not reference retired src/services/invocation_transport physical path" \
    'src/services/invocation_transport(/|$)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not reference retired src/services trust physical paths" \
    'src/services/(realm_trust_anchor\.rs|trust_anchor_cell\.rs|trust_anchor_key_resolver\.rs)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not reference retired src/services identity/keyring physical paths" \
    'src/services/(keyring\.rs|self_identity\.rs)' \
    "${SCAN_ROOTS[@]}"

scan_must_be_empty \
    "active code must not reference retired src/daemon.rs physical path" \
    'src/daemon\.rs' \
    "${SCAN_ROOTS[@]}"

echo "check-project-structure-v1: ok"
