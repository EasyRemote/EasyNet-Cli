// EasyNet CLI — Cross-realm user binding token (PR-N4 commit 1/N)
// =================================================================
//
// File: src/daemon/keyring/user_binding_chain.rs
// Description: Wire shape for the federated user identity binding
//              token introduced by PR-N4 (`pr-drafts/PR-N4-spec-
//              cross-realm-user-binding.md §commit 1/N`).
//
//              `UserBindingToken` is the Ed25519-signed proof of
//              "user U on realm A intends to be recognised as
//              their realm B identity". Realm A's backend signs;
//              realm B's `device.keyring.consume_federate_user_token`
//              ability verifies via PR-N2's FederatedKeyResolver
//              and writes a binding entry.
//
// Why a fresh module
// ------------------
// The existing keyring module (RFC-002) handles per-device key
// material; user-level cross-realm binding is a new concern with
// distinct invariants (INV-1..INV-5 from the spec). Putting it
// in its own file keeps the audit boundary clear: every byte of
// signed data, every reject reason, lives in this file and can
// be reviewed without grepping across abilities/resolver/store.
//
// Authoring discipline
// --------------------
// `canonical_user_binding_bytes` is the bytes-over-the-wire-that-
// matter contract: the function defines exactly what the signer
// signs, in what order, with what length-prefixed encoding, and
// the caller cannot deviate without breaking signature verify.
// Field order is locked by spec §commit 1/N — changing it is a
// wire-compat break that requires a fresh ABILITY constant.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};

/// Length of a raw Ed25519 verifying-key in bytes.
pub const ED25519_PUBKEY_LEN: usize = 32;

/// Length of a raw Ed25519 signature in bytes.
pub const ED25519_SIG_LEN: usize = 64;

/// Length of the random nonce in `UserBindingToken`. 32 bytes is
/// overkill against accidental collision and keeps the wire-shape
/// uniform with the cross-realm receipt nonce family.
pub const USER_BINDING_NONCE_LEN: usize = 32;

/// Default freshness window for token consumption: 24 hours in
/// milliseconds. PR-N4 spec §commit 3/N consumes-side check.
pub const USER_BINDING_FRESHNESS_MS: u64 = 24 * 60 * 60 * 1000;

/// **PR-N4 spec §commit 1/N**. Federated user identity binding
/// token. Wire shape — sent over JWT custom claim or out-of-band
/// channel from realm A to realm B; the consuming realm B daemon
/// verifies the signature using PR-N2's FederatedKeyResolver to
/// fetch realm A's backend pubkey.
///
/// Field order is part of the canonical bytes contract — see
/// `canonical_user_binding_bytes`. Changing field order is a
/// wire-break.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserBindingToken {
    /// Realm of the issuing user / hub. Must be a non-empty
    /// realm string per the canonical URI scheme.
    pub source_realm: String,
    /// User URI on the source realm, in the canonical form
    /// `easynet:///r/<source_realm>/user/<user-id>`.
    pub source_user_ura: String,
    /// Source user's Ed25519 verifying-key bytes. Carried inline
    /// so the consuming realm doesn't need to round-trip
    /// `federation.resolve_key` separately for the user URI; the
    /// signature step still proves the *backend* identity issued
    /// the token, not just any actor with the user's pubkey.
    /// Wire shape is a `Vec<u8>` for serde compat (stable serde
    /// only auto-derives arrays up to length 32; the same
    /// reasoning applies to `signature` below); structural
    /// length is validated on every consume + verify path.
    pub source_user_pubkey: Vec<u8>,
    /// Intended target realm. The consumer rejects when this
    /// does not match its own realm — INV-3 unidirectional check
    /// stops a token issued for realm B from being replayed at
    /// realm C.
    pub target_realm: String,
    /// Issuance epoch-ms. Consumer rejects when (now - issued_at)
    /// exceeds `USER_BINDING_FRESHNESS_MS` — bounds the replay
    /// window even if the per-nonce dedup store loses state.
    pub issued_at_ms: u64,
    /// Random nonce. Consumer's replay store dedups against this
    /// 32-byte value so the same token cannot be consumed twice.
    /// Vec<u8> for serde compat; length validated everywhere
    /// the bytes are consumed.
    pub nonce: Vec<u8>,
    /// Ed25519 signature over `canonical_user_binding_bytes`,
    /// produced by realm A's backend daemon identity. Vec<u8>
    /// because stable serde caps array auto-derive at length 32.
    pub signature: Vec<u8>,
}

