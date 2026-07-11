// EasyNet CLI — plugin load planner
// =================================
//
// File: src/daemon/plugins/load_plan.rs
// Description: Convert package index entries into per-boot load decisions.

use crate::daemon::plugins::companion::{
    DesktopCompanionPlan, DesktopCompanionPlanner, DesktopCompanionSessionProbe,
};
use crate::daemon::plugins::index::PluginPackageIndex;
use crate::daemon::plugins::manifest::{CallMode, PluginDeclarativeBinding, PluginKind};
use crate::daemon::plugins::package::SharedPluginPackage;

/// Per-boot load state for one package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PluginLoadStatus {
    Loaded,
    DisabledByEnv { env_var: &'static str },
    PlatformMismatch { current: String },
    NotLoadableInThisRelease,
    MissingEntrypoint { path: String },
    EntrypointNotExecutable { path: String },
    MissingBuiltinBinding,
    CompanionUnsupportedPlatform { current: String },
    CompanionUnsupportedSession { reason: String },
    CompanionInvalidSpec { reason: String },
}

/// Load decision for one indexed package.
#[derive(Clone)]
pub struct PluginLoadPlanEntry {
    package: SharedPluginPackage,
    status: PluginLoadStatus,
    companion_plan: Option<DesktopCompanionPlan>,
}

impl PluginLoadPlanEntry {
    /// Package covered by this decision.
    pub fn package(&self) -> &SharedPluginPackage {
        &self.package
    }

    /// Per-boot status.
    pub fn status(&self) -> &PluginLoadStatus {
        &self.status
    }

    /// Whether this package should register handlers in this boot.
    pub fn is_loaded(&self) -> bool {
        self.status == PluginLoadStatus::Loaded
            && self.package.manifest().kind() != PluginKind::DesktopCompanion
    }

    /// Companion plan, when this package declares a user-session UI process
    /// for the current host platform.
    pub fn companion_plan(&self) -> Option<&DesktopCompanionPlan> {
        self.companion_plan.as_ref()
    }
}

/// Complete per-boot plugin load plan.
#[derive(Clone, Default)]
pub struct PluginLoadPlan {
    entries: Vec<PluginLoadPlanEntry>,
}

impl PluginLoadPlan {
    /// All package load entries.
    pub fn entries(&self) -> &[PluginLoadPlanEntry] {
        &self.entries
    }

    /// Resolve a package's per-boot load entry by package id and version.
    pub fn entry_for_package(&self, id: &str, version: &str) -> Option<&PluginLoadPlanEntry> {
        self.entries.iter().find(|entry| {
            entry.package().id().as_str() == id && entry.package().version().as_str() == version
        })
    }
}

/// Planner converting install/package state into daemon boot state.
#[derive(Clone)]
pub struct PluginLoadPlanner {
    platform: String,
    respect_env_gates: bool,
    companion_planner: DesktopCompanionPlanner,
    companion_session_probe: DesktopCompanionSessionProbe,
}

impl PluginLoadPlanner {
    /// Build a planner for a concrete platform string.
    pub fn new(platform: impl Into<String>) -> Self {
        let platform = platform.into();
        Self {
            companion_planner: DesktopCompanionPlanner::new(platform.clone()),
            companion_session_probe: DesktopCompanionSessionProbe::current(),
            platform,
            respect_env_gates: true,
        }
    }

    /// Build a planner for the current target platform.
    pub fn current() -> Self {
        Self::new(current_platform())
    }

    /// Build a planner for deterministic descriptor/test projection.
    ///
    /// This keeps platform filtering but deliberately ignores host env gates so
    /// helpers such as `published_abilities()` do not change shape because an
    /// operator exported `EASYNET_*_PLUGIN=off` in their shell.
    pub fn current_without_env_gates() -> Self {
        Self {
            platform: current_platform().to_string(),
            respect_env_gates: false,
            companion_planner: DesktopCompanionPlanner::current(),
            companion_session_probe: DesktopCompanionSessionProbe::current(),
        }
    }

    /// Produce a load plan. This does not mutate the package index or catalog.
    pub fn plan(&self, index: &PluginPackageIndex) -> PluginLoadPlan {
        let entries = index
            .packages()
            .iter()
            .cloned()
            .map(|package| {
                let (status, companion_plan) = self.status_for(&package);
                PluginLoadPlanEntry {
                    package,
                    status,
                    companion_plan,
                }
            })
            .collect();
        PluginLoadPlan { entries }
    }

