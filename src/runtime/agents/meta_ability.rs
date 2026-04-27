// EasyNet CLI — meta.{describe, list_abilities} ability handlers
// =================================================================
//
// File: src/runtime/agents/meta_ability.rs
//
// Per-Agent self-introspection. Per RFC §18, both abilities are
// PUBLIC (callable by anyone), with `meta.list_abilities`'s result
// row-filtered against AbilityDescriptor.visibility — that filter
// belongs at the admission/dispatch layer, not in the handler, so
// the handler returns the full local catalog and lets the gate
// trim per visibility rule (§1.6).
//
// What lives here
// ---------------
//   * meta.describe — `{ uri, identity_summary, abilities_summary,
//                         metadata }` for the host device-profile.
//                     identity_summary surfaces the canonical URA
//                     and signing-authority hint; abilities_summary
//                     is the count + namespace breakdown so a caller
//                     can decide whether to follow up with a full
//                     meta.list_abilities.
//   * meta.list_abilities — `{ abilities: AbilityDescriptor[] }`.
//                           The same descriptor catalog mcp.bridge.
//                           list_tools projects to MCP, but in the
//                           native ontology shape (no MCP wrapper).
//                           This is the canonical Invoke surface for
//                           ability discovery; the MCP ability is
//                           the edge-protocol projection of the same
//                           data.
//
// Why two abilities, not one
// --------------------------
// describe is cheap (a constant-shape summary blob) and is what a
// federation peer hits to confirm "is this the device I think it
// is?" — pulling the full ability list for that question would burn
// bandwidth on every cache check. list_abilities is the catalog
// fetch. Splitting them lets a caller pay only for what it needs,
// the same way the MCP spec splits resource description from full
// resource fetch.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_descriptor::AbilityDescriptor;
use crate::runtime::ability_dispatch::LocalAbilityRegistry;

pub const ABILITY_DESCRIBE: &str = "meta.describe";
pub const ABILITY_LIST_ABILITIES: &str = "meta.list_abilities";

/// Register both meta abilities on the registry.
///
/// `descriptors_provider` runs at handler-call time so future
/// hot-reload of the descriptor catalog is reflected without
/// re-registration. Same closure type as `mcp_bridge_ability::register`
/// so the daemon wires both off `profiles::load_host_descriptors`.
pub fn register<F>(reg: &mut LocalAbilityRegistry, descriptors_provider: F)
where
    F: Fn() -> Vec<AbilityDescriptor> + Send + Sync + 'static,
{
    let provider: Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync> =
        Arc::new(descriptors_provider);
    let p_for_describe = Arc::clone(&provider);
    reg.register_rpc(
        ABILITY_DESCRIBE,
        Arc::new(move |_args: Value| describe_handler(&p_for_describe)),
    );
    reg.register_rpc(
        ABILITY_LIST_ABILITIES,
        Arc::new(move |_args: Value| list_abilities_handler(&provider)),
    );
}

fn describe_handler(
    descriptors_provider: &Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync>,
) -> anyhow::Result<Value> {
    let descriptors = descriptors_provider();

    // Identity comes from local-agents.json. Pre-join state surfaces
    // as uri:"self" so a caller still sees a well-formed describe
    // response — they can re-poll after the daemon completes join.
    let local = crate::persistence::local_agents::load().unwrap_or_default();
    let host_uri = if local.host_device_agent_uri.is_empty() {
        "self".to_string()
    } else {
        local.host_device_agent_uri.clone()
    };
    let signing_authority = if local.host_device_agent_uri.is_empty() {
        "unprovisioned" // pre-join: no key bound yet
    } else {
        "self" // device-profile is Model A (own keypair)
    };

    // abilities_summary = count + per-namespace count. The breakdown
    // is what makes the response useful to a caller deciding whether
    // to fetch the full catalogue: "12 abilities, 4 in fleet.* and
    // 3 in consent.*" tells you what the device actually does.
    let mut by_namespace: BTreeMap<String, usize> = BTreeMap::new();
    for d in &descriptors {
        let ns = d
            .name
            .split_once('.')
            .map(|(ns, _)| ns.to_string())
            .unwrap_or_else(|| "(no-namespace)".to_string());
        *by_namespace.entry(ns).or_insert(0) += 1;
    }

    Ok(json!({
        "uri": host_uri,
        "identity_summary": {
            "signing_authority": signing_authority,
        },
        "abilities_summary": {
            "total": descriptors.len(),
            "by_namespace": by_namespace,
        },
        "metadata": {
            "hosted_agent_count": local.hosted_agents.len(),
        },
    }))
}

