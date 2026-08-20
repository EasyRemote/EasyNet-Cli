#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/tools/scripts/check-invocation-route-target-boundary.sh"

fail() {
  printf 'test_check_invocation_route_target_boundary: %s\n' "$1" >&2
  exit 1
}

make_fixture() {
  local fixture
  fixture="$(mktemp -d)"
  for file in \
    src/daemon/invocation/routing/route_target.rs \
    src/daemon/invocation/routing/remote_invoke.rs \
    src/daemon/ability/builtins/integrations/a2a/client.rs \
    src/cli/daemon_client/remote_system_ability.rs \
    src/cli/commands/invoke.rs \
    src/cli/commands/ability_stream.rs \
    src/cli/commands/ability_bidi.rs
  do
    mkdir -p "$fixture/$(dirname "$file")"
    cp "$ROOT/$file" "$fixture/$file"
  done
  printf '%s\n' "$fixture"
}

run_check() {
  CHECK_INVOCATION_ROUTE_TARGET_ROOT="$1" bash "$SCRIPT"
}

fixture="$(make_fixture)"
run_check "$fixture" >/dev/null || fail 'happy fixture must pass'
rm -rf "$fixture"

fixture="$(make_fixture)"
printf '\nfn parse_node_ura() {}\n' >>"$fixture/src/cli/commands/invoke.rs"
if run_check "$fixture" >/dev/null 2>&1; then
  rm -rf "$fixture"
  fail 'retired generic node parser must fail'
fi
rm -rf "$fixture"

fixture="$(make_fixture)"
perl -0pi -e 's/identity\.as_str\(\) != selector\.owner_ura\(\)/false/' \
  "$fixture/src/daemon/invocation/routing/route_target.rs"
if run_check "$fixture" >/dev/null 2>&1; then
  rm -rf "$fixture"
  fail 'removing exact callee ownership binding must fail'
fi
rm -rf "$fixture"

printf 'test_check_invocation_route_target_boundary: all cases passed\n'
