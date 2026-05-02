// EasyNet CLI — federation-backed device probe helpers
// ====================================================
//
// File: src/runtime/agents/federation_probe.rs
// Description: Shared device-discovery + direct-probe helpers used by
//              fleet.list_nodes and observe.network_health.
//
// Why this module exists
// ----------------------
// The current bug is not "no heartbeat packet exists"; it is that the
// operator-facing surfaces were reading local files and hard-coding
// HEALTHY/local_only instead of consulting the live federation path.
// This module centralises the two live steps we do have today:
//
//   1. `federation.resolve` against the realm directory.
//   2. `federation.forward_invoke(target_uri, "observe.health")`
//      for a direct reachability probe to each device-profile Agent.
//
// Keeping the logic here gives one bounded, testable definition of
// "what devices are visible" and "what counts as reachable" instead of
// four call sites drifting independently.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::time::Instant;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde_json::{json, Value};

use crate::persistence::config;
use crate::runtime::advertise::{self, BridgeAbilityInvoker};
use crate::runtime::federation_client::{ForwardInvokeReceipt, ResolvedAgent};

const DEVICE_HEALTH_ABILITY: &str = "observe.health";
const DEVICE_FLEET_ABILITY: &str = "fleet.list_nodes";
const DEVICE_NETWORK_HEALTH_ABILITY: &str = "observe.network_health";
const MAX_DEVICE_PROBES: usize = 64;

