#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_MAIN_CRATE_TEST_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
REMOTE_ROOT="$ROOT/plugins/remote-desktop/src"
STANDALONE_LIB="$REMOTE_ROOT/lib.rs"
EMBEDDED_IMPL="$REMOTE_ROOT/embedded.rs"
HOST_AUDIO="$REMOTE_ROOT/media/host_audio.rs"
HOST_AUDIO_CAPABILITY="$REMOTE_ROOT/media/host_audio_capability.rs"
LINUX_PROCESS_TREE_AUDIO="$REMOTE_ROOT/media/linux_process_tree_audio.rs"
VIEW_DEVICE="$REMOTE_ROOT/view_device.rs"
WEBRTC_ENDPOINT="$REMOTE_ROOT/transport/webrtc_endpoint.rs"
WEBRTC_AUDIO="$REMOTE_ROOT/transport/webrtc_audio.rs"
WEBRTC_BASELINE_MEDIA="$REMOTE_ROOT/transport/webrtc_baseline_media.rs"
WEBRTC_HOSTED_MEDIA="$REMOTE_ROOT/transport/webrtc_hosted_media.rs"
OBSOLETE_WEBRTC_NATIVE_MEDIA="$REMOTE_ROOT/transport/webrtc_native_media.rs"

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

require_multiline() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  perl -0ne "exit(($pattern) ? 0 : 1)" "$path" || fail "$message"
}

reject() {
  local pattern="$1"
  local path="$2"
  local message="$3"
  if rg -q -- "$pattern" "$path"; then
    fail "$message"
  fi
}

[[ -d "$REMOTE_ROOT" ]] || fail "missing remote desktop plugin source root"
[[ -f "$STANDALONE_LIB" ]] || fail "missing remote desktop standalone plugin lib"
[[ -f "$EMBEDDED_IMPL" ]] || fail "missing daemon-embedded remote desktop implementation root"
for source in "$HOST_AUDIO" "$HOST_AUDIO_CAPABILITY" "$LINUX_PROCESS_TREE_AUDIO" "$VIEW_DEVICE" "$WEBRTC_ENDPOINT" "$WEBRTC_AUDIO" "$WEBRTC_BASELINE_MEDIA" "$WEBRTC_HOSTED_MEDIA"; do
  [[ -f "$source" ]] || fail "missing RemoteApp implementation source ${source#"$ROOT/"}"
done
[[ ! -e "$OBSOLETE_WEBRTC_NATIVE_MEDIA" ]] || \
  fail "obsolete daemon-local WebRTC native media implementation remains: ${OBSOLETE_WEBRTC_NATIVE_MEDIA#"$ROOT/"}"

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

# Target-scoped host audio is one admitted media contract. Capability
# projection, SDP setup and the live backend must consume the same source plan;
# unsupported target audio fails closed before answer and a terminal failure of
# an accepted audio track fails the current transport generation.
require 'struct HostAudioSourcePlan' "$HOST_AUDIO" \
  "host audio must have one typed target-source admission plan"
require 'LinuxProcessTreeAudioBackend' "$LINUX_PROCESS_TREE_AUDIO" \
  "Linux target audio must fan in through a dedicated process-tree backend"
require 'current_process_tree' "$LINUX_PROCESS_TREE_AUDIO" \
  "Linux target audio must derive the current descendant PID set"
require 'retain\(\|node_id, _\| eligible_nodes\.contains\(node_id\)\)' "$LINUX_PROCESS_TREE_AUDIO" \
  "Linux target audio must unlink output nodes that leave the authorized process tree"
require 'add_timer' "$LINUX_PROCESS_TREE_AUDIO" \
  "Linux target audio must revalidate process authority independently of PipeWire graph churn"
require 'empty_authority_set_revokes_all_previously_eligible_nodes' "$LINUX_PROCESS_TREE_AUDIO" \
  "Linux target audio must prove that loss of process authority revokes every node"
require 'contradictory_node_and_client_pid_identity_fails_closed_in_both_directions' "$LINUX_PROCESS_TREE_AUDIO" \
  "Linux target audio must fail closed when PipeWire node and owning-client identities disagree"
require 'HostAudioSourcePlan::for_target\(' "$VIEW_DEVICE" \
  "device capability projection tests must consume the canonical target-scoped host-audio source plan"
require 'struct HostAudioRuntimeProbe' "$HOST_AUDIO_CAPABILITY" \
  "host audio runtime availability must be owned by one plugin-scoped capability monitor"
require 'expires_at_monotonic' "$HOST_AUDIO_CAPABILITY" \
  "host audio offer admission must use monotonic snapshot freshness"
