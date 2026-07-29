// EasyNet CLI - daemon exact route runtime adapter
// =================================================
//
// File: src/daemon/invocation/dispatch/daemon_route_runtime.rs
// Description: Registers daemon-owned exact routes in the shared Axon
//              LocalRuntime and dispatches admitted gRPC requests through
//              the descriptor-bound runtime API.
//
// Protocol Responsibility:
// - Preserve the caller's descriptor-bound seven-tuple at the runtime boundary.
// - Make Axon LocalRuntime the sole owner of admission and terminal receipts.
//
// Implementation Approach:
// - Register all DaemonUnaryRoute, DaemonStreamRoute, and DaemonBidiRoute
//   handlers atomically as owner-bound abilities backed by route-family
//   providers.
// - Resolve registration proof facts once, then drain only Axon's canonical
//   finalized handles when projecting transport responses.
//
// Usage Contract:
// - Boot must call register before exposing either invocation listener.
// - Product handlers return payload bytes, stream values, or AxonError; they
//   never construct receipt or terminal state.
//
// Architectural Position:
// - Daemon transport/runtime adapter. Product behavior remains in the
//   route-family providers; protocol lifecycle remains in Axon LocalRuntime.

use std::sync::Arc;

use axon_sdk::invocation::{
    make_ability, AbilityCallModes, AbilityOptions, AbilityRegistration, AxonError, AxonErrorKind,
    BidiInputFrame, CallMode, ErrorCode, ErrorStage, LocalRuntime, SecurityClass,
    StreamingInvocationHandle,
};
use axon_sdk::pb::axon::v1::{
    Envelope, EnvelopeOpen, InvokeBidiDown, InvokeBidiUp, InvokeRequest, InvokeResponse,
    InvokeServerStreamRequest,
};
use prost::Message as _;
use sha2::{Digest as _, Sha256};
use tokio_stream::StreamExt as _;
use tonic::{Response, Status, Streaming};

use crate::daemon::ability::dispatch::stream_env_ability_with_options;
use crate::daemon::ability::CallMode as DescriptorCallMode;
use crate::daemon::invocation::admission::hosted_agent_delegation::{
    HostedAgentDelegationIngress, HostedAgentDelegationIssuer,
};
use crate::daemon::invocation::admission::register_device_pubkey::verify_user_register_pubkey_bootstrap_claim;
use crate::daemon::invocation::dispatch::cancellation::{
    InvocationCancellationRegistry, RegisteredInvocationLifecycle,
};
use crate::daemon::invocation::dispatch::daemon_invocation_service::{
    DaemonBidiRoute, DaemonStreamRoute, DaemonUnaryRoute,
};
use crate::daemon::invocation::dispatch::descriptor_binding::RuntimeBoundAbility;
use crate::daemon::invocation::dispatch::invocation_wire::status_from_axon_invoke_error;
use crate::daemon::invocation::dispatch::unary_dispatcher::{
    rpc_dispatch_outcome_response, DaemonUnaryRouteProvider,
};
use crate::daemon::invocation::streams::stream_dispatcher::DaemonStreamRouteProvider;

const PRODUCT_GRPC_CODE_CONTEXT: &str = "easynet.daemon.product.grpc_code";
pub(crate) const SESSION_OPEN_EXT_METADATA_KEY: &str = "x-easynet-session-open-ext-bin";

/// Runtime binding for the daemon's exact unary route family.
pub(crate) struct DaemonRouteRuntimeAdapter {
    runtime: Arc<LocalRuntime>,
    cancellations: InvocationCancellationRegistry,
    admission: crate::daemon::invocation::admission::admission_facade::AdmissionFacade,
    runtime_admission: Arc<
        crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionCoordinator,
    >,
}

/// Transport-origin fact selected before canonical runtime admission.
///
/// The value selects which public Axon request constructor is valid for this
/// ingress. Bootstrap carries only an immutable proof of the envelope's own
/// key/tuple binding; it is not an admitted identity or replay capability.
pub(crate) enum DaemonRouteIngress {
    ExternalSigned,
    TrustedLocalSystem,
    Bootstrap {
        proof: BootstrapCandidateProof,
        key_provider: Arc<crate::daemon::axon_bridge::runtime_admin::BootstrapCandidateKeyProvider>,
    },
}

/// Self-contained first-key bootstrap claim derived from the presented key.
///
/// This value proves that the canonical caller, subject, route, public key,
/// and payload are one immutable claim. It is transport policy context only:
/// accepting its signature and nonce still belongs exclusively to Axon
/// LocalRuntime's bootstrap admission mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BootstrapCandidateProof {
    public_key: [u8; 32],
    principal_ura: String,
    ability: &'static str,
    args_digest: [u8; 32],
}

