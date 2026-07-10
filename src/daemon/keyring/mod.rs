// EasyNet CLI - daemon keyring device identity vault
// =======================================================
//
// File: src/daemon/keyring/mod.rs
//
// RFC-001 plan v4.1.5 Phase 3A. The keyring is a process-external
// vault for the Ed25519 private key(s) this device signs as. Every
// EasyNet process on a given host (backend hub-role, daemon
// device-role, CLI agent-role) shares one device-level secret —
// the role overlays (`HubURA(realm)` vs `DeviceURA(realm, uuid)`)
// fan out from the *same* keypair, anchoring "this physical
// machine" as the load-bearing identity unit.
//
// Why a separate process
// ----------------------
// 1. Process isolation. Backend / daemon / CLI never hold the raw
//    seed bytes — they request `sign(self_ura, canonical_bytes)`
//    over a UDS socket and receive a 64-byte signature back. A
//    compromised callee cannot exfiltrate the key.
// 2. ACL surface. UDS file mode 0600 + per-user owner is the
//    minimum-credible boundary. macOS Keychain / libsecret /
//    Credential Manager wrap a fancier ACL but couple us to OS
//    semantics that break on headless Linux / Docker — the path
//    EasyNet production deploys actually run on. We ship our own.
// 3. EasyNet-native. The "vault" entity belongs in EasyNet's own
//    ontology, not borrowed from the host OS. RFC-001 ratify §3.5
//    (planned in v4.1.5) places the keyring at the storage layer
//    underneath the six URA roles — every URA's signing path
//    flows through it.
//
// What this module IS
// -------------------
// A pure-data + crypto core that implements:
//   * `KeyringFile` — the on-disk encrypted blob (aes-gcm-256
//     framed by a randomly-generated nonce) holding a list of
//     `KeyringEntry { primary_self, role_overlays, sealed_seed }`
//     records.
//   * `MasterKey` — the symmetric key the file is sealed under,
//     derived from a passphrase via Argon2id with a per-file salt.
//   * `Vault` — the in-memory open form: master key + decrypted
//     entries. Provides `put / sign / derive_pubkey / list / forget`
//     against the entry list, and `seal()` to write back to disk.
//   * `MasterKeySource` — passphrase prompt vs `EASYNET_KEYRING_PASSPHRASE`
//     env vs explicit-bytes (tests).
//
// What this module is NOT
// -----------------------
// - Not a UDS server. The bin (`src/bin/easynet-keyring.rs`) wraps
//   `Vault` in a `tokio::net::UnixListener` accept loop; this file
//   stays sync + transport-free so it's trivially unit-testable.
// - Not a key generator. Pairing flow (`validatePairingLogic`,
//   `easynet device join`) mints the seed. The keyring just
//   stores + signs.
// - Not a client. Phase 3B's `crate::daemon::identity::self_identity`
//   layers a typed client on top of the wire protocol this
//   module's serde shapes pin.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, Version};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};

pub mod abilities;
pub mod bridge_forward;
pub mod federated_bindings;
pub mod forward;
pub mod resolver;
pub mod user_binding_chain;

/// Ed25519 seed length (32 bytes per RFC 8032 §5.1.5).
pub const ED25519_SEED_LEN: usize = 32;

/// AES-256-GCM key length (32 bytes).
pub const AES_KEY_LEN: usize = 32;

/// AES-GCM nonce length (12 bytes per NIST SP 800-38D §5.2.1.1).
pub const AES_NONCE_LEN: usize = 12;

/// Argon2id salt length. 16 bytes is what `password_hash`'s
/// `SaltString::generate` defaults to; we keep it explicit so a
/// future bump to 32 is one constant.
pub const KDF_SALT_LEN: usize = 16;

/// Argon2id memory cost (KiB). 64 MiB matches the OWASP 2024
/// "minimum for interactive passphrase" recommendation. The
/// passphrase-prompt path tolerates ~150 ms; the env-injected path
/// pays this cost once at boot.
pub const KDF_MEMORY_KIB: u32 = 64 * 1024;

/// Argon2id iteration count.
pub const KDF_TIME_COST: u32 = 3;

/// Argon2id parallelism. 1 keeps the derivation single-threaded —
/// the keyring daemon blocks during boot anyway, no benefit from
/// fanning out.
pub const KDF_PARALLELISM: u32 = 1;

/// Persisted vault file structure. v1 layout:
///
/// ```text
/// {
///   "version": 1,
///   "kdf_salt_b64": "<base64 of 16 random bytes>",
///   "vault_nonce_b64": "<base64 of 12 random bytes>",
///   "vault_ciphertext_b64": "<base64 of aes-gcm(plaintext) with auth tag>"
/// }
/// ```
///
/// `vault_ciphertext_b64` decrypts to the JSON-serialised
/// `VaultPlaintext`. The outer file is therefore safe to read by
/// any process — the master key passphrase is the access gate.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct KeyringFile {
    pub version: u32,
    pub kdf_salt_b64: String,
    pub vault_nonce_b64: String,
    pub vault_ciphertext_b64: String,
}

impl KeyringFile {
    /// Current on-disk format version. Bumping this requires a
    /// migration path tested in `migrations_v*` modules.
    pub const CURRENT_VERSION: u32 = 1;
}

/// Plaintext form of the vault. JSON-serialised inside
/// `KeyringFile::vault_ciphertext_b64`.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct VaultPlaintext {
    pub entries: Vec<KeyringEntry>,
    /// The rotatable, subject-bound signing inventory.  It is stored in the
    /// same encrypted daemon vault as runtime identities but deliberately has
    /// a different record model and lifecycle.
    #[serde(default)]
    pub managed_signing: ManagedSigningInventory,
}

/// One key entry. `primary_self` is the canonical URA this key was
/// minted for (per CTO ratify, the device-role URA on this host).
/// `role_overlays` lists every other URA this same keypair signs as
/// — RFC-001 v4.1.5 §3.5: "role overlays share the underlying
/// keypair so the host's identity is unitary across roles".
///
/// `seed_hex` is the 32-byte Ed25519 seed in lowercase hex. It
/// only appears inside the encrypted blob — the unencrypted
/// `KeyringFile` never carries it.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct KeyringEntry {
    pub primary_self: String,
    #[serde(default)]
    pub role_overlays: Vec<String>,
    pub seed_hex: String,
}

/// Lifecycle status of a subject-bound managed signing key.
///
/// The state graph is `active -> retired -> revoked` plus
/// `active -> revoked`.  It is intentionally not shared with runtime identity
/// records: runtime identity is an ownership anchor, while managed signing is
/// an explicitly rotatable policy inventory.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedSigningStatus {
    Active,
    Retired,
    Revoked,
}

/// Encrypted private record for one managed signing key. `seed_hex` never
/// crosses the daemon protocol; it exists only inside `VaultPlaintext`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ManagedSigningKey {
    pub key_id: String,
    pub purpose: String,
    pub public_key_b64: String,
    pub seed_hex: String,
    pub status: ManagedSigningStatus,
    pub rotation_epoch: u64,
    pub bound_subject: Option<String>,
    pub rotated_from: Option<String>,
    pub created_unix_ms: i64,
    pub expires_unix_ms: Option<i64>,
    pub revoked_unix_ms: Option<i64>,
}

