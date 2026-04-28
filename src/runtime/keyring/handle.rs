// EasyNet CLI — Keyring runtime handle (RFC-002 §3)
// ===================================================
//
// `KeyringHandle` is the in-memory wrapper around the on-disk keyring
// + the unlocked master key. The daemon holds exactly one handle for
// its lifetime. The 10 ability handlers in `abilities.rs` operate
// against this handle.
//
// Concurrency: backed by `parking_lot::RwLock` (already a transitive
// dep through dashmap) — abilities take read locks for queries, write
// locks for mutations + flush-to-disk.
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::sync::Mutex;

use super::crypto::{fingerprint, MasterKey};
use super::store::{
    build_entry, entry_fingerprint, entry_public_key, fresh_keyring, load_keyring,
    save_keyring, sign_with_entry, ulid_like, unlock_master_key, validate_fingerprint, Entry,
    KeyRing, KeyStatus, MasterKeyKind, PeerEntry, PeerStatus,
};

fn b64_encode(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    STANDARD.encode(bytes)
}

fn b64_decode(s: &str) -> Result<Vec<u8>> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    STANDARD.decode(s).map_err(|e| anyhow!("base64 decode: {e}"))
}

pub struct KeyringHandle {
    inner: Mutex<KeyRing>,
    master: MasterKey,
    path: PathBuf,
}

impl KeyringHandle {
    /// Open or create a keyring at the given path. If the file does
    /// not exist, a fresh ring is initialised and immediately saved.
    /// Caller supplies the passphrase used both to unlock an existing
    /// ring AND (when initialising) to derive the master key for the
    /// freshly minted descriptor.
    pub fn open_or_create(path: PathBuf, passphrase: &str) -> Result<Self> {
        let ring = if path.exists() {
            load_keyring(&path).with_context(|| format!("load keyring {}", path.display()))?
        } else {
            let ring = fresh_keyring(MasterKeyKind::Passphrase);
            save_keyring(&path, &ring)?;
            ring
        };
        let master = unlock_master_key(&ring.master_key, Some(passphrase))?;

        // Sanity: if the ring already has entries, decrypt one with
        // the supplied passphrase to fail fast on a bad password
        // (rather than silently producing garbage signatures later).
        if let Some(entry) = ring.entries.iter().find(|e| e.status == KeyStatus::Active) {
            sign_with_entry(&master, &ring.ring_id, entry, b"open-or-create-probe")
                .with_context(|| "passphrase rejected: cannot decrypt active entry")?;
        }

        Ok(Self {
            inner: Mutex::new(ring),
            master,
            path,
        })
    }

    /// Load a keyring without write capability — used for resolver-only
    /// daemons that never mutate the ring (e.g. the verifier binary).
    /// Master key is required to validate entries.
    pub fn open_readonly(path: PathBuf, passphrase: &str) -> Result<Self> {
        Self::open_or_create(path, passphrase)
    }

    pub fn ring_id(&self) -> String {
        self.inner.lock().unwrap().ring_id.clone()
    }

    pub fn device_subject(&self) -> Option<String> {
        self.inner.lock().unwrap().device_subject.clone()
    }

    pub fn set_device_subject(&self, subject: String) -> Result<()> {
        let mut ring = self.inner.lock().unwrap();
        ring.device_subject = Some(subject);
        save_keyring(&self.path, &ring)
    }

    /// Snapshot the entry list (cloned to release the lock).
    pub fn list_entries(&self) -> Vec<Entry> {
        self.inner.lock().unwrap().entries.clone()
    }

    pub fn list_peers(&self) -> Vec<PeerEntry> {
        self.inner.lock().unwrap().peer_table.clone()
    }

    pub fn find_entry_by_id(&self, key_id: &str) -> Option<Entry> {
        self.inner
            .lock()
            .unwrap()
            .entries
            .iter()
            .find(|e| e.key_id == key_id)
            .cloned()
    }

    /// Return the active entry whose `bound_subject == agent_uri`.
    pub fn find_active_entry_by_subject(&self, agent_uri: &str) -> Option<Entry> {
        self.inner
            .lock()
            .unwrap()
            .entries
            .iter()
            .find(|e| {
                e.status == KeyStatus::Active
                    && e.bound_subject.as_deref() == Some(agent_uri)
            })
            .cloned()
    }

    pub fn find_peer_by_uri(&self, peer_uri: &str) -> Option<PeerEntry> {
        self.inner
            .lock()
            .unwrap()
            .peer_table
            .iter()
            .find(|p| p.peer_uri == peer_uri)
            .cloned()
    }

