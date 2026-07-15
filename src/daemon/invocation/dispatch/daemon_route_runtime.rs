// EasyNet CLI - daemon unary route runtime adapter
// =================================================
//
// File: src/daemon/invocation/dispatch/daemon_route_runtime.rs
// Description: Registers daemon-owned exact unary routes in the shared Axon
//              LocalRuntime and dispatches admitted gRPC requests through the
//              descriptor-bound runtime API.
//
// Protocol Responsibility:
// - Preserve the caller's descriptor-bound seven-tuple at the runtime boundary.
// - Make Axon LocalRuntime the sole owner of admission and terminal receipts.
//
// Implementation Approach:
// - Register all DaemonUnaryRoute handlers atomically as owner-bound RPC
//   abilities backed by one DaemonUnaryRouteProvider.
// - Resolve registration proof facts once, then drain only Axon's canonical
//   finalized handle when projecting an InvokeResponse.
//
// Usage Contract:
// - Boot must call register before exposing either invocation listener.
// - Product handlers return payload bytes or AxonError; they never construct
//   receipt or terminal state.
//
// Architectural Position:
// - Daemon transport/runtime adapter. Product behavior remains in
//   UnaryDispatcher; protocol lifecycle remains in Axon LocalRuntime.

use std::sync::Arc;

use easynet_axon::invocation::{
    make_ability, AbilityCallModes, AbilityOptions, AbilityRegistration, AuthorityBinding,
    AxonError, CallMode, CallerSignature, LocalRuntime,
};
use easynet_axon::pb::axon::v1::{Envelope, InvokeRequest, InvokeResponse};
use sha2::{Digest as _, Sha256};
use tonic::{Response, Status};

use crate::daemon::invocation::admission::hosted_agent_delegation::HostedAgentDelegationIssuer;
use crate::daemon::invocation::dispatch::cancellation::InvocationCancellationRegistry;
use crate::daemon::invocation::dispatch::daemon_invocation_service::DaemonUnaryRoute;
use crate::daemon::invocation::dispatch::descriptor_binding::RuntimeBoundAbility;
use crate::daemon::invocation::dispatch::invocation_wire::{
    status_from_axon_invoke_error, SIGNED_DESCRIPTOR_REF_METADATA_KEY,
};
use crate::daemon::invocation::dispatch::unary_dispatcher::{
    rpc_dispatch_outcome_response, DaemonUnaryRouteProvider,
};

const PRODUCT_GRPC_CODE_CONTEXT: &str = "easynet.daemon.product.grpc_code";

/// Runtime binding for the daemon's exact unary route family.
pub(crate) struct DaemonRouteRuntimeAdapter {
    runtime: Arc<LocalRuntime>,
    cancellations: InvocationCancellationRegistry,
}

/// Transport-origin fact selected before canonical runtime admission.
///
/// The value selects which public Axon request constructor is valid for this
/// ingress. Bootstrap carries only an immutable proof of the envelope's own
/// key/tuple binding; it is not an admitted identity or replay capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DaemonRouteIngress {
    ExternalSigned,
    TrustedLocalSystem,
    Bootstrap(ProvisionalJoinProof),
}

/// Self-contained federation bootstrap claim derived from the join key.
///
/// This value proves that the provisional caller digest, membership subject,
/// realm, route, and payload are one immutable claim. It is transport policy
/// context only: accepting its signature and nonce still belongs exclusively
/// to Axon LocalRuntime's bootstrap admission mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProvisionalJoinProof {
    caller_digest: [u8; 32],
    public_key: [u8; 32],
    membership_ura: String,
    realm: String,
    ability: &'static str,
    args_digest: [u8; 32],
}

