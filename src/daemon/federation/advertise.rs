//! Canonical outbound federation payload projections.
//!
//! Transport and session lifecycle are owned by `session_initiator`. This
//! module only encodes persisted owner-projection facts for that transport.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::daemon::federation::hosted_agent_publication::HostedAgentIncarnationId;
use crate::daemon::federation::read_model::owner_projection::{
    AbilityProjectionSummary, OwnerProjectionPublication,
};

/// Hub acknowledgement for one complete owner-projection replacement.
///
/// Transport success is not publication success: the Hub may return a valid
/// response with `ack=false`, and an acknowledged response must account for
/// the exact complete set sent by the Device (including zero for tombstones).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct AdvertiseAbilitiesResponse {
    pub(crate) ack: bool,
    pub(crate) count: usize,
}

/// Decode and validate the terminal Hub receipt for
/// `federation.advertise_abilities`.
///
/// Every outbound publication path uses this function so reconnect, prelude,
/// hot start, and removal tombstones share one acceptance rule.
pub(crate) fn decode_advertise_abilities_response(
    result_bytes: &[u8],
    expected_count: usize,
) -> Result<AdvertiseAbilitiesResponse, String> {
    let response: AdvertiseAbilitiesResponse = serde_json::from_slice(result_bytes)
        .map_err(|error| format!("decode federation.advertise_abilities response: {error}"))?;
    if !response.ack {
        return Err(format!(
            "Hub rejected federation.advertise_abilities publication (accepted_count={}, expected_count={expected_count})",
            response.count
        ));
    }
    if response.count != expected_count {
        return Err(format!(
            "Hub acknowledged federation.advertise_abilities with count mismatch (accepted_count={}, expected_count={expected_count})",
            response.count
        ));
    }
    Ok(response)
}

#[derive(Debug, Serialize)]
struct AdvertiseAgentArgs<'a> {
    agent_ura: &'a str,
    incarnation_id: &'a HostedAgentIncarnationId,
}

pub(crate) fn advertise_agent_payload(
    agent_ura: &str,
    incarnation_id: &HostedAgentIncarnationId,
) -> Result<Value, String> {
    serde_json::to_value(AdvertiseAgentArgs {
        agent_ura,
        incarnation_id,
    })
    .map_err(|error| format!("encode advertise_agent args: {error}"))
}

pub(crate) fn advertise_agent_payload_bytes(
    agent_ura: &str,
    incarnation_id: &HostedAgentIncarnationId,
) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&advertise_agent_payload(agent_ura, incarnation_id)?)
        .map_err(|error| format!("serialize advertise_agent args: {error}"))
}

#[derive(Debug, Serialize)]
struct AdvertiseAbilitiesArgs<'a> {
    agent_ura: &'a str,
    owner_ura: &'a str,
    host_device_ura: &'a str,
    generation: u64,
    projection_revision: u64,
    projection_digest: &'a str,
    lease_expires_unix_ms: i64,
    ability_summaries: &'a [AbilityProjectionSummary],
}

pub(crate) fn advertise_abilities_payload(
    agent_ura: &str,
    projection: &OwnerProjectionPublication,
) -> Result<Value, String> {
    serde_json::to_value(AdvertiseAbilitiesArgs {
        agent_ura,
        owner_ura: &projection.owner_ura,
        host_device_ura: &projection.host_device_ura,
        generation: projection.generation,
        projection_revision: projection.projection_revision,
        projection_digest: &projection.projection_digest,
        lease_expires_unix_ms: projection.lease_expires_unix_ms,
        ability_summaries: &projection.ability_summaries,
    })
    .map_err(|error| format!("encode advertise_abilities args: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::federation::read_model::owner_projection::{
        AbilityCallableSummary, PurgeProjectionDelivery,
    };

    #[test]
    fn advertise_abilities_response_requires_positive_ack() {
        let error = decode_advertise_abilities_response(br#"{"ack":false,"count":0}"#, 0)
            .expect_err("negative acknowledgement must fail publication");

        assert!(error.contains("Hub rejected"), "{error}");
    }

    #[test]
    fn advertise_abilities_response_requires_exact_accepted_count() {
        let error = decode_advertise_abilities_response(br#"{"ack":true,"count":1}"#, 2)
            .expect_err("partial acknowledgement must fail publication");

        assert!(error.contains("count mismatch"), "{error}");
        assert!(error.contains("accepted_count=1"), "{error}");
        assert!(error.contains("expected_count=2"), "{error}");
    }

    #[test]
    fn advertise_abilities_response_accepts_exact_tombstone_count() {
        let response = decode_advertise_abilities_response(br#"{"ack":true,"count":0}"#, 0)
            .expect("zero-count tombstone acknowledgement");

        assert_eq!(
            response,
            AdvertiseAbilitiesResponse {
                ack: true,
                count: 0
            }
        );
    }

    #[test]
    fn advertise_agent_payload_carries_only_hub_registration_command_fields() {
        let incarnation_id = HostedAgentIncarnationId::parse("1".repeat(32)).unwrap();
        let payload =
            advertise_agent_payload("easynet:///r/localhost/agent/dev.worker", &incarnation_id)
                .expect("hosted agent advertise payload");

        assert_eq!(
            payload["agent_ura"],
            "easynet:///r/localhost/agent/dev.worker"
        );
        assert_eq!(payload["incarnation_id"], "1".repeat(32));
        assert_eq!(payload.as_object().unwrap().len(), 2);
    }

    #[test]
    fn advertise_abilities_payload_carries_generation() {
        let projection = OwnerProjectionPublication {
            owner_ura: "easynet:///r/localhost/device/dev-1".to_string(),
            host_device_ura: "easynet:///r/localhost/device/dev-1".to_string(),
            generation: 7,
            projection_revision: 3,
            projection_digest: "sha256:projection".to_string(),
            lease_expires_unix_ms: 123_456,
            purge_delivery: None::<PurgeProjectionDelivery>,
            ability_summaries: vec![AbilityProjectionSummary {
                ability_ura: "easynet:///r/localhost/ability/device.dev-1.echo".to_string(),
                owner_ura: "easynet:///r/localhost/device/dev-1".to_string(),
                namespace: "device".to_string(),
                local_name: "echo".to_string(),
                descriptor_revision: "sha256:descriptor".to_string(),
                schema_ref: None,
                schema_hash: None,
                policy_ref: "visibility:PUBLIC".to_string(),
                route_summary_ref: None,
                tags: Vec::new(),
                callable_summary: AbilityCallableSummary::new(
                    "device.echo",
                    "device.echo",
                    crate::daemon::ability::descriptors::CallMode::Rpc,
                    crate::daemon::ability::descriptors::ReceiptSemantics::Operational,
                    Vec::new(),
                    Default::default(),
                ),
            }],
        };

        let payload =
            advertise_abilities_payload("easynet:///r/localhost/agent/alice.agent", &projection)
                .expect("projection payload serializes");
        assert_eq!(payload["generation"], 7);
        assert_eq!(payload["projection_revision"], 3);
    }
}
