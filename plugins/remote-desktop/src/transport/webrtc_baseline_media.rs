// EasyNet CLI — direct WebRTC baseline media paths
// =================================================
//
// File: plugins/remote-desktop/src/transport/webrtc_baseline_media.rs
// Description: xcap/OpenH264 capture, pacing and adaptive send strategies.
//
// Protocol Responsibility:
// - None. Session authority, transport epochs and browser feedback admission
//   are owned by the RemoteDesktop session aggregate.
//
// Implementation Approach:
// - Keep recorder input latest-frame bounded and apply an exact monotonic FPS
//   gate before pixel conversion/encoding.
// - Sample current-epoch receiver pressure once per second and transactionally
//   replace OpenH264 before committing a bitrate/FPS proposal.
//
// Usage Contract:
// - This module never creates a session or transport generation. All state and
//   telemetry writes remain fenced by `DirectWebRtcMediaExecution`.
//
// Architectural Position:
// - RemoteDesktop plugin baseline media strategy for Windows/Linux and the
//   non-native fallback on supported hosts.

use std::collections::VecDeque;
#[cfg(all(feature = "native-media", target_os = "windows"))]
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::{Duration, Instant};

use openh264::encoder::Encoder;
use serde_json::{json, Value};
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::rtp_transceiver::RtpSender;

#[cfg(all(feature = "native-media", target_os = "windows"))]
use crate::daemon::ability::builtins::resources::media::screen_snapshot::rgba_bytes_to_rgb_frame;
#[cfg(test)]
use crate::daemon::ability::builtins::resources::media::screen_snapshot::CaptureResizeMode;
use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
    capture_rgb_with_xcap, ScreenCaptureOptions,
};
use crate::daemon::plugins::remote_desktop::constants::ABILITY_SET_DESCRIPTION;
use crate::daemon::plugins::remote_desktop::media::adaptation::{
    effective_fps_for_bitrate, effective_fps_for_writer_service, AdaptiveBitrateController,
    ReceiverPressure, ReceiverPressureTracker,
};
#[cfg(all(feature = "native-media", target_os = "windows"))]
use crate::daemon::plugins::remote_desktop::media::encode::latest_recorder_frame_with_drop_count;
use crate::daemon::plugins::remote_desktop::media::encode::{
    build_openh264_encoder, even_rgb_frame, write_h264_sample, BuiltinH264Config,
};
#[cfg(all(
    feature = "native-media",
    any(target_os = "windows", target_os = "linux")
))]
use crate::daemon::plugins::remote_desktop::media::host_audio::{
    HostAudioCaptureStats, PreparedHostAudioRebind, RunningHostAudioCapture,
};
use crate::daemon::plugins::remote_desktop::media::{
    H264_ANNEX_B_CONTENT_TYPE, MEDIA_PIPELINE_STATS_CONTRACT, WEBRTC_VIDEO_TRANSPORT,
};
use crate::daemon::plugins::remote_desktop::session::now_ms;
use crate::daemon::plugins::remote_desktop::target::{
    DiagnosticCaptureSubject, RemoteAppTargetBinding, TargetResolutionError,
};
#[cfg(all(
    feature = "native-media",
    any(target_os = "windows", target_os = "linux")
))]
use crate::daemon::plugins::remote_desktop::transport::webrtc_audio::RemoteAppAudioPipeline;
use crate::daemon::plugins::remote_desktop::transport::webrtc_audio::{
    audio_stats_backend_unavailable, audio_stats_not_negotiated, RemoteAppAudioStats,
};
use crate::daemon::plugins::remote_desktop::transport::webrtc_media::DirectWebRtcMediaExecution;
use crate::daemon::plugins::remote_desktop::transport::webrtc_sender_feedback::RtcpReceiverPressureTracker;

#[cfg(all(feature = "native-media", target_os = "windows"))]
const RECORDER_FRAME_TIMEOUT_MS: u64 = 250;
const BASELINE_ADAPTATION_INTERVAL: Duration = Duration::from_secs(1);
const MIN_SAMPLE_DURATION: Duration = Duration::from_millis(1);
const MAX_SAMPLE_DURATION: Duration = Duration::from_millis(250);
const BASELINE_LATENCY_WINDOW_SAMPLES: usize = 512;
const ENCODER_RECONFIGURE_MAX_ATTEMPTS: u8 = 3;

#[cfg(all(feature = "native-media", target_os = "windows"))]
struct BaselineRecorderGuard {
    recorder: xcap::VideoRecorder,
}

#[cfg(all(feature = "native-media", target_os = "windows"))]
impl BaselineRecorderGuard {
    fn start(recorder: xcap::VideoRecorder) -> anyhow::Result<Self> {
        recorder.start()?;
        Ok(Self { recorder })
    }
}

#[cfg(all(feature = "native-media", target_os = "windows"))]
impl Drop for BaselineRecorderGuard {
    fn drop(&mut self) {
        let _ = self.recorder.stop();
    }
}

/// Immutable inputs shared by xcap-backed WebRTC baseline streams.
pub(in crate::daemon::plugins::remote_desktop) struct BaselineMediaInputs<'a> {
    pub(in crate::daemon::plugins::remote_desktop) track: &'a Arc<TrackLocalStaticSample>,
    pub(in crate::daemon::plugins::remote_desktop) video_sender: &'a Arc<dyn RtpSender>,
    pub(in crate::daemon::plugins::remote_desktop) ssrc: u32,
    pub(in crate::daemon::plugins::remote_desktop) payload_type: u8,
    pub(in crate::daemon::plugins::remote_desktop) audio_track:
        Option<&'a Arc<TrackLocalStaticSample>>,
    pub(in crate::daemon::plugins::remote_desktop) audio_payload_type: Option<u8>,
    pub(in crate::daemon::plugins::remote_desktop) options: &'a ScreenCaptureOptions,
    pub(in crate::daemon::plugins::remote_desktop) config: &'a BuiltinH264Config,
    pub(in crate::daemon::plugins::remote_desktop) target_binding: &'a RemoteAppTargetBinding,
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "windows", target_os = "linux")
))]
struct BaselineHostAudio {
    sink: crate::daemon::plugins::remote_desktop::media::audio::AudioSink,
    capture: RunningHostAudioCapture,
    pipeline: RemoteAppAudioPipeline,
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "windows", target_os = "linux")
))]
struct PreparedBaselineAudioRebind {
    capture: PreparedHostAudioRebind,
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "windows", target_os = "linux")
))]
impl BaselineHostAudio {
    async fn start(
        track: &Arc<TrackLocalStaticSample>,
        payload_type: u8,
        binding: &RemoteAppTargetBinding,
    ) -> anyhow::Result<Self> {
        // Start capture before constructing the writer. If either preparation
        // fails, both objects are dropped and the current generation remains
        // untouched.
        let capture = RunningHostAudioCapture::start(binding)?;
        let (sink, pipeline) = RemoteAppAudioPipeline::new(track, payload_type).await?;
        Ok(Self {
            sink,
            capture,
            pipeline,
        })
    }

    fn activate(&mut self) -> anyhow::Result<()> {
        self.capture.discard_pending()
    }

    async fn prepare_rebind(
        &mut self,
        binding: &RemoteAppTargetBinding,
    ) -> anyhow::Result<PreparedBaselineAudioRebind> {
        self.pipeline.quiesce_for_rebind().await?;
        match self.capture.prepare_rebind(binding) {
            Ok(capture) => Ok(PreparedBaselineAudioRebind { capture }),
            Err(error) => {
                // Capture preparation preserves/resumes the previous source on
                // failure. Restore its sole writer before returning.
                self.pipeline.activate_after_rebind();
                Err(error)
            }
        }
    }

