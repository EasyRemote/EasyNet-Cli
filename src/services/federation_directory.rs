// EasyNet CLI — Cross-realm directory federation types (RFC-N3)
// ===============================================================
//
// File: src/services/federation_directory.rs
// Description: Wire shapes for the cross-realm directory
//              federation surface introduced by PR-N3
//              (`pr-drafts/PR-N3-spec-cross-realm-directory-v2.md`).
//
//              This commit (N3-1) lands `DirectoryEntry` only.
//              `DirectoryEvent` (the event-stream tagged enum) +
//              the `subscribe_directory` long-stream FSM upgrade
//              live in N3-2; the per-peer `RemoteDirectoryClient`
//              + `SharedFederatedDirectoryView` cell live in N3-3.
//
// Why a new module
// ----------------
// `federation_wrappers.rs` hosts the original PR-1 `federation.*`
// ability surface (`AgentSummary`, `JoinResponse`, etc.) which
// represents *presence* — "is this URI online right now". The
// RFC-N3 surface represents the *cross-realm directory* — a
// federated, mutually-subscribed view of every paired device on
// every trusted peer hub, with origin-realm provenance carried in
// the wire bytes. Mixing the two in one file would conflate two
// different audit boundaries (presence is per-stream-lifetime;
// directory entries persist across reconnects and are subject to
// the §2.4 origin_realm rewrite chokepoint).
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};

