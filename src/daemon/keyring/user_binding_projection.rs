// EasyNet CLI — user-binding response projections
// =================================================
//
// File: src/daemon/keyring/user_binding_projection.rs
// Description: Public DTO projections for federated user-binding abilities.
//
// Protocol Responsibility
// -----------------------
// Keep user-binding token exchange response shapes separate from token
// issuance, signature verification, replay tracking, and binding persistence.
//
// Implementation Approach
// -----------------------
// Projection constructors copy only response-safe fields from user-binding
// domain objects and expose fail-closed serde DTOs. The module contains no
// key custody, cryptographic verification, nonce tracking, or store mutation.
//
// Usage Contract
// --------------
// Keyring ability handlers call these constructors after the user-binding
// state machine succeeds. Handlers must not assemble these public response
// payloads with ad hoc JSON objects.
//
// Architectural Position
// ----------------------
// Keyring projection layer. Depends on public user-binding token/binding
// domain projections, but not on provider, Vault, or persistence internals.

use serde::{Deserialize, Serialize};

use super::federated_bindings::FederatedUserBinding;
use super::user_binding_chain::UserBindingToken;

pub const USER_BINDING_TRANSPORT_HINT_JWT_CUSTOM_CLAIM: &str = "jwt-custom-claim";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UserBindingIssueResponse {
    pub token: UserBindingToken,
    pub transport_hint: String,
}

impl UserBindingIssueResponse {
    pub fn issued(token: UserBindingToken) -> Self {
        Self {
            token,
            transport_hint: USER_BINDING_TRANSPORT_HINT_JWT_CUSTOM_CLAIM.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UserBindingConsumeResponse {
    pub binding_recorded: bool,
    pub source_realm: String,
    pub source_user_ura: String,
    pub local_user_id: String,
}

impl UserBindingConsumeResponse {
    pub fn recorded(binding: &FederatedUserBinding) -> Self {
        Self {
            binding_recorded: true,
            source_realm: binding.source_realm.clone(),
            source_user_ura: binding.source_user_ura.clone(),
            local_user_id: binding.local_user_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn token() -> UserBindingToken {
        UserBindingToken::new_unsigned(
            "realm-a",
            "easynet:///r/realm-a/user/user-c",
            [1; 32],
            "realm-b",
            1_714_500_000_000,
            [2; 32],
        )
    }

    fn binding() -> FederatedUserBinding {
        FederatedUserBinding {
            source_realm: "realm-a".to_string(),
            source_user_ura: "easynet:///r/realm-a/user/user-c".to_string(),
            source_user_pubkey_b64: "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=".to_string(),
            local_user_id: "user-c-on-realm-b".to_string(),
            bound_at_unix_ms: 1_714_500_001_000,
        }
    }

    #[test]
    fn user_binding_issue_response_preserves_public_shape() {
        let response = UserBindingIssueResponse::issued(token());
        let wire = serde_json::to_value(&response).expect("issue response serializes");

        assert_eq!(wire["transport_hint"], "jwt-custom-claim");
        assert_eq!(wire["token"]["source_realm"], "realm-a");
        assert_eq!(
            wire["token"]["source_user_ura"],
            "easynet:///r/realm-a/user/user-c"
        );
        assert_eq!(wire["token"]["target_realm"], "realm-b");
        assert_eq!(wire["token"]["issued_at_ms"], 1_714_500_000_000_u64);
        assert!(wire.get("managed_key_id").is_none());
        assert!(wire.get("source_user_pubkey_b64").is_none());
    }

    #[test]
    fn user_binding_consume_response_preserves_public_shape() {
        let response = UserBindingConsumeResponse::recorded(&binding());
        let wire = serde_json::to_value(&response).expect("consume response serializes");

        assert_eq!(wire["binding_recorded"], true);
        assert_eq!(wire["source_realm"], "realm-a");
        assert_eq!(wire["source_user_ura"], "easynet:///r/realm-a/user/user-c");
        assert_eq!(wire["local_user_id"], "user-c-on-realm-b");
        assert!(wire.get("source_user_pubkey_b64").is_none());
        assert!(wire.get("bound_at_unix_ms").is_none());
    }

    #[test]
    fn user_binding_response_dtos_reject_unknown_fields() {
        let issue_error = serde_json::from_value::<UserBindingIssueResponse>(json!({
            "token": token(),
            "transport_hint": "jwt-custom-claim",
            "managed_key_id": "key-1"
        }))
        .expect_err("issue response must reject request echo fields");
        assert!(
            issue_error.to_string().contains("managed_key_id"),
            "strict issue response error should name unknown field: {issue_error}"
        );

        let consume_error = serde_json::from_value::<UserBindingConsumeResponse>(json!({
            "binding_recorded": true,
            "source_realm": "realm-a",
            "source_user_ura": "easynet:///r/realm-a/user/user-c",
            "local_user_id": "user-c-on-realm-b",
            "source_user_pubkey_b64": "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE="
        }))
        .expect_err("consume response must reject persistence fields");
        assert!(
            consume_error.to_string().contains("source_user_pubkey_b64"),
            "strict consume response error should name unknown field: {consume_error}"
        );
    }
}
