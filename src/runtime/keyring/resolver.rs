// EasyNet CLI — KeyResolver implementations (RFC-002 §4)
// =======================================================
//
// LocalKeyringResolver  — resolves agent_ura → VerifyingKey from local
//                         keyring entries (bound_subject match).
// PeerKeyringResolver   — resolves from the peer_table (TOFU public
//                         keys recorded for federation peers).
// ChainResolver         — tries each resolver in order; first hit wins.
//
// FederationDirectoryResolver (calls federation.resolve_key on a hub)
// will land in P6 alongside the hub-side ability.
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use ed25519_dalek::VerifyingKey;
use std::sync::Arc;

use super::handle::KeyringHandle;
use crate::ura::{parse_ura, URAKind};

#[derive(Debug, thiserror::Error)]
pub enum KeyResolveError {
    #[error("agent_ura not known to this resolver")]
    Unknown,
    #[error("public key recorded for {agent_ura} is malformed: {reason}")]
    Malformed { agent_ura: String, reason: String },
}

pub trait KeyResolver: Send + Sync {
    fn resolve(&self, agent_ura: &str) -> std::result::Result<VerifyingKey, KeyResolveError>;
}

fn decode_pk(agent_ura: &str, b64: &str) -> std::result::Result<VerifyingKey, KeyResolveError> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    let bytes = STANDARD
        .decode(b64)
        .map_err(|e| KeyResolveError::Malformed {
            agent_ura: agent_ura.to_string(),
            reason: format!("base64: {e}"),
        })?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| KeyResolveError::Malformed {
            agent_ura: agent_ura.to_string(),
            reason: format!("public key length {} != 32", bytes.len()),
        })?;
    VerifyingKey::from_bytes(&arr).map_err(|e| KeyResolveError::Malformed {
        agent_ura: agent_ura.to_string(),
        reason: format!("invalid ed25519 point: {e}"),
    })
}

pub struct LocalKeyringResolver {
    keyring: Arc<KeyringHandle>,
}

impl LocalKeyringResolver {
    pub fn new(keyring: Arc<KeyringHandle>) -> Self {
        Self { keyring }
    }
}

impl KeyResolver for LocalKeyringResolver {
    fn resolve(&self, agent_ura: &str) -> std::result::Result<VerifyingKey, KeyResolveError> {
        let entry = self
            .keyring
            .find_active_entry_by_subject(agent_ura)
            .ok_or(KeyResolveError::Unknown)?;
        decode_pk(agent_ura, &entry.public_key_b64)
    }
}

pub struct PeerKeyringResolver {
    keyring: Arc<KeyringHandle>,
}

impl PeerKeyringResolver {
    pub fn new(keyring: Arc<KeyringHandle>) -> Self {
        Self { keyring }
    }
}

impl KeyResolver for PeerKeyringResolver {
    fn resolve(&self, agent_ura: &str) -> std::result::Result<VerifyingKey, KeyResolveError> {
        let peer = self
            .keyring
            .find_peer_by_uri(agent_ura)
            .ok_or(KeyResolveError::Unknown)?;
        if peer.status != super::store::PeerStatus::Trusted {
            return Err(KeyResolveError::Unknown);
        }
        decode_pk(agent_ura, &peer.public_key_b64)
    }
}

pub struct ChainResolver {
    resolvers: Vec<Box<dyn KeyResolver>>,
}

impl ChainResolver {
    pub fn new(resolvers: Vec<Box<dyn KeyResolver>>) -> Self {
        Self { resolvers }
    }
}

impl KeyResolver for ChainResolver {
    fn resolve(&self, agent_ura: &str) -> std::result::Result<VerifyingKey, KeyResolveError> {
        for r in &self.resolvers {
            match r.resolve(agent_ura) {
                Ok(k) => return Ok(k),
                Err(KeyResolveError::Unknown) => continue,
                Err(e) => return Err(e),
            }
        }
        Err(KeyResolveError::Unknown)
    }
}

/// Convenience constructor for the daemon's default resolver chain:
/// local entries first, then peer table.
pub fn default_chain(keyring: Arc<KeyringHandle>) -> ChainResolver {
    ChainResolver::new(vec![
        Box::new(LocalKeyringResolver::new(keyring.clone())),
        Box::new(PeerKeyringResolver::new(keyring)),
    ])
}

// ── FederatedUserResolver (PR-N4 commit 4/N) ───────────────────────

/// Resolver for "is this cross-realm URI bound to a known local
/// user?" — distinct from the `KeyResolver` trait above (which
/// answers "what is this URI's verifying key").
///
/// **PR-N4 spec §commit 4/N**. Wraps the local keyring + the
/// `FederatedBindingsStore` from commit 3/N in a chain:
///
///   1. **local-first**: if `agent_ura`'s realm matches
///      `local_realm`, the URI's user-id is the URI itself —
///      no federated lookup needed (and INV-3 says the user
///      always speaks for themselves on their home realm).
///   2. **federated fallback**: otherwise, look up
///      `(parsed_realm, agent_ura)` in the bindings store.
///      Some(local_user_id) ⇒ the cross-realm user has been
///      consumed-bound; None ⇒ the URI belongs to a federated
///      realm but the user has not opted into binding (INV-5
///      privacy default).
///
/// Used by `<self>.discover` Tier-3 to filter cross-realm
/// directory entries: only show devices whose URI's realm has
/// a binding for the calling user.
pub struct FederatedUserResolver {
    local_realm: String,
    bindings: Arc<super::federated_bindings::FederatedBindingsStore>,
}

