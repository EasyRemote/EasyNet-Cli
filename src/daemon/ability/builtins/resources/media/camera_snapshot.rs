// EasyNet CLI - camera media handlers (RFC-005 v3.2 A3/A4)
// =========================================================
//
// File: src/daemon/ability/builtins/resources/media/camera_snapshot.rs
//
// Binds the metadata-only media capability contracts for `camera.snapshot`,
// `camera.subscribe`, `camera.record_start`, and `camera.record_stop` to
// envelope-aware real handlers that:
//
//   1. Reads `EnvelopeContext.subject` (per **INV-SUBJECT-ENVELOPE**:
//      handler MUST get subject from the envelope, NOT from args).
//   2. Resolves subject → `ResourceEntry` via
//      `persistence::resources::lookup_by_ura`.
//   3. Branches per **INV-RESOURCE-VALIDITY**:
//        * subject absent     → InvalidArgument with reason="subject_required"
//        * URA not in table   → terminal failure with reason="resource_not_found"
//        * type ≠ camera      → terminal failure with reason="resource_type_mismatch"
//        * resource present but unavailable → "resource_unavailable"
//   4. Captures a still photo via the configured backend. Production
//      uses AVCapturePhotoOutput on macOS and nokhwa on non-macOS.
//      Tests use `SyntheticBackend` for hardware-free runs.
//   5. Returns the established JSON snapshot receipt with inline base64 JPEG.
//      The high-frequency preview path is binary; this unary compatibility
//      shape remains until a separately versioned raw-unary ABI exists.
//   7. Opens a latest-frame JPEG lane for `camera.subscribe`. Frames admitted
//      to the public stream remain typed `image/jpeg` bytes; Runtime owns
//      sequence and terminal metadata. Preview frames are transient.
//   8. Drives bounded `camera.record_start`/`camera.record_stop`
//      sessions and persists the finalized capture through the context
//      store. macOS pushes native camera sample buffers directly into
//      AVAssetWriter (H.264 MOV); the portable/test engine emits multipart
//      MJPEG.
//
// What's not in this module
// -------------------------
// * PayloadStore overflow path for >2 MiB images. Returns a
//   clear "image too large for inline" error rather than silently
//   truncating an Axon frame.
// * Alternate image formats. Snapshot and subscribe both emit JPEG.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use tokio::sync::{mpsc as tokio_mpsc, watch};

use crate::daemon::ability::builtins::resources::media::resource_subject::{
    self, resolve_required_resource_subject, ResourceSubjectSpec,
};
use crate::daemon::ability::builtins::resources::media::{
    self, ABILITY_CAMERA_RECORD_START, ABILITY_CAMERA_RECORD_STOP, ABILITY_CAMERA_SNAPSHOT,
    ABILITY_CAMERA_SUBSCRIBE,
};
use crate::daemon::ability::dispatch::OwnerKind;
use crate::daemon::ability::dispatch::{
    AxonAbilityCatalog, EnvelopeContext, StreamOutputFrame, StreamSource,
};
use crate::daemon::persistence::config::state_dir;
use crate::daemon::persistence::resources::{ResourceEntry, ResourceType};
use crate::daemon::resources::context::device_scope::ContextDeviceScope;

/// Maximum inline image size, in encoded JPEG bytes (NOT the base64
/// expansion). Above this the handler refuses with an explicit
/// "use payloadstore" error rather than risking an oversized Axon
/// frame. 2 MiB keeps the base64-expanded body below the 4 MiB IPC
/// frame limit while allowing normal laptop camera frames through.
const MAX_INLINE_BYTES: usize = 2 * 1024 * 1024;
const PREVIEW_OUTPUT_CAPACITY: usize = 1;
const DEFAULT_CAMERA_FPS: u32 = 30;
const MIN_CAMERA_FPS: u32 = 1;
const MAX_CAMERA_FPS: u32 = 60;
const DEFAULT_RECORDING_MAX_DURATION_MS: u64 = 5 * 60 * 1000;
const MAX_RECORDING_MAX_DURATION_MS: u64 = 30 * 60 * 1000;
const DEFAULT_RECORDING_MAX_BYTES: u64 = 128 * 1024 * 1024;
const MAX_RECORDING_MAX_BYTES: u64 = 256 * 1024 * 1024;
const RECORDING_START_TIMEOUT: Duration = Duration::from_secs(10);
// Must exceed the native backend's bounded container-finalization window.
// A successful stop receipt means the movie is closed and durable, not merely
// that stop was requested.
const RECORDING_STOP_TIMEOUT: Duration = Duration::from_secs(15);
const RECORDING_BOUNDARY: &str = "easynet-camera-frame";
const MJPEG_CONTENT_TYPE: &str = "multipart/x-mixed-replace; boundary=easynet-camera-frame";
#[cfg(all(feature = "native-media", not(target_os = "macos")))]
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

/// One-shot and live camera capture backend. Production registers
/// `NokhwaBackend`; tests register `SyntheticBackend` so dispatcher
/// and receipt semantics are validated without hardware.
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
    ) -> anyhow::Result<CameraFrameStream>;
}

#[derive(Debug, Clone)]
pub struct EncodedFrame {
    pub jpeg_bytes: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
}

