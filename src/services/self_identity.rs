// EasyNet CLI — SelfIdentity client (RFC-001 plan v4.1.5 Phase 3B)
// =================================================================
//
// File: src/services/self_identity.rs
//
// Typed sign-only handle for the device's Ed25519 keypair. Every
// EasyNet process on a host (the Rust daemon, the CLI, future
// host-mode tooling) calls `SelfIdentity::sign(self_ura,
// canonical_bytes) -> Signature` instead of holding the seed
// itself. Backed by the `easynet-keyring` daemon's UDS. The trait
// is intentionally narrow: a caller can sign and read public
// keys, full stop. There is no API to extract the seed.
//
// Backends
// --------
// `KeyringClient` — production. Connects to the keyring daemon at
//   ~/.easynet/keyring.sock and speaks the length-prefixed JSON
//   wire from `crate::services::keyring`.
// `InMemoryVault` — test backend. Wraps a `services::keyring::Vault`
//   directly so unit tests don't need to spawn a daemon. Behaves
//   identically except no IPC.
//
// Why a trait
// -----------
// Boot wiring varies: device-mode daemon runs alongside the
// keyring daemon, so it uses `KeyringClient`. Headless production
// without a keyring daemon (Phase 3F future work) will plug in a
// file-backed fallback. `Arc<dyn SelfIdentity>` lets boot pick the
// right impl without leaking the choice to every callsite.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use ed25519_dalek::{Signature, VerifyingKey};

#[cfg(unix)]
use std::os::unix::net::UnixStream;

use crate::services::keyring::{
    default_socket_path, KeyringRequest, KeyringResponse, MasterKeySource, Vault,
};

/// Errors surfaced by `SelfIdentity` callers. Most are 1:1 with
/// the keyring daemon's typed responses; transport-level failures
/// (socket missing, broken pipe) get their own variant so callers
/// can decide whether to retry or fall back.
#[derive(Debug, thiserror::Error)]
pub enum SelfIdentityError {
    #[error("self-identity: keyring daemon offline at {path}: {reason}")]
    DaemonOffline { path: PathBuf, reason: String },
    #[error("self-identity: keyring transport: {0}")]
    Transport(String),
    #[error("self-identity: keyring framing: {0}")]
    Framing(String),
    #[error("self-identity: keyring rejected request: kind={kind}, msg={message}")]
    Rejected { kind: String, message: String },
    #[error("self-identity: unexpected response variant: {0}")]
    Unexpected(String),
    #[error("self-identity: signature decode: {0}")]
    SignatureDecode(String),
    #[error("self-identity: pubkey decode: {0}")]
    PublicKeyDecode(String),
}