impl ProvisionalJoinProof {
    pub(crate) fn verify(route: DaemonUnaryRoute, request: &InvokeRequest) -> Result<Self, Status> {
        if route != DaemonUnaryRoute::FederationJoin {
            return Err(Status::permission_denied(
                "provisional bootstrap is restricted to federation.join",
            ));
        }
        let envelope = request.envelope.as_ref().ok_or_else(|| {
            Status::invalid_argument("federation.join bootstrap envelope is required")
        })?;
        let join: crate::daemon::invocation::dispatch::federation_wrappers::JoinRequest =
            serde_json::from_slice(&request.arguments).map_err(|error| {
                Status::invalid_argument(format!(
                    "federation.join bootstrap arguments JSON decode failed: {error}"
                ))
            })?;
        let public_key_bytes = hex::decode(join.public_key_hex.trim()).map_err(|error| {
            Status::invalid_argument(format!(
                "federation.join bootstrap public_key_hex is invalid: {error}"
            ))
        })?;
        let public_key: [u8; 32] = public_key_bytes.try_into().map_err(|bytes: Vec<u8>| {
            Status::invalid_argument(format!(
                "federation.join bootstrap public key must be 32 bytes, got {}",
                bytes.len()
            ))
        })?;
        let expected_digest: [u8; 32] = Sha256::digest(public_key).into();
        let caller_digest = provisional_caller_digest(envelope)?;
        if !constant_time_eq_32(&caller_digest, &expected_digest) {
            return Err(Status::permission_denied(
                "federation.join provisional caller does not match join public key",
            ));
        }
        validate_join_tuple(envelope, &join)?;
        Ok(Self {
            caller_digest,
            public_key,
            membership_ura: join.membership_ura,
            realm: join.realm,
            ability: DaemonUnaryRoute::FederationJoin.name(),
            args_digest: Sha256::digest(&request.arguments).into(),
        })
    }

    fn validate_request(
        &self,
        route: DaemonUnaryRoute,
        request: &InvokeRequest,
    ) -> Result<(), Status> {
        if route.name() != self.ability {
            return Err(Status::permission_denied(
                "provisional bootstrap route binding mismatch",
            ));
        }
        let envelope = request.envelope.as_ref().ok_or_else(|| {
            Status::invalid_argument("federation.join bootstrap envelope is required")
        })?;
        let presented_caller_digest = provisional_caller_digest(envelope)?;
        let presented_args_digest: [u8; 32] = Sha256::digest(&request.arguments).into();
        if !constant_time_eq_32(&presented_caller_digest, &self.caller_digest)
            || !constant_time_eq_32(&presented_args_digest, &self.args_digest)
        {
            return Err(Status::permission_denied(
                "federation.join bootstrap claim changed after verification",
            ));
        }
        let join: crate::daemon::invocation::dispatch::federation_wrappers::JoinRequest =
            serde_json::from_slice(&request.arguments).map_err(|error| {
                Status::invalid_argument(format!(
                    "federation.join bootstrap arguments JSON decode failed: {error}"
                ))
            })?;
        validate_join_tuple(envelope, &join)?;
        if join.membership_ura != self.membership_ura
            || join.realm != self.realm
            || hex::decode(join.public_key_hex.trim()).ok().as_deref()
                != Some(self.public_key.as_slice())
        {
            return Err(Status::permission_denied(
                "federation.join bootstrap claim binding mismatch",
            ));
        }
        Ok(())
    }
}

impl DaemonRouteRuntimeAdapter {
    pub(crate) fn new(
        runtime: Arc<LocalRuntime>,
        cancellations: InvocationCancellationRegistry,
    ) -> Self {
        Self {
            runtime,
            cancellations,
        }
    }