impl EncodedFrame {
    pub(crate) fn new(jpeg_bytes: Vec<u8>, width: u32, height: u32) -> Self {
        Self {
            jpeg_bytes: jpeg_bytes.into(),
            width,
            height,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum CameraFrameUpdate {
    Pending,
    Frame(EncodedFrame),
    Failed(Arc<str>),
}

#[derive(Debug)]
pub struct CameraFrameStream {
    receiver: watch::Receiver<CameraFrameUpdate>,
}

impl CameraFrameStream {
    fn blocking_next(&mut self) -> Option<CameraFrameUpdate> {
        futures::executor::block_on(self.receiver.changed()).ok()?;
        Some(self.receiver.borrow_and_update().clone())
    }

    pub(super) fn try_next(&mut self) -> CameraFramePoll {
        match self.receiver.has_changed() {
            Ok(true) => match self.receiver.borrow_and_update().clone() {
                CameraFrameUpdate::Pending => CameraFramePoll::Pending,
                CameraFrameUpdate::Frame(frame) => CameraFramePoll::Frame(frame),
                CameraFrameUpdate::Failed(message) => CameraFramePoll::Failed(message),
            },
            Ok(false) => CameraFramePoll::Pending,
            Err(_) => CameraFramePoll::Closed,
        }
    }
}

pub(super) enum CameraFramePoll {
    Pending,
    Frame(EncodedFrame),
    Failed(Arc<str>),
    Closed,
}

pub(crate) type CameraFrameSender = watch::Sender<CameraFrameUpdate>;

pub(crate) fn camera_frame_channel() -> (CameraFrameSender, CameraFrameStream) {
    let (sender, receiver) = watch::channel(CameraFrameUpdate::Pending);
    (sender, CameraFrameStream { receiver })
}

pub(crate) fn publish_camera_frame(sender: &CameraFrameSender, encoded: EncodedFrame) {
    sender.send_replace(CameraFrameUpdate::Frame(encoded));
}

pub(crate) fn publish_camera_failure(sender: &CameraFrameSender, error: impl ToString) {
    sender.send_replace(CameraFrameUpdate::Failed(Arc::from(error.to_string())));
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

#[derive(Debug)]
struct ActiveCameraStream {
    options: CameraStreamOptions,
    sender: CameraFrameSender,
}

#[derive(Debug, Default)]
struct ActiveCameraStreams {
    streams: Mutex<HashMap<String, ActiveCameraStream>>,
}

enum CameraStreamLease {
    Existing(CameraFrameStream),
    Producer {
        sender: CameraFrameSender,
        stream: CameraFrameStream,
    },
}

impl ActiveCameraStreams {
    fn acquire(&self, entry: &ResourceEntry, options: &CameraStreamOptions) -> CameraStreamLease {
        let mut streams = self
            .streams
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(active) = streams.get(&entry.resource_ura) {
            if active.options == *options && active.sender.receiver_count() > 0 {
                let mut receiver = active.sender.subscribe();
                // A newly joined preview or recorder needs the current latest
                // frame immediately; it must not wait for the next camera tick.
                receiver.mark_changed();
                return CameraStreamLease::Existing(CameraFrameStream { receiver });
            }
        }

        let (sender, stream) = camera_frame_channel();
        streams.insert(
            entry.resource_ura.clone(),
            ActiveCameraStream {
                options: options.clone(),
                sender: sender.clone(),
            },
        );
        CameraStreamLease::Producer { sender, stream }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraVideoResolution {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct CameraRecordingOptions {
    pub(crate) stream: CameraStreamOptions,
    pub(crate) codec: Option<String>,
    pub(crate) max_duration_ms: u64,
    pub(crate) max_bytes: u64,
}

trait CameraRecordingEngine: Send + Sync {
    fn content_type(&self) -> &'static str;
    fn validate_options(&self, options: &CameraRecordingOptions) -> anyhow::Result<()>;
    fn record(
        &self,
        entry: ResourceEntry,
        options: CameraRecordingOptions,
        stop: Arc<AtomicBool>,
        session_id: &str,
        ready: mpsc::Sender<anyhow::Result<()>>,
    ) -> anyhow::Result<CameraRecordingArtifact>;
}

struct MjpegRecordingEngine {
    backend: Arc<dyn SnapshotBackend>,
}

impl CameraRecordingEngine for MjpegRecordingEngine {
    fn content_type(&self) -> &'static str {
        MJPEG_CONTENT_TYPE
    }

    fn validate_options(&self, options: &CameraRecordingOptions) -> anyhow::Result<()> {
        if options
            .codec
            .as_deref()
            .is_some_and(|codec| codec != "mjpeg")
        {
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_START}: this platform recording engine requires codec=\"mjpeg\"; \
                 reason=invalid_argument"
            );
        }
        Ok(())
    }

    fn record(
        &self,
        entry: ResourceEntry,
        options: CameraRecordingOptions,
        stop: Arc<AtomicBool>,
        session_id: &str,
        ready: mpsc::Sender<anyhow::Result<()>>,
    ) -> anyhow::Result<CameraRecordingArtifact> {
        run_mjpeg_recording_worker(
            Arc::clone(&self.backend),
            entry,
            options,
            stop,
            recording_temp_path(session_id, "mjpeg")?,
            ready,
        )
    }
}

#[cfg(target_os = "macos")]
#[derive(Debug, Default)]
struct AvFoundationRecordingEngine;

#[cfg(target_os = "macos")]
impl CameraRecordingEngine for AvFoundationRecordingEngine {
    fn content_type(&self) -> &'static str {
        "video/quicktime"
    }

    fn validate_options(&self, options: &CameraRecordingOptions) -> anyhow::Result<()> {
        if options
            .codec
            .as_deref()
            .is_some_and(|codec| codec != "h264")
        {
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_START}: macOS native recording requires codec=\"h264\"; \
                 reason=invalid_argument"
            );
        }
        Ok(())
    }

    fn record(
        &self,
        entry: ResourceEntry,
        options: CameraRecordingOptions,
        stop: Arc<AtomicBool>,
        session_id: &str,
        ready: mpsc::Sender<anyhow::Result<()>>,
    ) -> anyhow::Result<CameraRecordingArtifact> {
        super::avfoundation_camera::record_movie(
            entry,
            options,
            stop,
            recording_temp_path(session_id, "mov")?,
            ready,
        )
    }
}

#[derive(Debug)]
struct CameraRecordingSession {
    id: String,
    device_ura: String,
    resource_ura: String,
    hardware_id: String,
    started_at: String,
    stop: Arc<AtomicBool>,
    done: Option<Receiver<anyhow::Result<CameraRecordingArtifact>>>,
}

#[derive(Debug)]
struct CameraRecordingStopLease {
    id: String,
    device_ura: String,
    resource_ura: String,
    hardware_id: String,
    started_at: String,
    done: Receiver<anyhow::Result<CameraRecordingArtifact>>,
}

#[derive(Debug)]
pub(crate) struct CameraRecordingArtifact {
    pub(crate) temp_path: PathBuf,
    pub(crate) extension: &'static str,
    pub(crate) content_type: &'static str,
    pub(crate) stopped_at: String,
    pub(crate) duration_ms: u64,
    pub(crate) frame_count: Option<u64>,
    pub(crate) byte_size: u64,
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) stop_reason: &'static str,
}

static CAMERA_RECORDING_SESSIONS: OnceLock<Mutex<HashMap<String, CameraRecordingSession>>> =
    OnceLock::new();

fn recording_sessions() -> &'static Mutex<HashMap<String, CameraRecordingSession>> {
    CAMERA_RECORDING_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

// ── Production camera backend ────────────────────────────────

/// Real camera backend. On macOS, still photos and live preview use
/// direct AVFoundation bindings. On non-macOS, nokhwa opens the
/// platform camera backend, decodes to RGB, and re-encodes as JPEG
/// to match the receipt body shape camera.snapshot promises.
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
#[cfg(feature = "native-media")]
pub struct NokhwaBackend {
    active_streams: ActiveCameraStreams,
}

#[cfg(feature = "native-media")]
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
    ) -> anyhow::Result<CameraFrameStream> {
        let (sender, stream) = match self.active_streams.acquire(&entry, &options) {
            CameraStreamLease::Existing(stream) => return Ok(stream),
            CameraStreamLease::Producer { sender, stream } => (sender, stream),
        };
        #[cfg(target_os = "macos")]
        {
            super::avfoundation_camera::open_jpeg_stream(entry, options, sender)?;
        }

        #[cfg(not(target_os = "macos"))]
        open_stream_with_nokhwa(entry, options, sender)?;

        Ok(stream)
    }
}

#[cfg(all(feature = "native-media", not(target_os = "macos")))]
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

#[cfg(all(feature = "native-media", not(target_os = "macos")))]
fn capture_jpeg_with_nokhwa(entry: &ResourceEntry) -> anyhow::Result<EncodedFrame> {
    let mut cam =
        open_camera_with_nokhwa(entry, ABILITY_CAMERA_SNAPSHOT, None, DEFAULT_CAMERA_FPS)?;
    open_nokhwa_stream(&mut cam, ABILITY_CAMERA_SNAPSHOT)?;
    // The first frame after opening a UVC/V4L2 camera is commonly a stale
    // driver buffer. Consume exactly one frame instead of sleeping a fixed
    // 350 ms: `frame()` already blocks for device readiness, so this preserves
    // warm-up without adding unconditional wall-clock latency.
    let _ = cam.frame().map_err(|e| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_SNAPSHOT}: nokhwa warm-up frame failed: {e}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    capture_open_nokhwa_frame(&mut cam, ABILITY_CAMERA_SNAPSHOT, true)
}

