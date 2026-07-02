// EasyNet CLI — plugin realtime activation planning
// =================================================
//
// File: src/runtime/plugin_host/realtime.rs
// Description: Runtime read model for plugin-declared realtime capabilities.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::runtime::plugin_host::manifest::{
    PluginPackageManifest, PluginRealtimeCapability, PluginRealtimeKind, PluginRealtimeMode,
    PluginRealtimeTransport,
};
use crate::runtime::system_abilities::resources::media;

/// Runtime readiness for one plugin realtime capability declaration.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginRealtimeActivationStatus {
    /// The daemon ability catalog was not supplied, so readiness is unknown.
    Unknown,
    /// Every required activation ability is currently available.
    Ready,
    /// Some, but not all, required activation abilities are currently available.
    Partial,
    /// No executable activation path is currently available.
    Blocked,
    /// The declaration names modes that have no canonical daemon ability and no
    /// plugin-owned activation ability.
    Unsupported,
}

/// Daemon-owned activation read model for one realtime capability.
///
/// This is not an AbilityDescriptor and it does not register handlers. It only
/// tells the UI/CLI which existing daemon abilities can activate the declared
/// realtime surface, and which gaps still need implementation.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PluginRealtimeActivationPlan {
    pub package_id: String,
    pub package_version: String,
    pub capability: PluginRealtimeCapability,
    #[serde(default)]
    pub transport_adapter: PluginRealtimeTransportReadiness,
    pub canonical_abilities: Vec<String>,
    pub activation_abilities: Vec<String>,
    pub available_abilities: Vec<String>,
    pub missing_abilities: Vec<String>,
    pub unsupported_modes: Vec<PluginRealtimeMode>,
    pub status: PluginRealtimeActivationStatus,
}

impl PluginRealtimeActivationPlan {
    pub fn is_quick_add(&self) -> bool {
        self.capability.quick_add()
    }

    pub fn required_abilities(&self) -> Vec<String> {
        if self.activation_abilities.is_empty() {
            self.canonical_abilities.clone()
        } else {
            self.activation_abilities.clone()
        }
    }
}

/// Readiness of a declared realtime transport and its optional fallback.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PluginRealtimeTransportReadiness {
    pub primary: PluginRealtimeTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<PluginRealtimeTransport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<PluginRealtimeTransport>,
    pub status: PluginRealtimeTransportReadinessStatus,
    pub adapters: Vec<PluginRealtimeTransportAdapterReadiness>,
}

