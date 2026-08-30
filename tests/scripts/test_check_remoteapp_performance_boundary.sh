#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-performance-boundary.sh"

fail() {
  printf '%s\n' "$1" >&2
  exit 1
}

SB="$(mktemp -d)"
trap 'rm -rf "$SB"' EXIT

mkdir -p \
  "$SB/tools/scripts" \
  "$SB/docs/design" \
  "$SB/src/daemon/ability/builtins/resources/media" \
  "$SB/src/daemon/ability/builtins/resources" \
  "$SB/plugins/remote-desktop/src/transport" \
  "$SB/plugins/remote-desktop/src/handlers" \
  "$SB/plugins/remote-desktop/abilities" \
  "$SB/plugins/remote-desktop/src/media" \
  "$SB/plugins/remote-desktop/media-host/src" \
  "$SB/plugins/remote-desktop/native-host/src" \
  "$SB/plugins/remote-desktop/src" \
  "$SB/tests"
cp "$SCRIPT" "$SB/tools/scripts/check-remoteapp-performance-boundary.sh"

cat >"$SB/docs/design/remoteapp-targeted-session-spec.md" <<'MD'
PERF-01 PERF-02 PERF-03 PERF-04 PERF-05 PERF-06 PERF-07 PERF-08
MD

cat >"$SB/src/daemon/ability/builtins/resources/media/resource_bootstrap.rs" <<'RS'
fn refresh() { upsert_resources_indexed(); }
#[test]
fn remote_target_refresh_handles_large_persisted_inventory_with_indexed_batch() {
    const PERSISTED_RESOURCE_COUNT: usize = 10_000;
    const WINDOW_COUNT: usize = 2_000;
    const APPLICATION_COUNT: usize = 200;
}
RS

cat >"$SB/src/daemon/ability/builtins/resources/list.rs" <<'RS'
#[test]
fn meta_list_resources_is_read_only_cache_projection() {
    let _ = std::fs::read(&path);
    let _ = metadata.modified();
}
RS

cat >"$SB/src/daemon/ability/builtins/resources/watch_remote_targets.rs" <<'RS'
const EVENT_TARGET_INVENTORY_UNAVAILABLE: &str = "target_inventory_unavailable";

fn snapshot_refresh() {
    let inventory_hash = inventory_hash(response.screen_target_discovery_available, &signatures);
}

impl RemoteTargetWatchEvent {
    fn inventory_unavailable_without_removals() -> Self {
        Self {
            event_type: EVENT_TARGET_INVENTORY_UNAVAILABLE.to_string(),
            removed_resource_uras: Vec::new(),
        }
    }
}

#[test]
fn unavailable_inventory_delta_does_not_report_targets_removed() {}

#[test]
fn discovery_availability_participates_in_inventory_hash() {}

#[test]
fn watch_handler_emits_unavailable_without_removed_targets() {}

#[test]
fn watch_input_schema_has_single_types_description_contract() {}
RS

cat >"$SB/plugins/remote-desktop/src/target_observer.rs" <<'RS'
#[test]
fn sampled_host_target_observations_bound_session_fanout_to_one_enumeration_per_tick() {
    const SESSION_COUNT: usize = 128;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "PERF-03 sampled target observer must use one host enumeration for 128 session ticks in one monitor tick"
    );
}
RS

cat >"$SB/plugins/remote-desktop/native-host/src/lib.rs" <<'RS'
fn sample_xcap_target_observations() {
    let windows = xcap::Window::all();
}
RS

cat >"$SB/plugins/remote-desktop/src/target_monitor.rs" <<'RS'
fn poll_tracked_sessions() {
    let provider = snapshot_executor.sample_for_generation(generation, provider_deadline)?;
    tracked.retain(|session_id| {
        observe_bound_session_target_once(&sessions, session_id, &provider);
        true
    });
}
RS

cat >"$SB/plugins/remote-desktop/src/event_log.rs" <<'RS'
struct RemoteDesktopEventReplay;
#[test]
fn event_log_retains_fixed_ring_and_monotonic_sequences_under_large_storm() {
    let _ = 100_000;
}
#[test]
fn event_replay_projects_compaction_before_retained_window() {
    let _ = "EVENT_LOG_COMPACTED";
    let _ = "requested_from_sequence";
    let _ = "first_retained_sequence";
}
RS

cat >"$SB/plugins/remote-desktop/src/handlers/watch_events.rs" <<'RS'
fn handle(args: Args, session: Session) {
    let from_sequence =
        optional_u64_field(&args, "from_sequence", ABILITY_WATCH_EVENTS)?.unwrap_or(0);
    let _ = session.replay_events_from(from_sequence);
}
RS

cat >"$SB/plugins/remote-desktop/src/request.rs" <<'RS'
pub(in crate::daemon::plugins::remote_desktop) fn optional_u64_field() {}
RS

cat >"$SB/plugins/remote-desktop/src/handlers/mod.rs" <<'RS'
#[test]
fn watch_events_rejects_malformed_replay_cursor() {
    let _ = "from_sequence";
    let _ = REASON_INVALID_ARGUMENT;
}
RS

cat >"$SB/plugins/remote-desktop/src/handlers/add_ice_candidate.rs" <<'RS'
struct DirectWebRtcEndpoint;
struct Value;

enum RemoteIceAdmission {
    Reserved(DirectWebRtcEndpoint),
    Committed(Value),
}

fn apply(admission: RemoteIceAdmission) -> Value {
    let endpoint = match admission {
        RemoteIceAdmission::Committed(view) => return view,
        RemoteIceAdmission::Reserved(endpoint) => endpoint,
    };
    let _ = endpoint;
    Value
}
RS

