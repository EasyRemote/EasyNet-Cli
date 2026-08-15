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

use std::path::Path;
use std::time::Duration;

#[cfg(feature = "axon-pb")]
use std::collections::{HashMap, HashSet, VecDeque};
#[cfg(feature = "axon-pb")]
use std::path::PathBuf;
#[cfg(feature = "axon-pb")]
use std::sync::{Arc, Mutex, OnceLock};

#[cfg(feature = "axon-pb")]
use crate::daemon::ability::{
    HostedAgentDelegationRequest, HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY,
};
#[cfg(feature = "axon-pb")]
use crate::daemon::invocation::dispatch::invocation_wire::{
    wire_invocation_target, InvocationDerivationPolicy, LocalDaemonSystemInvocation,
};
#[cfg(feature = "axon-pb")]
use crate::daemon::persistence::daemon_config;
#[cfg(feature = "axon-pb")]
use tonic::transport::{Channel, Endpoint, Uri as GrpcEndpointLocator};

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
) -> anyhow::Result<Channel> {
    let endpoint = Endpoint::try_from("http://[::1]:50051")?
        .timeout(timeout)
        .connect_timeout(connect_timeout);

    #[cfg(unix)]
    {
        return endpoint
            .connect_with_connector(tower::service_fn(move |_: GrpcEndpointLocator| {
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
            .connect_with_connector(tower::service_fn(move |_: GrpcEndpointLocator| {
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
enum LocalDaemonSystemCalleePolicy {
    Explicit(String),
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalDaemonSystemSubjectPolicy {
    Explicit(String),
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq)]
enum LocalDaemonSystemDerivationPolicy {
    FreshRoot,
    ExplicitCausal {
        invocation_nonce: [u8; 16],
        causal_context: axon_sdk::pb::axon::v1::CausalContext,
    },
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
struct LocalDaemonSystemTuplePlan {
    function_name: String,
    payload_json: serde_json::Value,
    caller_ura: String,
    callee_policy: LocalDaemonSystemCalleePolicy,
    subject_policy: LocalDaemonSystemSubjectPolicy,
    derivation_policy: LocalDaemonSystemDerivationPolicy,
    authority_metadata:
        Option<crate::daemon::invocation::admission::authority_metadata::IssuedAuthorityMetadata>,
    timeout: Duration,
}

pub(crate) struct LocalDaemonTargetedBidiRequest<'a> {
    pub function_name: &'a str,
    pub payload_json: serde_json::Value,
    pub callee_ura: &'a str,
    pub subject_ura: &'a str,
    pub invocation_nonce: [u8; 16],
    pub causal_context: axon_sdk::invocation::CausalContext,
    pub timeout: Duration,
    pub input_frames: Vec<serde_json::Value>,
    pub max_frames: Option<usize>,
}

#[cfg(feature = "axon-pb")]
impl LocalDaemonSystemCalleePolicy {
    fn explicit(callee_ura: &str) -> anyhow::Result<Self> {
        Ok(Self::Explicit(normalized_local_daemon_ura(
            callee_ura,
            "callee_ura",
        )?))
    }

    fn resolve(&self) -> anyhow::Result<String> {
        match self {
            Self::Explicit(callee_ura) => normalized_local_daemon_ura(callee_ura, "callee_ura"),
        }
    }
}

#[cfg(feature = "axon-pb")]
impl LocalDaemonSystemSubjectPolicy {
    fn required_explicit(subject_ura: &str) -> anyhow::Result<Self> {
        Ok(Self::Explicit(normalized_local_daemon_ura(
            subject_ura,
            "subject_ura",
        )?))
    }

    fn resolve(&self) -> anyhow::Result<String> {
        match self {
            Self::Explicit(subject) => normalized_local_daemon_ura(subject, "subject_ura"),
        }
    }
}

#[cfg(feature = "axon-pb")]
impl LocalDaemonSystemDerivationPolicy {
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
impl LocalDaemonSystemTuplePlan {
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
            LocalDaemonSystemCalleePolicy::explicit(callee_ura)?,
            LocalDaemonSystemSubjectPolicy::required_explicit(subject_ura)?,
            LocalDaemonSystemDerivationPolicy::fresh_root(),
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
            LocalDaemonSystemCalleePolicy::explicit(callee_ura)?,
            LocalDaemonSystemSubjectPolicy::required_explicit(subject_ura)?,
            LocalDaemonSystemDerivationPolicy::explicit_causal(invocation_nonce, causal_context)?,
            timeout,
        )
    }

    fn new(
        function_name: &str,
        payload_json: serde_json::Value,
        callee_policy: LocalDaemonSystemCalleePolicy,
        subject_policy: LocalDaemonSystemSubjectPolicy,
        derivation_policy: LocalDaemonSystemDerivationPolicy,
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
            caller_ura: local_daemon_system_caller_ura()?,
            callee_policy,
            subject_policy,
            derivation_policy,
            authority_metadata: None,
            timeout,
        })
    }

    fn with_authority_metadata(
        mut self,
        authority_metadata: crate::daemon::invocation::admission::authority_metadata::IssuedAuthorityMetadata,
    ) -> Self {
        self.authority_metadata = Some(authority_metadata);
        self
    }

    fn into_invocation(self) -> anyhow::Result<LocalDaemonSystemInvocation> {
        let callee_ura = self.callee_policy.resolve()?;
        let subject_ura = self.subject_policy.resolve()?;
        LocalDaemonSystemInvocation::from_target(
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
    crate::core::identity::RuntimeIdentityUra::parse(value)
        .map(crate::core::identity::RuntimeIdentityUra::into_string)
        .map_err(|error| anyhow::anyhow!("{field} {error}"))
}

#[cfg(feature = "axon-pb")]
fn local_daemon_system_invocation_from_tuple_plan(
    tuple_plan: LocalDaemonSystemTuplePlan,
) -> anyhow::Result<LocalDaemonSystemInvocation> {
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
            "connect to local daemon Invocation endpoint at {}: {source:#}",
            socket_path.display()
        )),
    )
}

#[cfg(feature = "axon-pb")]
fn local_daemon_offline_error(socket_path: &std::path::Path) -> anyhow::Error {
    anyhow::Error::new(
        crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
            "daemon not running (local daemon Invocation endpoint unreachable at {}). \
             Start it with `easynet runtime start`.",
            socket_path.display()
        )),
    )
}

#[cfg(feature = "axon-pb")]
fn ensure_local_daemon_accepting() -> anyhow::Result<std::path::PathBuf> {
    let socket_path = daemon_config::resolved_local_uds_path_with_env_override();
    if !probe_accepting(&socket_path) {
        return Err(local_daemon_offline_error(&socket_path));
    }
    Ok(socket_path)
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_system_ability_targeted_root_timeout(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    subject_ura: &str,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let tuple_plan = LocalDaemonSystemTuplePlan::targeted_root_for_subject(
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        timeout,
    )?;
    invoke_local_daemon_ability_with_tuple_plan(tuple_plan)
}

/// Invoke one daemon-system ability through an explicitly attached daemon
/// Invocation endpoint and verify its terminal receipt before returning the
/// business payload.
///
/// This is the session-bound counterpart of the process-default local issuer.
/// C ABI handles use it when their `control.json` names a daemon other than the
/// process default; silently consulting the default socket would cross runtime
/// and tenant attachment boundaries.
#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_attached_daemon_system_ability_targeted_root_timeout(
    endpoint: PathBuf,
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    subject_ura: &str,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let tuple_plan = LocalDaemonSystemTuplePlan::targeted_root_for_subject(
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        timeout,
    )?;
    invoke_local_daemon_ability_with_tuple_plan_at_verified(endpoint, tuple_plan)
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_system_ability_targeted_root_with_authority_timeout(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    subject_ura: &str,
    authority_metadata: crate::daemon::invocation::admission::authority_metadata::IssuedAuthorityMetadata,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let tuple_plan = LocalDaemonSystemTuplePlan::targeted_root_for_subject(
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        timeout,
    )?
    .with_authority_metadata(authority_metadata);
    invoke_local_daemon_ability_with_tuple_plan(tuple_plan)
}

#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_targeted_explicit_causal_timeout(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    subject_ura: &str,
    invocation_nonce: [u8; 16],
    causal_context: axon_sdk::invocation::CausalContext,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    let tuple_plan = LocalDaemonSystemTuplePlan::targeted_explicit_causal(
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        invocation_nonce,
        public_causal_context_to_wire(&causal_context),
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
    let tuple_plan = LocalDaemonSystemTuplePlan::targeted_root_for_subject(
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        timeout,
    )?;
    invoke_local_daemon_ability_stream_with_tuple_plan(tuple_plan, max_frames)
}

#[cfg(feature = "axon-pb")]
#[expect(
    clippy::too_many_arguments,
    reason = "stable facade preserves the complete explicit invocation tuple for existing callers"
)]
pub(crate) fn invoke_local_daemon_ability_targeted_stream_explicit_causal(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    subject_ura: &str,
    invocation_nonce: [u8; 16],
    causal_context: axon_sdk::invocation::CausalContext,
    timeout: Duration,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    let tuple_plan = LocalDaemonSystemTuplePlan::targeted_explicit_causal(
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        invocation_nonce,
        public_causal_context_to_wire(&causal_context),
        timeout,
    )?;
    invoke_local_daemon_ability_stream_with_tuple_plan(tuple_plan, max_frames)
}

/// Open a daemon-hosted bidirectional ability through Axon's local
/// Invocation gRPC transport and drain JSON-frame down output.
#[cfg(feature = "axon-pb")]
pub(crate) fn invoke_local_daemon_ability_targeted_bidi_json_frames_explicit_causal(
    request: LocalDaemonTargetedBidiRequest<'_>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalBidiFrame>> {
    let LocalDaemonTargetedBidiRequest {
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        invocation_nonce,
        causal_context,
        timeout,
        input_frames,
        max_frames,
    } = request;
    let tuple_plan = LocalDaemonSystemTuplePlan::targeted_explicit_causal(
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        invocation_nonce,
        public_causal_context_to_wire(&causal_context),
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
    tuple_plan: LocalDaemonSystemTuplePlan,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    use anyhow::Context;
    let timeout = tuple_plan.timeout;

    let socket_path = ensure_local_daemon_accepting()?;

    let invocation = local_daemon_system_invocation_from_tuple_plan(tuple_plan)?;
    let function_name = invocation.function_name().to_string();
    let request = invocation.stream_request()?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for local daemon Invocation stream invoke")?;

    runtime.block_on(async move {
        let channel = connect_channel(socket_path.clone(), timeout, Duration::from_secs(10))
            .await
            .map_err(|source| local_daemon_connect_error(&socket_path, source))?;
        let mut client = crate::daemon::invocation::transport::invocation_client(channel);
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
    tuple_plan: LocalDaemonSystemTuplePlan,
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

    let socket_path = ensure_local_daemon_accepting()?;

    let invocation = local_daemon_system_invocation_from_tuple_plan(tuple_plan)?;
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
        .context("build tokio runtime for local daemon Invocation bidi invoke")?;

    runtime.block_on(async move {
        let channel = connect_channel(socket_path.clone(), timeout, Duration::from_secs(10))
            .await
            .map_err(|source| local_daemon_connect_error(&socket_path, source))?;
        let mut client = crate::daemon::invocation::transport::invocation_client(channel);

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

#[cfg(feature = "axon-pb")]
fn public_causal_context_to_wire(
    causal_context: &axon_sdk::invocation::CausalContext,
) -> axon_sdk::pb::axon::v1::CausalContext {
    use axon_sdk::invocation::CausalContext;
    use axon_sdk::pb::axon::v1 as pb;

    let receipt_ref_to_wire = |reference: &axon_sdk::invocation::ReceiptRef| pb::ReceiptRef {
        receipt_hash: reference.receipt_hash.to_vec(),
        receipt_ura: reference.receipt_ura.clone(),
    };

    pb::CausalContext {
        form: Some(match causal_context {
            CausalContext::None => pb::causal_context::Form::None(pb::Empty {}),
            CausalContext::Scalar(reference) => {
                pb::causal_context::Form::Scalar(receipt_ref_to_wire(reference))
            }
            CausalContext::List(prior) => pb::causal_context::Form::List(pb::ReceiptList {
                prior: prior.iter().map(receipt_ref_to_wire).collect(),
            }),
            CausalContext::Merkle { root, proof_ura } => {
                pb::causal_context::Form::Merkle(pb::MerkleRoot {
                    root: root.to_vec(),
                    proof_ura: proof_ura.clone(),
                })
            }
        }),
    }
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
        "streaming `{}` through the local daemon Invocation endpoint requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
}

#[cfg(not(feature = "axon-pb"))]
#[expect(
    clippy::too_many_arguments,
    reason = "feature-disabled facade must retain the same stable signature as the enabled implementation"
)]
pub(crate) fn invoke_local_daemon_ability_targeted_stream_explicit_causal(
    function_name: &str,
    _payload_json: serde_json::Value,
    _callee_ura: &str,
    _subject_ura: &str,
    _invocation_nonce: [u8; 16],
    _causal_context: axon_sdk::invocation::CausalContext,
    _timeout: Duration,
    _max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    anyhow::bail!(
        "streaming `{}` through the local daemon Invocation endpoint requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_targeted_bidi_json_frames_explicit_causal(
    request: LocalDaemonTargetedBidiRequest<'_>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalBidiFrame>> {
    let function_name = request.function_name;
    anyhow::bail!(
        "bidirectional `{}` through the local daemon Invocation endpoint requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
}

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_with_tuple_plan(
    tuple_plan: LocalDaemonSystemTuplePlan,
) -> anyhow::Result<serde_json::Value> {
    use anyhow::Context;
    let timeout = tuple_plan.timeout;

    let socket_path = ensure_local_daemon_accepting()?;

    let authority_metadata = tuple_plan.authority_metadata.clone();
    let invocation = local_daemon_system_invocation_from_tuple_plan(tuple_plan)?;
    let function_name = invocation.function_name().to_string();
    let mut request = invocation.invoke_request()?;
    if let Some(authority_metadata) = authority_metadata {
        request.metadata.insert(
            authority_metadata.key().to_string(),
            authority_metadata.value().to_string(),
        );
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for local daemon Invocation invoke")?;

    runtime.block_on(async move {
        let channel = connect_channel(socket_path.clone(), timeout, Duration::from_secs(10))
            .await
            .map_err(|source| local_daemon_connect_error(&socket_path, source))?;
        let mut client = crate::daemon::invocation::transport::invocation_client(channel);
        let (value, _) = invoke_local_daemon_json(&mut client, request, &function_name).await?;
        Ok(value)
    })
}

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_with_tuple_plan_at_verified(
    socket_path: PathBuf,
    tuple_plan: LocalDaemonSystemTuplePlan,
) -> anyhow::Result<serde_json::Value> {
    use anyhow::{anyhow, Context};

    let timeout = tuple_plan.timeout;
    let invocation = local_daemon_system_invocation_from_tuple_plan(tuple_plan)?;
    let function_name = invocation.function_name().to_string();
    let request = invocation.invoke_request()?;
    let submitted = SubmittedInvocationProjection::from_request(&request, &function_name)?;
    let thread_name = format!("easynet-receipt-key-resolve-{function_name}");
    let response = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("build tokio runtime for daemon delegated receipt key resolution")?;
            runtime.block_on(async move {
                let channel =
                    connect_channel(socket_path.clone(), timeout, Duration::from_secs(10))
                        .await
                        .map_err(|source| local_daemon_connect_error(&socket_path, source))?;
                let mut client = crate::daemon::invocation::transport::invocation_client(channel);
                let (value, response) =
                    invoke_local_daemon_json(&mut client, request, &function_name).await?;
                Ok::<_, anyhow::Error>((value, response, function_name))
            })
        })
        .map_err(|error| anyhow!("spawn daemon delegated receipt key resolver failed: {error}"))?
        .join()
        .map_err(|_| anyhow!("daemon delegated receipt key resolver panicked"))??;
    let (value, response, function_name) = response;
    let terminal = UnverifiedTerminalInvocationProjection::from_response(
        &response,
        &submitted,
        &function_name,
    )?
    .verify(&LocalKeyServiceReceiptResolver::new(), &function_name)?;
    record_verified_causal_anchor(&terminal.causal_anchor)?;
    Ok(value)
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
pub(crate) fn invoke_local_daemon_ability_targeted_stream_with_invocation_context(
    request: LocalDaemonTargetedInvocationMetaRequest<'_>,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    use anyhow::{anyhow, bail};
    use axon_sdk::pb::axon::v1 as pb;

    let LocalDaemonTargetedInvocationMetaRequest {
        function_name,
        payload_json,
        callee_ura,
        subject_ura,
        invocation_nonce,
        causal_parents,
        step_timeout,
        trace_id: _,
    } = request;
    let function_name = function_name.trim().to_string();
    if function_name.is_empty() {
        bail!("function_name must not be empty");
    }

    let receipt_refs = verified_receipt_refs_from_causal_parents(causal_parents)?;
    let mut refs = receipt_refs;
    let causal_form = match refs.len() {
        0 => pb::causal_context::Form::None(pb::Empty {}),
        1 => pb::causal_context::Form::Scalar(refs.remove(0)),
        _ => pb::causal_context::Form::List(pb::ReceiptList { prior: refs }),
    };
    let tuple_plan = LocalDaemonSystemTuplePlan::targeted_explicit_causal(
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
    invoke_local_daemon_ability_stream_with_tuple_plan(tuple_plan, max_frames)
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
    realm_trust: RealmReceiptTrustSource,
    local_self_identity: LocalKeyServiceReceiptResolver,
    daemon_federated_trust: Option<Arc<dyn axon_sdk::invocation::KeyResolver>>,
}

#[cfg(feature = "axon-pb")]
struct DaemonFederatedReceiptResolver {
    endpoint: PathBuf,
    timeout: Duration,
    cache: Mutex<HashMap<String, Vec<ed25519_dalek::VerifyingKey>>>,
}

#[cfg(feature = "axon-pb")]
enum RealmReceiptTrustSource {
    Loaded(crate::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver),
    Empty { path: PathBuf },
    Missing { path: PathBuf },
    LoadFailed { path: PathBuf, error: String },
}

#[cfg(feature = "axon-pb")]
impl RealmReceiptTrustSource {
    fn load(path: PathBuf) -> Self {
        match crate::daemon::trust::anchor::RealmTrustAnchor::load_with_state(&path) {
            Ok(crate::daemon::trust::anchor::RealmTrustAnchorLoadState::Loaded(anchor))
                if anchor.is_empty() =>
            {
                Self::Empty { path }
            }
            Ok(crate::daemon::trust::anchor::RealmTrustAnchorLoadState::Loaded(anchor)) => {
                Self::Loaded(
                    crate::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver::new(
                        crate::daemon::trust::cell::SharedTrustAnchor::new(Arc::new(anchor)),
                    ),
                )
            }
            Ok(crate::daemon::trust::anchor::RealmTrustAnchorLoadState::Missing { path }) => {
                Self::Missing { path }
            }
            Err(error) => Self::LoadFailed {
                path,
                error: error.to_string(),
            },
        }
    }

    fn unavailable_detail(&self) -> Option<String> {
        match self {
            Self::Loaded(_) => None,
            Self::Empty { path } => {
                Some(format!("realm trust anchor at {} is empty", path.display()))
            }
            Self::Missing { path } => Some(format!(
                "realm trust anchor at {} is missing",
                path.display()
            )),
            Self::LoadFailed { path, error } => Some(format!(
                "realm trust anchor at {} failed to load: {error}",
                path.display()
            )),
        }
    }

    fn resolve_all(
        &self,
        signer_ura: &str,
    ) -> Result<Vec<ed25519_dalek::VerifyingKey>, axon_sdk::invocation::AxonError> {
        match self {
            Self::Loaded(resolver) => {
                axon_sdk::invocation::KeyResolver::resolve_all(resolver, signer_ura)
            }
            source => {
                let detail = source
                    .unavailable_detail()
                    .expect("non-loaded realm trust source must explain unavailability");
                Err(axon_sdk::invocation::AxonError::permission_denied(
                    "runtime_receipt_realm_trust_unavailable",
                )
                .with_message(detail))
            }
        }
    }
}

#[cfg(feature = "axon-pb")]
impl DaemonFederatedReceiptResolver {
    fn new(endpoint: PathBuf) -> Self {
        Self {
            endpoint,
            timeout: Duration::from_secs(10),
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn resolve_keyset(
        &self,
        signer_ura: &str,
    ) -> Result<Vec<ed25519_dalek::VerifyingKey>, axon_sdk::invocation::AxonError> {
        let signer_ura = crate::core::identity::RuntimeIdentityUra::parse(signer_ura)
            .map(crate::core::identity::RuntimeIdentityUra::into_string)
            .map_err(|error| {
                delegated_receipt_key_error(signer_ura, format!("invalid signer URA: {error}"))
            })?;
        if let Some(keys) = self
            .cache
            .lock()
            .map_err(|_| {
                delegated_receipt_key_error(&signer_ura, "daemon receipt key cache is poisoned")
            })?
            .get(&signer_ura)
            .cloned()
        {
            return Ok(keys);
        }

        let keys = self.resolve_keyset_uncached(&signer_ura)?;
        self.cache
            .lock()
            .map_err(|_| {
                delegated_receipt_key_error(&signer_ura, "daemon receipt key cache is poisoned")
            })?
            .insert(signer_ura, keys.clone());
        Ok(keys)
    }

    fn resolve_keyset_uncached(
        &self,
        signer_ura: &str,
    ) -> Result<Vec<ed25519_dalek::VerifyingKey>, axon_sdk::invocation::AxonError> {
        let function_name = crate::daemon::ability::conformance::ABILITY_FEDERATION_RESOLVE_KEY;
        let callee_ura = local_daemon_identity_ura().map_err(|error| {
            delegated_receipt_key_error(
                signer_ura,
                format!("resolve local daemon identity: {error}"),
            )
        })?;
        let subject_ura = crate::core::ura::owner_ability_ura(&callee_ura, function_name)
            .ok_or_else(|| {
                delegated_receipt_key_error(
                    signer_ura,
                    format!("derive {function_name} subject for {callee_ura}"),
                )
            })?;
        let request = crate::daemon::federation::wire_contract::ResolveKeyRequest::new(signer_ura);
        let payload_json = serde_json::to_value(&request).map_err(|error| {
            delegated_receipt_key_error(signer_ura, format!("encode resolve_key request: {error}"))
        })?;
        let tuple_plan = LocalDaemonSystemTuplePlan::targeted_root_for_subject(
            function_name,
            payload_json,
            &callee_ura,
            &subject_ura,
            self.timeout,
        )
        .map_err(|error| {
            delegated_receipt_key_error(
                signer_ura,
                format!("build daemon delegated receipt key invocation: {error}"),
            )
        })?;
        let endpoint = self.endpoint.clone();
        let value = invoke_local_daemon_ability_with_tuple_plan_at_verified(endpoint, tuple_plan)
            .map_err(|error| {
            delegated_receipt_key_error(
                signer_ura,
                format!("daemon delegated {function_name} failed: {error:#}"),
            )
        })?;
        let response: crate::daemon::federation::wire_contract::ResolveKeyResponse =
            serde_json::from_value(value).map_err(|error| {
                delegated_receipt_key_error(
                    signer_ura,
                    format!("daemon delegated resolve_key response schema invalid: {error}"),
                )
            })?;
        let mut keys = Vec::new();
        for (index, public_key_b64) in response
            .public_keys_b64
            .iter()
            .take(axon_sdk::invocation::MAX_KEYS_PER_AGENT_URA)
            .enumerate()
        {
            keys.push(decode_delegated_receipt_pubkey(
                signer_ura,
                public_key_b64,
                index,
            )?);
        }
        if keys.is_empty() {
            keys.push(decode_delegated_receipt_pubkey(
                signer_ura,
                &response.public_key_b64,
                0,
            )?);
        }
        Ok(keys)
    }
}

#[cfg(feature = "axon-pb")]
impl axon_sdk::invocation::KeyResolver for DaemonFederatedReceiptResolver {
    fn resolve(
        &self,
        signer_ura: &str,
    ) -> Result<ed25519_dalek::VerifyingKey, axon_sdk::invocation::AxonError> {
        self.resolve_keyset(signer_ura)?
            .into_iter()
            .next()
            .ok_or_else(|| {
                delegated_receipt_key_error(signer_ura, "daemon returned no receipt signer keys")
            })
    }

    fn resolve_all(
        &self,
        signer_ura: &str,
    ) -> Result<Vec<ed25519_dalek::VerifyingKey>, axon_sdk::invocation::AxonError> {
        self.resolve_keyset(signer_ura)
    }
}

#[cfg(feature = "axon-pb")]
fn delegated_receipt_key_error(
    signer_ura: &str,
    detail: impl Into<String>,
) -> axon_sdk::invocation::AxonError {
    axon_sdk::invocation::AxonError::permission_denied("runtime_receipt_signer_key_untrusted")
        .with_message(format!(
            "daemon delegated receipt trust cannot resolve signer {signer_ura:?}: {}",
            detail.into()
        ))
}

#[cfg(feature = "axon-pb")]
fn decode_delegated_receipt_pubkey(
    signer_ura: &str,
    public_key_b64: &str,
    index: usize,
) -> Result<ed25519_dalek::VerifyingKey, axon_sdk::invocation::AxonError> {
    use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};

    let raw = B64_STANDARD
        .decode(public_key_b64.as_bytes())
        .map_err(|error| {
            delegated_receipt_key_error(
                signer_ura,
                format!("public_keys_b64[{index}] base64 invalid: {error}"),
            )
        })?;
    let bytes: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
        delegated_receipt_key_error(
            signer_ura,
            format!(
                "public_keys_b64[{index}] is {} bytes; expected 32",
                raw.len()
            ),
        )
    })?;
    ed25519_dalek::VerifyingKey::from_bytes(&bytes).map_err(|error| {
        delegated_receipt_key_error(
            signer_ura,
            format!("public_keys_b64[{index}] is not a valid Ed25519 point: {error}"),
        )
    })
}

#[cfg(feature = "axon-pb")]
impl CanonicalRuntimeReceiptResolver {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self::with_daemon_federated_trust(None)
    }

    pub(crate) fn for_daemon_endpoint(endpoint: PathBuf) -> Self {
        Self::with_daemon_federated_trust(Some(Arc::new(DaemonFederatedReceiptResolver::new(
            endpoint,
        ))))
    }

    fn with_daemon_federated_trust(
        daemon_federated_trust: Option<Arc<dyn axon_sdk::invocation::KeyResolver>>,
    ) -> Self {
        let trust_anchor_path =
            crate::daemon::trust::anchor::trust_anchor_path_from_env_or_default();
        Self {
            realm_trust: RealmReceiptTrustSource::load(trust_anchor_path),
            local_self_identity: LocalKeyServiceReceiptResolver::new(),
            daemon_federated_trust,
        }
    }

    #[cfg(test)]
    fn with_test_delegated_trust(delegated: Arc<dyn axon_sdk::invocation::KeyResolver>) -> Self {
        Self::with_daemon_federated_trust(Some(delegated))
    }
}

#[cfg(feature = "axon-pb")]
impl axon_sdk::invocation::KeyResolver for CanonicalRuntimeReceiptResolver {
    fn resolve(
        &self,
        signer_ura: &str,
    ) -> Result<ed25519_dalek::VerifyingKey, axon_sdk::invocation::AxonError> {
        self.resolve_all(signer_ura)?
            .into_iter()
            .next()
            .ok_or_else(|| {
                delegated_receipt_key_error(
                    signer_ura,
                    "canonical runtime trust returned no signer keys",
                )
            })
    }

    fn resolve_all(
        &self,
        signer_ura: &str,
    ) -> Result<Vec<ed25519_dalek::VerifyingKey>, axon_sdk::invocation::AxonError> {
        let local_error = match axon_sdk::invocation::KeyResolver::resolve_all(
            &self.local_self_identity,
            signer_ura,
        ) {
            Ok(keys) if !keys.is_empty() => return Ok(keys),
            Ok(_) => "local self identity returned no signer keys".to_string(),
            Err(error) => error.to_string(),
        };
        let realm_error = match self.realm_trust.resolve_all(signer_ura) {
            Ok(keys) if !keys.is_empty() => return Ok(keys),
            Ok(_) => "realm trust anchor returned no signer keys".to_string(),
            Err(error) => error.to_string(),
        };
        if let Some(daemon_federated_trust) = self.daemon_federated_trust.as_ref() {
            match daemon_federated_trust.resolve_all(signer_ura) {
                Ok(keys) if !keys.is_empty() => return Ok(keys),
                Ok(_) => {
                    let daemon_error = "daemon delegated trust returned no signer keys";
                    return Err(axon_sdk::invocation::AxonError::permission_denied(
                        "runtime_receipt_signer_key_untrusted",
                    )
                    .with_message(format!(
                        "canonical runtime trust cannot resolve receipt signer {signer_ura:?}: \
                         local_self_identity={local_error}; realm_trust={realm_error}; \
                         daemon_federated_trust={daemon_error}"
                    )));
                }
                Err(daemon_error) => {
                    return Err(axon_sdk::invocation::AxonError::permission_denied(
                        "runtime_receipt_signer_key_untrusted",
                    )
                    .with_message(format!(
                        "canonical runtime trust cannot resolve receipt signer {signer_ura:?}: \
                         local_self_identity={local_error}; realm_trust={realm_error}; \
                         daemon_federated_trust={daemon_error}"
                    )));
                }
            }
        }
        Err(axon_sdk::invocation::AxonError::permission_denied(
            "runtime_receipt_signer_key_untrusted",
        )
        .with_message(format!(
            "canonical runtime trust cannot resolve receipt signer {signer_ura:?}: \
             local_self_identity={local_error}; realm_trust={realm_error}; \
             daemon_federated_trust=not configured"
        )))
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug)]
struct SubmittedInvocationProjection {
    envelope: axon_sdk::pb::axon::v1::Envelope,
    function_name: String,
    arguments_json: serde_json::Value,
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
        let arguments_json = serde_json::from_slice(&request.arguments).map_err(|error| {
            anyhow::anyhow!("{ability}: submitted invocation args are not JSON: {error}")
        })?;
        Ok(Self {
            envelope,
            function_name:
                crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
                    "local daemon submitted invocation",
                    request.target.as_ref(),
                )?
                .to_string(),
            arguments_json,
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

        let checkpoint_proof =
            finalization_checkpoint_proof_json(&self.admission_receipt, &self.terminal_receipt)?;
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
            "verification_checkpoints": checkpoint_proof,
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
fn finalization_checkpoint_proof_json(
    admission: &axon_sdk::pb::axon::v1::InvocationReceipt,
    terminal: &axon_sdk::pb::axon::v1::InvocationReceipt,
) -> anyhow::Result<serde_json::Value> {
    use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};
    use prost::Message as _;

    fn encode_receipt(
        receipt: &axon_sdk::pb::axon::v1::InvocationReceipt,
    ) -> anyhow::Result<String> {
        let mut bytes = Vec::with_capacity(receipt.encoded_len());
        receipt.encode(&mut bytes)?;
        Ok(B64_STANDARD.encode(bytes))
    }

    Ok(serde_json::json!({
        "encoding": "prost.base64",
        "admission_receipt_b64": encode_receipt(admission)?,
        "terminal_receipt_b64": encode_receipt(terminal)?,
    }))
}

#[cfg(feature = "axon-pb")]
fn decode_checkpoint_receipt_b64(
    value: &serde_json::Value,
    field: &'static str,
) -> anyhow::Result<axon_sdk::pb::axon::v1::InvocationReceipt> {
    use anyhow::Context;
    use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};
    use prost::Message as _;

    let encoded = value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("verified invocation proof missing {field}"))?;
    let bytes = B64_STANDARD
        .decode(encoded)
        .with_context(|| format!("decode verified invocation proof {field}"))?;
    axon_sdk::pb::axon::v1::InvocationReceipt::decode(bytes.as_slice())
        .with_context(|| format!("decode verified invocation proof {field} as InvocationReceipt"))
}

#[cfg(feature = "axon-pb")]
pub(crate) fn import_verified_causal_parent_from_invocation_meta(
    metadata: &serde_json::Value,
    expected_ability: &str,
    expected_subject_ura: &str,
) -> anyhow::Result<serde_json::Value> {
    import_verified_causal_parent_from_invocation_meta_with_resolver(
        metadata,
        expected_ability,
        expected_subject_ura,
        &LocalKeyServiceReceiptResolver::new(),
    )
}

#[cfg(feature = "axon-pb")]
fn import_verified_causal_parent_from_invocation_meta_with_resolver(
    metadata: &serde_json::Value,
    expected_ability: &str,
    expected_subject_ura: &str,
    resolver: &dyn axon_sdk::invocation::KeyResolver,
) -> anyhow::Result<serde_json::Value> {
    use anyhow::{anyhow, bail};
    use axon_sdk::invocation::InvocationState;

    let expected_ability = expected_ability.trim();
    if expected_ability.is_empty() {
        bail!("verified invocation import expected_ability must not be empty");
    }
    let expected_subject_ura = expected_subject_ura.trim();
    if expected_subject_ura.is_empty() {
        bail!("verified invocation import expected_subject_ura must not be empty");
    }

    if let Some(projected_ability) = metadata
        .get("ability")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if projected_ability != expected_ability {
            bail!(
                "verified invocation proof ability {projected_ability:?} does not match expected {expected_ability:?}"
            );
        }
    }

    let proof = metadata
        .get("receipt")
        .and_then(|receipt| receipt.get("verification_checkpoints"))
        .ok_or_else(|| {
            anyhow!("verified invocation metadata missing receipt.verification_checkpoints")
        })?;
    let encoding = proof
        .get("encoding")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("verified invocation proof missing encoding"))?;
    if encoding != "prost.base64" {
        bail!("unsupported verified invocation proof encoding {encoding:?}");
    }

    let admission_receipt = decode_checkpoint_receipt_b64(proof, "admission_receipt_b64")?;
    let terminal_receipt = decode_checkpoint_receipt_b64(proof, "terminal_receipt_b64")?;

    let terminal_state = InvocationState::try_from(terminal_receipt.state)
        .map_err(|error| anyhow!("verified invocation proof terminal state invalid: {error}"))?;
    if terminal_state != InvocationState::Completed {
        bail!("verified invocation proof terminal state must be Completed, got {terminal_state:?}");
    }
    let terminal_subject = terminal_receipt
        .subject_binding
        .as_ref()
        .map(|binding| binding.ura.trim())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("verified invocation proof terminal receipt omitted subject"))?;
    if terminal_subject != expected_subject_ura {
        bail!(
            "verified invocation proof subject {terminal_subject:?} does not match expected {expected_subject_ura:?}"
        );
    }
    let ability_ura =
        axon_sdk::invocation::ability_ura_from_descriptor_ref(&terminal_receipt.ability_binding)
            .map_err(|error| {
                anyhow!("verified invocation proof ability binding is invalid: {error}")
            })?;
    let public_name = axon_sdk::ura::qualified_ability_name(ability_ura)
        .ok_or_else(|| anyhow!("verified invocation proof ability binding has no public name"))?;
    if public_name != expected_ability {
        bail!(
            "verified invocation proof terminal ability {public_name:?} does not match expected {expected_ability:?}"
        );
    }
    if let Some(request_id) = metadata
        .get("request_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if request_id != terminal_receipt.invocation_id {
            bail!(
                "verified invocation proof request_id {request_id:?} does not match terminal receipt invocation {:?}",
                terminal_receipt.invocation_id
            );
        }
    }

    let verified = UnverifiedTerminalInvocationProjection {
        state: "completed",
        admission_receipt,
        terminal_receipt,
    }
    .verify(resolver, expected_ability)?;
    record_verified_causal_anchor(&verified.causal_anchor)?;
    Ok(verified.causal_anchor.projection())
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn import_verified_causal_parent_from_invocation_meta(
    _metadata: &serde_json::Value,
    expected_ability: &str,
    _expected_subject_ura: &str,
) -> anyhow::Result<serde_json::Value> {
    anyhow::bail!(
        "{expected_ability}: verified invocation proof import requires the axon-pb provider"
    )
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

    let socket_path = ensure_local_daemon_accepting()?;

    let receipt_refs = verified_receipt_refs_from_causal_parents(causal_parents)?;
    let mut refs = receipt_refs;
    let causal_form = match refs.len() {
        0 => pb::causal_context::Form::None(pb::Empty {}),
        1 => pb::causal_context::Form::Scalar(refs.remove(0)),
        _ => pb::causal_context::Form::List(pb::ReceiptList { prior: refs }),
    };
    let tuple_plan = LocalDaemonSystemTuplePlan::targeted_explicit_causal(
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
    let submitted_subject_ura = tuple_plan.subject_policy.resolve()?;
    let invocation =
        local_daemon_system_invocation_from_tuple_plan(tuple_plan)?.with_trace_id(trace_id);
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
        .context("build tokio runtime for local daemon Invocation invoke")?;

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
        "args": submitted.arguments_json,
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
pub(crate) fn invoke_local_daemon_ability_targeted_stream_with_invocation_context(
    request: LocalDaemonTargetedInvocationMetaRequest<'_>,
    _max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::platform::local_invoke::LocalStreamFrame>> {
    let function_name = request.function_name;
    anyhow::bail!(
        "streaming `{}` through the local daemon Invocation endpoint requires the `axon-pb` feature; \
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
pub(crate) fn invoke_local_daemon_system_ability_targeted_root_timeout(
    function_name: &str,
    _payload_json: serde_json::Value,
    _callee_ura: &str,
    _subject_ura: &str,
    _timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    Err(anyhow::Error::new(
        crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
            "invoking targeted `{function_name}` through the local daemon Invocation endpoint requires the \
             `axon-pb` feature; rebuild with `cargo build --features axon-pb`"
        )),
    ))
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_system_ability_targeted_root_with_authority_timeout(
    function_name: &str,
    _payload_json: serde_json::Value,
    _callee_ura: &str,
    _subject_ura: &str,
    _authority_metadata: crate::daemon::invocation::admission::authority_metadata::IssuedAuthorityMetadata,
    _timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    Err(anyhow::Error::new(
        crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
            "invoking authority-bound `{function_name}` through the local daemon Invocation endpoint requires the \
             `axon-pb` feature; rebuild with `cargo build --features axon-pb`"
        )),
    ))
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_targeted_explicit_causal_timeout(
    function_name: &str,
    _payload_json: serde_json::Value,
    _callee_ura: &str,
    _subject_ura: &str,
    _invocation_nonce: [u8; 16],
    _causal_context: axon_sdk::invocation::CausalContext,
    _timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    Err(anyhow::Error::new(
        crate::support::platform::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
            "invoking explicit-root `{function_name}` through the local daemon Invocation endpoint requires the \
             `axon-pb` feature; rebuild with `cargo build --features axon-pb`"
        )),
    ))
}

#[cfg(feature = "axon-pb")]
fn local_daemon_system_caller_ura() -> anyhow::Result<String> {
    Ok(crate::daemon::identity::local_invocation::LOCAL_SYSTEM_AGENT_URA.to_string())
}

#[cfg(feature = "axon-pb")]
fn local_daemon_identity_ura() -> anyhow::Result<String> {
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

    struct StaticDelegatedReceiptResolver {
        signer_ura: String,
        keys: Vec<VerifyingKey>,
    }

    impl axon_sdk::invocation::KeyResolver for StaticDelegatedReceiptResolver {
        fn resolve(&self, signer_ura: &str) -> Result<VerifyingKey, AxonError> {
            self.resolve_all(signer_ura)?
                .into_iter()
                .next()
                .ok_or_else(|| AxonError::permission_denied("static_delegated_receipt_key_empty"))
        }

        fn resolve_all(&self, signer_ura: &str) -> Result<Vec<VerifyingKey>, AxonError> {
            if signer_ura == self.signer_ura {
                return Ok(self.keys.clone());
            }
            Err(AxonError::permission_denied(
                "static_delegated_receipt_key_not_found",
            ))
        }
    }

    #[test]
    fn local_system_invoke_request_does_not_pre_resolve_descriptor_ref() {
        let tuple_plan = LocalDaemonSystemTuplePlan::new(
            "discover",
            serde_json::json!({"query": "capabilities"}),
            LocalDaemonSystemCalleePolicy::explicit("easynet:///r/default/agent/dev.worker")
                .expect("callee policy"),
            LocalDaemonSystemSubjectPolicy::required_explicit(
                "easynet:///r/default/resource/daemon.local/catalog/discover",
            )
            .expect("subject policy"),
            LocalDaemonSystemDerivationPolicy::fresh_root(),
            Duration::from_secs(5),
        )
        .expect("tuple plan");
        let invocation = local_daemon_system_invocation_from_tuple_plan(tuple_plan)
            .expect("local system invocation projection");
        let request = invocation.invoke_request().expect("local system request");

        assert_eq!(
            crate::daemon::invocation::dispatch::invocation_wire::function_name_from_invocation_target(
                "test local system request",
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
            "daemon local system requests are promoted to LocalSystem inside the daemon dispatcher"
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
            Some("easynet:///r/default/resource/daemon.local/catalog/discover")
        );
    }

    #[test]
    fn local_system_tuple_plan_requires_explicit_targeted_subject() {
        let tuple_plan = LocalDaemonSystemTuplePlan::targeted_root_for_subject(
            "job.run",
            serde_json::json!({"job": 1}),
            "easynet:///r/acme/device/edge-1",
            "easynet:///r/acme/resource/user.jobs/job-1",
            Duration::from_secs(5),
        )
        .expect("tuple plan");
        assert_eq!(
            tuple_plan.subject_policy,
            LocalDaemonSystemSubjectPolicy::Explicit(
                "easynet:///r/acme/resource/user.jobs/job-1".to_string()
            )
        );

        let invocation = local_daemon_system_invocation_from_tuple_plan(tuple_plan)
            .expect("local system invocation");
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

        assert!(LocalDaemonSystemTuplePlan::targeted_root_for_subject(
            "job.run",
            serde_json::json!({"job": 1}),
            "easynet:///r/acme/device/edge-1",
            "",
            Duration::from_secs(5),
        )
        .is_err());
    }

    #[test]
    fn local_system_tuple_plan_rejects_all_zero_principal_before_transport() {
        let placeholder = "00000000-0000-0000-0000-000000000000";
        let subject = "easynet:///r/acme/resource/user.jobs/job-1";

        for (field, callee, candidate_subject) in [
            (
                "callee_ura",
                crate::core::ura::user_ura("acme", placeholder),
                subject.to_string(),
            ),
            (
                "subject_ura",
                "easynet:///r/acme/device/edge-1".to_string(),
                crate::core::ura::resource_dot_ura("acme", &format!("user.{placeholder}"), "job-1"),
            ),
        ] {
            let error = LocalDaemonSystemTuplePlan::targeted_root_for_subject(
                "job.run",
                serde_json::json!({"job": 1}),
                &callee,
                &candidate_subject,
                Duration::from_secs(5),
            )
            .expect_err("all-zero principal must fail before local daemon-system transport");
            let message = error.to_string();
            assert!(
                message.contains(field) && message.contains("all-zero principal placeholder"),
                "wrong {field} error: {message}"
            );
        }
    }

    #[test]
    fn local_system_tuple_plan_preserves_explicit_causal_context() {
        let parent = axon_sdk::pb::axon::v1::ReceiptRef {
            receipt_ura: "easynet:///r/acme/resource/user.jobs/job-1/invocation/i1/receipt/1"
                .to_string(),
            receipt_hash: [7_u8; 32].to_vec(),
        };
        let tuple_plan = LocalDaemonSystemTuplePlan::targeted_explicit_causal(
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

        let LocalDaemonSystemDerivationPolicy::ExplicitCausal {
            invocation_nonce, ..
        } = &tuple_plan.derivation_policy
        else {
            panic!("expected explicit causal derivation");
        };
        assert_eq!(*invocation_nonce, [9_u8; 16]);

        let invocation = local_daemon_system_invocation_from_tuple_plan(tuple_plan)
            .expect("local system invocation");
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

        assert!(LocalDaemonSystemTuplePlan::targeted_explicit_causal(
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

    #[test]
    fn submitted_invocation_projection_preserves_json_args_bound_by_input_hash() {
        let tuple_plan = LocalDaemonSystemTuplePlan::targeted_root_for_subject(
            "job.run",
            serde_json::json!({
                "job": 1,
                "mode": "view_only",
                "consent_ticket": "ticket-1"
            }),
            "easynet:///r/acme/device/edge-1",
            "easynet:///r/acme/resource/user.jobs/job-1",
            Duration::from_secs(5),
        )
        .expect("tuple plan");
        let invocation = local_daemon_system_invocation_from_tuple_plan(tuple_plan)
            .expect("local system invocation");
        let request = invocation.invoke_request().expect("invoke request");
        let submitted =
            SubmittedInvocationProjection::from_request(&request, "job.run").expect("submitted");

        assert_eq!(
            submitted.arguments_json,
            serde_json::json!({
                "job": 1,
                "mode": "view_only",
                "consent_ticket": "ticket-1"
            })
        );
        assert_eq!(
            submitted.input_hash,
            axon_sdk::invocation::sha256(&request.arguments)
        );
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

        let tuple_plan = LocalDaemonSystemTuplePlan::targeted_root_for_subject(
            "job.run",
            serde_json::json!({"job": 1}),
            "easynet:///r/acme/device/edge-1",
            "easynet:///r/acme/resource/user.jobs/job-1",
            Duration::from_secs(5),
        )
        .expect("tuple plan");
        let invocation = local_daemon_system_invocation_from_tuple_plan(tuple_plan)
            .expect("local system invocation");
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
        let dispatch =
            crate::daemon::axon_bridge::descriptor_bound_dispatch::local_system_from_wire_parts(
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
                let outcome =
                    crate::daemon::axon_bridge::descriptor_bound_dispatch::dispatch_rpc_admitted(
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
    fn verified_invocation_metadata_proof_can_rehydrate_causal_parent() {
        let (submitted, response, signing_key) =
            completed_receipt_response_fixture(0x34, "inv-rehydrated-proof");
        let resolver = fixture_resolver(&response, &signing_key);
        let projection =
            UnverifiedTerminalInvocationProjection::from_response(&response, &submitted, "job.run")
                .expect("well-formed unverified projection")
                .verify(&resolver, "job.run")
                .expect("cryptographically verified finalization projection");
        let parent = causal_parent_claim(&response);
        let error = verified_receipt_refs_from_causal_parents(&[parent.clone()])
            .expect_err("fixture parent must not be accepted before proof import");
        assert!(error
            .to_string()
            .contains("was not cryptographically verified"));

        let metadata = serde_json::json!({
            "ability": "job.run",
            "request_id": projection.invocation_id,
            "receipt": projection.receipt,
        });
        let imported = import_verified_causal_parent_from_invocation_meta_with_resolver(
            &metadata,
            "job.run",
            &projection.subject_ura,
            &resolver,
        )
        .expect("verified metadata proof imports causal parent");

        assert_eq!(imported["receipt_ura"], parent["receipt_ura"]);
        assert_eq!(imported["receipt_hash"], parent["receipt_hash"]);
        let refs = verified_receipt_refs_from_causal_parents(&[parent])
            .expect("imported proof restores causal receipt capability");
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
    fn canonical_receipt_resolver_preserves_malformed_realm_trust_source() {
        let _guard = crate::cli::commands::test_support::env_lock();
        let previous = std::env::var_os("EASYNET_REALM_TRUST_PATH");
        let trust = tempfile::NamedTempFile::new().expect("trust file");
        std::fs::write(
            trust.path(),
            r#"
[[trusted_agent]]
agent_ura = "easynet:///r/local/authority"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "not_a_role"
added_at_unix_ms = 1
"#,
        )
        .expect("write malformed trust anchor");
        std::env::set_var("EASYNET_REALM_TRUST_PATH", trust.path());

        let resolver = CanonicalRuntimeReceiptResolver::new();
        let error =
            axon_sdk::invocation::KeyResolver::resolve(&resolver, "easynet:///r/local/authority")
                .expect_err("malformed realm trust source must fail closed");
        let message = error.to_string();

        match previous {
            Some(value) => std::env::set_var("EASYNET_REALM_TRUST_PATH", value),
            None => std::env::remove_var("EASYNET_REALM_TRUST_PATH"),
        }

        assert!(
            message.contains("realm trust anchor at")
                && message.contains("failed to load")
                && message.contains("not_a_role"),
            "malformed trust source was not preserved: {message}"
        );
        assert!(
            !message.contains("empty or unavailable"),
            "malformed trust source must not collapse to legacy availability wording: {message}"
        );
    }

    #[test]
    fn canonical_receipt_resolver_preserves_missing_realm_trust_source() {
        let _guard = crate::cli::commands::test_support::env_lock();
        let previous = std::env::var_os("EASYNET_REALM_TRUST_PATH");
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing-realm-trust.toml");
        std::env::set_var("EASYNET_REALM_TRUST_PATH", &missing);

        let resolver = CanonicalRuntimeReceiptResolver::new();
        let error =
            axon_sdk::invocation::KeyResolver::resolve(&resolver, "easynet:///r/local/authority")
                .expect_err("missing realm trust source must fail closed");
        let message = error.to_string();

        match previous {
            Some(value) => std::env::set_var("EASYNET_REALM_TRUST_PATH", value),
            None => std::env::remove_var("EASYNET_REALM_TRUST_PATH"),
        }

        assert!(
            message.contains("realm trust anchor at") && message.contains("is missing"),
            "missing trust source was not preserved: {message}"
        );
        assert!(
            !message.contains("empty or unavailable"),
            "missing trust source must not collapse to legacy availability wording: {message}"
        );
    }

    #[test]
    fn canonical_receipt_resolver_uses_delegated_trust_after_local_and_realm_miss() {
        let _guard = crate::cli::commands::test_support::env_lock();
        let previous = std::env::var_os("EASYNET_REALM_TRUST_PATH");
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing-realm-trust.toml");
        std::env::set_var("EASYNET_REALM_TRUST_PATH", &missing);
        let signer_ura = "easynet:///r/peer/device/dev-1".to_string();
        let key_a = SigningKey::from_bytes(&[7u8; 32]).verifying_key();
        let key_b = SigningKey::from_bytes(&[8u8; 32]).verifying_key();
        let delegated = Arc::new(StaticDelegatedReceiptResolver {
            signer_ura: signer_ura.clone(),
            keys: vec![key_a, key_b],
        });
        let resolver = CanonicalRuntimeReceiptResolver::with_test_delegated_trust(delegated);

        let resolved = axon_sdk::invocation::KeyResolver::resolve_all(&resolver, &signer_ura)
            .expect("delegated receipt trust should resolve remote signer keyset");

        match previous {
            Some(value) => std::env::set_var("EASYNET_REALM_TRUST_PATH", value),
            None => std::env::remove_var("EASYNET_REALM_TRUST_PATH"),
        }

        assert_eq!(resolved, vec![key_a, key_b]);
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
