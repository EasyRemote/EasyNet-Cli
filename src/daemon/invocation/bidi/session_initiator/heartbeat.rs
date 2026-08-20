use std::sync::Arc;
use std::time::Duration;

use axon_sdk::pb::axon::v1::BidiControl;
use tonic::transport::Channel;

use super::prelude::{invoke_prelude_unary, signed_prelude_request};
use super::tasks::AbortOnDrop;
use super::{SessionUpSender, SESSION_UP_HEARTBEAT_INTERVAL};
use crate::daemon::federation::client::ability_contract::HeartbeatArgs;
use crate::daemon::federation::read_model::authority_published_abilities::AuthorityPublishedAbilityStore;
use crate::daemon::identity::self_identity::CanonicalSigner;

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

pub(super) fn spawn_federation_heartbeat(
    channel: Channel,
    signer: Arc<dyn CanonicalSigner>,
    authority_published_abilities: Arc<AuthorityPublishedAbilityStore>,
) -> AbortOnDrop {
    AbortOnDrop(tokio::spawn(async move {
        let mut heartbeat_client = crate::daemon::invocation::transport::invocation_client(channel);
        let mut last_error: Option<String> = None;
        loop {
            tokio::time::sleep(FEDERATION_HEARTBEAT_INTERVAL).await;
            match send_federation_heartbeat(
                &mut heartbeat_client,
                signer.as_ref(),
                &authority_published_abilities,
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
/// Refreshes this device's liveness in the hub directory and applies the
/// Authority broadcast abilities diff
/// from the receipt (AXON-RFC-001 v4.1.7) so the local
/// `AuthorityPublishedAbilityStore` stays current between re-dials. Owner
/// projections are event-driven and non-expiring; the retained wire field is
/// sent empty so heartbeat is not a second existence/ownership protocol.
async fn send_federation_heartbeat(
    client: &mut axon_sdk::pb::axon::v1::invocation_client::InvocationClient<Channel>,
    signer: &dyn CanonicalSigner,
    authority_published_abilities: &AuthorityPublishedAbilityStore,
) -> Result<(), tonic::Status> {
    let caller_ura = signer.owner_ura();
    let args = heartbeat_args(authority_published_abilities.revision());
    let arguments = serde_json::to_vec(&args)
        .map_err(|e| tonic::Status::internal(format!("federation.heartbeat serialize: {e}")))?;
    let request =
        signed_prelude_request(signer, caller_ura, "federation.heartbeat", arguments).await?;
    let response = invoke_prelude_unary(client, request, "federation.heartbeat").await?;
    apply_federation_heartbeat_receipt(&response.result, authority_published_abilities)?;
    Ok(())
}

fn heartbeat_args(since_abilities_revision: u64) -> HeartbeatArgs {
    HeartbeatArgs {
        since_abilities_revision,
        refresh_owner_uras: Vec::new(),
    }
}

fn apply_federation_heartbeat_receipt(
    body_bytes: &[u8],
    authority_published_abilities: &AuthorityPublishedAbilityStore,
) -> Result<(), tonic::Status> {
    if body_bytes.is_empty() {
        return Err(tonic::Status::failed_precondition(
            "federation.heartbeat receipt body is empty",
        ));
    }
    let receipt = crate::daemon::federation::client::ability_contract::parse_receipt::<
        crate::daemon::federation::client::ability_contract::HeartbeatReceipt,
    >(body_bytes)
    .map_err(|error| {
        tonic::Status::failed_precondition(format!("federation.heartbeat receipt invalid: {error}"))
    })?;
    authority_published_abilities
        .apply_diff(receipt.authority_abilities_diff)
        .map_err(|error| {
            tonic::Status::failed_precondition(format!(
                "federation.heartbeat Authority-published ability catalog invalid: {error}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::{apply_federation_heartbeat_receipt, heartbeat_args};
    use crate::daemon::ability::descriptors::{AbilityDescriptor, AdmissionAction, Visibility};
    use crate::daemon::federation::client::ability_contract::AuthorityAbilityEntry;
    use crate::daemon::federation::read_model::authority_published_abilities::AuthorityPublishedAbilityStore;

    fn canonical_authority_entry(name: &str) -> AuthorityAbilityEntry {
        AuthorityAbilityEntry {
            name: name.to_string(),
            descriptor: serde_json::to_value(
                AbilityDescriptor::new(
                    name,
                    crate::core::ura::hub_ura("realm"),
                    Visibility::Public,
                    AdmissionAction::Read,
                )
                .expect("canonical realm Authority descriptor"),
            )
            .expect("descriptor json"),
        }
    }

    #[test]
    fn federation_heartbeat_receipt_rejects_empty_or_malformed_body() {
        let store = AuthorityPublishedAbilityStore::new();
        let empty = apply_federation_heartbeat_receipt(&[], &store)
            .expect_err("empty heartbeat receipt must fail closed");
        assert_eq!(empty.code(), tonic::Code::FailedPrecondition);
        assert!(empty.message().contains("receipt body is empty"));

        let malformed = apply_federation_heartbeat_receipt(br#"not-json"#, &store)
            .expect_err("malformed heartbeat receipt must fail closed");
        assert_eq!(malformed.code(), tonic::Code::FailedPrecondition);
        assert!(malformed
            .message()
            .contains("federation.heartbeat receipt invalid"));
    }

    #[test]
    fn federation_heartbeat_receipt_applies_revision_only_diff() {
        let store = AuthorityPublishedAbilityStore::new();
        let body = serde_json::to_vec(&serde_json::json!({
            "membership_status": "active",
            "realm_directory_size": 1,
            "authority_abilities_diff": {
                "revision": 21,
                "added": [],
                "removed": []
            }
        }))
        .expect("heartbeat receipt json");

        apply_federation_heartbeat_receipt(&body, &store)
            .expect("canonical revision-only heartbeat receipt");

        assert_eq!(store.revision(), 21);
        assert!(store.is_empty());
    }

    #[test]
    fn federation_heartbeat_receipt_applies_canonical_added_rows() {
        let store = AuthorityPublishedAbilityStore::new();
        let body = serde_json::to_vec(&serde_json::json!({
            "membership_status": "active",
            "realm_directory_size": 1,
            "authority_abilities_diff": {
                "revision": 22,
                "added": [canonical_authority_entry("test.scope")],
                "removed": []
            }
        }))
        .expect("heartbeat receipt json");

        apply_federation_heartbeat_receipt(&body, &store).expect("canonical heartbeat receipt");

        assert_eq!(store.revision(), 22);
        assert_eq!(store.snapshot()[0].public_name(), "test.scope");
    }

    #[test]
    fn heartbeat_does_not_reintroduce_projection_lease_ownership() {
        let args = heartbeat_args(42);
        assert_eq!(args.since_abilities_revision, 42);
        assert!(args.refresh_owner_uras.is_empty());
    }
}
