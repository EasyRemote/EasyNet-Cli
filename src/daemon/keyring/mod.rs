// EasyNet CLI - daemon keyring device identity vault
// =======================================================
//
// File: src/daemon/keyring/mod.rs
//
// The keyring is a process-external vault for runtime-owner Ed25519
// private keys. Every canonical owner URA has a distinct key: host
// co-location never grants one runtime another role's authority.
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
//     framed by a randomly-generated nonce) holding runtime-owner
//     records plus the managed-signing inventory.
//   * `MasterKey` — the symmetric key the file is sealed under,
//     derived from a passphrase via Argon2id with a per-file salt.
//   * `Vault` — the crate-private in-memory open form: master key +
//     decrypted entries. It is reachable only through the canonical
//     key-service dispatcher in production.
//   * `MasterKeySource` — process-local passphrase bytes supplied only by the
//     key-service-owned passphrase store (plus pre-derived test keys).
//
// What this module is NOT
// -----------------------
// - Not a UDS server. The `service` submodule owns framing, bounded
//   transport policy, and request dispatch; `src/bin/easynet-keyring.rs`
//   is only the process bootstrap.
// - Not a caller-supplied key store. Identity and managed-signing
//   seeds are generated inside the service custody boundary.
// - Not a client. `crate::daemon::identity::self_identity` layers a
//   typed client on top of the wire protocol pinned by this module's
//   serde shapes.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::daemon::persistence::config::{
    AtomicWriteCommitState, AtomicWriteError, WritePermissions,
};

pub mod abilities;
pub mod federated_bindings;
pub mod lifecycle;
pub mod managed_signing_projection;
pub mod managed_signing_provider;
mod passphrase;
pub mod resolver;
pub mod service;
pub mod user_binding_chain;
pub mod user_binding_consume;
pub mod user_binding_issue;
pub mod user_binding_projection;

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

/// Maximum canonical payload accepted by runtime and managed signers.  It is
/// aligned with the daemon Invocation gRPC message bound.
pub const MAX_KEY_SERVICE_CANONICAL_BYTES: usize = 64 * 1024 * 1024;

/// Maximum length-prefixed JSON frame. Base64 expands a 64 MiB canonical
/// payload to just under 86 MiB; the remaining headroom covers method and
/// identity metadata without making the transport unbounded.
pub const MAX_KEY_SERVICE_FRAME_BYTES: usize = 90 * 1024 * 1024;

/// Every inventory response is a bounded page even when the caller omits a
/// limit. SDK compatibility helpers may walk pages explicitly.
pub const MAX_MANAGED_SIGNING_PAGE_SIZE: usize = 16;

/// Compatibility collectors are bounded even when a peer keeps returning
/// advancing cursors. Page APIs remain the canonical surface.
pub const MAX_KEY_SERVICE_AUTO_PAGES: usize = 1024;
pub const MAX_KEY_SERVICE_AUTO_ITEMS: usize = 16_384;

/// Key-service wire protocol. Version 2 binds managed signing intents to the
/// expected key purpose and rejects the former purpose-blind request shape.
pub const KEY_SERVICE_PROTOCOL_VERSION: u32 = 2;

/// A filtered inventory page performs at most this many record inspections.
/// Continuation cursors may therefore advance across an empty filtered page;
/// clients must follow the cursor instead of treating an empty page as EOF.
const MAX_MANAGED_SIGNING_PAGE_SCAN: usize = 256;

/// Opaque pagination cursors remain bounded independently of page content.
pub const MAX_MANAGED_SIGNING_CURSOR_BYTES: usize = 4096;

const MAX_MANAGED_SIGNING_PURPOSE_BYTES: usize = 128;
const MAX_MANAGED_SIGNING_URA_BYTES: usize = 1024;
const MAX_MANAGED_SIGNING_KEY_ID_BYTES: usize = 128;

/// Persisted vault file structure. v2 layout:
///
/// ```text
/// {
///   "version": 2,
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
#[serde(deny_unknown_fields)]
struct KeyringFile {
    pub version: u32,
    pub kdf_salt_b64: String,
    pub vault_nonce_b64: String,
    pub vault_ciphertext_b64: String,
}

impl KeyringFile {
    /// The canonical persisted representation.
    pub const CURRENT_VERSION: u32 = 2;
}

/// Plaintext form of the vault. JSON-serialised inside
/// `KeyringFile::vault_ciphertext_b64`.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct VaultPlaintext {
    pub entries: Vec<KeyringEntry>,
    /// The rotatable, subject-bound signing inventory.  It is stored in the
    /// same encrypted daemon vault as runtime identities but deliberately has
    /// a different record model and lifecycle.
    pub managed_signing: ManagedSigningInventory,
}

/// One key entry. `primary_self` is the sole canonical runtime owner URA this
/// key was minted for. A key never carries authority for another owner role.
///
/// `seed_hex` is the 32-byte Ed25519 seed in lowercase hex. It
/// only appears inside the encrypted blob — the unencrypted
/// `KeyringFile` never carries it.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct KeyringEntry {
    pub primary_self: String,
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
#[serde(deny_unknown_fields)]
struct ManagedSigningKey {
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
#[serde(deny_unknown_fields)]
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
                managed_signer_policy_ref(
                    &key.purpose,
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
#[serde(deny_unknown_fields)]
pub struct ManagedPeer {
    pub peer_ura: String,
    pub fingerprint_b64: String,
    pub public_key_b64: String,
    pub via_authority: Option<String>,
    pub added_unix_ms: i64,
    pub last_seen_unix_ms: i64,
}

/// Managed-signing domain persisted inside the daemon vault.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ManagedSigningInventory {
    keys: Vec<ManagedSigningKey>,
    peers: Vec<ManagedPeer>,
}

impl ManagedSigningInventory {
    /// Establish the in-memory ordered-index invariant once when encrypted
    /// state is opened. Duplicate identifiers are corruption, never silently
    /// collapsed. The persisted representation remains a compact vector while
    /// every lookup and page walk uses binary-search/range semantics.
    fn normalize(&mut self) -> Result<(), VaultError> {
        self.validate_persisted_identity_contract()?;
        self.keys
            .sort_by(|left, right| left.key_id.cmp(&right.key_id));
        if self
            .keys
            .windows(2)
            .any(|pair| pair[0].key_id == pair[1].key_id)
        {
            return Err(VaultError::Corrupt(
                "managed signing inventory contains duplicate key IDs".into(),
            ));
        }
        self.peers
            .sort_by(|left, right| left.peer_ura.cmp(&right.peer_ura));
        if self
            .peers
            .windows(2)
            .any(|pair| pair[0].peer_ura == pair[1].peer_ura)
        {
            return Err(VaultError::Corrupt(
                "managed peer inventory contains duplicate URAs".into(),
            ));
        }
        Ok(())
    }