cat >"$SB/plugins/remote-desktop/src/session_signaling.rs" <<'RS'
enum RemoteDesktopNegotiatedMediaScope {
    VideoOnly,
    AudioVideo,
}

impl RemoteDesktopNegotiatedMediaScope {
    fn from_local_answer(answer: &Value) -> anyhow::Result<Self> {
        Ok(Self::VideoOnly)
    }
}

impl RemoteDesktopSessionDescription {
    fn new(value: Value) -> anyhow::Result<Self> {
        validate_signaling_description_size(&value)?;
        Ok(Self)
    }
}

fn set_local_webrtc_answer(answer: Value) -> anyhow::Result<()> {
    let _scope = RemoteDesktopNegotiatedMediaScope::from_local_answer(&answer)?;
    RemoteDesktopSessionDescription::new(answer)?;
    Ok(())
}

fn to_bounded_view() {
    "signaling_limits";
    "remote_ice_candidates_elided": true;
}

#[test]
fn remote_desktop_signaling_rejects_more_than_ten_thousand_candidates_without_growth() {}

#[test]
fn remote_desktop_signaling_bounded_view_projects_counts_and_limits() {}

#[test]
fn signaling_state_validates_local_and_remote_ice_rows_before_storage() {}

#[test]
fn signaling_state_rejects_oversized_descriptions_before_storage() {}
RS

cat >"$SB/plugins/remote-desktop/src/session_store.rs" <<'RS'
pub(in crate::daemon::plugins::remote_desktop) const MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION: usize =
    4;

fn prune_terminal_rows_to_active_bound_locked() {
    let terminal_limit = active_count.saturating_mul(MAX_TERMINAL_ROWS_PER_ACTIVE_SESSION);
}

#[test]
fn serialized_session_view_remains_bounded_at_signaling_limits() {
    let _ = "remote_ice_candidates_elided";
}

#[test]
fn terminal_rows_are_pruned_to_four_times_active_sessions() {}

#[test]
fn terminal_rows_are_removed_when_no_active_sessions_remain() {}

fn assert_current_thread_unlocked(stage: &str) {
    assert_eq!(0, 0, "{stage} must not run while RemoteDesktopSessionStore is locked");
}
RS

cat >"$SB/plugins/remote-desktop/src/session_lifecycle.rs" <<'RS'
fn prune_inactive_sessions() {}
RS

cat >"$SB/plugins/remote-desktop/src/session.rs" <<'RS'
#[test]
fn production_readiness_reports_client_blocker_and_route_degradation_before_presentation() {
    assert_eq!(
        view["production_readiness"]["route_readiness_blocker"]["frontend_action"],
        json!("retry_session")
    );
}
RS

cat >"$SB/plugins/remote-desktop/src/view_device.rs" <<'RS'
pub(in crate::daemon::plugins::remote_desktop) const AUDIO_UNSUPPORTED_REASON: &str =
    "native_media_disabled";

fn audio_support_view(runtime: HostAudioRuntimeSnapshot) {
    let _ = runtime.compiled_supported();
    let _ = runtime.runtime_reachable();
    json!(["display", "window", "application"]);
    json!({
        "supported": false,
        "capture_ready": false,
        "send_ready": false,
        "supported_target_kinds": [],
        "codec_profiles": [],
        "blocked_reason": AUDIO_UNSUPPORTED_REASON,
    });
    json!({
        "unsupported_capabilities": [
            {
                "capability": "host_audio",
                "reason": AUDIO_UNSUPPORTED_REASON,
            }
        ]
    });
}

fn audio_support_view_for_binding() {}

fn media_pipeline_support_view() {
    json!({
        "media_pipeline_support": media_pipeline_support,
        "product_ready": false,
        "product_blockers": [
            AUDIO_UNSUPPORTED_REASON,
            "remoteapp_media_adaptation_e2e_artifact_missing"
        ],
        "video": {
            "backpressure_policy": "bounded_queue_drop_stale_frames",
            "adaptation_policy": "native_bitrate_adaptation_from_webrtc_stats_and_encoder_pressure",
        },
        "diagnostic": {
            "adaptation_policy": "receiver_feedback_openh264_rebuild",
        }
    });
}

#[test]
fn device_capabilities_report_platform_host_audio_support() {}

#[test]
fn device_capabilities_project_media_pipeline_support_matrix() {}
RS

cat >"$SB/plugins/remote-desktop/src/view.rs" <<'RS'
fn serialize_session(session: Session, transport_route_state: Value) {
    let _ = session.signaling_view(transport_route_state.clone());
    let audio = audio_support_view();
    json!({
        "audio": audio.clone(),
        "route_readiness_blocker": transport_view.readiness_blocker(),
    });
}

fn production_readiness_view() {
    let audio_support = audio_support_view();
    let negotiated_media_scope = session.negotiated_media_scope();
    let audio_required = negotiated_media_scope.is_some_and(|scope| scope.requires_audio());
    let audio_ready = session.audio_operational_ready();
    let audio_blocked_reason = json!("host_audio_not_yet_ready");
    json!({
        "media_scope": negotiated_media_scope.map(|scope| scope.as_str()).unwrap_or("not_negotiated"),
        "audio_required": audio_required,
        "audio_ready": audio_ready,
        "audio_blocked_reason": audio_blocked_reason,
    });
}

#[test]
fn session_view_separates_platform_audio_capability_from_unnegotiated_scope() {}

#[test]
fn video_only_negotiation_requires_bound_decode_but_not_audio_runtime_stats() {}

#[test]
fn audio_video_negotiation_requires_live_audio_runtime_stats() {}
RS

