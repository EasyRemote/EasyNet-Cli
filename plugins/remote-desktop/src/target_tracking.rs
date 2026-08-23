// EasyNet CLI — remote desktop target tracking state
// ==================================================
//
// File: plugins/remote-desktop/src/target_tracking.rs
// Description: Session-owned target tracking state for app/window/display
// remote desktop sessions.
//
// Boundary:
// - RemoteAppTargetBindingStateMachine is not a platform poller and does not mutate resource
//   inventory. It is the session aggregate's committed view of target
//   observations.
// - Platform trackers such as macOS CGWindowList/ScreenCaptureKit diff loops
//   submit TargetObservation values. The session aggregate remains the single
//   writer for state transitions and ordered event-log rows.

use serde_json::{json, Value};

use crate::daemon::plugins::remote_desktop::target::{
    AppWindowSetProof, FrontendAction, RemoteAppTargetBinding, ResolvedCaptureTargetProof,
    TargetGeometry, TargetResolutionError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum TargetBindingPhase {
    Unresolved,
    Resolved,
    Stale,
    Lost,
    Rebinding,
    Invalidated,
}

impl TargetBindingPhase {
    pub(in crate::daemon::plugins::remote_desktop) fn as_str(self) -> &'static str {
        debug_assert!(ALL_TARGET_BINDING_PHASES.contains(&self));
        match self {
            Self::Unresolved => "unresolved",
            Self::Resolved => "resolved",
            Self::Stale => "stale",
            Self::Lost => "lost",
            Self::Rebinding => "rebinding",
            Self::Invalidated => "invalidated",
        }
    }

    fn recoverability(self) -> &'static str {
        match self {
            Self::Unresolved => "resolve_required",
            Self::Resolved => "continue",
            Self::Stale => "refresh_required",
            Self::Lost => "terminate",
            Self::Rebinding => "retry_session",
            Self::Invalidated => "terminate",
        }
    }
}

const ALL_TARGET_BINDING_PHASES: &[TargetBindingPhase] = &[
    TargetBindingPhase::Unresolved,
    TargetBindingPhase::Resolved,
    TargetBindingPhase::Stale,
    TargetBindingPhase::Lost,
    TargetBindingPhase::Rebinding,
    TargetBindingPhase::Invalidated,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum TargetVisibilityState {
    Visible,
    Hidden,
    Minimized,
    Lost,
}

impl TargetVisibilityState {
    pub(in crate::daemon::plugins::remote_desktop) fn as_str(self) -> &'static str {
        debug_assert!(ALL_TARGET_VISIBILITY_STATES.contains(&self));
        match self {
            Self::Visible => "visible",
            Self::Hidden => "hidden",
            Self::Minimized => "minimized",
            Self::Lost => "lost",
        }
    }

    fn input_enabled(self) -> bool {
        matches!(self, Self::Visible)
    }
}

const ALL_TARGET_VISIBILITY_STATES: &[TargetVisibilityState] = &[
    TargetVisibilityState::Visible,
    TargetVisibilityState::Hidden,
    TargetVisibilityState::Minimized,
    TargetVisibilityState::Lost,
];

const LOST_DEBOUNCE_REQUIRED_MISSES: u32 = 2;
const LOST_DEBOUNCE_MS: u64 = 1_000;
const AUTOMATIC_REBIND_WINDOW_MS: u64 = 30_000;
const TARGET_LIFECYCLE_EVENT_COALESCE_INTERVAL_MS: u64 = 100;

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetTrackerSnapshot {
    binding_id: String,
    binding_epoch: u64,
    target_identity_epoch: u64,
    target_geometry_revision: u64,
    target_focus_epoch: u64,
    media_source_epoch: u64,
    status: TargetBindingPhase,
    visibility_state: TargetVisibilityState,
    title: Option<String>,
    focused: Option<bool>,
    input_blocked_reason_override: Option<&'static str>,
    available_display_ids: Vec<u64>,
    geometry: TargetGeometry,
    diagnostic: Value,
}

