#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_MAIN_CRATE_TEST_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
REMOTE_ROOT="$ROOT/plugins/remote-desktop/src"
STANDALONE_LIB="$REMOTE_ROOT/lib.rs"
EMBEDDED_IMPL="$REMOTE_ROOT/embedded.rs"

fail() {
  printf 'check-remoteapp-main-crate-implementation-tests: %s\n' "$1" >&2
  exit 1
}

require() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  rg -q -- "$pattern" "$path" || fail "$message"
}

[[ -d "$REMOTE_ROOT" ]] || fail "missing remote desktop plugin source root"
[[ -f "$STANDALONE_LIB" ]] || fail "missing remote desktop standalone plugin lib"
[[ -f "$EMBEDDED_IMPL" ]] || fail "missing daemon-embedded remote desktop implementation root"

# The published plugin crate is intentionally a manifest/provider shim. The
# implementation and implementation tests are compiled through the main
# EasyNet crate where `embedded.rs` is mounted under
# `crate::daemon::plugins::remote_desktop`.
require 'pub use easynet_cli::daemon::plugins::remote_desktop::provider' "$STANDALONE_LIB" \
  "standalone remote-desktop crate must remain a provider shim; do not treat its 0-test result as implementation evidence"
require 'pub\(crate\) mod target_observer;' "$EMBEDDED_IMPL" \
  "daemon-embedded remote desktop implementation must own target_observer tests"
require 'pub\(crate\) mod input;' "$EMBEDDED_IMPL" \
  "daemon-embedded remote desktop implementation must own input tests"
require 'pub\(crate\) mod media;' "$EMBEDDED_IMPL" \
  "daemon-embedded remote desktop implementation must own media tests"

run_main_crate_test() {
  local filter="$1"
  local output
  output="$(mktemp "${TMPDIR:-/tmp}/remoteapp-main-crate-test.XXXXXX")"
  if ! (
    cd "$ROOT"
    cargo test --features axon-pb "$filter" --lib -- --nocapture
  ) >"$output" 2>&1; then
    sed -n '1,220p' "$output" >&2
    rm -f "$output"
    fail "main-crate implementation test failed: $filter"
  fi
  if ! rg -q 'running [1-9][0-9]* tests?' "$output"; then
    sed -n '1,220p' "$output" >&2
    rm -f "$output"
    fail "main-crate implementation test filter matched zero tests: $filter"
  fi
  if ! rg -q 'test result: ok\.' "$output"; then
    sed -n '1,220p' "$output" >&2
    rm -f "$output"
    fail "main-crate implementation test did not report ok: $filter"
  fi
  rm -f "$output"
}

run_main_crate_test 'application_observer_reports_committed_window_set_drift_as_rebind'
run_main_crate_test 'snapshot_observer_reappearance_requires_explicit_rebind_policy'
run_main_crate_test 'unsupported_platform_observer_fails_app_window_targets_closed'
run_main_crate_test 'direct_webrtc_binding_never_uses_xcap_fallback_for_window_or_application'
run_main_crate_test 'catalog_declares_native_plugin_state_per_platform'
run_main_crate_test 'device_capabilities_project_native_target_subject_matrix'
run_main_crate_test 'device_capabilities_project_cross_platform_support_matrix'
run_main_crate_test 'device_capabilities_project_input_control_support_matrix'
run_main_crate_test 'device_capabilities_project_media_pipeline_support_matrix'
run_main_crate_test 'current_session_input_policy_reapplies_session_input_scope_to_latest_snapshot'
