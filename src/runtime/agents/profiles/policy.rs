//! policy profile — RFC-001 §1.
//!
//! An Agent advertising policy.* abilities. The kernel admission
//! gate calls policy.evaluate as an in-process invocation; per RFC
//! §A6, that sub-invocation carries `admission_internal=true` and
//! is exempt from recursive policy admission.
//!
//! Descriptor ownership
//! --------------------
//! Policy descriptors are generated from the dispatch registry entries whose
//! owner is `OwnerKind::Agent(DEFAULT_POLICY_AGENT_ID)`. This file does not
//! infer ownership from ability name prefixes.

/// AbilityDescriptors for every policy ability in the system registry,
/// anchored to the policy-profile's canonical URA. All SCOPED per
/// §18 (only the admission gate calls policy.evaluate; only
/// operators publish/list policies). P4.7 narrows scope axes.
pub fn descriptors_for(
    owner_ura: &str,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    use crate::runtime::ability_descriptor::Visibility;
    use crate::runtime::ability_dispatch::OwnerKind;

    super::system_descriptors_for_owner(
        owner_ura,
        OwnerKind::Agent(super::DEFAULT_POLICY_AGENT_ID.to_string()),
        |_| Visibility::Scoped,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_follow_registry_owner() {
        let descriptors = descriptors_for("easynet:///r/acme/agent/u1.01POL");
        let names = descriptors
            .iter()
            .map(|d| d.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert!(names.contains("policy.evaluate"));
        assert!(names.contains("policy.simulate"));
        assert!(!names.contains("consent.subscribe"));
        assert!(!names.contains("skill.list"));
    }
}
