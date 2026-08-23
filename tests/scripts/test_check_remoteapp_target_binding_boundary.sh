#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$REPO_ROOT/tools/scripts/check-remoteapp-target-binding-boundary.sh"
SANDBOX="$(mktemp -d)"
trap 'rm -rf "$SANDBOX"' EXIT

mkdir -p "$SANDBOX/plugins/remote-desktop/src/handlers"
mkdir -p "$SANDBOX/plugins/remote-desktop/src/media"
mkdir -p "$SANDBOX/plugins/remote-desktop/src/transport"
mkdir -p "$SANDBOX/src/daemon/ability/builtins/resources/media"
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

    fn target_model_for_platform(self, _platform: &str) -> &'static str {
        self.target_model()
    }
}

impl RemoteAppTargetBinding {
    fn to_value(&self) {
        json!({
            "target_model": self.target_kind.target_model_for_platform(&self.platform),
        });
    }

    fn target_bound_event_payload(&self) {
        json!({
            "target_model": self.target_kind.target_model_for_platform(&self.platform),
        });
    }
}

impl AppWindowSetProof {
    fn contains_window_id(&self, window_id: u64) -> bool {
        true
    }

    fn missing_window_ids(&self, observed_window_ids: &[u64]) -> Vec<u64> {
        Vec::new()
    }
}

impl ScopeAudit {
    fn to_value(&self) {
        json!({
            "target_model": self.effective_target_kind.target_model_for_platform(platform),
        });
    }
}

struct NativeAppIdentityCandidate;
struct NativeAppIdentityExpectation;

impl NativeTargetLocator {
    fn app_identity_expectation(&self) -> NativeAppIdentityExpectation {
        NativeAppIdentityExpectation
    }
}

impl NativeAppIdentityExpectation {
    fn evaluate(&self, candidate: NativeAppIdentityCandidate) -> NativeAppIdentityMatch {
        NativeAppIdentityMatch
    }
}

struct NativeAppIdentityMatch;

impl NativeAppIdentityMatch {
    fn matched(&self) -> bool {
        true
    }
}

struct ResolvedCaptureTargetProof;

impl ResolvedCaptureTargetProof {
    fn validate_for_binding(&self, binding: &RemoteAppTargetBinding) {
        binding
            .native_locator()
            .app_identity_expectation()
            .evaluate(self.native_app_identity_candidate())
            .matched();
    }

    fn matches_committed_identity(&self, committed: &Self) -> bool {
        committed
            .native_app_identity_expectation()
            .evaluate(self.native_app_identity_candidate())
            .matched()
    }

    fn native_app_identity_expectation(&self) -> NativeAppIdentityExpectation {
        NativeAppIdentityExpectation
    }