/// One entry in the federated directory view.
///
/// **Spec v2 §2.1**. Schema-B fields (`origin_realm`,
/// `hub_endpoint`, `last_seen_unix_ms`) ride `#[serde(default)]`
/// so a legacy reader (PR-N1 commit 8/N consumer that only knows
/// `agent_uri`/`node_id`/`display_name`/`status`) deserialises
/// new bytes unchanged. New readers project the optional fields
/// when present.
///
/// `origin_realm` carries the provenance of the entry. **None**
/// ⇔ the entry was constructed by the hub serving this view
/// (i.e. it speaks for its own realm). **Some(realm)** ⇔ the
/// entry was received from a peer hub's `subscribe_directory`
/// stream and the receiving hub stamped the peer's realm into
/// this field at the merge boundary. Wire-tampering is blocked
/// by the §2.4 rewrite chokepoint — the receiving hub
/// **overwrites** this field with the peer's authenticated realm
/// regardless of what the peer's bytes claimed, so a malicious
/// peer cannot pretend its entries originate elsewhere.
///
/// `hub_endpoint` is the hub URL/address that owns this device.
/// Useful for backend `listDevices` views and for the CLI to
/// render which hub a remote device is paired against. The
/// daemon-side `<self>.discover` path is allowed to leave this
/// `None` for local entries (local readers already know the
/// daemon's own endpoint).
///
/// `last_seen_unix_ms` is the epoch-ms timestamp of the last
/// heartbeat the *origin* hub observed for this device. Local
/// entries fill from `PresenceRegistry`; cross-realm entries
/// reflect the peer's reported value verbatim — no clock
/// translation, the peer's clock and ours are assumed
/// approximately synchronised (NTP-coordinated production
/// machines).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DirectoryEntry {
    /// Canonical agent URI. Always realm-prefixed
    /// (`easynet:///r/<realm>/agent/<id>` per PR-7 §5.1).
    pub agent_uri: String,
    /// Stable node id within the realm. Matches the device's
    /// `credentials.json::node_id`.
    pub node_id: String,
    /// Operator-set display name. `None` ⇒ CLI renders
    /// `node_id` as a fallback.
    #[serde(default)]
    pub display_name: Option<String>,
    /// `"active"` | `"stale"` | `"draining"`. Stale = last
    /// heartbeat older than the realm's keepalive deadline;
    /// draining = the device announced shutdown but has not
    /// yet been removed from the registry.
    pub status: String,
    /// Realm of origin. See struct docs for the rewrite
    /// chokepoint that authenticates this field.
    #[serde(default)]
    pub origin_realm: Option<String>,
    /// Hub endpoint that owns the device.
    #[serde(default)]
    pub hub_endpoint: Option<String>,
    /// Last-heartbeat epoch-ms.
    #[serde(default)]
    pub last_seen_unix_ms: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_entry_json() -> &'static str {
        // The PR-N1 commit 8/N readers see this exact shape on
        // the wire. Legacy emit drops the schema-B fields
        // entirely; new emit includes them.
        r#"{
            "agent_uri": "easynet:///r/realm-a/agent/device-A",
            "node_id": "node-1",
            "display_name": "silan-laptop",
            "status": "active"
        }"#
    }

    fn full_entry_json() -> &'static str {
        r#"{
            "agent_uri": "easynet:///r/realm-a/agent/device-A",
            "node_id": "node-1",
            "display_name": "silan-laptop",
            "status": "active",
            "origin_realm": "realm-a",
            "hub_endpoint": "https://hub-a.example:50443",
            "last_seen_unix_ms": 1714492800000
        }"#
    }

    #[test]
    fn legacy_entry_deserialises_with_origin_realm_none() {
        // Schema-B forward-compat: a 4-field legacy entry
        // round-trips without errors and the new optional
        // fields surface as None / None / None.
        let entry: DirectoryEntry = serde_json::from_str(legacy_entry_json()).expect("deserialise");
        assert_eq!(entry.agent_uri, "easynet:///r/realm-a/agent/device-A");
        assert_eq!(entry.node_id, "node-1");
        assert_eq!(entry.display_name.as_deref(), Some("silan-laptop"));
        assert_eq!(entry.status, "active");
        assert_eq!(entry.origin_realm, None);
        assert_eq!(entry.hub_endpoint, None);
        assert_eq!(entry.last_seen_unix_ms, None);
    }

    #[test]
    fn full_entry_round_trips_all_fields() {
        let entry: DirectoryEntry = serde_json::from_str(full_entry_json()).expect("deserialise");
        assert_eq!(entry.origin_realm.as_deref(), Some("realm-a"));
        assert_eq!(
            entry.hub_endpoint.as_deref(),
            Some("https://hub-a.example:50443")
        );
        assert_eq!(entry.last_seen_unix_ms, Some(1_714_492_800_000));

        // Re-serialise and confirm the fields persist through
        // the round-trip. Field order is serde-determined; we
        // assert each key/value pair via JSON parse rather than
        // string match to stay byte-format-tolerant.
        let bytes = serde_json::to_vec(&entry).expect("serialise");
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("re-parse");
        assert_eq!(parsed["agent_uri"], "easynet:///r/realm-a/agent/device-A");
        assert_eq!(parsed["origin_realm"], "realm-a");
        assert_eq!(parsed["hub_endpoint"], "https://hub-a.example:50443");
        assert_eq!(parsed["last_seen_unix_ms"], 1_714_492_800_000_i64);
    }

    #[test]
    fn local_entry_serialised_with_none_fields_emits_nulls() {
        // Local entry (origin_realm = None, no hub_endpoint).
        // serde's default Option behaviour emits these as JSON
        // null. Legacy readers ignore unknown fields; null is
        // identically interpretable as "field absent" by the
        // schema-B convention.
        let local = DirectoryEntry {
            agent_uri: "easynet:///r/realm-a/agent/local-1".to_string(),
            node_id: "local-1".to_string(),
            display_name: None,
            status: "active".to_string(),
            origin_realm: None,
            hub_endpoint: None,
            last_seen_unix_ms: None,
        };
        let bytes = serde_json::to_vec(&local).expect("serialise");
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).expect("re-parse");
        assert!(parsed["origin_realm"].is_null());
        assert!(parsed["hub_endpoint"].is_null());
        assert!(parsed["last_seen_unix_ms"].is_null());
    }

    #[test]
    fn round_trip_through_serde_preserves_field_equality() {
        // PartialEq derive lets us assert byte-stable round-
        // trips for testing receivers that compare entries to
        // detect changes between subscribe-stream snapshots.
        let original = DirectoryEntry {
            agent_uri: "easynet:///r/realm-b/agent/peer-device".to_string(),
            node_id: "peer-1".to_string(),
            display_name: Some("silan-phone".to_string()),
            status: "stale".to_string(),
            origin_realm: Some("realm-b".to_string()),
            hub_endpoint: Some("https://hub-b.example:50443".to_string()),
            last_seen_unix_ms: Some(1_714_500_000_000),
        };
        let bytes = serde_json::to_vec(&original).expect("serialise");
        let restored: DirectoryEntry = serde_json::from_slice(&bytes).expect("deserialise");
        assert_eq!(original, restored);
    }
}
