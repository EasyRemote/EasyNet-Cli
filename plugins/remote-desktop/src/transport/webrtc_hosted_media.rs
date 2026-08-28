// EasyNet CLI — supervised RemoteApp media-host WebRTC bridge
// ============================================================
//
// Platform capture and encoding run in the plugin-private media host. This
// daemon module owns the generation fence, lifecycle barriers, adaptation
// decisions, WebRTC writes and session-state projection.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use easynet_remoteapp_native_protocol::media_session::{
    AudioCodec, AudioConfig, CaptureProof, Command, CommandBody, EventBody, FailureReason,
    GenerationFence, MediaObservation, MediaStats, NativeTargetPlan, StartContract, VideoCodec,
    VideoConfig, MAX_PAYLOAD_BYTES, PROTOCOL, SCHEMA_VERSION,
};
use rand::RngCore;
use serde_json::{json, Value};
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::Track;

/// rtc/webrtc's sample track packetizer uses a 1200-byte outbound MTU and a
/// 12-byte base RTP header. Keeping NALs at 1160 bytes leaves room for header
/// extensions while retaining the original mapped `Bytes` slice in the common
/// single-NAL packetization path.
const H264_PACKETIZATION_NAL_BYTES: u32 = 1_160;

use crate::daemon::plugins::remote_desktop::constants::ABILITY_SET_DESCRIPTION;
use crate::daemon::plugins::remote_desktop::media::adaptation::{
    effective_fps_for_bitrate, effective_fps_for_writer_service, AdaptiveBitrateController,
    ReceiverPressureTracker,
};
use crate::daemon::plugins::remote_desktop::media::encode::BuiltinH264Config;
use crate::daemon::plugins::remote_desktop::media::{
    MEDIA_PIPELINE_STATS_CONTRACT, WEBRTC_VIDEO_TRANSPORT,
};
use crate::daemon::plugins::remote_desktop::media_host_probe::{
    project_capture_proof, target_plan,
};
use crate::daemon::plugins::remote_desktop::native_host_process::{
    media_host_build_id, MediaHostMediaEvent, MediaHostProcess,
};
use crate::daemon::plugins::remote_desktop::session::now_ms;
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, TargetResolutionError,
};
use crate::daemon::plugins::remote_desktop::transport::webrtc_encoded_audio::{
    EncodedAudioPacket, EncodedAudioWriter, EncodedAudioWriterSnapshot, ENCODED_AUDIO_QUEUE_DEPTH,
};
use crate::daemon::plugins::remote_desktop::transport::webrtc_media::{
    DirectWebRtcMediaExecution, HostedMediaInputs,
};
use crate::daemon::plugins::remote_desktop::transport::webrtc_sender_feedback::{
    RtcpReceiverPressure, RtcpReceiverPressureTracker,
};
use crate::daemon::plugins::remote_desktop::MEDIA_HOST_EXECUTABLE;

const CONTROL_TRANSITION_DEADLINE: Duration = Duration::from_secs(10);
const MEDIA_POLL_INTERVAL: Duration = Duration::from_millis(2);
const VIDEO_RECOVERY_DEADLINE: Duration = Duration::from_secs(2);
const HOSTED_LATENCY_WINDOW_SAMPLES: usize = 256;
const H264_ANNEX_B_CONTENT_TYPE: &str = "video/h264; stream-format=annexb";
const OPUS_CONTENT_TYPE: &str = "audio/opus";
static NEXT_MEDIA_PROCESS_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Typed helper failure retained across the private process/WebRTC boundary.
///
/// The outer transport loop uses this classification to distinguish a stale or
/// permission-blocked capture target from a healthy target whose transport or
/// codec generation failed.
#[derive(Debug)]
pub(super) struct HostedMediaHostFailure {
    reason: FailureReason,
    detail: String,
}

impl HostedMediaHostFailure {
    pub(super) fn new(reason: FailureReason, detail: String) -> Self {
        Self { reason, detail }
    }

    pub(super) const fn target_reason(&self) -> Option<TargetResolutionError> {
        match self.reason {
            FailureReason::PermissionDenied | FailureReason::PermissionRevoked => {
                Some(TargetResolutionError::TargetPermissionMissing)
            }
            FailureReason::TargetInvalidated => Some(TargetResolutionError::TargetStale),
            FailureReason::DeviceLost => Some(TargetResolutionError::TargetDisplayUnavailable),
            FailureReason::CaptureUnavailable
            | FailureReason::EncoderUnavailable
            | FailureReason::AudioUnavailable
            | FailureReason::ProtocolViolation
            | FailureReason::Internal => None,
        }
    }
}

impl std::fmt::Display for HostedMediaHostFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "RemoteApp media-host failed ({:?}): {}",
            self.reason, self.detail
        )
    }
}

impl std::error::Error for HostedMediaHostFailure {}

struct HostedGeneration {
    process: MediaHostProcess,
    fence: GenerationFence,
    command_sequence: u64,
    target: NativeTargetPlan,
    video: VideoConfig,
    audio: Option<AudioConfig>,
}

impl HostedGeneration {
    fn prepare(
        transport_epoch: u64,
        binding: &RemoteAppTargetBinding,
        video: VideoConfig,
        audio: Option<AudioConfig>,
    ) -> anyhow::Result<(Self, CaptureProof)> {
        let target = target_plan(binding)?;
        let contract = StartContract {
            target: target.clone(),
            video: video.clone(),
            audio: audio.clone(),
        };
        let build_id = media_host_build_id(MEDIA_HOST_EXECUTABLE)?;
        let process_generation = NEXT_MEDIA_PROCESS_GENERATION.fetch_add(1, Ordering::Relaxed);
        anyhow::ensure!(
            process_generation > 0 && process_generation < u64::MAX,
            "RemoteApp media-host process generation exhausted"
        );
        let mut nonce = [0_u8; 16];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let fence = GenerationFence {
            process_generation,
            build_id,
            session_nonce: lower_hex(&nonce),
            transport_epoch,
            media_source_epoch: binding.media_source_epoch(),
            contract_digest: contract.digest()?.to_string(),
        };
        let mut process = MediaHostProcess::spawn(
            process_generation,
            MEDIA_HOST_EXECUTABLE,
            fence.clone(),
            &contract,
            &[],
        )?;
        let start = Command {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.to_string(),
            fence: fence.clone(),
            sequence: 1,
            body: CommandBody::StartPrepared { contract },
        };
        process.send_command(&start)?;
        let mut generation = Self {
            process,
            fence,
            command_sequence: 1,
            target,
            video,
            audio,
        };
        let prepared = generation.await_control("prepared", |body| match body {
            EventBody::Prepared { capture_proof, .. } => Some(capture_proof.clone()),
            _ => None,
        })?;
        Ok((generation, prepared))
    }

    fn activate(&mut self) -> anyhow::Result<()> {
        let activate_sequence = self.send(CommandBody::Activate)?;
        self.await_control("activated", |body| match body {
            EventBody::Activated { command_sequence } if *command_sequence == activate_sequence => {
                Some(())
            }
            _ => None,
        })?;
        self.send(CommandBody::BeginMedia {
            activation_command_sequence: activate_sequence,
        })?;
        Ok(())
    }

    fn reconfigure(&mut self, video: VideoConfig) -> anyhow::Result<()> {
        let reconfigure_sequence = self.send(CommandBody::Reconfigure {
            video: video.clone(),
            force_keyframe: true,
        })?;
        self.await_control("reconfigured", |body| match body {
            EventBody::Reconfigured {
                command_sequence,
                video: acknowledged,
                ..
            } if *command_sequence == reconfigure_sequence && *acknowledged == video => Some(()),
            _ => None,
        })?;
        self.send(CommandBody::ResumeMedia {
            reconfigure_command_sequence: reconfigure_sequence,
        })?;
        self.video = video;
        Ok(())
    }

