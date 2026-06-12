// EasyNet CLI — daemon Invocation transport boot wiring
// =====================================================
//
// File: src/services/invocation_transport/boot.rs
// Description: Loads RFC-003 PR-1 configuration from disk and brings
//              the gRPC InvocationServer online as the daemon's
//              first-class Invocation transport.
//
// What this module does
// ---------------------
// `boot::start_daemon_invocation_transport(...)` is the one function
// the daemon binary calls to bring the Invocation transport online. It:
//
// 1. Loads `~/.easynet/daemon-config.toml` via `DaemonConfig::load`.
//    A missing or malformed file is a soft failure — we log and
//    return without spawning any listener so the legacy daemon
//    subsystems (control.sock, runtime-dispatch, heartbeat) keep
//    working unchanged.
// 2. Loads `~/.easynet/credentials.json` to derive the daemon's own
//    URA; threads it into `AdmissionFacade` as the loopback bypass
//    so the daemon can call its own RPCs without entering the
//    realm trust set.
// 3. Loads `/etc/easynet/realm-trust.toml` (or the
//    daemon-config-supplied override) via
//    `RealmTrustAnchor::load_or_empty`. Empty fallback is fine for
//    PR-1; PR-7 populates it via the pairing flow; PR-10 canary
//    refuses to swap without a non-empty file.
// 3.5 On unix, listens for SIGHUP and reloads the trust anchor from
//     disk into the shared cell. This is the operator-facing "manual
//     edit + hup" path PR-7 checklist requires before a future file
//     watcher RFC exists.
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
// - Touch the existing daemon subsystems (Kernel, ScheduleService,
//   control.sock server, runtime-dispatch.sock,
//   heartbeat). Those keep running unchanged.
// - Implement graceful shutdown. PR-1 spec §1 and §7.2 cite
//   `systemctl restart easynet-daemon` as the operational restart
//   recipe for both config reload and TLS cert rotation; tonic's
//   `serve_with_shutdown` plus the existing ctrlc handler can be
//   wired in a follow-up commit but is not on PR-1's critical
//   path.
// - Pre-create the UDS file's parent directory. The existing daemon
//   already ensures `~/.easynet/` exists before the control.sock
//   bind earlier in `main`; this transport runs after, so the
//   directory is guaranteed present.
//
// Failure handling
// ----------------
// Missing transport config remains a soft skip for pre-transport
// devices. Once device-mode transport config exists, local gRPC
// listener readiness is daemon-owned: the daemon reports its UDS
// Invocation surface as ready when it is actually listening. Hub
// `<self>.session` admission is observed in a background task and
// logged as an admission signal, but transient hub latency must not
// make the local control plane kill an otherwise healthy daemon
// before it can reconnect and republish owner projections.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::SigningKey;

use crate::runtime::axon_bridge::hot_agent_registrar::{
    HotAgentAdvertiseOutcome, HotAgentAdvertiseRequest, HotAgentAdvertiser, HotAgentRevokeRequest,
};
use crate::services::invocation_transport::invoke_remote_initiator::{
    RequestOutcome, SessionRequestError,
};
use serde::Deserialize;

#[cfg(windows)]
use tonic::transport::server::Connected;
use tonic::transport::{Identity, Server, ServerTlsConfig};

#[cfg(windows)]
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
#[cfg(windows)]
use tokio::net::windows::named_pipe::NamedPipeServer;
#[cfg(windows)]
use tokio_stream::wrappers::ReceiverStream;
#[cfg(unix)]
use tokio_stream::wrappers::UnixListenerStream;

use crate::persistence::daemon_config::{
    DaemonConfig, DaemonConfigError, DaemonMode, DEFAULT_DAEMON_CONFIG_PATH,
};
use crate::runtime::publish::derive_subject_keypair;
use crate::services::invocation_transport::admission_facade::AdmissionFacade;
use crate::services::invocation_transport::daemon_invocation_service::DaemonInvocationService;
use crate::services::invocation_transport::local_session_dispatcher::LocalAxonSessionDispatcher;
use crate::services::invocation_transport::session_initiator::SessionSigningSeed;
use crate::services::invocation_transport::session_initiator::{
    initial_session_admission_probe, run_session_supervisor,
};
use crate::services::pending_dispatch::PendingDispatchMap;
use crate::services::presence_registry::PresenceRegistry;
use crate::services::realm_trust_anchor::{
    RealmTrustAnchor, TrustedAgent, TrustedAgentRole, DEFAULT_REALM_TRUST_PATH,
};
use crate::services::trust_anchor_cell::SharedTrustAnchor;
use crate::services::usage_quota_store::SharedUsageQuotaGate;
#[cfg(windows)]
use crate::support::named_pipe::PipeListener;
use easynet_axon::pb::axon::v1::invocation_server::InvocationServer;

/// Maximum decoded gRPC message size for InvocationServer/Client on
/// both directions. tonic's default cap is 4 MiB which aborted
/// `<self>.session` the moment any frame envelope grew past it (real
/// trigger: file-transfer uploads ≥ 1 MB whose accumulated down
/// frames cross 4 MiB). 64 MiB is deliberately a transport-envelope
/// cap, not an ability payload cap: large files and snapshots must be
/// chunked above gRPC instead of granting every peer a near-unbounded
/// single-message allocation. Exposed `pub` because the **client** side
/// (`session_initiator`, `invoke_remote_initiator`) must apply the
/// same cap as the server side; without that the asymmetry triggers
/// `OutOfRange: decoded message length too large` mid-stream.
pub const MAX_INVOCATION_GRPC_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
const INITIAL_SESSION_ADMISSION_TIMEOUT: Duration = Duration::from_secs(15);

#[cfg(windows)]
#[derive(Debug)]
struct NamedPipeGrpcIo(NamedPipeServer);

#[cfg(windows)]
impl Connected for NamedPipeGrpcIo {
    type ConnectInfo = ();

    fn connect_info(&self) -> Self::ConnectInfo {}
}

