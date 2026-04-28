// EasyNet CLI — Federation Advertise (RFC-001 §3 step 6)
// =========================================================
//
// File: src/runtime/advertise.rs
//
// Wraps `bridge.ability_call_raw` for the two federation.advertise_*
// abilities the daemon calls at boot to register itself + its
// hosted Agents in the realm directory:
//
//   federation.advertise_agent      — one call per Agent (the
//                                      device-profile + each
//                                      consent/policy/mcp/llm
//                                      hosted Agent the daemon
//                                      runs).
//   federation.advertise_abilities  — bulk descriptor publication
//                                      so peers can discover what
//                                      this daemon offers without
//                                      invoking anything.
//
// What this module IS
// -------------------
// A typed, testable wrapper around the bridge's raw ability_call.
// It builds the §1.4 / §1.6 wire payloads using `federation_client`
// shapes, hands them to the bridge, parses the receipt body, and
// returns a strongly-typed outcome the daemon-boot path consumes.
//
// What this module is NOT
// -----------------------
// - Not the daemon-boot caller. P4.7 wires this into the actual
//   boot sequence (mint URAs from local-agents.json, then call
//   advertise_*). For now this module is callable from any context
//   that has a live bridge.
// - Not a replacement for `publish.rs` yet. P4.7 deletes
//   publish.rs's Ok(false) stubs in favor of calls into here.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::runtime::ability_descriptor::AbilityDescriptor;
use crate::runtime::federation_client::{
    AdvertiseAgentArgs, AdvertiseAgentReceipt, AdvertisedSigningAuthority, args_to_bytes,
    parse_receipt_value,
};
use serde::Serialize;
use serde_json::Value;

/// One advertise outcome — either the parsed receipt or a
/// descriptive error. Mirrors `PublishOutcome` from publish.rs but
/// carries the typed receipt for callers that act on it.
#[derive(Debug)]
pub struct AdvertiseOutcome {
    pub agent_uri: String,
    pub result: Result<AdvertiseAgentReceipt, String>,
}

/// Resource URI form expected by the bridge for federation.* calls
/// against the hub-profile Agent. `<realm>` is filled at call time
/// from the daemon's join receipt; `<hub_uri>` segment is fixed
/// because every realm has exactly one canonical hub-profile Agent.
// Visibility token must be one of `pub | org | prv` per the SDK
// canonicalizer (client-sdk/src/domain/easynet/semantic.rs); the
// pre-existing `private` literal silently failed canonicalization
// with `invalid visibility for r easynet URI` on every advertise
// call — operators saw "advertise X failed" with no entry in the
// realm directory. Hub-profile URIs are not public, so we use `prv`.
const FED_ADVERTISE_AGENT_RESOURCE_FMT: &str =
    "easynet:///r/prv/hub/{realm}/abilities/federation.advertise_agent@1?tenant_id={tenant}";

const FED_ADVERTISE_ABILITIES_RESOURCE_FMT: &str =
    "easynet:///r/prv/hub/{realm}/abilities/federation.advertise_abilities@1?tenant_id={tenant}";

/// Build the canonical resource URI for `federation.advertise_agent`
/// against the realm's hub. Public so call sites can construct the
/// URI consistently without re-typing the format string.
///
/// `tenant_id` is mandatory: the SDK canonicalizer rejects any non-
/// `pub` URI that does not carry `?tenant_id=...` and it must match
/// the envelope tenant. Building the query here keeps every advertise
/// caller from re-implementing it.
pub fn advertise_agent_resource_uri(realm: &str, tenant_id: &str) -> String {
    FED_ADVERTISE_AGENT_RESOURCE_FMT
        .replace("{realm}", realm)
        .replace("{tenant}", tenant_id)
}

pub fn advertise_abilities_resource_uri(realm: &str, tenant_id: &str) -> String {
    FED_ADVERTISE_ABILITIES_RESOURCE_FMT
        .replace("{realm}", realm)
        .replace("{tenant}", tenant_id)
}

