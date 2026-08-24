// EasyNet CLI — direct WebRTC native media path
// ==============================================
//
// File: plugins/remote-desktop/src/transport/webrtc_native_media.rs
// Description: macOS ScreenCaptureKit + VideoToolbox strategy for direct WebRTC.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use rtc::media::Sample;
use serde_json::{json, Map, Value};
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::Track;
use webrtc::peer_connection::PeerConnection;

use crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenCaptureOptions;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_SET_DESCRIPTION;
use crate::daemon::plugins::remote_desktop::media::encode::BuiltinH264Config;
use crate::daemon::plugins::remote_desktop::media::native::{
    is_webrtc_sender_backpressure, latest_native_rtp_units, native_capture_dimensions,
    native_rtp_sample_duration, webrtc_cmtime, webrtc_stats_snapshot, NativeAdaptiveBitrate,
    NativeLatencyStats, NativeReceiverPressureTracker,
};
use crate::daemon::plugins::remote_desktop::screencapturekit_audio::{
    AudioCaptureEvent, AudioSink, RemoteAppOpusEncoder, REMOTEAPP_AUDIO_CHANNELS,
    REMOTEAPP_AUDIO_SAMPLE_RATE_HZ,
};
use crate::daemon::plugins::remote_desktop::screencapturekit_capture::{
    target_for_binding, target_for_pending_application_rebind, CapturedFrame,
    ScreenCaptureKitSinks, ScreenCaptureKitStream,
};
use crate::daemon::plugins::remote_desktop::session::now_ms;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, RemoteAppTargetError,
};
use crate::daemon::plugins::remote_desktop::transport::webrtc_media::DirectWebRtcMediaExecution;
use crate::daemon::plugins::remote_desktop::videotoolbox_encoder::VideoToolboxEncoder;

const NATIVE_WEBRTC_VIDEO_CODEC: &str = "h264";
const NATIVE_WEBRTC_VIDEO_PAYLOAD_CONTENT_TYPE: &str = "video/h264; stream-format=annexb";
const NATIVE_WEBRTC_VIDEO_TRANSPORT: &str = "webrtc";
const NATIVE_WEBRTC_AUDIO_CODEC: &str = "opus";
const NATIVE_WEBRTC_AUDIO_PAYLOAD_CONTENT_TYPE: &str = "audio/opus";
const NATIVE_MEDIA_PIPELINE_STATS_CONTRACT: &str = "remoteapp_media_pipeline_stats_v1";
const NATIVE_AUDIO_CAPTURE_QUEUE_DEPTH: usize = 4;

/// A single replaceable pending value for a live-media worker.
///
/// The producer never waits for a slow transport. While one sample is being
/// written, newer samples replace the pending sample so recovery resumes from
/// the freshest desktop state instead of replaying stale frames.
#[derive(Debug)]
struct LatestPendingWrite<T> {
    value: Mutex<Option<T>>,
}

impl<T> Default for LatestPendingWrite<T> {
    fn default() -> Self {
        Self {
            value: Mutex::new(None),
        }
    }
}

impl<T> LatestPendingWrite<T> {
    fn replace(&self, value: T) -> bool {
        self.value
            .lock()
            .expect("latest pending media write mutex poisoned")
            .replace(value)
            .is_some()
    }

    fn take(&self) -> Option<T> {
        self.value
            .lock()
            .expect("latest pending media write mutex poisoned")
            .take()
    }
}

#[derive(Debug)]
struct NativeVideoWriteCommand {
    data: Bytes,
    is_keyframe: bool,
    pts_ms: u64,
    encode_submitted_at_ms: u64,
    encoded_at_ms: u64,
    encode_latency_ms: u64,
}

#[derive(Debug)]
enum NativeVideoWriteOutcome {
    Written {
        bytes_len: u64,
        is_keyframe: bool,
        encode_submitted_at_ms: u64,
        encoded_at_ms: u64,
        encode_latency_ms: u64,
        rtp_write_started_ms: u64,
        rtp_write_finished_ms: u64,
    },
    Failed(String),
}

/// Owns the only RTP sample writer for the native video track.
///
/// `TrackLocalStaticSample::write_sample` may legitimately await transport
/// capacity for an unbounded interval. Keeping that await in this worker lets
/// the media control loop continue sampling pressure, adapting bitrate/FPS,
/// handling cancellation, and replacing stale pending frames. The worker is
/// aborted with the session, so a disconnected transport cannot leak a task.
struct NativeVideoWriter {
    pending: Arc<LatestPendingWrite<NativeVideoWriteCommand>>,
    pending_notify: Arc<tokio::sync::Notify>,
    outcomes: Receiver<NativeVideoWriteOutcome>,
    task: tokio::task::JoinHandle<()>,
}

impl NativeVideoWriter {
    fn start(
        track: Arc<TrackLocalStaticSample>,
        ssrc: u32,
        payload_type: u8,
        frame_duration: Duration,
        control_wakeup: Arc<tokio::sync::Notify>,
    ) -> Self {
        let pending: Arc<LatestPendingWrite<NativeVideoWriteCommand>> =
            Arc::new(LatestPendingWrite::default());
        let pending_notify = Arc::new(tokio::sync::Notify::new());
        let (outcome_tx, outcomes) = channel();
        let worker_pending = Arc::clone(&pending);
        let worker_notify = Arc::clone(&pending_notify);
        let task = tokio::spawn(async move {
            let mut last_written_pts_ms = None;
            loop {
                worker_notify.notified().await;
                while let Some(command) = worker_pending.take() {
                    let sample_duration = native_rtp_sample_duration(
                        last_written_pts_ms,
                        command.pts_ms,
                        frame_duration,
                    );
                    let rtp_write_started_ms = now_ms();
                    let result = track
                        .sample_writer(ssrc, payload_type)
                        .write_sample(&Sample {
                            data: command.data.clone(),
                            duration: sample_duration,
                            ..Default::default()
                        })
                        .await;
                    let outcome = match result {
                        Ok(()) => {
                            last_written_pts_ms = Some(command.pts_ms);
                            NativeVideoWriteOutcome::Written {
                                bytes_len: command.data.len() as u64,
                                is_keyframe: command.is_keyframe,
                                encode_submitted_at_ms: command.encode_submitted_at_ms,
                                encoded_at_ms: command.encoded_at_ms,
                                encode_latency_ms: command.encode_latency_ms,
                                rtp_write_started_ms,
                                rtp_write_finished_ms: now_ms(),
                            }
                        }
                        Err(error) => NativeVideoWriteOutcome::Failed(error.to_string()),
                    };
                    if outcome_tx.send(outcome).is_err() {
                        return;
                    }
                    control_wakeup.notify_one();
                }
            }
        });
        Self {
            pending,
            pending_notify,
            outcomes,
            task,
        }
    }

