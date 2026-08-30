#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-platform-input-backends.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

write_fixture() {
  rm -rf "$SANDBOX/plugins" "$SANDBOX/Cargo.toml"
  mkdir -p \
    "$SANDBOX/plugins/remote-desktop/src/input" \
    "$SANDBOX/plugins/remote-desktop/native-platform/src"
  cp "$REPO_ROOT/Cargo.toml" "$SANDBOX/Cargo.toml"
  cp "$REPO_ROOT/plugins/remote-desktop/plugin.toml" "$SANDBOX/plugins/remote-desktop/plugin.toml"
  cp "$REPO_ROOT/plugins/remote-desktop/src/input.rs" "$SANDBOX/plugins/remote-desktop/src/input.rs"
  cp "$REPO_ROOT/plugins/remote-desktop/src/input/windows.rs" "$SANDBOX/plugins/remote-desktop/src/input/windows.rs"
  cp "$REPO_ROOT/plugins/remote-desktop/src/input/linux.rs" "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
  cp "$REPO_ROOT/plugins/remote-desktop/native-platform/src/lib.rs" "$SANDBOX/plugins/remote-desktop/native-platform/src/lib.rs"
  cp "$REPO_ROOT/plugins/remote-desktop/src/input/wheel.rs" "$SANDBOX/plugins/remote-desktop/src/input/wheel.rs"
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
perl -0pi -e 's/windows_native_units/windows_unbounded_units/g' "$SANDBOX/plugins/remote-desktop/src/input/windows.rs"
run_fail 'Windows wheel injection must use the canonical bounded wheel translator'

write_fixture
perl -0pi -e 's/virtual_desktop_absolute_mapping_clamps_multi_monitor_coordinates/virtual_desktop_mapping_unchecked/g' "$SANDBOX/plugins/remote-desktop/src/input/windows.rs"
run_fail 'Windows virtual-desktop normalization must retain an executable unit contract'

write_fixture
perl -0pi -e 's/xtest::GetVersion/xtest::AssumeAvailable/g' "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
run_fail 'Linux input must prove XTest availability before advertising runtime readiness'

write_fixture
perl -0pi -e 's/x11_detent_steps/x11_unbounded_steps/g' "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
run_fail 'Linux wheel injection must use the canonical bounded wheel translator'

write_fixture
perl -0pi -e 's/linux_dom_key_mapping_is_deterministic/linux_dom_key_mapping_unverified/g' "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
run_fail 'Linux DOM-key translation must retain an executable unit contract'

write_fixture
perl -0pi -e 's/rejected_pointer_operations_are_resolved_before_native_injection/rejected_pointer_operations_may_move_cursor/g' "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
run_fail 'Linux rejected pointer frames must retain a side-effect-free executable contract'

write_fixture
perl -0pi -e 's/fn barrier\(/fn unchecked_flush\(/g' "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
run_fail 'Linux applied input must wait for a checked X11 reply barrier'

write_fixture
perl -0pi -e 's/X11ServerGrab::begin/X11ServerGrab::assume/g' "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
run_fail 'Linux target-local input must acquire one checked X11 server transaction'

write_fixture
perl -0pi -e 's/ProcessInstance::resolve/ProcessInstance::from_pid_only/g' "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
run_fail 'Linux target-local input must consume the canonical boot-scoped process identity'

write_fixture
perl -0pi -e 's/res::QueryClientIds/res::QueryClientsWithoutIdentity/g' "$SANDBOX/plugins/remote-desktop/native-platform/src/lib.rs"
run_fail 'native platform authority must resolve authoritative X-Resource owner PIDs'

write_fixture
perl -0pi -e 's/WAYLAND_DISPLAY/WAYLAND_IGNORED/g' "$SANDBOX/plugins/remote-desktop/src/input/linux.rs"
run_fail 'Linux backend must detect Wayland rather than silently injecting through XWayland'

write_fixture
perl -0pi -e 's/"macos", "linux", "windows"/"macos", "linux"/' "$SANDBOX/plugins/remote-desktop/plugin.toml"
run_fail 'RemoteApp plugin manifest must install on Windows as well as macOS/Linux'

write_fixture
perl -0pi -e 's/live_e2e_required/source_complete/g' "$SANDBOX/plugins/remote-desktop/src/view_device.rs"
run_fail 'device capabilities must retain the live OS-effect certification requirement'

printf 'test_check_remoteapp_platform_input_backends: all cases passed\n'
