// EasyNet CLI — invocation_transport — DaemonInvocationService
// ===================================================
//
// File: src/daemon/invocation/daemon_invocation_service.rs
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
// Dispatcher shape
// ----------------
// The tonic shell verifies transport admission, applies quota policy,
// and then delegates to small dispatcher objects:
//
//   - `UnaryDispatcher` handles Hub/Federation control-plane arms,
//     identity writes, runtime-admin handshakes, and the
//     resolve-first LocalRuntime catch-all.
//   - `StreamDispatcher` handles server-stream control-plane arms
//     and stream-mode LocalRuntime dispatch.
//   - `BidiDispatcher` handles session, remote dispatch, and
//     bidi-mode LocalRuntime dispatch.
//
// Result content type
// -------------------
// All `federation.*` wrappers serialise their typed response with
// `serde_json::to_vec` into `InvokeResponse.result` and set
// `result_content_type = "application/json"`. This matches the
// JSON-encoded shape captured by PR-4's schema-compat baselines
// per DEC-001 + DEC-003.
//
// Failure projection policy
// -------------------------
// Once a unary invocation enters the canonical runtime, operational
// rejection and terminal failure are projected as `InvokeResponse`
// with state `Failed` and a typed protocol error. gRPC `Status` is
// reserved for failures that prevent construction or projection of
// that canonical outcome. Stream and bidi setup failures remain
// transport statuses until their invocation handle exists.
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
//     zero, EnvelopeOpen missing a typed target function name, non-
//     STRICT bidi stream ordering, etc. The caller can fix this
//     by changing the request.
//
//   * `Status::failed_precondition` — the request is well-formed
//     and the caller has admission, but THIS daemon is not
//     configured to serve it. Examples: `with_pending(...)` was
//     not called at boot so session dispatch has no
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

use crate::daemon::ability::CallMode;
use crate::daemon::federation::client::FederationClient;
use crate::daemon::federation::peers::SharedFederatedPeers;
use crate::daemon::identity::self_identity::CanonicalSigner;
use crate::daemon::invocation::admission::admission_facade::{
    AdmissionFacade, AdmissionTransportBoundary,
};
use crate::daemon::invocation::admission::list_user_pubkeys::ABILITY_IDENTITY_LIST_USER_PUBKEYS;
use crate::daemon::invocation::admission::principal_lifecycle::{
    ABILITY_PRINCIPAL_ADD_KEY, ABILITY_PRINCIPAL_BIND_FIRST_KEY,
    ABILITY_PRINCIPAL_CONFIGURE_RECOVERY, ABILITY_PRINCIPAL_CREATE, ABILITY_PRINCIPAL_DELETE,
    ABILITY_PRINCIPAL_GET, ABILITY_PRINCIPAL_ISSUE_ENROLLMENT, ABILITY_PRINCIPAL_ISSUE_GRANT,
    ABILITY_PRINCIPAL_REACTIVATE, ABILITY_PRINCIPAL_RECOVER, ABILITY_PRINCIPAL_REVOKE_ENROLLMENT,
    ABILITY_PRINCIPAL_REVOKE_GRANT, ABILITY_PRINCIPAL_REVOKE_KEY, ABILITY_PRINCIPAL_ROTATE_KEY,
    ABILITY_PRINCIPAL_SUSPEND,
};
use crate::daemon::invocation::admission::register_device_pubkey::{
    RegisterPubkeyBootstrapTuple, ABILITY_IDENTITY_REGISTER_PUBKEY,
};
use crate::daemon::invocation::admission::revoke_user_pubkey::ABILITY_IDENTITY_REVOKE_USER_PUBKEY;
use crate::daemon::invocation::admission::target_gate::TargetGate;
use crate::daemon::invocation::bidi::bidi_dispatcher::{
    validate_and_extract_bidi_frame0, BidiDispatcher, BidiDispatcherDeps,
};
use crate::daemon::invocation::bidi::session_initiator::ABILITY_SESSION_OPEN;
use crate::daemon::invocation::dispatch::deps::{
    DirectoryPlane, FederationDial, IdentityPlane, RegisterPubkeyContext, RuntimeBinding,
    RuntimePlane, SessionPlane,
};
use crate::daemon::invocation::dispatch::federation_wrappers::{
    ABILITY_FEDERATION_ADVERTISE_ABILITIES, ABILITY_FEDERATION_ADVERTISE_AGENT,
    ABILITY_FEDERATION_DISCOVER, ABILITY_FEDERATION_HEARTBEAT, ABILITY_FEDERATION_JOIN,
    ABILITY_FEDERATION_LIST_USER_DEVICES, ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
    ABILITY_FEDERATION_RESOLVE, ABILITY_FEDERATION_RESOLVE_KEY, ABILITY_FEDERATION_REVOKE,
    ABILITY_FEDERATION_STATUS, ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
    ABILITY_NAMESPACE_PROXY_RESOLVE, ABILITY_NAMESPACE_RESOLVE,
};
use crate::daemon::invocation::dispatch::invocation_wire::BoxedDownStream;
use crate::daemon::invocation::dispatch::unary_dispatcher::UnaryDispatcher;
use crate::daemon::invocation::streams::stream_dispatcher::StreamDispatcher;

use crate::daemon::invocation::bidi::state::pending_dispatch::{
    PendingDispatchMap, PendingStreamDispatchMap,
};
use crate::daemon::invocation::bidi::state::presence::PresenceRegistry;
use crate::daemon::trust::cell::SharedTrustAnchor;

/// Production unary daemon Invocation routes served by the exact-match arms
/// in [`Invocation::invoke`]. The dispatcher and conformance gate both use
/// this enum, so the route list has one typed owner instead of parallel string
/// lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DaemonUnaryRoute {
    FederationJoin,
    FederationAdvertiseAgent,
    FederationAdvertiseAbilities,
    FederationHeartbeat,
    FederationStatus,
    FederationResolve,
    NamespaceResolve,
    FederationResolveKey,
    FederationDiscover,
    FederationListUserDevices,
    FederationProxyListUserDevices,
    NamespaceProxyResolve,
    FederationRevoke,
    IdentityRegisterPubkey,
    IdentityRevokeUserPubkey,
    IdentityListUserPubkeys,
    PrincipalCreate,
    PrincipalBindFirstKey,
    PrincipalAddKey,
    PrincipalRotateKey,
    PrincipalRevokeKey,
    PrincipalConfigureRecovery,
    PrincipalRecover,
    PrincipalSuspend,
    PrincipalReactivate,
    PrincipalDelete,
    PrincipalIssueEnrollment,
    PrincipalRevokeEnrollment,
    PrincipalIssueGrant,
    PrincipalRevokeGrant,
    PrincipalGet,
}