    fn commit_rebind(&mut self, prepared: PreparedBaselineAudioRebind) {
        // Writer activation is infallible and happens before capture resumes,
        // so the first admitted PCM chunk always has a live consumer.
        self.pipeline.activate_after_rebind();
        self.capture.commit_rebind(prepared.capture);
    }

    fn rollback_rebind(&mut self, prepared: PreparedBaselineAudioRebind) -> anyhow::Result<()> {
        self.capture.rollback_rebind(prepared.capture)?;
        self.pipeline.activate_after_rebind();
        Ok(())
    }

    fn pump(&mut self) {
        self.capture.pump(&self.sink);
        self.pipeline.drain();
    }

    fn stats(&self) -> RemoteAppAudioStats {
        let capture = self.capture.stats();
        let mut stats = self.pipeline.stats();
        apply_capture_stats(&mut stats, capture);
        stats
    }

    async fn shutdown_discard(&mut self) {
        self.capture.stop();
        self.pipeline.shutdown_discard().await;
    }
}

/// Audio is an independently negotiated media component. An unavailable host
/// audio backend must remain visible in product evidence, but it cannot abort
/// an otherwise valid video session. Failures after a running audio pipeline
/// has been admitted remain terminal for that negotiated media contract.
enum BaselineAudioState {
    NotNegotiated,
    #[cfg(all(
        feature = "native-media",
        any(target_os = "windows", target_os = "linux")
    ))]
    Running(BaselineHostAudio),
    NegotiatedButBackendUnavailable {
        reason: String,
    },
}

impl BaselineAudioState {
    #[cfg(all(
        feature = "native-media",
        any(target_os = "windows", target_os = "linux")
    ))]
    async fn prepare(
        track: Option<&Arc<TrackLocalStaticSample>>,
        payload_type: Option<u8>,
        binding: &RemoteAppTargetBinding,
    ) -> anyhow::Result<Self> {
        let (track, payload_type) = match (track, payload_type) {
            (None, None) => return Ok(Self::NotNegotiated),
            (Some(track), Some(payload_type)) => (track, payload_type),
            _ => {
                anyhow::bail!("WebRTC audio negotiation returned an incomplete track/payload pair")
            }
        };
        match BaselineHostAudio::start(track, payload_type, binding).await {
            Ok(audio) => Ok(Self::Running(audio)),
            Err(error) => Ok(Self::NegotiatedButBackendUnavailable {
                reason: format!("host_audio_backend_unavailable: {error:#}"),
            }),
        }
    }

    #[cfg(all(
        feature = "native-media",
        any(target_os = "windows", target_os = "linux")
    ))]
    async fn activate(&mut self) {
        let activation_error = match self {
            Self::Running(audio) => audio.activate().err(),
            Self::NotNegotiated | Self::NegotiatedButBackendUnavailable { .. } => None,
        };
        let Some(error) = activation_error else {
            return;
        };
        let previous = std::mem::replace(
            self,
            Self::NegotiatedButBackendUnavailable {
                reason: format!("host_audio_backend_unavailable: {error:#}"),
            },
        );
        if let Self::Running(mut audio) = previous {
            audio.shutdown_discard().await;
        }
    }

    #[cfg(all(
        feature = "native-media",
        any(target_os = "windows", target_os = "linux")
    ))]
    fn running_mut(&mut self) -> Option<&mut BaselineHostAudio> {
        match self {
            Self::Running(audio) => Some(audio),
            Self::NotNegotiated | Self::NegotiatedButBackendUnavailable { .. } => None,
        }
    }

    fn pump(&mut self) -> RemoteAppAudioStats {
        match self {
            Self::NotNegotiated => audio_stats_not_negotiated(),
            #[cfg(all(
                feature = "native-media",
                any(target_os = "windows", target_os = "linux")
            ))]
            Self::Running(audio) => {
                audio.pump();
                audio.stats()
            }
            Self::NegotiatedButBackendUnavailable { reason } => {
                audio_stats_backend_unavailable(reason.clone())
            }
        }
    }

    fn terminal_failure(&self, _stats: &RemoteAppAudioStats) -> Option<String> {
        match self {
            #[cfg(all(
                feature = "native-media",
                any(target_os = "windows", target_os = "linux")
            ))]
            Self::Running(_) => _stats.terminal_failure().map(str::to_string),
            Self::NotNegotiated | Self::NegotiatedButBackendUnavailable { .. } => None,
        }
    }

    #[cfg(all(
        feature = "native-media",
        any(target_os = "windows", target_os = "linux")
    ))]
    async fn shutdown_discard(&mut self) {
        if let Self::Running(audio) = self {
            audio.shutdown_discard().await;
        }
    }
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "windows", target_os = "linux")
))]
fn apply_capture_stats(stats: &mut RemoteAppAudioStats, capture: HostAudioCaptureStats) {
    stats.capture_source = capture.source;
    stats.capture_chunks_forwarded = capture.chunks_forwarded;
    stats.capture_backend_chunks_dropped = capture.backend_chunks_dropped;
    stats.capture_stall_events = capture.stall_events;
    stats.capture_recovery_events = capture.recovery_events;
    stats.precommit_chunks_discarded = capture.precommit_chunks_discarded;
    if stats.blocker.is_none() {
        stats.blocker = capture.terminal_error;
    }
}

fn pump_baseline_audio(audio: &mut BaselineAudioState) -> RemoteAppAudioStats {
    audio.pump()
}

#[cfg(not(all(
    feature = "native-media",
    any(target_os = "windows", target_os = "linux")
)))]
fn prepare_unavailable_baseline_audio(
    track: Option<&Arc<TrackLocalStaticSample>>,
    payload_type: Option<u8>,
) -> anyhow::Result<BaselineAudioState> {
    match (track, payload_type) {
        (None, None) => Ok(BaselineAudioState::NotNegotiated),
        (Some(_), Some(_)) => Ok(BaselineAudioState::NegotiatedButBackendUnavailable {
            reason: format!(
                "baseline host-audio backend unavailable on {} with native-media={}",
                std::env::consts::OS,
                cfg!(feature = "native-media")
            ),
        }),
        _ => anyhow::bail!("WebRTC audio negotiation returned an incomplete track/payload pair"),
    }
}

fn terminal_audio_failure(
    audio: &BaselineAudioState,
    stats: &RemoteAppAudioStats,
) -> Option<String> {
    audio.terminal_failure(stats)
}

#[derive(Debug)]
struct BaselineFramePacer {
    interval: Duration,
    last_admitted_at: Option<Instant>,
    next_frame_at: Option<Instant>,
}

impl BaselineFramePacer {
    fn new(fps: u32) -> Self {
        Self {
            interval: frame_interval(fps),
            last_admitted_at: None,
            next_frame_at: None,
        }
    }

    fn set_fps(&mut self, fps: u32) {
        self.interval = frame_interval(fps);
        if let Some(last_admitted_at) = self.last_admitted_at {
            let rate_ceiling = last_admitted_at + self.interval;
            self.next_frame_at = Some(
                self.next_frame_at
                    .map_or(rate_ceiling, |deadline| deadline.max(rate_ceiling)),
            );
        }
    }

