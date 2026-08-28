#!/usr/bin/env bash
# Static architecture gate for the Remote Desktop plugin-private native host.

set -euo pipefail

ROOT="${CHECK_REMOTEAPP_NATIVE_HOST_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
SNAPSHOT="$ROOT/plugins/remote-desktop/src/target_snapshot.rs"
PROCESS="$ROOT/plugins/remote-desktop/src/native_host_process.rs"
PERMISSIONS="$ROOT/plugins/remote-desktop/src/permissions.rs"
MEDIA_HOST_PROBE="$ROOT/plugins/remote-desktop/src/media_host_probe.rs"
TARGET="$ROOT/plugins/remote-desktop/src/target.rs"
INVOKE_BIDI="$ROOT/plugins/remote-desktop/src/invoke_bidi.rs"
HOST_AUDIO="$ROOT/plugins/remote-desktop/src/media/host_audio_capability.rs"
VIEW_DEVICE="$ROOT/plugins/remote-desktop/src/view_device.rs"
EMBEDDED="$ROOT/plugins/remote-desktop/src/embedded.rs"
MANIFEST="$ROOT/plugins/remote-desktop/plugin.toml"
HOST_CARGO="$ROOT/plugins/remote-desktop/native-host/Cargo.toml"
HOST_MAIN="$ROOT/plugins/remote-desktop/native-host/src/main.rs"
HOST_LIB="$ROOT/plugins/remote-desktop/native-host/src/lib.rs"
PROTOCOL_CARGO="$ROOT/plugins/remote-desktop/native-protocol/Cargo.toml"
PROTOCOL_LIB="$ROOT/plugins/remote-desktop/native-protocol/src/lib.rs"
CAPTURE_PROBE_PROTOCOL="$ROOT/plugins/remote-desktop/native-protocol/src/capture_probe.rs"
MEDIA_PROTOCOL="$ROOT/plugins/remote-desktop/native-protocol/src/media_capabilities.rs"
MEDIA_SESSION_PROTOCOL="$ROOT/plugins/remote-desktop/native-protocol/src/media_session.rs"
SHARED_MEDIA_PROTOCOL="$ROOT/plugins/remote-desktop/native-protocol/src/shared_media_lane.rs"
SCREEN_PERMISSION_PROTOCOL="$ROOT/plugins/remote-desktop/native-protocol/src/screen_capture_permission.rs"
MEDIA_HOST_CARGO="$ROOT/plugins/remote-desktop/media-host/Cargo.toml"
MEDIA_HOST_MAIN="$ROOT/plugins/remote-desktop/media-host/src/main.rs"
MEDIA_HOST_LIB="$ROOT/plugins/remote-desktop/media-host/src/lib.rs"
MEDIA_HOST_LINUX="$ROOT/plugins/remote-desktop/media-host/src/linux_x11.rs"
MEDIA_HOST_MAC="$ROOT/plugins/remote-desktop/media-host/src/macos_sck.rs"
MEDIA_HOST_MAC_MULTIAPP="$ROOT/plugins/remote-desktop/media-host/src/macos_multiapp.rs"
MEDIA_HOST_MAC_ENCODER="$ROOT/plugins/remote-desktop/media-host/src/macos_videotoolbox.rs"
WEBRTC_MEDIA="$ROOT/plugins/remote-desktop/src/transport/webrtc_media.rs"
WEBRTC_ENDPOINT="$ROOT/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
WEBRTC_HOSTED_MEDIA="$ROOT/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs"
TRANSPORT_MOD="$ROOT/plugins/remote-desktop/src/transport/mod.rs"
LIVE="$ROOT/tools/scripts/host-remoteapp-native-host-e2e.sh"
MEDIA_LIVE="$ROOT/tools/scripts/host-remoteapp-media-host-e2e.sh"
MEDIA_LIVE_TEST="$ROOT/plugins/remote-desktop/media-host/tests/linux_x11_process.rs"
MEDIA_PROCESS_LIFECYCLE_TEST="$ROOT/plugins/remote-desktop/media-host/tests/process_lifecycle.rs"

fail() { echo "[FAIL] $*" >&2; exit 1; }
require_file() { [[ -f "$1" ]] || fail "missing required file: $1"; }
require() {
  local literal="$1" file="$2" reason="$3"
  grep -Fq -- "$literal" "$file" || fail "$reason"
}
forbid() {
  local pattern="$1" file="$2" reason="$3"
  if grep -Eq -- "$pattern" "$file"; then fail "$reason"; fi
}

