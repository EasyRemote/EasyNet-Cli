// EasyNet CLI — desktop companion manager
// =======================================
//
// File: src/daemon/plugins/companion/mod.rs
// Description: Daemon/plugin lifecycle model for user-session UI companions.

pub mod linux;
pub mod macos;
pub mod planner;
pub mod state_store;
pub mod status;
pub mod windows;

use serde_json::json;

use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::manifest::{
    PluginCompanionBootPolicy, PluginCompanionStopPolicy, PluginKind,
};
use crate::daemon::plugins::package::SharedPluginPackage;

pub use planner::{
    current_platform, DesktopCompanionPlan, DesktopCompanionPlanner, PlatformCompanionSpec,
};
pub use state_store::DesktopCompanionStateStore;
pub use status::{
    boot_policy_wire, health_wire, project_state, stop_policy_wire, CompanionDesiredState,
    CompanionObservation, CompanionObservedState, CompanionProjectedState, CompanionSessionStatus,
    CompanionSupervisorState, DesktopCompanionStatus,
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
        let desired = self
            .state_store
            .desired_state(&plan.package_id, &plan.package_version)?;
        let session = self.supervisor.probe_session();
        let supervisor = if !session.is_available() {
            CompanionSupervisorState::UnsupportedSession
        } else {
            self.supervisor.supervisor_state(plan)
        };
        let observation = self.supervisor.observe(plan);
        Ok(status_from_parts(plan, desired, supervisor, observation))
    }

    pub fn ensure_running_after_daemon_ready(
        &self,
        packages: &[SharedPluginPackage],
    ) -> Vec<String> {
        let mut warnings = Vec::new();
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
            if let Err(err) = self.supervisor.start(&plan) {
                warnings.push(format!(
                    "{}@{} start_failed: {err}",
                    plan.package_id, plan.package_version
                ));
            }
        }
        warnings
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

    pub fn status_json(&self, package: &SharedPluginPackage) -> Result<serde_json::Value> {
        let status = self.status_for_package(package)?;
        serde_json::to_value(status)
            .ok()
            .and_then(|value| crate::protocol::companion_contract::project_status(&value).ok())
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
        let _ = self.supervisor.stop(&plan);
        let _ = self.supervisor.remove(&plan);
        self.state_store
            .remove(&plan.package_id, &plan.package_version)
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
) -> DesktopCompanionStatus {
    let projected = project_state(desired, supervisor, observation.observed_state);
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
        error: observation
            .error
            .map(|message| json!({ "message": message })),
    }
}

fn action_result(
    package_id: &str,
    action: &str,
    before: Option<DesktopCompanionStatus>,
    after: Option<DesktopCompanionStatus>,
    changed: bool,
    error: Option<String>,
) -> Result<serde_json::Value> {
    crate::protocol::companion_contract::project_action_result(&json!({
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

pub fn companion_status_file(package_id: &str) -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".easynet/companions")
        .join(package_id)
        .join("status.json")
}
