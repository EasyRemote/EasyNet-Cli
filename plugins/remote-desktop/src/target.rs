// EasyNet CLI — remote desktop target binding domain
// ==================================================
//
// File: plugins/remote-desktop/src/target.rs
// Description: Typed target binding model for display/window/application
// remote desktop sessions.
//
// Architectural position:
// - ResourceEntry is the resource-inventory DTO and Invocation subject.
// - RemoteAppTargetBinding is the session-owned execution boundary consumed by
//   media, input, lifecycle tracking, and audit projection.
// - Platform/native lookup belongs behind ResourceEntryTargetResolver or
//   explicit Rebinding, never in transport handlers.

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use axon_sdk::invocation::{AxonError, AxonErrorKind, ErrorCode, ErrorStage, SecurityClass};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::ura::{parse_ura, URAKind};
use crate::daemon::persistence::resources::{
    application_surface_layout_epoch, application_window_set_epoch_with_process_instance,
    ResourceBinding, ResourceEntry, ResourceType,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum RemoteDesktopTargetKind {
    Display,
    Window,
    Application,
}

impl RemoteDesktopTargetKind {
    pub(in crate::daemon::plugins::remote_desktop) fn as_str(self) -> &'static str {
        match self {
            Self::Display => "display",
            Self::Window => "window",
            Self::Application => "application",
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn resource_type(self) -> ResourceType {
        match self {
            Self::Display => ResourceType::Display,
            Self::Window => ResourceType::Window,
            Self::Application => ResourceType::Application,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_model(self) -> &'static str {
        match self {
            Self::Display => "display_surface",
            Self::Window => "window_surface",
            Self::Application => "process_scoped_application_window_set",
        }
    }

    fn target_model_for_platform(self, platform: &str) -> &'static str {
        match (self, platform) {
            (Self::Application, "macos") => "multi_surface_application_window_set",
            (Self::Application, _) => "process_scoped_application_window_set",
            _ => self.target_model(),
        }
    }

    fn from_recovery_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "display" => Ok(Self::Display),
            "window" => Ok(Self::Window),
            "application" => Ok(Self::Application),
            other => anyhow::bail!("unsupported RemoteApp recovery target_kind {other:?}"),
        }
    }
}

impl TryFrom<ResourceType> for RemoteDesktopTargetKind {
    type Error = RemoteAppTargetError;

    fn try_from(kind: ResourceType) -> Result<Self, Self::Error> {
        match kind {
            ResourceType::Display => Ok(Self::Display),
            ResourceType::Window => Ok(Self::Window),
            ResourceType::Application => Ok(Self::Application),
            _ => Err(RemoteAppTargetError::new(
                "",
                TargetResolutionError::UnsupportedCaptureScope,
                format!(
                    "remote desktop target must be display/window/application, got {}",
                    kind.as_str()
                ),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // Canonical SPEC vocabulary uses the `*Surface` suffix.
pub(in crate::daemon::plugins::remote_desktop) enum CaptureScope {
    DisplaySurface,
    WindowSurface,
    AppSurface,
}

impl CaptureScope {
    pub(in crate::daemon::plugins::remote_desktop) fn as_str(self) -> &'static str {
        match self {
            Self::DisplaySurface => "DisplaySurface",
            Self::WindowSurface => "WindowSurface",
            Self::AppSurface => "AppSurface",
        }
    }

    fn from_recovery_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "DisplaySurface" => Ok(Self::DisplaySurface),
            "WindowSurface" => Ok(Self::WindowSurface),
            "AppSurface" => Ok(Self::AppSurface),
            other => anyhow::bail!("unsupported RemoteApp recovery capture_scope {other:?}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum InputScope {
    ViewOnly,
    TargetLocal,
    DisplayGlobal,
}

impl InputScope {
    pub(in crate::daemon::plugins::remote_desktop) fn as_str(self) -> &'static str {
        debug_assert!(ALL_INPUT_SCOPES.contains(&self));
        match self {
            Self::ViewOnly => "view_only",
            Self::TargetLocal => "target_local",
            Self::DisplayGlobal => "display_global",
        }
    }

    fn from_recovery_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "view_only" => Ok(Self::ViewOnly),
            "target_local" => Ok(Self::TargetLocal),
            "display_global" => Ok(Self::DisplayGlobal),
            other => anyhow::bail!("unsupported RemoteApp recovery input_scope {other:?}"),
        }
    }
}

const ALL_INPUT_SCOPES: &[InputScope] = &[
    InputScope::ViewOnly,
    InputScope::TargetLocal,
    InputScope::DisplayGlobal,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputScopeReason {
    RequestedViewOnly,
    InputControlGranted,
    InputConsentRequired,
    TargetScopedInputGuarded,
    TargetScopedInputUnsafe,
}

impl InputScopeReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::RequestedViewOnly => "requested_view_only",
            Self::InputControlGranted => "input_control_granted",
            Self::InputConsentRequired => "input_consent_required",
            Self::TargetScopedInputGuarded => "target_scoped_input_guarded",
            Self::TargetScopedInputUnsafe => "target_scoped_keyboard_pointer_dispatch_unsafe",
        }
    }

    fn from_recovery_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "requested_view_only" => Ok(Self::RequestedViewOnly),
            "input_control_granted" => Ok(Self::InputControlGranted),
            "input_consent_required" => Ok(Self::InputConsentRequired),
            "target_scoped_input_guarded" => Ok(Self::TargetScopedInputGuarded),
            "target_scoped_keyboard_pointer_dispatch_unsafe" => Ok(Self::TargetScopedInputUnsafe),
            other => anyhow::bail!("unsupported RemoteApp recovery input_scope_reason {other:?}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InputScopeDecision {
    scope: InputScope,
    reason: InputScopeReason,
}

impl InputScopeDecision {
    const fn new(scope: InputScope, reason: InputScopeReason) -> Self {
        Self { scope, reason }
    }

    const fn scope(self) -> InputScope {
        self.scope
    }

    const fn reason(self) -> InputScopeReason {
        self.reason
    }
}

/// Compile-time proof that this daemon contains the complete host-side guard
/// needed before target-local input can be admitted. Runtime permission and
/// environment readiness remain owned by the platform input adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetScopedInputIsolation {
    MacosAccessibilityCoreGraphics,
    WindowsXcapUser32,
    /// XTest injects into the desktop-global input stream. Even with a final
    /// XGrabServer target check it cannot bind a press/release lifecycle to one
    /// window, so Window/Application sessions must remain view-only.
    LinuxX11Unisolated,
    Unsupported,
}

impl TargetScopedInputIsolation {
    const CURRENT: Self = if cfg!(target_os = "macos") {
        Self::MacosAccessibilityCoreGraphics
    } else if cfg!(all(target_os = "windows", feature = "native-media")) {
        Self::WindowsXcapUser32
    } else if cfg!(all(target_os = "linux", feature = "native-media")) {
        Self::LinuxX11Unisolated
    } else {
        Self::Unsupported
    };

    const fn is_safe(self) -> bool {
        matches!(
            self,
            Self::MacosAccessibilityCoreGraphics | Self::WindowsXcapUser32
        )
    }
}

pub(in crate::daemon::plugins::remote_desktop) const fn target_scoped_input_guard_available() -> bool
{
    TargetScopedInputIsolation::CURRENT.is_safe()
}

pub(in crate::daemon::plugins::remote_desktop) const fn target_scoped_input_guard_unavailable_reason(
) -> &'static str {
    InputScopeReason::TargetScopedInputUnsafe.as_str()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum FrontendAction {
    RefreshTargets,
    RequestPermission,
    FocusTargetLocally,
    RetrySession,
    DowngradeViewOnly,
    ShowUnsupported,
    CloseSession,
}

impl FrontendAction {
    pub(in crate::daemon::plugins::remote_desktop) fn as_str(self) -> &'static str {
        debug_assert!(ALL_FRONTEND_ACTIONS.contains(&self));
        match self {
            Self::RefreshTargets => "refresh_targets",
            Self::RequestPermission => "request_permission",
            Self::FocusTargetLocally => "focus_target_locally",
            Self::RetrySession => "retry_session",
            Self::DowngradeViewOnly => "downgrade_view_only",
            Self::ShowUnsupported => "show_unsupported",
            Self::CloseSession => "close_session",
        }
    }
}

