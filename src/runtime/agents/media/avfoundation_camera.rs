// EasyNet CLI — native macOS camera capture
// =========================================
//
// File: src/runtime/agents/media/avfoundation_camera.rs
// Description: AVFoundation one-shot camera capture for camera.snapshot.
//
// Why this exists:
// - macOS camera permission is a TCC contract tied to the requesting process.
//   Shelling out to helper binaries makes the helper the requester and often
//   cannot surface the system prompt from the daemon path.
// - AVFoundation is the native API that owns both authorization and capture.
//
// Architectural Position:
// - EasyNet-Cli device adapter, native media plugin (macOS only).

#![cfg(target_os = "macos")]

use std::ptr;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, define_class, msg_send, AnyThread, DefinedClass};
use objc2_core_video::{
    kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA, kCVReturnSuccess,
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{
    NSArray, NSError, NSMutableDictionary, NSNumber, NSObject, NSObjectProtocol, NSString,
};

use crate::persistence::resources::ResourceEntry;
use crate::runtime::agents::media::camera_snapshot::{
    EncodedFrame, REASON_PERMISSION_DENIED, REASON_RESOURCE_UNAVAILABLE,
};
use crate::runtime::agents::media_abilities::ABILITY_CAMERA_SNAPSHOT;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}

const AV_MEDIA_TYPE_VIDEO: &str = "vide";
const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(30);
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(6);
const EXPOSURE_SETTLE_DURATION: Duration = Duration::from_millis(1500);

// AVAuthorizationStatus.
const AUTH_NOT_DETERMINED: isize = 0;
const AUTH_RESTRICTED: isize = 1;
const AUTH_DENIED: isize = 2;
const AUTH_AUTHORIZED: isize = 3;

struct VideoFrameDelegateIvars {
    sender: Mutex<Option<SyncSender<anyhow::Result<EncodedFrame>>>>,
    started_at: Instant,
    best_frame: Mutex<Option<FrameCandidate>>,
}

#[derive(Debug, Clone)]
struct FrameCandidate {
    frame: EncodedFrame,
    luma_mean: f64,
    luma_p99: u8,
}

impl FrameCandidate {
    fn score(&self) -> f64 {
        self.luma_mean + f64::from(self.luma_p99) * 0.25
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "EasyNetAVFoundationVideoFrameDelegate"]
    #[ivars = VideoFrameDelegateIvars]
    struct VideoFrameDelegate;

    impl VideoFrameDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        unsafe fn capture_output(
            &self,
            _output: &AnyObject,
            sample_buffer: &objc2_core_media::CMSampleBuffer,
            _connection: &AnyObject,
        ) {
            let candidate = match unsafe { sample_buffer.image_buffer() } {
                Some(image_buffer) => encode_bgra_pixel_buffer_as_jpeg(&image_buffer),
                None => Err(anyhow::anyhow!(
                    "{ABILITY_CAMERA_SNAPSHOT}: AVFoundation sample contained no image buffer; \
                     reason={REASON_RESOURCE_UNAVAILABLE}"
                )),
            };

            match candidate {
                Ok(candidate) => {
                    remember_best_frame(&self.ivars().best_frame, candidate);
                    if self.ivars().started_at.elapsed() >= EXPOSURE_SETTLE_DURATION {
                        if let Some(sender) = take_completion_sender(&self.ivars().sender) {
                            let frame = self
                                .ivars()
                                .best_frame
                                .lock()
                                .map(|mut guard| guard.take())
                                .unwrap_or_else(|poisoned| poisoned.into_inner().take())
                                .map(|candidate| candidate.frame)
                                .ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "{ABILITY_CAMERA_SNAPSHOT}: AVFoundation did not retain a camera frame; \
                                         reason={REASON_RESOURCE_UNAVAILABLE}"
                                    )
                                });
                            let _ = sender.send(frame);
                        }
                    }
                }
                Err(err) => {
                    if frame_count(&self.ivars().best_frame) == 0 {
                        if let Some(sender) = take_completion_sender(&self.ivars().sender) {
                            let _ = sender.send(Err(err));
                        }
                    }
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for VideoFrameDelegate {}
);

