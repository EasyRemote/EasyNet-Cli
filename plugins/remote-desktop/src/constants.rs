// EasyNet CLI — remote desktop constants
// ======================================
//
// File: plugins/remote-desktop/src/constants.rs
// Description: Stable ability names, reason codes, and runtime defaults.

pub const ABILITY_CREATE_SESSION: &str = "remote_desktop.create_session";
pub const ABILITY_FOCUS_TARGET: &str = "remote_desktop.focus_target";
pub const ABILITY_GRANT_CONSENT: &str = "remote_desktop.grant_consent";
pub const ABILITY_SHOW_SESSION: &str = "remote_desktop.show_session";
pub const ABILITY_SET_DESCRIPTION: &str = "remote_desktop.set_description";
pub const ABILITY_ADD_ICE_CANDIDATE: &str = "remote_desktop.add_ice_candidate";
pub const ABILITY_REPORT_CLIENT_STATE: &str = "remote_desktop.report_client_state";
pub const ABILITY_WATCH_EVENTS: &str = "remote_desktop.watch_events";
pub const ABILITY_REFRESH_LEASE: &str = "remote_desktop.refresh_lease";
pub const ABILITY_END_SESSION: &str = "remote_desktop.end_session";
pub const ABILITY_ATTACH_SESSION: &str = "remote_desktop.attach";
pub const ABILITY_PERMISSION_STATUS: &str = "remote_desktop.permission_status";
pub const ABILITY_REQUEST_PERMISSION: &str = "remote_desktop.request_permission";

pub const REASON_RESOURCE_TYPE_MISMATCH: &str = "resource_type_mismatch";
pub const REASON_SESSION_NOT_FOUND: &str = "session_not_found";
pub const REASON_SESSION_TERMINAL: &str = "session_terminal";
pub const REASON_SESSION_EXPIRED: &str = "session_expired";
pub const REASON_TARGET_PERMISSION_REVOKED: &str = "target_permission_revoked";
pub const REASON_SESSION_TOKEN_REQUIRED: &str = "session_token_required";
pub const REASON_SESSION_TOKEN_MISMATCH: &str = "session_token_mismatch";
pub const REASON_SESSION_CALLER_MISMATCH: &str = "session_caller_mismatch";
pub const REASON_CONSENT_RECEIPT_REQUIRED: &str = "consent_receipt_required";
pub const REASON_CONSENT_RECEIPT_MISMATCH: &str = "consent_receipt_mismatch";
pub const REASON_SESSION_STORE_FULL: &str = "session_store_full";
pub const REASON_INVALID_ARGUMENT: &str = "invalid_argument";
pub const REASON_RESOURCE_EXHAUSTED: &str = "resource_exhausted";
pub const REASON_RESOURCE_UNAVAILABLE: &str = "resource_unavailable";
pub const REASON_PREVIEW_CLIENT_CLOSED: &str = "preview_client_closed";
pub const REASON_PREVIEW_CAPTURE_FAILED: &str = "preview_capture_failed";
pub const REASON_TRANSPORT_SETTLEMENT_FAILED: &str = "transport_settlement_failed";

pub(crate) const DEFAULT_LEASE_TTL_MS: u64 = 30_000;
pub(crate) const MAX_LEASE_TTL_MS: u64 = 300_000;
pub(crate) const TRANSPORT_WEBRTC: &str = "webrtc";
/// Preferred dynamic RTP payload type for local H.264 codec registration.
///
/// Offer/answer can remap this value. Media writers must use the negotiated
/// payload type captured from `RtpSender`, never this registration preference.
pub(crate) const DIRECT_WEBRTC_H264_PREFERRED_PAYLOAD_TYPE: u8 = 102;
pub(crate) const TRANSPORT_INVOKE_BIDI: &str = "invoke_bidi";
pub(crate) const TRANSPORT_PREVIEW_STREAM: &str = "preview_stream";
pub(crate) const MIN_ATTACH_FPS: u32 = 1;
pub(crate) const MAX_ATTACH_FPS: u32 = 144;
pub(crate) const DEFAULT_TARGET_FPS: u32 = 144;
pub(crate) const DEFAULT_MIN_ACCEPTABLE_FPS: u32 = 60;
pub(crate) const DEFAULT_TARGET_BITRATE_KBPS: u32 = 50_000;
pub(crate) const DEFAULT_GLASS_TO_GLASS_LATENCY_MS: u32 = 50;
pub(crate) const DEFAULT_CAPTURE_TO_ENCODE_MS: u32 = 8;
pub(crate) const DEFAULT_FRAME_QUEUE_DEPTH: u32 = 1;
pub(crate) const MAX_FRAME_QUEUE_DEPTH: u32 = 1;
pub(crate) const MAX_VIDEO_DIMENSION: u64 = 8192;
pub(crate) const DIAGNOSTIC_RELAY_TARGET_BITRATE_KBPS: u32 = 24_000;
pub(crate) const DEFAULT_VIDEO_STREAM_BITRATE_KBPS: u32 = DIAGNOSTIC_RELAY_TARGET_BITRATE_KBPS;
pub(crate) const ATTACH_ENCODING_ANNEXB_H264: &str = "annexb_h264";
pub(crate) const ATTACH_ENCODING_JPEG_BINARY: &str = "jpeg_binary";
const DIRECT_WEBRTC_ENDPOINT_PREFIX: &str = "easynet:///r/local/resource/remote-desktop-transport.";

pub(crate) fn direct_webrtc_endpoint_ura(session_id: &str) -> String {
    format!(
        "{DIRECT_WEBRTC_ENDPOINT_PREFIX}{}/endpoint/webrtc",
        hex::encode(session_id.as_bytes())
    )
}
pub(crate) const NATIVE_MIN_BITRATE_KBPS: u32 = 4_000;
pub(crate) const NATIVE_MAX_BITRATE_KBPS: u32 = 250_000;
pub(crate) const MAX_SIGNALING_DESCRIPTION_BYTES: usize = 256 * 1024;
pub(crate) const MAX_ICE_CANDIDATE_BYTES: usize = 4 * 1024;
pub(crate) const MAX_REMOTE_ICE_CANDIDATES: usize = 64;
pub(crate) const MAX_LOCAL_ICE_CANDIDATES: usize = 64;

#[cfg(test)]
mod tests {
    use super::direct_webrtc_endpoint_ura;

    #[test]
    fn direct_webrtc_endpoint_ura_encodes_session_id_as_easynet_ura() {
        let endpoint_ura = direct_webrtc_endpoint_ura("rd/session:1");

        assert_eq!(
            endpoint_ura,
            "easynet:///r/local/resource/remote-desktop-transport.72642f73657373696f6e3a31/endpoint/webrtc"
        );
        assert!(!endpoint_ura.contains("rd/session:1"));
    }
}
