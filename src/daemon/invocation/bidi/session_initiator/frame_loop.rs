use std::sync::Arc;
use std::time::{Duration, Instant};

use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;
use easynet_axon::pb::axon::v1::{InvokeBidiDown, InvokeBidiUp};
use futures::StreamExt as _;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::Channel;

use super::heartbeat::SessionUpHeartbeatTask;
use super::supervisor::{DeviceSessionPhase, SessionPhaseTracker};
use super::{
    build_session_envelope_open, connection_state::project_connection_state,
    InitialSessionAdmissionProbe, SessionCloseStats, SessionConnectionStateSink, SessionError,
    SessionFrameDispatcher, SessionUpSender, DEVICE_DISPATCH_CONTRACT_VERSION,
    REASON_BIDI_DOWN_SEQUENCE, SESSION_UP_CHANNEL_CAPACITY,
};
use crate::daemon::identity::self_identity::CanonicalSigner;

pub(super) struct LiveSessionRun<'a, D: SessionFrameDispatcher> {
    pub(super) client: InvocationClient<Channel>,
    pub(super) hub_endpoint: String,
    pub(super) signer: Arc<dyn CanonicalSigner>,
    pub(super) dispatcher: Arc<D>,
    pub(super) escalation_outbox:
        Option<&'a crate::daemon::invocation::bidi::session_escalation::SharedSessionOutbox>,
    pub(super) idle_timeout: Duration,
    pub(super) initial_admission: Option<InitialSessionAdmissionProbe>,
    pub(super) connection_state_sink: Arc<dyn SessionConnectionStateSink>,
}

pub(super) async fn run_live_session<D: SessionFrameDispatcher>(
    request: LiveSessionRun<'_, D>,
    phase: &mut SessionPhaseTracker,
) -> Result<SessionCloseStats, SessionError> {
    let LiveSessionRun {
        mut client,
        hub_endpoint,
        signer,
        dispatcher,
        escalation_outbox,
        idle_timeout,
        initial_admission,
        connection_state_sink,
    } = request;
    let caller_ura = signer.owner_ura().to_string();

    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(SESSION_UP_CHANNEL_CAPACITY);
    let outbound_tx = SessionUpSender::new(up_tx.clone());

    let frame0 = build_session_envelope_open(signer.as_ref())
        .await
        .map_err(|source| SessionError::SigningIdentity {
            owner_ura: caller_ura.clone(),
            operation: "session.open frame 0",
            source,
        })?;
    up_tx
        .send(frame0)
        .await
        .map_err(|_| SessionError::SendFailed("frame 0 EnvelopeOpen"))?;

    let outbound = ReceiverStream::new(up_rx);
    let response =
        client
            .invoke_bidi(outbound)
            .await
            .map_err(|status| SessionError::HubRejected {
                endpoint: hub_endpoint.clone(),
                status,
            })?;

    let mut down_stream = response.into_inner();
    let opened_at = Instant::now();
    phase.transition(DeviceSessionPhase::Live, "bidi_accepted");
    crate::op_event!(
        component = session,
        kind = bidi_opened,
        hub_endpoint = hub_endpoint,
        caller_ura = caller_ura,
        message = "awaiting down-stream frames",
    );
    if let Some(probe) = &initial_admission {
        probe.admitted();
    }

    if let Some(outbox) = escalation_outbox {
        outbox.set(outbound_tx.clone());
    }
    let _outbox_guard = OutboxGuard::new(escalation_outbox.cloned());
    let _up_heartbeat = SessionUpHeartbeatTask::spawn(
        outbound_tx.clone(),
        hub_endpoint.clone(),
        caller_ura.clone(),
    );
    let mut expected_down_sequence = 0_u64;

    loop {
        let frame_result = match tokio::time::timeout(idle_timeout, down_stream.next()).await {
            Ok(Some(frame_result)) => frame_result,
            Ok(None) => break,
            Err(_elapsed) => {
                return Err(SessionError::IdleTimeout {
                    endpoint: hub_endpoint,
                    timeout: idle_timeout,
                });
            }
        };

        match frame_result {
            Ok(frame) => {
                if frame.sequence != expected_down_sequence {
                    return Err(SessionError::DownStreamSequence {
                        endpoint: hub_endpoint,
                        expected: expected_down_sequence,
                        actual: frame.sequence,
                        reason: REASON_BIDI_DOWN_SEQUENCE,
                    });
                }
                expected_down_sequence = expected_down_sequence.saturating_add(1);
                if frame.sequence == 0 {
                    apply_session_contract(
                        &frame,
                        &outbound_tx,
                        &hub_endpoint,
                        connection_state_sink.as_ref(),
                    );
                }
                if let Err(err) = dispatcher.handle_down(frame, &outbound_tx).await {
                    let err_msg = format!("{err}");
                    crate::op_event!(
                        component = session,
                        kind = frame_dispatch_error,
                        error = err_msg,
                        message = "continuing",
                    );
                }
            }
            Err(status) => {
                return Err(SessionError::DownStreamError {
                    endpoint: hub_endpoint,
                    status,
                });
            }
        }
    }

    Ok(SessionCloseStats {
        uptime: opened_at.elapsed(),
        frames_received: expected_down_sequence,
    })
}

