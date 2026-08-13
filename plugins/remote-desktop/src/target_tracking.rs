// EasyNet CLI — remote desktop target tracking state
// ==================================================
//
// File: plugins/remote-desktop/src/target_tracking.rs
// Description: Session-owned target tracking state for app/window/display
// remote desktop sessions.
//
// Boundary:
// - TargetTrackerState is not a platform poller and does not mutate resource
//   inventory. It is the session aggregate's committed view of target
//   observations.
// - Platform trackers such as macOS CGWindowList/ScreenCaptureKit diff loops
//   submit TargetObservation values. The session aggregate remains the single
//   writer for state transitions and ordered event-log rows.

use serde_json::{Value, json};

use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, TargetGeometry, TargetResolutionError,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum TargetTrackingStatus {
    Resolved,
    Stale,
    Lost,
    Invalidated,
}

impl TargetTrackingStatus {
    pub(in crate::daemon::plugins::remote_desktop) fn as_str(self) -> &'static str {
        debug_assert!(ALL_TARGET_TRACKING_STATUSES.contains(&self));
        match self {
            Self::Resolved => "resolved",
            Self::Stale => "stale",
            Self::Lost => "lost",
            Self::Invalidated => "invalidated",
        }
    }

    fn recoverability(self) -> &'static str {
        match self {
            Self::Resolved => "continue",
            Self::Stale => "refresh_required",
            Self::Lost => "terminate",
            Self::Invalidated => "terminate",
        }
    }

    fn input_enabled(self) -> bool {
        matches!(self, Self::Resolved)
    }
}