    /// Returns true when an older pending sample was replaced.
    fn enqueue(&self, command: NativeVideoWriteCommand) -> bool {
        let replaced = self.pending.replace(command);
        self.pending_notify.notify_one();
        replaced
    }
}

impl Drop for NativeVideoWriter {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Debug, Clone, Copy)]
struct NativeMediaDropCounters {
    input_dropped_frames: u64,
    adaptation_skipped_frames: u64,
    output_dropped_units: u64,
    rtp_stale_units_dropped: u64,
    rtp_sender_backpressure_drops: u64,
    receiver_dropped_frames: u64,
}

#[derive(Debug, Clone)]
struct NativeAudioStats {
    negotiated: bool,
    packets_written: u64,
    bytes_written: u64,
    capture_chunks_dropped: u64,
    sender_backpressure_drops: u64,
    blocker: Option<String>,
}

struct NativeAudioPipeline {
    track: Arc<TrackLocalStaticSample>,
    ssrc: u32,
    payload_type: u8,
    encoder: Option<RemoteAppOpusEncoder>,
    capture_rx: Receiver<AudioCaptureEvent>,
    capture_chunks_dropped: Arc<AtomicU64>,
    packets_written: u64,
    bytes_written: u64,
    sender_backpressure_drops: u64,
    blocker: Option<String>,
}

impl NativeAudioPipeline {
    async fn new(
        track: &Arc<TrackLocalStaticSample>,
        payload_type: u8,
    ) -> anyhow::Result<(AudioSink, Self)> {
        let ssrc = track
            .ssrcs()
            .await
            .first()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("direct WebRTC audio track has no SSRC"))?;
        let (capture_tx, capture_rx) = sync_channel(NATIVE_AUDIO_CAPTURE_QUEUE_DEPTH);
        let capture_chunks_dropped = Arc::new(AtomicU64::new(0));
        let dropped_for_sink = Arc::clone(&capture_chunks_dropped);
        let sink: AudioSink = Arc::new(move |event| {
            if let Err(error) = capture_tx.try_send(event) {
                if matches!(error, TrySendError::Full(_)) {
                    dropped_for_sink.fetch_add(1, Ordering::Relaxed);
                }
            }
        });
        Ok((
            sink,
            Self {
                track: Arc::clone(track),
                ssrc,
                payload_type,
                encoder: Some(RemoteAppOpusEncoder::new()?),
                capture_rx,
                capture_chunks_dropped,
                packets_written: 0,
                bytes_written: 0,
                sender_backpressure_drops: 0,
                blocker: None,
            },
        ))
    }

    async fn drain(&mut self) {
        while let Ok(event) = self.capture_rx.try_recv() {
            let chunk = match event {
                Ok(chunk) => chunk,
                Err(reason) => {
                    self.blocker = Some(format!("host_audio_capture_failed: {reason}"));
                    self.encoder = None;
                    continue;
                }
            };
            let Some(encoder) = self.encoder.as_mut() else {
                continue;
            };
            let packets = match encoder.push_chunk(chunk) {
                Ok(packets) => packets,
                Err(err) => {
                    self.blocker = Some(format!("host_audio_encode_failed: {err}"));
                    self.encoder = None;
                    continue;
                }
            };
            for packet in packets {
                let bytes = packet.payload.len() as u64;
                let result = self
                    .track
                    .sample_writer(self.ssrc, self.payload_type)
                    .write_sample(&Sample {
                        data: Bytes::from(packet.payload),
                        duration: packet.duration,
                        ..Default::default()
                    })
                    .await;
                match result {
                    Ok(()) => {
                        self.packets_written = self.packets_written.saturating_add(1);
                        self.bytes_written = self.bytes_written.saturating_add(bytes);
                    }
                    Err(err) if is_webrtc_sender_backpressure(&err) => {
                        self.sender_backpressure_drops =
                            self.sender_backpressure_drops.saturating_add(1);
                    }
                    Err(err) => {
                        self.blocker = Some(format!("host_audio_send_failed: {err}"));
                        self.encoder = None;
                        break;
                    }
                }
            }
        }
    }

    fn stats(&self) -> NativeAudioStats {
        NativeAudioStats {
            negotiated: true,
            packets_written: self.packets_written,
            bytes_written: self.bytes_written,
            capture_chunks_dropped: self.capture_chunks_dropped.load(Ordering::Relaxed),
            sender_backpressure_drops: self.sender_backpressure_drops,
            blocker: self.blocker.clone(),
        }
    }
}

fn audio_stats_not_negotiated() -> NativeAudioStats {
    NativeAudioStats {
        negotiated: false,
        packets_written: 0,
        bytes_written: 0,
        capture_chunks_dropped: 0,
        sender_backpressure_drops: 0,
        blocker: Some("host_audio_not_negotiated".to_string()),
    }
}

impl NativeMediaDropCounters {
    fn total_frames_dropped(self) -> u64 {
        self.input_dropped_frames
            .saturating_add(self.output_dropped_units)
            .saturating_add(self.rtp_stale_units_dropped)
            .saturating_add(self.rtp_sender_backpressure_drops)
            .saturating_add(self.receiver_dropped_frames)
    }
}

struct NativeMediaStatsSample<'a> {
    session_id: &'a str,
    transport_epoch: TransportEpoch,
    selected_resource_ura: &'a str,
    media_source_epoch: u64,
    config: &'a BuiltinH264Config,
    width: usize,
    height: usize,
    stream_elapsed: Duration,
    sampled_at_ms: u64,
    current_bitrate_kbps: u32,
    effective_fps: u32,
    min_bitrate_kbps: u32,
    written_units: u64,
    written_keyframes: u64,
    written_bytes: u64,
    drop_counters: NativeMediaDropCounters,
    queued_units: usize,
    in_flight_frames: usize,
    max_in_flight_frames: usize,
    submitted_frames: u64,
    emitted_units: u64,
    configured_bitrate_kbps: u32,
    adaptation_events: Vec<Value>,
    webrtc_stats: Value,
    latency_stats: Value,
    terminal: bool,
    audio: NativeAudioStats,
}