impl TargetTrackerSnapshot {
    pub(in crate::daemon::plugins::remote_desktop) fn from_binding(
        binding: &RemoteAppTargetBinding,
    ) -> Self {
        Self {
            binding_id: binding.binding_id().to_string(),
            binding_epoch: binding.binding_epoch(),
            target_identity_epoch: binding.target_identity_epoch(),
            target_geometry_revision: binding.target_geometry_revision(),
            target_focus_epoch: 1,
            media_source_epoch: binding.media_source_epoch(),
            status: TargetBindingPhase::Resolved,
            visibility_state: TargetVisibilityState::Visible,
            title: binding.native_locator().title().map(str::to_string),
            focused: None,
            input_blocked_reason_override: None,
            available_display_ids: binding.native_locator().display_id().into_iter().collect(),
            geometry: binding.geometry().clone(),
            diagnostic: binding.latest_target_diagnostic_value(),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "binding_id": self.binding_id,
            "binding_epoch": self.binding_epoch,
            "target_identity_epoch": self.target_identity_epoch,
            "target_geometry_revision": self.target_geometry_revision,
            "target_focus_epoch": self.target_focus_epoch,
            "media_source_epoch": self.media_source_epoch,
            "status": self.status.as_str(),
            "visibility_state": self.visibility_state.as_str(),
            "title": self.title,
            "focused": self.focused,
            "input_blocked_reason": self.input_blocked_reason(),
            "available_display_ids": self.available_display_ids,
            "geometry": self.geometry.to_value(),
            "input_enabled": self.input_enabled(),
        })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn latest_diagnostic(&self) -> Value {
        self.diagnostic.clone()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn pointer_target_value(&self) -> Option<Value> {
        if !self.input_enabled() {
            return None;
        }
        let origin_x = self.geometry.x?;
        let origin_y = self.geometry.y?;
        Some(json!({
            "binding_id": self.binding_id,
            "binding_epoch": self.binding_epoch,
            "target_identity_epoch": self.target_identity_epoch,
            "target_geometry_revision": self.target_geometry_revision,
            "target_focus_epoch": self.target_focus_epoch,
            "origin_x": origin_x,
            "origin_y": origin_y,
            "width": self.geometry.width,
            "height": self.geometry.height,
        }))
    }

    pub(in crate::daemon::plugins::remote_desktop) fn input_enabled(&self) -> bool {
        self.input_blocked_reason().is_none()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn input_blocked_reason(
        &self,
    ) -> Option<&'static str> {
        if let Some(reason) = self.input_blocked_reason_override {
            return Some(reason);
        }
        if matches!(
            self.status,
            TargetBindingPhase::Unresolved
                | TargetBindingPhase::Lost
                | TargetBindingPhase::Rebinding
                | TargetBindingPhase::Invalidated
        ) {
            return match self.status {
                TargetBindingPhase::Unresolved => Some("target_unresolved"),
                TargetBindingPhase::Lost => Some("target_lost"),
                TargetBindingPhase::Rebinding => Some("target_rebinding"),
                TargetBindingPhase::Invalidated => Some("target_invalidated"),
                TargetBindingPhase::Resolved | TargetBindingPhase::Stale => None,
            };
        }
        if !self.visibility_state.input_enabled() {
            return match self.visibility_state {
                TargetVisibilityState::Hidden => Some("target_hidden"),
                TargetVisibilityState::Minimized => Some("target_minimized"),
                TargetVisibilityState::Lost => Some("target_lost"),
                TargetVisibilityState::Visible => None,
            };
        }
        if self.status == TargetBindingPhase::Stale {
            return Some("target_stale");
        }
        if self.focused == Some(false) {
            return Some("target_blurred");
        }
        None
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_geometry_revision(&self) -> u64 {
        self.target_geometry_revision
    }

    pub(in crate::daemon::plugins::remote_desktop) fn geometry(&self) -> &TargetGeometry {
        &self.geometry
    }

    pub(in crate::daemon::plugins::remote_desktop) fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn focused(&self) -> Option<bool> {
        self.focused
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_focus_epoch(&self) -> u64 {
        self.target_focus_epoch
    }
}

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) enum TargetObservation {
    // Constructed by the platform TargetTracker provider once the macOS
    // CGWindowList/ScreenCaptureKit poller is wired. Kept here as the
    // session-owned seam so future providers cannot bypass the aggregate.
    #[allow(dead_code)]
    GeometryChanged {
        geometry: TargetGeometry,
        target_geometry_revision: u64,
        observed_at_ms: u64,
    },
    VisibilityChanged {
        visibility_state: TargetVisibilityState,
        target_geometry_revision: u64,
        observed_at_ms: u64,
    },
    TitleChanged {
        title: Option<String>,
        observed_at_ms: u64,
    },
    FocusChanged {
        focused: bool,
        observed_at_ms: u64,
    },
    ApplicationWindowSetChanged {
        app_window_set: AppWindowSetProof,
        geometry: TargetGeometry,
        target_identity_epoch: u64,
        target_geometry_revision: u64,
        observed_at_ms: u64,
    },
    PermissionRevoked {
        detail: String,
        observed_at_ms: u64,
    },
    DisplayTopologyChanged {
        available_display_ids: Vec<u64>,
        selected_display_available: bool,
        observed_at_ms: u64,
    },
    // Constructed by the platform TargetTracker provider when the bound target
    // disappears independently from WebRTC transport state.
    #[allow(dead_code)]
    Lost {
        reason: TargetResolutionError,
        detail: String,
        observed_at_ms: u64,
    },
}

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetTrackingEmission {
    events: Vec<TargetTrackingEvent>,
}

#[derive(Debug, Clone)]
struct TargetTrackingEvent {
    event_type: &'static str,
    payload: Value,
}

impl TargetTrackingEmission {
    fn single(event_type: &'static str, payload: Value) -> Self {
        Self {
            events: vec![TargetTrackingEvent {
                event_type,
                payload,
            }],
        }
    }

    fn ordered(event_types: &[&'static str], payload: Value) -> Option<Self> {
        let events = event_types
            .iter()
            .map(|event_type| TargetTrackingEvent {
                event_type,
                payload: payload.clone(),
            })
            .collect::<Vec<_>>();
        (!events.is_empty()).then_some(Self { events })
    }

    pub(in crate::daemon::plugins::remote_desktop) fn event_type(&self) -> &'static str {
        self.events
            .first()
            .expect("target tracking emission is constructed non-empty")
            .event_type
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn payload(&self) -> Value {
        self.events
            .first()
            .expect("target tracking emission is constructed non-empty")
            .payload
            .clone()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn ordered_events(
        &self,
    ) -> Vec<(&'static str, Value)> {
        self.events
            .iter()
            .map(|event| (event.event_type, event.payload.clone()))
            .collect()
    }

    #[cfg(test)]
    fn ordered_event_types(&self) -> Vec<&'static str> {
        self.ordered_events()
            .into_iter()
            .map(|(event_type, _)| event_type)
            .collect()
    }
}

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteAppTargetBindingStateMachine {
    binding: RemoteAppTargetBinding,
    snapshot: TargetTrackerSnapshot,
    pending_media_rebind: Option<PendingMediaRebind>,
    pending_lost: Option<PendingLostObservation>,
    latest_loss_observed_at_ms: Option<u64>,
    rebind_started_at_ms: Option<u64>,
    rebind_failure_emitted: bool,
    lifecycle_event_coalescer: TargetLifecycleEventCoalescer,
}

#[derive(Debug, Clone)]
struct PendingMediaRebind {
    binding: RemoteAppTargetBinding,
    previous_binding_epoch: u64,
    previous_target_identity_epoch: u64,
    previous_target_geometry_revision: u64,
    previous_media_source_epoch: u64,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    detail: &'static str,
    observed_at_ms: u64,
}

#[derive(Debug, Clone)]
struct PendingLostObservation {
    reason: TargetResolutionError,
    detail: String,
    first_observed_at_ms: u64,
    latest_observed_at_ms: u64,
    consecutive_misses: u32,
}

#[derive(Debug, Clone, Default)]
struct TargetLifecycleEventCoalescer {
    last_emitted_at_ms: Option<u64>,
    suppressed_since_last: u64,
}

#[derive(Debug, Clone, Copy)]
struct TargetLifecycleEventEmission {
    suppressed_since_last: u64,
}

impl TargetLifecycleEventCoalescer {
    fn observe(&mut self, observed_at_ms: u64) -> Option<TargetLifecycleEventEmission> {
        if self.last_emitted_at_ms.is_none_or(|last_emitted_at_ms| {
            observed_at_ms.saturating_sub(last_emitted_at_ms)
                >= TARGET_LIFECYCLE_EVENT_COALESCE_INTERVAL_MS
        }) {
            let emission = TargetLifecycleEventEmission {
                suppressed_since_last: self.suppressed_since_last,
            };
            self.last_emitted_at_ms = Some(observed_at_ms);
            self.suppressed_since_last = 0;
            return Some(emission);
        }
        self.suppressed_since_last = self.suppressed_since_last.saturating_add(1);
        None
    }
}

impl RemoteAppTargetBindingStateMachine {
    pub(in crate::daemon::plugins::remote_desktop) fn from_binding(
        binding: RemoteAppTargetBinding,
    ) -> Self {
        Self {
            snapshot: TargetTrackerSnapshot::from_binding(&binding),
            binding,
            pending_media_rebind: None,
            pending_lost: None,
            latest_loss_observed_at_ms: None,
            rebind_started_at_ms: None,
            rebind_failure_emitted: false,
            lifecycle_event_coalescer: TargetLifecycleEventCoalescer::default(),
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn binding(&self) -> &RemoteAppTargetBinding {
        &self.binding
    }

    pub(in crate::daemon::plugins::remote_desktop) fn snapshot(&self) -> &TargetTrackerSnapshot {
        &self.snapshot
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn pending_media_rebind_binding(
        &self,
    ) -> Option<&RemoteAppTargetBinding> {
        self.pending_media_rebind
            .as_ref()
            .map(|pending| &pending.binding)
    }

    #[cfg(test)]
    pub(in crate::daemon::plugins::remote_desktop) fn commit_observation(
        &mut self,
        observation: TargetObservation,
    ) -> Option<TargetTrackingEmission> {
        self.commit_observation_with_media_source_activity(observation, false)
    }

    pub(in crate::daemon::plugins::remote_desktop) fn commit_observation_with_media_source_activity(
        &mut self,
        observation: TargetObservation,
        media_source_active: bool,
    ) -> Option<TargetTrackingEmission> {
        match observation {
            TargetObservation::GeometryChanged {
                geometry,
                target_geometry_revision,
                observed_at_ms,
            } => self.commit_geometry(geometry, target_geometry_revision, observed_at_ms),
            TargetObservation::VisibilityChanged {
                visibility_state,
                target_geometry_revision,
                observed_at_ms,
            } => self.commit_visibility(visibility_state, target_geometry_revision, observed_at_ms),
            TargetObservation::TitleChanged {
                title,
                observed_at_ms,
            } => self.commit_title(title, observed_at_ms),
            TargetObservation::FocusChanged {
                focused,
                observed_at_ms,
            } => self.commit_focus(focused, observed_at_ms),
            TargetObservation::ApplicationWindowSetChanged {
                app_window_set,
                geometry,
                target_identity_epoch,
                target_geometry_revision,
                observed_at_ms,
            } => {
                if media_source_active {
                    self.stage_application_window_set_media_rebind(
                        app_window_set,
                        geometry,
                        target_identity_epoch,
                        target_geometry_revision,
                        observed_at_ms,
                    )
                } else {
                    self.commit_application_window_set(
                        app_window_set,
                        geometry,
                        target_identity_epoch,
                        target_geometry_revision,
                        observed_at_ms,
                    )
                }
            }
            TargetObservation::PermissionRevoked {
                detail,
                observed_at_ms,
            } => self.commit_permission_revoked(detail, observed_at_ms),
            TargetObservation::DisplayTopologyChanged {
                available_display_ids,
                selected_display_available,
                observed_at_ms,
            } => self.commit_display_topology(
                available_display_ids,
                selected_display_available,
                observed_at_ms,
            ),
            TargetObservation::Lost {
                reason,
                detail,
                observed_at_ms,
            } => self.commit_lost(reason, detail, observed_at_ms),
        }
    }

    fn commit_geometry(
        &mut self,
        geometry: TargetGeometry,
        target_geometry_revision: u64,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if self.snapshot.status == TargetBindingPhase::Lost {
            return self.begin_rebinding("target_geometry_after_loss", observed_at_ms);
        }
        if self.snapshot.status == TargetBindingPhase::Rebinding {
            return self.commit_rebind_failed("target_geometry_after_loss", observed_at_ms);
        }
        self.clear_pending_lost();
        let previous = self.snapshot.target_geometry_revision;
        if target_geometry_revision <= previous && geometry == self.snapshot.geometry {
            return None;
        }
        let event_types = geometry_event_types(&self.snapshot.geometry, &geometry);
        self.snapshot.status = TargetBindingPhase::Resolved;
        self.snapshot.visibility_state = TargetVisibilityState::Visible;
        self.snapshot.geometry = geometry;
        self.snapshot.target_geometry_revision = target_geometry_revision.max(previous + 1);
        self.snapshot.diagnostic = self.diagnostic_projection(
            "resolved",
            Value::Null,
            "target_geometry_changed",
            observed_at_ms,
        );
        self.coalesced_lifecycle_events(
            &event_types,
            self.event_payload("target_geometry_changed", observed_at_ms, Some(previous)),
            observed_at_ms,
        )
    }

    fn commit_visibility(
        &mut self,
        visibility_state: TargetVisibilityState,
        target_geometry_revision: u64,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if self.snapshot.status == TargetBindingPhase::Lost {
            if visibility_state == TargetVisibilityState::Lost {
                return None;
            }
            return self.begin_rebinding("target_visibility_after_loss", observed_at_ms);
        }
        if self.snapshot.status == TargetBindingPhase::Rebinding {
            if visibility_state == TargetVisibilityState::Lost {
                return None;
            }
            return self.commit_rebind_failed("target_visibility_after_loss", observed_at_ms);
        }
        if visibility_state == TargetVisibilityState::Lost {
            return self.commit_lost(
                TargetResolutionError::TargetNotFound,
                "target visibility reported lost".to_string(),
                observed_at_ms,
            );
        }
        self.clear_pending_lost();
        if self.snapshot.visibility_state == visibility_state
            && self.snapshot.target_geometry_revision >= target_geometry_revision
        {
            return None;
        }
        let was_visible = self.snapshot.visibility_state == TargetVisibilityState::Visible;
        let previous = self.snapshot.target_geometry_revision;
        self.snapshot.visibility_state = visibility_state;
        self.snapshot.status = match visibility_state {
            TargetVisibilityState::Visible => TargetBindingPhase::Resolved,
            TargetVisibilityState::Hidden | TargetVisibilityState::Minimized => {
                TargetBindingPhase::Stale
            }
            TargetVisibilityState::Lost => TargetBindingPhase::Lost,
        };
        self.snapshot.target_geometry_revision = target_geometry_revision.max(previous + 1);
        let target_reason = match visibility_state {
            TargetVisibilityState::Hidden => Some(TargetResolutionError::TargetHidden),
            TargetVisibilityState::Minimized => Some(TargetResolutionError::TargetMinimized),
            TargetVisibilityState::Visible | TargetVisibilityState::Lost => None,
        };
        let reason =
            target_reason
                .map(TargetResolutionError::as_str)
                .unwrap_or(match visibility_state {
                    TargetVisibilityState::Visible if !was_visible => "target_restored",
                    TargetVisibilityState::Visible => "target_visible",
                    TargetVisibilityState::Hidden => "target_hidden",
                    TargetVisibilityState::Minimized => "target_minimized",
                    TargetVisibilityState::Lost => "target_lost",
                });
        let diagnostic = self.diagnostic_projection(
            self.snapshot.status.as_str(),
            json!(reason),
            reason,
            observed_at_ms,
        );
        self.snapshot.diagnostic = if let Some(reason) = target_reason {
            target_failure_payload(diagnostic, reason.frontend_action().as_str())
        } else {
            diagnostic
        };
        let payload = self.event_payload(reason, observed_at_ms, Some(previous));
        let payload = if let Some(reason) = target_reason {
            target_failure_payload(payload, reason.frontend_action().as_str())
        } else {
            payload
        };
        Some(TargetTrackingEmission::single(
            match visibility_state {
                TargetVisibilityState::Visible if !was_visible => "TARGET_RESTORED",
                TargetVisibilityState::Visible => "TARGET_VISIBLE",
                TargetVisibilityState::Hidden => "TARGET_HIDDEN",
                TargetVisibilityState::Minimized => "TARGET_MINIMIZED",
                TargetVisibilityState::Lost => "TARGET_LOST",
            },
            payload,
        ))
    }

    fn commit_title(
        &mut self,
        title: Option<String>,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if self.snapshot.status == TargetBindingPhase::Lost {
            return self.begin_rebinding("target_title_after_loss", observed_at_ms);
        }
        if self.snapshot.status == TargetBindingPhase::Rebinding {
            return self.commit_rebind_failed("target_title_after_loss", observed_at_ms);
        }
        if self.snapshot.status != TargetBindingPhase::Resolved || self.snapshot.title == title {
            return None;
        }
        let previous_title = std::mem::replace(&mut self.snapshot.title, title);
        self.snapshot.diagnostic = self.diagnostic_projection(
            self.snapshot.status.as_str(),
            Value::Null,
            "target_title_changed",
            observed_at_ms,
        );
        let mut payload = self.event_payload("target_title_changed", observed_at_ms, None);
        payload["previous_title"] = json!(previous_title);
        payload["title"] = json!(self.snapshot.title);
        self.coalesced_lifecycle_event("TARGET_TITLE_CHANGED", payload, observed_at_ms)
    }

    fn commit_focus(
        &mut self,
        focused: bool,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if self.snapshot.status == TargetBindingPhase::Lost {
            return self.begin_rebinding("target_focus_after_loss", observed_at_ms);
        }
        if self.snapshot.status == TargetBindingPhase::Rebinding {
            return self.commit_rebind_failed("target_focus_after_loss", observed_at_ms);
        }
        if self.snapshot.status != TargetBindingPhase::Resolved
            || self.snapshot.focused == Some(focused)
        {
            return None;
        }
        self.set_focused(focused);
        let reason = if focused {
            "target_focused"
        } else {
            "target_blurred"
        };
        let target_action = (!focused).then_some(FrontendAction::RetrySession);
        let diagnostic = self.diagnostic_projection(
            self.snapshot.status.as_str(),
            Value::Null,
            reason,
            observed_at_ms,
        );
        self.snapshot.diagnostic = if let Some(action) = target_action {
            target_failure_payload(diagnostic, action.as_str())
        } else {
            diagnostic
        };
        let mut payload = self.event_payload(reason, observed_at_ms, None);
        payload["focused"] = json!(focused);
        let payload = if let Some(action) = target_action {
            target_failure_payload(payload, action.as_str())
        } else {
            payload
        };
        Some(TargetTrackingEmission::single(
            if focused {
                "TARGET_FOCUSED"
            } else {
                "TARGET_BLURRED"
            },
            payload,
        ))
    }

    fn commit_application_window_set(
        &mut self,
        app_window_set: AppWindowSetProof,
        geometry: TargetGeometry,
        target_identity_epoch: u64,
        target_geometry_revision: u64,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if self.snapshot.status == TargetBindingPhase::Lost {
            return self.begin_rebinding("application_window_set_after_loss", observed_at_ms);
        }
        if self.snapshot.status == TargetBindingPhase::Rebinding {
            return self.commit_rebind_failed("application_window_set_after_loss", observed_at_ms);
        }
        self.clear_pending_lost();
        let previous_identity_epoch = self.snapshot.target_identity_epoch;
        let previous_geometry_revision = self.snapshot.target_geometry_revision;
        if previous_identity_epoch == target_identity_epoch && self.snapshot.geometry == geometry {
            return None;
        }
        let next_geometry_revision = target_geometry_revision.max(previous_geometry_revision + 1);
        let Some(next_binding) = self.binding.application_window_set_rebind_candidate(
            app_window_set.clone(),
            geometry.clone(),
            next_geometry_revision,
            false,
        ) else {
            return self.commit_lost(
                TargetResolutionError::TargetMetadataIncomplete,
                "application window-set observation cannot update non-application binding"
                    .to_string(),
                observed_at_ms,
            );
        };
        self.binding = next_binding;
        self.snapshot.status = TargetBindingPhase::Resolved;
        self.snapshot.visibility_state = TargetVisibilityState::Visible;
        self.snapshot.binding_epoch = self.binding.binding_epoch();
        self.snapshot.geometry = geometry;
        self.snapshot.target_identity_epoch = target_identity_epoch;
        self.snapshot.target_geometry_revision = next_geometry_revision;
        self.snapshot.diagnostic = self.diagnostic_projection(
            "resolved",
            Value::Null,
            "application_window_set_changed",
            observed_at_ms,
        );
        let mut payload = self.event_payload(
            "application_window_set_changed",
            observed_at_ms,
            Some(previous_geometry_revision),
        );
        payload["previous_target_identity_epoch"] = json!(previous_identity_epoch);
        payload["target_identity_epoch"] = json!(target_identity_epoch);
        payload["app_window_set"] = app_window_set.to_value();
        self.coalesced_lifecycle_event("TARGET_REBOUND", payload, observed_at_ms)
    }

    fn stage_application_window_set_media_rebind(
        &mut self,
        app_window_set: AppWindowSetProof,
        geometry: TargetGeometry,
        target_identity_epoch: u64,
        target_geometry_revision: u64,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if self.snapshot.status == TargetBindingPhase::Lost {
            return self.begin_rebinding("application_window_set_after_loss", observed_at_ms);
        }
        if self.snapshot.status == TargetBindingPhase::Rebinding
            && self.pending_media_rebind.as_ref().is_some_and(|pending| {
                pending.binding.target_identity_epoch() == target_identity_epoch
                    && pending.binding.geometry() == &geometry
            })
        {
            return None;
        }
        self.clear_pending_lost();
        let previous_binding_epoch = self.snapshot.binding_epoch;
        let previous_target_identity_epoch = self.snapshot.target_identity_epoch;
        let previous_target_geometry_revision = self.snapshot.target_geometry_revision;
        let previous_media_source_epoch = self.snapshot.media_source_epoch;
        if previous_target_identity_epoch == target_identity_epoch
            && self.snapshot.geometry == geometry
        {
            return None;
        }
        let next_geometry_revision =
            target_geometry_revision.max(previous_target_geometry_revision + 1);
        let Some(candidate) = self.binding.application_window_set_rebind_candidate(
            app_window_set.clone(),
            geometry.clone(),
            next_geometry_revision,
            true,
        ) else {
            return self.commit_lost(
                TargetResolutionError::TargetMetadataIncomplete,
                "application window-set observation cannot stage media rebind for non-application binding"
                    .to_string(),
                observed_at_ms,
            );
        };
        let detail = "application_window_set_requires_media_source_rebuild";
        self.pending_media_rebind = Some(PendingMediaRebind {
            binding: candidate.clone(),
            previous_binding_epoch,
            previous_target_identity_epoch,
            previous_target_geometry_revision,
            previous_media_source_epoch,
            detail,
            observed_at_ms,
        });
        self.snapshot.status = TargetBindingPhase::Rebinding;
        self.rebind_started_at_ms = Some(observed_at_ms);
        self.snapshot.diagnostic = target_failure_payload(
            json!({
                "status": TargetBindingPhase::Rebinding.as_str(),
                "reason": "target_rebind_attempted",
                "detail": detail,
                "subject_ura": self.binding.subject_ura(),
                "binding_id": self.snapshot.binding_id,
                "binding_epoch": self.snapshot.binding_epoch,
                "target_identity_epoch": self.snapshot.target_identity_epoch,
                "target_geometry_revision": self.snapshot.target_geometry_revision,
                "media_source_epoch": self.snapshot.media_source_epoch,
                "pending_binding_epoch": candidate.binding_epoch(),
                "pending_target_identity_epoch": candidate.target_identity_epoch(),
                "pending_target_geometry_revision": candidate.target_geometry_revision(),
                "pending_media_source_epoch": candidate.media_source_epoch(),
                "pending_app_window_set": app_window_set.to_value(),
                "visibility_state": self.snapshot.visibility_state.as_str(),
                "recoverability": TargetBindingPhase::Rebinding.recoverability(),
                "rebind_deadline_ms": observed_at_ms.saturating_add(AUTOMATIC_REBIND_WINDOW_MS),
                "observed_at_ms": observed_at_ms,
            }),
            FrontendAction::RetrySession.as_str(),
        );
        let payload = self.with_event_target_context(target_failure_payload(
            json!({
                "subject_ura": self.binding.subject_ura(),
                "binding_id": self.snapshot.binding_id,
                "binding_epoch": self.snapshot.binding_epoch,
                "previous_binding_epoch": previous_binding_epoch,
                "pending_binding_epoch": candidate.binding_epoch(),
                "previous_target_identity_epoch": previous_target_identity_epoch,
                "target_identity_epoch": self.snapshot.target_identity_epoch,
                "pending_target_identity_epoch": candidate.target_identity_epoch(),
                "previous_target_geometry_revision": previous_target_geometry_revision,
                "target_geometry_revision": self.snapshot.target_geometry_revision,
                "pending_target_geometry_revision": candidate.target_geometry_revision(),
                "media_source_epoch": self.snapshot.media_source_epoch,
                "pending_media_source_epoch": candidate.media_source_epoch(),
                "previous_media_source_epoch": previous_media_source_epoch,
                "visibility_state": self.snapshot.visibility_state.as_str(),
                "target_status": TargetBindingPhase::Rebinding.as_str(),
                "reason_code": "target_rebind_attempted",
                "detail": detail,
                "recoverability": TargetBindingPhase::Rebinding.recoverability(),
                "rebind_deadline_ms": observed_at_ms.saturating_add(AUTOMATIC_REBIND_WINDOW_MS),
                "observed_at_ms": observed_at_ms,
                "geometry": self.snapshot.geometry.to_value(),
                "pending_geometry": geometry.to_value(),
                "app_window_set": app_window_set.to_value(),
            }),
            FrontendAction::RetrySession.as_str(),
        ));
        Some(TargetTrackingEmission::single(
            "TARGET_REBIND_ATTEMPTED",
            payload,
        ))
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn commit_pending_media_rebind(
        &mut self,
        binding_epoch: u64,
        media_source_epoch: u64,
        capture_proof: ResolvedCaptureTargetProof,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        let pending = self.pending_media_rebind.take()?;
        if pending.binding.binding_epoch() != binding_epoch
            || pending.binding.media_source_epoch() != media_source_epoch
        {
            self.pending_media_rebind = Some(pending);
            return None;
        }
        let mut rebound_binding = pending.binding;
        if rebound_binding
            .commit_capture_proof(
                "remote_desktop.target.commit_pending_media_rebind",
                capture_proof,
            )
            .is_err()
        {
            self.pending_media_rebind = Some(PendingMediaRebind {
                binding: rebound_binding,
                previous_binding_epoch: pending.previous_binding_epoch,
                previous_target_identity_epoch: pending.previous_target_identity_epoch,
                previous_target_geometry_revision: pending.previous_target_geometry_revision,
                previous_media_source_epoch: pending.previous_media_source_epoch,
                detail: pending.detail,
                observed_at_ms: pending.observed_at_ms,
            });
            return None;
        }
        self.binding = rebound_binding;
        self.snapshot.status = TargetBindingPhase::Resolved;
        self.snapshot.visibility_state = TargetVisibilityState::Visible;
        self.snapshot.binding_epoch = self.binding.binding_epoch();
        self.snapshot.target_identity_epoch = self.binding.target_identity_epoch();
        self.snapshot.target_geometry_revision = self.binding.target_geometry_revision();
        self.snapshot.media_source_epoch = self.binding.media_source_epoch();
        self.snapshot.geometry = self.binding.geometry().clone();
        self.snapshot.diagnostic = self.diagnostic_projection(
            "resolved",
            Value::Null,
            "application_window_set_media_source_rebound",
            observed_at_ms,
        );
        self.rebind_started_at_ms = None;
        self.rebind_failure_emitted = false;
        let mut payload = self.event_payload(
            "application_window_set_media_source_rebound",
            observed_at_ms,
            Some(pending.previous_target_geometry_revision),
        );
        payload["previous_binding_epoch"] = json!(pending.previous_binding_epoch);
        payload["binding_epoch"] = json!(self.snapshot.binding_epoch);
        payload["previous_target_identity_epoch"] = json!(pending.previous_target_identity_epoch);
        payload["target_identity_epoch"] = json!(self.snapshot.target_identity_epoch);
        payload["previous_media_source_epoch"] = json!(pending.previous_media_source_epoch);
        payload["media_source_epoch"] = json!(self.snapshot.media_source_epoch);
        payload["rebind_started_at_ms"] = json!(pending.observed_at_ms);
        payload["detail"] = json!(pending.detail);
        Some(TargetTrackingEmission::single("TARGET_REBOUND", payload))
    }

    pub(in crate::daemon::plugins::remote_desktop) fn commit_pending_media_rebind_failed(
        &mut self,
        reason: TargetResolutionError,
        detail: String,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if self.snapshot.status != TargetBindingPhase::Rebinding || self.rebind_failure_emitted {
            return None;
        }
        let pending = self.pending_media_rebind.take()?;
        self.rebind_failure_emitted = true;
        let rebind_started_at_ms = self
            .rebind_started_at_ms
            .take()
            .or(Some(pending.observed_at_ms));
        let rebind_deadline_ms =
            rebind_started_at_ms.map(|started| started.saturating_add(AUTOMATIC_REBIND_WINDOW_MS));
        self.snapshot.status = TargetBindingPhase::Lost;
        self.snapshot.visibility_state = TargetVisibilityState::Lost;
        self.set_focused(false);
        let frontend_action = reason.frontend_action().as_str();
        let reason_code = reason.as_str();
        self.snapshot.diagnostic = target_failure_payload(
            json!({
                "status": TargetBindingPhase::Lost.as_str(),
                "reason": reason_code,
                "detail": detail,
                "subject_ura": self.binding.subject_ura(),
                "binding_id": self.snapshot.binding_id,
                "binding_epoch": self.snapshot.binding_epoch,
                "previous_binding_epoch": pending.previous_binding_epoch,
                "pending_binding_epoch": pending.binding.binding_epoch(),
                "target_identity_epoch": self.snapshot.target_identity_epoch,
                "previous_target_identity_epoch": pending.previous_target_identity_epoch,
                "pending_target_identity_epoch": pending.binding.target_identity_epoch(),
                "target_geometry_revision": self.snapshot.target_geometry_revision,
                "previous_target_geometry_revision": pending.previous_target_geometry_revision,
                "pending_target_geometry_revision": pending.binding.target_geometry_revision(),
                "media_source_epoch": self.snapshot.media_source_epoch,
                "previous_media_source_epoch": pending.previous_media_source_epoch,
                "pending_media_source_epoch": pending.binding.media_source_epoch(),
                "visibility_state": self.snapshot.visibility_state.as_str(),
                "target_status": TargetBindingPhase::Lost.as_str(),
                "recoverability": "new_session_required",
                "observed_at_ms": observed_at_ms,
                "rebind_started_at_ms": rebind_started_at_ms,
                "rebind_deadline_ms": rebind_deadline_ms,
            }),
            frontend_action,
        );
        let payload = self.with_event_target_context(target_failure_payload(
            json!({
                "subject_ura": self.binding.subject_ura(),
                "binding_id": self.snapshot.binding_id,
                "binding_epoch": self.snapshot.binding_epoch,
                "previous_binding_epoch": pending.previous_binding_epoch,
                "pending_binding_epoch": pending.binding.binding_epoch(),
                "previous_target_identity_epoch": pending.previous_target_identity_epoch,
                "target_identity_epoch": self.snapshot.target_identity_epoch,
                "pending_target_identity_epoch": pending.binding.target_identity_epoch(),
                "previous_target_geometry_revision": pending.previous_target_geometry_revision,
                "target_geometry_revision": self.snapshot.target_geometry_revision,
                "pending_target_geometry_revision": pending.binding.target_geometry_revision(),
                "previous_media_source_epoch": pending.previous_media_source_epoch,
                "media_source_epoch": self.snapshot.media_source_epoch,
                "pending_media_source_epoch": pending.binding.media_source_epoch(),
                "visibility_state": self.snapshot.visibility_state.as_str(),
                "target_status": TargetBindingPhase::Lost.as_str(),
                "reason_code": reason_code,
                "detail": detail,
                "recoverability": "new_session_required",
                "observed_at_ms": observed_at_ms,
                "rebind_started_at_ms": rebind_started_at_ms,
                "rebind_deadline_ms": rebind_deadline_ms,
                "geometry": self.snapshot.geometry.to_value(),
                "pending_geometry": pending.binding.geometry().to_value(),
            }),
            frontend_action,
        ));
        Some(TargetTrackingEmission::single(
            "TARGET_REBIND_FAILED",
            payload,
        ))
    }

    pub(in crate::daemon::plugins::remote_desktop) fn expire_rebind_deadline(
        &mut self,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if self.snapshot.status != TargetBindingPhase::Rebinding || self.rebind_failure_emitted {
            return None;
        }
        let rebind_started_at_ms = self.rebind_started_at_ms?;
        let rebind_deadline_ms = rebind_started_at_ms.saturating_add(AUTOMATIC_REBIND_WINDOW_MS);
        if observed_at_ms < rebind_deadline_ms {
            return None;
        }
        if self.pending_media_rebind.is_some() {
            return self.commit_pending_media_rebind_failed(
                TargetResolutionError::TargetStale,
                "rebind_window_expired".to_string(),
                observed_at_ms,
            );
        }
        self.commit_rebind_failed("rebind_window_expired", observed_at_ms)
    }

    fn commit_permission_revoked(
        &mut self,
        detail: String,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if self.snapshot.status == TargetBindingPhase::Invalidated {
            return None;
        }
        self.clear_pending_lost();
        self.snapshot.status = TargetBindingPhase::Invalidated;
        self.snapshot.visibility_state = TargetVisibilityState::Lost;
        self.set_focused(false);
        self.snapshot.diagnostic = json!({
            "status": self.snapshot.status.as_str(),
            "reason": TargetResolutionError::TargetPermissionMissing.as_str(),
            "detail": detail,
            "subject_ura": self.binding.subject_ura(),
            "binding_id": self.snapshot.binding_id,
            "binding_epoch": self.snapshot.binding_epoch,
            "target_identity_epoch": self.snapshot.target_identity_epoch,
            "target_geometry_revision": self.snapshot.target_geometry_revision,
            "media_source_epoch": self.snapshot.media_source_epoch,
            "visibility_state": self.snapshot.visibility_state.as_str(),
            "input_enabled": false,
            "recoverability": self.snapshot.status.recoverability(),
            "frontend_action": TargetResolutionError::TargetPermissionMissing.frontend_action().as_str(),
            "observed_at_ms": observed_at_ms,
        });
        let mut payload = self.event_payload(
            TargetResolutionError::TargetPermissionMissing.as_str(),
            observed_at_ms,
            None,
        );
        payload["detail"] = json!(detail);
        payload["frontend_action"] = json!(TargetResolutionError::TargetPermissionMissing
            .frontend_action()
            .as_str());
        Some(TargetTrackingEmission::single(
            "TARGET_PERMISSION_REVOKED",
            target_failure_payload(
                payload,
                TargetResolutionError::TargetPermissionMissing
                    .frontend_action()
                    .as_str(),
            ),
        ))
    }

    fn commit_display_topology(
        &mut self,
        mut available_display_ids: Vec<u64>,
        selected_display_available: bool,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        available_display_ids.sort_unstable();
        available_display_ids.dedup();
        let display_unavailable_reason = TargetResolutionError::TargetDisplayUnavailable.as_str();
        let was_selected_display_unavailable =
            self.snapshot.input_blocked_reason_override == Some(display_unavailable_reason);
        if self.snapshot.available_display_ids == available_display_ids
            && ((selected_display_available && !was_selected_display_unavailable)
                || (!selected_display_available && was_selected_display_unavailable))
        {
            return None;
        }
        let previous_display_ids = std::mem::replace(
            &mut self.snapshot.available_display_ids,
            available_display_ids,
        );
        if selected_display_available {
            self.snapshot.status =
                if self.snapshot.visibility_state == TargetVisibilityState::Visible {
                    TargetBindingPhase::Resolved
                } else {
                    TargetBindingPhase::Stale
                };
            if was_selected_display_unavailable {
                self.snapshot.input_blocked_reason_override = None;
            }
        } else {
            self.snapshot.status = TargetBindingPhase::Stale;
            self.snapshot.input_blocked_reason_override = Some(display_unavailable_reason);
        }
        let diagnostic_reason = if selected_display_available {
            Value::Null
        } else {
            json!(display_unavailable_reason)
        };
        self.snapshot.diagnostic = self.diagnostic_projection(
            self.snapshot.status.as_str(),
            diagnostic_reason,
            "display_topology_changed",
            observed_at_ms,
        );
        let reason = if selected_display_available {
            "display_topology_changed"
        } else {
            display_unavailable_reason
        };
        let mut payload = self.event_payload(reason, observed_at_ms, None);
        payload["detail"] = json!("display_topology_changed");
        payload["previous_display_ids"] = json!(previous_display_ids);
        payload["available_display_ids"] = json!(self.snapshot.available_display_ids);
        payload["selected_display_available"] = json!(selected_display_available);
        let payload = if selected_display_available {
            payload
        } else {
            target_failure_payload(
                payload,
                TargetResolutionError::TargetDisplayUnavailable
                    .frontend_action()
                    .as_str(),
            )
        };
        Some(TargetTrackingEmission::single(
            "DISPLAY_TOPOLOGY_CHANGED",
            payload,
        ))
    }

    fn commit_lost(
        &mut self,
        reason: TargetResolutionError,
        detail: String,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if self.snapshot.status == TargetBindingPhase::Lost {
            self.latest_loss_observed_at_ms = Some(
                self.latest_loss_observed_at_ms
                    .map_or(observed_at_ms, |latest| latest.max(observed_at_ms)),
            );
            return None;
        }
        let pending_snapshot = {
            let pending = self
                .pending_lost
                .get_or_insert_with(|| PendingLostObservation {
                    reason,
                    detail: detail.clone(),
                    first_observed_at_ms: observed_at_ms,
                    latest_observed_at_ms: observed_at_ms,
                    consecutive_misses: 0,
                });
            pending.reason = reason;
            pending.detail = detail;
            pending.latest_observed_at_ms = observed_at_ms;
            pending.consecutive_misses = pending.consecutive_misses.saturating_add(1);
            pending.clone()
        };

        if pending_snapshot.consecutive_misses < LOST_DEBOUNCE_REQUIRED_MISSES
            && observed_at_ms.saturating_sub(pending_snapshot.first_observed_at_ms)
                < LOST_DEBOUNCE_MS
        {
            self.snapshot.input_blocked_reason_override = Some("target_loss_pending");
            self.snapshot.diagnostic = self.pending_lost_diagnostic(&pending_snapshot);
            return None;
        }

        let pending = self
            .pending_lost
            .take()
            .expect("pending lost exists after debounce gate");
        let previous = self.snapshot.target_geometry_revision;
        self.snapshot.status = TargetBindingPhase::Lost;
        self.snapshot.visibility_state = TargetVisibilityState::Lost;
        self.snapshot.input_blocked_reason_override = None;
        self.latest_loss_observed_at_ms = Some(observed_at_ms);
        self.snapshot.diagnostic = json!({
            "status": TargetBindingPhase::Lost.as_str(),
            "reason": pending.reason.as_str(),
            "detail": pending.detail,
            "subject_ura": self.binding.subject_ura(),
            "binding_id": self.snapshot.binding_id,
            "binding_epoch": self.snapshot.binding_epoch,
            "target_identity_epoch": self.snapshot.target_identity_epoch,
            "target_geometry_revision": self.snapshot.target_geometry_revision,
            "visibility_state": self.snapshot.visibility_state.as_str(),
            "recoverability": TargetBindingPhase::Lost.recoverability(),
            "frontend_action": pending.reason.frontend_action().as_str(),
            "lost_debounce": {
                "first_observed_at_ms": pending.first_observed_at_ms,
                "latest_observed_at_ms": pending.latest_observed_at_ms,
                "consecutive_misses": pending.consecutive_misses,
            },
            "observed_at_ms": observed_at_ms,
        });
        let mut payload =
            self.event_payload(pending.reason.as_str(), observed_at_ms, Some(previous));
        payload["detail"] = json!(pending.detail);
        Some(TargetTrackingEmission::single(
            "TARGET_LOST",
            target_failure_payload(payload, pending.reason.frontend_action().as_str()),
        ))
    }

    fn begin_rebinding(
        &mut self,
        detail: &'static str,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if self.rebind_failure_emitted
            || self
                .latest_loss_observed_at_ms
                .is_some_and(|lost_at| observed_at_ms <= lost_at)
        {
            return None;
        }
        self.snapshot.status = TargetBindingPhase::Rebinding;
        self.rebind_started_at_ms = Some(observed_at_ms);
        let frontend_action = FrontendAction::RetrySession.as_str();
        self.snapshot.diagnostic = target_failure_payload(
            json!({
                "status": TargetBindingPhase::Rebinding.as_str(),
                "reason": "target_rebind_attempted",
                "detail": detail,
                "subject_ura": self.binding.subject_ura(),
                "binding_id": self.snapshot.binding_id,
                "binding_epoch": self.snapshot.binding_epoch,
                "target_identity_epoch": self.snapshot.target_identity_epoch,
                "target_geometry_revision": self.snapshot.target_geometry_revision,
                "media_source_epoch": self.snapshot.media_source_epoch,
                "visibility_state": self.snapshot.visibility_state.as_str(),
                "recoverability": TargetBindingPhase::Rebinding.recoverability(),
                "rebind_deadline_ms": observed_at_ms.saturating_add(AUTOMATIC_REBIND_WINDOW_MS),
                "observed_at_ms": observed_at_ms,
            }),
            frontend_action,
        );
        let payload = self.with_event_target_context(target_failure_payload(
            json!({
                "subject_ura": self.binding.subject_ura(),
                "binding_id": self.snapshot.binding_id,
                "binding_epoch": self.snapshot.binding_epoch,
                "previous_target_identity_epoch": self.snapshot.target_identity_epoch,
                "target_identity_epoch": self.snapshot.target_identity_epoch,
                "target_geometry_revision": self.snapshot.target_geometry_revision,
                "media_source_epoch": self.snapshot.media_source_epoch,
                "visibility_state": self.snapshot.visibility_state.as_str(),
                "target_status": TargetBindingPhase::Rebinding.as_str(),
                "reason_code": "target_rebind_attempted",
                "detail": detail,
                "recoverability": TargetBindingPhase::Rebinding.recoverability(),
                "rebind_deadline_ms": observed_at_ms.saturating_add(AUTOMATIC_REBIND_WINDOW_MS),
                "observed_at_ms": observed_at_ms,
                "geometry": self.snapshot.geometry.to_value(),
            }),
            frontend_action,
        ));
        Some(TargetTrackingEmission::single(
            "TARGET_REBIND_ATTEMPTED",
            payload,
        ))
    }

    fn commit_rebind_failed(
        &mut self,
        detail: &'static str,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if self.snapshot.status != TargetBindingPhase::Rebinding {
            return None;
        }
        if self.rebind_failure_emitted {
            return None;
        }
        self.rebind_failure_emitted = true;
        let rebind_started_at_ms = self.rebind_started_at_ms.take();
        let rebind_deadline_ms =
            rebind_started_at_ms.map(|started| started.saturating_add(AUTOMATIC_REBIND_WINDOW_MS));
        self.snapshot.status = TargetBindingPhase::Lost;
        let reason_code = "explicit_rebind_required";
        let frontend_action = FrontendAction::RefreshTargets.as_str();
        self.snapshot.diagnostic = target_failure_payload(
            json!({
                "status": TargetBindingPhase::Lost.as_str(),
                "reason": reason_code,
                "detail": detail,
                "subject_ura": self.binding.subject_ura(),
                "binding_id": self.snapshot.binding_id,
                "binding_epoch": self.snapshot.binding_epoch,
                "target_identity_epoch": self.snapshot.target_identity_epoch,
                "target_geometry_revision": self.snapshot.target_geometry_revision,
                "visibility_state": self.snapshot.visibility_state.as_str(),
                "target_status": TargetBindingPhase::Lost.as_str(),
                "recoverability": "new_session_required",
                "observed_at_ms": observed_at_ms,
                "rebind_started_at_ms": rebind_started_at_ms,
                "rebind_deadline_ms": rebind_deadline_ms,
            }),
            frontend_action,
        );
        let payload = self.with_event_target_context(target_failure_payload(
            json!({
                "subject_ura": self.binding.subject_ura(),
                "binding_id": self.snapshot.binding_id,
                "binding_epoch": self.snapshot.binding_epoch,
                "previous_target_identity_epoch": self.snapshot.target_identity_epoch,
                "target_identity_epoch": self.snapshot.target_identity_epoch,
                "previous_target_geometry_revision": self.snapshot.target_geometry_revision,
                "target_geometry_revision": self.snapshot.target_geometry_revision,
                "media_source_epoch": self.snapshot.media_source_epoch,
                "visibility_state": self.snapshot.visibility_state.as_str(),
                "target_status": TargetBindingPhase::Lost.as_str(),
                "reason_code": reason_code,
                "detail": detail,
                "recoverability": "new_session_required",
                "observed_at_ms": observed_at_ms,
                "rebind_started_at_ms": rebind_started_at_ms,
                "rebind_deadline_ms": rebind_deadline_ms,
                "geometry": self.snapshot.geometry.to_value(),
            }),
            frontend_action,
        ));
        Some(TargetTrackingEmission::single(
            "TARGET_REBIND_FAILED",
            payload,
        ))
    }

    fn clear_pending_lost(&mut self) {
        self.pending_lost = None;
        if self.snapshot.input_blocked_reason_override == Some("target_loss_pending") {
            self.snapshot.input_blocked_reason_override = None;
        }
    }

    fn coalesced_lifecycle_event(
        &mut self,
        event_type: &'static str,
        payload: Value,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        self.coalesced_lifecycle_events(&[event_type], payload, observed_at_ms)
    }

    fn coalesced_lifecycle_events(
        &mut self,
        event_types: &[&'static str],
        mut payload: Value,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEmission> {
        if event_types.is_empty() {
            return None;
        }
        let emission = self.lifecycle_event_coalescer.observe(observed_at_ms)?;
        payload["coalesced_target_events"] = json!(emission.suppressed_since_last);
        payload["coalesce_interval_ms"] = json!(TARGET_LIFECYCLE_EVENT_COALESCE_INTERVAL_MS);
        TargetTrackingEmission::ordered(event_types, payload)
    }

    fn pending_lost_diagnostic(&self, pending: &PendingLostObservation) -> Value {
        target_failure_payload(
            json!({
                "status": self.snapshot.status.as_str(),
                "reason": pending.reason.as_str(),
                "detail": pending.detail,
                "subject_ura": self.binding.subject_ura(),
                "binding_id": self.snapshot.binding_id,
                "binding_epoch": self.snapshot.binding_epoch,
                "target_identity_epoch": self.snapshot.target_identity_epoch,
                "target_geometry_revision": self.snapshot.target_geometry_revision,
                "visibility_state": self.snapshot.visibility_state.as_str(),
                "input_blocked_reason": "target_loss_pending",
                "recoverability": "debounce_pending",
                "lost_debounce": {
                    "state": "pending",
                    "required_misses": LOST_DEBOUNCE_REQUIRED_MISSES,
                    "required_elapsed_ms": LOST_DEBOUNCE_MS,
                    "first_observed_at_ms": pending.first_observed_at_ms,
                    "latest_observed_at_ms": pending.latest_observed_at_ms,
                    "consecutive_misses": pending.consecutive_misses,
                },
                "observed_at_ms": pending.latest_observed_at_ms,
            }),
            pending.reason.frontend_action().as_str(),
        )
    }

    fn diagnostic_projection(
        &self,
        status: &str,
        reason: Value,
        detail: &str,
        observed_at_ms: u64,
    ) -> Value {
        json!({
            "status": status,
            "reason": reason,
            "detail": detail,
            "subject_ura": self.binding.subject_ura(),
            "binding_id": self.snapshot.binding_id,
            "binding_epoch": self.snapshot.binding_epoch,
            "target_identity_epoch": self.snapshot.target_identity_epoch,
            "target_geometry_revision": self.snapshot.target_geometry_revision,
            "visibility_state": self.snapshot.visibility_state.as_str(),
            "input_blocked_reason": self.snapshot.input_blocked_reason(),
            "recoverability": self.snapshot.status.recoverability(),
            "frontend_action": Value::Null,
            "observed_at_ms": observed_at_ms,
        })
    }

    fn event_payload(
        &self,
        reason_code: &str,
        observed_at_ms: u64,
        previous_target_geometry_revision: Option<u64>,
    ) -> Value {
        self.with_event_target_context(json!({
            "subject_ura": self.binding.subject_ura(),
            "binding_id": self.snapshot.binding_id,
            "binding_epoch": self.snapshot.binding_epoch,
            "consent_epoch": self.binding.consent_epoch(),
            "previous_target_identity_epoch": self.snapshot.target_identity_epoch,
            "target_identity_epoch": self.snapshot.target_identity_epoch,
            "previous_target_geometry_revision": previous_target_geometry_revision,
            "target_geometry_revision": self.snapshot.target_geometry_revision,
            "target_focus_epoch": self.snapshot.target_focus_epoch,
            "media_source_epoch": self.snapshot.media_source_epoch,
            "visibility_state": self.snapshot.visibility_state.as_str(),
            "target_status": self.snapshot.status.as_str(),
            "input_enabled": self.snapshot.input_enabled(),
            "input_blocked_reason": self.snapshot.input_blocked_reason(),
            "reason_code": reason_code,
            "recoverability": self.snapshot.status.recoverability(),
            "frontend_action": Value::Null,
            "observed_at_ms": observed_at_ms,
            "geometry": self.snapshot.geometry.to_value(),
        }))
    }

    fn with_event_target_context(&self, mut payload: Value) -> Value {
        let Value::Object(fields) = &mut payload else {
            return payload;
        };
        fields.insert("subject_ura".to_string(), json!(self.binding.subject_ura()));
        fields.insert("binding_id".to_string(), json!(self.snapshot.binding_id));
        fields.insert(
            "binding_epoch".to_string(),
            json!(self.snapshot.binding_epoch),
        );
        fields.insert(
            "target_identity_epoch".to_string(),
            json!(self.snapshot.target_identity_epoch),
        );
        fields.insert(
            "target_geometry_revision".to_string(),
            json!(self.snapshot.target_geometry_revision),
        );
        fields.insert(
            "target_focus_epoch".to_string(),
            json!(self.snapshot.target_focus_epoch),
        );
        fields.insert(
            "media_source_epoch".to_string(),
            json!(self.snapshot.media_source_epoch),
        );
        fields.insert(
            "consent_epoch".to_string(),
            json!(self.binding.consent_epoch()),
        );
        fields.insert(
            "target_binding".to_string(),
            self.binding.to_tracking_value(
                self.snapshot.target_identity_epoch,
                self.snapshot.target_geometry_revision,
                self.snapshot.media_source_epoch,
                &self.snapshot.geometry,
            ),
        );
        fields.insert("scope_audit".to_string(), self.binding.scope_audit_value());
        fields.insert(
            "latest_target_diagnostic".to_string(),
            self.snapshot.diagnostic.clone(),
        );
        payload
    }

    fn set_focused(&mut self, focused: bool) {
        if self.snapshot.focused != Some(focused) {
            self.snapshot.target_focus_epoch = self.snapshot.target_focus_epoch.saturating_add(1);
        }
        self.snapshot.focused = Some(focused);
    }
}

fn target_failure_payload(mut payload: Value, frontend_action: &str) -> Value {
    payload["failure_domain"] = json!("target");
    payload["input_enabled"] = json!(false);
    payload["frontend_action"] = json!(frontend_action);
    payload
}

fn geometry_event_types(previous: &TargetGeometry, next: &TargetGeometry) -> Vec<&'static str> {
    let moved = previous.x != next.x || previous.y != next.y;
    let resized = previous.width != next.width || previous.height != next.height;
    let mut event_types = Vec::with_capacity(2);
    if moved {
        event_types.push("TARGET_MOVED");
    }
    if resized {
        event_types.push("TARGET_RESIZED");
    }
    if event_types.is_empty() {
        event_types.push("TARGET_MOVED");
    }
    event_types
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::{AUTOMATIC_REBIND_WINDOW_MS, TARGET_LIFECYCLE_EVENT_COALESCE_INTERVAL_MS};

    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use crate::daemon::plugins::remote_desktop::target::{
        AppWindowSetProof, ResourceEntryTargetResolver, TargetGeometry, TargetResolutionError,
    };
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        RemoteAppTargetBindingStateMachine, TargetObservation, TargetVisibilityState,
    };
    use crate::daemon::plugins::remote_desktop::test_support::{
        live_remote_target_metadata, test_application_target_binding,
    };

    fn window_binding() -> crate::daemon::plugins::remote_desktop::target::RemoteAppTargetBinding {
        ResourceEntryTargetResolver
            .resolve_for_session(
                "test.ability",
                &ResourceEntry {
                    resource_ura: "easynet:///r/acme/resource/window.test".into(),
                    owner_agent: "easynet:///r/acme/agent/device.dev-1.media".into(),
                    kind: ResourceType::Window,
                    binding: ResourceBinding::LocalDevice,
                    hardware_id: "window:macos:cgwindow:10:42".into(),
                    display_name: "Cursor".into(),
                    metadata: live_remote_target_metadata(json!({
                        "window_id": 42,
                        "pid": 10,
                        "app_name": "Cursor",
                        "x": 100,
                        "y": 200,
                        "width": 800,
                        "height": 600,
                        "target_identity_epoch": 7,
                        "geometry_revision": 3,
                    })),
                    first_seen_at: "2026-06-01T00:00:00Z".into(),
                },
                "view_only",
                1,
            )
            .expect("window target binding resolves")
    }

    fn application_binding(
    ) -> crate::daemon::plugins::remote_desktop::target::RemoteAppTargetBinding {
        test_application_target_binding()
    }

    fn lost_window_tracker() -> RemoteAppTargetBindingStateMachine {
        let binding = window_binding();
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);
        assert!(tracker
            .commit_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "window disappeared".into(),
                observed_at_ms: 10,
            })
            .is_none());
        tracker
            .commit_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "window still disappeared".into(),
                observed_at_ms: 20,
            })
            .expect("lost commits after debounce");
        assert_eq!(tracker.snapshot().to_value()["status"], json!("lost"));
        tracker
    }

    #[test]
    fn tracker_snapshot_starts_from_session_binding() {
        let binding = window_binding();
        let binding_id = binding.binding_id().to_string();
        let tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);
        let snapshot = tracker.snapshot().to_value();

        assert_eq!(snapshot["binding_id"], json!(binding_id));
        assert_eq!(snapshot["target_identity_epoch"], json!(7));
        assert_eq!(snapshot["target_geometry_revision"], json!(3));
        assert_eq!(snapshot["target_focus_epoch"], json!(1));
        assert_eq!(snapshot["status"], json!("resolved"));
        assert_eq!(snapshot["visibility_state"], json!("visible"));
    }

