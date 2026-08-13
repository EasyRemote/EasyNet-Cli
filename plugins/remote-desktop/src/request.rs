// EasyNet CLI — remote desktop request parsing
// ============================================
//
// File: plugins/remote-desktop/src/request.rs
// Description: Input argument parsing for remote desktop abilities.

use rand::RngCore as _;
use serde_json::{json, Map, Value};

use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
    ScreenCaptureOptions, VideoResolution,
};
use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_ATTACH_SESSION, ABILITY_CREATE_SESSION, ABILITY_SET_DESCRIPTION,
    ATTACH_ENCODING_ANNEXB_H264, ATTACH_ENCODING_JPEG_BINARY, DEFAULT_FRAME_QUEUE_DEPTH,
    DEFAULT_GLASS_TO_GLASS_LATENCY_MS, DEFAULT_LEASE_TTL_MS, DEFAULT_TARGET_BITRATE_KBPS,
    DEFAULT_TARGET_FPS, MAX_ATTACH_FPS, MAX_FRAME_QUEUE_DEPTH, MAX_LEASE_TTL_MS,
    MAX_VIDEO_DIMENSION, MIN_ATTACH_FPS, NATIVE_MAX_BITRATE_KBPS, NATIVE_MIN_BITRATE_KBPS,
    REASON_INVALID_ARGUMENT, TRANSPORT_INVOKE_BIDI, TRANSPORT_PREVIEW_STREAM, TRANSPORT_WEBRTC,
};
use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
use crate::daemon::plugins::remote_desktop::target::{InputScope, RemoteAppTargetBinding};

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
    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
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
    pub(in crate::daemon::plugins::remote_desktop) fn constrained_for_binding(
        mut self,
        binding: &RemoteAppTargetBinding,
    ) -> Self {
        self.clipboard_enabled = false;
        self.file_drop_enabled = false;
        match binding.input_scope() {
            InputScope::ViewOnly => {
                self.keyboard_enabled = false;
                self.pointer_enabled = false;
            }
            InputScope::TargetLocal => {
                // Target-local pointer input may become safe once platform focus
                // and hit-test validation are implemented. Keyboard, clipboard,
                // and file-drop are never target-local on macOS today.
                self.keyboard_enabled = false;
            }
            InputScope::DisplayGlobal => {}
        }
        self
    }

    pub(in crate::daemon::plugins::remote_desktop) fn to_value(&self) -> Value {
        json!({
            "keyboard_enabled": self.keyboard_enabled,
            "pointer_enabled": self.pointer_enabled,
            "clipboard_enabled": self.clipboard_enabled,
            "file_drop_enabled": self.file_drop_enabled,
            "unsupported_input_types": ["clipboard", "file_drop"],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttachEncoding {
    AnnexBH264,
    JpegBinary,
}

pub(in crate::daemon::plugins::remote_desktop) fn require_str<'a>(
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

pub(in crate::daemon::plugins::remote_desktop) fn validate_session_id(
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

pub(in crate::daemon::plugins::remote_desktop) fn mint_session_id() -> String {
    let mut bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("rdp-{}", hex::encode(bytes))
}

pub(in crate::daemon::plugins::remote_desktop) fn mint_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn parse_mode(args: &Value) -> anyhow::Result<String> {
    let mode = match args.get("mode") {
        Some(value) => non_empty_string_field(value, "mode", ABILITY_CREATE_SESSION)?,
        None => "view_only",
    };
    match mode {
        "view_only" | "interactive" => Ok(mode.to_string()),
        _ => anyhow::bail!(
            "{ABILITY_CREATE_SESSION}: mode must be view_only or interactive; reason={REASON_INVALID_ARGUMENT}"
        ),
    }
}

pub(crate) fn parse_lease_ttl_ms(args: &Value) -> anyhow::Result<u64> {
    let ttl = match optional_u64_field(args, "lease_ttl_ms", "remote_desktop")? {
        Some(ttl) => ttl,
        None => optional_u64_field(args, "requested_ttl_seconds", "remote_desktop")?
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
    let raw = optional_object_field(args, "video", ABILITY_CREATE_SESSION)?;
    let empty = Map::new();
    let raw = raw.unwrap_or(&empty);
    validate_video_keys(raw)?;
    let max_width = parse_video_u32(raw, "max_width", 1920, 1, MAX_VIDEO_DIMENSION)?;
    let max_height = parse_video_u32(raw, "max_height", 1080, 1, MAX_VIDEO_DIMENSION)?;
    let max_fps = parse_video_u32(
        raw,
        "max_fps",
        DEFAULT_TARGET_FPS,
        MIN_ATTACH_FPS as u64,
        MAX_ATTACH_FPS as u64,
    )?;
    let max_bitrate_kbps = parse_video_u32(
        raw,
        "max_bitrate_kbps",
        DEFAULT_TARGET_BITRATE_KBPS,
        1,
        250_000,
    )?;
    let target_latency_ms = parse_video_u32(
        raw,
        "target_latency_ms",
        DEFAULT_GLASS_TO_GLASS_LATENCY_MS,
        1,
        1000,
    )?;
    let max_frame_queue_depth = parse_video_u32(
        raw,
        "max_frame_queue_depth",
        DEFAULT_FRAME_QUEUE_DEPTH,
        1,
        MAX_FRAME_QUEUE_DEPTH as u64,
    )?;
    let codec_preferences = parse_codec_preferences(raw)?;
    let scale_mode = optional_string_field(raw, "scale_mode", ABILITY_CREATE_SESSION)?
        .unwrap_or("native")
        .to_string();
    let region = optional_string_field(raw, "region", ABILITY_CREATE_SESSION)?
        .unwrap_or("")
        .to_string();
    Ok(RemoteDesktopVideoConstraints {
        max_width,
        max_height,
        max_fps,
        max_bitrate_kbps,
        scale_mode,
        region,
        codec_preferences,
        target_latency_ms,
        hardware_acceleration_required: optional_bool_field(
            raw,
            "hardware_acceleration_required",
            ABILITY_CREATE_SESSION,
        )?
        .unwrap_or(true),
        max_frame_queue_depth,
        drop_stale_frames: optional_bool_field(raw, "drop_stale_frames", ABILITY_CREATE_SESSION)?
            .unwrap_or(true),
    })
}

fn parse_video_u32(
    raw: &Map<String, Value>,
    key: &'static str,
    default: u32,
    min: u64,
    max: u64,
) -> anyhow::Result<u32> {
    let value = match raw.get(key) {
        Some(value) => value.as_u64().ok_or_else(|| {
            anyhow::anyhow!(
                "{ABILITY_CREATE_SESSION}: video.{key} must be an integer; reason={REASON_INVALID_ARGUMENT}"
            )
        })?,
        None => default as u64,
    };
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

pub(crate) fn parse_input_policy(
    args: &Value,
    mode: &str,
) -> anyhow::Result<RemoteDesktopInputPolicy> {
    let requested = optional_input_policy(args)?;
    let empty = Map::new();
    let requested = requested.unwrap_or(&empty);
    validate_input_policy_keys(requested)?;
    let interactive = mode == "interactive";
    let read_bool = |key: &'static str| -> anyhow::Result<bool> {
        Ok(optional_bool_field(requested, key, ABILITY_CREATE_SESSION)?.unwrap_or(false))
    };
    let keyboard_enabled = read_bool("keyboard_enabled")? || read_bool("keyboard")?;
    let pointer_enabled = read_bool("pointer_enabled")? || read_bool("pointer")?;
    let _clipboard_enabled = read_bool("clipboard_enabled")?;
    let _clipboard = read_bool("clipboard")?;
    let _file_drop_enabled = read_bool("file_drop_enabled")?;
    let _file_drop = read_bool("file_drop")?;
    Ok(RemoteDesktopInputPolicy {
        keyboard_enabled: interactive && keyboard_enabled,
        pointer_enabled: interactive && pointer_enabled,
        clipboard_enabled: false,
        file_drop_enabled: false,
    })
}

pub(crate) fn parse_optional_session_id(args: &Value) -> anyhow::Result<Option<String>> {
    let Some(value) = args.get("session_id") else {
        return Ok(None);
    };
    let session_id = non_empty_string_field(value, "session_id", ABILITY_CREATE_SESSION)?;
    validate_session_id(session_id)?;
    Ok(Some(session_id.to_string()))
}

fn optional_object_field<'a>(
    args: &'a Value,
    key: &'static str,
    ability: &str,
) -> anyhow::Result<Option<&'a Map<String, Value>>> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value.as_object().map(Some).ok_or_else(|| {
        anyhow::anyhow!("{ability}: `{key}` must be an object; reason={REASON_INVALID_ARGUMENT}")
    })
}

fn optional_input_policy(args: &Value) -> anyhow::Result<Option<&Map<String, Value>>> {
    let input_policy = optional_object_field(args, "input_policy", ABILITY_CREATE_SESSION)?;
    let input = optional_object_field(args, "input", ABILITY_CREATE_SESSION)?;
    match (input_policy, input) {
        (Some(_), Some(_)) => anyhow::bail!(
            "{ABILITY_CREATE_SESSION}: input_policy and input are mutually exclusive; reason={REASON_INVALID_ARGUMENT}"
        ),
        (Some(input_policy), None) => Ok(Some(input_policy)),
        (None, Some(input)) => Ok(Some(input)),
        (None, None) => Ok(None),
    }
}

fn optional_u64_field(
    args: &Value,
    key: &'static str,
    ability: &str,
) -> anyhow::Result<Option<u64>> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value.as_u64().map(Some).ok_or_else(|| {
        anyhow::anyhow!("{ability}: `{key}` must be an integer; reason={REASON_INVALID_ARGUMENT}")
    })
}

