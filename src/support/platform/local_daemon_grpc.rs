// EasyNet CLI — local daemon Invocation transport helpers
// =======================================================
//
// File: src/support/local_daemon_grpc.rs
// Description: Resolve, probe, and (when `axon-pb` is enabled)
//              connect to the local easynet-daemon Invocation
//              listener across Unix UDS and Windows named pipes.
//
// Why this exists
// ---------------
// The CLI had grown four separate copies of "resolve daemon.sock,
// build a tonic endpoint, connect over UnixStream". That made the
// Windows port brittle: every copy needed a different named-pipe
// branch, and missing one would silently strand a subcommand. This
// module is the single control point for the local daemon transport.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(feature = "axon-pb")]
use std::collections::{HashSet, VecDeque};
#[cfg(feature = "axon-pb")]
use std::sync::{Mutex, OnceLock};

#[cfg(feature = "axon-pb")]
use crate::daemon::ability::{
    HostedAgentDelegationRequest, HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY,
};
#[cfg(feature = "axon-pb")]
use crate::daemon::invocation::dispatch::invocation_wire::{
    wire_invocation_target, InvocationDerivationPolicy, LocalDaemonLoopbackInvocation,
};

/// Resolve the local daemon Invocation endpoint. Thin re-export of
/// [`crate::daemon::persistence::daemon_config::resolved_local_uds_path_with_env_override`]
/// kept here so the existing CLI call sites
/// (`cli/federation_discover.rs`, `cli/start.rs`,
/// `support/remote_invoke.rs`) need no rewrite. The body itself
/// lives in `persistence/` because it consults `daemon-config.toml`
/// — keeping it there preserves the `support/` leaf-layer invariant
/// documented in `src/support/mod.rs`.
pub(crate) fn resolve_socket_path() -> PathBuf {
    crate::daemon::persistence::daemon_config::resolved_local_uds_path_with_env_override()
}

/// Best-effort local liveness probe used by `runtime start` to avoid
/// racing a second daemon process against an already-live listener.
pub(crate) fn probe_accepting(path: &Path) -> bool {
    #[cfg(unix)]
    {
        if !path.exists() {
            return false;
        }
        std::os::unix::net::UnixStream::connect(path).is_ok()
    }

    #[cfg(windows)]
    {
        let Some(name) = path.to_str() else {
            return false;
        };
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(_) => return false,
        };
        return runtime
            .block_on(crate::support::platform::named_pipe::connect_with_retry(
                name,
                Duration::from_millis(250),
            ))
            .is_ok();
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        false
    }
}

#[cfg(feature = "axon-pb")]
pub(crate) async fn connect_channel(
    path: PathBuf,
    timeout: Duration,
    connect_timeout: Duration,
) -> anyhow::Result<tonic::transport::Channel> {
    let endpoint = tonic::transport::Endpoint::try_from("http://[::1]:50051")?
        .timeout(timeout)
        .connect_timeout(connect_timeout);

    #[cfg(unix)]
    {
        return endpoint
            .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let path = path.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                }
            }))
            .await
            .map_err(Into::into);
    }

    #[cfg(windows)]
    {
        let pipe_name = path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("named-pipe path is not valid UTF-8"))?
            .to_string();
        return endpoint
            .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let pipe_name = pipe_name.clone();
                async move {
                    let stream = crate::support::platform::named_pipe::connect_with_retry(
                        &pipe_name,
                        connect_timeout,
                    )
                    .await?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                }
            }))
            .await
            .map_err(Into::into);
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        let _ = timeout;
        let _ = connect_timeout;
        anyhow::bail!("local daemon Invocation transport is unsupported on this platform");
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalDaemonLoopbackCalleePolicy {
    LocalDaemon,
    Explicit(String),
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalDaemonLoopbackSubjectPolicy {
    Explicit(String),
    LocalDaemonSelf,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq)]
enum LocalDaemonLoopbackDerivationPolicy {
    FreshRoot,
    ExplicitCausal {
        invocation_nonce: [u8; 16],
        causal_context: axon_sdk::pb::axon::v1::CausalContext,
    },
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
struct LocalDaemonLoopbackTuplePlan {
    function_name: String,
    payload_json: serde_json::Value,
    caller_ura: String,
    callee_policy: LocalDaemonLoopbackCalleePolicy,
    subject_policy: LocalDaemonLoopbackSubjectPolicy,
    derivation_policy: LocalDaemonLoopbackDerivationPolicy,
    timeout: Duration,
}

pub(crate) struct LocalDaemonTargetedBidiRequest<'a> {
    pub function_name: &'a str,
    pub payload_json: serde_json::Value,
    pub callee_ura: &'a str,
    pub subject_ura: &'a str,
    pub invocation_nonce: [u8; 16],
    pub timeout: Duration,
    pub input_frames: Vec<serde_json::Value>,
    pub max_frames: Option<usize>,
}

#[cfg(feature = "axon-pb")]
impl LocalDaemonLoopbackCalleePolicy {
    fn local_daemon() -> Self {
        Self::LocalDaemon
    }

    fn explicit(callee_ura: &str) -> anyhow::Result<Self> {
        Ok(Self::Explicit(normalized_local_daemon_ura(
            callee_ura,
            "callee_ura",
        )?))
    }

    fn resolve(&self) -> anyhow::Result<String> {
        match self {
            Self::LocalDaemon => local_daemon_default_callee_ura(),
            Self::Explicit(callee_ura) => normalized_local_daemon_ura(callee_ura, "callee_ura"),
        }
    }
}

#[cfg(feature = "axon-pb")]
impl LocalDaemonLoopbackSubjectPolicy {
    fn required_explicit(subject_ura: &str) -> anyhow::Result<Self> {
        Ok(Self::Explicit(normalized_local_daemon_ura(
            subject_ura,
            "subject_ura",
        )?))
    }

    fn local_daemon_self() -> Self {
        Self::LocalDaemonSelf
    }

    fn resolve(&self, callee_ura: &str) -> anyhow::Result<String> {
        match self {
            Self::Explicit(subject) => normalized_local_daemon_ura(subject, "subject_ura"),
            Self::LocalDaemonSelf => normalized_local_daemon_ura(callee_ura, "callee_ura"),
        }
    }

    fn explicit(subject: &str) -> anyhow::Result<Self> {
        normalized_local_daemon_ura(subject, "subject_ura").map(Self::Explicit)
    }
}

#[cfg(feature = "axon-pb")]
impl LocalDaemonLoopbackDerivationPolicy {
    fn fresh_root() -> Self {
        Self::FreshRoot
    }

    fn explicit_causal(
        invocation_nonce: [u8; 16],
        causal_context: axon_sdk::pb::axon::v1::CausalContext,
    ) -> anyhow::Result<Self> {
        if invocation_nonce == [0; 16] {
            anyhow::bail!("invocation_nonce must not be all-zero");
        }
        InvocationDerivationPolicy::try_explicit_from_wire_causal_context(
            invocation_nonce,
            causal_context.clone(),
        )
        .map_err(|error| anyhow::anyhow!("invalid explicit causal context: {error}"))?;
        Ok(Self::ExplicitCausal {
            invocation_nonce,
            causal_context,
        })
    }

    fn as_axon(&self) -> anyhow::Result<InvocationDerivationPolicy> {
        match self {
            Self::FreshRoot => Ok(InvocationDerivationPolicy::FreshRoot),
            Self::ExplicitCausal {
                invocation_nonce,
                causal_context,
            } => InvocationDerivationPolicy::try_explicit_from_wire_causal_context(
                *invocation_nonce,
                causal_context.clone(),
            )
            .map_err(|error| anyhow::anyhow!("invalid explicit causal context: {error}")),
        }
    }
}

#[cfg(feature = "axon-pb")]
impl LocalDaemonLoopbackTuplePlan {
    fn local_root(
        function_name: &str,
        payload_json: serde_json::Value,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        Self::new(
            function_name,
            payload_json,
            LocalDaemonLoopbackCalleePolicy::local_daemon(),
            LocalDaemonLoopbackSubjectPolicy::local_daemon_self(),
            LocalDaemonLoopbackDerivationPolicy::fresh_root(),
            timeout,
        )
    }

    fn local_root_for_subject(
        function_name: &str,
        payload_json: serde_json::Value,
        subject_ura: &str,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        Self::new(
            function_name,
            payload_json,
            LocalDaemonLoopbackCalleePolicy::local_daemon(),
            LocalDaemonLoopbackSubjectPolicy::explicit(subject_ura)?,
            LocalDaemonLoopbackDerivationPolicy::fresh_root(),
            timeout,
        )
    }

    fn targeted_root_for_subject(
        function_name: &str,
        payload_json: serde_json::Value,
        callee_ura: &str,
        subject_ura: &str,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        Self::new(
            function_name,
            payload_json,
            LocalDaemonLoopbackCalleePolicy::explicit(callee_ura)?,
            LocalDaemonLoopbackSubjectPolicy::required_explicit(subject_ura)?,
            LocalDaemonLoopbackDerivationPolicy::fresh_root(),
            timeout,
        )
    }

    fn targeted_explicit_causal(
        function_name: &str,
        payload_json: serde_json::Value,
        callee_ura: &str,
        subject_ura: &str,
        invocation_nonce: [u8; 16],
        causal_context: axon_sdk::pb::axon::v1::CausalContext,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        Self::new(
            function_name,
            payload_json,
            LocalDaemonLoopbackCalleePolicy::explicit(callee_ura)?,
            LocalDaemonLoopbackSubjectPolicy::required_explicit(subject_ura)?,
            LocalDaemonLoopbackDerivationPolicy::explicit_causal(invocation_nonce, causal_context)?,
            timeout,
        )
    }

