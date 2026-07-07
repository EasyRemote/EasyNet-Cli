// EasyNet CLI — remote desktop constants
// ======================================
//
// File: plugins/remote-desktop/src/constants.rs
// Description: Stable ability names, reason codes, and runtime defaults.

pub const ABILITY_CREATE_SESSION: &str = "remote_desktop.create_session";
pub const ABILITY_SHOW_SESSION: &str = "remote_desktop.show_session";
pub const ABILITY_SET_DESCRIPTION: &str = "remote_desktop.set_description";
pub const ABILITY_ADD_ICE_CANDIDATE: &str = "remote_desktop.add_ice_candidate";
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
pub const REASON_SESSION_TOKEN_REQUIRED: &str = "session_token_required";
pub const REASON_SESSION_TOKEN_MISMATCH: &str = "session_token_mismatch";
pub const REASON_SESSION_CALLER_MISMATCH: &str = "session_caller_mismatch";
pub const REASON_CONSENT_RECEIPT_REQUIRED: &str = "consent_receipt_required";
pub const REASON_CONSENT_RECEIPT_MISMATCH: &str = "consent_receipt_mismatch";
pub const REASON_SESSION_STORE_FULL: &str = "session_store_full";
pub const REASON_INVALID_ARGUMENT: &str = "invalid_argument";
pub const REASON_RESOURCE_UNAVAILABLE: &str = "resource_unavailable";
pub const REASON_PREVIEW_CLIENT_CLOSED: &str = "preview_client_closed";
pub const REASON_PREVIEW_CAPTURE_FAILED: &str = "preview_capture_failed";

pub(crate) const DEFAULT_LEASE_TTL_MS: u64 = 30_000;
pub(crate) const MAX_LEASE_TTL_MS: u64 = 300_000;
pub(crate) const TRANSPORT_WEBRTC: &str = "webrtc";
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
pub(crate) const DIRECT_WEBRTC_ENDPOINT_PREFIX: &str = "webrtc://direct/";
pub(crate) const NATIVE_MIN_BITRATE_KBPS: u32 = 4_000;
pub(crate) const NATIVE_MAX_BITRATE_KBPS: u32 = 250_000;
