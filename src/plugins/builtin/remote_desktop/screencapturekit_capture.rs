// EasyNet CLI — ScreenCaptureKit display capture
// ===============================================
//
// File: src/plugins/builtin/remote_desktop/screencapturekit_capture.rs
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
// preflights and requests that native permission before enumerating
// SCShareableContent so callers get a deterministic permission error instead
// of an opaque stalled stream.
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
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::CVImageBuffer;
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol, NSString};
use objc2_screen_capture_kit::{
    SCContentFilter, SCDisplay, SCRunningApplication, SCShareableContent, SCStream,
    SCStreamConfiguration, SCStreamOutput, SCStreamOutputType, SCWindow,
};
use serde_json::Value;

use crate::persistence::resources::{ResourceEntry, ResourceType};

const REASON_PERMISSION_DENIED: &str = "permission_denied";

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
/// exposes the native dimensions that should be used when Axon requests
/// `resolution=native`.
pub struct ScreenCaptureKitTarget {
    filter: Retained<SCContentFilter>,
    native_width: usize,
    native_height: usize,
}

impl ScreenCaptureKitTarget {
    pub fn native_dimensions(&self) -> (usize, usize) {
        (self.native_width, self.native_height)
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
fn shareable_content() -> anyhow::Result<Retained<SCShareableContent>> {
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
        Ok(Err(msg)) => anyhow::bail!("{msg}"),
        Err(_) => anyhow::bail!("SCShareableContent enumeration timed out"),
    }
}

pub fn screen_capture_permission_granted() -> bool {
    unsafe { macos_screen_capture_tcc::preflight_screen_capture_access() }
}

pub fn request_screen_capture_permission() -> bool {
    unsafe { macos_screen_capture_tcc::request_screen_capture_access() }
}

fn ensure_screen_capture_permission() -> anyhow::Result<()> {
    if screen_capture_permission_granted() || request_screen_capture_permission() {
        return Ok(());
    }
    anyhow::bail!(
        "macOS Screen Recording permission is not granted for {}; \
         open System Settings > Privacy & Security > Screen & System Audio Recording, \
         grant access to this binary, then restart the daemon; reason={REASON_PERMISSION_DENIED}",
        std::env::current_exe()
            .ok()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "the EasyNet daemon process".to_string())
    )
}

pub fn target_for_entry(entry: &ResourceEntry) -> anyhow::Result<ScreenCaptureKitTarget> {
    ensure_screen_capture_permission()?;
    let content = shareable_content()?;
    let displays = unsafe { content.displays() };
    let display = select_display(&displays, entry)?;
    let filter = match entry.kind {
        ResourceType::Display => {
            let empty: Retained<NSArray<SCWindow>> = NSArray::new();
            unsafe {
                SCContentFilter::initWithDisplay_excludingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &empty,
                )
            }
        }
        ResourceType::Window => {
            let windows = unsafe { content.windows() };
            let window = select_window(&windows, entry)?;
            unsafe {
                SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window)
            }
        }
        ResourceType::Application => {
            let applications = unsafe { content.applications() };
            let app = select_application(&applications, entry)?;
            let apps = NSArray::from_slice(&[&*app]);
            let empty: Retained<NSArray<SCWindow>> = NSArray::new();
            unsafe {
                SCContentFilter::initWithDisplay_includingApplications_exceptingWindows(
                    SCContentFilter::alloc(),
                    &display,
                    &apps,
                    &empty,
                )
            }
        }
        _ => anyhow::bail!("ScreenCaptureKit target must be display/window/application"),
    };
    let (native_width, native_height) = filter_dimensions(&filter, entry, &display)?;
    Ok(ScreenCaptureKitTarget {
        filter,
        native_width,
        native_height,
    })
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
    pub fn start(
        target: ScreenCaptureKitTarget,
        width: usize,
        height: usize,
        fps: u32,
        sink: FrameSink,
    ) -> anyhow::Result<Self> {
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
                    anyhow::anyhow!(
                        "SCStream addStreamOutput failed: {}",
                        err.localizedDescription()
                    )
                })?;
        }

        start_capture_sync(&stream)?;

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

