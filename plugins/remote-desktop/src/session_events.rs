// EasyNet CLI — remote desktop session event payloads
// ====================================================
//
// File: plugins/remote-desktop/src/session_events.rs
// Description: Canonical event payload projections for remote desktop
// sessions.

use serde_json::{json, Value};

use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_ATTACH_SESSION, TRANSPORT_INVOKE_BIDI, TRANSPORT_WEBRTC,
};
use crate::daemon::plugins::remote_desktop::target::{
    FrontendAction, RemoteAppTargetBinding, TargetResolutionError,
};
use crate::daemon::plugins::remote_desktop::transport_blocker::RemoteDesktopTransportBlocker;

#[derive(Debug, Clone, PartialEq)]
pub(in crate::daemon::plugins::remote_desktop) struct RemoteDesktopEventProjection {
    event_type: &'static str,
    payload: Value,
}

impl RemoteDesktopEventProjection {
    fn new(event_type: &'static str, payload: Value) -> Self {
        Self {
            event_type,
            payload,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn event_type(&self) -> &'static str {
        self.event_type
    }

    pub(in crate::daemon::plugins::remote_desktop) fn into_payload(self) -> Value {
        self.payload
    }

    #[cfg(test)]
    fn into_parts(self) -> (&'static str, Value) {
        (self.event_type, self.payload)
    }
}

/// Domain event used when a production WebRTC worker reaches a terminal
/// failure.
///
/// The transport worker can fail because the bound media source is no longer
/// valid, or because the WebRTC/relay transport failed while the target remains
/// valid. Keeping this choice explicit prevents the event stream from
/// collapsing SPEC-distinct recovery paths into a generic session failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::daemon::plugins::remote_desktop) enum WebRtcFailureEventKind {
    MediaSourceLost,
    TransportFailed,
}

impl WebRtcFailureEventKind {
    const fn event_type(self) -> &'static str {
        match self {
            Self::MediaSourceLost => "MEDIA_SOURCE_LOST",
            Self::TransportFailed => "TRANSPORT_FAILED",
        }
    }
}

/// Build the immutable `SESSION_CREATED` payload.
///
/// This projection module does not mutate session state and does not write to
/// the bounded event log. It only keeps event payload shape stable and
/// reviewable in one place.
pub(in crate::daemon::plugins::remote_desktop) fn session_created() -> RemoteDesktopEventProjection
{
    RemoteDesktopEventProjection::new(
        "SESSION_CREATED",
        json!({
            "transport_kind": TRANSPORT_WEBRTC,
            "media_transport_ready": false,
            "preview_ability": ABILITY_ATTACH_SESSION,
            "reason_code": "session_created",
            "recoverability": "continue",
        }),
    )
}

/// Build the target-resolution audit payload emitted when session construction
/// starts from a resolved binding rather than an unresolved ResourceEntry.
pub(in crate::daemon::plugins::remote_desktop) fn capture_target_resolved(
    binding: &RemoteAppTargetBinding,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "CAPTURE_TARGET_RESOLVED",
        json!({
            "subject_ura": binding.subject_ura(),
            "binding_id": binding.binding_id(),
            "binding_epoch": binding.binding_epoch(),
            "previous_target_identity_epoch": Value::Null,
            "target_identity_epoch": binding.target_identity_epoch(),
            "target_geometry_revision": binding.target_geometry_revision(),
            "media_source_epoch": binding.media_source_epoch(),
            "consent_epoch": binding.consent_epoch(),
            "reason_code": "capture_target_resolved",
            "recoverability": "continue",
            "target_binding": binding.to_value(),
            "scope_audit": binding.scope_audit_value(),
            "latest_target_diagnostic": binding.latest_target_diagnostic_value(),
        }),
    )
}

/// Build the target-bound event payload. The session aggregate owns the
/// binding; this event is an ordered audit projection, not a second source of
/// target truth.
pub(in crate::daemon::plugins::remote_desktop) fn target_bound(
    binding: &RemoteAppTargetBinding,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new("TARGET_BOUND", binding.target_bound_event_payload())
}

