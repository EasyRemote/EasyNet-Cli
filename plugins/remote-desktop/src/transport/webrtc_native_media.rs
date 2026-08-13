// EasyNet CLI — direct WebRTC native media path
// ==============================================
//
// File: plugins/remote-desktop/src/transport/webrtc_native_media.rs
// Description: macOS ScreenCaptureKit + VideoToolbox strategy for direct WebRTC.

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use rtc::media::Sample;
use serde_json::json;
use tokio::sync::watch;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::peer_connection::PeerConnection;

use crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenCaptureOptions;
use crate::daemon::plugins::remote_desktop::constants::ABILITY_SET_DESCRIPTION;
use crate::daemon::plugins::remote_desktop::media::encode::BuiltinH264Config;
use crate::daemon::plugins::remote_desktop::media::native::{
    is_webrtc_sender_backpressure, latest_native_rtp_units, native_capture_dimensions,
    native_rtp_sample_duration, webrtc_cmtime, webrtc_stats_snapshot, NativeAdaptiveBitrate,
    NativeLatencyStats,
};
use crate::daemon::plugins::remote_desktop::screencapturekit_capture::{
    target_for_binding, CapturedFrame, ScreenCaptureKitStream,
};
use crate::daemon::plugins::remote_desktop::session::now_ms;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::target::RemoteAppTargetBinding;
use crate::daemon::plugins::remote_desktop::target_observer::{
    observe_session_target_once, PlatformTargetObservationProvider,
};
use crate::daemon::plugins::remote_desktop::videotoolbox_encoder::VideoToolboxEncoder;

/// Immutable inputs for the macOS native direct-WebRTC strategy.
///
/// This type is the native strategy boundary. It is not a session owner, does
/// not choose whether native capture is enabled, and does not update session
/// lifecycle state directly.
pub(in crate::daemon::plugins::remote_desktop) struct NativeMediaInputs<'a> {
    track: &'a Arc<TrackLocalStaticSample>,
    ssrc: u32,
    payload_type: u8,
    options: &'a ScreenCaptureOptions,
    config: &'a BuiltinH264Config,
}

impl<'a> NativeMediaInputs<'a> {
    /// Builds immutable inputs after the parent loop has resolved WebRTC SSRC.
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        track: &'a Arc<TrackLocalStaticSample>,
        ssrc: u32,
        payload_type: u8,
        options: &'a ScreenCaptureOptions,
        config: &'a BuiltinH264Config,
    ) -> Self {
        Self {
            track,
            ssrc,
            payload_type,
            options,
            config,
        }
    }
}

