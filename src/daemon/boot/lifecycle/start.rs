//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/boot/lifecycle/start.rs
//! Description: start preflight for daemon lifecycle.
//!
//! Protocol Responsibility:
//! - Ensures start attaches only to a daemon whose local lifecycle identity
//!   matches the requested EasyNet product role.
//!
//! Implementation Approach:
//! - Runs a pure status decision plus explicit identity validation before CLI
//!   code asks the daemon SDK to spawn or attach.
//!
//! Usage Contract:
//! - Callers must build the request after resolving credentials or hub config;
//!   preflight without requested identity is not attach-safe.
//!
//! Architectural Position:
//! - `daemon::boot::lifecycle` start state machine.
//!
//! This module owns the decision of what `easynet start` should do
//! before it asks `DaemonStartConfig` to start or attach. It does not
//! parse CLI flags and it does not spawn the daemon.

use crate::daemon::control::discovery::DaemonIdentity;

use super::{RuntimeLifecycleError, RuntimeLifecycleStatus, RuntimeStatusReport};

/// Requested daemon identity for start attach decisions.
///
/// Invariants:
/// 1. Device requests carry a non-empty node id.
/// 2. Hub requests accept daemon identities `hub` or `both`.
/// 3. Realm comparison is exact after caller-side config resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStartRequest {
    mode: RuntimeStartMode,
    realm: String,
    node_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeStartMode {
    Device,
    Hub,
}

impl RuntimeStartRequest {
    /// Build a device-mode start request.
    pub fn device(realm: impl Into<String>, node_id: impl Into<String>) -> Self {
        Self {
            mode: RuntimeStartMode::Device,
            realm: realm.into(),
            node_id: Some(node_id.into()),
        }
    }

    /// Build a hub-mode start request.
    pub fn hub(realm: impl Into<String>) -> Self {
        Self {
            mode: RuntimeStartMode::Hub,
            realm: realm.into(),
            node_id: None,
        }
    }

    /// Requested realm.
    pub fn realm(&self) -> &str {
        &self.realm
    }
}

/// Action selected by start preflight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStartPreflightAction {
    /// No daemon facts and no projection were found.
    CleanStart,
    /// A stale projection was removed because no daemon fact remained.
    RemovedStaleProjection,
    /// A daemon is live but `runtime.json` is absent; start should attach
    /// and rebuild the projection after Ready.
    AttachAndRebuildProjection,
    /// Projection and daemon facts already describe a live daemon.
    AlreadyRunning,
}

/// Result of start preflight.
///
/// Invariants:
/// 1. `RemovedStaleProjection` is returned only after `runtime.json`
///    was removed successfully or was already absent.
/// 2. `AttachAndRebuildProjection` never starts a second daemon; the
///    subsequent daemon SDK start path must attach by identity.
/// 3. `ControlOnlyInvocationDown` is represented as an error, because
///    product start must not attach to a daemon that cannot accept
///    Invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeStartPreflightReport {
    action: RuntimeStartPreflightAction,
}

impl RuntimeStartPreflightReport {
    /// Build a preflight report from an already selected action.
    pub fn new(action: RuntimeStartPreflightAction) -> Self {
        Self { action }
    }

    /// Selected preflight action.
    pub fn action(&self) -> RuntimeStartPreflightAction {
        self.action
    }
}

pub(crate) fn preflight_start(
    request: &RuntimeStartRequest,
    report: &RuntimeStatusReport,
) -> Result<RuntimeStartPreflightReport, RuntimeLifecycleError> {
    let action = match report.status() {
        RuntimeLifecycleStatus::Stopped => RuntimeStartPreflightAction::CleanStart,
        RuntimeLifecycleStatus::ProjectionPresentProcessMissing => {
            RuntimeStartPreflightAction::RemovedStaleProjection
        }
        RuntimeLifecycleStatus::ProjectionMissingProcessRunning => {
            validate_attach_identity(request, report)?;
            RuntimeStartPreflightAction::AttachAndRebuildProjection
        }
        RuntimeLifecycleStatus::Running => {
            validate_attach_identity(request, report)?;
            RuntimeStartPreflightAction::AlreadyRunning
        }
        RuntimeLifecycleStatus::ControlOnlyInvocationDown => {
            return Err(
                RuntimeLifecycleError::StartRefusedControlOnlyInvocationDown {
                    control: report.daemon().endpoints().control().to_path_buf(),
                    invocation: report.daemon().endpoints().invocation().to_path_buf(),
                },
            );
        }
        RuntimeLifecycleStatus::LegacyAxonBridge => {
            return Err(RuntimeLifecycleError::StartRefusedLegacyAxonBridge);
        }
        RuntimeLifecycleStatus::IdentityMismatch => {
            return Err(RuntimeLifecycleError::StartRefusedIdentityMismatch {
                field: "identity",
                requested: request.realm().to_string(),
                actual: RuntimeLifecycleStatus::IdentityMismatch
                    .as_wire_str()
                    .to_string(),
            });
        }
        RuntimeLifecycleStatus::StartProjectionCommitFailed
        | RuntimeLifecycleStatus::StopTimedOut
        | RuntimeLifecycleStatus::LegacyCleanupFailed => {
            return Err(RuntimeLifecycleError::StartRefusedMissingDaemonIdentity);
        }
    };
    Ok(RuntimeStartPreflightReport::new(action))
}