/// Public projection of a managed signing key. This is the only managed key
/// shape exposed by the daemon protocol and SDK consumers.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ManagedSigningKeyProjection {
    pub key_id: String,
    pub purpose: String,
    pub public_key_b64: String,
    pub status: ManagedSigningStatus,
    pub rotation_epoch: u64,
    pub bound_subject: Option<String>,
    /// Daemon-derived policy binding for this key and its immutable subject.
    /// It is absent until the key is subject-bound.
    pub signer_policy_ref: Option<String>,
    pub rotated_from: Option<String>,
    pub created_unix_ms: i64,
    pub expires_unix_ms: Option<i64>,
    pub revoked_unix_ms: Option<i64>,
}

impl From<&ManagedSigningKey> for ManagedSigningKeyProjection {
    fn from(key: &ManagedSigningKey) -> Self {
        Self {
            key_id: key.key_id.clone(),
            purpose: key.purpose.clone(),
            public_key_b64: key.public_key_b64.clone(),
            status: key.status,
            rotation_epoch: key.rotation_epoch,
            bound_subject: key.bound_subject.clone(),
            signer_policy_ref: key.bound_subject.as_ref().map(|subject_ura| {
                crate::protocol::identity_contract::signer_policy_ref(
                    subject_ura,
                    &key.key_id,
                    &key.public_key_b64,
                )
            }),
            rotated_from: key.rotated_from.clone(),
            created_unix_ms: key.created_unix_ms,
            expires_unix_ms: key.expires_unix_ms,
            revoked_unix_ms: key.revoked_unix_ms,
        }
    }
}

/// A trusted public peer projection used by federation resolution. It has no
/// relationship to local private-key lifecycle beyond sharing the service's
/// custody and audit boundary.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ManagedPeer {
    pub peer_ura: String,
    pub fingerprint_b64: String,
    pub public_key_b64: String,
    pub via_hub: Option<String>,
    pub added_unix_ms: i64,
    pub last_seen_unix_ms: i64,
}

/// Managed-signing domain persisted inside the daemon vault.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct ManagedSigningInventory {
    #[serde(default)]
    keys: Vec<ManagedSigningKey>,
    #[serde(default)]
    peers: Vec<ManagedPeer>,
}

/// Errors surfaced by the vault crypto layer. Wire layer (the
/// daemon) maps these to typed JSON error responses.
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("keyring io: {0}")]
    Io(#[from] io::Error),
    #[error("keyring serde: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("keyring kdf: {0}")]
    Kdf(String),
    #[error("keyring crypto: {0}")]
    Crypto(String),
    #[error("keyring base64: {0}")]
    Base64(String),
    #[error("keyring entry not found: {0}")]
    NotFound(String),
    #[error("keyring entry already exists: {0}")]
    AlreadyExists(String),
    #[error("keyring corrupt: {0}")]
    Corrupt(String),
    #[error("keyring seed length: expected {ED25519_SEED_LEN}, got {got}")]
    BadSeedLen { got: usize },
    #[error("managed signing lifecycle: {0}")]
    Lifecycle(String),
    #[error("managed signing policy: {0}")]
    Policy(String),
}

/// Where the master key passphrase comes from. The v1 vault
/// supports two production-credible sources and one test-only
/// source.
#[derive(Debug, Clone)]
pub enum MasterKeySource {
    /// Read from `EASYNET_KEYRING_PASSPHRASE`. The deployment
    /// pipeline is responsible for injecting it (systemd
    /// LoadCredentialEncrypted, Vault sidecar, k8s secret mount).
    Env,
    /// Take the passphrase verbatim. Used by tests and by the
    /// interactive boot path that prompts the operator.
    Explicit(String),
    /// Derived master key bytes are supplied directly. Reserved
    /// for hardware-root setups (TPM-derived secret) — not used
    /// in v1 but kept in the enum so adding it later is not a
    /// breaking change.
    PreDerived([u8; AES_KEY_LEN]),
}

impl MasterKeySource {
    /// Read the env var. Errors when unset — a missing master key
    /// is fatal at boot, not silently fallback (we do not want
    /// production to ever silently downgrade to no-encryption).
    pub fn from_env() -> Result<Self, VaultError> {
        match std::env::var("EASYNET_KEYRING_PASSPHRASE") {
            Ok(s) if !s.is_empty() => Ok(MasterKeySource::Env),
            _ => Err(VaultError::Kdf(
                "EASYNET_KEYRING_PASSPHRASE is unset or empty".into(),
            )),
        }
    }

    /// Resolve to the actual passphrase string (or pre-derived
    /// bytes). Called once per `Vault::open`.
    fn resolve_passphrase(&self) -> Result<Option<String>, VaultError> {
        match self {
            MasterKeySource::Env => match std::env::var("EASYNET_KEYRING_PASSPHRASE") {
                Ok(s) if !s.is_empty() => Ok(Some(s)),
                _ => Err(VaultError::Kdf(
                    "EASYNET_KEYRING_PASSPHRASE is unset or empty".into(),
                )),
            },
            MasterKeySource::Explicit(s) => Ok(Some(s.clone())),
            MasterKeySource::PreDerived(_) => Ok(None),
        }
    }

    fn pre_derived(&self) -> Option<[u8; AES_KEY_LEN]> {
        match self {
            MasterKeySource::PreDerived(k) => Some(*k),
            _ => None,
        }
    }
}

/// Argon2id derive `(salt, passphrase) → 32-byte key`.
fn derive_master_key(passphrase: &str, salt: &[u8]) -> Result<[u8; AES_KEY_LEN], VaultError> {
    let params = Params::new(
        KDF_MEMORY_KIB,
        KDF_TIME_COST,
        KDF_PARALLELISM,
        Some(AES_KEY_LEN),
    )
    .map_err(|e| VaultError::Kdf(format!("argon2 params: {e}")))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; AES_KEY_LEN];
    argon
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|e| VaultError::Kdf(format!("argon2 hash: {e}")))?;
    Ok(out)
}

/// In-memory open vault. Constructed by `open_or_init`; mutated
/// by `put / forget`; queried by `sign / derive_pubkey / list`;
/// persisted by `seal`.
///
/// Holds the master key and the decrypted entries. Drops zeroize
/// the master key automatically at scope exit (best-effort —
/// Rust's heap allocator may copy bytes around, the only hard
/// guarantee comes from process isolation).
pub struct Vault {
    path: PathBuf,
    master_key: [u8; AES_KEY_LEN],
    salt: [u8; KDF_SALT_LEN],
    entries: HashMap<String, KeyringEntry>,
    managed_signing: ManagedSigningInventory,
}

// Manual `Debug` so the master key never lands in a log line. The
// derived `Debug` would print every byte of `master_key` —
// catastrophic for ops who tail the daemon log. We keep `path` and
// the entry URAs (no seeds) which is plenty for an operator
// debugging "which vault did this process open and what entries
// are in it".
impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("path", &self.path)
            .field("master_key", &"<redacted>")
            .field("salt_len", &self.salt.len())
            .field("entry_uras", &self.entries.keys().collect::<Vec<_>>())
            .field(
                "managed_signing_key_count",
                &self.managed_signing.keys.len(),
            )
            .field("managed_peer_count", &self.managed_signing.peers.len())
            .finish()
    }
}

impl Vault {
    /// Open the vault file at `path`, decrypt with the master key
    /// derived from `source`. If the file does not exist, mint a
    /// fresh empty vault under a freshly-generated salt — the
    /// first `put` then writes it. This is the convenience boot
    /// path for "first time on this host".
    pub fn open_or_init(path: &Path, source: &MasterKeySource) -> Result<Self, VaultError> {
        if path.exists() {
            Self::open(path, source)
        } else {
            Self::init(path, source)
        }
    }