    #[test]
    fn active_application_window_set_change_waits_for_media_rebind_commit() {
        let binding = application_binding();
        let original_binding_epoch = binding.binding_epoch();
        let original_identity_epoch = binding.target_identity_epoch();
        let original_geometry_revision = binding.target_geometry_revision();
        let original_media_source_epoch = binding.media_source_epoch();
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);
        let next_window_set = AppWindowSetProof::new(
            42,
            Some("com.example.Editor".to_string()),
            Some(9001),
            vec![10, 11, 12],
        );
        let next_geometry = TargetGeometry {
            x: Some(10.0),
            y: Some(20.0),
            width: Some(320.0),
            height: Some(120.0),
        };

        let attempted = tracker
            .commit_observation_with_media_source_activity(
                TargetObservation::ApplicationWindowSetChanged {
                    app_window_set: next_window_set,
                    geometry: next_geometry,
                    target_identity_epoch: 100,
                    target_geometry_revision: original_geometry_revision + 1,
                    observed_at_ms: 10,
                },
                true,
            )
            .expect("active app window-set drift stages a media rebind");

        assert_eq!(attempted.event_type(), "TARGET_REBIND_ATTEMPTED");
        assert_eq!(
            tracker.binding().binding_epoch(),
            original_binding_epoch,
            "active binding must not advance before the media source filter is rebuilt"
        );
        assert_eq!(
            tracker.binding().target_identity_epoch(),
            original_identity_epoch
        );
        assert_eq!(
            tracker.binding().media_source_epoch(),
            original_media_source_epoch
        );
        assert_eq!(tracker.snapshot().to_value()["status"], json!("rebinding"));
        assert_eq!(
            tracker.snapshot().to_value()["input_blocked_reason"],
            json!("target_rebinding")
        );

