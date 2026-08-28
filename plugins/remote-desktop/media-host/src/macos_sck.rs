// EasyNet CLI — macOS ScreenCaptureKit media-host backend
// ========================================================
//
// Owns exact target resolution, ScreenCaptureKit capture, multi-window
// composition, and VideoToolbox submission inside the killable RemoteApp
// media-host process. Runtime authority, session state, WebRTC, receipts, and
// adaptation policy remain daemon-owned.

#![cfg(all(feature = "native-media", target_os = "macos"))]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchRetained};
use easynet_remoteapp_native_protocol::media_session::{
    CaptureBackend, CaptureProof, EventBody, FailureReason, MediaStats, NativeTargetPlan,
    StartContract, TargetKind, VideoConfig,
};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_core_media::{CMSampleBuffer, CMTime, CMTimeFlags};
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

use super::macos_audio::{
    captured_audio_chunk, AudioCaptureEvent, AudioSink, EncodedOpusPacket, OpusPacketizer,
    CHANNELS as AUDIO_CHANNELS, SAMPLES_PER_CHANNEL as AUDIO_SAMPLES_PER_CHANNEL,
    SAMPLE_RATE_HZ as AUDIO_SAMPLE_RATE_HZ,
};
use super::macos_multiapp::{MultiAppSurfaceCompositor, MultiAppSurfaceTarget};
use super::macos_videotoolbox::{EncoderSession, VideoToolboxEncoder};
use super::{now_ms, BackendEvent, BackendFailure, SessionBackend};

const SCK_LIVE_QUEUE_DEPTH: isize = 3;
const SHAREABLE_CONTENT_TIMEOUT: Duration = Duration::from_secs(10);
const CAPTURE_START_TIMEOUT: Duration = Duration::from_secs(10);
const CAPTURE_STOP_TIMEOUT: Duration = Duration::from_secs(3);
const DIAGNOSTIC_FRAME_TIMEOUT: Duration = Duration::from_secs(3);
const FRAME_STALL_TIMEOUT_MS: u64 = 3_000;
const AUDIO_CAPTURE_QUEUE_DEPTH: usize = 4;
const AUDIO_PACKET_QUEUE_DEPTH: usize = 4;

pub(super) struct CapturedFrame {
    pub image_buffer: Retained<CVImageBuffer>,
    pub pts: CMTime,
}

// SAFETY: CVImageBuffer is a CoreFoundation object with atomic retain/release;
// an owned reference may be transported between the SCK and VT queues.
unsafe impl Send for CapturedFrame {}

pub(super) type FrameSink = Arc<dyn Fn(CapturedFrame) + Send + Sync>;

enum CapturePlan {
    Single(Retained<SCContentFilter>),
    MultiApplication(MultiAppSurfaceTarget),
}

struct ResolvedTarget {
    plan: CapturePlan,
    native_width: u32,
    native_height: u32,
}

struct ActiveStream {
    stream: Retained<SCStream>,
    _delegate: Retained<StreamOutputDelegate>,
}

struct ScreenCaptureKitStream {
    streams: Vec<ActiveStream>,
}

impl ScreenCaptureKitStream {
    fn start(
        target: ResolvedTarget,
        width: u32,
        height: u32,
        fps: u32,
        sink: FrameSink,
        audio_sink: Option<AudioSink>,
    ) -> Result<Self, BackendFailure> {
        let streams = match target.plan {
            CapturePlan::Single(filter) => vec![start_active_stream(
                &filter,
                width as usize,
                height as usize,
                fps,
                sink,
                audio_sink,
            )?],
            CapturePlan::MultiApplication(target) => {
                let surfaces = target
                    .scale_to(width as usize, height as usize)
                    .map_err(capture_unavailable)?;
                let compositor = MultiAppSurfaceCompositor::new(
                    width as usize,
                    height as usize,
                    &surfaces,
                    fps,
                    sink,
                );
                let mut streams = Vec::with_capacity(surfaces.len());
                for (surface_index, surface) in surfaces.into_iter().enumerate() {
                    let compositor = Arc::clone(&compositor);
                    let window_id = surface.window_id;
                    let sink: FrameSink = Arc::new(move |frame| {
                        if let Err(error) = compositor.accept(surface_index, frame) {
                            eprintln!(
                                "[remoteapp-media-host] macOS application surface {window_id} composition failed: {error}"
                            );
                        }
                    });
                    // All surfaces belong to the same committed application.
                    // One filtered SCK stream owns its audio output so the
                    // application mix is not duplicated once per window.
                    let stream_audio_sink = (surface_index == 0)
                        .then(|| audio_sink.as_ref().map(Arc::clone))
                        .flatten();
                    match start_active_stream(
                        &surface.filter,
                        surface.width,
                        surface.height,
                        fps,
                        sink,
                        stream_audio_sink,
                    ) {
                        Ok(stream) => streams.push(stream),
                        Err(error) => {
                            stop_active_streams(&streams);
                            return Err(error);
                        }
                    }
                }
                streams
            }
        };
        Ok(Self { streams })
    }

    fn stop(&mut self) {
        stop_active_streams(&self.streams);
        self.streams.clear();
    }
}

impl Drop for ScreenCaptureKitStream {
    fn drop(&mut self) {
        self.stop();
    }
}

