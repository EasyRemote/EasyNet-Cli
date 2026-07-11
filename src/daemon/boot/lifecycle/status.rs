//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/boot/lifecycle/status.rs
//! Description: lifecycle status classification.
//!
//! Protocol Responsibility:
//! - Classifies local daemon lifecycle from process facts first and projection
//!   second.
//!
//! Implementation Approach:
//! - Pure classifier over `DaemonDiscoverySnapshot`,
//!   `RuntimeSessionProjection`, and optional product presence observation.
//!
//! Usage Contract:
//! - Callers render the report; they do not reinterpret missing projection as
//!   stopped.
//!
//! Architectural Position:
//! - `daemon::boot::lifecycle` state machine.
//!
//! Classification is intentionally pure: given a runtime projection and
//! daemon facts, it returns one domain state. CLI commands then decide
//! whether to render, start, stop, or repair.

use serde_json::{json, Value};

use super::{DaemonDiscoverySnapshot, ProductPresenceSnapshot, RuntimeSessionProjection};

/// Operator-facing lifecycle state.
///
/// Invariants:
/// 1. `Stopped` requires both absent projection and absent daemon
///    facts; missing `runtime.json` alone is not stopped.
/// 2. `ProjectionPresentProcessMissing` is degraded state, not a live
///    runtime.
/// 3. `ControlOnlyInvocationDown` is never treated as attachable start
///    state because product calls require Invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeLifecycleStatus {
    /// No projection and no daemon process facts.
    Stopped,
    /// Projection and daemon facts agree enough to treat the runtime as live.
    Running,
    /// The daemon appears live but `runtime.json` is absent.
    ProjectionMissingProcessRunning,
    /// `runtime.json` exists but no daemon process fact remains.
    ProjectionPresentProcessMissing,
    /// Control accepts but Invocation does not; the daemon is half alive.
    ControlOnlyInvocationDown,
    /// Projection describes the retired raw Axon bridge runtime.
    LegacyAxonBridge,
    /// Start refused because an existing daemon identity does not match.
    IdentityMismatch,
    /// Daemon reached Ready but projection commit failed.
    StartProjectionCommitFailed,
    /// Stop did not reach process/socket terminal postconditions.
    StopTimedOut,
    /// Legacy janitor failed while cleaning retired runtime artifacts.
    LegacyCleanupFailed,
}

impl RuntimeLifecycleStatus {
    /// Stable status string for JSON and logs.
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Running => "running",
            Self::ProjectionMissingProcessRunning => "projection_missing_process_running",
            Self::ProjectionPresentProcessMissing => "projection_present_process_missing",
            Self::ControlOnlyInvocationDown => "control_only_invocation_down",
            Self::LegacyAxonBridge => "legacy_axon_bridge",
            Self::IdentityMismatch => "identity_mismatch",
            Self::StartProjectionCommitFailed => "start_projection_commit_failed",
            Self::StopTimedOut => "stop_timed_out",
            Self::LegacyCleanupFailed => "legacy_cleanup_failed",
        }
    }
}

/// Full lifecycle report used by CLI, FFI, and tests.
///
/// Invariants:
/// 1. `status` is derived only by `classify`; callers cannot construct
///    an inconsistent report through public APIs.
/// 2. `daemon` is always present, even when it contains no live facts,
///    so stop/start code can reason from one shape.
/// 3. `projection` may be absent while daemon facts are present; that
///    state is explicitly modeled instead of folded into stopped.
#[derive(Debug, Clone)]
pub struct RuntimeStatusReport {
    projection: Option<RuntimeSessionProjection>,
    daemon: DaemonDiscoverySnapshot,
    product_presence: Option<ProductPresenceSnapshot>,
    desktop_companions: Vec<Value>,
    status: RuntimeLifecycleStatus,
}

impl RuntimeStatusReport {
    /// Build a report from already captured inputs.
    pub fn from_parts(
        projection: Option<RuntimeSessionProjection>,
        daemon: DaemonDiscoverySnapshot,
    ) -> Self {
        Self::from_parts_with_presence(projection, daemon, None)
    }

    /// Build a report from captured inputs and product presence.
    pub fn from_parts_with_presence(
        projection: Option<RuntimeSessionProjection>,
        daemon: DaemonDiscoverySnapshot,
        product_presence: Option<ProductPresenceSnapshot>,
    ) -> Self {
        Self::from_parts_with_observations(projection, daemon, product_presence, Vec::new())
    }

