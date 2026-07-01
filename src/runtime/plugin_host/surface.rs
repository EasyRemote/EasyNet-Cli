// EasyNet CLI — plugin product surface projection
// ===============================================
//
// File: src/runtime/plugin_host/surface.rs
// Description: Operator-visible plugin package and ability state.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::runtime::plugin_host::index::PluginPackageIndex;
use crate::runtime::plugin_host::index::PluginPackageIndexError;
use crate::runtime::plugin_host::load_plan::{PluginLoadPlan, PluginLoadStatus};
use crate::runtime::plugin_host::manifest::{PluginAbilityLayer, PluginCallMode, PluginKind};
use crate::runtime::plugin_host::realtime::{
    activation_plans_for_manifest, PluginRealtimeActivationPlan,
};

/// Operator-visible plugin package and ability report.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PluginSurfaceReport {
    pub packages: Vec<PluginPackageSurfaceRecord>,
    pub abilities: Vec<PluginAbilitySurfaceRecord>,
}

/// One operator-visible plugin package row.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct PluginPackageSurfaceRecord {
    pub package_id: String,
    pub package_version: String,
    pub kind: PluginKindView,
    pub planned_load_status: String,
    pub daemon_runtime_status: String,
    #[serde(default)]
    pub load_status: String,
    pub ability_count: usize,
    pub descriptor_published: bool,
    pub runtime_published: bool,
    pub invokable: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub realtime_activation_plans: Vec<PluginRealtimeActivationPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Product policy for where a plugin-owned ability may be shown.
///
/// What this is NOT: a runtime registration decision. Registration is still
/// owned by [`PluginLoadPlan`] and [`crate::runtime::plugin_host::host_api`].
/// This type exists so CLI discovery and descriptor generation do not silently
/// disagree about what "published" means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginAbilitySurface {
    /// Static descriptor TOML can be generated from package metadata.
    Descriptor,
    /// Daemon runtime discovery should advertise the ability this boot.
    RuntimeDiscovery,
    /// The ability may be invoked through LocalRuntime this boot.
    Invocation,
}

/// One operator-visible plugin ability row.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct PluginAbilitySurfaceRecord {
    pub package_id: String,
    pub package_version: String,
    pub ability: String,
    pub kind: PluginKindView,
    pub layer: PluginAbilityLayerView,
    pub call_mode: PluginCallModeView,
    pub planned_load_status: String,
    pub daemon_runtime_status: String,
    #[serde(default)]
    pub load_status: String,
    pub descriptor_published: bool,
    pub runtime_published: bool,
    pub invokable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Serializable plugin kind for CLI/API output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginKindView {
    Declarative,
    Sidecar,
    Builtin,
    Unknown,
}

/// Serializable ability layer for CLI/API output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginAbilityLayerView {
    Introspection,
    Control,
    Observation,
    Operational,
    Unknown,
}

/// Serializable ability call mode for CLI/API output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCallModeView {
    Rpc,
    Stream,
    Bidi,
    Unknown,
}

/// Project package index and load plan into the product-facing plugin list.
pub struct PluginSurfaceProjector;

impl PluginSurfaceProjector {
    /// Return package and ability rows for offline projection.
    pub fn project_report(
        index: &PluginPackageIndex,
        load_plan: &PluginLoadPlan,
    ) -> PluginSurfaceReport {
        Self::project_report_with_daemon(index, load_plan, None, &[])
    }

    /// Return package and ability rows, optionally checking daemon runtime
    /// registration.
    pub fn project_report_with_daemon(
        index: &PluginPackageIndex,
        load_plan: &PluginLoadPlan,
        daemon_abilities: Option<&BTreeSet<String>>,
        index_errors: &[PluginPackageIndexError],
    ) -> PluginSurfaceReport {
        PluginSurfaceReport {
            packages: Self::project_packages_with_daemon(
                index,
                load_plan,
                daemon_abilities,
                index_errors,
            ),
            abilities: Self::project_with_daemon(index, load_plan, daemon_abilities, index_errors),
        }
    }

    /// Return one deterministic row per indexed plugin package.
    pub fn project_packages(
        index: &PluginPackageIndex,
        load_plan: &PluginLoadPlan,
    ) -> Vec<PluginPackageSurfaceRecord> {
        Self::project_packages_with_daemon(index, load_plan, None, &[])
    }

