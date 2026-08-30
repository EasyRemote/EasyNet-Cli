#!/usr/bin/env bash
set -euo pipefail

ROOT="${CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

fail() {
  printf 'check-remoteapp-performance-boundary: %s\n' "$*" >&2
  exit 1
}

require() {
  local pattern="$1"
  local file="$2"
  local message="$3"
  [[ -f "$file" ]] || fail "missing $file"
  if ! rg -n -- "$pattern" "$file" >/dev/null; then
    fail "$message"
  fi
}

require_multiline() {
  local pattern="$1"
  local file="$2"
  local message="$3"
  [[ -f "$file" ]] || fail "missing $file"
  perl -0ne "exit(($pattern) ? 0 : 1)" "$file" || fail "$message"
}

reject() {
  local pattern="$1"
  local file="$2"
  local message="$3"
  [[ -f "$file" ]] || fail "missing $file"
  if rg -n -- "$pattern" "$file" >/dev/null; then
    fail "$message"
  fi
}

SPEC="$ROOT/docs/design/remoteapp-targeted-session-spec.md"
RESOURCE_BOOTSTRAP="$ROOT/src/daemon/ability/builtins/resources/media/resource_bootstrap.rs"
RESOURCE_LIST="$ROOT/src/daemon/ability/builtins/resources/list.rs"
WATCH_REMOTE_TARGETS="$ROOT/src/daemon/ability/builtins/resources/watch_remote_targets.rs"
TARGET_OBSERVER="$ROOT/plugins/remote-desktop/src/target_observer.rs"
TARGET_MONITOR="$ROOT/plugins/remote-desktop/src/target_monitor.rs"
NATIVE_HOST="$ROOT/plugins/remote-desktop/native-host/src/lib.rs"
MEDIA_HOST="$ROOT/plugins/remote-desktop/media-host/src/lib.rs"
NATIVE_HOST_PROCESS="$ROOT/plugins/remote-desktop/src/native_host_process.rs"
EVENT_LOG="$ROOT/plugins/remote-desktop/src/event_log.rs"
REQUEST="$ROOT/plugins/remote-desktop/src/request.rs"
HANDLERS="$ROOT/plugins/remote-desktop/src/handlers/mod.rs"
WATCH_EVENTS="$ROOT/plugins/remote-desktop/src/handlers/watch_events.rs"
ADD_ICE_CANDIDATE="$ROOT/plugins/remote-desktop/src/handlers/add_ice_candidate.rs"
SESSION_SIGNALING="$ROOT/plugins/remote-desktop/src/session_signaling.rs"
SESSION="$ROOT/plugins/remote-desktop/src/session.rs"
SESSION_STORE="$ROOT/plugins/remote-desktop/src/session_store.rs"
VIEW="$ROOT/plugins/remote-desktop/src/view.rs"
VIEW_DEVICE="$ROOT/plugins/remote-desktop/src/view_device.rs"
VIEW_TRANSPORT="$ROOT/plugins/remote-desktop/src/view_transport.rs"
SESSION_TRANSPORT_STATE="$ROOT/plugins/remote-desktop/src/session_transport_state.rs"
MEDIA_ADAPTATION="$ROOT/plugins/remote-desktop/src/media/adaptation.rs"
WEBRTC_HOSTED_MEDIA="$ROOT/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs"
WEBRTC_BASELINE_MEDIA="$ROOT/plugins/remote-desktop/src/transport/webrtc_baseline_media.rs"
WEBRTC_MEDIA="$ROOT/plugins/remote-desktop/src/transport/webrtc_media.rs"
WEBRTC_SENDER_FEEDBACK="$ROOT/plugins/remote-desktop/src/transport/webrtc_sender_feedback.rs"
HOST_AUDIO_CAPABILITY="$ROOT/plugins/remote-desktop/src/media/host_audio_capability.rs"
LINUX_PROCESS_TREE_AUDIO="$ROOT/plugins/remote-desktop/src/media/linux_process_tree_audio.rs"
SDP="$ROOT/plugins/remote-desktop/src/sdp.rs"
TARGET="$ROOT/plugins/remote-desktop/src/target.rs"
WEBRTC_ENDPOINT="$ROOT/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
WEBRTC_AUDIO="$ROOT/plugins/remote-desktop/src/transport/webrtc_audio.rs"
TRANSPORT_MANAGER="$ROOT/plugins/remote-desktop/src/transport/manager.rs"
INPUT="$ROOT/plugins/remote-desktop/src/input.rs"
REMOTE_DESKTOP_SCHEMA="$ROOT/plugins/remote-desktop/src/schema.rs"
REPORT_CLIENT_STATE="$ROOT/plugins/remote-desktop/src/handlers/report_client_state.rs"
REPORT_CLIENT_STATE_DESCRIPTOR="$ROOT/plugins/remote-desktop/abilities/remote_desktop.report_client_state.ability.toml"
SCRIPT_CHECKS="$ROOT/tests/script_checks.rs"

