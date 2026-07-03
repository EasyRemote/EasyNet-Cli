// EasyNet CLI — remote desktop request parsing
// ============================================
//
// File: src/daemon/resources/remote_desktop/request.rs
// Description: Input argument parsing for remote desktop abilities.

use rand::RngCore as _;
use serde_json::{json, Value};

use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
    ScreenCaptureOptions, VideoResolution,
};
use crate::daemon::resources::remote_desktop::constants::{
    ABILITY_ATTACH_SESSION, ABILITY_CREATE_SESSION, ABILITY_SET_DESCRIPTION,
    ATTACH_ENCODING_ANNEXB_H264, ATTACH_ENCODING_JPEG_BINARY, DEFAULT_FRAME_QUEUE_DEPTH,
    DEFAULT_GLASS_TO_GLASS_LATENCY_MS, DEFAULT_LEASE_TTL_MS, DEFAULT_TARGET_BITRATE_KBPS,
    DEFAULT_TARGET_FPS, MAX_ATTACH_FPS, MAX_FRAME_QUEUE_DEPTH, MAX_LEASE_TTL_MS,
    MAX_VIDEO_DIMENSION, MIN_ATTACH_FPS, NATIVE_MAX_BITRATE_KBPS, NATIVE_MIN_BITRATE_KBPS,
    REASON_INVALID_ARGUMENT, TRANSPORT_INVOKE_BIDI, TRANSPORT_PREVIEW_STREAM, TRANSPORT_WEBRTC,
};
use crate::daemon::resources::remote_desktop::session::RemoteDesktopSession;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteDesktopVideoConstraints {
    max_width: u32,
    max_height: u32,
    max_fps: u32,
    max_bitrate_kbps: u32,
    scale_mode: String,
    region: String,
    codec_preferences: Vec<String>,
    target_latency_ms: u32,
    hardware_acceleration_required: bool,
    max_frame_queue_depth: u32,
    drop_stale_frames: bool,
}

impl RemoteDesktopVideoConstraints {
    pub(in crate::daemon::resources::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "max_width": self.max_width,
            "max_height": self.max_height,
            "max_fps": self.max_fps,
            "max_bitrate_kbps": self.max_bitrate_kbps,
            "scale_mode": self.scale_mode,
            "region": self.region,
            "codec_preferences": self.codec_preferences,
            "target_latency_ms": self.target_latency_ms,
            "hardware_acceleration_required": self.hardware_acceleration_required,
            "max_frame_queue_depth": self.max_frame_queue_depth,
            "drop_stale_frames": self.drop_stale_frames,
        })
    }

    fn max_fps(&self) -> u64 {
        self.max_fps as u64
    }

    fn resolution(&self) -> Option<VideoResolution> {
        Some(VideoResolution {
            width: self.max_width,
            height: self.max_height,
        })
        .filter(|resolution| resolution.width > 0 && resolution.height > 0)
    }

    fn bitrate_kbps(&self) -> u32 {
        self.max_bitrate_kbps
            .clamp(NATIVE_MIN_BITRATE_KBPS, NATIVE_MAX_BITRATE_KBPS)
    }

    fn frame_queue_depth(&self) -> usize {
        (self.max_frame_queue_depth as usize).clamp(1, MAX_FRAME_QUEUE_DEPTH as usize)
    }
}

impl Default for RemoteDesktopVideoConstraints {
    fn default() -> Self {
        Self {
            max_width: 1920,
            max_height: 1080,
            max_fps: DEFAULT_TARGET_FPS,
            max_bitrate_kbps: DEFAULT_TARGET_BITRATE_KBPS,
            scale_mode: "native".to_string(),
            region: String::new(),
            codec_preferences: vec!["h264".into(), "hevc".into(), "av1".into(), "vp9".into()],
            target_latency_ms: DEFAULT_GLASS_TO_GLASS_LATENCY_MS,
            hardware_acceleration_required: true,
            max_frame_queue_depth: DEFAULT_FRAME_QUEUE_DEPTH,
            drop_stale_frames: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RemoteDesktopInputPolicy {
    keyboard_enabled: bool,
    pointer_enabled: bool,
    clipboard_enabled: bool,
    file_drop_enabled: bool,
}

impl RemoteDesktopInputPolicy {
    pub(in crate::daemon::resources::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "keyboard_enabled": self.keyboard_enabled,
            "pointer_enabled": self.pointer_enabled,
            "clipboard_enabled": self.clipboard_enabled,
            "file_drop_enabled": self.file_drop_enabled,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttachEncoding {
    AnnexBH264,
    JpegBinary,
}

pub(in crate::daemon::resources::remote_desktop) fn require_str<'a>(
    args: &'a Value,
    key: &str,
    ability: &str,
) -> anyhow::Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("{ability}: `{key}` is required; reason={REASON_INVALID_ARGUMENT}")
        })
}

pub(in crate::daemon::resources::remote_desktop) fn validate_session_id(
    session_id: &str,
) -> anyhow::Result<()> {
    if session_id.is_empty()
        || session_id.len() > 128
        || session_id
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        anyhow::bail!(
            "{ABILITY_CREATE_SESSION}: session_id must be 1..128 visible non-whitespace characters; reason={REASON_INVALID_ARGUMENT}"
        );
    }
    Ok(())
}