    /// Rehydration is an admission boundary: encrypted legacy state must not
    /// regain signer or peer authority merely because it predates the current
    /// mutation guards.
    fn validate_persisted_identity_contract(&self) -> Result<(), VaultError> {
        for key in &self.keys {
            if let Some(subject_ura) = key.bound_subject.as_deref() {
                validate_persisted_ura(subject_ura, "managed signing subject")?;
            }
        }
        for peer in &self.peers {
            validate_persisted_ura(&peer.peer_ura, "managed peer URA")?;
            if let Some(via_authority) = peer.via_authority.as_deref() {
                let via_authority =
                    parse_persisted_ura(via_authority, "managed peer authority URA")?;
                if via_authority.kind() != crate::core::ura::URAKind::Authority {
                    return Err(VaultError::Corrupt(
                        "managed peer authority URA must be an Authority URA".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Errors surfaced by the vault crypto layer. Wire layer (the
/// daemon) maps these to typed JSON error responses.
#[derive(Debug, thiserror::Error)]
pub(crate) enum VaultError {
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
    #[error("keyring persistence: {0}")]
    Persistence(#[from] AtomicWriteError),
    #[error("keyring fail-stopped: {0}")]
    FailStopped(String),
}

/// Process-local master-key material. In production the sole constructor is
/// fed by `PassphraseStore` inside the key-service process.
#[derive(Debug, Clone)]
pub(crate) enum MasterKeySource {
    /// Take the passphrase verbatim inside the custody process.
    Explicit(String),
    /// Derived master key bytes supplied directly by crypto unit tests.
    #[cfg(test)]
    PreDerived([u8; AES_KEY_LEN]),
}

impl MasterKeySource {
    /// Resolve to the actual passphrase string (or pre-derived
    /// bytes). Called once per `Vault::open`.
    fn resolve_passphrase(&self) -> Result<Option<String>, VaultError> {
        match self {
            MasterKeySource::Explicit(s) => Ok(Some(s.clone())),
            #[cfg(test)]
            MasterKeySource::PreDerived(_) => Ok(None),
        }
    }

    fn pre_derived(&self) -> Option<[u8; AES_KEY_LEN]> {
        match self {
            #[cfg(test)]
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
pub(crate) struct Vault {
    path: PathBuf,
    master_key: [u8; AES_KEY_LEN],
    salt: [u8; KDF_SALT_LEN],
    entries: BTreeMap<String, KeyringEntry>,
    managed_signing: ManagedSigningInventory,
    fail_stopped: Option<String>,
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
            .field("fail_stopped", &self.fail_stopped)
            .finish()
    }
}

impl Vault {
    /// Open the vault file at `path`, decrypt with the master key
    /// derived from `source`. If the file does not exist, mint a
    /// fresh empty vault under a freshly-generated salt — the
    /// first service mutation then writes it. This is the convenience boot
    /// path for "first time on this host".
    pub(crate) fn open_or_init(path: &Path, source: &MasterKeySource) -> Result<Self, VaultError> {
        if path.exists() {
            Self::open(path, source)
        } else {
            Self::init(path, source)
        }
    }

    /// Open an existing vault file.
    pub(crate) fn open(path: &Path, source: &MasterKeySource) -> Result<Self, VaultError> {
        let raw = fs::read_to_string(path)?;
        let file: KeyringFile = serde_json::from_str(&raw)
            .map_err(|e| VaultError::Corrupt(format!("parse {}: {e}", path.display())))?;
        if file.version != KeyringFile::CURRENT_VERSION {
            return Err(VaultError::Corrupt(format!(
                "unsupported keyring version {} (expected {})",
                file.version,
                KeyringFile::CURRENT_VERSION,
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
        let plaintext_entries = plaintext.entries;
        let mut managed_signing = plaintext.managed_signing;

        for entry in &plaintext_entries {
            validate_persisted_ura(&entry.primary_self, "runtime signing owner")?;
        }
        let expected_entry_count = plaintext_entries.len();
        let entries = plaintext_entries
            .into_iter()
            .map(|e| (e.primary_self.clone(), e))
            .collect::<BTreeMap<_, _>>();
        if entries.len() != expected_entry_count {
            return Err(VaultError::Corrupt(
                "runtime signing inventory contains duplicate owner URAs".into(),
            ));
        }

        managed_signing.normalize()?;

        // A previous process may have observed rename success followed by a
        // directory-fsync failure. Re-synchronising the parent is the only
        // safe point at which a restarted process may accept that visible
        // replacement as durable state.
        crate::daemon::persistence::config::sync_parent_dir(path)
            .map_err(|error| io::Error::other(error.to_string()))?;

        let vault = Self {
            path: path.to_path_buf(),
            master_key,
            salt,
            entries,
            managed_signing,
            fail_stopped: None,
        };

        Ok(vault)
    }

    /// Mint a fresh empty vault under a new random salt.
    /// `seal()` is required before any other process can read it.
    pub(crate) fn init(path: &Path, source: &MasterKeySource) -> Result<Self, VaultError> {
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
            entries: BTreeMap::new(),
            managed_signing: ManagedSigningInventory::default(),
            fail_stopped: None,
        })
    }

    /// Ensure one owner resolves to one daemon-custodied key. Generation and
    /// durable persistence are a single service operation; callers never
    /// provide private material or alias another owner role.
    pub(super) fn ensure(&mut self, primary_self: &str) -> Result<(), VaultError> {
        let primary_self = validate_managed_ura(primary_self, "runtime signing owner")?;
        if self.entries.contains_key(&primary_self) {
            return Ok(());
        }

        self.mutate_and_seal(|vault| {
            let mut seed = [0u8; ED25519_SEED_LEN];
            OsRng.fill_bytes(&mut seed);
            vault.insert_seed(&primary_self, hex::encode(seed))
        })
    }

    /// Deterministic seed injection exists only for crate unit tests. There is
    /// no production symbol that can import an Ed25519 seed.
    #[cfg(test)]
    pub(crate) fn put(&mut self, primary_self: &str, seed_hex: String) -> Result<(), VaultError> {
        self.insert_seed(primary_self, seed_hex)
    }

    fn insert_seed(&mut self, primary_self: &str, seed_hex: String) -> Result<(), VaultError> {
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
                seed_hex,
            },
        );
        Ok(())
    }

    /// Sign `canonical_bytes` with the keypair owned by `self_ura`.
    /// Returns the 64-byte ed25519 signature.
    #[cfg(test)]
    pub(crate) fn sign(
        &self,
        self_ura: &str,
        canonical_bytes: &[u8],
    ) -> Result<Signature, VaultError> {
        let entry = self.lookup(self_ura)?;
        let signing_key = signing_key_from_entry(entry)?;
        Ok(signing_key.sign(canonical_bytes))
    }

    /// Sign a runtime-owner intent bound to the caller's cached public
    /// projection. This prevents runtime code from silently switching owners
    /// between projection and private-key use.
    pub(crate) fn sign_bound(
        &self,
        self_ura: &str,
        public_key_b64: &str,
        signer_policy_ref: &str,
        canonical_bytes: &[u8],
    ) -> Result<Signature, VaultError> {
        let self_ura = validate_managed_ura(self_ura, "runtime signing owner")?;
        let entry = self.lookup(&self_ura)?;
        let signing_key = signing_key_from_entry(entry)?;
        let expected_public_key_b64 = encode_b64(&signing_key.verifying_key().to_bytes());
        if public_key_b64 != expected_public_key_b64 {
            return Err(VaultError::Policy(
                "runtime signing public projection does not match the owner key".into(),
            ));
        }
        let expected_policy_ref = crate::daemon::identity::signer_policy_ref(
            &self_ura,
            &self_ura,
            &expected_public_key_b64,
        );
        if signer_policy_ref != expected_policy_ref {
            return Err(VaultError::Policy(
                "runtime signing policy reference does not match the owner projection".into(),
            ));
        }
        if canonical_bytes.is_empty() || canonical_bytes.len() > MAX_KEY_SERVICE_CANONICAL_BYTES {
            return Err(VaultError::Policy(format!(
                "runtime signing canonical bytes must contain 1..={MAX_KEY_SERVICE_CANONICAL_BYTES} bytes"
            )));
        }
        Ok(signing_key.sign(canonical_bytes))
    }

    /// Return the public key for `self_ura`. Same lookup rule as
    /// `sign`.
    pub(crate) fn derive_pubkey(&self, self_ura: &str) -> Result<VerifyingKey, VaultError> {
        let entry = self.lookup(self_ura)?;
        let signing_key = signing_key_from_entry(entry)?;
        Ok(signing_key.verifying_key())
    }

    /// List all primary_self URAs the vault holds.
    #[cfg(test)]
    pub(crate) fn list(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    pub(crate) fn owner_count(&self) -> usize {
        self.entries.len()
    }

    /// Return one stable owner-URA-ordered page. Runtime inventory is an
    /// operational projection, never a health probe or an unbounded dump.
    pub(crate) fn list_page(
        &self,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<(Vec<String>, Option<String>), VaultError> {
        let limit = validate_managed_page_limit(limit)?;
        let cursor = cursor
            .map(|value| validate_managed_ura(value, "runtime owner cursor"))
            .transpose()?;
        let mut entries = if let Some(cursor) = cursor.as_deref() {
            self.entries
                .range(cursor.to_string()..)
                .filter(|(owner, _)| owner.as_str() > cursor)
                .take(limit + 1)
                .map(|(owner, _)| owner.clone())
                .collect::<Vec<_>>()
        } else {
            self.entries.keys().take(limit + 1).cloned().collect()
        };
        let has_more = entries.len() > limit;
        entries.truncate(limit);
        let next_cursor = has_more.then(|| {
            entries
                .last()
                .expect("non-empty runtime owner page with continuation")
                .clone()
        });
        Ok((entries, next_cursor))
    }

    /// Forget an entry. Idempotent — forgetting a non-existent
    /// URA returns Ok. The hard-fail variant lives in
    /// `forget_strict` for the rare caller that wants to
    /// distinguish "I just removed it" from "it wasn't there".
    pub(crate) fn forget(&mut self, primary_self: &str) {
        self.entries.remove(primary_self);
    }

    /// Strict variant of `forget`. Errors when the entry is absent.
    #[cfg(test)]
    pub(crate) fn forget_strict(&mut self, primary_self: &str) -> Result<(), VaultError> {
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
    #[cfg(test)]
    pub(crate) fn seal(&self) -> Result<(), VaultError> {
        self.seal_with_directory_sync(crate::daemon::persistence::config::sync_directory)
    }

    fn seal_with_directory_sync<F>(&self, sync: F) -> Result<(), VaultError>
    where
        F: FnOnce(&Path) -> anyhow::Result<()>,
    {
        self.ensure_operational()?;
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

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::daemon::persistence::config::atomic_write_with_permissions_and_sync(
            &self.path,
            &json,
            WritePermissions::OwnerReadWrite,
            sync,
        )?;
        Ok(())
    }

    /// Apply one mutation using the writer's explicit commit state.
    ///
    /// Domain errors and pre-rename persistence failures restore the prior
    /// state. A successful replacement publishes the new state normally. If
    /// rename made the replacement visible but parent-directory fsync failed,
    /// the new in-memory state is retained (matching the visible file) and the
    /// vault fail-stops until restart re-confirms directory durability.
    pub(crate) fn mutate_and_seal<T>(
        &mut self,
        mutation: impl FnOnce(&mut Self) -> Result<T, VaultError>,
    ) -> Result<T, VaultError> {
        self.mutate_and_seal_with_directory_sync(
            mutation,
            crate::daemon::persistence::config::sync_directory,
        )
    }

    fn mutate_and_seal_with_directory_sync<T, M, F>(
        &mut self,
        mutation: M,
        sync: F,
    ) -> Result<T, VaultError>
    where
        M: FnOnce(&mut Self) -> Result<T, VaultError>,
        F: FnOnce(&Path) -> anyhow::Result<()>,
    {
        self.ensure_operational()?;
        let entries_before = self.entries.clone();
        let inventory_before = self.managed_signing.clone();
        match mutation(self) {
            Ok(output) => match self.seal_with_directory_sync(sync) {
                Ok(()) => Ok(output),
                Err(VaultError::Persistence(persistence))
                    if persistence.commit_state()
                        == AtomicWriteCommitState::ReplacementVisibleButDurabilityUncertain =>
                {
                    let error = VaultError::Persistence(persistence);
                    self.fail_stopped = Some(error.to_string());
                    Err(error)
                }
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

    pub(crate) fn fail_stop_reason(&self) -> Option<&str> {
        self.fail_stopped.as_deref()
    }

    fn ensure_operational(&self) -> Result<(), VaultError> {
        match self.fail_stop_reason() {
            Some(reason) => Err(VaultError::FailStopped(reason.to_string())),
            None => Ok(()),
        }
    }

    /// Return whether an entry exists. Cheap; does not unseal the
    /// keypair.
    #[cfg(test)]
    pub(crate) fn contains(&self, self_ura: &str) -> bool {
        self.entries.contains_key(self_ura)
    }

    /// Create one subject-bound, rotatable managed signing key. The seed is
    /// generated inside the daemon vault and is never a request or response
    /// field. The returned value is a public projection.
    pub(crate) fn inventory_create(
        &mut self,
        purpose: String,
        bound_subject: Option<String>,
    ) -> Result<ManagedSigningKeyProjection, VaultError> {
        let purpose = validate_managed_text(
            &purpose,
            "managed signing purpose",
            MAX_MANAGED_SIGNING_PURPOSE_BYTES,
        )?;
        let bound_subject = bound_subject
            .map(|subject| validate_managed_ura(&subject, "managed signing subject"))
            .transpose()?;
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
        let insert_at = self
            .managed_signing
            .keys
            .binary_search_by(|existing| existing.key_id.cmp(&key.key_id))
            .unwrap_or_else(|index| index);
        self.managed_signing.keys.insert(insert_at, key);
        Ok(projection)
    }

    /// Return all public metadata in crate unit tests. Production callers use
    /// the bounded page operation below.
    #[cfg(test)]
    pub(crate) fn inventory_list(
        &self,
        purpose: Option<&str>,
        status: Option<ManagedSigningStatus>,
    ) -> Vec<ManagedSigningKeyProjection> {
        self.managed_signing
            .keys
            .iter()
            .filter(|key| purpose.map(|p| key.purpose == p).unwrap_or(true))
            .filter(|key| status.map(|s| key.status == s).unwrap_or(true))
            .map(ManagedSigningKeyProjection::from)
            .collect()
    }

    /// Return one stable, key-ID-ordered page of public projections.
    pub(crate) fn inventory_list_page(
        &self,
        purpose: Option<&str>,
        status: Option<ManagedSigningStatus>,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<(Vec<ManagedSigningKeyProjection>, Option<String>), VaultError> {
        let purpose = purpose
            .map(|value| {
                validate_managed_text(
                    value,
                    "managed signing purpose filter",
                    MAX_MANAGED_SIGNING_PURPOSE_BYTES,
                )
            })
            .transpose()?;
        let cursor = cursor
            .map(|value| {
                validate_managed_text(
                    value,
                    "managed signing cursor",
                    MAX_MANAGED_SIGNING_CURSOR_BYTES,
                )
            })
            .transpose()?;
        let limit = validate_managed_page_limit(limit)?;
        let start = cursor
            .as_deref()
            .map(|cursor| {
                self.managed_signing
                    .keys
                    .partition_point(|key| key.key_id.as_str() <= cursor)
            })
            .unwrap_or(0);
        let mut keys = Vec::with_capacity(limit);
        let mut last_inspected = None;
        let mut has_unexamined = false;
        for (inspected, key) in self.managed_signing.keys.iter().skip(start).enumerate() {
            if inspected == MAX_MANAGED_SIGNING_PAGE_SCAN || keys.len() > limit {
                has_unexamined = true;
                break;
            }
            last_inspected = Some(key.key_id.as_str());
            if purpose
                .as_deref()
                .map(|purpose| key.purpose != purpose)
                .unwrap_or(false)
                || status.map(|status| key.status != status).unwrap_or(false)
            {
                continue;
            }
            keys.push(ManagedSigningKeyProjection::from(key));
        }
        if keys.len() > limit {
            let continuation = keys[limit - 1].key_id.clone();
            keys.truncate(limit);
            return Ok((keys, Some(continuation)));
        }
        let next_cursor = has_unexamined
            .then(|| last_inspected.map(str::to_owned))
            .flatten();
        Ok((keys, next_cursor))
    }

    pub(crate) fn inventory_public_key(
        &self,
        key_id: &str,
    ) -> Result<ManagedSigningKeyProjection, VaultError> {
        let key_id = validate_managed_key_id(key_id)?;
        self.inventory_key(&key_id)
            .map(ManagedSigningKeyProjection::from)
    }

    /// Sign only with an active and unexpired managed key.
    #[cfg(test)]
    pub(crate) fn inventory_sign(
        &self,
        key_id: &str,
        canonical_bytes: &[u8],
    ) -> Result<Signature, VaultError> {
        self.ensure_operational()?;
        let key_id = validate_managed_key_id(key_id)?;
        if canonical_bytes.is_empty() || canonical_bytes.len() > MAX_KEY_SERVICE_CANONICAL_BYTES {
            return Err(VaultError::Policy(format!(
                "managed signing canonical bytes must contain 1..={MAX_KEY_SERVICE_CANONICAL_BYTES} bytes"
            )));
        }
        let key = self.inventory_key(&key_id)?;
        self.ensure_inventory_signable(key)?;
        Ok(managed_signing_key_from(key)?.sign(canonical_bytes))
    }

    /// Sign a typed managed-key intent. The service validates that the caller
    /// is using the immutable subject and daemon-derived policy reference from
    /// the selected public projection before private-key use.
    pub(crate) fn inventory_sign_bound(
        &self,
        key_id: &str,
        expected_purpose: &str,
        subject_ura: &str,
        signer_policy_ref: &str,
        canonical_bytes: &[u8],
    ) -> Result<Signature, VaultError> {
        self.ensure_operational()?;
        let key_id = validate_managed_key_id(key_id)?;
        let expected_purpose = validate_managed_text(
            expected_purpose,
            "managed signing intent purpose",
            MAX_MANAGED_SIGNING_PURPOSE_BYTES,
        )?;
        let subject_ura = validate_managed_ura(subject_ura, "managed signing intent subject")?;
        let signer_policy_ref = validate_managed_text(
            signer_policy_ref,
            "managed signing intent policy reference",
            MAX_MANAGED_SIGNING_CURSOR_BYTES,
        )?;
        let key = self.inventory_key(&key_id)?;
        if key.purpose != expected_purpose {
            return Err(VaultError::Policy(
                "managed signing intent purpose does not match the key projection".into(),
            ));
        }
        if key.bound_subject.as_deref() != Some(subject_ura.as_str()) {
            return Err(VaultError::Policy(
                "managed signing intent subject does not match the immutable key binding".into(),
            ));
        }
        let expected_policy_ref = managed_signer_policy_ref(
            &expected_purpose,
            &subject_ura,
            &key.key_id,
            &key.public_key_b64,
        );
        if signer_policy_ref != expected_policy_ref {
            return Err(VaultError::Policy(
                "managed signing intent policy reference does not match the key projection".into(),
            ));
        }
        if canonical_bytes.is_empty() || canonical_bytes.len() > MAX_KEY_SERVICE_CANONICAL_BYTES {
            return Err(VaultError::Policy(format!(
                "managed signing canonical bytes must contain 1..={MAX_KEY_SERVICE_CANONICAL_BYTES} bytes"
            )));
        }
        self.ensure_inventory_signable(key)?;
        Ok(managed_signing_key_from(key)?.sign(canonical_bytes))
    }

    /// Atomically retire an active predecessor and append its successor.
    pub(crate) fn inventory_rotate(
        &mut self,
        key_id: &str,
    ) -> Result<ManagedSigningKeyProjection, VaultError> {
        let key_id = validate_managed_key_id(key_id)?;
        let predecessor_index = self.inventory_key_index(&key_id)?;
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
        let insert_at = self
            .managed_signing
            .keys
            .binary_search_by(|existing| existing.key_id.cmp(&successor.key_id))
            .unwrap_or_else(|index| index);
        self.managed_signing.keys.insert(insert_at, successor);
        Ok(projection)
    }

    /// Move an active or retired key to its terminal revoked state.
    pub(crate) fn inventory_revoke(&mut self, key_id: &str) -> Result<i64, VaultError> {
        let key_id = validate_managed_key_id(key_id)?;
        let key = self.inventory_key_mut(&key_id)?;
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

    pub(crate) fn inventory_set_expiry(
        &mut self,
        key_id: &str,
        expires_unix_ms: i64,
    ) -> Result<(), VaultError> {
        if expires_unix_ms <= 0 {
            return Err(VaultError::Policy(
                "managed signing expiry must be a positive Unix-millisecond timestamp".into(),
            ));
        }
        let key_id = validate_managed_key_id(key_id)?;
        let key = self.inventory_key_mut(&key_id)?;
        if key.status == ManagedSigningStatus::Revoked {
            return Err(VaultError::Lifecycle(
                "cannot set expiry on a revoked managed signing key".into(),
            ));
        }
        key.expires_unix_ms = Some(expires_unix_ms);
        Ok(())
    }

    /// Bind a key exactly once. Rebinding requires a successor key.
    pub(crate) fn inventory_bind_subject(
        &mut self,
        key_id: &str,
        subject_ura: String,
    ) -> Result<(), VaultError> {
        let key_id = validate_managed_key_id(key_id)?;
        let subject_ura = validate_managed_ura(&subject_ura, "managed signing subject")?;
        let key = self.inventory_key_mut(&key_id)?;
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
    pub(crate) fn inventory_peer_add(
        &mut self,
        peer_ura: String,
        public_key_b64: String,
        via_authority: Option<String>,
    ) -> Result<bool, VaultError> {
        use base64::Engine;
        let peer_ura = validate_managed_ura(&peer_ura, "managed peer URA")?;
        let via_authority = via_authority
            .map(|ura| validate_authority_ura(&ura, "managed peer via-authority URA"))
            .transpose()?;
        let public_key = base64::engine::general_purpose::STANDARD
            .decode(&public_key_b64)
            .map_err(|err| VaultError::Base64(format!("managed peer public key: {err}")))?;
        let _: [u8; 32] = public_key.as_slice().try_into().map_err(|_| {
            VaultError::Policy(format!(
                "managed peer public key length must be 32, got {}",
                public_key.len()
            ))
        })?;
        let public_key_b64 = encode_b64(&public_key);
        let now = chrono::Utc::now().timestamp_millis();
        let fingerprint_b64 = encode_b64(&public_key_fingerprint(&public_key));
        if let Ok(index) = self
            .managed_signing
            .peers
            .binary_search_by(|peer| peer.peer_ura.cmp(&peer_ura))
        {
            let peer = &mut self.managed_signing.peers[index];
            if peer.public_key_b64 != public_key_b64 {
                return Err(VaultError::Policy(format!(
                    "managed peer {} is already bound to a different public key; explicit retrust is required",
                    peer.peer_ura
                )));
            }
            peer.via_authority = via_authority;
            peer.last_seen_unix_ms = now;
            return Ok(false);
        }
        let peer = ManagedPeer {
            peer_ura,
            fingerprint_b64,
            public_key_b64,
            via_authority,
            added_unix_ms: now,
            last_seen_unix_ms: now,
        };
        let insert_at = self
            .managed_signing
            .peers
            .binary_search_by(|existing| existing.peer_ura.cmp(&peer.peer_ura))
            .unwrap_or_else(|index| index);
        self.managed_signing.peers.insert(insert_at, peer);
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn inventory_peer_list(&self) -> Vec<ManagedPeer> {
        self.managed_signing.peers.clone()
    }

    pub(crate) fn inventory_peer_list_page(
        &self,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<(Vec<ManagedPeer>, Option<String>), VaultError> {
        let limit = validate_managed_page_limit(limit)?;
        let cursor = cursor
            .map(|value| validate_managed_ura(value, "managed peer cursor"))
            .transpose()?;
        let start = cursor
            .as_deref()
            .map(|cursor| {
                self.managed_signing
                    .peers
                    .partition_point(|peer| peer.peer_ura.as_str() <= cursor)
            })
            .unwrap_or(0);
        let mut peers = self
            .managed_signing
            .peers
            .iter()
            .skip(start)
            .take(limit + 1)
            .cloned()
            .collect::<Vec<_>>();
        let has_more = peers.len() > limit;
        peers.truncate(limit);
        let next_cursor = has_more.then(|| {
            peers
                .last()
                .expect("non-empty managed peer page with continuation")
                .peer_ura
                .clone()
        });
        Ok((peers, next_cursor))
    }

    fn inventory_key(&self, key_id: &str) -> Result<&ManagedSigningKey, VaultError> {
        self.managed_signing
            .keys
            .binary_search_by(|key| key.key_id.as_str().cmp(key_id))
            .map(|index| &self.managed_signing.keys[index])
            .map_err(|_| VaultError::NotFound(format!("managed signing key {key_id}")))
    }

    fn inventory_key_mut(&mut self, key_id: &str) -> Result<&mut ManagedSigningKey, VaultError> {
        let index = self.inventory_key_index(key_id)?;
        self.managed_signing
            .keys
            .get_mut(index)
            .ok_or_else(|| VaultError::NotFound(format!("managed signing key {key_id}")))
    }

    fn inventory_key_index(&self, key_id: &str) -> Result<usize, VaultError> {
        self.managed_signing
            .keys
            .binary_search_by(|key| key.key_id.as_str().cmp(key_id))
            .map_err(|_| VaultError::NotFound(format!("managed signing key {key_id}")))
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
        self.ensure_operational()?;
        self.entries
            .get(self_ura)
            .ok_or_else(|| VaultError::NotFound(self_ura.to_string()))
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

fn validate_managed_text(value: &str, field: &str, max_bytes: usize) -> Result<String, VaultError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed != value {
        return Err(VaultError::Policy(format!(
            "{field} must be non-empty canonical text without surrounding whitespace"
        )));
    }
    if value.len() > max_bytes {
        return Err(VaultError::Policy(format!(
            "{field} exceeds {max_bytes} bytes"
        )));
    }
    Ok(value.to_string())
}

fn validate_managed_key_id(value: &str) -> Result<String, VaultError> {
    validate_managed_text(
        value,
        "managed signing key ID",
        MAX_MANAGED_SIGNING_KEY_ID_BYTES,
    )
}

fn validate_managed_page_limit(limit: Option<usize>) -> Result<usize, VaultError> {
    let limit = limit.unwrap_or(MAX_MANAGED_SIGNING_PAGE_SIZE);
    if !(1..=MAX_MANAGED_SIGNING_PAGE_SIZE).contains(&limit) {
        return Err(VaultError::Policy(format!(
            "managed signing page limit must be within 1..={MAX_MANAGED_SIGNING_PAGE_SIZE}"
        )));
    }
    Ok(limit)
}

fn validate_managed_ura(value: &str, field: &str) -> Result<String, VaultError> {
    let value = validate_managed_text(value, field, MAX_MANAGED_SIGNING_URA_BYTES)?;
    crate::core::identity::RuntimeIdentityUra::parse(value)
        .map(crate::core::identity::RuntimeIdentityUra::into_string)
        .map_err(|error| {
            VaultError::Policy(format!("{field} is not an admissible runtime URA: {error}"))
        })
}

fn parse_persisted_ura(
    value: &str,
    field: &str,
) -> Result<crate::core::identity::RuntimeIdentityUra, VaultError> {
    if value.len() > MAX_MANAGED_SIGNING_URA_BYTES {
        return Err(VaultError::Corrupt(format!(
            "{field} exceeds {MAX_MANAGED_SIGNING_URA_BYTES} bytes"
        )));
    }
    if value.trim() != value {
        return Err(VaultError::Corrupt(format!(
            "{field} contains surrounding whitespace"
        )));
    }
    crate::core::identity::RuntimeIdentityUra::parse(value)
        .map_err(|error| VaultError::Corrupt(format!("{field} is not admissible: {error}")))
}

fn validate_persisted_ura(value: &str, field: &str) -> Result<(), VaultError> {
    parse_persisted_ura(value, field).map(|_| ())
}

fn validate_authority_ura(value: &str, field: &str) -> Result<String, VaultError> {
    let value = validate_managed_ura(value, field)?;
    let parsed = crate::core::ura::parse_ura(&value)
        .map_err(|error| VaultError::Policy(format!("{field} is not a canonical URA: {error}")))?;
    if parsed.kind != crate::core::ura::URAKind::Authority {
        return Err(VaultError::Policy(format!(
            "{field} must be an Authority URA"
        )));
    }
    Ok(value)
}

/// Purpose-aware, versioned policy binding for managed signers.
///
/// This policy reference authenticates the public authority projection. It
/// does not transform or re-canonicalise the payload: the signature remains
/// over the exact canonical bytes supplied by Axon/downstream runtimes.
pub(crate) fn managed_signer_policy_ref(
    purpose: &str,
    subject_ura: &str,
    key_id: &str,
    public_key_b64: &str,
) -> String {
    use sha2::Digest as _;

    let mut hasher = sha2::Sha256::new();
    for component in [
        "canonical-runtime.managed-signing.policy",
        "v2",
        purpose,
        subject_ura,
        key_id,
        public_key_b64,
    ] {
        hasher.update(component.as_bytes());
        hasher.update(b"\0");
    }
    let digest = hasher.finalize();
    format!("managed-signing:v2:sha256:{}", hex::encode(&digest[..16]))
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

// ── Wire protocol ────────────────────────────────────────────────
//
// The key-service process speaks length-prefixed JSON over its local
// transport. The process runtime and typed clients share these crate-private
// shapes; downstream SDKs are pinned by conformance fixtures, not Rust types.

/// Request from a keyring client (backend / daemon / CLI).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum KeyringRequest {
    /// Constant-size protocol/liveness probe. Health never enumerates keys.
    Health {},
    /// Ensure a runtime owner has a signing identity. The keyring generates
    /// the seed itself when absent and returns only the corresponding public
    /// key. Callers never provide or receive private key material.
    Ensure { primary_self: String },
    /// Sign canonical bytes with the keypair indexed by
    /// `self_ura`.
    Sign {
        self_ura: String,
        public_key_b64: String,
        signer_policy_ref: String,
        canonical_bytes_b64: String,
    },
    /// Return the public key for `self_ura`.
    DerivePubkey { self_ura: String },
    /// Return one bounded runtime-owner inventory page.
    #[serde(rename = "runtime.list")]
    RuntimeList {
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        cursor: Option<String>,
    },
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
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        cursor: Option<String>,
    },
    /// Return one managed key's public projection.
    #[serde(rename = "inventory.public_key")]
    InventoryPublicKey { key_id: String },
    /// Sign canonical bytes with one managed key.
    #[serde(rename = "inventory.sign")]
    InventorySign {
        key_id: String,
        expected_purpose: String,
        subject_ura: String,
        signer_policy_ref: String,
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
        via_authority: Option<String>,
    },
    /// List trusted peer public projections.
    #[serde(rename = "inventory.peer_list")]
    InventoryPeerList {
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        cursor: Option<String>,
    },
}

/// Response to a `KeyringRequest`. Errors are typed so the client
/// can pattern-match without parsing strings.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum KeyringResponse {
    Health {
        protocol_version: u32,
    },
    Ok,
    Signature {
        signature_b64: String,
    },
    PublicKey {
        public_key_b64: String,
    },
    RuntimeEntries {
        entries: Vec<String>,
        next_cursor: Option<String>,
    },
    InventoryKey {
        entry: ManagedSigningKeyProjection,
    },
    InventoryKeys {
        entries: Vec<ManagedSigningKeyProjection>,
        next_cursor: Option<String>,
    },
    InventoryRevoked {
        revoked_unix_ms: i64,
    },
    InventoryPeerAdded {
        added: bool,
    },
    InventoryPeers {
        peers: Vec<ManagedPeer>,
        next_cursor: Option<String>,
    },
    Error {
        kind: String,
        message: String,
    },
}

impl KeyringResponse {
    pub(crate) fn err(kind: &str, message: impl Into<String>) -> Self {
        KeyringResponse::Error {
            kind: kind.into(),
            message: message.into(),
        }
    }
}

/// Map a `VaultError` to the wire `KeyringResponse::Error` variant.
pub(crate) fn vault_error_to_response(err: VaultError) -> KeyringResponse {
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
        VaultError::Persistence(error) => match error.commit_state() {
            AtomicWriteCommitState::NotCommitted => "io",
            AtomicWriteCommitState::ReplacementVisibleButDurabilityUncertain => {
                "durability_uncertain"
            }
        },
        VaultError::FailStopped(_) => "fail_stopped",
    };
    KeyringResponse::err(kind, err.to_string())
}

/// Default local endpoint for the one per-user key-service authority. It is
/// deliberately not owner-derived: all runtime owners share one custody
/// process and are separated by typed owner-bound capabilities.
pub const DEFAULT_KEYRING_SOCKET_REL: &str = ".easynet/keyring.sock";

/// Default vault file path.
pub const DEFAULT_VAULT_REL: &str = ".easynet/keyring.enc";

/// Single canonical passphrase file owned and read only by the key-service
/// process. It is deliberately not an environment-configurable public API.
const DEFAULT_PASSPHRASE_REL: &str = ".easynet/keyring.pass";

/// Resolve a `~/.easynet/...` path against an explicit `$HOME`.
pub fn home_relative(rel: &str) -> anyhow::Result<PathBuf> {
    home_relative_from(rel, std::env::var_os("HOME").as_deref())
}

pub(super) fn home_relative_from(rel: &str, home: Option<&OsStr>) -> anyhow::Result<PathBuf> {
    let home = home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("HOME is required for daemon key-service custody paths"))?;
    Ok(home.join(rel))
}

/// Default transport endpoint for the keyring daemon.
pub fn default_socket_path() -> PathBuf {
    try_default_socket_path().expect("resolve default daemon key-service socket path")
}

/// Fallible default transport endpoint for daemon-owned lifecycle startup.
pub fn try_default_socket_path() -> anyhow::Result<PathBuf> {
    if let Some(path) =
        std::env::var_os("EASYNET_KEYRING_SOCKET_PATH").filter(|path| !path.is_empty())
    {
        return Ok(PathBuf::from(path));
    }

    #[cfg(windows)]
    {
        return Ok(PathBuf::from(
            crate::support::platform::named_pipe::scoped_pipe_name("keyring"),
        ));
    }

    #[cfg(not(windows))]
    home_relative(DEFAULT_KEYRING_SOCKET_REL)
}

/// Default encrypted vault path owned exclusively by the key-service process.
pub fn default_vault_path() -> PathBuf {
    try_default_vault_path().expect("resolve default daemon key-service vault path")
}

/// Fallible default encrypted vault path owned by the key-service process.
pub fn try_default_vault_path() -> anyhow::Result<PathBuf> {
    home_relative(DEFAULT_VAULT_REL)
}

fn try_default_passphrase_path() -> anyhow::Result<PathBuf> {
    home_relative(DEFAULT_PASSPHRASE_REL)
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

    fn fresh_seed_hex() -> String {
        let mut seed = [0u8; ED25519_SEED_LEN];
        OsRng.fill_bytes(&mut seed);
        hex::encode(seed)
    }

    fn explicit_pass() -> MasterKeySource {
        MasterKeySource::Explicit("test-passphrase-which-is-long-enough".into())
    }

    #[test]
    fn home_relative_rejects_missing_home_before_cwd_fallback() {
        let error = home_relative_from(DEFAULT_VAULT_REL, None)
            .expect_err("missing HOME must fail before resolving key-service custody path");

        assert!(
            error
                .to_string()
                .contains("HOME is required for daemon key-service custody paths"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn home_relative_rejects_blank_home_before_cwd_fallback() {
        let error = home_relative_from(DEFAULT_VAULT_REL, Some(OsStr::new("")))
            .expect_err("blank HOME must fail before resolving key-service custody path");

        assert!(
            error
                .to_string()
                .contains("HOME is required for daemon key-service custody paths"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn home_relative_resolves_against_explicit_home() {
        let path = home_relative_from(DEFAULT_VAULT_REL, Some(OsStr::new("/tmp/easynet-home")))
            .expect("explicit HOME should resolve");

        assert_eq!(
            path,
            PathBuf::from("/tmp/easynet-home/.easynet/keyring.enc")
        );
    }

    #[test]
    fn put_sign_verify_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let device_ura = "easynet:///r/localhost/device/dev-uuid".to_string();
        vault.put(&device_ura, fresh_seed_hex()).unwrap();
        vault.seal().unwrap();

        let pubkey = vault.derive_pubkey(&device_ura).unwrap();
        let msg = b"axiom canonical bytes";
        let sig = vault.sign(&device_ura, msg).unwrap();
        pubkey.verify(msg, &sig).expect("sig verifies");
    }

    #[test]
    fn device_and_authority_owners_use_distinct_keypairs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let device_ura = "easynet:///r/localhost/device/dev-uuid".to_string();
        let authority_ura = crate::core::ura::hub_ura("localhost");
        vault.put(&device_ura, fresh_seed_hex()).unwrap();
        vault.put(&authority_ura, fresh_seed_hex()).unwrap();

        let pubkey_via_device = vault.derive_pubkey(&device_ura).unwrap();
        let pubkey_via_authority = vault.derive_pubkey(&authority_ura).unwrap();
        assert_ne!(
            pubkey_via_device.to_bytes(),
            pubkey_via_authority.to_bytes(),
            "Device and Authority owners must never share a keypair"
        );
    }

    #[test]
    fn ensure_never_aliases_device_and_authority_owner() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let device_ura = crate::core::ura::device_ura("localhost", "dev-uuid");
        let authority_ura = crate::core::ura::hub_ura("localhost");

        vault.ensure(&device_ura).unwrap();
        assert!(matches!(
            vault.derive_pubkey(&authority_ura),
            Err(VaultError::NotFound(_))
        ));
        vault.ensure(&authority_ura).unwrap();
        assert_ne!(
            vault.derive_pubkey(&device_ura).unwrap().to_bytes(),
            vault.derive_pubkey(&authority_ura).unwrap().to_bytes()
        );
    }

    #[test]
    fn managed_keyring_subject_rejects_all_zero_user_before_persistence() {
        let error = validate_managed_ura(
            "easynet:///r/localhost/user/00000000-0000-0000-0000-000000000000",
            "managed signing subject",
        )
        .expect_err("all-zero User must not become a managed keyring subject");
        assert!(
            error.to_string().contains("all-zero principal placeholder"),
            "wrong keyring validation error: {error}"
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
            vault.put(&device_ura, seed.clone()).unwrap();
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
                .put("easynet:///r/r/device/u", fresh_seed_hex())
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
        vault.put(&ura, fresh_seed_hex()).unwrap();
        let err = vault.put(&ura, fresh_seed_hex()).unwrap_err();
        assert!(matches!(err, VaultError::AlreadyExists(_)));
    }

    #[test]
    fn forget_then_sign_fails() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let ura = "easynet:///r/r/device/u".to_string();
        vault.put(&ura, fresh_seed_hex()).unwrap();
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
        vault.put(&ura, fresh_seed_hex()).unwrap();
        vault.forget_strict(&ura).expect("removes once");
        let err = vault.forget_strict(&ura).unwrap_err();
        assert!(matches!(err, VaultError::NotFound(_)));
    }

    #[test]
    fn bad_seed_len_rejected() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let err = vault.put("u", "deadbeef".into()).unwrap_err(); // 4 bytes
        assert!(matches!(err, VaultError::BadSeedLen { got: 4 }));
    }

    #[test]
    fn seal_writes_mode_0600_on_unix() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        vault
            .put("easynet:///r/r/device/u", fresh_seed_hex())
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
        vault.put("z-ura", fresh_seed_hex()).unwrap();
        vault.put("a-ura", fresh_seed_hex()).unwrap();
        vault.put("m-ura", fresh_seed_hex()).unwrap();
        assert_eq!(vault.list(), vec!["a-ura", "m-ura", "z-ura"]);
    }

    #[test]
    fn runtime_owner_inventory_is_bounded_and_cursor_ordered() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        for index in 0..(MAX_MANAGED_SIGNING_PAGE_SIZE + 2) {
            vault
                .put(
                    &crate::core::ura::device_ura("r", &format!("device-{index:02}")),
                    fresh_seed_hex(),
                )
                .unwrap();
        }
        let (first, cursor) = vault.list_page(None, None).unwrap();
        assert_eq!(first.len(), MAX_MANAGED_SIGNING_PAGE_SIZE);
        let cursor = cursor.expect("first runtime-owner page continuation");
        let (second, terminal) = vault.list_page(None, Some(&cursor)).unwrap();
        assert_eq!(second.len(), 2);
        assert!(terminal.is_none());
        assert!(first.last().unwrap() < second.first().unwrap());
    }

    #[test]
    fn contains_matches_only_the_exact_owner() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        vault.put("primary", fresh_seed_hex()).unwrap();
        assert!(vault.contains("primary"));
        assert!(!vault.contains("overlay"));
        assert!(!vault.contains("ghost"));
    }

    #[test]
    fn managed_signing_state_machine_preserves_subject_and_blocks_retired_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let subject = "easynet:///r/test.local/agent/alice.main".to_string();
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
    fn managed_signing_inventory_pages_are_bounded_and_stable() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        for index in 0..(MAX_MANAGED_SIGNING_PAGE_SIZE + 3) {
            vault
                .inventory_create(format!("purpose-{index}"), None)
                .unwrap();
        }

        let (first, cursor) = vault.inventory_list_page(None, None, None, None).unwrap();
        assert_eq!(first.len(), MAX_MANAGED_SIGNING_PAGE_SIZE);
        let cursor = cursor.expect("first page continuation");
        let (second, terminal) = vault
            .inventory_list_page(None, None, None, Some(&cursor))
            .unwrap();
        assert_eq!(second.len(), 3);
        assert!(terminal.is_none());
        assert!(first.last().unwrap().key_id < second.first().unwrap().key_id);
    }

    #[test]
    fn managed_signing_rejects_noncanonical_subject_ura() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let error = vault
            .inventory_create("invocation".into(), Some("not-a-ura".into()))
            .unwrap_err();
        assert!(matches!(error, VaultError::Policy(_)));
    }

    #[test]
    fn managed_signing_requires_exact_subject_and_policy_intent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let subject = "easynet:///r/test.local/agent/alice.main";
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let key = vault
            .inventory_create("invocation".into(), Some(subject.into()))
            .unwrap();
        let policy = key.signer_policy_ref.as_deref().unwrap();
        assert!(vault
            .inventory_sign_bound(&key.key_id, &key.purpose, subject, policy, b"canonical")
            .is_ok());
        assert!(matches!(
            vault.inventory_sign_bound(
                &key.key_id,
                &key.purpose,
                "easynet:///r/test.local/agent/bob.main",
                policy,
                b"canonical",
            ),
            Err(VaultError::Policy(_))
        ));
        assert!(matches!(
            vault.inventory_sign_bound(
                &key.key_id,
                &key.purpose,
                subject,
                "wrong-policy",
                b"canonical",
            ),
            Err(VaultError::Policy(_))
        ));
        assert!(matches!(
            vault.inventory_sign_bound(
                &key.key_id,
                "different-purpose",
                subject,
                policy,
                b"canonical",
            ),
            Err(VaultError::Policy(_))
        ));
    }

    #[test]
    fn managed_signer_policy_v2_matches_cross_language_fixture() {
        let public_key_b64 = encode_b64(
            &SigningKey::from_bytes(&[1u8; ED25519_SEED_LEN])
                .verifying_key()
                .to_bytes(),
        );
        assert_eq!(
            managed_signer_policy_ref(
                "invocation",
                "easynet:///r/acme/agent/signer",
                "managed-key-1",
                &public_key_b64,
            ),
            "managed-signing:v2:sha256:e7e82ca6208b6a4ebf2369739a2c260a"
        );
    }

    #[test]
    fn runtime_signing_requires_exact_public_projection_and_policy() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let owner = "easynet:///r/test.local/authority";
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        vault.ensure(owner).unwrap();
        let public_key_b64 = encode_b64(&vault.derive_pubkey(owner).unwrap().to_bytes());
        let policy = crate::daemon::identity::signer_policy_ref(owner, owner, &public_key_b64);
        assert!(vault
            .sign_bound(owner, &public_key_b64, &policy, b"canonical")
            .is_ok());
        assert!(matches!(
            vault.sign_bound(owner, &encode_b64(&[9; 32]), &policy, b"canonical"),
            Err(VaultError::Policy(_))
        ));
        assert!(matches!(
            vault.sign_bound(owner, &public_key_b64, "wrong-policy", b"canonical"),
            Err(VaultError::Policy(_))
        ));
    }

    #[test]
    fn filtered_inventory_scan_is_bounded_and_cursor_can_advance_empty_page() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        for _ in 0..=MAX_MANAGED_SIGNING_PAGE_SCAN {
            vault.inventory_create("other".into(), None).unwrap();
        }
        vault
            .managed_signing
            .keys
            .last_mut()
            .expect("inventory contains keys")
            .purpose = "target".into();

        let (first, cursor) = vault
            .inventory_list_page(Some("target"), None, None, None)
            .unwrap();
        assert!(first.is_empty());
        let cursor = cursor.expect("bounded scan must advance its cursor");
        let (second, terminal) = vault
            .inventory_list_page(Some("target"), None, None, Some(&cursor))
            .unwrap();
        assert_eq!(second.len(), 1);
        assert!(terminal.is_none());
    }

    #[test]
    fn managed_signing_revoke_expiry_and_binding_are_terminal_or_immutable() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let mut vault = Vault::open_or_init(&path, &explicit_pass()).unwrap();
        let key = vault.inventory_create("invocation".into(), None).unwrap();
        vault
            .inventory_bind_subject(
                &key.key_id,
                "easynet:///r/test.local/agent/alice.main".into(),
            )
            .unwrap();
        assert!(matches!(
            vault.inventory_bind_subject(
                &key.key_id,
                "easynet:///r/test.local/agent/bob.main".into(),
            ),
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
            .inventory_create(
                "invocation".into(),
                Some("easynet:///r/r/agent/alice.main".into()),
            )
            .unwrap();
        std::fs::create_dir(&path).unwrap();

        let error = vault
            .mutate_and_seal(|candidate| candidate.inventory_rotate(&original.key_id))
            .expect_err("renaming a file over a directory must fail");
        assert!(matches!(
            error,
            VaultError::Persistence(ref persistence)
                if persistence.commit_state() == AtomicWriteCommitState::NotCommitted
        ));

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
    fn post_rename_sync_failure_keeps_visible_state_and_fail_stops_until_reopen() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let owner = "easynet:///r/r/device/runtime";
        let subject = "easynet:///r/r/agent/alice.main";
        let mut vault = Vault::init(&path, &explicit_pass()).unwrap();
        vault.ensure(owner).unwrap();
        let original = vault
            .mutate_and_seal(|candidate| {
                candidate.inventory_create("invocation".into(), Some(subject.into()))
            })
            .unwrap();

        let error = vault
            .mutate_and_seal_with_directory_sync(
                |candidate| candidate.inventory_rotate(&original.key_id),
                |_| anyhow::bail!("injected post-rename directory fsync failure"),
            )
            .unwrap_err();
        assert!(matches!(
            error,
            VaultError::Persistence(ref persistence)
                if persistence.commit_state()
                    == AtomicWriteCommitState::ReplacementVisibleButDurabilityUncertain
        ));

        let keys = vault.inventory_list(None, None);
        assert_eq!(keys.len(), 2, "visible replacement must remain in memory");
        assert_eq!(
            keys.iter()
                .find(|key| key.key_id == original.key_id)
                .unwrap()
                .status,
            ManagedSigningStatus::Retired
        );
        let successor = keys
            .iter()
            .find(|key| key.key_id != original.key_id)
            .unwrap()
            .clone();
        assert!(vault.fail_stop_reason().is_some());
        assert!(matches!(
            vault.inventory_sign(&successor.key_id, b"canonical"),
            Err(VaultError::FailStopped(_))
        ));
        assert!(matches!(
            vault.derive_pubkey(owner),
            Err(VaultError::FailStopped(_))
        ));

        drop(vault);
        let reopened = Vault::open(&path, &explicit_pass()).unwrap();
        assert!(reopened.fail_stop_reason().is_none());
        assert_eq!(
            reopened
                .inventory_public_key(&successor.key_id)
                .unwrap()
                .status,
            ManagedSigningStatus::Active
        );
        assert!(reopened
            .inventory_sign(&successor.key_id, b"canonical")
            .is_ok());
    }

    #[test]
    fn vault_open_rejects_legacy_v1_and_noncanonical_plaintext_shapes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let source = explicit_pass();
        let vault = Vault::init(&path, &source).unwrap();

        let write_encrypted_shape = |plaintext: &[u8], version: u32| {
            let nonce = [7u8; AES_NONCE_LEN];
            let file = KeyringFile {
                version,
                kdf_salt_b64: encode_b64(&vault.salt),
                vault_nonce_b64: encode_b64(&nonce),
                vault_ciphertext_b64: encode_b64(
                    &encrypt(&vault.master_key, &nonce, plaintext).unwrap(),
                ),
            };
            std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        };

        write_encrypted_shape(
            br#"{"entries":[{"primary_self":"easynet:///r/example/device/host","seed_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]}"#,
            1,
        );
        let error = Vault::open(&path, &source).expect_err("legacy v1 vaults are not supported");
        assert!(matches!(error, VaultError::Corrupt(_)));
        let persisted_file: KeyringFile =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            persisted_file.version, 1,
            "rejected legacy files must not be rewritten"
        );

        write_encrypted_shape(br#"{"entries":[]}"#, KeyringFile::CURRENT_VERSION);
        assert!(matches!(
            Vault::open(&path, &source),
            Err(VaultError::Corrupt(_))
        ));

        write_encrypted_shape(
            br#"{"entries":[],"managed_signing":{"keys":[],"peers":[]},"legacy":true}"#,
            KeyringFile::CURRENT_VERSION,
        );
        assert!(matches!(
            Vault::open(&path, &source),
            Err(VaultError::Corrupt(_))
        ));

        write_encrypted_shape(
            br#"{"entries":[{"primary_self":"easynet:///r/example/user/00000000-0000-0000-0000-000000000000","seed_hex":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}],"managed_signing":{"keys":[],"peers":[]}}"#,
            KeyringFile::CURRENT_VERSION,
        );
        assert!(matches!(
            Vault::open(&path, &source),
            Err(VaultError::Corrupt(_))
        ));

        write_encrypted_shape(
            br#"{"entries":[],"managed_signing":{"keys":[],"peers":[{"peer_ura":"easynet:///r/example/user/00000000-0000-0000-0000-000000000000","fingerprint_b64":"fingerprint","public_key_b64":"public","via_authority":null,"added_unix_ms":1,"last_seen_unix_ms":1}]}}"#,
            KeyringFile::CURRENT_VERSION,
        );
        assert!(matches!(
            Vault::open(&path, &source),
            Err(VaultError::Corrupt(_))
        ));
    }

    #[test]
    fn key_service_dtos_reject_unknown_fields() {
        assert!(
            serde_json::from_str::<KeyringRequest>(r#"{"method":"health","legacy":true}"#).is_err()
        );

        let projection = ManagedSigningKeyProjection {
            key_id: "msk-test".into(),
            purpose: "invocation".into(),
            public_key_b64: encode_b64(&[1u8; 32]),
            status: ManagedSigningStatus::Active,
            rotation_epoch: 0,
            bound_subject: None,
            signer_policy_ref: None,
            rotated_from: None,
            created_unix_ms: 1,
            expires_unix_ms: None,
            revoked_unix_ms: None,
        };
        let mut value = serde_json::to_value(projection).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("legacy".into(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<ManagedSigningKeyProjection>(value).is_err());
    }

    #[test]
    fn pre_derived_master_key_path() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("keyring.enc");
        let key = [42u8; AES_KEY_LEN];
        let source = MasterKeySource::PreDerived(key);
        let mut vault = Vault::open_or_init(&path, &source).unwrap();
        vault
            .put("easynet:///r/r/device/u", fresh_seed_hex())
            .unwrap();
        vault.seal().unwrap();
        let _re = Vault::open(&path, &source).expect("pre-derived round trip");
    }

    #[test]
    fn keyring_request_response_serde_round_trip() {
        let req = KeyringRequest::Sign {
            self_ura: "easynet:///r/r/device/u".into(),
            public_key_b64: "public".into(),
            signer_policy_ref: "policy".into(),
            canonical_bytes_b64: encode_b64(b"hello"),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: KeyringRequest = serde_json::from_str(&json).unwrap();
        match back {
            KeyringRequest::Sign {
                self_ura,
                public_key_b64,
                signer_policy_ref,
                canonical_bytes_b64,
            } => {
                assert_eq!(self_ura, "easynet:///r/r/device/u");
                assert_eq!(public_key_b64, "public");
                assert_eq!(signer_policy_ref, "policy");
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
