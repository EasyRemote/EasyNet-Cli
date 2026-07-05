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
//!   device   — daemon-local host abilities advertised by the device-profile
//!              Agent under device authority: fs.*, process.*, shell.*,
//!              terminal.*, session.*, browser/media/voice, skill management,
//!              device.*, admin.*, meta.*, schedule.*, loop.*, discuss.*
//!   consent  — consent.* (human-in-the-loop approval flow)
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
pub mod consent;
pub mod device;
pub mod llm;
pub mod mcp;

/// Hosted Agent id for the default consent profile.
pub const DEFAULT_CONSENT_AGENT_ID: &str = "consent-default";
/// Hosted Agent id for the default MCP profile.
pub const DEFAULT_MCP_AGENT_ID: &str = "mcp-default";

fn system_descriptors_for_owner(
    owner_ura: &str,
    owner: crate::daemon::ability::dispatch::OwnerKind,
    visibility_for: impl Fn(&str) -> crate::daemon::ability::descriptors::Visibility,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    use crate::daemon::ability::descriptors::AbilityDescriptor;

    crate::daemon::ability::catalog::published_system_abilities_for_owner(owner)
        .into_iter()
        .map(|m| {
            AbilityDescriptor::new(m.name.clone(), owner_ura, visibility_for(&m.name))
                .expect("registry-derived names satisfy descriptor invariants")
                .with_input_schema(m.input_schema.clone())
                .with_hints(m.hints.clone())
                .with_source("kernel:built-in")
                .with_description(m.description)
        })
        .collect()
}

/// Read `~/.easynet/local-agents.json` and project it into the full
/// host descriptor catalog. Returns an empty catalog when the file is
/// missing or malformed (pre-join state, brand-new install) — callers
/// that need a strict error path should call `local_agents::load`
/// directly. Pre-join, descriptors anchor on a literal "self" URA
/// marker so the catalog is still well-formed; once join completes
/// and a daemon restart picks up the canonical URA, the catalog
/// re-anchors automatically on the next call.
///
/// Single source of truth for the recipe used by both the MCP stdio
/// server (advertised tool surface) and the in-process
/// `mcp.bridge.list_tools` ability handler — keeping them on one
/// helper is what guarantees external and internal MCP callers see
/// the same catalog.
pub fn load_host_descriptors() -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    let local = crate::daemon::persistence::local_agents::load().unwrap_or_default();
    let host_ura = if local.host_device_agent_ura.is_empty() {
        "self".to_string()
    } else {
        local.host_device_agent_ura.clone()
    };
    let consent_ura =
        crate::daemon::persistence::local_agents::lookup_hosted_ura(&local, "consent", "default");
    let mcp_ura =
        crate::daemon::persistence::local_agents::lookup_hosted_ura(&local, "mcp", "default");
    let llm_uras: Vec<(String, String)> = local
        .hosted_agents
        .iter()
        .filter(|e| e.profile == "llm")
        .map(|e| (e.name.clone(), e.agent_ura.clone()))
        .collect();
    all_descriptors_for_host(
        &host_ura,
        consent_ura.as_deref(),
        mcp_ura.as_deref(),
        &llm_uras,
    )
}

/// Aggregate every profile's descriptors into one list, anchored to
/// the same host's URAs. Used by P4.6's `federation.advertise_abilities`
/// publisher: the daemon advertises the union of every profile it hosts.
///
/// `device_ura` is the host device-profile Agent URA, not a raw hardware
/// locator. The other URAs are looked up from `local-agents.json`
/// (P3.4 / P4.7).
pub fn all_descriptors_for_host(
    device_ura: &str,
    consent_ura: Option<&str>,
    mcp_ura: Option<&str>,
    llm_uras: &[(String, String)], // (sub_agent_name, ura)
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    let llm_catalog = if llm_uras.is_empty() {
        None
    } else {
        Some(llm::LlmProfileAbilityCatalog::load())
    };
    let mut out = Vec::new();
    out.extend(device::descriptors_for(device_ura));
    if let Some(ura) = consent_ura {
        out.extend(consent::descriptors_for(ura));
    }
    if let Some(ura) = mcp_ura {
        out.extend(mcp::descriptors_for(ura));
    }
    for (_name, ura) in llm_uras {
        let catalog = llm_catalog
            .as_ref()
            .expect("llm_catalog is present when llm_uras is non-empty");
        out.extend(llm::descriptors_for_with_catalog(ura, None, catalog));
    }
    out
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
        let all = all_descriptors_for_host(device_ura, None, None, &[]);
        assert!(!all.is_empty());
        for d in &all {
            assert_eq!(d.owner_ura, device_ura);
        }
        assert!(all.iter().any(|d| d.name == "fs.read"));
        assert!(all.iter().all(|d| d.name != "consent.subscribe"));
    }

    #[test]
    fn aggregator_includes_each_provided_profile_owner() {
        // We only assert profiles whose namespace is reliably present
        // in the default `build_registry()` (which skips per-agent
        // chat handlers because no AgentRegistry is loaded). Device
        // and consent are guaranteed; llm/mcp depend on
        // optional sub-systems and are exercised in their own tests.
        // URA v4.1.5: device-profile is anchored on the `device`
        // role (built-ins governed by device authority); hosted-user agents
        // (consent / mcp / llm) use the user-anchored
        // `agent/<user-uuid>.<agent-id>` shape per §A.URA-5.
        let device_ura = "easynet:///r/acme/device/4065c47a-ec6f-4330-87a5-0d69787709b8";
        let consent_ura = "easynet:///r/acme/agent/00000000-0000-0000-0000-000000000001.consent";
        let all = all_descriptors_for_host(device_ura, Some(consent_ura), None, &[]);
        let owners: std::collections::HashSet<&str> =
            all.iter().map(|d| d.owner_ura.as_str()).collect();
        assert!(owners.contains(device_ura));
        assert!(owners.contains(consent_ura));
    }
}
