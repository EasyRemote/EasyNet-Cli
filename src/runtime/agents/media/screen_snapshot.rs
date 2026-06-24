// EasyNet CLI — screen.snapshot real handler (RFC-005 v3.2 A8)
// ==============================================================
//
// File: src/runtime/agents/media/screen_snapshot.rs
//
// Resource-scoped envelope-aware handler for `screen.snapshot`.
// Its flow mirrors `camera_snapshot`:
//
//   1. envelope.subject required (INV-SUBJECT-ENVELOPE)
//   2. resolve subject → ResourceEntry, reject when type is not
//      one of {display, application, window} per RFC-005 v3.2
//      §A4 (the screen-target type taxonomy is shared between
//      subscribe and snapshot)
//   3. capture one RGBA frame via the configured backend, encode
//      to JPEG, base64 the bytes if under MAX_INLINE_BYTES
//   4. return { image_bytes_b64, content_type, width, height,
//               byte_size, captured_at, hardware_id }
//
// The backend trait keeps stub / synthetic / real switchable per
// the AXON-RFC-005-device-backend-selection note. The xcap backend
// resolves display resources by monitor id/index metadata when
// present, falls back to the primary monitor, and captures
// window/application resources through xcap window selection.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::persistence::resources::{ResourceEntry, ResourceType};
use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::ability_dispatch::{AxonAbilityCatalog, EnvelopeContext, StreamSource};
use crate::runtime::agents::media::resource_subject::{
    self, resolve_required_resource_subject, ResourceSubjectSpec,
};
use crate::runtime::agents::media_abilities::{ABILITY_SCREEN_SNAPSHOT, ABILITY_SCREEN_SUBSCRIBE};

/// 2 MiB inline cap — same shape as camera.snapshot. This keeps
/// base64-expanded receipts below Axon's 4 MiB IPC frame limit while
/// allowing ordinary laptop screenshots through. 4K/full-desktop
/// streaming still belongs on the BIDI/payloadstore path.
const MAX_INLINE_BYTES: usize = 2 * 1024 * 1024;
const BROADCAST_CAPACITY: usize = 8;
const DEFAULT_SCREEN_FPS: u32 = 60;
const MIN_SCREEN_FPS: u32 = 1;
const MAX_SCREEN_FPS: u32 = 60;

pub const REASON_SUBJECT_REQUIRED: &str = resource_subject::REASON_SUBJECT_REQUIRED;
pub const REASON_SUBJECT_IN_ARGS: &str = resource_subject::REASON_SUBJECT_IN_ARGS;
pub const REASON_RESOURCE_NOT_FOUND: &str = resource_subject::REASON_RESOURCE_NOT_FOUND;
pub const REASON_RESOURCE_TYPE_MISMATCH: &str = resource_subject::REASON_RESOURCE_TYPE_MISMATCH;
pub const REASON_RESOURCE_UNAVAILABLE: &str = "resource_unavailable";
pub const REASON_IMAGE_TOO_LARGE: &str = "image_too_large_for_inline";
pub const REASON_INVALID_ARGUMENT: &str = "invalid_argument";

