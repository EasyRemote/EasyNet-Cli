use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::support::platform::local_daemon_grpc;

use super::DaemonInvocation;
use super::{
    DaemonInvocationBuilder, InvocationDraft, InvocationTuple, PrepareOptions, PreparedInvocation,
    SignedInvocation,
};
use crate::daemon::boot::DaemonEndpoints;
use crate::daemon::identity::self_identity::CanonicalSigner;
use crate::daemon::{DaemonError, Result};

#[cfg(feature = "axon-pb")]
const DEFAULT_INVOKE_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(feature = "axon-pb")]
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Connect to the local daemon's Axon Invocation endpoint.
///
/// What this type is: a Rust SDK client for daemon-hosted Axon
/// Invocation gRPC. It validates endpoint reachability at construction
/// and preserves tonic status details through `DaemonError`.
///
/// What this type is not: it is not a daemon process lifecycle handle.
/// Starting and stopping the product daemon belongs to
/// `DaemonStartConfig`/`DaemonHandle` in `daemon::boot::process`.
#[derive(Debug, Clone)]
pub struct DaemonClient {
    endpoint: PathBuf,
    #[cfg(feature = "axon-pb")]
    timeout: Duration,
    #[cfg(feature = "axon-pb")]
    connect_timeout: Duration,
}

impl DaemonClient {
    /// Build a client for an explicit Invocation endpoint.
    pub fn connect(endpoint: impl Into<PathBuf>) -> Result<Self> {
        let endpoint = endpoint.into();
        if !local_daemon_grpc::probe_accepting(&endpoint) {
            return Err(DaemonError::InvocationEndpointDown { endpoint });
        }
        Ok(Self {
            endpoint,
            #[cfg(feature = "axon-pb")]
            timeout: DEFAULT_INVOKE_TIMEOUT,
            #[cfg(feature = "axon-pb")]
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        })
    }

    /// Build a client for the currently configured local daemon.
    pub fn local() -> Result<Self> {
        Self::connect(DaemonEndpoints::try_current()?.invocation)
    }

    /// Endpoint this client dials.
    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }

    /// Submit a signed daemon Invocation through Axon's gRPC
    /// `Invocation.Invoke` method.
    #[cfg(feature = "axon-pb")]
    pub async fn invoke(
        &self,
        signed: SignedInvocation,
    ) -> Result<axon_sdk::pb::axon::v1::InvokeResponse> {
        let ability = signed.prepared().descriptor_ref().to_string();
        let invocation = signed.into_daemon_invocation();
        let request = invocation.into_request()?;
        let channel = local_daemon_grpc::connect_channel(
            self.endpoint.clone(),
            self.timeout,
            self.connect_timeout,
        )
        .await
        .map_err(|source| DaemonError::Connect {
            endpoint: self.endpoint.clone(),
            source,
        })?;
        let mut client = crate::daemon::invocation::transport::invocation_client(channel);
        client
            .invoke(request)
            .await
            .map(tonic::Response::into_inner)
            .map_err(|status| DaemonError::InvokeStatus {
                ability,
                code: status.code(),
                message: status.message().to_string(),
            })
    }

    /// Open a signed daemon Invocation through Axon's gRPC
    /// `Invocation.InvokeStream` method.
    #[cfg(feature = "axon-pb")]
    pub async fn invoke_stream(
        &self,
        signed: SignedInvocation,
    ) -> Result<tonic::Streaming<axon_sdk::pb::axon::v1::InvokeStreamChunk>> {
        let ability = signed.prepared().descriptor_ref().to_string();
        let invocation = signed.into_daemon_invocation();
        let request = invocation.into_server_stream_request()?;
        let channel = local_daemon_grpc::connect_channel(
            self.endpoint.clone(),
            self.timeout,
            self.connect_timeout,
        )
        .await
        .map_err(|source| DaemonError::Connect {
            endpoint: self.endpoint.clone(),
            source,
        })?;
        let mut client = crate::daemon::invocation::transport::invocation_client(channel);
        client
            .invoke_stream(request)
            .await
            .map(tonic::Response::into_inner)
            .map_err(|status| DaemonError::InvokeStreamStatus {
                ability,
                code: status.code(),
                message: status.message().to_string(),
            })
    }

    /// Open a signed daemon Invocation through Axon's gRPC
    /// `Invocation.InvokeBidi` method.
    ///
    /// Frame 0 is an `EnvelopeOpen` generated from the same signed tuple used
    /// by unary and server-stream calls.
    #[cfg(feature = "axon-pb")]
    pub async fn invoke_bidi(
        &self,
        signed: SignedInvocation,
        streams: Vec<axon_sdk::pb::axon::v1::StreamDescriptor>,
    ) -> Result<DaemonBidiSession> {
        let ability = signed.prepared().descriptor_ref().to_string();
        let invocation = signed.into_daemon_invocation();
        let frame0 = invocation.into_bidi_open_frame(streams)?;
        let channel = local_daemon_grpc::connect_channel(
            self.endpoint.clone(),
            self.timeout,
            self.connect_timeout,
        )
        .await
        .map_err(|source| DaemonError::Connect {
            endpoint: self.endpoint.clone(),
            source,
        })?;
        let mut client = crate::daemon::invocation::transport::invocation_client(channel);
        let (up_tx, up_rx) = tokio::sync::mpsc::channel(64);
        up_tx
            .send(frame0)
            .await
            .map_err(|_| DaemonError::InvokeBidiClosed {
                ability: ability.clone(),
            })?;
        let down = client
            .invoke_bidi(tonic::Request::new(
                tokio_stream::wrappers::ReceiverStream::new(up_rx),
            ))
            .await
            .map(tonic::Response::into_inner)
            .map_err(|status| DaemonError::InvokeBidiStatus {
                ability: ability.clone(),
                code: status.code(),
                message: status.message().to_string(),
            })?;
        Ok(DaemonBidiSession {
            ability,
            up_tx,
            down,
        })
    }
}

