// EasyNet CLI — direct WebRTC media loop
// =======================================
//
// File: plugins/remote-desktop/src/transport/webrtc_media.rs
// Description: Media-stream lifecycle for direct WebRTC remote desktop.

use std::sync::Arc;
use std::time::Duration;

use rtc::rtp_transceiver::PayloadType;
use tokio::sync::watch;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::Track;
use webrtc::peer_connection::PeerConnection;

#[cfg(feature = "native-media")]
use crate::daemon::ability::builtins::resources::media::screen_snapshot::open_display_recorder_with_xcap;
use crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenCaptureOptions;
use crate::daemon::plugins::remote_desktop::media::encode::BuiltinH264Config;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::target::RemoteAppTargetBinding;
#[cfg(feature = "native-media")]
use crate::daemon::plugins::remote_desktop::transport::webrtc_baseline_media::run_direct_webrtc_recorder_stream;
use crate::daemon::plugins::remote_desktop::transport::webrtc_baseline_media::{
    run_direct_webrtc_polling_stream, BaselineMediaInputs,
};
#[cfg(target_os = "macos")]
use crate::daemon::plugins::remote_desktop::transport::webrtc_native_media::{
    run_direct_webrtc_native_stream, NativeMediaInputs,
};

/// Owned per-session context handed to the media loop's dedicated thread.
///
/// Invariant 1: the peer connection and track belong to exactly one direct
/// WebRTC media loop.
/// Invariant 2: session state is updated only through `sessions`; this media
/// loop does not mutate session objects directly.
pub(in crate::daemon::plugins::remote_desktop) struct DirectWebRtcSession {
    pub(in crate::daemon::plugins::remote_desktop) session_id: String,
    pub(in crate::daemon::plugins::remote_desktop) epoch: TransportEpoch,
    pub(in crate::daemon::plugins::remote_desktop) peer_connection: Arc<dyn PeerConnection>,
    pub(in crate::daemon::plugins::remote_desktop) track: Arc<TrackLocalStaticSample>,
    /// Payload type selected by the completed offer/answer negotiation.
    pub(in crate::daemon::plugins::remote_desktop) payload_type: PayloadType,
    pub(in crate::daemon::plugins::remote_desktop) target_binding: RemoteAppTargetBinding,
    pub(in crate::daemon::plugins::remote_desktop) options: ScreenCaptureOptions,
    pub(in crate::daemon::plugins::remote_desktop) config: BuiltinH264Config,
}

pub(in crate::daemon::plugins::remote_desktop) async fn run_direct_webrtc_media_loop(
    sessions: Arc<RemoteDesktopSessionStore>,
    session: DirectWebRtcSession,
    mut connected_rx: webrtc::runtime::Receiver<()>,
    mut done_rx: webrtc::runtime::Receiver<()>,
    mut stop_rx: watch::Receiver<bool>,
) {
    let DirectWebRtcSession {
        session_id,
        epoch,
        peer_connection,
        track,
        payload_type,
        target_binding,
        options,
        config,
    } = session;
    loop {
        if *stop_rx.borrow() || done_rx.try_recv().is_ok() {
            let _ = peer_connection.close().await;
            return;
        }
        if connected_rx.try_recv().is_ok() {
            break;
        }
        if stop_rx.has_changed().unwrap_or(false) && *stop_rx.borrow_and_update() {
            let _ = peer_connection.close().await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let Some(ssrc) = track.ssrcs().await.first().copied() else {
        let _ = peer_connection.close().await;
        return;
    };
    #[cfg(target_os = "macos")]
    if config.backend.production_ready() {
        let native_inputs = NativeMediaInputs::new(&track, ssrc, payload_type, &options, &config);
        match run_direct_webrtc_native_stream(
            &sessions,
            &peer_connection,
            &native_inputs,
            &session_id,
            epoch,
            &target_binding,
            &mut done_rx,
            &mut stop_rx,
        )
        .await
        {
            Ok(()) => {
                let _ = peer_connection.close().await;
                return;
            }
            Err(err) => {
                let message = err.to_string();
                crate::op_event!(
                    component = remote_desktop,
                    kind = direct_webrtc_native_unavailable,
                    reason = message.clone(),
                );
                sessions.mark_direct_webrtc_failed(
                    &session_id,
                    epoch,
                    "native_media_pipeline_failed",
                    message,
                );
                let _ = peer_connection.close().await;
                return;
            }
        }
    }

    let baseline_inputs = BaselineMediaInputs {
        track: &track,
        ssrc,
        payload_type,
        options: &options,
        config: &config,
    };
    #[cfg(feature = "native-media")]
    let result = {
        let capture_subject = target_binding.diagnostic_capture_subject().clone();
        let recorder_entry = capture_subject.to_backend_resource_entry();
        if let Ok((recorder, rx)) = open_display_recorder_with_xcap(&recorder_entry) {
            run_direct_webrtc_recorder_stream(
                &sessions,
                &session_id,
                epoch,
                &baseline_inputs,
                recorder,
                rx,
                &mut done_rx,
                &mut stop_rx,
            )
            .await
        } else {
            run_direct_webrtc_polling_stream(
                &sessions,
                &session_id,
                epoch,
                &baseline_inputs,
                &capture_subject,
                &mut done_rx,
                &mut stop_rx,
            )
            .await
        }
    };
    #[cfg(not(feature = "native-media"))]
    let result = run_direct_webrtc_polling_stream(
        &sessions,
        &session_id,
        epoch,
        &baseline_inputs,
        target_binding.diagnostic_capture_subject(),
        &mut done_rx,
        &mut stop_rx,
    )
    .await;
    if let Err(err) = result {
        sessions.mark_direct_webrtc_failed(
            &session_id,
            epoch,
            "baseline_media_pipeline_failed",
            err.to_string(),
        );
        crate::op_event!(
            component = remote_desktop,
            kind = direct_webrtc_media_failed,
            reason = err.to_string(),
        );
    }
    let _ = peer_connection.close().await;
}