    fn native_app_identity_candidate(&self) -> NativeAppIdentityCandidate {
        NativeAppIdentityCandidate
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
            "target_model": target_kind.target_model_for_platform(&platform),
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

    #[test]
    fn native_app_identity_expectation_matches_canonical_bundle_aliases() {}

    #[test]
    fn native_app_identity_expectation_requires_all_declared_identity_fields() {}

    #[test]
    fn capture_proof_revalidation_uses_native_app_identity_aliases() {}
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
    let input_control_granted = consent.permits_input_control();
    ResourceEntryTargetResolver.resolve_for_session_with_input_consent(input_control_granted);
    verify_target_binding_for_session();
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/session_identity.rs" <<'RS'
struct RemoteDesktopSessionProfile {
    session_id: String,
    subject_ura: String,
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/session.rs" <<'RS'
impl RemoteDesktopSession {
    fn subject_type(&self) -> ResourceType {
        self.target.binding().target_kind().resource_type()
    }
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
    let target_binding = session.target_binding().clone();
    let input_policy = session.input_policy().clone();
    EffectiveRemoteDesktopInputPolicy::for_binding(&input_policy, &target_binding);
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

struct ApplicationWindowSetTarget {
    proof: AppWindowSetProof,
    excepting_windows: Retained<NSArray<SCWindow>>,
}

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

fn sck_app_identity_match(expected: NativeAppIdentityExpectation, app: SCRunningApplication) {
    NativeAppIdentityCandidate;
    expected.evaluate(app);
}

fn select_application_for_binding(binding: &RemoteAppTargetBinding) {
    let expected = binding.native_locator().app_identity_expectation();
    sck_app_identity_match(expected, app);
}

fn resolve_target_for_binding(ability: &'static str, binding: &RemoteAppTargetBinding) {
    let app_window_set = select_application_window_set_for_binding(ability, windows, binding, display).unwrap();
    let filter = SCContentFilter::initWithDisplay_includingApplications_exceptingWindows(
        SCContentFilter::alloc(),
        &display,
        &included_applications,
        &app_window_set.excepting_windows,
    );
}

fn select_application_window_set_for_binding() -> Result<(), RemoteAppTargetError> {
    let committed_window_set = binding.committed_app_window_set()?;
    let mut uncommitted_same_display_windows = Vec::new();
    let off_display_window_ids = vec![10];
    for window_id in [10] {
        let overlaps_selected_display = sck_window_overlaps_display(&window, display);
        if !committed_window_set.contains_window_id(window_id) {
            if overlaps_selected_display {
                uncommitted_same_display_windows.push(window);
            }
            continue;
        }
    }
    if !off_display_window_ids.is_empty() {
        return Err(RemoteAppTargetError::new(
            "remote_desktop.create_session",
            TargetResolutionError::TargetMultiDisplayUnsupported,
            "application target requires MultiAppSurface support",
        ));
    }
    let missing_window_ids = committed_window_set.missing_window_ids(&window_ids);
    if !missing_window_ids.is_empty() {
        return Err(RemoteAppTargetError::new(
            "remote_desktop.create_session",
            TargetResolutionError::TargetIdentityChanged,
            "committed application window set changed",
        ));
    }
    let proof = committed_window_set.clone();
    let excepting_window_refs = uncommitted_same_display_windows
        .iter()
        .map(|window| window.as_ref())
        .collect::<Vec<_>>();
    let excepting_windows = NSArray::from_slice(&excepting_window_refs);
    Ok(ApplicationWindowSetTarget { proof, excepting_windows })
}

#[cfg(test)]
mod tests {
    #[test]
    fn application_window_set_selector_excludes_uncommitted_same_display_windows() {}
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
    XcapBaseline,
}

struct DirectWebRtcMediaSourceFactory;

impl RemoteAppMediaSourceFactory for DirectWebRtcMediaSourceFactory {
    fn start_from_binding(binding: Binding) -> Result<RemoteAppMediaSource, RemoteAppTargetError> {
        binding.require_capture_proof(ABILITY_SET_DESCRIPTION)?;
        validate_available_webrtc_backend(request.config.backend, binding)?;
        if request.config.backend.production_ready() {
            validate_native_production_binding(request.config.backend, binding)?;
        }
        if binding.supports_xcap_adapter() {
            Ok(RemoteAppMediaSource::XcapBaseline)
        } else {
            Err(anyhow!("xcap baseline cannot bind without widening its scope"))
        }
    }
}

fn validate_available_webrtc_backend(backend: Backend, binding: Binding) -> Result<(), RemoteAppTargetError> {
    if !backend.is_available() || !backend.is_webrtc_transport() || !backend.transport_ready() {
        return Err(TargetResolutionError::CaptureBackendUnavailable.into());
    }
    Ok(())
}

fn validate_native_production_binding(backend: Backend, binding: Binding) -> Result<(), RemoteAppTargetError> {
    if !backend.supports_subject(binding.target_kind().resource_type()) {
        return Err(TargetResolutionError::CaptureBackendUnavailable.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn fake_factory_receives_session_owned_binding_without_resource_re_resolution() {
        let seen_binding_id = Some(expected_binding_id);
        assert_eq!(seen_binding_id, Some(expected_binding_id));
    }

    #[test]
    fn direct_factory_rejects_uncommitted_target_binding_before_media_selection() {}
}
RS

cat >"$SANDBOX/plugins/remote-desktop/src/media/mod.rs" <<'RS'
const XCAP_OPENH264_BACKEND: RemoteDesktopMediaBackendDescriptor =
    RemoteDesktopMediaBackendDescriptor {
        supported_subjects: &["display", "window", "application"],
    };

const XCAP_OPENH264_WEBRTC_BACKEND: RemoteDesktopMediaBackendDescriptor =
    RemoteDesktopMediaBackendDescriptor {
        supported_subjects: &["display", "window", "application"],
    };

fn xcap_supported_screen_entry(entry: ResourceEntry) -> bool {
    let backend = entry.metadata.get("backend").and_then(Value::as_str);
    backend == Some("xcap") && match entry.kind {
        ResourceType::Display => true,
        ResourceType::Window | ResourceType::Application => screen_target_metadata_resolvable(entry),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn xcap_baseline_catalog_supports_exact_window_and_application_targets() {
        assert!(select_builtin_h264_backend(&discovered_window_entry("xcap")).is_some());
    }

    #[test]
    fn direct_webrtc_binding_uses_xcap_without_widening_window_or_application_scope() {
        assert!(true);
    }
}
RS

cat >"$SANDBOX/src/daemon/ability/builtins/resources/media/screen_snapshot.rs" <<'RS'
const MAX_APPLICATION_COMPOSITE_PIXELS: u64 = 33_177_600;
fn capture_application_rgb_with_xcap() {}
fn application_compositor_cross_display_gap_is_black_not_host_display_content() {}
RS

CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e 's/application_compositor_cross_display_gap_is_black_not_host_display_content/application_compositor_cross_display_gap_uses_host_display_content/' \
  "$SANDBOX/src/daemon/ability/builtins/resources/media/screen_snapshot.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted application compositor without cross-display leakage regression" >&2
  exit 1
fi

perl -0pi -e 's/application_compositor_cross_display_gap_uses_host_display_content/application_compositor_cross_display_gap_is_black_not_host_display_content/' \
  "$SANDBOX/src/daemon/ability/builtins/resources/media/screen_snapshot.rs"

perl -0pi -e 's/input_control_granted/input_control_implicit/g' \
  "$SANDBOX/plugins/remote-desktop/src/session_creation.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted implicit input-control consent at target binding resolution" >&2
  exit 1
fi

perl -0pi -e 's/input_control_implicit/input_control_granted/g' \
  "$SANDBOX/plugins/remote-desktop/src/session_creation.rs"

perl -0pi -e 's/&app_window_set\.excepting_windows/&excepting_windows/' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted application SCK filter without committed exceptingWindows" >&2
  exit 1
fi

perl -0pi -e 's/&excepting_windows/&app_window_set.excepting_windows/' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

perl -0pi -e 's/uncommitted_same_display_windows\.push\(window\);/let _ = window;/' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted application selector without uncommitted same-app window exclusions" >&2
  exit 1
fi

perl -0pi -e 's/let _ = window;/uncommitted_same_display_windows.push(window);/' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

perl -0pi -e 's/application_window_set_selector_excludes_uncommitted_same_display_windows/application_window_set_selector_allows_uncommitted_same_display_windows/' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing uncommitted same-app exclusion regression test" >&2
  exit 1
fi

perl -0pi -e 's/application_window_set_selector_allows_uncommitted_same_display_windows/application_window_set_selector_excludes_uncommitted_same_display_windows/' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

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

perl -0pi -e 's/struct RemoteDesktopSessionProfile \{/struct RemoteDesktopSessionProfile {\\n    subject_type: ResourceType,/' \
  "$SANDBOX/plugins/remote-desktop/src/session_identity.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted cached subject_type in session profile" >&2
  exit 1
fi

perl -0pi -e 's/\\n    subject_type: ResourceType,//' \
  "$SANDBOX/plugins/remote-desktop/src/session_identity.rs"

perl -0pi -e 's/self\.target\.binding\(\)\.target_kind\(\)\.resource_type\(\)/self.profile.subject_type()/g' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted subject_type projection from session profile" >&2
  exit 1
fi

perl -0pi -e 's/self\.profile\.subject_type\(\)/self.target.binding().target_kind().resource_type()/g' \
  "$SANDBOX/plugins/remote-desktop/src/session.rs"

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

perl -0pi -e 's/EffectiveRemoteDesktopInputPolicy::for_binding\(&input_policy, &target_binding\);/RemoteDesktopInputPolicy::default();/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_negotiation.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted WebRTC input policy without the session-owned target binding" >&2
  exit 1
fi

perl -0pi -e 's/RemoteDesktopInputPolicy::default\(\);/EffectiveRemoteDesktopInputPolicy::for_binding(&input_policy, &target_binding);/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/webrtc_negotiation.rs"

perl -0pi -e 's/supported_subjects: &\["display", "window", "application"\]/supported_subjects: &["display"]/g' \
  "$SANDBOX/plugins/remote-desktop/src/media/mod.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted display-only xcap target catalog" >&2
  exit 1
fi

perl -0pi -e 's/supported_subjects: &\["display"\]/supported_subjects: &["display", "window", "application"]/g' \
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

perl -0pi -e 's/binding\.supports_xcap_adapter\(\)/true/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted unguarded baseline fallback" >&2
  exit 1
fi

perl -0pi -e 's/if true/if binding.supports_xcap_adapter()/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

perl -0pi -e 's/\n        binding\.require_capture_proof\(ABILITY_SET_DESCRIPTION\)\?;//' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted media source factory without committed capture proof gate" >&2
  exit 1
fi

perl -0pi -e 's/fn start_from_binding\(binding: Binding\) -> Result<RemoteAppMediaSource, RemoteAppTargetError> \{\n/fn start_from_binding(binding: Binding) -> Result<RemoteAppMediaSource, RemoteAppTargetError> {\n        binding.require_capture_proof(ABILITY_SET_DESCRIPTION)?;\n/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

perl -0pi -e 's/direct_factory_rejects_uncommitted_target_binding_before_media_selection/direct_factory_allows_uncommitted_target_binding/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing uncommitted binding media source test" >&2
  exit 1
fi

perl -0pi -e 's/direct_factory_allows_uncommitted_target_binding/direct_factory_rejects_uncommitted_target_binding_before_media_selection/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

perl -0pi -e 's/\n        validate_available_webrtc_backend\(request\.config\.backend, binding\)\?;//' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted media source factory without backend availability validation" >&2
  exit 1
fi

perl -0pi -e 's/(binding\.require_capture_proof\(ABILITY_SET_DESCRIPTION\)\?;)/$1\n        validate_available_webrtc_backend(request.config.backend, binding)?;/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

perl -0pi -e 's/!backend\.is_available\(\) \|\| !backend\.is_webrtc_transport\(\) \|\| !backend\.transport_ready\(\)/false/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted backend availability helper without availability/transport predicate" >&2
  exit 1
fi

perl -0pi -e 's/if false/if !backend.is_available() || !backend.is_webrtc_transport() || !backend.transport_ready()/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

perl -0pi -e 's/\n            validate_native_production_binding\(request\.config\.backend, binding\)\?;//' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted production media source without native binding validation" >&2
  exit 1
fi

perl -0pi -e 's/(if request\.config\.backend\.production_ready\(\) \{\n)/$1            validate_native_production_binding(request.config.backend, binding)?;\n/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

perl -0pi -e 's/!backend\.supports_subject\(binding\.target_kind\(\)\.resource_type\(\)\)/false/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted production binding helper without backend subject predicate" >&2
  exit 1
fi

perl -0pi -e 's/if false/if !backend.supports_subject(binding.target_kind().resource_type())/' \
  "$SANDBOX/plugins/remote-desktop/src/transport/media_source.rs"

CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null

perl -0pi -e 's/NativeAppIdentityExpectation/NativeAppIdentityExpectationRemoved/g' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted missing native app identity expectation" >&2
  exit 1
fi

perl -0pi -e 's/NativeAppIdentityExpectationRemoved/NativeAppIdentityExpectation/g' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

perl -0pi -e 's/\.evaluate\(self\.native_app_identity_candidate\(\)\)/.manual_compare(self.native_app_identity_candidate())/g' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted capture proof validation without centralized identity matcher" >&2
  exit 1
fi

perl -0pi -e 's/\.manual_compare\(self\.native_app_identity_candidate\(\)\)/.evaluate(self.native_app_identity_candidate())/g' \
  "$SANDBOX/plugins/remote-desktop/src/target.rs"

perl -0pi -e 's/sck_app_identity_match/manual_app_identity_match/g' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted SCK selector without centralized identity matcher" >&2
  exit 1
fi

perl -0pi -e 's/manual_app_identity_match/sck_app_identity_match/g' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

perl -0pi -e 's/fn select_application_for_binding\(binding: &RemoteAppTargetBinding\) \{\n/fn select_application_for_binding(binding: &RemoteAppTargetBinding) {\n    let expected_pid = 42;\n/' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

if CHECK_REMOTEAPP_TARGET_BINDING_ROOT="$SANDBOX" bash "$SCRIPT" >/dev/null 2>&1; then
  echo "remoteapp target binding checker accepted SCK-local expected_pid matcher state" >&2
  exit 1
fi

perl -0pi -e 's/\n    let expected_pid = 42;//' \
  "$SANDBOX/plugins/remote-desktop/src/screencapturekit_capture.rs"

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

struct ApplicationWindowSetTarget {
    proof: AppWindowSetProof,
    excepting_windows: Retained<NSArray<SCWindow>>,
}

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

fn sck_app_identity_match(expected: NativeAppIdentityExpectation, app: SCRunningApplication) {
    NativeAppIdentityCandidate;
    expected.evaluate(app);
}

fn select_application_for_binding(binding: &RemoteAppTargetBinding) {
    let expected = binding.native_locator().app_identity_expectation();
    sck_app_identity_match(expected, app);
}

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

fn sck_app_identity_match(expected: NativeAppIdentityExpectation, app: SCRunningApplication) {
    NativeAppIdentityCandidate;
    expected.evaluate(app);
}

fn select_application_for_binding(binding: &RemoteAppTargetBinding) {
    let expected = binding.native_locator().app_identity_expectation();
    sck_app_identity_match(expected, app);
}

fn resolve_target_for_binding(ability: &'static str, binding: &RemoteAppTargetBinding) {
    let app_window_set = select_application_window_set_for_binding(ability, windows, binding, display).unwrap();
    let filter = SCContentFilter::initWithDisplay_includingApplications_exceptingWindows(
        SCContentFilter::alloc(),
        &display,
        &included_applications,
        &app_window_set.excepting_windows,
    );
}

fn select_application_window_set_for_binding() -> Result<(), RemoteAppTargetError> {
    let committed_window_set = binding.committed_app_window_set()?;
    let mut uncommitted_same_display_windows = Vec::new();
    let off_display_window_ids = vec![10];
    for window_id in [10] {
        let overlaps_selected_display = sck_window_overlaps_display(&window, display);
        if !committed_window_set.contains_window_id(window_id) {
            if overlaps_selected_display {
                uncommitted_same_display_windows.push(window);
            }
            continue;
        }
    }
    if !off_display_window_ids.is_empty() {
        return Err(RemoteAppTargetError::new(
            "remote_desktop.create_session",
            TargetResolutionError::TargetMultiDisplayUnsupported,
            "application target requires MultiAppSurface support",
        ));
    }
    let missing_window_ids = committed_window_set.missing_window_ids(&window_ids);
    if !missing_window_ids.is_empty() {
        return Err(RemoteAppTargetError::new(
            "remote_desktop.create_session",
            TargetResolutionError::TargetIdentityChanged,
            "committed application window set changed",
        ));
    }
    let proof = committed_window_set.clone();
    let excepting_window_refs = uncommitted_same_display_windows
        .iter()
        .map(|window| window.as_ref())
        .collect::<Vec<_>>();
    let excepting_windows = NSArray::from_slice(&excepting_window_refs);
    Ok(ApplicationWindowSetTarget { proof, excepting_windows })
}

#[cfg(test)]
mod tests {
    #[test]
    fn application_window_set_selector_excludes_uncommitted_same_display_windows() {}
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
