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

use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_core_foundation::CGRect;
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{
    kCVPixelFormatType_32BGRA, kCVReturnSuccess, CVImageBuffer, CVPixelBufferGetBaseAddress,
    CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCRunningApplication, SCShareableContent, SCStream,
    SCStreamConfiguration, SCStreamOutput, SCStreamOutputType, SCWindow,
};

use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
    bgra_bytes_to_rgb_frame, rgb_frame_to_jpeg, EncodedFrame, ScreenCaptureOptions,
};
use crate::daemon::plugins::remote_desktop::target::{
    AppWindowSetProof, RemoteAppTargetBinding, RemoteAppTargetError, RemoteDesktopTargetKind,
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

/// Holds the capture stream + delegate alive for the session lifetime.
pub struct ScreenCaptureKitStream {
    stream: Retained<SCStream>,
    _delegate: Retained<StreamOutputDelegate>,
}

/// A resolved ScreenCaptureKit target. It owns the platform content filter and
/// exposes the native dimensions that should be used when the ability requests
/// `resolution=native`.
pub struct ScreenCaptureKitTarget {
    filter: Retained<SCContentFilter>,
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
    sink: FrameSink,
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
            if output_type != SCStreamOutputType::Screen {
                return;
            }
            // Only forward samples that carry a ready image buffer.
            let Some(image_buffer) = (unsafe { sample_buffer.image_buffer() }) else {
                return;
            };
            let pts = unsafe { sample_buffer.presentation_time_stamp() };
            (self.ivars().sink)(CapturedFrame {
                image_buffer: image_buffer.into(),
                pts,
            });
        }
    }
);

impl StreamOutputDelegate {
    fn new(sink: FrameSink) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { sink });
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
    let stream = ScreenCaptureKitStream::start(ability, target, width, height, options.fps, sink)?;
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
    let (filter, proof_display_id, selected_display) = match binding.target_kind() {
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
            (filter, proof_display_id, Some(display))
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
            (filter, binding.native_locator().display_id(), None)
        }
        RemoteDesktopTargetKind::Application => {
            let displays = unsafe { content.displays() };
            let display = select_display_for_binding(ability, &displays, binding)?;
            let proof_display_id = Some(unsafe { display.displayID() as u64 });
            let applications = unsafe { content.applications() };
            let app = select_application_for_binding(ability, &applications, binding)?;
            let identity = running_application_identity(&app);
            proof_pid = identity.0;
            proof_app_identity = identity.1.clone();
            proof_bundle_id = identity.1;
            let windows = unsafe { content.windows() };
            let app_window_set =
                select_application_window_set_for_binding(ability, &windows, binding, &display)?;
            let window_refs = app_window_set
                .windows
                .iter()
                .map(|window| window.as_ref())
                .collect::<Vec<_>>();
            let included_windows = NSArray::from_slice(&window_refs);
            proof_app_window_set = Some(app_window_set.proof);
            let filter = unsafe {
                SCContentFilter::initWithDisplay_includingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &included_windows,
                )
            };
            (filter, proof_display_id, None)
        }
    };
    let (native_width, native_height) = filter_dimensions_for_kind(
        ability,
        &filter,
        binding.target_kind(),
        selected_display.as_deref(),
    )?;
    let mut capture_proof = ResolvedCaptureTargetProof::new(
        "screencapturekit",
        binding.target_kind(),
        proof_display_id,
        proof_window_id,
        proof_pid,
        proof_app_identity,
        proof_bundle_id,
        Some((native_width, native_height)),
    );
    if let Some(app_window_set) = proof_app_window_set {
        capture_proof = capture_proof.with_app_window_set(app_window_set);
    }
    Ok(ScreenCaptureKitTarget {
        filter,
        native_width,
        native_height,
        capture_proof,
    })
}

struct ApplicationWindowSetTarget {
    windows: Vec<Retained<SCWindow>>,
    proof: AppWindowSetProof,
}

