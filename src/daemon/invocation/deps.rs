// EasyNet Daemon — Invocation Service Dependency Planes
// ======================================================
//
// File: src/daemon/invocation/deps.rs
// Description: Domain-grouped dependency bundles for
//              `DaemonInvocationService`. The service used to hold
//              twenty flat fields; every new transport feature widened
//              the god-struct further (commit-plan-2 Axis E / E1,
//              to-be-fix.spec §A5). Each plane below answers one
//              question, and later dispatcher extractions (E2+) borrow
//              the plane they need instead of the whole service.
//
//   DirectoryPlane — "who is where, and what do they advertise?"
//   FederationDial — "how do we reach and sign for a peer realm?"
//   SessionPlane   — "which device<->hub correlation channels exist?"
//   IdentityPlane  — "what realm/trust-write surface does this daemon own?"
//   RuntimePlane   — "how do local abilities execute and get audited?"
//
// Admission stays a first-class field on the service: it is the gate
// every RPC method consults before any plane is touched, not a
// dependency of one domain.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use crate::daemon::invocation::device_trust_sync::DeviceTrustSync;
use crate::daemon::invocation::runtime_trust::RuntimeTrustContext;
use crate::daemon::invocation::session_escalation::SessionEscalationHandle;
use crate::daemon::invocation::session_initiator::SessionSigningSeed;
use crate::runtime::ability_wire::AbilityWireRegistry;
use crate::runtime::keyring::federated_bindings::FederatedBindingsStore;
use crate::services::ability_catalog_store::AbilityCatalogStore;
use crate::services::advertised_agent_store::AdvertisedAgentStore;
use crate::services::federated_peers_cell::SharedFederatedPeers;
use crate::services::federation_client::FederationClient;
use crate::services::federation_directory::SharedFederatedDirectoryView;
use crate::services::pending_dispatch::{PendingDispatchMap, PendingStreamDispatchMap};
use crate::services::presence_registry::PresenceRegistry;

/// Directory read plane: live device sessions, hosted-agent rows,
/// per-agent ability catalogs, and the federated directory view.
/// Everything here answers resolve/discover/subscribe reads; nothing
/// here dials a peer.
#[derive(Clone)]
pub(crate) struct DirectoryPlane {
    /// Live device sessions; liveness source of truth on hub daemons.
    pub(crate) presence: Arc<PresenceRegistry>,
    /// Hosted-agent directory rows published by
    /// `federation.advertise_agent`; maps `/agent/...` rows to their
    /// host device URA so resolve derives liveness from the host.
    pub(crate) advertised_agents: Arc<AdvertisedAgentStore>,
    /// Per-agent ability catalog populated by
    /// `federation.advertise_abilities`, projected through
    /// `federation.resolve(include_abilities=true)`.
    pub(crate) ability_catalog: Arc<AbilityCatalogStore>,
    /// Daemon-wide federated directory snapshot cell written by
    /// per-peer `RemoteDirectoryClient` tasks, read by
    /// `federation.discover`.
    pub(crate) federated_directory: SharedFederatedDirectoryView,
    /// Federated user binding store; filters cross-realm discover
    /// entries when the request supplies a `local_user_id`. `None` ⇒
    /// operator query path, no filter.
    pub(crate) federated_bindings: Option<Arc<FederatedBindingsStore>>,
    /// Heartbeat cadence (ms) for the v2 subscribe_directory server
    /// stream. Spec §2.3 pins 30 000ms in production; tests override
    /// to drive the keepalive path in real time. Always nonzero.
    pub(crate) subscribe_v2_heartbeat_interval_ms: u64,
}

