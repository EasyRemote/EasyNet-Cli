// EasyNet CLI — invocation_transport — DaemonInvocationService
// ===================================================
//
// File: src/services/invocation_transport/daemon_invocation_service.rs
// Description: Concrete implementation of axon's
//              `pb::axon::v1::invocation_server::Invocation` trait
//              for the new daemon transport plane.
//
// State + behaviour binding
// -------------------------
// The struct is the single owner of every dependency the three RPC
// methods (Invoke / InvokeStream / InvokeBidi) need at runtime. All
// dependencies are injected through the `new` constructor; the
// struct holds them by `Arc` so individual RPC method calls clone
// cheaply.
//
// What this commit lands
// ----------------------
// Commit 6/9: dispatcher wiring. The service now holds an
// `Arc<PresenceRegistry>` injected at construction; the three RPC
// methods route by `InvokeRequest.function_name`:
//
//   - `Invoke`:   federation.{join, advertise_agent, heartbeat,
//                 resolve, revoke, forward_invoke} → federation
//                 wrappers; anything else returns Unimplemented
//                 with a follow-up commit (transport policy facade,
//                 AxonAbilityCatalog forwarding) note
//   - `InvokeStream`: `federation.subscribe_directory` →
//                 initial-snapshot frame from
//                 `build_subscribe_directory_initial`; the
//                 broadcast pump for incremental events lands in
//                 commit 7/9 alongside the AxonAbilityCatalog
//                 stream forward path
//   - `InvokeBidi`: still returns Unimplemented; PR-2 implements
//                 `<self>.session` accept and PR-3 implements
//                 `<self>.invoke_remote`
//
// What the dispatcher does NOT yet do
// -----------------------------------
// - Run the transport policy gate (commit 7/9, alongside the
//   realm-trust loader and `easynet-axon` policy helpers integration)
// - Forward unmatched abilities to AxonAbilityCatalog (commit 7/9)
// - Push frames down `<self>.session` reverse channels for
//   `federation.forward_invoke` (commit 8/9)
// - Spawn the broadcast pump for `subscribe_directory` incremental
//   events (commit 8/9)
//
// Result content type
// -------------------
// All `federation.*` wrappers serialise their typed response with
// `serde_json::to_vec` into `InvokeResponse.result` and set
// `result_content_type = "application/json"`. This matches the
// JSON-encoded shape captured by PR-4's schema-compat baselines
// per DEC-001 + DEC-003.
//
// gRPC status-code policy
// -----------------------
// Three classes of failure surface back to the caller. Pick one
// class per failure site; do not improvise. Future arms added to
// this file MUST match the policy below — the policy is the only
// way a reviewer can tell, without reading the call stack, what
// the caller is being told.
//
//   * `Status::internal` — the daemon, the wire, or a backing
//     dependency (tonic streaming, serialisation, the
//     `easynet-axon` SDK encoding path) is broken. The caller did
//     nothing wrong; retrying with the same arguments will fail
//     the same way until the daemon is fixed. Wire-level
//     `tonic::Streaming::next()` errors fall here.
//
//   * `Status::invalid_argument` — the caller violated the wire
//     protocol or sent a malformed request that admission
//     accepted but dispatch cannot honour. Frame-0 sequence not
//     zero, EnvelopeOpen missing `target.ability_name`, non-
//     STRICT bidi stream ordering, etc. The caller can fix this
//     by changing the request.
//
//   * `Status::failed_precondition` — the request is well-formed
//     and the caller has admission, but THIS daemon is not
//     configured to serve it. Examples: `with_pending(...)` was
//     not called at boot so `<self>.invoke_remote` has no
//     correlation map; `LocalRuntime` is not wired so a self-
//     dispatch ability cannot run; the daemon was constructed
//     without an `InvocationLedger`. The caller can retry the
//     request against a daemon that IS configured, or the
//     operator can fix the boot wiring. Per PR-B in
//     `docs/rfc/industrial-textbook-followups-2026-05-29.md`,
//     these per-arm checks will move into `Builder::build()`
//     once the file is split; until then, each arm rejects
//     locally with this code and a message naming the missing
//     capability + the boot step that would supply it.
//
//   * `Status::not_found` — the target ability is unknown to
//     this daemon's catalogue, OR the named subject (a
//     correlation id, an agent URA the daemon does NOT host)
//     does not exist on this side. Distinct from
//     `failed_precondition` because retrying against a different
//     daemon could succeed without operator intervention.
//
//   * `Status::permission_denied` — admission was attempted and
//     rejected. Distinct from `unauthenticated` because the
//     identity IS established; it just lacks the right.
//
//   * `Status::unimplemented` — the ability name is reserved for
//     a future arm that lands in a follow-up PR. The message
//     MUST cite the follow-up so a `git grep` for the PR tag
//     surfaces the wiring site.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