    fn new(
        function_name: &str,
        payload_json: serde_json::Value,
        callee_policy: LocalDaemonLoopbackCalleePolicy,
        subject_policy: LocalDaemonLoopbackSubjectPolicy,
        derivation_policy: LocalDaemonLoopbackDerivationPolicy,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        let function_name = function_name.trim();
        if function_name.is_empty() {
            anyhow::bail!("function_name must not be empty");
        }
        if timeout.is_zero() {
            anyhow::bail!("{function_name}: timeout must be greater than zero");
        }
        Ok(Self {
            function_name: function_name.to_string(),
            payload_json,
            caller_ura: local_daemon_loopback_caller_ura()?,
            callee_policy,
            subject_policy,
            derivation_policy,
            timeout,
        })
    }

    fn into_invocation(self) -> anyhow::Result<LocalDaemonLoopbackInvocation> {
        let callee_ura = self.callee_policy.resolve()?;
        let subject_ura = self.subject_policy.resolve(&callee_ura)?;
        LocalDaemonLoopbackInvocation::from_target(
            &self.function_name,
            self.payload_json,
            self.caller_ura,
            callee_ura,
            subject_ura,
            self.derivation_policy.as_axon()?,
            self.timeout,
        )
    }
}

#[cfg(feature = "axon-pb")]
fn normalized_local_daemon_ura(value: &str, field: &str) -> anyhow::Result<String> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("{field} must not be empty");
    }
    crate::core::ura::parse_ura(value)
        .map_err(|err| anyhow::anyhow!("{field} is not a valid URA: {err}"))?;
    Ok(value.to_string())
}

#[cfg(feature = "axon-pb")]
fn local_daemon_loopback_invocation_from_tuple_plan(
    tuple_plan: LocalDaemonLoopbackTuplePlan,
) -> anyhow::Result<LocalDaemonLoopbackInvocation> {
    tuple_plan.into_invocation()
}

#[cfg(feature = "axon-pb")]
fn local_daemon_status_error(function_name: &str, status: tonic::Status) -> anyhow::Error {
    anyhow::Error::new(
        crate::support::platform::local_invoke::LocalInvokeFailure::DaemonStatus {
            ability: function_name.to_string(),
            code: status.code().into(),
            message: status.message().to_string(),
        },
    )
}

#[cfg(feature = "axon-pb")]
fn local_daemon_connect_error(
    socket_path: &std::path::Path,
    source: anyhow::Error,
) -> anyhow::Error {
    anyhow::Error::new(
        crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
            "connect to local Axon daemon gRPC endpoint at {}: {source:#}",
            socket_path.display()
        )),
    )
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability(
    function_name: &str,
    payload_json: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let tuple_plan = LocalDaemonLoopbackTuplePlan::local_root(
        function_name,
        payload_json,
        Duration::from_secs(30),
    )?;
    invoke_local_daemon_ability_with_tuple_plan(tuple_plan)
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_system_ability_root_for_subject_timeout(
    function_name: &str,
    payload_json: serde_json::Value,
    subject_ura: &str,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let tuple_plan = LocalDaemonLoopbackTuplePlan::local_root_for_subject(
        function_name,
        payload_json,
        subject_ura,
        timeout,
    )?;
    invoke_local_daemon_ability_with_tuple_plan(tuple_plan)
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_system_ability_targeted_root_timeout(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    subject_ura: &str,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let tuple_plan = LocalDaemonLoopbackTuplePlan::targeted_root_for_subject(
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        timeout,
    )?;
    invoke_local_daemon_ability_with_tuple_plan(tuple_plan)
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_targeted_explicit_root_timeout(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    subject_ura: &str,
    invocation_nonce: [u8; 16],
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let tuple_plan = LocalDaemonLoopbackTuplePlan::targeted_explicit_causal(
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        invocation_nonce,
        axon_sdk::pb::axon::v1::CausalContext {
            form: Some(axon_sdk::pb::axon::v1::causal_context::Form::None(
                axon_sdk::pb::axon::v1::Empty {},
            )),
        },
        timeout,
    )?;
    invoke_local_daemon_ability_with_tuple_plan(tuple_plan)
}

/// Invoke a daemon-hosted server-stream ability through Axon's local
/// Invocation gRPC transport and drain its JSON frames.
///
/// This is the stream-mode twin of the unary daemon-system root issuer. It
/// deliberately talks to the daemon process, not an in-process test runtime,
/// because stateful stream abilities keep their session state inside that
/// process.
#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_system_ability_targeted_stream_root(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    subject_ura: &str,
    timeout: Duration,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    let tuple_plan = LocalDaemonLoopbackTuplePlan::targeted_root_for_subject(
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        timeout,
    )?;
    invoke_local_daemon_ability_stream_with_tuple_plan(tuple_plan, max_frames)
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_targeted_stream_explicit_root(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    subject_ura: &str,
    invocation_nonce: [u8; 16],
    timeout: Duration,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    let tuple_plan = LocalDaemonLoopbackTuplePlan::targeted_explicit_causal(
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        invocation_nonce,
        axon_sdk::pb::axon::v1::CausalContext {
            form: Some(axon_sdk::pb::axon::v1::causal_context::Form::None(
                axon_sdk::pb::axon::v1::Empty {},
            )),
        },
        timeout,
    )?;
    invoke_local_daemon_ability_stream_with_tuple_plan(tuple_plan, max_frames)
}

/// Open a daemon-hosted bidirectional ability through Axon's local
/// Invocation gRPC transport and drain JSON-frame down output.
#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_targeted_bidi_json_frames_explicit_root(
    request: LocalDaemonTargetedBidiRequest<'_>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalBidiFrame>> {
    let LocalDaemonTargetedBidiRequest {
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        invocation_nonce,
        timeout,
        input_frames,
        max_frames,
    } = request;
    let tuple_plan = LocalDaemonLoopbackTuplePlan::targeted_explicit_causal(
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        invocation_nonce,
        axon_sdk::pb::axon::v1::CausalContext {
            form: Some(axon_sdk::pb::axon::v1::causal_context::Form::None(
                axon_sdk::pb::axon::v1::Empty {},
            )),
        },
        timeout,
    )?;
    invoke_local_daemon_ability_bidi_json_frames_with_tuple_plan(
        tuple_plan,
        input_frames,
        max_frames,
    )
}

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_stream_with_tuple_plan(
    tuple_plan: LocalDaemonLoopbackTuplePlan,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    use anyhow::Context;
    let timeout = tuple_plan.timeout;

    let socket_path = resolve_socket_path();
    if !probe_accepting(&socket_path) {
        return Err(anyhow::Error::new(
            crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
                "daemon not running (local Axon gRPC listener unreachable at {}). \
                 Start it with `easynet runtime start`.",
                socket_path.display()
            )),
        ));
    }

    let invocation = local_daemon_loopback_invocation_from_tuple_plan(tuple_plan)?;
    let function_name = invocation.function_name().to_string();
    let request = invocation.stream_request()?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for local Axon daemon stream invoke")?;

    runtime.block_on(async move {
        let channel = connect_channel(socket_path.clone(), timeout, Duration::from_secs(10))
            .await
            .map_err(|source| local_daemon_connect_error(&socket_path, source))?;
        let mut client = axon_sdk::pb::axon::v1::invocation_client::InvocationClient::new(channel);
        let mut stream = client
            .invoke_stream(request)
            .await
            .map_err(|status| local_daemon_status_error(&function_name, status))?
            .into_inner();

        let mut frames = Vec::new();
        while let Some(chunk) = stream
            .message()
            .await
            .map_err(|status| local_daemon_status_error(&function_name, status))?
        {
            let payload = if chunk.payload.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::from_slice(&chunk.payload)
                    .with_context(|| format!("decode {function_name} stream frame JSON"))?
            };
            let terminal = chunk.terminal;
            frames.push(crate::support::platform::local_invoke::LocalStreamFrame {
                sequence: chunk.sequence,
                content_type: chunk.content_type,
                terminal,
                payload,
            });
            if terminal {
                break;
            }
            if max_frames.is_some_and(|limit| frames.len() >= limit) {
                break;
            }
        }
        Ok::<_, anyhow::Error>(frames)
    })
}

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_bidi_json_frames_with_tuple_plan(
    tuple_plan: LocalDaemonLoopbackTuplePlan,
    input_frames: Vec<serde_json::Value>,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalBidiFrame>> {
    use anyhow::Context;
    use axon_sdk::pb::axon::v1::{
        invoke_bidi_up::Payload as UpPayload, BinaryChunk, ContentEnvelope, EnvelopeOpen,
        InvokeBidiUp, StreamDescriptor,
    };
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;
    let timeout = tuple_plan.timeout;

    let socket_path = resolve_socket_path();
    if !probe_accepting(&socket_path) {
        return Err(anyhow::Error::new(
            crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
                "daemon not running (local Axon gRPC listener unreachable at {}). \
                 Start it with `easynet runtime start`.",
                socket_path.display()
            )),
        ));
    }

    let invocation = local_daemon_loopback_invocation_from_tuple_plan(tuple_plan)?;
    let function_name = invocation.function_name().to_string();
    let envelope_open = EnvelopeOpen {
        envelope: Some(invocation.envelope()?),
        target: Some(wire_invocation_target(
            function_name.clone(),
            function_name.clone(),
        )?),
        initial_args: invocation.arguments().to_vec(),
        args_content_type: "application/json".to_string(),
        streams: vec![StreamDescriptor {
            stream_id: 1,
            content_type: "application/json".to_string(),
            ordering: "STRICT".to_string(),
            ..StreamDescriptor::default()
        }],
        content_envelope: Some(ContentEnvelope {
            content_type: "application/json".to_string(),
            encoding: "identity".to_string(),
            ..ContentEnvelope::default()
        }),
        ..EnvelopeOpen::default()
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for local Axon daemon bidi invoke")?;

    runtime.block_on(async move {
        let channel = connect_channel(socket_path.clone(), timeout, Duration::from_secs(10))
            .await
            .map_err(|source| local_daemon_connect_error(&socket_path, source))?;
        let mut client = axon_sdk::pb::axon::v1::invocation_client::InvocationClient::new(channel)
            .max_decoding_message_size(
                crate::daemon::boot::invocation::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
            )
            .max_encoding_message_size(
                crate::daemon::boot::invocation::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
            );

        let (up_tx, up_rx) = mpsc::channel::<InvokeBidiUp>(16);
        up_tx
            .send(InvokeBidiUp {
                sequence: 0,
                mac: Vec::new(),
                payload: Some(UpPayload::EnvelopeOpen(envelope_open)),
            })
            .await
            .map_err(|_| anyhow::anyhow!("local bidi up channel closed before frame 0"))?;

        let mut next_sequence = 1_u64;
        for input in input_frames {
            let data = serde_json::to_vec(&input)
                .with_context(|| format!("encode {function_name} bidi input JSON frame"))?;
            up_tx
                .send(InvokeBidiUp {
                    sequence: next_sequence,
                    mac: Vec::new(),
                    payload: Some(UpPayload::BinaryChunk(BinaryChunk {
                        stream_id: 1,
                        data,
                        ..BinaryChunk::default()
                    })),
                })
                .await
                .map_err(|_| anyhow::anyhow!("local bidi up channel closed while sending input"))?;
            next_sequence = next_sequence.saturating_add(1);
        }

        let mut down = client
            .invoke_bidi(tonic::Request::new(ReceiverStream::new(up_rx)))
            .await
            .map_err(|status| local_daemon_status_error(&function_name, status))?
            .into_inner();

        let mut frames = Vec::new();
        while let Some(frame) = down
            .message()
            .await
            .map_err(|status| local_daemon_status_error(&function_name, status))?
        {
            let Some(projected) =
                crate::support::platform::local_invoke::project_invoke_bidi_down_frame(frame)?
            else {
                continue;
            };
            let terminal = projected.terminal;
            frames.push(projected);
            if terminal {
                break;
            }
            if max_frames.is_some_and(|limit| frames.len() >= limit) {
                break;
            }
        }
        drop(up_tx);
        Ok::<_, anyhow::Error>(frames)
    })
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_system_ability_targeted_stream_root(
    function_name: &str,
    _payload_json: serde_json::Value,
    _callee_ura: &str,
    _subject_ura: &str,
    _timeout: Duration,
    _max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    anyhow::bail!(
        "streaming `{}` through the local Axon daemon requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_targeted_stream_explicit_root(
    function_name: &str,
    _payload_json: serde_json::Value,
    _callee_ura: &str,
    _subject_ura: &str,
    _invocation_nonce: [u8; 16],
    _timeout: Duration,
    _max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    anyhow::bail!(
        "streaming `{}` through the local Axon daemon requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_targeted_bidi_json_frames_explicit_root(
    request: LocalDaemonTargetedBidiRequest<'_>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalBidiFrame>> {
    let function_name = request.function_name;
    anyhow::bail!(
        "bidirectional `{}` through the local Axon daemon requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
}

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_with_tuple_plan(
    tuple_plan: LocalDaemonLoopbackTuplePlan,
) -> anyhow::Result<serde_json::Value> {
    use anyhow::Context;
    let timeout = tuple_plan.timeout;

    let socket_path = resolve_socket_path();
    if !probe_accepting(&socket_path) {
        return Err(anyhow::Error::new(
            crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
                "daemon not running (local Axon gRPC listener unreachable at {}). \
                 Start it with `easynet runtime start`.",
                socket_path.display()
            )),
        ));
    }

    let invocation = local_daemon_loopback_invocation_from_tuple_plan(tuple_plan)?;
    let function_name = invocation.function_name().to_string();
    let request = invocation.invoke_request()?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for local Axon daemon invoke")?;

    runtime.block_on(async move {
        let channel = connect_channel(socket_path.clone(), timeout, Duration::from_secs(10))
            .await
            .map_err(|source| local_daemon_connect_error(&socket_path, source))?;
        let mut client = axon_sdk::pb::axon::v1::invocation_client::InvocationClient::new(channel);
        let (value, _) = invoke_local_daemon_json(&mut client, request, &function_name).await?;
        Ok(value)
    })
}

pub(crate) struct LocalDaemonTargetedInvocationMetaRequest<'a> {
    pub(crate) function_name: &'a str,
    pub(crate) payload_json: serde_json::Value,
    pub(crate) callee_ura: &'a str,
    pub(crate) subject_ura: &'a str,
    pub(crate) invocation_nonce: [u8; 16],
    pub(crate) causal_parents: &'a [serde_json::Value],
    pub(crate) step_timeout: Duration,
    pub(crate) trace_id: Option<&'a str>,
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_targeted_with_invocation_meta(
    request: LocalDaemonTargetedInvocationMetaRequest<'_>,
) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
    invoke_local_daemon_ability_with_invocation_meta_inner(request, None)
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_targeted_with_hosted_agent_delegation(
    request: LocalDaemonTargetedInvocationMetaRequest<'_>,
    hosted_agent_ura: &str,
) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
    let delegated = HostedAgentDelegationRequest::new(hosted_agent_ura)?;
    invoke_local_daemon_ability_with_invocation_meta_inner(request, Some(delegated))
}

#[cfg(feature = "axon-pb")]
#[derive(Debug)]
struct UnverifiedTerminalInvocationProjection {
    state: &'static str,
    admission_receipt: axon_sdk::pb::axon::v1::InvocationReceipt,
    terminal_receipt: axon_sdk::pb::axon::v1::InvocationReceipt,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug)]
