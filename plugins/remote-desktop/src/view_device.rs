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
    input_injection_available, input_injection_backend, input_injection_unavailable_reason,
    unsupported_input_channel_types_value, INPUT_DATA_CHANNEL_LABEL,
};
#[cfg(test)]
use crate::daemon::plugins::remote_desktop::media::host_audio::{
    HostAudioSourcePlan, HostAudioSourcePlanError,
};
use crate::daemon::plugins::remote_desktop::media::host_audio_capability::HostAudioRuntimeSnapshot;
#[cfg(target_os = "macos")]
use crate::daemon::plugins::remote_desktop::media::host_audio_capability::HostAudioSourceClass;
#[cfg(any(test, not(target_os = "macos")))]
use crate::daemon::plugins::remote_desktop::media::host_audio_capability::REASON_ACTIVE_MEDIA_SESSION_AUDIO_UNAVAILABLE;
use crate::daemon::plugins::remote_desktop::media::{
    backend_catalog_view, native_webrtc_backend_runtime_descriptor, production_gate_view,
    sdk_contract_view, MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID, XCAP_MACOS_RECORDER_MAX_FPS,
    XCAP_OPENH264_BACKEND_ID, XCAP_OPENH264_WEBRTC_BACKEND,
};
use crate::daemon::plugins::remote_desktop::target::{
    target_scoped_input_guard_available, target_scoped_input_guard_unavailable_reason,
};

#[cfg(any(test, not(target_os = "macos")))]
pub(in crate::daemon::plugins::remote_desktop) const AUDIO_UNSUPPORTED_REASON: &str =
    REASON_ACTIVE_MEDIA_SESSION_AUDIO_UNAVAILABLE;
const PLATFORM_REASON_MACOS_NATIVE_BACKEND_READY: &str =
    "macos_screencapturekit_videotoolbox_ready";
const PLATFORM_REASON_LINUX_XCAP_BASELINE_READY: &str = "linux_xcap_target_baseline_ready";
const PLATFORM_REASON_WINDOWS_XCAP_BASELINE_READY: &str = "windows_xcap_target_baseline_ready";
const INPUT_REASON_MACOS_PERMISSION_GRANTED: &str = "macos_accessibility_permission_granted";
const INPUT_REASON_MACOS_TARGET_GUARD_READY: &str = "macos_target_input_guard_ready";
const INPUT_REASON_LINUX_X11_DISPLAY_READY: &str = "linux_x11_xcb_atomic_display_global_ready";
const INPUT_REASON_LINUX_X11_TARGET_UNISOLATED: &str =
    "linux_x11_xtest_cannot_isolate_press_release_to_target";
