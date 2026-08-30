#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CHECK="$ROOT/tools/scripts/check-remoteapp-native-host-boundary.sh"

make_sandbox() {
  local sandbox
  sandbox="$(mktemp -d)"
  mkdir -p "$sandbox/plugins/remote-desktop/src" \
    "$sandbox/plugins/remote-desktop/src/media" \
    "$sandbox/plugins/remote-desktop/src/transport" \
    "$sandbox/plugins/remote-desktop/native-host/src" \
    "$sandbox/plugins/remote-desktop/native-protocol/src" \
    "$sandbox/plugins/remote-desktop/media-host/src" \
    "$sandbox/plugins/remote-desktop/media-host/tests" "$sandbox/tools/scripts"
  cp "$ROOT/plugins/remote-desktop/src/target_snapshot.rs" "$sandbox/plugins/remote-desktop/src/target_snapshot.rs"
  cp "$ROOT/plugins/remote-desktop/src/native_host_process.rs" "$sandbox/plugins/remote-desktop/src/native_host_process.rs"
  cp "$ROOT/plugins/remote-desktop/src/permissions.rs" "$sandbox/plugins/remote-desktop/src/permissions.rs"
  cp "$ROOT/plugins/remote-desktop/src/media_host_probe.rs" "$sandbox/plugins/remote-desktop/src/media_host_probe.rs"
  cp "$ROOT/plugins/remote-desktop/src/target.rs" "$sandbox/plugins/remote-desktop/src/target.rs"
  cp "$ROOT/plugins/remote-desktop/src/invoke_bidi.rs" "$sandbox/plugins/remote-desktop/src/invoke_bidi.rs"
  cp "$ROOT/plugins/remote-desktop/src/media/host_audio_capability.rs" "$sandbox/plugins/remote-desktop/src/media/host_audio_capability.rs"
  cp "$ROOT/plugins/remote-desktop/src/view_device.rs" "$sandbox/plugins/remote-desktop/src/view_device.rs"
  cp "$ROOT/plugins/remote-desktop/src/embedded.rs" "$sandbox/plugins/remote-desktop/src/embedded.rs"
  cp "$ROOT/plugins/remote-desktop/plugin.toml" "$sandbox/plugins/remote-desktop/plugin.toml"
  cp "$ROOT/plugins/remote-desktop/native-host/Cargo.toml" "$sandbox/plugins/remote-desktop/native-host/Cargo.toml"
  cp "$ROOT/plugins/remote-desktop/native-host/src/main.rs" "$sandbox/plugins/remote-desktop/native-host/src/main.rs"
  cp "$ROOT/plugins/remote-desktop/native-host/src/lib.rs" "$sandbox/plugins/remote-desktop/native-host/src/lib.rs"
  cp "$ROOT/plugins/remote-desktop/native-protocol/Cargo.toml" "$sandbox/plugins/remote-desktop/native-protocol/Cargo.toml"
  cp "$ROOT/plugins/remote-desktop/native-protocol/src/lib.rs" "$sandbox/plugins/remote-desktop/native-protocol/src/lib.rs"
  cp "$ROOT/plugins/remote-desktop/native-protocol/src/capture_probe.rs" "$sandbox/plugins/remote-desktop/native-protocol/src/capture_probe.rs"
  cp "$ROOT/plugins/remote-desktop/native-protocol/src/media_capabilities.rs" "$sandbox/plugins/remote-desktop/native-protocol/src/media_capabilities.rs"
  cp "$ROOT/plugins/remote-desktop/native-protocol/src/media_session.rs" "$sandbox/plugins/remote-desktop/native-protocol/src/media_session.rs"
  cp "$ROOT/plugins/remote-desktop/native-protocol/src/shared_media_lane.rs" "$sandbox/plugins/remote-desktop/native-protocol/src/shared_media_lane.rs"
  cp "$ROOT/plugins/remote-desktop/native-protocol/src/screen_capture_permission.rs" "$sandbox/plugins/remote-desktop/native-protocol/src/screen_capture_permission.rs"
  cp "$ROOT/plugins/remote-desktop/media-host/Cargo.toml" "$sandbox/plugins/remote-desktop/media-host/Cargo.toml"
  cp "$ROOT/plugins/remote-desktop/media-host/src/main.rs" "$sandbox/plugins/remote-desktop/media-host/src/main.rs"
  cp "$ROOT/plugins/remote-desktop/media-host/src/lib.rs" "$sandbox/plugins/remote-desktop/media-host/src/lib.rs"
  cp "$ROOT/plugins/remote-desktop/media-host/src/linux_x11.rs" "$sandbox/plugins/remote-desktop/media-host/src/linux_x11.rs"
  cp "$ROOT/plugins/remote-desktop/media-host/src/macos_sck.rs" "$sandbox/plugins/remote-desktop/media-host/src/macos_sck.rs"
  cp "$ROOT/plugins/remote-desktop/media-host/src/macos_multiapp.rs" "$sandbox/plugins/remote-desktop/media-host/src/macos_multiapp.rs"
  cp "$ROOT/plugins/remote-desktop/media-host/src/macos_videotoolbox.rs" "$sandbox/plugins/remote-desktop/media-host/src/macos_videotoolbox.rs"
  cp "$ROOT/plugins/remote-desktop/src/transport/mod.rs" "$sandbox/plugins/remote-desktop/src/transport/mod.rs"
  cp "$ROOT/plugins/remote-desktop/src/transport/webrtc_media.rs" "$sandbox/plugins/remote-desktop/src/transport/webrtc_media.rs"
  cp "$ROOT/plugins/remote-desktop/src/transport/webrtc_endpoint.rs" "$sandbox/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
  cp "$ROOT/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs" "$sandbox/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs"
  cp "$ROOT/tools/scripts/host-remoteapp-native-host-e2e.sh" "$sandbox/tools/scripts/host-remoteapp-native-host-e2e.sh"
  cp "$ROOT/tools/scripts/host-remoteapp-media-host-e2e.sh" "$sandbox/tools/scripts/host-remoteapp-media-host-e2e.sh"
  cp "$ROOT/plugins/remote-desktop/media-host/tests/linux_x11_process.rs" "$sandbox/plugins/remote-desktop/media-host/tests/linux_x11_process.rs"
  cp "$ROOT/plugins/remote-desktop/media-host/tests/process_lifecycle.rs" "$sandbox/plugins/remote-desktop/media-host/tests/process_lifecycle.rs"
  echo "$sandbox"
}

