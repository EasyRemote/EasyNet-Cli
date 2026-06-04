// EasyNet CLI — remote desktop H.264 encoding
// ===========================================
//
// File: src/plugins/builtin/remote_desktop/media/encode.rs
// Description: OpenH264/Annex-B media encoding paths for remote desktop.

use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, IntraFramePeriod, Level, Profile,
    RateControlMode, UsageType,
};
use openh264::formats::{RgbSliceU8, YUVBuffer};
use openh264::{OpenH264API, Timestamp};
use serde_json::json;
use tokio::sync::{mpsc, watch};
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;

use crate::persistence::resources::ResourceEntry;
use crate::plugins::remote_desktop::config::MIN_FRAME_QUEUE_DEPTH;
use crate::plugins::remote_desktop::constants::{
    DEFAULT_VIDEO_STREAM_BITRATE_KBPS, MAX_ATTACH_FPS, MAX_FRAME_QUEUE_DEPTH, MIN_ATTACH_FPS,
    NATIVE_MAX_BITRATE_KBPS, NATIVE_MIN_BITRATE_KBPS, REASON_PREVIEW_CAPTURE_FAILED,
    REASON_PREVIEW_CLIENT_CLOSED, REASON_RESOURCE_UNAVAILABLE, TRANSPORT_INVOKE_BIDI,
};
use crate::plugins::remote_desktop::media::{
    select_builtin_h264_backend, webrtc_transport_backend_for_entry,
    RemoteDesktopMediaBackendDescriptor,
};
use crate::plugins::remote_desktop::transport::BidiTerminalGuard;
use crate::runtime::ability_dispatch::BidiOutputFrame;
use crate::runtime::agents::media::screen_snapshot::{
    capture_rgb_with_xcap, open_display_recorder_with_xcap, rgba_bytes_to_rgb_frame, RawRgbFrame,
    ScreenCaptureOptions,
};

const H264_ANNEX_B_CONTENT_TYPE: &str = "video/h264; stream-format=annexb";
pub(in crate::plugins::builtin::remote_desktop) const DIAGNOSTIC_RELAY_GOP_MAX_FRAMES: u32 = 15;
const RECORDER_FRAME_TIMEOUT_MS: u64 = 250;

/// Runtime H.264 stream configuration selected from local media backends.
///
/// Invariant 1: `requested_fps` preserves the caller's clamped request.
/// Invariant 2: `fps` is the actual backend-safe rate used for capture and
/// encoding.
/// Invariant 3: `max_frame_queue_depth` is already clamped to the manifest
/// runtime limit before a producer thread is spawned.
#[derive(Debug, Clone)]
pub(in crate::plugins::builtin::remote_desktop) struct BuiltinH264Config {
    pub(in crate::plugins::builtin::remote_desktop) backend: RemoteDesktopMediaBackendDescriptor,
    pub(in crate::plugins::builtin::remote_desktop) requested_fps: u32,
    pub(in crate::plugins::builtin::remote_desktop) fps: u32,
    pub(in crate::plugins::builtin::remote_desktop) bitrate_kbps: u32,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::plugins::builtin::remote_desktop) max_frame_queue_depth: usize,
    pub(in crate::plugins::builtin::remote_desktop) keyframe_interval_frames: u32,
}

/// Terminal outcome of the built-in H.264 preview worker.
///
/// What this is NOT: session state. The media layer reports the bounded
/// terminal fact, and `invoke_bidi` projects it into session-store state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::plugins::builtin::remote_desktop) enum BuiltinH264StreamTerminal {
    Closed(&'static str),
    Failed {
        reason: &'static str,
        message: String,
    },
}

impl BuiltinH264StreamTerminal {
    fn close_reason(&self) -> &'static str {
        match self {
            Self::Closed(reason) => reason,
            Self::Failed { reason, .. } => reason,
        }
    }
}

pub(in crate::plugins::builtin::remote_desktop) type BuiltinH264TerminalCallback =
    Arc<dyn Fn(BuiltinH264StreamTerminal) + Send + Sync + 'static>;

