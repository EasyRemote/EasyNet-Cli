// EasyNet CLI — managed signing response projections
// ==================================================
//
// File: src/daemon/keyring/managed_signing_projection.rs
// Description: Public DTO projections for keyring managed-signing abilities.
//
// Protocol Responsibility
// -----------------------
// Keep managed-signing public ability responses separate from key custody and
// inventory lifecycle. The vault owns private seed material and state-machine
// transitions; this module owns only response shapes safe for daemon callers.
//
// Implementation Approach
// -----------------------
// Projection constructors copy only public fields from managed-signing key
// projections and use typed DTO structs with fail-closed serde boundaries.
//
// Usage Contract
// --------------
// Keyring ability handlers should call these constructors instead of assembling
// raw JSON response objects. List entries intentionally omit public key bytes
// and signer policy refs; callers that need public key material must use the
// explicit get-public path.
//
// Architectural Position
// ----------------------
// Keyring projection layer. Depends on managed-signing public inventory
// projections but contains no provider calls, private-key custody, signing, or
// persistence logic.

use serde::{Deserialize, Serialize};

use super::{ManagedSigningKeyProjection, ManagedSigningStatus};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedSigningCreateResponse {
    pub key_id: String,
    pub public_key: String,
    pub fingerprint: String,
    pub rotation_epoch: u64,
}

