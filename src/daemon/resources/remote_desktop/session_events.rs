// EasyNet CLI — remote desktop session event payloads
// ====================================================
//
// File: src/daemon/resources/remote_desktop/session_events.rs
// Description: Canonical event payload projections for remote desktop
// sessions.

use serde_json::{json, Value};

use crate::daemon::resources::remote_desktop::constants::{
    TRANSPORT_INVOKE_BIDI, TRANSPORT_WEBRTC,
};

type RemoteDesktopEventProjection = (&'static str, Value);

/// Build the immutable `SESSION_CREATED` payload.
///
/// This projection module does not mutate session state and does not write to
/// the bounded event log. It only keeps event payload shape stable and
/// reviewable in one place.
pub(in crate::daemon::resources::remote_desktop) fn session_created() -> RemoteDesktopEventProjection
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

/// Build a generic SDP description-set payload.
pub(in crate::daemon::resources::remote_desktop) fn description_set(
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
pub(in crate::daemon::resources::remote_desktop) fn local_webrtc_answer_set(
    backend_id: &str,
    production_ready: bool,
) -> RemoteDesktopEventProjection {
    (
        "DESCRIPTION_SET",
        json!({
            "transport_kind": TRANSPORT_WEBRTC,
            "side": "local",
            "media_transport_ready": false,
            "backend_id": backend_id,
            "production_ready": production_ready,
        }),
    )
}

/// Build a remote ICE candidate append payload.
pub(in crate::daemon::resources::remote_desktop) fn remote_ice_candidate_added(
    candidate_count: usize,
    applied_to_live_endpoint: bool,
    media_transport_ready: bool,
) -> RemoteDesktopEventProjection {
    (
        "ICE_CANDIDATE_ADDED",
        json!({
            "candidate_count": candidate_count,
            "applied_to_live_endpoint": applied_to_live_endpoint,
            "media_transport_ready": media_transport_ready,
        }),
    )
}

/// Build an InvokeBidi diagnostic preview-connected payload.
pub(in crate::daemon::resources::remote_desktop) fn preview_transport_connected(
) -> RemoteDesktopEventProjection {
    (
        "TRANSPORT_CONNECTED",
        json!({
            "transport_kind": TRANSPORT_INVOKE_BIDI,
            "encoding": "metadata_json_plus_binary",
            "media_transport_ready": false,
            "fallback_transport_ready": true,
            "diagnostic_only": true,
        }),
    )
}

/// Build an InvokeBidi diagnostic preview-detached payload.
pub(in crate::daemon::resources::remote_desktop) fn preview_transport_detached(
    reason: &str,
) -> RemoteDesktopEventProjection {
    (
        "TRANSPORT_DETACHED",
        json!({
            "transport_kind": TRANSPORT_INVOKE_BIDI,
            "reason": reason,
            "media_transport_ready": false,
            "fallback_transport_ready": false,
            "diagnostic_only": true,
        }),
    )
}

/// Build an InvokeBidi diagnostic preview failure payload.
pub(in crate::daemon::resources::remote_desktop) fn preview_transport_failed(
    reason: &str,
    message: String,
) -> RemoteDesktopEventProjection {
    (
        "SESSION_FAILED",
        json!({
            "transport_kind": TRANSPORT_INVOKE_BIDI,
            "reason": reason,
            "message": message,
            "media_transport_ready": false,
            "fallback_transport_ready": false,
            "diagnostic_only": true,
        }),
    )
}

/// Build a lease-refresh payload.
pub(in crate::daemon::resources::remote_desktop) fn lease_refreshed(
    lease_expires_at_ms: u64,
) -> RemoteDesktopEventProjection {
    (
        "LEASE_REFRESHED",
        json!({ "lease_expires_at_ms": lease_expires_at_ms }),
    )
}

/// Build a transport-blocked payload.
pub(in crate::daemon::resources::remote_desktop) fn transport_blocked(
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
pub(in crate::daemon::resources::remote_desktop) fn local_ice_candidate(
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
pub(in crate::daemon::resources::remote_desktop) fn webrtc_diagnostic(
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
pub(in crate::daemon::resources::remote_desktop) fn input_channel_diagnostic(
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
pub(in crate::daemon::resources::remote_desktop) fn media_pipeline_stats(
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
pub(in crate::daemon::resources::remote_desktop) fn session_closing(
    reason: &str,
) -> RemoteDesktopEventProjection {
    ("SESSION_CLOSING", json!({ "reason": reason }))
}

/// Build a caller-requested closed payload.
pub(in crate::daemon::resources::remote_desktop) fn session_closed(
    reason: &str,
) -> RemoteDesktopEventProjection {
    ("SESSION_CLOSED", json!({ "reason": reason }))
}

/// Build a lease-expiry closed payload.
pub(in crate::daemon::resources::remote_desktop) fn session_expired(
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
pub(in crate::daemon::resources::remote_desktop) fn webrtc_connected(
    endpoint_ura: String,
) -> RemoteDesktopEventProjection {
    (
        "TRANSPORT_CONNECTED",
        json!({
            "transport_kind": TRANSPORT_WEBRTC,
            "media_transport_ready": true,
            "endpoint_ura": endpoint_ura,
            "codec": "h264",
            "carrier": "rtp_srtp",
        }),
    )
}

/// Build a WebRTC failure payload.
pub(in crate::daemon::resources::remote_desktop) fn webrtc_failed(
    reason: &str,
    message: String,
) -> RemoteDesktopEventProjection {
    (
        "SESSION_FAILED",
        json!({
            "reason": reason,
            "message": message,
            "transport_kind": TRANSPORT_WEBRTC,
            "media_transport_ready": false,
        }),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{preview_transport_connected, webrtc_connected};

    #[test]
    fn remote_desktop_event_payloads_keep_transport_kind_explicit() {
        let (_, preview_payload) = preview_transport_connected();
        let (_, webrtc_payload) = webrtc_connected("easynet-rd://session".to_string());

        assert_eq!(preview_payload["transport_kind"], json!("invoke_bidi"));
        assert_eq!(webrtc_payload["transport_kind"], json!("webrtc"));
    }
}
