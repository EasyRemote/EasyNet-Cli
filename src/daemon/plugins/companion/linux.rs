// EasyNet CLI — Linux desktop companion supervisor
// ================================================

use crate::daemon::plugins::errors::Result;

use super::planner::{DesktopCompanionPlan, PlatformCompanionSpec};
use super::session::DesktopCompanionSessionProbe;
use super::status::{
    CompanionObservation, CompanionObservedState, CompanionSessionStatus, CompanionSupervisorState,
};
use super::{CompanionActionReport, DesktopCompanionSupervisor};

#[derive(Default)]
pub struct LinuxDesktopCompanionSupervisor;

impl LinuxDesktopCompanionSupervisor {
    pub const fn new() -> Self {
        Self
    }
}

impl DesktopCompanionSupervisor for LinuxDesktopCompanionSupervisor {
    fn platform(&self) -> &'static str {
        "linux"
    }

    fn probe_session(&self) -> CompanionSessionStatus {
        DesktopCompanionSessionProbe::current().probe("linux")
    }

    fn install(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged(
            "Linux tray installation is not supported in this release",
        ))
    }

    fn enable(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged(
            "Linux tray enable is not supported in this release",
        ))
    }

    fn disable(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged(
            "Linux tray disable is not supported in this release",
        ))
    }

    fn remove(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged(
            "Linux tray removal is not supported in this release",
        ))
    }

    fn start(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged(
            "Linux tray start is not supported in this release",
        ))
    }

    fn stop(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged(
            "Linux tray stop is not supported in this release",
        ))
    }

    fn supervisor_state(&self, plan: &DesktopCompanionPlan) -> CompanionSupervisorState {
        if !self.probe_session().is_available() {
            return CompanionSupervisorState::UnsupportedSession;
        }
        match &plan.spec {
            PlatformCompanionSpec::Linux { .. } => CompanionSupervisorState::UnsupportedPlatform,
            _ => CompanionSupervisorState::UnsupportedPlatform,
        }
    }

    fn observe(&self, plan: &DesktopCompanionPlan) -> CompanionObservation {
        CompanionObservation {
            observed_state: CompanionObservedState::NotRunning,
            launch_method: Some(plan.spec.launch_method().to_string()),
            ..Default::default()
        }
    }
}

pub struct UnsupportedDesktopCompanionSupervisor {
    platform: &'static str,
}

impl UnsupportedDesktopCompanionSupervisor {
    pub const fn new(platform: &'static str) -> Self {
        Self { platform }
    }
}

impl DesktopCompanionSupervisor for UnsupportedDesktopCompanionSupervisor {
    fn platform(&self) -> &'static str {
        self.platform
    }

    fn probe_session(&self) -> CompanionSessionStatus {
        CompanionSessionStatus::Unsupported {
            reason: format!("{} is unsupported", self.platform),
        }
    }

    fn install(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged("unsupported platform"))
    }

    fn enable(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged("unsupported platform"))
    }

    fn disable(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged("unsupported platform"))
    }

    fn remove(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged("unsupported platform"))
    }

    fn start(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged("unsupported platform"))
    }

    fn stop(&self, _plan: &DesktopCompanionPlan) -> Result<CompanionActionReport> {
        Ok(CompanionActionReport::unchanged("unsupported platform"))
    }

    fn supervisor_state(&self, _plan: &DesktopCompanionPlan) -> CompanionSupervisorState {
        CompanionSupervisorState::UnsupportedPlatform
    }

    fn observe(&self, _plan: &DesktopCompanionPlan) -> CompanionObservation {
        CompanionObservation {
            observed_state: CompanionObservedState::Unknown,
            ..Default::default()
        }
    }
}