impl VideoFrameDelegate {
    fn new(sender: SyncSender<anyhow::Result<EncodedFrame>>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(VideoFrameDelegateIvars {
            sender: Mutex::new(Some(sender)),
            started_at: Instant::now(),
            best_frame: Mutex::new(None),
        });
        unsafe { msg_send![super(this), init] }
    }
}

pub fn capture_jpeg(entry: &ResourceEntry) -> anyhow::Result<EncodedFrame> {
    ensure_camera_authorized()?;

    let media_type = NSString::from_str(AV_MEDIA_TYPE_VIDEO);
    let device = select_camera_device(&media_type, entry)?;
    let input = device_input(&device)?;

    let session: Retained<AnyObject> = unsafe { msg_send![class!(AVCaptureSession), new] };
    let output: Retained<AnyObject> = unsafe { msg_send![class!(AVCaptureVideoDataOutput), new] };
    let settings = bgra_video_settings();
    let (tx, rx) = sync_channel::<anyhow::Result<EncodedFrame>>(1);
    let delegate = VideoFrameDelegate::new(tx);
    let queue = capture_queue();

    unsafe {
        let _: () = msg_send![&*session, beginConfiguration];

        // AVCaptureSession defaults to PresetHigh — 720p VIDEO on
        // FaceTime HD cameras. camera.snapshot is a STILL: ask for
        // the photo preset, which delivers the sensor's full still
        // resolution (1080p+ on MacBook cameras). Guarded fall-back
        // to the default for devices that can't do photo (some
        // external UVC cams).
        let photo_preset = NSString::from_str("AVCaptureSessionPresetPhoto");
        let can_set_preset: bool = msg_send![&*session, canSetSessionPreset: &*photo_preset];
        if can_set_preset {
            let _: () = msg_send![&*session, setSessionPreset: &*photo_preset];
        }

        let can_add_input: bool = msg_send![&*session, canAddInput: &*input];
        if !can_add_input {
            anyhow::bail!(
                "{ABILITY_CAMERA_SNAPSHOT}: AVFoundation cannot add camera input; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            );
        }
        let _: () = msg_send![&*session, addInput: &*input];

        let _: () = msg_send![&*output, setAlwaysDiscardsLateVideoFrames: true];
        let _: () = msg_send![&*output, setVideoSettings: &*settings];
        let _: () = msg_send![&*output, setSampleBufferDelegate: &*delegate, queue: &*queue];

        let can_add_output: bool = msg_send![&*session, canAddOutput: &*output];
        if !can_add_output {
            anyhow::bail!(
                "{ABILITY_CAMERA_SNAPSHOT}: AVFoundation cannot add video output; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            );
        }
        let _: () = msg_send![&*session, addOutput: &*output];
        let _: () = msg_send![&*session, commitConfiguration];
        let _: () = msg_send![&*session, startRunning];
    }

    let frame = match rx.recv_timeout(FIRST_FRAME_TIMEOUT) {
        Ok(Ok(frame)) => Ok(frame),
        Ok(Err(err)) => Err(err),
        Err(_) => Err(anyhow::anyhow!(
            "{ABILITY_CAMERA_SNAPSHOT}: AVFoundation did not deliver a camera frame within {}ms; \
             reason={REASON_RESOURCE_UNAVAILABLE}",
            FIRST_FRAME_TIMEOUT.as_millis()
        )),
    };

    unsafe {
        let _: () = msg_send![&*session, stopRunning];
        let _: () = msg_send![
            &*output,
            setSampleBufferDelegate: ptr::null::<AnyObject>(),
            queue: ptr::null::<AnyObject>()
        ];
    }

    frame
}

fn ensure_camera_authorized() -> anyhow::Result<()> {
    let media_type = NSString::from_str(AV_MEDIA_TYPE_VIDEO);
    let status: isize = unsafe {
        msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: &*media_type]
    };

    match status {
        AUTH_AUTHORIZED => Ok(()),
        AUTH_DENIED | AUTH_RESTRICTED => anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: macOS camera access is denied or restricted; \
             reason={REASON_PERMISSION_DENIED}"
        ),
        AUTH_NOT_DETERMINED => request_camera_access(&media_type),
        other => anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: unknown AVFoundation camera authorization status {other}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        ),
    }
}

