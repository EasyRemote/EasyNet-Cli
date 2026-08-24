//! Agent profile registry — RFC-001 §1 implementation profiles.
//!
//! Per AXON-RFC-001 plan v4.1.2 §A4: "profile" is documentation
//! shorthand for "an Agent advertising the corresponding ability
//! namespace". These are NOT protocol-level types or kind values.
//! They are implementation modules that group ability handlers by
//! the Agent projection that advertises them.
//!
//! Registered profiles
//! -------------------
//!   device   — remaining daemon-local host abilities advertised by the
//!              direct Device-owner migration projection: selected
//!              lifecycle/governance/introspection bridges, and compatibility
//!              surfaces awaiting SystemAgent migration.
//!   system-agent:locomotion — fs.*, process.exec, shell.run, http.request,
//!              fs.transfer, and net.tunnel advertised by the device-sponsored locomotion
//!              SystemAgent.
//!   system-agent:terminal — terminal.* PTY lifecycle and I/O abilities
//!              advertised by the device-sponsored terminal SystemAgent.
//!   system-agent:session — session.list and session.attach observation/control
//!              abilities advertised by the device-sponsored session
//!              SystemAgent. `session.open` remains runtime-admin.
//!   system-agent:node-management — node.describe and node.remove node
//!              directory/lifecycle abilities advertised by the
//!              device-sponsored node-management SystemAgent.
//!   system-agent:automation — mission.*, discuss.*, loop.*, and schedule.*
//!              control abilities advertised by the device-sponsored
//!              automation SystemAgent.
//!   system-agent:runtime-governance — authority.binding.*, policy.request.*,
//!              and admission.explain abilities advertised by the
//!              device-sponsored runtime-governance SystemAgent.
//!   system-agent:runtime-health — observe.health, observe.network_health, and
//!              admin.status advertised by the device-sponsored runtime-health
//!              SystemAgent.
//!   system-agent:runtime-introspection — meta.describe,
//!              meta.list_abilities, and meta.list_resources advertised by the
//!              device-sponsored runtime-introspection SystemAgent.
//!   system-agent:descriptor-transfer — meta.teach, meta.acquire, and
//!              meta.forget advertised by the device-sponsored
//!              descriptor-transfer SystemAgent.
//!   system-agent:api-key-management — `<user>.api_key.*` local bearer-token
//!              lifecycle abilities advertised by the device-sponsored
//!              api-key-management SystemAgent.
//!   system-agent:keyring-management — `device.keyring.*` legacy local-name
//!              managed-signing/keyring administration abilities advertised by
//!              the device-sponsored keyring-management SystemAgent. The
//!              `device.` name prefix is not the owner/callee identity.
//!   system-agent:ability-management — ability.publish, ability.unpublish,
//!              ability.deploy, and ability.uninstall control abilities
//!              advertised by the device-sponsored ability-management
//!              SystemAgent.
//!   service:<user>.pages — daemon-hosted Pages registry and project handlers
//!              advertised by the principal-scoped Pages Service. Page
//!              resource identity remains user-scoped; execution remains
//!              bound to the hosting Device through the descriptor host facts.
//!   system-agent:files — daemon-local blob-store abilities advertised by the
//!              sponsoring Device's Files SystemAgent.
//!   system-agent:openai-compat — openai.* compatibility adapter abilities
//!              advertised by the device-sponsored openai-compat SystemAgent.
//!   system-agent:remote-desktop — remote_desktop.* product abilities
//!              advertised by the device-sponsored remote-desktop SystemAgent;
//!              plugin-management owns plugin lifecycle, not this product API.
//!   system-agent:a2a-integration — a2a.bridge.* and a2a.client.send_task
//!              adapter abilities advertised by the device-sponsored
//!              a2a-integration SystemAgent.
//!   system-agent:consent-management — consent.* human-in-the-loop approval
//!              abilities advertised by a device-sponsored SystemAgent.
//!   mcp      — mcp.bridge.* + mcp.client.* (edge MCP adapter, single
//!              Agent projection advertises both inbound and outbound per
//!              RFC §1 [P6])
//!   llm      — conversation.* plus private per-skill abilities advertised by
//!              each LLM sub-agent projection (claude / codex / etc.)
//!
//! Handler bodies live under `daemon::ability::builtins` or remaining
//! migration-phase agent modules. Profile files only declare WHICH
//! abilities each profile advertises.
//!
//! See:
//!   docs/rfc/AXON-RFC-001-plan-v4.1.2.md §1 — profile catalogue
//!   docs/rfc/AXON-RFC-001-plan-v4.1.2.md §18 — standard ability registry
//!   docs/rfc/AXON-RFC-001-restatement-mapping.md — old → new mapping

pub mod bootstrap;
pub mod device;
pub mod llm;
pub mod mcp;

struct SystemAgentDescriptorProjection {
    system_agent_id: &'static str,
    owner: fn() -> crate::daemon::ability::dispatch::OwnerKind,
}

