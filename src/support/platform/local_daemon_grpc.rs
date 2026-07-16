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
    HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY, HostedAgentDelegationRequest,
};
#[cfg(feature = "axon-pb")]
use crate::daemon::persistence::agent_aggregate::{
    AgentAggregateRepository, HostedAgentNameLookupError,
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

/// Small value object for invoking daemon-hosted Axon abilities over
/// the local Invocation transport. CLI modules should depend on this
/// client instead of rebuilding socket/protobuf/tonic plumbing.
#[derive(Debug, Clone)]
pub(crate) struct LocalDaemonAbilityClient;

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalDaemonSubjectPolicy {
    Explicit(String),
    DeclaredDefault(String),
    SelfTarget,
}

pub(crate) struct LocalDaemonTargetedBidiRequest<'a> {
    pub function_name: &'a str,
    pub payload_json: serde_json::Value,
    pub callee_ura: &'a str,
    pub default_subject_ura: &'a str,
    pub subject: Option<String>,
    pub timeout: Duration,
    pub input_frames: Vec<serde_json::Value>,
    pub max_frames: Option<usize>,
}

#[cfg(feature = "axon-pb")]
impl LocalDaemonSubjectPolicy {
    fn explicit_or_self_target(subject: Option<String>) -> anyhow::Result<Self> {
        match Self::explicit(subject)? {
            Some(subject) => Ok(subject),
            None => Ok(Self::SelfTarget),
        }
    }

    fn explicit_or_declared_default(
        default_subject_ura: &str,
        subject: Option<String>,
    ) -> anyhow::Result<Self> {
        match Self::explicit(subject)? {
            Some(subject) => Ok(subject),
            None => Ok(Self::DeclaredDefault(Self::normalized_ura(
                default_subject_ura,
                "default_subject_ura",
            )?)),
        }
    }

    fn resolve(&self, callee_ura: &str) -> anyhow::Result<String> {
        match self {
            Self::Explicit(subject) | Self::DeclaredDefault(subject) => Ok(subject.clone()),
            Self::SelfTarget => Self::normalized_ura(callee_ura, "callee_ura"),
        }
    }

    fn explicit(subject: Option<String>) -> anyhow::Result<Option<Self>> {
        subject
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|subject| Self::normalized_ura(subject, "subject_ura").map(Self::Explicit))
            .transpose()
    }

    fn normalized_ura(value: &str, field: &str) -> anyhow::Result<String> {
        let value = value.trim();
        if value.is_empty() {
            anyhow::bail!("{field} must not be empty");
        }
        crate::core::ura::parse_ura(value)
            .map_err(|err| anyhow::anyhow!("{field} is not a valid URA: {err}"))?;
        Ok(value.to_string())
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
struct LocalDaemonLoopbackInvocation {
    function_name: String,
    caller_ura: String,
    callee_ura: String,
    subject_ura: String,
    arguments: Vec<u8>,
    timeout: Duration,
    causal_context: Option<easynet_axon::pb::axon::v1::CausalContext>,
    trace_id: Option<String>,
}

#[cfg(feature = "axon-pb")]
impl LocalDaemonLoopbackInvocation {
    fn from_subject_policy(
        function_name: &str,
        payload_json: serde_json::Value,
        callee_override: Option<&str>,
        subject_policy: LocalDaemonSubjectPolicy,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        let default_callee_ura = local_daemon_default_callee_ura();
        let callee_ura = callee_override
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(default_callee_ura.as_str())
            .to_string();
        let subject_ura = subject_policy.resolve(&callee_ura)?;
        Self::from_target(
            function_name,
            payload_json,
            callee_ura,
            subject_ura,
            timeout,
        )
    }

    fn from_target(
        function_name: &str,
        payload_json: serde_json::Value,
        callee_ura: String,
        subject_ura: String,
        timeout: Duration,
    ) -> anyhow::Result<Self> {
        let function_name = normalized_local_daemon_function_name(function_name)?;
        let arguments = serde_json::to_vec(&payload_json)
            .map_err(|err| anyhow::anyhow!("encode {function_name} args: {err}"))?;
        Ok(Self {
            function_name,
            caller_ura: local_daemon_loopback_caller_ura()?,
            callee_ura: normalized_local_daemon_ura(&callee_ura, "callee_ura")?,
            subject_ura: normalized_local_daemon_ura(&subject_ura, "subject_ura")?,
            arguments,
            timeout,
            causal_context: None,
            trace_id: None,
        })
    }

    #[must_use]
    fn with_causal_context(
        mut self,
        causal_context: easynet_axon::pb::axon::v1::CausalContext,
    ) -> Self {
        self.causal_context = Some(causal_context);
        self
    }

    #[must_use]
    fn with_trace_id(mut self, trace_id: Option<&str>) -> Self {
        self.trace_id = trace_id
            .map(str::trim)
            .filter(|trace_id| !trace_id.is_empty())
            .map(str::to_string);
        self
    }

    fn invoke_request(&self) -> anyhow::Result<easynet_axon::pb::axon::v1::InvokeRequest> {
        let request = easynet_axon::pb::axon::v1::InvokeRequest {
            envelope: Some(self.envelope()?),
            function_name: self.function_name.clone(),
            arguments: self.arguments.clone(),
            content_type: "application/json".to_string(),
            timeout_seconds: self.timeout_seconds(),
            ..easynet_axon::pb::axon::v1::InvokeRequest::default()
        };
        if request.function_name.trim().is_empty() {
            anyhow::bail!("function_name must not be empty");
        }
        Ok(request)
    }

    fn stream_request(
        &self,
    ) -> anyhow::Result<easynet_axon::pb::axon::v1::InvokeServerStreamRequest> {
        Ok(easynet_axon::pb::axon::v1::InvokeServerStreamRequest {
            envelope: Some(self.envelope()?),
            function_name: self.function_name.clone(),
            arguments: self.arguments.clone(),
            content_type: "application/json".to_string(),
            timeout_seconds: self.timeout_seconds(),
            ..easynet_axon::pb::axon::v1::InvokeServerStreamRequest::default()
        })
    }

    fn envelope(&self) -> anyhow::Result<easynet_axon::pb::axon::v1::Envelope> {
        let mut envelope = crate::daemon::invocation::ProtoEnvelope::targeted(
            self.caller_ura.clone(),
            self.callee_ura.clone(),
            self.subject_ura.clone(),
        )?;
        if let Some(causal_context) = self.causal_context.clone() {
            envelope = envelope.with_causal_context(causal_context);
        }
        let mut envelope = envelope.into_inner();
        if let Some(trace_id) = self.trace_id.as_ref() {
            envelope.trace_id = trace_id.clone();
        }
        Ok(envelope)
    }

    fn timeout_seconds(&self) -> i32 {
        i32::try_from(self.timeout.as_secs()).unwrap_or(i32::MAX)
    }
}

#[cfg(feature = "axon-pb")]
fn normalized_local_daemon_function_name(function_name: &str) -> anyhow::Result<String> {
    let function_name = function_name.trim();
    if function_name.is_empty() {
        anyhow::bail!("function_name must not be empty");
    }
    Ok(function_name.to_string())
}

#[cfg(feature = "axon-pb")]
fn normalized_local_daemon_ura(value: &str, field: &str) -> anyhow::Result<String> {
    LocalDaemonSubjectPolicy::normalized_ura(value, field)
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
impl LocalDaemonAbilityClient {
    fn grpc() -> Self {
        Self
    }

    pub(crate) fn new() -> anyhow::Result<Self> {
        Self::validate_socket()?;
        Ok(Self::grpc())
    }

    pub(crate) fn invoke_with_subject(
        &self,
        function_name: &str,
        payload_json: serde_json::Value,
        subject: Option<String>,
    ) -> anyhow::Result<serde_json::Value> {
        let subject_policy = LocalDaemonSubjectPolicy::explicit_or_self_target(subject)?;
        self.invoke_with_subject_and_timeout(
            function_name,
            payload_json,
            subject_policy,
            Duration::from_secs(30),
        )
    }

    fn invoke_with_subject_and_timeout(
        &self,
        function_name: &str,
        payload_json: serde_json::Value,
        subject_policy: LocalDaemonSubjectPolicy,
        timeout: Duration,
    ) -> anyhow::Result<serde_json::Value> {
        self.invoke_with_subject_policy(function_name, payload_json, subject_policy, timeout)
    }

    fn invoke_with_subject_policy(
        &self,
        function_name: &str,
        payload_json: serde_json::Value,
        subject_policy: LocalDaemonSubjectPolicy,
        timeout: Duration,
    ) -> anyhow::Result<serde_json::Value> {
        invoke_local_daemon_ability_with_subject_policy(
            function_name,
            payload_json,
            subject_policy,
            timeout,
        )
    }

    fn validate_socket() -> anyhow::Result<()> {
        let socket_path = resolve_socket_path();
        if !probe_accepting(&socket_path) {
            return Err(anyhow::Error::new(
                crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
                    "daemon not running: gRPC listener not reachable at {}. Start it with `easynet runtime start` or `easynet start`.",
                    socket_path.display()
                )),
            ));
        }
        Ok(())
    }
}

