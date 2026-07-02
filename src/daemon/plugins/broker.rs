// EasyNet CLI — daemon plugin resource and policy brokers
// =======================================================
//
// File: src/daemon/plugins/broker.rs
// Description: Daemon-owned readiness brokers for plugin activation metadata.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::daemon::plugins::manifest::PluginRealtimeCapability;
use crate::daemon::plugins::realtime::{
    PluginRealtimeActivationPlan, PluginRealtimeActivationStatus, PluginRealtimeTransportReadiness,
    PluginRealtimeTransportReadinessStatus,
};
use crate::daemon::plugins::surface::{PluginPackageSurfaceRecord, PluginSurfaceReport};
use crate::persistence::resources::ResourcesFile;

/// Package-level activation broker for plugin realtime capabilities.
///
/// This broker composes resource, permission, and transport readiness into the
/// operator-facing activation outcome. Lifecycle abilities call this broker;
/// they do not own resource or policy interpretation.
pub struct PluginActivationBroker {
    resource_broker: PluginResourceBroker,
    policy_broker: PluginPolicyBroker,
}

impl PluginActivationBroker {
    pub fn new(resources: &ResourcesFile) -> Self {
        Self {
            resource_broker: PluginResourceBroker::new(resources),
            policy_broker: PluginPolicyBroker,
        }
    }

    pub fn realtime_outcomes(
        &self,
        surface: &PluginSurfaceReport,
        package_id: &str,
        package_version: Option<&str>,
    ) -> Vec<PluginRealtimeActivationOutcome> {
        surface
            .packages
            .iter()
            .filter(|package| package.package_id == package_id)
            .filter(|package| {
                package_version
                    .map(|version| package.package_version == version)
                    .unwrap_or(true)
            })
            .flat_map(|package| {
                package
                    .realtime_activation_plans
                    .iter()
                    .map(|plan| self.activation_outcome(package, plan))
            })
            .collect()
    }

    fn activation_outcome(
        &self,
        package: &PluginPackageSurfaceRecord,
        plan: &PluginRealtimeActivationPlan,
    ) -> PluginRealtimeActivationOutcome {
        let resources = self.resource_broker.readiness(plan.capability.resources());
        let permissions = self
            .policy_broker
            .readiness(plan.capability.permissions(), plan);
        let status = outcome_status(plan.status, resources.ready, plan.transport_adapter.status);
        PluginRealtimeActivationOutcome {
            package_id: package.package_id.clone(),
            package_version: package.package_version.clone(),
            quick_add: plan.is_quick_add(),
            ready: status == PluginRealtimeOutcomeStatus::Ready,
            status,
            plan_status: plan.status,
            capability: plan.capability.clone(),
            activation_abilities: plan.activation_abilities.clone(),
            canonical_abilities: plan.canonical_abilities.clone(),
            available_abilities: plan.available_abilities.clone(),
            missing_abilities: plan.missing_abilities.clone(),
            unsupported_modes: plan.unsupported_modes.clone(),
            transport: plan.transport_adapter.clone(),
            resources,
            permissions,
            publish: publish_readiness(plan),
        }
    }
}

/// Read-only resource broker for plugin-declared local resource needs.
///
/// What this is NOT: live device arbitration. It projects the daemon's
/// persisted resource table into activation readiness; handlers still verify
/// live availability at invocation time.
pub struct PluginResourceBroker {
    resource_counts: BTreeMap<String, usize>,
}

impl PluginResourceBroker {
    pub fn new(resources: &ResourcesFile) -> Self {
        let mut resource_counts = BTreeMap::new();
        for entry in &resources.resources {
            *resource_counts
                .entry(entry.kind.as_str().to_string())
                .or_insert(0) += 1;
        }
        Self { resource_counts }
    }

    pub fn readiness(&self, required_resources: &[String]) -> PluginRealtimeResourceReadiness {
        let required = normalize_unique(required_resources);
        let mut available = Vec::new();
        let mut missing = Vec::new();
        for required_kind in &required {
            let count = self
                .resource_counts
                .get(required_kind.as_str())
                .copied()
                .unwrap_or(0);
            if count == 0 {
                missing.push(required_kind.clone());
            } else {
                available.push(PluginRealtimeResourceMatch {
                    kind: required_kind.clone(),
                    count,
                });
            }
        }
        PluginRealtimeResourceReadiness {
            required,
            ready: missing.is_empty(),
            available,
            missing,
        }
    }
}

/// Read-only policy broker for plugin-declared permission needs.
///
/// This broker reports whether the daemon currently exposes a plugin-owned
/// status/request path for the declared permissions. Permission grant, denial,
/// user consent, and Axon admission remain owned by their existing handlers.
#[derive(Clone, Copy, Debug, Default)]
pub struct PluginPolicyBroker;

