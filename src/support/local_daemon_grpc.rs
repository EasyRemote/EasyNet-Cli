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
use crate::runtime::ability::{
    HostedAgentDelegationRequest, HOSTED_AGENT_DELEGATION_REQUEST_METADATA_KEY,
};

/// Resolve the local daemon Invocation endpoint. Thin re-export of
/// [`crate::persistence::daemon_config::resolved_local_uds_path_with_env_override`]
/// kept here so the existing CLI call sites
/// (`cli/federation_discover.rs`, `cli/start.rs`,
/// `support/federation_invoke.rs`) need no rewrite. The body itself
/// lives in `persistence/` because it consults `daemon-config.toml`
/// — keeping it there preserves the `support/` leaf-layer invariant
/// documented in `src/support/mod.rs`.
pub(crate) fn resolve_socket_path() -> PathBuf {
    crate::persistence::daemon_config::resolved_local_uds_path_with_env_override()
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
            .block_on(crate::support::named_pipe::connect_with_retry(
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
                    let stream =
                        crate::support::named_pipe::connect_with_retry(&pipe_name, connect_timeout)
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
pub(crate) struct LocalDaemonAbilityClient {
    #[cfg(all(test, feature = "axon-pb"))]
    transport: LocalDaemonAbilityTransport,
}

#[cfg(all(test, feature = "axon-pb"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocalDaemonAbilityTransport {
    Grpc,
    InProcessAgentManagement,
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalDaemonSubjectPolicy {
    Explicit(String),
    DeclaredDefault(String),
    SelfTarget,
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
        crate::ura::parse_ura(value)
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
impl LocalDaemonAbilityClient {
    fn grpc() -> Self {
        Self {
            #[cfg(test)]
            transport: LocalDaemonAbilityTransport::Grpc,
        }
    }

    pub(crate) fn new() -> anyhow::Result<Self> {
        Self::validate_socket()?;
        Ok(Self::grpc())
    }

    fn require_host_device_ura(caller_ura: impl Into<String>) -> anyhow::Result<()> {
        let caller_ura = caller_ura.into();
        if caller_ura.trim().is_empty() {
            anyhow::bail!("host device URI is empty; cannot invoke local daemon abilities");
        }
        normalized_local_daemon_ura(caller_ura.trim(), "host_device_ura")?;
        Ok(())
    }

    /// Constructor for CLI agent-management commands.
    ///
    /// **Production semantics** (release builds): requires a paired
    /// device caller URA and a live daemon gRPC listener. Without
    /// either, returns an `Err` whose message tells the operator to
    /// pair/start the daemon. This is the ONLY production behaviour.
    ///
    /// **Test seam** (`cfg(test)` only): if the caller URA is
    /// missing OR the daemon socket is not reachable, falls back to
    /// `for_agent_management_in_process_test_only` — an in-process
    /// catalog wired with just the agent-list + agent-lifecycle
    /// abilities. Lets unit tests exercise `easynet agent …` CLI
    /// surfaces without spinning up a real daemon. Production
    /// builds never see this branch: the
    /// `LocalDaemonAbilityTransport::InProcessAgentManagement`
    /// variant is `cfg(all(test, feature = "axon-pb"))`, so the
    /// fallback constructor below cannot be reached from a release
    /// binary even if the gating accidentally drifted on one site.
    pub(crate) fn for_agent_management(caller_ura: Option<String>) -> anyhow::Result<Self> {
        let trimmed = caller_ura
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        #[cfg(test)]
        {
            if let Some(caller_ura) = trimmed {
                if Self::require_host_device_ura(caller_ura).is_ok()
                    && Self::validate_socket().is_ok()
                {
                    return Ok(Self::grpc());
                }
            }
            Ok(Self::for_agent_management_in_process_test_only())
        }

        #[cfg(not(test))]
        {
            let caller_ura = trimmed.ok_or_else(|| {
                anyhow::anyhow!(
                    "local daemon caller URI is unavailable; pair/start this device before \
                     using agent management"
                )
            })?;
            Self::require_host_device_ura(caller_ura)?;
            Self::validate_socket()?;
            Ok(Self::grpc())
        }
    }

    /// `cfg(test)`-only in-process flavour. Tests that need to
    /// pin in-process behaviour explicitly call this constructor
    /// rather than relying on `for_agent_management`'s implicit
    /// fallback; the name makes "this is a test seam" obvious to
    /// the next reader of the test code.
    #[cfg(test)]
    fn for_agent_management_in_process_test_only() -> Self {
        Self {
            transport: LocalDaemonAbilityTransport::InProcessAgentManagement,
        }
    }

    pub(crate) fn invoke(
        &self,
        function_name: &str,
        payload_json: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        self.invoke_with_subject_policy(
            function_name,
            payload_json,
            LocalDaemonSubjectPolicy::SelfTarget,
            Duration::from_secs(30),
        )
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
        #[cfg(test)]
        if self.transport == LocalDaemonAbilityTransport::InProcessAgentManagement {
            let _ = subject_policy;
            return invoke_agent_management_in_process(function_name, payload_json);
        }

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
                crate::support::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
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
        anyhow::bail!(
            "invoking daemon-hosted Axon abilities requires the `axon-pb` feature; \
             rebuild with `cargo build --features axon-pb`"
        )
    }

    pub(crate) fn for_agent_management(_caller_ura: Option<String>) -> anyhow::Result<Self> {
        #[cfg(test)]
        {
            Ok(Self {})
        }
        #[cfg(not(test))]
        anyhow::bail!(
            "agent management requires the daemon-hosted Axon ability surface; rebuild with \
             --features axon-pb"
        )
    }

    pub(crate) fn invoke(
        &self,
        function_name: &str,
        _payload_json: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        #[cfg(test)]
        {
            invoke_agent_management_in_process(function_name, _payload_json)
        }
        #[cfg(not(test))]
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

#[cfg(test)]
fn invoke_agent_management_in_process(
    function_name: &str,
    payload_json: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let mut catalog = crate::runtime::ability_dispatch::AxonAbilityCatalog::new();
    crate::daemon::ability::builtins::agents::list::register(&mut catalog, || {
        crate::registry::agents::load_agents().unwrap_or_default()
    });
    let hot_registrar: std::sync::Arc<
        crate::daemon::ability::builtins::agents::lifecycle::SharedHotRegistrarCell,
    > = std::sync::Arc::new(std::sync::OnceLock::new());
    crate::daemon::ability::builtins::agents::lifecycle::register(&mut catalog, hot_registrar);
    catalog.invoke_rpc_json(function_name, payload_json)
}

/// Invoke a daemon-hosted ability through Axon's local Invocation
/// gRPC transport (`daemon.sock`). Transport-level entry; CLI
/// surfaces MUST go through [`crate::support::local_invoke::invoke_local_ability_with_subject`]
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
) -> anyhow::Result<Vec<crate::support::local_invoke::LocalStreamFrame>> {
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
    function_name: &str,
    payload_json: serde_json::Value,
    callee_ura: &str,
    default_subject_ura: &str,
    subject: Option<String>,
    timeout: Duration,
    input_frames: Vec<serde_json::Value>,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::local_invoke::LocalBidiFrame>> {
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
) -> anyhow::Result<Vec<crate::support::local_invoke::LocalStreamFrame>> {
    use anyhow::Context;

    let socket_path = resolve_socket_path();
    if !probe_accepting(&socket_path) {
        return Err(anyhow::Error::new(
            crate::support::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
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
            .with_context(|| {
                format!(
                    "connect to local Axon daemon gRPC endpoint at {}",
                    socket_path.display()
                )
            })?;
        let mut client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel);
        let mut stream = client
            .invoke_stream(request)
            .await
            .map_err(|status| {
                anyhow::anyhow!(
                    "daemon error streaming {function_name} through Axon \
                     (code={:?}): {}",
                    status.code(),
                    status.message()
                )
            })?
            .into_inner();

        let mut frames = Vec::new();
        while let Some(chunk) = stream
            .message()
            .await
            .with_context(|| format!("read {function_name} InvokeStream chunk from local daemon"))?
        {
            let payload = if chunk.payload.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::from_slice(&chunk.payload)
                    .with_context(|| format!("decode {function_name} stream frame JSON"))?
            };
            let terminal = chunk.terminal;
            frames.push(crate::support::local_invoke::LocalStreamFrame {
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
) -> anyhow::Result<Vec<crate::support::local_invoke::LocalBidiFrame>> {
    use anyhow::Context;
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use easynet_axon::pb::axon::v1::{
        invoke_bidi_down::Payload as DownPayload, invoke_bidi_up::Payload as UpPayload,
        BinaryChunk, ContentEnvelope, EnvelopeOpen, InvocationTarget, InvokeBidiUp,
        StreamDescriptor,
    };
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    let socket_path = resolve_socket_path();
    if !probe_accepting(&socket_path) {
        return Err(anyhow::Error::new(
            crate::support::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
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
            .with_context(|| {
                format!(
                    "connect to local Axon daemon gRPC endpoint at {}",
                    socket_path.display()
                )
            })?;
        let mut client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel)
                .max_decoding_message_size(
                    crate::daemon::invocation::boot::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
                )
                .max_encoding_message_size(
                    crate::daemon::invocation::boot::MAX_INVOCATION_GRPC_MESSAGE_BYTES,
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
            .map_err(|status| {
                anyhow::anyhow!(
                    "daemon error opening bidi {function_name} through Axon \
                     (code={:?}): {}",
                    status.code(),
                    status.message()
                )
            })?
            .into_inner();

        let mut frames = Vec::new();
        while let Some(frame) = down
            .message()
            .await
            .with_context(|| format!("read {function_name} InvokeBidi down frame"))?
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
                    crate::support::local_invoke::LocalBidiFrame {
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
                    crate::support::local_invoke::LocalBidiFrame {
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
                DownPayload::Control(_) => crate::support::local_invoke::LocalBidiFrame {
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
) -> anyhow::Result<Vec<crate::support::local_invoke::LocalStreamFrame>> {
    anyhow::bail!(
        "streaming `{}` through the local Axon daemon requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
}

#[cfg(not(feature = "axon-pb"))]
pub(crate) fn invoke_local_daemon_ability_targeted_bidi_json_frames_with_subject(
    function_name: &str,
    _payload_json: serde_json::Value,
    _callee_ura: &str,
    _default_subject_ura: &str,
    _subject: Option<String>,
    _timeout: Duration,
    _input_frames: Vec<serde_json::Value>,
    _max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::local_invoke::LocalBidiFrame>> {
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
            crate::support::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
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
            .with_context(|| {
                format!(
                    "connect to local Axon daemon gRPC endpoint at {}",
                    socket_path.display()
                )
            })?;
        let mut client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel);
        let response = client.invoke(request).await.map_err(|status| {
            anyhow::anyhow!(
                "daemon error invoking {function_name} through Axon \
                 (code={:?}): {}",
                status.code(),
                status.message()
            )
        })?;
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
            crate::support::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
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
            .with_context(|| {
                format!(
                    "connect to local Axon daemon gRPC endpoint at {}",
                    socket_path.display()
                )
            })?;
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
/// through [`crate::support::local_invoke::invoke_local_ability_with_invocation_meta`].
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
///   2. After the invoke completes, the daemon's invocation ledger is
///      read directly by `request_id` for the persisted record. This is
///      intentionally not an `invocation.history.get` call: metadata
///      observation must not create a second invocation in the audit trail.
///      The returned metadata carries the
///      ledger-assigned `invocation_ura`, `trace_id`, and receipt
///      anchors — the material a downstream step needs to reference
///      THIS invocation as its causal parent.
///
/// A parent entry missing a usable `(receipt_ura, receipt_hash)` pair
/// is rejected before dispatch. Omitting an edge would mutate the
/// invocation DAG while still returning a successful step result.
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
        callee_agent,
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
        callee_agent: None,
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
    callee_agent: Option<&'a str>,
}

/// State machine for projecting daemon invocation metadata after a terminal
/// unary response.
///
/// Invariant 1: `ReceiptBacked` is the only state that may be used as a
/// causal parent because it owns a complete `(receipt_ura, receipt_hash)`
/// anchor.
/// Invariant 2: pending states still return the invocation result and submitted
/// tuple echoes. Observation surfaces may report them; child-invocation builders
/// must reject them before dispatch.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
enum InvocationMetadataProjection {
    ReceiptBacked {
        ledger_record: serde_json::Value,
        terminal_receipt: serde_json::Value,
    },
    LedgerPending {
        reason: String,
    },
    ReceiptAnchorPending {
        ledger_record: serde_json::Value,
        terminal_receipt: serde_json::Value,
        reason: String,
    },
}

#[cfg(feature = "axon-pb")]
impl InvocationMetadataProjection {
    fn from_polled_ledger_record(
        ledger_record: serde_json::Value,
        request_id: &str,
        ability: &str,
    ) -> Self {
        if ledger_record.is_null() {
            return Self::LedgerPending {
                reason: format!(
                    "ledger did not expose invocation record for request_id {request_id} after local invoke {ability}"
                ),
            };
        }

        match terminal_receipt_from_ledger_record(&ledger_record) {
            Some(terminal_receipt) if terminal_receipt_has_complete_anchor(&terminal_receipt) => {
                Self::ReceiptBacked {
                    ledger_record,
                    terminal_receipt,
                }
            }
            Some(terminal_receipt) => Self::ReceiptAnchorPending {
                ledger_record,
                terminal_receipt,
                reason: format!(
                    "ledger record for request_id {request_id} after local invoke {ability} has no complete terminal receipt anchor"
                ),
            },
            None => Self::ReceiptAnchorPending {
                ledger_record,
                terminal_receipt: serde_json::Value::Null,
                reason: format!(
                    "ledger record for request_id {request_id} after local invoke {ability} has no receipt_chain projection"
                ),
            },
        }
    }

    fn state_label(&self) -> &'static str {
        match self {
            Self::ReceiptBacked { .. } => "receipt_backed",
            Self::LedgerPending { .. } => "ledger_pending",
            Self::ReceiptAnchorPending { .. } => "receipt_anchor_pending",
        }
    }

    fn ledger_field(&self, key: &str) -> serde_json::Value {
        match self {
            Self::ReceiptBacked { ledger_record, .. }
            | Self::ReceiptAnchorPending { ledger_record, .. } => ledger_record
                .get(key)
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            Self::LedgerPending { .. } => serde_json::Value::Null,
        }
    }

    fn receipt(&self) -> serde_json::Value {
        match self {
            Self::ReceiptBacked {
                terminal_receipt, ..
            }
            | Self::ReceiptAnchorPending {
                terminal_receipt, ..
            } => terminal_receipt.clone(),
            Self::LedgerPending { .. } => serde_json::Value::Null,
        }
    }

    fn diagnostic(&self) -> Option<&str> {
        match self {
            Self::ReceiptBacked { .. } => None,
            Self::LedgerPending { reason } | Self::ReceiptAnchorPending { reason, .. } => {
                Some(reason.as_str())
            }
        }
    }
}

#[cfg(feature = "axon-pb")]
use crate::support::invocation_receipt_projection::{
    terminal_receipt_from_ledger_record, terminal_receipt_has_complete_anchor,
};

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
) -> anyhow::Result<(serde_json::Value, String)> {
    use anyhow::{anyhow, Context};
    use serde_json::Value;

    let response = client.invoke(request).await.map_err(|status| {
        anyhow!(
            "daemon error invoking {function_name} through Axon \
             (code={:?}): {}",
            status.code(),
            status.message()
        )
    })?;
    let body = response.into_inner();
    let request_id = body
        .header
        .as_ref()
        .map(|header| header.request_id.clone())
        .unwrap_or_default();
    let value = if body.result.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body.result)
            .with_context(|| format!("decode {function_name} Axon response"))?
    };
    Ok((value, request_id))
}

#[cfg(feature = "axon-pb")]
async fn invoke_local_daemon_first_stream_payload(
    client: &mut LocalInvocationGrpcClient,
    request: easynet_axon::pb::axon::v1::InvokeServerStreamRequest,
    function_name: &str,
) -> anyhow::Result<LocalDaemonStreamProjection> {
    use anyhow::{anyhow, bail, Context};

    let mut stream = client
        .invoke_stream(request)
        .await
        .map_err(|status| {
            anyhow!(
                "daemon error streaming {function_name} through Axon \
                 (code={:?}): {}",
                status.code(),
                status.message()
            )
        })?
        .into_inner();

    let mut frames_seen = 0_usize;
    while let Some(chunk) = stream
        .message()
        .await
        .with_context(|| format!("read {function_name} InvokeStream chunk from local daemon"))?
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
    use anyhow::{anyhow, bail, Context};
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
        callee_agent,
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

    let default_callee_ura = local_daemon_default_callee_ura();
    // Identity granularity: a caller-provided agent name resolves through
    // local-agents.json to the canonical hosted Agent URA. The local device
    // remains only the execution substrate and mission-resource owner; it is
    // not allowed to synthesize a new callee identity during invocation.
    let callee_ura = match callee_agent.map(str::trim).filter(|a| !a.is_empty()) {
        Some(agent) => canonical_hosted_agent_ura_by_name(agent)?,
        _ => default_callee_ura.clone(),
    };
    let subject_ura = match subject.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(explicit) => explicit.to_string(),
        None => match trace_id.map(str::trim).filter(|t| !t.is_empty()) {
            Some(mission_id) => {
                LocalMissionSubjectOwner::from_runtime_identity()?.mission_subject_ura(mission_id)
            }
            _ => callee_ura.clone(),
        },
    };
    let receipt_refs = receipt_refs_from_causal_parents(causal_parents)?;
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
    let (result_value, request_id) = runtime.block_on(async move {
        let channel = connect_channel(
            invoke_socket.clone(),
            request_timeout,
            Duration::from_secs(10),
        )
        .await
        .with_context(|| {
            format!(
                "connect to local Axon daemon gRPC endpoint at {}",
                invoke_socket.display()
            )
        })?;
        let mut client =
            easynet_axon::pb::axon::v1::invocation_client::InvocationClient::new(channel);
        invoke_local_daemon_json(&mut client, request, &invoke_fn).await
    })?;
    if request_id.is_empty() {
        bail!(
            "daemon response invoking {function_name} did not include request_id; \
             cannot build ledger-backed invocation metadata"
        );
    }

    // The unary invoke returns at terminal state, but the ledger sink persists
    // asynchronously. Poll the daemon's side-effect-free `invocation.record.get`
    // read RPC rather than opening the redb ledger file from this CLI process:
    // redb takes an exclusive cross-process lock, so a second-process open fails
    // hard whenever the daemon is running. The daemon services the read off its
    // own in-process ledger handle and writes no row, so observation never
    // appends another invocation to the ledger it is observing.
    let record_reader = LocalDaemonAbilityClient::new()?;
    let mut ledger_record = Value::Null;
    for _ in 0..10 {
        let response = record_reader.invoke(
            crate::daemon::ability::builtins::governance::invocation_history::ABILITY_INVOCATION_RECORD_GET,
            serde_json::json!({ "request_id": request_id }),
        )?;
        if let Some(record) = response.get("record").filter(|record| !record.is_null()) {
            ledger_record = record.clone();
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let metadata_projection = InvocationMetadataProjection::from_polled_ledger_record(
        ledger_record,
        &request_id,
        &function_name,
    );

    // Identity fields are the LEDGER's persisted values, not the
    // submitted ones — the daemon may rewrite the callee during route
    // selection, and the record must report what was actually
    // persisted (same rule as trace_id). `submitted_*` keep the
    // pre-admission view for audit diffing.
    let mut meta = serde_json::json!({
        "request_id": request_id,
        "trace_id": metadata_projection.ledger_field("trace_id"),
        "invocation_ura": metadata_projection.ledger_field("invocation_ura"),
        "caller_ura": metadata_projection.ledger_field("caller_ura"),
        "callee_ura": metadata_projection.ledger_field("callee_ura"),
        "subject_ura": metadata_projection.ledger_field("subject_ura"),
        "submitted_caller_ura": wire_caller_ura,
        "submitted_callee_ura": callee_ura,
        "submitted_subject_ura": subject_ura,
        "ability": function_name,
        "nonce": nonce_hex,
        "causal_context": { "parents": causal_parents },
        "receipt": metadata_projection.receipt(),
        "metadata_state": metadata_projection.state_label(),
        "ledger_state": metadata_projection.ledger_field("state"),
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
    if let Some(diagnostic) = metadata_projection.diagnostic() {
        meta["metadata_diagnostic"] = serde_json::json!(diagnostic);
    }

    Ok((result_value, meta))
}

#[cfg(feature = "axon-pb")]
fn receipt_refs_from_causal_parents(
    causal_parents: &[serde_json::Value],
) -> anyhow::Result<Vec<easynet_axon::pb::axon::v1::ReceiptRef>> {
    use anyhow::{bail, Context};
    use easynet_axon::pb::axon::v1 as pb;
    use serde_json::Value;

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
        if receipt_hash.len() != 32 {
            bail!(
                "causal parent #{idx} receipt_hash must decode to 32 bytes, got {}",
                receipt_hash.len()
            );
        }
        refs.push(pb::ReceiptRef {
            receipt_hash,
            receipt_ura: receipt_ura.to_string(),
        });
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
    #[cfg(test)]
    {
        if function_name.starts_with("agent.") {
            return invoke_agent_management_in_process(function_name, _payload_json);
        }
        anyhow::bail!(
            "daemon not running: local Axon gRPC listener is unavailable in this test build. \
             Start it with `easynet runtime start` or `easynet start`."
        );
    }
    #[cfg(not(test))]
    anyhow::bail!(
        "invoking `{}` through the local Axon daemon requires the `axon-pb` feature; \
         rebuild with `cargo build --features axon-pb`",
        function_name
    )
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
    Ok(crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA.to_string())
}

#[cfg(feature = "axon-pb")]
fn local_daemon_default_callee_ura() -> String {
    crate::runtime::local_invocation_identity::local_device_ura()
}

#[cfg(feature = "axon-pb")]
fn canonical_hosted_agent_ura_by_name(agent_name: &str) -> anyhow::Result<String> {
    let agent_name = agent_name.trim();
    if agent_name.is_empty() {
        anyhow::bail!("hosted agent callee name must not be empty");
    }
    let local_agents = crate::persistence::local_agents::load()
        .map_err(|err| anyhow::anyhow!("load local hosted agents: {err}"))?;
    let entry = crate::persistence::local_agents::lookup_hosted_agent_by_name(
        &local_agents,
        agent_name,
    )?
    .ok_or_else(|| {
        anyhow::anyhow!(
            "hosted agent {agent_name:?} is not registered in local-agents.json; run federation join/agent advertise before invoking as that agent"
        )
    })?;
    let parsed = crate::ura::parse_ura(&entry.agent_ura).map_err(|err| {
        anyhow::anyhow!(
            "hosted agent {agent_name:?} has invalid Agent URA {:?}: {err}",
            entry.agent_ura
        )
    })?;
    if parsed.kind != crate::ura::URAKind::Agent {
        anyhow::bail!(
            "hosted agent {agent_name:?} resolved to non-Agent URA {:?}",
            entry.agent_ura
        );
    }
    Ok(entry.agent_ura.clone())
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
        Self::from_device_ura(&crate::runtime::local_invocation_identity::local_device_ura())
    }

    fn from_device_ura(device_ura: &str) -> anyhow::Result<Self> {
        let parsed = crate::ura::parse_ura(device_ura).map_err(|err| {
            anyhow::anyhow!(
                "local mission subject owner has invalid Device URA {device_ura:?}: {err}"
            )
        })?;
        if parsed.kind != crate::ura::URAKind::Device {
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
        crate::ura::resource_dot_ura(
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
                crate::daemon::invocation::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
            ),
            "local loopback projection must not pre-bind descriptor metadata"
        );
        let envelope = request.envelope.as_ref().expect("request envelope");
        assert_eq!(
            envelope.caller.as_ref().map(|caller| caller.ura.as_str()),
            Some(crate::runtime::local_invocation_identity::LOCAL_SYSTEM_AGENT_URA)
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
    fn invocation_metadata_projection_models_missing_ledger_as_pending_state() {
        let projection = InvocationMetadataProjection::from_polled_ledger_record(
            serde_json::Value::Null,
            "req-1",
            "discover",
        );

        assert_eq!(projection.state_label(), "ledger_pending");
        assert_eq!(projection.receipt(), serde_json::Value::Null);
        assert!(projection
            .diagnostic()
            .expect("pending diagnostic")
            .contains("req-1"));
    }

    #[test]
    fn invocation_metadata_projection_models_incomplete_anchor_as_pending_state() {
        let projection = InvocationMetadataProjection::from_polled_ledger_record(
            serde_json::json!({
                "state": "completed",
                "trace_id": "trace-1",
                "receipt_chain": {
                    "head_receipt_hash": "",
                    "anchors": []
                }
            }),
            "req-2",
            "discover",
        );

        assert_eq!(projection.state_label(), "receipt_anchor_pending");
        assert_eq!(
            projection.ledger_field("trace_id"),
            serde_json::json!("trace-1")
        );
        assert!(projection
            .diagnostic()
            .expect("anchor diagnostic")
            .contains("no complete terminal receipt anchor"));
    }

    #[test]
    fn stream_first_payload_projection_has_bounded_empty_frame_budget() {
        assert!(
            STREAM_FIRST_FRAME_PROJECTION_MAX_FRAMES > 0
                && STREAM_FIRST_FRAME_PROJECTION_MAX_FRAMES <= 64
        );
    }
}
