// EasyNet CLI — Provisional URA helper (RFC-001 §A2 / §A5)
// ==========================================================
//
// File: src/core/ura/provisional.rs
//
// Computes the §A2 pre-membership identity for a freshly-generated
// device keypair: `provisional:<sha256(public_key) hex>`. This is
// the only caller URI a non-member is permitted to present, and it
// is only valid as caller of `federation.join` (§A6, enforced by
// the membership gate at the receiving daemon).
//
// Why a dedicated module
// ----------------------
// The fingerprint algorithm is a protocol-level constant: any drift
// between this CLI and the hub's verifier silently breaks the
// genesis path. Pinning it in one named function makes it easy to
// grep for, easy to test, and easy to audit when the algorithm
// rolls (e.g. if we ever migrate from sha256 to blake3).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use sha2::{Digest, Sha256};

/// Per RFC §A2: provisional URAs are the literal string
/// `provisional:` followed by the lowercase hex of `sha256(public_key)`.
/// `public_key` is the raw 32-byte Ed25519 public key the joining
/// device generated.
pub fn provisional_ura_for_pubkey(public_key: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(public_key);
    let digest = hasher.finalize();
    format!("provisional:{}", hex::encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisional_ura_has_protocol_prefix() {
        let ura = provisional_ura_for_pubkey(&[0u8; 32]);
        assert!(ura.starts_with("provisional:"));
    }

    #[test]
    fn provisional_ura_is_deterministic_for_same_pubkey() {
        let pk = [42u8; 32];
        assert_eq!(
            provisional_ura_for_pubkey(&pk),
            provisional_ura_for_pubkey(&pk),
        );
    }

    #[test]
    fn provisional_ura_differs_across_distinct_pubkeys() {
        let a = provisional_ura_for_pubkey(&[1u8; 32]);
        let b = provisional_ura_for_pubkey(&[2u8; 32]);
        assert_ne!(a, b);
    }

    #[test]
    fn provisional_ura_emits_64_char_hex_digest() {
        // sha256 = 32 bytes = 64 hex chars. Anything else means the
        // algorithm changed without updating the spec — fail loud.
        let ura = provisional_ura_for_pubkey(&[0u8; 32]);
        let suffix = ura.strip_prefix("provisional:").unwrap();
        assert_eq!(
            suffix.len(),
            64,
            "provisional URA suffix must be 64 hex chars (sha256)"
        );
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn provisional_ura_matches_known_vector_for_zeros() {
        // Locks the algorithm. sha256(32 zero bytes) is a stable
        // public test vector; if this assertion ever fires, a new
        // CLI build would fail to join an existing hub — guard
        // that by failing in unit tests first.
        let ura = provisional_ura_for_pubkey(&[0u8; 32]);
        assert_eq!(
            ura,
            "provisional:66687aadf862bd776c8fc18b8e9f8e20089714856ee233b3902a591d0d5f2925"
        );
    }
}
