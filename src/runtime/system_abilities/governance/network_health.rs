// EasyNet CLI — observe.network_health ability handler
// =======================================================
//
// File: src/runtime/system_abilities/governance/network_health.rs
//
// Per §18: input `{}`, body `{links[], latency, ...}`. Sibling of
// observe.health (which is a unary smoke probe — does the ability
// pipeline answer at all). network_health surfaces the *network*
// posture: who do we believe we're joined to, what hosted Agents are
// alive locally, is dendrite reachable.
//
// What v2 returns
// ---------------
//   {
//     "joined":              bool,
//     "host_device_ura":     string|null,
//     "hosted_agent_count":  number,
//     "peer_count":          number,
//     "links": [
//       {"target": "local-daemon", "status": "reachable", ...},
//       {"target": "realm-hub", "status": "reachable" | "unreachable", ...},
//       {"target": "<peer-agent-ura>", "status": "reachable" | "probe_failed", ...}
//     ],
//     "latency_ms":          number|null,  // federation.resolve latency
//     "schema":              "v2",
//     "view":                "live"
//   }
//
// Probe scope
// -----------
// The point of this ability is operator truth, not a complete graph
// crawler. One call performs:
//
//   * local daemon liveness (the handler itself),
//   * one federation.resolve,
//   * at most one direct observe.health probe per discovered
//     device-profile Agent.
//
// That is enough to answer the CLI-facing question "is this daemon
// alive, is the realm reachable, and which peers are directly
// callable from here?" without building a second monitoring stack.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::AxonAbilityCatalog;
use crate::runtime::ability_dispatch::OwnerKind;
use crate::runtime::system_abilities::integrations::federation_probe;

pub const ABILITY_NETWORK_HEALTH: &str =
    crate::runtime::ability_names::governance::OBSERVE_NETWORK_HEALTH;

pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner(
        ABILITY_NETWORK_HEALTH,
        OwnerKind::Device,
        Arc::new(|_args: Value| handler()),
    );
}

fn handler() -> anyhow::Result<Value> {
    let local = crate::persistence::local_agents::load().unwrap_or_default();
    let view = federation_probe::collect_device_view();
    let self_node = view.nodes.iter().find(|n| n.is_self);
    let joined =
        self_node.map(|n| n.paired).unwrap_or(false) || !local.host_device_agent_ura.is_empty();
    let host_ura: Value = if !local.host_device_agent_ura.is_empty() {
        Value::String(local.host_device_agent_ura.clone())
    } else if let Some(ura) = self_node.and_then(|n| n.agent_ura.clone()) {
        Value::String(ura)
    } else {
        Value::Null
    };
    let mut links = Vec::new();
    if let Some(node) = self_node {
        links.push(json!({
            "target": "local-daemon",
            "node_id": node.node_id.clone(),
            "status": "reachable",
            "state": node.state.clone(),
        }));
    }
    links.push(json!({
        "target": "realm-hub",
        "status": if !joined {
            "unjoined"
        } else if view.federation_view == "federated" {
            "reachable"
        } else {
            "unreachable"
        },
        "latency_ms": view.resolve_latency_ms,
        "detail": view.federation_view_reason.clone(),
    }));
    for node in view.nodes.iter().filter(|n| !n.is_self) {
        links.push(json!({
            "target": node.agent_ura.clone(),
            "node_id": node.node_id.clone(),
            "status": node.probe_status.clone(),
            "state": node.state.clone(),
            "online": node.online,
            "latency_ms": node.latency_ms,
            "error": node.probe_error.clone(),
        }));
    }
    let peer_count = view.nodes.iter().filter(|n| !n.is_self).count();
    let resolve_latency_ms = view.resolve_latency_ms;
    let federation_view = view.federation_view;
    let federation_view_reason = view.federation_view_reason;

    Ok(json!({
        "joined": joined,
        "host_device_ura": host_ura,
        "hosted_agent_count": local.hosted_agents.len(),
        "peer_count": peer_count,
        "links": Value::Array(links),
        "latency_ms": resolve_latency_ms,
        "schema": "v2",
        "view": "live",
        "federation_view": federation_view,
        "federation_view_reason": federation_view_reason,
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
    "Report the daemon's live network posture: local daemon reachability, \
     realm-directory reachability, and direct observe.health probe status \
     for each discovered peer device."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_makes_ability_dispatchable() {
        let mut reg = AxonAbilityCatalog::new();
        register(&mut reg);
        assert!(reg.get_rpc(ABILITY_NETWORK_HEALTH).is_some());
    }

    #[test]
    fn handler_returns_structurally_complete_response() {
        // Whatever the local membership state happens to be, the
        // operator-facing response must stay structurally complete.
        let resp = handler().unwrap();
        for field in [
            "joined",
            "host_device_ura",
            "hosted_agent_count",
            "links",
            "latency_ms",
            "schema",
            "view",
            "federation_view",
        ] {
            assert!(
                resp.get(field).is_some(),
                "response missing required field {field}"
            );
        }
        assert_eq!(resp["schema"], "v2");
        assert_eq!(resp["view"], "live");
        assert!(
            resp["links"].is_array(),
            "links must be an array even when empty"
        );
        assert!(
            !resp["links"].as_array().unwrap().is_empty(),
            "network health must emit at least local-daemon + realm-hub links"
        );
    }

    #[test]
    fn input_schema_is_empty_object() {
        let s = input_schema();
        assert_eq!(s["type"], "object");
        assert!(s["properties"].as_object().unwrap().is_empty());
        assert_eq!(s["additionalProperties"], false);
    }
}
