// EasyNet CLI — federation-backed device probe helpers
// ====================================================
//
// File: src/daemon/ability/builtins/integrations/federation_probe.rs
// Description: Shared device-discovery + direct-probe helpers used by
//              node.list and observe.network_health.
//
// Why this module exists
// ----------------------
// The current bug is not "no heartbeat packet exists"; it is that the
// operator-facing surfaces were reading local files and hard-coding
// HEALTHY/local_only instead of consulting the live federation path.
// This module centralises the two live steps we do have today:
//
//   1. `federation.resolve` against the realm directory.
//   2. A signed canonical `InvokeRequest` for `observe.health` against
//      each device-profile Agent.
//
// Keeping the logic here gives one bounded, testable definition of
// "what devices are visible" and "what counts as reachable" instead of
// four call sites drifting independently.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
#[cfg(feature = "axon-pb")]
use std::time::Duration;
use std::time::Instant;

use serde_json::{json, Value};

use crate::daemon::ability::builtins::agents::discover::DiscoverFederationResolver;
use crate::daemon::ability::names::{federation, governance};
use crate::daemon::federation::client::ability_contract::ResolvedAgent;
#[cfg(feature = "axon-pb")]
use crate::daemon::invocation::routing::remote_invoke::{
    RemoteAbilityInvocationTarget, RemoteInvocationSubject, RemoteSystemInvocationIssuer,
};
use crate::daemon::persistence::config;

const DEVICE_HEALTH_ABILITY: &str = governance::OBSERVE_HEALTH;
const DEVICE_NODE_LIST_ABILITY: &str = federation::NODE_LIST;
const DEVICE_NETWORK_HEALTH_ABILITY: &str = governance::OBSERVE_NETWORK_HEALTH;
const MAX_DEVICE_PROBES: usize = 64;