struct VerifiedTerminalInvocationProjection {
    invocation_id: String,
    invocation_ura: String,
    caller_ura: String,
    callee_ura: String,
    subject_ura: String,
    state: &'static str,
    receipt: serde_json::Value,
    causal_anchor: VerifiedCausalAnchor,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CausalAnchorKey {
    receipt_ura: String,
    receipt_hash: [u8; 32],
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
struct VerifiedCausalReceiptRef(CausalAnchorKey);

#[cfg(feature = "axon-pb")]
impl VerifiedCausalReceiptRef {
    fn to_wire(&self) -> axon_sdk::pb::axon::v1::ReceiptRef {
        axon_sdk::pb::axon::v1::ReceiptRef {
            receipt_ura: self.0.receipt_ura.clone(),
            receipt_hash: self.0.receipt_hash.to_vec(),
        }
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
struct VerifiedCausalAnchor {
    reference: VerifiedCausalReceiptRef,
    receipt_type: String,
    state: &'static str,
    timestamp_unix_ms: i64,
    anchor_count: u64,
}

#[cfg(feature = "axon-pb")]
impl VerifiedCausalAnchor {
    fn from_terminal(
        invocation_ura: &str,
        terminal: &axon_sdk::invocation::SignedInvocationReceipt,
        state: &'static str,
    ) -> Self {
        Self {
            reference: VerifiedCausalReceiptRef(CausalAnchorKey {
                receipt_ura: format!(
                    "{}/receipt/{}",
                    invocation_ura.trim_end_matches('/'),
                    terminal.index()
                ),
                receipt_hash: terminal.self_hash(),
            }),
            receipt_type: terminal.receipt_type().to_string(),
            state,
            timestamp_unix_ms: terminal.timestamp_unix_ms(),
            anchor_count: terminal.index().saturating_add(1),
        }
    }

    fn projection(&self) -> serde_json::Value {
        serde_json::json!({
            "receipt_ura": self.reference.0.receipt_ura,
            "receipt_hash": hex::encode(self.reference.0.receipt_hash),
            "receipt_type": self.receipt_type,
            "state": self.state,
            "timestamp_unix_ms": self.timestamp_unix_ms,
        })
    }
}

#[cfg(feature = "axon-pb")]
const MAX_VERIFIED_CAUSAL_ANCHORS: usize = 4_096;

#[cfg(feature = "axon-pb")]
#[derive(Debug, Default)]
struct VerifiedCausalAnchorRegistry {
    anchors: HashSet<CausalAnchorKey>,
    insertion_order: VecDeque<CausalAnchorKey>,
}

#[cfg(feature = "axon-pb")]
impl VerifiedCausalAnchorRegistry {
    fn record(&mut self, anchor: &VerifiedCausalAnchor) {
        let key = anchor.reference.0.clone();
        if !self.anchors.insert(key.clone()) {
            return;
        }
        self.insertion_order.push_back(key);
        while self.insertion_order.len() > MAX_VERIFIED_CAUSAL_ANCHORS {
            if let Some(expired) = self.insertion_order.pop_front() {
                self.anchors.remove(&expired);
            }
        }
    }

    fn restore(&self, claim: &CausalAnchorKey) -> Option<VerifiedCausalReceiptRef> {
        self.anchors
            .contains(claim)
            .then(|| VerifiedCausalReceiptRef(claim.clone()))
    }
}

#[cfg(feature = "axon-pb")]
fn verified_causal_anchor_registry() -> &'static Mutex<VerifiedCausalAnchorRegistry> {
    static REGISTRY: OnceLock<Mutex<VerifiedCausalAnchorRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(VerifiedCausalAnchorRegistry::default()))
}

#[cfg(feature = "axon-pb")]
fn record_verified_causal_anchor(anchor: &VerifiedCausalAnchor) -> anyhow::Result<()> {
    verified_causal_anchor_registry()
        .lock()
        .map_err(|_| anyhow::anyhow!("verified causal anchor registry is poisoned"))?
        .record(anchor);
    Ok(())
}

#[cfg(feature = "axon-pb")]
pub(crate) struct LocalKeyServiceReceiptResolver {
    key_service: crate::daemon::identity::self_identity::KeyringClient,
}

#[cfg(feature = "axon-pb")]
impl LocalKeyServiceReceiptResolver {
    pub(crate) fn new() -> Self {
        Self {
            key_service: crate::daemon::identity::self_identity::KeyringClient::default_path(),
        }
    }
}

#[cfg(feature = "axon-pb")]
impl axon_sdk::invocation::KeyResolver for LocalKeyServiceReceiptResolver {
    fn resolve(
        &self,
        signer_ura: &str,
    ) -> Result<ed25519_dalek::VerifyingKey, axon_sdk::invocation::AxonError> {
        use crate::daemon::identity::self_identity::SelfIdentity as _;

        self.key_service.public_key(signer_ura).map_err(|error| {
            axon_sdk::invocation::AxonError::permission_denied("local_receipt_signer_key_untrusted")
                .with_message(format!(
                "trusted local key service cannot resolve receipt signer {signer_ura:?}: {error}"
            ))
        })
    }
}

#[cfg(feature = "axon-pb")]
pub(crate) struct CanonicalRuntimeReceiptResolver {
    realm_trust: Option<crate::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver>,
    local_self_identity: LocalKeyServiceReceiptResolver,
}

#[cfg(feature = "axon-pb")]
impl CanonicalRuntimeReceiptResolver {
    pub(crate) fn new() -> Self {
        let trust_anchor_path =
            crate::daemon::trust::anchor::trust_anchor_path_from_env_or_default();
        let realm_trust =
            crate::daemon::trust::anchor::RealmTrustAnchor::load_or_empty(&trust_anchor_path)
                .ok()
                .filter(|anchor| !anchor.is_empty())
                .map(|anchor| {
                    crate::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver::new(
                        crate::daemon::trust::cell::SharedTrustAnchor::new(std::sync::Arc::new(
                            anchor,
                        )),
                    )
                });
        Self {
            realm_trust,
            local_self_identity: LocalKeyServiceReceiptResolver::new(),
        }
    }
}

#[cfg(feature = "axon-pb")]
impl axon_sdk::invocation::KeyResolver for CanonicalRuntimeReceiptResolver {
    fn resolve(
        &self,
        signer_ura: &str,
    ) -> Result<ed25519_dalek::VerifyingKey, axon_sdk::invocation::AxonError> {
        let local_error =
            match axon_sdk::invocation::KeyResolver::resolve(&self.local_self_identity, signer_ura)
            {
                Ok(key) => return Ok(key),
                Err(error) => error.to_string(),
            };
        match self.realm_trust.as_ref() {
            Some(resolver) => {
                match axon_sdk::invocation::KeyResolver::resolve(resolver, signer_ura) {
                    Ok(key) => return Ok(key),
                    Err(realm_error) => Err(axon_sdk::invocation::AxonError::permission_denied(
                        "runtime_receipt_signer_key_untrusted",
                    )
                    .with_message(format!(
                        "canonical runtime trust cannot resolve receipt signer {signer_ura:?}: \
                         local_self_identity={local_error}; realm_trust={realm_error}"
                    ))),
                }
            }
            None => Err(axon_sdk::invocation::AxonError::permission_denied(
                "runtime_receipt_signer_key_untrusted",
            )
            .with_message(format!(
                "canonical runtime trust cannot resolve receipt signer {signer_ura:?}: \
                 local_self_identity={local_error}; realm_trust=realm trust anchor is empty or unavailable"
            ))),
        }
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug)]
struct SubmittedInvocationProjection {
    envelope: axon_sdk::pb::axon::v1::Envelope,
    function_name: String,
    input_hash: [u8; 32],
}

#[cfg(feature = "axon-pb")]
impl SubmittedInvocationProjection {
    fn from_request(
        request: &axon_sdk::pb::axon::v1::InvokeRequest,
        ability: &str,
    ) -> anyhow::Result<Self> {
        let envelope = request
            .envelope
            .clone()
            .ok_or_else(|| anyhow::anyhow!("{ability}: invoke request omitted its envelope"))?;
        Ok(Self {
            envelope,
            function_name:
                crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
                    "local daemon submitted invocation",
                    request.target.as_ref(),
                )?
                .to_string(),
            input_hash: axon_sdk::invocation::sha256(&request.arguments),
        })
    }
}

#[cfg(feature = "axon-pb")]
impl UnverifiedTerminalInvocationProjection {
    fn from_response(
        response: &axon_sdk::pb::axon::v1::InvokeResponse,
        submitted: &SubmittedInvocationProjection,
        ability: &str,
    ) -> anyhow::Result<Self> {
        use anyhow::{anyhow, bail};
        use axon_sdk::invocation::InvocationState;

        let state_label = require_completed_invoke_response(response, ability)?;
        let admission = response.admission_receipt.as_ref().ok_or_else(|| {
            anyhow!("{ability}: completed InvokeResponse omitted its signed admission receipt")
        })?;
        let receipt = response.terminal_receipt.as_ref().ok_or_else(|| {
            anyhow!("{ability}: terminal InvokeResponse omitted its signed terminal receipt")
        })?;
        if receipt.invocation_id.trim().is_empty() {
            bail!("{ability}: terminal receipt omitted invocation_id");
        }
        let receipt_state = InvocationState::try_from(receipt.state)
            .map_err(|error| anyhow!("{ability}: invalid terminal receipt state: {error}"))?;
        if receipt_state != InvocationState::Completed {
            bail!(
                "{ability}: response state Completed does not match terminal receipt state {receipt_state:?}"
            );
        }
        if receipt.receipt_type != state_label {
            bail!(
                "{ability}: terminal receipt type {:?} does not match state {state_label:?}",
                receipt.receipt_type
            );
        }
        validate_receipt_signature_shape(admission, "admission", ability)?;
        validate_receipt_signature_shape(receipt, "terminal", ability)?;
        if admission.receipt_type != "admitted"
            || admission.state != InvocationState::Admitted.to_wire_i32()
        {
            bail!("{ability}: admission checkpoint is not an admitted receipt");
        }
        if receipt.index <= admission.index {
            bail!("{ability}: terminal checkpoint index is not after admission");
        }
        if !receipt.cleanup_complete {
            bail!("{ability}: completed terminal receipt did not attest cleanup completion");
        }
        if receipt.failure.is_some() {
            bail!("{ability}: completed terminal receipt carried a typed failure");
        }
        if admission.invocation_id != receipt.invocation_id {
            bail!("{ability}: admission and terminal receipts name different invocations");
        }
        validate_submitted_receipt_binding(admission, submitted, ability, "admission")?;
        validate_submitted_receipt_binding(receipt, submitted, ability, "terminal")?;
        validate_receipt_chain_binding(admission, receipt, ability)?;
        if receipt.output_hash != axon_sdk::invocation::sha256(&response.result) {
            bail!("{ability}: terminal receipt output_hash does not bind the response payload");
        }
        let caller_ura = receipt
            .caller_binding
            .as_ref()
            .map(|binding| binding.ura.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("{ability}: terminal receipt omitted caller binding"))?
            .to_string();
        let callee_ura = receipt
            .callee_binding
            .as_ref()
            .map(|binding| binding.ura.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("{ability}: terminal receipt omitted callee binding"))?
            .to_string();
        let subject_ura = receipt
            .subject_binding
            .as_ref()
            .map(|binding| binding.ura.trim())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("{ability}: terminal receipt omitted subject binding"))?
            .to_string();
        for (field, ura) in [
            ("caller", caller_ura.as_str()),
            ("callee", callee_ura.as_str()),
            ("subject", subject_ura.as_str()),
        ] {
            crate::core::ura::parse_ura(ura)
                .map_err(|error| anyhow!("{ability}: receipt {field} is not a URA: {error}"))?;
        }
        axon_sdk::ura::invocation_record_ura_for_binding(
            &subject_ura,
            &callee_ura,
            &caller_ura,
            &receipt.invocation_id,
        )
        .ok_or_else(|| {
            anyhow!(
                "{ability}: no receipt binding can own invocation {}",
                receipt.invocation_id
            )
        })?;
        Ok(Self {
            state: state_label,
            admission_receipt: admission.clone(),
            terminal_receipt: receipt.clone(),
        })
    }

