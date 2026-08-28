// EasyNet remoteapp decoded-frame receiver
// =======================================
//
// This host-side verifier is intentionally an example/tool, not a daemon API.
// It completes the SPEC host proof loop:
//
//   create_session JSON -> WebRTC offer -> remote_desktop.set_description
//   -> H.264 RTP receive -> OpenH264 decode -> pixel sentinel assertions
//   -> EASYNET_REMOTEAPP_FRAME_ANALYSIS_JSON
//
// The selected Resource URA, session token, and consent receipt stay inside
// the existing EasyNet CLI hidden binding. This tool never constructs Axon
// Invocation fields itself.

#![cfg(feature = "remote-desktop")]

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use openh264::decoder::{Decoder as H264Decoder, DecoderConfig, Flush};
use openh264::formats::YUVSource;
use openh264::OpenH264API;
use opus::{Channels as OpusChannels, Decoder as OpusDecoder};
use rtc::interceptor::Registry;
use rtc::media::io::h26x_writer::H26xWriter;
use rtc::media::io::Writer as H26xRtpWriter;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::{
    MediaEngine, MIME_TYPE_H264, MIME_TYPE_OPUS,
};
use rtc::peer_connection::configuration::RTCConfigurationBuilder;
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::rtp_transceiver::rtp_sender::{RTCRtpCodec, RTCRtpCodecParameters, RtpCodecKind};
use rtc::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
use serde::Serialize;
use serde_json::{json, Value};
use webrtc::data_channel::{DataChannel, RTCDataChannelState};
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
    RTCPeerConnectionState,
};
use webrtc::runtime::{channel, default_runtime, Runtime, Sender};

const H264_PAYLOAD_TYPE: u8 = 102;
const H264_CLOCK_RATE: u32 = 90_000;
const OPUS_PAYLOAD_TYPE: u8 = 111;
const OPUS_CLOCK_RATE: u32 = 48_000;
const OPUS_CHANNELS: usize = 2;
const MAX_OPUS_SAMPLES_PER_CHANNEL: usize = 5_760;
const MAX_AUDIO_ANALYSIS_SAMPLES: usize = OPUS_CLOCK_RATE as usize;
const ICE_GATHER_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_TIMEOUT_MS: u64 = 15_000;
// This verifier asserts decoded WebRTC media, not a raw screenshot. Hardware
// H.264 encode/decode and YUV/RGB conversion can move saturated fixture colors
// by more than 32 levels on secondary channels while preserving unambiguous
// target identity. Keep the default strict enough to reject the green unrelated
// sentinel, but tolerant enough for VideoToolbox's decoded red fixture.
const DEFAULT_TOLERANCE: u8 = 64;
const DEFAULT_MIN_SELECTED_PIXELS: usize = 8;
const MAX_DECODE_ERRORS_BEFORE_FAILURE_OBSERVATION: usize = 64;
const INPUT_DATA_CHANNEL_LABEL: &str = "easynet.remote_desktop.input.v1";
const INPUT_CHANNEL_OPEN_TIMEOUT_MS: u64 = 5_000;
const INPUT_SETTLE_MS: u64 = 750;
const DEFAULT_AUDIO_FREQUENCY_TOLERANCE_HZ: f64 = 25.0;
const DEFAULT_AUDIO_MIN_RMS: f64 = 0.01;
const DEFAULT_AUDIO_MIN_PACKETS: usize = 8;
const DEFAULT_AUDIO_SELECTED_POWER_RATIO: f64 = 4.0;
const MIN_AUDIO_SELECTED_POWER_FRACTION: f64 = 0.05;
const AUDIO_DOMINANT_SCAN_MIN_HZ: f64 = 80.0;
const AUDIO_DOMINANT_SCAN_MAX_HZ: f64 = 2_000.0;
const AUDIO_DOMINANT_SCAN_STEP_HZ: f64 = 2.0;

fn main() -> Result<()> {
    let runtime = default_runtime().ok_or_else(|| anyhow!("no WebRTC async runtime available"))?;
    let result_slot = Arc::new(Mutex::new(None));
    let result_writer = Arc::clone(&result_slot);
    runtime.block_on(Box::pin(async move {
        let result = async_main().await;
        *result_writer
            .lock()
            .expect("receiver result mutex poisoned") = Some(result);
    }));
    let result = result_slot
        .lock()
        .expect("receiver result mutex poisoned")
        .take()
        .unwrap_or_else(|| Err(anyhow!("receiver runtime returned without a result")));
    result
}

async fn async_main() -> Result<()> {
    let config = ReceiverConfig::from_env()?;
    let result = run_receiver(&config).await;
    match result {
        Ok(observation) => {
            if observation.assertions_satisfied(&config) {
                write_analysis(&config, AnalysisStatus::Passed, observation, None)?;
                Ok(())
            } else {
                let error = observation.failure_reason(&config);
                write_analysis(
                    &config,
                    AnalysisStatus::Failed,
                    observation,
                    Some(error.clone()),
                )?;
                Err(anyhow!(error))
            }
        }
        Err(error) => {
            let fallback = ReceiverObservation::failed();
            let _ = write_analysis(
                &config,
                AnalysisStatus::Failed,
                fallback,
                Some(error.to_string()),
            );
            Err(error)
        }
    }
}

#[derive(Clone)]
struct ReceiverConfig {
    session_json: PathBuf,
    frame_analysis_json: PathBuf,
    out_dir: PathBuf,
    easynet_bin: Option<PathBuf>,
    timeout: Duration,
    assertions: PixelAssertions,
    audio_assertions: Option<AudioAssertions>,
    session_artifact: SessionArtifactBinding,
    input_transmission_json: Option<PathBuf>,
}

impl ReceiverConfig {
    fn from_env() -> Result<Self> {
        let session_json = env_path("EASYNET_REMOTEAPP_SESSION_JSON")?;
        let frame_analysis_json = env_path("EASYNET_REMOTEAPP_FRAME_ANALYSIS_JSON")?;
        let out_dir = frame_analysis_json
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow!("EASYNET_REMOTEAPP_FRAME_ANALYSIS_JSON has no parent"))?;
        let easynet_bin = std::env::var_os("EASYNET_REMOTEAPP_EASYNET_BIN").map(PathBuf::from);
        let timeout = Duration::from_millis(env_u64(
            "EASYNET_REMOTEAPP_RECEIVER_TIMEOUT_MS",
            DEFAULT_TIMEOUT_MS,
        )?);
        let assertions = PixelAssertions::from_env()?;
        let audio_assertions = AudioAssertions::from_env()?;
        let session_artifact = SessionArtifactBinding::from_session_json(&session_json)?;
        let input_transmission_json = std::env::var_os("EASYNET_REMOTEAPP_INPUT_TRANSMISSION_JSON")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty());
        Ok(Self {
            session_json,
            frame_analysis_json,
            out_dir,
            easynet_bin,
            timeout,
            assertions,
            audio_assertions,
            session_artifact,
            input_transmission_json,
        })
    }
}

#[derive(Clone, Serialize)]
struct SessionArtifactBinding {
    session_id: String,
    subject_ura: String,
    binding_id: String,
    binding_epoch: u64,
    target_identity_epoch: u64,
    target_geometry_revision: u64,
    media_source_epoch: u64,
    consent_epoch: u64,
    capture_scope: String,
}

impl SessionArtifactBinding {
    fn from_session_json(path: &Path) -> Result<Self> {
        let value: Value = serde_json::from_slice(
            &fs::read(path).with_context(|| format!("read session JSON {}", path.display()))?,
        )
        .with_context(|| format!("parse session JSON {}", path.display()))?;
        Self::from_session_value(&value)
    }

    fn from_session_value(value: &Value) -> Result<Self> {
        let session = value.get("session").unwrap_or(&value);
        let target_binding = session.get("target_binding").and_then(Value::as_object);
        let session_id = session
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("session JSON missing session.session_id"))?
            .to_string();
        let target_binding =
            target_binding.ok_or_else(|| anyhow!("session JSON missing session.target_binding"))?;
        let binding_id = target_binding
            .get("binding_id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("session target_binding missing binding_id"))?
            .to_string();
        let subject_ura = target_binding
            .get("subject_ura")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("session target_binding missing subject_ura"))?
            .to_string();
        let binding_epoch = target_binding
            .get("binding_epoch")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or_else(|| anyhow!("session target_binding missing positive binding_epoch"))?;
        let target_identity_epoch = target_binding
            .get("target_identity_epoch")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                anyhow!("session target_binding missing positive target_identity_epoch")
            })?;
        let target_geometry_revision = target_binding
            .get("target_geometry_revision")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                anyhow!("session target_binding missing positive target_geometry_revision")
            })?;
        let media_source_epoch = target_binding
            .get("media_source_epoch")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or_else(|| anyhow!("session target_binding missing positive media_source_epoch"))?;
        let consent_epoch = target_binding
            .get("consent_epoch")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or_else(|| anyhow!("session target_binding missing positive consent_epoch"))?;
        let capture_scope = target_binding
            .get("capture_scope")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("session target_binding missing capture_scope"))?
            .to_string();
        Ok(Self {
            session_id,
            subject_ura,
            binding_id,
            binding_epoch,
            target_identity_epoch,
            target_geometry_revision,
            media_source_epoch,
            consent_epoch,
            capture_scope,
        })
    }
}

#[derive(Clone)]
struct PixelAssertions {
    selected: RgbAssertion,
    secondary_selected: Option<RgbAssertion>,
    unrelated: RgbAssertion,
}

