// EasyNet CLI — Federation Ability Contract DTOs (RFC-001 §3, §1.4)
// =================================================================
//
// File: src/daemon/federation/client/ability_contract.rs
//
// Typed argument/response helpers for the four `federation.*`
// abilities the hub-profile Agent exposes (per AXON-RFC-001 plan
// v4.1.1 §18 + EasyNet-Axon
// `core/runtime-rs/src/services/invocation/hub_profile.rs`):
//
//   * federation.join             — pre-membership genesis call
//   * federation.advertise_agent  — host registers a hosted Agent
//   * federation.heartbeat        — periodic membership liveness
//   * federation.resolve          — directory query
//
// What this module is
// -------------------
// Pure data shapes + a small protocol-helper layer. It does not
// itself open a connection to the hub. Callers hand the JSON args
// produced here to whichever transport they already have wired
// (today: `DendriteBridge::ability_call_raw`; tomorrow: a thin
// `axon_runtime_local_invoke` shim once the daemon-internal IPC
// path lands).
//
// Why this split exists
// ---------------------
// Keeping arg/response shapes here (instead of inlining JSON in
// federation publish/advertise or the daemon boot path) gives us:
//
//   1. One file the hub-profile maintainer can grep when its
//      ability schemas change. The hub-profile's serde structs and
//      these structs are wire-compatible; if they ever drift, the
//      round-trip integration test catches it.
//   2. Unit-testable construction. We assert the JSON payloads we
//      emit match the hub's `JoinArgs` / `AdvertiseAgentArgs`
//      shape without needing a live runtime.
//   3. A natural seam for adding DelegationProof later. When P3+
//      ships JWT-derived delegation, only this module changes; the
//      bridge call site stays untouched.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// RFC-005 federation.forward_invoke argument shape.
#[derive(Debug, Clone, Serialize)]
pub struct ForwardInvokeArgs {
    pub target_ura: String,
    pub ability_ura: String,
    /// Standard base64 of the serialized argument payload (typically
    /// JSON bytes for ability calls). Hub forwards verbatim.
    pub arguments_b64: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ForwardInvokeReceipt {
    pub ok: bool,
    pub state_code: i32,
    #[serde(default)]
    pub result_b64: String,
    #[serde(default)]
    pub result_content_type: String,
    #[serde(default)]
    pub error_code: String,
    #[serde(default)]
    pub error_message: String,
}

/// RFC-002 §5.1 federation.resolve_key argument shape.
#[derive(Debug, Clone, Serialize)]
pub struct ResolveKeyArgs {
    pub agent_ura: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ResolveKeyReceipt {
    #[serde(default)]
    pub agent_ura: String,
    pub public_key_hex: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub key_id: String,
    #[serde(default)]
    pub rotation_epoch: u64,
}

/// Arguments for `federation.join`. Matches the hub-profile's
/// `JoinArgs` struct in
/// `EasyNet-Axon/core/runtime-rs/src/services/invocation/hub_profile.rs`.
#[derive(Debug, Clone, Serialize)]
pub struct JoinArgs {
    pub realm: String,
    /// Canonical membership identity requested by the joining device.
    /// This is the post-genesis device URA the hub binds to
    /// `public_key_hex` and returns in `JoinReceipt.membership_ura`.
    pub membership_ura: String,
    /// Lowercase hex of the Ed25519 public key the joining device
    /// just generated. The hub binds this key to the canonical URA
    /// in its receipt.
    pub public_key_hex: String,
    /// Optional pairing-secret carrier; the P3 hub does not yet
    /// validate but accepts the field for forward compatibility.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pairing_secret: Option<String>,
}

/// Receipt body returned by a successful `federation.join`. The
/// `join_receipt_hash` is the device's §A8 [P3] membership-lineage
/// root and MUST be persisted into `~/.easynet/credentials.json`.
///
/// AXON-RFC-001 v4.1.7 hub-broadcast contract adds three fields:
/// `hub_published_abilities` (the snapshot of hub-owned abilities
/// the hub advertises to every member), `hub_abilities_revision`
/// (the monotonic counter the device passes back as
/// `since_abilities_revision` on subsequent heartbeats), and
/// `advertise_contract` (the prefix bounds the device must respect
/// on outbound `federation.advertise_*` calls). All three default
/// when absent so a v4.1.6 device reading a v4.1.7 hub (or vice
/// versa) interops without breaking — empty snapshot, revision 0,
/// default contract.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct JoinReceipt {
    pub membership_ura: String,
    pub realm: String,
    pub join_receipt_hash: String,
    #[serde(default)]
    pub hub_published_abilities: Vec<HubAbilityEntry>,
    #[serde(default)]
    pub hub_abilities_revision: u64,
    #[serde(default)]
    pub advertise_contract: AdvertiseContract,
}

/// One hub-owned ability descriptor as broadcast by the hub. The
/// `descriptor` field is opaque (`Value`) — the hub-side schema
/// can evolve without forcing a Cli release.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HubAbilityEntry {
    pub name: String,
    pub descriptor: Value,
}