    fn admit(&mut self, observed_at: Instant) -> bool {
        if self
            .next_frame_at
            .is_some_and(|next_frame_at| observed_at < next_frame_at)
        {
            return false;
        }
        // Do not catch up with bursts after capture or scheduling stalls. The
        // newest admitted frame starts a fresh interval.
        self.last_admitted_at = Some(observed_at);
        self.next_frame_at = Some(observed_at + self.interval);
        true
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingEncoderReconfiguration {
    bitrate_kbps: u32,
    fps: u32,
    pressure: ReceiverPressure,
    attempts: u8,
}

#[derive(Debug)]
struct BaselineReconfigurationOutcome {
    event: Value,
    applied: bool,
}

#[derive(Debug, Default)]
struct BaselineLatencyWindow {
    encode_to_write_samples_ms: VecDeque<f64>,
    writer_service_samples_ms: VecDeque<f64>,
}

impl BaselineLatencyWindow {
    fn record(&mut self, latency: Duration) {
        if self.encode_to_write_samples_ms.len() == BASELINE_LATENCY_WINDOW_SAMPLES {
            self.encode_to_write_samples_ms.pop_front();
        }
        self.encode_to_write_samples_ms
            .push_back(latency.as_secs_f64() * 1000.0);
    }

    fn record_writer_service(&mut self, latency: Duration) {
        if self.writer_service_samples_ms.len() == BASELINE_LATENCY_WINDOW_SAMPLES {
            self.writer_service_samples_ms.pop_front();
        }
        self.writer_service_samples_ms
            .push_back(latency.as_secs_f64() * 1000.0);
    }

    fn writer_service_p95(&self) -> (usize, f64) {
        let mut sorted: Vec<f64> = self.writer_service_samples_ms.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        let p95_ms = sorted
            .get(((sorted.len().saturating_sub(1)) * 95) / 100)
            .copied()
            .unwrap_or(0.0);
        (sorted.len(), p95_ms)
    }

    fn to_json(&self) -> Value {
        let mut sorted: Vec<f64> = self.encode_to_write_samples_ms.iter().copied().collect();
        sorted.sort_by(f64::total_cmp);
        let p95_ms = if sorted.is_empty() {
            0.0
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
}

#[derive(Debug)]
struct BaselineMediaController {
    session_id: String,
    transport_epoch: u64,
    selected_resource_ura: String,
    media_source_epoch: u64,
    media_pipeline_id: &'static str,
    bitrate: AdaptiveBitrateController,
    receiver_pressure: ReceiverPressureTracker,
    pending_reconfiguration: Option<PendingEncoderReconfiguration>,
    effective_fps: u32,
    pacer: BaselineFramePacer,
    stream_started_at: Instant,
    generation_started_at: Instant,
    last_adaptation_at: Instant,
    last_written_at: Option<Instant>,
    received_frames: u64,
    stale_frames_dropped: u64,
    pacing_frames_dropped: u64,
    receiver_frames_dropped: u64,
    encoded_frames: u64,
    encoded_bytes: u64,
    encoder_reconfigurations: u64,
    adaptation_event_sequence: u64,
    active_keyframe_interval_frames: u32,
    last_width: usize,
    last_height: usize,
    latency: BaselineLatencyWindow,
}

impl BaselineMediaController {
    fn new(
        config: &BuiltinH264Config,
        execution: &DirectWebRtcMediaExecution<'_>,
        target_binding: &RemoteAppTargetBinding,
    ) -> Self {
        let now = Instant::now();
        Self {
            session_id: execution.session_id().to_string(),
            transport_epoch: execution.epoch().value(),
            selected_resource_ura: target_binding.subject_ura().to_string(),
            media_source_epoch: target_binding.media_source_epoch(),
            media_pipeline_id: config.backend.backend_id(),
            bitrate: AdaptiveBitrateController::new(config.bitrate_kbps),
            receiver_pressure: ReceiverPressureTracker::default(),
            pending_reconfiguration: None,
            effective_fps: config.fps.max(1),
            pacer: BaselineFramePacer::new(config.fps),
            stream_started_at: now,
            generation_started_at: now,
            last_adaptation_at: now,
            last_written_at: None,
            received_frames: 0,
            stale_frames_dropped: 0,
            pacing_frames_dropped: 0,
            receiver_frames_dropped: 0,
            encoded_frames: 0,
            encoded_bytes: 0,
            encoder_reconfigurations: 0,
            adaptation_event_sequence: 0,
            active_keyframe_interval_frames: config.keyframe_interval_frames,
            last_width: 0,
            last_height: 0,
            latency: BaselineLatencyWindow::default(),
        }
    }

    #[cfg(all(feature = "native-media", target_os = "windows"))]
    fn observe_recorder_batch(&mut self, stale_dropped: u64, observed_at: Instant) -> bool {
        self.received_frames = self
            .received_frames
            .saturating_add(stale_dropped.saturating_add(1));
        self.stale_frames_dropped = self.stale_frames_dropped.saturating_add(stale_dropped);
        if self.pacer.admit(observed_at) {
            true
        } else {
            self.pacing_frames_dropped = self.pacing_frames_dropped.saturating_add(1);
            false
        }
    }

    fn observe_polling_frame(&mut self, observed_at: Instant) {
        self.received_frames = self.received_frames.saturating_add(1);
        let _ = self.pacer.admit(observed_at);
    }

    fn presentation(&self, observed_at: Instant) -> (u64, Duration) {
        let timestamp_ms = observed_at
            .saturating_duration_since(self.stream_started_at)
            .as_millis()
            .min(u64::MAX as u128) as u64;
        let nominal = frame_interval(self.effective_fps);
        let duration = self
            .last_written_at
            .map(|last| observed_at.saturating_duration_since(last))
            .unwrap_or(nominal)
            .clamp(MIN_SAMPLE_DURATION, MAX_SAMPLE_DURATION);
        (timestamp_ms, duration)
    }

    fn record_encoded(
        &mut self,
        bytes: Option<usize>,
        observed_at: Instant,
        writer_service: Duration,
        width: usize,
        height: usize,
    ) {
        let Some(bytes) = bytes else {
            return;
        };
        self.encoded_frames = self.encoded_frames.saturating_add(1);
        self.encoded_bytes = self.encoded_bytes.saturating_add(bytes as u64);
        self.last_written_at = Some(observed_at);
        self.last_width = width;
        self.last_height = height;
        self.latency.record(observed_at.elapsed());
        self.latency.record_writer_service(writer_service);
    }

    async fn adapt_and_record(
        &mut self,
        execution: &DirectWebRtcMediaExecution<'_>,
        video_sender: &Arc<dyn RtpSender>,
        rtcp_receiver: &mut RtcpReceiverPressureTracker,
        encoder: &mut Encoder,
        config: &BuiltinH264Config,
        audio: RemoteAppAudioStats,
        force: bool,
        terminal: bool,
    ) {
        if !force && self.last_adaptation_at.elapsed() < BASELINE_ADAPTATION_INTERVAL {
            return;
        }
        let sampled_at_ms = now_ms();
        let feedback = execution
            .sessions()
            .client_media_feedback_for_session(execution.session_id(), execution.epoch());
        let pressure = self.receiver_pressure.observe(feedback, Instant::now());
        let rtcp_receiver_pressure = rtcp_receiver.observe(video_sender, Instant::now()).await;
        let mut adaptation_events = Vec::new();
        if !terminal {
            if pressure.pressure_units() > 0 || rtcp_receiver_pressure.pressure_units() > 0 {
                self.receiver_frames_dropped = self
                    .receiver_frames_dropped
                    .saturating_add(pressure.frames_dropped_delta);
                adaptation_events.push(self.event(
                    "backpressure_detected",
                    sampled_at_ms,
                    json!({
                        "receiver_frame_drop_delta": pressure.frames_dropped_delta,
                        "receiver_freeze_delta": pressure.freeze_delta,
                        "receiver_elevated_jitter": pressure.elevated_jitter,
                        "rtcp_report_fresh": rtcp_receiver_pressure.fresh,
                        "rtcp_packets_lost_delta": rtcp_receiver_pressure.packets_lost_delta,
                        "rtcp_fraction_lost": rtcp_receiver_pressure.fraction_lost,
                        "rtcp_round_trip_time_ms": rtcp_receiver_pressure.round_trip_time_ms,
                        "rtcp_stats_read_failed": rtcp_receiver_pressure.stats_read_failed,
                    }),
                ));
                if pressure.frames_dropped_delta > 0 {
                    adaptation_events.push(self.event(
                        "frame_drop",
                        sampled_at_ms,
                        json!({
                            "receiver_frame_drop_delta": pressure.frames_dropped_delta,
                            "source": "authenticated_browser_feedback",
                        }),
                    ));
                }
            }
            let proposal = self.pending_reconfiguration.take().or_else(|| {
                let proposed_bitrate = self.bitrate.propose(
                    0,
                    0,
                    0,
                    0,
                    pressure
                        .pressure_units()
                        .saturating_add(rtcp_receiver_pressure.pressure_units()),
                );
                let bitrate_kbps = proposed_bitrate.unwrap_or(self.bitrate.current_kbps);
                let bitrate_fps =
                    effective_fps_for_bitrate(config.fps, bitrate_kbps, self.bitrate.target_kbps);
                let (writer_samples, writer_p95_ms) = self.latency.writer_service_p95();
                let writer_fps =
                    effective_fps_for_writer_service(config.fps, writer_p95_ms, writer_samples);
                let fps = bitrate_fps.min(writer_fps);
                (bitrate_kbps != self.bitrate.current_kbps || fps != self.effective_fps).then_some(
                    PendingEncoderReconfiguration {
                        bitrate_kbps,
                        fps,
                        pressure,
                        attempts: 0,
                    },
                )
            });
            if let Some(proposal) = proposal {
                let outcome = self.apply_proposal(
                    encoder,
                    config,
                    proposal.bitrate_kbps,
                    proposal.fps,
                    proposal.pressure,
                    proposal.attempts.saturating_add(1),
                    sampled_at_ms,
                );
                self.retain_failed_proposal(proposal, outcome.applied);
                if outcome.applied {
                    let previous_fps = outcome.event["detail"]["previous_fps"]
                        .as_u64()
                        .unwrap_or(self.effective_fps as u64)
                        as u32;
                    let next_fps = outcome.event["detail"]["next_fps"]
                        .as_u64()
                        .unwrap_or(self.effective_fps as u64)
                        as u32;
                    if next_fps != previous_fps {
                        adaptation_events.push(self.event(
                            if next_fps < previous_fps {
                                "fps_downshift"
                            } else {
                                "fps_upshift"
                            },
                            sampled_at_ms,
                            json!({
                                "previous_fps": previous_fps,
                                "next_fps": next_fps,
                                "cause": "writer_service_or_bitrate_control",
                            }),
                        ));
                    }
                }
                adaptation_events.push(outcome.event);
            }
        }
        execution.record_pipeline_stats(self.stats(
            config,
            sampled_at_ms,
            adaptation_events,
            audio,
            terminal,
        ));
        self.last_adaptation_at = Instant::now();
    }

    fn retain_failed_proposal(
        &mut self,
        mut proposal: PendingEncoderReconfiguration,
        applied: bool,
    ) {
        if applied {
            return;
        }
        proposal.attempts = proposal.attempts.saturating_add(1);
        if proposal.attempts < ENCODER_RECONFIGURE_MAX_ATTEMPTS {
            self.pending_reconfiguration = Some(proposal);
        }
    }

    fn apply_proposal(
        &mut self,
        encoder: &mut Encoder,
        config: &BuiltinH264Config,
        next_bitrate_kbps: u32,
        next_fps: u32,
        pressure: ReceiverPressure,
        attempt: u8,
        sampled_at_ms: u64,
    ) -> BaselineReconfigurationOutcome {
        self.apply_proposal_with(
            encoder,
            config,
            next_bitrate_kbps,
            next_fps,
            pressure,
            attempt,
            sampled_at_ms,
            build_openh264_encoder,
        )
    }

    fn apply_proposal_with<F>(
        &mut self,
        encoder: &mut Encoder,
        config: &BuiltinH264Config,
        next_bitrate_kbps: u32,
        next_fps: u32,
        pressure: ReceiverPressure,
        attempt: u8,
        sampled_at_ms: u64,
        build_encoder: F,
    ) -> BaselineReconfigurationOutcome
    where
        F: FnOnce(&BuiltinH264Config) -> anyhow::Result<Encoder>,
    {
        let previous_bitrate_kbps = self.bitrate.current_kbps;
        let previous_fps = self.effective_fps;
        let mut next_config = config.clone();
        next_config.bitrate_kbps = next_bitrate_kbps;
        next_config.fps = next_fps;
        next_config.keyframe_interval_frames = next_fps.clamp(1, 30);
        match build_encoder(&next_config) {
            Ok(next_encoder) => {
                *encoder = next_encoder;
                self.bitrate.commit_applied(next_bitrate_kbps);
                self.effective_fps = next_fps;
                self.pacer.set_fps(next_fps);
                self.encoder_reconfigurations = self.encoder_reconfigurations.saturating_add(1);
                self.active_keyframe_interval_frames = next_config.keyframe_interval_frames;
                BaselineReconfigurationOutcome {
                    applied: true,
                    event: self.event(
                        if next_bitrate_kbps < previous_bitrate_kbps {
                            "bitrate_downshift"
                        } else if next_bitrate_kbps > previous_bitrate_kbps {
                            "bitrate_upshift"
                        } else {
                            "fps_reconfigure"
                        },
                        sampled_at_ms,
                        json!({
                            "algorithm": "receiver_feedback_openh264_rebuild",
                            "previous_bitrate_kbps": previous_bitrate_kbps,
                            "next_bitrate_kbps": next_bitrate_kbps,
                            "previous_fps": previous_fps,
                            "next_fps": next_fps,
                            "frames_dropped_delta": pressure.frames_dropped_delta,
                            "freeze_delta": pressure.freeze_delta,
                            "elevated_jitter": pressure.elevated_jitter,
                            "reconfigure_attempt": attempt,
                            "reconfigure_max_attempts": ENCODER_RECONFIGURE_MAX_ATTEMPTS,
                        }),
                    ),
                }
            }
            Err(error) => BaselineReconfigurationOutcome {
                applied: false,
                event: self.event(
                    "encoder_reconfigure_failed",
                    sampled_at_ms,
                    json!({
                        "algorithm": "receiver_feedback_openh264_rebuild",
                        "requested_bitrate_kbps": next_bitrate_kbps,
                        "active_bitrate_kbps": previous_bitrate_kbps,
                        "active_fps": previous_fps,
                        "reconfigure_attempt": attempt,
                        "reconfigure_max_attempts": ENCODER_RECONFIGURE_MAX_ATTEMPTS,
                        "reconfigure_exhausted": attempt >= ENCODER_RECONFIGURE_MAX_ATTEMPTS,
                        "error": error.to_string(),
                    }),
                ),
            },
        }
    }

    fn event(&mut self, event_type: &'static str, observed_at_ms: u64, detail: Value) -> Value {
        self.adaptation_event_sequence = self.adaptation_event_sequence.saturating_add(1);
        json!({
            "event_type": event_type,
            "sequence": self.adaptation_event_sequence,
            "observed_at_ms": observed_at_ms,
            "session_id": self.session_id,
            "transport_epoch": self.transport_epoch,
            "selected_resource_ura": self.selected_resource_ura,
            "media_source_epoch": self.media_source_epoch,
            "media_pipeline_id": self.media_pipeline_id,
            "video_codec": "h264",
            "video_transport": WEBRTC_VIDEO_TRANSPORT,
            "audio_codec": Value::Null,
            "detail": detail,
        })
    }

    fn stats(
        &self,
        config: &BuiltinH264Config,
        sampled_at_ms: u64,
        adaptation_events: Vec<Value>,
        audio: RemoteAppAudioStats,
        terminal: bool,
    ) -> Value {
        let frames_dropped = self
            .stale_frames_dropped
            .saturating_add(self.pacing_frames_dropped)
            .saturating_add(self.receiver_frames_dropped);
        let elapsed = self.generation_started_at.elapsed();
        let elapsed_secs = elapsed.as_secs_f64();
        let measured_fps = if elapsed_secs > 0.0 {
            self.encoded_frames as f64 / elapsed_secs
        } else {
            0.0
        };
        let observed_bitrate_kbps = if elapsed_secs > 0.0 {
            (self.encoded_bytes as f64 * 8.0 / 1000.0) / elapsed_secs
        } else {
            0.0
        };
        let mut stats = json!({
            "contract": MEDIA_PIPELINE_STATS_CONTRACT,
            "path": "baseline_webrtc",
            "sampled_at_ms": sampled_at_ms,
            "session_id": self.session_id,
            "transport_epoch": self.transport_epoch,
            "selected_resource_ura": self.selected_resource_ura,
            "media_source_epoch": self.media_source_epoch,
            "media_pipeline_id": config.backend.backend_id(),
            "capture_api": config.backend.capture_api(),
            "backend_id": config.backend.backend_id(),
            "encoder_name": config.backend.encoder(),
            "carrier": config.backend.carrier(),
            "video_codec": "h264",
            "codec_negotiated": true,
            "payload_content_type": H264_ANNEX_B_CONTENT_TYPE,
            "video_transport": WEBRTC_VIDEO_TRANSPORT,
            "target_bitrate_kbps": self.bitrate.current_kbps,
            "configured_target_bitrate_kbps": self.bitrate.target_kbps,
            "min_bitrate_kbps": self.bitrate.min_kbps,
            "requested_fps": config.requested_fps,
            "configured_fps": config.fps,
            "effective_fps": self.effective_fps,
            "target_fps": self.effective_fps,
        });
        let Value::Object(stats_object) = &mut stats else {
            unreachable!("baseline stats projection must be an object");
        };
        let Value::Object(runtime_stats) = json!({
            "measured_fps": measured_fps,
            "observed_bitrate_kbps": observed_bitrate_kbps,
            "keyframe_interval_frames": self.active_keyframe_interval_frames,
            "width": self.last_width,
            "height": self.last_height,
            "received_frames": self.received_frames,
            "frames_encoded": self.encoded_frames,
            "encoded_bytes": self.encoded_bytes,
            "frames_dropped": frames_dropped,
            "stale_frames_dropped": self.stale_frames_dropped,
            "pacing_frames_dropped": self.pacing_frames_dropped,
            "receiver_frames_dropped": self.receiver_frames_dropped,
            "drop_stale_frames": true,
            "drop_policy": "bounded_queue_drop_stale_frames",
            "backpressure_policy": "latest_frame_and_receiver_feedback",
            "max_frame_queue_depth": config.max_frame_queue_depth,
            "queued_units": 0,
            "in_flight_frames": 0,
            "encoder_reconfigurations": self.encoder_reconfigurations,
            "adaptation_algorithm": "transport_feedback",
            "adaptation_events": adaptation_events,
            "latency_stats": self.latency.to_json(),
            "stream_elapsed_ms": elapsed.as_millis().min(u64::MAX as u128) as u64,
            "terminal": terminal,
        }) else {
            unreachable!("baseline runtime stats projection must be an object");
        };
        stats_object.extend(runtime_stats);
        audio.append_json(stats_object);
        stats
    }

    fn commit_media_rebind(&mut self, binding: &RemoteAppTargetBinding) {
        self.begin_media_generation(binding.subject_ura(), binding.media_source_epoch());
    }

    fn begin_media_generation(&mut self, selected_resource_ura: &str, media_source_epoch: u64) {
        self.selected_resource_ura = selected_resource_ura.to_string();
        self.media_source_epoch = media_source_epoch;
        self.generation_started_at = Instant::now();
        self.last_written_at = None;
        self.received_frames = 0;
        self.stale_frames_dropped = 0;
        self.pacing_frames_dropped = 0;
        self.receiver_frames_dropped = 0;
        self.encoded_frames = 0;
        self.encoded_bytes = 0;
        self.encoder_reconfigurations = 0;
        self.last_width = 0;
        self.last_height = 0;
        self.latency = BaselineLatencyWindow::default();
        self.receiver_pressure = ReceiverPressureTracker::default();
        self.pending_reconfiguration = None;
    }

    fn active_encoder_config(&self, base: &BuiltinH264Config) -> BuiltinH264Config {
        let mut active = base.clone();
        active.bitrate_kbps = self.bitrate.current_kbps;
        active.fps = self.effective_fps;
        active.keyframe_interval_frames = self.active_keyframe_interval_frames;
        active
    }
}

/// Windows xcap uses a synchronous producer handoff. Linux xcap recorder
/// implementations use unbounded channels, so Linux intentionally stays on
/// the paced polling path instead of claiming bounded latest-frame capture.
#[cfg(all(feature = "native-media", target_os = "windows"))]
pub(in crate::daemon::plugins::remote_desktop) async fn run_direct_webrtc_recorder_stream(
    execution: &mut DirectWebRtcMediaExecution<'_>,
    inputs: &BaselineMediaInputs<'_>,
    recorder: xcap::VideoRecorder,
    rx: std::sync::mpsc::Receiver<xcap::Frame>,
) -> anyhow::Result<()> {
    let BaselineMediaInputs {
        track,
        video_sender,
        ssrc,
        payload_type,
        audio_track,
        audio_payload_type,
        options,
        config,
        target_binding,
    } = *inputs;
    let mut encoder = build_openh264_encoder(config)?;
    let mut controller = BaselineMediaController::new(config, execution, target_binding);
    let mut rtcp_receiver = RtcpReceiverPressureTracker::default();
    let mut audio =
        BaselineAudioState::prepare(audio_track, audio_payload_type, target_binding).await?;
    audio.activate().await;
    let _recorder_guard = BaselineRecorderGuard::start(recorder)?;
    let mut media_ready_reported = false;
    let mut terminal_audio_error = None;
    loop {
        if execution.should_stop() {
            break;
        }
        if execution
            .sessions()
            .pending_media_rebind_binding_for_session(
                execution.session_id(),
                execution.epoch(),
                controller.media_source_epoch,
            )
            .is_some()
        {
            anyhow::bail!("baseline recorder generation must restart before a target media rebind");
        }
        match rx.recv_timeout(Duration::from_millis(RECORDER_FRAME_TIMEOUT_MS)) {
            Ok(frame) => {
                let (frame, stale_dropped) = latest_recorder_frame_with_drop_count(&rx, frame);
                let observed_at = Instant::now();
                if !controller.observe_recorder_batch(stale_dropped, observed_at) {
                    let audio_stats = pump_baseline_audio(&mut audio);
                    let audio_error = terminal_audio_failure(&audio, &audio_stats);
                    controller
                        .adapt_and_record(
                            execution,
                            video_sender,
                            &mut rtcp_receiver,
                            &mut encoder,
                            config,
                            audio_stats,
                            false,
                            false,
                        )
                        .await;
                    if let Some(error) = audio_error {
                        terminal_audio_error = Some(error);
                        break;
                    }
                    continue;
                }
                let frame = rgba_bytes_to_rgb_frame(frame.raw, frame.width, frame.height, options)
                    .map(even_rgb_frame)?;
                let (timestamp_ms, sample_duration) = controller.presentation(observed_at);
                let writer_started_at = Instant::now();
                let written = write_h264_sample(
                    track,
                    ssrc,
                    payload_type,
                    &mut encoder,
                    &frame,
                    timestamp_ms,
                    sample_duration,
                )
                .await?;
                if written.is_some() && !media_ready_reported {
                    execution.mark_media_ready();
                    media_ready_reported = true;
                }
                controller.record_encoded(
                    written,
                    observed_at,
                    writer_started_at.elapsed(),
                    frame.width as usize,
                    frame.height as usize,
                );
                let audio_stats = pump_baseline_audio(&mut audio);
                let audio_error = terminal_audio_failure(&audio, &audio_stats);
                controller
                    .adapt_and_record(
                        execution,
                        video_sender,
                        &mut rtcp_receiver,
                        &mut encoder,
                        config,
                        audio_stats,
                        false,
                        false,
                    )
                    .await;
                if let Some(error) = audio_error {
                    terminal_audio_error = Some(error);
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                let audio_stats = pump_baseline_audio(&mut audio);
                let audio_error = terminal_audio_failure(&audio, &audio_stats);
                controller
                    .adapt_and_record(
                        execution,
                        video_sender,
                        &mut rtcp_receiver,
                        &mut encoder,
                        config,
                        audio_stats,
                        false,
                        false,
                    )
                    .await;
                if let Some(error) = audio_error {
                    terminal_audio_error = Some(error);
                    break;
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let audio_stats = pump_baseline_audio(&mut audio);
    terminal_audio_error =
        terminal_audio_error.or_else(|| terminal_audio_failure(&audio, &audio_stats));
    controller
        .adapt_and_record(
            execution,
            video_sender,
            &mut rtcp_receiver,
            &mut encoder,
            config,
            audio_stats,
            true,
            true,
        )
        .await;
    audio.shutdown_discard().await;
    match terminal_audio_error {
        Some(error) => anyhow::bail!("negotiated host-audio pipeline failed: {error}"),
        None => Ok(()),
    }
}

pub(in crate::daemon::plugins::remote_desktop) async fn run_direct_webrtc_polling_stream(
    execution: &mut DirectWebRtcMediaExecution<'_>,
    inputs: &BaselineMediaInputs<'_>,
    capture_subject: &DiagnosticCaptureSubject,
) -> anyhow::Result<()> {
    let BaselineMediaInputs {
        track,
        video_sender,
        ssrc,
        payload_type,
        audio_track,
        audio_payload_type,
        options,
        config,
        target_binding,
    } = *inputs;
    let mut active_capture_subject = capture_subject.clone();
    let mut encoder = build_openh264_encoder(config)?;
    let mut controller = BaselineMediaController::new(config, execution, target_binding);
    let mut rtcp_receiver = RtcpReceiverPressureTracker::default();
    #[cfg(all(
        feature = "native-media",
        any(target_os = "windows", target_os = "linux")
    ))]
    let mut audio =
        BaselineAudioState::prepare(audio_track, audio_payload_type, target_binding).await?;
    #[cfg(all(
        feature = "native-media",
        any(target_os = "windows", target_os = "linux")
    ))]
    audio.activate().await;
    #[cfg(not(all(
        feature = "native-media",
        any(target_os = "windows", target_os = "linux")
    )))]
    let mut audio = prepare_unavailable_baseline_audio(audio_track, audio_payload_type)?;
    let mut media_ready_reported = false;
    let mut terminal_audio_error = None;
    loop {
        if execution.should_stop() {
            break;
        }
        let capture_started = Instant::now();
        let pending_binding = execution
            .sessions()
            .pending_media_rebind_binding_for_session(
                execution.session_id(),
                execution.epoch(),
                controller.media_source_epoch,
            );
        let capture_entry = pending_binding
            .as_ref()
            .map(|pending| pending.binding.diagnostic_capture_subject())
            .unwrap_or(&active_capture_subject)
            .to_backend_resource_entry();
        let frame = match capture_rgb_with_xcap(&capture_entry, options) {
            Ok(frame) => even_rgb_frame(frame),
            Err(error) if pending_binding.is_some() => {
                execution
                    .sessions()
                    .supersede_pending_media_rebind_for_session(
                        execution.session_id(),
                        execution.epoch(),
                        &pending_binding
                            .as_ref()
                            .expect("pending capture failure has a rebind token")
                            .attempt_token,
                        TargetResolutionError::CaptureBackendUnavailable,
                        format!("xcap pending target capture failed: {error}"),
                    );
                continue;
            }
            Err(error) => return Err(error),
        };
        if let Some(pending) = pending_binding {
            let pending_binding = &pending.binding;
            let proof = match pending_binding.require_capture_proof(ABILITY_SET_DESCRIPTION) {
                Ok(proof) => proof
                    .clone()
                    .reverified_with_native_dimensions(Some(frame.native_dimensions())),
                Err(error) => {
                    execution
                        .sessions()
                        .supersede_pending_media_rebind_for_session(
                            execution.session_id(),
                            execution.epoch(),
                            &pending.attempt_token,
                            error.reason(),
                            error.to_string(),
                        );
                    continue;
                }
            };
            if let Err(error) = pending_binding
                .validate_pending_media_rebind_capture_proof(ABILITY_SET_DESCRIPTION, &proof)
            {
                execution
                    .sessions()
                    .supersede_pending_media_rebind_for_session(
                        execution.session_id(),
                        execution.epoch(),
                        &pending.attempt_token,
                        error.reason(),
                        error.to_string(),
                    );
                continue;
            }
            let next_encoder = build_openh264_encoder(&controller.active_encoder_config(config))?;
            #[cfg(all(
                feature = "native-media",
                any(target_os = "windows", target_os = "linux")
            ))]
            let prepared_audio = match audio.running_mut() {
                Some(audio) => match audio.prepare_rebind(&pending_binding).await {
                    Ok(prepared) => Some(prepared),
                    Err(error) => {
                        execution
                            .sessions()
                            .supersede_pending_media_rebind_for_session(
                                execution.session_id(),
                                execution.epoch(),
                                &pending.attempt_token,
                                TargetResolutionError::CaptureBackendUnavailable,
                                format!("pending target host-audio preparation failed: {error}"),
                            );
                        continue;
                    }
                },
                None => None,
            };
            if !execution
                .sessions()
                .commit_pending_media_rebind_for_session(
                    execution.session_id(),
                    execution.epoch(),
                    pending_binding.binding_epoch(),
                    pending_binding.media_source_epoch(),
                    &pending.attempt_token,
                    proof,
                )
            {
                #[cfg(all(
                    feature = "native-media",
                    any(target_os = "windows", target_os = "linux")
                ))]
                if let (Some(audio), Some(prepared)) = (audio.running_mut(), prepared_audio) {
                    audio.rollback_rebind(prepared).map_err(|error| {
                        anyhow::anyhow!(
                            "rollback target host-audio after media rebind race: {error}"
                        )
                    })?;
                }
                continue;
            }
            encoder = next_encoder;
            active_capture_subject = pending_binding.diagnostic_capture_subject().clone();
            controller.commit_media_rebind(&pending_binding);
            #[cfg(all(
                feature = "native-media",
                any(target_os = "windows", target_os = "linux")
            ))]
            {
                if let (Some(audio), Some(prepared)) = (audio.running_mut(), prepared_audio) {
                    audio.commit_rebind(prepared);
                }
            }
        }
        let observed_at = Instant::now();
        controller.observe_polling_frame(observed_at);
        let (timestamp_ms, sample_duration) = controller.presentation(observed_at);
        let writer_started_at = Instant::now();
        let written = write_h264_sample(
            track,
            ssrc,
            payload_type,
            &mut encoder,
            &frame,
            timestamp_ms,
            sample_duration,
        )
        .await?;
        if written.is_some() && !media_ready_reported {
            execution.mark_media_ready();
            media_ready_reported = true;
        }
        controller.record_encoded(
            written,
            observed_at,
            writer_started_at.elapsed(),
            frame.width as usize,
            frame.height as usize,
        );
        let audio_stats = pump_baseline_audio(&mut audio);
        let audio_error = terminal_audio_failure(&audio, &audio_stats);
        controller
            .adapt_and_record(
                execution,
                video_sender,
                &mut rtcp_receiver,
                &mut encoder,
                config,
                audio_stats,
                false,
                false,
            )
            .await;
        if let Some(error) = audio_error {
            terminal_audio_error = Some(error);
            break;
        }
        let interval = frame_interval(controller.effective_fps);
        if let Some(remaining) = interval.checked_sub(capture_started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
    let audio_stats = pump_baseline_audio(&mut audio);
    terminal_audio_error =
        terminal_audio_error.or_else(|| terminal_audio_failure(&audio, &audio_stats));
    controller
        .adapt_and_record(
            execution,
            video_sender,
            &mut rtcp_receiver,
            &mut encoder,
            config,
            audio_stats,
            true,
            true,
        )
        .await;
    #[cfg(all(
        feature = "native-media",
        any(target_os = "windows", target_os = "linux")
    ))]
    audio.shutdown_discard().await;
    match terminal_audio_error {
        Some(error) => anyhow::bail!("negotiated host-audio pipeline failed: {error}"),
        None => Ok(()),
    }
}

