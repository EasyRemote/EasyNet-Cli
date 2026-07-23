// EasyNet CLI — API key response projections
// ==========================================
//
// File: src/daemon/ability/builtins/governance/api_key_projection.rs
// Description: Public DTO projections for governance API key abilities.
//
// Protocol Responsibility
// -----------------------
// Keep bearer-token lifecycle responses separate from the API key TOML store.
// The store carries token hashes and local audit fields; public responses expose
// only the canonical operator-facing key facts.
//
// Implementation Approach
// -----------------------
// Projection constructors copy only public fields from API key store records and
// use typed DTO structs with fail-closed serde boundaries.
//
// Usage Contract
// --------------
// API key handlers should call these constructors instead of assembling raw JSON
// response objects. `token_hash` must never cross this boundary, and the raw
// bearer token is only accepted by the one-shot create response constructor.
//
// Architectural Position
// ----------------------
// Governance projection layer. Depends on the API key store record type but
// contains no token minting, hashing, persistence, admission, or routing logic.

use serde::{Deserialize, Serialize};

use super::api_key::ApiKeyEntry;

pub const API_KEY_CREATE_WARNING: &str = "Save the token now. It is the only time we will show it.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyCreateResponse {
    pub token: String,
    pub key_ura: String,
    pub id_prefix: String,
    pub user_ura: String,
    pub label: Option<String>,
    pub created_at: u64,
    pub warning: String,
}

impl ApiKeyCreateResponse {
    pub fn one_time_token(
        token: impl Into<String>,
        key_ura: impl Into<String>,
        entry: &ApiKeyEntry,
    ) -> Self {
        Self {
            token: token.into(),
            key_ura: key_ura.into(),
            id_prefix: entry.id_prefix.clone(),
            user_ura: entry.user_ura.clone(),
            label: entry.label.clone(),
            created_at: entry.created_at,
            warning: API_KEY_CREATE_WARNING.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyListItem {
    pub id_prefix: String,
    pub label: Option<String>,
    pub created_at: u64,
    pub last_used_at: Option<u64>,
    pub revoked: bool,
    pub revoked_at: Option<u64>,
}

impl ApiKeyListItem {
    pub fn from_entry(entry: &ApiKeyEntry) -> Self {
        Self {
            id_prefix: entry.id_prefix.clone(),
            label: entry.label.clone(),
            created_at: entry.created_at,
            last_used_at: entry.last_used_at,
            revoked: entry.revoked_at.is_some(),
            revoked_at: entry.revoked_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyListResponse {
    pub keys: Vec<ApiKeyListItem>,
}

impl ApiKeyListResponse {
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = &'a ApiKeyEntry>) -> Self {
        Self {
            keys: entries
                .into_iter()
                .map(ApiKeyListItem::from_entry)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyRevokeResponse {
    pub revoked: String,
}

impl ApiKeyRevokeResponse {
    pub fn revoked(id_prefix: impl Into<String>) -> Self {
        Self {
            revoked: id_prefix.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry() -> ApiKeyEntry {
        ApiKeyEntry {
            id_prefix: "abcdef123456".to_string(),
            token_hash: "sha256-token".to_string(),
            user_ura: "easynet:///r/example/user/alice".to_string(),
            label: Some("dev".to_string()),
            created_at: 42,
            revoked_at: None,
            last_used_at: Some(84),
        }
    }

    #[test]
    fn api_key_create_response_preserves_public_shape_without_hash() {
        let response = ApiKeyCreateResponse::one_time_token(
            "easynet-sk-secret",
            "easynet:///r/example/resource/api_key.secret",
            &entry(),
        );
        let wire = serde_json::to_value(&response).expect("create response serializes");

        assert_eq!(wire["token"], "easynet-sk-secret");
        assert_eq!(
            wire["key_ura"],
            "easynet:///r/example/resource/api_key.secret"
        );
        assert_eq!(wire["id_prefix"], "abcdef123456");
        assert_eq!(wire["user_ura"], "easynet:///r/example/user/alice");
        assert_eq!(wire["label"], "dev");
        assert_eq!(wire["created_at"], 42);
        assert_eq!(wire["warning"], API_KEY_CREATE_WARNING);
        assert!(wire.get("token_hash").is_none());
    }

    #[test]
    fn api_key_list_response_preserves_public_shape_without_secret_material() {
        let response = ApiKeyListResponse::from_entries([&entry()]);
        let wire = serde_json::to_value(&response).expect("list response serializes");

        assert_eq!(wire["keys"][0]["id_prefix"], "abcdef123456");
        assert_eq!(wire["keys"][0]["label"], "dev");
        assert_eq!(wire["keys"][0]["created_at"], 42);
        assert_eq!(wire["keys"][0]["last_used_at"], 84);
        assert_eq!(wire["keys"][0]["revoked"], false);
        assert_eq!(wire["keys"][0]["revoked_at"], serde_json::Value::Null);
        assert!(wire["keys"][0].get("token").is_none());
        assert!(wire["keys"][0].get("token_hash").is_none());
        assert!(wire["keys"][0].get("user_ura").is_none());
    }

    #[test]
    fn api_key_revoke_response_preserves_public_shape() {
        let response = ApiKeyRevokeResponse::revoked("abcdef123456");
        let wire = serde_json::to_value(&response).expect("revoke response serializes");

        assert_eq!(wire["revoked"], "abcdef123456");
    }

    #[test]
    fn api_key_response_dtos_reject_unknown_fields() {
        let create_error = serde_json::from_value::<ApiKeyCreateResponse>(json!({
            "token": "easynet-sk-secret",
            "key_ura": "easynet:///r/example/resource/api_key.secret",
            "id_prefix": "abcdef123456",
            "user_ura": "easynet:///r/example/user/alice",
            "label": "dev",
            "created_at": 42,
            "warning": API_KEY_CREATE_WARNING,
            "token_hash": "sha256-token"
        }))
        .expect_err("create response must reject token hash");
        assert!(
            create_error.to_string().contains("token_hash"),
            "strict create response error should name unknown field: {create_error}"
        );

        let list_error = serde_json::from_value::<ApiKeyListResponse>(json!({
            "keys": [{
                "id_prefix": "abcdef123456",
                "label": "dev",
                "created_at": 42,
                "last_used_at": 84,
                "revoked": false,
                "revoked_at": null,
                "token": "easynet-sk-secret"
            }]
        }))
        .expect_err("list response must reject raw token");
        assert!(
            list_error.to_string().contains("token"),
            "strict list response error should name unknown field: {list_error}"
        );

        let revoke_error = serde_json::from_value::<ApiKeyRevokeResponse>(json!({
            "revoked": "abcdef123456",
            "user_ura": "easynet:///r/example/user/alice"
        }))
        .expect_err("revoke response must reject user scope leakage");
        assert!(
            revoke_error.to_string().contains("user_ura"),
            "strict revoke response error should name unknown field: {revoke_error}"
        );
    }
}
