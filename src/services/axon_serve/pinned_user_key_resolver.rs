// EasyNet CLI — DEC-EU pinned-pubkey resolver for user envelopes
// =================================================================
//
// File: src/services/axon_serve/pinned_user_key_resolver.rs
//
// DEC-EU §multi-device. The SDK's `KeyResolver` trait signature is
// `fn resolve(&self, caller_uri: &str) -> VerifyingKey`. For
// hub/backend/device URIs that's a 1:1 mapping. For user URIs it
// is NOT: under DEC-EU multi-device, one user URI carries N
// pubkeys (one per signing device). A bare-URI resolver has to
// pick one — `RealmTrustAnchor::lookup` returns the lex-smallest
// — and the wrong pick silently fails `verify_invocation_signature`
// for every device whose pubkey is not lex-smallest.
//
// This module provides the pin: a resolver that already knows the
// envelope's presented public key (the runtime peels it from
// `envelope.caller_signature.public_key` at admission time) and
// answers "yes, that exact key" if and only if it is registered
// under `caller_uri` in the trust anchor's user bucket.
//
// Used by `admission_facade::run_strict_admission` for User-role
// callers. Hub / backend / device callers continue to use
// `FederatedKeyResolver` because their 1:1 mapping is correct.
//
// What this resolver IS
// ---------------------
// - A read-only adapter over `RealmTrustAnchor` keyed by both URI
//   AND presented pubkey
// - Same-realm only — cross-realm user roaming is the
//   FederatedKeyResolver's job and is the next step on this fix
//
// What this resolver is NOT
// -------------------------
// - Not a signature verifier — it returns the key; SDK
//   `verify_invocation_signature` does the actual Ed25519 check
// - Not a caller-URI parser — it trusts the caller did the right
//   role-dispatch (admission_facade does)
// - Not a cache — every resolve is a fresh snapshot lookup,
//   matching `FederatedKeyResolver`'s local-fast path semantics
//
// Invariants
// ----------
// **Invariant 1 (one resolver per envelope)**: a resolver instance
// is bound to exactly one presented pubkey. Reusing the same
// resolver across envelopes is a programming error — the pubkey
// pinning becomes ambiguous. `run_strict_admission` constructs
// one fresh resolver per call.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026-2027 EasyNet. All rights reserved.

use std::sync::Arc;

use base64::prelude::*;
use easynet_axon::invocation::axiom::KeyResolver;
use easynet_axon::invocation::AxonError;
use ed25519_dalek::VerifyingKey;

use crate::services::realm_trust_anchor::RealmTrustAnchor;

/// Resolver pinned to the public key the envelope presented. Yes/no
/// answer keyed by (caller_uri, presented_pubkey) — anything else
/// surfaces as `unknown_agent_uri`, matching the
/// `FederatedKeyResolver` failure shape.
pub struct PinnedUserKeyResolver {
    trust_anchor: Arc<RealmTrustAnchor>,
    presented_pubkey_b64: String,
}

impl PinnedUserKeyResolver {
    pub fn new(trust_anchor: Arc<RealmTrustAnchor>, presented_pubkey_b64: String) -> Self {
        Self {
            trust_anchor,
            presented_pubkey_b64,
        }
    }
}

impl KeyResolver for PinnedUserKeyResolver {
    fn resolve(&self, caller_uri: &str) -> Result<VerifyingKey, AxonError> {
        let entry = self
            .trust_anchor
            .lookup_user_by_pubkey(caller_uri, &self.presented_pubkey_b64)
            .ok_or_else(|| {
                AxonError::new(easynet_axon::invocation::AxonErrorKind::InvalidArgument)
                    .with_reason("unknown_agent_uri")
                    .with_message(format!(
                        "user_uri:{caller_uri}:presented_pubkey_not_registered"
                    ))
            })?;
        let raw = BASE64_STANDARD.decode(&entry.public_key_b64).map_err(|e| {
            AxonError::new(easynet_axon::invocation::AxonErrorKind::InvalidArgument)
                .with_reason("public_key_b64_decode_failed")
                .with_message(format!("user_uri:{caller_uri}:{e}"))
        })?;
        let arr: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
            AxonError::new(easynet_axon::invocation::AxonErrorKind::InvalidArgument)
                .with_reason("public_key_wrong_length")
                .with_message(format!(
                    "user_uri:{caller_uri}:expected_32_got_{}",
                    raw.len()
                ))
        })?;
        VerifyingKey::from_bytes(&arr).map_err(|e| {
            AxonError::new(easynet_axon::invocation::AxonErrorKind::InvalidArgument)
                .with_reason("public_key_parse_failed")
                .with_message(format!("user_uri:{caller_uri}:{e}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::realm_trust_anchor::{TrustedAgent, TrustedAgentRole};

    fn b64_pubkey(seed: u8) -> String {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        BASE64_STANDARD.encode(sk.verifying_key().to_bytes())
    }

    fn anchor_with_two_user_keys() -> (Arc<RealmTrustAnchor>, String, String, &'static str) {
        let pk_a = b64_pubkey(1);
        let pk_b = b64_pubkey(2);
        let alice = "easynet:///r/realm/user/alice";
        let mut anchor = RealmTrustAnchor::default();
        for pk in [&pk_a, &pk_b] {
            anchor
                .append_agent(TrustedAgent {
                    agent_uri: alice.to_string(),
                    public_key_b64: pk.clone(),
                    role: TrustedAgentRole::User,
                    added_at_unix_ms: 1_714_000_000_000,
                    origin_tenant_id: None,
                    hub_uri: None,
                    tls_ca_pem_path: None,
                })
                .expect("append");
        }
        (Arc::new(anchor), pk_a, pk_b, alice)
    }

    #[test]
    fn pins_the_specific_pubkey_envelope_presented() {
        // Multi-device user. Resolver pinned to pk_b returns pk_b
        // even though lex-smallest is pk_a — the whole point of the
        // pin is that the SDK trait's 1:1 lookup cannot pick the
        // wrong key.
        let (anchor, _pk_a, pk_b, alice) = anchor_with_two_user_keys();
        let resolver = PinnedUserKeyResolver::new(anchor, pk_b.clone());
        let key = resolver.resolve(alice).expect("resolves pk_b");
        let raw = BASE64_STANDARD.decode(&pk_b).unwrap();
        let expected: [u8; 32] = raw.as_slice().try_into().unwrap();
        assert_eq!(key.as_bytes(), &expected);
    }

    #[test]
    fn rejects_unregistered_pubkey() {
        let (anchor, _pk_a, _pk_b, alice) = anchor_with_two_user_keys();
        let unregistered = b64_pubkey(99);
        let resolver = PinnedUserKeyResolver::new(anchor, unregistered);
        let err = resolver.resolve(alice).expect_err("must reject");
        assert!(
            err.reason.contains("unknown_agent_uri"),
            "want unknown_agent_uri, got {err:?}"
        );
    }

    #[test]
    fn rejects_unknown_user_uri() {
        let (anchor, pk_a, _pk_b, _alice) = anchor_with_two_user_keys();
        let resolver = PinnedUserKeyResolver::new(anchor, pk_a);
        let err = resolver
            .resolve("easynet:///r/realm/user/bob")
            .expect_err("must reject");
        assert!(err.reason.contains("unknown_agent_uri"));
    }
}
