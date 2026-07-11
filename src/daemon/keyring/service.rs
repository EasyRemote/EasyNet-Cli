//! Canonical key-service runtime.
//!
//! This module is the only production owner of the decrypted vault.  The
//! process entry point supplies transport framing; every mutation and signing
//! decision remains here so binaries, SDKs, and product runtimes can only use
//! the public request/response protocol.  In particular, no constructor or
//! method in this facade accepts or returns an Ed25519 seed.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::{watch, Mutex, Semaphore};
use tokio::time::{timeout, timeout_at, Instant};

#[cfg(windows)]
use crate::support::platform::named_pipe::PipeListener;

use super::passphrase::PassphraseStore;
#[cfg(test)]
use super::ManagedSigningKeyProjection;
use super::{
    default_passphrase_path, default_socket_path, default_vault_path, vault_error_to_response,
    KeyringRequest, KeyringResponse, MasterKeySource, Vault, KEY_SERVICE_PROTOCOL_VERSION,
    MAX_KEY_SERVICE_CANONICAL_BYTES, MAX_KEY_SERVICE_FRAME_BYTES,
};
use crate::daemon::persistence::file_lock::ExclusiveFileLock;

const MAX_CONCURRENT_KEY_SERVICE_CONNECTIONS: usize = 4;
const MAX_REQUESTS_PER_KEY_SERVICE_CONNECTION: usize = 256;
const KEY_SERVICE_FRAME_DEADLINE: Duration = Duration::from_secs(30);
const KEY_SERVICE_TERMINAL_CLOSE_GRACE: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug)]
struct ConnectionPolicy {
    frame_deadline: Duration,
    max_requests: usize,
}

impl Default for ConnectionPolicy {
    fn default() -> Self {
        Self {
            frame_deadline: KEY_SERVICE_FRAME_DEADLINE,
            max_requests: MAX_REQUESTS_PER_KEY_SERVICE_CONNECTION,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("key-service frame deadline elapsed after {budget:?}")]
struct FrameDeadlineElapsed {
    budget: Duration,
}

#[derive(Debug)]
enum FrameDisposition {
    Continue,
    PeerClosed,
    ServiceFailStopped,
}

/// Failure to initialise the process-local key-service core.
///
/// The encrypted-vault implementation stays private to this crate.  Callers
/// receive an operational error without gaining a handle to the vault or its
/// master-key source.
#[derive(Debug, thiserror::Error)]
enum KeyServiceRuntimeError {
    #[error("key-service configuration: {0}")]
    Configuration(String),
    #[error("key-service storage: {0}")]
    Storage(String),
}

/// Shareable request dispatcher owned by the key-service process.
///
/// Clones share the same serialised vault state.  Its only public capability
/// is dispatching the seed-free wire protocol.
#[derive(Clone)]
struct KeyServiceRuntime {
    vault: Arc<Mutex<Vault>>,
    _vault_lease: Arc<ExclusiveFileLock>,
    /// One-way process-lifecycle signal.  A Vault can enter fail-stop only
    /// after a replacement was made visible but parent-directory durability
    /// could not be proven.  The service must then exit after flushing the
    /// triggering response so an attached (not only owning) supervisor can
    /// observe endpoint disappearance and start a fresh process.
    fail_stop_tx: watch::Sender<Option<String>>,
}

impl std::fmt::Debug for KeyServiceRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KeyServiceRuntime")
            .finish_non_exhaustive()
    }
}

impl KeyServiceRuntime {
    /// Open the configured encrypted vault using the production master-key
    /// resolution policy.
    fn open_default() -> Result<Self, KeyServiceRuntimeError> {
        let (passphrase, _) = PassphraseStore::new(default_passphrase_path())
            .load_or_create()
            .map_err(|error| KeyServiceRuntimeError::Configuration(error.to_string()))?;
        let source = MasterKeySource::Explicit(passphrase);
        Self::open_with_source(&default_vault_path(), &source)
    }