impl BootstrapCandidateProof {
    pub(crate) fn verify(route: DaemonUnaryRoute, request: &InvokeRequest) -> Result<Self, Status> {
        match route {
            DaemonUnaryRoute::FederationJoin => Self::verify_federation_join(request),
            DaemonUnaryRoute::IdentityRegisterPubkey => {
                Self::verify_identity_register_pubkey(request)
            }
            _ => Err(Status::permission_denied(
                "bootstrap candidate is restricted to federation.join and identity.register_pubkey",
            )),
        }
    }

    fn verify_federation_join(request: &InvokeRequest) -> Result<Self, Status> {
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
        validate_join_tuple(envelope, &join)?;
        Ok(Self {
            public_key,
            principal_ura: join.membership_ura,
            ability: DaemonUnaryRoute::FederationJoin.name(),
            args_digest: Sha256::digest(&request.arguments).into(),
        })
    }

    fn verify_identity_register_pubkey(request: &InvokeRequest) -> Result<Self, Status> {
        let envelope = request.envelope.as_ref().ok_or_else(|| {
            Status::invalid_argument("identity.register_pubkey bootstrap envelope is required")
        })?;
        let claim = verify_user_register_pubkey_bootstrap_claim(envelope, &request.arguments)?;
        Ok(Self {
            public_key: claim.public_key(),
            principal_ura: claim.principal_ura().to_string(),
            ability: DaemonUnaryRoute::IdentityRegisterPubkey.name(),
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
                "bootstrap candidate route binding mismatch",
            ));
        }
        let presented_args_digest: [u8; 32] = Sha256::digest(&request.arguments).into();
        if !constant_time_eq_32(&presented_args_digest, &self.args_digest) {
            return Err(Status::permission_denied(
                "bootstrap candidate claim changed after verification",
            ));
        }
        match route {
            DaemonUnaryRoute::FederationJoin => self.validate_federation_join_request(request),
            DaemonUnaryRoute::IdentityRegisterPubkey => {
                self.validate_identity_register_pubkey_request(request)
            }
            _ => Err(Status::permission_denied(
                "bootstrap candidate route binding mismatch",
            )),
        }
    }

    fn validate_federation_join_request(&self, request: &InvokeRequest) -> Result<(), Status> {
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
        validate_join_tuple(envelope, &join)?;
        if join.membership_ura != self.principal_ura || public_key != self.public_key {
            return Err(Status::permission_denied(
                "federation.join bootstrap claim binding mismatch",
            ));
        }
        Ok(())
    }

    fn validate_identity_register_pubkey_request(
        &self,
        request: &InvokeRequest,
    ) -> Result<(), Status> {
        let envelope = request.envelope.as_ref().ok_or_else(|| {
            Status::invalid_argument("identity.register_pubkey bootstrap envelope is required")
        })?;
        let claim = verify_user_register_pubkey_bootstrap_claim(envelope, &request.arguments)?;
        if claim.principal_ura() != self.principal_ura || claim.public_key() != self.public_key {
            return Err(Status::permission_denied(
                "identity.register_pubkey bootstrap claim binding mismatch",
            ));
        }
        Ok(())
    }
}

impl DaemonRouteRuntimeAdapter {
    pub(crate) fn new(
        runtime: Arc<LocalRuntime>,
        cancellations: InvocationCancellationRegistry,
        admission: crate::daemon::invocation::admission::admission_facade::AdmissionFacade,
        runtime_admission: Arc<
            crate::daemon::invocation::admission::admission_facade::DaemonRuntimeAdmissionCoordinator,
        >,
    ) -> Self {
        Self {
            runtime,
            cancellations,
            admission,
            runtime_admission,
        }
    }

    /// Atomically install the complete exact-route family for every local
    /// authority root. A partial owner or route surface is never observable,
    /// including when registration collides.
    pub(crate) async fn register_for_owners(
        &self,
        owner_uras: &[String],
        catalog: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
        provider: DaemonUnaryRouteProvider,
    ) -> Result<(), AxonError> {
        let mut registrations =
            Vec::with_capacity(owner_uras.len().saturating_mul(DaemonUnaryRoute::ALL.len()));
        for owner_ura in owner_uras {
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
        }
        self.runtime.register_many(registrations).await
    }

