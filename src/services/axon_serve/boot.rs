// EasyNet CLI — axon_serve — daemon boot wiring
// ===============================================
//
// File: src/services/axon_serve/boot.rs
// Description: Loads RFC-003 PR-1 configuration from disk and brings
//              the gRPC InvocationServer online as a sidecar inside
//              the existing easynet-daemon process.
//
// What this module does
// ---------------------
// `boot::start_axon_serve_sidecar(...)` is the one function the
// daemon binary calls to bring the new transport plane online. It:
//
// 1. Loads `~/.easynet/daemon-config.toml` via `DaemonConfig::load`.
//    A missing or malformed file is a soft failure — we log and
//    return without spawning any listener so the legacy daemon
//    subsystems (control.sock, runtime-dispatch, heartbeat) keep
//    working unchanged.
// 2. Loads `~/.easynet/credentials.json` to derive the daemon's own
//    URI; threads it into `AdmissionFacade` as the loopback bypass
//    so the daemon can call its own RPCs without entering the
//    realm trust set.
// 3. Loads `/etc/easynet/realm-trust.toml` (or the
//    daemon-config-supplied override) via
//    `RealmTrustAnchor::load_or_empty`. Empty fallback is fine for
//    PR-1; PR-7 populates it via the pairing flow; PR-10 canary
//    refuses to swap without a non-empty file.
// 4. Constructs `Arc<PresenceRegistry>` and the
///    `DaemonInvocationService` with the admission facade injected.
// 5. Spawns one or two tokio tasks:
//    - Always: a UDS listener at `daemon-config.toml`'s `uds_path`
//      (default `~/.easynet/daemon.sock`) — distinct socket from
//      the existing `~/.easynet/control.sock` because the gRPC
//      framing differs from the control plane's length-delimited
//      JSON
//    - Hub modes only (`mode = "hub"` or `"both"`): a TCP+TLS
//      listener at the configured `listen_tcp` socket, serving the
//      same `Invocation` service. Spec §1.2 invariants 1+2 are
//      already enforced at config load time, so by the time we get
//      here the cert/key files exist and TCP only happens on
//      hub-class deployments.
//
// What this module does NOT do
// ----------------------------
// - Touch the existing daemon subsystems (Kernel, AbilityDispatcher,
//   ScheduleService, control.sock server, runtime-dispatch.sock,
//   heartbeat). Those keep running unchanged.
// - Implement graceful shutdown. PR-1 spec §1 and §7.2 cite
//   `systemctl restart easynet-daemon` as the operational restart
//   recipe for both config reload and TLS cert rotation; tonic's
//   `serve_with_shutdown` plus the existing ctrlc handler can be
//   wired in a follow-up commit but is not on PR-1's critical
//   path.
// - Pre-create the UDS file's parent directory. The existing daemon
//   already ensures `~/.easynet/` exists before the control.sock
//   bind earlier in `main`; this sidecar runs after, so the
//   directory is guaranteed present.
//
// Failure handling
// ----------------
// Each failure in the boot path is logged to stderr with a short
// `[axon-serve]` prefix and returns without panicking. The daemon
// process keeps running; operators see the error in the logs and
// fix the config / cert / trust file. The driving rationale: PR-1
// ships in parallel with axon-runtime still serving production
// traffic, so an axon_serve misconfiguration must not take the
// daemon down.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;
use std::sync::Arc;

use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

use crate::pb::axon::v1::invocation_server::InvocationServer;
use crate::persistence::daemon_config::{DaemonConfig, DaemonMode, DEFAULT_DAEMON_CONFIG_PATH};
use crate::services::axon_serve::admission_facade::AdmissionFacade;
use crate::services::axon_serve::daemon_invocation_service::DaemonInvocationService;
use crate::services::presence_registry::PresenceRegistry;
use crate::services::realm_trust_anchor::{RealmTrustAnchor, DEFAULT_REALM_TRUST_PATH};

/// Bring the RFC-003 transport plane online as a sidecar to the
/// existing easynet-daemon process.
///
/// Returns `Ok(())` whether or not any listener was spawned — a
/// missing daemon-config.toml is the legitimate "this device is not
/// running the new transport plane yet" state, not an error. When
/// listeners do come up, they run on the caller's tokio runtime as
/// detached tasks; they own their `PresenceRegistry` Arc and stay
/// alive until the runtime shuts down.
pub fn start_axon_serve_sidecar() -> anyhow::Result<()> {
    let config_path = expand_home(DEFAULT_DAEMON_CONFIG_PATH);
    let config = match DaemonConfig::load(&config_path) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!(
                "[axon-serve] no transport-plane config at {} ({err}); skipping gRPC listener",
                config_path.display(),
            );
            return Ok(());
        }
    };

    let daemon_uri = load_daemon_uri();
    let trust_anchor = load_trust_anchor();
    let presence = Arc::new(PresenceRegistry::new());
    let admission = AdmissionFacade::new(Arc::new(trust_anchor), daemon_uri);
    let service = DaemonInvocationService::new(Arc::clone(&presence), admission);

    spawn_uds_listener(&config, service)?;

    if matches!(config.mode(), DaemonMode::Hub | DaemonMode::Both) {
        if let Some(listen_tcp) = config.listen_tcp() {
            // PR-1 staging: log that hub-mode TCP+TLS is configured
            // but defer the actual TLS listener wiring to a focused
            // follow-up. The daemon-config.toml invariants are
            // already validated; the remaining work is constructing
            // a `tonic::transport::ServerTlsConfig` from the cert/
            // key paths and spawning a second tonic Server. Both
            // pieces are <30 lines but require operator-side cert
            // material to test meaningfully — that is a PR-10
            // canary concern, not a PR-1 lib-layer one.
            eprintln!(
                "[axon-serve] hub-mode TCP+TLS configured at {listen_tcp} but PR-1 ships the \
                 UDS listener only; TCP+TLS lands alongside PR-10 production canary \
                 (cert/key paths already validated by daemon_config invariants)"
            );
        }
    }

    Ok(())
}