impl Default for PluginRealtimeTransportReadiness {
    fn default() -> Self {
        Self {
            primary: PluginRealtimeTransport::InvokeBidi,
            fallback: None,
            selected: None,
            status: PluginRealtimeTransportReadinessStatus::Unknown,
            adapters: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginRealtimeTransportReadinessStatus {
    Unknown,
    Ready,
    FallbackReady,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PluginRealtimeTransportAdapterReadiness {
    pub transport: PluginRealtimeTransport,
    pub required_abilities: Vec<String>,
    pub available_abilities: Vec<String>,
    pub missing_abilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<PluginRealtimeTransportRoleReadiness>,
    pub status: PluginRealtimeTransportAdapterStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginRealtimeTransportAdapterStatus {
    Unknown,
    Ready,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PluginRealtimeTransportRoleReadiness {
    pub role: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ability: Option<String>,
    pub status: PluginRealtimeTransportRoleStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginRealtimeTransportRoleStatus {
    Unknown,
    Satisfied,
    Missing,
}

/// Daemon-owned realtime transport adapter registry.
///
/// The registry does not instantiate media endpoints. It maps plugin-declared
/// transports (`invoke_stream`, `invoke_bidi`, `webrtc`) onto the daemon
/// ability surface that can activate them. Concrete WebRTC SDP/ICE/media state
/// stays inside the plugin implementation that contributed those abilities.
#[derive(Clone, Debug, Default)]
pub struct PluginRealtimeTransportAdapterRegistry;

impl PluginRealtimeTransportAdapterRegistry {
    pub fn readiness_for(
        &self,
        capability: &PluginRealtimeCapability,
        required_abilities: &[String],
        package_abilities: &BTreeSet<String>,
        daemon_abilities: Option<&BTreeSet<String>>,
    ) -> PluginRealtimeTransportReadiness {
        let primary = adapter_readiness_for_transport(
            capability.transport(),
            required_abilities,
            package_abilities,
            daemon_abilities,
        );
        let fallback = capability.fallback_transport().map(|transport| {
            adapter_readiness_for_transport(
                transport,
                required_abilities,
                package_abilities,
                daemon_abilities,
            )
        });
        let selected = if primary.status == PluginRealtimeTransportAdapterStatus::Ready {
            Some(primary.transport)
        } else {
            fallback
                .as_ref()
                .filter(|readiness| readiness.status == PluginRealtimeTransportAdapterStatus::Ready)
                .map(|readiness| readiness.transport)
        };
        let status = match (
            daemon_abilities,
            selected,
            primary.status,
            fallback.as_ref(),
        ) {
            (None, _, _, _) => PluginRealtimeTransportReadinessStatus::Unknown,
            (Some(_), Some(selected), _, _) if selected == capability.transport() => {
                PluginRealtimeTransportReadinessStatus::Ready
            }
            (Some(_), Some(_), _, _) => PluginRealtimeTransportReadinessStatus::FallbackReady,
            (Some(_), None, PluginRealtimeTransportAdapterStatus::Unknown, _) => {
                PluginRealtimeTransportReadinessStatus::Unknown
            }
            (Some(_), None, _, Some(fallback))
                if fallback.status == PluginRealtimeTransportAdapterStatus::Unknown =>
            {
                PluginRealtimeTransportReadinessStatus::Unknown
            }
            (Some(_), None, _, _) => PluginRealtimeTransportReadinessStatus::Blocked,
        };
        let mut adapters = vec![primary];
        if let Some(fallback) = fallback {
            adapters.push(fallback);
        }
        PluginRealtimeTransportReadiness {
            primary: capability.transport(),
            fallback: capability.fallback_transport(),
            selected,
            status,
            adapters,
        }
    }
}

/// Project every realtime capability in a package into an activation plan.
pub fn activation_plans_for_manifest(
    package_id: &str,
    package_version: &str,
    manifest: &PluginPackageManifest,
    daemon_abilities: Option<&BTreeSet<String>>,
) -> Vec<PluginRealtimeActivationPlan> {
    let package_abilities = manifest
        .abilities()
        .iter()
        .map(|ability| ability.name().to_string())
        .collect::<BTreeSet<_>>();
    manifest
        .realtime_capabilities()
        .iter()
        .map(|capability| {
            activation_plan_for_capability(
                package_id,
                package_version,
                capability,
                &package_abilities,
                daemon_abilities,
            )
        })
        .collect()
}

fn activation_plan_for_capability(
    package_id: &str,
    package_version: &str,
    capability: &PluginRealtimeCapability,
    package_abilities: &BTreeSet<String>,
    daemon_abilities: Option<&BTreeSet<String>>,
) -> PluginRealtimeActivationPlan {
    let mut canonical_abilities = Vec::new();
    let mut unsupported_modes = Vec::new();
    for mode in capability.modes() {
        match canonical_abilities_for_mode(capability.kind(), *mode) {
            Some(names) => {
                for name in names {
                    push_unique(&mut canonical_abilities, (*name).to_string());
                }
            }
            None => push_unique(&mut unsupported_modes, *mode),
        }
    }

    let activation_abilities = capability.activation_abilities().to_vec();
    if !activation_abilities.is_empty() {
        unsupported_modes.clear();
    }
    let required_abilities = if activation_abilities.is_empty() {
        canonical_abilities.clone()
    } else {
        activation_abilities.clone()
    };

    let (available_abilities, missing_abilities, status) = match daemon_abilities {
        None => (
            Vec::new(),
            Vec::new(),
            PluginRealtimeActivationStatus::Unknown,
        ),
        Some(daemon_abilities) => {
            let mut available = Vec::new();
            let mut missing = Vec::new();
            for ability in &required_abilities {
                if daemon_abilities.contains(ability) {
                    available.push(ability.clone());
                } else {
                    missing.push(ability.clone());
                }
            }
            let status = activation_status(
                activation_abilities.is_empty(),
                &available,
                &missing,
                &unsupported_modes,
            );
            (available, missing, status)
        }
    };

    let transport_adapter = PluginRealtimeTransportAdapterRegistry.readiness_for(
        capability,
        &required_abilities,
        package_abilities,
        daemon_abilities,
    );

    PluginRealtimeActivationPlan {
        package_id: package_id.to_string(),
        package_version: package_version.to_string(),
        capability: capability.clone(),
        transport_adapter,
        canonical_abilities,
        activation_abilities,
        available_abilities,
        missing_abilities,
        unsupported_modes,
        status,
    }
}

fn adapter_readiness_for_transport(
    transport: PluginRealtimeTransport,
    required_abilities: &[String],
    package_abilities: &BTreeSet<String>,
    daemon_abilities: Option<&BTreeSet<String>>,
) -> PluginRealtimeTransportAdapterReadiness {
    let roles = transport_roles_for(transport, package_abilities, daemon_abilities);
    let role_missing_abilities = roles
        .iter()
        .filter(|role| role.required && role.status != PluginRealtimeTransportRoleStatus::Satisfied)
        .filter_map(|role| role.ability.clone())
        .collect::<Vec<_>>();
    let missing_required_roles = roles
        .iter()
        .any(|role| role.required && role.status == PluginRealtimeTransportRoleStatus::Missing);
    let (available_abilities, missing_abilities, status) = match daemon_abilities {
        None => (
            Vec::new(),
            Vec::new(),
            PluginRealtimeTransportAdapterStatus::Unknown,
        ),
        Some(daemon_abilities) => {
            let mut available = Vec::new();
            let mut missing = Vec::new();
            for ability in required_abilities {
                if daemon_abilities.contains(ability) {
                    available.push(ability.clone());
                } else {
                    missing.push(ability.clone());
                }
            }
            for ability in role_missing_abilities {
                push_unique(&mut missing, ability);
            }
            let status = if missing.is_empty() && !missing_required_roles {
                PluginRealtimeTransportAdapterStatus::Ready
            } else {
                PluginRealtimeTransportAdapterStatus::Blocked
            };
            (available, missing, status)
        }
    };
    PluginRealtimeTransportAdapterReadiness {
        transport,
        required_abilities: required_abilities.to_vec(),
        available_abilities,
        missing_abilities,
        roles,
        status,
    }
}

fn transport_roles_for(
    transport: PluginRealtimeTransport,
    package_abilities: &BTreeSet<String>,
    daemon_abilities: Option<&BTreeSet<String>>,
) -> Vec<PluginRealtimeTransportRoleReadiness> {
    match transport {
        PluginRealtimeTransport::Webrtc => [
            ("session_create", ".create_session"),
            ("description_exchange", ".set_description"),
            ("ice_trickle", ".add_ice_candidate"),
            ("session_end", ".end_session"),
        ]
        .into_iter()
        .map(|(role, suffix)| required_role(role, suffix, package_abilities, daemon_abilities))
        .collect(),
        PluginRealtimeTransport::InvokeStream | PluginRealtimeTransport::InvokeBidi => Vec::new(),
    }
}

fn required_role(
    role: &str,
    suffix: &str,
    package_abilities: &BTreeSet<String>,
    daemon_abilities: Option<&BTreeSet<String>>,
) -> PluginRealtimeTransportRoleReadiness {
    let ability = package_abilities
        .iter()
        .find(|ability| ability.ends_with(suffix))
        .cloned();
    let status = match (daemon_abilities, ability.as_ref()) {
        (None, _) => PluginRealtimeTransportRoleStatus::Unknown,
        (Some(_), None) => PluginRealtimeTransportRoleStatus::Missing,
        (Some(daemon), Some(ability)) if daemon.contains(ability) => {
            PluginRealtimeTransportRoleStatus::Satisfied
        }
        (Some(_), Some(_)) => PluginRealtimeTransportRoleStatus::Missing,
    };
    PluginRealtimeTransportRoleReadiness {
        role: role.to_string(),
        required: true,
        ability,
        status,
    }
}

fn activation_status(
    uses_canonical_abilities: bool,
    available: &[String],
    missing: &[String],
    unsupported_modes: &[PluginRealtimeMode],
) -> PluginRealtimeActivationStatus {
    if uses_canonical_abilities && !unsupported_modes.is_empty() && missing.is_empty() {
        return PluginRealtimeActivationStatus::Unsupported;
    }
    if missing.is_empty() {
        return PluginRealtimeActivationStatus::Ready;
    }
    if available.is_empty() {
        return PluginRealtimeActivationStatus::Blocked;
    }
    PluginRealtimeActivationStatus::Partial
}

fn canonical_abilities_for_mode(
    kind: PluginRealtimeKind,
    mode: PluginRealtimeMode,
) -> Option<&'static [&'static str]> {
    match (kind, mode) {
        (PluginRealtimeKind::Camera, PluginRealtimeMode::Snapshot) => {
            Some(&[media::ABILITY_CAMERA_SNAPSHOT])
        }
        (PluginRealtimeKind::Camera, PluginRealtimeMode::Subscribe) => {
            Some(&[media::ABILITY_CAMERA_SUBSCRIBE])
        }
        (PluginRealtimeKind::Camera, PluginRealtimeMode::Record) => Some(&[
            media::ABILITY_CAMERA_RECORD_START,
            media::ABILITY_CAMERA_RECORD_STOP,
        ]),
        (PluginRealtimeKind::Mic, PluginRealtimeMode::Subscribe) => {
            Some(&[media::ABILITY_MIC_SUBSCRIBE])
        }
        (PluginRealtimeKind::Screen, PluginRealtimeMode::Snapshot) => {
            Some(&[media::ABILITY_SCREEN_SNAPSHOT])
        }
        (PluginRealtimeKind::Screen, PluginRealtimeMode::Subscribe) => {
            Some(&[media::ABILITY_SCREEN_SUBSCRIBE])
        }
        (PluginRealtimeKind::Speaker, PluginRealtimeMode::Publish) => {
            Some(&[media::ABILITY_SPEAKER_PUBLISH])
        }
        (PluginRealtimeKind::Voice, PluginRealtimeMode::Subscribe) => {
            Some(&[media::ABILITY_VOICE_SUBSCRIBE])
        }
        (PluginRealtimeKind::Voice, PluginRealtimeMode::Transcribe) => {
            Some(&[media::ABILITY_VOICE_TRANSCRIBE])
        }
        (PluginRealtimeKind::Mic, PluginRealtimeMode::Record)
        | (PluginRealtimeKind::Screen, PluginRealtimeMode::Record) => None,
        _ => None,
    }
}

fn push_unique<T>(items: &mut Vec<T>, item: T)
where
    T: PartialEq,
{
    if !items.contains(&item) {
        items.push(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::plugin_host::PluginPackageManifest;

    #[test]
    fn activation_plan_uses_declared_plugin_activation_abilities() {
        let manifest = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            &test_manifest(
                r#"
[[ability_metadata]]
name = "test.camera.open"
layer = "operational"
call_mode = "bidi"
bidi_wire_kind = "json_frames"

[[realtime_capability]]
kind = "camera"
modes = ["snapshot", "subscribe"]
transport = "invoke_bidi"
activation_abilities = ["test.camera.open"]
resources = ["camera"]
quick_add = true
"#,
            ),
        )
        .expect("manifest");
        let mut daemon = BTreeSet::new();
        daemon.insert("test.camera.open".to_string());

        let plans = activation_plans_for_manifest("test.plugin", "0.1.0", &manifest, Some(&daemon));

        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].status, PluginRealtimeActivationStatus::Ready);
        assert_eq!(
            plans[0].activation_abilities,
            vec!["test.camera.open".to_string()]
        );
        assert_eq!(
            plans[0].canonical_abilities,
            vec![
                "camera.snapshot".to_string(),
                "camera.subscribe".to_string()
            ]
        );
        assert!(plans[0].unsupported_modes.is_empty());
    }

    #[test]
    fn activation_plan_reports_canonical_missing_abilities() {
        let manifest = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            &test_manifest(
                r#"
[[ability_metadata]]
name = "test.camera"
layer = "operational"
call_mode = "bidi"

[[realtime_capability]]
kind = "camera"
modes = ["snapshot", "record"]
transport = "invoke_stream"
resources = ["camera"]
"#,
            ),
        )
        .expect("manifest");
        let mut daemon = BTreeSet::new();
        daemon.insert("camera.snapshot".to_string());

        let plans = activation_plans_for_manifest("test.plugin", "0.1.0", &manifest, Some(&daemon));

        assert_eq!(plans[0].status, PluginRealtimeActivationStatus::Partial);
        assert_eq!(
            plans[0].available_abilities,
            vec!["camera.snapshot".to_string()]
        );
        assert_eq!(
            plans[0].missing_abilities,
            vec![
                "camera.record_start".to_string(),
                "camera.record_stop".to_string()
            ]
        );
    }

    #[test]
    fn activation_plan_reports_unsupported_mode_without_plugin_binding() {
        let manifest = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            &test_manifest(
                r#"
[[ability_metadata]]
name = "test.screen"
layer = "operational"
call_mode = "bidi"

[[realtime_capability]]
kind = "screen"
modes = ["record"]
transport = "invoke_bidi"
resources = ["display"]
"#,
            ),
        )
        .expect("manifest");
        let daemon = BTreeSet::new();

        let plans = activation_plans_for_manifest("test.plugin", "0.1.0", &manifest, Some(&daemon));

        assert_eq!(plans[0].status, PluginRealtimeActivationStatus::Unsupported);
        assert_eq!(plans[0].unsupported_modes, [PluginRealtimeMode::Record]);
    }

    #[test]
    fn activation_plan_treats_plugin_bound_noncanonical_mode_as_supported() {
        let manifest = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            &test_manifest(
                r#"
[[ability_metadata]]
name = "test.screen.open"
layer = "operational"
call_mode = "bidi"

[[realtime_capability]]
kind = "screen"
modes = ["record"]
transport = "invoke_bidi"
activation_abilities = ["test.screen.open"]
resources = ["display"]
"#,
            ),
        )
        .expect("manifest");
        let daemon = BTreeSet::from(["test.screen.open".to_string()]);

        let plans = activation_plans_for_manifest("test.plugin", "0.1.0", &manifest, Some(&daemon));

        assert_eq!(plans[0].status, PluginRealtimeActivationStatus::Ready);
        assert!(plans[0].unsupported_modes.is_empty());
        assert_eq!(
            plans[0].available_abilities,
            vec!["test.screen.open".to_string()]
        );
    }

    #[test]
    fn activation_plan_maps_webrtc_transport_and_fallback_adapter() {
        let manifest = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            &test_manifest(
                r#"
[[ability_metadata]]
name = "test.screen.create_session"
layer = "control"
call_mode = "rpc"

[[ability_metadata]]
name = "test.screen.set_description"
layer = "control"
call_mode = "rpc"

[[ability_metadata]]
name = "test.screen.add_ice_candidate"
layer = "control"
call_mode = "rpc"

[[ability_metadata]]
name = "test.screen.end_session"
layer = "control"
call_mode = "rpc"

[[ability_metadata]]
name = "test.screen.open"
layer = "operational"
call_mode = "bidi"

[[realtime_capability]]
kind = "screen"
modes = ["subscribe", "record"]
transport = "webrtc"
fallback_transport = "invoke_bidi"
activation_abilities = [
  "test.screen.create_session",
  "test.screen.set_description",
  "test.screen.add_ice_candidate",
  "test.screen.end_session",
  "test.screen.open",
]
resources = ["display"]
"#,
            ),
        )
        .expect("manifest");
        let daemon = BTreeSet::from([
            "test.screen.create_session".to_string(),
            "test.screen.set_description".to_string(),
            "test.screen.add_ice_candidate".to_string(),
            "test.screen.end_session".to_string(),
            "test.screen.open".to_string(),
        ]);

        let plans = activation_plans_for_manifest("test.plugin", "0.1.0", &manifest, Some(&daemon));

        assert_eq!(
            plans[0].transport_adapter.primary,
            PluginRealtimeTransport::Webrtc
        );
        assert_eq!(
            plans[0].transport_adapter.fallback,
            Some(PluginRealtimeTransport::InvokeBidi)
        );
        assert_eq!(
            plans[0].transport_adapter.selected,
            Some(PluginRealtimeTransport::Webrtc)
        );
        assert_eq!(
            plans[0].transport_adapter.status,
            PluginRealtimeTransportReadinessStatus::Ready
        );
        assert_eq!(plans[0].transport_adapter.adapters.len(), 2);
        assert_eq!(
            plans[0].transport_adapter.adapters[0].status,
            PluginRealtimeTransportAdapterStatus::Ready
        );
        assert_eq!(plans[0].transport_adapter.adapters[0].roles.len(), 4);
        assert!(plans[0].transport_adapter.adapters[0]
            .roles
            .iter()
            .all(|role| role.status == PluginRealtimeTransportRoleStatus::Satisfied));
    }

    fn test_manifest(extra: &str) -> String {
        format!(
            r#"
schema_version = "1"
id = "test.plugin"
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

{extra}
"#
        )
    }
}
