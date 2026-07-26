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
pub struct AuthorityAbilityEntry {
    pub name: String,
    pub descriptor: Value,
}

/// Bound on what a device may advertise to this realm Authority.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
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