impl DaemonUnaryRoute {
    pub(crate) const ALL: &'static [Self] = &[
        Self::FederationJoin,
        Self::FederationAdvertiseAgent,
        Self::FederationAdvertiseAbilities,
        Self::FederationHeartbeat,
        Self::FederationStatus,
        Self::FederationResolve,
        Self::NamespaceResolve,
        Self::FederationResolveKey,
        Self::FederationDiscover,
        Self::FederationListUserDevices,
        Self::FederationProxyListUserDevices,
        Self::NamespaceProxyResolve,
        Self::FederationRevoke,
        Self::IdentityRegisterPubkey,
        Self::IdentityRevokeUserPubkey,
        Self::IdentityListUserPubkeys,
        Self::PrincipalCreate,
        Self::PrincipalBindFirstKey,
        Self::PrincipalAddKey,
        Self::PrincipalRotateKey,
        Self::PrincipalRevokeKey,
        Self::PrincipalConfigureRecovery,
        Self::PrincipalRecover,
        Self::PrincipalSuspend,
        Self::PrincipalReactivate,
        Self::PrincipalDelete,
        Self::PrincipalIssueEnrollment,
        Self::PrincipalRevokeEnrollment,
        Self::PrincipalIssueGrant,
        Self::PrincipalRevokeGrant,
        Self::PrincipalGet,
    ];

    #[must_use]
    pub(crate) fn from_function(function: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|route| route.name() == function)
    }

    #[must_use]
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::FederationJoin => ABILITY_FEDERATION_JOIN,
            Self::FederationAdvertiseAgent => ABILITY_FEDERATION_ADVERTISE_AGENT,
            Self::FederationAdvertiseAbilities => ABILITY_FEDERATION_ADVERTISE_ABILITIES,
            Self::FederationHeartbeat => ABILITY_FEDERATION_HEARTBEAT,
            Self::FederationStatus => ABILITY_FEDERATION_STATUS,
            Self::FederationResolve => ABILITY_FEDERATION_RESOLVE,
            Self::NamespaceResolve => ABILITY_NAMESPACE_RESOLVE,
            Self::FederationResolveKey => ABILITY_FEDERATION_RESOLVE_KEY,
            Self::FederationDiscover => ABILITY_FEDERATION_DISCOVER,
            Self::FederationListUserDevices => ABILITY_FEDERATION_LIST_USER_DEVICES,
            Self::FederationProxyListUserDevices => ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
            Self::NamespaceProxyResolve => ABILITY_NAMESPACE_PROXY_RESOLVE,
            Self::FederationRevoke => ABILITY_FEDERATION_REVOKE,
            Self::IdentityRegisterPubkey => ABILITY_IDENTITY_REGISTER_PUBKEY,
            Self::IdentityRevokeUserPubkey => ABILITY_IDENTITY_REVOKE_USER_PUBKEY,
            Self::IdentityListUserPubkeys => ABILITY_IDENTITY_LIST_USER_PUBKEYS,
            Self::PrincipalCreate => ABILITY_PRINCIPAL_CREATE,
            Self::PrincipalBindFirstKey => ABILITY_PRINCIPAL_BIND_FIRST_KEY,
            Self::PrincipalAddKey => ABILITY_PRINCIPAL_ADD_KEY,
            Self::PrincipalRotateKey => ABILITY_PRINCIPAL_ROTATE_KEY,
            Self::PrincipalRevokeKey => ABILITY_PRINCIPAL_REVOKE_KEY,
            Self::PrincipalConfigureRecovery => ABILITY_PRINCIPAL_CONFIGURE_RECOVERY,
            Self::PrincipalRecover => ABILITY_PRINCIPAL_RECOVER,
            Self::PrincipalSuspend => ABILITY_PRINCIPAL_SUSPEND,
            Self::PrincipalReactivate => ABILITY_PRINCIPAL_REACTIVATE,
            Self::PrincipalDelete => ABILITY_PRINCIPAL_DELETE,
            Self::PrincipalIssueEnrollment => ABILITY_PRINCIPAL_ISSUE_ENROLLMENT,
            Self::PrincipalRevokeEnrollment => ABILITY_PRINCIPAL_REVOKE_ENROLLMENT,
            Self::PrincipalIssueGrant => ABILITY_PRINCIPAL_ISSUE_GRANT,
            Self::PrincipalRevokeGrant => ABILITY_PRINCIPAL_REVOKE_GRANT,
            Self::PrincipalGet => ABILITY_PRINCIPAL_GET,
        }
    }

    #[must_use]
    pub(crate) const fn call_mode(self) -> CallMode {
        CallMode::Rpc
    }
}

pub(crate) const DAEMON_INVOCATION_UNARY_ROUTES: &[DaemonUnaryRoute] = DaemonUnaryRoute::ALL;

/// Production server-stream daemon Invocation routes served by the exact-match
/// arms in [`Invocation::invoke_stream`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DaemonStreamRoute {
    FederationSubscribeDirectoryV2,
}

impl DaemonStreamRoute {
    pub(crate) const ALL: &'static [Self] = &[Self::FederationSubscribeDirectoryV2];

    #[must_use]
    pub(crate) fn from_function(function: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|route| route.name() == function)
    }

    #[must_use]
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::FederationSubscribeDirectoryV2 => ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
        }
    }

    #[must_use]
    pub(crate) const fn call_mode(self) -> CallMode {
        CallMode::Stream
    }
}

pub(crate) const DAEMON_INVOCATION_STREAM_ROUTES: &[DaemonStreamRoute] = DaemonStreamRoute::ALL;

/// Production bidirectional daemon Invocation routes served by exact frame-0
/// `EnvelopeOpen.target.typed_target.ability.function_name` matches in
/// [`Invocation::invoke_bidi`].
///
/// This is the bidi peer of [`DaemonUnaryRoute`] and [`DaemonStreamRoute`].
/// `session.open` owns a product carrier lifecycle behind its descriptor-bound
/// LocalRuntime registration. Keeping it in this typed inventory makes route
/// ownership, registration, and dispatch mechanically identical to the other
/// exact daemon route families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DaemonBidiRoute {
    SessionOpen,
}

impl DaemonBidiRoute {
    pub(crate) const ALL: &'static [Self] = &[Self::SessionOpen];

    #[must_use]
    pub(crate) fn from_function(function: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|route| route.name() == function)
    }

    #[must_use]
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::SessionOpen => ABILITY_SESSION_OPEN,
        }
    }

    #[must_use]
    pub(crate) const fn call_mode(self) -> CallMode {
        CallMode::Bidi
    }
}

pub(crate) const DAEMON_INVOCATION_BIDI_ROUTES: &[DaemonBidiRoute] = DaemonBidiRoute::ALL;

