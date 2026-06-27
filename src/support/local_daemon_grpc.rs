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
use crate::core::ability_spec::AbilityManifest;
#[cfg(feature = "axon-pb")]
use crate::runtime::ability::HostedAgentDelegationClaims;
#[cfg(feature = "axon-pb")]
use crate::services::self_identity::{LocalDaemonSigner, SelfIdentity};

/// Resolve the local daemon Invocation endpoint. Thin re-export of
/// [`crate::persistence::daemon_config::resolved_local_uds_path_with_env_override`]
/// kept here so the existing CLI call sites
/// (`facade/cli/federation_discover.rs`, `facade/cli/start.rs`,
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
    #[cfg(feature = "axon-pb")]
    caller_override: Option<String>,
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
impl LocalDaemonAbilityClient {
    fn grpc(caller_override: Option<String>) -> Self {
        Self {
            caller_override,
            #[cfg(test)]
            transport: LocalDaemonAbilityTransport::Grpc,
        }
    }

    pub(crate) fn new() -> anyhow::Result<Self> {
        Self::validate_socket()?;
        Ok(Self::grpc(None))
    }

    pub(crate) fn with_caller_ura(caller_ura: impl Into<String>) -> anyhow::Result<Self> {
        let caller_ura = caller_ura.into();
        if caller_ura.trim().is_empty() {
            anyhow::bail!("host device URI is empty; cannot invoke local daemon abilities");
        }
        Self::validate_socket()?;
        Ok(Self::grpc(Some(caller_ura)))
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
                if let Ok(client) = Self::with_caller_ura(caller_ura) {
                    return Ok(client);
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
            Self::with_caller_ura(caller_ura)
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
            caller_override: None,
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

        invoke_local_daemon_ability_with_caller_and_subject_policy(
            function_name,
            payload_json,
            self.caller_override.as_deref(),
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

    pub(crate) fn with_caller_ura(_caller_ura: impl Into<String>) -> anyhow::Result<Self> {
        Self::new()
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
    crate::runtime::agents::agent_list_ability::register(&mut catalog, || {
        crate::registry::agents::load_agents().unwrap_or_default()
    });
    let hot_registrar: std::sync::Arc<
        crate::runtime::agents::agent_lifecycle_ability::SharedHotRegistrarCell,
    > = std::sync::Arc::new(std::sync::OnceLock::new());
    crate::runtime::agents::agent_lifecycle_ability::register(&mut catalog, hot_registrar);
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
    invoke_local_daemon_ability_with_caller_callee_and_subject(
        function_name,
        payload_json,
        None,
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

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_stream_with_target(
    function_name: &str,
    payload_json: serde_json::Value,
    callee_override: Option<&str>,
    subject_policy: LocalDaemonSubjectPolicy,
    timeout: Duration,
    max_frames: Option<usize>,
) -> anyhow::Result<Vec<crate::support::local_invoke::LocalStreamFrame>> {
    use anyhow::{anyhow, bail, Context};
    use easynet_axon::pb::axon::v1::InvokeServerStreamRequest;

    let function_name = function_name.trim();
    if function_name.is_empty() {
        bail!("function_name must not be empty");
    }

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

    let caller_ura = local_daemon_loopback_caller_ura()?;
    let default_callee_ura = local_daemon_default_callee_ura();
    let callee_ura = callee_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_callee_ura.as_str())
        .to_string();
    let subject_ura = subject_policy.resolve(&callee_ura)?;
    let arguments = serde_json::to_vec(&payload_json)
        .with_context(|| format!("encode {function_name} args"))?;
    let descriptor_ref = resolve_local_signed_descriptor_ref(&callee_ura, function_name)
        .with_context(|| format!("resolve descriptor ref for {function_name}"))?;
    let envelope = signed_local_daemon_envelope(
        caller_ura.clone(),
        callee_ura,
        subject_ura,
        &descriptor_ref,
        &arguments,
    )
    .with_context(|| format!("sign {function_name} Axon InvokeStream envelope"))?;
    let mut request = InvokeServerStreamRequest {
        envelope: Some(envelope),
        function_name: function_name.to_string(),
        arguments,
        content_type: "application/json".to_string(),
        timeout_seconds: i32::try_from(timeout.as_secs()).unwrap_or(i32::MAX),
        ..InvokeServerStreamRequest::default()
    };
    request.metadata.insert(
        crate::services::invocation_transport::invocation_wire::SIGNED_DESCRIPTOR_REF_METADATA_KEY
            .to_string(),
        descriptor_ref,
    );

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
                anyhow!(
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

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_with_caller_and_subject_policy(
    function_name: &str,
    payload_json: serde_json::Value,
    caller_override: Option<&str>,
    subject_policy: LocalDaemonSubjectPolicy,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    invoke_local_daemon_ability_with_caller_callee_and_subject(
        function_name,
        payload_json,
        caller_override,
        None,
        subject_policy,
        timeout,
    )
}

#[cfg(feature = "axon-pb")]
fn invoke_local_daemon_ability_with_caller_callee_and_subject(
    function_name: &str,
    payload_json: serde_json::Value,
    caller_override: Option<&str>,
    callee_override: Option<&str>,
    subject_policy: LocalDaemonSubjectPolicy,
    timeout: Duration,
) -> anyhow::Result<serde_json::Value> {
    use anyhow::{anyhow, bail, Context};

    let function_name = function_name.trim();
    if function_name.is_empty() {
        bail!("function_name must not be empty");
    }

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

    let caller_ura = match caller_override.map(str::trim).filter(|s| !s.is_empty()) {
        Some(caller) => caller.to_string(),
        None => local_daemon_loopback_caller_ura()?,
    };
    let default_callee_ura = local_daemon_default_callee_ura();
    let callee_ura = callee_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_callee_ura.as_str())
        .to_string();
    let subject_ura = subject_policy.resolve(&callee_ura)?;
    let arguments = serde_json::to_vec(&payload_json)
        .with_context(|| format!("encode {function_name} args"))?;
    let request = signed_local_daemon_invoke_request(
        caller_ura.clone(),
        callee_ura,
        subject_ura,
        function_name,
        arguments,
    )
    .with_context(|| format!("build signed {function_name} Axon InvokeRequest"))?;

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
            anyhow!(
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
///      polled (`invocation.history.get` by `request_id`) for the
///      persisted record, so the returned metadata carries the
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
    let delegated = HostedAgentDelegation::parse(hosted_agent_ura)?;
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
#[derive(Debug, Clone)]
struct HostedAgentDelegation {
    agent_ura: String,
}

#[cfg(feature = "axon-pb")]
struct LocalDaemonInvocationMetaRequest<'a> {
    function_name: &'a str,
    payload_json: serde_json::Value,
    subject: Option<String>,
    causal_parents: &'a [serde_json::Value],
    step_timeout: Option<Duration>,
    trace_id: Option<&'a str>,
    delegation: Option<HostedAgentDelegation>,
    callee_agent: Option<&'a str>,
}

#[cfg(feature = "axon-pb")]
impl HostedAgentDelegation {
    fn parse(agent_ura: &str) -> anyhow::Result<Self> {
        let agent_ura = agent_ura.trim();
        if agent_ura.is_empty() {
            anyhow::bail!("hosted agent delegation requires a non-empty Agent URA");
        }
        let parsed = crate::ura::parse_ura(agent_ura)
            .map_err(|err| anyhow::anyhow!("hosted agent delegation URA is invalid: {err}"))?;
        if parsed.kind != crate::ura::URAKind::Agent {
            anyhow::bail!(
                "hosted agent delegation requires an Agent URA, got {:?}",
                parsed.kind
            );
        }
        Ok(Self {
            agent_ura: agent_ura.to_string(),
        })
    }

    fn metadata_value(
        &self,
        wire_caller_ura: &str,
        callee_ura: &str,
        subject_ura: &str,
        request_id: &str,
        invocation_nonce_hex: &str,
        ability: &str,
        signer: &dyn SelfIdentity,
    ) -> anyhow::Result<String> {
        let claims = HostedAgentDelegationClaims::new(
            self.agent_ura.clone(),
            "host_device",
            wire_caller_ura,
            callee_ura,
            subject_ura,
            request_id,
            invocation_nonce_hex,
            ability,
        )?;
        let signature = signer.sign(
            wire_caller_ura,
            &claims.signing_payload_bytes(wire_caller_ura),
        )?;
        claims.signed_metadata_value(wire_caller_ura, &signature)
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
enum InvocationMetadataProjection {
    ReceiptBacked {
        ledger_record: serde_json::Value,
        terminal_receipt: serde_json::Value,
    },
}

#[cfg(feature = "axon-pb")]
impl InvocationMetadataProjection {
    fn from_ledger_record(
        ledger_record: serde_json::Value,
        request_id: &str,
        ability: &str,
    ) -> anyhow::Result<Self> {
        if ledger_record.is_null() {
            anyhow::bail!(
                "ledger did not expose invocation record for request_id {request_id} after local invoke {ability}; cannot build receipt-backed invocation metadata"
            );
        }

        match terminal_receipt_from_ledger_record(&ledger_record) {
            Some(terminal_receipt) if terminal_receipt_has_complete_anchor(&terminal_receipt) => {
                Ok(Self::ReceiptBacked {
                    ledger_record,
                    terminal_receipt,
                })
            }
            _ => anyhow::bail!(
                "ledger record for request_id {request_id} after local invoke {ability} has no complete terminal receipt anchor; cannot build causal parent metadata"
            ),
        }
    }

    fn state_label(&self) -> &'static str {
        match self {
            Self::ReceiptBacked { .. } => "receipt_backed",
        }
    }

    fn ledger_field(&self, key: &str) -> serde_json::Value {
        match self {
            Self::ReceiptBacked { ledger_record, .. } => ledger_record
                .get(key)
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        }
    }

    fn receipt(&self) -> serde_json::Value {
        match self {
            Self::ReceiptBacked {
                terminal_receipt, ..
            } => terminal_receipt.clone(),
        }
    }
}

#[cfg(feature = "axon-pb")]
fn terminal_receipt_from_ledger_record(
    ledger_record: &serde_json::Value,
) -> Option<serde_json::Value> {
    let chain = ledger_record.get("receipt_chain")?;
    let head_hash = chain
        .get("head_receipt_hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let anchors = chain
        .get("anchors")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let head_anchor = anchors
        .iter()
        .find(|anchor| {
            anchor
                .get("receipt_hash")
                .and_then(serde_json::Value::as_str)
                == Some(head_hash.as_str())
                && !head_hash.is_empty()
        })
        .or_else(|| anchors.last())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    Some(serde_json::json!({
        "head_receipt_hash": head_hash,
        "anchor": head_anchor,
        "anchor_count": anchors.len(),
    }))
}

#[cfg(feature = "axon-pb")]
fn terminal_receipt_has_complete_anchor(receipt: &serde_json::Value) -> bool {
    let anchor = match receipt.get("anchor") {
        Some(anchor) => anchor,
        None => return false,
    };
    let has_ura = anchor
        .get("receipt_ura")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    let has_hash = anchor
        .get("receipt_hash")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    has_ura && has_hash
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

    let wire_caller_ura = local_daemon_loopback_caller_ura()?;
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
    let arguments = serde_json::to_vec(&payload_json)
        .with_context(|| format!("encode {function_name} args"))?;
    let envelope = crate::services::invocation_transport::ProtoEnvelope::targeted(
        wire_caller_ura.clone(),
        callee_ura.clone(),
        subject_ura.clone(),
    )?
    .with_causal_context(pb::CausalContext {
        form: Some(causal_form),
    });
    let signer = LocalDaemonSigner::for_caller(&wire_caller_ura);
    let descriptor_ref = resolve_local_signed_descriptor_ref(&callee_ura, &function_name)
        .with_context(|| format!("resolve descriptor ref for {function_name}"))?;
    let mut request = envelope
        .signed_descriptor_ref_invoke_request(
            &function_name,
            descriptor_ref.clone(),
            arguments,
            &signer,
        )
        .with_context(|| format!("build signed {function_name} Axon InvokeRequest"))?;
    let (wire_request_id, nonce_hex) = request
        .envelope
        .as_ref()
        .map(|env| {
            (
                env.request_id.trim().to_string(),
                hex::encode(&env.invocation_nonce),
            )
        })
        .ok_or_else(|| anyhow!("build signed {function_name} request without envelope"))?;
    if wire_request_id.is_empty() {
        bail!("build signed {function_name} request without envelope request_id");
    }
    if let Some(delegation) = delegation.as_ref() {
        let metadata_value = delegation.metadata_value(
            &wire_caller_ura,
            &callee_ura,
            &subject_ura,
            &wire_request_id,
            &nonce_hex,
            &descriptor_ref,
            &signer,
        )?;
        request.metadata.insert(
            crate::runtime::ability::HOSTED_AGENT_DELEGATION_METADATA_KEY.to_string(),
            metadata_value,
        );
    }
    if let Some(envelope) = request.envelope.as_mut() {
        // `trace_id` is Envelope operational metadata (outside the
        // caller-signature region), so stamping it post-build is safe.
        if let Some(trace_id) = trace_id.map(str::trim).filter(|t| !t.is_empty()) {
            envelope.trace_id = trace_id.to_string();
        }
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
        let response = client.invoke(request).await.map_err(|status| {
            anyhow!(
                "daemon error invoking {invoke_fn} through Axon \
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
                .with_context(|| format!("decode {invoke_fn} Axon response"))?
        };
        Ok::<_, anyhow::Error>((value, request_id))
    })?;
    if request_id.is_empty() {
        bail!(
            "daemon response invoking {function_name} did not include request_id; \
             cannot build ledger-backed invocation metadata"
        );
    }

    // The unary invoke returns at terminal state, but the ledger sink
    // persists asynchronously — poll briefly rather than racing it.
    let mut ledger_record = Value::Null;
    for _ in 0..10 {
        if let Ok(found) = invoke_local_daemon_ability_with_subject(
            "invocation.history.get",
            serde_json::json!({ "key": { "request_id": request_id } }),
            None,
        ) {
            let record = found.get("record").cloned().unwrap_or(Value::Null);
            if !record.is_null() {
                ledger_record = record;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let metadata_projection = InvocationMetadataProjection::from_ledger_record(
        ledger_record,
        &request_id,
        &function_name,
    )?;

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
            "agent_ura": delegation.agent_ura,
            "signing_authority": "host_device",
            "wire_caller_ura": meta.get("caller_ura").cloned().unwrap_or(Value::Null),
            "wire_callee_ura": meta.get("callee_ura").cloned().unwrap_or(Value::Null),
        });
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

#[cfg(feature = "axon-pb")]
fn signed_local_daemon_invoke_request(
    caller_ura: String,
    callee_ura: String,
    subject_ura: String,
    function_name: &str,
    arguments: Vec<u8>,
) -> anyhow::Result<easynet_axon::pb::axon::v1::InvokeRequest> {
    let signer = LocalDaemonSigner::for_caller(&caller_ura);
    let descriptor_ref = resolve_local_signed_descriptor_ref(&callee_ura, function_name)?;
    crate::services::invocation_transport::ProtoEnvelope::targeted(
        caller_ura,
        callee_ura,
        subject_ura,
    )?
    .signed_descriptor_ref_invoke_request(function_name, descriptor_ref, arguments, &signer)
}

#[cfg(feature = "axon-pb")]
pub(crate) fn resolve_local_signed_descriptor_ref(
    callee_ura: &str,
    function_name: &str,
) -> anyhow::Result<String> {
    if let Ok(descriptor_ref) =
        crate::runtime::axon_bridge::descriptor_ref::require_descriptor_ref_for_wire(
            callee_ura,
            function_name,
        )
    {
        return Ok(descriptor_ref);
    }

    if let Some(version) = descriptor_version_from_device_store(callee_ura, function_name)? {
        return crate::runtime::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
            callee_ura,
            function_name,
            &version,
        )
        .map_err(|err| anyhow::anyhow!("{err}"));
    }

    if let Some(version) = descriptor_version_from_daemon_wrapper(function_name) {
        return crate::runtime::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
            callee_ura,
            function_name,
            version,
        )
        .map_err(|err| anyhow::anyhow!("{err}"));
    }

    if let Some(version) = descriptor_version_from_system_manifest(function_name)? {
        return crate::runtime::axon_bridge::descriptor_ref::ability_descriptor_ref_for_wire(
            callee_ura,
            function_name,
            &version,
        )
        .map_err(|err| anyhow::anyhow!("{err}"));
    }

    anyhow::bail!(
        "cannot resolve descriptor ref for local daemon ability {function_name:?} under \
         callee {callee_ura:?}; no explicit descriptor ref, device deploy snapshot, or \
         daemon/system descriptor source was found"
    )
}

#[cfg(feature = "axon-pb")]
fn descriptor_version_from_device_store(
    callee_ura: &str,
    function_name: &str,
) -> anyhow::Result<Option<String>> {
    let rows = crate::runtime::agents::device_ability_store::DeviceAbilityStore::open_default()
        .load()
        .map_err(|err| {
            anyhow::anyhow!("read device ability store for descriptor signing: {err}")
        })?;
    for row in rows {
        if row.public_name() != function_name {
            continue;
        }
        let selector = match crate::ura::AbilitySelector::parse(row.ability_ura()) {
            Ok(selector) => selector,
            Err(_) => continue,
        };
        if selector.owner_ura() != callee_ura {
            continue;
        }
        let bytes = row.manifest_bytes()?;
        let manifest = AbilityManifest::from_json_slice(&bytes)?;
        return Ok(Some(manifest.descriptor_version().to_string()));
    }
    Ok(None)
}

#[cfg(feature = "axon-pb")]
fn descriptor_version_from_daemon_wrapper(function_name: &str) -> Option<&'static str> {
    use crate::services::invocation_transport::federation_wrappers::{
        ABILITY_FEDERATION_ADVERTISE_ABILITIES, ABILITY_FEDERATION_ADVERTISE_AGENT,
        ABILITY_FEDERATION_DISCOVER, ABILITY_FEDERATION_FORWARD_INVOKE,
        ABILITY_FEDERATION_HEARTBEAT, ABILITY_FEDERATION_JOIN,
        ABILITY_FEDERATION_LIST_USER_DEVICES, ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES,
        ABILITY_FEDERATION_RESOLVE, ABILITY_FEDERATION_RESOLVE_KEY, ABILITY_FEDERATION_REVOKE,
        ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY, ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2,
        ABILITY_NAMESPACE_PROXY_RESOLVE, ABILITY_NAMESPACE_RESOLVE,
        ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY,
    };

    match function_name {
        ABILITY_FEDERATION_JOIN
        | ABILITY_FEDERATION_ADVERTISE_AGENT
        | ABILITY_FEDERATION_HEARTBEAT
        | ABILITY_FEDERATION_RESOLVE
        | ABILITY_NAMESPACE_RESOLVE
        | ABILITY_NAMESPACE_PROXY_RESOLVE
        | ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY
        | ABILITY_FEDERATION_REVOKE
        | ABILITY_FEDERATION_FORWARD_INVOKE
        | ABILITY_FEDERATION_RESOLVE_KEY
        | ABILITY_FEDERATION_DISCOVER
        | ABILITY_FEDERATION_SUBSCRIBE_DIRECTORY_V2
        | ABILITY_FEDERATION_LIST_USER_DEVICES
        | ABILITY_FEDERATION_PROXY_LIST_USER_DEVICES
        | ABILITY_FEDERATION_ADVERTISE_ABILITIES
        | ABILITY_RUNTIME_BOOTSTRAP_SELF_IDENTITY => Some("1.0.0"),
        _ => None,
    }
}

#[cfg(feature = "axon-pb")]
fn descriptor_version_from_system_manifest(function_name: &str) -> anyhow::Result<Option<String>> {
    if function_name.contains('/') || function_name.contains('\\') {
        return Ok(None);
    }
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("abilities")
        .join("system")
        .join(format!("{function_name}.ability.toml"));
    if !manifest_path.is_file() {
        return Ok(None);
    }
    let body = std::fs::read_to_string(&manifest_path).map_err(|err| {
        anyhow::anyhow!(
            "read system ability manifest {} for descriptor signing: {err}",
            manifest_path.display()
        )
    })?;
    let manifest = AbilityManifest::from_toml_str(&body).map_err(|err| {
        anyhow::anyhow!(
            "parse system ability manifest {} for descriptor signing: {err}",
            manifest_path.display()
        )
    })?;
    if manifest.name() != function_name {
        anyhow::bail!(
            "system ability manifest {} names {:?}, expected {:?}",
            manifest_path.display(),
            manifest.name(),
            function_name
        );
    }
    Ok(Some(manifest.descriptor_version().to_string()))
}

#[cfg(feature = "axon-pb")]
fn signed_local_daemon_envelope(
    caller_ura: String,
    callee_ura: String,
    subject_ura: String,
    descriptor_ref: &str,
    arguments: &[u8],
) -> anyhow::Result<easynet_axon::pb::axon::v1::Envelope> {
    let signer = LocalDaemonSigner::for_caller(&caller_ura);
    Ok(
        crate::services::invocation_transport::ProtoEnvelope::targeted(
            caller_ura,
            callee_ura,
            subject_ura,
        )?
        .sign_descriptor_bound(descriptor_ref, arguments, &signer)?
        .into_inner(),
    )
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;

    #[test]
    fn daemon_wrapper_descriptor_resolves_without_system_manifest_fallback() {
        let callee = crate::ura::hub_ura("acme");
        let function_name = crate::services::invocation_transport::federation_wrappers::ABILITY_FEDERATION_FORWARD_INVOKE;
        let descriptor_ref = resolve_local_signed_descriptor_ref(&callee, function_name).unwrap();
        assert_eq!(
            descriptor_ref,
            format!(
                "{}@1.0.0",
                crate::ura::owner_ability_ura(&callee, function_name).unwrap()
            )
        );
    }

    #[test]
    fn unknown_local_signed_descriptor_ref_fails_closed() {
        let err = resolve_local_signed_descriptor_ref(
            &crate::ura::hub_ura("acme"),
            "unknown/internal-wrapper",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("cannot resolve descriptor ref"),
            "unknown wrappers must not fall back to a fabricated descriptor version: {err}"
        );
    }
}
