// EasyNet CLI — daemon Invocation transport boot wiring
// =====================================================
//
// File: src/daemon/invocation/boot.rs
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
// `session.open` admission is observed in a background task and
// logged as an admission signal, but transient hub latency must not
// make the local control plane kill an otherwise healthy daemon
// before it can reconnect and republish owner projections.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;

use crate::daemon::axon_bridge::hot_agent_registrar::{
    HotAgentAdvertiseOutcome, HotAgentAdvertiseRequest, HotAgentAdvertiser, HotAgentRevokeRequest,
};
use crate::daemon::invocation::bidi::session_wire::{RequestOutcome, SessionRequestError};

use crate::daemon::invocation::admission::admission_facade::AdmissionFacade;
use crate::daemon::invocation::admission::principal_lifecycle::{
    principal_lifecycle_store_path_for_trust_anchor, PrincipalLifecycleReader,
};
use crate::daemon::invocation::admission::usage_quota::SharedUsageQuotaGate;
use crate::daemon::invocation::bidi::session_initiator::{
    initial_session_admission_probe, run_session_supervisor,
};
use crate::daemon::invocation::bidi::state::pending_dispatch::PendingDispatchMap;
use crate::daemon::invocation::bidi::state::presence::PresenceRegistry;
use crate::daemon::invocation::dispatch::daemon_invocation_service::DaemonInvocationService;
use crate::daemon::invocation::dispatch::local_session_dispatcher::LocalAxonSessionDispatcher;
use crate::daemon::persistence::daemon_config::{
    DaemonConfig, DaemonConfigError, DaemonMode, DEFAULT_DAEMON_CONFIG_PATH,
};
use crate::daemon::trust::cell::SharedTrustAnchor;

mod identity;
mod listeners;
#[cfg(unix)]
mod local_peer;
mod paths;
mod presence_seed;
mod trust;

use crate::daemon::trust::anchor::trust_anchor_path_from_env_or_default;
#[cfg(test)]
use identity::{canonical_caller_ura_from_stored_identity, StoredDeviceIdentity};
use identity::{
    load_daemon_identity_for_mode, load_runtime_signer, maybe_bootstrap_runtime_self_identity,
    DaemonIdentity,
};
use listeners::{spawn_tcp_tls_listener, spawn_uds_listener};
use paths::expand_home;
use presence_seed::seed_boot_presence;
use trust::{
    load_trust_anchor_from, reload_daemon_config_cells_from, reload_trust_anchor_cell_from,
    upsert_hub_identity,
};

/// Maximum decoded gRPC message size for InvocationServer/Client on
/// both directions. tonic's default cap is 4 MiB which aborted
/// `session.open` the moment any frame envelope grew past it (real
/// trigger: file-transfer uploads ≥ 1 MB whose accumulated down
/// frames cross 4 MiB). 64 MiB is deliberately a transport-envelope
/// cap, not an ability payload cap: large files and snapshots must be
/// chunked above gRPC instead of granting every peer a near-unbounded
/// single-message allocation. Exposed `pub` because the **client** side
/// (`session_initiator`, `session_wire`) must apply the
/// same cap as the server side; without that the asymmetry triggers
/// `OutOfRange: decoded message length too large` mid-stream.
pub const MAX_INVOCATION_GRPC_MESSAGE_BYTES: usize = 64 * 1024 * 1024;
const INITIAL_SESSION_ADMISSION_TIMEOUT: Duration = Duration::from_secs(15);