/// Bound on what a device may advertise at this hub. The hub
/// pre-declares which name prefixes it accepts on
/// `federation.advertise_*` calls; the device's session prelude
/// filters its outbound advertise set against this list. v0
/// default: `["device."]` + `allows_hosted_agents = true`. Old
/// hubs that don't send the field land on this default — same
/// behavior they had before the contract existed.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AdvertiseContract {
    #[serde(default)]
    pub allowed_owner_prefixes: Vec<String>,
    #[serde(default = "default_allows_hosted_agents")]
    pub allows_hosted_agents: bool,
}

impl Default for AdvertiseContract {
    fn default() -> Self {
        Self {
            allowed_owner_prefixes: vec!["device.".to_string()],
            allows_hosted_agents: true,
        }
    }
}

fn default_allows_hosted_agents() -> bool {
    true
}

/// Heartbeat outbound args. v4.1.7 carries the device's last-seen
/// hub-abilities revision so the hub can answer with an
/// incremental diff. v4.1.6 hubs ignore the field; v4.1.7 hubs
/// treat absent/zero as "fully out of date" and return the full
/// snapshot in the diff's `added`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HeartbeatArgs {
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub since_abilities_revision: u64,
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

/// Hub-broadcast contract diff returned in `HeartbeatReceipt`.
/// Empty `added` + empty `removed` at `revision >= since` means
/// the device is current.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct HubAbilitiesDiff {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub added: Vec<HubAbilityEntry>,
    #[serde(default)]
    pub removed: Vec<String>,
}

/// Arguments for `federation.advertise_agent`. The hosting
/// device-profile uses this to register hosted Agents (consent,
/// policy, mcp, llm-per-sub-agent) with the realm directory.
#[derive(Debug, Clone, Serialize)]
pub struct AdvertiseAgentArgs {
    pub agent_ura: String,
    /// Empty when the hosted Agent has no key of its own (the
    /// common case for §1.3 Model B; receipts are signed by the
    /// host's key, attested via host_attestation in the
    /// DirectoryEntry).
    #[serde(default)]
    pub public_key_hex: String,
    pub signing_authority: AdvertisedSigningAuthority,
    /// RFC-002 §5.2 forward_invoke routing key. The advertising
    /// daemon supplies its own runtime node_id so the hub knows
    /// which UDS-bound local-tool registration to dispatch into
    /// when an inbound forward arrives for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_node_id: Option<String>,
}