    /// Return package rows with package-level realtime readiness.
    pub fn project_packages_with_daemon(
        index: &PluginPackageIndex,
        load_plan: &PluginLoadPlan,
        daemon_abilities: Option<&BTreeSet<String>>,
        index_errors: &[PluginPackageIndexError],
    ) -> Vec<PluginPackageSurfaceRecord> {
        let mut rows = Vec::new();
        for package in index.packages() {
            let planned_load_status = load_plan
                .entry_for_package(package.id().as_str(), package.version().as_str())
                .map(|entry| status_label(entry.status()))
                .unwrap_or("not_planned")
                .to_string();
            let runtime_registered_count = package
                .manifest()
                .abilities()
                .iter()
                .filter(|ability| {
                    daemon_abilities
                        .map(|abilities| abilities.contains(ability.name()))
                        .unwrap_or(false)
                })
                .count();
            let daemon_runtime_status = match daemon_abilities {
                Some(_)
                    if runtime_registered_count == package.manifest().abilities().len()
                        && runtime_registered_count > 0 =>
                {
                    "loaded"
                }
                Some(_) if runtime_registered_count > 0 => "partial",
                Some(_) => "not_loaded",
                None => "offline",
            }
            .to_string();
            rows.push(PluginPackageSurfaceRecord {
                package_id: package.id().as_str().to_string(),
                package_version: package.version().as_str().to_string(),
                kind: package.manifest().kind().into(),
                planned_load_status: planned_load_status.clone(),
                daemon_runtime_status,
                load_status: planned_load_status,
                ability_count: package.manifest().abilities().len(),
                descriptor_published: true,
                runtime_published: runtime_registered_count > 0,
                invokable: runtime_registered_count > 0,
                realtime_activation_plans: activation_plans_for_manifest(
                    package.id().as_str(),
                    package.version().as_str(),
                    package.manifest(),
                    daemon_abilities,
                ),
                error: None,
            });
        }
        for error in index_errors {
            rows.push(PluginPackageSurfaceRecord {
                package_id: error.id.clone(),
                package_version: error.version.clone(),
                kind: PluginKindView::Unknown,
                planned_load_status: "index_error".to_string(),
                daemon_runtime_status: daemon_abilities
                    .map(|_| "not_loaded")
                    .unwrap_or("offline")
                    .to_string(),
                load_status: "index_error".to_string(),
                ability_count: 0,
                descriptor_published: false,
                runtime_published: false,
                invokable: false,
                realtime_activation_plans: Vec::new(),
                error: Some(format!("{}: {}", error.package_dir.display(), error.reason)),
            });
        }
        rows.sort_by(|a, b| {
            a.package_id
                .cmp(&b.package_id)
                .then(a.package_version.cmp(&b.package_version))
        });
        rows
    }

    /// Return one deterministic row per indexed plugin ability.
    pub fn project(
        index: &PluginPackageIndex,
        load_plan: &PluginLoadPlan,
    ) -> Vec<PluginAbilitySurfaceRecord> {
        Self::project_with_daemon(index, load_plan, None, &[])
    }

    /// Return one deterministic row per indexed plugin ability, optionally
    /// checking actual daemon catalog registration.
    ///
    /// What this is NOT: a load-plan recomputation. The load status still comes
    /// from `PluginLoadPlan`; the daemon ability set only tightens
    /// runtime/invocation visibility after registration or hot reload.
    pub fn project_with_daemon(
        index: &PluginPackageIndex,
        load_plan: &PluginLoadPlan,
        daemon_abilities: Option<&BTreeSet<String>>,
        index_errors: &[PluginPackageIndexError],
    ) -> Vec<PluginAbilitySurfaceRecord> {
        let mut rows = Vec::new();
        for package in index.packages() {
            let planned_load_status = load_plan
                .entry_for_package(package.id().as_str(), package.version().as_str())
                .map(|entry| status_label(entry.status()))
                .unwrap_or("not_planned")
                .to_string();
            for ability in package.manifest().abilities() {
                let runtime_registered = daemon_abilities
                    .map(|abilities| abilities.contains(ability.name()))
                    .unwrap_or(false);
                let daemon_runtime_status = match daemon_abilities {
                    Some(_) if runtime_registered => "loaded",
                    Some(_) => "not_loaded",
                    None => "offline",
                }
                .to_string();
                rows.push(PluginAbilitySurfaceRecord {
                    package_id: package.id().as_str().to_string(),
                    package_version: package.version().as_str().to_string(),
                    ability: ability.name().to_string(),
                    kind: package.manifest().kind().into(),
                    layer: ability.layer().into(),
                    call_mode: ability.call_mode().into(),
                    planned_load_status: planned_load_status.clone(),
                    daemon_runtime_status,
                    load_status: planned_load_status.clone(),
                    descriptor_published: true,
                    runtime_published: runtime_registered,
                    invokable: runtime_registered,
                    error: None,
                });
            }
        }
        for error in index_errors {
            rows.push(PluginAbilitySurfaceRecord {
                package_id: error.id.clone(),
                package_version: error.version.clone(),
                ability: "<package>".to_string(),
                kind: PluginKindView::Unknown,
                layer: PluginAbilityLayerView::Unknown,
                call_mode: PluginCallModeView::Unknown,
                planned_load_status: "index_error".to_string(),
                daemon_runtime_status: daemon_abilities
                    .map(|_| "not_loaded")
                    .unwrap_or("offline")
                    .to_string(),
                load_status: "index_error".to_string(),
                descriptor_published: false,
                runtime_published: false,
                invokable: false,
                error: Some(format!("{}: {}", error.package_dir.display(), error.reason)),
            });
        }
        rows.sort_by(|a, b| {
            a.package_id
                .cmp(&b.package_id)
                .then(a.package_version.cmp(&b.package_version))
                .then(a.ability.cmp(&b.ability))
        });
        rows
    }