#[cfg(all(feature = "native-media", not(target_os = "macos")))]
fn open_camera_with_nokhwa(
    entry: &ResourceEntry,
    ability: &'static str,
    resolution: Option<CameraVideoResolution>,
    fps: u32,
) -> anyhow::Result<nokhwa::Camera> {
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
    let preferred = resolution.unwrap_or(CameraVideoResolution {
        width: 1280,
        height: 720,
    });
    let fps = fps.clamp(MIN_CAMERA_FPS, MAX_CAMERA_FPS);

    let candidate_formats = [
        RequestedFormatType::Closest(CameraFormat::new(
            Resolution::new(preferred.width, preferred.height),
            FrameFormat::NV12,
            fps,
        )),
        RequestedFormatType::Closest(CameraFormat::new(
            Resolution::new(640, 480),
            FrameFormat::NV12,
            fps,
        )),
        RequestedFormatType::Closest(CameraFormat::new(
            Resolution::new(preferred.width, preferred.height),
            FrameFormat::MJPEG,
            fps,
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
    let cam = cam.ok_or_else(|| {
        anyhow::anyhow!(
            "{ability}: nokhwa Camera::new(index={index}) failed: \
                 {}; reason={REASON_RESOURCE_UNAVAILABLE}",
            last_err.unwrap_or_else(|| "no compatible format candidates".to_string())
        )
    })?;
    Ok(cam)
}

#[cfg(all(feature = "native-media", not(target_os = "macos")))]
fn open_nokhwa_stream(cam: &mut nokhwa::Camera, ability: &'static str) -> anyhow::Result<()> {
    cam.open_stream().map_err(|e| {
        anyhow::anyhow!(
            "{ability}: nokhwa open_stream failed: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })
}

#[cfg(all(feature = "native-media", not(target_os = "macos")))]
fn capture_open_nokhwa_frame(
    cam: &mut nokhwa::Camera,
    ability: &'static str,
    reject_all_black: bool,
) -> anyhow::Result<EncodedFrame> {
    use nokhwa::pixel_format::RgbFormat;

    let buf = cam.frame().map_err(|e| {
        anyhow::anyhow!(
            "{ability}: nokhwa frame() failed: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let rgb_image = buf.decode_image::<RgbFormat>().map_err(|e| {
        anyhow::anyhow!(
            "{ability}: nokhwa decode_image failed: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let width = rgb_image.width();
    let height = rgb_image.height();
    let rgb = rgb_image.into_raw();
    if reject_all_black && rgb.iter().all(|&b| b == 0) {
        anyhow::bail!(
            "{ability}: camera returned an all-black frame; \
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
    Ok(EncodedFrame::new(jpeg, width, height))
}

#[cfg(all(feature = "native-media", not(target_os = "macos")))]
fn open_stream_with_nokhwa(
    entry: ResourceEntry,
    options: CameraStreamOptions,
    tx: CameraFrameSender,
) -> anyhow::Result<()> {
    let worker_entry = entry.clone();
    std::thread::Builder::new()
        .name("easynet-camera-nokhwa".into())
        .spawn(move || {
            let interval = Duration::from_secs_f64(1.0 / options.fps as f64);
            let mut cam = match open_camera_with_nokhwa(
                &worker_entry,
                ABILITY_CAMERA_SUBSCRIBE,
                options.resolution,
                options.fps,
            ) {
                Ok(cam) => cam,
                Err(err) => {
                    publish_camera_failure(&tx, err);
                    return;
                }
            };
            if let Err(err) = open_nokhwa_stream(&mut cam, ABILITY_CAMERA_SUBSCRIBE) {
                publish_camera_failure(&tx, err);
                return;
            }
            loop {
                if tx.receiver_count() == 0 {
                    break;
                }
                let started = Instant::now();
                match capture_open_nokhwa_frame(&mut cam, ABILITY_CAMERA_SUBSCRIBE, false) {
                    Ok(frame) => {
                        publish_camera_frame(&tx, frame);
                    }
                    Err(err) => {
                        publish_camera_failure(&tx, err);
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
    Ok(())
}

// ── Synthetic backend (tests) ────────────────────────────────

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
pub struct SyntheticBackend {
    active_streams: ActiveCameraStreams,
}

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

        Ok(EncodedFrame::new(jpeg, W, H))
    }

    fn open_stream(
        &self,
        entry: ResourceEntry,
        options: CameraStreamOptions,
    ) -> anyhow::Result<CameraFrameStream> {
        let (tx, rx) = match self.active_streams.acquire(&entry, &options) {
            CameraStreamLease::Existing(stream) => return Ok(stream),
            CameraStreamLease::Producer { sender, stream } => (sender, stream),
        };
        let frame = self.capture_jpeg(&entry)?;
        publish_camera_frame(&tx, frame);
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

/// Register camera snapshot, preview stream, and recording transition
/// handlers backed by `backend`.
///
/// `media::register` deliberately skips the camera names
/// once this real module exists, so each dispatch slot has one
/// handler family. `camera.subscribe` opens a realtime in-memory
/// preview stream; recording consumes that same stream source and
/// persists only the explicit recording artifact.
pub fn register_with_backend(reg: &mut AxonAbilityCatalog, backend: Arc<dyn SnapshotBackend>) {
    let recorder: Arc<dyn CameraRecordingEngine> = Arc::new(MjpegRecordingEngine {
        backend: Arc::clone(&backend),
    });
    register_with_components(reg, backend, recorder);
}

fn register_with_components(
    reg: &mut AxonAbilityCatalog,
    backend: Arc<dyn SnapshotBackend>,
    recorder: Arc<dyn CameraRecordingEngine>,
) {
    let subscribe_backend = Arc::clone(&backend);
    reg.register_rpc_with_envelope_and_spec_and_semantics(
        ABILITY_CAMERA_SNAPSHOT,
        OwnerKind::media_system(),
        media::registry_manifest(ABILITY_CAMERA_SNAPSHOT),
        media::receipt_semantics(ABILITY_CAMERA_SNAPSHOT)
            .expect("camera.snapshot receipt semantics"),
        Arc::new(move |env: EnvelopeContext, args: Value| snapshot_handler(&backend, env, args)),
    );
    reg.register_stream_with_envelope_and_spec_and_semantics(
        ABILITY_CAMERA_SUBSCRIBE,
        OwnerKind::media_system(),
        media::registry_manifest(ABILITY_CAMERA_SUBSCRIBE),
        media::receipt_semantics(ABILITY_CAMERA_SUBSCRIBE)
            .expect("camera.subscribe receipt semantics"),
        Arc::new(move |env: EnvelopeContext, args: Value| {
            subscribe_handler(&subscribe_backend, env, args)
        }),
    );
    reg.register_rpc_with_envelope_and_spec_and_semantics(
        ABILITY_CAMERA_RECORD_START,
        OwnerKind::media_system(),
        media::registry_manifest(ABILITY_CAMERA_RECORD_START),
        media::receipt_semantics(ABILITY_CAMERA_RECORD_START)
            .expect("camera.record_start receipt semantics"),
        Arc::new(move |env: EnvelopeContext, args: Value| {
            record_start_handler(&recorder, env, args)
        }),
    );
    reg.register_rpc_with_envelope_and_spec_and_semantics(
        ABILITY_CAMERA_RECORD_STOP,
        OwnerKind::media_system(),
        media::registry_manifest(ABILITY_CAMERA_RECORD_STOP),
        media::receipt_semantics(ABILITY_CAMERA_RECORD_STOP)
            .expect("camera.record_stop receipt semantics"),
        Arc::new(record_stop_handler),
    );
}

/// Register with the production `NokhwaBackend`. The
/// daemon boot path calls this after `media::register`,
/// which now skips camera names to keep registration ownership
/// explicit.
/// Tests that need a hardware-free path call
/// `register_with_backend(reg, Arc::new(SyntheticBackend::default()))`
/// instead — the trait keeps the dispatch / receipt code
/// backend-agnostic.
pub fn register(reg: &mut AxonAbilityCatalog) {
    #[cfg(feature = "native-media")]
    {
        let backend: Arc<dyn SnapshotBackend> = Arc::new(NokhwaBackend::default());
        #[cfg(target_os = "macos")]
        let recorder: Arc<dyn CameraRecordingEngine> = Arc::new(AvFoundationRecordingEngine);
        #[cfg(not(target_os = "macos"))]
        let recorder: Arc<dyn CameraRecordingEngine> = Arc::new(MjpegRecordingEngine {
            backend: Arc::clone(&backend),
        });
        register_with_components(reg, backend, recorder);
    }
    #[cfg(all(not(feature = "native-media"), feature = "headless-media"))]
    register_with_backend(reg, Arc::new(SyntheticBackend::default()));
}

// ── Handler core ─────────────────────────────────────────────

fn snapshot_handler(
    backend: &Arc<dyn SnapshotBackend>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let entry = resolve_camera_subject(&env, &args, ABILITY_CAMERA_SNAPSHOT)?;
    let device_scope = ContextDeviceScope::from_execution_actor(env.callee())?;

    // Capture + encode.
    let EncodedFrame {
        jpeg_bytes,
        width,
        height,
    } = backend.capture_jpeg(&entry)?;

    if jpeg_bytes.len() > MAX_INLINE_BYTES {
        anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: encoded image {} bytes exceeds \
             inline cap {MAX_INLINE_BYTES}; use a payloadstore-backed capture path; \
             reason={REASON_IMAGE_TOO_LARGE}",
            jpeg_bytes.len()
        );
    }

    let capture = crate::daemon::persistence::context_store::record_capture(
        crate::daemon::persistence::context_store::CaptureRecord {
            device: device_scope.as_str(),
            ability: ABILITY_CAMERA_SNAPSHOT,
            ext: "jpg",
            bytes: jpeg_bytes.as_ref(),
            content_type: "image/jpeg",
            width: Some(width),
            height: Some(height),
            duration_ms: None,
            preview: format!("Photo {width}x{height}"),
        },
    )?;
    let local_path = crate::daemon::persistence::context_store::captures_dir()
        .join(ABILITY_CAMERA_SNAPSHOT)
        .join(&capture.file);
    let image_bytes_b64 = BASE64_STANDARD.encode(jpeg_bytes.as_ref());

    Ok(json!({
        "image_bytes_b64": image_bytes_b64,
        "content_type": "image/jpeg",
        "width": width,
        "height": height,
        "byte_size": jpeg_bytes.len(),
        "captured_at": capture.timestamp,
        "local_path": local_path.display().to_string(),
        "capture_id": capture.id,
        "capture_file": capture.file,
        "hardware_id": entry.hardware_id,
    }))
}

fn subscribe_handler(
    backend: &Arc<dyn SnapshotBackend>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<StreamSource> {
    let entry = resolve_camera_subject(&env, &args, ABILITY_CAMERA_SUBSCRIBE)?;
    let options = parse_stream_options_for(ABILITY_CAMERA_SUBSCRIBE, &args)?;
    let rx = backend.open_stream(entry, options)?;
    project_camera_preview_stream(rx)
}

fn record_start_handler(
    recorder: &Arc<dyn CameraRecordingEngine>,
    env: EnvelopeContext,
    args: Value,
) -> anyhow::Result<Value> {
    let entry = resolve_camera_subject(&env, &args, ABILITY_CAMERA_RECORD_START)?;
    let device_scope = ContextDeviceScope::from_execution_actor(env.callee())?;
    let options = parse_recording_options(&args)?;
    recorder.validate_options(&options)?;
    let session_id = format!("camera-rec-{}", uuid::Uuid::new_v4().simple());
    let started_at = chrono::Utc::now().to_rfc3339();
    let stop = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let worker_recorder = Arc::clone(recorder);
    let worker_entry = entry.clone();
    let worker_stop = Arc::clone(&stop);
    let worker_options = options.clone();
    let worker_session_id = session_id.clone();

    let session = CameraRecordingSession {
        id: session_id.clone(),
        device_ura: device_scope.as_str().to_string(),
        resource_ura: entry.resource_ura.clone(),
        hardware_id: entry.hardware_id.clone(),
        started_at: started_at.clone(),
        stop: Arc::clone(&stop),
        done: Some(done_rx),
    };
    {
        let mut sessions = recording_sessions().lock().map_err(|_| {
            anyhow::anyhow!("{ABILITY_CAMERA_RECORD_START}: recording session lock poisoned")
        })?;
        if sessions
            .values()
            .any(|existing| existing.resource_ura == entry.resource_ura)
        {
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_START}: camera resource already has an active recording; \
                 reason=recording_already_active"
            );
        }
        sessions.insert(session_id.clone(), session);
    }

    if let Err(e) = std::thread::Builder::new()
        .name("easynet-camera-recording".into())
        .spawn(move || {
            let result = worker_recorder.record(
                worker_entry,
                worker_options,
                worker_stop,
                &worker_session_id,
                ready_tx.clone(),
            );
            if let Err(error) = &result {
                let _ = ready_tx.send(Err(anyhow::anyhow!("{error:#}")));
            }
            let _ = done_tx.send(result);
        })
    {
        remove_recording_session(&session_id);
        anyhow::bail!(
            "{ABILITY_CAMERA_RECORD_START}: failed to spawn camera recording worker: {e}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }

    match ready_rx.recv_timeout(RECORDING_START_TIMEOUT) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            stop.store(true, Ordering::Relaxed);
            remove_recording_session(&session_id);
            return Err(error);
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            stop.store(true, Ordering::Relaxed);
            remove_recording_session(&session_id);
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_START}: recording backend did not become ready within {}ms; \
                 reason=recording_start_timeout",
                RECORDING_START_TIMEOUT.as_millis()
            );
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            stop.store(true, Ordering::Relaxed);
            remove_recording_session(&session_id);
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_START}: recording backend disconnected before readiness; \
                 reason=recording_worker_disconnected"
            );
        }
    }

    Ok(json!({
        "recording_session_id": session_id,
        "state": "recording",
        "started_at": started_at,
        "subject_ura": entry.resource_ura,
        "hardware_id": entry.hardware_id,
        "content_type": recorder.content_type(),
        "max_duration_ms": options.max_duration_ms,
        "max_bytes": options.max_bytes,
    }))
}

fn record_stop_handler(env: EnvelopeContext, args: Value) -> anyhow::Result<Value> {
    let entry = resolve_camera_subject(&env, &args, ABILITY_CAMERA_RECORD_STOP)?;
    let session_id =
        required_string_arg(&args, "recording_session_id", ABILITY_CAMERA_RECORD_STOP)?;
    let stop_lease = {
        let mut sessions = recording_sessions().lock().map_err(|_| {
            anyhow::anyhow!("{ABILITY_CAMERA_RECORD_STOP}: recording session lock poisoned")
        })?;
        let Some(existing) = sessions.get_mut(&session_id) else {
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_STOP}: unknown recording_session_id {session_id:?}; \
                 reason=recording_session_not_found"
            );
        };
        if existing.resource_ura != entry.resource_ura {
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_STOP}: recording_session_id {session_id:?} belongs to {}, not {}; \
                 reason=recording_subject_mismatch",
                existing.resource_ura,
                entry.resource_ura
            );
        }
        existing.stop.store(true, Ordering::Relaxed);
        let Some(done) = existing.done.take() else {
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_STOP}: recording_session_id {session_id:?} is already stopping; \
                 reason=recording_stop_in_progress"
            );
        };
        CameraRecordingStopLease {
            id: existing.id.clone(),
            device_ura: existing.device_ura.clone(),
            resource_ura: existing.resource_ura.clone(),
            hardware_id: existing.hardware_id.clone(),
            started_at: existing.started_at.clone(),
            done,
        }
    };
    let artifact = match stop_lease.done.recv_timeout(RECORDING_STOP_TIMEOUT) {
        Ok(result) => {
            remove_recording_session(&stop_lease.id);
            result?
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            let lease_id = stop_lease.id.clone();
            restore_recording_stop_lease(stop_lease);
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_STOP}: timed out waiting for recording session {lease_id} to stop; \
                 reason=recording_stop_timeout"
            );
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            let lease_id = stop_lease.id.clone();
            remove_recording_session(&lease_id);
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_STOP}: recording worker disconnected for session {lease_id}; \
                 reason=recording_worker_disconnected"
            );
        }
    };
    let preview = artifact
        .frame_count
        .map(|frames| {
            format!(
                "Camera recording {frames} frame{}",
                if frames == 1 { "" } else { "s" }
            )
        })
        .unwrap_or_else(|| "Native camera recording".to_string());
    let capture = crate::daemon::persistence::context_store::record_capture_file(
        crate::daemon::persistence::context_store::CaptureFileRecord {
            device: &stop_lease.device_ura,
            ability: ABILITY_CAMERA_RECORD_STOP,
            ext: artifact.extension,
            source_path: &artifact.temp_path,
            content_type: artifact.content_type,
            width: artifact.width,
            height: artifact.height,
            duration_ms: Some(artifact.duration_ms),
            preview,
        },
    )?;
    let local_path = crate::daemon::persistence::context_store::captures_dir()
        .join(ABILITY_CAMERA_RECORD_STOP)
        .join(&capture.file);

    Ok(json!({
        "recording_session_id": stop_lease.id,
        "state": "stopped",
        "started_at": stop_lease.started_at,
        "stopped_at": artifact.stopped_at,
        "duration_ms": artifact.duration_ms,
        "frame_count": artifact.frame_count,
        "byte_size": artifact.byte_size,
        "content_type": artifact.content_type,
        "local_path": local_path.display().to_string(),
        "capture_id": capture.id,
        "capture_file": capture.file,
        "subject_ura": stop_lease.resource_ura,
        "hardware_id": stop_lease.hardware_id,
        "width": artifact.width,
        "height": artifact.height,
        "stop_reason": artifact.stop_reason,
    }))
}

fn remove_recording_session(session_id: &str) {
    if let Ok(mut sessions) = recording_sessions().lock() {
        sessions.remove(session_id);
    }
}

fn restore_recording_stop_lease(stop_lease: CameraRecordingStopLease) {
    if let Ok(mut sessions) = recording_sessions().lock() {
        if let Some(session) = sessions.get_mut(&stop_lease.id) {
            if session.done.is_none() {
                session.done = Some(stop_lease.done);
            }
        }
    }
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

fn project_camera_preview_stream(mut frames: CameraFrameStream) -> anyhow::Result<StreamSource> {
    let (sender, receiver) = tokio_mpsc::channel(PREVIEW_OUTPUT_CAPACITY);
    std::thread::Builder::new()
        .name("easynet-camera-preview-projector".into())
        .spawn(move || {
            while let Some(update) = frames.blocking_next() {
                let output = match update {
                    CameraFrameUpdate::Pending => continue,
                    CameraFrameUpdate::Frame(frame) => Ok(StreamOutputFrame::new(
                        frame.jpeg_bytes.as_ref().to_vec(),
                        "image/jpeg",
                    )),
                    CameraFrameUpdate::Failed(message) => Err(anyhow::anyhow!(
                        "{ABILITY_CAMERA_SUBSCRIBE}: {}; reason={REASON_RESOURCE_UNAVAILABLE}",
                        message
                    )),
                };
                let terminal = output.is_err();
                if sender.blocking_send(output).is_err() || terminal {
                    break;
                }
            }
        })
        .map_err(|error| {
            anyhow::anyhow!(
                "{ABILITY_CAMERA_SUBSCRIBE}: failed to spawn preview projector: {error}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
    Ok(StreamSource::TypedBackpressuredLive(receiver))
}

fn run_mjpeg_recording_worker(
    backend: Arc<dyn SnapshotBackend>,
    entry: ResourceEntry,
    options: CameraRecordingOptions,
    stop: Arc<AtomicBool>,
    temp_path: PathBuf,
    ready: mpsc::Sender<anyhow::Result<()>>,
) -> anyhow::Result<CameraRecordingArtifact> {
    let cleanup_path = temp_path.clone();
    let result = run_mjpeg_recording_worker_inner(backend, entry, options, stop, temp_path, ready);
    if result.is_err() {
        let _ = fs::remove_file(cleanup_path);
    }
    result
}

fn run_mjpeg_recording_worker_inner(
    backend: Arc<dyn SnapshotBackend>,
    entry: ResourceEntry,
    options: CameraRecordingOptions,
    stop: Arc<AtomicBool>,
    temp_path: PathBuf,
    ready: mpsc::Sender<anyhow::Result<()>>,
) -> anyhow::Result<CameraRecordingArtifact> {
    if let Some(parent) = temp_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut rx = backend.open_stream(entry, options.stream.clone())?;
    let started = Instant::now();
    let mut writer = BufWriter::new(File::create(&temp_path)?);
    ready.send(Ok(())).map_err(|_| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_RECORD_START}: caller abandoned recording readiness; \
             reason=recording_start_cancelled"
        )
    })?;
    let mut frame_count = 0u64;
    let mut byte_size = 0u64;
    let mut width = None;
    let mut height = None;
    let mut stop_reason = "stopped";

    loop {
        if started.elapsed().as_millis() as u64 >= options.max_duration_ms {
            stop_reason = "duration_limit";
            break;
        }
        match rx.try_next() {
            CameraFramePoll::Frame(frame) => {
                let frame = frame;
                write_mjpeg_part(&mut writer, frame.jpeg_bytes.as_ref())?;
                frame_count += 1;
                byte_size += frame.jpeg_bytes.len() as u64;
                width = Some(frame.width);
                height = Some(frame.height);
                if byte_size >= options.max_bytes {
                    stop_reason = "byte_limit";
                    break;
                }
            }
            CameraFramePoll::Pending => {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            CameraFramePoll::Failed(message) => {
                anyhow::bail!(
                    "{ABILITY_CAMERA_RECORD_START}: camera stream failed while recording: {}; \
                     reason={REASON_RESOURCE_UNAVAILABLE}",
                    message
                );
            }
            CameraFramePoll::Closed => {
                stop_reason = "stream_closed";
                break;
            }
        }
        if stop.load(Ordering::Relaxed) {
            break;
        }
    }

    if frame_count == 0 {
        anyhow::bail!(
            "{ABILITY_CAMERA_RECORD_STOP}: recording produced no camera frames; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }
    write!(writer, "--{RECORDING_BOUNDARY}--\r\n")?;
    writer.flush()?;
    let stopped_at = chrono::Utc::now().to_rfc3339();
    let byte_size = fs::metadata(&temp_path)?.len();
    Ok(CameraRecordingArtifact {
        temp_path,
        extension: "mjpeg",
        content_type: MJPEG_CONTENT_TYPE,
        stopped_at,
        duration_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        frame_count: Some(frame_count),
        byte_size,
        width,
        height,
        stop_reason,
    })
}

fn write_mjpeg_part(writer: &mut BufWriter<File>, jpeg_bytes: &[u8]) -> anyhow::Result<()> {
    write!(
        writer,
        "--{RECORDING_BOUNDARY}\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        jpeg_bytes.len()
    )?;
    writer.write_all(jpeg_bytes)?;
    writer.write_all(b"\r\n")?;
    Ok(())
}

fn recording_temp_path(session_id: &str, extension: &str) -> anyhow::Result<PathBuf> {
    if !session_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        anyhow::bail!("{ABILITY_CAMERA_RECORD_START}: invalid recording session id");
    }
    Ok(state_dir()
        .join("captures")
        .join("camera-recording-sessions")
        .join(format!("{session_id}.{extension}.tmp")))
}

fn parse_recording_options(args: &Value) -> anyhow::Result<CameraRecordingOptions> {
    let stream = parse_stream_options_for(ABILITY_CAMERA_RECORD_START, args)?;
    let codec = args
        .get("codec")
        .and_then(Value::as_str)
        .map(str::to_string);
    let max_duration_ms = optional_u64_arg(args, "max_duration_ms")?
        .unwrap_or(DEFAULT_RECORDING_MAX_DURATION_MS)
        .clamp(1_000, MAX_RECORDING_MAX_DURATION_MS);
    let max_bytes = optional_u64_arg(args, "max_bytes")?
        .unwrap_or(DEFAULT_RECORDING_MAX_BYTES)
        .clamp(1_048_576, MAX_RECORDING_MAX_BYTES);
    Ok(CameraRecordingOptions {
        stream,
        codec,
        max_duration_ms,
        max_bytes,
    })
}

fn optional_u64_arg(args: &Value, key: &str) -> anyhow::Result<Option<u64>> {
    let Value::Object(map) = args else {
        return Ok(None);
    };
    let Some(value) = map.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("{key} must be a non-negative integer"))
}

fn required_string_arg(args: &Value, key: &str, ability: &'static str) -> anyhow::Result<String> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("{ability}: `{key}` is required; reason=invalid_argument")
        })?;
    Ok(value.to_string())
}

fn parse_stream_options_for(
    ability: &'static str,
    args: &Value,
) -> anyhow::Result<CameraStreamOptions> {
    let mut options = CameraStreamOptions::default();
    if let Value::Object(map) = args {
        if ability == ABILITY_CAMERA_SUBSCRIBE {
            if let Some(value) = map.get("codec") {
                let codec = value.as_str().ok_or_else(|| {
                    anyhow::anyhow!(
                        "{ability}: codec must be a string; \
                         reason=invalid_argument"
                    )
                })?;
                if codec != "jpeg" {
                    anyhow::bail!(
                        "{ability}: codec must be \"jpeg\"; \
                         reason=invalid_argument"
                    );
                }
            }
        }
        if let Some(value) = map.get("fps") {
            let fps = value.as_u64().ok_or_else(|| {
                anyhow::anyhow!(
                    "{ability}: fps must be an integer; \
                     reason=invalid_argument"
                )
            })?;
            if !(MIN_CAMERA_FPS as u64..=MAX_CAMERA_FPS as u64).contains(&fps) {
                anyhow::bail!(
                    "{ability}: fps {fps} outside {MIN_CAMERA_FPS}..={MAX_CAMERA_FPS}; \
                     reason=invalid_argument"
                );
            }
            options.fps = fps as u32;
        }
        if let Some(value) = map.get("resolution") {
            options.resolution = parse_resolution_for(ability, value)?;
        }
    }
    Ok(options)
}

fn parse_resolution_for(
    ability: &'static str,
    value: &Value,
) -> anyhow::Result<Option<CameraVideoResolution>> {
    let Some(raw) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
        anyhow::bail!(
            "{ability}: resolution must be a string; \
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
                    "{ability}: resolution {raw:?} must be native, 480p, 720p, 1080p, or <width>x<height>; \
                     reason=invalid_argument"
                );
            };
            CameraVideoResolution {
                width: parse_positive_u32(ability, w, "resolution width")?,
                height: parse_positive_u32(ability, h, "resolution height")?,
            }
        }
    };
    Ok(Some(resolution))
}

