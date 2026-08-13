// EasyNet CLI — direct WebRTC negotiation service
// ===============================================
//
// File: plugins/remote-desktop/src/transport/webrtc_negotiation.rs
// Description: Direct WebRTC remote-offer negotiation orchestration.

use std::sync::Arc;

use serde_json::Value;

use crate::daemon::ability::dispatch::EnvelopeContext;
use crate::daemon::plugins::remote_desktop::constants::{
    ABILITY_SET_DESCRIPTION, DIRECT_WEBRTC_ENDPOINT_PREFIX, REASON_SESSION_NOT_FOUND,
};
use crate::daemon::plugins::remote_desktop::input::input_policy_for_binding;
use crate::daemon::plugins::remote_desktop::media::{
    webrtc_transport_backend_for_binding, MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID,
};
use crate::daemon::plugins::remote_desktop::request::{
    bitrate_kbps_from_video_constraints, capture_options_from_video_constraints,
    frame_queue_depth_from_video_constraints, RemoteDesktopVideoConstraints,
};
use crate::daemon::plugins::remote_desktop::runtime::RemoteDesktopPlugin;
use crate::daemon::plugins::remote_desktop::session_lifecycle::ensure_session_control_access;
use crate::daemon::plugins::remote_desktop::target::RemoteAppTargetBinding;
use crate::daemon::plugins::remote_desktop::transport::{
    start_direct_webrtc_endpoint, StartDirectWebRtcEndpointRequest,
};

/// Parsed remote-offer negotiation request.
///
/// What this is NOT: raw ability argument parsing. The handler validates
/// `session_id`, `side`, `description`, and offer SDP before constructing this
/// request. This service owns the transport-side start/rollback sequence.
pub(in crate::daemon::plugins::remote_desktop) struct RemoteOfferNegotiation {
    pub(in crate::daemon::plugins::remote_desktop) plugin: Arc<RemoteDesktopPlugin>,
    pub(in crate::daemon::plugins::remote_desktop) access_env: EnvelopeContext,
    pub(in crate::daemon::plugins::remote_desktop) access_args: Value,
    pub(in crate::daemon::plugins::remote_desktop) session_id: String,
    pub(in crate::daemon::plugins::remote_desktop) side: String,
    pub(in crate::daemon::plugins::remote_desktop) description: Value,
    pub(in crate::daemon::plugins::remote_desktop) offer_sdp: String,
}

/// Complete direct WebRTC negotiation and return the affected session id.
///
/// Invariant 1: failed SDP/endpoint setup does not commit remote/local
/// description state.
///
/// Invariant 2: once an endpoint is started, the session is looked up and
/// access-checked again before committing the answer. If the second check
/// fails, the endpoint is stopped before returning the error.
///
/// Invariant 3: unavailable capture backend is an auditable transport-blocked
/// session event, not a silent fallback.
pub(in crate::daemon::plugins::remote_desktop) fn negotiate_remote_offer(
    request: RemoteOfferNegotiation,
) -> anyhow::Result<String> {
    let start_params = collect_start_params(&request)?;
    let (session_id, target_binding, video, input_policy, offer_sdp) = start_params;
    let Some(media_backend) = webrtc_transport_backend_for_binding(&target_binding) else {
        mark_backend_unavailable(&request, &session_id)?;
        return Ok(session_id);
    };

    let options = capture_options_from_video_constraints(&video)?;
    let target_bitrate_kbps = bitrate_kbps_from_video_constraints(&video);
    let max_frame_queue_depth = frame_queue_depth_from_video_constraints(&video);
    let input_policy = input_policy_for_binding(input_policy, &target_binding);
    let epoch = request.plugin.transport_manager().allocate_epoch();
    request
        .plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<()> {
            let session = sessions.get_mut(&session_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "{ABILITY_SET_DESCRIPTION}: session {session_id:?} disappeared before WebRTC negotiation; reason={REASON_SESSION_NOT_FOUND}"
                )
            })?;
            ensure_session_control_access(
                &request.plugin,
                ABILITY_SET_DESCRIPTION,
                &request.access_env,
                &request.access_args,
                session,
            )?;
            session.begin_webrtc_negotiation(epoch);
            Ok(())
        })?;
    let answer = match start_direct_webrtc_endpoint(StartDirectWebRtcEndpointRequest {
        sessions: request.plugin.session_store(),
        transports: request.plugin.transport_manager(),
        session_id: session_id.clone(),
        epoch,
        target_binding,
        options,
        target_bitrate_kbps,
        max_frame_queue_depth,
        input_policy,
        offer_sdp,
    }) {
        Ok(answer) => answer,
        Err(error) => {
            request.plugin.session_store().mark_direct_webrtc_failed(
                &session_id,
                epoch,
                "webrtc_endpoint_setup_failed",
                error.to_string(),
            );
            return Err(error);
        }
    };

    commit_started_endpoint(
        &request,
        &session_id,
        epoch,
        answer,
        media_backend.backend_id(),
        media_backend.production_ready(),
    )?;
    Ok(session_id)
}