/// Wire shape for `federation.advertise_abilities` arguments. Not in
/// federation_client.rs because it embeds AbilityDescriptor — keeping
/// the dependency direction one-way (advertise.rs → federation_client,
/// never the reverse) preserves federation_client as a pure-data crate.
#[derive(Debug, Serialize)]
struct AdvertiseAbilitiesArgs<'a> {
    agent_uri: &'a str,
    abilities: &'a [AbilityDescriptor],
}

/// Bridge handle abstraction. Sized to what advertise.rs needs;
/// production callers pass `BridgeAbilityInvoker`, tests pass a fake.
pub trait AbilityInvoker {
    /// Invoke the named ability. Returns the JSON receipt body on
    /// success, or a string-form error on any failure (transport,
    /// admission, hub-side rejection — all collapsed because the
    /// caller's recovery is the same in every case: log + continue).
    fn invoke_ability(
        &self,
        tenant_id: &str,
        resource_uri: &str,
        payload_json: Value,
    ) -> Result<Value, String>;
}

/// Production adapter wrapping the SDK's `DendriteBridge`. Sits in
/// this module so the daemon-boot wiring imports a single trait
/// implementer instead of having to hand-roll the
/// `ability_call_raw` invocation per call site.
pub struct BridgeAbilityInvoker<'a> {
    bridge: &'a easynet_axon::dendrite_bridge::DendriteBridge,
    /// Per-call timeout. Boot-time advertise must not block
    /// indefinitely if the runtime is wedged — operators see a
    /// failed advertise in logs and re-run later.
    pub timeout_ms: u64,
}

impl<'a> BridgeAbilityInvoker<'a> {
    pub fn new(bridge: &'a easynet_axon::dendrite_bridge::DendriteBridge) -> Self {
        Self {
            bridge,
            // Generous default for advertise — runtime IPC is local
            // and a 5-second budget covers ordinary cold-start
            // latency without making startup hang.
            timeout_ms: 5_000,
        }
    }
}

impl<'a> AbilityInvoker for BridgeAbilityInvoker<'a> {
    fn invoke_ability(
        &self,
        tenant_id: &str,
        resource_uri: &str,
        payload_json: Value,
    ) -> Result<Value, String> {
        self.bridge
            .ability_call_raw(
                tenant_id,
                resource_uri,
                payload_json,
                None,
                None,
                self.timeout_ms,
            )
            .map_err(|e| format!("{e}"))
    }
}

/// Build the `federation.advertise_agent` payload + invoke it.
/// Returns the typed receipt on success.
pub fn advertise_agent<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    args: &AdvertiseAgentArgs,
) -> AdvertiseOutcome {
    let resource_uri = advertise_agent_resource_uri(realm, tenant_id);
    let payload: Value = match serde_json::from_slice(&args_to_bytes(args)) {
        Ok(v) => v,
        Err(e) => {
            return AdvertiseOutcome {
                agent_uri: args.agent_uri.clone(),
                result: Err(format!("encode advertise_agent args: {e}")),
            };
        }
    };
    match invoker.invoke_ability(tenant_id, &resource_uri, payload) {
        Ok(receipt_body) => match parse_receipt_value::<AdvertiseAgentReceipt>(&receipt_body) {
            Ok(parsed) => AdvertiseOutcome {
                agent_uri: args.agent_uri.clone(),
                result: Ok(parsed),
            },
            Err(e) => AdvertiseOutcome {
                agent_uri: args.agent_uri.clone(),
                result: Err(format!("parse advertise_agent receipt: {e}")),
            },
        },
        Err(e) => AdvertiseOutcome {
            agent_uri: args.agent_uri.clone(),
            result: Err(e),
        },
    }
}

