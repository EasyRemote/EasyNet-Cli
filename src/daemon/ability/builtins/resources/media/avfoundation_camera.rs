// EasyNet CLI — native macOS camera capture
// =========================================
//
// File: src/daemon/ability/builtins/resources/media/avfoundation_camera.rs
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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, ProtocolObject};
use objc2::{class, define_class, msg_send, AnyThread, DefinedClass};
use objc2_av_foundation::{
    AVCapturePhoto, AVCapturePhotoCaptureDelegate, AVCapturePhotoOutput, AVCapturePhotoSettings,
};
use objc2_core_video::{
    kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA, kCVReturnSuccess,
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight,
    CVPixelBufferGetPixelFormatType, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{
    NSArray, NSError, NSMutableDictionary, NSNumber, NSObject, NSObjectProtocol, NSString,
};
use serde_json::{json, Value};
use tokio::sync::broadcast;

use crate::daemon::ability::builtins::resources::media::camera_snapshot::{
    build_camera_stream_frame, CameraStreamOptions, EncodedFrame, REASON_PERMISSION_DENIED,
    REASON_RESOURCE_UNAVAILABLE,
};
use crate::daemon::ability::builtins::resources::media::{
    ABILITY_CAMERA_SNAPSHOT, ABILITY_CAMERA_SUBSCRIBE,
};
use crate::persistence::resources::ResourceEntry;

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}

const AV_MEDIA_TYPE_VIDEO: &str = "vide";
const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(30);
const PHOTO_CAPTURE_TIMEOUT: Duration = Duration::from_secs(8);

// AVAuthorizationStatus.
const AUTH_NOT_DETERMINED: isize = 0;
const AUTH_RESTRICTED: isize = 1;
const AUTH_DENIED: isize = 2;
const AUTH_AUTHORIZED: isize = 3;

struct PhotoCaptureDelegateIvars {
    sender: Mutex<Option<SyncSender<anyhow::Result<Vec<u8>>>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "EasyNetAVFoundationPhotoCaptureDelegate"]
    #[ivars = PhotoCaptureDelegateIvars]
    struct PhotoCaptureDelegate;

    unsafe impl NSObjectProtocol for PhotoCaptureDelegate {}

    unsafe impl AVCapturePhotoCaptureDelegate for PhotoCaptureDelegate {
        #[unsafe(method(captureOutput:didFinishProcessingPhoto:error:))]
        unsafe fn capture_output_did_finish_processing_photo_error(
            &self,
            _output: &AVCapturePhotoOutput,
            photo: &AVCapturePhoto,
            error: Option<&NSError>,
        ) {
            let result = match error {
                Some(error) => Err(anyhow::anyhow!(
                    "{ABILITY_CAMERA_SNAPSHOT}: AVCapturePhotoOutput failed: {}; \
                     reason={REASON_RESOURCE_UNAVAILABLE}",
                    error.localizedDescription()
                )),
                None => unsafe { photo.fileDataRepresentation() }
                    .map(|data| data.to_vec())
                    .filter(|bytes| !bytes.is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "{ABILITY_CAMERA_SNAPSHOT}: AVCapturePhotoOutput returned no file data; \
                             reason={REASON_RESOURCE_UNAVAILABLE}"
                        )
                    }),
            };
            if let Some(sender) = take_completion_sender(&self.ivars().sender) {
                let _ = sender.send(result);
            }
        }
    }
);

impl PhotoCaptureDelegate {
    fn new(sender: SyncSender<anyhow::Result<Vec<u8>>>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(PhotoCaptureDelegateIvars {
            sender: Mutex::new(Some(sender)),
        });
        unsafe { msg_send![super(this), init] }
    }
}