impl NativeMediaStatsSample<'_> {
    fn to_json(self) -> Value {
        let adaptation_event_types: Vec<Value> = self
            .adaptation_events
            .iter()
            .filter_map(|event| event.get("event_type").and_then(Value::as_str))
            .map(|event_type| json!(event_type))
            .collect();
        let frames_dropped = self.drop_counters.total_frames_dropped();
        let mut payload = Map::new();
        payload.insert(
            "contract".to_string(),
            json!(NATIVE_MEDIA_PIPELINE_STATS_CONTRACT),
        );
        payload.insert("path".to_string(), json!("native_webrtc"));
        payload.insert(
            "media_pipeline_id".to_string(),
            json!(self.config.backend.backend_id()),
        );
        payload.insert(
            "selected_resource_ura".to_string(),
            json!(self.selected_resource_ura),
        );
        payload.insert("session_id".to_string(), json!(self.session_id));
        payload.insert(
            "transport_epoch".to_string(),
            json!(self.transport_epoch.value()),
        );
        payload.insert(
            "media_source_epoch".to_string(),
            json!(self.media_source_epoch),
        );
        payload.insert("sampled_at_ms".to_string(), json!(self.sampled_at_ms));
        payload.insert("terminal".to_string(), json!(self.terminal));
        payload.insert(
            "backend_id".to_string(),
            json!(self.config.backend.backend_id()),
        );
        payload.insert(
            "capture_api".to_string(),
            json!(self.config.backend.capture_api()),
        );
        payload.insert(
            "encoder_name".to_string(),
            json!(self.config.backend.encoder()),
        );
        payload.insert("carrier".to_string(), json!(self.config.backend.carrier()));
        payload.insert("video_codec".to_string(), json!(NATIVE_WEBRTC_VIDEO_CODEC));
        payload.insert("codec_negotiated".to_string(), json!(true));
        payload.insert(
            "payload_content_type".to_string(),
            json!(NATIVE_WEBRTC_VIDEO_PAYLOAD_CONTENT_TYPE),
        );
        payload.insert(
            "video_transport".to_string(),
            json!(NATIVE_WEBRTC_VIDEO_TRANSPORT),
        );
        payload.insert(
            "requested_fps".to_string(),
            json!(self.config.requested_fps),
        );
        payload.insert("effective_fps".to_string(), json!(self.effective_fps));
        payload.insert("target_fps".to_string(), json!(self.effective_fps));
        payload.insert(
            "measured_fps".to_string(),
            json!(measured_fps(self.written_units, self.stream_elapsed)),
        );
        payload.insert(
            "target_bitrate_kbps".to_string(),
            json!(self.current_bitrate_kbps),
        );
        payload.insert(
            "configured_target_bitrate_kbps".to_string(),
            json!(self.config.bitrate_kbps),
        );
        payload.insert(
            "observed_bitrate_kbps".to_string(),
            json!(observed_bitrate_kbps(
                self.written_bytes,
                self.stream_elapsed
            )),
        );
        payload.insert(
            "keyframe_interval_frames".to_string(),
            json!(self.config.keyframe_interval_frames),
        );
        payload.insert("width".to_string(), json!(self.width));
        payload.insert("height".to_string(), json!(self.height));
        payload.insert("frames_encoded".to_string(), json!(self.emitted_units));
        payload.insert("rtp_units_written".to_string(), json!(self.written_units));
        payload.insert(
            "rtp_keyframes_written".to_string(),
            json!(self.written_keyframes),
        );
        payload.insert("rtp_bytes_written".to_string(), json!(self.written_bytes));
        payload.insert("frames_dropped".to_string(), json!(frames_dropped));
        payload.insert(
            "input_dropped_frames".to_string(),
            json!(self.drop_counters.input_dropped_frames),
        );
        payload.insert(
            "adaptation_skipped_frames".to_string(),
            json!(self.drop_counters.adaptation_skipped_frames),
        );
        payload.insert(
            "output_dropped_units".to_string(),
            json!(self.drop_counters.output_dropped_units),
        );
        payload.insert(
            "rtp_stale_units_dropped".to_string(),
            json!(self.drop_counters.rtp_stale_units_dropped),
        );
        payload.insert(
            "rtp_sender_backpressure_drops".to_string(),
            json!(self.drop_counters.rtp_sender_backpressure_drops),
        );
        payload.insert(
            "receiver_dropped_frames".to_string(),
            json!(self.drop_counters.receiver_dropped_frames),
        );
        payload.insert(
            "max_frame_queue_depth".to_string(),
            json!(self.config.max_frame_queue_depth),
        );
        payload.insert("queued_units".to_string(), json!(self.queued_units));
        payload.insert("in_flight_frames".to_string(), json!(self.in_flight_frames));
        payload.insert(
            "max_in_flight_frames".to_string(),
            json!(self.max_in_flight_frames),
        );
        payload.insert("drop_stale_frames".to_string(), json!(true));
        payload.insert(
            "drop_policy".to_string(),
            json!("bounded_queue_drop_stale_frames"),
        );
        payload.insert(
            "backpressure_policy".to_string(),
            json!("drop_frame_and_adapt_bitrate"),
        );
        payload.insert(
            "adaptation_algorithm".to_string(),
            json!("native_encoder_feedback"),
        );
        payload.insert(
            "adaptation_events".to_string(),
            json!(self.adaptation_events),
        );
        payload.insert(
            "adaptation_event_types".to_string(),
            json!(adaptation_event_types),
        );
        let audio_ready = self.audio.negotiated && self.audio.blocker.is_none();
        payload.insert(
            "audio_codec".to_string(),
            self.audio
                .negotiated
                .then(|| json!(NATIVE_WEBRTC_AUDIO_CODEC))
                .unwrap_or(Value::Null),
        );
        payload.insert("audio_ready".to_string(), json!(audio_ready));
        payload.insert(
            "audio_media_observed".to_string(),
            json!(self.audio.packets_written > 0),
        );
        payload.insert("host_audio_not_implemented".to_string(), json!(false));
        payload.insert(
            "audio_blocker".to_string(),
            self.audio.blocker.map(Value::String).unwrap_or(Value::Null),
        );
        payload.insert(
            "audio_payload_content_type".to_string(),
            self.audio
                .negotiated
                .then(|| json!(NATIVE_WEBRTC_AUDIO_PAYLOAD_CONTENT_TYPE))
                .unwrap_or(Value::Null),
        );
        payload.insert(
            "audio_sample_rate_hz".to_string(),
            json!(REMOTEAPP_AUDIO_SAMPLE_RATE_HZ),
        );
        payload.insert(
            "audio_channels".to_string(),
            json!(REMOTEAPP_AUDIO_CHANNELS),
        );
        payload.insert(
            "audio_packets_written".to_string(),
            json!(self.audio.packets_written),
        );
        payload.insert(
            "audio_bytes_written".to_string(),
            json!(self.audio.bytes_written),
        );
        payload.insert(
            "audio_capture_chunks_dropped".to_string(),
            json!(self.audio.capture_chunks_dropped),
        );
        payload.insert(
            "audio_sender_backpressure_drops".to_string(),
            json!(self.audio.sender_backpressure_drops),
        );
        payload.insert(
            "adaptive_bitrate".to_string(),
            json!({
                "current_kbps": self.current_bitrate_kbps,
                "target_kbps": self.config.bitrate_kbps,
                "min_kbps": self.min_bitrate_kbps,
            }),
        );
        payload.insert("webrtc_stats".to_string(), self.webrtc_stats);
        payload.insert("latency_stats".to_string(), self.latency_stats);
        payload.insert(
            "encoder_stats".to_string(),
            json!({
                "submitted_frames": self.submitted_frames,
                "emitted_units": self.emitted_units,
                "queued_units": self.queued_units,
                "in_flight_frames": self.in_flight_frames,
                "max_in_flight_frames": self.max_in_flight_frames,
                "configured_bitrate_kbps": self.configured_bitrate_kbps,
                "input_dropped_frames": self.drop_counters.input_dropped_frames,
                "output_dropped_units": self.drop_counters.output_dropped_units,
            }),
        );
        Value::Object(payload)
    }
}

