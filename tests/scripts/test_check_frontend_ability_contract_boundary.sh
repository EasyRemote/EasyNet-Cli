#!/usr/bin/env bash

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-frontend-ability-contract-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p "$SANDBOX/ability-descriptors/system"
mkdir -p "$SANDBOX/plugins/remote-desktop/abilities"
cp "$REPO_ROOT/ability-descriptors/system/governance/meta.list_abilities.ability.toml" \
  "$SANDBOX/ability-descriptors/system/meta.list_abilities.ability.toml"
cp "$REPO_ROOT/plugins/remote-desktop/abilities/remote_desktop.create_session.ability.toml" \
  "$SANDBOX/plugins/remote-desktop/abilities/remote_desktop.create_session.ability.toml"

CHECK_FRONTEND_ABILITY_CONTRACT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e 's/subject_contract_kind = "route-target"/subject_contract_kind = "dedicated-surface"/' \
  "$SANDBOX/ability-descriptors/system/meta.list_abilities.ability.toml"
if CHECK_FRONTEND_ABILITY_CONTRACT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "frontend contract checker accepted a dedicated subject without a surface" >&2
  exit 1
fi
perl -0pi -e 's/subject_contract_kind = "dedicated-surface"/subject_contract_kind = "route-target"/' \
  "$SANDBOX/ability-descriptors/system/meta.list_abilities.ability.toml"
perl -0pi -e 's/dedicated_surface = "remote_desktop"/dedicated_surface = "media"/' \
  "$SANDBOX/plugins/remote-desktop/abilities/remote_desktop.create_session.ability.toml"
if CHECK_FRONTEND_ABILITY_CONTRACT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "frontend contract checker accepted remote_desktop ability on media surface" >&2
  exit 1
fi
perl -0pi -e 's/dedicated_surface = "media"/dedicated_surface = "remote_desktop"/' \
  "$SANDBOX/plugins/remote-desktop/abilities/remote_desktop.create_session.ability.toml"
perl -0pi -e 's/name = "meta.list_abilities"/name = "meta.remote_desktop_fake"/; s/dedicated_surface = "none"/dedicated_surface = "remote_desktop"/; s/subject_contract_kind = "route-target"/subject_contract_kind = "dedicated-surface"/' \
  "$SANDBOX/ability-descriptors/system/meta.list_abilities.ability.toml"
if CHECK_FRONTEND_ABILITY_CONTRACT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "frontend contract checker accepted non-remote_desktop ability on remote_desktop surface" >&2
  exit 1
fi

echo "test_check_frontend_ability_contract_boundary.sh: all cases passed"
