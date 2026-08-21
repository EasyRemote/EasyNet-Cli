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
  "$SB/plugins/remote-desktop/src" \
  "$SB/tests"
cp "$SCRIPT" "$SB/tools/scripts/check-remoteapp-performance-boundary.sh"

cat >"$SB/docs/design/remoteapp-targeted-session-spec.md" <<'MD'
PERF-01 PERF-02 PERF-03 PERF-04 PERF-05 PERF-06 PERF-07
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
pub(in crate::daemon::plugins::remote_desktop) fn sample_platform_target_observations() {}

fn macos_sampler() {
    sample_host_target_observations(&MacOsHostTargetSnapshotProvider);
}

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

cat >"$SB/plugins/remote-desktop/src/target_monitor.rs" <<'RS'
fn poll_tracked_sessions() {
    let provider = sample_platform_target_observations();
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
impl RemoteDesktopSessionDescription {
    fn new(value: Value) -> anyhow::Result<Self> {
        validate_signaling_description_size(&value)?;
        Ok(Self)
    }
}

fn set_local_webrtc_answer(answer: Value) -> anyhow::Result<()> {
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

cat >"$SB/plugins/remote-desktop/src/view.rs" <<'RS'
fn serialize_session(session: Session, transport_route_state: Value) {
    let _ = session.signaling_view(transport_route_state.clone());
    json!({
        "route_readiness_blocker": transport_view.readiness_blocker(),
    });
}
RS

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
fn start_direct_webrtc_endpoint(transports: RemoteDesktopTransportManager) -> anyhow::Result<()> {
    transports.block_on(create_direct_webrtc_endpoint())??;
    std::thread::Builder::new()
        .name("easynet-remote-desktop-webrtc".into())
        .spawn(move || {
            if let Err(err) = transports.block_on(run_direct_webrtc_media_loop()) {
                eprintln!(
                    "[remote-desktop-webrtc] direct media loop runtime unavailable: {err}"
                );
            }
        })?;
    Ok(())
}

#[test]
fn endpoint_start_boundary_refuses_to_run_while_session_store_lock_is_held() {}
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
        .map_err(|err| anyhow::anyhow!("build remote desktop WebRTC runtime: {err}"))?;
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
perl -0pi -e 's/let provider = sample_platform_target_observations\(\);/let provider = PlatformTargetObservationProvider;/' \
  "$SB/plugins/remote-desktop/src/target_monitor.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-production-sampler.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "target monitor per-session platform observer should exit 1 (got $rc)"
grep -q "sample host state once" /tmp/check-remoteapp-performance-boundary-production-sampler.out || fail "expected PERF-03 target monitor sampler failure"

perl -0pi -e 's/let provider = PlatformTargetObservationProvider;/let provider = sample_platform_target_observations();/' \
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
perl -0pi -e 's/(fn runtime_handle\(&self\) -> anyhow::Result<Handle> \{\n)/$1    expect("build remote desktop WebRTC runtime");\n/' \
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

perl -0pi -e 's/    expect\("build remote desktop WebRTC runtime"\);\n//' \
  "$SB/plugins/remote-desktop/src/transport/manager.rs"

cat >"$SB/plugins/remote-desktop/src/session.rs" <<'RS'
#[test]
fn production_readiness_reports_client_blocker_and_route_degradation_before_presentation() {
    assert_eq!(
        view["production_readiness"]["route_readiness_blocker"]["frontend_action"],
        json!("retry_session")
    );
}
RS
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

printf 'test_check_remoteapp_performance_boundary.sh: all cases passed\n'
