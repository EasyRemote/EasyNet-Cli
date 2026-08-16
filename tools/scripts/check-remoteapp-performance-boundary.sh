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
  if ! rg -n "$pattern" "$file" >/dev/null; then
    fail "$message"
  fi
}

reject() {
  local pattern="$1"
  local file="$2"
  local message="$3"
  [[ -f "$file" ]] || fail "missing $file"
  if rg -n "$pattern" "$file" >/dev/null; then
    fail "$message"
  fi
}

SPEC="$ROOT/docs/design/remoteapp-targeted-session-spec.md"
RESOURCE_BOOTSTRAP="$ROOT/src/daemon/ability/builtins/resources/media/resource_bootstrap.rs"
RESOURCE_LIST="$ROOT/src/daemon/ability/builtins/resources/list.rs"
TARGET_OBSERVER="$ROOT/plugins/remote-desktop/src/target_observer.rs"
EVENT_LOG="$ROOT/plugins/remote-desktop/src/event_log.rs"
REQUEST="$ROOT/plugins/remote-desktop/src/request.rs"
HANDLERS="$ROOT/plugins/remote-desktop/src/handlers/mod.rs"
WATCH_EVENTS="$ROOT/plugins/remote-desktop/src/handlers/watch_events.rs"
SESSION_SIGNALING="$ROOT/plugins/remote-desktop/src/session_signaling.rs"
SESSION="$ROOT/plugins/remote-desktop/src/session.rs"
SESSION_STORE="$ROOT/plugins/remote-desktop/src/session_store.rs"
VIEW="$ROOT/plugins/remote-desktop/src/view.rs"
VIEW_TRANSPORT="$ROOT/plugins/remote-desktop/src/view_transport.rs"
SDP="$ROOT/plugins/remote-desktop/src/sdp.rs"
TARGET="$ROOT/plugins/remote-desktop/src/target.rs"
WEBRTC_ENDPOINT="$ROOT/plugins/remote-desktop/src/transport/webrtc_endpoint.rs"
INPUT="$ROOT/plugins/remote-desktop/src/input.rs"
SCRIPT_CHECKS="$ROOT/tests/script_checks.rs"

for checkpoint in PERF-01 PERF-02 PERF-03 PERF-04 PERF-05 PERF-06 PERF-07; do
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

require 'shared_host_snapshot_provider_coalesces_session_observer_reads' "$TARGET_OBSERVER" \
  'PERF-03 must prove shared target sampling coalesces host enumeration'
require 'shared_host_snapshot_provider_bounds_session_fanout_to_one_enumeration_per_tick' "$TARGET_OBSERVER" \
  'PERF-03 must prove 128 session ticks share one host snapshot per refresh window'
require 'static SNAPSHOTS: OnceLock' "$TARGET_OBSERVER" \
  'PERF-03 production platform observer must own a shared host snapshot cache'
require 'SharedHostTargetSnapshotProvider::new' "$TARGET_OBSERVER" \
  'PERF-03 production platform observer must construct the shared host snapshot provider'
require 'PLATFORM_TARGET_SNAPSHOT_MIN_REFRESH' "$TARGET_OBSERVER" \
  'PERF-03 production platform observer must use the bounded target snapshot refresh window'
require 'SnapshotBackedTargetObservationProvider::new\(snapshots\)\.observe\(binding, snapshot\)' "$TARGET_OBSERVER" \
  'PERF-03 production platform observer must observe through the shared snapshot-backed provider'
require 'const SESSION_COUNT: usize = 128' "$TARGET_OBSERVER" \
  'PERF-03 shared sampler test must cover S=128 active session ticks'
require 'Duration::ZERO' "$TARGET_OBSERVER" \
  'PERF-03 shared sampler test must prove cache expiry permits a new bounded enumeration'
require 'calls\.load\(Ordering::SeqCst\)' "$TARGET_OBSERVER" \
  'PERF-03 must inspect the host snapshot call count'
require 'shared target observer must not multiply OS enumeration by session count' "$TARGET_OBSERVER" \
  'PERF-03 must assert one host snapshot call per shared sampler tick'
require 'one host enumeration for 128 session ticks' "$TARGET_OBSERVER" \
  'PERF-03 must assert fanout is bounded to one enumeration for 128 session ticks'

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

require 'remoteapp_performance_boundary_script_holds' "$SCRIPT_CHECKS" \
  'remoteapp performance boundary must be wired into cargo script_checks'

printf 'check-remoteapp-performance-boundary: ok\n'
