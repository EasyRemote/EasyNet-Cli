// EasyNet CLI — plugin load planner
// =================================
//
// File: src/daemon/plugins/load_plan.rs
// Description: Convert package index entries into per-boot load decisions.

use crate::daemon::plugins::index::PluginPackageIndex;
use crate::daemon::plugins::manifest::{PluginCallMode, PluginDeclarativeBinding, PluginKind};
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
}

/// Load decision for one indexed package.
#[derive(Clone)]
pub struct PluginLoadPlanEntry {
    package: SharedPluginPackage,
    status: PluginLoadStatus,
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
}

impl PluginLoadPlanner {
    /// Build a planner for a concrete platform string.
    pub fn new(platform: impl Into<String>) -> Self {
        Self {
            platform: platform.into(),
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
        }
    }

    /// Produce a load plan. This does not mutate the package index or catalog.
    pub fn plan(&self, index: &PluginPackageIndex) -> PluginLoadPlan {
        let entries = index
            .packages()
            .iter()
            .cloned()
            .map(|package| {
                let status = self.status_for(&package);
                PluginLoadPlanEntry { package, status }
            })
            .collect();
        PluginLoadPlan { entries }
    }

    fn status_for(&self, package: &SharedPluginPackage) -> PluginLoadStatus {
        if !supports_platform(package, &self.platform) {
            return PluginLoadStatus::PlatformMismatch {
                current: self.platform.clone(),
            };
        }
        let Some(binding) = package.builtin_binding() else {
            return match package.manifest().kind() {
                PluginKind::Builtin => PluginLoadStatus::MissingBuiltinBinding,
                PluginKind::Declarative => declarative_load_status(package),
                PluginKind::Sidecar => sidecar_load_status(package),
            };
        };
        if self.respect_env_gates {
            if let Some(env_var) = binding.enabled_env_var {
                if env_disabled(env_var) {
                    return PluginLoadStatus::DisabledByEnv { env_var };
                }
            }
        }
        PluginLoadStatus::Loaded
    }
}

fn sidecar_load_status(package: &SharedPluginPackage) -> PluginLoadStatus {
    let path = package.entrypoint_path();
    let meta = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(_) => {
            return PluginLoadStatus::MissingEntrypoint {
                path: path.display().to_string(),
            };
        }
    };
    if !meta.is_file() {
        return PluginLoadStatus::EntrypointNotExecutable {
            path: path.display().to_string(),
        };
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if meta.permissions().mode() & 0o111 == 0 {
            return PluginLoadStatus::EntrypointNotExecutable {
                path: path.display().to_string(),
            };
        }
    }
    PluginLoadStatus::Loaded
}

fn declarative_load_status(package: &SharedPluginPackage) -> PluginLoadStatus {
    match package.manifest().declarative_binding() {
        Some(PluginDeclarativeBinding::Exec { .. }) => PluginLoadStatus::Loaded,
        Some(PluginDeclarativeBinding::Eal { .. } | PluginDeclarativeBinding::Mcp { .. })
            if package
                .manifest()
                .abilities()
                .iter()
                .all(|ability| ability.call_mode() == PluginCallMode::Rpc) =>
        {
            PluginLoadStatus::Loaded
        }
        Some(_) | None => PluginLoadStatus::NotLoadableInThisRelease,
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
}