    fn verify(
        self,
        resolver: &dyn axon_sdk::invocation::KeyResolver,
        ability: &str,
    ) -> anyhow::Result<VerifiedTerminalInvocationProjection> {
        use anyhow::anyhow;

        let checkpoints =
            crate::daemon::invocation::receipts::finalization_projection::verify_wire_finalization_checkpoints(
                self.admission_receipt,
                self.terminal_receipt,
                resolver,
            )
            .map_err(|error| {
                let message = format!("{ability}: {error}");
                anyhow::Error::new(error).context(message)
            })?;
        let terminal = checkpoints.terminal();
        let binding = terminal.axiom_binding();
        let caller_ura = binding.caller.ura.clone();
        let callee_ura = binding.callee.ura.clone();
        let subject_ura = binding.subject.ura.clone();
        let invocation_ura = axon_sdk::ura::invocation_record_ura_for_binding(
            &subject_ura,
            &callee_ura,
            &caller_ura,
            terminal.invocation_id(),
        )
        .ok_or_else(|| {
            anyhow!(
                "{ability}: no verified receipt binding can own invocation {}",
                terminal.invocation_id()
            )
        })?;
        let causal_anchor =
            VerifiedCausalAnchor::from_terminal(&invocation_ura, terminal, self.state);
        let receipt = serde_json::json!({
            "head_receipt_hash": hex::encode(terminal.self_hash()),
            "anchor": causal_anchor.projection(),
            "anchor_count": causal_anchor.anchor_count,
            "cryptographic_verification": "finalization_checkpoints_verified",
            "verification_scope": "admission_and_terminal",
        });
        Ok(VerifiedTerminalInvocationProjection {
            invocation_id: terminal.invocation_id().to_string(),
            invocation_ura,
            caller_ura,
            callee_ura,
            subject_ura,
            state: self.state,
            receipt,
            causal_anchor,
        })
    }
}