#[derive(Debug, Clone)]
pub struct EncodedFrame {
    pub jpeg_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct RawRgbFrame {
    pub rgb_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCaptureOptions {
    pub fps: u32,
    pub resolution: Option<VideoResolution>,
    pub region: Option<CaptureRegion>,
}

impl Default for ScreenCaptureOptions {
    fn default() -> Self {
        Self {
            fps: DEFAULT_SCREEN_FPS,
            resolution: None,
            region: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoResolution {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureRegion {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Backend seam. Same shape as camera.snapshot's
/// `SnapshotBackend`; an `XcapBackend` ships in this PR, a future
/// Wayland-specific or per-window backend can drop in without
/// touching the dispatch / receipt code below.
pub trait ScreenSnapshotBackend: Send + Sync {
    /// Capture one frame from the resource described by `entry`.
    /// Returns the encoded JPEG bytes plus the actual dimensions
    /// (which may differ from any requested resolution if the
    /// backend can't satisfy it exactly).
    fn capture_jpeg(
        &self,
        entry: &ResourceEntry,
        options: &ScreenCaptureOptions,
    ) -> anyhow::Result<EncodedFrame>;

    /// Open a live screen stream. Each frame is JSON shaped like
    /// `screen.snapshot`'s receipt plus a monotonically increasing
    /// `seq`. The broadcast depth is intentionally bounded so a
    /// slow HTTP/SSE client cannot turn desktop capture into an
    /// unbounded daemon buffer.
    fn open_stream(
        &self,
        entry: ResourceEntry,
        options: ScreenCaptureOptions,
    ) -> anyhow::Result<broadcast::Receiver<Value>>;
}

// ── XcapBackend (real) ───────────────────────────────────────

/// xcap-backed real backend. Display captures select by resource metadata
/// (`monitor_id` or `monitor_index`) when available and otherwise fall back
/// to the primary monitor. Window/application captures select an xcap window
/// by recorded id, pid, title, or application name.
#[derive(Debug, Default)]
pub struct XcapBackend;

impl ScreenSnapshotBackend for XcapBackend {
    fn capture_jpeg(
        &self,
        entry: &ResourceEntry,
        options: &ScreenCaptureOptions,
    ) -> anyhow::Result<EncodedFrame> {
        capture_screen_with_xcap(entry, options)
    }

    fn open_stream(
        &self,
        entry: ResourceEntry,
        options: ScreenCaptureOptions,
    ) -> anyhow::Result<broadcast::Receiver<Value>> {
        let (tx, rx) = broadcast::channel::<Value>(BROADCAST_CAPACITY);
        let seq = Arc::new(AtomicU64::new(0));
        std::thread::Builder::new()
            .name("easynet-screen-xcap".into())
            .spawn(move || {
                let interval = Duration::from_secs_f64(1.0 / options.fps as f64);
                let backend = XcapBackend;
                loop {
                    let started = Instant::now();
                    match backend.capture_jpeg(&entry, &options) {
                        Ok(frame) => {
                            let value = build_screen_frame(&seq, &entry.hardware_id, frame);
                            if tx.send(value).is_err() {
                                break;
                            }
                        }
                        Err(err) => {
                            crate::op_event!(
                                component = screen_capture,
                                kind = stream_capture_failed,
                                reason = err.to_string(),
                            );
                            let _ = tx.send(json!({
                                "type": "error",
                                "message": err.to_string(),
                                "reason": REASON_RESOURCE_UNAVAILABLE,
                            }));
                            break;
                        }
                    }
                    if let Some(remaining) = interval.checked_sub(started.elapsed()) {
                        std::thread::sleep(remaining);
                    }
                }
            })
            .map_err(|e| {
                anyhow::anyhow!(
                    "{ABILITY_SCREEN_SUBSCRIBE}: failed to spawn screen worker: {e}; \
                     reason={REASON_RESOURCE_UNAVAILABLE}"
                )
            })?;
        Ok(rx)
    }
}

fn capture_screen_with_xcap(
    entry: &ResourceEntry,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<EncodedFrame> {
    let raw = capture_rgb_with_xcap(entry, options)?;
    let jpeg = encode_jpeg_checked(&raw.rgb_bytes, raw.width, raw.height)?;
    Ok(EncodedFrame {
        jpeg_bytes: jpeg,
        width: raw.width,
        height: raw.height,
    })
}

pub fn capture_rgb_with_xcap(
    entry: &ResourceEntry,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<RawRgbFrame> {
    match entry.kind {
        ResourceType::Display => capture_display_rgb_with_xcap(entry, options),
        ResourceType::Application | ResourceType::Window => {
            capture_window_rgb_with_xcap(entry, options)
        }
        _ => anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: subject type {} is not capturable as screen media; \
             reason={REASON_RESOURCE_TYPE_MISMATCH}",
            entry.kind.as_str()
        ),
    }
}

fn capture_display_rgb_with_xcap(
    entry: &ResourceEntry,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<RawRgbFrame> {
    let monitors = xcap::Monitor::all().map_err(|e| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: xcap Monitor::all failed: {e}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let monitor = select_monitor(monitors, entry)?;
    let rgba = monitor.capture_image().map_err(|e| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: xcap capture_image failed (likely \
             macOS Screen Recording permission not granted): {e}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    rgba_image_to_rgb_frame(rgba, options)
}

fn capture_window_rgb_with_xcap(
    entry: &ResourceEntry,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<RawRgbFrame> {
    let window = select_window(entry)?;
    let rgba = window.capture_image().map_err(|e| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: xcap window capture_image failed: {e}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    rgba_image_to_rgb_frame(rgba, options)
}

fn rgba_image_to_rgb_frame(
    rgba: xcap::image::RgbaImage,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<RawRgbFrame> {
    let width = rgba.width();
    let height = rgba.height();
    rgba_bytes_to_rgb_frame(rgba.into_raw(), width, height, options)
}

pub fn rgba_bytes_to_rgb_frame(
    rgba: Vec<u8>,
    source_width: u32,
    source_height: u32,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<RawRgbFrame> {
    let region = options.region.unwrap_or(CaptureRegion {
        x: 0,
        y: 0,
        w: source_width,
        h: source_height,
    });
    validate_region(region, source_width, source_height)?;
    let mut rgb = rgba_region_to_rgb(rgba, source_width, region);
    let mut width = region.w;
    let mut height = region.h;
    if let Some(target) = options.resolution {
        rgb = resize_rgb_nearest(&rgb, width, height, target.width, target.height);
        width = target.width;
        height = target.height;
    }
    Ok(RawRgbFrame {
        rgb_bytes: rgb,
        width,
        height,
    })
}

fn select_monitor(
    monitors: Vec<xcap::Monitor>,
    entry: &ResourceEntry,
) -> anyhow::Result<xcap::Monitor> {
    let expected_id = entry.metadata.get("monitor_id").and_then(|v| v.as_u64());
    let expected_index = entry.metadata.get("monitor_index").and_then(|v| v.as_u64());
    let mut fallback_primary = None;
    for (idx, monitor) in monitors.into_iter().enumerate() {
        if expected_id.is_some_and(|expected| {
            monitor
                .id()
                .ok()
                .map(|actual| actual as u64 == expected)
                .unwrap_or(false)
        }) {
            return Ok(monitor);
        }
        if expected_index.is_some_and(|expected| idx as u64 == expected) {
            return Ok(monitor);
        }
        if fallback_primary.is_none() && monitor.is_primary().unwrap_or(false) {
            fallback_primary = Some(monitor);
        }
    }
    fallback_primary.ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: no matching or primary monitor reported by \
             xcap; reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })
}

pub fn open_display_recorder_with_xcap(
    entry: &ResourceEntry,
) -> anyhow::Result<(xcap::VideoRecorder, Receiver<xcap::Frame>)> {
    if entry.kind != ResourceType::Display {
        anyhow::bail!(
            "{ABILITY_SCREEN_SUBSCRIBE}: xcap video recorder only supports display resources; \
             subject type is {}; reason={REASON_RESOURCE_TYPE_MISMATCH}",
            entry.kind.as_str()
        );
    }
    let monitors = xcap::Monitor::all().map_err(|e| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SUBSCRIBE}: xcap Monitor::all failed: {e}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let monitor = select_monitor(monitors, entry)?;
    monitor.video_recorder().map_err(|e| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SUBSCRIBE}: xcap video_recorder failed: {e}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })
}

fn select_window(entry: &ResourceEntry) -> anyhow::Result<xcap::Window> {
    let windows = xcap::Window::all().map_err(|e| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: xcap Window::all failed: {e}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    match entry.kind {
        ResourceType::Window => select_window_by_id_or_name(windows, entry),
        ResourceType::Application => select_application_window(windows, entry),
        _ => unreachable!("select_window called for non-window resource"),
    }
}

fn select_window_by_id_or_name(
    windows: Vec<xcap::Window>,
    entry: &ResourceEntry,
) -> anyhow::Result<xcap::Window> {
    let expected_id = entry.metadata.get("window_id").and_then(Value::as_u64);
    let expected_pid = entry.metadata.get("pid").and_then(Value::as_u64);
    let expected_title = entry.metadata.get("title").and_then(Value::as_str);
    let expected_app = entry.metadata.get("app_name").and_then(Value::as_str);
    windows
        .into_iter()
        .find(|window| {
            let id_matches = expected_id.is_some_and(|id| {
                window
                    .id()
                    .ok()
                    .map(|actual| actual as u64 == id)
                    .unwrap_or(false)
            });
            if id_matches {
                return true;
            }
            let pid_matches = expected_pid.is_some_and(|pid| {
                window
                    .pid()
                    .ok()
                    .map(|actual| actual as u64 == pid)
                    .unwrap_or(false)
            });
            let app_matches = expected_app.is_some_and(|app| {
                window
                    .app_name()
                    .ok()
                    .map(|actual| actual == app)
                    .unwrap_or(false)
            });
            let title_matches = expected_title.is_some_and(|title| {
                window
                    .title()
                    .ok()
                    .map(|actual| actual == title)
                    .unwrap_or(false)
            });
            pid_matches && app_matches && title_matches
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: requested window is no longer available; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })
}

fn select_application_window(
    windows: Vec<xcap::Window>,
    entry: &ResourceEntry,
) -> anyhow::Result<xcap::Window> {
    let expected_app = entry
        .metadata
        .get("app_name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: application resource missing app_name metadata; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
    let mut candidates = windows
        .into_iter()
        .filter(|window| {
            window
                .app_name()
                .ok()
                .map(|app| app == expected_app)
                .unwrap_or(false)
                && window.width().ok().unwrap_or(0) >= 160
                && window.height().ok().unwrap_or(0) >= 120
                && window.is_minimized().ok() != Some(true)
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: application {expected_app:?} has no capturable windows; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }
    if let Some(index) = candidates
        .iter()
        .position(|window| window.is_focused().ok() == Some(true))
    {
        return Ok(candidates.swap_remove(index));
    }
    Ok(candidates.remove(0))
}

fn validate_region(region: CaptureRegion, width: u32, height: u32) -> anyhow::Result<()> {
    let x2 = region.x.checked_add(region.w).ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: region overflows; reason={REASON_INVALID_ARGUMENT}"
        )
    })?;
    let y2 = region.y.checked_add(region.h).ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: region overflows; reason={REASON_INVALID_ARGUMENT}"
        )
    })?;
    if region.w == 0 || region.h == 0 || x2 > width || y2 > height {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: region {:?} exceeds source bounds {}x{}; \
             reason={REASON_INVALID_ARGUMENT}",
            region,
            width,
            height
        );
    }
    Ok(())
}

fn rgba_region_to_rgb(rgba: Vec<u8>, source_width: u32, region: CaptureRegion) -> Vec<u8> {
    let mut rgb = Vec::with_capacity((region.w * region.h * 3) as usize);
    for y in region.y..(region.y + region.h) {
        let row_start = ((y * source_width + region.x) * 4) as usize;
        for x in 0..region.w {
            let i = row_start + (x * 4) as usize;
            rgb.push(rgba[i]);
            rgb.push(rgba[i + 1]);
            rgb.push(rgba[i + 2]);
        }
    }
    rgb
}

fn resize_rgb_nearest(
    src: &[u8],
    src_width: u32,
    src_height: u32,
    dst_width: u32,
    dst_height: u32,
) -> Vec<u8> {
    if src_width == dst_width && src_height == dst_height {
        return src.to_vec();
    }
    let mut out = vec![0u8; (dst_width * dst_height * 3) as usize];
    for y in 0..dst_height {
        let src_y = ((y as u64 * src_height as u64) / dst_height as u64) as u32;
        for x in 0..dst_width {
            let src_x = ((x as u64 * src_width as u64) / dst_width as u64) as u32;
            let src_i = ((src_y * src_width + src_x) * 3) as usize;
            let dst_i = ((y * dst_width + x) * 3) as usize;
            out[dst_i..dst_i + 3].copy_from_slice(&src[src_i..src_i + 3]);
        }
    }
    out
}

fn encode_jpeg_checked(rgb: &[u8], width: u32, height: u32) -> anyhow::Result<Vec<u8>> {
    let width = u16::try_from(width).map_err(|_| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: width exceeds JPEG encoder limit; \
             reason={REASON_INVALID_ARGUMENT}"
        )
    })?;
    let height = u16::try_from(height).map_err(|_| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: height exceeds JPEG encoder limit; \
             reason={REASON_INVALID_ARGUMENT}"
        )
    })?;
    encode_jpeg(rgb, width, height)
}

fn build_screen_frame(seq: &AtomicU64, hardware_id: &str, frame: EncodedFrame) -> Value {
    let n = seq.fetch_add(1, Ordering::Relaxed);
    let image_bytes_b64 = BASE64_STANDARD.encode(&frame.jpeg_bytes);
    json!({
        "seq":             n,
        "content_type":    "image/jpeg",
        "width":           frame.width,
        "height":          frame.height,
        "byte_size":       frame.jpeg_bytes.len(),
        "captured_at":     chrono::Utc::now().to_rfc3339(),
        "image_bytes_b64": image_bytes_b64,
        "hardware_id":     hardware_id,
    })
}

fn encode_jpeg(rgb: &[u8], width: u16, height: u16) -> anyhow::Result<Vec<u8>> {
    let mut jpeg = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut jpeg, 80);
    encoder
        .encode(rgb, width, height, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| anyhow::anyhow!("jpeg encode failed: {e}"))?;
    Ok(jpeg)
}

fn parse_capture_options(args: &Value, include_fps: bool) -> anyhow::Result<ScreenCaptureOptions> {
    let mut options = ScreenCaptureOptions::default();
    if let Value::Object(map) = args {
        if include_fps {
            if let Some(value) = map.get("fps") {
                let fps = value.as_u64().ok_or_else(|| {
                    anyhow::anyhow!(
                        "{ABILITY_SCREEN_SUBSCRIBE}: fps must be an integer; \
                         reason={REASON_INVALID_ARGUMENT}"
                    )
                })?;
                if !(MIN_SCREEN_FPS as u64..=MAX_SCREEN_FPS as u64).contains(&fps) {
                    anyhow::bail!(
                        "{ABILITY_SCREEN_SUBSCRIBE}: fps {fps} outside {MIN_SCREEN_FPS}..={MAX_SCREEN_FPS}; \
                         reason={REASON_INVALID_ARGUMENT}"
                    );
                }
                options.fps = fps as u32;
            }
        }
        if let Some(value) = map.get("resolution") {
            options.resolution = parse_resolution(value)?;
        }
        if let Some(value) = map.get("region") {
            options.region = Some(parse_region(value)?);
        }
    }
    Ok(options)
}

fn parse_resolution(value: &Value) -> anyhow::Result<Option<VideoResolution>> {
    let Some(raw) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
        anyhow::bail!(
            "{ABILITY_SCREEN_SUBSCRIBE}: resolution must be a string; \
             reason={REASON_INVALID_ARGUMENT}"
        );
    };
    if raw.eq_ignore_ascii_case("native") {
        return Ok(None);
    }
    let lowered = raw.to_ascii_lowercase();
    let resolution = match lowered.as_str() {
        "480p" => VideoResolution {
            width: 854,
            height: 480,
        },
        "720p" => VideoResolution {
            width: 1280,
            height: 720,
        },
        "1080p" => VideoResolution {
            width: 1920,
            height: 1080,
        },
        _ => {
            let Some((w, h)) = lowered.split_once('x') else {
                anyhow::bail!(
                    "{ABILITY_SCREEN_SUBSCRIBE}: resolution {raw:?} must be native, 480p, 720p, 1080p, or <width>x<height>; \
                     reason={REASON_INVALID_ARGUMENT}"
                );
            };
            VideoResolution {
                width: parse_positive_u32(w, "resolution width")?,
                height: parse_positive_u32(h, "resolution height")?,
            }
        }
    };
    Ok(Some(resolution))
}

fn parse_region(value: &Value) -> anyhow::Result<CaptureRegion> {
    let Value::Object(map) = value else {
        anyhow::bail!(
            "{ABILITY_SCREEN_SUBSCRIBE}: region must be an object; \
             reason={REASON_INVALID_ARGUMENT}"
        );
    };
    Ok(CaptureRegion {
        x: parse_u32_field(map.get("x"), "region.x")?,
        y: parse_u32_field(map.get("y"), "region.y")?,
        w: parse_u32_field(map.get("w"), "region.w")?,
        h: parse_u32_field(map.get("h"), "region.h")?,
    })
}

fn parse_u32_field(value: Option<&Value>, name: &str) -> anyhow::Result<u32> {
    let n = value.and_then(Value::as_u64).ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SUBSCRIBE}: {name} must be an integer; \
             reason={REASON_INVALID_ARGUMENT}"
        )
    })?;
    u32::try_from(n).map_err(|_| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SUBSCRIBE}: {name} too large; \
             reason={REASON_INVALID_ARGUMENT}"
        )
    })
}

