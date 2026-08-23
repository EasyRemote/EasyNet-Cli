// EasyNet CLI — ScreenCaptureKit display capture
// ===============================================
//
// File: plugins/remote-desktop/src/screencapturekit_capture.rs
// Description: macOS ScreenCaptureKit capture stream that delivers
// CVPixelBuffers from a display/window/application to a frame callback. Phase 2 of
// plugin.macos.screencapturekit.videotoolbox.webrtc.v1.
//
// Protocol Responsibility:
// - Owns ONLY capture: pick a display, configure an SCStream, deliver
//   CVImageBuffers via a delegate. Encoding (VideoToolbox, phase 1) and
//   WebRTC packetization (phase 3) live elsewhere.
//
// Implementation Approach:
// - SCShareableContent enumeration is async (completion handler block);
//   bridge it to sync with a channel + parking.
// - SCStreamConfiguration sets width/height/fps/pixel format. The
//   delegate (an objc2 define_class! object implementing SCStreamOutput)
//   forwards each screen sample's CVImageBuffer to a user callback.
//
// Capture requires the Screen Recording TCC permission; target resolution
// preflights that native permission before enumerating SCShareableContent so
// callers get a deterministic permission error instead of an opaque stalled
// stream. The OS prompt is only triggered by `remote_desktop.request_permission`.
//
// Architectural Position:
// - EasyNet-Cli device adapter, native media plugin (macOS only).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{
    kCVPixelFormatType_32BGRA, kCVReturnSuccess, CVImageBuffer, CVPixelBufferGetBaseAddress,
    CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCRunningApplication, SCShareableContent, SCStream,
    SCStreamConfiguration, SCStreamOutput, SCStreamOutputType, SCWindow,
};

use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
    bgra_bytes_to_rgb_frame, rgb_frame_to_jpeg, EncodedFrame, ScreenCaptureOptions,
};
use crate::daemon::plugins::remote_desktop::screencapturekit_audio::{
    captured_audio_chunk, AudioSink,
};
use crate::daemon::plugins::remote_desktop::screencapturekit_multiapp::{
    MultiAppSurfaceCompositor, MultiAppSurfaceTarget,
};
use crate::daemon::plugins::remote_desktop::target::{
    AppWindowSetProof, NativeAppIdentityCandidate, NativeAppIdentityExpectation,
    NativeAppIdentityMatch, RemoteAppTargetBinding, RemoteAppTargetError, RemoteDesktopTargetKind,
    ResolvedCaptureTargetProof, TargetResolutionError,
};

/// ScreenCaptureKit queue depth for live remote desktop.
///
/// Invariant: native capture must not buffer old desktop frames behind the
/// encoder. At 144 Hz, depth three is roughly 21 ms of capture-side slack.
const SCK_LIVE_QUEUE_DEPTH: isize = 3;

/// A captured frame: a CVImageBuffer plus its presentation timestamp.
///
/// CVImageBuffer (a CoreVideo/CoreFoundation reference-counted type) is safe
/// to retain, release, and hand between threads; objc2 marks `Retained` as
/// `!Send` conservatively, so we transport the frame to the encode thread
/// through a channel and assert thread-safety explicitly.
pub struct CapturedFrame {
    pub image_buffer: Retained<CVImageBuffer>,
    pub pts: CMTime,
}

// SAFETY: CVImageBuffer is a toll-free CoreFoundation object with atomic
// reference counting; moving an owned reference across threads is sound.
unsafe impl Send for CapturedFrame {}

/// Callback invoked on the capture queue for every screen sample.
pub type FrameSink = Arc<dyn Fn(CapturedFrame) + Send + Sync>;

#[derive(Clone)]
pub struct ScreenCaptureKitSinks {
    pub video: FrameSink,
    pub audio: Option<AudioSink>,
}

impl ScreenCaptureKitSinks {
    pub fn video_only(video: FrameSink) -> Self {
        Self { video, audio: None }
    }
}

/// Holds every native stream + delegate in one capture plan alive for the
/// session lifetime. A display/window uses one stream; an AppSurface uses one
/// desktop-independent stream per committed window and one bounded compositor.
pub struct ScreenCaptureKitStream {
    streams: Vec<ActiveScreenCaptureKitStream>,
    start_config: ScreenCaptureKitStartConfig,
    output_router: Arc<CaptureOutputRouter>,
    active_generation: u64,
}

struct ActiveScreenCaptureKitStream {
    stream: Retained<SCStream>,
    _delegate: Retained<StreamOutputDelegate>,
}

#[derive(Clone)]
struct ScreenCaptureKitStartConfig {
    width: usize,
    height: usize,
    fps: u32,
}

pub(in crate::daemon::plugins::remote_desktop) struct PreparedScreenCaptureKitRebind {
    streams: Option<Vec<ActiveScreenCaptureKitStream>>,
    capture_proof: ResolvedCaptureTargetProof,
    generation: u64,
}

struct CaptureOutputRouter {
    state: Mutex<CaptureOutputRouteState>,
}

struct CaptureOutputRouteState {
    active_generation: Option<u64>,
    sinks: ScreenCaptureKitSinks,
}

