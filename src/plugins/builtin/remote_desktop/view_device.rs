// EasyNet CLI — remote desktop device view projection
// ====================================================
//
// File: src/plugins/builtin/remote_desktop/view_device.rs
// Description: Device capability, quality target, and metric DTO projection.

use serde_json::{json, Value};

use crate::plugins::remote_desktop::constants::{
    DEFAULT_CAPTURE_TO_ENCODE_MS, DEFAULT_FRAME_QUEUE_DEPTH, DEFAULT_GLASS_TO_GLASS_LATENCY_MS,
    DEFAULT_MIN_ACCEPTABLE_FPS, DEFAULT_TARGET_BITRATE_KBPS, DEFAULT_TARGET_FPS,
    DEFAULT_VIDEO_STREAM_BITRATE_KBPS, MAX_ATTACH_FPS, NATIVE_MAX_BITRATE_KBPS,
};
use crate::plugins::remote_desktop::input::{input_injection_available, INPUT_DATA_CHANNEL_LABEL};
use crate::plugins::remote_desktop::media::{
    backend_catalog_view, production_gate_view, sdk_contract_view, MACOS_SCK_VIDEOTOOLBOX_BACKEND,
    MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID, XCAP_MACOS_RECORDER_MAX_FPS, XCAP_OPENH264_BACKEND_ID,
};

/// Build media quality targets from create-session video constraints.
pub(in crate::plugins::builtin::remote_desktop) fn quality_targets(video: &Value) -> Value {
    json!({
        "target_fps": video.get("max_fps").and_then(Value::as_u64).unwrap_or(DEFAULT_TARGET_FPS as u64),
        "min_acceptable_fps": DEFAULT_MIN_ACCEPTABLE_FPS,
        "max_glass_to_glass_latency_ms": video
            .get("target_latency_ms")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_GLASS_TO_GLASS_LATENCY_MS as u64),
        "max_capture_to_encode_ms": DEFAULT_CAPTURE_TO_ENCODE_MS,
        "max_frame_queue_depth": video
            .get("max_frame_queue_depth")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_FRAME_QUEUE_DEPTH as u64),
        "target_bitrate_kbps": video
            .get("max_bitrate_kbps")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TARGET_BITRATE_KBPS as u64),
        "drop_stale_frames": video
            .get("drop_stale_frames")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

/// Build device/media capability projection for session responses.
pub(in crate::plugins::builtin::remote_desktop) fn device_capabilities_view() -> Value {
    let production_backend = MACOS_SCK_VIDEOTOOLBOX_BACKEND;
    let production_ready = production_backend.production_ready();
    let max_fps = if production_ready {
        production_backend.effective_fps(MAX_ATTACH_FPS)
    } else {
        XCAP_MACOS_RECORDER_MAX_FPS
    };
    let capture_backends = if production_ready {
        json!([
            "macos_screencapturekit_videotoolbox_webrtc",
            "xcap_video_recorder_openh264_annexb",
            "xcap_openh264_annexb",
            "xcap_snapshot_fallback"
        ])
    } else {
        json!([
            "xcap_video_recorder_openh264_annexb",
            "xcap_openh264_annexb",
            "xcap_snapshot_fallback"
        ])
    };
    let codec_profiles = if production_ready {
        json!([
            {
                "codec": "h264",
                "profile": "baseline",
                "max_width": 3840,
                "max_height": 2160,
                "max_fps": max_fps,
                "max_bitrate_kbps": NATIVE_MAX_BITRATE_KBPS,
                "hardware_accelerated": true,
                "scalability_mode": "none",
                "backend_id": MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID,
            },
            {
                "codec": "h264",
                "profile": "baseline",
                "max_width": 1920,
                "max_height": 1080,
                "max_fps": XCAP_MACOS_RECORDER_MAX_FPS,
                "max_bitrate_kbps": DEFAULT_VIDEO_STREAM_BITRATE_KBPS,
                "hardware_accelerated": false,
                "scalability_mode": "none",
                "backend_id": XCAP_OPENH264_BACKEND_ID,
            }
        ])
    } else {
        json!([
            {
                "codec": "h264",
                "profile": "baseline",
                "max_width": 1920,
                "max_height": 1080,
                "max_fps": XCAP_MACOS_RECORDER_MAX_FPS,
                "max_bitrate_kbps": DEFAULT_VIDEO_STREAM_BITRATE_KBPS,
                "hardware_accelerated": false,
                "scalability_mode": "none",
                "backend_id": XCAP_OPENH264_BACKEND_ID,
            }
        ])
    };
    let display_capture_source = if production_ready {
        "screencapturekit_stream"
    } else {
        "xcap_video_recorder"
    };
    let display_capture_api = if production_ready {
        "macos.screencapturekit"
    } else {
        "xcap.avcapture_screen_input"
    };
    let reason = if production_ready {
        "native ScreenCaptureKit/VideoToolbox WebRTC backend is available for display capture"
    } else {
        "current builtin backend is capped by xcap macOS recorder; 144Hz requires the ScreenCaptureKit/VideoToolbox plugin backend"
    };
    json!({
        "capture_backends": capture_backends,
        "media_sdk": sdk_contract_view(),
        "media_backends": backend_catalog_view(),
        "production_gate": production_gate_view(),
        "codec_profiles": codec_profiles,
        "hardware_cursor": false,
        "input_injection": input_injection_available(),
        "data_channel_input": true,
        "input_channel_label": INPUT_DATA_CHANNEL_LABEL,
        "supported_input_events": ["pointer.move", "pointer.down", "pointer.up", "key.down", "key.up"],
        "input_plane": {
            "kind": "webrtc_data_channel",
            "label": INPUT_DATA_CHANNEL_LABEL,
            "low_latency": true,
            "supported_events": ["pointer.move", "pointer.down", "pointer.up", "key.down", "key.up"],
        },
        "low_latency_mode": true,
        "max_fps": max_fps,
        "requested_fps_ceiling": MAX_ATTACH_FPS,
        "max_width": 1920,
        "max_height": 1080,
        "metadata": {
            "production_media_endpoint": "direct_webrtc_h264",
            "diagnostic_media_endpoint": "builtin_openh264_annexb",
            "display_capture_source": display_capture_source,
            "display_capture_api": display_capture_api,
            "webrtc_endpoint": "device_side_peer_connection",
            "next_required_backend": MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID,
            "reason": reason,
        }
    })
}

/// Empty latest-metrics DTO used until runtime metrics arrive.
pub(in crate::plugins::builtin::remote_desktop) fn empty_pipeline_metrics() -> Value {
    json!({
        "observed_fps": 0,
        "capture_to_encode_ms": 0,
        "encode_to_network_ms": 0,
        "jitter_buffer_ms": 0,
        "decode_to_present_ms": 0,
        "estimated_glass_to_glass_ms": 0,
        "frame_queue_depth": 0,
        "packet_loss_ratio": 0.0,
        "dropped_frames": 0,
    })
}
