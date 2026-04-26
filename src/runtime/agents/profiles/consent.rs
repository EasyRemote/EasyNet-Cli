//! consent profile — RFC-001 §1.
//!
//! An Agent advertising consent.* abilities. Replaces the old
//! permission_broker side-channel (which was system.permission.*).
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
