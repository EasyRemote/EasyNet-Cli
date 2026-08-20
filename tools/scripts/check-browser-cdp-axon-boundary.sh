#!/usr/bin/env bash
set -euo pipefail

ROOT="${EASYNET_BROWSER_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "$ROOT"

fail() {
  printf 'browser CDP/Axon boundary violation: %s\n' "$1" >&2
  exit 1
}

require_file() {
  [[ -f "$1" ]] || fail "missing $1"
}

require_fixed() {
  local token="$1"
  local path="$2"
  grep -Fq -- "$token" "$path" || fail "$path must contain $token"
}

PLUGIN="plugins/browser"
ABILITY_DIR="$PLUGIN/abilities"

for path in \
  "$PLUGIN/Cargo.toml" \
  "$PLUGIN/plugin.toml" \
  "$PLUGIN/src/lib.rs" \
  "$PLUGIN/src/embedded.rs" \
  "$PLUGIN/src/cdp.rs" \
  "$PLUGIN/src/chrome.rs" \
  "$PLUGIN/src/handlers.rs" \
  "$PLUGIN/src/performance.rs" \
  "$PLUGIN/src/registration.rs" \
  "$PLUGIN/src/runtime.rs" \
  "$PLUGIN/src/session.rs" \
  "$PLUGIN/tools/install-current-chrome-for-testing.sh"
do
  require_file "$path"
done

[[ ! -e src/runtime/agents/browser_session_ability.rs ]] \
  || fail "retired runtime-agent browser implementation must stay deleted"
[[ ! -e src/daemon/ability/builtins/device_control/browser.rs ]] \
  || fail "browser execution must not return to the system device-control owner"

require_fixed 'entrypoint = "easynet_plugin_browser::provider"' "$PLUGIN/plugin.toml"
require_fixed 'kind = "builtin"' "$PLUGIN/plugin.toml"
require_fixed '"plugins/browser"' Cargo.toml
require_fixed '#[path = "../../../plugins/browser/src/embedded.rs"]' src/daemon/plugins/mod.rs
require_fixed 'crate::daemon::plugins::browser::provider()' src/daemon/plugins/mod.rs
require_fixed 'OwnerKind::plugin_management_system()' src/daemon/plugins/contribution.rs

descriptor_count="$(find "$ABILITY_DIR" -maxdepth 1 -name 'browser.*.ability.toml' -type f | wc -l | tr -d '[:space:]')"
[[ "$descriptor_count" == "6" ]] \
  || fail "browser package must own exactly 6 descriptors, found $descriptor_count"

for descriptor in "$ABILITY_DIR"/browser.*.ability.toml; do
  require_fixed 'schema_version = "3"' "$descriptor"
  require_fixed 'exposure = "operator"' "$descriptor"
  require_fixed 'dedicated_surface = "browser"' "$descriptor"
  require_fixed 'subject_contract_kind = "dedicated-surface"' "$descriptor"
  require_fixed 'capability_state = "provider_backed"' "$descriptor"
  if [[ "$(basename "$descriptor")" != "browser.open_session.ability.toml" ]]; then
    require_fixed 'scope_subjects_uras = ["resource"]' "$descriptor"
  fi
done
require_fixed 'call_mode = "bidi"' "$ABILITY_DIR/browser.attach_session.ability.toml"
require_fixed 'bidi_wire_kind = "json_frames"' "$PLUGIN/plugin.toml"

