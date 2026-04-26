//! policy profile — RFC-001 §1.
//!
//! An Agent advertising policy.* abilities. The kernel admission
//! gate calls policy.evaluate as an in-process invocation; per RFC
//! §A6, that sub-invocation carries `admission_internal=true` and
//! is exempt from recursive policy admission.
//!
//! Owned ability namespaces
//! ------------------------
//!   policy.evaluate          (admission gate consumer)
//!   policy.simulate          (operator dry-run)
//!   policy.publish           (operator registers / updates a policy)
//!   policy.list / policy.get
//!   policy.create_override / policy.revoke_override
//!   policy.get_decision      (auditor)

pub const POLICY_PROFILE_ABILITY_PREFIXES: &[&str] = &["policy."];

pub fn owns(ability_name: &str) -> bool {
    POLICY_PROFILE_ABILITY_PREFIXES
        .iter()
        .any(|p| ability_name.starts_with(p))
}

/// AbilityDescriptors for every policy.* in the live registry,
/// anchored to the policy-profile's canonical URA. All SCOPED per
/// §18 (only the admission gate calls policy.evaluate; only
/// operators publish/list policies). P4.7 narrows scope axes.
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
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owns_recognizes_policy_namespace() {
        assert!(owns("policy.evaluate"));
        assert!(owns("policy.simulate"));
        assert!(owns("policy.publish"));
        assert!(owns("policy.create_override"));
    }

    #[test]
    fn owns_rejects_other_profiles() {
        assert!(!owns("consent.request"));
        assert!(!owns("fleet.list_abilities"));
    }
}