for checkpoint in PERF-01 PERF-02 PERF-03 PERF-04 PERF-05 PERF-06 PERF-07 PERF-08; do
  require "$checkpoint" "$SPEC" "SPEC must retain $checkpoint performance checkpoint"
done

require 'upsert_resources_indexed' "$RESOURCE_BOOTSTRAP" \
  'PERF-01 remote target refresh must use indexed resource mutation'
require 'remote_target_refresh_handles_large_persisted_inventory_with_indexed_batch' "$RESOURCE_BOOTSTRAP" \
  'PERF-01 must have a large R=10k/W=2k/A=200 remote target refresh test'
require 'const PERSISTED_RESOURCE_COUNT: usize = 10_000' "$RESOURCE_BOOTSTRAP" \
  'PERF-01 test must use R=10k persisted resources'
require 'const WINDOW_COUNT: usize = 2_000' "$RESOURCE_BOOTSTRAP" \
  'PERF-01 test must use W=2k windows'
require 'const APPLICATION_COUNT: usize = 200' "$RESOURCE_BOOTSTRAP" \
  'PERF-01 test must use A=200 applications'

require 'meta_list_resources_is_read_only_cache_projection' "$RESOURCE_LIST" \
  'PERF-02 must prove meta.list_resources is a read-only cache projection'
require 'std::fs::read\(&path\)' "$RESOURCE_LIST" \
  'PERF-02 must compare resources.json bytes before and after meta.list_resources'
require 'modified\(\)' "$RESOURCE_LIST" \
  'PERF-02 must compare resources.json mtime before and after meta.list_resources'

require 'EVENT_TARGET_INVENTORY_UNAVAILABLE' "$WATCH_REMOTE_TARGETS" \
  'resource.watch_remote_targets must expose a typed inventory-unavailable event'
require 'target_inventory_unavailable' "$WATCH_REMOTE_TARGETS" \
  'resource.watch_remote_targets must publish target_inventory_unavailable for discovery outages'
require_multiline 'm/inventory_hash\(\s*response\.screen_target_discovery_available,\s*&signatures\s*\)/s' "$WATCH_REMOTE_TARGETS" \
  'watch inventory hash must include discovery availability so unavailable-empty does not coalesce with available-empty'
require 'inventory_unavailable_without_removals' "$WATCH_REMOTE_TARGETS" \
  'watch unavailable projection must have a named invariant-preserving constructor'
require_multiline 'm/fn inventory_unavailable_without_removals\((?:(?!\n    \}).)*removed_resource_uras: Vec::new\(\)/s' "$WATCH_REMOTE_TARGETS" \
  'watch unavailable observations must not report previous targets as removed'
require 'unavailable_inventory_delta_does_not_report_targets_removed' "$WATCH_REMOTE_TARGETS" \
  'watch inventory must test unavailable observations do not emit removed_resource_uras'
require 'discovery_availability_participates_in_inventory_hash' "$WATCH_REMOTE_TARGETS" \
  'watch inventory must test discovery availability participates in the stable hash'
require 'watch_handler_emits_unavailable_without_removed_targets' "$WATCH_REMOTE_TARGETS" \
  'watch handler must test typed unavailable frames through the stream boundary'
require 'watch_input_schema_has_single_types_description_contract' "$WATCH_REMOTE_TARGETS" \
  'resource.watch_remote_targets must test that descriptor schema source does not duplicate types.description'

require 'sampled_host_target_observations_bound_session_fanout_to_one_enumeration_per_tick' "$TARGET_OBSERVER" \
  'PERF-03 must prove a sampled target observer fans out one host enumeration to 128 sessions'
require 'fn sample_xcap_target_observations' "$NATIVE_HOST" \
  'PERF-03 production host must own one explicit native inventory sampler'
require 'xcap::Window::all\(\)' "$NATIVE_HOST" \
  'PERF-03 native host must enumerate the host window set once per request'
