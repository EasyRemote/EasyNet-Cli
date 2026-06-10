// EasyNet CLI — remote desktop transport view projection
// ======================================================
//
// File: src/plugins/builtin/remote_desktop/view_transport.rs
// Description: JSON transport projections for remote desktop session views.

use serde_json::{json, Value};

use crate::plugins::remote_desktop::constants::{
    DIRECT_WEBRTC_ENDPOINT_PREFIX, TRANSPORT_INVOKE_BIDI, TRANSPORT_PREVIEW_STREAM,
    TRANSPORT_WEBRTC,
};
use crate::plugins::remote_desktop::input::INPUT_DATA_CHANNEL_LABEL;
use crate::plugins::remote_desktop::session::RemoteDesktopSession;

/// Transport view facts derived from one session row.
///
/// This type is a DTO helper only. It does not decide session lifecycle,
/// mutate signaling state, or select media backends.
pub(in crate::plugins::builtin::remote_desktop) struct RemoteDesktopTransportView {
    endpoint_ura: Value,
    unavailable_reason: Value,
    message: &'static str,
}

impl RemoteDesktopTransportView {
    /// Derive stable transport view facts from a session.
    pub(in crate::plugins::builtin::remote_desktop) fn from_session(
        session: &RemoteDesktopSession,
    ) -> Self {
        let endpoint_ura = direct_endpoint_ura(session);
        let unavailable_reason = transport_unavailable_reason(session);
        let message = transport_message(session);
        Self {
            endpoint_ura,
            unavailable_reason,
            message,
        }
    }

    /// Build the legacy `transport` object.
    pub(in crate::plugins::builtin::remote_desktop) fn summary(
        &self,
        session: &RemoteDesktopSession,
    ) -> Value {
        json!({
            "kind": TRANSPORT_WEBRTC,
            "primary_transport": TRANSPORT_WEBRTC,
            "primary_ready": session.media_transport_ready(),
            "preferred": session.transport_preferences(),
            "fallback_transports": [TRANSPORT_INVOKE_BIDI, TRANSPORT_PREVIEW_STREAM],
            "endpoint_ura": self.endpoint_ura.clone(),
            "preview_ability": "screen.subscribe",
            "message": self.message,
            "unavailable_reason": self.unavailable_reason.clone(),
            "input_channel_label": INPUT_DATA_CHANNEL_LABEL,
            "required_runtime": ["os_capture_stream", "video_encoder", "webrtc_peer_connection", "data_channel_input"]
        })
    }

    /// Build the ordered transport capability list.
    pub(in crate::plugins::builtin::remote_desktop) fn transport_list(
        &self,
        session: &RemoteDesktopSession,
    ) -> Value {
        json!([
            {
                "transport": TRANSPORT_WEBRTC,
                "transport_proto": "REMOTE_DESKTOP_TRANSPORT_WEBRTC",
                "ready": session.media_transport_ready(),
                "endpoint_ura": self.endpoint_ura.clone(),
                "metadata": {
                    "role": "primary",
                    "signaling_plane": "axon_signed_invocation",
                    "media_plane": "rtp_srtp",
                    "input_plane": "webrtc_data_channel",
                    "input_channel_label": INPUT_DATA_CHANNEL_LABEL,
                    "unavailable_reason": self.unavailable_reason.clone()
                },
            },
            {
                "transport": TRANSPORT_INVOKE_BIDI,
                "transport_proto": "REMOTE_DESKTOP_TRANSPORT_INVOKE_BIDI",
                "ready": session.preview_attached(),
                "endpoint_ura": null,
                "metadata": {
                    "role": "diagnostic_fallback",
                    "diagnostic_only": "true",
                    "media_plane": "metadata_json_plus_binary_preview",
                    "drop_stale_frames": "true",
                },
            },
            {
                "transport": TRANSPORT_PREVIEW_STREAM,
                "transport_proto": "REMOTE_DESKTOP_TRANSPORT_PREVIEW_STREAM",
                "ready": false,
                "endpoint_ura": "ability:screen.subscribe",
                "metadata": {
                    "role": "debug_preview",
                    "diagnostic_only": "true",
                    "media_plane": "jpeg_preview",
                },
            }
        ])
    }
}

fn direct_endpoint_ura(session: &RemoteDesktopSession) -> Value {
    if session.local_description().is_some() {
        json!(format!(
            "{DIRECT_WEBRTC_ENDPOINT_PREFIX}{}",
            session.session_id()
        ))
    } else {
        Value::Null
    }
}

fn transport_unavailable_reason(session: &RemoteDesktopSession) -> Value {
    if session.media_transport_ready() {
        Value::Null
    } else if let Some(error) = session.webrtc_error() {
        json!(error)
    } else if session.webrtc_ice_state() == Some("failed") {
        json!("webrtc_ice_failed")
    } else if session.local_description().is_some() {
        json!("webrtc_ice_connecting")
    } else {
        json!("webrtc_offer_required")
    }
}

fn transport_message(session: &RemoteDesktopSession) -> &'static str {
    if session.media_transport_ready() {
        "Direct device-side WebRTC endpoint is ready; InvokeBidi and preview_stream remain diagnostic fallbacks."
    } else if session.webrtc_error() == Some("native_media_plugin_required") {
        "Direct WebRTC RTP/SRTP is blocked until a native capture/encode plugin is installed; InvokeBidi remains an explicit diagnostic fallback."
    } else if session.webrtc_error() == Some("native_media_pipeline_failed") {
        "Native ScreenCaptureKit/VideoToolbox media pipeline failed before producing frames; check the session failure event for the platform error."
    } else if session.webrtc_error() == Some("webrtc_transport_backend_unavailable") {
        "Direct WebRTC RTP/SRTP is blocked because this capture subject has no available device-side WebRTC backend; InvokeBidi remains an explicit diagnostic fallback."
    } else if session.local_description().is_some() {
        "Direct device-side WebRTC endpoint is negotiating ICE/DTLS; InvokeBidi and preview_stream remain diagnostic-only fallbacks."
    } else {
        "WebRTC endpoint requires a browser SDP offer; InvokeBidi and preview_stream are diagnostic-only fallbacks until negotiation completes."
    }
}