// `StreamExt` brings `.next().await` into scope. Aliased to `_`
// because we use the trait method only — we don't reference the
// trait by name. Per letter 22 §4 b: avoid the name-collision risk
// Hit when bringing both `futures::StreamExt` and
// `tokio_stream::StreamExt` into scope.
use futures::StreamExt as _;
use tonic::{Request, Response, Status, Streaming};

use crate::services::federated_peers_cell::SharedFederatedPeers;
use crate::services::federation_client::FederationClient;
use crate::services::invocation_transport::admission_facade::AdmissionFacade;
use crate::services::invocation_transport::bidi_dispatcher::{
    validate_and_extract_bidi_frame0, BidiDispatcher, BidiDispatcherDeps,
};
use crate::services::invocation_transport::deps::{
    DirectoryPlane, FederationDial, IdentityPlane, RegisterPubkeyContext, RuntimePlane,
    SessionPlane,
};
use crate::services::invocation_transport::federation_wrappers::{
    ABILITY_FEDERATION_ADVERTISE_ABILITIES, ABILITY_FEDERATION_ADVERTISE_AGENT,
    ABILITY_FEDERATION_DISCOVER, ABILITY_FEDERATION_FORWARD_INVOKE, ABILITY_FEDERATION_HEARTBEAT,
    ABILITY_FEDERATION_JOIN, ABILITY_FEDERATION_LIST_USER_DEVICES,
    ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES, ABILITY_FEDERATION_RESOLVE,
    ABILITY_FEDERATION_RESOLVE_KEY, ABILITY_FEDERATION_REVOKE,
    ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY, ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
    ABILITY_NAMESPACE_PROXY_RESOLVE, ABILITY_NAMESPACE_RESOLVE,
};
use crate::services::invocation_transport::invocation_wire::BoxedDownStream;
use crate::services::invocation_transport::ledger_projection::build_unary_ledger_record;
use crate::services::invocation_transport::list_user_pubkeys::ABILITY_SELF_LIST_USER_PUBKEYS;
use crate::services::invocation_transport::quota_meter::quota_metered_ability_for_request;
use crate::services::invocation_transport::register_device_pubkey::ABILITY_SELF_REGISTER_DEVICE_PUBKEY;
use crate::services::invocation_transport::revoke_user_pubkey::ABILITY_SELF_REVOKE_USER_PUBKEY;
use crate::services::invocation_transport::session_initiator::SessionSigningSeed;
use crate::services::invocation_transport::stream_dispatcher::StreamDispatcher;
use crate::services::invocation_transport::target_gate::TargetGate;
use crate::services::invocation_transport::unary_dispatcher::{
    is_runtime_admin_ability, UnaryDispatcher,
};

use crate::services::federation_directory::now_unix_ms;
use crate::services::pending_dispatch::{PendingDispatchMap, PendingStreamDispatchMap};
use crate::services::presence_registry::PresenceRegistry;
use crate::services::trust_anchor_cell::SharedTrustAnchor;
use easynet_axon::pb::axon::v1::invocation_server::Invocation;
use easynet_axon::pb::axon::v1::{
    InvokeBidiDown, InvokeBidiUp, InvokeRequest, InvokeResponse, InvokeServerStreamRequest,
    InvokeStreamChunk,
};

/// gRPC `Invocation` service hosted by `easynet-daemon`.
///
/// Holds the dependencies the three RPC methods need:
///
/// - `presence` — the `PresenceRegistry` consulted by federation
///   wrappers (resolve / forward_invoke / revoke / heartbeat /
///   subscribe_directory) and by the future `<self>.session` accept
///   path in PR-2
/// - `admission` — the transport policy facade consulted at the start
///   of every RPC method, before any dispatch.
///
/// Future-shape (commit 8/9 onward) will add:
/// `ability_dispatch: Arc<AxonAbilityCatalog>` for the unmatched-
/// ability fallthrough. Construction will switch to
/// `new(presence, admission, ability_dispatch)` then.
///
/// `Clone` is derived so PR-10's TCP+TLS listener can register the
/// same service surface on a second tonic `Server` without holding
/// the original instance hostage. All fields are `Arc`/`Option<Arc>`/
/// `Option<String>`; clone is cheap.
#[derive(Clone)]
pub struct DaemonInvocationService {
    /// Transport policy gate consulted at the start of every RPC method,
    /// before any plane is touched.
    admission: AdmissionFacade,
    /// Directory read plane: presence, hosted-agent rows, ability
    /// catalogs, federated directory view. See [`DirectoryPlane`].
    directory: DirectoryPlane,
    /// Cross-realm dial plane: federation client, peer map, hub
    /// signing seed, auto-route posture. See [`FederationDial`].
    federation: FederationDial,
    /// Device<->hub correlation plane: pending dispatch maps,
    /// escalation handle, device trust sync. See [`SessionPlane`].
    sessions: SessionPlane,
    /// Identity/trust write surface: register-pubkey context and
    /// daemon realm. See [`IdentityPlane`].
    identity: IdentityPlane,
    /// Local execution + audit plane: Axon LocalRuntime, invocation
    /// ledger, bidi wire registry. See [`RuntimePlane`].
    runtime: RuntimePlane,
}