fn request_camera_access(media_type: &NSString) -> anyhow::Result<()> {
    let (tx, rx) = sync_channel::<bool>(1);
    let tx = Mutex::new(Some(tx));
    let handler = RcBlock::new(move |granted: Bool| {
        if let Some(tx) = take_completion_sender(&tx) {
            let _ = tx.send(granted.as_bool());
        }
    });

    unsafe {
        let _: () = msg_send![
            class!(AVCaptureDevice),
            requestAccessForMediaType: media_type,
            completionHandler: &*handler
        ];
    }

    match rx.recv_timeout(AUTHORIZATION_TIMEOUT) {
        Ok(true) => Ok(()),
        Ok(false) => anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: macOS camera access was not granted; \
             reason={REASON_PERMISSION_DENIED}"
        ),
        Err(_) => anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: timed out waiting for macOS camera authorization; \
             reason={REASON_PERMISSION_DENIED}"
        ),
    }
}

fn select_camera_device(
    media_type: &NSString,
    entry: &ResourceEntry,
) -> anyhow::Result<Retained<AnyObject>> {
    let devices: Retained<NSArray<AnyObject>> =
        unsafe { msg_send![class!(AVCaptureDevice), devicesWithMediaType: media_type] };
    if devices.is_empty() {
        anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: AVFoundation returned no video capture devices; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }

    if let Some(index) = entry.metadata.get("camera_index").and_then(|v| v.as_u64()) {
        let index = index as usize;
        if index < devices.len() {
            return Ok(devices.objectAtIndex(index));
        }
        anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: requested camera_index={index} but AVFoundation only \
             reported {} device(s); reason={REASON_RESOURCE_UNAVAILABLE}",
            devices.len()
        );
    }

    if !entry.display_name.is_empty() {
        for device in devices.iter() {
            let name: Retained<NSString> = unsafe { msg_send![&*device, localizedName] };
            if name.to_string() == entry.display_name {
                return Ok(device);
            }
        }
    }

    Ok(devices.objectAtIndex(0))
}

fn device_input(device: &AnyObject) -> anyhow::Result<Retained<AnyObject>> {
    let mut error: *mut NSError = ptr::null_mut();
    let input: Option<Retained<AnyObject>> = unsafe {
        msg_send![
            class!(AVCaptureDeviceInput),
            deviceInputWithDevice: device,
            error: &mut error
        ]
    };
    input.ok_or_else(|| {
        let msg = if error.is_null() {
            "unknown error".to_string()
        } else {
            unsafe { &*error }.localizedDescription().to_string()
        };
        anyhow::anyhow!(
            "{ABILITY_CAMERA_SNAPSHOT}: AVCaptureDeviceInput failed: {msg}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })
}

fn bgra_video_settings() -> Retained<NSMutableDictionary<AnyObject, AnyObject>> {
    let settings = NSMutableDictionary::<AnyObject, AnyObject>::new();
    let format = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
    let key = unsafe { cfstring_as_object(kCVPixelBufferPixelFormatTypeKey) };
    unsafe {
        let _: () = msg_send![&*settings, setObject: &*format, forKey: key];
    }
    settings
}

unsafe fn cfstring_as_object(
    value: &'static objc2_core_foundation::CFString,
) -> &'static AnyObject {
    unsafe { &*(value as *const _ as *const AnyObject) }
}

fn encode_bgra_pixel_buffer_as_jpeg(
    pixel_buffer: &objc2_core_video::CVPixelBuffer,
) -> anyhow::Result<FrameCandidate> {
    let pixel_format = CVPixelBufferGetPixelFormatType(pixel_buffer);
    if pixel_format != kCVPixelFormatType_32BGRA {
        anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: AVFoundation returned unsupported pixel format 0x{pixel_format:08x}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }

    let lock_flags = CVPixelBufferLockFlags::ReadOnly;
    let lock_result = unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, lock_flags) };
    if lock_result != kCVReturnSuccess {
        anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: CVPixelBufferLockBaseAddress failed with {lock_result}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }

    let result = encode_locked_bgra_pixel_buffer(pixel_buffer);
    let _ = unsafe { CVPixelBufferUnlockBaseAddress(pixel_buffer, lock_flags) };
    result
}

