// EasyNet CLI — Federation Advertise (RFC-001 §3 step 6)
// =========================================================
//
// File: src/daemon/federation/advertise.rs
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
// It builds the §1.4 / §1.6 wire payloads using `ability_contract`
// shapes, hands them to the bridge, parses the receipt body, and
// returns a strongly-typed outcome the daemon-boot path consumes.
//
// What this module is NOT
// -----------------------
// - Not the daemon-boot caller. P4.7 wires this into the actual
//   boot sequence (mint URAs from local-agents.json, then call
//   advertise_*). For now this module is callable from any context
//   that has a live bridge.
// - Not a replacement for the higher-level publish orchestration.
//   That path decides which local/hosted agents to advertise; this
//   module only owns typed federation ability calls.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use crate::daemon::ability::descriptors::AbilityDescriptor;
use crate::daemon::federation::client::ability_contract::{
    args_to_bytes, parse_receipt_value, AdvertiseAgentArgs, AdvertiseAgentReceipt,
    AdvertisedSigningAuthority, ResolveArgs, ResolveFilter, ResolveReceipt, ResolvedAgent,
};
use crate::daemon::federation::read_model::owner_projection::AbilityProjectionSummary;
use serde::Serialize;
use serde_json::Value;

/// One advertise outcome — either the parsed receipt or a
/// descriptive error. Mirrors `PublishOutcome` from publish.rs but
/// carries the typed receipt for callers that act on it.
#[derive(Debug)]
pub struct AdvertiseOutcome {
    pub agent_ura: String,
    pub result: Result<AdvertiseAgentReceipt, String>,
}

const FED_ADVERTISE_AGENT_ABILITY_NAME: &str = "federation.advertise_agent";
const FED_ADVERTISE_ABILITIES_ABILITY_NAME: &str = "federation.advertise_abilities";
const FED_RESOLVE_ABILITY_NAME: &str = "federation.resolve";
const FED_REVOKE_ABILITY_NAME: &str = "federation.revoke";
const FED_HEARTBEAT_ABILITY_NAME: &str = "federation.heartbeat";
const FED_RESOLVE_KEY_ABILITY_NAME: &str = "federation.resolve_key";

fn hub_ability_resource_ura(realm: &str, ability_name: &str) -> String {
    crate::core::ura::hub_ability_ura(realm, ability_name)
}

/// Build the canonical hub-owned ability URA for
/// `federation.advertise_agent`.
pub fn advertise_agent_resource_ura(realm: &str, _tenant_id: &str) -> String {
    let _ = _tenant_id;
    hub_ability_resource_ura(realm, FED_ADVERTISE_AGENT_ABILITY_NAME)
}

pub fn advertise_abilities_resource_ura(realm: &str, _tenant_id: &str) -> String {
    let _ = _tenant_id;
    hub_ability_resource_ura(realm, FED_ADVERTISE_ABILITIES_ABILITY_NAME)
}

/// Build the canonical hub-owned ability URA for
/// `federation.resolve`. Inbound counterpart to `advertise_*`:
/// peers query this to discover what every other agent in the realm
/// has published.
pub fn resolve_resource_ura(realm: &str, _tenant_id: &str) -> String {
    let _ = _tenant_id;
    hub_ability_resource_ura(realm, FED_RESOLVE_ABILITY_NAME)
}

/// Build the canonical hub-owned ability URA for
/// `federation.revoke`. Used by the CLI shutdown path to remove the
/// daemon's own directory entry.
pub fn revoke_resource_ura(realm: &str, _tenant_id: &str) -> String {
    let _ = _tenant_id;
    hub_ability_resource_ura(realm, FED_REVOKE_ABILITY_NAME)
}

/// Build the canonical hub-owned ability URA for
/// `federation.heartbeat`. Periodic invocation keeps the daemon's
/// directory entry alive.
pub fn heartbeat_resource_ura(realm: &str, _tenant_id: &str) -> String {
    let _ = _tenant_id;
    hub_ability_resource_ura(realm, FED_HEARTBEAT_ABILITY_NAME)
}