    /// Open an existing vault file.
    pub fn open(path: &Path, source: &MasterKeySource) -> Result<Self, VaultError> {
        let raw = fs::read_to_string(path)?;
        let file: KeyringFile = serde_json::from_str(&raw)
            .map_err(|e| VaultError::Corrupt(format!("parse {}: {e}", path.display())))?;
        if file.version != KeyringFile::CURRENT_VERSION {
            return Err(VaultError::Corrupt(format!(
                "unsupported keyring version {} (expected {})",
                file.version,
                KeyringFile::CURRENT_VERSION
            )));
        }

        let salt = decode_b64_fixed::<KDF_SALT_LEN>(&file.kdf_salt_b64, "kdf_salt")?;
        let nonce = decode_b64_fixed::<AES_NONCE_LEN>(&file.vault_nonce_b64, "vault_nonce")?;
        let ciphertext = decode_b64(&file.vault_ciphertext_b64, "vault_ciphertext")?;

        let master_key = if let Some(pre) = source.pre_derived() {
            pre
        } else {
            let passphrase = source.resolve_passphrase()?.ok_or_else(|| {
                VaultError::Kdf("master key source produced neither passphrase nor key".into())
            })?;
            derive_master_key(&passphrase, &salt)?
        };

        let plaintext_bytes = decrypt(&master_key, &nonce, &ciphertext)?;
        let plaintext: VaultPlaintext = serde_json::from_slice(&plaintext_bytes)
            .map_err(|e| VaultError::Corrupt(format!("decrypt-then-parse: {e}")))?;

        let entries = plaintext
            .entries
            .into_iter()
            .map(|e| (e.primary_self.clone(), e))
            .collect();

        Ok(Self {
            path: path.to_path_buf(),
            master_key,
            salt,
            entries,
            managed_signing: plaintext.managed_signing,
        })
    }

    /// Mint a fresh empty vault under a new random salt.
    /// `seal()` is required before any other process can read it.
    pub fn init(path: &Path, source: &MasterKeySource) -> Result<Self, VaultError> {
        let mut salt = [0u8; KDF_SALT_LEN];
        OsRng.fill_bytes(&mut salt);

        let master_key = if let Some(pre) = source.pre_derived() {
            pre
        } else {
            let passphrase = source.resolve_passphrase()?.ok_or_else(|| {
                VaultError::Kdf("master key source produced neither passphrase nor key".into())
            })?;
            derive_master_key(&passphrase, &salt)?
        };

        Ok(Self {
            path: path.to_path_buf(),
            master_key,
            salt,
            entries: HashMap::new(),
            managed_signing: ManagedSigningInventory::default(),
        })
    }

    /// Insert a new entry. `seed_hex` MUST be a 32-byte ed25519
    /// seed in lowercase hex. Rejects with `AlreadyExists` when
    /// the URA already has an entry — the caller (pairing flow)
    /// is responsible for explicit `forget` before re-keying so
    /// silent overwrite cannot happen by accident.
    pub fn put(
        &mut self,
        primary_self: &str,
        role_overlays: Vec<String>,
        seed_hex: String,
    ) -> Result<(), VaultError> {
        let seed_bytes = hex::decode(&seed_hex)
            .map_err(|e| VaultError::Corrupt(format!("seed_hex decode: {e}")))?;
        if seed_bytes.len() != ED25519_SEED_LEN {
            return Err(VaultError::BadSeedLen {
                got: seed_bytes.len(),
            });
        }

        if self.entries.contains_key(primary_self) {
            return Err(VaultError::AlreadyExists(primary_self.to_string()));
        }

        self.entries.insert(
            primary_self.to_string(),
            KeyringEntry {
                primary_self: primary_self.to_string(),
                role_overlays,
                seed_hex,
            },
        );
        Ok(())
    }

    /// Sign `canonical_bytes` with the keypair indexed by
    /// `self_ura`. Looks up by primary_self first, then by
    /// role_overlays — so backend can sign as `r/<r>/hub` even
    /// though the vault entry is keyed by `r/<r>/device/<uuid>`.
    /// Returns the 64-byte ed25519 signature.
    pub fn sign(&self, self_ura: &str, canonical_bytes: &[u8]) -> Result<Signature, VaultError> {
        let entry = self.lookup(self_ura)?;
        let signing_key = signing_key_from_entry(entry)?;
        Ok(signing_key.sign(canonical_bytes))
    }

    /// Return the public key for `self_ura`. Same lookup rule as
    /// `sign`.
    pub fn derive_pubkey(&self, self_ura: &str) -> Result<VerifyingKey, VaultError> {
        let entry = self.lookup(self_ura)?;
        let signing_key = signing_key_from_entry(entry)?;
        Ok(signing_key.verifying_key())
    }

    /// Export the raw 32-byte Ed25519 seed for `self_ura`.
    ///
    /// Phase 3D bridge: the daemon's `boot::load_daemon_identity`
    /// stores the seed (not a `SigningKey`) inside `DaemonIdentity`
    /// so subjective-self channels can re-derive on demand. This
    /// is the ONLY exfiltration path for raw seed bytes — every
    /// other vault method takes canonical bytes in and gives a
    /// signature out, which is the right ergonomics for everything
    /// except boot's "I need a seed to feed the existing
    /// `derive_subject_keypair` path" requirement.
    ///
    /// Same role-overlay lookup rules as `sign` / `derive_pubkey`.
    pub fn export_seed(&self, self_ura: &str) -> Result<[u8; ED25519_SEED_LEN], VaultError> {
        let entry = self.lookup(self_ura)?;
        let seed = hex::decode(&entry.seed_hex)
            .map_err(|e| VaultError::Corrupt(format!("seed_hex decode: {e}")))?;
        seed.as_slice()
            .try_into()
            .map_err(|_| VaultError::BadSeedLen { got: seed.len() })
    }

    /// List all primary_self URAs the vault holds.
    pub fn list(&self) -> Vec<String> {
        let mut out: Vec<String> = self.entries.keys().cloned().collect();
        out.sort();
        out
    }

    /// Forget an entry. Idempotent — forgetting a non-existent
    /// URA returns Ok. The hard-fail variant lives in
    /// `forget_strict` for the rare caller that wants to
    /// distinguish "I just removed it" from "it wasn't there".
    pub fn forget(&mut self, primary_self: &str) {
        self.entries.remove(primary_self);
    }

    /// Strict variant of `forget`. Errors when the entry is absent.
    pub fn forget_strict(&mut self, primary_self: &str) -> Result<(), VaultError> {
        self.entries
            .remove(primary_self)
            .ok_or_else(|| VaultError::NotFound(primary_self.to_string()))?;
        Ok(())
    }

    /// Encrypt + persist. Atomic write via tempfile + rename so a
    /// crash mid-write cannot leave a half-encrypted vault on
    /// disk. The salt is preserved across seals — rotating the
    /// salt would invalidate every passphrase derivation, which
    /// is the wrong policy (operator's passphrase doesn't change
    /// when entries do).
    pub fn seal(&self) -> Result<(), VaultError> {
        let plaintext = VaultPlaintext {
            entries: self.entries.values().cloned().collect(),
            managed_signing: self.managed_signing.clone(),
        };
        let plaintext_bytes = serde_json::to_vec(&plaintext)?;

        let mut nonce_bytes = [0u8; AES_NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);

        let ciphertext = encrypt(&self.master_key, &nonce_bytes, &plaintext_bytes)?;

        let file = KeyringFile {
            version: KeyringFile::CURRENT_VERSION,
            kdf_salt_b64: encode_b64(&self.salt),
            vault_nonce_b64: encode_b64(&nonce_bytes),
            vault_ciphertext_b64: encode_b64(&ciphertext),
        };
        let json = serde_json::to_vec_pretty(&file)?;

        atomic_write(&self.path, &json)?;
        Ok(())
    }

