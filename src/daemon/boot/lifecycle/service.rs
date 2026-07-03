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
    start, DaemonDiscoverySnapshot, ProductPresenceSnapshot, RuntimeLifecycleError,
    RuntimeSessionProjection, RuntimeStartPreflightReport, RuntimeStartRequest,
    RuntimeStatusReport, RuntimeStopPlan,
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
#[derive(Debug, Default, Clone, Copy)]
pub struct RuntimeLifecycleService;

impl RuntimeLifecycleService {
    /// Construct a lifecycle service using the current process
    /// environment and EasyNet state directory.
    pub fn new() -> Self {
        Self
    }

    /// Observe the current lifecycle state.
    pub fn status(&self) -> RuntimeStatusReport {
        RuntimeStatusReport::from_parts_with_presence(
            RuntimeSessionProjection::load_current(),
            DaemonDiscoverySnapshot::capture_current(),
            ProductPresenceSnapshot::capture_current(),
        )
    }

    /// Evaluate start preflight from process facts, removing stale
    /// projection only when no daemon fact remains.
    pub fn preflight_start(
        &self,
        request: &RuntimeStartRequest,
    ) -> Result<RuntimeStartPreflightReport, RuntimeLifecycleError> {
        start::preflight_start(request, &self.status())
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
        if let Err(err) = config::save(state) {
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
}