impl CaptureOutputRouter {
    fn new(active_generation: u64, sinks: ScreenCaptureKitSinks) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(CaptureOutputRouteState {
                active_generation: Some(active_generation),
                sinks,
            }),
        })
    }

    fn sinks_for_generation(self: &Arc<Self>, generation: u64) -> ScreenCaptureKitSinks {
        let video_router = Arc::clone(self);
        let video: FrameSink = Arc::new(move |frame| {
            video_router.deliver_if_active(generation, |sinks| (sinks.video)(frame));
        });
        let has_audio = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .sinks
            .audio
            .is_some();
        let audio = has_audio.then(|| {
            let audio_router = Arc::clone(self);
            Arc::new(move |event| {
                audio_router.deliver_if_active(generation, |sinks| {
                    if let Some(audio) = &sinks.audio {
                        audio(event);
                    }
                });
            }) as AudioSink
        });
        ScreenCaptureKitSinks { video, audio }
    }

    /// The route lock is intentionally held through delivery. Rebind first
    /// acquires this same lock to pause the old generation, so an old callback
    /// cannot pass the generation check and complete after the binding commit.
    fn deliver_if_active(&self, generation: u64, deliver: impl FnOnce(&ScreenCaptureKitSinks)) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.active_generation == Some(generation) {
            deliver(&state.sinks);
        }
    }

    fn select_generation(&self, generation: Option<u64>) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .active_generation = generation;
    }
}

impl Drop for PreparedScreenCaptureKitRebind {
    fn drop(&mut self) {
        if let Some(streams) = self.streams.take() {
            stop_active_streams(&streams);
        }
    }
}

enum ScreenCaptureKitCapturePlan {
    Single(Retained<SCContentFilter>),
    MultiApp(MultiAppSurfaceTarget),
}

/// A resolved ScreenCaptureKit target. It owns the platform content filter and
/// exposes the native dimensions that should be used when the ability requests
/// `resolution=native`.
pub struct ScreenCaptureKitTarget {
    capture_plan: ScreenCaptureKitCapturePlan,
    native_width: usize,
    native_height: usize,
    capture_proof: ResolvedCaptureTargetProof,
}

impl ScreenCaptureKitTarget {
    pub fn native_dimensions(&self) -> (usize, usize) {
        (self.native_width, self.native_height)
    }

    fn capture_proof(&self) -> &ResolvedCaptureTargetProof {
        &self.capture_proof
    }
}

struct DelegateIvars {
    sinks: ScreenCaptureKitSinks,
}

define_class!(
    // SCStreamOutput delegate: receives CMSampleBuffers on the capture queue
    // and forwards the wrapped CVImageBuffer to the frame sink.
    #[unsafe(super(NSObject))]
    #[name = "EasyNetSCKStreamOutput"]
    #[ivars = DelegateIvars]
    struct StreamOutputDelegate;

    unsafe impl NSObjectProtocol for StreamOutputDelegate {}

    unsafe impl SCStreamOutput for StreamOutputDelegate {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn stream_did_output(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            output_type: SCStreamOutputType,
        ) {
            match output_type {
                SCStreamOutputType::Screen => {
                    // Only forward samples that carry a ready image buffer.
                    let Some(image_buffer) = (unsafe { sample_buffer.image_buffer() }) else {
                        return;
                    };
                    let pts = unsafe { sample_buffer.presentation_time_stamp() };
                    (self.ivars().sinks.video)(CapturedFrame {
                        image_buffer: image_buffer.into(),
                        pts,
                    });
                }
                SCStreamOutputType::Audio => {
                    if let Some(audio) = &self.ivars().sinks.audio {
                        audio(captured_audio_chunk(sample_buffer));
                    }
                }
                _ => {}
            }
        }
    }
);

impl StreamOutputDelegate {
    fn new(sinks: ScreenCaptureKitSinks) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { sinks });
        unsafe { msg_send![super(this), init] }
    }
}

/// Synchronously enumerate shareable content (bridges the async
/// SCShareableContent completion handler).
fn shareable_content(
    ability: &'static str,
) -> Result<Retained<SCShareableContent>, RemoteAppTargetError> {
    let (tx, rx) = sync_channel::<Result<Retained<SCShareableContent>, String>>(1);
    let tx = Mutex::new(Some(tx));

    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            let result = if !error.is_null() {
                let err = unsafe { &*error };
                Err(format!(
                    "SCShareableContent error: {}",
                    err.localizedDescription()
                ))
            } else if content.is_null() {
                Err("SCShareableContent returned null".to_string())
            } else {
                match unsafe { Retained::retain(content) } {
                    Some(content) => Ok(content),
                    None => Err("SCShareableContent retain failed".to_string()),
                }
            };
            if let Some(tx) = take_completion_sender(&tx) {
                let _ = tx.send(result);
            }
        },
    );

    unsafe {
        SCShareableContent::getShareableContentWithCompletionHandler(&handler);
    }

    // The completion handler runs on a background dispatch queue, so a plain
    // recv with timeout is sufficient; we are not blocking its queue.
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(displays)) => Ok(displays),
        Ok(Err(msg)) => Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::ScreenCaptureKitEnumerationFailed,
            msg,
        )),
        Err(_) => Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::ScreenCaptureKitEnumerationFailed,
            "SCShareableContent enumeration timed out",
        )),
    }
}

