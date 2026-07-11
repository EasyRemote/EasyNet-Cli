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

use crate::daemon::ability::builtins::governance::invocation_history::{
    record_by_request_id, ABILITY_INVOCATION_RECORD_GET,
};
use crate::daemon::ability::CallMode;
use crate::daemon::federation::client::FederationClient;
use crate::daemon::federation::peers::SharedFederatedPeers;
use crate::daemon::identity::self_identity::CanonicalSigner;
use crate::daemon::invocation::admission::admission_facade::AdmissionFacade;
use crate::daemon::invocation::admission::list_user_pubkeys::ABILITY_IDENTITY_LIST_USER_PUBKEYS;
use crate::daemon::invocation::admission::principal_lifecycle::{
    ABILITY_PRINCIPAL_ADD_KEY, ABILITY_PRINCIPAL_BIND_FIRST_KEY,
    ABILITY_PRINCIPAL_CONFIGURE_RECOVERY, ABILITY_PRINCIPAL_CREATE, ABILITY_PRINCIPAL_DELETE,
    ABILITY_PRINCIPAL_GET, ABILITY_PRINCIPAL_ISSUE_ENROLLMENT, ABILITY_PRINCIPAL_ISSUE_GRANT,
    ABILITY_PRINCIPAL_REACTIVATE, ABILITY_PRINCIPAL_RECOVER, ABILITY_PRINCIPAL_REVOKE_ENROLLMENT,
    ABILITY_PRINCIPAL_REVOKE_GRANT, ABILITY_PRINCIPAL_REVOKE_KEY, ABILITY_PRINCIPAL_ROTATE_KEY,
    ABILITY_PRINCIPAL_SUSPEND,
};
use crate::daemon::invocation::admission::quota_meter::quota_meters_request;
use crate::daemon::invocation::admission::register_device_pubkey::ABILITY_IDENTITY_REGISTER_PUBKEY;
use crate::daemon::invocation::admission::revoke_user_pubkey::ABILITY_IDENTITY_REVOKE_USER_PUBKEY;
use crate::daemon::invocation::admission::target_gate::TargetGate;
use crate::daemon::invocation::bidi::bidi_dispatcher::{
    validate_and_extract_bidi_frame0, BidiDispatcher, BidiDispatcherDeps,
};
use crate::daemon::invocation::dispatch::deps::{
    DirectoryPlane, FederationDial, IdentityPlane, RegisterPubkeyContext, RuntimePlane,
    SessionPlane,
};
use crate::daemon::invocation::dispatch::federation_wrappers::{
    ABILITY_FEDERATION_ADVERTISE_ABILITIES, ABILITY_FEDERATION_ADVERTISE_AGENT,
    ABILITY_FEDERATION_DISCOVER, ABILITY_FEDERATION_HEARTBEAT, ABILITY_FEDERATION_JOIN,
    ABILITY_FEDERATION_LIST_USER_DEVICES, ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
    ABILITY_FEDERATION_RESOLVE, ABILITY_FEDERATION_RESOLVE_KEY, ABILITY_FEDERATION_REVOKE,
    ABILITY_FEDERATION_STATUS, ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY,
    ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2, ABILITY_NAMESPACE_PROXY_RESOLVE,
    ABILITY_NAMESPACE_RESOLVE,
};
use crate::daemon::invocation::dispatch::invocation_wire::{wrap_json_response, BoxedDownStream};
use crate::daemon::invocation::dispatch::unary_dispatcher::{
    is_runtime_admin_ability, UnaryDispatcher,
};
use crate::daemon::invocation::receipts::ledger_projection::build_unary_ledger_record;
use crate::daemon::invocation::streams::stream_dispatcher::StreamDispatcher;

use crate::daemon::federation::directory::now_unix_ms;
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
    FederationSubscribeDirectory,
    FederationSubscribeDirectoryV2,
}

impl DaemonStreamRoute {
    pub(crate) const ALL: &'static [Self] = &[
        Self::FederationSubscribeDirectory,
        Self::FederationSubscribeDirectoryV2,
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
            Self::FederationSubscribeDirectory => ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY,
            Self::FederationSubscribeDirectoryV2 => ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
        }
    }

    #[must_use]
    pub(crate) const fn call_mode(self) -> CallMode {
        CallMode::Stream
    }
}

