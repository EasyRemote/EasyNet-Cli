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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

use crate::pb::axon::v1::invocation_server::InvocationServer;
use crate::persistence::daemon_config::{DaemonConfig, DaemonMode, DEFAULT_DAEMON_CONFIG_PATH};
use crate::runtime::ability_dispatch::AbilityDispatcher;
use crate::services::axon_serve::admission_facade::AdmissionFacade;
use crate::services::axon_serve::daemon_invocation_service::DaemonInvocationService;
use crate::services::axon_serve::local_ability_dispatcher::LocalAbilityDispatcher;
use crate::services::axon_serve::session_initiator::run_session_supervisor;
use crate::services::pending_dispatch::PendingDispatchMap;
use crate::services::presence_registry::PresenceRegistry;
use crate::services::realm_trust_anchor::{RealmTrustAnchor, DEFAULT_REALM_TRUST_PATH};
use crate::services::trust_anchor_cell::SharedTrustAnchor;

/// Bring the RFC-003 transport plane online as a sidecar to the
/// existing easynet-daemon process.
///
/// Returns `Ok(())` whether or not any listener was spawned — a
/// missing daemon-config.toml is the legitimate "this device is not
/// running the new transport plane yet" state, not an error. When
/// listeners do come up, they run on the caller's tokio runtime as
/// detached tasks; they own their `PresenceRegistry` Arc and stay
/// alive until the runtime shuts down.
pub fn start_axon_serve_sidecar(dispatcher: Arc<AbilityDispatcher>) -> anyhow::Result<()> {
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
    // PR-7 commit 7/N adds an env-override seam: production deploys
    // use `/etc/easynet/realm-trust.toml`; tests / smoke runs set
    // `EASYNET_REALM_TRUST_PATH` to a tempdir-rooted path so the
    // daemon writes its trust set under the test's HOME instead of
    // requiring `/etc/easynet/` write permission. The override is
    // intentionally narrow (one path, no other behaviour change) so
    // production paths cannot diverge accidentally.
    let trust_anchor_path = trust_anchor_path_from_env_or_default();
    let trust_anchor = load_trust_anchor_from(&trust_anchor_path);
    // PR-7 commit 5/N: wrap the boot-time anchor in a reload-friendly
    // cell. The same cell is handed to the admission facade *and* to
    // `<self>.register_device_pubkey`'s handler context — a successful
    // register call atomically writes the file and republishes the
    // cell so the next admission sees the new entry without a daemon
    // restart.
    let trust_anchor_cell = SharedTrustAnchor::new(Arc::new(trust_anchor));
    let presence = Arc::new(PresenceRegistry::new());
    let pending = Arc::new(PendingDispatchMap::new());
    let admission =
        AdmissionFacade::with_trust_anchor_cell(trust_anchor_cell.clone(), daemon_uri.clone());
    let service = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_pending(Arc::clone(&pending))
        .with_register_pubkey(
            config.realm().to_string(),
            trust_anchor_path,
            trust_anchor_cell,
        );

    spawn_uds_listener(&config, service)?;

    // Hub-mode TCP+TLS — staged for PR-10 (cert material required).
    if matches!(config.mode(), DaemonMode::Hub | DaemonMode::Both) {
        if let Some(listen_tcp) = config.listen_tcp() {
            eprintln!(
                "[axon-serve] hub-mode TCP+TLS configured at {listen_tcp} but PR-1 ships the \
                 UDS listener only; TCP+TLS lands alongside PR-10 production canary \
                 (cert/key paths already validated by daemon_config invariants)"
            );
        }
    }

    // Device-mode: dial the configured hub and hold a long-lived
    // `<self>.session` bidi open for the daemon's lifetime. This is
    // what makes "device 连 hub + 保活" a real-world fact rather than
    // a library-level capability. Spec §1.3 ties the outbound dial
    // to device mode only.
    if matches!(config.mode(), DaemonMode::Device) {
        if let (Some(hub_endpoint), Some(caller_uri)) =
            (config.hub_endpoint().map(str::to_string), daemon_uri)
        {
            spawn_session_supervisor(hub_endpoint, caller_uri, dispatcher);
        } else {
            eprintln!(
                "[axon-serve] device-mode daemon missing either hub_endpoint or \
                 credentials.json agent_uri; outbound `<self>.session` not started"
            );
        }
    }

    Ok(())
}