struct DelegateIvars {
    sink: FrameSink,
    audio_sink: Option<AudioSink>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "EasyNetRemoteAppMediaHostSCKOutput"]
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
                    let Some(image_buffer) = (unsafe { sample_buffer.image_buffer() }) else {
                        return;
                    };
                    let pts = unsafe { sample_buffer.presentation_time_stamp() };
                    (self.ivars().sink)(CapturedFrame {
                        image_buffer: image_buffer.into(),
                        pts,
                    });
                }
                SCStreamOutputType::Audio => {
                    if let Some(sink) = &self.ivars().audio_sink {
                        sink(captured_audio_chunk(sample_buffer));
                    }
                }
                _ => {}
            }
        }
    }
);

impl StreamOutputDelegate {
    fn new(sink: FrameSink, audio_sink: Option<AudioSink>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { sink, audio_sink });
        unsafe { msg_send![super(this), init] }
    }
}

pub(super) struct MacOsScreenCaptureKitSessionBackend {
    contract: Option<StartContract>,
    resolved_target: Option<ResolvedTarget>,
    stream: Option<ScreenCaptureKitStream>,
    encoder: Option<VideoToolboxEncoder>,
    encode_enabled: Arc<AtomicBool>,
    audio_enabled: Arc<AtomicBool>,
    audio_chunks: Option<Receiver<AudioCaptureEvent>>,
    audio_packetizer: Option<OpusPacketizer>,
    audio_packets: VecDeque<EncodedOpusPacket>,
    audio_capture_depth: Arc<AtomicU64>,
    audio_capture_dropped: Arc<AtomicU64>,
    audio_packets_dropped: u64,
    encoded_audio_packets: u64,
    audio_pts_48khz: u64,
    audio_discontinuity: bool,
    audio_bytes: u64,
    capture_frames: Arc<AtomicU64>,
    last_frame_at_ms: Arc<AtomicU64>,
    encode_error: Arc<Mutex<Option<String>>>,
    media_gate: u32,
    codec_generation: u32,
    discontinuity: bool,
    video_bytes: u64,
    last_stats_at: Instant,
    media_started_at_ms: u64,
}

impl Default for MacOsScreenCaptureKitSessionBackend {
    fn default() -> Self {
        Self {
            contract: None,
            resolved_target: None,
            stream: None,
            encoder: None,
            encode_enabled: Arc::new(AtomicBool::new(false)),
            audio_enabled: Arc::new(AtomicBool::new(false)),
            audio_chunks: None,
            audio_packetizer: None,
            audio_packets: VecDeque::with_capacity(AUDIO_PACKET_QUEUE_DEPTH),
            audio_capture_depth: Arc::new(AtomicU64::new(0)),
            audio_capture_dropped: Arc::new(AtomicU64::new(0)),
            audio_packets_dropped: 0,
            encoded_audio_packets: 0,
            audio_pts_48khz: 0,
            audio_discontinuity: true,
            audio_bytes: 0,
            capture_frames: Arc::new(AtomicU64::new(0)),
            last_frame_at_ms: Arc::new(AtomicU64::new(0)),
            encode_error: Arc::new(Mutex::new(None)),
            media_gate: 0,
            codec_generation: 1,
            discontinuity: true,
            video_bytes: 0,
            last_stats_at: Instant::now(),
            media_started_at_ms: 0,
        }
    }
}