pub(crate) const DAEMON_INVOCATION_STREAM_ROUTES: &[DaemonStreamRoute] = DaemonStreamRoute::ALL;

pub(crate) fn dispatch_function_name_for_route_table(
    function_name: &str,
    envelope: Option<&easynet_axon::pb::axon::v1::Envelope>,
) -> String {
    descriptor_ref_public_name_for_callee(function_name, envelope)
        .unwrap_or_else(|| function_name.to_string())
}

fn descriptor_ref_public_name_for_callee(
    function_name: &str,
    envelope: Option<&easynet_axon::pb::axon::v1::Envelope>,
) -> Option<String> {
    let callee_ura = envelope?
        .callee
        .as_ref()
        .map(|callee| callee.ura.trim())
        .filter(|callee| !callee.is_empty())?;
    let descriptor_ref =
        easynet_axon::invocation::canonical_ability_descriptor_ref(function_name).ok()?;
    let ability_ura = crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
        &descriptor_ref,
    )
    .ok()?;
    let selector = crate::core::ura::AbilitySelector::parse(&ability_ura).ok()?;
    if selector.owner_ura() != callee_ura {
        return None;
    }
    Some(selector.public_name().to_string())
}

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
///   wrappers (resolve / canonical_invoke / revoke / heartbeat /
///   subscribe_directory) and by the future `session.open` accept
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
}

