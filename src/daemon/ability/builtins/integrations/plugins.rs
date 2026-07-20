// EasyNet CLI — Plugin Lifecycle Ability
// ======================================
//
// File: src/daemon/ability/builtins/integrations/plugins.rs
// Description: Daemon-local plugin runtime refresh surface.

use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::daemon::plugins::{
    DesktopCompanionManager, PluginActivationBroker, PluginRealtimeActivationReport,
    PluginRuntimeManager,
};

/// Daemon-local ability used by `easynet plugin install/update/remove` after
/// the filesystem transaction commits.
pub const RELOAD_ABILITY: &str = crate::daemon::ability::names::integrations::PLUGIN_RELOAD;
/// Daemon-local ability used by `easynet plugin list` to report actual runtime
/// plugin status instead of an offline load-plan approximation.
pub const STATUS_ABILITY: &str = crate::daemon::ability::names::integrations::PLUGIN_STATUS;
/// Daemon-local ability used by `easynet plugin activate-realtime` to project
/// a package's realtime declaration into concrete runtime prerequisites.
pub const ACTIVATE_REALTIME_ABILITY: &str =
    crate::daemon::ability::names::integrations::PLUGIN_ACTIVATE_REALTIME;
/// Daemon-local ability used by companion-aware CLI/SDK status calls.
pub const COMPANION_STATUS_ABILITY: &str =
    crate::daemon::ability::names::integrations::PLUGIN_COMPANION_STATUS;
/// Daemon-local ability used by daemon post-ready and local CLI reconciliation.
pub const COMPANION_RECONCILE_ABILITY: &str =
    crate::daemon::ability::names::integrations::PLUGIN_COMPANION_RECONCILE;

pub type SharedPluginRegistryCell = OnceLock<Arc<AxonAbilityCatalog>>;

/// Register plugin lifecycle control abilities.
pub fn register(
    reg: &mut AxonAbilityCatalog,
    registry: Arc<SharedPluginRegistryCell>,
    plugin_runtime_manager: Arc<PluginRuntimeManager>,
) {
    let reload_registry = Arc::clone(&registry);
    let reload_runtime_manager = Arc::clone(&plugin_runtime_manager);
    reg.register_rpc_with_owner(
        RELOAD_ABILITY,
        OwnerKind::Device,
        Arc::new(move |args| reload_plugins(args, &reload_registry, &reload_runtime_manager)),
    );
    let status_registry = Arc::clone(&registry);
    let status_runtime_manager = Arc::clone(&plugin_runtime_manager);
    reg.register_rpc_with_owner(
        STATUS_ABILITY,
        OwnerKind::Device,
        Arc::new(move |args| plugin_status(args, &status_registry, &status_runtime_manager)),
    );
    let activate_registry = Arc::clone(&registry);
    let activate_runtime_manager = Arc::clone(&plugin_runtime_manager);
    reg.register_rpc_with_owner(
        ACTIVATE_REALTIME_ABILITY,
        OwnerKind::Device,
        Arc::new(move |args| {
            activate_realtime(args, &activate_registry, &activate_runtime_manager)
        }),
    );
    let companion_status_runtime_manager = Arc::clone(&plugin_runtime_manager);
    reg.register_rpc_with_owner_and_action(
        COMPANION_STATUS_ABILITY,
        OwnerKind::Device,
        crate::daemon::ability::descriptors::AdmissionAction::Read,
        Arc::new(move |args| companion_status(args, &companion_status_runtime_manager)),
    );
    let companion_reconcile_runtime_manager = Arc::clone(&plugin_runtime_manager);
    reg.register_rpc_with_owner_and_action(
        COMPANION_RECONCILE_ABILITY,
        OwnerKind::Device,
        crate::daemon::ability::descriptors::AdmissionAction::Manage,
        Arc::new(move |args| companion_reconcile(args, &companion_reconcile_runtime_manager)),
    );
}

pub fn reload_description() -> &'static str {
    "Reload installed plugin packages into the running daemon runtime after a plugin install, update, or remove transaction."
}

pub fn reload_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

pub fn status_description() -> &'static str {
    "Report plugin package load planning and actual daemon runtime registration status."
}

pub fn status_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

pub fn activate_realtime_description() -> &'static str {
    "Check the concrete daemon abilities, local resources, permissions, and publication state required to activate a plugin-declared realtime capability."
}

pub fn activate_realtime_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["package_id"],
        "properties": {
            "package_id": {
                "type": "string",
                "minLength": 1,
                "description": "Plugin package id to activate, for example easynet.remote_desktop."
            },
            "package_version": {
                "type": "string",
                "minLength": 1,
                "description": "Optional package version. When omitted, every installed version with the requested id is checked."
            }
        }
    })
}