fn select_application_window_set_for_binding(
    ability: &'static str,
    windows: &NSArray<SCWindow>,
    binding: &RemoteAppTargetBinding,
    display: &SCDisplay,
) -> Result<ApplicationWindowSetTarget, RemoteAppTargetError> {
    let locator = binding.native_locator();
    let display_id = locator.display_id().ok_or_else(|| {
        RemoteAppTargetError::new(
            ability,
            TargetResolutionError::DisplayIdentityMissing,
            "application ScreenCaptureKit proof requires a display-scoped binding",
        )
    })?;
    let mut selected_windows = Vec::new();
    let mut window_ids = Vec::new();
    let mut off_display_window_ids = Vec::new();
    for window in windows.iter() {
        let Some(app) = (unsafe { window.owningApplication() }) else {
            continue;
        };
        if sck_app_matches_binding(
            locator.pid(),
            locator.bundle_id(),
            locator.app_identity(),
            &app,
        ) {
            let window_id = unsafe { window.windowID() as u64 };
            if sck_window_overlaps_display(&window, display) {
                window_ids.push(window_id);
                selected_windows.push(window);
            } else {
                off_display_window_ids.push(window_id);
            }
        }
    }
    if !off_display_window_ids.is_empty() {
        off_display_window_ids.sort_unstable();
        off_display_window_ids.dedup();
        return Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetMultiDisplayUnsupported,
            format!(
                "application target spans windows outside display {display_id}; \
                 multi-display application capture requires MultiAppSurface support; \
                 off_display_window_ids={off_display_window_ids:?}"
            ),
        ));
    }
    if window_ids.is_empty() {
        return Err(RemoteAppTargetError::new(
            ability,
            TargetResolutionError::TargetNotFound,
            "bound application has no ScreenCaptureKit windows in the current shareable content",
        ));
    }
    let proof = AppWindowSetProof::new(
        display_id,
        locator.bundle_id().map(str::to_string),
        locator.pid(),
        window_ids,
    );
    Ok(ApplicationWindowSetTarget {
        windows: selected_windows,
        proof,
    })
}

fn sck_window_overlaps_display(window: &SCWindow, display: &SCDisplay) -> bool {
    rects_overlap(unsafe { window.frame() }, unsafe { display.frame() })
}

