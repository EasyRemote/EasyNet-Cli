// EasyNet CLI — Keyring crypto primitives (RFC-002 §3.2)
// =======================================================
//
// AES-GCM-256 for at-rest encryption of private keys.
// Argon2id (m=64MB, t=3, p=4) for passphrase-derived master keys.
// CSPRNG for nonces, salts, and ed25519 keypair generation.
//
// All operations are pure — no I/O, no global state. The store
// layer composes these with file reads/writes.
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngCore;

/// Master key wrapping a 32-byte AES-256 key.
#[derive(Clone)]
pub struct MasterKey([u8; 32]);

impl MasterKey {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Generate a fresh random master key (used when creating a new
    /// keyring with passphrase fallback — the wrapped form on disk
    /// is the passphrase-Argon2id-derived key encrypting THIS random
    /// key, allowing passphrase change without re-encrypting every
    /// entry. v1 simplification: master key IS the passphrase-derived
    /// key directly. Re-keying support is RFC-002.1.
    pub fn random() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        Self(bytes)
    }
}

/// Argon2id parameters — RFC-002 §3.2: m=64MB (65536 KiB), t=3, p=4.
pub const ARGON2_MEMORY_KIB: u32 = 65_536;
pub const ARGON2_ITERATIONS: u32 = 3;
pub const ARGON2_PARALLELISM: u32 = 4;
pub const ARGON2_SALT_LEN: usize = 16;

/// Derive a 32-byte master key from a passphrase + salt.
pub fn derive_master_key_from_passphrase(passphrase: &str, salt: &[u8]) -> Result<MasterKey> {
    if salt.len() < 8 {
        return Err(anyhow!("argon2 salt too short"));
    }
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        Some(32),
    )
    .map_err(|e| anyhow!("argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|e| anyhow!("argon2 derive: {e}"))?;
    Ok(MasterKey::from_bytes(out))
}

/// Generate a fresh Argon2id salt.
pub fn fresh_salt() -> [u8; ARGON2_SALT_LEN] {
    let mut salt = [0u8; ARGON2_SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    salt
}

/// AES-GCM nonce length (12 bytes per spec).
pub const AES_GCM_NONCE_LEN: usize = 12;

/// Ciphertext layout: `nonce(12) || ct_with_tag`.
pub struct WrappedSecret {
    pub bytes: Vec<u8>,
}

impl WrappedSecret {
    pub fn nonce(&self) -> &[u8] {
        &self.bytes[..AES_GCM_NONCE_LEN]
    }

    pub fn ciphertext_with_tag(&self) -> &[u8] {
        &self.bytes[AES_GCM_NONCE_LEN..]
    }
}

/// Encrypt a secret with the master key. Output is `nonce || ct || tag`.
pub fn aead_encrypt(master: &MasterKey, plaintext: &[u8], aad: &[u8]) -> Result<WrappedSecret> {
    let key = Key::<Aes256Gcm>::from_slice(master.as_bytes());
    let cipher = Aes256Gcm::new(key);
    let mut nonce_bytes = [0u8; AES_GCM_NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(
            nonce,
            aes_gcm::aead::Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|e| anyhow!("aes-gcm encrypt: {e}"))?;
    let mut out = Vec::with_capacity(AES_GCM_NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(WrappedSecret { bytes: out })
}

/// Decrypt a wrapped secret with the master key.
pub fn aead_decrypt(master: &MasterKey, wrapped: &WrappedSecret, aad: &[u8]) -> Result<Vec<u8>> {
    if wrapped.bytes.len() < AES_GCM_NONCE_LEN + 16 {
        return Err(anyhow!("wrapped secret too short"));
    }
    let key = Key::<Aes256Gcm>::from_slice(master.as_bytes());
    let cipher = Aes256Gcm::new(key);
    let nonce = Nonce::from_slice(wrapped.nonce());
    cipher
        .decrypt(
            nonce,
            aes_gcm::aead::Payload {
                msg: wrapped.ciphertext_with_tag(),
                aad,
            },
        )
        .map_err(|e| anyhow!("aes-gcm decrypt (wrong master key or tampered): {e}"))
}

/// SHA-256 fingerprint over a public key.
pub fn fingerprint(public_key: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(public_key);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

/// Generate a fresh ed25519 keypair. Returns (private_key_seed, public_key).
/// The seed is what gets encrypted at rest; the verifying key is
/// reconstructible from it on demand.
pub fn fresh_ed25519_keypair() -> Result<([u8; 32], [u8; 32])> {
    use ed25519_dalek::SigningKey;
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let sk = SigningKey::from_bytes(&seed);
    let pk = sk.verifying_key().to_bytes();
    Ok((seed, pk))
}

/// Sign a payload with an ed25519 private-key seed. Returns 64-byte signature.
pub fn ed25519_sign(seed: &[u8], payload: &[u8]) -> Result<[u8; 64]> {
    use ed25519_dalek::{Signer, SigningKey};
    let seed_arr: [u8; 32] = seed
        .try_into()
        .with_context(|| format!("ed25519 seed must be 32 bytes, got {}", seed.len()))?;
    let sk = SigningKey::from_bytes(&seed_arr);
    Ok(sk.sign(payload).to_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argon2_derives_deterministically_for_same_inputs() {
        let salt = b"0123456789abcdef";
        let a = derive_master_key_from_passphrase("hunter2", salt).unwrap();
        let b = derive_master_key_from_passphrase("hunter2", salt).unwrap();
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn argon2_diverges_on_different_passphrases() {
        let salt = b"0123456789abcdef";
        let a = derive_master_key_from_passphrase("hunter2", salt).unwrap();
        let b = derive_master_key_from_passphrase("hunter3", salt).unwrap();
        assert_ne!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn aead_round_trip_recovers_plaintext() {
        let mk = MasterKey::random();
        let pt = b"super secret seed bytes";
        let aad = b"keyring/v1/aad";
        let wrapped = aead_encrypt(&mk, pt, aad).unwrap();
        let recovered = aead_decrypt(&mk, &wrapped, aad).unwrap();
        assert_eq!(recovered, pt);
    }

    #[test]
    fn aead_decrypt_with_wrong_master_fails() {
        let mk1 = MasterKey::random();
        let mk2 = MasterKey::random();
        let wrapped = aead_encrypt(&mk1, b"secret", b"aad").unwrap();
        assert!(aead_decrypt(&mk2, &wrapped, b"aad").is_err());
    }

    #[test]
    fn aead_decrypt_with_tampered_aad_fails() {
        let mk = MasterKey::random();
        let wrapped = aead_encrypt(&mk, b"secret", b"aad-correct").unwrap();
        assert!(aead_decrypt(&mk, &wrapped, b"aad-wrong").is_err());
    }

    #[test]
    fn ed25519_sign_verify_roundtrip() {
        use ed25519_dalek::{Verifier, VerifyingKey};
        let (seed, pk) = fresh_ed25519_keypair().unwrap();
        let sig = ed25519_sign(&seed, b"payload").unwrap();
        let vk = VerifyingKey::from_bytes(&pk).unwrap();
        let sig_obj = ed25519_dalek::Signature::from_bytes(&sig);
        assert!(vk.verify(b"payload", &sig_obj).is_ok());
        assert!(vk.verify(b"different payload", &sig_obj).is_err());
    }

    #[test]
    fn fingerprint_is_sha256() {
        let pk = b"some 32 byte public key here----";
        let fp = fingerprint(pk);
        assert_eq!(fp.len(), 32);
        // Stable across runs:
        let fp2 = fingerprint(pk);
        assert_eq!(fp, fp2);
    }
}