    /// Apply one mutation as an atomic in-memory + durable transaction.
    ///
    /// The mutation is published in memory only when the encrypted vault was
    /// atomically replaced on disk. Any domain error or persistence failure
    /// restores the prior state, preventing a process from signing with a key
    /// state that clients were told had failed to commit.
    pub fn mutate_and_seal<T>(
        &mut self,
        mutation: impl FnOnce(&mut Self) -> Result<T, VaultError>,
    ) -> Result<T, VaultError> {
        let entries_before = self.entries.clone();
        let inventory_before = self.managed_signing.clone();
        match mutation(self) {
            Ok(output) => match self.seal() {
                Ok(()) => Ok(output),
                Err(error) => {
                    self.entries = entries_before;
                    self.managed_signing = inventory_before;
                    Err(error)
                }
            },
            Err(error) => {
                self.entries = entries_before;
                self.managed_signing = inventory_before;
                Err(error)
            }
        }
    }

    /// Return whether an entry exists. Cheap; does not unseal the
    /// keypair.
    pub fn contains(&self, self_ura: &str) -> bool {
        self.entries.contains_key(self_ura)
            || self
                .entries
                .values()
                .any(|e| e.role_overlays.iter().any(|o| o == self_ura))
    }

    /// Create one subject-bound, rotatable managed signing key. The seed is
    /// generated inside the daemon vault and is never a request or response
    /// field. The returned value is a public projection.
    pub fn inventory_create(
        &mut self,
        purpose: String,
        bound_subject: Option<String>,
    ) -> Result<ManagedSigningKeyProjection, VaultError> {
        if purpose.trim().is_empty() {
            return Err(VaultError::Policy(
                "managed signing purpose must not be empty".into(),
            ));
        }
        let mut seed = [0u8; ED25519_SEED_LEN];
        OsRng.fill_bytes(&mut seed);
        let signing_key = SigningKey::from_bytes(&seed);
        let key = ManagedSigningKey {
            key_id: next_managed_key_id(),
            purpose,
            public_key_b64: encode_b64(&signing_key.verifying_key().to_bytes()),
            seed_hex: hex::encode(seed),
            status: ManagedSigningStatus::Active,
            rotation_epoch: 0,
            bound_subject,
            rotated_from: None,
            created_unix_ms: chrono::Utc::now().timestamp_millis(),
            expires_unix_ms: None,
            revoked_unix_ms: None,
        };
        let projection = ManagedSigningKeyProjection::from(&key);
        self.managed_signing.keys.push(key);
        Ok(projection)
    }

    /// Return public metadata only. No inventory caller can observe a seed.
    pub fn inventory_list(
        &self,
        purpose: Option<&str>,
        status: Option<ManagedSigningStatus>,
    ) -> Vec<ManagedSigningKeyProjection> {
        let mut keys = self
            .managed_signing
            .keys
            .iter()
            .filter(|key| purpose.map(|p| key.purpose == p).unwrap_or(true))
            .filter(|key| status.map(|s| key.status == s).unwrap_or(true))
            .map(ManagedSigningKeyProjection::from)
            .collect::<Vec<_>>();
        keys.sort_by(|a, b| a.key_id.cmp(&b.key_id));
        keys
    }

    pub fn inventory_public_key(
        &self,
        key_id: &str,
    ) -> Result<ManagedSigningKeyProjection, VaultError> {
        self.inventory_key(key_id)
            .map(ManagedSigningKeyProjection::from)
    }

    /// Sign only with an active and unexpired managed key.
    pub fn inventory_sign(
        &self,
        key_id: &str,
        canonical_bytes: &[u8],
    ) -> Result<Signature, VaultError> {
        let key = self.inventory_key(key_id)?;
        self.ensure_inventory_signable(key)?;
        Ok(managed_signing_key_from(key)?.sign(canonical_bytes))
    }

    /// Atomically retire an active predecessor and append its successor.
    pub fn inventory_rotate(
        &mut self,
        key_id: &str,
    ) -> Result<ManagedSigningKeyProjection, VaultError> {
        let predecessor_index = self.inventory_key_index(key_id)?;
        let predecessor = self.managed_signing.keys[predecessor_index].clone();
        if predecessor.status != ManagedSigningStatus::Active {
            return Err(VaultError::Lifecycle(
                "only active managed signing keys can rotate".into(),
            ));
        }
        let mut seed = [0u8; ED25519_SEED_LEN];
        OsRng.fill_bytes(&mut seed);
        let signing_key = SigningKey::from_bytes(&seed);
        let successor = ManagedSigningKey {
            key_id: next_managed_key_id(),
            purpose: predecessor.purpose.clone(),
            public_key_b64: encode_b64(&signing_key.verifying_key().to_bytes()),
            seed_hex: hex::encode(seed),
            status: ManagedSigningStatus::Active,
            rotation_epoch: predecessor.rotation_epoch.checked_add(1).ok_or_else(|| {
                VaultError::Lifecycle("managed signing rotation epoch overflow".into())
            })?,
            bound_subject: predecessor.bound_subject.clone(),
            rotated_from: Some(predecessor.key_id.clone()),
            created_unix_ms: chrono::Utc::now().timestamp_millis(),
            expires_unix_ms: predecessor.expires_unix_ms,
            revoked_unix_ms: None,
        };
        self.managed_signing.keys[predecessor_index].status = ManagedSigningStatus::Retired;
        let projection = ManagedSigningKeyProjection::from(&successor);
        self.managed_signing.keys.push(successor);
        Ok(projection)
    }

    /// Move an active or retired key to its terminal revoked state.
    pub fn inventory_revoke(&mut self, key_id: &str) -> Result<i64, VaultError> {
        let key = self.inventory_key_mut(key_id)?;
        if key.status == ManagedSigningStatus::Revoked {
            return Err(VaultError::Lifecycle(
                "managed signing key is already revoked".into(),
            ));
        }
        key.status = ManagedSigningStatus::Revoked;
        let timestamp = chrono::Utc::now().timestamp_millis();
        key.revoked_unix_ms = Some(timestamp);
        Ok(timestamp)
    }

    pub fn inventory_set_expiry(
        &mut self,
        key_id: &str,
        expires_unix_ms: i64,
    ) -> Result<(), VaultError> {
        let key = self.inventory_key_mut(key_id)?;
        if key.status == ManagedSigningStatus::Revoked {
            return Err(VaultError::Lifecycle(
                "cannot set expiry on a revoked managed signing key".into(),
            ));
        }
        key.expires_unix_ms = Some(expires_unix_ms);
        Ok(())
    }

    /// Bind a key exactly once. Rebinding requires a successor key.
    pub fn inventory_bind_subject(
        &mut self,
        key_id: &str,
        subject_ura: String,
    ) -> Result<(), VaultError> {
        if subject_ura.trim().is_empty() {
            return Err(VaultError::Policy(
                "managed signing subject must not be empty".into(),
            ));
        }
        let key = self.inventory_key_mut(key_id)?;
        if key.status != ManagedSigningStatus::Active {
            return Err(VaultError::Lifecycle(
                "only active managed signing keys can bind a subject".into(),
            ));
        }
        match &key.bound_subject {
            None => key.bound_subject = Some(subject_ura),
            Some(existing) if existing == &subject_ura => {}
            Some(_) => {
                return Err(VaultError::Policy(
                    "managed signing subject is immutable; rotate before rebinding".into(),
                ));
            }
        }
        Ok(())
    }