impl std::fmt::Debug for DaemonInvocationService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonInvocationService")
            .field("presence", &self.directory.presence)
            .field("admission", &self.admission)
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
    /// service, the `session.open` accept loop (PR-2), and any
    /// audit-log subscriber. The policy facade is constructed
    /// from `RealmTrustAnchor::load_or_empty(...)` at daemon boot.
    ///
    /// Session-routed calls require a `PendingDispatchMap`; use
    /// `with_pending(...)` to attach one.
    #[must_use]
    pub fn new(presence: Arc<PresenceRegistry>, admission: AdmissionFacade) -> Self {
        Self {
            admission,
            directory: DirectoryPlane {
                presence,
                advertised_agents: Arc::new(
                    crate::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore::new(),
                ),
                ability_catalog: Arc::new(
                    crate::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new(),
                ),
                federated_directory:
                    crate::daemon::federation::directory::SharedFederatedDirectoryView::default(),
                federated_bindings: None,
                subscribe_v2_heartbeat_interval_ms: 30_000,
            },
            federation: FederationDial {
                client: None,
                peers: SharedFederatedPeers::default(),
                hub_signer: None,
                allow_directory_auto_route: false,
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
                local_runtime: None,
                invocation_ledger: None,
                ability_wire: Arc::new(crate::daemon::ability::wire::AbilityWireRegistry::core()),
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

    /// Side-effect-free `invocation.record.get`: fetch one ledger record by
    /// `request_id` off the in-process ledger handle and return it as JSON.
    ///
    /// Services the out-of-process CLI's observe-my-own-request read without
    /// dispatching a second invocation (which would corrupt the audit trail)
    /// and without opening the daemon-owned redb from a second process (which
    /// redb forbids via its exclusive lock). The caller (`skip_ledger_record`)
    /// guarantees this writes no ledger row. A `null` record is a valid
    /// "not yet projected" answer, not an error.
    fn dispatch_invocation_record_get(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        #[derive(serde::Deserialize)]
        struct RecordGetRequest {
            request_id: String,
        }
        let request: RecordGetRequest = serde_json::from_slice(arguments).map_err(|err| {
            Status::invalid_argument(format!(
                "invocation.record.get: failed to decode JSON arguments: {err}"
            ))
        })?;
        let Some(ledger) = self.runtime.invocation_ledger.as_ref() else {
            return Err(Status::failed_precondition(
                "invocation.record.get: daemon has no invocation ledger wired",
            ));
        };
        let record = record_by_request_id(ledger, &request.request_id).map_err(|err| match err
            .to_string()
        {
            msg if msg.contains("request_id must not be empty") => Status::invalid_argument(msg),
            msg => Status::internal(msg),
        })?;
        wrap_json_response(&serde_json::json!({ "record": record }))
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
            self.sessions.clone(),
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

    /// Attach the correlation table for typed session dispatch results.
    /// Builder-style so existing
    /// `DaemonInvocationService::new(presence, admission)` callers
    /// stay source-compatible.
    ///
    /// PR-3 ownership (this commit). PR-2's `session.open`
    /// accept handler will share the same `Arc<PendingDispatchMap>`
    /// to call `complete(call_id, ...)` when target devices send
    /// `Result` frames back up their session streams.
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
        self.admission = self.admission.with_hub_signer(Arc::clone(&signer));
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

    #[must_use]
    pub fn with_invocation_ledger(
        mut self,
        ledger: Arc<easynet_axon::invocation::InvocationLedger>,
    ) -> Self {
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

    /// Attach the shared Axon `LocalRuntime`. Boot installs its key resolver
    /// and ledger sink before exposing the Invocation service.
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
        let started_unix_ms = now_unix_ms();
        if let Err(err) = self.admission.verify_invoke(&inner) {
            let result: Result<Response<InvokeResponse>, Status> = Err(err);
            self.record_unary_invocation(&inner, started_unix_ms, &result);
            return result;
        }
        let function = inner.function_name.as_str();
        let route_function =
            dispatch_function_name_for_route_table(function, inner.envelope.as_ref());
        // #185: meter the caller after the transport policy gate. This
        // is not an Axon runtime-admitted token; descriptor-bound local
        // dispatch still enters LocalRuntime through public Axon
        // admission below. A throttled caller is rejected here with
        // `ResourceExhausted` before any dispatch work.
        let rate_limit = if quota_meters_request(&inner) {
            match self.admission.check_quota(&inner) {
                Ok(info) => info,
                Err(err) => {
                    let result: Result<Response<InvokeResponse>, Status> = Err(err);
                    self.record_unary_invocation(&inner, started_unix_ms, &result);
                    return result;
                }
            }
        } else {
            None
        };

        // Phase 5e: flag set by the Axon-routed catch-all arm so we
        // skip the manual `record_unary_invocation` call below
        // (otherwise the LedgerSink write + manual write produce two
        // rows for the same call).
        let unary = self.unary_dispatcher();
        let mut axon_took_it = false;
        // Set by the side-effect-free `invocation.record.get` read arm: it
        // observes the ledger without dispatching, so it must write NO ledger
        // row (neither the Axon LedgerSink nor the manual `record_unary_invocation`
        // path). Distinct from `axon_took_it`, which means "Axon already wrote a
        // row" — here nothing should be written at all.
        let mut skip_ledger_record = false;
        let result = if route_function == ABILITY_INVOCATION_RECORD_GET {
            skip_ledger_record = true;
            self.dispatch_invocation_record_get(&inner.arguments)
        } else {
            match DaemonUnaryRoute::from_function(&route_function) {
                Some(DaemonUnaryRoute::FederationJoin) => {
                    unary.dispatch_federation_join(&inner.arguments)
                }
                Some(DaemonUnaryRoute::FederationAdvertiseAgent) => {
                    unary.dispatch_federation_advertise_agent(
                        &inner.arguments,
                        inner.envelope.as_ref(),
                    )
                }
                Some(DaemonUnaryRoute::FederationAdvertiseAbilities) => {
                    unary.dispatch_federation_advertise_abilities(
                        &inner.arguments,
                        inner.envelope.as_ref(),
                    )
                }
                Some(DaemonUnaryRoute::FederationHeartbeat) => {
                    unary.dispatch_federation_heartbeat(&inner.arguments)
                }
                Some(DaemonUnaryRoute::FederationStatus) => unary.dispatch_federation_status(),
                Some(DaemonUnaryRoute::FederationResolve) => {
                    unary.dispatch_federation_resolve(&inner.arguments)
                }
                Some(DaemonUnaryRoute::NamespaceResolve) => {
                    unary.dispatch_namespace_resolve(&inner.arguments).await
                }
                Some(DaemonUnaryRoute::FederationResolveKey) => {
                    unary.dispatch_federation_resolve_key(&inner.arguments)
                }
                Some(DaemonUnaryRoute::FederationDiscover) => {
                    unary.dispatch_federation_discover(&inner.arguments)
                }
                Some(DaemonUnaryRoute::FederationListUserDevices) => unary
                    .dispatch_federation_list_user_devices(
                        inner.envelope.as_ref(),
                        &inner.arguments,
                    ),
                Some(DaemonUnaryRoute::FederationProxyListUserDevices) => {
                    unary
                        .dispatch_federation_proxy_list_user_devices(
                            inner.envelope.as_ref(),
                            &inner.arguments,
                        )
                        .await
                }
                Some(DaemonUnaryRoute::NamespaceProxyResolve) => {
                    unary
                        .dispatch_namespace_proxy_resolve(inner.envelope.as_ref(), &inner.arguments)
                        .await
                }
                Some(DaemonUnaryRoute::FederationRevoke) => {
                    unary.dispatch_federation_revoke(&inner.arguments)
                }
                Some(DaemonUnaryRoute::IdentityRegisterPubkey) => {
                    unary.dispatch_register_device_pubkey(inner.envelope.as_ref(), &inner.arguments)
                }
                Some(DaemonUnaryRoute::IdentityRevokeUserPubkey) => {
                    unary.dispatch_revoke_user_pubkey(inner.envelope.as_ref(), &inner.arguments)
                }
                Some(DaemonUnaryRoute::IdentityListUserPubkeys) => {
                    unary.dispatch_list_user_pubkeys(&inner.arguments)
                }
                Some(
                    DaemonUnaryRoute::PrincipalCreate
                    | DaemonUnaryRoute::PrincipalBindFirstKey
                    | DaemonUnaryRoute::PrincipalAddKey
                    | DaemonUnaryRoute::PrincipalRotateKey
                    | DaemonUnaryRoute::PrincipalRevokeKey
                    | DaemonUnaryRoute::PrincipalConfigureRecovery
                    | DaemonUnaryRoute::PrincipalRecover
                    | DaemonUnaryRoute::PrincipalSuspend
                    | DaemonUnaryRoute::PrincipalReactivate
                    | DaemonUnaryRoute::PrincipalDelete
                    | DaemonUnaryRoute::PrincipalIssueEnrollment
                    | DaemonUnaryRoute::PrincipalRevokeEnrollment
                    | DaemonUnaryRoute::PrincipalIssueGrant
                    | DaemonUnaryRoute::PrincipalRevokeGrant
                    | DaemonUnaryRoute::PrincipalGet,
                ) => unary.dispatch_principal_lifecycle(&route_function, &inner.arguments),
                None if DaemonStreamRoute::from_function(&route_function).is_some() => {
                    Err(Status::invalid_argument(format!(
                        "{route_function} is a server-stream ability and must be invoked via InvokeStream, not Invoke"
                    )))
                }
                // `runtime.*` are node-internal admin handshakes hosted by the
                // receiving daemon, not owner-routed abilities. Dispatch them
                // directly on the LocalRuntime so a hub-owner callee URA does
                // not get rejected as `NXDOMAIN owner is not online`.
                None if is_runtime_admin_ability(&route_function) => {
                    unary
                        .dispatch_runtime_admin_ability(&inner, &route_function)
                        .await
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
                None => {
                    let (r, axon) = unary.dispatch_local_rpc_selected_route(&inner).await;
                    axon_took_it = axon;
                    r
                }
            }
        };
        if !axon_took_it && !skip_ledger_record {
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
    /// snapshot plus subsequent presence transitions.
    async fn invoke_stream(
        &self,
        request: Request<InvokeServerStreamRequest>,
    ) -> Result<Response<Self::InvokeStreamStream>, Status> {
        let inner = request.into_inner();
        self.admission.verify_invoke_stream(&inner)?;
        let function = inner.function_name.as_str();
        let route_function =
            dispatch_function_name_for_route_table(function, inner.envelope.as_ref());

        let streams = self.stream_dispatcher();
        match DaemonStreamRoute::from_function(&route_function) {
            Some(DaemonStreamRoute::FederationSubscribeDirectory) => {
                streams.dispatch_subscribe_directory_initial()
            }
            Some(DaemonStreamRoute::FederationSubscribeDirectoryV2) => {
                streams.dispatch_subscribe_directory_v2()
            }
            None => streams.dispatch_selected_route(&inner).await,
        }
    }

    type InvokeBidiStream = BoxedDownStream<InvokeBidiDown>;

    /// Spec §2.1 reference. Routes by frame-0
    /// `EnvelopeOpen.target.ability_name`:
    ///
    /// - `session.open` accepts a long-lived reverse channel.
    /// - registered builtin/plugin bidi abilities use their declared wire
    ///   profile.
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
