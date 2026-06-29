// EasyNet CLI — camera.snapshot real handler (RFC-005 v3.2 A3)
// ==============================================================
//
// File: src/runtime/agents/media/camera_snapshot.rs
//
// PR3a vertical slice. Replaces the `query_stub` in
// `media_abilities.rs` for the `camera.snapshot` name with a real
// envelope-aware handler that:
//
//   1. Reads `EnvelopeContext.subject` (per **INV-SUBJECT-ENVELOPE**:
//      handler MUST get subject from the envelope, NOT from args).
//   2. Resolves subject → `ResourceEntry` via
//      `persistence::resources::lookup_by_ura`.
//   3. Branches per **INV-RESOURCE-VALIDITY**:
//        * subject absent     → InvalidArgument with reason="subject_required"
//        * URA not in table   → terminal failure with reason="resource_not_found"
//        * type ≠ camera      → terminal failure with reason="resource_type_mismatch"
//        * (PR3a synthetic backend skips the "binding alive" check
//          — every present entry is "available"; the real
//          cpal/nokhwa swap adds the device-still-plugged-in test
//          and returns reason="resource_unavailable" when the
//          camera was unplugged after the table was last scanned.)
//   4. Captures one frame via the configured backend
//      (`SyntheticBackend` in PR3a — produces a deterministic
//       64×48 JPEG so the wire path is testable without hardware
//       or macOS permission prompts).
//   5. base64-encodes the JPEG bytes (per the design discussion —
//      base64 inline is the right tradeoff for snapshot-shaped
//      receipts up to a small-blob threshold; large-image overflow
//      to PayloadStore is the next consumer's PR).
//   6. Returns the receipt body shape declared in
//      `media_abilities::ABILITY_CAMERA_SNAPSHOT`'s description:
//      `{ image_bytes_b64, captured_at, content_type, width,
//         height, hardware_id, local_path }`.
//
// What's NOT in PR3a
// ------------------
// * Real camera capture (cpal/nokhwa). The `SnapshotBackend` trait
//   is the seam: a future PR drops in a `NokhwaBackend` impl
//   without touching the dispatch / receipt code.
// * PayloadStore overflow path for >2 MiB images. Returns a
//   clear "image too large for inline; payloadstore path not yet
//   wired" error so a future operator hitting the limit gets a
//   pointer instead of a silent truncation.
// * Args parsing for `format` / `region`. PR3a accepts whatever
//   args are passed (only `subject` matters) and always emits
//   JPEG. Wiring the format selector is a follow-up.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::fs;
use std::path::PathBuf;
#[cfg(not(target_os = "macos"))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(not(target_os = "macos"))]
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::persistence::config::{atomic_write_with_permissions, state_dir, WritePermissions};
use crate::persistence::resources::{ResourceEntry, ResourceType};
use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::ability_dispatch::{AxonAbilityCatalog, EnvelopeContext, StreamSource};
use crate::runtime::agents::media::resource_subject::{
    self, resolve_required_resource_subject, ResourceSubjectSpec,
};
use crate::runtime::agents::media_abilities::{ABILITY_CAMERA_SNAPSHOT, ABILITY_CAMERA_SUBSCRIBE};

/// Maximum inline image size, in encoded JPEG bytes (NOT the base64
/// expansion). Above this the handler refuses with an explicit
/// "use payloadstore" error rather than risking an oversized Axon
/// frame. 2 MiB keeps the base64-expanded body below the 4 MiB IPC
/// frame limit while allowing normal laptop camera frames through.
const MAX_INLINE_BYTES: usize = 2 * 1024 * 1024;
const BROADCAST_CAPACITY: usize = 8;
const DEFAULT_CAMERA_FPS: u32 = 30;
const MIN_CAMERA_FPS: u32 = 1;
const MAX_CAMERA_FPS: u32 = 60;
#[cfg(not(target_os = "macos"))]
const NOKHWA_CAPTURE_TIMEOUT: Duration = Duration::from_secs(3);

