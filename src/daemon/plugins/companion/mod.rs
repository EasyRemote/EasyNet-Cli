// EasyNet CLI — desktop companion manager
// =======================================
//
// File: src/daemon/plugins/companion/mod.rs
// Description: Daemon/plugin lifecycle model for user-session UI companions.

mod artifact;
mod heartbeat;
pub mod linux;
pub mod macos;
pub mod planner;
mod projection;
mod session;
pub mod state_store;
pub mod status;
mod status_file;
pub mod windows;

use serde_json::json;

use self::artifact::artifact_fingerprint;
use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::manifest::{
    PluginCompanionBootPolicy, PluginCompanionStopPolicy, PluginKind,
};
use crate::daemon::plugins::package::SharedPluginPackage;

pub use planner::{
    current_platform, DesktopCompanionPlan, DesktopCompanionPlanner, PlatformCompanionSpec,
};
pub(crate) use projection::{project_action_result, project_status};
pub use session::DesktopCompanionSessionProbe;
pub use state_store::DesktopCompanionStateStore;
pub use status::{
    boot_policy_wire, health_wire, project_state, project_state_with_action_error,
    stop_policy_wire, CompanionDesiredState, CompanionObservation, CompanionObservedState,
    CompanionProjectedState, CompanionSessionStatus, CompanionSupervisorState,
    DesktopCompanionStatus,
};

/// Report returned by platform supervisor actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompanionActionReport {
    pub changed: bool,
    pub message: String,
}

impl CompanionActionReport {
    pub fn unchanged(message: impl Into<String>) -> Self {
        Self {
            changed: false,
            message: message.into(),
        }
    }

