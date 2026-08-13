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

#[cfg(feature = "native-media")]
use crate::daemon::ability::builtins::resources::media::screen_snapshot::open_display_recorder_with_xcap;
use crate::daemon::ability::builtins::resources::media::screen_snapshot::ScreenCaptureOptions;
use crate::daemon::plugins::remote_desktop::media::encode::BuiltinH264Config;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::target::{
    RemoteAppTargetBinding, RemoteAppTargetError,
};
use crate::daemon::plugins::remote_desktop::transport::media_source::{
    DirectWebRtcMediaSourceFactory, MediaStartRequest, RemoteAppMediaSource,
    RemoteAppMediaSourceFactory,
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

#[derive(Debug)]
struct DirectWebRtcFailureProjection {
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
    let source = DirectWebRtcMediaSourceFactory
        .start_from_binding(&target_binding, MediaStartRequest { config: &config });
    match source {
        Ok(RemoteAppMediaSource::NativeProduction) => {
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
                        let failure =
                            direct_webrtc_native_failure_projection(&err, &target_binding);
                        crate::op_event!(
                            component = remote_desktop,
                            kind = direct_webrtc_native_unavailable,
                            reason = failure.message.clone(),
                        );
                        sessions.mark_direct_webrtc_failed_with_context(
                            &session_id,
                            epoch,
                            &failure.reason,
                            failure.message,
                            failure.context,
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
        Err(err) => {
            let failure = direct_webrtc_target_failure_projection(&err, &target_binding);
            sessions.mark_direct_webrtc_failed_with_context(
                &session_id,
                epoch,
                &failure.reason,
                failure.message,
                failure.context,
            );
            let _ = peer_connection.close().await;
            return;
        }
        Ok(RemoteAppMediaSource::DisplayBaseline) => {}
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

fn direct_webrtc_target_failure_projection(
    target_error: &RemoteAppTargetError,
    target_binding: &RemoteAppTargetBinding,
) -> DirectWebRtcFailureProjection {
    let target_reason = target_error.reason();
    DirectWebRtcFailureProjection {
        reason: target_reason.as_str().to_string(),
        message: target_error.to_string(),
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

fn direct_webrtc_native_failure_projection(
    err: &anyhow::Error,
    target_binding: &RemoteAppTargetBinding,
) -> DirectWebRtcFailureProjection {
    let message = err.to_string();
    let Some(target_error) = err.downcast_ref::<RemoteAppTargetError>() else {
        return DirectWebRtcFailureProjection {
            reason: "native_media_pipeline_failed".to_string(),
            message,
            context: Value::Null,
        };
    };
    direct_webrtc_target_failure_projection(target_error, target_binding)
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use crate::daemon::plugins::remote_desktop::target::{
        RemoteAppTargetResolver, ResourceEntryTargetResolver, TargetResolutionError,
    };

    #[test]
    fn native_target_failure_preserves_frontend_recovery_context() {
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
                    metadata: json!({
                        "platform": "macos",
                        "backend": "macos_core_graphics",
                        "window_id": 7,
                        "pid": 9001,
                        "bundle_id": "com.example.Editor",
                    }),
                    first_seen_at: "2026-06-01T00:00:00Z".to_string(),
                },
                "view_only",
                1,
            )
            .expect("window binding");
        let err = anyhow::Error::new(RemoteAppTargetError::new(
            "remote_desktop.set_description",
            TargetResolutionError::TargetIdentityChanged,
            "live capture target no longer matches committed binding",
        ));

        let failure = direct_webrtc_native_failure_projection(&err, &binding);

        assert_eq!(failure.reason, "target_identity_changed");
        assert_eq!(failure.context["failure_domain"], json!("target"));
        assert_eq!(failure.context["frontend_action"], json!("refresh_targets"));
        assert_eq!(
            failure.context["subject_ura"],
            json!("easynet:///r/acme/resource/window.7")
        );
        assert_eq!(failure.context["target_kind"], json!("window"));
        assert_eq!(failure.context["binding_epoch"], json!(1));
    }

    #[test]
    fn native_non_target_failure_stays_pipeline_failure() {
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
        let err = anyhow::anyhow!("encoder queue closed");

        let failure = direct_webrtc_native_failure_projection(&err, &binding);

        assert_eq!(failure.reason, "native_media_pipeline_failed");
        assert_eq!(failure.message, "encoder queue closed");
        assert!(failure.context.is_null());
    }
}