#[cfg(feature = "axon-pb")]
fn require_completed_invoke_response(
    response: &axon_sdk::pb::axon::v1::InvokeResponse,
    ability: &str,
) -> anyhow::Result<&'static str> {
    use anyhow::{anyhow, bail};
    use axon_sdk::invocation::InvocationState;

    let state = InvocationState::try_from(response.state)
        .map_err(|error| anyhow!("{ability}: invalid terminal state: {error}"))?;
    if !state.is_terminal() {
        bail!("{ability}: InvokeResponse is not terminal: {state:?}");
    }
    if state != InvocationState::Completed {
        let failure = response
            .error
            .as_ref()
            .or(response.proof_error.as_ref())
            .map(|error| format!("{}: {}", error.code, error.message))
            .unwrap_or_else(|| "terminal response omitted typed failure details".to_string());
        bail!("{ability}: invocation ended in {state:?}: {failure}");
    }
    if response.error.is_some() || response.proof_error.is_some() {
        bail!("{ability}: completed response carried a protocol error");
    }
    state
        .default_event_type()
        .ok_or_else(|| anyhow!("{ability}: terminal response projected an unspecified state"))
}

#[cfg(feature = "axon-pb")]
fn validate_receipt_signature_shape(
    receipt: &axon_sdk::pb::axon::v1::InvocationReceipt,
    stage: &str,
    ability: &str,
) -> anyhow::Result<()> {
    use anyhow::{anyhow, bail};

    if receipt.self_hash.len() != 32 {
        bail!(
            "{ability}: {stage} receipt self_hash must be 32 bytes, got {}",
            receipt.self_hash.len()
        );
    }
    let signature = receipt
        .callee_signature
        .as_ref()
        .ok_or_else(|| anyhow!("{ability}: {stage} receipt omitted its callee signature"))?;
    if signature.algorithm != "ed25519" || signature.signature.len() != 64 {
        bail!("{ability}: {stage} receipt carried an invalid signature shape");
    }
    Ok(())
}

#[cfg(feature = "axon-pb")]
fn validate_submitted_receipt_binding(
    receipt: &axon_sdk::pb::axon::v1::InvocationReceipt,
    submitted: &SubmittedInvocationProjection,
    ability: &str,
    stage: &str,
) -> anyhow::Result<()> {
    use anyhow::bail;

    let envelope = &submitted.envelope;
    if receipt.caller_binding != envelope.caller
        || receipt.callee_binding != envelope.callee
        || receipt.subject_binding != envelope.subject
        || receipt.invocation_nonce != envelope.invocation_nonce
        || receipt.causal_binding != envelope.causal_context
    {
        bail!("{ability}: {stage} receipt does not bind the submitted invocation tuple");
    }
    let ability_ura = axon_sdk::invocation::ability_ura_from_descriptor_ref(
        &receipt.ability_binding,
    )
    .map_err(|error| {
        anyhow::anyhow!("{ability}: {stage} receipt ability binding is invalid: {error}")
    })?;
    let public_name = axon_sdk::ura::qualified_ability_name(ability_ura).ok_or_else(|| {
        anyhow::anyhow!("{ability}: {stage} receipt ability binding has no public ability name")
    })?;
    if public_name != submitted.function_name {
        bail!("{ability}: {stage} receipt ability does not match submitted function_name");
    }
    if receipt.input_hash != submitted.input_hash {
        bail!("{ability}: {stage} receipt input_hash does not bind submitted arguments");
    }
    if receipt.authority_binding.is_none()
        || receipt.subject_ref.is_none()
        || receipt.descriptor_version.trim().is_empty()
        || receipt.schema_hash.len() != 32
        || receipt.schema_hash.iter().all(|byte| *byte == 0)
        || receipt.impl_hash.len() != 32
        || receipt.impl_hash.iter().all(|byte| *byte == 0)
        || receipt.runtime_env.trim().is_empty()
    {
        bail!("{ability}: {stage} receipt omitted required descriptor-bound proof facts");
    }
    let expected_parents = submitted_parent_receipts(envelope.causal_context.as_ref(), ability)?;
    if receipt.parent_receipts != expected_parents {
        bail!("{ability}: {stage} receipt parent list does not bind causal_context");
    }
    Ok(())
}

#[cfg(feature = "axon-pb")]
fn submitted_parent_receipts(
    causal: Option<&axon_sdk::pb::axon::v1::CausalContext>,
    ability: &str,
) -> anyhow::Result<Vec<axon_sdk::pb::axon::v1::ReceiptRef>> {
    use anyhow::bail;
    use axon_sdk::pb::axon::v1::causal_context::Form;

    Ok(match causal.and_then(|context| context.form.as_ref()) {
        None | Some(Form::None(_)) => Vec::new(),
        Some(Form::Scalar(reference)) => vec![reference.clone()],
        Some(Form::List(list)) => list.prior.clone(),
        Some(Form::Merkle(_)) => {
            bail!("{ability}: local mission invocation does not support Merkle causal parents")
        }
    })
}

#[cfg(feature = "axon-pb")]
fn validate_receipt_chain_binding(
    admission: &axon_sdk::pb::axon::v1::InvocationReceipt,
    terminal: &axon_sdk::pb::axon::v1::InvocationReceipt,
    ability: &str,
) -> anyhow::Result<()> {
    use anyhow::bail;

    if admission.caller_binding != terminal.caller_binding
        || admission.callee_binding != terminal.callee_binding
        || admission.subject_binding != terminal.subject_binding
        || admission.invocation_nonce != terminal.invocation_nonce
        || admission.causal_binding != terminal.causal_binding
        || admission.signer_binding != terminal.signer_binding
        || admission.host_attestation != terminal.host_attestation
        || admission.authority_binding != terminal.authority_binding
        || admission.ability_binding != terminal.ability_binding
        || admission.subject_ref != terminal.subject_ref
        || admission.descriptor_version != terminal.descriptor_version
        || admission.schema_hash != terminal.schema_hash
        || admission.impl_hash != terminal.impl_hash
        || admission.runtime_env != terminal.runtime_env
        || admission.authority_proof != terminal.authority_proof
        || admission.input_hash != terminal.input_hash
        || admission.parent_receipts != terminal.parent_receipts
    {
        bail!("{ability}: terminal receipt changed admission-bound proof facts");
    }
    Ok(())
}

#[cfg(feature = "axon-pb")]
type LocalInvocationGrpcClient =
    axon_sdk::pb::axon::v1::invocation_client::InvocationClient<tonic::transport::Channel>;

#[cfg(feature = "axon-pb")]
async fn invoke_local_daemon_json(
    client: &mut LocalInvocationGrpcClient,
    request: axon_sdk::pb::axon::v1::InvokeRequest,
    function_name: &str,
) -> anyhow::Result<(serde_json::Value, axon_sdk::pb::axon::v1::InvokeResponse)> {
    use anyhow::Context;
    use serde_json::Value;

    let response = client
        .invoke(request)
        .await
        .map_err(|status| local_daemon_status_error(function_name, status))?;
    let body = response.into_inner();
    require_completed_invoke_response(&body, function_name)?;
    let value = if body.result.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body.result)
            .with_context(|| format!("decode {function_name} Axon response"))?
    };
    Ok((value, body))
}

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_with_invocation_meta_inner(
    request: LocalDaemonTargetedInvocationMetaRequest<'_>,
    delegation: Option<HostedAgentDelegationRequest>,
) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
    use anyhow::{anyhow, bail, Context};
    use axon_sdk::pb::axon::v1 as pb;
    use serde_json::Value;

    let LocalDaemonTargetedInvocationMetaRequest {
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        invocation_nonce,
        causal_parents,
        step_timeout,
        trace_id,
    } = request;
    let function_name = function_name.trim().to_string();
    if function_name.is_empty() {
        bail!("function_name must not be empty");
    }

    let socket_path = resolve_socket_path();
    if !probe_accepting(&socket_path) {
        bail!(
            "daemon not running (local Axon gRPC listener unreachable at {}). \
             Start it with `easynet runtime start`.",
            socket_path.display()
        );
    }

    let receipt_refs = verified_receipt_refs_from_causal_parents(causal_parents)?;
    let mut refs = receipt_refs;
    let causal_form = match refs.len() {
        0 => pb::causal_context::Form::None(pb::Empty {}),
        1 => pb::causal_context::Form::Scalar(refs.remove(0)),
        _ => pb::causal_context::Form::List(pb::ReceiptList { prior: refs }),
    };
    let tuple_plan = LocalDaemonLoopbackTuplePlan::targeted_explicit_causal(
        &function_name,
        payload_json,
        callee_ura,
        subject_ura,
        invocation_nonce,
        pb::CausalContext {
            form: Some(causal_form),
        },
        step_timeout,
    )
    .map_err(|error| anyhow!("{function_name}: {error}"))?;
    let submitted_callee_ura = tuple_plan.callee_policy.resolve()?;
    let submitted_subject_ura = tuple_plan.subject_policy.resolve(&submitted_callee_ura)?;
    let invocation =
        local_daemon_loopback_invocation_from_tuple_plan(tuple_plan)?.with_trace_id(trace_id);
    let mut request = invocation.invoke_request()?;
    let wire_caller_ura = invocation.caller_ura().to_string();
    let nonce_hex = request
        .envelope
        .as_ref()
        .map(|env| hex::encode(env.invocation_nonce.as_slice()))
        .ok_or_else(|| anyhow!("build {function_name} request without envelope"))?;
    if let Some(delegation) = delegation.as_ref() {
        request.metadata.insert(
            HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY.to_string(),
            delegation.metadata_value()?,
        );
    }
    let submitted = SubmittedInvocationProjection::from_request(&request, &function_name)?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for local Axon daemon invoke")?;

    // The channel timeout is per-request: it must cover the step's own
    // execution budget (the daemon-side executor enforces the manifest
    // timeout) plus admission/ledger overhead, or a slow-but-legitimate
    // step gets cut off at the transport layer instead of by its
    // declared deadline.
    let request_timeout = step_timeout
        .checked_add(Duration::from_secs(30))
        .ok_or_else(|| anyhow!("{function_name}: transport timeout overflow"))?;
    let invoke_socket = socket_path.clone();
    let invoke_fn = function_name.clone();
    let (result_value, response) = runtime.block_on(async move {
        let channel = connect_channel(
            invoke_socket.clone(),
            request_timeout,
            Duration::from_secs(10),
        )
        .await
        .map_err(|source| local_daemon_connect_error(&invoke_socket, source))?;
        let mut client = axon_sdk::pb::axon::v1::invocation_client::InvocationClient::new(channel);
        invoke_local_daemon_json(&mut client, request, &invoke_fn).await
    })?;
    let request_id = response
        .header
        .as_ref()
        .map(|header| header.request_id.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow!("daemon response invoking {function_name} did not include request_id")
        })?
        .to_string();
    let terminal = UnverifiedTerminalInvocationProjection::from_response(
        &response,
        &submitted,
        &function_name,
    )?
    .verify(&LocalKeyServiceReceiptResolver::new(), &function_name)?;
    if request_id != terminal.invocation_id {
        bail!(
            "daemon response invoking {function_name} returned request_id {request_id:?} \
             but terminal receipt belongs to {:?}",
            terminal.invocation_id
        );
    }
    record_verified_causal_anchor(&terminal.causal_anchor)?;

    // Identity fields come from the signed terminal receipt, not from the
    // submitted request and not from an asynchronously persisted product
    // ledger projection. This makes the causal anchor available atomically
    // with the terminal response and keeps Axon as the protocol truth owner.
    let mut meta = serde_json::json!({
        "request_id": request_id,
        "trace_id": trace_id,
        "invocation_ura": terminal.invocation_ura,
        "caller_ura": terminal.caller_ura,
        "callee_ura": terminal.callee_ura,
        "subject_ura": terminal.subject_ura,
        "submitted_caller_ura": wire_caller_ura,
        "submitted_callee_ura": submitted_callee_ura,
        "submitted_subject_ura": submitted_subject_ura,
        "ability": function_name,
        "nonce": nonce_hex,
        "causal_context": { "parents": causal_parents },
        "receipt": terminal.receipt,
        "metadata_state": "finalization_checkpoints_verified",
        "ledger_state": terminal.state,
    });
    if let Some(delegation) = delegation {
        meta["delegation"] = serde_json::json!({
            "kind": "hosted_agent",
            "agent_ura": delegation.agent_ura(),
            "signing_authority": "host_device",
            "wire_caller_ura": meta.get("caller_ura").cloned().unwrap_or(Value::Null),
            "wire_callee_ura": meta.get("callee_ura").cloned().unwrap_or(Value::Null),
        });
    }
    Ok((result_value, meta))
}