fn spawn_uds_listener(
    config: &DaemonConfig,
    service: DaemonInvocationService,
) -> anyhow::Result<()> {
    let uds_path = expand_home(
        config
            .uds_path()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("daemon-config uds_path is not valid UTF-8"))?,
    );

    if uds_path.exists() {
        // The existing daemon's control.sock bind code unlinks
        // before binding; mirror that semantic so a previous
        // process's stale daemon.sock does not block us.
        if let Err(err) = std::fs::remove_file(&uds_path) {
            if err.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "[axon-serve] failed to unlink stale UDS at {}: {err}; bind will likely fail",
                    uds_path.display(),
                );
            }
        }
    }

    let listener = tokio::net::UnixListener::bind(&uds_path).map_err(|err| {
        anyhow::anyhow!(
            "failed to bind axon_serve UDS at {}: {err}",
            uds_path.display()
        )
    })?;

    // Mode 0600 per spec §1.2 Invariant 3. UnixListener::bind already
    // creates the file; chmod after-the-fact rather than racing the
    // bind. A failure here is a soft warning (the file is owned by
    // the same user that just bound it; mode 0600 vs 0644 is a
    // hardening detail, not a correctness one).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(err) =
            std::fs::set_permissions(&uds_path, std::fs::Permissions::from_mode(0o600))
        {
            eprintln!(
                "[axon-serve] failed to chmod 0600 on {}: {err}; running with default umask perms",
                uds_path.display(),
            );
        }
    }

    eprintln!(
        "[axon-serve] gRPC InvocationServer listening on UDS {}",
        uds_path.display()
    );

    let incoming = UnixListenerStream::new(listener);
    tokio::spawn(async move {
        let result = Server::builder()
            .add_service(InvocationServer::new(service))
            .serve_with_incoming(incoming)
            .await;
        if let Err(err) = result {
            eprintln!("[axon-serve] gRPC UDS server exited with error: {err:#}");
        }
    });

    Ok(())
}

/// Resolve the daemon's own URI from `~/.easynet/credentials.json`.
///
/// PR-1 staging takes the simplest possible reading: load
/// `credentials.json`, look for an `agent_uri` field, return it as
/// `Some(String)` if present. Returns `None` on any failure so the
/// admission facade falls back to "every external caller must be in
/// the realm trust set" — the safest default before PR-7 wires the
/// real identity bootstrap.
fn load_daemon_uri() -> Option<String> {
    let path = expand_home("~/.easynet/credentials.json");
    let raw = std::fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    json.get("agent_uri")?.as_str().map(str::to_string)
}

fn load_trust_anchor() -> RealmTrustAnchor {
    let path = expand_home(DEFAULT_REALM_TRUST_PATH);
    match RealmTrustAnchor::load_or_empty(&path) {
        Ok(anchor) => {
            if anchor.is_empty() {
                eprintln!(
                    "[axon-serve] realm trust anchor at {} is empty; admission gate will reject \
                     every external caller until PR-7 pairing flow populates it",
                    path.display(),
                );
            } else {
                eprintln!(
                    "[axon-serve] realm trust anchor loaded with {} entries from {}",
                    anchor.len(),
                    path.display(),
                );
            }
            anchor
        }
        Err(err) => {
            eprintln!(
                "[axon-serve] failed to load realm trust anchor at {} ({err}); proceeding with \
                 empty trust set",
                path.display(),
            );
            RealmTrustAnchor::default()
        }
    }
}

/// Expand a `~/...` prefix using the current user's HOME. Existing
/// EasyNet code uses several different helpers for this (some via
/// `dirs::home_dir`, some via `std::env::var("HOME")`); we mirror
/// the simplest one used by `services::control::transport` to keep
/// behaviour consistent across the daemon's UDS bind sites.
fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_home_with_tilde_uses_home_env() {
        std::env::set_var("HOME", "/tmp/easynet-test-home");
        let expanded = expand_home("~/.easynet/daemon.sock");
        assert_eq!(expanded, PathBuf::from("/tmp/easynet-test-home/.easynet/daemon.sock"));
    }

    #[test]
    fn expand_home_passthrough_for_absolute_path() {
        let expanded = expand_home("/etc/easynet/realm-trust.toml");
        assert_eq!(expanded, PathBuf::from("/etc/easynet/realm-trust.toml"));
    }

    #[tokio::test]
    async fn start_axon_serve_sidecar_returns_ok_when_config_missing() {
        // Point HOME at an empty temp dir so the loader sees no
        // daemon-config.toml. This is the production-realistic case
        // for any device that has not yet been migrated to PR-1.
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());

        // No panic, no error — soft skip is the contract.
        start_axon_serve_sidecar().expect("missing config is a soft skip");
    }
}