/// Reason strings the handler emits on terminal failures. Pinned
/// as constants so the integration tests + PR2 sibling-handler
/// guards reference the exact same strings.
pub const REASON_SUBJECT_REQUIRED: &str = resource_subject::REASON_SUBJECT_REQUIRED;
pub const REASON_SUBJECT_IN_ARGS: &str = resource_subject::REASON_SUBJECT_IN_ARGS;
pub const REASON_RESOURCE_NOT_FOUND: &str = resource_subject::REASON_RESOURCE_NOT_FOUND;
pub const REASON_RESOURCE_TYPE_MISMATCH: &str = resource_subject::REASON_RESOURCE_TYPE_MISMATCH;
/// Camera was in the resources table at scan time but is now
/// busy / unplugged / permission-denied. Distinct from
/// `resource_not_found` per INV-RESOURCE-VALIDITY: the URA
/// resolves to an entry, but the underlying device cannot serve
/// a frame right now.
pub const REASON_RESOURCE_UNAVAILABLE: &str = "resource_unavailable";
pub const REASON_PERMISSION_DENIED: &str = "permission_denied";
pub const REASON_IMAGE_TOO_LARGE: &str = "image_too_large_for_inline";

// ── Backend trait ────────────────────────────────────────────

/// One-shot camera capture. The trait is the seam between this
/// PR (which provides `SyntheticBackend`) and a future PR (which
/// drops in `NokhwaBackend` or similar without touching the
/// dispatch / receipt-shaping code above).
pub trait SnapshotBackend: Send + Sync {
    /// Capture one frame from the resource described by `entry`.
    /// Returns the encoded JPEG bytes plus the actual dimensions
    /// (so the receipt can record what was captured even when the
    /// requested resolution couldn't be honoured).
    fn capture_jpeg(&self, entry: &ResourceEntry) -> anyhow::Result<EncodedFrame>;

    /// Open a realtime preview stream. Frames are shaped like
    /// `camera.snapshot` receipts but are transient: the live path
    /// must not persist capture artifacts.
    fn open_stream(
        &self,
        entry: ResourceEntry,
        options: CameraStreamOptions,
    ) -> anyhow::Result<broadcast::Receiver<Value>>;
}

#[derive(Debug, Clone)]
pub struct EncodedFrame {
    pub jpeg_bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraStreamOptions {
    pub fps: u32,
    pub resolution: Option<CameraVideoResolution>,
}

impl Default for CameraStreamOptions {
    fn default() -> Self {
        Self {
            fps: DEFAULT_CAMERA_FPS,
            resolution: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraVideoResolution {
    pub width: u32,
    pub height: u32,
}

// ── Nokhwa backend (real, PR3) ───────────────────────────────

/// Real camera backend backed by `nokhwa`. Opens the platform's
/// default backend (AVFoundation on macOS, V4L2 on Linux), grabs
/// one device-selected frame, decodes to RGB, and re-encodes as
/// JPEG to match the receipt body shape camera.snapshot promises.
///
/// `hardware_id` ↔ camera index mapping
/// ------------------------------------
/// v1 ignores `entry.hardware_id` and grabs the default camera
/// (CameraIndex::Index(0)). The mint-an-index-per-camera scan
/// happens in the daemon's first-boot resource scan, but the
/// stable mapping back to a `CameraIndex` is platform-specific
/// and not yet plumbed through `ResourceEntry.metadata`. As soon
/// as the scan records the index, this backend reads it from
/// `entry.metadata["camera_index"]`.
///
/// Failure mapping
/// ---------------
/// Every `NokhwaError` becomes `reason="resource_unavailable"` —
/// the camera was in the resources table at scan time but is now
/// busy / unplugged / permission-denied. This matches
/// INV-RESOURCE-VALIDITY's split between
/// `resource_not_found` (handled by the dispatch layer above)
/// and `resource_unavailable` (this branch).
#[derive(Debug, Default)]
pub struct NokhwaBackend;

impl SnapshotBackend for NokhwaBackend {
    fn capture_jpeg(&self, entry: &ResourceEntry) -> anyhow::Result<EncodedFrame> {
        #[cfg(target_os = "macos")]
        {
            super::avfoundation_camera::capture_jpeg(entry)
        }

        #[cfg(not(target_os = "macos"))]
        capture_jpeg_with_nokhwa_with_timeout(entry)
    }

    fn open_stream(
        &self,
        entry: ResourceEntry,
        options: CameraStreamOptions,
    ) -> anyhow::Result<broadcast::Receiver<Value>> {
        #[cfg(target_os = "macos")]
        {
            super::avfoundation_camera::open_jpeg_stream(entry, options)
        }

        #[cfg(not(target_os = "macos"))]
        open_stream_with_nokhwa(entry, options)
    }
}

#[cfg(not(target_os = "macos"))]
fn capture_jpeg_with_nokhwa_with_timeout(entry: &ResourceEntry) -> anyhow::Result<EncodedFrame> {
    let entry = entry.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(capture_jpeg_with_nokhwa(&entry));
    });

    rx.recv_timeout(NOKHWA_CAPTURE_TIMEOUT).map_err(|_| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_SNAPSHOT}: nokhwa capture timed out after {}ms; \
             reason={REASON_RESOURCE_UNAVAILABLE}",
            NOKHWA_CAPTURE_TIMEOUT.as_millis()
        )
    })?
}

