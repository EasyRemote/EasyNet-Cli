// EasyNet CLI — daemon SDK surface
// =================================
//
// File: src/daemon.rs
// Description: Rust SDK facade for starting, inspecting, stopping,
//              and invoking the local `easynet-daemon`.
//
// Responsibility boundary
// -----------------------
// This module is the public Rust API for the EasyNet product daemon.
// It owns process lifecycle and local endpoint discovery for
// `easynet-daemon`; it does not own Axon protocol semantics. When the
// `axon-pb` feature is enabled, `DaemonClient` submits complete Axon
// Invocation requests to the daemon-hosted Invocation transport.
//
// What this module is NOT
// -----------------------
// - It is not `persistence::daemon_config::DaemonConfig`. That type is
//   the validated representation of `~/.easynet/daemon-config.toml`;
//   this module's `DaemonConfig` is a start/configuration object for
//   the SDK call that launches a process.
// - It is not the gRPC service implementation. That lives in
//   `services::invocation_transport`.
// - It is not an Axon runtime lifecycle API. `axon-runtime` remains
//   owned by the Axon SDK; this module starts only `easynet-daemon`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use thiserror::Error;

use crate::persistence::config;
use crate::services::control::transport;
use crate::support::{local_daemon_grpc, net};

const DEFAULT_DAEMON_BIN: &str = "easynet-daemon";
const DEFAULT_STOP_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(feature = "axon-pb")]
const DEFAULT_INVOKE_TIMEOUT: Duration = Duration::from_secs(30);
#[cfg(feature = "axon-pb")]
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Result alias for the daemon SDK surface.
pub type Result<T> = std::result::Result<T, DaemonError>;

/// Errors emitted by the daemon SDK boundary.
#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("daemon node id must not be empty")]
    EmptyNodeId,
    #[error("daemon binary path must not be empty")]
    EmptyBinaryPath,
    #[error("failed to create daemon log directory {path}: {source}")]
    CreateLogDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to open daemon log file {path}: {source}")]
    OpenLog {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to spawn {binary}: {source}")]
    Spawn {
        binary: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to create daemon pidfile directory {path}: {source}")]
    CreatePidDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write daemon pidfile {path}: {source}")]
    WritePid {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("pid {pid} is no longer alive")]
    PidNotAlive { pid: u32 },
    #[error("pid {pid} is not an EasyNet process")]
    PidReuseRefused { pid: u32 },
    #[error("pid {pid} did not exit within {timeout_ms} ms")]
    StopTimedOut { pid: u32, timeout_ms: u64 },
    #[error("daemon invocation transport is not reachable at {endpoint}")]
    InvocationEndpointDown { endpoint: PathBuf },
    #[cfg(feature = "axon-pb")]
    #[error("invalid daemon invocation: {0}")]
    InvalidInvocation(String),
    #[cfg(feature = "axon-pb")]
    #[error("failed to encode invocation arguments: {0}")]
    EncodeArguments(serde_json::Error),
    #[cfg(feature = "axon-pb")]
    #[error("failed to build daemon gRPC channel to {endpoint}: {source}")]
    Connect {
        endpoint: PathBuf,
        source: anyhow::Error,
    },
    #[cfg(feature = "axon-pb")]
    #[error("daemon returned gRPC status for {ability}: code={code:?}, message={message}")]
    InvokeStatus {
        ability: String,
        code: tonic::Code,
        message: String,
    },
}

/// SDK configuration for launching `easynet-daemon`.
///
/// Invariants:
/// 1. `node_id` is non-empty. Device mode passes the paired device id;
///    hub mode uses the stable sentinel `hub`.
/// 2. `daemon_bin` is either caller-supplied or resolved by the same
///    production rule as the CLI: `EASYNET_DAEMON_BIN`, sibling of the
///    current executable, then `easynet-daemon` on `PATH`.
/// 3. `start` never steals a live daemon. If the control endpoint is
///    already accepting, it returns a `DaemonHandle` with no child.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    node_id: String,
    daemon_bin: Option<PathBuf>,
    env: BTreeMap<String, String>,
    log_path: Option<PathBuf>,
    detach: bool,
}

impl DaemonConfig {
    /// Build a daemon start config for device mode.
    pub fn device(node_id: impl Into<String>) -> Result<Self> {
        Self::new(node_id)
    }

    /// Build a daemon start config for hub mode.
    pub fn hub() -> Self {
        Self {
            node_id: "hub".to_string(),
            daemon_bin: None,
            env: BTreeMap::new(),
            log_path: None,
            detach: true,
        }
    }

