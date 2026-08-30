// EasyNet CLI — direct WebRTC media loop
// =======================================
//
// File: plugins/remote-desktop/src/transport/webrtc_media.rs
// Description: Media-stream lifecycle for direct WebRTC remote desktop.

use std::sync::Arc;
use std::time::Duration;

use rtc::rtp_transceiver::PayloadType;
use serde_json::{json, Value};
use tokio::sync::watch;
use webrtc::media_stream::track_local::static_sample::TrackLocalStaticSample;
use webrtc::media_stream::Track;
use webrtc::peer_connection::PeerConnection;
use webrtc::rtp_transceiver::RtpSender;

use crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenCaptureOptions;
use crate::daemon::plugins::remote_desktop::media::encode::BuiltinH264Config;
use crate::daemon::plugins::remote_desktop::session_events::WebRtcFailureEventKind;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
use crate::daemon::plugins::remote_desktop::target::FrontendAction;
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, RemoteAppTargetError, TargetResolutionError,
};
use crate::daemon::plugins::remote_desktop::transport::media_source::{
    start_remote_app_media_source, DirectWebRtcMediaSourceFactory, MediaStartRequest,
    RemoteAppMediaSource,
};
#[cfg(not(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
)))]
use crate::daemon::plugins::remote_desktop::transport::webrtc_baseline_media::{
    run_direct_webrtc_polling_stream, BaselineMediaInputs,
};
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
use crate::daemon::plugins::remote_desktop::transport::webrtc_hosted_media::{
    run_direct_webrtc_hosted_stream, HostedMediaHostFailure,
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
    pub(in crate::daemon::plugins::remote_desktop) video_sender: Arc<dyn RtpSender>,
    /// Payload type selected by the completed offer/answer negotiation.
    pub(in crate::daemon::plugins::remote_desktop) payload_type: PayloadType,
    pub(in crate::daemon::plugins::remote_desktop) audio_track: Option<Arc<TrackLocalStaticSample>>,
    pub(in crate::daemon::plugins::remote_desktop) audio_payload_type: Option<PayloadType>,
    pub(in crate::daemon::plugins::remote_desktop) target_binding: RemoteAppTargetBinding,
    pub(in crate::daemon::plugins::remote_desktop) options: ScreenCaptureOptions,
    pub(in crate::daemon::plugins::remote_desktop) config: BuiltinH264Config,
}

/// Borrowed media inputs shared by the daemon-owned WebRTC carrier and its
/// selected platform media strategy.
#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
pub(in crate::daemon::plugins::remote_desktop) struct HostedMediaInputs<'a> {
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

/// Session-owned lifecycle and state projection shared by every direct-WebRTC
/// media strategy.
///
/// Capture/encode strategies own only their media mechanics. Stop observation,
/// readiness publication, transport-epoch fencing, and diagnostic projection
/// remain identical regardless of the selected capture backend.
pub(in crate::daemon::plugins::remote_desktop) struct DirectWebRtcMediaExecution<'a> {
    sessions: &'a RemoteDesktopSessionStore,
    session_id: &'a str,
    epoch: TransportEpoch,
    done_rx: &'a mut webrtc::runtime::Receiver<()>,
    stop_rx: &'a mut watch::Receiver<bool>,
}

impl<'a> DirectWebRtcMediaExecution<'a> {
    fn new(
        sessions: &'a RemoteDesktopSessionStore,
        session_id: &'a str,
        epoch: TransportEpoch,
        done_rx: &'a mut webrtc::runtime::Receiver<()>,
        stop_rx: &'a mut watch::Receiver<bool>,
    ) -> Self {
        Self {
            sessions,
            session_id,
            epoch,
            done_rx,
            stop_rx,
        }
    }