const SYSTEM_AGENT_DESCRIPTOR_PROJECTIONS: &[SystemAgentDescriptorProjection] = &[
    SystemAgentDescriptorProjection {
        system_agent_id: crate::daemon::ability::names::agents::AGENT_MANAGEMENT_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::agent_management_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id: crate::daemon::ability::names::device_control::LOCOMOTION_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::locomotion_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id: crate::daemon::ability::names::device_control::TERMINAL_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::terminal_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id: crate::daemon::ability::names::device_control::SESSION_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::session_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id:
            crate::daemon::ability::names::device_control::NODE_MANAGEMENT_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::node_management_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id: crate::daemon::ability::names::resources::SKILL_MANAGEMENT_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::skill_management_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id: crate::daemon::ability::names::resources::CONTEXT_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::context_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id: crate::daemon::ability::names::resources::FILES_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::files_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id: crate::daemon::ability::names::resources::MEDIA_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::media_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id:
            crate::daemon::ability::names::integrations::PLUGIN_MANAGEMENT_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::plugin_management_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id:
            crate::daemon::ability::names::integrations::REMOTE_DESKTOP_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::remote_desktop_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id:
            crate::daemon::ability::names::integrations::MCP_INTEGRATION_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::mcp_integration_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id: crate::daemon::ability::names::automation::AUTOMATION_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::automation_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id:
            crate::daemon::ability::names::governance::RUNTIME_GOVERNANCE_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::runtime_governance_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id: crate::daemon::ability::names::governance::CONSENT_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::consent_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id: crate::daemon::ability::names::governance::RUNTIME_HEALTH_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::runtime_health_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id:
            crate::daemon::ability::names::governance::RUNTIME_INTROSPECTION_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::runtime_introspection_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id:
            crate::daemon::ability::names::governance::DESCRIPTOR_TRANSFER_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::descriptor_transfer_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id:
            crate::daemon::ability::names::governance::API_KEY_MANAGEMENT_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::api_key_management_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id:
            crate::daemon::ability::names::governance::KEYRING_MANAGEMENT_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::keyring_management_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id:
            crate::daemon::ability::names::federation::ABILITY_MANAGEMENT_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::ability_management_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id: crate::daemon::ability::names::integrations::OPENAI_COMPAT_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::openai_compat_system,
    },
    SystemAgentDescriptorProjection {
        system_agent_id:
            crate::daemon::ability::names::integrations::A2A_INTEGRATION_SYSTEM_AGENT_ID,
        owner: crate::daemon::ability::dispatch::OwnerKind::a2a_integration_system,
    },
];

pub(crate) fn is_declared_daemon_native_system_agent_id(system_agent_id: &str) -> bool {
    SYSTEM_AGENT_DESCRIPTOR_PROJECTIONS
        .iter()
        .any(|projection| projection.system_agent_id == system_agent_id)
}

fn system_descriptors_for_owner(
    owner_ura: &str,
    owner: crate::daemon::ability::dispatch::OwnerKind,
    visibility_for: impl Fn(&str) -> crate::daemon::ability::descriptors::Visibility,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    crate::daemon::ability::catalog::published_system_abilities_for_owner(owner)
        .into_iter()
        .map(|descriptor| {
            let visibility = visibility_for(&descriptor.name);
            descriptor
                .rebind_owner_ura(owner_ura)
                .expect("registry-derived descriptor accepts canonical profile owner")
                .with_visibility(visibility)
                .with_source("kernel:built-in")
        })
        .collect()
}

/// Read `~/.easynet/local-agents.json` and project it into the full
/// host descriptor catalog. Returns an empty catalog when the file is
/// missing, malformed, or does not yet contain a canonical host authority
/// URA. Pre-join state has no routable descriptor identity; it is not
/// represented by a synthetic owner locator.
///
/// Single source of truth for the recipe used by both the MCP stdio
/// server (advertised tool surface) and the in-process
/// `mcp.bridge.list_tools` ability handler — keeping them on one
/// helper is what guarantees external and internal MCP callers see
/// the same catalog.
pub fn load_host_descriptors() -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    use crate::daemon::ability::descriptors::AbilityDescriptor;

    let Ok(snapshot) =
        crate::daemon::persistence::agent_aggregate::AgentAggregateRepository::load_hosted_identity_snapshot()
    else {
        return Vec::new();
    };
    let projection = snapshot.host_descriptor_identity_projection();
    let Some(host_ura) = projection.host_device_ura() else {
        return Vec::new();
    };
    if AbilityDescriptor::validate_owner_ura(host_ura).is_err() {
        return Vec::new();
    }
    let llm_uras: Vec<(String, String)> = projection
        .llm_agent_uras()
        .iter()
        .filter(|(_, agent_ura)| AbilityDescriptor::validate_owner_ura(agent_ura).is_ok())
        .cloned()
        .collect();
    all_descriptors_for_host(host_ura, &llm_uras)
}