    fn request_keyframe(&mut self) -> anyhow::Result<()> {
        let request_sequence = self.send(CommandBody::RequestKeyframe)?;
        self.await_control("keyframe request", |body| match body {
            EventBody::KeyframeRequested { command_sequence }
                if *command_sequence == request_sequence =>
            {
                Some(())
            }
            _ => None,
        })
    }

    fn stop(&mut self) -> anyhow::Result<()> {
        if self.process.protocol_violated() {
            anyhow::bail!("RemoteApp media-host protocol was violated before stop");
        }
        let stop_sequence = self.send(CommandBody::Stop)?;
        self.await_control("stopped", |body| match body {
            EventBody::Stopped { command_sequence } if *command_sequence == stop_sequence => {
                Some(())
            }
            _ => None,
        })?;
        self.process.close_commands();
        Ok(())
    }

    fn send(&mut self, body: CommandBody) -> anyhow::Result<u64> {
        self.command_sequence = self
            .command_sequence
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("RemoteApp media-host command sequence overflow"))?;
        let command = Command {
            schema_version: SCHEMA_VERSION,
            protocol: PROTOCOL.to_string(),
            fence: self.fence.clone(),
            sequence: self.command_sequence,
            body,
        };
        self.process.send_command(&command)?;
        Ok(self.command_sequence)
    }

    fn await_control<T>(
        &mut self,
        transition: &str,
        mut select: impl FnMut(&EventBody) -> Option<T>,
    ) -> anyhow::Result<T> {
        let deadline = Instant::now() + CONTROL_TRANSITION_DEADLINE;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            anyhow::ensure!(
                !remaining.is_zero(),
                "RemoteApp media-host {transition} deadline exceeded"
            );
            let event = self.process.recv_control_timeout(remaining)?;
            if let EventBody::Failed { reason, detail } = &event.metadata.body {
                return Err(anyhow::Error::new(HostedMediaHostFailure::new(
                    *reason,
                    detail.clone(),
                )));
            }
            if let Some(value) = select(&event.metadata.body) {
                return Ok(value);
            }
            if !matches!(event.metadata.body, EventBody::Stats { .. }) {
                anyhow::bail!(
                    "RemoteApp media-host emitted an unexpected control event during {transition}"
                );
            }
        }
    }
}

/// Session-owned bridge from already encoded media-host Opus packets to the
/// negotiated WebRTC audio sender. The independent bounded writer prevents a
/// slow audio sender from stalling capture, video, control, or adaptation.
struct HostedAudioTransport {
    track: std::sync::Arc<TrackLocalStaticSample>,
    ssrc: u32,
    payload_type: u8,
    writer: Option<EncodedAudioWriter>,
    stale_packets_dropped: u64,
}

impl HostedAudioTransport {
    async fn new(
        track: &std::sync::Arc<TrackLocalStaticSample>,
        payload_type: u8,
    ) -> anyhow::Result<Self> {
        let ssrc = track
            .ssrcs()
            .await
            .first()
            .copied()
            .ok_or_else(|| anyhow::anyhow!("direct WebRTC audio track has no SSRC"))?;
        Ok(Self {
            track: std::sync::Arc::clone(track),
            ssrc,
            payload_type,
            writer: Some(EncodedAudioWriter::spawn(
                std::sync::Arc::clone(track),
                ssrc,
                payload_type,
            )),
            stale_packets_dropped: 0,
        })
    }

    fn enqueue(&mut self, payload: Bytes, duration_samples: u16) -> anyhow::Result<()> {
        let writer = self
            .writer
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("hosted audio writer is not active"))?;
        if writer.enqueue(EncodedAudioPacket {
            payload,
            duration: Duration::from_secs_f64(f64::from(duration_samples) / 48_000.0),
        }) {
            self.stale_packets_dropped = self.stale_packets_dropped.saturating_add(1);
        }
        self.ensure_healthy()
    }

    fn ensure_healthy(&self) -> anyhow::Result<()> {
        if let Some(error) = self.snapshot().fatal_error {
            anyhow::bail!("hosted audio WebRTC sender failed: {error}");
        }
        Ok(())
    }

    fn snapshot(&self) -> EncodedAudioWriterSnapshot {
        self.writer
            .as_ref()
            .map(EncodedAudioWriter::snapshot)
            .unwrap_or(EncodedAudioWriterSnapshot {
                packets_written: 0,
                bytes_written: 0,
                sender_backpressure_errors: 0,
                fatal_error: None,
            })
    }

    fn queued_packets(&self) -> usize {
        self.writer
            .as_ref()
            .map(EncodedAudioWriter::queued_packets)
            .unwrap_or(0)
    }

    async fn quiesce_for_rebind(&mut self) {
        if let Some(writer) = self.writer.take() {
            writer.shutdown_discard().await;
        }
        self.stale_packets_dropped = 0;
    }

    fn activate_after_rebind(&mut self) {
        assert!(
            self.writer.is_none(),
            "hosted audio writer must be quiesced before source activation"
        );
        self.writer = Some(EncodedAudioWriter::spawn(
            std::sync::Arc::clone(&self.track),
            self.ssrc,
            self.payload_type,
        ));
    }

    async fn shutdown_discard(&mut self) {
        if let Some(writer) = self.writer.take() {
            writer.shutdown_discard().await;
        }
    }
}

#[derive(Debug, Default)]
struct HostedLatencyWindow {
    encode_to_write_samples_ms: VecDeque<u64>,
    writer_service_samples_ms: VecDeque<f64>,
}

impl HostedLatencyWindow {
    fn record_encode_submit_to_rtp_write(&mut self, encode_submitted_at_ms: u64) {
        if encode_submitted_at_ms == 0 {
            return;
        }
        if self.encode_to_write_samples_ms.len() == HOSTED_LATENCY_WINDOW_SAMPLES {
            self.encode_to_write_samples_ms.pop_front();
        }
        // The source timestamp uses millisecond resolution. A completed write
        // in the same clock tick is therefore a sub-millisecond observation,
        // not a zero-latency sample. Preserve that distinction so product
        // telemetry cannot confuse coarse clock resolution with missing work.
        self.encode_to_write_samples_ms
            .push_back(now_ms().saturating_sub(encode_submitted_at_ms).max(1));
    }

    fn record_writer_service(&mut self, elapsed: Duration) {
        if self.writer_service_samples_ms.len() == HOSTED_LATENCY_WINDOW_SAMPLES {
            self.writer_service_samples_ms.pop_front();
        }
        self.writer_service_samples_ms
            .push_back(elapsed.as_secs_f64() * 1_000.0);
    }

    fn writer_service_p95(&self) -> (usize, f64) {
        let mut sorted = self
            .writer_service_samples_ms
            .iter()
            .copied()
            .collect::<Vec<_>>();
        sorted.sort_by(f64::total_cmp);
        let p95_ms = sorted
            .get(((sorted.len().saturating_sub(1)) * 95) / 100)
            .copied()
            .unwrap_or(0.0);
        (sorted.len(), p95_ms)
    }