require_multiline 'm/let provider = snapshot_executor\.sample_for_generation\(generation, provider_deadline\)\?;\s*tracked\.retain/s' "$TARGET_MONITOR" \
  'PERF-03 target monitor must obtain one generation-scoped host sample before retaining tracked sessions'
reject 'PlatformTargetObservationProvider' "$TARGET_MONITOR" \
  'PERF-03 target monitor must not instantiate a per-session platform observation provider'
reject 'SharedHostTargetSnapshotProvider' "$TARGET_OBSERVER" \
  'PERF-03 must not rely on refresh-window cache compatibility instead of explicit tick sampling'
reject 'xcap::|CGWindowListCopyWindowInfo|NSWorkspace' "$TARGET_OBSERVER" \
  'PERF-03 daemon-side target observer must not execute native inventory APIs'
require 'const SESSION_COUNT: usize = 128' "$TARGET_OBSERVER" \
  'PERF-03 shared sampler test must cover S=128 active session ticks'
require 'calls\.load\(Ordering::SeqCst\)' "$TARGET_OBSERVER" \
  'PERF-03 must inspect the host snapshot call count'
require 'one host enumeration for 128 session ticks in one monitor tick' "$TARGET_OBSERVER" \
  'PERF-03 must assert fanout is bounded to one enumeration for 128 session ticks in one monitor tick'

require 'event_log_retains_fixed_ring_and_monotonic_sequences_under_large_storm' "$EVENT_LOG" \
  'PERF-04 must prove bounded event ring behavior under a large storm'
require '100_000' "$EVENT_LOG" \
  'PERF-04 event storm test must push 100k events'
require 'RemoteDesktopEventReplay' "$EVENT_LOG" \
  'PERF-04 watch_events replay must use a domain replay projection, not handler-side JSON filtering'
require 'event_replay_projects_compaction_before_retained_window' "$EVENT_LOG" \
  'PERF-04 must prove replay reports compaction before the retained ring window'
require 'EVENT_LOG_COMPACTED' "$EVENT_LOG" \
  'PERF-04 replay must expose an explicit compaction diagnostic frame'
require 'requested_from_sequence' "$EVENT_LOG" \
  'PERF-04 compaction diagnostic must carry the requested replay cursor'
require 'first_retained_sequence' "$EVENT_LOG" \
  'PERF-04 compaction diagnostic must carry the first retained event sequence'
require 'replay_events_from' "$WATCH_EVENTS" \
  'watch_events handler must delegate replay-window semantics to the session aggregate'
require 'optional_u64_field\(&args, "from_sequence", ABILITY_WATCH_EVENTS\)' "$WATCH_EVENTS" \
  'watch_events handler must parse from_sequence through the typed request parser'
reject '\.get\("from_sequence"\)' "$WATCH_EVENTS" \
  'watch_events handler must not silently parse malformed from_sequence with direct JSON access'
require 'pub\(in crate::daemon::plugins::remote_desktop\) fn optional_u64_field' "$REQUEST" \
  'remote desktop request parser must expose typed optional u64 parsing to ability handlers'
require 'watch_events_rejects_malformed_replay_cursor' "$HANDLERS" \
  'PERF-04 must prove watch_events rejects malformed replay cursors'
require 'REASON_INVALID_ARGUMENT' "$HANDLERS" \
  'watch_events malformed replay cursor test must assert invalid_argument diagnostics'

require 'remote_desktop_signaling_rejects_more_than_ten_thousand_candidates_without_growth' "$SESSION_SIGNALING" \
  'PERF-05 must reject >10k trickle candidates without unbounded growth'
require 'fn to_bounded_view\(' "$SESSION_SIGNALING" \
  'PERF-05 signaling state must own bounded public view projection'
require 'remote_desktop_signaling_bounded_view_projects_counts_and_limits' "$SESSION_SIGNALING" \
  'PERF-05 must test bounded signaling view limits and candidate elision'
require 'signaling_state_validates_local_and_remote_ice_rows_before_storage' "$SESSION_SIGNALING" \
  'PERF-05 signaling state must validate local and remote ICE rows before storage'
require 'signaling_state_rejects_oversized_descriptions_before_storage' "$SESSION_SIGNALING" \
  'PERF-05 signaling state must reject oversized local and remote descriptions before storage'
require 'validate_signaling_description_size\(&value\)' "$SESSION_SIGNALING" \
  'PERF-05 signaling description byte limits must be enforced by the signaling domain object'