pub fn screen_capture_permission_granted() -> bool {
    unsafe { macos_screen_capture_tcc::preflight_screen_capture_access() }
}

pub fn request_screen_capture_permission() -> bool {
    unsafe { macos_screen_capture_tcc::request_screen_capture_access() }
}

fn ensure_screen_capture_permission(ability: &'static str) -> Result<(), RemoteAppTargetError> {
    if screen_capture_permission_granted() {
        return Ok(());
    }
    Err(RemoteAppTargetError::new(
        ability,
        TargetResolutionError::TargetPermissionMissing,
        format!(
            "macOS Screen Recording permission is not granted for {}; \
             open System Settings > Privacy & Security > Screen & System Audio Recording, \
             grant access to this binary, then restart the daemon",
            std::env::current_exe()
                .ok()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "the EasyNet daemon process".to_string())
        ),
    ))
}

pub(in crate::daemon::plugins::remote_desktop) fn target_for_binding(
    ability: &'static str,
    binding: &RemoteAppTargetBinding,
) -> Result<ScreenCaptureKitTarget, RemoteAppTargetError> {
    let target = resolve_target_for_binding(ability, binding)?;
    binding.validate_reverified_capture_proof(ability, target.capture_proof())?;
    Ok(target)
}

pub(in crate::daemon::plugins::remote_desktop) fn capture_jpeg_for_binding(
    ability: &'static str,
    binding: &RemoteAppTargetBinding,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<EncodedFrame> {
    let target = target_for_binding(ability, binding)?;
    let (width, height) = target.native_dimensions();
    let (tx, rx) = sync_channel::<CapturedFrame>(1);
    let sender = Arc::new(Mutex::new(Some(tx)));
    let sink: FrameSink = {
        let sender = Arc::clone(&sender);
        Arc::new(move |frame| {
            if let Some(tx) = take_completion_sender(&sender) {
                let _ = tx.send(frame);
            }
        })
    };
    let stream = ScreenCaptureKitStream::start(
        ability,
        target,
        width,
        height,
        options.fps,
        ScreenCaptureKitSinks::video_only(sink),
    )?;
    let frame = rx.recv_timeout(Duration::from_secs(3)).map_err(|err| {
        anyhow::anyhow!(
            "{ability}: ScreenCaptureKit did not produce a diagnostic frame for binding {}: {err}; \
             reason={}",
            binding.binding_id(),
            TargetResolutionError::CaptureBackendUnavailable.as_str()
        )
    })?;
    drop(stream);
    encode_bgra_image_buffer_as_jpeg(&frame.image_buffer, ability, options)
}

fn encode_bgra_image_buffer_as_jpeg(
    image_buffer: &CVImageBuffer,
    ability: &'static str,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<EncodedFrame> {
    let pixel_format = CVPixelBufferGetPixelFormatType(image_buffer);
    if pixel_format != kCVPixelFormatType_32BGRA {
        anyhow::bail!(
            "{ability}: ScreenCaptureKit returned unsupported pixel format 0x{pixel_format:08x}; \
             reason={}",
            TargetResolutionError::CaptureBackendUnavailable.as_str()
        );
    }

    let lock_flags = CVPixelBufferLockFlags::ReadOnly;
    let lock_result = unsafe { CVPixelBufferLockBaseAddress(image_buffer, lock_flags) };
    if lock_result != kCVReturnSuccess {
        anyhow::bail!(
            "{ability}: CVPixelBufferLockBaseAddress failed with {lock_result}; \
             reason={}",
            TargetResolutionError::CaptureBackendUnavailable.as_str()
        );
    }

    let result = encode_locked_bgra_image_buffer_as_jpeg(image_buffer, ability, options);
    let _ = unsafe { CVPixelBufferUnlockBaseAddress(image_buffer, lock_flags) };
    result
}

fn encode_locked_bgra_image_buffer_as_jpeg(
    image_buffer: &CVImageBuffer,
    ability: &'static str,
    options: &ScreenCaptureOptions,
) -> anyhow::Result<EncodedFrame> {
    let width = CVPixelBufferGetWidth(image_buffer);
    let height = CVPixelBufferGetHeight(image_buffer);
    let stride = CVPixelBufferGetBytesPerRow(image_buffer);
    let base = CVPixelBufferGetBaseAddress(image_buffer);
    if width == 0 || height == 0 || stride < width.saturating_mul(4) || base.is_null() {
        anyhow::bail!(
            "{ability}: ScreenCaptureKit returned invalid pixel buffer dimensions \
             {width}x{height} stride={stride}; reason={}",
            TargetResolutionError::CaptureBackendUnavailable.as_str()
        );
    }
    let bytes = unsafe { std::slice::from_raw_parts(base.cast::<u8>(), stride * height) };
    let rgb = bgra_bytes_to_rgb_frame(bytes, width as u32, height as u32, stride, options)?;
    rgb_frame_to_jpeg(rgb)
}

fn resolve_target_for_binding(
    ability: &'static str,
    binding: &RemoteAppTargetBinding,
) -> Result<ScreenCaptureKitTarget, RemoteAppTargetError> {
    ensure_screen_capture_permission(ability)?;
    let content = shareable_content(ability)?;
    let mut proof_window_id = None;
    let mut proof_pid = None;
    let mut proof_app_identity = None;
    let mut proof_bundle_id = None;
    let mut proof_app_window_set = None;
    let mut proof_app_surface_layout = None;
    let (capture_plan, native_dimensions, proof_display_id) = match binding.target_kind() {
        RemoteDesktopTargetKind::Display => {
            let displays = unsafe { content.displays() };
            let display = select_display_for_binding(ability, &displays, binding)?;
            let proof_display_id = Some(unsafe { display.displayID() as u64 });
            let empty: Retained<NSArray<SCWindow>> = NSArray::new();
            let filter = unsafe {
                SCContentFilter::initWithDisplay_excludingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &empty,
                )
            };
            let native_dimensions = filter_dimensions_for_kind(ability, &filter, Some(&display))?;
            (
                ScreenCaptureKitCapturePlan::Single(filter),
                native_dimensions,
                proof_display_id,
            )
        }
        RemoteDesktopTargetKind::Window => {
            let windows = unsafe { content.windows() };
            let window = select_window_for_binding(ability, &windows, binding)?;
            proof_window_id = Some(unsafe { window.windowID() as u64 });
            if let Some(app) = unsafe { window.owningApplication() }.as_deref() {
                let identity = running_application_identity(app);
                proof_pid = identity.0;
                proof_app_identity = identity.1.clone();
                proof_bundle_id = identity.1;
            }
            let filter = unsafe {
                SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window)
            };
            let native_dimensions = filter_dimensions_for_kind(ability, &filter, None)?;
            (
                ScreenCaptureKitCapturePlan::Single(filter),
                native_dimensions,
                binding.native_locator().display_id(),
            )
        }
        RemoteDesktopTargetKind::Application => {
            let applications = unsafe { content.applications() };
            let app = select_application_for_binding(ability, &applications, binding)?;
            let identity = running_application_identity(&app);
            proof_pid = identity.0;
            proof_app_identity = identity.1.clone();
            proof_bundle_id = identity.1;
            let windows = unsafe { content.windows() };
            let app_window_set =
                select_application_windows_for_binding(ability, &windows, binding)?;
            proof_app_window_set = Some(app_window_set.proof);
            let multi_app = MultiAppSurfaceTarget::from_windows(ability, app_window_set.windows)?;
            proof_app_surface_layout = Some(multi_app.surface_layout_proof().clone());
            let native_dimensions = multi_app.native_dimensions();
            (
                ScreenCaptureKitCapturePlan::MultiApp(multi_app),
                native_dimensions,
                None,
            )
        }
    };
    let (native_width, native_height) = native_dimensions;
    let mut capture_proof =
        ResolvedCaptureTargetProof::new("screencapturekit", binding.target_kind())
            .with_native_identity(
                proof_display_id,
                proof_window_id,
                proof_pid,
                proof_app_identity,
                proof_bundle_id,
            )
            .with_native_dimensions(Some((native_width, native_height)));
    if let Some(app_window_set) = proof_app_window_set {
        capture_proof = capture_proof.with_app_window_set(app_window_set);
    }
    if let Some(app_surface_layout) = proof_app_surface_layout {
        capture_proof = capture_proof.with_app_surface_layout(app_surface_layout);
    }
    Ok(ScreenCaptureKitTarget {
        capture_plan,
        native_width,
        native_height,
        capture_proof,
    })
}