    /// Build a daemon start config for an explicit runtime identity.
    pub fn new(node_id: impl Into<String>) -> Result<Self> {
        let node_id = node_id.into();
        if node_id.trim().is_empty() {
            return Err(DaemonError::EmptyNodeId);
        }
        Ok(Self {
            node_id,
            daemon_bin: None,
            env: BTreeMap::new(),
            log_path: None,
            detach: true,
        })
    }

    /// Override the daemon binary path.
    pub fn with_daemon_bin(mut self, path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(DaemonError::EmptyBinaryPath);
        }
        self.daemon_bin = Some(path);
        Ok(self)
    }

    /// Add an environment variable to the daemon process.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Override the daemon log file path.
    pub fn with_log_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.log_path = Some(path.into());
        self
    }

    /// Control whether Unix starts detach into a new session.
    pub fn detached(mut self, detach: bool) -> Self {
        self.detach = detach;
        self
    }

    /// Return the node id that will be passed through
    /// `EASYNET_NODE_ID`.
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Start `easynet-daemon`, or return a handle to the already-live
    /// daemon when the control endpoint is accepting.
    pub fn start(&self) -> Result<DaemonHandle> {
        let endpoints = DaemonEndpoints::current();
        if local_daemon_grpc::probe_accepting(&endpoints.control) {
            return Ok(DaemonHandle {
                child: None,
                pid: discover_existing_daemon_pid(),
                endpoints,
            });
        }

        let binary = self.resolve_daemon_bin();
        let log_path = self.resolve_log_path();
        let child = self.spawn_child(&binary, &log_path)?;
        let pid = child.id();
        if let Err(err) = write_daemon_pid(pid) {
            let _ = net::kill_and_wait(pid, DEFAULT_STOP_TIMEOUT);
            return Err(err);
        }
        Ok(DaemonHandle {
            child: Some(child),
            pid: Some(pid),
            endpoints,
        })
    }

    fn resolve_daemon_bin(&self) -> PathBuf {
        if let Some(path) = &self.daemon_bin {
            return path.clone();
        }
        std::env::var_os("EASYNET_DAEMON_BIN")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join(DEFAULT_DAEMON_BIN)))
            })
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DAEMON_BIN))
    }

    fn resolve_log_path(&self) -> PathBuf {
        self.log_path.clone().unwrap_or_else(default_log_path)
    }

    fn spawn_child(&self, binary: &Path, log_path: &Path) -> Result<Child> {
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| DaemonError::CreateLogDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)
            .map_err(|source| DaemonError::OpenLog {
                path: log_path.to_path_buf(),
                source,
            })?;

        let mut cmd = Command::new(binary);
        cmd.env("EASYNET_NODE_ID", self.node_id.trim());
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
        cmd.stdin(Stdio::null());
        if let Ok(stdout) = log.try_clone() {
            cmd.stdout(Stdio::from(stdout));
        }
        cmd.stderr(Stdio::from(log));

        #[cfg(unix)]
        if self.detach {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if libc::signal(libc::SIGHUP, libc::SIG_IGN) == libc::SIG_ERR {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }

        cmd.spawn().map_err(|source| DaemonError::Spawn {
            binary: binary.to_path_buf(),
            source,
        })
    }
}

/// Runtime endpoints exposed by the local daemon.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DaemonEndpoints {
    control: PathBuf,
    invocation: PathBuf,
}

impl DaemonEndpoints {
    /// Resolve endpoints from the current process environment and
    /// daemon configuration files.
    pub fn current() -> Self {
        Self {
            control: transport::default_socket_path(),
            invocation: local_daemon_grpc::resolve_socket_path(),
        }
    }

    /// Legacy control-plane endpoint (`control.sock` or named pipe).
    pub fn control(&self) -> &Path {
        &self.control
    }

    /// Axon Invocation endpoint (`daemon.sock` or named pipe).
    pub fn invocation(&self) -> &Path {
        &self.invocation
    }
}

/// Handle returned by `DaemonConfig::start`.
///
/// Invariants:
/// 1. `child.is_some()` means this handle owns the process it just
///    spawned. `child.is_none()` means a daemon was already alive.
/// 2. `pid` is best-effort discovery for status/stop; endpoint probes
///    are the authoritative liveness checks.
/// 3. Dropping the handle does not stop the daemon. Call `stop()` for
///    explicit teardown, matching the CLI's background lifecycle.
#[derive(Debug)]
pub struct DaemonHandle {
    child: Option<Child>,
    pid: Option<u32>,
    endpoints: DaemonEndpoints,
}

