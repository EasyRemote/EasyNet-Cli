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
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, RemoteDesktopTargetKind,
};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectWebRtcMediaSourcePlan {
    NativeProduction,
    DisplayBaseline,
    Unsupported { reason: &'static str },
}

impl DirectWebRtcMediaSourcePlan {
    fn for_binding(config: &BuiltinH264Config, binding: &RemoteAppTargetBinding) -> Self {
        Self::from_backend_state(config.backend.production_ready(), binding.target_kind())
    }

    fn from_backend_state(production_ready: bool, target_kind: RemoteDesktopTargetKind) -> Self {
        if production_ready {
            #[cfg(target_os = "macos")]
            {
                return Self::NativeProduction;
            }
            #[cfg(not(target_os = "macos"))]
            {
                return Self::Unsupported {
                    reason: "native_media_unavailable",
                };
            }
        }
        if target_kind == RemoteDesktopTargetKind::Display {
            Self::DisplayBaseline
        } else {
            Self::Unsupported {
                reason: "display_fallback_forbidden",
            }
        }
    }

    fn unsupported_message(self, binding: &RemoteAppTargetBinding) -> String {
        match self {
            Self::Unsupported {
                reason: "display_fallback_forbidden",
            } => format!(
                "direct WebRTC baseline capture is display-only and cannot satisfy a {} target binding",
                binding.target_kind().as_str()
            ),
            Self::Unsupported {
                reason: "native_media_unavailable",
            } => format!(
                "direct WebRTC native media is required for a production-ready {} target binding on this platform",
                binding.target_kind().as_str()
            ),
            Self::Unsupported { reason } => format!(
                "direct WebRTC media source is unavailable for {} target binding; reason={reason}",
                binding.target_kind().as_str()
            ),
            Self::NativeProduction | Self::DisplayBaseline => String::new(),
        }
    }
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
    match DirectWebRtcMediaSourcePlan::for_binding(&config, &target_binding) {
        DirectWebRtcMediaSourcePlan::NativeProduction => {
            #[cfg(target_os = "macos")]
            {
                let native_inputs =
                    NativeMediaInputs::new(&track, ssrc, payload_type, &options, &config);
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
            #[cfg(not(target_os = "macos"))]
            {
                sessions.mark_direct_webrtc_failed(
                    &session_id,
                    epoch,
                    "native_media_unavailable",
                    "direct WebRTC native media is not available on this platform".to_string(),
                );
                let _ = peer_connection.close().await;
                return;
            }
        }
        DirectWebRtcMediaSourcePlan::Unsupported { reason } => {
            let plan = DirectWebRtcMediaSourcePlan::Unsupported { reason };
            sessions.mark_direct_webrtc_failed(
                &session_id,
                epoch,
                reason,
                plan.unsupported_message(&target_binding),
            );
            let _ = peer_connection.close().await;
            return;
        }
        DirectWebRtcMediaSourcePlan::DisplayBaseline => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_native_window_and_application_sources_fail_closed_before_display_baseline() {
        for target_kind in [
            RemoteDesktopTargetKind::Window,
            RemoteDesktopTargetKind::Application,
        ] {
            assert_eq!(
                DirectWebRtcMediaSourcePlan::from_backend_state(false, target_kind),
                DirectWebRtcMediaSourcePlan::Unsupported {
                    reason: "display_fallback_forbidden"
                }
            );
        }
    }

    #[test]
    fn display_source_may_use_baseline_when_native_backend_is_not_selected() {
        assert_eq!(
            DirectWebRtcMediaSourcePlan::from_backend_state(
                false,
                RemoteDesktopTargetKind::Display
            ),
            DirectWebRtcMediaSourcePlan::DisplayBaseline
        );
    }
}