struct ApplicationWindowSetTarget {
    proof: AppWindowSetProof,
    windows: Vec<Retained<SCWindow>>,
}

fn select_application_windows_for_binding(
    ability: &'static str,
    windows: &NSArray<SCWindow>,
    binding: &RemoteAppTargetBinding,
) -> Result<ApplicationWindowSetTarget, RemoteAppTargetError> {
    let committed_window_set = binding.committed_app_window_set().ok_or_else(|| {
        RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetMetadataIncomplete,
            "application ScreenCaptureKit capture requires a committed application window set",
        )
    })?;
    let mut window_ids = Vec::new();
    let mut selected_windows = Vec::new();
    let mut matched_application = false;
    for window in windows.iter() {
        let Some(app) = (unsafe { window.owningApplication() }) else {
            continue;
        };
        if sck_app_matches_binding(binding, &app) {
            matched_application = true;
            let window_id = unsafe { window.windowID() as u64 };
            if !committed_window_set.contains_window_id(window_id) {
                continue;
            }
            window_ids.push(window_id);
            selected_windows.push(window);
        }
    }
    if !matched_application {
        return Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetNotFound,
            "bound application has no ScreenCaptureKit windows in the current shareable content",
        ));
    }
    let missing_window_ids = committed_window_set.missing_window_ids(&window_ids);
    if !missing_window_ids.is_empty() {
        return Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetIdentityChanged,
            format!(
                "application ScreenCaptureKit window set no longer contains committed windows; \
                missing_window_ids={missing_window_ids:?}"
            ),
        ));
    }
    if window_ids.is_empty() {
        return Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetNotFound,
            "bound application has no committed ScreenCaptureKit windows in the current shareable content",
        ));
    }
    if let Some(committed_layout) = binding.committed_app_surface_layout() {
        let mut by_id = selected_windows
            .into_iter()
            .map(|window| (unsafe { window.windowID() as u64 }, window))
            .collect::<HashMap<_, _>>();
        selected_windows = committed_layout
            .front_to_back_window_ids()
            .map(|window_id| {
                by_id.remove(&window_id).ok_or_else(|| {
                    RemoteAppTargetError::new(
                        ability,
                        TargetResolutionError::TargetIdentityChanged,
                        format!(
                            "application surface layout references missing committed window {window_id}"
                        ),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !by_id.is_empty() {
            return Err(RemoteAppTargetError::new(
                ability,
                TargetResolutionError::TargetIdentityChanged,
                "application surface layout does not cover every committed window",
            ));
        }
    }
    let proof = committed_window_set.clone();
    Ok(ApplicationWindowSetTarget {
        proof,
        windows: selected_windows,
    })
}

#[cfg_attr(not(feature = "native-media"), allow(dead_code))]
pub(in crate::daemon::plugins::remote_desktop) fn verify_target_binding_for_session(
    ability: &'static str,
    binding: &RemoteAppTargetBinding,
) -> Result<ResolvedCaptureTargetProof, RemoteAppTargetError> {
    resolve_target_for_binding(ability, binding).map(|target| target.capture_proof)
}

mod macos_screen_capture_tcc {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    pub unsafe fn preflight_screen_capture_access() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub unsafe fn request_screen_capture_access() -> bool {
        unsafe { CGRequestScreenCaptureAccess() }
    }
}

impl ScreenCaptureKitStream {
    /// Start capturing the resolved target at the requested dimensions/fps,
    /// forwarding each frame to `sink`.
    ///
    /// `fps` is clamped to >= 1. Pixel format is BGRA (the format
    /// VideoToolbox accepts directly for H.264).
    pub(in crate::daemon::plugins::remote_desktop) fn start(
        ability: &'static str,
        target: ScreenCaptureKitTarget,
        width: usize,
        height: usize,
        fps: u32,
        sinks: ScreenCaptureKitSinks,
    ) -> Result<Self, RemoteAppTargetError> {
        let start_config = ScreenCaptureKitStartConfig {
            width,
            height,
            fps: fps.max(1),
        };
        let active_generation = 1;
        let output_router = CaptureOutputRouter::new(active_generation, sinks);
        let routed_sinks = output_router.sinks_for_generation(active_generation);
        let streams =
            start_capture_plan(ability, target.capture_plan, &start_config, &routed_sinks)?;
        Ok(Self {
            streams,
            start_config,
            output_router,
            active_generation,
        })
    }

    /// Replace the complete native capture plan after a successful target
    /// re-verification. Application rebind may change the number and geometry
    /// of native streams, so filter-only mutation cannot preserve semantics.
    pub(in crate::daemon::plugins::remote_desktop) fn prepare_content_filter_update(
        &self,
        ability: &'static str,
        target: ScreenCaptureKitTarget,
    ) -> Result<PreparedScreenCaptureKitRebind, RemoteAppTargetError> {
        let generation = self.active_generation.checked_add(1).ok_or_else(|| {
            RemoteAppTargetError::new(
                ability,
                TargetResolutionError::CaptureBackendUnavailable,
                "ScreenCaptureKit output generation exhausted",
            )
        })?;
        let capture_proof = target.capture_proof().clone();
        let routed_sinks = self.output_router.sinks_for_generation(generation);
        let streams = start_capture_plan(
            ability,
            target.capture_plan,
            &self.start_config,
            &routed_sinks,
        )?;
        Ok(PreparedScreenCaptureKitRebind {
            streams: Some(streams),
            capture_proof,
            generation,
        })
    }

    /// Pause both capture generations, run the Runtime-owned binding commit,
    /// and select exactly one output generation from the result. The prepared
    /// plan has already started, so success cannot fail after Runtime state is
    /// committed; a rejected/stale commit restores the old generation and the
    /// prepared plan is stopped by Drop.
    pub(in crate::daemon::plugins::remote_desktop) fn commit_prepared_content_filter_update(
        &mut self,
        mut prepared: PreparedScreenCaptureKitRebind,
        commit_binding: impl FnOnce(&ResolvedCaptureTargetProof) -> bool,
    ) -> bool {
        let Some(replacement) = prepared.streams.take() else {
            return false;
        };
        self.output_router.select_generation(None);
        if !commit_binding(&prepared.capture_proof) {
            self.output_router
                .select_generation(Some(self.active_generation));
            stop_active_streams(&replacement);
            return false;
        }
        self.output_router
            .select_generation(Some(prepared.generation));
        let previous = std::mem::replace(&mut self.streams, replacement);
        self.active_generation = prepared.generation;
        stop_active_streams(&previous);
        true
    }

    /// Stop every stream in the active capture plan (best-effort).
    pub fn stop(&self) {
        stop_active_streams(&self.streams);
    }
}

fn stop_active_streams(streams: &[ActiveScreenCaptureKitStream]) {
    for active in streams {
        stop_capture_sync(&active.stream);
    }
}

fn start_capture_plan(
    ability: &'static str,
    capture_plan: ScreenCaptureKitCapturePlan,
    config: &ScreenCaptureKitStartConfig,
    sinks: &ScreenCaptureKitSinks,
) -> Result<Vec<ActiveScreenCaptureKitStream>, RemoteAppTargetError> {
    match capture_plan {
        ScreenCaptureKitCapturePlan::Single(filter) => Ok(vec![start_active_stream(
            ability,
            &filter,
            config.width,
            config.height,
            config.fps,
            sinks.clone(),
        )?]),
        ScreenCaptureKitCapturePlan::MultiApp(target) => {
            let surfaces = target.scale_to(ability, config.width, config.height)?;
            let compositor = MultiAppSurfaceCompositor::new(
                config.width,
                config.height,
                &surfaces,
                config.fps,
                Arc::clone(&sinks.video),
            );
            let mut streams = Vec::with_capacity(surfaces.len());
            for (surface_index, surface) in surfaces.into_iter().enumerate() {
                let compositor = Arc::clone(&compositor);
                let window_id = surface.window_id;
                let video: FrameSink = Arc::new(move |frame| {
                    if let Err(err) = compositor.accept(surface_index, frame) {
                        crate::op_event!(
                            component = remote_desktop,
                            kind = multi_app_surface_compose_failed,
                            window_id = window_id,
                            reason = err.to_string(),
                        );
                    }
                });
                let sinks = ScreenCaptureKitSinks {
                    video,
                    // ScreenCaptureKit audio is process-filtered by the first
                    // committed application window; registering it once avoids
                    // duplicate PCM delivery from every window stream.
                    audio: (surface_index == 0).then(|| sinks.audio.clone()).flatten(),
                };
                match start_active_stream(
                    ability,
                    &surface.filter,
                    surface.width,
                    surface.height,
                    config.fps,
                    sinks,
                ) {
                    Ok(stream) => streams.push(stream),
                    Err(err) => {
                        for active in &streams {
                            stop_capture_sync(&active.stream);
                        }
                        return Err(err);
                    }
                }
            }
            Ok(streams)
        }
    }
}

fn start_active_stream(
    ability: &'static str,
    filter: &SCContentFilter,
    width: usize,
    height: usize,
    fps: u32,
    sinks: ScreenCaptureKitSinks,
) -> Result<ActiveScreenCaptureKitStream, RemoteAppTargetError> {
    let config = unsafe {
        let c = SCStreamConfiguration::new();
        c.setWidth(width);
        c.setHeight(height);
        // 'BGRA' fourcc = 0x42475241.
        c.setPixelFormat(u32::from_be_bytes(*b"BGRA"));
        let fps = fps.max(1);
        c.setMinimumFrameInterval(CMTime {
            value: 1,
            timescale: fps as i32,
            flags: objc2_core_media::CMTimeFlags::Valid,
            epoch: 0,
        });
        c.setQueueDepth(SCK_LIVE_QUEUE_DEPTH);
        c.setCapturesAudio(sinks.audio.is_some());
        if sinks.audio.is_some() {
            c.setSampleRate(48_000);
            c.setChannelCount(2);
            c.setExcludesCurrentProcessAudio(true);
        }
        c
    };

    let captures_audio = sinks.audio.is_some();
    let delegate = StreamOutputDelegate::new(sinks);
    let stream = unsafe {
        SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), filter, &config, None)
    };

    // Register the output delegate on a dedicated capture queue.
    let queue = capture_queue();
    let output_proto = ProtocolObject::from_ref(&*delegate);
    unsafe {
        stream
            .addStreamOutput_type_sampleHandlerQueue_error(
                output_proto,
                SCStreamOutputType::Screen,
                Some(&queue),
            )
            .map_err(|err| {
                RemoteAppTargetError::new(
                    ability,
                    TargetResolutionError::ScreenCaptureKitFilterFailed,
                    format!(
                        "SCStream addStreamOutput failed: {}",
                        err.localizedDescription()
                    ),
                )
            })?;
        if captures_audio {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    output_proto,
                    SCStreamOutputType::Audio,
                    Some(&audio_capture_queue()),
                )
                .map_err(|err| {
                    RemoteAppTargetError::new(
                        ability,
                        TargetResolutionError::ScreenCaptureKitFilterFailed,
                        format!(
                            "SCStream add audio output failed: {}",
                            err.localizedDescription()
                        ),
                    )
                })?;
        }
    }

    start_capture_sync(ability, &stream)?;

    Ok(ActiveScreenCaptureKitStream {
        stream,
        _delegate: delegate,
    })
}

fn select_display_for_binding(
    ability: &'static str,
    displays: &NSArray<SCDisplay>,
    binding: &RemoteAppTargetBinding,
) -> Result<Retained<SCDisplay>, RemoteAppTargetError> {
    let expected_id = binding.native_locator().display_id();
    for display in displays.iter() {
        if expected_id.is_some_and(|id| unsafe { display.displayID() as u64 == id }) {
            return Ok(display);
        }
    }
    if binding.native_locator().primary_display() {
        return displays.firstObject().ok_or_else(|| {
            RemoteAppTargetError::new(
                ability,
                TargetResolutionError::TargetDisplayUnavailable,
                "no shareable primary display available",
            )
        });
    }
    if expected_id.is_some() {
        return Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::DisplayIdentityMismatch,
            "requested display identity is not available",
        ));
    }
    Err(RemoteAppTargetError::new(
        ability,
        TargetResolutionError::DisplayIdentityMissing,
        "display identity is required for ScreenCaptureKit binding",
    ))
}