cat >"$SB/plugins/remote-desktop/src/media/host_audio_capability.rs" <<'RS'
struct HostAudioRuntimeProbe;
struct HostAudioProbeCoordinator;
struct HostAudioCoordinatorState {
    refresh_requested: bool,
    attempt_running: bool,
}
fn monitor() {
    let _ = expires_at_monotonic;
    let _ = mpsc::sync_channel(1);
}
#[test]
fn blocked_native_probe_does_not_block_supervisor_shutdown() {}
#[test]
fn native_probe_projection_keeps_source_readiness_independent() {}
RS

cat >"$SB/plugins/remote-desktop/src/transport/webrtc_audio.rs" <<'RS'
fn project() { let _ = "audio_operational_ready"; }
RS

cat >"$SB/plugins/remote-desktop/src/session_transport_state.rs" <<'RS'
struct ClientRenderEvidence;
fn state() {
    let _ = client_render_evidence_sequence;
}
RS

cat >"$SB/plugins/remote-desktop/src/media/adaptation.rs" <<'RS'
const ADAPTIVE_MIN_BITRATE_KBPS: u32 = 128;
fn effective_fps_for_writer_service() {}
fn adaptive(available_kbps: u32) {
    let _ = available_kbps;
}
#[test]
fn writer_service_time_independently_bounds_frame_rate() {}
RS

cat >"$SB/plugins/remote-desktop/src/transport/webrtc_hosted_media.rs" <<'RS'
async fn adapt(inputs: HostedMediaInputs) {
    let pressure = rtcp_receiver.observe(inputs.video_sender, Instant::now()).await;
    record_writer_service();
    effective_fps_for_writer_service();
}
RS

cat >"$SB/plugins/remote-desktop/src/transport/webrtc_baseline_media.rs" <<'RS'
async fn adapt(video_sender: Sender) {
    let pressure = rtcp_receiver.observe(video_sender, Instant::now()).await;
    record_writer_service();
    effective_fps_for_writer_service();
}
#[test]
fn writer_service_can_reconfigure_fps_without_falsifying_bitrate_change() {}
RS

cat >"$SB/plugins/remote-desktop/media-host/src/lib.rs" <<'RS'
struct SharedMediaLaneProducer;
RS

cat >"$SB/plugins/remote-desktop/src/native_host_process.rs" <<'RS'
fn read_frame(lease: Lease) {
    let frame = Bytes::from_owner(lease);
    let payload = Bytes::from_owner(transport_pool.copy_from_slice(payload_view));
}
RS

cat >>"$SB/plugins/remote-desktop/src/session.rs" <<'RS'
const CLIENT_RENDER_EVIDENCE_MAX_AGE: Duration = Duration::from_secs(10);
fn client_decode_ready() {}
RS

cat >"$SB/plugins/remote-desktop/src/handlers/report_client_state.rs" <<'RS'
#[test]
fn render_probe_requires_exact_active_session_binding_tuple() {}
#[test]
fn render_probe_rejects_replay_and_counter_regression() {}
RS

cat >"$SB/plugins/remote-desktop/src/schema.rs" <<'RS'
fn schema() {
    let _ = (session_id, transport_epoch, binding_id, binding_epoch, media_source_epoch,
        media_pipeline_id, video_codec, video_transport, decoded_video_frames, frame_width, frame_height);
}
#[test]
fn authored_descriptor_and_runtime_schema_are_identical() {}
RS

cat >"$SB/plugins/remote-desktop/abilities/remote_desktop.report_client_state.ability.toml" <<'TOML'
required = ["session_id", "transport_epoch", "binding_id", "binding_epoch", "media_source_epoch", "media_pipeline_id", "video_codec", "video_transport", "decoded_video_frames", "frame_width", "frame_height"]
TOML

cat >"$SB/plugins/remote-desktop/src/view_transport.rs" <<'RS'
struct RemoteDesktopTransportReadinessBlocker;

fn summary() {
    json!({
        "readiness_blocker": self.readiness_blocker(),
    });
    json!({
        "metadata": {
            "readiness_blocker": self.readiness_blocker(),
        },
    });
}

#[test]
fn route_readiness_blockers_project_frontend_recovery_action() {
    assert_eq!(summary["readiness_blocker"]["frontend_action"], json!("retry_session"));
    assert_eq!(transports[0]["metadata"]["readiness_blocker"], summary["readiness_blocker"]);
}
RS

cat >"$SB/plugins/remote-desktop/src/media/linux_process_tree_audio.rs" <<'RS'
struct LinuxProcessTreeAudioBackend;
fn install_revalidation_timer() { main_loop.loop_().add_timer(|_| {}); }

#[test]
fn process_tree_includes_all_descendants_and_excludes_unrelated_processes() {}

#[test]
fn audio_node_selection_includes_every_node_in_the_process_tree() {}

#[test]
fn reused_root_pid_fails_closed_instead_of_selecting_new_process_tree() {}

#[test]
fn empty_authority_set_revokes_all_previously_eligible_nodes() {}

#[test]
fn contradictory_node_and_client_pid_identity_fails_closed_in_both_directions() {}
RS

cat >"$SB/plugins/remote-desktop/src/sdp.rs" <<'RS'
#[test]
fn signaling_rejects_oversized_sdp_and_ice_rows() {}
RS

cat >"$SB/plugins/remote-desktop/src/target.rs" <<'RS'
#[test]
fn resolver_refuses_to_run_while_session_store_lock_is_held() {}
fn resolve_for_session() {
    assert_current_thread_unlocked("remote_desktop.target.resolve_for_session");
}
RS

