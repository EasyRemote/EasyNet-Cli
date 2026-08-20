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
    #[error("daemon runtime HOME is required for {context}")]
    DaemonHomeUnavailable { context: &'static str },
    #[error("daemon state root is unavailable for {context}: {source}")]
    DaemonStateRootUnavailable {
        context: &'static str,
        source: anyhow::Error,
    },
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
    #[error("failed to acquire daemon process lease at {path}: {source}")]
    AcquireProcessLease {
        path: PathBuf,
        source: anyhow::Error,
    },
    #[error("daemon process lease at {path} is already held by {owner}")]
    ProcessLeaseHeld { path: PathBuf, owner: String },
    #[error(
        "daemon pid {pid} is alive while runtime endpoints are unavailable: control={control}, invocation={invocation}"
    )]
    ProcessAliveEndpointsDown {
        pid: u32,
        control: PathBuf,
        invocation: PathBuf,
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

/// Typed daemon invocation failure projection for adapter boundaries.
///
/// FFI and language bindings consume this enum instead of inspecting
/// `DaemonError` display strings. The daemon SDK boundary owns the small
/// amount of transport-detail classification needed to preserve canonical
/// runtime readiness states across the process boundary.
#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonInvocationErrorProjection {
    DaemonDown,
    CallerSignerUnavailable,
    DescriptorOwnerOffline,
    TransportEnvelopeExceeded,
    Status(tonic::Code),
    InvalidInvocation,
    Cancelled,
    Generic,
}

#[cfg(feature = "axon-pb")]
impl DaemonError {
    pub fn invocation_error_projection(&self) -> DaemonInvocationErrorProjection {
        use crate::daemon::runtime_failure::RuntimeFailureFacts;

        match self {
            Self::InvocationEndpointDown { .. }
            | Self::InvocationEndpointMissing { .. }
            | Self::Connect { .. } => DaemonInvocationErrorProjection::DaemonDown,
            Self::InvokeStatus { code, message, .. }
            | Self::InvokeStreamStatus { code, message, .. }
            | Self::InvokeBidiStatus { code, message, .. }
                if RuntimeFailureFacts::is_descriptor_owner_offline_status(*code, message) =>
            {
                DaemonInvocationErrorProjection::DescriptorOwnerOffline
            }
            Self::InvokeStatus { code, message, .. }
            | Self::InvokeStreamStatus { code, message, .. }
            | Self::InvokeBidiStatus { code, message, .. }
                if matches!(
                    code,
                    tonic::Code::OutOfRange | tonic::Code::ResourceExhausted
                ) && message.contains("message length too large") =>
            {
                DaemonInvocationErrorProjection::TransportEnvelopeExceeded
            }
            Self::InvokeStatus { code, .. }
            | Self::InvokeStreamStatus { code, .. }
            | Self::InvokeBidiStatus { code, .. } => DaemonInvocationErrorProjection::Status(*code),
            Self::InvalidInvocation(message)
                if RuntimeFailureFacts::is_caller_signer_unavailable_message(message) =>
            {
                DaemonInvocationErrorProjection::CallerSignerUnavailable
            }
            Self::InvalidInvocation(_) => DaemonInvocationErrorProjection::InvalidInvocation,
            Self::InvokeBidiClosed { .. } => DaemonInvocationErrorProjection::Cancelled,
            _ => DaemonInvocationErrorProjection::Generic,
        }
    }
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::{DaemonError, DaemonInvocationErrorProjection};

    #[test]
    fn projects_caller_signer_unavailable_without_adapter_message_parsing() {
        let error = DaemonError::InvalidInvocation(
            "CALLER_SIGNER_UNAVAILABLE: remote invocation requires a caller signer".to_string(),
        );

        assert_eq!(
            error.invocation_error_projection(),
            DaemonInvocationErrorProjection::CallerSignerUnavailable
        );
    }

    #[test]
    fn projects_descriptor_owner_offline_without_adapter_message_parsing() {
        let error = DaemonError::InvokeStatus {
            ability: "meta.list_abilities".to_string(),
            code: tonic::Code::Unavailable,
            message: "DESCRIPTOR_OWNER_OFFLINE: descriptor owner is not online".to_string(),
        };

        assert_eq!(
            error.invocation_error_projection(),
            DaemonInvocationErrorProjection::DescriptorOwnerOffline
        );
    }

    #[test]
    fn route_text_does_not_project_descriptor_owner_offline() {
        let error = DaemonError::InvokeStatus {
            ability: "meta.list_abilities".to_string(),
            code: tonic::Code::Unavailable,
            message: "ROUTE_NEGATIVE: namespace.resolve negative: \
                 NEGATIVE_REASON_NXDOMAIN: owner is not online"
                .to_string(),
        };

        assert_eq!(
            error.invocation_error_projection(),
            DaemonInvocationErrorProjection::Status(tonic::Code::Unavailable)
        );
    }

    #[test]
    fn preserves_plain_unavailable_as_daemon_down_projection() {
        let error = DaemonError::InvokeStatus {
            ability: "observe.health".to_string(),
            code: tonic::Code::Unavailable,
            message: "transport unavailable".to_string(),
        };

        assert_eq!(
            error.invocation_error_projection(),
            DaemonInvocationErrorProjection::Status(tonic::Code::Unavailable)
        );
    }

    #[test]
    fn projects_decoded_message_overflow_as_transport_capacity() {
        let error = DaemonError::InvokeStatus {
            ability: "invocation.history.list".to_string(),
            code: tonic::Code::OutOfRange,
            message: "Error, decoded message length too large: found 6607756 bytes, the limit is: 4194304 bytes".to_string(),
        };

        assert_eq!(
            error.invocation_error_projection(),
            DaemonInvocationErrorProjection::TransportEnvelopeExceeded
        );
    }

    #[test]
    fn preserves_domain_out_of_range_as_status() {
        let error = DaemonError::InvokeStatus {
            ability: "media.seek".to_string(),
            code: tonic::Code::OutOfRange,
            message: "position exceeds media duration".to_string(),
        };

        assert_eq!(
            error.invocation_error_projection(),
            DaemonInvocationErrorProjection::Status(tonic::Code::OutOfRange)
        );
    }
}
