//! EasyNet Axon for AgentNet
//! =========================
//!
//! File: src/daemon/boot/lifecycle/service.rs
//! Description: lifecycle service facade.
//!
//! Protocol Responsibility:
//! - Centralizes local daemon lifecycle sequencing while preserving Axon
//!   protocol ownership outside this module.
//!
//! Implementation Approach:
//! - Captures process facts first, session projection second, and product
//!   presence as an optional read-only observer.
//!
//! Usage Contract:
//! - CLI command modules call this object for lifecycle decisions and do not
//!   open-code projection/process ordering.
//!
//! Architectural Position:
//! - `daemon::boot::lifecycle` facade.
//!
//! `RuntimeLifecycleService` is the one object CLI commands use to
//! observe and mutate lifecycle state. It owns the sequencing rule:
//! process facts first, projection second, rollback when projection
//! persistence fails after a fresh daemon spawn.

use crate::daemon::persistence::config;
use crate::daemon::DaemonHandle;

use super::{
    start, DaemonDiscoveryObserver, ProductPresenceObserver, RuntimeLifecycleError,
    RuntimeProjectionStore, RuntimeStartPreflightAction, RuntimeStartPreflightReport,
    RuntimeStartRequest, RuntimeStatusReport, RuntimeStopPlan,
};

/// Entry point for runtime lifecycle operations.
///
/// Invariants:
/// 1. `status` never starts or stops processes; it only observes.
/// 2. `save_projection_after_ready` is called only after daemon Ready,
///    so a projection is never written for a daemon that has not bound
///    its product Invocation endpoint.
/// 3. If projection persistence fails after this service started a
///    daemon, it attempts to stop that daemon before returning.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeLifecycleService {
    discovery: DaemonDiscoveryObserver,
    projection_store: RuntimeProjectionStore,
    presence_observer: ProductPresenceObserver,
}

impl Default for RuntimeLifecycleService {
    fn default() -> Self {
        Self {
            discovery: DaemonDiscoveryObserver,
            projection_store: RuntimeProjectionStore,
            presence_observer: ProductPresenceObserver,
        }
    }
}

impl RuntimeLifecycleService {
    /// Construct a lifecycle service using the current process
    /// environment and EasyNet state directory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Observe the current lifecycle state.
    pub fn status(&self) -> RuntimeStatusReport {
        RuntimeStatusReport::from_parts_with_observations(
            self.projection_store.load(),
            self.discovery.capture(),
            self.presence_observer.capture(),
            super::status::desktop_companion_statuses(),
        )
    }

    /// Evaluate start preflight from process facts, removing stale
    /// projection only when no daemon fact remains.
    pub fn preflight_start(
        &self,
        request: &RuntimeStartRequest,
    ) -> Result<RuntimeStartPreflightReport, RuntimeLifecycleError> {
        let report = start::preflight_start(request, &self.status())?;
        if matches!(
            report.action(),
            RuntimeStartPreflightAction::RemovedStaleProjection
        ) {
            self.projection_store.remove().map_err(|source| {
                RuntimeLifecycleError::ProjectionRemoveFailed {
                    message: source.to_string(),
                }
            })?;
        }
        Ok(report)
    }

    /// Build the side-effect-free stop plan for the current host.
    pub fn stop_plan(&self) -> RuntimeStopPlan {
        RuntimeStopPlan::from_report(&self.status())
    }

    /// Persist `runtime.json` after daemon Ready, rolling back a newly
    /// spawned daemon if the projection cannot be saved.
    pub fn save_projection_after_ready(
        &self,
        handle: &mut DaemonHandle,
        state: &config::RuntimeState,
    ) -> Result<(), RuntimeLifecycleError> {
        if let Err(err) = self.projection_store.save(state) {
            let message = err.to_string();
            if handle.child_mut().is_some() {
                return match handle.stop() {
                    Ok(()) => Err(RuntimeLifecycleError::ProjectionPersistRolledBack { message }),
                    Err(rollback) => Err(RuntimeLifecycleError::ProjectionPersistRollbackFailed {
                        message,
                        rollback,
                    }),
                };
            }
            return Err(RuntimeLifecycleError::ProjectionPersistFailed { message });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;

    #[test]
    fn service_reports_without_requiring_runtime_projection() {
        let report = RuntimeLifecycleService::new().status();

        assert!(
            matches!(
                report.status(),
                super::super::status::RuntimeLifecycleStatus::Stopped
                    | super::super::status::RuntimeLifecycleStatus::Running
                    | super::super::status::RuntimeLifecycleStatus::ProjectionMissingProcessRunning
                    | super::super::status::RuntimeLifecycleStatus::ProjectionPresentProcessMissing
                    | super::super::status::RuntimeLifecycleStatus::ControlOnlyInvocationDown
                    | super::super::status::RuntimeLifecycleStatus::LegacyAxonBridge
            ),
            "Invariant 1: status observation must classify every host state"
        );
    }

    #[test]
    fn service_removes_stale_projection_during_start_preflight() {
        let _home = HomeGuard::new();
        let state = config::RuntimeState {
            endpoint: "/tmp/easynet-stale-daemon.sock".to_string(),
            runtime_kind: config::RuntimeKind::DaemonOnly,
            pid: Some(999_999),
            hub: None,
            tenant: Some("tenant-test".to_string()),
            label: Some("node-test".to_string()),
            started_at: None,
            credential_verified: None,
        };
        config::save(&state).expect("seed stale runtime projection");

        let report = RuntimeLifecycleService::new()
            .preflight_start(&RuntimeStartRequest::device("tenant-test", "node-test"))
            .expect("stale projection should be removable");

        assert!(
            matches!(
                report.action(),
                RuntimeStartPreflightAction::RemovedStaleProjection
            ),
            "service owns stale projection removal after pure start classification"
        );
        assert!(
            config::load().is_err(),
            "runtime.json must be removed only by the lifecycle service side-effect boundary"
        );
    }
}