fn measured_fps(written_units: u64, elapsed: Duration) -> f64 {
    let elapsed_ms = elapsed.as_millis();
    if elapsed_ms == 0 || written_units == 0 {
        return 0.0;
    }
    let fps = written_units as f64 * 1000.0 / elapsed_ms as f64;
    (fps * 10.0).round() / 10.0
}

fn observed_bitrate_kbps(written_bytes: u64, elapsed: Duration) -> u64 {
    let elapsed_ms = elapsed.as_millis();
    if elapsed_ms == 0 || written_bytes == 0 {
        return 0;
    }
    ((written_bytes as u128 * 8).saturating_add(elapsed_ms.saturating_sub(1)) / elapsed_ms) as u64
}

/// Immutable inputs for the macOS native direct-WebRTC strategy.
///
/// This type is the native strategy boundary. It is not a session owner, does
/// not choose whether native capture is enabled, and does not update session
/// lifecycle state directly.
pub(in crate::daemon::plugins::remote_desktop) struct NativeMediaInputs<'a> {
    track: &'a Arc<TrackLocalStaticSample>,
    ssrc: u32,
    payload_type: u8,
    audio_track: Option<&'a Arc<TrackLocalStaticSample>>,
    audio_payload_type: Option<u8>,
    options: &'a ScreenCaptureOptions,
    config: &'a BuiltinH264Config,
}

impl<'a> NativeMediaInputs<'a> {
    /// Builds immutable inputs after the parent loop has resolved WebRTC SSRC.
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        track: &'a Arc<TrackLocalStaticSample>,
        ssrc: u32,
        payload_type: u8,
        audio_track: Option<&'a Arc<TrackLocalStaticSample>>,
        audio_payload_type: Option<u8>,
        options: &'a ScreenCaptureOptions,
        config: &'a BuiltinH264Config,
    ) -> Self {
        Self {
            track,
            ssrc,
            payload_type,
            audio_track,
            audio_payload_type,
            options,
            config,
        }
    }
}