/// Bulk-publish a list of AbilityDescriptors for one Agent. Mirrors
/// the §3 step 6 sequence: advertise_agent → advertise_abilities.
pub fn advertise_abilities<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_uri: &str,
    abilities: &[AbilityDescriptor],
) -> Result<Value, String> {
    let resource_uri = advertise_abilities_resource_uri(realm, tenant_id);
    let args = AdvertiseAbilitiesArgs {
        agent_uri,
        abilities,
    };
    let payload =
        serde_json::to_value(&args).map_err(|e| format!("encode advertise_abilities args: {e}"))?;
    invoker.invoke_ability(tenant_id, &resource_uri, payload)
}

/// Convenience: advertise the device-profile Agent itself (Selfsigned
/// Model A per §1.3) as a single helper since it's the very first
/// call any daemon makes after federation.join.
pub fn advertise_self_signed_device<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_uri: &str,
    public_key_hex: &str,
) -> AdvertiseOutcome {
    let args = AdvertiseAgentArgs {
        agent_uri: agent_uri.to_string(),
        public_key_hex: public_key_hex.to_string(),
        signing_authority: AdvertisedSigningAuthority::SelfSigned,
    };
    advertise_agent(invoker, tenant_id, realm, &args)
}

/// Convenience: advertise a hosted Agent (Model B) under the host
/// device-profile. Used for consent/policy/mcp/llm-profile Agents.
pub fn advertise_hosted_agent<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_uri: &str,
    host_uri: &str,
) -> AdvertiseOutcome {
    let args = AdvertiseAgentArgs {
        agent_uri: agent_uri.to_string(),
        public_key_hex: String::new(),
        signing_authority: AdvertisedSigningAuthority::HostedBy {
            host_uri: host_uri.to_string(),
        },
    };
    advertise_agent(invoker, tenant_id, realm, &args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};
    use std::cell::RefCell;

    /// In-memory fake invoker that records every call and returns a
    /// canned reply. Lets us assert the resource URI shape, the
    /// payload contents, and the receipt-parsing path without a
    /// running axon-runtime.
    struct RecordingInvoker {
        last_resource_uri: RefCell<Option<String>>,
        last_payload: RefCell<Option<Value>>,
        reply: Value,
    }

    impl RecordingInvoker {
        fn new(reply: Value) -> Self {
            Self {
                last_resource_uri: RefCell::new(None),
                last_payload: RefCell::new(None),
                reply,
            }
        }
    }

    impl AbilityInvoker for RecordingInvoker {
        fn invoke_ability(
            &self,
            _tenant_id: &str,
            resource_uri: &str,
            payload_json: Value,
        ) -> Result<Value, String> {
            *self.last_resource_uri.borrow_mut() = Some(resource_uri.to_string());
            *self.last_payload.borrow_mut() = Some(payload_json);
            Ok(self.reply.clone())
        }
    }

    struct AlwaysFails;
    impl AbilityInvoker for AlwaysFails {
        fn invoke_ability(
            &self,
            _tenant_id: &str,
            _resource_uri: &str,
            _payload_json: Value,
        ) -> Result<Value, String> {
            Err("transport timeout".into())
        }
    }

    #[test]
    fn resource_uri_substitutes_realm_correctly() {
        assert_eq!(
            advertise_agent_resource_uri("acme", "tenant-1"),
            "easynet:///r/prv/hub/acme/abilities/federation.advertise_agent@1?tenant_id=tenant-1"
        );
        assert_eq!(
            advertise_abilities_resource_uri("contoso", "tenant-2"),
            "easynet:///r/prv/hub/contoso/abilities/federation.advertise_abilities@1?tenant_id=tenant-2"
        );
    }

    #[test]
    fn advertise_self_signed_device_emits_correct_payload_shape() {
        let invoker = RecordingInvoker::new(serde_json::json!({
            "ack": true,
            "replaced_prior": false
        }));
        let outcome = advertise_self_signed_device(
            &invoker,
            "tenant",
            "acme",
            "easynet:///r/acme/agent/01DEV",
            "deadbeef",
        );
        let receipt = outcome.result.expect("must succeed");
        assert!(receipt.ack);
        assert!(!receipt.replaced_prior);

        let payload = invoker.last_payload.borrow().clone().unwrap();
        assert_eq!(payload["agent_uri"], "easynet:///r/acme/agent/01DEV");
        assert_eq!(payload["public_key_hex"], "deadbeef");
        assert_eq!(payload["signing_authority"]["kind"], "self_signed");
    }

    #[test]
    fn advertise_hosted_agent_emits_hosted_by_with_host_uri() {
        let invoker = RecordingInvoker::new(serde_json::json!({
            "ack": true,
            "replaced_prior": true
        }));
        let outcome = advertise_hosted_agent(
            &invoker,
            "tenant",
            "acme",
            "easynet:///r/acme/agent/01LLM",
            "easynet:///r/acme/agent/01DEV",
        );
        let receipt = outcome.result.expect("must succeed");
        assert!(receipt.replaced_prior);

        let payload = invoker.last_payload.borrow().clone().unwrap();
        assert_eq!(payload["signing_authority"]["kind"], "hosted_by");
        assert_eq!(
            payload["signing_authority"]["host_uri"],
            "easynet:///r/acme/agent/01DEV"
        );
        assert_eq!(payload["public_key_hex"], "");
    }

    #[test]
    fn advertise_returns_err_outcome_when_invoker_fails() {
        let outcome = advertise_self_signed_device(
            &AlwaysFails,
            "tenant",
            "acme",
            "easynet:///r/acme/agent/01DEV",
            "00",
        );
        let err = outcome.result.expect_err("must surface invoker error");
        assert!(err.contains("transport timeout"));
        assert_eq!(outcome.agent_uri, "easynet:///r/acme/agent/01DEV");
    }

    #[test]
    fn advertise_returns_err_when_receipt_shape_mismatches() {
        let invoker = RecordingInvoker::new(serde_json::json!({
            "unexpected": "shape"
        }));
        let outcome = advertise_self_signed_device(
            &invoker,
            "tenant",
            "acme",
            "easynet:///r/acme/agent/01DEV",
            "00",
        );
        let err = outcome.result.expect_err("malformed receipt must surface");
        assert!(err.contains("parse advertise_agent receipt"));
    }

    #[test]
    fn advertise_abilities_serializes_descriptors_in_payload() {
        let invoker = RecordingInvoker::new(serde_json::json!({"ack": true}));
        let descriptors = vec![
            AbilityDescriptor::new(
                "observe.health",
                "easynet:///r/acme/agent/01DEV",
                Visibility::Public,
            )
            .unwrap()
            .with_source("kernel:built-in"),
            AbilityDescriptor::new(
                "fleet.list_agents",
                "easynet:///r/acme/agent/01DEV",
                Visibility::Scoped,
            )
            .unwrap(),
        ];
        let _ = advertise_abilities(
            &invoker,
            "tenant",
            "acme",
            "easynet:///r/acme/agent/01DEV",
            &descriptors,
        )
        .expect("must succeed");

        let payload = invoker.last_payload.borrow().clone().unwrap();
        assert_eq!(payload["agent_uri"], "easynet:///r/acme/agent/01DEV");
        let abilities = payload["abilities"].as_array().expect("abilities must be array");
        assert_eq!(abilities.len(), 2);
        assert_eq!(abilities[0]["name"], "observe.health");
        assert_eq!(abilities[0]["visibility"], "PUBLIC");
        assert_eq!(abilities[1]["visibility"], "SCOPED");
    }

    #[test]
    fn advertise_abilities_targets_correct_resource_uri() {
        let invoker = RecordingInvoker::new(serde_json::json!({"ack": true}));
        let _ = advertise_abilities(&invoker, "tenant", "acme", "u", &[]).unwrap();
        assert_eq!(
            invoker.last_resource_uri.borrow().as_deref().unwrap(),
            "easynet:///r/prv/hub/acme/abilities/federation.advertise_abilities@1?tenant_id={tenant}"
        );
    }
}
