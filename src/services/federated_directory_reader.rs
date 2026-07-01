// EasyNet CLI — federated directory reader
// ========================================
//
// File: src/services/federated_directory_reader.rs
// Description: Product-layer read boundary for federation discovery.
//
// `services::invocation_transport::federation_invoke` is gated behind
// `axon-pb` because it depends on tonic and SDK-owned protobuf types.
// Ability discovery is a product read path: it can compile in a
// minimal local-only build, but federation-specific callers must see
// an explicit capability error rather than a fabricated empty directory.

use serde_json::Value;

/// Read the federated directory through the local daemon's
/// `federation.discover` ability.
#[cfg(feature = "axon-pb")]
pub fn read_federated_directory(agent_ura_filter: Option<&str>) -> anyhow::Result<Vec<Value>> {
    crate::services::invocation_transport::federation_invoke::invoke_federation_discover(
        agent_ura_filter,
    )
}

#[cfg(feature = "axon-pb")]
pub fn read_federated_directory_filtered(
    agent_ura_filter: Option<&str>,
    local_user_id_filter: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    crate::services::invocation_transport::federation_invoke::invoke_federation_discover_filtered(
        agent_ura_filter,
        local_user_id_filter,
    )
}

#[cfg(not(feature = "axon-pb"))]
pub fn read_federated_directory(_agent_ura_filter: Option<&str>) -> anyhow::Result<Vec<Value>> {
    Err(feature_unavailable_error("federation.discover"))
}

#[cfg(not(feature = "axon-pb"))]
pub fn read_federated_directory_filtered(
    _agent_ura_filter: Option<&str>,
    _local_user_id_filter: Option<&str>,
) -> anyhow::Result<Vec<Value>> {
    Err(feature_unavailable_error("federation.discover"))
}

#[cfg(not(feature = "axon-pb"))]
fn feature_unavailable_error(surface: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "{surface}: this binary was built without the `axon-pb` feature, so daemon-backed \
         federation discovery is unavailable"
    )
}

#[cfg(all(test, not(feature = "axon-pb")))]
mod tests {
    use super::*;

    #[test]
    fn feature_off_reports_unavailable_instead_of_empty_success() {
        let err = read_federated_directory(None).unwrap_err();
        assert!(
            err.to_string().contains("without the `axon-pb` feature"),
            "{err}"
        );
    }

    #[test]
    fn feature_off_filtered_reports_same_capability_error() {
        let err = read_federated_directory_filtered(None, None).unwrap_err();
        assert!(
            err.to_string().contains("without the `axon-pb` feature"),
            "{err}"
        );
    }
}