/// Runtime connection state exposed by the daemon SDK.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum ClientConnectionState {
    Idle,
    Resolving,
    Connecting,
    Ready,
    Degraded,
    Reconnecting,
    Failed,
    Closed,
}

impl ClientConnectionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Resolving => "Resolving",
            Self::Connecting => "Connecting",
            Self::Ready => "Ready",
            Self::Degraded => "Degraded",
            Self::Reconnecting => "Reconnecting",
            Self::Failed => "Failed",
            Self::Closed => "Closed",
        }
    }
}

/// SDK runtime client over the daemon Invocation endpoint.
///
/// What this type is: the public OOP Runtime Core client that owns
/// connection readiness, prepare/sign/submit dispatch, health, and
/// typed result projection.
///
/// What this type is not: it is not a daemon lifecycle handle and
/// does not expose gRPC, UDS, or Axon protobuf types.
#[derive(Clone)]
pub struct RuntimeClient {
    inner: DaemonClient,
    state: ClientConnectionState,
    cancellation_authority: Option<InvocationCancellationAuthority>,
}

impl std::fmt::Debug for RuntimeClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeClient")
            .field("inner", &self.inner)
            .field("state", &self.state)
            .field(
                "cancellation_authority_owner",
                &self
                    .cancellation_authority
                    .as_ref()
                    .map(InvocationCancellationAuthority::owner_ura),
            )
            .finish()
    }
}

/// Owner-bound authority for signing canonical lifecycle-control commands.
///
/// The capability contains no private key material. Production signers delegate
/// to the daemon KeyService; explicit caller signers may provide the same
/// narrow [`CanonicalSigner`] port. Keeping this dependency separate from the
/// transport prevents `RuntimeClient` from selecting or minting authority.
#[derive(Clone)]
pub struct InvocationCancellationAuthority {
    signer: Arc<dyn CanonicalSigner>,
}

impl InvocationCancellationAuthority {
    pub fn new(signer: Arc<dyn CanonicalSigner>) -> Self {
        Self { signer }
    }

    pub fn owner_ura(&self) -> &str {
        self.signer.owner_ura()
    }

    async fn sign(&self, prepared: PreparedInvocation) -> Result<SignedInvocation> {
        prepared
            .sign_with_canonical_signer(self.signer.as_ref())
            .await
    }
}

impl std::fmt::Debug for InvocationCancellationAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InvocationCancellationAuthority")
            .field("owner_ura", &self.owner_ura())
            .finish_non_exhaustive()
    }
}

impl RuntimeClient {
    /// Connect to an explicit daemon Invocation endpoint.
    pub fn connect(endpoint: impl Into<PathBuf>) -> Result<Self> {
        let inner = DaemonClient::connect(endpoint)?;
        Ok(Self {
            inner,
            state: ClientConnectionState::Ready,
            cancellation_authority: None,
        })
    }

    /// Connect to the current local daemon Invocation endpoint.
    pub fn local() -> Result<Self> {
        Self::connect(DaemonEndpoints::try_current()?.invocation)
    }

    /// Current SDK-observed connection state.
    pub fn state(&self) -> ClientConnectionState {
        self.state
    }

    /// Endpoint this runtime client dials.
    pub fn endpoint(&self) -> &Path {
        self.inner.endpoint()
    }

    /// Bind the explicit owner authority used for lifecycle-control commands.
    ///
    /// Connection creation deliberately does not infer an identity. Callers
    /// that need cancellation must provide either their signer capability or a
    /// daemon-KeyService-backed capability bound at their ingress boundary.
    pub fn with_cancellation_authority(
        mut self,
        authority: InvocationCancellationAuthority,
    ) -> Self {
        self.cancellation_authority = Some(authority);
        self
    }

    /// Start an SDK invocation builder.
    pub fn new_invocation(
        &self,
        caller_ura: impl Into<String>,
        callee_ura: impl Into<String>,
        descriptor_ref: impl Into<String>,
        subject_ura: impl Into<String>,
        derivation_policy: axon_sdk::invocation::InvocationDerivationPolicy,
    ) -> Result<DaemonInvocationBuilder> {
        DaemonInvocation::builder(
            caller_ura,
            callee_ura,
            descriptor_ref,
            subject_ura,
            derivation_policy,
        )
    }

    /// Prepare canonical signing material for an immutable draft.
    pub fn prepare(
        &self,
        draft: &InvocationDraft,
        options: PrepareOptions,
    ) -> Result<PreparedInvocation> {
        if self.state != ClientConnectionState::Ready {
            return Err(DaemonError::InvocationEndpointDown {
                endpoint: self.inner.endpoint().to_path_buf(),
            });
        }
        draft.prepare(options)
    }

    /// Submit a signed Invocation and return an observable handle.
    pub async fn submit_signed(&self, signed: SignedInvocation) -> Result<InvocationHandle> {
        if self.state != ClientConnectionState::Ready {
            return Err(DaemonError::InvocationEndpointDown {
                endpoint: self.inner.endpoint().to_path_buf(),
            });
        }
        let tuple = signed.prepared().tuple();
        let response = self.inner.invoke(signed).await?;
        let resolver = local_daemon_grpc::CanonicalRuntimeReceiptResolver::for_daemon_endpoint(
            self.inner.endpoint().to_path_buf(),
        );
        let outcome = InvocationOutcome::from_invoke_response(tuple, response, &resolver)?;
        Ok(InvocationHandle { outcome })
    }