impl UserBindingToken {
    /// Construct an unsigned token (signature filled with zeros).
    /// Used by signers as a building block before stamping the
    /// real signature; the unsigned form has no security
    /// properties on its own.
    #[must_use]
    pub fn new_unsigned(
        source_realm: impl Into<String>,
        source_user_ura: impl Into<String>,
        source_user_pubkey: [u8; ED25519_PUBKEY_LEN],
        target_realm: impl Into<String>,
        issued_at_ms: u64,
        nonce: [u8; USER_BINDING_NONCE_LEN],
    ) -> Self {
        Self {
            source_realm: source_realm.into(),
            source_user_ura: source_user_ura.into(),
            source_user_pubkey: source_user_pubkey.to_vec(),
            target_realm: target_realm.into(),
            issued_at_ms,
            nonce: nonce.to_vec(),
            signature: vec![0u8; ED25519_SIG_LEN],
        }
    }

    /// Validate the structural lengths of the byte fields before
    /// any cryptographic operation. Returns
    /// `Err(UserBindingError::Malformed)` with a precise reason
    /// when a field is the wrong length. Verifying or consuming
    /// callers run this first; canonical-bytes generation
    /// tolerates wrong lengths because it serialises whatever it
    /// is given (the wire-bytes contract is symmetric — we want
    /// "canonical of malformed token" to also be a stable
    /// function for replay-style audit).
    pub fn validate_lengths(&self) -> Result<(), UserBindingError> {
        if self.source_user_pubkey.len() != ED25519_PUBKEY_LEN {
            return Err(UserBindingError::Malformed(format!(
                "source_user_pubkey: expected {ED25519_PUBKEY_LEN} bytes, \
                 got {}",
                self.source_user_pubkey.len()
            )));
        }
        if self.nonce.len() != USER_BINDING_NONCE_LEN {
            return Err(UserBindingError::Malformed(format!(
                "nonce: expected {USER_BINDING_NONCE_LEN} bytes, got {}",
                self.nonce.len()
            )));
        }
        if self.signature.len() != ED25519_SIG_LEN {
            return Err(UserBindingError::Malformed(format!(
                "signature: expected {ED25519_SIG_LEN} bytes, got {}",
                self.signature.len()
            )));
        }
        Ok(())
    }
}

/// Reject reasons surfaced by `consume_federate_user_token` and
/// related verification helpers. Each variant maps to a wire-
/// stable reason string consumers can grep in audit logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserBindingError {
    /// Signature byte length wrong, or signature verify against
    /// the supplied source pubkey + canonical bytes failed.
    InvalidSignature,
    /// `target_realm` field does not match the consuming daemon's
    /// own realm. INV-3 unidirectional check.
    WrongTargetRealm { expected: String, actual: String },
    /// `issued_at_ms` is older than `USER_BINDING_FRESHNESS_MS`
    /// against the consumer's clock.
    ExpiredToken { issued_at_ms: u64, now_ms: u64 },
    /// Source realm's backend pubkey could not be resolved via
    /// PR-N2's FederatedKeyResolver (peer hub down, not
    /// federated, etc.).
    UnknownSourceRealm(String),
    /// Token's nonce is already in the consumer's replay store.
    ReplayDetected,
    /// Token bytes failed structural decode (malformed length-
    /// prefix, truncated, etc.). Surfaces under `InvalidSignature`
    /// upstream because a malformed token cannot be honestly
    /// signed; carried separately here so the test harness can
    /// distinguish parse failures from sig failures.
    Malformed(String),
}