fn rects_overlap(a: CGRect, b: CGRect) -> bool {
    let a_min_x = a.origin.x;
    let a_min_y = a.origin.y;
    let a_max_x = a.origin.x + a.size.width;
    let a_max_y = a.origin.y + a.size.height;
    let b_min_x = b.origin.x;
    let b_min_y = b.origin.y;
    let b_max_x = b.origin.x + b.size.width;
    let b_max_y = b.origin.y + b.size.height;

    a.size.width > 0.0
        && a.size.height > 0.0
        && b.size.width > 0.0
        && b.size.height > 0.0
        && a_min_x < b_max_x
        && a_max_x > b_min_x
        && a_min_y < b_max_y
        && a_max_y > b_min_y
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
        sink: FrameSink,
    ) -> Result<Self, RemoteAppTargetError> {
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
            c
        };

        let delegate = StreamOutputDelegate::new(sink);
        let stream = unsafe {
            SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &target.filter,
                &config,
                None,
            )
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
        }

        start_capture_sync(ability, &stream)?;

        Ok(Self {
            stream,
            _delegate: delegate,
        })
    }

    /// Stop the capture stream (best-effort; errors are logged, not fatal).
    pub fn stop(&self) {
        let (tx, rx) = sync_channel::<()>(1);
        let tx = Mutex::new(Some(tx));
        let handler = RcBlock::new(move |_error: *mut NSError| {
            if let Some(tx) = take_completion_sender(&tx) {
                let _ = tx.send(());
            }
        });
        unsafe {
            self.stream.stopCaptureWithCompletionHandler(Some(&handler));
        }
        let _ = rx.recv_timeout(Duration::from_secs(3));
    }
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
    let expected_pid = locator.pid();
    let expected_app_identity = locator.app_identity();
    let expected_bundle_id = locator.bundle_id();
    let mut candidates = Vec::new();
    let mut id_seen = false;
    for window in windows.iter() {
        let id_matches = expected_id.is_some_and(|id| unsafe { window.windowID() as u64 == id });
        if !id_matches {
            continue;
        }
        id_seen = true;
        let app = unsafe { window.owningApplication() };
        let pid_matches = expected_pid.map(|pid| {
            app.as_deref()
                .map(|app| unsafe { app.processID() as i64 == pid })
                .unwrap_or(false)
        });
        let bundle_matches = expected_bundle_id.map(|bundle_id| {
            app.as_deref()
                .map(|app| ns_string_eq(unsafe { app.bundleIdentifier() }.as_ref(), bundle_id))
                .unwrap_or(false)
        });
        let app_identity_matches = expected_app_identity.map(|app_identity| {
            app.as_deref()
                .map(|app| ns_string_eq(unsafe { app.bundleIdentifier() }.as_ref(), app_identity))
                .unwrap_or(false)
        });
        let owner_matches = pid_matches.unwrap_or(true)
            && bundle_matches.unwrap_or(true)
            && app_identity_matches.unwrap_or(true);
        if owner_matches {
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
    let expected_pid = locator.pid();
    let expected_app_identity = locator.app_identity();
    let expected_bundle_id = locator.bundle_id();
    let mut candidates = Vec::new();
    let mut identity_seen = false;
    for app in applications.iter() {
        let pid_matches = expected_pid
            .map(|pid| unsafe { app.processID() as i64 == pid })
            .unwrap_or(true);
        if expected_pid.is_some() && pid_matches {
            identity_seen = true;
        }
        let bundle_matches = expected_bundle_id
            .map(|bundle_id| ns_string_eq(unsafe { app.bundleIdentifier() }.as_ref(), bundle_id))
            .unwrap_or(true);
        if expected_bundle_id.is_some() && bundle_matches {
            identity_seen = true;
        }
        let app_identity_matches = expected_app_identity
            .map(|app_identity| {
                ns_string_eq(unsafe { app.bundleIdentifier() }.as_ref(), app_identity)
            })
            .unwrap_or(true);
        if expected_app_identity.is_some() && app_identity_matches {
            identity_seen = true;
        }
        if pid_matches && bundle_matches && app_identity_matches {
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

fn sck_app_matches_binding(
    expected_pid: Option<i64>,
    expected_bundle_id: Option<&str>,
    expected_app_identity: Option<&str>,
    app: &SCRunningApplication,
) -> bool {
    expected_pid.is_none_or(|pid| unsafe { app.processID() as i64 == pid })
        && expected_bundle_id.is_none_or(|bundle_id| {
            ns_string_eq(unsafe { app.bundleIdentifier() }.as_ref(), bundle_id)
        })
        && expected_app_identity.is_none_or(|app_identity| {
            ns_string_eq(unsafe { app.bundleIdentifier() }.as_ref(), app_identity)
        })
}

fn filter_dimensions_for_kind(
    ability: &'static str,
    filter: &SCContentFilter,
    target_kind: RemoteDesktopTargetKind,
    display: Option<&SCDisplay>,
) -> Result<(usize, usize), RemoteAppTargetError> {
    if target_kind == RemoteDesktopTargetKind::Display {
        let fallback_display = display.ok_or_else(|| {
            RemoteAppTargetError::new(
                ability,
                TargetResolutionError::DisplayIdentityMissing,
                "display dimensions require a resolved ScreenCaptureKit display",
            )
        })?;
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

fn ns_string_eq(value: &NSString, expected: &str) -> bool {
    value.to_string() == expected
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
