//! consent profile — RFC-001 §1.
//!
//! An Agent advertising consent.* abilities. Replaces the old
//! permission_broker side-channel (formerly registered under a
//! retired pre-RFC-001 namespace).
//! Per RFC §A6: human-in-the-loop approval flow goes through this
//! Agent's abilities, never through a side-channel.
//!
//! Descriptor projection
//! ---------------------
//! Consent descriptors are generated from the dispatch registry entries whose
//! projection class is `OwnerKind::Agent(DEFAULT_CONSENT_AGENT_ID)`. This file
//! does not infer ownership from ability name prefixes.
//!
//! Currently wired in agents/permission_ability.rs (which renames to
//! consent_ability.rs in a follow-up cleanup; for now the file name
//! is legacy but the registered ability strings are the new
//! `consent.*` per P2.2).

/// AbilityDescriptors for every consent ability in the system registry,
/// anchored to the consent-profile's canonical URA. Per RFC §18,
/// every consent.* defaults to SCOPED — the kernel is the only
/// expected consumer of `consent.request`, and UI clients are
/// the only expected consumers of `consent.subscribe` / `decide`.
/// P4.7 narrows scope_subjects/scope_agents.
pub fn descriptors_for(
    owner_ura: &str,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    use crate::daemon::ability::descriptors::Visibility;
    use crate::daemon::ability::dispatch::OwnerKind;

    super::system_descriptors_for_owner(
        owner_ura,
        OwnerKind::Agent(super::DEFAULT_CONSENT_AGENT_ID.to_string()),
        |_| Visibility::Scoped,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_follow_registry_owner() {
        let descriptors = descriptors_for("easynet:///r/acme/agent/u1.01CON");
        let names = descriptors
            .iter()
            .map(|d| d.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(names.contains("consent.subscribe"));
        assert!(names.contains("consent.decide"));
        assert!(names.contains("consent.list_pending"));
        assert!(!names.contains("skill.list"));
    }
}
