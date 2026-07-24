// EasyNet CLI — plugin contribution boundary
// ==========================================
//
// File: src/daemon/plugins/contribution.rs
// Description: Daemon-owned binding layer for plugin-provided AbilityImpls.

use std::collections::BTreeSet;

use crate::daemon::ability::dispatch::{
    AxonAbilityCatalog, ControlPlaneImplementation, LocalBidiHandlerWithEnvelope,
    LocalRpcHandlerWithEnvelope, LocalStreamHandlerWithEnvelope, OwnerKind,
};
use crate::daemon::ability::{AbilityImplSource, CallMode, RuntimeEnv};
use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::manifest::{PluginKind, PluginRealtimeCapability, PluginRuntimeLimits};

/// Resource and permission declarations supplied by a plugin package.
///
/// What this is NOT: an admission decision. The daemon policy layer decides
/// whether the declared requirements are satisfiable before/while binding or
/// invoking a contribution.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginRequirementSet {
    permissions: Vec<String>,
    resources: Vec<String>,
}

impl PluginRequirementSet {
    pub fn new(permissions: Vec<String>, resources: Vec<String>) -> Self {
        Self {
            permissions,
            resources,
        }
    }

    pub fn permissions(&self) -> &[String] {
        &self.permissions
    }

    pub fn resources(&self) -> &[String] {
        &self.resources
    }
}

/// Executable binding metadata for one plugin-provided ability.
#[derive(Clone)]
pub struct PluginImplementationBinding {
    source: AbilityImplSource,
    runtime_env: RuntimeEnv,
    handler: PluginAbilityHandler,
}

impl PluginImplementationBinding {
    pub fn new(
        source: AbilityImplSource,
        runtime_env: RuntimeEnv,
        handler: PluginAbilityHandler,
    ) -> Self {
        Self {
            source,
            runtime_env,
            handler,
        }
    }

    pub fn source(&self) -> &AbilityImplSource {
        &self.source
    }

    pub fn runtime_env(&self) -> &RuntimeEnv {
        &self.runtime_env
    }

    pub fn handler(&self) -> &PluginAbilityHandler {
        &self.handler
    }
}

/// Executable handler supplied by a plugin implementation.
#[derive(Clone)]
pub enum PluginAbilityHandler {
    Rpc(LocalRpcHandlerWithEnvelope),
    Stream(LocalStreamHandlerWithEnvelope),
    Bidi(LocalBidiHandlerWithEnvelope),
}

impl PluginAbilityHandler {
    pub const fn call_mode(&self) -> CallMode {
        match self {
            Self::Rpc(_) => CallMode::Rpc,
            Self::Stream(_) => CallMode::Stream,
            Self::Bidi(_) => CallMode::Bidi,
        }
    }
}

/// One ability implementation contributed by a plugin package.
#[derive(Clone)]
pub struct PluginAbilityContribution {
    name: String,
    call_mode: CallMode,
    manifest: crate::daemon::ability::manifest::AbilityManifest,
    implementation: PluginImplementationBinding,
}