    /// Atomically install the complete exact-route family. A partial route
    /// surface is never observable, including when registration collides.
    pub(crate) async fn register(
        &self,
        owner_ura: &str,
        catalog: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
        provider: DaemonUnaryRouteProvider,
    ) -> Result<(), AxonError> {
        let mut registrations = Vec::with_capacity(DaemonUnaryRoute::ALL.len());
        for route in DaemonUnaryRoute::ALL.iter().copied() {
            let ability_ura = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
                owner_ura,
                route.name(),
            )?;
            let route_provider = provider.clone();
            let function = make_ability(move |context| {
                let provider = route_provider.clone();
                async move { provider.invoke(route, context).await }
            });
            registrations.push(
                AbilityRegistration::new(ability_ura, function)
                    .with_options(route_registration_options(catalog, owner_ura, route)?),
            );
        }
        self.runtime.register_many(registrations).await
    }

    /// Execute one exact route through the same descriptor-bound runtime path
    /// used by normal local abilities. The returned response is only a
    /// projection of Axon's finalized outcome.
    pub(crate) async fn dispatch(
        &self,
        route: DaemonUnaryRoute,
        request: &InvokeRequest,
        ingress: DaemonRouteIngress,
    ) -> Result<Response<InvokeResponse>, Status> {
        let envelope = request.envelope.clone().ok_or_else(|| {
            Status::invalid_argument(format!("{}: envelope is required", route.name()))
        })?;
        let callee_ura = envelope
            .callee
            .as_ref()
            .map(|identity| identity.ura.trim())
            .filter(|ura| !ura.is_empty())
            .ok_or_else(|| {
                Status::invalid_argument(format!("{}: envelope callee is required", route.name()))
            })?;
        let bound = RuntimeBoundAbility::from_wire_target(
            "daemon exact unary route",
            self.runtime.as_ref(),
            callee_ura,
            route.name(),
        )
        .await?;
        let registered_ref = bound
            .descriptor_ref_for_mode("daemon exact unary route", callee_ura, CallMode::Rpc, None)?
            .into_descriptor_ref();

        let wire = match ingress {
            DaemonRouteIngress::Bootstrap(proof) => {
                proof.validate_request(route, request)?;
                let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                    &request.metadata,
                    &envelope,
                    false,
                    route.name(),
                )?;
                let signed_ref = bound
                    .signed_descriptor_ref_from_metadata(
                        "daemon exact unary route",
                        callee_ura,
                        CallMode::Rpc,
                        &metadata,
                    )?
                    .ok_or_else(|| {
                        Status::invalid_argument(format!(
                            "{}: signed bootstrap Invoke is missing `{SIGNED_DESCRIPTOR_REF_METADATA_KEY}`",
                            route.name()
                        ))
                    })?
                    .into_descriptor_ref();
                let caller_ura = envelope
                    .caller
                    .as_ref()
                    .map(|identity| identity.ura.trim().to_string())
                    .filter(|ura| !ura.is_empty())
                    .ok_or_else(|| {
                        Status::invalid_argument(format!(
                            "{}: bootstrap envelope caller is required",
                            route.name()
                        ))
                    })?;
                let ability_ura =
                    crate::daemon::axon_bridge::descriptor_ref::ability_ura_from_descriptor_ref(
                        &signed_ref,
                    )
                    .map_err(|error| {
                        Status::invalid_argument(format!(
                            "{}: bootstrap descriptor ref is invalid: {error}",
                            route.name()
                        ))
                    })?;
                let authority_binding = AuthorityBinding::Bootstrap {
                    principal_ura: caller_ura,
                    realm: proof.realm.clone(),
                    ability: ability_ura,
                };
                let wire =
                    crate::daemon::axon_bridge::dispatch_shim::bootstrap_preverified_from_wire_parts(
                        envelope,
                        signed_ref,
                        request.arguments.clone(),
                        metadata,
                        authority_binding,
                )
                .map_err(|error| status_from_axon_invoke_error("Invoke", route.name(), *error))?;
                verify_preverified_bootstrap_signature(route.name(), &wire, &proof.public_key)?;
                Ok(wire)
            }
            DaemonRouteIngress::TrustedLocalSystem => {
                let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                    &request.metadata,
                    &envelope,
                    true,
                    route.name(),
                )?;
                crate::daemon::axon_bridge::dispatch_shim::local_system_from_wire_parts(
                    envelope,
                    registered_ref,
                    request.arguments.clone(),
                    metadata,
                )
            }
            DaemonRouteIngress::ExternalSigned => {
                let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                    &request.metadata,
                    &envelope,
                    false,
                    route.name(),
                )?;
                let signed_ref = bound
                    .signed_descriptor_ref_from_metadata(
                        "daemon exact unary route",
                        callee_ura,
                        CallMode::Rpc,
                        &metadata,
                    )?
                    .ok_or_else(|| {
                        Status::invalid_argument(format!(
                            "{}: signed public Invoke is missing `{SIGNED_DESCRIPTOR_REF_METADATA_KEY}`",
                            route.name()
                        ))
                    })?
                    .into_descriptor_ref();
                crate::daemon::axon_bridge::dispatch_shim::external_signed_from_wire_parts(
                    envelope,
                    signed_ref,
                    request.arguments.clone(),
                    metadata,
                )
            }
        }
        .map_err(|error| status_from_axon_invoke_error("Invoke", route.name(), *error))?;

        let outcome = crate::daemon::axon_bridge::dispatch_shim::dispatch_rpc_admitted(
            &self.runtime,
            wire,
            &self.cancellations,
        )
        .await;
        daemon_route_outcome_response(route.name(), outcome)
    }
}

