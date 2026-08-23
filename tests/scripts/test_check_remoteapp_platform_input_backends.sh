#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-platform-input-backends.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

write_fixture() {
  rm -rf "$SANDBOX/plugins" "$SANDBOX/Cargo.toml"
  mkdir -p "$SANDBOX/plugins/remote-desktop/src/input"
  cp "$REPO_ROOT/Cargo.toml" "$SANDBOX/Cargo.toml"
  cp "$REPO_ROOT/plugins/remote-desktop/plugin.toml" "$SANDBOX/plugins/remote-desktop/plugin.toml"
  cp "$REPO_ROOT/plugins/remote-desktop/src/input.rs" "$SANDBOX/plugins/remote-desktop/src/input.rs"
  cp "$REPO_ROOT/plugins/remote-desktop/src/input/windows.rs" "$SANDBOX/plugins/remote-desktop/src/input/windows.rs"
  cp "$REPO_ROOT/plugins/remote-desktop/src/input/linux.rs" "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
  cp "$REPO_ROOT/plugins/remote-desktop/src/view_device.rs" "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
}

run_ok() {
  CHECK_REMOTEAPP_PLATFORM_INPUT_ROOT="$SANDBOX" "$SCRIPT" >/dev/null
}

run_fail() {
  local expected="$1"
  local output
  if output="$(CHECK_REMOTEAPP_PLATFORM_INPUT_ROOT="$SANDBOX" "$SCRIPT" 2>&1)"; then
    printf 'expected failure containing %s\n' "$expected" >&2
    exit 1
  fi
  [[ "$output" == *"$expected"* ]] || {
    printf 'expected failure containing %s, got:\n%s\n' "$expected" "$output" >&2
    exit 1
  }
}

write_fixture
run_ok

write_fixture
perl -0pi -e 's/SendInput/LegacyInput/g' "$SANDBOX/plugins/remote-desktop/src/input/windows.rs"
run_fail 'Windows input must execute through User32 SendInput'

write_fixture
perl -0pi -e 's/MOUSEEVENTF_VIRTUALDESK/MOUSEEVENTF_ABSOLUTE_ONLY/g' "$SANDBOX/plugins/remote-desktop/src/input/windows.rs"
run_fail 'Windows absolute pointer mapping must cover the virtual desktop'

write_fixture
perl -0pi -e 's/MAX_WHEEL_DELTA_PER_FRAME/UNBOUNDED_WHEEL_DELTA/g' "$SANDBOX/plugins/remote-desktop/src/input/windows.rs"
run_fail 'Windows wheel injection must be bounded per input frame'

write_fixture
perl -0pi -e 's/virtual_desktop_absolute_mapping_clamps_multi_monitor_coordinates/virtual_desktop_mapping_unchecked/g' "$SANDBOX/plugins/remote-desktop/src/input/windows.rs"
run_fail 'Windows virtual-desktop normalization must retain an executable unit contract'

write_fixture
perl -0pi -e 's/XTestQueryExtension/XTestAssumeAvailable/g' "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
run_fail 'Linux input must prove XTest availability before advertising runtime readiness'

write_fixture
perl -0pi -e 's/MAX_WHEEL_STEPS_PER_AXIS/UNBOUNDED_WHEEL_STEPS/g' "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
run_fail 'Linux wheel injection expansion must be explicitly bounded'

write_fixture
perl -0pi -e 's/linux_dom_key_mapping_is_deterministic/linux_dom_key_mapping_unverified/g' "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
run_fail 'Linux DOM-key translation must retain an executable unit contract'

write_fixture
perl -0pi -e 's/WAYLAND_DISPLAY/WAYLAND_IGNORED/g' "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
run_fail 'Linux backend must detect Wayland rather than silently injecting through XWayland'

write_fixture
perl -0pi -e 's/"macos", "linux", "windows"/"macos", "linux"/' "$SANDBOX/plugins/remote-desktop/plugin.toml"
run_fail 'RemoteApp plugin manifest must install on Windows as well as macOS/Linux'

write_fixture
perl -0pi -e 's/live_e2e_required/source_complete/' "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must retain the live OS-effect certification requirement'

printf 'test_check_remoteapp_platform_input_backends: all cases passed\n'