require_fixed 'CallMode::Bidi' "$PLUGIN/src/registration.rs"
require_fixed 'PluginBidiWireKind::JsonFrames' "$PLUGIN/src/registration.rs"
require_fixed 'BuiltinPluginFrontendContract::OPERATOR_BROWSER' "$PLUGIN/src/registration.rs"
require_fixed 'BidiSource' "$PLUGIN/src/handlers.rs"
require_fixed 'BidiOutputFrame' "$PLUGIN/src/handlers.rs"
require_fixed '"transport": "axon_invoke_bidi"' "$PLUGIN/src/handlers.rs"
require_fixed 'validate_agent_command' "$PLUGIN/src/handlers.rs"
require_fixed '"cdp.batch"' "$PLUGIN/src/handlers.rs"
require_fixed 'ATTACH_BATCH_COMMAND_BOUND' "$PLUGIN/src/handlers.rs"
require_fixed 'current_chrome_axon_bidi_performance' "$PLUGIN/src/performance.rs"
require_fixed 'MAX_CORRELATION_ID_BYTES' "$PLUGIN/src/handlers.rs"
require_fixed 'MAX_CDP_METHOD_BYTES' "$PLUGIN/src/cdp.rs"
require_fixed 'MAX_INPUT_TEXT_BYTES' "$PLUGIN/src/input.rs"
require_fixed '"maxLength": MAX_URL_BYTES' "$PLUGIN/src/schema.rs"

require_fixed '--remote-debugging-port=0' "$PLUGIN/src/chrome.rs"
require_fixed '--remote-debugging-address=127.0.0.1' "$PLUGIN/src/chrome.rs"
require_fixed '--user-data-dir=' "$PLUGIN/src/chrome.rs"
require_fixed 'if options.headless' "$PLUGIN/src/chrome.rs"
require_fixed 'Browser.getVersion' "$PLUGIN/src/chrome.rs"
require_fixed 'Target.attachToTarget' "$PLUGIN/src/chrome.rs"
require_fixed '"flatten": true' "$PLUGIN/src/chrome.rs"
require_fixed 'DevToolsActivePort' "$PLUGIN/src/chrome.rs"
require_fixed 'executable_version_key' "$PLUGIN/src/chrome.rs"
require_fixed 'require_loopback' "$PLUGIN/src/chrome.rs"
require_fixed 'EASYNET_BROWSER_CHROME_ROOT' "$PLUGIN/src/chrome.rs"
require_fixed 'last-known-good-versions-with-downloads.json' "$PLUGIN/tools/install-current-chrome-for-testing.sh"
require_fixed 'chrome-for-testing-public' "$PLUGIN/tools/install-current-chrome-for-testing.sh"
require_fixed 'actual_version' "$PLUGIN/tools/install-current-chrome-for-testing.sh"

for state in Starting Active Closing Closed Failed; do
  require_fixed "$state" "$PLUGIN/src/session.rs"
done
require_fixed 'require_access' "$PLUGIN/src/session.rs"
require_fixed 'CloseDisposition' "$PLUGIN/src/session.rs"
require_fixed 'begin_attachment' "$PLUGIN/src/session.rs"
require_fixed 'begin_capture' "$PLUGIN/src/session.rs"
require_fixed 'closing_sessions' "$PLUGIN/src/runtime.rs"
require_fixed 'PendingCallLease' "$PLUGIN/src/cdp.rs"
require_fixed 'wait_for_response_or_disconnect' "$PLUGIN/src/cdp.rs"

if rg -n 'Google Chrome Canary' "$PLUGIN/src/chrome.rs"; then
  fail "automatic browser resolution must stay on Stable channels"
fi

if rg -n 'V0 MOCK|PLACEHOLDER_WEBP|is_placeholder|browser_session_ability|\.agent-browser' "$PLUGIN"; then
  fail "retired mock or foreign-project browser ownership leaked into the package"
fi

if rg -n 'TcpListener|WebSocketUpgrade|axum::serve|warp::serve|Server::bind' "$PLUGIN/src"; then
  fail "browser plugin must not expose a parallel public socket server"
fi

tungstenite_owners="$(rg -l 'tokio_tungstenite' "$PLUGIN/src" || true)"
[[ "$tungstenite_owners" == "$PLUGIN/src/cdp.rs" ]] \
  || fail "only the internal CDP adapter may own the Chrome WebSocket"

printf 'browser CDP/Axon boundary checks passed\n'