pub(in crate::daemon::plugins::remote_desktop) async fn run_direct_webrtc_native_stream(
    execution: &mut DirectWebRtcMediaExecution<'_>,
    peer_connection: &Arc<dyn PeerConnection>,
    inputs: &NativeMediaInputs<'_>,
    target_binding: &RemoteAppTargetBinding,
) -> anyhow::Result<()> {
    use std::sync::Arc as StdArc;

    let NativeMediaInputs {
        track,
        ssrc,
        payload_type,
        audio_track,
        audio_payload_type,
        options,
        config,
    } = *inputs;

    let capture_target = target_for_binding(ABILITY_SET_DESCRIPTION, target_binding)?;
    let (req_width, req_height) =
        native_capture_dimensions(options, || Ok(capture_target.native_dimensions()))?;
    let fps = config.fps.max(1);
    let encoder_wakeup = StdArc::new(tokio::sync::Notify::new());
    let encoder_wakeup_for_callback = StdArc::clone(&encoder_wakeup);
    let encoder_notify = StdArc::new(move || encoder_wakeup_for_callback.notify_one())
        as crate::daemon::plugins::remote_desktop::videotoolbox_encoder::EncoderWakeup;
    let encoder = VideoToolboxEncoder::new_with_wakeup_and_in_flight(
        req_width as i32,
        req_height as i32,
        config.bitrate_kbps,
        config.keyframe_interval_frames,
        fps,
        Some(encoder_notify),
        config.max_frame_queue_depth,
    )?;

    let encoder_for_sink = encoder.session();
    let effective_fps = StdArc::new(AtomicU32::new(fps));
    let frame_admission_phase = StdArc::new(AtomicU64::new(0));
    let adaptation_skipped_frames = StdArc::new(AtomicU64::new(0));
    let effective_fps_for_sink = StdArc::clone(&effective_fps);
    let frame_admission_phase_for_sink = StdArc::clone(&frame_admission_phase);
    let adaptation_skipped_frames_for_sink = StdArc::clone(&adaptation_skipped_frames);
    let sink: crate::daemon::plugins::remote_desktop::screencapturekit_capture::FrameSink =
        StdArc::new(move |frame: CapturedFrame| {
            let admitted_fps = effective_fps_for_sink.load(Ordering::Relaxed).clamp(1, fps);
            let phase = frame_admission_phase_for_sink
                .fetch_add(u64::from(admitted_fps), Ordering::Relaxed)
                % u64::from(fps);
            if phase.saturating_add(u64::from(admitted_fps)) < u64::from(fps) {
                adaptation_skipped_frames_for_sink.fetch_add(1, Ordering::Relaxed);
                return;
            }
            let pts = frame.pts;
            let duration = webrtc_cmtime(1, admitted_fps);
            let _ = encoder_for_sink.encode(&frame.image_buffer, pts, duration);
        });

    let (audio_sink, mut audio_pipeline) = match (audio_track, audio_payload_type) {
        (Some(track), Some(payload_type)) => {
            let (sink, pipeline) = NativeAudioPipeline::new(track, payload_type).await?;
            (Some(sink), Some(pipeline))
        }
        (None, None) => (None, None),
        _ => anyhow::bail!("direct WebRTC audio track/payload negotiation is inconsistent"),
    };

    let mut capture = ScreenCaptureKitStream::start(
        ABILITY_SET_DESCRIPTION,
        capture_target,
        req_width,
        req_height,
        fps,
        ScreenCaptureKitSinks {
            video: sink,
            audio: audio_sink,
        },
    )?;
    let mut active_media_source_epoch = target_binding.media_source_epoch();

    let stream_started_at = Instant::now();
    let frame_dur = Duration::from_secs_f64(1.0 / fps as f64);
    let video_writer = NativeVideoWriter::start(
        Arc::clone(track),
        ssrc,
        payload_type,
        frame_dur,
        Arc::clone(&encoder_wakeup),
    );
    let mut written_units = 0_u64;
    let mut written_keyframes = 0_u64;
    let mut written_bytes = 0_u64;
    let mut rtp_stale_units_dropped = 0_u64;
    let mut rtp_sender_backpressure_drops = 0_u64;
    let mut last_reported_frames_dropped = 0_u64;
    let mut last_reported_backpressure_drops = 0_u64;
    let mut adaptation_event_sequence = 0_u64;
    let mut last_stats_at = Instant::now();
    let mut bitrate_controller = NativeAdaptiveBitrate::new(config.bitrate_kbps);
    let mut receiver_pressure_tracker = NativeReceiverPressureTracker::default();
    let mut latency_stats = NativeLatencyStats::default();
    let mut decoder_primed = false;
    loop {
        if execution.should_stop() {
            break;
        }
        active_media_source_epoch = apply_pending_media_rebind(
            execution.sessions(),
            &mut capture,
            execution.session_id(),
            execution.epoch(),
            active_media_source_epoch,
        )?;
        if let Some(audio) = audio_pipeline.as_mut() {
            audio.drain().await;
        }
        while let Ok(outcome) = video_writer.outcomes.try_recv() {
            match outcome {
                NativeVideoWriteOutcome::Written {
                    bytes_len,
                    is_keyframe,
                    encode_submitted_at_ms,
                    encoded_at_ms,
                    encode_latency_ms,
                    rtp_write_started_ms,
                    rtp_write_finished_ms,
                } => {
                    latency_stats.record_encoded_unit(
                        encode_submitted_at_ms,
                        encoded_at_ms,
                        encode_latency_ms,
                        rtp_write_started_ms,
                        rtp_write_finished_ms,
                    );
                    if is_keyframe {
                        if !decoder_primed {
                            execution.mark_media_ready();
                        }
                        decoder_primed = true;
                        written_keyframes = written_keyframes.saturating_add(1);
                    }
                    written_units = written_units.saturating_add(1);
                    written_bytes = written_bytes.saturating_add(bytes_len);
                }
                NativeVideoWriteOutcome::Failed(error) => {
                    if is_webrtc_sender_backpressure(&error) {
                        rtp_sender_backpressure_drops =
                            rtp_sender_backpressure_drops.saturating_add(1);
                    } else {
                        anyhow::bail!("native WebRTC RTP writer failed: {error}");
                    }
                }
            }
        }
        let (units, stale_dropped) = latest_native_rtp_units(encoder.poll(), decoder_primed);
        rtp_stale_units_dropped = rtp_stale_units_dropped.saturating_add(stale_dropped as u64);
        for unit in units {
            if video_writer.enqueue(NativeVideoWriteCommand {
                data: Bytes::from(unit.annexb),
                is_keyframe: unit.is_keyframe,
                pts_ms: unit.pts_ms,
                encode_submitted_at_ms: unit.encode_submitted_at_ms,
                encoded_at_ms: unit.encoded_at_ms,
                encode_latency_ms: unit.encode_latency_ms,
            }) {
                rtp_sender_backpressure_drops = rtp_sender_backpressure_drops.saturating_add(1);
            }
        }
        if last_stats_at.elapsed() >= Duration::from_secs(1) {
            let stats = encoder.stats();
            let (webrtc_stats, available_outgoing_bitrate_bps) =
                webrtc_stats_snapshot(peer_connection).await;
            let sampled_at_ms = now_ms();
            let client_feedback = execution
                .sessions()
                .client_media_feedback_for_session(execution.session_id(), execution.epoch());
            let receiver_pressure =
                receiver_pressure_tracker.observe(client_feedback, sampled_at_ms);
            let previous_bitrate_kbps = bitrate_controller.current_kbps;
            let applied_bitrate = if let Some(next_bitrate) = bitrate_controller.propose(
                stats.input_dropped_frames,
                stats
                    .output_dropped_units
                    .saturating_add(rtp_stale_units_dropped)
                    .saturating_add(rtp_sender_backpressure_drops),
                stats.queued_units,
                stats.in_flight_frames,
                available_outgoing_bitrate_bps,
                receiver_pressure.pressure_units(),
            ) {
                match encoder.set_bitrate_kbps(next_bitrate) {
                    Ok(()) => {
                        bitrate_controller.commit_applied(next_bitrate);
                        Some(next_bitrate)
                    }
                    Err(err) => {
                        crate::op_event!(
                            component = remote_desktop,
                            kind = native_bitrate_adaptation_failed,
                            requested_bitrate_kbps = next_bitrate,
                            active_bitrate_kbps = bitrate_controller.current_kbps,
                            reason = err.to_string(),
                        );
                        None
                    }
                }
            } else {
                None
            };
            let previous_effective_fps = effective_fps.load(Ordering::Relaxed);
            let next_effective_fps = effective_fps_for_bitrate(
                fps,
                bitrate_controller.current_kbps,
                bitrate_controller.target_kbps,
            );
            effective_fps.store(next_effective_fps, Ordering::Relaxed);
            let skipped_frames = adaptation_skipped_frames.load(Ordering::Relaxed);
            let drop_counters = NativeMediaDropCounters {
                input_dropped_frames: stats.input_dropped_frames.saturating_add(skipped_frames),
                adaptation_skipped_frames: skipped_frames,
                output_dropped_units: stats.output_dropped_units,
                rtp_stale_units_dropped,
                rtp_sender_backpressure_drops,
                receiver_dropped_frames: client_feedback
                    .map(|feedback| feedback.frames_dropped)
                    .unwrap_or(0),
            };
            let total_frames_dropped = drop_counters.total_frames_dropped();
            let frame_drop_delta =
                total_frames_dropped.saturating_sub(last_reported_frames_dropped);
            let sender_backpressure_delta =
                rtp_sender_backpressure_drops.saturating_sub(last_reported_backpressure_drops);
            let receiver_non_drop_pressure = receiver_pressure
                .freeze_delta
                .saturating_add(u64::from(receiver_pressure.elevated_jitter));
            // A bounded latest-frame queue dropping input/output/stale units is
            // itself pipeline backpressure even when the WebRTC writer accepts
            // the newest RTP sample. Writer errors are only one pressure source.
            let backpressure_delta = frame_drop_delta.saturating_add(receiver_non_drop_pressure);
            let mut adaptation_events = Vec::new();
            if let Some(next_bitrate_kbps) = applied_bitrate {
                adaptation_event_sequence = adaptation_event_sequence.saturating_add(1);
                adaptation_events.push(native_media_adaptation_event(
                    adaptation_event_sequence,
                    if next_bitrate_kbps < previous_bitrate_kbps {
                        "bitrate_downshift"
                    } else {
                        "bitrate_upshift"
                    },
                    sampled_at_ms,
                    execution.session_id(),
                    execution.epoch(),
                    target_binding,
                    active_media_source_epoch,
                    config.backend.backend_id(),
                    json!({
                        "algorithm": "native_encoder_feedback",
                        "previous_bitrate_kbps": previous_bitrate_kbps,
                        "next_bitrate_kbps": next_bitrate_kbps,
                        "available_outgoing_bitrate_bps": available_outgoing_bitrate_bps,
                    }),
                ));
            }
            if next_effective_fps != previous_effective_fps {
                adaptation_event_sequence = adaptation_event_sequence.saturating_add(1);
                adaptation_events.push(native_media_adaptation_event(
                    adaptation_event_sequence,
                    if next_effective_fps < previous_effective_fps {
                        "fps_downshift"
                    } else {
                        "fps_upshift"
                    },
                    sampled_at_ms,
                    execution.session_id(),
                    execution.epoch(),
                    target_binding,
                    active_media_source_epoch,
                    config.backend.backend_id(),
                    json!({
                        "algorithm": "native_encoder_feedback",
                        "previous_fps": previous_effective_fps,
                        "next_fps": next_effective_fps,
                        "requested_fps": fps,
                    }),
                ));
            }
            if backpressure_delta > 0 {
                adaptation_event_sequence = adaptation_event_sequence.saturating_add(1);
                adaptation_events.push(native_media_adaptation_event(
                    adaptation_event_sequence,
                    "backpressure_detected",
                    sampled_at_ms,
                    execution.session_id(),
                    execution.epoch(),
                    target_binding,
                    active_media_source_epoch,
                    config.backend.backend_id(),
                    json!({
                        "algorithm": "native_encoder_feedback",
                        "delta": backpressure_delta,
                        "sender_drop_delta": sender_backpressure_delta,
                        "pipeline_frame_drop_delta": frame_drop_delta,
                        "receiver_frame_drop_delta": receiver_pressure.frames_dropped_delta,
                        "receiver_freeze_delta": receiver_pressure.freeze_delta,
                        "receiver_elevated_jitter": receiver_pressure.elevated_jitter,
                        "sender_drop_total": rtp_sender_backpressure_drops,
                    }),
                ));
            }
            if frame_drop_delta > 0 {
                adaptation_event_sequence = adaptation_event_sequence.saturating_add(1);
                adaptation_events.push(native_media_adaptation_event(
                    adaptation_event_sequence,
                    "frame_drop",
                    sampled_at_ms,
                    execution.session_id(),
                    execution.epoch(),
                    target_binding,
                    active_media_source_epoch,
                    config.backend.backend_id(),
                    json!({
                        "algorithm": "native_encoder_feedback",
                        "delta": frame_drop_delta,
                        "total": total_frames_dropped,
                        "receiver_frame_drop_delta": receiver_pressure.frames_dropped_delta,
                    }),
                ));
            }
            last_reported_frames_dropped = total_frames_dropped;
            last_reported_backpressure_drops = rtp_sender_backpressure_drops;
            execution.record_pipeline_stats(
                NativeMediaStatsSample {
                    session_id: execution.session_id(),
                    transport_epoch: execution.epoch(),
                    selected_resource_ura: target_binding.subject_ura(),
                    media_source_epoch: active_media_source_epoch,
                    config,
                    width: req_width,
                    height: req_height,
                    stream_elapsed: stream_started_at.elapsed(),
                    sampled_at_ms,
                    current_bitrate_kbps: bitrate_controller.current_kbps,
                    effective_fps: next_effective_fps,
                    min_bitrate_kbps: bitrate_controller.min_kbps,
                    written_units,
                    written_keyframes,
                    written_bytes,
                    drop_counters,
                    queued_units: stats.queued_units,
                    in_flight_frames: stats.in_flight_frames,
                    max_in_flight_frames: stats.max_in_flight_frames,
                    submitted_frames: stats.submitted_frames,
                    emitted_units: stats.emitted_units,
                    configured_bitrate_kbps: stats.configured_bitrate_kbps,
                    adaptation_events,
                    webrtc_stats,
                    latency_stats: latency_stats.to_json(),
                    terminal: false,
                    audio: audio_pipeline
                        .as_ref()
                        .map(NativeAudioPipeline::stats)
                        .unwrap_or_else(audio_stats_not_negotiated),
                }
                .to_json(),
            );
            last_stats_at = Instant::now();
        }
        tokio::select! {
            _ = encoder_wakeup.notified() => {}
            _ = tokio::time::sleep(Duration::from_millis(2)) => {}
        }
    }
    capture.stop();
    let stats = encoder.stats();
    let (webrtc_stats, _) = webrtc_stats_snapshot(peer_connection).await;
    execution.record_pipeline_stats(
        NativeMediaStatsSample {
            session_id: execution.session_id(),
            transport_epoch: execution.epoch(),
            selected_resource_ura: target_binding.subject_ura(),
            media_source_epoch: active_media_source_epoch,
            config,
            width: req_width,
            height: req_height,
            stream_elapsed: stream_started_at.elapsed(),
            sampled_at_ms: now_ms(),
            current_bitrate_kbps: bitrate_controller.current_kbps,
            effective_fps: effective_fps.load(Ordering::Relaxed),
            min_bitrate_kbps: bitrate_controller.min_kbps,
            written_units,
            written_keyframes,
            written_bytes,
            drop_counters: NativeMediaDropCounters {
                input_dropped_frames: stats
                    .input_dropped_frames
                    .saturating_add(adaptation_skipped_frames.load(Ordering::Relaxed)),
                adaptation_skipped_frames: adaptation_skipped_frames.load(Ordering::Relaxed),
                output_dropped_units: stats.output_dropped_units,
                rtp_stale_units_dropped,
                rtp_sender_backpressure_drops,
                receiver_dropped_frames: execution
                    .sessions()
                    .client_media_feedback_for_session(execution.session_id(), execution.epoch())
                    .map(|feedback| feedback.frames_dropped)
                    .unwrap_or(0),
            },
            queued_units: stats.queued_units,
            in_flight_frames: stats.in_flight_frames,
            max_in_flight_frames: stats.max_in_flight_frames,
            submitted_frames: stats.submitted_frames,
            emitted_units: stats.emitted_units,
            configured_bitrate_kbps: stats.configured_bitrate_kbps,
            adaptation_events: Vec::new(),
            webrtc_stats,
            latency_stats: latency_stats.to_json(),
            terminal: true,
            audio: audio_pipeline
                .as_ref()
                .map(NativeAudioPipeline::stats)
                .unwrap_or_else(audio_stats_not_negotiated),
        }
        .to_json(),
    );
    Ok(())
}