impl PixelAssertions {
    fn from_env() -> Result<Self> {
        let selected = RgbAssertion {
            rgb: env_rgb("EASYNET_REMOTEAPP_SELECTED_SENTINEL_RGB")?,
            tolerance: env_u8("EASYNET_REMOTEAPP_SENTINEL_TOLERANCE", DEFAULT_TOLERANCE)?,
            min_pixels: env_usize(
                "EASYNET_REMOTEAPP_SELECTED_SENTINEL_MIN_PIXELS",
                DEFAULT_MIN_SELECTED_PIXELS,
            )?,
        };
        let unrelated = RgbAssertion {
            rgb: env_rgb("EASYNET_REMOTEAPP_UNRELATED_SENTINEL_RGB")?,
            tolerance: selected.tolerance,
            min_pixels: 1,
        };
        let secondary_selected =
            optional_env_rgb("EASYNET_REMOTEAPP_SELECTED_SECONDARY_SENTINEL_RGB")?.map(|rgb| {
                RgbAssertion {
                    rgb,
                    tolerance: selected.tolerance,
                    min_pixels: selected.min_pixels,
                }
            });
        Ok(Self {
            selected,
            secondary_selected,
            unrelated,
        })
    }
}

#[derive(Clone)]
struct AudioAssertions {
    expected_frequency_hz: f64,
    unrelated_frequency_hz: f64,
    frequency_tolerance_hz: f64,
    min_rms: f64,
    min_packets: usize,
    selected_power_ratio: f64,
}

impl AudioAssertions {
    fn from_env() -> Result<Option<Self>> {
        let Some(expected_frequency_hz) =
            optional_env_f64("EASYNET_REMOTEAPP_EXPECTED_AUDIO_FREQUENCY_HZ")?
        else {
            if optional_env_f64("EASYNET_REMOTEAPP_UNRELATED_AUDIO_FREQUENCY_HZ")?.is_some() {
                bail!(
                    "EASYNET_REMOTEAPP_UNRELATED_AUDIO_FREQUENCY_HZ requires EASYNET_REMOTEAPP_EXPECTED_AUDIO_FREQUENCY_HZ"
                );
            }
            return Ok(None);
        };
        let unrelated_frequency_hz =
            required_env_f64("EASYNET_REMOTEAPP_UNRELATED_AUDIO_FREQUENCY_HZ")?;
        let frequency_tolerance_hz = env_f64(
            "EASYNET_REMOTEAPP_AUDIO_FREQUENCY_TOLERANCE_HZ",
            DEFAULT_AUDIO_FREQUENCY_TOLERANCE_HZ,
        )?;
        let min_rms = env_f64("EASYNET_REMOTEAPP_AUDIO_MIN_RMS", DEFAULT_AUDIO_MIN_RMS)?;
        let min_packets = env_usize(
            "EASYNET_REMOTEAPP_AUDIO_MIN_PACKETS",
            DEFAULT_AUDIO_MIN_PACKETS,
        )?;
        let selected_power_ratio = env_f64(
            "EASYNET_REMOTEAPP_AUDIO_SELECTED_POWER_RATIO",
            DEFAULT_AUDIO_SELECTED_POWER_RATIO,
        )?;
        for (name, value) in [
            (
                "EASYNET_REMOTEAPP_EXPECTED_AUDIO_FREQUENCY_HZ",
                expected_frequency_hz,
            ),
            (
                "EASYNET_REMOTEAPP_UNRELATED_AUDIO_FREQUENCY_HZ",
                unrelated_frequency_hz,
            ),
            (
                "EASYNET_REMOTEAPP_AUDIO_FREQUENCY_TOLERANCE_HZ",
                frequency_tolerance_hz,
            ),
            ("EASYNET_REMOTEAPP_AUDIO_MIN_RMS", min_rms),
            (
                "EASYNET_REMOTEAPP_AUDIO_SELECTED_POWER_RATIO",
                selected_power_ratio,
            ),
        ] {
            if !value.is_finite() || value <= 0.0 {
                bail!("{name} must be finite and greater than zero");
            }
        }
        if min_packets == 0 {
            bail!("EASYNET_REMOTEAPP_AUDIO_MIN_PACKETS must be greater than zero");
        }
        if (expected_frequency_hz - unrelated_frequency_hz).abs() <= frequency_tolerance_hz * 2.0 {
            bail!(
                "selected and unrelated audio frequencies must be separated by more than twice the tolerance"
            );
        }
        if expected_frequency_hz >= OPUS_CLOCK_RATE as f64 / 2.0
            || unrelated_frequency_hz >= OPUS_CLOCK_RATE as f64 / 2.0
        {
            bail!("audio assertion frequencies must be below the 48 kHz Nyquist frequency");
        }
        Ok(Some(Self {
            expected_frequency_hz,
            unrelated_frequency_hz,
            frequency_tolerance_hz,
            min_rms,
            min_packets,
            selected_power_ratio,
        }))
    }
}

#[derive(Clone)]
struct RgbAssertion {
    rgb: [u8; 3],
    tolerance: u8,
    min_pixels: usize,
}

#[derive(Debug, Clone, Serialize)]
struct AudioObservation {
    rtp_packet_count: usize,
    encoded_byte_count: usize,
    decoded_packet_count: usize,
    decoded_samples_per_channel: usize,
    retained_analysis_samples: usize,
    decode_error_count: usize,
    last_decode_error: Option<String>,
    rms: f64,
    dominant_frequency_hz: Option<f64>,
    expected_frequency_hz: f64,
    unrelated_frequency_hz: f64,
    selected_frequency_power: f64,
    unrelated_frequency_power: f64,
    selected_power_ratio: f64,
    selected_power_fraction: f64,
    selected_tone_present: bool,
    unrelated_tone_rejected: bool,
}

impl AudioObservation {
    fn failed(assertions: &AudioAssertions) -> Self {
        Self {
            rtp_packet_count: 0,
            encoded_byte_count: 0,
            decoded_packet_count: 0,
            decoded_samples_per_channel: 0,
            retained_analysis_samples: 0,
            decode_error_count: 0,
            last_decode_error: None,
            rms: 0.0,
            dominant_frequency_hz: None,
            expected_frequency_hz: assertions.expected_frequency_hz,
            unrelated_frequency_hz: assertions.unrelated_frequency_hz,
            selected_frequency_power: 0.0,
            unrelated_frequency_power: 0.0,
            selected_power_ratio: 0.0,
            selected_power_fraction: 0.0,
            selected_tone_present: false,
            unrelated_tone_rejected: false,
        }
    }

    fn assertions_satisfied(&self, assertions: &AudioAssertions) -> bool {
        self.rtp_packet_count >= assertions.min_packets
            && self.decoded_packet_count >= assertions.min_packets
            && self.rms >= assertions.min_rms
            && self.dominant_frequency_hz.is_some_and(|frequency| {
                (frequency - assertions.expected_frequency_hz).abs()
                    <= assertions.frequency_tolerance_hz
            })
            && self.selected_power_ratio >= assertions.selected_power_ratio
            && self.selected_power_fraction >= MIN_AUDIO_SELECTED_POWER_FRACTION
            && self.selected_tone_present
            && self.unrelated_tone_rejected
    }

    fn failure_reason(&self, assertions: &AudioAssertions) -> String {
        if self.rtp_packet_count < assertions.min_packets {
            return format!(
                "remoteapp receiver observed only {} Opus RTP packets; required at least {}",
                self.rtp_packet_count, assertions.min_packets
            );
        }
        if self.decoded_packet_count < assertions.min_packets {
            return format!(
                "remoteapp receiver decoded only {} Opus packets; required at least {}; decode_errors={}{}",
                self.decoded_packet_count,
                assertions.min_packets,
                self.decode_error_count,
                self.last_decode_error
                    .as_ref()
                    .map(|error| format!("; last_decode_error={error}"))
                    .unwrap_or_default()
            );
        }
        if self.rms < assertions.min_rms {
            return format!(
                "remoteapp receiver decoded silent/near-silent audio: rms={} required={}",
                self.rms, assertions.min_rms
            );
        }
        if !self.selected_tone_present {
            return format!(
                "remoteapp decoded audio dominant frequency {:?} does not match selected {} Hz within {} Hz",
                self.dominant_frequency_hz,
                assertions.expected_frequency_hz,
                assertions.frequency_tolerance_hz
            );
        }
        format!(
            "remoteapp decoded audio did not isolate the selected application tone: selected_power={} unrelated_power={} ratio={} required_ratio={}",
            self.selected_frequency_power,
            self.unrelated_frequency_power,
            self.selected_power_ratio,
            assertions.selected_power_ratio
        )
    }
}

#[derive(Debug, Clone, Serialize)]
struct FrameObservation {
    count: usize,
    rtp_packet_count: usize,
    annexb_chunk_count: usize,
    annexb_byte_count: usize,
    dropped_access_unit_count: usize,
    decode_error_count: usize,
    last_decode_error: Option<String>,
    width: Option<usize>,
    height: Option<usize>,
    selected_content_present: bool,
    secondary_selected_content_present: Option<bool>,
    unrelated_sentinel_present: bool,
    full_display_leak_detected: bool,
    selected_pixel_count: usize,
    secondary_selected_pixel_count: Option<usize>,
    unrelated_pixel_count: usize,
    decoded_frame_sample: Option<PathBuf>,
}