fn verify_preverified_bootstrap_signature(
    route_name: &str,
    wire: &crate::daemon::axon_bridge::dispatch_shim::WireDispatch,
    public_key: &[u8; 32],
) -> Result<(), Status> {
    let signature = match &wire.ingress {
        crate::daemon::axon_bridge::dispatch_shim::WireDispatchIngress::BootstrapPreverified {
            signature,
            ..
        } => signature,
        _ => {
            return Err(Status::internal(format!(
                "{route_name}: bootstrap verification received non-bootstrap ingress"
            )));
        }
    };
    verify_ed25519_descriptor_signature(route_name, &wire.envelope, signature, public_key)
}

fn verify_ed25519_descriptor_signature(
    route_name: &str,
    envelope: &easynet_axon::invocation::DescriptorBoundEnvelope,
    signature: &CallerSignature,
    public_key: &[u8; 32],
) -> Result<(), Status> {
    if signature.algorithm != "ed25519" {
        return Err(Status::permission_denied(format!(
            "{route_name}: bootstrap caller signature algorithm must be ed25519"
        )));
    }
    let signature_bytes: [u8; 64] = signature.signature.as_slice().try_into().map_err(|_| {
        Status::permission_denied(format!(
            "{route_name}: bootstrap caller signature must be 64 bytes"
        ))
    })?;
    let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(public_key).map_err(|error| {
        Status::permission_denied(format!(
            "{route_name}: bootstrap public key is invalid: {error}"
        ))
    })?;
    let signature = ed25519_dalek::Signature::from_bytes(&signature_bytes);
    ed25519_dalek::Verifier::verify(&verifying_key, &envelope.canonical_bytes(), &signature)
        .map_err(|_| {
            Status::permission_denied(format!(
                "{route_name}: bootstrap caller signature does not verify"
            ))
        })
}

fn provisional_caller_digest(envelope: &Envelope) -> Result<[u8; 32], Status> {
    let caller = envelope
        .caller
        .as_ref()
        .map(|caller| caller.ura.trim())
        .filter(|caller| !caller.is_empty())
        .ok_or_else(|| Status::invalid_argument("federation.join bootstrap caller is required"))?;
    let encoded = caller.strip_prefix("provisional:").ok_or_else(|| {
        Status::permission_denied("federation.join bootstrap caller must be provisional")
    })?;
    if encoded.len() != 64 || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Status::permission_denied(
            "federation.join provisional caller digest is malformed",
        ));
    }
    hex::decode(encoded)
        .map_err(|_| Status::permission_denied("federation.join provisional caller is malformed"))?
        .try_into()
        .map_err(|_| {
            Status::permission_denied("federation.join provisional digest length mismatch")
        })
}

fn validate_join_tuple(
    envelope: &Envelope,
    join: &crate::daemon::invocation::dispatch::federation_wrappers::JoinRequest,
) -> Result<(), Status> {
    let callee = envelope
        .callee
        .as_ref()
        .map(|callee| callee.ura.trim())
        .filter(|callee| !callee.is_empty())
        .ok_or_else(|| Status::invalid_argument("federation.join bootstrap callee is required"))?;
    let subject = envelope
        .subject
        .as_ref()
        .map(|subject| subject.ura.trim())
        .filter(|subject| !subject.is_empty())
        .ok_or_else(|| Status::invalid_argument("federation.join bootstrap subject is required"))?;
    let callee = crate::core::ura::parse_ura(callee).map_err(|error| {
        Status::invalid_argument(format!(
            "federation.join bootstrap callee is invalid: {error}"
        ))
    })?;
    let subject_parsed = crate::core::ura::parse_ura(subject).map_err(|error| {
        Status::invalid_argument(format!(
            "federation.join bootstrap subject is invalid: {error}"
        ))
    })?;
    if callee.kind != crate::core::ura::URAKind::Hub
        || subject_parsed.kind != crate::core::ura::URAKind::Device
        || join.membership_ura != subject
        || join.realm != callee.realm
        || join.realm != subject_parsed.realm
    {
        return Err(Status::permission_denied(
            "federation.join bootstrap membership/realm binding mismatch",
        ));
    }
    Ok(())
}

