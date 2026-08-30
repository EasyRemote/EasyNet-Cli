// EasyNet CLI — native macOS camera capture
// =========================================
//
// File: src/daemon/ability/builtins/resources/media/avfoundation_camera.rs
// Description: Native AVFoundation photo, preview, and movie capture.
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

use std::fs;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::daemon::ability::builtins::resources::media::camera_snapshot::{
    publish_camera_failure, publish_camera_frame, CameraFrameSender, CameraRecordingArtifact,
    CameraRecordingOptions, CameraStreamOptions, EncodedFrame, REASON_PERMISSION_DENIED,
    REASON_RESOURCE_UNAVAILABLE,
};
use crate::daemon::ability::builtins::resources::media::{
    ABILITY_CAMERA_RECORD_START, ABILITY_CAMERA_RECORD_STOP, ABILITY_CAMERA_SNAPSHOT,
    ABILITY_CAMERA_SUBSCRIBE,
};
use crate::daemon::persistence::resources::ResourceEntry;
use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{AnyObject, Bool};
use objc2::{class, define_class, msg_send, AnyThread, DefinedClass};
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVCaptureDevice, AVFileTypeQuickTimeMovie, AVMediaTypeVideo,
    AVVideoAverageBitRateKey, AVVideoCodecKey, AVVideoCodecTypeH264,
    AVVideoCompressionPropertiesKey, AVVideoExpectedSourceFrameRateKey, AVVideoHeightKey,
    AVVideoMaxKeyFrameIntervalKey, AVVideoWidthKey,
};
use objc2_core_media::CMSampleBuffer;
use objc2_core_video::{
    kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange, kCVReturnSuccess, CVPixelBufferGetBaseAddress,
    CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSMutableDictionary, NSNumber, NSObject, NSObjectProtocol,
    NSString, NSURL,
};

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}

const AV_MEDIA_TYPE_VIDEO: &str = "vide";
const AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(30);
const RECORDING_START_TIMEOUT: Duration = Duration::from_secs(5);
const RECORDING_FINALIZE_TIMEOUT: Duration = Duration::from_secs(10);
const RECORDING_FILE_SIZE_CHECK_INTERVAL: Duration = Duration::from_millis(100);

// AVAuthorizationStatus.
const AUTH_NOT_DETERMINED: isize = 0;
const AUTH_RESTRICTED: isize = 1;
const AUTH_DENIED: isize = 2;
const AUTH_AUTHORIZED: isize = 3;

#[derive(Debug)]
struct NativeRecordingCompletion {
    duration_ms: u64,
    frame_count: u64,
    width: u32,
    height: u32,
    stop_reason: &'static str,
}

struct AssetWriterState {
    writer: Option<Retained<AVAssetWriter>>,
    input: Option<Retained<AVAssetWriterInput>>,
    started_at: Option<Instant>,
    next_size_check_at: Instant,
    frame_count: u64,
    width: u32,
    height: u32,
    finished: bool,
}

struct AssetWriterVideoDelegateIvars {
    state: Mutex<AssetWriterState>,
    output_url: Retained<NSURL>,
    output_path: PathBuf,
    options: CameraRecordingOptions,
    stop: Arc<AtomicBool>,
    ready: Mutex<Option<mpsc::Sender<anyhow::Result<()>>>>,
    completion: Mutex<Option<SyncSender<anyhow::Result<NativeRecordingCompletion>>>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "EasyNetAVFoundationAssetWriterVideoDelegate"]
    #[ivars = AssetWriterVideoDelegateIvars]
    struct AssetWriterVideoDelegate;

    impl AssetWriterVideoDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        unsafe fn capture_output(
            &self,
            _output: &AnyObject,
            sample_buffer: &CMSampleBuffer,
            _connection: &AnyObject,
        ) {
            autoreleasepool(|_| self.consume_sample(sample_buffer));
        }
    }

    unsafe impl NSObjectProtocol for AssetWriterVideoDelegate {}
);

