// EasyNet CLI — Federation Receipt Contract Facts
// ==============================================
//
// File: src/daemon/federation/receipt_contract.rs
// Description: Shared federation receipt fact DTOs for Authority producers
//              and device consumers.
//
// Protocol Responsibility
// -----------------------
// These types define the explicit runtime facts a realm Authority must emit
// for a device to seed and advance its Authority-published ability catalog.
// They are not product defaults and they are not optional enrichments.
//
// Implementation Approach
// -----------------------
// Keep the structs serde-only and fail-closed by construction: required facts
// have no serde defaults. Empty catalog/diff states remain valid only when the
// Authority explicitly serializes the empty arrays and revision value.
//
// Usage Contract
// --------------
// Authority-side wrappers construct these facts. Device-side clients
// deserialize the same shapes. A missing field is a protocol error because
// the receiver cannot distinguish "Authority deliberately published nothing"
// from "old/incorrect Authority omitted the route facts".
//
// Architectural Position
// ----------------------
// This module belongs to daemon federation, above raw Axon transport and below
// CLI/device product flows. It prevents client/server DTO drift inside this
// repository while Axon carries the generic invocation envelope.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Receipt body returned by a successful `federation.join`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct JoinReceipt {
    pub membership_ura: String,
    pub realm: String,
    pub join_receipt_hash: String,
    pub authority_published_abilities: Vec<AuthorityAbilityEntry>,
    pub authority_abilities_revision: u64,
    pub advertise_contract: AdvertiseContract,
}

/// One Authority-published ability descriptor as broadcast by the realm
/// Authority.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthorityAbilityEntry {
    pub name: String,
    pub descriptor: Value,
}

/// Bound on what a device may advertise to this realm Authority.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdvertiseContract {
    pub allowed_owner_prefixes: Vec<String>,
    pub allows_hosted_agents: bool,
}

impl AdvertiseContract {
    #[must_use]
    pub fn device_default() -> Self {
        Self {
            allowed_owner_prefixes: vec!["device.".to_string()],
            allows_hosted_agents: true,
        }
    }
}

/// Authority broadcast contract diff returned in `HeartbeatReceipt`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthorityAbilitiesDiff {
    pub revision: u64,
    pub added: Vec<AuthorityAbilityEntry>,
    pub removed: Vec<String>,
}

impl AuthorityAbilitiesDiff {
    #[must_use]
    pub fn empty_at(revision: u64) -> Self {
        Self {
            revision,
            added: Vec::new(),
            removed: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn join_receipt_rejects_shadow_membership_fields() {
        let err = serde_json::from_value::<JoinReceipt>(json!({
            "membership_ura": "easynet:///r/acme/device/01DEV",
            "realm": "acme",
            "join_receipt_hash": "abc123",
            "authority_published_abilities": [],
            "authority_abilities_revision": 0,
            "advertise_contract": {
                "allowed_owner_prefixes": ["device."],
                "allows_hosted_agents": true
            },
            "legacy_membership_token": "retired"
        }))
        .expect_err("join receipts must reject retired membership aliases");

        assert!(err.to_string().contains("legacy_membership_token"), "{err}");
    }

    #[test]
    fn authority_ability_entry_rejects_shadow_descriptor_fields() {
        let err = serde_json::from_value::<AuthorityAbilityEntry>(json!({
            "name": "meta.list_abilities",
            "descriptor": {
                "name": "meta.list_abilities"
            },
            "legacy_descriptor_ref": "route-ref::legacy"
        }))
        .expect_err("authority ability entries must reject shadow descriptor refs");

        assert!(err.to_string().contains("legacy_descriptor_ref"), "{err}");
    }

    #[test]
    fn advertise_contract_rejects_shadow_owner_fields() {
        let err = serde_json::from_value::<AdvertiseContract>(json!({
            "allowed_owner_prefixes": ["device."],
            "allows_hosted_agents": true,
            "legacy_owner_scope": "device.*"
        }))
        .expect_err("advertise contracts must reject shadow owner scopes");

        assert!(err.to_string().contains("legacy_owner_scope"), "{err}");
    }

    #[test]
    fn authority_abilities_diff_rejects_shadow_revision_fields() {
        let err = serde_json::from_value::<AuthorityAbilitiesDiff>(json!({
            "revision": 7,
            "added": [],
            "removed": [],
            "legacy_revision": 6
        }))
        .expect_err("authority ability diffs must reject retired revision aliases");

        assert!(err.to_string().contains("legacy_revision"), "{err}");
    }
}
