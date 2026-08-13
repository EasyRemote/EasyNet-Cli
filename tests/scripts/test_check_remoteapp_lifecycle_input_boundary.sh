#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-lifecycle-input-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p "$SANDBOX/docs/design"
mkdir -p "$SANDBOX/plugins/remote-desktop/src/handlers"

write_fixture() {
  rm -rf "$SANDBOX/docs" "$SANDBOX/plugins"
  mkdir -p "$SANDBOX/docs/design"
  mkdir -p "$SANDBOX/plugins/remote-desktop/src/handlers"

  cat >"$SANDBOX/docs/design/remoteapp-targeted-session-spec.md" <<'MD'
| E2E-08 move/resize tracking | move and resize events advance target geometry revision and input consumes that revision |
| E2E-09 target loss vs transport failure | selected target loss emits target/media loss without transport failure |
| E2E-10 weak identity ambiguity | ambiguous weak native identity fails closed before stream start |
| E2E-11 view-only input safety | app/window sessions remain view-only without a focus-safe input validator |
MD

  cat >"$SANDBOX/plugins/remote-desktop/src/target_tracking.rs" <<'RS'
pub struct TargetTrackerSnapshot {
    pub target_geometry_revision: u64,
}

fn commit_geometry() {
    geometry_event_type();
}

fn geometry_event_type() -> &'static str {
    if moved() {
        "TARGET_MOVED"
    } else {
        "TARGET_RESIZED"
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tracker_commits_move_resize_and_lost_without_rebinding() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session.rs" <<'RS'
fn record_target_observation() {
    if event.event_type() == "TARGET_LOST" {
        self.lifecycle.mark_degraded();
        self.transport.mark_media_source_lost(epoch);
        session_events::media_source_lost();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn target_lost_stops_active_media_source_without_transport_failure() {
        assert!(target_lost_index < media_source_lost_index);
        assert_eq!(session.target_tracking_state()["input_enabled"], json!(false));
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_events.rs" <<'RS'
fn media_source_lost() {
    json!({
        "event_type": "MEDIA_SOURCE_LOST",
        "failure_domain": "target",
        "media_transport_ready": false,
    });
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/input.rs" <<'RS'
fn input_policy_for_target_snapshot() {
    let _ = policy["pointer_target"]["target_geometry_revision"];
}

fn current_input_policy() {
    let snapshot = session.target_snapshot();
    if !snapshot.input_enabled() {
        return None;
    }
    input_policy_for_target_snapshot(base_policy.clone(), snapshot);
}

fn input_policy_for_scope() {
    match scope {
        InputScope::ViewOnly => {
            disable_input_policy_key(map, "keyboard_enabled");
            disable_input_policy_key(map, "pointer_enabled");
        }
    }
}

fn input_policy_reject_reason() -> Option<&'static str> {
    Some("input_scope_unsupported")
}

fn record_rejection() {
    InputRejectSample::new(reason, rejected_count);
}

#[cfg(test)]
mod tests {
    #[test]
    fn pointer_policy_consumes_latest_target_tracker_snapshot() {
        let _ = policy["pointer_target"]["target_geometry_revision"];
    }

    #[test]
    fn maps_window_relative_pointer_to_global_screen_point() {
        assert!(!input_policy_allows(&policy, "pointer"));
    }

    #[test]
    fn maps_application_pointer_through_primary_window_bounds() {
        assert!(!input_policy_allows(&policy, "pointer"));
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/target.rs" <<'RS'
enum TargetResolutionError {
    TargetIdentityAmbiguous,
    TargetMetadataIncomplete,
}

const TARGET_IDENTITY_AMBIGUOUS: TargetResolutionError =
    TargetResolutionError::TargetIdentityAmbiguous;
const TARGET_METADATA_INCOMPLETE: TargetResolutionError =
    TargetResolutionError::TargetMetadataIncomplete;

fn target_identity_ambiguous() {}

fn input_scope_for_request() {
    match kind {
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
            // target-scoped keyboard/pointer dispatch is unsafe until a focus-safe validator exists.
            InputScope::ViewOnly
        }
        _ => InputScope::DisplayGlobal,
    }
}

fn validate_window() {
    return Err("window targets require owner pid, app_identity, or bundle_id");
}

fn validate_application() {
    return Err("application targets require primary_pid, app_identity, or bundle_id");
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_requires_stable_owner_identity_not_app_name_only() {}

    #[test]
    fn application_requires_display_scoped_stable_identity() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs" <<'RS'
fn handle() {
    let workflow = RemoteDesktopSessionCreationWorkflow::start(&env, &args)?
        .consume_consent(&registry, &env)?
        .resolve_target()?;
    let session = RemoteDesktopSession::new(workflow.into_session_init());
}

#[cfg(test)]
mod tests {
    #[test]
    fn create_session_rejects_weak_window_identity_before_session_insert() {
        assert!(err.to_string().contains("target_identity_ambiguous"));
        assert!(!sessions.contains_key("rd-weak-window"));
    }
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/session_creation.rs" <<'RS'
enum RemoteDesktopSessionCreationState {
    ReadyToInsert,
}

const READY_TO_INSERT: RemoteDesktopSessionCreationState =
    RemoteDesktopSessionCreationState::ReadyToInsert;
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/invoke_bidi.rs" <<'RS'
fn handle_bidi_input_frame() {
    input_policy_reject_reason(input_policy, kind);
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostic_bidi_view_only_input_reports_scope_unsupported() {}
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs" <<'RS'
fn select_window_for_binding() {
    Err(TargetResolutionError::TargetIdentityAmbiguous(
        "requested ScreenCaptureKit window identity is ambiguous",
    ))
}

fn select_application_for_binding() {
    Err(TargetResolutionError::TargetIdentityAmbiguous(
        "requested ScreenCaptureKit application identity is ambiguous",
    ))
}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/request.rs" <<'RS'
#[test]
fn input_policy_reports_clipboard_and_file_drop_unsupported_even_when_requested() {}
RS

  cat >"$SANDBOX/plugins/remote-desktop/src/target_observer.rs" <<'RS'
fn observe_bound_session_target_once() {
    record_target_observation_for_session();
}

#[cfg(test)]
mod tests {
    #[test]
    fn observation_provider_commits_through_session_store_boundary() {}

    #[test]
    fn stale_observation_cannot_commit_after_session_binding_reuse() {}

    #[test]
    fn lost_observation_returns_media_source_stop_effect_after_debounce() {}
}
RS
}

run_ok() {
  CHECK_REMOTEAPP_LIFECYCLE_INPUT_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null
}

run_fail() {
  local expected="$1"
  if CHECK_REMOTEAPP_LIFECYCLE_INPUT_ROOT="$SANDBOX" bash "$SCRIPT" >"$SANDBOX/out" 2>"$SANDBOX/err"; then
    printf 'expected failure containing %s\n' "$expected" >&2
    exit 1
  fi
  rg -q -- "$expected" "$SANDBOX/err" || {
    printf 'expected failure containing %s, got:\n' "$expected" >&2
    cat "$SANDBOX/err" >&2
    exit 1
  }
}

write_fixture
run_ok

write_fixture
perl -0pi -e 's/tracker_commits_move_resize_and_lost_without_rebinding/tracker_misses_regression/' \
  "$SANDBOX/plugins/remote-desktop/src/target_tracking.rs"
run_fail 'E2E-08 must have move/resize/lost tracker regression coverage'

write_fixture
perl -0pi -e 's/target_lost_index < media_source_lost_index/media_source_lost_index < target_lost_index/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'E2E-09 must prove TARGET_LOST is ordered before MEDIA_SOURCE_LOST'

write_fixture
perl -0pi -e 's/let snapshot = session\.target_snapshot\(\);/let snapshot = cached_snapshot;/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'production input path must read the latest session target snapshot'

write_fixture
perl -0pi -e 's/json!\(false\)/json!(true)/' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"
run_fail 'E2E-09 must assert target loss disables input'

write_fixture
perl -0pi -e 's/TargetResolutionError::TargetIdentityAmbiguous/TargetResolutionError::TargetNotFound/g' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"
run_fail 'ScreenCaptureKit binding must fail closed on native identity ambiguity'

write_fixture
perl -0pi -e 's/target_identity_ambiguous/target_metadata_incomplete/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"
run_fail 'weak target identity handler test must assert target_identity_ambiguous'

write_fixture
perl -0pi -e 's/disable_input_policy_key\(map, "pointer_enabled"\);/enable_input_policy_key(map, "pointer_enabled");/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'view-only input policy must disable pointer'

write_fixture
perl -0pi -e 's/Some\("input_scope_unsupported"\)/Some("input_disabled")/' \
  "$SANDBOX/plugins/remote-desktop/src/input.rs"
run_fail 'view-only key/pointer rejection must report input_scope_unsupported'

printf 'test_check_remoteapp_lifecycle_input_boundary.sh: all cases passed\n'