fn optional_string_field<'a>(
    raw: &'a Map<String, Value>,
    key: &'static str,
    ability: &str,
) -> anyhow::Result<Option<&'a str>> {
    let Some(value) = raw.get(key) else {
        return Ok(None);
    };
    value.as_str().map(Some).ok_or_else(|| {
        anyhow::anyhow!("{ability}: `{key}` must be a string; reason={REASON_INVALID_ARGUMENT}")
    })
}

fn non_empty_string_field<'a>(
    value: &'a Value,
    key: &'static str,
    ability: &str,
) -> anyhow::Result<&'a str> {
    let raw = value.as_str().ok_or_else(|| {
        anyhow::anyhow!("{ability}: `{key}` must be a string; reason={REASON_INVALID_ARGUMENT}")
    })?;
    if raw.trim().is_empty() {
        anyhow::bail!(
            "{ability}: `{key}` must be a non-empty string; reason={REASON_INVALID_ARGUMENT}"
        );
    }
    Ok(raw)
}

fn optional_bool_field(
    raw: &Map<String, Value>,
    key: &'static str,
    ability: &str,
) -> anyhow::Result<Option<bool>> {
    let Some(value) = raw.get(key) else {
        return Ok(None);
    };
    value.as_bool().map(Some).ok_or_else(|| {
        anyhow::anyhow!("{ability}: `{key}` must be a boolean; reason={REASON_INVALID_ARGUMENT}")
    })
}

