//! Daemon-owned federation ability payloads.
//!
//! These records describe EasyNet's directory and peer-policy abilities. They
//! are product contracts consumed by the daemon, not canonical Axon SDK
//! abstractions. Axon owns the Invocation transport carrying their JSON bytes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DirectoryEntry {
    pub agent_ura: String,
    pub node_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub status: String,
    #[serde(default)]
    pub origin_realm: Option<String>,
    #[serde(default)]
    pub hub_endpoint: Option<String>,
    #[serde(default)]
    pub last_seen_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SigningAuthority {
    SelfSigned,
    HostedBy { host_ura: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DirectoryAgentSummary {
    pub agent_ura: String,
    pub signing_authority: SigningAuthority,
    pub status: String,
    pub ability_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DirectoryEvent {
    Snapshot {
        agents: Vec<DirectoryAgentSummary>,
        snapshot_unix_ms: i64,
    },
    AgentAdvertised {
        agent_ura: String,
        signing_authority: SigningAuthority,
        replaced_prior: bool,
        unix_ms: i64,
    },
    AgentRevoked {
        agent_ura: String,
        was_active: bool,
        reason: String,
        unix_ms: i64,
    },
    Heartbeat {
        unix_ms: i64,
    },
    OwnerProjectionChanged {
        owner_ura: String,
        host_device_ura: String,
        projection_revision: u64,
        projection_digest: String,
        ability_count: u64,
        stale_count: u64,
        removed_count: u64,
        lease_expires_unix_ms: i64,
        unix_ms: i64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ListUserDevicesRequest {
    pub realm: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ListUserDevicesResponse {
    pub devices: Vec<DirectoryEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DiscoverRequest {
    #[serde(default)]
    pub agent_ura: Option<String>,
    #[serde(default)]
    pub local_user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DiscoverResponse {
    pub entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResolveFilterRequest {
    #[serde(default)]
    pub agent_ura_prefix: Option<String>,
    #[serde(default)]
    pub include_abilities: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResolveRequest {
    #[serde(default)]
    pub ura_prefix: Option<String>,
    #[serde(default)]
    pub include_abilities: bool,
    #[serde(default)]
    pub filter: Option<ResolveFilterRequest>,
}

impl ResolveRequest {
    #[must_use]
    pub fn effective_ura_prefix(&self) -> Option<&str> {
        self.ura_prefix.as_deref().or_else(|| {
            self.filter
                .as_ref()
                .and_then(|filter| filter.agent_ura_prefix.as_deref())
        })
    }

    #[must_use]
    pub fn wants_abilities(&self) -> bool {
        self.include_abilities
            || self
                .filter
                .as_ref()
                .is_some_and(|filter| filter.include_abilities)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResolveAgentSummary {
    pub ura: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_node_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub abilities: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResolveResponse {
    pub agents: Vec<ResolveAgentSummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResolveKeyRequest {
    pub agent_ura: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub presented_pubkey_b64: Option<String>,
}

impl ResolveKeyRequest {
    #[must_use]
    pub fn new(agent_ura: impl Into<String>) -> Self {
        Self {
            agent_ura: agent_ura.into(),
            presented_pubkey_b64: None,
        }
    }

    #[must_use]
    pub fn with_presented_pubkey_b64(mut self, presented_pubkey_b64: impl Into<String>) -> Self {
        let presented_pubkey_b64 = presented_pubkey_b64.into().trim().to_string();
        if !presented_pubkey_b64.is_empty() {
            self.presented_pubkey_b64 = Some(presented_pubkey_b64);
        }
        self
    }

    pub fn to_arguments_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResolveKeyResponse {
    pub public_key_b64: String,
    pub public_key_hex: String,
    #[serde(default)]
    pub public_keys_b64: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub principal_owner_ura: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub principal_owner_user_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_unknown_field_rejected<T>(value: serde_json::Value, field: &str)
    where
        T: for<'de> Deserialize<'de>,
    {
        let error = match serde_json::from_value::<T>(value) {
            Ok(_) => panic!("unknown field {field:?} must fail closed"),
            Err(error) => error,
        };
        let message = error.to_string();
        assert!(
            message.contains(&format!("unknown field `{field}`")),
            "unknown field {field:?} must be named in parse error: {message}"
        );
    }

    fn sample_summary() -> DirectoryAgentSummary {
        DirectoryAgentSummary {
            agent_ura: "easynet:///r/acme/device/dev-1".into(),
            signing_authority: SigningAuthority::SelfSigned,
            status: "active".into(),
            ability_count: 3,
        }
    }

    #[test]
    fn resolve_key_request_encodes_absent_presented_pubkey_as_absent_field() {
        let request =
            ResolveKeyRequest::new("easynet:///r/acme/user/alice").with_presented_pubkey_b64("   ");
        let value: serde_json::Value =
            serde_json::from_slice(&request.to_arguments_bytes().expect("encode")).expect("decode");

        assert_eq!(value["agent_ura"], "easynet:///r/acme/user/alice");
        assert!(
            value.get("presented_pubkey_b64").is_none(),
            "blank presented key must not be serialized as compatibility data: {value}"
        );
        assert!(
            value.get("presented_pubkey_hex").is_none(),
            "hex pin must not be serialized as compatibility data: {value}"
        );
    }

    #[test]
    fn resolve_key_request_rejects_retired_presented_pubkey_hex() {
        let error = serde_json::from_value::<ResolveKeyRequest>(serde_json::json!({
            "agent_ura": "easynet:///r/acme/user/alice",
            "presented_pubkey_hex": "00"
        }))
        .expect_err("retired hex presented-key pin must fail closed");

        assert!(
            error.to_string().contains("presented_pubkey_hex"),
            "rejection must name retired field: {error}"
        );
    }

    #[test]
    fn resolve_key_request_encodes_presented_pubkey_pin() {
        let request = ResolveKeyRequest::new("easynet:///r/acme/user/alice")
            .with_presented_pubkey_b64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=");
        let value: serde_json::Value =
            serde_json::from_slice(&request.to_arguments_bytes().expect("encode")).expect("decode");

        assert_eq!(value["agent_ura"], "easynet:///r/acme/user/alice");
        assert_eq!(
            value["presented_pubkey_b64"],
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
        );
    }

    #[test]
    fn directory_event_tags_match_product_producer_vocabulary() {
        let cases = [
            (
                DirectoryEvent::Snapshot {
                    agents: vec![sample_summary()],
                    snapshot_unix_ms: 100,
                },
                "snapshot",
            ),
            (
                DirectoryEvent::AgentAdvertised {
                    agent_ura: "easynet:///r/acme/device/dev-1".into(),
                    signing_authority: SigningAuthority::SelfSigned,
                    replaced_prior: false,
                    unix_ms: 101,
                },
                "agent_advertised",
            ),
            (
                DirectoryEvent::AgentRevoked {
                    agent_ura: "easynet:///r/acme/device/dev-1".into(),
                    was_active: true,
                    reason: "drained".into(),
                    unix_ms: 102,
                },
                "agent_revoked",
            ),
            (DirectoryEvent::Heartbeat { unix_ms: 103 }, "heartbeat"),
            (
                DirectoryEvent::OwnerProjectionChanged {
                    owner_ura: "easynet:///r/acme/device/dev-1".into(),
                    host_device_ura: "easynet:///r/acme/device/dev-1".into(),
                    projection_revision: 5,
                    projection_digest: "deadbeef".into(),
                    ability_count: 2,
                    stale_count: 0,
                    removed_count: 1,
                    lease_expires_unix_ms: 50_000,
                    unix_ms: 104,
                },
                "owner_projection_changed",
            ),
        ];

        for (event, expected_tag) in cases {
            let value = serde_json::to_value(&event).unwrap();
            assert_eq!(value["type"], expected_tag);
            assert_eq!(
                serde_json::from_value::<DirectoryEvent>(value).unwrap(),
                event
            );
        }
    }

    #[test]
    fn federation_wire_requests_reject_retired_fields() {
        assert_unknown_field_rejected::<DiscoverRequest>(
            serde_json::json!({
                "agent_ura": "easynet:///r/acme/device/dev-1",
                "target_ura": "easynet:///r/acme/device/dev-1"
            }),
            "target_ura",
        );
        assert_unknown_field_rejected::<ResolveRequest>(
            serde_json::json!({
                "ura_prefix": "easynet:///r/acme",
                "legacy_prefix": "easynet:///r/acme"
            }),
            "legacy_prefix",
        );
        assert_unknown_field_rejected::<ResolveFilterRequest>(
            serde_json::json!({
                "agent_ura_prefix": "easynet:///r/acme/device",
                "include_abilities": true,
                "include_legacy_rows": true
            }),
            "include_legacy_rows",
        );
        assert_unknown_field_rejected::<ResolveKeyRequest>(
            serde_json::json!({
                "agent_ura": "easynet:///r/acme/user/alice",
                "retired_agent_locator": "easynet:///r/acme/user/alice"
            }),
            "retired_agent_locator",
        );
    }

    #[test]
    fn federation_wire_responses_reject_retired_fields() {
        assert_unknown_field_rejected::<DirectoryEntry>(
            serde_json::json!({
                "agent_ura": "easynet:///r/acme/device/dev-1",
                "node_id": "dev-1",
                "status": "online",
                "retired_agent_locator": "easynet:///r/acme/device/dev-1"
            }),
            "retired_agent_locator",
        );
        assert_unknown_field_rejected::<DirectoryAgentSummary>(
            serde_json::json!({
                "agent_ura": "easynet:///r/acme/device/dev-1",
                "signing_authority": {"kind": "self_signed"},
                "status": "active",
                "ability_count": 3,
                "legacy_status": "active"
            }),
            "legacy_status",
        );
        assert_unknown_field_rejected::<DirectoryEvent>(
            serde_json::json!({
                "type": "heartbeat",
                "unix_ms": 103,
                "legacy_keepalive": true
            }),
            "legacy_keepalive",
        );
        assert_unknown_field_rejected::<ResolveKeyResponse>(
            serde_json::json!({
                "public_key_b64": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                "public_key_hex": "00",
                "public_key": "legacy"
            }),
            "public_key",
        );
    }

    #[test]
    fn list_user_devices_requires_product_realm_key() {
        let request = serde_json::from_value::<ListUserDevicesRequest>(serde_json::json!({
            "tenant_id": "acme"
        }));
        assert!(request.is_err());
    }
}