impl DaemonHandle {
    /// PID of the daemon process, when known.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Borrow the spawned child process, when this handle started it.
    pub fn child_mut(&mut self) -> Option<&mut Child> {
        self.child.as_mut()
    }

    /// Local control endpoint.
    pub fn control_endpoint(&self) -> &Path {
        self.endpoints.control()
    }

    /// Local Axon Invocation endpoint.
    pub fn invocation_endpoint(&self) -> &Path {
        self.endpoints.invocation()
    }

    /// Snapshot daemon liveness through pid and endpoint probes.
    pub fn status(&self) -> DaemonStatus {
        DaemonStatus::from_parts(self.pid, self.endpoints.clone())
    }

    /// Stop this daemon if a PID is known.
    pub fn stop(&self) -> Result<()> {
        let Some(pid) = self.pid else {
            return Ok(());
        };
        stop_pid(pid, DEFAULT_STOP_TIMEOUT)?;
        let _ = std::fs::remove_file(config::easynet_daemon_pid_path());
        Ok(())
    }
}

/// Current daemon liveness snapshot.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DaemonStatus {
    pid: Option<u32>,
    pid_alive: bool,
    control_accepting: bool,
    invocation_accepting: bool,
    endpoints: DaemonEndpoints,
}

impl DaemonStatus {
    /// Resolve status for the current host without starting a daemon.
    pub fn current() -> Self {
        let endpoints = DaemonEndpoints::current();
        Self::from_parts(discover_existing_daemon_pid(), endpoints)
    }

    fn from_parts(pid: Option<u32>, endpoints: DaemonEndpoints) -> Self {
        let pid_alive = pid.is_some_and(net::is_pid_alive);
        let control_accepting = local_daemon_grpc::probe_accepting(&endpoints.control);
        let invocation_accepting = local_daemon_grpc::probe_accepting(&endpoints.invocation);
        Self {
            pid,
            pid_alive,
            control_accepting,
            invocation_accepting,
            endpoints,
        }
    }

    /// PID of the daemon process, when known.
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Whether the known PID is alive.
    pub fn pid_alive(&self) -> bool {
        self.pid_alive
    }

    /// Whether the control endpoint accepts connections.
    pub fn control_accepting(&self) -> bool {
        self.control_accepting
    }

    /// Whether the Invocation endpoint accepts connections.
    pub fn invocation_accepting(&self) -> bool {
        self.invocation_accepting
    }

    /// Endpoints this status checked.
    pub fn endpoints(&self) -> &DaemonEndpoints {
        &self.endpoints
    }
}

/// Stop the daemon named by the SDK pidfile, if it exists.
pub fn stop_daemon() -> Result<()> {
    let Some(pid) = read_daemon_pid() else {
        return Ok(());
    };
    stop_pid(pid, DEFAULT_STOP_TIMEOUT)?;
    let _ = std::fs::remove_file(config::easynet_daemon_pid_path());
    Ok(())
}

/// Start `easynet-daemon` with the supplied SDK config.
pub fn start_daemon(config: &DaemonConfig) -> Result<DaemonHandle> {
    config.start()
}

/// Connect to the local daemon's Axon Invocation endpoint.
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
}

/// Complete unary Invocation submitted through `DaemonClient`.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
pub struct DaemonInvocation {
    caller_ura: String,
    callee_ura: String,
    ability: String,
    subject_ura: String,
    nonce: [u8; 16],
    causal_context: easynet_axon::pb::axon::v1::CausalContext,
    args: Vec<u8>,
    content_type: String,
}

#[cfg(feature = "axon-pb")]
impl DaemonInvocation {
    /// Start building a complete Invocation. A fresh nonce is
    /// generated immediately so callers can inspect it before
    /// dispatch.
    pub fn builder(
        caller_ura: impl Into<String>,
        callee_ura: impl Into<String>,
        ability: impl Into<String>,
        subject_ura: impl Into<String>,
    ) -> Result<DaemonInvocationBuilder> {
        DaemonInvocationBuilder::new(caller_ura, callee_ura, ability, subject_ura)
    }

    /// Caller URA.
    pub fn caller_ura(&self) -> &str {
        &self.caller_ura
    }

    /// Callee URA.
    pub fn callee_ura(&self) -> &str {
        &self.callee_ura
    }