/// Wire shape for `federation.advertise_abilities` arguments. Not in
/// ability_contract.rs because this is CLI owner-projection publication:
/// descriptors are local input, while the wire carries bounded summaries and
/// projection metadata only.
#[derive(Debug, Serialize)]
struct AdvertiseAbilitiesArgs<'a> {
    agent_ura: &'a str,
    owner_ura: &'a str,
    host_device_ura: &'a str,
    projection_revision: u64,
    projection_digest: &'a str,
    lease_expires_unix_ms: i64,
    ability_summaries: &'a [AbilityProjectionSummary],
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
        resource_ura: &str,
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
    /// Caller URA to stamp on every envelope this invoker emits
    /// when the resource URA's subject is hub-shaped. The bridge's
    /// unsigned-invoke path uses this as the caller URA field
    /// instead of synthesising one from the subject_id (which for
    /// `easynet:prv:hub:<realm>` would otherwise produce a
    /// nonsensical `agents/easynet:prv:hub:<realm>` literal). Empty
    /// string means "fall back to SDK default", which is correct
    /// for tests and pre-join callers that don't yet know their
    /// own agent URA.
    pub caller_ura_for_hub: String,
}

impl<'a> BridgeAbilityInvoker<'a> {
    pub fn new(bridge: &'a easynet_axon::dendrite_bridge::DendriteBridge) -> Self {
        Self {
            bridge,
            // Generous default for advertise — runtime IPC is local
            // and a 5-second budget covers ordinary cold-start
            // latency without making startup hang.
            timeout_ms: 5_000,
            caller_ura_for_hub: String::new(),
        }
    }

    /// Same as `new` but pins the caller URA to use on hub-shaped
    /// federation calls. Production callers (the daemon boot path)
    /// build this with their own device-profile Agent URA; tests
    /// keep using `new()` so the SDK fallback is exercised.
    pub fn with_caller_ura(
        bridge: &'a easynet_axon::dendrite_bridge::DendriteBridge,
        caller_ura: impl Into<String>,
    ) -> Self {
        Self {
            bridge,
            timeout_ms: 5_000,
            caller_ura_for_hub: caller_ura.into(),
        }
    }
}

impl<'a> AbilityInvoker for BridgeAbilityInvoker<'a> {
    fn invoke_ability(
        &self,
        tenant_id: &str,
        resource_ura: &str,
        payload_json: Value,
    ) -> Result<Value, String> {
        // Build the subject_id from the URA shape so the runtime's
        // `verify_easynet_invocation_metadata` (security.rs:222)
        // sees a subject that matches the URA's parsed
        // `<visibility>:<subject_type>:<subject_value>` decomposition.
        //
        // Two canonical shapes the daemon legitimately calls:
        //   1. `easynet:///r/<realm>/hub` — hub profile. Subject MUST
        //      be `easynet:prv:hub:<realm>`.
        //   2. `easynet:///r/<realm>/ability/hub.<ns>.<verb>` —
        //      hub-owned ability. Subject is still the hub profile,
        //      so we override to the same `easynet:prv:hub:<realm>`.
        //
        // Pre-fix every call passed `subject_id = None`, which the
        // SDK defaulted to the agent form. Hub-owned ability URAs
        // therefore got `agent.<node>` as subject and the runtime
        // rejected with AXON_EASYNET_SUBJECT_MISMATCH
        // ("subject_id does not match resource URA subject"), even
        // though the daemon's bootstrap_self_identity had succeeded
        // and topology had the key.
        let subject_id = subject_id_from_resource_ura(resource_ura);
        // The metadata bag carries TWO load-bearing entries:
        //
        // 1. `easynet.resource_ura` — every call sets this. It lets
        //    axon's `verify_easynet_invocation_metadata` recover the
        //    ability name when the caller didn't fill `target.ability_name`
        //    or `function_name` (the bridge's `ability_call_raw` path
        //    doesn't fill those). Without it axon's pre-ability-name
        //    extraction falls through to "" and the `runtime.*` /
        //    `federation.*` security exemptions never fire — so a
        //    `runtime.register_local_tool` (which is daemon-internal,
        //    same-process admin) gets rejected with
        //    AXON_EASYNET_SUBJECT_MISMATCH because the verifier expects
        //    `agent.<owner>` shaped subjects but the URA is hub-shaped.
        //    Pre-fix this caused 0/N runtime.register_local_tool calls
        //    to succeed at boot — the daemon's local abilities were
        //    advertised to peers but not dispatchable cross-process.
        //
        // 2. `easynet.caller_ura_override` — only set for hub-shaped
        //    subjects when the daemon has a joined caller URA to
        //    pin. Without it the bridge's unsigned-invoke path
        //    synthesises `agents/easynet:prv:hub:<realm>` which the
        //    runtime's caller-URA check rejects.
        let mut map = std::collections::HashMap::new();
        map.insert("easynet.resource_ura".to_string(), resource_ura.to_string());
        if subject_id
            .as_deref()
            .map(|s| s.contains(":hub:"))
            .unwrap_or(false)
            && !self.caller_ura_for_hub.is_empty()
        {
            map.insert(
                "easynet.caller_ura_override".to_string(),
                self.caller_ura_for_hub.clone(),
            );
        }
        let metadata: Option<std::collections::HashMap<String, String>> = Some(map);
        self.bridge
            .ability_call_raw(easynet_axon::dendrite_bridge::AbilityRawCallOptions {
                tenant_id,
                resource_ura,
                payload_json,
                subject_id: subject_id.as_deref(),
                metadata: metadata.as_ref(),
                timeout_ms: self.timeout_ms,
            })
            .map_err(|e| format!("{e}"))
    }
}

