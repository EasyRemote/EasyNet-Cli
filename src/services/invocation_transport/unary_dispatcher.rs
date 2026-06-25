// EasyNet Daemon — Invoke (Unary) Dispatcher
// ============================================
//
// File: src/services/invocation_transport/unary_dispatcher.rs
// Description: Owns every unary `Invoke` routing arm the daemon serves
//              after transport policy + quota (commit-plan-2 Axis E / E2):
//
//                * federation prelude writes — join / advertise_agent /
//                  advertise_abilities / heartbeat
//                * federation reads — resolve / resolve_key / discover /
//                  list_user_devices (+ backend proxy variants) / revoke
//                * RFC-005 namespace.resolve (+ backend proxy variant)
//                * identity verbs — <self>.register_device_pubkey /
//                  revoke_user_pubkey / list_user_pubkeys
//                * runtime.* node-internal admin handshakes
//                * the resolve-first LocalRuntime catch-all
//
//              The forward_invoke family stays on the service until its
//              own extraction (E2c) — it spans sessions/escalation and
//              the peer dial plane.
//
//              Like StreamDispatcher, this type is a pure consumer of
//              the dependency planes plus the `TargetGate`; it never
//              sees the tonic service.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use std::collections::BTreeSet;
use std::sync::Arc;

use tonic::{Response, Status};

use easynet_axon::pb::axon::v1::{
    Envelope, InvokeBidiDown, InvokeRequest, InvokeResponse, ResponseHeader,
};

use std::collections::{BTreeMap, HashMap};

use tokio::sync::mpsc;

use crate::services::federation_client::FederationClient;
use crate::services::federation_directory::now_unix_ms;
use crate::services::invocation_transport::admission_facade::AdmissionFacade;
use crate::services::invocation_transport::deps::{
    DirectoryPlane, FederationDial, IdentityPlane, RuntimePlane, SessionPlane,
};
use crate::services::invocation_transport::descriptor_binding::RuntimeBoundAbility;
use crate::services::invocation_transport::federation_wrappers;
use crate::services::invocation_transport::federation_wrappers::{
    ABILITY_FEDERATION_FORWARD_INVOKE, ABILITY_FEDERATION_LIST_USER_DEVICES,
    ABILITY_NAMESPACE_RESOLVE,
};
use crate::services::invocation_transport::invocation_wire::{
    dispatch_key_mismatch_message, parse_json_args, status_from_axon_invoke_error,
    status_from_dispatch_key_mismatch, target_ura_from_envelope, wrap_json_response,
    BoxedDownStream, FEDERATION_RESULT_CONTENT_TYPE,
};
use crate::services::invocation_transport::invoke_remote_initiator::{
    build_carrier_v1_dispatch_frame, build_invoke_remote_dispatch_frame,
    build_invoke_remote_terminal_frame, decode_inner_payload, invoke_remote_inband_error_response,
    InnerPayload, InvokeRemoteDispatchFrameRequest, InvokeRemoteDown, RequestOutcome,
    SessionContentEnvelope, SessionRequestError,
};
use crate::services::invocation_transport::list_user_pubkeys::handle as handle_list_user_pubkeys;
use crate::services::invocation_transport::peer_envelope_signer::{
    build_peer_envelope, sign_peer_request_envelope,
};
use crate::services::invocation_transport::register_device_pubkey::handle as handle_register_device_pubkey;
use crate::services::invocation_transport::register_device_pubkey::parse_realm_from_ura;
use crate::services::invocation_transport::revoke_user_pubkey::handle as handle_revoke_user_pubkey;
use crate::services::invocation_transport::route_resolver::{
    DelegatedInvokeRoute, SelectedInvokeRoute,
};
use crate::services::invocation_transport::target_gate::{
    envelope_with_selected_callee, route_negative_status, route_owner_mismatch_message,
    route_profile_blocked_status, selected_host_unavailable_message, TargetGate,
    ROUTE_SELECTED_REMOTE_HOST_CODE,
};
use crate::services::pending_dispatch::DispatchResult;
use crate::services::session_failure::SessionFailure;
use tokio_stream::wrappers::ReceiverStream;

fn rpc_dispatch_outcome_response(
    ability: &str,
    failure_prefix: &str,
    outcome: crate::runtime::axon_bridge::dispatch_shim::RpcDispatchOutcome,
) -> (Result<Response<InvokeResponse>, Status>, bool) {
    let crate::runtime::axon_bridge::dispatch_shim::RpcDispatchOutcome {
        invocation_id,
        payload_bytes,
        error,
        admission_receipt,
        terminal_receipt,
        ..
    } = outcome;
    let axon_started = invocation_id.is_some();
    let response = match error {
        None => Ok(Response::new(InvokeResponse {
            header: invocation_id.map(|request_id| ResponseHeader {
                request_id,
                status: "completed".to_string(),
                ..ResponseHeader::default()
            }),
            result: payload_bytes,
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            admission_receipt: admission_receipt
                .as_ref()
                .map(easynet_axon::invocation::wire::receipt_to_wire),
            terminal_receipt: terminal_receipt
                .as_ref()
                .map(easynet_axon::invocation::wire::receipt_to_wire),
            ..InvokeResponse::default()
        })),
        Some(err) => Err(Status::failed_precondition(format!(
            "local-rpc axon dispatch: {failure_prefix} `{ability}` failed: {err}"
        ))),
    };
    (response, axon_started)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UnarySelfTargetSubject {
    Explicit(String),
    DescriptorDefault(String),
}

impl UnarySelfTargetSubject {
    fn from_optional(
        explicit_subject_ura: Option<&str>,
        descriptor_default_subject_ura: String,
    ) -> Result<Self, Status> {
        match explicit_subject_ura
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(subject) => {
                Self::validate(subject).map_err(|err| {
                    Status::invalid_argument(format!(
                        "self-targeted dispatch subject `{subject}` is not a valid URA: {err}"
                    ))
                })?;
                Ok(Self::Explicit(subject.to_string()))
            }
            None => {
                Self::validate(&descriptor_default_subject_ura).map_err(|err| {
                    Status::failed_precondition(format!(
                        "self-targeted dispatch descriptor default subject `{descriptor_default_subject_ura}` is invalid: {err}"
                    ))
                })?;
                Ok(Self::DescriptorDefault(descriptor_default_subject_ura))
            }
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Self::Explicit(subject) | Self::DescriptorDefault(subject) => subject,
        }
    }

    fn validate(subject_ura: &str) -> Result<(), String> {
        crate::ura::parse_ura(subject_ura)
            .map(|_| ())
            .map_err(|err| err.to_string())
    }
}

/// Hard ceiling on one presence-dispatch round-trip: the time between
/// pushing a `Dispatch` frame down a device's `<self>.session` and
/// that device's `Result` frame completing the pending entry. The
/// presence-offline watcher already fail-fasts waiters whose session
/// drops; this deadline covers every other never-reply shape (a
/// device that accepted the frame and wedged, a drain-only presence
/// entry) so a unary caller gets a structured error instead of
/// hanging for the life of the connection.
pub(crate) const PRESENCE_DISPATCH_REPLY_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(60);

/// Unary `Invoke` routing surface. Cheap per-call construction: every
/// plane and the gate are `Arc`-shaped.
#[derive(Clone)]
pub(crate) struct UnaryDispatcher {
    admission: AdmissionFacade,
    directory: DirectoryPlane,
    federation: FederationDial,
    sessions: SessionPlane,
    identity: IdentityPlane,
    runtime: RuntimePlane,
    gate: TargetGate,
}

impl UnaryDispatcher {
    pub(crate) fn new(
        admission: AdmissionFacade,
        directory: DirectoryPlane,
        federation: FederationDial,
        sessions: SessionPlane,
        identity: IdentityPlane,
        runtime: RuntimePlane,
        gate: TargetGate,
    ) -> Self {
        Self {
            admission,
            directory,
            federation,
            sessions,
            identity,
            runtime,
            gate,
        }
    }

    /// Axon descriptor subjects are concrete executors/resources, not
    /// namespace owners. A selected hub route is still addressed to the
    /// hub URA on the EasyNet routing plane, but local Axon admission must
    /// be anchored to this daemon's concrete device identity.
    fn default_self_target_subject_ura(&self, callee_ura: &str) -> Result<String, Status> {
        let parsed = crate::ura::parse_ura(callee_ura).map_err(|err| {
            Status::invalid_argument(format!(
                "self-targeted dispatch callee `{callee_ura}` is not a valid URA: {err}"
            ))
        })?;
        if parsed.kind == crate::ura::URAKind::Hub {
            let daemon_ura = self.admission.daemon_ura().ok_or_else(|| {
                Status::failed_precondition(
                    "self-targeted hub dispatch requires this daemon's concrete device URA",
                )
            });
            let daemon_ura = daemon_ura?;
            let daemon_ref = crate::ura::parse_ura(daemon_ura).map_err(|err| {
                Status::failed_precondition(format!(
                    "self-targeted hub dispatch daemon URA `{daemon_ura}` is invalid: {err}"
                ))
            })?;
            if daemon_ref.kind != crate::ura::URAKind::Device {
                return Err(Status::failed_precondition(format!(
                    "self-targeted hub dispatch daemon URA `{daemon_ura}` must be a device URA, \
                     got {:?}",
                    daemon_ref.kind
                )));
            }
            return Ok(daemon_ura.to_string());
        }
        Ok(callee_ura.to_string())
    }

    pub(crate) fn dispatch_federation_join(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::JoinRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_join(&request);
        wrap_json_response(&response)
    }

    pub(crate) fn dispatch_federation_advertise_agent(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::AdvertiseAgentRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_advertise_agent(
            &request,
            Some(self.directory.advertised_agents.as_ref()),
        );
        wrap_json_response(&response)
    }

