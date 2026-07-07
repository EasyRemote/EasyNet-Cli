// EasyNet CLI — plugin provider contract
// ======================================
//
// File: src/daemon/plugins/provider.rs
// Description: Generic provider interface for plugin-owned implementation
//              projects linked into or loaded by the daemon plugin host.
//
// Protocol Responsibility:
// - None. Providers contribute daemon AbilityImpl bindings only; Axon keeps
//   Invocation, receipt, admission, stream/bidi, signing, and URA semantics.
//
// Implementation Approach:
// - Keep provider identity separate from package manifest identity.
// - Project native-static providers into the existing package binding model so
//   index/load/status behavior remains stable while ownership moves out of
//   daemon resource modules.
//
// Usage Contract:
// - Provider implementations must be package-owned.
// - Daemon code may call only this generic interface, never plugin-specific
//   handler modules.
//
// Architectural Position:
// - Daemon plugin host boundary, not product behavior.

use std::fmt;
use std::sync::Arc;

use crate::daemon::plugins::contribution::PluginContributionBuilder;
use crate::daemon::plugins::manifest::PluginRuntimeLimits;
use crate::daemon::plugins::package::BuiltinPluginAbilitySpec;
use crate::daemon::plugins::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PluginProviderId {
    pub package_id: &'static str,
    pub provider_kind: PluginProviderKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PluginProviderKind {
    NativeStatic,
    Sidecar,
    Declarative,
    DesktopCompanion,
}

pub trait PluginProvider: Send + Sync {
    fn package_id(&self) -> &'static str;
    fn provider_kind(&self) -> PluginProviderKind;
    fn manifest_body(&self) -> &'static str;
    fn manifest_path(&self) -> &'static str;
    fn expected_entrypoint(&self) -> &'static str;
    fn enabled_env_var(&self) -> Option<&'static str> {
        None
    }
    fn ability_specs(&self) -> Vec<BuiltinPluginAbilitySpec>;
    fn contribute(
        &self,
        builder: &mut PluginContributionBuilder,
        limits: PluginRuntimeLimits,
    ) -> Result<()>;
}

#[derive(Clone)]
pub struct ProviderBackedBuiltinBinding {
    provider: Arc<dyn PluginProvider>,
}

impl ProviderBackedBuiltinBinding {
    pub fn new(provider: Arc<dyn PluginProvider>) -> Self {
        Self { provider }
    }

    pub fn provider(&self) -> &Arc<dyn PluginProvider> {
        &self.provider
    }

    pub fn manifest_path(&self) -> &'static str {
        self.provider.manifest_path()
    }

    pub fn manifest_body(&self) -> &'static str {
        self.provider.manifest_body()
    }

    pub fn expected_entrypoint(&self) -> &'static str {
        self.provider.expected_entrypoint()
    }

    pub fn enabled_env_var(&self) -> Option<&'static str> {
        self.provider.enabled_env_var()
    }

    pub fn ability_specs(&self) -> Vec<BuiltinPluginAbilitySpec> {
        self.provider.ability_specs()
    }

    pub fn contribute(
        &self,
        builder: &mut PluginContributionBuilder,
        limits: PluginRuntimeLimits,
    ) -> Result<()> {
        self.provider.contribute(builder, limits)
    }
}

impl fmt::Debug for ProviderBackedBuiltinBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderBackedBuiltinBinding")
            .field("package_id", &self.provider.package_id())
            .field("provider_kind", &self.provider.provider_kind())
            .field("manifest_path", &self.provider.manifest_path())
            .field("expected_entrypoint", &self.provider.expected_entrypoint())
            .finish_non_exhaustive()
    }
}