    /// Add or refresh a trusted peer's public projection. The fingerprint is
    /// derived by the daemon; callers cannot assert one for arbitrary bytes.
    pub fn inventory_peer_add(
        &mut self,
        peer_ura: String,
        public_key_b64: String,
        via_hub: Option<String>,
    ) -> Result<bool, VaultError> {
        use base64::Engine;
        if peer_ura.trim().is_empty() {
            return Err(VaultError::Policy("peer URA must not be empty".into()));
        }
        let public_key = base64::engine::general_purpose::STANDARD
            .decode(&public_key_b64)
            .map_err(|err| VaultError::Base64(format!("managed peer public key: {err}")))?;
        let _: [u8; 32] = public_key.as_slice().try_into().map_err(|_| {
            VaultError::Policy(format!(
                "managed peer public key length must be 32, got {}",
                public_key.len()
            ))
        })?;
        let now = chrono::Utc::now().timestamp_millis();
        let fingerprint_b64 = encode_b64(&public_key_fingerprint(&public_key));
        if let Some(peer) = self
            .managed_signing
            .peers
            .iter_mut()
            .find(|peer| peer.peer_ura == peer_ura)
        {
            peer.public_key_b64 = public_key_b64;
            peer.fingerprint_b64 = fingerprint_b64;
            peer.via_hub = via_hub;
            peer.last_seen_unix_ms = now;
            return Ok(false);
        }
        self.managed_signing.peers.push(ManagedPeer {
            peer_ura,
            fingerprint_b64,
            public_key_b64,
            via_hub,
            added_unix_ms: now,
            last_seen_unix_ms: now,
        });
        Ok(true)
    }

    pub fn inventory_peer_list(&self) -> Vec<ManagedPeer> {
        let mut peers = self.managed_signing.peers.clone();
        peers.sort_by(|a, b| a.peer_ura.cmp(&b.peer_ura));
        peers
    }

    fn inventory_key(&self, key_id: &str) -> Result<&ManagedSigningKey, VaultError> {
        self.managed_signing
            .keys
            .iter()
            .find(|key| key.key_id == key_id)
            .ok_or_else(|| VaultError::NotFound(format!("managed signing key {key_id}")))
    }

    fn inventory_key_mut(&mut self, key_id: &str) -> Result<&mut ManagedSigningKey, VaultError> {
        self.managed_signing
            .keys
            .iter_mut()
            .find(|key| key.key_id == key_id)
            .ok_or_else(|| VaultError::NotFound(format!("managed signing key {key_id}")))
    }

    fn inventory_key_index(&self, key_id: &str) -> Result<usize, VaultError> {
        self.managed_signing
            .keys
            .iter()
            .position(|key| key.key_id == key_id)
            .ok_or_else(|| VaultError::NotFound(format!("managed signing key {key_id}")))
    }

    fn ensure_inventory_signable(&self, key: &ManagedSigningKey) -> Result<(), VaultError> {
        if key.status != ManagedSigningStatus::Active {
            return Err(VaultError::Lifecycle(
                "only active managed signing keys can sign".into(),
            ));
        }
        if key
            .expires_unix_ms
            .map(|expires| expires <= chrono::Utc::now().timestamp_millis())
            .unwrap_or(false)
        {
            return Err(VaultError::Lifecycle(
                "managed signing key is expired".into(),
            ));
        }
        Ok(())
    }

    fn lookup(&self, self_ura: &str) -> Result<&KeyringEntry, VaultError> {
        if let Some(entry) = self.entries.get(self_ura) {
            return Ok(entry);
        }
        // Role-overlay lookup. RFC-001 v4.1.5 §3.5: same keypair
        // signs as different URAs.
        for entry in self.entries.values() {
            if entry.role_overlays.iter().any(|o| o == self_ura) {
                return Ok(entry);
            }
        }
        Err(VaultError::NotFound(self_ura.to_string()))
    }
}

fn managed_signing_key_from(key: &ManagedSigningKey) -> Result<SigningKey, VaultError> {
    let seed = hex::decode(&key.seed_hex)
        .map_err(|err| VaultError::Corrupt(format!("managed signing seed decode: {err}")))?;
    let seed: [u8; ED25519_SEED_LEN] = seed
        .as_slice()
        .try_into()
        .map_err(|_| VaultError::BadSeedLen { got: seed.len() })?;
    Ok(SigningKey::from_bytes(&seed))
}

fn next_managed_key_id() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    format!("msk-{}", hex::encode(bytes))
}

/// SHA-256 fingerprint over a public key projection.
pub fn public_key_fingerprint(public_key: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let digest = sha2::Sha256::digest(public_key);
    digest.into()
}

impl Drop for Vault {
    fn drop(&mut self) {
        // Best-effort scrub. The key bytes may have been copied
        // by the heap allocator; this only zeroes the original
        // buffer. Production process isolation is the actual
        // hard guarantee.
        for byte in self.master_key.iter_mut() {
            *byte = 0;
        }
    }
}

fn signing_key_from_entry(entry: &KeyringEntry) -> Result<SigningKey, VaultError> {
    let seed = hex::decode(&entry.seed_hex)
        .map_err(|e| VaultError::Corrupt(format!("seed_hex decode: {e}")))?;
    let arr: [u8; ED25519_SEED_LEN] = seed
        .as_slice()
        .try_into()
        .map_err(|_| VaultError::BadSeedLen { got: seed.len() })?;
    Ok(SigningKey::from_bytes(&arr))
}

fn encrypt(
    key: &[u8; AES_KEY_LEN],
    nonce: &[u8; AES_NONCE_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>, VaultError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .encrypt(Nonce::from_slice(nonce), plaintext)
        .map_err(|e| VaultError::Crypto(format!("aes-gcm encrypt: {e}")))
}

fn decrypt(
    key: &[u8; AES_KEY_LEN],
    nonce: &[u8; AES_NONCE_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>, VaultError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|e| VaultError::Crypto(format!("aes-gcm decrypt: {e} (wrong passphrase?)")))
}

fn encode_b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode_b64(s: &str, field: &str) -> Result<Vec<u8>, VaultError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| VaultError::Base64(format!("{field}: {e}")))
}

fn decode_b64_fixed<const N: usize>(s: &str, field: &str) -> Result<[u8; N], VaultError> {
    let v = decode_b64(s, field)?;
    v.as_slice()
        .try_into()
        .map_err(|_| VaultError::Corrupt(format!("{field} length: expected {N}, got {}", v.len())))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_name = format!(
        ".{}.{}.tmp",
        path.file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "keyring".into()),
        std::process::id(),
    );
    let tmp_path = path.with_file_name(tmp_name);
    fs::write(&tmp_path, bytes)?;
    // 0600 so peer users on the host cannot read the encrypted
    // blob. Even with passphrase protection, removing the public
    // surface area is cheap defence in depth.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Salt-string helper for any future caller that wants a salt
/// matching `argon2`'s expected encoding.
pub fn fresh_salt_string() -> SaltString {
    SaltString::generate(&mut OsRng)
}

// ── Wire protocol ────────────────────────────────────────────────
//
// The keyring daemon speaks length-prefixed JSON over UDS. v1
// shape locked here so Phase 3B's client and the daemon binary
// can both depend on these types.