impl FrameObservation {
    fn failed() -> Self {
        Self {
            count: 0,
            rtp_packet_count: 0,
            annexb_chunk_count: 0,
            annexb_byte_count: 0,
            dropped_access_unit_count: 0,
            decode_error_count: 0,
            last_decode_error: None,
            width: None,
            height: None,
            selected_content_present: false,
            secondary_selected_content_present: None,
            unrelated_sentinel_present: false,
            full_display_leak_detected: false,
            selected_pixel_count: 0,
            secondary_selected_pixel_count: None,
            unrelated_pixel_count: 0,
            decoded_frame_sample: None,
        }
    }

    fn assertions_satisfied(&self) -> bool {
        self.decode_error_count == 0
            && self.selected_content_present
            && self.secondary_selected_content_present.unwrap_or(true)
            && !self.unrelated_sentinel_present
    }

    fn failure_reason(&self) -> String {
        if self.rtp_packet_count == 0 {
            return "remoteapp receiver observed no H.264 RTP packets".to_string();
        }
        if self.count == 0 {
            return format!(
                "remoteapp receiver decoded no frames from {} RTP packets and {} Annex-B chunks; decode_errors={}{}",
                self.rtp_packet_count,
                self.annexb_chunk_count,
                self.decode_error_count,
                self.last_decode_error
                    .as_ref()
                    .map(|err| format!("; last_decode_error={err}"))
                    .unwrap_or_default()
            );
        }
        if self.decode_error_count > 0 {
            return format!(
                "remoteapp receiver observed {} H.264 decode error(s) before a valid frame{}",
                self.decode_error_count,
                self.last_decode_error
                    .as_ref()
                    .map(|err| format!("; last_decode_error={err}"))
                    .unwrap_or_default()
            );
        }
        if self.unrelated_sentinel_present {
            return format!(
                "remoteapp receiver detected unrelated sentinel content in decoded target frame; selected_pixels={}, unrelated_pixels={}",
                self.selected_pixel_count, self.unrelated_pixel_count
            );
        }
        if self.secondary_selected_content_present == Some(false) {
            return format!(
                "remoteapp receiver decoded {} frame(s) but the secondary selected surface was absent; secondary_selected_pixels={}",
                self.count,
                self.secondary_selected_pixel_count.unwrap_or(0)
            );
        }
        format!(
            "remoteapp receiver decoded {} frame(s) but selected sentinel was absent; selected_pixels={}, required selected_content_present=true",
            self.count, self.selected_pixel_count
        )
    }
}

#[derive(Debug, Clone)]
struct SignalingAnswer {
    answer: RTCSessionDescription,
    transport_epoch: u64,
}

#[derive(Debug, Clone)]
struct ReceiverObservation {
    session_view: Option<Value>,
    frame: FrameObservation,
    audio: Option<AudioObservation>,
}

impl ReceiverObservation {
    fn failed() -> Self {
        Self {
            session_view: None,
            frame: FrameObservation::failed(),
            audio: None,
        }
    }

    fn assertions_satisfied(&self, config: &ReceiverConfig) -> bool {
        self.frame.assertions_satisfied()
            && match &config.audio_assertions {
                Some(assertions) => self
                    .audio
                    .as_ref()
                    .is_some_and(|audio| audio.assertions_satisfied(assertions)),
                None => true,
            }
    }

    fn failure_reason(&self, config: &ReceiverConfig) -> String {
        if !self.frame.assertions_satisfied() {
            return self.frame.failure_reason();
        }
        match (&config.audio_assertions, &self.audio) {
            (Some(assertions), Some(audio)) => audio.failure_reason(assertions),
            (Some(_), None) => {
                "remoteapp receiver negotiated audio but observed no Opus track evidence"
                    .to_string()
            }
            (None, _) => self.frame.failure_reason(),
        }
    }
}

enum AnalysisStatus {
    Passed,
    Failed,
}

impl AnalysisStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone)]
struct Handler {
    runtime: Arc<dyn Runtime>,
    gather_complete_tx: Sender<()>,
    done_tx: Sender<()>,
    observation_tx: mpsc::Sender<FrameObservation>,
    audio_observation_tx: mpsc::Sender<AudioObservation>,
    audio_assertions: Option<AudioAssertions>,
    assertions: PixelAssertions,
    sample_path: PathBuf,
}

