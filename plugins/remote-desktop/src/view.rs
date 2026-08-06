// EasyNet CLI — remote desktop session view projection
// ====================================================
//
// File: plugins/remote-desktop/src/view.rs
// Description: JSON response projection for remote desktop sessions.

use serde_json::{json, Value};

use crate::daemon::plugins::remote_desktop::input::{
    input_injection_available, INPUT_DATA_CHANNEL_LABEL,
};
use crate::daemon::plugins::remote_desktop::media::{
    backend_catalog_view, production_gate_view, sdk_contract_view,
};
use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
use crate::daemon::plugins::remote_desktop::view_device::{
    device_capabilities_view, empty_pipeline_metrics, quality_targets,
};
use crate::daemon::plugins::remote_desktop::view_transport::RemoteDesktopTransportView;

/// Build the public session response without echoing the session token.
///
/// This is a projection of the current device-side session row using the
/// product-owned wire vocabulary. Axon carries the surrounding canonical
/// invocation and receipt without owning remote desktop lifecycle semantics.
pub(in crate::daemon::plugins::remote_desktop) fn serialize_session(
    session: &RemoteDesktopSession,
) -> Value {
    let transport_view = RemoteDesktopTransportView::from_session(session);
    let video = session.video().to_value();
    let input_policy = session.input_policy().to_value();
    let remote_ice_candidates = session.remote_ice_candidates();
    let local_ice_candidates = session.local_ice_candidates();
    let media_stats = session.media_stats();
    json!({
        "session_id": session.session_id(),
        "state": session.state().json_name(),
        "state_proto": session.state().wire_name(),
        "subject_ura": session.subject_ura(),
        "subject_type": session.subject_type().as_str(),
        "subject_display_name": session.subject_display_name(),
        "mode": session.mode(),
        "created_at_ms": session.created_at_ms(),
        "updated_at_ms": session.updated_at_ms(),
        "lease_expires_at_ms": session.lease_expires_at_ms(),
        "end_reason": session.end_reason(),
        "video": video.clone(),
        "input_policy": input_policy.clone(),
        "consent": session.consent().to_value(),
        "media_transport_ready": session.media_transport_ready(),
        "client_media_ready": session.client_media_ready(),
        "transport_epoch": session.transport_epoch(),
        "transport_state": session.transport_state(),
        "input_plane": {
            "kind": "webrtc_data_channel",
            "label": INPUT_DATA_CHANNEL_LABEL,
            "policy": input_policy,
            "input_injection_available": input_injection_available(),
        },
        "quality": quality_targets(&video),
        "media_sdk": sdk_contract_view(),
        "media_backends": backend_catalog_view(),
        "production_gate": production_gate_view(),
        "device_capabilities": device_capabilities_view(),
        "latest_metrics": media_stats.clone().unwrap_or_else(empty_pipeline_metrics),
        "media_stats": media_stats,
        "negotiated_codec": session.negotiated_codec(),
        "transport": transport_view.summary(session),
        "transports": transport_view.transport_list(session),
        "signaling": {
            "local_description": session.local_description(),
            "remote_description": session.remote_description(),
            "ice_candidate_count": remote_ice_candidates.len(),
            "local_ice_candidate_count": local_ice_candidates.len(),
            "local_ice_candidates": local_ice_candidates,
            "webrtc_ice_state": session.webrtc_ice_state(),
            "webrtc_peer_state": session.webrtc_peer_state(),
            "webrtc_error": session.webrtc_error(),
        },
        "events": session.events(),
    })
}

/// Build the create-session response, where the opaque session token is
/// intentionally returned exactly once.
pub(in crate::daemon::plugins::remote_desktop) fn serialize_session_with_token(
    session: &RemoteDesktopSession,
) -> Value {
    let mut view = serialize_session(session);
    if let Some(map) = view.as_object_mut() {
        map.insert(
            "session_token".to_string(),
            json!(session.session_token_for_create_response()),
        );
    }
    view
}
