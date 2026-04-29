// EasyNet CLI — KeyResolver implementations (RFC-002 §4)
// =======================================================
//
// LocalKeyringResolver  — resolves agent_uri → VerifyingKey from local
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

#[derive(Debug, thiserror::Error)]
pub enum KeyResolveError {
    #[error("agent_uri not known to this resolver")]
    Unknown,
    #[error("public key recorded for {agent_uri} is malformed: {reason}")]
    Malformed { agent_uri: String, reason: String },
}

pub trait KeyResolver: Send + Sync {
    fn resolve(&self, agent_uri: &str) -> std::result::Result<VerifyingKey, KeyResolveError>;
}

fn decode_pk(agent_uri: &str, b64: &str) -> std::result::Result<VerifyingKey, KeyResolveError> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    let bytes = STANDARD.decode(b64).map_err(|e| KeyResolveError::Malformed {
        agent_uri: agent_uri.to_string(),
        reason: format!("base64: {e}"),
    })?;
    let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| KeyResolveError::Malformed {
        agent_uri: agent_uri.to_string(),
        reason: format!("public key length {} != 32", bytes.len()),
    })?;
    VerifyingKey::from_bytes(&arr).map_err(|e| KeyResolveError::Malformed {
        agent_uri: agent_uri.to_string(),
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
    fn resolve(&self, agent_uri: &str) -> std::result::Result<VerifyingKey, KeyResolveError> {
        let entry = self
            .keyring
            .find_active_entry_by_subject(agent_uri)
            .ok_or(KeyResolveError::Unknown)?;
        decode_pk(agent_uri, &entry.public_key_b64)
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
    fn resolve(&self, agent_uri: &str) -> std::result::Result<VerifyingKey, KeyResolveError> {
        let peer = self
            .keyring
            .find_peer_by_uri(agent_uri)
            .ok_or(KeyResolveError::Unknown)?;
        if peer.status != super::store::PeerStatus::Trusted {
            return Err(KeyResolveError::Unknown);
        }
        decode_pk(agent_uri, &peer.public_key_b64)
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
    fn resolve(&self, agent_uri: &str) -> std::result::Result<VerifyingKey, KeyResolveError> {
        for r in &self.resolvers {
            match r.resolve(agent_uri) {
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
        let subject = "easynet:///r/prv/reg/agent.foo".to_string();
        h.create_entry("agent_signing", Some(subject.clone())).unwrap();
        let r = LocalKeyringResolver::new(h);
        assert!(r.resolve(&subject).is_ok());
        assert!(matches!(
            r.resolve("easynet:///r/prv/reg/agent.unknown").unwrap_err(),
            KeyResolveError::Unknown
        ));
    }

    #[test]
    fn peer_resolver_finds_trusted_peer() {
        let (h, _dir) = handle();
        let entry = h.create_entry("agent_signing", None).unwrap();
        h.peer_add(
            "easynet:///r/org/reg/agent.alice",
            &entry.public_key_b64,
            None,
            None,
        )
        .unwrap();
        let r = PeerKeyringResolver::new(h);
        assert!(r.resolve("easynet:///r/org/reg/agent.alice").is_ok());
        assert!(matches!(
            r.resolve("easynet:///r/org/reg/agent.bob").unwrap_err(),
            KeyResolveError::Unknown
        ));
    }

    #[test]
    fn chain_falls_through_on_unknown() {
        let (h, _dir) = handle();
        let entry = h.create_entry("agent_signing", None).unwrap();
        h.peer_add(
            "easynet:///r/org/reg/agent.alice",
            &entry.public_key_b64,
            None,
            None,
        )
        .unwrap();
        let chain = default_chain(h);
        assert!(chain.resolve("easynet:///r/org/reg/agent.alice").is_ok());
        assert!(matches!(
            chain.resolve("easynet:///r/prv/reg/agent.unknown").unwrap_err(),
            KeyResolveError::Unknown
        ));
    }
}
