// EasyNet CLI — daemon plugin host
// =================================
//
// File: src/runtime/plugin_host/mod.rs
// Description: Ability extension host for installed and builtin plugins.

pub mod descriptor;
pub mod errors;
pub mod host_api;
pub mod index;
pub mod install;
pub mod load_plan;
pub mod manifest;
pub mod package;
pub mod runtime_manager;
pub mod sidecar;
pub mod surface;
pub mod wire;

pub use descriptor::{PluginAbilityMetadata, PluginDescriptorProjector};
pub use errors::{PluginHostError, Result};
pub use host_api::{PluginHotReloadReport, PluginRuntimeHost};
pub use index::{PluginPackageIndex, PluginPackageIndexError, PluginPackageIndexLoadReport};
pub use install::{InstalledPluginRecord, PluginInstaller, PluginStateToml};
pub use load_plan::{PluginLoadPlan, PluginLoadPlanner, PluginLoadStatus};
pub use manifest::{
    PluginAbilityLayer, PluginBidiWireKind, PluginCallMode, PluginDeclarativeBinding, PluginKind,
    PluginPackageManifest, PluginRuntimeLimits,
};
pub use package::PluginAbilityDescriptor;
pub use runtime_manager::{PluginRuntimeManager, PluginRuntimeState};
pub use surface::{PluginAbilitySurface, PluginAbilitySurfaceRecord, PluginSurfaceProjector};
pub use wire::PluginWireRegistry;

use std::sync::Arc;

use serde_json::Value;

/// Return true when an ability is exported by a plugin package loaded in this
/// daemon boot profile.
pub fn is_plugin_ability(name: &str) -> bool {
    default_loaded_package_for_ability(name).is_ok()
}

/// Resolve the canonical descriptor path for a loaded plugin-owned ability.
pub fn ability_descriptor_path(name: &str) -> Option<String> {
    let package = default_loaded_package_for_ability(name).ok()?;
    package
        .manifest()
        .ability(name)
        .map(|ability| ability.descriptor_path().to_string())
}

/// Resolve plugin-owned human-readable ability metadata.
pub fn description_for(name: &str) -> Option<&'static str> {
    let package = default_loaded_package_for_ability(name).ok()?;
    let binding = package.builtin_binding()?;
    (binding.ability_specs)()
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| (spec.description)())
}

/// Resolve plugin-owned human-readable ability metadata as an owned value.
pub fn description_for_owned(name: &str) -> Option<String> {
    descriptor_for(name).map(|descriptor| descriptor.description().to_string())
}

/// Resolve builtin plugin descriptor text without reading installed package
/// state or env-disable gates.
pub fn builtin_description_for_owned(name: &str) -> Option<String> {
    builtin_descriptor_for(name).map(|descriptor| descriptor.description().to_string())
}

/// Resolve plugin-owned ability input schema metadata.
pub fn input_schema_for(name: &str) -> Option<Value> {
    descriptor_for(name).map(|descriptor| descriptor.input_schema().clone())
}

/// Resolve builtin plugin schema without reading installed package state or
/// env-disable gates.
pub fn builtin_input_schema_for(name: &str) -> Option<Value> {
    builtin_descriptor_for(name).map(|descriptor| descriptor.input_schema().clone())
}

/// Resolve full plugin descriptor metadata for a loaded plugin ability.
pub fn descriptor_for(name: &str) -> Option<Arc<PluginAbilityDescriptor>> {
    let package = default_loaded_package_for_ability(name).ok()?;
    package.ability_descriptor(name)
}

fn builtin_descriptor_for(name: &str) -> Option<Arc<PluginAbilityDescriptor>> {
    let index = PluginPackageIndex::builtin().ok()?;
    index
        .packages()
        .iter()
        .find_map(|package| package.ability_descriptor(name))
}

/// Return descriptor metadata for every plugin ability, independent of load
/// status.
pub fn published_plugin_abilities() -> Result<Vec<PluginAbilityMetadata>> {
    let state = default_state()?;
    PluginDescriptorProjector::project(state.index())
}

/// Resolve the product/runtime layer declared by a plugin ability.
pub fn ability_layer_for(name: &str) -> Option<PluginAbilityLayer> {
    let package = default_loaded_package_for_ability(name).ok()?;
    package
        .manifest()
        .ability(name)
        .map(|ability| ability.layer())
}