#[cfg(windows)]
impl AsyncRead for NamedPipeGrpcIo {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

#[cfg(windows)]
impl AsyncWrite for NamedPipeGrpcIo {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

/// Bring the daemon Invocation transport online inside the
/// `easynet-daemon` process.
///
/// Returns `Ok(())` whether or not any listener was spawned — a
/// missing daemon-config.toml is the legitimate "this device is not
/// running the new transport plane yet" state, not an error. When
/// listeners do come up, they run on the caller's tokio runtime as
/// detached tasks; they own their `PresenceRegistry` Arc and stay
/// alive until the runtime shuts down.
/// **Phase 5c**. `hot_agent_registrar_cell` is the shared
/// `OnceLock<Arc<HotAgentRegistrar>>` stashed by
/// `build_registry_with_services`. We call
/// `registrar.set_runtime(local_runtime)` once `local_runtime` is
/// constructed below so post-boot `agent.start` invocations
/// land their `<agent>.{chat,discover,invoke}` rows into the live
/// Axon runtime instead of skipping runtime registration.
/// Owns the session supervisor's cancel oneshot. Dropping it (at
/// daemon shutdown) resolves the supervisor's `cancel` branch, so the
/// in-flight `<self>.session` dial drains cleanly instead of being
/// killed at the process level (the hub then sees a clean Eof, not a
/// StreamReset). An empty handle (hub mode, unconfigured, no device
/// identity) is a no-op on drop.
#[must_use = "drop at shutdown to cancel the session supervisor; \
              dropping immediately tears the session down"]
pub struct SessionShutdown(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for SessionShutdown {
    fn drop(&mut self) {
        // Dropping the sender resolves the supervisor's `cancel`
        // future; an explicit Drop also makes the field's role
        // (consumed only at teardown) legible to readers and clippy.
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

impl SessionShutdown {
    fn none() -> Self {
        Self(None)
    }
}

pub fn start_daemon_invocation_transport(
    local_runtime: Arc<easynet_axon::invocation::LocalRuntime>,
    invocation_ledger: Option<Arc<easynet_axon::invocation::InvocationLedger>>,
    hot_agent_registrar_cell: Arc<
        crate::runtime::agents::agent_lifecycle_ability::SharedHotRegistrarCell,
    >,
    plugin_runtime_manager: Option<Arc<crate::runtime::plugin_host::PluginRuntimeManager>>,
) -> anyhow::Result<SessionShutdown> {
    let config_path = expand_home(DEFAULT_DAEMON_CONFIG_PATH);
    let config = match DaemonConfig::load(&config_path) {
        Ok(cfg) => cfg,
        // An ABSENT config is the legitimate "this device has not opted into
        // the transport plane yet" state: skip the listener, boot continues.
        Err(DaemonConfigError::ReadFailed { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            let config_path_display = format!("{}", config_path.display());
            crate::op_event!(
                component = daemon_invocation,
                kind = transport_plane_config_absent,
                config_path = config_path_display,
                message = "no daemon-config.toml; skipping gRPC listener",
            );
            return Ok(SessionShutdown::none());
        }
        // A PRESENT-but-broken config (parse error, illegal field, bad TLS
        // pairing, …) is an operator mistake, not "unconfigured". Silently
        // skipping the listener here is how a hub boots "successfully" while
        // never binding its Invocation surface. Fail fast so the mistake is
        // visible at boot instead of as a mysteriously dead hub.
        Err(err) => {
            return Err(anyhow::Error::new(err).context(format!(
                "daemon-config.toml at {} is present but invalid; refusing to \
                 boot the Invocation transport with a broken config",
                config_path.display(),
            )));
        }
    };

    let daemon_identity = load_daemon_identity();
    let daemon_ura = daemon_identity
        .as_ref()
        .map(|identity| identity.caller_ura.clone());
    if let Some(identity) = daemon_identity.as_ref() {
        maybe_bootstrap_runtime_self_identity(identity);
    }
    // PR-7 commit 7/N adds an env-override seam: production deploys
    // use `/etc/easynet/realm-trust.toml`; tests / smoke runs set
    // `EASYNET_REALM_TRUST_PATH` to a tempdir-rooted path so the
    // daemon writes its trust set under the test's HOME instead of
    // requiring `/etc/easynet/` write permission. The override is
    // intentionally narrow (one path, no other behaviour change) so
    // production paths cannot diverge accidentally.
    let trust_anchor_path = trust_anchor_path_from_env_or_default();
    let trust_anchor = upsert_backend_identity_from_disk(
        config.realm(),
        &trust_anchor_path,
        load_trust_anchor_from(&trust_anchor_path),
    );
    // PR-7 commit 5/N: wrap the boot-time anchor in a reload-friendly
    // cell. The same cell is handed to the admission facade *and* to
    // `<self>.register_device_pubkey`'s handler context — a successful
    // register call atomically writes the file and republishes the
    // cell so the next admission sees the new entry without a daemon
    // restart.
    let trust_anchor_cell = SharedTrustAnchor::new(Arc::new(trust_anchor));
    let presence = Arc::new(PresenceRegistry::new());
    let pending = Arc::new(PendingDispatchMap::new());
    let pending_stream =
        Arc::new(crate::services::pending_dispatch::PendingStreamDispatchMap::new());

    // Demo-only presence seed (cfg-gated). Production binaries
    // built without `--features demo-fixture` cannot honour the
    // `EASYNET_DEMO_PRESENCE_SEED` env var no matter how it gets
    // injected (container env, systemd unit override, etc.) —
    // the symbol simply isn't there. Demo / e2e scripts pass
    // `cargo build --features demo-fixture` to opt in.
    maybe_seed_demo_presence(&presence);

    // Device-mode self-presence seed.
    //
    // In device-mode the daemon's local PresenceRegistry is used by
    // backend's `federation.resolve` (over the daemon UDS) to answer
    // "which devices in this realm are online?". The hub-side
    // presence registry holds the canonical answer, but in
    // host-mode dev rigs (backend → device daemon UDS, no separate
    // hub-mode daemon process) the backend never reaches the hub's
    // presence — it queries this daemon's local one instead.
    // Pre-this-fix: device daemon's local presence was empty because
    // <self>.session is an OUTBOUND dial (the daemon dials the hub),
    // not an inbound register, so nothing populated the local table.
    // backend's `federation.resolve` then returned no agents; every
    // device showed REMOVED in /api/v1/devices despite the bidi
    // being healthy.
    //
    // Seed the local presence with the daemon's own URA on boot so
    // the local resolve answers "yes I'm here" when the operator's
    // backend asks. The dispatch sender pushes into a drain task
    // (kept alive as long as the daemon process), so try_send never
    // observes Closed/Full and the entry stays in the registry.
    // For actual ability invokes targeting this URA, the
    // `daemon_invocation_service` <self>.invoke_remote handler
    // already short-circuits self-targeted invocations to the
    // local Axon session dispatcher BEFORE try_send fires (see
    // dispatch_self_targeted_forward_invoke in PR-1 commit 7/9).
    //
    // Hub / Both modes don't need this: their local presence is
    // already populated by inbound device sessions, and the hub
    // itself is the directory-of-record. Device-only.
    if matches!(config.mode(), DaemonMode::Device) {
        if let Some(ura) = daemon_ura.as_ref() {
            let (noop_tx, mut noop_rx) = tokio::sync::mpsc::channel(
                crate::services::presence_registry::DISPATCH_CHANNEL_CAPACITY,
            );
            // Drain task: holds the receiver alive for the lifetime
            // of the daemon process. Without this, the receiver
            // gets dropped when the seeding scope ends and the
            // sender's first try_send observes Closed → presence
            // entry deleted → the very state we're trying to fix.
            tokio::spawn(async move {
                while let Some(_frame) = noop_rx.recv().await {
                    // Drop on the floor. The self-targeted
                    // dispatch path runs inline through Axon
                    // LocalRuntime; only defensive out-of-path frames
                    // land here.
                }
            });
            let prior = presence.insert(ura.clone(), noop_tx);
            if prior.is_none() {
                crate::op_event!(
                    component = daemon_invocation,
                    kind = device_mode_self_presence_seeded,
                    self_ura = ura,
                    message = "drain task holds receiver; self-targeted invokes route through Axon LocalRuntime",
                );
            }
        }
    }

    // Federated_peers cell first so we can hand it to BOTH the
    // DaemonInvocationService (for cross-hub `forward_invoke`
    // routing) and the AdmissionFacade (for `FederatedKeyResolver`
    // cross-realm signature verify against peer hubs).
    let federated_peers_cell = crate::services::federated_peers_cell::SharedFederatedPeers::new(
        config.federated_peers().clone(),
    );

    // **PR-N3 commit N3-3 + N3-4**. The cross-realm directory
    // cell. Lives at the daemon scope so any consumer of the
    // current federated directory snapshot — the
    // `federation.discover` dispatch arm (N3-4),
    // `federation.list_user_devices` peer projection (N3-5), or
    // a future audit query — calls `.snapshot()` for an Arc
    // clone that stays stable for the duration of one read.
    // The per-peer `RemoteDirectoryClient` tasks that populate
    // this cell are spawned by the follow-up commit (N3-3.1)
    // that integrates with `services::federation_client::
    // CrossHubDialer`'s subscribe-stream surface; today the
    // cell starts empty so consumers see no peer entries yet
    // (and gracefully degrade to local-only behaviour, the
    // same shape they show on a single-realm daemon).
    let federated_directory_cell =
        crate::services::federation_directory::SharedFederatedDirectoryView::default();

    // PR-N1 commit 9/N + PR-N2 commit 1/N: hub-mode daemons
    // construct one CrossHubDialer that backs both the daemon's
    // outbound `forward_invoke` routing AND the admission gate's
    // `FederatedKeyResolver` so a cross-realm caller's URA can be
    // resolved via `federation.resolve_key` against the peer hub.
    // Device-mode daemons never originate federation calls, so
    // both surfaces stay local-only.
    let dialer: Option<Arc<dyn crate::services::federation_client::FederationClient>> =
        if matches!(config.mode(), DaemonMode::Hub | DaemonMode::Both) {
            Some(Arc::new(
                crate::services::federation_client::CrossHubDialer::with_trust_anchor_cell(
                    trust_anchor_cell.clone(),
                ),
            ))
        } else {
            None
        };

    let mut admission =
        AdmissionFacade::with_trust_anchor_cell(trust_anchor_cell.clone(), daemon_ura.clone());
    if let Some(client) = dialer.clone() {
        admission = admission.with_federation(client, federated_peers_cell.clone());
    }
    // #185: one hot-swappable quota gate is shared by both listeners.
    // The gate exists even when quota starts disabled so SIGHUP can
    // enable, disable, or retune `[daemon.quota]` without rebuilding
    // the admission facade.
    let quota_gate = SharedUsageQuotaGate::from_policy(config.quota().cloned());
    admission = admission.with_quota_gate(quota_gate.clone());
    // Grab a clone of the federated-key cache handle BEFORE
    // ownership of the AdmissionFacade moves into the service,
    // so the unified SIGHUP reload task (below) can flush
    // cached cross-realm pubkeys after every reload (key
    // rotation must not wait for the 5-min per-entry TTL).
    let federated_key_cache = admission.federated_key_cache();
    // **Unified SIGHUP reload coordinator** (replaces the
    // previous independent tasks). One task, one signal
    // listener, processes trust-anchor reload + federated_peers
    // reload + quota reload + key-cache flush in deterministic sequence per
    // signal — eliminates the race window where a federated
    // cross-realm admission could fire between the individual
    // reloads landing.
    spawn_unified_sighup_reload_task(
        trust_anchor_path.clone(),
        trust_anchor_cell.clone(),
        config_path.clone(),
        federated_peers_cell.clone(),
        quota_gate.clone(),
        federated_key_cache,
    );
    let mut service = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_pending(Arc::clone(&pending))
        .with_pending_stream(Arc::clone(&pending_stream))
        .with_session_realm(config.realm().to_string())
        .with_register_pubkey(
            config.realm().to_string(),
            trust_anchor_path.clone(),
            trust_anchor_cell.clone(),
        )
        .with_federated_directory_cell(federated_directory_cell.clone())
        // **2026-05-25 P0 hardening**. Thread the operator's
        // directory-auto-route opt-in (default false) so the
        // federation dispatcher refuses to dial a peer hub whose
        // endpoint came only from a `federated_directory`
        // observation unless the operator explicitly enabled it.
        // See `hub_resolver.rs` for the threat model.
        .with_allow_directory_auto_route(config.allow_directory_auto_route());

    // ── Invocation ledger ──────────────────────────────────────────
    //
    // Resolve the ledger ONCE so the same `Arc<InvocationLedger>`
    // can be handed to both:
    //   (a) the dispatch service's legacy unary-record-write path
    //       (kept until Phase 4 retires it), AND
    //   (b) the Axon `LedgerSink` installed on the `LocalRuntime`
    //       below, which will own all terminal persistence once
    //       Phase 4 routes every invoke through Axon.
    //
    // Resolution order: explicit `invocation_ledger` argument from
    // the daemon main first (tests use this seam), then a default
    // open at `<ledger_dir>/invocations.redb`. Open failure leaves
    // the slot `None` — Phase 4 ledger writes silently no-op, same
    // operational degradation as before.
    let resolved_ledger: Option<Arc<easynet_axon::invocation::InvocationLedger>> =
        match invocation_ledger {
            Some(ledger) => Some(ledger),
            None => {
                match easynet_axon::invocation::InvocationLedger::open(
                    config.ledger_dir().join("invocations.redb"),
                ) {
                    Ok(ledger) => Some(Arc::new(ledger)),
                    Err(err) => {
                        let ledger_dir_display = format!("{}", config.ledger_dir().display());
                        let err_msg = format!("{err}");
                        crate::op_event!(
                            component = daemon_invocation,
                            kind = invocation_ledger_disabled,
                            ledger_dir = ledger_dir_display,
                            error = err_msg,
                        );
                        None
                    }
                }
            }
        };
    if let Some(ledger) = resolved_ledger.as_ref() {
        service = service.with_invocation_ledger(Arc::clone(ledger));
    }

    // ── Shared LocalRuntime ───────────────────────────────────────
    //
    // The daemon creates the Axon `LocalRuntime` before building
    // abilities, so registration lands directly in the runtime.
    // This transport only installs transport-plane configuration:
    //   * a `RealmTrustAnchorKeyResolver` over the SAME
    //     `trust_anchor_cell` admission uses, so signature
    //     verification inside `invoke_externally_signed_*` sees
    //     hot-reload edits immediately;
    //   * a `LedgerSink` over the SAME ledger handle the dispatch
    //     service already writes to, so once Phase 4 routes
    //     through Axon, terminal records get persisted via the
    //     SDK-canonical path (one writer, not two).
    //
    crate::runtime::axon_bridge::runtime_factory::configure_local_runtime(
        &local_runtime,
        Some(Arc::new(
            crate::services::trust_anchor_key_resolver::RealmTrustAnchorKeyResolver::new(
                trust_anchor_cell.clone(),
            ),
        )),
        resolved_ledger.clone(),
    );
    if let Err(err) =
        futures::executor::block_on(local_runtime.install_bootstrap_self_identity_admin())
    {
        let err_msg = err.to_string();
        crate::op_event!(
            component = daemon_invocation,
            kind = axon_local_runtime_admin_install_failed,
            level = "warn",
            error = err_msg,
            message = "failed to install Axon SDK runtime.bootstrap_self_identity admin ability",
        );
    }

    // **Phase 5c**. Attach the live `LocalRuntime` to the hot-agent
    // runtime registrar that `build_registry_with_services` constructed
    // earlier. After this `set_runtime` call, every subsequent
    // `agent.start` invocation reaches into the registrar's
    // populated runtime cell and lands `<agent>.{chat,discover,invoke}`
    // into Axon's `LocalRuntime` — closing the bug where hot-added
    // agents were visible in product metadata but not materialized in
    // Axon's runtime, therefore skipping the `LedgerSink` audit row.
    //
    // The cell is normally populated by `build_registry_with_services`,
    // so `.get()` returns `Some`. We log + skip when absent to keep
    // smoke tests that boot only the transport (without a full registry
    // build) green: those tests don't call `agent.start`, so a
    // pending registrar is observably harmless.
    if let Some(registrar) = hot_agent_registrar_cell.get() {
        registrar.set_runtime(Arc::clone(&local_runtime));
        crate::op_event!(
            component = daemon_invocation,
            kind = hot_agent_registrar_runtime_attached,
            message = "HotAgentRegistrar.runtime attached; \
                       agent.start can now register into LocalRuntime",
        );
    } else {
        crate::op_event!(
            component = daemon_invocation,
            kind = hot_agent_registrar_cell_empty,
            level = "warn",
            message = "hot_agent_registrar_cell empty at boot — \
                       agent.start runtime registration will be skipped \
                       (Invocation transport booted without a populated registry?)",
        );
    }

    let runtime_ability_count = futures::executor::block_on(local_runtime.list_abilities()).len();
    let runtime_ability_count_str = runtime_ability_count.to_string();
    crate::op_event!(
        component = daemon_invocation,
        kind = axon_local_runtime_wired,
        has_ledger_sink = resolved_ledger.is_some().to_string().as_str(),
        runtime_abilities = runtime_ability_count_str.as_str(),
        message = "Axon LocalRuntime configured; ability registration already landed directly in LocalRuntime",
    );

    let ability_wire_registry = plugin_runtime_manager
        .as_ref()
        .map(|manager| manager.ability_wire_registry())
        .unwrap_or_else(|| {
            match crate::runtime::ability_wire::AbilityWireRegistry::load_default_profile() {
                Ok(registry) => Arc::new(registry),
                Err(err) => {
                    let error = err.to_string();
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = ability_wire_registry_load_failed,
                        level = "warn",
                        error = error.as_str(),
                        message = "daemon will use core bidi wire profiles only",
                    );
                    Arc::new(crate::runtime::ability_wire::AbilityWireRegistry::core())
                }
            }
        });

    service = service.with_local_runtime(Arc::clone(&local_runtime));
    service = service.with_ability_wire_registry(Arc::clone(&ability_wire_registry));

    if let Ok(seed) =
        crate::services::invocation_transport::peer_envelope_signer::read_hub_identity_seed(
            config.realm(),
        )
    {
        service = service.with_hub_signing_seed(seed);
    }

    // PR-N1 commit 6/N (boot wiring) + commit 9/N (SIGHUP-aware
    // trust anchor) + commit 10/N (SHIGHUP-aware federated_peers)
    // + PR-N2 commit 1/N (FederatedKeyResolver wiring): the dialer
    // and federated_peers cell were constructed above so the
    // AdmissionFacade could pick them up too. Here we forward the
    // same handles to the DaemonInvocationService for the
    // cross-realm `forward_invoke` dispatch path.
    if let Some(client) = dialer.clone() {
        service = service
            .with_federation_client(client)
            .with_federated_peers_cell(federated_peers_cell.clone());
    }

    // **PR-N6 C4**. Device-mode daemon's `forward_invoke` escalates
    // up the long-lived `<self>.session` bidi to the hub instead of
    // consulting its (always-empty) local PresenceRegistry. Three
    // collaborators wired here:
    //
    //   1. `EscalationCorrelation` — call_id → oneshot table.
    //      Cloned into the service's `LocalAxonSessionDispatcher`
    //      builder so inbound `RequestResult` frames complete the
    //      awaiting dispatcher future.
    //   2. `SharedSessionOutbox` — published by the session
    //      supervisor on every successful dial, cleared on
    //      disconnect. The escalation consumer reads it
    //      per-Request.
    //   3. `SessionEscalationHandle` — what the dispatcher's
    //      `escalate_forward_invoke` arm calls. Wired into the
    //      service via `with_session_escalation`.
    //
    // Hub / Both modes leave `escalation_state = None`. Their
    // dispatcher's existing local-presence + cross-hub dial arms
    // run unchanged.
    let escalation_state = if matches!(config.mode(), DaemonMode::Device) {
        let correlation =
            crate::services::invocation_transport::session_escalation::EscalationCorrelation::new();
        let outbox =
            crate::services::invocation_transport::session_escalation::SharedSessionOutbox::new();
        let handle = std::sync::Arc::new(
            crate::services::invocation_transport::session_escalation::spawn_escalation_consumer_with_outbox(
                Arc::clone(&correlation),
                outbox.clone(),
                config.realm().to_string(),
            ),
        );
        if let (Some(registrar), Some(identity)) =
            (hot_agent_registrar_cell.get(), daemon_identity.as_ref())
        {
            registrar.set_hot_agent_advertiser(Arc::new(SessionHotAgentAdvertiser::new(
                Arc::clone(&handle),
                identity.caller_ura.clone(),
            )));
        }
        service = service.with_session_escalation(Arc::clone(&handle));
        // One DeviceTrustSync per daemon: the self-targeted
        // `<self>.invoke_remote` dispatch arm (service) and the
        // `<self>.session` dispatcher both warm the anchor through
        // this instance, sharing its single-flight map and negative
        // cache. It rides the session escalation channel built above.
        let device_trust_sync = Arc::new(
            crate::services::invocation_transport::device_trust_sync::DeviceTrustSync::new(
                config.realm().to_string(),
                trust_anchor_path.clone(),
                trust_anchor_cell.clone(),
                Arc::clone(&handle),
            ),
        );
        service = service.with_device_trust_sync(Arc::clone(&device_trust_sync));
        Some(DeviceEscalationState {
            correlation,
            outbox,
            device_trust_sync,
        })
    } else {
        None
    };

    // **PR-N3 commit N3-3.1**. Spawn the polling task that
    // populates the federated directory cell by calling each
    // peer's `federation.discover` ability on a fixed cadence.
    // Cadence is 5 seconds — fast enough for the spec §八
    // scenario (4) "new peer SIGHUP appears in <self>.discover
    // within ~5s" + slow enough that peer hubs aren't pounded.
    // The task reads the federated_peers cell each round so a
    // SIGHUP-driven add/drop is naturally picked up; no
    // separate add/drop signalling needed.
    if let Some(client) = dialer.clone() {
        // **Streaming-only directory federation**. The
        // streaming supervisor (PR-N3 N3-streaming-4) watches
        // the federated_peers cell and spawns one
        // `subscribe_directory_v2` subscriber per entry. Every
        // current peer hub runs the same daemon binary which
        // serves v2 unconditionally, so the legacy poll task
        // (`spawn_federated_directory_poll_task`) is dead code
        // in production — its dual-path required a race-fix
        // (PR-N3 N3-streaming-10) and contributed nothing once
        // the streaming supervisor stabilised. The standalone
        // `poll_once` helper stays available for a future
        // "operator manual poll" CLI command but is no longer
        // wired into boot.
        //
        // Use the daemon's own URA as the subscribe-stream
        // envelope's caller. Falls back to a generic CLI-style
        // URA when the daemon has no credentials yet (test /
        // smoke builds) so the peer's strict-admission still
        // sees a non-empty caller field. The fallback uses the
        // v4.1.5 device shape (`r/cli/device/local`) — the
        // legacy CLI agent-placeholder shape would fail the
        // strict parser (§A.URA-3: agent tail needs a dot).
        let supervisor_caller_ura = daemon_ura
            .clone()
            .unwrap_or_else(|| "easynet:///r/cli/device/local".to_string());
        spawn_federated_directory_streaming_supervisor(
            client,
            federated_peers_cell.clone(),
            federated_directory_cell.clone(),
            supervisor_caller_ura,
        );
    }

    spawn_uds_listener(&config, service.clone())?;

    // Hub-mode TCP+TLS — PR-10 commit 1/N: real listener.
    // `DaemonConfig` already enforces invariant 2 (TCP requires
    // both cert and key); a missing cert/key file at boot is a
    // hard error, not a silent skip. Cert/key are loaded once at
    // boot — rotation requires a daemon restart. PR-10 spec INV-1
    // is fail-closed for missing material.
    if matches!(config.mode(), DaemonMode::Hub | DaemonMode::Both) {
        if let Some(listen_tcp) = config.listen_tcp() {
            // The TCP+TLS socket is off-box reachable, so its admission
            // gate must NOT honour the loopback bypass — otherwise a
            // caller that spoofs the daemon's own URA in `caller.ura`
            // would skip the trust-anchor / signature / replay pipeline.
            // The UDS listener above keeps the default (trusted) bypass.
            spawn_tcp_tls_listener(&config, listen_tcp, service.with_loopback_trusted(false))?;
        }
    }

    // Device-mode: dial the configured hub and hold a long-lived
    // `<self>.session` bidi open for the daemon's lifetime. This is
    // what makes "device 连 hub + 保活" a real-world fact rather than
    // a library-level capability. Spec §1.3 ties the outbound dial
    // to device mode only.
    let mut session_shutdown = SessionShutdown::none();
    if matches!(config.mode(), DaemonMode::Device) {
        if let (Some(hub_endpoint), Some(identity)) =
            (config.hub_endpoint().map(str::to_string), daemon_identity)
        {
            // Resolve the operator-pinned CA for this hub from
            // realm-trust.toml. With a publicly-trusted hub cert
            // (production deploy, Let's Encrypt etc.) the trust
            // anchor has no entry whose `hub_endpoint` matches and we
            // pass `None` — tonic falls back to the system trust
            // store. With a self-signed hub cert (staging /
            // single-machine demo) the operator has already
            // pinned the CA via `[[trusted_agent]] role = "hub"`
            // and `tls_ca_pem_path = ...`; we forward that path
            // so the device-side dial can validate the leaf.
            let hub_ca_pem_path = trust_anchor_cell
                .snapshot()
                .lookup_peer_hub(&hub_endpoint)
                .and_then(|entry| entry.tls_ca_pem_path.clone());
            // PR-N6 C4: forward the (correlation, outbox) pair the
            // outer block constructed when this daemon is device-
            // mode. The supervisor publishes the active up_tx into
            // the outbox on every successful dial; the
            // LocalAxonSessionDispatcher inside the supervisor receives
            // the correlation table so inbound RequestResult frames
            // resolve the awaiting dispatcher futures.
            // DEC-EU user-key sync: hand the supervisor the trust-
            // anchor write handle so each established session imports
            // the paired user's signing key from the hub registrar
            // (see session_initiator::UserTrustSync).
            let user_trust_sync =
                crate::services::invocation_transport::session_initiator::UserTrustSync {
                    daemon_realm: config.realm().to_string(),
                    trust_anchor_path: trust_anchor_path.clone(),
                    cell: trust_anchor_cell.clone(),
                };
            session_shutdown = spawn_session_supervisor(
                hub_endpoint,
                identity,
                hub_ca_pem_path,
                escalation_state,
                Arc::clone(&local_runtime),
                Arc::clone(&ability_wire_registry),
                plugin_runtime_manager.clone(),
                user_trust_sync,
            )?;
        } else {
            crate::op_event!(
                component = daemon_invocation,
                kind = device_mode_session_supervisor_not_started,
                reason = "missing_hub_endpoint_or_device_identity",
                message = "device-mode daemon missing either hub_endpoint or credentials.json device identity; outbound `<self>.session` not started",
            );
        }
    }

    Ok(session_shutdown)
}

/// Device-mode hot-advertise adapter for `agent.start`.
///
/// It reuses the already-open `<self>.session` bidi instead of
/// opening a second hub client from the lifecycle handler. The
/// lifecycle layer only sees the [`HotAgentAdvertiser`] trait; this
/// adapter owns the session-specific wire shape and caller identity.
struct SessionHotAgentAdvertiser {
    escalation:
        Arc<crate::services::invocation_transport::session_escalation::SessionEscalationHandle>,
    caller_ura: String,
    host_node_id: Option<String>,
}

impl SessionHotAgentAdvertiser {
    fn new(
        escalation: Arc<
            crate::services::invocation_transport::session_escalation::SessionEscalationHandle,
        >,
        caller_ura: String,
    ) -> Self {
        let host_node_id = crate::ura::parse_ura(&caller_ura)
            .ok()
            .filter(|parsed| parsed.kind == crate::ura::URAKind::Device)
            .and_then(|parsed| parsed.device_id().map(str::to_string));
        Self {
            escalation,
            caller_ura,
            host_node_id,
        }
    }
}

impl HotAgentAdvertiser for SessionHotAgentAdvertiser {
    fn advertise_hosted_agent(
        &self,
        request: HotAgentAdvertiseRequest,
    ) -> HotAgentAdvertiseOutcome {
        let mut body = serde_json::json!({
            "agent_ura": request.agent_ura,
            "signing_authority": {
                "kind": "hosted_by",
                "host_ura": self.caller_ura,
            },
        });
        if let Some(node_id) = self.host_node_id.as_ref() {
            if let Some(map) = body.as_object_mut() {
                map.insert(
                    "host_node_id".to_string(),
                    serde_json::Value::String(node_id.clone()),
                );
            }
        }
        let args = match serde_json::to_vec(&body) {
            Ok(args) => args,
            Err(err) => {
                return HotAgentAdvertiseOutcome {
                    advertised: false,
                    error: Some(format!("encode federation.advertise_agent args: {err}")),
                };
            }
        };
        // ISS-002: carry the abilities advertise alongside the identity
        // advertise so a hot ability add/remove reaches the hub on the
        // same `<self>.session` escalation, immediately — not at the
        // next heartbeat. Identity is advertised first (the abilities
        // projection references the agent record); the abilities
        // advertise is best-effort and reported via the outcome error.
        let abilities_payload = request.abilities_payload;
        let escalation = Arc::clone(&self.escalation);
        let Some(outcome) = crate::support::async_bridge::try_run_blocking_in_tokio(async move {
            let agent_outcome = escalation
                .escalate_with_timeout(
                    "federation.advertise_agent".to_string(),
                    args,
                    Duration::from_secs(5),
                )
                .await;
            // Only advertise abilities if the identity advertise landed —
            // an abilities projection for an unknown agent is rejected.
            // `escalate_with_timeout` builds the hub ability URA from the
            // ability name + session realm, so no resource URA is needed.
            if matches!(agent_outcome, RequestOutcome::Ok { .. }) {
                if let Some(payload) = abilities_payload {
                    let abilities_outcome = escalation
                        .escalate_with_timeout(
                            "federation.advertise_abilities".to_string(),
                            payload,
                            Duration::from_secs(5),
                        )
                        .await;
                    if let RequestOutcome::Err { error } = abilities_outcome {
                        // Identity is up; abilities will reconcile on the
                        // next heartbeat refresh. Surface the soft error.
                        return RequestOutcome::Err { error };
                    }
                }
            }
            agent_outcome
        }) else {
            return HotAgentAdvertiseOutcome {
                advertised: false,
                error: Some(
                    "no tokio runtime available for hot federation.advertise_agent".to_string(),
                ),
            };
        };
        match outcome {
            RequestOutcome::Ok { .. } => HotAgentAdvertiseOutcome {
                advertised: true,
                error: None,
            },
            RequestOutcome::Err { error } => HotAgentAdvertiseOutcome {
                advertised: false,
                error: Some(render_session_request_error(&error)),
            },
        }
    }

    fn revoke_hosted_agent(&self, request: HotAgentRevokeRequest) -> HotAgentAdvertiseOutcome {
        // ISS-002 (agent.stop, symmetric to advertise): remove the agent
        // identity from the hub directory via `federation.revoke` on the
        // same `<self>.session` escalation. `escalate_with_timeout`
        // builds the hub ability URA from the ability name + session
        // realm, so only the JSON args are passed here.
        let body = serde_json::json!({
            "agent_ura": request.agent_ura,
            "reason": request.reason,
        });
        let args = match serde_json::to_vec(&body) {
            Ok(args) => args,
            Err(err) => {
                return HotAgentAdvertiseOutcome {
                    advertised: false,
                    error: Some(format!("encode federation.revoke args: {err}")),
                };
            }
        };
        let escalation = Arc::clone(&self.escalation);
        let Some(outcome) = crate::support::async_bridge::try_run_blocking_in_tokio(async move {
            escalation
                .escalate_with_timeout(
                    "federation.revoke".to_string(),
                    args,
                    Duration::from_secs(5),
                )
                .await
        }) else {
            return HotAgentAdvertiseOutcome {
                advertised: false,
                error: Some("no tokio runtime available for hot federation.revoke".to_string()),
            };
        };
        match outcome {
            RequestOutcome::Ok { .. } => HotAgentAdvertiseOutcome {
                advertised: true,
                error: None,
            },
            RequestOutcome::Err { error } => HotAgentAdvertiseOutcome {
                advertised: false,
                error: Some(render_session_request_error(&error)),
            },
        }
    }
}

fn render_session_request_error(error: &SessionRequestError) -> String {
    match error {
        SessionRequestError::TargetOffline => "target_offline".to_string(),
        SessionRequestError::PermissionDenied { reason } => {
            format!("permission_denied: {reason}")
        }
        SessionRequestError::UpstreamFailure { reason } => {
            format!("upstream_failure: {reason}")
        }
        SessionRequestError::UpstreamTimeout => "upstream_timeout".to_string(),
    }
}

/// Spawn the long-lived device-side `<self>.session` supervisor. The
/// supervisor dials the hub at boot, holds the bidi open, and
/// reconnects with exponential backoff on failure (250ms → 30s).
/// Runs forever on the daemon's tokio runtime; cancelled implicitly
/// when the runtime shuts down (the `cancel` oneshot we hand it is
/// dropped, which the supervisor treats the same as a cancel signal).
/// Device-mode session escalation wiring, grouped so it travels as one
/// named value instead of an `Option<(Arc<...>, ..., Arc<...>)>` tuple
/// through the boot path. Built once in device mode; `None` in
/// hub/both modes.
struct DeviceEscalationState {
    correlation:
        Arc<crate::services::invocation_transport::session_escalation::EscalationCorrelation>,
    outbox: crate::services::invocation_transport::session_escalation::SharedSessionOutbox,
    device_trust_sync:
        Arc<crate::services::invocation_transport::device_trust_sync::DeviceTrustSync>,
}

fn spawn_session_supervisor(
    hub_endpoint: String,
    identity: DaemonIdentity,
    hub_ca_pem_path: Option<std::path::PathBuf>,
    escalation_state: Option<DeviceEscalationState>,
    local_runtime: Arc<easynet_axon::invocation::LocalRuntime>,
    ability_wire_registry: Arc<crate::runtime::ability_wire::AbilityWireRegistry>,
    plugin_runtime_manager: Option<Arc<crate::runtime::plugin_host::PluginRuntimeManager>>,
    user_trust_sync: crate::services::invocation_transport::session_initiator::UserTrustSync,
) -> anyhow::Result<SessionShutdown> {
    // Build the device-owner descriptor projection from the same profile
    // registry that powers `meta.list_abilities`. RFC-005 route selection
    // consumes the hub-side owner projection; constructing it from bare
    // `LocalRuntime.list_abilities()` names made the prelude a second,
    // lossy catalogue path and could omit newly-added device abilities from
    // `namespace.resolve` while the local daemon could still dispatch them.
    let ability_descriptors =
        device_owner_session_descriptors(&identity.caller_ura, plugin_runtime_manager.as_deref());
    let signing_state = if identity.signing_seed.is_some() {
        "signed frame0"
    } else {
        "legacy unsigned frame0"
    };
    let ca_state = match hub_ca_pem_path.as_deref() {
        Some(path) => format!("pinned CA `{}`", path.display()),
        None => "system trust roots".to_string(),
    };
    let escalation_state_str = if escalation_state.is_some() {
        "forward_invoke escalation wired"
    } else {
        "forward_invoke escalation OFF"
    };
    let caller_ura_display = identity.caller_ura.clone();
    crate::op_event!(
        component = daemon_invocation,
        kind = device_mode_dialing_self_session,
        hub_endpoint = hub_endpoint,
        caller_ura = caller_ura_display,
        signing_state = signing_state,
        tls = ca_state,
        escalation_state = escalation_state_str,
        message = "LocalAxonSessionDispatcher will execute inbound SessionDispatch::Dispatch frames through Axon LocalRuntime",
    );
    // Cancel oneshot held for the daemon process's lifetime. Hub
    // admission is observed asynchronously below: the local Invocation
    // transport is a daemon-owned readiness surface and must not be
    // blocked by federation prelude latency or transient hub outages.
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    let (initial_admission, initial_admission_rx) = initial_session_admission_probe();

    // PR-N6 C4: when escalation is wired (device mode), inject the
    // correlation table into the LocalAxonSessionDispatcher so inbound
    // RequestResult frames complete the matching pending entry,
    // and forward the SharedSessionOutbox to the supervisor so it
    // publishes the active up_tx on every successful dial.
    let (correlation, outbox, device_trust_sync) = match escalation_state {
        Some(state) => (
            Some(state.correlation),
            Some(state.outbox),
            Some(state.device_trust_sync),
        ),
        None => (None, None, None),
    };
    let mut local_dispatcher = LocalAxonSessionDispatcher::new();
    if let Some(correlation) = correlation {
        local_dispatcher = local_dispatcher.with_escalation_correlation(correlation);
    }
    local_dispatcher = local_dispatcher.with_local_runtime(Arc::clone(&local_runtime));
    local_dispatcher =
        local_dispatcher.with_ability_wire_registry(Arc::clone(&ability_wire_registry));
    // Cross-device origin-caller claims: warm the anchor from the hub
    // on a miss, over the SAME authenticated session channel the
    // paired-user sync and hot-agent advertising use (a device-local
    // resolve_key invoke would be answered from this daemon's own
    // anchor and can never learn a new key). The Arc is the daemon's
    // single DeviceTrustSync, built next to the escalation consumer
    // in `start_daemon_invocation_transport` and shared with the
    // service's self-targeted `<self>.invoke_remote` dispatch arm.
    if let Some(sync) = device_trust_sync {
        local_dispatcher = local_dispatcher.with_device_trust_sync(sync);
    }
    let dispatcher = Arc::new(local_dispatcher);
    let hub_endpoint_for_wait = hub_endpoint.clone();
    let caller_ura_for_wait = identity.caller_ura.clone();
    tokio::spawn(run_session_supervisor(
        hub_endpoint,
        identity.caller_ura,
        identity.signing_seed,
        hub_ca_pem_path,
        dispatcher,
        outbox,
        ability_descriptors,
        Some(initial_admission),
        Some(user_trust_sync),
        cancel_rx,
    ));
    spawn_initial_session_admission_observer(
        hub_endpoint_for_wait,
        caller_ura_for_wait,
        initial_admission_rx,
    );
    // The cancel sender travels back to the daemon's shutdown path
    // (was Box::leak'd, which made the supervisor's cancel branch dead
    // code — F-007). Dropping the returned handle at shutdown resolves
    // `cancel_rx` and the supervisor drains the live dial.
    Ok(SessionShutdown(Some(cancel_tx)))
}

fn device_owner_session_descriptors(
    owner_ura: &str,
    plugin_runtime_manager: Option<&crate::runtime::plugin_host::PluginRuntimeManager>,
) -> Vec<crate::runtime::ability_descriptor::AbilityDescriptor> {
    use crate::runtime::ability_descriptor::{AbilityDescriptor, Visibility};

    let mut descriptors = crate::runtime::agents::profiles::device::descriptors_for(owner_ura);
    let Some(manager) = plugin_runtime_manager else {
        return descriptors;
    };
    let Ok(state) = manager.state() else {
        return descriptors;
    };
    let Ok(plugin_descriptors) =
        crate::runtime::plugin_host::PluginDescriptorProjector::project(state.index())
    else {
        return descriptors;
    };

    descriptors.extend(plugin_descriptors.into_iter().filter_map(|plugin| {
        AbilityDescriptor::new(plugin.name, owner_ura, Visibility::Scoped)
            .ok()
            .map(|descriptor| {
                let descriptor = descriptor
                    .with_description(plugin.description)
                    .with_input_schema(plugin.input_schema)
                    .with_hints(plugin.hints)
                    .with_source("plugin:package");
                if let Some(output_schema) = plugin.output_schema {
                    descriptor.with_output_schema(output_schema)
                } else {
                    descriptor
                }
            })
    }));
    descriptors
}

fn spawn_initial_session_admission_observer(
    hub_endpoint: String,
    caller_ura: String,
    rx: tokio::sync::oneshot::Receiver<Result<(), String>>,
) {
    tokio::spawn(async move {
        match tokio::time::timeout(INITIAL_SESSION_ADMISSION_TIMEOUT, rx).await {
            Ok(Ok(Ok(()))) => {
                crate::op_event!(
                    component = session,
                    kind = initial_admission_observed,
                    hub_endpoint = hub_endpoint,
                    caller_ura = caller_ura,
                );
            }
            Ok(Ok(Err(reason))) => {
                crate::op_event!(
                    component = session,
                    kind = initial_admission_failed,
                    hub_endpoint = hub_endpoint,
                    caller_ura = caller_ura,
                    reason = reason,
                    message = "daemon remains up; session supervisor will reconnect with backoff",
                );
            }
            Ok(Err(_closed)) => {
                crate::op_event!(
                    component = session,
                    kind = initial_admission_probe_closed,
                    hub_endpoint = hub_endpoint,
                    caller_ura = caller_ura,
                    message = "session supervisor ended before reporting initial admission",
                );
            }
            Err(_elapsed) => {
                crate::op_event!(
                    component = session,
                    kind = initial_admission_pending,
                    hub_endpoint = hub_endpoint,
                    caller_ura = caller_ura,
                    timeout_ms = INITIAL_SESSION_ADMISSION_TIMEOUT.as_millis(),
                    message =
                        "daemon remains up; session supervisor is still attempting hub admission",
                );
            }
        }
    });
}

#[cfg(unix)]
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
                let uds_path_display = format!("{}", uds_path.display());
                let err_msg = format!("{err}");
                crate::op_event!(
                    component = daemon_invocation,
                    kind = uds_unlink_failed,
                    uds_path = uds_path_display,
                    error = err_msg,
                    message = "bind will likely fail",
                );
            }
        }
    }