    fn status_for(
        &self,
        package: &SharedPluginPackage,
    ) -> (PluginLoadStatus, Option<DesktopCompanionPlan>) {
        if !supports_platform(package, &self.platform) {
            return (
                PluginLoadStatus::PlatformMismatch {
                    current: self.platform.clone(),
                },
                None,
            );
        }
        let Some(binding) = package.builtin_binding() else {
            return match package.manifest().kind() {
                PluginKind::Builtin => (PluginLoadStatus::MissingBuiltinBinding, None),
                PluginKind::Declarative => declarative_load_status(package),
                PluginKind::Sidecar => sidecar_load_status(package),
                PluginKind::DesktopCompanion => companion_load_status(
                    package,
                    &self.companion_planner,
                    &self.companion_session_probe,
                ),
            };
        };
        if self.respect_env_gates {
            if let Some(env_var) = binding.enabled_env_var() {
                if env_disabled(env_var) {
                    return (PluginLoadStatus::DisabledByEnv { env_var }, None);
                }
            }
        }
        (PluginLoadStatus::Loaded, None)
    }
}

fn companion_load_status(
    package: &SharedPluginPackage,
    planner: &DesktopCompanionPlanner,
    session_probe: &DesktopCompanionSessionProbe,
) -> (PluginLoadStatus, Option<DesktopCompanionPlan>) {
    if package.manifest().companion().is_none() {
        return (
            PluginLoadStatus::CompanionInvalidSpec {
                reason: "desktop_companion package does not declare [companion]".to_string(),
            },
            None,
        );
    }
    match planner.plan_package(package) {
        Ok(plan) => {
            let session = session_probe.probe(&plan.platform);
            if let crate::daemon::plugins::CompanionSessionStatus::Unsupported { reason } = session
            {
                return (
                    PluginLoadStatus::CompanionUnsupportedSession { reason },
                    Some(plan),
                );
            }
            (PluginLoadStatus::Loaded, Some(plan))
        }
        Err(reason) => (
            PluginLoadStatus::CompanionUnsupportedPlatform {
                current: if reason.is_empty() {
                    planner.platform().to_string()
                } else {
                    format!("{}: {reason}", planner.platform())
                },
            },
            None,
        ),
    }
}

fn sidecar_load_status(
    package: &SharedPluginPackage,
) -> (PluginLoadStatus, Option<DesktopCompanionPlan>) {
    let path = package.entrypoint_path();
    let meta = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(_) => {
            return (
                PluginLoadStatus::MissingEntrypoint {
                    path: path.display().to_string(),
                },
                None,
            );
        }
    };
    if !meta.is_file() {
        return (
            PluginLoadStatus::EntrypointNotExecutable {
                path: path.display().to_string(),
            },
            None,
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if meta.permissions().mode() & 0o111 == 0 {
            return (
                PluginLoadStatus::EntrypointNotExecutable {
                    path: path.display().to_string(),
                },
                None,
            );
        }
    }
    (PluginLoadStatus::Loaded, None)
}

fn declarative_load_status(
    package: &SharedPluginPackage,
) -> (PluginLoadStatus, Option<DesktopCompanionPlan>) {
    match package.manifest().declarative_binding() {
        Some(PluginDeclarativeBinding::Exec { .. }) => (PluginLoadStatus::Loaded, None),
        Some(PluginDeclarativeBinding::Eal { .. } | PluginDeclarativeBinding::Mcp { .. })
            if package
                .manifest()
                .abilities()
                .iter()
                .all(|ability| ability.call_mode() == CallMode::Rpc) =>
        {
            (PluginLoadStatus::Loaded, None)
        }
        Some(_) | None => (PluginLoadStatus::NotLoadableInThisRelease, None),
    }
}

fn supports_platform(package: &SharedPluginPackage, platform: &str) -> bool {
    let platforms = package.manifest().platforms();
    platforms.is_empty() || platforms.iter().any(|candidate| candidate == platform)
}

fn env_disabled(env_var: &'static str) -> bool {
    std::env::var(env_var)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off"
            )
        })
        .unwrap_or(false)
}