require 'set_local_webrtc_answer\(' "$SESSION_SIGNALING" \
  'PERF-05 local WebRTC answer commits must flow through bounded signaling state'
reject 'RemoteDesktopSessionDescription::new\("local", answer\)\.expect' "$SESSION_SIGNALING" \
  'PERF-05 generated local WebRTC answers must not panic or bypass signaling description limits'
require '"signaling_limits"' "$SESSION_SIGNALING" \
  'PERF-05 bounded signaling view must publish explicit signaling limits'
require '"remote_ice_candidates_elided": true' "$SESSION_SIGNALING" \
  'PERF-05 bounded signaling view must keep remote ICE candidate rows elided'
require 'session\.signaling_view\(transport_route_state\.clone\(\)\)' "$VIEW" \
  'PERF-05 public session view must consume the bounded signaling projection'
require 'production_readiness_reports_client_blocker_and_route_degradation_before_presentation' "$SESSION" \
  'PERF-05 production readiness must report client presentation blockers while preserving route degradation evidence'
require 'struct RemoteDesktopTransportReadinessBlocker' "$VIEW_TRANSPORT" \
  'PERF-05 transport route blockers must be centralized in a transport readiness blocker projection'
require '"readiness_blocker": self\.readiness_blocker\(\)' "$VIEW_TRANSPORT" \
  'PERF-05 transport summary and metadata must project the canonical readiness blocker'
require '"route_readiness_blocker": transport_view\.readiness_blocker\(\)' "$VIEW" \
  'PERF-05 production_readiness must expose the transport route blocker without making it the media readiness blocker'
require 'audio_support_view' "$VIEW_DEVICE" \
  'RemoteApp device view must expose explicit host-audio product state'
require 'AUDIO_UNSUPPORTED_REASON' "$VIEW_DEVICE" \
  'RemoteApp device view must use a stable host-audio blocked reason'
require 'runtime\.compiled_supported\(\)' "$VIEW_DEVICE" \
  'RemoteApp device view must project compiled host-audio support from the runtime capability snapshot'
require 'runtime\.runtime_reachable\(\)' "$VIEW_DEVICE" \
  'RemoteApp device view must keep compiled support separate from live host-audio reachability'
require '"supported_target_kinds"' "$VIEW_DEVICE" \
  'RemoteApp device view must publish target-scoped host-audio support'
require 'audio_support_view_for_binding' "$VIEW_DEVICE" \
  'RemoteApp session audio capability must be resolved against the bound target'
require 'struct HostAudioRuntimeProbe' "$HOST_AUDIO_CAPABILITY" \
  'RemoteApp host-audio runtime probing must be plugin-owned and cached outside session serialization'
require 'expires_at_monotonic' "$HOST_AUDIO_CAPABILITY" \
  'RemoteApp host-audio offer admission must fail closed on monotonic snapshot expiry'
require 'refresh_requested: bool' "$HOST_AUDIO_CAPABILITY" \
  'RemoteApp host-audio refresh work must coalesce into one fixed-state bit'
require 'mpsc::sync_channel\(1\)' "$HOST_AUDIO_CAPABILITY" \
  'RemoteApp host-audio supervisor wake queue must be capacity one'
reject 'mpsc::channel\(\)' "$HOST_AUDIO_CAPABILITY" \
  'RemoteApp host-audio capability monitor must not retain an unbounded command queue'
require 'attempt_running: bool' "$HOST_AUDIO_CAPABILITY" \
  'RemoteApp host-audio capability monitor must track at most one native probe attempt'
require 'blocked_native_probe_does_not_block_supervisor_shutdown' "$HOST_AUDIO_CAPABILITY" \
  'RemoteApp host-audio tests must prove a blocked native probe cannot hang daemon shutdown'
require 'native_probe_projection_keeps_source_readiness_independent' "$HOST_AUDIO_CAPABILITY" \
  'RemoteApp native source readiness must remain independent when projected into daemon admission state'
require 'LinuxProcessTreeAudioBackend' "$LINUX_PROCESS_TREE_AUDIO" \
  'RemoteApp Linux target audio must use the dedicated PipeWire process-tree backend'
require 'process_tree_includes_all_descendants_and_excludes_unrelated_processes' "$LINUX_PROCESS_TREE_AUDIO" \
  'RemoteApp Linux target audio must test root and descendant process selection'
