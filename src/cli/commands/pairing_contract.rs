//! EasyNet Hub pairing REST response contract.
//!
//! Pairing credentials belong to the product account and daemon lifecycle.
//! They are deliberately not part of the canonical Axon runtime SDK.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct FederatedPeerEntry {
    pub realm: String,
    pub peer_hub_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_hub_pubkey: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairingCredentialEnvelope {
    pub node_id: String,
    pub display_name: String,
    pub state: String,
    pub trust_level: String,
    pub device_group: String,
    pub os: String,
    pub arch: String,
    pub auth_binding: String,
    pub credential_provisioned: bool,
    pub public_key_registered: bool,
    pub device_public_key: String,
    pub device_public_key_fingerprint: String,
    pub credential_token: String,
    pub hub_endpoint: String,
    pub realm: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub deploy_signature: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub federated_peers: Vec<FederatedPeerEntry>,
    pub ura: String,
    pub last_seen_unix_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_pairing_envelope_json() -> serde_json::Value {
        serde_json::json!({
            "node_id": "dev-1",
            "display_name": "Workstation",
            "state": "paired",
            "trust_level": "trusted",
            "device_group": "default",
            "os": "macos",
            "arch": "arm64",
            "auth_binding": "user",
            "credential_provisioned": true,
            "public_key_registered": true,
            "device_public_key": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "device_public_key_fingerprint": "sha256:device",
            "credential_token": "cred",
            "hub_endpoint": "https://hub.example",
            "realm": "acme",
            "username": "alice",
            "user_id": "7a0d75be-1c47-44ce-83a1-60bdf14f3a0d",
            "deploy_signature": "deploy-signature",
            "federated_peers": [],
            "ura": "easynet:///r/acme/device/dev-1",
            "last_seen_unix_ms": 1
        })
    }

    #[test]
    fn pairing_credential_rejects_retired_tenant_id_alias() {
        let mut body = canonical_pairing_envelope_json();
        body["tenant_id"] = serde_json::json!("acme");

        let envelope = serde_json::from_value::<PairingCredentialEnvelope>(body);
        let err = envelope.expect_err("retired tenant_id alias must fail at schema ingress");
        assert!(
            err.to_string().contains("tenant_id"),
            "schema error should name the retired alias: {err}"
        );
    }

    #[test]
    fn pairing_credential_carries_immutable_user_binding() {
        let envelope =
            serde_json::from_value::<PairingCredentialEnvelope>(canonical_pairing_envelope_json())
                .unwrap();

        assert_eq!(
            envelope.user_id.as_deref(),
            Some("7a0d75be-1c47-44ce-83a1-60bdf14f3a0d")
        );
        assert_eq!(envelope.username.as_deref(), Some("alice"));
    }

    #[test]
    fn pairing_credential_rejects_missing_runtime_custody_facts() {
        for field in [
            "auth_binding",
            "credential_provisioned",
            "public_key_registered",
            "device_public_key",
            "device_public_key_fingerprint",
            "deploy_signature",
            "federated_peers",
            "ura",
            "last_seen_unix_ms",
        ] {
            let mut body = canonical_pairing_envelope_json();
            body.as_object_mut().expect("object").remove(field);

            let err = serde_json::from_value::<PairingCredentialEnvelope>(body)
                .expect_err("missing runtime custody facts must fail closed");

            assert!(err.to_string().contains(field), "{field}: {err}");
        }
    }

    #[test]
    fn federated_peer_entry_rejects_missing_peer_hub_url() {
        let err = serde_json::from_value::<FederatedPeerEntry>(serde_json::json!({
            "realm": "peer"
        }))
        .expect_err("federated peer rows must include the Hub endpoint fact");

        assert!(err.to_string().contains("peer_hub_url"), "{err}");
    }
}