/// Build the daemon-restart recovery event emitted when a durable non-terminal
/// session snapshot is restored into the runtime aggregate.
///
/// The selected target fields intentionally mirror target-bound event
/// projection so crash/restart evidence can be tied to the selected Resource,
/// geometry revision, media source epoch, and consent epoch without requiring
/// a separate show_session snapshot.
pub(in crate::daemon::plugins::remote_desktop) fn session_rehydrated(
    binding: &RemoteAppTargetBinding,
    previous_lifecycle_state: &str,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "SESSION_REHYDRATED",
        json!({
            "subject_ura": binding.subject_ura(),
            "binding_id": binding.binding_id(),
            "binding_epoch": binding.binding_epoch(),
            "previous_target_identity_epoch": Value::Null,
            "target_identity_epoch": binding.target_identity_epoch(),
            "target_geometry_revision": binding.target_geometry_revision(),
            "media_source_epoch": binding.media_source_epoch(),
            "consent_epoch": binding.consent_epoch(),
            "reason_code": "daemon_restart_rehydrated",
            "recoverability": "retry_session",
            "failure_domain": "daemon_restart",
            "frontend_action": "retry_session",
            "media_transport_ready": false,
            "client_media_ready": false,
            "previous_lifecycle_state": previous_lifecycle_state,
            "target_binding": binding.to_value(),
            "scope_audit": binding.scope_audit_value(),
            "latest_target_diagnostic": binding.latest_target_diagnostic_value(),
        }),
    )
}

/// Build a generic SDP description-set payload.
pub(in crate::daemon::plugins::remote_desktop) fn description_set(
    side: &str,
    media_transport_ready: bool,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "DESCRIPTION_SET",
        json!({
            "side": side,
            "media_transport_ready": media_transport_ready,
        }),
    )
}

/// Build the local WebRTC answer projection.
pub(in crate::daemon::plugins::remote_desktop) fn local_webrtc_answer_set(
    backend_id: &str,
    production_ready: bool,
    transport_epoch: u64,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "DESCRIPTION_SET",
        json!({
            "transport_kind": TRANSPORT_WEBRTC,
            "side": "local",
            "media_transport_ready": false,
            "backend_id": backend_id,
            "production_ready": production_ready,
            "transport_epoch": transport_epoch,
        }),
    )
}

/// Build a remote ICE candidate append payload.
pub(in crate::daemon::plugins::remote_desktop) fn remote_ice_candidate_added(
    candidate_count: usize,
    application_state: &str,
    transport_epoch: Option<u64>,
    media_transport_ready: bool,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "ICE_CANDIDATE_ADDED",
        json!({
            "candidate_count": candidate_count,
            "application_state": application_state,
            "applied_to_live_endpoint": application_state == "applied",
            "transport_epoch": transport_epoch,
            "media_transport_ready": media_transport_ready,
        }),
    )
}

/// Build an InvokeBidi diagnostic preview-connected payload.
pub(in crate::daemon::plugins::remote_desktop) fn preview_transport_connected(
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "TRANSPORT_CONNECTED",
        json!({
            "transport_kind": TRANSPORT_INVOKE_BIDI,
            "encoding": "metadata_json_plus_binary",
            "media_transport_ready": false,
            "diagnostic_only": true,
        }),
    )
}

/// Build an InvokeBidi diagnostic preview-detached payload.
pub(in crate::daemon::plugins::remote_desktop) fn preview_transport_detached(
    reason: &str,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "TRANSPORT_DETACHED",
        json!({
            "transport_kind": TRANSPORT_INVOKE_BIDI,
            "reason": reason,
            "media_transport_ready": false,
            "diagnostic_only": true,
        }),
    )
}

/// Build an InvokeBidi diagnostic preview failure payload.
pub(in crate::daemon::plugins::remote_desktop) fn preview_transport_failed(
    reason: &str,
    message: String,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "DIAGNOSTIC_PREVIEW_FAILED",
        json!({
            "transport_kind": TRANSPORT_INVOKE_BIDI,
            "reason": reason,
            "message": message,
            "media_transport_ready": false,
            "diagnostic_only": true,
        }),
    )
}

/// Build a lease-refresh payload.
pub(in crate::daemon::plugins::remote_desktop) fn lease_refreshed(
    lease_expires_at_ms: u64,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "LEASE_REFRESHED",
        json!({ "lease_expires_at_ms": lease_expires_at_ms }),
    )
}

