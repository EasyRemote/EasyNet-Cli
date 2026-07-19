// EasyNet CLI - canonical runtime identity client
// =================================================
//
// File: src/daemon/identity/self_identity.rs
//
// Typed sign-only handle for daemon-custodied Ed25519 identities. Every
// runtime consumer calls `SelfIdentity::sign(self_ura,
// canonical_bytes) -> Signature` instead of holding the seed
// itself. Backed by the `easynet-keyring` daemon's UDS. The trait
// is intentionally narrow: a caller can sign and read public
// keys, full stop. There is no API to extract the seed.
//
// Backends
// --------
// `KeyringClient` — production. Connects to the keyring daemon at
//   ~/.easynet/keyring.sock and speaks the length-prefixed JSON
//   wire from `crate::daemon::keyring`.
// `InMemoryVault` — test-only backend. Wraps a `daemon::keyring::Vault`
//   directly so unit tests don't need to spawn a daemon.
//
// Why a trait
// -----------
// Boot wiring varies: device-mode daemon runs alongside the
// keyring daemon, so it uses `KeyringClient`. Production has no
// in-process vault or file-backed fallback. `Arc<dyn SelfIdentity>`
// keeps consumers independent of the transport implementation.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ed25519_dalek::Verifier as _;
use ed25519_dalek::{Signature, VerifyingKey};

#[cfg(unix)]
use std::os::unix::net::UnixStream;

#[cfg(test)]
use crate::daemon::keyring::Vault;
use crate::daemon::keyring::{
    default_socket_path, managed_signer_policy_ref, KeyringRequest, KeyringResponse, ManagedPeer,
    ManagedSigningKeyProjection, ManagedSigningStatus, KEY_SERVICE_PROTOCOL_VERSION,
    MAX_KEY_SERVICE_AUTO_ITEMS, MAX_KEY_SERVICE_AUTO_PAGES, MAX_KEY_SERVICE_CANONICAL_BYTES,
    MAX_KEY_SERVICE_FRAME_BYTES, MAX_MANAGED_SIGNING_PAGE_SIZE,
};

pub const USER_SIGNING_CLI_PURPOSE: &str = "user_signing.cli";

/// Errors surfaced by `SelfIdentity` callers. Most are 1:1 with
/// the keyring daemon's typed responses; transport-level failures
/// (socket missing, broken pipe) get their own variant so callers
/// can decide whether to retry or fall back.
#[derive(Debug, thiserror::Error)]
pub enum SelfIdentityError {
    #[error("self-identity: owner URA is required")]
    InvalidOwner,
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
/// `self_ura` (e.g. `easynet:///r/<realm>/authority` or
/// `easynet:///r/<realm>/device/<uuid>`) and the canonical bytes
/// to sign; gets back a 64-byte ed25519 signature.
pub trait SelfIdentity: Send + Sync {
    /// Sign `canonical_bytes` with the keypair indexed by
    /// `self_ura`. Each owner URA has a distinct key; no role-alias lookup is
    /// permitted inside the custody service.
    fn sign(&self, self_ura: &str, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError>;

    /// Sign against an already-resolved public projection. Runtime consumers
    /// use this capability-bound path so projection and private-key selection
    /// cannot diverge between boot and signing.
    fn sign_bound(
        &self,
        self_ura: &str,
        expected_public_key: &VerifyingKey,
        canonical_bytes: &[u8],
    ) -> Result<Signature, SelfIdentityError> {
        let actual = self.public_key(self_ura)?;
        if actual != *expected_public_key {
            return Err(SelfIdentityError::Rejected {
                kind: "policy".into(),
                message: "runtime signing public projection changed before signing".into(),
            });
        }
        self.sign(self_ura, canonical_bytes)
    }

    /// Return the public key for `self_ura`. Useful at boot when
    /// the daemon needs to publish its pubkey into the realm
    /// directory or write a trust-anchor entry.
    fn public_key(&self, self_ura: &str) -> Result<VerifyingKey, SelfIdentityError>;

    /// Best-effort health probe. Backends should override when they expose a
    /// constant-size liveness operation.
    fn ping(&self) -> Result<(), SelfIdentityError> {
        let _ = self.public_key("__ping__")?;
        Ok(())
    }
}

/// Owner-bound signing capability shared by every runtime consumer.
///
/// `SelfIdentity` is the daemon key-service port and can address more than one
/// owner. Runtime components must not receive that authority directly: they
/// receive this narrower capability, permanently bound to one owner URA.
/// Consequently admission, dispatch, and session code can request signatures
/// without selecting an arbitrary identity or observing private material.
#[async_trait::async_trait]
pub trait CanonicalSigner: Send + Sync {
    fn owner_ura(&self) -> &str;