#[derive(Debug, Clone)]
pub(crate) struct LocalIdentity {
    pub node_id: String,
    pub tenant_id: String,
    pub hub_endpoint: Option<String>,
    pub paired: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DeviceNodeSnapshot {
    pub node_id: String,
    pub tenant_id: String,
    pub agent_ura: Option<String>,
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
pub(crate) struct DeviceNetworkView {
    pub nodes: Vec<DeviceNodeSnapshot>,
    pub unavailable_nodes: Vec<DeviceNodeSnapshot>,
    pub federation_view: String,
    pub federation_view_reason: Option<String>,
    pub resolve_latency_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedDeviceRecord {
    pub node: DeviceNodeSnapshot,
    pub ability_summaries: Vec<Value>,
}

/// Build the local device record from the daemon's canonical catalog. This
/// path is deliberately independent of the optional federation bridge: the
/// daemon owns its local identity and descriptors even in daemon-only mode.
pub(crate) fn local_device_record(
    catalog: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
) -> anyhow::Result<Option<ResolvedDeviceRecord>> {
    let local = local_identity();
    if !local.paired {
        return Ok(None);
    }
    let owner_ura = crate::core::ura::device_ura(&local.tenant_id, &local.node_id);
    let abilities =
        crate::daemon::ability::catalog::LocalAbilityPublicationSnapshot::capture(catalog)
            .owner_projection_values(&owner_ura)
            .map_err(|error| anyhow::anyhow!("local device ability publication: {error}"))?;
    Ok(Some(ResolvedDeviceRecord {
        node: DeviceNodeSnapshot {
            node_id: local.node_id.clone(),
            tenant_id: local.tenant_id.clone(),
            agent_ura: Some(crate::core::ura::device_ura(
                &local.tenant_id,
                &local.node_id,
            )),
            is_self: true,
            paired: true,
            hub_endpoint: local.hub_endpoint,
            state: "HEALTHY".to_string(),
            online: true,
            probe_status: "local".to_string(),
            probe_error: None,
            latency_ms: None,
        },
        ability_summaries: abilities,
    }))
}

#[derive(Debug, Clone)]
struct ProbeOutcome {
    online: bool,
    state: &'static str,
    probe_status: &'static str,
    probe_error: Option<String>,
    latency_ms: Option<u64>,
}

trait DeviceProbe {
    fn probe(&self, agent_ura: &str) -> ProbeOutcome;
}

struct RemoteDeviceProbe;

impl DeviceProbe for RemoteDeviceProbe {
    fn probe(&self, agent_ura: &str) -> ProbeOutcome {
        probe_remote_device(agent_ura)
    }
}

#[derive(Debug, Default)]
struct DeviceProfileAbilitySet {
    has_health: bool,
    has_fleet: bool,
    has_network: bool,
}

impl DeviceProfileAbilitySet {
    fn observe(&mut self, public_name: &str) {
        self.has_health |= public_name == expected_device_public_ability(DEVICE_HEALTH_ABILITY);
        self.has_fleet |= public_name == expected_device_public_ability(DEVICE_NODE_LIST_ABILITY);
        self.has_network |=
            public_name == expected_device_public_ability(DEVICE_NETWORK_HEALTH_ABILITY);
    }

    fn is_device_profile(&self) -> bool {
        self.has_health && (self.has_fleet || self.has_network)
    }
}

pub(crate) fn local_identity() -> LocalIdentity {
    match config::load_credentials() {
        Ok(c) => LocalIdentity {
            node_id: c.node_id,
            tenant_id: c.realm,
            hub_endpoint: if c.hub_endpoint.trim().is_empty() {
                None
            } else {
                Some(c.hub_endpoint)
            },
            paired: true,
        },
        Err(_) => LocalIdentity {
            node_id: crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_DEVICE_ID
                .to_string(),
            tenant_id: crate::daemon::identity::local_invocation::UNPAIRED_LOCAL_REALM.to_string(),
            hub_endpoint: None,
            paired: false,
        },
    }
}

pub(crate) fn collect_device_view(resolver: &dyn DiscoverFederationResolver) -> DeviceNetworkView {
    collect_device_view_with_probe(resolver, &RemoteDeviceProbe)
}

fn collect_device_view_with_probe(
    resolver: &dyn DiscoverFederationResolver,
    probe: &dyn DeviceProbe,
) -> DeviceNetworkView {
    let local = local_identity();
    let mut nodes = vec![DeviceNodeSnapshot {
        node_id: local.node_id.clone(),
        tenant_id: local.tenant_id.clone(),
        agent_ura: if local.paired {
            Some(crate::core::ura::device_ura(
                &local.tenant_id,
                &local.node_id,
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
    let mut unavailable_nodes = Vec::new();
    if !local.paired {
        return DeviceNetworkView {
            nodes,
            unavailable_nodes,
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
            return DeviceNetworkView {
                nodes,
                unavailable_nodes,
                federation_view: "local_only".to_string(),
                federation_view_reason: Some(format!("device credentials are unavailable: {e}")),
                resolve_latency_ms: None,
            };
        }
    };
    let caller_ura = crate::core::ura::device_ura(&creds.realm, &creds.node_id);

    let resolve_started = Instant::now();
    let resolved = match resolver.resolve_agents(&creds.realm, &creds.realm, caller_ura, None) {
        Ok(r) => r,
        Err(e) => {
            return DeviceNetworkView {
                nodes,
                unavailable_nodes,
                federation_view: "local_only".to_string(),
                federation_view_reason: Some(format!(
                    "federation.resolve failed against realm {:?}: {e}",
                    creds.realm
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
        if let Some(node_id) = node_id_from_agent_ura(&agent.ura) {
            device_agents.entry(node_id).or_insert(agent.ura);
        }
    }

    let mut probed = 0usize;
    for (node_id, agent_ura) in device_agents {
        if node_id == local.node_id {
            if let Some(self_node) = nodes.first_mut() {
                self_node.agent_ura = Some(agent_ura);
            }
            continue;
        }
        let probe = if probed < MAX_DEVICE_PROBES {
            probed += 1;
            probe.probe(&agent_ura)
        } else {
            ProbeOutcome {
                online: false,
                state: "UNAVAILABLE",
                probe_status: "probe_budget_exceeded",
                probe_error: Some(format!(
                    "probe budget exceeded after {MAX_DEVICE_PROBES} devices; device is not route-visible"
                )),
                latency_ms: None,
            }
        };
        let node = DeviceNodeSnapshot {
            node_id,
            tenant_id: local.tenant_id.clone(),
            agent_ura: Some(agent_ura),
            is_self: false,
            paired: true,
            hub_endpoint: None,
            state: probe.state.to_string(),
            online: probe.online,
            probe_status: probe.probe_status.to_string(),
            probe_error: probe.probe_error,
            latency_ms: probe.latency_ms,
        };
        if node.online {
            nodes.push(node);
        } else {
            unavailable_nodes.push(node);
        }
    }

    nodes.sort_by(|a, b| {
        b.is_self
            .cmp(&a.is_self)
            .then_with(|| a.node_id.cmp(&b.node_id))
    });
    unavailable_nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));

    let federation_view_reason = if nodes.len() == 1 {
        Some(if unavailable_nodes.is_empty() {
            "realm directory was reachable, but no peer device profiles were advertised".to_string()
        } else {
            format!(
                "realm directory was reachable, but {} peer device profile(s) were not route-visible",
                unavailable_nodes.len()
            )
        })
    } else {
        None
    };
    DeviceNetworkView {
        nodes,
        unavailable_nodes,
        federation_view: "federated".to_string(),
        federation_view_reason,
        resolve_latency_ms,
    }
}

/// Resolve one device by concrete `node_id`. Search the caller's own
/// tenant first, then fall back to the cross-tenant catalogue (`*`)
/// so `node.describe` can locate cross-hub peers by the UUID
/// operators already have in hand.
pub(crate) fn resolve_device_record(
    resolver: &dyn DiscoverFederationResolver,
    node_id: &str,
) -> anyhow::Result<Option<ResolvedDeviceRecord>> {
    resolve_device_record_with_probe(resolver, node_id, &RemoteDeviceProbe)
}

fn resolve_device_record_with_probe(
    resolver: &dyn DiscoverFederationResolver,
    node_id: &str,
    probe: &dyn DeviceProbe,
) -> anyhow::Result<Option<ResolvedDeviceRecord>> {
    let local = local_identity();
    if !local.paired {
        return Ok(None);
    }

    let creds = config::load_credentials()
        .map_err(|e| anyhow::anyhow!("device credentials are unavailable: {e}"))?;
    let caller_ura = crate::core::ura::device_ura(&creds.realm, &creds.node_id);

    if let Some(record) =
        resolve_device_record_with_filter(resolver, &creds, &caller_ura, node_id, None, probe)?
    {
        return Ok(Some(record));
    }
    resolve_device_record_with_filter(
        resolver,
        &creds,
        &caller_ura,
        node_id,
        Some("*".to_string()),
        probe,
    )
}

fn resolve_device_record_with_filter(
    resolver: &dyn DiscoverFederationResolver,
    creds: &config::Credentials,
    caller_ura: &str,
    node_id: &str,
    tenant_filter: Option<String>,
    probe: &dyn DeviceProbe,
) -> anyhow::Result<Option<ResolvedDeviceRecord>> {
    let resolved = resolver
        .resolve_agents(
            &creds.realm,
            &creds.realm,
            caller_ura.to_string(),
            tenant_filter,
        )
        .map_err(|e| {
            anyhow::anyhow!(
                "federation.resolve failed against realm {:?}: {e}",
                creds.realm
            )
        })?;

    for agent in resolved {
        if agent.status != "active" || !is_device_profile_agent(&agent) {
            continue;
        }
        let Some(resolved_node_id) = node_id_from_agent_ura(&agent.ura) else {
            continue;
        };
        if resolved_node_id != node_id {
            continue;
        }

        let agent_realm = crate::core::ura::realm_from_ura(&agent.ura);
        let is_self = resolved_node_id == creds.node_id && agent_realm == creds.realm;
        let probe = if is_self {
            ProbeOutcome {
                online: true,
                state: "HEALTHY",
                probe_status: "local",
                probe_error: None,
                latency_ms: None,
            }
        } else {
            probe.probe(&agent.ura)
        };
        if !probe.online {
            anyhow::bail!(
                "node.describe: node {node_id:?} is not route-visible: {}{}",
                probe.probe_status,
                probe
                    .probe_error
                    .as_deref()
                    .map(|error| format!(" ({error})"))
                    .unwrap_or_default()
            );
        }

        return Ok(Some(ResolvedDeviceRecord {
            node: DeviceNodeSnapshot {
                node_id: resolved_node_id,
                tenant_id: if agent_realm.is_empty() {
                    creds.realm.clone()
                } else {
                    agent_realm
                },
                agent_ura: Some(agent.ura.clone()),
                is_self,
                paired: true,
                hub_endpoint: if is_self && !creds.hub_endpoint.trim().is_empty() {
                    Some(creds.hub_endpoint.clone())
                } else {
                    None
                },
                state: probe.state.to_string(),
                online: probe.online,
                probe_status: probe.probe_status.to_string(),
                probe_error: probe.probe_error,
                latency_ms: probe.latency_ms,
            },
            ability_summaries: agent.ability_summaries.clone(),
        }));
    }

    Ok(None)
}

pub(crate) fn node_to_json(node: &DeviceNodeSnapshot) -> Value {
    json!({
        "node_id": node.node_id.clone(),
        "tenant_id": node.tenant_id.clone(),
        "agent_ura": node.agent_ura.clone(),
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

/// Extract the node id from a canonical device-profile URA.
pub(crate) fn node_id_from_agent_ura(ura: &str) -> Option<String> {
    if let Ok(parsed) = crate::core::ura::parse_ura(ura) {
        return parsed.device_id().map(str::to_string);
    }
    None
}

fn is_device_profile_agent(agent: &ResolvedAgent) -> bool {
    let owner_ura = agent.ura.trim();
    if node_id_from_agent_ura(owner_ura).is_none() {
        return false;
    }

    let mut abilities = DeviceProfileAbilitySet::default();
    for desc in &agent.ability_summaries {
        let Some(public_name) = device_profile_public_ability_name(owner_ura, desc) else {
            continue;
        };
        abilities.observe(&public_name);
    }
    abilities.is_device_profile()
}

fn device_profile_public_ability_name(owner_ura: &str, summary: &Value) -> Option<String> {
    let summary =
        crate::daemon::federation::read_model::owner_projection::summary_from_value(summary)?;
    let ability_ura = summary.ability_ura.trim();
    if ability_ura.is_empty() {
        return None;
    }
    crate::core::ura::public_ability_name_from_ability_ura(owner_ura, ability_ura)
}

fn expected_device_public_ability(registry_key: &str) -> &str {
    registry_key.strip_prefix("device.").unwrap_or(registry_key)
}

fn probe_remote_device(agent_ura: &str) -> ProbeOutcome {
    let started = Instant::now();
    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = agent_ura;
        return ProbeOutcome {
            online: false,
            state: "UNKNOWN",
            probe_status: "transport_unavailable",
            probe_error: Some("remote device probe requires the axon-pb feature".to_string()),
            latency_ms: Some(started.elapsed().as_millis() as u64),
        };
    }

    #[cfg(feature = "axon-pb")]
    let result = (|| {
        let target = RemoteAbilityInvocationTarget::for_target_owned_selector(
            agent_ura,
            DEVICE_HEALTH_ABILITY,
        )?;
        let request = RemoteSystemInvocationIssuer::root_plan(
            &target,
            crate::daemon::identity::local_invocation::local_daemon_ura()?,
            RemoteInvocationSubject::DaemonTargetOwned(target.callee_ura().to_string()),
            json!({
                "source": "node.list",
                "probe": "alive",
            }),
            Duration::from_secs(30),
        )?
        .into_request()?;
        crate::daemon::invocation::routing::remote_invoke::invoke_remote_target(request)
    })();
    #[cfg(feature = "axon-pb")]
    match result {
        Ok(_) => ProbeOutcome {
            online: true,
            state: "HEALTHY",
            probe_status: "reachable",
            probe_error: None,
            latency_ms: Some(started.elapsed().as_millis() as u64),
        },
        Err(e) => ProbeOutcome {
            online: false,
            state: "SUSPECT",
            probe_status: "probe_failed",
            probe_error: Some(e.to_string()),
            latency_ms: Some(started.elapsed().as_millis() as u64),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct StaticResolver {
        agents: Vec<ResolvedAgent>,
    }

    impl DiscoverFederationResolver for StaticResolver {
        fn resolve_agents(
            &self,
            _tenant: &str,
            _realm: &str,
            _caller_ura: String,
            _tenant_filter: Option<String>,
        ) -> Result<
            Vec<crate::daemon::federation::client::ability_contract::ResolvedAgent>,
            crate::daemon::ability::builtins::agents::discover::DiscoverFederationResolveError,
        > {
            Ok(self.agents.clone())
        }
    }

    #[derive(Debug)]
    struct FixedProbe {
        outcome: ProbeOutcome,
    }

    impl DeviceProbe for FixedProbe {
        fn probe(&self, _agent_ura: &str) -> ProbeOutcome {
            self.outcome.clone()
        }
    }

    fn save_paired_test_credentials() {
        config::save_credentials(&config::Credentials {
            node_id: "local-node".into(),
            credential_token: "token".into(),
            hub_endpoint: "https://hub.example:50443".into(),
            realm: "acme".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        })
        .expect("save paired test credentials");
    }

    fn unreachable_probe() -> FixedProbe {
        FixedProbe {
            outcome: ProbeOutcome {
                online: false,
                state: "SUSPECT",
                probe_status: "probe_failed",
                probe_error: Some("owner is not online".into()),
                latency_ms: Some(7),
            },
        }
    }

    #[test]
    fn node_id_from_v414_device_ura_extracts_uuid() {
        // URA v4.1.4: device-profile URA is `device/<uuid>`.
        let uuid = "4065c47a-ec6f-4330-87a5-0d69787709b8";
        assert_eq!(
            node_id_from_agent_ura(&crate::core::ura::device_ura("localhost", uuid)),
            Some(uuid.to_string())
        );
    }

    #[test]
    fn node_id_from_agent_ura_rejects_non_device_shapes() {
        assert_eq!(
            node_id_from_agent_ura("easynet:///r/acme/agent/01DEV"),
            None,
            "agent URAs must not project as devices"
        );
        assert_eq!(
            node_id_from_agent_ura("easynet:///r/acme/agent/alice.claude"),
            None,
            "real agent URAs must not parse as devices"
        );
        assert_eq!(
            node_id_from_agent_ura("easynet:///r/acme/ability/device.01DEV.federation.probe"),
            None,
            "ability URAs must not project as devices"
        );
    }

    #[test]
    fn device_profile_detection_requires_health_plus_device_surface() {
        let device_ura = "easynet:///r/acme/device/01DEV";
        let device = ResolvedAgent {
            ura: device_ura.into(),
            status: "active".into(),
            host_node_id: None,
            ability_summaries: vec![
                ability_summary(device_ura, "observe", "health"),
                ability_summary(device_ura, "node", "list"),
            ],
        };
        let hosted_ura = "easynet:///r/acme/agent/u1.01LLM";
        let hosted = ResolvedAgent {
            ura: hosted_ura.into(),
            status: "active".into(),
            host_node_id: None,
            ability_summaries: vec![ability_summary(hosted_ura, "", "chat")],
        };
        assert!(is_device_profile_agent(&device));
        assert!(!is_device_profile_agent(&hosted));
    }

    #[test]
    fn node_to_json_preserves_explicit_online_flag() {
        let node = DeviceNodeSnapshot {
            node_id: "01DEV".into(),
            tenant_id: "acme".into(),
            agent_ura: Some("easynet:///r/acme/device/01DEV".into()),
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

    #[test]
    fn collect_device_view_does_not_expose_unrouteable_directory_devices_as_nodes() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        save_paired_test_credentials();
        let remote_device_ura = crate::core::ura::device_ura("acme", "remote-node");
        let resolver = StaticResolver {
            agents: vec![ResolvedAgent {
                ura: remote_device_ura,
                status: "active".into(),
                host_node_id: None,
                ability_summaries: vec![
                    ability_summary("easynet:///r/acme/device/remote-node", "observe", "health"),
                    ability_summary("easynet:///r/acme/device/remote-node", "node", "list"),
                ],
            }],
        };

        let view = collect_device_view_with_probe(&resolver, &unreachable_probe());

        assert_eq!(
            view.nodes.iter().filter(|node| !node.is_self).count(),
            0,
            "unreachable directory rows must not be product-selectable nodes: {view:#?}"
        );
        assert_eq!(view.unavailable_nodes.len(), 1);
        assert_eq!(view.unavailable_nodes[0].node_id, "remote-node");
        assert_eq!(view.unavailable_nodes[0].probe_status, "probe_failed");
        assert!(view
            .federation_view_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("not route-visible")));
    }

    #[test]
    fn resolve_device_record_rejects_unrouteable_directory_ability_facts() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        save_paired_test_credentials();
        let remote_device_ura = crate::core::ura::device_ura("acme", "remote-node");
        let resolver = StaticResolver {
            agents: vec![ResolvedAgent {
                ura: remote_device_ura,
                status: "active".into(),
                host_node_id: None,
                ability_summaries: vec![
                    ability_summary("easynet:///r/acme/device/remote-node", "observe", "health"),
                    ability_summary("easynet:///r/acme/device/remote-node", "node", "list"),
                    ability_summary(
                        "easynet:///r/acme/device/remote-node",
                        "browser",
                        "open_session",
                    ),
                ],
            }],
        };

        let error =
            resolve_device_record_with_probe(&resolver, "remote-node", &unreachable_probe())
                .expect_err("unrouteable device must not return stale ability_summaries");

        let message = error.to_string();
        assert!(
            message.contains("not route-visible"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains("owner is not online"),
            "probe evidence must be preserved: {message}"
        );
    }

    #[test]
    fn resolved_device_record_keeps_cross_tenant_realm_and_abilities() {
        let device_ura = "easynet:///r/realm-b/device/01DEV";
        let agent = ResolvedAgent {
            ura: device_ura.into(),
            status: "active".into(),
            host_node_id: None,
            ability_summaries: vec![
                ability_summary(device_ura, "observe", "health"),
                ability_summary(device_ura, "shell", "run"),
            ],
        };
        let resolved_node_id = node_id_from_agent_ura(&agent.ura).expect("node id");
        let realm = crate::core::ura::realm_from_ura(&agent.ura);
        let record = ResolvedDeviceRecord {
            node: DeviceNodeSnapshot {
                node_id: resolved_node_id,
                tenant_id: realm,
                agent_ura: Some(agent.ura.clone()),
                is_self: false,
                paired: true,
                hub_endpoint: None,
                state: "HEALTHY".into(),
                online: true,
                probe_status: "reachable".into(),
                probe_error: None,
                latency_ms: Some(5),
            },
            ability_summaries: agent.ability_summaries.clone(),
        };
        assert_eq!(record.node.tenant_id, "realm-b");
        assert_eq!(record.ability_summaries.len(), 2);
        assert_eq!(record.ability_summaries[1]["namespace"], "shell");
        assert_eq!(record.ability_summaries[1]["local_name"], "run");
    }

    #[test]
    fn device_profile_detection_uses_ability_ura_not_summary_names() {
        let device_ura = "easynet:///r/acme/device/01DEV";
        let mut health = ability_summary(device_ura, "wrong", "health");
        health["ability_ura"] =
            json!(crate::core::ura::owner_ability_ura(device_ura, "observe.health").unwrap());
        let mut fleet = ability_summary(device_ura, "wrong", "list");
        fleet["ability_ura"] =
            json!(crate::core::ura::owner_ability_ura(device_ura, "node.list").unwrap());
        let device = ResolvedAgent {
            ura: device_ura.into(),
            status: "active".into(),
            host_node_id: None,
            ability_summaries: vec![health, fleet],
        };

        assert!(is_device_profile_agent(&device));
    }

    #[test]
    fn device_profile_detection_rejects_wrong_owner_ability_uras() {
        let device_ura = "easynet:///r/acme/device/01DEV";
        let other_device_ura = "easynet:///r/acme/device/01OTHER";
        let device = ResolvedAgent {
            ura: device_ura.into(),
            status: "active".into(),
            host_node_id: None,
            ability_summaries: vec![
                ability_summary(other_device_ura, "observe", "health"),
                ability_summary(other_device_ura, "node", "list"),
            ],
        };

        assert!(!is_device_profile_agent(&device));
    }

    #[test]
    fn device_profile_detection_rejects_missing_ability_ura() {
        let device_ura = "easynet:///r/acme/device/01DEV";
        let mut health = ability_summary(device_ura, "observe", "health");
        health["ability_ura"] = Value::Null;
        let device = ResolvedAgent {
            ura: device_ura.into(),
            status: "active".into(),
            host_node_id: None,
            ability_summaries: vec![health, ability_summary(device_ura, "node", "list")],
        };

        assert!(!is_device_profile_agent(&device));
    }

    fn ability_summary(owner_ura: &str, namespace: &str, local_name: &str) -> Value {
        let public_name = if namespace.is_empty() {
            local_name.to_string()
        } else {
            format!("{namespace}.{local_name}")
        };
        let ability_ura = crate::core::ura::owner_ability_ura(owner_ura, &public_name)
            .expect("owner ability URA");
        json!({
            "ability_ura": ability_ura,
            "owner_ura": owner_ura,
            "namespace": namespace,
            "local_name": local_name,
            "descriptor_revision": "sha256:descriptor",
            "schema_ref": Value::Null,
            "schema_hash": Value::Null,
            "policy_ref": "visibility:PUBLIC",
            "route_summary_ref": Value::Null,
            "tags": ["class:unary"],
        })
    }
}