    fn projection(&self) -> Value {
        let mut sorted = self
            .encode_to_write_samples_ms
            .iter()
            .copied()
            .collect::<Vec<_>>();
        sorted.sort_unstable();
        let p95_ms = if sorted.is_empty() {
            0
        } else {
            sorted[((sorted.len() - 1) * 95) / 100]
        };
        let (writer_samples, writer_p95_ms) = self.writer_service_p95();
        json!({
            "encode_submit_to_rtp_write": {
                "samples": sorted.len(),
                "p95_ms": p95_ms,
            },
            "rtp_writer_service": {
                "samples": writer_samples,
                "p95_ms": writer_p95_ms,
            }
        })
    }

    fn reset(&mut self) {
        self.encode_to_write_samples_ms.clear();
        self.writer_service_samples_ms.clear();
    }
}

/// Product-facing telemetry projection for the hosted media architecture.
///
/// The media-host owns capture/encode counters; this daemon-side controller
/// binds them to the admitted session/Resource/WebRTC generation and adds the
/// transport-write facts that the helper cannot observe.
struct HostedMediaTelemetry {
    session_id: String,
    transport_epoch: u64,
    selected_resource_ura: String,
    media_source_epoch: u64,
    media_pipeline_id: &'static str,
    generation_started_at: Instant,
    adaptation_event_sequence: u64,
    latency: HostedLatencyWindow,
}

impl HostedMediaTelemetry {
    fn new(
        execution: &DirectWebRtcMediaExecution<'_>,
        binding: &RemoteAppTargetBinding,
        config: &BuiltinH264Config,
    ) -> Self {
        Self {
            session_id: execution.session_id().to_string(),
            transport_epoch: execution.epoch().value(),
            selected_resource_ura: binding.subject_ura().to_string(),
            media_source_epoch: binding.media_source_epoch(),
            media_pipeline_id: config.backend.backend_id(),
            generation_started_at: Instant::now(),
            adaptation_event_sequence: 0,
            latency: HostedLatencyWindow::default(),
        }
    }

    fn begin_generation(&mut self, binding: &RemoteAppTargetBinding) {
        self.selected_resource_ura = binding.subject_ura().to_string();
        self.media_source_epoch = binding.media_source_epoch();
        self.generation_started_at = Instant::now();
        self.latency.reset();
    }

    fn record_video_write(&mut self, encode_submitted_at_ms: u64, writer_service: Duration) {
        self.latency
            .record_encode_submit_to_rtp_write(encode_submitted_at_ms);
        self.latency.record_writer_service(writer_service);
    }

    fn event(&mut self, event_type: &'static str, detail: Value) -> Value {
        self.adaptation_event_sequence = self.adaptation_event_sequence.saturating_add(1);
        json!({
            "event_type": event_type,
            "sequence": self.adaptation_event_sequence,
            "observed_at_ms": now_ms(),
            "session_id": self.session_id,
            "transport_epoch": self.transport_epoch,
            "selected_resource_ura": self.selected_resource_ura,
            "media_source_epoch": self.media_source_epoch,
            "media_pipeline_id": self.media_pipeline_id,
            "video_codec": "h264",
            "video_transport": WEBRTC_VIDEO_TRANSPORT,
            "detail": detail,
        })
    }

    fn adaptation_events(
        &mut self,
        observation: &HostedAdaptationObservation,
        applied: Option<&HostedReconfigurationProposal>,
    ) -> Vec<Value> {
        let mut events = Vec::new();
        let receiver_pressure = observation.receiver_frames_dropped_delta > 0
            || observation.receiver_freeze_delta > 0
            || observation.receiver_elevated_jitter
            || observation.rtcp_receiver_pressure.pressure_units() > 0;
        if receiver_pressure {
            events.push(self.event(
                "backpressure_detected",
                json!({
                    "receiver_frame_drop_delta": observation.receiver_frames_dropped_delta,
                    "receiver_freeze_delta": observation.receiver_freeze_delta,
                    "receiver_elevated_jitter": observation.receiver_elevated_jitter,
                    "rtcp_report_fresh": observation.rtcp_receiver_pressure.fresh,
                    "rtcp_packets_lost_delta": observation.rtcp_receiver_pressure.packets_lost_delta,
                    "rtcp_fraction_lost": observation.rtcp_receiver_pressure.fraction_lost,
                    "rtcp_round_trip_time_ms": observation.rtcp_receiver_pressure.round_trip_time_ms,
                    "rtcp_stats_read_failed": observation.rtcp_receiver_pressure.stats_read_failed,
                }),
            ));
        }
        if observation.input_drop_delta > 0 || observation.output_drop_delta > 0 {
            events.push(self.event(
                "frame_drop",
                json!({
                    "input_frame_drop_delta": observation.input_drop_delta,
                    "output_frame_drop_delta": observation.output_drop_delta,
                    "source": "bounded_host_and_daemon_media_queues",
                }),
            ));
        }
        if let Some(proposal) = applied {
            if proposal.video.bitrate_kbps != proposal.previous_bitrate_kbps {
                events.push(self.event(
                    if proposal.video.bitrate_kbps < proposal.previous_bitrate_kbps {
                        "bitrate_downshift"
                    } else {
                        "bitrate_upshift"
                    },
                    json!({
                        "algorithm": "receiver_feedback_media_host_reconfigure",
                        "previous_bitrate_kbps": proposal.previous_bitrate_kbps,
                        "next_bitrate_kbps": proposal.video.bitrate_kbps,
                        "receiver_frame_drop_delta": observation.receiver_frames_dropped_delta,
                        "receiver_freeze_delta": observation.receiver_freeze_delta,
                        "receiver_elevated_jitter": observation.receiver_elevated_jitter,
                        "rtcp_packets_lost_delta": observation.rtcp_receiver_pressure.packets_lost_delta,
                        "rtcp_fraction_lost": observation.rtcp_receiver_pressure.fraction_lost,
                        "rtcp_round_trip_time_ms": observation.rtcp_receiver_pressure.round_trip_time_ms,
                    }),
                ));
            }
            if proposal.video.fps != proposal.previous_fps {
                events.push(self.event(
                    if proposal.video.fps < proposal.previous_fps {
                        "fps_downshift"
                    } else {
                        "fps_upshift"
                    },
                    json!({
                        "previous_fps": proposal.previous_fps,
                        "next_fps": proposal.video.fps,
                        "cause": if proposal.writer_service_limited {
                            "rtp_writer_service_time"
                        } else {
                            "applied_bitrate_change"
                        },
                    }),
                ));
            }
        }
        events
    }

