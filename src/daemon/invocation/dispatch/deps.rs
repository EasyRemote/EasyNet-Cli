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
// Product admission stays a first-class policy capability on the service, but
// executes through RuntimePlane's receipt-provider coordinator at Axon's
// canonical admission boundary.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::ability::wire::AbilityWireRegistry;
use crate::daemon::federation::client::FederationClient;
use crate::daemon::federation::directory::SharedFederatedDirectoryView;
use crate::daemon::federation::peers::SharedFederatedPeers;
use crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore;
use crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore;
use crate::daemon::identity::self_identity::CanonicalSigner;
use crate::daemon::invocation::admission::device_trust_sync::DeviceTrustSync;
use crate::daemon::invocation::admission::principal_lifecycle::PrincipalLifecycleContext;
use crate::daemon::invocation::admission::runtime_trust::RuntimeTrustContext;
use crate::daemon::invocation::bidi::session_escalation::SessionEscalationHandle;
use crate::daemon::invocation::bidi::state::pending_dispatch::{
    PendingDispatchMap, PendingStreamDispatchMap,
};
use crate::daemon::invocation::bidi::state::presence::PresenceRegistry;
use crate::daemon::keyring::federated_bindings::FederatedBindingsStore;

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
    /// Live daemon ability control plane. Local resolver publication and route
    /// admission capture one immutable snapshot from this same aggregate.
    pub(crate) local_ability_catalog: Option<Arc<AxonAbilityCatalog>>,
    /// Daemon-wide federated directory snapshot cell written by
    /// per-peer `RemoteDirectoryClient` tasks, read by
    /// `federation.discover`.
    pub(crate) federated_directory: SharedFederatedDirectoryView,
    /// Federated user binding store; filters cross-realm discover entries when
    /// the request supplies a `local_user_id`. User-scoped discovery fails
    /// closed when this dependency is absent; only explicit operator/audit
    /// requests omit the user id and read the unfiltered directory.
    pub(crate) federated_bindings: Option<Arc<FederatedBindingsStore>>,
    /// Heartbeat cadence (ms) for the v2 subscribe_directory server
    /// stream. Spec §2.3 pins 30 000ms in production; tests override
    /// to drive the keepalive path in real time. Always nonzero.
    pub(crate) subscribe_v2_heartbeat_interval_ms: u64,
}

/// Cross-realm dial plane: the federation client, the operator-curated
/// peer map, and the owner-bound hub signer for peer-envelope signatures.
#[derive(Clone)]
pub(crate) struct FederationDial {
    /// Cross-hub federation client. `None` ⇒ cross-realm targets
    /// return `target_offline` without dialing.
    pub(crate) client: Option<Arc<dyn FederationClient>>,
    /// Operator-curated `realm → hub_endpoint` cell per
    /// `DaemonConfig::federated_peers`; SIGHUP reloads surface to the
    /// next dispatch within ~50ms.
    pub(crate) peers: SharedFederatedPeers,
    /// Owner-bound hub signer for cross-hub `Invocation::Invoke`
    /// peer-envelope signatures. `None` is valid only for deployments without
    /// federation; an attempted peer request fails closed.
    pub(crate) hub_signer: Option<Arc<dyn CanonicalSigner>>,
}

