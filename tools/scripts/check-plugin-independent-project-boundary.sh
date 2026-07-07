#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail() {
  printf 'plugin-independent-project boundary violation: %s\n' "$1" >&2
  exit 1
}

test ! -e src/daemon/resources/remote_desktop \
  || fail "src/daemon/resources/remote_desktop must not exist"

if rg -n 'resources::remote_desktop|daemon::resources::remote_desktop' src plugins >/tmp/easynet-plugin-boundary-rg.txt; then
  cat /tmp/easynet-plugin-boundary-rg.txt >&2
  fail "remote desktop code must not be imported through daemon resources"
fi

if rg -n 'src/daemon/resources/remote_desktop' src plugins >/tmp/easynet-plugin-boundary-rg.txt; then
  cat /tmp/easynet-plugin-boundary-rg.txt >&2
  fail "remote desktop file ownership headers must not point at daemon resources"
fi

grep -q 'entrypoint = "easynet_plugin_remote_desktop::provider"' \
  plugins/remote-desktop/plugin.toml \
  || fail "remote desktop manifest must name the provider export"

test -f plugins/remote-desktop/Cargo.toml \
  || fail "remote desktop package must own a Cargo.toml"
test -f plugins/remote-desktop/src/lib.rs \
  || fail "remote desktop provider export must live under plugins/remote-desktop/src"
test -f plugins/remote-desktop/src/embedded.rs \
  || fail "remote desktop native-static implementation must be package-owned"

test -d plugins/desktop-menubar/companion/macos/EasyNetMenuBar \
  || fail "macOS companion app must be package-owned"
test -d plugins/desktop-menubar/companion/windows/EasyNetTray \
  || fail "Windows companion app must be package-owned"
test ! -e platforms \
  || fail "obsolete platforms/ app owner tree must not exist"

if rg -n 'platforms/(macos/EasyNetMenuBar|windows/EasyNetTray)' plugins tools src >/tmp/easynet-plugin-boundary-rg.txt; then
  cat /tmp/easynet-plugin-boundary-rg.txt >&2
  fail "active source must not point companion ownership at platforms/"
fi

printf 'plugin-independent-project boundary checks passed\n'