    pub(crate) fn dispatch_federation_advertise_abilities(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::AdvertiseAbilitiesRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_advertise_abilities(
            &request,
            Some(self.directory.ability_catalog.as_ref()),
        );
        wrap_json_response(&response)
    }

    pub(crate) fn dispatch_federation_heartbeat(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::HeartbeatRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_heartbeat(
            &request,
            &self.directory.presence,
            Some(self.directory.ability_catalog.as_ref()),
            now_unix_ms(),
        );
        wrap_json_response(&response)
    }

    /// Unary `Invoke` catch-all backed by RFC-005 namespace.resolve
    /// followed by Axon's `LocalRuntime`.
    ///
    /// Returns `(response, axon_took_it)`. The caller in
    /// [`Self::invoke`] consults `axon_took_it` to decide whether
    /// the post-dispatch `record_unary_invocation` should fire:
    ///   * `true` — Axon actually started an invocation and returned
    ///     its `invocation_id`; Axon's `LedgerSink` wrote the
    ///     canonical row on the terminal event, so the manual record
    ///     would only produce a duplicate keyed by `request_id`.
    ///   * `false` — no handler ran (runtime missing or ability
    ///     unknown), so the manual failed row may be recorded.
    async fn resolve_local_rpc_route(
        &self,
        request: &InvokeRequest,
    ) -> Result<SelectedInvokeRoute, Status> {
        let target_ura = local_invoke_target_ura(request)?;
        let ability = request.function_name.trim();
        if ability.is_empty() {
            return Err(Status::invalid_argument(
                "Invoke request missing function_name for namespace.resolve",
            ));
        }

        let selected_route = self
            .gate
            .route_resolver()
            .await
            .resolve_route(&target_ura, ability)
            .map_err(route_negative_status)?;

        if !selected_route.is_authoritative_local_or_better() {
            return Err(route_profile_blocked_status(&selected_route));
        }
        Ok(selected_route)
    }

    pub(crate) async fn dispatch_local_rpc_selected_route(
        &self,
        request: &InvokeRequest,
    ) -> (Result<Response<InvokeResponse>, Status>, bool) {
        let ability = request.function_name.trim();
        let arguments = request.arguments.as_slice();
        let selected_route = match self.resolve_local_rpc_route(request).await {
            Ok(route) => route,
            Err(status) => return (Err(status), false),
        };
        // step-4 / T2.1b: locality is the daemon's decision, not the
        // caller's. A resolver-selected remote host dispatches through
        // that device's `<self>.session`. Like the federation-wrapper
        // arms, this is a service-handler path — the manual unary
        // record runs (axon_took_it = false); the executing device's
        // own runtime holds the canonical ledger row.
        if !self
            .gate
            .matches_self_target_ura(&selected_route.execution_host_ura)
            .await
        {
            return (
                self.dispatch_remote_rpc_selected_route(request, &selected_route)
                    .await,
                false,
            );
        }
        let Some(runtime) = self.runtime.local_runtime.as_ref() else {
            return (
                Err(Status::failed_precondition(format!(
                    "easynet-daemon: ability `{ability}` cannot run because Axon LocalRuntime \
                     is not wired at boot"
                ))),
                false,
            );
        };
        let selected_ability_ura = selected_route.ability_ura.clone();
        let selected_descriptor_ref = match RuntimeBoundAbility::from_selected_route(
            "easynet-daemon",
            runtime,
            &selected_route,
        )
        .await
        .and_then(|bound_ability| {
            bound_ability.descriptor_ref_for_mode(
                "easynet-daemon",
                &selected_route.callee_ura,
                easynet_axon::invocation::CallMode::Rpc,
                Some(&selected_route.route_ura),
            )
        }) {
            Ok(ref_) => ref_.into_descriptor_ref(),
            Err(status) => return (Err(status), false),
        };
        crate::op_event!(
            component = daemon_invocation,
            kind = dispatch_local_rpc_selected_route,
            ability = ability,
            dispatch_ability = selected_ability_ura.as_str(),
            local_dispatch_key = selected_route.dispatch_name.as_str(),
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            route_ura = selected_route.route_ura.as_str(),
        );
        let signed_ability_ura =
            match crate::runtime::axon_bridge::descriptor_ref::ability_ura_for_wire(
                &selected_route.callee_ura,
                ability,
            ) {
                Ok(ability_ura) => ability_ura,
                Err(err) => {
                    return (
                        Err(Status::invalid_argument(format!(
                            "Invoke: signed ability `{ability}` is not valid for callee `{}`: {err}",
                            selected_route.callee_ura
                        ))),
                        false,
                    );
                }
            };
        if signed_ability_ura != selected_ability_ura {
            return (
                Err(status_from_dispatch_key_mismatch(
                    "Invoke",
                    ability,
                    &selected_ability_ura,
                    &selected_route.route_ura,
                )),
                false,
            );
        }
        let wire = match request.envelope.clone() {
            Some(envelope) => {
                crate::runtime::axon_bridge::dispatch_shim::external_signed_from_wire_parts(
                    envelope,
                    selected_descriptor_ref,
                    arguments.to_vec(),
                    request.metadata.clone(),
                )
            }
            None => Err(Box::new(
                easynet_axon::invocation::AxonError::invalid_argument(
                    "Invoke request missing envelope",
                ),
            )),
        };
        let wire = match wire {
            Ok(wire) => wire,
            Err(err) => {
                return (
                    Err(status_from_axon_invoke_error("Invoke", ability, *err)),
                    false,
                );
            }
        };
        let outcome =
            crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc_admitted(runtime, wire).await;
        rpc_dispatch_outcome_response(ability, "ability", outcome)
    }

    /// Dispatch a node-internal `runtime.*` admin ability directly on this
    /// daemon's `LocalRuntime`, bypassing `namespace.resolve`.
    ///
    /// `runtime.*` abilities (e.g. `runtime.bootstrap_self_identity`) are
    /// node-internal control-plane handshakes hosted by whichever daemon
    /// receives them — exactly like `<self>.*`. Their wire `callee` is the
    /// caller's *claimed* authority owner (a backend sets it to the hub
    /// URA), which is not a routable owner on this daemon's presence
    /// directory. Routing them through owner resolution therefore returns
    /// a spurious `NXDOMAIN owner is not online`. The admin handler is
    /// registered in the runtime under the ability name verbatim, so we
    /// dispatch by name and let the SDK admin surface enforce its own
    /// authority checks.
    pub(crate) async fn dispatch_runtime_admin_ability(
        &self,
        request: &InvokeRequest,
    ) -> Result<Response<InvokeResponse>, Status> {
        let ability = request.function_name.trim();
        let Some(runtime) = self.runtime.local_runtime.as_ref() else {
            return Err(Status::failed_precondition(format!(
                "easynet-daemon: runtime admin ability `{ability}` cannot run because Axon \
                 LocalRuntime is not wired at boot"
            )));
        };
        if runtime.ability_options(ability).await.is_none() {
            return Err(Status::not_found(format!(
                "easynet-daemon: runtime admin ability `{ability}` is not installed in Axon \
                 LocalRuntime on this node"
            )));
        }
        // Admin abilities are installed bare (no owner, no descriptor
        // proof). Dispatch by the bare registered name — the descriptor-
        // bound wire path would canonicalize to a device-owned URA the
        // runtime never registered and demand a proof the handler lacks.
        let outcome = crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc_local_admin_bare(
            runtime,
            ability,
            request.arguments.clone(),
        )
        .await;
        rpc_dispatch_outcome_response(ability, "runtime admin ability", outcome).0
    }

