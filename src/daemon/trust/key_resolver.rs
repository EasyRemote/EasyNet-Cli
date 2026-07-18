//! `RealmTrustAnchor` -> `axon_sdk::invocation::KeyResolver`.
//!
//! This is a daemon-trust adapter: the daemon owns `RealmTrustAnchor`
//! and its hot-reload cell, while Axon's `LocalRuntime` only needs a
//! `KeyResolver` trait object.

use axon_sdk::invocation::{AxonError, ErrorCode, ErrorStage, KeyResolver, SecurityClass};
use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};
use ed25519_dalek::VerifyingKey;

use crate::daemon::trust::cell::SharedTrustAnchor;

/// `axon_sdk::invocation::KeyResolver` wired to the daemon's
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

fn caller_key_unavailable(agent_ura: &str, detail: impl Into<String>) -> AxonError {
    AxonError::invalid_argument(ErrorCode::CallerKeyNotFound.as_str())
        .with_code(ErrorCode::CallerKeyNotFound)
        .with_stage(ErrorStage::CallerAuthentication)
        .with_security_class(SecurityClass::Identity)
        .with_message(format!(
            "realm_trust_anchor: caller {agent_ura}: {}",
            detail.into()
        ))
}

fn decode_pubkey(public_key_b64: &str, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
    let raw = B64_STANDARD
        .decode(public_key_b64.as_bytes())
        .map_err(|err| {
            caller_key_unavailable(agent_ura, format!("pubkey base64 invalid: {err}"))
        })?;
    let bytes: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
        caller_key_unavailable(
            agent_ura,
            format!("pubkey is {} bytes; expected 32", raw.len()),
        )
    })?;
    VerifyingKey::from_bytes(&bytes).map_err(|err| {
        caller_key_unavailable(
            agent_ura,
            format!("pubkey is not a valid Ed25519 point: {err}"),
        )
    })
}

impl KeyResolver for RealmTrustAnchorKeyResolver {
    fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        let anchor = self.trust_anchor.snapshot();
        let entry = anchor.lookup(agent_ura).ok_or_else(|| {
            caller_key_unavailable(
                agent_ura,
                "caller is not in the realm trust anchor; no trust-anchor entry",
            )
        })?;
        decode_pubkey(&entry.public_key_b64, agent_ura)
    }

    /// DEC-EU multi-device admission: a user URA legitimately carries
    /// one key per signing device (browser, phone, tablet). The SDK's
    /// `verify_invocation_signature` admits if ANY returned key
    /// verifies; its equivalence invariant holds because the user
    /// bucket is partitioned by exact URA. Bounded to the verifier's
    /// `MAX_KEYS_PER_AGENT_URA` ceiling. A row with a corrupt pubkey
    /// is skipped rather than poisoning the user's valid keys.
    fn resolve_all(&self, agent_ura: &str) -> Result<Vec<VerifyingKey>, AxonError> {
        let anchor = self.trust_anchor.snapshot();
        let user_rows = anchor.lookup_user_all(agent_ura);
        if user_rows.is_empty() {
            return self.resolve(agent_ura).map(|key| vec![key]);
        }
        let keys: Vec<VerifyingKey> = user_rows
            .iter()
            .take(axon_sdk::invocation::MAX_KEYS_PER_AGENT_URA)
            .filter_map(|row| decode_pubkey(&row.public_key_b64, agent_ura).ok())
            .collect();
        if keys.is_empty() {
            return Err(caller_key_unavailable(agent_ura, "no decodable user key"));
        }
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};

    fn make_anchor_with(agent_ura: &str, signing_key: &SigningKey) -> SharedTrustAnchor {
        let entry = TrustedAgent {
            agent_ura: agent_ura.to_string(),
            public_key_b64: B64_STANDARD.encode(signing_key.verifying_key().to_bytes()),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
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
    fn resolve_returns_typed_caller_key_failure_for_unknown_agent() {
        let signing_key = SigningKey::from_bytes(&[0x22; 32]);
        let anchor = make_anchor_with("easynet:///r/test/device/d1", &signing_key);
        let resolver = RealmTrustAnchorKeyResolver::new(anchor);

        let err = resolver
            .resolve("easynet:///r/test/device/UNKNOWN")
            .expect_err("unknown agent must reject");
        assert_eq!(err.code, ErrorCode::CallerKeyNotFound);
        assert_eq!(err.stage, Some(ErrorStage::CallerAuthentication));
        assert_eq!(err.security_class, Some(SecurityClass::Identity));
        assert!(
            err.to_string().contains("no trust-anchor entry"),
            "diagnostic must name the missing URA: {err}"
        );
    }

    #[test]
    fn resolve_all_returns_every_user_key() {
        // DEC-EU multi-device regression: a user with two registered
        // browser keys must have BOTH admissible — the 2026-06-10
        // CALLER_SIGNATURE_INVALID came from single-key resolve
        // verifying only the first row.
        let user_ura = "easynet:///r/test/user/dev";
        let key_a = SigningKey::from_bytes(&[0x55; 32]);
        let key_b = SigningKey::from_bytes(&[0x66; 32]);
        let row = |sk: &SigningKey| TrustedAgent {
            agent_ura: user_ura.to_string(),
            public_key_b64: B64_STANDARD.encode(sk.verifying_key().to_bytes()),
            role: TrustedAgentRole::User,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };
        let anchor = RealmTrustAnchor::from_entries(vec![row(&key_a), row(&key_b)])
            .expect("multi-key user anchor");
        let resolver = RealmTrustAnchorKeyResolver::new(SharedTrustAnchor::new(Arc::new(anchor)));

        let keys = resolver.resolve_all(user_ura).expect("user keys resolve");
        let got: Vec<[u8; 32]> = keys.iter().map(|k| k.to_bytes()).collect();
        assert_eq!(keys.len(), 2, "both user keys must be admissible");
        assert!(got.contains(&key_a.verifying_key().to_bytes()));
        assert!(got.contains(&key_b.verifying_key().to_bytes()));
    }

    #[test]
    fn resolve_all_falls_back_to_single_for_devices() {
        let signing_key = SigningKey::from_bytes(&[0x77; 32]);
        let anchor = make_anchor_with("easynet:///r/test/device/d1", &signing_key);
        let resolver = RealmTrustAnchorKeyResolver::new(anchor);
        let keys = resolver
            .resolve_all("easynet:///r/test/device/d1")
            .expect("device resolves");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].to_bytes(), signing_key.verifying_key().to_bytes());
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
            origin_realm: None,
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