const ALL_FRONTEND_ACTIONS: &[FrontendAction] = &[
    FrontendAction::RefreshTargets,
    FrontendAction::RequestPermission,
    FrontendAction::FocusTargetLocally,
    FrontendAction::RetrySession,
    FrontendAction::DowngradeViewOnly,
    FrontendAction::ShowUnsupported,
    FrontendAction::CloseSession,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum TargetResolutionError {
    TargetNotFound,
    TargetStale,
    TargetMetadataIncomplete,
    TargetIdentityAmbiguous,
    TargetIdentityChanged,
    TargetIdentityMismatch,
    TargetPermissionMissing,
    UnsupportedCaptureScope,
    CaptureBackendUnavailable,
    TargetHidden,
    TargetMinimized,
    TargetDisplayUnavailable,
    TargetMultiDisplayUnsupported,
    DisplayIdentityMissing,
    DisplayIdentityMismatch,
    DisplayFallbackForbidden,
    InputScopeUnsupported,
    TransportRouteUnavailable,
    ScreenCaptureKitEnumerationFailed,
    ScreenCaptureKitFilterFailed,
    ScreenCaptureKitStreamStartFailed,
}

impl TargetResolutionError {
    pub(in crate::daemon::plugins::remote_desktop) fn as_str(self) -> &'static str {
        debug_assert!(ALL_TARGET_RESOLUTION_ERRORS.contains(&self));
        match self {
            Self::TargetNotFound => "target_not_found",
            Self::TargetStale => "target_stale",
            Self::TargetMetadataIncomplete => "target_metadata_incomplete",
            Self::TargetIdentityAmbiguous => "target_identity_ambiguous",
            Self::TargetIdentityChanged => "target_identity_changed",
            Self::TargetIdentityMismatch => "target_identity_mismatch",
            Self::TargetPermissionMissing => "target_permission_missing",
            Self::UnsupportedCaptureScope => "unsupported_capture_scope",
            Self::CaptureBackendUnavailable => "capture_backend_unavailable",
            Self::TargetHidden => "target_hidden",
            Self::TargetMinimized => "target_minimized",
            Self::TargetDisplayUnavailable => "target_display_unavailable",
            Self::TargetMultiDisplayUnsupported => "target_multi_display_unsupported",
            Self::DisplayIdentityMissing => "display_identity_missing",
            Self::DisplayIdentityMismatch => "display_identity_mismatch",
            Self::DisplayFallbackForbidden => "display_fallback_forbidden",
            Self::InputScopeUnsupported => "input_scope_unsupported",
            Self::TransportRouteUnavailable => "transport_route_unavailable",
            Self::ScreenCaptureKitEnumerationFailed => "screencapturekit_enumeration_failed",
            Self::ScreenCaptureKitFilterFailed => "screencapturekit_filter_failed",
            Self::ScreenCaptureKitStreamStartFailed => "screencapturekit_stream_start_failed",
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn frontend_action(self) -> FrontendAction {
        match self {
            Self::TargetNotFound
            | Self::TargetStale
            | Self::TargetIdentityAmbiguous
            | Self::TargetIdentityChanged
            | Self::TargetIdentityMismatch
            | Self::DisplayIdentityMismatch => FrontendAction::RefreshTargets,
            Self::TargetPermissionMissing => FrontendAction::RequestPermission,
            Self::TargetMetadataIncomplete
            | Self::UnsupportedCaptureScope
            | Self::CaptureBackendUnavailable
            | Self::TargetDisplayUnavailable
            | Self::TargetMultiDisplayUnsupported
            | Self::DisplayIdentityMissing
            | Self::DisplayFallbackForbidden
            | Self::TransportRouteUnavailable
            | Self::ScreenCaptureKitEnumerationFailed
            | Self::ScreenCaptureKitFilterFailed
            | Self::ScreenCaptureKitStreamStartFailed => FrontendAction::ShowUnsupported,
            Self::InputScopeUnsupported => FrontendAction::DowngradeViewOnly,
            Self::TargetHidden | Self::TargetMinimized => FrontendAction::RetrySession,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_event_type(
        self,
    ) -> Option<&'static str> {
        match self {
            Self::TargetNotFound => Some("TARGET_LOST"),
            Self::TargetStale => Some("CAPTURE_TARGET_STALE"),
            Self::TargetIdentityAmbiguous => Some("CAPTURE_TARGET_AMBIGUOUS"),
            Self::TargetIdentityChanged | Self::TargetIdentityMismatch => {
                Some("CAPTURE_TARGET_IDENTITY_MISMATCH")
            }
            Self::TargetPermissionMissing => Some("SCREEN_CAPTURE_PERMISSION_DENIED"),
            Self::TargetHidden => Some("TARGET_HIDDEN"),
            Self::TargetMinimized => Some("TARGET_MINIMIZED"),
            Self::TargetDisplayUnavailable => Some("DISPLAY_TOPOLOGY_CHANGED"),
            Self::DisplayFallbackForbidden => Some("DISPLAY_FALLBACK_FORBIDDEN"),
            Self::UnsupportedCaptureScope
            | Self::CaptureBackendUnavailable
            | Self::TargetMetadataIncomplete
            | Self::TargetMultiDisplayUnsupported
            | Self::DisplayIdentityMissing
            | Self::DisplayIdentityMismatch
            | Self::InputScopeUnsupported
            | Self::TransportRouteUnavailable
            | Self::ScreenCaptureKitEnumerationFailed
            | Self::ScreenCaptureKitFilterFailed
            | Self::ScreenCaptureKitStreamStartFailed => None,
        }
    }

    fn axon_projection(self) -> (AxonErrorKind, ErrorCode, ErrorStage, SecurityClass) {
        match self {
            Self::TargetPermissionMissing => (
                AxonErrorKind::PermissionDenied,
                ErrorCode::AuthorityRequired,
                ErrorStage::AuthorityValidation,
                SecurityClass::Authority,
            ),
            Self::TargetNotFound
            | Self::TargetStale
            | Self::TargetHidden
            | Self::TargetMinimized
            | Self::TargetDisplayUnavailable => (
                AxonErrorKind::InvalidArgument,
                ErrorCode::NotFound,
                ErrorStage::Execution,
                SecurityClass::Resource,
            ),
            Self::TargetMetadataIncomplete
            | Self::TargetIdentityAmbiguous
            | Self::TargetIdentityChanged
            | Self::TargetIdentityMismatch
            | Self::UnsupportedCaptureScope
            | Self::TargetMultiDisplayUnsupported
            | Self::DisplayIdentityMissing
            | Self::DisplayIdentityMismatch
            | Self::DisplayFallbackForbidden
            | Self::InputScopeUnsupported => (
                AxonErrorKind::InvalidArgument,
                ErrorCode::RequestMetadataInvalid,
                ErrorStage::RequestValidation,
                SecurityClass::Resource,
            ),
            Self::CaptureBackendUnavailable
            | Self::TransportRouteUnavailable
            | Self::ScreenCaptureKitEnumerationFailed
            | Self::ScreenCaptureKitFilterFailed
            | Self::ScreenCaptureKitStreamStartFailed => (
                AxonErrorKind::Unavailable,
                ErrorCode::ExecutionFailed,
                ErrorStage::Execution,
                SecurityClass::Resource,
            ),
        }
    }
}

const ALL_TARGET_RESOLUTION_ERRORS: &[TargetResolutionError] = &[
    TargetResolutionError::TargetNotFound,
    TargetResolutionError::TargetStale,
    TargetResolutionError::TargetMetadataIncomplete,
    TargetResolutionError::TargetIdentityAmbiguous,
    TargetResolutionError::TargetIdentityChanged,
    TargetResolutionError::TargetIdentityMismatch,
    TargetResolutionError::TargetPermissionMissing,
    TargetResolutionError::UnsupportedCaptureScope,
    TargetResolutionError::CaptureBackendUnavailable,
    TargetResolutionError::TargetHidden,
    TargetResolutionError::TargetMinimized,
    TargetResolutionError::TargetDisplayUnavailable,
    TargetResolutionError::TargetMultiDisplayUnsupported,
    TargetResolutionError::DisplayIdentityMissing,
    TargetResolutionError::DisplayIdentityMismatch,
    TargetResolutionError::DisplayFallbackForbidden,
    TargetResolutionError::InputScopeUnsupported,
    TargetResolutionError::TransportRouteUnavailable,
    TargetResolutionError::ScreenCaptureKitEnumerationFailed,
    TargetResolutionError::ScreenCaptureKitFilterFailed,
    TargetResolutionError::ScreenCaptureKitStreamStartFailed,
];

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteAppTargetError {
    ability: &'static str,
    reason: TargetResolutionError,
    detail: String,
}

impl RemoteAppTargetError {
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        ability: &'static str,
        reason: TargetResolutionError,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            ability,
            reason,
            detail: detail.into(),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn reason(&self) -> TargetResolutionError {
        self.reason
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_axon(&self) -> AxonError {
        let (kind, code, stage, security_class) = self.reason.axon_projection();
        let mut error = AxonError::new(kind)
            .with_code(code)
            .with_reason(self.reason.as_str())
            .with_stage(stage)
            .with_security_class(security_class)
            .with_context("target_reason", self.reason.as_str())
            .with_context("frontend_action", self.reason.frontend_action().as_str())
            .with_message(self.to_string());
        if let Some(target_event_type) = self.reason.target_event_type() {
            error = error.with_context("target_event_type", target_event_type);
        }
        if !self.ability.is_empty() {
            error = error.with_context("ability", self.ability);
        }
        error
    }
}

impl fmt::Display for RemoteAppTargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ability.is_empty() {
            write!(f, "{}; reason={}", self.detail, self.reason.as_str())
        } else {
            write!(
                f,
                "{}: {}; reason={}; frontend_action={}",
                self.ability,
                self.detail,
                self.reason.as_str(),
                self.reason.frontend_action().as_str()
            )
        }
    }
}

impl std::error::Error for RemoteAppTargetError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetGeometry {
    pub(in crate::daemon::plugins::remote_desktop) x: Option<f64>,
    pub(in crate::daemon::plugins::remote_desktop) y: Option<f64>,
    pub(in crate::daemon::plugins::remote_desktop) width: Option<f64>,
    pub(in crate::daemon::plugins::remote_desktop) height: Option<f64>,
}

impl TargetGeometry {
    fn from_metadata(entry: &ResourceEntry, prefix: Option<&str>) -> Self {
        let key = |name: &str| -> String {
            prefix
                .map(|prefix| format!("{prefix}_{name}"))
                .unwrap_or_else(|| name.to_string())
        };
        Self {
            x: metadata_f64(entry, &key("x")),
            y: metadata_f64(entry, &key("y")),
            width: metadata_f64(entry, &key("width")),
            height: metadata_f64(entry, &key("height")),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "x": self.x,
            "y": self.y,
            "width": self.width,
            "height": self.height,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn from_recovery_value(
        value: &Value,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            x: optional_f64(value, "x")?,
            y: optional_f64(value, "y")?,
            width: optional_f64(value, "width")?,
            height: optional_f64(value, "height")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetIdentity {
    pub(in crate::daemon::plugins::remote_desktop) hardware_id: String,
    pub(in crate::daemon::plugins::remote_desktop) display_id: Option<u64>,
    pub(in crate::daemon::plugins::remote_desktop) window_id: Option<u64>,
    pub(in crate::daemon::plugins::remote_desktop) pid: Option<i64>,
    pub(in crate::daemon::plugins::remote_desktop) process_instance_id: Option<String>,
    pub(in crate::daemon::plugins::remote_desktop) app_identity: Option<String>,
    pub(in crate::daemon::plugins::remote_desktop) bundle_id: Option<String>,
    pub(in crate::daemon::plugins::remote_desktop) app_name: Option<String>,
    pub(in crate::daemon::plugins::remote_desktop) title: Option<String>,
}

impl TargetIdentity {
    fn from_entry(entry: &ResourceEntry, display_id: Option<u64>) -> Self {
        Self {
            hardware_id: entry.hardware_id.clone(),
            display_id,
            window_id: metadata_u64(entry, "window_id"),
            pid: metadata_i64(entry, "pid").or_else(|| metadata_i64(entry, "primary_pid")),
            process_instance_id: metadata_string(entry, "process_instance_id"),
            app_identity: metadata_string(entry, "app_identity"),
            bundle_id: metadata_string(entry, "bundle_id"),
            app_name: metadata_string(entry, "app_name"),
            title: metadata_string(entry, "title"),
        }
    }

    fn to_value(&self) -> Value {
        json!({
            "hardware_id": self.hardware_id,
            "display_id": self.display_id,
            "window_id": self.window_id,
            "pid": self.pid,
            "process_instance_id": self.process_instance_id,
            "app_identity": self.app_identity,
            "bundle_id": self.bundle_id,
            "app_name": self.app_name,
            "title": self.title,
        })
    }

    fn from_recovery_value(value: &Value) -> anyhow::Result<Self> {
        Ok(Self {
            hardware_id: required_owned_string(value, "hardware_id")?,
            display_id: optional_u64(value, "display_id")?,
            window_id: optional_u64(value, "window_id")?,
            pid: optional_i64(value, "pid")?,
            process_instance_id: optional_string(value, "process_instance_id")?,
            app_identity: optional_string(value, "app_identity")?,
            bundle_id: optional_string(value, "bundle_id")?,
            app_name: optional_string(value, "app_name")?,
            title: optional_string(value, "title")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct ResolvedCaptureTargetProof {
    backend: String,
    target_kind: RemoteDesktopTargetKind,
    display_id: Option<u64>,
    window_id: Option<u64>,
    pid: Option<i64>,
    process_instance_id: Option<String>,
    app_identity: Option<String>,
    bundle_id: Option<String>,
    app_window_set: Option<AppWindowSetProof>,
    app_surface_layout: Option<AppSurfaceLayoutProof>,
    native_width: Option<usize>,
    native_height: Option<usize>,
    verified_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct AppWindowSetProof {
    display_id: Option<u64>,
    display_ids: Vec<u64>,
    bundle_id: Option<String>,
    primary_pid: Option<i64>,
    process_instance_id: Option<String>,
    resolved_window_ids: Vec<u64>,
    window_set_epoch: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AppWindowSurfaceProof {
    window_id: u64,
    x: i64,
    y: i64,
    width: u64,
    height: u64,
}

/// Committed composition layout for one application surface.
///
/// The vector is front-to-back and therefore captures both per-window native
/// geometry and z-order. It is deliberately distinct from `AppWindowSetProof`:
/// moving/reordering an existing window rebuilds media without pretending that
/// the application's process/window identity changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct AppSurfaceLayoutProof {
    front_to_back_surfaces: Vec<AppWindowSurfaceProof>,
    layout_epoch: u64,
}

impl AppSurfaceLayoutProof {
    fn from_entry(entry: &ResourceEntry) -> Option<Self> {
        let surfaces = entry
            .metadata
            .get("front_to_back_surfaces")?
            .as_array()?
            .iter()
            .map(|surface| {
                Some(AppWindowSurfaceProof {
                    window_id: surface.get("window_id")?.as_u64()?,
                    x: surface.get("x")?.as_i64()?,
                    y: surface.get("y")?.as_i64()?,
                    width: surface.get("width")?.as_u64()?,
                    height: surface.get("height")?.as_u64()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let proof = Self::from_canonical_surfaces(surfaces)?;
        (metadata_u64(entry, "surface_layout_epoch") == Some(proof.layout_epoch)).then_some(proof)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn from_front_to_back_geometries<'a>(
        surfaces: impl IntoIterator<Item = (u64, &'a TargetGeometry)>,
    ) -> Option<Self> {
        let front_to_back_surfaces = surfaces
            .into_iter()
            .map(|(window_id, geometry)| {
                Some(AppWindowSurfaceProof {
                    window_id,
                    x: canonical_surface_origin(geometry.x?)?,
                    y: canonical_surface_origin(geometry.y?)?,
                    width: canonical_surface_dimension(geometry.width?)?,
                    height: canonical_surface_dimension(geometry.height?)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Self::from_canonical_surfaces(front_to_back_surfaces)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn layout_epoch(&self) -> u64 {
        self.layout_epoch
    }

    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    pub(in crate::daemon::plugins::remote_desktop) fn front_to_back_surfaces(
        &self,
    ) -> impl Iterator<Item = (u64, i64, i64, u64, u64)> + '_ {
        self.front_to_back_surfaces.iter().map(|surface| {
            (
                surface.window_id,
                surface.x,
                surface.y,
                surface.width,
                surface.height,
            )
        })
    }

    fn union_geometry(&self) -> Option<TargetGeometry> {
        let first = self.front_to_back_surfaces.first()?;
        let mut min_x = i128::from(first.x);
        let mut min_y = i128::from(first.y);
        let mut max_x = min_x.checked_add(i128::from(first.width))?;
        let mut max_y = min_y.checked_add(i128::from(first.height))?;
        for surface in &self.front_to_back_surfaces[1..] {
            let x = i128::from(surface.x);
            let y = i128::from(surface.y);
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x.checked_add(i128::from(surface.width))?);
            max_y = max_y.max(y.checked_add(i128::from(surface.height))?);
        }
        Some(TargetGeometry {
            x: Some(min_x as f64),
            y: Some(min_y as f64),
            width: Some(max_x.checked_sub(min_x)? as f64),
            height: Some(max_y.checked_sub(min_y)? as f64),
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "front_to_back_surfaces": self.front_to_back_surfaces.iter().map(|surface| json!({
                "window_id": surface.window_id,
                "x": surface.x,
                "y": surface.y,
                "width": surface.width,
                "height": surface.height,
            })).collect::<Vec<_>>(),
            "layout_epoch": self.layout_epoch,
        })
    }

    fn from_recovery_value(value: &Value) -> anyhow::Result<Self> {
        let surfaces = value
            .get("front_to_back_surfaces")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "RemoteApp recovery app_surface_layout requires front_to_back_surfaces"
                )
            })?
            .iter()
            .map(|surface| {
                Ok(AppWindowSurfaceProof {
                    window_id: required_u64(surface, "window_id")?,
                    x: required_i64(surface, "x")?,
                    y: required_i64(surface, "y")?,
                    width: required_u64(surface, "width")?,
                    height: required_u64(surface, "height")?,
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let proof = Self::from_canonical_surfaces(surfaces).ok_or_else(|| {
            anyhow::anyhow!("RemoteApp recovery app_surface_layout is empty or invalid")
        })?;
        let recovered_epoch = required_u64(value, "layout_epoch")?;
        anyhow::ensure!(
            proof.layout_epoch == recovered_epoch,
            "RemoteApp recovery app_surface_layout epoch does not match canonical surfaces"
        );
        Ok(proof)
    }

    fn from_canonical_surfaces(surfaces: Vec<AppWindowSurfaceProof>) -> Option<Self> {
        if surfaces.is_empty()
            || surfaces
                .iter()
                .any(|surface| surface.window_id == 0 || surface.width == 0 || surface.height == 0)
        {
            return None;
        }
        let mut ids = surfaces
            .iter()
            .map(|surface| surface.window_id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        if ids.len() != surfaces.len() {
            return None;
        }
        let canonical = surfaces
            .iter()
            .map(|surface| {
                (
                    surface.window_id,
                    surface.x,
                    surface.y,
                    surface.width,
                    surface.height,
                )
            })
            .collect::<Vec<_>>();
        Some(Self {
            layout_epoch: application_surface_layout_epoch(&canonical),
            front_to_back_surfaces: surfaces,
        })
    }
}

fn canonical_surface_origin(value: f64) -> Option<i64> {
    (value.is_finite() && value >= i64::MIN as f64 && value <= i64::MAX as f64)
        .then(|| value.round() as i64)
}

fn canonical_surface_dimension(value: f64) -> Option<u64> {
    (value.is_finite() && value > 0.0 && value <= u64::MAX as f64)
        .then(|| value.round() as u64)
        .filter(|value| *value > 0)
}

impl AppWindowSetProof {
    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        display_id: u64,
        bundle_id: Option<String>,
        primary_pid: Option<i64>,
        resolved_window_ids: Vec<u64>,
    ) -> Self {
        Self::new_platform_scoped(
            Some(display_id),
            vec![display_id],
            bundle_id,
            primary_pid,
            resolved_window_ids,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn new_platform_scoped(
        display_id: Option<u64>,
        display_ids: Vec<u64>,
        bundle_id: Option<String>,
        primary_pid: Option<i64>,
        resolved_window_ids: Vec<u64>,
    ) -> Self {
        let mut resolved_window_ids = resolved_window_ids;
        resolved_window_ids.sort_unstable();
        resolved_window_ids.dedup();
        let mut display_ids = display_ids;
        if let Some(display_id) = display_id {
            display_ids.push(display_id);
        }
        display_ids.retain(|display_id| *display_id > 0);
        display_ids.sort_unstable();
        display_ids.dedup();
        let window_set_epoch = compute_window_set_epoch(
            display_id,
            bundle_id.as_deref(),
            primary_pid,
            None,
            &resolved_window_ids,
        );
        Self {
            display_id,
            display_ids,
            bundle_id,
            primary_pid,
            process_instance_id: None,
            resolved_window_ids,
            window_set_epoch,
        }
    }

    fn from_entry(entry: &ResourceEntry, display_id: Option<u64>) -> Option<Self> {
        let mut resolved_window_ids = metadata_u64_array(entry, "resolved_window_ids");
        resolved_window_ids.sort_unstable();
        resolved_window_ids.dedup();
        if resolved_window_ids.is_empty() {
            return None;
        }
        let bundle_id = metadata_string(entry, "bundle_id");
        let primary_pid = metadata_i64(entry, "primary_pid").or_else(|| metadata_i64(entry, "pid"));
        let process_instance_id = metadata_string(entry, "process_instance_id");
        let mut display_ids = metadata_u64_array(entry, "display_ids");
        if let Some(display_id) = display_id {
            display_ids.push(display_id);
        }
        display_ids.retain(|display_id| *display_id > 0);
        display_ids.sort_unstable();
        display_ids.dedup();
        let window_set_epoch = metadata_u64(entry, "window_set_epoch").unwrap_or_else(|| {
            compute_window_set_epoch(
                display_id,
                bundle_id.as_deref(),
                primary_pid,
                process_instance_id.as_deref(),
                &resolved_window_ids,
            )
        });
        Some(Self {
            display_id,
            display_ids,
            bundle_id,
            primary_pid,
            process_instance_id,
            resolved_window_ids,
            window_set_epoch,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn window_set_epoch(&self) -> u64 {
        self.window_set_epoch
    }

    pub(in crate::daemon::plugins::remote_desktop) fn contains_window_id(
        &self,
        window_id: u64,
    ) -> bool {
        self.resolved_window_ids.binary_search(&window_id).is_ok()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn resolved_window_ids(&self) -> &[u64] {
        &self.resolved_window_ids
    }

    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    pub(in crate::daemon::plugins::remote_desktop) fn display_id(&self) -> Option<u64> {
        self.display_id
    }

    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    pub(in crate::daemon::plugins::remote_desktop) fn display_ids(&self) -> &[u64] {
        &self.display_ids
    }

    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    pub(in crate::daemon::plugins::remote_desktop) fn primary_pid(&self) -> Option<i64> {
        self.primary_pid
    }

    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    pub(in crate::daemon::plugins::remote_desktop) fn process_instance_id(&self) -> Option<&str> {
        self.process_instance_id.as_deref()
    }

    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    pub(in crate::daemon::plugins::remote_desktop) fn bundle_id(&self) -> Option<&str> {
        self.bundle_id.as_deref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn with_process_instance_id(
        mut self,
        process_instance_id: Option<String>,
    ) -> Self {
        self.process_instance_id = process_instance_id;
        self.window_set_epoch = compute_window_set_epoch(
            self.display_id,
            self.bundle_id.as_deref(),
            self.primary_pid,
            self.process_instance_id.as_deref(),
            &self.resolved_window_ids,
        );
        self
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "display_id": self.display_id,
            "display_ids": self.display_ids,
            "bundle_id": self.bundle_id,
            "primary_pid": self.primary_pid,
            "process_instance_id": self.process_instance_id,
            "resolved_window_ids": self.resolved_window_ids,
            "window_set_epoch": self.window_set_epoch,
        })
    }

    fn from_recovery_value(value: &Value) -> anyhow::Result<Self> {
        let display_id = optional_u64(value, "display_id")?;
        if let Some(display_id) = display_id {
            anyhow::ensure!(
                display_id > 0,
                "RemoteApp recovery app_window_set display_id must be positive"
            );
        }
        let display_ids = value
            .get("display_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                anyhow::anyhow!("RemoteApp recovery app_window_set requires display_ids")
            })?
            .iter()
            .map(|item| {
                item.as_u64()
                    .filter(|display_id| *display_id > 0)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "RemoteApp recovery app_window_set display_ids must be positive integers"
                        )
                    })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut canonical_display_ids = display_ids.clone();
        canonical_display_ids.sort_unstable();
        canonical_display_ids.dedup();
        anyhow::ensure!(
            display_ids == canonical_display_ids,
            "RemoteApp recovery app_window_set display_ids must be sorted and unique"
        );
        if let Some(display_id) = display_id {
            anyhow::ensure!(
                display_ids.binary_search(&display_id).is_ok(),
                "RemoteApp recovery app_window_set display_ids must contain display_id"
            );
        }
        let resolved_window_ids = value
            .get("resolved_window_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                anyhow::anyhow!("RemoteApp recovery app_window_set requires resolved_window_ids")
            })?
            .iter()
            .map(|item| {
                item.as_u64().ok_or_else(|| {
                    anyhow::anyhow!(
                        "RemoteApp recovery app_window_set resolved_window_ids must be integers"
                    )
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self {
            display_id,
            display_ids,
            bundle_id: optional_string(value, "bundle_id")?,
            primary_pid: optional_i64(value, "primary_pid")?,
            process_instance_id: optional_string(value, "process_instance_id")?,
            resolved_window_ids,
            window_set_epoch: required_u64(value, "window_set_epoch")?,
        })
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn resolved_window_count(&self) -> usize {
        self.resolved_window_ids.len()
    }

    fn diagnostic_label(&self) -> String {
        format!(
            "display_id={:?}, display_ids={:?}, bundle_id={:?}, primary_pid={:?}, process_instance_id={:?}, resolved_window_ids={:?}, window_set_epoch={}",
            self.display_id,
            self.display_ids,
            self.bundle_id,
            self.primary_pid,
            self.process_instance_id,
            self.resolved_window_ids,
            self.window_set_epoch
        )
    }
}

impl ResolvedCaptureTargetProof {
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        backend: impl Into<String>,
        target_kind: RemoteDesktopTargetKind,
    ) -> Self {
        Self {
            backend: backend.into(),
            target_kind,
            display_id: None,
            window_id: None,
            pid: None,
            process_instance_id: None,
            app_identity: None,
            bundle_id: None,
            app_window_set: None,
            app_surface_layout: None,
            native_width: None,
            native_height: None,
            verified_at_ms: unix_epoch_ms(),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn with_native_identity(
        mut self,
        display_id: Option<u64>,
        window_id: Option<u64>,
        pid: Option<i64>,
        app_identity: Option<String>,
        bundle_id: Option<String>,
    ) -> Self {
        self.display_id = display_id;
        self.window_id = window_id;
        self.pid = pid;
        self.app_identity = app_identity;
        self.bundle_id = bundle_id;
        self
    }

    #[cfg(any(test, feature = "native-media"))]
    pub(in crate::daemon::plugins::remote_desktop) fn with_process_instance_id(
        mut self,
        process_instance_id: Option<String>,
    ) -> Self {
        self.process_instance_id = process_instance_id;
        self
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn with_native_dimensions(
        mut self,
        native_dimensions: Option<(usize, usize)>,
    ) -> Self {
        (self.native_width, self.native_height) = native_dimensions
            .map(|(width, height)| (Some(width), Some(height)))
            .unwrap_or((None, None));
        self
    }

    #[cfg(any(
        test,
        not(all(
            feature = "native-media",
            any(target_os = "linux", target_os = "macos", target_os = "windows")
        ))
    ))]
    pub(in crate::daemon::plugins::remote_desktop) fn reverified_with_native_dimensions(
        mut self,
        native_dimensions: Option<(usize, usize)>,
    ) -> Self {
        self = self.with_native_dimensions(native_dimensions);
        self.verified_at_ms = unix_epoch_ms();
        self
    }

    pub(in crate::daemon::plugins::remote_desktop) fn with_app_window_set(
        mut self,
        app_window_set: AppWindowSetProof,
    ) -> Self {
        self.app_window_set = Some(app_window_set);
        self
    }

    pub(in crate::daemon::plugins::remote_desktop) fn with_app_surface_layout(
        mut self,
        app_surface_layout: AppSurfaceLayoutProof,
    ) -> Self {
        self.app_surface_layout = Some(app_surface_layout);
        self
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "backend": self.backend,
            "target_kind": self.target_kind.as_str(),
            "display_id": self.display_id,
            "window_id": self.window_id,
            "pid": self.pid,
            "process_instance_id": self.process_instance_id,
            "app_identity": self.app_identity,
            "bundle_id": self.bundle_id,
            "app_window_set": self.app_window_set.as_ref().map(AppWindowSetProof::to_value),
            "app_surface_layout": self.app_surface_layout.as_ref().map(AppSurfaceLayoutProof::to_value),
            "native_width": self.native_width,
            "native_height": self.native_height,
            "verified_at_ms": self.verified_at_ms,
        })
    }

    #[cfg(feature = "native-media")]
    pub(in crate::daemon::plugins::remote_desktop) fn native_dimensions(
        &self,
    ) -> Option<(usize, usize)> {
        Some((self.native_width?, self.native_height?))
    }

    fn from_recovery_value(value: &Value) -> anyhow::Result<Self> {
        Ok(Self {
            backend: required_owned_string(value, "backend")?,
            target_kind: RemoteDesktopTargetKind::from_recovery_str(required_string(
                value,
                "target_kind",
            )?)?,
            display_id: optional_u64(value, "display_id")?,
            window_id: optional_u64(value, "window_id")?,
            pid: optional_i64(value, "pid")?,
            process_instance_id: optional_string(value, "process_instance_id")?,
            app_identity: optional_string(value, "app_identity")?,
            bundle_id: optional_string(value, "bundle_id")?,
            app_window_set: optional_object_value(value, "app_window_set")
                .map(AppWindowSetProof::from_recovery_value)
                .transpose()?,
            app_surface_layout: optional_object_value(value, "app_surface_layout")
                .map(AppSurfaceLayoutProof::from_recovery_value)
                .transpose()?,
            native_width: optional_u64(value, "native_width")?.map(|value| value as usize),
            native_height: optional_u64(value, "native_height")?.map(|value| value as usize),
            verified_at_ms: required_u64(value, "verified_at_ms")?,
        })
    }

    fn validate_for_binding(
        &self,
        ability: &'static str,
        binding: &RemoteAppTargetBinding,
        phase: CaptureProofValidationPhase,
    ) -> Result<(), RemoteAppTargetError> {
        if self.backend != binding.backend {
            return Err(RemoteAppTargetError::new(
                ability,
                TargetResolutionError::TargetIdentityMismatch,
                format!(
                    "capture proof backend {} does not match binding backend {}",
                    self.backend, binding.backend
                ),
            ));
        }
        if self.target_kind != binding.target_kind {
            return Err(RemoteAppTargetError::new(
                ability,
                TargetResolutionError::TargetIdentityMismatch,
                format!(
                    "capture proof kind {} does not match binding kind {}",
                    self.target_kind.as_str(),
                    binding.target_kind.as_str()
                ),
            ));
        }
        let locator = binding.native_locator();
        if let Some(expected) = locator.display_id() {
            if self.display_id != Some(expected) {
                return Err(RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::DisplayIdentityMismatch,
                    format!(
                        "capture proof display {:?} does not match binding display {expected}",
                        self.display_id
                    ),
                ));
            }
        }
        if let Some(expected) = locator.window_id() {
            if self.window_id != Some(expected) {
                return Err(RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::TargetIdentityMismatch,
                    format!(
                        "capture proof window {:?} does not match binding window {expected}",
                        self.window_id
                    ),
                ));
            }
        }
        if !locator
            .app_identity_expectation()
            .evaluate(self.native_app_identity_candidate())
            .matched()
        {
            return Err(RemoteAppTargetError::new(
                ability,
                TargetResolutionError::TargetIdentityMismatch,
                format!(
                    "capture proof native app identity pid={:?}, app_identity={:?}, bundle_id={:?} does not match binding native app identity",
                    self.pid, self.app_identity, self.bundle_id
                ),
            ));
        }
        if binding.target_kind == RemoteDesktopTargetKind::Application {
            let actual = self.app_window_set.as_ref().ok_or_else(|| {
                RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::TargetMetadataIncomplete,
                    "application capture proof has no resolved platform-scoped window-set proof",
                )
            })?;
            let actual_surface_layout =
                if binding.platform == "macos" && binding.backend == "screencapturekit" {
                    Some(self.app_surface_layout.as_ref().ok_or_else(|| {
                        RemoteAppTargetError::new(
                            ability,
                            TargetResolutionError::TargetMetadataIncomplete,
                            "macOS application capture proof has no committed surface layout proof",
                        )
                    })?)
                } else {
                    self.app_surface_layout.as_ref()
                };
            match phase {
                CaptureProofValidationPhase::InitialCommit => {}
                CaptureProofValidationPhase::PendingMediaRebind
                | CaptureProofValidationPhase::ReverifyCommitted => {
                    let expected = binding.app_window_set.as_ref().ok_or_else(|| {
                        RemoteAppTargetError::new(
                            ability,
                            TargetResolutionError::TargetMetadataIncomplete,
                            "application target binding has no committed platform-scoped window-set proof",
                        )
                    })?;
                    if actual != expected {
                        return Err(RemoteAppTargetError::new(
                            ability,
                            TargetResolutionError::TargetIdentityChanged,
                            format!(
                                "capture proof application window set no longer matches the bound target; expected={}, actual={}",
                                expected.diagnostic_label(),
                                actual.diagnostic_label()
                            ),
                        ));
                    }
                    if phase == CaptureProofValidationPhase::ReverifyCommitted
                        && actual_surface_layout != binding.app_surface_layout.as_ref()
                    {
                        return Err(RemoteAppTargetError::new(
                            ability,
                            TargetResolutionError::TargetIdentityChanged,
                            "capture proof application surface layout no longer matches the bound target",
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    #[cfg(any(
        test,
        all(
            feature = "native-media",
            any(target_os = "linux", target_os = "macos", target_os = "windows")
        )
    ))]
    fn matches_committed_identity(&self, committed: &Self) -> bool {
        self.backend == committed.backend
            && self.target_kind == committed.target_kind
            && self.display_id == committed.display_id
            && self.window_id == committed.window_id
            && committed
                .native_app_identity_expectation()
                .evaluate(self.native_app_identity_candidate())
                .matched()
            && self.app_window_set == committed.app_window_set
            && self.app_surface_layout == committed.app_surface_layout
    }

    #[cfg(any(
        test,
        all(
            feature = "native-media",
            any(target_os = "linux", target_os = "macos", target_os = "windows")
        )
    ))]
    fn native_app_identity_expectation(&self) -> NativeAppIdentityExpectation<'_> {
        NativeAppIdentityExpectation {
            expected_pid: self.pid,
            expected_process_instance_id: self.process_instance_id.as_deref(),
            expected_bundle_id: self.bundle_id.as_deref(),
            expected_app_identity: self.app_identity.as_deref(),
        }
    }

    fn native_app_identity_candidate(&self) -> NativeAppIdentityCandidate<'_> {
        NativeAppIdentityCandidate::new(
            self.pid,
            self.bundle_id.as_deref(),
            self.app_identity.as_deref(),
        )
        .with_process_instance_id(self.process_instance_id.as_deref())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureProofValidationPhase {
    InitialCommit,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    PendingMediaRebind,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    ReverifyCommitted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct NativeTargetLocator {
    platform: String,
    discovery_backend: String,
    capture_backend: String,
    primary_display: bool,
    display_id: Option<u64>,
    window_id: Option<u64>,
    pid: Option<i64>,
    process_instance_id: Option<String>,
    app_identity: Option<String>,
    bundle_id: Option<String>,
    app_name: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct NativeAppIdentityCandidate<'a> {
    pid: Option<i64>,
    process_instance_id: Option<&'a str>,
    bundle_id: Option<&'a str>,
    app_identity: Option<&'a str>,
}

impl<'a> NativeAppIdentityCandidate<'a> {
    pub(in crate::daemon::plugins::remote_desktop) const fn new(
        pid: Option<i64>,
        bundle_id: Option<&'a str>,
        app_identity: Option<&'a str>,
    ) -> Self {
        Self {
            pid,
            process_instance_id: None,
            bundle_id,
            app_identity,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) const fn with_process_instance_id(
        mut self,
        process_instance_id: Option<&'a str>,
    ) -> Self {
        self.process_instance_id = process_instance_id;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct NativeAppIdentityExpectation<'a> {
    expected_pid: Option<i64>,
    expected_process_instance_id: Option<&'a str>,
    expected_bundle_id: Option<&'a str>,
    expected_app_identity: Option<&'a str>,
}

impl<'a> NativeAppIdentityExpectation<'a> {
    pub(in crate::daemon::plugins::remote_desktop) fn evaluate(
        self,
        candidate: NativeAppIdentityCandidate<'_>,
    ) -> NativeAppIdentityMatch {
        let pid_matches = self
            .expected_pid
            .is_none_or(|expected| candidate.pid == Some(expected));
        let process_instance_matches = self
            .expected_process_instance_id
            .is_none_or(|expected| candidate.process_instance_id == Some(expected));
        let bundle_matches = self.expected_bundle_id.is_none_or(|expected| {
            candidate.bundle_id == Some(expected) || candidate.app_identity == Some(expected)
        });
        let app_identity_matches = self.expected_app_identity.is_none_or(|expected| {
            candidate.app_identity == Some(expected) || candidate.bundle_id == Some(expected)
        });
        let any_expected_field_seen = self
            .expected_pid
            .is_some_and(|expected| candidate.pid == Some(expected))
            || self
                .expected_process_instance_id
                .is_some_and(|expected| candidate.process_instance_id == Some(expected))
            || self.expected_bundle_id.is_some_and(|expected| {
                candidate.bundle_id == Some(expected) || candidate.app_identity == Some(expected)
            })
            || self.expected_app_identity.is_some_and(|expected| {
                candidate.app_identity == Some(expected) || candidate.bundle_id == Some(expected)
            });
        NativeAppIdentityMatch {
            matched: pid_matches
                && process_instance_matches
                && bundle_matches
                && app_identity_matches,
            any_expected_field_seen,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct NativeAppIdentityMatch {
    matched: bool,
    any_expected_field_seen: bool,
}

impl NativeAppIdentityMatch {
    pub(in crate::daemon::plugins::remote_desktop) const fn matched(self) -> bool {
        self.matched
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) const fn any_expected_field_seen(self) -> bool {
        self.any_expected_field_seen
    }
}

impl NativeTargetLocator {
    pub(in crate::daemon::plugins::remote_desktop) fn display_id(&self) -> Option<u64> {
        self.display_id
    }

    pub(in crate::daemon::plugins::remote_desktop) fn window_id(&self) -> Option<u64> {
        self.window_id
    }

    pub(in crate::daemon::plugins::remote_desktop) fn pid(&self) -> Option<i64> {
        self.pid
    }

    pub(in crate::daemon::plugins::remote_desktop) fn process_instance_id(&self) -> Option<&str> {
        self.process_instance_id.as_deref()
    }

    #[cfg_attr(target_os = "macos", allow(dead_code))]
    #[cfg(any(test, feature = "native-media"))]
    pub(in crate::daemon::plugins::remote_desktop) fn app_identity(&self) -> Option<&str> {
        self.app_identity.as_deref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn bundle_id(&self) -> Option<&str> {
        self.bundle_id.as_deref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn app_identity_expectation(
        &self,
    ) -> NativeAppIdentityExpectation<'_> {
        NativeAppIdentityExpectation {
            expected_pid: self.pid,
            expected_process_instance_id: self.process_instance_id.as_deref(),
            expected_bundle_id: self.bundle_id.as_deref(),
            expected_app_identity: self.app_identity.as_deref(),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn capture_backend(&self) -> &str {
        &self.capture_backend
    }

    fn to_value(&self) -> Value {
        json!({
            "platform": self.platform,
            "discovery_backend": self.discovery_backend,
            "capture_backend": self.capture_backend,
            "primary_display": self.primary_display,
            "display_id": self.display_id,
            "window_id": self.window_id,
            "pid": self.pid,
            "process_instance_id": self.process_instance_id,
            "app_identity": self.app_identity,
            "bundle_id": self.bundle_id,
            "app_name": self.app_name,
            "title": self.title,
        })
    }

    fn from_recovery_value(value: &Value) -> anyhow::Result<Self> {
        Ok(Self {
            platform: required_owned_string(value, "platform")?,
            discovery_backend: required_owned_string(value, "discovery_backend")?,
            capture_backend: required_owned_string(value, "capture_backend")?,
            primary_display: value
                .get("primary_display")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            display_id: optional_u64(value, "display_id")?,
            window_id: optional_u64(value, "window_id")?,
            pid: optional_i64(value, "pid")?,
            process_instance_id: optional_string(value, "process_instance_id")?,
            app_identity: optional_string(value, "app_identity")?,
            bundle_id: optional_string(value, "bundle_id")?,
            app_name: optional_string(value, "app_name")?,
            title: optional_string(value, "title")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct ScopeAudit {
    requested_target_kind: RemoteDesktopTargetKind,
    effective_target_kind: RemoteDesktopTargetKind,
    capture_scope: CaptureScope,
    input_scope: InputScope,
    input_scope_reason: InputScopeReason,
    scope_widened: bool,
    display_fallback_used: bool,
}

impl ScopeAudit {
    fn to_value(&self, platform: &str) -> Value {
        json!({
            "requested_target_kind": self.requested_target_kind.as_str(),
            "effective_target_kind": self.effective_target_kind.as_str(),
            "target_model": self.effective_target_kind.target_model_for_platform(platform),
            "capture_surface": self.capture_scope.as_str(),
            "input_mode": self.input_scope.as_str(),
            "input_scope_reason": self.input_scope_reason.as_str(),
            "scope_widened": self.scope_widened,
            "display_fallback_used": self.display_fallback_used,
        })
    }
}

/// Minimal capture subject consumed by diagnostic/baseline capture adapters.
///
/// This is deliberately not `ResourceEntry`: the session aggregate owns a
/// resolved target binding, while xcap/synthetic capture traits still accept
/// the historical inventory DTO at the lowest backend boundary.
#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct DiagnosticCaptureSubject {
    resource_ura: String,
    owner_agent: String,
    kind: ResourceType,
    binding: ResourceBinding,
    hardware_id: String,
    display_name: String,
    metadata: Value,
    first_seen_at: String,
}

impl DiagnosticCaptureSubject {
    fn from_entry(entry: &ResourceEntry) -> Self {
        Self {
            resource_ura: entry.resource_ura.clone(),
            owner_agent: entry.owner_agent.clone(),
            kind: entry.kind,
            binding: entry.binding,
            hardware_id: entry.hardware_id.clone(),
            display_name: entry.display_name.clone(),
            metadata: entry.metadata.clone(),
            first_seen_at: entry.first_seen_at.clone(),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn hardware_id(&self) -> &str {
        &self.hardware_id
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_backend_resource_entry(
        &self,
    ) -> ResourceEntry {
        ResourceEntry {
            resource_ura: self.resource_ura.clone(),
            owner_agent: self.owner_agent.clone(),
            kind: self.kind,
            binding: self.binding,
            hardware_id: self.hardware_id.clone(),
            display_name: self.display_name.clone(),
            metadata: self.metadata.clone(),
            first_seen_at: self.first_seen_at.clone(),
        }
    }

    fn commit_application_window_set(&mut self, window_set: &AppWindowSetProof) {
        let Value::Object(metadata) = &mut self.metadata else {
            return;
        };
        metadata.insert(
            "display_id".to_string(),
            window_set.display_id.map_or(Value::Null, Value::from),
        );
        metadata.insert("display_ids".to_string(), json!(window_set.display_ids));
        metadata.insert(
            "primary_pid".to_string(),
            window_set.primary_pid.map_or(Value::Null, Value::from),
        );
        metadata.insert(
            "process_instance_id".to_string(),
            window_set
                .process_instance_id
                .as_ref()
                .map_or(Value::Null, |value| Value::String(value.clone())),
        );
        metadata.insert(
            "bundle_id".to_string(),
            window_set
                .bundle_id
                .as_ref()
                .map_or(Value::Null, |value| Value::String(value.clone())),
        );
        metadata.insert(
            "resolved_window_ids".to_string(),
            json!(window_set.resolved_window_ids),
        );
        metadata.insert(
            "window_set_epoch".to_string(),
            json!(window_set.window_set_epoch),
        );
        metadata.insert(
            "target_identity_epoch".to_string(),
            json!(window_set.window_set_epoch),
        );
    }

    fn commit_application_surface_layout(&mut self, layout: &AppSurfaceLayoutProof) {
        let Value::Object(metadata) = &mut self.metadata else {
            return;
        };
        metadata.insert(
            "front_to_back_surfaces".to_string(),
            layout.to_value()["front_to_back_surfaces"].clone(),
        );
        metadata.insert(
            "surface_layout_epoch".to_string(),
            json!(layout.layout_epoch()),
        );
        if let Some(geometry) = layout.union_geometry() {
            metadata.insert("union_x".to_string(), json!(geometry.x));
            metadata.insert("union_y".to_string(), json!(geometry.y));
            metadata.insert("union_width".to_string(), json!(geometry.width));
            metadata.insert("union_height".to_string(), json!(geometry.height));
        }
    }

    fn commit_target_geometry(&mut self, geometry: &TargetGeometry) {
        let Value::Object(metadata) = &mut self.metadata else {
            return;
        };
        metadata.insert("x".to_string(), json!(geometry.x));
        metadata.insert("y".to_string(), json!(geometry.y));
        metadata.insert("width".to_string(), json!(geometry.width));
        metadata.insert("height".to_string(), json!(geometry.height));
    }
}

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteAppTargetBinding {
    subject_ura: String,
    subject_display_name: String,
    target_kind: RemoteDesktopTargetKind,
    binding_id: String,
    binding_epoch: u64,
    target_identity_epoch: u64,
    target_geometry_revision: u64,
    media_source_epoch: u64,
    consent_epoch: u64,
    platform: String,
    backend: String,
    capture_scope: CaptureScope,
    input_scope: InputScope,
    native_locator: NativeTargetLocator,
    resolved_identity: TargetIdentity,
    app_window_set: Option<AppWindowSetProof>,
    app_surface_layout: Option<AppSurfaceLayoutProof>,
    geometry: TargetGeometry,
    scope_audit: ScopeAudit,
    diagnostic: Value,
    diagnostic_capture_subject: DiagnosticCaptureSubject,
    capture_proof: Option<ResolvedCaptureTargetProof>,
}

impl RemoteAppTargetBinding {
    pub(in crate::daemon::plugins::remote_desktop) fn from_recovery_value(
        value: &Value,
        subject_display_name: &str,
    ) -> anyhow::Result<Self> {
        let target_kind =
            RemoteDesktopTargetKind::from_recovery_str(required_string(value, "target_kind")?)?;
        let capture_scope =
            CaptureScope::from_recovery_str(required_string(value, "capture_scope")?)?;
        let input_scope = InputScope::from_recovery_str(required_string(value, "input_scope")?)?;
        let input_scope_reason =
            InputScopeReason::from_recovery_str(required_string(value, "input_scope_reason")?)?;
        let native_locator = NativeTargetLocator::from_recovery_value(required_object_value(
            value,
            "native_locator",
        )?)?;
        let resolved_identity = TargetIdentity::from_recovery_value(required_object_value(
            value,
            "resolved_identity",
        )?)?;
        let geometry =
            TargetGeometry::from_recovery_value(required_object_value(value, "bounds")?)?;
        let app_window_set = optional_object_value(value, "app_window_set")
            .map(AppWindowSetProof::from_recovery_value)
            .transpose()?;
        let app_surface_layout = optional_object_value(value, "app_surface_layout")
            .map(AppSurfaceLayoutProof::from_recovery_value)
            .transpose()?;
        let capture_proof = optional_object_value(value, "capture_proof")
            .map(ResolvedCaptureTargetProof::from_recovery_value)
            .transpose()?;
        let scope_ready = value
            .get("scope_ready")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| {
                value
                    .get("binding_ready")
                    .and_then(Value::as_bool)
                    .unwrap_or(true)
            });
        let subject_ura = required_owned_string(value, "subject_ura")?;
        let platform = required_owned_string(value, "platform")?;
        let backend = required_owned_string(value, "backend")?;
        if platform == "linux"
            && matches!(
                target_kind,
                RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application
            )
        {
            anyhow::bail!(
                "Linux RemoteApp Window/Application recovery requires a fresh X11 window-generation lease; recreate the session from fresh inventory"
            );
        }
        let diagnostic = json!({
            "status": "rehydrated",
            "reason_code": "daemon_restart_rehydrated",
            "recoverability": "retry_session",
            "frontend_action": FrontendAction::RetrySession.as_str(),
            "subject_ura": subject_ura,
            "target_kind": target_kind.as_str(),
            "binding_id": required_string(value, "binding_id")?,
        });
        let mut diagnostic_capture_subject = DiagnosticCaptureSubject {
            resource_ura: subject_ura.clone(),
            owner_agent: "easynet:///r/local/agent/remote-desktop.recovered".to_string(),
            kind: target_kind.resource_type(),
            binding: ResourceBinding::LocalDevice,
            hardware_id: resolved_identity.hardware_id.clone(),
            display_name: subject_display_name.to_string(),
            metadata: json!({
                "recovered": true,
                "platform": platform,
                "backend": native_locator.discovery_backend,
                "monitor_id": native_locator.display_id,
                "display_id": native_locator.display_id,
                "primary_display": native_locator.primary_display,
                "window_id": native_locator.window_id,
                "pid": native_locator.pid,
                "primary_pid": native_locator.pid,
                "process_instance_id": native_locator.process_instance_id,
                "app_identity": native_locator.app_identity,
                "bundle_id": native_locator.bundle_id,
                "app_name": native_locator.app_name,
                "title": native_locator.title,
            }),
            first_seen_at: String::new(),
        };
        if let Some(window_set) = app_window_set.as_ref() {
            diagnostic_capture_subject.commit_application_window_set(window_set);
        }
        if let Some(layout) = app_surface_layout.as_ref() {
            diagnostic_capture_subject.commit_application_surface_layout(layout);
        }
        Ok(Self {
            diagnostic_capture_subject,
            scope_audit: ScopeAudit {
                requested_target_kind: target_kind,
                effective_target_kind: target_kind,
                capture_scope,
                input_scope,
                input_scope_reason,
                scope_widened: !scope_ready,
                display_fallback_used: false,
            },
            subject_ura,
            subject_display_name: subject_display_name.to_string(),
            target_kind,
            binding_id: required_owned_string(value, "binding_id")?,
            binding_epoch: required_u64(value, "binding_epoch")?,
            target_identity_epoch: required_u64(value, "target_identity_epoch")?,
            target_geometry_revision: required_u64(value, "target_geometry_revision")?,
            media_source_epoch: required_u64(value, "media_source_epoch")?,
            consent_epoch: required_u64(value, "consent_epoch")?,
            platform,
            backend,
            capture_scope,
            input_scope,
            native_locator,
            resolved_identity,
            app_window_set,
            app_surface_layout,
            geometry,
            diagnostic,
            capture_proof,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn subject_ura(&self) -> &str {
        &self.subject_ura
    }

    pub(in crate::daemon::plugins::remote_desktop) fn subject_display_name(&self) -> &str {
        &self.subject_display_name
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_kind(
        &self,
    ) -> RemoteDesktopTargetKind {
        self.target_kind
    }

    pub(in crate::daemon::plugins::remote_desktop) fn binding_id(&self) -> &str {
        &self.binding_id
    }

    pub(in crate::daemon::plugins::remote_desktop) fn binding_epoch(&self) -> u64 {
        self.binding_epoch
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_identity_epoch(&self) -> u64 {
        self.target_identity_epoch
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_geometry_revision(&self) -> u64 {
        self.target_geometry_revision
    }

    pub(in crate::daemon::plugins::remote_desktop) fn media_source_epoch(&self) -> u64 {
        self.media_source_epoch
    }

    pub(in crate::daemon::plugins::remote_desktop) fn consent_epoch(&self) -> u64 {
        self.consent_epoch
    }

    pub(in crate::daemon::plugins::remote_desktop) fn input_scope(&self) -> InputScope {
        self.input_scope
    }

    pub(in crate::daemon::plugins::remote_desktop) fn input_scope_reason(&self) -> &'static str {
        self.scope_audit.input_scope_reason.as_str()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn native_locator(
        &self,
    ) -> &NativeTargetLocator {
        &self.native_locator
    }

    pub(in crate::daemon::plugins::remote_desktop) fn committed_app_window_set(
        &self,
    ) -> Option<&AppWindowSetProof> {
        self.app_window_set.as_ref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn committed_app_surface_layout(
        &self,
    ) -> Option<&AppSurfaceLayoutProof> {
        self.app_surface_layout.as_ref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn application_surface_rebind_candidate(
        &self,
        app_window_set: AppWindowSetProof,
        app_surface_layout: Option<AppSurfaceLayoutProof>,
        geometry: TargetGeometry,
        target_geometry_revision: u64,
        rebuild_media_source: bool,
    ) -> Option<Self> {
        if self.target_kind != RemoteDesktopTargetKind::Application {
            return None;
        }
        let mut candidate = self.clone();
        candidate.binding_epoch = candidate.binding_epoch.saturating_add(1);
        candidate.target_identity_epoch = app_window_set.window_set_epoch();
        candidate.target_geometry_revision = target_geometry_revision;
        if rebuild_media_source {
            candidate.media_source_epoch = candidate.media_source_epoch.saturating_add(1);
        }
        candidate.geometry = geometry;
        candidate.app_window_set = Some(app_window_set.clone());
        candidate.app_surface_layout = app_surface_layout.clone();
        candidate
            .diagnostic_capture_subject
            .commit_application_window_set(&app_window_set);
        if let Some(layout) = &app_surface_layout {
            candidate
                .diagnostic_capture_subject
                .commit_application_surface_layout(layout);
        }
        candidate.capture_proof = candidate.capture_proof.clone().map(|proof| {
            let proof = proof.with_app_window_set(app_window_set);
            match app_surface_layout {
                Some(layout) => proof.with_app_surface_layout(layout),
                None => proof,
            }
        });
        Some(candidate)
    }

    /// Build the next committed geometry generation for a display/window.
    ///
    /// Application geometry is derived from its committed surface-layout proof
    /// and must use `application_surface_rebind_candidate` instead. A resize of
    /// an active capture source advances `media_source_epoch`; a position-only
    /// move advances only the binding/geometry generation.
    pub(in crate::daemon::plugins::remote_desktop) fn geometry_rebind_candidate(
        &self,
        geometry: TargetGeometry,
        target_geometry_revision: u64,
        rebuild_media_source: bool,
    ) -> Option<Self> {
        if self.target_kind == RemoteDesktopTargetKind::Application {
            return None;
        }
        let mut candidate = self.clone();
        candidate.binding_epoch = candidate.binding_epoch.saturating_add(1);
        candidate.target_geometry_revision = target_geometry_revision;
        if rebuild_media_source {
            candidate.media_source_epoch = candidate.media_source_epoch.saturating_add(1);
        }
        candidate.geometry = geometry;
        candidate
            .diagnostic_capture_subject
            .commit_target_geometry(&candidate.geometry);
        Some(candidate)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn geometry(&self) -> &TargetGeometry {
        &self.geometry
    }

    pub(in crate::daemon::plugins::remote_desktop) fn diagnostic_capture_subject(
        &self,
    ) -> &DiagnosticCaptureSubject {
        &self.diagnostic_capture_subject
    }

    pub(in crate::daemon::plugins::remote_desktop) fn require_capture_proof(
        &self,
        ability: &'static str,
    ) -> Result<&ResolvedCaptureTargetProof, RemoteAppTargetError> {
        self.capture_proof.as_ref().ok_or_else(|| {
            RemoteAppTargetError::new(
                ability,
                TargetResolutionError::TargetMetadataIncomplete,
                "session target binding has no committed capture proof",
            )
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn commit_capture_proof(
        &mut self,
        ability: &'static str,
        proof: ResolvedCaptureTargetProof,
    ) -> Result<(), RemoteAppTargetError> {
        proof.validate_for_binding(ability, self, CaptureProofValidationPhase::InitialCommit)?;
        if self.target_kind == RemoteDesktopTargetKind::Application {
            self.app_window_set = Some(proof.app_window_set.clone().ok_or_else(|| {
                RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::TargetMetadataIncomplete,
                    "application capture proof has no resolved platform-scoped window-set proof",
                )
            })?);
            if let Some(committed_window_set) = self.app_window_set.as_ref() {
                self.target_identity_epoch = committed_window_set.window_set_epoch;
                self.diagnostic_capture_subject
                    .commit_application_window_set(committed_window_set);
            }
            self.app_surface_layout = proof.app_surface_layout.clone();
            if let Some(layout) = self.app_surface_layout.as_ref() {
                self.geometry = layout.union_geometry().ok_or_else(|| {
                    RemoteAppTargetError::new(
                        ability,
                        TargetResolutionError::TargetMetadataIncomplete,
                        "application capture proof surface layout has no valid union geometry",
                    )
                })?;
                self.diagnostic_capture_subject
                    .commit_application_surface_layout(layout);
            }
        }
        self.capture_proof = Some(proof);
        if let Value::Object(fields) = &mut self.diagnostic {
            fields.insert("live_identity_reverified".to_string(), json!(true));
        }
        Ok(())
    }

    #[cfg(any(
        test,
        all(
            feature = "native-media",
            any(target_os = "linux", target_os = "macos", target_os = "windows")
        )
    ))]
    pub(in crate::daemon::plugins::remote_desktop) fn validate_reverified_capture_proof(
        &self,
        ability: &'static str,
        proof: &ResolvedCaptureTargetProof,
    ) -> Result<(), RemoteAppTargetError> {
        proof.validate_for_binding(
            ability,
            self,
            CaptureProofValidationPhase::ReverifyCommitted,
        )?;
        let committed = self.require_capture_proof(ability)?;
        if !proof.matches_committed_identity(committed) {
            return Err(RemoteAppTargetError::new(
                ability,
                TargetResolutionError::TargetIdentityChanged,
                "live capture target no longer matches the session committed capture proof",
            ));
        }
        Ok(())
    }

    /// Validate a replacement application generation without treating the
    /// observer's surface geometry snapshot as capture-provider authority.
    /// Application identity and the exact window-id set stay closed; the
    /// ScreenCaptureKit layout proof is committed with the prepared generation.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn validate_pending_media_rebind_capture_proof(
        &self,
        ability: &'static str,
        proof: &ResolvedCaptureTargetProof,
    ) -> Result<(), RemoteAppTargetError> {
        proof.validate_for_binding(
            ability,
            self,
            CaptureProofValidationPhase::PendingMediaRebind,
        )
    }

    pub(in crate::daemon::plugins::remote_desktop) fn supports_xcap_adapter(&self) -> bool {
        if self.native_locator.capture_backend != "xcap" {
            return false;
        }
        match self.target_kind {
            RemoteDesktopTargetKind::Display => true,
            RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
                matches!(
                    self.native_locator.discovery_backend.as_str(),
                    "xcap" | "macos_core_graphics"
                ) && self.target_metadata_resolvable()
            }
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn supports_native_adapter(&self) -> bool {
        if self.platform != "macos" || self.native_locator.capture_backend != "screencapturekit" {
            return false;
        }
        match self.target_kind {
            RemoteDesktopTargetKind::Display => {
                self.native_locator.display_id.is_some() || self.native_locator.primary_display
            }
            RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
                self.target_metadata_resolvable()
            }
        }
    }

    fn target_metadata_resolvable(&self) -> bool {
        if self.platform == "linux"
            && matches!(
                self.target_kind,
                RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application
            )
            && self.native_locator.process_instance_id.is_none()
        {
            return false;
        }
        match self.target_kind {
            RemoteDesktopTargetKind::Application => {
                self.native_locator.pid.is_some()
                    || self.native_locator.bundle_id.is_some()
                    || self.native_locator.app_identity.is_some()
            }
            RemoteDesktopTargetKind::Window => {
                self.native_locator.window_id.is_some()
                    && (self.native_locator.pid.is_some()
                        || self.native_locator.app_identity.is_some()
                        || self.native_locator.bundle_id.is_some())
            }
            RemoteDesktopTargetKind::Display => true,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        let binding_ready = self.production_scope_ready();
        json!({
            "subject_ura": self.subject_ura,
            "target_kind": self.target_kind.as_str(),
            "target_model": self.target_kind.target_model_for_platform(&self.platform),
            "binding_id": self.binding_id,
            "binding_epoch": self.binding_epoch,
            "target_identity_epoch": self.target_identity_epoch,
            "target_geometry_revision": self.target_geometry_revision,
            "media_source_epoch": self.media_source_epoch,
            "consent_epoch": self.consent_epoch,
            "platform": self.platform,
            "backend": self.backend,
            "capture_scope": self.capture_scope.as_str(),
            "input_scope": self.input_scope.as_str(),
            "input_scope_reason": self.scope_audit.input_scope_reason.as_str(),
            "native_locator": self.native_locator.to_value(),
            "resolved_identity": self.resolved_identity.to_value(),
            "app_window_set": self.app_window_set.as_ref().map(AppWindowSetProof::to_value),
            "app_surface_layout": self.app_surface_layout.as_ref().map(AppSurfaceLayoutProof::to_value),
            "capture_proof": self.capture_proof.as_ref().map(ResolvedCaptureTargetProof::to_value),
            "bounds": self.geometry.to_value(),
            "binding_ready": binding_ready,
            "scope_ready": binding_ready,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_tracking_value(
        &self,
        target_identity_epoch: u64,
        target_geometry_revision: u64,
        media_source_epoch: u64,
        geometry: &TargetGeometry,
    ) -> Value {
        let mut value = self.to_value();
        let Value::Object(fields) = &mut value else {
            return value;
        };
        fields.insert(
            "target_identity_epoch".to_string(),
            json!(target_identity_epoch),
        );
        fields.insert(
            "target_geometry_revision".to_string(),
            json!(target_geometry_revision),
        );
        fields.insert("media_source_epoch".to_string(), json!(media_source_epoch));
        fields.insert("bounds".to_string(), geometry.to_value());
        value
    }

    pub(in crate::daemon::plugins::remote_desktop) fn scope_audit_value(&self) -> Value {
        self.scope_audit.to_value(&self.platform)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn latest_target_diagnostic_value(
        &self,
    ) -> Value {
        self.diagnostic.clone()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn production_scope_ready(&self) -> bool {
        !self.scope_audit.scope_widened && !self.scope_audit.display_fallback_used
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn with_scope_audit_for_test(
        mut self,
        scope_widened: bool,
        display_fallback_used: bool,
    ) -> Self {
        self.scope_audit.scope_widened = scope_widened;
        self.scope_audit.display_fallback_used = display_fallback_used;
        self
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_bound_event_payload(&self) -> Value {
        json!({
            "subject_ura": self.subject_ura,
            "target_kind": self.target_kind.as_str(),
            "target_model": self.target_kind.target_model_for_platform(&self.platform),
            "binding_id": self.binding_id,
            "binding_epoch": self.binding_epoch,
            "target_identity_epoch": self.target_identity_epoch,
            "target_geometry_revision": self.target_geometry_revision,
            "media_source_epoch": self.media_source_epoch,
            "consent_epoch": self.consent_epoch,
            "capture_scope": self.capture_scope.as_str(),
            "input_scope": self.input_scope.as_str(),
            "input_scope_reason": self.scope_audit.input_scope_reason.as_str(),
            "app_window_set": self.app_window_set.as_ref().map(AppWindowSetProof::to_value),
            "app_surface_layout": self.app_surface_layout.as_ref().map(AppSurfaceLayoutProof::to_value),
            "capture_proof": self.capture_proof.as_ref().map(ResolvedCaptureTargetProof::to_value),
            "reason_code": "target_bound",
            "recoverability": "continue",
            "binding_ready": self.production_scope_ready(),
            "scope_ready": self.production_scope_ready(),
            "scope_widened": self.scope_audit.scope_widened,
            "display_fallback_used": self.scope_audit.display_fallback_used,
        })
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn verify_target_binding_for_session(
    ability: &'static str,
    binding: &RemoteAppTargetBinding,
) -> Result<ResolvedCaptureTargetProof, RemoteAppTargetError> {
    crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore::assert_current_thread_unlocked(
        "remote_desktop.target.verify_target_binding_for_session",
    );
    platform_live_resolution::verify_target_binding_for_session(ability, binding)
}

#[cfg(all(target_os = "macos", feature = "native-media"))]
mod platform_live_resolution {
    use super::{RemoteAppTargetBinding, RemoteAppTargetError, ResolvedCaptureTargetProof};

    pub(super) fn verify_target_binding_for_session(
        ability: &'static str,
        binding: &RemoteAppTargetBinding,
    ) -> Result<ResolvedCaptureTargetProof, RemoteAppTargetError> {
        crate::daemon::plugins::remote_desktop::media_host_probe::verify_binding(ability, binding)
    }
}

#[cfg(all(not(target_os = "macos"), feature = "native-media"))]
mod platform_live_resolution {
    use super::{
        RemoteAppTargetBinding, RemoteAppTargetError, RemoteDesktopTargetKind,
        ResolvedCaptureTargetProof, TargetResolutionError,
    };
    use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
        capture_rgb_with_xcap, ScreenCaptureOptions,
    };

    pub(super) fn verify_target_binding_for_session(
        ability: &'static str,
        binding: &RemoteAppTargetBinding,
    ) -> Result<ResolvedCaptureTargetProof, RemoteAppTargetError> {
        if !binding.supports_xcap_adapter() {
            return Err(RemoteAppTargetError::new(
                ability,
                TargetResolutionError::CaptureBackendUnavailable,
                format!(
                    "{} target binding cannot be resolved by the xcap platform adapter",
                    binding.target_kind().as_str()
                ),
            ));
        }
        let frame = capture_rgb_with_xcap(
            &binding
                .diagnostic_capture_subject()
                .to_backend_resource_entry(),
            &ScreenCaptureOptions::default(),
        )
        .map_err(|error| {
            RemoteAppTargetError::new(
                ability,
                TargetResolutionError::CaptureBackendUnavailable,
                format!(
                    "xcap failed to prove the exact {} capture target: {error}",
                    binding.target_kind().as_str()
                ),
            )
        })?;
        let locator = binding.native_locator();
        let mut proof =
            ResolvedCaptureTargetProof::new(locator.capture_backend.clone(), binding.target_kind())
                .with_native_identity(
                    locator.display_id(),
                    locator.window_id(),
                    locator.pid(),
                    locator.app_identity().map(ToOwned::to_owned),
                    locator.bundle_id().map(ToOwned::to_owned),
                )
                .with_process_instance_id(locator.process_instance_id().map(ToOwned::to_owned))
                .with_native_dimensions(Some(frame.native_dimensions()));
        if binding.target_kind() == RemoteDesktopTargetKind::Application {
            let window_set = binding.committed_app_window_set().cloned().ok_or_else(|| {
                RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::TargetMetadataIncomplete,
                    "xcap application binding has no committed process-scoped window set",
                )
            })?;
            let surface_layout =
                binding
                    .committed_app_surface_layout()
                    .cloned()
                    .ok_or_else(|| {
                        RemoteAppTargetError::new(
                            ability,
                            TargetResolutionError::TargetMetadataIncomplete,
                            "xcap application binding has no committed surface-layout proof",
                        )
                    })?;
            proof = proof
                .with_app_window_set(window_set)
                .with_app_surface_layout(surface_layout);
        }
        Ok(proof)
    }
}

#[cfg(not(feature = "native-media"))]
mod platform_live_resolution {
    use super::{
        RemoteAppTargetBinding, RemoteAppTargetError, RemoteDesktopTargetKind,
        ResolvedCaptureTargetProof, TargetResolutionError,
    };

    pub(super) fn verify_target_binding_for_session(
        ability: &'static str,
        binding: &RemoteAppTargetBinding,
    ) -> Result<ResolvedCaptureTargetProof, RemoteAppTargetError> {
        match binding.target_kind() {
            RemoteDesktopTargetKind::Display => Ok(ResolvedCaptureTargetProof::new(
                binding.native_locator().capture_backend.clone(),
                RemoteDesktopTargetKind::Display,
            )
            .with_native_identity(
                binding.native_locator().display_id(),
                None,
                None,
                None,
                None,
            )),
            RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
                Err(RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::CaptureBackendUnavailable,
                    format!(
                        "{} targets require the native-media platform capture feature",
                        binding.target_kind().as_str()
                    ),
                ))
            }
        }
    }
}

#[derive(Debug, Default, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct ResourceEntryTargetResolver;

impl ResourceEntryTargetResolver {
    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn resolve_for_session(
        &self,
        ability: &'static str,
        entry: &ResourceEntry,
        requested_mode: &str,
        consent_epoch: u64,
    ) -> Result<RemoteAppTargetBinding, RemoteAppTargetError> {
        resolve_resource_entry_for_session(ability, entry, requested_mode, consent_epoch, false)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn resolve_for_session_with_input_consent(
        &self,
        ability: &'static str,
        entry: &ResourceEntry,
        requested_mode: &str,
        consent_epoch: u64,
        input_control_granted: bool,
    ) -> Result<RemoteAppTargetBinding, RemoteAppTargetError> {
        resolve_resource_entry_for_session(
            ability,
            entry,
            requested_mode,
            consent_epoch,
            input_control_granted,
        )
    }
}

fn resolve_resource_entry_for_session(
    ability: &'static str,
    entry: &ResourceEntry,
    requested_mode: &str,
    consent_epoch: u64,
    input_control_granted: bool,
) -> Result<RemoteAppTargetBinding, RemoteAppTargetError> {
    crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore::assert_current_thread_unlocked(
        "remote_desktop.target.resolve_for_session",
    );
    if entry.binding != ResourceBinding::LocalDevice {
        return Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::UnsupportedCaptureScope,
            format!(
                "remote desktop target {} is not locally bound",
                entry.resource_ura
            ),
        ));
    }
    validate_owner_agent_ura(ability, entry)?;
    let target_kind = RemoteDesktopTargetKind::try_from(entry.kind)
        .map_err(|error| RemoteAppTargetError::new(ability, error.reason(), error.to_string()))?;
    validate_resource_inventory_state(ability, entry, target_kind)?;
    let capture_scope = capture_scope_for_kind(target_kind);
    let input_scope_decision = input_scope_for_request(
        target_kind,
        requested_mode,
        input_control_granted,
        TargetScopedInputIsolation::CURRENT,
    );
    let input_scope = input_scope_decision.scope();
    let platform = metadata_string(entry, "platform").unwrap_or_else(|| {
        if cfg!(target_os = "macos") {
            "macos".to_string()
        } else if cfg!(target_os = "windows") {
            "windows".to_string()
        } else if cfg!(target_os = "linux") {
            "linux".to_string()
        } else {
            "unknown".to_string()
        }
    });
    let display_id = display_id(entry);
    validate_required_identity(ability, entry, target_kind, display_id, &platform)?;
    let discovery_backend =
        metadata_string(entry, "backend").unwrap_or_else(|| "resource_registry".to_string());
    let capture_backend = capture_backend_for_entry(&platform, entry, target_kind);
    let geometry = match target_kind {
        RemoteDesktopTargetKind::Application => TargetGeometry::from_metadata(entry, Some("union")),
        _ => TargetGeometry::from_metadata(entry, None),
    };
    let native_locator = NativeTargetLocator {
        platform: platform.clone(),
        discovery_backend,
        capture_backend: capture_backend.clone(),
        primary_display: metadata_bool(entry, "primary_display"),
        display_id,
        window_id: metadata_u64(entry, "window_id"),
        pid: metadata_i64(entry, "pid").or_else(|| metadata_i64(entry, "primary_pid")),
        process_instance_id: metadata_string(entry, "process_instance_id"),
        app_identity: metadata_string(entry, "app_identity"),
        bundle_id: metadata_string(entry, "bundle_id"),
        app_name: metadata_string(entry, "app_name"),
        title: metadata_string(entry, "title"),
    };
    let resolved_identity = TargetIdentity::from_entry(entry, display_id);
    let app_window_set = AppWindowSetProof::from_entry(entry, display_id);
    let app_surface_layout = AppSurfaceLayoutProof::from_entry(entry);
    let binding_id = mint_binding_id(entry, &native_locator);
    let target_identity_epoch = metadata_u64(entry, "lifecycle_epoch")
        .or_else(|| metadata_u64(entry, "target_identity_epoch"))
        .unwrap_or(1);
    let target_geometry_revision = metadata_u64(entry, "geometry_revision").unwrap_or(1);
    let media_source_epoch = 1;
    let inventory_observed_at_ms = metadata_freshness_u64(entry, "observed_at_ms");
    let inventory_stale_after_ms = metadata_freshness_u64(entry, "stale_after_ms");
    let inventory_cache_expired_before_live_verification = matches!(
        target_kind,
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application
    ) && inventory_stale_after_ms
        .is_some_and(|stale_after_ms| stale_after_ms <= unix_epoch_ms());
    let scope_audit = ScopeAudit {
        requested_target_kind: target_kind,
        effective_target_kind: target_kind,
        capture_scope,
        input_scope,
        input_scope_reason: input_scope_decision.reason(),
        scope_widened: false,
        display_fallback_used: false,
    };
    let diagnostic = json!({
        "status": "resolved",
        "reason": Value::Null,
        "requested_identity": requested_identity_projection(entry),
        "resolved_identity": resolved_identity.to_value(),
        "match_strategy": match_strategy_for_kind(target_kind, &platform),
        "capture_backend": capture_backend,
        "target_model": target_kind.target_model_for_platform(&platform),
        "inventory_observed_at_ms": inventory_observed_at_ms,
        "inventory_stale_after_ms": inventory_stale_after_ms,
        "inventory_cache_expired_before_live_verification": inventory_cache_expired_before_live_verification,
        "live_identity_reverified": false,
        "display_fallback_used": false,
        "frontend_action": Value::Null,
    });
    Ok(RemoteAppTargetBinding {
        subject_ura: entry.resource_ura.clone(),
        subject_display_name: entry.display_name.clone(),
        target_kind,
        binding_id,
        binding_epoch: 1,
        target_identity_epoch,
        target_geometry_revision,
        media_source_epoch,
        consent_epoch,
        platform,
        backend: capture_backend,
        capture_scope,
        input_scope,
        native_locator,
        resolved_identity,
        app_window_set,
        app_surface_layout,
        geometry,
        scope_audit,
        diagnostic,
        diagnostic_capture_subject: DiagnosticCaptureSubject::from_entry(entry),
        capture_proof: None,
    })
}

fn validate_owner_agent_ura(
    ability: &'static str,
    entry: &ResourceEntry,
) -> Result<(), RemoteAppTargetError> {
    let owner_agent = entry.owner_agent.trim();
    let parsed = parse_ura(owner_agent).map_err(|error| {
        RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetMetadataIncomplete,
            format!(
                "remote desktop target {} owner_agent must be an Agent/SystemAgent URA; got invalid owner_agent `{}`: {}",
                entry.resource_ura, entry.owner_agent, error
            ),
        )
    })?;

    if parsed.kind == URAKind::Agent {
        return Ok(());
    }

    Err(RemoteAppTargetError::new(
        ability,
        TargetResolutionError::TargetMetadataIncomplete,
        format!(
            "remote desktop target {} owner_agent must be an Agent/SystemAgent URA; got {} URA `{}`",
            entry.resource_ura,
            crate::core::ura::ura_kind_scope_label(parsed.kind),
            entry.owner_agent
        ),
    ))
}

fn validate_resource_inventory_state(
    ability: &'static str,
    entry: &ResourceEntry,
    target_kind: RemoteDesktopTargetKind,
) -> Result<(), RemoteAppTargetError> {
    let availability = metadata_string(entry, "availability");
    if let Some(availability_value) = availability.as_deref() {
        if availability_value != "available" {
            let stale_reason = metadata_string(entry, "stale_reason");
            let reason = stale_reason
                .as_deref()
                .and_then(target_resolution_reason_from_str)
                .unwrap_or(TargetResolutionError::TargetStale);
            return Err(RemoteAppTargetError::new(
                ability,
                reason,
                format!(
                    "remote desktop target {} is not available in the live inventory; availability={availability_value}; stale_reason={}",
                    entry.resource_ura,
                    stale_reason.as_deref().unwrap_or("unknown")
                ),
            ));
        }
    }

    if matches!(
        target_kind,
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application
    ) && availability.as_deref() != Some("available")
    {
        return Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetStale,
            format!(
                "{} target {} is missing live inventory availability; call resource.refresh_remote_targets before creating a session",
                target_kind.as_str(),
                entry.resource_ura
            ),
        ));
    }

    if matches!(
        target_kind,
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application
    ) && (metadata_freshness_u64(entry, "observed_at_ms").is_none()
        || metadata_freshness_u64(entry, "stale_after_ms").is_none()
        || metadata_freshness_string(entry, "source").is_none())
    {
        return Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetStale,
            format!(
                "{} target {} is missing live inventory freshness; call resource.refresh_remote_targets before creating a session",
                target_kind.as_str(),
                entry.resource_ura
            ),
        ));
    }

    Ok(())
}

fn target_resolution_reason_from_str(value: &str) -> Option<TargetResolutionError> {
    ALL_TARGET_RESOLUTION_ERRORS
        .iter()
        .copied()
        .find(|reason| reason.as_str() == value)
}

fn validate_required_identity(
    ability: &'static str,
    entry: &ResourceEntry,
    target_kind: RemoteDesktopTargetKind,
    display_id: Option<u64>,
    platform: &str,
) -> Result<(), RemoteAppTargetError> {
    if platform == "linux"
        && matches!(
            target_kind,
            RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application
        )
    {
        validate_linux_process_instance_identity(ability, entry, target_kind)?;
    }
    match target_kind {
        RemoteDesktopTargetKind::Display => {
            if display_id.is_some() || metadata_bool(entry, "primary_display") {
                return Ok(());
            }
            Err(RemoteAppTargetError::new(
                ability,
                TargetResolutionError::DisplayIdentityMissing,
                "display targets require display_id/monitor_id or explicit primary_display=true",
            ))
        }
        RemoteDesktopTargetKind::Window => {
            if metadata_u64(entry, "window_id").is_none() {
                return Err(RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::TargetMetadataIncomplete,
                    "window targets require a stable window_id",
                ));
            }
            if metadata_i64(entry, "pid").is_none()
                && metadata_string(entry, "app_identity").is_none()
                && metadata_string(entry, "bundle_id").is_none()
            {
                return Err(RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::TargetIdentityAmbiguous,
                    "window targets require owner pid, app_identity, or bundle_id in addition to window_id; app_name/title are diagnostic hints, not production routing identity",
                ));
            }
            Ok(())
        }
        RemoteDesktopTargetKind::Application => {
            if metadata_i64(entry, "primary_pid").is_none()
                && metadata_string(entry, "app_identity").is_none()
                && metadata_string(entry, "bundle_id").is_none()
            {
                return Err(RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::TargetIdentityAmbiguous,
                    "application targets require primary_pid, app_identity, or bundle_id; app_name alone is not production routing identity",
                ));
            }
            if metadata_u64_array(entry, "resolved_window_ids").is_empty()
                || metadata_u64(entry, "window_set_epoch").is_none()
            {
                return Err(RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::TargetMetadataIncomplete,
                    "application targets require resolved_window_ids and window_set_epoch so capture can prove the committed app window set",
                ));
            }
            if AppSurfaceLayoutProof::from_entry(entry).is_none() {
                return Err(RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::TargetMetadataIncomplete,
                    "application targets require a canonical front_to_back_surfaces layout and matching surface_layout_epoch",
                ));
            }
            Ok(())
        }
    }
}

fn validate_linux_process_instance_identity(
    ability: &'static str,
    entry: &ResourceEntry,
    target_kind: RemoteDesktopTargetKind,
) -> Result<(), RemoteAppTargetError> {
    let pid = metadata_i64(entry, "pid")
        .or_else(|| metadata_i64(entry, "primary_pid"))
        .filter(|pid| *pid > 0);
    let start_ticks = metadata_u64(entry, "process_start_ticks").filter(|ticks| *ticks > 0);
    let boot_id =
        metadata_string(entry, "process_boot_id").filter(|value| !value.trim().is_empty());
    let process_instance_id =
        metadata_string(entry, "process_instance_id").filter(|value| !value.trim().is_empty());
    let Some((pid, start_ticks, boot_id, process_instance_id)) = pid
        .zip(start_ticks)
        .zip(boot_id)
        .zip(process_instance_id)
        .map(|(((pid, start_ticks), boot_id), process_instance_id)| {
            (pid, start_ticks, boot_id, process_instance_id)
        })
    else {
        return Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetMetadataIncomplete,
            format!(
                "Linux {} targets require process_instance_id, process_boot_id, positive process_start_ticks, and owner pid from the same live observation",
                target_kind.as_str()
            ),
        ));
    };
    let expected = format!("linux:{boot_id}:{pid}:{start_ticks}");
    if process_instance_id != expected {
        return Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetIdentityMismatch,
            format!(
                "Linux {} target process instance metadata is inconsistent with its owner pid/starttime/boot identity",
                target_kind.as_str()
            ),
        ));
    }
    Ok(())
}

fn capture_scope_for_kind(target_kind: RemoteDesktopTargetKind) -> CaptureScope {
    match target_kind {
        RemoteDesktopTargetKind::Display => CaptureScope::DisplaySurface,
        RemoteDesktopTargetKind::Window => CaptureScope::WindowSurface,
        RemoteDesktopTargetKind::Application => CaptureScope::AppSurface,
    }
}

fn input_scope_for_request(
    target_kind: RemoteDesktopTargetKind,
    requested_mode: &str,
    input_control_granted: bool,
    target_isolation: TargetScopedInputIsolation,
) -> InputScopeDecision {
    if requested_mode != "interactive" {
        return InputScopeDecision::new(InputScope::ViewOnly, InputScopeReason::RequestedViewOnly);
    }
    match target_kind {
        RemoteDesktopTargetKind::Display => {
            if input_control_granted {
                InputScopeDecision::new(
                    InputScope::DisplayGlobal,
                    InputScopeReason::InputControlGranted,
                )
            } else {
                // Capture/session consent does not authorize keyboard or
                // pointer input. Display-global input requires an explicit
                // input-control grant in the consumed consent ticket.
                InputScopeDecision::new(
                    InputScope::ViewOnly,
                    InputScopeReason::InputConsentRequired,
                )
            }
        }
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
            if !target_isolation.is_safe() {
                return InputScopeDecision::new(
                    InputScope::ViewOnly,
                    InputScopeReason::TargetScopedInputUnsafe,
                );
            }
            if input_control_granted {
                InputScopeDecision::new(
                    InputScope::TargetLocal,
                    InputScopeReason::TargetScopedInputGuarded,
                )
            } else {
                InputScopeDecision::new(
                    InputScope::ViewOnly,
                    InputScopeReason::InputConsentRequired,
                )
            }
        }
    }
}

fn capture_backend_for_entry(
    platform: &str,
    entry: &ResourceEntry,
    target_kind: RemoteDesktopTargetKind,
) -> String {
    if platform == "macos"
        && cfg!(target_os = "macos")
        && matches!(
            target_kind,
            RemoteDesktopTargetKind::Display
                | RemoteDesktopTargetKind::Window
                | RemoteDesktopTargetKind::Application
        )
    {
        return "screencapturekit".to_string();
    }
    metadata_string(entry, "backend").unwrap_or_else(|| platform.to_string())
}

fn match_strategy_for_kind(target_kind: RemoteDesktopTargetKind, platform: &str) -> &'static str {
    match target_kind {
        RemoteDesktopTargetKind::Display => "display_id_or_explicit_primary",
        RemoteDesktopTargetKind::Window => "window_id_plus_owner",
        RemoteDesktopTargetKind::Application if platform == "macos" => {
            "multi_surface_app_identity_window_set"
        }
        RemoteDesktopTargetKind::Application => "process_scoped_app_window_set",
    }
}

fn requested_identity_projection(entry: &ResourceEntry) -> Value {
    json!({
        "resource_ura": entry.resource_ura,
        "resource_type": entry.kind.as_str(),
        "hardware_id": entry.hardware_id,
        "metadata": entry.metadata,
    })
}

fn display_id(entry: &ResourceEntry) -> Option<u64> {
    metadata_u64(entry, "display_id").or_else(|| metadata_u64(entry, "monitor_id"))
}

fn metadata_u64(entry: &ResourceEntry, key: &str) -> Option<u64> {
    entry.metadata.get(key).and_then(Value::as_u64)
}

fn metadata_i64(entry: &ResourceEntry, key: &str) -> Option<i64> {
    entry.metadata.get(key).and_then(Value::as_i64)
}

fn metadata_f64(entry: &ResourceEntry, key: &str) -> Option<f64> {
    entry
        .metadata
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

fn metadata_bool(entry: &ResourceEntry, key: &str) -> bool {
    entry
        .metadata
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn metadata_string(entry: &ResourceEntry, key: &str) -> Option<String> {
    entry
        .metadata
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn metadata_freshness_u64(entry: &ResourceEntry, key: &str) -> Option<u64> {
    entry
        .metadata
        .get("freshness")
        .and_then(Value::as_object)
        .and_then(|freshness| freshness.get(key))
        .and_then(Value::as_u64)
}

fn metadata_freshness_string(entry: &ResourceEntry, key: &str) -> Option<String> {
    entry
        .metadata
        .get("freshness")
        .and_then(Value::as_object)
        .and_then(|freshness| freshness.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn metadata_u64_array(entry: &ResourceEntry, key: &str) -> Vec<u64> {
    let mut values = entry
        .metadata
        .get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_u64).collect::<Vec<_>>())
        .unwrap_or_default();
    values.sort_unstable();
    values.dedup();
    values
}

fn required_object_value<'a>(value: &'a Value, field: &'static str) -> anyhow::Result<&'a Value> {
    let child = value
        .get(field)
        .ok_or_else(|| anyhow::anyhow!("RemoteApp recovery target_binding requires {field}"))?;
    if !child.is_object() {
        anyhow::bail!("RemoteApp recovery target_binding {field} must be an object");
    }
    Ok(child)
}

fn optional_object_value<'a>(value: &'a Value, field: &'static str) -> Option<&'a Value> {
    value.get(field).filter(|child| child.is_object())
}

fn required_string<'a>(value: &'a Value, field: &'static str) -> anyhow::Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("RemoteApp recovery target_binding requires non-empty {field}")
        })
}

fn required_owned_string(value: &Value, field: &'static str) -> anyhow::Result<String> {
    required_string(value, field).map(str::to_string)
}

fn optional_string(value: &Value, field: &'static str) -> anyhow::Result<Option<String>> {
    match value.get(field) {
        Some(Value::String(raw)) => {
            let trimmed = raw.trim();
            Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
        }
        Some(Value::Null) | None => Ok(None),
        Some(_) => anyhow::bail!("RemoteApp recovery target_binding {field} must be a string"),
    }
}

fn required_u64(value: &Value, field: &'static str) -> anyhow::Result<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("RemoteApp recovery target_binding requires u64 {field}"))
}

fn required_i64(value: &Value, field: &'static str) -> anyhow::Result<i64> {
    value
        .get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| anyhow::anyhow!("RemoteApp recovery requires integer field `{field}`"))
}

fn optional_u64(value: &Value, field: &'static str) -> anyhow::Result<Option<u64>> {
    match value.get(field) {
        Some(Value::Number(number)) => number.as_u64().map(Some).ok_or_else(|| {
            anyhow::anyhow!("RemoteApp recovery target_binding {field} must be a u64")
        }),
        Some(Value::Null) | None => Ok(None),
        Some(_) => anyhow::bail!("RemoteApp recovery target_binding {field} must be a u64"),
    }
}

fn optional_i64(value: &Value, field: &'static str) -> anyhow::Result<Option<i64>> {
    match value.get(field) {
        Some(Value::Number(number)) => number.as_i64().map(Some).ok_or_else(|| {
            anyhow::anyhow!("RemoteApp recovery target_binding {field} must be an i64")
        }),
        Some(Value::Null) | None => Ok(None),
        Some(_) => anyhow::bail!("RemoteApp recovery target_binding {field} must be an i64"),
    }
}

fn optional_f64(value: &Value, field: &'static str) -> anyhow::Result<Option<f64>> {
    match value.get(field) {
        Some(Value::Number(number)) => number
            .as_f64()
            .filter(|value| value.is_finite())
            .map(Some)
            .ok_or_else(|| {
                anyhow::anyhow!("RemoteApp recovery target_binding {field} must be a finite f64")
            }),
        Some(Value::Null) | None => Ok(None),
        Some(_) => anyhow::bail!("RemoteApp recovery target_binding {field} must be a number"),
    }
}

fn mint_binding_id(entry: &ResourceEntry, locator: &NativeTargetLocator) -> String {
    let mut hasher = DefaultHasher::new();
    entry.resource_ura.hash(&mut hasher);
    entry.hardware_id.hash(&mut hasher);
    locator.display_id.hash(&mut hasher);
    locator.window_id.hash(&mut hasher);
    locator.pid.hash(&mut hasher);
    locator.process_instance_id.hash(&mut hasher);
    locator.app_identity.hash(&mut hasher);
    locator.bundle_id.hash(&mut hasher);
    locator.app_name.hash(&mut hasher);
    locator.title.hash(&mut hasher);
    let now = unix_epoch_ms();
    format!("tb_{now:x}_{:016x}", hasher.finish())
}

fn compute_window_set_epoch(
    display_id: Option<u64>,
    bundle_id: Option<&str>,
    primary_pid: Option<i64>,
    process_instance_id: Option<&str>,
    resolved_window_ids: &[u64],
) -> u64 {
    application_window_set_epoch_with_process_instance(
        display_id,
        bundle_id,
        primary_pid,
        process_instance_id,
        resolved_window_ids,
    )
}

fn unix_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;

    fn entry(kind: ResourceType, metadata: Value) -> ResourceEntry {
        ResourceEntry {
            resource_ura: "easynet:///r/acme/resource/device.01DEV/streams/display.test"
                .to_string(),
            owner_agent: "easynet:///r/acme/agent/device.01DEV.runtime-resources".to_string(),
            kind,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "test".to_string(),
            display_name: "Test Target".to_string(),
            metadata,
            first_seen_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn live_metadata(mut metadata: Value) -> Value {
        let map = metadata
            .as_object_mut()
            .expect("test metadata must be an object");
        map.insert("availability".to_string(), json!("available"));
        map.insert(
            "freshness".to_string(),
            json!({
                "observed_at_ms": 1,
                "stale_after_ms": u64::MAX,
                "source": "live_refresh",
            }),
        );
        if !map.contains_key("front_to_back_surfaces") {
            let window_ids = map
                .get("resolved_window_ids")
                .and_then(Value::as_array)
                .map(|values| values.iter().filter_map(Value::as_u64).collect::<Vec<_>>())
                .unwrap_or_default();
            if !window_ids.is_empty() {
                let layout = application_layout(&window_ids);
                map.insert(
                    "front_to_back_surfaces".to_string(),
                    layout.to_value()["front_to_back_surfaces"].clone(),
                );
                map.insert(
                    "surface_layout_epoch".to_string(),
                    json!(layout.layout_epoch()),
                );
            }
        }
        metadata
    }

    fn application_layout(window_ids: &[u64]) -> AppSurfaceLayoutProof {
        let geometries = window_ids
            .iter()
            .enumerate()
            .map(|(index, window_id)| {
                (
                    *window_id,
                    TargetGeometry {
                        x: Some(index as f64 * 120.0 - 40.0),
                        y: Some(20.0),
                        width: Some(100.0),
                        height: Some(80.0),
                    },
                )
            })
            .collect::<Vec<_>>();
        AppSurfaceLayoutProof::from_front_to_back_geometries(
            geometries
                .iter()
                .map(|(window_id, geometry)| (*window_id, geometry)),
        )
        .expect("test application layout")
    }

    fn interactive_application_binding() -> RemoteAppTargetBinding {
        ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Application,
                    live_metadata(json!({
                        "platform": "macos",
                        "bundle_id": "com.apple.Safari",
                        "app_identity": "com.apple.Safari",
                        "app_name": "Safari",
                        "primary_pid": 42,
                        "display_ids": [1, 2],
                        "resolved_window_ids": [7, 8],
                        "window_set_epoch": 99,
                        "target_identity_epoch": 99,
                        "union_x": 0,
                        "union_y": 0,
                        "union_width": 1600,
                        "union_height": 900,
                    })),
                ),
                "interactive",
                1,
            )
            .expect("multi-surface application identity must resolve")
    }

    fn interactive_application_binding_with_input_consent() -> RemoteAppTargetBinding {
        ResourceEntryTargetResolver
            .resolve_for_session_with_input_consent(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Application,
                    live_metadata(json!({
                        "platform": "macos",
                        "bundle_id": "com.apple.Safari",
                        "app_identity": "com.apple.Safari",
                        "app_name": "Safari",
                        "primary_pid": 42,
                        "display_ids": [1, 2],
                        "resolved_window_ids": [7, 8],
                        "window_set_epoch": 99,
                        "target_identity_epoch": 99,
                        "union_x": 0,
                        "union_y": 0,
                        "union_width": 1600,
                        "union_height": 900,
                    })),
                ),
                "interactive",
                1,
                true,
            )
            .expect("application identity must resolve with explicit input consent")
    }

    fn interactive_display_binding() -> RemoteAppTargetBinding {
        ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Display,
                    live_metadata(json!({
                        "display_id": 1,
                        "target_identity_epoch": 9,
                    })),
                ),
                "interactive",
                1,
            )
            .expect("display identity must resolve")
    }

    fn interactive_display_binding_with_input_consent() -> RemoteAppTargetBinding {
        ResourceEntryTargetResolver
            .resolve_for_session_with_input_consent(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Display,
                    live_metadata(json!({
                        "display_id": 1,
                        "target_identity_epoch": 9,
                    })),
                ),
                "interactive",
                1,
                true,
            )
            .expect("display identity must resolve with explicit input consent")
    }

    #[test]
    fn windows_xcap_application_binding_is_process_scoped_without_fake_display() {
        let binding = ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Application,
                    live_metadata(json!({
                        "platform": "windows",
                        "backend": "xcap",
                        "app_name": "Editor",
                        "primary_pid": 9001,
                        "resolved_window_ids": [11, 10],
                        "window_set_epoch": 77,
                        "target_identity_epoch": 77,
                    })),
                ),
                "interactive",
                1,
            )
            .expect("Windows xcap application identity must resolve without display widening");

        let projection = binding.to_value();
        assert_eq!(projection["platform"], json!("windows"));
        assert_eq!(projection["backend"], json!("xcap"));
        assert_eq!(
            projection["target_model"],
            json!("process_scoped_application_window_set")
        );
        assert_eq!(
            binding.scope_audit_value()["target_model"],
            json!("process_scoped_application_window_set")
        );
        assert_eq!(
            binding.latest_target_diagnostic_value()["target_model"],
            json!("process_scoped_application_window_set")
        );
        assert_eq!(
            binding.target_bound_event_payload()["target_model"],
            json!("process_scoped_application_window_set")
        );
        assert_eq!(
            projection["app_surface_layout"]["front_to_back_surfaces"]
                .as_array()
                .map(Vec::len),
            Some(2),
            "application binding must atomically own the inventory surface layout"
        );
        assert!(projection["app_surface_layout"]["layout_epoch"]
            .as_u64()
            .is_some_and(|epoch| epoch > 0));
        assert!(projection["native_locator"]["display_id"].is_null());
        assert_eq!(
            projection["app_window_set"]["resolved_window_ids"],
            json!([10, 11])
        );
        assert_eq!(projection["capture_scope"], json!("AppSurface"));
        assert!(!binding.scope_audit.display_fallback_used);
    }

    #[test]
    #[should_panic(expected = "remote_desktop.target.resolve_for_session")]
    fn resolver_refuses_to_run_while_session_store_lock_is_held() {
        let store = RemoteDesktopSessionStore::new();
        let _guard = store.lock();
        let resolver = ResourceEntryTargetResolver;

        let _ = resolver.resolve_for_session(
            "remote_desktop.create_session",
            &entry(ResourceType::Display, json!({"display_id": 7})),
            "view_only",
            1,
        );
    }

    #[test]
    fn every_target_resolution_reason_has_canonical_frontend_action_and_axon_context() {
        let mut reason_codes = std::collections::BTreeSet::new();
        for reason in ALL_TARGET_RESOLUTION_ERRORS {
            let reason_code = reason.as_str();
            assert!(
                reason_codes.insert(reason_code),
                "{reason_code} must be unique"
            );
            assert!(
                ALL_FRONTEND_ACTIONS.contains(&reason.frontend_action()),
                "{reason_code} must map to a declared frontend action"
            );

            let error = RemoteAppTargetError::new(
                "remote_desktop.create_session",
                *reason,
                "synthetic target failure",
            );
            let axon = error.to_axon();

            assert_eq!(
                axon.reason, reason_code,
                "{reason_code} must be the canonical Axon reason"
            );
            assert_eq!(
                axon.context.get("target_reason").map(String::as_str),
                Some(reason_code),
                "{reason_code} must be projected as target_reason context"
            );
            assert_eq!(
                axon.context.get("frontend_action").map(String::as_str),
                Some(reason.frontend_action().as_str()),
                "{reason_code} must carry its frontend recovery action"
            );
            assert_eq!(
                axon.context.get("target_event_type").map(String::as_str),
                reason.target_event_type(),
                "{reason_code} must project the matching SPEC target event type only when one exists"
            );
            assert!(
                !axon.message.is_empty() && axon.message.contains(reason_code),
                "{reason_code} must remain visible in the diagnostic message"
            );
        }

        assert_eq!(
            reason_codes.len(),
            ALL_TARGET_RESOLUTION_ERRORS.len(),
            "canonical target reason table must not contain aliases"
        );
    }

    #[test]
    fn native_app_identity_expectation_matches_canonical_bundle_aliases() {
        let expected = NativeAppIdentityExpectation {
            expected_pid: Some(42),
            expected_process_instance_id: None,
            expected_bundle_id: Some("com.example.Editor"),
            expected_app_identity: Some("com.example.Editor"),
        };

        let matched = expected.evaluate(NativeAppIdentityCandidate::new(
            Some(42),
            Some("com.example.Editor"),
            None,
        ));
        assert!(matched.matched());
        assert!(matched.any_expected_field_seen());

        let aliased = expected.evaluate(NativeAppIdentityCandidate::new(
            Some(42),
            None,
            Some("com.example.Editor"),
        ));
        assert!(
            aliased.matched(),
            "macOS bundle id and app identity may arrive through either platform projection field"
        );
        assert!(aliased.any_expected_field_seen());
    }

    #[test]
    fn native_app_identity_expectation_requires_all_declared_identity_fields() {
        let expected = NativeAppIdentityExpectation {
            expected_pid: Some(42),
            expected_process_instance_id: None,
            expected_bundle_id: Some("com.example.Editor"),
            expected_app_identity: Some("com.example.Editor"),
        };

        let pid_only = expected.evaluate(NativeAppIdentityCandidate::new(
            Some(42),
            Some("com.example.Other"),
            Some("com.example.Other"),
        ));
        assert!(!pid_only.matched());
        assert!(
            pid_only.any_expected_field_seen(),
            "selectors should classify partial identity sightings as mismatch instead of not_found"
        );

        let no_field_seen = expected.evaluate(NativeAppIdentityCandidate::new(
            Some(7),
            Some("com.example.Other"),
            Some("com.example.Other"),
        ));
        assert!(!no_field_seen.matched());
        assert!(!no_field_seen.any_expected_field_seen());
    }

    #[test]
    fn native_app_identity_expectation_rejects_reused_linux_pid() {
        let expected = NativeAppIdentityExpectation {
            expected_pid: Some(42),
            expected_process_instance_id: Some("linux:boot-a:42:100"),
            expected_bundle_id: None,
            expected_app_identity: None,
        };
        assert!(expected
            .evaluate(
                NativeAppIdentityCandidate::new(Some(42), None, None)
                    .with_process_instance_id(Some("linux:boot-a:42:100")),
            )
            .matched());
        assert!(!expected
            .evaluate(
                NativeAppIdentityCandidate::new(Some(42), None, None)
                    .with_process_instance_id(Some("linux:boot-a:42:200")),
            )
            .matched());
    }

    #[test]
    fn target_resolution_reasons_project_spec_event_taxonomy_for_create_session_failures() {
        let expected = [
            (TargetResolutionError::TargetNotFound, Some("TARGET_LOST")),
            (
                TargetResolutionError::TargetStale,
                Some("CAPTURE_TARGET_STALE"),
            ),
            (
                TargetResolutionError::TargetIdentityAmbiguous,
                Some("CAPTURE_TARGET_AMBIGUOUS"),
            ),
            (
                TargetResolutionError::TargetIdentityChanged,
                Some("CAPTURE_TARGET_IDENTITY_MISMATCH"),
            ),
            (
                TargetResolutionError::TargetIdentityMismatch,
                Some("CAPTURE_TARGET_IDENTITY_MISMATCH"),
            ),
            (
                TargetResolutionError::TargetPermissionMissing,
                Some("SCREEN_CAPTURE_PERMISSION_DENIED"),
            ),
            (TargetResolutionError::TargetHidden, Some("TARGET_HIDDEN")),
            (
                TargetResolutionError::TargetMinimized,
                Some("TARGET_MINIMIZED"),
            ),
            (
                TargetResolutionError::TargetDisplayUnavailable,
                Some("DISPLAY_TOPOLOGY_CHANGED"),
            ),
            (
                TargetResolutionError::DisplayFallbackForbidden,
                Some("DISPLAY_FALLBACK_FORBIDDEN"),
            ),
            (TargetResolutionError::UnsupportedCaptureScope, None),
            (TargetResolutionError::CaptureBackendUnavailable, None),
            (TargetResolutionError::TargetMetadataIncomplete, None),
            (TargetResolutionError::TargetMultiDisplayUnsupported, None),
            (TargetResolutionError::DisplayIdentityMissing, None),
            (TargetResolutionError::DisplayIdentityMismatch, None),
            (TargetResolutionError::InputScopeUnsupported, None),
            (TargetResolutionError::TransportRouteUnavailable, None),
            (
                TargetResolutionError::ScreenCaptureKitEnumerationFailed,
                None,
            ),
            (TargetResolutionError::ScreenCaptureKitFilterFailed, None),
            (
                TargetResolutionError::ScreenCaptureKitStreamStartFailed,
                None,
            ),
        ];

        assert_eq!(expected.len(), ALL_TARGET_RESOLUTION_ERRORS.len());
        for (reason, target_event_type) in expected {
            assert_eq!(reason.target_event_type(), target_event_type);
            let error = RemoteAppTargetError::new(
                "remote_desktop.create_session",
                reason,
                "synthetic target failure",
            )
            .to_axon();
            assert_eq!(
                error.context.get("target_event_type").map(String::as_str),
                target_event_type,
                "{} must project expected target_event_type",
                reason.as_str()
            );
        }
    }

    #[test]
    #[should_panic(expected = "remote_desktop.target.verify_target_binding_for_session")]
    fn live_target_proof_refuses_to_run_while_session_store_lock_is_held() {
        let resolver = ResourceEntryTargetResolver;
        let binding = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(ResourceType::Display, json!({"display_id": 7})),
                "view_only",
                1,
            )
            .expect("display target resolves");
        let store = RemoteDesktopSessionStore::new();
        let _guard = store.lock();

        let _ = verify_target_binding_for_session("remote_desktop.create_session", &binding);
    }

    #[test]
    fn display_requires_identity_or_explicit_primary_subject() {
        let resolver = ResourceEntryTargetResolver;
        let err = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(ResourceType::Display, json!({})),
                "view_only",
                1,
            )
            .unwrap_err();
        assert_eq!(err.reason(), TargetResolutionError::DisplayIdentityMissing);
        assert!(resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(ResourceType::Display, json!({"primary_display": true})),
                "view_only",
                1,
            )
            .is_ok());
    }

    #[test]
    fn target_binding_rejects_non_agent_owner_projection() {
        let resolver = ResourceEntryTargetResolver;
        for owner_agent in [
            "",
            "easynet:///r/acme/device/01DEV",
            "easynet:///r/acme/user/u-1",
            "easynet:///r/acme/service/u-1.pages",
            "easynet:///r/acme/resource/device.01DEV/streams/display.test",
        ] {
            let mut target = entry(ResourceType::Display, json!({"display_id": 7}));
            target.owner_agent = owner_agent.to_string();
            let err = resolver
                .resolve_for_session("remote_desktop.create_session", &target, "view_only", 1)
                .expect_err("non-Agent owner_agent projection must fail before target binding");

            assert_eq!(
                err.reason(),
                TargetResolutionError::TargetMetadataIncomplete
            );
            assert!(
                err.to_string()
                    .contains("owner_agent must be an Agent/SystemAgent URA"),
                "unexpected owner projection error for {owner_agent:?}: {err}"
            );
        }

        let mut target = entry(ResourceType::Display, json!({"display_id": 7}));
        target.owner_agent = "easynet:///r/acme/agent/device.01DEV.remote-desktop".to_string();
        resolver
            .resolve_for_session("remote_desktop.create_session", &target, "view_only", 1)
            .expect("Agent/SystemAgent owner projection must remain admissible");
    }

    #[test]
    fn window_requires_stable_owner_identity_not_app_name_only() {
        let resolver = ResourceEntryTargetResolver;
        let err = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Window,
                    live_metadata(json!({
                        "window_id": 7,
                        "app_name": "Terminal",
                        "title": "same-looking shell",
                    })),
                ),
                "view_only",
                1,
            )
            .unwrap_err();
        assert_eq!(err.reason(), TargetResolutionError::TargetIdentityAmbiguous);
        assert!(
            err.to_string().contains("app_name/title are diagnostic"),
            "unexpected error: {err}"
        );

        let binding = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Window,
                    live_metadata(json!({
                        "window_id": 7,
                        "pid": 4242,
                        "app_name": "Terminal",
                        "title": "same-looking shell",
                    })),
                ),
                "view_only",
                1,
            )
            .expect("window_id plus pid is stable enough for session binding");
        let projection = binding.to_value();
        assert_eq!(projection["target_kind"], json!("window"));
        assert_eq!(projection["native_locator"]["display_id"], Value::Null);
        assert!(
            binding.supports_native_adapter(),
            "exact window capture is desktop-independent and must not require display identity"
        );
        assert_eq!(projection["native_locator"]["window_id"], json!(7));
        assert_eq!(projection["native_locator"]["pid"], json!(4242));
        assert_eq!(projection["native_locator"]["app_name"], json!("Terminal"));
        assert_eq!(
            projection["native_locator"]["title"],
            json!("same-looking shell")
        );
    }

    #[test]
    fn resolver_rejects_unavailable_or_unproven_inventory_rows_before_binding() {
        let resolver = ResourceEntryTargetResolver;
        let err = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Window,
                    json!({
                        "availability": "unavailable",
                        "stale_reason": "target_not_found",
                        "window_id": 7,
                        "pid": 4242,
                    }),
                ),
                "view_only",
                1,
            )
            .expect_err("unavailable live inventory rows must fail closed");
        assert_eq!(err.reason(), TargetResolutionError::TargetNotFound);
        assert!(
            err.to_string().contains("frontend_action=refresh_targets"),
            "unexpected error: {err}"
        );

        let err = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Window,
                    json!({
                        "window_id": 7,
                        "pid": 4242,
                    }),
                ),
                "view_only",
                1,
            )
            .expect_err("app/window rows without live freshness must fail closed");
        assert_eq!(err.reason(), TargetResolutionError::TargetStale);
        assert!(
            err.to_string().contains("missing live inventory freshness")
                || err
                    .to_string()
                    .contains("missing live inventory availability"),
            "unexpected error: {err}"
        );

        let mut inconsistent_layout = live_metadata(json!({
            "platform": "windows",
            "backend": "xcap",
            "primary_pid": 9001,
            "resolved_window_ids": [10, 11],
            "window_set_epoch": 77,
        }));
        inconsistent_layout["surface_layout_epoch"] = json!(1);
        let err = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(ResourceType::Application, inconsistent_layout),
                "view_only",
                1,
            )
            .expect_err("application layout epoch drift must fail before session creation");
        assert_eq!(
            err.reason(),
            TargetResolutionError::TargetMetadataIncomplete
        );
        assert!(err.to_string().contains("surface_layout_epoch"));

        let mut expired = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Window,
                    json!({
                        "availability": "available",
                        "freshness": {
                            "observed_at_ms": 1,
                            "stale_after_ms": 1,
                            "source": "live_refresh",
                        },
                        "window_id": 7,
                        "pid": 4242,
                    }),
                ),
                "view_only",
                1,
            )
            .expect("expired picker cache must defer authority to live target verification");
        assert_eq!(
            expired.latest_target_diagnostic_value()
                ["inventory_cache_expired_before_live_verification"],
            json!(true)
        );
        assert_eq!(
            expired.latest_target_diagnostic_value()["live_identity_reverified"],
            json!(false)
        );
        let locator = expired.native_locator();
        let proof = ResolvedCaptureTargetProof::new(
            locator.capture_backend(),
            RemoteDesktopTargetKind::Window,
        )
        .with_native_identity(
            locator.display_id(),
            locator.window_id(),
            locator.pid(),
            locator.app_identity().map(ToOwned::to_owned),
            locator.bundle_id().map(ToOwned::to_owned),
        )
        .with_native_dimensions(Some((800, 600)));
        expired
            .commit_capture_proof("remote_desktop.create_session", proof)
            .expect("matching live proof commits after picker cache expiry");
        assert_eq!(
            expired.latest_target_diagnostic_value()["live_identity_reverified"],
            json!(true)
        );
    }

    #[cfg(not(all(target_os = "macos", feature = "native-media")))]
    #[test]
    fn non_native_target_proof_fails_closed_for_app_and_window_targets() {
        let resolver = ResourceEntryTargetResolver;
        let display = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(ResourceType::Display, json!({"display_id": 7})),
                "view_only",
                1,
            )
            .expect("display binding");
        let proof = verify_target_binding_for_session("remote_desktop.create_session", &display)
            .expect("display binding remains supported by headless/display providers");
        assert_eq!(proof.to_value()["display_id"], json!(7));

        let window = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Window,
                    live_metadata(json!({
                        "window_id": 7,
                        "pid": 4242,
                        "app_name": "Terminal",
                    })),
                ),
                "view_only",
                1,
            )
            .expect("window binding metadata resolves before live proof");
        let err = verify_target_binding_for_session("remote_desktop.create_session", &window)
            .expect_err("window proof must fail closed without native platform capture");
        assert_eq!(
            err.reason(),
            TargetResolutionError::CaptureBackendUnavailable
        );

        let application_window_set =
            AppWindowSetProof::new(1, Some("com.apple.Safari".to_string()), Some(42), vec![70]);
        let application = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Application,
                    live_metadata(json!({
                        "display_id": 1,
                        "bundle_id": "com.apple.Safari",
                        "app_identity": "com.apple.Safari",
                        "primary_pid": 42,
                        "resolved_window_ids": [70],
                        "window_set_epoch": application_window_set.window_set_epoch,
                    })),
                ),
                "view_only",
                1,
            )
            .expect("application binding metadata resolves before live proof");
        let err = verify_target_binding_for_session("remote_desktop.create_session", &application)
            .expect_err("application proof must fail closed without native platform capture");
        assert_eq!(
            err.reason(),
            TargetResolutionError::CaptureBackendUnavailable
        );
    }

    #[test]
    fn capture_proof_is_committed_into_session_binding_and_revalidated_exactly() {
        let resolver = ResourceEntryTargetResolver;
        let mut binding = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Window,
                    live_metadata(json!({
                        "window_id": 7,
                        "pid": 4242,
                        "bundle_id": "com.apple.Terminal",
                    })),
                ),
                "view_only",
                1,
            )
            .expect("window binding");
        assert!(binding.to_value()["capture_proof"].is_null());

        let wrong_window = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Window,
        )
        .with_native_identity(
            None,
            Some(8),
            Some(4242),
            Some("com.apple.Terminal".to_string()),
            Some("com.apple.Terminal".to_string()),
        )
        .with_native_dimensions(Some((1280, 720)));
        let err = binding
            .commit_capture_proof("remote_desktop.create_session", wrong_window)
            .expect_err("proof must match the binding identity before it is stored");
        assert_eq!(err.reason(), TargetResolutionError::TargetIdentityMismatch);

        let proof = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Window,
        )
        .with_native_identity(
            None,
            Some(7),
            Some(4242),
            Some("com.apple.Terminal".to_string()),
            Some("com.apple.Terminal".to_string()),
        )
        .with_native_dimensions(Some((1280, 720)));
        binding
            .commit_capture_proof("remote_desktop.create_session", proof.clone())
            .expect("matching proof commits");
        assert_eq!(binding.to_value()["capture_proof"]["window_id"], json!(7));
        binding
            .validate_reverified_capture_proof("remote_desktop.set_description", &proof)
            .expect("same live proof remains valid");

        let drifted_pid = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Window,
        )
        .with_native_identity(
            None,
            Some(7),
            Some(5150),
            Some("com.apple.Terminal".to_string()),
            Some("com.apple.Terminal".to_string()),
        )
        .with_native_dimensions(Some((1280, 720)));
        let err = binding
            .validate_reverified_capture_proof("remote_desktop.set_description", &drifted_pid)
            .expect_err("media path must fail if live target drifts from committed proof");
        assert_eq!(err.reason(), TargetResolutionError::TargetIdentityMismatch);
    }

    #[test]
    fn capture_proof_reverification_refreshes_dimensions_and_timestamp() {
        let mut proof =
            ResolvedCaptureTargetProof::new("xcap", RemoteDesktopTargetKind::Application)
                .with_native_dimensions(Some((1020, 380)));
        proof.verified_at_ms = 1;
        let proof = proof.reverified_with_native_dimensions(Some((840, 500)));
        assert_eq!(proof.native_width, Some(840));
        assert_eq!(proof.native_height, Some(500));
        assert!(proof.verified_at_ms > 1);
    }

    #[test]
    fn capture_proof_revalidation_uses_native_app_identity_aliases() {
        let resolver = ResourceEntryTargetResolver;
        let mut binding = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Window,
                    live_metadata(json!({
                        "window_id": 7,
                        "pid": 4242,
                        "bundle_id": "com.example.Editor",
                    })),
                ),
                "view_only",
                1,
            )
            .expect("window binding");

        let committed_from_app_identity = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Window,
        )
        .with_native_identity(
            None,
            Some(7),
            Some(4242),
            Some("com.example.Editor".to_string()),
            None,
        )
        .with_native_dimensions(Some((1280, 720)));
        binding
            .commit_capture_proof("remote_desktop.create_session", committed_from_app_identity)
            .expect("proof may project bundle identity through app_identity");

        let reverified_from_bundle_id = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Window,
        )
        .with_native_identity(
            None,
            Some(7),
            Some(4242),
            None,
            Some("com.example.Editor".to_string()),
        )
        .with_native_dimensions(Some((1280, 720)));
        binding
            .validate_reverified_capture_proof(
                "remote_desktop.set_description",
                &reverified_from_bundle_id,
            )
            .expect("same native app identity may arrive through bundle_id on reverify");

        let mismatched_identity = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Window,
        )
        .with_native_identity(
            None,
            Some(7),
            Some(4242),
            None,
            Some("com.example.Other".to_string()),
        )
        .with_native_dimensions(Some((1280, 720)));
        let err = binding
            .validate_reverified_capture_proof(
                "remote_desktop.set_description",
                &mismatched_identity,
            )
            .expect_err("different native app identity must still fail closed");
        assert_eq!(err.reason(), TargetResolutionError::TargetIdentityMismatch);
    }

    #[test]
    fn application_capture_proof_requires_exact_window_set_and_surface_layout() {
        let resolver = ResourceEntryTargetResolver;
        let expected_window_set = AppWindowSetProof::new(
            42,
            Some("com.example.Editor".to_string()),
            Some(9001),
            vec![11, 10, 10],
        );
        let mut binding = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Application,
                    live_metadata(json!({
                        "display_id": 42,
                        "bundle_id": "com.example.Editor",
                        "app_identity": "com.example.Editor",
                        "primary_pid": 9001,
                        "resolved_window_ids": [10, 11],
                        "window_set_epoch": expected_window_set.window_set_epoch,
                        "target_identity_epoch": expected_window_set.window_set_epoch,
                    })),
                ),
                "view_only",
                1,
            )
            .expect("application binding");

        let proof = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Application,
        )
        .with_native_identity(
            Some(42),
            None,
            Some(9001),
            Some("com.example.Editor".to_string()),
            Some("com.example.Editor".to_string()),
        )
        .with_native_dimensions(Some((1440, 900)))
        .with_app_window_set(expected_window_set)
        .with_app_surface_layout(application_layout(&[10, 11]));
        binding
            .commit_capture_proof("remote_desktop.create_session", proof.clone())
            .expect("matching app window set proof commits");
        assert_eq!(
            binding.to_value()["capture_proof"]["app_window_set"]["resolved_window_ids"],
            json!([10, 11])
        );

        let drifted_window_set = AppWindowSetProof::new(
            42,
            Some("com.example.Editor".to_string()),
            Some(9001),
            vec![10, 12],
        );
        let drifted_proof = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Application,
        )
        .with_native_identity(
            Some(42),
            None,
            Some(9001),
            Some("com.example.Editor".to_string()),
            Some("com.example.Editor".to_string()),
        )
        .with_native_dimensions(Some((1440, 900)))
        .with_app_window_set(drifted_window_set)
        .with_app_surface_layout(application_layout(&[10, 12]));
        let err = binding
            .validate_reverified_capture_proof("remote_desktop.set_description", &drifted_proof)
            .expect_err("application media proof must fail when the live window set drifts");
        assert_eq!(err.reason(), TargetResolutionError::TargetIdentityChanged);
    }

    #[test]
    fn pending_application_rebind_accepts_provider_layout_for_exact_window_set() {
        let resolver = ResourceEntryTargetResolver;
        let expected_window_set = AppWindowSetProof::new(
            42,
            Some("com.example.Editor".to_string()),
            Some(9001),
            vec![10, 11],
        );
        let binding = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Application,
                    live_metadata(json!({
                        "display_id": 42,
                        "bundle_id": "com.example.Editor",
                        "app_identity": "com.example.Editor",
                        "primary_pid": 9001,
                        "resolved_window_ids": [10, 11],
                        "window_set_epoch": expected_window_set.window_set_epoch,
                        "target_identity_epoch": expected_window_set.window_set_epoch,
                    })),
                ),
                "view_only",
                1,
            )
            .expect("application binding");

        let provider_geometries = [
            (
                10,
                TargetGeometry {
                    x: Some(0.0),
                    y: Some(0.0),
                    width: Some(800.0),
                    height: Some(600.0),
                },
            ),
            (
                11,
                TargetGeometry {
                    x: Some(120.0),
                    y: Some(80.0),
                    width: Some(640.0),
                    height: Some(480.0),
                },
            ),
        ];
        let provider_layout = AppSurfaceLayoutProof::from_front_to_back_geometries(
            provider_geometries
                .iter()
                .map(|(window_id, geometry)| (*window_id, geometry)),
        )
        .expect("provider layout");
        let proof = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Application,
        )
        .with_native_identity(
            Some(42),
            None,
            Some(9001),
            Some("com.example.Editor".to_string()),
            Some("com.example.Editor".to_string()),
        )
        .with_native_dimensions(Some((1440, 900)))
        .with_app_window_set(expected_window_set.clone())
        .with_app_surface_layout(provider_layout);

        binding
            .validate_pending_media_rebind_capture_proof("remote_desktop.set_description", &proof)
            .expect("provider layout is authoritative while exact window identity remains closed");

        let wrong_window_set = proof.clone().with_app_window_set(AppWindowSetProof::new(
            42,
            Some("com.example.Editor".to_string()),
            Some(9001),
            vec![10, 12],
        ));
        let err = binding
            .validate_pending_media_rebind_capture_proof(
                "remote_desktop.set_description",
                &wrong_window_set,
            )
            .expect_err("different window ids remain identity drift");
        assert_eq!(err.reason(), TargetResolutionError::TargetIdentityChanged);
    }

    #[test]
    fn application_initial_capture_commit_rebinds_to_current_live_window_set() {
        let resolver = ResourceEntryTargetResolver;
        let inventory_window_set = AppWindowSetProof::new(
            42,
            Some("com.example.Editor".to_string()),
            Some(9001),
            vec![10],
        );
        let mut binding = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Application,
                    live_metadata(json!({
                        "display_id": 42,
                        "bundle_id": "com.example.Editor",
                        "app_identity": "com.example.Editor",
                        "primary_pid": 9001,
                        "resolved_window_ids": [10],
                        "window_set_epoch": inventory_window_set.window_set_epoch,
                        "target_identity_epoch": inventory_window_set.window_set_epoch,
                    })),
                ),
                "view_only",
                1,
            )
            .expect("application binding");

        let live_window_set = AppWindowSetProof::new(
            42,
            Some("com.example.Editor".to_string()),
            Some(9001),
            vec![10, 11],
        );
        assert_ne!(
            inventory_window_set.window_set_epoch,
            live_window_set.window_set_epoch
        );
        let live_proof = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Application,
        )
        .with_native_identity(
            Some(42),
            None,
            Some(9001),
            Some("com.example.Editor".to_string()),
            Some("com.example.Editor".to_string()),
        )
        .with_native_dimensions(Some((1440, 900)))
        .with_app_window_set(live_window_set.clone())
        .with_app_surface_layout(application_layout(&[10, 11]));

        binding
            .commit_capture_proof("remote_desktop.create_session", live_proof.clone())
            .expect("initial create_session commit rebinds to current live app window set");
        assert_eq!(
            binding
                .committed_app_window_set()
                .expect("committed app window set"),
            &live_window_set
        );
        assert_eq!(
            binding.to_value()["target_identity_epoch"],
            json!(live_window_set.window_set_epoch)
        );
        assert_eq!(
            binding.to_value()["bounds"],
            json!({"x": -40.0, "y": 20.0, "width": 220.0, "height": 80.0})
        );
        assert_eq!(
            binding.diagnostic_capture_subject.metadata["surface_layout_epoch"],
            binding.to_value()["app_surface_layout"]["layout_epoch"]
        );
        binding
            .validate_reverified_capture_proof("remote_desktop.set_description", &live_proof)
            .expect("same live proof remains valid after initial commit");

        let drifted_after_commit = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Application,
        )
        .with_native_identity(
            Some(42),
            None,
            Some(9001),
            Some("com.example.Editor".to_string()),
            Some("com.example.Editor".to_string()),
        )
        .with_native_dimensions(Some((1440, 900)))
        .with_app_window_set(AppWindowSetProof::new(
            42,
            Some("com.example.Editor".to_string()),
            Some(9001),
            vec![10, 12],
        ))
        .with_app_surface_layout(application_layout(&[10, 12]));
        let err = binding
            .validate_reverified_capture_proof(
                "remote_desktop.set_description",
                &drifted_after_commit,
            )
            .expect_err("post-commit app window-set drift must fail closed");
        assert_eq!(err.reason(), TargetResolutionError::TargetIdentityChanged);
    }

    #[test]
    fn application_requires_stable_identity_and_exact_window_set() {
        let resolver = ResourceEntryTargetResolver;
        let err = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Application,
                    live_metadata(json!({"app_name": "Safari"})),
                ),
                "view_only",
                1,
            )
            .unwrap_err();
        assert_eq!(err.reason(), TargetResolutionError::TargetIdentityAmbiguous);
        let err = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Application,
                    live_metadata(json!({
                        "platform": "macos",
                        "bundle_id": "com.apple.Safari",
                        "app_identity": "com.apple.Safari",
                        "primary_pid": 42,
                    })),
                ),
                "view_only",
                1,
            )
            .unwrap_err();
        assert_eq!(
            err.reason(),
            TargetResolutionError::TargetMetadataIncomplete
        );
        let binding = interactive_application_binding();
        let projection = binding.to_value();
        assert_eq!(projection["target_kind"], json!("application"));
        assert_eq!(
            projection["target_model"],
            json!("multi_surface_application_window_set")
        );
        assert_eq!(projection["capture_scope"], json!("AppSurface"));
        assert_eq!(projection["input_scope"], json!("view_only"));
        assert_eq!(
            projection["input_scope_reason"],
            json!("input_consent_required")
        );
        assert_eq!(
            projection["resolved_identity"]["bundle_id"],
            json!("com.apple.Safari")
        );
        assert_eq!(projection["resolved_identity"]["display_id"], Value::Null);
        assert_eq!(projection["app_window_set"]["display_ids"], json!([1, 2]));
        assert_eq!(
            binding.scope_audit_value()["target_model"],
            json!("multi_surface_application_window_set")
        );
        assert_eq!(
            binding.scope_audit_value()["input_scope_reason"],
            json!("input_consent_required")
        );
        assert_eq!(
            binding.latest_target_diagnostic_value()["target_model"],
            json!("multi_surface_application_window_set")
        );
        assert_eq!(
            binding.target_bound_event_payload()["target_model"],
            json!("multi_surface_application_window_set")
        );
        assert_eq!(
            binding.target_bound_event_payload()["consent_epoch"],
            json!(binding.consent_epoch())
        );
        assert_eq!(
            binding.target_bound_event_payload()["input_scope_reason"],
            json!("input_consent_required")
        );
    }

    #[test]
    fn application_recovery_accepts_unknown_platform_display_and_rejects_contradictions() {
        let platform_unknown = AppWindowSetProof::from_recovery_value(&json!({
            "display_id": null,
            "display_ids": [],
            "bundle_id": "com.example.Editor",
            "primary_pid": 42,
            "resolved_window_ids": [70],
            "window_set_epoch": 1
        }))
        .expect("Windows/Linux xcap application sets do not expose display topology");
        assert_eq!(platform_unknown.to_value()["display_ids"], json!([]));

        let contradictory = AppWindowSetProof::from_recovery_value(&json!({
            "display_id": 2,
            "display_ids": [],
            "bundle_id": "com.example.Editor",
            "primary_pid": 42,
            "resolved_window_ids": [70],
            "window_set_epoch": 1
        }))
        .expect_err("a known primary display must belong to the display topology");
        assert!(
            contradictory
                .to_string()
                .contains("must contain display_id"),
            "{contradictory}"
        );

        let noncanonical = AppWindowSetProof::from_recovery_value(&json!({
            "display_id": null,
            "display_ids": [2, 1, 2],
            "bundle_id": "com.example.Editor",
            "primary_pid": 42,
            "resolved_window_ids": [70],
            "window_set_epoch": 1
        }))
        .expect_err("noncanonical application display topology must fail closed");
        assert!(
            noncanonical
                .to_string()
                .contains("must be sorted and unique"),
            "{noncanonical}"
        );
    }

    #[test]
    fn application_interactive_with_input_consent_projects_guarded_target_scope() {
        let binding = interactive_application_binding_with_input_consent();
        assert_eq!(binding.to_value()["input_scope"], json!("target_local"));
        assert_eq!(
            binding.to_value()["input_scope_reason"],
            json!("target_scoped_input_guarded")
        );
        assert_eq!(
            binding.scope_audit_value()["input_scope_reason"],
            json!("target_scoped_input_guarded")
        );
        assert_eq!(
            binding.target_bound_event_payload()["input_scope_reason"],
            json!("target_scoped_input_guarded")
        );
    }

    #[test]
    fn supported_platform_guards_admit_window_and_application_target_local_input() {
        for target_guard in [
            TargetScopedInputIsolation::MacosAccessibilityCoreGraphics,
            TargetScopedInputIsolation::WindowsXcapUser32,
        ] {
            for target_kind in [
                RemoteDesktopTargetKind::Window,
                RemoteDesktopTargetKind::Application,
            ] {
                assert_eq!(
                    input_scope_for_request(target_kind, "interactive", true, target_guard),
                    InputScopeDecision::new(
                        InputScope::TargetLocal,
                        InputScopeReason::TargetScopedInputGuarded,
                    ),
                    "compiled exact-target guard {target_guard:?} must make {target_kind:?} input reachable",
                );
            }
        }
    }

    #[test]
    fn unsupported_platform_guard_keeps_target_local_input_fail_closed() {
        for target_kind in [
            RemoteDesktopTargetKind::Window,
            RemoteDesktopTargetKind::Application,
        ] {
            assert_eq!(
                input_scope_for_request(
                    target_kind,
                    "interactive",
                    true,
                    TargetScopedInputIsolation::Unsupported,
                ),
                InputScopeDecision::new(
                    InputScope::ViewOnly,
                    InputScopeReason::TargetScopedInputUnsafe,
                ),
            );
        }
    }

    #[test]
    fn linux_x11_window_and_application_input_remain_view_only_without_press_release_isolation() {
        for target_kind in [
            RemoteDesktopTargetKind::Window,
            RemoteDesktopTargetKind::Application,
        ] {
            assert_eq!(
                input_scope_for_request(
                    target_kind,
                    "interactive",
                    true,
                    TargetScopedInputIsolation::LinuxX11Unisolated,
                ),
                InputScopeDecision::new(
                    InputScope::ViewOnly,
                    InputScopeReason::TargetScopedInputUnsafe,
                ),
            );
        }
    }

    #[test]
    fn display_interactive_downgrades_until_input_consent_exists() {
        let binding = interactive_display_binding();
        assert_eq!(binding.to_value()["input_scope"], json!("view_only"));
        assert_eq!(
            binding.to_value()["input_scope_reason"],
            json!("input_consent_required")
        );
        assert_eq!(
            binding.scope_audit_value()["input_scope_reason"],
            json!("input_consent_required")
        );
        assert_eq!(
            binding.target_bound_event_payload()["input_scope_reason"],
            json!("input_consent_required")
        );
    }

    #[test]
    fn display_interactive_with_input_consent_projects_display_global_scope() {
        let binding = interactive_display_binding_with_input_consent();
        assert_eq!(binding.to_value()["input_scope"], json!("display_global"));
        assert_eq!(
            binding.to_value()["input_scope_reason"],
            json!("input_control_granted")
        );
        assert_eq!(
            binding.scope_audit_value()["input_scope_reason"],
            json!("input_control_granted")
        );
        assert_eq!(
            binding.target_bound_event_payload()["input_scope_reason"],
            json!("input_control_granted")
        );
    }
}