const ALL_TARGET_TRACKING_STATUSES: &[TargetTrackingStatus] = &[
    TargetTrackingStatus::Resolved,
    TargetTrackingStatus::Stale,
    TargetTrackingStatus::Lost,
    TargetTrackingStatus::Invalidated,
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

#[derive(Debug, Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct TargetTrackerSnapshot {
    binding_id: String,
    binding_epoch: u64,
    target_identity_epoch: u64,
    target_geometry_revision: u64,
    media_source_epoch: u64,
    status: TargetTrackingStatus,
    visibility_state: TargetVisibilityState,
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
            status: TargetTrackingStatus::Resolved,
            visibility_state: TargetVisibilityState::Visible,
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
        self.status.input_enabled() && self.visibility_state.input_enabled()
    }

    pub(in crate::daemon::plugins::remote_desktop) fn target_geometry_revision(&self) -> u64 {
        self.target_geometry_revision
    }

    pub(in crate::daemon::plugins::remote_desktop) fn geometry(&self) -> &TargetGeometry {
        &self.geometry
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
pub(in crate::daemon::plugins::remote_desktop) struct TargetTrackerState {
    snapshot: TargetTrackerSnapshot,
    pending_lost: Option<PendingLostObservation>,
    latest_loss_observed_at_ms: Option<u64>,
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

impl TargetTrackerState {
    pub(in crate::daemon::plugins::remote_desktop) fn from_binding(
        binding: &RemoteAppTargetBinding,
    ) -> Self {
        Self {
            snapshot: TargetTrackerSnapshot::from_binding(binding),
            pending_lost: None,
            latest_loss_observed_at_ms: None,
            rebind_failure_emitted: false,
        }
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
        if self.snapshot.status == TargetTrackingStatus::Lost {
            return self.commit_rebind_failed("target_geometry_after_loss", observed_at_ms);
        }
        self.clear_pending_lost();
        let previous = self.snapshot.target_geometry_revision;
        if target_geometry_revision <= previous && geometry == self.snapshot.geometry {
            return None;
        }
        let event_type = geometry_event_type(&self.snapshot.geometry, &geometry);
        self.snapshot.status = TargetTrackingStatus::Resolved;
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
        if self.snapshot.status == TargetTrackingStatus::Lost {
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
        let previous = self.snapshot.target_geometry_revision;
        self.snapshot.visibility_state = visibility_state;
        self.snapshot.status = match visibility_state {
            TargetVisibilityState::Visible => TargetTrackingStatus::Resolved,
            TargetVisibilityState::Hidden | TargetVisibilityState::Minimized => {
                TargetTrackingStatus::Stale
            }
            TargetVisibilityState::Lost => TargetTrackingStatus::Lost,
        };
        self.snapshot.target_geometry_revision = target_geometry_revision.max(previous + 1);
        let reason = match visibility_state {
            TargetVisibilityState::Visible => "target_visible",
            TargetVisibilityState::Hidden => "target_hidden",
            TargetVisibilityState::Minimized => "target_minimized",
            TargetVisibilityState::Lost => "target_lost",
        };
        self.snapshot.diagnostic = self.diagnostic_projection(
            self.snapshot.status.as_str(),
            json!(reason),
            reason,
            observed_at_ms,
        );
        Some(TargetTrackingEvent {
            event_type: match visibility_state {
                TargetVisibilityState::Visible => "TARGET_VISIBLE",
                TargetVisibilityState::Hidden => "TARGET_HIDDEN",
                TargetVisibilityState::Minimized => "TARGET_MINIMIZED",
                TargetVisibilityState::Lost => "TARGET_LOST",
            },
            payload: self.event_payload(reason, observed_at_ms, Some(previous)),
        })
    }

    fn commit_lost(
        &mut self,
        reason: TargetResolutionError,
        detail: String,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEvent> {
        if self.snapshot.status == TargetTrackingStatus::Lost {
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
        self.snapshot.status = TargetTrackingStatus::Lost;
        self.snapshot.visibility_state = TargetVisibilityState::Lost;
        self.latest_loss_observed_at_ms = Some(observed_at_ms);
        self.snapshot.diagnostic = json!({
            "status": TargetTrackingStatus::Lost.as_str(),
            "reason": pending.reason.as_str(),
            "detail": pending.detail,
            "binding_id": self.snapshot.binding_id,
            "binding_epoch": self.snapshot.binding_epoch,
            "target_identity_epoch": self.snapshot.target_identity_epoch,
            "target_geometry_revision": self.snapshot.target_geometry_revision,
            "visibility_state": self.snapshot.visibility_state.as_str(),
            "recoverability": TargetTrackingStatus::Lost.recoverability(),
            "frontend_action": pending.reason.frontend_action().as_str(),
            "lost_debounce": {
                "first_observed_at_ms": pending.first_observed_at_ms,
                "latest_observed_at_ms": pending.latest_observed_at_ms,
                "consecutive_misses": pending.consecutive_misses,
            },
            "observed_at_ms": observed_at_ms,
        });
        Some(TargetTrackingEvent {
            event_type: "TARGET_LOST",
            payload: self.event_payload(pending.reason.as_str(), observed_at_ms, Some(previous)),
        })
    }

    fn commit_rebind_failed(
        &mut self,
        detail: &'static str,
        observed_at_ms: u64,
    ) -> Option<TargetTrackingEvent> {
        if self
            .latest_loss_observed_at_ms
            .is_some_and(|lost_observed_at_ms| observed_at_ms <= lost_observed_at_ms)
        {
            return None;
        }
        if self.rebind_failure_emitted {
            return None;
        }
        self.rebind_failure_emitted = true;
        let reason_code = "explicit_rebind_required";
        self.snapshot.diagnostic = json!({
            "status": TargetTrackingStatus::Lost.as_str(),
            "reason": reason_code,
            "detail": detail,
            "binding_id": self.snapshot.binding_id,
            "binding_epoch": self.snapshot.binding_epoch,
            "target_identity_epoch": self.snapshot.target_identity_epoch,
            "target_geometry_revision": self.snapshot.target_geometry_revision,
            "visibility_state": self.snapshot.visibility_state.as_str(),
            "target_status": TargetTrackingStatus::Lost.as_str(),
            "input_enabled": false,
            "recoverability": "new_session_required",
            "frontend_action": "refresh_targets",
            "observed_at_ms": observed_at_ms,
        });
        Some(TargetTrackingEvent {
            event_type: "TARGET_REBIND_FAILED",
            payload: json!({
                "binding_id": self.snapshot.binding_id,
                "binding_epoch": self.snapshot.binding_epoch,
                "target_identity_epoch": self.snapshot.target_identity_epoch,
                "previous_target_geometry_revision": self.snapshot.target_geometry_revision,
                "target_geometry_revision": self.snapshot.target_geometry_revision,
                "media_source_epoch": self.snapshot.media_source_epoch,
                "visibility_state": self.snapshot.visibility_state.as_str(),
                "target_status": TargetTrackingStatus::Lost.as_str(),
                "input_enabled": false,
                "reason_code": reason_code,
                "detail": detail,
                "recoverability": "new_session_required",
                "frontend_action": "refresh_targets",
                "observed_at_ms": observed_at_ms,
                "geometry": self.snapshot.geometry.to_value(),
            }),
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
            "binding_id": self.snapshot.binding_id,
            "binding_epoch": self.snapshot.binding_epoch,
            "target_identity_epoch": self.snapshot.target_identity_epoch,
            "previous_target_geometry_revision": previous_target_geometry_revision,
            "target_geometry_revision": self.snapshot.target_geometry_revision,
            "media_source_epoch": self.snapshot.media_source_epoch,
            "visibility_state": self.snapshot.visibility_state.as_str(),
            "reason_code": reason_code,
            "recoverability": self.snapshot.status.recoverability(),
            "observed_at_ms": observed_at_ms,
            "geometry": self.snapshot.geometry.to_value(),
        })
    }
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
    use serde_json::json;

    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use crate::daemon::plugins::remote_desktop::target::{
        RemoteAppTargetResolver, ResourceEntryTargetResolver, TargetGeometry, TargetResolutionError,
    };
    use crate::daemon::plugins::remote_desktop::target_tracking::{
        TargetObservation, TargetTrackerState, TargetVisibilityState,
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
                    metadata: json!({
                        "window_id": 42,
                        "pid": 10,
                        "app_name": "Cursor",
                        "x": 100,
                        "y": 200,
                        "width": 800,
                        "height": 600,
                        "target_identity_epoch": 7,
                        "geometry_revision": 3,
                    }),
                    first_seen_at: "2026-06-01T00:00:00Z".into(),
                },
                "view_only",
                1,
            )
            .expect("window target binding resolves")
    }

    #[test]
    fn tracker_snapshot_starts_from_session_binding() {
        let binding = window_binding();
        let tracker = TargetTrackerState::from_binding(&binding);
        let snapshot = tracker.snapshot().to_value();

        assert_eq!(snapshot["binding_id"], json!(binding.binding_id()));
        assert_eq!(snapshot["target_identity_epoch"], json!(7));
        assert_eq!(snapshot["target_geometry_revision"], json!(3));
        assert_eq!(snapshot["status"], json!("resolved"));
        assert_eq!(snapshot["visibility_state"], json!("visible"));
    }

    #[test]
    fn tracker_commits_move_resize_and_lost_without_rebinding() {
        let binding = window_binding();
        let mut tracker = TargetTrackerState::from_binding(&binding);

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
        assert_eq!(tracker.snapshot().to_value()["status"], json!("lost"));
        assert!(tracker.snapshot().pointer_target_value().is_none());
    }

    #[test]
    fn tracker_debounces_single_transient_lost_observation() {
        let binding = window_binding();
        let mut tracker = TargetTrackerState::from_binding(&binding);

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
        let mut tracker = TargetTrackerState::from_binding(&binding);

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
        let binding = window_binding();
        let mut tracker = TargetTrackerState::from_binding(&binding);

        assert!(
            tracker
                .commit_observation(TargetObservation::Lost {
                    reason: TargetResolutionError::TargetNotFound,
                    detail: "window disappeared".into(),
                    observed_at_ms: 10,
                })
                .is_none()
        );
        tracker
            .commit_observation(TargetObservation::Lost {
                reason: TargetResolutionError::TargetNotFound,
                detail: "window still missing".into(),
                observed_at_ms: 20,
            })
            .expect("lost commits after debounce");

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
        assert!(
            tracker
                .commit_observation(TargetObservation::Lost {
                    reason: TargetResolutionError::TargetNotFound,
                    detail: "window still lost".into(),
                    observed_at_ms: 40,
                })
                .is_none()
        );
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

        let rebind_failed = tracker
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
            .expect("post-loss target observation must produce explicit rebind failure");

        assert_eq!(rebind_failed.event_type(), "TARGET_REBIND_FAILED");
        assert_eq!(
            rebind_failed.payload()["reason_code"],
            json!("explicit_rebind_required")
        );
        assert_eq!(
            rebind_failed.payload()["frontend_action"],
            json!("refresh_targets")
        );
        assert_eq!(rebind_failed.payload()["target_status"], json!("lost"));
        assert_eq!(rebind_failed.payload()["input_enabled"], json!(false));
        assert_eq!(
            rebind_failed.payload()["previous_target_geometry_revision"],
            rebind_failed.payload()["target_geometry_revision"]
        );
        assert_eq!(
            tracker.snapshot().latest_diagnostic()["recoverability"],
            json!("new_session_required")
        );
        assert_eq!(tracker.snapshot().to_value()["status"], json!("lost"));
        assert!(
            tracker
                .commit_observation(TargetObservation::VisibilityChanged {
                    visibility_state: TargetVisibilityState::Visible,
                    target_geometry_revision: 9,
                    observed_at_ms: 60,
                })
                .is_none()
        );
    }

    #[test]
    fn tracker_disables_input_for_hidden_or_minimized_targets() {
        let binding = window_binding();
        let mut tracker = TargetTrackerState::from_binding(&binding);

        tracker
            .commit_observation(TargetObservation::VisibilityChanged {
                visibility_state: TargetVisibilityState::Minimized,
                target_geometry_revision: 4,
                observed_at_ms: 10,
            })
            .expect("visibility change commits");

        assert_eq!(tracker.snapshot().to_value()["status"], json!("stale"));
        assert!(tracker.snapshot().pointer_target_value().is_none());
    }
}
