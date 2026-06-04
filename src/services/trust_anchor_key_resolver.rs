//! `RealmTrustAnchor` -> `easynet_axon::invocation::KeyResolver`.
//!
//! This is a services-layer adapter: the daemon owns
//! `RealmTrustAnchor` and its hot-reload cell, while Axon's
//! `LocalRuntime` only needs a `KeyResolver` trait object.

use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};
use easynet_axon::invocation::{AxonError, KeyResolver};
use ed25519_dalek::VerifyingKey;

use crate::services::trust_anchor_cell::SharedTrustAnchor;

/// `easynet_axon::invocation::KeyResolver` wired to the daemon's
/// `SharedTrustAnchor`.
pub struct RealmTrustAnchorKeyResolver {
    trust_anchor: SharedTrustAnchor,
}

impl RealmTrustAnchorKeyResolver {
    #[must_use]
    pub fn new(trust_anchor: SharedTrustAnchor) -> Self {
        Self { trust_anchor }
    }
}

impl KeyResolver for RealmTrustAnchorKeyResolver {
    fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        let anchor = self.trust_anchor.snapshot();
        let entry = anchor.lookup(agent_ura).ok_or_else(|| {
            AxonError::permission_denied(format!(
                "realm_trust_anchor: no entry for caller {agent_ura}"
            ))
        })?;
        let raw = B64_STANDARD
            .decode(entry.public_key_b64.as_bytes())
            .map_err(|err| {
                AxonError::permission_denied(format!(
                    "realm_trust_anchor: pubkey base64 invalid for {agent_ura}: {err}"
                ))
            })?;
        let bytes: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
            AxonError::permission_denied(format!(
                "realm_trust_anchor: pubkey for {agent_ura} is {} bytes; expected 32",
                raw.len()
            ))
        })?;
        VerifyingKey::from_bytes(&bytes).map_err(|err| {
            AxonError::permission_denied(format!(
                "realm_trust_anchor: pubkey for {agent_ura} is not a valid Ed25519 point: {err}"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

    fn make_anchor_with(agent_ura: &str, signing_key: &SigningKey) -> SharedTrustAnchor {
        let entry = TrustedAgent {
            agent_ura: agent_ura.to_string(),
            public_key_b64: B64_STANDARD.encode(signing_key.verifying_key().to_bytes()),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_tenant_id: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };
        let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
        SharedTrustAnchor::new(Arc::new(anchor))
    }

    #[test]
    fn resolve_returns_verifying_key_for_known_agent() {
        let signing_key = SigningKey::from_bytes(&[0x11; 32]);
        let expected = signing_key.verifying_key();
        let anchor = make_anchor_with("easynet:///r/test/device/d1", &signing_key);

        let resolver = RealmTrustAnchorKeyResolver::new(anchor);
        let got = resolver
            .resolve("easynet:///r/test/device/d1")
            .expect("known agent must resolve");
        assert_eq!(got.to_bytes(), expected.to_bytes());
    }

    #[test]
    fn resolve_returns_permission_denied_for_unknown_agent() {
        let signing_key = SigningKey::from_bytes(&[0x22; 32]);
        let anchor = make_anchor_with("easynet:///r/test/device/d1", &signing_key);
        let resolver = RealmTrustAnchorKeyResolver::new(anchor);

        let err = resolver
            .resolve("easynet:///r/test/device/UNKNOWN")
            .expect_err("unknown agent must reject");
        assert!(
            err.to_string().contains("no entry for caller"),
            "diagnostic must name the missing URA: {err}"
        );
    }

    #[test]
    fn resolve_sees_hot_reload_through_shared_anchor() {
        let signing_a = SigningKey::from_bytes(&[0x33; 32]);
        let anchor = make_anchor_with("easynet:///r/test/device/a", &signing_a);
        let resolver = RealmTrustAnchorKeyResolver::new(anchor.clone());

        assert!(
            resolver.resolve("easynet:///r/test/device/b").is_err(),
            "b absent before swap"
        );

        let signing_b = SigningKey::from_bytes(&[0x44; 32]);
        let new_entry = TrustedAgent {
            agent_ura: "easynet:///r/test/device/b".to_string(),
            public_key_b64: B64_STANDARD.encode(signing_b.verifying_key().to_bytes()),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_tenant_id: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };
        let new_anchor = RealmTrustAnchor::from_entries(vec![new_entry]).expect("anchor");
        anchor.replace(Arc::new(new_anchor));

        let got = resolver
            .resolve("easynet:///r/test/device/b")
            .expect("post-swap resolve must succeed");
        assert_eq!(got.to_bytes(), signing_b.verifying_key().to_bytes());
    }
}
