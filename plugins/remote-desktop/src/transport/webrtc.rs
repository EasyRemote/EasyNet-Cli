// EasyNet CLI — direct WebRTC transport callbacks
// =================================================
//
// File: plugins/remote-desktop/src/transport/webrtc.rs
// Description: PeerConnection event handling for direct WebRTC transport.

use std::sync::Arc;

use serde_json::{json, Value};
use webrtc::data_channel::DataChannel;
use webrtc::peer_connection::{
    PeerConnection, PeerConnectionEventHandler, RTCIceConnectionState, RTCIceGatheringState,
    RTCPeerConnectionIceErrorEvent, RTCPeerConnectionIceEvent, RTCPeerConnectionState,
};
use webrtc::runtime::Sender;

use crate::daemon::plugins::remote_desktop::input::{
    input_injection_available, record_input_channel_event, run_remote_desktop_input_channel,
    INPUT_DATA_CHANNEL_LABEL,
};
use crate::daemon::plugins::remote_desktop::sdp::remote_ice_candidate_inits;
use crate::daemon::plugins::remote_desktop::session_store::RemoteDesktopSessionStore;
use crate::daemon::plugins::remote_desktop::session_transport_state::TransportEpoch;
use crate::daemon::plugins::remote_desktop::transport::RemoteDesktopTransportManager;

/// Direct WebRTC PeerConnection callback adapter.
///
/// Invariant 1: callback code only projects transport events into session-store
/// methods; it does not own session lifecycle rules.
/// Invariant 2: data-channel input handling is delegated to the input module,
/// so transport callbacks do not parse or inject input frames.
#[derive(Clone)]
pub(in crate::daemon::plugins::remote_desktop) struct DirectWebRtcHandler {
    sessions: Arc<RemoteDesktopSessionStore>,
    transports: Arc<RemoteDesktopTransportManager>,
    session_id: String,
    epoch: TransportEpoch,
    input_policy: Value,
    gather_complete_tx: Sender<()>,
    connected_tx: Sender<()>,
    done_tx: Sender<()>,
}

impl DirectWebRtcHandler {
    pub(in crate::daemon::plugins::remote_desktop) fn new(
        sessions: Arc<RemoteDesktopSessionStore>,
        transports: Arc<RemoteDesktopTransportManager>,
        session_id: String,
        epoch: TransportEpoch,
        input_policy: Value,
        gather_complete_tx: Sender<()>,
        connected_tx: Sender<()>,
        done_tx: Sender<()>,
    ) -> Self {
        Self {
            sessions,
            transports,
            session_id,
            epoch,
            input_policy,
            gather_complete_tx,
            connected_tx,
            done_tx,
        }
    }
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for DirectWebRtcHandler {
    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        if let Ok(candidate) = event.candidate.to_json() {
            eprintln!(
                "[remote-desktop-webrtc] local_candidate={}",
                candidate.candidate
            );
            match serde_json::to_value(candidate) {
                Ok(candidate) => {
                    if let Err(err) = self.sessions.record_local_webrtc_candidate(
                        &self.session_id,
                        self.epoch,
                        candidate,
                    ) {
                        self.sessions.record_webrtc_diagnostic(
                            &self.session_id,
                            self.epoch,
                            "ICE_CANDIDATE_SCHEMA_INVALID",
                            Some(err.to_string()),
                            json!({ "stage": "local_candidate_projection" }),
                        );
                    }
                }
                Err(err) => {
                    self.sessions.record_webrtc_diagnostic(
                        &self.session_id,
                        self.epoch,
                        "ICE_CANDIDATE_SCHEMA_INVALID",
                        Some(err.to_string()),
                        json!({ "stage": "local_candidate_serialization" }),
                    );
                }
            }
        }
    }

    async fn on_ice_candidate_error(&self, event: RTCPeerConnectionIceErrorEvent) {
        let message = format!(
            "{}:{} {} {}",
            event.address, event.port, event.error_code, event.error_text
        );
        eprintln!("[remote-desktop-webrtc] ice_candidate_error={message}");
        self.sessions.record_webrtc_diagnostic(
            &self.session_id,
            self.epoch,
            "ICE_CANDIDATE_ERROR",
            Some(message),
            json!({
                "address": event.address,
                "port": event.port,
                "url": event.url,
                "error_code": event.error_code,
                "error_text": event.error_text,
            }),
        );
    }

    async fn on_ice_connection_state_change(&self, state: RTCIceConnectionState) {
        let state = state.to_string();
        eprintln!("[remote-desktop-webrtc] ice_connection_state={state}");
        self.sessions.record_webrtc_diagnostic(
            &self.session_id,
            self.epoch,
            "ICE_CONNECTION_STATE_CHANGED",
            None,
            json!({ "ice_connection_state": state }),
        );
    }