    /// Open a service runtime at an explicit path.
    ///
    /// This is useful for isolated service deployments and integration tests.
    /// The passphrase protects the encrypted container; signing seeds are
    /// generated only inside [`dispatch`](Self::dispatch) and never cross this
    /// API.
    #[cfg(test)]
    fn open(
        vault_path: impl AsRef<Path>,
        passphrase: impl Into<String>,
    ) -> Result<Self, KeyServiceRuntimeError> {
        Self::open_with_source(
            vault_path.as_ref(),
            &MasterKeySource::Explicit(passphrase.into()),
        )
    }

    fn open_with_source(
        vault_path: &Path,
        source: &MasterKeySource,
    ) -> Result<Self, KeyServiceRuntimeError> {
        let lease = ExclusiveFileLock::try_acquire_for_data_path(vault_path)
            .map_err(|error| KeyServiceRuntimeError::Storage(error.to_string()))?
            .ok_or_else(|| {
                KeyServiceRuntimeError::Storage(format!(
                    "encrypted vault {} already has an active process owner",
                    vault_path.display()
                ))
            })?;
        let vault = Vault::open_or_init(vault_path, source)
            .map_err(|error| KeyServiceRuntimeError::Storage(error.to_string()))?;
        let (fail_stop_tx, _) = watch::channel(None);
        Ok(Self {
            vault: Arc::new(Mutex::new(vault)),
            _vault_lease: Arc::new(lease),
            fail_stop_tx,
        })
    }

    /// Number of runtime owner entries, for operational boot logging only.
    async fn owner_count(&self) -> usize {
        self.vault.lock().await.owner_count()
    }

    fn fail_stop_receiver(&self) -> watch::Receiver<Option<String>> {
        self.fail_stop_tx.subscribe()
    }

    /// Publish process shutdown only after the current response has been
    /// flushed.  The Vault itself remains fail-stopped immediately; this
    /// signal is solely the lifecycle handoff that makes recovery possible
    /// even when the observer did not spawn this process.
    async fn request_shutdown_if_fail_stopped(&self) -> bool {
        let reason = {
            let vault = self.vault.lock().await;
            vault.fail_stop_reason().map(str::to_owned)
        };
        let Some(reason) = reason else {
            return false;
        };
        self.fail_stop_tx.send_replace(Some(reason));
        true
    }