impl MacOsScreenCaptureKitSessionBackend {
    fn create_audio_sink(&mut self) -> Option<AudioSink> {
        self.contract.as_ref()?.audio.as_ref()?;
        let (sender, receiver) = sync_channel::<AudioCaptureEvent>(AUDIO_CAPTURE_QUEUE_DEPTH);
        self.audio_chunks = Some(receiver);
        self.audio_capture_depth.store(0, Ordering::Release);
        let enabled = Arc::clone(&self.audio_enabled);
        let depth = Arc::clone(&self.audio_capture_depth);
        let dropped = Arc::clone(&self.audio_capture_dropped);
        Some(Arc::new(move |event| {
            if !enabled.load(Ordering::Acquire) {
                return;
            }
            depth.fetch_add(1, Ordering::Relaxed);
            match sender.try_send(event) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    depth.fetch_sub(1, Ordering::Relaxed);
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
                Err(TrySendError::Disconnected(_)) => {
                    depth.fetch_sub(1, Ordering::Relaxed);
                }
            }
        }))
    }

    fn clear_audio_queues(&mut self) {
        if let Some(receiver) = &self.audio_chunks {
            while receiver.try_recv().is_ok() {}
        }
        self.audio_chunks = None;
        self.audio_packets.clear();
        self.audio_capture_depth.store(0, Ordering::Release);
    }

    fn queue_encoded_audio(
        &mut self,
        chunk: super::macos_audio::CapturedAudioChunk,
    ) -> Result<(), BackendFailure> {
        let packetizer = self
            .audio_packetizer
            .as_mut()
            .ok_or_else(|| internal("macOS audio arrived without an Opus packetizer"))?;
        let packets = packetizer.push_chunk(chunk).map_err(|error| {
            BackendFailure::new(
                FailureReason::AudioUnavailable,
                format!("macOS Opus packetization failed: {error}"),
            )
        })?;
        for packet in packets {
            if self.audio_packets.len() == AUDIO_PACKET_QUEUE_DEPTH {
                self.audio_packets.pop_front();
                self.audio_packets_dropped = self.audio_packets_dropped.saturating_add(1);
            }
            self.audio_packets.push_back(packet);
        }
        Ok(())
    }

    fn poll_audio(&mut self) -> Result<Option<BackendEvent>, BackendFailure> {
        let event = self
            .audio_chunks
            .as_ref()
            .map(Receiver::try_recv)
            .transpose();
        match event {
            Ok(Some(event)) => {
                self.audio_capture_depth.fetch_sub(1, Ordering::Relaxed);
                match event {
                    Ok(chunk) => self.queue_encoded_audio(chunk)?,
                    Err(detail) => {
                        return Err(BackendFailure::new(
                            FailureReason::AudioUnavailable,
                            format!("ScreenCaptureKit audio extraction failed: {detail}"),
                        ));
                    }
                }
            }
            Ok(None) | Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) if self.audio_enabled.load(Ordering::Acquire) => {
                return Err(BackendFailure::new(
                    FailureReason::AudioUnavailable,
                    "ScreenCaptureKit audio output disconnected during active media",
                ));
            }
            Err(TryRecvError::Disconnected) => {}
        }
        let Some(packet) = self.audio_packets.pop_front() else {
            return Ok(None);
        };
        let pts_48khz = self.audio_pts_48khz;
        self.audio_pts_48khz = self
            .audio_pts_48khz
            .checked_add(AUDIO_SAMPLES_PER_CHANNEL as u64)
            .ok_or_else(|| internal("macOS audio timestamp overflow"))?;
        let discontinuity = self.audio_discontinuity;
        self.audio_discontinuity = false;
        self.encoded_audio_packets = self.encoded_audio_packets.saturating_add(1);
        self.audio_bytes = self.audio_bytes.saturating_add(packet.payload.len() as u64);
        Ok(Some(BackendEvent::Audio {
            body: EventBody::AudioOpus {
                media_gate: self.media_gate,
                pts_48khz,
                duration_samples: AUDIO_SAMPLES_PER_CHANNEL as u16,
                discontinuity,
                sample_rate_hz: AUDIO_SAMPLE_RATE_HZ,
                channels: AUDIO_CHANNELS as u8,
            },
            payload: packet.payload,
        }))
    }

    fn start_stream(&mut self) -> Result<(), BackendFailure> {
        if self.stream.is_some() {
            return Ok(());
        }
        let target = self
            .resolved_target
            .take()
            .ok_or_else(|| internal("macOS capture target missing before stream start"))?;
        let video = self
            .contract
            .as_ref()
            .ok_or_else(|| internal("macOS media contract missing before stream start"))?
            .video
            .clone();
        let encoder_session = self
            .encoder
            .as_ref()
            .ok_or_else(|| internal("VideoToolbox encoder missing before stream start"))?
            .session();
        let sink = encoding_sink(
            encoder_session,
            video.fps,
            Arc::clone(&self.encode_enabled),
            Arc::clone(&self.capture_frames),
            Arc::clone(&self.last_frame_at_ms),
            Arc::clone(&self.encode_error),
        );
        let audio_sink = self.create_audio_sink();
        self.stream = Some(ScreenCaptureKitStream::start(
            target,
            video.width,
            video.height,
            video.fps,
            sink,
            audio_sink,
        )?);
        Ok(())
    }

    fn retire_stream(&mut self) {
        self.encode_enabled.store(false, Ordering::Release);
        self.audio_enabled.store(false, Ordering::Release);
        if let Some(mut stream) = self.stream.take() {
            stream.stop();
        }
        self.clear_audio_queues();
    }

    fn take_encode_error(&self) -> Option<String> {
        self.encode_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    fn observe_stall(&self) -> Option<BackendFailure> {
        if !self.encode_enabled.load(Ordering::Acquire) || self.media_started_at_ms == 0 {
            return None;
        }
        let last = self.last_frame_at_ms.load(Ordering::Acquire);
        let reference = last.max(self.media_started_at_ms);
        if now_ms().saturating_sub(reference) < FRAME_STALL_TIMEOUT_MS {
            return None;
        }
        if !super::platform_screen_capture_permission_granted() {
            Some(BackendFailure::new(
                FailureReason::PermissionRevoked,
                "macOS Screen Recording permission was revoked during the active media generation",
            ))
        } else {
            Some(BackendFailure::new(
                FailureReason::CaptureUnavailable,
                "ScreenCaptureKit produced no frame within the bounded live-media deadline",
            ))
        }
    }
}

impl SessionBackend for MacOsScreenCaptureKitSessionBackend {
    fn prepare(&mut self, contract: &StartContract) -> Result<CaptureProof, BackendFailure> {
        let target = resolve_target(&contract.target)?;
        let proof = CaptureProof {
            backend: CaptureBackend::ScreenCaptureKit,
            observed_target: contract.target.clone(),
            native_width: target.native_width,
            native_height: target.native_height,
            verified_at_ms: now_ms(),
        };
        self.encoder = Some(build_encoder(&contract.video)?);
        self.audio_packetizer = contract
            .audio
            .as_ref()
            .map(|_| OpusPacketizer::new())
            .transpose()
            .map_err(|error| {
                BackendFailure::new(
                    FailureReason::AudioUnavailable,
                    format!("initialize macOS Opus packetizer: {error}"),
                )
            })?;
        self.audio_pts_48khz = 0;
        self.audio_discontinuity = true;
        self.audio_capture_dropped.store(0, Ordering::Relaxed);
        self.audio_packets_dropped = 0;
        self.encoded_audio_packets = 0;
        self.audio_bytes = 0;
        self.resolved_target = Some(target);
        self.contract = Some(contract.clone());
        self.last_stats_at = Instant::now();
        Ok(proof)
    }

    fn activate(&mut self) -> Result<(), BackendFailure> {
        self.start_stream()
    }