require 'refresh_requested: bool' "$HOST_AUDIO_CAPABILITY" \
  "host audio refresh requests must coalesce into one fixed-state bit"
require 'mpsc::sync_channel\(1\)' "$HOST_AUDIO_CAPABILITY" \
  "host audio worker wake queue must have capacity one"
reject 'mpsc::channel\(\)' "$HOST_AUDIO_CAPABILITY" \
  "host audio capability monitor must not use an unbounded command queue"
require_multiline 'm/Err\(error\)\s*=>\s*\{.+?"offer_ready".+?false.+?"capture_ready".+?false.+?"send_ready".+?false.+?"target_admissible".+?false/s' "$VIEW_DEVICE" \
  "blocked target audio must keep platform support distinct while closing offer, capture, send and target admission"
reject 'support\["supported"\] = json!\(false\)' "$VIEW_DEVICE" \
  "a target-specific admission failure must not erase compiled platform support"
require 'admit_host_audio_offer\(' "$WEBRTC_ENDPOINT" \
  "audio-video offer admission must validate the cached runtime snapshot and exact target source before answer creation"
require 'fn terminal_failure\(&self\)' "$WEBRTC_AUDIO" \
  "negotiated audio must expose one typed terminal-failure boundary"
require 'negotiated host-audio pipeline failed' "$WEBRTC_BASELINE_MEDIA" \
  "Windows/Linux media loops must propagate negotiated host-audio terminal failure"
require 'audio\.enqueue\(event\.payload, duration_samples\)\?' "$WEBRTC_HOSTED_MEDIA" \
  "hosted media must propagate an accepted Opus packet into the bounded WebRTC audio writer"
require 'audio\.ensure_healthy\(\)\?' "$WEBRTC_HOSTED_MEDIA" \
  "hosted media must fail the current transport generation when the negotiated audio writer fails"

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
run_main_crate_test 'process_scoped_application_observer_tracks_window_set_without_display_identity'
run_main_crate_test 'snapshot_observer_reappearance_requires_explicit_rebind_policy'
run_main_crate_test 'unsupported_platform_observer_fails_app_window_targets_closed'
run_main_crate_test 'windows_xcap_application_binding_is_process_scoped_without_fake_display'
run_main_crate_test 'application_compositor'
run_main_crate_test 'direct_webrtc_binding_uses_xcap_without_widening_window_or_application_scope'
run_main_crate_test 'catalog_declares_native_plugin_state_per_platform'
run_main_crate_test 'application_recovery_accepts_unknown_platform_display_and_rejects_contradictions'
run_main_crate_test 'device_capabilities_project_native_target_subject_matrix'
run_main_crate_test 'device_capabilities_project_cross_platform_support_matrix'
run_main_crate_test 'device_capabilities_project_input_control_support_matrix'
run_main_crate_test 'device_capabilities_project_media_pipeline_support_matrix'
run_main_crate_test 'target_audio_capability_uses_exact_source_admission_and_fails_closed'
run_main_crate_test 'expired_ready_snapshot_fails_audio_offer_admission_closed'
run_main_crate_test 'unreachable_runtime_fails_audio_offer_admission_closed'
run_main_crate_test 'plugin_owned_worker_prewarms_and_refreshes_one_generation_at_a_time'
run_main_crate_test 'invalidate_is_synchronous_source_scoped_and_obsoletes_inflight_success'
run_main_crate_test 'blocked_native_probe_does_not_block_supervisor_shutdown'
run_main_crate_test 'render_probe_requires_exact_active_session_binding_tuple'
run_main_crate_test 'render_probe_rejects_replay_and_counter_regression'
run_main_crate_test 'authored_descriptor_and_runtime_schema_are_identical'
run_main_crate_test 'process_tree_includes_all_descendants_and_excludes_unrelated_processes'
run_main_crate_test 'audio_node_selection_includes_every_node_in_the_process_tree'
run_main_crate_test 'reused_root_pid_fails_closed_instead_of_selecting_new_process_tree'
run_main_crate_test 'empty_authority_set_revokes_all_previously_eligible_nodes'
run_main_crate_test 'contradictory_node_and_client_pid_identity_fails_closed_in_both_directions'
run_main_crate_test 'negotiated_audio_blocker_is_terminal_but_not_negotiated_is_not'
run_main_crate_test 'negotiated_audio_never_degrades_to_not_negotiated_when_backend_is_unavailable'
run_main_crate_test 'current_session_input_policy_reapplies_session_input_scope_to_latest_snapshot'