/// Spawn the long-lived device-side `<self>.session` supervisor. The
/// supervisor dials the hub at boot, holds the bidi open, and
/// reconnects with exponential backoff on failure (250ms → 30s).
/// Runs forever on the daemon's tokio runtime; cancelled implicitly
/// when the runtime shuts down (the `cancel` oneshot we hand it is
/// dropped, which the supervisor treats the same as a cancel signal).
fn spawn_session_supervisor(
    hub_endpoint: String,
    caller_uri: String,
    dispatcher: Arc<AbilityDispatcher>,
) {
    eprintln!(
        "[axon-serve] STAGING: device-mode dialing `<self>.session` against {hub_endpoint} as \
         {caller_uri}; LocalAbilityDispatcher now holds the boot-threaded AbilityDispatcher Arc, \
         but SessionDispatch::Dispatch frames still receive a typed \"not-yet-wired\" error \
         reply until PR-2 commit 2/N wires real LocalAbilityRegistry dispatch"
    );
    // Cancel oneshot held for the daemon process's lifetime — the
    // supervisor exits when the cancel sender drops, which happens
    // when the tokio runtime tears down at process shutdown. PR-7
    // wires real graceful-shutdown via the SIGTERM signal handler;
    // until then `Box::leak` is the idiomatic "this thing lives as
    // long as the process" expression (clearer than
    // `std::mem::forget` because it makes the leak the explicit
    // intent rather than a side-effect of forgetting to drop).
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    Box::leak(Box::new(cancel_tx));
    let dispatcher = Arc::new(LocalAbilityDispatcher::new(dispatcher));
    tokio::spawn(run_session_supervisor(
        hub_endpoint,
        caller_uri,
        dispatcher,
        cancel_rx,
    ));
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
///
// TODO(pr7): replace this stringly-typed `serde_json::Value` lookup
// with `easynet_cli::persistence::config::Credentials` (typed
// loader, ed25519 seed validation, atomic-write semantics). The
// staging shape here is intentionally minimal so PR-1's binary
// integration unblocks without depending on PR-7's identity
// bootstrap; the Credentials struct already exists in the
// persistence layer and is the right consumer for both this
// loader and the eventual envelope-signing path.
fn load_daemon_uri() -> Option<String> {
    let path = expand_home("~/.easynet/credentials.json");
    let raw = std::fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    json.get("agent_uri")?.as_str().map(str::to_string)
}

/// Resolve the realm-trust file path from the env override or fall
/// back to `/etc/easynet/realm-trust.toml`. The override is the one
/// seam the PR-7 commit 7/N e2e test uses to redirect the daemon's
/// trust write to a tempdir; production callers leave it unset.
fn trust_anchor_path_from_env_or_default() -> PathBuf {
    if let Some(override_path) = std::env::var_os("EASYNET_REALM_TRUST_PATH") {
        return expand_home(override_path.to_string_lossy().as_ref());
    }
    expand_home(DEFAULT_REALM_TRUST_PATH)
}

fn load_trust_anchor_from(path: &Path) -> RealmTrustAnchor {
    match RealmTrustAnchor::load_or_empty(path) {
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
        assert_eq!(
            expanded,
            PathBuf::from("/tmp/easynet-test-home/.easynet/daemon.sock")
        );
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
        let registry = Arc::new(crate::runtime::ability_dispatch::LocalAbilityRegistry::default());
        let gateway: Arc<dyn crate::runtime::gateway_api::GatewayApi> =
            Arc::new(crate::runtime::gateway::NoopGateway::new());
        let dispatcher = Arc::new(AbilityDispatcher::new(registry, gateway));

        // No panic, no error — soft skip is the contract.
        start_axon_serve_sidecar(dispatcher).expect("missing config is a soft skip");
    }
}