    fn begin_media(&mut self, media_gate: u32) -> Result<(), BackendFailure> {
        if media_gate == 0 || self.stream.is_none() {
            return Err(internal(
                "macOS media began without an active capture generation",
            ));
        }
        self.media_gate = media_gate;
        self.discontinuity = true;
        self.media_started_at_ms = now_ms();
        self.last_frame_at_ms.store(0, Ordering::Release);
        self.encoder
            .as_ref()
            .ok_or_else(|| internal("VideoToolbox encoder missing at media begin"))?
            .request_keyframe();
        self.encode_enabled.store(true, Ordering::Release);
        if self
            .contract
            .as_ref()
            .is_some_and(|contract| contract.audio.is_some())
        {
            self.audio_enabled.store(true, Ordering::Release);
        }
        Ok(())
    }

    fn reconfigure(
        &mut self,
        video: &VideoConfig,
        force_keyframe: bool,
    ) -> Result<(), BackendFailure> {
        self.retire_stream();
        let contract = self
            .contract
            .as_mut()
            .ok_or_else(|| internal("macOS reconfigure before preparation"))?;
        contract.video = video.clone();
        self.resolved_target = Some(resolve_target(&contract.target)?);
        self.encoder = Some(build_encoder(video)?);
        self.audio_packetizer = contract
            .audio
            .as_ref()
            .map(|_| OpusPacketizer::new())
            .transpose()
            .map_err(|error| {
                BackendFailure::new(
                    FailureReason::AudioUnavailable,
                    format!("reinitialize macOS Opus packetizer: {error}"),
                )
            })?;
        self.codec_generation = self
            .codec_generation
            .checked_add(1)
            .ok_or_else(|| internal("macOS codec generation overflow"))?;
        if force_keyframe {
            self.encoder
                .as_ref()
                .expect("encoder was just installed")
                .request_keyframe();
        }
        self.discontinuity = true;
        self.audio_discontinuity = true;
        Ok(())
    }

    fn resume_media(&mut self, media_gate: u32) -> Result<(), BackendFailure> {
        if media_gate == 0 {
            return Err(internal("macOS media resumed without a generation gate"));
        }
        self.start_stream()?;
        self.media_gate = media_gate;
        self.discontinuity = true;
        self.media_started_at_ms = now_ms();
        self.last_frame_at_ms.store(0, Ordering::Release);
        self.encoder
            .as_ref()
            .ok_or_else(|| internal("VideoToolbox encoder missing at media resume"))?
            .request_keyframe();
        self.encode_enabled.store(true, Ordering::Release);
        if self
            .contract
            .as_ref()
            .is_some_and(|contract| contract.audio.is_some())
        {
            self.audio_enabled.store(true, Ordering::Release);
        }
        Ok(())
    }

    fn request_keyframe(&mut self) -> Result<(), BackendFailure> {
        self.encoder
            .as_ref()
            .ok_or_else(|| internal("macOS keyframe requested before preparation"))?
            .request_keyframe();
        Ok(())
    }

    fn poll(&mut self, timeout: Duration) -> Result<Option<BackendEvent>, BackendFailure> {
        if let Some(detail) = self.take_encode_error() {
            return Err(BackendFailure::new(
                FailureReason::EncoderUnavailable,
                detail,
            ));
        }
        if let Some(failure) = self.observe_stall() {
            return Err(failure);
        }
        if let Some(event) = self.poll_audio()? {
            return Ok(Some(event));
        }
        let unit = self
            .encoder
            .as_ref()
            .and_then(VideoToolboxEncoder::poll_one);
        if let Some(unit) = unit {
            let video = &self
                .contract
                .as_ref()
                .ok_or_else(|| internal("macOS encoded unit has no media contract"))?
                .video;
            if unit.annexb.len() > video.max_access_unit_bytes as usize {
                return Err(BackendFailure::new(
                    FailureReason::EncoderUnavailable,
                    format!(
                        "VideoToolbox access unit {} exceeds negotiated {} byte bound",
                        unit.annexb.len(),
                        video.max_access_unit_bytes
                    ),
                ));
            }
            let (_, sps_pps_present) = inspect_annex_b(&unit.annexb);
            let discontinuity = self.discontinuity;
            self.discontinuity = false;
            self.video_bytes = self.video_bytes.saturating_add(unit.annexb.len() as u64);
            return Ok(Some(BackendEvent::Video {
                body: EventBody::VideoH264 {
                    media_gate: self.media_gate,
                    pts_90khz: unit.pts_ms.saturating_mul(90),
                    duration_90khz: (90_000 / video.fps.max(1)).max(1),
                    keyframe: unit.is_keyframe,
                    sps_pps_present,
                    discontinuity,
                    codec_generation: self.codec_generation,
                    width: video.width,
                    height: video.height,
                    encode_submitted_at_ms: unit.encode_submitted_at_ms,
                    encoded_at_ms: unit.encoded_at_ms.max(unit.encode_submitted_at_ms),
                },
                payload: unit.annexb,
            }));
        }
        if self.last_stats_at.elapsed() >= Duration::from_secs(1) {
            self.last_stats_at = Instant::now();
            let stats = self
                .encoder
                .as_ref()
                .ok_or_else(|| internal("macOS stats requested without encoder"))?
                .stats();
            return Ok(Some(BackendEvent::Stats(MediaStats {
                capture_frames: self.capture_frames.load(Ordering::Relaxed),
                encoded_video_frames: stats.emitted_units,
                encoded_audio_packets: self.encoded_audio_packets,
                raw_video_frames_dropped: stats.input_dropped_frames,
                encoded_video_frames_dropped: stats.output_dropped_units,
                audio_packets_dropped: self
                    .audio_capture_dropped
                    .load(Ordering::Relaxed)
                    .saturating_add(self.audio_packets_dropped),
                video_queue_depth: stats.queued_units as u32,
                audio_queue_depth: self
                    .audio_capture_depth
                    .load(Ordering::Relaxed)
                    .max(self.audio_packets.len() as u64)
                    .min(AUDIO_PACKET_QUEUE_DEPTH as u64) as u32,
                video_bytes: self.video_bytes,
                audio_bytes: self.audio_bytes,
            })));
        }
        std::thread::park_timeout(timeout);
        Ok(None)
    }

