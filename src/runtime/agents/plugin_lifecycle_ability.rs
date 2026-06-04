// EasyNet CLI — Plugin Lifecycle Ability
// ======================================
//
// File: src/runtime/agents/plugin_lifecycle_ability.rs
// Description: Daemon-local plugin runtime refresh surface.

use std::sync::{Arc, OnceLock};

use serde_json::{json, Value};

use crate::runtime::ability_dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::runtime::plugin_host::PluginRuntimeManager;

/// Daemon-local ability used by `easynet plugin install/update/remove` after
/// the filesystem transaction commits.
pub const RELOAD_ABILITY: &str = "device.plugin.reload";
/// Daemon-local ability used by `easynet plugin list` to report actual runtime
/// plugin status instead of an offline load-plan approximation.
pub const STATUS_ABILITY: &str = "device.plugin.status";

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
    reg.register_rpc_with_owner(
        STATUS_ABILITY,
        OwnerKind::Device,
        Arc::new(move |args| plugin_status(args, &registry, &plugin_runtime_manager)),
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
    let rows = plugin_runtime_manager.daemon_surface_rows(catalog)?;
    Ok(json!({
        "ok": true,
        "abilities": rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_rejects_non_empty_args() {
        let cell = SharedPluginRegistryCell::new();
        let manager = PluginRuntimeManager::new();
        let err = reload_plugins(json!({"unexpected": true}), &cell, &manager).unwrap_err();
        assert!(
            format!("{err}").contains("empty object"),
            "wrong error: {err}"
        );
    }

    #[test]
    fn reload_fails_typed_before_registry_is_initialised() {
        let cell = SharedPluginRegistryCell::new();
        let manager = PluginRuntimeManager::new();
        let err = reload_plugins(json!({}), &cell, &manager).unwrap_err();
        assert!(
            format!("{err}").contains("not initialised"),
            "wrong error: {err}"
        );
    }

    #[test]
    fn status_rejects_non_empty_args() {
        let cell = SharedPluginRegistryCell::new();
        let manager = PluginRuntimeManager::new();
        let err = plugin_status(json!({"unexpected": true}), &cell, &manager).unwrap_err();
        assert!(
            format!("{err}").contains("empty object"),
            "wrong error: {err}"
        );
    }
}