fn select_window_for_binding(
    ability: &'static str,
    windows: &NSArray<SCWindow>,
    binding: &RemoteAppTargetBinding,
) -> Result<Retained<SCWindow>, RemoteAppTargetError> {
    let locator = binding.native_locator();
    let expected_id = locator.window_id();
    let expected_owner = locator.app_identity_expectation();
    let mut candidates = Vec::new();
    let mut id_seen = false;
    for window in windows.iter() {
        let id_matches = expected_id.is_some_and(|id| unsafe { window.windowID() as u64 == id });
        if !id_matches {
            continue;
        }
        id_seen = true;
        let app = unsafe { window.owningApplication() };
        if app
            .as_deref()
            .is_some_and(|app| sck_app_identity_match(expected_owner, app).matched())
        {
            candidates.push(window);
        }
    }
    match candidates.len() {
        0 if id_seen => Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetIdentityMismatch,
            "requested ScreenCaptureKit window owner identity does not match the bound target",
        )),
        0 => Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetNotFound,
            "requested ScreenCaptureKit window is no longer available",
        )),
        1 => Ok(candidates.remove(0)),
        _ => Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetIdentityAmbiguous,
            "requested ScreenCaptureKit window identity is ambiguous",
        )),
    }
}

fn select_application_for_binding(
    ability: &'static str,
    applications: &NSArray<SCRunningApplication>,
    binding: &RemoteAppTargetBinding,
) -> Result<Retained<SCRunningApplication>, RemoteAppTargetError> {
    let locator = binding.native_locator();
    let expected_owner = locator.app_identity_expectation();
    let mut candidates = Vec::new();
    let mut identity_seen = false;
    for app in applications.iter() {
        let result = sck_app_identity_match(expected_owner, &app);
        identity_seen |= result.any_expected_field_seen();
        if result.matched() {
            candidates.push(app);
        }
    }
    match candidates.len() {
        0 if identity_seen => Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetIdentityMismatch,
            "requested ScreenCaptureKit application metadata no longer matches the bound target",
        )),
        0 => Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetNotFound,
            "requested ScreenCaptureKit application is no longer available",
        )),
        1 => Ok(candidates.remove(0)),
        _ => Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetIdentityAmbiguous,
            "requested ScreenCaptureKit application identity is ambiguous",
        )),
    }
}