    #[allow(clippy::too_many_arguments)]
    fn projection(
        &self,
        config: &BuiltinH264Config,
        video: &VideoConfig,
        stats: &MediaStats,
        process_generation: u64,
        daemon_video_frames_dropped: u64,
        daemon_video_queue_depth: usize,
        audio_negotiated: bool,
        audio_transport: Option<&HostedAudioTransport>,
        adaptation: &HostedAdaptation,
        adaptation_events: Vec<Value>,
    ) -> Value {
        let elapsed = self.generation_started_at.elapsed();
        let elapsed_secs = elapsed.as_secs_f64();
        let measured_fps = if elapsed_secs > 0.0 {
            stats.encoded_video_frames as f64 / elapsed_secs
        } else {
            0.0
        };
        let observed_bitrate_kbps = if elapsed_secs > 0.0 {
            (stats.video_bytes as f64 * 8.0 / 1000.0) / elapsed_secs
        } else {
            0.0
        };
        let frames_dropped = stats
            .raw_video_frames_dropped
            .saturating_add(stats.encoded_video_frames_dropped)
            .saturating_add(daemon_video_frames_dropped);
        let audio_writer = audio_transport.map(HostedAudioTransport::snapshot);
        let audio_sender_ready = audio_transport.is_some()
            && audio_writer
                .as_ref()
                .is_some_and(|writer| writer.fatal_error.is_none());
        let audio_packets_written = audio_writer
            .as_ref()
            .map_or(0, |value| value.packets_written);
        let audio_bytes_written = audio_writer.as_ref().map_or(0, |value| value.bytes_written);
        let audio_sender_errors = audio_writer
            .as_ref()
            .map_or(0, |value| value.sender_backpressure_errors);
        let audio_stale_drops = audio_transport.map_or(0, |value| value.stale_packets_dropped);
        let audio_queue_depth = audio_transport.map_or(0, HostedAudioTransport::queued_packets);
        let audio_operational_ready = audio_negotiated && audio_sender_ready;
        let audio_blocker = if !audio_negotiated {
            Some("audio_not_negotiated")
        } else if !audio_sender_ready {
            Some("audio_sender_unavailable")
        } else {
            None
        };
        let queued_units = (stats.video_queue_depth as usize).max(daemon_video_queue_depth);
        let mut projection = json!({
            "contract": MEDIA_PIPELINE_STATS_CONTRACT,
            "path": "hosted_media_process_webrtc",
            "pipeline": "remoteapp_media_host_v1",
            "sampled_at_ms": now_ms(),
            "session_id": self.session_id,
            "transport_epoch": self.transport_epoch,
            "selected_resource_ura": self.selected_resource_ura,
            "media_source_epoch": self.media_source_epoch,
            "media_pipeline_id": self.media_pipeline_id,
            "capture_api": config.backend.capture_api(),
            "backend_id": config.backend.backend_id(),
            "encoder_name": config.backend.encoder(),
            "carrier": config.backend.carrier(),
            "process_generation": process_generation,
            "video_codec": "h264",
            "codec_negotiated": true,
            "payload_content_type": H264_ANNEX_B_CONTENT_TYPE,
            "video_transport": WEBRTC_VIDEO_TRANSPORT,
            "target_bitrate_kbps": video.bitrate_kbps,
            "configured_target_bitrate_kbps": adaptation.bitrate.target_kbps,
            "min_bitrate_kbps": adaptation.bitrate.min_kbps,
            "requested_fps": config.requested_fps,
            "configured_fps": config.fps,
            "effective_fps": video.fps,
            "target_fps": video.fps,
        });
        let Value::Object(projection_object) = &mut projection else {
            unreachable!("hosted media projection must be an object");
        };
        let Value::Object(video_runtime) = json!({
            "measured_fps": measured_fps,
            "observed_bitrate_kbps": observed_bitrate_kbps,
            "keyframe_interval_frames": video.keyframe_interval_frames,
            "width": video.width,
            "height": video.height,
            "capture_frames": stats.capture_frames,
            "received_frames": stats.capture_frames,
            "encoded_video_frames": stats.encoded_video_frames,
            "frames_encoded": stats.encoded_video_frames,
            "video_bytes": stats.video_bytes,
            "encoded_bytes": stats.video_bytes,
            "raw_video_frames_dropped": stats.raw_video_frames_dropped,
            "encoded_video_frames_dropped": stats.encoded_video_frames_dropped,
            "daemon_video_frames_dropped": daemon_video_frames_dropped,
            "frames_dropped": frames_dropped,
            "video_queue_depth": stats.video_queue_depth,
            "daemon_video_queue_depth": daemon_video_queue_depth,
            "max_frame_queue_depth": video.max_pending_frames,
            "queued_units": queued_units,
            "in_flight_frames": 0,
            "drop_stale_frames": true,
            "drop_policy": "latest_frame_bounded_gop",
            "backpressure_policy": "latest_frame_gop_recovery_and_receiver_feedback",
            "adaptation_algorithm": "transport_feedback",
            "adaptation_events": adaptation_events,
            "latency_stats": self.latency.projection(),
            "stream_elapsed_ms": elapsed.as_millis().min(u64::MAX as u128) as u64,
            "terminal": false,
        }) else {
            unreachable!("hosted video runtime projection must be an object");
        };
        projection_object.extend(video_runtime);
        let Value::Object(audio_runtime) = json!({
            "audio_negotiated": audio_negotiated,
            "audio_codec": audio_negotiated.then_some("opus"),
            "audio_payload_content_type": audio_negotiated.then_some(OPUS_CONTENT_TYPE),
            "audio_sample_rate_hz": audio_negotiated.then_some(48_000).unwrap_or(0),
            "audio_channels": audio_negotiated.then_some(2).unwrap_or(0),
            "audio_ready": audio_operational_ready,
            "audio_operational_ready": audio_operational_ready,
            "audio_capture_started": audio_negotiated && stats.encoded_audio_packets > 0,
            "audio_sender_ready": audio_sender_ready,
            "audio_media_observed": audio_packets_written > 0,
            "audio_backend_available": audio_negotiated,
            "host_audio_not_implemented": false,
            "audio_blocker": audio_blocker,
            "encoded_audio_packets": stats.encoded_audio_packets,
            "audio_packets_dropped": stats.audio_packets_dropped,
            "media_host_audio_queue_depth": stats.audio_queue_depth,
            "audio_bytes": stats.audio_bytes,
            "audio_packets_written": audio_packets_written,
            "audio_bytes_written": audio_bytes_written,
            "audio_queue_depth": audio_queue_depth,
            "audio_max_queue_depth": ENCODED_AUDIO_QUEUE_DEPTH,
            "audio_transport_write_isolated": audio_negotiated,
            "audio_drop_stale_packets": audio_negotiated,
            "audio_drop_policy": audio_negotiated.then_some("bounded_queue_drop_oldest_audio_packet"),
            "audio_stale_packets_dropped": audio_stale_drops,
            "audio_sender_backpressure_errors": audio_sender_errors,
            "audio_sender_backpressure_drops": audio_stale_drops.saturating_add(audio_sender_errors),
        }) else {
            unreachable!("hosted audio runtime projection must be an object");
        };
        projection_object.extend(audio_runtime);
        projection
    }
}

struct HostedAdaptation {
    bitrate: AdaptiveBitrateController,
    receiver: ReceiverPressureTracker,
    requested_fps: u32,
    last_raw_dropped: u64,
    last_encoded_dropped: u64,
}

#[derive(Debug, Clone)]
struct HostedReconfigurationProposal {
    video: VideoConfig,
    previous_bitrate_kbps: u32,
    previous_fps: u32,
    writer_service_limited: bool,
}

#[derive(Debug, Clone)]
struct HostedAdaptationObservation {
    reconfiguration: Option<HostedReconfigurationProposal>,
    input_drop_delta: u64,
    output_drop_delta: u64,
    receiver_frames_dropped_delta: u64,
    receiver_freeze_delta: u64,
    receiver_elevated_jitter: bool,
    rtcp_receiver_pressure: RtcpReceiverPressure,
}

impl HostedAdaptation {
    fn new(config: &BuiltinH264Config) -> Self {
        Self {
            bitrate: AdaptiveBitrateController::new(config.bitrate_kbps),
            receiver: ReceiverPressureTracker::default(),
            requested_fps: config.fps,
            last_raw_dropped: 0,
            last_encoded_dropped: 0,
        }
    }

