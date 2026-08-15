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
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use openh264::decoder::Decoder;
use openh264::formats::YUVSource;
use rtc::interceptor::Registry;
use rtc::media::io::h26x_writer::H26xWriter;
use rtc::media::io::Writer as H26xRtpWriter;
use rtc::peer_connection::configuration::interceptor_registry::register_default_interceptors;
use rtc::peer_connection::configuration::media_engine::{MediaEngine, MIME_TYPE_H264};
use rtc::peer_connection::configuration::RTCConfigurationBuilder;
use rtc::peer_connection::sdp::RTCSessionDescription;
use rtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtc::rtp_transceiver::rtp_sender::{RTCRtpCodec, RTCRtpCodecParameters, RtpCodecKind};
use rtc::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};
use serde::Serialize;
use serde_json::{json, Value};
use webrtc::media_stream::track_remote::{TrackRemote, TrackRemoteEvent};
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler, RTCIceGatheringState,
    RTCPeerConnectionState,
};
use webrtc::runtime::{block_on, channel, default_runtime, sleep, Runtime, Sender};

const H264_PAYLOAD_TYPE: u8 = 102;
const H264_CLOCK_RATE: u32 = 90_000;
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

fn main() -> Result<()> {
    block_on(async_main())
}

async fn async_main() -> Result<()> {
    let config = ReceiverConfig::from_env()?;
    let result = run_receiver(&config).await;
    match result {
        Ok(observation) => {
            if observation.frame.assertions_satisfied() {
                write_analysis(&config, AnalysisStatus::Passed, observation, None)?;
                Ok(())
            } else {
                let error = observation.frame.failure_reason();
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
    session_artifact: SessionArtifactBinding,
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
        let session_artifact = SessionArtifactBinding::from_session_json(&session_json)?;
        Ok(Self {
            session_json,
            frame_analysis_json,
            out_dir,
            easynet_bin,
            timeout,
            assertions,
            session_artifact,
        })
    }
}

#[derive(Clone, Serialize)]
struct SessionArtifactBinding {
    session_id: String,
    binding_id: String,
    binding_epoch: u64,
    capture_scope: String,
}

impl SessionArtifactBinding {
    fn from_session_json(path: &Path) -> Result<Self> {
        let value: Value = serde_json::from_slice(
            &fs::read(path).with_context(|| format!("read session JSON {}", path.display()))?,
        )
        .with_context(|| format!("parse session JSON {}", path.display()))?;
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
        let binding_epoch = target_binding
            .get("binding_epoch")
            .and_then(Value::as_u64)
            .filter(|value| *value > 0)
            .ok_or_else(|| anyhow!("session target_binding missing positive binding_epoch"))?;
        let capture_scope = target_binding
            .get("capture_scope")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("session target_binding missing capture_scope"))?
            .to_string();
        Ok(Self {
            session_id,
            binding_id,
            binding_epoch,
            capture_scope,
        })
    }
}

#[derive(Clone)]
struct PixelAssertions {
    selected: RgbAssertion,
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
        Ok(Self {
            selected,
            unrelated,
        })
    }
}

#[derive(Clone)]
struct RgbAssertion {
    rgb: [u8; 3],
    tolerance: u8,
    min_pixels: usize,
}