    fn stop(&mut self) -> Result<(), BackendFailure> {
        self.retire_stream();
        self.encoder.take();
        self.audio_packetizer.take();
        self.resolved_target.take();
        self.contract.take();
        Ok(())
    }
}

pub(super) struct DiagnosticFrame {
    pub(super) capture_proof: CaptureProof,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) jpeg: Vec<u8>,
}

pub(super) fn probe_target(target: &NativeTargetPlan) -> Result<CaptureProof, BackendFailure> {
    let resolved = resolve_target(target)?;
    Ok(CaptureProof {
        backend: CaptureBackend::ScreenCaptureKit,
        observed_target: target.clone(),
        native_width: resolved.native_width,
        native_height: resolved.native_height,
        verified_at_ms: now_ms(),
    })
}

pub(super) fn capture_diagnostic_jpeg(
    target: &NativeTargetPlan,
    width: u32,
    height: u32,
) -> Result<DiagnosticFrame, BackendFailure> {
    let resolved = resolve_target(target)?;
    let capture_proof = CaptureProof {
        backend: CaptureBackend::ScreenCaptureKit,
        observed_target: target.clone(),
        native_width: resolved.native_width,
        native_height: resolved.native_height,
        verified_at_ms: now_ms(),
    };
    let (sender, receiver) = sync_channel::<CapturedFrame>(1);
    let sender = Arc::new(Mutex::new(Some(sender)));
    let sink: FrameSink = Arc::new(move |frame| {
        if let Some(sender) = take_completion_sender(&sender) {
            let _ = sender.send(frame);
        }
    });
    let stream = ScreenCaptureKitStream::start(resolved, width, height, 1, sink, None)?;
    let frame = receiver
        .recv_timeout(DIAGNOSTIC_FRAME_TIMEOUT)
        .map_err(|_| {
            capture_unavailable(
                "ScreenCaptureKit did not produce a diagnostic frame within 3 seconds",
            )
        })?;
    drop(stream);
    let jpeg = encode_diagnostic_frame(&frame.image_buffer, width, height)?;
    Ok(DiagnosticFrame {
        capture_proof,
        width,
        height,
        jpeg,
    })
}

fn encode_diagnostic_frame(
    image_buffer: &CVImageBuffer,
    expected_width: u32,
    expected_height: u32,
) -> Result<Vec<u8>, BackendFailure> {
    let format = CVPixelBufferGetPixelFormatType(image_buffer);
    if format != kCVPixelFormatType_32BGRA {
        return Err(capture_unavailable(format!(
            "ScreenCaptureKit diagnostic pixel format 0x{format:08x} is not BGRA"
        )));
    }
    let flags = CVPixelBufferLockFlags::ReadOnly;
    let lock = unsafe { CVPixelBufferLockBaseAddress(image_buffer, flags) };
    if lock != kCVReturnSuccess {
        return Err(capture_unavailable(format!(
            "diagnostic CVPixelBuffer lock failed with {lock}"
        )));
    }
    let encoded = (|| {
        let width = CVPixelBufferGetWidth(image_buffer);
        let height = CVPixelBufferGetHeight(image_buffer);
        let stride = CVPixelBufferGetBytesPerRow(image_buffer);
        let base = CVPixelBufferGetBaseAddress(image_buffer);
        if width != expected_width as usize
            || height != expected_height as usize
            || stride < width.saturating_mul(4)
            || base.is_null()
        {
            return Err(capture_unavailable(format!(
                "diagnostic pixel buffer differs from requested dimensions; requested={expected_width}x{expected_height}, observed={width}x{height}, stride={stride}"
            )));
        }
        let bytes = unsafe { std::slice::from_raw_parts(base.cast::<u8>(), stride * height) };
        let mut rgb = Vec::with_capacity(width.saturating_mul(height).saturating_mul(3));
        for row in bytes.chunks_exact(stride).take(height) {
            for pixel in row[..width * 4].chunks_exact(4) {
                rgb.extend_from_slice(&[pixel[2], pixel[1], pixel[0]]);
            }
        }
        let mut jpeg = Vec::new();
        jpeg_encoder::Encoder::new(&mut jpeg, 80)
            .encode(
                &rgb,
                expected_width as u16,
                expected_height as u16,
                jpeg_encoder::ColorType::Rgb,
            )
            .map_err(|error| capture_unavailable(format!("encode diagnostic JPEG: {error}")))?;
        if jpeg.is_empty()
            || jpeg.len()
                > easynet_remoteapp_native_protocol::capture_probe::MAX_DIAGNOSTIC_JPEG_BYTES
        {
            return Err(capture_unavailable(format!(
                "diagnostic JPEG size {} exceeds private control-frame bound",
                jpeg.len()
            )));
        }
        Ok(jpeg)
    })();
    let _ = unsafe { CVPixelBufferUnlockBaseAddress(image_buffer, flags) };
    encoded
}

