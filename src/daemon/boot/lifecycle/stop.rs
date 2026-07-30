//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/boot/lifecycle/stop.rs
//! Description: stop planning for daemon lifecycle.
//!
//! Protocol Responsibility:
//! - Select daemon, projection, and process cleanup shapes from one lifecycle
//!   authority.
//! - Own pidfile/discovery process-stop transitions for runtime stop.
//!
//! Implementation Approach:
//! - Derives a side-effect-free plan from `RuntimeStatusReport`.
//! - Exposes a focused process controller whose outcomes are separate from CLI
//!   rendering.
//!
//! Usage Contract:
//! - Missing projection with daemon facts must still plan daemon shutdown.
//! - CLI callers may render outcomes, but must not own process liveness,
//!   pid-reuse, or pgrep sweep decisions.
//!
//! Architectural Position:
//! - `daemon::boot::lifecycle` stop state machine.
//!
//! The plan is side-effect free. CLI presentation decides how to render
//! stages, but the lifecycle module decides what kind of runtime must
//! be stopped from the authoritative status report.

use std::path::Path;
use std::time::Duration;

use crate::support::platform::net;

use super::RuntimeStatusReport;

const DEFAULT_STOP_TIMEOUT: Duration = Duration::from_secs(3);

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

/// Runtime process-stop authority used by `easynet runtime stop`.
///
/// This controller owns the OS-facing lifecycle transitions that are specific
/// to the current EasyNet runtime: pidfile stop and discovery-pid stop. CLI
/// code consumes the typed outcomes only for presentation.
#[derive(Debug, Clone, Copy, Default)]
pub struct RuntimeStopProcessController;

impl RuntimeStopProcessController {
    /// Construct the production process controller.
    pub fn new() -> Self {
        Self
    }

    /// Pidfile -> liveness check -> easynet-process check -> SIGTERM with a
    /// bounded wait. Removes the pidfile after the attempt regardless of
    /// outcome so a stale file from a crashed daemon does not block the next
    /// `easynet runtime start`.
    pub fn stop_pidfile_process(&self, pid_path: &Path) -> PidfileStopOutcome {
        let pid: u32 = match read_pidfile(pid_path) {
            Some(pid) => pid,
            None => return PidfileStopOutcome::NoPidfile,
        };
        if !net::is_pid_alive(pid) {
            let _ = std::fs::remove_file(pid_path);
            return PidfileStopOutcome::StalePidfile { pid };
        }
        if !net::is_easynet_process(pid) {
            let _ = std::fs::remove_file(pid_path);
            return PidfileStopOutcome::PidReuseRefused { pid };
        }
        let stopped = net::kill_and_wait(pid, DEFAULT_STOP_TIMEOUT);
        let _ = std::fs::remove_file(pid_path);
        if stopped {
            PidfileStopOutcome::Stopped { pid }
        } else {
            PidfileStopOutcome::TimedOut { pid }
        }
    }

    /// Stop a daemon PID discovered from lifecycle facts rather than from the
    /// pidfile.
    pub fn stop_discovered_daemon_process(&self, pid: u32) -> LiveProcessStopOutcome {
        if !net::is_pid_alive(pid) {
            return LiveProcessStopOutcome::StalePid { pid };
        }
        if !net::is_easynet_process(pid) {
            return LiveProcessStopOutcome::PidReuseRefused { pid };
        }
        if net::kill_and_wait(pid, DEFAULT_STOP_TIMEOUT) {
            LiveProcessStopOutcome::Stopped { pid }
        } else {
            LiveProcessStopOutcome::TimedOut { pid }
        }
    }
}

/// Result of attempting to stop a process named by a pidfile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PidfileStopOutcome {
    NoPidfile,
    StalePidfile { pid: u32 },
    PidReuseRefused { pid: u32 },
    Stopped { pid: u32 },
    TimedOut { pid: u32 },
}

/// Result of stopping a live process discovered outside a pidfile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveProcessStopOutcome {
    StalePid { pid: u32 },
    PidReuseRefused { pid: u32 },
    Stopped { pid: u32 },
    TimedOut { pid: u32 },
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

fn read_pidfile(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| contents.trim().parse::<u32>().ok())
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
    fn stop_plan_treats_invalid_discovery_as_cleanup_state() {
        let daemon =
            DaemonDiscoverySnapshot::from_invalid_discovery("control.json malformed", endpoints());
        let report = RuntimeStatusReport::from_parts(None, daemon);

        let plan = RuntimeStopPlan::from_report(&report);

        assert!(
            matches!(plan.shape(), RuntimeStopShape::DaemonOnly),
            "invalid discovery must not be hidden as a stateless runtime"
        );
        assert_eq!(plan.discovery_pid(), None);
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

    #[test]
    fn process_controller_reports_missing_pidfile_without_side_effects() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_path = dir.path().join("missing.pid");

        let outcome = RuntimeStopProcessController::new().stop_pidfile_process(&pid_path);

        assert_eq!(outcome, PidfileStopOutcome::NoPidfile);
        assert!(
            !pid_path.exists(),
            "missing pidfile stop must not create lifecycle state"
        );
    }

    #[test]
    fn process_controller_reports_malformed_pidfile_as_no_pidfile_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_path = dir.path().join("easynet-daemon.pid");
        std::fs::write(&pid_path, "not-a-pid").expect("write pidfile");

        let outcome = RuntimeStopProcessController::new().stop_pidfile_process(&pid_path);

        assert_eq!(outcome, PidfileStopOutcome::NoPidfile);
        assert!(
            pid_path.exists(),
            "malformed pidfile is not claimed as a daemon process transition"
        );
    }
}
