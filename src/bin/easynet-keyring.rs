// EasyNet Keyring — process entry point
// =====================================
//
// File: src/bin/easynet-keyring.rs
//
// Long-running daemon that owns the Ed25519 device-identity vault
// for one user-on-one-host. RFC-001 plan v4.1.5 Phase 3A. Speaks
// length-prefixed JSON over a UDS socket.
//
// Lifecycle
// ---------
// 1. Resolve master-key source. Prefer `EASYNET_KEYRING_PASSPHRASE`
//    env. Fall back to interactive prompt on a TTY (one read,
//    cached for the daemon's lifetime). Headless deploys without
//    the env are a hard failure — no plaintext fallback.
// 2. Open or init the vault file at
//    `~/.easynet/keyring.enc` (override via
//    `EASYNET_KEYRING_VAULT_PATH`).
// 3. Bind UDS at `~/.easynet/keyring.sock` (override via
//    `EASYNET_KEYRING_SOCKET_PATH`). 0600 file mode. Refuse to
//    overwrite an existing socket the daemon doesn't own — let
//    the operator clean it up.
// 4. Accept loop: per-connection task, read length-prefixed JSON
//    requests, dispatch to the in-memory `Vault`, write
//    length-prefixed JSON responses.
//
// Wire framing
// ------------
// Each frame: `u32 BE length || JSON bytes`. Length = JSON byte
// count. Max 64 KiB per frame — anything larger is a client bug
// (sign requests carry canonical bytes that never approach this
// in practice, and `forget` / `list` are tiny).
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use easynet_cli::daemon::keyring::{
    default_socket_path, home_relative, vault_error_to_response, KeyringRequest, KeyringResponse,
    MasterKeySource, Vault, DEFAULT_VAULT_REL,
};
#[cfg(windows)]
use easynet_cli::support::platform::named_pipe::PipeListener;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::Mutex;

const MAX_FRAME_BYTES: usize = 64 * 1024;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let vault_path = std::env::var_os("EASYNET_KEYRING_VAULT_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_relative(DEFAULT_VAULT_REL));
    let socket_path = std::env::var_os("EASYNET_KEYRING_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(default_socket_path);

    let source = resolve_master_key_source()?;
    let vault = Vault::open_or_init(&vault_path, &source).map_err(|e| {
        format!(
            "[easynet-keyring] open/init vault at {}: {e}",
            vault_path.display()
        )
    })?;
    eprintln!(
        "[easynet-keyring] vault opened at {} ({} entries)",
        vault_path.display(),
        vault.list().len()
    );

    #[cfg(unix)]
    let listener = {
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if socket_path.exists() {
            // Refuse to overwrite. If a previous daemon crashed leaving
            // the socket file, the operator removes it explicitly so a
            // running daemon doesn't get its socket yanked silently.
            return Err(format!(
                "[easynet-keyring] socket already exists at {} — remove it before starting (likely a stale file from a previous crash)",
                socket_path.display()
            )
            .into());
        }
        let listener = UnixListener::bind(&socket_path)?;
        use std::os::unix::fs::PermissionsExt;
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

    let shared = Arc::new(Mutex::new(vault));
    #[cfg(unix)]
    {
        let socket_for_cleanup = socket_path.clone();
        // Best-effort cleanup on Ctrl-C / SIGTERM. The daemon process
        // exiting without unlinking the socket leaves the host in a
        // state where the next daemon restart fails the
        // `socket_path.exists()` guard above. Catching SIGTERM /
        // SIGINT and unlinking before exit makes the operator's
        // restart cycle smooth.
        tokio::spawn(async move {
            let mut sigterm =
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("[easynet-keyring] cannot watch SIGTERM: {e}");
                        return;
                    }
                };
            let mut sigint =
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("[easynet-keyring] cannot watch SIGINT: {e}");
                        return;
                    }
                };
            tokio::select! {
                _ = sigterm.recv() => eprintln!("[easynet-keyring] SIGTERM, shutting down"),
                _ = sigint.recv() => eprintln!("[easynet-keyring] SIGINT, shutting down"),
            }
            let _ = std::fs::remove_file(&socket_for_cleanup);
            std::process::exit(0);
        });
    }

    #[cfg(unix)]
    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[easynet-keyring] accept: {e}");
                continue;
            }
        };
        let vault_for_conn = Arc::clone(&shared);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, vault_for_conn).await {
                eprintln!("[easynet-keyring] connection: {e}");
            }
        });
    }

    #[cfg(windows)]
    loop {
        let stream = match listener.accept().await {
            Ok(stream) => stream,
            Err(e) => {
                eprintln!("[easynet-keyring] accept: {e}");
                continue;
            }
        };
        let vault_for_conn = Arc::clone(&shared);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, vault_for_conn).await {
                eprintln!("[easynet-keyring] connection: {e}");
            }
        });
    }
}