fn sck_app_matches_binding(binding: &RemoteAppTargetBinding, app: &SCRunningApplication) -> bool {
    sck_app_identity_match(binding.native_locator().app_identity_expectation(), app).matched()
}

fn sck_app_identity_match(
    expected: NativeAppIdentityExpectation<'_>,
    app: &SCRunningApplication,
) -> NativeAppIdentityMatch {
    let bundle_id = unsafe { app.bundleIdentifier() }.to_string();
    let bundle_id = bundle_id.trim();
    let bundle_id = (!bundle_id.is_empty()).then_some(bundle_id);
    expected.evaluate(NativeAppIdentityCandidate::new(
        Some(unsafe { app.processID() as i64 }),
        bundle_id,
        bundle_id,
    ))
}

fn filter_dimensions_for_kind(
    ability: &'static str,
    filter: &SCContentFilter,
    display: Option<&SCDisplay>,
) -> Result<(usize, usize), RemoteAppTargetError> {
    if let Some(fallback_display) = display {
        let info = unsafe { SCShareableContent::infoForFilter(filter) };
        let scale = f64::from(unsafe { info.pointPixelScale() }.max(1.0));
        let width = unsafe { fallback_display.width() };
        let height = unsafe { fallback_display.height() };
        return positive_dimensions(ability, width as f64 * scale, height as f64 * scale);
    }
    let info = unsafe { SCShareableContent::infoForFilter(filter) };
    let rect = unsafe { info.contentRect() };
    let scale = f64::from(unsafe { info.pointPixelScale() }.max(1.0));
    positive_dimensions(ability, rect.size.width * scale, rect.size.height * scale)
}