require 'audio_node_selection_includes_every_node_in_the_process_tree' "$LINUX_PROCESS_TREE_AUDIO" \
  'RemoteApp Linux target audio must prove multiple nodes from the authorized process tree are retained'
require 'reused_root_pid_fails_closed_instead_of_selecting_new_process_tree' "$LINUX_PROCESS_TREE_AUDIO" \
  'RemoteApp Linux target audio must fail closed across root PID reuse'
require 'add_timer' "$LINUX_PROCESS_TREE_AUDIO" \
  'RemoteApp Linux target audio must revalidate process authority without relying on graph events'
require 'empty_authority_set_revokes_all_previously_eligible_nodes' "$LINUX_PROCESS_TREE_AUDIO" \
  'RemoteApp Linux target audio must prove stale nodes are revoked when process authority disappears'
require 'contradictory_node_and_client_pid_identity_fails_closed_in_both_directions' "$LINUX_PROCESS_TREE_AUDIO" \
  'RemoteApp Linux target audio must reject contradictory node and owning-client process identities'
require 'json!\(\["display", "window", "application"\]\)' "$VIEW_DEVICE" \
  'RemoteApp Linux host-audio capability must publish every implemented target kind'
require '"capability": "host_audio"' "$VIEW_DEVICE" \
  'RemoteApp unsupported capability list must include host_audio when the current runtime cannot provide it'
require 'device_capabilities_report_platform_host_audio_support' "$VIEW_DEVICE" \
  'RemoteApp device capability tests must pin platform host-audio state'
require 'media_pipeline_support_view' "$VIEW_DEVICE" \
  'RemoteApp device view must expose a canonical media pipeline support projection'
require '"media_pipeline_support": media_pipeline_support' "$VIEW_DEVICE" \
  'RemoteApp device metadata must include media_pipeline_support'
require '"product_ready": false' "$VIEW_DEVICE" \
  'RemoteApp media pipeline support must not report full audio/video product readiness while host audio and live E2E are missing'
require 'remoteapp_media_adaptation_e2e_artifact_missing' "$VIEW_DEVICE" \
  'RemoteApp media pipeline support must expose missing live media-adaptation E2E as a product blocker'
require 'bounded_queue_drop_stale_frames' "$VIEW_DEVICE" \
  'RemoteApp media pipeline support must publish bounded queue stale-frame drop policy'
require 'native_bitrate_adaptation_from_webrtc_stats_and_encoder_pressure' "$VIEW_DEVICE" \
  'RemoteApp media pipeline support must publish native bitrate adaptation policy'
require 'receiver_feedback_openh264_rebuild' "$VIEW_DEVICE" \
  'RemoteApp diagnostic media pipeline must publish its receiver-feedback encoder rebuild policy'
require 'device_capabilities_project_media_pipeline_support_matrix' "$VIEW_DEVICE" \
  'RemoteApp device capability tests must pin media pipeline support projection'
require '"audio": audio\.clone\(\)' "$VIEW" \
  'RemoteApp public session view must expose explicit audio product state'
require 'enum RemoteDesktopNegotiatedMediaScope' "$SESSION_SIGNALING" \
  'RemoteApp signaling state must model negotiated media scope as a typed domain fact'
require 'RemoteDesktopNegotiatedMediaScope::from_local_answer' "$SESSION_SIGNALING" \
  'RemoteApp must derive negotiated media scope from the committed local WebRTC answer'
require 'let negotiated_media_scope = session\.negotiated_media_scope\(\)' "$VIEW" \
  'RemoteApp production readiness must read per-session negotiated media scope'
require 'let audio_required = negotiated_media_scope\.is_some_and' "$VIEW" \
  'RemoteApp production readiness must require audio only for an audio-video negotiation'
require '"media_scope": negotiated_media_scope' "$VIEW" \
  'RemoteApp production readiness must project negotiated scope instead of host capability'
reject '"media_scope": if audio_support\["supported"\]' "$VIEW" \
  'RemoteApp production readiness must not infer negotiation from host audio capability'
require 'let audio_ready = session\.audio_operational_ready\(\)' "$VIEW" \
  'RemoteApp production readiness must consume the session-owned operational audio predicate'
require 'audio_operational_ready' "$WEBRTC_AUDIO" \
  'RemoteApp audio runtime stats must distinguish operational readiness from media observation'
require '"audio_blocked_reason": audio_blocked_reason' "$VIEW" \
  'RemoteApp production readiness must expose the effective runtime or capability audio blocker'
