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
fn handle(session: Session) {
    let _ = session.replay_events_from(0);
}
RS

cat >"$SB/plugins/remote-desktop/src/session_signaling.rs" <<'RS'
fn to_bounded_view() {
    "signaling_limits";
    "remote_ice_candidates_elided": true;
}

#[test]
fn remote_desktop_signaling_rejects_more_than_ten_thousand_candidates_without_growth() {}

#[test]
fn remote_desktop_signaling_bounded_view_projects_counts_and_limits() {}
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

cat >"$SB/plugins/remote-desktop/src/view.rs" <<'RS'
fn serialize_session(session: Session, transport_route_state: Value) {
    let _ = session.signaling_view(transport_route_state.clone());
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
