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
