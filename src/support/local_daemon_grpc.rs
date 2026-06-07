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
#[cfg(any(windows, feature = "axon-pb"))]
use std::time::Duration;

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
        self.invoke_with_subject(function_name, payload_json, None)
    }

    pub(crate) fn invoke_with_subject(
        &self,
        function_name: &str,
        payload_json: serde_json::Value,
        subject: Option<String>,
    ) -> anyhow::Result<serde_json::Value> {
        #[cfg(test)]
        if self.transport == LocalDaemonAbilityTransport::InProcessAgentManagement {
            let _ = subject;
            return invoke_agent_management_in_process(function_name, payload_json);
        }

        invoke_local_daemon_ability_with_caller_and_subject(
            function_name,
            payload_json,
            self.caller_override.as_deref(),
            subject,
        )
    }

    fn validate_socket() -> anyhow::Result<()> {
        let socket_path = resolve_socket_path();
        if !probe_accepting(&socket_path) {
            anyhow::bail!(
                "daemon not running: gRPC listener not reachable at {}. Start it with `easynet runtime start` or `easynet start`.",
                socket_path.display()
            );
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
fn invoke_local_daemon_ability_with_caller_and_subject(
    function_name: &str,
    payload_json: serde_json::Value,
    caller_override: Option<&str>,
    subject: Option<String>,
) -> anyhow::Result<serde_json::Value> {
    use anyhow::{anyhow, bail, Context};

    let function_name = function_name.trim();
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

    let caller_ura = caller_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(local_daemon_loopback_caller_ura);
    let subject_ura = subject
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(caller_ura.as_str())
        .to_string();
    let arguments = serde_json::to_vec(&payload_json)
        .with_context(|| format!("encode {function_name} args"))?;
    let request = crate::services::invocation_transport::ProtoEnvelope::targeted(
        caller_ura.clone(),
        caller_ura,
        subject_ura,
    )
    .and_then(|env| env.invoke_request(function_name, arguments))
    .with_context(|| format!("build {function_name} Axon InvokeRequest"))?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build tokio runtime for local Axon daemon invoke")?;

    runtime.block_on(async move {
        let channel = connect_channel(
            socket_path.clone(),
            Duration::from_secs(30),
            Duration::from_secs(10),
        )
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

#[cfg(feature = "axon-pb")]
fn local_daemon_loopback_caller_ura() -> String {
    crate::persistence::config::load_credentials()
        .ok()
        .map(|creds| crate::ura::device_ura(&creds.realm, &creds.node_id))
        .unwrap_or_else(|| crate::ura::device_ura("cli", "local"))
}
