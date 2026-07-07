// EasyNet CLI — desktop companion planning
// ========================================
//
// File: src/daemon/plugins/companion/planner.rs
// Description: Converts companion manifests into platform-specific plans.

use std::path::{Path, PathBuf};

use crate::daemon::plugins::manifest::{
    PluginCompanionBootPolicy, PluginCompanionHealthMode, PluginCompanionStopPolicy,
};
use crate::daemon::plugins::package::SharedPluginPackage;

use super::status_file::CompanionStatusFilePath;

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

    pub fn executable_artifact_path(&self) -> &Path {
        match self {
            Self::Macos { app_bundle, .. } => app_bundle.as_path(),
            Self::Windows { exe, .. } | Self::Linux { exe, .. } => exe.as_path(),
        }
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
            status_file: companion.status_file().map(|status_file| {
                CompanionStatusFilePath::resolve(package.root(), status_file).into_path_buf()
            }),
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::daemon::plugins::package::PluginPackage;

    use super::*;

    #[test]
    fn planner_resolves_manifest_status_file_into_plan() {
        let root = tempfile::tempdir().expect("package root");
        write_companion_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));

        let plan = DesktopCompanionPlanner::new("macos")
            .plan_package(&package)
            .expect("plan");

        let status_file = plan.status_file.expect("status file");
        assert!(status_file.ends_with("companions/easynet.desktop.menubar/status.json"));
        assert!(!status_file.starts_with(root.path()));
    }

    fn write_companion_package(root: &std::path::Path) {
        std::fs::create_dir_all(root.join("dist/macos/EasyNetMenuBar.app/Contents/MacOS"))
            .expect("app bundle dir");
        std::fs::write(
            root.join("dist/macos/EasyNetMenuBar.app/Contents/MacOS/EasyNetMenuBar"),
            "",
        )
        .expect("app executable");
        std::fs::write(
            root.join("plugin.toml"),
            r#"
schema_version = "1"
id = "easynet.desktop.menubar"
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
status_file = "companions/easynet.desktop.menubar/status.json"

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