/// Aggregate every profile's descriptors into one list, anchored to
/// the same host's URAs. Used by P4.6's `federation.advertise_abilities`
/// publisher: the daemon advertises the union of every profile it hosts.
///
/// `device_ura` is the canonical host Device URA that sponsors daemon-native
/// SystemAgents. Hosted User Agent URAs are looked up from
/// `local-agents.json` (P3.4 / P4.7).
pub fn all_descriptors_for_host(
    device_ura: &str,
    llm_uras: &[(String, String)], // (sub_agent_name, ura)
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    let llm_catalog = if llm_uras.is_empty() {
        None
    } else {
        Some(llm::LlmProfileAbilityCatalog::load())
    };
    let mut out = Vec::new();
    out.extend(device::descriptors_for(device_ura));
    out.extend(system_agent_descriptor_projections_for_device(device_ura));
    for (_name, ura) in llm_uras {
        let catalog = llm_catalog
            .as_ref()
            .expect("llm_catalog is present when llm_uras is non-empty");
        out.extend(llm::descriptors_for_with_catalog(ura, None, catalog));
    }
    out
}

fn system_agent_descriptor_projections_for_device(
    device_ura: &str,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    SYSTEM_AGENT_DESCRIPTOR_PROJECTIONS
        .iter()
        .flat_map(|projection| {
            system_agent_descriptors_for_device(
                device_ura,
                projection.system_agent_id,
                (projection.owner)(),
            )
        })
        .collect()
}

fn system_agent_descriptors_for_device(
    device_ura: &str,
    system_agent_id: &str,
    owner: crate::daemon::ability::dispatch::OwnerKind,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    let Ok(device) = crate::core::ura::parse_ura(device_ura) else {
        return Vec::new();
    };
    if device.kind != crate::core::ura::URAKind::Device {
        return Vec::new();
    }
    let Some(device_id) = device.device_id() else {
        return Vec::new();
    };
    let system_agent_ura =
        crate::core::ura::device_agent_ura(&device.realm, device_id, system_agent_id);
    system_descriptors_for_owner(&system_agent_ura, owner, |_| {
        crate::daemon::ability::descriptors::Visibility::Scoped
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregator_with_only_device_returns_device_descriptors_only() {
        // URA v4.1.5 §A.URA-1: device URAs use the `device` role
        // (not the legacy v1 `agent/01DEV` placeholder). Production
        // mints this via `crate::core::ura::device_ura` (start.rs:623).
        let device_ura = "easynet:///r/acme/device/4065c47a-ec6f-4330-87a5-0d69787709b8";
        let all = all_descriptors_for_host(device_ura, &[]);
        assert!(!all.is_empty());
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::device_control::FS_READ
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.locomotion"
        }));
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::agents::AGENT_LIST
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.agent-management"
        }));
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::device_control::TERMINAL_LIST
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.terminal"
        }));
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::device_control::SESSION_LIST
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.session"
        }));
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::device_control::NODE_DESCRIBE
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.node-management"
        }));
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::automation::MISSION_RUN
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.automation"
        }));
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::governance::AUTHORITY_BINDING_LIST
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.runtime-governance"
        }));
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::governance::OBSERVE_HEALTH
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.runtime-health"
        }));
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::governance::META_LIST_ABILITIES
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.runtime-introspection"
        }));
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::governance::META_ACQUIRE
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.descriptor-transfer"
        }));
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::federation::ABILITY_DEPLOY
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.ability-management"
        }));
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::integrations::OPENAI_CHAT_COMPLETIONS
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.openai-compat"
        }));
        assert!(all.iter().any(|d| {
            d.name == crate::daemon::ability::names::integrations::A2A_BRIDGE_LIST_SKILLS
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.a2a-integration"
        }));
        assert!(all.iter().any(|d| {
            d.name == "device.keyring.list"
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.keyring-management"
        }));
        assert!(all.iter().any(|d| {
            d.name == "consent.subscribe"
                && d.owner_ura
                    == "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.consent-management"
        }));
    }

    #[test]
    fn aggregator_includes_each_provided_profile_owner() {
        // We only assert profiles whose namespace is reliably present
        // in the default `build_registry()` (which skips per-agent
        // chat handlers because no AgentRegistry is loaded). Device
        // and daemon-native SystemAgents are guaranteed; llm/mcp depend on
        // optional sub-systems and are exercised in their own tests.
        // URA v4.1.5: the Device remains the host/custody substrate. Public
        // device-native descriptors are owned by device-sponsored SystemAgent
        // URAs; hosted-user agents (mcp / llm) use the user-anchored
        // `agent/<user-uuid>.<agent-id>` shape per §A.URA-5.
        let device_ura = "easynet:///r/acme/device/4065c47a-ec6f-4330-87a5-0d69787709b8";
        let descriptor_transfer_ura = "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.descriptor-transfer";
        let consent_system_agent_ura = "easynet:///r/acme/agent/device.4065c47a-ec6f-4330-87a5-0d69787709b8.consent-management";
        let all = all_descriptors_for_host(device_ura, &[]);
        let owners: std::collections::HashSet<&str> =
            all.iter().map(|d| d.owner_ura.as_str()).collect();
        assert!(!owners.contains(device_ura));
        assert!(owners.contains(descriptor_transfer_ura));
        assert!(owners.contains(consent_system_agent_ura));
    }
}