pub(in crate::daemon::resources::remote_desktop) fn mint_session_id() -> String {
    let mut bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("rdp-{}", hex::encode(bytes))
}

pub(in crate::daemon::resources::remote_desktop) fn mint_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn parse_mode(args: &Value) -> anyhow::Result<String> {
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("view_only");
    match mode {
        "view_only" | "interactive" => Ok(mode.to_string()),
        _ => anyhow::bail!(
            "{ABILITY_CREATE_SESSION}: mode must be view_only or interactive; reason={REASON_INVALID_ARGUMENT}"
        ),
    }
}

pub(crate) fn parse_lease_ttl_ms(args: &Value) -> anyhow::Result<u64> {
    let ttl = match args.get("lease_ttl_ms").and_then(Value::as_u64) {
        Some(ttl) => ttl,
        None => args
            .get("requested_ttl_seconds")
            .and_then(Value::as_u64)
            .map(|seconds| seconds.saturating_mul(1000))
            .unwrap_or(DEFAULT_LEASE_TTL_MS),
    };
    if ttl == 0 || ttl > MAX_LEASE_TTL_MS {
        anyhow::bail!(
            "remote_desktop: lease_ttl_ms must be in 1..={MAX_LEASE_TTL_MS}; reason={REASON_INVALID_ARGUMENT}"
        );
    }
    Ok(ttl)
}

pub(crate) fn parse_transport_preferences(args: &Value) -> anyhow::Result<Vec<String>> {
    let Some(raw) = args.get("transport_preferences") else {
        return Ok(vec![
            TRANSPORT_WEBRTC.to_string(),
            TRANSPORT_INVOKE_BIDI.to_string(),
            TRANSPORT_PREVIEW_STREAM.to_string(),
        ]);
    };
    let transports = raw.as_array().ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_CREATE_SESSION}: transport_preferences must be an array; reason={REASON_INVALID_ARGUMENT}"
        )
    })?;
    let mut parsed = Vec::new();
    for item in transports {
        let name = item.as_str().ok_or_else(|| {
            anyhow::anyhow!(
                "{ABILITY_CREATE_SESSION}: transport_preferences entries must be strings; reason={REASON_INVALID_ARGUMENT}"
            )
        })?;
        match name {
            TRANSPORT_WEBRTC | TRANSPORT_INVOKE_BIDI | TRANSPORT_PREVIEW_STREAM => {
                if !parsed.iter().any(|existing| existing == name) {
                    parsed.push(name.to_string());
                }
            }
            _ => anyhow::bail!(
                "{ABILITY_CREATE_SESSION}: unsupported transport {name:?}; reason={REASON_INVALID_ARGUMENT}"
            ),
        }
    }
    if parsed.is_empty() {
        anyhow::bail!(
            "{ABILITY_CREATE_SESSION}: transport_preferences must not be empty; reason={REASON_INVALID_ARGUMENT}"
        );
    }
    Ok(parsed)
}

