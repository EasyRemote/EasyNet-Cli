#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_PLATFORM_INPUT_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
INPUT="$ROOT/plugins/remote-desktop/src/input.rs"
WINDOWS="$ROOT/plugins/remote-desktop/src/input/windows.rs"
LINUX="$ROOT/plugins/remote-desktop/src/input/linux.rs"
WHEEL="$ROOT/plugins/remote-desktop/src/input/wheel.rs"
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

for path in "$INPUT" "$WINDOWS" "$LINUX" "$WHEEL" "$VIEW_DEVICE" "$MANIFEST" "$CARGO_MANIFEST"; do
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
require 'validate_target_input_observation' "$INPUT" \
  'keyboard injection must remain downstream of the fresh target guard'
require 'validate_target_pointer_input_observation' "$INPUT" \
  'pointer injection must remain downstream of fresh target and occlusion guards'
require 'struct AppliedInputState' "$INPUT" \
  'device input channel must own successfully applied pressed-state lifecycle'
require 'MAX_TRACKED_PRESSED_KEYS' "$INPUT" \
  'device pressed-key state must remain hard-bounded'
require 'MAX_TRACKED_PRESSED_BUTTONS' "$INPUT" \
  'device pressed-button state must remain hard-bounded'
require 'fn tracked_release\(' "$INPUT" \
  'matching key/button releases must be recognized as reducing operations'
reject 'impl Drop for AppliedInputState' "$INPUT" \
  'plain pressed-state data must not perform ungoverned host effects from Drop'
require 'struct AppliedInputReleaseGuard' "$INPUT" \
  'input task cancellation must couple pressed state to an explicit cleanup capability'
require 'impl Drop for AppliedInputReleaseGuard' "$INPUT" \
  'input task cancellation must retain a terminal pressed-state cleanup guard'
require 'TargetSafetyReleasePermit' "$INPUT" \
  'terminal cleanup must use a transport-exact reducing-operation permit'
require 'terminal_input_release' "$INPUT" \
  'input channel close diagnostics must expose terminal release outcomes'

require 'SendInput' "$WINDOWS" \
  'Windows input must execute through User32 SendInput'
require 'MOUSEEVENTF_VIRTUALDESK' "$WINDOWS" \
  'Windows absolute pointer mapping must cover the virtual desktop'
require 'GetSystemMetrics' "$WINDOWS" \
  'Windows pointer normalization must use live virtual desktop bounds'
require 'map_pointer_point\(frame, target\)' "$WINDOWS" \
  'Windows input must consume the committed target-local coordinate mapping'
require 'windows_native_units' "$WINDOWS" \
  'Windows wheel injection must use the canonical bounded wheel translator'
require 'applied as usize == inputs\.len\(\)' "$WINDOWS" \
  'Windows input must reject partial SendInput application'
require 'windows_send_input_denied' "$WINDOWS" \
  'Windows UIPI/SendInput denial must have a stable reason'
require 'release_pointer_button' "$WINDOWS" \
  'Windows channel cleanup must release mouse buttons without target movement'
require 'release_key_frame' "$WINDOWS" \
  'Windows channel cleanup must release tracked keys'
require 'virtual_desktop_absolute_mapping_clamps_multi_monitor_coordinates' "$WINDOWS" \
  'Windows virtual-desktop normalization must retain an executable unit contract'
require 'windows_translation_preserves_high_resolution_delta_and_bounds_bursts' "$WHEEL" \
  'Windows wheel bounds must retain an executable unit contract'
reject 'SetCursorPos' "$WINDOWS" \
  'Windows pointer and button effects must use one auditable SendInput path'

require 'OnceLock<Mutex<Option<X11TargetInputExecutor>>>' "$LINUX" \
  'Linux X11 connection must be cached and serialized'
require 'Library::new' "$LINUX" \
  'Linux X11/XTest must be dynamically loaded without distro dev-package linkage'
require 'xtest::GetVersion' "$LINUX" \
  'Linux input must prove XTest availability before advertising runtime readiness'
