// EasyNet CLI — remote desktop target observation provider
// ========================================================
//
// File: plugins/remote-desktop/src/target_observer.rs
// Description: Platform target observation seam for remote app/window sessions.
//
// Boundary:
// - Providers inspect local OS state and return TargetObservation values.
// - Providers do not mutate resources.json, session state, media streams, or
//   input state.
// - RemoteDesktopSession remains the only committed target lifecycle writer.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::daemon::plugins::remote_desktop::session::now_ms;
use crate::daemon::plugins::remote_desktop::session::TargetMediaSourceLost;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::target::{
    AppSurfaceLayoutProof, AppWindowSetProof, NativeAppIdentityCandidate, RemoteAppTargetBinding,
    RemoteDesktopTargetKind, TargetGeometry, TargetResolutionError,
};
use crate::daemon::plugins::remote_desktop::target_tracking::{
    TargetObservation, TargetTrackerSnapshot, TargetVisibilityState,
};

pub(in crate::daemon::plugins::remote_desktop) trait TargetObservationProvider {
    fn observe(
        &self,
        binding: &RemoteAppTargetBinding,
        snapshot: &TargetTrackerSnapshot,
    ) -> Option<TargetObservation>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetObservationPollResult {
    pub(in crate::daemon::plugins::remote_desktop) keep_tracking: bool,
    pub(in crate::daemon::plugins::remote_desktop) state_changed: bool,
    pub(in crate::daemon::plugins::remote_desktop) media_source_lost: Option<TargetMediaSourceLost>,
}

impl TargetObservationPollResult {
    fn keep_tracking() -> Self {
        Self {
            keep_tracking: true,
            state_changed: false,
            media_source_lost: None,
        }
    }

    fn stop_tracking() -> Self {
        Self {
            keep_tracking: false,
            state_changed: false,
            media_source_lost: None,
        }
    }