const INPUT_REASON_WINDOWS_READY: &str = "windows_sendinput_target_guard_ready";

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
pub(in crate::daemon::plugins::remote_desktop) fn device_capabilities_view(
    audio_runtime: &HostAudioRuntimeSnapshot,
) -> Value {
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
    let audio = audio_support_view(audio_runtime);
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
        "multi_surface_application_window_set",
        "process_scoped_application_window_set"
    ]);
    let reason = if production_ready {
        "native ScreenCaptureKit/VideoToolbox WebRTC backend is available for display/window/application target capture"
    } else {
        "current builtin display/window/application target backend is capped by the xcap macOS recorder; 144Hz requires the ScreenCaptureKit/VideoToolbox plugin backend"
    };
    let platform_support = platform_support_view(production_ready, &production_backend);
    let input_available = input_injection_available();
    let target_local_guard_available = target_scoped_input_guard_available();
    let input_control_support =
        input_control_support_view(input_available, target_local_guard_available);
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
        "receiver_feedback_openh264_rebuild"
    };

    let audio_offer_ready = audio["offer_ready"] == json!(true);
    let mut product_blockers = vec![json!("remoteapp_media_adaptation_e2e_artifact_missing")];
    if !audio_offer_ready {
        product_blockers.insert(0, audio["blocked_reason"].clone());
    }
    json!({
        "schema_version": 1,
        "media_scope": if audio_offer_ready { "audio_video" } else { "video_only" },
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

fn input_support(status: &str, reason: &str, scope: Value, backend: &str) -> Value {
    json!({
        "status": status,
        "reason": reason,
        "scope": scope,
        "backend": backend,
        "certification": "live_e2e_required",
    })
}

fn input_control_support_view(input_available: bool, target_local_guard_available: bool) -> Value {
    let current_os = std::env::consts::OS;
    let runtime_backend = input_injection_backend();
    let runtime_blocked_reason = input_injection_unavailable_reason();
    let platform_row = |platform: &str,
                        non_current_status: &str,
                        backend: &str,
                        ready_reason: &str,
                        scope: Value,
                        requires_target_guard: bool| {
        if current_os == platform {
            let target_guard_blocked = requires_target_guard && !target_local_guard_available;
            input_support(
                if target_guard_blocked {
                    "unavailable"
                } else if input_available {
                    "available"
                } else if platform == "macos" {
                    "permission_denied"
                } else {
                    "unavailable"
                },
                if target_guard_blocked {
                    target_scoped_input_guard_unavailable_reason()
                } else if input_available {
                    ready_reason
                } else {
                    runtime_blocked_reason.unwrap_or("input_injection_unavailable")
                },
                scope,
                runtime_backend,
            )
        } else {
            input_support(non_current_status, ready_reason, scope, backend)
        }
    };

    json!({
        "schema_version": 1,
        "current_host_os": current_os,
        "runtime_backend": runtime_backend,
        "runtime_available": input_available,
        "runtime_blocked_reason": runtime_blocked_reason,
        "target_local_guard_compiled": target_local_guard_available,
        "target_local_runtime_available": input_available && target_local_guard_available,
        "target_local_runtime_blocked_reason": if !target_local_guard_available {
            json!(target_scoped_input_guard_unavailable_reason())
        } else {
            runtime_blocked_reason.map_or(Value::Null, Value::from)
        },
        "requires_input_control_consent": true,
        "input_transport": "webrtc_data_channel",
        "platforms": {
            "macos": {
                "display": platform_row("macos", "implementation_ready", "macos_coregraphics_cgevent", INPUT_REASON_MACOS_PERMISSION_GRANTED, json!("display_global"), false),
                "window": platform_row("macos", "implementation_ready", "macos_coregraphics_cgevent", INPUT_REASON_MACOS_TARGET_GUARD_READY, json!("target_local"), true),
                "application": platform_row("macos", "implementation_ready", "macos_coregraphics_cgevent", INPUT_REASON_MACOS_TARGET_GUARD_READY, json!("target_local"), true),
            },
            "linux": {
                "display": platform_row("linux", "x11_display_global_ready", "linux_x11_xcb_atomic_xtest", INPUT_REASON_LINUX_X11_DISPLAY_READY, json!("display_global"), false),
                "window": platform_row("linux", "view_only_only", "linux_x11_xcb_atomic_xtest", INPUT_REASON_LINUX_X11_TARGET_UNISOLATED, json!("view_only"), true),
                "application": platform_row("linux", "view_only_only", "linux_x11_xcb_atomic_xtest", INPUT_REASON_LINUX_X11_TARGET_UNISOLATED, json!("view_only"), true),
            },
            "windows": {
                "display": platform_row("windows", "baseline_ready", "windows_user32_sendinput", INPUT_REASON_WINDOWS_READY, json!("display_global"), false),
                "window": platform_row("windows", "baseline_ready", "windows_user32_sendinput", INPUT_REASON_WINDOWS_READY, json!("target_local"), true),
                "application": platform_row("windows", "baseline_ready", "windows_user32_sendinput", INPUT_REASON_WINDOWS_READY, json!("target_local"), true),
            },
        },
        "environment_constraints": {
            "linux": "X11/XTest supports display-global input only; Window/Application input stays view-only until a target-bound press/release device exists. Pure Wayland requires xdg-desktop-portal RemoteDesktop plus libei.",
            "windows": "SendInput is subject to UIPI integrity-level restrictions",
        },
        "non_claim": "executable input backends do not replace live Windows/Linux/macOS OS input injection E2E evidence",
    })
}

fn target_support(status: &str, backend: Value, reason: &str) -> Value {
    json!({
        "status": status,
        "backend": backend,
        "reason": reason,
        "certification": "live_e2e_required",
    })
}

fn application_target_support(
    status: &str,
    backend: Value,
    reason: &str,
    scope: &str,
    multi_display: bool,
    blocked_reason: Option<&str>,
) -> Value {
    let mut support = target_support(status, backend, reason);
    support["application_surface"] = json!({
        "scope": scope,
        "multi_window": true,
        "multi_display": multi_display,
        "blocked_reason": blocked_reason,
    });
    support
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
        "schema_version": 2,
        "current_host_os": std::env::consts::OS,
        "platforms": {
            "macos": {
                "display": target_support(macos_status, macos_backend.clone(), macos_reason),
                "window": target_support(macos_status, macos_backend.clone(), macos_reason),
                "application": application_target_support(
                    macos_status,
                    macos_backend,
                    macos_reason,
                    "multi_surface",
                    true,
                    None,
                ),
            },
            "linux": {
                "display": target_support(
                    "baseline_ready",
                    json!(XCAP_OPENH264_WEBRTC_BACKEND.backend_id()),
                    PLATFORM_REASON_LINUX_XCAP_BASELINE_READY
                ),
                "window": target_support(
                    "baseline_ready",
                    json!(XCAP_OPENH264_WEBRTC_BACKEND.backend_id()),
                    PLATFORM_REASON_LINUX_XCAP_BASELINE_READY
                ),
                "application": application_target_support(
                    "baseline_ready",
                    json!(XCAP_OPENH264_WEBRTC_BACKEND.backend_id()),
                    PLATFORM_REASON_LINUX_XCAP_BASELINE_READY,
                    "process_scoped",
                    true,
                    None,
                ),
            },
            "windows": {
                "display": target_support(
                    "baseline_ready",
                    json!(XCAP_OPENH264_WEBRTC_BACKEND.backend_id()),
                    PLATFORM_REASON_WINDOWS_XCAP_BASELINE_READY
                ),
                "window": target_support(
                    "baseline_ready",
                    json!(XCAP_OPENH264_WEBRTC_BACKEND.backend_id()),
                    PLATFORM_REASON_WINDOWS_XCAP_BASELINE_READY
                ),
                "application": application_target_support(
                    "baseline_ready",
                    json!(XCAP_OPENH264_WEBRTC_BACKEND.backend_id()),
                    PLATFORM_REASON_WINDOWS_XCAP_BASELINE_READY,
                    "process_scoped",
                    true,
                    None,
                ),
            },
        },
        "non_claim": "platform support metadata does not replace live cross-platform capture E2E evidence",
    })
}