    /// Submit an independently signed `invocation.cancel` command for a target
    /// Invocation. The returned handle proves only the command lifecycle; the
    /// target's terminal outcome remains observable through its original
    /// submit handle.
    pub async fn request_cancel_signed(
        &self,
        signed: SignedInvocation,
        reason: String,
    ) -> Result<InvocationHandle> {
        if self.state != ClientConnectionState::Ready {
            return Err(DaemonError::InvocationEndpointDown {
                endpoint: self.inner.endpoint().to_path_buf(),
            });
        }
        let authority = self.cancellation_authority.as_ref().ok_or_else(|| {
            DaemonError::InvalidInvocation(
                "invocation.cancel requires an explicit caller signer or daemon KeyService authority"
                    .to_string(),
            )
        })?;
        let prepared = signed.prepare_cancel_command(reason)?;
        let signed_cancel = authority.sign(prepared).await?;
        let tuple = signed_cancel.prepared().tuple();
        let response = self.inner.invoke(signed_cancel).await?;
        let resolver = local_daemon_grpc::CanonicalRuntimeReceiptResolver::for_daemon_endpoint(
            self.inner.endpoint().to_path_buf(),
        );
        let outcome = InvocationOutcome::from_invoke_response(tuple, response, &resolver)?;
        Ok(InvocationHandle { outcome })
    }

    /// Return typed runtime readiness without using JSON control
    /// product dispatch.
    pub fn health(&self) -> RuntimeHealth {
        let invocation_ready = local_daemon_grpc::probe_accepting(self.inner.endpoint());
        RuntimeHealth {
            sdk_version: env!("CARGO_PKG_VERSION").to_string(),
            endpoint: self.inner.endpoint().display().to_string(),
            connection_state: if invocation_ready {
                self.state
            } else {
                ClientConnectionState::Degraded
            },
            invocation_ready,
            runtime_ready: invocation_ready && self.state == ClientConnectionState::Ready,
            last_error: None,
        }
    }

    /// Close the client. Closing a runtime client never stops the daemon.
    pub fn close(&mut self) {
        self.state = ClientConnectionState::Closed;
    }
}

/// Typed runtime health projection.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RuntimeHealth {
    pub sdk_version: String,
    pub endpoint: String,
    pub connection_state: ClientConnectionState,
    pub invocation_ready: bool,
    pub runtime_ready: bool,
    pub last_error: Option<RuntimeErrorSummary>,
}

/// Submitted Invocation observer. Unary submit currently resolves to a
/// terminal result immediately, but the handle preserves the SDK object
/// boundary required by stream/bidi and future async observation.
#[derive(Debug, Clone)]
pub struct InvocationHandle {
    outcome: InvocationOutcome,
}

impl InvocationHandle {
    /// Consume the handle and return its established terminal-result
    /// projection.
    ///
    /// This method intentionally preserves the original SDK surface. Call
    /// [`Self::await_outcome`] when both receipt checkpoints are required.
    pub fn await_result(self) -> InvocationResult {
        self.outcome.into_result()
    }

    /// Read the established terminal-result projection.
    pub fn result(&self) -> &InvocationResult {
        self.outcome.result()
    }

    /// Read the admission and terminal receipt checkpoints associated with
    /// this invocation.
    pub fn stages(&self) -> &InvocationReceiptStages {
        self.outcome.stages()
    }

    /// Read the complete immutable invocation outcome.
    pub fn outcome(&self) -> &InvocationOutcome {
        &self.outcome
    }

    /// Consume the handle and return the complete immutable invocation
    /// outcome.
    pub fn await_outcome(self) -> InvocationOutcome {
        self.outcome
    }
}

/// Terminal unary Invocation projection.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InvocationResult {
    pub tuple: InvocationTuple,
    pub terminal_state: String,
    pub output_content_type: String,
    pub output: Vec<u8>,
    pub elapsed_ms: u64,
    /// Terminal execution receipt.
    pub receipt: Option<ReceiptSummary>,
    pub error: Option<RuntimeErrorSummary>,
}

/// Receipt checkpoints produced while an Invocation advances from admission
/// to its deterministic terminal state.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct InvocationReceiptStages {
    pub admission: Option<ReceiptSummary>,
    pub terminal: Option<ReceiptSummary>,
}

impl InvocationReceiptStages {
    /// Admission checkpoint, when the runtime emitted one.
    pub fn admission(&self) -> Option<&ReceiptSummary> {
        self.admission.as_ref()
    }

    /// Terminal execution checkpoint, when the runtime emitted one.
    pub fn terminal(&self) -> Option<&ReceiptSummary> {
        self.terminal.as_ref()
    }
}

/// Immutable unary Invocation outcome.
///
/// This aggregate owns the two-checkpoint receipt model.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InvocationOutcome {
    result: InvocationResult,
    stages: InvocationReceiptStages,
}