    /// Return true when a row is visible on the requested surface.
    pub fn visible_on(row: &PluginAbilitySurfaceRecord, surface: PluginAbilitySurface) -> bool {
        match surface {
            PluginAbilitySurface::Descriptor => row.descriptor_published,
            PluginAbilitySurface::RuntimeDiscovery => row.runtime_published,
            PluginAbilitySurface::Invocation => row.invokable,
        }
    }
}

fn status_label(status: &PluginLoadStatus) -> &'static str {
    match status {
        PluginLoadStatus::Loaded => "loaded",
        PluginLoadStatus::DisabledByEnv { .. } => "disabled_by_env",
        PluginLoadStatus::PlatformMismatch { .. } => "platform_mismatch",
        PluginLoadStatus::NotLoadableInThisRelease => "not_loadable_in_this_release",
        PluginLoadStatus::MissingEntrypoint { .. } => "missing_entrypoint",
        PluginLoadStatus::EntrypointNotExecutable { .. } => "entrypoint_not_executable",
        PluginLoadStatus::MissingBuiltinBinding => "missing_builtin_binding",
    }
}

impl From<PluginKind> for PluginKindView {
    fn from(value: PluginKind) -> Self {
        match value {
            PluginKind::Declarative => Self::Declarative,
            PluginKind::Sidecar => Self::Sidecar,
            PluginKind::Builtin => Self::Builtin,
        }
    }
}

impl From<PluginAbilityLayer> for PluginAbilityLayerView {
    fn from(value: PluginAbilityLayer) -> Self {
        match value {
            PluginAbilityLayer::Introspection => Self::Introspection,
            PluginAbilityLayer::Control => Self::Control,
            PluginAbilityLayer::Observation => Self::Observation,
            PluginAbilityLayer::Operational => Self::Operational,
        }
    }
}

