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
    build_session_envelope_open_with_seed, InitialSessionAdmissionProbe, SessionCloseStats,
    SessionError, SessionFrameDispatcher, SessionSigningSeed, SessionUpSender,
    DEVICE_DISPATCH_CONTRACT_VERSION, REASON_BIDI_DOWN_SEQUENCE, SESSION_UP_CHANNEL_CAPACITY,
};

pub(super) struct LiveSessionRun<'a, D: SessionFrameDispatcher> {
    pub(super) client: InvocationClient<Channel>,
    pub(super) hub_endpoint: String,
    pub(super) caller_ura: String,
    pub(super) signing_seed: Option<SessionSigningSeed>,
    pub(super) dispatcher: Arc<D>,
    pub(super) escalation_outbox:
        Option<&'a crate::daemon::invocation::session_escalation::SharedSessionOutbox>,
    pub(super) idle_timeout: Duration,
    pub(super) initial_admission: Option<InitialSessionAdmissionProbe>,
}

pub(super) async fn run_live_session<D: SessionFrameDispatcher>(
    request: LiveSessionRun<'_, D>,
    phase: &mut SessionPhaseTracker,
) -> Result<SessionCloseStats, SessionError> {
    let LiveSessionRun {
        mut client,
        hub_endpoint,
        caller_ura,
        signing_seed,
        dispatcher,
        escalation_outbox,
        idle_timeout,
        initial_admission,
    } = request;

    let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(SESSION_UP_CHANNEL_CAPACITY);
    let outbound_tx = SessionUpSender::new(up_tx.clone());

    let frame0 = build_session_envelope_open_with_seed(&caller_ura, signing_seed);
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
                    apply_session_contract(&frame, &outbound_tx, &hub_endpoint);
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

fn apply_session_contract(frame: &InvokeBidiDown, outbound: &SessionUpSender, hub_endpoint: &str) {
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
    record_connection_state(
        crate::runtime::join_connection_state::JoinConnectionState::ConnectedOnline,
        crate::runtime::join_connection_state::JoinTransition::AdmitPresence,
        "session.contract_negotiated",
    );
}

/// Re-record the process-global join-connection snapshot at `state`, preserving
/// the realm/node/hub identity fields from the latest snapshot (recorded by
/// `cli.start`). The session initiator only knows the live transport result, not
/// the full credential bundle, so it derives the new state from what boot already
/// published rather than reconstructing it.
pub(super) fn record_connection_state(
    state: crate::runtime::join_connection_state::JoinConnectionState,
    transition: crate::runtime::join_connection_state::JoinTransition,
    source: &str,
) {
    let prior = crate::runtime::join_connection_state::latest_snapshot();
    crate::runtime::join_connection_state::record_snapshot(
        crate::runtime::join_connection_state::JoinConnectionSnapshot::from_parts(
            state,
            Some(transition),
            prior.realm,
            prior.node_id,
            prior.hub_endpoint,
            source.to_string(),
        ),
    );
}

struct OutboxGuard {
    outbox: Option<crate::daemon::invocation::session_escalation::SharedSessionOutbox>,
}

impl OutboxGuard {
    fn new(
        outbox: Option<crate::daemon::invocation::session_escalation::SharedSessionOutbox>,
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