    /// Build a report from all captured runtime-status observations.
    pub fn from_parts_with_observations(
        projection: Option<RuntimeSessionProjection>,
        daemon: DaemonDiscoverySnapshot,
        product_presence: Option<ProductPresenceSnapshot>,
        desktop_companions: Vec<Value>,
    ) -> Self {
        let status = classify(projection.as_ref(), &daemon);
        Self {
            projection,
            daemon,
            product_presence,
            desktop_companions,
            status,
        }
    }

    /// Classified lifecycle state.
    pub fn status(&self) -> RuntimeLifecycleStatus {
        self.status
    }

    /// Runtime projection, when `runtime.json` exists.
    pub fn projection(&self) -> Option<&RuntimeSessionProjection> {
        self.projection.as_ref()
    }

    /// Daemon process facts.
    pub fn daemon(&self) -> &DaemonDiscoverySnapshot {
        &self.daemon
    }

    /// Product presence observation, when locally available.
    pub fn product_presence(&self) -> Option<&ProductPresenceSnapshot> {
        self.product_presence.as_ref()
    }

    /// Desktop companion DTOs captured for runtime status.
    pub fn desktop_companions(&self) -> &[Value] {
        &self.desktop_companions
    }

    /// JSON representation used by `easynet runtime status --json`.
    pub fn to_json(&self, connection: Value) -> Value {
        json!({
            "connection": connection,
            "runtime_status": self.status.as_wire_str(),
            "runtime": self.projection.as_ref().map(RuntimeSessionProjection::to_json),
            "daemon": self.daemon.to_json(),
            "desktop_companions": self.desktop_companions.clone(),
            "product_presence": self.product_presence.as_ref().map(ProductPresenceSnapshot::to_json),
        })
    }
}

pub(super) fn desktop_companion_statuses() -> Vec<Value> {
    let Ok(state) = crate::daemon::plugins::default_state() else {
        return Vec::new();
    };
    let manager = crate::daemon::plugins::DesktopCompanionManager::current();
    state
        .index()
        .packages()
        .iter()
        .filter(|package| {
            package.manifest().kind() == crate::daemon::plugins::PluginKind::DesktopCompanion
        })
        .filter_map(|package| manager.status_json(package).ok())
        .collect()
}