fn frame_interval(fps: u32) -> Duration {
    Duration::from_secs_f64(1.0 / fps.max(1) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negotiated_audio_never_degrades_to_not_negotiated_when_backend_is_unavailable() {
        let state = BaselineAudioState::NegotiatedButBackendUnavailable {
            reason: "injected unavailable backend".to_string(),
        };
        let mut state = state;
        let stats = pump_baseline_audio(&mut state);
        assert!(stats.negotiated);
        assert!(!stats.backend_available);
        assert_eq!(
            stats.blocker.as_deref(),
            Some("injected unavailable backend")
        );
        assert_eq!(terminal_audio_failure(&state, &stats), None);
    }

    use crate::daemon::ability::builtins::resources::media::screen_snapshot::RawRgbFrame;
    use crate::daemon::persistence::resources::{self, ResourcesFile};
    use crate::daemon::plugins::remote_desktop::media::encode::{
        build_direct_webrtc_h264_config, encode_openh264_frame,
    };
    use crate::daemon::plugins::remote_desktop::test_support::seed_xcap_display;

    fn test_config() -> BuiltinH264Config {
        let mut file = ResourcesFile::default();
        let ura = seed_xcap_display(&mut file, "baseline-adaptation-test-display");
        let entry = resources::lookup_by_ura(&file, &ura).expect("seeded display");
        build_direct_webrtc_h264_config(
            entry,
            &ScreenCaptureOptions {
                resolution: None,
                fps: 30,
                resize_mode: CaptureResizeMode::FitWithin,
                region: None,
            },
            6_000,
            2,
        )
        .expect("baseline test config")
    }

    fn test_controller(config: &BuiltinH264Config) -> BaselineMediaController {
        let now = Instant::now();
        BaselineMediaController {
            session_id: "session-test".to_string(),
            transport_epoch: 7,
            selected_resource_ura: "easynet:///r/test/resource/device.test/display.test"
                .to_string(),
            media_source_epoch: 3,
            media_pipeline_id: config.backend.backend_id(),
            bitrate: AdaptiveBitrateController::new(config.bitrate_kbps),
            receiver_pressure: ReceiverPressureTracker::default(),
            pending_reconfiguration: None,
            effective_fps: config.fps,
            pacer: BaselineFramePacer::new(config.fps),
            stream_started_at: now,
            generation_started_at: now,
            last_adaptation_at: now,
            last_written_at: None,
            received_frames: 0,
            stale_frames_dropped: 0,
            pacing_frames_dropped: 0,
            receiver_frames_dropped: 0,
            encoded_frames: 0,
            encoded_bytes: 0,
            encoder_reconfigurations: 0,
            adaptation_event_sequence: 0,
            active_keyframe_interval_frames: config.keyframe_interval_frames,
            last_width: 0,
            last_height: 0,
            latency: BaselineLatencyWindow::default(),
        }
    }

    #[test]
    fn recorder_pacer_drops_early_frames_without_catch_up_bursts() {
        let mut pacer = BaselineFramePacer::new(20);
        let start = Instant::now();
        assert!(pacer.admit(start));
        assert!(!pacer.admit(start + Duration::from_millis(20)));
        assert!(pacer.admit(start + Duration::from_millis(50)));
        assert!(!pacer.admit(start + Duration::from_millis(55)));
        assert!(pacer.admit(start + Duration::from_millis(200)));
        assert!(!pacer.admit(start + Duration::from_millis(201)));
    }

    #[test]
    fn pacer_reconfiguration_obeys_new_lower_fps_ceiling_immediately() {
        let mut pacer = BaselineFramePacer::new(60);
        let start = Instant::now();
        assert!(pacer.admit(start));
        pacer.set_fps(15);
        assert!(!pacer.admit(start + Duration::from_millis(1)));
        assert!(!pacer.admit(start + Duration::from_millis(17)));
        assert!(!pacer.admit(start + Duration::from_millis(50)));
        assert!(pacer.admit(start + Duration::from_millis(67)));
        assert!(!pacer.admit(start + Duration::from_millis(84)));
    }

    #[test]
    fn failed_encoder_reconfiguration_preserves_active_rate_state() {
        let config = test_config();
        let mut encoder = build_openh264_encoder(&config).expect("initial encoder");
        let mut controller = test_controller(&config);

        let outcome = controller.apply_proposal_with(
            &mut encoder,
            &config,
            4_800,
            24,
            ReceiverPressure {
                frames_dropped_delta: 1,
                ..ReceiverPressure::default()
            },
            1,
            123,
            |_| anyhow::bail!("injected encoder rejection"),
        );

        assert!(!outcome.applied);
        assert_eq!(outcome.event["event_type"], "encoder_reconfigure_failed");
        assert_eq!(outcome.event["detail"]["requested_bitrate_kbps"], 4_800);
        assert_eq!(outcome.event["detail"]["active_bitrate_kbps"], 6_000);
        assert_eq!(outcome.event["detail"]["reconfigure_attempt"], 1);
        assert_eq!(outcome.event["detail"]["reconfigure_exhausted"], false);
        assert_eq!(controller.bitrate.current_kbps, 6_000);
        assert_eq!(controller.effective_fps, config.fps);
        assert_eq!(controller.encoder_reconfigurations, 0);

        let frame = RawRgbFrame {
            rgb_bytes: vec![0x44; 16 * 16 * 3],
            width: 16,
            height: 16,
            native_width: 16,
            native_height: 16,
        };
        assert!(
            encode_openh264_frame(&mut encoder, &frame, 0, controller.effective_fps)
                .expect("the previous encoder remains usable after replacement failure")
                .len()
                > 0
        );
    }

    #[test]
    fn writer_service_can_reconfigure_fps_without_falsifying_bitrate_change() {
        let config = test_config();
        let mut encoder = build_openh264_encoder(&config).expect("initial encoder");
        let mut controller = test_controller(&config);
        let active_bitrate = controller.bitrate.current_kbps;

        let outcome = controller.apply_proposal_with(
            &mut encoder,
            &config,
            active_bitrate,
            10,
            ReceiverPressure::default(),
            1,
            123,
            build_openh264_encoder,
        );

        assert!(outcome.applied);
        assert_eq!(outcome.event["event_type"], "fps_reconfigure");
        assert_eq!(controller.bitrate.current_kbps, active_bitrate);
        assert_eq!(controller.effective_fps, 10);
    }

    #[test]
    fn encoder_reconfiguration_retry_budget_is_bounded() {
        let config = test_config();
        let mut encoder = build_openh264_encoder(&config).expect("initial encoder");
        let mut controller = test_controller(&config);
        let mut proposal = PendingEncoderReconfiguration {
            bitrate_kbps: 4_800,
            fps: 24,
            pressure: ReceiverPressure {
                frames_dropped_delta: 1,
                ..ReceiverPressure::default()
            },
            attempts: 0,
        };

        for attempt in 1..=ENCODER_RECONFIGURE_MAX_ATTEMPTS {
            let outcome = controller.apply_proposal_with(
                &mut encoder,
                &config,
                proposal.bitrate_kbps,
                proposal.fps,
                proposal.pressure,
                proposal.attempts.saturating_add(1),
                120 + u64::from(attempt),
                |_| anyhow::bail!("injected persistent rejection"),
            );
            assert_eq!(
                outcome.event["detail"]["reconfigure_exhausted"],
                attempt == ENCODER_RECONFIGURE_MAX_ATTEMPTS
            );
            controller.retain_failed_proposal(proposal, outcome.applied);
            if attempt < ENCODER_RECONFIGURE_MAX_ATTEMPTS {
                proposal = controller
                    .pending_reconfiguration
                    .take()
                    .expect("retry remains pending inside budget");
                assert_eq!(proposal.attempts, attempt);
            }
        }

        assert!(controller.pending_reconfiguration.is_none());
        assert_eq!(controller.bitrate.current_kbps, config.bitrate_kbps);
        assert_eq!(controller.encoder_reconfigurations, 0);
    }

    #[test]
    fn transient_reconfiguration_failure_retries_then_commits_decoder_safe_encoder() {
        let config = test_config();
        let mut encoder = build_openh264_encoder(&config).expect("initial encoder");
        let mut controller = test_controller(&config);
        let proposal = PendingEncoderReconfiguration {
            bitrate_kbps: 4_800,
            fps: 24,
            pressure: ReceiverPressure {
                frames_dropped_delta: 1,
                ..ReceiverPressure::default()
            },
            attempts: 0,
        };
        let failed = controller.apply_proposal_with(
            &mut encoder,
            &config,
            proposal.bitrate_kbps,
            proposal.fps,
            proposal.pressure,
            proposal.attempts.saturating_add(1),
            123,
            |_| anyhow::bail!("injected transient rejection"),
        );
        controller.retain_failed_proposal(proposal, failed.applied);
        let retry = controller
            .pending_reconfiguration
            .take()
            .expect("failed proposal retained for bounded retry");
        assert_eq!(retry.attempts, 1);

        let applied = controller.apply_proposal_with(
            &mut encoder,
            &config,
            retry.bitrate_kbps,
            retry.fps,
            retry.pressure,
            retry.attempts.saturating_add(1),
            124,
            build_openh264_encoder,
        );
        controller.retain_failed_proposal(retry, applied.applied);
        assert!(applied.applied);
        assert_eq!(controller.bitrate.current_kbps, 4_800);
        assert_eq!(controller.encoder_reconfigurations, 1);
        assert!(controller.pending_reconfiguration.is_none());
        let active_config = controller.active_encoder_config(&config);
        assert_eq!(active_config.bitrate_kbps, 4_800);
        assert_eq!(active_config.fps, controller.effective_fps);
        assert_eq!(
            active_config.keyframe_interval_frames,
            controller.active_keyframe_interval_frames
        );

        let frame = RawRgbFrame {
            rgb_bytes: vec![0x44; 16 * 16 * 3],
            width: 16,
            height: 16,
            native_width: 16,
            native_height: 16,
        };
        let access_unit = encode_openh264_frame(&mut encoder, &frame, 0, controller.effective_fps)
            .expect("replacement encoder emits first access unit");
        let nal_types = annex_b_nal_types(&access_unit);
        assert!(nal_types.contains(&7), "first access unit must contain SPS");
        assert!(nal_types.contains(&8), "first access unit must contain PPS");
        assert!(nal_types.contains(&5), "first access unit must contain IDR");
    }

    #[test]
    fn baseline_stats_and_events_use_canonical_product_evidence_contract() {
        let config = test_config();
        let mut controller = test_controller(&config);
        controller.encoded_frames = 3;
        controller.encoded_bytes = 30_000;
        controller.last_width = 1280;
        controller.last_height = 720;
        controller.latency.record(Duration::from_millis(5));
        controller
            .latency
            .record_writer_service(Duration::from_millis(73));
        let event = controller.event("bitrate_downshift", 123, json!({}));
        let stats = controller.stats(
            &config,
            124,
            vec![event.clone()],
            audio_stats_not_negotiated(),
            false,
        );

        assert_eq!(stats["contract"], MEDIA_PIPELINE_STATS_CONTRACT);
        assert_eq!(stats["video_transport"], WEBRTC_VIDEO_TRANSPORT);
        assert_eq!(stats["payload_content_type"], H264_ANNEX_B_CONTENT_TYPE);
        assert_eq!(stats["codec_negotiated"], true);
        assert_eq!(stats["frames_encoded"], 3);
        assert_eq!(stats["width"], 1280);
        assert_eq!(stats["height"], 720);
        assert_eq!(stats["latency_stats"]["rtp_writer_service"]["p95_ms"], 73.0);
        assert_eq!(stats["adaptation_algorithm"], "transport_feedback");
        assert_eq!(stats["host_audio_not_implemented"], false);
        assert_eq!(stats["audio_backend_available"], true);
        assert_eq!(event["media_pipeline_id"], config.backend.backend_id());
        assert_eq!(
            event["selected_resource_ura"],
            controller.selected_resource_ura
        );
    }

    #[test]
    fn media_rebind_starts_fresh_generation_evidence_without_resetting_rtp_clock() {
        let config = test_config();
        let mut controller = test_controller(&config);
        let stream_started_at = controller.stream_started_at;
        controller.received_frames = 9;
        controller.stale_frames_dropped = 2;
        controller.pacing_frames_dropped = 1;
        controller.receiver_frames_dropped = 3;
        controller.encoded_frames = 6;
        controller.encoded_bytes = 60_000;
        controller.encoder_reconfigurations = 2;
        controller.adaptation_event_sequence = 4;
        controller.last_width = 1280;
        controller.last_height = 720;
        controller.latency.record(Duration::from_millis(5));
        controller.pending_reconfiguration = Some(PendingEncoderReconfiguration {
            bitrate_kbps: 4_000,
            fps: 20,
            pressure: ReceiverPressure::default(),
            attempts: 1,
        });

        controller.begin_media_generation("easynet:///r/test/resource/application.next", 4);

        assert_eq!(controller.stream_started_at, stream_started_at);
        assert_eq!(controller.media_source_epoch, 4);
        assert_eq!(
            controller.selected_resource_ura,
            "easynet:///r/test/resource/application.next"
        );
        assert_eq!(controller.received_frames, 0);
        assert_eq!(controller.stale_frames_dropped, 0);
        assert_eq!(controller.pacing_frames_dropped, 0);
        assert_eq!(controller.receiver_frames_dropped, 0);
        assert_eq!(controller.encoded_frames, 0);
        assert_eq!(controller.encoded_bytes, 0);
        assert_eq!(controller.encoder_reconfigurations, 0);
        assert_eq!(
            controller.adaptation_event_sequence, 4,
            "event ordering remains transport-monotonic across media generations"
        );
        assert_eq!(controller.last_width, 0);
        assert_eq!(controller.last_height, 0);
        assert!(controller.latency.encode_to_write_samples_ms.is_empty());
        assert!(controller.latency.writer_service_samples_ms.is_empty());
        assert!(controller.pending_reconfiguration.is_none());
    }

    fn annex_b_nal_types(bytes: &[u8]) -> Vec<u8> {
        let mut result = Vec::new();
        let mut index = 0;
        while index + 3 < bytes.len() {
            let start_len = if bytes[index..].starts_with(&[0, 0, 0, 1]) {
                4
            } else if bytes[index..].starts_with(&[0, 0, 1]) {
                3
            } else {
                index += 1;
                continue;
            };
            let nal_index = index + start_len;
            if nal_index < bytes.len() {
                result.push(bytes[nal_index] & 0x1f);
            }
            index = nal_index.saturating_add(1);
        }
        result
    }
}