/// Derive the canonical `subject_id` an envelope should carry for a
/// given EasyNet resource URA. Returns `None` for URAs the helper
/// doesn't recognise (the SDK falls back to its default
/// `easynet:prv:reg:agent.<owner>` form, which is fine for non-hub
/// callers). Legacy `.../abilities/...` resource shapes are retired
/// and intentionally not recognised here.
///
/// Visible to tests via the module re-export below; the function is
/// pure (no I/O, no state) so a test that pins each shape is
/// sufficient.
pub(crate) fn subject_id_from_resource_ura(resource_ura: &str) -> Option<String> {
    if let Ok(parsed) = crate::core::ura::parse_ura(resource_ura) {
        match parsed.kind {
            crate::core::ura::URAKind::Hub => {
                return Some(format!("easynet:prv:hub:{}", parsed.realm))
            }
            crate::core::ura::URAKind::Ability
                if parsed.ability().is_some_and(|ability| {
                    matches!(ability.owner, crate::core::ura::AbilityOwner::Hub)
                }) =>
            {
                return Some(format!("easynet:prv:hub:{}", parsed.realm));
            }
            _ => {}
        }
    }
    None
}

/// Build the `federation.advertise_agent` payload + invoke it.
/// Returns the typed receipt on success.
pub fn advertise_agent<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    args: &AdvertiseAgentArgs,
) -> AdvertiseOutcome {
    let resource_ura = advertise_agent_resource_ura(realm, tenant_id);
    let payload: Value = match serde_json::from_slice(&args_to_bytes(args)) {
        Ok(v) => v,
        Err(e) => {
            return AdvertiseOutcome {
                agent_ura: args.agent_ura.clone(),
                result: Err(format!("encode advertise_agent args: {e}")),
            };
        }
    };
    match invoker.invoke_ability(tenant_id, &resource_ura, payload) {
        Ok(response) => {
            let body = unwrap_result_json(response);
            match parse_receipt_value::<AdvertiseAgentReceipt>(&body) {
                Ok(parsed) => AdvertiseOutcome {
                    agent_ura: args.agent_ura.clone(),
                    result: Ok(parsed),
                },
                Err(e) => AdvertiseOutcome {
                    agent_ura: args.agent_ura.clone(),
                    result: Err(format!("parse advertise_agent receipt: {e}")),
                },
            }
        }
        Err(e) => AdvertiseOutcome {
            agent_ura: args.agent_ura.clone(),
            result: Err(e),
        },
    }
}

