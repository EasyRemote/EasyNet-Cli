use std::sync::Arc;
use std::time::Duration;

use easynet_axon::pb::axon::v1::BidiControl;
use tonic::transport::Channel;

use super::prelude::{invoke_prelude_unary, signed_prelude_request};
use super::tasks::AbortOnDrop;
use super::SessionSigningSeed;
use super::{SessionUpSender, SESSION_UP_HEARTBEAT_INTERVAL};
use crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore;

pub(super) struct SessionUpHeartbeatTask {
    handle: tokio::task::JoinHandle<()>,
}

impl SessionUpHeartbeatTask {
    pub(super) fn spawn(sender: SessionUpSender, hub_endpoint: String, caller_ura: String) -> Self {
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(SESSION_UP_HEARTBEAT_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            // First tick fires immediately; consume it so the first
            // keepalive is sent after one full heartbeat window.
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Err(err) = sender.send_control(BidiControl::default()).await {
                    let err_msg = format!("{err}");
                    crate::op_event!(
                        component = session,
                        kind = up_heartbeat_send_failed,
                        caller_ura = caller_ura,
                        hub_endpoint = hub_endpoint,
                        error = err_msg,
                        message = "stopping heartbeat task",
                    );
                    break;
                }
            }
        });
        Self { handle }
    }
}

impl Drop for SessionUpHeartbeatTask {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Cadence of the session-lifetime `federation.heartbeat` loop. The
/// hub-side sweeper demotes an Active directory record to Suspended
/// after 15 s without a heartbeat (3× this cadence), and the Web UI
/// renders anything non-active as offline — this tick is what keeps
/// a healthy device's directory entry green between session re-dials.
const FEDERATION_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Hub-side cap on the owner-projection lease batch
/// (`hub_profile/heartbeat.rs::MAX_HEARTBEAT_LEASE_REFRESH_OWNERS`).
/// A batch over the cap is rejected outright — which would silently
/// kill device liveness — so the sender truncates instead.
const MAX_HEARTBEAT_LEASE_REFRESH_OWNERS: usize = 64;

pub(super) fn spawn_federation_heartbeat(
    channel: Channel,
    caller_ura: String,
    signing_seed: Option<SessionSigningSeed>,
    hub_published_abilities: Arc<HubPublishedAbilityStore>,
) -> AbortOnDrop {
    AbortOnDrop(tokio::spawn(async move {
        let mut heartbeat_client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel);
        let mut last_error: Option<String> = None;
        loop {
            tokio::time::sleep(FEDERATION_HEARTBEAT_INTERVAL).await;
            match send_federation_heartbeat(
                &mut heartbeat_client,
                &caller_ura,
                signing_seed,
                &hub_published_abilities,
            )
            .await
            {
                Ok(()) => {
                    if last_error.take().is_some() {
                        crate::op_event!(
                            component = session,
                            kind = federation_heartbeat_recovered,
                        );
                    }
                }
                Err(status) => {
                    let msg = format!("{status}");
                    // Dedupe identical consecutive failures — at a 5 s
                    // cadence a hub outage would otherwise write 12
                    // identical lines per minute.
                    if last_error.as_deref() != Some(msg.as_str()) {
                        crate::op_event!(
                            component = session,
                            kind = federation_heartbeat_failed,
                            error = msg,
                        );
                    }
                    last_error = Some(msg);
                }
            }
        }
    }))
}

/// One `federation.heartbeat` unary over the session channel.
///
/// Refreshes this device's `last_heartbeat_unix_ms` in the hub
/// directory plus the owner-projection leases this daemon has
/// published (RFC-005), and applies the hub-broadcast abilities diff
/// from the receipt (AXON-RFC-001 v4.1.7) so the local
/// `HubPublishedAbilityStore` stays current between re-dials. Wire
/// shape mirrors the hub's `hub_profile/heartbeat.rs::HeartbeatArgs`;
/// the hub keys the record refresh on the envelope caller URA and
/// auto-includes it in the lease batch, so an empty batch after a
/// cursor-load failure still refreshes device liveness.
async fn send_federation_heartbeat(
    client: &mut easynet_axon::pb::axon::v1::invocation_client::InvocationClient<Channel>,
    caller_ura: &str,
    signing_seed: Option<SessionSigningSeed>,
    hub_published_abilities: &HubPublishedAbilityStore,
) -> Result<(), tonic::Status> {
    let mut refresh_owner_uras =
        crate::daemon::federation::read_model::owner_projection::heartbeat_refresh_owner_uras()
            .unwrap_or_default();
    refresh_owner_uras.truncate(MAX_HEARTBEAT_LEASE_REFRESH_OWNERS);
    let body = serde_json::json!({
        "since_abilities_revision": hub_published_abilities.revision(),
        "refresh_owner_uras": refresh_owner_uras,
    });
    let arguments = serde_json::to_vec(&body)
        .map_err(|e| tonic::Status::internal(format!("federation.heartbeat serialize: {e}")))?;
    let request = signed_prelude_request(
        caller_ura,
        caller_ura,
        "federation.heartbeat",
        arguments,
        signing_seed,
    )?;
    let response = invoke_prelude_unary(client, request, "federation.heartbeat").await?;
    let body_bytes = response.result;
    if !body_bytes.is_empty() {
        if let Ok(receipt) = serde_json::from_slice::<
            crate::daemon::federation::client::ability_contract::HeartbeatReceipt,
        >(&body_bytes)
        {
            let diff = receipt.hub_abilities_diff;
            if !diff.added.is_empty() || !diff.removed.is_empty() {
                hub_published_abilities.apply_diff(diff);
            }
        }
    }
    Ok(())
}
