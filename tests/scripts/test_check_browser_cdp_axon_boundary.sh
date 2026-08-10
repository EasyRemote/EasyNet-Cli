#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-browser-cdp-axon-boundary.sh"

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

fail() {
  printf 'test_check_browser_cdp_axon_boundary.sh: %s\n' "$1" >&2
  exit 1
}

mkdir -p "$SB/tools/scripts" "$SB/plugins" "$SB/src/daemon/plugins"
cp "$SCRIPT" "$SB/tools/scripts/check-browser-cdp-axon-boundary.sh"
cp "$REPO_ROOT/Cargo.toml" "$SB/Cargo.toml"
cp -R "$REPO_ROOT/plugins/browser" "$SB/plugins/browser"
cp "$REPO_ROOT/src/daemon/plugins/mod.rs" "$SB/src/daemon/plugins/mod.rs"
cp "$REPO_ROOT/src/daemon/plugins/contribution.rs" "$SB/src/daemon/plugins/contribution.rs"

(
  EASYNET_BROWSER_BOUNDARY_ROOT="$SB" \
    bash "$SB/tools/scripts/check-browser-cdp-axon-boundary.sh"
) >/dev/null || fail "canonical browser plugin fixture should pass"

mkdir -p "$SB/src/runtime/agents"
printf 'const PLACEHOLDER_WEBP: &[u8] = &[];\n' \
  >"$SB/src/runtime/agents/browser_session_ability.rs"
set +e
EASYNET_BROWSER_BOUNDARY_ROOT="$SB" \
  bash "$SB/tools/scripts/check-browser-cdp-axon-boundary.sh" \
  >/tmp/check-browser-cdp-axon-legacy.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "retired runtime browser owner should exit 1 (got $rc)"
rm "$SB/src/runtime/agents/browser_session_ability.rs"

printf '\nuse tokio::net::TcpListener;\n' >>"$SB/plugins/browser/src/handlers.rs"
set +e
EASYNET_BROWSER_BOUNDARY_ROOT="$SB" \
  bash "$SB/tools/scripts/check-browser-cdp-axon-boundary.sh" \
  >/tmp/check-browser-cdp-axon-socket.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "parallel public socket should exit 1 (got $rc)"
perl -0pi -e 's/\nuse tokio::net::TcpListener;\n$//' "$SB/plugins/browser/src/handlers.rs"

perl -0pi -e 's/--user-data-dir=/--profile-directory=/' "$SB/plugins/browser/src/chrome.rs"
set +e
EASYNET_BROWSER_BOUNDARY_ROOT="$SB" \
  bash "$SB/tools/scripts/check-browser-cdp-axon-boundary.sh" \
  >/tmp/check-browser-cdp-axon-profile.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "default-profile debugging regression should exit 1 (got $rc)"

printf 'test_check_browser_cdp_axon_boundary.sh: all cases passed\n'