impl InvocationOutcome {
    pub(crate) fn from_invoke_response(
        tuple: InvocationTuple,
        response: axon_sdk::pb::axon::v1::InvokeResponse,
        resolver: &dyn axon_sdk::invocation::KeyResolver,
    ) -> Result<Self> {
        use axon_sdk::invocation::InvocationState;

        let response_state = InvocationState::try_from(response.state).map_err(|error| {
            DaemonError::InvalidInvocation(format!("InvokeResponse state is invalid: {error}"))
        })?;
        let error = response.error.as_ref().map(RuntimeErrorSummary::from_wire);
        let stages = match (
            response.admission_receipt.clone(),
            response.terminal_receipt.clone(),
        ) {
            (Some(admission_wire), Some(terminal_wire)) => {
                let checkpoints =
                    crate::daemon::invocation::receipts::finalization_projection::verify_wire_finalization_checkpoints(
                        admission_wire,
                        terminal_wire,
                        resolver,
                    )
                    .map_err(|error| DaemonError::InvalidInvocation(error.to_string()))?;
                if checkpoints.terminal().state() != response_state {
                    return Err(DaemonError::InvalidInvocation(
                        "verified terminal receipt state disagrees with InvokeResponse state"
                            .to_string(),
                    ));
                }
                InvocationReceiptStages {
                    admission: Some(ReceiptSummary::from_signed(checkpoints.admission())?),
                    terminal: Some(ReceiptSummary::from_signed(checkpoints.terminal())?),
                }
            }
            (None, None)
                if response_state == InvocationState::Failed
                    && response
                        .error
                        .as_ref()
                        .is_some_and(is_pre_admission_failure)
                    && response.proof_error.is_none() =>
            {
                InvocationReceiptStages::default()
            }
            (None, None) => {
                return Err(DaemonError::InvalidInvocation(
                    "receipt-free InvokeResponse must be a typed pre-admission Failed outcome"
                        .to_string(),
                ))
            }
            _ => {
                return Err(DaemonError::InvalidInvocation(
                    "InvokeResponse carried a partial receipt checkpoint chain".to_string(),
                ))
            }
        };
        let result = InvocationResult {
            tuple,
            terminal_state: invocation_state_name(response.state),
            output_content_type: response.result_content_type,
            output: response.result,
            elapsed_ms: response.elapsed_ms.max(0) as u64,
            receipt: stages.terminal.clone(),
            error,
        };
        Ok(Self::new(result, stages))
    }

    pub(crate) fn new(result: InvocationResult, stages: InvocationReceiptStages) -> Self {
        debug_assert_eq!(result.receipt, stages.terminal);
        Self { result, stages }
    }

    /// Read the canonical terminal-result projection.
    pub fn result(&self) -> &InvocationResult {
        &self.result
    }

    /// Read the admission and terminal receipt checkpoints.
    pub fn stages(&self) -> &InvocationReceiptStages {
        &self.stages
    }

    /// Consume the outcome and return its canonical terminal-result projection.
    pub fn into_result(self) -> InvocationResult {
        self.result
    }

    pub(crate) fn into_parts(self) -> (InvocationResult, InvocationReceiptStages) {
        (self.result, self.stages)
    }
}

fn is_pre_admission_failure(error: &axon_sdk::pb::axon::v1::Error) -> bool {
    use axon_sdk::pb::axon::v1::ErrorStage;

    matches!(
        ErrorStage::try_from(error.stage),
        Ok(ErrorStage::GlobalAdmission
            | ErrorStage::CallerAuthentication
            | ErrorStage::AuthorityValidation
            | ErrorStage::BootstrapAuthorization
            | ErrorStage::Quota
            | ErrorStage::AbilityResolution
            | ErrorStage::AbilityPolicy
            | ErrorStage::RequestValidation)
    )
}

fn invocation_state_name(state: i32) -> String {
    use axon_sdk::invocation::InvocationState;

    if state == InvocationState::Accepted.to_wire_i32() {
        "Accepted".to_string()
    } else if state == InvocationState::Admitted.to_wire_i32() {
        "Admitted".to_string()
    } else if state == InvocationState::Dispatched.to_wire_i32() {
        "Dispatched".to_string()
    } else if state == InvocationState::Running.to_wire_i32() {
        "Running".to_string()
    } else if state == InvocationState::Completed.to_wire_i32() {
        "Completed".to_string()
    } else if state == InvocationState::Failed.to_wire_i32() {
        "Failed".to_string()
    } else if state == InvocationState::TimedOut.to_wire_i32() {
        "TimedOut".to_string()
    } else if state == InvocationState::Cancelled.to_wire_i32() {
        "Cancelled".to_string()
    } else {
        state.to_string()
    }
}

/// Receipt summary DTO. This is a projection, not a full cryptographic
/// verification claim.
#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
pub struct ReceiptSummary {
    pub verification: ReceiptVerification,
    pub receipt_ura: String,
    pub index: u64,
    pub invocation_id: String,
    pub receipt_type: String,
    pub state: String,
    pub timestamp_unix_ms: i64,
    pub prev_receipt_hash_hex: String,
    pub self_hash_hex: String,
    pub payload_content_type: String,
    pub cleanup_complete: bool,
    pub reason: String,
    pub child_invocation_id: String,
    pub payload_base64: String,
    pub caller_binding: Option<ReceiptAgentBindingSummary>,
    pub callee_binding: Option<ReceiptAgentBindingSummary>,
    pub subject_binding: Option<ReceiptSubjectBindingSummary>,
    pub invocation_nonce_base64: String,
    pub causal_binding_kind: String,
    pub causal_binding: serde_json::Value,
    pub callee_signature: Option<ReceiptSignatureSummary>,
    pub signer_binding: Option<ReceiptAgentBindingSummary>,
    pub host_attestation_base64: String,
    pub authority_binding_kind: String,
    pub authority_binding: serde_json::Value,
    pub ability_binding: String,
    pub failure: Option<ReceiptFailureSummary>,
    pub usage: Option<ReceiptUsageSummary>,
    pub subject_ref: Option<ReceiptEntityRefSummary>,
    pub descriptor_version: String,
    pub schema_hash_hex: String,
    pub impl_hash_hex: String,
    pub runtime_env: String,
    pub authority_proof: Option<ReceiptAuthorityProofSummary>,
    pub input_hash_hex: String,
    pub output_hash_hex: String,
    pub parent_receipts: Vec<ReceiptRefSummary>,
}

