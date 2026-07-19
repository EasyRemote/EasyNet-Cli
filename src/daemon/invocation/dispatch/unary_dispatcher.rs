// EasyNet Daemon — Invoke (Unary) Dispatcher
// ============================================
//
// File: src/daemon/invocation/unary_dispatcher.rs
// Description: Owns every unary `Invoke` routing arm the daemon serves
//              after transport policy + quota (commit-plan-2 Axis E / E2):
//
//                * federation prelude writes — join / advertise_agent /
//                  advertise_abilities / heartbeat
//                * federation reads — resolve / resolve_key / discover /
//                  list_user_devices (+ backend proxy variants) / revoke
//                * RFC-005 namespace.resolve (+ backend proxy variant)
//                * identity verbs — identity.register_pubkey /
//                  identity.revoke_user_pubkey / identity.list_user_pubkeys
//                * descriptor-bound runtime-admin handshakes
//                * the resolve-first LocalRuntime catch-all
//
//              The invoke family stays on the service until its
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

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use tonic::{Response, Status};

use axon_sdk::invocation::{AbilityContext, AxonError, CallMode};
use axon_sdk::pb::axon::v1::{Envelope, InvokeRequest, InvokeResponse, ResponseHeader};

use std::collections::BTreeMap;

use crate::daemon::federation::client::FederationClientError;
use crate::daemon::federation::directory::now_unix_ms;
use crate::daemon::invocation::admission::admission_facade::AdmissionFacade;
use crate::daemon::invocation::admission::decision::SignatureDecisionReason;
use crate::daemon::invocation::admission::hosted_agent_delegation::HostedAgentDelegationIssuer;
use crate::daemon::invocation::admission::hosted_agent_publication::HostedAgentPublication;
use crate::daemon::invocation::admission::list_user_pubkeys::handle as handle_list_user_pubkeys;
use crate::daemon::invocation::admission::owner_projection_publication::OwnerProjectionPublicationAuthority;
use crate::daemon::invocation::admission::peer_envelope_signer::PeerInvokeRequest;
use crate::daemon::invocation::admission::register_device_pubkey::handle as handle_register_device_pubkey;
use crate::daemon::invocation::admission::register_device_pubkey::parse_register_pubkey_intent;
use crate::daemon::invocation::admission::revoke_user_pubkey::{
    handle_with_outcome as handle_revoke_user_pubkey_with_outcome, parse_revoke_user_pubkey_intent,
};
use crate::daemon::invocation::admission::runtime_trust::RuntimeTrust;
use crate::daemon::invocation::admission::runtime_trust_invalidator::{
    RuntimeTrustConnectionStateProjector, RuntimeTrustInvalidator,
};
use crate::daemon::invocation::admission::target_gate::{
    route_negative_status, route_profile_blocked_status, signed_envelope_for_selected_route,
    TargetGate,
};
use crate::daemon::invocation::bidi::session_wire::{
    build_carrier_v1_dispatch_frame, require_canonical_dispatch_session, SessionRequestError,
};
use crate::daemon::invocation::bidi::state::pending_dispatch::DispatchResult;
use crate::daemon::invocation::dispatch::daemon_invocation_service::DaemonUnaryRoute;
use crate::daemon::invocation::dispatch::daemon_route_runtime::product_status_to_axon_error;
use crate::daemon::invocation::dispatch::deps::{
    DirectoryPlane, FederationDial, IdentityPlane, RuntimePlane, SessionPlane,
};
use crate::daemon::invocation::dispatch::descriptor_binding::RuntimeBoundAbility;
use crate::daemon::invocation::dispatch::federation_wrappers;
use crate::daemon::invocation::dispatch::federation_wrappers::{
    ABILITY_FEDERATION_LIST_USER_DEVICES, ABILITY_NAMESPACE_RESOLVE,
};
use crate::daemon::invocation::dispatch::forwarded_finalization::{
    ensure_forwarded_receipt_signer_key, ensure_forwarded_response_receipt_signer_keys,
    ForwardedFinalizedInvocation, ForwardedInvocationBinding,
};
use crate::daemon::invocation::dispatch::invocation_wire::{
    descriptor_ref_from_invocation_target, encode_json_payload,
    function_name_from_invocation_target, parse_json_args, status_from_axon_invoke_error,
    target_ura_from_envelope, FEDERATION_RESULT_CONTENT_TYPE,
};
use crate::daemon::invocation::dispatch::remote_failure::{
    is_admission_denial_message, status_from_remote_failure,
};
use crate::daemon::invocation::routing::route_resolver::{
    CanonicalRouteDispatch, CanonicalRouteSelection, DelegatedInvokeRoute, SelectedInvokeRoute,
};
use crate::daemon::trust::anchor::{TrustedAgentRole, TrustedPrincipalOwner};

pub(crate) fn rpc_dispatch_outcome_response(
    outcome: crate::daemon::axon_bridge::dispatch_shim::RpcDispatchOutcome,
) -> (Result<Response<InvokeResponse>, Status>, bool) {
    let crate::daemon::axon_bridge::dispatch_shim::RpcDispatchOutcome {
        invocation_id,
        state,
        payload_bytes,
        error,
        admission_receipt,
        terminal_receipt,
        ..
    } = outcome;
    let axon_started = invocation_id.is_some();
    let admission_receipt = match admission_receipt
        .as_ref()
        .map(axon_sdk::invocation::wire::receipt_to_wire)
        .transpose()
    {
        Ok(receipt) => receipt,
        Err(error) => {
            return (
                Err(Status::failed_precondition(format!(
                    "CANONICAL_ADMISSION_PROJECTION_FAILED: {error}"
                ))),
                axon_started,
            )
        }
    };
    let terminal_receipt = match terminal_receipt
        .as_ref()
        .map(axon_sdk::invocation::wire::receipt_to_wire)
        .transpose()
    {
        Ok(receipt) => receipt,
        Err(error) => {
            return (
                Err(Status::failed_precondition(format!(
                    "CANONICAL_TERMINAL_PROJECTION_FAILED: {error}"
                ))),
                axon_started,
            )
        }
    };
    (
        Ok(Response::new(InvokeResponse {
            header: invocation_id.map(|request_id| ResponseHeader {
                request_id,
                status: state.as_str().to_string(),
                ..ResponseHeader::default()
            }),
            result: payload_bytes,
            result_content_type: FEDERATION_RESULT_CONTENT_TYPE.to_string(),
            state: state.to_wire_i32(),
            error: error
                .as_ref()
                .map(axon_sdk::invocation::wire::error_to_wire),
            admission_receipt,
            terminal_receipt,
            ..InvokeResponse::default()
        })),
        axon_started,
    )
}

#[cfg(test)]
mod rpc_dispatch_outcome_response_tests {
    use super::*;
    use axon_sdk::invocation::{AxonError, ErrorCode, ErrorStage, InvocationState, SecurityClass};