    let listener = tokio::net::UnixListener::bind(&uds_path).map_err(|err| {
        anyhow::anyhow!(
            "failed to bind daemon Invocation UDS at {}: {err}",
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
            let uds_path_display = format!("{}", uds_path.display());
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = uds_chmod_failed,
                uds_path = uds_path_display,
                error = err_msg,
                message = "running with default umask perms",
            );
        }
    }

    let uds_path_display = format!("{}", uds_path.display());
    crate::op_event!(
        component = daemon_invocation,
        kind = grpc_invocation_server_listening,
        transport = "uds",
        uds_path = uds_path_display,
    );

    let incoming = UnixListenerStream::new(listener);
    tokio::spawn(async move {
        let result = Server::builder()
            // UDS is loopback-only; keepalive is purely defensive
            // for symmetry with the TCP+TLS listener below. Same
            // 5s ping cadence as the TCP+TLS server so behaviour
            // is uniform across listener types.
            .http2_keepalive_interval(Some(Duration::from_secs(5)))
            .http2_keepalive_timeout(Some(Duration::from_secs(10)))
            .tcp_keepalive(Some(Duration::from_secs(15)))
            .add_service(
                InvocationServer::new(service)
                    .max_decoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES)
                    .max_encoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES),
            )
            .serve_with_incoming(incoming)
            .await;
        if let Err(err) = result {
            let err_msg = format!("{err:#}");
            crate::op_event!(
                component = daemon_invocation,
                kind = grpc_server_exited_with_error,
                transport = "uds",
                error = err_msg,
            );
        }
    });

    Ok(())
}