for file in "$SNAPSHOT" "$PROCESS" "$PERMISSIONS" "$MEDIA_HOST_PROBE" "$TARGET" "$INVOKE_BIDI" "$HOST_AUDIO" "$VIEW_DEVICE" "$EMBEDDED" "$MANIFEST" "$HOST_CARGO" "$HOST_MAIN" "$HOST_LIB" "$PROTOCOL_CARGO" "$PROTOCOL_LIB" "$CAPTURE_PROBE_PROTOCOL" "$MEDIA_PROTOCOL" "$MEDIA_SESSION_PROTOCOL" "$SHARED_MEDIA_PROTOCOL" "$SCREEN_PERMISSION_PROTOCOL" "$MEDIA_HOST_CARGO" "$MEDIA_HOST_MAIN" "$MEDIA_HOST_LIB" "$MEDIA_HOST_LINUX" "$MEDIA_HOST_MAC" "$MEDIA_HOST_MAC_MULTIAPP" "$MEDIA_HOST_MAC_ENCODER" "$WEBRTC_MEDIA" "$WEBRTC_ENDPOINT" "$WEBRTC_HOSTED_MEDIA" "$TRANSPORT_MOD" "$LIVE" "$MEDIA_LIVE" "$MEDIA_LIVE_TEST" "$MEDIA_PROCESS_LIFECYCLE_TEST"; do
  require_file "$file"
done

for obsolete in \
  "$ROOT/plugins/remote-desktop/src/screencapturekit_capture.rs" \
  "$ROOT/plugins/remote-desktop/src/screencapturekit_multiapp.rs" \
  "$ROOT/plugins/remote-desktop/src/screencapturekit_audio.rs" \
  "$ROOT/plugins/remote-desktop/src/videotoolbox_encoder.rs" \
  "$ROOT/plugins/remote-desktop/src/media/native.rs" \
  "$ROOT/plugins/remote-desktop/src/transport/webrtc_native_media.rs"; do
  [[ ! -e "$obsolete" ]] || fail "obsolete daemon-local native media implementation remains: $obsolete"
done

require 'inventory: ProcessTargetSnapshotLane' "$SNAPSHOT" \
  'inventory sampling must own an independent killable helper lane'
require 'guard: ProcessTargetSnapshotLane' "$SNAPSHOT" \
  'input/focus guards must not queue behind inventory sampling'
require 'pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;' "$PROTOCOL_LIB" \
  'native-host frames must have a four MiB pre-allocation limit'
require 'const SNAPSHOT_MAILBOX_CAPACITY: usize = 8;' "$SNAPSHOT" \
  'native-host request admission must be bounded'
require 'protocol: PROTOCOL.to_string()' "$PROTOCOL_LIB" \
  'every helper frame must carry the declared private protocol identity'
require 'process_generation' "$SNAPSHOT" \
  'helper responses must be fenced by process generation'
require '.env_clear()' "$PROCESS" \
  'native host must not inherit daemon credentials or ambient environment'
require '"SystemRoot", "WINDIR", "TEMP", "TMP"' "$PROCESS" \
  'Windows native host must receive the explicit minimal OS bootstrap environment'
require 'self.child.kill()' "$PROCESS" \
  'failed native-host generations must be killable'
require 'self.child.wait()' "$PROCESS" \
  'killed native-host generations must be reaped'
require 'libc::PR_SET_PDEATHSIG' "$PROCESS" \
  'Linux native host must die with its daemon parent'
require 'JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE' "$PROCESS" \
  'Windows native host must be assigned to a kill-on-close Job Object'
require 'pub const PARENT_LIVENESS_FD_ENV: &str = "EASYNET_REMOTEAPP_PARENT_LIVENESS_FD";' "$PROTOCOL_LIB" \
  'private protocol must own the Unix parent-liveness environment identity'
require 'start_parent_liveness_watchdog()' "$HOST_LIB" \
  'Unix native host must retain an explicit parent-liveness watchdog'
require 'hung_native_generation_is_killed_reaped_and_replaced' "$SNAPSHOT" \
  'fault-injected regression must prove deadline kill/reap and generation replacement'
require 'repeated_native_hangs_are_killed_and_reaped_before_restart' "$SNAPSHOT" \
  'fault-injected regression must cover repeated hung generations'
