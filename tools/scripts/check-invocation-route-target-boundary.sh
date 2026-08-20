#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_INVOCATION_ROUTE_TARGET_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'check-invocation-route-target-boundary: %s\n' "$1" >&2
  exit 1
}

require() {
  local pattern="$1"
  local file="$2"
  local message="$3"
  rg -q -- "$pattern" "$file" || fail "$message"
}

reject() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  if rg -q -- "$pattern" "$path"; then
    fail "$message"
  fi
}

ROUTING=src/daemon/invocation/routing/route_target.rs

reject 'parse_node_ura' src \
  'generic Device-or-Authority node parser is retired; use typed placement or exact-callee targets'
require 'pub\(crate\) fn parse_device_placement_ura\(' "$ROUTING" \
  'device-hosted APIs must use a Device-only placement parser'
require 'pub\(crate\) enum RemoteAbilityRouteTarget' "$ROUTING" \
  'public ability routing must use an explicit route-target value object'
require 'DevicePlacement\(String\)' "$ROUTING" \
  'route target must model Device as placement, not callee'
require 'ExactCallee\(String\)' "$ROUTING" \
  'route target must model Agent/SystemAgent/Service/Authority as exact callees'
require 'URAKind::Agent | URAKind::Service | URAKind::Authority' "$ROUTING" \
  'exact callable route target must accept Service-owned public callees'
require 'identity\.as_str\(\) != selector\.owner_ura\(\)' "$ROUTING" \
  'exact callable targets must match the selected Ability owner'
require 'direct Device-owned ability URA' src/daemon/invocation/routing/remote_invoke.rs \
  'remote routing must continue rejecting Device-owned ability inference'

for file in \
  src/cli/commands/invoke.rs \
  src/cli/commands/ability_stream.rs \
  src/cli/commands/ability_bidi.rs
do
  require 'RemoteAbilityRouteTarget::parse\(' "$file" \
    "$file must parse --node through the typed ability route target"
done

require 'parse_device_placement_ura\(' src/cli/daemon_client/remote_system_ability.rs \
  'device system APIs must retain Device-only placement semantics'
require 'URAKind::Device | URAKind::Authority' src/daemon/ability/builtins/integrations/a2a/client.rs \
  'A2A target_node must remain bounded to canonical Device or Authority URAs'

printf 'check-invocation-route-target-boundary: ok\n'