pub(crate) fn dispatch_function_name_for_route_table(
    function_name: &str,
    envelope: Option<&axon_sdk::pb::axon::v1::Envelope>,
) -> Result<String, Status> {
    if !is_descriptor_ref_route_token(function_name) {
        return Ok(function_name.to_string());
    }
    descriptor_ref_public_name_for_callee(function_name, envelope)
}

fn descriptor_ref_public_name_for_callee(
    function_name: &str,
    envelope: Option<&axon_sdk::pb::axon::v1::Envelope>,
) -> Result<String, Status> {
    let callee_ura = envelope
        .ok_or_else(|| {
            Status::invalid_argument(
                "daemon route descriptor_ref projection requires invocation envelope",
            )
        })?
        .callee
        .as_ref()
        .map(|callee| callee.ura.trim())
        .filter(|callee| !callee.is_empty())
        .ok_or_else(|| {
            Status::invalid_argument(
                "daemon route descriptor_ref projection requires envelope callee_ura",
            )
        })?;
    let selector =
        crate::daemon::axon_bridge::descriptor_ref::ability_selector_from_descriptor_ref(
            function_name,
        )
        .map_err(|error| {
            Status::invalid_argument(format!(
                "daemon route descriptor_ref selector projection failed: {error}"
            ))
        })?;
    if selector.owner_ura() != callee_ura {
        return Err(Status::invalid_argument(format!(
            "daemon route descriptor_ref owner `{}` does not match envelope callee `{callee_ura}`",
            selector.owner_ura()
        )));
    }
    Ok(selector.public_name().to_string())
}

fn is_descriptor_ref_route_token(function_name: &str) -> bool {
    let function_name = function_name.trim();
    function_name.starts_with("easynet:///")
        || function_name.contains('@')
        || function_name.contains('#')
        || function_name.contains('!')
}

fn missing_invocation_attempt_ledger() -> Status {
    Status::internal(
        "invocation attempt audit ledger is not wired; refusing to dispatch without \
         pre-runtime failure observability",
    )
}

fn invocation_attempt_audit_status(error: anyhow::Error) -> Status {
    Status::internal(format!("invocation attempt audit unavailable: {error:#}"))
}

use axon_sdk::pb::axon::v1::invocation_server::Invocation;
use axon_sdk::pb::axon::v1::{
    Envelope, InvokeBidiDown, InvokeBidiUp, InvokeRequest, InvokeResponse,
    InvokeServerStreamRequest, InvokeStreamChunk,
};

/// gRPC `Invocation` service hosted by `easynet-daemon`.
///
/// Holds the dependencies the three RPC methods need:
///
/// - `presence` — the `PresenceRegistry` consulted by federation
///   wrappers (resolve / canonical_invoke / revoke / heartbeat /
///   subscribe_directory_v2) and owned for `session.open` lifecycle mutation by
///   the registered Hub provider
/// - `admission_plane` — canonical runtime admission verifier shared by
///   descriptor-bound exact routes, route resolution, and remaining generic
///   carriers.
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
struct RuntimeAdmissionPlane {
    verifier: AdmissionFacade,
}

impl RuntimeAdmissionPlane {
    fn new(verifier: AdmissionFacade) -> Self {
        Self { verifier }
    }

    fn verifier(&self) -> AdmissionFacade {
        self.verifier.clone()
    }

    #[cfg(test)]
    fn verifier_ref(&self) -> &AdmissionFacade {
        &self.verifier
    }

    fn accepts_local_system_envelope(
        &self,
        envelope: Option<&axon_sdk::pb::axon::v1::Envelope>,
    ) -> bool {
        self.verifier.accepts_local_system_envelope(envelope)
    }

    fn with_transport_boundary(mut self, boundary: AdmissionTransportBoundary) -> Self {
        self.verifier = self.verifier.with_transport_boundary(boundary);
        self
    }

    #[cfg(test)]
    fn with_ability_catalog(
        mut self,
        catalog: Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog>,
    ) -> Self {
        self.verifier = self.verifier.with_ability_catalog(catalog);
        self
    }

    #[cfg(test)]
    fn access_control_stores(
        &self,
    ) -> Arc<crate::daemon::persistence::access_control::AccessControlStoreRegistry> {
        self.verifier.access_control_stores()
    }
}

impl std::fmt::Debug for RuntimeAdmissionPlane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.verifier.fmt(f)
    }
}

#[derive(Clone)]
pub struct DaemonInvocationService {
    /// Canonical runtime admission plane shared by route resolution, exact-route
    /// LocalRuntime providers, and generic carriers that still enter through the
    /// daemon Invocation service.
    admission_plane: RuntimeAdmissionPlane,
    /// Directory read plane: presence, hosted-agent rows, ability
    /// catalogs, federated directory view. See [`DirectoryPlane`].
    directory: DirectoryPlane,
    /// Cross-realm dial plane: federation client, peer map, owner-bound hub
    /// signer, auto-route posture. See [`FederationDial`].
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
    /// Exact-route provider installation lifecycle shared by every clone of
    /// this service. The cell transitions once from uninitialized to either a
    /// fully registered, sorted authority-root set or a terminal installation
    /// failure.
    daemon_unary_route_registration: Arc<tokio::sync::OnceCell<Result<Vec<String>, String>>>,
    /// Exact stream-route provider installation lifecycle. Kept separate from
    /// unary registration so a failed stream cutover cannot be mistaken for a
    /// partially valid all-route runtime surface.
    daemon_stream_route_registration: Arc<tokio::sync::OnceCell<Result<String, String>>>,
    /// Exact bidi-route provider installation lifecycle. Hub listeners may
    /// become reachable only after this cell records the complete route
    /// inventory under the Hub owner.
    daemon_bidi_route_registration: Arc<tokio::sync::OnceCell<Result<String, String>>>,
    /// Strong owner for live exact stream route pumps. Runtime-registered
    /// providers keep only a weak reference, so dropping the service tears
    /// down product stream bridges without weakening Axon's admission owner.
    daemon_stream_route_lifecycle: Arc<()>,
}

impl std::fmt::Debug for DaemonInvocationService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonInvocationService")
            .field("presence", &self.directory.presence)
            .field("admission_plane", &self.admission_plane)
            .field("pending", &self.sessions.pending)
            .field("runtime_trust", &self.identity.runtime_trust)
            .field("session_realm", &self.identity.session_realm)
            .field(
                "hub_signer",
                &self
                    .federation
                    .hub_signer
                    .as_ref()
                    .map(|signer| signer.owner_ura()),
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
                &self.runtime.local_runtime().map(|_| "<axon LocalRuntime>"),
            )
            .finish()
    }
}