fn validate_video_keys(raw: &Map<String, Value>) -> anyhow::Result<()> {
    const ALLOWED: &[&str] = &[
        "max_width",
        "max_height",
        "max_fps",
        "max_bitrate_kbps",
        "scale_mode",
        "region",
        "codec_preferences",
        "target_latency_ms",
        "hardware_acceleration_required",
        "max_frame_queue_depth",
        "drop_stale_frames",
    ];
    validate_keys(raw, "video", ALLOWED)
}

fn validate_input_policy_keys(raw: &Map<String, Value>) -> anyhow::Result<()> {
    const ALLOWED: &[&str] = &[
        "keyboard_enabled",
        "keyboard",
        "pointer_enabled",
        "pointer",
        "clipboard_enabled",
        "clipboard",
        "file_drop_enabled",
        "file_drop",
    ];
    validate_keys(raw, "input_policy", ALLOWED)
}

fn validate_keys(raw: &Map<String, Value>, object: &str, allowed: &[&str]) -> anyhow::Result<()> {
    if let Some(key) = raw.keys().find(|key| !allowed.contains(&key.as_str())) {
        anyhow::bail!(
            "{ABILITY_CREATE_SESSION}: {object}.{key} is not supported; reason={REASON_INVALID_ARGUMENT}"
        );
    }
    Ok(())
}

fn parse_codec_preferences(raw: &Map<String, Value>) -> anyhow::Result<Vec<String>> {
    let Some(value) = raw.get("codec_preferences") else {
        return Ok(vec![
            "h264".into(),
            "hevc".into(),
            "av1".into(),
            "vp9".into(),
        ]);
    };
    let items = value.as_array().ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_CREATE_SESSION}: video.codec_preferences must be an array; reason={REASON_INVALID_ARGUMENT}"
        )
    })?;
    if items.is_empty() {
        anyhow::bail!(
            "{ABILITY_CREATE_SESSION}: video.codec_preferences must not be empty; reason={REASON_INVALID_ARGUMENT}"
        );
    }
    let mut parsed = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let codec = item.as_str().ok_or_else(|| {
            anyhow::anyhow!(
                "{ABILITY_CREATE_SESSION}: video.codec_preferences[{index}] must be a string; reason={REASON_INVALID_ARGUMENT}"
            )
        })?;
        match codec {
            "h264" | "hevc" | "av1" | "vp9" | "vp8" => {
                if !parsed.iter().any(|existing| existing == codec) {
                    parsed.push(codec.to_string());
                }
            }
            _ => anyhow::bail!(
                "{ABILITY_CREATE_SESSION}: unsupported video codec {codec:?}; reason={REASON_INVALID_ARGUMENT}"
            ),
        }
    }
    Ok(parsed)
}