impl UserBindingError {
    /// Wire-stable reason string for audit logs. Distinct from
    /// the `Display` impl so the audit channel format does not
    /// drift with future enum-display tweaks.
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            UserBindingError::InvalidSignature => "user_binding_invalid_signature",
            UserBindingError::WrongTargetRealm { .. } => "user_binding_wrong_target_realm",
            UserBindingError::ExpiredToken { .. } => "user_binding_expired_token",
            UserBindingError::UnknownSourceRealm(_) => "user_binding_unknown_source_realm",
            UserBindingError::ReplayDetected => "user_binding_replay_detected",
            UserBindingError::Malformed(_) => "user_binding_malformed",
        }
    }
}

impl std::fmt::Display for UserBindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserBindingError::InvalidSignature => f.write_str("invalid signature"),
            UserBindingError::WrongTargetRealm { expected, actual } => write!(
                f,
                "wrong target_realm: expected {expected:?}, got {actual:?}"
            ),
            UserBindingError::ExpiredToken {
                issued_at_ms,
                now_ms,
            } => write!(
                f,
                "expired token: issued_at_ms={issued_at_ms}, now_ms={now_ms}, freshness_window_ms={USER_BINDING_FRESHNESS_MS}"
            ),
            UserBindingError::UnknownSourceRealm(realm) => {
                write!(f, "unknown source_realm: {realm:?}")
            }
            UserBindingError::ReplayDetected => f.write_str("replay detected"),
            UserBindingError::Malformed(detail) => write!(f, "malformed token: {detail}"),
        }
    }
}

impl std::error::Error for UserBindingError {}

/// Compute the canonical bytes the signer signs over.
///
/// Format (length-prefixed concatenation, big-endian u32 length
/// for variable-length string fields, fixed-length for byte
/// arrays):
///
/// ```text
///   "easynet/user-binding/v1\n"        (24-byte ASCII domain tag)
/// + u32(source_realm.len())  || source_realm bytes
/// + u32(source_user_ura.len())  || source_user_ura bytes
/// + source_user_pubkey                 (32 bytes)
/// + u32(target_realm.len())  || target_realm bytes
/// + u64(issued_at_ms, big-endian)      (8 bytes)
/// + nonce                              (32 bytes)
/// ```
///
/// Domain tag binds the bytes to this specific use; same key
/// signing different domain tags cannot be cross-replayed.
/// Length-prefix encoding prevents extension attacks where a
/// malicious source_realm could absorb bytes from the next field.
#[must_use]
pub fn canonical_user_binding_bytes(token: &UserBindingToken) -> Vec<u8> {
    const DOMAIN_TAG: &[u8] = b"easynet/user-binding/v1\n";
    let mut out = Vec::with_capacity(
        DOMAIN_TAG.len()
            + 4
            + token.source_realm.len()
            + 4
            + token.source_user_ura.len()
            + ED25519_PUBKEY_LEN
            + 4
            + token.target_realm.len()
            + 8
            + USER_BINDING_NONCE_LEN,
    );
    out.extend_from_slice(DOMAIN_TAG);
    write_lp_string(&mut out, &token.source_realm);
    write_lp_string(&mut out, &token.source_user_ura);
    // Length-prefix pubkey + nonce too — we want canonical bytes
    // to remain unambiguous if (defensively) a malformed token
    // shows up with wrong-length fields. This makes
    // `canonical_user_binding_bytes` a total function over the
    // whole struct, including malformed inputs.
    write_lp_bytes(&mut out, &token.source_user_pubkey);
    write_lp_string(&mut out, &token.target_realm);
    out.extend_from_slice(&token.issued_at_ms.to_be_bytes());
    write_lp_bytes(&mut out, &token.nonce);
    out
}