pub(in crate::daemon::plugins::remote_desktop) async fn run_direct_webrtc_native_stream(
    sessions: &RemoteDesktopSessionStore,
    peer_connection: &Arc<dyn PeerConnection>,
    inputs: &NativeMediaInputs<'_>,
    session_id: &str,
    epoch: TransportEpoch,
    target_binding: &RemoteAppTargetBinding,
    done_rx: &mut webrtc::runtime::Receiver<()>,
    stop_rx: &mut watch::Receiver<bool>,
) -> anyhow::Result<()> {
    use std::sync::Arc as StdArc;

    let NativeMediaInputs {
        track,
        ssrc,
        payload_type,
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
    let sink: crate::daemon::plugins::remote_desktop::screencapturekit_capture::FrameSink =
        StdArc::new(move |frame: CapturedFrame| {
            let pts = frame.pts;
            let duration = webrtc_cmtime(1, fps);
            let _ = encoder_for_sink.encode(&frame.image_buffer, pts, duration);
        });

    let capture = ScreenCaptureKitStream::start(
        ABILITY_SET_DESCRIPTION,
        capture_target,
        req_width,
        req_height,
        fps,
        sink,
    )?;

    let frame_dur = Duration::from_secs_f64(1.0 / fps as f64);
    let mut written_units = 0_u64;
    let mut written_keyframes = 0_u64;
    let mut written_bytes = 0_u64;
    let mut rtp_stale_units_dropped = 0_u64;
    let mut rtp_sender_backpressure_drops = 0_u64;
    let mut last_stats_at = Instant::now();
    let mut bitrate_controller = NativeAdaptiveBitrate::new(config.bitrate_kbps);
    let mut latency_stats = NativeLatencyStats::default();
    let mut decoder_primed = false;
    let mut last_written_pts_ms: Option<u64> = None;
    let target_observer = PlatformTargetObservationProvider;
    let mut last_target_observation_at = Instant::now();
    loop {
        if *stop_rx.borrow() || done_rx.try_recv().is_ok() {
            break;
        }
        if last_target_observation_at.elapsed() >= Duration::from_millis(250) {
            observe_session_target_once(sessions, session_id, epoch, &target_observer);
            last_target_observation_at = Instant::now();
        }
        let (units, stale_dropped) = latest_native_rtp_units(encoder.poll(), decoder_primed);
        rtp_stale_units_dropped = rtp_stale_units_dropped.saturating_add(stale_dropped as u64);
        for unit in units {
            let bytes_len = unit.annexb.len() as u64;
            let is_keyframe = unit.is_keyframe;
            let encode_submitted_at_ms = unit.encode_submitted_at_ms;
            let encoded_at_ms = unit.encoded_at_ms;
            let encode_latency_ms = unit.encode_latency_ms;
            let sample_duration =
                native_rtp_sample_duration(last_written_pts_ms, unit.pts_ms, frame_dur);
            let rtp_write_started_ms = now_ms();
            let write_result = track
                .sample_writer(ssrc, payload_type)
                .write_sample(&Sample {
                    data: Bytes::from(unit.annexb),
                    duration: sample_duration,
                    ..Default::default()
                })
                .await;
            if let Err(err) = write_result {
                if is_webrtc_sender_backpressure(&err) {
                    rtp_sender_backpressure_drops = rtp_sender_backpressure_drops.saturating_add(1);
                    continue;
                }
                return Err(err.into());
            }
            last_written_pts_ms = Some(unit.pts_ms);
            latency_stats.record_encoded_unit(
                encode_submitted_at_ms,
                encoded_at_ms,
                encode_latency_ms,
                rtp_write_started_ms,
                now_ms(),
            );
            if is_keyframe {
                if !decoder_primed {
                    sessions.mark_direct_webrtc_media_ready(session_id, epoch);
                }
                decoder_primed = true;
            }
            written_units = written_units.saturating_add(1);
            written_bytes = written_bytes.saturating_add(bytes_len);
            if is_keyframe {
                written_keyframes = written_keyframes.saturating_add(1);
            }
        }
        if last_stats_at.elapsed() >= Duration::from_secs(1) {
            let stats = encoder.stats();
            let (webrtc_stats, available_outgoing_bitrate_bps) =
                webrtc_stats_snapshot(peer_connection).await;
            if let Some(next_bitrate) = bitrate_controller.update(
                stats.input_dropped_frames,
                stats
                    .output_dropped_units
                    .saturating_add(rtp_stale_units_dropped)
                    .saturating_add(rtp_sender_backpressure_drops),
                stats.queued_units,
                stats.in_flight_frames,
                available_outgoing_bitrate_bps,
            ) {
                if let Err(err) = encoder.set_bitrate_kbps(next_bitrate) {
                    crate::op_event!(
                        component = remote_desktop,
                        kind = native_bitrate_adaptation_failed,
                        reason = err.to_string(),
                    );
                }
            }
            record_media_pipeline_stats(
                sessions,
                session_id,
                epoch,
                json!({
                    "path": "native_webrtc",
                    "backend_id": config.backend.backend_id(),
                    "capture_api": config.backend.capture_api(),
                    "encoder_name": config.backend.encoder(),
                    "carrier": config.backend.carrier(),
                    "target_fps": fps,
                    "target_bitrate_kbps": config.bitrate_kbps,
                    "width": req_width,
                    "height": req_height,
                    "adaptive_bitrate": {
                        "current_kbps": bitrate_controller.current_kbps,
                        "target_kbps": bitrate_controller.target_kbps,
                        "min_kbps": bitrate_controller.min_kbps,
                    },
                    "webrtc_stats": webrtc_stats,
                    "latency_stats": latency_stats.to_json(),
                    "rtp_units_written": written_units,
                    "rtp_keyframes_written": written_keyframes,
                    "rtp_bytes_written": written_bytes,
                    "rtp_stale_units_dropped": rtp_stale_units_dropped,
                    "rtp_sender_backpressure_drops": rtp_sender_backpressure_drops,
                    "encoder_stats": {
                        "submitted_frames": stats.submitted_frames,
                        "input_dropped_frames": stats.input_dropped_frames,
                        "output_dropped_units": stats.output_dropped_units,
                        "emitted_units": stats.emitted_units,
                        "queued_units": stats.queued_units,
                        "in_flight_frames": stats.in_flight_frames,
                        "max_in_flight_frames": stats.max_in_flight_frames,
                        "configured_bitrate_kbps": stats.configured_bitrate_kbps,
                    },
                }),
            );
            last_stats_at = Instant::now();
        }
        if stop_rx.has_changed().unwrap_or(false) && *stop_rx.borrow_and_update() {
            break;
        }
        tokio::select! {
            _ = encoder_wakeup.notified() => {}
            _ = tokio::time::sleep(Duration::from_millis(2)) => {}
        }
    }
    capture.stop();
    let stats = encoder.stats();
    let (webrtc_stats, _) = webrtc_stats_snapshot(peer_connection).await;
    record_media_pipeline_stats(
        sessions,
        session_id,
        epoch,
        json!({
            "path": "native_webrtc",
            "backend_id": config.backend.backend_id(),
            "terminal": true,
            "rtp_units_written": written_units,
            "rtp_keyframes_written": written_keyframes,
            "rtp_bytes_written": written_bytes,
            "rtp_stale_units_dropped": rtp_stale_units_dropped,
            "rtp_sender_backpressure_drops": rtp_sender_backpressure_drops,
            "adaptive_bitrate": {
                "current_kbps": bitrate_controller.current_kbps,
                "target_kbps": bitrate_controller.target_kbps,
                "min_kbps": bitrate_controller.min_kbps,
            },
            "webrtc_stats": webrtc_stats,
            "latency_stats": latency_stats.to_json(),
            "encoder_stats": {
                "submitted_frames": stats.submitted_frames,
                "input_dropped_frames": stats.input_dropped_frames,
                "output_dropped_units": stats.output_dropped_units,
                "emitted_units": stats.emitted_units,
                "queued_units": stats.queued_units,
                "in_flight_frames": stats.in_flight_frames,
                "max_in_flight_frames": stats.max_in_flight_frames,
                "configured_bitrate_kbps": stats.configured_bitrate_kbps,
            },
        }),
    );
    Ok(())
}

fn record_media_pipeline_stats(
    sessions: &RemoteDesktopSessionStore,
    session_id: &str,
    epoch: TransportEpoch,
    stats: serde_json::Value,
) {
    sessions.record_media_pipeline_stats(session_id, epoch, stats);
}
