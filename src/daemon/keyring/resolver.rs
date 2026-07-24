//! EasyNet CLI — federated user binding resolver
//! ===============================================
//!
//! File: src/daemon/keyring/resolver.rs
//! Description: Classify local and cross-realm user URAs against the daemon's
//! explicit federated-user binding store.
//!
//! Protocol Responsibility:
//! - Distinguish local users, explicitly bound federated users, unbound users,
//!   and malformed URAs for directory visibility policy.
//!
//! Implementation Approach:
//! - Parse the canonical user URA, compare its realm with the local realm, and
//!   consult `FederatedBindingsStore` only for cross-realm users.
//!
//! Usage Contract:
//! - Callers must preserve `Malformed` separately from privacy-default
//!   `NotBound` when producing audit or directory results.
//!
//! Architectural Position:
//! - Daemon federation policy. Cryptographic caller-key resolution belongs to
//!   the canonical admission `FederatedKeyResolver`, not this module.

use std::sync::Arc;

/// Resolver for "is this cross-realm URA bound to a known local user?"
///
/// This is a directory-visibility resolver, not a cryptographic key resolver.
/// Invocation admission resolves caller keys through its canonical
/// `FederatedKeyResolver` pipeline.
///
/// **PR-N4 spec §commit 4/N**. Applies the local realm rule before consulting
/// `FederatedBindingsStore`:
///
///   1. **local-first**: if `agent_ura`'s realm matches
///      `local_realm`, the URA's user-id is the URA itself —
///      no federated lookup needed (and INV-3 says the user
///      always speaks for themselves on their home realm).
///   2. **cross-realm binding lookup**: otherwise, look up
///      `(parsed_realm, agent_ura)` in the bindings store.
///      Some(local_user_id) ⇒ the cross-realm user has been
///      consumed-bound; None ⇒ the URA belongs to a federated
///      realm but the user has not opted into binding (INV-5
///      privacy default).
///
/// Used by `<agent>.discover` Tier-3 to filter cross-realm
/// directory entries: only show devices whose URA's realm has
/// a binding for the calling user.
pub struct FederatedUserResolver {
    local_realm: String,
    bindings: Arc<super::federated_bindings::FederatedBindingsStore>,
}
/// Outcome of `FederatedUserResolver::resolve_user`. A typed
/// enum rather than `Option<String>` so the caller can
/// distinguish "URA is local" (== return verbatim) from
/// "URA is bound to local_user_id" from "URA is from a
/// federated realm we have no binding for".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FederatedUserOutcome {
    /// URA's realm == `local_realm`; the URA is the user-id.
    Local,
    /// URA is in a federated realm with a known binding;
    /// `local_user_id` is who they map to here.
    BoundLocalUser(String),
    /// URA's realm is not the local realm and no binding
    /// exists. Caller filters this URA out of cross-realm
    /// surfaces (INV-5 privacy default).
    NotBound,
    /// URA did not parse as a canonical EasyNet URA.
    /// Distinct from `NotBound` so audit can flag malformed
    /// inputs separately.
    Malformed,
}

impl FederatedUserResolver {
    #[must_use]
    pub fn new(
        local_realm: impl Into<String>,
        bindings: Arc<super::federated_bindings::FederatedBindingsStore>,
    ) -> Self {
        Self {
            local_realm: local_realm.into(),
            bindings,
        }
    }

    /// Resolve a URA to a federated-binding outcome.
    #[must_use]
    pub fn resolve_user(&self, user_ura: &str) -> FederatedUserOutcome {
        let Ok(identity) = crate::core::identity::RuntimeIdentityUra::parse(user_ura) else {
            return FederatedUserOutcome::Malformed;
        };
        if identity.kind() != crate::core::ura::URAKind::User {
            return FederatedUserOutcome::Malformed;
        }
        let realm = identity.realm().to_string();
        if realm == self.local_realm {
            return FederatedUserOutcome::Local;
        }
        match self.bindings.find_local_user(&realm, user_ura) {
            Some(local) => FederatedUserOutcome::BoundLocalUser(local),
            None => FederatedUserOutcome::NotBound,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use super::super::federated_bindings::{FederatedBindingsStore, FederatedUserBinding};

    #[test]
    fn federated_user_resolver_local_realm_returns_local() {
        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        let resolver = FederatedUserResolver::new("realm-b", bindings);
        let outcome = resolver.resolve_user("easynet:///r/realm-b/user/user-on-b");
        assert_eq!(outcome, FederatedUserOutcome::Local);
    }

    #[test]
    fn federated_user_resolver_cross_realm_with_binding_returns_local_user() {
        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        bindings
            .record_binding(
                FederatedUserBinding {
                    source_realm: "realm-a".to_string(),
                    source_user_ura: "easynet:///r/realm-a/user/user-c".to_string(),
                    source_user_pubkey_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
                        .to_string(),
                    local_user_id: "user-c-on-realm-b".to_string(),
                    bound_at_unix_ms: 1_714_500_000_000,
                },
                "n".to_string(),
            )
            .unwrap();
        let resolver = FederatedUserResolver::new("realm-b", bindings);
        let outcome = resolver.resolve_user("easynet:///r/realm-a/user/user-c");
        assert_eq!(
            outcome,
            FederatedUserOutcome::BoundLocalUser("user-c-on-realm-b".to_string())
        );
    }

    #[test]
    fn federated_user_resolver_cross_realm_without_binding_returns_not_bound() {
        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        let resolver = FederatedUserResolver::new("realm-b", bindings);
        let outcome = resolver.resolve_user("easynet:///r/realm-c/user/no-binding");
        assert_eq!(outcome, FederatedUserOutcome::NotBound);
    }

    #[test]
    fn federated_user_resolver_malformed_ura_returns_malformed() {
        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        let resolver = FederatedUserResolver::new("realm-b", bindings);
        let outcome = resolver.resolve_user("not-a-canonical-ura");
        assert_eq!(outcome, FederatedUserOutcome::Malformed);
    }

    #[test]
    fn federated_user_resolver_all_zero_user_returns_malformed() {
        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        let resolver = FederatedUserResolver::new("realm-b", bindings);
        let outcome =
            resolver.resolve_user("easynet:///r/realm-b/user/00000000-0000-0000-0000-000000000000");
        assert_eq!(outcome, FederatedUserOutcome::Malformed);
    }

    #[test]
    fn federated_user_resolver_binding_for_other_user_does_not_match() {
        // Binding exists for one user URA; querying a different
        // URA in the same realm must NOT match (the resolver
        // keys on full URA, not just realm).
        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        bindings
            .record_binding(
                FederatedUserBinding {
                    source_realm: "realm-a".to_string(),
                    source_user_ura: "easynet:///r/realm-a/user/user-c".to_string(),
                    source_user_pubkey_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
                        .to_string(),
                    local_user_id: "user-c-on-realm-b".to_string(),
                    bound_at_unix_ms: 1_714_500_000_000,
                },
                "n".to_string(),
            )
            .unwrap();
        let resolver = FederatedUserResolver::new("realm-b", bindings);
        let outcome = resolver.resolve_user("easynet:///r/realm-a/user/user-OTHER");
        assert_eq!(
            outcome,
            FederatedUserOutcome::NotBound,
            "different URA in same realm must not match an existing binding"
        );
    }
}