/// Outcome of `FederatedUserResolver::resolve_user`. A typed
/// enum rather than `Option<String>` so the caller can
/// distinguish "URI is local" (== return verbatim) from
/// "URI is bound to local_user_id" from "URI is from a
/// federated realm we have no binding for".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FederatedUserOutcome {
    /// URI's realm == `local_realm`; the URI is the user-id.
    Local,
    /// URI is in a federated realm with a known binding;
    /// `local_user_id` is who they map to here.
    BoundLocalUser(String),
    /// URI's realm is not the local realm and no binding
    /// exists. Caller filters this URI out of cross-realm
    /// surfaces (INV-5 privacy default).
    NotBound,
    /// URI did not parse as a canonical EasyNet URI.
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

    /// Resolve a URI to a federated-binding outcome.
    #[must_use]
    pub fn resolve_user(&self, user_ura: &str) -> FederatedUserOutcome {
        let Some(realm) = parse_realm_from_user_ura(user_ura) else {
            return FederatedUserOutcome::Malformed;
        };
        if realm == self.local_realm {
            return FederatedUserOutcome::Local;
        }
        match self.bindings.find_local_user(&realm, user_ura) {
            Some(local) => FederatedUserOutcome::BoundLocalUser(local),
            None => FederatedUserOutcome::NotBound,
        }
    }
}

/// Parse the realm slice from a canonical EasyNet user URI
/// (`easynet:///r/<realm>/user/<id>`). Mirrors
/// `runtime::keyring::abilities::parse_realm_from_user_ura` —
/// duplicated rather than re-exported to keep the resolver
/// layer free of any cross-module imports beyond
/// `super::federated_bindings`.
fn parse_realm_from_user_ura(uri: &str) -> Option<String> {
    let parsed = parse_ura(uri).ok()?;
    (parsed.kind == URAKind::User).then_some(parsed.realm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn handle() -> (Arc<KeyringHandle>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keyring.json");
        let h = Arc::new(KeyringHandle::open_or_create(path, "p").unwrap());
        (h, dir)
    }

    #[test]
    fn local_resolver_finds_bound_entry() {
        let (h, _dir) = handle();
        let subject = "easynet:///r/test.local/agent/u.foo".to_string();
        h.create_entry("agent_signing", Some(subject.clone()))
            .unwrap();
        let r = LocalKeyringResolver::new(h);
        assert!(r.resolve(&subject).is_ok());
        assert!(matches!(
            r.resolve("easynet:///r/test.local/agent/u.unknown")
                .unwrap_err(),
            KeyResolveError::Unknown
        ));
    }

    #[test]
    fn peer_resolver_finds_trusted_peer() {
        let (h, _dir) = handle();
        let entry = h.create_entry("agent_signing", None).unwrap();
        h.peer_add(
            "easynet:///r/test.local/agent/u.alice",
            &entry.public_key_b64,
            None,
            None,
        )
        .unwrap();
        let r = PeerKeyringResolver::new(h);
        assert!(r.resolve("easynet:///r/test.local/agent/u.alice").is_ok());
        assert!(matches!(
            r.resolve("easynet:///r/test.local/agent/u.bob")
                .unwrap_err(),
            KeyResolveError::Unknown
        ));
    }

    #[test]
    fn chain_falls_through_on_unknown() {
        let (h, _dir) = handle();
        let entry = h.create_entry("agent_signing", None).unwrap();
        h.peer_add(
            "easynet:///r/test.local/agent/u.alice",
            &entry.public_key_b64,
            None,
            None,
        )
        .unwrap();
        let chain = default_chain(h);
        assert!(chain
            .resolve("easynet:///r/test.local/agent/u.alice")
            .is_ok());
        assert!(matches!(
            chain
                .resolve("easynet:///r/test.local/agent/u.unknown")
                .unwrap_err(),
            KeyResolveError::Unknown
        ));
    }

    // ── PR-N4 commit 4/N — FederatedUserResolver ────────────

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
    fn federated_user_resolver_malformed_uri_returns_malformed() {
        let bindings = Arc::new(FederatedBindingsStore::in_memory());
        let resolver = FederatedUserResolver::new("realm-b", bindings);
        let outcome = resolver.resolve_user("not-a-canonical-uri");
        assert_eq!(outcome, FederatedUserOutcome::Malformed);
    }

    #[test]
    fn federated_user_resolver_binding_for_other_user_does_not_match() {
        // Binding exists for one user URI; querying a different
        // URI in the same realm must NOT match (the resolver
        // keys on full URI, not just realm).
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
            "different URI in same realm must not match an existing binding"
        );
    }
}