    pub(crate) fn dispatch_register_device_pubkey(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let ctx = self.identity.register_pubkey.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "<self>.register_device_pubkey: this daemon was booted without the trust-write \
                 surface (use `with_register_pubkey(...)` at boot to enable). PR-7 production \
                 daemons always wire this; an unwired daemon is a smoke-test or fixture build.",
            )
        })?;
        let body = handle_register_device_pubkey(
            arguments,
            &ctx.daemon_realm,
            &ctx.trust_anchor_path,
            &ctx.cell,
        )?;
        Ok(Response::new(InvokeResponse {
            result: body,
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            ..InvokeResponse::default()
        }))
    }

    /// DEC-EU §revocation. Same trust-write ctx the register ability
    /// uses; the revoke surface only mutates user-role entries.
    pub(crate) fn dispatch_revoke_user_pubkey(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let ctx = self.identity.register_pubkey.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "<self>.revoke_user_pubkey: this daemon was booted without the trust-write \
                 surface (use `with_register_pubkey(...)` at boot to enable).",
            )
        })?;
        let body = handle_revoke_user_pubkey(
            arguments,
            &ctx.daemon_realm,
            &ctx.trust_anchor_path,
            &ctx.cell,
        )?;
        Ok(Response::new(InvokeResponse {
            result: body,
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            ..InvokeResponse::default()
        }))
    }

    /// DEC-EU §multi-host-list. Read-only inventory of user-role
    /// pubkeys. Uses the same cell as register/revoke so list
    /// results always agree with the in-memory authoritative state
    /// admission consults.
    pub(crate) fn dispatch_list_user_pubkeys(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let ctx = self.identity.register_pubkey.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "<self>.list_user_pubkeys: this daemon was booted without the trust \
                 surface; no listing available.",
            )
        })?;
        let body = handle_list_user_pubkeys(arguments, &ctx.cell)?;
        Ok(Response::new(InvokeResponse {
            result: body,
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            ..InvokeResponse::default()
        }))
    }

    pub(crate) fn dispatch_federation_resolve(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::ResolveRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_resolve(
            &request,
            &self.directory.presence,
            Some(self.directory.advertised_agents.as_ref()),
            Some(self.directory.ability_catalog.as_ref()),
            self.admission.daemon_ura(),
        );
        wrap_json_response(&response)
    }

    pub(crate) async fn dispatch_namespace_resolve(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: serde_json::Value = parse_json_args(arguments)?;
        let response = self
            .gate
            .route_resolver()
            .await
            .resolve_query_json(&request);
        wrap_json_response(&response)
    }

    /// **PR-N2 commit 2/N**. Peer-side `federation.resolve_key`
    /// dispatch. Reads the daemon's `SharedTrustAnchor` (so a
    /// SIGHUP-triggered `realm-trust.toml` reload is reflected
    /// without a restart) and returns the matching
    /// `public_key_b64` for the requested URA.
    ///
    /// On miss we surface `Status::not_found` so the calling
    /// `FederatedKeyResolver` can distinguish "URA is not in
    /// this hub's trust set" from a network or admission
    /// failure (which arrive as `unavailable` /
    /// `permission_denied`). The resolver then maps both into
    /// `CALLER_KEY_NOT_FOUND` for INV-4 fail-closed admission, but
    /// the wire-level distinction is useful for operator audit
    /// and matches the rest of the federation.* surface where
    /// `not_found` means "no entry" and `failed_precondition`
    /// means "entry present but unusable".
    pub(crate) fn dispatch_federation_resolve_key(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::ResolveKeyRequest = parse_json_args(arguments)?;
        let trust_anchor = self.admission.trust_anchor_snapshot();
        match federation_wrappers::handle_resolve_key(&request, &trust_anchor) {
            Some(response) => wrap_json_response(&response),
            None => Err(Status::not_found(format!(
                "federation.resolve_key: agent_ura `{}` not in this hub's trust set",
                request.agent_ura
            ))),
        }
    }

    /// **PR-N3 commit N3-4 + N3-N4 dispatch wire**. Cross-realm
    /// directory lookup dispatch. Reads the daemon-wide
    /// `SharedFederatedDirectoryView` cell snapshot, fans out
    /// across federated peers per spec §3.2 (lex tie-break,
    /// dedupe by agent_ura), returns matching `DirectoryEntry`
    /// list.
    ///
    /// When the request carries a `local_user_id` AND the
    /// daemon has both a `FederatedBindingsStore` and a
    /// `session_realm` wired, the dispatch routes through
    /// `handle_discover_with_user_filter` so cross-realm
    /// entries are filtered by the user's binding state per
    /// PR-N4 INV-5 privacy default. Otherwise (no user id or
    /// no bindings store), routes through the unfiltered
    /// `handle_discover` for backwards-compat with operator /
    /// audit query callers.
    ///
    /// Pure read; no I/O — single-realm daemons that haven't
    /// accumulated any peer views just return an empty
    /// response, gracefully degrading to local-only behaviour.
    pub(crate) fn dispatch_federation_discover(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::DiscoverRequest = parse_json_args(arguments)?;
        let response = match (
            request.local_user_id.as_deref(),
            self.directory.federated_bindings.as_ref(),
            self.identity.session_realm.as_deref(),
        ) {
            (Some(_user_id), Some(bindings), Some(realm)) => {
                let resolver = crate::runtime::keyring::resolver::FederatedUserResolver::new(
                    realm,
                    std::sync::Arc::clone(bindings),
                );
                federation_wrappers::handle_discover_with_user_filter(
                    &request,
                    &self.directory.federated_directory,
                    &resolver,
                )
            }
            _ => {
                federation_wrappers::handle_discover(&request, &self.directory.federated_directory)
            }
        };
        wrap_json_response(&response)
    }

    /// **PR-N3 commit N3-5**. Hub-side projection of local
    /// presence-registry entries for a given realm. Spec §3.5
    /// admission filter: only callers whose URA is in the local
    /// trust anchor with `role = Hub` may invoke this. Other
    /// roles (Backend, Device) are rejected with
    /// `Status::permission_denied`. The general transport policy gate
    /// has already accepted the call for routing; this filter narrows
    /// to the hub-only sub-surface.
    ///
    /// Loopback bypass: the daemon's own URA is admitted into
    /// every dispatch arm regardless of role, so a hub-mode
    /// daemon listing its own users from a CLI on the same
    /// machine works without configuring itself as a Hub trust
    /// entry.
    pub(crate) fn dispatch_federation_list_user_devices(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        // Spec §3.5 admission filter — caller must be a Hub-role
        // peer (or the daemon itself).
        let caller_ura = caller_envelope
            .and_then(|env| env.caller.as_ref())
            .map(|c| c.ura.as_str())
            .ok_or_else(|| {
                Status::invalid_argument(
                    "federation.list_user_devices: missing caller envelope.caller.ura",
                )
            })?;

        let trust_anchor = self.admission.trust_anchor_snapshot();
        let is_hub_role = trust_anchor.lookup(caller_ura).is_some_and(|entry| {
            matches!(
                entry.role,
                crate::services::realm_trust_anchor::TrustedAgentRole::Hub
            )
        });
        let is_loopback = self
            .admission
            .daemon_ura()
            .is_some_and(|self_ura| self_ura == caller_ura);
        if !(is_hub_role || is_loopback) {
            return Err(Status::permission_denied(format!(
                "federation.list_user_devices: caller `{caller_ura}` is not a hub-role peer; \
                 only trusted hubs and the daemon itself may enumerate user devices"
            )));
        }

        let request: federation_wrappers::ListUserDevicesRequest = parse_json_args(arguments)?;
        let response =
            federation_wrappers::handle_list_user_devices(&request, &self.directory.presence);
        wrap_json_response(&response)
    }

    pub(crate) fn require_backend_or_loopback_proxy_caller(
        &self,
        caller_envelope: Option<&Envelope>,
        ability_name: &str,
    ) -> Result<(), Status> {
        let caller_ura = caller_envelope
            .and_then(|env| env.caller.as_ref())
            .map(|c| c.ura.as_str())
            .ok_or_else(|| {
                Status::invalid_argument(format!(
                    "{ability_name}: missing caller envelope.caller.ura"
                ))
            })?;

        let trust_anchor = self.admission.trust_anchor_snapshot();
        let trusted_entry = trust_anchor.lookup(caller_ura);
        let is_backend_role = trusted_entry.is_some_and(|entry| {
            matches!(
                entry.role,
                crate::services::realm_trust_anchor::TrustedAgentRole::Backend
            )
        });
        let is_local_hub_identity = self
            .identity
            .session_realm
            .as_deref()
            .is_some_and(|realm| crate::ura::hub_ura(realm) == caller_ura);
        let is_local_hub_role = is_local_hub_identity
            && trusted_entry.is_some_and(|entry| {
                matches!(
                    entry.role,
                    crate::services::realm_trust_anchor::TrustedAgentRole::Backend
                        | crate::services::realm_trust_anchor::TrustedAgentRole::Hub
                )
            });
        let is_loopback = self
            .admission
            .daemon_ura()
            .is_some_and(|self_ura| self_ura == caller_ura);
        if !(is_backend_role || is_local_hub_role || is_loopback) {
            return Err(Status::permission_denied(format!(
                "{ability_name}: caller `{caller_ura}` is not the local backend; \
                 only the backend and daemon loopback may proxy peer calls"
            )));
        }
        Ok(())
    }

    /// Daemon-local caller-side path for user-scoped peer device
    /// enumeration. The backend passes the exact peer hub URLs from
    /// `user_peer_hubs`; the daemon fans out to each via its
    /// existing cross-hub transport, stamps the merge-boundary
    /// metadata (`origin_realm`, `hub_endpoint`), and returns a
    /// typed `DirectoryEntry` list. This keeps peer dial / trust /
    /// signing inside the daemon and prevents the Go backend from
    /// growing its own cross-hub stack.
    pub(crate) async fn dispatch_federation_proxy_list_user_devices(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        self.require_backend_or_loopback_proxy_caller(
            caller_envelope,
            "federation.proxy_list_user_devices",
        )?;

        let request: federation_wrappers::ProxyListUserDevicesRequest = parse_json_args(arguments)?;
        let realm = request.realm.trim();
        if realm.is_empty() {
            return Err(Status::invalid_argument(
                "federation.proxy_list_user_devices: realm is required",
            ));
        }

        let Some(client) = self.federation.client.as_ref() else {
            return wrap_json_response(&federation_wrappers::ProxyListUserDevicesResponse {
                devices: Vec::new(),
            });
        };

        let peer_hub_urls: Vec<String> = request
            .peer_hub_urls
            .into_iter()
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if peer_hub_urls.is_empty() {
            return wrap_json_response(&federation_wrappers::ProxyListUserDevicesResponse {
                devices: Vec::new(),
            });
        }

        let inner_arguments = serde_json::to_vec(&federation_wrappers::ListUserDevicesRequest {
            realm: realm.to_string(),
        })
        .map_err(|err| {
            Status::internal(format!(
                "federation.proxy_list_user_devices: encode peer request: {err}"
            ))
        })?;

        let trust_anchor = self.admission.trust_anchor_snapshot();
        let local_realm = self.identity.session_realm.as_deref();
        let mut fanout = FuturesUnordered::new();
        for peer_hub_url in peer_hub_urls {
            let Some(peer_entry) = trust_anchor.lookup_peer_hub(&peer_hub_url).cloned() else {
                crate::op_event!(
                    component = daemon_invocation,
                    kind = proxy_list_user_devices_skip_untrusted_peer,
                    peer_hub_url = peer_hub_url,
                );
                continue;
            };
            let Some(peer_realm) = peer_entry.origin_realm.clone() else {
                crate::op_event!(
                    component = daemon_invocation,
                    kind = proxy_list_user_devices_skip_peer_missing_origin_tenant,
                    peer_hub_url = peer_hub_url,
                );
                continue;
            };
            let client = Arc::clone(client);
            let mut peer_request = InvokeRequest {
                envelope: Some(build_peer_envelope(
                    caller_envelope,
                    &peer_entry.agent_ura,
                    local_realm,
                )?),
                function_name: ABILITY_FEDERATION_LIST_USER_DEVICES.to_string(),
                arguments: inner_arguments.clone(),
                ..InvokeRequest::default()
            };
            if let Some(envelope) = peer_request.envelope.as_mut() {
                sign_peer_request_envelope(
                    envelope,
                    &peer_request.function_name,
                    &peer_request.arguments,
                    local_realm,
                    self.federation.hub_signing_seed.as_ref(),
                )?;
            }
            fanout.push(async move {
                match client.forward_invoke(&peer_hub_url, peer_request).await {
                    Ok(response) => {
                        let mut body: federation_wrappers::ListUserDevicesResponse =
                            serde_json::from_slice(&response.result).map_err(|err| {
                                format!(
                                    "decode peer {peer_hub_url} list_user_devices response: {err}"
                                )
                            })?;
                        for device in &mut body.devices {
                            device.origin_realm = Some(peer_realm.clone());
                            device.hub_endpoint = Some(peer_hub_url.clone());
                        }
                        Ok(body.devices)
                    }
                    Err(err) => Err(format!(
                        "dial peer {peer_hub_url} for list_user_devices failed: {err}"
                    )),
                }
            });
        }

        let mut devices = Vec::new();
        while let Some(result) = fanout.next().await {
            match result {
                Ok(mut entries) => devices.append(&mut entries),
                Err(err) => {
                    let err_msg = err.to_string();
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = proxy_list_user_devices_fanout_error,
                        error = err_msg,
                    );
                }
            }
        }
        devices.sort_by(|a, b| {
            a.hub_endpoint
                .as_deref()
                .unwrap_or("")
                .cmp(b.hub_endpoint.as_deref().unwrap_or(""))
                .then_with(|| a.agent_ura.cmp(&b.agent_ura))
        });

        wrap_json_response(&federation_wrappers::ProxyListUserDevicesResponse { devices })
    }

    pub(crate) async fn dispatch_namespace_proxy_resolve(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        self.require_backend_or_loopback_proxy_caller(caller_envelope, "namespace.proxy_resolve")?;

        let request: federation_wrappers::NamespaceProxyResolveRequest =
            parse_json_args(arguments)?;
        let Some(client) = self.federation.client.as_ref() else {
            return wrap_json_response(&namespace_proxy_resolve_empty_answer(&request));
        };

        let peer_hub_urls = sorted_non_empty_urls(request.peer_hub_urls.clone());
        if peer_hub_urls.is_empty() {
            return wrap_json_response(&namespace_proxy_resolve_empty_answer(&request));
        }

        let inner_arguments = namespace_proxy_resolve_peer_arguments(&request)?;
        let trust_anchor = self.admission.trust_anchor_snapshot();
        let local_realm = self.identity.session_realm.as_deref();
        let mut fanout = FuturesUnordered::new();
        for peer_hub_url in peer_hub_urls {
            let Some(peer_entry) = trust_anchor.lookup_peer_hub(&peer_hub_url).cloned() else {
                crate::op_event!(
                    component = daemon_invocation,
                    kind = namespace_proxy_resolve_skip_untrusted_peer,
                    peer_hub_url = peer_hub_url,
                );
                continue;
            };
            let client = Arc::clone(client);
            let mut peer_request = InvokeRequest {
                envelope: Some(build_peer_envelope(
                    caller_envelope,
                    &peer_entry.agent_ura,
                    local_realm,
                )?),
                function_name: ABILITY_NAMESPACE_RESOLVE.to_string(),
                arguments: inner_arguments.clone(),
                ..InvokeRequest::default()
            };
            if let Some(envelope) = peer_request.envelope.as_mut() {
                sign_peer_request_envelope(
                    envelope,
                    &peer_request.function_name,
                    &peer_request.arguments,
                    local_realm,
                    self.federation.hub_signing_seed.as_ref(),
                )?;
            }
            fanout.push(async move {
                match client.forward_invoke(&peer_hub_url, peer_request).await {
                    Ok(response) => {
                        let body: serde_json::Value = serde_json::from_slice(&response.result)
                            .map_err(|err| {
                                format!(
                                    "decode peer {peer_hub_url} namespace.resolve response: {err}"
                                )
                            })?;
                        Ok(body)
                    }
                    Err(err) => Err(format!(
                        "dial peer {peer_hub_url} for namespace.resolve failed: {err}"
                    )),
                }
            });
        }

        let mut peer_answers = Vec::new();
        while let Some(result) = fanout.next().await {
            match result {
                Ok(answer) => peer_answers.push(answer),
                Err(err) => {
                    let err_msg = err.to_string();
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = namespace_proxy_resolve_fanout_error,
                        error = err_msg,
                    );
                }
            }
        }

        wrap_json_response(&namespace_proxy_resolve_merge_answer(
            &request,
            peer_answers,
        ))
    }

    pub(crate) fn dispatch_federation_revoke(
        &self,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let request: federation_wrappers::RevokeRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_revoke(
            &request,
            &self.directory.presence,
            Some(self.directory.advertised_agents.as_ref()),
        );
        wrap_json_response(&response)
    }
    /// RFC-005 route-first `federation.forward_invoke` dispatch:
    ///
    /// 1. Decode the inner payload and validate that its canonical
    ///    `ability_ura` belongs to the supplied `target_ura`.
    /// 2. Ask `namespace.resolve` for a local `FinalRoute`.
    /// 3. If a route is selected locally, dispatch only by selected
    ///    `execution_host_ura`, `callee_ura`, and `dispatch_name`.
    ///    `target_ura` remains an owner consistency proof, never an
    ///    execution endpoint.
    /// 4. If local resolution is negative in the local realm, either
    ///    return the typed resolver failure or fan out to configured
    ///    same-realm peers.
    /// 5. If the target realm is remote and no local FinalRoute exists,
    ///    ask `namespace.resolve` for a `PeerHub` delegation and issue
    ///    the same `federation.forward_invoke` request to the selected
    ///    peer hub.
    pub(crate) async fn dispatch_federation_forward_invoke(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        // PR-N6 C4: device-mode escalation. When this daemon
        // owns no PresenceRegistry of its own (mode = device),
        // it cannot execute a resolver-selected local-session
        // dispatch. Send the call up the existing
        // `<self>.session` bidi to the hub, await the matching
        // RequestResult, and surface its outcome on the unary
        // wire. Hub-mode and `both`-mode daemons leave
        // `escalation = None` and take the existing arm.
        if let Some(handle) = self.sessions.escalation.as_ref() {
            return self.escalate_forward_invoke(handle, arguments).await;
        }

        let request: federation_wrappers::ForwardInvokeRequest = parse_json_args(arguments)?;

        // RFC-005 owner proof: route the exact target owner the
        // caller supplied. Legacy `/agent/<bare-id>` device aliases
        // are intentionally not repaired here; callers must address
        // devices with canonical `/device/<id>` owner URAs.

        let target_realm = parse_realm_from_ura(&request.target_ura);
        let local_realm = self.identity.session_realm.as_deref();

        let is_local_realm = match (target_realm.as_deref(), local_realm) {
            (Some(target), Some(local)) => target == local,
            // Daemon has no realm context wired (smoke-test
            // build) — preserve PR-1 staging behavior and treat
            // every target as local.
            (_, None) => true,
            // Malformed target URA — fall through to the local
            // target-offline shape so a typo never accidentally hits the
            // cross-hub path.
            (None, Some(_)) => true,
        };
        let has_target_presence = self
            .directory
            .presence
            .lookup(&request.target_ura)
            .is_some();

        // Observable trace for operators debugging answer-sheet /
        // demo runs — proves which dispatch arm fired without
        // requiring an envelope-level packet capture. Cheap (one
        // eprintln per call) and the only daemon-A-side signal
        // that distinguishes "took cross-realm arm" from "took
        // local-presence arm" when the inner ability happens to
        // be a hub-served one (e.g. federation.heartbeat).
        // Render `Option<&str>` as a stable string so SRE pipelines
        // grep `target_realm=<value>` (or `=<none>` for the absent
        // case) without seeing Rust's `Some("…")` / `None` Debug
        // literal sneaking into the field value.
        let target_realm_field = target_realm.as_deref().unwrap_or("<none>");
        let local_realm_field = local_realm.unwrap_or("<none>");
        crate::op_event!(
            component = daemon_invocation,
            kind = forward_invoke_dispatch,
            target_ura = request.target_ura,
            target_realm = target_realm_field,
            local_realm = local_realm_field,
            is_local_realm = is_local_realm,
            has_target_presence = has_target_presence,
        );

        // Decode the inner payload up front. The
        // `correlation_call_id` field is required by DEC-N4 §2.1
        // so both arms (local selected route AND peer delegation)
        // can thread it back to the caller. Decode failure
        // surfaces as `Status::invalid_argument`; the CLI bridge
        // is the producer and must always supply a non-empty
        // `call_id` field.
        let inner_payload = decode_inner_payload(&request.inner_envelope_b64)?;
        let correlation_call_id = inner_payload.call_id.clone();

        // RFC-005 route-first dispatch selection. `request.target_ura`
        // proves owner intent and realm placement, but it is not an
        // execution endpoint. Once namespace.resolve returns a
        // FinalRoute, every local decision is made from the selected
        // route: self dispatch checks selected `execution_host_ura`,
        // session dispatch pushes to selected `execution_host_ura`,
        // and the frame carries selected `callee_ura` +
        // `dispatch_name`.
        let selected_local_route = match self
            .resolve_forward_invoke_route(&request, &inner_payload)
            .await
        {
            Ok(route) => Some(route),
            Err(err) => {
                if is_local_realm {
                    return Err(err);
                }
                None
            }
        };

        if let Some(selected_route) = selected_local_route {
            let selected_host_is_self = self
                .gate
                .matches_self_target_ura(&selected_route.execution_host_ura)
                .await;
            let selected_host_present = self
                .directory
                .presence
                .lookup(&selected_route.execution_host_ura)
                .is_some();
            crate::op_event!(
                component = daemon_invocation,
                kind = forward_invoke_selected_route,
                target_ura = request.target_ura,
                route_ura = selected_route.route_ura.as_str(),
                callee_ura = selected_route.callee_ura.as_str(),
                execution_host_ura = selected_route.execution_host_ura.as_str(),
                dispatch_name = selected_route.dispatch_name.as_str(),
                selected_host_is_self = selected_host_is_self,
                selected_host_present = selected_host_present,
            );

            if selected_host_is_self {
                return self
                    .dispatch_self_targeted_forward_invoke(
                        &inner_payload,
                        &selected_route,
                        &correlation_call_id,
                    )
                    .await;
            }

            match self
                .dispatch_local_presence_forward_invoke(
                    &inner_payload,
                    &selected_route,
                    &correlation_call_id,
                    request.origin_caller.as_ref(),
                    caller_envelope,
                )
                .await
            {
                Ok(response) => return Ok(response),
                Err(status) => return Err(status),
            }
        }

        // Cross-realm path. Missing federation client OR
        // missing peer entry both surface as
        // `failed_precondition(target_offline)` per DEC-N4 §2.1
        // — the older "Ok with target_online:false" shape is
        // gone. DEC-N5 §1 still requires a caller-hub
        // ForwardReceipt with `result_digest = None` for every
        // target_offline outcome.
        let record_offline_receipt = || {};
        let Some(client) = self.federation.client.as_ref() else {
            record_offline_receipt();
            return Err(Status::failed_precondition(
                federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
            ));
        };
        // Cross-realm dispatch derives the authoritative target realm
        // from the resolver's `NextHop::PeerHub` delegation answer
        // (`delegation.realm`), not from the URA-parsed `target_realm`:
        // the latter only feeds the `is_local_realm` arm above, which
        // already collapses every `None`/local realm to a local arm —
        // so reaching this cross-realm tail proves `target_realm` was
        // `Some` and equal to a non-local realm. We intentionally do
        // not re-thread it here.
        let delegated_route = match self
            .resolve_cross_realm_forward_delegation(&request, &inner_payload)
            .await
        {
            Ok(route) => route,
            Err(status) => {
                record_offline_receipt();
                return Err(status);
            }
        };
        let target_hub_endpoint = delegated_route.primary_endpoint().ok_or_else(|| {
            Status::failed_precondition(federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON)
        })?;
        let peer_request = self.build_forward_invoke_peer_request(caller_envelope, &request)?;
        let result = self
            .dispatch_forward_invoke_peer(
                client,
                target_hub_endpoint,
                peer_request,
                &request.target_ura,
                &correlation_call_id,
                "cross_realm",
            )
            .await;
        if result.is_err() {
            record_offline_receipt();
        }
        result
    }

    pub(crate) async fn resolve_forward_invoke_route(
        &self,
        request: &federation_wrappers::ForwardInvokeRequest,
        inner_payload: &InnerPayload,
    ) -> Result<SelectedInvokeRoute, Status> {
        let selector =
            crate::ura::AbilitySelector::parse(&inner_payload.ability_ura).map_err(|err| {
                Status::invalid_argument(format!(
                    "federation.forward_invoke: invalid canonical ability_ura `{}`: {err}",
                    inner_payload.ability_ura,
                ))
            })?;
        // Hosted-agent abilities are owned by an AGENT but execute on
        // the device that hosts it — whether `target` hosts the agent
        // is the resolver's to confirm (checked against the resolved
        // execution host below), not a local string equality.
        let owner_is_agent = crate::ura::parse_ura(selector.owner_ura())
            .map(|parsed| parsed.kind == crate::ura::URAKind::Agent)
            .unwrap_or(false);
        if selector.owner_ura() != request.target_ura && !owner_is_agent {
            return Err(Status::invalid_argument(format!(
                "federation.forward_invoke: ability_ura `{}` does not belong to target `{}`",
                inner_payload.ability_ura, request.target_ura,
            )));
        }

        let selected_route = self
            .gate
            .route_resolver()
            .await
            .resolve_route(&inner_payload.ability_ura, "")
            .map_err(route_negative_status)?;

        if !selected_route.is_authoritative_local_or_better() {
            return Err(route_profile_blocked_status(&selected_route));
        }
        // The forward target must be either the route's OWNER (the
        // pre-existing contract: device/hub/agent-targeted forwards)
        // or its EXECUTION HOST (hosted-agent abilities addressed via
        // the device that hosts them). Anything else is a
        // mis-addressed forward.
        let target_matches = selected_route.owner_ura == request.target_ura
            || selected_route.execution_host_ura == request.target_ura;
        if !target_matches {
            return Err(Status::invalid_argument(route_owner_mismatch_message(
                &selected_route.execution_host_ura,
                &inner_payload.ability_ura,
                &request.target_ura,
            )));
        }

        Ok(selected_route)
    }

    fn build_forward_invoke_peer_request(
        &self,
        caller_envelope: Option<&Envelope>,
        request: &federation_wrappers::ForwardInvokeRequest,
    ) -> Result<InvokeRequest, Status> {
        let nested = federation_wrappers::ForwardInvokeRequest {
            target_ura: request.target_ura.clone(),
            inner_envelope_b64: request.inner_envelope_b64.clone(),
            causal_context_bytes: request.causal_context_bytes.clone(),
            forward_deadline_ms: request.forward_deadline_ms,
            origin_caller: request.origin_caller.clone(),
        };
        let nested_arguments = serde_json::to_vec(&nested).map_err(|err| {
            Status::internal(format!(
                "federation.forward_invoke: encode nested ForwardInvokeRequest for peer \
                 delegation: {err}"
            ))
        })?;
        let mut peer_request = InvokeRequest {
            envelope: Some(build_peer_envelope(
                caller_envelope,
                &request.target_ura,
                self.identity.session_realm.as_deref(),
            )?),
            function_name: ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
            arguments: nested_arguments,
            ..InvokeRequest::default()
        };
        if let Some(envelope) = peer_request.envelope.as_mut() {
            sign_peer_request_envelope(
                envelope,
                &peer_request.function_name,
                &peer_request.arguments,
                self.identity.session_realm.as_deref(),
                self.federation.hub_signing_seed.as_ref(),
            )?;
        }
        Ok(peer_request)
    }

    pub(crate) async fn dispatch_forward_invoke_peer(
        &self,
        client: &Arc<dyn FederationClient>,
        target_hub_endpoint: &str,
        peer_request: InvokeRequest,
        target_ura: &str,
        correlation_call_id: &str,
        scope: &str,
    ) -> Result<Response<InvokeResponse>, Status> {
        let target_hub_endpoint = target_hub_endpoint.to_string();
        match client
            .forward_invoke(&target_hub_endpoint, peer_request)
            .await
        {
            Ok(peer_response) => {
                let peer_body: federation_wrappers::ForwardInvokeResponse =
                    match serde_json::from_slice(&peer_response.result) {
                        Ok(body) => body,
                        Err(err) => {
                            let err_msg = format!("{err}");
                            crate::op_event!(
                                component = daemon_invocation,
                                kind = forward_invoke_peer_response_malformed,
                                scope = scope,
                                error = err_msg,
                                message = "forwarding raw bytes for forward-compat",
                            );
                            federation_wrappers::ForwardInvokeResponse {
                                result_bytes: peer_response.result.clone(),
                                correlation_call_id: correlation_call_id.to_string(),
                            }
                        }
                    };
                let result_bytes_len = peer_body.result_bytes.len();
                crate::op_event!(
                    component = daemon_invocation,
                    kind = forward_invoke_peer_delegation_ok,
                    scope = scope,
                    target_ura = target_ura,
                    target_hub_endpoint = target_hub_endpoint,
                    result_bytes_len = result_bytes_len,
                );
                let response = federation_wrappers::ForwardInvokeResponse {
                    result_bytes: peer_body.result_bytes,
                    correlation_call_id: correlation_call_id.to_string(),
                };
                wrap_json_response(&response)
            }
            Err(err) => {
                let err_msg = format!("{err}");
                crate::op_event!(
                    component = daemon_invocation,
                    kind = forward_invoke_peer_delegation_failed,
                    scope = scope,
                    target_ura = target_ura,
                    target_hub_endpoint = target_hub_endpoint,
                    error = err_msg,
                );
                Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ))
            }
        }
    }

    pub(crate) async fn resolve_cross_realm_forward_delegation(
        &self,
        request: &federation_wrappers::ForwardInvokeRequest,
        inner_payload: &InnerPayload,
    ) -> Result<DelegatedInvokeRoute, Status> {
        let selector =
            crate::ura::AbilitySelector::parse(&inner_payload.ability_ura).map_err(|err| {
                Status::invalid_argument(format!(
                    "federation.forward_invoke: invalid canonical ability_ura `{}`: {err}",
                    inner_payload.ability_ura,
                ))
            })?;
        if selector.owner_ura() != request.target_ura {
            return Err(Status::invalid_argument(format!(
                "federation.forward_invoke: ability_ura `{}` does not belong to target `{}`",
                inner_payload.ability_ura, request.target_ura,
            )));
        }

        let delegation = self
            .gate
            .route_resolver()
            .await
            .resolve_delegation(&inner_payload.ability_ura, "")
            .map_err(route_negative_status)?
            .ok_or_else(|| {
                Status::failed_precondition(format!(
                    "{ROUTE_SELECTED_REMOTE_HOST_CODE}: federation.forward_invoke expected \
                     cross-realm namespace.resolve delegation for `{}`",
                    inner_payload.ability_ura,
                ))
            })?;

        for endpoint in &delegation.endpoints {
            if endpoint
                .metadata
                .get("source")
                .and_then(serde_json::Value::as_str)
                == Some("federated_directory")
            {
                if let Some(target_ura) = endpoint
                    .metadata
                    .get("targetUra")
                    .and_then(serde_json::Value::as_str)
                {
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = auto_route,
                        source = "federated_directory",
                        target_realm = delegation.realm.as_str(),
                        target_ura = target_ura,
                        hub_endpoint = endpoint.endpoint.as_str(),
                    );
                }
            }
        }

        Ok(delegation)
    }

    /// Shared presence-dispatch core (DEC-F004 single-settle
    /// discipline): pending registration BEFORE the frame push so a
    /// fast device reply lands a real `complete()` (race-free
    /// correlation), offline fast-fail on both send-failure shapes,
    /// then the awaited `DispatchResult`. Frame construction stays
    /// with the caller — the carrier choice is arm-specific, the
    /// mechanics are not.
    ///
    /// `register_pending_for(execution_host_ura)` keeps the
    /// presence-offline watcher able to fail-fast this entry the
    /// moment the host's `<self>.session` drops mid-call;
    /// [`PRESENCE_DISPATCH_REPLY_TIMEOUT`] backstops every reply that
    /// neither completes nor goes offline (structured
    /// `DeadlineExceeded` instead of an open-ended hang).
    async fn dispatch_frame_to_presence(
        &self,
        selected_route: &SelectedInvokeRoute,
        label: &str,
        build_frame: impl FnOnce(
            u64,
        )
            -> Result<crate::services::presence_registry::DispatchFrame, Status>,
    ) -> Result<(u64, DispatchResult), Status> {
        // Self guard: in device mode the boot seed registers a
        // resolve-only no-op presence entry under the daemon's own URA
        // (boot/presence_seed.rs) whose drain task accepts every frame
        // and never completes the pending entry — a frame dispatched
        // there parks the waiter until the deadline. Self-targeted
        // invocations belong to the local-runtime arms; refuse loudly
        // here rather than queue a frame that can never be answered.
        if self
            .admission
            .daemon_ura()
            .is_some_and(|self_ura| self_ura == selected_route.execution_host_ura)
        {
            crate::op_event!(
                component = daemon_invocation,
                kind = presence_dispatch_refused_self_host,
                label = label,
                execution_host_ura = selected_route.execution_host_ura.as_str(),
                route_ura = selected_route.route_ura.as_str(),
            );
            return Err(Status::failed_precondition(format!(
                "{label}: selected execution host `{}` is this daemon itself; \
                 self-targeted invocations dispatch through the local runtime, \
                 never the presence reverse channel (device-mode self-presence \
                 is resolve-only)",
                selected_route.execution_host_ura,
            )));
        }
        let pending = self.sessions.pending.as_ref().ok_or_else(|| {
            Status::failed_precondition(format!(
                "{label}: daemon was constructed without a PendingDispatchMap; call \
                 DaemonInvocationService::with_pending(...) at boot to enable \
                 cross-device dispatch",
            ))
        })?;
        let (session_id, sender) = self
            .directory
            .presence
            .lookup_tracked(&selected_route.execution_host_ura)
            .ok_or_else(|| {
                Status::failed_precondition(selected_host_unavailable_message(selected_route))
            })?;
        let handle = pending.register_pending_for(&selected_route.execution_host_ura);
        let call_id = handle.call_id();
        let dispatch_frame = build_frame(call_id)?;
        match sender.try_send(Ok(dispatch_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Full = device is slow, not dead: keep its session,
                // fail only this call as retryable backpressure.
                return Err(Status::resource_exhausted(
                    federation_wrappers::FORWARD_INVOKE_TARGET_BUSY_REASON,
                ));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.directory.presence.remove_if_session(
                    &selected_route.execution_host_ura,
                    session_id,
                    crate::services::presence_registry::OfflineReason::StreamClosed,
                );
                return Err(Status::failed_precondition(
                    federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
                ));
            }
        }
        crate::op_event!(
            component = daemon_invocation,
            kind = presence_dispatch_awaiting_reply,
            label = label,
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            ability = selected_route.dispatch_name.as_str(),
            route_ura = selected_route.route_ura.as_str(),
            call_id = call_id,
        );
        let result =
            match tokio::time::timeout(PRESENCE_DISPATCH_REPLY_TIMEOUT, handle.await_reply()).await
            {
                Ok(Ok(result)) => result,
                Ok(Err(_recv_err)) => {
                    return Err(Status::unavailable(format!(
                        "{label}: selected execution host `{}` session disconnected before reply \
                     (call_id={call_id})",
                        selected_route.execution_host_ura,
                    )));
                }
                Err(_elapsed) => {
                    // Timing out drops the future that owns the
                    // PendingHandle; its Drop evicts the map entry, so a
                    // late Result frame is a silent no-op complete.
                    crate::op_event!(
                        component = daemon_invocation,
                        kind = presence_dispatch_reply_timeout,
                        label = label,
                        execution_host_ura = selected_route.execution_host_ura.as_str(),
                        route_ura = selected_route.route_ura.as_str(),
                        call_id = call_id,
                        timeout_ms = PRESENCE_DISPATCH_REPLY_TIMEOUT.as_millis(),
                    );
                    return Err(Status::deadline_exceeded(format!(
                        "{label}: selected execution host `{}` accepted the dispatch frame but \
                     sent no Result within {}s (call_id={call_id})",
                        selected_route.execution_host_ura,
                        PRESENCE_DISPATCH_REPLY_TIMEOUT.as_secs(),
                    )));
                }
            };
        Ok((call_id, result))
    }

    /// step-4 / T2.1b: the canonical-face remote arm. A catch-all
    /// `Invoke` whose resolver-selected execution host is another
    /// device dispatches through that device's `<self>.session` — the
    /// same single-settle core as forward_invoke, but the caller's
    /// envelope travels verbatim (transplant, not translation):
    /// content, authority fields and the presigned caller signature
    /// are already ON the request, so nothing is wrapped and nothing
    /// is re-minted. v1 devices receive the canonical carrier; v0
    /// devices receive only the legacy JSON shape with the selected
    /// subject and no fabricated origin-caller claim.
    pub(crate) async fn dispatch_remote_rpc_selected_route(
        &self,
        request: &InvokeRequest,
        selected_route: &SelectedInvokeRoute,
    ) -> Result<Response<InvokeResponse>, Status> {
        let ability = request.function_name.trim().to_string();
        let Some(envelope) = request.envelope.clone() else {
            return Err(Status::invalid_argument(format!(
                "Invoke: remote-hosted ability `{ability}` requires the seven-tuple \
                 envelope on the canonical Invocation face",
            )));
        };
        // The resolver's verdict is authoritative: send the SELECTED
        // callee downstream, not the caller-supplied one.
        let envelope = envelope_with_selected_callee(envelope, selected_route);
        let dispatch_ability = selected_route.ability_ura.clone();
        let target_contract_v1 = self
            .directory
            .presence
            .dispatch_contract_version(&selected_route.execution_host_ura)
            .unwrap_or(0)
            >= 1;
        crate::op_event!(
            component = daemon_invocation,
            kind = remote_rpc_selected_route_dispatch,
            ability = ability.as_str(),
            dispatch_ability = dispatch_ability.as_str(),
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            route_ura = selected_route.route_ura.as_str(),
            carrier_v1 = target_contract_v1,
        );
        let arguments = request.arguments.clone();
        let (_call_id, dispatch_result) = self
            .dispatch_frame_to_presence(selected_route, "Invoke", |call_id| {
                if target_contract_v1 {
                    Ok(build_carrier_v1_dispatch_frame(
                        call_id,
                        easynet_axon::pb::axon::v1::InvokeRequest {
                            envelope: Some(envelope.clone()),
                            function_name: selected_route.dispatch_name.clone(),
                            arguments: arguments.clone(),
                            ..easynet_axon::pb::axon::v1::InvokeRequest::default()
                        },
                        false,
                    ))
                } else {
                    // v0 JSON fallback (one release window, dies with
                    // step 5). No origin-caller claim: the canonical
                    // envelope's CallerSignature carries no signer
                    // pubkey (verifiers resolve keys independently),
                    // so minting a claim here would fabricate key
                    // material. Per the claim contract fidelity is
                    // additive, never gating — v0 devices execute
                    // with the trust-domain identity.
                    let subject_ura = envelope.subject.as_ref().map(|s| s.ura.clone());
                    let Some(subject_ura) = subject_ura
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    else {
                        return Err(Status::invalid_argument(
                            "federation.forward_invoke: missing inner subject_ura",
                        ));
                    };
                    build_invoke_remote_dispatch_frame(InvokeRemoteDispatchFrameRequest {
                        call_id,
                        callee_ura: &selected_route.callee_ura,
                        subject_ura,
                        ability: &dispatch_ability,
                        args: &arguments,
                        args_content_envelope: SessionContentEnvelope::plaintext_json(),
                        metadata: HashMap::new(),
                        origin_caller: None,
                    })
                }
            })
            .await?;
        let DispatchResult {
            // Hub-side receipt projection for the unary face follows
            // the bidi step-2c path in the next slice; the executing
            // device's runtime already holds the canonical row.
            receipt: _,
            payload: result_bytes,
            error,
            failure,
            request_id,
        } = dispatch_result;
        if let Some(err) = error {
            let detail = failure
                .as_ref()
                .map(SessionFailure::status_detail)
                .unwrap_or(err);
            return Err(Status::failed_precondition(format!(
                "Invoke: remote route `{}` ability `{}` failed: {detail}",
                selected_route.route_ura, selected_route.dispatch_name,
            )));
        }
        Ok(Response::new(InvokeResponse {
            header: request_id.map(|rid| ResponseHeader {
                request_id: rid,
                status: "completed".to_string(),
                ..ResponseHeader::default()
            }),
            result: result_bytes,
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
            ..InvokeResponse::default()
        }))
    }

    /// Reverse-channel dispatch for a resolver-selected
    /// `federation.forward_invoke` route.
    ///
    /// Mirrors `dispatch_invoke_remote`'s pattern: register a
    /// `PendingDispatchMap` entry, push a
    /// `SessionDispatch::Dispatch{call_id, ability, args}` frame
    /// down the selected execution host's session bidi (the same wire shape
    /// device-side `LocalAxonSessionDispatcher::handle_down` expects),
    /// `await_reply` for the matching `SessionDispatch::Result`
    /// arriving via `drain_session_up_stream`, return the bytes
    /// inline as `ForwardInvokeResponse.result_bytes`.
    ///
    /// Errors:
    /// - selected execution host unavailable, or push fails →
    ///   `Status::failed_precondition(target_offline)`,
    ///   so the caller's same-realm fall-through arm can fan
    ///   out to federated peers.
    /// - target's session crashed mid-call (sender dropped
    ///   without complete) → `Status::unavailable` so the CLI
    ///   sees a structured upstream-failure rather than empty
    ///   bytes pretending success.
    /// - daemon was constructed without `PendingDispatchMap` →
    ///   `Status::failed_precondition` with a clear message
    ///   pointing at the boot configuration.
    pub(crate) async fn dispatch_local_presence_forward_invoke(
        &self,
        inner_payload: &InnerPayload,
        selected_route: &SelectedInvokeRoute,
        correlation_call_id: &str,
        origin_caller: Option<
            &crate::services::invocation_transport::origin_caller::OriginCallerClaim,
        >,
        caller_envelope: Option<&Envelope>,
    ) -> Result<Response<InvokeResponse>, Status> {
        let dispatch_ability = selected_route.dispatch_key();

        // Carrier selection (DEC-F004): v1 device + a real caller
        // envelope in hand → canonical proto frame, verbatim forward.
        // No envelope (legacy bridge args) or an origin-caller claim
        // → JSON shape until step 5 retires both.
        let target_contract_v1 = self
            .directory
            .presence
            .dispatch_contract_version(&selected_route.execution_host_ura)
            .unwrap_or(0)
            >= 1;
        let (call_id, dispatch_result) = self
            .dispatch_frame_to_presence(selected_route, "federation.forward_invoke", |call_id| {
                match (target_contract_v1, caller_envelope, &origin_caller) {
                    (true, Some(envelope), None) => Ok(build_carrier_v1_dispatch_frame(
                        call_id,
                        easynet_axon::pb::axon::v1::InvokeRequest {
                            envelope: Some(envelope.clone()),
                            function_name: selected_route.dispatch_name.clone(),
                            arguments: inner_payload.args_bytes.clone(),
                            ..easynet_axon::pb::axon::v1::InvokeRequest::default()
                        },
                        false,
                    )),
                    _ => build_invoke_remote_dispatch_frame(InvokeRemoteDispatchFrameRequest {
                        call_id,
                        callee_ura: &selected_route.callee_ura,
                        subject_ura: &selected_route.callee_ura,
                        ability: &dispatch_ability,
                        args: &inner_payload.args_bytes,
                        args_content_envelope: SessionContentEnvelope::plaintext_json(),
                        metadata: HashMap::new(),
                        origin_caller: origin_caller.cloned(),
                    }),
                }
            })
            .await?;

        let DispatchResult {
            // Forward-path receipt projection rides T2.1b (the backend
            // submits real envelopes; context lands here then).
            receipt: _,
            payload: result_bytes,
            error,
            failure,
            request_id: _,
        } = dispatch_result;
        // Diagnostic: forward the mac-side outcome verbatim so a
        // session-frame-correlation race is visible in the hub log
        // without having to attach a debugger. Cheap (one op-event
        // per round-trip). Render `Option<String>` via as_deref so
        // SRE pipelines see `error=<value>` (or `error=<none>`)
        // instead of Rust's `Some("…")` / `None` Debug literal.
        let result_bytes_len = result_bytes.len();
        let error_field = error.as_deref().unwrap_or("<none>");
        let failure_code = failure
            .as_ref()
            .map(|failure| failure.code.as_str())
            .unwrap_or("<none>");
        crate::op_event!(
            component = daemon_invocation,
            kind = forward_invoke_local_presence_dispatch_result,
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            ability = selected_route.dispatch_name.as_str(),
            route_ura = selected_route.route_ura.as_str(),
            call_id = call_id,
            result_bytes_len = result_bytes_len,
            error = error_field,
            failure_code = failure_code,
        );
        if let Some(err) = error {
            let detail = failure
                .as_ref()
                .map(SessionFailure::status_detail)
                .unwrap_or(err);
            return Err(Status::failed_precondition(format!(
                "federation.forward_invoke: selected route `{}` ability `{}` failed: {detail}",
                selected_route.route_ura, selected_route.dispatch_name,
            )));
        }

        // DEC-N5 §1: write the ForwardReceipt with a real
        // result_digest (not None) since we have the bytes
        // inline.

        let response = federation_wrappers::ForwardInvokeResponse {
            result_bytes,
            correlation_call_id: correlation_call_id.to_string(),
        };
        wrap_json_response(&response)
    }

    /// **PR-1 commit 7/9 (LB-56)**. Synchronous self-targeted
    /// `federation.forward_invoke` dispatch.
    ///
    /// Caller has confirmed the target URA names this daemon. We
    /// resolve the inner ability against the daemon's Axon
    /// `LocalRuntime`, write a single ForwardReceipt with a real
    /// `result_digest` (no async second update), and return the bytes
    /// inline in `ForwardInvokeResponse.result_bytes`.
    ///
    /// Errors map to `tonic::Status`:
    /// - runtime missing → `Status::failed_precondition`
    /// - ability not registered → `Status::not_found`
    /// - handler returned an Axon error → `Status::failed_precondition`
    ///   with the underlying SDK error.
    pub(crate) async fn dispatch_self_targeted_forward_invoke(
        &self,
        inner_payload: &InnerPayload,
        selected_route: &SelectedInvokeRoute,
        correlation_call_id: &str,
    ) -> Result<Response<InvokeResponse>, Status> {
        let Some(runtime) = self.runtime.local_runtime.as_ref() else {
            return Err(Status::failed_precondition(
                "federation.forward_invoke: self-targeted dispatch cannot run because \
                 Axon LocalRuntime is not wired at boot",
            ));
        };

        let dispatch_ability = selected_route.ability_ura.clone();
        let runtime_ability = dispatch_ability.clone();
        if !runtime.has_ability(&runtime_ability).await {
            return Err(Status::not_found(format!(
                "federation.forward_invoke: self-targeted ability `{dispatch_ability}` is not \
                 registered in Axon LocalRuntime as `{runtime_ability}`"
            )));
        }

        let descriptor_subject_ura =
            self.default_self_target_subject_ura(&selected_route.callee_ura)?;

        crate::op_event!(
            component = daemon_invocation,
            kind = forward_invoke_self_target_dispatch,
            callee_ura = selected_route.callee_ura.as_str(),
            descriptor_subject_ura = descriptor_subject_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            ability = selected_route.dispatch_name.as_str(),
            route_ura = selected_route.route_ura.as_str(),
            dispatch_ability = dispatch_ability.as_str(),
            call_id = correlation_call_id,
        );

        let outcome = crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc_local_with_subject(
            runtime,
            &selected_route.callee_ura,
            &descriptor_subject_ura,
            &dispatch_ability,
            inner_payload.args_bytes.clone(),
        )
        .await;
        let result_bytes = match outcome.error {
            None => outcome.payload_bytes,
            Some(err) => {
                return Err(Status::failed_precondition(format!(
                    "federation.forward_invoke: self-targeted dispatch of ability `{dispatch_ability}` failed: {err}",
                )));
            }
        };
        if outcome.state != easynet_axon::invocation::InvocationState::Completed {
            return Err(Status::failed_precondition(format!(
                "federation.forward_invoke: self-targeted dispatch of ability `{dispatch_ability}` ended in state {}",
                outcome.state.as_str(),
            )));
        }
        if result_bytes.is_empty() {
            crate::op_event!(
                component = daemon_invocation,
                kind = forward_invoke_self_target_empty_result,
                callee_ura = selected_route.callee_ura.as_str(),
                execution_host_ura = selected_route.execution_host_ura.as_str(),
                ability = selected_route.dispatch_name.as_str(),
                route_ura = selected_route.route_ura.as_str(),
                call_id = correlation_call_id,
            );
        }

        // Single ForwardReceipt write with real result_digest —
        // unlike the bidi-push path, no PR-N5 second-update is
        // needed because the bytes are already known.

        let response = federation_wrappers::ForwardInvokeResponse {
            result_bytes,
            correlation_call_id: correlation_call_id.to_string(),
        };
        wrap_json_response(&response)
    }

    /// Self-targeted `<self>.invoke_remote` shortcut.
    ///
    /// When the daemon receives `<self>.invoke_remote` whose
    /// subject_device equals its own URA, dispatch the ability
    /// through the shared Axon `LocalRuntime` and return the result
    /// on a one-shot down stream. This fires in two scenarios:
    ///
    ///   1. Host-mode dev rig: backend invokes a device.* ability
    ///      against the local device daemon's own URA. The
    ///      daemon's PresenceRegistry self-presence seed
    ///      (boot.rs) makes the target findable; this shortcut
    ///      dispatches inline without trying to push frames
    ///      down a drain channel that nobody consumes.
    ///
    ///   2. Hub-mode self-call: a hub invoking an ability on
    ///      its own URA (rare but valid; the hub is a Both-mode
    ///      daemon and the local runtime hosts its registered tools).
    ///
    /// Mirrors `dispatch_self_targeted_forward_invoke` for the
    /// federation.forward_invoke surface — same idea, different
    /// envelope shape.
    pub(crate) async fn dispatch_self_targeted_invoke_remote(
        &self,
        selected_route: &SelectedInvokeRoute,
        subject_ura: Option<&str>,
        args: &[u8],
        _metadata: &std::collections::HashMap<String, String>,
        origin_claim: Option<
            &crate::services::invocation_transport::origin_caller::OriginCallerClaim,
        >,
    ) -> Result<Response<BoxedDownStream<InvokeBidiDown>>, Status> {
        crate::op_event!(
            component = daemon_invocation,
            kind = invoke_remote_self_target_dispatch,
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            ability = selected_route.dispatch_name.as_str(),
            route_ura = selected_route.route_ura.as_str(),
        );

        let Some(runtime) = self.runtime.local_runtime.as_ref() else {
            return Err(Status::failed_precondition(
                "<self>.invoke_remote: self-targeted dispatch cannot run because \
                 Axon LocalRuntime is not wired at boot",
            ));
        };

        let dispatch_ability = selected_route.ability_ura.clone();
        let default_subject_ura =
            self.default_self_target_subject_ura(&selected_route.callee_ura)?;
        let inner_subject =
            UnarySelfTargetSubject::from_optional(subject_ura, default_subject_ura)?;

        // Inner user-caller pass-through: when the hub/backend attached a
        // typed browser-signed user claim, dispatch the inner ability with
        // the real user as caller via descriptor-bound Axon admission.
        let origin_caller =
            match crate::services::invocation_transport::origin_caller::OriginCaller::resolve(
                origin_claim,
            ) {
                Ok(oc) => oc,
                Err(err) => {
                    // A present-but-malformed authority must fail closed, not
                    // silently downgrade to _system.
                    return invoke_remote_inband_error_response(format!(
                        "<self>.invoke_remote: invalid origin caller claim: {err}"
                    ));
                }
            };

        let outcome = if let Some(origin) = origin_caller {
            crate::op_event!(
                component = daemon_invocation,
                kind = invoke_remote_self_target_user_caller,
                caller_ura = origin.caller_ura.as_str(),
                ability = dispatch_ability.as_str(),
            );
            // Cross-device callers: warm the anchor from the hub on a
            // miss (resolve_key trust sync), mirroring the
            // `<self>.session` dispatcher arm. Admission below stays
            // local-anchor-authoritative; a failed sync just lets the
            // dispatch fail closed with the precise admission error.
            if let Some(sync) = self.sessions.device_trust_sync.as_ref() {
                sync.ensure_caller_key(&origin.caller_ura).await;
            }
            if selected_route.dispatch_name != origin.public_ability() {
                return invoke_remote_inband_error_response(dispatch_key_mismatch_message(
                    "<self>.invoke_remote",
                    origin.public_ability(),
                    &selected_route.dispatch_name,
                    &selected_route.route_ura,
                ));
            }
            let wire = match origin.into_wire_dispatch(
                &selected_route.callee_ura,
                inner_subject.as_str(),
                args.to_vec(),
            ) {
                Ok(wire) => wire,
                Err(err) => {
                    return invoke_remote_inband_error_response(format!(
                        "<self>.invoke_remote: invalid origin caller dispatch: {err}"
                    ));
                }
            };
            crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc(runtime, wire).await
        } else {
            crate::runtime::axon_bridge::dispatch_shim::dispatch_rpc_local_with_subject(
                runtime,
                &selected_route.callee_ura,
                inner_subject.as_str(),
                &dispatch_ability,
                args.to_vec(),
            )
            .await
        };
        let request_id = outcome.invocation_id.clone();
        let (payload, error) =
            crate::runtime::axon_bridge::dispatch_shim::outcome_to_invoke_remote_result(outcome);

        let down = InvokeRemoteDown::Result {
            payload,
            failure: error
                .as_ref()
                .map(|reason| SessionFailure::from_reason(reason, "INVOCATION_FAILED", false)),
            error,
            request_id,
        };
        let frame = build_invoke_remote_terminal_frame(&down)?;

        // One-shot down stream: yield the terminal frame, close.
        let (down_tx, down_rx) = mpsc::channel::<Result<InvokeBidiDown, Status>>(1);
        tokio::spawn(async move {
            let _ = down_tx.send(Ok(frame)).await;
        });
        let stream = ReceiverStream::new(down_rx);
        Ok(Response::new(
            Box::pin(stream) as BoxedDownStream<InvokeBidiDown>
        ))
    }

    /// PR-N6 C4: device-mode `forward_invoke` escalation. Sends
    /// the call up the open `<self>.session` bidi to the hub via
    /// the supplied escalation handle, awaits the matching
    /// `RequestResult`, and translates the typed outcome onto the
    /// existing unary wire shape callers already understand.
    ///
    /// On `Ok { result_bytes }` the caller sees the same
    /// `ForwardInvokeResponse` shape PR-N1 already returns on
    /// hub-mode success. On `Err { error: TargetOffline }` the
    /// caller sees `Status::failed_precondition(target_offline)`
    /// — wire-stable with the existing reason text so a CLI
    /// upstream of this daemon doesn't have to branch on
    /// device-vs-hub mode. Other typed errors map to the
    /// closest existing wire reason.
    async fn escalate_forward_invoke(
        &self,
        handle: &std::sync::Arc<
            crate::services::invocation_transport::session_escalation::SessionEscalationHandle,
        >,
        arguments: &[u8],
    ) -> Result<Response<InvokeResponse>, Status> {
        let outcome = handle
            .escalate(
                ABILITY_FEDERATION_FORWARD_INVOKE.to_string(),
                arguments.to_vec(),
            )
            .await;
        match outcome {
            RequestOutcome::Ok { result_bytes } => Ok(Response::new(InvokeResponse {
                result: result_bytes,
                result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
                state: easynet_axon::invocation::InvocationState::Completed.to_wire_i32(),
                ..InvokeResponse::default()
            })),
            RequestOutcome::Err {
                error: SessionRequestError::TargetOffline,
            } => Err(Status::failed_precondition(
                federation_wrappers::FORWARD_INVOKE_TARGET_OFFLINE_REASON,
            )),
            RequestOutcome::Err {
                error: SessionRequestError::PermissionDenied { reason },
            } => Err(Status::permission_denied(reason)),
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamFailure { reason },
            } => Err(Status::unavailable(format!(
                "session escalation upstream failure: {reason}"
            ))),
            RequestOutcome::Err {
                error: SessionRequestError::UpstreamTimeout,
            } => Err(Status::deadline_exceeded(
                "session escalation timed out waiting for hub RequestResult",
            )),
        }
    }
}

