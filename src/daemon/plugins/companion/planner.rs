// EasyNet CLI — desktop companion planning
// ========================================
//
// File: src/daemon/plugins/companion/planner.rs
// Description: Converts companion manifests into platform-specific plans.

use std::path::PathBuf;

use crate::daemon::plugins::manifest::{
    PluginCompanionBootPolicy, PluginCompanionHealthMode, PluginCompanionStopPolicy,
};
use crate::daemon::plugins::package::SharedPluginPackage;

/// Platform-specific executable and supervisor declaration selected from a
/// `desktop_companion` package manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlatformCompanionSpec {
    Macos {
        bundle_id: String,
        app_bundle: PathBuf,
        launch_agent_label: String,
        session: String,
    },
    Windows {
        exe: PathBuf,
        task_name: String,
        session: String,
    },
    Linux {
        exe: PathBuf,
        unit_name: String,
        session: String,
    },
}

impl PlatformCompanionSpec {
    pub fn launch_method(&self) -> &'static str {
        match self {
            Self::Macos { .. } => "launch_agent",
            Self::Windows { .. } => "startup_task",
            Self::Linux { .. } => "systemd_user",
        }
    }

    pub fn executable_name(&self) -> Option<String> {
        let path = match self {
            Self::Macos { app_bundle, .. } => app_bundle
                .file_stem()
                .map(|stem| app_bundle.join("Contents/MacOS").join(stem)),
            Self::Windows { exe, .. } | Self::Linux { exe, .. } => Some(exe.clone()),
        }?;
        path.file_stem()
            .map(|name| name.to_string_lossy().to_string())
    }
}

/// Companion package plan for the current host platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesktopCompanionPlan {
    pub package_id: String,
    pub package_version: String,
    pub display_name: String,
    pub package_root: PathBuf,
    pub platform: String,
    pub spec: PlatformCompanionSpec,
    pub boot_policy: PluginCompanionBootPolicy,
    pub stop_policy: PluginCompanionStopPolicy,
    pub health: PluginCompanionHealthMode,
    pub status_file: Option<PathBuf>,
}

/// Pure planner. It does not probe OS state or mutate the filesystem.
#[derive(Clone, Debug)]
pub struct DesktopCompanionPlanner {
    platform: String,
}

impl DesktopCompanionPlanner {
    pub fn new(platform: impl Into<String>) -> Self {
        Self {
            platform: platform.into(),
        }
    }

    pub fn current() -> Self {
        Self::new(current_platform())
    }

    pub fn platform(&self) -> &str {
        &self.platform
    }

    pub fn plan_package(
        &self,
        package: &SharedPluginPackage,
    ) -> Result<DesktopCompanionPlan, String> {
        let manifest = package.manifest();
        let companion = manifest
            .companion()
            .ok_or_else(|| "desktop_companion package does not declare [companion]".to_string())?;
        let spec = match self.platform.as_str() {
            "macos" => companion.macos().map(|macos| PlatformCompanionSpec::Macos {
                bundle_id: macos.bundle_id().to_string(),
                app_bundle: package.root().join(macos.app_bundle()),
                launch_agent_label: macos.launch_agent_label().to_string(),
                session: macos.session().to_string(),
            }),
            "windows" => companion
                .windows()
                .map(|windows| PlatformCompanionSpec::Windows {
                    exe: package.root().join(windows.exe()),
                    task_name: windows.task_name().to_string(),
                    session: windows.session().to_string(),
                }),
            "linux" => companion.linux().map(|linux| PlatformCompanionSpec::Linux {
                exe: package.root().join(linux.exe()),
                unit_name: linux.unit_name().to_string(),
                session: linux.session().to_string(),
            }),
            _ => None,
        }
        .ok_or_else(|| format!("desktop companion does not support {}", self.platform))?;

        Ok(DesktopCompanionPlan {
            package_id: manifest.id().to_string(),
            package_version: manifest.version().to_string(),
            display_name: companion.display_name().to_string(),
            package_root: package.root().to_path_buf(),
            platform: self.platform.clone(),
            spec,
            boot_policy: companion.boot_policy(),
            stop_policy: companion.stop_policy(),
            health: companion.health(),
            status_file: companion
                .status_file()
                .map(|status_file| package.root().join(status_file)),
        })
    }
}

pub fn current_platform() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        "unknown"
    }
}
