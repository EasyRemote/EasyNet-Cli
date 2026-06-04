//! Agent profile registry — RFC-001 §1 implementation profiles.
//!
//! Per AXON-RFC-001 plan v4.1.2 §A4: "profile" is documentation
//! shorthand for "an Agent advertising the corresponding ability
//! namespace". These are NOT protocol-level types or kind values.
//! They are implementation modules that group ability handlers by
//! the Agent that hosts them.
//!
//! Registered profiles
//! -------------------
//!   device   — device.*, admin.*, meta.*, schedule.*,
//!              loop.*, discuss.* (host-resident operational abilities)
//!   consent  — consent.* (human-in-the-loop approval flow)
//!   policy   — policy.* (admission policy evaluation)
//!   mcp      — device.mcp.bridge.* + device.mcp.client.* (edge MCP adapter, single
//!              Agent owns both inbound and outbound per RFC §1 [P6])
//!   llm      — conversation.*, session.*, meta.* per LLM sub-agent
//!              (claude / codex / etc.)
//!
//! The handlers themselves live in the per-feature files (chat_ability.rs,
//! session_ability.rs, etc.) at the parent agents/ module. The profile
//! files here only declare WHICH abilities each profile owns; the actual
//! `register_*` functions are imported from the feature modules.
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
pub mod policy;

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
/// `device.mcp.bridge.list_tools` ability handler — keeping them on one
/// helper is what guarantees external and internal MCP callers see
/// the same catalog.
pub fn load_host_descriptors() -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    let local = crate::persistence::local_agents::load().unwrap_or_default();
    let host_ura = if local.host_device_agent_ura.is_empty() {
        "self".to_string()
    } else {
        local.host_device_agent_ura.clone()
    };
    let consent_ura =
        crate::persistence::local_agents::lookup_hosted_ura(&local, "consent", "default");
    let policy_ura =
        crate::persistence::local_agents::lookup_hosted_ura(&local, "policy", "default");
    let mcp_uri = crate::persistence::local_agents::lookup_hosted_ura(&local, "mcp", "default");
    let llm_uras: Vec<(String, String)> = local
        .hosted_agents
        .iter()
        .filter(|e| e.profile == "llm")
        .map(|e| (e.name.clone(), e.agent_ura.clone()))
        .collect();
    all_descriptors_for_host(
        &host_ura,
        consent_ura.as_deref(),
        policy_ura.as_deref(),
        mcp_uri.as_deref(),
        &llm_uras,
    )
}

/// Aggregate every profile's descriptors into one list, anchored to
/// the same host's URAs. Used by P4.6's `federation.advertise_abilities`
/// publisher: the daemon advertises the union of every profile it hosts.
///
/// `device_ura` is the host device-profile Agent URA. The other URAs
/// are looked up from `local-agents.json` (P3.4 / P4.7).
pub fn all_descriptors_for_host(
    device_ura: &str,
    consent_ura: Option<&str>,
    policy_ura: Option<&str>,
    mcp_uri: Option<&str>,
    llm_uras: &[(String, String)], // (sub_agent_name, ura)
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    let mut out = Vec::new();
    out.extend(device::descriptors_for(device_ura));
    if let Some(ura) = consent_ura {
        out.extend(consent::descriptors_for(ura));
    }
    if let Some(ura) = policy_ura {
        out.extend(policy::descriptors_for(ura));
    }
    if let Some(ura) = mcp_uri {
        out.extend(mcp::descriptors_for(ura));
    }
    for (_name, ura) in llm_uras {
        out.extend(llm::descriptors_for(ura));
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
        // mints this via `crate::ura::device_ura` (start.rs:623).
        let device_ura = "easynet:///r/acme/device/4065c47a-ec6f-4330-87a5-0d69787709b8";
        let all = all_descriptors_for_host(device_ura, None, None, None, &[]);
        assert!(!all.is_empty());
        for d in &all {
            assert_eq!(d.owner_agent_ura, device_ura);
            assert!(device::owns(&d.name));
        }
    }

    #[test]
    fn aggregator_includes_each_provided_profile_owner() {
        // We only assert profiles whose namespace is reliably present
        // in the default `build_registry()` (which skips per-agent
        // chat handlers because no AgentRegistry is loaded). Device
        // and consent are guaranteed; llm/policy/mcp depend on
        // optional sub-systems and are exercised in their own tests.
        // URA v4.1.5: device-profile is anchored on the `device`
        // role (built-ins owned by the device); hosted-user agents
        // (consent / policy / mcp / llm) use the user-anchored
        // `agent/<user-uuid>.<agent-id>` shape per §A.URA-5.
        let device_ura = "easynet:///r/acme/device/4065c47a-ec6f-4330-87a5-0d69787709b8";
        let consent_ura = "easynet:///r/acme/agent/00000000-0000-0000-0000-000000000001.consent";
        let all = all_descriptors_for_host(device_ura, Some(consent_ura), None, None, &[]);
        let owners: std::collections::HashSet<&str> =
            all.iter().map(|d| d.owner_agent_ura.as_str()).collect();
        assert!(owners.contains(device_ura));
        assert!(owners.contains(consent_ura));
    }
}
