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

use axon_sdk::invocation::{AxonError, AxonErrorKind, ErrorCode, ErrorStage, SecurityClass};
use serde_json::{json, Value};

use crate::core::ura::{parse_ura, URAKind};
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

    pub(in crate::daemon::plugins::remote_desktop) fn target_model(self) -> &'static str {
        match self {
            Self::Display => "display_surface",
            Self::Window => "window_surface",
            Self::Application => "display_scoped_application_window_set",
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
enum InputScopeReason {
    RequestedViewOnly,
    InputConsentRequired,
    TargetScopedInputUnsafe,
}

impl InputScopeReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::RequestedViewOnly => "requested_view_only",
            Self::InputConsentRequired => "input_consent_required",
            Self::TargetScopedInputUnsafe => "target_scoped_keyboard_pointer_dispatch_unsafe",
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

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
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
pub(in crate::daemon::plugins::remote_desktop) struct ResolvedCaptureTargetProof {
    backend: String,
    target_kind: RemoteDesktopTargetKind,
    display_id: Option<u64>,
    window_id: Option<u64>,
    pid: Option<i64>,
    app_identity: Option<String>,
    bundle_id: Option<String>,
    app_window_set: Option<AppWindowSetProof>,
    native_width: Option<usize>,
    native_height: Option<usize>,
    verified_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct AppWindowSetProof {
    display_id: u64,
    bundle_id: Option<String>,
    primary_pid: Option<i64>,
    resolved_window_ids: Vec<u64>,
    window_set_epoch: u64,
}

impl AppWindowSetProof {
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        display_id: u64,
        bundle_id: Option<String>,
        primary_pid: Option<i64>,
        resolved_window_ids: Vec<u64>,
    ) -> Self {
        let mut resolved_window_ids = resolved_window_ids;
        resolved_window_ids.sort_unstable();
        resolved_window_ids.dedup();
        let window_set_epoch = compute_window_set_epoch(
            Some(display_id),
            bundle_id.as_deref(),
            primary_pid,
            &resolved_window_ids,
        );
        Self {
            display_id,
            bundle_id,
            primary_pid,
            resolved_window_ids,
            window_set_epoch,
        }
    }

    fn from_entry(entry: &ResourceEntry, display_id: Option<u64>) -> Option<Self> {
        let display_id = display_id?;
        let resolved_window_ids = metadata_u64_array(entry, "resolved_window_ids");
        if resolved_window_ids.is_empty() {
            return None;
        }
        let bundle_id = metadata_string(entry, "bundle_id");
        let primary_pid = metadata_i64(entry, "primary_pid").or_else(|| metadata_i64(entry, "pid"));
        let window_set_epoch = metadata_u64(entry, "window_set_epoch").unwrap_or_else(|| {
            compute_window_set_epoch(
                Some(display_id),
                bundle_id.as_deref(),
                primary_pid,
                &resolved_window_ids,
            )
        });
        Some(Self {
            display_id,
            bundle_id,
            primary_pid,
            resolved_window_ids,
            window_set_epoch,
        })
    }

    fn to_value(&self) -> Value {
        json!({
            "display_id": self.display_id,
            "bundle_id": self.bundle_id,
            "primary_pid": self.primary_pid,
            "resolved_window_ids": self.resolved_window_ids,
            "window_set_epoch": self.window_set_epoch,
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn contains_window_id(
        &self,
        window_id: u64,
    ) -> bool {
        self.resolved_window_ids.binary_search(&window_id).is_ok()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn resolved_window_count(&self) -> usize {
        self.resolved_window_ids.len()
    }

    fn diagnostic_label(&self) -> String {
        format!(
            "display_id={}, bundle_id={:?}, primary_pid={:?}, resolved_window_ids={:?}, window_set_epoch={}",
            self.display_id,
            self.bundle_id,
            self.primary_pid,
            self.resolved_window_ids,
            self.window_set_epoch
        )
    }
}

impl ResolvedCaptureTargetProof {
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        backend: impl Into<String>,
        target_kind: RemoteDesktopTargetKind,
        display_id: Option<u64>,
        window_id: Option<u64>,
        pid: Option<i64>,
        app_identity: Option<String>,
        bundle_id: Option<String>,
        native_dimensions: Option<(usize, usize)>,
    ) -> Self {
        let (native_width, native_height) = native_dimensions
            .map(|(width, height)| (Some(width), Some(height)))
            .unwrap_or((None, None));
        Self {
            backend: backend.into(),
            target_kind,
            display_id,
            window_id,
            pid,
            app_identity,
            bundle_id,
            app_window_set: None,
            native_width,
            native_height,
            verified_at_ms: unix_epoch_ms(),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn with_app_window_set(
        mut self,
        app_window_set: AppWindowSetProof,
    ) -> Self {
        self.app_window_set = Some(app_window_set);
        self
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "backend": self.backend,
            "target_kind": self.target_kind.as_str(),
            "display_id": self.display_id,
            "window_id": self.window_id,
            "pid": self.pid,
            "app_identity": self.app_identity,
            "bundle_id": self.bundle_id,
            "app_window_set": self.app_window_set.as_ref().map(AppWindowSetProof::to_value),
            "native_width": self.native_width,
            "native_height": self.native_height,
            "verified_at_ms": self.verified_at_ms,
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
                    "application capture proof has no resolved display-scoped window-set proof",
                )
            })?;
            match phase {
                CaptureProofValidationPhase::InitialCommit => {}
                CaptureProofValidationPhase::ReverifyCommitted => {
                    let expected = binding.app_window_set.as_ref().ok_or_else(|| {
                        RemoteAppTargetError::new(
                            ability,
                            TargetResolutionError::TargetMetadataIncomplete,
                            "application target binding has no committed display-scoped window-set proof",
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
                }
            }
        }
        Ok(())
    }

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
    }

    fn native_app_identity_expectation(&self) -> NativeAppIdentityExpectation<'_> {
        NativeAppIdentityExpectation {
            expected_pid: self.pid,
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
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureProofValidationPhase {
    InitialCommit,
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
    app_identity: Option<String>,
    bundle_id: Option<String>,
    app_name: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct NativeAppIdentityCandidate<'a> {
    pid: Option<i64>,
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
            bundle_id,
            app_identity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) struct NativeAppIdentityExpectation<'a> {
    expected_pid: Option<i64>,
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
        let bundle_matches = self.expected_bundle_id.is_none_or(|expected| {
            candidate.bundle_id == Some(expected) || candidate.app_identity == Some(expected)
        });
        let app_identity_matches = self.expected_app_identity.is_none_or(|expected| {
            candidate.app_identity == Some(expected) || candidate.bundle_id == Some(expected)
        });
        let any_expected_field_seen = self
            .expected_pid
            .is_some_and(|expected| candidate.pid == Some(expected))
            || self.expected_bundle_id.is_some_and(|expected| {
                candidate.bundle_id == Some(expected) || candidate.app_identity == Some(expected)
            })
            || self.expected_app_identity.is_some_and(|expected| {
                candidate.app_identity == Some(expected) || candidate.bundle_id == Some(expected)
            });
        NativeAppIdentityMatch {
            matched: pid_matches && bundle_matches && app_identity_matches,
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

    pub(in crate::daemon::plugins::remote_desktop) const fn any_expected_field_seen(self) -> bool {
        self.any_expected_field_seen
    }
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

    #[cfg(test)]
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
    input_scope_reason: InputScopeReason,
    scope_widened: bool,
    display_fallback_used: bool,
}

impl ScopeAudit {
    fn to_value(&self) -> Value {
        json!({
            "requested_target_kind": self.requested_target_kind.as_str(),
            "effective_target_kind": self.effective_target_kind.as_str(),
            "target_model": self.effective_target_kind.target_model(),
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
    geometry: TargetGeometry,
    scope_audit: ScopeAudit,
    diagnostic: Value,
    diagnostic_capture_subject: DiagnosticCaptureSubject,
    capture_proof: Option<ResolvedCaptureTargetProof>,
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
                    "application capture proof has no resolved display-scoped window-set proof",
                )
            })?);
            if let Some(committed_window_set) = self.app_window_set.as_ref() {
                self.target_identity_epoch = committed_window_set.window_set_epoch;
            }
        }
        self.capture_proof = Some(proof);
        Ok(())
    }

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
            "target_model": self.target_kind.target_model(),
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
            "capture_proof": self.capture_proof.as_ref().map(ResolvedCaptureTargetProof::to_value),
            "bounds": self.geometry.to_value(),
            "binding_ready": binding_ready,
            "scope_ready": binding_ready,
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
            "target_model": self.target_kind.target_model(),
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
        crate::daemon::plugins::remote_desktop::screencapturekit_capture::verify_target_binding_for_session(
            ability, binding,
        )
    }
}

#[cfg(not(all(target_os = "macos", feature = "native-media")))]
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
                binding.native_locator().display_id(),
                None,
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
                        "{} targets require a native platform capture backend; \
                         headless/display providers cannot prove app/window binding",
                        binding.target_kind().as_str()
                    ),
                ))
            }
        }
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
        let target_kind = RemoteDesktopTargetKind::try_from(entry.kind).map_err(|error| {
            RemoteAppTargetError::new(ability, error.reason(), error.to_string())
        })?;
        validate_resource_inventory_state(ability, entry, target_kind)?;
        let capture_scope = capture_scope_for_kind(target_kind);
        let input_scope_decision = input_scope_for_request(target_kind, requested_mode);
        let input_scope = input_scope_decision.scope();
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
        let app_window_set = AppWindowSetProof::from_entry(entry, display_id);
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
            input_scope_reason: input_scope_decision.reason(),
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
            "target_model": target_kind.target_model(),
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
            geometry,
            scope_audit,
            diagnostic,
            diagnostic_capture_subject: DiagnosticCaptureSubject::from_entry(entry),
            capture_proof: None,
        })
    }
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

    if let Some(stale_after_ms) = metadata_freshness_u64(entry, "stale_after_ms") {
        let now_ms = unix_epoch_ms();
        if stale_after_ms <= now_ms {
            return Err(RemoteAppTargetError::new(
                ability,
                TargetResolutionError::TargetStale,
                format!(
                    "remote desktop target {} live inventory row expired at {stale_after_ms}; refresh targets before creating a session",
                    entry.resource_ura
                ),
            ));
        }
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
                    "application targets require resolved_window_ids and window_set_epoch so capture can prove the display-scoped app window set",
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
) -> InputScopeDecision {
    if requested_mode != "interactive" {
        return InputScopeDecision::new(InputScope::ViewOnly, InputScopeReason::RequestedViewOnly);
    }
    match target_kind {
        // Capture/session consent does not authorize keyboard or pointer input.
        // Until a separate EasyNet input-consent authority is available, even
        // display sessions requested as interactive remain view-only.
        RemoteDesktopTargetKind::Display => {
            InputScopeDecision::new(InputScope::ViewOnly, InputScopeReason::InputConsentRequired)
        }
        RemoteDesktopTargetKind::Window | RemoteDesktopTargetKind::Application => {
            // macOS target-scoped keyboard/pointer dispatch is unsafe until the
            // focus/activation validator is implemented. The session can still
            // capture app/window surfaces in view-only mode.
            InputScopeDecision::new(
                InputScope::ViewOnly,
                InputScopeReason::TargetScopedInputUnsafe,
            )
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
    let now = unix_epoch_ms();
    format!("tb_{now:x}_{:016x}", hasher.finish())
}

fn compute_window_set_epoch(
    display_id: Option<u64>,
    bundle_id: Option<&str>,
    primary_pid: Option<i64>,
    resolved_window_ids: &[u64],
) -> u64 {
    let mut hasher = DefaultHasher::new();
    display_id.hash(&mut hasher);
    bundle_id.hash(&mut hasher);
    primary_pid.hash(&mut hasher);
    resolved_window_ids.hash(&mut hasher);
    hasher.finish()
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
        metadata
    }

    fn interactive_application_binding() -> RemoteAppTargetBinding {
        ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Application,
                    live_metadata(json!({
                        "display_id": 1,
                        "bundle_id": "com.apple.Safari",
                        "app_identity": "com.apple.Safari",
                        "app_name": "Safari",
                        "primary_pid": 42,
                        "resolved_window_ids": [7, 8],
                        "window_set_epoch": 99,
                        "target_identity_epoch": 99,
                    })),
                ),
                "interactive",
                1,
            )
            .expect("display-scoped application identity must resolve")
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
    fn resolver_rejects_unavailable_or_expired_inventory_rows_before_binding() {
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

        let err = resolver
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
            .expect_err("expired live inventory rows must fail closed");
        assert_eq!(err.reason(), TargetResolutionError::TargetStale);
        assert!(
            err.to_string().contains("frontend_action=refresh_targets"),
            "unexpected error: {err}"
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
            None,
            Some(8),
            Some(4242),
            Some("com.apple.Terminal".to_string()),
            Some("com.apple.Terminal".to_string()),
            Some((1280, 720)),
        );
        let err = binding
            .commit_capture_proof("remote_desktop.create_session", wrong_window)
            .expect_err("proof must match the binding identity before it is stored");
        assert_eq!(err.reason(), TargetResolutionError::TargetIdentityMismatch);

        let proof = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Window,
            None,
            Some(7),
            Some(4242),
            Some("com.apple.Terminal".to_string()),
            Some("com.apple.Terminal".to_string()),
            Some((1280, 720)),
        );
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
            None,
            Some(7),
            Some(5150),
            Some("com.apple.Terminal".to_string()),
            Some("com.apple.Terminal".to_string()),
            Some((1280, 720)),
        );
        let err = binding
            .validate_reverified_capture_proof("remote_desktop.set_description", &drifted_pid)
            .expect_err("media path must fail if live target drifts from committed proof");
        assert_eq!(err.reason(), TargetResolutionError::TargetIdentityMismatch);
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
            None,
            Some(7),
            Some(4242),
            Some("com.example.Editor".to_string()),
            None,
            Some((1280, 720)),
        );
        binding
            .commit_capture_proof("remote_desktop.create_session", committed_from_app_identity)
            .expect("proof may project bundle identity through app_identity");

        let reverified_from_bundle_id = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Window,
            None,
            Some(7),
            Some(4242),
            None,
            Some("com.example.Editor".to_string()),
            Some((1280, 720)),
        );
        binding
            .validate_reverified_capture_proof(
                "remote_desktop.set_description",
                &reverified_from_bundle_id,
            )
            .expect("same native app identity may arrive through bundle_id on reverify");

        let mismatched_identity = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Window,
            None,
            Some(7),
            Some(4242),
            None,
            Some("com.example.Other".to_string()),
            Some((1280, 720)),
        );
        let err = binding
            .validate_reverified_capture_proof(
                "remote_desktop.set_description",
                &mismatched_identity,
            )
            .expect_err("different native app identity must still fail closed");
        assert_eq!(err.reason(), TargetResolutionError::TargetIdentityMismatch);
    }

    #[test]
    fn application_capture_proof_requires_exact_display_scoped_window_set() {
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
            Some(42),
            None,
            Some(9001),
            Some("com.example.Editor".to_string()),
            Some("com.example.Editor".to_string()),
            Some((1440, 900)),
        )
        .with_app_window_set(expected_window_set);
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
            Some(42),
            None,
            Some(9001),
            Some("com.example.Editor".to_string()),
            Some("com.example.Editor".to_string()),
            Some((1440, 900)),
        )
        .with_app_window_set(drifted_window_set);
        let err = binding
            .validate_reverified_capture_proof("remote_desktop.set_description", &drifted_proof)
            .expect_err("application media proof must fail when the live window set drifts");
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
            Some(42),
            None,
            Some(9001),
            Some("com.example.Editor".to_string()),
            Some("com.example.Editor".to_string()),
            Some((1440, 900)),
        )
        .with_app_window_set(live_window_set.clone());

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
        binding
            .validate_reverified_capture_proof("remote_desktop.set_description", &live_proof)
            .expect("same live proof remains valid after initial commit");

        let drifted_after_commit = ResolvedCaptureTargetProof::new(
            binding.native_locator().capture_backend.clone(),
            RemoteDesktopTargetKind::Application,
            Some(42),
            None,
            Some(9001),
            Some("com.example.Editor".to_string()),
            Some("com.example.Editor".to_string()),
            Some((1440, 900)),
        )
        .with_app_window_set(AppWindowSetProof::new(
            42,
            Some("com.example.Editor".to_string()),
            Some(9001),
            vec![10, 12],
        ));
        let err = binding
            .validate_reverified_capture_proof(
                "remote_desktop.set_description",
                &drifted_after_commit,
            )
            .expect_err("post-commit app window-set drift must fail closed");
        assert_eq!(err.reason(), TargetResolutionError::TargetIdentityChanged);
    }

    #[test]
    fn application_requires_display_scoped_stable_identity() {
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
        assert_eq!(err.reason(), TargetResolutionError::DisplayIdentityMissing);
        let err = resolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &entry(
                    ResourceType::Application,
                    live_metadata(json!({"display_id": 1, "app_name": "Safari"})),
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
                        "display_id": 1,
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
            json!("display_scoped_application_window_set")
        );
        assert_eq!(projection["capture_scope"], json!("AppSurface"));
        assert_eq!(projection["input_scope"], json!("view_only"));
        assert_eq!(
            projection["input_scope_reason"],
            json!("target_scoped_keyboard_pointer_dispatch_unsafe")
        );
        assert_eq!(
            projection["resolved_identity"]["bundle_id"],
            json!("com.apple.Safari")
        );
        assert_eq!(projection["resolved_identity"]["display_id"], json!(1));
        assert_eq!(
            binding.scope_audit_value()["target_model"],
            json!("display_scoped_application_window_set")
        );
        assert_eq!(
            binding.scope_audit_value()["input_scope_reason"],
            json!("target_scoped_keyboard_pointer_dispatch_unsafe")
        );
        assert_eq!(
            binding.latest_target_diagnostic_value()["target_model"],
            json!("display_scoped_application_window_set")
        );
        assert_eq!(
            binding.target_bound_event_payload()["target_model"],
            json!("display_scoped_application_window_set")
        );
        assert_eq!(
            binding.target_bound_event_payload()["consent_epoch"],
            json!(binding.consent_epoch())
        );
        assert_eq!(
            binding.target_bound_event_payload()["input_scope_reason"],
            json!("target_scoped_keyboard_pointer_dispatch_unsafe")
        );
    }

    #[test]
    fn application_interactive_downgrade_projects_input_scope_reason() {
        let binding = interactive_application_binding();
        assert_eq!(
            binding.to_value()["input_scope_reason"],
            json!("target_scoped_keyboard_pointer_dispatch_unsafe")
        );
        assert_eq!(
            binding.scope_audit_value()["input_scope_reason"],
            json!("target_scoped_keyboard_pointer_dispatch_unsafe")
        );
        assert_eq!(
            binding.target_bound_event_payload()["input_scope_reason"],
            json!("target_scoped_keyboard_pointer_dispatch_unsafe")
        );
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
}