fn write_lp_string(out: &mut Vec<u8>, s: &str) {
    let len = u32::try_from(s.len()).expect("string len fits in u32");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn write_lp_bytes(out: &mut Vec<u8>, b: &[u8]) {
    let len = u32::try_from(b.len()).expect("byte slice len fits in u32");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(b);
}

/// Sign an unsigned token with the supplied Ed25519 signing key.
/// Mutates the token's `signature` field in-place. The signing
/// key MUST be the source realm's backend daemon identity per
/// INV-1; this function does not enforce that — callers
/// (the `device.keyring.federate_user_identity_token` ability
/// handler) verify before calling.
pub fn sign_user_binding_token(
    token: &mut UserBindingToken,
    signing_key: &ed25519_dalek::SigningKey,
) {
    use ed25519_dalek::Signer;
    let bytes = canonical_user_binding_bytes(token);
    let sig = signing_key.sign(&bytes);
    token.signature = sig.to_bytes().to_vec();
}

/// Verify a signed token's structural correctness + signature.
/// Returns `Ok(())` when the signature verifies against the
/// supplied source pubkey, `Err(UserBindingError::InvalidSignature)`
/// otherwise.
///
/// This is the **structural** verify only. Caller MUST ALSO
/// check `target_realm == self_realm`, freshness, and replay
/// (per-call concerns the caller owns the clock + replay store
/// for). PR-N4 spec §commit 3/N's `consume_federate_user_token`
/// runs all four checks in order and surfaces typed reasons.
pub fn verify_user_binding_signature(token: &UserBindingToken) -> Result<(), UserBindingError> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    token.validate_lengths()?;
    let pubkey_arr: [u8; ED25519_PUBKEY_LEN] = token
        .source_user_pubkey
        .as_slice()
        .try_into()
        .map_err(|_| UserBindingError::InvalidSignature)?;
    let pubkey =
        VerifyingKey::from_bytes(&pubkey_arr).map_err(|_| UserBindingError::InvalidSignature)?;
    let sig_arr: [u8; ED25519_SIG_LEN] = token
        .signature
        .as_slice()
        .try_into()
        .map_err(|_| UserBindingError::InvalidSignature)?;
    let sig = Signature::from_bytes(&sig_arr);
    let bytes = canonical_user_binding_bytes(token);
    pubkey
        .verify(&bytes, &sig)
        .map_err(|_| UserBindingError::InvalidSignature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn fixture_token(signing: &SigningKey) -> UserBindingToken {
        let mut token = UserBindingToken::new_unsigned(
            "realm-a",
            "easynet:///r/realm-a/user/user-c",
            signing.verifying_key().to_bytes(),
            "realm-b",
            1_714_500_000_000,
            [0xAA; USER_BINDING_NONCE_LEN],
        );
        sign_user_binding_token(&mut token, signing);
        token
    }

    #[test]
    fn canonical_bytes_are_deterministic() {
        // Two calls on the same token produce byte-identical
        // output. The wire-stable contract: same input ⇒ same
        // bytes for the signer.
        let signing = SigningKey::from_bytes(&[0x11; 32]);
        let token = fixture_token(&signing);
        let bytes_1 = canonical_user_binding_bytes(&token);
        let bytes_2 = canonical_user_binding_bytes(&token);
        assert_eq!(bytes_1, bytes_2);
    }

    #[test]
    fn canonical_bytes_carry_domain_tag_at_front() {
        let signing = SigningKey::from_bytes(&[0x22; 32]);
        let token = fixture_token(&signing);
        let bytes = canonical_user_binding_bytes(&token);
        assert!(
            bytes.starts_with(b"easynet/user-binding/v1\n"),
            "domain tag must lead the canonical bytes — without it, a key \
             signing this domain could be replayed against another"
        );
    }

    #[test]
    fn canonical_bytes_distinguish_field_swaps() {
        // Swapping source_realm and target_realm produces
        // different bytes — proves length-prefix encoding
        // doesn't allow field-content blurring.
        let signing = SigningKey::from_bytes(&[0x33; 32]);
        let mut token_a = UserBindingToken::new_unsigned(
            "realm-a",
            "easynet:///r/realm-a/user/user",
            signing.verifying_key().to_bytes(),
            "realm-b",
            1_714_500_000_000,
            [0; USER_BINDING_NONCE_LEN],
        );
        sign_user_binding_token(&mut token_a, &signing);
        let mut token_b = token_a.clone();
        token_b.source_realm = "realm-b".to_string();
        token_b.target_realm = "realm-a".to_string();
        sign_user_binding_token(&mut token_b, &signing);
        assert_ne!(
            canonical_user_binding_bytes(&token_a),
            canonical_user_binding_bytes(&token_b),
            "swapping source/target realms must produce distinct bytes"
        );
    }

    #[test]
    fn round_trip_serde_preserves_all_fields() {
        let signing = SigningKey::from_bytes(&[0x44; 32]);
        let token = fixture_token(&signing);
        let bytes = serde_json::to_vec(&token).expect("serialise");
        let restored: UserBindingToken = serde_json::from_slice(&bytes).expect("deserialise");
        assert_eq!(token, restored);
    }

    #[test]
    fn signed_token_verifies_with_correct_pubkey() {
        let signing = SigningKey::from_bytes(&[0x55; 32]);
        let token = fixture_token(&signing);
        verify_user_binding_signature(&token).expect("happy-path verify");
    }

    #[test]
    fn tampered_source_realm_fails_verify() {
        let signing = SigningKey::from_bytes(&[0x66; 32]);
        let mut token = fixture_token(&signing);
        token.source_realm = "realm-attacker".to_string();
        let err = verify_user_binding_signature(&token).expect_err("must fail");
        assert!(matches!(err, UserBindingError::InvalidSignature));
        assert_eq!(err.reason(), "user_binding_invalid_signature");
    }

    #[test]
    fn tampered_target_realm_fails_verify() {
        let signing = SigningKey::from_bytes(&[0x77; 32]);
        let mut token = fixture_token(&signing);
        token.target_realm = "realm-stolen".to_string();
        verify_user_binding_signature(&token).expect_err("tampered target_realm rejected");
    }

    #[test]
    fn tampered_issued_at_fails_verify() {
        let signing = SigningKey::from_bytes(&[0x88; 32]);
        let mut token = fixture_token(&signing);
        token.issued_at_ms = token.issued_at_ms.wrapping_add(1);
        verify_user_binding_signature(&token).expect_err("tampered issued_at rejected");
    }

    #[test]
    fn tampered_nonce_fails_verify() {
        let signing = SigningKey::from_bytes(&[0x99; 32]);
        let mut token = fixture_token(&signing);
        token.nonce[0] ^= 0x01;
        verify_user_binding_signature(&token).expect_err("tampered nonce rejected");
    }

    #[test]
    fn signature_with_wrong_key_fails_verify() {
        let real_signing = SigningKey::from_bytes(&[0xAA; 32]);
        let attacker_signing = SigningKey::from_bytes(&[0xBB; 32]);
        let mut token = UserBindingToken::new_unsigned(
            "realm-a",
            "easynet:///r/realm-a/user/u",
            // Embed the LEGITIMATE pubkey so verify pulls the
            // right key — but sign with the attacker's key.
            real_signing.verifying_key().to_bytes(),
            "realm-b",
            1_714_500_000_000,
            [0xCC; USER_BINDING_NONCE_LEN],
        );
        sign_user_binding_token(&mut token, &attacker_signing);
        verify_user_binding_signature(&token).expect_err("signature by wrong key must fail verify");
    }

    #[test]
    fn error_reasons_are_wire_stable() {
        // Regression-pin every variant's reason string. PR-N5
        // audit pipelines grep these.
        assert_eq!(
            UserBindingError::InvalidSignature.reason(),
            "user_binding_invalid_signature"
        );
        assert_eq!(
            UserBindingError::WrongTargetRealm {
                expected: "x".into(),
                actual: "y".into()
            }
            .reason(),
            "user_binding_wrong_target_realm"
        );
        assert_eq!(
            UserBindingError::ExpiredToken {
                issued_at_ms: 0,
                now_ms: 0
            }
            .reason(),
            "user_binding_expired_token"
        );
        assert_eq!(
            UserBindingError::UnknownSourceRealm("x".into()).reason(),
            "user_binding_unknown_source_realm"
        );
        assert_eq!(
            UserBindingError::ReplayDetected.reason(),
            "user_binding_replay_detected"
        );
        assert_eq!(
            UserBindingError::Malformed("x".into()).reason(),
            "user_binding_malformed"
        );
    }
}
