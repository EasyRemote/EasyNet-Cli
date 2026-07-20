// EasyNet CLI — remote desktop ability metadata
// =============================================
//
// File: plugins/remote-desktop/src/schema.rs
// Description: Ability descriptions and JSON input schemas for the remote
//              desktop plugin.
//
// Architectural Position:
// - Pure metadata module. Runtime session state, WebRTC transport, and media
//   capture logic stay in the session, transport, and media modules.

use serde_json::{json, Value};

use super::constants::{
    ATTACH_ENCODING_ANNEXB_H264, ATTACH_ENCODING_JPEG_BINARY, MAX_ATTACH_FPS,
    MAX_FRAME_QUEUE_DEPTH, MAX_LEASE_TTL_MS, MAX_VIDEO_DIMENSION, MIN_ATTACH_FPS,
    NATIVE_MAX_BITRATE_KBPS,
};

/// Human-readable contract for `remote_desktop.create_session`.
pub fn create_session_description() -> &'static str {
    "Create a remote desktop control session for a display/window/application \
     resource. Subject MUST be the resource_ura in the invocation envelope. \
     The session advertises WebRTC as the production media transport and \
     exposes quality targets; preview paths stay marked as diagnostic \
     transports."
}

/// JSON input schema for `remote_desktop.create_session`.
pub fn create_session_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "session_id": { "type": "string" },
            "mode": { "type": "string", "enum": ["view_only", "interactive"] },
            "lease_ttl_ms": { "type": "integer", "minimum": 1, "maximum": MAX_LEASE_TTL_MS },
            "requested_ttl_seconds": { "type": "integer", "minimum": 1, "maximum": MAX_LEASE_TTL_MS / 1000 },
            "transport_preferences": {
                "type": "array",
                "items": { "type": "string", "enum": ["webrtc", "invoke_bidi", "preview_stream"] }
            },
            "video": remote_desktop_video_schema(),
            "input": remote_desktop_input_policy_schema(),
            "input_policy": remote_desktop_input_policy_schema()
        }
    })
}

fn remote_desktop_video_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "max_width": { "type": "integer", "minimum": 1, "maximum": MAX_VIDEO_DIMENSION },
            "max_height": { "type": "integer", "minimum": 1, "maximum": MAX_VIDEO_DIMENSION },
            "max_fps": { "type": "integer", "minimum": MIN_ATTACH_FPS, "maximum": MAX_ATTACH_FPS },
            "max_bitrate_kbps": { "type": "integer", "minimum": 1, "maximum": NATIVE_MAX_BITRATE_KBPS },
            "scale_mode": { "type": "string" },
            "region": { "type": "string" },
            "codec_preferences": {
                "type": "array",
                "minItems": 1,
                "items": { "type": "string", "enum": ["h264", "hevc", "av1", "vp9", "vp8"] }
            },
            "target_latency_ms": { "type": "integer", "minimum": 1, "maximum": 1000 },
            "hardware_acceleration_required": { "type": "boolean" },
            "max_frame_queue_depth": { "type": "integer", "minimum": 1, "maximum": MAX_FRAME_QUEUE_DEPTH },
            "drop_stale_frames": { "type": "boolean" }
        }
    })
}

fn remote_desktop_input_policy_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "keyboard_enabled": { "type": "boolean" },
            "keyboard": { "type": "boolean" },
            "pointer_enabled": { "type": "boolean" },
            "pointer": { "type": "boolean" },
            "clipboard_enabled": { "type": "boolean" },
            "clipboard": { "type": "boolean" },
            "file_drop_enabled": { "type": "boolean" },
            "file_drop": { "type": "boolean" }
        }
    })
}

/// Human-readable contract for `remote_desktop.show_session`.
pub fn show_session_description() -> &'static str {
    "Read one remote desktop session state. Subject MUST match the bound \
     resource_ura and session_token MUST match the create_session response."
}

/// JSON input schema for `remote_desktop.show_session`.
pub fn show_session_input_schema() -> Value {
    session_identity_schema()
}

/// Human-readable contract for `remote_desktop.set_description`.
pub fn set_description_description() -> &'static str {
    "Attach a WebRTC-style local or remote session description to a remote \
     desktop session. A remote SDP offer starts the device-side direct WebRTC \
     media endpoint and returns the SDP answer in local_description; media is \
     not routed through unary Invoke."
}

