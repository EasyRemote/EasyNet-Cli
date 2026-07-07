use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::support::platform::local_daemon_grpc;

use super::DaemonInvocation;
use super::{
    DaemonInvocationBuilder, InvocationDraft, InvocationTuple, PrepareOptions, PreparedInvocation,
    SignedInvocation,
};
use crate::daemon::boot::DaemonEndpoints;
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
        Self::connect(DaemonEndpoints::current().invocation)
    }

    /// Endpoint this client dials.
    pub fn endpoint(&self) -> &Path {
        &self.endpoint
    }

    /// Invoke a complete daemon Invocation through Axon's gRPC
    /// `Invocation.Invoke` method.
    #[cfg(feature = "axon-pb")]
    pub async fn invoke(
        &self,
        invocation: DaemonInvocation,
    ) -> Result<easynet_axon::pb::axon::v1::InvokeResponse> {
        let ability = invocation.descriptor_ref().to_string();
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
        let mut client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel);
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

    /// Open a complete daemon Invocation through Axon's gRPC
    /// `Invocation.InvokeStream` method.
    #[cfg(feature = "axon-pb")]
    pub async fn invoke_stream(
        &self,
        invocation: DaemonInvocation,
    ) -> Result<tonic::Streaming<easynet_axon::pb::axon::v1::InvokeStreamChunk>> {
        let ability = invocation.descriptor_ref().to_string();
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
        let mut client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel);
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

    /// Open a complete daemon Invocation through Axon's gRPC
    /// `Invocation.InvokeBidi` method.
    ///
    /// Frame 0 is an `EnvelopeOpen` generated from the same complete
    /// `DaemonInvocation` tuple used by unary and server-stream calls.
    #[cfg(feature = "axon-pb")]
    pub async fn invoke_bidi(
        &self,
        invocation: DaemonInvocation,
        streams: Vec<easynet_axon::pb::axon::v1::StreamDescriptor>,
    ) -> Result<DaemonBidiSession> {
        let ability = invocation.descriptor_ref().to_string();
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
        let mut client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel);
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
            next_sequence: 1,
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
#[derive(Debug, Clone)]
pub struct RuntimeClient {
    inner: DaemonClient,
    state: ClientConnectionState,
}

impl RuntimeClient {
    /// Connect to an explicit daemon Invocation endpoint.
    pub fn connect(endpoint: impl Into<PathBuf>) -> Result<Self> {
        let inner = DaemonClient::connect(endpoint)?;
        Ok(Self {
            inner,
            state: ClientConnectionState::Ready,
        })
    }

    /// Connect to the current local daemon Invocation endpoint.
    pub fn local() -> Result<Self> {
        Self::connect(DaemonEndpoints::current().invocation)
    }

    /// Current SDK-observed connection state.
    pub fn state(&self) -> ClientConnectionState {
        self.state
    }

    /// Endpoint this runtime client dials.
    pub fn endpoint(&self) -> &Path {
        self.inner.endpoint()
    }

    /// Start an SDK invocation builder.
    pub fn new_invocation(
        &self,
        caller_ura: impl Into<String>,
        callee_ura: impl Into<String>,
        descriptor_ref: impl Into<String>,
        subject_ura: impl Into<String>,
    ) -> Result<DaemonInvocationBuilder> {
        DaemonInvocation::builder(caller_ura, callee_ura, descriptor_ref, subject_ura)
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
        let response = self.inner.invoke(signed.into_daemon_invocation()).await?;
        Ok(InvocationHandle {
            result: InvocationResult::from_invoke_response(tuple, response),
        })
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
    result: InvocationResult,
}

impl InvocationHandle {
    pub fn await_result(self) -> InvocationResult {
        self.result
    }

    pub fn result(&self) -> &InvocationResult {
        &self.result
    }
}

/// Terminal unary Invocation projection.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InvocationResult {
    pub tuple: InvocationTuple,
    pub terminal_state: String,
    pub output_content_type: String,
    pub output: Vec<u8>,
    pub selected_node_id: String,
    pub scheduling_reason: String,
    pub elapsed_ms: u64,
    pub receipt: Option<ReceiptSummary>,
    pub error: Option<RuntimeErrorSummary>,
}