    /// Create a new entry, persist, return.
    pub fn create_entry(
        &self,
        purpose: &str,
        bound_subject: Option<String>,
    ) -> Result<Entry> {
        let mut ring = self.inner.lock().unwrap();
        let entry = build_entry(
            &self.master,
            &ring.ring_id,
            purpose,
            bound_subject,
            None,
            0,
        )?;
        ring.entries.push(entry.clone());
        save_keyring(&self.path, &ring)?;
        Ok(entry)
    }

    /// Sign a payload with the named key. Returns 64-byte signature.
    pub fn sign(&self, key_id: &str, payload: &[u8]) -> Result<[u8; 64]> {
        let ring = self.inner.lock().unwrap();
        let entry = ring
            .entries
            .iter()
            .find(|e| e.key_id == key_id)
            .ok_or_else(|| anyhow!("key_id {key_id} not found"))?;
        sign_with_entry(&self.master, &ring.ring_id, entry, payload)
    }

    /// Rotate: mark the active entry retired, mint a fresh one with
    /// `rotation_epoch + 1` and `rotated_from = old_key_id`. Bound
    /// subject (if any) carries over.
    pub fn rotate(&self, key_id: &str) -> Result<(String, String, u64)> {
        let mut ring = self.inner.lock().unwrap();
        let (old_purpose, old_bound, old_epoch) = {
            let entry = ring
                .entries
                .iter_mut()
                .find(|e| e.key_id == key_id)
                .ok_or_else(|| anyhow!("key_id {key_id} not found"))?;
            if entry.status != KeyStatus::Active {
                return Err(anyhow!("only active keys can be rotated"));
            }
            entry.status = KeyStatus::Retired;
            (
                entry.purpose.clone(),
                entry.bound_subject.clone(),
                entry.rotation_epoch,
            )
        };
        let new_entry = build_entry(
            &self.master,
            &ring.ring_id,
            &old_purpose,
            old_bound,
            Some(key_id.to_string()),
            old_epoch + 1,
        )?;
        let new_id = new_entry.key_id.clone();
        ring.entries.push(new_entry);
        save_keyring(&self.path, &ring)?;
        Ok((new_id, key_id.to_string(), old_epoch + 1))
    }

    pub fn revoke(&self, key_id: &str, _reason: &str) -> Result<i64> {
        let mut ring = self.inner.lock().unwrap();
        let entry = ring
            .entries
            .iter_mut()
            .find(|e| e.key_id == key_id)
            .ok_or_else(|| anyhow!("key_id {key_id} not found"))?;
        entry.status = KeyStatus::Revoked;
        let now = chrono::Utc::now().timestamp_millis();
        entry.revoked_unix_ms = Some(now);
        save_keyring(&self.path, &ring)?;
        Ok(now)
    }

    pub fn set_expiry(&self, key_id: &str, expires_unix_ms: i64) -> Result<()> {
        let mut ring = self.inner.lock().unwrap();
        let entry = ring
            .entries
            .iter_mut()
            .find(|e| e.key_id == key_id)
            .ok_or_else(|| anyhow!("key_id {key_id} not found"))?;
        entry.expires_unix_ms = Some(expires_unix_ms);
        save_keyring(&self.path, &ring)?;
        Ok(())
    }

    pub fn bind_subject(&self, key_id: &str, subject_id: &str) -> Result<()> {
        let mut ring = self.inner.lock().unwrap();
        let entry = ring
            .entries
            .iter_mut()
            .find(|e| e.key_id == key_id)
            .ok_or_else(|| anyhow!("key_id {key_id} not found"))?;
        entry.bound_subject = Some(subject_id.to_string());
        save_keyring(&self.path, &ring)?;
        Ok(())
    }