pub fn companion_status_description() -> &'static str {
    "Report one local desktop companion plugin status through the shared companion DTO contract."
}

pub fn companion_reconcile_description() -> &'static str {
    "Reconcile one local desktop companion plugin against desired state and boot policy."
}

fn reload_plugins(
    args: Value,
    registry: &SharedPluginRegistryCell,
    plugin_runtime_manager: &PluginRuntimeManager,
) -> anyhow::Result<Value> {
    if !args.is_null() && !args.as_object().is_some_and(|object| object.is_empty()) {
        anyhow::bail!("{RELOAD_ABILITY} accepts only an empty object or null");
    }
    let catalog = registry.get().ok_or_else(|| {
        anyhow::anyhow!(
            "{RELOAD_ABILITY}: daemon plugin registry is not initialised; retry after boot ready"
        )
    })?;
    let report = plugin_runtime_manager.reload_default_plugins(catalog)?;
    Ok(json!({
        "ok": true,
        "loaded_packages": report.loaded_packages,
        "registered_abilities": report.registered_abilities,
        "unregistered_abilities": report.unregistered_abilities,
        "realtime_activation_hints": report.realtime_activation_hints,
        "realtime_activation_plans": report.realtime_activation_plans,
    }))
}

fn plugin_status(
    args: Value,
    registry: &SharedPluginRegistryCell,
    plugin_runtime_manager: &PluginRuntimeManager,
) -> anyhow::Result<Value> {
    if !args.is_null() && !args.as_object().is_some_and(|object| object.is_empty()) {
        anyhow::bail!("{STATUS_ABILITY} accepts only an empty object or null");
    }
    let catalog = registry.get().ok_or_else(|| {
        anyhow::anyhow!(
            "{STATUS_ABILITY}: daemon plugin registry is not initialised; retry after boot ready"
        )
    })?;
    let report = plugin_runtime_manager.daemon_surface_report(catalog)?;
    Ok(json!({
        "ok": true,
        "packages": report.packages,
        "abilities": report.abilities,
    }))
}

fn activate_realtime(
    args: Value,
    registry: &SharedPluginRegistryCell,
    plugin_runtime_manager: &PluginRuntimeManager,
) -> anyhow::Result<Value> {
    let request = parse_activate_realtime_args(args)?;
    let catalog = registry.get().ok_or_else(|| {
        anyhow::anyhow!(
            "{ACTIVATE_REALTIME_ABILITY}: daemon plugin registry is not initialised; retry after boot ready"
        )
    })?;
    let surface = plugin_runtime_manager.daemon_surface_report(catalog)?;
    let resources = crate::daemon::persistence::resources::load()?;
    let outcomes = PluginActivationBroker::new(&resources).realtime_outcomes(
        &surface,
        &request.package_id,
        request.package_version.as_deref(),
    );
    if outcomes.is_empty() {
        let version = request
            .package_version
            .as_deref()
            .map(|version| format!("@{version}"))
            .unwrap_or_default();
        anyhow::bail!(
            "{ACTIVATE_REALTIME_ABILITY}: no realtime capability found for {}{}",
            request.package_id,
            version
        );
    }
    serde_json::to_value(PluginRealtimeActivationReport {
        ok: true,
        package_id: request.package_id,
        package_version: request.package_version,
        outcomes,
    })
    .map_err(Into::into)
}

fn companion_status(
    args: Value,
    plugin_runtime_manager: &PluginRuntimeManager,
) -> anyhow::Result<Value> {
    let request = parse_companion_request(args, COMPANION_STATUS_ABILITY)?;
    let state = plugin_runtime_manager.state()?;
    let package = resolve_package(
        &state,
        &request.package_id,
        request.package_version.as_deref(),
        COMPANION_STATUS_ABILITY,
    )?;
    DesktopCompanionManager::current()
        .status_json(&package)
        .map_err(Into::into)
}