pub(in crate::plugins::builtin::remote_desktop) fn spawn_builtin_h264_stream(
    entry: ResourceEntry,
    options: ScreenCaptureOptions,
    max_frame_queue_depth: usize,
    to_client: mpsc::Sender<BidiOutputFrame>,
    stop_rx: watch::Receiver<bool>,
    terminal_guard: BidiTerminalGuard,
    terminal_callback: BuiltinH264TerminalCallback,
) -> bool {
    let Some(config) = build_builtin_h264_config(&entry, &options, max_frame_queue_depth) else {
        return false;
    };
    std::thread::Builder::new()
        .name("easynet-remote-desktop-openh264".into())
        .spawn(move || {
            let terminal =
                run_builtin_h264_stream(entry, options, config, to_client, stop_rx, terminal_guard);
            terminal_callback(terminal);
        })
        .map(|_| true)
        .unwrap_or_else(|err| {
            crate::op_event!(
                component = remote_desktop,
                kind = builtin_h264_spawn_failed,
                reason = err.to_string(),
            );
            false
        })
}

pub(in crate::plugins::builtin::remote_desktop) fn build_builtin_h264_config(
    entry: &ResourceEntry,
    options: &ScreenCaptureOptions,
    max_frame_queue_depth: usize,
) -> Option<BuiltinH264Config> {
    let backend = select_builtin_h264_backend(entry)?;
    let requested_fps = options.fps.clamp(MIN_ATTACH_FPS, MAX_ATTACH_FPS);
    let actual_fps = backend.effective_fps(requested_fps);
    Some(BuiltinH264Config {
        backend,
        requested_fps,
        fps: actual_fps,
        bitrate_kbps: DEFAULT_VIDEO_STREAM_BITRATE_KBPS,
        max_frame_queue_depth: max_frame_queue_depth.max(MIN_FRAME_QUEUE_DEPTH),
        keyframe_interval_frames: actual_fps.clamp(1, DIAGNOSTIC_RELAY_GOP_MAX_FRAMES),
    })
}

pub(in crate::plugins::builtin::remote_desktop) fn build_direct_webrtc_h264_config(
    entry: &ResourceEntry,
    options: &ScreenCaptureOptions,
    target_bitrate_kbps: u32,
    max_frame_queue_depth: usize,
) -> Option<BuiltinH264Config> {
    let backend = webrtc_transport_backend_for_entry(entry)?;
    let requested_fps = options.fps.clamp(MIN_ATTACH_FPS, MAX_ATTACH_FPS);
    let actual_fps = backend.effective_fps(requested_fps);
    let mut config = BuiltinH264Config {
        backend,
        requested_fps,
        fps: actual_fps,
        bitrate_kbps: target_bitrate_kbps.clamp(NATIVE_MIN_BITRATE_KBPS, NATIVE_MAX_BITRATE_KBPS),
        max_frame_queue_depth: max_frame_queue_depth.clamp(1, MAX_FRAME_QUEUE_DEPTH as usize),
        keyframe_interval_frames: actual_fps.clamp(1, DIAGNOSTIC_RELAY_GOP_MAX_FRAMES),
    };
    config.keyframe_interval_frames = config.fps.clamp(1, 30);
    Some(config)
}

fn run_builtin_h264_stream(
    entry: ResourceEntry,
    options: ScreenCaptureOptions,
    config: BuiltinH264Config,
    to_client: mpsc::Sender<BidiOutputFrame>,
    stop_rx: watch::Receiver<bool>,
    terminal_guard: BidiTerminalGuard,
) -> BuiltinH264StreamTerminal {
    if let Some(terminal) = run_builtin_h264_recorder_stream(
        &entry,
        &options,
        &config,
        to_client.clone(),
        stop_rx.clone(),
        terminal_guard.clone(),
    ) {
        return terminal;
    }
    run_builtin_h264_polling_stream(entry, options, config, to_client, stop_rx, terminal_guard)
}

fn h264_failure(
    to_client: &mpsc::Sender<BidiOutputFrame>,
    message: impl Into<String>,
) -> BuiltinH264StreamTerminal {
    let message = message.into();
    let _ = to_client.blocking_send(BidiOutputFrame::json(json!({
        "type": "error",
        "code": REASON_RESOURCE_UNAVAILABLE,
        "message": message.clone(),
    })));
    BuiltinH264StreamTerminal::Failed {
        reason: REASON_PREVIEW_CAPTURE_FAILED,
        message,
    }
}