/// Device<->hub session correlation plane: per-call dispatch maps for
/// typed session dispatch, the device-mode escalation handle, and the
/// on-miss device trust sync that rides the same session channel.
#[derive(Clone)]
pub(crate) struct SessionPlane {
    /// Cross-call correlation for typed session dispatches
    /// awaiting a target-device reply. `None` ⇒ the ability is
    /// unavailable on this daemon (`failed_precondition`).
    pub(crate) pending: Option<Arc<PendingDispatchMap>>,
    /// Streaming correlation for remote bidi bridges that need chunked
    /// replies; same-hub `fs.transfer` is the first consumer.
    pub(crate) pending_stream: Option<Arc<PendingStreamDispatchMap>>,
    /// Device-mode escalation handle: when `Some`, federation
    /// canonical_invoke routes through the existing `session.open`
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
    /// Durable PrincipalLifecycle provider using the same runtime trust
    /// substrate. `None` ⇒ principal.lifecycle.* routes fail closed.
    pub(crate) principal_lifecycle: Option<PrincipalLifecycleContext>,
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
    /// Explicit runtime binding state. A canonical-only runtime cannot carry
    /// daemon routes because it has no daemon admission graph; a daemon
    /// assembly always carries the exact graph installed at construction.
    pub(crate) binding: RuntimeBinding,
    /// Workspace-scoped invocation ledger
    /// (`<ledger_dir>/invocations.redb`); complete unary records are
    /// written through the Axon SDK object.
    pub(crate) invocation_ledger: Option<Arc<axon_sdk::invocation::InvocationLedger>>,
    /// Transport-boundary attempt audit
    /// (`<ledger_dir>/invocation-attempts.jsonl`); records requests that
    /// reject before Axon creates a canonical invocation id.
    pub(crate) attempt_ledger:
        Option<Arc<crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptLedger>>,
    /// Daemon-owned local bidi wire profile registry, projected from
    /// plugin wire metadata at boot.
    pub(crate) ability_wire: Arc<AbilityWireRegistry>,
    /// Correlates transport cancel requests with the one Axon-owned unary
    /// lifecycle that can produce terminal proof.
    pub(crate) cancellations:
        crate::daemon::invocation::dispatch::cancellation::InvocationCancellationRegistry,
}

#[derive(Clone, Default)]
pub(crate) enum RuntimeBinding {
    #[default]
    Unconfigured,
    Daemon(crate::daemon::axon_bridge::runtime_factory::DaemonRuntimeAssembly),
}

impl RuntimePlane {
    pub(crate) fn local_runtime(&self) -> Option<Arc<axon_sdk::invocation::LocalRuntime>> {
        match &self.binding {
            RuntimeBinding::Unconfigured => None,
            RuntimeBinding::Daemon(assembly) => Some(assembly.runtime()),
        }
    }

    pub(crate) fn require_local_runtime(
        &self,
        context: impl std::fmt::Display,
    ) -> Result<Arc<axon_sdk::invocation::LocalRuntime>, tonic::Status> {
        self.local_runtime().ok_or_else(|| {
            tonic::Status::failed_precondition(format!(
                "{context} requires canonical daemon runtime assembly: missing LocalRuntime"
            ))
        })
    }

    pub(crate) fn daemon_admission_graph(
        &self,
    ) -> Option<Arc<crate::daemon::axon_bridge::runtime_factory::DaemonRuntimeAdmissionGraph>> {
        match &self.binding {
            RuntimeBinding::Daemon(assembly) => Some(assembly.admission_graph()),
            RuntimeBinding::Unconfigured => None,
        }
    }

    pub(crate) fn runtime_admission(
        &self,
    ) -> Result<
        Arc<
            crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionCoordinator,
        >,
        tonic::Status,
    >{
        self.daemon_admission_graph()
            .as_ref()
            .map(|graph| graph.runtime_admission())
            .ok_or_else(|| {
                tonic::Status::failed_precondition(
                    "canonical daemon runtime assembly requires runtime admission graph",
                )
            })
    }

    pub(crate) fn stage_runtime_admission(
        &self,
        facade: &crate::daemon::invocation::admission::admission_facade::AdmissionFacade,
        wire: &crate::daemon::axon_bridge::dispatch_shim::WireDispatch,
        ability: &str,
        call_mode: axon_sdk::invocation::CallMode,
    ) -> Result<
        crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionLease,
        tonic::Status,
    > {
        self.runtime_admission()?
            .stage(facade, wire, ability, call_mode)
    }
}
