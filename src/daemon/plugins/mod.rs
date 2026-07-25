// EasyNet CLI — daemon plugin host
// =================================
//
// File: src/daemon/plugins/mod.rs
// Description: Ability extension host for installed and builtin plugins.

pub mod broker;
pub mod companion;
pub mod contribution;
pub mod descriptor;
pub mod errors;
pub mod host_api;
pub mod index;
pub mod install;
pub mod load_plan;
pub mod manifest;
pub mod package;
pub mod provider;
pub mod provider_registry;
pub mod realtime;
pub mod runtime_manager;
pub mod sidecar;
pub mod surface;
pub mod wire;

#[cfg(feature = "remote-desktop")]
#[path = "../../../plugins/remote-desktop/src/embedded.rs"]
pub mod remote_desktop;

pub use broker::{
    PluginActivationBroker, PluginPolicyBroker, PluginRealtimeActivationOutcome,
    PluginRealtimeActivationReport, PluginRealtimeOutcomeStatus, PluginRealtimePermissionReadiness,
    PluginRealtimePermissionStatus, PluginRealtimePublishReadiness, PluginRealtimeResourceMatch,
    PluginRealtimeResourceReadiness, PluginResourceBroker,
};
pub use companion::{
    current_platform as current_companion_platform, CompanionDesiredState, CompanionObservedState,
    CompanionProjectedState, CompanionSessionStatus, CompanionSupervisorState,
    DesktopCompanionManager, DesktopCompanionPlan, DesktopCompanionPlanner,
    DesktopCompanionStateStore, DesktopCompanionStatus, PlatformCompanionSpec,
};
pub use contribution::{
    DaemonPluginBinder, PluginAbilityContribution, PluginAbilityHandler, PluginContributionBuilder,
    PluginContributionSet, PluginImplementationBinding, PluginPackageContribution,
    PluginRequirementSet,
};
pub use descriptor::{PluginAbilityMetadata, PluginDescriptorProjector};
pub use errors::{PluginHostError, Result};
pub use host_api::{PluginHotReloadReport, PluginRealtimeActivationHint, PluginRuntimeHost};
pub use index::{PluginPackageIndex, PluginPackageIndexError, PluginPackageIndexLoadReport};
pub use install::{InstalledPluginRecord, PluginInstaller, PluginStateToml};
pub use load_plan::{PluginLoadPlan, PluginLoadPlanner, PluginLoadStatus};
pub use manifest::{
    PluginAbilityLayer, PluginBidiWireKind, PluginDeclarativeBinding, PluginKind,
    PluginPackageManifest, PluginRealtimeCapability, PluginRealtimeKind, PluginRealtimeMode,
    PluginRealtimeTransport, PluginRuntimeLimits,
};
pub use package::PluginAbilityDescriptor;
pub use provider::{PluginProvider, PluginProviderId, PluginProviderKind};
pub use provider_registry::PluginProviderRegistry;
pub use realtime::{
    activation_plans_for_manifest, PluginRealtimeActivationPlan, PluginRealtimeActivationStatus,
    PluginRealtimeTransportAdapterReadiness, PluginRealtimeTransportAdapterRegistry,
    PluginRealtimeTransportAdapterStatus, PluginRealtimeTransportReadiness,
    PluginRealtimeTransportReadinessStatus, PluginRealtimeTransportRoleReadiness,
    PluginRealtimeTransportRoleStatus,
};
pub use runtime_manager::{PluginRuntimeManager, PluginRuntimeState};
pub use surface::{
    PluginAbilitySurface, PluginAbilitySurfaceRecord, PluginKindView, PluginPackageSurfaceRecord,
    PluginSurfaceProjector, PluginSurfaceReport,
};
pub use wire::PluginWireRegistry;

use crate::daemon::plugins::package::{BuiltinPluginAbilitySpec, BuiltinPluginBinding};

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde_json::Value;

/// Return every builtin plugin binding compiled into this binary.
///
/// Runtime loading policy stays in the plugin host. Builtin implementation
/// packages live under their product resource owner, not under a separate
/// top-level plugin source root.
pub fn builtin_bindings() -> Vec<BuiltinPluginBinding> {
    let mut registry = PluginProviderRegistry::new();
    registry
        .register(desktop_menubar_provider())
        .expect("desktop menubar provider registration is static and unique");
    #[cfg(feature = "remote-desktop")]
    registry
        .register(crate::daemon::plugins::remote_desktop::provider())
        .expect("remote desktop provider registration is static and unique");
    registry
        .into_builtin_bindings()
        .expect("static provider binding projection must be valid")
}

struct DesktopMenubarProvider;

fn desktop_menubar_provider() -> Arc<dyn PluginProvider> {
    Arc::new(DesktopMenubarProvider)
}

