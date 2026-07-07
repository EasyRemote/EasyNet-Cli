// EasyNet CLI — Windows desktop companion supervisor
// ==================================================

use std::process::Command;

use crate::daemon::plugins::errors::{PluginHostError, Result};

use super::planner::{DesktopCompanionPlan, PlatformCompanionSpec};
use super::status::{
    CompanionObservation, CompanionObservedState, CompanionSessionStatus, CompanionSupervisorState,
};
use super::{companion_status_file, CompanionActionReport, DesktopCompanionSupervisor};

pub struct WindowsDesktopCompanionSupervisor;

impl WindowsDesktopCompanionSupervisor {
    pub const fn new() -> Self {
        Self
    }
}

impl DesktopCompanionSupervisor for WindowsDesktopCompanionSupervisor {
    fn platform(&self) -> &'static str {
        "windows"
    }

    fn probe_session(&self) -> CompanionSessionStatus {
        #[cfg(target_os = "windows")]
        {
            CompanionSessionStatus::Available
        }
        #[cfg(not(target_os = "windows"))]
        {
            CompanionSessionStatus::Unsupported {
                reason: "not running on Windows".to_string(),
            }
        }
    }

    fn install(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged(
            "Windows artifact install is handled by package install in this release",
        ))
    }

    fn enable(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        #[cfg(target_os = "windows")]
        {
            let PlatformCompanionSpec::Windows { exe, task_name, .. } = &plan.spec else {
                return Ok(CompanionActionReport::unchanged(
                    "not a Windows companion plan",
                ));
            };
            let status = Command::new("reg")
                .args([
                    "add",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    task_name,
                    "/t",
                    "REG_SZ",
                    "/d",
                    &exe.display().to_string(),
                    "/f",
                ])
                .status()
                .map_err(|source| PluginHostError::WriteFailed {
                    path: std::path::PathBuf::from("reg"),
                    source,
                })?;
            if !status.success() {
                return Err(PluginHostError::InvalidCompanionManifest {
                    id: plan.package_id.clone(),
                    reason: format!("registry enable failed with {status}"),
                });
            }
            Ok(CompanionActionReport::changed("registered HKCU Run entry"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = plan;
            Ok(CompanionActionReport::unchanged("unsupported platform"))
        }
    }

    fn disable(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        #[cfg(target_os = "windows")]
        {
            let PlatformCompanionSpec::Windows { task_name, .. } = &plan.spec else {
                return Ok(CompanionActionReport::unchanged(
                    "not a Windows companion plan",
                ));
            };
            let _ = Command::new("reg")
                .args([
                    "delete",
                    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                    "/v",
                    task_name,
                    "/f",
                ])
                .status();
            Ok(CompanionActionReport::changed("removed HKCU Run entry"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = plan;
            Ok(CompanionActionReport::unchanged("unsupported platform"))
        }
    }

    fn remove(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        let _ = self.disable(plan);
        let _ = std::fs::remove_file(companion_status_file(&plan.package_id));
        Ok(CompanionActionReport::changed(
            "removed Windows companion state",
        ))
    }

    fn start(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        let PlatformCompanionSpec::Windows { exe, .. } = &plan.spec else {
            return Ok(CompanionActionReport::unchanged(
                "not a Windows companion plan",
            ));
        };
        Command::new(exe)
            .spawn()
            .map_err(|source| PluginHostError::WriteFailed {
                path: exe.clone(),
                source,
            })?;
        Ok(CompanionActionReport::changed("started Windows companion"))
    }

    fn stop(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged(
            "Windows stop uses status-file pid in a later adapter upgrade",
        ))
    }

    fn supervisor_state(&self, _plan: &DesktopCompanionPlan) -> CompanionSupervisorState {
        if self.probe_session().is_available() {
            CompanionSupervisorState::InstalledDisabled
        } else {
            CompanionSupervisorState::UnsupportedSession
        }
    }

    fn observe(&self, plan: &DesktopCompanionPlan) -> CompanionObservation {
        let path = companion_status_file(&plan.package_id);
        if let Ok(body) = std::fs::read_to_string(path) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&body) {
                return CompanionObservation {
                    observed_state: CompanionObservedState::Running,
                    pid: value["pid"].as_u64(),
                    version: value["package_version"].as_str().map(ToOwned::to_owned),
                    last_seen_unix_ms: value["last_seen_unix_ms"].as_u64(),
                    launch_method: Some(plan.spec.launch_method().to_string()),
                    error: None,
                };
            }
        }
        CompanionObservation {
            observed_state: CompanionObservedState::NotRunning,
            launch_method: Some(plan.spec.launch_method().to_string()),
            ..Default::default()
        }
    }
}