require 'unsolicited_response_marks_protocol_violation_without_blocking_reader' "$PROCESS" \
  'unsolicited helper responses must fail the protocol without blocking the reader'
require 'TrySendError::Full' "$PROCESS" \
  'helper response delivery must remain bounded and non-blocking'
require 'terminate_spawn_failure(&mut child' "$PROCESS" \
  'partially started helper processes must be transactionally killed and reaped'
require 'process_executor_round_trips_real_sibling_helper_and_reaps_on_drop' "$SNAPSHOT" \
  'a real sibling-process regression must cover the parent supervisor path'
forbid 'Command::new\(std::env::current_exe' "$SNAPSHOT" \
  'native host must never fall back to running the daemon executable'
forbid 'EASYNET_INTERNAL_REMOTEAPP_CAPTURE_HELPER' "$SNAPSHOT" \
  'native host must not be hidden behind a daemon environment mode'
forbid 'HOME|EASYNET_(CREDENTIAL|TOKEN|KEY|SESSION)' "$PROCESS" \
  'native host environment projection must not include identity/session secrets'

require 'pub(super) const NATIVE_HOST_EXECUTABLE: &str = "easynet-remoteapp-native-host";' "$EMBEDDED" \
  'the plugin must own one canonical native-host artifact name'
require '[[runtime_helper]]' "$MANIFEST" \
  'the private helper must be explicit plugin manifest state'
require 'name = "native_target_observation"' "$MANIFEST" \
  'helper manifest semantics must describe the implemented observation scope'
require 'protocol = "remoteapp_native_host_v1"' "$MANIFEST" \
  'plugin manifest and helper wire protocol must share one version identity'
require 'isolation = "per_lane"' "$MANIFEST" \
  'plugin manifest must disclose the helper failure-domain shape'
require 'required = true' "$MANIFEST" \
  'there must be no in-process production fallback when the helper is absent'
require 'name = "easynet-remoteapp-native-host"' "$HOST_CARGO" \
  'RemoteApp native host must be a separately built plugin-private executable'
forbid '"axon-pb"' "$HOST_CARGO" \
  'the native host must not enable the Runtime Axon protocol surface'
forbid 'easynet_cli|package[[:space:]]*=[[:space:]]*"easynet"|path[[:space:]]*=[[:space:]]*"\.\./\.\./\.\."' "$HOST_CARGO" \
  'native host must not depend on the root Runtime crate'
require 'easynet-remoteapp-native-protocol = { path = "../native-protocol" }' "$HOST_CARGO" \
  'native host and daemon client must share the plugin-private protocol crate'
require 'easynet_remoteapp_native_host::run()' "$HOST_MAIN" \
  'native-host main must remain a thin private execution entrypoint'
forbid '^use .*daemon::(invocation|identity|persistence)|Axon|tonic::' "$HOST_MAIN" \
  'native-host entrypoint must not import Runtime/authority infrastructure'
forbid 'easynet_cli::|axon_sdk::|tonic::' "$HOST_LIB" \
  'native-host implementation must remain independent from Runtime and Axon crates'
forbid 'run_native_host' "$EMBEDDED" \
  'daemon plugin module must not export the native-host server implementation'
require 'pub struct Request' "$PROTOCOL_LIB" \
  'private protocol crate must own the request DTO'
require 'pub struct Response' "$PROTOCOL_LIB" \
  'private protocol crate must own the response DTO'
require 'pub mod capture_probe;' "$PROTOCOL_LIB" \
  'private protocol crate must expose the canonical one-shot capture probe'
forbid 'axon|tonic|easynet_cli' "$PROTOCOL_CARGO" \
  'private protocol crate must remain product-runtime independent'
require 'parent-liveness' "$LIVE" \
  'real-process smoke must prove parent-death termination'
require 'oversized frame' "$LIVE" \
  'real-process smoke must prove oversized frame rejection'

require 'name = "native_media"' "$MANIFEST" \
  'native media must be an explicit private helper'
require 'protocol = "remoteapp_media_host_v1"' "$MANIFEST" \
  'media-host manifest and schema versions must agree'
require 'lifecycle = "per_generation"' "$MANIFEST" \
  'active media must have one process per immutable generation'
require 'isolation = "control_video_audio_lanes"' "$MANIFEST" \
  'media-host manifest must disclose its physical lane boundary'