#[cfg(windows)]
fn spawn_uds_listener(
    config: &DaemonConfig,
    service: DaemonInvocationService,
) -> anyhow::Result<()> {
    let pipe_name = config
        .uds_path()
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("daemon-config named-pipe path is not valid UTF-8"))?
        .to_string();
    let mut listener = PipeListener::bind(pipe_name.clone()).map_err(|err| {
        anyhow::anyhow!(
            "failed to bind daemon Invocation named pipe {}: {err}",
            pipe_name
        )
    })?;

    let pipe_name_log = pipe_name.clone();
    crate::op_event!(
        component = daemon_invocation,
        kind = grpc_invocation_server_listening,
        transport = "named_pipe",
        pipe_name = pipe_name_log,
    );

    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<NamedPipeGrpcIo>>(32);
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok(stream) => {
                    if tx.send(Ok(NamedPipeGrpcIo(stream))).await.is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(err)).await;
                    break;
                }
            }
        }
    });

    let incoming = ReceiverStream::new(rx);
    tokio::spawn(async move {
        let result = Server::builder()
            .http2_keepalive_interval(Some(Duration::from_secs(5)))
            .http2_keepalive_timeout(Some(Duration::from_secs(10)))
            .tcp_keepalive(Some(Duration::from_secs(15)))
            .add_service(
                InvocationServer::new(service)
                    .max_decoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES)
                    .max_encoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES),
            )
            .serve_with_incoming(incoming)
            .await;
        if let Err(err) = result {
            let err_msg = format!("{err:#}");
            crate::op_event!(
                component = daemon_invocation,
                kind = grpc_server_exited_with_error,
                transport = "named_pipe",
                error = err_msg,
            );
        }
    });

    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn spawn_uds_listener(
    _config: &DaemonConfig,
    _service: DaemonInvocationService,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "daemon Invocation local listener is unavailable on this platform until the local transport \
         backend lands"
    )
}