#[cfg(not(feature = "axon-pb"))]
#[allow(dead_code)]
impl LocalDaemonAbilityClient {
    pub(crate) fn new() -> anyhow::Result<Self> {
        Err(anyhow::Error::new(
            crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(
                "invoking daemon-hosted Axon abilities requires the `axon-pb` feature; rebuild \
                 with `cargo build --features axon-pb`"
                    .to_string(),
            ),
        ))
    }

    pub(crate) fn invoke(
        &self,
        function_name: &str,
        _payload_json: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        anyhow::bail!(
            "invoking `{}` through the local Axon daemon requires the `axon-pb` feature; \
             rebuild with `cargo build --features axon-pb`",
            function_name
        )
    }

    pub(crate) fn invoke_with_subject(
        &self,
        function_name: &str,
        payload_json: serde_json::Value,
        _subject: Option<String>,
    ) -> anyhow::Result<serde_json::Value> {
        self.invoke(function_name, payload_json)
    }
}

/// Invoke a daemon-hosted ability through Axon's local Invocation
/// gRPC transport (`daemon.sock`). Transport-level entry; CLI
/// surfaces MUST go through [`crate::support::platform::local_invoke::invoke_local_ability_with_subject`]
/// instead so the "one CLI subcommand = one ability invoke"
/// contract is held in exactly one module. This free fn exists
/// for `support`-internal use only (the `local_invoke` shim
/// forwards here).
#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_with_subject(
    function_name: &str,
    payload_json: serde_json::Value,
    subject: Option<String>,
) -> anyhow::Result<serde_json::Value> {
    LocalDaemonAbilityClient::new()?.invoke_with_subject(function_name, payload_json, subject)
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_with_subject_timeout(
    function_name: &str,
    payload_json: serde_json::Value,
    subject: Option<String>,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let subject_policy = LocalDaemonSubjectPolicy::explicit_or_self_target(subject)?;
    invoke_local_daemon_ability_with_callee_and_subject(
        function_name,
        payload_json,
        None,
        subject_policy,
        timeout,
    )
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_stream_first_payload_with_subject(
    function_name: &str,
    payload_json: serde_json::Value,
    subject: Option<String>,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let subject_policy = LocalDaemonSubjectPolicy::explicit_or_self_target(subject)?;
    invoke_local_daemon_ability_stream_first_payload_with_target(
        function_name,
        payload_json,
        None,
        subject_policy,
        timeout,
    )
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_targeted_timeout(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    default_subject_ura: &str,
    subject: Option<String>,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let subject_policy =
        LocalDaemonSubjectPolicy::explicit_or_declared_default(default_subject_ura, subject)?;
    invoke_local_daemon_ability_with_callee_and_subject(
        function_name,
        payload_json,
        Some(callee_ura),
        subject_policy,
        timeout,
    )
}

/// Invoke a daemon-hosted server-stream ability through Axon's local
/// Invocation gRPC transport and drain its JSON frames.
///
/// This is the stream-mode twin of
/// [`invoke_local_daemon_ability_with_subject`]. It deliberately
/// talks to the daemon process, not an in-process test runtime,
/// because stateful stream abilities keep their session state inside
/// that process.
#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_targeted_stream_with_subject(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    default_subject_ura: &str,
    subject: Option<String>,
    timeout: Duration,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    let subject_policy =
        LocalDaemonSubjectPolicy::explicit_or_declared_default(default_subject_ura, subject)?;
    invoke_local_daemon_ability_stream_with_target(
        function_name,
        payload_json,
        Some(callee_ura),
        subject_policy,
        timeout,
        max_frames,
    )
}

/// Open a daemon-hosted bidirectional ability through Axon's local
/// Invocation gRPC transport and drain JSON-frame down output.
#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_targeted_bidi_json_frames_with_subject(
    request: LocalDaemonTargetedBidiRequest<'_>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalBidiFrame>> {
    let LocalDaemonTargetedBidiRequest {
        function_name,
        payload_json,
        callee_ura,
        default_subject_ura,
        subject,
        timeout,
        input_frames,
        max_frames,
    } = request;
    let subject_policy =
        LocalDaemonSubjectPolicy::explicit_or_declared_default(default_subject_ura, subject)?;
    invoke_local_daemon_ability_bidi_json_frames_with_target(
        function_name,
        payload_json,
        Some(callee_ura),
        subject_policy,
        timeout,
        input_frames,
        max_frames,
    )
}

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_stream_with_target(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_override: Option<&str>,
    subject_policy: LocalDaemonSubjectPolicy,
    timeout: Duration,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    use anyhow::Context;

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

    let invocation = LocalDaemonLoopbackInvocation::from_subject_policy(
        function_name,
        payload_json,
        callee_override,
        subject_policy,
        timeout,
    )?;
    let function_name = invocation.function_name.clone();
    let request = invocation.stream_request()?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for local Axon daemon stream invoke")?;

    runtime.block_on(async move {
        let channel = connect_channel(socket_path.clone(), timeout, Duration::from_secs(10))
            .await
            .map_err(|source| local_daemon_connect_error(&socket_path, source))?;
        let mut client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel);
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
fn invoke_local_daemon_ability_bidi_json_frames_with_target(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_override: Option<&str>,
    subject_policy: LocalDaemonSubjectPolicy,
    timeout: Duration,
    input_frames: Vec<serde_json::Value>,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalBidiFrame>> {
    use anyhow::Context;
    use base64::{Engine as _, engine::general_purpose::STANDARD as B64};
    use easynet_axon::pb::axon::v1::{
        BinaryChunk, ContentEnvelope, EnvelopeOpen, InvocationTarget, InvokeBidiUp,
        StreamDescriptor, invoke_bidi_down::Payload as DownPayload,
        invoke_bidi_up::Payload as UpPayload,
    };
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

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

    let invocation = LocalDaemonLoopbackInvocation::from_subject_policy(
        function_name,
        payload_json,
        callee_override,
        subject_policy,
        timeout,
    )?;
    let function_name = invocation.function_name.clone();
    let envelope_open = EnvelopeOpen {
        envelope: Some(invocation.envelope()?),
        target: Some(InvocationTarget {
            ability_name: function_name.clone(),
            ..InvocationTarget::default()
        }),
        initial_args: invocation.arguments.clone(),
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
        let mut client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel)
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
            let sequence = frame.sequence;
            let Some(payload) = frame.payload else {
                continue;
            };
            let projected = match payload {
                DownPayload::BinaryChunk(chunk) => {
                    let payload = serde_json::from_slice(&chunk.data).unwrap_or_else(|_| {
                        serde_json::json!({
                            "type": "binary",
                            "stream_id": chunk.stream_id,
                            "data_b64": B64.encode(&chunk.data),
                        })
                    });
                    crate::support::platform::local_invoke::LocalBidiFrame {
                        sequence,
                        content_type: "application/json".to_string(),
                        terminal: false,
                        payload,
                    }
                }
                DownPayload::Receipt(receipt) => {
                    let terminal = receipt.state
                        != easynet_axon::invocation::InvocationState::Admitted.to_wire_i32();
                    let receipt_payload = if receipt.payload.is_empty() {
                        serde_json::Value::Null
                    } else {
                        serde_json::from_slice(&receipt.payload).unwrap_or_else(|_| {
                            serde_json::json!({
                                "data_b64": B64.encode(&receipt.payload),
                            })
                        })
                    };
                    crate::support::platform::local_invoke::LocalBidiFrame {
                        sequence,
                        content_type: receipt.payload_content_type.clone(),
                        terminal,
                        payload: serde_json::json!({
                            "type": "receipt",
                            "state": receipt.state,
                            "reason": receipt.reason,
                            "cleanup_complete": receipt.cleanup_complete,
                            "failure": receipt.failure.map(|failure| serde_json::json!({
                                "code": failure.code,
                                "message": failure.message,
                                "retryable": failure.retryable,
                            })),
                            "payload": receipt_payload,
                        }),
                    }
                }
                DownPayload::Control(_) => crate::support::platform::local_invoke::LocalBidiFrame {
                    sequence,
                    content_type: "application/json".to_string(),
                    terminal: false,
                    payload: serde_json::json!({"type": "control"}),
                },
                DownPayload::DispatchCall(_) | DownPayload::ReverseDispatchResult(_) => continue,
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
pub(crate) fn invoke_local_daemon_ability_targeted_stream_with_subject(
    function_name: &str,
    _payload_json: serde_json::Value,
    _callee_ura: &str,
    _default_subject_ura: &str,
    _subject: Option<String>,
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
pub(crate) fn invoke_local_daemon_ability_targeted_bidi_json_frames_with_subject(
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
fn invoke_local_daemon_ability_with_subject_policy(
    function_name: &str,
    payload_json: serde_json::Value,
    subject_policy: LocalDaemonSubjectPolicy,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    invoke_local_daemon_ability_with_callee_and_subject(
        function_name,
        payload_json,
        None,
        subject_policy,
        timeout,
    )
}

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_with_callee_and_subject(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_override: Option<&str>,
    subject_policy: LocalDaemonSubjectPolicy,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    use anyhow::Context;

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

    let invocation = LocalDaemonLoopbackInvocation::from_subject_policy(
        function_name,
        payload_json,
        callee_override,
        subject_policy,
        timeout,
    )?;
    let function_name = invocation.function_name.clone();
    let request = invocation.invoke_request()?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for local Axon daemon invoke")?;

    runtime.block_on(async move {
        let channel = connect_channel(socket_path.clone(), timeout, Duration::from_secs(10))
            .await
            .map_err(|source| local_daemon_connect_error(&socket_path, source))?;
        let mut client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel);
        let response = client
            .invoke(request)
            .await
            .map_err(|status| local_daemon_status_error(&function_name, status))?;
        let body = response.into_inner();
        if body.result.is_empty() {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_slice(&body.result)
            .with_context(|| format!("decode {function_name} Axon response"))
    })
}

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_stream_first_payload_with_target(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_override: Option<&str>,
    subject_policy: LocalDaemonSubjectPolicy,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    use anyhow::Context;

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

    let invocation = LocalDaemonLoopbackInvocation::from_subject_policy(
        function_name,
        payload_json,
        callee_override,
        subject_policy,
        timeout,
    )?;
    let function_name = invocation.function_name.clone();
    let stream_request = invocation.stream_request()?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for local Axon daemon stream invoke")?;

    runtime.block_on(async move {
        let channel = connect_channel(socket_path.clone(), timeout, Duration::from_secs(10))
            .await
            .map_err(|source| local_daemon_connect_error(&socket_path, source))?;
        let mut client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel);
        let projection =
            invoke_local_daemon_first_stream_payload(&mut client, stream_request, &function_name)
                .await?;
        let value = projection.value;
        Ok::<_, anyhow::Error>(value)
    })
}

/// Invoke a daemon-hosted ability AND return the invocation record
/// alongside the result. Transport-level entry; CLI surfaces MUST go
/// through [`crate::support::platform::local_invoke::invoke_local_ability_with_invocation_meta`].
///
/// This differs from [`invoke_local_daemon_ability_with_subject`] in
/// two protocol-visible ways:
///
///   1. The envelope's `causal_context` is set explicitly from the
///      caller-provided parent receipt anchors: `Empty` for a root
///      invocation, a scalar `ReceiptRef` for one parent, an ordered
///      `ReceiptList` for a fan-in join. The default path leaves the
///      field unset; this path makes causal placement a first-class
///      input so receipt-DAG reconstruction has real edges to read.
///   2. The terminal `InvokeResponse` supplies the signed receipt used
///      to construct the invocation URA and causal receipt anchor. The
///      response is the atomic protocol result; this path never polls a
///      product ledger projection after execution.
///
/// A parent entry must match a `(receipt_ura, receipt_hash)` capability
/// produced by a finalization projection verified in this process. Shape-only
/// or stale claims are rejected before dispatch; they cannot be upgraded into
/// causal evidence by JSON parsing.
#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_with_invocation_meta(
    function_name: &str,
    payload_json: serde_json::Value,
    subject: Option<String>,
    causal_parents: &[serde_json::Value],
    step_timeout: Option<Duration>,
    trace_id: Option<&str>,
    callee_agent: Option<&str>,
) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
    invoke_local_daemon_ability_with_invocation_meta_inner(LocalDaemonInvocationMetaRequest {
        function_name,
        payload_json,
        subject,
        causal_parents,
        step_timeout,
        trace_id,
        delegation: None,
        target: LocalDaemonInvocationMetaTarget::MissionCompatibility { callee_agent },
    })
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_targeted_with_invocation_meta(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    default_subject_ura: &str,
    subject: Option<String>,
    causal_parents: &[serde_json::Value],
    step_timeout: Option<Duration>,
    trace_id: Option<&str>,
) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
    invoke_local_daemon_ability_with_invocation_meta_inner(LocalDaemonInvocationMetaRequest {
        function_name,
        payload_json,
        subject,
        causal_parents,
        step_timeout,
        trace_id,
        delegation: None,
        target: LocalDaemonInvocationMetaTarget::Canonical {
            callee_ura,
            default_subject_ura,
        },
    })
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_with_hosted_agent_delegation(
    function_name: &str,
    payload_json: serde_json::Value,
    subject: Option<String>,
    causal_parents: &[serde_json::Value],
    step_timeout: Option<Duration>,
    trace_id: Option<&str>,
    hosted_agent_ura: &str,
) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
    let delegated = HostedAgentDelegationRequest::new(hosted_agent_ura)?;
    invoke_local_daemon_ability_with_invocation_meta_inner(LocalDaemonInvocationMetaRequest {
        function_name,
        payload_json,
        subject,
        causal_parents,
        step_timeout,
        trace_id,
        delegation: Some(delegated),
        target: LocalDaemonInvocationMetaTarget::MissionCompatibility { callee_agent: None },
    })
}

#[cfg(feature = "axon-pb")]
struct LocalDaemonInvocationMetaRequest<'a> {
    function_name: &'a str,
    payload_json: serde_json::Value,
    subject: Option<String>,
    causal_parents: &'a [serde_json::Value],
    step_timeout: Option<Duration>,
    trace_id: Option<&'a str>,
    delegation: Option<HostedAgentDelegationRequest>,
    target: LocalDaemonInvocationMetaTarget<'a>,
}

#[cfg(feature = "axon-pb")]
enum LocalDaemonInvocationMetaTarget<'a> {
    MissionCompatibility {
        callee_agent: Option<&'a str>,
    },
    Canonical {
        callee_ura: &'a str,
        default_subject_ura: &'a str,
    },
}

#[cfg(feature = "axon-pb")]
struct ResolvedLocalDaemonInvocationMetaTarget {
    callee_ura: String,
    subject_ura: String,
}

#[cfg(feature = "axon-pb")]
impl LocalDaemonInvocationMetaTarget<'_> {
    fn resolve(
        self,
        subject: Option<String>,
        trace_id: Option<&str>,
    ) -> anyhow::Result<ResolvedLocalDaemonInvocationMetaTarget> {
        match self {
            Self::Canonical {
                callee_ura,
                default_subject_ura,
            } => {
                let callee_ura = normalized_local_daemon_ura(callee_ura, "callee_ura")?;
                let subject_ura = LocalDaemonSubjectPolicy::explicit_or_declared_default(
                    default_subject_ura,
                    subject,
                )?
                .resolve(&callee_ura)?;
                Ok(ResolvedLocalDaemonInvocationMetaTarget {
                    callee_ura,
                    subject_ura,
                })
            }
            Self::MissionCompatibility { callee_agent } => {
                let callee_ura = match callee_agent.map(str::trim).filter(|a| !a.is_empty()) {
                    Some(agent) => canonical_hosted_agent_ura_by_name(agent)?,
                    None => local_daemon_default_callee_ura(),
                };
                let subject_ura = match subject.as_deref().map(str::trim).filter(|s| !s.is_empty())
                {
                    Some(explicit) => explicit.to_string(),
                    None => match trace_id.map(str::trim).filter(|t| !t.is_empty()) {
                        Some(mission_id) => LocalMissionSubjectOwner::from_runtime_identity()?
                            .mission_subject_ura(mission_id),
                        None => callee_ura.clone(),
                    },
                };
                Ok(ResolvedLocalDaemonInvocationMetaTarget {
                    callee_ura,
                    subject_ura,
                })
            }
        }
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug)]
struct UnverifiedTerminalInvocationProjection {
    state: &'static str,
    admission_receipt: easynet_axon::pb::axon::v1::InvocationReceipt,
    terminal_receipt: easynet_axon::pb::axon::v1::InvocationReceipt,
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
    fn to_wire(&self) -> easynet_axon::pb::axon::v1::ReceiptRef {
        easynet_axon::pb::axon::v1::ReceiptRef {
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
        terminal: &easynet_axon::invocation::SignedInvocationReceipt,
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
impl easynet_axon::invocation::KeyResolver for LocalKeyServiceReceiptResolver {
    fn resolve(
        &self,
        signer_ura: &str,
    ) -> Result<ed25519_dalek::VerifyingKey, easynet_axon::invocation::AxonError> {
        use crate::daemon::identity::self_identity::SelfIdentity as _;

        self.key_service.public_key(signer_ura).map_err(|error| {
            easynet_axon::invocation::AxonError::permission_denied(
                "local_receipt_signer_key_untrusted",
            )
            .with_message(format!(
                "trusted local key service cannot resolve receipt signer {signer_ura:?}: {error}"
            ))
        })
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug)]
struct SubmittedInvocationProjection {
    envelope: easynet_axon::pb::axon::v1::Envelope,
    function_name: String,
    input_hash: [u8; 32],
}

#[cfg(feature = "axon-pb")]
impl SubmittedInvocationProjection {
    fn from_request(
        request: &easynet_axon::pb::axon::v1::InvokeRequest,
        ability: &str,
    ) -> anyhow::Result<Self> {
        let envelope = request
            .envelope
            .clone()
            .ok_or_else(|| anyhow::anyhow!("{ability}: invoke request omitted its envelope"))?;
        Ok(Self {
            envelope,
            function_name: request.function_name.clone(),
            input_hash: easynet_axon::invocation::sha256(&request.arguments),
        })
    }
}

#[cfg(feature = "axon-pb")]
impl UnverifiedTerminalInvocationProjection {
    fn from_response(
        response: &easynet_axon::pb::axon::v1::InvokeResponse,
        submitted: &SubmittedInvocationProjection,
        ability: &str,
    ) -> anyhow::Result<Self> {
        use anyhow::{anyhow, bail};
        use easynet_axon::invocation::InvocationState;

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
        if receipt_state != state {
            bail!(
                "{ability}: response state {state:?} does not match terminal receipt state {receipt_state:?}"
            );
        }
        let state_label = state
            .default_event_type()
            .ok_or_else(|| anyhow!("{ability}: terminal receipt projected an unspecified state"))?;
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
        if receipt.output_hash != easynet_axon::invocation::sha256(&response.result) {
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
        easynet_axon::ura::invocation_record_ura_for_binding(
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
        resolver: &dyn easynet_axon::invocation::KeyResolver,
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
        let invocation_ura = easynet_axon::ura::invocation_record_ura_for_binding(
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
fn validate_receipt_signature_shape(
    receipt: &easynet_axon::pb::axon::v1::InvocationReceipt,
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
    receipt: &easynet_axon::pb::axon::v1::InvocationReceipt,
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
    let ability_ura =
        easynet_axon::invocation::ability_ura_from_descriptor_ref(&receipt.ability_binding)
            .map_err(|error| {
                anyhow::anyhow!("{ability}: {stage} receipt ability binding is invalid: {error}")
            })?;
    let public_name = easynet_axon::ura::qualified_ability_name(ability_ura).ok_or_else(|| {
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
    causal: Option<&easynet_axon::pb::axon::v1::CausalContext>,
    ability: &str,
) -> anyhow::Result<Vec<easynet_axon::pb::axon::v1::ReceiptRef>> {
    use anyhow::bail;
    use easynet_axon::pb::axon::v1::causal_context::Form;

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
    admission: &easynet_axon::pb::axon::v1::InvocationReceipt,
    terminal: &easynet_axon::pb::axon::v1::InvocationReceipt,
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
const STREAM_FIRST_FRAME_PROJECTION_MAX_FRAMES: usize = 64;

#[cfg(feature = "axon-pb")]
type LocalInvocationGrpcClient =
    easynet_axon::pb::axon::v1::invocation_client::InvocationClient<tonic::transport::Channel>;

#[cfg(feature = "axon-pb")]
#[derive(Debug)]
struct LocalDaemonStreamProjection {
    value: serde_json::Value,
}

#[cfg(feature = "axon-pb")]
async fn invoke_local_daemon_json(
    client: &mut LocalInvocationGrpcClient,
    request: easynet_axon::pb::axon::v1::InvokeRequest,
    function_name: &str,
) -> anyhow::Result<(
    serde_json::Value,
    easynet_axon::pb::axon::v1::InvokeResponse,
)> {
    use anyhow::Context;
    use serde_json::Value;

    let response = client
        .invoke(request)
        .await
        .map_err(|status| local_daemon_status_error(function_name, status))?;
    let body = response.into_inner();
    let value = if body.result.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body.result)
            .with_context(|| format!("decode {function_name} Axon response"))?
    };
    Ok((value, body))
}

#[cfg(feature = "axon-pb")]
async fn invoke_local_daemon_first_stream_payload(
    client: &mut LocalInvocationGrpcClient,
    request: easynet_axon::pb::axon::v1::InvokeServerStreamRequest,
    function_name: &str,
) -> anyhow::Result<LocalDaemonStreamProjection> {
    use anyhow::{Context, bail};

    let mut stream = client
        .invoke_stream(request)
        .await
        .map_err(|status| local_daemon_status_error(function_name, status))?
        .into_inner();

    let mut frames_seen = 0_usize;
    while let Some(chunk) = stream
        .message()
        .await
        .map_err(|status| local_daemon_status_error(function_name, status))?
    {
        frames_seen = frames_seen.saturating_add(1);
        if !chunk.payload.is_empty() {
            let value = serde_json::from_slice(&chunk.payload)
                .with_context(|| format!("decode {function_name} first stream payload JSON"))?;
            return Ok(LocalDaemonStreamProjection { value });
        }
        if frames_seen >= STREAM_FIRST_FRAME_PROJECTION_MAX_FRAMES {
            bail!(
                "stream projection for {function_name} did not produce a JSON payload \
                 within {STREAM_FIRST_FRAME_PROJECTION_MAX_FRAMES} frames; \
                 scalar projection requires a payload frame"
            );
        }
    }

    bail!("stream projection for {function_name} ended before a JSON payload frame")
}

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_with_invocation_meta_inner(
    request: LocalDaemonInvocationMetaRequest<'_>,
) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
    use anyhow::{Context, anyhow, bail};
    use easynet_axon::pb::axon::v1 as pb;
    use serde_json::Value;

    let LocalDaemonInvocationMetaRequest {
        function_name,
        payload_json,
        subject,
        causal_parents,
        step_timeout,
        trace_id,
        delegation,
        target,
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

    let ResolvedLocalDaemonInvocationMetaTarget {
        callee_ura,
        subject_ura,
    } = target.resolve(subject, trace_id)?;
    let receipt_refs = verified_receipt_refs_from_causal_parents(causal_parents)?;
    let mut refs = receipt_refs;
    let causal_form = match refs.len() {
        0 => pb::causal_context::Form::None(pb::Empty {}),
        1 => pb::causal_context::Form::Scalar(refs.remove(0)),
        _ => pb::causal_context::Form::List(pb::ReceiptList { prior: refs }),
    };
    let invocation = LocalDaemonLoopbackInvocation::from_target(
        &function_name,
        payload_json,
        callee_ura.clone(),
        subject_ura.clone(),
        step_timeout.unwrap_or_else(|| Duration::from_secs(30)),
    )?
    .with_causal_context(pb::CausalContext {
        form: Some(causal_form),
    })
    .with_trace_id(trace_id);
    let mut request = invocation.invoke_request()?;
    let wire_caller_ura = invocation.caller_ura.clone();
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
        .map(|t| t + Duration::from_secs(30))
        .unwrap_or_else(|| Duration::from_secs(60));
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
        let mut client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel);
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
        "submitted_callee_ura": callee_ura,
        "submitted_subject_ura": subject_ura,
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
) -> anyhow::Result<Vec<easynet_axon::pb::axon::v1::ReceiptRef>> {
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
pub(crate) fn invoke_local_daemon_ability_with_invocation_meta(
    function_name: &str,
    _payload_json: serde_json::Value,
    _subject: Option<String>,
    _causal_parents: &[serde_json::Value],
    _step_timeout: Option<std::time::Duration>,
    _trace_id: Option<&str>,
    _callee_agent: Option<&str>,
) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
    anyhow::bail!(
        "invoking `{}` with invocation metadata requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_targeted_with_invocation_meta(
    function_name: &str,
    _payload_json: serde_json::Value,
    _callee_ura: &str,
    _default_subject_ura: &str,
    _subject: Option<String>,
    _causal_parents: &[serde_json::Value],
    _step_timeout: Option<std::time::Duration>,
    _trace_id: Option<&str>,
) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
    anyhow::bail!(
        "invoking targeted `{}` with invocation metadata requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_with_hosted_agent_delegation(
    function_name: &str,
    _payload_json: serde_json::Value,
    _subject: Option<String>,
    _causal_parents: &[serde_json::Value],
    _step_timeout: Option<std::time::Duration>,
    _trace_id: Option<&str>,
    _hosted_agent_ura: &str,
) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
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
pub(crate) fn invoke_local_daemon_ability_with_subject(
    function_name: &str,
    payload_json: serde_json::Value,
    _subject: Option<String>,
) -> anyhow::Result<serde_json::Value> {
    invoke_local_daemon_ability(function_name, payload_json)
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_with_subject_timeout(
    function_name: &str,
    payload_json: serde_json::Value,
    _subject: Option<String>,
    _timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    invoke_local_daemon_ability(function_name, payload_json)
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_stream_first_payload_with_subject(
    function_name: &str,
    _payload_json: serde_json::Value,
    _subject: Option<String>,
    _timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    anyhow::bail!(
        "streaming `{}` through the local Axon daemon requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_targeted_timeout(
    function_name: &str,
    payload_json: serde_json::Value,
    _callee_ura: &str,
    _default_subject_ura: &str,
    _subject: Option<String>,
    _timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    invoke_local_daemon_ability(function_name, payload_json)
}

#[cfg(feature = "axon-pb")]
fn local_daemon_loopback_caller_ura() -> anyhow::Result<String> {
    Ok(crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA.to_string())
}

#[cfg(feature = "axon-pb")]
fn local_daemon_default_callee_ura() -> String {
    crate::daemon::identity::local_invocation::local_daemon_ura()
}

#[cfg(feature = "axon-pb")]
fn canonical_hosted_agent_ura_by_name(agent_name: &str) -> anyhow::Result<String> {
    let agent_name = agent_name.trim();
    if agent_name.is_empty() {
        anyhow::bail!("hosted agent callee name must not be empty");
    }
    let snapshot = AgentAggregateRepository::try_load_snapshot()
        .map_err(|err| anyhow::anyhow!("load Agent aggregate for hosted agent callee: {err:#}"))?;
    let agent_ura = snapshot
        .hosted_agent_ura_by_name(agent_name)
        .map_err(|error| hosted_agent_callee_lookup_error(agent_name, error))?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "hosted agent {agent_name:?} is not registered in the Agent aggregate; run federation join/agent advertise before invoking as that agent"
            )
        })?;
    Ok(agent_ura.to_string())
}

#[cfg(feature = "axon-pb")]
fn hosted_agent_callee_lookup_error(
    agent_name: &str,
    error: HostedAgentNameLookupError,
) -> anyhow::Error {
    match error {
        HostedAgentNameLookupError::Ambiguous {
            first_profile,
            second_profile,
            ..
        } => anyhow::anyhow!(
            "hosted agent {agent_name:?} is ambiguous across profiles {first_profile:?} and {second_profile:?}"
        ),
        HostedAgentNameLookupError::InvalidUra {
            agent_ura, reason, ..
        } => anyhow::anyhow!(
            "hosted agent {agent_name:?} has invalid Agent URA {agent_ura:?}: {reason}"
        ),
        HostedAgentNameLookupError::NonAgentUra { agent_ura, .. } => {
            anyhow::anyhow!("hosted agent {agent_name:?} resolved to non-Agent URA {agent_ura:?}")
        }
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalMissionSubjectOwner {
    realm: String,
    device_id: String,
}

#[cfg(feature = "axon-pb")]
impl LocalMissionSubjectOwner {
    fn from_runtime_identity() -> anyhow::Result<Self> {
        Self::from_device_ura(&crate::daemon::identity::local_invocation::local_device_ura())
    }

    fn from_device_ura(device_ura: &str) -> anyhow::Result<Self> {
        let parsed = crate::core::ura::parse_ura(device_ura).map_err(|err| {
            anyhow::anyhow!(
                "local mission subject owner has invalid Device URA {device_ura:?}: {err}"
            )
        })?;
        if parsed.kind != crate::core::ura::URAKind::Device {
            anyhow::bail!("local mission subject owner must be a Device URA, got {device_ura:?}");
        }
        let device_id = parsed
            .device_id()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "local mission subject owner Device URA has no device id: {device_ura:?}"
                )
            })?
            .to_string();
        Ok(Self {
            realm: parsed.realm,
            device_id,
        })
    }

    fn mission_subject_ura(&self, mission_id: &str) -> String {
        crate::core::ura::resource_dot_ura(
            &self.realm,
            &format!("device.{}.missions", self.device_id),
            mission_id,
        )
    }
}

#[cfg(all(feature = "axon-pb", test))]
mod local_mission_subject_owner_tests {
    use super::LocalMissionSubjectOwner;

    #[test]
    fn mission_subject_uses_device_ura_identity() {
        let owner = LocalMissionSubjectOwner::from_device_ura("easynet:///r/acme/device/device-1")
            .expect("device owner");

        assert_eq!(
            owner.mission_subject_ura("mission-42"),
            "easynet:///r/acme/resource/device.device-1.missions/mission-42"
        );
    }

    #[test]
    fn mission_subject_accepts_unpaired_local_identity() {
        let owner = LocalMissionSubjectOwner::from_device_ura("easynet:///r/default/device/local")
            .expect("unpaired local device owner");

        assert_eq!(
            owner.mission_subject_ura("mission-42"),
            "easynet:///r/default/resource/device.local.missions/mission-42"
        );
    }
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;

    #[test]
    fn loopback_invoke_request_does_not_pre_resolve_descriptor_ref() {
        let invocation = LocalDaemonLoopbackInvocation::from_subject_policy(
            "discover",
            serde_json::json!({"query": "capabilities"}),
            Some("easynet:///r/default/agent/dev.worker"),
            LocalDaemonSubjectPolicy::SelfTarget,
            Duration::from_secs(5),
        )
        .expect("loopback invocation projection");
        let request = invocation.invoke_request().expect("loopback request");

        assert_eq!(request.function_name, "discover");
        assert!(
            !request.metadata.contains_key(
                crate::daemon::invocation::dispatch::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
            ),
            "local loopback projection must not pre-bind descriptor metadata"
        );
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
    }

    #[test]
    fn canonical_invocation_meta_target_preserves_declared_owner_and_subject() {
        let target = LocalDaemonInvocationMetaTarget::Canonical {
            callee_ura: "easynet:///r/acme/agent/alice.tools",
            default_subject_ura: "easynet:///r/acme/ability/alice.tools.files.read",
        }
        .resolve(None, Some("mission-id-must-not-rewrite-subject"))
        .expect("canonical targeted meta identity");

        assert_eq!(target.callee_ura, "easynet:///r/acme/agent/alice.tools");
        assert_eq!(
            target.subject_ura,
            "easynet:///r/acme/ability/alice.tools.files.read"
        );
        assert_ne!(target.callee_ura, local_daemon_default_callee_ura());
    }

    fn completed_receipt_response_fixture(
        seed: u8,
        _invocation_id: &str,
    ) -> (
        SubmittedInvocationProjection,
        easynet_axon::pb::axon::v1::InvokeResponse,
        ed25519_dalek::SigningKey,
    ) {
        use easynet_axon::invocation::{
            AbilityCallModes, AbilityOptions, AgentIdentity, Ed25519ReceiptSigningAuthority,
            InvocationState, ReceiptSigningAuthority, StaticReceiptSigningAuthorityProvider,
            UraProfile, make_ability,
        };
        use easynet_axon::pb::axon::v1::InvokeResponse;
        use ed25519_dalek::SigningKey;

        let invocation = LocalDaemonLoopbackInvocation::from_subject_policy(
            "job.run",
            serde_json::json!({"job": 1}),
            Some("easynet:///r/acme/device/edge-1"),
            LocalDaemonSubjectPolicy::Explicit(
                "easynet:///r/acme/resource/user.jobs/job-1".to_string(),
            ),
            Duration::from_secs(5),
        )
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

        let runtime = {
            let authority: std::sync::Arc<dyn ReceiptSigningAuthority> =
                std::sync::Arc::new(Ed25519ReceiptSigningAuthority::self_signed(
                    callee.clone(),
                    signing_key.clone(),
                    "local-daemon-grpc-fixture",
                ));
            let mut provider = StaticReceiptSigningAuthorityProvider::new();
            provider
                .insert(authority)
                .expect("insert fixture receipt authority");
            easynet_axon::invocation::LocalRuntime::new_with_receipt_signing_authority_provider(
                std::sync::Arc::new(provider),
            )
        };
        crate::daemon::axon_bridge::runtime_factory::configure_local_runtime(&runtime, None, None);
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
                        easynet_axon::invocation::wire::receipt_to_wire(
                            &outcome.admission_receipt.expect("admission receipt"),
                        )
                        .expect("signed admission fixture projects to wire"),
                    ),
                    terminal_receipt: Some(
                        easynet_axon::invocation::wire::receipt_to_wire(
                            &outcome.terminal_receipt.expect("terminal receipt"),
                        )
                        .expect("signed terminal fixture projects to wire"),
                    ),
                    ..Default::default()
                }
            });
        (submitted, response, signing_key)
    }

    struct FixedReceiptKeyResolver {
        signer_ura: String,
        key: ed25519_dalek::VerifyingKey,
    }

    impl easynet_axon::invocation::KeyResolver for FixedReceiptKeyResolver {
        fn resolve(
            &self,
            signer_ura: &str,
        ) -> Result<ed25519_dalek::VerifyingKey, easynet_axon::invocation::AxonError> {
            if signer_ura == self.signer_ura {
                return Ok(self.key);
            }
            Err(easynet_axon::invocation::AxonError::permission_denied(
                "test_receipt_key_unknown",
            ))
        }
    }

    struct UnknownReceiptKeyResolver;

    impl easynet_axon::invocation::KeyResolver for UnknownReceiptKeyResolver {
        fn resolve(
            &self,
            _signer_ura: &str,
        ) -> Result<ed25519_dalek::VerifyingKey, easynet_axon::invocation::AxonError> {
            Err(easynet_axon::invocation::AxonError::permission_denied(
                "test_receipt_key_unknown",
            ))
        }
    }

    fn fixture_resolver(
        response: &easynet_axon::pb::axon::v1::InvokeResponse,
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

    fn causal_parent_claim(
        response: &easynet_axon::pb::axon::v1::InvokeResponse,
    ) -> serde_json::Value {
        let terminal = response
            .terminal_receipt
            .as_ref()
            .expect("terminal receipt");
        let caller_ura = &terminal.caller_binding.as_ref().expect("caller").ura;
        let callee_ura = &terminal.callee_binding.as_ref().expect("callee").ura;
        let subject_ura = &terminal.subject_binding.as_ref().expect("subject").ura;
        let invocation_ura = easynet_axon::ura::invocation_record_ura_for_binding(
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
        assert!(
            projection.receipt["anchor"]["receipt_ura"]
                .as_str()
                .expect("receipt URA")
                .ends_with(&expected_anchor_suffix)
        );
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
        assert!(
            error
                .to_string()
                .contains("terminal receipt signature is invalid")
        );
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
            .find_map(|cause| cause.downcast_ref::<easynet_axon::invocation::AxonError>())
            .expect("verification error preserves the source AxonError");
        assert_eq!(axon_error.reason, "test_receipt_key_unknown");

        let error = verified_receipt_refs_from_causal_parents(&[parent])
            .expect_err("unverified anchor must not enter causal context");
        assert!(
            error
                .to_string()
                .contains("was not cryptographically verified")
        );
    }

    #[test]
    fn terminal_invocation_projection_rejects_response_receipt_state_mismatch() {
        use easynet_axon::invocation::InvocationState;
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
        use easynet_axon::invocation::InvocationState;

        let (submitted, mut response, _signing_key) =
            completed_receipt_response_fixture(0x35, "inv-failed-response");
        response.state = InvocationState::Failed.to_wire_i32();

        let error =
            UnverifiedTerminalInvocationProjection::from_response(&response, &submitted, "job.run")
                .expect_err("failed terminal state must not become a successful product result");
        assert!(error.to_string().contains("ended in Failed"));
    }

    #[test]
    fn stream_first_payload_projection_has_bounded_empty_frame_budget() {
        const {
            assert!(
                STREAM_FIRST_FRAME_PROJECTION_MAX_FRAMES > 0
                    && STREAM_FIRST_FRAME_PROJECTION_MAX_FRAMES <= 64
            );
        }
    }
}