impl std::fmt::Debug for DaemonInvocationService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonInvocationService")
            .field("presence", &self.directory.presence)
            .field("admission", &self.admission)
            .field("pending", &self.sessions.pending)
            .field("register_pubkey", &self.identity.register_pubkey)
            .field("session_realm", &self.identity.session_realm)
            .field(
                "hub_signing_seed",
                &self.federation.hub_signing_seed.as_ref().map(|_| "<seed>"),
            )
            .field(
                "federation_client",
                &self
                    .federation
                    .client
                    .as_ref()
                    .map(|_| "<dyn FederationClient>"),
            )
            .field(
                "federated_peers_count",
                &self.federation.peers.snapshot().len(),
            )
            .field(
                "federated_bindings",
                &self
                    .directory
                    .federated_bindings
                    .as_ref()
                    .map(|_| "<store>"),
            )
            .field(
                "invocation_ledger",
                &self.runtime.invocation_ledger.as_ref().map(|_| "<redb>"),
            )
            .field(
                "local_runtime",
                &self
                    .runtime
                    .local_runtime
                    .as_ref()
                    .map(|_| "<axon LocalRuntime>"),
            )
            .finish()
    }
}

impl DaemonInvocationService {
    /// Construct a service against the supplied presence registry
    /// and transport policy facade. Production callers wire one registry
    /// per daemon process and share it via `Arc` between the
    /// service, the `<self>.session` accept loop (PR-2), and any
    /// audit-log subscriber. The policy facade is constructed
    /// from `RealmTrustAnchor::load_or_empty(...)` at daemon boot.
    ///
    /// `<self>.invoke_remote` requires an additional
    /// `PendingDispatchMap`; use `with_pending(...)` to attach one.
    /// Daemons constructed without it reject `<self>.invoke_remote`
    /// calls as not-configured rather than crashing.
    #[must_use]
    pub fn new(presence: Arc<PresenceRegistry>, admission: AdmissionFacade) -> Self {
        Self {
            admission,
            directory: DirectoryPlane {
                presence,
                advertised_agents: Arc::new(
                    crate::services::advertised_agent_store::AdvertisedAgentStore::new(),
                ),
                ability_catalog: Arc::new(
                    crate::services::ability_catalog_store::AbilityCatalogStore::new(),
                ),
                federated_directory:
                    crate::services::federation_directory::SharedFederatedDirectoryView::default(),
                federated_bindings: None,
                subscribe_v2_heartbeat_interval_ms: 30_000,
            },
            federation: FederationDial {
                client: None,
                peers: SharedFederatedPeers::default(),
                hub_signing_seed: None,
                allow_directory_auto_route: false,
            },
            sessions: SessionPlane {
                pending: None,
                pending_stream: None,
                escalation: None,
                device_trust_sync: None,
            },
            identity: IdentityPlane {
                register_pubkey: None,
                session_realm: None,
            },
            runtime: RuntimePlane {
                local_runtime: None,
                invocation_ledger: None,
                ability_wire: Arc::new(crate::runtime::ability_wire::AbilityWireRegistry::core()),
            },
        }
    }

    /// Attach the hosted-agent and owner-projection read models used
    /// by federation directory abilities.
    ///
    /// Registry-built `<agent>.discover` handlers can hold the same
    /// `Arc` stores, so `federation.advertise_*` writes and discover's
    /// user/public tiers observe one daemon-owned directory state.
    #[must_use]
    pub fn with_directory_read_models(
        mut self,
        advertised_agents: Arc<crate::services::advertised_agent_store::AdvertisedAgentStore>,
        ability_catalog: Arc<crate::services::ability_catalog_store::AbilityCatalogStore>,
    ) -> Self {
        self.directory.advertised_agents = advertised_agents;
        self.directory.ability_catalog = ability_catalog;
        self
    }