require 'session_view_separates_platform_audio_capability_from_unnegotiated_scope' "$VIEW" \
  'RemoteApp session view tests must separate platform capability from session negotiation'
require 'video_only_negotiation_requires_bound_decode_but_not_audio_runtime_stats' "$VIEW" \
  'RemoteApp must prove video-only readiness requires exact receiver decode evidence without waiting for an absent audio track'
require 'audio_video_negotiation_requires_live_audio_runtime_stats' "$VIEW" \
  'RemoteApp must prove audio-video negotiation remains gated on live audio evidence'
require 'struct ClientRenderEvidence' "$SESSION_TRANSPORT_STATE" \
  'RemoteApp browser decode evidence must be a typed transport-generation fact'
require 'client_render_evidence_sequence' "$SESSION_TRANSPORT_STATE" \
  'RemoteApp browser decode evidence must use daemon-owned admission ordering'
require 'CLIENT_RENDER_EVIDENCE_MAX_AGE' "$SESSION" \
  'RemoteApp product readiness must expire stale receiver decode evidence'
require 'client_decode_ready\(\)' "$SESSION" \
  'RemoteApp session aggregate must own the exact receiver decode readiness predicate'
require 'render_probe_requires_exact_active_session_binding_tuple' "$REPORT_CLIENT_STATE" \
  'RemoteApp client reports must test the complete current session/binding/media-source tuple'
require 'render_probe_rejects_replay_and_counter_regression' "$REPORT_CLIENT_STATE" \
  'RemoteApp client render evidence must reject replay and cumulative counter regression'
require 'authored_descriptor_and_runtime_schema_are_identical' "$REMOTE_DESKTOP_SCHEMA" \
  'RemoteApp authored and NativeStatic report_client_state schemas must be parity-gated'
for field in session_id transport_epoch binding_id binding_epoch media_source_epoch media_pipeline_id video_codec video_transport decoded_video_frames frame_width frame_height; do
  require "$field" "$REPORT_CLIENT_STATE_DESCRIPTOR" \
    "RemoteApp report_client_state descriptor must require render evidence field $field"
done
require 'view\["production_readiness"\]\["route_readiness_blocker"\]\["frontend_action"\]' "$SESSION" \
  'PERF-05 production readiness tests must prove route blockers publish frontend recovery action separately'
require 'summary\["readiness_blocker"\]\["frontend_action"\]' "$VIEW_TRANSPORT" \
  'PERF-05 transport view tests must prove blockers publish frontend recovery action'
require 'transports\[0\]\["metadata"\]\["readiness_blocker"\]' "$VIEW_TRANSPORT" \
  'PERF-05 transport metadata must reuse the summary readiness blocker'
reject 'let remote_ice_candidates = session\.remote_ice_candidates\(\)' "$VIEW" \
  'PERF-05 public session view must not manually clone remote ICE candidates'
reject 'let local_ice_candidates = session\.local_ice_candidates\(\)' "$VIEW" \
  'PERF-05 public session view must not manually clone local ICE candidates'
require 'serialized_session_view_remains_bounded_at_signaling_limits' "$SESSION_STORE" \
  'PERF-05 must prove serialized session views stay bounded'
require 'remote_ice_candidates_elided' "$SESSION_STORE" \
  'PERF-05 serialized session view test must assert remote candidate elision'
require 'signaling_rejects_oversized_sdp_and_ice_rows' "$SDP" \
  'PERF-05 must reject oversized SDP and ICE rows before storage'
require 'Reserved\(DirectWebRtcEndpoint\)' "$ADD_ICE_CANDIDATE" \
  'Remote ICE admission must carry the reserved endpoint snapshot in the domain result'
require 'RemoteIceAdmission::Reserved\(endpoint\) => endpoint' "$ADD_ICE_CANDIDATE" \
  'Remote ICE candidate application must consume the typed reserved endpoint without panic proof'
reject 'reserved admission requires an active endpoint' "$ADD_ICE_CANDIDATE" \
  'Remote ICE admission must not prove endpoint presence with expect'
reject 'unreachable!' "$ADD_ICE_CANDIDATE" \
  'Remote ICE admission must fail closed instead of using unreachable panic paths'
require 'MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION: usize' "$SESSION_STORE" \
  'SPEC terminal row retention must encode T <= 4S as a store-level constant'
require '^    4;$' "$SESSION_STORE" \
  'SPEC terminal row retention constant must be exactly four rows per active session'