pub(crate) fn parse_video_constraints(
    args: &Value,
) -> anyhow::Result<RemoteDesktopVideoConstraints> {
    let raw = args
        .get("video")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let max_width = parse_video_u32(&raw, "max_width", 1920, 1, MAX_VIDEO_DIMENSION)?;
    let max_height = parse_video_u32(&raw, "max_height", 1080, 1, MAX_VIDEO_DIMENSION)?;
    let max_fps = parse_video_u32(
        &raw,
        "max_fps",
        DEFAULT_TARGET_FPS,
        MIN_ATTACH_FPS as u64,
        MAX_ATTACH_FPS as u64,
    )?;
    let max_bitrate_kbps = parse_video_u32(
        &raw,
        "max_bitrate_kbps",
        DEFAULT_TARGET_BITRATE_KBPS,
        1,
        250_000,
    )?;
    let target_latency_ms = parse_video_u32(
        &raw,
        "target_latency_ms",
        DEFAULT_GLASS_TO_GLASS_LATENCY_MS,
        1,
        1000,
    )?;
    let max_frame_queue_depth = parse_video_u32(
        &raw,
        "max_frame_queue_depth",
        DEFAULT_FRAME_QUEUE_DEPTH,
        1,
        MAX_FRAME_QUEUE_DEPTH as u64,
    )?;
    let codec_preferences = raw
        .get("codec_preferences")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|codec| matches!(*codec, "h264" | "hevc" | "av1" | "vp9" | "vp8"))
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| vec!["h264".into(), "hevc".into(), "av1".into(), "vp9".into()]);
    let scale_mode = raw
        .get("scale_mode")
        .and_then(Value::as_str)
        .unwrap_or("native");
    let region = raw.get("region").and_then(Value::as_str).unwrap_or("");
    Ok(RemoteDesktopVideoConstraints {
        max_width,
        max_height,
        max_fps,
        max_bitrate_kbps,
        scale_mode: scale_mode.to_string(),
        region: region.to_string(),
        codec_preferences,
        target_latency_ms,
        hardware_acceleration_required: raw
            .get("hardware_acceleration_required")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        max_frame_queue_depth,
        drop_stale_frames: raw
            .get("drop_stale_frames")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

fn parse_video_u32(
    raw: &serde_json::Map<String, Value>,
    key: &'static str,
    default: u32,
    min: u64,
    max: u64,
) -> anyhow::Result<u32> {
    let value = raw
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or(default as u64);
    if value < min || value > max {
        anyhow::bail!(
            "{ABILITY_CREATE_SESSION}: video.{key} must be in {min}..={max}; reason={REASON_INVALID_ARGUMENT}"
        );
    }
    Ok(value as u32)
}

pub(crate) fn bitrate_kbps_from_video_constraints(video: &RemoteDesktopVideoConstraints) -> u32 {
    video.bitrate_kbps()
}

pub(crate) fn frame_queue_depth_from_video_constraints(
    video: &RemoteDesktopVideoConstraints,
) -> usize {
    video.frame_queue_depth()
}

pub(crate) fn parse_input_policy(args: &Value, mode: &str) -> RemoteDesktopInputPolicy {
    let raw = args
        .get("input_policy")
        .or_else(|| args.get("input"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let requested = raw.as_object();
    let interactive = mode == "interactive";
    let read_bool = |key: &str| {
        requested
            .and_then(|map| map.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    RemoteDesktopInputPolicy {
        keyboard_enabled: interactive && (read_bool("keyboard_enabled") || read_bool("keyboard")),
        pointer_enabled: interactive && (read_bool("pointer_enabled") || read_bool("pointer")),
        clipboard_enabled: interactive
            && (read_bool("clipboard_enabled") || read_bool("clipboard")),
        file_drop_enabled: interactive
            && (read_bool("file_drop_enabled") || read_bool("file_drop")),
    }
}

pub(in crate::daemon::resources::remote_desktop) fn parse_attach_capture_options(
    args: &Value,
    session: &RemoteDesktopSession,
) -> anyhow::Result<ScreenCaptureOptions> {
    let video = if args.get("video").is_some() {
        parse_video_constraints(args)?
    } else {
        session.video().clone()
    };
    let fps = args
        .get("fps")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| video.max_fps());
    if !(MIN_ATTACH_FPS as u64..=MAX_ATTACH_FPS as u64).contains(&fps) {
        anyhow::bail!(
            "{ABILITY_ATTACH_SESSION}: fps {fps} outside {MIN_ATTACH_FPS}..={MAX_ATTACH_FPS}; reason={REASON_INVALID_ARGUMENT}"
        );
    }
    let resolution = match args.get("resolution") {
        Some(Value::String(raw)) if raw.eq_ignore_ascii_case("native") => None,
        Some(Value::String(raw)) => parse_resolution_string(raw)?,
        Some(_) => anyhow::bail!(
            "{ABILITY_ATTACH_SESSION}: resolution must be a string; reason={REASON_INVALID_ARGUMENT}"
        ),
        None => {
            video.resolution()
        }
    };
    Ok(ScreenCaptureOptions {
        fps: fps as u32,
        resolution,
        region: None,
    })
}

pub(crate) fn capture_options_from_video_constraints(
    video: &RemoteDesktopVideoConstraints,
) -> anyhow::Result<ScreenCaptureOptions> {
    let fps = video.max_fps();
    if !(MIN_ATTACH_FPS as u64..=MAX_ATTACH_FPS as u64).contains(&fps) {
        anyhow::bail!(
            "{ABILITY_SET_DESCRIPTION}: fps {fps} outside {MIN_ATTACH_FPS}..={MAX_ATTACH_FPS}; reason={REASON_INVALID_ARGUMENT}"
        );
    }
    let resolution = video.resolution();
    Ok(ScreenCaptureOptions {
        fps: fps as u32,
        resolution,
        region: None,
    })
}

pub(crate) fn parse_attach_encoding(args: &Value) -> anyhow::Result<AttachEncoding> {
    match args.get("encoding").and_then(Value::as_str) {
        None | Some("") => Ok(AttachEncoding::JpegBinary),
        Some(ATTACH_ENCODING_ANNEXB_H264) => Ok(AttachEncoding::AnnexBH264),
        Some(ATTACH_ENCODING_JPEG_BINARY) | Some("jpeg") | Some("image/jpeg") => {
            Ok(AttachEncoding::JpegBinary)
        }
        Some(other) => anyhow::bail!(
            "{ABILITY_ATTACH_SESSION}: unsupported encoding {other:?}; expected {ATTACH_ENCODING_ANNEXB_H264} or {ATTACH_ENCODING_JPEG_BINARY}; reason={REASON_INVALID_ARGUMENT}"
        ),
    }
}

fn parse_resolution_string(raw: &str) -> anyhow::Result<Option<VideoResolution>> {
    let raw = raw.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("native") {
        return Ok(None);
    }
    let lowered = raw.to_ascii_lowercase();
    match lowered.as_str() {
        "480p" => Ok(Some(VideoResolution {
            width: 854,
            height: 480,
        })),
        "720p" => Ok(Some(VideoResolution {
            width: 1280,
            height: 720,
        })),
        "1080p" => Ok(Some(VideoResolution {
            width: 1920,
            height: 1080,
        })),
        _ => {
            let (w, h) = lowered.split_once('x').ok_or_else(|| {
                anyhow::anyhow!(
                    "{ABILITY_ATTACH_SESSION}: resolution must be native, 480p, 720p, 1080p, or <width>x<height>; reason={REASON_INVALID_ARGUMENT}"
                )
            })?;
            let width = w.parse::<u32>().map_err(|_| {
                anyhow::anyhow!(
                    "{ABILITY_ATTACH_SESSION}: invalid resolution width; reason={REASON_INVALID_ARGUMENT}"
                )
            })?;
            let height = h.parse::<u32>().map_err(|_| {
                anyhow::anyhow!(
                    "{ABILITY_ATTACH_SESSION}: invalid resolution height; reason={REASON_INVALID_ARGUMENT}"
                )
            })?;
            if width == 0 || height == 0 {
                anyhow::bail!(
                    "{ABILITY_ATTACH_SESSION}: resolution dimensions must be positive; reason={REASON_INVALID_ARGUMENT}"
                );
            }
            Ok(Some(VideoResolution { width, height }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_constraints_reject_frame_queue_depth_above_manifest_limit() {
        let err = parse_video_constraints(&json!({
            "video": {
                "max_frame_queue_depth": MAX_FRAME_QUEUE_DEPTH + 1
            }
        }))
        .expect_err("runtime parser must enforce plugin manifest max_frame_queue");

        let message = err.to_string();
        assert!(
            message.contains("video.max_frame_queue_depth"),
            "error must name the drifted field; got: {message}"
        );
        assert!(
            message.contains(REASON_INVALID_ARGUMENT),
            "error must carry invalid_argument reason; got: {message}"
        );
    }

    #[test]
    fn frame_queue_depth_projection_uses_manifest_limit() {
        let video = parse_video_constraints(&json!({
            "video": { "max_frame_queue_depth": MAX_FRAME_QUEUE_DEPTH }
        }))
        .expect("video constraints");
        assert_eq!(
            frame_queue_depth_from_video_constraints(&video),
            MAX_FRAME_QUEUE_DEPTH as usize
        );
    }
}