fn build_encoder(video: &VideoConfig) -> Result<VideoToolboxEncoder, BackendFailure> {
    VideoToolboxEncoder::new_with_wakeup_and_limits(
        video.width as i32,
        video.height as i32,
        video.bitrate_kbps,
        video.keyframe_interval_frames,
        video.fps,
        video.max_nal_unit_bytes,
        None,
        video.max_pending_frames as usize,
        video.max_pending_frames as usize,
    )
    .map_err(|error| {
        BackendFailure::new(
            FailureReason::EncoderUnavailable,
            format!("create VideoToolbox encoder: {error}"),
        )
    })
}

fn encoding_sink(
    encoder: EncoderSession,
    fps: u32,
    enabled: Arc<AtomicBool>,
    capture_frames: Arc<AtomicU64>,
    last_frame_at_ms: Arc<AtomicU64>,
    encode_error: Arc<Mutex<Option<String>>>,
) -> FrameSink {
    let duration = CMTime {
        value: 1,
        timescale: fps.max(1) as i32,
        flags: CMTimeFlags::Valid,
        epoch: 0,
    };
    Arc::new(move |frame| {
        last_frame_at_ms.store(now_ms(), Ordering::Release);
        capture_frames.fetch_add(1, Ordering::Relaxed);
        if !enabled.load(Ordering::Acquire) {
            return;
        }
        if let Err(error) = encoder.encode(&frame.image_buffer, frame.pts, duration) {
            let mut slot = encode_error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if slot.is_none() {
                *slot = Some(format!("VideoToolbox frame submission failed: {error}"));
            }
        }
    })
}

fn resolve_target(target: &NativeTargetPlan) -> Result<ResolvedTarget, BackendFailure> {
    if !super::platform_screen_capture_permission_granted() {
        return Err(BackendFailure::new(
            FailureReason::PermissionDenied,
            format!(
                "macOS Screen Recording permission is not granted to media-host executable {}",
                std::env::current_exe()
                    .ok()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "<unknown>".into())
            ),
        ));
    }
    let content = shareable_content()?;
    match target.kind {
        TargetKind::Display => resolve_display(&content, target),
        TargetKind::Window => resolve_window(&content, target),
        TargetKind::Application => resolve_application(&content, target),
    }
}

fn resolve_display(
    content: &SCShareableContent,
    target: &NativeTargetPlan,
) -> Result<ResolvedTarget, BackendFailure> {
    let expected = target
        .display_id
        .ok_or_else(|| internal("display contract has no exact display id"))?;
    let displays = unsafe { content.displays() };
    let display = displays
        .iter()
        .find(|display| unsafe { display.displayID() as u64 == expected })
        .ok_or_else(|| {
            target_invalidated(format!("ScreenCaptureKit display {expected} disappeared"))
        })?;
    let empty: Retained<NSArray<SCWindow>> = NSArray::new();
    let filter = unsafe {
        SCContentFilter::initWithDisplay_excludingWindows(
            SCContentFilter::alloc(),
            &display,
            &empty,
        )
    };
    let (native_width, native_height) = filter_dimensions(&filter, Some(&display))?;
    Ok(ResolvedTarget {
        plan: CapturePlan::Single(filter),
        native_width,
        native_height,
    })
}

fn resolve_window(
    content: &SCShareableContent,
    target: &NativeTargetPlan,
) -> Result<ResolvedTarget, BackendFailure> {
    let expected = target
        .window_id
        .ok_or_else(|| internal("window contract has no exact window id"))?;
    let windows = unsafe { content.windows() };
    let mut matching_id = windows
        .iter()
        .filter(|window| unsafe { window.windowID() as u64 == expected });
    let window = matching_id.next().ok_or_else(|| {
        target_invalidated(format!("ScreenCaptureKit window {expected} disappeared"))
    })?;
    if matching_id.next().is_some() {
        return Err(target_invalidated(format!(
            "ScreenCaptureKit window id {expected} is ambiguous"
        )));
    }
    let owner = unsafe { window.owningApplication() }.ok_or_else(|| {
        target_invalidated(format!("window {expected} lost its owning application"))
    })?;
    validate_application_identity(target, &owner)?;
    let filter = unsafe {
        SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window)
    };
    let (native_width, native_height) = filter_dimensions(&filter, None)?;
    Ok(ResolvedTarget {
        plan: CapturePlan::Single(filter),
        native_width,
        native_height,
    })
}