require 'prune_terminal_rows_to_active_bound_locked' "$SESSION_STORE" \
  'SPEC terminal row retention must be enforced by the session store aggregate'
require 'active_count\.saturating_mul\(MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION\)' "$SESSION_STORE" \
  'SPEC terminal row retention must use active session count S, not configured capacity'
require 'terminal_rows_are_pruned_to_four_times_active_sessions' "$SESSION_STORE" \
  'SPEC terminal row retention must prove T is pruned to 4S'
require 'terminal_rows_are_removed_when_no_active_sessions_remain' "$SESSION_STORE" \
  'SPEC terminal row retention must prove S=0 prunes all tombstone rows at maintenance boundary'
reject 'max_sessions\(\)\.saturating_mul\(4\)' "$ROOT/plugins/remote-desktop/src/session_lifecycle.rs" \
  'terminal row retention must not use configured capacity as S'

require 'resolver_refuses_to_run_while_session_store_lock_is_held' "$TARGET" \
  'PERF-06 must reject target resolution while session store lock is held'
require 'endpoint_start_boundary_refuses_to_run_while_session_store_lock_is_held' "$WEBRTC_ENDPOINT" \
  'PERF-06 must reject WebRTC endpoint startup while session store lock is held'
require_multiline 'm/pub\(in crate::daemon::plugins::remote_desktop\) fn block_on<F: Future>\(.+?\) -> anyhow::Result<F::Output>/s' "$TRANSPORT_MANAGER" \
  'PERF-06 direct WebRTC async runtime boundary must propagate runtime initialization failure'
require 'build RemoteApp WebRTC runtime: \{error\}' "$TRANSPORT_MANAGER" \
  'PERF-06 direct WebRTC runtime initialization failure must be surfaced as a transport error'
reject 'expect\("build RemoteApp WebRTC runtime"\)' "$TRANSPORT_MANAGER" \
  'PERF-06 direct WebRTC runtime initialization must not panic'
require 'transports\.block_on\(create_direct_webrtc_endpoint' "$WEBRTC_ENDPOINT" \
  'PERF-06 endpoint setup must run through the fallible transport runtime boundary'
require_multiline 'm/let build = transports\.block_on\(create_direct_webrtc_endpoint\(.+?\}\)\)\?;\s*let \(answer, peer_connection, completion\) = match build/s' "$WEBRTC_ENDPOINT" \
  'PERF-06 endpoint setup must preserve the nested construction result for reservation-owned cleanup'
require 'let transport_runtime = endpoint_config\.transports\.runtime_handle\(\)\?;' "$WEBRTC_ENDPOINT" \
  'PERF-06 media-loop runtime acquisition must fail before worker ownership crosses the thread boundary'
require 'transport_runtime\.block_on\(run_direct_webrtc_media_loop' "$WEBRTC_ENDPOINT" \
  'PERF-06 media-loop worker must use the pre-acquired fallible runtime handle'
require 'remote_desktop\.target\.resolve_for_session' "$TARGET" \
  'PERF-06 must identify the target resolver lock-boundary stage'
require '\{stage\} must not run while RemoteDesktopSessionStore is locked' "$SESSION_STORE" \
  'PERF-06 must use an explicit shared lock-boundary diagnostic'

require 'input_reject_diagnostics_are_coalesced_under_high_rate_storm' "$INPUT" \
  'PERF-07 must prove input reject diagnostics are coalesced under a high-rate storm'
require 'const REJECT_STORM: u64 = 10_000' "$INPUT" \
  'PERF-07 input storm test must use 10k rejected frames'
require 'const MAX_INPUT_REJECTION_DIAGNOSTIC_SAMPLES_PER_SIGNATURE' "$INPUT" \
  'PERF-07 input reject diagnostics must have a hard sample cap per signature'
require '< MAX_INPUT_REJECTION_DIAGNOSTIC_SAMPLES_PER_SIGNATURE' "$INPUT" \
  'PERF-07 input reject coalescer must enforce the hard sample cap'
require 'sample cap plus flush summary' "$INPUT" \
  'PERF-07 input storm test must assert sample cap plus flush summary'
require 'diagnostic_sample_limit' "$INPUT" \
  'PERF-07 input reject diagnostics must publish the sample limit'
require 'coalesced_rejections' "$INPUT" \
  'PERF-07 must expose coalesced rejection counts'