require 'pub(super) const MEDIA_HOST_EXECUTABLE' "$EMBEDDED" \
  'the plugin must own one canonical media-host artifact name'
require 'name = "easynet-remoteapp-media-host"' "$MEDIA_HOST_CARGO" \
  'native media must be a separately built executable'
require 'easynet_remoteapp_media_host::run()' "$MEDIA_HOST_MAIN" \
  'media-host main must remain a thin private entrypoint'
require 'pub const PROTOCOL: &str = crate::media_session::PROTOCOL;' "$MEDIA_PROTOCOL" \
  'capability mode and active sessions must share one media-host protocol identity'
require 'pub const PROTOCOL: &str = "remoteapp_media_host_capture_probe_v1";' "$CAPTURE_PROBE_PROTOCOL" \
  'exact target verification and diagnostic capture must use a versioned private contract'
require 'pub const MAX_DIAGNOSTIC_DIMENSION: u32 = u16::MAX as u32;' "$CAPTURE_PROBE_PROTOCOL" \
  'diagnostic capture dimensions must fit the concrete JPEG encoder ABI'
require 'InitialFrame::CaptureProbe(request)' "$MEDIA_HOST_LIB" \
  'canonical media-host must dispatch capture probes in the same executable identity'
require 'run_capture_probe_request' "$MEDIA_HOST_LIB" \
  'canonical media-host must own one-shot exact capture execution'
require 'execute_one_shot_native_host' "$HOST_AUDIO" \
  'daemon host-audio admission must use the canonical media-host process'
forbid 'pipewire::|flexaudio_os_(linux|windows)::|RtlGetVersion' "$HOST_AUDIO" \
  'daemon host-audio capability monitor must not call native OS probe APIs'
forbid 'easynet_cli::|axon_sdk::|tonic::|PeerConnection|TrackLocal' "$MEDIA_HOST_LIB" \
  'media-host implementation must remain independent from Runtime, Axon and WebRTC crates'
forbid 'easynet_cli|package[[:space:]]*=[[:space:]]*"easynet"|path[[:space:]]*=[[:space:]]*"\.\./\.\./\.\."' "$MEDIA_HOST_CARGO" \
  'media-host must not depend on the root Runtime crate'
require 'mod webrtc_hosted_media;' "$TRANSPORT_MOD" \
  'native production transport must compile the supervised media-host bridge'
require 'any(target_os = "linux", target_os = "macos", target_os = "windows")' "$TRANSPORT_MOD" \
  'Linux, macOS and Windows native production transport must select the hosted media bridge'
require 'run_direct_webrtc_hosted_stream(&mut execution, &hosted_inputs)' "$WEBRTC_MEDIA" \
  'native direct WebRTC must dispatch through the supervised media-host path'
forbid 'open_display_recorder_with_xcap|run_direct_webrtc_recorder_stream' "$WEBRTC_MEDIA" \
  'Windows production WebRTC must not retain the daemon-local recorder fallback'
require 'MediaHostProcess::spawn' "$WEBRTC_HOSTED_MEDIA" \
  'the WebRTC bridge must launch a separately fenced media-host generation'
require 'pending_media_rebind_binding_for_session' "$WEBRTC_HOSTED_MEDIA" \
  'target rebind must restart the media-host generation through session-owned state'
require 'commit_pending_media_rebind_for_session' "$WEBRTC_HOSTED_MEDIA" \
  'replacement capture proof must commit before the new generation activates'
require 'CommandBody::Reconfigure' "$WEBRTC_HOSTED_MEDIA" \
  'daemon-owned adaptation must use the explicit media-host reconfiguration barrier'
require 'video_backpressure_drops_dependency_chain_until_recovery_idr' "$PROCESS" \
  'bounded video pressure must discard an undecodable dependency chain until a recovery IDR'
require 'take_video_recovery_request' "$PROCESS" \
  'video queue saturation must surface one explicit keyframe recovery request to the daemon'
require 'generation.request_keyframe()?' "$WEBRTC_HOSTED_MEDIA" \
  'the daemon WebRTC bridge must request an IDR after host-lane backpressure'
require 'daemon_video_frames_dropped' "$WEBRTC_HOSTED_MEDIA" \
  'daemon-side pressure drops must feed observability and adaptation instead of disappearing'
require 'pub(super) struct HostedMediaHostFailure' "$WEBRTC_HOSTED_MEDIA" \
  'helper failures must retain typed target/transport meaning across the private process boundary'