impl AssetWriterVideoDelegate {
    fn new(
        output_url: Retained<NSURL>,
        output_path: PathBuf,
        options: CameraRecordingOptions,
        stop: Arc<AtomicBool>,
        ready: mpsc::Sender<anyhow::Result<()>>,
        completion: SyncSender<anyhow::Result<NativeRecordingCompletion>>,
    ) -> Retained<Self> {
        let this = Self::alloc().set_ivars(AssetWriterVideoDelegateIvars {
            state: Mutex::new(AssetWriterState {
                writer: None,
                input: None,
                started_at: None,
                next_size_check_at: Instant::now(),
                frame_count: 0,
                width: 0,
                height: 0,
                finished: false,
            }),
            output_url,
            output_path,
            options,
            stop,
            ready: Mutex::new(Some(ready)),
            completion: Mutex::new(Some(completion)),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn consume_sample(&self, sample_buffer: &CMSampleBuffer) {
        let mut state = lock_unpoisoned(&self.ivars().state);
        if state.finished {
            return;
        }

        if state.writer.is_none() {
            let Some(pixel_buffer) = (unsafe { sample_buffer.image_buffer() }) else {
                self.fail_locked(
                    &mut state,
                    anyhow::anyhow!(
                        "{ABILITY_CAMERA_RECORD_START}: AVFoundation sample contained no image buffer; \
                         reason={REASON_RESOURCE_UNAVAILABLE}"
                    ),
                );
                return;
            };
            let width = CVPixelBufferGetWidth(&pixel_buffer).min(u32::MAX as usize) as u32;
            let height = CVPixelBufferGetHeight(&pixel_buffer).min(u32::MAX as usize) as u32;
            if width == 0 || height == 0 {
                self.fail_locked(
                    &mut state,
                    anyhow::anyhow!(
                        "{ABILITY_CAMERA_RECORD_START}: AVFoundation returned invalid video dimensions \
                         {width}x{height}; reason={REASON_RESOURCE_UNAVAILABLE}"
                    ),
                );
                return;
            }
            match create_asset_writer(
                &self.ivars().output_url,
                width,
                height,
                &self.ivars().options,
            ) {
                Ok((writer, input)) => {
                    unsafe {
                        writer.startSessionAtSourceTime(sample_buffer.presentation_time_stamp())
                    };
                    state.writer = Some(writer);
                    state.input = Some(input);
                    state.started_at = Some(Instant::now());
                    state.next_size_check_at = Instant::now() + RECORDING_FILE_SIZE_CHECK_INTERVAL;
                    state.width = width;
                    state.height = height;
                }
                Err(error) => {
                    self.fail_locked(&mut state, error);
                    return;
                }
            }
        }

        let started_at = state
            .started_at
            .expect("asset writer initialization sets start time");
        let recording_elapsed_ms = elapsed_ms(started_at);
        let stop_reason = if self.ivars().stop.load(Ordering::Relaxed) {
            Some("stopped")
        } else if recording_elapsed_ms >= self.ivars().options.max_duration_ms {
            Some("duration_limit")
        } else {
            None
        };
        if let Some(reason) = stop_reason {
            self.finish_locked(&mut state, reason);
            return;
        }

        let input = state
            .input
            .as_ref()
            .expect("asset writer initialization sets input");
        if !unsafe { input.isReadyForMoreMediaData() } {
            // This is a real-time source. Keeping the old frame would increase
            // both latency and memory usage, so producer backpressure is a
            // deliberate single-frame drop rather than an intermediate queue.
            return;
        }
        if !unsafe { input.appendSampleBuffer(sample_buffer) } {
            let message = state
                .writer
                .as_ref()
                .map(|writer| asset_writer_error(writer))
                .unwrap_or_else(|| "writer disappeared".to_string());
            self.fail_locked(
                &mut state,
                anyhow::anyhow!(
                    "{ABILITY_CAMERA_RECORD_STOP}: AVAssetWriter rejected a camera sample: {message}; \
                     reason={REASON_RESOURCE_UNAVAILABLE}"
                ),
            );
            return;
        }

        state.frame_count += 1;
        if let Some(sender) = take_completion_sender(&self.ivars().ready) {
            if sender.send(Ok(())).is_err() {
                self.fail_locked(
                    &mut state,
                    anyhow::anyhow!(
                        "{ABILITY_CAMERA_RECORD_START}: caller abandoned native recording readiness; \
                         reason=recording_start_cancelled"
                    ),
                );
                return;
            }
        }

        let now = Instant::now();
        if now >= state.next_size_check_at {
            state.next_size_check_at = now + RECORDING_FILE_SIZE_CHECK_INTERVAL;
            if fs::metadata(&self.ivars().output_path)
                .map(|metadata| metadata.len() >= self.ivars().options.max_bytes)
                .unwrap_or(false)
            {
                self.finish_locked(&mut state, "byte_limit");
                return;
            }
        }

        if elapsed_ms(started_at) >= self.ivars().options.max_duration_ms {
            self.finish_locked(&mut state, "duration_limit");
        }
    }

    fn finish_locked(&self, state: &mut AssetWriterState, stop_reason: &'static str) {
        if state.finished {
            return;
        }
        state.finished = true;
        let Some(input) = state.input.take() else {
            self.fail_locked(
                state,
                anyhow::anyhow!(
                    "{ABILITY_CAMERA_RECORD_STOP}: AVAssetWriter input unavailable during finalization; \
                     reason={REASON_RESOURCE_UNAVAILABLE}"
                ),
            );
            return;
        };
        let Some(writer) = state.writer.take() else {
            self.fail_locked(
                state,
                anyhow::anyhow!(
                    "{ABILITY_CAMERA_RECORD_STOP}: AVAssetWriter unavailable during finalization; \
                     reason={REASON_RESOURCE_UNAVAILABLE}"
                ),
            );
            return;
        };

        unsafe { input.markAsFinished() };
        #[allow(deprecated)]
        let finished = unsafe { writer.finishWriting() };
        if !finished {
            self.fail_locked(
                state,
                anyhow::anyhow!(
                    "{ABILITY_CAMERA_RECORD_STOP}: AVAssetWriter failed to finalize: {}; \
                     reason={REASON_RESOURCE_UNAVAILABLE}",
                    asset_writer_error(&writer)
                ),
            );
            return;
        }

        let completion = NativeRecordingCompletion {
            duration_ms: state.started_at.map(elapsed_ms).unwrap_or(0),
            frame_count: state.frame_count,
            width: state.width,
            height: state.height,
            stop_reason,
        };
        if let Some(sender) = take_completion_sender(&self.ivars().completion) {
            let _ = sender.send(Ok(completion));
        }
    }

    fn fail_locked(&self, state: &mut AssetWriterState, error: anyhow::Error) {
        state.finished = true;
        if let Some(writer) = state.writer.take() {
            unsafe { writer.cancelWriting() };
        }
        state.input.take();
        let message = error.to_string();
        if let Some(sender) = take_completion_sender(&self.ivars().ready) {
            let _ = sender.send(Err(anyhow::anyhow!(message.clone())));
        }
        if let Some(sender) = take_completion_sender(&self.ivars().completion) {
            let _ = sender.send(Err(anyhow::anyhow!(message)));
        }
    }

    fn fail_before_first_sample(&self, error: anyhow::Error) {
        let mut state = lock_unpoisoned(&self.ivars().state);
        if !state.finished {
            self.fail_locked(&mut state, error);
        }
    }
}

struct LiveVideoFrameDelegateIvars {
    sender: CameraFrameSender,
    frame_interval: Duration,
    last_sent_at: Mutex<Option<Instant>>,
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
                    publish_camera_frame(&ivars.sender, frame);
                }
                Err(err) => {
                    if !ivars.failed.swap(true, Ordering::Relaxed) {
                        publish_camera_failure(&ivars.sender, err);
                    }
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for LiveVideoFrameDelegate {}
);

impl LiveVideoFrameDelegate {
    fn new(sender: CameraFrameSender, frame_interval: Duration) -> Retained<Self> {
        let this = Self::alloc().set_ivars(LiveVideoFrameDelegateIvars {
            sender,
            frame_interval,
            last_sent_at: Mutex::new(None),
            failed: AtomicBool::new(false),
        });
        unsafe { msg_send![super(this), init] }
    }
}

fn create_asset_writer(
    output_url: &NSURL,
    width: u32,
    height: u32,
    options: &CameraRecordingOptions,
) -> anyhow::Result<(Retained<AVAssetWriter>, Retained<AVAssetWriterInput>)> {
    let file_type = unsafe { AVFileTypeQuickTimeMovie }.ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_RECORD_START}: AVFileTypeQuickTimeMovie unavailable; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let media_type = unsafe { AVMediaTypeVideo }.ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_RECORD_START}: AVMediaTypeVideo unavailable; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;
    let codec_key = unsafe { AVVideoCodecKey }.ok_or_else(|| missing_av_key("AVVideoCodecKey"))?;
    let width_key = unsafe { AVVideoWidthKey }.ok_or_else(|| missing_av_key("AVVideoWidthKey"))?;
    let height_key =
        unsafe { AVVideoHeightKey }.ok_or_else(|| missing_av_key("AVVideoHeightKey"))?;
    let bitrate_key = unsafe { AVVideoAverageBitRateKey }
        .ok_or_else(|| missing_av_key("AVVideoAverageBitRateKey"))?;
    let compression_key = unsafe { AVVideoCompressionPropertiesKey }
        .ok_or_else(|| missing_av_key("AVVideoCompressionPropertiesKey"))?;
    let fps_key = unsafe { AVVideoExpectedSourceFrameRateKey }
        .ok_or_else(|| missing_av_key("AVVideoExpectedSourceFrameRateKey"))?;
    let keyframe_key = unsafe { AVVideoMaxKeyFrameIntervalKey }
        .ok_or_else(|| missing_av_key("AVVideoMaxKeyFrameIntervalKey"))?;
    let codec = unsafe { AVVideoCodecTypeH264 }.ok_or_else(|| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_RECORD_START}: AVVideoCodecTypeH264 unavailable; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        )
    })?;

    let width_number = NSNumber::new_u32(width);
    let height_number = NSNumber::new_u32(height);
    let bitrate_number = NSNumber::new_u64(target_h264_bitrate(width, height, options));
    let fps_number = NSNumber::new_u32(options.stream.fps);
    let keyframe_number = NSNumber::new_u32(options.stream.fps.saturating_mul(2));
    let codec_object: &AnyObject = codec;
    let width_object: &AnyObject = &width_number;
    let height_object: &AnyObject = &height_number;
    let bitrate_object: &AnyObject = &bitrate_number;
    let fps_object: &AnyObject = &fps_number;
    let keyframe_object: &AnyObject = &keyframe_number;
    let compression: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(
        &[bitrate_key, fps_key, keyframe_key],
        &[bitrate_object, fps_object, keyframe_object],
    );
    let compression_object: &AnyObject = &compression;
    let settings: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(
        &[codec_key, width_key, height_key, compression_key],
        &[
            codec_object,
            width_object,
            height_object,
            compression_object,
        ],
    );

    let writer = unsafe { AVAssetWriter::assetWriterWithURL_fileType_error(output_url, file_type) }
        .map_err(|error| {
            anyhow::anyhow!(
                "{ABILITY_CAMERA_RECORD_START}: AVAssetWriter creation failed: {}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}",
                error.localizedDescription()
            )
        })?;
    let input = unsafe {
        AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
            media_type,
            Some(&settings),
        )
    };
    unsafe { input.setExpectsMediaDataInRealTime(true) };
    if !unsafe { writer.canAddInput(&input) } {
        anyhow::bail!(
            "{ABILITY_CAMERA_RECORD_START}: AVAssetWriter rejected H.264 video input; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }
    unsafe { writer.addInput(&input) };
    if !unsafe { writer.startWriting() } {
        anyhow::bail!(
            "{ABILITY_CAMERA_RECORD_START}: AVAssetWriter failed to start: {}; \
             reason={REASON_RESOURCE_UNAVAILABLE}",
            asset_writer_error(&writer)
        );
    }
    Ok((writer, input))
}

fn missing_av_key(name: &'static str) -> anyhow::Error {
    anyhow::anyhow!(
        "{ABILITY_CAMERA_RECORD_START}: {name} unavailable; \
         reason={REASON_RESOURCE_UNAVAILABLE}"
    )
}

fn target_h264_bitrate(width: u32, height: u32, options: &CameraRecordingOptions) -> u64 {
    // Roughly 0.125 bits/pixel/frame is a practical realtime H.264 ceiling for
    // webcam content. The storage-derived ceiling reserves 15% for container
    // overhead and encoder variability. The live byte check remains the
    // authoritative stop condition.
    let content_target = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(u64::from(options.stream.fps))
        / 8;
    let storage_target = options
        .max_bytes
        .saturating_mul(8)
        .saturating_mul(1_000)
        .saturating_mul(85)
        / options.max_duration_ms.max(1)
        / 100;
    content_target
        .clamp(250_000, 20_000_000)
        .min(storage_target.max(100_000))
}

fn asset_writer_error(writer: &AVAssetWriter) -> String {
    unsafe { writer.error() }
        .map(|error| error.localizedDescription().to_string())
        .unwrap_or_else(|| format!("status={:?}", unsafe { writer.status() }))
}

fn elapsed_ms(started_at: Instant) -> u64 {
    started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

pub(crate) fn record_movie(
    entry: ResourceEntry,
    options: CameraRecordingOptions,
    stop: Arc<AtomicBool>,
    temp_path: PathBuf,
    ready: mpsc::Sender<anyhow::Result<()>>,
) -> anyhow::Result<CameraRecordingArtifact> {
    let cleanup_path = temp_path.clone();
    let result = autoreleasepool(|_| record_movie_inner(entry, options, stop, temp_path, ready));
    if result.is_err() {
        let _ = fs::remove_file(cleanup_path);
    }
    result
}

fn record_movie_inner(
    entry: ResourceEntry,
    options: CameraRecordingOptions,
    stop: Arc<AtomicBool>,
    temp_path: PathBuf,
    ready: mpsc::Sender<anyhow::Result<()>>,
) -> anyhow::Result<CameraRecordingArtifact> {
    ensure_camera_authorized().map_err(rewrite_recording_error)?;
    if let Some(parent) = temp_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if temp_path.exists() {
        fs::remove_file(&temp_path)?;
    }

    let media_type = NSString::from_str(AV_MEDIA_TYPE_VIDEO);
    let device = select_camera_device(&media_type, &entry).map_err(rewrite_recording_error)?;
    let input = device_input(&device).map_err(rewrite_recording_error)?;
    let session: Retained<AnyObject> = unsafe { msg_send![class!(AVCaptureSession), new] };
    let output: Retained<AnyObject> = unsafe { msg_send![class!(AVCaptureVideoDataOutput), new] };
    let path = NSString::from_str(&temp_path.to_string_lossy());
    let output_url = NSURL::fileURLWithPath(&path);
    let (completion_tx, completion_rx) =
        sync_channel::<anyhow::Result<NativeRecordingCompletion>>(1);
    let delegate = AssetWriterVideoDelegate::new(
        output_url,
        temp_path.clone(),
        options.clone(),
        Arc::clone(&stop),
        ready,
        completion_tx,
    );
    let queue = capture_queue();

    unsafe {
        let _: () = msg_send![&*session, beginConfiguration];
        let preset = live_session_preset(&options.stream);
        let can_set_preset: bool = msg_send![&*session, canSetSessionPreset: &*preset];
        if can_set_preset {
            let _: () = msg_send![&*session, setSessionPreset: &*preset];
        }

        let can_add_input: bool = msg_send![&*session, canAddInput: &*input];
        if !can_add_input {
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_START}: AVFoundation cannot add camera input; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            );
        }
        let _: () = msg_send![&*session, addInput: &*input];

        let settings = yuv_video_settings();
        let _: () = msg_send![&*output, setAlwaysDiscardsLateVideoFrames: true];
        let _: () = msg_send![&*output, setVideoSettings: &*settings];
        let _: () = msg_send![&*output, setSampleBufferDelegate: &*delegate, queue: &*queue];

        let can_add_output: bool = msg_send![&*session, canAddOutput: &*output];
        if !can_add_output {
            anyhow::bail!(
                "{ABILITY_CAMERA_RECORD_START}: AVFoundation cannot add video data output; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            );
        }
        let _: () = msg_send![&*session, addOutput: &*output];
        let _: () = msg_send![&*session, commitConfiguration];
        let _: () = msg_send![&*session, startRunning];
    }

    let capture_wait_started = Instant::now();
    let hard_deadline = capture_wait_started
        + Duration::from_millis(options.max_duration_ms)
        + RECORDING_FINALIZE_TIMEOUT;
    let completion = loop {
        match completion_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(result) => break result,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err(anyhow::anyhow!(
                    "{ABILITY_CAMERA_RECORD_STOP}: native asset writer delegate disconnected; \
                     reason={REASON_RESOURCE_UNAVAILABLE}"
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if capture_wait_started.elapsed() >= RECORDING_START_TIMEOUT
            && lock_unpoisoned(&delegate.ivars().state).frame_count == 0
        {
            let error = anyhow::anyhow!(
                "{ABILITY_CAMERA_RECORD_START}: AVFoundation delivered no writable camera sample within {}ms; \
                 reason={REASON_RESOURCE_UNAVAILABLE}",
                RECORDING_START_TIMEOUT.as_millis()
            );
            delegate.fail_before_first_sample(anyhow::anyhow!(error.to_string()));
            break Err(error);
        }
        if Instant::now() >= hard_deadline {
            let error = anyhow::anyhow!(
                "{ABILITY_CAMERA_RECORD_STOP}: AVAssetWriter did not finalize within {}ms of the recording limit; \
                 reason=recording_stop_timeout",
                RECORDING_FINALIZE_TIMEOUT.as_millis()
            );
            delegate.fail_before_first_sample(anyhow::anyhow!(error.to_string()));
            break Err(error);
        }
    };

    unsafe {
        let _: () = msg_send![&*session, stopRunning];
        let _: () = msg_send![
            &*output,
            setSampleBufferDelegate: ptr::null::<AnyObject>(),
            queue: ptr::null::<AnyObject>()
        ];
    }
    let completion = completion?;

    let metadata = fs::metadata(&temp_path).map_err(|error| {
        anyhow::anyhow!(
            "{ABILITY_CAMERA_RECORD_STOP}: native movie artifact {} unavailable: {error}; \
             reason={REASON_RESOURCE_UNAVAILABLE}",
            temp_path.display()
        )
    })?;
    if metadata.len() == 0 {
        anyhow::bail!(
            "{ABILITY_CAMERA_RECORD_STOP}: native movie artifact is empty; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }
    Ok(CameraRecordingArtifact {
        temp_path,
        extension: "mov",
        content_type: "video/quicktime",
        stopped_at: chrono::Utc::now().to_rfc3339(),
        duration_ms: completion.duration_ms,
        frame_count: Some(completion.frame_count),
        byte_size: metadata.len(),
        width: Some(completion.width),
        height: Some(completion.height),
        stop_reason: completion.stop_reason,
    })
}

pub fn open_jpeg_stream(
    entry: ResourceEntry,
    options: CameraStreamOptions,
    tx: CameraFrameSender,
) -> anyhow::Result<()> {
    ensure_camera_authorized().map_err(rewrite_subscribe_error)?;

    let worker_tx = tx.clone();
    std::thread::Builder::new()
        .name("easynet-camera-avfoundation".into())
        .spawn(move || {
            if let Err(err) = run_jpeg_stream(entry, options, worker_tx.clone()) {
                publish_camera_failure(&worker_tx, rewrite_subscribe_error(err));
            }
        })
        .map_err(|e| {
            anyhow::anyhow!(
                "{ABILITY_CAMERA_SUBSCRIBE}: failed to spawn AVFoundation camera worker: {e}; \
                 reason={REASON_RESOURCE_UNAVAILABLE}"
            )
        })?;
    Ok(())
}

fn run_jpeg_stream(
    entry: ResourceEntry,
    options: CameraStreamOptions,
    sender: CameraFrameSender,
) -> anyhow::Result<()> {
    let media_type = NSString::from_str(AV_MEDIA_TYPE_VIDEO);
    let device = select_camera_device(&media_type, &entry)?;
    let input = device_input(&device)?;

    let session: Retained<AnyObject> = unsafe { msg_send![class!(AVCaptureSession), new] };
    let output: Retained<AnyObject> = unsafe { msg_send![class!(AVCaptureVideoDataOutput), new] };
    let settings = bgra_video_settings();
    let delegate = LiveVideoFrameDelegate::new(
        sender.clone(),
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

fn rewrite_recording_error(err: anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!(
        "{}",
        err.to_string()
            .replacen(ABILITY_CAMERA_SNAPSHOT, ABILITY_CAMERA_RECORD_START, 1)
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
) -> anyhow::Result<Retained<AVCaptureDevice>> {
    let devices: Retained<NSArray<AVCaptureDevice>> =
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
            let name = unsafe { device.localizedName() };
            if name.to_string() == entry.display_name {
                return Ok(device);
            }
        }
    }

    Ok(devices.objectAtIndex(0))
}

fn device_input(device: &AVCaptureDevice) -> anyhow::Result<Retained<AnyObject>> {
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
    video_data_output_settings(kCVPixelFormatType_32BGRA)
}

fn yuv_video_settings() -> Retained<NSMutableDictionary<AnyObject, AnyObject>> {
    // NV12 is a native input format for Apple's H.264 encoder, avoiding the
    // BGRA -> YUV conversion that the preview/JPEG path legitimately needs.
    video_data_output_settings(kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange)
}

fn video_data_output_settings(
    pixel_format: u32,
) -> Retained<NSMutableDictionary<AnyObject, AnyObject>> {
    let settings = NSMutableDictionary::<AnyObject, AnyObject>::new();
    let format = NSNumber::new_u32(pixel_format);
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
    let image = StridedBgra {
        bytes,
        width: width as u16,
        height: height as u16,
        stride,
    };
    if reject_all_black && image.is_all_black() {
        anyhow::bail!(
            "{ability}: camera returned an all-black frame; \
             reason={REASON_RESOURCE_UNAVAILABLE}"
        );
    }

    let mut jpeg = Vec::new();
    let encoder = jpeg_encoder::Encoder::new(&mut jpeg, 80);
    encoder
        .encode_image(image)
        .map_err(|e| anyhow::anyhow!("jpeg encode failed: {e}"))?;
    Ok(EncodedFrame::new(jpeg, width as u32, height as u32))
}

struct StridedBgra<'a> {
    bytes: &'a [u8],
    width: u16,
    height: u16,
    stride: usize,
}

impl StridedBgra<'_> {
    fn row(&self, y: u16) -> &[u8] {
        let start = usize::from(y) * self.stride;
        &self.bytes[start..start + usize::from(self.width) * 4]
    }

    fn is_all_black(&self) -> bool {
        (0..self.height).all(|y| {
            self.row(y)
                .chunks_exact(4)
                .all(|pixel| pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0)
        })
    }
}

impl jpeg_encoder::ImageBuffer for StridedBgra<'_> {
    fn get_jpeg_color_type(&self) -> jpeg_encoder::JpegColorType {
        jpeg_encoder::JpegColorType::Ycbcr
    }

    fn width(&self) -> u16 {
        self.width
    }

    fn height(&self) -> u16 {
        self.height
    }

    fn fill_buffers(&self, y: u16, buffers: &mut [Vec<u8>; 4]) {
        for pixel in self.row(y).chunks_exact(4) {
            let (luma, cb, cr) = jpeg_encoder::rgb_to_ycbcr(pixel[2], pixel[1], pixel[0]);
            buffers[0].push(luma);
            buffers[1].push(cb);
            buffers[2].push(cr);
        }
    }
}

fn take_completion_sender<T>(slot: &Mutex<Option<T>>) -> Option<T> {
    match slot.lock() {
        Ok(mut guard) => guard.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn capture_queue() -> DispatchRetained<DispatchQueue> {
    DispatchQueue::new("tech.easynet.camera.avfoundation.capture", None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::builtins::resources::media::camera_snapshot::{
        camera_frame_channel, CameraFramePoll,
    };
    use crate::daemon::persistence::resources::{ResourceBinding, ResourceType};
    use serde_json::Value;

    fn physical_camera_entry() -> ResourceEntry {
        ResourceEntry {
            resource_ura: "easynet:///r/probe/resource/probe-camera".to_string(),
            owner_agent: String::new(),
            kind: ResourceType::Camera,
            binding: ResourceBinding::LocalDevice,
            hardware_id: "default-0".to_string(),
            display_name: "default camera".to_string(),
            metadata: Value::Null,
            first_seen_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    #[ignore = "requires a physical macOS camera and TCC authorization"]
    fn physical_camera_preview_delivers_live_jpeg_frames() {
        let (sender, mut stream) = camera_frame_channel();
        open_jpeg_stream(
            physical_camera_entry(),
            CameraStreamOptions {
                fps: 30,
                resolution: Some(super::super::camera_snapshot::CameraVideoResolution {
                    width: 1280,
                    height: 720,
                }),
            },
            sender,
        )
        .expect("open native preview stream");

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut frame_count = 0;
        while frame_count < 3 && Instant::now() < deadline {
            match stream.try_next() {
                CameraFramePoll::Frame(frame) => {
                    assert!(frame.jpeg_bytes.starts_with(&[0xff, 0xd8]));
                    assert!(frame.jpeg_bytes.ends_with(&[0xff, 0xd9]));
                    assert!(frame.width > 0 && frame.height > 0);
                    frame_count += 1;
                }
                CameraFramePoll::Pending => std::thread::sleep(Duration::from_millis(5)),
                CameraFramePoll::Failed(message) => panic!("native preview failed: {message}"),
                CameraFramePoll::Closed => panic!("native preview closed before three frames"),
            }
        }
        assert_eq!(
            frame_count, 3,
            "native preview did not deliver three frames"
        );
    }

    #[test]
    #[ignore = "requires a physical macOS camera and TCC authorization"]
    fn physical_camera_records_and_finalizes_h264_movie() {
        let entry = physical_camera_entry();
        let options = CameraRecordingOptions {
            stream: CameraStreamOptions {
                fps: 30,
                resolution: Some(super::super::camera_snapshot::CameraVideoResolution {
                    width: 1280,
                    height: 720,
                }),
            },
            codec: Some("h264".to_string()),
            max_duration_ms: 5_000,
            max_bytes: 32 * 1024 * 1024,
        };
        let output = std::env::temp_dir().join(format!(
            "easynet-camera-recording-probe-{}.mov",
            uuid::Uuid::new_v4().simple()
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker_output = output.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            record_movie(entry, options, worker_stop, worker_output, ready_tx)
        });

        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("native recorder readiness channel")
            .expect("native recorder becomes ready");
        std::thread::sleep(Duration::from_secs(1));
        stop.store(true, Ordering::Relaxed);
        let artifact = worker
            .join()
            .expect("native recorder thread")
            .expect("native recorder result");
        assert_eq!(artifact.content_type, "video/quicktime");
        assert_eq!(artifact.extension, "mov");
        assert!(artifact.byte_size > 0);
        assert!(artifact.frame_count.unwrap_or_default() > 0);
        assert_eq!(artifact.stop_reason, "stopped");
        assert_eq!(
            std::fs::metadata(&artifact.temp_path).unwrap().len(),
            artifact.byte_size
        );
        std::fs::remove_file(artifact.temp_path).unwrap();
    }
}