run_check() {
  CHECK_REMOTEAPP_NATIVE_HOST_ROOT="$1" \
    CHECK_REMOTEAPP_NATIVE_HOST_SKIP_CARGO_TREE=1 \
    bash "$CHECK" >/dev/null 2>&1
}

sandbox="$(make_sandbox)"
run_check "$sandbox"
rm -rf "$sandbox"

for mutation in daemon_mode one_lane optional_helper unbounded_frame no_reap root_dependency media_root_dependency media_without_nonce media_without_lane_sequence media_without_command_state media_not_wired media_without_recovery_drop media_untyped_failure media_application_proof_echo media_without_real_process media_without_invalidation media_without_parent_liveness media_false_audio_capability daemon_false_audio_capability media_permission_in_daemon media_without_capture_probe media_probe_not_wired mac_capture_not_hosted mac_encoder_not_hosted media_json_hot_lane media_without_shared_lane windows_not_hosted windows_payload_fallback windows_without_named_mapping windows_without_named_notification; do
  sandbox="$(make_sandbox)"
  case "$mutation" in
    daemon_mode)
      printf '\nconst EASYNET_INTERNAL_REMOTEAPP_CAPTURE_HELPER: &str = "1";\n' >>"$sandbox/plugins/remote-desktop/src/target_snapshot.rs"
      ;;
    one_lane)
      sed -i.bak '/guard: ProcessTargetSnapshotLane/d' "$sandbox/plugins/remote-desktop/src/target_snapshot.rs"
      ;;
    optional_helper)
      sed -i.bak 's/required = true/required = false/' "$sandbox/plugins/remote-desktop/plugin.toml"
      ;;
    unbounded_frame)
      sed -i.bak 's/4 \* 1024 \* 1024/usize::MAX/' "$sandbox/plugins/remote-desktop/native-protocol/src/lib.rs"
      ;;
    no_reap)
      sed -i.bak 's/self\.child\.wait()/self.child.try_wait()/' "$sandbox/plugins/remote-desktop/src/native_host_process.rs"
      ;;
    root_dependency)
      printf '\neasynet_cli = { package = "easynet", path = "../../.." }\n' >>"$sandbox/plugins/remote-desktop/native-host/Cargo.toml"
      ;;
    media_root_dependency)
      printf '\neasynet_cli = { package = "easynet", path = "../../.." }\n' >>"$sandbox/plugins/remote-desktop/media-host/Cargo.toml"
      ;;
    media_without_nonce)
      sed -i.bak 's/session_nonce/session_token/' "$sandbox/plugins/remote-desktop/native-protocol/src/media_session.rs"
      ;;
    media_without_lane_sequence)
      sed -i.bak 's/non-contiguous media sequence or regressing observation time/media sequence accepted/' "$sandbox/plugins/remote-desktop/native-protocol/src/media_session.rs"
      ;;
    media_without_command_state)
      sed -i.bak 's/pub struct MediaHostCommandValidator/pub struct MissingMediaHostCommandValidator/' "$sandbox/plugins/remote-desktop/native-protocol/src/media_session.rs"
      ;;
    media_not_wired)
      sed -i.bak 's/run_direct_webrtc_hosted_stream/run_unhosted_media/' "$sandbox/plugins/remote-desktop/src/transport/webrtc_media.rs"
      ;;
    media_without_recovery_drop)
      sed -i.bak 's/video_backpressure_drops_dependency_chain_until_recovery_idr/video_backpressure_restarts_generation/' "$sandbox/plugins/remote-desktop/src/native_host_process.rs"
      ;;
    media_untyped_failure)
      sed -i.bak 's/pub(super) struct HostedMediaHostFailure/struct GenericHostedFailure/' "$sandbox/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs"
      ;;
    media_application_proof_echo)
      sed -i.bak 's/actual_membership != application.window_ids/false/' "$sandbox/plugins/remote-desktop/media-host/src/linux_x11.rs"
      ;;
    media_without_real_process)
      sed -i.bak 's/real_x11_window_and_application_sessions_emit_recoverable_h264/media_host_smoke_placeholder/' "$sandbox/tools/scripts/host-remoteapp-media-host-e2e.sh"
      ;;
    media_without_invalidation)
      sed -i.bak 's/run_application_invalidation_session/run_application_without_invalidation/' "$sandbox/plugins/remote-desktop/media-host/tests/linux_x11_process.rs"
      ;;
    media_without_parent_liveness)
      sed -i.bak 's/parent_liveness_eof_kills_a_hung_session_generation/parent_liveness_is_not_observed/' "$sandbox/plugins/remote-desktop/media-host/tests/process_lifecycle.rs"
      ;;
    media_false_audio_capability)
      sed -i.bak 's/assert!(!response.capability.compiled_supported)/assert!(response.capability.compiled_supported)/' "$sandbox/plugins/remote-desktop/media-host/src/lib.rs"
      ;;
    daemon_false_audio_capability)
      sed -i.bak 's/cfg!(all(feature = "native-media", target_os = "macos"))/cfg!(all(feature = "native-media", target_os = "windows"))/' "$sandbox/plugins/remote-desktop/src/media/host_audio_capability.rs"
      ;;
    media_permission_in_daemon)
      sed -i.bak 's/execute_one_shot_native_host/execute_daemon_local_permission/' "$sandbox/plugins/remote-desktop/src/permissions.rs"
      ;;
    media_without_capture_probe)
      sed -i.bak 's/pub mod capture_probe;/mod removed_capture_probe;/' "$sandbox/plugins/remote-desktop/native-protocol/src/lib.rs"
      ;;
    media_probe_not_wired)
      sed -i.bak 's/media_host_probe::verify_binding/media_host_probe::verify_binding_removed/' "$sandbox/plugins/remote-desktop/src/target.rs"
      ;;
    mac_capture_not_hosted)
      sed -i.bak 's/mod macos_sck;/mod macos_sck_removed;/' "$sandbox/plugins/remote-desktop/media-host/src/lib.rs"
      ;;
    mac_encoder_not_hosted)
      sed -i.bak 's/mod macos_videotoolbox;/mod macos_videotoolbox_removed;/' "$sandbox/plugins/remote-desktop/media-host/src/lib.rs"
      ;;
    media_json_hot_lane)
      sed -i.bak 's/MediaLane::Video => write_video_event_frame/MediaLane::Video => write_control_event_frame/' "$sandbox/plugins/remote-desktop/native-protocol/src/media_session.rs"
      ;;
    media_without_shared_lane)
      sed -i.bak 's/let frame = Bytes::from_owner(lease);/let frame = Bytes::copy_from_slice(lease.as_ref());/' "$sandbox/plugins/remote-desktop/src/native_host_process.rs"
      ;;
    windows_not_hosted)
      sed -i.bak 's/any(target_os = "linux", target_os = "macos", target_os = "windows")/any(target_os = "linux", target_os = "macos")/' "$sandbox/plugins/remote-desktop/src/transport/mod.rs"
      ;;
    windows_payload_fallback)
      printf '\nuse crate::open_display_recorder_with_xcap;\n' >>"$sandbox/plugins/remote-desktop/src/transport/webrtc_media.rs"
      ;;
    windows_without_named_mapping)
      sed -i.bak 's/CreateFileMappingW/CreateRetiredPayloadPipeW/g' "$sandbox/plugins/remote-desktop/native-protocol/src/shared_media_lane.rs"
      ;;
    windows_without_named_notification)
      sed -i.bak 's/CreateNamedPipeW/CreateRetiredPayloadPipeW/g' "$sandbox/plugins/remote-desktop/native-protocol/src/shared_media_lane.rs"
      ;;
  esac
  if run_check "$sandbox"; then
    rm -rf "$sandbox"
    echo "[FAIL] mutation unexpectedly passed: $mutation" >&2
    exit 1
  fi
  rm -rf "$sandbox"
done

echo "test_check_remoteapp_native_host_boundary: all cases passed"