fn constant_time_eq_32(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right.iter())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod provisional_join_proof_tests {
    use super::*;

    fn join_request(public_key: [u8; 32]) -> InvokeRequest {
        let public_key_hex = hex::encode(public_key);
        let membership_ura = "easynet:///r/bootstrap-test/device/node-1";
        let caller = crate::core::ura::provisional::provisional_ura_for_pubkey(&public_key);
        let arguments = serde_json::to_vec(&serde_json::json!({
            "membership_ura": membership_ura,
            "realm": "bootstrap-test",
            "public_key_hex": public_key_hex,
        }))
        .expect("join arguments");
        crate::daemon::invocation::ProtoEnvelope::federation_join_genesis(
            caller,
            crate::core::ura::hub_ura("bootstrap-test"),
            membership_ura,
        )
        .expect("join envelope")
        .invoke_request(DaemonUnaryRoute::FederationJoin.name(), arguments)
        .expect("join request")
    }

    #[test]
    fn arbitrary_provisional_format_is_rejected() {
        let mut request = join_request([0x11; 32]);
        request
            .envelope
            .as_mut()
            .expect("envelope")
            .caller
            .as_mut()
            .expect("caller")
            .ura = "provisional:attacker-key".to_string();
        assert!(ProvisionalJoinProof::verify(DaemonUnaryRoute::FederationJoin, &request).is_err());
    }

    #[test]
    fn mismatched_join_key_is_rejected() {
        let mut request = join_request([0x22; 32]);
        let arguments = serde_json::to_vec(&serde_json::json!({
            "membership_ura": "easynet:///r/bootstrap-test/device/node-1",
            "realm": "bootstrap-test",
            "public_key_hex": hex::encode([0x23; 32]),
        }))
        .expect("substituted join arguments");
        request.arguments = arguments;
        assert!(ProvisionalJoinProof::verify(DaemonUnaryRoute::FederationJoin, &request).is_err());
    }

    #[test]
    fn payload_substitution_after_proof_is_rejected() {
        let mut request = join_request([0x33; 32]);
        let proof = ProvisionalJoinProof::verify(DaemonUnaryRoute::FederationJoin, &request)
            .expect("valid proof");
        request.arguments.push(b' ');
        assert!(proof
            .validate_request(DaemonUnaryRoute::FederationJoin, &request)
            .is_err());
    }

    #[test]
    fn provisional_identity_on_non_join_route_is_rejected() {
        let request = join_request([0x44; 32]);
        assert!(
            ProvisionalJoinProof::verify(DaemonUnaryRoute::FederationStatus, &request).is_err()
        );
    }
}

fn daemon_route_outcome_response(
    ability: &str,
    outcome: crate::daemon::axon_bridge::dispatch_shim::RpcDispatchOutcome,
) -> Result<Response<InvokeResponse>, Status> {
    if outcome.admission_receipt.is_some() && outcome.terminal_receipt.is_some() {
        if let Some(error) = outcome.error.clone() {
            return Err(product_status_from_axon_error(&error)
                .unwrap_or_else(|| status_from_axon_invoke_error("Invoke", ability, error)));
        }
    }
    rpc_dispatch_outcome_response(ability, "daemon route", outcome).0
}

pub(crate) fn product_status_to_axon_error(status: Status) -> AxonError {
    let code = status.code();
    let message = status.message().to_string();
    let error = match code {
        tonic::Code::Cancelled => AxonError::cancelled(message),
        tonic::Code::DeadlineExceeded => AxonError::deadline_exceeded(message),
        tonic::Code::Unavailable => AxonError::unavailable(message),
        tonic::Code::InvalidArgument => AxonError::invalid_argument(message),
        tonic::Code::ResourceExhausted => AxonError::resource_exhausted(message),
        tonic::Code::PermissionDenied | tonic::Code::Unauthenticated => {
            AxonError::permission_denied(message)
        }
        _ => AxonError::internal(message),
    };
    error.with_context(
        PRODUCT_GRPC_CODE_CONTEXT,
        tonic_code_number(code).to_string(),
    )
}