require 'direct_webrtc_hosted_failure_projection' "$WEBRTC_MEDIA" \
  'Linux hosted-media failures must project typed recovery actions instead of baseline failure labels'
require 'WebRtcFailureEventKind::MediaSourceLost' "$WEBRTC_MEDIA" \
  'target-invalidated hosted failures must project the existing media-source-loss lifecycle'
require 'active_media_session_audio_unavailable' "$MEDIA_HOST_LIB" \
  'media-host capability mode must not advertise audio that active session mode cannot emit'
require 'any(target_os = "linux", target_os = "windows", target_os = "macos")' "$MEDIA_HOST_LIB" \
  'false-audio capability must cover every native platform until each hosted Opus backend lands'
require 'assert!(!response.capability.compiled_supported)' "$MEDIA_HOST_LIB" \
  'media-host capability regression test must reject primitive-only audio readiness'
require 'cfg!(all(feature = "native-media", target_os = "macos"))' "$HOST_AUDIO" \
  'daemon capability truth must count only the canonical hosted Opus path as compiled audio support'
require 'any(target_os = "linux", target_os = "windows")' "$WEBRTC_ENDPOINT" \
  'Linux and Windows SDP admission must fail before negotiating audio the media-host cannot emit'
require 'REASON_ACTIVE_MEDIA_SESSION_AUDIO_UNAVAILABLE' "$VIEW_DEVICE" \
  'device capability projection must expose the canonical active-session audio blocker'
require 'the canonical media-host session cannot emit validator-checked Opus on this platform' "$VIEW_DEVICE" \
  'device capability projection must not advertise daemon-local audio primitives as product support'
require 'ScreenCapturePermission' "$MEDIA_SESSION_PROTOCOL" \
  'canonical media-host first frame must expose the bounded permission-control mode'
require 'execute_one_shot_native_host' "$PERMISSIONS" \
  'macOS permission abilities must invoke the canonical media-host process'
require 'ScreenCapturePermissionRequest::new' "$PERMISSIONS" \
  'macOS permission request must bind operation, generation and request id'
require 'CGRequestScreenCaptureAccess' "$MEDIA_HOST_LIB" \
  'the canonical media-host executable must own the macOS Screen Recording prompt'
if grep -R -Fq -- 'CGRequestScreenCaptureAccess' "$ROOT/plugins/remote-desktop/src"; then
  fail 'daemon plugin source must not retain the macOS Screen Recording prompt'
fi
require 'pub const REQUEST_KIND: &str = "screen_capture_permission";' "$SCREEN_PERMISSION_PROTOCOL" \
  'screen-capture permission helper mode must use a versioned private request contract'
require 'previously_granted && !self.granted' "$SCREEN_PERMISSION_PROTOCOL" \
  'permission response validation must reject within-request grant regression'
require 'actual_membership != application.window_ids' "$MEDIA_HOST_LINUX" \
  'Linux application capture must revalidate complete process-owned window membership'
require 'actual_front_to_back != expected_front_to_back' "$MEDIA_HOST_LINUX" \
  'Linux application capture must revalidate X11 stacking order inside the media process'
require 'mod macos_sck;' "$MEDIA_HOST_LIB" \
  'canonical media-host must own ScreenCaptureKit session execution'
require 'mod macos_multiapp;' "$MEDIA_HOST_LIB" \
  'canonical media-host must own macOS application multi-surface composition'
require 'mod macos_videotoolbox;' "$MEDIA_HOST_LIB" \
  'canonical media-host must own VideoToolbox encoding'
require 'MacOsScreenCaptureKitSessionBackend as PlatformSessionBackend' "$MEDIA_HOST_LIB" \
  'macOS active media must select the hosted ScreenCaptureKit backend'
require 'pub(super) fn probe_target' "$MEDIA_HOST_MAC" \
  'macOS exact target verification must execute inside the canonical media-host'
require 'pub(super) fn capture_diagnostic_jpeg' "$MEDIA_HOST_MAC" \
  'macOS diagnostic capture must execute inside the canonical media-host'
require 'observed_ids != expected_front_to_back' "$MEDIA_HOST_MAC" \
  'macOS application capture must revalidate complete membership and native stacking order'
require 'actual != required' "$MEDIA_HOST_MAC" \
  'macOS application capture must revalidate every committed surface geometry'
require 'MultiAppSurfaceCompositor::new' "$MEDIA_HOST_MAC" \
  'macOS application media must compose the exact committed surface set'