fn local_invoke_target_ura(request: &InvokeRequest) -> Result<String, Status> {
    target_ura_from_envelope(request.envelope.as_ref(), "Invoke")
}

/// True for node-internal `runtime.*` admin handshakes that the receiving
/// daemon hosts directly on its `LocalRuntime` and must not route through
/// owner-presence resolution (e.g. `runtime.bootstrap_self_identity`).
pub(crate) fn is_runtime_admin_ability(function: &str) -> bool {
    function.trim().starts_with("runtime.")
}

fn sorted_non_empty_urls(urls: Vec<String>) -> Vec<String> {
    urls.into_iter()
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn namespace_proxy_resolve_peer_arguments(
    request: &federation_wrappers::NamespaceProxyResolveRequest,
) -> Result<Vec<u8>, Status> {
    serde_json::to_vec(&serde_json::json!({
        "queryName": non_empty_json_string(&request.query_name),
        "qtype": non_empty_json_string(&request.qtype)
            .unwrap_or_else(|| "RESOLVE_TYPE_DIRECTORY_LISTING".to_string()),
        "callerUra": non_empty_json_string(&request.caller_ura),
        "subjectUra": non_empty_json_string(&request.subject_ura),
        "realmHint": non_empty_json_string(&request.realm_hint),
        "abilityName": non_empty_json_string(&request.ability_name),
    }))
    .map_err(|err| {
        Status::internal(format!(
            "namespace.proxy_resolve: encode peer request: {err}"
        ))
    })
}

fn namespace_proxy_resolve_empty_answer(
    request: &federation_wrappers::NamespaceProxyResolveRequest,
) -> serde_json::Value {
    namespace_proxy_resolve_merge_answer(request, Vec::new())
}

fn namespace_proxy_resolve_merge_answer(
    request: &federation_wrappers::NamespaceProxyResolveRequest,
    peer_answers: Vec<serde_json::Value>,
) -> serde_json::Value {
    let mut records = BTreeMap::<String, serde_json::Value>::new();
    for answer in peer_answers {
        let Some(rows) = answer.get("records").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for row in rows {
            let key = namespace_record_merge_key(row);
            records.entry(key).or_insert_with(|| row.clone());
        }
    }

    serde_json::json!({
        "answerKind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
        "canonicalName": non_empty_json_string(&request.query_name),
        "records": records.into_values().collect::<Vec<_>>(),
        "releaseProfile": "RESOLVER_RELEASE_PROFILE_PRODUCTION",
        "cachePolicy": {
            "ttlMs": 0,
            "sharedCacheable": false,
            "retryAfterUnixMs": 0,
        },
    })
}

fn namespace_record_merge_key(row: &serde_json::Value) -> String {
    let name = row
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let record_type = row
        .get("recordType")
        .or_else(|| row.get("record_type"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    format!("{name}\u{1f}{record_type}")
}

fn non_empty_json_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