#[derive(Debug, Clone)]
pub(crate) struct LocalIdentity {
    pub node_id: String,
    pub tenant_id: String,
    pub hub_endpoint: Option<String>,
    pub paired: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FleetNodeSnapshot {
    pub node_id: String,
    pub tenant_id: String,
    pub agent_uri: Option<String>,
    pub is_self: bool,
    pub paired: bool,
    pub hub_endpoint: Option<String>,
    pub state: String,
    pub online: bool,
    pub probe_status: String,
    pub probe_error: Option<String>,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct FleetView {
    pub nodes: Vec<FleetNodeSnapshot>,
    pub federation_view: String,
    pub federation_view_reason: Option<String>,
    pub resolve_latency_ms: Option<u64>,
}

#[derive(Debug)]
struct ProbeOutcome {
    online: bool,
    state: &'static str,
    probe_status: &'static str,
    probe_error: Option<String>,
    latency_ms: Option<u64>,
}

pub(crate) fn local_identity() -> LocalIdentity {
    match config::load_credentials() {
        Ok(c) => LocalIdentity {
            node_id: c.node_id,
            tenant_id: c.tenant_id,
            hub_endpoint: if c.hub_endpoint.trim().is_empty() {
                None
            } else {
                Some(c.hub_endpoint)
            },
            paired: true,
        },
        Err(_) => LocalIdentity {
            node_id: "local".to_string(),
            tenant_id: "default".to_string(),
            hub_endpoint: None,
            paired: false,
        },
    }
}

pub(crate) fn collect_fleet_view() -> FleetView {
    let local = local_identity();
    let mut nodes = vec![FleetNodeSnapshot {
        node_id: local.node_id.clone(),
        tenant_id: local.tenant_id.clone(),
        agent_uri: if local.paired {
            Some(format!(
                "easynet:///r/{}/agent/{}",
                local.tenant_id, local.node_id
            ))
        } else {
            None
        },
        is_self: true,
        paired: local.paired,
        hub_endpoint: local.hub_endpoint.clone(),
        state: if local.paired {
            "HEALTHY".to_string()
        } else {
            "STANDALONE".to_string()
        },
        online: true,
        probe_status: "local".to_string(),
        probe_error: None,
        latency_ms: None,
    }];
    if !local.paired {
        return FleetView {
            nodes,
            federation_view: "local_only".to_string(),
            federation_view_reason: Some(
                "device is not paired with a realm; only the local daemon can be described"
                    .to_string(),
            ),
            resolve_latency_ms: None,
        };
    }

    let creds = match config::load_credentials() {
        Ok(c) => c,
        Err(e) => {
            return FleetView {
                nodes,
                federation_view: "local_only".to_string(),
                federation_view_reason: Some(format!("device credentials are unavailable: {e}")),
                resolve_latency_ms: None,
            };
        }
    };
    let (bridge, _state) = match config::load_and_connect() {
        Ok(pair) => pair,
        Err(e) => {
            return FleetView {
                nodes,
                federation_view: "local_only".to_string(),
                federation_view_reason: Some(format!("local runtime bridge is unavailable: {e}")),
                resolve_latency_ms: None,
            };
        }
    };

    let caller_uri = crate::uri::device_uri(&creds.tenant_id, &creds.node_id);
    let invoker = BridgeAbilityInvoker::with_caller_uri(&bridge, caller_uri);

    let resolve_started = Instant::now();
    let resolved = match advertise::resolve_agents_with_filter(
        &invoker,
        &creds.tenant_id,
        &creds.tenant_id,
        "",
        true,
        None,
    ) {
        Ok(r) => r,
        Err(e) => {
            return FleetView {
                nodes,
                federation_view: "local_only".to_string(),
                federation_view_reason: Some(format!(
                    "federation.resolve failed against realm {:?}: {e}",
                    creds.tenant_id
                )),
                resolve_latency_ms: Some(resolve_started.elapsed().as_millis() as u64),
            };
        }
    };
    let resolve_latency_ms = Some(resolve_started.elapsed().as_millis() as u64);

    let mut device_agents: BTreeMap<String, String> = BTreeMap::new();
    for agent in resolved {
        if agent.status != "active" || !is_device_profile_agent(&agent) {
            continue;
        }
        if let Some(node_id) = node_id_from_agent_uri(&agent.uri) {
            device_agents.entry(node_id).or_insert(agent.uri);
        }
    }

    let mut probed = 0usize;
    for (node_id, agent_uri) in device_agents {
        if node_id == local.node_id {
            if let Some(self_node) = nodes.first_mut() {
                self_node.agent_uri = Some(agent_uri);
            }
            continue;
        }
        let probe = if probed < MAX_DEVICE_PROBES {
            probed += 1;
            probe_remote_device(&invoker, &creds.tenant_id, &creds.tenant_id, &agent_uri)
        } else {
            ProbeOutcome {
                online: true,
                state: "PROBATION",
                probe_status: "directory_only",
                probe_error: Some(format!(
                    "probe budget exceeded after {MAX_DEVICE_PROBES} devices"
                )),
                latency_ms: None,
            }
        };
        nodes.push(FleetNodeSnapshot {
            node_id,
            tenant_id: local.tenant_id.clone(),
            agent_uri: Some(agent_uri),
            is_self: false,
            paired: true,
            hub_endpoint: None,
            state: probe.state.to_string(),
            online: probe.online,
            probe_status: probe.probe_status.to_string(),
            probe_error: probe.probe_error,
            latency_ms: probe.latency_ms,
        });
    }

    nodes.sort_by(|a, b| {
        b.is_self
            .cmp(&a.is_self)
            .then_with(|| a.node_id.cmp(&b.node_id))
    });

    let federation_view_reason = if nodes.len() == 1 {
        Some(
            "realm directory was reachable, but no peer device profiles were advertised"
                .to_string(),
        )
    } else {
        None
    };
    FleetView {
        nodes,
        federation_view: "federated".to_string(),
        federation_view_reason,
        resolve_latency_ms,
    }
}

pub(crate) fn node_to_json(node: &FleetNodeSnapshot) -> Value {
    json!({
        "node_id": node.node_id.clone(),
        "tenant_id": node.tenant_id.clone(),
        "agent_uri": node.agent_uri.clone(),
        "is_self": node.is_self,
        "paired": node.paired,
        "hub_endpoint": node.hub_endpoint.clone(),
        "state": node.state.clone(),
        "online": node.online,
        "probe_status": node.probe_status.clone(),
        "probe_error": node.probe_error.clone(),
        "latency_ms": node.latency_ms,
    })
}

/// URI v4.1.4: extract the device UUID from a device-profile URA.
/// Strict per the v4.1.4 ontology — only `device/<uuid>` URAs are
/// recognised. v1 `reg/agent.<id>` and v2 `agent/<id>` shapes return
/// None (caller treats as "not a device row" and skips). Migration
/// callers needing the legacy shape must `easynet device join`
/// again to mint a v4.1.4 URA.
pub(crate) fn node_id_from_agent_uri(uri: &str) -> Option<String> {
    let parsed = crate::uri::parse_ura(uri).ok()?;
    if parsed.device_id.is_empty() {
        None
    } else {
        Some(parsed.device_id)
    }
}

fn is_device_profile_agent(agent: &ResolvedAgent) -> bool {
    let mut has_health = false;
    let mut has_fleet = false;
    let mut has_network = false;
    for desc in &agent.abilities {
        let Some(name) = desc.get("name").and_then(Value::as_str) else {
            continue;
        };
        has_health |= matches_ability_name(name, DEVICE_HEALTH_ABILITY);
        has_fleet |= matches_ability_name(name, DEVICE_FLEET_ABILITY);
        has_network |= matches_ability_name(name, DEVICE_NETWORK_HEALTH_ABILITY);
    }
    has_health && (has_fleet || has_network)
}

fn matches_ability_name(candidate: &str, expected: &str) -> bool {
    candidate == expected || candidate.ends_with(&format!(".{expected}"))
}

fn probe_remote_device(
    invoker: &BridgeAbilityInvoker<'_>,
    tenant_id: &str,
    realm: &str,
    agent_uri: &str,
) -> ProbeOutcome {
    let started = Instant::now();
    let receipt = advertise::forward_invoke(
        invoker,
        tenant_id,
        realm,
        agent_uri,
        DEVICE_HEALTH_ABILITY,
        &json!({
            "source": "fleet.list_nodes",
            "probe": "alive",
        }),
    );
    match receipt {
        Ok(receipt) => probe_outcome_from_receipt(receipt, started.elapsed().as_millis() as u64),
        Err(e) => ProbeOutcome {
            online: false,
            state: "SUSPECT",
            probe_status: "probe_failed",
            probe_error: Some(e),
            latency_ms: Some(started.elapsed().as_millis() as u64),
        },
    }
}

fn probe_outcome_from_receipt(receipt: ForwardInvokeReceipt, latency_ms: u64) -> ProbeOutcome {
    if !receipt.ok {
        let error = format!(
            "{}{}{}",
            receipt.error_code,
            if receipt.error_code.is_empty() || receipt.error_message.is_empty() {
                ""
            } else {
                ": "
            },
            receipt.error_message
        );
        return ProbeOutcome {
            online: false,
            state: "SUSPECT",
            probe_status: "probe_failed",
            probe_error: Some(if error.is_empty() {
                "forward_invoke returned ok=false".to_string()
            } else {
                error
            }),
            latency_ms: Some(latency_ms),
        };
    }
    if !receipt.result_b64.is_empty() {
        let decoded = match BASE64_STANDARD.decode(&receipt.result_b64) {
            Ok(bytes) => bytes,
            Err(e) => {
                return ProbeOutcome {
                    online: false,
                    state: "SUSPECT",
                    probe_status: "probe_failed",
                    probe_error: Some(format!("decode forward_invoke result_b64: {e}")),
                    latency_ms: Some(latency_ms),
                };
            }
        };
        if let Err(e) = serde_json::from_slice::<Value>(&decoded) {
            return ProbeOutcome {
                online: false,
                state: "SUSPECT",
                probe_status: "probe_failed",
                probe_error: Some(format!("parse forward_invoke result body: {e}")),
                latency_ms: Some(latency_ms),
            };
        }
    }
    ProbeOutcome {
        online: true,
        state: "HEALTHY",
        probe_status: "reachable",
        probe_error: None,
        latency_ms: Some(latency_ms),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_from_v414_device_uri_extracts_uuid() {
        // URI v4.1.4: device-profile URA is `device/<uuid>`.
        let uuid = "4065c47a-ec6f-4330-87a5-0d69787709b8";
        assert_eq!(
            node_id_from_agent_uri(&format!("easynet:///r/localhost/device/{uuid}")),
            Some(uuid.to_string())
        );
    }

    #[test]
    fn node_id_from_v1_v2_shapes_returns_none() {
        // URI v4.1.4 strict parser: v1 `reg/agent.<id>` and v2
        // `agent/<id>` shapes are no longer recognised. Caller
        // treats None as "skip this row" — pre-v4.1.4 credentials
        // require `easynet device join` to mint a v4.1.4 URA.
        assert_eq!(
            node_id_from_agent_uri("easynet:///r/acme/agent/01DEV"),
            None,
            "v2 agent/<id> shape must not parse as a v4.1.4 device"
        );
        assert_eq!(
            node_id_from_agent_uri("easynet:///r/prv/reg/agent.01DEV?tenant_id=acme"),
            None,
            "v1 reg/agent.<id> shape must not parse as a v4.1.4 device"
        );
    }

    #[test]
    fn device_profile_detection_requires_health_plus_device_surface() {
        let device = ResolvedAgent {
            uri: "easynet:///r/acme/agent/01DEV".into(),
            status: "active".into(),
            abilities: vec![
                json!({"name": "observe.health"}),
                json!({"name": "fleet.list_nodes"}),
            ],
        };
        let hosted = ResolvedAgent {
            uri: "easynet:///r/acme/agent/01LLM".into(),
            status: "active".into(),
            abilities: vec![json!({"name": "alice.chat"})],
        };
        assert!(is_device_profile_agent(&device));
        assert!(!is_device_profile_agent(&hosted));
    }

    #[test]
    fn node_to_json_preserves_explicit_online_flag() {
        let node = FleetNodeSnapshot {
            node_id: "01DEV".into(),
            tenant_id: "acme".into(),
            agent_uri: Some("easynet:///r/acme/agent/01DEV".into()),
            is_self: false,
            paired: true,
            hub_endpoint: None,
            state: "SUSPECT".into(),
            online: false,
            probe_status: "probe_failed".into(),
            probe_error: Some("timeout".into()),
            latency_ms: Some(123),
        };
        let value = node_to_json(&node);
        assert_eq!(value["online"], Value::Bool(false));
        assert_eq!(value["probe_status"], "probe_failed");
        assert_eq!(value["latency_ms"], 123);
    }
}
