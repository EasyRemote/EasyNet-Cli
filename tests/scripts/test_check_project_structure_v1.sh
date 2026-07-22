#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-project-structure-v1.sh"

fail() {
  printf 'test_check_project_structure_v1: %s\n' "$1" >&2
  exit 1
}

mkfinal() {
  local dir="$1"

  mkdir -p "$dir"
  touch "$dir/Cargo.toml" "$dir/Cargo.lock" "$dir/README.md" "$dir/PROJECT_STRUCTURE.md" "$dir/build.rs"
  touch "$dir/README.pdf" "$dir/VERSION"
  mkdir -p "$dir/include"
  touch "$dir/include/easynet_cli.h" "$dir/include/easynet_cli.exports.v6"

  mkdir -p "$dir/src/bin"
  touch \
    "$dir/src/bin/easynet.rs" \
    "$dir/src/bin/easynet-daemon.rs" \
    "$dir/src/bin/easynet-keyring.rs" \
    "$dir/src/bin/gen-ability-tomls.rs" \
    "$dir/src/bin/real-user-smoke.rs" \
    "$dir/src/bin/verify-voice-contract.rs"

  mkdir -p \
    "$dir/src/core/agent" \
    "$dir/src/core/identity" \
    "$dir/src/core/ura" \
    "$dir/src/core/domain" \
    "$dir/src/daemon/boot" \
    "$dir/src/daemon/control" \
    "$dir/src/daemon/invocation/admission" \
    "$dir/src/daemon/invocation/routing" \
    "$dir/src/daemon/invocation/dispatch" \
    "$dir/src/daemon/invocation/receipts" \
    "$dir/src/daemon/invocation/streams" \
    "$dir/src/daemon/invocation/bidi" \
    "$dir/src/daemon/ability/names" \
    "$dir/src/daemon/ability/descriptors" \
    "$dir/src/daemon/ability/authority" \
    "$dir/src/daemon/ability/impl_bindings" \
    "$dir/src/daemon/ability/catalog" \
    "$dir/src/daemon/ability/wire" \
    "$dir/src/daemon/ability/builtins/agents" \
    "$dir/src/daemon/ability/builtins/device_control" \
    "$dir/src/daemon/ability/builtins/resources" \
    "$dir/src/daemon/ability/builtins/automation" \
    "$dir/src/daemon/ability/builtins/integrations" \
    "$dir/src/daemon/ability/builtins/governance" \
    "$dir/src/daemon/execution/pty" \
    "$dir/src/daemon/execution/mcp" \
    "$dir/src/daemon/execution/mission" \
    "$dir/src/daemon/execution/schedule" \
    "$dir/src/daemon/execution/loop_instance" \
    "$dir/src/daemon/execution/permission" \
    "$dir/src/daemon/execution/session" \
    "$dir/src/daemon/resources/skills" \
    "$dir/src/daemon/resources/pages" \
    "$dir/src/daemon/resources/context" \
    "$dir/src/daemon/resources/files" \
    "$dir/src/daemon/resources/media" \
    "$dir/src/daemon/identity" \
    "$dir/src/daemon/trust" \
    "$dir/src/daemon/keyring" \
    "$dir/src/daemon/federation" \
    "$dir/src/daemon/plugins" \
    "$dir/src/daemon/persistence" \
    "$dir/src/daemon/axon_bridge" \
    "$dir/src/daemon/telemetry" \
    "$dir/src/cli/commands" \
    "$dir/src/cli/presentation" \
    "$dir/src/cli/daemon_client" \
    "$dir/src/cli/mcp" \
    "$dir/src/ffi/daemon" \
    "$dir/src/ffi/client" \
    "$dir/src/ffi/invocation" \
    "$dir/src/ffi/errors" \
    "$dir/src/ffi/features" \
    "$dir/src/ffi/strings" \
    "$dir/src/eal/parser" \
    "$dir/src/eal/interpreter" \
    "$dir/src/eal/runtime" \
    "$dir/src/eal/diagnostics" \
    "$dir/src/support/async_bridge" \
    "$dir/src/support/shellguard" \
    "$dir/src/support/platform"

  touch "$dir/src/lib.rs" "$dir/src/ffi/mod.rs" "$dir/src/eal/mod.rs" "$dir/src/support/mod.rs"

  mkdir -p \
    "$dir/sdk/go" "$dir/sdk/python" "$dir/sdk/node" "$dir/sdk/java" "$dir/sdk/swift" \
    "$dir/sdk/schemas" "$dir/sdk/conformance/cases" "$dir/sdk/conformance/fixtures" "$dir/sdk/conformance/runner" \
    "$dir/ability-descriptors/system/agents" \
    "$dir/ability-descriptors/system/federation" \
    "$dir/ability-descriptors/system/device_control" \
    "$dir/ability-descriptors/system/resources" \
    "$dir/ability-descriptors/system/automation" \
    "$dir/ability-descriptors/system/integrations" \
    "$dir/ability-descriptors/system/governance" \
    "$dir/schemas/descriptor" "$dir/schemas/receipt" \
    "$dir/plugins" "$dir/skills" "$dir/examples" "$dir/gallery" "$dir/docs" \
    "$dir/tests/e2e" "$dir/tests/conformance" "$dir/tests/fixtures" "$dir/tests/scripts" "$dir/tests/support" \
    "$dir/tools/benches" "$dir/tools/sdk-conformance-runner/src" \
    "$dir/provider_routes" \
    "$dir/packaging/docker" "$dir/packaging/release" \
    "$dir/.github/workflows"
  touch "$dir/schemas/control_plane.proto" "$dir/schemas/common.proto"
  touch "$dir/tools/sdk-conformance-runner/Cargo.toml" "$dir/tools/sdk-conformance-runner/src/main.rs"
  touch \
    "$dir/provider_routes/easynet-access-control-routes.v1.json" \
    "$dir/provider_routes/easynet-principal-lifecycle-routes.v1.json" \
    "$dir/provider_routes/easynet-receipt-routes.v1.json" \
    "$dir/provider_routes/easynet-runtime-admin-routes.v1.json" \
    "$dir/provider_routes/generate_access_control_routes.py" \
    "$dir/provider_routes/generate_principal_routes.py" \
    "$dir/provider_routes/generate_receipt_routes.py" \
    "$dir/provider_routes/generate_runtime_admin_routes.py" \
    "$dir/provider_routes/route_generator.py"
}