fn companion_reconcile(
    args: Value,
    plugin_runtime_manager: &PluginRuntimeManager,
) -> anyhow::Result<Value> {
    let request = parse_companion_request(args, COMPANION_RECONCILE_ABILITY)?;
    let state = plugin_runtime_manager.state()?;
    let package = resolve_package(
        &state,
        &request.package_id,
        request.package_version.as_deref(),
        COMPANION_RECONCILE_ABILITY,
    )?;
    DesktopCompanionManager::current()
        .reconcile(&package)
        .map_err(Into::into)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ActivateRealtimeRequest {
    package_id: String,
    #[serde(default)]
    package_version: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct CompanionRequest {
    package_id: String,
    #[serde(default)]
    package_version: Option<String>,
}

fn parse_activate_realtime_args(args: Value) -> anyhow::Result<ActivateRealtimeRequest> {
    if args.is_null() {
        anyhow::bail!("{ACTIVATE_REALTIME_ABILITY} requires a package_id");
    }
    let mut request: ActivateRealtimeRequest = serde_json::from_value(args)
        .map_err(|err| anyhow::anyhow!("{ACTIVATE_REALTIME_ABILITY} args invalid: {err}"))?;
    request.package_id = request.package_id.trim().to_string();
    if request.package_id.is_empty() {
        anyhow::bail!("{ACTIVATE_REALTIME_ABILITY} package_id must not be empty");
    }
    if let Some(version) = &mut request.package_version {
        *version = version.trim().to_string();
        if version.is_empty() {
            anyhow::bail!("{ACTIVATE_REALTIME_ABILITY} package_version must not be empty");
        }
    }
    Ok(request)
}

fn parse_companion_request(args: Value, ability: &str) -> anyhow::Result<CompanionRequest> {
    if args.is_null() {
        anyhow::bail!("{ability} requires a package_id");
    }
    let mut request: CompanionRequest = serde_json::from_value(args)
        .map_err(|err| anyhow::anyhow!("{ability} args invalid: {err}"))?;
    request.package_id = request.package_id.trim().to_string();
    if request.package_id.is_empty() {
        anyhow::bail!("{ability} package_id must not be empty");
    }
    if let Some(version) = &mut request.package_version {
        *version = version.trim().to_string();
        if version.is_empty() {
            anyhow::bail!("{ability} package_version must not be empty");
        }
    }
    Ok(request)
}

fn resolve_package(
    state: &crate::daemon::plugins::PluginRuntimeState,
    package_id: &str,
    package_version: Option<&str>,
    ability: &str,
) -> anyhow::Result<crate::daemon::plugins::package::SharedPluginPackage> {
    let matches = state
        .index()
        .packages()
        .iter()
        .filter(|package| {
            package.id().as_str() == package_id
                && package_version
                    .map(|version| package.version().as_str() == version)
                    .unwrap_or(true)
        })
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [package]
            if package.manifest().kind()
                == crate::daemon::plugins::PluginKind::DesktopCompanion =>
        {
            Ok(package.clone())
        }
        [package] => anyhow::bail!(
            "{ability}: package {}@{} is not a desktop_companion",
            package.id().as_str(),
            package.version().as_str()
        ),
        [] => anyhow::bail!("{ability}: no package found for {package_id}"),
        _ => anyhow::bail!(
            "{ability}: multiple package versions found for {package_id}; pass package_version"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::persistence::resources::ResourcesFile;
    #[cfg(feature = "remote-desktop")]
    use crate::daemon::persistence::resources::{ResourceBinding, ResourceEntry, ResourceType};
    use crate::daemon::plugins::surface::PluginKindView;
    #[cfg(feature = "remote-desktop")]
    use crate::daemon::plugins::PluginRealtimePermissionStatus;
    use crate::daemon::plugins::{
        activation_plans_for_manifest, PluginPackageManifest, PluginPackageSurfaceRecord,
        PluginRealtimeOutcomeStatus, PluginRuntimeState, PluginSurfaceReport,
    };
    use std::collections::BTreeSet;

    fn empty_plugin_runtime_manager() -> PluginRuntimeManager {
        PluginRuntimeManager::from_state(PluginRuntimeState::from_index(
            crate::daemon::plugins::PluginPackageIndex::default(),
        ))
    }

    #[test]
    fn reload_rejects_non_empty_args() {
        let cell = SharedPluginRegistryCell::new();
        let manager = empty_plugin_runtime_manager();
        let err = reload_plugins(json!({"unexpected": true}), &cell, &manager).unwrap_err();
        assert!(
            format!("{err}").contains("empty object"),
            "wrong error: {err}"
        );
    }

    #[test]
    fn reload_fails_typed_before_registry_is_initialised() {
        let cell = SharedPluginRegistryCell::new();
        let manager = empty_plugin_runtime_manager();
        let err = reload_plugins(json!({}), &cell, &manager).unwrap_err();
        assert!(
            format!("{err}").contains("not initialised"),
            "wrong error: {err}"
        );
    }

    #[test]
    fn status_rejects_non_empty_args() {
        let cell = SharedPluginRegistryCell::new();
        let manager = empty_plugin_runtime_manager();
        let err = plugin_status(json!({"unexpected": true}), &cell, &manager).unwrap_err();
        assert!(
            format!("{err}").contains("empty object"),
            "wrong error: {err}"
        );
    }

    #[test]
    fn activate_realtime_rejects_missing_package_id() {
        let err = parse_activate_realtime_args(json!({})).unwrap_err();
        assert!(
            format!("{err}").contains("package_id"),
            "wrong error: {err}"
        );
    }

    #[cfg(feature = "remote-desktop")]
    #[test]
    fn activate_realtime_projects_ready_resource_and_permission_actions() {
        let provider = crate::daemon::plugins::remote_desktop::provider();
        let manifest =
            PluginPackageManifest::parse(provider.manifest_path(), provider.manifest_body())
                .unwrap();
        let daemon_abilities: BTreeSet<String> = provider
            .ability_specs()
            .into_iter()
            .map(|spec| spec.name.to_string())
            .collect();
        let package = PluginPackageSurfaceRecord {
            package_id: provider.package_id().to_string(),
            package_version: manifest.version().to_string(),
            kind: PluginKindView::Builtin,
            planned_load_status: "loaded".to_string(),
            daemon_runtime_status: "loaded".to_string(),
            load_status: "loaded".to_string(),
            ability_count: daemon_abilities.len(),
            descriptor_published: true,
            runtime_published: true,
            invokable: true,
            companion: None,
            realtime_activation_plans: activation_plans_for_manifest(
                provider.package_id(),
                manifest.version(),
                &manifest,
                Some(&daemon_abilities),
            ),
            error: None,
        };
        let surface = PluginSurfaceReport {
            packages: vec![package],
            abilities: Vec::new(),
        };
        let resources = ResourcesFile {
            resources: vec![
                resource(ResourceType::Display, "display-1"),
                resource(ResourceType::Window, "window-1"),
                resource(ResourceType::Application, "application-1"),
            ],
        };

        let outcomes = PluginActivationBroker::new(&resources).realtime_outcomes(
            &surface,
            "easynet.remote_desktop",
            None,
        );

        assert_eq!(outcomes.len(), 1);
        let outcome = &outcomes[0];
        assert!(outcome.ready);
        assert_eq!(outcome.status, PluginRealtimeOutcomeStatus::Ready);
        assert_eq!(
            outcome.permissions.status,
            PluginRealtimePermissionStatus::StatusAbilityAvailable
        );
        assert_eq!(outcome.resources.missing, Vec::<String>::new());
    }

    #[cfg(feature = "remote-desktop")]
    fn resource(kind: ResourceType, hardware_id: &str) -> ResourceEntry {
        ResourceEntry {
            resource_ura: crate::daemon::persistence::resources::build_resource_ura(
                "acme",
                hardware_id,
            ),
            owner_agent: crate::core::ura::device_ura("acme", "dev-a"),
            kind,
            binding: ResourceBinding::LocalDevice,
            hardware_id: hardware_id.to_string(),
            display_name: hardware_id.to_string(),
            metadata: json!({}),
            first_seen_at: "2026-07-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn activate_realtime_blocks_when_required_resource_is_missing() {
        let manifest = PluginPackageManifest::parse(
            "plugins/test/plugin.toml",
            r#"
id = "easynet.camera"
schema_version = "1"
version = "0.1.0"
kind = "builtin"
entrypoint = "builtin:camera"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "camera.snapshot"
descriptor_path = "camera/snapshot.toml"
layer = "operational"
call_mode = "rpc"

[[realtime_capability]]
kind = "camera"
modes = ["snapshot"]
transport = "invoke_stream"
resources = ["camera"]
quick_add = true
"#,
        )
        .unwrap();
        let daemon_abilities = BTreeSet::from(["camera.snapshot".to_string()]);
        let package = PluginPackageSurfaceRecord {
            package_id: "easynet.camera".to_string(),
            package_version: "0.1.0".to_string(),
            kind: PluginKindView::Builtin,
            planned_load_status: "loaded".to_string(),
            daemon_runtime_status: "loaded".to_string(),
            load_status: "loaded".to_string(),
            ability_count: 1,
            descriptor_published: true,
            runtime_published: true,
            invokable: true,
            companion: None,
            realtime_activation_plans: activation_plans_for_manifest(
                "easynet.camera",
                "0.1.0",
                &manifest,
                Some(&daemon_abilities),
            ),
            error: None,
        };
        let surface = PluginSurfaceReport {
            packages: vec![package],
            abilities: Vec::new(),
        };

        let outcomes = PluginActivationBroker::new(&ResourcesFile::default()).realtime_outcomes(
            &surface,
            "easynet.camera",
            Some("0.1.0"),
        );

        assert_eq!(outcomes.len(), 1);
        assert!(!outcomes[0].ready);
        assert_eq!(outcomes[0].status, PluginRealtimeOutcomeStatus::Blocked);
        assert_eq!(outcomes[0].resources.missing, vec!["camera".to_string()]);
    }
}
