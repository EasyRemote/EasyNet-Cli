#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-target-binding-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p "$SANDBOX/plugins/remote-desktop/src/handlers"
mkdir -p "$SANDBOX/plugins/remote-desktop/src/media"
mkdir -p "$SANDBOX/plugins/remote-desktop/src/transport"
mkdir -p "$SANDBOX/docs/design"

cat >"$SANDBOX/docs/design/remoteapp-targeted-session-spec.md" <<'MD'
| E2E-05 stale window fail-closed | stale window/application targets must fail closed before active session insertion |
| E2E-06 no media re-resolution | native media startup must consume the committed target binding instead of re-resolving a ResourceEntry |
| E2E-10 weak identity ambiguity | weak app/window identity must fail closed before stream startup |
MD

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

    fn target_event_type(self) -> Option<&'static str> {
        match self {
            Self::TargetStale => Some("CAPTURE_TARGET_STALE"),
            Self::TargetIdentityAmbiguous => Some("CAPTURE_TARGET_AMBIGUOUS"),
            Self::DisplayFallbackForbidden => Some("DISPLAY_FALLBACK_FORBIDDEN"),
            Self::TargetPermissionMissing => Some("SCREEN_CAPTURE_PERMISSION_DENIED"),
            _ => None,
        }
    }
}
impl RemoteAppTargetError {
    fn to_axon(&self) -> AxonError {
        let mut error = AxonError::new()
            .with_context("target_reason", self.reason.as_str())
            .with_context("frontend_action", self.reason.frontend_action().as_str());
        if let Some(target_event_type) = self.reason.target_event_type() {
            error = error.with_context("target_event_type", target_event_type);
        }
        error
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
    fn resolve_for_session(&self, ability: &'static str, entry: &ResourceEntry, target_kind: RemoteDesktopTargetKind) {
        validate_owner_agent_ura(ability, entry)?;
        validate_resource_inventory_state();
        metadata_freshness_u64();
        let _ = TargetResolutionError::TargetIdentityAmbiguous;
        let _ = "app_name/title are diagnostic hints, not production routing identity";
        let _ = "app_name alone is not production routing identity";
        json!({
            "target_model": target_kind.target_model(),
        });
    }
}

fn validate_owner_agent_ura(ability: &'static str, entry: &ResourceEntry) -> Result<(), RemoteAppTargetError> {
    let _ = ability;
    let _ = entry;
    let _ = "owner_agent must be an Agent/SystemAgent URA";
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_target_resolution_reason_has_canonical_frontend_action_and_axon_context() {
        for reason in ALL_TARGET_RESOLUTION_ERRORS {
            assert!(ALL_FRONTEND_ACTIONS.contains(&reason.frontend_action()));
        }
    }

    #[test]
    fn target_resolution_reasons_project_spec_event_taxonomy_for_create_session_failures() {
        let expected = [
            (TargetResolutionError::TargetStale, Some("CAPTURE_TARGET_STALE")),
            (
                TargetResolutionError::TargetIdentityAmbiguous,
                Some("CAPTURE_TARGET_AMBIGUOUS"),
            ),
            (
                TargetResolutionError::DisplayFallbackForbidden,
                Some("DISPLAY_FALLBACK_FORBIDDEN"),
            ),
            (
                TargetResolutionError::TargetPermissionMissing,
                Some("SCREEN_CAPTURE_PERMISSION_DENIED"),
            ),
        ];
        for (reason, target_event_type) in expected {
            assert_eq!(reason.target_event_type(), target_event_type);
        }
    }

    #[test]
    fn window_requires_stable_owner_identity_not_app_name_only() {}

    #[test]
    fn application_requires_display_scoped_stable_identity() {}

    #[test]
    fn target_binding_rejects_non_agent_owner_projection() {}
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs" <<'RS'
fn create_session() {
    RemoteDesktopSessionCreationWorkflow::start();
}

#[cfg(test)]
mod tests {
    #[test]
    fn create_session_rejects_stale_window_inventory_before_session_insert() {
        assert!(err.to_string().contains("target_not_found"));
        assert!(err.to_string().contains("frontend_action=refresh_targets"));
        assert!(!sessions.contains_key("rd-stale-window"));
    }

    #[test]
    fn create_session_rejects_weak_window_identity_before_session_insert() {
        assert!(err.to_string().contains("target_identity_ambiguous"));
        assert!(!sessions.contains_key("rd-weak-window"));
    }
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

cat >"$SANDBOX/plugins/remote-desktop/src/invoke_bidi.rs" <<'RS'
fn capture_binding_diagnostic_jpeg(binding: RemoteAppTargetBinding) {
    capture_native_binding_diagnostic_jpeg(binding);
}

fn capture_native_binding_diagnostic_jpeg(binding: RemoteAppTargetBinding) {}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostic_jpeg_window_capture_does_not_use_resource_entry_backend() {}
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
struct ScreenCaptureKitTarget;

impl ScreenCaptureKitTarget {
    fn capture_proof(&self) -> &ResolvedCaptureTargetProof {
        todo!()
    }
}

fn target_for_binding(ability: &'static str, binding: &RemoteAppTargetBinding) {
    let target = resolve_target_for_binding(ability, binding).unwrap();
    binding.validate_reverified_capture_proof(ability, target.capture_proof());
}

fn capture_jpeg_for_binding(ability: &'static str, binding: &RemoteAppTargetBinding) {}

fn select_application_window_set_for_binding() -> Result<(), RemoteAppTargetError> {
    let off_display_window_ids = vec![10];
    if !off_display_window_ids.is_empty() {
        return Err(RemoteAppTargetError::new(
            "remote_desktop.create_session",
            TargetResolutionError::TargetMultiDisplayUnsupported,
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

#[cfg(test)]
mod tests {
    #[test]
    fn fake_factory_receives_session_owned_binding_without_resource_re_resolution() {
        let seen_binding_id = Some(expected_binding_id);
        assert_eq!(seen_binding_id, Some(expected_binding_id));
    }
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/media/mod.rs" <<'RS'
const XCAP_OPENH264_BACKEND: RemoteDesktopMediaBackendDescriptor =
    RemoteDesktopMediaBackendDescriptor {
        supported_subjects: &["display"],
    };

const XCAP_OPENH264_WEBRTC_BACKEND: RemoteDesktopMediaBackendDescriptor =
    RemoteDesktopMediaBackendDescriptor {
        supported_subjects: &["display"],
    };

fn xcap_supported_screen_entry(entry: ResourceEntry) -> bool {
    let backend = entry.metadata.get("backend").and_then(Value::as_str);
    entry.kind == ResourceType::Display && backend == Some("xcap")
}

#[cfg(test)]
mod tests {
    #[test]
    fn xcap_baseline_catalog_is_display_only_for_remoteapp_targets() {
        assert!(
            select_builtin_h264_backend(&discovered_window_entry("xcap")).is_none(),
            "diagnostic xcap baseline must not advertise app/window capture; exact remoteapp capture requires native target binding"
        );
    }
}
RS

CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e 's/create_session_rejects_stale_window_inventory_before_session_insert/create_session_accepts_stale_window_inventory/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing stale-window fail-closed test" >&2
  exit 1
fi

perl -0pi -e 's/create_session_accepts_stale_window_inventory/create_session_rejects_stale_window_inventory_before_session_insert/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"

perl -0pi -e 's/assert!\(!sessions\.contains_key\("rd-stale-window"\)\);/assert!(sessions.contains_key("rd-stale-window"));/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted stale target session insertion" >&2
  exit 1
fi

perl -0pi -e 's/assert!\(sessions\.contains_key\("rd-stale-window"\)\);/assert!(!sessions.contains_key("rd-stale-window"));/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"

perl -0pi -e 's/create_session_rejects_weak_window_identity_before_session_insert/create_session_accepts_weak_window_identity/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing weak-identity fail-closed test" >&2
  exit 1
fi

perl -0pi -e 's/create_session_accepts_weak_window_identity/create_session_rejects_weak_window_identity_before_session_insert/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"
perl -0pi -e 's/assert!\(!sessions\.contains_key\("rd-weak-window"\)\);/assert!(sessions.contains_key("rd-weak-window"));/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted weak target session insertion" >&2
  exit 1
fi

perl -0pi -e 's/assert!\(sessions\.contains_key\("rd-weak-window"\)\);/assert!(!sessions.contains_key("rd-weak-window"));/' \
  "$SANDBOX/plugins/remote-desktop/src/handlers/create_session.rs"

perl -0pi -e 's@app_name/title are diagnostic hints, not production routing identity@app_name title can route production target@' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted app_name/title as production identity" >&2
  exit 1
fi

perl -0pi -e 's@app_name title can route production target@app_name/title are diagnostic hints, not production routing identity@' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

perl -0pi -e 's/fn target_event_type/fn legacy_target_event_type/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing target_event_type taxonomy helper" >&2
  exit 1
fi

perl -0pi -e 's/fn legacy_target_event_type/fn target_event_type/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

perl -0pi -e 's/error = error\.with_context\("target_event_type", target_event_type\);/let _ = target_event_type;/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted target errors without target_event_type Axon context" >&2
  exit 1
fi

perl -0pi -e 's/let _ = target_event_type;/error = error.with_context("target_event_type", target_event_type);/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

perl -0pi -e 's/Self::TargetStale => Some\("CAPTURE_TARGET_STALE"\)/Self::TargetStale => None/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing CAPTURE_TARGET_STALE taxonomy mapping" >&2
  exit 1
fi

perl -0pi -e 's/Self::TargetStale => None/Self::TargetStale => Some("CAPTURE_TARGET_STALE")/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

perl -0pi -e 's/fake_factory_receives_session_owned_binding_without_resource_re_resolution/fake_factory_re_resolves_resource_entry/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing no-re-resolution media factory test" >&2
  exit 1
fi

perl -0pi -e 's/fake_factory_re_resolves_resource_entry/fake_factory_receives_session_owned_binding_without_resource_re_resolution/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

perl -0pi -e 's/Some\(expected_binding_id\)/None/g' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted media factory without stored binding assertion" >&2
  exit 1
fi

perl -0pi -e 's/None/Some(expected_binding_id)/g' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e 's/supported_subjects: &\["display"\]/supported_subjects: &["display", "window", "application"]/g' \
  "$SANDBOX/plugins/remote-desktop/src/media/mod.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted xcap baseline app/window catalog support" >&2
  exit 1
fi

perl -0pi -e 's/supported_subjects: &\["display", "window", "application"\]/supported_subjects: &["display"]/g' \
  "$SANDBOX/plugins/remote-desktop/src/media/mod.rs"

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

perl -0pi -e 's/\n    binding\.validate_reverified_capture_proof\(ability, target\.capture_proof\(\)\);//' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted SCK target startup without committed binding proof revalidation" >&2
  exit 1
fi

perl -0pi -e 's/(let target = resolve_target_for_binding\(ability, binding\)\.unwrap\(\);)/$1\n    binding.validate_reverified_capture_proof(ability, target.capture_proof());/' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

perl -0pi -e 's/\n        validate_resource_inventory_state\(\);//' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing inventory-state validation" >&2
  exit 1
fi

perl -0pi -e 's/(\n        metadata_freshness_u64\(\);)/\n        validate_resource_inventory_state();$1/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e 's/\n        validate_owner_agent_ura\(ability, entry\)\?;//' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing owner_agent validation" >&2
  exit 1
fi

perl -0pi -e 's/(\n        validate_resource_inventory_state\(\);)/\n        validate_owner_agent_ura(ability, entry)?;$1/' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

cat >"$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs" <<'RS'
struct ScreenCaptureKitTarget;

impl ScreenCaptureKitTarget {
    fn capture_proof(&self) -> &ResolvedCaptureTargetProof {
        todo!()
    }
}

fn target_for_binding(ability: &'static str, binding: &RemoteAppTargetBinding) {
    let target = resolve_target_for_binding(ability, binding).unwrap();
    binding.validate_reverified_capture_proof(ability, target.capture_proof());
}

fn capture_jpeg_for_binding(ability: &'static str, binding: &RemoteAppTargetBinding) {}

fn select_application_window_set_for_binding() -> Result<(), RemoteAppTargetError> {
    Ok(())
}
RS

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing off-display application guard" >&2
  exit 1
fi

cat >"$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs" <<'RS'
struct ScreenCaptureKitTarget;

impl ScreenCaptureKitTarget {
    fn capture_proof(&self) -> &ResolvedCaptureTargetProof {
        todo!()
    }
}

fn target_for_binding(ability: &'static str, binding: &RemoteAppTargetBinding) {
    let target = resolve_target_for_binding(ability, binding).unwrap();
    binding.validate_reverified_capture_proof(ability, target.capture_proof());
}

fn capture_jpeg_for_binding(ability: &'static str, binding: &RemoteAppTargetBinding) {}

fn select_application_window_set_for_binding() -> Result<(), RemoteAppTargetError> {
    let off_display_window_ids = vec![10];
    if !off_display_window_ids.is_empty() {
        return Err(RemoteAppTargetError::new(
            "remote_desktop.create_session",
            TargetResolutionError::TargetMultiDisplayUnsupported,
            "application target requires MultiAppSurface support",
        ));
    }
    Ok(())
}
RS

CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e 's/TargetResolutionError::TargetMultiDisplayUnsupported/TargetResolutionError::UnsupportedCaptureScope/' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted generic unsupported_capture_scope for multi-display app binding" >&2
  exit 1
fi

perl -0pi -e 's/TargetResolutionError::UnsupportedCaptureScope/TargetResolutionError::TargetMultiDisplayUnsupported/' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

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