/// Strip the SDK's invoke-response wrapper to expose the receipt
/// body. `bridge.ability_call_raw` returns the full Invoke response
/// envelope (`{result_json, ok?, ...}`) — receipts live in
/// `result_json`. When `result_json` is a string the caller
/// pre-stringified the body, so we re-parse it. Anything that
/// already looks like a top-level receipt (no `result_json` field)
/// passes through verbatim, which keeps tests + future SDK
/// refactors that flatten the shape working without a version
/// branch.
fn unwrap_result_json(response: Value) -> Value {
    let inner = if response.get("result_json").is_some() {
        response.get("result_json").cloned().unwrap_or(Value::Null)
    } else {
        return response;
    };
    match inner {
        Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
        other => other,
    }
}

/// Bulk-publish a list of AbilityDescriptors for one Agent. Mirrors
/// the §3 step 6 sequence: advertise_agent → advertise_abilities.
pub fn advertise_abilities<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_ura: &str,
    host_device_ura: &str,
    abilities: &[AbilityDescriptor],
) -> Result<Value, String> {
    let resource_ura = advertise_abilities_resource_ura(realm, tenant_id);
    let projection = crate::daemon::federation::read_model::owner_projection::prepare_and_persist(
        agent_ura,
        host_device_ura,
        abilities,
    )?;
    let payload = advertise_abilities_payload(agent_ura, &projection)?;
    invoker.invoke_ability(tenant_id, &resource_ura, payload)
}

/// Build the `federation.advertise_abilities` wire payload from an
/// already-persisted owner projection. Single source of the wire shape
/// so the boot-time `advertise_abilities` path and the event-driven
/// hot-add advertiser (see `agent_lifecycle_ability` / the
/// `session.open` advertiser) cannot drift. ISS-002.
pub(crate) fn advertise_abilities_payload(
    agent_ura: &str,
    projection: &crate::daemon::federation::read_model::owner_projection::OwnerProjectionPublication,
) -> Result<Value, String> {
    let args = AdvertiseAbilitiesArgs {
        agent_ura,
        owner_ura: &projection.owner_ura,
        host_device_ura: &projection.host_device_ura,
        projection_revision: projection.projection_revision,
        projection_digest: &projection.projection_digest,
        lease_expires_unix_ms: projection.lease_expires_unix_ms,
        ability_summaries: &projection.ability_summaries,
    };
    serde_json::to_value(&args).map_err(|e| format!("encode advertise_abilities args: {e}"))
}

/// Inbound counterpart to the advertise pair above:
/// `federation.resolve` against the realm's hub. Returns the typed
/// `Vec<ResolvedAgent>` the discover ladder consumes.
///
/// `prefix` filters by `agent_ura_prefix` server-side — pass an empty
/// string for "every agent". `include_abilities` is the load-bearing
/// flag for `<agent>.discover(scope: "easynet")`: without it every
/// peer record is just `{ura, status}` with no descriptors, and the
/// LLM can't pick a candidate.
///
/// Errors are stringified per the AbilityInvoker contract — the
/// caller's recovery is invariant: log + degrade. The discover
/// handler propagates them as a typed `federation_unavailable`
/// envelope so the LLM falls through gracefully.
pub fn resolve_agents<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    prefix: &str,
    include_abilities: bool,
) -> Result<Vec<ResolvedAgent>, String> {
    resolve_agents_with_filter(invoker, tenant_id, realm, prefix, include_abilities, None)
}