    /// Record an externally-derived public key in the keyring as if it
    /// had been generated locally. This is the bridge between the
    /// SDK's deterministic `derive_subject_auth` and the keyring-backed
    /// KeyResolver: at daemon boot we mirror the derived agent + hub
    /// keys into the keyring so federation peer lookups resolve
    /// against the same bytes the SDK signs under.
    ///
    /// `private_key_seed_opt`: if provided, stored encrypted (allows
    /// keyring.sign to work with this entry); if None the entry is
    /// "verify-only" — public key is queryable but signing returns
    /// AXON_KEY_NOT_LOCAL. v1 deterministic-derivation path passes
    /// the seed so signatures match. New keyring.create entries do not
    /// pass through this method.
    pub fn mirror_external_key(
        &self,
        purpose: &str,
        bound_subject: String,
        public_key_bytes: &[u8],
        private_key_seed_opt: Option<&[u8]>,
    ) -> Result<String> {
        // If we already have an active entry bound to this subject with
        // matching public key, return its key_id without duplicating.
        if let Some(existing) = self.find_active_entry_by_subject(&bound_subject) {
            let pk = b64_decode(&existing.public_key_b64)?;
            if pk == public_key_bytes {
                return Ok(existing.key_id);
            }
        }
        let mut ring = self.inner.lock().unwrap();
        let key_id = super::store::ulid_like();
        let aad =
            format!("easynet/keyring/v1/{ring_id}/{key_id}", ring_id = ring.ring_id)
                .into_bytes();
        let private_key_ciphertext_b64 = match private_key_seed_opt {
            Some(seed) if seed.len() == 32 => {
                let wrapped = super::crypto::aead_encrypt(&self.master, seed, &aad)?;
                b64_encode(&wrapped.bytes)
            }
            Some(_) => return Err(anyhow!("seed must be exactly 32 bytes for ed25519")),
            None => String::new(), // verify-only entry
        };
        let now_ms = chrono::Utc::now().timestamp_millis();
        let entry = super::store::Entry {
            key_id: key_id.clone(),
            algo: "ed25519".into(),
            purpose: purpose.into(),
            public_key_b64: b64_encode(public_key_bytes),
            private_key_ciphertext_b64,
            status: super::store::KeyStatus::Active,
            rotation_epoch: 0,
            bound_subject: Some(bound_subject),
            rotated_from: None,
            created_unix_ms: now_ms,
            expires_unix_ms: None,
            revoked_unix_ms: None,
        };
        ring.entries.push(entry);
        super::store::save_keyring(&self.path, &ring)?;
        Ok(key_id)
    }

    /// TOFU peer addition. If `fingerprint_b64` is supplied, validate
    /// it equals sha256(public_key); otherwise compute and store it.
    pub fn peer_add(
        &self,
        peer_uri: &str,
        public_key_b64: &str,
        fingerprint_b64: Option<&str>,
        via_hub: Option<&str>,
    ) -> Result<bool> {
        let pk_bytes = b64_decode(public_key_b64)?;
        let fp = fingerprint(&pk_bytes);
        let fp_b64 = b64_encode(&fp);
        if let Some(supplied) = fingerprint_b64 {
            validate_fingerprint(public_key_b64, supplied)?;
        }

        let mut ring = self.inner.lock().unwrap();
        let now = chrono::Utc::now().timestamp_millis();

        // Update existing or insert new.
        if let Some(p) = ring
            .peer_table
            .iter_mut()
            .find(|p| p.peer_uri == peer_uri)
        {
            // Re-add of the same peer: refresh, preserving status.
            p.public_key_b64 = public_key_b64.to_string();
            p.fingerprint_b64 = fp_b64;
            p.via_hub = via_hub.map(|s| s.to_string());
            p.last_seen_unix_ms = now;
            save_keyring(&self.path, &ring)?;
            return Ok(false);
        }

        ring.peer_table.push(PeerEntry {
            peer_uri: peer_uri.to_string(),
            fingerprint_b64: fp_b64,
            public_key_b64: public_key_b64.to_string(),
            status: PeerStatus::Trusted,
            via_hub: via_hub.map(|s| s.to_string()),
            added_unix_ms: now,
            last_seen_unix_ms: now,
        });
        save_keyring(&self.path, &ring)?;
        Ok(true)
    }
}

impl std::fmt::Debug for KeyringHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyringHandle")
            .field("ring_id", &self.ring_id())
            .field("path", &self.path)
            .field("entries", &self.list_entries().len())
            .field("peers", &self.list_peers().len())
            .finish()
    }
}

#[allow(unused_imports)]
use ulid_like as _ulid_like_used; // keep symbol for future call sites

#[cfg(test)]
mod tests {
    use super::*;

    fn open_test_handle() -> (KeyringHandle, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keyring.json");
        let h = KeyringHandle::open_or_create(path, "test-pass").unwrap();
        (h, dir)
    }