fn validate_attach_identity(
    request: &RuntimeStartRequest,
    report: &RuntimeStatusReport,
) -> Result<(), RuntimeLifecycleError> {
    let identity = report
        .daemon()
        .identity()
        .ok_or(RuntimeLifecycleError::StartRefusedMissingDaemonIdentity)?;
    validate_mode(request, identity)?;
    validate_realm(request, identity)?;
    validate_node_id(request, identity)?;
    Ok(())
}

fn validate_mode(
    request: &RuntimeStartRequest,
    identity: &DaemonIdentity,
) -> Result<(), RuntimeLifecycleError> {
    let ok = match request.mode {
        RuntimeStartMode::Device => identity.mode == "device",
        RuntimeStartMode::Hub => identity.mode == "hub" || identity.mode == "both",
    };
    if ok {
        Ok(())
    } else {
        Err(RuntimeLifecycleError::StartRefusedIdentityMismatch {
            field: "mode",
            requested: match request.mode {
                RuntimeStartMode::Device => "device",
                RuntimeStartMode::Hub => "hub|both",
            }
            .to_string(),
            actual: identity.mode.clone(),
        })
    }
}

fn validate_realm(
    request: &RuntimeStartRequest,
    identity: &DaemonIdentity,
) -> Result<(), RuntimeLifecycleError> {
    if request.realm == identity.realm {
        Ok(())
    } else {
        Err(RuntimeLifecycleError::StartRefusedIdentityMismatch {
            field: "realm",
            requested: request.realm.clone(),
            actual: identity.realm.clone(),
        })
    }
}

fn validate_node_id(
    request: &RuntimeStartRequest,
    identity: &DaemonIdentity,
) -> Result<(), RuntimeLifecycleError> {
    if !matches!(request.mode, RuntimeStartMode::Device) {
        return Ok(());
    }
    let requested = request.node_id.clone().unwrap_or_default();
    let actual = identity.node_id.clone().unwrap_or_default();
    if requested == actual {
        Ok(())
    } else {
        Err(RuntimeLifecycleError::StartRefusedIdentityMismatch {
            field: "node_id",
            requested,
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::daemon::boot::DaemonEndpoints;
    use crate::daemon::control::discovery;
    use crate::daemon::lifecycle::{DaemonDiscoverySnapshot, RuntimeStatusReport};

    use super::*;

    fn endpoints() -> DaemonEndpoints {
        DaemonEndpoints {
            control: PathBuf::from("/tmp/easynet-start-control.sock"),
            invocation: PathBuf::from("/tmp/easynet-start-daemon.sock"),
        }
    }

    fn identity() -> discovery::DaemonIdentity {
        discovery::DaemonIdentity {
            mode: "device".to_string(),
            realm: "tenant-test".to_string(),
            node_id: Some("node-test".to_string()),
        }
    }

    fn discovery_with_identity(identity: discovery::DaemonIdentity) -> discovery::ControlDiscovery {
        discovery::ControlDiscovery {
            socket_path: Some(PathBuf::from("/tmp/easynet-start-control.sock")),
            pipe_name: None,
            invocation_endpoint: Some(PathBuf::from("/tmp/easynet-start-daemon.sock")),
            daemon_identity: Some(identity),
            pid: std::process::id(),
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            supported_ipc_versions: discovery::IpcVersionRange::single(discovery::IPC_VERSION_V1),
            capability_flags: Vec::new(),
            pages_port: None,
        }
    }

    #[test]
    fn start_preflight_attaches_when_projection_is_missing_but_daemon_is_live() {
        let daemon = DaemonDiscoverySnapshot::from_parts(
            Some(discovery_with_identity(identity())),
            Some(std::process::id()),
            true,
            false,
            false,
            endpoints(),
        );
        let status = RuntimeStatusReport::from_parts(None, daemon);

        let report = preflight_start(
            &RuntimeStartRequest::device("tenant-test", "node-test"),
            &status,
        )
        .expect("preflight");

        assert_eq!(
            report.action(),
            RuntimeStartPreflightAction::AttachAndRebuildProjection
        );
    }

    #[test]
    fn start_preflight_refuses_control_only_daemon() {
        let daemon =
            DaemonDiscoverySnapshot::from_parts(None, None, false, true, false, endpoints());
        let status = RuntimeStatusReport::from_parts(None, daemon);

        let err = preflight_start(&RuntimeStartRequest::hub("tenant-test"), &status)
            .expect_err("control-only daemon must be refused");

        assert!(
            matches!(
                err,
                RuntimeLifecycleError::StartRefusedControlOnlyInvocationDown { .. }
            ),
            "Invariant 3: start must not attach to control-only daemon"
        );
    }

    #[test]
    fn start_preflight_refuses_identity_mismatch() {
        let daemon = DaemonDiscoverySnapshot::from_parts(
            Some(discovery_with_identity(identity())),
            Some(std::process::id()),
            true,
            true,
            true,
            endpoints(),
        );
        let status = RuntimeStatusReport::from_parts(None, daemon);

        let err = preflight_start(
            &RuntimeStartRequest::device("tenant-test", "other-node"),
            &status,
        )
        .expect_err("mismatched daemon identity must be refused");

        assert!(matches!(
            err,
            RuntimeLifecycleError::StartRefusedIdentityMismatch {
                field: "node_id",
                ..
            }
        ));
    }
}
