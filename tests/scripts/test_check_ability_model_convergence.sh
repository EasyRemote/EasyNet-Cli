#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-ability-model-convergence.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p "$SB/tools/scripts" \
  "$SB/src/core" \
  "$SB/src/daemon/ability/descriptors" \
  "$SB/src/daemon/ability/catalog" \
  "$SB/src/daemon/ability/builtins/resources/pages" \
  "$SB/src/daemon/ability/names" \
  "$SB/src/daemon/ability" \
  "$SB/src/daemon/plugins" \
  "$SB/plugins" \
  "$SB/tests"
cp "$SCRIPT" "$SB/tools/scripts/check-ability-model-convergence.sh"

cat >"$SB/src/daemon/ability/descriptors/mod.rs" <<'RS'
pub enum CallMode {
    Rpc,
}
RS

cat >"$SB/src/daemon/ability/dispatch.rs" <<'RS'
pub fn register_rpc_with_owner() {}
RS

cat >"$SB/src/daemon/ability/catalog/build.rs" <<'RS'
pub fn build_registry_for_daemon_result() {}
RS

cat >"$SB/src/daemon/ability/names/federation.rs" <<'RS'
pub const RESOLVE: &str = "federation.resolve";
RS

cat >"$SB/src/daemon/ability/names/device_control.rs" <<'RS'
pub const NODE_DESCRIBE: &str = "node.describe";
pub const NODE_REMOVE: &str = "node.remove";
RS

cat >"$SB/src/daemon/plugins/manifest.rs" <<'RS'
use crate::daemon::ability::CallMode;

pub struct PluginAbilityManifest {
    call_mode: CallMode,
}
RS

cat >"$SB/src/daemon/plugins/descriptor.rs" <<'RS'
use crate::daemon::ability::CallMode;

fn hints_for_call_mode(mode: CallMode) {}
RS

cat >"$SB/src/daemon/plugins/mod.rs" <<'RS'
pub use manifest::{PluginAbilityLayer, PluginPackageManifest};
RS

(
  cd "$SB"
  bash tools/scripts/check-ability-model-convergence.sh
) >/dev/null || fail "happy path should pass"

cat >"$SB/src/daemon/plugins/manifest.rs" <<'RS'
pub use crate::daemon::ability::CallMode;
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-ability-model-convergence.sh
) >/tmp/check-ability-model-convergence.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "manifest CallMode re-export should exit 1 (got $rc)"
grep -Fq "plugin manifest must consume descriptor CallMode" \
  /tmp/check-ability-model-convergence.out \
  || fail "re-export failure should name plugin manifest ownership"

cat >"$SB/src/daemon/plugins/manifest.rs" <<'RS'
use crate::daemon::ability::CallMode;
RS
cat >"$SB/src/daemon/plugins/descriptor.rs" <<'RS'
use crate::daemon::plugins::manifest::CallMode;
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-ability-model-convergence.sh
) >/tmp/check-ability-model-convergence.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "manifest-owned CallMode import should exit 1 (got $rc)"
grep -Fq "plugin code must import CallMode from daemon::ability" \
  /tmp/check-ability-model-convergence.out \
  || fail "import failure should name daemon::ability owner"

cat >"$SB/src/daemon/plugins/descriptor.rs" <<'RS'
use crate::daemon::ability::CallMode;
RS
cat >"$SB/src/daemon/plugins/mod.rs" <<'RS'
pub use manifest::{CallMode, PluginAbilityLayer};
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-ability-model-convergence.sh
) >/tmp/check-ability-model-convergence.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "plugins module CallMode re-export should exit 1 (got $rc)"
grep -Fq "plugin module must not re-export descriptor CallMode" \
  /tmp/check-ability-model-convergence.out \
  || fail "module re-export failure should name plugins module ownership"

cat >"$SB/src/daemon/plugins/mod.rs" <<'RS'
pub use manifest::{PluginAbilityLayer, PluginPackageManifest};
RS
cat >"$SB/src/daemon/ability/names/federation.rs" <<'RS'
pub const NODE_LIST: &str = "node.list";
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-ability-model-convergence.sh
) >/tmp/check-ability-model-convergence.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "federation-owned node.list should exit 1 (got $rc)"
grep -Fq "device lifecycle ability names must be owned by names::device_control" \
  /tmp/check-ability-model-convergence.out \
  || fail "node ownership failure should name device_control ownership"

cat >"$SB/src/daemon/ability/names/federation.rs" <<'RS'
pub const RESOLVE: &str = "federation.resolve";
RS
cat >"$SB/src/daemon/ability/names/device_control.rs" <<'RS'
pub const NODE_LIST: &str = "node.list";
pub const NODE_DESCRIBE: &str = "node.describe";
pub const NODE_REMOVE: &str = "node.remove";
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-ability-model-convergence.sh
) >/tmp/check-ability-model-convergence.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "retired device-owned node.list should exit 1 (got $rc)"
grep -Fq "retired node.list fleet route must not re-enter" \
  /tmp/check-ability-model-convergence.out \
  || fail "retired node.list failure should name catalogue route retirement"

cat >"$SB/src/daemon/ability/names/device_control.rs" <<'RS'
pub const NODE_DESCRIBE: &str = "node.describe";
pub const NODE_REMOVE: &str = "node.remove";
RS
cat >"$SB/src/daemon/ability/catalog/build.rs" <<'RS'
fn bad() {
    let _ = crate::daemon::ability::names::federation::NODE_LIST;
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-ability-model-convergence.sh
) >/tmp/check-ability-model-convergence.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "federation node constant import should exit 1 (got $rc)"
grep -Fq "device lifecycle callers must import node.* constants from names::device_control" \
  /tmp/check-ability-model-convergence.out \
  || fail "node import failure should name device_control import"

echo "test_check_ability_model_convergence.sh: all cases passed"
