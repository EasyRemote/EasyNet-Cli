// EasyNet CLI — remote desktop session event payloads
// ====================================================
//
// File: plugins/remote-desktop/src/session_events.rs
// Description: Canonical event payload projections for remote desktop
// sessions.

use serde_json::{json, Value};

use crate::daemon::plugins::remote_desktop::constants::{TRANSPORT_INVOKE_BIDI, TRANSPORT_WEBRTC};
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, TargetResolutionError,
};

type RemoteDesktopEventProjection = (&'static str, Value);

/// Build the immutable `SESSION_CREATED` payload.
///
/// This projection module does not mutate session state and does not write to
/// the bounded event log. It only keeps event payload shape stable and
/// reviewable in one place.
pub(in crate::daemon::plugins::remote_desktop) fn session_created() -> RemoteDesktopEventProjection
{
    (
        "SESSION_CREATED",
        json!({
            "transport_kind": TRANSPORT_WEBRTC,
            "media_transport_ready": false,
            "preview_ability": "screen.subscribe",
        }),
    )
}

/// Build the target-resolution audit payload emitted when session construction
/// starts from a resolved binding rather than an unresolved ResourceEntry.
pub(in crate::daemon::plugins::remote_desktop) fn capture_target_resolved(
    binding: &RemoteAppTargetBinding,
) -> RemoteDesktopEventProjection {
    (
        "CAPTURE_TARGET_RESOLVED",
        json!({
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
    ("TARGET_BOUND", binding.target_bound_event_payload())
}

/// Build a generic SDP description-set payload.
pub(in crate::daemon::plugins::remote_desktop) fn description_set(
    side: &str,
    media_transport_ready: bool,
) -> RemoteDesktopEventProjection {
    (
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
    (
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
    (
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
    (
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
    (
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
    (
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
    (
        "LEASE_REFRESHED",
        json!({ "lease_expires_at_ms": lease_expires_at_ms }),
    )
}

/// Build a transport-blocked payload.
pub(in crate::daemon::plugins::remote_desktop) fn transport_blocked(
    reason: &str,
    required_backend: &str,
) -> RemoteDesktopEventProjection {
    (
        "TRANSPORT_BLOCKED",
        json!({
            "transport_kind": TRANSPORT_WEBRTC,
            "reason": reason,
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
    (
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

/// Build a media-pipeline stats payload.
#[cfg(target_os = "macos")]
pub(in crate::daemon::plugins::remote_desktop) fn media_pipeline_stats(
    media_transport_ready: bool,
    stats: Value,
) -> RemoteDesktopEventProjection {
    (
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
    ("SESSION_CLOSING", json!({ "reason": reason }))
}

/// Build a caller-requested closed payload.
pub(in crate::daemon::plugins::remote_desktop) fn session_closed(
    reason: &str,
) -> RemoteDesktopEventProjection {
    ("SESSION_CLOSED", json!({ "reason": reason }))
}

/// Build a lease-expiry closed payload.
pub(in crate::daemon::plugins::remote_desktop) fn session_expired(
    reason: &str,
    lease_expires_at_ms: u64,
) -> RemoteDesktopEventProjection {
    (
        "SESSION_CLOSED",
        json!({
            "reason": reason,
            "lease_expires_at_ms": lease_expires_at_ms,
        }),
    )
}

/// Build a production WebRTC connected payload.
pub(in crate::daemon::plugins::remote_desktop) fn webrtc_sender_ready(
    endpoint_ura: String,
    transport_epoch: u64,
) -> RemoteDesktopEventProjection {
    (
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
    (
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
        }),
    )
}

/// Build a target-scoped media source loss event.
pub(in crate::daemon::plugins::remote_desktop) fn media_source_lost(
    binding: &RemoteAppTargetBinding,
    reason: TargetResolutionError,
    transport_epoch: u64,
) -> RemoteDesktopEventProjection {
    (
        "MEDIA_SOURCE_LOST",
        json!({
            "subject_ura": binding.subject_ura(),
            "binding_id": binding.binding_id(),
            "binding_epoch": binding.binding_epoch(),
            "previous_target_identity_epoch": binding.target_identity_epoch(),
            "target_identity_epoch": binding.target_identity_epoch(),
            "target_geometry_revision": binding.target_geometry_revision(),
            "media_source_epoch": binding.media_source_epoch(),
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
    ("SESSION_FAILED", payload)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::daemon::plugins::remote_desktop::constants::direct_webrtc_endpoint_ura;
    use crate::daemon::plugins::remote_desktop::target::TargetResolutionError;
    use crate::daemon::plugins::remote_desktop::test_support::test_session_init;

    use super::{
        media_source_lost, preview_transport_connected, webrtc_failed_with_context,
        webrtc_sender_ready,
    };

    #[test]
    fn remote_desktop_event_payloads_keep_transport_kind_explicit() {
        let (_, preview_payload) = preview_transport_connected();
        let (_, webrtc_payload) = webrtc_sender_ready(direct_webrtc_endpoint_ura("session"), 1);

        assert_eq!(preview_payload["transport_kind"], json!("invoke_bidi"));
        assert_eq!(webrtc_payload["transport_kind"], json!("webrtc"));
    }

    #[test]
    fn webrtc_failure_payload_preserves_typed_target_context() {
        let (event_type, payload) = webrtc_failed_with_context(
            "target_identity_changed",
            "target changed".to_string(),
            7,
            json!({
                "failure_domain": "target",
                "frontend_action": "refresh_targets",
                "binding_id": "tb_test",
            }),
        );

        assert_eq!(event_type, "SESSION_FAILED");
        assert_eq!(payload["reason"], json!("target_identity_changed"));
        assert_eq!(payload["message"], json!("target changed"));
        assert_eq!(payload["transport_kind"], json!("webrtc"));
        assert_eq!(payload["transport_epoch"], json!(7));
        assert_eq!(payload["failure_domain"], json!("target"));
        assert_eq!(payload["frontend_action"], json!("refresh_targets"));
        assert_eq!(payload["binding_id"], json!("tb_test"));
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
        );

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
        assert_eq!(payload["reason"], json!("target_permission_missing"));
        assert_eq!(payload["reason_code"], json!("target_permission_missing"));
        assert_eq!(payload["recoverability"], json!("requires_target_refresh"));
        assert_eq!(payload["failure_domain"], json!("target"));
        assert_eq!(payload["frontend_action"], json!("request_permission"));
        assert_eq!(payload["transport_kind"], json!("webrtc"));
        assert_eq!(payload["media_transport_ready"], json!(false));
        assert_eq!(payload["transport_epoch"], json!(9));
    }
}
