use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::support::local_daemon_grpc;

use super::{DaemonEndpoints, DaemonError, DaemonInvocation, Result};

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
/// `DaemonStartConfig`/`DaemonHandle` in `daemon::process`.
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
        let ability = invocation.ability().to_string();
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
        let ability = invocation.ability().to_string();
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
        let ability = invocation.ability().to_string();
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