    #[test]
    fn pre_admission_rejection_remains_an_in_band_failed_outcome() {
        let error = AxonError::invalid_argument("AXON_CALLER_SIGNATURE_INVALID")
            .with_code(ErrorCode::CallerSignatureInvalid)
            .with_stage(ErrorStage::CallerAuthentication)
            .with_security_class(SecurityClass::Authentication)
            .with_message("ed25519_signature_wrong_length");
        let (response, axon_started) = rpc_dispatch_outcome_response(
            crate::daemon::axon_bridge::dispatch_shim::RpcDispatchOutcome {
                invocation_id: None,
                state: InvocationState::Failed,
                payload_bytes: Vec::new(),
                error: Some(error),
                admission_receipt: None,
                terminal_receipt: None,
            },
        );

        let response = response.expect("pre-admission rejection must remain in-band");
        let response = response.into_inner();
        assert!(!axon_started);
        assert_eq!(response.state, InvocationState::Failed.to_wire_i32());
        assert!(response.admission_receipt.is_none());
        assert!(response.terminal_receipt.is_none());
        let error = response.error.expect("typed rejection");
        assert_eq!(error.code, ErrorCode::CallerSignatureInvalid.as_str());
        assert_eq!(
            error.stage,
            axon_sdk::pb::axon::v1::ErrorStage::CallerAuthentication as i32
        );
    }
}

fn validate_federation_join_request(
    request: &federation_wrappers::JoinRequest,
    daemon_realm: &str,
) -> Result<(), Status> {
    if request.realm.trim() != daemon_realm {
        return Err(Status::invalid_argument(format!(
            "federation.join: request realm `{}` does not match daemon realm `{daemon_realm}`",
            request.realm
        )));
    }
    let parsed = crate::core::ura::parse_ura(request.membership_ura.trim()).map_err(|err| {
        Status::invalid_argument(format!(
            "federation.join: membership_ura `{}` is not a canonical device URA: {err}",
            request.membership_ura
        ))
    })?;
    if parsed.kind != crate::core::ura::URAKind::Device {
        return Err(Status::invalid_argument(format!(
            "federation.join: membership_ura `{}` must identify a device, got {:?}",
            request.membership_ura, parsed.kind
        )));
    }
    if parsed.realm != request.realm.trim() {
        return Err(Status::invalid_argument(format!(
            "federation.join: membership realm `{}` does not match request realm `{}`",
            parsed.realm, request.realm
        )));
    }
    Ok(())
}

fn admitted_join_principal_owner(
    request: &federation_wrappers::JoinRequest,
    lifecycle: Option<
        &crate::daemon::invocation::admission::principal_lifecycle::PrincipalLifecycleContext,
    >,
) -> Result<Option<TrustedPrincipalOwner>, Status> {
    let Some(principal_enrollment) = request.principal_enrollment.as_ref() else {
        return Ok(None);
    };
    let principal_ura = principal_enrollment.principal_ura.trim();
    let parsed_principal = crate::core::ura::parse_ura(principal_ura).map_err(|err| {
        Status::invalid_argument(format!(
            "federation.join: principal_enrollment.principal_ura is not canonical: {err}"
        ))
    })?;
    if parsed_principal.kind != crate::core::ura::URAKind::User {
        return Err(Status::invalid_argument(
            "federation.join: principal_enrollment.principal_ura must identify a User URA"
                .to_string(),
        ));
    }
    if parsed_principal.realm != request.realm.trim() {
        return Err(Status::permission_denied(format!(
            "federation.join: principal realm `{}` must match join realm `{}`",
            parsed_principal.realm, request.realm
        )));
    }
    let Some(owner_user_id) = parsed_principal.user_id().map(str::to_string) else {
        return Err(Status::invalid_argument(
            "federation.join: principal_enrollment.principal_ura missing user id",
        ));
    };
    let lifecycle = lifecycle.ok_or_else(|| {
        Status::failed_precondition(
            "federation.join: principal enrollment proof supplied but PrincipalLifecycle is not wired",
        )
    })?;
    lifecycle.reader().verify_join_enrollment_proof(
        principal_ura,
        principal_enrollment.proof.kind.trim(),
        principal_enrollment.proof.reference.trim(),
    )?;
    Ok(Some(TrustedPrincipalOwner {
        principal_ura: request.membership_ura.trim().to_string(),
        owner_username: Some(owner_user_id.clone()),
        owner_user_id,
        owner_ura: principal_ura.to_string(),
        added_at_unix_ms: crate::daemon::invocation::admission::runtime_trust::now_unix_ms(),
    }))
}

fn public_key_hex_to_b64(public_key_hex: &str) -> Result<String, Status> {
    let raw = hex::decode(public_key_hex.trim()).map_err(|err| {
        Status::invalid_argument(format!("federation.join: public_key_hex is not hex: {err}"))
    })?;
    if raw.len() != 32 {
        return Err(Status::invalid_argument(format!(
            "federation.join: public_key_hex must decode to 32 bytes, got {}",
            raw.len()
        )));
    }
    Ok(B64.encode(raw))
}

/// Hard ceiling on one presence-dispatch round-trip: the time between
/// pushing a `Dispatch` frame down a device's `session.open` and
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

/// Product provider installed behind every daemon exact-route runtime row.
/// It owns no invocation lifecycle state: one call executes one product
/// handler and returns only its payload to Axon.
#[derive(Clone)]
pub(crate) struct DaemonUnaryRouteProvider {
    dispatcher: UnaryDispatcher,
}

impl DaemonUnaryRouteProvider {
    pub(crate) fn new(dispatcher: UnaryDispatcher) -> Self {
        Self { dispatcher }
    }

    pub(crate) async fn invoke(
        &self,
        route: DaemonUnaryRoute,
        context: Arc<AbilityContext>,
    ) -> Result<Vec<u8>, AxonError> {
        let arguments = context.payload.as_slice();
        let envelope = runtime_context_envelope(&context)?;
        match route {
            DaemonUnaryRoute::FederationJoin => self.dispatcher.dispatch_federation_join(arguments),
            DaemonUnaryRoute::FederationAdvertiseAgent => self
                .dispatcher
                .dispatch_federation_advertise_agent(arguments, Some(&envelope)),
            DaemonUnaryRoute::FederationAdvertiseAbilities => self
                .dispatcher
                .dispatch_federation_advertise_abilities(arguments, Some(&envelope)),
            DaemonUnaryRoute::FederationHeartbeat => {
                self.dispatcher.dispatch_federation_heartbeat(arguments)
            }
            DaemonUnaryRoute::FederationStatus => self.dispatcher.dispatch_federation_status(),
            DaemonUnaryRoute::FederationResolve => {
                self.dispatcher.dispatch_federation_resolve(arguments)
            }
            DaemonUnaryRoute::NamespaceResolve => {
                self.dispatcher.dispatch_namespace_resolve(arguments).await
            }
            DaemonUnaryRoute::FederationResolveKey => {
                self.dispatcher.dispatch_federation_resolve_key(arguments)
            }
            DaemonUnaryRoute::FederationDiscover => {
                self.dispatcher.dispatch_federation_discover(arguments)
            }
            DaemonUnaryRoute::FederationListUserDevices => self
                .dispatcher
                .dispatch_federation_list_user_devices(Some(&envelope), arguments),
            DaemonUnaryRoute::FederationProxyListUserDevices => {
                self.dispatcher
                    .dispatch_federation_proxy_list_user_devices(Some(&envelope), arguments)
                    .await
            }
            DaemonUnaryRoute::NamespaceProxyResolve => {
                self.dispatcher
                    .dispatch_namespace_proxy_resolve(Some(&envelope), arguments)
                    .await
            }
            DaemonUnaryRoute::FederationRevoke => {
                self.dispatcher.dispatch_federation_revoke(arguments)
            }
            DaemonUnaryRoute::IdentityRegisterPubkey => self
                .dispatcher
                .dispatch_register_device_pubkey(Some(&envelope), arguments),
            DaemonUnaryRoute::IdentityRevokeUserPubkey => self
                .dispatcher
                .dispatch_revoke_user_pubkey(Some(&envelope), arguments),
            DaemonUnaryRoute::IdentityListUserPubkeys => {
                self.dispatcher.dispatch_list_user_pubkeys(arguments)
            }
            principal_route @ (DaemonUnaryRoute::PrincipalCreate
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
            | DaemonUnaryRoute::PrincipalGet) => self
                .dispatcher
                .dispatch_principal_lifecycle(principal_route.name(), arguments),
        }
        .map_err(product_status_to_axon_error)
    }
}