fn collect_start_params(
    request: &RemoteOfferNegotiation,
) -> anyhow::Result<(
    String,
    RemoteAppTargetBinding,
    RemoteDesktopVideoConstraints,
    Value,
    String,
)> {
    request
        .plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<_> {
            let session = sessions.get_mut(&request.session_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "{ABILITY_SET_DESCRIPTION}: session {:?} not found; reason={REASON_SESSION_NOT_FOUND}",
                    request.session_id
                )
            })?;
            ensure_session_control_access(
                &request.plugin,
                ABILITY_SET_DESCRIPTION,
                &request.access_env,
                &request.access_args,
                session,
            )?;
            Ok((
                session.session_id().to_string(),
                session.target_binding().clone(),
                session.video().clone(),
                session.input_policy().to_value(),
                request.offer_sdp.clone(),
            ))
        })
}

fn mark_backend_unavailable(
    request: &RemoteOfferNegotiation,
    session_id: &str,
) -> anyhow::Result<()> {
    request
        .plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<()> {
            let session = sessions.get_mut(session_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "{ABILITY_SET_DESCRIPTION}: session {session_id:?} disappeared during media transport gate check; reason={REASON_SESSION_NOT_FOUND}"
                )
            })?;
            ensure_session_control_access(
                &request.plugin,
                ABILITY_SET_DESCRIPTION,
                &request.access_env,
                &request.access_args,
                session,
            )?;
            session.set_description(&request.side, request.description.clone())?;
            session.mark_transport_blocked(
                "webrtc_transport_backend_unavailable",
                MACOS_SCK_VIDEOTOOLBOX_BACKEND_ID,
            );
            Ok(())
        })
}

fn commit_started_endpoint(
    request: &RemoteOfferNegotiation,
    session_id: &str,
    epoch: crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch,
    answer: Value,
    backend_id: &str,
    production_ready: bool,
) -> anyhow::Result<()> {
    request
        .plugin
        .session_store()
        .with_sessions(|sessions| -> anyhow::Result<()> {
            let session = match sessions.get_mut(session_id) {
                Some(session) => session,
                None => {
                    request
                        .plugin
                        .transport_manager()
                        .stop_endpoint_if_epoch(session_id, epoch);
                    anyhow::bail!(
                        "{ABILITY_SET_DESCRIPTION}: session {session_id:?} disappeared during WebRTC setup; reason={REASON_SESSION_NOT_FOUND}"
                    );
                }
            };
            if let Err(err) = ensure_session_control_access(
                &request.plugin,
                ABILITY_SET_DESCRIPTION,
                &request.access_env,
                &request.access_args,
                session,
            ) {
                request
                    .plugin
                    .transport_manager()
                    .stop_endpoint_if_epoch(session_id, epoch);
                return Err(err);
            }
            if session.transport_epoch() != Some(epoch.value()) {
                request
                    .plugin
                    .transport_manager()
                    .stop_endpoint_if_epoch(session_id, epoch);
                anyhow::bail!(
                    "{ABILITY_SET_DESCRIPTION}: transport epoch {} was superseded before answer commit",
                    epoch.value()
                );
            }
            session.set_description(&request.side, request.description.clone())?;
            session.set_local_webrtc_answer(
                epoch,
                answer,
                backend_id,
                production_ready,
                format!("{DIRECT_WEBRTC_ENDPOINT_PREFIX}{session_id}"),
            );
            Ok(())
        })
}