pub(in crate::daemon::plugins::remote_desktop) fn parse_attach_capture_options(
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

    #[test]
    fn video_constraints_reject_present_malformed_fields() {
        for (args, expected) in [
            (json!({"video": "native"}), "`video` must be an object"),
            (
                json!({"video": {"max_width": "1920"}}),
                "video.max_width must be an integer",
            ),
            (
                json!({"video": {"codec_preferences": ["h264", 7]}}),
                "video.codec_preferences[1] must be a string",
            ),
            (
                json!({"video": {"codec_preferences": ["h264", "cinepak"]}}),
                "unsupported video codec",
            ),
            (
                json!({"video": {"hardware_acceleration_required": "true"}}),
                "`hardware_acceleration_required` must be a boolean",
            ),
            (
                json!({"video": {"legacy_quality": "auto"}}),
                "video.legacy_quality is not supported",
            ),
        ] {
            let err = parse_video_constraints(&args)
                .expect_err("present malformed video schema must fail closed")
                .to_string();
            assert!(err.contains(expected), "expected {expected:?}; got {err}");
            assert!(
                err.contains(REASON_INVALID_ARGUMENT),
                "error must carry invalid_argument reason; got: {err}"
            );
        }
    }

    #[test]
    fn create_session_scalar_defaults_apply_only_when_absent() {
        assert_eq!(parse_mode(&json!({})).unwrap(), "view_only");
        assert_eq!(
            parse_lease_ttl_ms(&json!({})).unwrap(),
            DEFAULT_LEASE_TTL_MS
        );
        assert!(parse_optional_session_id(&json!({})).unwrap().is_none());

        for (args, expected) in [
            (json!({"mode": 42}), "`mode` must be a string"),
            (
                json!({"lease_ttl_ms": "30000"}),
                "`lease_ttl_ms` must be an integer",
            ),
            (
                json!({"requested_ttl_seconds": "30"}),
                "`requested_ttl_seconds` must be an integer",
            ),
            (
                json!({"session_id": ""}),
                "`session_id` must be a non-empty string",
            ),
            (
                json!({"session_id": ["rdp"]}),
                "`session_id` must be a string",
            ),
        ] {
            let err = match expected {
                e if e.contains("mode") => parse_mode(&args).unwrap_err().to_string(),
                e if e.contains("ttl") => parse_lease_ttl_ms(&args).unwrap_err().to_string(),
                _ => parse_optional_session_id(&args).unwrap_err().to_string(),
            };
            assert!(err.contains(expected), "expected {expected:?}; got {err}");
            assert!(
                err.contains(REASON_INVALID_ARGUMENT),
                "error must carry invalid_argument reason; got: {err}"
            );
        }
    }

    #[test]
    fn input_policy_rejects_malformed_policy_scope() {
        for (args, expected) in [
            (
                json!({"input_policy": "interactive"}),
                "`input_policy` must be an object",
            ),
            (
                json!({"input": {}, "input_policy": {}}),
                "input_policy and input are mutually exclusive",
            ),
            (
                json!({"input_policy": {"keyboard_enabled": "true"}}),
                "`keyboard_enabled` must be a boolean",
            ),
            (
                json!({"input_policy": {"clipboard_enabled": "true"}}),
                "`clipboard_enabled` must be a boolean",
            ),
            (
                json!({"input_policy": {"file_drop_enabled": "true"}}),
                "`file_drop_enabled` must be a boolean",
            ),
            (
                json!({"input_policy": {"legacy_pointer": true}}),
                "input_policy.legacy_pointer is not supported",
            ),
        ] {
            let err = parse_input_policy(&args, "interactive")
                .expect_err("present malformed input policy must fail closed")
                .to_string();
            assert!(err.contains(expected), "expected {expected:?}; got {err}");
            assert!(
                err.contains(REASON_INVALID_ARGUMENT),
                "error must carry invalid_argument reason; got: {err}"
            );
        }
    }

    #[test]
    fn input_policy_reports_clipboard_and_file_drop_unsupported_even_when_requested() {
        let policy = parse_input_policy(
            &json!({
                "input_policy": {
                    "keyboard_enabled": true,
                    "pointer_enabled": true,
                    "clipboard_enabled": true,
                    "file_drop_enabled": true
                }
            }),
            "interactive",
        )
        .expect("well-formed interactive input policy parses");
        let value = policy.to_value();

        assert_eq!(value["keyboard_enabled"], json!(true));
        assert_eq!(value["pointer_enabled"], json!(true));
        assert_eq!(value["clipboard_enabled"], json!(false));
        assert_eq!(value["file_drop_enabled"], json!(false));
        assert_eq!(
            value["unsupported_input_types"],
            json!(["clipboard", "file_drop"])
        );
    }
}