impl ManagedSigningCreateResponse {
    pub fn new(entry: &ManagedSigningKeyProjection, fingerprint: impl Into<String>) -> Self {
        Self {
            key_id: entry.key_id.clone(),
            public_key: entry.public_key_b64.clone(),
            fingerprint: fingerprint.into(),
            rotation_epoch: entry.rotation_epoch,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedSigningListEntry {
    pub key_id: String,
    pub algo: String,
    pub purpose: String,
    pub status: String,
    pub rotation_epoch: u64,
    pub bound_subject: Option<String>,
    pub rotated_from: Option<String>,
    pub created_unix_ms: i64,
    pub expires_unix_ms: Option<i64>,
    pub revoked_unix_ms: Option<i64>,
}

impl ManagedSigningListEntry {
    pub fn from_projection(entry: &ManagedSigningKeyProjection) -> Self {
        Self {
            key_id: entry.key_id.clone(),
            algo: "ed25519".to_string(),
            purpose: entry.purpose.clone(),
            status: managed_signing_status_wire(entry.status).to_string(),
            rotation_epoch: entry.rotation_epoch,
            bound_subject: entry.bound_subject.clone(),
            rotated_from: entry.rotated_from.clone(),
            created_unix_ms: entry.created_unix_ms,
            expires_unix_ms: entry.expires_unix_ms,
            revoked_unix_ms: entry.revoked_unix_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedSigningListResponse {
    pub entries: Vec<ManagedSigningListEntry>,
}

impl ManagedSigningListResponse {
    pub fn from_entries<'a>(
        entries: impl IntoIterator<Item = &'a ManagedSigningKeyProjection>,
    ) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(ManagedSigningListEntry::from_projection)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManagedSigningPublicResponse {
    pub public_key: String,
    pub fingerprint: String,
    pub status: String,
    pub rotation_epoch: u64,
}

impl ManagedSigningPublicResponse {
    pub fn new(entry: &ManagedSigningKeyProjection, fingerprint: impl Into<String>) -> Self {
        Self {
            public_key: entry.public_key_b64.clone(),
            fingerprint: fingerprint.into(),
            status: managed_signing_status_wire(entry.status).to_string(),
            rotation_epoch: entry.rotation_epoch,
        }
    }
}

pub const fn managed_signing_status_wire(status: ManagedSigningStatus) -> &'static str {
    match status {
        ManagedSigningStatus::Active => "active",
        ManagedSigningStatus::Retired => "retired",
        ManagedSigningStatus::Revoked => "revoked",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry() -> ManagedSigningKeyProjection {
        ManagedSigningKeyProjection {
            key_id: "key-1".to_string(),
            purpose: "agent_signing".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            status: ManagedSigningStatus::Active,
            rotation_epoch: 0,
            bound_subject: Some("easynet:///r/example/user/alice".to_string()),
            signer_policy_ref: Some("policy-ref".to_string()),
            rotated_from: None,
            created_unix_ms: 42,
            expires_unix_ms: None,
            revoked_unix_ms: None,
        }
    }

    #[test]
    fn managed_signing_create_response_preserves_public_shape() {
        let response = ManagedSigningCreateResponse::new(&entry(), "fingerprint");
        let wire = serde_json::to_value(&response).expect("create response serializes");

        assert_eq!(wire["key_id"], "key-1");
        assert_eq!(
            wire["public_key"],
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        );
        assert_eq!(wire["fingerprint"], "fingerprint");
        assert_eq!(wire["rotation_epoch"], 0);
        assert!(wire.get("seed_hex").is_none());
        assert!(wire.get("signer_policy_ref").is_none());
    }

    #[test]
    fn managed_signing_list_response_preserves_public_shape_without_key_material() {
        let response = ManagedSigningListResponse::from_entries([&entry()]);
        let wire = serde_json::to_value(&response).expect("list response serializes");

        assert_eq!(wire["entries"][0]["key_id"], "key-1");
        assert_eq!(wire["entries"][0]["algo"], "ed25519");
        assert_eq!(wire["entries"][0]["purpose"], "agent_signing");
        assert_eq!(wire["entries"][0]["status"], "active");
        assert_eq!(wire["entries"][0]["rotation_epoch"], 0);
        assert_eq!(
            wire["entries"][0]["bound_subject"],
            "easynet:///r/example/user/alice"
        );
        assert_eq!(wire["entries"][0]["created_unix_ms"], 42);
        assert!(wire["entries"][0].get("public_key").is_none());
        assert!(wire["entries"][0].get("public_key_b64").is_none());
        assert!(wire["entries"][0].get("signer_policy_ref").is_none());
        assert!(wire["entries"][0].get("seed_hex").is_none());
    }

    #[test]
    fn managed_signing_public_response_preserves_public_shape() {
        let response = ManagedSigningPublicResponse::new(&entry(), "fingerprint");
        let wire = serde_json::to_value(&response).expect("public response serializes");

        assert_eq!(
            wire["public_key"],
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        );
        assert_eq!(wire["fingerprint"], "fingerprint");
        assert_eq!(wire["status"], "active");
        assert_eq!(wire["rotation_epoch"], 0);
    }

    #[test]
    fn managed_signing_response_dtos_reject_unknown_fields() {
        let create_error = serde_json::from_value::<ManagedSigningCreateResponse>(json!({
            "key_id": "key-1",
            "public_key": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "fingerprint": "fingerprint",
            "rotation_epoch": 0,
            "seed_hex": "private"
        }))
        .expect_err("create response must reject private seed leakage");
        assert!(
            create_error.to_string().contains("seed_hex"),
            "strict create response error should name unknown field: {create_error}"
        );

        let list_error = serde_json::from_value::<ManagedSigningListResponse>(json!({
            "entries": [{
                "key_id": "key-1",
                "algo": "ed25519",
                "purpose": "agent_signing",
                "status": "active",
                "rotation_epoch": 0,
                "bound_subject": "easynet:///r/example/user/alice",
                "rotated_from": null,
                "created_unix_ms": 42,
                "expires_unix_ms": null,
                "revoked_unix_ms": null,
                "public_key": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
            }]
        }))
        .expect_err("list response must reject public key material");
        assert!(
            list_error.to_string().contains("public_key"),
            "strict list response error should name unknown field: {list_error}"
        );

        let public_error = serde_json::from_value::<ManagedSigningPublicResponse>(json!({
            "public_key": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "fingerprint": "fingerprint",
            "status": "active",
            "rotation_epoch": 0,
            "signer_policy_ref": "policy-ref"
        }))
        .expect_err("public response must reject signer policy leakage");
        assert!(
            public_error.to_string().contains("signer_policy_ref"),
            "strict public response error should name unknown field: {public_error}"
        );
    }
}
