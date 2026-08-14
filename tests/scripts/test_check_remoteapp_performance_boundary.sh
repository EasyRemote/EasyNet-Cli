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
RS

cat >"$SB/plugins/remote-desktop/src/event_log.rs" <<'RS'
#[test]
fn event_log_retains_fixed_ring_and_monotonic_sequences_under_large_storm() {
    let _ = 100_000;
}
RS

cat >"$SB/plugins/remote-desktop/src/session_signaling.rs" <<'RS'
#[test]
fn remote_desktop_signaling_rejects_more_than_ten_thousand_candidates_without_growth() {}
RS

cat >"$SB/plugins/remote-desktop/src/session_store.rs" <<'RS'
#[test]
fn serialized_session_view_remains_bounded_at_signaling_limits() {}
fn assert_current_thread_unlocked(stage: &str) {
    assert_eq!(0, 0, "{stage} must not run while RemoteDesktopSessionStore is locked");
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
#[test]
fn input_reject_diagnostics_are_coalesced_under_high_rate_storm() {
    const REJECT_STORM: u64 = 10_000;
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

printf 'test_check_remoteapp_performance_boundary.sh: all cases passed\n'