#[cfg(feature = "axon-pb")]
fn verified_receipt_refs_from_causal_parents(
    causal_parents: &[serde_json::Value],
) -> anyhow::Result<Vec<axon_sdk::pb::axon::v1::ReceiptRef>> {
    use anyhow::Context;
    use serde_json::Value;

    let registry = verified_causal_anchor_registry()
        .lock()
        .map_err(|_| anyhow::anyhow!("verified causal anchor registry is poisoned"))?;
    let mut refs = Vec::with_capacity(causal_parents.len());
    for (idx, parent) in causal_parents.iter().enumerate() {
        let receipt_ura = parent
            .get("receipt_ura")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("causal parent #{idx} is missing receipt_ura"))?;
        let hash_hex = parent
            .get("receipt_hash")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("causal parent #{idx} is missing receipt_hash"))?;
        let receipt_hash = hex::decode(hash_hex)
            .with_context(|| format!("decode causal parent #{idx} receipt_hash as hex"))?;
        let receipt_hash: [u8; 32] = receipt_hash.try_into().map_err(|bytes: Vec<u8>| {
            anyhow::anyhow!(
                "causal parent #{idx} receipt_hash must decode to 32 bytes, got {}",
                bytes.len()
            )
        })?;
        let claim = CausalAnchorKey {
            receipt_ura: receipt_ura.to_string(),
            receipt_hash,
        };
        let verified = registry.restore(&claim).ok_or_else(|| {
            anyhow::anyhow!(
                "causal parent #{idx} was not cryptographically verified in this process"
            )
        })?;
        refs.push(verified.to_wire());
    }
    Ok(refs)
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_targeted_with_invocation_meta(
    request: LocalDaemonTargetedInvocationMetaRequest<'_>,
) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
    let LocalDaemonTargetedInvocationMetaRequest {
        function_name,
        payload_json: _,
        callee_ura: _,
        subject_ura: _,
        invocation_nonce: _,
        causal_parents: _,
        step_timeout: _,
        trace_id: _,
    } = request;
    anyhow::bail!(
        "invoking targeted `{}` with invocation metadata requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_targeted_with_hosted_agent_delegation(
    request: LocalDaemonTargetedInvocationMetaRequest<'_>,
    _hosted_agent_ura: &str,
) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
    let function_name = request.function_name;
    anyhow::bail!(
        "invoking `{}` with invocation metadata requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability(
    function_name: &str,
    _payload_json: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    Err(anyhow::Error::new(
        crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
            "invoking `{function_name}` through the local Axon daemon requires the `axon-pb` \
             feature; rebuild with `cargo build --features axon-pb`"
        )),
    ))
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_system_ability_root_for_subject_timeout(
    function_name: &str,
    _payload_json: serde_json::Value,
    _subject_ura: &str,
    _timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    Err(anyhow::Error::new(
        crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
            "invoking daemon-system `{function_name}` through the local Axon daemon requires the \
             `axon-pb` feature; rebuild with `cargo build --features axon-pb`"
        )),
    ))
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_system_ability_targeted_root_timeout(
    function_name: &str,
    payload_json: serde_json::Value,
    _callee_ura: &str,
    _subject_ura: &str,
    _timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    Err(anyhow::Error::new(
        crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
            "invoking targeted `{function_name}` through the local Axon daemon requires the \
             `axon-pb` feature; rebuild with `cargo build --features axon-pb`"
        )),
    ))
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_targeted_explicit_root_timeout(
    function_name: &str,
    _payload_json: serde_json::Value,
    _callee_ura: &str,
    _subject_ura: &str,
    _invocation_nonce: [u8; 16],
    _timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    Err(anyhow::Error::new(
        crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
            "invoking explicit-root `{function_name}` through the local Axon daemon requires the \
             `axon-pb` feature; rebuild with `cargo build --features axon-pb`"
        )),
    ))
}

#[cfg(feature = "axon-pb")]
fn local_daemon_loopback_caller_ura() -> anyhow::Result<String> {
    Ok(crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA.to_string())
}