struct LiveVideoFrameDelegateIvars {
    sender: broadcast::Sender<Value>,
    hardware_id: String,
    frame_interval: Duration,
    last_sent_at: Mutex<Option<Instant>>,
    seq: AtomicU64,
    failed: AtomicBool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "EasyNetAVFoundationLiveVideoFrameDelegate"]
    #[ivars = LiveVideoFrameDelegateIvars]
    struct LiveVideoFrameDelegate;

    impl LiveVideoFrameDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        unsafe fn capture_output(
            &self,
            _output: &AnyObject,
            sample_buffer: &objc2_core_media::CMSampleBuffer,
            _connection: &AnyObject,
        ) {
            let ivars = self.ivars();
            if ivars.sender.receiver_count() == 0 {
                return;
            }
            let now = Instant::now();
            {
                let mut last_sent_at = match ivars.last_sent_at.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if last_sent_at
                    .map(|previous| now.duration_since(previous) < ivars.frame_interval)
                    .unwrap_or(false)
                {
                    return;
                }
                *last_sent_at = Some(now);
            }

            let frame = match unsafe { sample_buffer.image_buffer() } {
                Some(image_buffer) => encode_bgra_pixel_buffer_as_jpeg(
                    &image_buffer,
                    ABILITY_CAMERA_SUBSCRIBE,
                    false,
                ),
                None => Err(anyhow::anyhow!(
                    "{ABILITY_CAMERA_SUBSCRIBE}: AVFoundation sample contained no image buffer; \
                     reason={REASON_RESOURCE_UNAVAILABLE}"
                )),
            };

            match frame {
                Ok(frame) => {
                    let seq = ivars.seq.fetch_add(1, Ordering::Relaxed);
                    let value = build_camera_stream_frame(seq, &ivars.hardware_id, frame);
                    let _ = ivars.sender.send(value);
                }
                Err(err) => {
                    if !ivars.failed.swap(true, Ordering::Relaxed) {
                        let _ = ivars.sender.send(json!({
                            "type": "error",
                            "message": err.to_string(),
                            "reason": REASON_RESOURCE_UNAVAILABLE,
                        }));
                    }
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for LiveVideoFrameDelegate {}
);

impl LiveVideoFrameDelegate {
    fn new(
        sender: broadcast::Sender<Value>,
        hardware_id: String,
        frame_interval: Duration,
    ) -> Retained<Self> {
        let this = Self::alloc().set_ivars(LiveVideoFrameDelegateIvars {
            sender,
            hardware_id,
            frame_interval,
            last_sent_at: Mutex::new(None),
            seq: AtomicU64::new(0),
            failed: AtomicBool::new(false),
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
    let output = unsafe { AVCapturePhotoOutput::new() };
    let (tx, rx) = sync_channel::<anyhow::Result<Vec<u8>>>(1);
    let delegate = PhotoCaptureDelegate::new(tx);
    let delegate_protocol = ProtocolObject::from_ref(&*delegate);

    unsafe {
        let _: () = msg_send![&*session, beginConfiguration];

        // camera.snapshot is a STILL ability. Use the photo preset and
        // AVCapturePhotoOutput; do not capture a preview frame from
        // AVCaptureVideoDataOutput and hope exposure has settled.
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

        let can_add_output: bool = msg_send![&*session, canAddOutput: &*output];
        if !can_add_output {
            anyhow::bail!(
                "{ABILITY_CAMERA_SNAPSHOT}: AVFoundation cannot add photo output; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            );
        }
        let _: () = msg_send![&*session, addOutput: &*output];
        let _: () = msg_send![&*session, commitConfiguration];
        let _: () = msg_send![&*session, startRunning];

        let settings = AVCapturePhotoSettings::photoSettings();
        output.capturePhotoWithSettings_delegate(&settings, delegate_protocol);
    }

    let jpeg_bytes = match rx.recv_timeout(PHOTO_CAPTURE_TIMEOUT) {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(err)) => Err(err),
        Err(_) => Err(anyhow::anyhow!(
            "{ABILITY_CAMERA_SNAPSHOT}: AVFoundation did not deliver a still photo within {}ms; \
             reason={REASON_RESOURCE_UNAVAILABLE}",
            PHOTO_CAPTURE_TIMEOUT.as_millis()
        )),
    }?;

    let (width, height) = jpeg_dimensions(&jpeg_bytes).ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_SNAPSHOT}: AVFoundation still photo was not JPEG file data; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;

    unsafe {
        let _: () = msg_send![&*session, stopRunning];
    }

    Ok(EncodedFrame {
        jpeg_bytes,
        width,
        height,
    })
}

pub fn open_jpeg_stream(
    entry: ResourceEntry,
    options: CameraStreamOptions,
) -> anyhow::Result<broadcast::Receiver<Value>> {
    ensure_camera_authorized().map_err(rewrite_subscribe_error)?;

    let (tx, rx) = broadcast::channel::<Value>(8);
    let worker_tx = tx.clone();
    std::thread::Builder::new()
        .name("easynet-camera-avfoundation".into())
        .spawn(move || {
            if let Err(err) = run_jpeg_stream(entry, options, worker_tx.clone()) {
                let _ = worker_tx.send(json!({
                    "type": "error",
                    "message": rewrite_subscribe_error(err).to_string(),
                    "reason": REASON_RESOURCE_UNAVAILABLE,
                }));
            }
        })
        .map_err(|e| {
            anyhow::anyhow!(
                "{ABILITY_CAMERA_SUBSCRIBE}: failed to spawn AVFoundation camera worker: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
    Ok(rx)
}

fn run_jpeg_stream(
    entry: ResourceEntry,
    options: CameraStreamOptions,
    sender: broadcast::Sender<Value>,
) -> anyhow::Result<()> {
    let media_type = NSString::from_str(AV_MEDIA_TYPE_VIDEO);
    let device = select_camera_device(&media_type, &entry)?;
    let input = device_input(&device)?;

    let session: Retained<AnyObject> = unsafe { msg_send![class!(AVCaptureSession), new] };
    let output: Retained<AnyObject> = unsafe { msg_send![class!(AVCaptureVideoDataOutput), new] };
    let settings = bgra_video_settings();
    let delegate = LiveVideoFrameDelegate::new(
        sender.clone(),
        entry.hardware_id.clone(),
        Duration::from_secs_f64(1.0 / options.fps as f64),
    );
    let queue = capture_queue();

    unsafe {
        let _: () = msg_send![&*session, beginConfiguration];

        let preset = live_session_preset(&options);
        let can_set_preset: bool = msg_send![&*session, canSetSessionPreset: &*preset];
        if can_set_preset {
            let _: () = msg_send![&*session, setSessionPreset: &*preset];
        }

        let can_add_input: bool = msg_send![&*session, canAddInput: &*input];
        if !can_add_input {
            anyhow::bail!(
                "{ABILITY_CAMERA_SUBSCRIBE}: AVFoundation cannot add camera input; \
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
                "{ABILITY_CAMERA_SUBSCRIBE}: AVFoundation cannot add video output; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            );
        }
        let _: () = msg_send![&*session, addOutput: &*output];
        let _: () = msg_send![&*session, commitConfiguration];
        let _: () = msg_send![&*session, startRunning];
    }

    while sender.receiver_count() > 0 {
        std::thread::sleep(Duration::from_millis(100));
    }

    unsafe {
        let _: () = msg_send![&*session, stopRunning];
        let _: () = msg_send![
            &*output,
            setSampleBufferDelegate: ptr::null::<AnyObject>(),
            queue: ptr::null::<AnyObject>()
        ];
    }

    drop(delegate);
    drop(queue);
    drop(output);
    drop(input);
    drop(device);
    drop(session);
    Ok(())
}

fn live_session_preset(options: &CameraStreamOptions) -> Retained<NSString> {
    let preset = match options.resolution {
        Some(resolution) if resolution.width <= 640 && resolution.height <= 480 => {
            "AVCaptureSessionPreset640x480"
        }
        Some(resolution) if resolution.width <= 1280 && resolution.height <= 720 => {
            "AVCaptureSessionPreset1280x720"
        }
        Some(resolution) if resolution.width <= 1920 && resolution.height <= 1080 => {
            "AVCaptureSessionPreset1920x1080"
        }
        _ => "AVCaptureSessionPresetHigh",
    };
    NSString::from_str(preset)
}

fn rewrite_subscribe_error(err: anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!(
        "{}",
        err.to_string()
            .replacen(ABILITY_CAMERA_SNAPSHOT, ABILITY_CAMERA_SUBSCRIBE, 1)
    )
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
    ability: &'static str,
    reject_all_black: bool,
) -> anyhow::Result<EncodedFrame> {
    let pixel_format = CVPixelBufferGetPixelFormatType(pixel_buffer);
    if pixel_format != kCVPixelFormatType_32BGRA {
        anyhow::bail!(
            "{ability}: AVFoundation returned unsupported pixel format 0x{pixel_format:08x}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }

    let lock_flags = CVPixelBufferLockFlags::ReadOnly;
    let lock_result = unsafe { CVPixelBufferLockBaseAddress(pixel_buffer, lock_flags) };
    if lock_result != kCVReturnSuccess {
        anyhow::bail!(
            "{ability}: CVPixelBufferLockBaseAddress failed with {lock_result}; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }

    let result = encode_locked_bgra_pixel_buffer(pixel_buffer, ability, reject_all_black);
    let _ = unsafe { CVPixelBufferUnlockBaseAddress(pixel_buffer, lock_flags) };
    result
}

fn encode_locked_bgra_pixel_buffer(
    pixel_buffer: &objc2_core_video::CVPixelBuffer,
    ability: &'static str,
    reject_all_black: bool,
) -> anyhow::Result<EncodedFrame> {
    let width = CVPixelBufferGetWidth(pixel_buffer);
    let height = CVPixelBufferGetHeight(pixel_buffer);
    let stride = CVPixelBufferGetBytesPerRow(pixel_buffer);
    let base = CVPixelBufferGetBaseAddress(pixel_buffer);
    if width == 0 || height == 0 || stride < width.saturating_mul(4) || base.is_null() {
        anyhow::bail!(
            "{ability}: AVFoundation returned invalid pixel buffer dimensions \
             {width}x{height} stride={stride}; reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }
    if width > u16::MAX as usize || height > u16::MAX as usize {
        anyhow::bail!(
            "{ability}: camera frame {width}x{height} is too large to JPEG-encode; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }

    let bytes = unsafe { std::slice::from_raw_parts(base.cast::<u8>(), stride * height) };
    let mut rgb = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        let row = &bytes[y * stride..y * stride + width * 4];
        for px in row.chunks_exact(4) {
            let r = px[2];
            let g = px[1];
            let b = px[0];
            rgb.push(r);
            rgb.push(g);
            rgb.push(b);
        }
    }
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
    Ok(EncodedFrame {
        jpeg_bytes: jpeg,
        width: width as u32,
        height: height as u32,
    })
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return None;
    }
    let mut i = 2;
    while i + 3 < bytes.len() {
        while i < bytes.len() && bytes[i] != 0xff {
            i += 1;
        }
        while i < bytes.len() && bytes[i] == 0xff {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        let marker = bytes[i];
        i += 1;
        if marker == 0xd9 || marker == 0xda {
            return None;
        }
        if matches!(marker, 0x01 | 0xd0..=0xd7) {
            continue;
        }
        if i + 1 >= bytes.len() {
            return None;
        }
        let segment_len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
        if segment_len < 2 || i + segment_len > bytes.len() {
            return None;
        }
        if is_start_of_frame_marker(marker) {
            if segment_len < 7 {
                return None;
            }
            let height = u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]) as u32;
            let width = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            return (width > 0 && height > 0).then_some((width, height));
        }
        i += segment_len;
    }
    None
}

fn is_start_of_frame_marker(marker: u8) -> bool {
    matches!(
        marker,
        0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
    )
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