    /// Ability/function name.
    pub fn ability(&self) -> &str {
        &self.ability
    }

    /// Subject URA.
    pub fn subject_ura(&self) -> &str {
        &self.subject_ura
    }

    /// Invocation nonce.
    pub fn nonce(&self) -> [u8; 16] {
        self.nonce
    }

    /// Causal context carried in the request envelope.
    pub fn causal_context(&self) -> &easynet_axon::pb::axon::v1::CausalContext {
        &self.causal_context
    }

    /// Raw ability arguments.
    pub fn args(&self) -> &[u8] {
        &self.args
    }

    /// Request content type.
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    fn into_request(self) -> Result<easynet_axon::pb::axon::v1::InvokeRequest> {
        use easynet_axon::pb::axon::v1::{AgentIdentity, Envelope, InvokeRequest, SubjectIdentity};
        let envelope = Envelope {
            caller: Some(AgentIdentity {
                ura: self.caller_ura,
                profile: crate::services::invocation_transport::DEFAULT_URA_PROFILE.to_string(),
            }),
            callee: Some(AgentIdentity {
                ura: self.callee_ura,
                profile: crate::services::invocation_transport::DEFAULT_URA_PROFILE.to_string(),
            }),
            subject: Some(SubjectIdentity {
                ura: self.subject_ura,
                profile: crate::services::invocation_transport::DEFAULT_URA_PROFILE.to_string(),
            }),
            invocation_nonce: self.nonce.to_vec(),
            causal_context: Some(self.causal_context),
            ..Envelope::default()
        };
        Ok(InvokeRequest {
            envelope: Some(envelope),
            function_name: self.ability,
            arguments: self.args,
            content_type: self.content_type,
            ..InvokeRequest::default()
        })
    }
}

/// Builder for `DaemonInvocation`.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone)]
pub struct DaemonInvocationBuilder {
    caller_ura: String,
    callee_ura: String,
    ability: String,
    subject_ura: String,
    nonce: [u8; 16],
    causal_context: easynet_axon::pb::axon::v1::CausalContext,
    args: Vec<u8>,
    content_type: String,
}

#[cfg(feature = "axon-pb")]
impl DaemonInvocationBuilder {
    fn new(
        caller_ura: impl Into<String>,
        callee_ura: impl Into<String>,
        ability: impl Into<String>,
        subject_ura: impl Into<String>,
    ) -> Result<Self> {
        let caller_ura = checked_ura(caller_ura.into(), "caller_ura")?;
        let callee_ura = checked_ura(callee_ura.into(), "callee_ura")?;
        let subject_ura = checked_ura(subject_ura.into(), "subject_ura")?;
        let ability = ability.into();
        if ability.trim().is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "ability must not be empty".to_string(),
            ));
        }
        Ok(Self {
            caller_ura,
            callee_ura,
            ability: ability.trim().to_string(),
            subject_ura,
            nonce: easynet_axon::invocation::fresh_nonce(),
            causal_context: empty_causal_context(),
            args: Vec::new(),
            content_type: "application/json".to_string(),
        })
    }

    /// Override the generated nonce. Primarily for deterministic
    /// tests and receipt-chain replay fixtures.
    pub fn nonce(mut self, nonce: [u8; 16]) -> Self {
        self.nonce = nonce;
        self
    }

    /// Override the default root causal context.
    pub fn causal_context(
        mut self,
        causal_context: easynet_axon::pb::axon::v1::CausalContext,
    ) -> Self {
        self.causal_context = causal_context;
        self
    }

    /// Supply raw argument bytes and content type.
    pub fn args_bytes(
        mut self,
        args: impl Into<Vec<u8>>,
        content_type: impl Into<String>,
    ) -> Result<Self> {
        let content_type = content_type.into();
        if content_type.trim().is_empty() {
            return Err(DaemonError::InvalidInvocation(
                "content_type must not be empty".to_string(),
            ));
        }
        self.args = args.into();
        self.content_type = content_type.trim().to_string();
        Ok(self)
    }

    /// Supply JSON arguments.
    pub fn args_json(mut self, value: &serde_json::Value) -> Result<Self> {
        self.args = serde_json::to_vec(value).map_err(DaemonError::EncodeArguments)?;
        self.content_type = "application/json".to_string();
        Ok(self)
    }

    /// Finish building the Invocation.
    pub fn build(self) -> DaemonInvocation {
        DaemonInvocation {
            caller_ura: self.caller_ura,
            callee_ura: self.callee_ura,
            ability: self.ability,
            subject_ura: self.subject_ura,
            nonce: self.nonce,
            causal_context: self.causal_context,
            args: self.args,
            content_type: self.content_type,
        }
    }
}