/// The minimal contract every callsite needs. Caller passes a
/// `self_ura` (e.g. `easynet:///r/<realm>/hub` or
/// `easynet:///r/<realm>/device/<uuid>`) and the canonical bytes
/// to sign; gets back a 64-byte ed25519 signature.
pub trait SelfIdentity: Send + Sync {
    /// Sign `canonical_bytes` with the keypair indexed by
    /// `self_ura`. Role-overlay lookup applies on the keyring
    /// side: a vault entry's `primary_self` and any of its
    /// `role_overlays` resolve to the same keypair.
    fn sign(&self, self_ura: &str, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError>;

    /// Return the public key for `self_ura`. Useful at boot when
    /// the daemon needs to publish its pubkey into the realm
    /// directory or write a trust-anchor entry.
    fn public_key(&self, self_ura: &str) -> Result<VerifyingKey, SelfIdentityError>;

    /// Best-effort health probe. Default is a `list` round-trip.
    /// Backends override if they have a cheaper check.
    fn ping(&self) -> Result<(), SelfIdentityError> {
        let _ = self.public_key("__ping__")?;
        Ok(())
    }
}

// ── KeyringClient (UDS to easynet-keyring) ─────────────────────

/// UDS-backed client. Holds the socket path and a serialised
/// connect-per-request strategy (the daemon's accept loop reads
/// many requests per connection, but the simple client opens a
/// fresh stream each call to keep the surface area small —
/// signing is not on a hot loop).
///
/// The `Mutex` exists because the underlying request/response on
/// any one connection is sequential; serialising at the client
/// layer means callers can share a single client `Arc` from many
/// threads without writing their own locking.
pub struct KeyringClient {
    socket_path: PathBuf,
    timeout: Duration,
    // Lock used only to serialise socket open + framing on a
    // single inner connection; a future enhancement could hold a
    // pool of connections, but v1 connect-per-call is plenty for
    // the daemon's signing volume (one envelope at a time on the
    // signed-invoke hot path).
    lock: Mutex<()>,
}

impl KeyringClient {
    /// Construct against an explicit socket path. Operators with
    /// a non-default keyring layout (e.g. Phase 3E's
    /// per-device-uuid path) build via this constructor.
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            timeout: Duration::from_secs(10),
            lock: Mutex::new(()),
        }
    }

    /// Construct against the default `~/.easynet/keyring.sock`
    /// path. Same default as `easynet-keyring` daemon's bind
    /// path so the typical case is "just works".
    pub fn default_path() -> Self {
        Self::new(default_socket_path())
    }

    /// Override the per-call timeout. Default is 10s.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn rpc(&self, req: &KeyringRequest) -> Result<KeyringResponse, SelfIdentityError> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());

        #[cfg(unix)]
        let mut stream = {
            let stream = UnixStream::connect(&self.socket_path).map_err(|e| {
                SelfIdentityError::DaemonOffline {
                    path: self.socket_path.clone(),
                    reason: e.to_string(),
                }
            })?;
            stream
                .set_read_timeout(Some(self.timeout))
                .map_err(|e| SelfIdentityError::Transport(format!("set_read_timeout: {e}")))?;
            stream
                .set_write_timeout(Some(self.timeout))
                .map_err(|e| SelfIdentityError::Transport(format!("set_write_timeout: {e}")))?;
            stream
        };

        #[cfg(windows)]
        let mut stream = {
            let deadline = std::time::Instant::now() + self.timeout;
            loop {
                match std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&self.socket_path)
                {
                    Ok(file) => break file,
                    Err(err)
                        if std::time::Instant::now() < deadline
                            && (err.kind() == std::io::ErrorKind::NotFound
                                || err.raw_os_error() == Some(231)) =>
                    {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(err) => {
                        return Err(SelfIdentityError::DaemonOffline {
                            path: self.socket_path.clone(),
                            reason: err.to_string(),
                        });
                    }
                }
            }
        };

        let body = serde_json::to_vec(req)
            .map_err(|e| SelfIdentityError::Framing(format!("encode request: {e}")))?;
        let len = (body.len() as u32).to_be_bytes();
        stream
            .write_all(&len)
            .map_err(|e| SelfIdentityError::Transport(format!("write len: {e}")))?;
        stream
            .write_all(&body)
            .map_err(|e| SelfIdentityError::Transport(format!("write body: {e}")))?;
        stream
            .flush()
            .map_err(|e| SelfIdentityError::Transport(format!("flush: {e}")))?;

        let mut len_buf = [0u8; 4];
        stream
            .read_exact(&mut len_buf)
            .map_err(|e| SelfIdentityError::Transport(format!("read len: {e}")))?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        if resp_len > 64 * 1024 {
            return Err(SelfIdentityError::Framing(format!(
                "response frame {resp_len} > 64 KiB"
            )));
        }
        let mut buf = vec![0u8; resp_len];
        stream
            .read_exact(&mut buf)
            .map_err(|e| SelfIdentityError::Transport(format!("read body: {e}")))?;
        serde_json::from_slice::<KeyringResponse>(&buf)
            .map_err(|e| SelfIdentityError::Framing(format!("decode response: {e}")))
    }
}