impl PluginPolicyBroker {
    pub fn readiness(
        self,
        required_permissions: &[String],
        plan: &PluginRealtimeActivationPlan,
    ) -> PluginRealtimePermissionReadiness {
        let required = normalize_unique(required_permissions);
        let status_abilities = plan
            .available_abilities
            .iter()
            .filter(|ability| ability.ends_with(".permission_status"))
            .cloned()
            .collect::<Vec<_>>();
        let request_abilities = plan
            .available_abilities
            .iter()
            .filter(|ability| ability.ends_with(".request_permission"))
            .cloned()
            .collect::<Vec<_>>();
        let status = if required.is_empty() {
            PluginRealtimePermissionStatus::NotRequired
        } else if !status_abilities.is_empty() {
            PluginRealtimePermissionStatus::StatusAbilityAvailable
        } else if !request_abilities.is_empty() {
            PluginRealtimePermissionStatus::RequestAbilityAvailable
        } else {
            PluginRealtimePermissionStatus::Unknown
        };
        PluginRealtimePermissionReadiness {
            required,
            status_abilities,
            request_abilities,
            status,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginRealtimeResourceReadiness {
    pub required: Vec<String>,
    pub available: Vec<PluginRealtimeResourceMatch>,
    pub missing: Vec<String>,
    pub ready: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginRealtimeResourceMatch {
    pub kind: String,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginRealtimePermissionReadiness {
    pub required: Vec<String>,
    pub status_abilities: Vec<String>,
    pub request_abilities: Vec<String>,
    pub status: PluginRealtimePermissionStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRealtimePermissionStatus {
    NotRequired,
    StatusAbilityAvailable,
    RequestAbilityAvailable,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginRealtimeActivationOutcome {
    pub package_id: String,
    pub package_version: String,
    pub quick_add: bool,
    pub ready: bool,
    pub status: PluginRealtimeOutcomeStatus,
    pub plan_status: PluginRealtimeActivationStatus,
    pub capability: PluginRealtimeCapability,
    pub activation_abilities: Vec<String>,
    pub canonical_abilities: Vec<String>,
    pub available_abilities: Vec<String>,
    pub missing_abilities: Vec<String>,
    pub unsupported_modes: Vec<crate::daemon::plugins::PluginRealtimeMode>,
    pub transport: PluginRealtimeTransportReadiness,
    pub resources: PluginRealtimeResourceReadiness,
    pub permissions: PluginRealtimePermissionReadiness,
    pub publish: PluginRealtimePublishReadiness,
}

/// Typed response returned by `plugin.activate_realtime`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginRealtimeActivationReport {
    pub ok: bool,
    pub package_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
    pub outcomes: Vec<PluginRealtimeActivationOutcome>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRealtimeOutcomeStatus {
    Ready,
    Blocked,
    Partial,
    Unsupported,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginRealtimePublishReadiness {
    pub local_runtime: String,
    pub realm_advertise: String,
    pub reason: String,
}

fn outcome_status(
    plan_status: PluginRealtimeActivationStatus,
    resources_ready: bool,
    transport_status: PluginRealtimeTransportReadinessStatus,
) -> PluginRealtimeOutcomeStatus {
    match plan_status {
        PluginRealtimeActivationStatus::Ready
            if resources_ready
                && matches!(
                    transport_status,
                    PluginRealtimeTransportReadinessStatus::Ready
                        | PluginRealtimeTransportReadinessStatus::FallbackReady
                ) =>
        {
            PluginRealtimeOutcomeStatus::Ready
        }
        PluginRealtimeActivationStatus::Ready
            if matches!(
                transport_status,
                PluginRealtimeTransportReadinessStatus::Unknown
            ) =>
        {
            PluginRealtimeOutcomeStatus::Unknown
        }
        PluginRealtimeActivationStatus::Ready => PluginRealtimeOutcomeStatus::Blocked,
        PluginRealtimeActivationStatus::Partial => PluginRealtimeOutcomeStatus::Partial,
        PluginRealtimeActivationStatus::Blocked => PluginRealtimeOutcomeStatus::Blocked,
        PluginRealtimeActivationStatus::Unsupported => PluginRealtimeOutcomeStatus::Unsupported,
        PluginRealtimeActivationStatus::Unknown => PluginRealtimeOutcomeStatus::Unknown,
    }
}

fn publish_readiness(plan: &PluginRealtimeActivationPlan) -> PluginRealtimePublishReadiness {
    let local_runtime = if plan.missing_abilities.is_empty()
        && plan.status != PluginRealtimeActivationStatus::Unknown
    {
        "activation_abilities_available"
    } else {
        "activation_abilities_incomplete"
    };
    PluginRealtimePublishReadiness {
        local_runtime: local_runtime.to_string(),
        realm_advertise: "deferred".to_string(),
        reason: "plugin.activate_realtime checks the daemon-local runtime surface; realm advertisement still uses the daemon publish/bootstrap path"
            .to_string(),
    }
}

fn normalize_unique(items: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for item in items {
        let trimmed = item.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::daemon::plugins::surface::PluginKindView;
    use crate::daemon::plugins::{
        activation_plans_for_manifest, PluginPackageManifest, PluginRealtimePermissionStatus,
        PluginRealtimeTransportReadinessStatus, PluginRealtimeTransportRoleStatus,
        PluginSurfaceReport,
    };
    use crate::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use std::collections::BTreeSet;

    #[test]
    fn resource_broker_counts_declared_resource_kinds() {
        let resources = ResourcesFile {
            resources: vec![
                resource(ResourceType::Display, "display-1"),
                resource(ResourceType::Window, "window-1"),
                resource(ResourceType::Window, "window-2"),
            ],
        };

        let readiness = PluginResourceBroker::new(&resources).readiness(&[
            "display".to_string(),
            "window".to_string(),
            "camera".to_string(),
            "window".to_string(),
        ]);

        assert!(!readiness.ready);
        assert_eq!(readiness.missing, vec!["camera".to_string()]);
        assert_eq!(
            readiness.available,
            vec![
                PluginRealtimeResourceMatch {
                    kind: "display".to_string(),
                    count: 1,
                },
                PluginRealtimeResourceMatch {
                    kind: "window".to_string(),
                    count: 2,
                }
            ]
        );
    }

    #[test]
    fn policy_broker_reports_permission_action_path() {
        let manifest = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            r#"
schema_version = "1"
id = "test.permissions"
version = "0.1.0"
kind = "sidecar"
entrypoint = "bin/plugin"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "test.permission_status"
layer = "observation"
call_mode = "rpc"

[[realtime_capability]]
kind = "camera"
modes = ["snapshot"]
transport = "invoke_stream"
activation_abilities = ["test.permission_status"]
permissions = ["camera"]
"#,
        )
        .expect("manifest");
        let daemon = BTreeSet::from(["test.permission_status".to_string()]);
        let plans =
            activation_plans_for_manifest("test.permissions", "0.1.0", &manifest, Some(&daemon));

        let readiness = PluginPolicyBroker.readiness(&["camera".to_string()], &plans[0]);

        assert_eq!(
            readiness.status,
            PluginRealtimePermissionStatus::StatusAbilityAvailable
        );
        assert_eq!(readiness.status_abilities, vec!["test.permission_status"]);
    }

    #[test]
    fn activation_broker_blocks_webrtc_when_signaling_roles_are_missing() {
        let manifest = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            r#"
schema_version = "1"
id = "test.webrtc"
version = "0.1.0"
kind = "sidecar"
entrypoint = "bin/plugin"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "test.screen.open"
layer = "operational"
call_mode = "bidi"

[[realtime_capability]]
kind = "screen"
modes = ["subscribe"]
transport = "webrtc"
activation_abilities = ["test.screen.open"]
resources = ["display"]
"#,
        )
        .expect("manifest");
        let daemon = BTreeSet::from(["test.screen.open".to_string()]);
        let surface = PluginSurfaceReport {
            packages: vec![PluginPackageSurfaceRecord {
                package_id: "test.webrtc".to_string(),
                package_version: "0.1.0".to_string(),
                kind: PluginKindView::Sidecar,
                planned_load_status: "loaded".to_string(),
                daemon_runtime_status: "loaded".to_string(),
                load_status: "loaded".to_string(),
                ability_count: 1,
                descriptor_published: true,
                runtime_published: true,
                invokable: true,
                realtime_activation_plans: activation_plans_for_manifest(
                    "test.webrtc",
                    "0.1.0",
                    &manifest,
                    Some(&daemon),
                ),
                error: None,
            }],
            abilities: Vec::new(),
        };
        let resources = ResourcesFile {
            resources: vec![resource(ResourceType::Display, "display-1")],
        };

        let outcomes = PluginActivationBroker::new(&resources).realtime_outcomes(
            &surface,
            "test.webrtc",
            None,
        );

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, PluginRealtimeOutcomeStatus::Blocked);
        assert_eq!(
            outcomes[0].transport.status,
            PluginRealtimeTransportReadinessStatus::Blocked
        );
        assert!(outcomes[0].transport.adapters[0].roles.iter().any(|role| {
            role.role == "description_exchange"
                && role.status == PluginRealtimeTransportRoleStatus::Missing
        }));
    }

    fn resource(kind: ResourceType, hardware_id: &str) -> ResourceEntry {
        ResourceEntry {
            resource_ura: crate::persistence::resources::build_resource_ura("acme", hardware_id),
            owner_agent: crate::ura::device_ura("acme", "dev-a"),
            kind,
            binding: ResourceBinding::LocalDevice,
            hardware_id: hardware_id.to_string(),
            display_name: hardware_id.to_string(),
            metadata: json!({}),
            first_seen_at: "2026-07-01T00:00:00Z".to_string(),
        }
    }
}