/// Build a transport-blocked payload.
pub(in crate::daemon::plugins::remote_desktop) fn transport_blocked(
    reason: &str,
    required_backend: &str,
) -> RemoteDesktopEventProjection {
    let blocker = RemoteDesktopTransportBlocker::from_webrtc_error(reason);
    RemoteDesktopEventProjection::new(
        "TRANSPORT_BLOCKED",
        json!({
            "transport_kind": TRANSPORT_WEBRTC,
            "reason": reason,
            "reason_code": blocker.map(RemoteDesktopTransportBlocker::reason_code_str),
            "recoverability": blocker.map(RemoteDesktopTransportBlocker::recoverability),
            "failure_domain": blocker.map(RemoteDesktopTransportBlocker::failure_domain),
            "frontend_action": blocker
                .map(RemoteDesktopTransportBlocker::frontend_action)
                .map(|action| action.as_str()),
            "required_backend": required_backend,
        }),
    )
}

/// Build a local ICE candidate payload.
pub(in crate::daemon::plugins::remote_desktop) fn local_ice_candidate(
    candidate: Value,
    candidate_count: usize,
    media_transport_ready: bool,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "LOCAL_ICE_CANDIDATE",
        json!({
            "candidate": candidate,
            "candidate_count": candidate_count,
            "media_transport_ready": media_transport_ready,
        }),
    )
}

/// Build a WebRTC diagnostic payload.
pub(in crate::daemon::plugins::remote_desktop) fn webrtc_diagnostic(
    media_transport_ready: bool,
    diagnostic: Value,
    webrtc_ice_state: Option<&str>,
    webrtc_error: Option<&str>,
) -> Value {
    json!({
        "media_transport_ready": media_transport_ready,
        "diagnostic": diagnostic,
        "webrtc_ice_state": webrtc_ice_state,
        "webrtc_error": webrtc_error,
    })
}

/// Build a WebRTC input-channel diagnostic payload.
pub(in crate::daemon::plugins::remote_desktop) fn input_channel_diagnostic(
    media_transport_ready: bool,
    diagnostic: Value,
) -> Value {
    json!({
        "transport_kind": TRANSPORT_WEBRTC,
        "input_plane": "webrtc_data_channel",
        "media_transport_ready": media_transport_ready,
        "diagnostic": diagnostic,
    })
}

pub(in crate::daemon::plugins::remote_desktop) fn input_permission_blocked(
    transport_epoch: u64,
    reason: &str,
    media_transport_ready: bool,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "INPUT_PERMISSION_BLOCKED",
        json!({
            "transport_kind": TRANSPORT_WEBRTC,
            "input_plane": "webrtc_data_channel",
            "transport_epoch": transport_epoch,
            "reason": reason,
            "input_activation": "blocked",
            "input_activation_reason": reason,
            "media_transport_ready": media_transport_ready,
            "recoverability": "request_input_permission",
            "frontend_action": FrontendAction::RequestPermission.as_str(),
        }),
    )
}

pub(in crate::daemon::plugins::remote_desktop) fn input_permission_restored(
    transport_epoch: u64,
    media_transport_ready: bool,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "INPUT_PERMISSION_RESTORED",
        json!({
            "transport_kind": TRANSPORT_WEBRTC,
            "input_plane": "webrtc_data_channel",
            "transport_epoch": transport_epoch,
            "input_activation": "enabled",
            "input_activation_reason": Value::Null,
            "media_transport_ready": media_transport_ready,
            "recoverability": "resolved",
            "frontend_action": Value::Null,
        }),
    )
}

/// Build a media-pipeline stats payload.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(in crate::daemon::plugins::remote_desktop) fn media_pipeline_stats(
    media_transport_ready: bool,
    stats: Value,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "MEDIA_PIPELINE_STATS",
        json!({
            "media_transport_ready": media_transport_ready,
            "stats": stats,
        }),
    )
}

/// Build a caller-requested closing payload.
pub(in crate::daemon::plugins::remote_desktop) fn session_closing(
    reason: &str,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "SESSION_CLOSING",
        json!({
            "reason": reason,
            "reason_code": reason,
            "recoverability": "closing",
        }),
    )
}

/// Build a caller-requested closed payload.
pub(in crate::daemon::plugins::remote_desktop) fn session_closed(
    reason: &str,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "SESSION_CLOSED",
        json!({
            "reason": reason,
            "reason_code": reason,
            "recoverability": "closed",
        }),
    )
}