    fn record_unary_invocation(
        &self,
        request: &InvokeRequest,
        started_unix_ms: i64,
        result: &Result<Response<InvokeResponse>, Status>,
    ) {
        let Some(ledger) = self.runtime.invocation_ledger.as_ref() else {
            return;
        };
        let completed_unix_ms = now_unix_ms();
        let record =
            match build_unary_ledger_record(request, started_unix_ms, completed_unix_ms, result) {
                Ok(record) => record,
                Err(err) => {
                    let err_msg = format!("{err}");
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = ledger_record_skipped,
                        shape = "unary",
                        error = err_msg,
                    );
                    return;
                }
            };
        if let Err(err) = ledger.put(&record) {
            let err_msg = format!("{err}");
            crate::op_event!(
                component = daemon_invocation,
                kind = ledger_write_failed,
                shape = "unary",
                invocation_ura = record.invocation_ura,
                error = err_msg,
            );
        }
    }

    /// Resolve-first gate shared by the unary/stream/bidi dispatch
    /// paths. Cheap per-call construction: every plane is `Arc`-shaped.
    pub(crate) fn target_gate(&self) -> TargetGate {
        TargetGate::new(
            self.admission.clone(),
            self.directory.clone(),
            self.federation.clone(),
            self.identity.clone(),
            self.runtime.clone(),
        )
    }

    /// `InvokeStream` routing surface (commit-plan-2 E2). Cheap
    /// per-call construction: planes and gate are `Arc`-shaped.
    fn stream_dispatcher(&self) -> StreamDispatcher {
        StreamDispatcher::new(
            self.admission.clone(),
            self.directory.clone(),
            self.runtime.clone(),
            self.target_gate(),
        )
    }

    /// `Invoke` (unary) routing surface (commit-plan-2 E2). pub(crate)
    /// so module tests can drive dispatch arms directly.
    pub(crate) fn unary_dispatcher(&self) -> UnaryDispatcher {
        UnaryDispatcher::new(
            self.admission.clone(),
            self.directory.clone(),
            self.federation.clone(),
            self.sessions.clone(),
            self.identity.clone(),
            self.runtime.clone(),
            self.target_gate(),
        )
    }

    /// `InvokeBidi` routing surface (commit-plan-2 E2). pub(crate) so
    /// module tests can drive session/bidi arms directly.
    pub(crate) fn bidi_dispatcher(&self) -> BidiDispatcher {
        BidiDispatcher::new(BidiDispatcherDeps {
            admission: self.admission.clone(),
            directory: self.directory.clone(),
            sessions: self.sessions.clone(),
            identity: self.identity.clone(),
            runtime: self.runtime.clone(),
            gate: self.target_gate(),
            unary: self.unary_dispatcher(),
        })
    }

    /// Attach a `PendingDispatchMap` for `<self>.invoke_remote`
    /// dispatch correlation. Builder-style so existing
    /// `DaemonInvocationService::new(presence, admission)` callers
    /// stay source-compatible.
    ///
    /// PR-3 ownership (this commit). PR-2's `<self>.session`
    /// accept handler will share the same `Arc<PendingDispatchMap>`
    /// to call `complete(call_id, ...)` when target devices send
    /// `Result` frames back up their session streams.
    #[must_use]
    pub fn with_pending(mut self, pending: Arc<PendingDispatchMap>) -> Self {
        // Spawn a presence-event watcher that fail-fasts every
        // pending dispatch whose target_ura just went offline.
        // Without this hook, `forward_invoke`'s `await_reply()`
        // blocks on the oneshot until the operator-side HTTP
        // request times out (~30s) for a target session that's
        // already known-dead — surfacing as "your invoke just
        // hung" UX. See pending_dispatch.rs::cancel_for for the
        // matching producer.
        let watcher_pending = Arc::clone(&pending);
        let watcher_presence = Arc::clone(&self.directory.presence);
        tokio::spawn(async move {
            use crate::services::presence_registry::PresenceEvent;
            let mut events = watcher_presence.subscribe_events();
            loop {
                match events.recv().await {
                    Ok(PresenceEvent::Offline { ura, reason }) => {
                        let cancelled = watcher_pending.cancel_for(&ura, "target_offline");
                        if cancelled > 0 {
                            // `OfflineReason: Display` renders the
                            // stable snake_case wire label; no Debug
                            // double-quoting at the op-event boundary.
                            crate::op_event!(
                                component = daemon_invocation,
                                kind = presence_offline_cancel,
                                target_ura = ura,
                                reason = reason,
                                cancelled = cancelled,
                            );
                        }
                    }
                    Ok(PresenceEvent::Online { .. }) => {
                        // Nothing to do on online — pending entries
                        // for new sessions register fresh.
                    }
                    Err(_) => {
                        // Broadcast channel closed → registry
                        // dropped → daemon shutting down. Exit the
                        // watcher cleanly.
                        return;
                    }
                }
            }
        });
        self.sessions.pending = Some(pending);
        self
    }

    #[must_use]
    pub fn with_pending_stream(mut self, pending: Arc<PendingStreamDispatchMap>) -> Self {
        self.sessions.pending_stream = Some(pending);
        self
    }

    /// Attach the `<self>.register_device_pubkey` handler context
    /// (PR-7 commit 5/N). The same `SharedTrustAnchor` cell is
    /// also threaded into the `AdmissionFacade` so a successful
    /// register-pubkey publish is visible to the next admission
    /// without restarting the daemon.
    #[must_use]
    pub fn with_register_pubkey(
        mut self,
        daemon_realm: impl Into<String>,
        trust_anchor_path: impl Into<PathBuf>,
        cell: SharedTrustAnchor,
    ) -> Self {
        self.identity.register_pubkey = Some(RegisterPubkeyContext {
            daemon_realm: daemon_realm.into(),
            trust_anchor_path: trust_anchor_path.into(),
            cell,
        });
        self
    }

    /// Attach the daemon's own realm for `<self>.session`
    /// cross-realm rejection. Kept as a dedicated builder so the
    /// PR-2 guardrail does not depend on the presence of the PR-7
    /// trust-write surface.
    #[must_use]
    pub fn with_session_realm(mut self, daemon_realm: impl Into<String>) -> Self {
        self.identity.session_realm = Some(daemon_realm.into());
        self
    }

    /// Attach the hub identity seed used to sign cross-hub
    /// peer-envelope rewrites. Boot wires this best-effort from
    /// backend's `~/.easynet-hub/<realm>/identity.json`; tests can
    /// inject a deterministic fixture to avoid relying on process
    /// `HOME`.
    #[must_use]
    pub fn with_hub_signing_seed(mut self, seed: SessionSigningSeed) -> Self {
        self.admission = self.admission.with_hub_signing_seed(seed);
        self.federation.hub_signing_seed = Some(seed);
        self
    }

    /// **PR-N6 C4**. Attach a session-escalation handle. When
    /// set, `dispatch_federation_forward_invoke` routes every
    /// inbound forward_invoke call up the existing
    /// `<self>.session` bidi to the hub instead of consulting
    /// the local PresenceRegistry. Boot wires this only under
    /// `mode = "device"`; hub/both daemons leave it `None` and
    /// take the existing dispatch arm.
    #[must_use]
    pub fn with_session_escalation(
        mut self,
        handle: std::sync::Arc<
            crate::services::invocation_transport::session_escalation::SessionEscalationHandle,
        >,
    ) -> Self {
        self.sessions.escalation = Some(handle);
        self
    }

    /// Attach the daemon's shared on-miss device trust sync. See the
    /// `device_trust_sync` field invariant: device-mode boot passes
    /// the SAME `Arc` it hands the `<self>.session` dispatcher.
    #[must_use]
    pub fn with_device_trust_sync(
        mut self,
        sync: Arc<crate::services::invocation_transport::device_trust_sync::DeviceTrustSync>,
    ) -> Self {
        self.sessions.device_trust_sync = Some(sync);
        self
    }

    #[must_use]
    pub fn with_invocation_ledger(
        mut self,
        ledger: Arc<easynet_axon::invocation::InvocationLedger>,
    ) -> Self {
        crate::support::local_invocation_ledger::register_process_ledger(Arc::clone(&ledger));
        self.runtime.invocation_ledger = Some(ledger);
        self
    }

    /// Set whether this service's transport policy gate honours the loopback
    /// bypass. Boot serves the *same* service over a loopback-only UDS
    /// and an off-box TCP+TLS socket; the TCP-fed clone is given
    /// `false` so a daemon-URA spoofer reaching the TCP port still
    /// runs the full strict pipeline. See
    /// [`AdmissionFacade::with_loopback_trusted`].
    #[must_use]
    pub fn with_loopback_trusted(mut self, loopback_trusted: bool) -> Self {
        self.admission = self.admission.with_loopback_trusted(loopback_trusted);
        self
    }

    /// **Phase 2 of the Axon-SDK migration**. Attach the shared
    /// `LocalRuntime` instance. Boot constructs it after the trust
    /// anchor and invocation ledger are available so the runtime
    /// can install its `KeyResolver` + `LedgerSink` before any
    /// invocation lands.
    ///
    /// Phase 4 flips `dispatch_invoke_remote` /
    /// `dispatch_federation_forward_invoke` to route through the
    /// runtime; until then this is wired but unread, matching the
    /// "non-destructive bring-up first, hard-swap second" cadence
    /// of the migration.
    #[must_use]
    pub fn with_local_runtime(
        mut self,
        runtime: Arc<easynet_axon::invocation::LocalRuntime>,
    ) -> Self {
        self.runtime.local_runtime = Some(runtime);
        self
    }

    /// Attach the daemon-owned wire profile registry used for local bidi
    /// dispatch. Boot computes this after plugin load planning.
    #[must_use]
    pub fn with_ability_wire_registry(
        mut self,
        registry: Arc<crate::runtime::ability_wire::AbilityWireRegistry>,
    ) -> Self {
        self.runtime.ability_wire = registry;
        self
    }

    /// **PR-N1 commit 3a/N**. Attach the cross-hub federation
    /// client. Daemons booted without one return `target_offline`
    /// for cross-realm
    /// `federation.forward_invoke` calls. PR-N1 commit 3b/N
    /// rewrites the dispatcher to consume the client; commit
    /// 3a/N only stores it as a field so that rewrite has a stable
    /// constructor surface to thread through.
    #[must_use]
    pub fn with_federation_client(mut self, client: Arc<dyn FederationClient>) -> Self {
        self.federation.client = Some(client);
        self
    }

    /// **PR-N1 commit 3a/N**. Attach the operator-curated
    /// `realm → hub_endpoint` map by-value. Wraps the supplied map in
    /// a fresh `SharedFederatedPeers` cell so test fixtures that
    /// don't care about hot-reload still get the cell shape under
    /// the hood. Production daemons use
    /// [`with_federated_peers_cell`] to share the boot-time cell
    /// with the SIGHUP reload task.
    ///
    /// Empty map (the default from `DaemonInvocationService::new`)
    /// means no cross-realm routing is configured; the
    /// dispatcher's cross-realm arm then refuses to dial
    /// regardless of `federation_client` presence.
    #[must_use]
    pub fn with_federated_peers(mut self, peers: BTreeMap<String, String>) -> Self {
        self.federation.peers = SharedFederatedPeers::new(peers);
        self
    }

    /// **PR-N1 commit 10/N**. Attach the live
    /// `SharedFederatedPeers` cell so SIGHUP-driven daemon-config
    /// reloads (operator editing `[daemon.federated_peers]`)
    /// republish into the dispatcher's view within ~50ms — same
    /// cadence as the trust-anchor reload landed by commit 9/N.
    /// Production `start_daemon_invocation_transport` uses this builder; the
    /// SIGHUP task in `boot.rs` calls `cell.replace(...)` on
    /// successful TOML reload.
    #[must_use]
    pub fn with_federated_peers_cell(mut self, cell: SharedFederatedPeers) -> Self {
        self.federation.peers = cell;
        self
    }

    /// **PR-N3 commit N3-3/N3-4**. Attach the live
    /// `SharedFederatedDirectoryView` cell so the
    /// `federation.discover` dispatch arm reads the daemon-
    /// wide cross-realm directory snapshot. Per-peer
    /// `RemoteDirectoryClient` tasks (lands in N3-3.1) write
    /// into the same cell as Snapshot/Upsert/Remove frames
    /// arrive. Defaults to empty (single-realm daemons report
    /// no federated entries — graceful degradation).
    #[must_use]
    pub fn with_federated_directory_cell(
        mut self,
        cell: crate::services::federation_directory::SharedFederatedDirectoryView,
    ) -> Self {
        self.directory.federated_directory = cell;
        self
    }

    /// **2026-05-25 P0 hardening**. Set the directory-auto-route
    /// security posture. Boot wires this from
    /// `DaemonConfig::allow_directory_auto_route()`. The default
    /// (`false`) is the secure shape; this builder is the single
    /// place that should ever set `true`, and it is intended to be
    /// called from the daemon's startup path with the value the
    /// operator deliberately opted into in `daemon-config.toml`.
    #[must_use]
    pub fn with_allow_directory_auto_route(mut self, allow: bool) -> Self {
        self.federation.allow_directory_auto_route = allow;
        self
    }

    /// **N3-N4 dispatch wire**. Attach the daemon-wide
    /// federated user binding store. When a `federation.discover`
    /// request supplies a `local_user_id`, the dispatch arm
    /// constructs a `FederatedUserResolver` from this store +
    /// the daemon's own realm and routes through the filtered
    /// handler, surfacing only entries the user has opted into
    /// via PR-N4's consume flow.
    #[must_use]
    pub fn with_federated_bindings_store(
        mut self,
        bindings: std::sync::Arc<
            crate::runtime::keyring::federated_bindings::FederatedBindingsStore,
        >,
    ) -> Self {
        self.directory.federated_bindings = Some(bindings);
        self
    }

    /// **PR-N3 N3-streaming-6**. Override the v2 subscribe
    /// stream's Heartbeat cadence in milliseconds. Production
    /// stays at the 30 000ms default (spec §2.3); tests pass
    /// a sub-second value (e.g. 50ms) to exercise the
    /// keepalive path in real time without virtualised clocks.
    ///
    /// `NonZeroU64` rules out the "cadence = 0 pins the CPU"
    /// foot-gun at the type level; the previous shape was a
    /// `u64` guarded by a runtime `assert!`.
    #[must_use]
    pub fn with_subscribe_v2_heartbeat_interval_ms(mut self, ms: std::num::NonZeroU64) -> Self {
        self.directory.subscribe_v2_heartbeat_interval_ms = ms.get();
        self
    }
}