/// Wire shape for the `signing_authority` field. Mirrors the
/// hub-profile's `AdvertisedSigningAuthority` enum exactly.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdvertisedSigningAuthority {
    /// Agent owns its own keypair (Model A — hubs, backends).
    SelfSigned,
    /// Agent is hosted by another Agent that signs its receipts
    /// (Model B — every CLI-spawned hosted Agent).
    HostedBy { host_ura: String },
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AdvertiseAgentReceipt {
    pub ack: bool,
    pub replaced_prior: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct HeartbeatResponseHeader {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub permanent: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct HeartbeatRejectedNode {
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct HeartbeatReceipt {
    #[serde(default)]
    pub membership_status: String,
    #[serde(default)]
    pub realm_directory_size: u64,
    /// Axon proto-compatible response header. Older hub wrappers used
    /// top-level `permanent` / `status`; keep those aliases below so
    /// heartbeat callers can consume either bridge shape without
    /// reintroducing JSON inspection in the CLI state machine.
    #[serde(default)]
    pub header: Option<HeartbeatResponseHeader>,
    #[serde(default)]
    pub permanent: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub rejected_nodes: Vec<HeartbeatRejectedNode>,
    /// AXON-RFC-001 v4.1.7 hub-broadcast contract: incremental
    /// update of hub-published abilities since the caller's
    /// `since_abilities_revision`. Defaults to an empty diff at
    /// revision 0 so v4.1.6 hubs that omit the field produce a
    /// no-op on the client (no perceived churn).
    #[serde(default)]
    pub hub_abilities_diff: HubAbilitiesDiff,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ResolveArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<ResolveFilter>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ResolveFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_ura_prefix: Option<String>,
    /// When true, the Hub includes each agent's
    /// `advertised_abilities` in the receipt. Default false keeps
    /// the listing payload small for the common "is this agent
    /// alive" check; `<agent>.discover(scope: "easynet")` flips it
    /// to true so the LLM sees what each peer offers.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub include_abilities: bool,
    /// Tenant scoping (RFC-002 §5 update).
    ///   * `None` or `Some("")` → hub auto-fills with caller tenant
    ///                            (the safe "show me my own agents"
    ///                            default for `scope: "user"`).
    ///   * `Some("*")`           → cross-tenant catalog listing
    ///                            (`scope: "public"`).
    ///   * any other literal     → exact match on advertised tenant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_filter: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ResolvedAgent {
    pub ura: String,
    pub status: String,
    #[serde(default)]
    pub host_node_id: Option<String>,
    /// RFC-005 owner projection summaries returned by
    /// `federation.resolve(include_abilities=true)`. Empty when the
    /// resolve call did not request abilities. Each entry has the
    /// `AbilityProjectionSummary` JSON shape (`ability_ura`,
    /// `namespace`, `local_name`, descriptor/schema hashes, policy
    /// reference, tags); it is not a raw implementation descriptor.
    #[serde(default, rename = "abilities")]
    pub ability_summaries: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ResolveReceipt {
    pub agents: Vec<ResolvedAgent>,
}

/// Serialize args to the JSON bytes the bridge will pass as
/// `arguments`. Centralised so callers do not hand-roll
/// `serde_json::to_vec` (and silently drop fields if a struct
/// loses `Serialize` later).
pub fn args_to_bytes<A: Serialize>(args: &A) -> Vec<u8> {
    serde_json::to_vec(args).expect("federation.* arg serialization is infallible")
}

/// Parse the receipt body bytes the runtime returned. Wraps
/// `serde_json::from_slice` with a domain-specific error to make
/// shape mismatches identifiable in logs.
pub fn parse_receipt<R: serde::de::DeserializeOwned>(bytes: &[u8]) -> anyhow::Result<R> {
    serde_json::from_slice(bytes).map_err(|e| {
        let preview = std::str::from_utf8(bytes)
            .map(|s| s.chars().take(200).collect::<String>())
            .unwrap_or_else(|_| format!("{} bytes (non-utf8)", bytes.len()));
        anyhow::anyhow!(
            "federation.* receipt body did not match expected shape: {e}; body preview: {preview}"
        )
    })
}

/// Convenience: parse a receipt body that the bridge surfaces as a
/// `serde_json::Value` (e.g. from `ability_call_raw`'s response).
pub fn parse_receipt_value<R: serde::de::DeserializeOwned>(value: &Value) -> anyhow::Result<R> {
    serde_json::from_value(value.clone())
        .map_err(|e| anyhow::anyhow!("federation.* receipt value parse: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn join_args_serializes_with_required_fields_only() {
        let args = JoinArgs {
            realm: "acme".into(),
            membership_ura: "easynet:///r/acme/device/dev-a".into(),
            public_key_hex: "deadbeef".into(),
            pairing_secret: None,
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["realm"], "acme");
        assert_eq!(v["membership_ura"], "easynet:///r/acme/device/dev-a");
        assert_eq!(v["public_key_hex"], "deadbeef");
        assert!(
            v.get("pairing_secret").is_none(),
            "absent pairing_secret must NOT be emitted to keep the hub's parser strict",
        );
    }

    #[test]
    fn join_args_includes_pairing_secret_when_set() {
        let args = JoinArgs {
            realm: "acme".into(),
            membership_ura: "easynet:///r/acme/device/dev-a".into(),
            public_key_hex: "00".into(),
            pairing_secret: Some("token-xyz".into()),
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["pairing_secret"], "token-xyz");
    }

    #[test]
    fn advertise_args_serializes_self_signed_kind() {
        let args = AdvertiseAgentArgs {
            agent_ura: "easynet:///r/acme/device/01DEV".into(),
            public_key_hex: "aa".into(),
            signing_authority: AdvertisedSigningAuthority::SelfSigned,
            host_node_id: None,
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["signing_authority"]["kind"], "self_signed");
        assert_eq!(v["agent_ura"], "easynet:///r/acme/device/01DEV");
    }

    #[test]
    fn advertise_args_serializes_hosted_kind_with_host_ura() {
        let args = AdvertiseAgentArgs {
            agent_ura: "easynet:///r/acme/agent/u1.01LLM".into(),
            public_key_hex: "".into(),
            signing_authority: AdvertisedSigningAuthority::HostedBy {
                host_ura: "easynet:///r/acme/device/01DEV".into(),
            },
            host_node_id: None,
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["signing_authority"]["kind"], "hosted_by");
        assert_eq!(
            v["signing_authority"]["host_ura"],
            "easynet:///r/acme/device/01DEV"
        );
    }

    #[test]
    fn join_receipt_round_trips_through_serde() {
        let body = json!({
            "membership_ura": "easynet:///r/acme/device/01DEV",
            "realm": "acme",
            "join_receipt_hash": "abc123"
        });
        let parsed: JoinReceipt = parse_receipt_value(&body).unwrap();
        assert_eq!(parsed.membership_ura, "easynet:///r/acme/device/01DEV");
        assert_eq!(parsed.realm, "acme");
        assert_eq!(parsed.join_receipt_hash, "abc123");
    }

    #[test]
    fn parse_receipt_reports_shape_mismatch_with_preview() {
        let bytes = br#"{"unexpected":"shape"}"#;
        let err: anyhow::Error = parse_receipt::<JoinReceipt>(bytes).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("did not match expected shape"));
        // Preview must include the offending body so logs are
        // actionable without needing log-level=trace.
        assert!(msg.contains("unexpected"));
    }

    #[test]
    fn resolve_filter_omits_empty_filter() {
        // ResolveArgs::default has filter = None; the hub treats
        // missing filter as "no filter". We verify we do not emit
        // an empty `{}` that would deserialize differently.
        let args = ResolveArgs::default();
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert!(v.get("filter").is_none());
    }

    #[test]
    fn resolve_filter_emits_prefix_when_set() {
        let args = ResolveArgs {
            filter: Some(ResolveFilter {
                agent_ura_prefix: Some("easynet:///r/acme/".into()),
                include_abilities: false,
                tenant_filter: None,
            }),
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["filter"]["agent_ura_prefix"], "easynet:///r/acme/");
    }

    #[test]
    fn heartbeat_receipt_parses_minimal_body() {
        let body = json!({
            "membership_status": "active",
            "realm_directory_size": 3
        });
        let parsed: HeartbeatReceipt = parse_receipt_value(&body).unwrap();
        assert_eq!(parsed.membership_status, "active");
        assert_eq!(parsed.realm_directory_size, 3);
    }

    #[test]
    fn resolved_agents_list_parses_status_strings() {
        let body = json!({
            "agents": [
                {"ura": crate::ura::hub_ura("acme"), "status": "active"},
                {"ura": "easynet:///r/acme/device/4065c47a-ec6f-4330-87a5-0d69787709b8", "status": "revoked"}
            ]
        });
        let parsed: ResolveReceipt = parse_receipt_value(&body).unwrap();
        assert_eq!(parsed.agents.len(), 2);
        assert_eq!(parsed.agents[0].status, "active");
        assert_eq!(parsed.agents[1].status, "revoked");
    }

    #[test]
    fn resolved_agent_maps_wire_abilities_to_projection_summaries() {
        let body = json!({
            "agents": [{
                "ura": "easynet:///r/acme/agent/alice.bot",
                "status": "active",
                "abilities": [{
                    "ability_ura": "easynet:///r/acme/ability/alice.bot.chat",
                    "owner_ura": "easynet:///r/acme/agent/alice.bot",
                    "namespace": "",
                    "local_name": "chat",
                    "descriptor_revision": "sha256:descriptor",
                    "schema_ref": null,
                    "schema_hash": null,
                    "policy_ref": "visibility:SCOPED",
                    "route_summary_ref": null,
                    "tags": ["class:query"]
                }]
            }]
        });
        let parsed: ResolveReceipt = parse_receipt_value(&body).unwrap();
        assert_eq!(parsed.agents.len(), 1);
        assert_eq!(parsed.agents[0].ability_summaries.len(), 1);
        assert_eq!(parsed.agents[0].ability_summaries[0]["local_name"], "chat");
        assert_eq!(
            parsed.agents[0].ability_summaries[0]["ability_ura"],
            "easynet:///r/acme/ability/alice.bot.chat"
        );
    }
}