fn parse_positive_u32(raw: &str, name: &str) -> anyhow::Result<u32> {
    let value = raw.parse::<u32>().map_err(|_| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SUBSCRIBE}: {name} must be an integer; \
             reason={REASON_INVALID_ARGUMENT}"
        )
    })?;
    if value == 0 {
        anyhow::bail!(
            "{ABILITY_SCREEN_SUBSCRIBE}: {name} must be positive; \
             reason={REASON_INVALID_ARGUMENT}"
        );
    }
    Ok(value)
}

fn ensure_region_allowed(
    entry: &ResourceEntry,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<()> {
    if options.region.is_some() && entry.kind != ResourceType::Display {
        anyhow::bail!(
            "{ABILITY_SCREEN_SUBSCRIBE}: region is only valid for display resources; \
             subject type is {}; reason={REASON_INVALID_ARGUMENT}",
            entry.kind.as_str()
        );
    }
    Ok(())
}

// ── SyntheticBackend (test-only) ─────────────────────────────

/// Same FNV-1a colour-from-hardware-id pattern camera.snapshot
/// uses. Lets tests round-trip the full handler without touching
/// the desktop, and lets two distinct screen resources produce
/// distinguishable bytes.
#[derive(Debug, Default)]
pub struct SyntheticScreenBackend;

impl ScreenSnapshotBackend for SyntheticScreenBackend {
    fn capture_jpeg(
        &self,
        entry: &ResourceEntry,
        options: &ScreenCaptureOptions,
    ) -> anyhow::Result<EncodedFrame> {
        const W: u32 = 64;
        const H: u32 = 48;
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in entry.hardware_id.bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let r = (hash & 0xff) as u8;
        let g = ((hash >> 8) & 0xff) as u8;
        let b = ((hash >> 16) & 0xff) as u8;
        let mut rgb = Vec::with_capacity((W * H * 3) as usize);
        for _ in 0..(W * H) {
            rgb.push(r);
            rgb.push(g);
            rgb.push(b);
        }
        let (width, height, rgb) = if let Some(target) = options.resolution {
            (
                target.width,
                target.height,
                resize_rgb_nearest(&rgb, W, H, target.width, target.height),
            )
        } else {
            (W, H, rgb)
        };
        let jpeg = encode_jpeg_checked(&rgb, width, height)?;
        Ok(EncodedFrame {
            jpeg_bytes: jpeg,
            width,
            height,
        })
    }

    fn open_stream(
        &self,
        entry: ResourceEntry,
        options: ScreenCaptureOptions,
    ) -> anyhow::Result<broadcast::Receiver<Value>> {
        let (tx, rx) = broadcast::channel::<Value>(8);
        let seq = Arc::new(AtomicU64::new(0));
        let frame = self.capture_jpeg(&entry, &options)?;
        let _ = tx.send(build_screen_frame(&seq, &entry.hardware_id, frame));
        Ok(rx)
    }
}

// ── Registration ─────────────────────────────────────────────

pub fn register_with_backend(
    reg: &mut AxonAbilityCatalog,
    backend: Arc<dyn ScreenSnapshotBackend>,
) {
    let snapshot_backend = Arc::clone(&backend);
    reg.register_rpc_with_envelope_and_owner(
        "screen.snapshot",
        OwnerKind::Device,
        Arc::new(move |env: EnvelopeContext, args: Value| {
            snapshot_handler(&snapshot_backend, env, args)
        }),
    );
    reg.register_stream_with_envelope_and_owner(
        "screen.subscribe",
        OwnerKind::Device,
        Arc::new(move |env: EnvelopeContext, args: Value| subscribe_handler(&backend, env, args)),
    );
}

/// Default registration uses the real `XcapBackend`.
/// `media_abilities::register` skips screen names once this module
/// exists, so the screen dispatch slots are single-owner. Tests
/// that need a hardware-free path call
/// `register_with_backend(reg, Arc::new(SyntheticScreenBackend))`
/// instead.
pub fn register(reg: &mut AxonAbilityCatalog) {
    register_with_backend(reg, Arc::new(XcapBackend));
}

// ── Handler core ─────────────────────────────────────────────

fn snapshot_handler(
    backend: &Arc<dyn ScreenSnapshotBackend>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let entry = resolve_screen_subject(&env, &args, ABILITY_SCREEN_SNAPSHOT)?;

    let options = parse_capture_options(&args, false)?;
    ensure_region_allowed(&entry, &options)?;
    let EncodedFrame {
        jpeg_bytes,
        width,
        height,
    } = backend.capture_jpeg(&entry, &options)?;

    if jpeg_bytes.len() > MAX_INLINE_BYTES {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: encoded image {} bytes exceeds inline \
             cap {MAX_INLINE_BYTES}; payloadstore path not yet wired; \
             reason={REASON_IMAGE_TOO_LARGE}",
            jpeg_bytes.len()
        );
    }

    // Context-surface persistence: the device daemon keeps every
    // snapshot under `context/captures/screen.snapshot/` so the
    // Context page can browse it as <device>/<ability>/<artifact>.
    // Best-effort by design — a full disk must not fail the snapshot
    // the caller is waiting on.
    if let Err(err) = crate::persistence::context_store::record_capture(
        crate::persistence::context_store::CaptureRecord {
            device: env.callee(),
            ability: ABILITY_SCREEN_SNAPSHOT,
            ext: "jpg",
            bytes: &jpeg_bytes,
            content_type: "image/jpeg",
            width: Some(width),
            height: Some(height),
            duration_ms: None,
            preview: format!("Screenshot {width}x{height}"),
        },
    ) {
        crate::op_event!(
            component = context,
            kind = capture_persist_failed,
            level = "warn",
            ability = ABILITY_SCREEN_SNAPSHOT,
            error = err,
        );
    }

    let image_bytes_b64 = BASE64_STANDARD.encode(&jpeg_bytes);
    let captured_at = chrono::Utc::now().to_rfc3339();
    Ok(json!({
        "image_bytes_b64": image_bytes_b64,
        "content_type":    "image/jpeg",
        "width":           width,
        "height":          height,
        "byte_size":       jpeg_bytes.len(),
        "captured_at":     captured_at,
        "hardware_id":     entry.hardware_id,
    }))
}