impl PluginProvider for DesktopMenubarProvider {
    fn package_id(&self) -> &'static str {
        "easynet.desktop.menubar"
    }

    fn provider_kind(&self) -> PluginProviderKind {
        PluginProviderKind::DesktopCompanion
    }

    fn manifest_body(&self) -> &'static str {
        include_str!("../../../plugins/desktop-menubar/plugin.toml")
    }

    fn manifest_path(&self) -> &'static str {
        "plugins/desktop-menubar/plugin.toml"
    }

    fn expected_entrypoint(&self) -> &'static str {
        "dist/macos/EasyNetMenuBar.app"
    }

    fn installable_package_root(&self) -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            option_env!("EASYNET_DESKTOP_MENUBAR_PACKAGE_ROOT").map(PathBuf::from)
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    fn ability_specs(&self) -> Vec<BuiltinPluginAbilitySpec> {
        Vec::new()
    }

    fn contribute(&self, _: &mut PluginContributionBuilder, _: PluginRuntimeLimits) -> Result<()> {
        Ok(())
    }
}

/// Return true when an ability is exported by a plugin package loaded in this
/// daemon boot profile.
pub fn is_plugin_ability(name: &str) -> bool {
    default_loaded_package_for_ability(name).is_ok()
}

/// Resolve the canonical descriptor path for a loaded plugin-owned ability.
pub fn ability_descriptor_path(name: &str) -> Option<String> {
    try_ability_descriptor_path(name).ok().flatten()
}

pub(crate) fn try_ability_descriptor_path(name: &str) -> Result<Option<String>> {
    Ok(try_loaded_package_for_ability(name)?.and_then(|package| {
        package
            .manifest()
            .ability(name)
            .map(|ability| ability.descriptor_path().to_string())
    }))
}

/// Resolve plugin-owned human-readable ability metadata.
pub fn description_for(name: &str) -> Option<&'static str> {
    try_description_for(name).ok().flatten()
}

pub(crate) fn try_description_for(name: &str) -> Result<Option<&'static str>> {
    let Some(package) = try_loaded_package_for_ability(name)? else {
        return Ok(None);
    };
    let Some(binding) = package.builtin_binding() else {
        return Ok(None);
    };
    Ok(binding
        .ability_specs()
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| (spec.description)()))
}

/// Resolve plugin-owned human-readable ability metadata as an owned value.
pub fn description_for_owned(name: &str) -> Option<String> {
    try_description_for_owned(name).ok().flatten()
}

pub(crate) fn try_description_for_owned(name: &str) -> Result<Option<String>> {
    Ok(try_descriptor_for(name)?.map(|descriptor| descriptor.description().to_string()))
}

/// Resolve builtin plugin descriptor text without reading installed package
/// state or env-disable gates.
pub fn builtin_description_for_owned(name: &str) -> Option<String> {
    try_builtin_description_for_owned(name).ok().flatten()
}

pub(crate) fn try_builtin_description_for_owned(name: &str) -> Result<Option<String>> {
    Ok(try_builtin_descriptor_for(name)?.map(|descriptor| descriptor.description().to_string()))
}

/// Resolve plugin-owned ability input schema metadata.
pub fn input_schema_for(name: &str) -> Option<Value> {
    try_input_schema_for(name).ok().flatten()
}

pub(crate) fn try_input_schema_for(name: &str) -> Result<Option<Value>> {
    Ok(try_descriptor_for(name)?.map(|descriptor| descriptor.input_schema().clone()))
}

/// Resolve builtin plugin schema without reading installed package state or
/// env-disable gates.
pub fn builtin_input_schema_for(name: &str) -> Option<Value> {
    try_builtin_input_schema_for(name).ok().flatten()
}

pub(crate) fn try_builtin_input_schema_for(name: &str) -> Result<Option<Value>> {
    Ok(try_builtin_descriptor_for(name)?.map(|descriptor| descriptor.input_schema().clone()))
}

pub(crate) fn try_descriptor_for(name: &str) -> Result<Option<Arc<PluginAbilityDescriptor>>> {
    Ok(try_loaded_package_for_ability(name)?.and_then(|package| package.ability_descriptor(name)))
}

fn try_builtin_descriptor_for(name: &str) -> Result<Option<Arc<PluginAbilityDescriptor>>> {
    let index = PluginPackageIndex::builtin()?;
    Ok(index
        .packages()
        .iter()
        .find_map(|package| package.ability_descriptor(name)))
}

/// Return descriptor metadata for every plugin ability, independent of load
/// status.
pub fn published_plugin_abilities() -> Result<Vec<PluginAbilityMetadata>> {
    let state = default_state()?;
    PluginDescriptorProjector::project(state.index())
}

/// Resolve the product/runtime layer declared by a plugin ability.
pub fn ability_layer_for(name: &str) -> Option<PluginAbilityLayer> {
    try_ability_layer_for(name).ok().flatten()
}

pub(crate) fn try_ability_layer_for(name: &str) -> Result<Option<PluginAbilityLayer>> {
    Ok(try_loaded_package_for_ability(name)?.and_then(|package| {
        package
            .manifest()
            .ability(name)
            .map(|ability| ability.layer())
    }))
}