cat >"$SB/plugins/remote-desktop/src/transport/webrtc_endpoint.rs" <<'RS'
fn configure_transport() {
    let registry = configure_twcc_sender_only(registry, &mut media_engine)?;
}
fn start_direct_webrtc_endpoint(transports: RemoteDesktopTransportManager) -> anyhow::Result<()> {
    let build = transports.block_on(create_direct_webrtc_endpoint(DirectWebRtcEndpointConfig {
        session_id,
    }))?;
    let (answer, peer_connection, completion) = match build {
        Ok(endpoint) => endpoint,
        Err(error) => return Err(error),
    };
    let transport_runtime = endpoint_config.transports.runtime_handle()?;
    std::thread::Builder::new()
        .name("easynet-remote-desktop-webrtc".into())
        .spawn(move || {
            transport_runtime.block_on(run_direct_webrtc_media_loop());
        })?;
    Ok(())
}

#[test]
fn endpoint_start_boundary_refuses_to_run_while_session_store_lock_is_held() {}
RS

cat >"$SB/plugins/remote-desktop/src/transport/webrtc_media.rs" <<'RS'
struct DirectWebRtcSession {
    video_sender: Arc<dyn RtpSender>,
}
RS

cat >"$SB/plugins/remote-desktop/src/transport/webrtc_sender_feedback.rs" <<'RS'
fn observe(entry: RTCStatsReportEntry) {
    if let RTCStatsReportEntry::RemoteInboundRtp(stats) = entry {
        let measurements = stats.round_trip_time_measurements;
        if sample.measurements <= self.last_measurements { return; }
    }
}
RS

cat >"$SB/plugins/remote-desktop/src/transport/manager.rs" <<'RS'
pub(in crate::daemon::plugins::remote_desktop) fn block_on<F: Future>(
    &self,
    future: F,
) -> anyhow::Result<F::Output> {
    Ok(self.runtime_handle()?.block_on(future))
}

fn runtime_handle(&self) -> anyhow::Result<Handle> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("easynet-webrtc-runtime")
        .enable_all()
        .build()
        .map_err(|error| anyhow::anyhow!("build RemoteApp WebRTC runtime: {error}"))?;
    Ok(handle)
}
RS

cat >"$SB/plugins/remote-desktop/src/input.rs" <<'RS'
const MAX_INPUT_REJECTION_DIAGNOSTIC_SAMPLES_PER_SIGNATURE: u64 = 8;

fn observe() {
    if emitted_diagnostic_samples < MAX_INPUT_REJECTION_DIAGNOSTIC_SAMPLES_PER_SIGNATURE {
        let _ = "diagnostic_sample_limit";
    }
}

#[test]
fn input_reject_diagnostics_are_coalesced_under_high_rate_storm() {
    const REJECT_STORM: u64 = 10_000;
    assert_eq!(emitted.len(), MAX_INPUT_REJECTION_DIAGNOSTIC_SAMPLES_PER_SIGNATURE + 1, "sample cap plus flush summary");
    let _ = "coalesced_rejections";
}
RS

cat >"$SB/tests/script_checks.rs" <<'RS'
fn remoteapp_performance_boundary_script_holds() {}
RS