fn list_abilities_handler(
    descriptors_provider: &Arc<dyn Fn() -> Vec<AbilityDescriptor> + Send + Sync>,
) -> anyhow::Result<Value> {
    let descriptors = descriptors_provider();
    // AbilityDescriptor is Serialize so we hand the catalog through
    // verbatim. Visibility filtering per §1.6 happens at the
    // admission/dispatch layer (the handler doesn't know who the
    // caller is); the full local catalog is what the gate filters.
    Ok(json!({ "abilities": descriptors }))
}

// ── Discovery surfaces ────────────────────────────────────────

pub fn describe_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

pub fn describe_description() -> &'static str {
    "Return this Agent's identity + ability summary. Lightweight \
     companion to meta.list_abilities — answers \"who are you and \
     roughly what do you do\" in one call so a peer doesn't have \
     to fetch the full descriptor catalogue for a cache check."
}

pub fn list_abilities_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

pub fn list_abilities_description() -> &'static str {
    "Return the full local AbilityDescriptor catalogue. Canonical \
     Invoke surface for ability discovery; the MCP-shaped projection \
     lives at mcp.bridge.list_tools for external MCP clients."
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};

    fn d(name: &str) -> AbilityDescriptor {
        AbilityDescriptor::new(name, "easynet:///r/test/agent/01DEV", Visibility::Public)
            .expect("test descriptor")
    }

    #[test]
    fn registration_makes_both_abilities_dispatchable() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, Vec::new);
        assert!(reg.get_rpc(ABILITY_DESCRIBE).is_some());
        assert!(reg.get_rpc(ABILITY_LIST_ABILITIES).is_some());
    }

    #[test]
    fn list_abilities_returns_descriptors_verbatim() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, || {
            vec![d("observe.health"), d("fleet.list_agents")]
        });
        let handler = reg.get_rpc(ABILITY_LIST_ABILITIES).unwrap();
        let resp = handler(json!({})).unwrap();
        let abilities = resp["abilities"].as_array().unwrap();
        assert_eq!(abilities.len(), 2);
        // Round-trips through serde — full descriptor shape preserved.
        assert_eq!(abilities[0]["name"], "observe.health");
        assert_eq!(abilities[0]["owner_agent_uri"], "easynet:///r/test/agent/01DEV");
    }

    #[test]
    fn describe_buckets_abilities_by_namespace() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, || {
            vec![
                d("observe.health"),
                d("fleet.list_agents"),
                d("fleet.list_sessions"),
                d("consent.subscribe"),
            ]
        });
        let handler = reg.get_rpc(ABILITY_DESCRIBE).unwrap();
        let resp = handler(json!({})).unwrap();
        assert_eq!(resp["abilities_summary"]["total"], 4);
        let by_ns = resp["abilities_summary"]["by_namespace"].as_object().unwrap();
        assert_eq!(by_ns["fleet"], 2);
        assert_eq!(by_ns["observe"], 1);
        assert_eq!(by_ns["consent"], 1);
    }

    #[test]
    fn describe_handles_empty_catalog() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg, Vec::new);
        let handler = reg.get_rpc(ABILITY_DESCRIBE).unwrap();
        let resp = handler(json!({})).unwrap();
        assert_eq!(resp["abilities_summary"]["total"], 0);
        // Empty by_namespace must be an object, not absent — caller
        // shouldn't have to special-case missing key.
        assert!(resp["abilities_summary"]["by_namespace"]
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn input_schemas_are_empty_objects() {
        for s in [describe_input_schema(), list_abilities_input_schema()] {
            assert_eq!(s["type"], "object");
            assert!(s["properties"].as_object().unwrap().is_empty());
            assert_eq!(s["additionalProperties"], false);
        }
    }
}