#[tonic::async_trait]
impl Invocation for DaemonInvocationService {
    /// Spec §2.1 + §4.1 reference. Routes by
    /// `InvokeRequest.function_name`:
    ///
    /// - `federation.join` / `federation.advertise_agent` /
    ///   `federation.heartbeat` / `federation.resolve` /
    ///   `federation.revoke` / `federation.forward_invoke` →
    ///   federation wrapper
    /// - anything else → Unimplemented with a "PR-1 staging" note;
    ///   commit 7/9 wires AxonAbilityCatalog as the fall-through
    async fn invoke(
        &self,
        request: Request<InvokeRequest>,
    ) -> Result<Response<InvokeResponse>, Status> {
        let inner = request.into_inner();
        let started_unix_ms = now_unix_ms();
        if let Err(err) = self.admission.verify_invoke(&inner) {
            let result: Result<Response<InvokeResponse>, Status> = Err(err);
            self.record_unary_invocation(&inner, started_unix_ms, &result);
            return result;
        }
        let function = inner.function_name.as_str();
        // #185: meter the caller after the transport policy gate. This
        // is not an Axon runtime-admitted token; descriptor-bound local
        // dispatch still enters LocalRuntime through public Axon
        // admission below. A throttled caller is rejected here with
        // `ResourceExhausted` before any dispatch work.
        let rate_limit = match quota_metered_ability_for_request(&inner) {
            Ok(Some(ability)) => match self.admission.check_quota_for_ability(&inner, &ability) {
                Ok(info) => info,
                Err(err) => {
                    let result: Result<Response<InvokeResponse>, Status> = Err(err);
                    self.record_unary_invocation(&inner, started_unix_ms, &result);
                    return result;
                }
            },
            Ok(None) => None,
            Err(err) => {
                let result: Result<Response<InvokeResponse>, Status> = Err(err);
                self.record_unary_invocation(&inner, started_unix_ms, &result);
                return result;
            }
        };

        // Phase 5e: flag set by the Axon-routed catch-all arm so we
        // skip the manual `record_unary_invocation` call below
        // (otherwise the LedgerSink write + manual write produce two
        // rows for the same call).
        let unary = self.unary_dispatcher();
        let mut axon_took_it = false;
        let result = match function {
            ABILITY_FEDERATION_JOIN => unary.dispatch_federation_join(&inner.arguments),
            ABILITY_FEDERATION_ADVERTISE_AGENT => {
                unary.dispatch_federation_advertise_agent(&inner.arguments)
            }
            ABILITY_FEDERATION_ADVERTISE_ABILITIES => {
                unary.dispatch_federation_advertise_abilities(&inner.arguments)
            }
            ABILITY_FEDERATION_HEARTBEAT => unary.dispatch_federation_heartbeat(&inner.arguments),
            ABILITY_FEDERATION_RESOLVE => unary.dispatch_federation_resolve(&inner.arguments),
            ABILITY_NAMESPACE_RESOLVE => unary.dispatch_namespace_resolve(&inner.arguments).await,
            ABILITY_FEDERATION_RESOLVE_KEY => {
                unary.dispatch_federation_resolve_key(&inner.arguments)
            }
            ABILITY_FEDERATION_DISCOVER => unary.dispatch_federation_discover(&inner.arguments),
            ABILITY_FEDERATION_LIST_USER_DEVICES => unary
                .dispatch_federation_list_user_devices(inner.envelope.as_ref(), &inner.arguments),
            ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES => {
                unary
                    .dispatch_federation_proxy_list_user_devices(
                        inner.envelope.as_ref(),
                        &inner.arguments,
                    )
                    .await
            }
            ABILITY_NAMESPACE_PROXY_RESOLVE => {
                unary
                    .dispatch_namespace_proxy_resolve(inner.envelope.as_ref(), &inner.arguments)
                    .await
            }
            ABILITY_FEDERATION_REVOKE => unary.dispatch_federation_revoke(&inner.arguments),
            ABILITY_FEDERATION_FORWARD_INVOKE => {
                unary
                    .dispatch_federation_forward_invoke(inner.envelope.as_ref(), &inner.arguments)
                    .await
            }
            ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY => Err(Status::invalid_argument(
                "federation.subscribe_directory is a server-stream ability and must be invoked \
                 via InvokeStream, not Invoke",
            )),
            ABILITY_SELF_REGISTER_DEVICE_PUBKEY => {
                unary.dispatch_register_device_pubkey(&inner.arguments)
            }
            ABILITY_SELF_REVOKE_USER_PUBKEY => unary.dispatch_revoke_user_pubkey(&inner.arguments),
            ABILITY_SELF_LIST_USER_PUBKEYS => unary.dispatch_list_user_pubkeys(&inner.arguments),
            // `runtime.*` are node-internal admin handshakes hosted by the
            // receiving daemon, not owner-routed abilities. Dispatch them
            // directly on the LocalRuntime so a hub-owner callee URA does
            // not get rejected as `NXDOMAIN owner is not online`.
            name if is_runtime_admin_ability(name) => {
                unary.dispatch_runtime_admin_ability(&inner).await
            }
            // Catch-all user abilities must pass through namespace.resolve
            // before Axon LocalRuntime dispatch. The runtime executes the
            // selected route; it is not a resolver fallback.
            //
            // `axon_took_it` gates the post-dispatch
            // `record_unary_invocation` so we don't write a duplicate
            // ledger row for calls Axon already persisted via
            // `LedgerSink`. Federation-wrapper arms above still run
            // the manual record path because they are explicit service
            // handlers rather than LocalRuntime ability dispatches.
            _other => {
                let (r, axon) = unary.dispatch_local_rpc_selected_route(&inner).await;
                axon_took_it = axon;
                r
            }
        };
        if !axon_took_it {
            self.record_unary_invocation(&inner, started_unix_ms, &result);
        }
        // #185: attach the caller's post-decrement quota status to the
        // successful response in one place, rather than threading it
        // through every dispatch arm's `InvokeResponse` builder. `None`
        // when the caller is unmetered / loopback / quota is off, in
        // which case the wire shape is unchanged.
        match (result, rate_limit) {
            (Ok(mut response), Some(info)) => {
                response.get_mut().rate_limit = Some(info);
                Ok(response)
            }
            (other, _) => other,
        }
    }