    fn observe(
        &mut self,
        execution: &DirectWebRtcMediaExecution<'_>,
        stats: &MediaStats,
        daemon_video_frames_dropped: u64,
        daemon_video_queue_depth: usize,
        current: &VideoConfig,
        rtcp_receiver_pressure: RtcpReceiverPressure,
        writer_service_samples: usize,
        writer_service_p95_ms: f64,
    ) -> HostedAdaptationObservation {
        let feedback = execution
            .sessions()
            .client_media_feedback_for_session(execution.session_id(), execution.epoch());
        let pressure = self.receiver.observe(feedback, Instant::now());
        let current_raw_dropped = stats.raw_video_frames_dropped;
        let current_encoded_dropped = stats
            .encoded_video_frames_dropped
            .saturating_add(daemon_video_frames_dropped);
        let input_drop_delta = current_raw_dropped.saturating_sub(self.last_raw_dropped);
        let output_drop_delta = current_encoded_dropped.saturating_sub(self.last_encoded_dropped);
        self.last_raw_dropped = self.last_raw_dropped.max(current_raw_dropped);
        self.last_encoded_dropped = self.last_encoded_dropped.max(current_encoded_dropped);
        let proposed_bitrate = self.bitrate.propose(
            self.last_raw_dropped,
            self.last_encoded_dropped,
            (stats.video_queue_depth as usize).max(daemon_video_queue_depth),
            0,
            pressure
                .pressure_units()
                .saturating_add(rtcp_receiver_pressure.pressure_units()),
        );
        let next_bitrate = proposed_bitrate.unwrap_or(current.bitrate_kbps);
        let bitrate_fps =
            effective_fps_for_bitrate(self.requested_fps, next_bitrate, self.bitrate.target_kbps);
        let writer_fps = effective_fps_for_writer_service(
            self.requested_fps,
            writer_service_p95_ms,
            writer_service_samples,
        );
        let next_fps = bitrate_fps.min(writer_fps);
        let reconfiguration = (next_bitrate != current.bitrate_kbps || next_fps != current.fps)
            .then(|| {
                let mut next = current.clone();
                next.bitrate_kbps = next_bitrate;
                next.fps = next_fps;
                next.keyframe_interval_frames = next.fps.clamp(1, 30);
                HostedReconfigurationProposal {
                    video: next,
                    previous_bitrate_kbps: current.bitrate_kbps,
                    previous_fps: current.fps,
                    writer_service_limited: writer_fps < bitrate_fps,
                }
            });
        HostedAdaptationObservation {
            reconfiguration,
            input_drop_delta,
            output_drop_delta,
            receiver_frames_dropped_delta: pressure.frames_dropped_delta,
            receiver_freeze_delta: pressure.freeze_delta,
            receiver_elevated_jitter: pressure.elevated_jitter,
            rtcp_receiver_pressure,
        }
    }

    fn commit(&mut self, video: &VideoConfig) {
        self.bitrate.commit_applied(video.bitrate_kbps);
    }
}