    pub(in crate::daemon::plugins::remote_desktop) fn should_stop(&mut self) -> bool {
        if *self.stop_rx.borrow() || self.done_rx.try_recv().is_ok() {
            return true;
        }
        self.stop_rx.has_changed().unwrap_or(false) && *self.stop_rx.borrow_and_update()
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn sessions(
        &self,
    ) -> &RemoteDesktopSessionStore {
        self.sessions
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn session_id(&self) -> &str {
        self.session_id
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn epoch(&self) -> TransportEpoch {
        self.epoch
    }

    pub(in crate::daemon::plugins::remote_desktop) fn mark_media_ready(&self) {
        self.sessions
            .mark_direct_webrtc_media_ready(self.session_id, self.epoch);
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(in crate::daemon::plugins::remote_desktop) fn record_pipeline_stats(&self, stats: Value) {
        self.sessions
            .record_media_pipeline_stats(self.session_id, self.epoch, stats);
    }
}

#[derive(Debug)]
struct DirectWebRtcFailureProjection {
    event_kind: WebRtcFailureEventKind,
    reason: String,
    message: String,
    context: Value,
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
        video_sender,
        payload_type,
        audio_track,
        audio_payload_type,
        target_binding,
        options,
        config,
    } = session;
    loop {
        if *stop_rx.borrow() || done_rx.try_recv().is_ok() {
            let _ = close_peer_connection_bounded(&peer_connection).await;
            return;
        }
        if connected_rx.try_recv().is_ok() {
            break;
        }
        if stop_rx.has_changed().unwrap_or(false) && *stop_rx.borrow_and_update() {
            let _ = close_peer_connection_bounded(&peer_connection).await;
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let Some(ssrc) = track.ssrcs().await.first().copied() else {
        let _ = close_peer_connection_bounded(&peer_connection).await;
        return;
    };
    let mut execution =
        DirectWebRtcMediaExecution::new(&sessions, &session_id, epoch, &mut done_rx, &mut stop_rx);
    let source = start_remote_app_media_source(
        &DirectWebRtcMediaSourceFactory,
        &target_binding,
        MediaStartRequest { config: &config },
    );
    match source {
        Ok(RemoteAppMediaSource::NativeProduction) => {
            #[cfg(not(all(
                feature = "native-media",
                any(target_os = "linux", target_os = "macos", target_os = "windows")
            )))]
            {
                sessions.mark_direct_webrtc_generation_failed(
                    &session_id,
                    epoch,
                    "native_media_unavailable",
                    "direct WebRTC native media is not available on this platform".to_string(),
                );
                let _ = close_peer_connection_bounded(&peer_connection).await;
                return;
            }
        }
        Err(err) => {
            let failure = direct_webrtc_target_failure_projection(&err, &target_binding);
            sessions.mark_direct_webrtc_generation_failed_with_context(
                &session_id,
                epoch,
                failure.event_kind,
                &failure.reason,
                failure.message,
                failure.context,
            );
            let _ = close_peer_connection_bounded(&peer_connection).await;
            return;
        }
        Ok(RemoteAppMediaSource::XcapBaseline) => {}
    }

    #[cfg(not(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )))]
    let baseline_inputs = BaselineMediaInputs {
        track: &track,
        video_sender: &video_sender,
        ssrc,
        payload_type,
        audio_track: audio_track.as_ref(),
        audio_payload_type,
        options: &options,
        config: &config,
        target_binding: &target_binding,
    };
    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    let hosted_inputs = HostedMediaInputs {
        track: &track,
        video_sender: &video_sender,
        ssrc,
        payload_type,
        audio_track: audio_track.as_ref(),
        audio_payload_type,
        options: &options,
        config: &config,
        target_binding: &target_binding,
    };
    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    let result = run_direct_webrtc_hosted_stream(&mut execution, &hosted_inputs).await;
    #[cfg(not(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )))]
    let result = run_direct_webrtc_polling_stream(
        &mut execution,
        &baseline_inputs,
        target_binding.diagnostic_capture_subject(),
    )
    .await;
    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    if let Err(err) = result {
        let failure = direct_webrtc_hosted_failure_projection(&err, &target_binding);
        sessions.mark_direct_webrtc_generation_failed_with_context(
            &session_id,
            epoch,
            failure.event_kind,
            &failure.reason,
            failure.message,
            failure.context,
        );
        crate::op_event!(
            component = remote_desktop,
            kind = direct_webrtc_media_failed,
            reason = err.to_string(),
        );
    }
    #[cfg(not(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )))]
    if let Err(err) = result {
        sessions.mark_direct_webrtc_generation_failed(
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
    let _ = close_peer_connection_bounded(&peer_connection).await;
}

const PEER_CONNECTION_CLOSE_DEADLINE: Duration = Duration::from_secs(2);

pub(in crate::daemon::plugins::remote_desktop) async fn close_peer_connection_bounded(
    peer_connection: &Arc<dyn PeerConnection>,
) -> bool {
    close_peer_connection_until(peer_connection, PEER_CONNECTION_CLOSE_DEADLINE).await
}

pub(in crate::daemon::plugins::remote_desktop) async fn close_peer_connection_until(
    peer_connection: &Arc<dyn PeerConnection>,
    deadline: Duration,
) -> bool {
    match tokio::time::timeout(deadline, peer_connection.close()).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            eprintln!("[remote-desktop-webrtc] peer connection close failed: {error}");
            false
        }
        Err(_) => {
            eprintln!(
                "[remote-desktop-webrtc] peer connection close exceeded {}ms; media task will terminate without waiting further",
                deadline.as_millis()
            );
            false
        }
    }
}

fn direct_webrtc_target_failure_projection(
    target_error: &RemoteAppTargetError,
    target_binding: &RemoteAppTargetBinding,
) -> DirectWebRtcFailureProjection {
    let target_reason = target_error.reason();
    direct_webrtc_target_reason_projection(target_reason, target_error.to_string(), target_binding)
}

fn direct_webrtc_target_reason_projection(
    target_reason: TargetResolutionError,
    message: String,
    target_binding: &RemoteAppTargetBinding,
) -> DirectWebRtcFailureProjection {
    DirectWebRtcFailureProjection {
        event_kind: WebRtcFailureEventKind::MediaSourceLost,
        reason: target_reason.as_str().to_string(),
        message,
        context: json!({
            "failure_domain": "target",
            "target_reason": target_reason.as_str(),
            "frontend_action": target_reason.frontend_action().as_str(),
            "subject_ura": target_binding.subject_ura(),
            "target_kind": target_binding.target_kind().as_str(),
            "binding_id": target_binding.binding_id(),
            "binding_epoch": target_binding.binding_epoch(),
            "target_identity_epoch": target_binding.target_identity_epoch(),
            "target_geometry_revision": target_binding.target_geometry_revision(),
            "media_source_epoch": target_binding.media_source_epoch(),
        }),
    }
}

#[cfg(all(
    feature = "native-media",
    any(target_os = "linux", target_os = "macos", target_os = "windows")
))]
fn direct_webrtc_hosted_failure_projection(
    err: &anyhow::Error,
    target_binding: &RemoteAppTargetBinding,
) -> DirectWebRtcFailureProjection {
    if let Some(failure) = err.downcast_ref::<HostedMediaHostFailure>() {
        if let Some(target_reason) = failure.target_reason() {
            return direct_webrtc_target_reason_projection(
                target_reason,
                failure.to_string(),
                target_binding,
            );
        }
    }
    DirectWebRtcFailureProjection {
        event_kind: WebRtcFailureEventKind::TransportFailed,
        reason: "hosted_media_pipeline_failed".to_string(),
        message: err.to_string(),
        context: json!({
            "failure_domain": "transport",
            "reason_code": TargetResolutionError::TransportRouteUnavailable.as_str(),
            "recoverability": "retry_session",
            "frontend_action": FrontendAction::RetrySession.as_str(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    use easynet_remoteapp_native_protocol::media_session::FailureReason;
    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    use serde_json::json;

    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    use crate::daemon::plugins::remote_desktop::target::ResourceEntryTargetResolver;
    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    use crate::daemon::plugins::remote_desktop::test_support::live_remote_target_metadata;

    #[test]
    fn media_execution_observes_transport_and_session_stop_signals() {
        let sessions = RemoteDesktopSessionStore::new();
        let (_done_tx, mut done_rx) = webrtc::runtime::channel::<()>(1);
        let (stop_tx, mut stop_rx) = watch::channel(false);
        let mut execution = DirectWebRtcMediaExecution::new(
            &sessions,
            "rd-media-execution",
            TransportEpoch::new(1),
            &mut done_rx,
            &mut stop_rx,
        );

        assert!(!execution.should_stop());
        stop_tx.send(true).expect("stop receiver remains active");
        assert!(execution.should_stop());

        let (_stop_tx, mut stop_rx) = watch::channel(false);
        let (done_tx_2, mut done_rx) = webrtc::runtime::channel::<()>(1);
        let mut execution = DirectWebRtcMediaExecution::new(
            &sessions,
            "rd-media-execution",
            TransportEpoch::new(2),
            &mut done_rx,
            &mut stop_rx,
        );
        done_tx_2
            .try_send(())
            .expect("transport completion signal records");
        assert!(execution.should_stop());
    }

    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    #[test]
    fn hosted_target_failure_preserves_frontend_recovery_context() {
        let binding = ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &ResourceEntry {
                    resource_ura: "easynet:///r/acme/resource/window.7".to_string(),
                    owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
                    kind: ResourceType::Window,
                    binding: ResourceBinding::LocalDevice,
                    hardware_id: "window:7".to_string(),
                    display_name: "Editor".to_string(),
                    metadata: live_remote_target_metadata(json!({
                        "platform": "macos",
                        "backend": "macos_core_graphics",
                        "window_id": 7,
                        "pid": 9001,
                        "bundle_id": "com.example.Editor",
                    })),
                    first_seen_at: "2026-06-01T00:00:00Z".to_string(),
                },
                "view_only",
                1,
            )
            .expect("window binding");
        let err = anyhow::Error::new(HostedMediaHostFailure::new(
            FailureReason::TargetInvalidated,
            "live capture target no longer matches committed binding".into(),
        ));

        let failure = direct_webrtc_hosted_failure_projection(&err, &binding);

        assert_eq!(failure.reason, "target_stale");
        assert_eq!(failure.event_kind, WebRtcFailureEventKind::MediaSourceLost);
        assert_eq!(failure.context["failure_domain"], json!("target"));
        assert_eq!(failure.context["frontend_action"], json!("refresh_targets"));
        assert_eq!(
            failure.context["subject_ura"],
            json!("easynet:///r/acme/resource/window.7")
        );
        assert_eq!(failure.context["target_kind"], json!("window"));
        assert_eq!(failure.context["binding_epoch"], json!(1));
    }

    #[cfg(all(
        feature = "native-media",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    ))]
    #[test]
    fn hosted_non_target_failure_stays_pipeline_failure() {
        let binding = ResourceEntryTargetResolver
            .resolve_for_session(
                "remote_desktop.create_session",
                &ResourceEntry {
                    resource_ura: "easynet:///r/acme/resource/display.1".to_string(),
                    owner_agent: "easynet:///r/acme/agent/device.01DEV.media".to_string(),
                    kind: ResourceType::Display,
                    binding: ResourceBinding::LocalDevice,
                    hardware_id: "display:1".to_string(),
                    display_name: "Display".to_string(),
                    metadata: json!({
                        "display_id": 1,
                    }),
                    first_seen_at: "2026-06-01T00:00:00Z".to_string(),
                },
                "view_only",
                1,
            )
            .expect("display binding");
        let err = anyhow::Error::new(HostedMediaHostFailure::new(
            FailureReason::EncoderUnavailable,
            "encoder queue closed".into(),
        ));

        let failure = direct_webrtc_hosted_failure_projection(&err, &binding);

        assert_eq!(failure.reason, "hosted_media_pipeline_failed");
        assert_eq!(failure.event_kind, WebRtcFailureEventKind::TransportFailed);
        assert!(failure.message.contains("encoder queue closed"));
        assert_eq!(failure.context["failure_domain"], json!("transport"));
    }
}
