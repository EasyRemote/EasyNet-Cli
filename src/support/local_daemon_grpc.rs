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
        return Err(anyhow::Error::new(
            crate::support::local_invoke::LocalInvokeFailure::DaemonOffline(format!(
                "daemon not running (local Axon gRPC listener unreachable at {}). \
                 Start it with `easynet runtime start`.",
                socket_path.display()
            )),
        ));
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
/// is skipped at the wire level (fabricating a receipt hash would be
/// worse than omitting the edge) but remains visible in the returned
/// metadata's `causal_context.parents` echo for audit.
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
    use anyhow::{anyhow, bail, Context};
    use easynet_axon::pb::axon::v1 as pb;
    use serde_json::Value;

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

    let caller_ura = local_daemon_loopback_caller_ura();
    // Identity granularity (ratified 2026-06-11): when the caller names
    // the owning agent, the callee is the device-owned agent URA
    // (`agent/device.<device-id>.<agent>`); when a trace id (mission
    // run id) is present and no explicit subject was given, the subject
    // is the mission-run resource
    // (`resource/device.<device-id>.missions/<mission-id>`). Both
    // shapes are Axon builders — nothing is string-assembled here.
    let parsed_caller = crate::ura::parse_ura(&caller_ura)
        .with_context(|| format!("parse loopback caller URA {caller_ura:?}"))?;
    let (realm, device_id) = (
        parsed_caller.realm.clone(),
        parsed_caller.device_id().unwrap_or_default().to_string(),
    );
    let callee_ura = match callee_agent.map(str::trim).filter(|a| !a.is_empty()) {
        Some(agent) if !device_id.is_empty() => {
            crate::ura::device_agent_ura(&realm, &device_id, agent)
        }
        _ => caller_ura.clone(),
    };
    let subject_ura = match subject.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(explicit) => explicit.to_string(),
        None => match trace_id.map(str::trim).filter(|t| !t.is_empty()) {
            Some(mission_id) if !device_id.is_empty() => crate::ura::resource_dot_ura(
                &realm,
                &format!("device.{device_id}.missions"),
                mission_id,
            ),
            _ => caller_ura.clone(),
        },
    };
    let arguments = serde_json::to_vec(&payload_json)
        .with_context(|| format!("encode {function_name} args"))?;
    let mut request = crate::services::invocation_transport::ProtoEnvelope::targeted(
        caller_ura.clone(),
        callee_ura.clone(),
        subject_ura.clone(),
    )
    .and_then(|env| env.invoke_request(&function_name, arguments))
    .with_context(|| format!("build {function_name} Axon InvokeRequest"))?;

    let receipt_refs: Vec<pb::ReceiptRef> = causal_parents
        .iter()
        .filter_map(|parent| {
            let receipt_ura = parent.get("receipt_ura").and_then(Value::as_str)?;
            let hash_hex = parent.get("receipt_hash").and_then(Value::as_str)?;
            let receipt_hash = hex::decode(hash_hex.trim()).ok()?;
            (!receipt_ura.trim().is_empty() && !receipt_hash.is_empty()).then(|| pb::ReceiptRef {
                receipt_hash,
                receipt_ura: receipt_ura.trim().to_string(),
            })
        })
        .collect();
    let mut refs = receipt_refs;
    let causal_form = match refs.len() {
        0 => pb::causal_context::Form::None(pb::Empty {}),
        1 => pb::causal_context::Form::Scalar(refs.remove(0)),
        _ => pb::causal_context::Form::List(pb::ReceiptList { prior: refs }),
    };
    let nonce_hex = request
        .envelope
        .as_ref()
        .map(|env| hex::encode(&env.invocation_nonce))
        .unwrap_or_default();
    if let Some(envelope) = request.envelope.as_mut() {
        envelope.causal_context = Some(pb::CausalContext {
            form: Some(causal_form),
        });
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

    // The unary invoke returns at terminal state, but the ledger sink
    // persists asynchronously — poll briefly rather than racing it.
    let mut ledger_record = Value::Null;
    if !request_id.is_empty() {
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
    }

    let terminal_receipt = ledger_record.get("receipt_chain").map(|chain| {
        let head_hash = chain
            .get("head_receipt_hash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let anchors = chain
            .get("anchors")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let head_anchor = anchors
            .iter()
            .find(|anchor| {
                anchor.get("receipt_hash").and_then(Value::as_str) == Some(head_hash.as_str())
                    && !head_hash.is_empty()
            })
            .or_else(|| anchors.last())
            .cloned()
            .unwrap_or(Value::Null);
        serde_json::json!({
            "head_receipt_hash": head_hash,
            "anchor": head_anchor,
            "anchor_count": anchors.len(),
        })
    });

    // Identity fields are the LEDGER's persisted values, not the
    // submitted ones — the daemon may rewrite the callee during route
    // selection, and the record must report what was actually
    // persisted (same rule as trace_id). `submitted_*` keep the
    // pre-admission view for audit diffing.
    let meta = serde_json::json!({
        "request_id": request_id,
        "trace_id": ledger_record.get("trace_id").cloned().unwrap_or(Value::Null),
        "invocation_ura": ledger_record.get("invocation_ura").cloned().unwrap_or(Value::Null),
        "caller_ura": ledger_record.get("caller_ura").cloned().unwrap_or(Value::Null),
        "callee_ura": ledger_record.get("callee_ura").cloned().unwrap_or(Value::Null),
        "subject_ura": ledger_record.get("subject_ura").cloned().unwrap_or(Value::Null),
        "submitted_callee_ura": callee_ura,
        "submitted_subject_ura": subject_ura,
        "ability": function_name,
        "nonce": nonce_hex,
        "causal_context": { "parents": causal_parents },
        "receipt": terminal_receipt.unwrap_or(Value::Null),
        "ledger_state": ledger_record.get("state").cloned().unwrap_or(Value::Null),
    });

    Ok((result_value, meta))
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