/// Bring the daemon Invocation transport online inside the
/// `easynet-daemon` process.
///
/// Returns `Ok(())` whether or not any listener was spawned — a
/// missing daemon-config.toml is the legitimate "this device is not
/// running the new transport plane yet" state, not an error. When
/// listeners do come up, they run on the caller's tokio runtime as
/// detached tasks; they own their `PresenceRegistry` Arc and stay
/// alive until the runtime shuts down.
/// `hot_agent_registrar_cell` is assembled by the ability catalogue against
/// this same `LocalRuntime`. Transport boot verifies it is `Ready`; it does not
/// own a second runtime-wiring path.
/// Owns the session supervisor's cancel oneshot. Dropping it (at
/// daemon shutdown) resolves the supervisor's `cancel` branch, so the
/// in-flight `session.open` dial drains cleanly instead of being
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
        crate::daemon::ability::builtins::agents::lifecycle::SharedHotRegistrarCell,
    >,
    plugin_runtime_manager: Option<Arc<crate::daemon::plugins::PluginRuntimeManager>>,
    hub_published_abilities: Arc<
        crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore,
    >,
    discover_federation_resolver: Option<
        Arc<crate::daemon::ability::builtins::agents::discover::DeferredDiscoverFederationResolver>,
    >,
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

    let daemon_identity = load_daemon_identity_for_mode(config.mode())?;
    if matches!(config.mode(), DaemonMode::Device | DaemonMode::Both) && daemon_identity.is_none() {
        anyhow::bail!(
            "{} daemon requires a daemon-owned Device runtime identity; run `easynet join <token>` first",
            config.mode().as_str()
        );
    }
    let hub_signer = if matches!(config.mode(), DaemonMode::Hub | DaemonMode::Both) {
        let hub_ura = crate::core::ura::hub_ura(config.realm());
        Some(load_runtime_signer(&hub_ura)?)
    } else {
        None
    };
    let daemon_ura = transport_daemon_ura(config.mode(), config.realm(), daemon_identity.as_ref());
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
    let trust_anchor = match hub_signer.as_deref() {
        Some(signer) => upsert_hub_identity(
            config.realm(),
            signer,
            &trust_anchor_path,
            load_trust_anchor_from(&trust_anchor_path),
        ),
        None => load_trust_anchor_from(&trust_anchor_path),
    };
    // PR-7 commit 5/N: wrap the boot-time anchor in a reload-friendly
    // cell. The same cell is handed to the admission facade *and* to
    // `identity.register_pubkey`'s handler context — a successful
    // register call atomically writes the file and republishes the
    // cell so the next admission sees the new entry without a daemon
    // restart.
    let trust_anchor_cell = SharedTrustAnchor::new(Arc::new(trust_anchor));
    let presence = Arc::new(PresenceRegistry::new());
    let advertised_agents = Arc::new(
        crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore::new(),
    );
    let ability_catalog = Arc::new(
        crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new(),
    );
    if let Some(resolver_cell) = discover_federation_resolver {
        let resolver = Arc::new(
            crate::daemon::ability::builtins::agents::discover::LocalDirectoryDiscoverFederationResolver::new(
                Arc::clone(&presence),
                Arc::clone(&advertised_agents),
                Arc::clone(&ability_catalog),
                daemon_ura.clone(),
            ),
        );
        if resolver_cell.set(resolver).is_err() {
            crate::op_event!(
                component = daemon_invocation,
                kind = discover_federation_resolver_second_writer,
                level = "warn",
                message = "discover federation resolver was already attached; keeping first writer",
            );
        }
    }
    let pending = Arc::new(PendingDispatchMap::new());
    let pending_stream = Arc::new(
        crate::daemon::invocation::bidi::state::pending_dispatch::PendingStreamDispatchMap::new(),
    );

    seed_boot_presence(config.mode(), daemon_ura.as_deref(), &presence);

    // Federated_peers cell first so we can hand it to BOTH the
    // DaemonInvocationService (for cross-hub `canonical_invoke`
    // routing) and the AdmissionFacade (for `FederatedKeyResolver`
    // cross-realm signature verify against peer hubs).
    let federated_peers_cell = crate::daemon::federation::peers::SharedFederatedPeers::new(
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
    // that integrates with `daemon::federation::client::
    // CrossHubDialer`'s subscribe-stream surface; today the
    // cell starts empty so consumers see no peer entries yet
    // (and gracefully degrade to local-only behaviour, the
    // same shape they show on a single-realm daemon).
    let federated_directory_cell =
        crate::daemon::federation::directory::SharedFederatedDirectoryView::default();

    // PR-N1 commit 9/N + PR-N2 commit 1/N: hub-mode daemons
    // construct one CrossHubDialer that backs both the daemon's
    // outbound `canonical_invoke` routing AND the transport policy gate's
    // `FederatedKeyResolver` so a cross-realm caller's URA can be
    // resolved via `federation.resolve_key` against the peer hub.
    // Device-mode daemons never originate federation calls, so
    // both surfaces stay local-only.
    let dialer: Option<Arc<dyn crate::daemon::federation::client::FederationClient>> =
        if matches!(config.mode(), DaemonMode::Hub | DaemonMode::Both) {
            Some(Arc::new(
                crate::daemon::federation::client::CrossHubDialer::with_trust_anchor_cell(
                    trust_anchor_cell.clone(),
                ),
            ))
        } else {
            None
        };
    let mut admission =
        AdmissionFacade::with_trust_anchor_cell(trust_anchor_cell.clone(), daemon_ura.clone());
    admission = admission.with_principal_lifecycle_reader(PrincipalLifecycleReader::new(
        principal_lifecycle_store_path_for_trust_anchor(&trust_anchor_path),
    ));
    if let Some(client) = dialer.clone() {
        admission = admission.with_federation(client, federated_peers_cell.clone());
    }
    if let Some(signer) = hub_signer.as_ref() {
        admission = admission.with_hub_signer(Arc::clone(signer));
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
    let session_admission = admission.clone();
    let mut service = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_directory_read_models(advertised_agents, ability_catalog)
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
    //     verification inside Axon's descriptor-bound request
    //     admission sees hot-reload edits immediately;
    //   * a `LedgerSink` over the SAME ledger handle the dispatch
    //     service already writes to, so once Phase 4 routes
    //     through Axon, terminal records get persisted via the
    //     SDK-canonical path (one writer, not two).
    //
    crate::daemon::axon_bridge::runtime_factory::configure_local_runtime(
        &local_runtime,
        Some(Arc::new(
            crate::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver::new(
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

    // The ability-catalog assembly owns registrar construction and runtime
    // attachment. Transport boot only verifies that the completed object is
    // Ready; attaching here as a second writer previously hid an architecture
    // fork between catalogue boot and transport boot.
    let hot_agent_registrar = hot_agent_registrar_cell.get().cloned().ok_or_else(|| {
        anyhow::anyhow!("Invocation transport requires a wired hot-Agent registrar")
    })?;
    hot_agent_registrar
        .require_ready()
        .context("Invocation transport requires a Ready hot-Agent registrar")?;

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
            match crate::daemon::ability::wire::AbilityWireRegistry::load_default_profile() {
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
                    Arc::new(crate::daemon::ability::wire::AbilityWireRegistry::core())
                }
            }
        });

    service = service.with_local_runtime(Arc::clone(&local_runtime));
    service = service.with_ability_wire_registry(Arc::clone(&ability_wire_registry));

    if let Some(signer) = hub_signer.as_ref() {
        service = service.with_hub_signer(Arc::clone(signer));
    }

    // PR-N1 commit 6/N (boot wiring) + commit 9/N (SIGHUP-aware
    // trust anchor) + commit 10/N (SHIGHUP-aware federated_peers)
    // + PR-N2 commit 1/N (FederatedKeyResolver wiring): the dialer
    // and federated_peers cell were constructed above so the
    // AdmissionFacade could pick them up too. Here we forward the
    // same handles to the DaemonInvocationService for the
    // cross-realm `canonical_invoke` dispatch path.
    if let Some(client) = dialer.clone() {
        service = service
            .with_federation_client(client)
            .with_federated_peers_cell(federated_peers_cell.clone());
    }

    // **PR-N6 C4**. Device-mode daemon's `canonical_invoke` escalates
    // up the long-lived `session.open` bidi to the hub instead of
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
    //      `escalate_canonical_invoke` arm calls. Wired into the
    //      service via `with_session_escalation`.
    //
    // Hub / Both modes leave `escalation_state = None`. Their
    // dispatcher's existing local-presence + cross-hub dial arms
    // run unchanged.
    let escalation_state = if matches!(config.mode(), DaemonMode::Device) {
        let correlation =
            crate::daemon::invocation::bidi::session_escalation::EscalationCorrelation::new();
        let outbox =
            crate::daemon::invocation::bidi::session_escalation::SharedSessionOutbox::new();
        let handle = std::sync::Arc::new(
            crate::daemon::invocation::bidi::session_escalation::spawn_escalation_consumer_with_outbox(
                Arc::clone(&correlation),
                outbox.clone(),
                config.realm().to_string(),
            ),
        );
        if let Some(identity) = daemon_identity.as_ref() {
            hot_agent_registrar.set_hot_agent_advertiser(Arc::new(
                SessionHotAgentAdvertiser::new(Arc::clone(&handle), identity.caller_ura.clone()),
            ))?;
        }
        service = service.with_session_escalation(Arc::clone(&handle));
        // One DeviceTrustSync per daemon: the self-targeted
        // canonical session dispatch arm (service) and the
        // `session.open` dispatcher both warm the anchor through
        // this instance, sharing its single-flight map and negative
        // cache. It rides the session escalation channel built above.
        let device_trust_sync = Arc::new(
            crate::daemon::invocation::admission::device_trust_sync::DeviceTrustSync::new(
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
    // scenario (4) "new peer SIGHUP appears in <agent>.discover
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
    // `session.open` bidi open for the daemon's lifetime. This is
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
                crate::daemon::invocation::bidi::session_initiator::UserTrustSync {
                    daemon_realm: config.realm().to_string(),
                    trust_anchor_path: trust_anchor_path.clone(),
                    cell: trust_anchor_cell.clone(),
                };
            session_shutdown = spawn_session_supervisor(SessionSupervisorConfig {
                hub_endpoint,
                identity,
                hub_ca_pem_path,
                escalation_state,
                local_runtime: Arc::clone(&local_runtime),
                ability_wire_registry: Arc::clone(&ability_wire_registry),
                admission: session_admission.clone(),
                plugin_runtime_manager: plugin_runtime_manager.clone(),
                hub_published_abilities: Arc::clone(&hub_published_abilities),
                user_trust_sync,
            })?;
        } else {
            crate::op_event!(
                component = daemon_invocation,
                kind = device_mode_session_supervisor_not_started,
                reason = "missing_hub_endpoint_or_device_identity",
                message = "device-mode daemon missing either hub_endpoint or credentials.json device identity; outbound `session.open` not started",
            );
        }
    }

    Ok(session_shutdown)
}

fn transport_daemon_ura(
    mode: DaemonMode,
    realm: &str,
    daemon_identity: Option<&DaemonIdentity>,
) -> Option<String> {
    match mode {
        DaemonMode::Hub | DaemonMode::Both => Some(crate::core::ura::hub_ura(realm)),
        DaemonMode::Device => daemon_identity.map(|identity| identity.caller_ura.clone()),
    }
}

/// Device-mode hot-advertise adapter for `agent.start`.
///
/// It reuses the already-open `session.open` bidi instead of
/// opening a second hub client from the lifecycle handler. The
/// lifecycle layer only sees the [`HotAgentAdvertiser`] trait; this
/// adapter owns the session-specific wire shape and caller identity.
struct SessionHotAgentAdvertiser {
    escalation: Arc<crate::daemon::invocation::bidi::session_escalation::SessionEscalationHandle>,
    caller_ura: String,
    host_node_id: Option<String>,
}

impl SessionHotAgentAdvertiser {
    fn new(
        escalation: Arc<
            crate::daemon::invocation::bidi::session_escalation::SessionEscalationHandle,
        >,
        caller_ura: String,
    ) -> Self {
        let host_node_id = crate::core::ura::parse_ura(&caller_ura)
            .ok()
            .filter(|parsed| parsed.kind == crate::core::ura::URAKind::Device)
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
                return HotAgentAdvertiseOutcome::failed(format!(
                    "encode federation.advertise_agent args: {err}"
                ));
            }
        };
        // ISS-002: carry the abilities advertise alongside the identity
        // advertise so a hot ability add/remove reaches the hub on the
        // same `session.open` escalation, immediately — not at the
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
            return HotAgentAdvertiseOutcome::failed(
                "no tokio runtime available for hot federation.advertise_agent",
            );
        };
        match outcome {
            RequestOutcome::Ok { .. } => HotAgentAdvertiseOutcome::succeeded(),
            RequestOutcome::Err { error } => {
                HotAgentAdvertiseOutcome::failed(render_session_request_error(&error))
            }
        }
    }

    fn revoke_hosted_agent(&self, request: HotAgentRevokeRequest) -> HotAgentAdvertiseOutcome {
        // ISS-002 (agent.stop, symmetric to advertise): remove the agent
        // identity from the hub directory via `federation.revoke` on the
        // same `session.open` escalation. `escalate_with_timeout`
        // builds the hub ability URA from the ability name + session
        // realm, so only the JSON args are passed here.
        let body = serde_json::json!({
            "agent_ura": request.agent_ura,
            "reason": request.reason,
        });
        let args = match serde_json::to_vec(&body) {
            Ok(args) => args,
            Err(err) => {
                return HotAgentAdvertiseOutcome::failed(format!(
                    "encode federation.revoke args: {err}"
                ));
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
            return HotAgentAdvertiseOutcome::failed(
                "no tokio runtime available for hot federation.revoke",
            );
        };
        match outcome {
            RequestOutcome::Ok { .. } => HotAgentAdvertiseOutcome::succeeded(),
            RequestOutcome::Err { error } => {
                HotAgentAdvertiseOutcome::failed(render_session_request_error(&error))
            }
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

/// Spawn the long-lived device-side `session.open` supervisor. The
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
    correlation: Arc<crate::daemon::invocation::bidi::session_escalation::EscalationCorrelation>,
    outbox: crate::daemon::invocation::bidi::session_escalation::SharedSessionOutbox,
    device_trust_sync:
        Arc<crate::daemon::invocation::admission::device_trust_sync::DeviceTrustSync>,
}

struct SessionSupervisorConfig {
    hub_endpoint: String,
    identity: DaemonIdentity,
    hub_ca_pem_path: Option<std::path::PathBuf>,
    escalation_state: Option<DeviceEscalationState>,
    local_runtime: Arc<easynet_axon::invocation::LocalRuntime>,
    ability_wire_registry: Arc<crate::daemon::ability::wire::AbilityWireRegistry>,
    admission: AdmissionFacade,
    plugin_runtime_manager: Option<Arc<crate::daemon::plugins::PluginRuntimeManager>>,
    hub_published_abilities: Arc<
        crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore,
    >,
    user_trust_sync: crate::daemon::invocation::bidi::session_initiator::UserTrustSync,
}

fn spawn_session_supervisor(config: SessionSupervisorConfig) -> anyhow::Result<SessionShutdown> {
    let SessionSupervisorConfig {
        hub_endpoint,
        identity,
        hub_ca_pem_path,
        escalation_state,
        local_runtime,
        ability_wire_registry,
        admission,
        plugin_runtime_manager,
        hub_published_abilities,
        user_trust_sync,
    } = config;

    // Build the device-owner descriptor projection from the same profile
    // registry that powers `meta.list_abilities`. RFC-005 route selection
    // consumes the hub-side owner projection; constructing it from bare
    // `LocalRuntime.list_abilities()` names made the prelude a second,
    // lossy catalogue path and could omit newly-added device abilities from
    // `namespace.resolve` while the local daemon could still dispatch them.
    let ability_descriptors =
        device_owner_session_descriptors(&identity.caller_ura, plugin_runtime_manager.as_deref());
    let signing_state = "daemon-custodied canonical signer";
    let ca_state = match hub_ca_pem_path.as_deref() {
        Some(path) => format!("pinned CA `{}`", path.display()),
        None => "system trust roots".to_string(),
    };
    let escalation_state_str = if escalation_state.is_some() {
        "canonical_invoke escalation wired"
    } else {
        "canonical_invoke escalation OFF"
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
        message = "LocalAxonSessionDispatcher will execute canonical DispatchCall frames through Axon LocalRuntime",
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
    local_dispatcher = local_dispatcher.with_admission_policy(admission);
    local_dispatcher =
        local_dispatcher.with_ability_wire_registry(Arc::clone(&ability_wire_registry));
    // Cross-device origin-caller claims: warm the anchor from the hub
    // on a miss, over the SAME authenticated session channel the
    // paired-user sync and hot-agent advertising use (a device-local
    // resolve_key invoke would be answered from this daemon's own
    // anchor and can never learn a new key). The Arc is the daemon's
    // single DeviceTrustSync, built next to the escalation consumer
    // in `start_daemon_invocation_transport` and shared with the
    // service's canonical session dispatch arm.
    if let Some(sync) = device_trust_sync {
        local_dispatcher = local_dispatcher.with_device_trust_sync(sync);
    }
    let dispatcher = Arc::new(local_dispatcher);
    let hub_endpoint_for_wait = hub_endpoint.clone();
    let caller_ura_for_wait = identity.caller_ura.clone();
    tokio::spawn(run_session_supervisor(
        crate::daemon::invocation::bidi::session_initiator::SessionSupervisorRunConfig {
            hub_endpoint,
            signer: identity.signer,
            hub_ca_pem_path,
            dispatcher,
            escalation_outbox: outbox,
            ability_descriptors,
            hub_published_abilities,
            initial_admission: Some(initial_admission),
            user_trust_sync: Some(user_trust_sync),
            connection_state_sink: Arc::new(
                crate::daemon::invocation::bidi::session_initiator::PersistentSessionConnectionStateSink,
            ),
            cancel: cancel_rx,
        },
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
    plugin_runtime_manager: Option<&crate::daemon::plugins::PluginRuntimeManager>,
) -> Vec<crate::daemon::ability::descriptors::AbilityDescriptor> {
    use crate::daemon::ability::descriptors::{AbilityDescriptor, Visibility};

    let mut descriptors =
        crate::daemon::ability::catalog::profiles::device::descriptors_for(owner_ura);
    let Some(manager) = plugin_runtime_manager else {
        return descriptors;
    };
    let Ok(state) = manager.state() else {
        return descriptors;
    };
    let Ok(plugin_descriptors) =
        crate::daemon::plugins::PluginDescriptorProjector::project(state.index())
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
    federated_peers_cell: crate::daemon::federation::peers::SharedFederatedPeers,
    quota_gate: SharedUsageQuotaGate,
    federated_key_cache: crate::daemon::invocation::admission::federated_key_resolver::SharedFederatedKeyCache,
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
    _federated_peers_cell: crate::daemon::federation::peers::SharedFederatedPeers,
    _quota_gate: SharedUsageQuotaGate,
    _federated_key_cache: crate::daemon::invocation::admission::federated_key_resolver::SharedFederatedKeyCache,
) {
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
    federation_client: Arc<dyn crate::daemon::federation::client::FederationClient>,
    federated_peers_cell: crate::daemon::federation::peers::SharedFederatedPeers,
    daemon_ura: Option<String>,
    federated_directory_cell: crate::daemon::federation::directory::SharedFederatedDirectoryView,
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
            let outcome = crate::daemon::federation::directory::poll_once(
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
    federation_client: Arc<dyn crate::daemon::federation::client::FederationClient>,
    federated_peers_cell: crate::daemon::federation::peers::SharedFederatedPeers,
    federated_directory_cell: crate::daemon::federation::directory::SharedFederatedDirectoryView,
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
                crate::daemon::federation::directory::reconcile_streaming_supervisors(
                    &snapshot,
                    &mut active,
                    |peer_realm, peer_hub_endpoint| {
                        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
                        let realm_owned = peer_realm.to_string();
                        let endpoint_owned = peer_hub_endpoint.to_string();
                        let caller_owned = caller_ura.clone();
                        let client_clone = Arc::clone(&federation_client_outer);
                        let cell_clone = directory_cell_outer.clone();
                        tokio::spawn(async move {
                            crate::daemon::federation::directory::run_per_peer_supervisor(
                                realm_owned,
                                endpoint_owned,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::SessionShutdown;
    use crate::daemon::identity::self_identity::{CanonicalSigner, TestCanonicalSigner};
    use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use ed25519_dalek::SigningKey;

    fn test_signer(owner_ura: &str, seed: [u8; 32]) -> Arc<dyn CanonicalSigner> {
        Arc::new(TestCanonicalSigner::new(owner_ura, seed))
    }

    fn test_daemon_identity(owner_ura: &str) -> DaemonIdentity {
        DaemonIdentity::bind(owner_ura.to_string(), test_signer(owner_ura, [0x33; 32]))
            .expect("bind test daemon identity")
    }

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
        let _g = crate::cli::commands::test_support::HomeGuard::new();
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
        let index = crate::daemon::plugins::PluginPackageIndex::builtin()
            .expect("builtin plugin index loads");
        let state = crate::daemon::plugins::PluginRuntimeState::from_index_with_planner(
            index,
            crate::daemon::plugins::PluginLoadPlanner::current_without_env_gates(),
        );
        let manager = crate::daemon::plugins::PluginRuntimeManager::from_state(state);
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
            agent_ura: Some(crate::core::ura::device_ura("realm-a", "device-123")),
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
        // collapsed this to `None`, which stopped the `session.open`
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
        let caller_ura = canonical_caller_ura_from_stored_identity(&stored)
            .expect("must derive a device identity");
        assert_eq!(
            caller_ura,
            "easynet:///r/localhost/device/01a5b007-f9c3-41f9-aa6f-7531267651bc",
        );
    }

    #[test]
    fn hub_only_identity_loading_does_not_require_device_credentials() {
        let _guard = crate::cli::commands::test_support::HomeGuard::new();
        let home = tempfile::tempdir().expect("hub-only test HOME");
        std::env::set_var("HOME", home.path());

        let identity = load_daemon_identity_for_mode(DaemonMode::Hub)
            .expect("Hub-only mode must not read missing device credentials");
        assert!(identity.is_none());
    }

    #[test]
    fn hub_only_identity_loading_ignores_malformed_device_credentials() {
        let _guard = crate::cli::commands::test_support::HomeGuard::new();
        let home = tempfile::tempdir().expect("hub-only test HOME");
        let state = home.path().join(".easynet");
        std::fs::create_dir_all(&state).expect("create test state directory");
        std::fs::write(state.join("credentials.json"), b"{not-json")
            .expect("write malformed credentials");
        std::env::set_var("HOME", home.path());

        let identity = load_daemon_identity_for_mode(DaemonMode::Hub)
            .expect("Hub-only mode must not parse device credentials");
        assert!(identity.is_none());
    }

    #[test]
    fn daemon_identity_from_stored_accepts_realm_only_credentials() {
        let stored = StoredDeviceIdentity {
            agent_ura: None,
            realm: Some("realm-a".to_string()),
            node_id: Some("device-123".to_string()),
            _retired_tenant_id: None,
        };
        let caller_ura = canonical_caller_ura_from_stored_identity(&stored).expect("identity");
        assert_eq!(caller_ura, "easynet:///r/realm-a/device/device-123");
    }

    #[test]
    fn hub_mode_transport_identity_uses_hub_ura_not_device_credentials() {
        let identity = test_daemon_identity("easynet:///r/cli/device/local");
        assert_eq!(
            transport_daemon_ura(DaemonMode::Hub, "hub-a.local", Some(&identity)).as_deref(),
            Some("easynet:///r/hub-a.local/hub"),
        );
        assert_eq!(
            transport_daemon_ura(DaemonMode::Both, "hub-a.local", Some(&identity)).as_deref(),
            Some("easynet:///r/hub-a.local/hub"),
        );
    }

    #[test]
    fn device_mode_transport_identity_uses_device_credentials() {
        let identity = test_daemon_identity("easynet:///r/hub-a.local/device/dev-1");
        assert_eq!(
            transport_daemon_ura(DaemonMode::Device, "hub-a.local", Some(&identity)).as_deref(),
            Some("easynet:///r/hub-a.local/device/dev-1"),
        );
        assert_eq!(
            transport_daemon_ura(DaemonMode::Device, "hub-a.local", None),
            None,
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
            canonical_caller_ura_from_stored_identity(&stored).is_none(),
            "agent_ura is no longer a fallback daemon identity"
        );
    }

    #[test]
    fn hub_identity_projection_replaces_stale_trust_anchor_key() {
        let _hg = crate::cli::commands::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());

        let realm = "realm-upsert";
        let hub_ura = crate::core::ura::hub_ura(realm);
        let new_seed = [0x42u8; 32];
        let signer = test_signer(&hub_ura, new_seed);

        let old_key = SigningKey::from_bytes(&[0x41u8; 32]);
        let old_pub = BASE64_STANDARD.encode(old_key.verifying_key().to_bytes());
        let trust_path = temp.path().join("realm-trust.toml");
        let stale = RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: hub_ura.clone(),
            public_key_b64: old_pub,
            role: TrustedAgentRole::Hub,
            added_at_unix_ms: 1,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }])
        .expect("stale anchor");

        let updated = upsert_hub_identity(realm, signer.as_ref(), &trust_path, stale);
        let want_pub =
            BASE64_STANDARD.encode(SigningKey::from_bytes(&new_seed).verifying_key().to_bytes());
        assert_eq!(
            updated
                .lookup(&hub_ura)
                .expect("backend entry")
                .public_key_b64,
            want_pub
        );
        let from_disk = RealmTrustAnchor::try_load_strict(&trust_path).expect("disk anchor");
        assert_eq!(
            from_disk
                .lookup(&hub_ura)
                .expect("backend entry on disk")
                .public_key_b64,
            want_pub
        );
    }

    #[test]
    fn hub_identity_projection_rejects_mismatched_signer_owner() {
        let _hg = crate::cli::commands::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());

        let realm = "realm-mismatched-signer";
        let hub_ura = crate::core::ura::hub_ura(realm);
        let old_key = SigningKey::from_bytes(&[0x41u8; 32]);
        let old_pub = BASE64_STANDARD.encode(old_key.verifying_key().to_bytes());
        let stale = RealmTrustAnchor::from_entries(vec![TrustedAgent {
            agent_ura: hub_ura.clone(),
            public_key_b64: old_pub.clone(),
            role: TrustedAgentRole::Hub,
            added_at_unix_ms: 1,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }])
        .expect("stale anchor");

        let wrong_signer = test_signer(&crate::core::ura::hub_ura("another-realm"), [0x42; 32]);
        let updated = upsert_hub_identity(
            realm,
            wrong_signer.as_ref(),
            &temp.path().join("realm-trust.toml"),
            stale,
        );
        assert_eq!(
            updated
                .lookup(&hub_ura)
                .expect("existing entry")
                .public_key_b64,
            old_pub,
            "a signer bound to another owner must not replace the trust entry"
        );
    }

    #[test]
    fn runtime_self_bootstrap_is_noop_without_runtime_state() {
        let _hg = crate::cli::commands::test_support::HomeGuard::new();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", temp.path());
        let identity = test_daemon_identity("easynet:///r/realm-a/device/device-123");
        maybe_bootstrap_runtime_self_identity(&identity);
    }

    #[tokio::test]
    async fn start_daemon_invocation_transport_returns_ok_when_config_missing() {
        // Point HOME at an empty temp dir so the loader sees no
        // daemon-config.toml. This is the production-realistic case
        // for any device that has not yet been migrated to PR-1.
        let _hg = crate::cli::commands::test_support::HomeGuard::new();
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
                crate::daemon::ability::builtins::agents::lifecycle::SharedHotRegistrarCell::new(),
            ),
            None,
            crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore::new(),
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
        let _hg = crate::cli::commands::test_support::HomeGuard::new();
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
                crate::daemon::ability::builtins::agents::lifecycle::SharedHotRegistrarCell::new(),
            ),
            None,
            crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore::new(),
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
        let hub_ura = crate::core::ura::hub_ura("realm");
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
        let hub_ura = crate::core::ura::hub_ura("realm");
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
        let _hg = crate::cli::commands::test_support::HomeGuard::new();
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
                    crate::daemon::ability::builtins::agents::lifecycle::SharedHotRegistrarCell::new(),
                ),
                None,
                crate::daemon::federation::read_model::hub_published_abilities::HubPublishedAbilityStore::new(),
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