    fn rebind_deadline_expired(media_source_lost: Option<TargetMediaSourceLost>) -> Self {
        Self {
            keep_tracking: true,
            state_changed: true,
            media_source_lost,
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct PlatformTargetObservationSample {
    state: PlatformTargetObservationSampleState,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum PlatformTargetObservationSampleState {
    HostSnapshot(HostTargetSnapshot),
    SnapshotFailed {
        detail: String,
        observed_at_ms: u64,
    },
    PermissionRevoked {
        detail: String,
        observed_at_ms: u64,
    },
    #[cfg(not(target_os = "macos"))]
    UnsupportedPlatform,
}

#[derive(Debug, Clone)]
struct ObservedWindow {
    window_id: u64,
    pid: Option<i64>,
    bundle_id: Option<String>,
    display_id: Option<u64>,
    title: Option<String>,
    focused: bool,
    geometry: TargetGeometry,
    visibility_state: TargetVisibilityState,
}

#[derive(Debug, Clone)]
struct HostTargetSnapshot {
    windows: Vec<ObservedWindow>,
    display_ids: BTreeSet<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum TargetInputGuardFailure {
    UnsupportedPlatform,
    SnapshotFailed,
    DisplayUnavailable,
    TargetNotFound,
    IdentityMismatch,
    NotVisible,
    FocusNotCommitted,
    NotFocused,
    GeometryStale,
    WindowSetStale,
    PointerOutsideTargetSurface,
    PointerOccluded,
}

impl TargetInputGuardFailure {
    pub(in crate::daemon::plugins::remote_desktop) const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "target_input_guard_unsupported_platform",
            Self::SnapshotFailed => "target_input_guard_snapshot_failed",
            Self::DisplayUnavailable => "target_input_guard_display_unavailable",
            Self::TargetNotFound => "target_input_guard_target_not_found",
            Self::IdentityMismatch => "target_input_guard_identity_mismatch",
            Self::NotVisible => "target_input_guard_not_visible",
            Self::FocusNotCommitted => "target_input_guard_focus_not_committed",
            Self::NotFocused => "target_input_guard_not_focused",
            Self::GeometryStale => "target_input_guard_geometry_stale",
            Self::WindowSetStale => "target_input_guard_window_set_stale",
            Self::PointerOutsideTargetSurface => {
                "target_input_guard_pointer_outside_target_surface"
            }
            Self::PointerOccluded => "target_input_guard_pointer_occluded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetInputGuardProof {
    subject_ura: String,
    target_kind: RemoteDesktopTargetKind,
    snapshot_started_at_ms: u64,
    validated_at_ms: u64,
    target_geometry_revision: u64,
    target_focus_epoch: u64,
    pointer_target_window_id: Option<u64>,
}

impl TargetInputGuardProof {
    fn from_validated_target(
        binding: &RemoteAppTargetBinding,
        snapshot: &TargetTrackerSnapshot,
        snapshot_started_at_ms: u64,
        validated_at_ms: u64,
        pointer_target_window_id: Option<u64>,
    ) -> Self {
        Self {
            subject_ura: binding.subject_ura().to_string(),
            target_kind: binding.target_kind(),
            snapshot_started_at_ms,
            validated_at_ms,
            target_geometry_revision: snapshot.target_geometry_revision(),
            target_focus_epoch: snapshot.target_focus_epoch(),
            pointer_target_window_id,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self, session_id: &str) -> Value {
        let mut proof = json!({
            "status": "passed",
            "subject_ura": self.subject_ura,
            "session_id": session_id,
            "target_kind": self.target_kind.as_str(),
            "snapshot_started_at_ms": self.snapshot_started_at_ms,
            "validated_at_ms": self.validated_at_ms,
            "identity_exact": true,
            "visible": true,
            "focused": true,
            "target_geometry_revision": self.target_geometry_revision,
            "target_focus_epoch": self.target_focus_epoch,
        });
        match self.target_kind {
            RemoteDesktopTargetKind::Window => proof["window_id_exact"] = json!(true),
            RemoteDesktopTargetKind::Application => proof["window_set_exact"] = json!(true),
            RemoteDesktopTargetKind::Display => {}
        }
        if let Some(window_id) = self.pointer_target_window_id {
            proof["pointer_target_window_id"] = json!(window_id);
            proof["pointer_occlusion_checked"] = json!(true);
        }
        proof
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
trait HostTargetSnapshotProvider {
    fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot>;
}

impl<T> HostTargetSnapshotProvider for &T
where
    T: HostTargetSnapshotProvider + ?Sized,
{
    fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot> {
        (*self).snapshot()
    }
}

impl PlatformTargetObservationSample {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn from_snapshot_result(result: anyhow::Result<HostTargetSnapshot>) -> Self {
        match result {
            Ok(snapshot) => Self {
                state: PlatformTargetObservationSampleState::HostSnapshot(snapshot),
            },
            Err(error) => Self {
                state: PlatformTargetObservationSampleState::SnapshotFailed {
                    detail: format!("host target snapshot failed: {error}"),
                    observed_at_ms: now_ms(),
                },
            },
        }
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn permission_revoked(detail: impl Into<String>) -> Self {
        Self {
            state: PlatformTargetObservationSampleState::PermissionRevoked {
                detail: detail.into(),
                observed_at_ms: now_ms(),
            },
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn unsupported_platform() -> Self {
        Self {
            state: PlatformTargetObservationSampleState::UnsupportedPlatform,
        }
    }
}

impl TargetObservationProvider for PlatformTargetObservationSample {
    fn observe(
        &self,
        binding: &RemoteAppTargetBinding,
        snapshot: &TargetTrackerSnapshot,
    ) -> Option<TargetObservation> {
        match &self.state {
            PlatformTargetObservationSampleState::HostSnapshot(host_snapshot) => {
                observe_binding_against_host_snapshot(binding, snapshot, host_snapshot)
            }
            PlatformTargetObservationSampleState::SnapshotFailed {
                detail,
                observed_at_ms,
            } => Some(TargetObservation::Lost {
                reason: TargetResolutionError::CaptureBackendUnavailable,
                detail: detail.clone(),
                observed_at_ms: *observed_at_ms,
            }),
            PlatformTargetObservationSampleState::PermissionRevoked {
                detail,
                observed_at_ms,
            } => Some(TargetObservation::PermissionRevoked {
                detail: detail.clone(),
                observed_at_ms: *observed_at_ms,
            }),
            #[cfg(not(target_os = "macos"))]
            PlatformTargetObservationSampleState::UnsupportedPlatform => {
                unsupported_platform_target_observation(binding)
            }
        }
    }
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn sample_host_target_observations<P>(source: &P) -> PlatformTargetObservationSample
where
    P: HostTargetSnapshotProvider,
{
    PlatformTargetObservationSample::from_snapshot_result(source.snapshot())
}

pub(in crate::daemon::plugins::remote_desktop) fn sample_platform_target_observations(
) -> PlatformTargetObservationSample {
    platform::sample_platform_target_observations()
}

/// Validate target-local input against a deadline-bounded host snapshot
/// acquired immediately before the caller posts an OS event.
pub(in crate::daemon::plugins::remote_desktop) fn validate_target_input_observation(
    observation: &PlatformTargetObservationSample,
    binding: &RemoteAppTargetBinding,
    snapshot: &TargetTrackerSnapshot,
    snapshot_started_at_ms: u64,
    validated_at_ms: u64,
) -> Result<TargetInputGuardProof, TargetInputGuardFailure> {
    let host_snapshot = input_guard_host_snapshot(observation)?;
    validate_target_input_against_host_snapshot(binding, snapshot, &host_snapshot)?;
    Ok(TargetInputGuardProof::from_validated_target(
        binding,
        snapshot,
        snapshot_started_at_ms,
        validated_at_ms,
        None,
    ))
}

/// Validate a mapped host point against a fresh front-to-back native window
/// snapshot. Target-local pointer input may only land on an unobscured window
/// that belongs to the committed target surface; black compositor gaps and
/// windows belonging to other applications fail closed.
pub(in crate::daemon::plugins::remote_desktop) fn validate_target_pointer_input_observation(
    observation: &PlatformTargetObservationSample,
    binding: &RemoteAppTargetBinding,
    snapshot: &TargetTrackerSnapshot,
    host_x: f64,
    host_y: f64,
    snapshot_started_at_ms: u64,
    validated_at_ms: u64,
) -> Result<TargetInputGuardProof, TargetInputGuardFailure> {
    let host_snapshot = input_guard_host_snapshot(observation)?;
    validate_target_input_against_host_snapshot(binding, snapshot, &host_snapshot)?;
    let window_id =
        validate_pointer_target_against_host_snapshot(binding, &host_snapshot, host_x, host_y)?;
    Ok(TargetInputGuardProof::from_validated_target(
        binding,
        snapshot,
        snapshot_started_at_ms,
        validated_at_ms,
        Some(window_id),
    ))
}

fn input_guard_host_snapshot(
    observation: &PlatformTargetObservationSample,
) -> Result<&HostTargetSnapshot, TargetInputGuardFailure> {
    match &observation.state {
        PlatformTargetObservationSampleState::HostSnapshot(snapshot) => Ok(snapshot),
        PlatformTargetObservationSampleState::SnapshotFailed { .. }
        | PlatformTargetObservationSampleState::PermissionRevoked { .. } => {
            Err(TargetInputGuardFailure::SnapshotFailed)
        }
        #[cfg(not(target_os = "macos"))]
        PlatformTargetObservationSampleState::UnsupportedPlatform => {
            Err(TargetInputGuardFailure::UnsupportedPlatform)
        }
    }
}

fn validate_target_input_against_host_snapshot(
    binding: &RemoteAppTargetBinding,
    snapshot: &TargetTrackerSnapshot,
    host_snapshot: &HostTargetSnapshot,
) -> Result<(), TargetInputGuardFailure> {
    if snapshot.focused() != Some(true) {
        return Err(TargetInputGuardFailure::FocusNotCommitted);
    }
    if let Some(display_id) = binding.native_locator().display_id() {
        if !host_snapshot.display_ids.contains(&display_id) {
            return Err(TargetInputGuardFailure::DisplayUnavailable);
        }
    }
    match binding.target_kind() {
        RemoteDesktopTargetKind::Display => Err(TargetInputGuardFailure::UnsupportedPlatform),
        RemoteDesktopTargetKind::Window => {
            validate_window_input(binding, snapshot, &host_snapshot.windows)
        }
        RemoteDesktopTargetKind::Application => {
            validate_application_input(binding, snapshot, &host_snapshot.windows)
        }
    }
}

fn validate_pointer_target_against_host_snapshot(
    binding: &RemoteAppTargetBinding,
    host_snapshot: &HostTargetSnapshot,
    host_x: f64,
    host_y: f64,
) -> Result<u64, TargetInputGuardFailure> {
    if !host_x.is_finite() || !host_y.is_finite() {
        return Err(TargetInputGuardFailure::PointerOutsideTargetSurface);
    }
    let topmost = host_snapshot
        .windows
        .iter()
        .find(|window| {
            window.visibility_state == TargetVisibilityState::Visible
                && geometry_contains_point(&window.geometry, host_x, host_y)
        })
        .ok_or(TargetInputGuardFailure::PointerOutsideTargetSurface)?;
    let belongs_to_target = match binding.target_kind() {
        RemoteDesktopTargetKind::Display => false,
        RemoteDesktopTargetKind::Window => {
            binding.native_locator().window_id() == Some(topmost.window_id)
                && owner_matches(binding, topmost)
        }
        RemoteDesktopTargetKind::Application => {
            binding
                .committed_app_window_set()
                .is_some_and(|window_set| window_set.contains_window_id(topmost.window_id))
                && app_owner_matches(binding, topmost)
        }
    };
    belongs_to_target
        .then_some(topmost.window_id)
        .ok_or(TargetInputGuardFailure::PointerOccluded)
}

fn geometry_contains_point(geometry: &TargetGeometry, x: f64, y: f64) -> bool {
    let (Some(origin_x), Some(origin_y), Some(width), Some(height)) = (
        finite_dimension(geometry.x),
        finite_dimension(geometry.y),
        positive_dimension(geometry.width),
        positive_dimension(geometry.height),
    ) else {
        return false;
    };
    x >= origin_x && y >= origin_y && x < origin_x + width && y < origin_y + height
}

fn validate_window_input(
    binding: &RemoteAppTargetBinding,
    snapshot: &TargetTrackerSnapshot,
    windows: &[ObservedWindow],
) -> Result<(), TargetInputGuardFailure> {
    let window_id = binding
        .native_locator()
        .window_id()
        .ok_or(TargetInputGuardFailure::IdentityMismatch)?;
    let window = windows
        .iter()
        .find(|window| window.window_id == window_id)
        .ok_or(TargetInputGuardFailure::TargetNotFound)?;
    if !owner_matches(binding, window) {
        return Err(TargetInputGuardFailure::IdentityMismatch);
    }
    if window.visibility_state != TargetVisibilityState::Visible {
        return Err(TargetInputGuardFailure::NotVisible);
    }
    if !window.focused {
        return Err(TargetInputGuardFailure::NotFocused);
    }
    if snapshot.geometry() != &window.geometry {
        return Err(TargetInputGuardFailure::GeometryStale);
    }
    Ok(())
}

fn validate_application_input(
    binding: &RemoteAppTargetBinding,
    snapshot: &TargetTrackerSnapshot,
    windows: &[ObservedWindow],
) -> Result<(), TargetInputGuardFailure> {
    let locator = binding.native_locator();
    let display_id = locator.display_id();
    let committed_window_set = binding
        .committed_app_window_set()
        .ok_or(TargetInputGuardFailure::IdentityMismatch)?;
    let owner_windows = windows
        .iter()
        .filter(|window| app_owner_matches(binding, window))
        .collect::<Vec<_>>();
    if display_id.is_some_and(|expected_display| {
        owner_windows.iter().any(|window| {
            window
                .display_id
                .is_some_and(|observed_display| observed_display != expected_display)
        })
    }) {
        return Err(TargetInputGuardFailure::WindowSetStale);
    }
    let matching = owner_windows
        .into_iter()
        .filter(|window| display_id.is_none_or(|expected| window.display_id == Some(expected)))
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return Err(TargetInputGuardFailure::TargetNotFound);
    }
    let visible = matching
        .into_iter()
        .filter(|window| window.visibility_state == TargetVisibilityState::Visible)
        .collect::<Vec<_>>();
    if visible.is_empty() {
        return Err(TargetInputGuardFailure::NotVisible);
    }
    let current_window_set = AppWindowSetProof::new_platform_scoped(
        display_id,
        locator.bundle_id().map(str::to_string),
        locator.pid(),
        visible.iter().map(|window| window.window_id).collect(),
    );
    if &current_window_set != committed_window_set {
        return Err(TargetInputGuardFailure::WindowSetStale);
    }
    let current_layout =
        application_surface_layout(&visible).ok_or(TargetInputGuardFailure::GeometryStale)?;
    if binding
        .committed_app_surface_layout()
        .is_some_and(|committed| committed != &current_layout)
    {
        return Err(TargetInputGuardFailure::GeometryStale);
    }
    if !visible.iter().any(|window| window.focused) {
        return Err(TargetInputGuardFailure::NotFocused);
    }
    let geometry = union_geometry(&visible).ok_or(TargetInputGuardFailure::GeometryStale)?;
    if snapshot.geometry() != &geometry {
        return Err(TargetInputGuardFailure::GeometryStale);
    }
    Ok(())
}

#[cfg(test)]
#[derive(Debug)]
struct SnapshotBackedTargetObservationProvider<P> {
    snapshots: P,
}

#[cfg(test)]
impl<P> SnapshotBackedTargetObservationProvider<P> {
    fn new(snapshots: P) -> Self {
        Self { snapshots }
    }
}

#[cfg(test)]
impl<P> TargetObservationProvider for SnapshotBackedTargetObservationProvider<P>
where
    P: HostTargetSnapshotProvider,
{
    fn observe(
        &self,
        binding: &RemoteAppTargetBinding,
        snapshot: &TargetTrackerSnapshot,
    ) -> Option<TargetObservation> {
        let host_snapshot = match self.snapshots.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return Some(TargetObservation::Lost {
                    reason: TargetResolutionError::CaptureBackendUnavailable,
                    detail: format!("host target snapshot failed: {error}"),
                    observed_at_ms: now_ms(),
                });
            }
        };
        observe_binding_against_host_snapshot(binding, snapshot, &host_snapshot)
    }
}

/// Sample one session target independently from media transport state.
///
/// Returns false only when the session is missing or terminal, allowing the
/// plugin-owned TargetMonitor to stop tracking without relying on WebRTC epoch.
pub(in crate::daemon::plugins::remote_desktop) fn observe_bound_session_target_once<P>(
    sessions: &RemoteDesktopSessionStore,
    session_id: &str,
    provider: &P,
) -> TargetObservationPollResult
where
    P: TargetObservationProvider,
{
    let Some(inputs) = sessions.target_observation_inputs_for_session(session_id) else {
        return TargetObservationPollResult::stop_tracking();
    };
    if let Some(expiration) = sessions.expire_target_rebind_deadline_for_session(
        session_id,
        &inputs.binding_id,
        inputs.binding_epoch,
        now_ms(),
    ) {
        return TargetObservationPollResult::rebind_deadline_expired(
            expiration.into_media_source_lost(),
        );
    }
    let Some(observation) = provider.observe(&inputs.binding, &inputs.snapshot) else {
        return TargetObservationPollResult::keep_tracking();
    };
    let commit = sessions.commit_target_observation_for_session(
        session_id,
        &inputs.binding_id,
        inputs.binding_epoch,
        observation,
    );
    TargetObservationPollResult {
        keep_tracking: true,
        state_changed: commit.as_ref().is_some_and(|commit| commit.state_changed),
        media_source_lost: commit.and_then(|commit| commit.media_source_lost),
    }
}

fn observe_binding_against_host_snapshot(
    binding: &RemoteAppTargetBinding,
    snapshot: &TargetTrackerSnapshot,
    host_snapshot: &HostTargetSnapshot,
) -> Option<TargetObservation> {
    if let Some(selected_display_id) = binding.native_locator().display_id() {
        if !host_snapshot.display_ids.contains(&selected_display_id) {
            return Some(TargetObservation::DisplayTopologyChanged {
                available_display_ids: host_snapshot.display_ids.iter().copied().collect(),
                selected_display_available: false,
                observed_at_ms: now_ms(),
            });
        }
    }
    match binding.target_kind() {
        RemoteDesktopTargetKind::Display => None,
        RemoteDesktopTargetKind::Window => {
            observe_window(binding, snapshot, &host_snapshot.windows)
        }
        RemoteDesktopTargetKind::Application => {
            observe_application(binding, snapshot, &host_snapshot.windows)
        }
    }
}

#[cfg(any(test, not(target_os = "macos")))]
fn unsupported_platform_target_observation(
    binding: &RemoteAppTargetBinding,
) -> Option<TargetObservation> {
    match binding.target_kind() {
        RemoteDesktopTargetKind::Display => None,
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
            Some(TargetObservation::Lost {
                reason: TargetResolutionError::UnsupportedCaptureScope,
                detail: format!(
                    "platform target observer cannot validate {} scoped capture",
                    binding.target_kind().as_str()
                ),
                observed_at_ms: now_ms(),
            })
        }
    }
}

fn observe_window(
    binding: &RemoteAppTargetBinding,
    snapshot: &TargetTrackerSnapshot,
    windows: &[ObservedWindow],
) -> Option<TargetObservation> {
    let locator = binding.native_locator();
    let expected_id = locator.window_id()?;
    let Some(window) = windows
        .iter()
        .find(|window| window.window_id == expected_id)
    else {
        return Some(lost(
            TargetResolutionError::TargetNotFound,
            "bound window is no longer present in host target snapshot",
        ));
    };
    if !owner_matches(binding, window) {
        return Some(lost(
            TargetResolutionError::TargetIdentityMismatch,
            "bound window owner identity changed",
        ));
    }
    if window.visibility_state != TargetVisibilityState::Visible {
        return Some(TargetObservation::VisibilityChanged {
            visibility_state: window.visibility_state,
            target_geometry_revision: snapshot.target_geometry_revision() + 1,
            observed_at_ms: now_ms(),
        });
    }
    if snapshot.title() != window.title.as_deref() {
        return Some(TargetObservation::TitleChanged {
            title: window.title.clone(),
            observed_at_ms: now_ms(),
        });
    }
    if snapshot.focused() != Some(window.focused) {
        return Some(TargetObservation::FocusChanged {
            focused: window.focused,
            observed_at_ms: now_ms(),
        });
    }
    geometry_observation(snapshot, window.geometry.clone())
}

fn observe_application(
    binding: &RemoteAppTargetBinding,
    snapshot: &TargetTrackerSnapshot,
    windows: &[ObservedWindow],
) -> Option<TargetObservation> {
    let locator = binding.native_locator();
    let Some(committed_window_set) = binding.committed_app_window_set() else {
        return Some(lost(
            TargetResolutionError::TargetMetadataIncomplete,
            "application target binding has no committed application window set",
        ));
    };
    let matching: Vec<&ObservedWindow> = windows
        .iter()
        .filter(|window| app_owner_matches(binding, window))
        .collect();
    if matching.is_empty() {
        return Some(lost(
            TargetResolutionError::TargetNotFound,
            "bound application has no visible windows in host target snapshot",
        ));
    }
    let visible_application_windows: Vec<&ObservedWindow> = matching
        .iter()
        .copied()
        .filter(|window| window.visibility_state == TargetVisibilityState::Visible)
        .collect();
    if visible_application_windows.is_empty() {
        let visibility_state = if matching
            .iter()
            .any(|window| window.visibility_state == TargetVisibilityState::Minimized)
        {
            TargetVisibilityState::Minimized
        } else {
            TargetVisibilityState::Hidden
        };
        return Some(TargetObservation::VisibilityChanged {
            visibility_state,
            target_geometry_revision: snapshot.target_geometry_revision() + 1,
            observed_at_ms: now_ms(),
        });
    }
    let Some(geometry) = union_geometry(&visible_application_windows) else {
        return Some(lost(
            TargetResolutionError::TargetMetadataIncomplete,
            "bound application window set has incomplete geometry in host target snapshot",
        ));
    };
    let Some(app_surface_layout) = application_surface_layout(&visible_application_windows) else {
        return Some(lost(
            TargetResolutionError::TargetMetadataIncomplete,
            "bound application surface has incomplete native geometry or duplicate windows",
        ));
    };
    let application_window_ids: BTreeSet<u64> = visible_application_windows
        .iter()
        .map(|window| window.window_id)
        .collect();
    let current_window_set = AppWindowSetProof::new_platform_scoped(
        None,
        locator.bundle_id().map(str::to_string),
        locator.pid(),
        application_window_ids.into_iter().collect(),
    );
    if &current_window_set != committed_window_set
        || binding
            .committed_app_surface_layout()
            .is_some_and(|committed| committed != &app_surface_layout)
        || snapshot.geometry() != &geometry
    {
        return Some(TargetObservation::ApplicationSurfaceChanged {
            target_identity_epoch: current_window_set.window_set_epoch(),
            app_window_set: current_window_set,
            app_surface_layout: Some(app_surface_layout),
            geometry,
            target_geometry_revision: snapshot.target_geometry_revision() + 1,
            observed_at_ms: now_ms(),
        });
    }
    let focused = visible_application_windows
        .iter()
        .any(|window| window.focused);
    if snapshot.focused() != Some(focused) {
        return Some(TargetObservation::FocusChanged {
            focused,
            observed_at_ms: now_ms(),
        });
    }
    geometry_observation(snapshot, geometry)
}

fn geometry_observation(
    snapshot: &TargetTrackerSnapshot,
    geometry: TargetGeometry,
) -> Option<TargetObservation> {
    if snapshot.geometry() == &geometry {
        return Some(TargetObservation::VisibilityChanged {
            visibility_state: TargetVisibilityState::Visible,
            target_geometry_revision: snapshot.target_geometry_revision(),
            observed_at_ms: now_ms(),
        });
    }
    Some(TargetObservation::GeometryChanged {
        geometry,
        target_geometry_revision: snapshot.target_geometry_revision() + 1,
        observed_at_ms: now_ms(),
    })
}

fn lost(reason: TargetResolutionError, detail: &'static str) -> TargetObservation {
    TargetObservation::Lost {
        reason,
        detail: detail.to_string(),
        observed_at_ms: now_ms(),
    }
}

fn owner_matches(binding: &RemoteAppTargetBinding, window: &ObservedWindow) -> bool {
    app_owner_matches(binding, window)
}

fn app_owner_matches(binding: &RemoteAppTargetBinding, window: &ObservedWindow) -> bool {
    binding
        .native_locator()
        .app_identity_expectation()
        .evaluate(NativeAppIdentityCandidate::new(
            window.pid,
            window.bundle_id.as_deref(),
            None,
        ))
        .matched()
}

fn union_geometry(windows: &[&ObservedWindow]) -> Option<TargetGeometry> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for window in windows {
        let x = finite_dimension(window.geometry.x)?;
        let y = finite_dimension(window.geometry.y)?;
        let width = positive_dimension(window.geometry.width)?;
        let height = positive_dimension(window.geometry.height)?;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x + width);
        max_y = max_y.max(y + height);
    }
    if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
        return None;
    }
    Some(TargetGeometry {
        x: Some(min_x),
        y: Some(min_y),
        width: Some((max_x - min_x).max(0.0)),
        height: Some((max_y - min_y).max(0.0)),
    })
}

fn application_surface_layout(windows: &[&ObservedWindow]) -> Option<AppSurfaceLayoutProof> {
    AppSurfaceLayoutProof::from_front_to_back_geometries(
        windows
            .iter()
            .map(|window| (window.window_id, &window.geometry)),
    )
}

fn finite_dimension(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite())
}

fn positive_dimension(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

#[cfg(target_os = "macos")]
mod platform {
    use std::collections::BTreeSet;
    use std::ffi::{c_char, c_void, CStr, CString};
    use std::ptr;

    use objc2_app_kit::{NSRunningApplication, NSWorkspace};

    use super::{
        sample_host_target_observations, HostTargetSnapshot, HostTargetSnapshotProvider,
        ObservedWindow, PlatformTargetObservationSample,
    };
    use crate::daemon::plugins::remote_desktop::target::TargetGeometry;
    use crate::daemon::plugins::remote_desktop::target_tracking::TargetVisibilityState;

    type CFArrayRef = *const c_void;
    type CFBooleanRef = *const c_void;
    type CFDictionaryRef = *const c_void;
    type CFIndex = isize;
    type CFNumberRef = *const c_void;
    type CFStringRef = *const c_void;
    type CFTypeID = usize;
    type CFTypeRef = *const c_void;
    type CGDirectDisplayID = u32;
    type CGError = i32;
    type CGWindowID = u32;

    const KCG_NULL_WINDOW_ID: CGWindowID = 0;
    const KCG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS: u32 = 1 << 4;
    const KCF_NUMBER_DOUBLE_TYPE: i32 = 13;
    const KCF_NUMBER_SINT64_TYPE: i32 = 4;
    const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relativeToWindow: CGWindowID) -> CFArrayRef;
        fn CGGetDisplaysWithRect(
            rect: CGRect,
            maxDisplays: u32,
            displays: *mut CGDirectDisplayID,
            matchingDisplayCount: *mut u32,
        ) -> CGError;
        fn CGGetActiveDisplayList(
            maxDisplays: u32,
            activeDisplays: *mut CGDirectDisplayID,
            displayCount: *mut u32,
        ) -> CGError;
        fn CGRectMakeWithDictionaryRepresentation(dict: CFDictionaryRef, rect: *mut CGRect) -> u8;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(array: CFArrayRef) -> CFIndex;
        fn CFArrayGetValueAtIndex(array: CFArrayRef, idx: CFIndex) -> *const c_void;
        fn CFBooleanGetTypeID() -> CFTypeID;
        fn CFBooleanGetValue(boolean: CFBooleanRef) -> u8;
        fn CFDictionaryGetValueIfPresent(
            dict: CFDictionaryRef,
            key: *const c_void,
            value: *mut *const c_void,
        ) -> u8;
        fn CFGetTypeID(value: CFTypeRef) -> CFTypeID;
        fn CFNumberGetTypeID() -> CFTypeID;
        fn CFNumberGetValue(number: CFNumberRef, theType: i32, valuePtr: *mut c_void) -> u8;
        fn CFRelease(value: *const c_void);
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            cStr: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringGetCString(
            theString: CFStringRef,
            buffer: *mut c_char,
            bufferSize: CFIndex,
            encoding: u32,
        ) -> u8;
        fn CFStringGetCStringPtr(theString: CFStringRef, encoding: u32) -> *const c_char;
        fn CFStringGetTypeID() -> CFTypeID;
    }

    struct CfOwned(*const c_void);

    impl CfOwned {
        fn new_string(value: &str) -> anyhow::Result<Self> {
            let value = CString::new(value)?;
            let ptr = unsafe {
                CFStringCreateWithCString(ptr::null(), value.as_ptr(), KCF_STRING_ENCODING_UTF8)
            };
            if ptr.is_null() {
                anyhow::bail!("CFStringCreateWithCString returned null");
            }
            Ok(Self(ptr))
        }

        fn as_ptr(&self) -> *const c_void {
            self.0
        }
    }

    impl Drop for CfOwned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    struct WindowKeys {
        alpha: CfOwned,
        bounds: CfOwned,
        layer: CfOwned,
        name: CfOwned,
        number: CfOwned,
        onscreen: CfOwned,
        owner_pid: CfOwned,
    }

    impl WindowKeys {
        fn new() -> anyhow::Result<Self> {
            Ok(Self {
                alpha: CfOwned::new_string("kCGWindowAlpha")?,
                bounds: CfOwned::new_string("kCGWindowBounds")?,
                layer: CfOwned::new_string("kCGWindowLayer")?,
                name: CfOwned::new_string("kCGWindowName")?,
                number: CfOwned::new_string("kCGWindowNumber")?,
                onscreen: CfOwned::new_string("kCGWindowIsOnscreen")?,
                owner_pid: CfOwned::new_string("kCGWindowOwnerPID")?,
            })
        }
    }

    #[derive(Debug, Clone, Copy, Default)]
    struct MacOsHostTargetSnapshotProvider;

    pub(super) fn sample_platform_target_observations() -> PlatformTargetObservationSample {
        if !crate::daemon::plugins::remote_desktop::screencapturekit_capture::screen_capture_permission_granted() {
            return PlatformTargetObservationSample::permission_revoked(
                "macOS Screen Recording permission is no longer granted",
            );
        }
        sample_host_target_observations(&MacOsHostTargetSnapshotProvider)
    }

    impl HostTargetSnapshotProvider for MacOsHostTargetSnapshotProvider {
        fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot> {
            Ok(HostTargetSnapshot {
                windows: observed_windows()?,
                display_ids: active_display_ids()?,
            })
        }
    }

    fn observed_windows() -> anyhow::Result<Vec<ObservedWindow>> {
        let keys = WindowKeys::new()?;
        let array = unsafe {
            CGWindowListCopyWindowInfo(KCG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS, KCG_NULL_WINDOW_ID)
        };
        if array.is_null() {
            anyhow::bail!("CGWindowListCopyWindowInfo returned null");
        }
        let array = CfOwned(array);
        let count = unsafe { CFArrayGetCount(array.as_ptr()) };
        if count < 0 {
            anyhow::bail!("CGWindowListCopyWindowInfo returned a negative count");
        }
        let mut windows = Vec::new();
        let frontmost_pid = NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|application| i64::from(application.processIdentifier()));
        let mut focused_regular_window_selected = false;
        for idx in 0..count {
            let dict = unsafe { CFArrayGetValueAtIndex(array.as_ptr(), idx) as CFDictionaryRef };
            if dict.is_null() {
                continue;
            }
            let Some(window_id) =
                get_i64(dict, keys.number.as_ptr()).and_then(|value| u64::try_from(value).ok())
            else {
                continue;
            };
            let Some(rect) = get_rect(dict, keys.bounds.as_ptr()) else {
                continue;
            };
            let layer = get_i64(dict, keys.layer.as_ptr()).unwrap_or(0);
            let alpha = get_f64(dict, keys.alpha.as_ptr()).unwrap_or(1.0);
            let onscreen = get_bool(dict, keys.onscreen.as_ptr()).unwrap_or(false);
            let pid = get_i64(dict, keys.owner_pid.as_ptr()).filter(|value| *value >= 0);
            let focused = !focused_regular_window_selected
                && pid.is_some()
                && pid == frontmost_pid
                && layer == 0
                && alpha > 0.01
                && onscreen;
            if focused {
                focused_regular_window_selected = true;
            }
            let bundle_id = pid
                .and_then(|pid| u32::try_from(pid).ok())
                .and_then(bundle_id_for_pid);
            windows.push(ObservedWindow {
                window_id,
                pid,
                bundle_id,
                display_id: display_id_for_rect(rect).map(u64::from),
                title: get_string(dict, keys.name.as_ptr()),
                focused,
                geometry: TargetGeometry {
                    x: Some(rect.origin.x.round()),
                    y: Some(rect.origin.y.round()),
                    width: positive_dimension(rect.size.width).map(f64::from),
                    height: positive_dimension(rect.size.height).map(f64::from),
                },
                visibility_state: if layer != 0 || alpha <= 0.01 {
                    TargetVisibilityState::Hidden
                } else if !onscreen {
                    TargetVisibilityState::Minimized
                } else {
                    TargetVisibilityState::Visible
                },
            });
        }
        Ok(windows)
    }

    fn active_display_ids() -> anyhow::Result<BTreeSet<u64>> {
        let mut displays = [0_u32; 32];
        let mut count = 0_u32;
        let error = unsafe {
            CGGetActiveDisplayList(displays.len() as u32, displays.as_mut_ptr(), &mut count)
        };
        if error != 0 {
            anyhow::bail!("CGGetActiveDisplayList failed with {error}");
        }
        Ok(
            displays[..usize::try_from(count).unwrap_or(0).min(displays.len())]
                .iter()
                .copied()
                .filter(|display_id| *display_id != 0)
                .map(u64::from)
                .collect(),
        )
    }

    fn display_id_for_rect(rect: CGRect) -> Option<u32> {
        let mut displays = [0_u32; 8];
        let mut count = 0_u32;
        let error = unsafe {
            CGGetDisplaysWithRect(
                rect,
                displays.len() as u32,
                displays.as_mut_ptr(),
                &mut count,
            )
        };
        if error != 0 || count == 0 {
            return None;
        }
        displays
            .first()
            .copied()
            .filter(|display_id| *display_id != 0)
    }

    fn bundle_id_for_pid(pid: u32) -> Option<String> {
        let app =
            NSRunningApplication::runningApplicationWithProcessIdentifier(pid as libc::pid_t)?;
        app.bundleIdentifier()
            .map(|bundle_id| bundle_id.to_string())
            .map(|bundle_id| bundle_id.trim().to_string())
            .filter(|bundle_id| !bundle_id.is_empty())
    }

    fn positive_dimension(value: f64) -> Option<u32> {
        if value.is_finite() && value > 0.0 && value <= f64::from(u32::MAX) {
            Some(value.round() as u32)
        } else {
            None
        }
    }

    fn get_value(dict: CFDictionaryRef, key: *const c_void) -> Option<*const c_void> {
        let mut value = ptr::null();
        let found = unsafe { CFDictionaryGetValueIfPresent(dict, key, &mut value) };
        (found != 0 && !value.is_null()).then_some(value)
    }

    fn get_i64(dict: CFDictionaryRef, key: *const c_void) -> Option<i64> {
        let value = get_value(dict, key)? as CFNumberRef;
        let is_number = unsafe { CFGetTypeID(value) == CFNumberGetTypeID() };
        if !is_number {
            return None;
        }
        let mut out = 0_i64;
        let ok = unsafe {
            CFNumberGetValue(
                value,
                KCF_NUMBER_SINT64_TYPE,
                &mut out as *mut i64 as *mut c_void,
            )
        };
        (ok != 0).then_some(out)
    }

    fn get_bool(dict: CFDictionaryRef, key: *const c_void) -> Option<bool> {
        let value = get_value(dict, key)? as CFBooleanRef;
        let is_boolean = unsafe { CFGetTypeID(value) == CFBooleanGetTypeID() };
        is_boolean.then(|| unsafe { CFBooleanGetValue(value) != 0 })
    }

    fn get_string(dict: CFDictionaryRef, key: *const c_void) -> Option<String> {
        let value = get_value(dict, key)? as CFStringRef;
        let is_string = unsafe { CFGetTypeID(value) == CFStringGetTypeID() };
        if !is_string {
            return None;
        }
        let ptr = unsafe { CFStringGetCStringPtr(value, KCF_STRING_ENCODING_UTF8) };
        if !ptr.is_null() {
            return unsafe { CStr::from_ptr(ptr) }
                .to_str()
                .ok()
                .map(str::to_string);
        }
        let mut buffer = [0 as c_char; 4096];
        let ok = unsafe {
            CFStringGetCString(
                value,
                buffer.as_mut_ptr(),
                buffer.len() as CFIndex,
                KCF_STRING_ENCODING_UTF8,
            )
        };
        if ok == 0 {
            return None;
        }
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_str()
            .ok()
            .map(str::to_string)
    }

    fn get_f64(dict: CFDictionaryRef, key: *const c_void) -> Option<f64> {
        let value = get_value(dict, key)? as CFNumberRef;
        let is_number = unsafe { CFGetTypeID(value) == CFNumberGetTypeID() };
        if !is_number {
            return None;
        }
        let mut out = 0.0_f64;
        let ok = unsafe {
            CFNumberGetValue(
                value,
                KCF_NUMBER_DOUBLE_TYPE,
                &mut out as *mut f64 as *mut c_void,
            )
        };
        (ok != 0).then_some(out)
    }

    fn get_rect(dict: CFDictionaryRef, key: *const c_void) -> Option<CGRect> {
        let value = get_value(dict, key)? as CFDictionaryRef;
        let mut rect = CGRect::default();
        let ok = unsafe { CGRectMakeWithDictionaryRepresentation(value, &mut rect) };
        (ok != 0).then_some(rect)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    use serde_json::json;

    use super::{
        observe_binding_against_host_snapshot, unsupported_platform_target_observation,
        validate_pointer_target_against_host_snapshot, validate_target_input_against_host_snapshot,
        HostTargetSnapshot, HostTargetSnapshotProvider, ObservedWindow,
        SnapshotBackedTargetObservationProvider, TargetInputGuardFailure, TargetInputGuardProof,
    };
    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use crate::daemon::plugins::remote_desktop::constants::direct_webrtc_endpoint_ura;
    use crate::daemon::plugins::remote_desktop::session::{
        RemoteDesktopSession, RemoteDesktopState,
    };
    use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
    use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
    use crate::daemon::plugins::remote_desktop::target::{
        AppSurfaceLayoutProof, AppWindowSetProof, RemoteAppTargetBinding, RemoteDesktopTargetKind,
        ResolvedCaptureTargetProof, ResourceEntryTargetResolver, TargetGeometry,
        TargetResolutionError,
    };
    use crate::daemon::plugins::remote_desktop::target_observer::{
        observe_bound_session_target_once, sample_host_target_observations,
        TargetObservationProvider,
    };
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        RemoteAppTargetBindingStateMachine, TargetObservation, TargetTrackerSnapshot,
        TargetVisibilityState,
    };
    use crate::daemon::plugins::remote_desktop::test_support::{
        live_remote_target_metadata, test_application_session_init, test_session_init,
    };

    struct FakeGeometryProvider;

    struct CountingObservationProvider {
        calls: Arc<AtomicUsize>,
    }

    #[derive(Debug)]
    struct CountingSnapshotProvider {
        calls: Arc<AtomicUsize>,
    }

    #[derive(Debug)]
    struct QueuedSnapshotProvider {
        snapshots: Mutex<VecDeque<HostTargetSnapshot>>,
    }

    impl QueuedSnapshotProvider {
        fn new(snapshots: Vec<HostTargetSnapshot>) -> Self {
            Self {
                snapshots: Mutex::new(VecDeque::from(snapshots)),
            }
        }
    }

    impl HostTargetSnapshotProvider for CountingSnapshotProvider {
        fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(HostTargetSnapshot {
                windows: Vec::new(),
                display_ids: BTreeSet::new(),
            })
        }
    }

    impl HostTargetSnapshotProvider for QueuedSnapshotProvider {
        fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot> {
            self.snapshots
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("queued host target snapshot exhausted"))
        }
    }