    async fn on_ice_gathering_state_change(&self, state: RTCIceGatheringState) {
        eprintln!("[remote-desktop-webrtc] ice_gathering_state={state}");
        if state == RTCIceGatheringState::Complete {
            let _ = self.gather_complete_tx.try_send(());
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        eprintln!("[remote-desktop-webrtc] peer_connection_state={state}");
        self.sessions.record_webrtc_diagnostic(
            &self.session_id,
            self.epoch,
            "PEER_CONNECTION_STATE_CHANGED",
            None,
            json!({ "peer_connection_state": state.to_string() }),
        );
        match state {
            RTCPeerConnectionState::Connected => {
                let _ = self.connected_tx.try_send(());
            }
            RTCPeerConnectionState::Failed => {
                self.sessions.mark_direct_webrtc_failed(
                    &self.session_id,
                    self.epoch,
                    "webrtc_peer_connection_failed",
                    "device-side peer connection entered failed".to_string(),
                );
                let _ = self.done_tx.try_send(());
                self.transports
                    .stop_endpoint_if_epoch(&self.session_id, self.epoch);
            }
            RTCPeerConnectionState::Disconnected | RTCPeerConnectionState::Closed => {
                self.sessions.mark_direct_webrtc_failed(
                    &self.session_id,
                    self.epoch,
                    "webrtc_peer_connection_closed",
                    format!("device-side peer connection entered {state}"),
                );
                let _ = self.done_tx.try_send(());
                self.transports
                    .stop_endpoint_if_epoch(&self.session_id, self.epoch);
            }
            _ => {}
        }
    }

    async fn on_data_channel(&self, data_channel: Arc<dyn DataChannel>) {
        let label = data_channel
            .label()
            .await
            .unwrap_or_else(|_| "<unknown>".to_string());
        if label != INPUT_DATA_CHANNEL_LABEL {
            record_input_channel_event(
                &self.sessions,
                &self.session_id,
                self.epoch,
                "INPUT_CHANNEL_REJECTED",
                json!({
                    "label": label,
                    "reason": "unsupported_data_channel_label",
                }),
            );
            let _ = data_channel.close().await;
            return;
        }
        record_input_channel_event(
            &self.sessions,
            &self.session_id,
            self.epoch,
            "INPUT_CHANNEL_OPENING",
            json!({
                "label": label,
                "input_policy": self.input_policy.clone(),
                "input_injection_available": input_injection_available(),
            }),
        );
        let session_id = self.session_id.clone();
        let input_policy = self.input_policy.clone();
        let sessions = Arc::clone(&self.sessions);
        let epoch = self.epoch;
        tokio::spawn(async move {
            run_remote_desktop_input_channel(
                sessions,
                session_id,
                epoch,
                input_policy,
                data_channel,
            )
            .await;
        });
    }
}

/// Apply remote ICE candidate values to a live direct WebRTC endpoint.
///
/// The signaling layer owns JSON argument validation and session authorization.
/// This transport helper owns only the wire-profile projection from accepted
/// candidate JSON into WebRTC candidate objects and the async runtime boundary
/// for `PeerConnection::add_ice_candidate`.
pub(in crate::daemon::plugins::remote_desktop) fn apply_remote_ice_candidate_values(
    transports: &RemoteDesktopTransportManager,
    peer_connection: &Arc<dyn PeerConnection>,
    candidates: &[Value],
) -> anyhow::Result<()> {
    for candidate in candidates {
        for candidate_init in remote_ice_candidate_inits(candidate)? {
            let candidate_label = if candidate_init.candidate.trim().is_empty() {
                "<end-of-candidates>"
            } else {
                candidate_init.candidate.as_str()
            };
            eprintln!("[remote-desktop-webrtc] apply_remote_candidate={candidate_label}");
            transports.block_on(peer_connection.add_ice_candidate(candidate_init))??;
        }
    }
    Ok(())
}

/// Apply all remote ICE candidates already recorded for a session.
///
/// This is used immediately after a direct WebRTC endpoint is created, before
/// later trickled candidates arrive through the add-ICE ability. The session
/// store supplies already-authorized candidate JSON; the transport boundary
/// owns endpoint lookup and WebRTC candidate application.
pub(in crate::daemon::plugins::remote_desktop) fn apply_pending_remote_ice_candidates(
    sessions: &RemoteDesktopSessionStore,
    transports: &RemoteDesktopTransportManager,
    session_id: &str,
    epoch: TransportEpoch,
) -> anyhow::Result<()> {
    let endpoint = transports.endpoint(session_id);
    let Some(endpoint) = endpoint else {
        return Ok(());
    };
    if endpoint.epoch != epoch {
        return Ok(());
    }
    let candidates = {
        let sessions = sessions.lock();
        sessions
            .get(session_id)
            .map(|session| session.remote_ice_candidates())
            .unwrap_or_default()
    };
    apply_remote_ice_candidate_values(transports, &endpoint.peer_connection, &candidates)
}