async fn handle_connection<S>(
    mut stream: S,
    vault: Arc<Mutex<Vault>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    loop {
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(Box::new(e)),
        }
        let frame_len = u32::from_be_bytes(len_buf) as usize;
        if frame_len > MAX_FRAME_BYTES {
            let resp = KeyringResponse::err(
                "frame_too_large",
                format!("request frame {frame_len} > max {MAX_FRAME_BYTES}"),
            );
            write_response(&mut stream, &resp).await?;
            return Err(format!("oversized request: {frame_len}").into());
        }

        let mut buf = vec![0u8; frame_len];
        stream.read_exact(&mut buf).await?;

        let resp = match serde_json::from_slice::<KeyringRequest>(&buf) {
            Ok(req) => dispatch(req, &vault).await,
            Err(e) => KeyringResponse::err("parse", format!("bad request: {e}")),
        };
        write_response(&mut stream, &resp).await?;
    }
}

async fn dispatch(req: KeyringRequest, vault: &Arc<Mutex<Vault>>) -> KeyringResponse {
    match req {
        KeyringRequest::Ensure {
            primary_self,
            role_overlays,
        } => {
            let mut guard = vault.lock().await;
            if !guard.contains(&primary_self) {
                use rand::RngCore;
                let mut seed = [0u8; 32];
                rand::rngs::OsRng.fill_bytes(&mut seed);
                if let Err(err) = guard.put(&primary_self, role_overlays, hex::encode(seed)) {
                    return vault_error_to_response(err);
                }
                if let Err(err) = guard.seal() {
                    return vault_error_to_response(err);
                }
            }
            match guard.derive_pubkey(&primary_self) {
                Ok(pk) => KeyringResponse::PublicKey {
                    public_key_b64: {
                        use base64::Engine;
                        base64::engine::general_purpose::STANDARD.encode(pk.to_bytes())
                    },
                },
                Err(err) => vault_error_to_response(err),
            }
        }
        KeyringRequest::Put {
            primary_self,
            role_overlays,
            seed_hex,
        } => {
            let mut guard = vault.lock().await;
            match guard.put(&primary_self, role_overlays, seed_hex) {
                Ok(()) => match guard.seal() {
                    Ok(()) => KeyringResponse::Ok,
                    Err(e) => vault_error_to_response(e),
                },
                Err(e) => vault_error_to_response(e),
            }
        }
        KeyringRequest::Sign {
            self_ura,
            canonical_bytes_b64,
        } => {
            use base64::Engine;
            let bytes = match base64::engine::general_purpose::STANDARD.decode(&canonical_bytes_b64)
            {
                Ok(b) => b,
                Err(e) => {
                    return KeyringResponse::err("base64", format!("canonical_bytes_b64: {e}"))
                }
            };
            let guard = vault.lock().await;
            match guard.sign(&self_ura, &bytes) {
                Ok(sig) => KeyringResponse::Signature {
                    signature_b64: base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()),
                },
                Err(e) => vault_error_to_response(e),
            }
        }
        KeyringRequest::DerivePubkey { self_ura } => {
            let guard = vault.lock().await;
            match guard.derive_pubkey(&self_ura) {
                Ok(pk) => KeyringResponse::PublicKey {
                    public_key_b64: {
                        use base64::Engine;
                        base64::engine::general_purpose::STANDARD.encode(pk.to_bytes())
                    },
                },
                Err(e) => vault_error_to_response(e),
            }
        }
        KeyringRequest::List => {
            let guard = vault.lock().await;
            KeyringResponse::List {
                entries: guard.list(),
            }
        }
        KeyringRequest::Forget { primary_self } => {
            let mut guard = vault.lock().await;
            guard.forget(&primary_self);
            match guard.seal() {
                Ok(()) => KeyringResponse::Ok,
                Err(e) => vault_error_to_response(e),
            }
        }
        KeyringRequest::InventoryCreate {
            purpose,
            bound_subject,
        } => {
            let mut guard = vault.lock().await;
            match guard.inventory_create(purpose, bound_subject) {
                Ok(entry) => match guard.seal() {
                    Ok(()) => KeyringResponse::InventoryKey { entry },
                    Err(err) => vault_error_to_response(err),
                },
                Err(err) => vault_error_to_response(err),
            }
        }
        KeyringRequest::InventoryList { purpose, status } => {
            let guard = vault.lock().await;
            KeyringResponse::InventoryKeys {
                entries: guard.inventory_list(purpose.as_deref(), status),
            }
        }
        KeyringRequest::InventoryPublicKey { key_id } => {
            let guard = vault.lock().await;
            match guard.inventory_public_key(&key_id) {
                Ok(entry) => KeyringResponse::InventoryKey { entry },
                Err(err) => vault_error_to_response(err),
            }
        }
        KeyringRequest::InventorySign {
            key_id,
            canonical_bytes_b64,
        } => {
            use base64::Engine;
            let bytes = match base64::engine::general_purpose::STANDARD.decode(&canonical_bytes_b64)
            {
                Ok(bytes) => bytes,
                Err(err) => {
                    return KeyringResponse::err("base64", format!("canonical_bytes_b64: {err}"));
                }
            };
            let guard = vault.lock().await;
            match guard.inventory_sign(&key_id, &bytes) {
                Ok(signature) => KeyringResponse::Signature {
                    signature_b64: base64::engine::general_purpose::STANDARD
                        .encode(signature.to_bytes()),
                },
                Err(err) => vault_error_to_response(err),
            }
        }
        KeyringRequest::InventoryRotate { key_id } => {
            let mut guard = vault.lock().await;
            match guard.inventory_rotate(&key_id) {
                Ok(entry) => match guard.seal() {
                    Ok(()) => KeyringResponse::InventoryKey { entry },
                    Err(err) => vault_error_to_response(err),
                },
                Err(err) => vault_error_to_response(err),
            }
        }
        KeyringRequest::InventoryRevoke { key_id } => {
            let mut guard = vault.lock().await;
            match guard.inventory_revoke(&key_id) {
                Ok(revoked_unix_ms) => match guard.seal() {
                    Ok(()) => KeyringResponse::InventoryRevoked { revoked_unix_ms },
                    Err(err) => vault_error_to_response(err),
                },
                Err(err) => vault_error_to_response(err),
            }
        }
        KeyringRequest::InventorySetExpiry {
            key_id,
            expires_unix_ms,
        } => {
            let mut guard = vault.lock().await;
            match guard.inventory_set_expiry(&key_id, expires_unix_ms) {
                Ok(()) => match guard.seal() {
                    Ok(()) => KeyringResponse::Ok,
                    Err(err) => vault_error_to_response(err),
                },
                Err(err) => vault_error_to_response(err),
            }
        }
        KeyringRequest::InventoryBindSubject {
            key_id,
            subject_ura,
        } => {
            let mut guard = vault.lock().await;
            match guard.inventory_bind_subject(&key_id, subject_ura) {
                Ok(()) => match guard.seal() {
                    Ok(()) => KeyringResponse::Ok,
                    Err(err) => vault_error_to_response(err),
                },
                Err(err) => vault_error_to_response(err),
            }
        }
        KeyringRequest::InventoryPeerAdd {
            peer_ura,
            public_key_b64,
            via_hub,
        } => {
            let mut guard = vault.lock().await;
            match guard.inventory_peer_add(peer_ura, public_key_b64, via_hub) {
                Ok(added) => match guard.seal() {
                    Ok(()) => KeyringResponse::InventoryPeerAdded { added },
                    Err(err) => vault_error_to_response(err),
                },
                Err(err) => vault_error_to_response(err),
            }
        }
        KeyringRequest::InventoryPeerList => {
            let guard = vault.lock().await;
            KeyringResponse::InventoryPeers {
                peers: guard.inventory_peer_list(),
            }
        }
    }
}