#[derive(Debug, Clone, Serialize)]
struct FrameObservation {
    count: usize,
    rtp_packet_count: usize,
    annexb_chunk_count: usize,
    annexb_byte_count: usize,
    decode_error_count: usize,
    last_decode_error: Option<String>,
    width: Option<usize>,
    height: Option<usize>,
    selected_content_present: bool,
    unrelated_sentinel_present: bool,
    full_display_leak_detected: bool,
    selected_pixel_count: usize,
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
            decode_error_count: 0,
            last_decode_error: None,
            width: None,
            height: None,
            selected_content_present: false,
            unrelated_sentinel_present: false,
            full_display_leak_detected: false,
            selected_pixel_count: 0,
            unrelated_pixel_count: 0,
            decoded_frame_sample: None,
        }
    }

    fn assertions_satisfied(&self) -> bool {
        self.selected_content_present && !self.unrelated_sentinel_present
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
        if self.unrelated_sentinel_present {
            return format!(
                "remoteapp receiver detected unrelated sentinel content in decoded target frame; selected_pixels={}, unrelated_pixels={}",
                self.selected_pixel_count, self.unrelated_pixel_count
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
}

#[derive(Debug, Clone)]
struct ReceiverObservation {
    session_view: Option<Value>,
    frame: FrameObservation,
}

impl ReceiverObservation {
    fn failed() -> Self {
        Self {
            session_view: None,
            frame: FrameObservation::failed(),
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
        if kind != RtpCodecKind::Video {
            return;
        }
        let media_ssrc = match track.ssrcs().await.first().copied() {
            Some(ssrc) => ssrc,
            None => return,
        };
        let pli_track = Arc::clone(&track);
        self.runtime.spawn(Box::pin(async move {
            loop {
                sleep(Duration::from_secs(3)).await;
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
    let registry = register_default_interceptors(Registry::new(), &mut media_engine)?;
    let runtime = default_runtime().ok_or_else(|| anyhow!("no WebRTC async runtime available"))?;
    let (gather_complete_tx, mut gather_complete_rx) = channel::<()>(1);
    let (done_tx, mut done_rx) = channel::<()>(1);
    let (observation_tx, observation_rx) = mpsc::channel::<FrameObservation>();
    let sample_path = config.out_dir.join("decoded-frame-sample.ppm");
    let handler = Arc::new(Handler {
        runtime: Arc::clone(&runtime),
        gather_complete_tx,
        done_tx,
        observation_tx,
        assertions: config.assertions.clone(),
        sample_path,
    });

    let peer_connection = PeerConnectionBuilder::new()
        .with_configuration(RTCConfigurationBuilder::new().build())
        .with_media_engine(media_engine)
        .with_interceptor_registry(registry)
        .with_handler(handler as Arc<dyn PeerConnectionEventHandler>)
        .with_runtime(runtime)
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

    let offer = peer_connection.create_offer(None).await?;
    peer_connection.set_local_description(offer).await?;
    wait_for_local_ice_gathering(&mut gather_complete_rx).await;
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
        match observation_rx.try_recv() {
            Ok(observation) => {
                if observation.assertions_satisfied() {
                    let latest_session_view = show_session_view(config)
                        .context("invoke remote_desktop.show_session after decoded frame")
                        .ok();
                    peer_connection.close().await?;
                    return Ok(ReceiverObservation {
                        session_view: latest_session_view,
                        frame: observation,
                    });
                }
                latest_observation = Some(ReceiverObservation {
                    session_view: None,
                    frame: observation,
                });
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                if let Some(mut observation) = latest_observation {
                    observation.session_view = show_session_view(config).ok();
                    peer_connection.close().await?;
                    return Ok(observation);
                }
                bail!("decoded frame observation channel closed before assertions passed");
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if started.elapsed() > config.timeout {
            if let Some(mut observation) = latest_observation {
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
        sleep(Duration::from_millis(100)).await;
    }
}

async fn wait_for_local_ice_gathering(gather_complete_rx: &mut webrtc::runtime::Receiver<()>) {
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
        sleep(Duration::from_millis(25)).await;
    }
}

async fn receive_decode_and_assert(
    track: Arc<dyn TrackRemote>,
    assertions: PixelAssertions,
    sample_path: PathBuf,
    observation_tx: mpsc::Sender<FrameObservation>,
) -> Result<()> {
    let annexb = AnnexBBuffer::default();
    let mut writer = H26xWriter::new(AnnexBWriter(Arc::clone(&annexb.0)), false);
    let mut decoder = Decoder::new()?;
    let mut decoded_count = 0usize;
    let mut rtp_packet_count = 0usize;
    let mut annexb_chunk_count = 0usize;
    let mut annexb_byte_count = 0usize;
    let mut decode_error_count = 0usize;
    let mut last_decode_error: Option<String> = None;
    let mut best_selected_pixels = 0usize;
    let mut best_unrelated_pixels = 0usize;
    while let Some(event) = track.poll().await {
        let TrackRemoteEvent::OnRtpPacket(packet) = event else {
            continue;
        };
        rtp_packet_count += 1;
        writer.write_rtp(&packet)?;
        let chunk = annexb.drain();
        if chunk.is_empty() {
            continue;
        }
        annexb_chunk_count += 1;
        annexb_byte_count += chunk.len();
        let decoded = match decoder.decode(&chunk) {
            Ok(Some(decoded)) => decoded,
            Ok(None) => continue,
            Err(error) => {
                decode_error_count += 1;
                last_decode_error = Some(error.to_string());
                eprintln!(
                    "easynet remoteapp receiver H.264 decode error #{decode_error_count}: {error}; annexb_chunk_bytes={}",
                    chunk.len()
                );
                decoder = Decoder::new()?;
                let _ = observation_tx.send(FrameObservation {
                    count: decoded_count,
                    rtp_packet_count,
                    annexb_chunk_count,
                    annexb_byte_count,
                    decode_error_count,
                    last_decode_error: last_decode_error.clone(),
                    width: None,
                    height: None,
                    selected_content_present: false,
                    unrelated_sentinel_present: best_unrelated_pixels
                        >= assertions.unrelated.min_pixels,
                    full_display_leak_detected: best_unrelated_pixels
                        >= assertions.unrelated.min_pixels,
                    selected_pixel_count: best_selected_pixels,
                    unrelated_pixel_count: best_unrelated_pixels,
                    decoded_frame_sample: None,
                });
                if decode_error_count >= MAX_DECODE_ERRORS_BEFORE_FAILURE_OBSERVATION {
                    let _ = observation_tx.send(FrameObservation {
                        count: decoded_count,
                        rtp_packet_count,
                        annexb_chunk_count,
                        annexb_byte_count,
                        decode_error_count,
                        last_decode_error,
                        width: None,
                        height: None,
                        selected_content_present: false,
                        unrelated_sentinel_present: best_unrelated_pixels
                            >= assertions.unrelated.min_pixels,
                        full_display_leak_detected: best_unrelated_pixels
                            >= assertions.unrelated.min_pixels,
                        selected_pixel_count: best_selected_pixels,
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
        best_selected_pixels = best_selected_pixels.max(selected_pixels);
        best_unrelated_pixels = best_unrelated_pixels.max(unrelated_pixels);
        write_ppm(&sample_path, width, height, &rgb)?;

        let selected_content_present = selected_pixels >= assertions.selected.min_pixels;
        let unrelated_sentinel_present = unrelated_pixels >= assertions.unrelated.min_pixels;
        let observation = FrameObservation {
            count: decoded_count,
            rtp_packet_count,
            annexb_chunk_count,
            annexb_byte_count,
            decode_error_count,
            last_decode_error: last_decode_error.clone(),
            width: Some(width),
            height: Some(height),
            selected_content_present,
            unrelated_sentinel_present,
            full_display_leak_detected: unrelated_sentinel_present,
            selected_pixel_count: selected_pixels,
            unrelated_pixel_count: unrelated_pixels,
            decoded_frame_sample: Some(sample_path.clone()),
        };
        let _ = observation_tx.send(observation.clone());
        if selected_content_present && !unrelated_sentinel_present {
            return Ok(());
        }
    }
    let _ = observation_tx.send(FrameObservation {
        count: decoded_count,
        rtp_packet_count,
        annexb_chunk_count,
        annexb_byte_count,
        decode_error_count,
        last_decode_error,
        width: None,
        height: None,
        selected_content_present: false,
        unrelated_sentinel_present: best_unrelated_pixels >= assertions.unrelated.min_pixels,
        full_display_leak_detected: best_unrelated_pixels >= assertions.unrelated.min_pixels,
        selected_pixel_count: best_selected_pixels,
        unrelated_pixel_count: best_unrelated_pixels,
        decoded_frame_sample: None,
    });
    Ok(())
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
    Ok(SignalingAnswer { answer })
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

#[derive(Default)]
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
    let session_view = observation.session_view.clone().unwrap_or(Value::Null);
    let analysis = json!({
        "status": status.as_str(),
        "error": error,
        "transport": {
            "kind": "webrtc",
            "codec": "h264",
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
            "decode_error_count": observation.frame.decode_error_count,
            "last_decode_error": observation.frame.last_decode_error,
            "width": observation.frame.width,
            "height": observation.frame.height,
            "selected_content_present": observation.frame.selected_content_present,
            "unrelated_sentinel_present": observation.frame.unrelated_sentinel_present,
            "full_display_leak_detected": observation.frame.full_display_leak_detected,
            "selected_pixel_count": observation.frame.selected_pixel_count,
            "unrelated_pixel_count": observation.frame.unrelated_pixel_count
        },
        "artifacts": {
            "decoded_frame_sample": decoded_frame_sample,
            "session_id": config.session_artifact.session_id,
            "binding_id": config.session_artifact.binding_id,
            "binding_epoch": config.session_artifact.binding_epoch,
            "capture_scope": config.session_artifact.capture_scope
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

#[cfg(not(feature = "remote-desktop"))]
fn main() {
    eprintln!("easynet-remoteapp-frame-receiver requires the remote-desktop feature");
    std::process::exit(64);
}