fn positive_dimensions(
    ability: &'static str,
    width: f64,
    height: f64,
) -> Result<(usize, usize), RemoteAppTargetError> {
    if width <= 0.0 || height <= 0.0 {
        return Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::ScreenCaptureKitFilterFailed,
            format!("ScreenCaptureKit target returned invalid dimensions {width}x{height}"),
        ));
    }
    Ok((
        width.round().max(2.0) as usize,
        height.round().max(2.0) as usize,
    ))
}

fn running_application_identity(app: &SCRunningApplication) -> (Option<i64>, Option<String>) {
    let pid = Some(unsafe { app.processID() as i64 });
    let bundle_id = Some(unsafe { app.bundleIdentifier() }.to_string())
        .filter(|value| !value.trim().is_empty());
    (pid, bundle_id)
}

impl Drop for ScreenCaptureKitStream {
    fn drop(&mut self) {
        self.stop();
    }
}

fn start_capture_sync(
    ability: &'static str,
    stream: &SCStream,
) -> Result<(), RemoteAppTargetError> {
    let (tx, rx) = sync_channel::<Result<(), String>>(1);
    let tx = Mutex::new(Some(tx));
    let handler = RcBlock::new(move |error: *mut NSError| {
        let result = if error.is_null() {
            Ok(())
        } else {
            let err = unsafe { &*error };
            Err(format!(
                "startCapture error: {}",
                err.localizedDescription()
            ))
        };
        if let Some(tx) = take_completion_sender(&tx) {
            let _ = tx.send(result);
        }
    });
    unsafe {
        stream.startCaptureWithCompletionHandler(Some(&handler));
    }
    match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(msg)) => Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::ScreenCaptureKitStreamStartFailed,
            msg,
        )),
        Err(_) => Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::ScreenCaptureKitStreamStartFailed,
            "SCStream startCapture timed out after 10s",
        )),
    }
}