    /// Sign canonical bytes without blocking the caller's async executor.
    ///
    /// Production implementations may cross a process boundary. Keeping this
    /// operation asynchronous prevents key-service UDS latency from occupying
    /// Tokio worker threads while preserving an owner-bound, object-safe port.
    async fn sign_canonical(&self, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError>;

    fn signing_public_key(&self) -> Result<VerifyingKey, SelfIdentityError>;
}

/// Canonical runtime projection of one daemon-custodied signing identity.
///
/// The public key is resolved once when the capability is bound. Signing
/// remains a call through the key-service port; no seed or private-key bytes
/// ever enter this object.
#[derive(Clone)]
pub struct RuntimeSigningIdentity {
    owner_ura: Arc<str>,
    public_key: VerifyingKey,
    provider: Arc<dyn SelfIdentity>,
}

impl std::fmt::Debug for RuntimeSigningIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeSigningIdentity")
            .field("owner_ura", &self.owner_ura)
            .field("public_key", &hex::encode(self.public_key.to_bytes()))
            .finish_non_exhaustive()
    }
}

impl RuntimeSigningIdentity {
    /// Resolve one owner through the canonical local key-service endpoint.
    pub fn load_default(owner_ura: impl Into<String>) -> Result<Self, SelfIdentityError> {
        Self::load(owner_ura, Arc::new(KeyringClient::default_path()))
    }

    /// Resolve and bind an existing daemon-owned identity.
    pub fn load(
        owner_ura: impl Into<String>,
        provider: Arc<dyn SelfIdentity>,
    ) -> Result<Self, SelfIdentityError> {
        let owner_ura = owner_ura.into();
        let owner_ura = owner_ura.trim();
        if owner_ura.is_empty() {
            return Err(SelfIdentityError::InvalidOwner);
        }
        let public_key = provider.public_key(owner_ura)?;
        Ok(Self::from_public_projection(
            owner_ura, public_key, provider,
        ))
    }

    /// Bind a public projection returned by an atomic key-service operation
    /// such as `ensure`, avoiding a redundant lookup round trip.
    fn from_public_projection(
        owner_ura: impl Into<String>,
        public_key: VerifyingKey,
        provider: Arc<dyn SelfIdentity>,
    ) -> Self {
        Self {
            owner_ura: Arc::from(owner_ura.into()),
            public_key,
            provider,
        }
    }
}

#[async_trait::async_trait]
impl CanonicalSigner for RuntimeSigningIdentity {
    fn owner_ura(&self) -> &str {
        &self.owner_ura
    }

    async fn sign_canonical(&self, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError> {
        validate_canonical_signing_bytes(canonical_bytes)?;
        let provider = Arc::clone(&self.provider);
        let owner_ura = Arc::clone(&self.owner_ura);
        let canonical_bytes = canonical_bytes.to_vec();
        let public_key = self.public_key;
        tokio::task::spawn_blocking(move || {
            provider.sign_bound(&owner_ura, &public_key, &canonical_bytes)
        })
        .await
        .map_err(|error| {
            SelfIdentityError::Transport(format!(
                "key-service signing worker terminated unexpectedly: {error}"
            ))
        })?
    }

    fn signing_public_key(&self) -> Result<VerifyingKey, SelfIdentityError> {
        Ok(self.public_key)
    }
}

/// Caller signer resolver used by runtime invocation facades.
///
/// Device and authority callers are runtime owners and keep the canonical
/// `self_ura -> keypair` lookup. User callers are different: the CLI
/// provisions them as managed, subject-bound signing keys so multiple user
/// devices can coexist without pretending the user is a daemon runtime owner.
pub fn load_runtime_caller_signer(
    owner_ura: impl Into<String>,
) -> Result<Arc<dyn CanonicalSigner>, SelfIdentityError> {
    let owner_ura = owner_ura.into();
    let provider = Arc::new(KeyringClient::default_path());
    if is_user_owner_ura(&owner_ura) {
        return ManagedRuntimeSigningIdentity::load_user(owner_ura, provider)
            .map(|signer| Arc::new(signer) as Arc<dyn CanonicalSigner>);
    }
    let self_identity_provider: Arc<dyn SelfIdentity> = provider.clone();
    RuntimeSigningIdentity::load(owner_ura, self_identity_provider)
        .map(|signer| Arc::new(signer) as Arc<dyn CanonicalSigner>)
}

fn is_user_owner_ura(owner_ura: &str) -> bool {
    crate::core::ura::parse_ura(owner_ura)
        .map(|parsed| parsed.kind == crate::core::ura::URAKind::User)
        .unwrap_or(false)
}

#[derive(Clone)]
struct ManagedRuntimeSigningIdentity {
    owner_ura: Arc<str>,
    projection: ManagedSigningKeyProjection,
    public_key: VerifyingKey,
    provider: Arc<KeyringClient>,
}

impl ManagedRuntimeSigningIdentity {
    fn load_user(
        owner_ura: impl Into<String>,
        provider: Arc<KeyringClient>,
    ) -> Result<Self, SelfIdentityError> {
        let owner_ura = owner_ura.into();
        let owner_ura = owner_ura.trim();
        if owner_ura.is_empty() {
            return Err(SelfIdentityError::InvalidOwner);
        }
        let projection = provider
            .inventory_list(
                Some(USER_SIGNING_CLI_PURPOSE.to_string()),
                Some(ManagedSigningStatus::Active),
            )?
            .into_iter()
            .find(|entry| entry.bound_subject.as_deref() == Some(owner_ura))
            .ok_or_else(|| SelfIdentityError::Rejected {
                kind: "not_found".into(),
                message: format!("managed user signing key not found for `{owner_ura}`"),
            })?;
        let public_key = decode_public_key(projection.public_key_b64.clone())?;
        Ok(Self {
            owner_ura: Arc::from(owner_ura.to_string()),
            projection,
            public_key,
            provider,
        })
    }
}

#[async_trait::async_trait]
impl CanonicalSigner for ManagedRuntimeSigningIdentity {
    fn owner_ura(&self) -> &str {
        &self.owner_ura
    }

