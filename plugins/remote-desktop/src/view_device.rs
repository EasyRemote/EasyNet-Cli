// EasyNet CLI — remote desktop device view projection
// ====================================================
//
// File: plugins/remote-desktop/src/view_device.rs
// Description: Device capability, quality target, and metric DTO projection.

use serde_json::{json, Value};

use crate::daemon::plugins::remote_desktop::constants::{
    DEFAULT_CAPTURE_TO_ENCODE_MS, DEFAULT_FRAME_QUEUE_DEPTH, DEFAULT_GLASS_TO_GLASS_LATENCY_MS,
    DEFAULT_MIN_ACCEPTABLE_FPS, DEFAULT_TARGET_BITRATE_KBPS, DEFAULT_TARGET_FPS,
    DEFAULT_VIDEO_STREAM_BITRATE_KBPS, MAX_ATTACH_FPS, NATIVE_MAX_BITRATE_KBPS,
};
use crate::daemon::plugins::remote_desktop::input::{
    input_injection_available, unsupported_input_channel_types_value, INPUT_DATA_CHANNEL_LABEL,
};
use crate::daemon::plugins::remote_desktop::media::{
    backend_catalog_view, native_webrtc_backend_runtime_descriptor, production_gate_view,
    sdk_contract_view, MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID, XCAP_MACOS_RECORDER_MAX_FPS,
    XCAP_OPENH264_BACKEND_ID, XCAP_OPENH264_WEBRTC_BACKEND,
};

#[cfg(not(target_os = "macos"))]
pub(in crate::daemon::plugins::remote_desktop) const AUDIO_UNSUPPORTED_REASON: &str =
    "host_audio_not_implemented";
const PLATFORM_REASON_MACOS_NATIVE_BACKEND_READY: &str =
    "macos_screencapturekit_videotoolbox_ready";
const PLATFORM_REASON_LINUX_DISPLAY_DIAGNOSTIC_ONLY: &str = "linux_display_diagnostic_only";
const PLATFORM_REASON_LINUX_APP_WINDOW_UNSUPPORTED: &str =
    "linux_app_window_native_backend_not_implemented";
const PLATFORM_REASON_WINDOWS_UNSUPPORTED: &str = "windows_native_backend_not_implemented";
const INPUT_REASON_MACOS_PERMISSION_GRANTED: &str = "macos_accessibility_permission_granted";
const INPUT_REASON_MACOS_PERMISSION_DENIED: &str = "macos_accessibility_permission_denied";
const INPUT_REASON_MACOS_TARGET_GUARD_READY: &str = "macos_target_input_guard_ready";
const INPUT_REASON_LINUX_UNSUPPORTED: &str = "linux_input_injection_backend_not_implemented";
const INPUT_REASON_WINDOWS_UNSUPPORTED: &str = "windows_input_injection_backend_not_implemented";

