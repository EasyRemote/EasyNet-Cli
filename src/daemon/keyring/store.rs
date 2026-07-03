// EasyNet CLI — Keyring on-disk schema (RFC-002 §3.1)
// =====================================================
//
// JSON file at $XDG_CONFIG_HOME/easynet/keyring.json containing:
//   - master-key descriptor (kind = passphrase | os_keychain) + salt
//   - per-key entries with AES-GCM-wrapped private-key seeds
//   - peer table (TOFU public-key bindings + via_hub routing hints)
//
// File format is JSON (not TOML/binary) for human auditability.
// Private-key bytes are NEVER stored cleartext; only the wrapped
// ciphertext + nonce. Master-key derivation is described in §3.2.
//
// v1 OS-keychain integration is stubbed: the kind enum admits the
// variant but unlocks fall through to passphrase. A follow-up
// commit wires the platform crates (security-framework on macOS,
// secret-service on Linux, dpapi on Windows). The ability surface
// is identical either way.
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::crypto::{
    aead_decrypt, aead_encrypt, derive_master_key_from_passphrase, ed25519_sign, fingerprint,
    fresh_ed25519_keypair, fresh_salt, MasterKey, WrappedSecret,
};

pub const KEYRING_FILE_NAME: &str = "keyring.json";
pub const KEYRING_FORMAT_VERSION: u32 = 1;

/// Encoded as `nonce(12) || ct||tag` on disk, base64-standard.
fn b64_encode(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    STANDARD.encode(bytes)
}