/// Spawn the hub-mode TCP+TLS gRPC listener (PR-10 commit 1/N).
/// `DaemonConfig` already enforces invariant 2 ("TCP requires
/// TLS"), so by the time we land here `tls_cert_pem` and
/// `tls_key_pem` are both `Some`. We fail boot — not silently
/// skip — if either file fails to load: PR-10 spec INV-1
/// (fail-closed) governs.
///
/// Cert/key are loaded once at boot. Rotation today requires a
/// daemon restart; an automated rotation surface (file watcher
/// + tonic `serve_with_shutdown` swap) is a future concern that
/// PR-10's runbook §"static cert lifecycle" covers as operator-
/// owned.
fn spawn_tcp_tls_listener(
    config: &DaemonConfig,
    listen_tcp: std::net::SocketAddr,
    service: DaemonInvocationService,
) -> anyhow::Result<()> {
    let cert_path = config
        .tls_cert_pem()
        .ok_or_else(|| anyhow::anyhow!("PR-10 invariant 1: TCP listener requires tls_cert_pem"))?;
    let key_path = config
        .tls_key_pem()
        .ok_or_else(|| anyhow::anyhow!("PR-10 invariant 1: TCP listener requires tls_key_pem"))?;

    let cert_pem = std::fs::read(cert_path).map_err(|err| {
        anyhow::anyhow!(
            "daemon-invocation: failed to read tls_cert_pem at {}: {err}",
            cert_path.display()
        )
    })?;
    let key_pem = std::fs::read(key_path).map_err(|err| {
        anyhow::anyhow!(
            "daemon-invocation: failed to read tls_key_pem at {}: {err}",
            key_path.display()
        )
    })?;

    let identity = Identity::from_pem(&cert_pem, &key_pem);
    let tls_config = ServerTlsConfig::new().identity(identity);

    let listen_tcp_display = format!("{listen_tcp}");
    let cert_path_display = format!("{}", cert_path.display());
    let key_path_display = format!("{}", key_path.display());
    crate::op_event!(
        component = daemon_invocation,
        kind = grpc_invocation_server_listening,
        transport = "tcp_tls",
        listen_tcp = listen_tcp_display,
        cert_pem = cert_path_display,
        key_pem = key_path_display,
    );

    // Production-WAN h2 hardening on the public TCP+TLS listener:
    // long-lived `<self>.session` bidi streams from devices behind
    // home/corporate NATs / hosting LBs need explicit keep-alive
    // PINGs or intermediaries silently drop the connection,
    // surfacing as "h2 protocol error: error reading a body" on
    // the device side and "session ended (StreamReset)" here.
    // 5s ping cadence: stays well under any NAT idle window
    // (~60s typical), surfaces dead streams in ~15s rather than
    // minutes, ~24 bytes/ping × 12/min ≈ negligible cost. Mirror
    // the device-client side at session_initiator.rs.
    let mut builder = match Server::builder().tls_config(tls_config) {
        Ok(b) => b
            .http2_keepalive_interval(Some(Duration::from_secs(5)))
            .http2_keepalive_timeout(Some(Duration::from_secs(10)))
            .tcp_keepalive(Some(Duration::from_secs(15))),
        Err(err) => {
            return Err(anyhow::anyhow!(
                "daemon-invocation: tls_config rejected by tonic: {err}"
            ));
        }
    };

    tokio::spawn(async move {
        let result = builder
            .add_service(
                InvocationServer::new(service)
                    .max_decoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES)
                    .max_encoding_message_size(MAX_INVOCATION_GRPC_MESSAGE_BYTES),
            )
            .serve(listen_tcp)
            .await;
        if let Err(err) = result {
            let err_msg = format!("{err:#}");
            crate::op_event!(
                component = daemon_invocation,
                kind = grpc_server_exited_with_error,
                transport = "tcp_tls",
                error = err_msg,
            );
        }
    });

    Ok(())
}

#[derive(Debug, Clone)]
struct DaemonIdentity {
    caller_ura: String,
    signing_seed: Option<SessionSigningSeed>,
}

/// Narrow read-projection of `~/.easynet/credentials.json` carrying
/// only the three fields the daemon needs to derive its caller URA +
/// signing seed.
///
/// MUST NOT use `#[serde(deny_unknown_fields)]`. The writer
/// (`persistence::config::Credentials`) owns the file and its field
/// set grows over time — `credential_token`, `hub_endpoint`,
/// `hub_api_base`, `username`, `hub_pubkey_b64`, `hub_tls_ca_pem_b64`
/// were all added after this projection. A strict reader would reject
/// the whole file the moment any such field appears, silently
/// collapsing `load_daemon_identity()` to `None` (the `.ok()?` at the
/// call site). That drops the daemon's device identity, so the
/// device-mode `<self>.session` supervisor never starts, the hub
/// never sees the device's presence, and the backend renders it
/// REMOVED. This is a projection, not a schema gate: tolerate unknown
/// fields and read only what we own.
///
/// One field IS still rejected: `tenant_id`. It is the retired alias
/// for `realm` (URA v4.1.4) — a credentials.json carrying it predates
/// the rename and would derive a daemon URA under the wrong namespace.
/// We reject it explicitly via a typed sentinel field rather than a
/// blanket `deny_unknown_fields`, so retirement enforcement survives
/// without re-introducing the field-drift regression above.
#[derive(Debug, serde::Deserialize)]
struct StoredDeviceIdentity {
    #[serde(default)]
    agent_ura: Option<String>,
    #[serde(default)]
    realm: Option<String>,
    #[serde(default)]
    node_id: Option<String>,
    /// Retired `realm` alias. Present only in pre-v4.1.4 files; its
    /// presence is a hard parse error (see `deserialize` below).
    #[serde(default, rename = "tenant_id")]
    _retired_tenant_id: Option<RejectedTenantId>,
}

/// Zero-sized marker whose `Deserialize` always errors, naming the
/// retired field. Used as the type of `StoredDeviceIdentity::tenant_id`
/// so any credentials.json still carrying `tenant_id` fails the parse
/// with a clear message, while every other unknown field is tolerated.
#[derive(Debug)]
struct RejectedTenantId;

impl<'de> serde::Deserialize<'de> for RejectedTenantId {
    fn deserialize<D>(_deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Err(serde::de::Error::custom(
            "credentials.json carries retired `tenant_id`; it was renamed to `realm` in URA \
             v4.1.4 — re-pair with `easynet join <token>` to rewrite the file",
        ))
    }
}

/// Resolve the daemon's caller URA plus the optional deterministic
/// signing seed from `~/.easynet/credentials.json`.
///
/// Contract:
/// - credentials must carry `(realm, node_id)`.
/// - `tenant_id` is a retired field and is rejected by serde.
/// - `agent_ura`, when present, is only a consistency checksum; it is
///   never a fallback identity.
/// - once we have the canonical `(realm, node_id)` pair, derive the same
///   deterministic Ed25519 seed the SDK uses for
///   `easynet:prv:reg:agent.<node>`.
fn load_daemon_identity() -> Option<DaemonIdentity> {
    let path = expand_home("~/.easynet/credentials.json");
    let raw = std::fs::read_to_string(&path).ok()?;
    let stored: StoredDeviceIdentity = serde_json::from_str(&raw).ok()?;
    daemon_identity_from_stored(&stored)
}

fn daemon_identity_from_stored(stored: &StoredDeviceIdentity) -> Option<DaemonIdentity> {
    let caller_ura = canonical_caller_ura_from_stored_identity(stored)?;

    let realm = stored
        .realm
        .as_deref()
        .map(str::trim)
        .filter(|realm| !realm.is_empty())
        .map(str::to_string);
    let node_id = stored
        .node_id
        .as_deref()
        .map(str::trim)
        .filter(|node| !node.is_empty())
        .map(str::to_string)
        .or_else(|| device_id_from_caller_ura(&caller_ura));

    // Phase 3D: prefer the keyring vault's seed when the operator
    // has opted in via EASYNET_KEYRING_PASSPHRASE. The vault's
    // primary_self for this device is `caller_ura`; the role
    // overlay also matches HubURI(realm) on the same host, so
    // backend (Go side, Phase 3D's Go reader) and daemon (Rust
    // side here) end up signing with the **same** Ed25519 seed.
    //
    // Misses (env unset, vault file missing, this URA not in
    // vault) silently fall through to the v4.1.4 deterministic
    // derive — operators who have not yet rolled their daemons
    // onto the keyring stay unaffected.
    let signing_seed = if let Some(seed) = try_load_daemon_seed_from_keyring(&caller_ura) {
        Some(seed)
    } else {
        match (realm.as_deref(), node_id.as_deref()) {
            (Some(realm), Some(node_id)) => {
                let subject_id = easynet_axon::invocation::private_agent_subject_id(node_id);
                Some(derive_subject_keypair(realm, &subject_id).0)
            }
            _ => None,
        }
    };

    Some(DaemonIdentity {
        caller_ura,
        signing_seed,
    })
}

/// Best-effort runtime-side self-identity bootstrap for daemon boots
/// that already have a live local runtime.
///
/// Why this exists:
/// - `easynet start` already bootstraps runtime key material before
///   republishing abilities.
/// - The heartbeat daemon also bootstraps before its first tick.
/// - `easynet-daemon` can, however, boot in shapes where neither of
///   those has fired yet while local CLI surfaces already route
///   through `BridgeAbilityInvoker` (`node.describe` ->
///   `federation.resolve`, `node.list`, etc.).
///
/// In that window the runtime rejects signed federation reads with
/// `AXON_EASYNET_SUBJECT_KEY_UNREGISTERED`. Bootstrapping here closes
/// the gap for any daemon boot that can already see a live runtime.
///
/// Best-effort by contract:
/// - no runtime state file -> silent skip (standalone daemon harnesses)
/// - runtime down / bridge connect fail -> log + continue
/// - bootstrap reject -> log + continue
///
/// The call is idempotent. If `easynet start` or the heartbeat daemon
/// already registered the keys, the runtime simply keeps the prior
/// entries and startup proceeds unchanged.
fn maybe_bootstrap_runtime_self_identity(identity: &DaemonIdentity) {
    let Some(realm) = realm_from_agent_ura(&identity.caller_ura) else {
        return;
    };
    let Some(node_id) = device_id_from_caller_ura(&identity.caller_ura) else {
        return;
    };

    let state = match crate::persistence::config::load() {
        Ok(state) => state,
        Err(_) => return,
    };
    if matches!(
        state.runtime_kind,
        crate::persistence::config::RuntimeKind::DaemonOnly
    ) {
        return;
    }
    let bridge = match state.connect_bridge() {
        Ok(bridge) => bridge,
        Err(err) => {
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = runtime_self_bootstrap_skipped,
                node_id = node_id,
                reason = "connect_local_runtime_bridge_failed",
                error = err_msg,
            );
            return;
        }
    };
    let invoker = crate::runtime::advertise::BridgeAbilityInvoker::with_caller_ura(
        &bridge,
        identity.caller_ura.clone(),
    );
    match crate::runtime::publish::bootstrap_self_identity_via_runtime(
        &invoker, &realm, &realm, &node_id,
    )
    .result
    {
        Ok(()) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = runtime_self_bootstrap_registered,
                node_id = node_id,
            );
        }
        Err(msg) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = runtime_self_bootstrap_failed,
                node_id = node_id,
                error = msg,
            );
        }
    }
}

fn try_load_daemon_seed_from_keyring(self_ura: &str) -> Option<[u8; 32]> {
    use crate::services::keyring::{MasterKeySource, Vault, VaultError};

    std::env::var("EASYNET_KEYRING_PASSPHRASE")
        .ok()
        .filter(|v| !v.is_empty())?;
    let path = if let Ok(p) = std::env::var("EASYNET_KEYRING_VAULT_PATH") {
        std::path::PathBuf::from(p)
    } else {
        expand_home(&format!(
            "~/{}",
            crate::services::keyring::DEFAULT_VAULT_REL
        ))
    };
    if !path.exists() {
        return None;
    }
    let source = match MasterKeySource::from_env() {
        Ok(s) => s,
        Err(err) => {
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = keyring_master_key_source_failed,
                error = err_msg,
            );
            return None;
        }
    };
    let vault = match Vault::open(&path, &source) {
        Ok(v) => v,
        Err(VaultError::NotFound(_)) => return None,
        Err(err) => {
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = keyring_open_failed,
                error = err_msg,
            );
            return None;
        }
    };
    match vault.export_seed(self_ura) {
        Ok(seed) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = keyring_daemon_seed_resolved,
                self_ura = self_ura,
            );
            Some(seed)
        }
        Err(VaultError::NotFound(_)) => None,
        Err(err) => {
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = keyring_export_seed_failed,
                self_ura = self_ura,
                error = err_msg,
            );
            None
        }
    }
}

fn canonical_caller_ura_from_stored_identity(stored: &StoredDeviceIdentity) -> Option<String> {
    let realm = stored
        .realm
        .as_deref()
        .map(str::trim)
        .filter(|realm| !realm.is_empty());
    let node_id = stored
        .node_id
        .as_deref()
        .map(str::trim)
        .filter(|node| !node.is_empty());

    let (Some(realm), Some(node_id)) = (realm, node_id) else {
        return None;
    };

    let expected = crate::ura::device_ura(realm, node_id);
    if let Some(agent_ura) = stored
        .agent_ura
        .as_deref()
        .map(str::trim)
        .filter(|ura| !ura.is_empty())
    {
        if agent_ura != expected {
            return None;
        }
    }

    Some(expected)
}