pub(in crate::daemon::plugins::remote_desktop) async fn run_direct_webrtc_hosted_stream(
    execution: &mut DirectWebRtcMediaExecution<'_>,
    inputs: &HostedMediaInputs<'_>,
) -> anyhow::Result<()> {
    let negotiated_audio = audio_config(inputs)?;
    let mut audio_transport = match (inputs.audio_track, inputs.audio_payload_type) {
        (Some(track), Some(payload_type)) => {
            Some(HostedAudioTransport::new(track, payload_type).await?)
        }
        (None, None) => None,
        _ => unreachable!("audio_config rejects a partially negotiated audio sender"),
    };
    let initial_video = video_config(inputs.target_binding, inputs.options, inputs.config)?;
    let (mut generation, capture_proof) = HostedGeneration::prepare(
        execution.epoch().value(),
        inputs.target_binding,
        initial_video,
        negotiated_audio.clone(),
    )?;
    let initial_proof =
        match project_capture_proof(inputs.target_binding, &generation.target, capture_proof) {
            Ok(proof) => proof,
            Err(error) => {
                if let Some(audio) = audio_transport.as_mut() {
                    audio.shutdown_discard().await;
                }
                let _ = generation.stop();
                return Err(error);
            }
        };
    if let Err(error) = inputs
        .target_binding
        .validate_reverified_capture_proof(ABILITY_SET_DESCRIPTION, &initial_proof)
    {
        let _ = generation.stop();
        return Err(error.into());
    }
    generation.activate()?;
    let mut active_binding = inputs.target_binding.clone();
    let mut adaptation = HostedAdaptation::new(inputs.config);
    let mut rtcp_receiver = RtcpReceiverPressureTracker::default();
    let mut telemetry = HostedMediaTelemetry::new(execution, &active_binding, inputs.config);
    let mut media_ready = false;
    let mut latest_stats = None;

    loop {
        if execution.should_stop() {
            if let Some(audio) = audio_transport.as_mut() {
                audio.shutdown_discard().await;
            }
            generation.stop()?;
            return Ok(());
        }
        if let Some(pending) = execution
            .sessions()
            .pending_media_rebind_binding_for_session(
                execution.session_id(),
                execution.epoch(),
                generation.fence.media_source_epoch,
            )
        {
            if let Some(audio) = audio_transport.as_mut() {
                audio.quiesce_for_rebind().await;
            }
            generation.stop()?;
            let retained_video = generation.video.clone();
            let retained_audio = generation.audio.clone();
            match HostedGeneration::prepare(
                execution.epoch().value(),
                &pending.binding,
                retained_video.clone(),
                retained_audio.clone(),
            ) {
                Ok((mut next, capture_proof)) => {
                    let proof = match project_capture_proof(
                        &pending.binding,
                        &next.target,
                        capture_proof,
                    ) {
                        Ok(proof) => proof,
                        Err(error) => {
                            next.stop()?;
                            execution
                                .sessions()
                                .supersede_pending_media_rebind_for_session(
                                    execution.session_id(),
                                    execution.epoch(),
                                    &pending.attempt_token,
                                    TargetResolutionError::TargetIdentityChanged,
                                    format!("project replacement media-host proof: {error}"),
                                );
                            generation = restart_generation(
                                execution,
                                &active_binding,
                                retained_video,
                                retained_audio,
                            )?;
                            reset_generation_observers(
                                &mut adaptation,
                                &mut telemetry,
                                inputs.config,
                                &active_binding,
                                &generation.video,
                            );
                            latest_stats = None;
                            if let Some(audio) = audio_transport.as_mut() {
                                audio.activate_after_rebind();
                            }
                            continue;
                        }
                    };
                    if let Err(error) = pending.binding.validate_pending_media_rebind_capture_proof(
                        ABILITY_SET_DESCRIPTION,
                        &proof,
                    ) {
                        next.stop()?;
                        execution
                            .sessions()
                            .supersede_pending_media_rebind_for_session(
                                execution.session_id(),
                                execution.epoch(),
                                &pending.attempt_token,
                                error.reason(),
                                error.to_string(),
                            );
                        generation = restart_generation(
                            execution,
                            &active_binding,
                            retained_video,
                            retained_audio,
                        )?;
                        reset_generation_observers(
                            &mut adaptation,
                            &mut telemetry,
                            inputs.config,
                            &active_binding,
                            &generation.video,
                        );
                        latest_stats = None;
                        if let Some(audio) = audio_transport.as_mut() {
                            audio.activate_after_rebind();
                        }
                        continue;
                    }
                    if !execution
                        .sessions()
                        .commit_pending_media_rebind_for_session(
                            execution.session_id(),
                            execution.epoch(),
                            pending.binding.binding_epoch(),
                            pending.binding.media_source_epoch(),
                            &pending.attempt_token,
                            proof,
                        )
                    {
                        next.stop()?;
                        generation = restart_generation(
                            execution,
                            &active_binding,
                            retained_video,
                            retained_audio,
                        )?;
                        reset_generation_observers(
                            &mut adaptation,
                            &mut telemetry,
                            inputs.config,
                            &active_binding,
                            &generation.video,
                        );
                        latest_stats = None;
                        if let Some(audio) = audio_transport.as_mut() {
                            audio.activate_after_rebind();
                        }
                        continue;
                    }
                    next.activate()?;
                    if let Some(audio) = audio_transport.as_mut() {
                        audio.activate_after_rebind();
                    }
                    active_binding = pending.binding;
                    generation = next;
                    reset_generation_observers(
                        &mut adaptation,
                        &mut telemetry,
                        inputs.config,
                        &active_binding,
                        &generation.video,
                    );
                    latest_stats = None;
                    continue;
                }
                Err(error) => {
                    execution
                        .sessions()
                        .supersede_pending_media_rebind_for_session(
                            execution.session_id(),
                            execution.epoch(),
                            &pending.attempt_token,
                            TargetResolutionError::CaptureBackendUnavailable,
                            format!("prepare replacement media-host generation: {error}"),
                        );
                    generation = restart_generation(
                        execution,
                        &active_binding,
                        retained_video,
                        retained_audio,
                    )?;
                    reset_generation_observers(
                        &mut adaptation,
                        &mut telemetry,
                        inputs.config,
                        &active_binding,
                        &generation.video,
                    );
                    latest_stats = None;
                    if let Some(audio) = audio_transport.as_mut() {
                        audio.activate_after_rebind();
                    }
                    continue;
                }
            }
        }
        anyhow::ensure!(
            !generation.process.protocol_violated(),
            "RemoteApp media-host violated a bounded lane or framing contract"
        );
        anyhow::ensure!(
            !generation
                .process
                .video_recovery_overdue(VIDEO_RECOVERY_DEADLINE),
            "RemoteApp media-host did not produce a recovery IDR within {}ms after backpressure",
            VIDEO_RECOVERY_DEADLINE.as_millis()
        );

        while let Some(event) = generation.process.try_recv_control()? {
            match event.metadata.body {
                EventBody::Stats { stats } => latest_stats = Some(stats),
                EventBody::Failed { reason, detail } => {
                    anyhow::bail!("RemoteApp media-host failed ({reason:?}): {detail}")
                }
                _ => anyhow::bail!("RemoteApp media-host emitted unsolicited control state"),
            }
        }
        while let Some(event) = generation.process.try_recv_audio()? {
            if event.observation != MediaObservation::Accepted {
                continue;
            }
            let EventBody::AudioOpus {
                duration_samples,
                sample_rate_hz,
                channels,
                ..
            } = event.metadata.body
            else {
                anyhow::bail!("RemoteApp media-host audio lane carried non-audio metadata");
            };
            anyhow::ensure!(
                sample_rate_hz == 48_000 && channels == 2 && duration_samples == 960,
                "RemoteApp media-host violated the negotiated Opus framing contract"
            );
            let audio = audio_transport.as_mut().ok_or_else(|| {
                anyhow::anyhow!("RemoteApp media-host emitted audio without a negotiated sender")
            })?;
            audio.enqueue(event.payload, duration_samples)?;
        }
        if let Some(audio) = audio_transport.as_ref() {
            audio.ensure_healthy()?;
        }

        let mut selected_video: Option<MediaHostMediaEvent> = None;
        while let Some(event) = generation.process.try_recv_video()? {
            if event.observation == MediaObservation::Accepted {
                select_video_for_rtp(&mut selected_video, event);
            }
        }
        if let Some(event) = selected_video {
            let EventBody::VideoH264 {
                duration_90khz,
                encode_submitted_at_ms,
                ..
            } = event.metadata.body
            else {
                anyhow::bail!("RemoteApp media-host video lane carried non-video metadata");
            };
            let writer_started_at = Instant::now();
            inputs
                .track
                .sample_writer(inputs.ssrc, inputs.payload_type)
                .write_sample(&rtc::media::Sample {
                    data: event.payload,
                    duration: Duration::from_secs_f64(f64::from(duration_90khz.max(1)) / 90_000.0),
                    ..Default::default()
                })
                .await?;
            telemetry.record_video_write(encode_submitted_at_ms, writer_started_at.elapsed());
            if !media_ready {
                execution.mark_media_ready();
                media_ready = true;
            }
        }

        if generation.process.take_video_recovery_request() {
            eprintln!(
                "[remoteapp-media-recovery] kind=recovery_command_sending process_generation={}",
                generation.process.id()
            );
            generation.request_keyframe()?;
            eprintln!(
                "[remoteapp-media-recovery] kind=recovery_command_acknowledged process_generation={}",
                generation.process.id()
            );
        }

        if let Some(stats) = latest_stats.take() {
            let rtcp_receiver_pressure = rtcp_receiver
                .observe(inputs.video_sender, Instant::now())
                .await;
            let (writer_service_samples, writer_service_p95_ms) =
                telemetry.latency.writer_service_p95();
            let daemon_video_frames_dropped = generation.process.video_frames_dropped();
            let daemon_video_queue_depth = generation.process.video_queue_depth();
            let observation = adaptation.observe(
                execution,
                &stats,
                daemon_video_frames_dropped,
                daemon_video_queue_depth,
                &generation.video,
                rtcp_receiver_pressure,
                writer_service_samples,
                writer_service_p95_ms,
            );
            let applied = if let Some(proposal) = observation.reconfiguration.as_ref() {
                generation.reconfigure(proposal.video.clone())?;
                adaptation.commit(&proposal.video);
                Some(proposal)
            } else {
                None
            };
            let adaptation_events = telemetry.adaptation_events(&observation, applied);
            execution.record_pipeline_stats(telemetry.projection(
                inputs.config,
                &generation.video,
                &stats,
                generation.process.id(),
                daemon_video_frames_dropped,
                daemon_video_queue_depth,
                negotiated_audio.is_some(),
                audio_transport.as_ref(),
                &adaptation,
                adaptation_events,
            ));
        }
        tokio::time::sleep(MEDIA_POLL_INTERVAL).await;
    }
}

fn select_video_for_rtp(
    selected: &mut Option<MediaHostMediaEvent>,
    candidate: MediaHostMediaEvent,
) {
    let candidate_is_recovery = is_video_recovery_event(&candidate);
    let selected_is_recovery = selected.as_ref().is_some_and(is_video_recovery_event);
    if candidate_is_recovery || !selected_is_recovery {
        *selected = Some(candidate);
    }
}

fn is_video_recovery_event(event: &MediaHostMediaEvent) -> bool {
    matches!(
        event.metadata.body,
        EventBody::VideoH264 {
            keyframe: true,
            sps_pps_present: true,
            ..
        }
    )
}

fn reset_generation_observers(
    adaptation: &mut HostedAdaptation,
    telemetry: &mut HostedMediaTelemetry,
    config: &BuiltinH264Config,
    binding: &RemoteAppTargetBinding,
    video: &VideoConfig,
) {
    *adaptation = HostedAdaptation::new(config);
    adaptation.commit(video);
    telemetry.begin_generation(binding);
}

fn restart_generation(
    execution: &DirectWebRtcMediaExecution<'_>,
    binding: &RemoteAppTargetBinding,
    video: VideoConfig,
    audio: Option<AudioConfig>,
) -> anyhow::Result<HostedGeneration> {
    let (mut generation, _) =
        HostedGeneration::prepare(execution.epoch().value(), binding, video, audio)?;
    generation.activate()?;
    Ok(generation)
}