/// Return the plugin-declared bidi wire kind for a plugin ability.
pub fn plugin_bidi_wire_kind(name: &str) -> Option<PluginBidiWireKind> {
    let package = default_loaded_package_for_ability(name).ok()?;
    package
        .manifest()
        .ability(name)
        .and_then(|ability| ability.bidi_wire_kind())
}

fn default_loaded_package_for_ability(
    name: &str,
) -> Result<crate::runtime::plugin_host::package::SharedPluginPackage> {
    let state = default_state()?;
    let load_plan = state.load_plan();
    load_plan
        .entries()
        .iter()
        .find(|entry| entry.is_loaded() && entry.package().manifest().ability(name).is_some())
        .map(|entry| std::sync::Arc::clone(entry.package()))
        .ok_or_else(|| PluginHostError::MissingBuiltinBinding(format!("loaded ability {name:?}")))
}

fn default_state() -> Result<DefaultPluginHostState> {
    PluginRuntimeState::load_default()
}

type DefaultPluginHostState = PluginRuntimeState;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::plugin_host::manifest::PluginRuntimeLimits;

    #[test]
    fn plugin_host_manifest_rejects_duplicate_abilities() {
        let raw = test_manifest(
            r#"
[[ability_metadata]]
name = "device.test.echo"
layer = "control"

[[ability_metadata]]
name = "device.test.echo"
layer = "observation"
"#,
        );
        assert_eq!(
            PluginPackageManifest::parse("plugins/test/plugin.toml", &raw),
            Err(PluginHostError::DuplicateAbility(
                "device.test.echo".to_string()
            ))
        );
    }

    #[test]
    fn plugin_host_manifest_rejects_path_shaped_ability_names() {
        let raw = test_manifest(
            r#"
[[ability_metadata]]
name = "../device.test.escape"
layer = "control"
"#,
        );
        assert_eq!(
            PluginPackageManifest::parse("plugins/test/plugin.toml", &raw),
            Err(PluginHostError::InvalidAbilityName(
                "../device.test.escape".to_string()
            ))
        );
    }

    #[test]
    fn plugin_host_manifest_rejects_zero_runtime_limits() {
        let raw = r#"
schema_version = "1"
id = "test.plugin"
version = "0.0.1"
kind = "builtin"
entrypoint = "test::register"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = ["macos"]

[limits]
max_sessions = 0
max_frame_queue = 1

[[ability_metadata]]
name = "device.test.echo"
layer = "control"
"#;
        assert_eq!(
            PluginPackageManifest::parse("plugins/test/plugin.toml", raw),
            Err(PluginHostError::InvalidRuntimeLimit("max_sessions"))
        );
    }

    #[test]
    #[cfg(feature = "remote-desktop")]
    fn plugin_host_descriptor_projection_ignores_env_disabled() {
        let _guard = EnvGuard::set("EASYNET_REMOTE_DESKTOP_PLUGIN", "off");
        let index = PluginPackageIndex::builtin().expect("builtin index loads");
        let projected = PluginDescriptorProjector::project(&index).expect("descriptors project");
        assert!(projected
            .iter()
            .any(|meta| meta.name == "device.remote_desktop.attach"));
        let load_plan = PluginLoadPlanner::current().plan(&index);
        assert!(load_plan
            .entries()
            .iter()
            .any(|entry| matches!(entry.status(), PluginLoadStatus::DisabledByEnv { .. })));
    }

    #[test]
    fn plugin_host_runtime_limits_constructor_pins_fields() {
        let limits = PluginRuntimeLimits::new(7, 3);
        assert_eq!(limits.max_sessions(), 7);
        assert_eq!(limits.max_frame_queue(), 3);
    }

    fn test_manifest(metadata: &str) -> String {
        format!(
            r#"
schema_version = "1"
id = "test.plugin"
version = "0.0.1"
kind = "builtin"
entrypoint = "test::register"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = ["macos"]

[limits]
max_sessions = 1
max_frame_queue = 1
{metadata}
"#
        )
    }

    #[cfg(feature = "remote-desktop")]
    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
    }

    #[cfg(feature = "remote-desktop")]
    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    #[cfg(feature = "remote-desktop")]
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