fn normalize_daemon_route_owners(
    owner_uras: &[String],
) -> Result<Vec<String>, axon_sdk::invocation::AxonError> {
    let mut normalized = std::collections::BTreeSet::new();
    for owner_ura in owner_uras {
        let owner_ura = owner_ura.trim();
        if owner_ura.is_empty() {
            return Err(axon_sdk::invocation::AxonError::invalid_argument(
                "daemon unary route owner URA must not be empty",
            ));
        }
        let parsed = crate::core::ura::parse_ura(owner_ura).map_err(|error| {
            axon_sdk::invocation::AxonError::invalid_argument(format!(
                "daemon unary route owner URA is invalid: {error}"
            ))
        })?;
        if !matches!(
            parsed.kind,
            crate::core::ura::URAKind::Device | crate::core::ura::URAKind::Authority
        ) {
            return Err(axon_sdk::invocation::AxonError::invalid_argument(
                "daemon unary route owner must be a canonical Device or Authority URA",
            ));
        }
        normalized.insert(owner_ura.to_string());
    }
    Ok(normalized.into_iter().collect())
}

impl DaemonInvocationService {
    /// Construct a service against the supplied presence registry and canonical
    /// runtime admission verifier. Production callers wire one registry per
    /// daemon process and share it via `Arc` between the service, the
    /// descriptor-bound `session.open` provider, and audit subscribers. The
    /// verifier is constructed from the reloadable realm trust anchor at daemon
    /// boot.
    ///
    /// Session-routed calls require a `PendingDispatchMap`; use
    /// `with_pending(...)` to attach one.
    #[must_use]
    pub fn new(presence: Arc<PresenceRegistry>, admission: AdmissionFacade) -> Self {
        Self {
            admission_plane: RuntimeAdmissionPlane::new(admission),
            directory: DirectoryPlane {
                presence,
                advertised_agents: Arc::new(
                    crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore::new(),
                ),
                ability_catalog: Arc::new(
                    crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new(),
                ),
                local_ability_catalog: None,
                federated_directory:
                    crate::daemon::federation::directory::SharedFederatedDirectoryView::default(),
                federated_bindings: None,
                subscribe_v2_heartbeat_interval_ms: 30_000,
            },
            federation: FederationDial {
                client: None,
                peers: SharedFederatedPeers::default(),
                hub_signer: None,
            },
            sessions: SessionPlane {
                pending: None,
                pending_stream: None,
                escalation: None,
                device_trust_sync: None,
            },
            identity: IdentityPlane {
                runtime_trust: None,
                principal_lifecycle: None,
                session_realm: None,
            },
            runtime: RuntimePlane {
                binding: RuntimeBinding::Unconfigured,
                invocation_ledger: None,
                attempt_ledger: default_invocation_attempt_ledger_for_construction(),
                ability_wire: Arc::new(crate::daemon::ability::wire::AbilityWireRegistry::core()),
                cancellations: Default::default(),
            },
            daemon_unary_route_registration: Arc::new(tokio::sync::OnceCell::new()),
            daemon_stream_route_registration: Arc::new(tokio::sync::OnceCell::new()),
            daemon_bidi_route_registration: Arc::new(tokio::sync::OnceCell::new()),
            daemon_stream_route_lifecycle: Arc::new(()),
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
        advertised_agents: Arc<
            crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore,
        >,
        ability_catalog: Arc<
            crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore,
        >,
    ) -> Self {
        self.directory.advertised_agents = advertised_agents;
        self.directory.ability_catalog = ability_catalog;
        self
    }

    /// Attach the process-wide live ability control plane used by both local
    /// route admission and directory publication.
    #[must_use]
    pub fn with_local_ability_catalog(
        mut self,
        catalog: Arc<crate::daemon::ability::dispatch::AxonAbilityCatalog>,
    ) -> Self {
        self.directory.local_ability_catalog = Some(catalog);
        self
    }

    /// Resolve-first gate shared by the unary/stream/bidi dispatch
    /// paths. Cheap per-call construction: every plane is `Arc`-shaped.
    pub(crate) fn target_gate(&self) -> TargetGate {
        TargetGate::new(
            self.admission_plane.verifier(),
            self.directory.clone(),
            self.federation.clone(),
            self.identity.clone(),
        )
    }

    /// `InvokeStream` routing surface (commit-plan-2 E2). Cheap
    /// per-call construction: planes and gate are `Arc`-shaped.
    fn stream_dispatcher(&self) -> StreamDispatcher {
        StreamDispatcher::new(
            self.admission_plane.verifier(),
            self.directory.clone(),
            self.sessions.clone(),
            self.runtime.clone(),
            self.target_gate(),
            Arc::downgrade(&self.daemon_stream_route_lifecycle),
        )
    }

    /// `Invoke` (unary) routing surface (commit-plan-2 E2). pub(crate)
    /// so module tests can drive dispatch arms directly.
    pub(crate) fn unary_dispatcher(&self) -> UnaryDispatcher {
        UnaryDispatcher::new(
            self.admission_plane.verifier(),
            self.directory.clone(),
            self.federation.clone(),
            self.sessions.clone(),
            self.identity.clone(),
            self.runtime.clone(),
            self.target_gate(),
        )
    }

    /// Install the complete exact unary route family into the shared runtime.
    /// Boot and integration harnesses call this only after all product planes
    /// have reached their final configuration and before either listener
    /// becomes reachable.
    pub async fn register_daemon_unary_routes(
        &self,
        owner_ura: &str,
    ) -> Result<(), axon_sdk::invocation::AxonError> {
        self.register_daemon_unary_routes_for_owners(&[owner_ura.to_string()])
            .await
    }

    /// Install one atomic exact-route family for every local daemon authority
    /// root represented by the live catalog.
    ///
    /// Combined-authority harnesses and future explicitly supported combined
    /// deployments use this entry point. Repeating the same normalized owner
    /// set is idempotent; attempting to mutate the set after installation
    /// fails closed.
    pub async fn register_daemon_unary_routes_for_owners(
        &self,
        owner_uras: &[String],
    ) -> Result<(), axon_sdk::invocation::AxonError> {
        let owner_uras = normalize_daemon_route_owners(owner_uras)?;
        if owner_uras.is_empty() {
            return Err(axon_sdk::invocation::AxonError::invalid_argument(
                "daemon unary route owner set must not be empty",
            ));
        }
        let registration = self
            .daemon_unary_route_registration
            .get_or_init(|| async {
                let runtime = self.runtime.local_runtime().ok_or_else(|| {
                    "daemon unary route registration requires shared LocalRuntime".to_string()
                })?;
                let catalog = self.directory.local_ability_catalog.as_ref().ok_or_else(|| {
                    "daemon unary route registration requires live ability catalog".to_string()
                })?;
                let unary = self.unary_dispatcher();
                let runtime_admission = self
                    .runtime
                    .runtime_admission()
                    .map_err(|status| status.to_string())?;
                crate::daemon::invocation::dispatch::daemon_route_runtime::DaemonRouteRuntimeAdapter::new(
                    runtime,
                    self.runtime.cancellations.clone(),
                    self.admission_plane.verifier(),
                    runtime_admission,
                )
                .register_for_owners(
                    &owner_uras,
                    catalog.as_ref(),
                    unary.daemon_route_provider(),
                )
                .await
                .map_err(|error| error.to_string())?;
                Ok(owner_uras.clone())
            })
            .await;
        match registration {
            Ok(registered_owners) if registered_owners == &owner_uras => Ok(()),
            Ok(registered_owners) => {
                Err(axon_sdk::invocation::AxonError::invalid_argument(format!(
                    "daemon unary routes are registered for {registered_owners:?}, not {owner_uras:?}"
                )))
            }
            Err(error) => Err(axon_sdk::invocation::AxonError::internal(format!(
                "daemon unary route registration failed: {error}"
            ))),
        }
    }

    /// Install the complete exact server-stream route family into the shared
    /// runtime before invocation listeners become reachable.
    pub async fn register_daemon_stream_routes(
        &self,
        owner_ura: &str,
    ) -> Result<(), axon_sdk::invocation::AxonError> {
        let owner_ura = owner_ura.trim();
        if owner_ura.is_empty() {
            return Err(axon_sdk::invocation::AxonError::invalid_argument(
                "daemon stream route owner URA must not be empty",
            ));
        }
        let registration = self
            .daemon_stream_route_registration
            .get_or_init(|| async {
                let runtime = self.runtime.local_runtime().ok_or_else(|| {
                    "daemon stream route registration requires shared LocalRuntime".to_string()
                })?;
                let catalog = self.directory.local_ability_catalog.as_ref().ok_or_else(|| {
                    "daemon stream route registration requires live ability catalog".to_string()
                })?;
                let streams = self.stream_dispatcher();
                let runtime_admission = self
                    .runtime
                    .runtime_admission()
                    .map_err(|status| status.to_string())?;
                crate::daemon::invocation::dispatch::daemon_route_runtime::DaemonRouteRuntimeAdapter::new(
                    runtime,
                    self.runtime.cancellations.clone(),
                    self.admission_plane.verifier(),
                    runtime_admission,
                )
                .register_streams(owner_ura, catalog.as_ref(), streams.daemon_route_provider())
                .await
                .map_err(|error| error.to_string())?;
                Ok(owner_ura.to_string())
            })
            .await;
        match registration {
            Ok(registered_owner) if registered_owner == owner_ura => Ok(()),
            Ok(registered_owner) => Err(axon_sdk::invocation::AxonError::invalid_argument(
                format!(
                    "daemon stream routes are registered for `{registered_owner}`, not `{owner_ura}`"
                ),
            )),
            Err(error) => Err(axon_sdk::invocation::AxonError::internal(format!(
                "daemon stream route registration failed: {error}"
            ))),
        }
    }

    /// Install every exact bidi route as a descriptor-bound Hub ability before
    /// invocation listeners become reachable.
    pub async fn register_daemon_bidi_routes(
        &self,
        owner_ura: &str,
    ) -> Result<(), axon_sdk::invocation::AxonError> {
        let owner_ura = owner_ura.trim();
        let parsed = crate::core::ura::parse_ura(owner_ura).map_err(|error| {
            axon_sdk::invocation::AxonError::invalid_argument(format!(
                "daemon bidi route owner URA is invalid: {error}"
            ))
        })?;
        if parsed.kind != crate::core::ura::URAKind::Authority {
            return Err(axon_sdk::invocation::AxonError::invalid_argument(
                "daemon exact bidi routes require the canonical realm Authority owner",
            ));
        }
        let registration = self
            .daemon_bidi_route_registration
            .get_or_init(|| async {
                let runtime = self.runtime.local_runtime().ok_or_else(|| {
                    "daemon bidi route registration requires shared LocalRuntime".to_string()
                })?;
                let catalog = self.directory.local_ability_catalog.as_ref().ok_or_else(|| {
                    "daemon bidi route registration requires live ability catalog".to_string()
                })?;
                let bidi = self.bidi_dispatcher();
                let runtime_admission = self
                    .runtime
                    .runtime_admission()
                    .map_err(|status| status.to_string())?;
                crate::daemon::invocation::dispatch::daemon_route_runtime::DaemonRouteRuntimeAdapter::new(
                    runtime,
                    self.runtime.cancellations.clone(),
                    self.admission_plane.verifier(),
                    runtime_admission,
                )
                .register_bidis(owner_ura, catalog.as_ref(), bidi.daemon_route_provider())
                .await
                .map_err(|error| error.to_string())?;
                Ok(owner_ura.to_string())
            })
            .await;
        match registration {
            Ok(registered_owner) if registered_owner == owner_ura => Ok(()),
            Ok(registered_owner) => {
                Err(axon_sdk::invocation::AxonError::invalid_argument(format!(
                    "daemon bidi routes are registered for `{registered_owner}`, not `{owner_ura}`"
                )))
            }
            Err(error) => Err(axon_sdk::invocation::AxonError::internal(format!(
                "daemon bidi route registration failed: {error}"
            ))),
        }
    }

    async fn dispatch_daemon_unary_route(
        &self,
        route: DaemonUnaryRoute,
        request: &InvokeRequest,
        ingress: crate::daemon::invocation::dispatch::daemon_route_runtime::DaemonRouteIngress,
    ) -> Result<Response<InvokeResponse>, Status> {
        self.unary_dispatcher()
            .dispatch_daemon_route_runtime(route, request, ingress)
            .await
    }

    fn daemon_route_ingress(
        &self,
        route: DaemonUnaryRoute,
        request: &InvokeRequest,
    ) -> Result<crate::daemon::invocation::dispatch::daemon_route_runtime::DaemonRouteIngress, Status>
    {
        use crate::daemon::invocation::dispatch::daemon_route_runtime::DaemonRouteIngress;

        let envelope = request.envelope.as_ref().ok_or_else(|| {
            Status::invalid_argument(format!("{}: envelope is required", route.name()))
        })?;
        envelope
            .caller
            .as_ref()
            .map(|caller| caller.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument(format!("{}: envelope caller is required", route.name()))
            })?;

        if route == DaemonUnaryRoute::FederationJoin
            && FederationJoinBootstrapTuple::matches(envelope)
        {
            let proof = crate::daemon::invocation::dispatch::daemon_route_runtime::BootstrapCandidateProof::verify(
                route, request,
            )?;
            let key_provider = self
                .runtime
                .daemon_admission_graph()
                .ok_or_else(|| {
                    Status::failed_precondition(
                        "federation.join bootstrap requires the LocalRuntime admission resolver",
                    )
                })?
                .bootstrap_candidate_provider();
            return Ok(DaemonRouteIngress::Bootstrap {
                proof,
                key_provider,
            });
        }
        if route == DaemonUnaryRoute::IdentityRegisterPubkey
            && RegisterPubkeyBootstrapTuple::matches(envelope)
        {
            let proof = crate::daemon::invocation::dispatch::daemon_route_runtime::BootstrapCandidateProof::verify(
                route, request,
            )?;
            let key_provider = self
                .runtime
                .daemon_admission_graph()
                .ok_or_else(|| {
                    Status::failed_precondition(
                        "identity.register_pubkey bootstrap requires the LocalRuntime admission resolver",
                    )
                })?
                .bootstrap_candidate_provider();
            return Ok(DaemonRouteIngress::Bootstrap {
                proof,
                key_provider,
            });
        }

        if envelope.caller_signature.is_none()
            && self
                .admission_plane
                .accepts_local_system_envelope(Some(envelope))
        {
            return Ok(DaemonRouteIngress::TrustedLocalSystem);
        }
        Ok(DaemonRouteIngress::ExternalSigned)
    }