    pub fn changed(message: impl Into<String>) -> Self {
        Self {
            changed: true,
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopCompanionReconcileFailure {
    pub package_id: String,
    pub package_version: String,
    pub action: &'static str,
    pub code: &'static str,
    pub reason: String,
}

impl DesktopCompanionReconcileFailure {
    fn start_failed(plan: &DesktopCompanionPlan, reason: impl Into<String>) -> Self {
        Self {
            package_id: plan.package_id.clone(),
            package_version: plan.package_version.clone(),
            action: "start",
            code: "start_failed",
            reason: reason.into(),
        }
    }

    fn state_store_failed(plan: &DesktopCompanionPlan, reason: impl Into<String>) -> Self {
        Self {
            package_id: plan.package_id.clone(),
            package_version: plan.package_version.clone(),
            action: "record",
            code: "state_store_failed",
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for DesktopCompanionReconcileFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}@{} {}: {}",
            self.package_id, self.package_version, self.code, self.reason
        )
    }
}

/// OS user-session launcher boundary.
pub trait DesktopCompanionSupervisor {
    fn platform(&self) -> &'static str;
    fn probe_session(&self) -> CompanionSessionStatus;
    fn install(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport>;
    fn enable(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport>;
    fn disable(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport>;
    fn remove(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport>;
    fn start(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport>;
    fn stop(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport>;
    fn supervisor_state(&self, plan: &DesktopCompanionPlan) -> CompanionSupervisorState;
    fn observe(&self, plan: &DesktopCompanionPlan) -> CompanionObservation;
}

/// Desktop companion lifecycle facade shared by CLI and daemon-local control.
pub struct DesktopCompanionManager {
    planner: DesktopCompanionPlanner,
    supervisor: Box<dyn DesktopCompanionSupervisor + Send + Sync>,
    state_store: DesktopCompanionStateStore,
}

impl DesktopCompanionManager {
    pub fn current() -> Self {
        Self::new(
            DesktopCompanionPlanner::current(),
            current_supervisor(),
            DesktopCompanionStateStore::new(DesktopCompanionStateStore::default_path()),
        )
    }

    pub fn new(
        planner: DesktopCompanionPlanner,
        supervisor: Box<dyn DesktopCompanionSupervisor + Send + Sync>,
        state_store: DesktopCompanionStateStore,
    ) -> Self {
        Self {
            planner,
            supervisor,
            state_store,
        }
    }

    pub fn plan_package(&self, package: &SharedPluginPackage) -> Result<DesktopCompanionPlan> {
        self.planner.plan_package(package).map_err(|reason| {
            PluginHostError::InvalidCompanionManifest {
                id: package.id().as_str().to_string(),
                reason,
            }
        })
    }

    pub fn status_for_package(
        &self,
        package: &SharedPluginPackage,
    ) -> Result<DesktopCompanionStatus> {
        if package.manifest().kind() != PluginKind::DesktopCompanion {
            return Err(PluginHostError::InvalidCompanionManifest {
                id: package.id().as_str().to_string(),
                reason: "package is not a desktop_companion".to_string(),
            });
        }
        let plan = self.plan_package(package)?;
        self.status_for_plan(&plan)
    }

    pub fn status_for_plan(&self, plan: &DesktopCompanionPlan) -> Result<DesktopCompanionStatus> {
        let state_record = self
            .state_store
            .record(&plan.package_id, &plan.package_version)?;
        let desired = state_record
            .as_ref()
            .map(|record| record.desired_state)
            .unwrap_or_default();
        let session = self.supervisor.probe_session();
        let supervisor = if !session.is_available() {
            CompanionSupervisorState::UnsupportedSession
        } else {
            self.supervisor.supervisor_state(plan)
        };
        let observation = self.supervisor.observe(plan);
        Ok(status_from_parts(
            plan,
            desired,
            supervisor,
            observation,
            state_record.as_ref(),
        ))
    }

    pub fn ensure_running_after_daemon_ready(
        &self,
        packages: &[SharedPluginPackage],
    ) -> Vec<DesktopCompanionReconcileFailure> {
        let mut failures = Vec::new();
        for package in packages {
            if package.manifest().kind() != PluginKind::DesktopCompanion {
                continue;
            }
            let Ok(plan) = self.plan_package(package) else {
                continue;
            };
            if plan.boot_policy != PluginCompanionBootPolicy::EnsureRunningAfterDaemonReady {
                continue;
            }
            let Ok(desired) = self
                .state_store
                .desired_state(&plan.package_id, &plan.package_version)
            else {
                continue;
            };
            if desired != CompanionDesiredState::Enabled {
                continue;
            }
            match self.supervisor.start(&plan) {
                Ok(_) => {
                    if let Err(err) = self.state_store.set_desired_state(
                        &plan.package_id,
                        &plan.package_version,
                        CompanionDesiredState::Enabled,
                        "start",
                        None,
                    ) {
                        failures.push(DesktopCompanionReconcileFailure::state_store_failed(
                            &plan,
                            err.to_string(),
                        ));
                    }
                }
                Err(err) => {
                    let reason = err.to_string();
                    if let Err(record_err) = self.state_store.set_desired_state(
                        &plan.package_id,
                        &plan.package_version,
                        CompanionDesiredState::Enabled,
                        "start",
                        Some(reason.clone()),
                    ) {
                        failures.push(DesktopCompanionReconcileFailure::state_store_failed(
                            &plan,
                            record_err.to_string(),
                        ));
                    }
                    failures.push(DesktopCompanionReconcileFailure::start_failed(
                        &plan, reason,
                    ));
                }
            }
        }
        failures
    }

    pub fn enable(&self, package: &SharedPluginPackage) -> Result<serde_json::Value> {
        let plan = self.plan_package(package)?;
        let before = self.status_for_plan(&plan).ok();
        self.supervisor.install(&plan)?;
        self.supervisor.enable(&plan)?;
        self.state_store.set_desired_state(
            &plan.package_id,
            &plan.package_version,
            CompanionDesiredState::Enabled,
            "enable",
            None,
        )?;
        if plan.boot_policy == PluginCompanionBootPolicy::EnsureRunningAfterDaemonReady {
            self.supervisor.start(&plan)?;
        }
        let after = self.status_for_plan(&plan).ok();
        action_result(&plan.package_id, "enable", before, after, true, None)
    }

    pub fn commit_package_install(
        &self,
        package: &SharedPluginPackage,
    ) -> Result<serde_json::Value> {
        let plan = self.plan_package(package)?;
        let before = self.status_for_plan(&plan).ok();
        self.supervisor.install(&plan)?;
        self.supervisor.enable(&plan)?;
        self.state_store.set_desired_state(
            &plan.package_id,
            &plan.package_version,
            CompanionDesiredState::Enabled,
            "install",
            None,
        )?;
        let after = self.status_for_plan(&plan).ok();
        action_result(&plan.package_id, "install", before, after, true, None)
    }

    pub fn commit_package_update(
        &self,
        package: &SharedPluginPackage,
        previous_status: Option<&DesktopCompanionStatus>,
        executable_artifact_changed: bool,
    ) -> Result<serde_json::Value> {
        let plan = self.plan_package(package)?;
        let desired = previous_status
            .and_then(|status| CompanionDesiredState::from_wire(&status.desired_state))
            .unwrap_or(CompanionDesiredState::Enabled);
        let was_running = previous_status.is_some_and(companion_status_is_running);
        let should_restart =
            desired == CompanionDesiredState::Enabled && was_running && executable_artifact_changed;
        let before = previous_status
            .cloned()
            .or_else(|| self.status_for_plan(&plan).ok());

        self.supervisor.install(&plan)?;
        match desired {
            CompanionDesiredState::Enabled => {
                self.supervisor.enable(&plan)?;
            }
            CompanionDesiredState::Disabled => {
                self.supervisor.disable(&plan)?;
            }
        }
        self.state_store.set_desired_state(
            &plan.package_id,
            &plan.package_version,
            desired,
            "update",
            None,
        )?;
        if let Some(previous) = previous_status {
            if previous.package_version != plan.package_version {
                self.state_store
                    .remove(&previous.package_id, &previous.package_version)?;
            }
        }
        if should_restart {
            self.supervisor.stop(&plan)?;
            self.supervisor.start(&plan)?;
        }
        let after = self.status_for_plan(&plan).ok();
        let action = if should_restart { "restart" } else { "install" };
        action_result(&plan.package_id, action, before, after, true, None)
    }

    pub fn executable_artifact_changed(
        &self,
        previous: &SharedPluginPackage,
        replacement: &SharedPluginPackage,
    ) -> Result<bool> {
        let previous_plan = self.plan_package(previous)?;
        let replacement_plan = self.plan_package(replacement)?;
        let previous_fingerprint =
            artifact_fingerprint(previous_plan.spec.executable_artifact_path())?;
        let replacement_fingerprint =
            artifact_fingerprint(replacement_plan.spec.executable_artifact_path())?;
        Ok(previous_fingerprint != replacement_fingerprint)
    }

    pub fn restore_package_after_failed_update(
        &self,
        package: &SharedPluginPackage,
        previous_status: &DesktopCompanionStatus,
    ) -> Result<()> {
        let plan = self.plan_package(package)?;
        let desired =
            CompanionDesiredState::from_wire(&previous_status.desired_state).ok_or_else(|| {
                PluginHostError::InvalidCompanionManifest {
                    id: previous_status.package_id.clone(),
                    reason: format!(
                        "invalid previous desired_state {:?}",
                        previous_status.desired_state
                    ),
                }
            })?;
        self.supervisor.install(&plan)?;
        match desired {
            CompanionDesiredState::Enabled => {
                self.supervisor.enable(&plan)?;
            }
            CompanionDesiredState::Disabled => {
                self.supervisor.disable(&plan)?;
            }
        }
        self.state_store.set_desired_state(
            &plan.package_id,
            &plan.package_version,
            desired,
            "update_rollback",
            None,
        )?;
        if desired == CompanionDesiredState::Enabled && companion_status_is_running(previous_status)
        {
            self.supervisor.start(&plan)?;
        }
        Ok(())
    }

    pub fn status_json(&self, package: &SharedPluginPackage) -> Result<serde_json::Value> {
        let status = self.status_for_package(package)?;
        serde_json::to_value(status)
            .ok()
            .and_then(|value| project_status(&value).ok())
            .ok_or_else(|| PluginHostError::InvalidCompanionManifest {
                id: package.id().as_str().to_string(),
                reason: "companion status projection failed".to_string(),
            })
    }

    pub fn start(&self, package: &SharedPluginPackage) -> Result<serde_json::Value> {
        let plan = self.plan_package(package)?;
        let before = self.status_for_plan(&plan).ok();
        self.supervisor.start(&plan)?;
        let after = self.status_for_plan(&plan).ok();
        action_result(&plan.package_id, "start", before, after, true, None)
    }

    pub fn stop(&self, package: &SharedPluginPackage) -> Result<serde_json::Value> {
        let plan = self.plan_package(package)?;
        let before = self.status_for_plan(&plan).ok();
        self.supervisor.stop(&plan)?;
        let after = self.status_for_plan(&plan).ok();
        action_result(&plan.package_id, "stop", before, after, true, None)
    }

    pub fn reconcile(&self, package: &SharedPluginPackage) -> Result<serde_json::Value> {
        let plan = self.plan_package(package)?;
        let before = self.status_for_plan(&plan).ok();
        let desired = self
            .state_store
            .desired_state(&plan.package_id, &plan.package_version)?;
        let mut changed = false;
        if desired == CompanionDesiredState::Enabled
            && plan.boot_policy == PluginCompanionBootPolicy::EnsureRunningAfterDaemonReady
        {
            self.supervisor.start(&plan)?;
            changed = true;
        }
        let after = self.status_for_plan(&plan).ok();
        action_result(&plan.package_id, "reconcile", before, after, changed, None)
    }

    pub fn disable(&self, package: &SharedPluginPackage) -> Result<serde_json::Value> {
        let plan = self.plan_package(package)?;
        let before = self.status_for_plan(&plan).ok();
        self.supervisor.stop(&plan)?;
        self.supervisor.disable(&plan)?;
        self.state_store.set_desired_state(
            &plan.package_id,
            &plan.package_version,
            CompanionDesiredState::Disabled,
            "disable",
            None,
        )?;
        let after = self.status_for_plan(&plan).ok();
        action_result(&plan.package_id, "disable", before, after, true, None)
    }

    pub fn remove(&self, package: &SharedPluginPackage) -> Result<()> {
        let plan = self.plan_package(package)?;
        self.supervisor.stop(&plan)?;
        self.supervisor.remove(&plan)?;
        self.state_store
            .remove(&plan.package_id, &plan.package_version)
    }

    pub fn cleanup_for_self_uninstall(&self, packages: &[SharedPluginPackage]) -> Vec<String> {
        let records = match self.state_store.read() {
            Ok(state) => state.companion,
            Err(err) => {
                return vec![format!("state_read_failed: {err}")];
            }
        };
        let mut warnings = Vec::new();
        for record in records {
            let package = packages.iter().find(|package| {
                package.manifest().kind() == PluginKind::DesktopCompanion
                    && package.id().as_str() == record.id
                    && package.version().as_str() == record.version
            });
            if let Some(package) = package {
                if let Err(err) = self.remove(package) {
                    warnings.push(format!(
                        "{}@{} remove_failed: {err}",
                        record.id, record.version
                    ));
                }
                continue;
            }
            if let Err(err) = self.remove_orphan_state_and_status(&record) {
                warnings.push(format!(
                    "{}@{} orphan_cleanup_failed: {err}",
                    record.id, record.version
                ));
            } else {
                warnings.push(format!(
                    "{}@{} package_missing: removed desired state and status files only",
                    record.id, record.version
                ));
            }
        }
        warnings
    }

    fn remove_orphan_state_and_status(
        &self,
        record: &state_store::CompanionStateRecord,
    ) -> Result<()> {
        let status_dir = self.companion_status_dir(&record.id);
        if status_dir.exists() {
            std::fs::remove_dir_all(&status_dir).map_err(|source| {
                PluginHostError::WriteFailed {
                    path: status_dir.clone(),
                    source,
                }
            })?;
        }
        self.state_store.remove(&record.id, &record.version)
    }

    fn companion_status_dir(&self, package_id: &str) -> std::path::PathBuf {
        self.state_store
            .path()
            .parent()
            .map(|parent| parent.join(package_id))
            .unwrap_or_else(|| std::path::PathBuf::from(package_id))
    }

    pub fn stop_for_runtime_stop(&self, packages: &[SharedPluginPackage]) -> Vec<String> {
        let mut warnings = Vec::new();
        for package in packages {
            if package.manifest().kind() != PluginKind::DesktopCompanion {
                continue;
            }
            let Ok(plan) = self.plan_package(package) else {
                continue;
            };
            if plan.stop_policy != PluginCompanionStopPolicy::StopOnRuntimeStop {
                continue;
            }
            if let Err(err) = self.supervisor.stop(&plan) {
                warnings.push(format!(
                    "{}@{} stop_failed: {err}",
                    plan.package_id, plan.package_version
                ));
            }
        }
        warnings
    }
}

fn status_from_parts(
    plan: &DesktopCompanionPlan,
    desired: CompanionDesiredState,
    supervisor: CompanionSupervisorState,
    observation: CompanionObservation,
    state_record: Option<&state_store::CompanionStateRecord>,
) -> DesktopCompanionStatus {
    let state_error = state_record.and_then(state_record_error);
    let observed_error = observation.error.and_then(non_empty);
    let error = status_error(observation.observed_state, observed_error, state_error);
    let projected = project_state_with_action_error(
        desired,
        supervisor,
        observation.observed_state,
        error
            .as_ref()
            .and_then(|value| value.get("code"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(action_error_code_is_projected),
    );
    DesktopCompanionStatus {
        package_id: plan.package_id.clone(),
        package_version: plan.package_version.clone(),
        display_name: plan.display_name.clone(),
        platform: plan.platform.clone(),
        desired_state: desired.as_wire_str().to_string(),
        supervisor_state: supervisor.as_wire_str().to_string(),
        observed_state: observation.observed_state.as_wire_str().to_string(),
        projected_state: projected.as_wire_str().to_string(),
        boot_policy: boot_policy_wire(plan.boot_policy).to_string(),
        stop_policy: stop_policy_wire(plan.stop_policy).to_string(),
        health: health_wire(plan.health).to_string(),
        pid: observation.pid,
        version: observation.version,
        last_seen_unix_ms: observation.last_seen_unix_ms,
        launch_method: Some(plan.spec.launch_method().to_string()),
        error,
    }
}

fn state_record_error(record: &state_store::CompanionStateRecord) -> Option<(&str, String)> {
    let message = record
        .last_error
        .as_ref()
        .and_then(|value| non_empty(value.clone()))?;
    let action = record.last_action.as_deref().unwrap_or("action");
    Some((action_error_code(action), message))
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn status_error(
    observed: CompanionObservedState,
    observed_error: Option<String>,
    state_error: Option<(&str, String)>,
) -> Option<serde_json::Value> {
    if let Some(message) = observed_error {
        return Some(json!({
            "code": observed_error_code(observed, &message),
            "message": message,
        }));
    }
    let (code, message) = state_error?;
    Some(json!({
        "code": code,
        "message": message,
    }))
}

fn observed_error_code(observed: CompanionObservedState, message: &str) -> &'static str {
    match observed {
        CompanionObservedState::VersionMismatch => "version_mismatch",
        CompanionObservedState::HealthError if message == "status_file_invalid" => {
            "status_file_invalid"
        }
        CompanionObservedState::HealthError => "health_stale",
        _ => "status_file_invalid",
    }
}

fn action_error_code(action: &str) -> &'static str {
    match action {
        "install" => "supervisor_install_failed",
        "enable" => "supervisor_enable_failed",
        "disable" => "supervisor_disable_failed",
        "start" => "start_failed",
        "stop" => "stop_failed",
        _ => "action_failed",
    }
}

fn action_error_code_is_projected(code: &str) -> bool {
    matches!(
        code,
        "supervisor_install_failed"
            | "supervisor_enable_failed"
            | "supervisor_disable_failed"
            | "start_failed"
            | "stop_failed"
            | "action_failed"
    )
}

fn companion_status_is_running(status: &DesktopCompanionStatus) -> bool {
    matches!(
        status.observed_state.as_str(),
        "running" | "starting" | "stale"
    )
}

fn action_result(
    package_id: &str,
    action: &str,
    before: Option<DesktopCompanionStatus>,
    after: Option<DesktopCompanionStatus>,
    changed: bool,
    error: Option<String>,
) -> Result<serde_json::Value> {
    project_action_result(&json!({
        "package_id": package_id,
        "action": action,
        "status_before": before,
        "status_after": after,
        "changed": changed,
        "error": error.map(|message| json!({ "message": message })),
    }))
    .map_err(|source| PluginHostError::InvalidCompanionManifest {
        id: package_id.to_string(),
        reason: source.to_string(),
    })
}

fn current_supervisor() -> Box<dyn DesktopCompanionSupervisor + Send + Sync> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacosDesktopCompanionSupervisor::new())
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(windows::WindowsDesktopCompanionSupervisor::new())
    }
    #[cfg(target_os = "linux")]
    {
        Box::new(linux::LinuxDesktopCompanionSupervisor::new())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Box::new(linux::UnsupportedDesktopCompanionSupervisor::new("unknown"))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use crate::daemon::plugins::package::PluginPackage;

    use super::*;

    #[test]
    fn post_ready_start_failure_is_nonfatal_and_status_visible() {
        let root = tempfile::tempdir().expect("package root");
        write_companion_test_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let state_root = tempfile::tempdir().expect("state root");
        let manager = test_manager(state_root.path().join("state.toml"), true);
        manager
            .state_store
            .set_desired_state(
                "test.desktop.menubar",
                "0.1.0",
                CompanionDesiredState::Enabled,
                "enable",
                None,
            )
            .expect("desired state");

        let failures = manager.ensure_running_after_daemon_ready(&[package.clone()]);

        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].code, "start_failed");
        assert_eq!(failures[0].action, "start");
        assert!(
            failures[0].reason.contains("injected start failure"),
            "failure reason should preserve supervisor detail"
        );

        let status = manager.status_for_package(&package).expect("status");
        assert_eq!(status.projected_state, "error");
        let error = status.error.expect("status error");
        assert_eq!(error["code"], "start_failed");
        assert!(error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("injected start failure"));
    }

    #[test]
    fn post_ready_start_success_clears_previous_start_error() {
        let root = tempfile::tempdir().expect("package root");
        write_companion_test_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let state_root = tempfile::tempdir().expect("state root");
        let manager = test_manager(state_root.path().join("state.toml"), false);
        manager
            .state_store
            .set_desired_state(
                "test.desktop.menubar",
                "0.1.0",
                CompanionDesiredState::Enabled,
                "start",
                Some("old start failure".to_string()),
            )
            .expect("desired state");

        let failures = manager.ensure_running_after_daemon_ready(&[package.clone()]);

        assert!(failures.is_empty());
        let status = manager.status_for_package(&package).expect("status");
        assert_eq!(status.projected_state, "ready_stopped");
        assert!(status.error.is_none());
    }

    #[test]
    fn status_file_invalid_error_code_is_preserved() {
        let error = status_error(
            CompanionObservedState::HealthError,
            Some("status_file_invalid".to_string()),
            None,
        )
        .expect("error");

        assert_eq!(error["code"], "status_file_invalid");
        assert_eq!(error["message"], "status_file_invalid");
    }

    #[test]
    fn self_uninstall_cleanup_enumerates_desired_state_records() {
        let root = tempfile::tempdir().expect("package root");
        write_companion_test_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let state_root = tempfile::tempdir().expect("state root");
        let state_path = state_root.path().join("state.toml");
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let manager = DesktopCompanionManager::new(
            DesktopCompanionPlanner::new("macos"),
            Box::new(RecordingSupervisor {
                calls: Arc::clone(&calls),
            }),
            DesktopCompanionStateStore::new(&state_path),
        );
        manager
            .state_store
            .set_desired_state(
                "test.desktop.menubar",
                "0.1.0",
                CompanionDesiredState::Enabled,
                "enable",
                None,
            )
            .expect("desired state");

        let warnings = manager.cleanup_for_self_uninstall(&[package]);

        assert!(warnings.is_empty());
        assert_eq!(*calls.lock().expect("calls"), vec!["stop", "remove"]);
        assert!(manager
            .state_store
            .read()
            .expect("state")
            .companion
            .is_empty());
    }

    #[test]
    fn self_uninstall_cleanup_removes_orphan_state_and_status_directory() {
        let state_root = tempfile::tempdir().expect("state root");
        let state_path = state_root.path().join("state.toml");
        let status_dir = state_root.path().join("orphan.desktop.companion");
        std::fs::create_dir_all(&status_dir).expect("status dir");
        std::fs::write(status_dir.join("status.json"), "{}").expect("status file");
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let manager = DesktopCompanionManager::new(
            DesktopCompanionPlanner::new("macos"),
            Box::new(RecordingSupervisor {
                calls: Arc::clone(&calls),
            }),
            DesktopCompanionStateStore::new(&state_path),
        );
        manager
            .state_store
            .set_desired_state(
                "orphan.desktop.companion",
                "9.9.9",
                CompanionDesiredState::Enabled,
                "enable",
                None,
            )
            .expect("desired state");

        let warnings = manager.cleanup_for_self_uninstall(&[]);

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("package_missing"));
        assert!(calls.lock().expect("calls").is_empty());
        assert!(!status_dir.exists());
        assert!(manager
            .state_store
            .read()
            .expect("state")
            .companion
            .is_empty());
    }

    #[test]
    fn running_package_update_restarts_with_stop_then_start() {
        let root = tempfile::tempdir().expect("package root");
        write_companion_test_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let state_root = tempfile::tempdir().expect("state root");
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let manager = DesktopCompanionManager::new(
            DesktopCompanionPlanner::new("macos"),
            Box::new(RecordingSupervisor {
                calls: Arc::clone(&calls),
            }),
            DesktopCompanionStateStore::new(state_root.path().join("state.toml")),
        );
        let previous = previous_status("running");

        let result = manager
            .commit_package_update(&package, Some(&previous), true)
            .expect("update");

        assert_eq!(result["action"], "restart");
        assert_eq!(
            *calls.lock().expect("calls"),
            vec!["install", "enable", "stop", "start"]
        );
    }

    #[test]
    fn running_package_update_skips_restart_when_artifact_is_unchanged() {
        let root = tempfile::tempdir().expect("package root");
        write_companion_test_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let state_root = tempfile::tempdir().expect("state root");
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let manager = DesktopCompanionManager::new(
            DesktopCompanionPlanner::new("macos"),
            Box::new(RecordingSupervisor {
                calls: Arc::clone(&calls),
            }),
            DesktopCompanionStateStore::new(state_root.path().join("state.toml")),
        );
        let previous = previous_status("running");

        let result = manager
            .commit_package_update(&package, Some(&previous), false)
            .expect("update");

        assert_eq!(result["action"], "install");
        assert_eq!(*calls.lock().expect("calls"), vec!["install", "enable"]);
    }

    #[test]
    fn stopped_package_update_preserves_desired_without_restart() {
        let root = tempfile::tempdir().expect("package root");
        write_companion_test_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let state_root = tempfile::tempdir().expect("state root");
        let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let manager = DesktopCompanionManager::new(
            DesktopCompanionPlanner::new("macos"),
            Box::new(RecordingSupervisor {
                calls: Arc::clone(&calls),
            }),
            DesktopCompanionStateStore::new(state_root.path().join("state.toml")),
        );
        let previous = previous_status("not_running");

        let result = manager
            .commit_package_update(&package, Some(&previous), true)
            .expect("update");

        assert_eq!(result["action"], "install");
        assert_eq!(*calls.lock().expect("calls"), vec!["install", "enable"]);
    }

    #[test]
    fn executable_artifact_changed_detects_current_platform_bundle_change() {
        let previous_root = tempfile::tempdir().expect("previous root");
        write_companion_test_package_with_executable(previous_root.path(), "same");
        let previous =
            Arc::new(PluginPackage::from_installed(previous_root.path(), None).expect("previous"));
        let replacement_root = tempfile::tempdir().expect("replacement root");
        write_companion_test_package_with_executable(replacement_root.path(), "same");
        let replacement = Arc::new(
            PluginPackage::from_installed(replacement_root.path(), None).expect("replacement"),
        );
        let changed_root = tempfile::tempdir().expect("changed root");
        write_companion_test_package_with_executable(changed_root.path(), "changed");
        let changed =
            Arc::new(PluginPackage::from_installed(changed_root.path(), None).expect("changed"));
        let state_root = tempfile::tempdir().expect("state root");
        let manager = test_manager(state_root.path().join("state.toml"), false);

        assert!(!manager
            .executable_artifact_changed(&previous, &replacement)
            .expect("unchanged artifact"));
        assert!(manager
            .executable_artifact_changed(&previous, &changed)
            .expect("changed artifact"));
    }

    fn previous_status(observed_state: &str) -> DesktopCompanionStatus {
        DesktopCompanionStatus {
            package_id: "test.desktop.menubar".to_string(),
            package_version: "0.1.0".to_string(),
            display_name: "EasyNet Menu Bar".to_string(),
            platform: "macos".to_string(),
            desired_state: "enabled".to_string(),
            supervisor_state: "installed_enabled".to_string(),
            observed_state: observed_state.to_string(),
            projected_state: "running".to_string(),
            boot_policy: "ensure_running_after_daemon_ready".to_string(),
            stop_policy: "keep_running".to_string(),
            health: "status_file".to_string(),
            pid: None,
            version: None,
            last_seen_unix_ms: None,
            launch_method: Some("launch_agent".to_string()),
            error: None,
        }
    }

    fn test_manager(state_path: std::path::PathBuf, fail_start: bool) -> DesktopCompanionManager {
        DesktopCompanionManager::new(
            DesktopCompanionPlanner::new("macos"),
            Box::new(TestCompanionSupervisor { fail_start }),
            DesktopCompanionStateStore::new(state_path),
        )
    }

    struct TestCompanionSupervisor {
        fail_start: bool,
    }

    struct RecordingSupervisor {
        calls: Arc<std::sync::Mutex<Vec<&'static str>>>,
    }

    impl RecordingSupervisor {
        fn record(&self, action: &'static str) {
            self.calls.lock().expect("calls").push(action);
        }
    }

    impl DesktopCompanionSupervisor for RecordingSupervisor {
        fn platform(&self) -> &'static str {
            "macos"
        }

        fn probe_session(&self) -> CompanionSessionStatus {
            CompanionSessionStatus::Available
        }

        fn install(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            self.record("install");
            Ok(CompanionActionReport::changed("installed"))
        }

        fn enable(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            self.record("enable");
            Ok(CompanionActionReport::changed("enabled"))
        }

        fn disable(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            self.record("disable");
            Ok(CompanionActionReport::changed("disabled"))
        }

        fn remove(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            self.record("remove");
            Ok(CompanionActionReport::changed("removed"))
        }

        fn start(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            self.record("start");
            Ok(CompanionActionReport::changed("started"))
        }

        fn stop(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            self.record("stop");
            Ok(CompanionActionReport::changed("stopped"))
        }

        fn supervisor_state(&self, _plan: &DesktopCompanionPlan) -> CompanionSupervisorState {
            CompanionSupervisorState::InstalledEnabled
        }

        fn observe(&self, _plan: &DesktopCompanionPlan) -> CompanionObservation {
            CompanionObservation {
                observed_state: CompanionObservedState::NotRunning,
                ..CompanionObservation::default()
            }
        }
    }

    impl DesktopCompanionSupervisor for TestCompanionSupervisor {
        fn platform(&self) -> &'static str {
            "macos"
        }

        fn probe_session(&self) -> CompanionSessionStatus {
            CompanionSessionStatus::Available
        }

        fn install(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            Ok(CompanionActionReport::changed("installed"))
        }

        fn enable(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            Ok(CompanionActionReport::changed("enabled"))
        }

        fn disable(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            Ok(CompanionActionReport::changed("disabled"))
        }

        fn remove(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            Ok(CompanionActionReport::changed("removed"))
        }

        fn start(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            if self.fail_start {
                return Err(PluginHostError::InvalidCompanionManifest {
                    id: plan.package_id.clone(),
                    reason: "injected start failure".to_string(),
                });
            }
            Ok(CompanionActionReport::changed("started"))
        }

        fn stop(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
            Ok(CompanionActionReport::changed("stopped"))
        }

        fn supervisor_state(&self, _plan: &DesktopCompanionPlan) -> CompanionSupervisorState {
            CompanionSupervisorState::InstalledEnabled
        }

        fn observe(&self, _plan: &DesktopCompanionPlan) -> CompanionObservation {
            CompanionObservation {
                observed_state: CompanionObservedState::NotRunning,
                ..CompanionObservation::default()
            }
        }
    }

    fn write_companion_test_package(root: &Path) {
        write_companion_test_package_with_executable(root, "test");
    }

    fn write_companion_test_package_with_executable(root: &Path, executable_body: &str) {
        let executable = root.join("dist/macos/EasyNetMenuBar.app/Contents/MacOS/EasyNetMenuBar");
        std::fs::create_dir_all(executable.parent().expect("executable parent"))
            .expect("app bundle dir");
        std::fs::write(&executable, executable_body).expect("app executable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let mut permissions = std::fs::metadata(&executable)
                .expect("app executable metadata")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&executable, permissions).expect("chmod app executable");
        }
        std::fs::write(
            root.join("plugin.toml"),
            r#"
schema_version = "1"
id = "test.desktop.menubar"
version = "0.1.0"
kind = "desktop_companion"
entrypoint = "dist/macos/EasyNetMenuBar.app"
abilities = []
permissions = ["clipboard_read"]
resources = ["desktop_session"]
platforms = ["macos"]

[limits]
max_sessions = 1
max_frame_queue = 1

[companion]
display_name = "EasyNet Menu Bar"
lifecycle = "user_session"
boot_policy = "ensure_running_after_daemon_ready"
stop_policy = "keep_running"
health = "status_file"
status_file = "companions/test.desktop.menubar/status.json"

[companion.macos]
bundle_id = "tech.silan.easynet.menubar"
app_bundle = "dist/macos/EasyNetMenuBar.app"
supervisor = "launch_agent"
launch_agent_label = "tech.silan.easynet.menubar"
session = "aqua"
"#,
        )
        .expect("manifest");
    }
}