require 'xtest::FakeInput' "$LINUX" \
  'Linux pointer and keyboard input must execute through typed XTest requests'
require 'X11ServerGrab::begin' "$LINUX" \
  'Linux target-local input must acquire one checked X11 server transaction'
require 'x::GrabServer' "$LINUX" \
  'Linux target-local input must use X11 GrabServer before final validation'
require 'x::UngrabServer' "$LINUX" \
  'Linux target-local input must release the X11 server on every path'
require 'res::QueryClientIds' "$LINUX" \
  'Linux target-local input must resolve authoritative X-Resource owner PIDs'
require 'LinuxProcessInstance::resolve' "$LINUX" \
  'Linux target-local input must reject PID reuse with a boot-scoped process identity'
require 'map_pointer_point\(frame, target\)' "$LINUX" \
  'Linux input must consume the committed target-local coordinate mapping'
require 'x11_detent_steps' "$LINUX" \
  'Linux wheel injection must use the canonical bounded wheel translator'
require 'WAYLAND_DISPLAY' "$LINUX" \
  'Linux backend must detect Wayland rather than silently injecting through XWayland'
require 'linux_wayland_portal_remote_desktop_not_implemented' "$LINUX" \
  'Linux pure/ambiguous Wayland input must remain explicitly fail-closed'
require 'linux_wheel_expansion_is_bounded' "$LINUX" \
  'Linux wheel expansion must retain an executable unit contract'
require 'linux_dom_key_mapping_is_deterministic' "$LINUX" \
  'Linux DOM-key translation must retain an executable unit contract'
require 'rejected_pointer_operations_are_resolved_before_native_injection' "$LINUX" \
  'Linux rejected pointer frames must retain a side-effect-free executable contract'
require 'fn barrier\(' "$LINUX" \
  'Linux applied input must wait for a checked X11 reply barrier'
require 'release_pointer_button' "$LINUX" \
  'Linux channel cleanup must release tracked XTest mouse buttons'
require 'release_key_frame' "$LINUX" \
  'Linux channel cleanup must release tracked XTest keys'
reject 'Command::new|xdotool|ydotool' "$LINUX" \
  'Linux input must not shell out to an ungoverned automation process'

require 'MAX_DETENTS_PER_FRAME' "$WHEEL" \
  'canonical wheel translation must retain a hard per-frame bound'
require 'x11_translation_emits_bounded_discrete_detents' "$WHEEL" \
  'Linux wheel bounds must retain an executable unit contract'

require 'Win32_UI_Input_KeyboardAndMouse' "$CARGO_MANIFEST" \
  'Cargo Windows features must include SendInput definitions'
require 'Win32_UI_WindowsAndMessaging' "$CARGO_MANIFEST" \
  'Cargo Windows features must include virtual desktop metrics'
require 'libloading = "0\.8"' "$CARGO_MANIFEST" \
  'Cargo Linux dependencies must declare the dynamic loader directly'
require 'xcb = \{ version = "1\.7", features = \["res", "xtest"\]' "$CARGO_MANIFEST" \
  'Cargo Linux dependencies must compile typed X-Resource and XTest protocols'
require 'platforms = \["macos", "linux", "windows"\]' "$MANIFEST" \
  'RemoteApp plugin manifest must install on Windows as well as macOS/Linux'

require 'windows_sendinput_target_guard_ready' "$VIEW_DEVICE" \
  'device capabilities must expose Windows guarded input baseline'
require 'linux_x11_xcb_atomic_display_global_ready' "$VIEW_DEVICE" \
  'device capabilities must expose Linux X11 display-global input readiness'
require 'linux_x11_xtest_cannot_isolate_press_release_to_target' "$VIEW_DEVICE" \
  'device capabilities must keep Linux Window/Application input view-only without target-bound press/release isolation'
require 'runtime_blocked_reason' "$VIEW_DEVICE" \
  'device capabilities must separate current-host runtime failure from implementation state'
require 'live_e2e_required' "$VIEW_DEVICE" \
  'device capabilities must retain the live OS-effect certification requirement'

printf 'check-remoteapp-platform-input-backends: ok\n'