#[cfg(not(target_os = "macos"))]
fn capture_jpeg_with_nokhwa(entry: &ResourceEntry) -> anyhow::Result<EncodedFrame> {
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{
        CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution,
    };
    use nokhwa::Camera;

    let index = entry
        .metadata
        .get("camera_index")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(0);

    let candidate_formats = [
        RequestedFormatType::Closest(CameraFormat::new(
            Resolution::new(1280, 720),
            FrameFormat::NV12,
            30,
        )),
        RequestedFormatType::Closest(CameraFormat::new(
            Resolution::new(640, 480),
            FrameFormat::NV12,
            30,
        )),
        RequestedFormatType::Closest(CameraFormat::new(
            Resolution::new(1280, 720),
            FrameFormat::MJPEG,
            30,
        )),
        RequestedFormatType::AbsoluteHighestResolution,
        RequestedFormatType::None,
    ];
    let mut last_err = None;
    let mut cam = None;
    for requested in candidate_formats {
        let req = RequestedFormat::new::<RgbFormat>(requested);
        match Camera::new(CameraIndex::Index(index), req) {
            Ok(opened) => {
                cam = Some(opened);
                break;
            }
            Err(err) => last_err = Some(format!("{requested}: {err}")),
        }
    }
    let mut cam = cam.ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_SNAPSHOT}: nokhwa Camera::new(index={index}) failed: \
                 {}; reason={REASON_RESOURCE_UNAVAILABLE}",
            last_err.unwrap_or_else(|| "no compatible format candidates".to_string())
        )
    })?;
    cam.open_stream().map_err(|e| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_SNAPSHOT}: nokhwa open_stream failed: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    std::thread::sleep(std::time::Duration::from_millis(350));
    let buf = cam.frame().map_err(|e| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_SNAPSHOT}: nokhwa frame() failed: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let rgb_image = buf.decode_image::<RgbFormat>().map_err(|e| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_SNAPSHOT}: nokhwa decode_image failed: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let width = rgb_image.width();
    let height = rgb_image.height();
    let rgb = rgb_image.into_raw();
    if rgb.iter().all(|&b| b == 0) {
        anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: camera returned an all-black frame; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }
    let mut jpeg = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut jpeg, 80);
    encoder
        .encode(
            &rgb,
            width as u16,
            height as u16,
            jpeg_encoder::ColorType::Rgb,
        )
        .map_err(|e| anyhow::anyhow!("jpeg encode failed: {e}"))?;
    Ok(EncodedFrame {
        jpeg_bytes: jpeg,
        width,
        height,
    })
}

