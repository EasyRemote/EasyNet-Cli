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
// produced here to the daemon's canonical Invocation/session transport.
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
//      transport call site stays untouched.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::daemon::federation::receipt_contract::{
    AdvertiseContract, AuthorityAbilitiesDiff, AuthorityAbilityEntry, JoinReceipt,
};

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ResolveKeyReceipt {
    pub public_key_b64: String,
    pub public_key_hex: String,
    pub public_keys_b64: Vec<String>,
    #[serde(default)]
    pub principal_owner_ura: Option<String>,
    #[serde(default)]
    pub principal_owner_user_id: Option<String>,
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
    /// Optional product-neutral PrincipalLifecycle proof. When present, the
    /// hub validates it against daemon-owned PrincipalLifecycle state before
    /// binding the joined Device URA to the Principal URA in RuntimeTrust.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_enrollment: Option<PrincipalEnrollmentProof>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrincipalEnrollmentProof {
    pub principal_ura: String,
    pub proof: PrincipalProofRef,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrincipalProofRef {
    pub kind: String,
    pub reference: String,
}

// JoinReceipt, AuthorityAbilityEntry, AdvertiseContract, and AuthorityAbilitiesDiff are
// re-exported from `daemon::federation::receipt_contract` so hub producers and
// device consumers bind to one required-facts receipt shape.

/// Heartbeat outbound args. The request is the same canonical shape the hub
/// dispatch wrapper accepts: the caller revision plus the explicit owner
/// projection leases to refresh. Caller identity comes from the signed
/// invocation envelope, not from a request `agent_ura` alias.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatArgs {
    pub since_abilities_revision: u64,
    pub refresh_owner_uras: Vec<String>,
}

