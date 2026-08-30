// EasyNet CLI — screen.snapshot real handler (RFC-005 v3.2 A8)
// ==============================================================
//
// File: src/daemon/ability/builtins/resources/media/screen_snapshot.rs
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
// resolves display resources by one explicit selector state: monitor
// id, monitor discovery index, or unpinned primary selection. Exact
// selectors never fall back to another display. Window/application
// resources are captured through xcap window selection.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "native-media")]
use std::sync::mpsc::Receiver;
use std::sync::Arc;
#[cfg(feature = "native-media")]
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "windows")
))]
use easynet_remoteapp_native_platform::PlatformWindowProcessIdentityProvider;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::daemon::ability::builtins::resources::media::resource_subject::{
    self, resolve_required_resource_subject, ResourceSubjectSpec,
};
use crate::daemon::ability::builtins::resources::media::{
    self, ABILITY_SCREEN_SNAPSHOT, ABILITY_SCREEN_SUBSCRIBE,
};
use crate::daemon::ability::dispatch::OwnerKind;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, EnvelopeContext, StreamSource};
use crate::daemon::persistence::resources::{ResourceEntry, ResourceType};
use crate::daemon::resources::context::device_scope::ContextDeviceScope;

/// 2 MiB inline cap — same shape as camera.snapshot. This keeps
/// base64-expanded receipts below Axon's 4 MiB IPC frame limit while
/// allowing ordinary laptop screenshots through. 4K/full-desktop
/// streaming still belongs on the BIDI/payloadstore path.
const MAX_INLINE_BYTES: usize = 2 * 1024 * 1024;
const BROADCAST_CAPACITY: usize = 8;
const DEFAULT_SCREEN_FPS: u32 = 60;
const MIN_SCREEN_FPS: u32 = 1;
const MAX_SCREEN_FPS: u32 = 60;
#[cfg(feature = "native-media")]
const MAX_APPLICATION_COMPOSITE_WINDOWS: usize = 32;
#[cfg(feature = "native-media")]
const MAX_APPLICATION_COMPOSITE_PIXELS: u64 = 33_177_600;

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
    /// Dimensions of the RGB payload after region/resolution processing.
    pub width: u32,
    pub height: u32,
    /// Dimensions of the native capture surface before presentation-domain
    /// crop/scale. Target identity proofs must use these dimensions; `width`
    /// and `height` belong to the encoded/presentation frame contract.
    pub native_width: u32,
    pub native_height: u32,
}