fn current_platform() -> &'static str {
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
    use super::*;
    use crate::daemon::plugins::package::PluginPackage;
    use crate::daemon::plugins::CompanionSessionStatus;

    #[test]
    fn desktop_companion_is_planned_but_not_runtime_loaded() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_companion_package(dir.path());
        let package = std::sync::Arc::new(
            PluginPackage::from_installed(dir.path(), None).expect("companion package"),
        );
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let plan = PluginLoadPlanner::new("macos").plan(&index);
        let entry = plan.entries().first().expect("plan entry");

        assert_eq!(entry.status(), &PluginLoadStatus::Loaded);
        assert!(entry.companion_plan().is_some());
        assert!(!entry.is_loaded());
    }

    #[test]
    fn linux_companion_without_graphical_session_is_session_unsupported() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_linux_companion_package(dir.path());
        let package = std::sync::Arc::new(
            PluginPackage::from_installed(dir.path(), None).expect("companion package"),
        );
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let planner = linux_test_planner(CompanionSessionStatus::Unsupported {
            reason: "headless test session".to_string(),
        });
        let plan = planner.plan(&index);
        let entry = plan.entries().first().expect("plan entry");

        assert_eq!(
            entry.status(),
            &PluginLoadStatus::CompanionUnsupportedSession {
                reason: "headless test session".to_string()
            }
        );
        assert!(entry.companion_plan().is_some());
        assert!(!entry.is_loaded());
    }

    #[test]
    fn linux_companion_with_graphical_session_loads_package_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_linux_companion_package(dir.path());
        let package = std::sync::Arc::new(
            PluginPackage::from_installed(dir.path(), None).expect("companion package"),
        );
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let planner = linux_test_planner(CompanionSessionStatus::Available);
        let plan = planner.plan(&index);
        let entry = plan.entries().first().expect("plan entry");

        assert_eq!(entry.status(), &PluginLoadStatus::Loaded);
        assert!(entry.companion_plan().is_some());
        assert!(!entry.is_loaded());
    }

    fn linux_test_planner(session: CompanionSessionStatus) -> PluginLoadPlanner {
        PluginLoadPlanner {
            platform: "linux".to_string(),
            respect_env_gates: true,
            companion_planner: DesktopCompanionPlanner::new("linux"),
            companion_session_probe: DesktopCompanionSessionProbe::with_linux_status(session),
        }
    }

    #[test]
    #[cfg(feature = "remote-desktop")]
    fn remote_desktop_builtin_loads_in_linux_docker_profile() {
        let index = PluginPackageIndex::builtin().expect("builtin package index must load");
        let plan = PluginLoadPlanner::new("linux").plan(&index);
        let entry = plan
            .entries()
            .iter()
            .find(|entry| entry.package().id().as_str() == "easynet.remote_desktop")
            .expect("remote desktop builtin package must be indexed");

        assert_eq!(
            entry.status(),
            &PluginLoadStatus::Loaded,
            "Linux Docker daemon builds compile remote-desktop by default, so the manifest \
             must allow the builtin package to register its baseline abilities"
        );
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

    fn write_linux_companion_package(root: &std::path::Path) {
        std::fs::create_dir_all(root.join("dist/linux")).expect("linux dist dir");
        std::fs::write(root.join("dist/linux/easynet-tray"), "").expect("linux executable");
        std::fs::write(
            root.join("plugin.toml"),
            r#"
schema_version = "1"
id = "easynet.desktop.tray"
version = "0.1.0"
kind = "desktop_companion"
entrypoint = "dist/linux/easynet-tray"
abilities = []
permissions = ["clipboard_read"]
resources = ["desktop_session"]
platforms = ["linux"]

[limits]
max_sessions = 1
max_frame_queue = 1

[companion]
display_name = "EasyNet Tray"
lifecycle = "user_session"
boot_policy = "ensure_running_after_daemon_ready"
stop_policy = "keep_running"
health = "status_file"
status_file = "companions/easynet.desktop.tray/status.json"

[companion.linux]
exe = "dist/linux/easynet-tray"
supervisor = "systemd_user"
unit_name = "easynet-tray.service"
session = "graphical"
"#,
        )
        .expect("manifest");
    }
}
