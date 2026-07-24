// EasyNet CLI — Federation Receipt Contract Facts
// ==============================================
//
// File: src/daemon/federation/receipt_contract.rs
// Description: Shared federation receipt fact DTOs for hub producers and
//              device consumers.
//
// Protocol Responsibility
// -----------------------
// These types define the explicit runtime facts a hub must emit for a device
// to seed and advance its hub-published ability catalog. They are not product
// defaults and they are not optional enrichments.
//
// Implementation Approach
// -----------------------
// Keep the structs serde-only and fail-closed by construction: required facts
// have no serde defaults. Empty catalog/diff states remain valid only when the
// hub explicitly serializes the empty arrays and revision value.
//
// Usage Contract
// --------------
// Hub-side wrappers construct these facts. Device-side clients deserialize the
// same shapes. A missing field is a protocol error because the receiver cannot
// distinguish "hub deliberately published nothing" from "old/incorrect hub
// omitted the route facts".
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
    pub hub_published_abilities: Vec<HubAbilityEntry>,
    pub hub_abilities_revision: u64,
    pub advertise_contract: AdvertiseContract,
}

/// One hub-owned ability descriptor as broadcast by the hub.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HubAbilityEntry {
    pub name: String,
    pub descriptor: Value,
}

/// Bound on what a device may advertise at this hub.
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

/// Hub-broadcast contract diff returned in `HeartbeatReceipt`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HubAbilitiesDiff {
    pub revision: u64,
    pub added: Vec<HubAbilityEntry>,
    pub removed: Vec<String>,
}

impl HubAbilitiesDiff {
    #[must_use]
    pub fn empty_at(revision: u64) -> Self {
        Self {
            revision,
            added: Vec::new(),
            removed: Vec::new(),
        }
    }
}
