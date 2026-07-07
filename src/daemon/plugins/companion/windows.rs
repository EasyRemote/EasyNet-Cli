// EasyNet CLI — Windows desktop companion supervisor
// ==================================================

use std::process::Command;

use crate::daemon::plugins::errors::{PluginHostError, Result};

use super::heartbeat::CompanionStatusFileObserver;
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

    fn stop(&self, plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        #[cfg(target_os = "windows")]
        {
            if let Some(pid) = self.observe(plan).pid {
                return stop_windows_pid(plan, pid);
            }
            let PlatformCompanionSpec::Windows { exe, .. } = &plan.spec else {
                return Ok(CompanionActionReport::unchanged(
                    "not a Windows companion plan",
                ));
            };
            let Some(image_name) = exe.file_name().and_then(|name| name.to_str()) else {
                return Err(PluginHostError::InvalidCompanionManifest {
                    id: plan.package_id.clone(),
                    reason: "Windows companion executable has no image name".to_string(),
                });
            };
            stop_windows_image(plan, image_name)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = plan;
            Ok(CompanionActionReport::unchanged("unsupported platform"))
        }
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
        if let Some(observation) = CompanionStatusFileObserver::current().observe_path(plan, &path)
        {
            return observation;
        }
        CompanionObservation {
            observed_state: CompanionObservedState::NotRunning,
            launch_method: Some(plan.spec.launch_method().to_string()),
            ..Default::default()
        }
    }
}

#[cfg(target_os = "windows")]
fn stop_windows_pid(plan: &DesktopCompanionPlan, pid: u64) -> Result<CompanionActionReport> {
    let pid_arg = pid.to_string();
    let status = Command::new("taskkill")
        .args(["/PID", pid_arg.as_str(), "/T", "/F"])
        .status()
        .map_err(|source| PluginHostError::WriteFailed {
            path: std::path::PathBuf::from("taskkill"),
            source,
        })?;
    if status.success() {
        Ok(CompanionActionReport::changed(
            "stopped Windows companion pid",
        ))
    } else {
        Err(PluginHostError::InvalidCompanionManifest {
            id: plan.package_id.clone(),
            reason: format!("taskkill by pid failed with {status}"),
        })
    }
}

#[cfg(target_os = "windows")]
fn stop_windows_image(
    plan: &DesktopCompanionPlan,
    image_name: &str,
) -> Result<CompanionActionReport> {
    let status = Command::new("taskkill")
        .args(["/IM", image_name, "/T", "/F"])
        .status()
        .map_err(|source| PluginHostError::WriteFailed {
            path: std::path::PathBuf::from("taskkill"),
            source,
        })?;
    if status.success() {
        Ok(CompanionActionReport::changed(
            "stopped Windows companion image",
        ))
    } else {
        Err(PluginHostError::InvalidCompanionManifest {
            id: plan.package_id.clone(),
            reason: format!("taskkill by image failed with {status}"),
        })
    }
}