        let pending = tracker
            .pending_media_rebind_binding()
            .expect("pending media rebind binding")
            .clone();
        assert_eq!(
            pending.binding_epoch(),
            original_binding_epoch + 1,
            "explicit application rebind creates a new binding epoch"
        );
        assert_eq!(
            pending.media_source_epoch(),
            original_media_source_epoch + 1
        );
        assert_ne!(pending.target_identity_epoch(), original_identity_epoch);
        let proof = pending
            .require_capture_proof("test.ability")
            .expect("pending proof")
            .clone();

        let rebound = tracker
            .commit_pending_media_rebind(
                pending.binding_epoch(),
                pending.media_source_epoch(),
                proof,
                20,
            )
            .expect("media source rebuild commits the target rebound");
        assert_eq!(rebound.event_type(), "TARGET_REBOUND");
        assert_eq!(tracker.binding().binding_epoch(), pending.binding_epoch());
        assert_eq!(
            tracker.binding().media_source_epoch(),
            pending.media_source_epoch()
        );
        assert_eq!(
            tracker.snapshot().to_value()["status"],
            json!("resolved"),
            "successful media source rebind restores target input eligibility"
        );
    }

    #[test]
    fn active_application_window_set_rebind_failure_is_typed() {
        let binding = application_binding();
        let original_binding_epoch = binding.binding_epoch();
        let original_identity_epoch = binding.target_identity_epoch();
        let original_media_source_epoch = binding.media_source_epoch();
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);
        let next_window_set = AppWindowSetProof::new(
            42,
            Some("com.example.Editor".to_string()),
            Some(9001),
            vec![10, 11, 12],
        );

        tracker
            .commit_observation_with_media_source_activity(
                TargetObservation::ApplicationWindowSetChanged {
                    app_window_set: next_window_set,
                    geometry: TargetGeometry {
                        x: Some(10.0),
                        y: Some(20.0),
                        width: Some(320.0),
                        height: Some(120.0),
                    },
                    target_identity_epoch: 100,
                    target_geometry_revision: 4,
                    observed_at_ms: 10,
                },
                true,
            )
            .expect("active app window-set drift stages media rebind");

        let pending = tracker
            .pending_media_rebind_binding()
            .expect("pending media rebind binding")
            .clone();
        let failed = tracker
            .commit_pending_media_rebind_failed(
                TargetResolutionError::ScreenCaptureKitFilterFailed,
                "native content filter rejected pending application window set".to_string(),
                20,
            )
            .expect("media rebind failure emits typed target lifecycle event");

        assert_eq!(failed.event_type(), "TARGET_REBIND_FAILED");
        assert_eq!(
            failed.payload()["reason_code"],
            json!("screencapturekit_filter_failed")
        );
        assert_eq!(failed.payload()["failure_domain"], json!("target"));
        assert_eq!(
            failed.payload()["frontend_action"],
            json!("show_unsupported")
        );
        assert_eq!(failed.payload()["target_status"], json!("lost"));
        assert_eq!(failed.payload()["input_enabled"], json!(false));
        assert_eq!(
            failed.payload()["binding_epoch"],
            json!(original_binding_epoch)
        );
        assert_eq!(
            failed.payload()["target_identity_epoch"],
            json!(original_identity_epoch)
        );
        assert_eq!(
            failed.payload()["media_source_epoch"],
            json!(original_media_source_epoch)
        );
        assert_eq!(
            failed.payload()["pending_binding_epoch"],
            json!(pending.binding_epoch())
        );
        assert_eq!(
            failed.payload()["pending_target_identity_epoch"],
            json!(pending.target_identity_epoch())
        );
        assert_eq!(
            failed.payload()["pending_media_source_epoch"],
            json!(pending.media_source_epoch())
        );
        assert_eq!(tracker.snapshot().to_value()["status"], json!("lost"));
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["reason"],
            json!("screencapturekit_filter_failed")
        );
        assert!(tracker.pending_media_rebind_binding().is_none());
    }

    #[test]
    fn pending_media_rebind_expires_at_rebind_deadline() {
        let binding = application_binding();
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);
        let next_window_set = AppWindowSetProof::new(
            42,
            Some("com.example.Editor".to_string()),
            Some(9001),
            vec![10, 11, 12],
        );

        let attempted = tracker
            .commit_observation_with_media_source_activity(
                TargetObservation::ApplicationWindowSetChanged {
                    app_window_set: next_window_set,
                    geometry: TargetGeometry {
                        x: Some(10.0),
                        y: Some(20.0),
                        width: Some(320.0),
                        height: Some(120.0),
                    },
                    target_identity_epoch: 100,
                    target_geometry_revision: 4,
                    observed_at_ms: 10,
                },
                true,
            )
            .expect("active application window-set drift starts pending media rebind");
        assert_eq!(attempted.event_type(), "TARGET_REBIND_ATTEMPTED");
        assert!(tracker.pending_media_rebind_binding().is_some());

        assert!(
            tracker
                .expire_rebind_deadline(10 + AUTOMATIC_REBIND_WINDOW_MS - 1)
                .is_none(),
            "deadline must not expire before the published rebind window"
        );

        let expired = tracker
            .expire_rebind_deadline(10 + AUTOMATIC_REBIND_WINDOW_MS)
            .expect("pending media rebind expires deterministically at deadline");
        assert_eq!(expired.event_type(), "TARGET_REBIND_FAILED");
        assert_eq!(expired.payload()["reason_code"], json!("target_stale"));
        assert_eq!(expired.payload()["detail"], json!("rebind_window_expired"));
        assert_eq!(
            expired.payload()["rebind_started_at_ms"],
            json!(10),
            "expiry evidence must preserve the start of the bounded rebind window"
        );
        assert_eq!(
            expired.payload()["rebind_deadline_ms"],
            json!(10 + AUTOMATIC_REBIND_WINDOW_MS)
        );
        assert_eq!(expired.payload()["target_status"], json!("lost"));
        assert_eq!(expired.payload()["input_enabled"], json!(false));
        assert_eq!(tracker.snapshot().to_value()["status"], json!("lost"));
        assert!(tracker.pending_media_rebind_binding().is_none());
    }

    #[test]
    fn post_loss_rebind_attempt_expires_at_rebind_deadline() {
        let mut tracker = lost_window_tracker();

        let attempted = tracker
            .commit_observation(TargetObservation::VisibilityChanged {
                visibility_state: TargetVisibilityState::Visible,
                target_geometry_revision: 5,
                observed_at_ms: 30,
            })
            .expect("post-loss observation starts explicit rebind attempt");
        assert_eq!(attempted.event_type(), "TARGET_REBIND_ATTEMPTED");
        assert_eq!(tracker.snapshot().to_value()["status"], json!("rebinding"));

        let expired = tracker
            .expire_rebind_deadline(30 + AUTOMATIC_REBIND_WINDOW_MS)
            .expect("post-loss rebind attempt expires deterministically at deadline");
        assert_eq!(expired.event_type(), "TARGET_REBIND_FAILED");
        assert_eq!(
            expired.payload()["reason_code"],
            json!("explicit_rebind_required")
        );
        assert_eq!(expired.payload()["detail"], json!("rebind_window_expired"));
        assert_eq!(expired.payload()["rebind_started_at_ms"], json!(30));
        assert_eq!(
            expired.payload()["rebind_deadline_ms"],
            json!(30 + AUTOMATIC_REBIND_WINDOW_MS)
        );
        assert_eq!(expired.payload()["target_status"], json!("lost"));
        assert_eq!(expired.payload()["input_enabled"], json!(false));
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["failure_domain"],
            json!("target")
        );
    }

    #[test]
    fn tracker_commits_move_resize_and_lost_without_rebinding() {
        let binding = window_binding();
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);

        let moved = tracker
            .commit_observation(TargetObservation::GeometryChanged {
                geometry: TargetGeometry {
                    x: Some(140.0),
                    y: Some(220.0),
                    width: Some(800.0),
                    height: Some(600.0),
                },
                target_geometry_revision: 4,
                observed_at_ms: 10,
            })
            .expect("move commits");
        assert_eq!(moved.event_type(), "TARGET_MOVED");
        assert_eq!(moved.payload()["target_geometry_revision"], json!(4));

        let resized = tracker
            .commit_observation(TargetObservation::GeometryChanged {
                geometry: TargetGeometry {
                    x: Some(140.0),
                    y: Some(220.0),
                    width: Some(1024.0),
                    height: Some(768.0),
                },
                target_geometry_revision: 5,
                observed_at_ms: 120,
            })
            .expect("resize commits");
        assert_eq!(resized.event_type(), "TARGET_RESIZED");

        assert!(tracker
            .commit_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "window disappeared".into(),
                observed_at_ms: 30,
            })
            .is_none());
        assert_eq!(tracker.snapshot().to_value()["status"], json!("resolved"));

        let lost = tracker
            .commit_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "window disappeared".into(),
                observed_at_ms: 40,
            })
            .expect("lost commits");
        assert_eq!(lost.event_type(), "TARGET_LOST");
        assert_eq!(lost.payload()["failure_domain"], json!("target"));
        assert_eq!(lost.payload()["frontend_action"], json!("refresh_targets"));
        assert_eq!(lost.payload()["target_status"], json!("lost"));
        assert_eq!(lost.payload()["input_enabled"], json!(false));
        assert_eq!(lost.payload()["input_blocked_reason"], json!("target_lost"));
        assert_eq!(tracker.snapshot().to_value()["status"], json!("lost"));
        assert_eq!(
            tracker.snapshot().to_value()["input_blocked_reason"],
            json!("target_lost")
        );
        assert!(tracker.snapshot().pointer_target_value().is_none());
    }

    #[test]
    fn tracker_expands_combined_move_resize_observation_into_ordered_events() {
        let binding = window_binding();
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);

        let event = tracker
            .commit_observation(TargetObservation::GeometryChanged {
                geometry: TargetGeometry {
                    x: Some(140.0),
                    y: Some(220.0),
                    width: Some(1024.0),
                    height: Some(768.0),
                },
                target_geometry_revision: 4,
                observed_at_ms: 10,
            })
            .expect("combined move+resize commits one target snapshot update");

        assert_eq!(
            event.ordered_event_types(),
            vec!["TARGET_MOVED", "TARGET_RESIZED"],
            "one host geometry observation must preserve both lifecycle facts"
        );
        let ordered = event.ordered_events();
        assert_eq!(ordered[0].1["target_geometry_revision"], json!(4));
        assert_eq!(ordered[1].1["target_geometry_revision"], json!(4));
        assert_eq!(ordered[0].1["geometry"], ordered[1].1["geometry"]);
        assert_eq!(
            tracker.snapshot().target_geometry_revision(),
            4,
            "combined event expansion must not mutate the target snapshot twice"
        );
    }

    #[test]
    fn tracker_coalesces_high_rate_geometry_and_title_events() {
        let binding = window_binding();
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);

        let first = tracker
            .commit_observation(TargetObservation::GeometryChanged {
                geometry: TargetGeometry {
                    x: Some(101.0),
                    y: Some(201.0),
                    width: Some(800.0),
                    height: Some(600.0),
                },
                target_geometry_revision: 4,
                observed_at_ms: 1,
            })
            .expect("first high-rate lifecycle event emits immediately");
        assert_eq!(first.event_type(), "TARGET_MOVED");
        assert_eq!(first.payload()["coalesced_target_events"], json!(0));
        assert_eq!(
            first.payload()["coalesce_interval_ms"],
            json!(TARGET_LIFECYCLE_EVENT_COALESCE_INTERVAL_MS)
        );

        for observed_at_ms in 2..=100 {
            let event = if observed_at_ms % 2 == 0 {
                tracker.commit_observation(TargetObservation::TitleChanged {
                    title: Some(format!("Cursor {observed_at_ms}")),
                    observed_at_ms,
                })
            } else {
                tracker.commit_observation(TargetObservation::GeometryChanged {
                    geometry: TargetGeometry {
                        x: Some(100.0 + observed_at_ms as f64),
                        y: Some(200.0 + observed_at_ms as f64),
                        width: Some(800.0 + observed_at_ms as f64),
                        height: Some(600.0),
                    },
                    target_geometry_revision: observed_at_ms,
                    observed_at_ms,
                })
            };
            assert!(
                event.is_none(),
                "geometry/title lifecycle events must be session-coalesced under 10Hz"
            );
        }

        assert_eq!(tracker.snapshot().title(), Some("Cursor 100"));
        assert_eq!(
            tracker.snapshot().target_geometry_revision(),
            99,
            "suppressed geometry observations still update the committed target snapshot"
        );

        let sampled = tracker
            .commit_observation(TargetObservation::TitleChanged {
                title: Some("Cursor sampled".to_string()),
                observed_at_ms: 101,
            })
            .expect("next event after the coalesce interval emits");
        assert_eq!(sampled.event_type(), "TARGET_TITLE_CHANGED");
        assert_eq!(sampled.payload()["coalesced_target_events"], json!(99));
        assert_eq!(
            sampled.payload()["coalesce_interval_ms"],
            json!(TARGET_LIFECYCLE_EVENT_COALESCE_INTERVAL_MS)
        );
        assert_eq!(tracker.snapshot().title(), Some("Cursor sampled"));
    }

    #[test]
    fn tracker_debounces_single_transient_lost_observation() {
        let binding = window_binding();
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);

        assert!(tracker
            .commit_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "transient snapshot miss".into(),
                observed_at_ms: 10,
            })
            .is_none());
        assert_eq!(tracker.snapshot().to_value()["status"], json!("resolved"));
        assert_eq!(tracker.snapshot().to_value()["input_enabled"], json!(false));
        assert_eq!(
            tracker.snapshot().to_value()["input_blocked_reason"],
            json!("target_loss_pending")
        );
        assert!(tracker.snapshot().pointer_target_value().is_none());
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["failure_domain"],
            json!("target")
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["input_enabled"],
            json!(false)
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["input_blocked_reason"],
            json!("target_loss_pending")
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["lost_debounce"]["state"],
            json!("pending")
        );

        tracker
            .commit_observation(TargetObservation::GeometryChanged {
                geometry: TargetGeometry {
                    x: Some(140.0),
                    y: Some(220.0),
                    width: Some(800.0),
                    height: Some(600.0),
                },
                target_geometry_revision: 4,
                observed_at_ms: 20,
            })
            .expect("recovered geometry commits");

        assert_eq!(tracker.snapshot().to_value()["input_enabled"], json!(true));
        assert_eq!(
            tracker.snapshot().to_value()["target_focus_epoch"],
            json!(1)
        );
        assert_eq!(
            tracker.snapshot().to_value()["input_blocked_reason"],
            Value::Null
        );
        assert!(tracker.snapshot().pointer_target_value().is_some());

        assert!(tracker
            .commit_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "second transient snapshot miss".into(),
                observed_at_ms: 30,
            })
            .is_none());
        assert_eq!(tracker.snapshot().to_value()["status"], json!("resolved"));
        assert_eq!(
            tracker.snapshot().to_value()["input_blocked_reason"],
            json!("target_loss_pending")
        );
        assert_eq!(tracker.snapshot().to_value()["input_enabled"], json!(false));
    }

    #[test]
    fn tracker_commits_lost_after_debounce_elapsed() {
        let binding = window_binding();
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);

        assert!(tracker
            .commit_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "window disappeared".into(),
                observed_at_ms: 10,
            })
            .is_none());

        let lost = tracker
            .commit_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "window still missing".into(),
                observed_at_ms: 1_010,
            })
            .expect("elapsed debounce commits lost");

        assert_eq!(lost.event_type(), "TARGET_LOST");
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["lost_debounce"]["latest_observed_at_ms"],
            json!(1_010)
        );
    }

    #[test]
    fn tracker_reports_rebind_failure_after_target_loss_without_policy() {
        let mut tracker = lost_window_tracker();

        assert!(
            tracker
                .commit_observation(TargetObservation::GeometryChanged {
                    geometry: TargetGeometry {
                        x: Some(180.0),
                        y: Some(230.0),
                        width: Some(800.0),
                        height: Some(600.0),
                    },
                    target_geometry_revision: 7,
                    observed_at_ms: 20,
                })
                .is_none(),
            "stale queued observations at or before committed loss must not look like rebind"
        );
        assert!(tracker
            .commit_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "window still lost".into(),
                observed_at_ms: 40,
            })
            .is_none());
        assert!(
            tracker
                .commit_observation(TargetObservation::VisibilityChanged {
                    visibility_state: TargetVisibilityState::Visible,
                    target_geometry_revision: 7,
                    observed_at_ms: 30,
                })
                .is_none(),
            "observations older than the latest lost signal must stay silent"
        );

        let rebind_attempted = tracker
            .commit_observation(TargetObservation::GeometryChanged {
                geometry: TargetGeometry {
                    x: Some(200.0),
                    y: Some(250.0),
                    width: Some(800.0),
                    height: Some(600.0),
                },
                target_geometry_revision: 8,
                observed_at_ms: 50,
            })
            .expect("post-loss target observation must enter explicit rebinding");

        assert_eq!(rebind_attempted.event_type(), "TARGET_REBIND_ATTEMPTED");
        assert_eq!(
            rebind_attempted.payload()["target_status"],
            json!("rebinding")
        );
        assert_eq!(
            rebind_attempted.payload()["failure_domain"],
            json!("target")
        );
        assert_eq!(
            rebind_attempted.payload()["frontend_action"],
            json!("retry_session")
        );
        assert_eq!(rebind_attempted.payload()["input_enabled"], json!(false));
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["failure_domain"],
            json!("target")
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["frontend_action"],
            json!("retry_session")
        );

        let rebind_failed = tracker
            .commit_observation(TargetObservation::VisibilityChanged {
                visibility_state: TargetVisibilityState::Visible,
                target_geometry_revision: 9,
                observed_at_ms: 60,
            })
            .expect("missing rebind policy must produce an explicit failure");

        assert_eq!(rebind_failed.event_type(), "TARGET_REBIND_FAILED");
        assert_eq!(
            rebind_failed.payload()["reason_code"],
            json!("explicit_rebind_required")
        );
        assert_eq!(
            rebind_failed.payload()["frontend_action"],
            json!("refresh_targets")
        );
        assert_eq!(rebind_failed.payload()["failure_domain"], json!("target"));
        assert_eq!(rebind_failed.payload()["target_status"], json!("lost"));
        assert_eq!(rebind_failed.payload()["input_enabled"], json!(false));
        assert_eq!(
            rebind_failed.payload()["previous_target_geometry_revision"],
            rebind_failed.payload()["target_geometry_revision"]
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["failure_domain"],
            json!("target")
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["frontend_action"],
            json!("refresh_targets")
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["recoverability"],
            json!("new_session_required")
        );
        assert_eq!(tracker.snapshot().to_value()["status"], json!("lost"));
        assert!(tracker
            .commit_observation(TargetObservation::VisibilityChanged {
                visibility_state: TargetVisibilityState::Visible,
                target_geometry_revision: 9,
                observed_at_ms: 70,
            })
            .is_none());
    }

    #[test]
    fn tracker_routes_post_loss_title_focus_through_explicit_rebind() {
        let mut title_tracker = lost_window_tracker();

        let title_rebind_attempted = title_tracker
            .commit_observation(TargetObservation::TitleChanged {
                title: Some("Cursor reopened".to_string()),
                observed_at_ms: 30,
            })
            .expect("post-loss title observation enters explicit rebind");
        assert_eq!(
            title_rebind_attempted.event_type(),
            "TARGET_REBIND_ATTEMPTED"
        );
        assert_eq!(
            title_rebind_attempted.payload()["detail"],
            json!("target_title_after_loss")
        );
        assert_eq!(
            title_rebind_attempted.payload()["failure_domain"],
            json!("target")
        );

        let title_rebind_failed = title_tracker
            .commit_observation(TargetObservation::TitleChanged {
                title: Some("Cursor reopened again".to_string()),
                observed_at_ms: 40,
            })
            .expect("second title observation fails closed without rebind policy");
        assert_eq!(title_rebind_failed.event_type(), "TARGET_REBIND_FAILED");
        assert_eq!(
            title_rebind_failed.payload()["reason_code"],
            json!("explicit_rebind_required")
        );
        assert_eq!(
            title_rebind_failed.payload()["failure_domain"],
            json!("target")
        );
        assert_eq!(
            title_tracker.snapshot().to_value()["input_enabled"],
            json!(false)
        );

        let mut focus_tracker = lost_window_tracker();
        let focus_rebind_attempted = focus_tracker
            .commit_observation(TargetObservation::FocusChanged {
                focused: true,
                observed_at_ms: 30,
            })
            .expect("post-loss focus observation enters explicit rebind");
        assert_eq!(
            focus_rebind_attempted.event_type(),
            "TARGET_REBIND_ATTEMPTED"
        );
        assert_eq!(
            focus_rebind_attempted.payload()["detail"],
            json!("target_focus_after_loss")
        );
        assert_eq!(
            focus_rebind_attempted.payload()["failure_domain"],
            json!("target")
        );

        let focus_rebind_failed = focus_tracker
            .commit_observation(TargetObservation::FocusChanged {
                focused: false,
                observed_at_ms: 40,
            })
            .expect("second focus observation fails closed without rebind policy");
        assert_eq!(focus_rebind_failed.event_type(), "TARGET_REBIND_FAILED");
        assert_eq!(
            focus_rebind_failed.payload()["frontend_action"],
            json!("refresh_targets")
        );
        assert_eq!(
            focus_rebind_failed.payload()["failure_domain"],
            json!("target")
        );
        assert_eq!(
            focus_tracker.snapshot().to_value()["input_enabled"],
            json!(false)
        );
    }

    #[test]
    fn display_topology_loss_projects_target_failure_recovery() {
        let binding = window_binding();
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);

        let topology_changed = tracker
            .commit_observation(TargetObservation::DisplayTopologyChanged {
                available_display_ids: vec![99, 42, 42],
                selected_display_available: false,
                observed_at_ms: 30,
            })
            .expect("selected display loss emits topology event");

        assert_eq!(topology_changed.event_type(), "DISPLAY_TOPOLOGY_CHANGED");
        assert_eq!(
            topology_changed.payload()["reason_code"],
            json!("target_display_unavailable")
        );
        assert_eq!(
            topology_changed.payload()["detail"],
            json!("display_topology_changed")
        );
        assert_eq!(
            topology_changed.payload()["failure_domain"],
            json!("target")
        );
        assert_eq!(
            topology_changed.payload()["frontend_action"],
            json!("show_unsupported")
        );
        assert_eq!(topology_changed.payload()["target_status"], json!("stale"));
        assert_eq!(topology_changed.payload()["input_enabled"], json!(false));
        assert_eq!(
            topology_changed.payload()["input_blocked_reason"],
            json!("target_display_unavailable")
        );
        assert_eq!(
            topology_changed.payload()["selected_display_available"],
            json!(false)
        );
        assert_eq!(
            topology_changed.payload()["available_display_ids"],
            json!([42, 99])
        );
        assert_eq!(tracker.snapshot().to_value()["status"], json!("stale"));
        assert_eq!(tracker.snapshot().to_value()["input_enabled"], json!(false));
        assert_eq!(
            tracker.snapshot().to_value()["input_blocked_reason"],
            json!("target_display_unavailable")
        );

        let topology_restored = tracker
            .commit_observation(TargetObservation::DisplayTopologyChanged {
                available_display_ids: vec![42, 99],
                selected_display_available: true,
                observed_at_ms: 40,
            })
            .expect("selected display recovery emits topology event even when display ids match");

        assert_eq!(topology_restored.event_type(), "DISPLAY_TOPOLOGY_CHANGED");
        assert_eq!(
            topology_restored.payload()["reason_code"],
            json!("display_topology_changed")
        );
        assert_eq!(topology_restored.payload()["input_enabled"], json!(true));
        assert_eq!(
            topology_restored.payload()["input_blocked_reason"],
            Value::Null
        );
        assert_eq!(
            tracker.snapshot().to_value()["input_blocked_reason"],
            Value::Null
        );
    }

    #[test]
    fn tracker_disables_input_for_hidden_or_minimized_targets() {
        let binding = window_binding();
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);

        let minimized = tracker
            .commit_observation(TargetObservation::VisibilityChanged {
                visibility_state: TargetVisibilityState::Minimized,
                target_geometry_revision: 4,
                observed_at_ms: 10,
            })
            .expect("visibility change commits");

        assert_eq!(minimized.event_type(), "TARGET_MINIMIZED");
        assert_eq!(
            minimized.payload()["reason_code"],
            json!("target_minimized")
        );
        assert_eq!(minimized.payload()["failure_domain"], json!("target"));
        assert_eq!(
            minimized.payload()["frontend_action"],
            json!("retry_session")
        );
        assert_eq!(minimized.payload()["input_enabled"], json!(false));
        assert_eq!(
            minimized.payload()["input_blocked_reason"],
            json!("target_minimized")
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["frontend_action"],
            json!("retry_session")
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["failure_domain"],
            json!("target")
        );

        assert_eq!(tracker.snapshot().to_value()["status"], json!("stale"));
        assert_eq!(
            tracker.snapshot().to_value()["input_blocked_reason"],
            json!("target_minimized")
        );
        assert!(tracker.snapshot().pointer_target_value().is_none());

        let hidden = tracker
            .commit_observation(TargetObservation::VisibilityChanged {
                visibility_state: TargetVisibilityState::Hidden,
                target_geometry_revision: 5,
                observed_at_ms: 20,
            })
            .expect("hidden visibility change commits");

        assert_eq!(hidden.event_type(), "TARGET_HIDDEN");
        assert_eq!(hidden.payload()["reason_code"], json!("target_hidden"));
        assert_eq!(hidden.payload()["failure_domain"], json!("target"));
        assert_eq!(hidden.payload()["frontend_action"], json!("retry_session"));
        assert_eq!(hidden.payload()["input_enabled"], json!(false));
        assert_eq!(
            hidden.payload()["input_blocked_reason"],
            json!("target_hidden")
        );
    }

    #[test]
    fn tracker_disables_input_when_target_loses_focus() {
        let binding = window_binding();
        let mut tracker = RemoteAppTargetBindingStateMachine::from_binding(binding);

        assert_eq!(tracker.snapshot().to_value()["input_enabled"], json!(true));

        let blurred = tracker
            .commit_observation(TargetObservation::FocusChanged {
                focused: false,
                observed_at_ms: 10,
            })
            .expect("focus loss commits");

        assert_eq!(blurred.event_type(), "TARGET_BLURRED");
        assert_eq!(blurred.payload()["reason_code"], json!("target_blurred"));
        assert_eq!(blurred.payload()["failure_domain"], json!("target"));
        assert_eq!(blurred.payload()["frontend_action"], json!("retry_session"));
        assert_eq!(blurred.payload()["input_enabled"], json!(false));
        assert_eq!(
            blurred.payload()["input_blocked_reason"],
            json!("target_blurred")
        );
        assert_eq!(tracker.snapshot().to_value()["focused"], json!(false));
        assert_eq!(
            tracker.snapshot().to_value()["target_focus_epoch"],
            json!(2)
        );
        assert_eq!(tracker.snapshot().to_value()["input_enabled"], json!(false));
        assert_eq!(
            tracker.snapshot().to_value()["input_blocked_reason"],
            json!("target_blurred")
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["failure_domain"],
            json!("target")
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["frontend_action"],
            json!("retry_session")
        );
        assert!(tracker.snapshot().pointer_target_value().is_none());

        let focused = tracker
            .commit_observation(TargetObservation::FocusChanged {
                focused: true,
                observed_at_ms: 20,
            })
            .expect("focus recovery commits");

        assert_eq!(focused.event_type(), "TARGET_FOCUSED");
        assert_eq!(focused.payload()["target_focus_epoch"], json!(3));
        assert_eq!(focused.payload()["frontend_action"], Value::Null);
        assert_eq!(focused.payload()["input_blocked_reason"], Value::Null);
        assert_eq!(tracker.snapshot().to_value()["focused"], json!(true));
        assert_eq!(
            tracker.snapshot().to_value()["target_focus_epoch"],
            json!(3)
        );
        assert_eq!(tracker.snapshot().to_value()["input_enabled"], json!(true));
        assert_eq!(
            tracker.snapshot().to_value()["input_blocked_reason"],
            Value::Null
        );
        assert!(tracker.snapshot().pointer_target_value().is_some());
    }
}
