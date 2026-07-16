//! Canonical outbound federation payload projections.
//!
//! Transport and session lifecycle are owned by `session_initiator`. This
//! module only encodes persisted owner-projection facts for that transport.

use serde::Serialize;
use serde_json::Value;

use crate::daemon::federation::read_model::owner_projection::{
    AbilityProjectionSummary, OwnerProjectionPublication,
};

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
                callable_summary: AbilityCallableSummary::default(),
            }],
        };

        let payload =
            advertise_abilities_payload("easynet:///r/localhost/agent/alice.agent", &projection)
                .expect("projection payload serializes");
        assert_eq!(payload["generation"], 7);
        assert_eq!(payload["projection_revision"], 3);
    }
}
