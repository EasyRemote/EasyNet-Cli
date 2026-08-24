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

/// Human-readable contract for `remote_desktop.grant_consent`.
pub fn grant_consent_description() -> &'static str {
    "Record explicit local-user consent for creating a remote desktop session \
     on the selected display/window/application resource. The terminal receipt \
     of this invocation must be supplied as causal_context to \
     remote_desktop.create_session."
}

/// JSON input schema for `remote_desktop.grant_consent`.
pub fn grant_consent_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["intent"],
        "properties": {
            "intent": { "type": "string", "enum": ["remote_desktop_session"] }
        }
    })
}

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
        "required": ["consent_ticket"],
        "properties": {
            "consent_ticket": { "type": "string", "minLength": 64, "maxLength": 64 },
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
    "Append one ICE candidate to the active epoch of a remote desktop session's signaling log."
}

/// JSON input schema for `remote_desktop.add_ice_candidate`.
pub fn add_ice_candidate_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_id", "session_token", "transport_epoch", "candidate"],
        "properties": {
            "session_id": { "type": "string" },
            "session_token": { "type": "string" },
            "transport_epoch": { "type": "integer", "minimum": 1 },
            "candidate": { "type": "object" }
        }
    })
}

pub fn report_client_state_description() -> &'static str {
    "Report browser-observed media presentation and bounded transport evidence for the active remote desktop transport epoch."
}

pub fn report_client_state_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["session_id", "session_token", "transport_epoch", "state"],
        "properties": {
            "session_id": { "type": "string" },
            "session_token": { "type": "string" },
            "transport_epoch": { "type": "integer", "minimum": 1 },
            "state": { "type": "string", "enum": ["presenting", "stalled", "detached"] },
            "client_transport": remote_desktop_client_transport_schema(),
            "browser_stats": remote_desktop_browser_stats_schema(),
            "render_probe": remote_desktop_render_probe_schema()
        }
    })
}

fn remote_desktop_client_transport_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "ice_connection_state": bounded_string_schema(),
            "peer_connection_state": bounded_string_schema(),
            "route_kind": bounded_string_schema(),
            "sampled_at_ms": { "type": "number" },
            "selected_candidate_pair": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "id": bounded_string_schema(),
                    "candidate_pair_id": bounded_string_schema(),
                    "local_candidate_id": bounded_string_schema(),
                    "remote_candidate_id": bounded_string_schema(),
                    "local_candidate_type": bounded_string_schema(),
                    "remote_candidate_type": bounded_string_schema(),
                    "selected_route_class": bounded_string_schema(),
                    "protocol": bounded_string_schema(),
                    "state": bounded_string_schema(),
                    "selected": { "type": "boolean" },
                    "nominated": { "type": "boolean" },
                    "current_round_trip_time_ms": { "type": "number" },
                    "available_outgoing_bitrate_bps": { "type": "number" },
                    "available_incoming_bitrate_bps": { "type": "number" },
                    "packets_discarded_on_send": { "type": "number" },
                    "bytes_discarded_on_send": { "type": "number" }
                }
            }
        }
    })
}

fn remote_desktop_browser_stats_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "sampled_at_ms": { "type": "number" },
            "frames_decoded": { "type": "number" },
            "frames_dropped": { "type": "number" },
            "frames_received": { "type": "number" },
            "frame_width": { "type": "number" },
            "frame_height": { "type": "number" },
            "jitter_buffer_avg_ms": { "type": "number" },
            "jitter_buffer_target_avg_ms": { "type": "number" },
            "decode_avg_ms": { "type": "number" },
            "processing_avg_ms": { "type": "number" },
            "freeze_count": { "type": "number" },
            "pause_count": { "type": "number" }
        }
    })
}

fn remote_desktop_render_probe_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "probe_source": bounded_string_schema(),
            "selected_resource_ura": bounded_string_schema(),
            "session_id": bounded_string_schema(),
            "media_pipeline_id": bounded_string_schema(),
            "video_codec": bounded_string_schema(),
            "video_transport": bounded_string_schema(),
            "audio_codec": bounded_string_schema(),
            "observed_at_ms": { "type": "number" },
            "decoded_video_frames": { "type": "number" },
            "decoded_audio_packets": { "type": "number" },
            "decoded_audio_samples": { "type": "number" },
            "video_payload_hash": bounded_string_schema(),
            "audio_payload_hash": bounded_string_schema(),
            "frame_width": { "type": "number" },
            "frame_height": { "type": "number" }
        }
    })
}

fn bounded_string_schema() -> Value {
    json!({ "type": "string", "minLength": 1, "maxLength": 256 })
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
    "Ask the operating system for the host permissions required by native \
     remote desktop. On macOS this requests Screen Recording for capture and \
     Accessibility for pointer/keyboard input injection from the daemon-side \
     ability process."
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