fn resolve_application(
    content: &SCShareableContent,
    target: &NativeTargetPlan,
) -> Result<ResolvedTarget, BackendFailure> {
    let contract = target
        .application
        .as_ref()
        .ok_or_else(|| internal("application contract has no exact window-set proof"))?;
    let applications = unsafe { content.applications() };
    let mut matches = applications
        .iter()
        .filter(|application| application_identity_matches(target, application));
    let application = matches.next().ok_or_else(|| {
        target_invalidated(format!(
            "ScreenCaptureKit application pid {} is no longer present",
            contract.primary_pid
        ))
    })?;
    if matches.next().is_some() {
        return Err(target_invalidated(
            "ScreenCaptureKit application identity is ambiguous",
        ));
    }
    validate_application_identity(target, &application)?;

    let windows = unsafe { content.windows() };
    let observed = windows
        .iter()
        .filter(|window| {
            unsafe { window.owningApplication() }
                .as_deref()
                .is_some_and(|owner| application_identity_matches(target, owner))
        })
        .collect::<Vec<_>>();
    let observed_ids = observed
        .iter()
        .map(|window| unsafe { window.windowID() as u64 })
        .collect::<Vec<_>>();
    let expected_front_to_back = contract
        .front_to_back_surfaces
        .iter()
        .map(|surface| surface.window_id)
        .collect::<Vec<_>>();
    if observed_ids != expected_front_to_back {
        return Err(target_invalidated(format!(
            "application window order/membership changed; expected={expected_front_to_back:?}, observed={observed_ids:?}"
        )));
    }
    let mut sorted_observed = observed_ids.clone();
    sorted_observed.sort_unstable();
    if sorted_observed != contract.window_ids {
        return Err(target_invalidated(format!(
            "application window set changed; expected={:?}, observed={sorted_observed:?}",
            contract.window_ids
        )));
    }
    for (window, expected) in observed.iter().zip(&contract.front_to_back_surfaces) {
        let frame = unsafe { window.frame() };
        let actual = (
            frame.origin.x.round() as i64,
            frame.origin.y.round() as i64,
            frame.size.width.round() as u32,
            frame.size.height.round() as u32,
        );
        let required = (expected.x, expected.y, expected.width, expected.height);
        if actual != required {
            return Err(target_invalidated(format!(
                "application window {} geometry changed; expected={required:?}, observed={actual:?}",
                expected.window_id
            )));
        }
    }
    let multi = MultiAppSurfaceTarget::from_windows(observed).map_err(target_invalidated)?;
    let (width, height) = multi.native_dimensions();
    let native_width = u32::try_from(width)
        .map_err(|_| capture_unavailable("application native width exceeds u32"))?;
    let native_height = u32::try_from(height)
        .map_err(|_| capture_unavailable("application native height exceeds u32"))?;
    Ok(ResolvedTarget {
        plan: CapturePlan::MultiApplication(multi),
        native_width,
        native_height,
    })
}

fn validate_application_identity(
    target: &NativeTargetPlan,
    application: &SCRunningApplication,
) -> Result<(), BackendFailure> {
    if application_identity_matches(target, application) {
        Ok(())
    } else {
        Err(target_invalidated(format!(
            "native application identity changed; expected pid={:?} app_identity={:?} bundle_id={:?}, observed pid={} bundle_id={:?}",
            target.pid,
            target.app_identity,
            target.bundle_id,
            unsafe { application.processID() },
            application_bundle_id(application)
        )))
    }
}

fn application_identity_matches(
    target: &NativeTargetPlan,
    application: &SCRunningApplication,
) -> bool {
    let pid = unsafe { application.processID() as i64 };
    let bundle = application_bundle_id(application);
    target.pid == Some(pid)
        && target
            .app_identity
            .as_deref()
            .is_none_or(|expected| bundle.as_deref() == Some(expected))
        && target
            .bundle_id
            .as_deref()
            .is_none_or(|expected| bundle.as_deref() == Some(expected))
}

fn application_bundle_id(application: &SCRunningApplication) -> Option<String> {
    let bundle = unsafe { application.bundleIdentifier() }.to_string();
    let bundle = bundle.trim();
    (!bundle.is_empty()).then(|| bundle.to_owned())
}

fn shareable_content() -> Result<Retained<SCShareableContent>, BackendFailure> {
    let (sender, receiver) = sync_channel::<Result<Retained<SCShareableContent>, String>>(1);
    let sender = Mutex::new(Some(sender));
    let completion = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            let result = if !error.is_null() {
                Err(unsafe { &*error }.localizedDescription().to_string())
            } else if content.is_null() {
                Err("SCShareableContent returned null".into())
            } else {
                unsafe { Retained::retain(content) }
                    .ok_or_else(|| "SCShareableContent retain failed".into())
            };
            if let Some(sender) = take_completion_sender(&sender) {
                let _ = sender.send(result);
            }
        },
    );
    unsafe {
        SCShareableContent::getShareableContentWithCompletionHandler(&completion);
    }
    match receiver.recv_timeout(SHAREABLE_CONTENT_TIMEOUT) {
        Ok(Ok(content)) => Ok(content),
        Ok(Err(error)) => Err(capture_unavailable(format!(
            "ScreenCaptureKit enumeration failed: {error}"
        ))),
        Err(_) => Err(capture_unavailable(
            "ScreenCaptureKit enumeration timed out after 10 seconds",
        )),
    }
}

fn filter_dimensions(
    filter: &SCContentFilter,
    display: Option<&SCDisplay>,
) -> Result<(u32, u32), BackendFailure> {
    let info = unsafe { SCShareableContent::infoForFilter(filter) };
    let scale = f64::from(unsafe { info.pointPixelScale() }.max(1.0));
    let (width, height) = if let Some(display) = display {
        (
            unsafe { display.width() } as f64 * scale,
            unsafe { display.height() } as f64 * scale,
        )
    } else {
        let rect = unsafe { info.contentRect() };
        (rect.size.width * scale, rect.size.height * scale)
    };
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err(capture_unavailable(format!(
            "ScreenCaptureKit returned invalid native dimensions {width}x{height}"
        )));
    }
    Ok((width.round() as u32, height.round() as u32))
}