/// Arguments for `federation.advertise_agent`. The hosting
/// device-profile uses this to register hosted Agents (consent,
/// policy, mcp, llm-per-sub-agent) with the realm directory.
#[derive(Debug, Clone, Serialize)]
pub struct AdvertiseAgentArgs {
    pub agent_ura: String,
    /// Durable owner-cursor generation for this Agent incarnation.
    pub generation: u64,
    /// Empty when the hosted Agent has no key of its own (the
    /// common case for §1.3 Model B; receipts are signed by the
    /// host's key, attested via host_attestation in the
    /// DirectoryEntry).
    #[serde(default)]
    pub public_key_hex: String,
    pub signing_authority: AdvertisedSigningAuthority,
    /// Runtime node hosting the agent's canonical invocation endpoint.
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
#[serde(deny_unknown_fields)]
pub struct AdvertiseAgentReceipt {
    pub ack: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatResponseHeader {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub permanent: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatRejectedNode {
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatReceipt {
    #[serde(default)]
    pub membership_status: String,
    #[serde(default)]
    pub realm_directory_size: u64,
    /// Axon proto-compatible response header. This is the only status header
    /// projection accepted by the federation client contract.
    #[serde(default)]
    pub header: Option<HeartbeatResponseHeader>,
    #[serde(default)]
    pub rejected_nodes: Vec<HeartbeatRejectedNode>,
    /// AXON-RFC-001 v4.1.7 realm Authority broadcast contract: explicit incremental
    /// update of realm Authority-published abilities since the caller's
    /// `since_abilities_revision`. Empty `added` and `removed` arrays are
    /// valid only when the realm Authority serializes this diff with a revision.
    pub authority_abilities_diff: AuthorityAbilitiesDiff,
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
            principal_enrollment: None,
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["realm"], "acme");
        assert_eq!(v["membership_ura"], "easynet:///r/acme/device/dev-a");
        assert_eq!(v["public_key_hex"], "deadbeef");
        assert!(
            v.get("pairing_secret").is_none(),
            "retired pairing_secret must NOT be emitted to keep the hub parser strict",
        );
    }

    #[test]
    fn join_args_does_not_emit_retired_pairing_secret() {
        let args = JoinArgs {
            realm: "acme".into(),
            membership_ura: "easynet:///r/acme/device/dev-a".into(),
            public_key_hex: "00".into(),
            principal_enrollment: None,
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert!(
            v.get("pairing_secret").is_none(),
            "pairing_secret is a retired product token carrier, not a runtime join fact",
        );
    }

    #[test]
    fn join_args_can_carry_product_neutral_principal_enrollment_proof() {
        let args = JoinArgs {
            realm: "acme".into(),
            membership_ura: "easynet:///r/acme/device/dev-a".into(),
            public_key_hex: "00".into(),
            principal_enrollment: Some(PrincipalEnrollmentProof {
                principal_ura: "easynet:///r/acme/user/alice".into(),
                proof: PrincipalProofRef {
                    kind: "active_key".into(),
                    reference: "binding-1".into(),
                },
            }),
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(
            v["principal_enrollment"]["principal_ura"],
            "easynet:///r/acme/user/alice"
        );
        assert_eq!(v["principal_enrollment"]["proof"]["kind"], "active_key");
        assert_eq!(v["principal_enrollment"]["proof"]["reference"], "binding-1");
        assert!(
            v.get("username").is_none() && v.get("user_id").is_none(),
            "federation.join must not grow product account fields"
        );
    }

    #[test]
    fn principal_enrollment_proof_rejects_product_account_aliases() {
        for field in ["user_id", "username", "device_ura"] {
            let mut body = serde_json::Map::new();
            body.insert(
                "principal_ura".to_string(),
                json!("easynet:///r/acme/user/alice"),
            );
            body.insert(
                "proof".to_string(),
                json!({
                    "kind": "active_key",
                    "reference": "binding-1"
                }),
            );
            body.insert(field.to_string(), json!("retired"));

            let err = serde_json::from_value::<PrincipalEnrollmentProof>(Value::Object(body))
                .expect_err("principal enrollment proof must reject product account aliases");

            assert!(
                err.to_string().contains(field),
                "retired field {field:?} must be named in parse error: {err}"
            );
        }
    }

    #[test]
    fn principal_proof_ref_rejects_unknown_proof_handle_fields() {
        let body = json!({
            "kind": "active_key",
            "reference": "binding-1",
            "key_id": "retired"
        });

        let err = serde_json::from_value::<PrincipalProofRef>(body)
            .expect_err("principal proof ref must reject retired key handles");

        assert!(err.to_string().contains("key_id"));
    }

    #[test]
    fn resolve_key_receipt_parses_canonical_key_facts() {
        let body = json!({
            "public_key_b64": "pub-b64",
            "public_key_hex": "707562",
            "public_keys_b64": ["pub-b64", "rotated-b64"],
            "principal_owner_ura": "easynet:///r/acme/user/alice",
            "principal_owner_user_id": "alice"
        });

        let parsed: ResolveKeyReceipt = parse_receipt_value(&body).unwrap();

        assert_eq!(parsed.public_key_b64, "pub-b64");
        assert_eq!(parsed.public_key_hex, "707562");
        assert_eq!(parsed.public_keys_b64, vec!["pub-b64", "rotated-b64"]);
        assert_eq!(
            parsed.principal_owner_ura.as_deref(),
            Some("easynet:///r/acme/user/alice")
        );
        assert_eq!(parsed.principal_owner_user_id.as_deref(), Some("alice"));
    }

    #[test]
    fn resolve_key_receipt_requires_schema_bound_key_set() {
        let body = json!({
            "public_key_b64": "pub-b64",
            "public_key_hex": "707562"
        });

        let err = parse_receipt_value::<ResolveKeyReceipt>(&body)
            .expect_err("resolve_key receipt must not repair legacy single-key facts");

        assert!(
            err.to_string().contains("public_keys_b64"),
            "missing canonical key set must fail closed: {err}"
        );
    }

    #[test]
    fn resolve_key_receipt_rejects_retired_directory_status_fields() {
        for field in ["agent_ura", "status", "key_id", "rotation_epoch"] {
            let mut body = serde_json::Map::new();
            body.insert("public_key_b64".to_string(), json!("pub-b64"));
            body.insert("public_key_hex".to_string(), json!("707562"));
            body.insert("public_keys_b64".to_string(), json!(["pub-b64"]));
            body.insert(field.to_string(), json!("retired"));

            let err = parse_receipt_value::<ResolveKeyReceipt>(&Value::Object(body))
                .expect_err("retired resolve_key receipt fields must fail closed");

            assert!(
                err.to_string().contains(field),
                "retired field {field:?} must be named in parse error: {err}"
            );
        }
    }

    #[test]
    fn advertise_args_serializes_self_signed_kind() {
        let args = AdvertiseAgentArgs {
            agent_ura: "easynet:///r/acme/device/01DEV".into(),
            generation: 1,
            public_key_hex: "aa".into(),
            signing_authority: AdvertisedSigningAuthority::SelfSigned,
            host_node_id: None,
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["signing_authority"]["kind"], "self_signed");
        assert_eq!(v["agent_ura"], "easynet:///r/acme/device/01DEV");
        assert_eq!(v["generation"], 1);
    }

    #[test]
    fn advertise_args_serializes_hosted_kind_with_host_ura() {
        let args = AdvertiseAgentArgs {
            agent_ura: "easynet:///r/acme/agent/u1.01LLM".into(),
            generation: 7,
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
        assert_eq!(v["generation"], 7);
    }

    #[test]
    fn advertise_agent_receipt_rejects_retired_fields() {
        for field in ["status", "agent_ura", "replaced_prior"] {
            let mut body = serde_json::Map::new();
            body.insert("ack".to_string(), json!(true));
            body.insert(field.to_string(), json!("retired"));

            let err = parse_receipt_value::<AdvertiseAgentReceipt>(&Value::Object(body))
                .expect_err("advertise receipt must reject retired fields");

            assert!(
                err.to_string().contains(field),
                "retired field {field:?} must be named in parse error: {err}"
            );
        }
    }

    #[test]
    fn heartbeat_args_are_closed_canonical_request_shape() {
        let args = HeartbeatArgs {
            since_abilities_revision: 7,
            refresh_owner_uras: vec!["easynet:///r/acme/device/01DEV".into()],
        };
        let v: Value = serde_json::from_slice(&args_to_bytes(&args)).unwrap();
        assert_eq!(v["since_abilities_revision"], 7);
        assert_eq!(v["refresh_owner_uras"][0], "easynet:///r/acme/device/01DEV");
        assert!(v.get("agent_ura").is_none());
        let mut retired = v.as_object().expect("object").clone();
        retired.insert(
            "agent_ura".into(),
            Value::String("easynet:///r/acme/device/01DEV".into()),
        );
        let error = serde_json::from_value::<HeartbeatArgs>(Value::Object(retired))
            .expect_err("retired heartbeat agent_ura must fail closed");
        assert!(error.to_string().contains("agent_ura"));
    }

    #[test]
    fn join_receipt_round_trips_with_required_runtime_facts() {
        let body = json!({
            "membership_ura": "easynet:///r/acme/device/01DEV",
            "realm": "acme",
            "join_receipt_hash": "abc123",
            "authority_published_abilities": [],
            "authority_abilities_revision": 0,
            "advertise_contract": {
                "allowed_owner_prefixes": ["device."],
                "allows_hosted_agents": true
            }
        });
        let parsed: JoinReceipt = parse_receipt_value(&body).unwrap();
        assert_eq!(parsed.membership_ura, "easynet:///r/acme/device/01DEV");
        assert_eq!(parsed.realm, "acme");
        assert_eq!(parsed.join_receipt_hash, "abc123");
        assert_eq!(parsed.authority_abilities_revision, 0);
        assert!(parsed.authority_published_abilities.is_empty());
        assert_eq!(
            parsed.advertise_contract.allowed_owner_prefixes,
            vec!["device.".to_string()]
        );
    }

    #[test]
    fn join_receipt_rejects_missing_authority_runtime_facts() {
        let body = json!({
            "membership_ura": "easynet:///r/acme/device/01DEV",
            "realm": "acme",
            "join_receipt_hash": "abc123"
        });
        let err = parse_receipt_value::<JoinReceipt>(&body).unwrap_err();
        assert!(
            err.to_string().contains("authority_published_abilities"),
            "missing Authority snapshot must fail closed: {err}"
        );
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
    fn heartbeat_receipt_parses_explicit_empty_authority_diff() {
        let body = json!({
            "membership_status": "active",
            "realm_directory_size": 3,
            "authority_abilities_diff": {
                "revision": 0,
                "added": [],
                "removed": []
            }
        });
        let parsed: HeartbeatReceipt = parse_receipt_value(&body).unwrap();
        assert_eq!(parsed.membership_status, "active");
        assert_eq!(parsed.realm_directory_size, 3);
        assert_eq!(parsed.authority_abilities_diff.revision, 0);
        assert!(parsed.authority_abilities_diff.added.is_empty());
    }

    #[test]
    fn heartbeat_receipt_rejects_missing_authority_diff() {
        let body = json!({
            "membership_status": "active",
            "realm_directory_size": 3
        });
        let err = parse_receipt_value::<HeartbeatReceipt>(&body).unwrap_err();
        assert!(
            err.to_string().contains("authority_abilities_diff"),
            "missing Authority ability diff must fail closed: {err}"
        );
    }

    #[test]
    fn heartbeat_receipt_rejects_retired_top_level_status_aliases() {
        for field in ["status", "permanent"] {
            let mut body = serde_json::Map::new();
            body.insert("membership_status".to_string(), json!("active"));
            body.insert("realm_directory_size".to_string(), json!(3));
            body.insert(
                "authority_abilities_diff".to_string(),
                json!({
                    "revision": 0,
                    "added": [],
                    "removed": []
                }),
            );
            body.insert(field.to_string(), json!("retired"));

            let err = parse_receipt_value::<HeartbeatReceipt>(&Value::Object(body))
                .expect_err("retired heartbeat aliases must fail closed");

            assert!(
                err.to_string().contains(field),
                "retired field {field:?} must be named in parse error: {err}"
            );
        }
    }

    #[test]
    fn resolved_agents_list_parses_status_strings() {
        let body = json!({
            "agents": [
                {"ura": crate::core::ura::hub_ura("acme"), "status": "active"},
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

    #[test]
    fn resolve_receipt_rejects_top_level_compat_agent_lists() {
        for field in ["items", "results", "directory"] {
            let mut body = serde_json::Map::new();
            body.insert("agents".to_string(), json!([]));
            body.insert(field.to_string(), json!([]));

            let err = parse_receipt_value::<ResolveReceipt>(&Value::Object(body))
                .expect_err("resolve receipt must reject alternate agent list aliases");

            assert!(
                err.to_string().contains(field),
                "retired field {field:?} must be named in parse error: {err}"
            );
        }
    }

    #[test]
    fn resolved_agent_rejects_retired_identity_and_directory_aliases() {
        for field in ["agent_ura", "node_id", "tenant_id"] {
            let mut agent = serde_json::Map::new();
            agent.insert(
                "ura".to_string(),
                json!("easynet:///r/acme/agent/alice.bot"),
            );
            agent.insert("status".to_string(), json!("active"));
            agent.insert(field.to_string(), json!("retired"));
            let body = json!({ "agents": [Value::Object(agent)] });

            let err = parse_receipt_value::<ResolveReceipt>(&body)
                .expect_err("resolved agent row must reject retired aliases");

            assert!(
                err.to_string().contains(field),
                "retired field {field:?} must be named in parse error: {err}"
            );
        }
    }
}
