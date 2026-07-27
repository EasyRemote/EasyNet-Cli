// EasyNet CLI — plugin provider registry
// ======================================
//
// File: src/daemon/plugins/provider_registry.rs
// Description: Central registry for native-static plugin providers.
//
// Protocol Responsibility:
// - None. This registry resolves daemon plugin implementation providers only.
//
// Implementation Approach:
// - Store providers by package id in deterministic order.
// - Validate provider identity before projection into package bindings.
//
// Usage Contract:
// - This is the only daemon-owned list of shipped native-static providers.
// - Adding a linked provider means registering a provider export here, not
//   importing plugin-specific handler modules elsewhere.
//
// Architectural Position:
// - Generic daemon plugin host wiring.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::manifest::{validate_builtin_entrypoint, PluginPackageManifest};
use crate::daemon::plugins::package::BuiltinPluginBinding;
use crate::daemon::plugins::provider::{
    PluginProvider, PluginProviderKind, ProviderBackedBuiltinBinding,
};

#[derive(Clone, Default)]
pub struct PluginProviderRegistry {
    providers: BTreeMap<&'static str, Arc<dyn PluginProvider>>,
}

impl PluginProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, provider: Arc<dyn PluginProvider>) -> Result<()> {
        if provider.provider_kind() != PluginProviderKind::NativeStatic
            && provider.provider_kind() != PluginProviderKind::DesktopCompanion
        {
            return Err(PluginHostError::PluginProjectBoundaryViolation {
                reason: format!(
                    "provider registry accepts linked providers only, got {:?} for {}",
                    provider.provider_kind(),
                    provider.package_id()
                ),
            });
        }
        let package_id = provider.package_id();
        if let Some(existing) = self.providers.get(package_id) {
            return Err(PluginHostError::ProviderRegistryDuplicate {
                package: existing.package_id(),
            });
        }
        self.providers.insert(package_id, provider);
        Ok(())
    }

    pub fn provider_for(&self, package_id: &str) -> Option<Arc<dyn PluginProvider>> {
        self.providers.get(package_id).map(Arc::clone)
    }

    pub fn into_builtin_bindings(self) -> Result<Vec<BuiltinPluginBinding>> {
        self.providers
            .into_values()
            .map(Self::binding_from_provider)
            .collect()
    }

    fn binding_from_provider(provider: Arc<dyn PluginProvider>) -> Result<BuiltinPluginBinding> {
        if provider.package_id().trim().is_empty() {
            return Err(PluginHostError::ProviderIdMismatch {
                registry: "<empty>",
                provider: provider.package_id(),
            });
        }
        let manifest =
            PluginPackageManifest::parse(provider.manifest_path(), provider.manifest_body())?;
        if manifest.id() != provider.package_id() {
            return Err(PluginHostError::ProviderManifestMismatch {
                package: provider.package_id().to_string(),
                manifest: manifest.id().to_string(),
                provider: provider.package_id(),
            });
        }
        validate_builtin_entrypoint(&manifest, provider.expected_entrypoint())?;
        if manifest.entrypoint().contains("src/daemon/resources")
            || manifest.entrypoint().contains("daemon::resources::")
        {
            return Err(PluginHostError::PluginProjectBoundaryViolation {
                reason: format!(
                    "provider {} manifest entrypoint {:?} points at daemon resources",
                    provider.package_id(),
                    manifest.entrypoint()
                ),
            });
        }
        Ok(BuiltinPluginBinding::new(
            ProviderBackedBuiltinBinding::new(provider),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::plugins::contribution::PluginContributionBuilder;
    use crate::daemon::plugins::manifest::PluginRuntimeLimits;
    use crate::daemon::plugins::package::BuiltinPluginAbilitySpec;

    struct TestProvider {
        package_id: &'static str,
        manifest_body: &'static str,
        expected_entrypoint: &'static str,
    }

    impl PluginProvider for TestProvider {
        fn package_id(&self) -> &'static str {
            self.package_id
        }

        fn provider_kind(&self) -> PluginProviderKind {
            PluginProviderKind::NativeStatic
        }

        fn manifest_body(&self) -> &'static str {
            self.manifest_body
        }

        fn manifest_path(&self) -> &'static str {
            "plugins/test/plugin.toml"
        }

        fn expected_entrypoint(&self) -> &'static str {
            self.expected_entrypoint
        }

        fn ability_specs(&self) -> Vec<BuiltinPluginAbilitySpec> {
            Vec::new()
        }

        fn contribute(
            &self,
            _: &mut PluginContributionBuilder,
            _: PluginRuntimeLimits,
        ) -> Result<()> {
            Ok(())
        }
    }

    fn provider(package_id: &'static str, manifest_body: &'static str) -> Arc<dyn PluginProvider> {
        provider_with_entrypoint(package_id, manifest_body, "test_plugin::provider")
    }

    fn provider_with_entrypoint(
        package_id: &'static str,
        manifest_body: &'static str,
        expected_entrypoint: &'static str,
    ) -> Arc<dyn PluginProvider> {
        Arc::new(TestProvider {
            package_id,
            manifest_body,
            expected_entrypoint,
        })
    }

    fn manifest(id: &str, entrypoint: &str) -> String {
        format!(
            r#"
schema_version = "1"
id = "{id}"
version = "0.1.0"
kind = "builtin"
entrypoint = "{entrypoint}"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "test.echo"
layer = "control"
"#
        )
    }

    #[test]
    fn rejects_duplicate_provider_package_id() {
        let body = Box::leak(manifest("test.plugin", "test_plugin::provider").into_boxed_str());
        let mut registry = PluginProviderRegistry::new();
        registry
            .register(provider("test.plugin", body))
            .expect("first provider");
        let err = registry
            .register(provider("test.plugin", body))
            .expect_err("duplicate provider must fail");
        assert!(matches!(
            err,
            PluginHostError::ProviderRegistryDuplicate { .. }
        ));
    }

    #[test]
    fn rejects_provider_manifest_id_mismatch() {
        let body = Box::leak(manifest("other.plugin", "test_plugin::provider").into_boxed_str());
        let mut registry = PluginProviderRegistry::new();
        registry
            .register(provider_with_entrypoint(
                "test.plugin",
                body,
                concat!(
                    "easynet_cli::daemon::",
                    "resources::remote_",
                    "desktop::contribute"
                ),
            ))
            .expect("provider registration");
        let err = registry
            .into_builtin_bindings()
            .expect_err("manifest/provider id mismatch must fail");
        assert!(matches!(
            err,
            PluginHostError::ProviderManifestMismatch { .. }
        ));
    }

    #[test]
    fn rejects_provider_manifest_entrypoint_mismatch() {
        let body = Box::leak(manifest("test.plugin", "other_plugin::provider").into_boxed_str());
        let mut registry = PluginProviderRegistry::new();
        registry
            .register(provider_with_entrypoint(
                "test.plugin",
                body,
                concat!(
                    "easynet_cli::daemon::",
                    "resources::remote_",
                    "desktop::contribute"
                ),
            ))
            .expect("provider registration");
        let err = registry
            .into_builtin_bindings()
            .expect_err("manifest/provider entrypoint mismatch must fail");
        assert!(matches!(err, PluginHostError::EntrypointMismatch { .. }));
    }

    #[test]
    fn rejects_provider_manifest_entrypoint_into_daemon_resources() {
        let body = Box::leak(
            manifest(
                "test.plugin",
                concat!(
                    "easynet_cli::daemon::",
                    "resources::remote_",
                    "desktop::contribute"
                ),
            )
            .into_boxed_str(),
        );
        let mut registry = PluginProviderRegistry::new();
        registry
            .register(provider_with_entrypoint(
                "test.plugin",
                body,
                concat!(
                    "easynet_cli::daemon::",
                    "resources::remote_",
                    "desktop::contribute"
                ),
            ))
            .expect("provider registration");
        let err = registry
            .into_builtin_bindings()
            .expect_err("daemon resource entrypoint must fail");
        assert!(matches!(
            err,
            PluginHostError::PluginProjectBoundaryViolation { .. }
        ));
    }
}