impl SelfIdentity for KeyringClient {
    fn sign(&self, self_ura: &str, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError> {
        use base64::Engine;
        let req = KeyringRequest::Sign {
            self_ura: self_ura.to_string(),
            canonical_bytes_b64: base64::engine::general_purpose::STANDARD.encode(canonical_bytes),
        };
        match self.rpc(&req)? {
            KeyringResponse::Signature { signature_b64 } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(signature_b64)
                    .map_err(|e| SelfIdentityError::SignatureDecode(format!("base64: {e}")))?;
                let arr: [u8; 64] = bytes.as_slice().try_into().map_err(|_| {
                    SelfIdentityError::SignatureDecode(format!(
                        "ed25519 signature must be 64 bytes, got {}",
                        bytes.len()
                    ))
                })?;
                Ok(Signature::from_bytes(&arr))
            }
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }

    fn public_key(&self, self_ura: &str) -> Result<VerifyingKey, SelfIdentityError> {
        let req = KeyringRequest::DerivePubkey {
            self_ura: self_ura.to_string(),
        };
        match self.rpc(&req)? {
            KeyringResponse::PublicKey { public_key_b64 } => {
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(public_key_b64)
                    .map_err(|e| SelfIdentityError::PublicKeyDecode(format!("base64: {e}")))?;
                let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
                    SelfIdentityError::PublicKeyDecode(format!(
                        "ed25519 pubkey must be 32 bytes, got {}",
                        bytes.len()
                    ))
                })?;
                VerifyingKey::from_bytes(&arr)
                    .map_err(|e| SelfIdentityError::PublicKeyDecode(format!("from_bytes: {e}")))
            }
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }

