// EasyNet CLI — desktop companion status model
// ============================================
//
// File: src/daemon/plugins/companion/status.rs
// Description: Shared state machine for desktop companion package projection.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::daemon::plugins::manifest::{
    PluginCompanionBootPolicy, PluginCompanionHealthMode, PluginCompanionStopPolicy,
};

/// Operator desired state remembered by the companion state store.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionDesiredState {
    Enabled,
    Disabled,
}

impl Default for CompanionDesiredState {
    fn default() -> Self {
        Self::Disabled
    }
}

impl CompanionDesiredState {
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "enabled" => Some(Self::Enabled),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }
}

/// State reported by the platform user-session launcher.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionSupervisorState {
    UnsupportedPlatform,
    UnsupportedSession,
    NotInstalled,
    InstalledDisabled,
    InstalledEnabled,
    InstallError,
    EnableError,
    DisableError,
}

impl CompanionSupervisorState {
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::UnsupportedSession => "unsupported_session",
            Self::NotInstalled => "not_installed",
            Self::InstalledDisabled => "installed_disabled",
            Self::InstalledEnabled => "installed_enabled",
            Self::InstallError => "install_error",
            Self::EnableError => "enable_error",
            Self::DisableError => "disable_error",
        }
    }

    pub const fn is_error(self) -> bool {
        matches!(
            self,
            Self::InstallError | Self::EnableError | Self::DisableError
        )
    }
}

/// Process or heartbeat state observed independently of supervisor metadata.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionObservedState {
    Unknown,
    NotRunning,
    Starting,
    Running,
    Stale,
    Exited,
    VersionMismatch,
    HealthError,
}

impl CompanionObservedState {
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::NotRunning => "not_running",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stale => "stale",
            Self::Exited => "exited",
            Self::VersionMismatch => "version_mismatch",
            Self::HealthError => "health_error",
        }
    }

    pub const fn is_error(self) -> bool {
        matches!(self, Self::VersionMismatch | Self::HealthError)
    }
}

/// Derived state shown to operators.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompanionProjectedState {
    Disabled,
    UnsupportedPlatform,
    UnsupportedSession,
    NotInstalled,
    InstalledDisabled,
    ReadyStopped,
    Starting,
    Running,
    Stale,
    Error,
}

impl CompanionProjectedState {
    pub const fn as_wire_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::UnsupportedSession => "unsupported_session",
            Self::NotInstalled => "not_installed",
            Self::InstalledDisabled => "installed_disabled",
            Self::ReadyStopped => "ready_stopped",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stale => "stale",
            Self::Error => "error",
        }
    }
}

/// Launch-session availability detected by the platform adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompanionSessionStatus {
    Available,
    Unsupported { reason: String },
}

impl CompanionSessionStatus {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Canonical manager-side status before SDK/control-plane DTO projection.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DesktopCompanionStatus {
    pub package_id: String,
    pub package_version: String,
    pub display_name: String,
    pub platform: String,
    pub desired_state: String,
    pub supervisor_state: String,
    pub observed_state: String,
    pub projected_state: String,
    pub boot_policy: String,
    pub stop_policy: String,
    pub health: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_unix_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub launch_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Value>,
}

/// Mutable process facts collected by a platform adapter.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompanionObservation {
    pub observed_state: CompanionObservedState,
    pub pid: Option<u64>,
    pub version: Option<String>,
    pub last_seen_unix_ms: Option<u64>,
    pub launch_method: Option<String>,
    pub error: Option<String>,
}

impl Default for CompanionObservedState {
    fn default() -> Self {
        Self::Unknown
    }
}

pub fn project_state(
    desired: CompanionDesiredState,
    supervisor: CompanionSupervisorState,
    observed: CompanionObservedState,
) -> CompanionProjectedState {
    if desired == CompanionDesiredState::Disabled {
        return CompanionProjectedState::Disabled;
    }
    match supervisor {
        CompanionSupervisorState::UnsupportedPlatform => {
            CompanionProjectedState::UnsupportedPlatform
        }
        CompanionSupervisorState::UnsupportedSession => CompanionProjectedState::UnsupportedSession,
        CompanionSupervisorState::NotInstalled => CompanionProjectedState::NotInstalled,
        CompanionSupervisorState::InstalledDisabled => CompanionProjectedState::InstalledDisabled,
        CompanionSupervisorState::InstallError
        | CompanionSupervisorState::EnableError
        | CompanionSupervisorState::DisableError => CompanionProjectedState::Error,
        CompanionSupervisorState::InstalledEnabled => match observed {
            CompanionObservedState::Running => CompanionProjectedState::Running,
            CompanionObservedState::Starting => CompanionProjectedState::Starting,
            CompanionObservedState::Stale => CompanionProjectedState::Stale,
            CompanionObservedState::NotRunning
            | CompanionObservedState::Exited
            | CompanionObservedState::Unknown => CompanionProjectedState::ReadyStopped,
            state if state.is_error() => CompanionProjectedState::Error,
            _ => CompanionProjectedState::Error,
        },
    }
}

pub const fn boot_policy_wire(policy: PluginCompanionBootPolicy) -> &'static str {
    match policy {
        PluginCompanionBootPolicy::Manual => "manual",
        PluginCompanionBootPolicy::EnsureRunningAfterDaemonReady => {
            "ensure_running_after_daemon_ready"
        }
    }
}

pub const fn stop_policy_wire(policy: PluginCompanionStopPolicy) -> &'static str {
    match policy {
        PluginCompanionStopPolicy::KeepRunning => "keep_running",
        PluginCompanionStopPolicy::StopOnRuntimeStop => "stop_on_runtime_stop",
        PluginCompanionStopPolicy::StopOnPluginDisable => "stop_on_plugin_disable",
    }
}

pub const fn health_wire(health: PluginCompanionHealthMode) -> &'static str {
    match health {
        PluginCompanionHealthMode::ProcessName => "process_name",
        PluginCompanionHealthMode::StatusFile => "status_file",
        PluginCompanionHealthMode::LocalIpc => "local_ipc",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_wins_over_observed_running() {
        assert_eq!(
            project_state(
                CompanionDesiredState::Disabled,
                CompanionSupervisorState::InstalledEnabled,
                CompanionObservedState::Running
            ),
            CompanionProjectedState::Disabled
        );
    }

    #[test]
    fn enabled_installed_running_projects_running() {
        assert_eq!(
            project_state(
                CompanionDesiredState::Enabled,
                CompanionSupervisorState::InstalledEnabled,
                CompanionObservedState::Running
            ),
            CompanionProjectedState::Running
        );
    }
}