fn product_status_from_axon_error(error: &AxonError) -> Option<Status> {
    let code = error
        .context
        .get(PRODUCT_GRPC_CODE_CONTEXT)?
        .parse::<i32>()
        .ok()
        .and_then(tonic_code_from_number)?;
    let message = if error.message.is_empty() {
        error.reason.clone()
    } else {
        error.message.clone()
    };
    Some(Status::new(code, message))
}

const fn tonic_code_number(code: tonic::Code) -> i32 {
    match code {
        tonic::Code::Ok => 0,
        tonic::Code::Cancelled => 1,
        tonic::Code::Unknown => 2,
        tonic::Code::InvalidArgument => 3,
        tonic::Code::DeadlineExceeded => 4,
        tonic::Code::NotFound => 5,
        tonic::Code::AlreadyExists => 6,
        tonic::Code::PermissionDenied => 7,
        tonic::Code::ResourceExhausted => 8,
        tonic::Code::FailedPrecondition => 9,
        tonic::Code::Aborted => 10,
        tonic::Code::OutOfRange => 11,
        tonic::Code::Unimplemented => 12,
        tonic::Code::Internal => 13,
        tonic::Code::Unavailable => 14,
        tonic::Code::DataLoss => 15,
        tonic::Code::Unauthenticated => 16,
    }
}

const fn tonic_code_from_number(code: i32) -> Option<tonic::Code> {
    match code {
        0 => Some(tonic::Code::Ok),
        1 => Some(tonic::Code::Cancelled),
        2 => Some(tonic::Code::Unknown),
        3 => Some(tonic::Code::InvalidArgument),
        4 => Some(tonic::Code::DeadlineExceeded),
        5 => Some(tonic::Code::NotFound),
        6 => Some(tonic::Code::AlreadyExists),
        7 => Some(tonic::Code::PermissionDenied),
        8 => Some(tonic::Code::ResourceExhausted),
        9 => Some(tonic::Code::FailedPrecondition),
        10 => Some(tonic::Code::Aborted),
        11 => Some(tonic::Code::OutOfRange),
        12 => Some(tonic::Code::Unimplemented),
        13 => Some(tonic::Code::Internal),
        14 => Some(tonic::Code::Unavailable),
        15 => Some(tonic::Code::DataLoss),
        16 => Some(tonic::Code::Unauthenticated),
        _ => None,
    }
}

fn route_registration_options(
    catalog: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
    owner_ura: &str,
    route: DaemonUnaryRoute,
) -> Result<AbilityOptions, AxonError> {
    let record = catalog
        .control_plane_record_for_authority_mode(owner_ura, route.name(), route.call_mode())
        .map_err(|error| {
            AxonError::invalid_argument(format!(
                "daemon route `{}` has ambiguous descriptor proof facts for owner `{owner_ura}` in {:?}: {error}",
                route.name(),
                route.call_mode()
            ))
        })?
        .ok_or_else(|| {
            AxonError::invalid_argument(format!(
                "daemon route `{}` has no live catalog descriptor proof for owner `{owner_ura}` in {:?}",
                route.name(),
                route.call_mode()
            ))
        })?;
    let descriptor = record
        .descriptor()
        .clone()
        .rebind_owner_ura(owner_ura)
        .map_err(|error| {
            AxonError::invalid_argument(format!(
                "daemon route `{}` descriptor cannot bind to owner `{owner_ura}`: {error}",
                route.name()
            ))
        })?;
    let implementation = record.implementation();
    Ok(AbilityOptions::default()
        .with_modes(AbilityCallModes::RPC)
        .with_descriptor_proof(
            descriptor.version.as_str(),
            descriptor.admission_action().as_str(),
            descriptor.descriptor_hash_bytes(),
            descriptor.schema_hash_bytes(),
            implementation.impl_hash(),
        ))
}