// URA v4.1.5: strict parsing via crate::ura::parse_ura per memory
// `feedback_no_legacy_ura.md`. The daemon's stored caller URA in
// v4.1.5 is always `easynet:///r/<realm>/device/<device-uuid>`
// (device-mode CLI's self-identity URA), so we only need to match
// that one shape.
//
// Legacy `r/{prv,org}/reg/agent.<id>?tenant_id=<t>` (URA v1) and
// `agent/<bare-id>` (URA v2 transitional) shapes are rejected —
// pre-v4.1.5 credential files cannot bootstrap signing seeds; users
// must `easynet device join` again to mint a v4.1.5 credential.
// Returning `None` triggers the parent code's "skip signing seed"
// branch (CLI starts unsigned, harmless in dev).

fn realm_from_agent_ura(ura: &str) -> Option<String> {
    let parsed = crate::ura::parse_ura(ura).ok()?;
    if parsed.realm.is_empty() {
        None
    } else {
        Some(parsed.realm)
    }
}

fn device_id_from_caller_ura(ura: &str) -> Option<String> {
    let parsed = crate::ura::parse_ura(ura).ok()?;
    // Only Device-kind URAs carry a device_id field; other kinds
    // leave it empty. Empty == not a device URA.
    parsed.device_id().map(str::to_string)
}

/// Resolve the realm-trust file path. Resolution order:
///
/// 1. `EASYNET_REALM_TRUST_PATH` env override (PR-7 commit 7/N
///    test-redirect seam, also used by docker-e2e fixtures).
/// 2. `/etc/easynet/realm-trust.toml` — production / packaged
///    deploys where the file is admin-owned. When this file
///    exists AND is non-empty we always prefer it.
/// 3. `$HOME/.easynet/realm-trust.toml` — fallback for host-mode
///    dev / unprivileged installs. `easynet device join` writes
///    the device + local-hub trust entries here at pairing time
///    (see `auto_wire_self_realm_trust_from_credentials`); the
///    daemon picks them up here without needing `sudo` to write
///    `/etc/easynet/`.
///
/// The home-mode fallback closes the operator-visible "I joined,
/// the daemon's trust file is empty, admission rejects everything"
/// failure mode that single-user host-mode installs hit when
/// neither root nor an env override is in play.
pub(crate) fn trust_anchor_path_from_env_or_default() -> PathBuf {
    if let Some(override_path) = std::env::var_os("EASYNET_REALM_TRUST_PATH") {
        return expand_home(override_path.to_string_lossy().as_ref());
    }
    let etc = expand_home(DEFAULT_REALM_TRUST_PATH);
    if let Ok(meta) = std::fs::metadata(&etc) {
        if meta.is_file() && meta.len() > 0 {
            return etc;
        }
    }
    expand_home("~/.easynet/realm-trust.toml")
}

fn load_trust_anchor_from(path: &Path) -> RealmTrustAnchor {
    match RealmTrustAnchor::load_or_empty(path) {
        Ok(anchor) => {
            let path_display = format!("{}", path.display());
            if anchor.is_empty() {
                crate::op_event!(
                    component = daemon_invocation,
                    kind = realm_trust_anchor_empty,
                    path = path_display,
                    message = "admission gate will reject every external caller until PR-7 pairing flow populates it",
                );
            } else {
                let entry_count = anchor.len();
                crate::op_event!(
                    component = daemon_invocation,
                    kind = realm_trust_anchor_loaded,
                    path = path_display,
                    entries = entry_count,
                );
            }
            anchor
        }
        Err(err) => {
            let path_display = format!("{}", path.display());
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = realm_trust_anchor_load_failed,
                path = path_display,
                error = err_msg,
                message = "proceeding with empty trust set",
            );
            RealmTrustAnchor::default()
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BackendIdentityRecord {
    private_key_seed_hex: String,
    #[serde(default)]
    agent_ura: String,
    #[serde(default, rename = "created_at_unix_ms")]
    _created_at_unix_ms: Option<u64>,
}

fn upsert_backend_identity_from_disk(
    realm: &str,
    trust_anchor_path: &Path,
    mut anchor: RealmTrustAnchor,
) -> RealmTrustAnchor {
    let Some(record) = read_backend_identity_record(realm) else {
        return anchor;
    };
    let expected_ura = crate::ura::hub_ura(realm);
    if !record.agent_ura.trim().is_empty() && record.agent_ura != expected_ura {
        crate::op_event!(
            component = daemon_invocation,
            kind = backend_identity_trust_upsert_skipped,
            expected_ura = expected_ura,
            actual_ura = record.agent_ura,
            message = "backend identity file does not match daemon realm",
        );
        return anchor;
    }
    let seed = match decode_backend_identity_seed(&record.private_key_seed_hex) {
        Ok(seed) => seed,
        Err(err) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = backend_identity_trust_upsert_failed,
                error = err,
                message = "backend identity seed is not usable",
            );
            return anchor;
        }
    };
    let signing_key = SigningKey::from_bytes(&seed);
    let entry = TrustedAgent {
        agent_ura: expected_ura.clone(),
        public_key_b64: BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes()),
        role: TrustedAgentRole::Backend,
        added_at_unix_ms: now_unix_ms(),
        origin_realm: None,
        hub_endpoint: None,
        tls_ca_pem_path: None,
    };
    if let Err(err) = anchor.upsert_singleton_agent(entry) {
        crate::op_event!(
            component = daemon_invocation,
            kind = backend_identity_trust_upsert_failed,
            error = format!("{err}"),
            message = "failed to merge backend identity into trust anchor",
        );
        return anchor;
    }
    if let Err(err) = anchor.save(trust_anchor_path) {
        crate::op_event!(
            component = daemon_invocation,
            kind = backend_identity_trust_save_failed,
            path = format!("{}", trust_anchor_path.display()),
            error = format!("{err}"),
            message = "using backend identity in memory; disk trust anchor was not updated",
        );
    } else {
        crate::op_event!(
            component = daemon_invocation,
            kind = backend_identity_trust_upserted,
            path = format!("{}", trust_anchor_path.display()),
            agent_ura = expected_ura,
            message = "backend identity public key is present in trust anchor",
        );
    }
    anchor
}

fn read_backend_identity_record(realm: &str) -> Option<BackendIdentityRecord> {
    let home = std::env::var_os("HOME")?;
    let path = Path::new(&home)
        .join(".easynet-hub")
        .join(realm)
        .join("identity.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = backend_identity_trust_upsert_failed,
                path = format!("{}", path.display()),
                error = format!("{err}"),
                message = "failed to read backend identity file",
            );
            return None;
        }
    };
    match serde_json::from_str(&raw) {
        Ok(record) => Some(record),
        Err(err) => {
            crate::op_event!(
                component = daemon_invocation,
                kind = backend_identity_trust_upsert_failed,
                path = format!("{}", path.display()),
                error = format!("{err}"),
                message = "failed to parse backend identity file",
            );
            None
        }
    }
}

fn decode_backend_identity_seed(raw: &str) -> Result<[u8; 32], String> {
    let bytes = hex::decode(raw.trim()).map_err(|err| format!("seed hex decode failed: {err}"))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("seed must decode to 32 bytes, got {}", bytes.len()))
}

fn now_unix_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn reload_trust_anchor_cell_from(
    path: &Path,
    trust_anchor_cell: &SharedTrustAnchor,
) -> anyhow::Result<usize> {
    let next = RealmTrustAnchor::load_or_empty(path)
        .map_err(|err| anyhow::anyhow!("load trust anchor from {}: {err}", path.display()))?;
    let len = next.len();
    trust_anchor_cell.replace(Arc::new(next));
    Ok(len)
}

/// Demo-only presence seed. Compiled into the daemon binary
/// only under `--features demo-fixture`; the production build
/// emits a no-op no matter what `EASYNET_DEMO_PRESENCE_SEED`
/// holds. The seed registers a no-op `DispatchSender` under
/// each comma-separated URA in the env var so cross-hub
/// `forward_invoke` targeting that URA survives the presence
/// registry lookup gate without a real device pair flow.
///
/// Channel capacity 8 mirrors the `<self>.session` accept
/// path. A drain task discards every queued frame so the
/// channel never reports full or closed; the demo's
/// transport-plane proof terminates at "frame queued for
/// delivery". Real ability responses flow through
/// `dispatch_federation_*` handlers that do not consult the
/// presence frame queue.
#[cfg(feature = "demo-fixture")]
fn maybe_seed_demo_presence(presence: &Arc<PresenceRegistry>) {
    let Ok(seed_value) = std::env::var("EASYNET_DEMO_PRESENCE_SEED") else {
        return;
    };
    for seed_ura in seed_value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<
            Result<crate::services::presence_registry::DispatchFrame, tonic::Status>,
        >(8);
        presence.insert(seed_ura.to_string(), tx);
        tokio::spawn(async move {
            while rx.recv().await.is_some() {
                // discard
            }
        });
        crate::op_event!(
            component = daemon_invocation,
            kind = demo_presence_seed_registered,
            seed_ura = seed_ura,
            message = "test fixture; do not use in production",
        );
    }
}

#[cfg(not(feature = "demo-fixture"))]
fn maybe_seed_demo_presence(_presence: &Arc<PresenceRegistry>) {
    // Production build: env var is ignored. If the operator
    // set it expecting the demo behaviour, the missing log line
    // is the signal — re-build with `--features demo-fixture`.
}

/// Unified SIGHUP-driven reload coordinator. Replaces the previous
/// three independent SIGHUP listeners (trust anchor, federated_peers,
/// federated-key cache flush) with a single task that processes all
/// three reloads sequentially per signal.
///
/// **Why unified.** Three independent listeners on the same signal
/// fire in non-deterministic order. Operator could observe a window
/// where trust anchor is reloaded but federated_peers is still stale
/// (or vice versa), and a federated cross-realm admission firing
/// inside that window would resolve against an inconsistent
/// snapshot. Unifying into one task gives "all-or-nothing per
/// signal" atomicity at the SIGHUP boundary.
///
/// **Order matters within a signal**. Trust anchor first (operator's
/// most-likely-edited file), then daemon-config (federated_peers
/// table + quota policy), then key-cache flush (so the next
/// admission re-resolves against the new anchor + peers).
///
/// Each reload step is independently fault-tolerant: a TOML parse
/// error in one step logs the error and continues to the next step
/// rather than aborting the whole signal. The cell keeps its
/// last-known-good value when a step fails.
#[cfg(unix)]
fn spawn_unified_sighup_reload_task(
    trust_anchor_path: PathBuf,
    trust_anchor_cell: SharedTrustAnchor,
    daemon_config_path: PathBuf,
    federated_peers_cell: crate::services::federated_peers_cell::SharedFederatedPeers,
    quota_gate: SharedUsageQuotaGate,
    federated_key_cache: crate::services::invocation_transport::federated_key_resolver::SharedFederatedKeyCache,
) {
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};

        let mut sighup = match signal(SignalKind::hangup()) {
            Ok(stream) => stream,
            Err(err) => {
                let err_msg = format!("{err}");
                crate::op_event!(
                    component = daemon_invocation,
                    kind = sighup_reload_handler_install_failed,
                    error = err_msg,
                );
                return;
            }
        };

        while sighup.recv().await.is_some() {
            // Step 1: trust anchor.
            let trust_anchor_path_display = format!("{}", trust_anchor_path.display());
            match reload_trust_anchor_cell_from(&trust_anchor_path, &trust_anchor_cell) {
                Ok(len) => {
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = sighup_trust_anchor_reloaded,
                        step = "1/3",
                        path = trust_anchor_path_display,
                        entries = len,
                    );
                }
                Err(err) => {
                    let err_msg = format!("{err}");
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = sighup_trust_anchor_reload_failed,
                        step = "1/3",
                        path = trust_anchor_path_display,
                        error = err_msg,
                        message = "keeping previous trust set",
                    );
                }
            }

            // Step 2: daemon-config federated_peers + quota.
            let daemon_config_path_display = format!("{}", daemon_config_path.display());
            match reload_daemon_config_cells_from(
                &daemon_config_path,
                &federated_peers_cell,
                &quota_gate,
            ) {
                Ok(snapshot) => {
                    let quota_state = if snapshot.quota_configured {
                        "configured"
                    } else {
                        "disabled"
                    };
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = sighup_daemon_config_reloaded,
                        step = "2/3",
                        path = daemon_config_path_display,
                        federated_peers = snapshot.federated_peers_len,
                        quota = quota_state,
                    );
                }
                Err(err) => {
                    let err_msg = format!("{err}");
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = sighup_daemon_config_reload_failed,
                        step = "2/3",
                        path = daemon_config_path_display,
                        error = err_msg,
                        message = "keeping previous federated_peers map and quota policy",
                    );
                }
            }

            // Step 3: flush federated-key TTL cache so the next
            // admission re-resolves cross-realm pubkeys against
            // the freshly-loaded trust anchor + peer map.
            federated_key_cache.flush();
            crate::op_event!(
                component = daemon_invocation,
                kind = sighup_federated_key_cache_flushed,
                step = "3/3",
                message = "cross-realm pubkeys will re-resolve on next admission",
            );
        }
    });
}