/// Request from a keyring client (backend / daemon / CLI).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum KeyringRequest {
    /// Ensure a runtime owner has a signing identity. The keyring generates
    /// the seed itself when absent and returns only the corresponding public
    /// key. Callers never provide or receive private key material.
    Ensure {
        primary_self: String,
        #[serde(default)]
        role_overlays: Vec<String>,
    },
    /// Sign canonical bytes with the keypair indexed by
    /// `self_ura`.
    Sign {
        self_ura: String,
        canonical_bytes_b64: String,
    },
    /// Return the public key for `self_ura`.
    DerivePubkey { self_ura: String },
    /// List every primary_self URA the vault holds.
    List,
    /// Remove an entry.
    Forget { primary_self: String },
    /// Create a subject-bound key in the managed signing inventory.
    #[serde(rename = "inventory.create")]
    InventoryCreate {
        purpose: String,
        #[serde(default)]
        bound_subject: Option<String>,
    },
    /// List public managed key projections.
    #[serde(rename = "inventory.list")]
    InventoryList {
        #[serde(default)]
        purpose: Option<String>,
        #[serde(default)]
        status: Option<ManagedSigningStatus>,
    },
    /// Return one managed key's public projection.
    #[serde(rename = "inventory.public_key")]
    InventoryPublicKey { key_id: String },
    /// Sign canonical bytes with one managed key.
    #[serde(rename = "inventory.sign")]
    InventorySign {
        key_id: String,
        canonical_bytes_b64: String,
    },
    /// Retire one key and create its successor atomically.
    #[serde(rename = "inventory.rotate")]
    InventoryRotate { key_id: String },
    /// Terminally revoke one managed key.
    #[serde(rename = "inventory.revoke")]
    InventoryRevoke { key_id: String },
    /// Set an explicit sign-expiry timestamp.
    #[serde(rename = "inventory.set_expiry")]
    InventorySetExpiry {
        key_id: String,
        expires_unix_ms: i64,
    },
    /// Bind an unbound active key to its immutable subject URA.
    #[serde(rename = "inventory.bind_subject")]
    InventoryBindSubject { key_id: String, subject_ura: String },
    /// Add or refresh a trusted peer public projection.
    #[serde(rename = "inventory.peer_add")]
    InventoryPeerAdd {
        peer_ura: String,
        public_key_b64: String,
        #[serde(default)]
        via_hub: Option<String>,
    },
    /// List trusted peer public projections.
    #[serde(rename = "inventory.peer_list")]
    InventoryPeerList,
}

/// Response to a `KeyringRequest`. Errors are typed so the client
/// can pattern-match without parsing strings.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum KeyringResponse {
    Ok,
    Signature {
        signature_b64: String,
    },
    PublicKey {
        public_key_b64: String,
    },
    List {
        entries: Vec<String>,
    },
    InventoryKey {
        entry: ManagedSigningKeyProjection,
    },
    InventoryKeys {
        entries: Vec<ManagedSigningKeyProjection>,
    },
    InventoryRevoked {
        revoked_unix_ms: i64,
    },
    InventoryPeerAdded {
        added: bool,
    },
    InventoryPeers {
        peers: Vec<ManagedPeer>,
    },
    Error {
        kind: String,
        message: String,
    },
}

impl KeyringResponse {
    pub fn err(kind: &str, message: impl Into<String>) -> Self {
        KeyringResponse::Error {
            kind: kind.into(),
            message: message.into(),
        }
    }
}

/// Map a `VaultError` to the wire `KeyringResponse::Error` variant.
pub fn vault_error_to_response(err: VaultError) -> KeyringResponse {
    let kind = match &err {
        VaultError::Io(_) => "io",
        VaultError::Serde(_) => "serde",
        VaultError::Kdf(_) => "kdf",
        VaultError::Crypto(_) => "crypto",
        VaultError::Base64(_) => "base64",
        VaultError::NotFound(_) => "not_found",
        VaultError::AlreadyExists(_) => "already_exists",
        VaultError::Corrupt(_) => "corrupt",
        VaultError::BadSeedLen { .. } => "bad_seed_len",
        VaultError::Lifecycle(_) => "lifecycle",
        VaultError::Policy(_) => "policy",
    };
    KeyringResponse::err(kind, err.to_string())
}

/// Default UDS path for the keyring daemon. Phase 3E will
/// replace this with a self-URA-derived path; v1 ships the flat
/// path so the daemon can boot before any URA is known.
pub const DEFAULT_KEYRING_SOCKET_REL: &str = ".easynet/keyring.sock";

/// Default vault file path.
pub const DEFAULT_VAULT_REL: &str = ".easynet/keyring.enc";

/// Auto-generated master-key passphrase file.
///
/// When the operator has not supplied `EASYNET_KEYRING_PASSPHRASE`,
/// `easynet join` generates a random passphrase and persists it here
/// (mode 0600) so that (a) the `easynet-keyring` daemon it spawns can
/// open/init the vault and (b) every subsequent `easynet runtime start`
/// can inject the same passphrase into the daemon's environment, which
/// is what lets the daemon read the encrypted vault across restarts.
///
/// This is a deliberate trade-off: the passphrase lands in plaintext
/// on the same disk as the vault, so it does not protect against an
/// attacker who already has read access to `~/.easynet`. It still
/// improves on the pre-keyring state (seed kept only via deterministic
/// derivation) by isolating the seed behind a rotatable file and
/// keeping it out of `credentials.json`. Operators who want a real
/// secret boundary export `EASYNET_KEYRING_PASSPHRASE` themselves and
/// this file is never written.
pub const DEFAULT_PASSPHRASE_REL: &str = ".easynet/keyring.pass";

/// Resolve a `~/.easynet/...` path against `$HOME` (or fallback).
pub fn home_relative(rel: &str) -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(rel)
}

/// Export the raw Ed25519 seed for one runtime owner from the default daemon
/// keyring vault.
///
/// This is the canonical same-process bootstrap seam for daemon components
/// that still need a seed-shaped API. It reads `EASYNET_KEYRING_VAULT_PATH`
/// when set, otherwise `~/.easynet/keyring.enc`, and requires
/// `EASYNET_KEYRING_PASSPHRASE`.
pub fn export_seed_from_default_vault(
    self_ura: &str,
) -> Result<[u8; ED25519_SEED_LEN], VaultError> {
    let path = std::env::var_os("EASYNET_KEYRING_VAULT_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_relative(DEFAULT_VAULT_REL));
    let source = MasterKeySource::from_env()?;
    let vault = Vault::open(&path, &source)?;
    vault.export_seed(self_ura)
}

/// Default transport endpoint for the keyring daemon.
pub fn default_socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("EASYNET_KEYRING_SOCKET_PATH") {
        return PathBuf::from(path);
    }

    #[cfg(windows)]
    {
        return PathBuf::from(crate::support::platform::named_pipe::scoped_pipe_name(
            "keyring",
        ));
    }

    #[cfg(not(windows))]
    home_relative(DEFAULT_KEYRING_SOCKET_REL)
}

/// Default path of the auto-generated passphrase file.
pub fn default_passphrase_path() -> PathBuf {
    home_relative(DEFAULT_PASSPHRASE_REL)
}

