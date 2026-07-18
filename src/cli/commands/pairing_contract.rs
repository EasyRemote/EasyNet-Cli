//! EasyNet Hub pairing REST response contract.
//!
//! Pairing credentials belong to the product account and daemon lifecycle.
//! They are deliberately not part of the canonical Axon runtime SDK.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct FederatedPeerEntry {
    pub realm: String,
    #[serde(default)]
    pub peer_hub_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_hub_pubkey: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PairingCredentialEnvelope {
    pub node_id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub trust_level: String,
    #[serde(default)]
    pub device_group: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub auth_binding: String,
    #[serde(default)]
    pub credential_provisioned: bool,
    #[serde(default)]
    pub public_key_registered: bool,
    #[serde(default)]
    pub device_public_key: String,
    #[serde(default)]
    pub device_public_key_fingerprint: String,
    pub credential_token: String,
    pub hub_endpoint: String,
    pub realm: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(default)]
    pub deploy_signature: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub federated_peers: Vec<FederatedPeerEntry>,
    #[serde(default)]
    pub ura: String,
    #[serde(default)]
    pub last_seen_unix_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_credential_requires_product_realm_key() {
        let envelope = serde_json::from_value::<PairingCredentialEnvelope>(serde_json::json!({
            "node_id": "dev-1",
            "credential_token": "cred",
            "hub_endpoint": "https://hub.example",
            "tenant_id": "acme"
        }));
        assert!(envelope.is_err());
    }

    #[test]
    fn pairing_credential_carries_immutable_user_binding() {
        let envelope = serde_json::from_value::<PairingCredentialEnvelope>(serde_json::json!({
            "node_id": "dev-1",
            "credential_token": "cred",
            "hub_endpoint": "https://hub.example",
            "realm": "acme",
            "username": "alice",
            "user_id": "7a0d75be-1c47-44ce-83a1-60bdf14f3a0d"
        }))
        .unwrap();

        assert_eq!(
            envelope.user_id.as_deref(),
            Some("7a0d75be-1c47-44ce-83a1-60bdf14f3a0d")
        );
        assert_eq!(envelope.username.as_deref(), Some("alice"));
    }
}