/// Build a lease-expiry closed payload.
pub(in crate::daemon::plugins::remote_desktop) fn session_expired(
    reason: &str,
    lease_expires_at_ms: u64,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "SESSION_CLOSED",
        json!({
            "reason": reason,
            "reason_code": reason,
            "recoverability": "closed",
            "lease_expires_at_ms": lease_expires_at_ms,
        }),
    )
}

/// Build a production WebRTC connected payload.
pub(in crate::daemon::plugins::remote_desktop) fn webrtc_sender_ready(
    endpoint_ura: String,
    transport_epoch: u64,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "MEDIA_SENDER_READY",
        json!({
            "transport_kind": TRANSPORT_WEBRTC,
            "media_transport_ready": true,
            "client_media_ready": false,
            "transport_epoch": transport_epoch,
            "endpoint_ura": endpoint_ura,
            "codec": "h264",
            "carrier": "rtp_srtp",
        }),
    )
}

pub(in crate::daemon::plugins::remote_desktop) fn client_media_state_changed(
    state: &str,
    transport_epoch: u64,
) -> RemoteDesktopEventProjection {
    let reason_code = client_media_reason_code(state);
    RemoteDesktopEventProjection::new(
        if state == "presenting" {
            "TRANSPORT_CONNECTED"
        } else {
            "CLIENT_MEDIA_STALLED"
        },
        json!({
            "transport_kind": TRANSPORT_WEBRTC,
            "client_state": state,
            "client_media_ready": state == "presenting",
            "transport_epoch": transport_epoch,
            "reason_code": reason_code,
            "recoverability": client_media_recoverability(state),
        }),
    )
}

/// Build the non-terminal lifecycle projection emitted when the client media
/// plane stops presenting while the device media source remains selected.
pub(in crate::daemon::plugins::remote_desktop) fn session_degraded(
    client_state: &str,
    transport_epoch: u64,
    primary_phase: &str,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "SESSION_DEGRADED",
        json!({
            "reason_code": client_media_reason_code(client_state),
            "recoverability": "retry_session",
            "failure_domain": "client_media",
            "frontend_action": FrontendAction::RetrySession.as_str(),
            "transport_kind": TRANSPORT_WEBRTC,
            "transport_epoch": transport_epoch,
            "primary_phase": primary_phase,
            "client_state": client_state,
            "media_transport_ready": true,
            "client_media_ready": false,
        }),
    )
}

fn client_media_reason_code(state: &str) -> &'static str {
    match state {
        "presenting" => "client_media_presenting",
        "detached" => "client_media_detached",
        _ => "client_media_stalled",
    }
}

fn client_media_recoverability(state: &str) -> &'static str {
    if state == "presenting" {
        "continue"
    } else {
        "retry_session"
    }
}

/// Build a target-scoped media source loss event.
pub(in crate::daemon::plugins::remote_desktop) fn media_source_lost(
    binding: &RemoteAppTargetBinding,
    reason: TargetResolutionError,
    transport_epoch: u64,
) -> RemoteDesktopEventProjection {
    RemoteDesktopEventProjection::new(
        "MEDIA_SOURCE_LOST",
        json!({
            "subject_ura": binding.subject_ura(),
            "binding_id": binding.binding_id(),
            "binding_epoch": binding.binding_epoch(),
            "previous_target_identity_epoch": binding.target_identity_epoch(),
            "target_identity_epoch": binding.target_identity_epoch(),
            "target_geometry_revision": binding.target_geometry_revision(),
            "media_source_epoch": binding.media_source_epoch(),
            "consent_epoch": binding.consent_epoch(),
            "reason": reason.as_str(),
            "reason_code": reason.as_str(),
            "recoverability": "requires_target_refresh",
            "failure_domain": "target",
            "frontend_action": reason.frontend_action().as_str(),
            "transport_kind": TRANSPORT_WEBRTC,
            "media_transport_ready": false,
            "transport_epoch": transport_epoch,
        }),
    )
}