fn subscribe_handler(
    backend: &Arc<dyn ScreenSnapshotBackend>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<StreamSource> {
    let entry = resolve_screen_subject(&env, &args, ABILITY_SCREEN_SUBSCRIBE)?;
    let options = parse_capture_options(&args, true)?;
    ensure_region_allowed(&entry, &options)?;
    let rx = backend.open_stream(entry.clone(), options)?;
    Ok(StreamSource::Live(rx))
}

fn resolve_screen_subject(
    env: &EnvelopeContext,
    args: &Value,
    ability: &'static str,
) -> anyhow::Result<ResourceEntry> {
    resolve_required_resource_subject(
        env,
        args,
        ResourceSubjectSpec {
            ability,
            required_subject: "a display/application/window",
            allowed_kinds: &[
                ResourceType::Display,
                ResourceType::Application,
                ResourceType::Window,
            ],
            allowed_label: "display/application/window",
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::resources::{
        self, upsert_resource, ResourceBinding, ResourceUpsert, ResourcesFile,
    };
    use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

    fn seed_display(file: &mut ResourcesFile, hardware_id: &str) -> String {
        upsert_resource(
            file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/device/01DEV",
                kind: ResourceType::Display,
                binding: ResourceBinding::LocalDevice,
                hardware_id,
                display_name: "Test Display",
                metadata: json!({}),
            },
        )
    }

    fn register_with_synthetic(reg: &mut AxonAbilityCatalog) {
        register_with_backend(reg, Arc::new(SyntheticScreenBackend));
    }

    #[test]
    fn synthetic_backend_emits_valid_jpeg() {
        let mut file = ResourcesFile::default();
        seed_display(&mut file, "h-display-test");
        let entry = file.resources[0].clone();
        let backend = SyntheticScreenBackend;
        let frame = backend
            .capture_jpeg(&entry, &ScreenCaptureOptions::default())
            .unwrap();
        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 48);
        assert_eq!(&frame.jpeg_bytes[..2], &[0xff, 0xd8]); // SOI
        assert_eq!(
            &frame.jpeg_bytes[frame.jpeg_bytes.len() - 2..],
            &[0xff, 0xd9]
        ); // EOI
    }

    #[test]
    fn handler_returns_receipt_with_base64_jpeg_when_subject_resolves() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "h-display-e2e");
        resources::save(&file).unwrap();
        let mut reg = AxonAbilityCatalog::new();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_SCREEN_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some(ura),
            causal_context: None,
        };
        let resp = dispatcher.execute_rpc(target).unwrap();
        for field in [
            "image_bytes_b64",
            "content_type",
            "width",
            "height",
            "byte_size",
            "captured_at",
            "hardware_id",
        ] {
            assert!(
                resp.get(field).is_some(),
                "receipt body missing `{field}`: {resp}"
            );
        }
        assert_eq!(resp["content_type"], "image/jpeg");
        assert_eq!(resp["hardware_id"], "h-display-e2e");
    }

    #[test]
    fn subscribe_returns_stream_frames_with_requested_resolution() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "h-display-stream");
        resources::save(&file).unwrap();
        let mut reg = AxonAbilityCatalog::new();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_SCREEN_SUBSCRIBE.to_string(),
            normalized_args: json!({"fps": 5, "resolution": "320x180"}),
            call_mode: CallMode::Stream,
            subject: Some(ura),
            causal_context: None,
        };
        let frames = dispatcher.execute_stream(target).unwrap().into_snapshot();
        assert!(
            !frames.is_empty(),
            "LocalRuntime bridge should surface the immediately-available synthetic frame"
        );
        assert_eq!(frames[0]["width"], json!(320));
        assert_eq!(frames[0]["height"], json!(180));
        let entry = file.resources[0].clone();
        let mut rx = SyntheticScreenBackend
            .open_stream(
                entry,
                ScreenCaptureOptions {
                    fps: 5,
                    resolution: Some(VideoResolution {
                        width: 320,
                        height: 180,
                    }),
                    region: None,
                },
            )
            .unwrap();
        let frame = rx.try_recv().expect("synthetic stream frame");
        assert_eq!(frame["width"], json!(320));
        assert_eq!(frame["height"], json!(180));
        assert!(frame["image_bytes_b64"].as_str().unwrap().len() > 100);
    }

    #[test]
    fn handler_rejects_missing_subject_with_subject_required_reason() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_SCREEN_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: None,
            causal_context: None,
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(err.to_string().contains(REASON_SUBJECT_REQUIRED));
    }

    #[test]
    fn handler_rejects_camera_subject_with_resource_type_mismatch_reason() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let cam_ura = upsert_resource(
            &mut file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/device/01DEV",
                kind: ResourceType::Camera, // wrong type for screen.snapshot
                binding: ResourceBinding::LocalDevice,
                hardware_id: "h-cam-not-screen",
                display_name: "Not A Screen",
                metadata: json!({}),
            },
        );
        resources::save(&file).unwrap();
        let mut reg = AxonAbilityCatalog::new();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_SCREEN_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some(cam_ura),
            causal_context: None,
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(err.to_string().contains(REASON_RESOURCE_TYPE_MISMATCH));
    }

    #[test]
    fn handler_rejects_unknown_subject_with_resource_not_found_reason() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        resources::save(&ResourcesFile::default()).unwrap();
        let mut reg = AxonAbilityCatalog::new();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_SCREEN_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some("easynet:///r/acme/resource/01NEVER".into()),
            causal_context: None,
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(err.to_string().contains(REASON_RESOURCE_NOT_FOUND));
    }

    #[test]
    fn handler_rejects_subject_in_args() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_SCREEN_SNAPSHOT.to_string(),
            normalized_args: json!({"subject": "easynet:///r/x/resource/y"}),
            call_mode: CallMode::Rpc,
            subject: Some("easynet:///r/acme/resource/01SCR".into()),
            causal_context: None,
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(err.to_string().contains(REASON_SUBJECT_IN_ARGS));
    }
}