impl ReceiptSummary {
    pub(crate) fn from_signed(
        receipt: &axon_sdk::invocation::SignedInvocationReceipt,
    ) -> Result<Self> {
        let receipt_ura = receipt_summary_ura(receipt)?;
        let wire = axon_sdk::invocation::wire::receipt_to_wire(receipt)
            .map_err(|error| DaemonError::InvalidInvocation(error.to_string()))?;
        Ok(Self::from_verified_wire(&wire, receipt_ura))
    }

    fn from_verified_wire(
        receipt: &axon_sdk::pb::axon::v1::InvocationReceipt,
        receipt_ura: String,
    ) -> Self {
        Self {
            verification: ReceiptVerification::Verified,
            receipt_ura,
            index: receipt.index,
            invocation_id: receipt.invocation_id.clone(),
            receipt_type: receipt.receipt_type.clone(),
            state: invocation_state_name(receipt.state),
            timestamp_unix_ms: receipt.timestamp_unix_ms,
            prev_receipt_hash_hex: hex::encode(&receipt.prev_receipt_hash),
            self_hash_hex: hex::encode(&receipt.self_hash),
            payload_content_type: receipt.payload_content_type.clone(),
            cleanup_complete: receipt.cleanup_complete,
            reason: receipt.reason.clone(),
            child_invocation_id: receipt.child_invocation_id.clone(),
            payload_base64: base64_bytes(&receipt.payload),
            caller_binding: receipt.caller_binding.as_ref().map(agent_binding_summary),
            callee_binding: receipt.callee_binding.as_ref().map(agent_binding_summary),
            subject_binding: receipt
                .subject_binding
                .as_ref()
                .map(subject_binding_summary),
            invocation_nonce_base64: base64_bytes(&receipt.invocation_nonce),
            causal_binding_kind: causal_binding_kind(receipt.causal_binding.as_ref()),
            causal_binding: causal_binding_summary(receipt.causal_binding.as_ref()),
            callee_signature: receipt.callee_signature.as_ref().map(signature_summary),
            signer_binding: receipt
                .signer_binding
                .as_ref()
                .or(receipt.callee_binding.as_ref())
                .map(agent_binding_summary),
            host_attestation_base64: base64_bytes(&receipt.host_attestation),
            authority_binding_kind: authority_binding_kind(receipt.authority_binding.as_ref()),
            authority_binding: authority_binding_summary(receipt.authority_binding.as_ref()),
            ability_binding: receipt.ability_binding.clone(),
            failure: receipt.failure.as_ref().map(failure_summary),
            usage: receipt.usage.as_ref().map(usage_summary),
            subject_ref: receipt.subject_ref.as_ref().map(entity_ref_summary),
            descriptor_version: receipt.descriptor_version.clone(),
            schema_hash_hex: hex::encode(&receipt.schema_hash),
            impl_hash_hex: hex::encode(&receipt.impl_hash),
            runtime_env: receipt.runtime_env.clone(),
            authority_proof: receipt
                .authority_proof
                .as_ref()
                .map(authority_proof_summary),
            input_hash_hex: hex::encode(&receipt.input_hash),
            output_hash_hex: hex::encode(&receipt.output_hash),
            parent_receipts: receipt
                .parent_receipts
                .iter()
                .map(receipt_ref_summary)
                .collect(),
        }
    }
}