    /// Atomically install the complete exact server-stream route family. The
    /// registered ability functions return product stream values; Axon owns
    /// admission, progress framing, terminal state, and receipts.
    pub(crate) async fn register_streams(
        &self,
        owner_ura: &str,
        catalog: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
        provider: DaemonStreamRouteProvider,
    ) -> Result<(), AxonError> {
        let mut registrations = Vec::with_capacity(DaemonStreamRoute::ALL.len());
        for route in DaemonStreamRoute::ALL.iter().copied() {
            let ability_ura = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
                owner_ura,
                route.name(),
            )?;
            let route_provider = provider.clone();
            let handler =
                Arc::new(move |_envelope, arguments| route_provider.invoke(route, arguments));
            let (function, _options) = stream_env_ability_with_options(handler);
            registrations.push(
                AbilityRegistration::new(ability_ura, function).with_options(
                    stream_route_registration_options(catalog, owner_ura, route)?,
                ),
            );
        }
        self.runtime.register_many(registrations).await
    }

    /// Atomically install the complete exact bidi route family. Product
    /// providers own the long-lived session behavior; LocalRuntime owns
    /// admission, lifecycle state, cancellation, and receipt finalization.
    pub(crate) async fn register_bidis(
        &self,
        owner_ura: &str,
        catalog: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
        provider: crate::daemon::invocation::bidi::bidi_dispatcher::DaemonBidiRouteProvider,
    ) -> Result<(), AxonError> {
        let mut registrations = Vec::with_capacity(DaemonBidiRoute::ALL.len());
        for route in DaemonBidiRoute::ALL.iter().copied() {
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
                    .with_options(bidi_route_registration_options(catalog, owner_ura, route)?),
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
            DaemonRouteIngress::Bootstrap {
                proof,
                key_provider,
            } => {
                proof.validate_request(route, request)?;
                let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                    &request.metadata,
                    &envelope,
                    HostedAgentDelegationIngress::BootstrapCandidate,
                    route.name(),
                )?;
                let signed_ref = bound
                    .signed_descriptor_ref_from_target(
                        "daemon exact unary route",
                        callee_ura,
                        CallMode::Rpc,
                        request.target.as_ref(),
                    )?
                    .into_descriptor_ref();
                crate::daemon::axon_bridge::descriptor_bound_dispatch::bootstrap_candidate_from_wire_parts(
                    envelope,
                    signed_ref,
                    request.arguments.clone(),
                    metadata,
                    proof.public_key,
                    &key_provider,
                )
            }
            DaemonRouteIngress::TrustedLocalSystem => {
                let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                    &request.metadata,
                    &envelope,
                    HostedAgentDelegationIngress::TrustedLocalSystem,
                    route.name(),
                )?;
                crate::daemon::axon_bridge::descriptor_bound_dispatch::local_system_from_wire_parts(
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
                    HostedAgentDelegationIngress::ExternalSigned,
                    route.name(),
                )?;
                let signed_ref = bound
                    .signed_descriptor_ref_from_target(
                        "daemon exact unary route",
                        callee_ura,
                        CallMode::Rpc,
                        request.target.as_ref(),
                    )?
                    .into_descriptor_ref();
                crate::daemon::axon_bridge::descriptor_bound_dispatch::external_signed_from_wire_parts(
                    envelope,
                    signed_ref,
                    request.arguments.clone(),
                    metadata,
                )
            }
        }
        .map_err(|error| status_from_axon_invoke_error("Invoke", route.name(), *error))?;
        let runtime_admission =
            self.runtime_admission
                .stage(&self.admission, &wire, route.name(), CallMode::Rpc)?;

        let outcome = crate::daemon::axon_bridge::descriptor_bound_dispatch::dispatch_rpc_admitted(
            &self.runtime,
            wire,
            &self.cancellations,
        )
        .await;
        if outcome.invocation_id.is_some() {
            runtime_admission.commit()?;
        }
        daemon_route_outcome_response(outcome)
    }

    /// Open one exact server-stream route through the descriptor-bound runtime
    /// path. The caller owns transport chunk projection after Axon returns the
    /// streaming handle.
    pub(crate) async fn open_stream(
        &self,
        route: DaemonStreamRoute,
        request: &InvokeServerStreamRequest,
        local_system_ingress: bool,
    ) -> Result<(StreamingInvocationHandle, RegisteredInvocationLifecycle), Status> {
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
            "daemon exact stream route",
            self.runtime.as_ref(),
            callee_ura,
            route.name(),
        )
        .await?;
        let registered_ref = bound
            .descriptor_ref_for_mode(
                "daemon exact stream route",
                callee_ura,
                CallMode::Stream,
                None,
            )?
            .into_descriptor_ref();

        let wire = if local_system_ingress {
            let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                &request.metadata,
                &envelope,
                HostedAgentDelegationIngress::TrustedLocalSystem,
                route.name(),
            )?;
            crate::daemon::axon_bridge::descriptor_bound_dispatch::local_system_from_wire_parts(
                envelope,
                registered_ref,
                request.arguments.clone(),
                metadata,
            )
        } else {
            let metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
                &request.metadata,
                &envelope,
                HostedAgentDelegationIngress::ExternalSigned,
                route.name(),
            )?;
            let signed_ref = bound
                .signed_descriptor_ref_from_target(
                    "daemon exact stream route",
                    callee_ura,
                    CallMode::Stream,
                    request.target.as_ref(),
                )?
                .into_descriptor_ref();
            crate::daemon::axon_bridge::descriptor_bound_dispatch::external_signed_from_wire_parts(
                envelope,
                signed_ref,
                request.arguments.clone(),
                metadata,
            )
        }
        .map_err(|error| status_from_axon_invoke_error("InvokeStream", route.name(), *error))?;
        let lifecycle_envelope = wire.envelope.clone();
        let runtime_admission =
            self.runtime_admission
                .stage(&self.admission, &wire, route.name(), CallMode::Stream)?;

        let handle = crate::daemon::axon_bridge::descriptor_bound_dispatch::open_stream_admitted(
            &self.runtime,
            wire,
        )
        .await
        .map_err(|err| status_from_axon_invoke_error("InvokeStream", route.name(), err))?;
        let lifecycle = match RegisteredInvocationLifecycle::register(
            self.cancellations.clone(),
            &lifecycle_envelope,
            handle.handle().clone(),
        ) {
            Ok(lifecycle) => lifecycle,
            Err(error) => {
                let _ = handle.cancel("stream lifecycle registration failed").await;
                let _ = handle.finalized().await;
                return Err(Status::failed_precondition(format!(
                    "InvokeStream `{}` lifecycle registration failed: {error}",
                    route.name()
                )));
            }
        };
        if let Err(error) = runtime_admission.commit() {
            let _ = lifecycle
                .cancel_and_finalize("stream runtime admission commit failed")
                .await;
            return Err(error);
        }
        Ok((handle, lifecycle))
    }

    /// Open one exact bidi route through the descriptor-bound LocalRuntime
    /// path, then bridge protobuf transport frames to the runtime handle.
    ///
    /// Session policy and PresenceRegistry mutation live exclusively in the
    /// registered provider. This method performs no product lifecycle work.
    pub(crate) async fn open_bidi(
        &self,
        route: DaemonBidiRoute,
        envelope_open: &EnvelopeOpen,
        mut up: Streaming<InvokeBidiUp>,
    ) -> Result<
        Response<
            crate::daemon::invocation::dispatch::invocation_wire::BoxedDownStream<InvokeBidiDown>,
        >,
        Status,
    > {
        let envelope = envelope_open.envelope.clone().ok_or_else(|| {
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
            "daemon exact bidi route",
            self.runtime.as_ref(),
            callee_ura,
            route.name(),
        )
        .await?;
        let registered_ref = bound
            .descriptor_ref_for_mode("daemon exact bidi route", callee_ura, CallMode::Bidi, None)?
            .into_descriptor_ref();
        let local_system_ingress = self
            .admission
            .accepts_local_system_envelope(envelope_open.envelope.as_ref());
        let mut metadata = HostedAgentDelegationIssuer::materialize_request_metadata(
            &envelope_open.metadata,
            &envelope,
            if local_system_ingress {
                HostedAgentDelegationIngress::TrustedLocalSystem
            } else {
                HostedAgentDelegationIngress::ExternalSigned
            },
            route.name(),
        )?;
        if let Some(extension) = envelope_open.session_ext.as_ref() {
            metadata.insert(
                SESSION_OPEN_EXT_METADATA_KEY.to_string(),
                hex::encode(extension.encode_to_vec()),
            );
        }
        let wire = if local_system_ingress {
            crate::daemon::axon_bridge::descriptor_bound_dispatch::local_system_from_wire_parts(
                envelope,
                registered_ref,
                envelope_open.initial_args.clone(),
                metadata,
            )
        } else {
            let signed_ref = bound
                .signed_descriptor_ref_from_target(
                    "daemon exact bidi route",
                    callee_ura,
                    CallMode::Bidi,
                    envelope_open.target.as_ref(),
                )?
                .into_descriptor_ref();
            crate::daemon::axon_bridge::descriptor_bound_dispatch::external_signed_from_wire_parts(
                envelope,
                signed_ref,
                envelope_open.initial_args.clone(),
                metadata,
            )
        }
        .map_err(|error| status_from_axon_invoke_error("InvokeBidi", route.name(), *error))?;
        let lifecycle_envelope = wire.envelope.clone();
        let runtime_admission =
            self.runtime_admission
                .stage(&self.admission, &wire, route.name(), CallMode::Bidi)?;
        let handle =
            crate::daemon::axon_bridge::descriptor_bound_dispatch::open_bidi_external_signed(
                &self.runtime,
                wire,
            )
            .await
            .map_err(|error| status_from_axon_invoke_error("InvokeBidi", route.name(), error))?;
        let lifecycle = match RegisteredInvocationLifecycle::register(
            self.cancellations.clone(),
            &lifecycle_envelope,
            handle.handle().clone(),
        ) {
            Ok(lifecycle) => lifecycle,
            Err(error) => {
                let _ = handle
                    .handle()
                    .cancel("bidi lifecycle registration failed")
                    .await;
                let _ = handle.handle().finalized().await;
                return Err(Status::failed_precondition(format!(
                    "InvokeBidi `{}` lifecycle registration failed: {error}",
                    route.name()
                )));
            }
        };
        if let Err(error) = runtime_admission.commit() {
            let _ = lifecycle
                .cancel_and_finalize("bidi runtime admission commit failed")
                .await;
            return Err(error);
        }
        let admission_receipt = match handle.admission_receipt().await {
            Ok(receipt) => receipt,
            Err(error) => {
                let _ = lifecycle.finalized().await;
                return Err(Status::failed_precondition(format!(
                    "CANONICAL_ADMISSION_REQUIRED: InvokeBidi `{}`: {error}",
                    route.name()
                )));
            }
        };
        let admission_wire = match axon_sdk::invocation::wire::receipt_to_wire(&admission_receipt) {
            Ok(receipt) => receipt,
            Err(error) => {
                let _ = lifecycle
                    .cancel_and_finalize("canonical admission projection failed")
                    .await;
                return Err(Status::failed_precondition(format!(
                    "CANONICAL_ADMISSION_PROJECTION_FAILED: {error}"
                )));
            }
        };
        let admission_frame = InvokeBidiDown {
            payload: Some(axon_sdk::pb::axon::v1::invoke_bidi_down::Payload::Receipt(
                admission_wire,
            )),
            ..InvokeBidiDown::default()
        };
        let (runtime_input, mut runtime_output) = handle.split();
        let (down_tx, down_rx) = tokio::sync::mpsc::channel::<Result<InvokeBidiDown, Status>>(16);

        let input_bridge = tokio::spawn(async move {
            while let Some(frame_result) = up.next().await {
                match frame_result {
                    Ok(frame) => {
                        if runtime_input
                            .send(
                                BidiInputFrame::new(frame.encode_to_vec()).with_content_type(
                                    crate::daemon::invocation::bidi::bidi_dispatcher::SESSION_RUNTIME_FRAME_CONTENT_TYPE,
                                ),
                            )
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(status) => {
                        let _ = runtime_input
                            .send(
                                BidiInputFrame::new(status.to_string().into_bytes())
                                    .with_content_type(
                                        crate::daemon::invocation::bidi::bidi_dispatcher::SESSION_RUNTIME_TRANSPORT_ERROR_CONTENT_TYPE,
                                    ),
                            )
                            .await;
                        break;
                    }
                }
            }
            let _ = runtime_input.close_input().await;
        });

        tokio::spawn(async move {
            let mut admission_pending = Some(admission_frame);
            let mut terminal_authority_observed = false;
            while let Some(frame_result) = runtime_output.next_frame().await {
                match frame_result {
                    Ok(frame) if frame.terminal => {
                        let projected =
                            crate::daemon::invocation::bidi::bidi_dispatcher::project_registered_finalized_bidi_receipt(
                                &lifecycle,
                            )
                            .await;
                        terminal_authority_observed = true;
                        if send_pending_bidi_admission(&down_tx, &mut admission_pending).await {
                            let _ = down_tx.send(projected).await;
                        }
                        break;
                    }
                    Ok(frame) => {
                        if frame.content_type
                            != crate::daemon::invocation::bidi::bidi_dispatcher::SESSION_RUNTIME_FRAME_CONTENT_TYPE
                        {
                            let reason = format!(
                                "session provider emitted unsupported content type `{}`",
                                frame.content_type
                            );
                            let projected = crate::daemon::invocation::bidi::bidi_dispatcher::cancel_registered_bidi(
                                &lifecycle,
                                reason.clone(),
                            )
                                .await
                                .map_err(|status| Status::internal(format!("{reason}; {status}")));
                            terminal_authority_observed = true;
                            if send_pending_bidi_admission(&down_tx, &mut admission_pending).await {
                                let _ = down_tx.send(projected).await;
                            }
                            break;
                        }
                        let decoded = match InvokeBidiDown::decode(frame.payload.as_slice()) {
                            Ok(decoded) => decoded,
                            Err(error) => {
                                let reason = format!(
                                    "session provider emitted malformed InvokeBidiDown: {error}"
                                );
                                let projected = crate::daemon::invocation::bidi::bidi_dispatcher::cancel_registered_bidi(
                                    &lifecycle,
                                    reason.clone(),
                                )
                                    .await
                                    .map_err(|status| {
                                        Status::internal(format!("{reason}; {status}"))
                                    });
                                terminal_authority_observed = true;
                                if send_pending_bidi_admission(&down_tx, &mut admission_pending)
                                    .await
                                {
                                    let _ = down_tx.send(projected).await;
                                }
                                break;
                            }
                        };
                        if !send_pending_bidi_admission(&down_tx, &mut admission_pending).await {
                            let _ = crate::daemon::invocation::bidi::bidi_dispatcher::cancel_registered_bidi(
                                &lifecycle,
                                "InvokeBidi transport response dropped",
                            )
                            .await;
                            terminal_authority_observed = true;
                            break;
                        }
                        if down_tx.send(Ok(decoded)).await.is_err() {
                            let _ = crate::daemon::invocation::bidi::bidi_dispatcher::cancel_registered_bidi(
                                &lifecycle,
                                "InvokeBidi transport response dropped",
                            )
                            .await;
                            terminal_authority_observed = true;
                            break;
                        }
                    }
                    Err(error) => {
                        let projected =
                            crate::daemon::invocation::bidi::bidi_dispatcher::project_registered_finalized_bidi_receipt(
                                &lifecycle,
                            )
                            .await
                            .map_err(|status| {
                                Status::internal(format!(
                                    "InvokeBidi exact-route runtime frame failed: {error}; {status}"
                                ))
                            });
                        terminal_authority_observed = true;
                        if send_pending_bidi_admission(&down_tx, &mut admission_pending).await {
                            let _ = down_tx.send(projected).await;
                        }
                        break;
                    }
                }
            }
            if !terminal_authority_observed {
                let projected = crate::daemon::invocation::bidi::bidi_dispatcher::project_registered_finalized_bidi_receipt(
                    &lifecycle,
                )
                .await;
                if send_pending_bidi_admission(&down_tx, &mut admission_pending).await {
                    let _ = down_tx.send(projected).await;
                }
            }
            input_bridge.abort();
        });

        let stream =
            crate::daemon::invocation::bidi::bidi_dispatcher::LocalBidiDownStream::new(down_rx);
        Ok(Response::new(Box::pin(stream)))
    }
}

async fn send_pending_bidi_admission(
    down_tx: &tokio::sync::mpsc::Sender<Result<InvokeBidiDown, Status>>,
    pending: &mut Option<InvokeBidiDown>,
) -> bool {
    match pending.take() {
        Some(admission) => down_tx.send(Ok(admission)).await.is_ok(),
        None => true,
    }
}

fn validate_join_tuple(
    envelope: &Envelope,
    join: &crate::daemon::invocation::dispatch::federation_wrappers::JoinRequest,
) -> Result<(), Status> {
    let caller = envelope
        .caller
        .as_ref()
        .map(|caller| caller.ura.trim())
        .filter(|caller| !caller.is_empty())
        .ok_or_else(|| Status::invalid_argument("federation.join bootstrap caller is required"))?;
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
    let caller_parsed = crate::core::ura::parse_ura(caller).map_err(|error| {
        Status::invalid_argument(format!(
            "federation.join bootstrap caller is invalid: {error}"
        ))
    })?;
    let subject_parsed = crate::core::ura::parse_ura(subject).map_err(|error| {
        Status::invalid_argument(format!(
            "federation.join bootstrap subject is invalid: {error}"
        ))
    })?;
    if callee.kind != crate::core::ura::URAKind::Authority
        || caller_parsed.kind != crate::core::ura::URAKind::Device
        || subject_parsed.kind != crate::core::ura::URAKind::Device
        || join.membership_ura != caller
        || join.membership_ura != subject
        || join.realm != callee.realm
        || join.realm != caller_parsed.realm
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
mod bootstrap_candidate_proof_tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    fn join_request(public_key: [u8; 32]) -> InvokeRequest {
        let public_key_hex = hex::encode(public_key);
        let membership_ura = "easynet:///r/bootstrap-test/device/node-1";
        let arguments = serde_json::to_vec(&serde_json::json!({
            "membership_ura": membership_ura,
            "realm": "bootstrap-test",
            "public_key_hex": public_key_hex,
        }))
        .expect("join arguments");
        crate::daemon::invocation::ProtoEnvelope::federation_join_bootstrap(
            crate::core::ura::hub_ura("bootstrap-test"),
            membership_ura,
            crate::daemon::invocation::InvocationDerivationPolicy::FreshRoot,
        )
        .expect("join envelope")
        .invoke_request(DaemonUnaryRoute::FederationJoin.name(), arguments)
        .expect("join request")
    }

    fn identity_register_request(public_key: [u8; 32]) -> InvokeRequest {
        let principal_ura = "easynet:///r/bootstrap-test/user/alice";
        let hub_ura = crate::core::ura::hub_ura("bootstrap-test");
        let descriptor_subject = crate::core::ura::owner_ability_ura(
            &hub_ura,
            DaemonUnaryRoute::IdentityRegisterPubkey.name(),
        )
        .expect("identity.register_pubkey descriptor subject");
        let arguments = serde_json::to_vec(&serde_json::json!({
            "principal_ura": principal_ura,
            "public_key_b64": BASE64_STANDARD.encode(public_key),
            "role": "user",
        }))
        .expect("identity.register_pubkey arguments");
        crate::daemon::invocation::ProtoEnvelope::from_target(
            principal_ura,
            hub_ura,
            descriptor_subject,
            crate::daemon::invocation::InvocationDerivationPolicy::FreshRoot,
        )
        .expect("identity.register_pubkey envelope")
        .invoke_request(DaemonUnaryRoute::IdentityRegisterPubkey.name(), arguments)
        .expect("identity.register_pubkey request")
    }

    #[test]
    fn non_ura_caller_is_rejected() {
        let mut request = join_request([0x11; 32]);
        request
            .envelope
            .as_mut()
            .expect("envelope")
            .caller
            .as_mut()
            .expect("caller")
            .ura = "not-a-ura".to_string();
        assert!(
            BootstrapCandidateProof::verify(DaemonUnaryRoute::FederationJoin, &request).is_err()
        );
    }

    #[test]
    fn membership_caller_mismatch_is_rejected() {
        let mut request = join_request([0x12; 32]);
        request
            .envelope
            .as_mut()
            .expect("envelope")
            .caller
            .as_mut()
            .expect("caller")
            .ura = "easynet:///r/bootstrap-test/device/other-node".to_string();
        assert!(
            BootstrapCandidateProof::verify(DaemonUnaryRoute::FederationJoin, &request).is_err()
        );
    }

    #[test]
    fn malformed_join_key_is_rejected() {
        let mut request = join_request([0x22; 32]);
        let arguments = serde_json::to_vec(&serde_json::json!({
            "membership_ura": "easynet:///r/bootstrap-test/device/node-1",
            "realm": "bootstrap-test",
            "public_key_hex": hex::encode([0x23; 31]),
        }))
        .expect("substituted join arguments");
        request.arguments = arguments;
        assert!(
            BootstrapCandidateProof::verify(DaemonUnaryRoute::FederationJoin, &request).is_err()
        );
    }

    #[test]
    fn federation_join_payload_substitution_after_proof_is_rejected() {
        let mut request = join_request([0x33; 32]);
        let proof = BootstrapCandidateProof::verify(DaemonUnaryRoute::FederationJoin, &request)
            .expect("valid proof");
        request.arguments.push(b' ');
        assert!(proof
            .validate_request(DaemonUnaryRoute::FederationJoin, &request)
            .is_err());
    }

    #[test]
    fn bootstrap_candidate_on_non_join_route_is_rejected() {
        let request = join_request([0x44; 32]);
        assert!(
            BootstrapCandidateProof::verify(DaemonUnaryRoute::FederationStatus, &request).is_err()
        );
    }

    #[test]
    fn identity_register_user_self_key_bootstrap_is_accepted() {
        let request = identity_register_request([0x55; 32]);
        let proof =
            BootstrapCandidateProof::verify(DaemonUnaryRoute::IdentityRegisterPubkey, &request)
                .expect("valid identity.register_pubkey bootstrap proof");
        proof
            .validate_request(DaemonUnaryRoute::IdentityRegisterPubkey, &request)
            .expect("bootstrap request remains bound");
    }

    #[test]
    fn identity_register_device_caller_is_rejected() {
        let mut request = identity_register_request([0x66; 32]);
        request
            .envelope
            .as_mut()
            .expect("envelope")
            .caller
            .as_mut()
            .expect("caller")
            .ura = "easynet:///r/bootstrap-test/device/not-user".to_string();
        assert!(BootstrapCandidateProof::verify(
            DaemonUnaryRoute::IdentityRegisterPubkey,
            &request
        )
        .is_err());
    }

    #[test]
    fn identity_register_payload_substitution_after_proof_is_rejected() {
        let mut request = identity_register_request([0x77; 32]);
        let proof =
            BootstrapCandidateProof::verify(DaemonUnaryRoute::IdentityRegisterPubkey, &request)
                .expect("valid proof");
        request.arguments = serde_json::to_vec(&serde_json::json!({
            "principal_ura": "easynet:///r/bootstrap-test/user/alice",
            "public_key_b64": BASE64_STANDARD.encode([0x78; 32]),
            "role": "user",
        }))
        .expect("substituted identity.register_pubkey arguments");
        assert!(proof
            .validate_request(DaemonUnaryRoute::IdentityRegisterPubkey, &request)
            .is_err());
    }

    #[test]
    fn identity_register_owner_binding_is_rejected() {
        let mut request = identity_register_request([0x88; 32]);
        request.arguments = serde_json::to_vec(&serde_json::json!({
            "principal_ura": "easynet:///r/bootstrap-test/user/alice",
            "public_key_b64": BASE64_STANDARD.encode([0x88; 32]),
            "role": "user",
            "principal_owner_ura": "easynet:///r/bootstrap-test/user/bob",
        }))
        .expect("owner-bound identity.register_pubkey arguments");
        assert!(BootstrapCandidateProof::verify(
            DaemonUnaryRoute::IdentityRegisterPubkey,
            &request
        )
        .is_err());
    }
}

fn daemon_route_outcome_response(
    outcome: crate::daemon::axon_bridge::descriptor_bound_dispatch::RpcDispatchOutcome,
) -> Result<Response<InvokeResponse>, Status> {
    rpc_dispatch_outcome_response(outcome).0
}

pub(crate) fn runtime_status_to_axon_error(status: Status) -> AxonError {
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
        tonic::Code::NotFound => AxonError::new(AxonErrorKind::InvalidArgument)
            .with_code(ErrorCode::NotFound)
            .with_message(message)
            .with_security_class(SecurityClass::Resource),
        _ => AxonError::internal(message),
    };
    error.with_stage(ErrorStage::Execution).with_context(
        PRODUCT_GRPC_CODE_CONTEXT,
        tonic_code_number(code).to_string(),
    )
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

#[cfg(test)]
mod product_status_projection_tests {
    use super::*;

    #[test]
    fn not_found_preserves_the_canonical_resource_condition() {
        let error = runtime_status_to_axon_error(Status::not_found("principal is not registered"));

        assert_eq!(error.kind, AxonErrorKind::InvalidArgument);
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(error.stage, Some(ErrorStage::Execution));
        assert_eq!(error.security_class, Some(SecurityClass::Resource));
        assert_eq!(error.message, "principal is not registered");
        assert_eq!(
            error.context.get(PRODUCT_GRPC_CODE_CONTEXT),
            Some(&tonic_code_number(tonic::Code::NotFound).to_string())
        );

        let wire = axon_sdk::invocation::wire::error_to_wire(&error);
        assert_eq!(wire.code, "NOT_FOUND");
        assert_eq!(
            wire.stage,
            axon_sdk::pb::axon::v1::ErrorStage::Execution as i32
        );
        assert_eq!(
            wire.security_class,
            axon_sdk::pb::axon::v1::SecurityClass::Resource as i32
        );
    }
}

fn route_registration_options(
    catalog: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
    owner_ura: &str,
    route: DaemonUnaryRoute,
) -> Result<AbilityOptions, AxonError> {
    daemon_route_registration_options(
        catalog,
        owner_ura,
        route.name(),
        route.call_mode(),
        AbilityOptions::default().with_modes(AbilityCallModes::RPC),
    )
}

fn stream_route_registration_options(
    catalog: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
    owner_ura: &str,
    route: DaemonStreamRoute,
) -> Result<AbilityOptions, AxonError> {
    daemon_route_registration_options(
        catalog,
        owner_ura,
        route.name(),
        route.call_mode(),
        AbilityOptions::default().with_modes(AbilityCallModes::STREAM),
    )
}

fn bidi_route_registration_options(
    catalog: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
    owner_ura: &str,
    route: DaemonBidiRoute,
) -> Result<AbilityOptions, AxonError> {
    daemon_route_registration_options(
        catalog,
        owner_ura,
        route.name(),
        route.call_mode(),
        AbilityOptions::bidi(),
    )
}

fn daemon_route_registration_options(
    catalog: &crate::daemon::ability::dispatch::AxonAbilityCatalog,
    owner_ura: &str,
    route_name: &str,
    call_mode: DescriptorCallMode,
    options: AbilityOptions,
) -> Result<AbilityOptions, AxonError> {
    let record = catalog
        .control_plane_record_for_authority_mode(owner_ura, route_name, call_mode)
        .map_err(|error| {
            AxonError::invalid_argument(format!(
                "daemon route `{route_name}` has ambiguous descriptor proof facts for owner `{owner_ura}` in {call_mode:?}: {error}",
            ))
        })?
        .ok_or_else(|| {
            AxonError::invalid_argument(format!(
                "daemon route `{route_name}` has no live catalog descriptor proof for owner `{owner_ura}` in {call_mode:?}",
            ))
        })?;
    let descriptor = record
        .descriptor()
        .clone()
        .rebind_owner_ura(owner_ura)
        .map_err(|error| {
            AxonError::invalid_argument(format!(
                "daemon route `{route_name}` descriptor cannot bind to owner `{owner_ura}`: {error}",
            ))
        })?;
    let implementation = record.implementation();
    Ok(options.with_descriptor_proof(
        descriptor.version.as_str(),
        descriptor.admission_action().as_str(),
        descriptor.descriptor_hash_bytes(),
        descriptor.schema_hash_bytes(),
        implementation.impl_hash(),
    ))
}
