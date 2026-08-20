//! EasyNet browser builtin provider
//! ================================
//!
//! File: plugins/browser/src/embedded.rs
//! Description: Package-owned provider for governed `browser.*` abilities.
//!
//! Protocol Responsibility:
//! - Contributes browser AbilityImpl handlers; Axon owns Invocation, receipt,
//!   admission, stream/bidi lifecycle, signing, and URA semantics.
//!
//! Implementation Approach:
//! - Keep Chrome/CDP/session business behavior under this package.
//! - Export one provider consumed by the generic daemon plugin host.
//!
//! Usage Contract:
//! - The daemon may call only the PluginProvider surface.
//!
//! Architectural Position:
//! - Native-static EasyNet plugin package boundary.

use std::sync::Arc;

use crate::daemon::plugins::package::BuiltinPluginAbilitySpec;
use crate::daemon::plugins::{
    PluginContributionBuilder, PluginProvider, PluginProviderKind, PluginRuntimeLimits, Result,
};

pub(crate) mod cdp;
pub(crate) mod chrome;
pub(crate) mod constants;
pub(crate) mod errors;
pub(crate) mod handlers;
pub(crate) mod input;
#[cfg(test)]
pub(crate) mod performance;
pub(crate) mod registration;
pub(crate) mod runtime;
pub(crate) mod schema;
pub(crate) mod session;

const MANIFEST_PATH: &str = "plugins/browser/plugin.toml";
const MANIFEST_BODY: &str = include_str!("../plugin.toml");
const ENTRYPOINT: &str = "easynet_plugin_browser::provider";
const ENABLED_ENV_VAR: &str = "EASYNET_BROWSER_PLUGIN";

struct BrowserProvider;

pub fn provider() -> Arc<dyn PluginProvider> {
    Arc::new(BrowserProvider)
}

impl PluginProvider for BrowserProvider {
    fn package_id(&self) -> &'static str {
        "easynet.browser"
    }

    fn provider_kind(&self) -> PluginProviderKind {
        PluginProviderKind::NativeStatic
    }

    fn manifest_body(&self) -> &'static str {
        MANIFEST_BODY
    }

    fn manifest_path(&self) -> &'static str {
        MANIFEST_PATH
    }

    fn expected_entrypoint(&self) -> &'static str {
        ENTRYPOINT
    }

    fn enabled_env_var(&self) -> Option<&'static str> {
        Some(ENABLED_ENV_VAR)
    }

    fn ability_specs(&self) -> Vec<BuiltinPluginAbilitySpec> {
        registration::ability_specs()
    }

    fn contribute(
        &self,
        builder: &mut PluginContributionBuilder,
        limits: PluginRuntimeLimits,
    ) -> Result<()> {
        registration::contribute(builder, limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_index_materializes_the_browser_package_and_all_descriptors() {
        let index = crate::daemon::plugins::PluginPackageIndex::builtin()
            .expect("builtin plugin package index");
        let package = index
            .package_for_ability(constants::ABILITY_OPEN_SESSION)
            .expect("browser package");
        assert_eq!(package.id().as_str(), "easynet.browser");
        assert_eq!(package.ability_descriptors().count(), 7);
        for ability in constants::PUBLIC_ABILITIES {
            assert!(
                package.ability_descriptor(ability).is_some(),
                "missing package descriptor for {ability}"
            );
        }
    }

    #[test]
    #[ignore]
    fn regenerate_checked_in_descriptors() {
        let index = crate::daemon::plugins::PluginPackageIndex::builtin().unwrap();
        let package = index
            .package_for_ability(constants::ABILITY_OPEN_SESSION)
            .unwrap();
        let metadata = crate::daemon::plugins::PluginDescriptorProjector::project(&index).unwrap();
        for meta in metadata
            .iter()
            .filter(|meta| constants::PUBLIC_ABILITIES.contains(&meta.name.as_str()))
        {
            let contract = crate::daemon::plugins::plugin_ability_contract(meta);
            let rendered =
                crate::daemon::ability::catalog::ability_toml::render_ability_contract_toml(
                    &contract,
                );
            let path = std::path::Path::new(package.manifest().descriptor_dir())
                .join(format!("{}.ability.toml", meta.name));
            std::fs::write(&path, rendered).unwrap();
        }
    }

    #[test]
    fn checked_in_browser_descriptors_match_the_canonical_projection() {
        let index = crate::daemon::plugins::PluginPackageIndex::builtin()
            .expect("builtin plugin package index");
        let package = index
            .package_for_ability(constants::ABILITY_OPEN_SESSION)
            .expect("browser package");
        let metadata = crate::daemon::plugins::PluginDescriptorProjector::project(&index)
            .expect("plugin descriptor metadata");
        for meta in metadata
            .iter()
            .filter(|meta| constants::PUBLIC_ABILITIES.contains(&meta.name.as_str()))
        {
            let contract = crate::daemon::plugins::plugin_ability_contract(meta);
            let expected =
                crate::daemon::ability::catalog::ability_toml::render_ability_contract_toml(
                    &contract,
                );
            let path = std::path::Path::new(package.manifest().descriptor_dir())
                .join(format!("{}.ability.toml", meta.name));
            let actual = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            assert_eq!(actual, expected, "descriptor drift: {}", path.display());
        }
    }
}
