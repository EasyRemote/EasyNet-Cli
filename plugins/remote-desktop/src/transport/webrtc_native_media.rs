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
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, RemoteAppTargetError,
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
    let mut active_media_source_epoch = target_binding.media_source_epoch();

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
    loop {
        if *stop_rx.borrow() || done_rx.try_recv().is_ok() {
            break;
        }
        active_media_source_epoch = apply_pending_media_rebind(
            sessions,
            &capture,
            session_id,
            epoch,
            active_media_source_epoch,
        )?;
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

fn apply_pending_media_rebind(
    sessions: &RemoteDesktopSessionStore,
    capture: &ScreenCaptureKitStream,
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
    let next_target = target_for_binding(ABILITY_SET_DESCRIPTION, &next_binding)
        .map_err(|err| fail_pending_media_rebind(sessions, session_id, epoch, &err))?;
    let capture_proof = capture
        .update_content_filter(ABILITY_SET_DESCRIPTION, next_target)
        .map_err(|err| fail_pending_media_rebind(sessions, session_id, epoch, &err))?;
    if sessions.commit_pending_media_rebind_for_session(
        session_id,
        epoch,
        next_binding.binding_epoch(),
        next_binding.media_source_epoch(),
        capture_proof,
    ) {
        Ok(next_binding.media_source_epoch())
    } else {
        Ok(active_media_source_epoch)
    }
}

fn fail_pending_media_rebind(
    sessions: &RemoteDesktopSessionStore,
    session_id: &str,
    epoch: TransportEpoch,
    err: &RemoteAppTargetError,
) -> RemoteAppTargetError {
    sessions.fail_pending_media_rebind_for_session(
        session_id,
        epoch,
        err.reason(),
        err.to_string(),
    );
    err.clone()
}

fn record_media_pipeline_stats(
    sessions: &RemoteDesktopSessionStore,
    session_id: &str,
    epoch: TransportEpoch,
    stats: serde_json::Value,
) {
    sessions.record_media_pipeline_stats(session_id, epoch, stats);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    use crate::daemon::plugins::remote_desktop::constants::{
        direct_webrtc_endpoint_ura, TRANSPORT_WEBRTC,
    };
    use crate::daemon::plugins::remote_desktop::session::RemoteDesktopSession;
    use crate::daemon::plugins::remote_desktop::target::{
        AppWindowSetProof, TargetGeometry, TargetResolutionError,
    };
    use crate::daemon::plugins::remote_desktop::target_tracking::TargetObservation;
    use crate::daemon::plugins::remote_desktop::test_support::test_application_session_init;

    #[test]
    fn native_media_rebind_failure_projects_typed_target_lifecycle() {
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
                .record_target_observation(TargetObservation::ApplicationWindowSetChanged {
                    app_window_set: AppWindowSetProof::new(
                        42,
                        Some("com.example.Editor".to_string()),
                        Some(9001),
                        vec![10, 11, 12],
                    ),
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
        let returned = fail_pending_media_rebind(&store, session_id, epoch, &err);

        assert_eq!(
            returned.reason(),
            TargetResolutionError::ScreenCaptureKitFilterFailed
        );
        store.with_sessions(|sessions| {
            let session = sessions.get(session_id).expect("session stored");
            assert_eq!(session.target_tracking_state()["status"], json!("lost"));
            assert!(session.pending_media_rebind_binding().is_none());
            let event = session
                .events()
                .into_iter()
                .find(|event| event["event_type"] == json!("TARGET_REBIND_FAILED"))
                .expect("target rebind failure event");
            assert_eq!(
                event["reason_code"],
                json!("screencapturekit_filter_failed")
            );
            assert_eq!(event["payload"]["failure_domain"], json!("target"));
            assert_eq!(
                event["payload"]["frontend_action"],
                json!("show_unsupported")
            );
            assert_eq!(
                event["payload"]["pending_media_source_epoch"],
                json!(pending.media_source_epoch())
            );
        });
    }
}