impl PluginAbilityContribution {
    pub fn new(
        name: impl Into<String>,
        call_mode: CallMode,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        implementation: PluginImplementationBinding,
    ) -> Result<Self> {
        let name = name.into();
        let handler_mode = implementation.handler().call_mode();
        if call_mode != handler_mode {
            return Err(PluginHostError::InvalidContribution {
                package: "<unknown>".to_string(),
                ability: name,
                reason: format!(
                    "declared call mode {call_mode:?} does not match handler mode {handler_mode:?}"
                ),
            });
        }
        Ok(Self {
            name,
            call_mode,
            manifest,
            implementation,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn call_mode(&self) -> CallMode {
        self.call_mode
    }

    pub fn manifest(&self) -> &crate::daemon::ability::manifest::AbilityManifest {
        &self.manifest
    }

    pub fn implementation(&self) -> &PluginImplementationBinding {
        &self.implementation
    }
}

/// All AbilityImpls contributed by one plugin package.
#[derive(Clone)]
pub struct PluginPackageContribution {
    package_id: String,
    package_version: String,
    kind: PluginKind,
    requirements: PluginRequirementSet,
    limits: PluginRuntimeLimits,
    realtime_capabilities: Vec<PluginRealtimeCapability>,
    abilities: Vec<PluginAbilityContribution>,
}

impl PluginPackageContribution {
    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    pub fn package_version(&self) -> &str {
        &self.package_version
    }

    pub const fn kind(&self) -> PluginKind {
        self.kind
    }

    pub fn requirements(&self) -> &PluginRequirementSet {
        &self.requirements
    }

    pub const fn limits(&self) -> PluginRuntimeLimits {
        self.limits
    }

    pub fn realtime_capabilities(&self) -> &[PluginRealtimeCapability] {
        &self.realtime_capabilities
    }

    pub fn abilities(&self) -> &[PluginAbilityContribution] {
        &self.abilities
    }

    pub fn ability_names(&self) -> impl Iterator<Item = &str> {
        self.abilities.iter().map(|ability| ability.name())
    }

    pub fn package_label(&self) -> String {
        format!("{}@{}", self.package_id, self.package_version)
    }
}

/// Builder used by plugin package loaders and builtin bindings.
///
/// The builder records executable implementation bindings only. It deliberately
/// has no authority/owner setters; those are daemon policy decisions applied by
/// [`DaemonPluginBinder`].
pub struct PluginContributionBuilder {
    package_id: String,
    package_version: String,
    kind: PluginKind,
    requirements: PluginRequirementSet,
    limits: PluginRuntimeLimits,
    realtime_capabilities: Vec<PluginRealtimeCapability>,
    ability_names: BTreeSet<String>,
    abilities: Vec<PluginAbilityContribution>,
}

impl PluginContributionBuilder {
    pub fn new(
        package_id: impl Into<String>,
        package_version: impl Into<String>,
        kind: PluginKind,
        limits: PluginRuntimeLimits,
        requirements: PluginRequirementSet,
        realtime_capabilities: Vec<PluginRealtimeCapability>,
    ) -> Self {
        Self {
            package_id: package_id.into(),
            package_version: package_version.into(),
            kind,
            requirements,
            limits,
            realtime_capabilities,
            ability_names: BTreeSet::new(),
            abilities: Vec::new(),
        }
    }

    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    pub fn package_version(&self) -> &str {
        &self.package_version
    }

    pub fn plugin_runtime_env(&self) -> RuntimeEnv {
        RuntimeEnv::plugin(self.package_id(), self.package_version())
    }

    pub fn rpc(
        &mut self,
        ability: impl Into<String>,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        source: AbilityImplSource,
        runtime_env: RuntimeEnv,
        handler: LocalRpcHandlerWithEnvelope,
    ) -> Result<()> {
        self.push(
            ability,
            CallMode::Rpc,
            manifest,
            PluginImplementationBinding::new(
                source,
                runtime_env,
                PluginAbilityHandler::Rpc(handler),
            ),
        )
    }

    pub fn stream(
        &mut self,
        ability: impl Into<String>,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        source: AbilityImplSource,
        runtime_env: RuntimeEnv,
        handler: LocalStreamHandlerWithEnvelope,
    ) -> Result<()> {
        self.push(
            ability,
            CallMode::Stream,
            manifest,
            PluginImplementationBinding::new(
                source,
                runtime_env,
                PluginAbilityHandler::Stream(handler),
            ),
        )
    }

    pub fn bidi(
        &mut self,
        ability: impl Into<String>,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        source: AbilityImplSource,
        runtime_env: RuntimeEnv,
        handler: LocalBidiHandlerWithEnvelope,
    ) -> Result<()> {
        self.push(
            ability,
            CallMode::Bidi,
            manifest,
            PluginImplementationBinding::new(
                source,
                runtime_env,
                PluginAbilityHandler::Bidi(handler),
            ),
        )
    }

    fn push(
        &mut self,
        ability: impl Into<String>,
        call_mode: CallMode,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        implementation: PluginImplementationBinding,
    ) -> Result<()> {
        let ability = ability.into();
        if self.ability_names.contains(&ability) {
            return Err(PluginHostError::InvalidContribution {
                package: self.package_label(),
                ability,
                reason: "duplicate contributed ability binding".to_string(),
            });
        }
        let contribution =
            PluginAbilityContribution::new(ability.clone(), call_mode, manifest, implementation)
                .map_err(|err| match err {
                    PluginHostError::InvalidContribution {
                        package: _,
                        ability,
                        reason,
                    } => PluginHostError::InvalidContribution {
                        package: self.package_label(),
                        ability,
                        reason,
                    },
                    other => other,
                })?;
        self.ability_names.insert(ability);
        self.abilities.push(contribution);
        Ok(())
    }

    pub fn finish(self) -> Result<PluginPackageContribution> {
        if self.abilities.is_empty() {
            return Err(PluginHostError::InvalidContribution {
                package: self.package_label(),
                ability: "<package>".to_string(),
                reason: "package did not contribute any ability implementation".to_string(),
            });
        }
        Ok(PluginPackageContribution {
            package_id: self.package_id,
            package_version: self.package_version,
            kind: self.kind,
            requirements: self.requirements,
            limits: self.limits,
            realtime_capabilities: self.realtime_capabilities,
            abilities: self.abilities,
        })
    }

    fn package_label(&self) -> String {
        format!("{}@{}", self.package_id, self.package_version)
    }
}

#[derive(Clone, Default)]
pub struct PluginContributionSet {
    packages: Vec<PluginPackageContribution>,
}

impl PluginContributionSet {
    pub fn new(packages: Vec<PluginPackageContribution>) -> Self {
        Self { packages }
    }

    pub fn is_empty(&self) -> bool {
        self.packages.is_empty()
    }

    pub fn packages(&self) -> &[PluginPackageContribution] {
        &self.packages
    }

    pub fn push(&mut self, contribution: PluginPackageContribution) {
        self.packages.push(contribution);
    }

    pub fn ability_names(&self) -> impl Iterator<Item = &str> {
        self.packages()
            .iter()
            .flat_map(|package| package.ability_names())
    }
}

enum BinderTarget<'a> {
    Static(&'a mut AxonAbilityCatalog),
    Dynamic(&'a AxonAbilityCatalog),
}

/// Daemon-owned applier for plugin contributions.
///
/// The binder is the boundary where plugin-provided implementation bindings
/// are projected into daemon authority policy and Axon runtime registration.
/// Today the policy is the existing device authority projection; keeping it
/// here prevents plugin packages from becoming authority roots.
pub struct DaemonPluginBinder<'a> {
    target: BinderTarget<'a>,
    owner_policy: OwnerKind,
}

impl<'a> DaemonPluginBinder<'a> {
    pub fn static_catalog(catalog: &'a mut AxonAbilityCatalog) -> Self {
        Self {
            target: BinderTarget::Static(catalog),
            owner_policy: OwnerKind::Device,
        }
    }

    pub fn dynamic_catalog(catalog: &'a AxonAbilityCatalog) -> Self {
        Self {
            target: BinderTarget::Dynamic(catalog),
            owner_policy: OwnerKind::Device,
        }
    }

    pub fn bind_set(&mut self, contributions: &PluginContributionSet) -> Result<Vec<String>> {
        let mut registered = Vec::new();
        for package in contributions.packages() {
            registered.extend(self.bind_package(package)?);
        }
        registered.sort();
        Ok(registered)
    }

    pub fn bind_package(&mut self, package: &PluginPackageContribution) -> Result<Vec<String>> {
        let mut registered = Vec::new();
        for ability in package.abilities() {
            self.bind_ability(ability)?;
            registered.push(ability.name().to_string());
        }
        Ok(registered)
    }

    fn bind_ability(&mut self, ability: &PluginAbilityContribution) -> Result<()> {
        let implementation = ControlPlaneImplementation::new(
            ability.implementation().source().clone(),
            ability.implementation().runtime_env().clone(),
        );
        match (&mut self.target, ability.implementation().handler()) {
            (BinderTarget::Static(catalog), PluginAbilityHandler::Rpc(handler)) => catalog
                .register_rpc_with_envelope_and_spec_and_impl(
                    ability.name().to_string(),
                    self.owner_policy.clone(),
                    ability.manifest().clone(),
                    handler.clone(),
                    implementation,
                )
                .map_err(|error| PluginHostError::ControlPlaneRegistrationFailed {
                    ability: ability.name().to_string(),
                    reason: error.to_string(),
                }),
            (BinderTarget::Static(catalog), PluginAbilityHandler::Stream(handler)) => catalog
                .register_stream_with_envelope_and_spec_and_impl(
                    ability.name().to_string(),
                    self.owner_policy.clone(),
                    ability.manifest().clone(),
                    handler.clone(),
                    implementation,
                )
                .map_err(|error| PluginHostError::ControlPlaneRegistrationFailed {
                    ability: ability.name().to_string(),
                    reason: error.to_string(),
                }),
            (BinderTarget::Static(catalog), PluginAbilityHandler::Bidi(handler)) => catalog
                .register_bidi_with_envelope_and_spec_and_impl(
                    ability.name().to_string(),
                    self.owner_policy.clone(),
                    ability.manifest().clone(),
                    handler.clone(),
                    implementation,
                )
                .map_err(|error| PluginHostError::ControlPlaneRegistrationFailed {
                    ability: ability.name().to_string(),
                    reason: error.to_string(),
                }),
            (BinderTarget::Dynamic(catalog), PluginAbilityHandler::Rpc(handler)) => catalog
                .hot_register_rpc_with_envelope_and_spec_and_impl(
                    ability.name().to_string(),
                    self.owner_policy.clone(),
                    ability.manifest().clone(),
                    handler.clone(),
                    implementation,
                )
                .map_err(|error| PluginHostError::ControlPlaneRegistrationFailed {
                    ability: ability.name().to_string(),
                    reason: error.to_string(),
                }),
            (BinderTarget::Dynamic(catalog), PluginAbilityHandler::Stream(handler)) => catalog
                .hot_register_stream_with_envelope_and_spec_and_impl(
                    ability.name().to_string(),
                    self.owner_policy.clone(),
                    ability.manifest().clone(),
                    handler.clone(),
                    implementation,
                )
                .map_err(|error| PluginHostError::ControlPlaneRegistrationFailed {
                    ability: ability.name().to_string(),
                    reason: error.to_string(),
                }),
            (BinderTarget::Dynamic(catalog), PluginAbilityHandler::Bidi(handler)) => catalog
                .hot_register_bidi_with_envelope_and_spec_and_impl(
                    ability.name().to_string(),
                    self.owner_policy.clone(),
                    ability.manifest().clone(),
                    handler.clone(),
                    implementation,
                )
                .map_err(|error| PluginHostError::ControlPlaneRegistrationFailed {
                    ability: ability.name().to_string(),
                    reason: error.to_string(),
                }),
        }
    }
}
