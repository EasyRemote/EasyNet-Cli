#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-plugin-independent-project-boundary.sh"

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

fail() {
  echo "test_check_plugin_independent_project_boundary.sh: $1" >&2
  exit 1
}

mkdir -p "$SB/tools/scripts" \
  "$SB/plugins/remote-desktop/src" \
  "$SB/plugins/desktop-menubar/companion/macos/EasyNetMenuBar" \
  "$SB/plugins/desktop-menubar/companion/windows/EasyNetTray" \
  "$SB/src/daemon/plugins"

cp "$SCRIPT" "$SB/tools/scripts/check-plugin-independent-project-boundary.sh"

cat >"$SB/plugins/remote-desktop/plugin.toml" <<'TOML'
entrypoint = "easynet_plugin_remote_desktop::provider"
TOML
cat >"$SB/plugins/remote-desktop/Cargo.toml" <<'TOML'
[package]
name = "remote-desktop"
version = "0.1.0"
edition = "2021"
TOML
cat >"$SB/plugins/remote-desktop/src/lib.rs" <<'RS'
pub fn provider() {}
RS
cat >"$SB/plugins/remote-desktop/src/embedded.rs" <<'RS'
pub fn provider_kind() -> &'static str { "NativeStatic" }
RS
cat >"$SB/src/daemon/plugins/provider.rs" <<'RS'
pub enum PluginProviderKind {
    NativeStatic,
    Sidecar,
    Declarative,
    InstallablePackage,
}
RS
cat >"$SB/src/daemon/plugins/provider_registry.rs" <<'RS'
fn accepts(kind: &str) -> bool {
    kind == "NativeStatic" || kind == "InstallablePackage"
}
RS
cat >"$SB/src/daemon/plugins/mod.rs" <<'RS'
fn desktop_menubar_provider_kind() -> &'static str { "InstallablePackage" }
RS
cat >"$SB/src/daemon/plugins/package.rs" <<'RS'
fn materialized_provider_kind() -> &'static str { "InstallablePackage" }
RS

(
  cd "$SB"
  bash tools/scripts/check-plugin-independent-project-boundary.sh
) >/tmp/check-plugin-independent-project-boundary.out 2>&1 \
  || fail "canonical fixture should pass"

cat >"$SB/src/daemon/plugins/provider.rs" <<'RS'
pub enum PluginProviderKind {
    NativeStatic,
    DesktopCompanion,
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-plugin-independent-project-boundary.sh
) >/tmp/check-plugin-independent-project-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "DesktopCompanion provider kind should exit 1 (got $rc)"
grep -Fq "desktop companion is a package kind, not a provider kind" \
  /tmp/check-plugin-independent-project-boundary.out \
  || fail "provider-kind failure should name package/provider ownership"

cat >"$SB/src/daemon/plugins/provider.rs" <<'RS'
pub enum PluginProviderKind {
    NativeStatic,
    InstallablePackage,
}
RS
cat >"$SB/src/daemon/plugins/mod.rs" <<'RS'
fn desktop_menubar_provider_kind() -> PluginProviderKind {
    PluginProviderKind::DesktopCompanion
}
RS

set +e
(
  cd "$SB"
  bash tools/scripts/check-plugin-independent-project-boundary.sh
) >/tmp/check-plugin-independent-project-boundary.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "DesktopCompanion provider-kind caller should exit 1 (got $rc)"
grep -Fq "provider kind classification must stay product-neutral" \
  /tmp/check-plugin-independent-project-boundary.out \
  || fail "provider-kind caller failure should name product-neutral classification"

echo "test_check_plugin_independent_project_boundary.sh: all cases passed"