fn video_config(
    binding: &RemoteAppTargetBinding,
    options: &crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenCaptureOptions,
    config: &BuiltinH264Config,
) -> anyhow::Result<VideoConfig> {
    let _ = options.resolution.ok_or_else(|| {
        anyhow::anyhow!("RemoteApp media-host requires an explicit negotiated resolution")
    })?;
    let (native_width, native_height) = binding
        .require_capture_proof(ABILITY_SET_DESCRIPTION)?
        .native_dimensions()
        .ok_or_else(|| {
            anyhow::anyhow!("RemoteApp media-host requires committed native target dimensions")
        })?;
    let native_width = u32::try_from(native_width)
        .map_err(|_| anyhow::anyhow!("RemoteApp native target width exceeds u32"))?;
    let native_height = u32::try_from(native_height)
        .map_err(|_| anyhow::anyhow!("RemoteApp native target height exceeds u32"))?;
    let (output_width, output_height) = options.output_dimensions(native_width, native_height);
    let width = output_width & !1;
    let height = output_height & !1;
    anyhow::ensure!(
        width > 0 && height > 0,
        "RemoteApp negotiated coded dimensions must remain positive after even alignment"
    );
    Ok(VideoConfig {
        codec: VideoCodec::H264AnnexB,
        width,
        height,
        fps: config.fps,
        bitrate_kbps: config.bitrate_kbps,
        keyframe_interval_frames: config.keyframe_interval_frames,
        max_pending_frames: u32::try_from(config.max_frame_queue_depth.min(3)).unwrap_or(3),
        max_access_unit_bytes: u32::try_from(MAX_PAYLOAD_BYTES).unwrap_or(u32::MAX),
        max_nal_unit_bytes: H264_PACKETIZATION_NAL_BYTES,
        h264_profile_idc: 66,
        h264_level_idc: config.h264_level.level_idc(),
    })
}