(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/dev/null || fail "happy path should pass"

perl -0pi -e 's/RemoteDesktopNegotiatedMediaScope::from_local_answer/derive_scope_from_host_capability/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-negotiated-scope-source.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "host-capability-derived negotiated media scope should exit 1 (got $rc)"
grep -q "committed local WebRTC answer" /tmp/check-remoteapp-performance-boundary-negotiated-scope-source.out || fail "expected negotiated media scope authority failure"

perl -0pi -e 's/derive_scope_from_host_capability/RemoteDesktopNegotiatedMediaScope::from_local_answer/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"
perl -0pi -e 's/"media_scope": negotiated_media_scope\.map\(\|scope\| scope\.as_str\(\)\)\.unwrap_or\("not_negotiated"\)/"media_scope": if audio_support["supported"] { "audio_video" } else { "video_only" }/' \
  "$SB/plugins/remote-desktop/src/view.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-capability-scope.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "capability-projected session media scope should exit 1 (got $rc)"
grep -q "project negotiated scope" /tmp/check-remoteapp-performance-boundary-capability-scope.out || fail "expected negotiated media scope projection failure"

perl -0pi -e 's/"media_scope": if audio_support\["supported"\] \{ "audio_video" \} else \{ "video_only" \}/"media_scope": negotiated_media_scope.map(|scope| scope.as_str()).unwrap_or("not_negotiated")/' \
  "$SB/plugins/remote-desktop/src/view.rs"

perl -0pi -e 's/let audio_ready = session\.audio_operational_ready\(\)/let audio_ready = false; \/\/ session-owned predicate removed/' \
  "$SB/plugins/remote-desktop/src/view.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-audio-ready.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "hard-coded host-audio readiness should exit 1 (got $rc)"
grep -q "session-owned operational audio predicate" /tmp/check-remoteapp-performance-boundary-audio-ready.out || fail "expected session-owned audio readiness failure"

perl -0pi -e 's/let audio_ready = false; \/\/ session-owned predicate removed/let audio_ready = session.audio_operational_ready()/' \
  "$SB/plugins/remote-desktop/src/view.rs"
perl -0pi -e 's/process_tree_includes_all_descendants_and_excludes_unrelated_processes/process_tree_ignores_descendants/g' \
  "$SB/plugins/remote-desktop/src/media/linux_process_tree_audio.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-audio-reason.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing Linux process-tree selection proof should exit 1 (got $rc)"
grep -q "test root and descendant process selection" /tmp/check-remoteapp-performance-boundary-audio-reason.out || fail "expected Linux process-tree selection failure"

perl -0pi -e 's/process_tree_ignores_descendants/process_tree_includes_all_descendants_and_excludes_unrelated_processes/g' \
  "$SB/plugins/remote-desktop/src/media/linux_process_tree_audio.rs"
perl -0pi -e 's/\.add_timer/\.graph_events_only/g' \
  "$SB/plugins/remote-desktop/src/media/linux_process_tree_audio.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-audio-revalidation.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing Linux process-authority revalidation should exit 1 (got $rc)"
grep -q "revalidate process authority" /tmp/check-remoteapp-performance-boundary-audio-revalidation.out || fail "expected Linux process-authority revalidation failure"

perl -0pi -e 's/\.graph_events_only/\.add_timer/g' \
  "$SB/plugins/remote-desktop/src/media/linux_process_tree_audio.rs"
perl -0pi -e 's/session_view_separates_platform_audio_capability_from_unnegotiated_scope/session_view_conflates_audio_capability_and_scope/' \
  "$SB/plugins/remote-desktop/src/view.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-audio-test.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing session audio product-state regression should exit 1 (got $rc)"
grep -q "separate platform capability from session negotiation" /tmp/check-remoteapp-performance-boundary-audio-test.out || fail "expected session audio regression failure"

perl -0pi -e 's/session_view_conflates_audio_capability_and_scope/session_view_separates_platform_audio_capability_from_unnegotiated_scope/' \
  "$SB/plugins/remote-desktop/src/view.rs"

perl -0pi -e 's/"product_ready": false/"product_ready": true/' \
  "$SB/plugins/remote-desktop/src/view_device.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-media-ready.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "misreported media product readiness should exit 1 (got $rc)"
grep -q "must not report full audio/video product readiness" /tmp/check-remoteapp-performance-boundary-media-ready.out || fail "expected media product readiness failure"

perl -0pi -e 's/"product_ready": true/"product_ready": false/' \
  "$SB/plugins/remote-desktop/src/view_device.rs"
perl -0pi -e 's/device_capabilities_project_media_pipeline_support_matrix/device_capabilities_omits_media_pipeline_support_matrix/' \
  "$SB/plugins/remote-desktop/src/view_device.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-media-test.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing media pipeline support regression should exit 1 (got $rc)"
grep -q "must pin media pipeline support projection" /tmp/check-remoteapp-performance-boundary-media-test.out || fail "expected media pipeline support regression failure"

perl -0pi -e 's/device_capabilities_omits_media_pipeline_support_matrix/device_capabilities_project_media_pipeline_support_matrix/' \
  "$SB/plugins/remote-desktop/src/view_device.rs"

perl -0pi -e 's/Reserved\(DirectWebRtcEndpoint\)/Reserved/' \
  "$SB/plugins/remote-desktop/src/handlers/add_ice_candidate.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-ice-admission-type.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "untyped ICE admission should exit 1 (got $rc)"
grep -q "reserved endpoint snapshot" /tmp/check-remoteapp-performance-boundary-ice-admission-type.out || fail "expected ICE admission endpoint snapshot failure"

perl -0pi -e 's/Reserved/Reserved(DirectWebRtcEndpoint)/' \
  "$SB/plugins/remote-desktop/src/handlers/add_ice_candidate.rs"
perl -0pi -e 's/let endpoint = match admission/let endpoint = endpoint.expect("reserved admission requires an active endpoint");\n    let endpoint = match admission/' \
  "$SB/plugins/remote-desktop/src/handlers/add_ice_candidate.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-ice-admission-expect.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "expect-based ICE admission proof should exit 1 (got $rc)"
grep -q "must not prove endpoint presence with expect" /tmp/check-remoteapp-performance-boundary-ice-admission-expect.out || fail "expected ICE admission expect failure"

perl -0pi -e 's/    let endpoint = endpoint\.expect\("reserved admission requires an active endpoint"\);\n//' \
  "$SB/plugins/remote-desktop/src/handlers/add_ice_candidate.rs"

perl -0pi -e 's/const WINDOW_COUNT: usize = 2_000;/const WINDOW_COUNT: usize = 200;/' \
  "$SB/src/daemon/ability/builtins/resources/media/resource_bootstrap.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-window.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "weakened PERF-01 window scale should exit 1 (got $rc)"
grep -q "W=2k windows" /tmp/check-remoteapp-performance-boundary-window.out || fail "expected PERF-01 window failure"

perl -0pi -e 's/const WINDOW_COUNT: usize = 200;/const WINDOW_COUNT: usize = 2_000;/' \
  "$SB/src/daemon/ability/builtins/resources/media/resource_bootstrap.rs"
perl -0pi -e 's/meta_list_resources_is_read_only_cache_projection/meta_list_resources_can_refresh_cache/' \
  "$SB/src/daemon/ability/builtins/resources/list.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-meta.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing PERF-02 read-only proof should exit 1 (got $rc)"
grep -q "PERF-02" /tmp/check-remoteapp-performance-boundary-meta.out || fail "expected PERF-02 failure"

perl -0pi -e 's/meta_list_resources_can_refresh_cache/meta_list_resources_is_read_only_cache_projection/' \
  "$SB/src/daemon/ability/builtins/resources/list.rs"
perl -0pi -e 's/const EVENT_TARGET_INVENTORY_UNAVAILABLE: &str = "target_inventory_unavailable";//' \
  "$SB/src/daemon/ability/builtins/resources/watch_remote_targets.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-watch-unavailable-event.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing watch unavailable event should exit 1 (got $rc)"
grep -Eq "typed inventory-unavailable event|target_inventory_unavailable" \
  /tmp/check-remoteapp-performance-boundary-watch-unavailable-event.out || fail "expected watch unavailable event failure"

perl -0pi -e 's#^#const EVENT_TARGET_INVENTORY_UNAVAILABLE: \&str = "target_inventory_unavailable";\n#' \
  "$SB/src/daemon/ability/builtins/resources/watch_remote_targets.rs"
perl -0pi -e 's/removed_resource_uras: Vec::new\(\)/removed_resource_uras: previous_removed_resource_uras/' \
  "$SB/src/daemon/ability/builtins/resources/watch_remote_targets.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-watch-unavailable-removal.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "unavailable watch removal projection should exit 1 (got $rc)"
grep -q "must not report previous targets as removed" /tmp/check-remoteapp-performance-boundary-watch-unavailable-removal.out || fail "expected watch unavailable removal failure"

perl -0pi -e 's/removed_resource_uras: previous_removed_resource_uras/removed_resource_uras: Vec::new()/' \
  "$SB/src/daemon/ability/builtins/resources/watch_remote_targets.rs"
perl -0pi -e 's/\n#\[test\]\nfn watch_input_schema_has_single_types_description_contract\(\) \{\}\n//' \
  "$SB/src/daemon/ability/builtins/resources/watch_remote_targets.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-watch-schema-source.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing watch schema-source contract should exit 1 (got $rc)"
grep -q "descriptor schema source does not duplicate types.description" /tmp/check-remoteapp-performance-boundary-watch-schema-source.out || fail "expected watch schema-source failure"

cat >>"$SB/src/daemon/ability/builtins/resources/watch_remote_targets.rs" <<'RS'

#[test]
fn watch_input_schema_has_single_types_description_contract() {}
RS
perl -0pi -e 's/const SESSION_COUNT: usize = 128;/const SESSION_COUNT: usize = 8;/' \
  "$SB/plugins/remote-desktop/src/target_observer.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-fanout.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "weakened PERF-03 session fanout should exit 1 (got $rc)"
grep -q "S=128 active session ticks" /tmp/check-remoteapp-performance-boundary-fanout.out || fail "expected PERF-03 fanout failure"

perl -0pi -e 's/const SESSION_COUNT: usize = 8;/const SESSION_COUNT: usize = 128;/' \
  "$SB/plugins/remote-desktop/src/target_observer.rs"
perl -0pi -e 's/let provider = snapshot_executor\.sample_for_generation\(generation, provider_deadline\)\?;/let provider = PlatformTargetObservationProvider;/' \
  "$SB/plugins/remote-desktop/src/target_monitor.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-production-sampler.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "target monitor per-session platform observer should exit 1 (got $rc)"
grep -q "generation-scoped host sample" /tmp/check-remoteapp-performance-boundary-production-sampler.out || fail "expected PERF-03 target monitor sampler failure"

perl -0pi -e 's/let provider = PlatformTargetObservationProvider;/let provider = snapshot_executor.sample_for_generation(generation, provider_deadline)?;/' \
  "$SB/plugins/remote-desktop/src/target_monitor.rs"
perl -0pi -e 's/fn event_replay_projects_compaction_before_retained_window/fn event_replay_silently_starts_at_retained_window/' \
  "$SB/plugins/remote-desktop/src/event_log.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-event-replay.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing PERF-04 compaction replay proof should exit 1 (got $rc)"
grep -q "compaction" /tmp/check-remoteapp-performance-boundary-event-replay.out || fail "expected PERF-04 compaction replay failure"

perl -0pi -e 's/fn event_replay_silently_starts_at_retained_window/fn event_replay_projects_compaction_before_retained_window/' \
  "$SB/plugins/remote-desktop/src/event_log.rs"
perl -0pi -e 's/optional_u64_field\(&args, "from_sequence", ABILITY_WATCH_EVENTS\)\?\.unwrap_or\(0\)/args.get("from_sequence").and_then(Value::as_u64).unwrap_or(0)/' \
  "$SB/plugins/remote-desktop/src/handlers/watch_events.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-watch-events-parser.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "handler-side malformed replay cursor parsing should exit 1 (got $rc)"
grep -q "typed request parser" /tmp/check-remoteapp-performance-boundary-watch-events-parser.out || fail "expected watch_events typed parser failure"

perl -0pi -e 's/args\.get\("from_sequence"\)\.and_then\(Value::as_u64\)\.unwrap_or\(0\)/optional_u64_field(\&args, "from_sequence", ABILITY_WATCH_EVENTS)?.unwrap_or(0)/' \
  "$SB/plugins/remote-desktop/src/handlers/watch_events.rs"
perl -0pi -e 's/pub\(in crate::daemon::plugins::remote_desktop\) fn optional_u64_field/fn optional_u64_field/' \
  "$SB/plugins/remote-desktop/src/request.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-u64-parser-visibility.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "non-shared replay cursor parser should exit 1 (got $rc)"
grep -q "typed optional u64 parsing" /tmp/check-remoteapp-performance-boundary-u64-parser-visibility.out || fail "expected optional_u64_field visibility failure"

perl -0pi -e 's/fn optional_u64_field/pub(in crate::daemon::plugins::remote_desktop) fn optional_u64_field/' \
  "$SB/plugins/remote-desktop/src/request.rs"
perl -0pi -e 's/watch_events_rejects_malformed_replay_cursor/watch_events_accepts_malformed_replay_cursor/' \
  "$SB/plugins/remote-desktop/src/handlers/mod.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-watch-events-test.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing malformed replay cursor regression test should exit 1 (got $rc)"
grep -q "malformed replay cursors" /tmp/check-remoteapp-performance-boundary-watch-events-test.out || fail "expected watch_events malformed cursor test failure"

perl -0pi -e 's/watch_events_accepts_malformed_replay_cursor/watch_events_rejects_malformed_replay_cursor/' \
  "$SB/plugins/remote-desktop/src/handlers/mod.rs"
perl -0pi -e 's/fn to_bounded_view/fn to_unbounded_view/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-signaling-view.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing bounded signaling view should exit 1 (got $rc)"
grep -q "bounded public view projection" /tmp/check-remoteapp-performance-boundary-signaling-view.out || fail "expected PERF-05 bounded signaling view failure"

perl -0pi -e 's/fn to_unbounded_view/fn to_bounded_view/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"
perl -0pi -e 's/fn signaling_state_validates_local_and_remote_ice_rows_before_storage/fn signaling_state_skips_ice_row_validation/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-ice-row-validation.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing signaling ICE row validation should exit 1 (got $rc)"
grep -q "validate local and remote ICE rows before storage" /tmp/check-remoteapp-performance-boundary-ice-row-validation.out || fail "expected PERF-05 ICE row validation failure"

perl -0pi -e 's/fn signaling_state_skips_ice_row_validation/fn signaling_state_validates_local_and_remote_ice_rows_before_storage/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"
perl -0pi -e 's/fn signaling_state_rejects_oversized_descriptions_before_storage/fn signaling_state_accepts_oversized_descriptions/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-description-validation-test.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing signaling description-size validation test should exit 1 (got $rc)"
grep -q "oversized local and remote descriptions" /tmp/check-remoteapp-performance-boundary-description-validation-test.out || fail "expected PERF-05 signaling description validation test failure"

perl -0pi -e 's/fn signaling_state_accepts_oversized_descriptions/fn signaling_state_rejects_oversized_descriptions_before_storage/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"
perl -0pi -e 's/validate_signaling_description_size\(&value\)\?;//' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-description-domain.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing signaling domain description-size validation should exit 1 (got $rc)"
grep -q "signaling description byte limits" /tmp/check-remoteapp-performance-boundary-description-domain.out || fail "expected PERF-05 signaling description domain validation failure"

perl -0pi -e 's/(fn new\(value: Value\) -> anyhow::Result<Self> \{\n)/$1        validate_signaling_description_size(\x26value)?;\n/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"
perl -0pi -e 's/RemoteDesktopSessionDescription::new\(answer\)\?;/RemoteDesktopSessionDescription::new("local", answer).expect("literal side");/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-local-answer-expect.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "local WebRTC answer expect path should exit 1 (got $rc)"
grep -q "generated local WebRTC answers" /tmp/check-remoteapp-performance-boundary-local-answer-expect.out || fail "expected PERF-05 local answer expect failure"

perl -0pi -e 's/RemoteDesktopSessionDescription::new\("local", answer\)\.expect\("literal side"\);/RemoteDesktopSessionDescription::new(answer)?;/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"
perl -0pi -e 's/"remote_ice_candidates_elided": true/"remote_ice_candidates": []/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-remote-elision.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing remote candidate elision should exit 1 (got $rc)"
grep -q "remote ICE candidate rows elided" /tmp/check-remoteapp-performance-boundary-remote-elision.out || fail "expected PERF-05 remote elision failure"

perl -0pi -e 's/"remote_ice_candidates": \[\]/"remote_ice_candidates_elided": true/' \
  "$SB/plugins/remote-desktop/src/session_signaling.rs"
perl -0pi -e 's/let _ = session\.signaling_view\(transport_route_state\.clone\(\)\);/let remote_ice_candidates = session.remote_ice_candidates();/' \
  "$SB/plugins/remote-desktop/src/view.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-manual-view.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "manual signaling view projection should exit 1 (got $rc)"
grep -q "bounded signaling projection" /tmp/check-remoteapp-performance-boundary-manual-view.out || fail "expected PERF-05 manual view projection failure"

perl -0pi -e 's/let remote_ice_candidates = session\.remote_ice_candidates\(\);/let _ = session.signaling_view(transport_route_state.clone());/' \
  "$SB/plugins/remote-desktop/src/view.rs"
perl -0pi -e 's/production_readiness_reports_client_blocker_and_route_degradation_before_presentation/production_readiness_reports_client_before_route/' \
  "$SB/plugins/remote-desktop/src/session.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-route-before-client.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing route-before-client readiness regression should exit 1 (got $rc)"
grep -q "client presentation blockers while preserving route degradation evidence" /tmp/check-remoteapp-performance-boundary-route-before-client.out || fail "expected PERF-05 route-degradation evidence failure"

perl -0pi -e 's/production_readiness_reports_client_before_route/production_readiness_reports_client_blocker_and_route_degradation_before_presentation/' \
  "$SB/plugins/remote-desktop/src/session.rs"
perl -0pi -e 's/view\["production_readiness"\]\["route_readiness_blocker"\]\["frontend_action"\]/view["production_readiness"]["route_readiness_blocker"]["missing_frontend_action"]/' \
  "$SB/plugins/remote-desktop/src/session.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-readiness-action.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing production readiness blocker action assertion should exit 1 (got $rc)"
grep -q "frontend recovery action" /tmp/check-remoteapp-performance-boundary-readiness-action.out || fail "expected readiness blocker action failure"

perl -0pi -e 's/view\["production_readiness"\]\["route_readiness_blocker"\]\["missing_frontend_action"\]/view["production_readiness"]["route_readiness_blocker"]["frontend_action"]/' \
  "$SB/plugins/remote-desktop/src/session.rs"

perl -0pi -e 's/anyhow::Result<F::Output>/F::Output/' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-runtime-result.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "non-fallible direct WebRTC runtime boundary should exit 1 (got $rc)"
grep -q "must propagate runtime initialization failure" /tmp/check-remoteapp-performance-boundary-runtime-result.out || fail "expected direct runtime fallibility failure"

perl -0pi -e 's/F::Output/anyhow::Result<F::Output>/' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"
perl -0pi -e 's/(fn runtime_handle\(&self\) -> anyhow::Result<Handle> \{\n)/$1    expect("build RemoteApp WebRTC runtime");\n/' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-runtime-expect.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "expect-based direct WebRTC runtime boundary should exit 1 (got $rc)"
grep -q "runtime initialization must not panic" /tmp/check-remoteapp-performance-boundary-runtime-expect.out || fail "expected direct runtime expect failure"

perl -0pi -e 's/    expect\("build RemoteApp WebRTC runtime"\);\n//' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"

perl -0pi -e 's/const MAX_INPUT_REJECTION_DIAGNOSTIC_SAMPLES_PER_SIGNATURE: u64 = 8;//' \
  "$SB/plugins/remote-desktop/src/input.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-input-cap.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing input reject sample cap should exit 1 (got $rc)"
grep -q "hard sample cap" /tmp/check-remoteapp-performance-boundary-input-cap.out || fail "expected PERF-07 sample cap failure"

perl -0pi -e 's/fn observe\(\) \{/const MAX_INPUT_REJECTION_DIAGNOSTIC_SAMPLES_PER_SIGNATURE: u64 = 8;\nfn observe() {/' \
  "$SB/plugins/remote-desktop/src/input.rs"
perl -0pi -e 's/emitted_diagnostic_samples < MAX_INPUT_REJECTION_DIAGNOSTIC_SAMPLES_PER_SIGNATURE/emitted_diagnostic_samples > 0/' \
  "$SB/plugins/remote-desktop/src/input.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-input-enforce.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing input reject sample cap enforcement should exit 1 (got $rc)"
grep -q "enforce the hard sample cap" /tmp/check-remoteapp-performance-boundary-input-enforce.out || fail "expected PERF-07 cap enforcement failure"

perl -0pi -e 's/emitted_diagnostic_samples > 0/emitted_diagnostic_samples < MAX_INPUT_REJECTION_DIAGNOSTIC_SAMPLES_PER_SIGNATURE/' \
  "$SB/plugins/remote-desktop/src/input.rs"

printf '%s\n' 'struct ClientMediaFeedback { available_outgoing_bitrate_bps: Option<f64> }' >> \
  "$SB/plugins/remote-desktop/src/session_transport_state.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-bandwidth-direction.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "reverse-direction Browser bandwidth authority should exit 1 (got $rc)"
grep -q "Browser outgoing/upload" /tmp/check-remoteapp-performance-boundary-bandwidth-direction.out || fail "expected PERF-08 bandwidth direction failure"

perl -ni -e 'print unless /available_outgoing_bitrate_bps/' \
  "$SB/plugins/remote-desktop/src/session_transport_state.rs"
perl -0pi -e 's/let payload = Bytes::from_owner\(transport_pool\.copy_from_slice\(payload_view\)\);/let payload = frame.slice(0..payload_view.len());/' \
  "$SB/plugins/remote-desktop/src/native_host_process.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-shared-lease.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "retaining a shared media slot into WebRTC should exit 1 (got $rc)"
grep -q "transport-owned bytes" /tmp/check-remoteapp-performance-boundary-shared-lease.out || fail "expected PERF-08 transport ownership failure"

perl -0pi -e 's/let payload = frame\.slice\(0\.\.payload_view\.len\(\)\);/let payload = Bytes::from_owner(transport_pool.copy_from_slice(payload_view));/' \
  "$SB/plugins/remote-desktop/src/native_host_process.rs"

perl -0pi -e 's/configure_twcc_sender_only/configure_twcc_receiver_only/' \
  "$SB/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-twcc-direction.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "receiver-only TWCC on a send-only endpoint should exit 1 (got $rc)"
grep -q "TWCC in the sender direction" /tmp/check-remoteapp-performance-boundary-twcc-direction.out || fail "expected PERF-08 TWCC direction failure"

perl -0pi -e 's/configure_twcc_receiver_only/configure_twcc_sender_only/' \
  "$SB/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"

perl -0pi -e 's/writer_service_time_independently_bounds_frame_rate/writer_service_rate_is_not_tested/' \
  "$SB/plugins/remote-desktop/src/media/adaptation.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-writer-service.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing writer-service FPS proof should exit 1 (got $rc)"
grep -q "slow writer lowers FPS" /tmp/check-remoteapp-performance-boundary-writer-service.out || fail "expected PERF-08 writer-service proof failure"

perl -0pi -e 's/writer_service_rate_is_not_tested/writer_service_time_independently_bounds_frame_rate/' \
  "$SB/plugins/remote-desktop/src/media/adaptation.rs"

perl -0pi -e 's/mpsc::sync_channel\(1\)/mpsc::channel()/' \
  "$SB/plugins/remote-desktop/src/media/host_audio_capability.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-host-audio-queue.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "unbounded host-audio wake queue should exit 1 (got $rc)"
grep -q "capacity one\|unbounded command queue" /tmp/check-remoteapp-performance-boundary-host-audio-queue.out || fail "expected bounded host-audio queue failure"

perl -0pi -e 's/mpsc::channel\(\)/mpsc::sync_channel(1)/' \
  "$SB/plugins/remote-desktop/src/media/host_audio_capability.rs"
perl -0pi -e 's/native_probe_projection_keeps_source_readiness_independent/native_probe_projection_conflates_source_readiness/' \
  "$SB/plugins/remote-desktop/src/media/host_audio_capability.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-windows-process-audio.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing independent native source-readiness projection proof should exit 1 (got $rc)"
grep -q "native source readiness must remain independent" /tmp/check-remoteapp-performance-boundary-windows-process-audio.out || fail "expected native source-readiness independence failure"

printf 'test_check_remoteapp_performance_boundary.sh: all cases passed\n'