#[cfg(feature = "axon-pb")]
fn checked_ura(value: String, field: &str) -> Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(DaemonError::InvalidInvocation(format!(
            "{field} must not be empty"
        )));
    }
    crate::ura::parse_ura(&value).map_err(|err| {
        DaemonError::InvalidInvocation(format!("{field} is not a valid URA: {err}"))
    })?;
    Ok(value)
}

#[cfg(feature = "axon-pb")]
fn empty_causal_context() -> easynet_axon::pb::axon::v1::CausalContext {
    use easynet_axon::pb::axon::v1::{causal_context, CausalContext, Empty};
    CausalContext {
        form: Some(causal_context::Form::None(Empty {})),
    }
}

fn default_log_path() -> PathBuf {
    config::state_dir().join("logs").join("easynet-daemon.log")
}

fn read_daemon_pid() -> Option<u32> {
    std::fs::read_to_string(config::easynet_daemon_pid_path())
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

fn write_daemon_pid(pid: u32) -> Result<()> {
    let pid_path = config::easynet_daemon_pid_path();
    if let Some(parent) = pid_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| DaemonError::CreatePidDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(&pid_path, pid.to_string()).map_err(|source| DaemonError::WritePid {
        path: pid_path,
        source,
    })
}

fn discover_existing_daemon_pid() -> Option<u32> {
    if let Some(pid) = read_daemon_pid().filter(|pid| net::is_pid_alive(*pid)) {
        return Some(pid);
    }

    #[cfg(windows)]
    {
        None
    }

    #[cfg(not(windows))]
    {
        let output = Command::new("pgrep")
            .args(["-f", DEFAULT_DAEMON_BIN])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .find(|pid| *pid != std::process::id() && net::is_pid_alive(*pid))
    }
}

fn stop_pid(pid: u32, timeout: Duration) -> Result<()> {
    if !net::is_pid_alive(pid) {
        return Err(DaemonError::PidNotAlive { pid });
    }
    if !net::is_easynet_process(pid) {
        return Err(DaemonError::PidReuseRefused { pid });
    }
    if net::kill_and_wait(pid, timeout) {
        Ok(())
    } else {
        Err(DaemonError::StopTimedOut {
            pid,
            timeout_ms: timeout.as_millis() as u64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_config_rejects_empty_node_id() {
        assert!(matches!(
            DaemonConfig::device("  "),
            Err(DaemonError::EmptyNodeId)
        ));
    }

    #[test]
    fn endpoints_current_uses_control_and_invocation_paths() {
        let endpoints = DaemonEndpoints::current();
        assert!(
            endpoints.control().ends_with("control.sock") || cfg!(windows),
            "control endpoint should resolve to control.sock on Unix"
        );
        assert!(
            endpoints.invocation().ends_with("daemon.sock") || cfg!(windows),
            "invocation endpoint should resolve to daemon.sock on Unix"
        );
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn invocation_builder_keeps_complete_tuple_inspectable() {
        let invocation = DaemonInvocation::builder(
            "easynet:///r/acme/device/dev-a",
            "easynet:///r/acme/hub",
            "device.observe.health",
            "easynet:///r/acme/device/dev-a",
        )
        .unwrap()
        .nonce([0x42; 16])
        .args_json(&serde_json::json!({"ok": true}))
        .unwrap()
        .build();

        assert_eq!(invocation.caller_ura(), "easynet:///r/acme/device/dev-a");
        assert_eq!(invocation.callee_ura(), "easynet:///r/acme/hub");
        assert_eq!(invocation.ability(), "device.observe.health");
        assert_eq!(invocation.subject_ura(), "easynet:///r/acme/device/dev-a");
        assert_eq!(invocation.nonce(), [0x42; 16]);
        assert_eq!(invocation.content_type(), "application/json");
        assert!(!invocation.args().is_empty());
    }

    #[cfg(feature = "axon-pb")]
    #[test]
    fn invocation_builder_rejects_invalid_ura() {
        let err = DaemonInvocation::builder(
            "not-a-ura",
            "easynet:///r/acme/hub",
            "x",
            "easynet:///r/acme/hub",
        )
        .unwrap_err();
        assert!(format!("{err}").contains("caller_ura"));
    }
}