#[async_trait]
impl PeerConnectionEventHandler for Handler {
    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        if state == RTCIceGatheringState::Complete {
            let _ = self.gather_complete_tx.try_send(());
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        match state {
            RTCPeerConnectionState::Failed | RTCPeerConnectionState::Closed => {
                let _ = self.done_tx.try_send(());
            }
            _ => {}
        }
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let kind = track.kind().await;
        if kind == RtpCodecKind::Audio {
            let Some(assertions) = self.audio_assertions.clone() else {
                return;
            };
            let observation_tx = self.audio_observation_tx.clone();
            self.runtime.spawn(Box::pin(async move {
                if let Err(error) =
                    receive_decode_and_assert_audio(track, assertions, observation_tx).await
                {
                    eprintln!("easynet remoteapp receiver audio track failed: {error:#}");
                }
            }));
            return;
        }
        if kind != RtpCodecKind::Video {
            return;
        }
        let media_ssrc = match track.ssrcs().await.first().copied() {
            Some(ssrc) => ssrc,
            None => return,
        };
        let pli_track = Arc::clone(&track);
        let runtime = Arc::clone(&self.runtime);
        self.runtime.spawn(Box::pin(async move {
            loop {
                runtime.sleep(Duration::from_secs(3)).await;
                if pli_track
                    .write_rtcp(vec![Box::new(PictureLossIndication {
                        sender_ssrc: 0,
                        media_ssrc,
                    })])
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }));

        let observation_tx = self.observation_tx.clone();
        let assertions = self.assertions.clone();
        let sample_path = self.sample_path.clone();
        self.runtime.spawn(Box::pin(async move {
            if let Err(error) =
                receive_decode_and_assert(track, assertions, sample_path, observation_tx).await
            {
                eprintln!("easynet remoteapp receiver track failed: {error:#}");
            }
        }));
    }
}

async fn run_receiver(config: &ReceiverConfig) -> Result<ReceiverObservation> {
    let mut media_engine = MediaEngine::default();
    media_engine.register_codec(
        RTCRtpCodecParameters {
            rtp_codec: RTCRtpCodec {
                mime_type: MIME_TYPE_H264.to_owned(),
                clock_rate: H264_CLOCK_RATE,
                channels: 0,
                sdp_fmtp_line:
                    "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f"
                        .to_owned(),
                rtcp_feedback: vec![],
            },
            payload_type: H264_PAYLOAD_TYPE,
            ..Default::default()
        },
        RtpCodecKind::Video,
    )?;
    if config.audio_assertions.is_some() {
        media_engine.register_codec(
            RTCRtpCodecParameters {
                rtp_codec: RTCRtpCodec {
                    mime_type: MIME_TYPE_OPUS.to_owned(),
                    clock_rate: OPUS_CLOCK_RATE,
                    channels: OPUS_CHANNELS as u16,
                    sdp_fmtp_line: "minptime=10;useinbandfec=1".to_owned(),
                    rtcp_feedback: vec![],
                },
                payload_type: OPUS_PAYLOAD_TYPE,
                ..Default::default()
            },
            RtpCodecKind::Audio,
        )?;
    }
    let registry = register_default_interceptors(Registry::new(), &mut media_engine)?;
    let runtime = default_runtime().ok_or_else(|| anyhow!("no WebRTC async runtime available"))?;
    let runtime_for_builder = Arc::clone(&runtime);
    let (gather_complete_tx, mut gather_complete_rx) = channel::<()>(1);
    let (done_tx, mut done_rx) = channel::<()>(1);
    let (observation_tx, observation_rx) = mpsc::channel::<FrameObservation>();
    let (audio_observation_tx, audio_observation_rx) = mpsc::channel::<AudioObservation>();
    let sample_path = config.out_dir.join("decoded-frame-sample.ppm");
    let handler = Arc::new(Handler {
        runtime: Arc::clone(&runtime),
        gather_complete_tx,
        done_tx,
        observation_tx,
        audio_observation_tx,
        audio_assertions: config.audio_assertions.clone(),
        assertions: config.assertions.clone(),
        sample_path,
    });

    let peer_connection = PeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .with_media_engine(media_engine)
        .with_interceptor_registry(registry)
        .with_handler(handler as Arc<dyn PeerConnectionEventHandler>)
        .with_runtime(runtime_for_builder)
        .with_udp_addrs(vec!["127.0.0.1:0".to_string()])
        .build()
        .await?;

    peer_connection
        .add_transceiver_from_kind(
            RtpCodecKind::Video,
            Some(RTCRtpTransceiverInit {
                direction: RTCRtpTransceiverDirection::Recvonly,
                ..Default::default()
            }),
        )
        .await?;

    if config.audio_assertions.is_some() {
        peer_connection
            .add_transceiver_from_kind(
                RtpCodecKind::Audio,
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Recvonly,
                    ..Default::default()
                }),
            )
            .await?;
    }

    let input_channel = if config.input_transmission_json.is_some() {
        Some(
            peer_connection
                .create_data_channel(INPUT_DATA_CHANNEL_LABEL, None)
                .await
                .context("create canonical RemoteApp input data channel")?,
        )
    } else {
        None
    };

    let offer = peer_connection.create_offer(None).await?;
    peer_connection.set_local_description(offer).await?;
    wait_for_local_ice_gathering(runtime.as_ref(), &mut gather_complete_rx).await;
    let offer = peer_connection
        .local_description()
        .await
        .ok_or_else(|| anyhow!("local WebRTC offer missing"))?;
    let signal = signal_offer(config, &offer).context("invoke remote_desktop.set_description")?;
    peer_connection
        .set_remote_description(signal.answer.clone())
        .await?;

    let started = Instant::now();
    let mut latest_observation: Option<ReceiverObservation> = None;
    loop {
        while let Ok(audio) = audio_observation_rx.try_recv() {
            match latest_observation.as_mut() {
                Some(observation) => observation.audio = Some(audio),
                None => {
                    latest_observation = Some(ReceiverObservation {
                        session_view: None,
                        frame: FrameObservation::failed(),
                        audio: Some(audio),
                    });
                }
            }
        }
        match observation_rx.try_recv() {
            Ok(observation) => {
                let audio = latest_observation
                    .as_ref()
                    .and_then(|latest| latest.audio.clone());
                latest_observation = Some(ReceiverObservation {
                    session_view: None,
                    frame: observation,
                    audio,
                });
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                if let Some(mut observation) = latest_observation.take() {
                    observation.session_view = show_session_view(config).ok();
                    peer_connection.close().await?;
                    return Ok(observation);
                }
                bail!("decoded frame observation channel closed before assertions passed");
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if latest_observation
            .as_ref()
            .is_some_and(|observation| observation.assertions_satisfied(config))
        {
            report_client_presenting(config, signal.transport_epoch)
                .context("invoke remote_desktop.report_client_state after decoded media")?;
            if let Some(input_channel) = input_channel.as_ref() {
                exercise_target_local_input(
                    config,
                    Arc::clone(input_channel),
                    signal.transport_epoch,
                    runtime.as_ref(),
                )
                .await
                .context("exercise target-local input through WebRTC data channel")?;
            }
            let latest_session_view = show_session_view(config)
                .context("invoke remote_desktop.show_session after receiver proof")
                .ok();
            let mut observation = latest_observation
                .take()
                .expect("checked receiver observation exists");
            observation.session_view = latest_session_view;
            peer_connection.close().await?;
            return Ok(observation);
        }
        if started.elapsed() > config.timeout {
            if let Some(mut observation) = latest_observation.take() {
                observation.session_view = show_session_view(config).ok();
                peer_connection.close().await?;
                return Ok(observation);
            }
            peer_connection.close().await?;
            bail!(
                "timed out after {} ms waiting for decoded remoteapp frame assertions",
                config.timeout.as_millis()
            );
        }
        if done_rx.try_recv().is_ok() {
            peer_connection.close().await?;
            bail!("WebRTC peer connection closed before decoded frame assertions passed");
        }
        runtime.sleep(Duration::from_millis(100)).await;
    }
}

async fn exercise_target_local_input(
    config: &ReceiverConfig,
    input_channel: Arc<dyn DataChannel>,
    transport_epoch: u64,
    runtime: &dyn Runtime,
) -> Result<()> {
    let input_path = config
        .input_transmission_json
        .as_ref()
        .ok_or_else(|| anyhow!("input transmission artifact path missing"))?;
    let session_view = show_session_view(config)
        .context("read current session input policy before input transmission")?;
    let policy = session_view
        .get("input_policy")
        .ok_or_else(|| anyhow!("session view missing input_policy"))?;
    let readiness = session_view
        .get("input_readiness")
        .ok_or_else(|| anyhow!("session view missing input_readiness"))?;
    if readiness.get("interactive_ready") != Some(&Value::Bool(true)) {
        bail!(
            "session input is not interactively ready: {}",
            readiness
                .get("blocked_reason")
                .cloned()
                .unwrap_or(Value::Null)
        );
    }
    if policy.get("input_scope").and_then(Value::as_str) != Some("target_local") {
        bail!("live target input proof requires input_policy.input_scope=target_local");
    }
    if policy.get("pointer_enabled") != Some(&Value::Bool(true))
        || policy.get("keyboard_enabled") != Some(&Value::Bool(true))
    {
        bail!("live target input proof requires pointer and keyboard policy enablement");
    }
    let pointer_target = policy
        .get("pointer_target")
        .ok_or_else(|| anyhow!("target-local input policy missing pointer_target"))?;
    let target_geometry_revision = positive_u64(
        pointer_target.get("target_geometry_revision"),
        "input_policy.pointer_target.target_geometry_revision",
    )?;
    let target_focus_epoch = positive_u64(
        policy.get("target_focus_epoch"),
        "input_policy.target_focus_epoch",
    )?;
    let origin_x = finite_f64(pointer_target.get("origin_x"), "pointer_target.origin_x")?;
    let origin_y = finite_f64(pointer_target.get("origin_y"), "pointer_target.origin_y")?;
    let width = positive_f64(pointer_target.get("width"), "pointer_target.width")?;
    let height = positive_f64(pointer_target.get("height"), "pointer_target.height")?;

    wait_for_input_channel_open(runtime, input_channel.as_ref()).await?;

    let pointer_sent_at_ms = wall_clock_ms()?;
    let pointer_frame = json!({
        "type": "pointer",
        "action": "down",
        "normalized_x": 0.5,
        "normalized_y": 0.5,
        "button": 0,
        "target_geometry_revision": target_geometry_revision,
        "target_focus_epoch": target_focus_epoch,
        "sent_at_ms": pointer_sent_at_ms,
        "client_sequence": 1,
    });
    input_channel
        .send_text(&serde_json::to_string(&pointer_frame)?)
        .await
        .context("send target-local pointer-down frame")?;

    let pointer_up_frame = json!({
        "type": "pointer",
        "action": "up",
        "normalized_x": 0.5,
        "normalized_y": 0.5,
        "button": 0,
        "target_geometry_revision": target_geometry_revision,
        "target_focus_epoch": target_focus_epoch,
        "sent_at_ms": wall_clock_ms()?,
        "client_sequence": 2,
    });
    input_channel
        .send_text(&serde_json::to_string(&pointer_up_frame)?)
        .await
        .context("send target-local pointer-up frame")?;

    let key_down_sent_at_ms = wall_clock_ms()?;
    let key_down_frame = json!({
        "type": "key",
        "action": "down",
        "key": "a",
        "code": "KeyA",
        "repeat": false,
        "target_focus_epoch": target_focus_epoch,
        "sent_at_ms": key_down_sent_at_ms,
        "client_sequence": 3,
    });
    input_channel
        .send_text(&serde_json::to_string(&key_down_frame)?)
        .await
        .context("send target-local key-down frame")?;

    let key_up_frame = json!({
        "type": "key",
        "action": "up",
        "key": "a",
        "code": "KeyA",
        "repeat": false,
        "target_focus_epoch": target_focus_epoch,
        "sent_at_ms": wall_clock_ms()?,
        "client_sequence": 4,
    });
    input_channel
        .send_text(&serde_json::to_string(&key_up_frame)?)
        .await
        .context("send target-local key-up frame")?;

    let stale_frame = json!({
        "type": "pointer",
        "action": "down",
        "normalized_x": 0.5,
        "normalized_y": 0.5,
        "button": 0,
        "target_geometry_revision": target_geometry_revision,
        "target_focus_epoch": target_focus_epoch,
        "sent_at_ms": wall_clock_ms()?,
        "client_sequence": 1,
    });
    input_channel
        .send_text(&serde_json::to_string(&stale_frame)?)
        .await
        .context("send stale-sequence rejection probe")?;

    runtime.sleep(Duration::from_millis(INPUT_SETTLE_MS)).await;
    input_channel
        .close()
        .await
        .context("close input proof channel")?;
    runtime.sleep(Duration::from_millis(INPUT_SETTLE_MS)).await;
    let post_input_session_view =
        show_session_view(config).context("read session events after input transmission")?;
    let artifact = json!({
        "status": "passed",
        "channel": {
            "label": INPUT_DATA_CHANNEL_LABEL,
            "opened": true,
        },
        "session_id": config.session_artifact.session_id,
        "subject_ura": config.session_artifact.subject_ura,
        "transport_epoch": transport_epoch,
        "input_scope": "target_local",
        "target_geometry_revision": target_geometry_revision,
        "target_focus_epoch": target_focus_epoch,
        "expected_pointer_position": {
            "x": origin_x + width * 0.5,
            "y": origin_y + height * 0.5,
        },
        "frames": [pointer_frame, pointer_up_frame, key_down_frame, key_up_frame, stale_frame],
        "session_view_before_input": session_view,
        "session_view_after_input": post_input_session_view,
    });
    fs::write(input_path, serde_json::to_vec_pretty(&artifact)?)
        .with_context(|| format!("write input transmission JSON {}", input_path.display()))
}

async fn wait_for_input_channel_open(
    runtime: &dyn Runtime,
    channel: &dyn DataChannel,
) -> Result<()> {
    let started = Instant::now();
    loop {
        match channel.ready_state().await? {
            RTCDataChannelState::Open => return Ok(()),
            RTCDataChannelState::Closing | RTCDataChannelState::Closed => {
                bail!("RemoteApp input data channel closed before opening")
            }
            RTCDataChannelState::Connecting => {}
            _ => {}
        }
        if started.elapsed() >= Duration::from_millis(INPUT_CHANNEL_OPEN_TIMEOUT_MS) {
            bail!(
                "timed out after {} ms waiting for RemoteApp input data channel",
                INPUT_CHANNEL_OPEN_TIMEOUT_MS
            );
        }
        runtime.sleep(Duration::from_millis(25)).await;
    }
}

fn positive_u64(value: Option<&Value>, field: &str) -> Result<u64> {
    value
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| anyhow!("{field} must be a positive integer"))
}