reject 'available_outgoing_bitrate_bps: Option<f64>' "$SESSION_TRANSPORT_STATE" \
  'PERF-08 must not treat Browser outgoing/upload capacity as device-to-Browser encoder authority'
reject 'pressure\.available_outgoing_bitrate_bps' "$WEBRTC_HOSTED_MEDIA" \
  'PERF-08 hosted media must not consume a reverse-direction Browser upload estimate'
reject 'pressure\.available_outgoing_bitrate_bps' "$WEBRTC_BASELINE_MEDIA" \
  'PERF-08 baseline media must not consume a reverse-direction Browser upload estimate'
require 'ADAPTIVE_MIN_BITRATE_KBPS: u32 = 128' "$MEDIA_ADAPTATION" \
  'PERF-08 adaptive encoder floor must remain distinct from the public requested-quality floor'
require 'SharedMediaLaneProducer' "$MEDIA_HOST" \
  'PERF-08 media-host must publish encoded payloads through the binary shared lane'
require 'Bytes::from_owner\(lease\)' "$NATIVE_HOST_PROCESS" \
  'PERF-08 daemon media ingress must validate the mapped shared slot without JSON/base64 payload copying'
require 'Bytes::from_owner\(transport_pool\.copy_from_slice\(payload_view\)\)' "$NATIVE_HOST_PROCESS" \
  'PERF-08 daemon media ingress must detach transport-owned bytes into a pooled buffer before RTP/NACK lifetime can pin the shared slot'
require 'video_sender: Arc<dyn RtpSender>' "$WEBRTC_MEDIA" \
  'PERF-08 each transport generation must retain its exact negotiated video sender'
require 'RTCStatsReportEntry::RemoteInboundRtp' "$WEBRTC_SENDER_FEEDBACK" \
  'PERF-08 sender adaptation must consume direction-correct remote-inbound RTCP receiver evidence'
require 'round_trip_time_measurements' "$WEBRTC_SENDER_FEEDBACK" \
  'PERF-08 RTCP pressure must have a monotonic fresh-report discriminator'
require 'sample\.measurements <= self\.last_measurements' "$WEBRTC_SENDER_FEEDBACK" \
  'PERF-08 repeated RTCP snapshots must not replay congestion pressure'
require 'configure_twcc_sender_only' "$WEBRTC_ENDPOINT" \
  'PERF-08 send-only RemoteApp video must negotiate TWCC in the sender direction'
reject 'register_default_interceptors' "$WEBRTC_ENDPOINT" \
  'PERF-08 endpoint must not restore the crate default receiver-only TWCC path'
require_multiline 'm/rtcp_receiver\s*\.observe\(inputs\.video_sender/s' "$WEBRTC_HOSTED_MEDIA" \
  'PERF-08 hosted media must consume local RTCP from its generation-owned sender'
require_multiline 'm/rtcp_receiver\s*\.observe\(video_sender/s' "$WEBRTC_BASELINE_MEDIA" \
  'PERF-08 baseline media must consume local RTCP from its generation-owned sender'
require 'effective_fps_for_writer_service' "$MEDIA_ADAPTATION" \
  'PERF-08 frame pacing must be independently bounded by measured RTP-writer service time'
require 'writer_service_time_independently_bounds_frame_rate' "$MEDIA_ADAPTATION" \
  'PERF-08 media policy must prove a slow writer lowers FPS independently of nominal bitrate'
require 'record_writer_service' "$WEBRTC_HOSTED_MEDIA" \
  'PERF-08 hosted media must measure the actual awaited RTP writer service duration'
require 'effective_fps_for_writer_service' "$WEBRTC_HOSTED_MEDIA" \
  'PERF-08 hosted media must apply the shared writer-service FPS policy'
require 'record_writer_service' "$WEBRTC_BASELINE_MEDIA" \
  'PERF-08 baseline media must measure the actual awaited RTP writer service duration'
require 'effective_fps_for_writer_service' "$WEBRTC_BASELINE_MEDIA" \
  'PERF-08 baseline media must apply the shared writer-service FPS policy'
require 'writer_service_can_reconfigure_fps_without_falsifying_bitrate_change' "$WEBRTC_BASELINE_MEDIA" \
  'PERF-08 baseline media must prove FPS-only reconfiguration is not reported as a bitrate change'

require 'remoteapp_performance_boundary_script_holds' "$SCRIPT_CHECKS" \
  'remoteapp performance boundary must be wired into cargo script_checks'

printf 'check-remoteapp-performance-boundary: ok\n'