    /// `InvokeBidi` routing surface (commit-plan-2 E2). pub(crate) so
    /// module tests can drive session/bidi arms directly.
    pub(crate) fn bidi_dispatcher(&self) -> BidiDispatcher {
        BidiDispatcher::new(BidiDispatcherDeps {
            admission: self.admission_plane.verifier(),
            directory: self.directory.clone(),
            sessions: self.sessions.clone(),
            identity: self.identity.clone(),
            runtime: self.runtime.clone(),
            gate: self.target_gate(),
            unary: self.unary_dispatcher(),
        })
    }

    /// Attach the correlation table for typed session dispatch results.
    /// Builder-style because session routing is an optional process plane during
    /// tests and single-node daemon modes.
    ///
    /// The descriptor-bound `session.open` provider shares this
    /// `Arc<PendingDispatchMap>` to settle typed results returned by target
    /// devices over their session carriers.
    #[must_use]
    pub fn with_pending(mut self, pending: Arc<PendingDispatchMap>) -> Self {
        // Spawn a presence-event watcher that fail-fasts every
        // pending dispatch whose target_ura just went offline.
        // Without this hook, a dispatch's `await_reply()`
        // blocks on the oneshot until the operator-side HTTP
        // request times out (~30s) for a target session that's
        // already known-dead — surfacing as "your invoke just
        // hung" UX. See pending_dispatch.rs::cancel_for for the
        // matching producer.
        let watcher_pending = Arc::clone(&pending);
        let watcher_presence = Arc::clone(&self.directory.presence);
        tokio::spawn(async move {
            use crate::daemon::invocation::bidi::state::presence::PresenceEvent;
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
        let watcher_pending = Arc::clone(&pending);
        let watcher_presence = Arc::clone(&self.directory.presence);
        tokio::spawn(async move {
            use crate::daemon::invocation::bidi::state::presence::PresenceEvent;
            let mut events = watcher_presence.subscribe_events();
            loop {
                match events.recv().await {
                    Ok(PresenceEvent::Offline { ura, reason }) => {
                        let cancelled = watcher_pending.cancel_for(&ura, "target_offline");
                        if cancelled > 0 {
                            crate::op_event!(
                                component = daemon_invocation,
                                kind = presence_offline_cancel_stream,
                                target_ura = ura,
                                reason = reason,
                                cancelled = cancelled,
                            );
                        }
                    }
                    Ok(PresenceEvent::Online { .. }) => {}
                    Err(_) => return,
                }
            }
        });
        self.sessions.pending_stream = Some(pending);
        self
    }

    /// Attach the RuntimeTrust context used by
    /// identity.register_pubkey, identity.list_user_pubkeys, and
    /// identity.revoke_user_pubkey. The same `SharedTrustAnchor`
    /// cell is also threaded into the `AdmissionFacade` so a
    /// successful trust publish is visible to the next admission
    /// without restarting the daemon.
    #[must_use]
    pub fn with_register_pubkey(
        mut self,
        daemon_realm: impl Into<String>,
        trust_anchor_path: impl Into<PathBuf>,
        cell: SharedTrustAnchor,
    ) -> Self {
        self.identity.runtime_trust = Some(RegisterPubkeyContext {
            daemon_realm: daemon_realm.into(),
            trust_anchor_path: trust_anchor_path.into(),
            cell,
        });
        self.identity.principal_lifecycle = self
            .identity
            .runtime_trust
            .clone()
            .map(crate::daemon::invocation::admission::principal_lifecycle::PrincipalLifecycleContext::from_runtime_trust);
        self
    }

    /// Attach the daemon's own realm for `session.open`
    /// cross-realm rejection. Kept as a dedicated builder so the
    /// PR-2 guardrail does not depend on the presence of the PR-7
    /// trust-write surface.
    #[must_use]
    pub fn with_session_realm(mut self, daemon_realm: impl Into<String>) -> Self {
        self.identity.session_realm = Some(daemon_realm.into());
        self
    }

    /// Attach the owner-bound hub capability used to sign cross-hub
    /// peer-envelope rewrites. No private material enters dispatch.
    #[must_use]
    pub fn with_hub_signer(mut self, signer: Arc<dyn CanonicalSigner>) -> Self {
        self.federation.hub_signer = Some(signer);
        self
    }

    /// Attach the device-mode reverse-session relay. Device daemons submit
    /// complete signed requests to their hub through `session.open`; hub/both
    /// daemons route directly from their authoritative presence state.
    #[must_use]
    pub fn with_session_escalation(
        mut self,
        handle: std::sync::Arc<
            crate::daemon::invocation::bidi::session_escalation::SessionEscalationHandle,
        >,
    ) -> Self {
        self.sessions.escalation = Some(handle);
        self
    }

    /// Attach the daemon's shared on-miss device trust sync. See the
    /// `device_trust_sync` field invariant: device-mode boot passes
    /// the SAME `Arc` it hands the `session.open` dispatcher.
    #[must_use]
    pub fn with_device_trust_sync(
        mut self,
        sync: Arc<crate::daemon::invocation::admission::device_trust_sync::DeviceTrustSync>,
    ) -> Self {
        self.sessions.device_trust_sync = Some(sync);
        self
    }

    fn begin_invoke_attempt(
        &self,
        request: &InvokeRequest,
    ) -> Result<crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptHandle, Status>
    {
        self.runtime
            .attempt_ledger
            .as_ref()
            .ok_or_else(missing_invocation_attempt_ledger)?
            .begin_invoke(request)
            .map_err(invocation_attempt_audit_status)
    }

    fn begin_stream_attempt(
        &self,
        request: &InvokeServerStreamRequest,
    ) -> Result<crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptHandle, Status>
    {
        self.runtime
            .attempt_ledger
            .as_ref()
            .ok_or_else(missing_invocation_attempt_ledger)?
            .begin_stream(request)
            .map_err(invocation_attempt_audit_status)
    }

    fn begin_bidi_attempt(
        &self,
    ) -> Result<crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptHandle, Status>
    {
        self.runtime
            .attempt_ledger
            .as_ref()
            .ok_or_else(missing_invocation_attempt_ledger)?
            .begin(
                "InvokeBidi",
                crate::daemon::invocation::dispatch::attempt_audit::AttemptIdentity::pending_bidi_open(),
            )
            .map_err(invocation_attempt_audit_status)
    }

    #[must_use]
    pub fn with_invocation_ledger(
        mut self,
        ledger: Arc<axon_sdk::invocation::InvocationLedger>,
    ) -> Self {
        self.runtime.invocation_ledger = Some(ledger);
        self
    }

    #[must_use]
    pub(crate) fn with_invocation_attempt_ledger(
        mut self,
        ledger: Arc<crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptLedger>,
    ) -> Self {
        self.runtime.attempt_ledger = Some(ledger);
        self
    }

    /// Attach the transport-boundary attempt audit ledger by path.
    ///
    /// This is the public configuration seam for integration harnesses and
    /// embedders that construct `DaemonInvocationService` directly instead of
    /// going through daemon boot. Boot code may still inject a pre-opened
    /// ledger so all service clones share the same writer handle.
    pub fn with_invocation_attempt_ledger_path(
        mut self,
        path: impl Into<PathBuf>,
    ) -> anyhow::Result<Self> {
        let ledger =
            crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptLedger::open(
                path,
            )?;
        self.runtime.attempt_ledger = Some(Arc::new(ledger));
        Ok(self)
    }

    /// Set this service's admission transport boundary. Boot serves the same
    /// service over local-only IPC and off-box TCP/TLS; the TCP-fed clone is
    /// given `OffBoxStrict` so a daemon-URA spoofer reaching the TCP port still
    /// runs the full strict pipeline. See
    /// [`AdmissionFacade::with_transport_boundary`].
    #[must_use]
    pub fn with_transport_boundary(mut self, boundary: AdmissionTransportBoundary) -> Self {
        self.admission_plane = self.admission_plane.with_transport_boundary(boundary);
        self
    }

    /// Attach the complete daemon runtime assembly. Product route registration
    /// cannot observe a runtime without its construction-time policy graph.
    #[must_use]
    pub fn with_daemon_runtime(
        mut self,
        assembly: crate::daemon::axon_bridge::runtime_factory::DaemonRuntimeAssembly,
    ) -> Self {
        self.runtime.binding = RuntimeBinding::Daemon(assembly);
        self
    }

    #[must_use]
    pub fn with_invocation_cancellation_registry(
        mut self,
        registry: crate::daemon::invocation::dispatch::cancellation::InvocationCancellationRegistry,
    ) -> Self {
        self.runtime.cancellations = registry;
        self
    }

    /// Attach the daemon-owned wire profile registry used for local bidi
    /// dispatch. Boot computes this after plugin load planning.
    #[must_use]
    pub fn with_ability_wire_registry(
        mut self,
        registry: Arc<crate::daemon::ability::wire::AbilityWireRegistry>,
    ) -> Self {
        self.runtime.ability_wire = registry;
        self
    }

    /// Attach the peer Invocation transport. Daemons without one reject
    /// cross-realm routes as unavailable.
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
        cell: crate::daemon::federation::directory::SharedFederatedDirectoryView,
    ) -> Self {
        self.directory.federated_directory = cell;
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
            crate::daemon::keyring::federated_bindings::FederatedBindingsStore,
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

struct FederationJoinBootstrapTuple;

impl FederationJoinBootstrapTuple {
    fn matches(envelope: &Envelope) -> bool {
        let Some(caller_ura) = envelope
            .caller
            .as_ref()
            .map(|identity| identity.ura.trim())
            .filter(|ura| !ura.is_empty())
        else {
            return false;
        };
        let Some(callee_ura) = envelope
            .callee
            .as_ref()
            .map(|identity| identity.ura.trim())
            .filter(|ura| !ura.is_empty())
        else {
            return false;
        };
        let Some(subject_ura) = envelope
            .subject
            .as_ref()
            .map(|identity| identity.ura.trim())
            .filter(|ura| !ura.is_empty())
        else {
            return false;
        };
        if caller_ura != subject_ura {
            return false;
        }

        let Ok(caller) = crate::core::ura::parse_ura(caller_ura) else {
            return false;
        };
        let Ok(callee) = crate::core::ura::parse_ura(callee_ura) else {
            return false;
        };
        let Ok(subject) = crate::core::ura::parse_ura(subject_ura) else {
            return false;
        };
        caller.kind == crate::core::ura::URAKind::Device
            && callee.kind == crate::core::ura::URAKind::Authority
            && subject.kind == crate::core::ura::URAKind::Device
            && caller.realm == callee.realm
            && caller.realm == subject.realm
    }
}

#[cfg(test)]
fn default_invocation_attempt_ledger_for_construction(
) -> Option<Arc<crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptLedger>> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let path = std::env::temp_dir().join(format!(
        "easynet-test-daemon-service-attempts-{}-{}.jsonl",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    Some(Arc::new(
        crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptLedger::open(path)
            .expect("test daemon invocation attempt ledger"),
    ))
}

#[cfg(not(test))]
fn default_invocation_attempt_ledger_for_construction(
) -> Option<Arc<crate::daemon::invocation::dispatch::attempt_audit::InvocationAttemptLedger>> {
    None
}

#[tonic::async_trait]
impl Invocation for DaemonInvocationService {
    /// Spec §2.1 + §4.1 reference. Routes unary calls by exact
    /// `InvokeRequest.function_name`. User/device abilities fall
    /// through to namespace.resolve and then Axon LocalRuntime.
    async fn invoke(
        &self,
        request: Request<InvokeRequest>,
    ) -> Result<Response<InvokeResponse>, Status> {
        let inner = request.into_inner();
        let attempt = self.begin_invoke_attempt(&inner)?;
        let function = match crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
            "Invoke",
            inner.target.as_ref(),
        ) {
            Ok(function) => function,
            Err(status) => {
                attempt
                    .reject_status("target", &status)
                    .map_err(invocation_attempt_audit_status)?;
                return Err(status);
            }
        };
        let route_function =
            match dispatch_function_name_for_route_table(function, inner.envelope.as_ref()) {
                Ok(route_function) => route_function,
                Err(status) => {
                    attempt
                        .reject_status("descriptor_ref_route_projection", &status)
                        .map_err(invocation_attempt_audit_status)?;
                    return Err(status);
                }
            };
        let daemon_route = DaemonUnaryRoute::from_function(&route_function);
        let daemon_route_ingress = match daemon_route
            .map(|route| self.daemon_route_ingress(route, &inner))
            .transpose()
        {
            Ok(ingress) => ingress,
            Err(status) => {
                attempt
                    .reject_status("daemon_route_ingress", &status)
                    .map_err(invocation_attempt_audit_status)?;
                return Err(status);
            }
        };

        let unary = self.unary_dispatcher();
        let result = match daemon_route {
            Some(route) => {
                self.dispatch_daemon_unary_route(
                    route,
                    &inner,
                    daemon_route_ingress.expect("exact route ingress must be classified"),
                )
                .await
            }
            None if DaemonStreamRoute::from_function(&route_function).is_some() => {
                Err(Status::invalid_argument(format!(
                    "{route_function} is a server-stream ability and must be invoked via InvokeStream, not Invoke"
                )))
            }
            // Catch-all user abilities must pass through namespace.resolve
            // before Axon LocalRuntime dispatch. The runtime executes the
            // selected route; it is not a resolver fallback.
            None => {
                let (r, _runtime_started) =
                    unary.dispatch_local_rpc_selected_route(&inner).await;
                r
            }
        };
        match result {
            Ok(response) => {
                attempt
                    .finalize_response("runtime", response.get_ref())
                    .map_err(invocation_attempt_audit_status)?;
                Ok(response)
            }
            Err(status) => {
                attempt
                    .reject_status("routing", &status)
                    .map_err(invocation_attempt_audit_status)?;
                Err(status)
            }
        }
    }

    type InvokeStreamStream = BoxedDownStream<InvokeStreamChunk>;

    /// Spec §4 reference. Routes by `InvokeServerStreamRequest.function_name`.
    /// The public federation directory stream is the canonical
    /// `federation.subscribe_directory_v2` DirectoryEvent surface.
    async fn invoke_stream(
        &self,
        request: Request<InvokeServerStreamRequest>,
    ) -> Result<Response<Self::InvokeStreamStream>, Status> {
        let inner = request.into_inner();
        let attempt = self.begin_stream_attempt(&inner)?;
        let function = match crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
            "InvokeStream",
            inner.target.as_ref(),
        ) {
            Ok(function) => function,
            Err(status) => {
                attempt
                    .reject_status("target", &status)
                    .map_err(invocation_attempt_audit_status)?;
                return Err(status);
            }
        };
        let route_function =
            match dispatch_function_name_for_route_table(function, inner.envelope.as_ref()) {
                Ok(route_function) => route_function,
                Err(status) => {
                    attempt
                        .reject_status("descriptor_ref_route_projection", &status)
                        .map_err(invocation_attempt_audit_status)?;
                    return Err(status);
                }
            };

        let streams = self.stream_dispatcher();
        let result = match DaemonStreamRoute::from_function(&route_function) {
            Some(route) => streams.dispatch_daemon_route_runtime(route, &inner).await,
            None => streams.dispatch_selected_route(&inner).await,
        };
        match result {
            Ok(response) => {
                attempt
                    .mark_started("stream_dispatch")
                    .map_err(invocation_attempt_audit_status)?;
                Ok(response)
            }
            Err(status) => {
                attempt
                    .reject_status("stream_dispatch", &status)
                    .map_err(invocation_attempt_audit_status)?;
                Err(status)
            }
        }
    }

    type InvokeBidiStream = BoxedDownStream<InvokeBidiDown>;

    /// Spec §2.1 reference. Routes by frame-0
    /// `EnvelopeOpen.target.typed_target.ability.function_name`:
    ///
    /// - `session.open` accepts a long-lived reverse channel.
    /// - registered builtin/plugin bidi abilities use their declared wire
    ///   profile.
    async fn invoke_bidi(
        &self,
        request: Request<Streaming<InvokeBidiUp>>,
    ) -> Result<Response<Self::InvokeBidiStream>, Status> {
        let mut attempt = self.begin_bidi_attempt()?;
        let mut up = request.into_inner();
        let frame0 = match up.next().await {
            Some(Ok(f)) => f,
            Some(Err(err)) => {
                let status = Status::internal(format!("InvokeBidi frame 0 recv: {err}"));
                attempt
                    .reject_status("bidi_frame0_recv", &status)
                    .map_err(invocation_attempt_audit_status)?;
                return Err(status);
            }
            None => {
                let status = Status::invalid_argument("InvokeBidi: empty up stream");
                attempt
                    .reject_status("bidi_frame0", &status)
                    .map_err(invocation_attempt_audit_status)?;
                return Err(status);
            }
        };

        let envelope_open = match validate_and_extract_bidi_frame0(&frame0) {
            Ok(open) => open,
            Err(status) => {
                attempt
                    .reject_status("bidi_frame0", &status)
                    .map_err(invocation_attempt_audit_status)?;
                return Err(status);
            }
        };
        attempt = attempt.with_identity(
            crate::daemon::invocation::dispatch::attempt_audit::AttemptIdentity::from_bidi_open(
                envelope_open,
            ),
        );
        let ability_name = match crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
            "InvokeBidi frame 0",
            envelope_open.target.as_ref(),
        ) {
            Ok(ability_name) => ability_name,
            Err(status) => {
                attempt
                    .reject_status("target", &status)
                    .map_err(invocation_attempt_audit_status)?;
                return Err(status);
            }
        };
        let route_function = match dispatch_function_name_for_route_table(
            ability_name,
            envelope_open.envelope.as_ref(),
        ) {
            Ok(route_function) => route_function,
            Err(status) => {
                attempt
                    .reject_status("descriptor_ref_route_projection", &status)
                    .map_err(invocation_attempt_audit_status)?;
                return Err(status);
            }
        };

        let dispatcher = self.bidi_dispatcher();
        let result = match DaemonBidiRoute::from_function(&route_function) {
            Some(route) => {
                dispatcher
                    .dispatch_daemon_route_runtime(route, envelope_open, up)
                    .await
            }
            None => {
                dispatcher
                    .dispatch(&route_function, envelope_open, up)
                    .await
            }
        };
        match result {
            Ok(response) => {
                attempt
                    .mark_started("bidi_dispatch")
                    .map_err(invocation_attempt_audit_status)?;
                Ok(response)
            }
            Err(status) => {
                attempt
                    .reject_status("bidi_dispatch", &status)
                    .map_err(invocation_attempt_audit_status)?;
                Err(status)
            }
        }
    }
}

#[cfg(test)]
#[path = "daemon_invocation_service_tests.rs"]
mod tests;