/// Return the plugin-declared bidi wire kind for a plugin ability.
pub fn plugin_bidi_wire_kind(name: &str) -> Option<PluginBidiWireKind> {
    try_plugin_bidi_wire_kind(name).ok().flatten()
}

pub(crate) fn try_plugin_bidi_wire_kind(name: &str) -> Result<Option<PluginBidiWireKind>> {
    Ok(try_loaded_package_for_ability(name)?.and_then(|package| {
        package
            .manifest()
            .ability(name)
            .and_then(|ability| ability.bidi_wire_kind())
    }))
}

fn default_loaded_package_for_ability(
    name: &str,
) -> Result<crate::daemon::plugins::package::SharedPluginPackage> {
    try_loaded_package_for_ability(name)?
        .ok_or_else(|| PluginHostError::MissingBuiltinBinding(format!("loaded ability {name:?}")))
}

fn try_loaded_package_for_ability(
    name: &str,
) -> Result<Option<crate::daemon::plugins::package::SharedPluginPackage>> {
    let state = default_state()?;
    let load_plan = state.load_plan();
    Ok(load_plan
        .entries()
        .iter()
        .find(|entry| entry.is_loaded() && entry.package().manifest().ability(name).is_some())
        .map(|entry| std::sync::Arc::clone(entry.package())))
}

/// Process-wide default plugin state snapshot keyed by the plugin root it was
/// loaded from.
///
/// Catalog listings resolve descriptors once per published ability; loading
/// package state from disk inside each lookup turns one listing into hundreds
/// of package re-hashes. Register/reload paths refresh this snapshot via
/// [`publish_default_state`] after they observe fresh package-store state. The
/// root key keeps processes that re-point `$HOME` (tests, tools) from reading
/// another root's snapshot.
struct DefaultStateSnapshot {
    plugin_root: PathBuf,
    state: Arc<PluginRuntimeState>,
}

static DEFAULT_STATE: RwLock<Option<DefaultStateSnapshot>> = RwLock::new(None);

/// Directory reads = snapshot reads (F-050): every catalog-shaped reader
/// inside the crate goes through here, never `PluginRuntimeState::
/// load_default()` directly — the only direct loads left are this getter's
/// miss path and the manager's register/reload writers, which re-read disk
/// on purpose and then [`publish_default_state`].
pub(crate) fn default_state() -> Result<Arc<PluginRuntimeState>> {
    let plugin_root = index::default_plugin_root();
    {
        let cached = DEFAULT_STATE.read().expect("default plugin state poisoned");
        if let Some(snapshot) = cached.as_ref() {
            if snapshot.plugin_root == plugin_root {
                return Ok(Arc::clone(&snapshot.state));
            }
        }
    }
    let state = Arc::new(PluginRuntimeState::load_default()?);
    *DEFAULT_STATE
        .write()
        .expect("default plugin state poisoned") = Some(DefaultStateSnapshot {
        plugin_root,
        state: Arc::clone(&state),
    });
    Ok(state)
}

/// Replace the default-state snapshot with state loaded from the current
/// plugin root. Called by manager register/reload so descriptor lookups
/// observe plugin install/remove/update without re-reading disk per lookup.
pub(crate) fn publish_default_state(state: &PluginRuntimeState) {
    *DEFAULT_STATE
        .write()
        .expect("default plugin state poisoned") = Some(DefaultStateSnapshot {
        plugin_root: index::default_plugin_root(),
        state: Arc::new(state.clone()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::plugins::manifest::PluginRuntimeLimits;

    #[test]
    fn plugin_host_manifest_rejects_duplicate_abilities() {
        let raw = test_manifest(
            r#"
[[ability_metadata]]
name = "test.echo"
layer = "control"

[[ability_metadata]]
name = "test.echo"
layer = "observation"
"#,
        );
        assert_eq!(
            PluginPackageManifest::parse("plugins/test/plugin.toml", &raw),
            Err(PluginHostError::DuplicateAbility("test.echo".to_string()))
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
name = "test.echo"
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
            .any(|meta| meta.name == "remote_desktop.attach"));
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

    #[test]
    fn plugin_metadata_try_helpers_distinguish_absent_plugin_from_lookup_failure() {
        let missing = "plugin.test.ability-that-is-not-loaded";

        assert!(
            try_loaded_package_for_ability(missing)
                .expect("default plugin state should load")
                .is_none(),
            "absent plugin ability is explicit None, not a lookup failure"
        );
        assert!(try_description_for(missing)
            .expect("description lookup should preserve absent plugin as None")
            .is_none());
        assert!(try_descriptor_for(missing)
            .expect("descriptor lookup should preserve absent plugin as None")
            .is_none());
        assert!(try_input_schema_for(missing)
            .expect("schema lookup should preserve absent plugin as None")
            .is_none());
        assert!(
            try_descriptor_for(missing)
                .expect("descriptor lookup should preserve absent plugin as None")
                .is_none(),
            "descriptor lookup must stay on the explicit Result<Option<_>> boundary"
        );
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