fn start_active_stream(
    filter: &SCContentFilter,
    width: usize,
    height: usize,
    fps: u32,
    sink: FrameSink,
    audio_sink: Option<AudioSink>,
) -> Result<ActiveStream, BackendFailure> {
    let captures_audio = audio_sink.is_some();
    let configuration = unsafe {
        let configuration = SCStreamConfiguration::new();
        configuration.setWidth(width);
        configuration.setHeight(height);
        configuration.setPixelFormat(u32::from_be_bytes(*b"BGRA"));
        configuration.setMinimumFrameInterval(CMTime {
            value: 1,
            timescale: fps.max(1) as i32,
            flags: CMTimeFlags::Valid,
            epoch: 0,
        });
        configuration.setQueueDepth(SCK_LIVE_QUEUE_DEPTH);
        configuration.setCapturesAudio(captures_audio);
        if captures_audio {
            configuration.setSampleRate(AUDIO_SAMPLE_RATE_HZ as isize);
            configuration.setChannelCount(AUDIO_CHANNELS as isize);
            configuration.setExcludesCurrentProcessAudio(true);
        }
        configuration
    };
    let delegate = StreamOutputDelegate::new(sink, audio_sink);
    let stream = unsafe {
        SCStream::initWithFilter_configuration_delegate(
            SCStream::alloc(),
            filter,
            &configuration,
            None,
        )
    };
    let queue = capture_queue();
    let output = ProtocolObject::from_ref(&*delegate);
    unsafe {
        stream
            .addStreamOutput_type_sampleHandlerQueue_error(
                output,
                SCStreamOutputType::Screen,
                Some(&queue),
            )
            .map_err(|error| {
                capture_unavailable(format!(
                    "ScreenCaptureKit add output failed: {}",
                    error.localizedDescription()
                ))
            })?;
        if captures_audio {
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    output,
                    SCStreamOutputType::Audio,
                    Some(&audio_capture_queue()),
                )
                .map_err(|error| {
                    capture_unavailable(format!(
                        "ScreenCaptureKit add audio output failed: {}",
                        error.localizedDescription()
                    ))
                })?;
        }
    }
    start_capture_sync(&stream)?;
    Ok(ActiveStream {
        stream,
        _delegate: delegate,
    })
}

fn start_capture_sync(stream: &SCStream) -> Result<(), BackendFailure> {
    let (sender, receiver) = sync_channel::<Result<(), String>>(1);
    let sender = Mutex::new(Some(sender));
    let completion = RcBlock::new(move |error: *mut NSError| {
        let result = if error.is_null() {
            Ok(())
        } else {
            Err(unsafe { &*error }.localizedDescription().to_string())
        };
        if let Some(sender) = take_completion_sender(&sender) {
            let _ = sender.send(result);
        }
    });
    unsafe {
        stream.startCaptureWithCompletionHandler(Some(&completion));
    }
    match receiver.recv_timeout(CAPTURE_START_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(capture_unavailable(format!(
            "ScreenCaptureKit start failed: {error}"
        ))),
        Err(_) => Err(capture_unavailable(
            "ScreenCaptureKit start timed out after 10 seconds",
        )),
    }
}

fn stop_active_streams(streams: &[ActiveStream]) {
    for active in streams {
        let (sender, receiver) = sync_channel::<()>(1);
        let sender = Mutex::new(Some(sender));
        let completion = RcBlock::new(move |_error: *mut NSError| {
            if let Some(sender) = take_completion_sender(&sender) {
                let _ = sender.send(());
            }
        });
        unsafe {
            active
                .stream
                .stopCaptureWithCompletionHandler(Some(&completion));
        }
        let _ = receiver.recv_timeout(CAPTURE_STOP_TIMEOUT);
    }
}

fn capture_queue() -> DispatchRetained<DispatchQueue> {
    DispatchQueue::new("tech.easynet.remoteapp.media-host.sck", None)
}

fn audio_capture_queue() -> DispatchRetained<DispatchQueue> {
    DispatchQueue::new("tech.easynet.remoteapp.media-host.sck-audio", None)
}

fn take_completion_sender<T>(slot: &Mutex<Option<SyncSender<T>>>) -> Option<SyncSender<T>> {
    match slot.lock() {
        Ok(mut guard) => guard.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

fn inspect_annex_b(payload: &[u8]) -> (bool, bool) {
    let mut keyframe = false;
    let mut sps = false;
    let mut pps = false;
    let mut index = 0;
    while index + 3 < payload.len() {
        let start_len = if payload[index..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if payload[index..].starts_with(&[0, 0, 1]) {
            3
        } else {
            index += 1;
            continue;
        };
        if let Some(header) = payload.get(index + start_len) {
            match header & 0x1f {
                5 => keyframe = true,
                7 => sps = true,
                8 => pps = true,
                _ => {}
            }
        }
        index += start_len + 1;
    }
    (keyframe, sps && pps)
}

fn target_invalidated(detail: impl std::fmt::Display) -> BackendFailure {
    BackendFailure::new(FailureReason::TargetInvalidated, detail.to_string())
}

fn capture_unavailable(detail: impl std::fmt::Display) -> BackendFailure {
    BackendFailure::new(FailureReason::CaptureUnavailable, detail.to_string())
}

fn internal(detail: impl Into<String>) -> BackendFailure {
    BackendFailure::new(FailureReason::Internal, detail)
}

#[cfg(test)]
mod tests {
    use super::inspect_annex_b;

    #[test]
    fn annex_b_inspection_requires_parameter_sets_for_keyframe_recovery() {
        assert_eq!(
            inspect_annex_b(&[0, 0, 0, 1, 0x67, 1, 0, 0, 1, 0x68, 1, 0, 0, 1, 0x65]),
            (true, true)
        );
        assert_eq!(inspect_annex_b(&[0, 0, 1, 0x41, 1]), (false, false));
    }
}