impl From<PluginCallMode> for PluginCallModeView {
    fn from(value: PluginCallMode) -> Self {
        match value {
            PluginCallMode::Rpc => Self::Rpc,
            PluginCallMode::Stream => Self::Stream,
            PluginCallMode::Bidi => Self::Bidi,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::runtime::plugin_host::package::tests::write_test_package;
    use crate::runtime::plugin_host::package::PluginPackage;
    use crate::runtime::plugin_host::{PluginLoadPlanner, PluginPackageIndex};

    #[test]
    fn plugin_host_surface_keeps_descriptor_independent_from_load_status() {
        let root = tempfile::tempdir().expect("root");
        write_test_package(root.path(), "0.1.0");
        let package = PluginPackage::from_installed(root.path(), None).expect("test package");
        let index = PluginPackageIndex::from_packages(vec![Arc::new(package)]).expect("index");
        let plan = PluginLoadPlanner::new("macos").plan(&index);
        let rows = PluginSurfaceProjector::project(&index, &plan);
        let row = rows
            .iter()
            .find(|row| row.ability == "test.echo")
            .expect("test plugin echo row");

        assert!(row.descriptor_published);
        assert!(!row.runtime_published);
        assert!(!row.invokable);
        assert_eq!(row.planned_load_status, "not_loadable_in_this_release");
        assert_eq!(row.daemon_runtime_status, "offline");
        assert_eq!(row.load_status, "not_loadable_in_this_release");
        assert!(PluginSurfaceProjector::visible_on(
            row,
            PluginAbilitySurface::Descriptor
        ));
        assert!(!PluginSurfaceProjector::visible_on(
            row,
            PluginAbilitySurface::Invocation
        ));
    }

    #[test]
    fn plugin_host_surface_marks_invokable_only_from_daemon_runtime() {
        let root = tempfile::tempdir().expect("root");
        write_test_package(root.path(), "0.1.0");
        let package = PluginPackage::from_installed(root.path(), None).expect("test package");
        let index = PluginPackageIndex::from_packages(vec![Arc::new(package)]).expect("index");
        let plan = PluginLoadPlanner::new("linux").plan(&index);
        let mut daemon = BTreeSet::new();
        daemon.insert("test.echo".to_string());

        let rows = PluginSurfaceProjector::project_with_daemon(&index, &plan, Some(&daemon), &[]);
        let row = rows
            .iter()
            .find(|row| row.ability == "test.echo")
            .expect("test plugin echo row");

        assert_eq!(row.daemon_runtime_status, "loaded");
        assert!(row.runtime_published);
        assert!(row.invokable);
    }

    #[test]
    fn plugin_host_surface_report_keeps_realtime_on_package_rows() {
        let root = tempfile::tempdir().expect("root");
        write_realtime_package(root.path());
        let package = PluginPackage::from_installed(root.path(), None).expect("test package");
        let index = PluginPackageIndex::from_packages(vec![Arc::new(package)]).expect("index");
        let plan = PluginLoadPlanner::new("linux").plan(&index);

        let report = PluginSurfaceProjector::project_report(&index, &plan);
        let ability = report
            .abilities
            .iter()
            .find(|ability| ability.ability == "test.camera")
            .expect("test plugin camera row");
        let package = report
            .packages
            .iter()
            .find(|package| package.package_id == "test.realtime")
            .expect("test plugin package row");

        assert_eq!(ability.ability, "test.camera");
        assert_eq!(package.realtime_activation_plans.len(), 1);
        assert!(package.realtime_activation_plans[0].is_quick_add());
        assert_eq!(
            package.realtime_activation_plans[0].status,
            crate::runtime::plugin_host::PluginRealtimeActivationStatus::Unknown
        );
    }

    #[test]
    fn plugin_host_surface_projects_realtime_activation_readiness() {
        let root = tempfile::tempdir().expect("root");
        write_realtime_package(root.path());
        let package = PluginPackage::from_installed(root.path(), None).expect("test package");
        let index = PluginPackageIndex::from_packages(vec![Arc::new(package)]).expect("index");
        let plan = PluginLoadPlanner::new("linux").plan(&index);
        let daemon = BTreeSet::from(["test.camera".to_string()]);

        let report =
            PluginSurfaceProjector::project_report_with_daemon(&index, &plan, Some(&daemon), &[]);
        let package = report
            .packages
            .iter()
            .find(|package| package.package_id == "test.realtime")
            .expect("test plugin package row");

        assert_eq!(package.realtime_activation_plans.len(), 1);
        assert_eq!(
            package.realtime_activation_plans[0].status,
            crate::runtime::plugin_host::PluginRealtimeActivationStatus::Ready
        );
        assert_eq!(
            package.realtime_activation_plans[0].available_abilities,
            vec!["test.camera".to_string()]
        );
    }

    #[test]
    fn plugin_host_surface_reports_index_errors_without_runtime_visibility() {
        let index = PluginPackageIndex::default();
        let plan = PluginLoadPlanner::new("linux").plan(&index);
        let errors = vec![PluginPackageIndexError {
            id: "broken.plugin".to_string(),
            version: "0.1.0".to_string(),
            package_dir: std::path::PathBuf::from("/tmp/broken.plugin/0.1.0"),
            reason: "hash mismatch".to_string(),
        }];

        let report =
            PluginSurfaceProjector::project_report_with_daemon(&index, &plan, None, &errors);

        assert_eq!(report.packages.len(), 1);
        assert_eq!(report.abilities.len(), 1);
        let package = &report.packages[0];
        assert_eq!(package.package_id, "broken.plugin");
        assert_eq!(package.planned_load_status, "index_error");
        assert!(!package.descriptor_published);
        assert!(!package.runtime_published);
        assert!(!package.invokable);
        assert!(package.realtime_activation_plans.is_empty());
        assert!(package.error.as_deref().unwrap().contains("hash mismatch"));
    }

    fn write_realtime_package(root: &std::path::Path) {
        std::fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        std::fs::write(
            root.join("plugin.toml"),
            r#"
schema_version = "1"
id = "test.realtime"
version = "0.1.0"
kind = "sidecar"
entrypoint = "bin/plugin"
abilities = ["abilities/*.ability.toml"]
permissions = ["camera"]
resources = ["camera"]
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "test.camera"
layer = "operational"
call_mode = "bidi"
bidi_wire_kind = "json_frames"

[[realtime_capability]]
kind = "camera"
modes = ["snapshot", "subscribe", "record"]
transport = "invoke_bidi"
fallback_transport = "invoke_stream"
activation_abilities = ["test.camera"]
permissions = ["camera"]
resources = ["camera"]
quick_add = true
"#,
        )
        .expect("manifest");
        std::fs::write(
            root.join("abilities/test.camera.ability.toml"),
            crate::runtime::plugin_host::package::tests::test_descriptor("test.camera"),
        )
        .expect("descriptor");
    }
}