fn effective_fps_for_bitrate(requested_fps: u32, current_kbps: u32, target_kbps: u32) -> u32 {
    let requested_fps = requested_fps.max(1);
    let minimum_fps = requested_fps.min(15);
    let target_kbps = target_kbps.max(1);
    requested_fps
        .saturating_mul(current_kbps)
        .saturating_div(target_kbps)
        .clamp(minimum_fps, requested_fps)
}

fn native_media_adaptation_event(
    sequence: u64,
    event_type: &'static str,
    observed_at_ms: u64,
    session_id: &str,
    transport_epoch: TransportEpoch,
    target_binding: &RemoteAppTargetBinding,
    media_source_epoch: u64,
    media_pipeline_id: &str,
    detail: Value,
) -> Value {
    json!({
        "event_type": event_type,
        "sequence": sequence,
        "observed_at_ms": observed_at_ms,
        "selected_resource_ura": target_binding.subject_ura(),
        "session_id": session_id,
        "transport_epoch": transport_epoch.value(),
        "media_source_epoch": media_source_epoch,
        "media_pipeline_id": media_pipeline_id,
        "video_codec": NATIVE_WEBRTC_VIDEO_CODEC,
        "video_transport": NATIVE_WEBRTC_VIDEO_TRANSPORT,
        "audio_codec": Value::Null,
        "detail": detail,
    })
}