fn finite_f64(value: Option<&Value>, field: &str) -> Result<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or_else(|| anyhow!("{field} must be finite"))
}

fn positive_f64(value: Option<&Value>, field: &str) -> Result<f64> {
    finite_f64(value, field).and_then(|value| {
        if value > 0.0 {
            Ok(value)
        } else {
            bail!("{field} must be positive")
        }
    })
}

fn wall_clock_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock precedes Unix epoch")?
        .as_millis()
        .try_into()
        .context("wall-clock millisecond timestamp exceeds u64")?)
}

async fn wait_for_local_ice_gathering(
    runtime: &dyn Runtime,
    gather_complete_rx: &mut webrtc::runtime::Receiver<()>,
) {
    let started = Instant::now();
    loop {
        if gather_complete_rx.try_recv().is_ok() {
            return;
        }
        if started.elapsed() >= Duration::from_millis(ICE_GATHER_TIMEOUT_MS) {
            eprintln!(
                "easynet remoteapp receiver ICE gathering did not complete within {} ms; continuing with current local description",
                ICE_GATHER_TIMEOUT_MS
            );
            return;
        }
        runtime.sleep(Duration::from_millis(25)).await;
    }
}

async fn receive_decode_and_assert(
    track: Arc<dyn TrackRemote>,
    assertions: PixelAssertions,
    sample_path: PathBuf,
    observation_tx: mpsc::Sender<FrameObservation>,
) -> Result<()> {
    let mut access_units = H264AccessUnitAssembler::new();
    let mut decoder = new_stream_decoder()?;
    let mut decoded_count = 0usize;
    let mut rtp_packet_count = 0usize;
    let mut annexb_chunk_count = 0usize;
    let mut annexb_byte_count = 0usize;
    let mut decode_error_count = 0usize;
    let mut last_decode_error: Option<String> = None;
    let mut best_selected_pixels = 0usize;
    let mut best_secondary_selected_pixels = 0usize;
    let mut best_unrelated_pixels = 0usize;
    while let Some(event) = track.poll().await {
        let TrackRemoteEvent::OnRtpPacket(packet) = event else {
            continue;
        };
        rtp_packet_count += 1;
        let output = access_units.output();
        let Some(access_unit) = access_units.push(
            packet.header.timestamp,
            packet.header.sequence_number,
            packet.header.marker,
            |writer| {
                writer.write_rtp(&packet)?;
                Ok(output.drain())
            },
        )?
        else {
            continue;
        };
        annexb_chunk_count += 1;
        annexb_byte_count += access_unit.len();
        let decoded = match decoder.decode(&access_unit) {
            Ok(Some(decoded)) => decoded,
            Ok(None) => continue,
            Err(error) => {
                decode_error_count += 1;
                last_decode_error = Some(error.to_string());
                eprintln!(
                    "easynet remoteapp receiver H.264 access-unit decode error #{decode_error_count}: {error}; access_unit_bytes={}; nal_types={:?}",
                    access_unit.len(),
                    annexb_nal_types(&access_unit)
                );
                let _ = observation_tx.send(FrameObservation {
                    count: decoded_count,
                    rtp_packet_count,
                    annexb_chunk_count,
                    annexb_byte_count,
                    dropped_access_unit_count: access_units.dropped_count(),
                    decode_error_count,
                    last_decode_error: last_decode_error.clone(),
                    width: None,
                    height: None,
                    selected_content_present: false,
                    secondary_selected_content_present: secondary_selected_presence(
                        &assertions,
                        best_secondary_selected_pixels,
                    ),
                    unrelated_sentinel_present: best_unrelated_pixels
                        >= assertions.unrelated.min_pixels,
                    full_display_leak_detected: best_unrelated_pixels
                        >= assertions.unrelated.min_pixels,
                    selected_pixel_count: best_selected_pixels,
                    secondary_selected_pixel_count: assertions
                        .secondary_selected
                        .as_ref()
                        .map(|_| best_secondary_selected_pixels),
                    unrelated_pixel_count: best_unrelated_pixels,
                    decoded_frame_sample: None,
                });
                if decode_error_count >= MAX_DECODE_ERRORS_BEFORE_FAILURE_OBSERVATION {
                    let _ = observation_tx.send(FrameObservation {
                        count: decoded_count,
                        rtp_packet_count,
                        annexb_chunk_count,
                        annexb_byte_count,
                        dropped_access_unit_count: access_units.dropped_count(),
                        decode_error_count,
                        last_decode_error,
                        width: None,
                        height: None,
                        selected_content_present: false,
                        secondary_selected_content_present: secondary_selected_presence(
                            &assertions,
                            best_secondary_selected_pixels,
                        ),
                        unrelated_sentinel_present: best_unrelated_pixels
                            >= assertions.unrelated.min_pixels,
                        full_display_leak_detected: best_unrelated_pixels
                            >= assertions.unrelated.min_pixels,
                        selected_pixel_count: best_selected_pixels,
                        secondary_selected_pixel_count: assertions
                            .secondary_selected
                            .as_ref()
                            .map(|_| best_secondary_selected_pixels),
                        unrelated_pixel_count: best_unrelated_pixels,
                        decoded_frame_sample: None,
                    });
                    return Ok(());
                }
                continue;
            }
        };
        let (width, height) = decoded.dimensions();
        let mut rgb = vec![0u8; width * height * 3];
        decoded.write_rgb8(&mut rgb);
        decoded_count += 1;

        let selected_pixels =
            count_rgb_matches(&rgb, assertions.selected.rgb, assertions.selected.tolerance);
        let unrelated_pixels = count_rgb_matches(
            &rgb,
            assertions.unrelated.rgb,
            assertions.unrelated.tolerance,
        );
        let secondary_selected_pixels = assertions
            .secondary_selected
            .as_ref()
            .map(|secondary| count_rgb_matches(&rgb, secondary.rgb, secondary.tolerance));
        best_selected_pixels = best_selected_pixels.max(selected_pixels);
        best_secondary_selected_pixels =
            best_secondary_selected_pixels.max(secondary_selected_pixels.unwrap_or(0));
        best_unrelated_pixels = best_unrelated_pixels.max(unrelated_pixels);
        write_ppm(&sample_path, width, height, &rgb)?;

        let selected_content_present = selected_pixels >= assertions.selected.min_pixels;
        let secondary_selected_content_present = assertions
            .secondary_selected
            .as_ref()
            .zip(secondary_selected_pixels)
            .map(|(secondary, pixels)| pixels >= secondary.min_pixels);
        let unrelated_sentinel_present = unrelated_pixels >= assertions.unrelated.min_pixels;
        let observation = FrameObservation {
            count: decoded_count,
            rtp_packet_count,
            annexb_chunk_count,
            annexb_byte_count,
            dropped_access_unit_count: access_units.dropped_count(),
            decode_error_count,
            last_decode_error: last_decode_error.clone(),
            width: Some(width),
            height: Some(height),
            selected_content_present,
            secondary_selected_content_present,
            unrelated_sentinel_present,
            full_display_leak_detected: unrelated_sentinel_present,
            selected_pixel_count: selected_pixels,
            secondary_selected_pixel_count: secondary_selected_pixels,
            unrelated_pixel_count: unrelated_pixels,
            decoded_frame_sample: Some(sample_path.clone()),
        };
        let _ = observation_tx.send(observation.clone());
        if observation.assertions_satisfied() {
            return Ok(());
        }
    }
    let _ = observation_tx.send(FrameObservation {
        count: decoded_count,
        rtp_packet_count,
        annexb_chunk_count,
        annexb_byte_count,
        dropped_access_unit_count: access_units.dropped_count(),
        decode_error_count,
        last_decode_error,
        width: None,
        height: None,
        selected_content_present: false,
        secondary_selected_content_present: secondary_selected_presence(
            &assertions,
            best_secondary_selected_pixels,
        ),
        unrelated_sentinel_present: best_unrelated_pixels >= assertions.unrelated.min_pixels,
        full_display_leak_detected: best_unrelated_pixels >= assertions.unrelated.min_pixels,
        selected_pixel_count: best_selected_pixels,
        secondary_selected_pixel_count: assertions
            .secondary_selected
            .as_ref()
            .map(|_| best_secondary_selected_pixels),
        unrelated_pixel_count: best_unrelated_pixels,
        decoded_frame_sample: None,
    });
    Ok(())
}