pub(in crate::daemon::plugins::remote_desktop) fn audio_support_view(
    runtime: &HostAudioRuntimeSnapshot,
) -> Value {
    #[cfg(target_os = "macos")]
    {
        let ready = runtime.compiled_supported()
            && runtime.runtime_reachable()
            && runtime.is_fresh()
            && runtime
                .source(HostAudioSourceClass::SystemLoopback)
                .is_ready();
        json!({
            "supported": runtime.compiled_supported(),
            "offer_ready": ready,
            "capture_ready": ready,
            "send_ready": false,
            "runtime_reachable": runtime.runtime_reachable(),
            "runtime_generation": runtime.generation(),
            "runtime_observed_at_ms": runtime.observed_at_ms(),
            "runtime_expires_at_ms": runtime.expires_at_ms(),
            "system_loopback_available": ready,
            "process_loopback_available": runtime
                .source(HostAudioSourceClass::ProcessTreeLoopback)
                .is_ready(),
            "target_admissible": Value::Null,
            "supported_target_kinds": ["display", "window", "application"],
            "unsupported_target_kinds": [],
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
                json!(runtime
                    .admission_blocker(HostAudioSourceClass::SystemLoopback)
                    .unwrap_or("native_media_backend_not_ready"))
            },
            "runtime_probe_detail": runtime.diagnostic_detail(),
            "transport": "webrtc",
            "non_claim": "offer readiness does not claim sender packets or browser-decoded host audio",
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        json!({
            "supported": false,
            "offer_ready": false,
            "capture_ready": false,
            "send_ready": false,
            "runtime_reachable": false,
            "runtime_generation": runtime.generation(),
            "runtime_observed_at_ms": runtime.observed_at_ms(),
            "runtime_expires_at_ms": runtime.expires_at_ms(),
            "system_loopback_available": false,
            "process_loopback_available": false,
            "target_admissible": false,
            "supported_target_kinds": [],
            "unsupported_target_kinds": [],
            "codec_profiles": [],
            "blocked_reason": AUDIO_UNSUPPORTED_REASON,
            "runtime_probe_detail": runtime.diagnostic_detail(),
            "transport": Value::Null,
            "non_claim": "the canonical media-host session cannot emit validator-checked Opus on this platform",
        })
    }
}

pub(in crate::daemon::plugins::remote_desktop) fn audio_support_view_for_binding(
    binding: &crate::daemon::plugins::remote_desktop::target::RemoteAppTargetBinding,
    runtime: &HostAudioRuntimeSnapshot,
) -> Value {
    let mut support = audio_support_view(runtime);
    let _ = binding;
    if support["supported"] == json!(true) {
        support["target_admissible"] = json!(true);
    }
    support
}

#[cfg(test)]
fn apply_target_audio_admission(
    mut support: Value,
    runtime: &HostAudioRuntimeSnapshot,
    admission: Result<HostAudioSourcePlan, HostAudioSourcePlanError>,
) -> Value {
    match admission {
        Ok(plan) => {
            let source = runtime.source(plan.source_class());
            let offer_ready = support["supported"] == json!(true)
                && runtime.runtime_reachable()
                && runtime.is_fresh()
                && source.is_ready();
            support["offer_ready"] = json!(offer_ready);
            support["capture_ready"] = json!(offer_ready);
            support["send_ready"] = json!(false);
            support["target_admissible"] = json!(true);
            support["target_source"] = json!(plan.source_label());
            support["blocked_reason"] = runtime
                .admission_blocker(plan.source_class())
                .map(|reason| json!(reason))
                .unwrap_or(Value::Null);
        }
        Err(error) => {
            support["offer_ready"] = json!(false);
            support["capture_ready"] = json!(false);
            support["send_ready"] = json!(false);
            support["target_admissible"] = json!(false);
            support["target_source"] = Value::Null;
            support["blocked_reason"] = json!(error.reason_code());
        }
    }
    support
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
    use crate::daemon::plugins::remote_desktop::media::host_audio::{
        HostAudioSourcePlan, HostAudioSourcePlanError,
    };
    use crate::daemon::plugins::remote_desktop::media::host_audio_capability::{
        HostAudioRuntimeSnapshot, REASON_HOST_AUDIO_SNAPSHOT_EXPIRED,
        REASON_PIPEWIRE_RUNTIME_UNAVAILABLE,
    };
    use crate::daemon::plugins::remote_desktop::target::RemoteDesktopTargetKind;

    use super::{
        apply_target_audio_admission, device_capabilities_view, input_control_support_view,
        target_scoped_input_guard_available, AUDIO_UNSUPPORTED_REASON,
    };

    fn test_host_audio_runtime() -> HostAudioRuntimeSnapshot {
        HostAudioRuntimeSnapshot::for_test(true, true, true, true, None)
    }

    #[test]
    fn device_capabilities_report_clipboard_and_file_transfer_unsupported() {
        let capabilities = device_capabilities_view(&test_host_audio_runtime());

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
        if capabilities["audio"]["supported"] == json!(true) {
            assert_eq!(
                capabilities["unsupported_capabilities"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
        } else {
            assert_eq!(
                capabilities["unsupported_capabilities"][2]["capability"],
                json!("host_audio")
            );
            assert_eq!(
                capabilities["unsupported_capabilities"][2]["reason"],
                capabilities["audio"]["blocked_reason"]
            );
        }
    }

    #[test]
    fn device_capabilities_report_platform_host_audio_support() {
        let capabilities = device_capabilities_view(&test_host_audio_runtime());

        if capabilities["audio"]["supported"] == json!(true) {
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
        } else {
            assert_eq!(capabilities["audio"]["supported"], json!(false));
            assert_eq!(capabilities["audio"]["capture_ready"], json!(false));
            assert_eq!(capabilities["audio"]["send_ready"], json!(false));
            assert_eq!(
                capabilities["audio"]["blocked_reason"],
                json!(AUDIO_UNSUPPORTED_REASON)
            );
            assert_eq!(capabilities["audio"]["codec_profiles"], json!([]));
        }
    }

    #[test]
    fn target_audio_capability_uses_exact_source_admission_and_fails_closed() {
        let runtime = test_host_audio_runtime();
        let ready_support = || {
            json!({
                "supported": true,
                "offer_ready": true,
                "capture_ready": true,
                "send_ready": false,
                "blocked_reason": null,
            })
        };
        let windows_process = apply_target_audio_admission(
            ready_support(),
            &runtime,
            HostAudioSourcePlan::for_target(
                "windows",
                RemoteDesktopTargetKind::Application,
                Some(4242),
            ),
        );
        assert_eq!(windows_process["supported"], json!(true));
        assert_eq!(windows_process["offer_ready"], json!(true));
        assert_eq!(windows_process["capture_ready"], json!(true));
        assert_eq!(windows_process["send_ready"], json!(false));
        assert_eq!(windows_process["target_admissible"], json!(true));
        assert_eq!(
            windows_process["target_source"],
            json!("process_tree_loopback")
        );

        let linux_process = apply_target_audio_admission(
            ready_support(),
            &runtime,
            HostAudioSourcePlan::for_target(
                "linux",
                RemoteDesktopTargetKind::Application,
                Some(4242),
            ),
        );
        assert_eq!(linux_process["supported"], json!(true));
        assert_eq!(linux_process["offer_ready"], json!(true));
        assert_eq!(linux_process["capture_ready"], json!(true));
        assert_eq!(linux_process["send_ready"], json!(false));
        assert_eq!(linux_process["target_admissible"], json!(true));
        assert_eq!(
            linux_process["target_source"],
            json!("process_tree_loopback")
        );

        for error in [HostAudioSourcePlanError::TargetPidMissing {
            target_kind: "window",
        }] {
            let blocked =
                apply_target_audio_admission(ready_support(), &runtime, Err(error.clone()));
            assert_eq!(blocked["supported"], json!(true));
            assert_eq!(blocked["offer_ready"], json!(false));
            assert_eq!(blocked["capture_ready"], json!(false));
            assert_eq!(blocked["send_ready"], json!(false));
            assert_eq!(blocked["target_admissible"], json!(false));
            assert_eq!(blocked["target_source"], json!(null));
            assert_eq!(blocked["blocked_reason"], json!(error.reason_code()));
        }
    }

    #[test]
    fn compiled_host_audio_stays_supported_when_runtime_is_unreachable() {
        let runtime = HostAudioRuntimeSnapshot::for_test(
            true,
            false,
            false,
            false,
            Some(REASON_PIPEWIRE_RUNTIME_UNAVAILABLE),
        );
        let support = super::audio_support_view(&runtime);

        assert_eq!(support["supported"], json!(true));
        assert_eq!(support["runtime_reachable"], json!(false));
        assert_eq!(support["offer_ready"], json!(false));
        assert_eq!(support["capture_ready"], json!(false));
        assert_eq!(support["send_ready"], json!(false));
        assert_eq!(
            support["blocked_reason"],
            json!(REASON_PIPEWIRE_RUNTIME_UNAVAILABLE)
        );
    }

    #[test]
    fn quiet_linux_process_source_does_not_require_a_default_sink() {
        let runtime = HostAudioRuntimeSnapshot::for_test(
            true,
            true,
            false,
            true,
            Some("pipewire_default_output_sink_unavailable"),
        );
        let support = apply_target_audio_admission(
            json!({
                "supported": true,
                "offer_ready": true,
                "capture_ready": true,
                "send_ready": false,
                "blocked_reason": null,
            }),
            &runtime,
            HostAudioSourcePlan::for_target(
                "linux",
                RemoteDesktopTargetKind::Application,
                Some(4242),
            ),
        );

        assert_eq!(support["target_admissible"], json!(true));
        assert_eq!(support["offer_ready"], json!(true));
        assert_eq!(support["target_source"], json!("process_tree_loopback"));
    }

    #[test]
    fn expired_ready_snapshot_cannot_authorize_an_audio_offer() {
        let mut runtime = test_host_audio_runtime();
        runtime.expire_for_test();
        let support = apply_target_audio_admission(
            json!({
                "supported": true,
                "offer_ready": true,
                "capture_ready": true,
                "send_ready": false,
                "blocked_reason": null,
            }),
            &runtime,
            HostAudioSourcePlan::for_target(
                "linux",
                RemoteDesktopTargetKind::Application,
                Some(4242),
            ),
        );

        assert_eq!(support["supported"], json!(true));
        assert_eq!(support["target_admissible"], json!(true));
        assert_eq!(support["offer_ready"], json!(false));
        assert_eq!(support["capture_ready"], json!(false));
        assert_eq!(
            support["blocked_reason"],
            json!(REASON_HOST_AUDIO_SNAPSHOT_EXPIRED)
        );
    }

    #[test]
    fn device_capabilities_project_native_target_subject_matrix() {
        let capabilities = device_capabilities_view(&test_host_audio_runtime());

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
            json!(["display", "window", "application"])
        );
        assert_eq!(
            capabilities["metadata"]["capture_target_models"],
            json!([
                "display_surface",
                "window_surface",
                "multi_surface_application_window_set",
                "process_scoped_application_window_set"
            ])
        );
        assert!(capabilities["metadata"]["reason"]
            .as_str()
            .is_some_and(|message| message.contains("display/window/application")));
    }

    #[test]
    fn device_capabilities_project_cross_platform_support_matrix() {
        let capabilities = device_capabilities_view(&test_host_audio_runtime());
        let platform_support = &capabilities["metadata"]["platform_support"];

        assert_eq!(platform_support["schema_version"], json!(2));
        assert_eq!(
            platform_support["platforms"]["linux"]["display"]["status"],
            json!("baseline_ready")
        );
        assert_eq!(
            platform_support["platforms"]["linux"]["display"]["backend"],
            json!("builtin.xcap.openh264.webrtc.v1")
        );
        assert_eq!(
            platform_support["platforms"]["linux"]["window"]["status"],
            json!("baseline_ready")
        );
        assert_eq!(
            platform_support["platforms"]["linux"]["application"]["reason"],
            json!("linux_xcap_target_baseline_ready")
        );
        assert_eq!(
            platform_support["platforms"]["linux"]["application"]["application_surface"],
            json!({
                "scope": "process_scoped",
                "multi_window": true,
                "multi_display": true,
                "blocked_reason": null,
            })
        );
        assert_eq!(
            platform_support["platforms"]["windows"]["display"]["status"],
            json!("baseline_ready")
        );
        assert_eq!(
            platform_support["platforms"]["windows"]["window"]["reason"],
            json!("windows_xcap_target_baseline_ready")
        );
        assert_eq!(
            platform_support["platforms"]["windows"]["application"]["status"],
            json!("baseline_ready")
        );
        assert_eq!(
            platform_support["platforms"]["windows"]["application"]["application_surface"]
                ["multi_display"],
            json!(true)
        );
        assert_eq!(
            platform_support["platforms"]["macos"]["application"]["application_surface"],
            json!({
                "scope": "multi_surface",
                "multi_window": true,
                "multi_display": true,
                "blocked_reason": null,
            })
        );
        assert_eq!(
            platform_support["platforms"]["macos"]["application"]["certification"],
            json!("live_e2e_required")
        );
        assert!(platform_support["non_claim"]
            .as_str()
            .is_some_and(|message| message.contains("live cross-platform capture E2E")));
    }

    #[test]
    fn device_capabilities_project_input_control_support_matrix() {
        let capabilities = device_capabilities_view(&test_host_audio_runtime());
        let input_support = &capabilities["metadata"]["input_control_support"];

        assert_eq!(input_support["schema_version"], json!(1));
        assert_eq!(
            input_support["runtime_available"],
            capabilities["input_injection"]
        );
        assert_eq!(
            input_support["target_local_guard_compiled"],
            json!(target_scoped_input_guard_available())
        );
        assert_eq!(
            input_support["target_local_runtime_available"],
            json!(
                capabilities["input_injection"] == json!(true)
                    && target_scoped_input_guard_available()
            )
        );
        assert_eq!(input_support["requires_input_control_consent"], json!(true));
        assert_eq!(
            input_support["input_transport"],
            json!("webrtc_data_channel")
        );
        if std::env::consts::OS == "macos" && capabilities["input_injection"] == json!(true) {
            assert_eq!(
                input_support["platforms"]["macos"]["display"]["status"],
                json!("available")
            );
            assert_eq!(
                input_support["platforms"]["macos"]["display"]["scope"],
                json!("display_global")
            );
        } else if std::env::consts::OS == "macos" {
            assert_eq!(
                input_support["platforms"]["macos"]["display"]["status"],
                json!("permission_denied")
            );
            assert_eq!(
                input_support["platforms"]["macos"]["display"]["reason"],
                json!("accessibility_permission_denied")
            );
        } else {
            assert_eq!(
                input_support["platforms"]["macos"]["display"]["status"],
                json!("implementation_ready")
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
            json!(if std::env::consts::OS != "macos"
                || capabilities["input_injection"] == json!(true)
            {
                "macos_target_input_guard_ready"
            } else {
                "accessibility_permission_denied"
            })
        );
        assert_eq!(
            input_support["platforms"]["linux"]["display"]["reason"],
            if std::env::consts::OS == "linux" && capabilities["input_injection"] != json!(true) {
                input_support["runtime_blocked_reason"].clone()
            } else {
                json!("linux_x11_xcb_atomic_display_global_ready")
            }
        );
        assert_eq!(
            input_support["platforms"]["windows"]["application"]["status"],
            json!(if std::env::consts::OS == "windows" {
                if capabilities["input_injection"] == json!(true) {
                    "available"
                } else {
                    "unavailable"
                }
            } else {
                "baseline_ready"
            })
        );
        assert_eq!(
            input_support["platforms"]["windows"]["application"]["backend"],
            json!("windows_user32_sendinput")
        );
        assert_eq!(
            input_support["platforms"]["linux"]["window"]["scope"],
            json!("view_only")
        );
        assert_eq!(
            input_support["platforms"]["linux"]["application"]["reason"],
            json!(if std::env::consts::OS == "linux" {
                "target_scoped_keyboard_pointer_dispatch_unsafe"
            } else {
                "linux_x11_xtest_cannot_isolate_press_release_to_target"
            })
        );
        assert!(input_support["non_claim"].as_str().is_some_and(
            |message| message.contains("live Windows/Linux/macOS OS input injection E2E")
        ));
    }

    #[test]
    fn input_capability_keeps_display_global_but_blocks_target_local_without_guard() {
        let support = input_control_support_view(true, false);
        let current = &support["platforms"][std::env::consts::OS];

        assert_eq!(support["runtime_available"], json!(true));
        assert_eq!(support["target_local_guard_compiled"], json!(false));
        assert_eq!(support["target_local_runtime_available"], json!(false));
        assert_eq!(current["display"]["status"], json!("available"));
        for target_kind in ["window", "application"] {
            assert_eq!(current[target_kind]["status"], json!("unavailable"));
            assert_eq!(current[target_kind]["scope"], json!("target_local"));
            assert_eq!(
                current[target_kind]["reason"],
                json!("target_scoped_keyboard_pointer_dispatch_unsafe")
            );
        }
    }

    #[test]
    fn device_capabilities_project_media_pipeline_support_matrix() {
        let capabilities = device_capabilities_view(&test_host_audio_runtime());
        let media_support = &capabilities["metadata"]["media_pipeline_support"];

        assert_eq!(media_support["schema_version"], json!(1));
        let audio_offer_ready = capabilities["audio"]["offer_ready"] == json!(true);
        assert_eq!(
            media_support["media_scope"],
            json!(if audio_offer_ready {
                "audio_video"
            } else {
                "video_only"
            })
        );
        assert_eq!(media_support["product_ready"], json!(false));
        if audio_offer_ready {
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
                json!("receiver_feedback_openh264_rebuild")
            );
        }
        assert_eq!(media_support["audio"], capabilities["audio"]);
        assert!(media_support["non_claim"]
            .as_str()
            .is_some_and(|message| { message.contains("live codec/audio/adaptation E2E") }));
    }
}