    /// Execute one canonical key-service request.
    async fn dispatch(&self, request: KeyringRequest) -> KeyringResponse {
        macro_rules! operational_vault {
            () => {{
                let vault = self.vault.lock().await;
                if let Some(reason) = vault.fail_stop_reason() {
                    return KeyringResponse::err(
                        "fail_stopped",
                        format!(
                            "key-service requires restart after uncertain durability: {reason}"
                        ),
                    );
                }
                vault
            }};
        }

        // Linearise every request, including malformed requests and health,
        // against a fail-stop transition. Individual operation locks repeat
        // the check after request decoding so a concurrent transition cannot
        // slip between admission and private-key use.
        drop(operational_vault!());
        match request {
            KeyringRequest::Health {} => KeyringResponse::Health {
                protocol_version: KEY_SERVICE_PROTOCOL_VERSION,
            },
            KeyringRequest::Ensure { primary_self } => {
                let mut vault = operational_vault!();
                if let Err(error) = vault.ensure(&primary_self) {
                    return vault_error_to_response(error);
                }
                match vault.derive_pubkey(&primary_self) {
                    Ok(public_key) => KeyringResponse::PublicKey {
                        public_key_b64: {
                            use base64::Engine as _;
                            base64::engine::general_purpose::STANDARD.encode(public_key.to_bytes())
                        },
                    },
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::Sign {
                self_ura,
                public_key_b64,
                signer_policy_ref,
                canonical_bytes_b64,
            } => {
                use base64::Engine as _;
                let bytes =
                    match base64::engine::general_purpose::STANDARD.decode(&canonical_bytes_b64) {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            return KeyringResponse::err(
                                "base64",
                                format!("canonical_bytes_b64: {error}"),
                            );
                        }
                    };
                if bytes.is_empty() || bytes.len() > MAX_KEY_SERVICE_CANONICAL_BYTES {
                    return KeyringResponse::err(
                        "policy",
                        format!(
                            "canonical bytes must contain 1..={MAX_KEY_SERVICE_CANONICAL_BYTES} bytes"
                        ),
                    );
                }
                let vault = operational_vault!();
                match vault.sign_bound(&self_ura, &public_key_b64, &signer_policy_ref, &bytes) {
                    Ok(signature) => KeyringResponse::Signature {
                        signature_b64: base64::engine::general_purpose::STANDARD
                            .encode(signature.to_bytes()),
                    },
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::DerivePubkey { self_ura } => {
                let vault = operational_vault!();
                match vault.derive_pubkey(&self_ura) {
                    Ok(public_key) => KeyringResponse::PublicKey {
                        public_key_b64: {
                            use base64::Engine as _;
                            base64::engine::general_purpose::STANDARD.encode(public_key.to_bytes())
                        },
                    },
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::RuntimeList { limit, cursor } => {
                let vault = operational_vault!();
                match vault.list_page(limit, cursor.as_deref()) {
                    Ok((entries, next_cursor)) => KeyringResponse::RuntimeEntries {
                        entries,
                        next_cursor,
                    },
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::Forget { primary_self } => {
                let mut vault = operational_vault!();
                match vault.mutate_and_seal(|vault| {
                    vault.forget(&primary_self);
                    Ok(())
                }) {
                    Ok(()) => KeyringResponse::Ok,
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::InventoryCreate {
                purpose,
                bound_subject,
            } => {
                let mut vault = operational_vault!();
                match vault.mutate_and_seal(|vault| vault.inventory_create(purpose, bound_subject))
                {
                    Ok(entry) => KeyringResponse::InventoryKey { entry },
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::InventoryList {
                purpose,
                status,
                limit,
                cursor,
            } => {
                let vault = operational_vault!();
                match vault.inventory_list_page(
                    purpose.as_deref(),
                    status,
                    limit,
                    cursor.as_deref(),
                ) {
                    Ok((entries, next_cursor)) => KeyringResponse::InventoryKeys {
                        entries,
                        next_cursor,
                    },
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::InventoryPublicKey { key_id } => {
                let vault = operational_vault!();
                match vault.inventory_public_key(&key_id) {
                    Ok(entry) => KeyringResponse::InventoryKey { entry },
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::InventorySign {
                key_id,
                expected_purpose,
                subject_ura,
                signer_policy_ref,
                canonical_bytes_b64,
            } => {
                use base64::Engine as _;
                let bytes =
                    match base64::engine::general_purpose::STANDARD.decode(&canonical_bytes_b64) {
                        Ok(bytes) => bytes,
                        Err(error) => {
                            return KeyringResponse::err(
                                "base64",
                                format!("canonical_bytes_b64: {error}"),
                            );
                        }
                    };
                let vault = operational_vault!();
                match vault.inventory_sign_bound(
                    &key_id,
                    &expected_purpose,
                    &subject_ura,
                    &signer_policy_ref,
                    &bytes,
                ) {
                    Ok(signature) => KeyringResponse::Signature {
                        signature_b64: base64::engine::general_purpose::STANDARD
                            .encode(signature.to_bytes()),
                    },
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::InventoryRotate { key_id } => {
                let mut vault = operational_vault!();
                match vault.mutate_and_seal(|vault| vault.inventory_rotate(&key_id)) {
                    Ok(entry) => KeyringResponse::InventoryKey { entry },
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::InventoryRevoke { key_id } => {
                let mut vault = operational_vault!();
                match vault.mutate_and_seal(|vault| vault.inventory_revoke(&key_id)) {
                    Ok(revoked_unix_ms) => KeyringResponse::InventoryRevoked { revoked_unix_ms },
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::InventorySetExpiry {
                key_id,
                expires_unix_ms,
            } => {
                let mut vault = operational_vault!();
                match vault
                    .mutate_and_seal(|vault| vault.inventory_set_expiry(&key_id, expires_unix_ms))
                {
                    Ok(()) => KeyringResponse::Ok,
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::InventoryBindSubject {
                key_id,
                subject_ura,
            } => {
                let mut vault = operational_vault!();
                match vault
                    .mutate_and_seal(|vault| vault.inventory_bind_subject(&key_id, subject_ura))
                {
                    Ok(()) => KeyringResponse::Ok,
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::InventoryPeerAdd {
                peer_ura,
                public_key_b64,
                via_hub,
            } => {
                let mut vault = operational_vault!();
                match vault.mutate_and_seal(|vault| {
                    vault.inventory_peer_add(peer_ura, public_key_b64, via_hub)
                }) {
                    Ok(added) => KeyringResponse::InventoryPeerAdded { added },
                    Err(error) => vault_error_to_response(error),
                }
            }
            KeyringRequest::InventoryPeerList { limit, cursor } => {
                let vault = operational_vault!();
                match vault.inventory_peer_list_page(limit, cursor.as_deref()) {
                    Ok((peers, next_cursor)) => {
                        KeyringResponse::InventoryPeers { peers, next_cursor }
                    }
                    Err(error) => vault_error_to_response(error),
                }
            }
        }
    }
}

/// Run the canonical key-service process at the configured local endpoint.
///
/// This is the only public server entry point. The decrypted runtime and its
/// vault lease stay private to this module for the process lifetime.
pub async fn run_default_key_service() -> Result<(), Box<dyn std::error::Error>> {
    let vault_path = default_vault_path();
    let socket_path = default_socket_path();
    let runtime = KeyServiceRuntime::open_default().map_err(|error| {
        format!(
            "[easynet-keyring] open/init vault at {}: {error}",
            vault_path.display()
        )
    })?;
    eprintln!(
        "[easynet-keyring] vault opened at {} ({} entries)",
        vault_path.display(),
        runtime.owner_count().await
    );

    #[cfg(unix)]
    let listener = {
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if socket_path.exists() {
            return Err(format!(
                "[easynet-keyring] socket already exists at {} — remove it before starting",
                socket_path.display()
            )
            .into());
        }
        let listener = UnixListener::bind(&socket_path)?;
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
        eprintln!(
            "[easynet-keyring] listening on {} (mode 0600)",
            socket_path.display()
        );
        listener
    };

    #[cfg(windows)]
    let mut listener = {
        let pipe_name = socket_path.to_string_lossy().to_string();
        let listener = PipeListener::bind(pipe_name.clone())?;
        eprintln!("[easynet-keyring] listening on {pipe_name}");
        listener
    };

    #[cfg(unix)]
    install_signal_cleanup(socket_path.clone());
    let connection_limit = Arc::new(Semaphore::new(MAX_CONCURRENT_KEY_SERVICE_CONNECTIONS));
    let mut fail_stop_rx = runtime.fail_stop_receiver();

    #[cfg(unix)]
    loop {
        tokio::select! {
            changed = fail_stop_rx.changed() => {
                let reason = match changed {
                    Ok(()) => fail_stop_rx
                        .borrow()
                        .clone()
                        .unwrap_or_else(|| "missing fail-stop reason".to_string()),
                    Err(_) => "key-service fail-stop channel closed".to_string(),
                };
                let _ = std::fs::remove_file(&socket_path);
                return Err(format!(
                    "[easynet-keyring] fail-stopped after uncertain vault durability: {reason}"
                ).into());
            }
            accepted = listener.accept() => {
                let (stream, _address) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        eprintln!("[easynet-keyring] accept: {error}");
                        continue;
                    }
                };
                let permit = Arc::clone(&connection_limit)
                    .acquire_owned()
                    .await
                    .map_err(|error| format!("key-service connection limiter closed: {error}"))?;
                let runtime = runtime.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_connection(stream, runtime).await {
                        eprintln!("[easynet-keyring] connection: {error}");
                    }
                });
            }
        }
    }

    #[cfg(windows)]
    loop {
        tokio::select! {
            changed = fail_stop_rx.changed() => {
                let reason = match changed {
                    Ok(()) => fail_stop_rx
                        .borrow()
                        .clone()
                        .unwrap_or_else(|| "missing fail-stop reason".to_string()),
                    Err(_) => "key-service fail-stop channel closed".to_string(),
                };
                return Err(format!(
                    "[easynet-keyring] fail-stopped after uncertain vault durability: {reason}"
                ).into());
            }
            accepted = listener.accept() => {
                let stream = match accepted {
                    Ok(stream) => stream,
                    Err(error) => {
                        eprintln!("[easynet-keyring] accept: {error}");
                        continue;
                    }
                };
                let permit = Arc::clone(&connection_limit)
                    .acquire_owned()
                    .await
                    .map_err(|error| format!("key-service connection limiter closed: {error}"))?;
                let runtime = runtime.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(error) = handle_connection(stream, runtime).await {
                        eprintln!("[easynet-keyring] connection: {error}");
                    }
                });
            }
        }
    }
}

/// Run a bounded real key-service instance for cross-module tests.
///
/// Unlike in-memory provider fixtures, this helper exercises the production
/// vault dispatcher and framed Unix transport. It is test-only so production
/// lifecycle code cannot acquire a second service entry point.
#[cfg(all(test, unix))]
pub(crate) fn run_test_unix_key_service(
    socket_path: std::path::PathBuf,
    vault_path: std::path::PathBuf,
    passphrase: String,
    caller: String,
    expected_connections: usize,
    ready: std::sync::mpsc::Sender<Result<ManagedSigningKeyProjection, String>>,
) {
    let result = (|| -> Result<_, String> {
        let runtime = KeyServiceRuntime::open(vault_path, passphrase)
            .map_err(|error| format!("open test key service: {error}"))?;
        let runtime_driver = tokio::runtime::Runtime::new()
            .map_err(|error| format!("create test key-service runtime: {error}"))?;
        let entry = runtime_driver.block_on(runtime.dispatch(KeyringRequest::InventoryCreate {
            purpose: "agent_signing".to_string(),
            bound_subject: Some(caller),
        }));
        let entry = match entry {
            KeyringResponse::InventoryKey { entry } => entry,
            other => return Err(format!("create test managed key: {other:?}")),
        };
        let listener = std::os::unix::net::UnixListener::bind(&socket_path)
            .map_err(|error| format!("bind test key-service socket: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("set test key-service socket nonblocking: {error}"))?;
        ready
            .send(Ok(entry))
            .map_err(|_| "test key-service caller dropped".to_string())?;
        runtime_driver.block_on(async move {
            let listener = tokio::net::UnixListener::from_std(listener)
                .map_err(|error| format!("adopt test key-service listener: {error}"))?;
            for _ in 0..expected_connections {
                let (stream, _) = listener
                    .accept()
                    .await
                    .map_err(|error| format!("accept test key-service connection: {error}"))?;
                handle_connection(stream, runtime.clone())
                    .await
                    .map_err(|error| format!("serve test key-service connection: {error}"))?;
            }
            Ok::<(), String>(())
        })?;
        let _ = std::fs::remove_file(socket_path);
        Ok(())
    })();
    if let Err(error) = result {
        let _ = ready.send(Err(error));
    }
}

#[cfg(unix)]
fn install_signal_cleanup(socket_path: std::path::PathBuf) {
    tokio::spawn(async move {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(error) => {
                    eprintln!("[easynet-keyring] cannot watch SIGTERM: {error}");
                    return;
                }
            };
        let mut sigint =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()) {
                Ok(signal) => signal,
                Err(error) => {
                    eprintln!("[easynet-keyring] cannot watch SIGINT: {error}");
                    return;
                }
            };
        tokio::select! {
            _ = sigterm.recv() => eprintln!("[easynet-keyring] SIGTERM, shutting down"),
            _ = sigint.recv() => eprintln!("[easynet-keyring] SIGINT, shutting down"),
        }
        let _ = std::fs::remove_file(socket_path);
        std::process::exit(0);
    });
}

async fn handle_connection<S>(
    stream: S,
    runtime: KeyServiceRuntime,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    handle_connection_with_policy(stream, runtime, ConnectionPolicy::default()).await
}

async fn handle_connection_with_policy<S>(
    mut stream: S,
    runtime: KeyServiceRuntime,
    policy: ConnectionPolicy,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    debug_assert!(policy.max_requests > 0);

    for _request_index in 0..policy.max_requests {
        // One absolute deadline owns the complete frame exchange.  In
        // particular, reading the length or body never refreshes the budget,
        // and dispatch plus response write/flush consume the same budget.
        let frame_deadline = Instant::now() + policy.frame_deadline;
        let disposition =
            match timeout_at(frame_deadline, process_frame(&mut stream, &runtime)).await {
                Ok(result) => result?,
                Err(_) => {
                    terminate_connection(&mut stream).await;
                    return Err(Box::new(FrameDeadlineElapsed {
                        budget: policy.frame_deadline,
                    }));
                }
            };

        match disposition {
            FrameDisposition::Continue => {}
            FrameDisposition::PeerClosed => return Ok(()),
            FrameDisposition::ServiceFailStopped => {
                terminate_connection(&mut stream).await;
                return Ok(());
            }
        }
    }

    // Bound resource retention even for a healthy but permanently connected
    // client.  Clients may reconnect and continue with a fresh budget.
    terminate_connection(&mut stream).await;
    Ok(())
}

async fn process_frame<S>(
    stream: &mut S,
    runtime: &KeyServiceRuntime,
) -> Result<FrameDisposition, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut length_bytes = [0u8; 4];
    match stream.read_exact(&mut length_bytes).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            return Ok(FrameDisposition::PeerClosed);
        }
        Err(error) => return Err(Box::new(error)),
    }
    let frame_length = u32::from_be_bytes(length_bytes) as usize;
    if frame_length > MAX_KEY_SERVICE_FRAME_BYTES {
        let response = KeyringResponse::err(
            "frame_too_large",
            format!("request frame {frame_length} > max {MAX_KEY_SERVICE_FRAME_BYTES}"),
        );
        write_response(stream, &response).await?;
        return Err(format!("oversized request: {frame_length}").into());
    }

    let mut body = vec![0u8; frame_length];
    stream.read_exact(&mut body).await?;
    let response = match serde_json::from_slice::<KeyringRequest>(&body) {
        Ok(request) => runtime.dispatch(request).await,
        Err(error) => KeyringResponse::err("parse", format!("bad request: {error}")),
    };
    write_response(stream, &response).await?;
    if runtime.request_shutdown_if_fail_stopped().await {
        return Ok(FrameDisposition::ServiceFailStopped);
    }
    Ok(FrameDisposition::Continue)
}

async fn terminate_connection<S>(stream: &mut S)
where
    S: AsyncWrite + Unpin,
{
    // Shutdown is best-effort and independently bounded.  Dropping the stream
    // immediately afterwards is the terminal close even if a hostile peer
    // prevents the graceful write-side shutdown from completing.
    let _ = timeout(KEY_SERVICE_TERMINAL_CLOSE_GRACE, stream.shutdown()).await;
}

async fn write_response<S>(
    stream: &mut S,
    response: &KeyringResponse,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(response)?;
    if body.len() > MAX_KEY_SERVICE_FRAME_BYTES {
        return Err(format!(
            "key-service response frame {} > max {MAX_KEY_SERVICE_FRAME_BYTES}",
            body.len()
        )
        .into());
    }
    stream.write_all(&(body.len() as u32).to_be_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::keyring::VaultError;

    fn test_runtime(home: &tempfile::TempDir) -> KeyServiceRuntime {
        KeyServiceRuntime::open(home.path().join("key-service.enc"), "test-passphrase")
            .expect("test key-service runtime")
    }

    fn framed(request: &KeyringRequest) -> Vec<u8> {
        let body = serde_json::to_vec(request).expect("serialize key-service request");
        let mut frame = Vec::with_capacity(4 + body.len());
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&body);
        frame
    }

    #[test]
    fn encrypted_vault_has_one_process_owner() {
        let home = tempfile::tempdir().expect("key-service test home");
        let path = home.path().join("key-service.enc");
        let first =
            KeyServiceRuntime::open(&path, "test-passphrase").expect("first key-service owner");

        let error = KeyServiceRuntime::open(&path, "test-passphrase")
            .expect_err("second owner must fail closed");
        assert!(error.to_string().contains("active process owner"));

        drop(first);
        KeyServiceRuntime::open(&path, "test-passphrase")
            .expect("lease is released with the service runtime");
    }

    #[tokio::test(start_paused = true)]
    async fn frame_deadline_is_not_refreshed_after_reading_length() {
        let home = tempfile::tempdir().expect("key-service test home");
        let runtime = test_runtime(&home);
        let (mut client, server) = tokio::io::duplex(64);
        let policy = ConnectionPolicy {
            frame_deadline: Duration::from_secs(5),
            max_requests: 1,
        };
        let handler = tokio::spawn(handle_connection_with_policy(server, runtime, policy));

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(3)).await;
        client
            .write_all(&8_u32.to_be_bytes())
            .await
            .expect("write only the request length");
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(3)).await;

        let error = handler
            .await
            .expect("connection task joins")
            .expect_err("incomplete frame must exhaust the original deadline");
        assert!(error.downcast_ref::<FrameDeadlineElapsed>().is_some());

        let mut byte = [0_u8; 1];
        assert_eq!(
            client
                .read(&mut byte)
                .await
                .expect("observe terminal close"),
            0
        );
    }

    #[tokio::test(start_paused = true)]
    async fn response_write_uses_the_inbound_frame_deadline() {
        let home = tempfile::tempdir().expect("key-service test home");
        let runtime = test_runtime(&home);
        // The response is larger than this buffer, so an unread response
        // blocks in write_all and exercises the outbound half of the budget.
        let (mut client, server) = tokio::io::duplex(16);
        let policy = ConnectionPolicy {
            frame_deadline: Duration::from_secs(5),
            max_requests: 1,
        };
        let handler = tokio::spawn(handle_connection_with_policy(server, runtime, policy));

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(4)).await;
        client
            .write_all(&framed(&KeyringRequest::Health {}))
            .await
            .expect("write complete request");
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(2)).await;

        let error = handler
            .await
            .expect("connection task joins")
            .expect_err("blocked response must exhaust the inbound deadline");
        assert!(error.downcast_ref::<FrameDeadlineElapsed>().is_some());
    }

    #[tokio::test]
    async fn request_budget_closes_connection_after_final_response() {
        let home = tempfile::tempdir().expect("key-service test home");
        let runtime = test_runtime(&home);
        let (mut client, server) = tokio::io::duplex(1024);
        let policy = ConnectionPolicy {
            frame_deadline: Duration::from_secs(5),
            max_requests: 1,
        };
        let handler = tokio::spawn(handle_connection_with_policy(server, runtime, policy));

        client
            .write_all(&framed(&KeyringRequest::Health {}))
            .await
            .expect("write request");
        let response_length = client.read_u32().await.expect("read response length") as usize;
        let mut response = vec![0_u8; response_length];
        client
            .read_exact(&mut response)
            .await
            .expect("read final response");
        serde_json::from_slice::<KeyringResponse>(&response).expect("decode final response");

        let mut byte = [0_u8; 1];
        assert_eq!(
            client.read(&mut byte).await.expect("observe budget close"),
            0
        );
        handler
            .await
            .expect("connection task joins")
            .expect("request budget closes cleanly");
    }

    #[tokio::test]
    async fn uncertain_durability_makes_health_and_followup_requests_fail_stopped() {
        let home = tempfile::tempdir().expect("key-service test home");
        let runtime = test_runtime(&home);
        {
            let mut vault = runtime.vault.lock().await;
            let error = vault
                .mutate_and_seal_with_directory_sync(
                    |candidate| {
                        candidate.inventory_create(
                            "invocation".into(),
                            Some("easynet:///r/test/agent/signer.main".into()),
                        )
                    },
                    |_| anyhow::bail!("injected post-rename directory fsync failure"),
                )
                .unwrap_err();
            assert!(matches!(error, VaultError::Persistence(_)));
        }

        for request in [
            KeyringRequest::Health {},
            KeyringRequest::InventoryList {
                purpose: None,
                status: None,
                limit: None,
                cursor: None,
            },
        ] {
            assert!(matches!(
                runtime.dispatch(request).await,
                KeyringResponse::Error { kind, .. } if kind == "fail_stopped"
            ));
        }
    }

    #[tokio::test]
    async fn fail_stopped_runtime_flushes_the_rejection_before_requesting_process_exit() {
        let home = tempfile::tempdir().expect("key-service test home");
        let runtime = test_runtime(&home);
        {
            let mut vault = runtime.vault.lock().await;
            vault
                .mutate_and_seal_with_directory_sync(
                    |candidate| {
                        candidate.inventory_create(
                            "invocation".into(),
                            Some("easynet:///r/test/agent/signer.main".into()),
                        )
                    },
                    |_| anyhow::bail!("injected post-rename directory fsync failure"),
                )
                .expect_err("injected uncertainty must fail-stop the Vault");
        }

        let mut fail_stop = runtime.fail_stop_receiver();
        let (mut client, server) = tokio::io::duplex(1024);
        let handler = tokio::spawn(handle_connection_with_policy(
            server,
            runtime,
            ConnectionPolicy {
                frame_deadline: Duration::from_secs(5),
                max_requests: 1,
            },
        ));

        client
            .write_all(&framed(&KeyringRequest::Health {}))
            .await
            .expect("write health request");
        let response_length = client.read_u32().await.expect("read response length") as usize;
        let mut response = vec![0_u8; response_length];
        client
            .read_exact(&mut response)
            .await
            .expect("fail-stop response must flush before shutdown");
        assert!(matches!(
            serde_json::from_slice::<KeyringResponse>(&response).expect("decode response"),
            KeyringResponse::Error { kind, .. } if kind == "fail_stopped"
        ));

        fail_stop
            .changed()
            .await
            .expect("response flush must publish process shutdown");
        assert!(fail_stop.borrow().is_some());
        let mut byte = [0_u8; 1];
        assert_eq!(
            client
                .read(&mut byte)
                .await
                .expect("observe terminal close"),
            0,
        );
        handler
            .await
            .expect("connection task joins")
            .expect("fail-stop closes one connection cleanly");
    }

    #[tokio::test]
    async fn peer_refresh_cannot_replace_existing_trust_anchor() {
        use base64::Engine as _;

        let home = tempfile::tempdir().expect("key-service test home");
        let runtime = test_runtime(&home);
        let peer_ura = "easynet:///r/peer/agent/verifier.main";
        let first_key = base64::engine::general_purpose::STANDARD.encode([1u8; 32]);
        let replacement_key = base64::engine::general_purpose::STANDARD.encode([2u8; 32]);

        let add = |public_key_b64: String| KeyringRequest::InventoryPeerAdd {
            peer_ura: peer_ura.into(),
            public_key_b64,
            via_hub: None,
        };
        assert!(matches!(
            runtime.dispatch(add(first_key.clone())).await,
            KeyringResponse::InventoryPeerAdded { added: true }
        ));
        assert!(matches!(
            runtime.dispatch(add(first_key.clone())).await,
            KeyringResponse::InventoryPeerAdded { added: false }
        ));
        assert!(matches!(
            runtime.dispatch(add(replacement_key)).await,
            KeyringResponse::Error { kind, .. } if kind == "policy"
        ));

        match runtime
            .dispatch(KeyringRequest::InventoryPeerList {
                limit: None,
                cursor: None,
            })
            .await
        {
            KeyringResponse::InventoryPeers { peers, .. } => {
                assert_eq!(peers.len(), 1);
                assert_eq!(peers[0].public_key_b64, first_key);
            }
            other => panic!("unexpected peer inventory response: {other:?}"),
        }
    }
}