/// RFC-002 §5 variant: pass an explicit tenant_filter so callers can
/// request "scope to caller's tenant" (None / "") or "cross-tenant
/// catalog listing" ("*"). The hub-side default is auto-fill from
/// envelope tenant; CLI's `<agent>.discover(scope: "user")` therefore
/// passes None and the hub fills in the right value.
pub fn resolve_agents_with_filter<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    prefix: &str,
    include_abilities: bool,
    tenant_filter: Option<String>,
) -> Result<Vec<ResolvedAgent>, String> {
    let resource_ura = resolve_resource_ura(realm, tenant_id);
    let filter = if prefix.is_empty() && !include_abilities && tenant_filter.is_none() {
        None
    } else {
        Some(ResolveFilter {
            agent_ura_prefix: if prefix.is_empty() {
                None
            } else {
                Some(prefix.to_string())
            },
            include_abilities,
            tenant_filter,
        })
    };
    let args = ResolveArgs { filter };
    let payload: Value = serde_json::from_slice(&args_to_bytes(&args))
        .map_err(|e| format!("encode resolve args: {e}"))?;
    let response = invoker.invoke_ability(tenant_id, &resource_ura, payload)?;
    // Same `result_json` unwrap as advertise_agent — the SDK's
    // raw invoke surface wraps every receipt in an Invoke response
    // envelope, and the federation receipt parsers want the inner
    // body verbatim. See `unwrap_result_json` doc comment.
    let receipt_body = unwrap_result_json(response);
    let receipt: ResolveReceipt = parse_receipt_value(&receipt_body)
        .map_err(|e| format!("parse federation.resolve receipt: {e}"))?;
    Ok(receipt.agents)
}

/// `federation.revoke` invocation. Removes the named Agent from the
/// realm directory. The CLI shutdown path calls this on the
/// daemon's own device-profile URA to clean up its directory
/// presence — replaces the deprecated `bridge.deregister_node`
/// gRPC call.
///
/// Best-effort by contract: a hub-side rejection (rare — the only
/// nonzero failure mode in v1 is "agent already revoked, no-op")
/// surfaces as an Err string and the caller logs + continues.
pub fn revoke_agent<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_ura: &str,
    reason: &str,
) -> Result<(), String> {
    let resource_ura = revoke_resource_ura(realm, tenant_id);
    let payload = serde_json::json!({
        "agent_ura": agent_ura,
        "reason": reason,
    });
    let _ = invoker.invoke_ability(tenant_id, &resource_ura, payload)?;
    Ok(())
}

/// `federation.heartbeat` invocation. Refreshes the daemon's
/// `last_heartbeat_unix_ms` in the directory and carries the
/// RFC-005 owner-projection lease refresh batch for projections
/// this daemon has already advertised. Replaces the deprecated
/// `bridge.NodeHeartbeat` keepalive RPC.
///
/// Best-effort: a transient hub-side failure surfaces as Err and
/// the caller's loop logs + retries on the next tick. Persistent
/// failure escalates to membership eviction same as before.
pub fn heartbeat<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    store: &crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore,
) -> Result<crate::daemon::federation::client::ability_contract::HeartbeatReceipt, String> {
    let resource_ura = heartbeat_resource_ura(realm, tenant_id);
    // AXON-RFC-001 v4.1.7 hub-broadcast contract: pass the
    // device's last-seen hub-abilities revision so the hub can
    // answer with an incremental diff. The store starts at
    // revision 0 (empty cache); the hub treats `since=0` as
    // "fully out of date" and replies with the full snapshot in
    // the diff's `added` field. v4.1.6 hubs ignore the field and
    // omit `hub_abilities_diff` from the receipt; the parse path
    // below treats absent diff as "no change".
    let since = store.revision();
    let refresh_owner_uras =
        crate::daemon::federation::read_model::owner_projection::heartbeat_refresh_owner_uras()?;
    let payload = serde_json::json!({
        "since_abilities_revision": since,
        "refresh_owner_uras": refresh_owner_uras,
    });
    let response = invoker.invoke_ability(tenant_id, &resource_ura, payload)?;
    // Best-effort diff application: a malformed body or a hub
    // that doesn't speak the contract leaves the store unchanged
    // (same as before this PR landed).
    let body = unwrap_result_json(response);
    let receipt = serde_json::from_value::<
        crate::daemon::federation::client::ability_contract::HeartbeatReceipt,
    >(body)
    .unwrap_or_default();
    let diff = receipt.hub_abilities_diff.clone();
    let added_n = diff.added.len();
    let removed_n = diff.removed.len();
    if added_n != 0 || removed_n != 0 {
        store.apply_diff(diff);
        eprintln!(
            "[heartbeat] hub-broadcast diff: +{added_n} -{removed_n} (rev now {})",
            store.revision()
        );
    }
    Ok(receipt)
}