fn apply_pending_media_rebind(
    sessions: &RemoteDesktopSessionStore,
    capture: &mut ScreenCaptureKitStream,
    session_id: &str,
    epoch: TransportEpoch,
    active_media_source_epoch: u64,
) -> Result<u64, RemoteAppTargetError> {
    let Some(next_binding) = sessions.pending_media_rebind_binding_for_session(
        session_id,
        epoch,
        active_media_source_epoch,
    ) else {
        return Ok(active_media_source_epoch);
    };
    let next_target =
        match target_for_pending_application_rebind(ABILITY_SET_DESCRIPTION, &next_binding) {
            Ok(target) => target,
            Err(err) => {
                supersede_pending_media_rebind(sessions, session_id, epoch, &err);
                return Ok(active_media_source_epoch);
            }
        };
    let prepared = match capture.prepare_content_filter_update(ABILITY_SET_DESCRIPTION, next_target)
    {
        Ok(prepared) => prepared,
        Err(err) => {
            supersede_pending_media_rebind(sessions, session_id, epoch, &err);
            return Ok(active_media_source_epoch);
        }
    };
    if capture.commit_prepared_content_filter_update(prepared, |capture_proof| {
        sessions.commit_pending_media_rebind_for_session(
            session_id,
            epoch,
            next_binding.binding_epoch(),
            next_binding.media_source_epoch(),
            capture_proof.clone(),
        )
    }) {
        Ok(next_binding.media_source_epoch())
    } else {
        Ok(active_media_source_epoch)
    }
}