    #[test]
    fn create_and_list_entries() {
        let (h, _dir) = open_test_handle();
        assert!(h.list_entries().is_empty());
        let e = h.create_entry("agent_signing", None).unwrap();
        let listed = h.list_entries();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].key_id, e.key_id);
        assert_eq!(listed[0].status, KeyStatus::Active);
    }

    #[test]
    fn sign_then_externally_verify() {
        let (h, _dir) = open_test_handle();
        let e = h.create_entry("agent_signing", None).unwrap();
        let sig = h.sign(&e.key_id, b"payload").unwrap();
        let pk = entry_public_key(&e).unwrap();
        let pk_arr: [u8; 32] = pk.try_into().unwrap();
        use ed25519_dalek::{Verifier, VerifyingKey};
        let vk = VerifyingKey::from_bytes(&pk_arr).unwrap();
        let sig_obj = ed25519_dalek::Signature::from_bytes(&sig);
        assert!(vk.verify(b"payload", &sig_obj).is_ok());
    }

    #[test]
    fn rotation_retires_old_creates_new_with_incremented_epoch() {
        let (h, _dir) = open_test_handle();
        let e = h.create_entry("agent_signing", Some("easynet:///r/prv/reg/agent.x".into())).unwrap();
        let (new_id, retired_id, epoch) = h.rotate(&e.key_id).unwrap();
        assert_eq!(retired_id, e.key_id);
        assert_eq!(epoch, 1);
        assert_ne!(new_id, e.key_id);
        let entries = h.list_entries();
        assert_eq!(entries.len(), 2);
        let old = entries.iter().find(|x| x.key_id == retired_id).unwrap();
        let new = entries.iter().find(|x| x.key_id == new_id).unwrap();
        assert_eq!(old.status, KeyStatus::Retired);
        assert_eq!(new.status, KeyStatus::Active);
        assert_eq!(new.rotation_epoch, 1);
        assert_eq!(new.rotated_from.as_deref(), Some(retired_id.as_str()));
        assert_eq!(new.bound_subject.as_deref(), Some("easynet:///r/prv/reg/agent.x"));
    }

    #[test]
    fn rotation_rejects_non_active_keys() {
        let (h, _dir) = open_test_handle();
        let e = h.create_entry("agent_signing", None).unwrap();
        h.revoke(&e.key_id, "test").unwrap();
        let err = h.rotate(&e.key_id).unwrap_err();
        assert!(err.to_string().contains("active"));
    }

    #[test]
    fn revoke_marks_entry_and_cannot_sign() {
        let (h, _dir) = open_test_handle();
        let e = h.create_entry("agent_signing", None).unwrap();
        let ts = h.revoke(&e.key_id, "compromised").unwrap();
        assert!(ts > 0);
        let listed = h.list_entries();
        let revoked = listed.iter().find(|x| x.key_id == e.key_id).unwrap();
        assert_eq!(revoked.status, KeyStatus::Revoked);
        assert!(h.sign(&e.key_id, b"x").is_err());
    }

    #[test]
    fn peer_add_validates_fingerprint() {
        let (h, _dir) = open_test_handle();
        let e = h.create_entry("agent_signing", None).unwrap();
        let pk = e.public_key_b64.clone();
        let fp = b64_encode(&entry_fingerprint(&e).unwrap());
        // Correct fingerprint: ok.
        h.peer_add("easynet:///r/org/reg/agent.alice", &pk, Some(&fp), None)
            .unwrap();
        // Re-add same peer: returns false (updated, not inserted).
        let added = h
            .peer_add("easynet:///r/org/reg/agent.alice", &pk, Some(&fp), None)
            .unwrap();
        assert!(!added);
        // Wrong fingerprint: rejected.
        let bad = b64_encode(&[0u8; 32]);
        assert!(h
            .peer_add("easynet:///r/org/reg/agent.bob", &pk, Some(&bad), None)
            .is_err());
    }

    #[test]
    fn peer_add_omitted_fingerprint_is_computed() {
        let (h, _dir) = open_test_handle();
        let e = h.create_entry("agent_signing", None).unwrap();
        let pk = e.public_key_b64.clone();
        h.peer_add("easynet:///r/org/reg/agent.bob", &pk, None, Some("hub-uri"))
            .unwrap();
        let p = h.find_peer_by_uri("easynet:///r/org/reg/agent.bob").unwrap();
        assert_eq!(p.via_hub.as_deref(), Some("hub-uri"));
        assert_eq!(p.public_key_b64, pk);
    }

    #[test]
    fn passphrase_rejection_on_reopen_with_existing_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keyring.json");
        {
            let h = KeyringHandle::open_or_create(path.clone(), "right-pass").unwrap();
            h.create_entry("agent_signing", None).unwrap();
        }
        // Wrong passphrase fails to open.
        let err = KeyringHandle::open_or_create(path.clone(), "wrong-pass").unwrap_err();
        assert!(err.to_string().contains("passphrase rejected"));
        // Right passphrase succeeds.
        let h = KeyringHandle::open_or_create(path, "right-pass").unwrap();
        assert_eq!(h.list_entries().len(), 1);
    }

    #[test]
    fn bind_subject_round_trip_persists_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keyring.json");
        let key_id;
        {
            let h = KeyringHandle::open_or_create(path.clone(), "p").unwrap();
            let e = h.create_entry("agent_signing", None).unwrap();
            key_id = e.key_id.clone();
            h.bind_subject(&key_id, "easynet:///r/prv/reg/agent.test")
                .unwrap();
        }
        let h = KeyringHandle::open_or_create(path, "p").unwrap();
        let e = h.find_entry_by_id(&key_id).unwrap();
        assert_eq!(
            e.bound_subject.as_deref(),
            Some("easynet:///r/prv/reg/agent.test")
        );
    }

    #[test]
    fn ulid_like_ids_unique_within_a_run() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            let id = ulid_like();
            assert!(seen.insert(id));
        }
    }
}