/// Build a WebRTC failure payload with typed domain context.
pub(in crate::daemon::plugins::remote_desktop) fn webrtc_failed_with_context(
    event_kind: WebRtcFailureEventKind,
    reason: &str,
    message: String,
    transport_epoch: u64,
    context: Value,
) -> RemoteDesktopEventProjection {
    let mut payload = json!({
        "reason": reason,
        "message": message,
        "transport_kind": TRANSPORT_WEBRTC,
        "media_transport_ready": false,
        "transport_epoch": transport_epoch,
    });
    if let (Some(payload), Some(context)) = (payload.as_object_mut(), context.as_object()) {
        for (key, value) in context {
            payload.insert(key.clone(), value.clone());
        }
    }
    RemoteDesktopEventProjection::new(event_kind.event_type(), payload)
}

/// Default domain context for direct WebRTC failures whose selected target is
/// still semantically valid but the production transport path failed.
pub(in crate::daemon::plugins::remote_desktop) fn webrtc_transport_failure_context() -> Value {
    json!({
        "reason_code": TargetResolutionError::TransportRouteUnavailable.as_str(),
        "recoverability": "retry_session",
        "failure_domain": "transport",
        "frontend_action": FrontendAction::RetrySession.as_str(),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::daemon::plugins::remote_desktop::constants::direct_webrtc_endpoint_ura;
    use crate::daemon::plugins::remote_desktop::target::TargetResolutionError;
    use crate::daemon::plugins::remote_desktop::test_support::test_session_init;

    use super::{
        capture_target_resolved, input_permission_blocked, input_permission_restored,
        media_source_lost, preview_transport_connected, session_closed, session_closing,
        session_created, session_degraded, session_expired, transport_blocked,
        webrtc_failed_with_context, webrtc_sender_ready, webrtc_transport_failure_context,
        WebRtcFailureEventKind,
    };

    #[test]
    fn remote_desktop_event_payloads_keep_transport_kind_explicit() {
        let (_, preview_payload) = preview_transport_connected().into_parts();
        let (_, webrtc_payload) =
            webrtc_sender_ready(direct_webrtc_endpoint_ura("session"), 1).into_parts();

        assert_eq!(preview_payload["transport_kind"], json!("invoke_bidi"));
        assert_eq!(webrtc_payload["transport_kind"], json!("webrtc"));
    }

    #[test]
    fn session_created_projects_remote_desktop_attach_as_preview_ability() {
        let (_, payload) = session_created().into_parts();

        assert_eq!(payload["preview_ability"], json!("remote_desktop.attach"));
    }

    #[test]
    fn session_closing_payload_projects_terminal_reason_code() {
        let (event_type, payload) = session_closing("caller_ended").into_parts();

        assert_eq!(event_type, "SESSION_CLOSING");
        assert_eq!(payload["reason"], json!("caller_ended"));
        assert_eq!(payload["reason_code"], json!("caller_ended"));
        assert_eq!(payload["recoverability"], json!("closing"));
    }

    #[test]
    fn session_created_payload_projects_initial_reason_code() {
        let (event_type, payload) = session_created().into_parts();

        assert_eq!(event_type, "SESSION_CREATED");
        assert_eq!(payload["transport_kind"], json!("webrtc"));
        assert_eq!(payload["media_transport_ready"], json!(false));
        assert_eq!(payload["reason_code"], json!("session_created"));
        assert_eq!(payload["recoverability"], json!("continue"));
    }

    #[test]
    fn capture_target_resolved_payload_projects_initial_binding_context() {
        let init = test_session_init(
            "rd-capture-target-resolved",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        );
        let (event_type, payload) = capture_target_resolved(&init.target_binding).into_parts();

        assert_eq!(event_type, "CAPTURE_TARGET_RESOLVED");
        assert_eq!(
            payload["subject_ura"],
            json!("easynet:///r/acme/resource/display.test")
        );
        assert_eq!(
            payload["binding_id"],
            json!(init.target_binding.binding_id())
        );
        assert_eq!(
            payload["binding_epoch"],
            json!(init.target_binding.binding_epoch())
        );
        assert_eq!(payload["previous_target_identity_epoch"], json!(null));
        assert_eq!(
            payload["target_identity_epoch"],
            json!(init.target_binding.target_identity_epoch())
        );
        assert_eq!(
            payload["target_geometry_revision"],
            json!(init.target_binding.target_geometry_revision())
        );
        assert_eq!(
            payload["media_source_epoch"],
            json!(init.target_binding.media_source_epoch())
        );
        assert_eq!(
            payload["consent_epoch"],
            json!(init.target_binding.consent_epoch())
        );
        assert_eq!(payload["reason_code"], json!("capture_target_resolved"));
        assert_eq!(payload["recoverability"], json!("continue"));
        assert_eq!(
            payload["target_binding"]["binding_id"],
            json!(init.target_binding.binding_id())
        );
    }

    #[test]
    fn session_closed_payload_projects_terminal_reason_code() {
        let (event_type, payload) = session_closed("caller_ended").into_parts();

        assert_eq!(event_type, "SESSION_CLOSED");
        assert_eq!(payload["reason"], json!("caller_ended"));
        assert_eq!(payload["reason_code"], json!("caller_ended"));
        assert_eq!(payload["recoverability"], json!("closed"));
    }

    #[test]
    fn session_expired_payload_projects_terminal_reason_code() {
        let (event_type, payload) = session_expired("session_expired", 42).into_parts();

        assert_eq!(event_type, "SESSION_CLOSED");
        assert_eq!(payload["reason"], json!("session_expired"));
        assert_eq!(payload["reason_code"], json!("session_expired"));
        assert_eq!(payload["recoverability"], json!("closed"));
        assert_eq!(payload["lease_expires_at_ms"], json!(42));
    }

    #[test]
    fn webrtc_failure_payload_preserves_typed_target_context() {
        let (event_type, payload) = webrtc_failed_with_context(
            WebRtcFailureEventKind::MediaSourceLost,
            "target_identity_changed",
            "target changed".to_string(),
            7,
            json!({
                "failure_domain": "target",
                "frontend_action": "refresh_targets",
                "binding_id": "tb_test",
            }),
        )
        .into_parts();

        assert_eq!(event_type, "MEDIA_SOURCE_LOST");
        assert_eq!(payload["reason"], json!("target_identity_changed"));
        assert_eq!(payload["message"], json!("target changed"));
        assert_eq!(payload["transport_kind"], json!("webrtc"));
        assert_eq!(payload["transport_epoch"], json!(7));
        assert_eq!(payload["failure_domain"], json!("target"));
        assert_eq!(payload["frontend_action"], json!("refresh_targets"));
        assert_eq!(payload["binding_id"], json!("tb_test"));
    }

    #[test]
    fn webrtc_failure_payload_projects_transport_failure_event() {
        let (event_type, payload) = webrtc_failed_with_context(
            WebRtcFailureEventKind::TransportFailed,
            "webrtc_peer_connection_failed",
            "peer connection failed".to_string(),
            11,
            webrtc_transport_failure_context(),
        )
        .into_parts();

        assert_eq!(event_type, "TRANSPORT_FAILED");
        assert_eq!(payload["reason"], json!("webrtc_peer_connection_failed"));
        assert_eq!(payload["transport_kind"], json!("webrtc"));
        assert_eq!(payload["transport_epoch"], json!(11));
        assert_eq!(payload["reason_code"], json!("transport_route_unavailable"));
        assert_eq!(payload["recoverability"], json!("retry_session"));
        assert_eq!(payload["failure_domain"], json!("transport"));
        assert_eq!(payload["frontend_action"], json!("retry_session"));
    }

    #[test]
    fn input_permission_block_projects_request_permission_recovery() {
        let (event_type, payload) =
            input_permission_blocked(17, "accessibility_permission_denied", true).into_parts();

        assert_eq!(event_type, "INPUT_PERMISSION_BLOCKED");
        assert_eq!(payload["transport_kind"], json!("webrtc"));
        assert_eq!(payload["input_plane"], json!("webrtc_data_channel"));
        assert_eq!(payload["transport_epoch"], json!(17));
        assert_eq!(payload["reason"], json!("accessibility_permission_denied"));
        assert_eq!(payload["input_activation"], json!("blocked"));
        assert_eq!(
            payload["input_activation_reason"],
            json!("accessibility_permission_denied")
        );
        assert_eq!(payload["media_transport_ready"], json!(true));
        assert_eq!(payload["recoverability"], json!("request_input_permission"));
        assert_eq!(payload["frontend_action"], json!("request_permission"));
    }

    #[test]
    fn input_permission_restore_projects_resolved_recovery() {
        let (event_type, payload) = input_permission_restored(18, true).into_parts();

        assert_eq!(event_type, "INPUT_PERMISSION_RESTORED");
        assert_eq!(payload["transport_kind"], json!("webrtc"));
        assert_eq!(payload["input_plane"], json!("webrtc_data_channel"));
        assert_eq!(payload["transport_epoch"], json!(18));
        assert_eq!(payload["input_activation"], json!("enabled"));
        assert_eq!(payload["input_activation_reason"], Value::Null);
        assert_eq!(payload["media_transport_ready"], json!(true));
        assert_eq!(payload["recoverability"], json!("resolved"));
        assert_eq!(payload["frontend_action"], Value::Null);
    }

    #[test]
    fn media_source_loss_projects_typed_frontend_action() {
        let init = test_session_init(
            "rd-media-source-lost",
            "easynet:///r/acme/resource/display.test",
            vec!["webrtc".into()],
        );
        let (event_type, payload) = media_source_lost(
            &init.target_binding,
            TargetResolutionError::TargetPermissionMissing,
            9,
        )
        .into_parts();

        assert_eq!(event_type, "MEDIA_SOURCE_LOST");
        assert_eq!(
            payload["subject_ura"],
            json!("easynet:///r/acme/resource/display.test")
        );
        assert_eq!(
            payload["binding_id"],
            json!(init.target_binding.binding_id())
        );
        assert_eq!(
            payload["binding_epoch"],
            json!(init.target_binding.binding_epoch())
        );
        assert_eq!(
            payload["target_identity_epoch"],
            json!(init.target_binding.target_identity_epoch())
        );
        assert_eq!(
            payload["target_geometry_revision"],
            json!(init.target_binding.target_geometry_revision())
        );
        assert_eq!(
            payload["media_source_epoch"],
            json!(init.target_binding.media_source_epoch())
        );
        assert_eq!(
            payload["consent_epoch"],
            json!(init.target_binding.consent_epoch())
        );
        assert_eq!(payload["reason"], json!("target_permission_missing"));
        assert_eq!(payload["reason_code"], json!("target_permission_missing"));
        assert_eq!(payload["recoverability"], json!("requires_target_refresh"));
        assert_eq!(payload["failure_domain"], json!("target"));
        assert_eq!(payload["frontend_action"], json!("request_permission"));
        assert_eq!(payload["transport_kind"], json!("webrtc"));
        assert_eq!(payload["media_transport_ready"], json!(false));
        assert_eq!(payload["transport_epoch"], json!(9));
    }

    #[test]
    fn session_degraded_payload_projects_recovery_context() {
        let (event_type, payload) = session_degraded("stalled", 13, "degraded").into_parts();

        assert_eq!(event_type, "SESSION_DEGRADED");
        assert_eq!(payload["reason_code"], json!("client_media_stalled"));
        assert_eq!(payload["recoverability"], json!("retry_session"));
        assert_eq!(payload["failure_domain"], json!("client_media"));
        assert_eq!(payload["frontend_action"], json!("retry_session"));
        assert_eq!(payload["transport_kind"], json!("webrtc"));
        assert_eq!(payload["transport_epoch"], json!(13));
        assert_eq!(payload["primary_phase"], json!("degraded"));
        assert_eq!(payload["client_state"], json!("stalled"));
        assert_eq!(payload["media_transport_ready"], json!(true));
        assert_eq!(payload["client_media_ready"], json!(false));
    }

    #[test]
    fn transport_blocked_projects_capture_backend_reason_code() {
        let (event_type, payload) =
            transport_blocked("webrtc_transport_backend_unavailable", "native").into_parts();

        assert_eq!(event_type, "TRANSPORT_BLOCKED");
        assert_eq!(
            payload["reason"],
            json!("webrtc_transport_backend_unavailable")
        );
        assert_eq!(payload["reason_code"], json!("capture_backend_unavailable"));
        assert_eq!(payload["recoverability"], json!("unsupported"));
        assert_eq!(payload["failure_domain"], json!("runtime"));
        assert_eq!(payload["frontend_action"], json!("show_unsupported"));
        assert_eq!(payload["transport_kind"], json!("webrtc"));
        assert_eq!(payload["required_backend"], json!("native"));
    }
}