fn receipt_summary_ura(receipt: &axon_sdk::invocation::SignedInvocationReceipt) -> Result<String> {
    let binding = receipt.axiom_binding();
    axon_sdk::ura::invocation_record_ura_for_binding(
        &binding.subject.ura,
        &binding.callee.ura,
        &binding.caller.ura,
        receipt.invocation_id(),
    )
    .map(|invocation_ura| {
        format!(
            "{}/receipt/{}",
            invocation_ura.trim_end_matches('/'),
            receipt.index()
        )
    })
    .ok_or_else(|| {
        DaemonError::InvalidInvocation(format!(
            "verified receipt has no canonical invocation URA anchor: invocation_id={}",
            receipt.invocation_id()
        ))
    })
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptVerification {
    Verified,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
pub struct ReceiptAgentBindingSummary {
    pub ura: String,
    pub profile: String,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
pub struct ReceiptSubjectBindingSummary {
    pub ura: String,
    pub profile: String,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
pub struct ReceiptEntityRefSummary {
    pub kind: i32,
    pub ura: String,
    pub profile: String,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
pub struct ReceiptSignatureSummary {
    pub algorithm: String,
    pub signature_base64: String,
    pub key_id_hint: String,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
pub struct ReceiptFailureSummary {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub stage: i32,
    pub security_class: i32,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
pub struct ReceiptUsageSummary {
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub duration_ms: u64,
    pub external_calls: u32,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
pub struct ReceiptRefSummary {
    pub receipt_hash_hex: String,
    pub receipt_ura: String,
}

#[derive(Debug, Clone, Eq, PartialEq, serde::Serialize)]
pub struct ReceiptAuthorityProofSummary {
    pub proof_type: String,
    pub binding_kind: String,
    pub binding: serde_json::Value,
    pub proof_payload_base64: String,
    pub proof_hash_hex: String,
    pub issuer: Option<ReceiptAgentBindingSummary>,
    pub signature: Option<ReceiptSignatureSummary>,
    pub admission_hook: String,
}

fn base64_bytes(bytes: &[u8]) -> String {
    use base64::Engine as _;

    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn agent_binding_summary(
    binding: &axon_sdk::pb::axon::v1::AgentIdentity,
) -> ReceiptAgentBindingSummary {
    ReceiptAgentBindingSummary {
        ura: binding.ura.clone(),
        profile: binding.profile.clone(),
    }
}

fn subject_binding_summary(
    binding: &axon_sdk::pb::axon::v1::SubjectIdentity,
) -> ReceiptSubjectBindingSummary {
    ReceiptSubjectBindingSummary {
        ura: binding.ura.clone(),
        profile: binding.profile.clone(),
    }
}

fn entity_ref_summary(reference: &axon_sdk::pb::axon::v1::EntityRef) -> ReceiptEntityRefSummary {
    ReceiptEntityRefSummary {
        kind: reference.kind,
        ura: reference.ura.clone(),
        profile: reference.profile.clone(),
    }
}

fn signature_summary(
    signature: &axon_sdk::pb::axon::v1::CalleeSignature,
) -> ReceiptSignatureSummary {
    ReceiptSignatureSummary {
        algorithm: signature.algorithm.clone(),
        signature_base64: base64_bytes(&signature.signature),
        key_id_hint: signature.key_id_hint.clone(),
    }
}

fn receipt_ref_summary(reference: &axon_sdk::pb::axon::v1::ReceiptRef) -> ReceiptRefSummary {
    ReceiptRefSummary {
        receipt_hash_hex: hex::encode(&reference.receipt_hash),
        receipt_ura: reference.receipt_ura.clone(),
    }
}

fn failure_summary(failure: &axon_sdk::pb::axon::v1::Error) -> ReceiptFailureSummary {
    ReceiptFailureSummary {
        code: failure.code.clone(),
        message: failure.message.clone(),
        retryable: failure.retryable,
        stage: failure.stage,
        security_class: failure.security_class,
    }
}

fn usage_summary(usage: &axon_sdk::pb::axon::v1::InvocationUsage) -> ReceiptUsageSummary {
    ReceiptUsageSummary {
        tokens_in: usage.tokens_in,
        tokens_out: usage.tokens_out,
        duration_ms: usage.duration_ms,
        external_calls: usage.external_calls,
    }
}

fn authority_proof_summary(
    proof: &axon_sdk::pb::axon::v1::InvocationAuthorityProof,
) -> ReceiptAuthorityProofSummary {
    ReceiptAuthorityProofSummary {
        proof_type: proof.proof_type.clone(),
        binding_kind: authority_binding_kind(proof.binding.as_ref()),
        binding: authority_binding_summary(proof.binding.as_ref()),
        proof_payload_base64: base64_bytes(&proof.proof_payload),
        proof_hash_hex: hex::encode(&proof.proof_hash),
        issuer: proof.issuer.as_ref().map(agent_binding_summary),
        signature: proof.signature.as_ref().map(signature_summary),
        admission_hook: proof.admission_hook.clone(),
    }
}

fn causal_binding_summary(
    causal: Option<&axon_sdk::pb::axon::v1::CausalContext>,
) -> serde_json::Value {
    use axon_sdk::pb::axon::v1::causal_context::Form;

    match causal.and_then(|context| context.form.as_ref()) {
        Some(Form::None(_)) => serde_json::json!({"form": "none"}),
        Some(Form::Scalar(receipt)) => serde_json::json!({
            "form": "scalar",
            "receipt": receipt_ref_summary(receipt),
        }),
        Some(Form::List(list)) => serde_json::json!({
            "form": "list",
            "prior": list.prior.iter().map(receipt_ref_summary).collect::<Vec<_>>(),
        }),
        Some(Form::Merkle(root)) => serde_json::json!({
            "form": "merkle",
            "root_hex": hex::encode(&root.root),
            "proof_ura": root.proof_ura,
        }),
        None => serde_json::Value::Null,
    }
}

fn causal_binding_kind(causal: Option<&axon_sdk::pb::axon::v1::CausalContext>) -> String {
    use axon_sdk::pb::axon::v1::causal_context::Form;

    match causal.and_then(|context| context.form.as_ref()) {
        Some(Form::None(_)) => "none",
        Some(Form::Scalar(_)) => "scalar",
        Some(Form::List(_)) => "list",
        Some(Form::Merkle(_)) => "merkle",
        None => "",
    }
    .to_string()
}

fn authority_binding_summary(
    binding: Option<&axon_sdk::pb::axon::v1::AuthorityBinding>,
) -> serde_json::Value {
    use axon_sdk::pb::axon::v1::authority_binding::Form;
    use axon_sdk::pb::axon::v1::authority_relation_binding::Evidence;

    match binding.and_then(|binding| binding.form.as_ref()) {
        Some(Form::Binding(binding)) => {
            let authority_ura = binding
                .authority
                .as_ref()
                .map(|identity| identity.ura.as_str())
                .unwrap_or_default();
            match &binding.evidence {
                Some(Evidence::Identity(_)) => serde_json::json!({
                    "kind": "self+identity",
                    "authority_ura": authority_ura,
                }),
                Some(Evidence::Delegation(value)) => {
                    let issuer_ura = value
                        .issuer
                        .as_ref()
                        .map(|identity| identity.ura.as_str())
                        .unwrap_or_default();
                    serde_json::json!({
                        "kind": "delegated_by+delegation",
                        "authority_ura": authority_ura,
                        "issuer_ura": issuer_ura,
                        "audience": value.audience,
                        "scopes": value.scopes,
                        "issued_at_ms": value.issued_at_ms,
                        "expires_at_ms": value.expires_at_ms,
                        "signature_base64": base64_bytes(&value.signature),
                    })
                }
                Some(Evidence::Session(value)) => {
                    let issuer_ura = value
                        .issuer
                        .as_ref()
                        .map(|identity| identity.ura.as_str())
                        .unwrap_or_default();
                    serde_json::json!({
                        "kind": "session_of+session",
                        "authority_ura": authority_ura,
                        "issuer_ura": issuer_ura,
                        "session_id": value.session_id,
                        "scopes": value.scopes,
                        "audiences": value.audiences,
                        "issued_at_ms": value.issued_at_ms,
                        "expires_at_ms": value.expires_at_ms,
                        "signature_base64": base64_bytes(&value.signature),
                    })
                }
                Some(Evidence::Attestation(_)) => serde_json::json!({
                    "kind": "credential_of+attestation",
                    "authority_ura": authority_ura,
                }),
                None => serde_json::Value::Null,
            }
        }
        Some(Form::Bootstrap(value)) => serde_json::json!({
            "kind": "bootstrap",
            "principal_ura": value.principal_ura,
            "realm": value.realm,
            "ability": value.ability,
        }),
        None => serde_json::Value::Null,
    }
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::authority_binding_summary;

    #[test]
    fn session_authority_summary_uses_public_generic_fields() {
        let binding = axon_sdk::pb::axon::v1::AuthorityBinding {
            form: Some(axon_sdk::pb::axon::v1::authority_binding::Form::Binding(
                axon_sdk::pb::axon::v1::AuthorityRelationBinding {
                    authority: Some(axon_sdk::pb::axon::v1::AgentIdentity {
                        ura: "easynet:///r/example/agent/alice".to_string(),
                        profile: "axon-strict-v2".to_string(),
                    }),
                    relation: axon_sdk::pb::axon::v1::AuthorityRelation::SessionOf as i32,
                    evidence: Some(
                        axon_sdk::pb::axon::v1::authority_relation_binding::Evidence::Session(
                            axon_sdk::pb::axon::v1::SessionEvidence {
                                issuer: Some(axon_sdk::pb::axon::v1::AgentIdentity {
                                    ura: "easynet:///r/example/agent/backend".to_string(),
                                    profile: "axon-strict-v2".to_string(),
                                }),
                                session_id: "session-1".to_string(),
                                scopes: vec!["invoke".to_string()],
                                audiences: vec!["easynet:///r/example/device/dev-a".to_string()],
                                issued_at_ms: 1,
                                expires_at_ms: 2,
                                signature: vec![0x73; 64],
                            },
                        ),
                    ),
                },
            )),
        };

        let projection = authority_binding_summary(Some(&binding));

        assert_eq!(projection["kind"], "session_of+session");
        assert_eq!(
            projection["issuer_ura"],
            "easynet:///r/example/agent/backend"
        );
        assert_eq!(
            projection["authority_ura"],
            "easynet:///r/example/agent/alice"
        );
        assert!(projection.get("backend_ura").is_none());
        assert!(projection.get("user_ura").is_none());
        assert!(projection.get("subject_ura").is_none());
    }
}

fn authority_binding_kind(binding: Option<&axon_sdk::pb::axon::v1::AuthorityBinding>) -> String {
    use axon_sdk::pb::axon::v1::authority_binding::Form;
    use axon_sdk::pb::axon::v1::authority_relation_binding::Evidence;

    match binding.and_then(|binding| binding.form.as_ref()) {
        Some(Form::Binding(binding)) => match &binding.evidence {
            Some(Evidence::Identity(_)) => "self+identity",
            Some(Evidence::Delegation(_)) => "delegated_by+delegation",
            Some(Evidence::Session(_)) => "session_of+session",
            Some(Evidence::Attestation(_)) => "credential_of+attestation",
            None => "",
        },
        Some(Form::Bootstrap(_)) => "bootstrap",
        None => "",
    }
    .to_string()
}

/// Stable SDK error summary for result DTOs and health.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RuntimeErrorSummary {
    pub code: String,
    pub stage: String,
    pub message: String,
    pub retryable: bool,
}

impl RuntimeErrorSummary {
    pub(crate) fn from_wire(error: &axon_sdk::pb::axon::v1::Error) -> Self {
        let stage = axon_sdk::pb::axon::v1::ErrorStage::try_from(error.stage)
            .map(|stage| {
                stage
                    .as_str_name()
                    .strip_prefix("ERROR_STAGE_")
                    .unwrap_or(stage.as_str_name())
                    .to_ascii_lowercase()
            })
            .unwrap_or_else(|_| error.stage.to_string());
        Self {
            code: error.code.clone(),
            stage,
            message: error.message.clone(),
            retryable: error.retryable,
        }
    }
}

/// Active SDK-owned InvokeBidi session.
///
/// Invariants:
/// 1. Frame 0 has already been sent before construction.
/// 2. Public convenience send helpers fail closed until a frame-chain-aware
///    sender can attach canonical N≥1 MACs.
/// 3. Dropping the session closes the up-direction stream. It does
///    not synthesize protocol EOF; callers that need graceful close
///    must use a frame-chain-aware sender.
#[cfg(feature = "axon-pb")]
pub struct DaemonBidiSession {
    ability: String,
    up_tx: tokio::sync::mpsc::Sender<axon_sdk::pb::axon::v1::InvokeBidiUp>,
    down: tonic::Streaming<axon_sdk::pb::axon::v1::InvokeBidiDown>,
}

#[cfg(feature = "axon-pb")]
impl DaemonBidiSession {
    /// Ability name this bidi session opened.
    pub fn ability(&self) -> &str {
        &self.ability
    }

    /// Split the session into its raw transport halves for crate
    /// internal adapters that must drive read and write tasks
    /// independently, such as the C ABI registry.
    pub(crate) fn into_parts(
        self,
    ) -> (
        String,
        tokio::sync::mpsc::Sender<axon_sdk::pb::axon::v1::InvokeBidiUp>,
        tonic::Streaming<axon_sdk::pb::axon::v1::InvokeBidiDown>,
    ) {
        (self.ability, self.up_tx, self.down)
    }

    /// Read the next down-direction frame.
    pub async fn next_down(&mut self) -> Result<Option<axon_sdk::pb::axon::v1::InvokeBidiDown>> {
        self.down
            .message()
            .await
            .map_err(|status| DaemonError::InvokeBidiStatus {
                ability: self.ability.clone(),
                code: status.code(),
                message: status.message().to_string(),
            })
    }

    /// Send a binary chunk on the up direction.
    pub async fn send_binary_chunk(
        &mut self,
        _chunk: axon_sdk::pb::axon::v1::BinaryChunk,
    ) -> Result<()> {
        Err(DaemonError::InvalidInvocation(
            "bidi binary send requires an explicit frame-chain MAC; use a frame-chain aware sender"
                .to_string(),
        ))
    }

    /// Send a control frame on the up direction.
    pub async fn send_control(
        &mut self,
        _control: axon_sdk::pb::axon::v1::BidiControl,
    ) -> Result<()> {
        Err(DaemonError::InvalidInvocation(
            "bidi control send requires an explicit frame-chain MAC; use a frame-chain aware sender"
                .to_string(),
        ))
    }

    /// Send a graceful EOF control frame.
    pub async fn send_eof(&mut self) -> Result<()> {
        Err(DaemonError::InvalidInvocation(
            "bidi EOF send requires an explicit frame-chain MAC; use a frame-chain aware sender"
                .to_string(),
        ))
    }
}

#[cfg(test)]
mod invocation_outcome_tests {
    use std::collections::HashMap;

    use super::*;
    use axon_sdk::invocation::InvocationState;
    use axon_sdk::pb::axon::v1::{Error, ErrorStage, InvocationReceipt, InvokeResponse};

    fn tuple() -> InvocationTuple {
        InvocationTuple {
            caller_ura: "easynet:///r/acme/device/dev-a".to_string(),
            callee_ura: "easynet:///r/acme/device/dev-a".to_string(),
            descriptor_ref: "easynet:///r/acme/ability/observe.health@1.0.0#descriptor!invoke"
                .to_string(),
            subject_ura: "easynet:///r/acme/device/dev-a".to_string(),
            nonce_base64: "AAAAAAAAAAAAAAAAAAAAAA==".to_string(),
            causal_context: serde_json::json!({"form": "none"}),
            args_digest_hex: "00".repeat(32),
            content_type: "application/json".to_string(),
            metadata: HashMap::new(),
            timeout_seconds: Some(30),
        }
    }

    fn resolver() -> local_daemon_grpc::LocalKeyServiceReceiptResolver {
        local_daemon_grpc::LocalKeyServiceReceiptResolver::new()
    }

    fn admission_error() -> Error {
        Error {
            code: "CALLER_SIGNATURE_INVALID".to_string(),
            message: "ed25519_signature_wrong_length".to_string(),
            stage: ErrorStage::CallerAuthentication as i32,
            ..Error::default()
        }
    }

    #[test]
    fn receipt_free_admission_rejection_is_a_typed_terminal_outcome() {
        let outcome = InvocationOutcome::from_invoke_response(
            tuple(),
            InvokeResponse {
                state: InvocationState::Failed.to_wire_i32(),
                error: Some(admission_error()),
                ..InvokeResponse::default()
            },
            &resolver(),
        )
        .expect("receipt-free pre-admission failure");

        assert_eq!(outcome.result().terminal_state, "Failed");
        assert!(outcome.result().receipt.is_none());
        assert!(outcome.stages().admission().is_none());
        assert!(outcome.stages().terminal().is_none());
        let error = outcome.result().error.as_ref().expect("typed error");
        assert_eq!(error.code, "CALLER_SIGNATURE_INVALID");
        assert_eq!(error.stage, "caller_authentication");
    }

    #[test]
    fn receipt_free_non_failed_response_is_rejected() {
        let error = InvocationOutcome::from_invoke_response(
            tuple(),
            InvokeResponse {
                state: InvocationState::Completed.to_wire_i32(),
                ..InvokeResponse::default()
            },
            &resolver(),
        )
        .expect_err("Completed must carry verified finalization receipts");

        assert!(error
            .to_string()
            .contains("receipt-free InvokeResponse must be a typed pre-admission Failed outcome"));
    }

    #[test]
    fn receipt_free_transport_or_execution_failure_is_rejected() {
        for stage in [
            ErrorStage::Unspecified,
            ErrorStage::Transport,
            ErrorStage::Execution,
        ] {
            let error = InvocationOutcome::from_invoke_response(
                tuple(),
                InvokeResponse {
                    state: InvocationState::Failed.to_wire_i32(),
                    error: Some(Error {
                        code: "NON_ADMISSION_FAILURE".to_string(),
                        message: "runtime did not produce a finalization chain".to_string(),
                        stage: stage as i32,
                        ..Error::default()
                    }),
                    ..InvokeResponse::default()
                },
                &resolver(),
            )
            .expect_err("receipt-free non-admission failure must fail closed");

            assert!(error.to_string().contains(
                "receipt-free InvokeResponse must be a typed pre-admission Failed outcome"
            ));
        }
    }

    #[test]
    fn partial_receipt_checkpoint_chain_is_rejected() {
        let error = InvocationOutcome::from_invoke_response(
            tuple(),
            InvokeResponse {
                state: InvocationState::Failed.to_wire_i32(),
                error: Some(admission_error()),
                admission_receipt: Some(InvocationReceipt::default()),
                ..InvokeResponse::default()
            },
            &resolver(),
        )
        .expect_err("partial receipt chain must fail closed");

        assert!(error
            .to_string()
            .contains("partial receipt checkpoint chain"));
    }
}
