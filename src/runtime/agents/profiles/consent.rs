//! consent profile — RFC-001 §1.
//!
//! An Agent advertising consent.* abilities. Replaces the old
//! permission_broker side-channel (formerly registered under a
//! retired pre-RFC-001 namespace).
//! Per RFC §A6: human-in-the-loop approval flow goes through this
//! Agent's abilities, never through a side-channel.
//!
//! Owned ability namespaces
//! ------------------------
//!   consent.request   (kernel admission gate sub-invocation)
//!   consent.subscribe (UI clients tail pending requests via InvokeStream)
//!   consent.decide    (UI clients deliver decisions)
//!   consent.list_pending (snapshot query)
//!   consent.grant / consent.revoke / consent.list_grants
//!     (long-lived consent grants per restatement-mapping §2 capability.proto)
//!
//! Currently wired in agents/permission_ability.rs (which renames to
//! consent_ability.rs in a follow-up cleanup; for now the file name
//! is legacy but the registered ability strings are the new
//! `consent.*` per P2.2).

/// Standard ability-name prefixes the consent profile owns.
pub const CONSENT_PROFILE_ABILITY_PREFIXES: &[&str] = &["consent."];

pub fn owns(ability_name: &str) -> bool {
    CONSENT_PROFILE_ABILITY_PREFIXES
        .iter()
        .any(|p| ability_name.starts_with(p))
}

/// AbilityDescriptors for every consent.* in the live registry,
/// anchored to the consent-profile's canonical URA. Per RFC §18,
/// every consent.* defaults to SCOPED — the kernel is the only
/// expected consumer of `consent.request`, and UI clients are
/// the only expected consumers of `consent.subscribe` / `decide`.
/// P4.7 narrows scope_subjects/scope_agents.
pub fn descriptors_for(
    owner_agent_uri: &str,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};
    crate::runtime::agents::published_abilities()
        .into_iter()
        .filter(|m| owns(&m.name))
        .map(|m| {
            AbilityDescriptor::new(m.name.clone(), owner_agent_uri, Visibility::Scoped)
                .expect("registry-derived names satisfy descriptor invariants")
                .with_input_schema(m.input_schema.clone())
                .with_source("kernel:built-in")
                .with_description(m.description)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owns_recognizes_consent_namespace() {
        assert!(owns("consent.request"));
        assert!(owns("consent.subscribe"));
        assert!(owns("consent.decide"));
        assert!(owns("consent.list_pending"));
        assert!(owns("consent.grant"));
    }

    #[test]
    fn owns_rejects_other_profiles() {
        assert!(!owns("policy.evaluate"));
        assert!(!owns("fleet.list_abilities"));
    }
}
