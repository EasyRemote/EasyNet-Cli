// EasyNet CLI — Federation Ability Client (RFC-001 §3, §1.4)
// ============================================================
//
// File: src/runtime/federation_client.rs
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
// `publish.rs` or the daemon boot path) gives us:
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

/// Arguments for `federation.join`. Matches the hub-profile's
/// `JoinArgs` struct in
/// `EasyNet-Axon/core/runtime-rs/src/services/invocation/hub_profile.rs`.
#[derive(Debug, Clone, Serialize)]
pub struct JoinArgs {
    pub realm: String,
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
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct JoinReceipt {
    pub canonical_agent_uri: String,
    pub realm: String,
    pub join_receipt_hash: String,
}

/// Arguments for `federation.advertise_agent`. The hosting
/// device-profile uses this to register hosted Agents (consent,
/// policy, mcp, llm-per-sub-agent) with the realm directory.
#[derive(Debug, Clone, Serialize)]
pub struct AdvertiseAgentArgs {
    pub agent_uri: String,
    /// Empty when the hosted Agent has no key of its own (the
    /// common case for §1.3 Model B; receipts are signed by the
    /// host's key, attested via host_attestation in the
    /// DirectoryEntry).
    #[serde(default)]
    pub public_key_hex: String,
    pub signing_authority: AdvertisedSigningAuthority,
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
    HostedBy { host_uri: String },
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct AdvertiseAgentReceipt {
    pub ack: bool,
    pub replaced_prior: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct HeartbeatReceipt {
    pub membership_status: String,
    pub realm_directory_size: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ResolveArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<ResolveFilter>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ResolveFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_uri_prefix: Option<String>,
    /// When true, the Hub includes each agent's
    /// `advertised_abilities` in the receipt. Default false keeps
    /// the listing payload small for the common "is this agent
    /// alive" check; `<self>.discover(scope: "easynet")` flips it
    /// to true so the LLM sees what each peer offers.
    #[serde(skip_serializing_if = "std::ops::Not::not", default)]
    pub include_abilities: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ResolvedAgent {
    pub uri: String,
    pub status: String,
    /// Per-ability descriptors as advertised through
    /// `federation.advertise_abilities`. Empty when the resolve
    /// call did not pass `include_abilities = true`. Each entry
    /// is a JSON object preserving whatever shape the publisher
    /// emitted (name, description, input_schema, …).
    #[serde(default)]
    pub abilities: Vec<Value>,
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
            public_key_hex: "deadbeef".into(),
            pairing_secret: None,
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["realm"], "acme");
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
            public_key_hex: "00".into(),
            pairing_secret: Some("token-xyz".into()),
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["pairing_secret"], "token-xyz");
    }

    #[test]
    fn advertise_args_serializes_self_signed_kind() {
        let args = AdvertiseAgentArgs {
            agent_uri: "easynet:///r/acme/agent/01DEV".into(),
            public_key_hex: "aa".into(),
            signing_authority: AdvertisedSigningAuthority::SelfSigned,
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["signing_authority"]["kind"], "self_signed");
        assert_eq!(v["agent_uri"], "easynet:///r/acme/agent/01DEV");
    }

    #[test]
    fn advertise_args_serializes_hosted_kind_with_host_uri() {
        let args = AdvertiseAgentArgs {
            agent_uri: "easynet:///r/acme/agent/01LLM".into(),
            public_key_hex: "".into(),
            signing_authority: AdvertisedSigningAuthority::HostedBy {
                host_uri: "easynet:///r/acme/agent/01DEV".into(),
            },
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["signing_authority"]["kind"], "hosted_by");
        assert_eq!(
            v["signing_authority"]["host_uri"],
            "easynet:///r/acme/agent/01DEV"
        );
    }

    #[test]
    fn join_receipt_round_trips_through_serde() {
        let body = json!({
            "canonical_agent_uri": "easynet:///r/acme/agent/01DEV",
            "realm": "acme",
            "join_receipt_hash": "abc123"
        });
        let parsed: JoinReceipt = parse_receipt_value(&body).unwrap();
        assert_eq!(parsed.canonical_agent_uri, "easynet:///r/acme/agent/01DEV");
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
                agent_uri_prefix: Some("easynet:///r/acme/".into()),
                include_abilities: false,
            }),
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["filter"]["agent_uri_prefix"], "easynet:///r/acme/");
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
                {"uri": "easynet:///r/acme/agent/01HUB", "status": "active"},
                {"uri": "easynet:///r/acme/agent/01OLD", "status": "revoked"}
            ]
        });
        let parsed: ResolveReceipt = parse_receipt_value(&body).unwrap();
        assert_eq!(parsed.agents.len(), 2);
        assert_eq!(parsed.agents[0].status, "active");
        assert_eq!(parsed.agents[1].status, "revoked");
    }
}