fn select_display(
    displays: &NSArray<SCDisplay>,
    entry: &ResourceEntry,
) -> anyhow::Result<Retained<SCDisplay>> {
    let expected_id = entry.metadata.get("monitor_id").and_then(Value::as_u64);
    for display in displays.iter() {
        if expected_id.is_some_and(|id| unsafe { display.displayID() as u64 == id }) {
            return Ok(display);
        }
    }
    displays
        .firstObject()
        .ok_or_else(|| anyhow::anyhow!("no shareable display available"))
}

fn select_window(
    windows: &NSArray<SCWindow>,
    entry: &ResourceEntry,
) -> anyhow::Result<Retained<SCWindow>> {
    let expected_id = entry.metadata.get("window_id").and_then(Value::as_u64);
    let expected_pid = entry.metadata.get("pid").and_then(Value::as_i64);
    let expected_title = entry.metadata.get("title").and_then(Value::as_str);
    let expected_app = entry.metadata.get("app_name").and_then(Value::as_str);
    for window in windows.iter() {
        if expected_id.is_some_and(|id| unsafe { window.windowID() as u64 == id }) {
            return Ok(window);
        }
        let app = unsafe { window.owningApplication() };
        let pid_matches = expected_pid.is_some_and(|pid| {
            app.as_deref()
                .map(|app| unsafe { app.processID() as i64 == pid })
                .unwrap_or(false)
        });
        let app_matches = expected_app.is_some_and(|name| {
            app.as_deref()
                .map(|app| ns_string_eq(unsafe { app.applicationName() }.as_ref(), name))
                .unwrap_or(false)
        });
        let title_matches = expected_title.is_some_and(|title| {
            unsafe { window.title() }
                .as_deref()
                .map(|actual| ns_string_eq(actual, title))
                .unwrap_or(false)
        });
        if pid_matches && app_matches && title_matches {
            return Ok(window);
        }
    }
    anyhow::bail!("requested ScreenCaptureKit window is no longer available")
}

fn select_application(
    applications: &NSArray<SCRunningApplication>,
    entry: &ResourceEntry,
) -> anyhow::Result<Retained<SCRunningApplication>> {
    let expected_pid = entry.metadata.get("primary_pid").and_then(Value::as_i64);
    let expected_app = entry.metadata.get("app_name").and_then(Value::as_str);
    for app in applications.iter() {
        if expected_pid.is_some_and(|pid| unsafe { app.processID() as i64 == pid }) {
            return Ok(app);
        }
        if expected_app
            .is_some_and(|name| ns_string_eq(unsafe { app.applicationName() }.as_ref(), name))
        {
            return Ok(app);
        }
    }
    anyhow::bail!("requested ScreenCaptureKit application is no longer available")
}

fn filter_dimensions(
    filter: &SCContentFilter,
    entry: &ResourceEntry,
    fallback_display: &SCDisplay,
) -> anyhow::Result<(usize, usize)> {
    if entry.kind == ResourceType::Display {
        let width = unsafe { fallback_display.width() };
        let height = unsafe { fallback_display.height() };
        return positive_dimensions(width as f64, height as f64);
    }
    let info = unsafe { SCShareableContent::infoForFilter(filter) };
    let rect = unsafe { info.contentRect() };
    let scale = f64::from(unsafe { info.pointPixelScale() }.max(1.0));
    positive_dimensions(rect.size.width * scale, rect.size.height * scale)
}

fn positive_dimensions(width: f64, height: f64) -> anyhow::Result<(usize, usize)> {
    if width <= 0.0 || height <= 0.0 {
        anyhow::bail!("ScreenCaptureKit target returned invalid dimensions {width}x{height}");
    }
    Ok((
        width.round().max(2.0) as usize,
        height.round().max(2.0) as usize,
    ))
}

fn ns_string_eq(value: &NSString, expected: &str) -> bool {
    value.to_string() == expected
}

impl Drop for ScreenCaptureKitStream {
    fn drop(&mut self) {
        self.stop();
    }
}

fn start_capture_sync(stream: &SCStream) -> anyhow::Result<()> {
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
        Ok(Err(msg)) => anyhow::bail!("{msg}"),
        Err(_) => anyhow::bail!("SCStream startCapture timed out (screen recording permission?)"),
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
