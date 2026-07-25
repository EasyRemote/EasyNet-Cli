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

use super::super::identity_fact::DeviceNodeIdFact;
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
        | RuntimeLifecycleStatus::StopTimedOut => {
            return Err(RuntimeLifecycleError::StartRefusedMissingDaemonIdentity);
        }
        RuntimeLifecycleStatus::DaemonDiscoveryInvalid => {
            return Err(RuntimeLifecycleError::StartRefusedInvalidDaemonDiscovery {
                message: report
                    .daemon()
                    .control_discovery_error()
                    .unwrap_or("lifecycle invariant violated: invalid discovery status has no discovery error")
                    .to_string(),
            });
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
    validate_attach_capabilities(request, report)?;
    Ok(())
}

fn validate_attach_capabilities(
    request: &RuntimeStartRequest,
    report: &RuntimeStatusReport,
) -> Result<(), RuntimeLifecycleError> {
    if !matches!(request.mode, RuntimeStartMode::Device) {
        return Ok(());
    }
    if report
        .daemon()
        .has_capability_flag(crate::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER)
    {
        return Ok(());
    }
    Err(
        RuntimeLifecycleError::StartRefusedMissingRuntimeCapability {
            mode: "device",
            capability: crate::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER,
        },
    )
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
    let requested = DeviceNodeIdFact::from_optional(request.node_id.as_deref());
    let actual = DeviceNodeIdFact::from_optional(identity.node_id.as_deref());
    match (requested.present_value(), actual.present_value()) {
        (Some(requested), Some(actual)) if requested == actual => Ok(()),
        _ => Err(RuntimeLifecycleError::StartRefusedIdentityMismatch {
            field: "node_id",
            requested: requested.mismatch_value(),
            actual: actual.mismatch_value(),
        }),
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
            capability_flags: vec![discovery::flags::PAIRED_USER_RUNTIME_SIGNER.to_string()],
            pages_port: None,
        }
    }

    fn discovery_with_identity_and_flags(
        identity: discovery::DaemonIdentity,
        capability_flags: Vec<String>,
    ) -> discovery::ControlDiscovery {
        discovery::ControlDiscovery {
            capability_flags,
            ..discovery_with_identity(identity)
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
    fn start_preflight_refuses_invalid_daemon_discovery() {
        let daemon =
            DaemonDiscoverySnapshot::from_invalid_discovery("control.json malformed", endpoints());
        let status = RuntimeStatusReport::from_parts(None, daemon);

        let err = preflight_start(&RuntimeStartRequest::hub("tenant-test"), &status)
            .expect_err("invalid daemon discovery must be refused");

        assert!(
            matches!(
                err,
                RuntimeLifecycleError::StartRefusedInvalidDaemonDiscovery { ref message }
                    if message.contains("control.json malformed")
            ),
            "invalid discovery must remain the explicit refusal cause: {err}"
        );
    }

    #[test]
    fn start_preflight_refuses_device_attach_without_paired_user_signer_readiness() {
        let daemon = DaemonDiscoverySnapshot::from_parts(
            Some(discovery_with_identity_and_flags(identity(), Vec::new())),
            Some(std::process::id()),
            true,
            true,
            true,
            endpoints(),
        );
        let status = RuntimeStatusReport::from_parts(None, daemon);

        let err = preflight_start(
            &RuntimeStartRequest::device("tenant-test", "node-test"),
            &status,
        )
        .expect_err("device attach must require paired user signer readiness");

        assert!(matches!(
            err,
            RuntimeLifecycleError::StartRefusedMissingRuntimeCapability {
                capability: discovery::flags::PAIRED_USER_RUNTIME_SIGNER,
                ..
            }
        ));
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

    #[test]
    fn start_preflight_refuses_missing_requested_device_node_id() {
        let daemon = DaemonDiscoverySnapshot::from_parts(
            Some(discovery_with_identity(identity())),
            Some(std::process::id()),
            true,
            true,
            true,
            endpoints(),
        );
        let status = RuntimeStatusReport::from_parts(None, daemon);
        let request = RuntimeStartRequest {
            mode: RuntimeStartMode::Device,
            realm: "tenant-test".to_string(),
            node_id: None,
        };

        let err =
            preflight_start(&request, &status).expect_err("missing requested node id must fail");

        assert!(matches!(
            err,
            RuntimeLifecycleError::StartRefusedIdentityMismatch {
                field: "node_id",
                requested,
                actual,
            } if requested == "<missing>" && actual == "node-test"
        ));
    }

    #[test]
    fn start_preflight_refuses_blank_requested_device_node_id() {
        let daemon = DaemonDiscoverySnapshot::from_parts(
            Some(discovery_with_identity(identity())),
            Some(std::process::id()),
            true,
            true,
            true,
            endpoints(),
        );
        let status = RuntimeStatusReport::from_parts(None, daemon);
        let request = RuntimeStartRequest::device("tenant-test", "   ");

        let err =
            preflight_start(&request, &status).expect_err("blank requested node id must fail");

        assert!(matches!(
            err,
            RuntimeLifecycleError::StartRefusedIdentityMismatch {
                field: "node_id",
                requested,
                actual,
            } if requested == "<blank>" && actual == "node-test"
        ));
    }

    #[test]
    fn start_preflight_refuses_missing_discovered_device_node_id() {
        let mut daemon_identity = identity();
        daemon_identity.node_id = None;
        let daemon = DaemonDiscoverySnapshot::from_parts(
            Some(discovery_with_identity(daemon_identity)),
            Some(std::process::id()),
            true,
            true,
            true,
            endpoints(),
        );
        let status = RuntimeStatusReport::from_parts(None, daemon);

        let err = preflight_start(
            &RuntimeStartRequest::device("tenant-test", "node-test"),
            &status,
        )
        .expect_err("missing discovered node id must fail");

        assert!(matches!(
            err,
            RuntimeLifecycleError::StartRefusedIdentityMismatch {
                field: "node_id",
                requested,
                actual,
            } if requested == "node-test" && actual == "<missing>"
        ));
    }

    #[test]
    fn start_preflight_refuses_blank_discovered_device_node_id() {
        let mut daemon_identity = identity();
        daemon_identity.node_id = Some("   ".to_string());
        let daemon = DaemonDiscoverySnapshot::from_parts(
            Some(discovery_with_identity(daemon_identity)),
            Some(std::process::id()),
            true,
            true,
            true,
            endpoints(),
        );
        let status = RuntimeStatusReport::from_parts(None, daemon);

        let err = preflight_start(
            &RuntimeStartRequest::device("tenant-test", "node-test"),
            &status,
        )
        .expect_err("blank discovered node id must fail");

        assert!(matches!(
            err,
            RuntimeLifecycleError::StartRefusedIdentityMismatch {
                field: "node_id",
                requested,
                actual,
            } if requested == "node-test" && actual == "<blank>"
        ));
    }
}