    async fn sign_canonical(&self, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError> {
        validate_canonical_signing_bytes(canonical_bytes)?;
        let provider = Arc::clone(&self.provider);
        let projection = self.projection.clone();
        let canonical_bytes = canonical_bytes.to_vec();
        tokio::task::spawn_blocking(move || {
            provider.inventory_sign_bound(&projection, &canonical_bytes)
        })
        .await
        .map_err(|error| {
            SelfIdentityError::Transport(format!(
                "managed key-service signing worker terminated unexpectedly: {error}"
            ))
        })?
    }

    fn signing_public_key(&self) -> Result<VerifyingKey, SelfIdentityError> {
        Ok(self.public_key)
    }
}

/// Deterministic in-process signer for unit tests only. Production code must
/// bind [`RuntimeSigningIdentity`] to the daemon key-service port.
#[cfg(test)]
pub(crate) struct TestCanonicalSigner {
    owner_ura: String,
    signing_key: ed25519_dalek::SigningKey,
}

#[cfg(test)]
impl TestCanonicalSigner {
    pub(crate) fn new(owner_ura: impl Into<String>, seed: [u8; 32]) -> Self {
        Self {
            owner_ura: owner_ura.into(),
            signing_key: ed25519_dalek::SigningKey::from_bytes(&seed),
        }
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl CanonicalSigner for TestCanonicalSigner {
    fn owner_ura(&self) -> &str {
        &self.owner_ura
    }

    async fn sign_canonical(&self, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError> {
        use ed25519_dalek::Signer as _;
        Ok(self.signing_key.sign(canonical_bytes))
    }

    fn signing_public_key(&self) -> Result<VerifyingKey, SelfIdentityError> {
        Ok(self.signing_key.verifying_key())
    }
}

// ── KeyringClient (UDS to easynet-keyring) ─────────────────────

/// UDS-backed client. Holds the socket path and a
/// connect-per-request strategy (the daemon's accept loop reads
/// many requests per connection, but the simple client opens a
/// fresh stream each call to keep the surface area small —
/// signing is not on a hot loop).
///
/// Each RPC owns its connection and can therefore run independently. There is
/// deliberately no client-wide mutex: serialising unrelated requests would
/// create head-of-line blocking without protecting shared transport state.
pub struct KeyringClient {
    socket_path: PathBuf,
    timeout: Duration,
}

impl KeyringClient {
    /// Ensure a runtime identity exists inside the daemon custody boundary and
    /// return only its public projection.
    pub fn ensure(&self, primary_self: &str) -> Result<VerifyingKey, SelfIdentityError> {
        match self.rpc(&KeyringRequest::Ensure {
            primary_self: primary_self.to_string(),
        })? {
            KeyringResponse::PublicKey { public_key_b64 } => decode_public_key(public_key_b64),
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }

    /// Construct against an explicit socket path. Operators with a
    /// non-default key-service layout build via this constructor.
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            timeout: Duration::from_secs(10),
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
        let deadline = Instant::now()
            .checked_add(self.timeout)
            .unwrap_or_else(Instant::now);

        #[cfg(unix)]
        let mut stream = {
            UnixStream::connect(&self.socket_path).map_err(|e| {
                SelfIdentityError::DaemonOffline {
                    path: self.socket_path.clone(),
                    reason: e.to_string(),
                }
            })?
        };

        #[cfg(windows)]
        let mut stream = {
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
        if body.len() > MAX_KEY_SERVICE_FRAME_BYTES || body.len() > u32::MAX as usize {
            return Err(SelfIdentityError::Framing(format!(
                "request frame {} > max {MAX_KEY_SERVICE_FRAME_BYTES}",
                body.len()
            )));
        }
        let len = (body.len() as u32).to_be_bytes();
        #[cfg(unix)]
        set_write_deadline(&stream, deadline, "write request length")?;
        stream
            .write_all(&len)
            .map_err(|e| SelfIdentityError::Transport(format!("write len: {e}")))?;
        #[cfg(unix)]
        set_write_deadline(&stream, deadline, "write request body")?;
        stream
            .write_all(&body)
            .map_err(|e| SelfIdentityError::Transport(format!("write body: {e}")))?;
        #[cfg(unix)]
        set_write_deadline(&stream, deadline, "flush request")?;
        stream
            .flush()
            .map_err(|e| SelfIdentityError::Transport(format!("flush: {e}")))?;

        let mut len_buf = [0u8; 4];
        #[cfg(unix)]
        set_read_deadline(&stream, deadline, "read response length")?;
        stream
            .read_exact(&mut len_buf)
            .map_err(|e| SelfIdentityError::Transport(format!("read len: {e}")))?;
        let resp_len = u32::from_be_bytes(len_buf) as usize;
        if resp_len > MAX_KEY_SERVICE_FRAME_BYTES {
            return Err(SelfIdentityError::Framing(format!(
                "response frame {resp_len} > max {MAX_KEY_SERVICE_FRAME_BYTES}"
            )));
        }
        let mut buf = vec![0u8; resp_len];
        #[cfg(unix)]
        set_read_deadline(&stream, deadline, "read response body")?;
        stream
            .read_exact(&mut buf)
            .map_err(|e| SelfIdentityError::Transport(format!("read body: {e}")))?;
        serde_json::from_slice::<KeyringResponse>(&buf)
            .map_err(|e| SelfIdentityError::Framing(format!("decode response: {e}")))
    }
}

#[cfg(unix)]
fn remaining_request_budget(
    deadline: Instant,
    operation: &str,
) -> Result<Duration, SelfIdentityError> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| {
            SelfIdentityError::Transport(format!(
                "key-service request deadline exceeded before {operation}"
            ))
        })?;

    // `SO_RCVTIMEO`/`SO_SNDTIMEO` are timeval-backed on Unix. macOS rejects
    // a non-zero `Duration` that truncates to a zero timeval with EINVAL.
    // Round the final sub-millisecond budget up to the smallest portable
    // socket-timeout quantum; the absolute deadline is still checked before
    // every I/O phase.
    Ok(remaining.max(Duration::from_millis(1)))
}

#[cfg(unix)]
fn set_write_deadline(
    stream: &UnixStream,
    deadline: Instant,
    operation: &str,
) -> Result<(), SelfIdentityError> {
    let remaining = remaining_request_budget(deadline, operation)?;
    match stream.set_write_timeout(Some(remaining)) {
        Ok(()) => Ok(()),
        Err(error) if socket_timeout_rejected_after_peer_close(&error) => Ok(()),
        Err(error) => Err(SelfIdentityError::Transport(format!(
            "set write deadline ({remaining:?} remaining): {error}"
        ))),
    }
}

#[cfg(unix)]
fn set_read_deadline(
    stream: &UnixStream,
    deadline: Instant,
    operation: &str,
) -> Result<(), SelfIdentityError> {
    let remaining = remaining_request_budget(deadline, operation)?;
    match stream.set_read_timeout(Some(remaining)) {
        Ok(()) => Ok(()),
        Err(error) if socket_timeout_rejected_after_peer_close(&error) => Ok(()),
        Err(error) => Err(SelfIdentityError::Transport(format!(
            "set read deadline ({remaining:?} remaining): {error}"
        ))),
    }
}

#[cfg(unix)]
fn socket_timeout_rejected_after_peer_close(error: &std::io::Error) -> bool {
    // Darwin returns EINVAL when SO_RCVTIMEO/SO_SNDTIMEO is applied after the
    // peer has already closed a Unix-domain stream, even while the response is
    // still buffered locally. The following read/write is guaranteed to
    // complete immediately with buffered bytes or EOF/EPIPE, so ignoring this
    // one platform-specific terminal-socket condition cannot extend the
    // absolute request deadline.
    cfg!(target_os = "macos") && error.raw_os_error() == Some(libc::EINVAL)
}

impl SelfIdentity for KeyringClient {
    fn sign(&self, self_ura: &str, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError> {
        validate_canonical_signing_bytes(canonical_bytes)?;
        let public_key = self.public_key(self_ura)?;
        self.sign_bound(self_ura, &public_key, canonical_bytes)
    }

    fn sign_bound(
        &self,
        self_ura: &str,
        expected_public_key: &VerifyingKey,
        canonical_bytes: &[u8],
    ) -> Result<Signature, SelfIdentityError> {
        use base64::Engine;
        validate_canonical_signing_bytes(canonical_bytes)?;
        let public_key_b64 =
            base64::engine::general_purpose::STANDARD.encode(expected_public_key.to_bytes());
        let req = KeyringRequest::Sign {
            self_ura: self_ura.to_string(),
            signer_policy_ref: crate::daemon::identity::signer_policy_ref(
                self_ura,
                self_ura,
                &public_key_b64,
            ),
            public_key_b64,
            canonical_bytes_b64: base64::engine::general_purpose::STANDARD.encode(canonical_bytes),
        };
        match self.rpc(&req)? {
            KeyringResponse::Signature { signature_b64 } => {
                let signature = decode_signature(signature_b64)?;
                expected_public_key
                    .verify(canonical_bytes, &signature)
                    .map_err(|error| SelfIdentityError::SignatureDecode(format!(
                        "daemon signature does not verify against the bound runtime projection: {error}"
                    )))?;
                Ok(signature)
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
            KeyringResponse::PublicKey { public_key_b64 } => decode_public_key(public_key_b64),
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }

    fn ping(&self) -> Result<(), SelfIdentityError> {
        self.health()
    }
}

impl KeyringClient {
    /// Constant-size protocol and liveness probe.
    pub fn health(&self) -> Result<(), SelfIdentityError> {
        let req = KeyringRequest::Health {};
        match self.rpc(&req)? {
            KeyringResponse::Health {
                protocol_version: KEY_SERVICE_PROTOCOL_VERSION,
            } => Ok(()),
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
        let mut entries = Vec::new();
        let mut cursor = None;
        for _page in 0..MAX_KEY_SERVICE_AUTO_PAGES {
            match self.rpc(&KeyringRequest::RuntimeList {
                limit: Some(MAX_MANAGED_SIGNING_PAGE_SIZE),
                cursor: cursor.clone(),
            })? {
                KeyringResponse::RuntimeEntries {
                    entries: page,
                    next_cursor,
                } => {
                    if entries.len().saturating_add(page.len()) > MAX_KEY_SERVICE_AUTO_ITEMS {
                        return Err(SelfIdentityError::Unexpected(
                            "runtime owner inventory exceeded bounded collection limit".into(),
                        ));
                    }
                    entries.extend(page);
                    match next_cursor {
                        Some(next) if cursor.as_deref() != Some(next.as_str()) => {
                            cursor = Some(next)
                        }
                        Some(_) => {
                            return Err(SelfIdentityError::Unexpected(
                                "runtime owner inventory returned a repeated cursor".into(),
                            ))
                        }
                        None => return Ok(entries),
                    }
                }
                KeyringResponse::Error { kind, message } => {
                    return Err(SelfIdentityError::Rejected { kind, message })
                }
                other => return Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
            }
        }
        Err(SelfIdentityError::Unexpected(
            "runtime owner inventory exceeded bounded page limit".into(),
        ))
    }

    /// Create a managed signing key wholly inside the daemon service.
    pub fn inventory_create(
        &self,
        purpose: impl Into<String>,
        bound_subject: Option<String>,
    ) -> Result<ManagedSigningKeyProjection, SelfIdentityError> {
        match self.rpc(&KeyringRequest::InventoryCreate {
            purpose: purpose.into(),
            bound_subject,
        })? {
            KeyringResponse::InventoryKey { entry } => Ok(entry),
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }

    pub fn inventory_list(
        &self,
        purpose: Option<String>,
        status: Option<ManagedSigningStatus>,
    ) -> Result<Vec<ManagedSigningKeyProjection>, SelfIdentityError> {
        let mut entries = Vec::new();
        let mut cursor = None;
        for _page_index in 0..MAX_KEY_SERVICE_AUTO_PAGES {
            match self.rpc(&KeyringRequest::InventoryList {
                purpose: purpose.clone(),
                status,
                limit: Some(MAX_MANAGED_SIGNING_PAGE_SIZE),
                cursor: cursor.clone(),
            })? {
                KeyringResponse::InventoryKeys {
                    entries: page,
                    next_cursor,
                } => {
                    if entries.len().saturating_add(page.len()) > MAX_KEY_SERVICE_AUTO_ITEMS {
                        return Err(SelfIdentityError::Unexpected(
                            "managed signing inventory exceeded bounded collection limit".into(),
                        ));
                    }
                    entries.extend(page);
                    match next_cursor {
                        Some(next) if cursor.as_deref() != Some(next.as_str()) => {
                            cursor = Some(next)
                        }
                        Some(_) => {
                            return Err(SelfIdentityError::Unexpected(
                                "managed signing inventory returned a repeated cursor".into(),
                            ))
                        }
                        None => return Ok(entries),
                    }
                }
                KeyringResponse::Error { kind, message } => {
                    return Err(SelfIdentityError::Rejected { kind, message })
                }
                other => return Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
            }
        }
        Err(SelfIdentityError::Unexpected(
            "managed signing inventory exceeded bounded page limit".into(),
        ))
    }

    pub fn inventory_public_key(
        &self,
        key_id: &str,
    ) -> Result<ManagedSigningKeyProjection, SelfIdentityError> {
        match self.rpc(&KeyringRequest::InventoryPublicKey {
            key_id: key_id.to_string(),
        })? {
            KeyringResponse::InventoryKey { entry } => Ok(entry),
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }

    pub(crate) fn inventory_sign(
        &self,
        key_id: &str,
        canonical_bytes: &[u8],
    ) -> Result<Signature, SelfIdentityError> {
        validate_canonical_signing_bytes(canonical_bytes)?;
        let projection = self.inventory_public_key(key_id)?;
        self.inventory_sign_bound(&projection, canonical_bytes)
    }

    pub(crate) fn inventory_sign_bound(
        &self,
        projection: &ManagedSigningKeyProjection,
        canonical_bytes: &[u8],
    ) -> Result<Signature, SelfIdentityError> {
        use base64::Engine;
        validate_canonical_signing_bytes(canonical_bytes)?;
        let subject_ura =
            projection
                .bound_subject
                .clone()
                .ok_or_else(|| SelfIdentityError::Rejected {
                    kind: "policy".into(),
                    message: "managed signing key is not bound to a subject".into(),
                })?;
        let signer_policy_ref =
            projection
                .signer_policy_ref
                .clone()
                .ok_or_else(|| SelfIdentityError::Rejected {
                    kind: "policy".into(),
                    message: "managed signing key has no signer policy reference".into(),
                })?;
        let expected_policy_ref = managed_signer_policy_ref(
            &projection.purpose,
            &subject_ura,
            &projection.key_id,
            &projection.public_key_b64,
        );
        if signer_policy_ref != expected_policy_ref {
            return Err(SelfIdentityError::Rejected {
                kind: "policy".into(),
                message:
                    "managed signing projection has a non-canonical purpose-aware policy reference"
                        .into(),
            });
        }
        let verifying_key = decode_public_key(projection.public_key_b64.clone())?;
        match self.rpc(&KeyringRequest::InventorySign {
            key_id: projection.key_id.clone(),
            expected_purpose: projection.purpose.clone(),
            subject_ura,
            signer_policy_ref,
            canonical_bytes_b64: base64::engine::general_purpose::STANDARD.encode(canonical_bytes),
        })? {
            KeyringResponse::Signature { signature_b64 } => {
                let signature = decode_signature(signature_b64)?;
                verifying_key
                    .verify(canonical_bytes, &signature)
                    .map_err(|error| SelfIdentityError::SignatureDecode(format!(
                        "daemon signature does not verify against the managed key projection: {error}"
                    )))?;
                Ok(signature)
            }
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }

    pub fn inventory_rotate(
        &self,
        key_id: &str,
    ) -> Result<ManagedSigningKeyProjection, SelfIdentityError> {
        match self.rpc(&KeyringRequest::InventoryRotate {
            key_id: key_id.to_string(),
        })? {
            KeyringResponse::InventoryKey { entry } => Ok(entry),
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }

    pub fn inventory_revoke(&self, key_id: &str) -> Result<i64, SelfIdentityError> {
        match self.rpc(&KeyringRequest::InventoryRevoke {
            key_id: key_id.to_string(),
        })? {
            KeyringResponse::InventoryRevoked { revoked_unix_ms } => Ok(revoked_unix_ms),
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }

    pub fn inventory_set_expiry(
        &self,
        key_id: &str,
        expires_unix_ms: i64,
    ) -> Result<(), SelfIdentityError> {
        self.inventory_ok(KeyringRequest::InventorySetExpiry {
            key_id: key_id.to_string(),
            expires_unix_ms,
        })
    }

    pub fn inventory_bind_subject(
        &self,
        key_id: &str,
        subject_ura: &str,
    ) -> Result<(), SelfIdentityError> {
        self.inventory_ok(KeyringRequest::InventoryBindSubject {
            key_id: key_id.to_string(),
            subject_ura: subject_ura.to_string(),
        })
    }

    pub fn inventory_peer_add(
        &self,
        peer_ura: &str,
        public_key_b64: &str,
        via_hub: Option<String>,
    ) -> Result<bool, SelfIdentityError> {
        match self.rpc(&KeyringRequest::InventoryPeerAdd {
            peer_ura: peer_ura.to_string(),
            public_key_b64: public_key_b64.to_string(),
            via_hub,
        })? {
            KeyringResponse::InventoryPeerAdded { added } => Ok(added),
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }

    pub fn inventory_peer_list(&self) -> Result<Vec<ManagedPeer>, SelfIdentityError> {
        let mut peers = Vec::new();
        let mut cursor = None;
        for _page_index in 0..MAX_KEY_SERVICE_AUTO_PAGES {
            match self.rpc(&KeyringRequest::InventoryPeerList {
                limit: Some(MAX_MANAGED_SIGNING_PAGE_SIZE),
                cursor: cursor.clone(),
            })? {
                KeyringResponse::InventoryPeers {
                    peers: page,
                    next_cursor,
                } => {
                    if peers.len().saturating_add(page.len()) > MAX_KEY_SERVICE_AUTO_ITEMS {
                        return Err(SelfIdentityError::Unexpected(
                            "managed peer inventory exceeded bounded collection limit".into(),
                        ));
                    }
                    peers.extend(page);
                    match next_cursor {
                        Some(next) if cursor.as_deref() != Some(next.as_str()) => {
                            cursor = Some(next)
                        }
                        Some(_) => {
                            return Err(SelfIdentityError::Unexpected(
                                "managed peer inventory returned a repeated cursor".into(),
                            ))
                        }
                        None => return Ok(peers),
                    }
                }
                KeyringResponse::Error { kind, message } => {
                    return Err(SelfIdentityError::Rejected { kind, message })
                }
                other => return Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
            }
        }
        Err(SelfIdentityError::Unexpected(
            "managed peer inventory exceeded bounded page limit".into(),
        ))
    }

    fn inventory_ok(&self, request: KeyringRequest) -> Result<(), SelfIdentityError> {
        match self.rpc(&request)? {
            KeyringResponse::Ok => Ok(()),
            KeyringResponse::Error { kind, message } => {
                Err(SelfIdentityError::Rejected { kind, message })
            }
            other => Err(SelfIdentityError::Unexpected(format!("{other:?}"))),
        }
    }
}

/// Provision the daemon's synthetic loopback caller in the canonical key
/// service. The caller remains daemon-internal, but its signing authority is
/// subject to the same custody, projection, and lifecycle rules as every
/// other runtime owner.
pub fn ensure_daemon_local_system_identity(
    client: &KeyringClient,
) -> Result<(), SelfIdentityError> {
    client.ensure(crate::core::ura::LOCAL_SYSTEM_AGENT_URA)?;
    Ok(())
}

fn decode_signature(signature_b64: String) -> Result<Signature, SelfIdentityError> {
    use base64::Engine;
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

fn decode_public_key(public_key_b64: String) -> Result<VerifyingKey, SelfIdentityError> {
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

fn validate_canonical_signing_bytes(canonical_bytes: &[u8]) -> Result<(), SelfIdentityError> {
    if canonical_bytes.is_empty() || canonical_bytes.len() > MAX_KEY_SERVICE_CANONICAL_BYTES {
        return Err(SelfIdentityError::Rejected {
            kind: "policy".into(),
            message: format!(
                "canonical signing bytes must contain 1..={MAX_KEY_SERVICE_CANONICAL_BYTES} bytes"
            ),
        });
    }
    Ok(())
}

// ── InMemoryVault (test-only) ──────────────────────────────────

/// `SelfIdentity` impl backed by an in-process `Vault` under a `Mutex`.
/// This type does not exist in production builds.
#[cfg(test)]
pub(crate) struct InMemoryVault {
    vault: Mutex<Vault>,
}

#[cfg(test)]
impl InMemoryVault {
    pub(crate) fn new(vault: Vault) -> Self {
        Self {
            vault: Mutex::new(vault),
        }
    }
}

#[cfg(test)]
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::keyring::{MasterKeySource, Vault};
    use base64::Engine as _;
    use ed25519_dalek::Verifier;
    use rand::rngs::OsRng;
    use rand::RngCore;
    use tempfile::TempDir;

    fn seed_hex() -> String {
        let mut s = [0u8; 32];
        OsRng.fill_bytes(&mut s);
        hex::encode(s)
    }

    fn make_in_memory_vault(ura: &str) -> InMemoryVault {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("v.enc");
        let pass = MasterKeySource::Explicit("test-passphrase-for-self-identity".into());
        let mut v = Vault::open_or_init(&path, &pass).unwrap();
        v.put(ura, seed_hex()).unwrap();
        std::mem::forget(dir); // keep tempdir alive for vault path
        InMemoryVault::new(v)
    }

    struct ThreadRecordingIdentity {
        signing_key: ed25519_dalek::SigningKey,
        sign_thread: Mutex<Option<std::thread::ThreadId>>,
    }

    impl SelfIdentity for ThreadRecordingIdentity {
        fn sign(
            &self,
            _self_ura: &str,
            canonical_bytes: &[u8],
        ) -> Result<Signature, SelfIdentityError> {
            use ed25519_dalek::Signer as _;

            *self
                .sign_thread
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) =
                Some(std::thread::current().id());
            Ok(self.signing_key.sign(canonical_bytes))
        }

        fn public_key(&self, _self_ura: &str) -> Result<VerifyingKey, SelfIdentityError> {
            Ok(self.signing_key.verifying_key())
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_signer_offloads_blocking_provider_from_tokio_thread() {
        let provider = Arc::new(ThreadRecordingIdentity {
            signing_key: ed25519_dalek::SigningKey::from_bytes(&[0x42; 32]),
            sign_thread: Mutex::new(None),
        });
        let provider_port: Arc<dyn SelfIdentity> = provider.clone();
        let signer =
            RuntimeSigningIdentity::load("easynet:///r/r/device/u".to_string(), provider_port)
                .expect("bind test provider");
        let tokio_thread = std::thread::current().id();

        let signature = signer
            .sign_canonical(b"canonical")
            .await
            .expect("sign through blocking provider");
        signer
            .signing_public_key()
            .expect("cached public projection")
            .verify(b"canonical", &signature)
            .expect("signature verifies");
        let sign_thread = provider
            .sign_thread
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .expect("provider recorded signing thread");
        assert_ne!(
            sign_thread, tokio_thread,
            "synchronous key-service providers must run on Tokio's blocking pool"
        );
    }

    #[test]
    fn in_memory_vault_signs_and_verifies() {
        let id = make_in_memory_vault("easynet:///r/r/device/u");
        let pk = id.public_key("easynet:///r/r/device/u").unwrap();
        let sig = id.sign("easynet:///r/r/device/u", b"hello").unwrap();
        pk.verify(b"hello", &sig).unwrap();
    }

    #[test]
    fn in_memory_runtime_owner_cannot_sign_as_hub() {
        let device = "easynet:///r/r/device/u";
        let hub = crate::core::ura::hub_ura("r");
        let id = make_in_memory_vault(device);
        assert!(id.public_key(device).is_ok());
        assert!(matches!(
            id.public_key(&hub),
            Err(SelfIdentityError::Rejected { .. })
        ));
    }

    #[test]
    fn in_memory_unknown_ura_rejected() {
        let id = make_in_memory_vault("known");
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

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn managed_user_runtime_signer_signs_with_subject_bound_inventory_key() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("key-service.sock");
        let vault_path = temp.path().join("key-service.enc");
        let user_ura = "easynet:///r/acme/user/alice";
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let socket_for_server = socket.clone();
        let server = std::thread::spawn(move || {
            crate::daemon::keyring::service::run_test_unix_key_service_with_purpose(
                socket_for_server,
                vault_path,
                "test-passphrase".to_string(),
                user_ura.to_string(),
                USER_SIGNING_CLI_PURPOSE.to_string(),
                2,
                ready_tx,
            );
        });
        let projection = ready_rx
            .recv()
            .expect("test key service reports readiness")
            .expect("test key service starts");
        let provider = Arc::new(KeyringClient::new(socket));
        let signer = ManagedRuntimeSigningIdentity::load_user(user_ura, provider)
            .expect("managed user signer loads");

        assert_eq!(signer.owner_ura(), user_ura);
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .encode(signer.signing_public_key().unwrap().to_bytes()),
            projection.public_key_b64
        );
        let signature = signer
            .sign_canonical(b"canonical user call")
            .await
            .expect("managed signer signs");
        signer
            .signing_public_key()
            .unwrap()
            .verify(b"canonical user call", &signature)
            .expect("managed signature verifies");
        server.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn deadline_configuration_accepts_a_peer_that_already_closed() {
        let (stream, peer) = UnixStream::pair().unwrap();
        drop(peer);
        let deadline = Instant::now() + Duration::from_secs(1);
        set_read_deadline(&stream, deadline, "closed-peer response").unwrap();
        set_write_deadline(&stream, deadline, "closed-peer request").unwrap();
    }
}