require 'VideoToolboxEncoder::new_with_wakeup_and_limits' "$MEDIA_HOST_MAC" \
  'macOS media-host must submit frames to the bounded VideoToolbox encoder'
require 'media_host_probe::verify_binding(ability, binding)' "$TARGET" \
  'macOS pre-session target verification must cross the canonical media-host boundary'
require 'media_host_probe::capture_diagnostic_jpeg' "$INVOKE_BIDI" \
  'macOS diagnostic capture must cross the canonical media-host boundary'
require 'pub(super) fn target_plan' "$MEDIA_HOST_PROBE" \
  'one exact target-plan projection must be shared by capture probes and active media'
require 'execute_one_shot_native_host' "$MEDIA_HOST_PROBE" \
  'capture probes must use bounded one-shot process supervision'
require 'any(target_os = "linux", target_os = "macos", target_os = "windows")' "$TRANSPORT_MOD" \
  'hosted shared-slot media transport must compile for Linux, macOS and Windows'
require 'real_x11_window_and_application_sessions_emit_recoverable_h264' "$MEDIA_LIVE" \
  'live media-host gate must execute the real X11 window/application process test'
require '#[ignore = "requires a real X11 display and the repository sentinel fixture"]' "$MEDIA_LIVE_TEST" \
  'the environment-owned real X11 proof must remain explicit instead of silently skipping'
require 'EventBody::VideoH264' "$MEDIA_LIVE_TEST" \
  'real media-host process proof must consume encoded H264 events'
require 'CommandBody::Reconfigure' "$MEDIA_LIVE_TEST" \
  'real media-host process proof must cover the codec reconfiguration barrier'
require 'run_application_invalidation_session' "$MEDIA_LIVE_TEST" \
  'real media-host process proof must invalidate a live generation after an application surface changes'
require 'FailureReason::TargetInvalidated' "$MEDIA_LIVE_TEST" \
  'real media-host process proof must assert typed target invalidation instead of generic process exit'
require 'injected_session_crash_closes_control_without_a_false_terminal' "$MEDIA_PROCESS_LIFECYCLE_TEST" \
  'real helper crash proof must reject a fabricated clean terminal frame'
require 'parent_liveness_eof_kills_a_hung_session_generation' "$MEDIA_PROCESS_LIFECYCLE_TEST" \
  'parent death must terminate a hung helper generation through the physical liveness lane'
require 'status.code() == Some(125)' "$MEDIA_PROCESS_LIFECYCLE_TEST" \
  'parent-liveness process proof must assert the dedicated watchdog exit outcome'
require 'real_process_permission_status_reports_the_media_host_identity' "$MEDIA_PROCESS_LIFECYCLE_TEST" \
  'macOS permission mode must be exercised through the real canonical helper process'

require 'pub const PROTOCOL: &str = "remoteapp_media_host_v1";' "$MEDIA_SESSION_PROTOCOL" \
  'active media must use the canonical versioned media-host protocol'
require 'pub enum MediaLane' "$MEDIA_SESSION_PROTOCOL" \
  'active media protocol must declare independent control/video/audio lanes'
require 'session_nonce' "$MEDIA_SESSION_PROTOCOL" \
  'every active media generation must be fenced by a daemon nonce'
require 'contract_digest' "$MEDIA_SESSION_PROTOCOL" \
  'active media frames must bind the exact start contract digest'
require 'pub struct MediaHostCommandValidator' "$MEDIA_SESSION_PROTOCOL" \
  'host commands must use an explicit lifecycle state machine'
require 'pub struct MediaConversationValidator' "$MEDIA_SESSION_PROTOCOL" \
  'daemon ingress must correlate commands, lifecycle and media'
require 'non-contiguous media sequence or regressing observation time' "$MEDIA_SESSION_PROTOCOL" \
  'active media lane sequences and clocks must fail closed'
require 'BeginMedia' "$MEDIA_SESSION_PROTOCOL" \
  'independent lanes must use an explicit post-activation release barrier'
require 'inspect_h264_annex_b(payload)' "$MEDIA_SESSION_PROTOCOL" \
  'active video validation must inspect raw Annex-B instead of trusting metadata'
require 'const VIDEO_FRAME_MAGIC: [u8; 4] = *b"RVID";' "$MEDIA_SESSION_PROTOCOL" \
  'video hot lane must use a fixed binary frame header instead of per-frame JSON metadata'