fn encode_locked_bgra_pixel_buffer(
    pixel_buffer: &objc2_core_video::CVPixelBuffer,
) -> anyhow::Result<FrameCandidate> {
    let width = CVPixelBufferGetWidth(pixel_buffer);
    let height = CVPixelBufferGetHeight(pixel_buffer);
    let stride = CVPixelBufferGetBytesPerRow(pixel_buffer);
    let base = CVPixelBufferGetBaseAddress(pixel_buffer);
    if width == 0 || height == 0 || stride < width.saturating_mul(4) || base.is_null() {
        anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: AVFoundation returned invalid pixel buffer dimensions \
             {width}x{height} stride={stride}; reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }
    if width > u16::MAX as usize || height > u16::MAX as usize {
        anyhow::bail!(
            "{ABILITY_CAMERA_SNAPSHOT}: camera frame {width}x{height} is too large to JPEG-encode; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }

    let bytes = unsafe { std::slice::from_raw_parts(base.cast::<u8>(), stride * height) };
    let mut rgb = Vec::with_capacity(width * height * 3);
    let mut luma_hist = [0u32; 256];
    let mut luma_sum = 0u64;
    for y in 0..height {
        let row = &bytes[y * stride..y * stride + width * 4];
        for px in row.chunks_exact(4) {
            let r = px[2];
            let g = px[1];
            let b = px[0];
            let luma =
                ((77u16 * u16::from(r) + 150u16 * u16::from(g) + 29u16 * u16::from(b)) >> 8) as u8;
            luma_hist[luma as usize] += 1;
            luma_sum += u64::from(luma);
            rgb.push(r);
            rgb.push(g);
            rgb.push(b);
        }
    }
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
    let pixel_count = width * height;
    Ok(FrameCandidate {
        frame: EncodedFrame {
            jpeg_bytes: jpeg,
            width: width as u32,
            height: height as u32,
        },
        luma_mean: luma_sum as f64 / pixel_count as f64,
        luma_p99: percentile_from_histogram(&luma_hist, pixel_count, 99),
    })
}

fn remember_best_frame(slot: &Mutex<Option<FrameCandidate>>, candidate: FrameCandidate) {
    let mut guard = match slot.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if guard
        .as_ref()
        .map(|current| candidate.score() > current.score())
        .unwrap_or(true)
    {
        *guard = Some(candidate);
    }
}

fn frame_count(slot: &Mutex<Option<FrameCandidate>>) -> usize {
    match slot.lock() {
        Ok(guard) => usize::from(guard.is_some()),
        Err(poisoned) => usize::from(poisoned.into_inner().is_some()),
    }
}

fn percentile_from_histogram(hist: &[u32; 256], total: usize, percentile: u32) -> u8 {
    if total == 0 {
        return 0;
    }
    let target = ((total as u64 * u64::from(percentile)).div_ceil(100)).max(1);
    let mut seen = 0u64;
    for (value, count) in hist.iter().enumerate() {
        seen += u64::from(*count);
        if seen >= target {
            return value as u8;
        }
    }
    u8::MAX
}

fn take_completion_sender<T>(slot: &Mutex<Option<SyncSender<T>>>) -> Option<SyncSender<T>> {
    match slot.lock() {
        Ok(mut guard) => guard.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

fn capture_queue() -> DispatchRetained<DispatchQueue> {
    DispatchQueue::new("tech.easynet.camera.avfoundation.capture", None)
}