/// Build media quality targets from create-session video constraints.
pub(in crate::daemon::plugins::remote_desktop) fn quality_targets(video: &Value) -> Value {
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
pub(in crate::daemon::plugins::remote_desktop) fn device_capabilities_view() -> Value {
    let production_backend = native_webrtc_backend_runtime_descriptor();
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
            "xcap_snapshot_diagnostic"
        ])
    } else {
        json!([
            "xcap_video_recorder_openh264_annexb",
            "xcap_openh264_annexb",
            "xcap_snapshot_diagnostic"
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
    let audio = audio_support_view();
    let mut unsupported_capabilities = vec![
        json!({
            "capability": "clipboard",
            "reason": "split_ability_required",
            "future_abilities": [
                "remote_desktop.clipboard.read",
                "remote_desktop.clipboard.write",
                "remote_desktop.clipboard.watch"
            ]
        }),
        json!({
            "capability": "file_transfer",
            "reason": "split_ability_required",
            "future_abilities": [
                "remote_desktop.file_transfer.create",
                "remote_desktop.file_transfer.accept",
                "remote_desktop.file_transfer.send",
                "remote_desktop.file_transfer.cancel"
            ]
        }),
    ];
    if audio["supported"] != json!(true) {
        unsupported_capabilities.push(json!({
            "capability": "host_audio",
            "reason": audio["blocked_reason"],
        }));
    }
    let production_target_subjects = if production_ready {
        production_backend.supported_subjects_value()
    } else {
        json!([])
    };
    let diagnostic_target_subjects = XCAP_OPENH264_WEBRTC_BACKEND.supported_subjects_value();
    let capture_target_models = json!([
        "display_surface",
        "window_surface",
        "display_scoped_application_window_set"
    ]);
    let reason = if production_ready {
        "native ScreenCaptureKit/VideoToolbox WebRTC backend is available for display/window/application target capture"
    } else {
        "current builtin backend is capped by xcap macOS recorder; 144Hz requires the ScreenCaptureKit/VideoToolbox plugin backend"
    };
    let platform_support = platform_support_view(production_ready, &production_backend);
    let input_available = input_injection_available();
    let input_control_support = input_control_support_view(input_available);
    let media_pipeline_support =
        media_pipeline_support_view(production_ready, &production_backend, max_fps, &audio);
    json!({
        "capture_backends": capture_backends,
        "media_sdk": sdk_contract_view(),
        "media_backends": backend_catalog_view(),
        "production_gate": production_gate_view(),
        "codec_profiles": codec_profiles,
        "audio": audio.clone(),
        "hardware_cursor": false,
        "input_injection": input_available,
        "data_channel_input": true,
        "input_channel_label": INPUT_DATA_CHANNEL_LABEL,
        "supported_input_events": ["pointer.move", "pointer.down", "pointer.up", "key.down", "key.up"],
        "unsupported_input_types": unsupported_input_channel_types_value(),
        "unsupported_capabilities": unsupported_capabilities,
        "input_plane": {
            "kind": "webrtc_data_channel",
            "label": INPUT_DATA_CHANNEL_LABEL,
            "low_latency": true,
            "supported_events": ["pointer.move", "pointer.down", "pointer.up", "key.down", "key.up"],
            "unsupported_input_types": unsupported_input_channel_types_value(),
        },
        "low_latency_mode": true,
        "max_fps": max_fps,
        "requested_fps_ceiling": MAX_ATTACH_FPS,
        "max_width": 1920,
        "max_height": 1080,
        "metadata": {
            "production_media_endpoint": "direct_webrtc_h264",
            "diagnostic_media_endpoint": "builtin_openh264_annexb",
            "production_target_subjects": production_target_subjects,
            "diagnostic_target_subjects": diagnostic_target_subjects,
            "production_target_subjects_source": if production_ready {
                MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID
            } else {
                "none"
            },
            "production_target_subjects_blocked_reason": if production_ready {
                Value::Null
            } else {
                json!(production_backend
                    .unavailable_reason()
                    .unwrap_or("production_backend_not_ready"))
            },
            "platform_support": platform_support,
            "input_control_support": input_control_support,
            "media_pipeline_support": media_pipeline_support,
            "capture_target_models": capture_target_models,
            "display_capture_source": display_capture_source,
            "display_capture_api": display_capture_api,
            "webrtc_endpoint": "device_side_peer_connection",
            "next_required_backend": MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID,
            "reason": reason,
        }
    })
}

fn media_pipeline_support_view(
    production_ready: bool,
    production_backend: &crate::daemon::plugins::remote_desktop::media::RemoteDesktopMediaBackendDescriptor,
    max_fps: u32,
    audio: &Value,
) -> Value {
    let video_backend = if production_ready {
        *production_backend
    } else {
        XCAP_OPENH264_WEBRTC_BACKEND
    };
    let video_status = if production_ready {
        "production_ready"
    } else {
        "diagnostic_only"
    };
    let max_bitrate_kbps = if production_ready {
        NATIVE_MAX_BITRATE_KBPS
    } else {
        DEFAULT_VIDEO_STREAM_BITRATE_KBPS
    };
    let adaptation_policy = if production_ready {
        "native_bitrate_adaptation_from_webrtc_stats_and_encoder_pressure"
    } else {
        "static_bitrate_with_bounded_stale_frame_drop"
    };

    let audio_ready = audio["capture_ready"] == json!(true) && audio["send_ready"] == json!(true);
    let mut product_blockers = vec![json!("remoteapp_media_adaptation_e2e_artifact_missing")];
    if !audio_ready {
        product_blockers.insert(0, audio["blocked_reason"].clone());
    }
    json!({
        "schema_version": 1,
        "media_scope": if audio_ready { "audio_video" } else { "video_only" },
        "product_ready": false,
        "product_blockers": product_blockers,
        "video": {
            "status": video_status,
            "backend_id": video_backend.backend_id(),
            "codec": "h264",
            "payload_content_type": "video/h264",
            "transport": video_backend.carrier(),
            "capture_api": video_backend.capture_api(),
            "encoder": video_backend.encoder(),
            "transport_ready": video_backend.transport_ready(),
            "production_ready": production_ready,
            "max_capture_fps": max_fps,
            "requested_fps_ceiling": MAX_ATTACH_FPS,
            "target_bitrate_kbps": DEFAULT_TARGET_BITRATE_KBPS,
            "max_bitrate_kbps": max_bitrate_kbps,
            "max_frame_queue_depth": DEFAULT_FRAME_QUEUE_DEPTH,
            "drop_stale_frames": true,
            "backpressure_policy": "bounded_queue_drop_stale_frames",
            "adaptation_policy": adaptation_policy,
            "runtime_stats_required": true,
        },
        "audio": audio,
        "non_claim": "media pipeline support metadata does not replace live codec/audio/adaptation E2E evidence",
    })
}

fn input_support(status: &str, reason: &str, scope: Value) -> Value {
    json!({
        "status": status,
        "reason": reason,
        "scope": scope,
    })
}

fn input_control_support_view(input_available: bool) -> Value {
    let macos_display_status = if input_available {
        "available"
    } else {
        "permission_denied"
    };
    let macos_display_reason = if input_available {
        INPUT_REASON_MACOS_PERMISSION_GRANTED
    } else {
        INPUT_REASON_MACOS_PERMISSION_DENIED
    };
    let macos_target_reason = if input_available {
        INPUT_REASON_MACOS_TARGET_GUARD_READY
    } else {
        INPUT_REASON_MACOS_PERMISSION_DENIED
    };

    json!({
        "schema_version": 1,
        "current_host_os": std::env::consts::OS,
        "requires_input_control_consent": true,
        "input_transport": "webrtc_data_channel",
        "platforms": {
            "macos": {
                "display": input_support(macos_display_status, macos_display_reason, json!("display_global")),
                "window": input_support(macos_display_status, macos_target_reason, json!("target_local")),
                "application": input_support(macos_display_status, macos_target_reason, json!("target_local")),
            },
            "linux": {
                "display": input_support("unsupported", INPUT_REASON_LINUX_UNSUPPORTED, Value::Null),
                "window": input_support("unsupported", INPUT_REASON_LINUX_UNSUPPORTED, Value::Null),
                "application": input_support("unsupported", INPUT_REASON_LINUX_UNSUPPORTED, Value::Null),
            },
            "windows": {
                "display": input_support("unsupported", INPUT_REASON_WINDOWS_UNSUPPORTED, Value::Null),
                "window": input_support("unsupported", INPUT_REASON_WINDOWS_UNSUPPORTED, Value::Null),
                "application": input_support("unsupported", INPUT_REASON_WINDOWS_UNSUPPORTED, Value::Null),
            },
        },
        "non_claim": "input support metadata does not replace live OS input injection E2E evidence",
    })
}

fn target_support(status: &str, backend: Value, reason: &str) -> Value {
    json!({
        "status": status,
        "backend": backend,
        "reason": reason,
    })
}

fn platform_support_view(
    production_ready: bool,
    production_backend: &crate::daemon::plugins::remote_desktop::media::RemoteDesktopMediaBackendDescriptor,
) -> Value {
    let macos_status = if production_ready {
        "production_ready"
    } else {
        "blocked"
    };
    let macos_backend = if production_ready {
        json!(MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID)
    } else {
        Value::Null
    };
    let macos_reason = if production_ready {
        PLATFORM_REASON_MACOS_NATIVE_BACKEND_READY
    } else {
        production_backend
            .unavailable_reason()
            .unwrap_or("production_backend_not_ready")
    };

    json!({
        "schema_version": 1,
        "current_host_os": std::env::consts::OS,
        "platforms": {
            "macos": {
                "display": target_support(macos_status, macos_backend.clone(), macos_reason),
                "window": target_support(macos_status, macos_backend.clone(), macos_reason),
                "application": target_support(macos_status, macos_backend, macos_reason),
            },
            "linux": {
                "display": target_support(
                    "diagnostic_only",
                    json!(XCAP_OPENH264_WEBRTC_BACKEND.backend_id()),
                    PLATFORM_REASON_LINUX_DISPLAY_DIAGNOSTIC_ONLY
                ),
                "window": target_support(
                    "unsupported",
                    Value::Null,
                    PLATFORM_REASON_LINUX_APP_WINDOW_UNSUPPORTED
                ),
                "application": target_support(
                    "unsupported",
                    Value::Null,
                    PLATFORM_REASON_LINUX_APP_WINDOW_UNSUPPORTED
                ),
            },
            "windows": {
                "display": target_support("unsupported", Value::Null, PLATFORM_REASON_WINDOWS_UNSUPPORTED),
                "window": target_support("unsupported", Value::Null, PLATFORM_REASON_WINDOWS_UNSUPPORTED),
                "application": target_support("unsupported", Value::Null, PLATFORM_REASON_WINDOWS_UNSUPPORTED),
            },
        },
        "non_claim": "platform support metadata does not replace live cross-platform capture E2E evidence",
    })
}

pub(in crate::daemon::plugins::remote_desktop) fn audio_support_view() -> Value {
    #[cfg(target_os = "macos")]
    {
        let backend = native_webrtc_backend_runtime_descriptor();
        let ready = backend.production_ready();
        return json!({
            "supported": true,
            "capture_ready": ready,
            "send_ready": ready,
            "codec_profiles": [{
                "codec": "opus",
                "sample_rate_hz": 48000,
                "channels": 2,
                "frame_duration_ms": 20,
                "transport": "webrtc",
            }],
            "blocked_reason": if ready {
                Value::Null
            } else {
                json!(backend.unavailable_reason().unwrap_or("native_media_backend_not_ready"))
            },
            "transport": "webrtc",
            "non_claim": "capability metadata does not replace live decoded host-audio E2E evidence",
        });
    }
    #[cfg(not(target_os = "macos"))]
    json!({
        "supported": false,
        "capture_ready": false,
        "send_ready": false,
        "codec_profiles": [],
        "blocked_reason": AUDIO_UNSUPPORTED_REASON,
        "transport": Value::Null,
        "non_claim": "host audio remains unsupported on this platform",
    })
}

/// Empty latest-metrics DTO used until runtime metrics arrive.
pub(in crate::daemon::plugins::remote_desktop) fn empty_pipeline_metrics() -> Value {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::daemon::plugins::remote_desktop::constants::{
        DEFAULT_FRAME_QUEUE_DEPTH, DEFAULT_TARGET_BITRATE_KBPS,
    };

    use super::device_capabilities_view;

    #[test]
    fn device_capabilities_report_clipboard_and_file_transfer_unsupported() {
        let capabilities = device_capabilities_view();

        assert_eq!(
            capabilities["unsupported_input_types"],
            json!(["clipboard", "file_drop"])
        );
        assert_eq!(
            capabilities["input_plane"]["unsupported_input_types"],
            json!(["clipboard", "file_drop"])
        );
        assert_eq!(
            capabilities["unsupported_capabilities"][0]["capability"],
            json!("clipboard")
        );
        assert_eq!(
            capabilities["unsupported_capabilities"][0]["reason"],
            json!("split_ability_required")
        );
        assert_eq!(
            capabilities["unsupported_capabilities"][1]["capability"],
            json!("file_transfer")
        );
        assert_eq!(
            capabilities["unsupported_capabilities"][1]["future_abilities"][2],
            json!("remote_desktop.file_transfer.send")
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            capabilities["unsupported_capabilities"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(
                capabilities["unsupported_capabilities"][2]["capability"],
                json!("host_audio")
            );
            assert_eq!(
                capabilities["unsupported_capabilities"][2]["reason"],
                json!("host_audio_not_implemented")
            );
        }
    }

    #[test]
    fn device_capabilities_report_platform_host_audio_support() {
        let capabilities = device_capabilities_view();

        #[cfg(target_os = "macos")]
        {
            assert_eq!(capabilities["audio"]["supported"], json!(true));
            assert_eq!(
                capabilities["audio"]["codec_profiles"][0]["codec"],
                json!("opus")
            );
            assert_eq!(
                capabilities["audio"]["codec_profiles"][0]["sample_rate_hz"],
                json!(48_000)
            );
            assert_eq!(
                capabilities["audio"]["codec_profiles"][0]["channels"],
                json!(2)
            );
            assert_eq!(capabilities["audio"]["transport"], json!("webrtc"));
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(capabilities["audio"]["supported"], json!(false));
            assert_eq!(capabilities["audio"]["capture_ready"], json!(false));
            assert_eq!(capabilities["audio"]["send_ready"], json!(false));
            assert_eq!(
                capabilities["audio"]["blocked_reason"],
                json!("host_audio_not_implemented")
            );
            assert_eq!(capabilities["audio"]["codec_profiles"], json!([]));
        }
    }

    #[test]
    fn device_capabilities_project_native_target_subject_matrix() {
        let capabilities = device_capabilities_view();

        if capabilities["production_gate"]["ready"] == json!(true) {
            assert_eq!(
                capabilities["metadata"]["production_target_subjects"],
                json!(["display", "window", "application"])
            );
            assert_eq!(
                capabilities["metadata"]["production_target_subjects_source"],
                json!("plugin.macos.screencapturekit.videotoolbox.webrtc.v1")
            );
            assert_eq!(
                capabilities["metadata"]["production_target_subjects_blocked_reason"],
                json!(null)
            );
        } else {
            assert_eq!(
                capabilities["metadata"]["production_target_subjects"],
                json!([])
            );
            assert_eq!(
                capabilities["metadata"]["production_target_subjects_source"],
                json!("none")
            );
            assert!(
                capabilities["metadata"]["production_target_subjects_blocked_reason"]
                    .as_str()
                    .is_some_and(|reason| !reason.is_empty()),
                "closed production gate must expose why production app/window subjects are not claimable"
            );
        }
        assert_eq!(
            capabilities["metadata"]["diagnostic_target_subjects"],
            json!(["display"])
        );
        assert_eq!(
            capabilities["metadata"]["capture_target_models"],
            json!([
                "display_surface",
                "window_surface",
                "display_scoped_application_window_set"
            ])
        );
        assert!(capabilities["metadata"]["reason"]
            .as_str()
            .is_some_and(|message| message.contains("display/window/application")));
    }

    #[test]
    fn device_capabilities_project_cross_platform_support_matrix() {
        let capabilities = device_capabilities_view();
        let platform_support = &capabilities["metadata"]["platform_support"];

        assert_eq!(platform_support["schema_version"], json!(1));
        assert_eq!(
            platform_support["platforms"]["linux"]["display"]["status"],
            json!("diagnostic_only")
        );
        assert_eq!(
            platform_support["platforms"]["linux"]["display"]["backend"],
            json!("builtin.xcap.openh264.webrtc.v1")
        );
        assert_eq!(
            platform_support["platforms"]["linux"]["window"]["status"],
            json!("unsupported")
        );
        assert_eq!(
            platform_support["platforms"]["linux"]["application"]["reason"],
            json!("linux_app_window_native_backend_not_implemented")
        );
        assert_eq!(
            platform_support["platforms"]["windows"]["display"]["status"],
            json!("unsupported")
        );
        assert_eq!(
            platform_support["platforms"]["windows"]["window"]["reason"],
            json!("windows_native_backend_not_implemented")
        );
        assert_eq!(
            platform_support["platforms"]["windows"]["application"]["status"],
            json!("unsupported")
        );
        assert!(platform_support["non_claim"]
            .as_str()
            .is_some_and(|message| message.contains("live cross-platform capture E2E")));
    }

    #[test]
    fn device_capabilities_project_input_control_support_matrix() {
        let capabilities = device_capabilities_view();
        let input_support = &capabilities["metadata"]["input_control_support"];

        assert_eq!(input_support["schema_version"], json!(1));
        assert_eq!(input_support["requires_input_control_consent"], json!(true));
        assert_eq!(
            input_support["input_transport"],
            json!("webrtc_data_channel")
        );
        if capabilities["input_injection"] == json!(true) {
            assert_eq!(
                input_support["platforms"]["macos"]["display"]["status"],
                json!("available")
            );
            assert_eq!(
                input_support["platforms"]["macos"]["display"]["scope"],
                json!("display_global")
            );
        } else {
            assert_eq!(
                input_support["platforms"]["macos"]["display"]["status"],
                json!("permission_denied")
            );
            assert_eq!(
                input_support["platforms"]["macos"]["display"]["reason"],
                json!("macos_accessibility_permission_denied")
            );
        }
        assert_eq!(
            input_support["platforms"]["macos"]["window"]["status"],
            input_support["platforms"]["macos"]["display"]["status"]
        );
        assert_eq!(
            input_support["platforms"]["macos"]["window"]["scope"],
            json!("target_local")
        );
        assert_eq!(
            input_support["platforms"]["macos"]["application"]["reason"],
            json!(if capabilities["input_injection"] == json!(true) {
                "macos_target_input_guard_ready"
            } else {
                "macos_accessibility_permission_denied"
            })
        );
        assert_eq!(
            input_support["platforms"]["linux"]["display"]["reason"],
            json!("linux_input_injection_backend_not_implemented")
        );
        assert_eq!(
            input_support["platforms"]["windows"]["application"]["status"],
            json!("unsupported")
        );
        assert!(input_support["non_claim"]
            .as_str()
            .is_some_and(|message| message.contains("live OS input injection E2E")));
    }

    #[test]
    fn device_capabilities_project_media_pipeline_support_matrix() {
        let capabilities = device_capabilities_view();
        let media_support = &capabilities["metadata"]["media_pipeline_support"];

        assert_eq!(media_support["schema_version"], json!(1));
        let audio_ready = capabilities["audio"]["capture_ready"] == json!(true)
            && capabilities["audio"]["send_ready"] == json!(true);
        assert_eq!(
            media_support["media_scope"],
            json!(if audio_ready {
                "audio_video"
            } else {
                "video_only"
            })
        );
        assert_eq!(media_support["product_ready"], json!(false));
        if audio_ready {
            assert_eq!(
                media_support["product_blockers"],
                json!(["remoteapp_media_adaptation_e2e_artifact_missing"])
            );
        } else {
            assert_eq!(
                media_support["product_blockers"][0],
                capabilities["audio"]["blocked_reason"]
            );
            assert_eq!(
                media_support["product_blockers"][1],
                json!("remoteapp_media_adaptation_e2e_artifact_missing")
            );
        }
        assert_eq!(media_support["video"]["codec"], json!("h264"));
        assert_eq!(
            media_support["video"]["payload_content_type"],
            json!("video/h264")
        );
        assert_eq!(
            media_support["video"]["target_bitrate_kbps"],
            json!(DEFAULT_TARGET_BITRATE_KBPS)
        );
        assert_eq!(
            media_support["video"]["max_frame_queue_depth"],
            json!(DEFAULT_FRAME_QUEUE_DEPTH)
        );
        assert_eq!(media_support["video"]["drop_stale_frames"], json!(true));
        assert_eq!(
            media_support["video"]["backpressure_policy"],
            json!("bounded_queue_drop_stale_frames")
        );
        if capabilities["production_gate"]["ready"] == json!(true) {
            assert_eq!(media_support["video"]["status"], json!("production_ready"));
            assert_eq!(
                media_support["video"]["backend_id"],
                json!("plugin.macos.screencapturekit.videotoolbox.webrtc.v1")
            );
            assert_eq!(
                media_support["video"]["adaptation_policy"],
                json!("native_bitrate_adaptation_from_webrtc_stats_and_encoder_pressure")
            );
        } else {
            assert_eq!(media_support["video"]["status"], json!("diagnostic_only"));
            assert_eq!(
                media_support["video"]["backend_id"],
                json!("builtin.xcap.openh264.webrtc.v1")
            );
            assert_eq!(
                media_support["video"]["adaptation_policy"],
                json!("static_bitrate_with_bounded_stale_frame_drop")
            );
        }
        assert_eq!(media_support["audio"], capabilities["audio"]);
        assert!(media_support["non_claim"]
            .as_str()
            .is_some_and(|message| { message.contains("live codec/audio/adaptation E2E") }));
    }
}
