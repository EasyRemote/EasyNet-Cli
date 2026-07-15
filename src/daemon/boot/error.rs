use std::path::PathBuf;

use thiserror::Error;

/// Errors emitted by the daemon SDK boundary.
///
/// What this enum is: the typed failure surface for controlling and
/// calling the EasyNet product daemon from Rust and C ABI adapters.
///
/// What this enum is not: it is not an Axon protocol error taxonomy.
/// Axon gRPC status is preserved inside the status variants so callers
/// can map it without flattening admission failures into daemon liveness.
#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("daemon node id must not be empty")]
    EmptyNodeId,
    #[error("daemon binary path must not be empty")]
    EmptyBinaryPath,
    #[error("daemon working directory must not be empty")]
    EmptyWorkingDir,
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
    #[error("failed to probe daemon child pid {pid}: {source}")]
    ProbeChild { pid: u32, source: std::io::Error },
    #[error(
        "daemon pid {pid} exited before endpoints were ready: status={status}, control={control}, invocation={invocation}"
    )]
    ExitedBeforeReady {
        pid: u32,
        status: String,
        control: PathBuf,
        invocation: PathBuf,
    },
    #[error(
        "daemon pid {pid} did not make endpoints ready within {timeout_ms} ms: control={control}, invocation={invocation}"
    )]
    ReadyTimedOut {
        pid: u32,
        timeout_ms: u64,
        control: PathBuf,
        invocation: PathBuf,
    },
    #[error("pid {pid} is no longer alive")]
    PidNotAlive { pid: u32 },
    #[error("pid {pid} is not an EasyNet process")]
    PidReuseRefused { pid: u32 },
    #[error("pid {pid} did not exit within {timeout_ms} ms")]
    StopTimedOut { pid: u32, timeout_ms: u64 },
    #[error("failed to signal daemon child pid {pid}: {source}")]
    SignalChild { pid: u32, source: std::io::Error },
    #[error("failed to wait for daemon child pid {pid}: {source}")]
    WaitChild { pid: u32, source: std::io::Error },
    #[error("daemon invocation transport is not reachable at {endpoint}")]
    InvocationEndpointDown { endpoint: PathBuf },
    #[cfg(feature = "axon-pb")]
    #[error(
        "daemon discovery at {control} did not advertise invocation_endpoint; daemon is not ready"
    )]
    InvocationEndpointMissing { control: PathBuf },
    #[error(
        "daemon control endpoint is accepting at {control}, but invocation endpoint is down at {invocation}"
    )]
    ControlAliveInvocationDown {
        control: PathBuf,
        invocation: PathBuf,
    },
    #[error("daemon discovery file is missing at {path}")]
    DiscoveryMissing { path: PathBuf },
    #[error("daemon discovery file at {path} does not advertise daemon identity")]
    DiscoveryIdentityMissing { path: PathBuf },
    #[error(
        "daemon discovery identity mismatch for {field}: requested={requested}, actual={actual}"
    )]
    DiscoveryIdentityMismatch {
        field: &'static str,
        requested: String,
        actual: String,
    },
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
    #[cfg(feature = "axon-pb")]
    #[error("daemon returned gRPC stream status for {ability}: code={code:?}, message={message}")]
    InvokeStreamStatus {
        ability: String,
        code: tonic::Code,
        message: String,
    },
    #[cfg(feature = "axon-pb")]
    #[error("daemon returned gRPC bidi status for {ability}: code={code:?}, message={message}")]
    InvokeBidiStatus {
        ability: String,
        code: tonic::Code,
        message: String,
    },
    #[cfg(feature = "axon-pb")]
    #[error("daemon bidi session for {ability} is closed")]
    InvokeBidiClosed { ability: String },
}