async fn receive_decode_and_assert_audio(
    track: Arc<dyn TrackRemote>,
    assertions: AudioAssertions,
    observation_tx: mpsc::Sender<AudioObservation>,
) -> Result<()> {
    let mut decoder = OpusDecoder::new(OPUS_CLOCK_RATE, OpusChannels::Stereo)?;
    let mut decode_buffer = vec![0.0_f32; MAX_OPUS_SAMPLES_PER_CHANNEL * OPUS_CHANNELS];
    let mut analysis_samples = Vec::with_capacity(MAX_AUDIO_ANALYSIS_SAMPLES);
    let mut rtp_packet_count = 0usize;
    let mut encoded_byte_count = 0usize;
    let mut decoded_packet_count = 0usize;
    let mut decoded_samples_per_channel = 0usize;
    let mut decode_error_count = 0usize;
    let mut last_decode_error = None;

    while let Some(event) = track.poll().await {
        let TrackRemoteEvent::OnRtpPacket(packet) = event else {
            continue;
        };
        rtp_packet_count = rtp_packet_count.saturating_add(1);
        encoded_byte_count = encoded_byte_count.saturating_add(packet.payload.len());
        let samples_per_channel =
            match decoder.decode_float(packet.payload.as_ref(), &mut decode_buffer, false) {
                Ok(samples) => samples,
                Err(error) => {
                    decode_error_count = decode_error_count.saturating_add(1);
                    last_decode_error = Some(error.to_string());
                    let observation = analyze_audio_samples(
                        &analysis_samples,
                        &assertions,
                        AudioDecodeCounters {
                            rtp_packet_count,
                            encoded_byte_count,
                            decoded_packet_count,
                            decoded_samples_per_channel,
                            decode_error_count,
                            last_decode_error: last_decode_error.clone(),
                        },
                    );
                    let _ = observation_tx.send(observation);
                    continue;
                }
            };
        decoded_packet_count = decoded_packet_count.saturating_add(1);
        decoded_samples_per_channel =
            decoded_samples_per_channel.saturating_add(samples_per_channel);
        let remaining = MAX_AUDIO_ANALYSIS_SAMPLES.saturating_sub(analysis_samples.len());
        for frame in 0..samples_per_channel.min(remaining) {
            let offset = frame * OPUS_CHANNELS;
            let mono = (decode_buffer[offset] + decode_buffer[offset + 1]) * 0.5;
            analysis_samples.push(mono);
        }
        let observation = analyze_audio_samples(
            &analysis_samples,
            &assertions,
            AudioDecodeCounters {
                rtp_packet_count,
                encoded_byte_count,
                decoded_packet_count,
                decoded_samples_per_channel,
                decode_error_count,
                last_decode_error: last_decode_error.clone(),
            },
        );
        let passed = observation.assertions_satisfied(&assertions);
        if observation_tx.send(observation).is_err() || passed {
            return Ok(());
        }
    }

    let _ = observation_tx.send(analyze_audio_samples(
        &analysis_samples,
        &assertions,
        AudioDecodeCounters {
            rtp_packet_count,
            encoded_byte_count,
            decoded_packet_count,
            decoded_samples_per_channel,
            decode_error_count,
            last_decode_error,
        },
    ));
    Ok(())
}

struct AudioDecodeCounters {
    rtp_packet_count: usize,
    encoded_byte_count: usize,
    decoded_packet_count: usize,
    decoded_samples_per_channel: usize,
    decode_error_count: usize,
    last_decode_error: Option<String>,
}

fn analyze_audio_samples(
    samples: &[f32],
    assertions: &AudioAssertions,
    counters: AudioDecodeCounters,
) -> AudioObservation {
    if samples.is_empty() {
        let mut observation = AudioObservation::failed(assertions);
        observation.rtp_packet_count = counters.rtp_packet_count;
        observation.encoded_byte_count = counters.encoded_byte_count;
        observation.decoded_packet_count = counters.decoded_packet_count;
        observation.decoded_samples_per_channel = counters.decoded_samples_per_channel;
        observation.decode_error_count = counters.decode_error_count;
        observation.last_decode_error = counters.last_decode_error;
        return observation;
    }

    let rms = (samples
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt();
    let windowed_samples = hann_windowed(samples);
    let dominant_frequency_hz =
        (counters.decoded_packet_count >= assertions.min_packets).then(|| {
            dominant_goertzel_frequency(
                &windowed_samples,
                OPUS_CLOCK_RATE,
                AUDIO_DOMINANT_SCAN_MIN_HZ,
                AUDIO_DOMINANT_SCAN_MAX_HZ,
                AUDIO_DOMINANT_SCAN_STEP_HZ,
            )
        });
    let selected_frequency_power = goertzel_power(
        &windowed_samples,
        OPUS_CLOCK_RATE,
        assertions.expected_frequency_hz,
    );
    let unrelated_frequency_power = goertzel_power(
        &windowed_samples,
        OPUS_CLOCK_RATE,
        assertions.unrelated_frequency_hz,
    );
    let selected_power_ratio = selected_frequency_power / unrelated_frequency_power.max(1e-18);
    let selected_power_fraction = selected_frequency_power / rms.powi(2).max(1e-18);
    let selected_tone_present = dominant_frequency_hz.is_some_and(|frequency| {
        (frequency - assertions.expected_frequency_hz).abs() <= assertions.frequency_tolerance_hz
    }) && selected_power_fraction >= MIN_AUDIO_SELECTED_POWER_FRACTION;
    let unrelated_tone_rejected = selected_power_ratio >= assertions.selected_power_ratio;

    AudioObservation {
        rtp_packet_count: counters.rtp_packet_count,
        encoded_byte_count: counters.encoded_byte_count,
        decoded_packet_count: counters.decoded_packet_count,
        decoded_samples_per_channel: counters.decoded_samples_per_channel,
        retained_analysis_samples: samples.len(),
        decode_error_count: counters.decode_error_count,
        last_decode_error: counters.last_decode_error,
        rms,
        dominant_frequency_hz,
        expected_frequency_hz: assertions.expected_frequency_hz,
        unrelated_frequency_hz: assertions.unrelated_frequency_hz,
        selected_frequency_power,
        unrelated_frequency_power,
        selected_power_ratio,
        selected_power_fraction,
        selected_tone_present,
        unrelated_tone_rejected,
    }
}

fn dominant_goertzel_frequency(
    windowed_samples: &[f64],
    sample_rate_hz: u32,
    min_frequency_hz: f64,
    max_frequency_hz: f64,
    step_hz: f64,
) -> f64 {
    let mut frequency_hz = min_frequency_hz;
    let mut best_frequency_hz = min_frequency_hz;
    let mut best_power = f64::NEG_INFINITY;
    while frequency_hz <= max_frequency_hz {
        let power = goertzel_power(windowed_samples, sample_rate_hz, frequency_hz);
        if power > best_power {
            best_power = power;
            best_frequency_hz = frequency_hz;
        }
        frequency_hz += step_hz;
    }
    best_frequency_hz
}

fn hann_windowed(samples: &[f32]) -> Vec<f64> {
    let denominator = (samples.len().saturating_sub(1)).max(1) as f64;
    samples
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            let window = 0.5 - 0.5 * (std::f64::consts::TAU * index as f64 / denominator).cos();
            f64::from(*sample) * window
        })
        .collect()
}

fn goertzel_power(samples: &[f64], sample_rate_hz: u32, frequency_hz: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let omega = std::f64::consts::TAU * frequency_hz / sample_rate_hz as f64;
    let coefficient = 2.0 * omega.cos();
    let mut previous = 0.0_f64;
    let mut previous_two = 0.0_f64;
    for sample in samples {
        let current = *sample + coefficient * previous - previous_two;
        previous_two = previous;
        previous = current;
    }
    let power = previous_two.powi(2) + previous.powi(2) - coefficient * previous * previous_two;
    power.max(0.0) / (samples.len() as f64).powi(2)
}

fn signal_offer(config: &ReceiverConfig, offer: &RTCSessionDescription) -> Result<SignalingAnswer> {
    let offer_path = config.out_dir.join("remoteapp-receiver-offer.json");
    fs::write(&offer_path, serde_json::to_vec_pretty(offer)?)?;
    let output = easynet_command(config)
        .args([
            "ability",
            "set-remote-desktop-description",
            "--session-json",
        ])
        .arg(&config.session_json)
        .args(["--side", "remote", "--description-json-file"])
        .arg(&offer_path)
        .args(["--format", "json"])
        .output()
        .context("spawn easynet ability set-remote-desktop-description")?;
    if !output.status.success() {
        bail!(
            "easynet set-remote-desktop-description failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let response: Value = serde_json::from_slice(&output.stdout)
        .context("parse remote_desktop.set_description JSON response")?;
    let answer = response
        .get("signaling")
        .and_then(|signaling| signaling.get("local_description"))
        .cloned()
        .ok_or_else(|| {
            anyhow!("remote_desktop.set_description response missing signaling.local_description")
        })?;
    let answer = serde_json::from_value(answer)
        .context("parse WebRTC answer from signaling.local_description")?;
    let transport_epoch = response
        .get("transport_epoch")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            anyhow!("remote_desktop.set_description response missing positive transport_epoch")
        })?;
    Ok(SignalingAnswer {
        answer,
        transport_epoch,
    })
}

