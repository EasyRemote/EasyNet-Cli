#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_PLATFORM_INPUT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
INPUT="$ROOT/plugins/remote-desktop/src/input.rs"
WINDOWS="$ROOT/plugins/remote-desktop/src/input/windows.rs"
LINUX="$ROOT/plugins/remote-desktop/src/input/linux.rs"
VIEW_DEVICE="$ROOT/plugins/remote-desktop/src/view_device.rs"
MANIFEST="$ROOT/plugins/remote-desktop/plugin.toml"
CARGO_MANIFEST="$ROOT/Cargo.toml"

fail() {
  printf 'check-remoteapp-platform-input-backends: %s\n' "$1" >&2
  exit 1
}

require() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  rg -q -- "$pattern" "$path" || fail "$message"
}

reject() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  if rg -q -- "$pattern" "$path"; then
    fail "$message"
  fi
}

for path in "$INPUT" "$WINDOWS" "$LINUX" "$VIEW_DEVICE" "$MANIFEST" "$CARGO_MANIFEST"; do
  [[ -f "$path" ]] || fail "missing required source ${path#"$ROOT/"}"
done

require '#\[cfg\(target_os = "windows"\)\]' "$INPUT" \
  'input platform dispatch must compile a dedicated Windows backend'
require '#\[path = "input/windows.rs"\]' "$INPUT" \
  'Windows input backend must remain independently bounded'
require '#\[cfg\(target_os = "linux"\)\]' "$INPUT" \
  'input platform dispatch must compile a dedicated Linux backend'
require '#\[path = "input/linux.rs"\]' "$INPUT" \
  'Linux input backend must remain independently bounded'
require 'input_injection_unavailable_reason\(\)' "$INPUT" \
  'session activation must preserve typed platform input failure reasons'
require 'validate_live_target_input' "$INPUT" \
  'platform injection must remain downstream of the fresh target guard'

require 'SendInput' "$WINDOWS" \
  'Windows input must execute through User32 SendInput'
require 'MOUSEEVENTF_VIRTUALDESK' "$WINDOWS" \
  'Windows absolute pointer mapping must cover the virtual desktop'
require 'GetSystemMetrics' "$WINDOWS" \
  'Windows pointer normalization must use live virtual desktop bounds'
require 'map_pointer_point\(frame, target\)' "$WINDOWS" \
  'Windows input must consume the committed target-local coordinate mapping'
require 'MAX_WHEEL_DELTA_PER_FRAME' "$WINDOWS" \
  'Windows wheel injection must be bounded per input frame'
require 'applied as usize == inputs\.len\(\)' "$WINDOWS" \
  'Windows input must reject partial SendInput application'
require 'windows_send_input_denied' "$WINDOWS" \
  'Windows UIPI/SendInput denial must have a stable reason'
require 'virtual_desktop_absolute_mapping_clamps_multi_monitor_coordinates' "$WINDOWS" \
  'Windows virtual-desktop normalization must retain an executable unit contract'
require 'windows_wheel_delta_is_bounded_per_frame' "$WINDOWS" \
  'Windows wheel bounds must retain an executable unit contract'
reject 'SetCursorPos' "$WINDOWS" \
  'Windows pointer and button effects must use one auditable SendInput path'

require 'OnceLock<Mutex<X11InputBackend>>' "$LINUX" \
  'Linux X11 connection must be cached and serialized'
require 'Library::new' "$LINUX" \
  'Linux X11/XTest must be dynamically loaded without distro dev-package linkage'
require 'XTestQueryExtension' "$LINUX" \
  'Linux input must prove XTest availability before advertising runtime readiness'
require 'XTestFakeMotionEvent' "$LINUX" \
  'Linux pointer input must execute through XTest'
require 'XTestFakeKeyEvent' "$LINUX" \
  'Linux keyboard input must execute through XTest'
require 'map_pointer_point\(frame, target\)' "$LINUX" \
  'Linux input must consume the committed target-local coordinate mapping'
require 'MAX_WHEEL_STEPS_PER_AXIS' "$LINUX" \
  'Linux wheel injection expansion must be explicitly bounded'
require 'WAYLAND_DISPLAY' "$LINUX" \
  'Linux backend must detect Wayland rather than silently injecting through XWayland'
require 'linux_wayland_portal_remote_desktop_not_implemented' "$LINUX" \
  'Linux pure/ambiguous Wayland input must remain explicitly fail-closed'
require 'linux_wheel_expansion_is_bounded' "$LINUX" \
  'Linux wheel expansion must retain an executable unit contract'
require 'linux_dom_key_mapping_is_deterministic' "$LINUX" \
  'Linux DOM-key translation must retain an executable unit contract'
reject 'Command::new|xdotool|ydotool' "$LINUX" \
  'Linux input must not shell out to an ungoverned automation process'

require 'Win32_UI_Input_KeyboardAndMouse' "$CARGO_MANIFEST" \
  'Cargo Windows features must include SendInput definitions'
require 'Win32_UI_WindowsAndMessaging' "$CARGO_MANIFEST" \
  'Cargo Windows features must include virtual desktop metrics'
require 'libloading = "0\.8"' "$CARGO_MANIFEST" \
  'Cargo Linux dependencies must declare the dynamic loader directly'
require 'platforms = \["macos", "linux", "windows"\]' "$MANIFEST" \
  'RemoteApp plugin manifest must install on Windows as well as macOS/Linux'

require 'windows_sendinput_target_guard_ready' "$VIEW_DEVICE" \
  'device capabilities must expose Windows guarded input baseline'
require 'linux_x11_xtest_target_guard_ready' "$VIEW_DEVICE" \
  'device capabilities must expose Linux X11 guarded input baseline'
require 'runtime_blocked_reason' "$VIEW_DEVICE" \
  'device capabilities must separate current-host runtime failure from implementation state'
require 'live_e2e_required' "$VIEW_DEVICE" \
  'device capabilities must retain the live OS-effect certification requirement'

printf 'check-remoteapp-platform-input-backends: ok\n'
