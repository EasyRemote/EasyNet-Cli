#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-target-binding-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p "$SANDBOX/plugins/remote-desktop/src/handlers"
mkdir -p "$SANDBOX/plugins/remote-desktop/src/transport"

cat >"$SANDBOX/plugins/remote-desktop/src/target.rs" <<'RS'
const ALL_TARGET_RESOLUTION_ERRORS: &[TargetResolutionError] = &[
    TargetResolutionError::TargetNotFound,
];
const ALL_FRONTEND_ACTIONS: &[FrontendAction] = &[
    FrontendAction::RefreshTargets,
];
impl TargetResolutionError {
    fn frontend_action(self) -> FrontendAction {
        FrontendAction::RefreshTargets
    }
}
impl RemoteAppTargetError {
    fn to_axon(&self) -> AxonError {
        AxonError::new()
            .with_context("target_reason", self.reason.as_str())
            .with_context("frontend_action", self.reason.frontend_action().as_str())
    }
}
pub struct ResourceEntryTargetResolver;
impl RemoteDesktopTargetKind {
    fn target_model(self) -> &'static str {
        match self {
            Self::Application => "display_scoped_application_window_set",
            _ => "surface",
        }
    }
}

impl RemoteAppTargetBinding {
    fn to_value(&self) {
        json!({
            "target_model": self.target_kind.target_model(),
        });
    }

    fn target_bound_event_payload(&self) {
        json!({
            "target_model": self.target_kind.target_model(),
        });
    }
}

impl ScopeAudit {
    fn to_value(&self) {
        json!({
            "target_model": self.effective_target_kind.target_model(),
        });
    }
}

impl ResourceEntryTargetResolver {
    fn resolve_for_session(&self, target_kind: RemoteDesktopTargetKind) {
        validate_resource_inventory_state();
        metadata_freshness_u64();
        json!({
            "target_model": target_kind.target_model(),
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_target_resolution_reason_has_canonical_frontend_action_and_axon_context() {
        for reason in ALL_TARGET_RESOLUTION_ERRORS {
            assert!(ALL_FRONTEND_ACTIONS.contains(&reason.frontend_action()));
        }
    }
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs" <<'RS'
fn create_session() {
    RemoteDesktopSessionCreationWorkflow::start();
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/session_creation.rs" <<'RS'
fn creation_workflow() {
    ResourceEntryTargetResolver.resolve_for_session();
    verify_target_binding_for_session();
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/handlers/attach.rs" <<'RS'
fn attach(session: Session) {
    let binding = session.target_binding();
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_negotiation.rs" <<'RS'
fn negotiate(session: Session) {
    let binding = session.target_binding().clone();
    input_policy_for_binding();
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_native_media.rs" <<'RS'
fn media(binding: Binding) {
    target_for_binding();
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_media.rs" <<'RS'
fn run(binding: Binding, config: Config) {
    start_remote_app_media_source(&DirectWebRtcMediaSourceFactory, binding, MediaStartRequest { config });
}

fn project_failure(err: anyhow::Error) {
    err.downcast_ref::<RemoteAppTargetError>();
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/registration.rs" <<'RS'
fn classify_handler_result(err: anyhow::Error) {
    err.downcast_ref::<RemoteAppTargetError>();
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/session_events.rs" <<'RS'
fn media_source_lost(reason: TargetResolutionError) {
    reason.frontend_action().as_str();
}

#[cfg(test)]
mod tests {
    #[test]
    fn media_source_loss_projects_typed_frontend_action() {
        media_source_lost(TargetResolutionError::TargetNotFound);
    }
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs" <<'RS'
fn select_application_window_set_for_binding() -> Result<(), RemoteAppTargetError> {
    let off_display_window_ids = vec![10];
    if !off_display_window_ids.is_empty() {
        return Err(RemoteAppTargetError::new(
            "remote_desktop.create_session",
            TargetResolutionError::UnsupportedCaptureScope,
            "application target requires MultiAppSurface support",
        ));
    }
    Ok(())
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs" <<'RS'
trait RemoteAppMediaSourceFactory {
    fn start_from_binding();
}

fn start_remote_app_media_source(factory: &dyn RemoteAppMediaSourceFactory, binding: Binding, request: MediaStartRequest) {
    factory.start_from_binding(binding, request);
}

enum RemoteAppMediaSource {
    DisplayBaseline,
}

struct DirectWebRtcMediaSourceFactory;

impl RemoteAppMediaSourceFactory for DirectWebRtcMediaSourceFactory {
    fn start_from_binding(binding: Binding) -> Result<RemoteAppMediaSource, RemoteAppTargetError> {
        if binding.target_kind() == RemoteDesktopTargetKind::Display {
            Ok(RemoteAppMediaSource::DisplayBaseline)
        } else {
            Err(TargetResolutionError::DisplayFallbackForbidden.into())
        }
    }
}
RS

CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

cat >>"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_native_media.rs" <<'RS'
fn bad(entry: ResourceEntry) {
    target_for_entry(entry);
}
RS

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted ResourceEntry native resolution" >&2
  exit 1
fi

perl -0pi -e 's/target_for_entry\(entry\);//' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_native_media.rs"
cat >>"$SANDBOX/plugins/remote-desktop/src/transport/webrtc_negotiation.rs" <<'RS'
fn bad_resolver() {
    ResourceEntryTargetResolver.resolve_for_session();
}
RS

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted resolver use after session creation" >&2
  exit 1
fi

perl -0pi -e 's/\nfn bad_resolver\(\) \{\n    ResourceEntryTargetResolver\.resolve_for_session\(\);\n\}\n//' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_negotiation.rs"

perl -0pi -e 's/binding\.target_kind\(\) == RemoteDesktopTargetKind::Display/true/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted unguarded baseline fallback" >&2
  exit 1
fi

perl -0pi -e 's/if true/if binding.target_kind\(\) == RemoteDesktopTargetKind::Display/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e 's/\n        validate_resource_inventory_state\(\);//' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing inventory-state validation" >&2
  exit 1
fi

perl -0pi -e 's/(\n        metadata_freshness_u64\(\);)/\n        validate_resource_inventory_state();$1/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

cat >"$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs" <<'RS'
fn select_application_window_set_for_binding() -> Result<(), RemoteAppTargetError> {
    Ok(())
}
RS

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing off-display application guard" >&2
  exit 1
fi

cat >"$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs" <<'RS'
fn select_application_window_set_for_binding() -> Result<(), RemoteAppTargetError> {
    let off_display_window_ids = vec![10];
    if !off_display_window_ids.is_empty() {
        return Err(RemoteAppTargetError::new(
            "remote_desktop.create_session",
            TargetResolutionError::UnsupportedCaptureScope,
            "application target requires MultiAppSurface support",
        ));
    }
    Ok(())
}
RS

CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e 's/display_scoped_application_window_set/application/g' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing application target model" >&2
  exit 1
fi

perl -0pi -e 's/\\.with_context\\("frontend_action", self\\.reason\\.frontend_action\\(\\)\\.as_str\\(\\)\\)//' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing frontend_action Axon context" >&2
  exit 1
fi

echo "test_check_remoteapp_target_binding_boundary.sh: all cases passed"