fn report_client_presenting(config: &ReceiverConfig, transport_epoch: u64) -> Result<Value> {
    let transport_epoch = transport_epoch.to_string();
    let output = easynet_command(config)
        .args([
            "ability",
            "report-remote-desktop-client-state",
            "--session-json",
        ])
        .arg(&config.session_json)
        .args([
            "--state",
            "presenting",
            "--transport-epoch",
            &transport_epoch,
            "--format",
            "json",
        ])
        .output()
        .context("spawn easynet ability report-remote-desktop-client-state")?;
    if !output.status.success() {
        bail!(
            "easynet report-remote-desktop-client-state failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout)
        .context("parse remote_desktop.report_client_state JSON response")
}

fn show_session_view(config: &ReceiverConfig) -> Result<Value> {
    let output = easynet_command(config)
        .args(["ability", "show-remote-desktop-session", "--session-json"])
        .arg(&config.session_json)
        .args(["--format", "json"])
        .output()
        .context("spawn easynet ability show-remote-desktop-session")?;
    if !output.status.success() {
        bail!(
            "easynet show-remote-desktop-session failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout)
        .context("parse remote_desktop.show_session JSON response")
}

fn easynet_command(config: &ReceiverConfig) -> Command {
    if let Some(path) = config.easynet_bin.as_ref() {
        return Command::new(path);
    }
    let debug_bin = PathBuf::from("target/debug/easynet");
    if debug_bin.is_file() {
        return Command::new(debug_bin);
    }
    let mut command = Command::new("cargo");
    command.args(["run", "--quiet", "--bin", "easynet", "--"]);
    command
}

/// Reassembles one decoder submission from all H.264 NAL units sharing an RTP
/// timestamp. `H26xWriter` owns RFC 6184 depacketization; this state machine
/// owns the access-unit boundary signalled by the RTP marker bit.
struct H264AccessUnitAssembler {
    depacketized_output: AnnexBBuffer,
    writer: H26xWriter<AnnexBWriter>,
    access_unit: Vec<u8>,
    active_timestamp: Option<u32>,
    next_sequence: Option<u16>,
    current_invalid: bool,
    dropped_count: usize,
}

impl H264AccessUnitAssembler {
    fn new() -> Self {
        let depacketized_output = AnnexBBuffer::default();
        let writer = H26xWriter::new(AnnexBWriter(Arc::clone(&depacketized_output.0)), false);
        Self {
            depacketized_output,
            writer,
            access_unit: Vec::new(),
            active_timestamp: None,
            next_sequence: None,
            current_invalid: false,
            dropped_count: 0,
        }
    }

    fn output(&self) -> AnnexBBuffer {
        self.depacketized_output.clone()
    }

    fn dropped_count(&self) -> usize {
        self.dropped_count
    }

    fn push<F>(
        &mut self,
        timestamp: u32,
        sequence: u16,
        marker: bool,
        depacketize: F,
    ) -> Result<Option<Vec<u8>>>
    where
        F: FnOnce(&mut H26xWriter<AnnexBWriter>) -> Result<Vec<u8>>,
    {
        let timestamp_discontinuity = self
            .active_timestamp
            .is_some_and(|active| active != timestamp);
        if timestamp_discontinuity {
            self.drop_current();
            self.reset_depacketizer();
        }
        if self.active_timestamp.is_none() {
            self.active_timestamp = Some(timestamp);
            self.current_invalid = false;
        }

        let sequence_discontinuity = self
            .next_sequence
            .is_some_and(|expected| expected != sequence);
        self.next_sequence = Some(sequence.wrapping_add(1));
        if sequence_discontinuity && !timestamp_discontinuity {
            self.current_invalid = true;
            self.reset_depacketizer();
        } else if sequence_discontinuity {
            // The missing packet belonged to an already discarded access unit.
            // Start the new timestamp from a clean depacketizer and let its
            // first packet prove whether it contains a usable keyframe.
            self.reset_depacketizer();
        }

        if self.current_invalid {
            if marker {
                self.drop_current();
            }
            return Ok(None);
        }

        let annexb = depacketize(&mut self.writer)?;
        if !annexb.is_empty() {
            self.access_unit.extend_from_slice(&annexb);
        }
        if !marker {
            return Ok(None);
        }

        self.active_timestamp = None;
        let access_unit = std::mem::take(&mut self.access_unit);
        if access_unit.is_empty() {
            return Ok(None);
        }
        Ok(Some(access_unit))
    }

    fn drop_current(&mut self) {
        if self.active_timestamp.take().is_some() {
            self.dropped_count = self.dropped_count.saturating_add(1);
        }
        self.current_invalid = false;
        self.access_unit.clear();
        let _ = self.depacketized_output.drain();
    }

    fn reset_depacketizer(&mut self) {
        self.access_unit.clear();
        let _ = self.depacketized_output.drain();
        self.writer = H26xWriter::new(AnnexBWriter(Arc::clone(&self.depacketized_output.0)), false);
    }
}

#[derive(Clone, Default)]
struct AnnexBBuffer(Arc<Mutex<Vec<u8>>>);

impl AnnexBBuffer {
    fn drain(&self) -> Vec<u8> {
        let mut guard = self.0.lock().expect("annexb buffer poisoned");
        std::mem::take(&mut *guard)
    }
}

struct AnnexBWriter(Arc<Mutex<Vec<u8>>>);

impl Write for AnnexBWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("annexb buffer poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn count_rgb_matches(rgb: &[u8], expected: [u8; 3], tolerance: u8) -> usize {
    let tolerance = tolerance as i16;
    rgb.chunks_exact(3)
        .filter(|pixel| {
            pixel
                .iter()
                .zip(expected.iter())
                .all(|(actual, expected)| (*actual as i16 - *expected as i16).abs() <= tolerance)
        })
        .count()
}

fn secondary_selected_presence(assertions: &PixelAssertions, pixel_count: usize) -> Option<bool> {
    assertions
        .secondary_selected
        .as_ref()
        .map(|secondary| pixel_count >= secondary.min_pixels)
}

fn new_stream_decoder() -> Result<H264Decoder> {
    H264Decoder::with_api_config(
        OpenH264API::from_source(),
        DecoderConfig::new().flush_after_decode(Flush::NoFlush),
    )
    .map_err(Into::into)
}

fn annexb_nal_types(bytes: &[u8]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut offset = 0usize;
    while offset + 3 < bytes.len() {
        let start_len = if bytes[offset..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if bytes[offset..].starts_with(&[0, 0, 1]) {
            3
        } else {
            offset += 1;
            continue;
        };
        let header = offset + start_len;
        if header < bytes.len() {
            types.push(bytes[header] & 0x1f);
        }
        offset = header.saturating_add(1);
    }
    types
}

fn write_ppm(path: &Path, width: usize, height: usize, rgb: &[u8]) -> Result<()> {
    let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
    bytes.extend_from_slice(rgb);
    fs::write(path, bytes).with_context(|| format!("write decoded frame sample {}", path.display()))
}

fn write_analysis(
    config: &ReceiverConfig,
    status: AnalysisStatus,
    observation: ReceiverObservation,
    error: Option<String>,
) -> Result<()> {
    let decoded_frame_sample = observation
        .frame
        .decoded_frame_sample
        .as_ref()
        .map(|path| path.display().to_string());
    let decoded_audio = serde_json::to_value(&observation.audio)?;
    let audio_required = config.audio_assertions.is_some();
    let session_view = observation.session_view.clone().unwrap_or(Value::Null);
    let artifact_binding = SessionArtifactBinding::from_session_value(&session_view)
        .unwrap_or_else(|_| config.session_artifact.clone());
    let analysis = json!({
        "status": status.as_str(),
        "error": error,
        "transport": {
            "kind": "webrtc",
            "codec": "h264",
            "audio_codec": audio_required.then_some("opus"),
            "media_scope": if audio_required { "audio_video" } else { "video_only" },
            "carrier": "rtp_srtp"
        },
        "session_view": session_view.clone(),
        "production_readiness": session_view
            .get("production_readiness")
            .cloned()
            .unwrap_or(Value::Null),
        "production_media_ready": session_view
            .get("production_media_ready")
            .cloned()
            .unwrap_or(Value::Bool(false)),
        "client_media_ready": session_view
            .get("production_readiness")
            .and_then(|readiness| readiness.get("client_media_ready"))
            .cloned()
            .unwrap_or(Value::Bool(false)),
        "decoded_frames": {
            "count": observation.frame.count,
            "rtp_packet_count": observation.frame.rtp_packet_count,
            "annexb_chunk_count": observation.frame.annexb_chunk_count,
            "annexb_byte_count": observation.frame.annexb_byte_count,
            "dropped_access_unit_count": observation.frame.dropped_access_unit_count,
            "decode_error_count": observation.frame.decode_error_count,
            "last_decode_error": observation.frame.last_decode_error,
            "width": observation.frame.width,
            "height": observation.frame.height,
            "selected_content_present": observation.frame.selected_content_present,
            "secondary_selected_content_present": observation.frame.secondary_selected_content_present,
            "unrelated_sentinel_present": observation.frame.unrelated_sentinel_present,
            "full_display_leak_detected": observation.frame.full_display_leak_detected,
            "selected_pixel_count": observation.frame.selected_pixel_count,
            "secondary_selected_pixel_count": observation.frame.secondary_selected_pixel_count,
            "unrelated_pixel_count": observation.frame.unrelated_pixel_count
        },
        "decoded_audio": decoded_audio,
        "artifacts": {
            "decoded_frame_sample": decoded_frame_sample,
            "session_id": artifact_binding.session_id,
            "subject_ura": artifact_binding.subject_ura,
            "binding_id": artifact_binding.binding_id,
            "binding_epoch": artifact_binding.binding_epoch,
            "target_identity_epoch": artifact_binding.target_identity_epoch,
            "target_geometry_revision": artifact_binding.target_geometry_revision,
            "media_source_epoch": artifact_binding.media_source_epoch,
            "consent_epoch": artifact_binding.consent_epoch,
            "capture_scope": artifact_binding.capture_scope
        }
    });
    fs::write(
        &config.frame_analysis_json,
        serde_json::to_vec_pretty(&analysis)?,
    )
    .with_context(|| {
        format!(
            "write frame analysis JSON {}",
            config.frame_analysis_json.display()
        )
    })
}

fn env_path(name: &'static str) -> Result<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| anyhow!("{name} is required"))
}

fn env_rgb(name: &'static str) -> Result<[u8; 3]> {
    let raw = std::env::var(name).with_context(|| format!("{name} is required"))?;
    parse_rgb(name, &raw)
}

fn optional_env_rgb(name: &'static str) -> Result<Option<[u8; 3]>> {
    match std::env::var(name) {
        Ok(raw) if !raw.trim().is_empty() => parse_rgb(name, &raw).map(Some),
        _ => Ok(None),
    }
}

fn parse_rgb(name: &'static str, raw: &str) -> Result<[u8; 3]> {
    let parts = raw
        .split(',')
        .map(|part| part.trim().parse::<u8>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("{name} must be formatted as r,g,b"))?;
    if parts.len() != 3 {
        bail!("{name} must contain exactly three comma-separated bytes");
    }
    Ok([parts[0], parts[1], parts[2]])
}

fn env_u64(name: &'static str, default: u64) -> Result<u64> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .trim()
            .parse::<u64>()
            .with_context(|| format!("{name} must be an unsigned integer")),
        _ => Ok(default),
    }
}

fn optional_env_f64(name: &'static str) -> Result<Option<f64>> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .trim()
            .parse::<f64>()
            .with_context(|| format!("{name} must be a number"))
            .map(Some),
        _ => Ok(None),
    }
}

fn required_env_f64(name: &'static str) -> Result<f64> {
    optional_env_f64(name)?.ok_or_else(|| anyhow!("{name} is required"))
}

fn env_f64(name: &'static str, default: f64) -> Result<f64> {
    optional_env_f64(name).map(|value| value.unwrap_or(default))
}

fn env_u8(name: &'static str, default: u8) -> Result<u8> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .trim()
            .parse::<u8>()
            .with_context(|| format!("{name} must be an unsigned byte")),
        _ => Ok(default),
    }
}

fn env_usize(name: &'static str, default: usize) -> Result<usize> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .trim()
            .parse::<usize>()
            .with_context(|| format!("{name} must be an unsigned integer")),
        _ => Ok(default),
    }
}

#[cfg(test)]
mod audio_tests {
    use super::*;
    use opus::{Application, Encoder};

    #[test]
    fn session_artifact_binding_tracks_latest_rebind_generation() {
        let session = json!({
            "session_id": "rd-artifact-generation",
            "target_binding": {
                "subject_ura": "easynet:///r/acme/resource/application.test",
                "binding_id": "tb_generation",
                "binding_epoch": 4,
                "target_identity_epoch": 8,
                "target_geometry_revision": 6,
                "media_source_epoch": 2,
                "consent_epoch": 1,
                "capture_scope": "AppSurface"
            }
        });
        let artifact = SessionArtifactBinding::from_session_value(&session)
            .expect("latest session generation projects to artifact binding");
        assert_eq!(artifact.binding_epoch, 4);
        assert_eq!(artifact.target_geometry_revision, 6);
        assert_eq!(artifact.media_source_epoch, 2);
    }

    fn assertions() -> AudioAssertions {
        AudioAssertions {
            expected_frequency_hz: 523.25,
            unrelated_frequency_hz: 880.0,
            frequency_tolerance_hz: 25.0,
            min_rms: 0.01,
            min_packets: 8,
            selected_power_ratio: 4.0,
        }
    }

    fn tone(frequency_hz: f64, samples: usize, amplitude: f32) -> Vec<f32> {
        (0..samples)
            .map(|index| {
                (std::f64::consts::TAU * frequency_hz * index as f64 / OPUS_CLOCK_RATE as f64).sin()
                    as f32
                    * amplitude
            })
            .collect()
    }

    fn counters(packet_count: usize, sample_count: usize) -> AudioDecodeCounters {
        AudioDecodeCounters {
            rtp_packet_count: packet_count,
            encoded_byte_count: packet_count * 80,
            decoded_packet_count: packet_count,
            decoded_samples_per_channel: sample_count,
            decode_error_count: 0,
            last_decode_error: None,
        }
    }

    #[test]
    fn selected_tone_passes_frequency_and_isolation_assertions() {
        let assertions = assertions();
        let samples = tone(523.25, 24_000, 0.2);
        let observation = analyze_audio_samples(&samples, &assertions, counters(25, samples.len()));

        assert!(observation.assertions_satisfied(&assertions));
        assert!(observation.selected_tone_present);
        assert!(observation.unrelated_tone_rejected);
        assert!(observation.selected_power_ratio > 100.0);
    }

    #[test]
    fn unrelated_tone_cannot_satisfy_selected_application_proof() {
        let assertions = assertions();
        let samples = tone(880.0, 24_000, 0.2);
        let observation = analyze_audio_samples(&samples, &assertions, counters(25, samples.len()));

        assert!(!observation.assertions_satisfied(&assertions));
        assert!(!observation.selected_tone_present);
        assert!(!observation.unrelated_tone_rejected);
    }

    #[test]
    fn opus_roundtrip_preserves_selected_tone_identity() {
        let assertions = assertions();
        let mut encoder = Encoder::new(OPUS_CLOCK_RATE, OpusChannels::Stereo, Application::Audio)
            .expect("Opus encoder");
        let mut decoder =
            OpusDecoder::new(OPUS_CLOCK_RATE, OpusChannels::Stereo).expect("Opus decoder");
        let selected = tone(523.25, 960 * 20, 0.2);
        let mut decoded_mono = Vec::new();
        let mut encoded_bytes = 0usize;
        for frame in selected.chunks_exact(960) {
            let mut stereo = Vec::with_capacity(1_920);
            for sample in frame {
                stereo.extend_from_slice(&[*sample, *sample]);
            }
            let mut packet = vec![0_u8; 1_275];
            let packet_len = encoder
                .encode_float(&stereo, &mut packet)
                .expect("encode Opus frame");
            encoded_bytes += packet_len;
            let mut decoded = vec![0.0_f32; MAX_OPUS_SAMPLES_PER_CHANNEL * OPUS_CHANNELS];
            let decoded_per_channel = decoder
                .decode_float(&packet[..packet_len], &mut decoded, false)
                .expect("decode Opus frame");
            for frame_index in 0..decoded_per_channel {
                decoded_mono.push(
                    (decoded[frame_index * OPUS_CHANNELS]
                        + decoded[frame_index * OPUS_CHANNELS + 1])
                        * 0.5,
                );
            }
        }
        let observation = analyze_audio_samples(
            &decoded_mono,
            &assertions,
            AudioDecodeCounters {
                rtp_packet_count: 20,
                encoded_byte_count: encoded_bytes,
                decoded_packet_count: 20,
                decoded_samples_per_channel: decoded_mono.len(),
                decode_error_count: 0,
                last_decode_error: None,
            },
        );

        assert!(observation.assertions_satisfied(&assertions));
        assert_eq!(observation.decode_error_count, 0);
    }
}

#[cfg(test)]
mod access_unit_tests {
    use super::*;

    fn packet_bytes(
        bytes: &'static [u8],
    ) -> impl FnOnce(&mut H26xWriter<AnnexBWriter>) -> Result<Vec<u8>> {
        move |_| Ok(bytes.to_vec())
    }

    #[test]
    fn emits_only_marker_complete_access_units() {
        let mut assembler = H264AccessUnitAssembler::new();

        assert!(assembler
            .push(90_000, 10, false, packet_bytes(&[0, 0, 0, 1, 7]))
            .unwrap()
            .is_none());
        let access_unit = assembler
            .push(90_000, 11, true, packet_bytes(&[0, 0, 0, 1, 5]))
            .unwrap()
            .expect("marker must complete access unit");

        assert_eq!(access_unit, [0, 0, 0, 1, 7, 0, 0, 0, 1, 5]);
        assert_eq!(assembler.dropped_count(), 0);
    }

    #[test]
    fn sequence_gap_discards_partial_access_unit() {
        let mut assembler = H264AccessUnitAssembler::new();

        assert!(assembler
            .push(90_000, 10, false, packet_bytes(&[0, 0, 0, 1, 7]))
            .unwrap()
            .is_none());
        assert!(assembler
            .push(90_000, 12, true, packet_bytes(&[0, 0, 0, 1, 5]))
            .unwrap()
            .is_none());

        assert_eq!(assembler.dropped_count(), 1);
    }

    #[test]
    fn timestamp_change_discards_unmarked_unit_and_accepts_new_boundary() {
        let mut assembler = H264AccessUnitAssembler::new();

        assert!(assembler
            .push(90_000, 10, false, packet_bytes(&[0, 0, 0, 1, 1]))
            .unwrap()
            .is_none());
        let access_unit = assembler
            .push(93_000, 11, true, packet_bytes(&[0, 0, 0, 1, 7]))
            .unwrap()
            .expect("new timestamp may recover from a clean keyframe boundary");

        assert_eq!(access_unit, [0, 0, 0, 1, 7]);
        assert_eq!(assembler.dropped_count(), 1);
    }

    #[test]
    fn sequence_tracking_wraps_at_u16_boundary() {
        let mut assembler = H264AccessUnitAssembler::new();

        assert!(assembler
            .push(90_000, u16::MAX, false, packet_bytes(&[1]))
            .unwrap()
            .is_none());
        assert!(assembler
            .push(90_000, 0, true, packet_bytes(&[2]))
            .unwrap()
            .is_some());
        assert_eq!(assembler.dropped_count(), 0);
    }

    #[test]
    fn reports_annexb_parameter_set_and_idr_types() {
        assert_eq!(
            annexb_nal_types(&[0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 4,]),
            [7, 8, 5]
        );
    }
}

#[cfg(not(feature = "remote-desktop"))]
fn main() {
    eprintln!("easynet-remoteapp-frame-receiver requires the remote-desktop feature");
    std::process::exit(64);
}
