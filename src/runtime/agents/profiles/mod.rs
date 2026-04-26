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
//!   device   — fleet.*, observe.*, admin.*, meta.*, schedule.*,
//!              loop.*, discuss.* (host-resident operational abilities)
//!   consent  — consent.* (human-in-the-loop approval flow)
//!   policy   — policy.* (admission policy evaluation)
//!   mcp      — mcp.bridge.* + mcp.client.* (edge MCP adapter, single
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
pub mod device;
pub mod consent;
pub mod policy;
pub mod mcp;
pub mod llm;

/// Aggregate every profile's descriptors into one list, anchored to
/// the same host's URAs. Used by P4.6's `federation.advertise_abilities`
/// publisher: the daemon advertises the union of every profile it hosts.
///
/// `device_uri` is the host device-profile Agent URA. The other URAs
/// are looked up from `local-agents.json` (P3.4 / P4.7).
pub fn all_descriptors_for_host(
    device_uri: &str,
    consent_uri: Option<&str>,
    policy_uri: Option<&str>,
    mcp_uri: Option<&str>,
    llm_uris: &[(String, String)], // (sub_agent_name, ura)
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    let mut out = Vec::new();
    out.extend(device::descriptors_for(device_uri));
    if let Some(uri) = consent_uri {
        out.extend(consent::descriptors_for(uri));
    }
    if let Some(uri) = policy_uri {
        out.extend(policy::descriptors_for(uri));
    }
    if let Some(uri) = mcp_uri {
        out.extend(mcp::descriptors_for(uri));
    }
    for (_name, uri) in llm_uris {
        out.extend(llm::descriptors_for(uri));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregator_with_only_device_returns_device_descriptors_only() {
        let device_uri = "easynet:///r/acme/agent/01DEV";
        let all = all_descriptors_for_host(device_uri, None, None, None, &[]);
        assert!(!all.is_empty());
        for d in &all {
            assert_eq!(d.owner_agent_uri, device_uri);
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
        let device_uri = "easynet:///r/acme/agent/01DEV";
        let consent_uri = "easynet:///r/acme/agent/01CON";
        let all = all_descriptors_for_host(
            device_uri,
            Some(consent_uri),
            None,
            None,
            &[],
        );
        let owners: std::collections::HashSet<&str> =
            all.iter().map(|d| d.owner_agent_uri.as_str()).collect();
        assert!(owners.contains(device_uri));
        assert!(owners.contains(consent_uri));
    }
}