fn audio_config(inputs: &HostedMediaInputs<'_>) -> anyhow::Result<Option<AudioConfig>> {
    match (inputs.audio_track, inputs.audio_payload_type) {
        (Some(_), Some(_)) => Ok(Some(AudioConfig {
            codec: AudioCodec::Opus,
            sample_rate_hz: 48_000,
            channels: 2,
            frame_duration_ms: 20,
            max_pending_packets: u32::try_from(ENCODED_AUDIO_QUEUE_DEPTH).unwrap_or(4),
        })),
        (None, None) => Ok(None),
        _ => anyhow::bail!(
            "RemoteApp audio negotiation must provide both the WebRTC track and payload type"
        ),
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
        CaptureResizeMode, ScreenCaptureOptions, VideoResolution,
    };
    use crate::daemon::plugins::remote_desktop::media::h264_level::H264Level;
    use crate::daemon::plugins::remote_desktop::media::XCAP_OPENH264_WEBRTC_BACKEND;
    use crate::daemon::plugins::remote_desktop::test_support::test_application_target_binding;

    fn accepted_video_event(sequence: u64, recovery: bool) -> MediaHostMediaEvent {
        let payload: &'static [u8] = if recovery {
            b"recovery-idr"
        } else {
            b"dependent-delta"
        };
        MediaHostMediaEvent {
            metadata: easynet_remoteapp_native_protocol::media_session::BinaryMediaEvent {
                sequence,
                observed_at_ms: sequence,
                body: EventBody::VideoH264 {
                    media_gate: 1,
                    pts_90khz: sequence * 3_000,
                    duration_90khz: 3_000,
                    keyframe: recovery,
                    sps_pps_present: recovery,
                    discontinuity: recovery,
                    codec_generation: 1,
                    width: 640,
                    height: 360,
                    encode_submitted_at_ms: sequence,
                    encoded_at_ms: sequence,
                },
            },
            payload: Bytes::from_static(payload),
            observation: MediaObservation::Accepted,
        }
    }

    #[test]
    fn recovery_idr_wins_over_newer_dependent_frames_in_one_drain_batch() {
        let mut selected = None;
        select_video_for_rtp(&mut selected, accepted_video_event(1, false));
        select_video_for_rtp(&mut selected, accepted_video_event(2, true));
        select_video_for_rtp(&mut selected, accepted_video_event(3, false));

        let selected = selected.expect("one RTP candidate must be selected");
        assert_eq!(selected.metadata.sequence, 2);
        assert!(is_video_recovery_event(&selected));
    }

    #[test]
    fn newest_recovery_idr_supersedes_an_older_recovery_in_one_drain_batch() {
        let mut selected = None;
        select_video_for_rtp(&mut selected, accepted_video_event(1, true));
        select_video_for_rtp(&mut selected, accepted_video_event(2, false));
        select_video_for_rtp(&mut selected, accepted_video_event(3, true));

        assert_eq!(selected.unwrap().metadata.sequence, 3);
    }

    #[test]
    fn video_contract_uses_exact_negotiated_limits() {
        let options = ScreenCaptureOptions {
            fps: 60,
            resolution: Some(VideoResolution {
                width: 1_280,
                height: 720,
            }),
            resize_mode: CaptureResizeMode::Exact,
            region: None,
        };
        let config = BuiltinH264Config {
            backend: XCAP_OPENH264_WEBRTC_BACKEND,
            requested_fps: 60,
            fps: 30,
            bitrate_kbps: 4_800,
            h264_level: H264Level::Level3_1,
            max_frame_queue_depth: 9,
            keyframe_interval_frames: 30,
        };
        let binding = test_application_target_binding();
        let video = video_config(&binding, &options, &config).expect("valid hosted video contract");
        assert_eq!((video.width, video.height), (1_280, 720));
        assert_eq!(video.fps, 30);
        assert_eq!(video.bitrate_kbps, 4_800);
        assert_eq!(video.max_pending_frames, 3);
        assert_eq!(video.max_nal_unit_bytes, H264_PACKETIZATION_NAL_BYTES);
        assert_eq!(video.h264_profile_idc, 66);
        assert_eq!(video.h264_level_idc, 31);
    }

    #[test]
    fn native_scale_mode_does_not_upscale_committed_target_dimensions() {
        let options = ScreenCaptureOptions {
            fps: 60,
            resolution: Some(VideoResolution {
                width: 1_280,
                height: 720,
            }),
            resize_mode: CaptureResizeMode::FitWithin,
            region: None,
        };
        let config = BuiltinH264Config {
            backend: XCAP_OPENH264_WEBRTC_BACKEND,
            requested_fps: 60,
            fps: 30,
            bitrate_kbps: 4_800,
            h264_level: H264Level::Level3_1,
            max_frame_queue_depth: 3,
            keyframe_interval_frames: 30,
        };

        let binding = test_application_target_binding();
        let video = video_config(&binding, &options, &config)
            .expect("native presentation must derive from committed capture proof");

        assert_eq!((video.width, video.height), (200, 100));
    }

    #[test]
    fn negotiated_nal_bound_keeps_rtp_payloader_on_borrowed_slice_path() {
        use rtc::rtp::codec::h264::H264Payloader;
        use rtc::rtp::packetizer::Payloader;

        const WEBRTC_H264_PAYLOAD_MTU: usize = 1_188;
        let mut access_unit = vec![0_u8; 4 + H264_PACKETIZATION_NAL_BYTES as usize];
        access_unit[..5].copy_from_slice(&[0, 0, 0, 1, 0x41]);
        let access_unit = Bytes::from(access_unit);
        let expected_nal_pointer = access_unit[4..].as_ptr();
        let payloads = H264Payloader::default()
            .payload(WEBRTC_H264_PAYLOAD_MTU, &access_unit)
            .expect("bounded Annex-B NAL packetizes");

        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].len(), H264_PACKETIZATION_NAL_BYTES as usize);
        assert_eq!(payloads[0].as_ptr(), expected_nal_pointer);
    }

    #[test]
    fn application_contract_preserves_membership_and_front_to_back_order() {
        let binding = test_application_target_binding();
        let plan = target_plan(&binding).expect("application binding forms a host contract");
        let application = plan.application.expect("application proof is carried");
        assert_eq!(application.window_ids, vec![10, 11]);
        assert_eq!(application.display_ids, vec![42]);
        assert_eq!(
            application
                .front_to_back_surfaces
                .iter()
                .map(|surface| surface.window_id)
                .collect::<Vec<_>>(),
            vec![10, 11]
        );
    }

    #[test]
    fn helper_failures_preserve_target_vs_transport_recovery_domain() {
        assert_eq!(
            HostedMediaHostFailure::new(
                FailureReason::PermissionRevoked,
                "permission changed".into(),
            )
            .target_reason(),
            Some(TargetResolutionError::TargetPermissionMissing)
        );
        assert_eq!(
            HostedMediaHostFailure::new(
                FailureReason::TargetInvalidated,
                "window set changed".into(),
            )
            .target_reason(),
            Some(TargetResolutionError::TargetStale)
        );
        assert_eq!(
            HostedMediaHostFailure::new(
                FailureReason::EncoderUnavailable,
                "encoder failed".into(),
            )
            .target_reason(),
            None
        );
    }

    #[test]
    fn hosted_stats_preserve_the_product_media_contract_after_process_split() {
        let binding = test_application_target_binding();
        let config = BuiltinH264Config {
            backend: XCAP_OPENH264_WEBRTC_BACKEND,
            requested_fps: 60,
            fps: 30,
            bitrate_kbps: 4_800,
            h264_level: H264Level::Level3_1,
            max_frame_queue_depth: 3,
            keyframe_interval_frames: 30,
        };
        let video = VideoConfig {
            codec: VideoCodec::H264AnnexB,
            width: 1_280,
            height: 720,
            fps: 24,
            bitrate_kbps: 3_800,
            keyframe_interval_frames: 24,
            max_pending_frames: 3,
            max_access_unit_bytes: 1_048_576,
            max_nal_unit_bytes: H264_PACKETIZATION_NAL_BYTES,
            h264_profile_idc: 66,
            h264_level_idc: 31,
        };
        let stats = MediaStats {
            capture_frames: 120,
            encoded_video_frames: 100,
            encoded_audio_packets: 0,
            raw_video_frames_dropped: 3,
            encoded_video_frames_dropped: 2,
            audio_packets_dropped: 0,
            video_queue_depth: 2,
            audio_queue_depth: 0,
            video_bytes: 2_400_000,
            audio_bytes: 0,
        };
        let mut telemetry = HostedMediaTelemetry {
            session_id: "session-hosted".into(),
            transport_epoch: 7,
            selected_resource_ura: binding.subject_ura().to_string(),
            media_source_epoch: binding.media_source_epoch(),
            media_pipeline_id: config.backend.backend_id(),
            generation_started_at: Instant::now() - Duration::from_secs(4),
            adaptation_event_sequence: 0,
            latency: HostedLatencyWindow::default(),
        };
        telemetry
            .latency
            .record_encode_submit_to_rtp_write(now_ms().saturating_sub(8));
        telemetry
            .latency
            .record_writer_service(Duration::from_millis(73));
        let adaptation = HostedAdaptation::new(&config);
        let projection = telemetry.projection(
            &config,
            &video,
            &stats,
            11,
            4,
            2,
            false,
            None,
            &adaptation,
            Vec::new(),
        );

        assert_eq!(projection["contract"], MEDIA_PIPELINE_STATS_CONTRACT);
        assert_eq!(projection["session_id"], "session-hosted");
        assert_eq!(projection["selected_resource_ura"], binding.subject_ura());
        assert_eq!(projection["media_pipeline_id"], config.backend.backend_id());
        assert_eq!(
            projection["latency_stats"]["rtp_writer_service"]["p95_ms"],
            73.0
        );
        assert_eq!(projection["process_generation"], 11);
        assert_eq!(
            projection["payload_content_type"],
            H264_ANNEX_B_CONTENT_TYPE
        );
        assert_eq!(projection["target_bitrate_kbps"], 3_800);
        assert_eq!(projection["requested_fps"], 60);
        assert_eq!(projection["effective_fps"], 24);
        assert_eq!(projection["frames_encoded"], 100);
        assert_eq!(projection["frames_dropped"], 9);
        assert_eq!(projection["drop_policy"], "latest_frame_bounded_gop");
        assert!(projection["measured_fps"].as_f64().unwrap() > 0.0);
        assert!(projection["observed_bitrate_kbps"].as_f64().unwrap() > 0.0);
        assert_eq!(
            projection["latency_stats"]["encode_submit_to_rtp_write"]["samples"],
            1
        );
        assert!(
            projection["latency_stats"]["encode_submit_to_rtp_write"]["p95_ms"]
                .as_u64()
                .unwrap()
                > 0
        );
    }

    #[test]
    fn hosted_adaptation_events_bind_applied_encoder_state_to_session_generation() {
        let binding = test_application_target_binding();
        let config = BuiltinH264Config {
            backend: XCAP_OPENH264_WEBRTC_BACKEND,
            requested_fps: 60,
            fps: 30,
            bitrate_kbps: 4_800,
            h264_level: H264Level::Level3_1,
            max_frame_queue_depth: 3,
            keyframe_interval_frames: 30,
        };
        let mut telemetry = HostedMediaTelemetry {
            session_id: "session-adapt".into(),
            transport_epoch: 9,
            selected_resource_ura: binding.subject_ura().to_string(),
            media_source_epoch: binding.media_source_epoch(),
            media_pipeline_id: config.backend.backend_id(),
            generation_started_at: Instant::now(),
            adaptation_event_sequence: 0,
            latency: HostedLatencyWindow::default(),
        };
        let proposal = HostedReconfigurationProposal {
            video: VideoConfig {
                codec: VideoCodec::H264AnnexB,
                width: 1_280,
                height: 720,
                fps: 24,
                bitrate_kbps: 3_800,
                keyframe_interval_frames: 24,
                max_pending_frames: 3,
                max_access_unit_bytes: 1_048_576,
                max_nal_unit_bytes: H264_PACKETIZATION_NAL_BYTES,
                h264_profile_idc: 66,
                h264_level_idc: 31,
            },
            previous_bitrate_kbps: 4_800,
            previous_fps: 30,
            writer_service_limited: false,
        };
        let observation = HostedAdaptationObservation {
            reconfiguration: Some(proposal.clone()),
            input_drop_delta: 1,
            output_drop_delta: 2,
            receiver_frames_dropped_delta: 3,
            receiver_freeze_delta: 1,
            receiver_elevated_jitter: true,
            rtcp_receiver_pressure: RtcpReceiverPressure::default(),
        };
        let events = telemetry.adaptation_events(&observation, Some(&proposal));
        let event_types = events
            .iter()
            .filter_map(|event| event["event_type"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            event_types,
            vec![
                "backpressure_detected",
                "frame_drop",
                "bitrate_downshift",
                "fps_downshift"
            ]
        );
        assert!(events.iter().all(|event| {
            event["session_id"] == "session-adapt"
                && event["transport_epoch"] == 9
                && event["selected_resource_ura"] == binding.subject_ura()
                && event["media_source_epoch"] == binding.media_source_epoch()
                && event["media_pipeline_id"] == config.backend.backend_id()
        }));
    }
}
