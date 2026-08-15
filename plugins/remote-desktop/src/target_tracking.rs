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
    FrontendAction, RemoteAppTargetBinding, TargetGeometry, TargetResolutionError,
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

    fn input_enabled(self) -> bool {
        matches!(self, Self::Resolved)
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

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetTrackerSnapshot {
    binding_id: String,
    binding_epoch: u64,
    target_identity_epoch: u64,
    target_geometry_revision: u64,
    media_source_epoch: u64,
    status: TargetBindingPhase,
    visibility_state: TargetVisibilityState,
    title: Option<String>,
    focused: Option<bool>,
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
            media_source_epoch: binding.media_source_epoch(),
            status: TargetBindingPhase::Resolved,
            visibility_state: TargetVisibilityState::Visible,
            title: binding.native_locator().title().map(str::to_string),
            focused: None,
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
            "media_source_epoch": self.media_source_epoch,
            "status": self.status.as_str(),
            "visibility_state": self.visibility_state.as_str(),
            "title": self.title,
            "focused": self.focused,
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
            "origin_x": origin_x,
            "origin_y": origin_y,
            "width": self.geometry.width,
            "height": self.geometry.height,
        }))
    }

    pub(in crate::daemon::plugins::remote_desktop) fn input_enabled(&self) -> bool {
        self.status.input_enabled()
            && self.visibility_state.input_enabled()
            && self.focused != Some(false)
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
pub(in crate::daemon::plugins::remote_desktop) struct TargetTrackingEvent {
    event_type: &'static str,
    payload: Value,
}

impl TargetTrackingEvent {
    pub(in crate::daemon::plugins::remote_desktop) fn event_type(&self) -> &'static str {
        self.event_type
    }

    pub(in crate::daemon::plugins::remote_desktop) fn payload(&self) -> Value {
        self.payload.clone()
    }
}

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteAppTargetBindingStateMachine {
    binding: RemoteAppTargetBinding,
    snapshot: TargetTrackerSnapshot,
    pending_lost: Option<PendingLostObservation>,
    latest_loss_observed_at_ms: Option<u64>,
    rebind_started_at_ms: Option<u64>,
    rebind_failure_emitted: bool,
}

#[derive(Debug, Clone)]
struct PendingLostObservation {
    reason: TargetResolutionError,
    detail: String,
    first_observed_at_ms: u64,
    latest_observed_at_ms: u64,
    consecutive_misses: u32,
}