    fn ping(&self) -> Result<(), SelfIdentityError> {
        let req = KeyringRequest::List;
        match self.rpc(&req)? {
            KeyringResponse::List { .. } => Ok(()),
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }
}

/// Convenience: helper for the join flow that needs to put a fresh
/// keypair into the vault. Not part of the `SelfIdentity` trait
/// because routine signing callers must NOT have access to the
/// put/forget surface (principle of least privilege at the API
/// layer).
impl KeyringClient {
    pub fn put(
        &self,
        primary_self: &str,
        role_overlays: Vec<String>,
        seed_hex: String,
    ) -> Result<(), SelfIdentityError> {
        let req = KeyringRequest::Put {
            primary_self: primary_self.to_string(),
            role_overlays,
            seed_hex,
        };
        match self.rpc(&req)? {
            KeyringResponse::Ok => Ok(()),
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }

    pub fn forget(&self, primary_self: &str) -> Result<(), SelfIdentityError> {
        let req = KeyringRequest::Forget {
            primary_self: primary_self.to_string(),
        };
        match self.rpc(&req)? {
            KeyringResponse::Ok => Ok(()),
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }

    pub fn list(&self) -> Result<Vec<String>, SelfIdentityError> {
        match self.rpc(&KeyringRequest::List)? {
            KeyringResponse::List { entries } => Ok(entries),
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }
}

// ── InMemoryVault (test + in-process boot path) ────────────────

/// `SelfIdentity` impl backed by an in-process `Vault` under a
/// `Mutex`. Used in unit tests and in headless deploys that have
/// no keyring daemon (the deploy injects the master key via env
/// and the boot process opens the vault directly).
pub struct InMemoryVault {
    vault: Mutex<Vault>,
}

impl InMemoryVault {
    pub fn new(vault: Vault) -> Self {
        Self {
            vault: Mutex::new(vault),
        }
    }

    pub fn open(path: &Path, source: &MasterKeySource) -> Result<Self, SelfIdentityError> {
        Vault::open(path, source)
            .map(Self::new)
            .map_err(|e| SelfIdentityError::Transport(format!("open vault: {e}")))
    }

    /// Direct mutator. Tests + the headless boot path that owns
    /// the vault use this; the trait deliberately exposes only
    /// signing.
    pub fn put(
        &self,
        primary_self: &str,
        role_overlays: Vec<String>,
        seed_hex: String,
    ) -> Result<(), SelfIdentityError> {
        let mut guard = self.vault.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .put(primary_self, role_overlays, seed_hex)
            .map_err(|e| SelfIdentityError::Rejected {
                kind: "vault".into(),
                message: e.to_string(),
            })
    }
}

impl SelfIdentity for InMemoryVault {
    fn sign(&self, self_ura: &str, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError> {
        let guard = self.vault.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .sign(self_ura, canonical_bytes)
            .map_err(|e| SelfIdentityError::Rejected {
                kind: "vault".into(),
                message: e.to_string(),
            })
    }

    fn public_key(&self, self_ura: &str) -> Result<VerifyingKey, SelfIdentityError> {
        let guard = self.vault.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .derive_pubkey(self_ura)
            .map_err(|e| SelfIdentityError::Rejected {
                kind: "vault".into(),
                message: e.to_string(),
            })
    }
}

// ── Join helper ─────────────────────────────────────────────────
//
// Phase 3C bridge for `easynet device join`. The pairing flow
// receives a fresh `(node_id, realm)` from the hub; this helper
// mints a random Ed25519 seed locally and pushes it into the
// keyring under the canonical device URA plus a hub-role overlay
// so the same keypair signs both. Best-effort: if the keyring
// daemon is offline, log + continue. v4.1.5 deterministic
// derivation in `boot.rs::load_daemon_identity` keeps the daemon
// signing as a fallback so the join itself never fails on
// keyring offline.

/// Build the canonical self URAs for this device. Returns
/// `(primary_self, role_overlays)`. v4.1.4 shape:
///   primary  = `easynet:///r/<realm>/device/<node_id>`
///   overlay  = `easynet:///r/<realm>/hub` (so backend-as-hub on
///              this host signs with the same keypair)
pub fn canonical_self_uras(realm: &str, node_id: &str) -> (String, Vec<String>) {
    let realm = realm.trim();
    let node_id = node_id.trim();
    let primary = crate::ura::device_ura(realm, node_id);
    let hub_overlay = crate::ura::hub_ura(realm);
    (primary, vec![hub_overlay])
}

/// Mint a fresh ed25519 seed (32 random bytes) hex-encoded so it
/// fits the keyring's `seed_hex` field. Each call returns a new
/// keypair; callers persist the result via `KeyringClient::put`.
pub fn fresh_seed_hex() -> String {
    use rand::rngs::OsRng;
    use rand::RngCore;
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    hex::encode(seed)
}

/// Probe whether a keyring daemon is reachable at the default
/// socket path. Used by the join flow to decide whether to push
/// the freshly-minted seed into the vault or fall back silently
/// to deterministic derivation.
pub fn keyring_daemon_available() -> bool {
    KeyringClient::default_path().ping().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::keyring::{MasterKeySource, Vault};
    use ed25519_dalek::Verifier;
    use rand::rngs::OsRng;
    use rand::RngCore;
    use tempfile::TempDir;

    fn seed_hex() -> String {
        let mut s = [0u8; 32];
        OsRng.fill_bytes(&mut s);
        hex::encode(s)
    }

    fn make_in_memory_vault(ura: &str, overlays: Vec<String>) -> InMemoryVault {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("v.enc");
        let pass = MasterKeySource::Explicit("test-passphrase-for-self-identity".into());
        let mut v = Vault::open_or_init(&path, &pass).unwrap();
        v.put(ura, overlays, seed_hex()).unwrap();
        std::mem::forget(dir); // keep tempdir alive for vault path
        InMemoryVault::new(v)
    }

    #[test]
    fn in_memory_vault_signs_and_verifies() {
        let id = make_in_memory_vault("easynet:///r/r/device/u", vec![]);
        let pk = id.public_key("easynet:///r/r/device/u").unwrap();
        let sig = id.sign("easynet:///r/r/device/u", b"hello").unwrap();
        pk.verify(b"hello", &sig).unwrap();
    }

    #[test]
    fn in_memory_role_overlay_signs_with_same_keypair() {
        let device = "easynet:///r/r/device/u";
        let hub = "easynet:///r/r/hub";
        let id = make_in_memory_vault(device, vec![hub.into()]);
        let pk_a = id.public_key(device).unwrap();
        let pk_b = id.public_key(hub).unwrap();
        assert_eq!(pk_a.to_bytes(), pk_b.to_bytes(), "overlay shares keypair");
        let sig_a = id.sign(device, b"x").unwrap();
        let sig_b = id.sign(hub, b"x").unwrap();
        assert_eq!(sig_a.to_bytes(), sig_b.to_bytes());
    }

    #[test]
    fn in_memory_unknown_ura_rejected() {
        let id = make_in_memory_vault("known", vec![]);
        let err = id.sign("unknown", b"x").unwrap_err();
        match err {
            SelfIdentityError::Rejected { kind, .. } => assert_eq!(kind, "vault"),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn keyring_client_daemon_offline_returns_typed_error() {
        let client = KeyringClient::new("/tmp/this-socket-does-not-exist.sock");
        let err = client.sign("u", b"x").unwrap_err();
        match err {
            SelfIdentityError::DaemonOffline { .. } => (),
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