#[cfg(not(unix))]
fn spawn_unified_sighup_reload_task(
    _trust_anchor_path: PathBuf,
    _trust_anchor_cell: SharedTrustAnchor,
    _daemon_config_path: PathBuf,
    _federated_peers_cell: crate::services::federated_peers_cell::SharedFederatedPeers,
    _quota_gate: SharedUsageQuotaGate,
    _federated_key_cache: crate::services::invocation_transport::federated_key_resolver::SharedFederatedKeyCache,
) {
}

struct ReloadedDaemonConfigCells {
    federated_peers_len: usize,
    quota_configured: bool,
}

/// Re-parse daemon-config TOML at `path` and republish all live cells
/// that are intentionally SIGHUP-managed from that file.
fn reload_daemon_config_cells_from(
    path: &Path,
    federated_peers_cell: &crate::services::federated_peers_cell::SharedFederatedPeers,
    quota_gate: &SharedUsageQuotaGate,
) -> anyhow::Result<ReloadedDaemonConfigCells> {
    let next_config = DaemonConfig::load(path)
        .map_err(|err| anyhow::anyhow!("reload daemon-config from {}: {err}", path.display()))?;
    let next_peers = next_config.federated_peers().clone();
    let len = next_peers.len();
    federated_peers_cell.replace(next_peers);

    let next_quota = next_config.quota().cloned();
    let quota_configured = next_quota.is_some();
    quota_gate.replace_policy(next_quota);

    Ok(ReloadedDaemonConfigCells {
        federated_peers_len: len,
        quota_configured,
    })
}

/// **PR-N3 commit N3-3.1**. Spawn the cross-realm directory
/// poll task. Kept available for a future "operator manual
/// poll" CLI surface, but no longer wired into boot — the
/// streaming supervisor handles every peer in production.
///
/// Calls `federation_directory::poll_once` every 5s against
/// every entry in the live `SharedFederatedPeers` cell
/// snapshot. The task reads the cell each round, so a SIGHUP-
/// driven federated_peers reload is naturally picked up — peers
/// added show up in the next poll, peers removed are dropped on
/// the round after the SIGHUP.
///
/// Per-peer failures (dial dropped, parse error) surface as
/// stderr trace; the task does not retry mid-round, just waits
/// for the next interval.
#[allow(dead_code)]
fn spawn_federated_directory_poll_task(
    federation_client: Arc<dyn crate::services::federation_client::FederationClient>,
    federated_peers_cell: crate::services::federated_peers_cell::SharedFederatedPeers,
    daemon_ura: Option<String>,
    federated_directory_cell: crate::services::federation_directory::SharedFederatedDirectoryView,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        // Skip the immediate-fire on first tick so the daemon
        // doesn't hammer peers during boot before they're up.
        // The first real poll fires 5s after spawn.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await;
        loop {
            interval.tick().await;
            let peers = federated_peers_cell.snapshot();
            if peers.is_empty() {
                continue;
            }
            let outcome = crate::services::federation_directory::poll_once(
                federation_client.as_ref(),
                &peers,
                daemon_ura.as_deref(),
                &federated_directory_cell,
            )
            .await;
            for (realm, err) in &outcome.failed_peers {
                // `realm: &String`, `err: impl Display`. Pass through
                // verbatim so the op-event field renders as
                // `peer_realm=tenant-a` (auto-quoted only if whitespace)
                // instead of Rust's `"tenant-a"` Debug literal.
                let err_msg = err.to_string();
                crate::op_event!(
                    component = federation_directory,
                    kind = poll_peer_failed,
                    peer_realm = realm,
                    error = err_msg,
                );
            }
        }
    });
}