async fn write_response<S>(
    stream: &mut S,
    resp: &KeyringResponse,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(resp)?;
    let len = (body.len() as u32).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

fn resolve_master_key_source() -> Result<MasterKeySource, Box<dyn std::error::Error>> {
    if let Ok(s) = std::env::var("EASYNET_KEYRING_PASSPHRASE") {
        if !s.is_empty() {
            return Ok(MasterKeySource::Env);
        }
    }
    // Interactive fallback only if stdin is a TTY. Headless deploys
    // (Docker / systemd / CI) without env are explicit failures —
    // we will not silently boot with no encryption.
    if atty_stdin() {
        eprint!("Enter EasyNet keyring passphrase: ");
        std::io::stderr().flush().ok();
        let mut pass = String::new();
        std::io::stdin().read_line(&mut pass)?;
        let pass = pass
            .trim_end_matches('\n')
            .trim_end_matches('\r')
            .to_string();
        if pass.is_empty() {
            return Err("[easynet-keyring] empty passphrase rejected".into());
        }
        return Ok(MasterKeySource::Explicit(pass));
    }
    Err(
        "[easynet-keyring] no master-key source: set EASYNET_KEYRING_PASSPHRASE or run on a TTY"
            .into(),
    )
}

fn atty_stdin() -> bool {
    // libc isatty(0) — minimal cross-platform probe without
    // pulling another crate.
    #[cfg(unix)]
    unsafe {
        libc::isatty(libc::STDIN_FILENO) == 1
    }
    #[cfg(not(unix))]
    {
        false
    }
}
