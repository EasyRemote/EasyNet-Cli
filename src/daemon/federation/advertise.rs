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
        projection_revision: projection.projection_revision,
        projection_digest: &projection.projection_digest,
        lease_expires_unix_ms: projection.lease_expires_unix_ms,
        ability_summaries: &projection.ability_summaries,
    })
    .map_err(|error| format!("encode advertise_abilities args: {error}"))
}