/// RFC-002 §5.1 federation.resolve_key client. Returns the public
/// key the directory has on file for the given URA.
pub fn resolve_key<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_ura: &str,
) -> Result<crate::daemon::federation::client::ability_contract::ResolveKeyReceipt, String> {
    let resource_ura = hub_ability_resource_ura(realm, FED_RESOLVE_KEY_ABILITY_NAME);
    let payload = serde_json::json!({ "agent_ura": agent_ura });
    let response = invoker.invoke_ability(tenant_id, &resource_ura, payload)?;
    let receipt_body = unwrap_result_json(response);
    serde_json::from_value(receipt_body).map_err(|e| format!("parse resolve_key receipt: {e}"))
}

/// Convenience: advertise the device-profile Agent itself (Selfsigned
/// Model A per §1.3) as a single helper since it's the very first
/// call any daemon makes after federation.join.
pub fn advertise_self_signed_device<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_ura: &str,
    public_key_hex: &str,
) -> AdvertiseOutcome {
    advertise_self_signed_device_with_host_node(
        invoker,
        tenant_id,
        realm,
        agent_ura,
        public_key_hex,
        None,
    )
}

/// Variant that records the runtime node hosting this agent.
pub fn advertise_self_signed_device_with_host_node<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_ura: &str,
    public_key_hex: &str,
    host_node_id: Option<String>,
) -> AdvertiseOutcome {
    let args = AdvertiseAgentArgs {
        agent_ura: agent_ura.to_string(),
        public_key_hex: public_key_hex.to_string(),
        signing_authority: AdvertisedSigningAuthority::SelfSigned,
        host_node_id,
    };
    advertise_agent(invoker, tenant_id, realm, &args)
}

/// Convenience: advertise a hosted Agent (Model B) under the host
/// device-profile. Used for consent/policy/mcp/llm-profile Agents.
pub fn advertise_hosted_agent<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_ura: &str,
    host_ura: &str,
) -> AdvertiseOutcome {
    advertise_hosted_agent_with_host_node(invoker, tenant_id, realm, agent_ura, host_ura, None)
}