expect_fail() {
  local dir="$1"
  if "$CHECK" "$dir" >/tmp/project-structure-v1-test.out 2>&1; then
    cat /tmp/project-structure-v1-test.out >&2
    fail "expected check to fail for $dir"
  fi
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB" /tmp/project-structure-v1-test.out' EXIT

mkfinal "$SB/pass"
"$CHECK" "$SB/pass" >/dev/null

cp -R "$SB/pass" "$SB/forbidden"
mkdir -p "$SB/forbidden/src/runtime"
expect_fail "$SB/forbidden"

cp -R "$SB/pass" "$SB/extra-bin"
touch "$SB/extra-bin/src/bin/snapshot-probe.rs"
expect_fail "$SB/extra-bin"

cp -R "$SB/pass" "$SB/extra-root-file"
touch "$SB/extra-root-file/CHANGELOG.pdf"
expect_fail "$SB/extra-root-file"

cp -R "$SB/pass" "$SB/missing-version-file"
rm "$SB/missing-version-file/VERSION"
expect_fail "$SB/missing-version-file"

cp -R "$SB/pass" "$SB/missing-readme-pdf"
rm "$SB/missing-readme-pdf/README.pdf"
expect_fail "$SB/missing-readme-pdf"

cp -R "$SB/pass" "$SB/flat-descriptor"
touch "$SB/flat-descriptor/ability-descriptors/system/fs.read.ability.toml"
expect_fail "$SB/flat-descriptor"

cp -R "$SB/pass" "$SB/provider-pycache"
mkdir -p "$SB/provider-pycache/provider_routes/__pycache__"
touch "$SB/provider-pycache/provider_routes/__pycache__/route_generator.cpython-312.pyc"
expect_fail "$SB/provider-pycache"

cp -R "$SB/pass" "$SB/missing-invocation"
rm -rf "$SB/missing-invocation/src/daemon/invocation/admission"
expect_fail "$SB/missing-invocation"

cp -R "$SB/pass" "$SB/flat-ffi"
touch "$SB/flat-ffi/src/ffi/daemon.rs"
expect_fail "$SB/flat-ffi"

printf 'test_check_project_structure_v1 ok\n'