fn classify(
    projection: Option<&RuntimeSessionProjection>,
    daemon: &DaemonDiscoverySnapshot,
) -> RuntimeLifecycleStatus {
    let has_projection = projection.is_some();
    let has_daemon_fact = daemon.has_daemon_fact();

    if projection.is_some_and(RuntimeSessionProjection::uses_bridge) {
        return RuntimeLifecycleStatus::LegacyAxonBridge;
    }

    if daemon.control_accepting() && !daemon.invocation_accepting() {
        return RuntimeLifecycleStatus::ControlOnlyInvocationDown;
    }

    match (has_projection, has_daemon_fact) {
        (false, false) => RuntimeLifecycleStatus::Stopped,
        (false, true) => RuntimeLifecycleStatus::ProjectionMissingProcessRunning,
        (true, false) => RuntimeLifecycleStatus::ProjectionPresentProcessMissing,
        (true, true) => RuntimeLifecycleStatus::Running,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::daemon::boot::DaemonEndpoints;
    use crate::daemon::control::discovery;
    use crate::daemon::persistence::config;

    use super::*;

    fn endpoints() -> DaemonEndpoints {
        DaemonEndpoints {
            control: PathBuf::from("/tmp/easynet-test-control.sock"),
            invocation: PathBuf::from("/tmp/easynet-test-daemon.sock"),
        }
    }

    fn live_daemon_discovery() -> discovery::ControlDiscovery {
        discovery::ControlDiscovery {
            socket_path: Some(PathBuf::from("/tmp/easynet-test-control.sock")),
            pipe_name: None,
            invocation_endpoint: Some(PathBuf::from("/tmp/easynet-test-daemon.sock")),
            daemon_identity: Some(discovery::DaemonIdentity {
                mode: "device".to_string(),
                realm: "tenant-test".to_string(),
                node_id: Some("node-test".to_string()),
            }),
            pid: std::process::id(),
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            supported_ipc_versions: discovery::IpcVersionRange::single(discovery::IPC_VERSION_V1),
            capability_flags: vec![discovery::flags::BOOT_STATUS.to_string()],
            pages_port: Some(8787),
        }
    }

    fn daemon_with_facts(
        control_accepting: bool,
        invocation_accepting: bool,
    ) -> DaemonDiscoverySnapshot {
        DaemonDiscoverySnapshot::from_parts(
            Some(live_daemon_discovery()),
            Some(std::process::id()),
            true,
            control_accepting,
            invocation_accepting,
            endpoints(),
        )
    }

    fn projection() -> RuntimeSessionProjection {
        RuntimeSessionProjection::from_state(config::RuntimeState {
            endpoint: "/tmp/easynet-test-daemon.sock".to_string(),
            runtime_kind: config::RuntimeKind::DaemonOnly,
            pid: Some(999_999),
            hub: None,
            tenant: Some("tenant-test".to_string()),
            label: Some("node-test".to_string()),
            started_at: None,
            credential_verified: None,
        })
    }

    fn companion_status_dto() -> Value {
        crate::daemon::plugins::companion::project_status(&json!({
            "package_id": "easynet.desktop.menubar",
            "package_version": "0.1.0",
            "display_name": "EasyNet Menu Bar",
            "platform": "macos",
            "desired_state": "enabled",
            "supervisor_state": "installed_enabled",
            "observed_state": "running",
            "projected_state": "running",
            "boot_policy": "ensure_running_after_daemon_ready",
            "stop_policy": "keep_running",
            "health": "status_file",
            "pid": 12345,
            "version": "0.1.0",
            "last_seen_unix_ms": 1783411200000_u64,
            "launch_method": "launch_agent",
            "error": null,
            "metadata": {"source": "runtime_status_test"}
        }))
        .expect("companion DTO")
    }

    #[test]
    fn status_classifier_detects_projection_missing_live_daemon() {
        let report = RuntimeStatusReport::from_parts(None, daemon_with_facts(false, false));

        assert_eq!(
            report.status(),
            RuntimeLifecycleStatus::ProjectionMissingProcessRunning,
            "Invariant 1: projection absence must not collapse live daemon facts into stopped"
        );
    }

    #[test]
    fn json_payload_exposes_projection_missing_daemon_state() {
        let report = RuntimeStatusReport::from_parts(None, daemon_with_facts(false, false));

        let payload = report.to_json(json!({"state": "test"}));

        assert_eq!(
            payload["runtime_status"],
            "projection_missing_process_running"
        );
        assert!(payload["runtime"].is_null());
        assert_eq!(payload["daemon"]["pid_alive"], true);
        assert_eq!(payload["daemon"]["identity"]["mode"], "device");
    }

    #[test]
    fn json_payload_exposes_companion_status_contract_shape() {
        let report = RuntimeStatusReport::from_parts_with_observations(
            Some(projection()),
            daemon_with_facts(true, true),
            None,
            vec![companion_status_dto()],
        );

        let payload = report.to_json(json!({"state": "test"}));
        let companion = &payload["desktop_companions"][0];

        assert_eq!(report.desktop_companions().len(), 1);
        assert_eq!(companion["profile"], "desktop_companion");
        assert_eq!(companion["kind"], "desktop_companion_status");
        assert_eq!(companion["package_id"], "easynet.desktop.menubar");
        assert_eq!(companion["projected_state"], "running");
        assert_eq!(companion["metadata"]["source"], "runtime_status_test");
    }

    #[test]
    fn status_classifier_keeps_projection_only_as_degraded_not_running() {
        let daemon =
            DaemonDiscoverySnapshot::from_parts(None, None, false, false, false, endpoints());
        let report = RuntimeStatusReport::from_parts(Some(projection()), daemon);

        assert_eq!(
            report.status(),
            RuntimeLifecycleStatus::ProjectionPresentProcessMissing
        );
    }

    #[test]
    fn status_classifier_marks_control_only_as_broken_daemon() {
        let report =
            RuntimeStatusReport::from_parts(Some(projection()), daemon_with_facts(true, false));

        assert_eq!(
            report.status(),
            RuntimeLifecycleStatus::ControlOnlyInvocationDown
        );
    }

    #[test]
    fn status_classifier_preserves_legacy_axon_bridge_projection() {
        let projection = RuntimeSessionProjection::from_state(config::RuntimeState {
            endpoint: "127.0.0.1:50091".to_string(),
            runtime_kind: config::RuntimeKind::AxonBridge,
            pid: Some(12_345),
            hub: None,
            tenant: None,
            label: None,
            started_at: None,
            credential_verified: None,
        });
        let daemon =
            DaemonDiscoverySnapshot::from_parts(None, None, false, false, false, endpoints());
        let report = RuntimeStatusReport::from_parts(Some(projection), daemon);

        assert_eq!(report.status(), RuntimeLifecycleStatus::LegacyAxonBridge);
    }
}
