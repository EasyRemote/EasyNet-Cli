// EasyNet CLI — direct WebRTC baseline media paths
// =================================================
//
// File: src/plugins/builtin/remote_desktop/transport/webrtc_baseline_media.rs
// Description: xcap-backed baseline capture strategies for direct WebRTC.

use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::watch;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;

use crate::daemon::ability::builtins::resources::media::screen_snapshot::{
    capture_rgb_with_xcap, rgba_bytes_to_rgb_frame, ScreenCaptureOptions,
};
use crate::persistence::resources::ResourceEntry;
use crate::plugins::remote_desktop::media::encode::{
    build_openh264_encoder, even_rgb_frame, latest_recorder_frame, write_h264_sample,
    BuiltinH264Config,
};

const RECORDER_FRAME_TIMEOUT_MS: u64 = 250;

/// Immutable inputs shared by xcap-backed WebRTC baseline streams.
///
/// This type is the strategy boundary for non-native capture paths. It is not
/// a session owner and it does not decide whether baseline streaming should run;
/// the parent media loop performs that load-time decision.
pub(in crate::plugins::builtin::remote_desktop) struct BaselineMediaInputs<'a> {
    pub(in crate::plugins::builtin::remote_desktop) track: &'a Arc<TrackLocalStaticSample>,
    pub(in crate::plugins::builtin::remote_desktop) ssrc: u32,
    pub(in crate::plugins::builtin::remote_desktop) options: &'a ScreenCaptureOptions,
    pub(in crate::plugins::builtin::remote_desktop) config: &'a BuiltinH264Config,
}

pub(in crate::plugins::builtin::remote_desktop) async fn run_direct_webrtc_recorder_stream(
    inputs: &BaselineMediaInputs<'_>,
    recorder: xcap::VideoRecorder,
    rx: std::sync::mpsc::Receiver<xcap::Frame>,
    done_rx: &mut webrtc::runtime::Receiver<()>,
    stop_rx: &mut watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let BaselineMediaInputs {
        track,
        ssrc,
        options,
        config,
    } = *inputs;
    let mut encoder = build_openh264_encoder(config)?;
    recorder.start()?;
    let mut seq = 0_u64;
    loop {
        if *stop_rx.borrow() || done_rx.try_recv().is_ok() {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(RECORDER_FRAME_TIMEOUT_MS)) {
            Ok(frame) => {
                let frame = latest_recorder_frame(&rx, frame);
                let frame = rgba_bytes_to_rgb_frame(frame.raw, frame.width, frame.height, options)
                    .map(even_rgb_frame)?;
                write_h264_sample(track, ssrc, &mut encoder, &frame, seq, config.fps).await?;
                seq = seq.saturating_add(1);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if stop_rx.has_changed().unwrap_or(false) && *stop_rx.borrow_and_update() {
            break;
        }
    }
    let _ = recorder.stop();
    Ok(())
}

pub(in crate::plugins::builtin::remote_desktop) async fn run_direct_webrtc_polling_stream(
    inputs: &BaselineMediaInputs<'_>,
    entry: &ResourceEntry,
    done_rx: &mut webrtc::runtime::Receiver<()>,
    stop_rx: &mut watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let BaselineMediaInputs {
        track,
        ssrc,
        options,
        config,
    } = *inputs;
    let mut encoder = build_openh264_encoder(config)?;
    let interval = Duration::from_secs_f64(1.0 / config.fps as f64);
    let mut seq = 0_u64;
    loop {
        if *stop_rx.borrow() || done_rx.try_recv().is_ok() {
            break;
        }
        let started = Instant::now();
        let frame = capture_rgb_with_xcap(entry, options).map(even_rgb_frame)?;
        write_h264_sample(track, ssrc, &mut encoder, &frame, seq, config.fps).await?;
        seq = seq.saturating_add(1);
        if let Some(remaining) = interval.checked_sub(started.elapsed()) {
            std::thread::sleep(remaining);
        }
    }
    Ok(())
}
