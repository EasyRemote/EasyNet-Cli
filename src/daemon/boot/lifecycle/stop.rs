//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/boot/lifecycle/stop.rs
//! Description: stop planning for daemon lifecycle.
//!
//! Protocol Responsibility:
//! - Selects daemon, projection, and legacy cleanup shapes without performing
//!   OS side effects.
//!
//! Implementation Approach:
//! - Derives a side-effect-free plan from `RuntimeStatusReport`; CLI stop owns
//!   stage rendering and process signaling.
//!
//! Usage Contract:
//! - Missing projection with daemon facts must still plan daemon shutdown.
//!
//! Architectural Position:
//! - `daemon::boot::lifecycle` stop state machine.
//!
//! The plan is side-effect free. CLI presentation decides how to render
//! stages, but the lifecycle module decides what kind of runtime must
//! be stopped from the authoritative status report.

use super::RuntimeStatusReport;

/// Runtime shape selected for stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStopShape {
    /// No projection and no daemon facts.
    Stateless,
    /// Modern product daemon shape.
    DaemonOnly,
}

/// Side-effect-free stop plan.
///
/// Invariants:
/// 1. Missing `runtime.json` plus daemon facts plans `DaemonOnly`, not
///    `Stateless`.
/// 2. `cleanup_runtime_projection` is true only when a projection
///    existed at plan time.
/// 3. `discovery_pid` is set only when the daemon snapshot selected a
///    live PID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStopPlan {
    shape: RuntimeStopShape,
    discovery_pid: Option<u32>,
    cleanup_runtime_projection: bool,
}

impl RuntimeStopPlan {
    /// Build a stop plan from a lifecycle status report.
    pub fn from_report(report: &RuntimeStatusReport) -> Self {
        let state = report.projection().map(|projection| projection.state());
        let shape = match state {
            None if report.daemon().has_daemon_fact() => RuntimeStopShape::DaemonOnly,
            None => RuntimeStopShape::Stateless,
            Some(_) => RuntimeStopShape::DaemonOnly,
        };
        Self {
            shape,
            discovery_pid: report
                .daemon()
                .pid()
                .filter(|_| report.daemon().pid_alive() && report.daemon().pid_matches_easynet()),
            cleanup_runtime_projection: state.is_some(),
        }
    }

    /// Runtime shape selected for stop.
    pub fn shape(&self) -> &RuntimeStopShape {
        &self.shape
    }

    /// Live daemon PID discovered outside `runtime.json`, when known.
    pub fn discovery_pid(&self) -> Option<u32> {
        self.discovery_pid
    }

    /// Whether `runtime.json` existed at plan time.
    pub fn should_cleanup_runtime_projection(&self) -> bool {
        self.cleanup_runtime_projection
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::daemon::boot::DaemonEndpoints;
    use crate::daemon::lifecycle::{
        DaemonDiscoverySnapshot, RuntimeSessionProjection, RuntimeStatusReport,
    };
    use crate::daemon::persistence::config;

    use super::*;

    fn endpoints() -> DaemonEndpoints {
        DaemonEndpoints {
            control: PathBuf::from("/tmp/easynet-stop-control.sock"),
            invocation: PathBuf::from("/tmp/easynet-stop-daemon.sock"),
        }
    }

    #[test]
    fn stop_plan_treats_projection_missing_live_daemon_as_daemon_only() {
        let pid = std::process::id();
        let daemon =
            DaemonDiscoverySnapshot::from_parts(None, Some(pid), true, false, false, endpoints());
        let report = RuntimeStatusReport::from_parts(None, daemon);

        let plan = RuntimeStopPlan::from_report(&report);

        assert!(
            matches!(plan.shape(), RuntimeStopShape::DaemonOnly),
            "Invariant 1: projection-missing daemon facts must still stop the daemon"
        );
        assert_eq!(plan.discovery_pid(), Some(pid));
        assert!(!plan.should_cleanup_runtime_projection());
    }

    #[test]
    fn stop_plan_maps_runtime_projection_to_daemon_only() {
        let projection = RuntimeSessionProjection::from_state(config::RuntimeState {
            endpoint: "/tmp/easynet-stop-daemon.sock".to_string(),
            runtime_kind: config::RuntimeKind::DaemonOnly,
            pid: Some(12_345),
            hub: None,
            tenant: Some("tenant-test".to_string()),
            label: Some("node-test".to_string()),
            started_at: None,
            credential_verified: None,
        });
        let daemon =
            DaemonDiscoverySnapshot::from_parts(None, None, false, false, false, endpoints());
        let report = RuntimeStatusReport::from_parts(Some(projection), daemon);

        let plan = RuntimeStopPlan::from_report(&report);

        assert!(matches!(plan.shape(), RuntimeStopShape::DaemonOnly));
        assert!(plan.should_cleanup_runtime_projection());
    }
}