impl RemoteAppTargetBindingStateMachine {
    pub(in crate::daemon::plugins::remote_desktop) fn from_binding(
        binding: RemoteAppTargetBinding,
    ) -> Self {
        Self {
            snapshot: TargetTrackerSnapshot::from_binding(&binding),
            binding,
            pending_lost: None,
            latest_loss_observed_at_ms: None,
            rebind_started_at_ms: None,
            rebind_failure_emitted: false,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn binding(&self) -> &RemoteAppTargetBinding {
        &self.binding
    }

    pub(in crate::daemon::plugins::remote_desktop) fn snapshot(&self) -> &TargetTrackerSnapshot {
        &self.snapshot
    }

    pub(in crate::daemon::plugins::remote_desktop) fn commit_observation(
        &mut self,
        observation: TargetObservation,
    ) -> Option<TargetTrackingEvent> {
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
    ) -> Option<TargetTrackingEvent> {
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
        let event_type = geometry_event_type(&self.snapshot.geometry, &geometry);
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
        Some(TargetTrackingEvent {
            event_type,
            payload: self.event_payload("target_geometry_changed", observed_at_ms, Some(previous)),
        })
    }

    fn commit_visibility(
        &mut self,
        visibility_state: TargetVisibilityState,
        target_geometry_revision: u64,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEvent> {
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
        Some(TargetTrackingEvent {
            event_type: match visibility_state {
                TargetVisibilityState::Visible if !was_visible => "TARGET_RESTORED",
                TargetVisibilityState::Visible => "TARGET_VISIBLE",
                TargetVisibilityState::Hidden => "TARGET_HIDDEN",
                TargetVisibilityState::Minimized => "TARGET_MINIMIZED",
                TargetVisibilityState::Lost => "TARGET_LOST",
            },
            payload,
        })
    }

    fn commit_title(
        &mut self,
        title: Option<String>,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEvent> {
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
        Some(TargetTrackingEvent {
            event_type: "TARGET_TITLE_CHANGED",
            payload,
        })
    }

    fn commit_focus(&mut self, focused: bool, observed_at_ms: u64) -> Option<TargetTrackingEvent> {
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
        self.snapshot.focused = Some(focused);
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
        Some(TargetTrackingEvent {
            event_type: if focused {
                "TARGET_FOCUSED"
            } else {
                "TARGET_BLURRED"
            },
            payload,
        })
    }

    fn commit_permission_revoked(
        &mut self,
        detail: String,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEvent> {
        if self.snapshot.status == TargetBindingPhase::Invalidated {
            return None;
        }
        self.clear_pending_lost();
        self.snapshot.status = TargetBindingPhase::Invalidated;
        self.snapshot.visibility_state = TargetVisibilityState::Lost;
        self.snapshot.focused = Some(false);
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
        Some(TargetTrackingEvent {
            event_type: "TARGET_PERMISSION_REVOKED",
            payload: target_failure_payload(
                payload,
                TargetResolutionError::TargetPermissionMissing
                    .frontend_action()
                    .as_str(),
            ),
        })
    }

    fn commit_display_topology(
        &mut self,
        mut available_display_ids: Vec<u64>,
        selected_display_available: bool,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEvent> {
        available_display_ids.sort_unstable();
        available_display_ids.dedup();
        if self.snapshot.available_display_ids == available_display_ids
            && (selected_display_available || self.snapshot.status == TargetBindingPhase::Stale)
        {
            return None;
        }
        let previous_display_ids = std::mem::replace(
            &mut self.snapshot.available_display_ids,
            available_display_ids,
        );
        if selected_display_available {
            self.snapshot.status = TargetBindingPhase::Resolved;
        } else {
            self.snapshot.status = TargetBindingPhase::Stale;
            self.snapshot.focused = Some(false);
        }
        self.snapshot.diagnostic = self.diagnostic_projection(
            self.snapshot.status.as_str(),
            json!(TargetResolutionError::TargetDisplayUnavailable.as_str()),
            "display_topology_changed",
            observed_at_ms,
        );
        let reason = if selected_display_available {
            "display_topology_changed"
        } else {
            TargetResolutionError::TargetDisplayUnavailable.as_str()
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
        Some(TargetTrackingEvent {
            event_type: "DISPLAY_TOPOLOGY_CHANGED",
            payload,
        })
    }

    fn commit_lost(
        &mut self,
        reason: TargetResolutionError,
        detail: String,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEvent> {
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
        Some(TargetTrackingEvent {
            event_type: "TARGET_LOST",
            payload: target_failure_payload(payload, pending.reason.frontend_action().as_str()),
        })
    }

    fn begin_rebinding(
        &mut self,
        detail: &'static str,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEvent> {
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
        let payload = target_failure_payload(
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
        );
        Some(TargetTrackingEvent {
            event_type: "TARGET_REBIND_ATTEMPTED",
            payload,
        })
    }

    fn commit_rebind_failed(
        &mut self,
        detail: &'static str,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEvent> {
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
        let payload = target_failure_payload(
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
        );
        Some(TargetTrackingEvent {
            event_type: "TARGET_REBIND_FAILED",
            payload,
        })
    }

    fn clear_pending_lost(&mut self) {
        self.pending_lost = None;
    }

    fn pending_lost_diagnostic(&self, pending: &PendingLostObservation) -> Value {
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
            "recoverability": self.snapshot.status.recoverability(),
            "frontend_action": pending.reason.frontend_action().as_str(),
            "lost_debounce": {
                "state": "pending",
                "required_misses": LOST_DEBOUNCE_REQUIRED_MISSES,
                "required_elapsed_ms": LOST_DEBOUNCE_MS,
                "first_observed_at_ms": pending.first_observed_at_ms,
                "latest_observed_at_ms": pending.latest_observed_at_ms,
                "consecutive_misses": pending.consecutive_misses,
            },
            "observed_at_ms": pending.latest_observed_at_ms,
        })
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
        json!({
            "subject_ura": self.binding.subject_ura(),
            "binding_id": self.snapshot.binding_id,
            "binding_epoch": self.snapshot.binding_epoch,
            "previous_target_identity_epoch": self.snapshot.target_identity_epoch,
            "target_identity_epoch": self.snapshot.target_identity_epoch,
            "previous_target_geometry_revision": previous_target_geometry_revision,
            "target_geometry_revision": self.snapshot.target_geometry_revision,
            "media_source_epoch": self.snapshot.media_source_epoch,
            "visibility_state": self.snapshot.visibility_state.as_str(),
            "target_status": self.snapshot.status.as_str(),
            "input_enabled": self.snapshot.input_enabled(),
            "reason_code": reason_code,
            "recoverability": self.snapshot.status.recoverability(),
            "frontend_action": Value::Null,
            "observed_at_ms": observed_at_ms,
            "geometry": self.snapshot.geometry.to_value(),
        })
    }
}

fn target_failure_payload(mut payload: Value, frontend_action: &str) -> Value {
    payload["failure_domain"] = json!("target");
    payload["input_enabled"] = json!(false);
    payload["frontend_action"] = json!(frontend_action);
    payload
}

fn geometry_event_type(previous: &TargetGeometry, next: &TargetGeometry) -> &'static str {
    if previous.width != next.width || previous.height != next.height {
        "TARGET_RESIZED"
    } else {
        "TARGET_MOVED"
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use crate::daemon::plugins::remote_desktop::target::{
        RemoteAppTargetResolver, ResourceEntryTargetResolver, TargetGeometry, TargetResolutionError,
    };
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        RemoteAppTargetBindingStateMachine, TargetObservation, TargetVisibilityState,
    };
    use crate::daemon::plugins::remote_desktop::test_support::live_remote_target_metadata;

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
        assert_eq!(snapshot["status"], json!("resolved"));
        assert_eq!(snapshot["visibility_state"], json!("visible"));
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
                observed_at_ms: 20,
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
        assert_eq!(tracker.snapshot().to_value()["status"], json!("lost"));
        assert!(tracker.snapshot().pointer_target_value().is_none());
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

        assert!(tracker
            .commit_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "second transient snapshot miss".into(),
                observed_at_ms: 30,
            })
            .is_none());
        assert_eq!(tracker.snapshot().to_value()["status"], json!("resolved"));
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
            topology_changed.payload()["selected_display_available"],
            json!(false)
        );
        assert_eq!(
            topology_changed.payload()["available_display_ids"],
            json!([42, 99])
        );
        assert_eq!(tracker.snapshot().to_value()["status"], json!("stale"));
        assert_eq!(tracker.snapshot().to_value()["input_enabled"], json!(false));
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
            tracker.snapshot().latest_diagnostic()["frontend_action"],
            json!("retry_session")
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["failure_domain"],
            json!("target")
        );

        assert_eq!(tracker.snapshot().to_value()["status"], json!("stale"));
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
        assert_eq!(tracker.snapshot().to_value()["focused"], json!(false));
        assert_eq!(tracker.snapshot().to_value()["input_enabled"], json!(false));
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
        assert_eq!(focused.payload()["frontend_action"], Value::Null);
        assert_eq!(tracker.snapshot().to_value()["focused"], json!(true));
        assert_eq!(tracker.snapshot().to_value()["input_enabled"], json!(true));
        assert!(tracker.snapshot().pointer_target_value().is_some());
    }
}