    type InvokeStreamStream = BoxedDownStream<InvokeStreamChunk>;

    /// Spec §4 reference. Routes by
    /// `InvokeServerStreamRequest.function_name`. PR-1 staging
    /// supports `federation.subscribe_directory` with the initial
    /// snapshot frame only; the broadcast pump for subsequent
    /// transitions lands in commit 8/9 alongside
    /// `federation.forward_invoke` reverse-channel push.
    async fn invoke_stream(
        &self,
        request: Request<InvokeServerStreamRequest>,
    ) -> Result<Response<Self::InvokeStreamStream>, Status> {
        let inner = request.into_inner();
        self.admission.verify_invoke_stream(&inner)?;
        let function = inner.function_name.as_str();

        let streams = self.stream_dispatcher();
        match function {
            ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY => {
                streams.dispatch_subscribe_directory_initial()
            }
            ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2 => streams.dispatch_subscribe_directory_v2(),
            _other => streams.dispatch_local_selected_route(&inner).await,
        }
    }

    type InvokeBidiStream = BoxedDownStream<InvokeBidiDown>;

    /// Spec §2.1 reference. Routes by frame-0
    /// `EnvelopeOpen.target.ability_name`:
    ///
    /// - `<self>.invoke_remote` → cross-device dispatch handler
    ///   (PR-3 commit 1/3, this commit). Requires `with_pending(...)`
    ///   to have wired a `PendingDispatchMap`; otherwise returns
    ///   `Status::failed_precondition` with explicit reason.
    /// - `<self>.session` → PR-2; arm added when PR-2 lands
    /// - anything else → `Status::unimplemented` citing PR-2/PR-3
    async fn invoke_bidi(
        &self,
        request: Request<Streaming<InvokeBidiUp>>,
    ) -> Result<Response<Self::InvokeBidiStream>, Status> {
        let mut up = request.into_inner();
        let frame0 = match up.next().await {
            Some(Ok(f)) => f,
            Some(Err(err)) => {
                return Err(Status::internal(format!("InvokeBidi frame 0 recv: {err}")));
            }
            None => return Err(Status::invalid_argument("InvokeBidi: empty up stream")),
        };

        let envelope_open = validate_and_extract_bidi_frame0(&frame0)?;
        // PR-7: full §5.2 admission for the bidi path. The facade
        // checks envelope presence + caller URA, runs the four-step
        // pipeline (envelope/structure/verify/replay), and rejects
        // with the canonical wire reasons. Ability name + initial
        // args feed `args_digest` exactly the way unary/server-stream
        // requests do.
        self.admission.verify_envelope_for_bidi(envelope_open)?;

        let ability_name = envelope_open
            .target
            .as_ref()
            .map(|t| t.ability_name.as_str())
            .filter(|n| !n.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "InvokeBidi frame 0 missing target.ability_name; cannot dispatch",
                )
            })?;

        self.bidi_dispatcher()
            .dispatch(ability_name, envelope_open, up)
            .await
    }
}

#[cfg(test)]
#[path = "daemon_invocation_service_tests.rs"]
mod tests;