#[cfg(not(target_os = "macos"))]
fn open_stream_with_nokhwa(
    entry: ResourceEntry,
    options: CameraStreamOptions,
) -> anyhow::Result<broadcast::Receiver<Value>> {
    let (tx, rx) = broadcast::channel::<Value>(BROADCAST_CAPACITY);
    let worker_entry = entry.clone();
    std::thread::Builder::new()
        .name("easynet-camera-nokhwa".into())
        .spawn(move || {
            let interval = Duration::from_secs_f64(1.0 / options.fps as f64);
            let seq = AtomicU64::new(0);
            loop {
                if tx.receiver_count() == 0 {
                    break;
                }
                let started = Instant::now();
                match capture_jpeg_with_nokhwa(&worker_entry) {
                    Ok(frame) => {
                        let value = build_camera_stream_frame(
                            seq.fetch_add(1, Ordering::Relaxed),
                            &worker_entry.hardware_id,
                            frame,
                        );
                        if tx.send(value).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = tx.send(json!({
                            "type": "error",
                            "message": rewrite_ability_in_message(err, ABILITY_CAMERA_SUBSCRIBE).to_string(),
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
                "{ABILITY_CAMERA_SUBSCRIBE}: failed to spawn camera worker: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
    Ok(rx)
}

fn persist_camera_snapshot(
    entry: &ResourceEntry,
    captured_at: &str,
    jpeg_bytes: &[u8],
) -> anyhow::Result<PathBuf> {
    let resource_id = entry
        .resource_ura
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown-resource");
    let safe_resource_id = safe_path_component(resource_id);
    let safe_timestamp = safe_path_component(captured_at);
    let dir = state_dir()
        .join("captures")
        .join("camera")
        .join(safe_resource_id);
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{safe_timestamp}.jpg"));
    atomic_write_with_permissions(&path, jpeg_bytes, WritePermissions::OwnerReadWrite)?;
    Ok(path)
}

fn safe_path_component(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

// ── Synthetic backend (PR3a, kept for tests) ─────────────────

/// Deterministic synthetic-frame backend. Produces a 64×48
/// solid-colour JPEG seeded from the `hardware_id` so two
/// invocations against the same resource produce byte-identical
/// frames (testable). Two distinct hardware_ids produce different
/// colours — an integration test seeing the same bytes for two
/// resources would catch a wrong-resource bug.
///
/// The colour scheme: pull three bytes from the FNV-1a hash of
/// `hardware_id` to pick (R, G, B). Cheap, deterministic, and
/// produces visibly distinct frames per resource for any human
/// debugging.
#[derive(Debug, Default)]
pub struct SyntheticBackend;

impl SnapshotBackend for SyntheticBackend {
    fn capture_jpeg(&self, entry: &ResourceEntry) -> anyhow::Result<EncodedFrame> {
        const W: u32 = 64;
        const H: u32 = 48;

        // FNV-1a 24-bit-equivalent over hardware_id → (R, G, B).
        let mut hash: u64 = 0xcbf29ce484222325;
        for b in entry.hardware_id.bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let r = (hash & 0xff) as u8;
        let g = ((hash >> 8) & 0xff) as u8;
        let b = ((hash >> 16) & 0xff) as u8;

        // Solid-colour RGB buffer, then JPEG-encode at quality 80.
        // Quality 80 is the standard "good enough" balance — the
        // synthetic content is uniform so even quality 50 would
        // compress identically; 80 keeps us in line with what a
        // real camera frame would use.
        let mut rgb = Vec::with_capacity((W * H * 3) as usize);
        for _ in 0..(W * H) {
            rgb.push(r);
            rgb.push(g);
            rgb.push(b);
        }

        let mut jpeg = Vec::new();
        let encoder = jpeg_encoder::Encoder::new(&mut jpeg, 80);
        encoder
            .encode(&rgb, W as u16, H as u16, jpeg_encoder::ColorType::Rgb)
            .map_err(|e| anyhow::anyhow!("jpeg encode failed: {e}"))?;

        Ok(EncodedFrame {
            jpeg_bytes: jpeg,
            width: W,
            height: H,
        })
    }

    fn open_stream(
        &self,
        entry: ResourceEntry,
        _options: CameraStreamOptions,
    ) -> anyhow::Result<broadcast::Receiver<Value>> {
        let (tx, rx) = broadcast::channel::<Value>(BROADCAST_CAPACITY);
        let frame = self.capture_jpeg(&entry)?;
        let _ = tx.send(build_camera_stream_frame(0, &entry.hardware_id, frame));
        let keepalive_tx = tx.clone();
        std::thread::Builder::new()
            .name("easynet-camera-synthetic".into())
            .spawn(move || {
                while keepalive_tx.receiver_count() > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            })
            .map_err(|e| {
                anyhow::anyhow!(
                    "{ABILITY_CAMERA_SUBSCRIBE}: failed to spawn synthetic camera worker: {e}; \
                     reason={REASON_RESOURCE_UNAVAILABLE}"
                )
            })?;
        Ok(rx)
    }
}

// ── Registration ─────────────────────────────────────────────

/// Register `camera.snapshot` and `camera.subscribe` with real
/// envelope-aware handlers backed by `backend`.
///
/// `media_abilities::register` deliberately skips the camera names
/// once this real module exists, so each dispatch slot has one
/// handler family. `camera.subscribe` opens a realtime in-memory
/// preview stream; it never persists the preview frames.
pub fn register_with_backend(reg: &mut AxonAbilityCatalog, backend: Arc<dyn SnapshotBackend>) {
    let subscribe_backend = Arc::clone(&backend);
    reg.register_rpc_with_envelope_and_owner(
        ABILITY_CAMERA_SNAPSHOT,
        OwnerKind::Device,
        Arc::new(move |env: EnvelopeContext, args: Value| snapshot_handler(&backend, env, args)),
    );
    reg.register_stream_with_envelope_and_owner(
        ABILITY_CAMERA_SUBSCRIBE,
        OwnerKind::Device,
        Arc::new(move |env: EnvelopeContext, args: Value| {
            subscribe_handler(&subscribe_backend, env, args)
        }),
    );
}

#[cfg(not(target_os = "macos"))]
fn rewrite_ability_in_message(err: anyhow::Error, ability: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{}",
        err.to_string()
            .replacen(ABILITY_CAMERA_SNAPSHOT, ability, 1)
    )
}

/// Register with the real `NokhwaBackend` (PR3 default). The
/// daemon boot path calls this after `media_abilities::register`,
/// which now skips camera names to keep registration ownership
/// explicit.
/// Tests that need a hardware-free path call
/// `register_with_backend(reg, Arc::new(SyntheticBackend))`
/// instead — the trait keeps the dispatch / receipt code
/// backend-agnostic.
pub fn register(reg: &mut AxonAbilityCatalog) {
    register_with_backend(reg, Arc::new(NokhwaBackend));
}

// ── Handler core ─────────────────────────────────────────────

fn snapshot_handler(
    backend: &Arc<dyn SnapshotBackend>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let entry = resolve_camera_subject(&env, &args, ABILITY_CAMERA_SNAPSHOT)?;

    // Capture + encode.
    let EncodedFrame {
        jpeg_bytes,
        width,
        height,
    } = backend.capture_jpeg(&entry)?;

    if jpeg_bytes.len() > MAX_INLINE_BYTES {
        anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: encoded image {} bytes exceeds \
             inline cap {MAX_INLINE_BYTES}; payloadstore path not yet \
             wired in PR3a; reason={REASON_IMAGE_TOO_LARGE}",
            jpeg_bytes.len()
        );
    }

    let captured_at = chrono::Utc::now().to_rfc3339();
    let local_path = persist_camera_snapshot(&entry, &captured_at, &jpeg_bytes)?;
    // Context-surface persistence (best-effort): browsable in the
    // Context page as <device>/camera.snapshot/<artifact>. The
    // legacy captures/camera tree above stays — it is keyed by
    // resource and consumed by the CLI; this one feeds the UI index.
    if let Err(err) = crate::persistence::context_store::record_capture(
        crate::persistence::context_store::CaptureRecord {
            device: env.callee(),
            ability: ABILITY_CAMERA_SNAPSHOT,
            ext: "jpg",
            bytes: &jpeg_bytes,
            content_type: "image/jpeg",
            width: Some(width),
            height: Some(height),
            duration_ms: None,
            preview: format!("Photo {width}x{height}"),
        },
    ) {
        crate::op_event!(
            component = context,
            kind = capture_persist_failed,
            level = "warn",
            ability = ABILITY_CAMERA_SNAPSHOT,
            error = err,
        );
    }
    let image_bytes_b64 = BASE64_STANDARD.encode(&jpeg_bytes);

    Ok(json!({
        "image_bytes_b64": image_bytes_b64,
        "content_type":    "image/jpeg",
        "width":           width,
        "height":          height,
        "byte_size":       jpeg_bytes.len(),
        "captured_at":     captured_at,
        "local_path":      local_path.display().to_string(),
        // hardware_id surfaces here (NOT in meta.list_resources's
        // wire shape, which keeps it audit-only) because a snapshot
        // receipt is the natural place to record "which physical
        // device produced this byte stream" for the auditor.
        "hardware_id":     entry.hardware_id,
    }))
}

fn subscribe_handler(
    backend: &Arc<dyn SnapshotBackend>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<StreamSource> {
    let entry = resolve_camera_subject(&env, &args, ABILITY_CAMERA_SUBSCRIBE)?;
    let options = parse_stream_options(&args)?;
    let rx = backend.open_stream(entry, options)?;
    Ok(StreamSource::Live(rx))
}

fn resolve_camera_subject(
    env: &EnvelopeContext,
    args: &Value,
    ability: &'static str,
) -> anyhow::Result<ResourceEntry> {
    resolve_required_resource_subject(
        env,
        args,
        ResourceSubjectSpec {
            ability,
            required_subject: "a camera",
            allowed_kinds: &[ResourceType::Camera],
            allowed_label: "a camera",
        },
    )
}

pub(crate) fn build_camera_stream_frame(seq: u64, hardware_id: &str, frame: EncodedFrame) -> Value {
    let image_bytes_b64 = BASE64_STANDARD.encode(&frame.jpeg_bytes);
    json!({
        "seq":             seq,
        "preview":         true,
        "source_ability":  ABILITY_CAMERA_SUBSCRIBE,
        "content_type":    "image/jpeg",
        "width":           frame.width,
        "height":          frame.height,
        "byte_size":       frame.jpeg_bytes.len(),
        "captured_at":     chrono::Utc::now().to_rfc3339(),
        "image_bytes_b64": image_bytes_b64,
        "hardware_id":     hardware_id,
    })
}

fn parse_stream_options(args: &Value) -> anyhow::Result<CameraStreamOptions> {
    let mut options = CameraStreamOptions::default();
    if let Value::Object(map) = args {
        if let Some(value) = map.get("fps") {
            let fps = value.as_u64().ok_or_else(|| {
                anyhow::anyhow!(
                    "{ABILITY_CAMERA_SUBSCRIBE}: fps must be an integer; \
                     reason=invalid_argument"
                )
            })?;
            if !(MIN_CAMERA_FPS as u64..=MAX_CAMERA_FPS as u64).contains(&fps) {
                anyhow::bail!(
                    "{ABILITY_CAMERA_SUBSCRIBE}: fps {fps} outside {MIN_CAMERA_FPS}..={MAX_CAMERA_FPS}; \
                     reason=invalid_argument"
                );
            }
            options.fps = fps as u32;
        }
        if let Some(value) = map.get("resolution") {
            options.resolution = parse_resolution(value)?;
        }
    }
    Ok(options)
}

fn parse_resolution(value: &Value) -> anyhow::Result<Option<CameraVideoResolution>> {
    let Some(raw) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
        anyhow::bail!(
            "{ABILITY_CAMERA_SUBSCRIBE}: resolution must be a string; \
             reason=invalid_argument"
        );
    };
    if raw.eq_ignore_ascii_case("native") {
        return Ok(None);
    }
    let lowered = raw.to_ascii_lowercase();
    let resolution = match lowered.as_str() {
        "480p" => CameraVideoResolution {
            width: 640,
            height: 480,
        },
        "720p" => CameraVideoResolution {
            width: 1280,
            height: 720,
        },
        "1080p" => CameraVideoResolution {
            width: 1920,
            height: 1080,
        },
        _ => {
            let Some((w, h)) = lowered.split_once('x') else {
                anyhow::bail!(
                    "{ABILITY_CAMERA_SUBSCRIBE}: resolution {raw:?} must be native, 480p, 720p, 1080p, or <width>x<height>; \
                     reason=invalid_argument"
                );
            };
            CameraVideoResolution {
                width: parse_positive_u32(w, "resolution width")?,
                height: parse_positive_u32(h, "resolution height")?,
            }
        }
    };
    Ok(Some(resolution))
}

fn parse_positive_u32(raw: &str, name: &str) -> anyhow::Result<u32> {
    let value = raw.parse::<u32>().map_err(|_| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_SUBSCRIBE}: {name} must be an integer; \
             reason=invalid_argument"
        )
    })?;
    if value == 0 {
        anyhow::bail!(
            "{ABILITY_CAMERA_SUBSCRIBE}: {name} must be positive; \
             reason=invalid_argument"
        );
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::resources::{
        self, upsert_resource, ResourceBinding, ResourceUpsert, ResourcesFile,
    };
    use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};

    /// Build a one-resource ResourcesFile and return its URA. The
    /// caller passes the file to `lookup_by_ura` via the on-disk
    /// path; tests that need to round-trip through the real handler
    /// must use HomeGuard so `resources::load` reads the right path.
    fn seed_camera(file: &mut ResourcesFile, hardware_id: &str) -> String {
        upsert_resource(
            file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/device/01DEV",
                kind: ResourceType::Camera,
                binding: ResourceBinding::LocalDevice,
                hardware_id,
                display_name: "Test Camera",
                metadata: json!({}),
            },
        )
    }

    /// Test register helper. Tests use SyntheticBackend so the
    /// suite runs hardware-free (CI / Linux-without-camera). The
    /// daemon's `register(reg)` defaults to `NokhwaBackend` which
    /// only works against a real `/dev/video*` or AVFoundation
    /// device.
    fn register_synthetic(reg: &mut AxonAbilityCatalog) {
        register_with_backend(reg, Arc::new(SyntheticBackend));
    }

    /// Synthetic-backend smoke: capture against a hand-built
    /// ResourceEntry, assert the bytes look like a JPEG (magic
    /// number 0xff 0xd8) and the dimensions match.
    #[test]
    fn synthetic_backend_emits_valid_jpeg_with_expected_dimensions() {
        let mut file = ResourcesFile::default();
        seed_camera(&mut file, "h-cam-test-1");
        let entry = file.resources[0].clone();
        let backend = SyntheticBackend;
        let frame = backend.capture_jpeg(&entry).unwrap();
        assert_eq!(frame.width, 64);
        assert_eq!(frame.height, 48);
        assert!(frame.jpeg_bytes.len() > 200, "tiny JPEG implausibly short");
        // SOI marker (Start of Image) — every JPEG starts with these
        // two bytes. If the encoder was misconfigured (e.g. raw RGB
        // instead of JPEG) this would fail.
        assert_eq!(&frame.jpeg_bytes[..2], &[0xff, 0xd8]);
        // EOI marker (End of Image) — last two bytes of every JPEG.
        assert_eq!(
            &frame.jpeg_bytes[frame.jpeg_bytes.len() - 2..],
            &[0xff, 0xd9]
        );
    }

    /// Two distinct hardware_ids produce distinct JPEG byte
    /// streams. Pin so a future "optimisation" that reused a
    /// cached buffer across resources would surface here as a
    /// wrong-camera attribution bug.
    #[test]
    fn synthetic_backend_emits_distinct_bytes_per_hardware_id() {
        let mut file = ResourcesFile::default();
        seed_camera(&mut file, "h-cam-A");
        seed_camera(&mut file, "h-cam-B");
        let backend = SyntheticBackend;
        let frame_a = backend.capture_jpeg(&file.resources[0]).unwrap();
        let frame_b = backend.capture_jpeg(&file.resources[1]).unwrap();
        assert_ne!(
            frame_a.jpeg_bytes, frame_b.jpeg_bytes,
            "different hardware_ids must produce different bytes"
        );
    }

    /// Same hardware_id → byte-identical output across calls.
    /// Determinism is the test fixture's load-bearing property.
    #[test]
    fn synthetic_backend_is_deterministic_per_hardware_id() {
        let mut file = ResourcesFile::default();
        seed_camera(&mut file, "h-cam-deterministic");
        let entry = file.resources[0].clone();
        let backend = SyntheticBackend;
        let one = backend.capture_jpeg(&entry).unwrap();
        let two = backend.capture_jpeg(&entry).unwrap();
        assert_eq!(one.jpeg_bytes, two.jpeg_bytes);
    }

    /// End-to-end: register with synthetic backend, invoke through
    /// the real dispatcher, assert receipt body shape + base64
    /// decodes back to the same JPEG the backend produced.
    #[test]
    fn handler_returns_receipt_with_base64_jpeg_when_subject_resolves() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        // Seed the on-disk resources.json so the handler's
        // `resources::load` finds it.
        let mut file = ResourcesFile::default();
        let ura = seed_camera(&mut file, "h-cam-e2e");
        resources::save(&file).unwrap();

        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_CAMERA_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some(ura),
            causal_context: None,
        };
        let resp = dispatcher.execute_rpc(target).unwrap();

        // Required receipt fields (matches the docstring contract).
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
        assert_eq!(resp["width"], 64);
        assert_eq!(resp["height"], 48);
        assert_eq!(resp["hardware_id"], "h-cam-e2e");

        // base64 round-trip back to the same bytes the synthetic
        // backend would have produced. Catches a malformed encoder
        // or a wrong-buffer bug.
        let b64 = resp["image_bytes_b64"].as_str().unwrap();
        let decoded = BASE64_STANDARD.decode(b64).unwrap();
        assert_eq!(decoded[..2], [0xff, 0xd8]); // JPEG SOI
        assert_eq!(decoded[decoded.len() - 2..], [0xff, 0xd9]); // EOI
        assert_eq!(decoded.len(), resp["byte_size"].as_u64().unwrap() as usize);
    }

    #[test]
    fn camera_subscribe_returns_live_preview_stream() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_camera(&mut file, "h-cam-preview");
        resources::save(&file).unwrap();

        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_CAMERA_SUBSCRIBE.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Stream,
            subject: Some(ura),
            causal_context: None,
        };
        let source = dispatcher.execute_stream(target).unwrap();
        let (snapshot, mut rx) = match source {
            StreamSource::Live(rx) => (Vec::new(), rx),
            StreamSource::SnapshotThenLive(snapshot, rx) => (snapshot, rx),
            other => panic!("camera.subscribe must return live stream, got {other:?}"),
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let resp = match snapshot.first() {
            Some(frame) => frame.clone(),
            None => rt.block_on(async { rx.recv().await.unwrap() }),
        };

        assert_eq!(resp["preview"], true);
        assert_eq!(resp["source_ability"], ABILITY_CAMERA_SUBSCRIBE);
        assert_eq!(resp["content_type"], "image/jpeg");
        assert_eq!(resp["width"], 64);
        assert_eq!(resp["height"], 48);
    }

    #[test]
    fn camera_subscribe_stream_preview_errors_name_subscribe_ability() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_CAMERA_SUBSCRIBE.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Stream,
            subject: None,
            causal_context: None,
        };
        let err = dispatcher.execute_stream(target).unwrap_err().to_string();

        assert!(
            err.contains(ABILITY_CAMERA_SUBSCRIBE),
            "subscribe preview error should name subscribe ability: {err}"
        );
        assert!(
            !err.contains(ABILITY_CAMERA_SNAPSHOT),
            "subscribe preview error must not look like snapshot dispatch: {err}"
        );
    }

    /// INV-SUBJECT-ENVELOPE positive half: invocation with no
    /// envelope subject (`subject: None`) MUST fail with
    /// reason="subject_required". Without this the handler would
    /// either crash or silently capture from "the first camera",
    /// either of which makes auditing a lie.
    #[test]
    fn handler_rejects_missing_subject_with_subject_required_reason() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_CAMERA_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: None, // no envelope subject
            causal_context: None,
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(
            err.to_string().contains(REASON_SUBJECT_REQUIRED),
            "expected reason={REASON_SUBJECT_REQUIRED}; got: {err}"
        );
    }

    /// INV-RESOURCE-VALIDITY: subject points at a URA not in the
    /// local table → reason="resource_not_found". Distinct from
    /// "URA present but device unplugged" (resource_unavailable —
    /// not testable in PR3a's synthetic backend; covered when the
    /// real backend lands).
    #[test]
    fn handler_rejects_unknown_subject_with_resource_not_found_reason() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        // Save an empty resources.json so load() returns Default
        // rather than picking up some prior test's state.
        resources::save(&ResourcesFile::default()).unwrap();
        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_CAMERA_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some("easynet:///r/acme/resource/01NEVER-EXISTED".into()),
            causal_context: None,
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(
            err.to_string().contains(REASON_RESOURCE_NOT_FOUND),
            "expected reason={REASON_RESOURCE_NOT_FOUND}; got: {err}"
        );
    }

    /// Caller asks `camera.snapshot` against a `mic` resource_ura
    /// → reason="resource_type_mismatch". A real-world UX bug that
    /// would otherwise produce a confusing later error from the
    /// camera backend; catching at the handler edge is much clearer.
    #[test]
    fn handler_rejects_wrong_type_subject_with_resource_type_mismatch_reason() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let mic_ura = upsert_resource(
            &mut file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/device/01DEV",
                kind: ResourceType::Mic, // not a camera
                binding: ResourceBinding::LocalDevice,
                hardware_id: "h-mic-not-camera",
                display_name: "Not A Camera",
                metadata: json!({}),
            },
        );
        resources::save(&file).unwrap();

        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_CAMERA_SNAPSHOT.to_string(),
            normalized_args: json!({}),
            call_mode: CallMode::Rpc,
            subject: Some(mic_ura),
            causal_context: None,
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(
            err.to_string().contains(REASON_RESOURCE_TYPE_MISMATCH),
            "expected reason={REASON_RESOURCE_TYPE_MISMATCH}; got: {err}"
        );
    }

    /// INV-SUBJECT-ENVELOPE negative half (defence in depth): if
    /// args carry a `subject` key the handler MUST reject before
    /// any other parsing, even on the envelope-aware path. The
    /// `media_abilities` stubs already enforce this for the
    /// args-only path; this test pins the env-aware sibling.
    #[test]
    fn handler_rejects_subject_in_args_even_on_envelope_path() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let mut reg = AxonAbilityCatalog::new();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = InvocationTarget {
            scope: TargetScope::Local,
            ability: ABILITY_CAMERA_SNAPSHOT.to_string(),
            normalized_args: json!({"subject": "easynet:///r/x/resource/y"}),
            call_mode: CallMode::Rpc,
            subject: Some("easynet:///r/acme/resource/01CAM".into()),
            causal_context: None,
        };
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(
            err.to_string().contains(REASON_SUBJECT_IN_ARGS),
            "expected reason={REASON_SUBJECT_IN_ARGS}; got: {err}"
        );
    }
}
