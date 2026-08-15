#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-ability-identity-owner-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

for file in \
  src/daemon/ability/catalog/ownership.rs \
  src/daemon/ability/catalog/profiles/device.rs \
  src/daemon/federation/read_model/owner_projection.rs \
  src/daemon/ability/descriptors/surface.rs \
  src/daemon/ability/builtins/agents/list.rs \
  src/support/platform/local_invoke.rs \
  src/daemon/invocation/routing/remote_invoke.rs \
  src/cli/daemon_client/ability_catalog.rs; do
  mkdir -p "$SANDBOX/$(dirname "$file")"
  cp "$REPO_ROOT/$file" "$SANDBOX/$file"
done

CHECK_ABILITY_IDENTITY_OWNER_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e 's/crate::core::ura::URAKind::Authority =>/crate::core::ura::URAKind::Device => parsed.device_id().map(|device_id| crate::core::ura::device_ura(\&parsed.realm, device_id)),\n            crate::core::ura::URAKind::Authority =>/' \
  "$SANDBOX/src/daemon/ability/descriptors/surface.rs"
if CHECK_ABILITY_IDENTITY_OWNER_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "identity-owner checker accepted Device as an AbilityDescriptor owner" >&2
  exit 1
fi
cp "$REPO_ROOT/src/daemon/ability/descriptors/surface.rs" \
  "$SANDBOX/src/daemon/ability/descriptors/surface.rs"

perl -0pi -e 's/runtime_introspection_owner_for_execution_target\(execution_target_ura\)\?/execution_target_ura.to_string()/' \
  "$SANDBOX/src/daemon/invocation/routing/remote_invoke.rs"
if CHECK_ABILITY_IDENTITY_OWNER_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "identity-owner checker accepted a Device-as-callee catalogue target" >&2
  exit 1
fi

echo "test_check_ability_identity_owner_boundary.sh: all cases passed"