    impl TargetObservationProvider for FakeGeometryProvider {
        fn observe(
            &self,
            _binding: &RemoteAppTargetBinding,
            snapshot: &TargetTrackerSnapshot,
        ) -> Option<TargetObservation> {
            Some(TargetObservation::GeometryChanged {
                geometry: TargetGeometry {
                    x: Some(10.0),
                    y: Some(20.0),
                    width: Some(300.0),
                    height: Some(200.0),
                },
                target_geometry_revision: snapshot.target_geometry_revision() + 1,
                observed_at_ms: 123,
            })
        }
    }

    impl TargetObservationProvider for CountingObservationProvider {
        fn observe(
            &self,
            _binding: &RemoteAppTargetBinding,
            _snapshot: &TargetTrackerSnapshot,
        ) -> Option<TargetObservation> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            None
        }
    }

    struct LostTargetProvider;

    impl TargetObservationProvider for LostTargetProvider {
        fn observe(
            &self,
            _binding: &RemoteAppTargetBinding,
            _snapshot: &TargetTrackerSnapshot,
        ) -> Option<TargetObservation> {
            Some(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "target disappeared from observer".into(),
                observed_at_ms: 456,
            })
        }
    }

    struct ReplacingGeometryProvider {
        store: Arc<RemoteDesktopSessionStore>,
        session_id: &'static str,
    }

    impl TargetObservationProvider for ReplacingGeometryProvider {
        fn observe(
            &self,
            _binding: &RemoteAppTargetBinding,
            snapshot: &TargetTrackerSnapshot,
        ) -> Option<TargetObservation> {
            let replacement = RemoteDesktopSession::new(test_session_init(
                self.session_id,
                "easynet:///r/acme/resource/display.reused",
                vec!["webrtc".into()],
            ));
            self.store.with_sessions(|sessions| {
                sessions.insert(self.session_id.to_string(), replacement);
            });
            Some(TargetObservation::GeometryChanged {
                geometry: TargetGeometry {
                    x: Some(10.0),
                    y: Some(20.0),
                    width: Some(300.0),
                    height: Some(200.0),
                },
                target_geometry_revision: snapshot.target_geometry_revision() + 1,
                observed_at_ms: 123,
            })
        }
    }

    fn window_binding() -> RemoteAppTargetBinding {
        ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &ResourceEntry {
                    resource_ura: "easynet:///r/acme/resource/window.editor".to_string(),
                    owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
                    kind: ResourceType::Window,
                    binding: ResourceBinding::LocalDevice,
                    hardware_id: "window:macos:cgwindow:9001:10".to_string(),
                    display_name: "Editor window".to_string(),
                    metadata: live_remote_target_metadata(json!({
                        "platform": "macos",
                        "backend": "macos_core_graphics",
                        "window_id": 10,
                        "pid": 9001,
                        "bundle_id": "com.example.Editor",
                        "app_identity": "com.example.Editor",
                        "app_name": "Editor",
                        "title": "Old title",
                        "x": 10,
                        "y": 20,
                        "width": 100,
                        "height": 80,
                        "geometry_revision": 1,
                    })),
                    first_seen_at: "2026-06-01T00:00:00Z".to_string(),
                },
                "interactive",
                1,
            )
            .expect("window target binding resolves")
    }

    fn application_binding() -> RemoteAppTargetBinding {
        let window_set_epoch = AppWindowSetProof::new_platform_scoped(
            None,
            Some("com.example.Editor".to_string()),
            Some(9001),
            vec![10, 11],
        )
        .window_set_epoch();
        ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &ResourceEntry {
                    resource_ura: "easynet:///r/acme/resource/application.editor".to_string(),
                    owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
                    kind: ResourceType::Application,
                    binding: ResourceBinding::LocalDevice,
                    hardware_id: "application:macos:cgwindow:bundle:com.example.Editor".to_string(),
                    display_name: "Editor".to_string(),
                    metadata: live_remote_target_metadata(json!({
                        "platform": "macos",
                        "backend": "macos_core_graphics",
                        "display_ids": [42],
                        "bundle_id": "com.example.Editor",
                        "app_identity": "com.example.Editor",
                        "primary_pid": 9001,
                        "resolved_window_ids": [10, 11],
                        "window_set_epoch": window_set_epoch,
                        "union_x": 10,
                        "union_y": 20,
                        "union_width": 190,
                        "union_height": 80,
                    })),
                    first_seen_at: "2026-06-01T00:00:00Z".to_string(),
                },
                "view_only",
                1,
            )
            .expect("application target binding resolves")
    }

    fn process_scoped_application_binding() -> RemoteAppTargetBinding {
        let window_set_epoch =
            AppWindowSetProof::new_platform_scoped(None, None, Some(9001), vec![10, 11])
                .window_set_epoch();
        ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &ResourceEntry {
                    resource_ura: "easynet:///r/acme/resource/application.editor-windows"
                        .to_string(),
                    owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
                    kind: ResourceType::Application,
                    binding: ResourceBinding::LocalDevice,
                    hardware_id: "application:xcap:windows:pid:9001".to_string(),
                    display_name: "Editor".to_string(),
                    metadata: live_remote_target_metadata(json!({
                        "platform": "windows",
                        "backend": "xcap",
                        "app_name": "Editor",
                        "primary_pid": 9001,
                        "resolved_window_ids": [10, 11],
                        "window_set_epoch": window_set_epoch,
                        "primary_x": 10,
                        "primary_y": 20,
                        "primary_width": 100,
                        "primary_height": 80,
                    })),
                    first_seen_at: "2026-06-01T00:00:00Z".to_string(),
                },
                "view_only",
                1,
            )
            .expect("process-scoped application target binding resolves")
    }

    fn visible_window_snapshot() -> HostTargetSnapshot {
        HostTargetSnapshot {
            windows: vec![ObservedWindow {
                window_id: 10,
                pid: Some(9001),
                bundle_id: Some("com.example.Editor".to_string()),
                display_id: Some(42),
                title: Some("Old title".to_string()),
                focused: false,
                geometry: TargetGeometry {
                    x: Some(10.0),
                    y: Some(20.0),
                    width: Some(100.0),
                    height: Some(80.0),
                },
                visibility_state: TargetVisibilityState::Visible,
            }],
            display_ids: BTreeSet::from([42]),
        }
    }

    fn app_window(window_id: u64, x: f64, width: f64) -> ObservedWindow {
        ObservedWindow {
            window_id,
            pid: Some(9001),
            bundle_id: Some("com.example.Editor".to_string()),
            display_id: Some(42),
            title: Some(format!("Editor window {window_id}")),
            focused: false,
            geometry: TargetGeometry {
                x: Some(x),
                y: Some(20.0),
                width: Some(width),
                height: Some(80.0),
            },
            visibility_state: TargetVisibilityState::Visible,
        }
    }

    fn commit_application_surface_layout(
        binding: &mut RemoteAppTargetBinding,
        windows: &[ObservedWindow],
    ) -> AppSurfaceLayoutProof {
        let layout = AppSurfaceLayoutProof::from_front_to_back_geometries(
            windows
                .iter()
                .map(|window| (window.window_id, &window.geometry)),
        )
        .expect("valid test application surface layout");
        let window_set = binding
            .committed_app_window_set()
            .cloned()
            .expect("application binding window set");
        binding
            .commit_capture_proof(
                "remote_desktop.create_session",
                ResolvedCaptureTargetProof::new(
                    "screencapturekit",
                    RemoteDesktopTargetKind::Application,
                )
                .with_native_identity(
                    None,
                    None,
                    Some(9001),
                    Some("com.example.Editor".to_string()),
                    Some("com.example.Editor".to_string()),
                )
                .with_app_window_set(window_set)
                .with_app_surface_layout(layout.clone()),
            )
            .expect("test capture proof commits");
        layout
    }

    fn no_window_snapshot() -> HostTargetSnapshot {
        HostTargetSnapshot {
            windows: Vec::new(),
            display_ids: BTreeSet::from([42]),
        }
    }

    fn focused_snapshot(binding: &RemoteAppTargetBinding) -> TargetTrackerSnapshot {
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding.clone());
        tracker.commit_observation(TargetObservation::FocusChanged {
            focused: true,
            observed_at_ms: 10,
        });
        tracker.snapshot().clone()
    }

    #[test]
    fn target_input_guard_accepts_only_exact_focused_window_state() {
        let binding = window_binding();
        let snapshot = focused_snapshot(&binding);
        let mut host = visible_window_snapshot();
        host.windows[0].focused = true;

        assert_eq!(
            validate_target_input_against_host_snapshot(&binding, &snapshot, &host),
            Ok(())
        );

        host.windows[0].focused = false;
        host.windows.push(ObservedWindow {
            window_id: 11,
            focused: true,
            ..host.windows[0].clone()
        });
        assert_eq!(
            validate_target_input_against_host_snapshot(&binding, &snapshot, &host),
            Err(TargetInputGuardFailure::NotFocused),
            "frontmost process identity is insufficient when another app window is focused"
        );
    }

    #[test]
    fn target_input_guard_rejects_geometry_drift_before_pointer_dispatch() {
        let binding = window_binding();
        let snapshot = focused_snapshot(&binding);
        let mut host = visible_window_snapshot();
        host.windows[0].focused = true;
        host.windows[0].geometry.x = Some(11.0);

        assert_eq!(
            validate_target_input_against_host_snapshot(&binding, &snapshot, &host),
            Err(TargetInputGuardFailure::GeometryStale)
        );
    }

    #[test]
    fn target_input_guard_binds_application_to_exact_focused_window_set() {
        let binding = application_binding();
        let snapshot = focused_snapshot(&binding);
        let mut first = app_window(10, 10.0, 100.0);
        first.focused = true;
        let host = HostTargetSnapshot {
            windows: vec![first, app_window(11, 110.0, 90.0)],
            display_ids: BTreeSet::from([42]),
        };

        assert_eq!(
            validate_target_input_against_host_snapshot(&binding, &snapshot, &host),
            Ok(())
        );

        let mut drifted = host.clone();
        drifted.windows.push(app_window(12, 210.0, 40.0));
        assert_eq!(
            validate_target_input_against_host_snapshot(&binding, &snapshot, &drifted),
            Err(TargetInputGuardFailure::WindowSetStale)
        );

        let mut multi_display = host;
        let mut other_display = app_window(12, 210.0, 40.0);
        other_display.display_id = Some(43);
        multi_display.windows.push(other_display);
        multi_display.display_ids.insert(43);
        assert_eq!(
            validate_target_input_against_host_snapshot(&binding, &snapshot, &multi_display),
            Err(TargetInputGuardFailure::WindowSetStale),
            "display-scoped application input must stop when the app spans displays"
        );
    }

    #[test]
    fn application_pointer_guard_rejects_black_gaps_and_occluding_windows() {
        let binding = application_binding();
        let first = app_window(10, 10.0, 100.0);
        let second = app_window(11, 210.0, 90.0);
        let mut unrelated = app_window(90, 0.0, 400.0);
        unrelated.pid = Some(7000);
        unrelated.bundle_id = Some("com.example.Other".to_string());

        let gap_with_underlying_window = HostTargetSnapshot {
            windows: vec![first.clone(), second.clone(), unrelated.clone()],
            display_ids: BTreeSet::from([42]),
        };
        assert_eq!(
            validate_pointer_target_against_host_snapshot(
                &binding,
                &gap_with_underlying_window,
                150.0,
                40.0,
            ),
            Err(TargetInputGuardFailure::PointerOccluded),
            "a black compositor gap must never click the desktop/other app underneath"
        );

        let empty_gap = HostTargetSnapshot {
            windows: vec![first.clone(), second.clone()],
            display_ids: BTreeSet::from([42]),
        };
        assert_eq!(
            validate_pointer_target_against_host_snapshot(&binding, &empty_gap, 150.0, 40.0),
            Err(TargetInputGuardFailure::PointerOutsideTargetSurface)
        );
        assert_eq!(
            validate_pointer_target_against_host_snapshot(&binding, &empty_gap, 50.0, 40.0),
            Ok(10)
        );

        let occluded_target = HostTargetSnapshot {
            windows: vec![unrelated, first, second],
            display_ids: BTreeSet::from([42]),
        };
        assert_eq!(
            validate_pointer_target_against_host_snapshot(&binding, &occluded_target, 50.0, 40.0),
            Err(TargetInputGuardFailure::PointerOccluded)
        );
    }

    #[test]
    fn target_input_guard_proof_projects_public_execution_evidence() {
        let binding = application_binding();
        let snapshot = focused_snapshot(&binding);
        let proof = TargetInputGuardProof::from_validated_target(&binding, &snapshot, 10, 11, None)
            .to_value("rd-target-input-proof");

        assert_eq!(proof["status"], json!("passed"));
        assert_eq!(proof["subject_ura"], json!(binding.subject_ura()));
        assert_eq!(proof["session_id"], json!("rd-target-input-proof"));
        assert_eq!(proof["target_kind"], json!("application"));
        assert_eq!(proof["identity_exact"], json!(true));
        assert_eq!(proof["visible"], json!(true));
        assert_eq!(proof["focused"], json!(true));
        assert_eq!(proof["window_set_exact"], json!(true));
        assert_eq!(
            proof["target_geometry_revision"],
            json!(snapshot.target_geometry_revision())
        );
        assert_eq!(
            proof["target_focus_epoch"],
            json!(snapshot.target_focus_epoch())
        );
        assert_eq!(proof["snapshot_started_at_ms"], json!(10));
        assert_eq!(proof["validated_at_ms"], json!(11));
    }

    #[test]
    fn application_observer_reports_committed_window_set_drift_as_rebind() {
        let binding = application_binding();
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let extra_window = HostTargetSnapshot {
            windows: vec![
                app_window(10, 10.0, 100.0),
                app_window(11, 120.0, 100.0),
                app_window(12, 240.0, 100.0),
            ],
            display_ids: BTreeSet::from([42]),
        };

        let extra_observation =
            observe_binding_against_host_snapshot(&binding, &snapshot, &extra_window)
                .expect("window-set drift must be reported");
        match extra_observation {
            TargetObservation::ApplicationSurfaceChanged {
                app_window_set,
                geometry,
                target_identity_epoch,
                ..
            } => {
                assert_eq!(app_window_set.resolved_window_count(), 3);
                assert_eq!(target_identity_epoch, app_window_set.window_set_epoch());
                assert_eq!(geometry.width, Some(330.0));
            }
            other => panic!("window-set expansion must project as app rebind: {other:?}"),
        }

        let missing_window = HostTargetSnapshot {
            windows: vec![app_window(10, 10.0, 100.0)],
            display_ids: BTreeSet::from([42]),
        };
        let missing_observation =
            observe_binding_against_host_snapshot(&binding, &snapshot, &missing_window)
                .expect("missing committed app window must be reported");
        match missing_observation {
            TargetObservation::ApplicationSurfaceChanged {
                app_window_set,
                geometry,
                target_identity_epoch,
                ..
            } => {
                assert_eq!(app_window_set.resolved_window_count(), 1);
                assert_eq!(target_identity_epoch, app_window_set.window_set_epoch());
                assert_eq!(geometry.width, Some(100.0));
            }
            other => panic!("window-set contraction must project as app rebind: {other:?}"),
        }
    }

    #[test]
    fn application_observer_rebinds_media_when_only_z_order_changes() {
        let mut binding = application_binding();
        let initial_windows = vec![app_window(10, 10.0, 100.0), app_window(11, 110.0, 90.0)];
        let initial_layout = commit_application_surface_layout(&mut binding, &initial_windows);
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let reordered = HostTargetSnapshot {
            windows: vec![initial_windows[1].clone(), initial_windows[0].clone()],
            display_ids: BTreeSet::from([42]),
        };

        let observation = observe_binding_against_host_snapshot(&binding, &snapshot, &reordered)
            .expect("z-order drift must rebuild the application media surface");
        match observation {
            TargetObservation::ApplicationSurfaceChanged {
                app_window_set,
                app_surface_layout: Some(app_surface_layout),
                target_identity_epoch,
                ..
            } => {
                assert_eq!(target_identity_epoch, app_window_set.window_set_epoch());
                assert_eq!(target_identity_epoch, binding.target_identity_epoch());
                assert_ne!(
                    app_surface_layout.layout_epoch(),
                    initial_layout.layout_epoch()
                );
            }
            other => panic!("z-order drift must stage an application surface rebind: {other:?}"),
        }
    }

    #[test]
    fn process_scoped_application_observer_tracks_window_set_without_display_identity() {
        let binding = process_scoped_application_binding();
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let host = HostTargetSnapshot {
            windows: [10_u64, 11, 12]
                .into_iter()
                .enumerate()
                .map(|(index, window_id)| ObservedWindow {
                    window_id,
                    pid: Some(9001),
                    bundle_id: None,
                    display_id: None,
                    title: Some(format!("Editor {window_id}")),
                    focused: index == 0,
                    geometry: TargetGeometry {
                        x: Some(10.0 + index as f64 * 110.0),
                        y: Some(20.0),
                        width: Some(100.0),
                        height: Some(80.0),
                    },
                    visibility_state: TargetVisibilityState::Visible,
                })
                .collect(),
            display_ids: BTreeSet::new(),
        };

        let observation = observe_binding_against_host_snapshot(&binding, &snapshot, &host)
            .expect("process window-set expansion must be observed");
        match observation {
            TargetObservation::ApplicationSurfaceChanged {
                app_window_set,
                geometry,
                ..
            } => {
                assert!(app_window_set.to_value()["display_id"].is_null());
                assert_eq!(app_window_set.resolved_window_count(), 3);
                assert_eq!(geometry.width, Some(320.0));
            }
            other => panic!("expected process-scoped application rebind, got {other:?}"),
        }
    }

    #[test]
    fn observation_provider_commits_through_session_store_boundary() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        let session = RemoteDesktopSession::new(test_session_init(
            "rd-provider-observation",
            "easynet:///r/acme/resource/display.provider",
            vec!["webrtc".into()],
        ));
        store.with_sessions(|sessions| {
            sessions.insert("rd-provider-observation".to_string(), session);
        });

        let result = observe_bound_session_target_once(
            &store,
            "rd-provider-observation",
            &FakeGeometryProvider,
        );
        assert!(result.keep_tracking);
        assert!(
            result.state_changed,
            "committed target events must request a durable recovery snapshot"
        );

        store.with_sessions(|sessions| {
            let session = sessions.get("rd-provider-observation").unwrap();
            assert_eq!(
                session.target_tracking_state()["target_geometry_revision"],
                json!(2)
            );
            assert!(session
                .events()
                .iter()
                .any(|event| event["event_type"] == json!("TARGET_RESIZED")));
        });
    }

    #[test]
    fn bound_session_observation_does_not_require_media_transport_epoch() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        let session = RemoteDesktopSession::new(test_session_init(
            "rd-bound-observation",
            "easynet:///r/acme/resource/display.bound",
            vec!["webrtc".into()],
        ));
        store.with_sessions(|sessions| {
            sessions.insert("rd-bound-observation".to_string(), session);
        });

        assert!(
            observe_bound_session_target_once(
                &store,
                "rd-bound-observation",
                &FakeGeometryProvider,
            )
            .keep_tracking
        );

        store.with_sessions(|sessions| {
            let session = sessions.get("rd-bound-observation").unwrap();
            assert_eq!(
                session.target_tracking_state()["target_geometry_revision"],
                json!(2)
            );
            assert!(session
                .events()
                .iter()
                .any(|event| event["event_type"] == json!("TARGET_RESIZED")));
        });
    }

    #[test]
    fn observer_stops_tracking_missing_or_terminal_sessions_without_polling_provider() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let provider = CountingObservationProvider {
            calls: Arc::clone(&calls),
        };

        let missing = observe_bound_session_target_once(&store, "rd-missing", &provider);
        assert!(
            !missing.keep_tracking,
            "missing sessions must stop target monitor tracking"
        );
        assert!(missing.media_source_lost.is_none());
        assert!(!missing.state_changed);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "missing sessions must not poll host target state"
        );

        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-terminal-observation",
            "easynet:///r/acme/resource/display.terminal",
            vec!["webrtc".into()],
        ));
        session.close("test_terminal");
        store.with_sessions(|sessions| {
            sessions.insert("rd-terminal-observation".to_string(), session);
        });

        let terminal =
            observe_bound_session_target_once(&store, "rd-terminal-observation", &provider);
        assert!(
            !terminal.keep_tracking,
            "terminal sessions must stop target monitor tracking"
        );
        assert!(terminal.media_source_lost.is_none());
        assert!(!terminal.state_changed);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "terminal sessions must not keep polling host target state"
        );
    }

    #[test]
    fn stale_observation_cannot_commit_after_session_binding_reuse() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        let session = RemoteDesktopSession::new(test_session_init(
            "rd-reused-observation",
            "easynet:///r/acme/resource/display.original",
            vec!["webrtc".into()],
        ));
        store.with_sessions(|sessions| {
            sessions.insert("rd-reused-observation".to_string(), session);
        });

        assert!(
            observe_bound_session_target_once(
                &store,
                "rd-reused-observation",
                &ReplacingGeometryProvider {
                    store: Arc::clone(&store),
                    session_id: "rd-reused-observation",
                },
            )
            .keep_tracking
        );

        store.with_sessions(|sessions| {
            let session = sessions.get("rd-reused-observation").unwrap();
            assert_eq!(
                session.subject_ura(),
                "easynet:///r/acme/resource/display.reused"
            );
            assert_eq!(
                session.target_tracking_state()["target_geometry_revision"],
                json!(1)
            );
            assert!(!session
                .events()
                .iter()
                .any(|event| event["event_type"] == json!("TARGET_RESIZED")));
        });
    }

    #[test]
    fn lost_observation_returns_media_source_stop_effect_after_debounce() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        let epoch = TransportEpoch::new(9);
        let mut session = RemoteDesktopSession::new(test_session_init(
            "rd-lost-observation",
            "easynet:///r/acme/resource/display.lost",
            vec!["webrtc".into()],
        ));
        session.begin_webrtc_negotiation(epoch);
        session.mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura("rd-lost-observation"));
        store.with_sessions(|sessions| {
            sessions.insert("rd-lost-observation".to_string(), session);
        });

        let first =
            observe_bound_session_target_once(&store, "rd-lost-observation", &LostTargetProvider);
        assert!(first.keep_tracking);
        assert!(first.media_source_lost.is_none());

        let second =
            observe_bound_session_target_once(&store, "rd-lost-observation", &LostTargetProvider);
        assert!(second.keep_tracking);
        let media_source_lost = second
            .media_source_lost
            .expect("debounced target loss must surface media source stop effect");
        assert_eq!(media_source_lost.transport_epoch, epoch);
        assert_eq!(
            media_source_lost.reason,
            TargetResolutionError::TargetNotFound
        );
    }

    #[test]
    fn snapshot_observer_reappearance_requires_explicit_rebind_policy() {
        let store = Arc::new(RemoteDesktopSessionStore::new());
        let epoch = TransportEpoch::new(17);
        let mut init = test_session_init(
            "rd-window-reappear",
            "easynet:///r/acme/resource/window.editor",
            vec!["webrtc".into()],
        );
        init.target_binding = window_binding();
        init.mode = "interactive".to_string();
        let mut session = RemoteDesktopSession::new(init);
        session.begin_webrtc_negotiation(epoch);
        session
            .set_local_webrtc_answer(
                epoch,
                json!({"type": "answer", "sdp": "v=0"}),
                "sck-native",
                true,
                "easynet:///r/acme/ability/remote-desktop.transport".into(),
            )
            .expect("local answer records");
        session.mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura("rd-window-reappear"));
        assert!(session.report_client_media_state(epoch, "presenting", None));
        assert!(
            session.production_media_ready(),
            "fixture starts with production media online before target loss"
        );
        store.with_sessions(|sessions| {
            sessions.insert("rd-window-reappear".to_string(), session);
        });

        let provider =
            SnapshotBackedTargetObservationProvider::new(QueuedSnapshotProvider::new(vec![
                no_window_snapshot(),
                no_window_snapshot(),
                visible_window_snapshot(),
                visible_window_snapshot(),
            ]));

        let first = observe_bound_session_target_once(&store, "rd-window-reappear", &provider);
        assert!(first.keep_tracking);
        assert!(
            first.media_source_lost.is_none(),
            "first lost snapshot is debounced"
        );

        let second = observe_bound_session_target_once(&store, "rd-window-reappear", &provider);
        assert!(second.keep_tracking);
        assert_eq!(
            second
                .media_source_lost
                .expect("second lost snapshot stops media source")
                .transport_epoch,
            epoch
        );

        std::thread::sleep(Duration::from_millis(2));

        let third = observe_bound_session_target_once(&store, "rd-window-reappear", &provider);
        assert!(third.keep_tracking);
        assert!(
            third.media_source_lost.is_none(),
            "rebind attempt must not restart or stop a second media source"
        );

        let fourth = observe_bound_session_target_once(&store, "rd-window-reappear", &provider);
        assert!(fourth.keep_tracking);
        assert!(
            fourth.media_source_lost.is_none(),
            "explicit rebind failure must not revive stale transport state"
        );

        store.with_sessions(|sessions| {
            let session = sessions
                .get("rd-window-reappear")
                .expect("session remains inspectable");
            assert_eq!(session.state(), RemoteDesktopState::Suspended);
            assert_eq!(
                session.transport_state()["primary"],
                json!("media_source_lost")
            );
            assert_eq!(
                session.target_tracking_state()["status"],
                json!("lost"),
                "same window id reappearing through the platform observer is not enough to restore the binding"
            );
            assert_eq!(
                session.target_tracking_state()["input_enabled"],
                json!(false)
            );
            assert!(!session.production_media_ready());

            let events = session.events();
            let rebind_attempted = events
                .iter()
                .find(|event| event["event_type"] == json!("TARGET_REBIND_ATTEMPTED"))
                .expect("observer-visible target reappearance attempts rebind");
            assert_eq!(
                rebind_attempted["payload"]["target_status"],
                json!("rebinding")
            );
            assert_eq!(
                rebind_attempted["payload"]["frontend_action"],
                json!("retry_session")
            );

            let rebind_failed = events
                .iter()
                .find(|event| event["event_type"] == json!("TARGET_REBIND_FAILED"))
                .expect("observer-visible reappearance without explicit rebind policy fails closed");
            assert_eq!(
                rebind_failed["payload"]["reason_code"],
                json!("explicit_rebind_required")
            );
            assert_eq!(
                rebind_failed["payload"]["frontend_action"],
                json!("refresh_targets")
            );
            assert_eq!(rebind_failed["payload"]["input_enabled"], json!(false));
        });
    }

    #[test]
    fn no_observation_tick_expires_rebind_deadline_before_polling_provider() {
        let store = RemoteDesktopSessionStore::new();
        let session_id = "rd-rebind-deadline-no-observation";
        let session = RemoteDesktopSession::new(test_session_init(
            session_id,
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        ));
        store.with_sessions(|sessions| {
            sessions.insert(session_id.to_string(), session);
        });
        let inputs = store
            .target_observation_inputs_for_session(session_id)
            .expect("target observation inputs");
        let rebind_observed_at_ms = super::now_ms().saturating_sub(31_000);
        store.commit_target_observation_for_session(
            session_id,
            &inputs.binding_id,
            inputs.binding_epoch,
            TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "target disappeared".into(),
                observed_at_ms: rebind_observed_at_ms.saturating_sub(1_200),
            },
        );
        store.commit_target_observation_for_session(
            session_id,
            &inputs.binding_id,
            inputs.binding_epoch,
            TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "target still disappeared".into(),
                observed_at_ms: rebind_observed_at_ms.saturating_sub(100),
            },
        );
        store.commit_target_observation_for_session(
            session_id,
            &inputs.binding_id,
            inputs.binding_epoch,
            TargetObservation::VisibilityChanged {
                visibility_state: TargetVisibilityState::Visible,
                target_geometry_revision: 9,
                observed_at_ms: rebind_observed_at_ms,
            },
        );

        let calls = Arc::new(AtomicUsize::new(0));
        let provider = CountingObservationProvider {
            calls: Arc::clone(&calls),
        };
        let result = observe_bound_session_target_once(&store, session_id, &provider);

        assert!(result.keep_tracking);
        assert!(result.media_source_lost.is_none());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "deadline expiry is a state-machine tick and must not depend on provider observation"
        );
        store.with_sessions(|sessions| {
            let session = sessions
                .get(session_id)
                .expect("session remains inspectable");
            let event = session
                .events()
                .into_iter()
                .find(|event| event["event_type"] == json!("TARGET_REBIND_FAILED"))
                .expect("deadline expiry event");
            assert_eq!(event["payload"]["detail"], json!("rebind_window_expired"));
            assert_eq!(event["payload"]["target_status"], json!("lost"));
            assert_eq!(event["payload"]["input_enabled"], json!(false));
        });
    }

    #[test]
    fn pending_media_rebind_deadline_expiry_stops_active_endpoint_by_epoch() {
        let store = RemoteDesktopSessionStore::new();
        let session_id = "rd-active-media-rebind-deadline";
        let epoch = TransportEpoch::new(41);
        let mut session = RemoteDesktopSession::new(test_application_session_init(
            session_id,
            vec!["webrtc".into()],
        ));
        session.begin_webrtc_negotiation(epoch);
        session
            .set_local_webrtc_answer(
                epoch,
                json!({"type": "answer", "sdp": "v=0"}),
                "sck-native",
                true,
                direct_webrtc_endpoint_ura(session_id),
            )
            .expect("local answer records");
        session.mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura(session_id));
        store.with_sessions(|sessions| {
            sessions.insert(session_id.to_string(), session);
        });

        let inputs = store
            .target_observation_inputs_for_session(session_id)
            .expect("target observation inputs");
        let rebind_observed_at_ms = super::now_ms().saturating_sub(31_000);
        assert!(store
            .commit_target_observation_for_session(
                session_id,
                &inputs.binding_id,
                inputs.binding_epoch,
                TargetObservation::ApplicationSurfaceChanged {
                    app_window_set: AppWindowSetProof::new(
                        42,
                        Some("com.example.Editor".to_string()),
                        Some(9001),
                        vec![10, 11, 12],
                    ),
                    app_surface_layout: None,
                    geometry: TargetGeometry {
                        x: Some(10.0),
                        y: Some(20.0),
                        width: Some(320.0),
                        height: Some(120.0),
                    },
                    target_identity_epoch: 100,
                    target_geometry_revision: 4,
                    observed_at_ms: rebind_observed_at_ms,
                },
            )
            .and_then(|commit| commit.media_source_lost)
            .is_none());

        let calls = Arc::new(AtomicUsize::new(0));
        let provider = CountingObservationProvider {
            calls: Arc::clone(&calls),
        };
        let result = observe_bound_session_target_once(&store, session_id, &provider);

        assert!(result.keep_tracking);
        let media_source_lost = result
            .media_source_lost
            .expect("deadline expiry returns a transport stop command");
        assert_eq!(media_source_lost.transport_epoch, epoch);
        assert_eq!(media_source_lost.reason, TargetResolutionError::TargetStale);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        store.with_sessions(|sessions| {
            let session = sessions
                .get(session_id)
                .expect("session remains inspectable");
            assert_eq!(
                session.transport_state()["primary"],
                json!("media_source_lost")
            );
            assert_eq!(session.transport_state()["device_sending"], json!(false));
            let events = session.events();
            let rebind_failed_index = events
                .iter()
                .position(|event| event["event_type"] == json!("TARGET_REBIND_FAILED"))
                .expect("deadline emits target rebind failure");
            let media_lost_index = events
                .iter()
                .position(|event| event["event_type"] == json!("MEDIA_SOURCE_LOST"))
                .expect("deadline emits media-source loss");
            assert!(rebind_failed_index < media_lost_index);
        });
    }

    #[test]
    fn sampled_host_target_observations_bound_session_fanout_to_one_enumeration_per_tick() {
        const SESSION_COUNT: usize = 128;

        let calls = Arc::new(AtomicUsize::new(0));
        let source = CountingSnapshotProvider {
            calls: Arc::clone(&calls),
        };
        let sample = sample_host_target_observations(&source);
        let binding = window_binding();
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);

        for _session_tick in 0..SESSION_COUNT {
            sample.observe(&binding, &snapshot);
        }

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "PERF-03 sampled target observer must use one host enumeration for 128 session ticks in one monitor tick"
        );
    }

    #[test]
    fn unsupported_platform_observer_fails_app_window_targets_closed() {
        let window_observation = unsupported_platform_target_observation(&window_binding())
            .expect("unsupported platform must fail window target closed");
        match window_observation {
            TargetObservation::Lost { reason, detail, .. } => {
                assert_eq!(reason, TargetResolutionError::UnsupportedCaptureScope);
                assert!(detail.contains("window scoped capture"));
            }
            other => panic!("expected unsupported window target loss, got {other:?}"),
        }

        let application_observation =
            unsupported_platform_target_observation(&application_binding())
                .expect("unsupported platform must fail application target closed");
        match application_observation {
            TargetObservation::Lost { reason, detail, .. } => {
                assert_eq!(reason, TargetResolutionError::UnsupportedCaptureScope);
                assert!(detail.contains("application scoped capture"));
            }
            other => panic!("expected unsupported application target loss, got {other:?}"),
        }

        let display_binding = test_session_init(
            "rd-display-unsupported-platform-observer",
            "easynet:///r/acme/resource/display.unsupported-platform",
            vec!["webrtc".into()],
        )
        .target_binding;
        assert!(
            unsupported_platform_target_observation(&display_binding).is_none(),
            "display target observation may remain a platform no-op because display capture is not app/window-scoped"
        );
    }

    #[test]
    fn window_observation_prioritizes_visibility_loss_over_title_or_focus_changes() {
        let binding = window_binding();
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let observation = observe_binding_against_host_snapshot(
            &binding,
            &snapshot,
            &HostTargetSnapshot {
                windows: vec![ObservedWindow {
                    window_id: 10,
                    pid: Some(9001),
                    bundle_id: Some("com.example.Editor".to_string()),
                    display_id: Some(42),
                    title: Some("New title while hidden".to_string()),
                    focused: true,
                    geometry: TargetGeometry {
                        x: Some(10.0),
                        y: Some(20.0),
                        width: Some(100.0),
                        height: Some(80.0),
                    },
                    visibility_state: TargetVisibilityState::Hidden,
                }],
                display_ids: BTreeSet::from([42]),
            },
        )
        .expect("hidden window observation");

        match observation {
            TargetObservation::VisibilityChanged {
                visibility_state, ..
            } => assert_eq!(visibility_state, TargetVisibilityState::Hidden),
            other => panic!(
                "hidden/minimized target availability must outrank title/focus observations, got {other:?}"
            ),
        }

        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);
        tracker
            .commit_observation(observation)
            .expect("hidden target observation commits");
        assert_eq!(tracker.snapshot().to_value()["status"], json!("stale"));
        assert!(
            tracker.snapshot().pointer_target_value().is_none(),
            "hidden window must disable pointer mapping before lower-priority title/focus updates"
        );
    }

    #[test]
    fn application_observation_tracks_exact_window_set_union_as_geometry() {
        let binding = application_binding();
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let observation = observe_binding_against_host_snapshot(
            &binding,
            &snapshot,
            &HostTargetSnapshot {
                windows: vec![
                    ObservedWindow {
                        window_id: 10,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(42),
                        geometry: TargetGeometry {
                            x: Some(10.0),
                            y: Some(20.0),
                            width: Some(100.0),
                            height: Some(80.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                    ObservedWindow {
                        window_id: 11,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(42),
                        geometry: TargetGeometry {
                            x: Some(140.0),
                            y: Some(60.0),
                            width: Some(70.0),
                            height: Some(40.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                ],
                display_ids: BTreeSet::from([42]),
            },
        )
        .expect("application observation");

        match observation {
            TargetObservation::ApplicationSurfaceChanged {
                geometry,
                target_geometry_revision,
                app_surface_layout: Some(_),
                ..
            } => {
                assert_eq!(target_geometry_revision, 2);
                assert_eq!(geometry.x, Some(10.0));
                assert_eq!(geometry.y, Some(20.0));
                assert_eq!(geometry.width, Some(200.0));
                assert_eq!(geometry.height, Some(80.0));
            }
            other => {
                panic!("expected exact app surface geometry to stage a media rebind, got {other:?}")
            }
        }
    }

    #[test]
    fn application_observation_rebinds_same_display_window_set_expansion() {
        let binding = application_binding();
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let observation = observe_binding_against_host_snapshot(
            &binding,
            &snapshot,
            &HostTargetSnapshot {
                windows: vec![
                    ObservedWindow {
                        window_id: 10,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(42),
                        geometry: TargetGeometry {
                            x: Some(10.0),
                            y: Some(20.0),
                            width: Some(100.0),
                            height: Some(80.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                    ObservedWindow {
                        window_id: 11,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(42),
                        geometry: TargetGeometry {
                            x: Some(130.0),
                            y: Some(60.0),
                            width: Some(70.0),
                            height: Some(40.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                    ObservedWindow {
                        window_id: 12,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(42),
                        geometry: TargetGeometry {
                            x: Some(220.0),
                            y: Some(30.0),
                            width: Some(60.0),
                            height: Some(60.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                ],
                display_ids: BTreeSet::from([42]),
            },
        )
        .expect("application window-set expansion observation");

        match observation {
            TargetObservation::ApplicationSurfaceChanged {
                app_window_set,
                geometry,
                target_identity_epoch,
                ..
            } => {
                assert_eq!(app_window_set.resolved_window_count(), 3);
                assert_eq!(target_identity_epoch, app_window_set.window_set_epoch());
                assert_eq!(geometry.width, Some(270.0));
            }
            other => panic!(
                "same-display application window-set expansion must produce rebind evidence, got {other:?}"
            ),
        }
    }

    #[test]
    fn application_observation_rebinds_same_app_window_set_subset() {
        let binding = ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &ResourceEntry {
                    resource_ura: "easynet:///r/acme/resource/application.editor.subset"
                        .to_string(),
                    owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
                    kind: ResourceType::Application,
                    binding: ResourceBinding::LocalDevice,
                    hardware_id: "application:macos:cgwindow:bundle:com.example.Editor".to_string(),
                    display_name: "Editor".to_string(),
                    metadata: live_remote_target_metadata(json!({
                        "platform": "macos",
                        "backend": "macos_core_graphics",
                        "display_ids": [42],
                        "bundle_id": "com.example.Editor",
                        "app_identity": "com.example.Editor",
                        "primary_pid": 9001,
                        "resolved_window_ids": [10, 11, 12],
                        "window_set_epoch": 123,
                        "union_x": 10,
                        "union_y": 20,
                        "union_width": 270,
                        "union_height": 80,
                    })),
                    first_seen_at: "2026-06-01T00:00:00Z".to_string(),
                },
                "view_only",
                1,
            )
            .expect("application target binding resolves");
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let observation = observe_binding_against_host_snapshot(
            &binding,
            &snapshot,
            &HostTargetSnapshot {
                windows: vec![ObservedWindow {
                    window_id: 10,
                    pid: Some(9001),
                    bundle_id: Some("com.example.Editor".to_string()),
                    display_id: Some(42),
                    geometry: TargetGeometry {
                        x: Some(10.0),
                        y: Some(20.0),
                        width: Some(100.0),
                        height: Some(80.0),
                    },
                    title: None,
                    focused: false,
                    visibility_state: TargetVisibilityState::Visible,
                }],
                display_ids: BTreeSet::from([42]),
            },
        )
        .expect("application subset observation");

        match observation {
            TargetObservation::ApplicationSurfaceChanged {
                app_window_set,
                geometry,
                target_identity_epoch,
                ..
            } => {
                assert_eq!(app_window_set.resolved_window_count(), 1);
                assert_eq!(target_identity_epoch, app_window_set.window_set_epoch());
                assert_eq!(geometry.width, Some(100.0));
            }
            other => panic!(
                "same app/display window-set drift must update the application binding, got {other:?}"
            ),
        }
    }

    #[test]
    fn application_observation_rebinds_cross_display_window_set() {
        let binding = application_binding();
        let snapshot = TargetTrackerSnapshot::from_binding(&binding);
        let observation = observe_binding_against_host_snapshot(
            &binding,
            &snapshot,
            &HostTargetSnapshot {
                windows: vec![
                    ObservedWindow {
                        window_id: 10,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(42),
                        geometry: TargetGeometry {
                            x: Some(10.0),
                            y: Some(20.0),
                            width: Some(100.0),
                            height: Some(80.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                    ObservedWindow {
                        window_id: 12,
                        pid: Some(9001),
                        bundle_id: Some("com.example.Editor".to_string()),
                        display_id: Some(99),
                        geometry: TargetGeometry {
                            x: Some(500.0),
                            y: Some(500.0),
                            width: Some(50.0),
                            height: Some(50.0),
                        },
                        title: None,
                        focused: false,
                        visibility_state: TargetVisibilityState::Visible,
                    },
                ],
                display_ids: BTreeSet::from([42, 99]),
            },
        )
        .expect("application observation");

        match observation {
            TargetObservation::ApplicationSurfaceChanged {
                app_window_set,
                geometry,
                ..
            } => {
                assert_eq!(app_window_set.resolved_window_count(), 2);
                assert_eq!(geometry.x, Some(10.0));
                assert_eq!(geometry.y, Some(20.0));
                assert_eq!(geometry.width, Some(540.0));
                assert_eq!(geometry.height, Some(530.0));
            }
            other => panic!("cross-display application drift must rebind, got {other:?}"),
        }
    }
}

#[cfg(all(not(target_os = "macos"), feature = "native-media"))]
mod platform {
    use std::collections::BTreeSet;

    use super::{
        sample_host_target_observations, HostTargetSnapshot, HostTargetSnapshotProvider,
        ObservedWindow, PlatformTargetObservationSample,
    };
    use crate::daemon::plugins::remote_desktop::target::TargetGeometry;
    use crate::daemon::plugins::remote_desktop::target_tracking::TargetVisibilityState;

    struct XcapHostTargetSnapshotProvider;

    pub(super) fn sample_platform_target_observations() -> PlatformTargetObservationSample {
        sample_host_target_observations(&XcapHostTargetSnapshotProvider)
    }

    impl HostTargetSnapshotProvider for XcapHostTargetSnapshotProvider {
        fn snapshot(&self) -> anyhow::Result<HostTargetSnapshot> {
            let windows = xcap::Window::all()
                .map_err(|error| anyhow::anyhow!("xcap Window::all failed: {error}"))?
                .into_iter()
                .filter_map(|window| {
                    let window_id = u64::from(window.id().ok()?);
                    let width = window.width().ok()?;
                    let height = window.height().ok()?;
                    if width == 0 || height == 0 {
                        return None;
                    }
                    let minimized = window.is_minimized().ok() == Some(true);
                    Some(ObservedWindow {
                        window_id,
                        pid: window.pid().ok().map(i64::from),
                        // xcap does not expose a cross-platform bundle id. The
                        // process id remains the load-bearing identity; app name
                        // is an additional owner discriminator when present.
                        bundle_id: window
                            .app_name()
                            .ok()
                            .filter(|name| !name.trim().is_empty()),
                        display_id: None,
                        title: window.title().ok().filter(|title| !title.trim().is_empty()),
                        focused: window.is_focused().ok() == Some(true),
                        geometry: TargetGeometry {
                            x: window.x().ok().map(f64::from),
                            y: window.y().ok().map(f64::from),
                            width: Some(f64::from(width)),
                            height: Some(f64::from(height)),
                        },
                        visibility_state: if minimized {
                            TargetVisibilityState::Minimized
                        } else {
                            TargetVisibilityState::Visible
                        },
                    })
                })
                .collect();
            let display_ids = xcap::Monitor::all()
                .map_err(|error| anyhow::anyhow!("xcap Monitor::all failed: {error}"))?
                .into_iter()
                .filter_map(|monitor| monitor.id().ok().map(u64::from))
                .collect::<BTreeSet<_>>();
            Ok(HostTargetSnapshot {
                windows,
                display_ids,
            })
        }
    }
}

#[cfg(all(not(target_os = "macos"), not(feature = "native-media")))]
mod platform {
    use super::PlatformTargetObservationSample;

    pub(super) fn sample_platform_target_observations() -> PlatformTargetObservationSample {
        PlatformTargetObservationSample::unsupported_platform()
    }
}