fn supersede_pending_media_rebind(
    sessions: &RemoteDesktopSessionStore,
    session_id: &str,
    epoch: TransportEpoch,
    err: &RemoteAppTargetError,
) {
    sessions.supersede_pending_media_rebind_for_session(
        session_id,
        epoch,
        err.reason(),
        err.to_string(),
    );
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    use crate::daemon::plugins::remote_desktop::constants::{
        direct_webrtc_endpoint_ura, TRANSPORT_WEBRTC,
    };
    use crate::daemon::plugins::remote_desktop::media::MACOS_SCK_VIDEOTOOLBOX_BACKEND;
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
    use crate::daemon::plugins::remote_desktop::target::{
        AppWindowSetProof, TargetGeometry, TargetResolutionError,
    };
    use crate::daemon::plugins::remote_desktop::target_tracking::TargetObservation;
    use crate::daemon::plugins::remote_desktop::test_support::test_application_session_init;

    #[test]
    fn latest_pending_write_replaces_stale_value_without_growing_a_queue() {
        let pending = LatestPendingWrite::default();
        assert!(!pending.replace(1_u64));
        assert!(pending.replace(2_u64));
        assert!(pending.replace(3_u64));
        assert_eq!(pending.take(), Some(3));
        assert_eq!(pending.take(), None);
    }

    #[test]
    fn native_media_rebind_candidate_failure_preserves_active_generation() {
        let store = RemoteDesktopSessionStore::new();
        let session_id = "rd-native-media-rebind-filter-failed";
        let epoch = TransportEpoch::new(31);
        let mut session = RemoteDesktopSession::new(test_application_session_init(
            session_id,
            vec![TRANSPORT_WEBRTC.to_string()],
        ));
        session.begin_webrtc_negotiation(epoch);
        session
            .set_local_webrtc_answer(
                epoch,
                json!({"type": "answer", "sdp": "v=0"}),
                "sck-native",
                true,
                "easynet:///r/acme/ability/remote-desktop.transport".into(),
            )
            .expect("local answer records");
        session.mark_webrtc_media_sending(epoch, direct_webrtc_endpoint_ura(session_id));
        assert!(
            session
                .record_target_observation(TargetObservation::ApplicationSurfaceChanged {
                    app_window_set: AppWindowSetProof::new(
                        42,
                        Some("com.example.Editor".to_string()),
                        Some(9001),
                        vec![10, 11, 12],
                    ),
                    app_surface_layout: None,
                    geometry: TargetGeometry {
                        x: Some(10.0),
                        y: Some(20.0),
                        width: Some(320.0),
                        height: Some(120.0),
                    },
                    target_identity_epoch: 100,
                    target_geometry_revision: 4,
                    observed_at_ms: 10,
                })
                .is_none(),
            "application window-set drift rebind must not be reported as media loss"
        );
        let pending = session
            .pending_media_rebind_binding()
            .expect("pending media rebind")
            .clone();
        store.with_sessions(|sessions| {
            sessions.insert(session_id.to_string(), session);
        });

        let err = RemoteAppTargetError::new(
            ABILITY_SET_DESCRIPTION,
            TargetResolutionError::ScreenCaptureKitFilterFailed,
            "native content filter rejected pending application window set",
        );
        supersede_pending_media_rebind(&store, session_id, epoch, &err);

        store.with_sessions(|sessions| {
            let session = sessions.get(session_id).expect("session stored");
            assert_eq!(session.target_tracking_state()["status"], json!("resolved"));
            assert_eq!(
                session.target_binding().media_source_epoch(),
                pending.media_source_epoch() - 1,
                "the committed media generation remains active"
            );
            assert!(session.pending_media_rebind_binding().is_none());
            let event = session
                .events()
                .into_iter()
                .find(|event| event["event_type"] == json!("TARGET_REBIND_SUPERSEDED"))
                .expect("target rebind superseded event");
            assert_eq!(
                event["reason_code"],
                json!("target_rebind_candidate_superseded")
            );
            assert_eq!(
                event["payload"]["candidate_rejection_reason"],
                json!("screencapturekit_filter_failed")
            );
            assert_eq!(event["payload"]["recoverability"], json!("continue"));
            assert!(event["payload"]["frontend_action"].is_null());
            assert_eq!(
                event["payload"]["pending_media_source_epoch"],
                json!(pending.media_source_epoch())
            );
            assert!(session
                .events()
                .iter()
                .all(|event| event["event_type"] != json!("MEDIA_SOURCE_LOST")));
        });
    }

    #[test]
    fn native_media_pipeline_stats_project_product_evidence_contract() {
        let init = test_application_session_init(
            "rd-native-media-stats",
            vec![TRANSPORT_WEBRTC.to_string()],
        );
        let config = BuiltinH264Config {
            backend: MACOS_SCK_VIDEOTOOLBOX_BACKEND,
            requested_fps: 144,
            fps: 60,
            bitrate_kbps: 6_000,
            max_frame_queue_depth: 3,
            keyframe_interval_frames: 30,
        };
        let adaptation_event = native_media_adaptation_event(
            7,
            "bitrate_downshift",
            1_700,
            "rd-native-media-stats",
            TransportEpoch::new(4),
            &init.target_binding,
            init.target_binding.media_source_epoch(),
            config.backend.backend_id(),
            json!({
                "algorithm": "native_encoder_feedback",
                "previous_bitrate_kbps": 6000,
                "next_bitrate_kbps": 4500,
            }),
        );

        let stats = NativeMediaStatsSample {
            session_id: "rd-native-media-stats",
            transport_epoch: TransportEpoch::new(4),
            selected_resource_ura: init.target_binding.subject_ura(),
            media_source_epoch: init.target_binding.media_source_epoch(),
            config: &config,
            width: 1512,
            height: 982,
            stream_elapsed: Duration::from_secs(2),
            sampled_at_ms: 1_700,
            current_bitrate_kbps: 4_500,
            effective_fps: 60,
            min_bitrate_kbps: 500,
            written_units: 120,
            written_keyframes: 4,
            written_bytes: 1_250_000,
            drop_counters: NativeMediaDropCounters {
                input_dropped_frames: 1,
                adaptation_skipped_frames: 0,
                output_dropped_units: 2,
                rtp_stale_units_dropped: 3,
                rtp_sender_backpressure_drops: 4,
                receiver_dropped_frames: 5,
            },
            queued_units: 1,
            in_flight_frames: 1,
            max_in_flight_frames: 2,
            submitted_frames: 126,
            emitted_units: 122,
            configured_bitrate_kbps: 4_500,
            adaptation_events: vec![adaptation_event],
            webrtc_stats: json!({
                "selected_candidate_pair": {
                    "selected_route_class": "direct"
                }
            }),
            latency_stats: json!({
                "encode_submit_to_rtp_write": {
                    "max_ms": 18
                }
            }),
            terminal: false,
            audio: NativeAudioStats {
                negotiated: true,
                packets_written: 96,
                bytes_written: 24_000,
                capture_chunks_dropped: 2,
                sender_backpressure_drops: 1,
                blocker: None,
            },
        }
        .to_json();

        assert_eq!(
            stats["contract"],
            json!(NATIVE_MEDIA_PIPELINE_STATS_CONTRACT)
        );
        assert_eq!(
            stats["media_pipeline_id"],
            json!(config.backend.backend_id())
        );
        assert_eq!(
            stats["selected_resource_ura"],
            json!(init.target_binding.subject_ura())
        );
        assert_eq!(
            stats["media_source_epoch"],
            json!(init.target_binding.media_source_epoch())
        );
        assert_eq!(stats["session_id"], json!("rd-native-media-stats"));
        assert_eq!(stats["transport_epoch"], json!(4));
        assert_eq!(stats["video_codec"], json!("h264"));
        assert_eq!(
            stats["payload_content_type"],
            json!("video/h264; stream-format=annexb")
        );
        assert_eq!(stats["video_transport"], json!("webrtc"));
        assert_eq!(stats["requested_fps"], json!(144));
        assert_eq!(stats["effective_fps"], json!(60));
        assert_eq!(stats["adaptation_skipped_frames"], json!(0));
        assert_eq!(stats["target_fps"], json!(60));
        assert_eq!(stats["measured_fps"], json!(60.0));
        assert_eq!(stats["target_bitrate_kbps"], json!(4_500));
        assert_eq!(stats["observed_bitrate_kbps"], json!(5_000));
        assert_eq!(stats["frames_dropped"], json!(15));
        assert_eq!(
            stats["drop_policy"],
            json!("bounded_queue_drop_stale_frames")
        );
        assert_eq!(
            stats["adaptation_event_types"],
            json!(["bitrate_downshift"])
        );
        assert_eq!(
            stats["adaptation_events"][0]["selected_resource_ura"],
            json!(init.target_binding.subject_ura())
        );
        assert_eq!(
            stats["adaptation_events"][0]["media_pipeline_id"],
            json!(config.backend.backend_id())
        );
        assert_eq!(stats["host_audio_not_implemented"], json!(false));
        assert_eq!(stats["audio_ready"], json!(true));
        assert_eq!(stats["audio_media_observed"], json!(true));
        assert_eq!(stats["audio_codec"], json!("opus"));
        assert_eq!(stats["audio_packets_written"], json!(96));
        assert_eq!(stats["audio_capture_chunks_dropped"], json!(2));
        assert_eq!(stats["terminal"], json!(false));
    }
}