/// Variant that records the runtime node hosting this agent.
pub fn advertise_hosted_agent_with_host_node<I: AbilityInvoker>(
    invoker: &I,
    tenant_id: &str,
    realm: &str,
    agent_ura: &str,
    host_ura: &str,
    host_node_id: Option<String>,
) -> AdvertiseOutcome {
    let args = AdvertiseAgentArgs {
        agent_ura: agent_ura.to_string(),
        public_key_hex: String::new(),
        signing_authority: AdvertisedSigningAuthority::HostedBy {
            host_ura: host_ura.to_string(),
        },
        host_node_id,
    };
    advertise_agent(invoker, tenant_id, realm, &args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::descriptors::{AbilityDescriptor, Visibility};
    use std::cell::RefCell;

    /// In-memory fake invoker that records every call and returns a
    /// canned reply. Lets us assert the resource URA shape, the
    /// payload contents, and the receipt-parsing path without a
    /// running axon-runtime.
    struct RecordingInvoker {
        last_resource_ura: RefCell<Option<String>>,
        last_payload: RefCell<Option<Value>>,
        reply: Value,
    }

    impl RecordingInvoker {
        fn new(reply: Value) -> Self {
            Self {
                last_resource_ura: RefCell::new(None),
                last_payload: RefCell::new(None),
                reply,
            }
        }
    }

    impl AbilityInvoker for RecordingInvoker {
        fn invoke_ability(
            &self,
            _tenant_id: &str,
            resource_ura: &str,
            payload_json: Value,
        ) -> Result<Value, String> {
            *self.last_resource_ura.borrow_mut() = Some(resource_ura.to_string());
            *self.last_payload.borrow_mut() = Some(payload_json);
            Ok(self.reply.clone())
        }
    }

    struct AlwaysFails;
    impl AbilityInvoker for AlwaysFails {
        fn invoke_ability(
            &self,
            _tenant_id: &str,
            _resource_ura: &str,
            _payload_json: Value,
        ) -> Result<Value, String> {
            Err("transport timeout".into())
        }
    }

    #[test]
    fn resource_ura_substitutes_realm_correctly() {
        // Public helpers now return canonical hub-owned ability URAs.
        assert_eq!(
            advertise_agent_resource_ura("acme", "tenant-1"),
            "easynet:///r/acme/ability/hub.federation.advertise_agent"
        );
        assert_eq!(
            advertise_abilities_resource_ura("contoso", "tenant-2"),
            "easynet:///r/contoso/ability/hub.federation.advertise_abilities"
        );
    }

    #[test]
    fn subject_id_parser_understands_canonical_hub_ability_ura() {
        assert_eq!(
            subject_id_from_resource_ura(
                "easynet:///r/acme/ability/hub.federation.advertise_agent"
            )
            .as_deref(),
            Some("easynet:prv:hub:acme")
        );
    }

    #[test]
    fn bridge_invoker_keeps_canonical_hub_owned_ability_ura() {
        assert_eq!(
            hub_ability_resource_ura("acme", "runtime.register_local_tool"),
            "easynet:///r/acme/ability/hub.runtime.register_local_tool"
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
            "easynet:///r/acme/device/01DEV",
            "deadbeef",
        );
        let receipt = outcome.result.expect("must succeed");
        assert!(receipt.ack);
        assert!(!receipt.replaced_prior);

        let payload = invoker.last_payload.borrow().clone().unwrap();
        assert_eq!(payload["agent_ura"], "easynet:///r/acme/device/01DEV");
        assert_eq!(payload["public_key_hex"], "deadbeef");
        assert_eq!(payload["signing_authority"]["kind"], "self_signed");
    }

    #[test]
    fn advertise_hosted_agent_emits_hosted_by_with_host_ura() {
        let invoker = RecordingInvoker::new(serde_json::json!({
            "ack": true,
            "replaced_prior": true
        }));
        let outcome = advertise_hosted_agent(
            &invoker,
            "tenant",
            "acme",
            "easynet:///r/acme/agent/u1.01LLM",
            "easynet:///r/acme/device/01DEV",
        );
        let receipt = outcome.result.expect("must succeed");
        assert!(receipt.replaced_prior);

        let payload = invoker.last_payload.borrow().clone().unwrap();
        assert_eq!(payload["signing_authority"]["kind"], "hosted_by");
        assert_eq!(
            payload["signing_authority"]["host_ura"],
            "easynet:///r/acme/device/01DEV"
        );
        assert_eq!(payload["public_key_hex"], "");
    }

    #[test]
    fn advertise_returns_err_outcome_when_invoker_fails() {
        let outcome = advertise_self_signed_device(
            &AlwaysFails,
            "tenant",
            "acme",
            "easynet:///r/acme/device/01DEV",
            "00",
        );
        let err = outcome.result.expect_err("must surface invoker error");
        assert!(err.contains("transport timeout"));
        assert_eq!(outcome.agent_ura, "easynet:///r/acme/device/01DEV");
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
            "easynet:///r/acme/device/01DEV",
            "00",
        );
        let err = outcome.result.expect_err("malformed receipt must surface");
        assert!(err.contains("parse advertise_agent receipt"));
    }

    #[test]
    fn advertise_abilities_serializes_owner_projection_without_raw_descriptors() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let invoker = RecordingInvoker::new(serde_json::json!({"ack": true}));
        let descriptors = vec![
            AbilityDescriptor::new(
                "fs.read",
                "easynet:///r/acme/device/01DEV",
                Visibility::Public,
            )
            .unwrap()
            .with_source("kernel:built-in"),
            AbilityDescriptor::new(
                "skill.list",
                "easynet:///r/acme/device/01DEV",
                Visibility::Scoped,
            )
            .unwrap(),
        ];
        let _ = advertise_abilities(
            &invoker,
            "tenant",
            "acme",
            "easynet:///r/acme/device/01DEV",
            "easynet:///r/acme/device/01DEV",
            &descriptors,
        )
        .expect("must succeed");

        let payload = invoker.last_payload.borrow().clone().unwrap();
        assert_eq!(payload["agent_ura"], "easynet:///r/acme/device/01DEV");
        assert_eq!(payload["owner_ura"], "easynet:///r/acme/device/01DEV");
        assert_eq!(payload["host_device_ura"], "easynet:///r/acme/device/01DEV");
        assert_eq!(payload["projection_revision"], 1);
        assert!(payload["projection_digest"].as_str().unwrap().len() >= 64);
        // C4: lease cancelled (ISS-002) — projection publishes lease=0.
        assert_eq!(payload["lease_expires_unix_ms"].as_i64().unwrap(), 0);
        assert!(
            payload.get("abilities").is_none(),
            "owner projection publication must not send raw AbilityDescriptor payloads"
        );

        let summaries = payload["ability_summaries"]
            .as_array()
            .expect("ability_summaries must be array");
        assert_eq!(summaries.len(), 2);
        assert_eq!(
            summaries[0]["ability_ura"],
            "easynet:///r/acme/ability/device.01DEV.fs.read"
        );
        assert_eq!(summaries[0]["namespace"], "fs");
        assert_eq!(summaries[0]["local_name"], "read");
        assert_eq!(summaries[0]["policy_ref"], "visibility:PUBLIC");
    }

    #[test]
    fn advertise_abilities_targets_correct_resource_ura() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        // Fake invokers observe the canonical business-layer URA.
        let invoker = RecordingInvoker::new(serde_json::json!({"ack": true}));
        let _ = advertise_abilities(
            &invoker,
            "tenant",
            "acme",
            "easynet:///r/acme/device/01DEV",
            "easynet:///r/acme/device/01DEV",
            &[],
        )
        .unwrap();
        assert_eq!(
            invoker.last_resource_ura.borrow().as_deref().unwrap(),
            "easynet:///r/acme/ability/hub.federation.advertise_abilities"
        );
    }

    #[test]
    fn heartbeat_includes_owner_projection_refresh_batch() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut file =
            crate::daemon::persistence::owner_projections::OwnerProjectionCursorFile::default();
        file.upsert(owner_projection_cursor(
            "easynet:///r/acme/agent/u1.01AGENT",
            "easynet:///r/acme/device/01DEV",
        ));
        file.upsert(owner_projection_cursor(
            "easynet:///r/acme/device/01DEV",
            "easynet:///r/acme/device/01DEV",
        ));
        crate::daemon::persistence::owner_projections::save(&file).expect("save projection cursor");

        let invoker = RecordingInvoker::new(serde_json::json!({
            "membership_status": "active",
            "realm_directory_size": 1
        }));
        let store = crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore::new();
        let _receipt = heartbeat(&invoker, "tenant", "acme", &store).expect("heartbeat succeeds");

        let payload = invoker.last_payload.borrow().clone().unwrap();
        assert_eq!(payload["since_abilities_revision"].as_u64(), Some(0));
        assert_eq!(
            payload["refresh_owner_uras"],
            serde_json::json!([
                "easynet:///r/acme/agent/u1.01AGENT",
                "easynet:///r/acme/device/01DEV"
            ])
        );
        assert_eq!(
            invoker.last_resource_ura.borrow().as_deref().unwrap(),
            "easynet:///r/acme/ability/hub.federation.heartbeat"
        );
    }

    fn owner_projection_cursor(
        owner_ura: &str,
        host_device_ura: &str,
    ) -> crate::daemon::persistence::owner_projections::OwnerProjectionCursor {
        crate::daemon::persistence::owner_projections::OwnerProjectionCursor {
            owner_ura: owner_ura.into(),
            host_device_ura: host_device_ura.into(),
            projection_revision: 1,
            projection_digest: format!("digest-{owner_ura}"),
            content_fingerprint: format!("fingerprint-{owner_ura}"),
            lease_expires_unix_ms: 61_000,
            updated_at: "1970-01-01T00:00:01.000Z".into(),
        }
    }
}