fn parse_positive_u32(ability: &'static str, raw: &str, name: &str) -> anyhow::Result<u32> {
    let value = raw.parse::<u32>().map_err(|_| {
        anyhow::anyhow!(
            "{ability}: {name} must be an integer; \
             reason=invalid_argument"
        )
    })?;
    if value == 0 {
        anyhow::bail!(
            "{ability}: {name} must be positive; \
             reason=invalid_argument"
        );
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::invocation::routing::target::CallMode;
    use crate::daemon::persistence::resources::{
        self, upsert_resource, ResourceBinding, ResourceUpsert, ResourcesFile,
    };

    /// Build a one-resource ResourcesFile and return its URA. The
    /// caller passes the file to `lookup_by_ura` via the on-disk
    /// path; tests that need to round-trip through the real handler
    /// must use HomeGuard so `resources::load` reads the right path.
    fn seed_camera(file: &mut ResourcesFile, hardware_id: &str) -> String {
        upsert_resource(
            file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
                kind: ResourceType::Camera,
                binding: ResourceBinding::LocalDevice,
                hardware_id,
                display_name: "Test Camera",
                metadata: json!({}),
            },
        )
        .expect("seed camera resource")
    }

    /// Test register helper. Tests use SyntheticBackend so the
    /// suite runs hardware-free (CI / Linux-without-camera). The
    /// daemon's `register(reg)` defaults to `NokhwaBackend` which
    /// only works against a real `/dev/video*` or AVFoundation
    /// device.
    fn register_synthetic(reg: &mut AxonAbilityCatalog) {
        register_with_backend(reg, Arc::new(SyntheticBackend::default()));
    }

    const TEST_DEVICE_URA: &str = "easynet:///r/test/device/camera-snapshot";

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

    fn clear_recording_sessions_for_test() {
        recording_sessions().lock().unwrap().clear();
    }

    fn recording_session_test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct FailsBeforeReadyRecordingEngine;

    impl CameraRecordingEngine for FailsBeforeReadyRecordingEngine {
        fn content_type(&self) -> &'static str {
            "video/quicktime"
        }

        fn validate_options(&self, _options: &CameraRecordingOptions) -> anyhow::Result<()> {
            Ok(())
        }

        fn record(
            &self,
            _entry: ResourceEntry,
            _options: CameraRecordingOptions,
            _stop: Arc<AtomicBool>,
            _session_id: &str,
            _ready: mpsc::Sender<anyhow::Result<()>>,
        ) -> anyhow::Result<CameraRecordingArtifact> {
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_START}: native camera could not start; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        }
    }

    #[test]
    fn registration_publishes_camera_descriptors_to_catalog_snapshot() {
        let mut reg = metadata_test_catalog();
        register_synthetic(&mut reg);
        let rows = reg.authority_ability_catalog_snapshot();

        for ability in [
            ABILITY_CAMERA_SNAPSHOT,
            ABILITY_CAMERA_SUBSCRIBE,
            ABILITY_CAMERA_RECORD_START,
            ABILITY_CAMERA_RECORD_STOP,
        ] {
            let descriptor = rows
                .iter()
                .find(|row| row.name == ability)
                .map(|row| &row.descriptor)
                .unwrap_or_else(|| panic!("{ability} must publish canonical descriptor"));
            assert_eq!(
                descriptor.description,
                media::description(ability).expect("camera description")
            );
            assert_eq!(
                descriptor.input_schema(),
                &media::input_schema(ability).expect("camera schema")
            );
        }
    }

    /// Synthetic-backend smoke: capture against a hand-built
    /// ResourceEntry, assert the bytes look like a JPEG (magic
    /// number 0xff 0xd8) and the dimensions match.
    #[test]
    fn synthetic_backend_emits_valid_jpeg_with_expected_dimensions() {
        let mut file = ResourcesFile::default();
        seed_camera(&mut file, "h-cam-test-1");
        let entry = file.resources[0].clone();
        let backend = SyntheticBackend::default();
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
        let backend = SyntheticBackend::default();
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
        let backend = SyntheticBackend::default();
        let one = backend.capture_jpeg(&entry).unwrap();
        let two = backend.capture_jpeg(&entry).unwrap();
        assert_eq!(one.jpeg_bytes, two.jpeg_bytes);
    }

    #[test]
    fn camera_frame_lane_coalesces_to_the_latest_frame() {
        let (sender, mut stream) = camera_frame_channel();

        publish_camera_frame(&sender, EncodedFrame::new(vec![1], 1, 1));
        publish_camera_frame(&sender, EncodedFrame::new(vec![2], 2, 2));
        publish_camera_frame(&sender, EncodedFrame::new(vec![3], 3, 3));

        match stream.try_next() {
            CameraFramePoll::Frame(frame) => {
                assert_eq!(frame.jpeg_bytes.as_ref(), &[3]);
                assert_eq!((frame.width, frame.height), (3, 3));
            }
            other => panic!(
                "latest-frame lane must expose the newest frame, got {}",
                camera_frame_poll_name(&other)
            ),
        }
    }

    #[test]
    fn snapshot_captures_a_new_photo_instead_of_reusing_preview_allocation() {
        let mut file = ResourcesFile::default();
        seed_camera(&mut file, "h-cam-live-snapshot");
        let entry = file.resources[0].clone();
        let backend = SyntheticBackend::default();
        let mut stream = backend
            .open_stream(entry.clone(), CameraStreamOptions::default())
            .unwrap();
        let live_bytes = match stream.try_next() {
            CameraFramePoll::Frame(frame) => frame.jpeg_bytes,
            other => panic!(
                "live camera must publish its initial frame, got {}",
                camera_frame_poll_name(&other)
            ),
        };

        let snapshot = backend.capture_jpeg(&entry).unwrap();
        assert!(
            !Arc::ptr_eq(&live_bytes, &snapshot.jpeg_bytes),
            "snapshot must be an independent capture, not a borrowed preview frame"
        );
    }

    #[test]
    fn matching_live_consumers_share_the_current_camera_frame() {
        let mut file = ResourcesFile::default();
        seed_camera(&mut file, "h-cam-shared-live");
        let entry = file.resources[0].clone();
        let backend = SyntheticBackend::default();
        let mut preview = backend
            .open_stream(entry.clone(), CameraStreamOptions::default())
            .unwrap();
        let preview_bytes = match preview.try_next() {
            CameraFramePoll::Frame(frame) => frame.jpeg_bytes,
            other => panic!(
                "first camera consumer must receive a frame, got {}",
                camera_frame_poll_name(&other)
            ),
        };

        let mut recorder = backend
            .open_stream(entry, CameraStreamOptions::default())
            .unwrap();
        let recorder_bytes = match recorder.try_next() {
            CameraFramePoll::Frame(frame) => frame.jpeg_bytes,
            other => panic!(
                "joining camera consumer must receive current frame, got {}",
                camera_frame_poll_name(&other)
            ),
        };
        assert!(Arc::ptr_eq(&preview_bytes, &recorder_bytes));
        assert_eq!(backend.active_streams.streams.lock().unwrap().len(), 1);
    }

    fn camera_frame_poll_name(poll: &CameraFramePoll) -> &'static str {
        match poll {
            CameraFramePoll::Pending => "pending",
            CameraFramePoll::Frame(_) => "frame",
            CameraFramePoll::Failed(_) => "failed",
            CameraFramePoll::Closed => "closed",
        }
    }

    /// End-to-end: the compatibility unary response remains JSON/base64 while
    /// capture itself uses the dedicated still-photo backend.
    #[test]
    fn handler_returns_receipt_with_base64_jpeg_when_subject_resolves() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        // Seed the on-disk resources.json so the handler's
        // `resources::load` finds it.
        let mut file = ResourcesFile::default();
        let ura = seed_camera(&mut file, "h-cam-e2e");
        resources::save(&file).unwrap();

        let mut reg = executable_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_CAMERA_SNAPSHOT,
            json!({}),
            CallMode::Rpc,
            ura,
        );
        let response = dispatcher.execute_rpc(target).unwrap();
        assert_eq!(response["content_type"], "image/jpeg");
        assert_eq!(response["width"], 64);
        assert_eq!(response["height"], 48);
        assert_eq!(response["hardware_id"], "h-cam-e2e");
        let decoded = BASE64_STANDARD
            .decode(response["image_bytes_b64"].as_str().unwrap())
            .unwrap();
        assert_eq!(decoded[..2], [0xff, 0xd8]);
        assert_eq!(decoded[decoded.len() - 2..], [0xff, 0xd9]);
        assert_eq!(
            decoded.len(),
            response["byte_size"].as_u64().unwrap() as usize
        );
        let captures = crate::daemon::persistence::context_store::list_captures(
            TEST_DEVICE_URA,
            Some(ABILITY_CAMERA_SNAPSHOT),
            10,
        )
        .unwrap();
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].width, Some(64));
        assert_eq!(captures[0].height, Some(48));
        let local_path = crate::daemon::persistence::context_store::captures_dir()
            .join(ABILITY_CAMERA_SNAPSHOT)
            .join(&captures[0].file);
        assert_eq!(std::fs::read(local_path).unwrap(), decoded);
    }

    #[test]
    fn snapshot_fails_when_context_media_cannot_be_committed() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_camera(&mut file, "h-cam-storage-failure");
        resources::save(&file).unwrap();
        std::fs::create_dir_all(crate::daemon::persistence::context_store::context_dir()).unwrap();
        std::fs::write(
            crate::daemon::persistence::context_store::captures_dir(),
            b"not a directory",
        )
        .unwrap();

        let mut reg = executable_catalog();
        register_synthetic(&mut reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_CAMERA_SNAPSHOT,
            json!({}),
            CallMode::Rpc,
            ura,
        );

        let error = Arc::new(reg)
            .execute_rpc(target)
            .expect_err("snapshot cannot report success without durable Context media");
        assert!(error.to_string().contains("Not a directory"), "{error:#}");
    }

    #[test]
    fn camera_subscribe_returns_live_preview_stream() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let ura = seed_camera(&mut file, "h-cam-preview");
        resources::save(&file).unwrap();

        let mut reg = executable_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_CAMERA_SUBSCRIBE,
            json!({}),
            CallMode::Stream,
            ura,
        );
        let source = dispatcher.execute_stream(target).unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let frame = match source {
            StreamSource::TypedBackpressuredLive(mut rx) => {
                rt.block_on(async { rx.recv().await.unwrap().unwrap() })
            }
            other => panic!("camera.subscribe must return live stream, got {other:?}"),
        };

        assert_eq!(frame.content_type, "image/jpeg");
        assert!(frame.payload.len() > 200);
        assert_eq!(&frame.payload[..2], &[0xff, 0xd8]);
        assert_eq!(&frame.payload[frame.payload.len() - 2..], &[0xff, 0xd9]);
    }

    #[test]
    fn camera_recording_start_stop_persists_mjpeg_artifact() {
        let _recording_guard = recording_session_test_guard();
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        clear_recording_sessions_for_test();
        let mut file = ResourcesFile::default();
        let ura = seed_camera(&mut file, "h-cam-recording");
        resources::save(&file).unwrap();

        let mut reg = executable_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let start = dispatcher
            .execute_rpc(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                ABILITY_CAMERA_RECORD_START,
                json!({"fps": 5, "max_duration_ms": 5000}),
                CallMode::Rpc,
                ura.clone(),
            ))
            .unwrap();
        let session_id = start["recording_session_id"].as_str().unwrap().to_string();
        assert_eq!(start["state"], "recording");

        let stop = dispatcher
            .execute_rpc(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                ABILITY_CAMERA_RECORD_STOP,
                json!({"recording_session_id": session_id}),
                CallMode::Rpc,
                ura,
            ))
            .unwrap();
        assert_eq!(stop["state"], "stopped");
        assert_eq!(stop["content_type"], MJPEG_CONTENT_TYPE);
        assert!(
            stop["frame_count"].as_u64().unwrap() >= 1,
            "recording must contain at least one frame: {stop}"
        );
        let local_path = stop["local_path"].as_str().unwrap();
        let bytes = std::fs::read(local_path).unwrap();
        assert!(
            bytes.starts_with(format!("--{RECORDING_BOUNDARY}\r\n").as_bytes()),
            "recording artifact must be multipart MJPEG"
        );
        assert!(stop["capture_id"].as_str().is_some());
        clear_recording_sessions_for_test();
    }

    #[test]
    fn camera_recording_start_does_not_claim_recording_before_backend_readiness() {
        let _recording_guard = recording_session_test_guard();
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        clear_recording_sessions_for_test();
        let mut file = ResourcesFile::default();
        let ura = seed_camera(&mut file, "h-cam-start-failure");
        resources::save(&file).unwrap();

        let backend: Arc<dyn SnapshotBackend> = Arc::new(SyntheticBackend::default());
        let recorder: Arc<dyn CameraRecordingEngine> = Arc::new(FailsBeforeReadyRecordingEngine);
        let mut reg = executable_catalog();
        register_with_components(&mut reg, backend, recorder);

        let error = Arc::new(reg)
            .execute_rpc(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                ABILITY_CAMERA_RECORD_START,
                json!({}),
                CallMode::Rpc,
                ura,
            ))
            .expect_err("start must fail until the native backend reports readiness");
        assert!(
            error.to_string().contains(REASON_RESOURCE_UNAVAILABLE),
            "{error:#}"
        );
        assert!(recording_sessions().lock().unwrap().is_empty());
    }

    #[test]
    fn camera_recording_rejects_duplicate_start_without_orphaning_first_session() {
        let _recording_guard = recording_session_test_guard();
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        clear_recording_sessions_for_test();
        let mut file = ResourcesFile::default();
        let ura = seed_camera(&mut file, "h-cam-recording-duplicate");
        resources::save(&file).unwrap();

        let mut reg = executable_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let start_args = json!({"fps": 5, "max_duration_ms": 5000, "max_bytes": 1048576});
        let first = dispatcher
            .execute_rpc(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                ABILITY_CAMERA_RECORD_START,
                start_args.clone(),
                CallMode::Rpc,
                ura.clone(),
            ))
            .unwrap();
        let session_id = first["recording_session_id"].as_str().unwrap().to_string();

        let duplicate_err = dispatcher
            .execute_rpc(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                ABILITY_CAMERA_RECORD_START,
                start_args,
                CallMode::Rpc,
                ura.clone(),
            ))
            .unwrap_err()
            .to_string();
        assert!(
            duplicate_err.contains("recording_already_active"),
            "duplicate start must fail with a stable reason: {duplicate_err}"
        );
        {
            let sessions = recording_sessions().lock().unwrap();
            let session = sessions
                .get(&session_id)
                .expect("first recording session must remain stoppable");
            assert_eq!(session.resource_ura, ura);
            assert!(
                session.done.is_some(),
                "duplicate admission failure must not steal the completion receiver"
            );
        }

        let stop = dispatcher
            .execute_rpc(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                ABILITY_CAMERA_RECORD_STOP,
                json!({"recording_session_id": session_id}),
                CallMode::Rpc,
                ura,
            ))
            .unwrap();
        assert_eq!(stop["state"], "stopped");
        clear_recording_sessions_for_test();
    }

    #[test]
    fn camera_subscribe_stream_preview_errors_name_subscribe_ability() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = executable_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target =
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
                ABILITY_CAMERA_SUBSCRIBE,
                json!({}),
                CallMode::Stream,
            );
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
    /// envelope subject MUST fail with reason="subject_required". Without this the handler would
    /// either crash or silently capture from "the first camera",
    /// either of which makes auditing a lie.
    #[test]
    fn handler_rejects_missing_subject_with_subject_required_reason() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = executable_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target =
            crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
                ABILITY_CAMERA_SNAPSHOT,
                json!({}),
                CallMode::Rpc,
            );
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(
            err.to_string().contains(REASON_SUBJECT_REQUIRED),
            "expected reason={REASON_SUBJECT_REQUIRED}; got: {err}"
        );
    }

    /// INV-RESOURCE-VALIDITY: subject points at a URA not in the
    /// local table → reason="resource_not_found". Distinct from
    /// "URA present but device unplugged" (resource_unavailable),
    /// which requires a production camera backend to exercise.
    #[test]
    fn handler_rejects_unknown_subject_with_resource_not_found_reason() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        // Save an empty resources.json so load() returns Default
        // rather than picking up some prior test's state.
        resources::save(&ResourcesFile::default()).unwrap();
        let mut reg = executable_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_CAMERA_SNAPSHOT,
            json!({}),
            CallMode::Rpc,
            "easynet:///r/acme/resource/01NEVER-EXISTED",
        );
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
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut file = ResourcesFile::default();
        let mic_ura = upsert_resource(
            &mut file,
            ResourceUpsert {
                realm: "acme",
                owner_agent: "easynet:///r/acme/agent/device.01DEV.media",
                kind: ResourceType::Mic, // not a camera
                binding: ResourceBinding::LocalDevice,
                hardware_id: "h-mic-not-camera",
                display_name: "Not A Camera",
                metadata: json!({}),
            },
        )
        .expect("seed wrong-type mic resource");
        resources::save(&file).unwrap();

        let mut reg = executable_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_CAMERA_SNAPSHOT,
            json!({}),
            CallMode::Rpc,
            mic_ura,
        );
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(
            err.to_string().contains(REASON_RESOURCE_TYPE_MISMATCH),
            "expected reason={REASON_RESOURCE_TYPE_MISMATCH}; got: {err}"
        );
    }

    /// INV-SUBJECT-ENVELOPE negative half (defence in depth): if
    /// args carry a `subject` key the handler MUST reject before
    /// any other parsing, even on the envelope-aware path. The
    /// `media` stubs already enforce this for the
    /// args-only path; this test pins the env-aware sibling.
    #[test]
    fn handler_rejects_subject_in_args_even_on_envelope_path() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let mut reg = executable_catalog();
        register_synthetic(&mut reg);
        let dispatcher = Arc::new(reg);
        let target = crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
            ABILITY_CAMERA_SNAPSHOT,
            json!({"subject": "easynet:///r/x/resource/y"}),
            CallMode::Rpc,
            "easynet:///r/acme/resource/01CAM",
        );
        let err = dispatcher.execute_rpc(target).unwrap_err();
        assert!(
            err.to_string().contains(REASON_SUBJECT_IN_ARGS),
            "expected reason={REASON_SUBJECT_IN_ARGS}; got: {err}"
        );
    }
}