#[cfg(feature = "axon-pb")]
fn local_daemon_default_callee_ura() -> anyhow::Result<String> {
    crate::daemon::identity::local_invocation::local_daemon_ura()
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;

    use axon_sdk::invocation::axiom::authority_proof_expected_hash;
    use axon_sdk::invocation::{
        AgentIdentity, AuthorityBinding, AxonError, CalleeSignature, CanonicalReceiptProvider,
        DescriptorBoundEnvelope, InvocationAuthorityProof, ReceiptSigningAuthority, UraProfile,
        VerifiedAdmissionPolicy,
    };
    use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
    use std::sync::Arc;

    #[test]
    fn loopback_invoke_request_does_not_pre_resolve_descriptor_ref() {
        let tuple_plan = LocalDaemonLoopbackTuplePlan::new(
            "discover",
            serde_json::json!({"query": "capabilities"}),
            LocalDaemonLoopbackCalleePolicy::explicit("easynet:///r/default/agent/dev.worker")
                .expect("callee policy"),
            LocalDaemonLoopbackSubjectPolicy::local_daemon_self(),
            LocalDaemonLoopbackDerivationPolicy::fresh_root(),
            Duration::from_secs(5),
        )
        .expect("tuple plan");
        let invocation = local_daemon_loopback_invocation_from_tuple_plan(tuple_plan)
            .expect("loopback invocation projection");
        let request = invocation.invoke_request().expect("loopback request");

        assert_eq!(
            crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
                "test loopback request",
                request.target.as_ref(),
            )
            .unwrap(),
            "discover"
        );
        let target = request.target.as_ref().expect("local typed target");
        let axon_sdk::pb::axon::v1::invocation_target::TypedTarget::Ability(ability) =
            target.typed_target.as_ref().expect("typed ability target");
        assert_eq!(ability.ability_name, "discover");
        assert_eq!(ability.function_name, "discover");
        let envelope = request.envelope.as_ref().expect("request envelope");
        assert_eq!(
            envelope.caller.as_ref().map(|caller| caller.ura.as_str()),
            Some(crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA)
        );
        assert!(
            envelope.caller_signature.is_none(),
            "daemon loopback requests are promoted to LocalSystem inside the daemon dispatcher"
        );
        assert_eq!(
            envelope.callee.as_ref().map(|callee| callee.ura.as_str()),
            Some("easynet:///r/default/agent/dev.worker")
        );
        assert_eq!(
            envelope
                .subject
                .as_ref()
                .map(|subject| subject.ura.as_str()),
            Some("easynet:///r/default/agent/dev.worker")
        );
    }

    #[test]
    fn loopback_tuple_plan_requires_explicit_targeted_subject() {
        let tuple_plan = LocalDaemonLoopbackTuplePlan::targeted_root_for_subject(
            "job.run",
            serde_json::json!({"job": 1}),
            "easynet:///r/acme/device/edge-1",
            "easynet:///r/acme/resource/user.jobs/job-1",
            Duration::from_secs(5),
        )
        .expect("tuple plan");
        assert_eq!(
            tuple_plan.subject_policy,
            LocalDaemonLoopbackSubjectPolicy::Explicit(
                "easynet:///r/acme/resource/user.jobs/job-1".to_string()
            )
        );

        let invocation = local_daemon_loopback_invocation_from_tuple_plan(tuple_plan)
            .expect("loopback invocation");
        let request = invocation.invoke_request().expect("invoke request");
        let envelope = request.envelope.as_ref().expect("request envelope");
        assert_eq!(
            envelope.callee.as_ref().map(|callee| callee.ura.as_str()),
            Some("easynet:///r/acme/device/edge-1")
        );
        assert_eq!(
            envelope
                .subject
                .as_ref()
                .map(|subject| subject.ura.as_str()),
            Some("easynet:///r/acme/resource/user.jobs/job-1")
        );

        assert!(LocalDaemonLoopbackTuplePlan::targeted_root_for_subject(
            "job.run",
            serde_json::json!({"job": 1}),
            "easynet:///r/acme/device/edge-1",
            "",
            Duration::from_secs(5),
        )
        .is_err());
        assert!(LocalDaemonLoopbackTuplePlan::local_root(
            "job.run",
            serde_json::json!({"job": 1}),
            Duration::ZERO,
        )
        .is_err());
        assert!(LocalDaemonLoopbackTuplePlan::local_root_for_subject(
            "job.run",
            serde_json::json!({"job": 1}),
            "",
            Duration::from_secs(5),
        )
        .is_err());
    }

    #[test]
    fn loopback_tuple_plan_preserves_explicit_causal_context() {
        let parent = axon_sdk::pb::axon::v1::ReceiptRef {
            receipt_ura: "easynet:///r/acme/resource/user.jobs/job-1/invocation/i1/receipt/1"
                .to_string(),
            receipt_hash: [7_u8; 32].to_vec(),
        };
        let tuple_plan = LocalDaemonLoopbackTuplePlan::targeted_explicit_causal(
            "job.run",
            serde_json::json!({"job": 1}),
            "easynet:///r/acme/device/edge-1",
            "easynet:///r/acme/resource/user.jobs/job-1",
            [9_u8; 16],
            axon_sdk::pb::axon::v1::CausalContext {
                form: Some(axon_sdk::pb::axon::v1::causal_context::Form::Scalar(
                    parent.clone(),
                )),
            },
            Duration::from_secs(5),
        )
        .expect("tuple plan");

        let LocalDaemonLoopbackDerivationPolicy::ExplicitCausal {
            invocation_nonce, ..
        } = &tuple_plan.derivation_policy
        else {
            panic!("expected explicit causal derivation");
        };
        assert_eq!(*invocation_nonce, [9_u8; 16]);

        let invocation = local_daemon_loopback_invocation_from_tuple_plan(tuple_plan)
            .expect("loopback invocation");
        let request = invocation.invoke_request().expect("invoke request");
        let envelope = request.envelope.as_ref().expect("request envelope");
        assert_eq!(envelope.invocation_nonce, [9_u8; 16].to_vec());
        assert_eq!(
            envelope
                .causal_context
                .as_ref()
                .and_then(|context| context.form.as_ref()),
            Some(&axon_sdk::pb::axon::v1::causal_context::Form::Scalar(
                parent
            ))
        );

        assert!(LocalDaemonLoopbackTuplePlan::targeted_explicit_causal(
            "job.run",
            serde_json::json!({"job": 1}),
            "easynet:///r/acme/device/edge-1",
            "easynet:///r/acme/resource/user.jobs/job-1",
            [0_u8; 16],
            axon_sdk::pb::axon::v1::CausalContext {
                form: Some(axon_sdk::pb::axon::v1::causal_context::Form::None(
                    axon_sdk::pb::axon::v1::Empty {},
                )),
            },
            Duration::from_secs(5),
        )
        .is_err());
    }

    fn completed_receipt_response_fixture(
        seed: u8,
        _invocation_id: &str,
    ) -> (
        SubmittedInvocationProjection,
        axon_sdk::pb::axon::v1::InvokeResponse,
        ed25519_dalek::SigningKey,
    ) {
        use axon_sdk::invocation::{
            make_ability, AbilityCallModes, AbilityOptions, InvocationState,
        };
        use axon_sdk::pb::axon::v1::InvokeResponse;

        let tuple_plan = LocalDaemonLoopbackTuplePlan::targeted_root_for_subject(
            "job.run",
            serde_json::json!({"job": 1}),
            "easynet:///r/acme/device/edge-1",
            "easynet:///r/acme/resource/user.jobs/job-1",
            Duration::from_secs(5),
        )
        .expect("tuple plan");
        let invocation = local_daemon_loopback_invocation_from_tuple_plan(tuple_plan)
            .expect("loopback invocation");
        let request = invocation.invoke_request().expect("invoke request");
        let submitted =
            SubmittedInvocationProjection::from_request(&request, "job.run").expect("submitted");
        let envelope = request.envelope.clone().expect("envelope");
        let callee_wire = envelope.callee.as_ref().expect("callee");
        let callee = AgentIdentity::new(
            callee_wire.ura.clone(),
            UraProfile::parse(&callee_wire.profile).expect("callee profile"),
        );
        let signing_key = SigningKey::from_bytes(&[seed; 32]);

        let runtime =
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime_with_receipt_provider(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                Arc::new(FixtureCanonicalReceiptProvider::new(
                    callee.clone(),
                    signing_key.clone(),
                )),
            );
        let ability_ura = crate::daemon::axon_bridge::descriptor_ref::ability_ura_for_wire(
            &callee.ura,
            "job.run",
        )
        .expect("ability URA");
        let descriptor_binding =
            crate::daemon::axon_bridge::descriptor_ref::descriptor_binding_for_wire(
                crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                [3; 32],
                "invoke",
            )
            .expect("descriptor binding");
        let descriptor_ref =
            crate::daemon::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
                &callee.ura,
                "job.run",
                &descriptor_binding,
            )
            .expect("descriptor ref");
        let result = br#"{"ok":true}"#.to_vec();
        let dispatch = crate::daemon::axon_bridge::dispatch_shim::local_system_from_wire_parts(
            envelope,
            descriptor_ref,
            request.arguments.clone(),
            Default::default(),
        )
        .expect("local-system descriptor-bound dispatch");
        let response = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fixture runtime")
            .block_on(async {
                runtime
                    .register_ability_with_options(
                        ability_ura,
                        make_ability({
                            let result = result.clone();
                            move |_| {
                                let result = result.clone();
                                async move { Ok(result) }
                            }
                        }),
                        AbilityOptions::default()
                            .with_modes(AbilityCallModes::RPC)
                            .with_descriptor_proof(
                                crate::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION,
                                "invoke",
                                [3; 32],
                                [1; 32],
                                [2; 32],
                            ),
                    )
                    .await
                    .expect("register fixture ability");
                let outcome = crate::daemon::axon_bridge::dispatch_shim::dispatch_rpc_admitted(
                    &runtime,
                    dispatch,
                    &Default::default(),
                )
                .await;
                assert_eq!(outcome.state, InvocationState::Completed);
                assert!(outcome.error.is_none(), "{:?}", outcome.error);
                InvokeResponse {
                    state: InvocationState::Completed.to_wire_i32(),
                    result: outcome.payload_bytes,
                    admission_receipt: Some(
                        axon_sdk::invocation::wire::receipt_to_wire(
                            &outcome.admission_receipt.expect("admission receipt"),
                        )
                        .expect("signed admission fixture projects to wire"),
                    ),
                    terminal_receipt: Some(
                        axon_sdk::invocation::wire::receipt_to_wire(
                            &outcome.terminal_receipt.expect("terminal receipt"),
                        )
                        .expect("signed terminal fixture projects to wire"),
                    ),
                    ..Default::default()
                }
            });
        (submitted, response, signing_key)
    }

    struct FixtureCanonicalReceiptProvider {
        authority: Arc<dyn ReceiptSigningAuthority>,
        signer_ura: String,
        verifying_key: VerifyingKey,
    }

    impl FixtureCanonicalReceiptProvider {
        fn new(callee: AgentIdentity, signing_key: SigningKey) -> Self {
            let authority = Arc::new(FixtureReceiptSigningAuthority::self_signed(
                callee.clone(),
                signing_key,
            ));
            Self {
                signer_ura: callee.ura,
                verifying_key: authority.verifying_key(),
                authority,
            }
        }
    }

    #[async_trait::async_trait]
    impl CanonicalReceiptProvider for FixtureCanonicalReceiptProvider {
        fn verify_admission_policy(
            &self,
            envelope: &DescriptorBoundEnvelope,
        ) -> Result<VerifiedAdmissionPolicy, AxonError> {
            let binding = AuthorityBinding::Self_ {
                principal_ura: envelope.envelope().caller.ura.clone(),
            };
            let mut proof = InvocationAuthorityProof::new(
                "local-daemon-grpc-fixture-verified-admission",
                Some(binding.clone()),
                Vec::new(),
                [0u8; 32],
                Some(envelope.envelope().callee.clone()),
                None,
                "easynet-cli.local-daemon-grpc-fixture.canonical_receipt_provider.admission.v1",
            );
            proof.proof_hash = authority_proof_expected_hash(&proof);
            VerifiedAdmissionPolicy::new(envelope, binding, proof)
        }

        async fn resolve_signing_authority(
            &self,
            callee: &AgentIdentity,
        ) -> Result<Arc<dyn ReceiptSigningAuthority>, AxonError> {
            if callee.ura != self.signer_ura {
                return Err(AxonError::permission_denied(
                    "local_daemon_grpc_fixture_callee_mismatch",
                ));
            }
            Ok(Arc::clone(&self.authority))
        }

        fn resolve_signer_key(&self, signer_ura: &str) -> Result<Option<VerifyingKey>, AxonError> {
            Ok((signer_ura == self.signer_ura).then_some(self.verifying_key))
        }
    }

    struct FixtureReceiptSigningAuthority {
        callee_identity: AgentIdentity,
        signing_key: SigningKey,
    }

    impl FixtureReceiptSigningAuthority {
        fn self_signed(callee_identity: AgentIdentity, signing_key: SigningKey) -> Self {
            Self {
                callee_identity,
                signing_key,
            }
        }

        fn verifying_key(&self) -> VerifyingKey {
            self.signing_key.verifying_key()
        }
    }

    #[async_trait::async_trait]
    impl ReceiptSigningAuthority for FixtureReceiptSigningAuthority {
        fn callee_identity(&self) -> &AgentIdentity {
            &self.callee_identity
        }

        fn signer_identity(&self) -> &AgentIdentity {
            &self.callee_identity
        }

        fn host_attestation(&self) -> &[u8] {
            &[]
        }

        fn verifying_key(&self) -> VerifyingKey {
            self.verifying_key()
        }

        async fn sign_and_verify(
            &self,
            canonical_receipt: &[u8],
        ) -> Result<CalleeSignature, AxonError> {
            let signature: Signature = self.signing_key.sign(canonical_receipt);
            self.verifying_key()
                .verify(canonical_receipt, &signature)
                .map_err(|_| {
                    AxonError::internal(
                        "local_daemon_grpc_fixture_receipt_signature_self_verify_failed",
                    )
                })?;
            Ok(CalleeSignature {
                algorithm: "ed25519".to_string(),
                signature: signature.to_bytes().to_vec(),
                key_id_hint: "local-daemon-grpc-fixture".to_string(),
            })
        }
    }

    struct FixedReceiptKeyResolver {
        signer_ura: String,
        key: ed25519_dalek::VerifyingKey,
    }

    impl axon_sdk::invocation::KeyResolver for FixedReceiptKeyResolver {
        fn resolve(
            &self,
            signer_ura: &str,
        ) -> Result<ed25519_dalek::VerifyingKey, axon_sdk::invocation::AxonError> {
            if signer_ura == self.signer_ura {
                return Ok(self.key);
            }
            Err(axon_sdk::invocation::AxonError::permission_denied(
                "test_receipt_key_unknown",
            ))
        }
    }

    struct UnknownReceiptKeyResolver;

    impl axon_sdk::invocation::KeyResolver for UnknownReceiptKeyResolver {
        fn resolve(
            &self,
            _signer_ura: &str,
        ) -> Result<ed25519_dalek::VerifyingKey, axon_sdk::invocation::AxonError> {
            Err(axon_sdk::invocation::AxonError::permission_denied(
                "test_receipt_key_unknown",
            ))
        }
    }

    fn fixture_resolver(
        response: &axon_sdk::pb::axon::v1::InvokeResponse,
        signing_key: &ed25519_dalek::SigningKey,
    ) -> FixedReceiptKeyResolver {
        let signer_ura = response
            .terminal_receipt
            .as_ref()
            .and_then(|receipt| receipt.callee_binding.as_ref())
            .expect("fixture callee")
            .ura
            .clone();
        FixedReceiptKeyResolver {
            signer_ura,
            key: signing_key.verifying_key(),
        }
    }

    fn causal_parent_claim(response: &axon_sdk::pb::axon::v1::InvokeResponse) -> serde_json::Value {
        let terminal = response
            .terminal_receipt
            .as_ref()
            .expect("terminal receipt");
        let caller_ura = &terminal.caller_binding.as_ref().expect("caller").ura;
        let callee_ura = &terminal.callee_binding.as_ref().expect("callee").ura;
        let subject_ura = &terminal.subject_binding.as_ref().expect("subject").ura;
        let invocation_ura = axon_sdk::ura::invocation_record_ura_for_binding(
            subject_ura,
            callee_ura,
            caller_ura,
            &terminal.invocation_id,
        )
        .expect("invocation URA");
        serde_json::json!({
            "receipt_ura": format!(
                "{}/receipt/{}",
                invocation_ura.trim_end_matches('/'),
                terminal.index
            ),
            "receipt_hash": hex::encode(&terminal.self_hash),
        })
    }

    #[test]
    fn verified_finalization_projection_is_the_only_causal_anchor_source() {
        let (submitted, response, signing_key) =
            completed_receipt_response_fixture(0x31, "inv-verified-chain");
        let terminal = response
            .terminal_receipt
            .as_ref()
            .expect("terminal receipt");
        let expected_hash = hex::encode(&terminal.self_hash);
        let expected_anchor_count = terminal.index + 1;
        let expected_anchor_suffix = format!("/receipt/{}", terminal.index);
        let resolver = fixture_resolver(&response, &signing_key);

        let projection =
            UnverifiedTerminalInvocationProjection::from_response(&response, &submitted, "job.run")
                .expect("well-formed unverified projection")
                .verify(&resolver, "job.run")
                .expect("cryptographically verified finalization projection");
        record_verified_causal_anchor(&projection.causal_anchor)
            .expect("record verified causal capability");
        assert_eq!(projection.state, "completed");
        assert_eq!(
            projection.receipt["anchor_count"],
            serde_json::json!(expected_anchor_count)
        );
        assert_eq!(
            projection.receipt["anchor"]["receipt_hash"],
            serde_json::json!(expected_hash)
        );
        assert!(projection.receipt["anchor"]["receipt_ura"]
            .as_str()
            .expect("receipt URA")
            .ends_with(&expected_anchor_suffix));
        assert_eq!(
            projection.receipt["cryptographic_verification"],
            "finalization_checkpoints_verified"
        );

        let parent = serde_json::json!({
            "receipt_ura": projection.receipt["anchor"]["receipt_ura"],
            "receipt_hash": projection.receipt["anchor"]["receipt_hash"],
        });
        let refs = verified_receipt_refs_from_causal_parents(&[parent])
            .expect("verified projection restores a causal receipt capability");
        assert_eq!(refs.len(), 1);
        assert_eq!(
            refs[0].receipt_hash,
            response
                .terminal_receipt
                .as_ref()
                .expect("terminal receipt")
                .self_hash
        );
    }

    #[test]
    fn tampered_terminal_receipt_never_becomes_a_verified_projection() {
        let (submitted, mut response, signing_key) =
            completed_receipt_response_fixture(0x32, "inv-tampered-terminal");
        response
            .terminal_receipt
            .as_mut()
            .and_then(|receipt| receipt.callee_signature.as_mut())
            .expect("terminal signature")
            .signature[0] ^= 1;
        let resolver = fixture_resolver(&response, &signing_key);

        let unverified =
            UnverifiedTerminalInvocationProjection::from_response(&response, &submitted, "job.run")
                .expect("signature tamper retains structural shape");
        let error = unverified
            .verify(&resolver, "job.run")
            .expect_err("tampered signature must fail closed");
        assert!(error
            .to_string()
            .contains("terminal receipt signature is invalid"));
    }

    #[test]
    fn unknown_receipt_key_cannot_authorize_a_causal_parent() {
        let (submitted, response, _signing_key) =
            completed_receipt_response_fixture(0x33, "inv-unknown-receipt-key");
        let parent = causal_parent_claim(&response);

        let unverified =
            UnverifiedTerminalInvocationProjection::from_response(&response, &submitted, "job.run")
                .expect("well-formed unverified projection");
        let error = unverified
            .verify(&UnknownReceiptKeyResolver, "job.run")
            .expect_err("unknown signer key must fail closed");
        let axon_error = error
            .chain()
            .find_map(|cause| cause.downcast_ref::<axon_sdk::invocation::AxonError>())
            .expect("verification error preserves the source AxonError");
        assert_eq!(axon_error.reason, "test_receipt_key_unknown");

        let error = verified_receipt_refs_from_causal_parents(&[parent])
            .expect_err("unverified anchor must not enter causal context");
        assert!(error
            .to_string()
            .contains("was not cryptographically verified"));
    }

    #[test]
    fn terminal_invocation_projection_rejects_response_receipt_state_mismatch() {
        use axon_sdk::invocation::InvocationState;
        let (submitted, mut response, _signing_key) =
            completed_receipt_response_fixture(0x34, "inv-state-mismatch");
        let terminal = response
            .terminal_receipt
            .as_mut()
            .expect("terminal receipt");
        terminal.receipt_type = "failed".to_string();
        terminal.state = InvocationState::Failed.to_wire_i32();

        let error =
            UnverifiedTerminalInvocationProjection::from_response(&response, &submitted, "job.run")
                .expect_err("mismatched response and receipt state must fail closed");
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn terminal_invocation_projection_rejects_failed_terminal_response() {
        use axon_sdk::invocation::InvocationState;

        let (submitted, mut response, _signing_key) =
            completed_receipt_response_fixture(0x35, "inv-failed-response");
        response.state = InvocationState::Failed.to_wire_i32();

        let error =
            UnverifiedTerminalInvocationProjection::from_response(&response, &submitted, "job.run")
                .expect_err("failed terminal state must not become a successful product result");
        assert!(error.to_string().contains("ended in Failed"));
    }

    #[test]
    fn canonical_unary_projection_preserves_in_band_terminal_failure() {
        use axon_sdk::invocation::InvocationState;
        use axon_sdk::pb::axon::v1::Error;

        let response = axon_sdk::pb::axon::v1::InvokeResponse {
            state: InvocationState::Failed.to_wire_i32(),
            error: Some(Error {
                code: "PERMISSION_DENIED".to_string(),
                message: "recovery proof reference has already been consumed".to_string(),
                ..Error::default()
            }),
            ..Default::default()
        };

        let error = require_completed_invoke_response(&response, "principal.lifecycle.recover")
            .expect_err("terminal failure must not decode as an empty successful payload");
        let message = error.to_string();
        assert!(message.contains("ended in Failed"), "{message}");
        assert!(message.contains("PERMISSION_DENIED"), "{message}");
        assert!(message.contains("already been consumed"), "{message}");
    }

    #[test]
    fn canonical_unary_projection_rejects_completed_protocol_error() {
        use axon_sdk::invocation::InvocationState;
        use axon_sdk::pb::axon::v1::Error;

        let response = axon_sdk::pb::axon::v1::InvokeResponse {
            state: InvocationState::Completed.to_wire_i32(),
            error: Some(Error {
                code: "INTERNAL".to_string(),
                message: "contradictory response".to_string(),
                ..Error::default()
            }),
            ..Default::default()
        };

        let error = require_completed_invoke_response(&response, "observe.health")
            .expect_err("completed response cannot carry a protocol error");
        assert!(error
            .to_string()
            .contains("completed response carried a protocol error"));
    }
}