fn runtime_context_envelope(context: &AbilityContext) -> Result<Envelope, AxonError> {
    let signed = context.signed_envelope().ok_or_else(|| {
        AxonError::internal("daemon unary route provider missing admitted signed envelope")
    })?;
    axon_sdk::invocation::project_wire_envelope(
        &signed.envelope,
        axon_sdk::invocation::WireEnvelopeMetadata {
            request_id: context.invocation_id.clone(),
            caller_signature: Some(signed.signature.clone()),
            ..axon_sdk::invocation::WireEnvelopeMetadata::default()
        },
    )
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

    pub(crate) fn daemon_route_provider(&self) -> DaemonUnaryRouteProvider {
        DaemonUnaryRouteProvider::new(self.clone())
    }

    pub(crate) async fn dispatch_daemon_route_runtime(
        &self,
        route: DaemonUnaryRoute,
        request: &InvokeRequest,
        ingress: crate::daemon::invocation::dispatch::daemon_route_runtime::DaemonRouteIngress,
    ) -> Result<Response<InvokeResponse>, Status> {
        let runtime = self.runtime.local_runtime().ok_or_else(|| {
            Status::failed_precondition(format!(
                "easynet-daemon: exact route `{}` cannot run because Axon LocalRuntime is not wired at boot",
                route.name()
            ))
        })?;
        crate::daemon::invocation::dispatch::daemon_route_runtime::DaemonRouteRuntimeAdapter::new(
            runtime,
            self.runtime.cancellations.clone(),
            self.admission.clone(),
            self.runtime.product_policy()?,
        )
        .dispatch(route, request, ingress)
        .await
    }

    pub(crate) fn dispatch_federation_join(&self, arguments: &[u8]) -> Result<Vec<u8>, Status> {
        let request: federation_wrappers::JoinRequest = parse_json_args(arguments)?;
        let ctx = self.identity.runtime_trust.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "federation.join: this hub was booted without the trust-write surface",
            )
        })?;
        validate_federation_join_request(&request, &ctx.daemon_realm)?;
        let public_key_b64 = public_key_hex_to_b64(&request.public_key_hex)?;
        let owner =
            admitted_join_principal_owner(&request, self.identity.principal_lifecycle.as_ref())?;
        RuntimeTrust::new(&ctx.daemon_realm, &ctx.trust_anchor_path, &ctx.cell)
            .register_pubkey_with_owner(
                request.membership_ura.clone(),
                public_key_b64,
                TrustedAgentRole::Device,
                owner,
            )?;
        let response = federation_wrappers::handle_join(&request);
        encode_json_payload(&response)
    }

    pub(crate) fn dispatch_federation_advertise_agent(
        &self,
        arguments: &[u8],
        envelope: Option<&Envelope>,
    ) -> Result<Vec<u8>, Status> {
        let request: federation_wrappers::AdvertiseAgentRequest = parse_json_args(arguments)?;
        let envelope = envelope.ok_or_else(|| {
            Status::invalid_argument("federation.advertise_agent: envelope is required")
        })?;
        let trust_anchor = self.admission.trust_anchor_snapshot();
        let publication = HostedAgentPublication::verify(
            envelope,
            &request,
            trust_anchor.as_ref(),
            self.admission.daemon_ura(),
        )
        .map_err(|err| {
            Status::permission_denied(format!(
                "federation.advertise_agent: hosted publication authority denied: {err}"
            ))
        })?;
        self.persist_hosted_agent_publication(&request, publication)
    }

    pub(crate) fn dispatch_federation_advertise_agent_from_session(
        &self,
        arguments: &[u8],
        caller_device_ura: &str,
    ) -> Result<Vec<u8>, Status> {
        let request: federation_wrappers::AdvertiseAgentRequest = parse_json_args(arguments)?;
        let hub_ura = self.admission.daemon_ura().ok_or_else(|| {
            Status::failed_precondition(
                "federation.advertise_agent: session carrier requires the selected hub identity",
            )
        })?;
        let trust_anchor = self.admission.trust_anchor_snapshot();
        let publication = HostedAgentPublication::verify_admitted_session(
            caller_device_ura,
            hub_ura,
            &request,
            trust_anchor.as_ref(),
        )
        .map_err(|err| {
            Status::permission_denied(format!(
                "federation.advertise_agent: session publication authority denied: {err}"
            ))
        })?;
        self.persist_hosted_agent_publication(&request, publication)
    }

    fn persist_hosted_agent_publication(
        &self,
        request: &federation_wrappers::AdvertiseAgentRequest,
        publication: HostedAgentPublication,
    ) -> Result<Vec<u8>, Status> {
        let ctx = self.identity.runtime_trust.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "federation.advertise_agent: this hub was booted without the trust-write surface",
            )
        })?;
        RuntimeTrust::new(&ctx.daemon_realm, &ctx.trust_anchor_path, &ctx.cell)
            .bind_principal_owner(publication.into_owner_binding(
                crate::daemon::invocation::admission::runtime_trust::now_unix_ms(),
            ))?;
        let response = federation_wrappers::handle_advertise_agent(
            request,
            Some(self.directory.advertised_agents.as_ref()),
        )
        .map_err(|error| {
            Status::failed_precondition(format!(
                "federation.advertise_agent durable inventory failed: {error:#}"
            ))
        })?;
        encode_json_payload(&response)
    }

    pub(crate) fn dispatch_federation_advertise_abilities(
        &self,
        arguments: &[u8],
        envelope: Option<&Envelope>,
    ) -> Result<Vec<u8>, Status> {
        let request: federation_wrappers::AdvertiseAbilitiesRequest = parse_json_args(arguments)?;
        let envelope = envelope.ok_or_else(|| {
            Status::invalid_argument("federation.advertise_abilities: envelope is required")
        })?;
        let trust_anchor = self.admission.trust_anchor_snapshot();
        OwnerProjectionPublicationAuthority::verify(
            envelope,
            &request,
            self.directory.advertised_agents.as_ref(),
            trust_anchor.as_ref(),
            self.admission.daemon_ura(),
        )
        .map_err(|err| {
            Status::permission_denied(format!(
                "federation.advertise_abilities: publication authority denied: {err}"
            ))
        })?;
        self.persist_owner_projection(&request)
    }

    pub(crate) fn dispatch_federation_advertise_abilities_from_session(
        &self,
        arguments: &[u8],
        caller_device_ura: &str,
    ) -> Result<Vec<u8>, Status> {
        let request: federation_wrappers::AdvertiseAbilitiesRequest = parse_json_args(arguments)?;
        let hub_ura = self.admission.daemon_ura().ok_or_else(|| {
            Status::failed_precondition(
                "federation.advertise_abilities: session carrier requires the selected hub identity",
            )
        })?;
        let trust_anchor = self.admission.trust_anchor_snapshot();
        OwnerProjectionPublicationAuthority::verify_admitted_session(
            caller_device_ura,
            hub_ura,
            &request,
            self.directory.advertised_agents.as_ref(),
            trust_anchor.as_ref(),
        )
        .map_err(|err| {
            Status::permission_denied(format!(
                "federation.advertise_abilities: session publication authority denied: {err}"
            ))
        })?;
        self.persist_owner_projection(&request)
    }

    fn persist_owner_projection(
        &self,
        request: &federation_wrappers::AdvertiseAbilitiesRequest,
    ) -> Result<Vec<u8>, Status> {
        if let Some(delivery) = &request.purge_delivery {
            crate::daemon::persistence::federation_revoke::record_projection_delivery(
                &crate::daemon::persistence::federation_revoke::PurgeProjectionDeliveryCommand {
                    protocol_version: delivery.protocol_version,
                    transaction_id: delivery.transaction_id.clone(),
                    owner_ura: request.owner_ura.clone(),
                    generation: request.generation,
                    projection_revision: request.projection_revision,
                    projection_digest: request.projection_digest.clone(),
                    authority_ura: request.host_device_ura.clone(),
                },
                delivery.delivery_fence,
            )
            .map_err(|error| {
                Status::failed_precondition(format!(
                    "federation.advertise_abilities purge fence rejected: {error:#}"
                ))
            })?;
        }
        let response = federation_wrappers::handle_advertise_abilities(
            request,
            Some(self.directory.ability_catalog.as_ref()),
        );
        encode_json_payload(&response)
    }

    pub(crate) fn dispatch_federation_heartbeat(
        &self,
        arguments: &[u8],
    ) -> Result<Vec<u8>, Status> {
        let request: federation_wrappers::HeartbeatRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_heartbeat(
            &request,
            &self.directory.presence,
            Some(self.directory.ability_catalog.as_ref()),
            now_unix_ms(),
        );
        encode_json_payload(&response)
    }

    pub(crate) fn dispatch_federation_status(&self) -> Result<Vec<u8>, Status> {
        let response = federation_wrappers::handle_status();
        encode_json_payload(&response)
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
    async fn resolve_canonical_rpc_route(
        &self,
        request: &InvokeRequest,
    ) -> Result<CanonicalRouteSelection, Status> {
        let target_ura = local_invoke_target_ura(request)?;
        let ability = function_name_from_invocation_target("Invoke", request.target.as_ref())?;
        if ability.is_empty() {
            return Err(Status::invalid_argument(
                "Invoke request missing function_name for namespace.resolve",
            ));
        }

        let selection = self
            .gate
            .route_resolver()
            .await
            .resolve_canonical_route(&target_ura, ability, CallMode::Rpc)
            .map_err(route_negative_status)?;

        if let CanonicalRouteDispatch::Local(selected_route) = selection.dispatch() {
            if !selected_route.is_authoritative_local_or_better() {
                return Err(route_profile_blocked_status(selected_route));
            }
        }
        Ok(selection)
    }

    pub(crate) async fn dispatch_local_rpc_selected_route(
        &self,
        request: &InvokeRequest,
    ) -> (Result<Response<InvokeResponse>, Status>, bool) {
        let ability = match function_name_from_invocation_target("Invoke", request.target.as_ref())
        {
            Ok(ability) => ability,
            Err(status) => return (Err(status), false),
        };
        let arguments = request.arguments.as_slice();
        let selection = match self.resolve_canonical_rpc_route(request).await {
            Ok(selection) => selection,
            Err(status) => {
                if let Some(handle) = self.sessions.escalation.as_ref() {
                    return (
                        self.escalate_canonical_invoke(handle, request, status)
                            .await,
                        false,
                    );
                }
                return (Err(status), false);
            }
        };
        let call_mode = selection.call_mode();
        let selected_route = match selection.into_dispatch() {
            CanonicalRouteDispatch::Local(route) => route,
            CanonicalRouteDispatch::Peer(route) => {
                return (
                    self.dispatch_peer_canonical_invoke(request, &route).await,
                    false,
                )
            }
        };
        // step-4 / T2.1b: locality is the daemon's decision, not the
        // caller's. A resolver-selected remote host dispatches through
        // that device's `session.open`. Like the federation-wrapper
        // arms, this is a service-handler path — the manual unary
        // record runs (axon_took_it = false); the executing device's
        // own runtime holds the canonical ledger row.
        let execution_host_is_self = self
            .gate
            .matches_self_target_ura(&selected_route.execution_host_ura)
            .await;
        if selected_route
            .dispatch_target(execution_host_is_self)
            .is_presence_session()
        {
            return (
                self.dispatch_remote_rpc_selected_route(request, &selected_route, call_mode)
                    .await,
                false,
            );
        }
        let Some(runtime) = self.runtime.local_runtime() else {
            return (
                Err(Status::failed_precondition(format!(
                    "easynet-daemon: ability `{ability}` cannot run because Axon LocalRuntime \
                     is not wired at boot"
                ))),
                false,
            );
        };
        let bound_ability = match RuntimeBoundAbility::from_selected_route(
            "easynet-daemon",
            &runtime,
            self.directory.local_ability_catalog.as_deref(),
            &selected_route,
            call_mode,
        )
        .await
        {
            Ok(bound_ability) => bound_ability,
            Err(status) => return (Err(status), false),
        };
        let selected_ability_ura = selected_route.ability_ura.clone();
        let runtime_descriptor_ref = match bound_ability.descriptor_ref_for_mode(
            "easynet-daemon",
            &selected_route.callee_ura,
            call_mode,
            Some(&selected_route.route_ura),
        ) {
            Ok(ref_) => ref_.into_descriptor_ref(),
            Err(status) => return (Err(status), false),
        };
        if let Err(status) = bound_ability.require_wire_target_matches(
            "Invoke",
            &selected_route.callee_ura,
            ability,
            &selected_route.route_ura,
        ) {
            return (Err(status), false);
        }
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
        let local_system_ingress = self
            .admission
            .accepts_local_system_envelope(request.envelope.as_ref());
        let wire = match request.envelope.clone() {
            Some(envelope) if local_system_ingress => {
                let metadata = match HostedAgentDelegationIssuer::materialize_request_metadata(
                    &request.metadata,
                    &envelope,
                    true,
                    ability,
                ) {
                    Ok(metadata) => metadata,
                    Err(status) => return (Err(status), false),
                };
                crate::daemon::axon_bridge::dispatch_shim::local_system_from_wire_parts(
                    envelope,
                    runtime_descriptor_ref,
                    arguments.to_vec(),
                    metadata,
                )
            }
            Some(envelope) => {
                let metadata = match HostedAgentDelegationIssuer::materialize_request_metadata(
                    &request.metadata,
                    &envelope,
                    false,
                    ability,
                ) {
                    Ok(metadata) => metadata,
                    Err(status) => return (Err(status), false),
                };
                let signed_descriptor_ref = match bound_ability.signed_descriptor_ref_from_target(
                    "Invoke",
                    &selected_route.callee_ura,
                    call_mode,
                    request.target.as_ref(),
                ) {
                    Ok(ref_) => ref_.into_descriptor_ref(),
                    Err(status) => return (Err(status), false),
                };
                crate::daemon::axon_bridge::dispatch_shim::external_signed_from_wire_parts(
                    envelope,
                    signed_descriptor_ref,
                    arguments.to_vec(),
                    metadata,
                )
            }
            None => Err(Box::new(axon_sdk::invocation::AxonError::invalid_argument(
                "Invoke request missing envelope",
            ))),
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
        let product_admission = match self.runtime.stage_product_admission(
            &self.admission,
            &wire,
            ability,
            CallMode::Rpc,
        ) {
            Ok(lease) => lease,
            Err(status) => return (Err(status), false),
        };
        let outcome = crate::daemon::axon_bridge::dispatch_shim::dispatch_rpc_admitted(
            &runtime,
            wire,
            &self.runtime.cancellations,
        )
        .await;
        let runtime_started = outcome.invocation_id.is_some();
        if runtime_started {
            if let Err(status) = product_admission.commit() {
                return (Err(status), true);
            }
        }
        rpc_dispatch_outcome_response(outcome)
    }

    pub(crate) fn dispatch_register_device_pubkey(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Vec<u8>, Status> {
        let ctx = self.identity.runtime_trust.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "identity.register_pubkey: this daemon was booted without the trust-write \
                 surface (use `with_register_pubkey(...)` at boot to enable). PR-7 production \
                 daemons always wire this; an unwired daemon is a smoke-test or fixture build.",
            )
        })?;
        let intent = parse_register_pubkey_intent(arguments)?;
        let write_gate =
            crate::daemon::invocation::admission::identity_write_gate::IdentityWriteGate::new(
                self.admission.trust_anchor_snapshot(),
                self.admission.daemon_ura().map(str::to_string),
                self.admission.transport_boundary(),
                ctx.daemon_realm.clone(),
            );
        write_gate.authorize_register_pubkey(caller_envelope, &intent)?;
        let body = handle_register_device_pubkey(
            arguments,
            &ctx.daemon_realm,
            &ctx.trust_anchor_path,
            &ctx.cell,
        )?;
        Ok(body)
    }

    /// DEC-EU §revocation. Same trust-write ctx the register ability
    /// uses; the revoke surface only mutates user-role entries.
    pub(crate) fn dispatch_revoke_user_pubkey(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Vec<u8>, Status> {
        let ctx = self.identity.runtime_trust.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "identity.revoke_user_pubkey: this daemon was booted without the trust-write \
                 surface (use `with_register_pubkey(...)` at boot to enable).",
            )
        })?;
        let intent = parse_revoke_user_pubkey_intent(arguments)?;
        let write_gate =
            crate::daemon::invocation::admission::identity_write_gate::IdentityWriteGate::new(
                self.admission.trust_anchor_snapshot(),
                self.admission.daemon_ura().map(str::to_string),
                self.admission.transport_boundary(),
                ctx.daemon_realm.clone(),
            );
        write_gate.authorize_revoke_user_pubkey(caller_envelope, &intent)?;
        let outcome = handle_revoke_user_pubkey_with_outcome(
            arguments,
            &ctx.daemon_realm,
            &ctx.trust_anchor_path,
            &ctx.cell,
        )?;
        let invalidation = RuntimeTrustInvalidator::new(
            self.directory.presence.clone(),
            self.directory.advertised_agents.clone(),
        )
        .with_connection_state_projector(
            RuntimeTrustConnectionStateProjector::from_local_credentials("daemon.runtime_trust"),
        )
        .invalidate_revoked_subject(
            intent.agent_ura(),
            Some(intent.public_key_b64()),
            outcome.removed,
        );
        if invalidation.removed_any_presence() || invalidation.removed_any_hosted_agent() {
            crate::op_event!(
                component = daemon_invocation,
                kind = runtime_trust_revoke_invalidated_presence,
                subject_ura = intent.agent_ura(),
                direct_presence_removed = invalidation.direct_presence_removed,
                hosted_agents_removed = invalidation.hosted_agents_removed,
                hosted_hosts_revoked = invalidation.hosted_hosts_revoked,
                connection_state_recorded = invalidation.connection_state_recorded,
            );
        }
        Ok(outcome.body)
    }

    /// DEC-EU §multi-host-list. Read-only inventory of user-role
    /// pubkeys. Uses the same cell as register/revoke so list
    /// results always agree with the in-memory authoritative state
    /// admission consults.
    pub(crate) fn dispatch_list_user_pubkeys(&self, arguments: &[u8]) -> Result<Vec<u8>, Status> {
        let ctx = self.identity.runtime_trust.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "identity.list_user_pubkeys: this daemon was booted without the trust \
                 surface; no listing available.",
            )
        })?;
        let body = handle_list_user_pubkeys(arguments, ctx.reader())?;
        Ok(body)
    }

    pub(crate) fn dispatch_principal_lifecycle(
        &self,
        ability: &str,
        arguments: &[u8],
    ) -> Result<Vec<u8>, Status> {
        let ctx = self.identity.principal_lifecycle.as_ref().ok_or_else(|| {
            Status::failed_precondition(
                "principal.lifecycle: this daemon was booted without the durable PrincipalLifecycle provider",
            )
        })?;
        let body = ctx.handle(ability, arguments)?;
        Ok(body)
    }

    pub(crate) fn dispatch_federation_resolve(&self, arguments: &[u8]) -> Result<Vec<u8>, Status> {
        let request: federation_wrappers::ResolveRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_resolve(
            &request,
            &self.directory.presence,
            Some(self.directory.advertised_agents.as_ref()),
            Some(self.directory.ability_catalog.as_ref()),
            self.directory.local_ability_catalog.as_deref(),
        );
        encode_json_payload(&response)
    }

    pub(crate) async fn dispatch_namespace_resolve(
        &self,
        arguments: &[u8],
    ) -> Result<Vec<u8>, Status> {
        let request: serde_json::Value = parse_json_args(arguments)?;
        let response = self
            .gate
            .route_resolver()
            .await
            .resolve_query_json(&request);
        encode_json_payload(&response)
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
    ) -> Result<Vec<u8>, Status> {
        let request: federation_wrappers::ResolveKeyRequest = parse_json_args(arguments)?;
        let trust_anchor = self.admission.trust_anchor_snapshot();
        match federation_wrappers::handle_resolve_key(&request, &trust_anchor) {
            Some(response) => encode_json_payload(&response),
            None => {
                let presented_pubkey_b64 = request
                    .presented_pubkey_b64
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .or_else(|| {
                        request
                            .presented_pubkey_hex
                            .as_deref()
                            .filter(|value| !value.is_empty())
                            .and_then(|hex| hex::decode(hex).ok())
                            .map(|raw| B64.encode(raw))
                    });
                let resolved = self.admission.resolve_federated_key_b64(
                    &request.agent_ura,
                    presented_pubkey_b64.as_deref(),
                )?;
                match resolved {
                    Some(public_key_b64) => {
                        encode_json_payload(&federation_wrappers::resolve_key_response(
                            &public_key_b64,
                            Vec::new(),
                            None,
                        ))
                    }
                    None => Err(Status::not_found(format!(
                        "federation.resolve_key: agent_ura `{}` not in this hub's trust set",
                        request.agent_ura
                    ))),
                }
            }
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
    pub(crate) fn dispatch_federation_discover(&self, arguments: &[u8]) -> Result<Vec<u8>, Status> {
        let request: federation_wrappers::DiscoverRequest = parse_json_args(arguments)?;
        let federated_response = match (
            request.local_user_id.as_deref(),
            self.directory.federated_bindings.as_ref(),
            self.identity.session_realm.as_deref(),
        ) {
            (Some(_user_id), Some(bindings), Some(realm)) => {
                let resolver = crate::daemon::keyring::resolver::FederatedUserResolver::new(
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
        let response =
            self.merge_local_presence_into_discover_response(&request, federated_response);
        encode_json_payload(&response)
    }

    fn merge_local_presence_into_discover_response(
        &self,
        request: &federation_wrappers::DiscoverRequest,
        federated_response: federation_wrappers::DiscoverResponse,
    ) -> federation_wrappers::DiscoverResponse {
        let mut by_agent_ura = BTreeMap::new();
        if let Some(realm) = self
            .identity
            .session_realm
            .as_deref()
            .filter(|realm| !realm.trim().is_empty())
        {
            let local = federation_wrappers::handle_list_user_devices(
                &federation_wrappers::ListUserDevicesRequest {
                    realm: realm.to_string(),
                },
                &self.directory.presence,
            );
            for entry in local.devices {
                if request
                    .agent_ura
                    .as_deref()
                    .is_some_and(|filter| filter != entry.agent_ura)
                {
                    continue;
                }
                by_agent_ura.insert(entry.agent_ura.clone(), entry);
            }
        }
        for entry in federated_response.entries {
            by_agent_ura.entry(entry.agent_ura.clone()).or_insert(entry);
        }
        federation_wrappers::DiscoverResponse {
            entries: by_agent_ura.into_values().collect(),
        }
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
    /// Local self admission: the daemon's own URA is admitted into every
    /// dispatch arm regardless of role only after the transport policy gate has
    /// accepted it on the local-only IPC boundary.
    pub(crate) fn dispatch_federation_list_user_devices(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Vec<u8>, Status> {
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
                crate::daemon::trust::anchor::TrustedAgentRole::Hub
            )
        });
        let is_local_self = self
            .admission
            .daemon_ura()
            .is_some_and(|self_ura| self_ura == caller_ura);
        if !(is_hub_role || is_local_self) {
            return Err(Status::permission_denied(format!(
                "federation.list_user_devices: caller `{caller_ura}` is not a hub-role peer; \
                 only trusted hubs and the daemon itself may enumerate user devices"
            )));
        }

        let request: federation_wrappers::ListUserDevicesRequest = parse_json_args(arguments)?;
        let response =
            federation_wrappers::handle_list_user_devices(&request, &self.directory.presence);
        encode_json_payload(&response)
    }

    pub(crate) fn require_backend_or_local_self_proxy_caller(
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
                crate::daemon::trust::anchor::TrustedAgentRole::Backend
            )
        });
        let is_local_hub_identity = self
            .identity
            .session_realm
            .as_deref()
            .is_some_and(|realm| crate::core::ura::hub_ura(realm) == caller_ura);
        let is_local_hub_role = is_local_hub_identity
            && trusted_entry.is_some_and(|entry| {
                matches!(
                    entry.role,
                    crate::daemon::trust::anchor::TrustedAgentRole::Backend
                        | crate::daemon::trust::anchor::TrustedAgentRole::Hub
                )
            });
        let is_local_self = self
            .admission
            .daemon_ura()
            .is_some_and(|self_ura| self_ura == caller_ura);
        if !(is_backend_role || is_local_hub_role || is_local_self) {
            return Err(Status::permission_denied(format!(
                "{ability_name}: caller `{caller_ura}` is not the local backend; \
                 only the backend and daemon local self caller may proxy peer calls"
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
    ) -> Result<Vec<u8>, Status> {
        self.require_backend_or_local_self_proxy_caller(
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
            return encode_json_payload(&federation_wrappers::ProxyListUserDevicesResponse {
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
            return encode_json_payload(&federation_wrappers::ProxyListUserDevicesResponse {
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
            let peer_request = PeerInvokeRequest::new(
                caller_envelope,
                &peer_entry.agent_ura,
                ABILITY_FEDERATION_LIST_USER_DEVICES,
                inner_arguments.clone(),
                local_realm,
                self.federation.hub_signer.as_deref(),
            )
            .into_invoke_request()
            .await?;
            fanout.push(async move {
                match client.invoke(&peer_hub_url, peer_request).await {
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

        encode_json_payload(&federation_wrappers::ProxyListUserDevicesResponse { devices })
    }

    pub(crate) async fn dispatch_namespace_proxy_resolve(
        &self,
        caller_envelope: Option<&Envelope>,
        arguments: &[u8],
    ) -> Result<Vec<u8>, Status> {
        self.require_backend_or_local_self_proxy_caller(
            caller_envelope,
            "namespace.proxy_resolve",
        )?;

        let request: federation_wrappers::NamespaceProxyResolveRequest =
            parse_json_args(arguments)?;
        let Some(client) = self.federation.client.as_ref() else {
            return encode_json_payload(&namespace_proxy_resolve_empty_answer(&request));
        };

        let peer_hub_urls = sorted_non_empty_urls(request.peer_hub_urls.clone());
        if peer_hub_urls.is_empty() {
            return encode_json_payload(&namespace_proxy_resolve_empty_answer(&request));
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
            let peer_request = PeerInvokeRequest::new(
                caller_envelope,
                &peer_entry.agent_ura,
                ABILITY_NAMESPACE_RESOLVE,
                inner_arguments.clone(),
                local_realm,
                self.federation.hub_signer.as_deref(),
            )
            .into_invoke_request()
            .await?;
            fanout.push(async move {
                match client.invoke(&peer_hub_url, peer_request).await {
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

        encode_json_payload(&namespace_proxy_resolve_merge_answer(
            &request,
            peer_answers,
        ))
    }

    pub(crate) fn dispatch_federation_revoke(&self, arguments: &[u8]) -> Result<Vec<u8>, Status> {
        let request: federation_wrappers::RevokeRequest = parse_json_args(arguments)?;
        let response = federation_wrappers::handle_revoke(
            &request,
            &self.directory.presence,
            Some(self.directory.advertised_agents.as_ref()),
            Some(self.directory.ability_catalog.as_ref()),
        )
        .map_err(|error| {
            Status::failed_precondition(format!(
                "federation.revoke durable mutation failed: {error:#}"
            ))
        })?;
        encode_json_payload(&response)
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
    /// moment the host's `session.open` drops mid-call;
    /// [`PRESENCE_DISPATCH_REPLY_TIMEOUT`] backstops every reply that
    /// neither completes nor goes offline (structured
    /// `DeadlineExceeded` instead of an open-ended hang).
    async fn dispatch_frame_to_presence(
        &self,
        selected_route: &SelectedInvokeRoute,
        label: &str,
        build_frame: impl FnOnce(
            u64,
        ) -> Result<
            crate::daemon::invocation::bidi::state::presence::DispatchFrame,
            Status,
        >,
    ) -> Result<(u64, DispatchResult, u32), Status> {
        // Self guard: in device mode the boot seed registers a
        // resolve-only no-op presence entry under the daemon's own URA
        // (boot/presence_seed.rs) whose drain task accepts every frame
        // and never completes the pending entry — a frame dispatched
        // there parks the waiter until the deadline. Self-targeted
        // invocations belong to the local-runtime arms; refuse loudly
        // here rather than queue a frame that can never be answered.
        self.reject_self_presence_host(selected_route, label)?;
        let pending = self.sessions.pending.as_ref().ok_or_else(|| {
            Status::failed_precondition(format!(
                "{label}: daemon was constructed without a PendingDispatchMap; call \
                 DaemonInvocationService::with_pending(...) at boot to enable \
                 cross-device dispatch",
            ))
        })?;
        let session = require_canonical_dispatch_session(
            &self.directory.presence,
            &selected_route.execution_host_ura,
            &selected_route.route_ura,
            label,
        )?;
        let session_id = session.session_id;
        let sender = session.sender;
        let carrier_version = session.contract_version;
        let handle = pending.register_pending_for(&selected_route.execution_host_ura);
        let call_id = handle.call_id();
        let dispatch_frame = build_frame(call_id)?;
        match sender.try_send(Ok(dispatch_frame)) {
            Ok(()) => {}
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Full = device is slow, not dead: keep its session,
                // fail only this call as retryable backpressure.
                return Err(Status::resource_exhausted(
                    crate::daemon::invocation::bidi::state::presence::DISPATCH_TARGET_BUSY_REASON,
                ));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                self.directory.presence.remove_if_session(
                    &selected_route.execution_host_ura,
                    session_id,
                    crate::daemon::invocation::bidi::state::presence::OfflineReason::StreamClosed,
                );
                return Err(Status::failed_precondition(
                    crate::daemon::invocation::bidi::state::presence::DISPATCH_TARGET_OFFLINE_REASON,
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
        Ok((call_id, result, carrier_version))
    }

    fn reject_self_presence_host(
        &self,
        selected_route: &SelectedInvokeRoute,
        label: &str,
    ) -> Result<(), Status> {
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
        Ok(())
    }

    async fn dispatch_peer_canonical_invoke(
        &self,
        request: &InvokeRequest,
        route: &DelegatedInvokeRoute,
    ) -> Result<Response<InvokeResponse>, Status> {
        require_complete_signed_remote_request(request)?;
        let forwarded_binding = ForwardedInvocationBinding::from_request(request)?;
        let receipt_resolver = self.admission.receipt_key_resolver();
        let client = self.federation.client.as_ref().ok_or_else(|| {
            Status::failed_precondition("remote Invoke: federation dialer is not configured")
        })?;
        let endpoint = route
            .primary_endpoint()
            .ok_or_else(|| {
                Status::failed_precondition("remote Invoke: peer route has no usable endpoint")
            })?
            .to_string();
        crate::op_event!(
            component = daemon_invocation,
            kind = canonical_invoke_peer_delegation,
            callee_ura = route.owner_ura.as_str(),
            peer_hub_ura = route.hub_ura.as_str(),
            endpoint = endpoint.as_str(),
        );
        match client.invoke(&endpoint, request.clone()).await {
            Ok(response) => {
                ensure_forwarded_response_receipt_signer_keys(
                    self.sessions.device_trust_sync.as_ref(),
                    &response,
                    "remote Invoke peer delegation",
                )
                .await?;
                let finalized = ForwardedFinalizedInvocation::verify_response(
                    &forwarded_binding,
                    response,
                    receipt_resolver.as_ref(),
                )?;
                Ok(Response::new(finalized.into_response()))
            }
            Err(err) => {
                if let Some(status) = target_admission_denial_status(&err) {
                    return Err(status);
                }
                Err(Status::unavailable(format!(
                    "remote Invoke peer delegation to `{endpoint}` failed: {err}"
                )))
            }
        }
    }

    async fn escalate_canonical_invoke(
        &self,
        handle: &Arc<crate::daemon::invocation::bidi::session_escalation::SessionEscalationHandle>,
        request: &InvokeRequest,
        local_route_failure: Status,
    ) -> Result<Response<InvokeResponse>, Status> {
        if matches!(
            local_route_failure.code(),
            tonic::Code::InvalidArgument | tonic::Code::PermissionDenied
        ) {
            return Err(local_route_failure);
        }
        require_complete_signed_remote_request(request)?;
        let forwarded_binding = ForwardedInvocationBinding::from_request(request)?;
        let receipt_resolver = self.admission.receipt_key_resolver();
        match handle.escalate_invoke(request.clone()).await {
            Ok(response) => {
                ensure_forwarded_response_receipt_signer_keys(
                    self.sessions.device_trust_sync.as_ref(),
                    &response,
                    "remote Invoke session escalation",
                )
                .await?;
                let finalized = ForwardedFinalizedInvocation::verify_response(
                    &forwarded_binding,
                    response,
                    receipt_resolver.as_ref(),
                )?;
                Ok(Response::new(finalized.into_response()))
            }
            Err(SessionRequestError::TargetOffline) => Err(Status::failed_precondition(
                "remote Invoke target is offline",
            )),
            Err(SessionRequestError::PermissionDenied { reason }) => {
                Err(Status::permission_denied(reason))
            }
            Err(SessionRequestError::UpstreamFailure { reason }) => Err(Status::unavailable(
                format!("remote Invoke session escalation failed: {reason}"),
            )),
            Err(SessionRequestError::UpstreamTimeout) => Err(Status::deadline_exceeded(
                "remote Invoke session escalation timed out",
            )),
        }
    }

    /// step-4 / T2.1b: the canonical-face remote arm. A catch-all
    /// `Invoke` whose resolver-selected execution host is another
    /// device dispatches through that device's `session.open` — the
    /// same single-settle core as invoke, but the caller's
    /// envelope travels verbatim (transplant, not translation):
    /// content, authority fields and the presigned caller signature
    /// are already ON the request, so nothing is wrapped and nothing
    /// is re-minted. Sessions that did not negotiate the canonical
    /// carrier fail closed; there is no field-projected fallback.
    pub(crate) async fn dispatch_remote_rpc_selected_route(
        &self,
        request: &InvokeRequest,
        selected_route: &SelectedInvokeRoute,
        call_mode: CallMode,
    ) -> Result<Response<InvokeResponse>, Status> {
        let ability =
            function_name_from_invocation_target("Invoke", request.target.as_ref())?.to_string();
        let Some(envelope) = request.envelope.clone() else {
            return Err(Status::invalid_argument(format!(
                "Invoke: remote-hosted ability `{ability}` requires the seven-tuple \
                 envelope on the canonical Invocation face",
            )));
        };
        require_complete_signed_remote_request(request)?;
        signed_envelope_for_selected_route(
            envelope,
            selected_route,
            request.target.as_ref(),
            &request.arguments,
        )?;
        let forwarded_binding = ForwardedInvocationBinding::from_request(request)?;
        ensure_forwarded_receipt_signer_key(
            self.sessions.device_trust_sync.as_ref(),
            &selected_route.execution_host_ura,
            "Invoke",
        )
        .await?;
        let receipt_resolver = self.admission.receipt_key_resolver();
        self.reject_self_presence_host(selected_route, "Invoke")?;
        let (_call_id, dispatch_result, carrier_version) = self
            .dispatch_frame_to_presence(selected_route, "Invoke", |call_id| {
                Ok(build_carrier_v1_dispatch_frame(
                    call_id,
                    request.clone(),
                    matches!(call_mode, CallMode::Bidi),
                ))
            })
            .await?;
        crate::op_event!(
            component = daemon_invocation,
            kind = remote_rpc_selected_route_dispatch,
            ability = ability.as_str(),
            dispatch_ability = selected_route.ability_ura.as_str(),
            callee_ura = selected_route.callee_ura.as_str(),
            execution_host_ura = selected_route.execution_host_ura.as_str(),
            route_ura = selected_route.route_ura.as_str(),
            carrier_version = carrier_version,
        );
        let DispatchResult {
            admission_receipt,
            terminal_receipt,
            payload,
            result_content_type,
            error,
            failure,
            request_id: _,
        } = dispatch_result;
        if let Some(err) =
            error.filter(|_| admission_receipt.is_none() || terminal_receipt.is_none())
        {
            return Err(status_from_remote_failure(
                &format!(
                    "Invoke: remote route `{}` ability `{}` failed",
                    selected_route.route_ura, selected_route.dispatch_name,
                ),
                &err,
                failure.as_ref(),
            ));
        }
        let admission_receipt = admission_receipt.ok_or_else(|| {
            Status::failed_precondition(
                "CANONICAL_ADMISSION_REQUIRED: remote unary omitted admission checkpoint",
            )
        })?;
        let terminal_receipt = terminal_receipt.ok_or_else(|| {
            Status::failed_precondition(
                "CANONICAL_TERMINAL_RECEIPT_REQUIRED: remote unary omitted terminal checkpoint",
            )
        })?;
        let finalized = ForwardedFinalizedInvocation::verify_with_carrier_result(
            &forwarded_binding,
            admission_receipt,
            terminal_receipt,
            payload,
            result_content_type,
            receipt_resolver.as_ref(),
        )?;
        Ok(Response::new(finalized.into_response()))
    }
}

fn local_invoke_target_ura(request: &InvokeRequest) -> Result<String, Status> {
    target_ura_from_envelope(request.envelope.as_ref(), "Invoke")
}

pub(crate) fn require_complete_signed_remote_request(
    request: &InvokeRequest,
) -> Result<(), Status> {
    function_name_from_invocation_target("remote Invoke", request.target.as_ref())?;
    let envelope = request.envelope.as_ref().ok_or_else(|| {
        Status::invalid_argument("remote Invoke requires the complete seven-tuple envelope")
    })?;
    let caller = envelope
        .caller
        .as_ref()
        .map(|identity| identity.ura.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Status::invalid_argument("remote Invoke envelope is missing caller"))?;
    let callee = envelope
        .callee
        .as_ref()
        .map(|identity| identity.ura.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Status::invalid_argument("remote Invoke envelope is missing callee"))?;
    envelope
        .subject
        .as_ref()
        .map(|identity| identity.ura.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Status::invalid_argument("remote Invoke envelope is missing subject"))?;
    if envelope.invocation_nonce.len() != 16 {
        return Err(Status::invalid_argument(format!(
            "remote Invoke nonce must be 16 bytes, got {}",
            envelope.invocation_nonce.len()
        )));
    }
    if envelope.causal_context.is_none() {
        return Err(Status::invalid_argument(
            "remote Invoke envelope is missing causal context",
        ));
    }
    let signature = envelope.caller_signature.as_ref().ok_or_else(|| {
        Status::permission_denied(format!(
            "remote Invoke from `{caller}` requires an explicit caller signature"
        ))
    })?;
    if signature.signature.is_empty() {
        return Err(Status::permission_denied(format!(
            "remote Invoke from `{caller}` carries an empty caller signature"
        )));
    }
    let descriptor_ref =
        descriptor_ref_from_invocation_target("remote Invoke", callee, request.target.as_ref())
            .map_err(|status| {
                Status::invalid_argument(format!(
                    "{}: {}",
                    SignatureDecisionReason::SignedDescriptorRefMissing.as_str(),
                    status.message()
                ))
            })?;
    let ability_ura = crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
        &descriptor_ref,
    )
    .map_err(|err| Status::invalid_argument(format!("remote Invoke descriptor: {err}")))?;
    let selector = crate::core::ura::AbilitySelector::parse(&ability_ura).map_err(|err| {
        Status::invalid_argument(format!("remote Invoke ability URA is invalid: {err}"))
    })?;
    if selector.owner_ura() != callee {
        return Err(Status::permission_denied(format!(
            "remote Invoke descriptor owner `{}` does not match callee `{callee}`",
            selector.owner_ura()
        )));
    }
    Ok(())
}

fn target_admission_denial_status(error: &FederationClientError) -> Option<Status> {
    let FederationClientError::InnerInvokeFailed { status, .. } = error else {
        return None;
    };
    if !is_admission_denial_message(status) {
        return None;
    }
    if status.contains("code=InvalidArgument") {
        Some(Status::invalid_argument(status.clone()))
    } else {
        Some(Status::permission_denied(status.clone()))
    }
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
        "query_name": non_empty_json_string(&request.query_name),
        "qtype": non_empty_json_string(&request.qtype)
            .unwrap_or_else(|| "RESOLVE_TYPE_DIRECTORY_LISTING".to_string()),
        "caller_ura": non_empty_json_string(&request.caller_ura),
        "subject_ura": non_empty_json_string(&request.subject_ura),
        "realm_hint": non_empty_json_string(&request.realm_hint),
        "ability_name": non_empty_json_string(&request.ability_name),
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
        "answer_kind": "RESOLVE_ANSWER_KIND_NON_DISPATCHABLE",
        "canonical_name": non_empty_json_string(&request.query_name),
        "records": records.into_values().collect::<Vec<_>>(),
        "release_profile": "RESOLVER_RELEASE_PROFILE_PRODUCTION",
        "cache_policy": {
            "ttl_ms": 0,
            "shared_cacheable": false,
            "retry_after_unix_ms": 0,
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