fn b64_decode(s: &str) -> Result<Vec<u8>> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    STANDARD
        .decode(s)
        .map_err(|e| anyhow!("base64 decode: {e}"))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MasterKeyKind {
    /// Passphrase-derived (Argon2id). Stored salt is mandatory.
    Passphrase,
    /// OS keychain-wrapped. v1 falls through to passphrase if the
    /// platform keychain is unavailable.
    OsKeychain,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MasterKeyDescriptor {
    pub kind: MasterKeyKind,
    /// Argon2id salt for `Passphrase` kind. base64.
    pub salt_b64: String,
    /// Wrapped DEK for `OsKeychain` kind (reserved for v1.1).
    /// Currently unused; left for future re-keying support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrapped_dek_b64: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyStatus {
    Active,
    Retired,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub key_id: String,
    pub algo: String, // "ed25519"
    pub purpose: String,
    pub public_key_b64: String,
    /// `nonce(12) || ct||tag`, AES-256-GCM under master key.
    pub private_key_ciphertext_b64: String,
    pub status: KeyStatus,
    pub rotation_epoch: u64,
    /// Bound subject — the AgentIdentity URA this key signs for.
    pub bound_subject: Option<String>,
    pub rotated_from: Option<String>,
    pub created_unix_ms: i64,
    pub expires_unix_ms: Option<i64>,
    pub revoked_unix_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PeerStatus {
    Trusted,
    Suspended,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerEntry {
    pub peer_ura: String,
    pub fingerprint_b64: String, // sha256(public_key)
    pub public_key_b64: String,
    pub status: PeerStatus,
    pub via_hub: Option<String>,
    pub added_unix_ms: i64,
    pub last_seen_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyRing {
    pub ring_id: String,
    pub device_subject: Option<String>,
    pub format_version: u32,
    pub master_key: MasterKeyDescriptor,
    pub entries: Vec<Entry>,
    pub peer_table: Vec<PeerEntry>,
}

impl KeyRing {
    pub fn empty(ring_id: String, master: MasterKeyDescriptor) -> Self {
        Self {
            ring_id,
            device_subject: None,
            format_version: KEYRING_FORMAT_VERSION,
            master_key: master,
            entries: Vec::new(),
            peer_table: Vec::new(),
        }
    }
}

/// Default config path: $XDG_CONFIG_HOME/easynet/keyring.json or
/// platform-specific equivalent.
pub fn default_keyring_path() -> Result<PathBuf> {
    let base = dirs::config_dir().ok_or_else(|| anyhow!("cannot locate user config directory"))?;
    Ok(base.join("easynet").join(KEYRING_FILE_NAME))
}

/// Read-only load of a keyring file (does NOT unlock private keys —
/// that requires the master key from `unlock_master_key`).
pub fn load_keyring(path: &Path) -> Result<KeyRing> {
    let bytes =
        std::fs::read(path).with_context(|| format!("read keyring at {}", path.display()))?;
    let kr: KeyRing = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse keyring at {}", path.display()))?;
    if kr.format_version > KEYRING_FORMAT_VERSION {
        return Err(anyhow!(
            "keyring format_version {} is newer than supported {}",
            kr.format_version,
            KEYRING_FORMAT_VERSION
        ));
    }
    Ok(kr)
}

/// Atomically save the keyring file (write tmp + rename).
pub fn save_keyring(path: &Path, kr: &KeyRing) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create keyring parent {}", parent.display()))?;
    }
    let mut tmp_name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from(KEYRING_FILE_NAME));
    tmp_name.push(format!(".{}.tmp", ulid_like()));
    let tmp = path.with_file_name(tmp_name);
    let bytes = serde_json::to_vec_pretty(kr)?;
    std::fs::write(&tmp, &bytes).with_context(|| format!("write tmp {}", tmp.display()))?;
    // Best-effort restrictive permissions (0o600) before publishing.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(&tmp, perms);
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Unlock the master key. Looks at the descriptor's kind:
///   - Passphrase: derive Argon2id from `passphrase` + stored salt.
///   - OsKeychain: v1 fallthrough to passphrase (operator must
///                 supply one even on platforms where keychain
///                 hooks are not yet wired). Returns error if
///                 passphrase is None.
pub fn unlock_master_key(
    descriptor: &MasterKeyDescriptor,
    passphrase: Option<&str>,
) -> Result<MasterKey> {
    match descriptor.kind {
        MasterKeyKind::Passphrase | MasterKeyKind::OsKeychain => {
            let pass =
                passphrase.ok_or_else(|| anyhow!("keyring requires passphrase to unlock"))?;
            let salt = b64_decode(&descriptor.salt_b64)?;
            derive_master_key_from_passphrase(pass, &salt)
        }
    }
}

/// Initialise a fresh keyring with an empty entry list. Kind defaults
/// to passphrase. Does NOT write to disk.
pub fn fresh_keyring(passphrase_kind: MasterKeyKind) -> KeyRing {
    let salt = fresh_salt();
    let descriptor = MasterKeyDescriptor {
        kind: passphrase_kind,
        salt_b64: b64_encode(&salt),
        wrapped_dek_b64: None,
    };
    let ring_id = ulid_like();
    KeyRing::empty(ring_id, descriptor)
}

/// ULID-style monotonic-ish identifier. v1 uses a 26-char timestamp
/// + random suffix; full ULID compliance is non-essential here.
pub fn ulid_like() -> String {
    use rand::RngCore;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut rand_bytes = [0u8; 8];
    rand::thread_rng().fill_bytes(&mut rand_bytes);
    format!(
        "01H{:013X}{}",
        now_ms,
        hex::encode(rand_bytes).to_uppercase()
    )
}

/// AAD used for AES-GCM seal/open. Binds ciphertext to the key entry
/// (so cross-entry replay fails) and to the keyring file format.
fn entry_aad(ring_id: &str, key_id: &str) -> Vec<u8> {
    format!("easynet/keyring/v1/{ring_id}/{key_id}").into_bytes()
}

/// Create a new active ed25519 entry under the master key. Returns
/// the populated Entry (caller writes it back into the ring).
pub fn build_entry(
    master: &MasterKey,
    ring_id: &str,
    purpose: &str,
    bound_subject: Option<String>,
    rotated_from: Option<String>,
    rotation_epoch: u64,
) -> Result<Entry> {
    let (seed, pk) = fresh_ed25519_keypair()?;
    let key_id = ulid_like();
    let aad = entry_aad(ring_id, &key_id);
    let wrapped = aead_encrypt(master, &seed, &aad)?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    Ok(Entry {
        key_id,
        algo: "ed25519".into(),
        purpose: purpose.into(),
        public_key_b64: b64_encode(&pk),
        private_key_ciphertext_b64: b64_encode(&wrapped.bytes),
        status: KeyStatus::Active,
        rotation_epoch,
        bound_subject,
        rotated_from,
        created_unix_ms: now_ms,
        expires_unix_ms: None,
        revoked_unix_ms: None,
    })
}

/// Decrypt + sign with an entry's private key. Aborts if the entry
/// is not Active.
pub fn sign_with_entry(
    master: &MasterKey,
    ring_id: &str,
    entry: &Entry,
    payload: &[u8],
) -> Result<[u8; 64]> {
    if entry.status != KeyStatus::Active {
        return Err(anyhow!(
            "key {} is not active (status={:?})",
            entry.key_id,
            entry.status
        ));
    }
    let aad = entry_aad(ring_id, &entry.key_id);
    let wrapped = WrappedSecret {
        bytes: b64_decode(&entry.private_key_ciphertext_b64)?,
    };
    let seed = aead_decrypt(master, &wrapped, &aad)?;
    if seed.len() != 32 {
        return Err(anyhow!(
            "decrypted seed length {} != 32 — corrupted entry",
            seed.len()
        ));
    }
    ed25519_sign(&seed, payload)
}

/// Project an entry's public key bytes (decode from base64).
pub fn entry_public_key(entry: &Entry) -> Result<Vec<u8>> {
    b64_decode(&entry.public_key_b64)
}

/// Project an entry's fingerprint (sha256(public_key)).
pub fn entry_fingerprint(entry: &Entry) -> Result<[u8; 32]> {
    let pk = entry_public_key(entry)?;
    Ok(fingerprint(&pk))
}

/// Project a peer's fingerprint.
pub fn peer_fingerprint(peer: &PeerEntry) -> Result<[u8; 32]> {
    let pk = b64_decode(&peer.public_key_b64)?;
    Ok(fingerprint(&pk))
}

/// Validate that the supplied fingerprint matches sha256(public_key).
pub fn validate_fingerprint(public_key_b64: &str, fingerprint_b64: &str) -> Result<()> {
    let pk = b64_decode(public_key_b64)?;
    let got = fingerprint(&pk);
    let expected = b64_decode(fingerprint_b64)?;
    if got.as_slice() != expected.as_slice() {
        return Err(anyhow!(
            "fingerprint mismatch: provided fingerprint does not equal sha256(public_key)"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unlock_test(kr: &KeyRing) -> MasterKey {
        unlock_master_key(&kr.master_key, Some("test-pass")).unwrap()
    }

    #[test]
    fn fresh_keyring_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keyring.json");
        let mut kr = fresh_keyring(MasterKeyKind::Passphrase);
        save_keyring(&path, &kr).unwrap();
        let loaded = load_keyring(&path).unwrap();
        assert_eq!(loaded.ring_id, kr.ring_id);
        assert_eq!(loaded.format_version, KEYRING_FORMAT_VERSION);
        assert!(loaded.entries.is_empty());

        // Add an entry, save, reload.
        let master = unlock_test(&kr);
        let entry = build_entry(&master, &kr.ring_id, "agent_signing", None, None, 0).unwrap();
        kr.entries.push(entry.clone());
        save_keyring(&path, &kr).unwrap();
        let reloaded = load_keyring(&path).unwrap();
        assert_eq!(reloaded.entries.len(), 1);
        assert_eq!(reloaded.entries[0].key_id, entry.key_id);
    }

    #[test]
    fn concurrent_saves_do_not_share_the_same_tmp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::sync::Arc::new(dir.path().join("keyring.json"));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();

        for _ in 0..2 {
            let path = std::sync::Arc::clone(&path);
            let barrier = std::sync::Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                let kr = fresh_keyring(MasterKeyKind::Passphrase);
                barrier.wait();
                save_keyring(&path, &kr)
            }));
        }

        for handle in handles {
            handle.join().unwrap().unwrap();
        }
        load_keyring(&path).unwrap();
    }

    #[test]
    fn entry_signs_and_round_trips() {
        let kr = fresh_keyring(MasterKeyKind::Passphrase);
        let master = unlock_test(&kr);
        let entry = build_entry(&master, &kr.ring_id, "agent_signing", None, None, 0).unwrap();
        let sig = sign_with_entry(&master, &kr.ring_id, &entry, b"hello").unwrap();
        assert_eq!(sig.len(), 64);

        // Verify signature externally:
        use ed25519_dalek::{Verifier, VerifyingKey};
        let pk = entry_public_key(&entry).unwrap();
        let pk_arr: [u8; 32] = pk.try_into().unwrap();
        let vk = VerifyingKey::from_bytes(&pk_arr).unwrap();
        let sig_obj = ed25519_dalek::Signature::from_bytes(&sig);
        assert!(vk.verify(b"hello", &sig_obj).is_ok());
    }

    #[test]
    fn cannot_sign_with_retired_entry() {
        let kr = fresh_keyring(MasterKeyKind::Passphrase);
        let master = unlock_test(&kr);
        let mut entry = build_entry(&master, &kr.ring_id, "agent_signing", None, None, 0).unwrap();
        entry.status = KeyStatus::Retired;
        let err = sign_with_entry(&master, &kr.ring_id, &entry, b"hello").unwrap_err();
        assert!(err.to_string().contains("not active"));
    }

    #[test]
    fn fingerprint_validation_round_trips() {
        let kr = fresh_keyring(MasterKeyKind::Passphrase);
        let master = unlock_test(&kr);
        let entry = build_entry(&master, &kr.ring_id, "agent_signing", None, None, 0).unwrap();
        let fp = entry_fingerprint(&entry).unwrap();
        let fp_b64 = b64_encode(&fp);
        validate_fingerprint(&entry.public_key_b64, &fp_b64).unwrap();
        // Tampering fingerprint fails.
        let bad = b64_encode(&[0u8; 32]);
        assert!(validate_fingerprint(&entry.public_key_b64, &bad).is_err());
    }

    #[test]
    fn peer_entry_rejects_retired_peer_uri_alias() {
        let value = serde_json::json!({
            "peer_uri": "easynet:///r/realm/device/dev-1",
            "fingerprint_b64": b64_encode(&[0_u8; 32]),
            "public_key_b64": b64_encode(&[1_u8; 32]),
            "status": "trusted",
            "via_hub": null,
            "added_unix_ms": 1,
            "last_seen_unix_ms": 1
        });
        let err = serde_json::from_value::<PeerEntry>(value)
            .expect_err("peer_uri must not deserialize as peer_ura")
            .to_string();
        assert!(
            err.contains("peer_ura"),
            "error should name the canonical keyring peer field: {err}"
        );
    }

    #[test]
    fn unlock_with_wrong_passphrase_does_not_match_seal() {
        let kr = fresh_keyring(MasterKeyKind::Passphrase);
        let mk_correct = unlock_master_key(&kr.master_key, Some("test-pass")).unwrap();
        let mk_wrong = unlock_master_key(&kr.master_key, Some("other-pass")).unwrap();
        let entry = build_entry(&mk_correct, &kr.ring_id, "x", None, None, 0).unwrap();
        // Decrypting with wrong master key fails.
        assert!(sign_with_entry(&mk_wrong, &kr.ring_id, &entry, b"x").is_err());
    }
}
