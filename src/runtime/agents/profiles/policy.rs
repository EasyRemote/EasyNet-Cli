//! policy profile — RFC-001 §1.
//!
//! An Agent advertising policy.* abilities. The kernel admission
//! gate calls device.policy.evaluate as an in-process invocation; per RFC
//! §A6, that sub-invocation carries `admission_internal=true` and
//! is exempt from recursive policy admission.
//!
//! Owned ability namespaces
//! ------------------------
//!   device.policy.evaluate          (admission gate consumer)
//!   device.policy.simulate          (operator dry-run)
//!   device.policy.publish           (operator registers / updates a policy)
//!   device.policy.list / device.policy.get
//!   device.policy.create_override / device.policy.revoke_override
//!   device.policy.get_decision      (auditor)

pub const POLICY_PROFILE_ABILITY_PREFIXES: &[&str] = &["device.policy.", "policy."];

pub fn owns(ability_name: &str) -> bool {
    POLICY_PROFILE_ABILITY_PREFIXES
        .iter()
        .any(|p| ability_name.starts_with(p))
}

/// AbilityDescriptors for every policy.* in the live registry,
/// anchored to the policy-profile's canonical URA. All SCOPED per
/// §18 (only the admission gate calls device.policy.evaluate; only
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
                .with_hints(m.hints.clone())
                .with_source("kernel:built-in")
                .with_description(m.description)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owns_recognizes_policy_namespace() {
        assert!(owns("device.policy.evaluate"));
        assert!(owns("device.policy.simulate"));
        assert!(owns("device.policy.publish"));
        assert!(owns("device.policy.create_override"));
    }

    #[test]
    fn owns_rejects_other_profiles() {
        assert!(!owns("device.consent.request"));
        assert!(!owns("device.fleet.list_abilities"));
    }
}