fn run_builtin_h264_recorder_stream(
    entry: &ResourceEntry,
    options: &ScreenCaptureOptions,
    config: &BuiltinH264Config,
    to_client: mpsc::Sender<BidiOutputFrame>,
    mut stop_rx: watch::Receiver<bool>,
    terminal_guard: BidiTerminalGuard,
) -> Option<BuiltinH264StreamTerminal> {
    let Ok((recorder, rx)) = open_display_recorder_with_xcap(entry) else {
        return None;
    };
    let mut encoder = match build_openh264_encoder(config) {
        Ok(encoder) => encoder,
        Err(err) => {
            let terminal = h264_failure(&to_client, format!("OpenH264 encoder unavailable: {err}"));
            terminal_guard.send_blocking_closed(&to_client, terminal.close_reason());
            return Some(terminal);
        }
    };
    if let Err(err) = recorder.start() {
        let terminal = h264_failure(
            &to_client,
            format!("xcap video recorder start failed: {err}"),
        );
        terminal_guard.send_blocking_closed(&to_client, terminal.close_reason());
        return Some(terminal);
    }
    let mut seq = 0_u64;
    let mut announced = false;
    let mut last_capture_started = Instant::now();
    let mut terminal = BuiltinH264StreamTerminal::Closed(REASON_PREVIEW_CLIENT_CLOSED);
    loop {
        if *stop_rx.borrow() {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(RECORDER_FRAME_TIMEOUT_MS)) {
            Ok(frame) => {
                last_capture_started = Instant::now();
                let frame = latest_recorder_frame(&rx, frame);
                let frame =
                    match rgba_bytes_to_rgb_frame(frame.raw, frame.width, frame.height, options)
                        .map(even_rgb_frame)
                    {
                        Ok(frame) => frame,
                        Err(err) => {
                            terminal = h264_failure(&to_client, err.to_string());
                            break;
                        }
                    };
                match encode_openh264_frame(&mut encoder, &frame, seq, config.fps) {
                    Ok(bytes) => {
                        if !announced {
                            if announce_h264_stream(
                                &to_client,
                                entry,
                                &frame,
                                config,
                                "xcap_video_recorder",
                                false,
                            )
                            .is_err()
                            {
                                break;
                            }
                            announced = true;
                        }
                        if !bytes.is_empty()
                            && !send_live_binary_frame(&to_client, bytes, H264_ANNEX_B_CONTENT_TYPE)
                        {
                            break;
                        }
                        seq = seq.saturating_add(1);
                    }
                    Err(err) => {
                        terminal = h264_failure(&to_client, err.to_string());
                        break;
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if last_capture_started.elapsed() > Duration::from_secs(2) {
                    let _ = to_client.blocking_send(BidiOutputFrame::json(json!({
                        "type": "warn",
                        "code": "capture_starved",
                        "message": "xcap video recorder has not produced a frame for 2 seconds",
                    })));
                    last_capture_started = Instant::now();
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                terminal = h264_failure(&to_client, "xcap video recorder disconnected");
                break;
            }
        }
        if stop_rx.has_changed().unwrap_or(false) && *stop_rx.borrow_and_update() {
            break;
        }
    }
    let _ = recorder.stop();
    terminal_guard.send_blocking_closed(&to_client, terminal.close_reason());
    Some(terminal)
}

pub(in crate::plugins::builtin::remote_desktop) fn latest_recorder_frame(
    rx: &std::sync::mpsc::Receiver<xcap::Frame>,
    mut frame: xcap::Frame,
) -> xcap::Frame {
    loop {
        match rx.try_recv() {
            Ok(next) => frame = next,
            Err(TryRecvError::Empty) => return frame,
            Err(TryRecvError::Disconnected) => return frame,
        }
    }
}

fn run_builtin_h264_polling_stream(
    entry: ResourceEntry,
    options: ScreenCaptureOptions,
    config: BuiltinH264Config,
    to_client: mpsc::Sender<BidiOutputFrame>,
    mut stop_rx: watch::Receiver<bool>,
    terminal_guard: BidiTerminalGuard,
) -> BuiltinH264StreamTerminal {
    let mut encoder = match build_openh264_encoder(&config) {
        Ok(encoder) => encoder,
        Err(err) => {
            let terminal = h264_failure(&to_client, format!("OpenH264 encoder unavailable: {err}"));
            terminal_guard.send_blocking_closed(&to_client, terminal.close_reason());
            return terminal;
        }
    };
    let interval = Duration::from_secs_f64(1.0 / config.fps as f64);
    let mut seq = 0_u64;
    let mut announced = false;
    let mut terminal = BuiltinH264StreamTerminal::Closed(REASON_PREVIEW_CLIENT_CLOSED);
    loop {
        if *stop_rx.borrow() {
            break;
        }
        let started = Instant::now();
        match capture_rgb_with_xcap(&entry, &options)
            .map(even_rgb_frame)
            .and_then(|frame| {
                encode_openh264_frame(&mut encoder, &frame, seq, config.fps)
                    .map(|bytes| (frame, bytes))
            }) {
            Ok((frame, bytes)) => {
                if !announced {
                    if announce_h264_stream(
                        &to_client,
                        &entry,
                        &frame,
                        &config,
                        "xcap_capture_image",
                        false,
                    )
                    .is_err()
                    {
                        break;
                    }
                    announced = true;
                }
                if !bytes.is_empty()
                    && !send_live_binary_frame(&to_client, bytes, H264_ANNEX_B_CONTENT_TYPE)
                {
                    break;
                }
                seq = seq.saturating_add(1);
            }
            Err(err) => {
                terminal = h264_failure(&to_client, err.to_string());
                break;
            }
        }
        if stop_rx.has_changed().unwrap_or(false) && *stop_rx.borrow_and_update() {
            break;
        }
        if let Some(remaining) = interval.checked_sub(started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
    terminal_guard.send_blocking_closed(&to_client, terminal.close_reason());
    terminal
}

fn announce_h264_stream(
    to_client: &mpsc::Sender<BidiOutputFrame>,
    entry: &ResourceEntry,
    frame: &RawRgbFrame,
    config: &BuiltinH264Config,
    capture_source: &str,
    hardware_accelerated: bool,
) -> Result<(), mpsc::error::SendError<BidiOutputFrame>> {
    to_client.blocking_send(BidiOutputFrame::json(json!({
        "type": "media_stream",
        "transport": TRANSPORT_INVOKE_BIDI,
        "encoding": "annexb_h264",
        "codec": "h264",
        "codec_string": "avc1.42E033",
        "content_type": H264_ANNEX_B_CONTENT_TYPE,
        "width": frame.width,
        "height": frame.height,
        "fps": config.fps,
        "requested_fps": config.requested_fps,
        "max_capture_fps": config.backend.max_capture_fps(),
        "max_encode_fps": config.backend.max_encode_fps(),
        "bitrate_kbps": config.bitrate_kbps,
        "keyframe_interval_frames": config.keyframe_interval_frames,
        "latency_mode": "latest_frame_bounded_gop",
        "drop_stale_frames": true,
        "relay_queue_depth": config.max_frame_queue_depth,
        "hardware_accelerated": hardware_accelerated,
        "encoder": config.backend.encoder(),
        "capture_source": capture_source,
        "capture_api": config.backend.capture_api(),
        "backend_id": config.backend.backend_id(),
        "media_sdk": config.backend.to_json(),
        "hardware_id": entry.hardware_id,
        "message": "Built-in OpenH264 Annex-B stream; no external encoder binary required.",
    })))
}

fn send_live_binary_frame(
    to_client: &mpsc::Sender<BidiOutputFrame>,
    bytes: Vec<u8>,
    content_type: &'static str,
) -> bool {
    match to_client.try_send(BidiOutputFrame::binary(bytes, content_type)) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(_)) => true,
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    }
}

pub(in crate::plugins::builtin::remote_desktop) fn build_openh264_encoder(
    config: &BuiltinH264Config,
) -> anyhow::Result<Encoder> {
    let bitrate_bps = config.bitrate_kbps.saturating_mul(1000);
    let keyframe_interval = config.keyframe_interval_frames.max(1);
    let encoder_config = EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .rate_control_mode(RateControlMode::Bitrate)
        .bitrate(BitRate::from_bps(bitrate_bps))
        .max_frame_rate(FrameRate::from_hz(config.fps as f32))
        .profile(Profile::Baseline)
        .level(Level::Level_5_1)
        .complexity(Complexity::Low)
        .intra_frame_period(IntraFramePeriod::from_num_frames(keyframe_interval));
    Encoder::with_api_config(OpenH264API::from_source(), encoder_config)
        .map_err(|err| anyhow::anyhow!("{err}"))
}

pub(in crate::plugins::builtin::remote_desktop) fn encode_openh264_frame(
    encoder: &mut Encoder,
    frame: &RawRgbFrame,
    seq: u64,
    fps: u32,
) -> anyhow::Result<Vec<u8>> {
    let rgb = RgbSliceU8::new(
        &frame.rgb_bytes,
        (frame.width as usize, frame.height as usize),
    );
    let yuv = YUVBuffer::from_rgb8_source(rgb);
    let timestamp_ms = seq.saturating_mul(1000) / fps.max(1) as u64;
    let bitstream = encoder
        .encode_at(&yuv, Timestamp::from_millis(timestamp_ms))
        .map_err(|err| anyhow::anyhow!("OpenH264 encode failed: {err}"))?;
    Ok(bitstream.to_vec())
}

pub(in crate::plugins::builtin::remote_desktop) fn even_rgb_frame(
    frame: RawRgbFrame,
) -> RawRgbFrame {
    let width = frame.width & !1;
    let height = frame.height & !1;
    if width == frame.width && height == frame.height {
        return frame;
    }
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for row in 0..height {
        let start = (row * frame.width * 3) as usize;
        let end = start + (width * 3) as usize;
        rgb.extend_from_slice(&frame.rgb_bytes[start..end]);
    }
    RawRgbFrame {
        rgb_bytes: rgb,
        width,
        height,
    }
}

pub(in crate::plugins::builtin::remote_desktop) async fn write_h264_sample(
    track: &TrackLocalStaticSample,
    ssrc: u32,
    encoder: &mut Encoder,
    frame: &RawRgbFrame,
    seq: u64,
    fps: u32,
) -> anyhow::Result<()> {
    let bytes = encode_openh264_frame(encoder, frame, seq, fps)?;
    if bytes.is_empty() {
        return Ok(());
    }
    track
        .sample_writer(ssrc)
        .write_sample(&rtc::media::Sample {
            data: Bytes::from(bytes),
            duration: Duration::from_secs_f64(1.0 / fps.max(1) as f64),
            ..Default::default()
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::persistence::resources::{self, ResourcesFile};
    use crate::plugins::remote_desktop::constants::{
        DEFAULT_FRAME_QUEUE_DEPTH, DEFAULT_TARGET_BITRATE_KBPS,
        DIAGNOSTIC_RELAY_TARGET_BITRATE_KBPS,
    };
    use crate::plugins::remote_desktop::media::{
        REMOTE_DESKTOP_MEDIA_SDK_ID, XCAP_MACOS_RECORDER_MAX_FPS, XCAP_OPENH264_BACKEND_ID,
    };
    use crate::plugins::remote_desktop::test_support::seed_xcap_display;

    #[test]
    fn builtin_h264_backend_reports_actual_fps_cap_and_sdk_contract() {
        let mut file = ResourcesFile::default();
        let ura = seed_xcap_display(&mut file, "remote-desktop-xcap-display");
        let entry = resources::lookup_by_ura(&file, &ura).unwrap();
        let config = build_builtin_h264_config(
            entry,
            &ScreenCaptureOptions {
                resolution: None,
                fps: 144,
                region: None,
            },
            DEFAULT_FRAME_QUEUE_DEPTH as usize,
        )
        .unwrap();

        assert_eq!(config.backend.backend_id(), XCAP_OPENH264_BACKEND_ID);
        assert_eq!(config.backend.sdk_id(), REMOTE_DESKTOP_MEDIA_SDK_ID);
        assert_eq!(config.requested_fps, 144);
        assert_eq!(config.fps, XCAP_MACOS_RECORDER_MAX_FPS);
        assert_eq!(config.bitrate_kbps, DIAGNOSTIC_RELAY_TARGET_BITRATE_KBPS);
        assert_eq!(
            config.keyframe_interval_frames,
            DIAGNOSTIC_RELAY_GOP_MAX_FRAMES
        );
        assert!(!config.backend.external_binary_required());
    }

    #[test]
    fn direct_webrtc_honors_invocation_bitrate_and_queue_target() {
        let mut file = ResourcesFile::default();
        let ura = seed_xcap_display(&mut file, "remote-desktop-xcap-display-webrtc");
        let entry = resources::lookup_by_ura(&file, &ura).unwrap();
        let options = ScreenCaptureOptions {
            resolution: None,
            fps: 144,
            region: None,
        };

        let config = build_direct_webrtc_h264_config(
            entry,
            &options,
            DEFAULT_TARGET_BITRATE_KBPS,
            DEFAULT_FRAME_QUEUE_DEPTH as usize,
        )
        .expect("xcap display should expose a direct WebRTC test backend");

        assert_eq!(config.backend.sdk_id(), REMOTE_DESKTOP_MEDIA_SDK_ID);
        assert_eq!(config.requested_fps, 144);
        assert_eq!(config.bitrate_kbps, DEFAULT_TARGET_BITRATE_KBPS);
        assert_eq!(config.max_frame_queue_depth, 1);
        assert_eq!(config.keyframe_interval_frames, 30);
    }
}
