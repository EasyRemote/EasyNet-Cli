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

cat >"$SB/plugins/remote-desktop/src/target_observer.rs" <<'RS'
const PLATFORM_TARGET_SNAPSHOT_MIN_REFRESH: Duration = Duration::from_millis(250);
static SNAPSHOTS: OnceLock<SharedHostTargetSnapshotProvider<MacOsHostTargetSnapshotProvider>> = OnceLock::new();

impl TargetObservationProvider for PlatformTargetObservationProvider {
    fn observe(&self, binding: &RemoteAppTargetBinding, snapshot: &TargetTrackerSnapshot) -> Option<TargetObservation> {
        let snapshots = SNAPSHOTS.get_or_init(|| {
            SharedHostTargetSnapshotProvider::new(
                MacOsHostTargetSnapshotProvider,
                PLATFORM_TARGET_SNAPSHOT_MIN_REFRESH,
            )
        });
        SnapshotBackedTargetObservationProvider::new(snapshots).observe(binding, snapshot)
    }
}

#[test]
fn shared_host_snapshot_provider_coalesces_session_observer_reads() {
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "shared target observer must not multiply OS enumeration by session count"
    );
}

#[test]
fn shared_host_snapshot_provider_bounds_session_fanout_to_one_enumeration_per_tick() {
    const SESSION_COUNT: usize = 128;
    let _ = Duration::ZERO;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "PERF-03 shared target sampler must use one host enumeration for 128 session ticks inside the same refresh window"
    );
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
#[test]
fn serialized_session_view_remains_bounded_at_signaling_limits() {
    let _ = "remote_ice_candidates_elided";
}
fn assert_current_thread_unlocked(stage: &str) {
    assert_eq!(0, 0, "{stage} must not run while RemoteDesktopSessionStore is locked");
}
RS

cat >"$SB/plugins/remote-desktop/src/session.rs" <<'RS'
#[test]
fn production_readiness_reports_route_blocker_before_client_presentation() {
    assert_eq!(
        view["production_readiness"]["readiness_blocker"]["frontend_action"],
        json!("retry_session")
    );
}
RS

cat >"$SB/plugins/remote-desktop/src/view.rs" <<'RS'
fn serialize_session(session: Session, transport_route_state: Value) {
    let _ = session.signaling_view(transport_route_state.clone());
    json!({
        "readiness_blocker": transport_view.readiness_blocker(),
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
#[test]
fn endpoint_start_boundary_refuses_to_run_while_session_store_lock_is_held() {}
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
perl -0pi -e 's/SnapshotBackedTargetObservationProvider::new\(snapshots\)\.observe\(binding, snapshot\)/MacOsHostTargetSnapshotProvider.snapshot().ok(); None/' \
  "$SB/plugins/remote-desktop/src/target_observer.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-production-sampler.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "production observer bypassing shared sampler should exit 1 (got $rc)"
grep -q "shared snapshot-backed provider" /tmp/check-remoteapp-performance-boundary-production-sampler.out || fail "expected PERF-03 production sampler failure"

perl -0pi -e 's/MacOsHostTargetSnapshotProvider\.snapshot\(\)\.ok\(\); None/SnapshotBackedTargetObservationProvider::new(snapshots).observe(binding, snapshot)/' \
  "$SB/plugins/remote-desktop/src/target_observer.rs"
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
perl -0pi -e 's/production_readiness_reports_route_blocker_before_client_presentation/production_readiness_reports_client_before_route/' \
  "$SB/plugins/remote-desktop/src/session.rs"

set +e
(
  cd "$SB"
  CHECK_REMOTEAPP_PERFORMANCE_BOUNDARY_ROOT="$SB" bash tools/scripts/check-remoteapp-performance-boundary.sh
) >/tmp/check-remoteapp-performance-boundary-route-before-client.out 2>&1
rc=$?
set -e
[[ "$rc" == "1" ]] || fail "missing route-before-client readiness regression should exit 1 (got $rc)"
grep -q "route blockers before client presentation blockers" /tmp/check-remoteapp-performance-boundary-route-before-client.out || fail "expected PERF-05 route-before-client failure"

perl -0pi -e 's/production_readiness_reports_client_before_route/production_readiness_reports_route_blocker_before_client_presentation/' \
  "$SB/plugins/remote-desktop/src/session.rs"
perl -0pi -e 's/view\["production_readiness"\]\["readiness_blocker"\]\["frontend_action"\]/view["production_readiness"]["readiness_blocker"]["missing_frontend_action"]/' \
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

cat >"$SB/plugins/remote-desktop/src/session.rs" <<'RS'
#[test]
fn production_readiness_reports_route_blocker_before_client_presentation() {
    assert_eq!(
        view["production_readiness"]["readiness_blocker"]["frontend_action"],
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
