// EasyNet CLI — observe.network_health ability handler
// =======================================================
//
// File: src/runtime/agents/network_health_ability.rs
//
// Per §18: input `{}`, body `{links[], latency, ...}`. Sibling of
// observe.health (which is a unary smoke probe — does the ability
// pipeline answer at all). network_health surfaces the *network*
// posture: who do we believe we're joined to, what hosted Agents are
// alive locally, is dendrite reachable.
//
// What v1 returns
// ---------------
//   {
//     "joined":              bool,         // credentials present
//     "host_device_uri":     string|null,  // canonical URA after join
//     "hosted_agent_count":  number,
//     "links": [                            // §18 contract field
//       {"target": "<hub-or-realm hint>", "status": "...", ...}
//     ],
//     "latency_ms":          null,         // §18 contract field; not yet probed
//     "schema":              "v1"
//   }
//
// What v1 does NOT do
// -------------------
// Probe live link latency, count active gRPC streams, or assert
// dendrite reachability. The daemon doesn't currently retain a
// handle the agents/ layer can read those off of, and the work to
// thread one through is its own milestone (probably bundled with the
// future `admin.status` lift in C-M13-B1). For now the `links` array
// is a single entry derived from the membership state — accurate and
// useful, just thin.
//
// Why ship a thin v1 vs. defer
// ----------------------------
// §18 names this ability; deferring it leaves the row blank in the
// description_for / input_schema_for tables and slows the audit. The
// contract is "structured response, not rich response." A caller
// that needs richer data can still call observe.health for liveness
// + meta.describe for ability-side state; this row exists so the
// network-side question has *some* answer.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::LocalAbilityRegistry;

pub const ABILITY_NETWORK_HEALTH: &str = "observe.network_health";

pub fn register(reg: &mut LocalAbilityRegistry) {
    reg.register_rpc(ABILITY_NETWORK_HEALTH, Arc::new(|_args: Value| handler()));
}

fn handler() -> anyhow::Result<Value> {
    let local = crate::persistence::local_agents::load().unwrap_or_default();
    let joined = !local.host_device_agent_uri.is_empty();
    let host_uri: Value = if joined {
        Value::String(local.host_device_agent_uri.clone())
    } else {
        Value::Null
    };

    // Single membership-derived link entry. status="joined" reads
    // straight off credentials presence; "unjoined" is the pre-pair
    // state. A future enrichment pass adds gRPC liveness, RTT, and
    // per-peer entries — the array shape is already the right one
    // for that growth.
    let link_status = if joined { "joined" } else { "unjoined" };
    let links = json!([
        {
            "target": "realm-hub",
            "status": link_status,
        }
    ]);

    Ok(json!({
        "joined": joined,
        "host_device_uri": host_uri,
        "hosted_agent_count": local.hosted_agents.len(),
        "links": links,
        "latency_ms": Value::Null,
        "schema": "v1",
    }))
}

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

pub fn description() -> &'static str {
    "Report the daemon's network posture (membership status, hosted \
     Agent count, link summary). v1 returns membership-derived state; \
     live link probing lands with the broader admin.status work."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_makes_ability_dispatchable() {
        let mut reg = LocalAbilityRegistry::new();
        register(&mut reg);
        assert!(reg.get_rpc(ABILITY_NETWORK_HEALTH).is_some());
    }

    #[test]
    fn handler_returns_structurally_complete_response() {
        // Whatever the membership state on the test host happens to
        // be, every §18-contract field MUST be present. A regression
        // that elided a field would let a downstream caller hit a
        // null-ref on what should be a typed-null.
        let resp = handler().unwrap();
        for field in ["joined", "host_device_uri", "hosted_agent_count",
                      "links", "latency_ms", "schema"] {
            assert!(
                resp.get(field).is_some(),
                "response missing required field {field}"
            );
        }
        assert_eq!(resp["schema"], "v1");
        assert!(resp["links"].is_array(), "links must be an array even when empty");
        assert!(!resp["links"].as_array().unwrap().is_empty(),
                "v1 always emits at least the realm-hub link entry");
    }

    #[test]
    fn input_schema_is_empty_object() {
        let s = input_schema();
        assert_eq!(s["type"], "object");
        assert!(s["properties"].as_object().unwrap().is_empty());
        assert_eq!(s["additionalProperties"], false);
    }
}