/// **PR-N3 commit N3-streaming-4**. Spawn a watcher task that
/// observes the live `SharedFederatedPeers` cell and maintains
/// one streaming subscriber task per peer. New peers (added
/// via SIGHUP-driven federated_peers reload) get a fresh
/// supervisor; removed peers get their supervisor cancelled
/// via a `oneshot::Sender`.
///
/// Cadence: the watcher rescans the cell every 2 seconds. A
/// finer cadence wastes CPU on the snapshot clone; a coarser
/// one delays peer add/drop visibility past the spec §八 (4)
/// "appears within ~5s" budget. 2s is the operator-visible
/// floor.
///
/// Per-peer supervisor lives until either (a) the cancel
/// signal fires (peer removed) or (b) the daemon shuts down
/// and tokio drops the runtime (the supervisor's awaits
/// abort).
fn spawn_federated_directory_streaming_supervisor(
    federation_client: Arc<dyn crate::services::federation_client::FederationClient>,
    federated_peers_cell: crate::services::federated_peers_cell::SharedFederatedPeers,
    federated_directory_cell: crate::services::federation_directory::SharedFederatedDirectoryView,
    caller_ura: String,
) {
    tokio::spawn(async move {
        // peer_realm -> oneshot::Sender that cancels the
        // supervisor when a peer is removed. The watcher
        // reconciles this map against the cell every tick
        // via `reconcile_streaming_supervisors` (PR-N3
        // N3-streaming-9), which extracts the diff logic so
        // it stays unit-testable.
        let mut active: std::collections::BTreeMap<String, tokio::sync::oneshot::Sender<()>> =
            std::collections::BTreeMap::new();
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let snapshot = federated_peers_cell.snapshot();

            let federation_client_outer = Arc::clone(&federation_client);
            let directory_cell_outer = federated_directory_cell.clone();
            let (spawned, cancelled) =
                crate::services::federation_directory::reconcile_streaming_supervisors(
                    &snapshot,
                    &mut active,
                    |peer_realm, peer_hub_endpoint| {
                        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
                        let realm_owned = peer_realm.to_string();
                        let uri_owned = peer_hub_endpoint.to_string();
                        let caller_owned = caller_ura.clone();
                        let client_clone = Arc::clone(&federation_client_outer);
                        let cell_clone = directory_cell_outer.clone();
                        tokio::spawn(async move {
                            crate::services::federation_directory::run_per_peer_supervisor(
                                realm_owned,
                                uri_owned,
                                caller_owned,
                                client_clone,
                                cell_clone,
                                cancel_rx,
                            )
                            .await;
                        });
                        cancel_tx
                    },
                );
            for realm in spawned {
                crate::op_event!(
                    component = federation_directory,
                    kind = streaming_supervisor_spawned,
                    peer_realm = realm,
                );
            }
            for realm in cancelled {
                crate::op_event!(
                    component = federation_directory,
                    kind = streaming_supervisor_cancelled,
                    peer_realm = realm,
                    reason = "no_longer_in_federated_peers",
                );
            }
        }
    });
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
    use super::SessionShutdown;

    #[tokio::test]
    async fn session_shutdown_drop_resolves_the_cancel_receiver() {
        // F-007 regression: the handle must carry a live sender whose
        // drop resolves the supervisor's cancel branch (the old
        // Box::leak made that branch dead in production).
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let handle = SessionShutdown(Some(tx));
        drop(handle);
        // Drop sends an explicit `()` cancel — the supervisor's
        // `_ = &mut cancel => return` arm fires on the received value.
        // Before F-007 the sender was Box::leak'd, so this never
        // arrived and the arm was dead code in production.
        assert_eq!(rx.await, Ok(()), "drop delivers the cancel signal");
    }

    #[test]
    fn session_shutdown_none_is_inert() {
        // hub/unconfigured mode: dropping a none handle is a no-op.
        drop(SessionShutdown::none());
    }

    use super::*;

    #[test]
    fn expand_home_with_tilde_uses_home_env() {
        // HomeGuard serialises HOME mutation across the suite.
        let _g = crate::facade::cli::test_support::HomeGuard::new();
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

    #[test]
    #[cfg(feature = "remote-desktop")]
    fn device_owner_session_descriptors_include_builtin_plugin_abilities() {
        let index = crate::runtime::plugin_host::PluginPackageIndex::builtin()
            .expect("builtin plugin index loads");
        let state = crate::runtime::plugin_host::PluginRuntimeState::from_index_with_planner(
            index,
            crate::runtime::plugin_host::PluginLoadPlanner::current_without_env_gates(),
        );
        let manager = crate::runtime::plugin_host::PluginRuntimeManager::from_state(state);
        let descriptors =
            device_owner_session_descriptors("easynet:///r/acme/device/dev-1", Some(&manager));
        let names = descriptors
            .iter()
            .map(|descriptor| descriptor.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        assert!(
            names.contains("remote_desktop.refresh_lease"),
            "device owner projection must publish remote desktop lease refresh; got {names:?}"
        );
        let watch_events = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "remote_desktop.watch_events")
            .expect("watch_events descriptor");
        assert!(
            watch_events.hints.streaming_only,
            "device owner projection must preserve plugin stream hints"
        );
        let attach = descriptors
            .iter()
            .find(|descriptor| descriptor.name == "remote_desktop.attach")
            .expect("attach descriptor");
        assert!(
            attach.hints.bidi_only,
            "device owner projection must preserve plugin bidi hints"
        );
    }

    #[test]
    fn canonical_caller_ura_accepts_matching_agent_ura_checksum() {
        let stored = StoredDeviceIdentity {
            agent_ura: Some(crate::ura::device_ura("realm-a", "device-123")),
            realm: Some("realm-a".to_string()),
            node_id: Some("device-123".to_string()),
            _retired_tenant_id: None,
        };
        assert_eq!(
            canonical_caller_ura_from_stored_identity(&stored).as_deref(),
            Some("easynet:///r/realm-a/device/device-123"),
        );
    }

    #[test]
    fn canonical_caller_ura_rejects_mismatched_agent_ura_checksum() {
        let stored = StoredDeviceIdentity {
            agent_ura: Some("easynet:///r/legacy/agent/old-node".to_string()),
            realm: Some("realm-a".to_string()),
            node_id: Some("device-123".to_string()),
            _retired_tenant_id: None,
        };
        assert_eq!(canonical_caller_ura_from_stored_identity(&stored), None);
    }

    #[test]
    fn daemon_identity_rejects_retired_tenant_id_credentials() {
        let raw = r#"{
  "realm": "realm-a",
  "tenant_id": "tenant-a",
  "node_id": "device-123"
}"#;
        let err = serde_json::from_str::<StoredDeviceIdentity>(raw)
            .expect_err("retired tenant_id must fail schema parse");
        assert!(
            err.to_string().contains("tenant_id"),
            "error must name retired field: {err}"
        );
    }

    #[test]
    fn daemon_identity_parses_full_modern_credentials_json() {
        // Regression: a credentials.json carrying the FULL modern
        // field set (everything `persistence::config::Credentials`
        // writes after v4.1.5 cold-start: credential_token,
        // hub_endpoint, deploy_signature, hub_api_base, username,
        // hub_pubkey_b64, hub_tls_ca_pem_b64) must still parse into a
        // device identity. A `deny_unknown_fields` projection silently
        // collapsed this to `None`, which stopped the `<self>.session`
        // supervisor and rendered the device REMOVED on the hub.
        let raw = r#"{
  "node_id": "01a5b007-f9c3-41f9-aa6f-7531267651bc",
  "credential_token": "2929dad1f03f",
  "hub_endpoint": "https://127.0.0.1:50443",
  "realm": "localhost",
  "deploy_signature": "",
  "hub_api_base": "http://127.0.0.1:8080",
  "username": "dev",
  "hub_pubkey_b64": "6Tp8qzyMm2",
  "hub_tls_ca_pem_b64": "LS0tLS1CRUdJ"
}"#;
        let stored = serde_json::from_str::<StoredDeviceIdentity>(raw)
            .expect("modern credentials.json must parse despite extra fields");
        let identity = daemon_identity_from_stored(&stored).expect("must derive a device identity");
        assert_eq!(
            identity.caller_ura,
            "easynet:///r/localhost/device/01a5b007-f9c3-41f9-aa6f-7531267651bc",
        );
        assert!(
            identity.signing_seed.is_some(),
            "device identity must carry a signing seed so `<self>.session` can dial the hub"
        );
    }

    #[test]
    fn daemon_identity_from_stored_accepts_realm_only_credentials() {
        let stored = StoredDeviceIdentity {
            agent_ura: None,
            realm: Some("realm-a".to_string()),
            node_id: Some("device-123".to_string()),
            _retired_tenant_id: None,
        };
        let identity = daemon_identity_from_stored(&stored).expect("identity");
        assert_eq!(
            identity.caller_ura,
            "easynet:///r/realm-a/device/device-123"
        );
        assert!(
            identity.signing_seed.is_some(),
            "realm+node credentials must derive a signing seed"
        );
    }

    #[test]
    fn daemon_identity_from_stored_rejects_agent_ura_fallback_when_fields_missing() {
        let stored = StoredDeviceIdentity {
            agent_ura: Some("easynet:///r/realm-a/agent/legacy-node".to_string()),
            realm: None,
            node_id: None,
            _retired_tenant_id: None,
        };
        assert!(
            daemon_identity_from_stored(&stored).is_none(),
            "agent_ura is no longer a fallback daemon identity"
        );
    }

    #[test]
    fn daemon_identity_prefers_keyring_seed_over_deterministic_derive() {
        use crate::services::keyring::{MasterKeySource, Vault};
        use ed25519_dalek::SigningKey;
        use std::sync::Mutex;
        // Serialise against other env-mutating tests in this file
        // (HOME, EASYNET_KEYRING_*). They all set_var at top of body
        // without a guard; this guard ensures no two of them race.
        static ENV_GUARD: Mutex<()> = Mutex::new(());
        let _guard = ENV_GUARD.lock().unwrap();

        let temp = tempfile::tempdir().expect("tempdir");
        let vault_path = temp.path().join("keyring.enc");
        let pass = "phase3d-daemon-boot-test";

        // Seed 0xAA repeated 32 times — distinguishable from
        // anything `derive_subject_keypair` would produce, so we
        // can pin the test on "this seed came from the vault".
        let seed = [0xAAu8; 32];

        let primary = "easynet:///r/host-test/device/dev-uuid";
        let hub_overlay = crate::ura::hub_ura("host-test");

        let source = MasterKeySource::Explicit(pass.to_string());
        let mut vault = Vault::init(&vault_path, &source).expect("init vault");
        vault
            .put(primary, vec![hub_overlay.to_string()], hex::encode(seed))
            .expect("put");
        vault.seal().expect("seal");

        std::env::set_var("EASYNET_KEYRING_PASSPHRASE", pass);
        std::env::set_var("EASYNET_KEYRING_VAULT_PATH", &vault_path);

        let stored = StoredDeviceIdentity {
            agent_ura: None,
            realm: Some("host-test".to_string()),
            node_id: Some("dev-uuid".to_string()),
            _retired_tenant_id: None,
        };
        let identity = daemon_identity_from_stored(&stored).expect("identity");

        std::env::remove_var("EASYNET_KEYRING_PASSPHRASE");
        std::env::remove_var("EASYNET_KEYRING_VAULT_PATH");

        assert_eq!(identity.caller_ura, primary);
        let got = identity.signing_seed.expect("seed");
        assert_eq!(
            got, seed,
            "daemon must use the vault's seed, not the deterministic derive"
        );

        // Sanity: the resulting keypair is the SAME one as what the
        // backend (Phase 3D Go reader) will pull from this vault
        // for the hub overlay — that's the load-bearing v4.1.5
        // host-mode invariant.
        let _signer = SigningKey::from_bytes(&got);
    }

    #[test]
    fn daemon_identity_falls_back_when_keyring_env_unset() {
        use std::sync::Mutex;
        static ENV_GUARD: Mutex<()> = Mutex::new(());
        let _guard = ENV_GUARD.lock().unwrap();
        std::env::remove_var("EASYNET_KEYRING_PASSPHRASE");
        std::env::remove_var("EASYNET_KEYRING_VAULT_PATH");

        let stored = StoredDeviceIdentity {
            agent_ura: None,
            realm: Some("realm-no-vault".to_string()),
            node_id: Some("dev-uuid".to_string()),
            _retired_tenant_id: None,
        };
        let identity = daemon_identity_from_stored(&stored).expect("identity");
        assert!(
            identity.signing_seed.is_some(),
            "deterministic derive must still work when the keyring is not opted into"
        );
    }

    #[test]
    fn backend_identity_upsert_replaces_stale_trust_anchor_key() {
        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());

        let realm = "realm-upsert";
        let identity_dir = temp.path().join(".easynet-hub").join(realm);
        std::fs::create_dir_all(&identity_dir).expect("identity dir");
        let new_seed = [0x42u8; 32];
        std::fs::write(
            identity_dir.join("identity.json"),
            serde_json::json!({
                "private_key_seed_hex": hex::encode(new_seed),
                "agent_ura": crate::ura::hub_ura(realm),
                "created_at_unix_ms": 1_714_492_800_000i64,
            })
            .to_string(),
        )
        .expect("identity file");

        let old_key = SigningKey::from_bytes(&[0x41u8; 32]);
        let old_pub = BASE64_STANDARD.encode(old_key.verifying_key().to_bytes());
        let trust_path = temp.path().join("realm-trust.toml");
        let stale = RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: crate::ura::hub_ura(realm),
            public_key_b64: old_pub,
            role: TrustedAgentRole::Backend,
            added_at_unix_ms: 1,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }])
        .expect("stale anchor");

        let updated = upsert_backend_identity_from_disk(realm, &trust_path, stale);
        let want_pub =
            BASE64_STANDARD.encode(SigningKey::from_bytes(&new_seed).verifying_key().to_bytes());
        assert_eq!(
            updated
                .lookup(&crate::ura::hub_ura(realm))
                .expect("backend entry")
                .public_key_b64,
            want_pub
        );
        let from_disk = RealmTrustAnchor::try_load_strict(&trust_path).expect("disk anchor");
        assert_eq!(
            from_disk
                .lookup(&crate::ura::hub_ura(realm))
                .expect("backend entry on disk")
                .public_key_b64,
            want_pub
        );
    }

    #[test]
    fn backend_identity_reader_rejects_retired_agent_uri_alias() {
        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());

        let realm = "realm-retired-agent-uri";
        let identity_dir = temp.path().join(".easynet-hub").join(realm);
        std::fs::create_dir_all(&identity_dir).expect("identity dir");
        std::fs::write(
            identity_dir.join("identity.json"),
            serde_json::json!({
                "private_key_seed_hex": hex::encode([0x42u8; 32]),
                "agent_uri": crate::ura::hub_ura(realm),
                "created_at_unix_ms": 1_714_492_800_000i64,
            })
            .to_string(),
        )
        .expect("identity file");

        assert!(
            read_backend_identity_record(realm).is_none(),
            "retired agent_uri must not be accepted as backend identity agent_ura"
        );
    }

    #[test]
    fn runtime_self_bootstrap_is_noop_without_runtime_state() {
        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());
        let identity = DaemonIdentity {
            caller_ura: "easynet:///r/realm-a/device/device-123".to_string(),
            signing_seed: None,
        };
        maybe_bootstrap_runtime_self_identity(&identity);
    }

    #[tokio::test]
    async fn start_daemon_invocation_transport_returns_ok_when_config_missing() {
        // Point HOME at an empty temp dir so the loader sees no
        // daemon-config.toml. This is the production-realistic case
        // for any device that has not yet been migrated to PR-1.
        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());

        // No panic, no error — soft skip is the contract. Bind the
        // returned SessionShutdown explicitly: its Drop runs the
        // graceful drain at scope end, which is the intended teardown
        // here (the bare-expression form reads as a dropped #[must_use]).
        let _shutdown = start_daemon_invocation_transport(
            easynet_axon::invocation::LocalRuntime::new(),
            None,
            Arc::new(
                crate::runtime::agents::agent_lifecycle_ability::SharedHotRegistrarCell::new(),
            ),
            None,
        )
        .expect("missing config is a soft skip");
    }

    #[tokio::test]
    async fn start_daemon_invocation_transport_fails_fast_on_broken_config() {
        // A PRESENT but unparseable daemon-config.toml must NOT be treated as
        // "unconfigured". Silently skipping the listener on a broken config is
        // how a hub boots "successfully" while never binding its Invocation
        // surface — the exact failure this guards against. Fail fast instead.
        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());
        let cfg_dir = temp.path().join(".easynet");
        std::fs::create_dir_all(&cfg_dir).expect("mkdir .easynet");
        std::fs::write(
            cfg_dir.join("daemon-config.toml"),
            "[daemon]\nmode = \"hub\"\nthis_is_not_a_valid_field = true\n",
        )
        .expect("write broken config");

        let result = start_daemon_invocation_transport(
            easynet_axon::invocation::LocalRuntime::new(),
            None,
            Arc::new(
                crate::runtime::agents::agent_lifecycle_ability::SharedHotRegistrarCell::new(),
            ),
            None,
        );
        assert!(
            result.is_err(),
            "a present-but-broken daemon-config.toml must fail fast, not soft-skip the listener",
        );
    }

    #[test]
    fn reload_trust_anchor_cell_from_replaces_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("realm-trust.toml");
        let hub_ura = crate::ura::hub_ura("realm");
        std::fs::write(
            &path,
            format!(
                r#"
[[trusted_agent]]
agent_ura = "{hub_ura}"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "backend"
added_at_unix_ms = 1714492800000
"#
            ),
        )
        .expect("write trust anchor");

        let cell = SharedTrustAnchor::default();
        let reloaded = reload_trust_anchor_cell_from(&path, &cell).expect("reload succeeds");
        assert_eq!(reloaded, 1);
        assert!(
            cell.snapshot().lookup(&hub_ura).is_some(),
            "SIGHUP reload must publish the on-disk entry to future admissions"
        );
    }

    #[test]
    fn reload_trust_anchor_cell_from_keeps_previous_snapshot_on_parse_error() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("realm-trust.toml");
        let hub_ura = crate::ura::hub_ura("realm");
        std::fs::write(
            &path,
            format!(
                r#"
[[trusted_agent]]
agent_ura = "{hub_ura}"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "backend"
added_at_unix_ms = 1714492800000
"#
            ),
        )
        .expect("write initial trust anchor");

        let cell = SharedTrustAnchor::default();
        reload_trust_anchor_cell_from(&path, &cell).expect("initial reload succeeds");

        std::fs::write(&path, "not valid toml = [").expect("corrupt trust anchor");
        let err = reload_trust_anchor_cell_from(&path, &cell).expect_err("reload must fail");
        assert!(
            err.to_string().contains("load trust anchor"),
            "error should name the reload path, got: {err}"
        );
        assert!(
            cell.snapshot().lookup(&hub_ura).is_some(),
            "failed reload must keep the previously published trust anchor"
        );
    }

    // ── PR-N1 commit 6/N: hub-mode boot wiring smoke ────────

    #[tokio::test]
    async fn hub_mode_boot_does_not_crash_with_federated_peers_config() {
        // Smoke-only: verify a hub-mode daemon boots with a
        // federated_peers map populated. We can't easily reach
        // into the constructed `DaemonInvocationService` (the
        // transport takes ownership), so the contract this asserts
        // is "boot returns Ok without panicking on the
        // CrossHubDialer + with_federated_peers wire-up". The
        // real-world canary smoke test (operator-side) does the
        // 2-daemon TLS round-trip; this exercise pins the boot
        // path so that test starts from a known-not-crashing
        // base.
        let _hg = crate::facade::cli::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());

        let easynet_dir = temp.path().join(".easynet");
        std::fs::create_dir_all(&easynet_dir).expect("mkdir .easynet");

        // Hub mode requires listen_tcp + cert + key. The cert
        // material does not need to be valid X.509 for the boot
        // smoke — `tls_config` parses on TLS handshake, which
        // does not run from `start_daemon_invocation_transport` (it's a
        // detached task that never receives a connection in
        // this test).
        let cert_path = easynet_dir.join("test-cert.pem");
        let key_path = easynet_dir.join("test-key.pem");
        std::fs::write(
            &cert_path,
            "-----BEGIN CERTIFICATE-----\nstub\n-----END CERTIFICATE-----\n",
        )
        .expect("seed cert");
        std::fs::write(
            &key_path,
            "-----BEGIN PRIVATE KEY-----\nstub\n-----END PRIVATE KEY-----\n",
        )
        .expect("seed key");

        let config_body = format!(
            r#"
[daemon]
mode = "hub"
realm = "test-realm-a"
listen_tcp = "127.0.0.1:0"
tls_cert_pem = {cert:?}
tls_key_pem = {key:?}

[daemon.federated_peers]
"peer-realm-b" = "https://peer-hub.example:50443"
"#,
            cert = cert_path.to_string_lossy(),
            key = key_path.to_string_lossy(),
        );
        let config_path = easynet_dir.join("daemon-config.toml");
        std::fs::write(&config_path, config_body).expect("seed daemon-config");

        // Hub-mode boot may legitimately fail at the TLS bind
        // because the cert PEM is a stub; the contract that
        // matters here is "the construction path does not panic
        // before reaching the bind stage". Wrap in
        // `catch_unwind` so a panic surfaces as a test failure
        // rather than aborting the test process.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Errors from the TLS bind are acceptable — what
            // matters is that the federation client + peers
            // wire-up did not panic before we got there.
            let _ = start_daemon_invocation_transport(
                easynet_axon::invocation::LocalRuntime::new(),
                None,
                Arc::new(
                    crate::runtime::agents::agent_lifecycle_ability::SharedHotRegistrarCell::new(),
                ),
                None,
            );
        }));
        // futures::FutureExt::catch_unwind would be nicer; we
        // use std::panic::catch_unwind via a synchronous wrapper
        // because the construction path itself is synchronous up
        // through `with_federation_client`.
        result.expect("hub-mode Invocation transport construction must not panic before bind");
    }
}