fn apply_session_contract(
    frame: &InvokeBidiDown,
    outbound: &SessionUpSender,
    hub_endpoint: &str,
    connection_state_sink: &dyn SessionConnectionStateSink,
) {
    use easynet_axon::pb::axon::v1::invoke_bidi_down::Payload;
    let Some(Payload::Receipt(receipt)) = frame.payload.as_ref() else {
        return;
    };
    let Ok(body) = serde_json::from_slice::<serde_json::Value>(&receipt.payload) else {
        return;
    };
    let Some(contract) = body.get("session_contract") else {
        return;
    };
    let version = contract
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0) as u32;
    let negotiated = version.min(DEVICE_DISPATCH_CONTRACT_VERSION);
    outbound.set_negotiated_contract(negotiated);
    let displaced_prior = contract
        .get("displaced_prior")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    crate::op_event!(
        component = session,
        kind = session_contract_negotiated,
        hub_endpoint = hub_endpoint,
        version = negotiated,
        displaced_prior = displaced_prior,
    );
    // The hub accepted `session.open` and returned the session contract — this
    // is the FIRST moment presence is truly admitted on the hub. Promote the
    // connection snapshot to ConnectedOnline here, not at daemon boot: `cli.start`
    // records the honest "self-session opening" (J500) state, and only this
    // hub-confirmed contract earns FRONTEND_CONNECTED. Without this, `doctor`
    // would under-report a healthy session as still "opening".
    project_connection_state(
        connection_state_sink,
        crate::daemon::boot::join_connection_state::JoinConnectionState::ConnectedOnline,
        crate::daemon::boot::join_connection_state::JoinTransition::AdmitPresence,
        "session.contract_negotiated",
    );
}

struct OutboxGuard {
    outbox: Option<crate::daemon::invocation::bidi::session_escalation::SharedSessionOutbox>,
}

impl OutboxGuard {
    fn new(
        outbox: Option<crate::daemon::invocation::bidi::session_escalation::SharedSessionOutbox>,
    ) -> Self {
        Self { outbox }
    }
}

impl Drop for OutboxGuard {
    fn drop(&mut self) {
        if let Some(outbox) = &self.outbox {
            outbox.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;
    use crate::daemon::boot::join_connection_state::{
        load_snapshot, record_snapshot, JoinConnectionSnapshot, JoinConnectionState, JoinTransition,
    };
    use crate::daemon::persistence::config::Credentials;

    fn credentials() -> Credentials {
        Credentials {
            node_id: "dev-1".to_string(),
            credential_token: "token".to_string(),
            hub_endpoint: "https://127.0.0.1:50443".to_string(),
            realm: "test".to_string(),
            deploy_signature: String::new(),
            hub_api_base: Some("http://127.0.0.1:8080".to_string()),
            username: Some("alice".to_string()),
            user_id: Some("alice".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        }
    }

    #[test]
    fn session_contract_projection_promotes_connection_snapshot() {
        let _home = HomeGuard::new();
        let credentials = credentials();
        record_snapshot(JoinConnectionSnapshot::from_credentials(
            JoinConnectionState::SelfSessionAdmissionPending,
            Some(JoinTransition::OpenSelfSession),
            &credentials,
            "test.start",
        ));

        assert!(project_connection_state(
            &super::super::PersistentSessionConnectionStateSink,
            JoinConnectionState::ConnectedOnline,
            JoinTransition::AdmitPresence,
            "session.contract_negotiated",
        ));

        let snapshot = load_snapshot().expect("snapshot");
        assert_eq!(snapshot.state, "FRONTEND_CONNECTED");
        assert_eq!(snapshot.state_code, "J800");
        assert_eq!(
            snapshot.transition_id.as_deref(),
            Some("T10_ADMIT_PRESENCE")
        );
        assert_eq!(snapshot.source, "session.contract_negotiated");
    }
}