/// Resolve the keyring master-key passphrase, generating and
/// persisting one on first use.
///
/// Resolution order:
///   1. `EASYNET_KEYRING_PASSPHRASE` env — operator-supplied secret
///      takes precedence and is never written to disk.
///   2. `~/.easynet/keyring.pass` if it already exists — reuse it so
///      the passphrase stays stable across joins and daemon restarts
///      (a changed passphrase would orphan the existing vault).
///   3. Otherwise mint a fresh 256-bit random passphrase, write it to
///      `~/.easynet/keyring.pass` at mode 0600, and return it.
///
/// Returns the passphrase plus whether it was newly generated (so the
/// caller can surface "generated" vs "reused" in the join stage line).
pub fn load_or_create_passphrase() -> std::io::Result<(String, bool)> {
    if let Ok(env) = std::env::var("EASYNET_KEYRING_PASSPHRASE") {
        if !env.is_empty() {
            return Ok((env, false));
        }
    }

    let path = default_passphrase_path();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return Ok((trimmed, false));
        }
    }

    let generated = mint_passphrase();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, &generated)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok((generated, true))
}

/// Mint a fresh 256-bit random passphrase, hex-encoded.
fn mint_passphrase() -> String {
    use rand::rngs::OsRng;
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use ed25519_dalek::Verifier;
    use tempfile::TempDir;

    /// Run `f` with `HOME` pointed at a fresh temp dir and
    /// `EASYNET_KEYRING_PASSPHRASE` forced to `env_pass`, restoring both
    /// afterwards. Serialised against the rest of the env-mutating tests
    /// through the shared process-env lock.
    fn with_home_and_env<R>(env_pass: Option<&str>, f: impl FnOnce(&std::path::Path) -> R) -> R {
        let _lock = crate::cli::commands::test_support::env_lock();
        let prev_home = std::env::var("HOME").ok();
        let prev_pass = std::env::var("EASYNET_KEYRING_PASSPHRASE").ok();
        let dir = TempDir::new().unwrap();
        std::env::set_var("HOME", dir.path());
        match env_pass {
            Some(p) => std::env::set_var("EASYNET_KEYRING_PASSPHRASE", p),
            None => std::env::remove_var("EASYNET_KEYRING_PASSPHRASE"),
        }
        let out = f(dir.path());
        match prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }
        match prev_pass {
            Some(p) => std::env::set_var("EASYNET_KEYRING_PASSPHRASE", p),
            None => std::env::remove_var("EASYNET_KEYRING_PASSPHRASE"),
        }
        out
    }

    #[test]
    fn passphrase_is_generated_then_reused_and_persisted_0600() {
        with_home_and_env(None, |home| {
            let (first, generated) = load_or_create_passphrase().unwrap();
            assert!(generated, "first call must mint a passphrase");
            assert_eq!(first.len(), 64, "256-bit hex passphrase");

            let path = home.join(DEFAULT_PASSPHRASE_REL);
            assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o600, "passphrase file must be 0600");
            }

            let (second, generated_again) = load_or_create_passphrase().unwrap();
            assert!(!generated_again, "second call must reuse the file");
            assert_eq!(first, second, "reused passphrase must be stable");
        });
    }

    #[test]
    fn env_passphrase_takes_precedence_and_is_not_persisted() {
        with_home_and_env(Some("operator-secret"), |home| {
            let (pass, generated) = load_or_create_passphrase().unwrap();
            assert!(!generated);
            assert_eq!(pass, "operator-secret");
            assert!(
                !home.join(DEFAULT_PASSPHRASE_REL).exists(),
                "env-supplied passphrase must never be written to disk"
            );
        });
    }

    fn fresh_seed_hex() -> String {
        let mut seed = [0u8; ED25519_SEED_LEN];
        OsRng.fill_bytes(&mut seed);
        hex::encode(seed)
    }

    fn explicit_pass() -> MasterKeySource {
        MasterKeySource::Explicit("test-passphrase-which-is-long-enough".into())
    }

    #[test]
    fn put_sign_verify_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let device_ura = "easynet:///r/localhost/device/dev-uuid".to_string();
        vault.put(&device_ura, vec![], fresh_seed_hex()).unwrap();
        vault.seal().unwrap();

        let pubkey = vault.derive_pubkey(&device_ura).unwrap();
        let msg = b"axiom canonical bytes";
        let sig = vault.sign(&device_ura, msg).unwrap();
        pubkey.verify(msg, &sig).expect("sig verifies");
    }

    #[test]
    fn role_overlay_signs_with_same_keypair() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let device_ura = "easynet:///r/localhost/device/dev-uuid".to_string();
        let hub_endpoint = crate::core::ura::hub_ura("localhost");
        vault
            .put(&device_ura, vec![hub_endpoint.clone()], fresh_seed_hex())
            .unwrap();

        let pubkey_via_device = vault.derive_pubkey(&device_ura).unwrap();
        let pubkey_via_hub = vault.derive_pubkey(&hub_endpoint).unwrap();
        assert_eq!(
            pubkey_via_device.to_bytes(),
            pubkey_via_hub.to_bytes(),
            "role overlay must reuse the device's keypair"
        );

        let msg = b"role overlay test";
        let sig_device = vault.sign(&device_ura, msg).unwrap();
        let sig_hub = vault.sign(&hub_endpoint, msg).unwrap();
        assert_eq!(
            sig_device.to_bytes(),
            sig_hub.to_bytes(),
            "deterministic ed25519 over identical bytes must match"
        );
    }

    #[test]
    fn seal_and_reopen_preserves_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let device_ura = "easynet:///r/localhost/device/dev-uuid".to_string();
        let seed = fresh_seed_hex();

        {
            let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
            vault.put(&device_ura, vec![], seed.clone()).unwrap();
            vault.seal().unwrap();
        }

        let vault2 = Vault::open(&path, &explicit_pass()).unwrap();
        assert_eq!(vault2.list(), vec![device_ura.clone()]);
        assert_eq!(vault2.entries[&device_ura].seed_hex, seed);
    }

    #[test]
    fn wrong_passphrase_rejected_with_crypto_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        {
            let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
            vault
                .put("easynet:///r/r/device/u", vec![], fresh_seed_hex())
                .unwrap();
            vault.seal().unwrap();
        }

        let wrong = MasterKeySource::Explicit("totally-different-passphrase-32-chars".into());
        let err = Vault::open(&path, &wrong).unwrap_err();
        match err {
            VaultError::Crypto(msg) => assert!(msg.contains("aes-gcm decrypt"), "{msg}"),
            other => panic!("expected Crypto, got {other:?}"),
        }
    }

    #[test]
    fn put_already_exists_rejected() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let ura = "easynet:///r/r/device/u".to_string();
        vault.put(&ura, vec![], fresh_seed_hex()).unwrap();
        let err = vault.put(&ura, vec![], fresh_seed_hex()).unwrap_err();
        assert!(matches!(err, VaultError::AlreadyExists(_)));
    }

    #[test]
    fn forget_then_sign_fails() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let ura = "easynet:///r/r/device/u".to_string();
        vault.put(&ura, vec![], fresh_seed_hex()).unwrap();
        vault.forget(&ura);
        let err = vault.sign(&ura, b"x").unwrap_err();
        assert!(matches!(err, VaultError::NotFound(_)));
    }

    #[test]
    fn forget_strict_distinguishes_present_vs_absent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let ura = "easynet:///r/r/device/u".to_string();
        vault.put(&ura, vec![], fresh_seed_hex()).unwrap();
        vault.forget_strict(&ura).expect("removes once");
        let err = vault.forget_strict(&ura).unwrap_err();
        assert!(matches!(err, VaultError::NotFound(_)));
    }

    #[test]
    fn bad_seed_len_rejected() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let err = vault.put("u", vec![], "deadbeef".into()).unwrap_err(); // 4 bytes
        assert!(matches!(err, VaultError::BadSeedLen { got: 4 }));
    }

    #[test]
    fn seal_writes_mode_0600_on_unix() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        vault
            .put("easynet:///r/r/device/u", vec![], fresh_seed_hex())
            .unwrap();
        vault.seal().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "vault file must be 0600 on unix");
        }
    }

    #[test]
    fn list_is_sorted() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        vault.put("z-ura", vec![], fresh_seed_hex()).unwrap();
        vault.put("a-ura", vec![], fresh_seed_hex()).unwrap();
        vault.put("m-ura", vec![], fresh_seed_hex()).unwrap();
        assert_eq!(vault.list(), vec!["a-ura", "m-ura", "z-ura"]);
    }

    #[test]
    fn contains_matches_primary_and_overlay() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        vault
            .put("primary", vec!["overlay".into()], fresh_seed_hex())
            .unwrap();
        assert!(vault.contains("primary"));
        assert!(vault.contains("overlay"));
        assert!(!vault.contains("ghost"));
    }

    #[test]
    fn managed_signing_state_machine_preserves_subject_and_blocks_retired_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let subject = "easynet:///r/test.local/agent/alice".to_string();
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let first = vault
            .inventory_create("invocation".into(), Some(subject.clone()))
            .unwrap();
        let first_key = first.key_id.clone();
        let first_public = first.public_key_b64.clone();
        let signature = vault
            .inventory_sign(&first_key, b"canonical invocation")
            .unwrap();
        let public = base64::engine::general_purpose::STANDARD
            .decode(first_public)
            .unwrap();
        let public: [u8; 32] = public.as_slice().try_into().unwrap();
        VerifyingKey::from_bytes(&public)
            .unwrap()
            .verify(b"canonical invocation", &signature)
            .unwrap();

        let successor = vault.inventory_rotate(&first_key).unwrap();
        assert_eq!(successor.rotation_epoch, 1);
        assert_eq!(successor.rotated_from.as_deref(), Some(first_key.as_str()));
        assert_eq!(successor.bound_subject.as_deref(), Some(subject.as_str()));
        assert!(matches!(
            vault.inventory_sign(&first_key, b"canonical invocation"),
            Err(VaultError::Lifecycle(_))
        ));
        assert!(vault
            .inventory_sign(&successor.key_id, b"canonical invocation")
            .is_ok());
    }

    #[test]
    fn managed_signing_revoke_expiry_and_binding_are_terminal_or_immutable() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let key = vault.inventory_create("invocation".into(), None).unwrap();
        vault
            .inventory_bind_subject(&key.key_id, "easynet:///r/test.local/agent/alice".into())
            .unwrap();
        assert!(matches!(
            vault.inventory_bind_subject(&key.key_id, "easynet:///r/test.local/agent/bob".into()),
            Err(VaultError::Policy(_))
        ));
        vault
            .inventory_set_expiry(&key.key_id, chrono::Utc::now().timestamp_millis() - 1)
            .unwrap();
        assert!(matches!(
            vault.inventory_sign(&key.key_id, b"canonical invocation"),
            Err(VaultError::Lifecycle(_))
        ));
        let successor = vault.inventory_rotate(&key.key_id).unwrap();
        assert!(
            matches!(
                vault.inventory_sign(&successor.key_id, b"canonical invocation"),
                Err(VaultError::Lifecycle(_))
            ),
            "an inherited expired key cannot sign"
        );
        let revoked = vault.inventory_revoke(&successor.key_id).unwrap();
        assert!(revoked > 0);
        assert!(matches!(
            vault.inventory_revoke(&successor.key_id),
            Err(VaultError::Lifecycle(_))
        ));
        assert!(matches!(
            vault.inventory_set_expiry(&successor.key_id, chrono::Utc::now().timestamp_millis()),
            Err(VaultError::Lifecycle(_))
        ));
    }

    #[test]
    fn managed_signing_inventory_is_encrypted_and_persists_public_projection_only() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let key_id = {
            let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
            let key = vault.inventory_create("invocation".into(), None).unwrap();
            vault.seal().unwrap();
            key.key_id
        };
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            !raw.contains(&key_id),
            "managed metadata is inside ciphertext"
        );
        let vault = Vault::open(&path, &explicit_pass()).unwrap();
        assert_eq!(vault.inventory_list(None, None).len(), 1);
        assert!(vault
            .inventory_sign(&key_id, b"canonical invocation")
            .is_ok());
    }

    #[test]
    fn failed_persistence_rolls_back_managed_signing_mutation() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::init(&path, &explicit_pass()).unwrap();
        let original = vault
            .inventory_create("invocation".into(), Some("easynet:///r/r/agent/a".into()))
            .unwrap();
        std::fs::create_dir(&path).unwrap();

        let error = vault
            .mutate_and_seal(|candidate| candidate.inventory_rotate(&original.key_id))
            .expect_err("renaming a file over a directory must fail");
        assert!(matches!(error, VaultError::Io(_)));

        let keys = vault.inventory_list(None, None);
        assert_eq!(
            keys.len(),
            1,
            "failed transaction must not append successor"
        );
        assert_eq!(keys[0].key_id, original.key_id);
        assert_eq!(keys[0].status, ManagedSigningStatus::Active);
        assert!(vault
            .inventory_sign(&original.key_id, b"canonical invocation")
            .is_ok());
    }

    #[test]
    fn pre_derived_master_key_path() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let key = [42u8; AES_KEY_LEN];
        let source = MasterKeySource::PreDerived(key);
        let mut vault = Vault::open_or_init(&path, &source).unwrap();
        vault
            .put("easynet:///r/r/device/u", vec![], fresh_seed_hex())
            .unwrap();
        vault.seal().unwrap();
        let _re = Vault::open(&path, &source).expect("pre-derived round trip");
    }

    #[test]
    fn keyring_request_response_serde_round_trip() {
        let req = KeyringRequest::Sign {
            self_ura: "easynet:///r/r/device/u".into(),
            canonical_bytes_b64: encode_b64(b"hello"),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: KeyringRequest = serde_json::from_str(&json).unwrap();
        match back {
            KeyringRequest::Sign {
                self_ura,
                canonical_bytes_b64,
            } => {
                assert_eq!(self_ura, "easynet:///r/r/device/u");
                assert_eq!(canonical_bytes_b64, encode_b64(b"hello"));
            }
            other => panic!("wrong variant: {other:?}"),
        }

        let resp = KeyringResponse::Signature {
            signature_b64: "abc".into(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: KeyringResponse = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, KeyringResponse::Signature { .. }));
    }

    #[test]
    fn vault_error_to_response_kind_strings_stable() {
        let cases = vec![
            (VaultError::NotFound("u".into()), "not_found"),
            (VaultError::AlreadyExists("u".into()), "already_exists"),
            (VaultError::BadSeedLen { got: 5 }, "bad_seed_len"),
            (VaultError::Crypto("decrypt".into()), "crypto"),
            (VaultError::Lifecycle("expired".into()), "lifecycle"),
            (VaultError::Policy("subject".into()), "policy"),
        ];
        for (err, want_kind) in cases {
            match vault_error_to_response(err) {
                KeyringResponse::Error { kind, .. } => assert_eq!(kind, want_kind),
                other => panic!("expected Error variant, got {other:?}"),
            }
        }
    }
}