require 'const AUDIO_FRAME_MAGIC: [u8; 4] = *b"RAUD";' "$MEDIA_SESSION_PROTOCOL" \
  'audio hot lane must use a fixed binary frame header instead of per-frame JSON metadata'
require 'MediaLane::Control => write_control_event_frame' "$MEDIA_SESSION_PROTOCOL" \
  'only the low-frequency control lane may retain JSON event framing'
require 'MediaLane::Video => write_video_event_frame' "$MEDIA_SESSION_PROTOCOL" \
  'video events must dispatch through the fixed binary writer'
require 'MediaLane::Audio => write_audio_event_frame' "$MEDIA_SESSION_PROTOCOL" \
  'audio events must dispatch through the fixed binary writer'
require 'read_binary_media_event_frame' "$MEDIA_SESSION_PROTOCOL" \
  'daemon ingress must decode and fence the fixed binary media header'
require 'pub const SHARED_SLOT_NOTIFICATION_BYTES: usize = 56;' "$SHARED_MEDIA_PROTOCOL" \
  'Unix media notifications must remain fixed tickets instead of payload frames'
require 'pub fn publish_media_event(' "$SHARED_MEDIA_PROTOCOL" \
  'shared media producer must encode directly into its bounded slot'
require 'SharedMediaOutput {' "$MEDIA_HOST_LIB" \
  'Unix media-host sessions must use the shared media output boundary'
require '.publish_media_event(sequence, observed_at_ms, body, payload)?' "$MEDIA_HOST_LIB" \
  'media-host must publish encoded payloads into shared slots instead of payload pipes'
require 'SharedMediaLaneConsumer::open' "$PROCESS" \
  'daemon media ingress must open the generation-owned shared mapping'
require 'let frame = Bytes::from_owner(lease);' "$PROCESS" \
  'daemon media ingress must retain the mapped slot as the WebRTC Bytes owner'
require 'WindowsNotificationPipeServer::create' "$PROCESS" \
  'Windows media ingress must create independent generation-scoped notification lanes'
require 'SharedMediaLaneConsumer::open_named' "$PROCESS" \
  'Windows media ingress must lease the same generation-scoped shared mappings'
require 'CreateFileMappingW' "$SHARED_MEDIA_PROTOCOL" \
  'Windows media payloads must live in generation-scoped file mappings'
require 'CreateNamedPipeW' "$SHARED_MEDIA_PROTOCOL" \
  'Windows media notifications must use independent named pipes'
require 'PIPE_REJECT_REMOTE_CLIENTS' "$SHARED_MEDIA_PROTOCOL" \
  'Windows plugin-private notification pipes must reject remote clients'
require 'data: event.payload' "$WEBRTC_HOSTED_MEDIA" \
  'WebRTC sample submission must consume the mapped Bytes payload directly'
require 'SharedMediaLaneConsumer::open' "$MEDIA_LIVE_TEST" \
  'real X11 process proof must exercise the shared media lane, not payload-pipe framing'
require 'audio_hot_lane_uses_fixed_header_without_json_metadata' "$MEDIA_SESSION_PROTOCOL" \
  'protocol tests must prove the Opus lane carries no per-frame JSON metadata'
require 'pub const MAX_OPUS_PACKET_BYTES: usize = 1_275;' "$MEDIA_SESSION_PROTOCOL" \
  'active Opus payloads must retain their codec-specific hard bound'
forbid '^use .*(Runtime|TrackLocal|PeerConnection)|easynet_cli::|axon_sdk::|tonic::' "$MEDIA_SESSION_PROTOCOL" \
  'private active-media wire DTO must not absorb daemon-owned Runtime/WebRTC state'

if [[ "${CHECK_REMOTEAPP_NATIVE_HOST_SKIP_CARGO_TREE:-0}" != "1" ]]; then
  if cargo tree --offline --manifest-path "$HOST_CARGO" --edges normal --prefix none \
    | grep -Eq '^easynet v'; then
    fail 'native-host dependency graph reaches the root easynet Runtime crate'
  fi
  if cargo tree --offline --manifest-path "$MEDIA_HOST_CARGO" --edges normal --prefix none \
    | grep -Eq '^easynet v'; then
    fail 'media-host dependency graph reaches the root easynet Runtime crate'
  fi
fi

echo "check-remoteapp-native-host-boundary: ok"
