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
// - Platform/native lookup belongs behind RemoteAppTargetResolver or explicit
//   Rebinding, never in transport handlers.

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};

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
}

const ALL_INPUT_SCOPES: &[InputScope] = &[
    InputScope::ViewOnly,
    InputScope::TargetLocal,
    InputScope::DisplayGlobal,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum FrontendAction {
    RefreshTargets,
    RequestPermission,
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

#[derive(Debug, Clone, PartialEq)]
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

    pub(in crate::daemon::plugins::remote_desktop) fn pointer_target_value(
        &self,
        binding: &RemoteAppTargetBinding,
    ) -> Option<Value> {
        let origin_x = self.x?;
        let origin_y = self.y?;
        Some(json!({
            "subject_type": binding.target_kind.as_str(),
            "binding_id": binding.binding_id,
            "binding_epoch": binding.binding_epoch,
            "target_identity_epoch": binding.target_identity_epoch,
            "target_geometry_revision": binding.target_geometry_revision,
            "origin_x": origin_x,
            "origin_y": origin_y,
            "width": self.width,
            "height": self.height,
        }))
    }

    fn to_value(&self) -> Value {
        json!({
            "x": self.x,
            "y": self.y,
            "width": self.width,
            "height": self.height,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetIdentity {
    pub(in crate::daemon::plugins::remote_desktop) hardware_id: String,
    pub(in crate::daemon::plugins::remote_desktop) display_id: Option<u64>,
    pub(in crate::daemon::plugins::remote_desktop) window_id: Option<u64>,
    pub(in crate::daemon::plugins::remote_desktop) pid: Option<i64>,
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
            "app_identity": self.app_identity,
            "bundle_id": self.bundle_id,
            "app_name": self.app_name,
            "title": self.title,
        })
    }
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
    app_identity: Option<String>,
    bundle_id: Option<String>,
    app_name: Option<String>,
    title: Option<String>,
}

impl NativeTargetLocator {
    pub(in crate::daemon::plugins::remote_desktop) fn primary_display(&self) -> bool {
        self.primary_display
    }

    pub(in crate::daemon::plugins::remote_desktop) fn display_id(&self) -> Option<u64> {
        self.display_id
    }

    pub(in crate::daemon::plugins::remote_desktop) fn window_id(&self) -> Option<u64> {
        self.window_id
    }

    pub(in crate::daemon::plugins::remote_desktop) fn pid(&self) -> Option<i64> {
        self.pid
    }

    pub(in crate::daemon::plugins::remote_desktop) fn app_name(&self) -> Option<&str> {
        self.app_name.as_deref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn app_identity(&self) -> Option<&str> {
        self.app_identity.as_deref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn bundle_id(&self) -> Option<&str> {
        self.bundle_id.as_deref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn title(&self) -> Option<&str> {
        self.title.as_deref()
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
            "app_identity": self.app_identity,
            "bundle_id": self.bundle_id,
            "app_name": self.app_name,
            "title": self.title,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct ScopeAudit {
    requested_target_kind: RemoteDesktopTargetKind,
    effective_target_kind: RemoteDesktopTargetKind,
    capture_scope: CaptureScope,
    input_scope: InputScope,
    scope_widened: bool,
    display_fallback_used: bool,
}

impl ScopeAudit {
    fn to_value(&self) -> Value {
        json!({
            "requested_target_kind": self.requested_target_kind.as_str(),
            "effective_target_kind": self.effective_target_kind.as_str(),
            "capture_surface": self.capture_scope.as_str(),
            "input_mode": self.input_scope.as_str(),
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
    geometry: TargetGeometry,
    scope_audit: ScopeAudit,
    diagnostic: Value,
    diagnostic_capture_subject: DiagnosticCaptureSubject,
}

impl RemoteAppTargetBinding {
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

    pub(in crate::daemon::plugins::remote_desktop) fn native_locator(
        &self,
    ) -> &NativeTargetLocator {
        &self.native_locator
    }

    pub(in crate::daemon::plugins::remote_desktop) fn geometry(&self) -> &TargetGeometry {
        &self.geometry
    }

    pub(in crate::daemon::plugins::remote_desktop) fn diagnostic_capture_subject(
        &self,
    ) -> &DiagnosticCaptureSubject {
        &self.diagnostic_capture_subject
    }

    pub(in crate::daemon::plugins::remote_desktop) fn supports_xcap_adapter(&self) -> bool {
        match self.target_kind {
            RemoteDesktopTargetKind::Display => self.native_locator.capture_backend == "xcap",
            RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
                matches!(
                    self.native_locator.discovery_backend.as_str(),
                    "xcap" | "macos_core_graphics"
                ) && self.target_metadata_resolvable()
            }
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn supports_native_adapter(&self) -> bool {
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
                        || self.native_locator.bundle_id.is_some()
                        || self.native_locator.app_name.is_some())
            }
            RemoteDesktopTargetKind::Display => true,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "subject_ura": self.subject_ura,
            "target_kind": self.target_kind.as_str(),
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
            "native_locator": self.native_locator.to_value(),
            "resolved_identity": self.resolved_identity.to_value(),
            "bounds": self.geometry.to_value(),
            "production_ready": !self.scope_audit.scope_widened
                && !self.scope_audit.display_fallback_used,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn scope_audit_value(&self) -> Value {
        self.scope_audit.to_value()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn latest_target_diagnostic_value(
        &self,
    ) -> Value {
        self.diagnostic.clone()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_bound_event_payload(&self) -> Value {
        json!({
            "subject_ura": self.subject_ura,
            "binding_id": self.binding_id,
            "binding_epoch": self.binding_epoch,
            "target_identity_epoch": self.target_identity_epoch,
            "target_geometry_revision": self.target_geometry_revision,
            "media_source_epoch": self.media_source_epoch,
            "capture_scope": self.capture_scope.as_str(),
            "input_scope": self.input_scope.as_str(),
            "reason_code": "target_bound",
            "recoverability": "continue",
            "display_fallback_used": false,
        })
    }
}

pub(in crate::daemon::plugins::remote_desktop) trait RemoteAppTargetResolver {
    fn resolve_for_session(
        &self,
        ability: &'static str,
        entry: &ResourceEntry,
        requested_mode: &str,
        consent_epoch: u64,
    ) -> Result<RemoteAppTargetBinding, RemoteAppTargetError>;
}

pub(in crate::daemon::plugins::remote_desktop) fn verify_target_binding_for_session(
    ability: &'static str,
    binding: &RemoteAppTargetBinding,
) -> Result<(), RemoteAppTargetError> {
    platform_live_resolution::verify_target_binding_for_session(ability, binding)
}

#[cfg(all(target_os = "macos", feature = "native-media"))]
mod platform_live_resolution {
    use super::{RemoteAppTargetBinding, RemoteAppTargetError};

    pub(super) fn verify_target_binding_for_session(
        ability: &'static str,
        binding: &RemoteAppTargetBinding,
    ) -> Result<(), RemoteAppTargetError> {
        crate::daemon::plugins::remote_desktop::screencapturekit_capture::verify_target_binding_for_session(
            ability, binding,
        )
    }
}

#[cfg(not(all(target_os = "macos", feature = "native-media")))]
mod platform_live_resolution {
    use super::{RemoteAppTargetBinding, RemoteAppTargetError};

    pub(super) fn verify_target_binding_for_session(
        _ability: &'static str,
        _binding: &RemoteAppTargetBinding,
    ) -> Result<(), RemoteAppTargetError> {
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct ResourceEntryTargetResolver;

impl RemoteAppTargetResolver for ResourceEntryTargetResolver {
    fn resolve_for_session(
        &self,
        ability: &'static str,
        entry: &ResourceEntry,
        requested_mode: &str,
        consent_epoch: u64,
    ) -> Result<RemoteAppTargetBinding, RemoteAppTargetError> {
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
        let target_kind = RemoteDesktopTargetKind::try_from(entry.kind).map_err(|error| {
            RemoteAppTargetError::new(ability, error.reason(), error.to_string())
        })?;
        let capture_scope = capture_scope_for_kind(target_kind);
        let input_scope = input_scope_for_request(target_kind, requested_mode);
        let display_id = display_id(entry);
        validate_required_identity(ability, entry, target_kind, display_id)?;
        let platform = metadata_string(entry, "platform").unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "macos".to_string()
            } else {
                "unknown".to_string()
            }
        });
        let discovery_backend =
            metadata_string(entry, "backend").unwrap_or_else(|| "resource_registry".to_string());
        let capture_backend = capture_backend_for_entry(&platform, entry, target_kind);
        let geometry = match target_kind {
            RemoteDesktopTargetKind::Application => {
                TargetGeometry::from_metadata(entry, Some("primary"))
            }
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
            app_identity: metadata_string(entry, "app_identity"),
            bundle_id: metadata_string(entry, "bundle_id"),
            app_name: metadata_string(entry, "app_name"),
            title: metadata_string(entry, "title"),
        };
        let resolved_identity = TargetIdentity::from_entry(entry, display_id);
        let binding_id = mint_binding_id(entry, &native_locator);
        let target_identity_epoch = metadata_u64(entry, "lifecycle_epoch")
            .or_else(|| metadata_u64(entry, "target_identity_epoch"))
            .unwrap_or(1);
        let target_geometry_revision = metadata_u64(entry, "geometry_revision").unwrap_or(1);
        let media_source_epoch = 1;
        let scope_audit = ScopeAudit {
            requested_target_kind: target_kind,
            effective_target_kind: target_kind,
            capture_scope,
            input_scope,
            scope_widened: false,
            display_fallback_used: false,
        };
        let diagnostic = json!({
            "status": "resolved",
            "reason": Value::Null,
            "requested_identity": requested_identity_projection(entry),
            "resolved_identity": resolved_identity.to_value(),
            "match_strategy": match_strategy_for_kind(target_kind),
            "capture_backend": capture_backend,
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
            geometry,
            scope_audit,
            diagnostic,
            diagnostic_capture_subject: DiagnosticCaptureSubject::from_entry(entry),
        })
    }
}

fn validate_required_identity(
    ability: &'static str,
    entry: &ResourceEntry,
    target_kind: RemoteDesktopTargetKind,
    display_id: Option<u64>,
) -> Result<(), RemoteAppTargetError> {
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
                && metadata_string(entry, "app_name").is_none()
            {
                return Err(RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::TargetMetadataIncomplete,
                    "window targets require owner pid, app_identity, bundle_id, or app_name in addition to window_id",
                ));
            }
            Ok(())
        }
        RemoteDesktopTargetKind::Application => {
            if display_id.is_none() {
                return Err(RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::DisplayIdentityMissing,
                    "application targets require display_id/monitor_id because macOS application capture is display-scoped",
                ));
            }
            if metadata_i64(entry, "primary_pid").is_none()
                && metadata_string(entry, "app_identity").is_none()
                && metadata_string(entry, "bundle_id").is_none()
            {
                return Err(RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::TargetMetadataIncomplete,
                    "application targets require primary_pid, app_identity, or bundle_id; app_name alone is not production routing identity",
                ));
            }
            Ok(())
        }
    }
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
) -> InputScope {
    if requested_mode != "interactive" {
        return InputScope::ViewOnly;
    }
    match target_kind {
        RemoteDesktopTargetKind::Display => InputScope::DisplayGlobal,
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
            // macOS target-scoped keyboard/pointer dispatch is unsafe until the
            // focus/activation validator is implemented. The session can still
            // capture app/window surfaces in view-only mode.
            InputScope::ViewOnly
        }
    }
}

fn capture_backend_for_entry(
    platform: &str,
    entry: &ResourceEntry,
    target_kind: RemoteDesktopTargetKind,
) -> String {
    if cfg!(target_os = "macos")
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

fn match_strategy_for_kind(target_kind: RemoteDesktopTargetKind) -> &'static str {
    match target_kind {
        RemoteDesktopTargetKind::Display => "display_id_or_explicit_primary",
        RemoteDesktopTargetKind::Window => "window_id_plus_owner",
        RemoteDesktopTargetKind::Application => "display_scoped_app_identity",
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

fn mint_binding_id(entry: &ResourceEntry, locator: &NativeTargetLocator) -> String {
    let mut hasher = DefaultHasher::new();
    entry.resource_ura.hash(&mut hasher);
    entry.hardware_id.hash(&mut hasher);
    locator.display_id.hash(&mut hasher);
    locator.window_id.hash(&mut hasher);
    locator.pid.hash(&mut hasher);
    locator.app_identity.hash(&mut hasher);
    locator.bundle_id.hash(&mut hasher);
    locator.app_name.hash(&mut hasher);
    locator.title.hash(&mut hasher);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("tb_{now:x}_{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn application_requires_display_scoped_stable_identity() {
        let resolver = ResourceEntryTargetResolver;
        let err = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(ResourceType::Application, json!({"app_name": "Safari"})),
                "view_only",
                1,
            )
            .unwrap_err();
        assert_eq!(err.reason(), TargetResolutionError::DisplayIdentityMissing);
        let err = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Application,
                    json!({"display_id": 1, "app_name": "Safari"}),
                ),
                "view_only",
                1,
            )
            .unwrap_err();
        assert_eq!(
            err.reason(),
            TargetResolutionError::TargetMetadataIncomplete
        );
    }
}