/// JSON input schema for `remote_desktop.set_description`.
pub fn set_description_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_id", "session_token", "side", "description"],
        "properties": {
            "session_id": { "type": "string" },
            "session_token": { "type": "string" },
            "side": { "type": "string", "enum": ["local", "remote"] },
            "description": { "type": "object" }
        }
    })
}

/// Human-readable contract for `remote_desktop.add_ice_candidate`.
pub fn add_ice_candidate_description() -> &'static str {
    "Append one ICE candidate to a remote desktop session's signaling log."
}

/// JSON input schema for `remote_desktop.add_ice_candidate`.
pub fn add_ice_candidate_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_id", "session_token", "candidate"],
        "properties": {
            "session_id": { "type": "string" },
            "session_token": { "type": "string" },
            "candidate": { "type": "object" }
        }
    })
}

/// Human-readable contract for `remote_desktop.watch_events`.
pub fn watch_events_description() -> &'static str {
    "Watch remote desktop control events. v1 returns the bounded current \
     snapshot through InvokeStream; live fan-out follows the canonical invocation contract."
}

/// JSON input schema for `remote_desktop.watch_events`.
pub fn watch_events_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_id", "session_token"],
        "properties": {
            "session_id": { "type": "string" },
            "session_token": { "type": "string" },
            "from_sequence": { "type": "integer", "minimum": 0 }
        }
    })
}

/// Human-readable contract for `remote_desktop.refresh_lease`.
pub fn refresh_lease_description() -> &'static str {
    "Refresh a non-terminal remote desktop session lease."
}

/// JSON input schema for `remote_desktop.refresh_lease`.
pub fn refresh_lease_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_id", "session_token"],
        "properties": {
            "session_id": { "type": "string" },
            "session_token": { "type": "string" },
            "lease_ttl_ms": { "type": "integer", "minimum": 1, "maximum": MAX_LEASE_TTL_MS },
            "requested_ttl_seconds": { "type": "integer", "minimum": 1, "maximum": MAX_LEASE_TTL_MS / 1000 }
        }
    })
}

/// Human-readable contract for `remote_desktop.end_session`.
pub fn end_session_description() -> &'static str {
    "Close a remote desktop session. Idempotent after the first terminal close."
}

/// JSON input schema for `remote_desktop.end_session`.
pub fn end_session_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_id", "session_token"],
        "properties": {
            "session_id": { "type": "string" },
            "session_token": { "type": "string" },
            "reason": { "type": "string" }
        }
    })
}

/// Human-readable contract for `remote_desktop.attach`.
pub fn attach_description() -> &'static str {
    "Attach a bounded Axon InvokeBidi media plane to an existing remote desktop \
     session. Subject MUST match the resource_ura bound when the session was \
     created. Control metadata remains reliable JSON, while live H.264/JPEG media \
     chunks are bounded and may drop stale frames to preserve low latency; WebRTC \
     remains the preferred direct transport when negotiation succeeds."
}

/// JSON input schema for `remote_desktop.attach`.
pub fn attach_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_id", "session_token"],
        "properties": {
            "session_id": { "type": "string" },
            "session_token": { "type": "string" },
            "encoding": {
                "type": "string",
                "enum": [
                    ATTACH_ENCODING_ANNEXB_H264,
                    ATTACH_ENCODING_JPEG_BINARY,
                    "jpeg",
                    "image/jpeg"
                ]
            },
            "fps": { "type": "integer", "minimum": MIN_ATTACH_FPS, "maximum": MAX_ATTACH_FPS },
            "resolution": { "type": "string" },
            "video": { "type": "object" }
        }
    })
}

/// Human-readable contract for `remote_desktop.permission_status`.
pub fn permission_status_description() -> &'static str {
    "Report whether this host process has the OS screen-capture permission \
     required by the native remote desktop media pipeline."
}

/// JSON input schema for `remote_desktop.permission_status`.
pub fn permission_status_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

/// Human-readable contract for `remote_desktop.request_permission`.
pub fn request_permission_description() -> &'static str {
    "Ask the operating system for the screen-capture permission required by \
     native remote desktop. On macOS this calls CoreGraphics' Screen Recording \
     TCC request API from the daemon-side ability process."
}

/// JSON input schema for `remote_desktop.request_permission`.
pub fn request_permission_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

fn session_identity_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_id", "session_token"],
        "properties": {
            "session_id": { "type": "string" },
            "session_token": { "type": "string" }
        }
    })
}