impl RawRgbFrame {
    pub fn native_dimensions(&self) -> (usize, usize) {
        (self.native_width as usize, self.native_height as usize)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenCaptureOptions {
    pub fps: u32,
    pub resolution: Option<VideoResolution>,
    pub resize_mode: CaptureResizeMode,
    pub region: Option<CaptureRegion>,
}

impl Default for ScreenCaptureOptions {
    fn default() -> Self {
        Self {
            fps: DEFAULT_SCREEN_FPS,
            resolution: None,
            resize_mode: CaptureResizeMode::Exact,
            region: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureResizeMode {
    /// Produce the explicitly requested coded dimensions. This is retained for
    /// snapshot/diagnostic callers that request an exact raster size.
    Exact,
    /// Treat the requested dimensions as upper bounds. Preserve aspect ratio
    /// and never upscale a native target merely to fill the codec canvas.
    FitWithin,
}

impl ScreenCaptureOptions {
    pub fn output_dimensions(&self, source_width: u32, source_height: u32) -> (u32, u32) {
        let Some(requested) = self.resolution else {
            return (source_width, source_height);
        };
        match self.resize_mode {
            CaptureResizeMode::Exact => (requested.width, requested.height),
            CaptureResizeMode::FitWithin => {
                fit_dimensions_within(source_width, source_height, requested)
            }
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
/// (`monitor_id` or `monitor_index`) when available and otherwise use the
/// primary monitor only for intentionally unpinned resources. Window/application
/// captures select an xcap window by recorded id, pid, title, or application
/// name.
#[derive(Debug, Default)]
#[cfg(feature = "native-media")]
pub struct XcapBackend;

#[cfg(feature = "native-media")]
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

#[cfg(feature = "native-media")]
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

#[cfg(feature = "native-media")]
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

#[cfg(not(feature = "native-media"))]
pub fn capture_rgb_with_xcap(
    entry: &ResourceEntry,
    _options: &ScreenCaptureOptions,
) -> anyhow::Result<RawRgbFrame> {
    anyhow::bail!(
        "{ABILITY_SCREEN_SNAPSHOT}: native xcap screen capture is not compiled for subject type {}; \
         reason={REASON_RESOURCE_UNAVAILABLE}",
        entry.kind.as_str()
    )
}

#[cfg(feature = "native-media")]
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

#[cfg(feature = "native-media")]
fn capture_window_rgb_with_xcap(
    entry: &ResourceEntry,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<RawRgbFrame> {
    if entry.kind == ResourceType::Application {
        return capture_application_rgb_with_xcap(entry, options);
    }
    let window = select_bound_window(entry)?;
    let rgba = window.capture_image().map_err(|e| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: xcap window capture_image failed: {e}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    ensure_xcap_window_owner(entry, &window)?;
    rgba_image_to_rgb_frame(rgba, options)
}

#[cfg(feature = "native-media")]
struct CapturedApplicationWindow {
    window_id: u64,
    z: i32,
    x: i64,
    y: i64,
    rgba: xcap::image::RgbaImage,
}

#[cfg(feature = "native-media")]
fn capture_application_rgb_with_xcap(
    entry: &ResourceEntry,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<RawRgbFrame> {
    let mut committed_ids = entry
        .metadata
        .get("resolved_window_ids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                anyhow::anyhow!(
                    "{ABILITY_SCREEN_SNAPSHOT}: application resolved_window_ids must contain integers; \
                     reason={REASON_RESOURCE_UNAVAILABLE}"
                )
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    committed_ids.sort_unstable();
    committed_ids.dedup();
    if committed_ids.is_empty() {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: application resource has no committed resolved_window_ids; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }
    if committed_ids.len() > MAX_APPLICATION_COMPOSITE_WINDOWS {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: application window set has {} windows, exceeding bounded composite limit {}; \
             reason={REASON_RESOURCE_UNAVAILABLE}",
            committed_ids.len(),
            MAX_APPLICATION_COMPOSITE_WINDOWS
        );
    }

    let windows = xcap::Window::all().map_err(|error| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: xcap Window::all failed: {error}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let mut captured = Vec::with_capacity(committed_ids.len());
    for window_id in committed_ids {
        let window = windows
            .iter()
            .find(|window| {
                window
                    .id()
                    .ok()
                    .is_some_and(|actual| u64::from(actual) == window_id)
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{ABILITY_SCREEN_SNAPSHOT}: committed application window {window_id} is no longer available; \
                     reason={REASON_RESOURCE_UNAVAILABLE}"
                )
            })?;
        ensure_xcap_window_owner(entry, window)?;
        if window.is_minimized().ok() == Some(true) {
            anyhow::bail!(
                "{ABILITY_SCREEN_SNAPSHOT}: committed application window {window_id} is minimized; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            );
        }
        let x = i64::from(window.x().map_err(|error| {
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: application window {window_id} x coordinate unavailable: {error}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?);
        let y = i64::from(window.y().map_err(|error| {
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: application window {window_id} y coordinate unavailable: {error}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?);
        let z = window.z().map_err(|error| {
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: application window {window_id} z-order unavailable: {error}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
        let rgba = window.capture_image().map_err(|error| {
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: xcap application window {window_id} capture_image failed: {error}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
        ensure_xcap_window_owner(entry, window)?;
        captured.push(CapturedApplicationWindow {
            window_id,
            z,
            x,
            y,
            rgba,
        });
    }
    compose_application_windows(captured, options)
}

#[cfg(feature = "native-media")]
fn compose_application_windows(
    mut windows: Vec<CapturedApplicationWindow>,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<RawRgbFrame> {
    // xcap reports a larger z for windows nearer the user. Composite from the
    // back forward so overlap matches the actual host surface.
    windows.sort_by_key(|window| window.z);
    let mut min_x = i64::MAX;
    let mut min_y = i64::MAX;
    let mut max_x = i64::MIN;
    let mut max_y = i64::MIN;
    for window in &windows {
        if window.rgba.width() == 0 || window.rgba.height() == 0 {
            anyhow::bail!(
                "{ABILITY_SCREEN_SNAPSHOT}: application window {} returned an empty frame; \
                 reason={REASON_RESOURCE_UNAVAILABLE}",
                window.window_id
            );
        }
        min_x = min_x.min(window.x);
        min_y = min_y.min(window.y);
        max_x = max_x.max(
            window
                .x
                .checked_add(i64::from(window.rgba.width()))
                .ok_or_else(|| anyhow::anyhow!("application composite x bound overflow"))?,
        );
        max_y = max_y.max(
            window
                .y
                .checked_add(i64::from(window.rgba.height()))
                .ok_or_else(|| anyhow::anyhow!("application composite y bound overflow"))?,
        );
    }
    let width = u32::try_from(max_x.checked_sub(min_x).unwrap_or_default()).map_err(|_| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: application composite width is out of range; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let height = u32::try_from(max_y.checked_sub(min_y).unwrap_or_default()).map_err(|_| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: application composite height is out of range; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width == 0 || height == 0 || pixels > MAX_APPLICATION_COMPOSITE_PIXELS {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: application composite {}x{} exceeds bounded pixel limit {}; \
             reason={REASON_RESOURCE_UNAVAILABLE}",
            width,
            height,
            MAX_APPLICATION_COMPOSITE_PIXELS
        );
    }
    let byte_len = usize::try_from(pixels.saturating_mul(4)).map_err(|_| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: application composite allocation is out of range; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let mut composite = vec![0_u8; byte_len];
    for pixel in composite.chunks_exact_mut(4) {
        pixel[3] = 255;
    }
    for window in windows {
        let offset_x = usize::try_from(window.x - min_x).expect("x offset is non-negative");
        let offset_y = usize::try_from(window.y - min_y).expect("y offset is non-negative");
        let source_width = window.rgba.width() as usize;
        for (source_y, row) in window
            .rgba
            .as_raw()
            .chunks_exact(source_width * 4)
            .enumerate()
        {
            for (source_x, source) in row.chunks_exact(4).enumerate() {
                let target_x = offset_x + source_x;
                let target_y = offset_y + source_y;
                let target_offset = (target_y * width as usize + target_x) * 4;
                let target = &mut composite[target_offset..target_offset + 4];
                let alpha = u16::from(source[3]);
                let inverse = 255_u16 - alpha;
                for channel in 0..3 {
                    target[channel] = ((u16::from(source[channel]) * alpha
                        + u16::from(target[channel]) * inverse)
                        / 255) as u8;
                }
            }
        }
    }
    rgba_bytes_to_rgb_frame(composite, width, height, options)
}

#[cfg(feature = "native-media")]
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
    let (output_width, output_height) = options.output_dimensions(width, height);
    if (output_width, output_height) != (width, height) {
        rgb = resize_rgb_nearest(&rgb, width, height, output_width, output_height);
        width = output_width;
        height = output_height;
    }
    Ok(RawRgbFrame {
        rgb_bytes: rgb,
        width,
        height,
        native_width: source_width,
        native_height: source_height,
    })
}

pub fn bgra_bytes_to_rgb_frame(
    bgra: &[u8],
    source_width: u32,
    source_height: u32,
    source_stride: usize,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<RawRgbFrame> {
    let minimum_stride = source_width as usize * 4;
    if source_width == 0
        || source_height == 0
        || source_stride < minimum_stride
        || bgra.len() < source_stride.saturating_mul(source_height as usize)
    {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: invalid BGRA frame dimensions {source_width}x{source_height} stride={source_stride}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }

    let region = options.region.unwrap_or(CaptureRegion {
        x: 0,
        y: 0,
        w: source_width,
        h: source_height,
    });
    validate_region(region, source_width, source_height)?;

    let mut rgb = Vec::with_capacity((region.w * region.h * 3) as usize);
    for y in region.y..region.y + region.h {
        let row_start = y as usize * source_stride + region.x as usize * 4;
        let row_end = row_start + region.w as usize * 4;
        let row = &bgra[row_start..row_end];
        for px in row.chunks_exact(4) {
            rgb.push(px[2]);
            rgb.push(px[1]);
            rgb.push(px[0]);
        }
    }

    let mut width = region.w;
    let mut height = region.h;
    let (output_width, output_height) = options.output_dimensions(width, height);
    if (output_width, output_height) != (width, height) {
        rgb = resize_rgb_nearest(&rgb, width, height, output_width, output_height);
        width = output_width;
        height = output_height;
    }
    Ok(RawRgbFrame {
        rgb_bytes: rgb,
        width,
        height,
        native_width: source_width,
        native_height: source_height,
    })
}

pub fn rgb_frame_to_jpeg(frame: RawRgbFrame) -> anyhow::Result<EncodedFrame> {
    let jpeg = encode_jpeg_checked(&frame.rgb_bytes, frame.width, frame.height)?;
    Ok(EncodedFrame {
        jpeg_bytes: jpeg,
        width: frame.width,
        height: frame.height,
    })
}

#[cfg(feature = "native-media")]
fn select_monitor(
    monitors: Vec<xcap::Monitor>,
    entry: &ResourceEntry,
) -> anyhow::Result<xcap::Monitor> {
    match display_monitor_selector(entry)? {
        DisplayMonitorSelector::PlatformId(expected_id) => {
            for monitor in monitors {
                if monitor
                    .id()
                    .ok()
                    .is_some_and(|actual| actual as u64 == expected_id)
                {
                    return Ok(monitor);
                }
            }
            Err(display_monitor_unavailable(
                "requested monitor_id is no longer available",
            ))
        }
        DisplayMonitorSelector::DiscoveryIndex(expected_index) => {
            for (idx, monitor) in monitors.into_iter().enumerate() {
                if idx as u64 == expected_index {
                    return Ok(monitor);
                }
            }
            Err(display_monitor_unavailable(
                "requested monitor_index is no longer available",
            ))
        }
        DisplayMonitorSelector::PrimaryUnpinned => monitors
            .into_iter()
            .find(|monitor| monitor.is_primary().unwrap_or(false))
            .ok_or_else(|| display_monitor_unavailable("no primary monitor reported by xcap")),
    }
}

#[cfg(feature = "native-media")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisplayMonitorSelector {
    PlatformId(u64),
    DiscoveryIndex(u64),
    PrimaryUnpinned,
}

#[cfg(feature = "native-media")]
fn display_monitor_selector(entry: &ResourceEntry) -> anyhow::Result<DisplayMonitorSelector> {
    if let Some(value) = entry.metadata.get("monitor_id") {
        return value
            .as_u64()
            .map(DisplayMonitorSelector::PlatformId)
            .ok_or_else(|| {
                display_monitor_unavailable(
                    "display resource monitor_id metadata must be an integer",
                )
            });
    }
    if let Some(value) = entry.metadata.get("monitor_index") {
        return value
            .as_u64()
            .map(DisplayMonitorSelector::DiscoveryIndex)
            .ok_or_else(|| {
                display_monitor_unavailable(
                    "display resource monitor_index metadata must be an integer",
                )
            });
    }
    Ok(DisplayMonitorSelector::PrimaryUnpinned)
}

#[cfg(feature = "native-media")]
fn display_monitor_unavailable(detail: &str) -> anyhow::Error {
    anyhow::anyhow!("{ABILITY_SCREEN_SNAPSHOT}: {detail}; reason={REASON_RESOURCE_UNAVAILABLE}")
}

#[cfg(feature = "native-media")]
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

#[cfg(feature = "native-media")]
fn select_bound_window(entry: &ResourceEntry) -> anyhow::Result<xcap::Window> {
    let windows = xcap::Window::all().map_err(|e| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: xcap Window::all failed: {e}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    if entry.kind != ResourceType::Window {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: exact window selector received subject type {}; \
             reason={REASON_RESOURCE_TYPE_MISMATCH}",
            entry.kind.as_str()
        );
    }
    select_window_by_exact_identity(windows, entry)
}

/// Resolve one committed Window Resource by exact native id and owner.
/// A closed/reopened window is a new target even when PID, app name, or title
/// happen to match; silently selecting it would violate the session subject.
#[cfg(feature = "native-media")]
fn select_window_by_exact_identity(
    windows: Vec<xcap::Window>,
    entry: &ResourceEntry,
) -> anyhow::Result<xcap::Window> {
    let expected_id = entry
        .metadata
        .get("window_id")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: window resource missing window_id; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
    let window = windows
        .into_iter()
        .find(|window| {
            window
                .id()
                .ok()
                .is_some_and(|actual| u64::from(actual) == expected_id)
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: requested window {expected_id} is no longer available; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
    ensure_xcap_window_owner(entry, &window)?;
    Ok(window)
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "windows")
))]
fn ensure_xcap_window_owner(entry: &ResourceEntry, window: &xcap::Window) -> anyhow::Result<()> {
    let expected_pid = entry
        .metadata
        .get("pid")
        .or_else(|| entry.metadata.get("primary_pid"))
        .and_then(Value::as_u64);
    let expected_process_instance_id = entry
        .metadata
        .get("process_instance_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: window/application resource has no process_instance_id; reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
    let window_id = window.id().map(u64::from).map_err(|error| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: read native window id: {error}; reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let provider = PlatformWindowProcessIdentityProvider::connect().map_err(|error| {
        anyhow::anyhow!(
            "{ABILITY_SCREEN_SNAPSHOT}: initialize platform process identity provider: {error}; reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let actual = provider
        .resolve_window(window_id)
        .map_err(|error| {
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: resolve native window owner for {window_id}: {error}; reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{ABILITY_SCREEN_SNAPSHOT}: native window owner is unavailable; reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
    if expected_pid != Some(u64::from(actual.pid()))
        || actual.stable_id() != expected_process_instance_id
    {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: native owner process instance no longer matches the committed resource; reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }
    Ok(())
}

#[cfg(all(
    feature = "native-media",
    not(any(target_os = "linux", target_os = "windows"))
))]
fn ensure_xcap_window_owner(entry: &ResourceEntry, window: &xcap::Window) -> anyhow::Result<()> {
    let expected_pid = entry
        .metadata
        .get("pid")
        .or_else(|| entry.metadata.get("primary_pid"))
        .and_then(Value::as_u64);
    let expected_app = entry.metadata.get("app_name").and_then(Value::as_str);
    let actual_pid = observed_xcap_window_pid(window)?;
    let actual_app = window.app_name().ok();
    if expected_pid.is_none() && expected_app.is_none() {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: window/application resource has no owner identity; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }
    if !native_window_owner_matches(
        expected_pid,
        expected_app,
        actual_pid,
        actual_app.as_deref(),
    ) {
        anyhow::bail!(
            "{ABILITY_SCREEN_SNAPSHOT}: native window owner no longer matches the committed resource; \
             expected_pid={expected_pid:?}, actual_pid={actual_pid:?}, \
             expected_app={expected_app:?}, actual_app={actual_app:?}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }
    Ok(())
}

#[cfg(all(
    feature = "native-media",
    any(test, not(any(target_os = "linux", target_os = "windows")))
))]
fn native_window_owner_matches(
    expected_pid: Option<u64>,
    expected_app: Option<&str>,
    actual_pid: Option<u64>,
    actual_app: Option<&str>,
) -> bool {
    match expected_pid {
        // A committed process id is the authoritative owner identity. The app
        // name is display metadata and X11 backends may normalize its casing
        // differently while still resolving the exact same local process.
        Some(expected) => actual_pid == Some(expected),
        None => expected_app.is_some_and(|expected| {
            actual_app.is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
        }),
    }
}

#[cfg(all(
    feature = "native-media",
    not(any(target_os = "linux", target_os = "windows"))
))]
fn observed_xcap_window_pid(window: &xcap::Window) -> anyhow::Result<Option<u64>> {
    Ok(window.pid().ok().map(u64::from))
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

fn fit_dimensions_within(
    source_width: u32,
    source_height: u32,
    bounds: VideoResolution,
) -> (u32, u32) {
    if source_width == 0 || source_height == 0 {
        return (source_width, source_height);
    }
    if source_width <= bounds.width && source_height <= bounds.height {
        return (source_width, source_height);
    }
    let width_limited_height = rounded_scaled_dimension(
        u64::from(source_height),
        u64::from(bounds.width),
        u64::from(source_width),
    );
    let height_limited_width = rounded_scaled_dimension(
        u64::from(source_width),
        u64::from(bounds.height),
        u64::from(source_height),
    );
    if width_limited_height <= u64::from(bounds.height) {
        (bounds.width.max(1), width_limited_height.max(1) as u32)
    } else {
        (height_limited_width.max(1) as u32, bounds.height.max(1))
    }
}

fn rounded_scaled_dimension(value: u64, numerator: u64, denominator: u64) -> u64 {
    value
        .saturating_mul(numerator)
        .saturating_add(denominator / 2)
        / denominator.max(1)
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
        let (tx, rx) = broadcast::channel::<Value>(BROADCAST_CAPACITY);
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
    reg.register_rpc_with_envelope_and_spec(
        ABILITY_SCREEN_SNAPSHOT,
        OwnerKind::media_system(),
        media::registry_manifest(ABILITY_SCREEN_SNAPSHOT),
        Arc::new(move |env: EnvelopeContext, args: Value| {
            snapshot_handler(&snapshot_backend, env, args)
        }),
    );
    reg.register_stream_with_envelope_and_spec(
        ABILITY_SCREEN_SUBSCRIBE,
        OwnerKind::media_system(),
        media::registry_manifest(ABILITY_SCREEN_SUBSCRIBE),
        Arc::new(move |env: EnvelopeContext, args: Value| subscribe_handler(&backend, env, args)),
    );
}

/// Default registration uses the real `XcapBackend`.
/// `media::register` skips screen names once this module
/// exists, so the screen dispatch slots are single-owner. Tests
/// that need a hardware-free path call
/// `register_with_backend(reg, Arc::new(SyntheticScreenBackend))`
/// instead.
pub fn register(reg: &mut AxonAbilityCatalog) {
    #[cfg(feature = "native-media")]
    register_with_backend(reg, Arc::new(XcapBackend));
    #[cfg(all(not(feature = "native-media"), feature = "headless-media"))]
    register_with_backend(reg, Arc::new(SyntheticScreenBackend));
}

// ── Handler core ─────────────────────────────────────────────

fn snapshot_handler(
    backend: &Arc<dyn ScreenSnapshotBackend>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let entry = resolve_screen_subject(&env, &args, ABILITY_SCREEN_SNAPSHOT)?;
    let device_scope = ContextDeviceScope::from_execution_actor(env.callee())?;

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

    // Snapshot success includes durable Context persistence. Returning an
    // inline image while silently losing its catalog entry would expose two
    // conflicting terminal states for the same capture.
    let capture = crate::daemon::persistence::context_store::record_capture(
        crate::daemon::persistence::context_store::CaptureRecord {
            device: device_scope.as_str(),
            ability: ABILITY_SCREEN_SNAPSHOT,
            ext: "jpg",
            bytes: &jpeg_bytes,
            content_type: "image/jpeg",
            width: Some(width),
            height: Some(height),
            duration_ms: None,
            preview: format!("Screenshot {width}x{height}"),
        },
    )?;
    let local_path = crate::daemon::persistence::context_store::captures_dir()
        .join(ABILITY_SCREEN_SNAPSHOT)
        .join(&capture.file);

    let image_bytes_b64 = BASE64_STANDARD.encode(&jpeg_bytes);
    Ok(json!({
        "image_bytes_b64": image_bytes_b64,
        "content_type":    "image/jpeg",
        "width":           width,
        "height":          height,
        "byte_size":       jpeg_bytes.len(),
        "captured_at":     capture.timestamp,
        "hardware_id":     entry.hardware_id,
        "capture_id":      capture.id,
        "capture_file":    capture.file,
        "local_path":      local_path.display().to_string(),
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
    use crate::daemon::invocation::routing::target::CallMode;
    use crate::daemon::persistence::resources::{
        self, upsert_resource, ResourceBinding, ResourceUpsert, ResourcesFile,
    };

    #[cfg(feature = "native-media")]
    #[test]
    fn native_window_owner_treats_exact_pid_as_authoritative() {
        assert!(native_window_owner_matches(
            Some(2971),
            Some("Easynetremoteappsentinel"),
            Some(2971),
            Some("EasyNetRemoteAppSentinel"),
        ));
    }

    #[cfg(feature = "native-media")]
    #[test]
    fn native_window_owner_rejects_pid_mismatch_even_when_app_matches() {
        assert!(!native_window_owner_matches(
            Some(2971),
            Some("EasyNetRemoteAppSentinel"),
            Some(4000),
            Some("EasyNetRemoteAppSentinel"),
        ));
    }

    #[cfg(feature = "native-media")]
    #[test]
    fn native_window_owner_app_fallback_is_case_insensitive_without_pid() {
        assert!(native_window_owner_matches(
            None,
            Some("Easynetremoteappsentinel"),
            None,
            Some("EasyNetRemoteAppSentinel"),
        ));
    }

    fn seed_display(file: &mut ResourcesFile, hardware_id: &str) -> String {
        seed_display_with_metadata(file, hardware_id, json!({}))
    }

    fn seed_display_with_metadata(
        file: &mut ResourcesFile,
        hardware_id: &str,
        metadata: Value,
    ) -> String {
        upsert_resource(
            file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
                kind: ResourceType::Display,
                binding: ResourceBinding::LocalDevice,
                hardware_id,
                display_name: "Test Display",
                metadata,
            },
        )
        .expect("seed display resource")
    }

    fn register_with_synthetic(reg: &mut AxonAbilityCatalog) {
        register_with_backend(reg, Arc::new(SyntheticScreenBackend));
    }

    const TEST_DEVICE_URA: &str = "easynet:///r/test/device/screen-snapshot";

    fn metadata_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_metadata_for_device_authority(TEST_DEVICE_URA)
    }

    fn executable_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_runtime_for_device_authority(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            TEST_DEVICE_URA,
        )
    }

    #[cfg(feature = "native-media")]
    #[test]
    fn application_compositor_preserves_exact_window_union_without_display_pixels() {
        let red = xcap::image::RgbaImage::from_raw(
            2,
            2,
            vec![
                255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
            ],
        )
        .expect("red window image");
        let green = xcap::image::RgbaImage::from_raw(
            2,
            2,
            vec![
                0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255, 0, 255,
            ],
        )
        .expect("green window image");

        let frame = compose_application_windows(
            vec![
                CapturedApplicationWindow {
                    window_id: 10,
                    z: 0,
                    x: -1,
                    y: 5,
                    rgba: red,
                },
                CapturedApplicationWindow {
                    window_id: 11,
                    z: 1,
                    x: 1,
                    y: 5,
                    rgba: green,
                },
            ],
            &ScreenCaptureOptions::default(),
        )
        .expect("exact application windows compose");

        assert_eq!((frame.width, frame.height), (4, 2));
        assert_eq!(&frame.rgb_bytes[0..3], &[255, 0, 0]);
        assert_eq!(&frame.rgb_bytes[6..9], &[0, 255, 0]);
    }

    #[cfg(feature = "native-media")]
    #[test]
    fn application_compositor_keeps_native_surface_dimensions_when_output_is_scaled() {
        let rgba = xcap::image::RgbaImage::from_raw(4, 2, vec![0x7f; 4 * 2 * 4])
            .expect("application window image");
        let frame = compose_application_windows(
            vec![CapturedApplicationWindow {
                window_id: 10,
                z: 0,
                x: 15,
                y: 25,
                rgba,
            }],
            &ScreenCaptureOptions {
                fps: 30,
                resolution: Some(VideoResolution {
                    width: 1280,
                    height: 720,
                }),
                resize_mode: CaptureResizeMode::Exact,
                region: None,
            },
        )
        .expect("scaled application surface");

        assert_eq!((frame.width, frame.height), (1280, 720));
        assert_eq!(frame.native_dimensions(), (4, 2));
    }

    #[cfg(feature = "native-media")]
    #[test]
    fn application_compositor_cross_display_gap_is_black_not_host_display_content() {
        let pixel =
            |rgba| xcap::image::RgbaImage::from_raw(1, 1, rgba).expect("one-pixel window image");
        let frame = compose_application_windows(
            vec![
                CapturedApplicationWindow {
                    window_id: 10,
                    z: 0,
                    x: -2,
                    y: -1,
                    rgba: pixel(vec![255, 0, 0, 255]),
                },
                CapturedApplicationWindow {
                    window_id: 11,
                    z: 0,
                    x: 2,
                    y: -1,
                    rgba: pixel(vec![0, 255, 0, 255]),
                },
            ],
            &ScreenCaptureOptions::default(),
        )
        .expect("cross-display application windows compose in virtual-desktop coordinates");

        assert_eq!((frame.width, frame.height), (5, 1));
        assert_eq!(&frame.rgb_bytes[0..3], &[255, 0, 0]);
        assert_eq!(&frame.rgb_bytes[3..12], &[0; 9]);
        assert_eq!(&frame.rgb_bytes[12..15], &[0, 255, 0]);
    }

    #[cfg(feature = "native-media")]
    #[test]
    fn application_compositor_preserves_host_window_z_order() {
        let pixel =
            |rgba| xcap::image::RgbaImage::from_raw(1, 1, rgba).expect("one-pixel window image");
        let frame = compose_application_windows(
            vec![
                CapturedApplicationWindow {
                    window_id: 20,
                    z: 9,
                    x: 0,
                    y: 0,
                    rgba: pixel(vec![0, 255, 0, 255]),
                },
                CapturedApplicationWindow {
                    window_id: 10,
                    z: 1,
                    x: 0,
                    y: 0,
                    rgba: pixel(vec![255, 0, 0, 255]),
                },
            ],
            &ScreenCaptureOptions::default(),
        )
        .expect("overlapping application windows compose in host z-order");

        assert_eq!(&frame.rgb_bytes, &[0, 255, 0]);
    }

    #[cfg(feature = "native-media")]
    #[test]
    fn application_compositor_rejects_unbounded_union_before_allocation() {
        let pixel = || {
            xcap::image::RgbaImage::from_raw(1, 1, vec![255, 255, 255, 255])
                .expect("one-pixel window image")
        };
        let error = compose_application_windows(
            vec![
                CapturedApplicationWindow {
                    window_id: 10,
                    z: 0,
                    x: 0,
                    y: 0,
                    rgba: pixel(),
                },
                CapturedApplicationWindow {
                    window_id: 11,
                    z: 1,
                    x: 100_000,
                    y: 100_000,
                    rgba: pixel(),
                },
            ],
            &ScreenCaptureOptions::default(),
        )
        .expect_err("unbounded application surface must fail before allocation");

        assert!(error.to_string().contains("bounded pixel limit"));
    }

    #[test]
    fn registration_publishes_screen_descriptors_to_catalog_snapshot() {
        let mut reg = metadata_test_catalog();
        register_with_synthetic(&mut reg);
        let rows = reg.authority_ability_catalog_snapshot();

        for ability in [ABILITY_SCREEN_SNAPSHOT, ABILITY_SCREEN_SUBSCRIBE] {
            let descriptor = rows
                .iter()
                .find(|row| row.name == ability)
                .map(|row| &row.descriptor)
                .unwrap_or_else(|| panic!("{ability} must publish canonical descriptor"));
            assert_eq!(
                descriptor.description,
                media::description(ability).expect("screen description")
            );
            assert_eq!(
                descriptor.input_schema(),
                &media::input_schema(ability).expect("screen schema")
            );
        }
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

    #[cfg(feature = "native-media")]
    #[test]
    fn display_monitor_selector_prefers_platform_monitor_id() {
        let mut file = ResourcesFile::default();
        seed_display_with_metadata(
            &mut file,
            "h-display-selector-id",
            json!({"monitor_id": 42, "monitor_index": 7}),
        );

        assert_eq!(
            display_monitor_selector(&file.resources[0]).unwrap(),
            DisplayMonitorSelector::PlatformId(42),
            "platform monitor id is the stable selector and must not fall through to index"
        );
    }

    #[cfg(feature = "native-media")]
    #[test]
    fn display_monitor_selector_uses_index_only_without_monitor_id() {
        let mut file = ResourcesFile::default();
        seed_display_with_metadata(
            &mut file,
            "h-display-selector-index",
            json!({"monitor_index": 2}),
        );

        assert_eq!(
            display_monitor_selector(&file.resources[0]).unwrap(),
            DisplayMonitorSelector::DiscoveryIndex(2)
        );
    }

    #[cfg(feature = "native-media")]
    #[test]
    fn display_monitor_selector_allows_primary_only_for_unpinned_resource() {
        let mut file = ResourcesFile::default();
        seed_display(&mut file, "h-display-selector-unpinned");

        assert_eq!(
            display_monitor_selector(&file.resources[0]).unwrap(),
            DisplayMonitorSelector::PrimaryUnpinned
        );
    }

    #[cfg(feature = "native-media")]
    #[test]
    fn display_monitor_selector_rejects_malformed_metadata_instead_of_primary_fallback() {
        let mut file = ResourcesFile::default();
        seed_display_with_metadata(
            &mut file,
            "h-display-selector-malformed",
            json!({"monitor_id": "42"}),
        );

        let error = display_monitor_selector(&file.resources[0])
            .expect_err("malformed monitor_id must fail before primary fallback");
        let message = error.to_string();
        assert!(message.contains("monitor_id metadata must be an integer"));
        assert!(message.contains(REASON_RESOURCE_UNAVAILABLE));
    }

    #[test]
    fn handler_returns_receipt_with_base64_jpeg_when_subject_resolves() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "h-display-e2e");
        resources::save(&file).unwrap();
        let mut reg = executable_catalog();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_SCREEN_SNAPSHOT,
            json!({}),
            CallMode::Rpc,
            ura,
        );
        let resp = dispatcher.execute_rpc(target).unwrap();
        for field in [
            "image_bytes_b64",
            "content_type",
            "width",
            "height",
            "byte_size",
            "captured_at",
            "hardware_id",
            "capture_id",
            "capture_file",
            "local_path",
        ] {
            assert!(
                resp.get(field).is_some(),
                "receipt body missing `{field}`: {resp}"
            );
        }
        assert_eq!(resp["content_type"], "image/jpeg");
        assert_eq!(resp["hardware_id"], "h-display-e2e");
        let captures = crate::daemon::persistence::context_store::list_captures(
            TEST_DEVICE_URA,
            Some(ABILITY_SCREEN_SNAPSHOT),
            10,
        )
        .unwrap();
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].id, resp["capture_id"].as_str().unwrap());
    }

    #[test]
    fn subscribe_returns_stream_frames_with_requested_resolution() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_display(&mut file, "h-display-stream");
        resources::save(&file).unwrap();
        let mut reg = executable_catalog();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_SCREEN_SUBSCRIBE,
            json!({"fps": 5, "resolution": "320x180"}),
            CallMode::Stream,
            ura,
        );
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
                    resize_mode: CaptureResizeMode::Exact,
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
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = executable_catalog();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target =
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
                ABILITY_SCREEN_SNAPSHOT,
                json!({}),
                CallMode::Rpc,
            );
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(err.to_string().contains(REASON_SUBJECT_REQUIRED));
    }

    #[test]
    fn handler_rejects_camera_subject_with_resource_type_mismatch_reason() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let cam_ura = upsert_resource(
            &mut file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
                kind: ResourceType::Camera, // wrong type for screen.snapshot
                binding: ResourceBinding::LocalDevice,
                hardware_id: "h-cam-not-screen",
                display_name: "Not A Screen",
                metadata: json!({}),
            },
        )
        .expect("seed wrong-type camera resource");
        resources::save(&file).unwrap();
        let mut reg = executable_catalog();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_SCREEN_SNAPSHOT,
            json!({}),
            CallMode::Rpc,
            cam_ura,
        );
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(err.to_string().contains(REASON_RESOURCE_TYPE_MISMATCH));
    }

    #[test]
    fn handler_rejects_unknown_subject_with_resource_not_found_reason() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        resources::save(&ResourcesFile::default()).unwrap();
        let mut reg = executable_catalog();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_SCREEN_SNAPSHOT,
            json!({}),
            CallMode::Rpc,
            "easynet:///r/acme/resource/01NEVER",
        );
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(err.to_string().contains(REASON_RESOURCE_NOT_FOUND));
    }

    #[test]
    fn handler_rejects_subject_in_args() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = executable_catalog();
        register_with_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_SCREEN_SNAPSHOT,
            json!({"subject": "easynet:///r/x/resource/y"}),
            CallMode::Rpc,
            "easynet:///r/acme/resource/01SCR",
        );
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(err.to_string().contains(REASON_SUBJECT_IN_ARGS));
    }
}