/// Cross-realm dial plane: the federation client, the operator-curated
/// peer map, the hub signing seed for peer-envelope signatures, and
/// the directory-auto-route security posture.
#[derive(Clone)]
pub(crate) struct FederationDial {
    /// Cross-hub federation client. `None` ⇒ cross-realm targets
    /// return `target_offline` without dialing.
    pub(crate) client: Option<Arc<dyn FederationClient>>,
    /// Operator-curated `realm → hub_endpoint` cell per
    /// `DaemonConfig::federated_peers`; SIGHUP reloads surface to the
    /// next dispatch within ~50ms.
    pub(crate) peers: SharedFederatedPeers,
    /// Hub signing seed for cross-hub `federation.forward_invoke`
    /// peer-envelope signatures. `None` preserves the on-demand read
    /// of `~/.easynet-hub/<realm>/identity.json`.
    pub(crate) hub_signing_seed: Option<SessionSigningSeed>,
    /// When `false` (default) the dispatcher refuses to dial a peer
    /// hub whose endpoint came from an observed `federated_directory`
    /// entry — see [`crate::daemon::invocation::hub_resolver`]
    /// for the threat model. Set at boot; never toggled at runtime.
    pub(crate) allow_directory_auto_route: bool,
}

/// Device<->hub session correlation plane: per-call dispatch maps for
/// `runtime.invoke_remote`, the device-mode escalation handle, and the
/// on-miss device trust sync that rides the same session channel.
#[derive(Clone)]
pub(crate) struct SessionPlane {
    /// Cross-call correlation for `runtime.invoke_remote` dispatches
    /// awaiting a target-device reply. `None` ⇒ the ability is
    /// unavailable on this daemon (`failed_precondition`).
    pub(crate) pending: Option<Arc<PendingDispatchMap>>,
    /// Streaming correlation for remote bidi bridges that need chunked
    /// replies; same-hub `fs.transfer` is the first consumer.
    pub(crate) pending_stream: Option<Arc<PendingStreamDispatchMap>>,
    /// Device-mode escalation handle: when `Some`, federation
    /// forward_invoke routes through the existing `session.open`
    /// bidi to the hub instead of the (empty) local PresenceRegistry.
    pub(crate) escalation: Option<Arc<SessionEscalationHandle>>,
    /// On-miss device trust sync shared with the device's
    /// `session.open` dispatcher; warms the local anchor for
    /// first-contact cross-device callers. `None` on hub/both daemons
    /// (the hub IS the realm's key registrar).
    pub(crate) device_trust_sync: Option<Arc<DeviceTrustSync>>,
}

/// Runtime trust aggregate context. The same
/// `SharedTrustAnchor` cell is threaded into the `AdmissionFacade` so
/// a successful register/revoke publish is visible to the next
/// admission without a daemon restart. Cloning is cheap.
pub(crate) type RegisterPubkeyContext = RuntimeTrustContext;

/// Identity/trust write surface this daemon owns.
#[derive(Clone)]
pub(crate) struct IdentityPlane {
    /// Runtime trust aggregate for identity.register_pubkey,
    /// identity.list_user_pubkeys, and identity.revoke_user_pubkey.
    /// `None` ⇒ these abilities return `failed_precondition`
    /// (typically a smoke-test setup).
    pub(crate) runtime_trust: Option<RuntimeTrustContext>,
    /// Daemon realm for `session.open` admission-time cross-realm
    /// rejection. `None` ⇒ constructed without realm context
    /// (typically a narrow unit test); the defense-in-depth check is
    /// skipped.
    pub(crate) session_realm: Option<String>,
}

/// Local execution + audit plane: the Axon `LocalRuntime` (sole
/// in-process source of truth for local abilities), the workspace
/// invocation ledger, and the daemon-owned bidi wire profile registry.
#[derive(Clone)]
pub(crate) struct RuntimePlane {
    /// Shared Axon `LocalRuntime` built at daemon boot; direct unary,
    /// stream, bidi, and self-targeted federation dispatch all enter
    /// through this handle.
    pub(crate) local_runtime: Option<Arc<easynet_axon::invocation::LocalRuntime>>,
    /// Workspace-scoped invocation ledger
    /// (`<ledger_dir>/invocations.redb`); complete unary records are
    /// written through the Axon SDK object.
    pub(crate) invocation_ledger: Option<Arc<easynet_axon::invocation::InvocationLedger>>,
    /// Daemon-owned local bidi wire profile registry, projected from
    /// plugin wire metadata at boot.
    pub(crate) ability_wire: Arc<AbilityWireRegistry>,
}