impl InvocationResult {
    fn from_invoke_response(
        tuple: InvocationTuple,
        response: easynet_axon::pb::axon::v1::InvokeResponse,
    ) -> Self {
        let error = response.error.as_ref().map(RuntimeErrorSummary::from_wire);
        Self {
            tuple,
            terminal_state: invocation_state_name(response.state),
            output_content_type: response.result_content_type,
            output: response.result,
            selected_node_id: response.selected_node_id,
            scheduling_reason: response.scheduling_reason,
            elapsed_ms: response.elapsed_ms.max(0) as u64,
            receipt: response
                .admission_receipt
                .as_ref()
                .map(ReceiptSummary::from_wire),
            error,
        }
    }
}

fn invocation_state_name(state: i32) -> String {
    use easynet_axon::invocation::InvocationState;

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
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ReceiptSummary {
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
}

impl ReceiptSummary {
    pub(crate) fn from_wire(receipt: &easynet_axon::pb::axon::v1::InvocationReceipt) -> Self {
        Self {
            index: receipt.index,
            invocation_id: receipt.invocation_id.clone(),
            receipt_type: receipt.receipt_type.clone(),
            state: receipt.state.to_string(),
            timestamp_unix_ms: receipt.timestamp_unix_ms,
            prev_receipt_hash_hex: hex::encode(&receipt.prev_receipt_hash),
            self_hash_hex: hex::encode(&receipt.self_hash),
            payload_content_type: receipt.payload_content_type.clone(),
            cleanup_complete: receipt.cleanup_complete,
            reason: receipt.reason.clone(),
            child_invocation_id: receipt.child_invocation_id.clone(),
        }
    }
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
    pub(crate) fn from_wire(error: &easynet_axon::pb::axon::v1::Error) -> Self {
        Self {
            code: error.code.clone(),
            stage: "runtime".to_string(),
            message: error.message.clone(),
            retryable: error.retryable,
        }
    }
}

/// Active SDK-owned InvokeBidi session.
///
/// Invariants:
/// 1. Frame 0 has already been sent before construction.
/// 2. `next_sequence` starts at 1 and increments exactly once per
///    up-direction frame sent through this session.
/// 3. Dropping the session closes the up-direction stream. It does
///    not synthesize protocol EOF; callers that need graceful close
///    should call `send_eof` first.
#[cfg(feature = "axon-pb")]
pub struct DaemonBidiSession {
    ability: String,
    up_tx: tokio::sync::mpsc::Sender<easynet_axon::pb::axon::v1::InvokeBidiUp>,
    down: tonic::Streaming<easynet_axon::pb::axon::v1::InvokeBidiDown>,
    next_sequence: u64,
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
        tokio::sync::mpsc::Sender<easynet_axon::pb::axon::v1::InvokeBidiUp>,
        tonic::Streaming<easynet_axon::pb::axon::v1::InvokeBidiDown>,
    ) {
        (self.ability, self.up_tx, self.down)
    }

    /// Read the next down-direction frame.
    pub async fn next_down(
        &mut self,
    ) -> Result<Option<easynet_axon::pb::axon::v1::InvokeBidiDown>> {
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
        chunk: easynet_axon::pb::axon::v1::BinaryChunk,
    ) -> Result<()> {
        use easynet_axon::pb::axon::v1::{invoke_bidi_up, InvokeBidiUp};
        let sequence = self.take_next_sequence();
        self.up_tx
            .send(InvokeBidiUp {
                sequence,
                mac: Vec::new(),
                payload: Some(invoke_bidi_up::Payload::BinaryChunk(chunk)),
            })
            .await
            .map_err(|_| DaemonError::InvokeBidiClosed {
                ability: self.ability.clone(),
            })
    }

    /// Send a control frame on the up direction.
    pub async fn send_control(
        &mut self,
        control: easynet_axon::pb::axon::v1::BidiControl,
    ) -> Result<()> {
        use easynet_axon::pb::axon::v1::{invoke_bidi_up, InvokeBidiUp};
        let sequence = self.take_next_sequence();
        self.up_tx
            .send(InvokeBidiUp {
                sequence,
                mac: Vec::new(),
                payload: Some(invoke_bidi_up::Payload::Control(control)),
            })
            .await
            .map_err(|_| DaemonError::InvokeBidiClosed {
                ability: self.ability.clone(),
            })
    }

    /// Send a graceful EOF control frame.
    pub async fn send_eof(&mut self) -> Result<()> {
        use easynet_axon::pb::axon::v1::{bidi_control, BidiControl};
        self.send_control(BidiControl {
            control: Some(bidi_control::Control::Eof(true)),
        })
        .await
    }

    fn take_next_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        sequence
    }
}