fn stop_capture_sync(stream: &SCStream) {
    let (tx, rx) = sync_channel::<()>(1);
    let tx = Mutex::new(Some(tx));
    let handler = RcBlock::new(move |_error: *mut NSError| {
        if let Some(tx) = take_completion_sender(&tx) {
            let _ = tx.send(());
        }
    });
    unsafe {
        stream.stopCaptureWithCompletionHandler(Some(&handler));
    }
    let _ = rx.recv_timeout(Duration::from_secs(3));
}

fn take_completion_sender<T>(slot: &Mutex<Option<SyncSender<T>>>) -> Option<SyncSender<T>> {
    match slot.lock() {
        Ok(mut guard) => guard.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

/// A serial dispatch queue for capture callbacks.
fn capture_queue() -> DispatchRetained<DispatchQueue> {
    // dispatch_queue_create via the dispatch2 crate; a serial queue keeps
    // sample ordering and avoids re-entrant callbacks.
    DispatchQueue::new("tech.easynet.remote-desktop.capture", None)
}

fn audio_capture_queue() -> DispatchRetained<DispatchQueue> {
    DispatchQueue::new("tech.easynet.remote-desktop.audio-capture", None)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[test]
    fn application_capture_uses_desktop_independent_committed_window_streams() {
        let source = include_str!("screencapturekit_capture.rs");
        let production = source
            .split("\n#[cfg(test)]")
            .next()
            .expect("production source exists");
        assert!(
            production.contains("ScreenCaptureKitCapturePlan::MultiApp"),
            "application capture must resolve a multi-surface plan"
        );
        assert!(
            production.contains("select_application_windows_for_binding"),
            "application capture must select the exact committed window set"
        );
        assert!(
            !production.contains(concat!(
                "initWithDisplay_",
                "includingApplications_exceptingWindows"
            )),
            "application capture must not remain display-scoped"
        );
    }

    #[test]
    fn application_selector_excludes_uncommitted_windows_without_display_fallback() {
        let source = include_str!("screencapturekit_capture.rs");
        let production = source
            .split("\n#[cfg(test)]")
            .next()
            .expect("production source exists");
        assert!(
            production.contains("!committed_window_set.contains_window_id(window_id)")
                && production.contains("selected_windows.push(window)"),
            "only committed owner-matched windows may enter the capture plan"
        );
        assert!(
            !production.contains("sck_window_overlaps_display")
                && !production.contains("TargetMultiDisplayUnsupported"),
            "macOS AppSurface selection must not reject or crop cross-display windows"
        );
    }

    #[test]
    fn output_router_isolates_prepared_and_stale_capture_generations() {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let audio_observed = Arc::clone(&observed);
        let sinks = ScreenCaptureKitSinks {
            video: Arc::new(|_| {}),
            audio: Some(Arc::new(move |event| {
                audio_observed
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push(event.expect_err("test route uses diagnostic errors"));
            })),
        };
        let router = CaptureOutputRouter::new(7, sinks);
        let active = router.sinks_for_generation(7).audio.expect("audio route");
        let prepared = router.sinks_for_generation(8).audio.expect("audio route");

        active(Err("old-active".into()));
        prepared(Err("prepared-muted".into()));
        router.select_generation(None);
        active(Err("old-paused".into()));
        prepared(Err("prepared-paused".into()));
        router.select_generation(Some(8));
        active(Err("old-stale".into()));
        prepared(Err("new-active".into()));

        assert_eq!(
            *observed
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            vec!["old-active".to_string(), "new-active".to_string()]
        );
    }
}
