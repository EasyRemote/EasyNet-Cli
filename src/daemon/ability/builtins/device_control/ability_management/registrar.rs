// EasyNet CLI — AbilityDeploymentRegistrar
// =================================================================
//
// File: src/daemon/ability/builtins/device_control/ability_management/registrar.rs
//
// The "runtime binding" leg of the `ability.deploy` transaction, and
// the boot-time replay of the durable catalog. Turns a deployed manifest whose
// implementation is hosted by this Device into a LIVE ability-management
// SystemAgent row in the Axon `LocalRuntime`, then verifies it is actually
// routable before the deploy handler may report ACTIVE.
//
//     ability.deploy = manifest materialization     (device_ops_ability)
//                    + runtime binding               (THIS FILE)
//                    + durable catalog commit        (ability_deployment_store)
//
// Pending-runtime pattern: constructed at registry-build time without a
// runtime (catalog is built before the runtime exists), boot calls
// `set_runtime` once. Mirrors `HotAgentRegistrar`.
//
// Ability deployment call-mode resolution is an explicit registrar value object, not a
// duplicated install/uninstall/replay branch. `host_stream` is transport;
// its manifest admission action selects the runtime geometry. Unsupported
// exec kinds fail closed before runtime binding.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use anyhow::Context;
use axon_sdk::invocation::{
    AbilityCallModes, AbilityDescriptor, AbilityFn, AbilityOptions, CallMode as AxonCallMode,
    LocalRuntime,
};
use serde::Serialize;

use crate::daemon::ability::builtins::agents::chat::{
    build_host_rpc_handler, build_host_stream_handler,
};
use crate::daemon::ability::builtins::device_control::ability_management::store::{
    manifest_digest, validate_ability_deployment_mutation_authority, AbilityDeploymentRecord,
    AbilityDeploymentStore,
};
use crate::daemon::ability::dispatch::{
    rpc_env_ability_with_options, stream_env_ability_with_options, AxonAbilityCatalog,
    ControlPlaneAuthorityModeTxn, ControlPlaneAuthorityRebind, ControlPlaneImplementation,
};
use crate::daemon::ability::manifest::{AbilityExec, AbilityManifest};
use crate::daemon::ability::{
    AbilityControlPlaneRecord, AbilityDescriptorKey, AbilityImplSource, AuthorityScope,
    CallMode as DescriptorCallMode, RuntimeEnv,
};
use crate::support::platform::errors::append_cleanup_error;

/// Outcome of one install attempt, mapped 1:1 to the deploy handler's
/// `state` field. `Active` requires route visibility AND the right
/// call mode (plan invariant 3); anything weaker is `Installed` (bound
/// but not provably routable) — never a false ACTIVE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallState {
    /// Bound, durably committed, and route resolver confirms the key is
    /// routable with the expected call mode.
    Active,
    /// Registered into the runtime but route visibility / mode check did
    /// not confirm — honest "not yet ACTIVE".
    Installed,
}

impl InstallState {
    #[must_use]
    pub fn as_wire(&self) -> &'static str {
        match self {
            InstallState::Active => "ACTIVE",
            InstallState::Installed => "INSTALLED",
        }
    }
}

/// One installed ability deployment, ready to register.
pub struct AbilityDeploymentInstall {
    /// Wire dispatch key (e.g. `er.generate`).
    key: String,
    namespace: String,
    ability_ura: String,
    manifest_path: String,
    manifest_bytes: Vec<u8>,
    manifest: AbilityManifest,
    mutated_by: String,
    creator_invocation_id: String,
    /// Caller-supplied install timestamp (runtime forbids ambient clock).
    installed_at_unix_ms: u64,
    /// Process-owned implementations must renew this lease to remain routed.
    binding_lease_ms: Option<u64>,
}

const MIN_BINDING_LEASE_MS: u64 = 1_000;
const MAX_BINDING_LEASE_MS: u64 = 300_000;
const BINDING_LEASE_RETRY_BASE_DELAY: Duration = Duration::from_millis(50);
const BINDING_LEASE_RETRY_MAX_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BindingLeaseGeneration(u64);

struct BindingLeaseRetryPolicy;

impl BindingLeaseRetryPolicy {
    fn delay_after_failure(failure_count: u32) -> Duration {
        let exponent = failure_count.saturating_sub(1).min(31);
        let multiplier = 1_u32 << exponent;
        BINDING_LEASE_RETRY_BASE_DELAY
            .saturating_mul(multiplier)
            .min(BINDING_LEASE_RETRY_MAX_DELAY)
    }
}

impl AbilityDeploymentInstall {
    pub fn new(
        key: impl Into<String>,
        namespace: impl Into<String>,
        ability_ura: impl Into<String>,
        manifest_path: impl Into<String>,
        manifest_bytes: Vec<u8>,
        manifest: AbilityManifest,
        installed_at_unix_ms: u64,
        mutated_by: impl Into<String>,
        creator_invocation_id: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let key = key.into();
        let namespace = namespace.into();
        let ability_ura = ability_ura.into();
        let manifest_path = manifest_path.into();
        let mutated_by = mutated_by.into();
        let creator_invocation_id = creator_invocation_id.into();
        Self::validate(
            &key,
            &namespace,
            &ability_ura,
            &manifest_path,
            &manifest_bytes,
            &manifest,
        )?;
        validate_ability_deployment_mutation_authority(&mutated_by, &creator_invocation_id)
            .context("ability.deploy mutation authority")?;
        Ok(Self {
            key,
            namespace,
            ability_ura,
            manifest_path,
            manifest_bytes,
            manifest,
            mutated_by,
            creator_invocation_id,
            installed_at_unix_ms,
            binding_lease_ms: None,
        })
    }

    pub fn with_binding_lease_ms(mut self, binding_lease_ms: Option<u64>) -> anyhow::Result<Self> {
        if let Some(duration) = binding_lease_ms {
            if !(MIN_BINDING_LEASE_MS..=MAX_BINDING_LEASE_MS).contains(&duration) {
                anyhow::bail!(
                    "ability.deploy: binding_lease_ms must be between {MIN_BINDING_LEASE_MS} and {MAX_BINDING_LEASE_MS}"
                );
            }
        }
        self.binding_lease_ms = binding_lease_ms;
        Ok(self)
    }

    fn validate(
        key: &str,
        namespace: &str,
        ability_ura: &str,
        manifest_path: &str,
        manifest_bytes: &[u8],
        manifest: &AbilityManifest,
    ) -> anyhow::Result<()> {
        AbilityDescriptorKey::default_version(key, DescriptorCallMode::Rpc).map_err(|e| {
            anyhow::anyhow!("ability.deploy: invalid public ability key {key:?}: {e}")
        })?;
        if namespace.trim().is_empty() || namespace.trim() != namespace {
            anyhow::bail!("ability.deploy: namespace must be non-empty and trimmed");
        }
        let expected_prefix = format!("{namespace}.");
        if key != manifest.name() && !key.starts_with(&expected_prefix) {
            anyhow::bail!(
                "ability.deploy: key {key:?} must be either the manifest name or start with namespace prefix {expected_prefix:?}"
            );
        }
        if key.starts_with(&expected_prefix) && &key[expected_prefix.len()..] != manifest.name() {
            anyhow::bail!(
                "ability.deploy: key {key:?} does not match manifest name {:?} under namespace {:?}",
                manifest.name(),
                namespace
            );
        }
        if manifest_path.trim().is_empty() || manifest_path.trim() != manifest_path {
            anyhow::bail!("ability.deploy: manifest_path must be non-empty and trimmed");
        }
        if manifest_bytes.is_empty() {
            anyhow::bail!("ability.deploy: manifest snapshot must not be empty");
        }
        let parsed_manifest = AbilityManifest::from_json_slice(manifest_bytes)
            .map_err(|e| anyhow::anyhow!("ability.deploy: manifest snapshot is invalid: {e}"))?;
        if &parsed_manifest != manifest {
            anyhow::bail!("ability.deploy: manifest snapshot does not match parsed manifest");
        }
        let selector = crate::core::ura::AbilitySelector::parse(ability_ura)?;
        if selector.owner_kind() != "system-agent"
            || selector.dispatch_target()
                != crate::daemon::ability::names::federation::ABILITY_MANAGEMENT_SYSTEM_AGENT_ID
        {
            anyhow::bail!(
                "ability.deploy: deployed descriptor must be owned by the ability-management SystemAgent, got owner kind {:?} and dispatch target {:?}",
                selector.owner_kind(),
                selector.dispatch_target()
            );
        }
        if selector.public_name() != key {
            anyhow::bail!(
                "ability.deploy: Ability URA public name {:?} does not match install key {:?}",
                selector.public_name(),
                key
            );
        }
        Ok(())
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn ability_ura(&self) -> &str {
        &self.ability_ura
    }

    pub fn manifest_path(&self) -> &str {
        &self.manifest_path
    }

    pub fn manifest_bytes(&self) -> &[u8] {
        &self.manifest_bytes
    }

    pub fn manifest(&self) -> &AbilityManifest {
        &self.manifest
    }

    pub fn installed_at_unix_ms(&self) -> u64 {
        self.installed_at_unix_ms
    }

    pub fn mutated_by(&self) -> &str {
        &self.mutated_by
    }

    pub fn creator_invocation_id(&self) -> &str {
        &self.creator_invocation_id
    }

    pub fn binding_lease_ms(&self) -> Option<u64> {
        self.binding_lease_ms
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityDeploymentUninstall {
    pub ability_ura: String,
    pub install_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbilityDeploymentUninstallOutcome {
    pub public_names: Vec<String>,
    pub install_ids: Vec<String>,
    pub runtime_removed: usize,
    pub control_plane_removed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbilityDeploymentUninstallStep {
    Planned,
    DurableTombstoned,
    RuntimeCleared,
    ControlPlaneCleared,
    StoreCommitted,
}

struct AbilityDeploymentUninstallTransaction {
    step: AbilityDeploymentUninstallStep,
    removed: Vec<AbilityDeploymentRecord>,
    public_names: Vec<String>,
    install_ids: Vec<String>,
    runtime_removed: usize,
    control_plane_removed: usize,
    resumed_tombstone: bool,
}

impl AbilityDeploymentUninstallTransaction {
    fn new(removed: Vec<AbilityDeploymentRecord>, resumed_tombstone: bool) -> Self {
        let mut public_names = removed
            .iter()
            .map(|record| record.public_name().to_string())
            .collect::<Vec<_>>();
        public_names.sort();
        public_names.dedup();
        let install_ids = removed
            .iter()
            .map(|record| record.install_id().to_string())
            .collect::<Vec<_>>();
        Self {
            step: AbilityDeploymentUninstallStep::Planned,
            removed,
            public_names,
            install_ids,
            runtime_removed: 0,
            control_plane_removed: 0,
            resumed_tombstone,
        }
    }

    fn advance(&mut self, step: AbilityDeploymentUninstallStep) {
        self.step = step;
    }

    fn outcome(self) -> AbilityDeploymentUninstallOutcome {
        AbilityDeploymentUninstallOutcome {
            public_names: self.public_names,
            install_ids: self.install_ids,
            runtime_removed: self.runtime_removed,
            control_plane_removed: self.control_plane_removed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeployedAbilityControlPlaneKey {
    authority_root: String,
    owner_projection: String,
    public_name: String,
    call_mode: DescriptorCallMode,
}

impl DeployedAbilityControlPlaneKey {
    fn from_install(
        install: &AbilityDeploymentInstall,
        call_mode: DescriptorCallMode,
    ) -> anyhow::Result<Self> {
        Self::from_ability_ura(install.ability_ura(), install.key(), call_mode)
    }

    fn from_record(
        record: &AbilityDeploymentRecord,
        call_mode: DescriptorCallMode,
    ) -> anyhow::Result<Self> {
        Self::from_ability_ura(record.ability_ura(), record.public_name(), call_mode)
    }

    fn from_ability_ura(
        ability_ura: &str,
        expected_public_name: &str,
        call_mode: DescriptorCallMode,
    ) -> anyhow::Result<Self> {
        let selector = crate::core::ura::AbilitySelector::parse(ability_ura)?;
        if selector.owner_kind() != "system-agent"
            || selector.dispatch_target()
                != crate::daemon::ability::names::federation::ABILITY_MANAGEMENT_SYSTEM_AGENT_ID
        {
            anyhow::bail!(
                "deployed ability control-plane key requires ability-management SystemAgent owner, got {:?} in {}",
                selector.owner_kind(),
                ability_ura
            );
        }
        if selector.public_name() != expected_public_name {
            anyhow::bail!(
                "ability deployment control-plane key public name drift: URA has {:?}, record has {:?}",
                selector.public_name(),
                expected_public_name
            );
        }
        Ok(Self {
            authority_root: selector.owner_ura().to_string(),
            owner_projection: format!("system-agent:{}", selector.dispatch_target()),
            public_name: expected_public_name.to_string(),
            call_mode,
        })
    }

    fn authority_root(&self) -> &str {
        &self.authority_root
    }

    fn public_name(&self) -> &str {
        &self.public_name
    }

    fn call_mode(&self) -> DescriptorCallMode {
        self.call_mode
    }

    fn authority_scope(&self) -> anyhow::Result<AuthorityScope> {
        AuthorityScope::new(&self.owner_projection, self.authority_root.clone()).map_err(|error| {
            anyhow::anyhow!(
                "deployed ability control-plane authority scope rejected for {}: {error}",
                self.public_name
            )
        })
    }

    fn label(&self) -> String {
        format!(
            "{}:{}:{}",
            self.authority_root,
            self.public_name,
            self.call_mode.as_str()
        )
    }
}

struct DeviceRuntimeBinding {
    runtime_key: String,
    ability_fn: AbilityFn,
    options: AbilityOptions,
    axon_call_mode: AxonCallMode,
}

impl DeviceRuntimeBinding {
    fn from_install(
        install: &AbilityDeploymentInstall,
        record: &AbilityControlPlaneRecord,
    ) -> anyhow::Result<Self> {
        Self::from_manifest(install.ability_ura(), install.manifest(), record)
    }

    fn from_record(
        row: &AbilityDeploymentRecord,
        manifest: &AbilityManifest,
        record: &AbilityControlPlaneRecord,
    ) -> anyhow::Result<Self> {
        Self::from_manifest(row.ability_ura(), manifest, record)
    }

    fn from_manifest(
        runtime_key: &str,
        manifest: &AbilityManifest,
        record: &AbilityControlPlaneRecord,
    ) -> anyhow::Result<Self> {
        let (ability_fn, options) = build_binding(manifest)?;
        let call_mode = AbilityDeploymentCallModeResolution::from_runtime_modes(options.modes)
            .descriptor_mode();
        if record.descriptor().call_mode() != call_mode {
            anyhow::bail!(
                "ability deployment runtime binding mode drift for {:?}: manifest implies {:?}, \
                 control-plane record has {:?}",
                record.ability(),
                call_mode,
                record.descriptor().call_mode()
            );
        }
        if record.descriptor().version.as_str() != manifest.descriptor_version() {
            anyhow::bail!(
                "ability deployment runtime binding version drift for {:?}: manifest has {}, \
                 control-plane record has {}",
                record.ability(),
                manifest.descriptor_version(),
                record.descriptor().version.as_str()
            );
        }
        let axon_call_mode =
            AbilityDeploymentCallModeResolution::from_descriptor_mode(call_mode).axon_mode();
        let options = options.with_mode_descriptor_proof(
            axon_call_mode,
            record.descriptor().version.as_str(),
            record.descriptor().admission_action().as_str(),
            record.descriptor().descriptor_hash_bytes(),
            record.descriptor().schema_hash_bytes(),
            record.implementation().impl_hash(),
        );
        assert_runtime_options_are_proof_bound(&options, axon_call_mode, record)?;
        Ok(Self {
            runtime_key: runtime_key.to_string(),
            ability_fn,
            options,
            axon_call_mode,
        })
    }

    fn modes(&self) -> AbilityCallModes {
        self.options.modes
    }

    fn into_parts(self) -> (String, AbilityFn, AbilityOptions) {
        (self.runtime_key, self.ability_fn, self.options)
    }
}

/// Shared registrar cells are late-wired by catalog assembly: boot attaches
/// the live `LocalRuntime` via [`AbilityDeploymentRegistrar::set_runtime`].
pub type SharedAbilityDeploymentRegistrarCell = OnceLock<Arc<AbilityDeploymentRegistrar>>;

/// Constructed pending at registry-build time; boot injects the runtime
/// via [`AbilityDeploymentRegistrar::set_runtime`]. Owns the durable store.
pub struct AbilityDeploymentRegistrar {
    runtime: OnceLock<Arc<LocalRuntime>>,
    control_plane_catalog: OnceLock<Weak<AxonAbilityCatalog>>,
    store: AbilityDeploymentStore,
    lifecycle: tokio::sync::Mutex<()>,
    active_leases: Mutex<HashMap<String, BindingLeaseGeneration>>,
    next_lease_generation: AtomicU64,
    #[cfg(test)]
    fail_next_runtime_replace: AtomicBool,
}

impl AbilityDeploymentRegistrar {
    pub fn try_new_pending() -> anyhow::Result<Arc<Self>> {
        Ok(Arc::new(Self {
            runtime: OnceLock::new(),
            control_plane_catalog: OnceLock::new(),
            store: AbilityDeploymentStore::try_open_default()
                .context("open canonical ability deployment store")?,
            lifecycle: tokio::sync::Mutex::new(()),
            active_leases: Mutex::new(HashMap::new()),
            next_lease_generation: AtomicU64::new(1),
            #[cfg(test)]
            fail_next_runtime_replace: AtomicBool::new(false),
        }))
    }

    /// Test seam: explicit store path.
    #[must_use]
    pub fn new_pending_with_store(store: AbilityDeploymentStore) -> Arc<Self> {
        Arc::new(Self {
            runtime: OnceLock::new(),
            control_plane_catalog: OnceLock::new(),
            store,
            lifecycle: tokio::sync::Mutex::new(()),
            active_leases: Mutex::new(HashMap::new()),
            next_lease_generation: AtomicU64::new(1),
            #[cfg(test)]
            fail_next_runtime_replace: AtomicBool::new(false),
        })
    }

    #[cfg(test)]
    fn fail_next_runtime_replace_for_test(&self) {
        self.fail_next_runtime_replace.store(true, Ordering::SeqCst);
    }

    #[cfg(test)]
    fn injected_runtime_replace_fault(&self) -> Option<anyhow::Error> {
        self.fail_next_runtime_replace
            .swap(false, Ordering::SeqCst)
            .then(|| anyhow::anyhow!("injected runtime replace failure"))
    }

    #[cfg(not(test))]
    fn injected_runtime_replace_fault(&self) -> Option<anyhow::Error> {
        None
    }

    /// Attach the live runtime exactly once. A second attachment is a boot
    /// wiring bug, not an idempotent no-op.
    pub fn set_runtime(&self, runtime: Arc<LocalRuntime>) -> anyhow::Result<()> {
        self.runtime
            .set(runtime)
            .map_err(|_| anyhow::anyhow!("ability deployment registrar: runtime already wired"))
    }

    /// Attach the boot-built ability catalogue as the control-plane
    /// registry for dynamically deployed device abilities.
    ///
    /// Stored as `Weak` because the catalogue's `ability.deploy`
    /// handler closes over this registrar; keeping a strong reference
    /// here would create a permanent cycle.
    pub fn set_control_plane_catalog(
        &self,
        catalog: Weak<AxonAbilityCatalog>,
    ) -> anyhow::Result<()> {
        self.control_plane_catalog.set(catalog).map_err(|_| {
            anyhow::anyhow!("ability deployment registrar: control-plane catalog already wired")
        })
    }

    fn runtime(&self) -> anyhow::Result<&Arc<LocalRuntime>> {
        self.runtime
            .get()
            .ok_or_else(|| anyhow::anyhow!("ability deployment registrar: runtime not wired yet"))
    }

    fn control_plane_catalog(
        &self,
        operation: &'static str,
    ) -> anyhow::Result<Arc<AxonAbilityCatalog>> {
        self.control_plane_catalog
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "{operation}: control-plane catalog is not wired; refusing to mutate durable \
                     store or LocalRuntime because the control-plane leg cannot be verified"
                )
            })
    }

    async fn binding_is_current(
        &self,
        candidate: &AbilityDeploymentRecord,
        runtime: &LocalRuntime,
        catalog: &AxonAbilityCatalog,
        key: &DeployedAbilityControlPlaneKey,
        want: AbilityCallModes,
    ) -> anyhow::Result<bool> {
        let Some(installed) = self
            .store
            .load()?
            .into_iter()
            .find(|row| row.install_id() == candidate.install_id())
        else {
            return Ok(false);
        };
        if installed.binding_lease_ms() != candidate.binding_lease_ms()
            || installed.ability_ura() != candidate.ability_ura()
            || installed.manifest_hash() != candidate.manifest_hash()
        {
            return Ok(false);
        }
        let Some(control_plane_record) = catalog.control_plane_record_for_authority_mode(
            key.authority_root(),
            key.public_name(),
            key.call_mode(),
        )?
        else {
            return Ok(false);
        };
        let Some(descriptor) = runtime.ability_descriptor(candidate.ability_ura()).await else {
            return Ok(false);
        };
        Ok(runtime_descriptor_matches_bound(
            &descriptor,
            candidate.ability_ura(),
            want,
            AbilityDeploymentCallModeResolution::from_descriptor_mode(key.call_mode()).axon_mode(),
            &control_plane_record,
        ))
    }

    fn configure_binding_lease(self: &Arc<Self>, install_id: &str, binding_lease_ms: Option<u64>) {
        let Some(binding_lease_ms) = binding_lease_ms else {
            self.active_leases
                .lock()
                .expect("ability deployment lease mutex poisoned")
                .remove(install_id);
            return;
        };
        let generation =
            BindingLeaseGeneration(self.next_lease_generation.fetch_add(1, Ordering::Relaxed));
        self.active_leases
            .lock()
            .expect("ability deployment lease mutex poisoned")
            .insert(install_id.to_string(), generation);

        self.spawn_binding_lease_supervisor(
            install_id.to_string(),
            generation,
            Duration::from_millis(binding_lease_ms),
        );
    }

    fn spawn_binding_lease_supervisor(
        self: &Arc<Self>,
        install_id: String,
        generation: BindingLeaseGeneration,
        initial_delay: Duration,
    ) {
        let registrar = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut delay = initial_delay;
            let mut failure_count = 0_u32;
            loop {
                tokio::time::sleep(delay).await;
                let Some(registrar) = registrar.upgrade() else {
                    return;
                };
                match registrar
                    .expire_binding_lease(&install_id, generation)
                    .await
                {
                    Ok(()) => return,
                    Err(error) => {
                        if !registrar.binding_lease_generation_is_current(&install_id, generation) {
                            return;
                        }
                        failure_count = failure_count.saturating_add(1);
                        delay = BindingLeaseRetryPolicy::delay_after_failure(failure_count);
                        eprintln!(
                            "[ability-deployment] failed to expire implementation binding \
                             {install_id} generation {} (attempt {failure_count}); retrying in \
                             {}ms: {error:#}",
                            generation.0,
                            delay.as_millis()
                        );
                    }
                }
            }
        });
    }

    fn binding_lease_generation_is_current(
        &self,
        install_id: &str,
        generation: BindingLeaseGeneration,
    ) -> bool {
        self.active_leases
            .lock()
            .expect("ability deployment lease mutex poisoned")
            .get(install_id)
            .copied()
            == Some(generation)
    }

    fn complete_binding_lease_generation(
        &self,
        install_id: &str,
        generation: BindingLeaseGeneration,
    ) {
        let mut leases = self
            .active_leases
            .lock()
            .expect("ability deployment lease mutex poisoned");
        if leases.get(install_id).copied() == Some(generation) {
            leases.remove(install_id);
        }
    }

    fn cancel_binding_leases(&self, install_ids: &[String]) {
        let mut leases = self
            .active_leases
            .lock()
            .expect("ability deployment lease mutex poisoned");
        for install_id in install_ids {
            leases.remove(install_id);
        }
    }

    async fn expire_binding_lease(
        &self,
        install_id: &str,
        generation: BindingLeaseGeneration,
    ) -> anyhow::Result<()> {
        let _lifecycle = self.lifecycle.lock().await;
        if !self.binding_lease_generation_is_current(install_id, generation) {
            return Ok(());
        }

        let Some(row) = self
            .store
            .load()?
            .into_iter()
            .find(|row| row.install_id() == install_id)
        else {
            self.complete_binding_lease_generation(install_id, generation);
            return Ok(());
        };
        if row.binding_lease_ms().is_none() {
            self.complete_binding_lease_generation(install_id, generation);
            return Ok(());
        }

        let runtime = self.runtime()?.clone();
        let catalog = self.control_plane_catalog("ability deployment binding lease expiry")?;
        let control_plane_keys = Self::control_plane_removal_keys(std::slice::from_ref(&row))?;
        let affected_owners = control_plane_keys
            .iter()
            .map(|key| key.authority_root().to_string())
            .collect::<Vec<_>>();
        catalog.prepare_dynamic_publication(&affected_owners)?;
        if !self.binding_lease_generation_is_current(install_id, generation) {
            return Ok(());
        }
        let _ = runtime.unregister_ability(row.ability_ura()).await;
        if runtime.has_ability(row.ability_ura()).await {
            anyhow::bail!(
                "ability deployment binding lease expiry left runtime route {} advertised",
                row.ability_ura()
            );
        }
        for key in &control_plane_keys {
            catalog.remove_control_plane_record_for_authority_mode(
                key.authority_root(),
                key.public_name(),
                key.call_mode(),
            );
            if catalog
                .control_plane_record_for_authority_mode(
                    key.authority_root(),
                    key.public_name(),
                    key.call_mode(),
                )?
                .is_some()
            {
                anyhow::bail!(
                    "ability deployment binding lease expiry left control-plane record {} advertised",
                    key.label()
                );
            }
        }
        self.complete_binding_lease_generation(install_id, generation);
        catalog.notify_dynamic_publication_committed();
        Ok(())
    }

    /// The full deploy transaction (plan §1e), in order:
    ///   build handler → fsync store → replace_ability → route+mode check.
    ///
    /// The durable row is written before replacing a live binding because
    /// Axon's `replace_ability` intentionally returns only previous options,
    /// not the old handler closure. If we replaced first and the store write
    /// failed, we could only unregister the name and would destroy an existing
    /// same-name ability. Writing the durable intent first preserves the old
    /// live binding on commit failure; if runtime replacement fails, the new
    /// store row is removed.
    pub async fn install(
        self: &Arc<Self>,
        install: AbilityDeploymentInstall,
    ) -> anyhow::Result<InstallState> {
        let _lifecycle = self.lifecycle.lock().await;
        let runtime = self.runtime()?.clone();
        let catalog = self.control_plane_catalog("ability.deploy")?;
        let key = install.key().to_string();
        if catalog.has_static_ability(&key) {
            anyhow::bail!(
                "ability.deploy: refusing to shadow boot-time ability {key:?}; \
                 choose a distinct ability deployment name"
            );
        }

        let mode_resolution =
            AbilityDeploymentCallModeResolution::from_manifest(install.manifest())?;
        let call_mode = mode_resolution.descriptor_mode();
        let want = mode_resolution.ability_modes();
        let control_plane_key = DeployedAbilityControlPlaneKey::from_install(&install, call_mode)?;

        // ── durable installing intent (hidden from boot replay) ─────
        let record = AbilityDeploymentRecord::new_installing_with_manifest_bytes(
            key.clone(),
            install.namespace().to_string(),
            install.ability_ura().to_string(),
            install.manifest_path().to_string(),
            install.manifest_bytes(),
            install.installed_at_unix_ms(),
            install.mutated_by().to_string(),
            install.creator_invocation_id().to_string(),
        )
        .with_binding_lease_ms(install.binding_lease_ms());

        if self
            .binding_is_current(&record, &runtime, &catalog, &control_plane_key, want)
            .await?
        {
            self.configure_binding_lease(record.install_id(), install.binding_lease_ms());
            return Ok(InstallState::Active);
        }
        catalog.prepare_dynamic_publication(&[control_plane_key.authority_root().to_string()])?;
        let overwritten =
            self.store
                .stage_install_record(record.clone())
                .map_err(|commit_err| {
                    anyhow::anyhow!(
                    "ability.deploy: durable install staging failed before binding: {commit_err}"
                )
                })?;

        // ── control-plane materialization ───────────────────────────
        // The returned record is the only authority for runtime proof
        // binding. Runtime rows must never fabricate descriptor version
        // or hashes from defaults after this point.
        let mut control_plane_txn =
            Self::begin_control_plane_transaction(&catalog, &control_plane_key);
        let control_plane_record = match Self::rebind_control_plane_record(
            &catalog,
            &control_plane_key,
            install.manifest(),
            record.manifest_hash(),
        ) {
            Ok(record) => record,
            Err(e) => {
                let control_plane_restore = control_plane_txn.rollback();
                let rollback = self
                    .store
                    .rollback_install(record.install_id(), overwritten);
                return Err(append_cleanup_error(
                    append_cleanup_error(
                        anyhow::anyhow!("ability.deploy: control-plane rebind({key}) failed: {e}"),
                        control_plane_restore,
                        "restore prior control-plane records",
                    ),
                    rollback,
                    "restore durable ability deployment store",
                ));
            }
        };

        // ── proof-bound runtime binding ─────────────────────────────
        let binding = match DeviceRuntimeBinding::from_install(&install, &control_plane_record) {
            Ok(binding) => binding,
            Err(e) => {
                let overwritten_for_restore = overwritten.clone();
                let rollback = self
                    .store
                    .rollback_install(record.install_id(), overwritten);
                let live_restore = self.restore_live_records(&overwritten_for_restore).await;
                let control_plane_restore = control_plane_txn.rollback();
                return Err(append_cleanup_error(
                    append_cleanup_error(
                        anyhow::anyhow!(
                            "ability.deploy: proof-bound runtime binding({key}) failed: {e}"
                        ),
                        rollback.and(live_restore.map(|_| ())),
                        "restore durable ability deployment store and prior live runtime binding",
                    ),
                    control_plane_restore,
                    "restore prior control-plane records",
                ));
            }
        };
        debug_assert_eq!(binding.modes(), want);
        let axon_call_mode = binding.axon_call_mode;
        let (runtime_key, ability_fn, options) = binding.into_parts();

        let replace_result = match self.injected_runtime_replace_fault() {
            Some(err) => Err(err),
            None => runtime
                .replace_ability(runtime_key.clone(), ability_fn, options)
                .await
                .map(|_| ())
                .map_err(|err| anyhow::anyhow!("{err}")),
        };
        if let Err(e) = replace_result {
            let overwritten_for_restore = overwritten.clone();
            let rollback = self
                .store
                .rollback_install(record.install_id(), overwritten);
            let live_restore = self.restore_live_records(&overwritten_for_restore).await;
            let control_plane_restore = control_plane_txn.rollback();
            return Err(append_cleanup_error(
                append_cleanup_error(
                    anyhow::anyhow!(
                        "ability.deploy: replace_ability({runtime_key}) for {key} failed: {e}"
                    ),
                    rollback.and(live_restore.map(|_| ())),
                    "restore durable ability deployment store and prior live runtime binding",
                ),
                control_plane_restore,
                "restore prior control-plane records",
            ));
        }

        // ── route + proof visibility (invariant 3, the danger point) ─
        // ACTIVE iff the runtime descriptor can see exactly this key,
        // supports the intended call mode, and carries the same proof facts
        // as the control-plane record. `has_ability` alone is NOT enough.
        let state = match runtime.ability_descriptor(&runtime_key).await {
            Some(desc)
                if runtime_descriptor_matches_bound(
                    &desc,
                    &runtime_key,
                    want,
                    axon_call_mode,
                    &control_plane_record,
                ) =>
            {
                InstallState::Active
            }
            _ => InstallState::Installed,
        };
        if let Err(e) = self.store.commit_installed(record.install_id()) {
            let _ = runtime.unregister_ability(&runtime_key).await;
            let overwritten_for_restore = overwritten.clone();
            let rollback = self
                .store
                .rollback_install(record.install_id(), overwritten);
            let live_restore = self.restore_live_records(&overwritten_for_restore).await;
            let control_plane_restore = control_plane_txn.rollback();
            return Err(append_cleanup_error(
                append_cleanup_error(
                    anyhow::anyhow!("ability.deploy: commit installed({key}) failed: {e}"),
                    rollback.and(live_restore.map(|_| ())),
                    "restore durable ability deployment store and prior live runtime binding",
                ),
                control_plane_restore,
                "restore prior control-plane records",
            ));
        }
        control_plane_txn.commit();
        catalog.notify_dynamic_publication_committed();
        self.cancel_binding_leases(
            &overwritten
                .iter()
                .map(|row| row.install_id().to_string())
                .collect::<Vec<_>>(),
        );
        self.configure_binding_lease(record.install_id(), install.binding_lease_ms());
        Ok(state)
    }

    /// Remove a deployed ability deployment from durable store, live
    /// LocalRuntime, and the daemon control-plane registry. This is the
    /// inverse of `install`; `ability.uninstall` must never report
    /// REMOVED while any of those three legs still advertises the
    /// binding.
    pub async fn uninstall(
        self: &Arc<Self>,
        uninstall: AbilityDeploymentUninstall,
    ) -> anyhow::Result<AbilityDeploymentUninstallOutcome> {
        let _lifecycle = self.lifecycle.lock().await;
        let runtime = self.runtime()?.clone();
        let catalog = self.control_plane_catalog("ability.uninstall")?;
        let removal_plan = self
            .store
            .stage_remove_by_ability(&uninstall.ability_ura, uninstall.install_id.as_deref())?;
        if removal_plan.is_empty() {
            match uninstall.install_id.as_deref() {
                Some(id) => anyhow::bail!(
                    "ability.uninstall: no installed ability deployment matched ability_ura {:?} \
                     and install_id {:?}",
                    uninstall.ability_ura,
                    id
                ),
                None => anyhow::bail!(
                    "ability.uninstall: ability_ura {:?} is not installed through ability.deploy",
                    uninstall.ability_ura
                ),
            }
        }
        let resumed_tombstone = removal_plan.resumed();
        let mut transaction = AbilityDeploymentUninstallTransaction::new(
            removal_plan.into_records(),
            resumed_tombstone,
        );
        transaction.advance(AbilityDeploymentUninstallStep::DurableTombstoned);
        let control_plane_removals = match Self::control_plane_removal_keys(&transaction.removed) {
            Ok(keys) => keys,
            Err(err) => {
                if let Err(restore_err) = self.store.restore_records(transaction.removed.clone()) {
                    anyhow::bail!(
                        "ability.uninstall: failed to resolve control-plane modes: {err}; \
                             additionally failed to restore durable store rows: {restore_err}"
                    );
                }
                anyhow::bail!(
                        "ability.uninstall: failed to resolve control-plane modes: {err}; durable store rows restored"
                    );
            }
        };
        let affected_owners = control_plane_removals
            .iter()
            .map(|key| key.authority_root().to_string())
            .collect::<Vec<_>>();
        if let Err(error) = catalog.prepare_dynamic_publication(&affected_owners) {
            self.store.restore_records(transaction.removed.clone())?;
            return Err(error.context(
                "ability.uninstall: durable publication fence failed before live mutation",
            ));
        }
        let mut runtime_still_advertised = Vec::new();
        let mut control_plane_still_advertised = Vec::new();
        let mut runtime_keys = transaction
            .removed
            .iter()
            .map(|record| record.ability_ura().to_string())
            .collect::<Vec<_>>();
        runtime_keys.sort();
        runtime_keys.dedup();
        for runtime_key in &runtime_keys {
            if runtime.unregister_ability(runtime_key).await.is_some() {
                transaction.runtime_removed += 1;
            }
            if runtime.has_ability(runtime_key).await {
                runtime_still_advertised.push(runtime_key.clone());
            }
        }
        transaction.advance(AbilityDeploymentUninstallStep::RuntimeCleared);

        for key in &control_plane_removals {
            if catalog.remove_control_plane_record_for_authority_mode(
                key.authority_root(),
                key.public_name(),
                key.call_mode(),
            ) {
                transaction.control_plane_removed += 1;
            }
            if catalog
                .control_plane_record_for_authority_mode(
                    key.authority_root(),
                    key.public_name(),
                    key.call_mode(),
                )?
                .is_some()
            {
                control_plane_still_advertised.push(key.label());
            }
        }
        transaction.advance(AbilityDeploymentUninstallStep::ControlPlaneCleared);

        if !runtime_still_advertised.is_empty() || !control_plane_still_advertised.is_empty() {
            let live_restore = self.restore_live_records(&transaction.removed).await;
            if let Err(restore_err) = self.store.restore_records(transaction.removed.clone()) {
                anyhow::bail!(
                    "ability.uninstall: removal stopped at {:?}; runtime still advertised {:?}, \
                     control-plane still advertised {:?}; additionally failed to restore durable \
                     store rows: {restore_err}; live restore result: {:?}",
                    transaction.step,
                    runtime_still_advertised,
                    control_plane_still_advertised,
                    live_restore
                );
            }
            if let Err(restore_err) = live_restore {
                anyhow::bail!(
                    "ability.uninstall: removal stopped at {:?}; runtime still advertised {:?}, \
                     control-plane still advertised {:?}; durable store rows restored, but failed \
                     to restore live runtime/control-plane rows: {restore_err}",
                    transaction.step,
                    runtime_still_advertised,
                    control_plane_still_advertised
                );
            }
            anyhow::bail!(
                "ability.uninstall: removal stopped at {:?}; runtime still advertised {:?}, \
                 control-plane still advertised {:?}; durable store and live rows restored",
                transaction.step,
                runtime_still_advertised,
                control_plane_still_advertised
            );
        }

        if let Err(commit_err) = self.store.commit_removed(&transaction.install_ids) {
            anyhow::bail!(
                "ability.uninstall: removal stopped at {:?}; runtime and control-plane are \
                 cleared, durable tombstones remain for crash-safe boot replay suppression: \
                 {commit_err}; retrying ability.uninstall will resume the same staged removal \
                 (resumed_tombstone={})",
                transaction.step,
                transaction.resumed_tombstone
            );
        }
        transaction.advance(AbilityDeploymentUninstallStep::StoreCommitted);
        self.cancel_binding_leases(&transaction.install_ids);
        catalog.notify_dynamic_publication_committed();

        Ok(transaction.outcome())
    }

    async fn restore_live_records(
        &self,
        records: &[AbilityDeploymentRecord],
    ) -> anyhow::Result<usize> {
        let runtime = self.runtime()?.clone();
        let catalog = self.control_plane_catalog("ability deployment restore")?;
        let mut restored = 0;
        for row in records {
            let bytes = row.manifest_bytes().map_err(|e| {
                anyhow::anyhow!("restore {}: read manifest: {e}", row.public_name())
            })?;
            let digest = manifest_digest(&bytes);
            if digest != row.manifest_hash() {
                anyhow::bail!(
                    "restore {}: manifest hash drifted (store {}, disk {})",
                    row.public_name(),
                    row.manifest_hash(),
                    digest
                );
            }
            let manifest = AbilityManifest::from_json_slice(&bytes).map_err(|e| {
                anyhow::anyhow!("restore {}: parse manifest: {e}", row.public_name())
            })?;
            let control_plane_key = DeployedAbilityControlPlaneKey::from_record(
                row,
                AbilityDeploymentCallModeResolution::from_manifest(&manifest)?.descriptor_mode(),
            )?;
            let control_plane_txn =
                Self::begin_control_plane_transaction(&catalog, &control_plane_key);
            let control_plane_record = Self::rebind_control_plane_record(
                &catalog,
                &control_plane_key,
                &manifest,
                row.manifest_hash(),
            )?;
            let binding = DeviceRuntimeBinding::from_record(row, &manifest, &control_plane_record)
                .map_err(|e| {
                    anyhow::anyhow!("restore {}: proof-bound binding: {e}", row.public_name())
                })?;
            let (runtime_key, ability_fn, options) = binding.into_parts();
            runtime
                .replace_ability(runtime_key, ability_fn, options)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "restore {}: replace runtime ability: {e}",
                        row.public_name()
                    )
                })?;
            control_plane_txn.commit();
            restored += 1;
        }
        Ok(restored)
    }

    fn control_plane_removal_keys(
        records: &[AbilityDeploymentRecord],
    ) -> anyhow::Result<Vec<DeployedAbilityControlPlaneKey>> {
        let mut keys = Vec::new();
        for row in records {
            let bytes = row.manifest_bytes().map_err(|e| {
                anyhow::anyhow!(
                    "resolve control-plane mode for {}: read manifest: {e}",
                    row.public_name()
                )
            })?;
            let manifest = AbilityManifest::from_json_slice(&bytes).map_err(|e| {
                anyhow::anyhow!(
                    "resolve control-plane mode for {}: parse manifest: {e}",
                    row.public_name()
                )
            })?;
            let key = DeployedAbilityControlPlaneKey::from_record(
                row,
                AbilityDeploymentCallModeResolution::from_manifest(&manifest)?.descriptor_mode(),
            )?;
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        Ok(keys)
    }

    /// Boot replay (invariant 7): recover uncommitted install intents,
    /// then re-register every non-leased committed row into the live runtime
    /// from its embedded manifest snapshot. Leased process bindings remain
    /// inactive until the owning host renews through `ability.deploy`.
    /// A mismatch or a register failure is reported as a `stale`/`error`
    /// row rather than silently skipped.
    pub async fn replay_from_store(&self) -> ReplayReport {
        let runtime = match self.runtime() {
            Ok(rt) => rt.clone(),
            Err(_) => {
                return ReplayReport {
                    runtime_not_ready: true,
                    ..ReplayReport::default()
                };
            }
        };
        let recovered_installing = match self.store.recover_installing() {
            Ok(count) => count,
            Err(_) => {
                return ReplayReport {
                    store_unreadable: true,
                    ..ReplayReport::default()
                };
            }
        };
        let rows = match self.store.load() {
            Ok(r) => r,
            Err(_) => {
                return ReplayReport {
                    store_unreadable: true,
                    ..ReplayReport::default()
                };
            }
        };
        if rows.is_empty() {
            return ReplayReport {
                recovered_installing,
                ..ReplayReport::default()
            };
        }
        let catalog = match self.control_plane_catalog("ability deployment replay") {
            Ok(catalog) => catalog,
            Err(err) => {
                let mut report = ReplayReport {
                    recovered_installing,
                    ..ReplayReport::default()
                };
                for row in rows {
                    report.push_errored(&row, format!("control-plane catalog unavailable: {err}"));
                }
                return report;
            }
        };
        let hosted_device_authority_root = match catalog.hosted_device_authority_root() {
            Some(root) => root.to_string(),
            None => {
                let mut report = ReplayReport {
                    recovered_installing,
                    ..ReplayReport::default()
                };
                for row in rows {
                    report.push_errored(
                        &row,
                        "control-plane catalog has no hosted device authority root",
                    );
                }
                return report;
            }
        };
        let quarantined = match self
            .store
            .quarantine_unhosted_device_authority(&hosted_device_authority_root)
        {
            Ok(rows) => rows,
            Err(_) => {
                return ReplayReport {
                    recovered_installing,
                    store_unreadable: true,
                    ..ReplayReport::default()
                };
            }
        };
        let rows = match self.store.load() {
            Ok(r) => r,
            Err(_) => {
                return ReplayReport {
                    recovered_installing,
                    store_unreadable: true,
                    ..ReplayReport::default()
                };
            }
        };

        let mut report = ReplayReport {
            recovered_installing,
            ..ReplayReport::default()
        };
        for row in quarantined {
            let owner = crate::core::ura::AbilitySelector::parse(row.ability_ura())
                .map(|selector| selector.owner_ura().to_string())
                .unwrap_or_else(|_| "<invalid ability ura>".to_string());
            report.push_quarantined(
                &row,
                format!(
                    "ability deployment owner {owner} is not hosted by current daemon authority {hosted_device_authority_root}; row hidden from boot replay"
                ),
            );
        }
        for row in rows {
            if row.binding_lease_ms().is_some() {
                report.push_lease_pending(
                    &row,
                    "leased implementation is inactive after daemon boot until its host renews",
                );
                continue;
            }
            // Re-read embedded manifest material and verify hash. A corrupt
            // snapshot must NOT be registered under the recorded binding.
            let bytes = match row.manifest_bytes() {
                Ok(b) => b,
                Err(err) => {
                    report.push_stale(&row, format!("manifest material unavailable: {err}"));
                    continue;
                }
            };
            let actual_hash = manifest_digest(&bytes);
            if actual_hash != row.manifest_hash() {
                report.push_stale(
                    &row,
                    format!(
                        "manifest hash drift: expected {}, got {}",
                        row.manifest_hash(),
                        actual_hash
                    ),
                );
                continue;
            }
            let manifest = match AbilityManifest::from_json_slice(&bytes) {
                Ok(m) => m,
                Err(err) => {
                    report.push_errored(&row, format!("parse embedded manifest: {err}"));
                    continue;
                }
            };
            let descriptor_call_mode =
                match AbilityDeploymentCallModeResolution::from_manifest(&manifest) {
                    Ok(resolution) => resolution.descriptor_mode(),
                    Err(err) => {
                        report.push_errored(&row, format!("resolve descriptor call mode: {err}"));
                        continue;
                    }
                };
            let control_plane_key =
                match DeployedAbilityControlPlaneKey::from_record(&row, descriptor_call_mode) {
                    Ok(key) => key,
                    Err(err) => {
                        report.push_errored(&row, format!("derive control-plane key: {err}"));
                        continue;
                    }
                };
            let mut control_plane_txn =
                Self::begin_control_plane_transaction(&catalog, &control_plane_key);
            let control_plane_record = match Self::rebind_control_plane_record(
                &catalog,
                &control_plane_key,
                &manifest,
                row.manifest_hash(),
            ) {
                Ok(record) => record,
                Err(err) => {
                    let rollback = control_plane_txn.rollback();
                    report.push_errored(
                        &row,
                        append_cleanup_error(
                            anyhow::anyhow!("rebind control-plane descriptor: {err}"),
                            rollback,
                            "restore prior control-plane records",
                        )
                        .to_string(),
                    );
                    continue;
                }
            };
            let binding =
                match DeviceRuntimeBinding::from_record(&row, &manifest, &control_plane_record) {
                    Ok(binding) => binding,
                    Err(err) => {
                        let _ = runtime.unregister_ability(row.ability_ura()).await;
                        let rollback = control_plane_txn.rollback();
                        report.push_errored(
                            &row,
                            append_cleanup_error(
                                anyhow::anyhow!("build proof-bound runtime binding: {err}"),
                                rollback,
                                "restore prior control-plane records",
                            )
                            .to_string(),
                        );
                        continue;
                    }
                };
            let (runtime_key, ability_fn, options) = binding.into_parts();
            match runtime
                .replace_ability(runtime_key, ability_fn, options)
                .await
            {
                Ok(_) => {
                    control_plane_txn.commit();
                    report.push_registered(&row);
                }
                Err(err) => {
                    let _ = runtime.unregister_ability(row.ability_ura()).await;
                    let rollback = control_plane_txn.rollback();
                    report.push_errored(
                        &row,
                        append_cleanup_error(
                            anyhow::anyhow!("replace runtime ability: {err}"),
                            rollback,
                            "restore prior control-plane records",
                        )
                        .to_string(),
                    );
                }
            }
        }
        report
    }

    fn begin_control_plane_transaction<'a>(
        catalog: &'a AxonAbilityCatalog,
        key: &DeployedAbilityControlPlaneKey,
    ) -> ControlPlaneAuthorityModeTxn<'a> {
        catalog.begin_control_plane_authority_mode_transaction(
            key.authority_root(),
            key.public_name(),
            key.call_mode(),
        )
    }

    fn rebind_control_plane_record(
        catalog: &AxonAbilityCatalog,
        key: &DeployedAbilityControlPlaneKey,
        manifest: &AbilityManifest,
        impl_content_hash: &str,
    ) -> anyhow::Result<AbilityControlPlaneRecord> {
        catalog.rebind_control_plane_record_with_authority_scope(ControlPlaneAuthorityRebind {
            ability: key.public_name(),
            authority_scope: key.authority_scope()?,
            manifest: Some(manifest),
            call_mode: key.call_mode(),
            implementation: ControlPlaneImplementation::new(
                AbilityImplSource::DeviceDeploy,
                RuntimeEnv::ability_deployment(deployed_exec_kind(manifest)),
            )
            .with_content_hash(impl_content_hash.to_string()),
        })
    }
}

/// Boot replay outcome (observability; never silently swallows).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReplayReport {
    /// Uncommitted installing intents removed before replay. Installed rows
    /// remain the replay authority until `commit_installed` succeeds.
    pub recovered_installing: usize,
    pub registered: usize,
    /// Durable descriptors whose process-owned implementation must renew
    /// before the daemon publishes a callable route.
    pub lease_pending: usize,
    /// Manifest gone or hash drifted — row not registered.
    pub stale: usize,
    /// Valid rows owned by a previous device authority — hidden from replay.
    pub quarantined: usize,
    /// Manifest unparseable or binding build/register failed.
    pub errored: usize,
    pub runtime_not_ready: bool,
    pub store_unreadable: bool,
    pub outcomes: Vec<ReplayOutcome>,
}

impl ReplayReport {
    fn push_registered(&mut self, row: &AbilityDeploymentRecord) {
        self.registered += 1;
        self.outcomes.push(ReplayOutcome::new(
            row,
            ReplayOutcomeStatus::Registered,
            "registered into LocalRuntime and control-plane catalog",
        ));
    }

    fn push_stale(&mut self, row: &AbilityDeploymentRecord, detail: impl Into<String>) {
        self.stale += 1;
        self.outcomes
            .push(ReplayOutcome::new(row, ReplayOutcomeStatus::Stale, detail));
    }

    fn push_lease_pending(&mut self, row: &AbilityDeploymentRecord, detail: impl Into<String>) {
        self.lease_pending += 1;
        self.outcomes.push(ReplayOutcome::new(
            row,
            ReplayOutcomeStatus::LeasePending,
            detail,
        ));
    }

    fn push_quarantined(&mut self, row: &AbilityDeploymentRecord, detail: impl Into<String>) {
        self.quarantined += 1;
        self.outcomes.push(ReplayOutcome::new(
            row,
            ReplayOutcomeStatus::Quarantined,
            detail,
        ));
    }

    fn push_errored(&mut self, row: &AbilityDeploymentRecord, detail: impl Into<String>) {
        self.errored += 1;
        self.outcomes.push(ReplayOutcome::new(
            row,
            ReplayOutcomeStatus::Errored,
            detail,
        ));
    }

    #[must_use]
    pub fn outcomes_json(&self) -> String {
        serde_json::to_string(&self.outcomes)
            .unwrap_or_else(|err| format!(r#"[{{"status":"errored","detail":"{err}"}}]"#))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplayOutcome {
    pub public_name: String,
    pub ability_ura: String,
    pub install_id: String,
    pub status: ReplayOutcomeStatus,
    pub detail: String,
}

impl ReplayOutcome {
    fn new(
        row: &AbilityDeploymentRecord,
        status: ReplayOutcomeStatus,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            public_name: row.public_name().to_string(),
            ability_ura: row.ability_ura().to_string(),
            install_id: row.install_id().to_string(),
            status,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplayOutcomeStatus {
    Registered,
    LeasePending,
    Stale,
    Quarantined,
    Errored,
}

/// Build the `(AbilityFn, AbilityOptions)` for a ability deployment from its
/// manifest's exec kind. host_stream is the external host transport; its
/// explicit admission action selects RPC or stream geometry. Shell exec is
/// rejected until a permission broker and receipt-audited operator
/// approval path exist; `ability.deploy` must not become an arbitrary
/// host-command surface without that broker.
/// The single deployable device exec kind, with the deployability policy and
/// its operator-facing rejection strings defined in exactly one place.
///
/// `build_binding` (handler + options) and `AbilityDeploymentCallModeResolution`
/// (call mode) both classify through here, so the "what may be deployed and why
/// not" decision — a security-relevant gate — cannot diverge between the two
/// consumers or grow two different error strings for the same condition.
enum DeployableExec<'a> {
    HostStream(&'a crate::daemon::ability::manifest::HostStreamExec),
}

impl<'a> DeployableExec<'a> {
    fn classify(manifest: &'a AbilityManifest) -> anyhow::Result<Self> {
        match manifest.exec() {
            Some(AbilityExec::HostStream(spec)) => Ok(Self::HostStream(spec)),
            Some(AbilityExec::Shell(_)) => Err(anyhow::anyhow!(
                "ability.deploy: shell exec requires a permission broker, bounded output policy, \
                 command allow-list, and receipt-audited operator approval; deploy host_stream \
                 abilities until that broker is wired"
            )),
            Some(other) => Err(anyhow::anyhow!(
                "ability.deploy: ability deployment exec kind {other:?} is not deployable \
                 (only host_stream is supported on the ability deployment path)"
            )),
            None => Err(anyhow::anyhow!(
                "ability.deploy: ability deployment manifest has no [exec] binding"
            )),
        }
    }

    fn call_mode_resolution(
        &self,
        admission_action: Option<&str>,
    ) -> AbilityDeploymentCallModeResolution {
        match self {
            Self::HostStream(_) if admission_action == Some("invoke") => {
                AbilityDeploymentCallModeResolution::Rpc
            }
            Self::HostStream(_) => AbilityDeploymentCallModeResolution::Stream,
        }
    }
}

fn build_binding(manifest: &AbilityManifest) -> anyhow::Result<(AbilityFn, AbilityOptions)> {
    match DeployableExec::classify(manifest)? {
        DeployableExec::HostStream(spec) if manifest.admission_action() == Some("invoke") => {
            let handler = build_host_rpc_handler(spec.clone());
            Ok(rpc_env_ability_with_options(handler))
        }
        DeployableExec::HostStream(spec) => {
            let handler = build_host_stream_handler(spec.clone());
            Ok(stream_env_ability_with_options(handler))
        }
    }
}

#[cfg(test)]
fn rpc_only() -> AbilityCallModes {
    AbilityCallModes {
        rpc: true,
        stream: false,
        bidi: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AbilityDeploymentCallModeResolution {
    Rpc,
    Stream,
    Bidi,
}

impl AbilityDeploymentCallModeResolution {
    fn from_manifest(manifest: &AbilityManifest) -> anyhow::Result<Self> {
        let exec = DeployableExec::classify(manifest)?;
        Ok(exec.call_mode_resolution(manifest.admission_action()))
    }

    fn from_runtime_modes(modes: AbilityCallModes) -> Self {
        if modes.bidi {
            Self::Bidi
        } else if modes.stream {
            Self::Stream
        } else {
            Self::Rpc
        }
    }

    fn from_descriptor_mode(mode: DescriptorCallMode) -> Self {
        match mode {
            DescriptorCallMode::Rpc => Self::Rpc,
            DescriptorCallMode::Stream => Self::Stream,
            DescriptorCallMode::Bidi => Self::Bidi,
        }
    }

    fn descriptor_mode(self) -> DescriptorCallMode {
        match self {
            Self::Rpc => DescriptorCallMode::Rpc,
            Self::Stream => DescriptorCallMode::Stream,
            Self::Bidi => DescriptorCallMode::Bidi,
        }
    }

    fn axon_mode(self) -> AxonCallMode {
        match self {
            Self::Rpc => AxonCallMode::Rpc,
            Self::Stream => AxonCallMode::Stream,
            Self::Bidi => AxonCallMode::Bidi,
        }
    }

    fn ability_modes(self) -> AbilityCallModes {
        AbilityCallModes {
            rpc: self == Self::Rpc,
            stream: self == Self::Stream,
            bidi: self == Self::Bidi,
        }
    }
}

fn deployed_exec_kind(manifest: &AbilityManifest) -> &'static str {
    match manifest.exec() {
        Some(AbilityExec::HostStream(_)) => "host_stream",
        Some(AbilityExec::Shell(_)) => "shell",
        Some(_) => "unsupported",
        None => "none",
    }
}

/// The only route visibility this layer can honestly prove is the
/// runtime's own descriptor for the exact dispatch key. Anything weaker
/// (for example a bare `has_ability`) can report false ACTIVE when the
/// mode set drifted; anything stronger belongs in the future external
/// route resolver, not as a stub here.
fn runtime_descriptor_matches(desc: &AbilityDescriptor, key: &str, want: AbilityCallModes) -> bool {
    desc.name == key
        && desc.options.modes.stream == want.stream
        && desc.options.modes.rpc == want.rpc
        && desc.options.modes.bidi == want.bidi
}

fn runtime_descriptor_matches_bound(
    desc: &AbilityDescriptor,
    key: &str,
    want: AbilityCallModes,
    axon_call_mode: AxonCallMode,
    record: &AbilityControlPlaneRecord,
) -> bool {
    runtime_descriptor_matches(desc, key, want)
        && assert_runtime_options_are_proof_bound(&desc.options, axon_call_mode, record).is_ok()
}

fn assert_runtime_options_are_proof_bound(
    options: &AbilityOptions,
    axon_call_mode: AxonCallMode,
    record: &AbilityControlPlaneRecord,
) -> anyhow::Result<()> {
    let proof = options.proof_for_mode(axon_call_mode).ok_or_else(|| {
        anyhow::anyhow!(
            "runtime ability {:?} has no descriptor proof for {:?}",
            record.ability(),
            record.descriptor().call_mode()
        )
    })?;
    if !proof.is_bound() {
        anyhow::bail!(
            "runtime ability {:?} has an incomplete descriptor proof for {:?}",
            record.ability(),
            record.descriptor().call_mode()
        );
    }
    if proof.descriptor_version != record.descriptor().version.as_str() {
        anyhow::bail!(
            "runtime ability {:?} descriptor version mismatch: runtime {}, control-plane {}",
            record.ability(),
            proof.descriptor_version,
            record.descriptor().version.as_str()
        );
    }
    if proof.schema_hash != record.descriptor().schema_hash_bytes() {
        anyhow::bail!(
            "runtime ability {:?} schema hash does not match control-plane descriptor",
            record.ability()
        );
    }
    if proof.impl_hash != record.implementation().impl_hash() {
        anyhow::bail!(
            "runtime ability {:?} impl hash does not match control-plane implementation",
            record.ability()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::dispatch::rpc_handler_to_ability_fn;

    const TEST_DEVICE_URA: &str = "easynet:///r/localhost/device/d1";

    #[test]
    fn install_state_wire_strings() {
        assert_eq!(InstallState::Active.as_wire(), "ACTIVE");
        assert_eq!(InstallState::Installed.as_wire(), "INSTALLED");
    }

    #[test]
    fn registrar_rejects_duplicate_runtime_wiring() {
        let registrar = pending_registrar_for_test();
        let first = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let second = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );

        registrar.set_runtime(first).unwrap();
        let err = registrar.set_runtime(second).unwrap_err();

        assert!(err.to_string().contains("runtime already wired"), "{err}");
    }

    #[test]
    fn registrar_rejects_duplicate_control_plane_wiring() {
        let registrar = pending_registrar_for_test();
        let rt = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let first = registrar_test_catalog(Arc::clone(&rt));
        let second = registrar_test_catalog(rt);

        registrar
            .set_control_plane_catalog(Arc::downgrade(&first))
            .unwrap();
        let err = registrar
            .set_control_plane_catalog(Arc::downgrade(&second))
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("control-plane catalog already wired"),
            "{err}"
        );
    }

    use crate::daemon::ability::manifest::{AbilityExec, HostStreamExec, ShellExec};

    fn host_stream_manifest(socket: &str, function: &str) -> AbilityManifest {
        host_stream_manifest_with_action(socket, function, "stream")
    }

    fn host_stream_manifest_with_action(
        socket: &str,
        function: &str,
        action: &str,
    ) -> AbilityManifest {
        AbilityManifest::new(
            "generate",
            "stream gen",
            serde_json::json!({"type": "object"}),
        )
        .unwrap()
        .with_admission_action(action)
        .unwrap()
        .with_exec(AbilityExec::HostStream(HostStreamExec {
            host_socket: socket.to_string(),
            function: function.to_string(),
        }))
        .unwrap()
    }

    fn host_stream_install(store_dir: &std::path::Path, socket: &str) -> AbilityDeploymentInstall {
        host_stream_install_with_version(store_dir, socket, None)
    }

    fn host_stream_install_with_version(
        store_dir: &std::path::Path,
        socket: &str,
        descriptor_version: Option<&str>,
    ) -> AbilityDeploymentInstall {
        let mut manifest = host_stream_manifest(socket, "er.generate");
        if let Some(descriptor_version) = descriptor_version {
            manifest = manifest
                .with_descriptor_version(descriptor_version)
                .expect("descriptor version must validate");
        }
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let manifest_path = store_dir.join("ability.json");
        std::fs::write(&manifest_path, &manifest_bytes).unwrap();
        AbilityDeploymentInstall::new(
            "er.generate",
            "er",
            &deployed_ability_management_ability_ura("er.generate"),
            manifest_path.to_string_lossy().into_owned(),
            manifest_bytes,
            manifest,
            1,
            "easynet:///r/localhost/user/test-user",
            "test-deploy-invocation",
        )
        .unwrap()
    }

    fn deployed_ability_management_owner_ura() -> String {
        crate::core::ura::device_agent_ura(
            "localhost",
            "d1",
            crate::daemon::ability::names::federation::ABILITY_MANAGEMENT_SYSTEM_AGENT_ID,
        )
    }

    fn deployed_ability_management_ability_ura(public_name: &str) -> String {
        crate::core::ura::owner_ability_ura(&deployed_ability_management_owner_ura(), public_name)
            .expect("ability-management SystemAgent ability URA")
    }

    fn er_generate_runtime_key() -> &'static str {
        "easynet:///r/localhost/ability/system-agent.d1.ability-management.er.generate"
    }

    fn wired_registrar(
        store: AbilityDeploymentStore,
    ) -> (
        Arc<AbilityDeploymentRegistrar>,
        Arc<LocalRuntime>,
        Arc<AxonAbilityCatalog>,
    ) {
        let registrar = AbilityDeploymentRegistrar::new_pending_with_store(store);
        let rt = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let catalog = registrar_test_catalog(Arc::clone(&rt));
        registrar.set_runtime(Arc::clone(&rt)).unwrap();
        registrar
            .set_control_plane_catalog(Arc::downgrade(&catalog))
            .unwrap();
        (registrar, rt, catalog)
    }

    fn registrar_test_catalog(runtime: Arc<LocalRuntime>) -> Arc<AxonAbilityCatalog> {
        Arc::new(AxonAbilityCatalog::new_test_runtime_for_device_authority(
            runtime,
            TEST_DEVICE_URA,
        ))
    }

    fn pending_registrar_for_test() -> Arc<AbilityDeploymentRegistrar> {
        let directory =
            tempfile::tempdir().expect("create ability deployment registrar test store");
        let store =
            AbilityDeploymentStore::open_at(directory.path().join("ability-deployments.json"));
        std::mem::forget(directory);
        AbilityDeploymentRegistrar::new_pending_with_store(store)
    }

    fn stream_control_plane_record(catalog: &AxonAbilityCatalog) -> AbilityControlPlaneRecord {
        catalog
            .control_plane_record_for_mode("er.generate", DescriptorCallMode::Stream)
            .expect("ability deployment control-plane lookup is unambiguous")
            .expect("ability deployment control-plane record")
    }

    #[test]
    fn ability_deployment_call_mode_resolution_maps_manifest_geometry() {
        let manifest = host_stream_manifest("/tmp/er-host.sock", "er.generate");
        let resolution = AbilityDeploymentCallModeResolution::from_manifest(&manifest)
            .expect("host_stream mode");

        assert_eq!(resolution, AbilityDeploymentCallModeResolution::Stream);
        assert_eq!(resolution.descriptor_mode(), DescriptorCallMode::Stream);
        assert_eq!(resolution.axon_mode(), AxonCallMode::Stream);

        let unary =
            host_stream_manifest_with_action("/tmp/er-host.sock", "er.ai_inference", "invoke");
        let resolution = AbilityDeploymentCallModeResolution::from_manifest(&unary)
            .expect("host_stream RPC mode");
        assert_eq!(resolution, AbilityDeploymentCallModeResolution::Rpc);
        assert_eq!(resolution.descriptor_mode(), DescriptorCallMode::Rpc);
        assert_eq!(resolution.axon_mode(), AxonCallMode::Rpc);

        let (_, options) = build_binding(&unary).expect("host_stream RPC binding");
        assert_eq!(options.modes, rpc_only());
    }

    #[test]
    fn ability_deployment_call_mode_resolution_projects_runtime_modes() {
        assert_eq!(
            AbilityDeploymentCallModeResolution::from_runtime_modes(rpc_only()).descriptor_mode(),
            DescriptorCallMode::Rpc
        );
        assert_eq!(
            AbilityDeploymentCallModeResolution::from_runtime_modes(
                AbilityOptions::streaming().modes
            )
            .descriptor_mode(),
            DescriptorCallMode::Stream
        );
        assert_eq!(
            AbilityDeploymentCallModeResolution::from_runtime_modes(AbilityCallModes {
                rpc: false,
                stream: true,
                bidi: true,
            })
            .descriptor_mode(),
            DescriptorCallMode::Bidi
        );
    }

    #[test]
    fn ability_deployment_call_mode_resolution_projects_descriptor_modes_to_axon() {
        assert_eq!(
            AbilityDeploymentCallModeResolution::from_descriptor_mode(DescriptorCallMode::Rpc)
                .axon_mode(),
            AxonCallMode::Rpc
        );
        assert_eq!(
            AbilityDeploymentCallModeResolution::from_descriptor_mode(DescriptorCallMode::Stream)
                .axon_mode(),
            AxonCallMode::Stream
        );
        assert_eq!(
            AbilityDeploymentCallModeResolution::from_descriptor_mode(DescriptorCallMode::Bidi)
                .axon_mode(),
            AxonCallMode::Bidi
        );
    }

    // ── Negative test matrix (plan §"必测四个失败态") ──────────────

    #[test]
    fn install_rejects_direct_device_owned_dynamic_ability_ura() {
        let dir = tempfile::tempdir().unwrap();
        let install = host_stream_install(dir.path(), "/tmp/er-host.sock");
        let result = AbilityDeploymentInstall::new(
            install.key(),
            install.namespace(),
            "easynet:///r/localhost/ability/device.d1.er.generate",
            install.manifest_path(),
            install.manifest_bytes().to_vec(),
            install.manifest().clone(),
            install.installed_at_unix_ms(),
            install.mutated_by(),
            install.creator_invocation_id(),
        );

        let err = match result {
            Ok(_) => panic!("dynamic deploy must not accept direct Device owner URA"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("ability-management SystemAgent"),
            "{err}"
        );
    }

    /// Failure 1 (invariant 1e): durable commit fails before binding,
    /// so the existing live binding is preserved instead of being
    /// destroyed by an impossible closure rollback.
    #[tokio::test]
    async fn commit_failure_preserves_existing_binding() {
        let dir = tempfile::tempdir().unwrap();
        // Point the store at a path whose parent is a FILE, so write_all
        // (create_dir_all on a file) fails → commit fails.
        let bogus_parent = dir.path().join("not-a-dir");
        std::fs::write(&bogus_parent, b"x").unwrap();
        let store = AbilityDeploymentStore::open_at(bogus_parent.join("ability-deployments.json"));
        let registrar = AbilityDeploymentRegistrar::new_pending_with_store(store);
        let rt = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let catalog = registrar_test_catalog(Arc::clone(&rt));
        registrar.set_runtime(Arc::clone(&rt)).unwrap();
        registrar
            .set_control_plane_catalog(Arc::downgrade(&catalog))
            .unwrap();
        rt.replace_ability(
            er_generate_runtime_key().to_string(),
            rpc_handler_to_ability_fn(Arc::new(|_| Ok(serde_json::json!({"old": true})))),
            AbilityOptions::default().with_modes(rpc_only()),
        )
        .await
        .unwrap();

        let install = host_stream_install(dir.path(), "/tmp/er-host.sock");
        let result = registrar.install(install).await;

        assert!(result.is_err(), "commit failure must surface as error");
        assert!(
            rt.has_ability(er_generate_runtime_key()).await,
            "existing binding must survive when the durable commit fails"
        );
        let desc = rt
            .ability_descriptor(er_generate_runtime_key())
            .await
            .unwrap();
        assert!(desc.options.modes.rpc);
        assert!(!desc.options.modes.stream);
    }

    #[tokio::test]
    async fn runtime_replace_failure_removes_new_store_row() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("ability-deployments.json");
        let store = AbilityDeploymentStore::open_at(store_path.clone());
        let registrar = AbilityDeploymentRegistrar::new_pending_with_store(store);
        let rt = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let catalog = registrar_test_catalog(Arc::clone(&rt));
        registrar.set_runtime(Arc::clone(&rt)).unwrap();
        registrar
            .set_control_plane_catalog(Arc::downgrade(&catalog))
            .unwrap();

        let install = host_stream_install(dir.path(), "/tmp/er-host.sock");
        let result = AbilityDeploymentInstall::new(
            "",
            install.namespace(),
            install.ability_ura(),
            install.manifest_path(),
            install.manifest_bytes().to_vec(),
            install.manifest().clone(),
            install.installed_at_unix_ms(),
            install.mutated_by(),
            install.creator_invocation_id(),
        );

        assert!(
            result.is_err(),
            "invalid runtime key must fail construction"
        );
        let rows = AbilityDeploymentStore::open_at(store_path).load().unwrap();
        assert!(
            rows.is_empty(),
            "invalid install construction must not create a durable row"
        );
    }

    #[test]
    fn runtime_descriptor_match_requires_exact_name_and_modes() {
        let desc = AbilityDescriptor {
            name: "er.generate".to_string(),
            options: AbilityOptions::streaming(),
            registered_at_unix_ms: 0,
            active_invocations: 0,
        };
        assert!(runtime_descriptor_matches(
            &desc,
            "er.generate",
            AbilityOptions::streaming().modes
        ));
        assert!(!runtime_descriptor_matches(
            &desc,
            "er.other",
            AbilityOptions::streaming().modes
        ));
        assert!(!runtime_descriptor_matches(
            &desc,
            "er.generate",
            AbilityOptions::default().with_modes(rpc_only()).modes
        ));
    }

    /// Failure 3 (invariant 3, the danger point): a stream ability binds
    /// stream-mode and the route+mode check passes → ACTIVE. The honest
    /// state must come from the descriptor's modes, not a bare has_ability.
    #[tokio::test]
    async fn stream_ability_active_only_when_route_and_mode_match() {
        let dir = tempfile::tempdir().unwrap();
        let store = AbilityDeploymentStore::open_at(dir.path().join("ability-deployments.json"));
        let (registrar, rt, _catalog) = wired_registrar(store);

        let install = host_stream_install(dir.path(), "/tmp/er-host.sock");
        let state = registrar.install(install).await.unwrap();

        assert_eq!(state, InstallState::Active);
        let desc = rt
            .ability_descriptor(er_generate_runtime_key())
            .await
            .unwrap();
        assert!(desc.options.modes.stream, "must be bound stream-mode");
        assert!(!desc.options.modes.rpc, "stream ability is not rpc");
        let proof = desc.options.proof_for_mode(AxonCallMode::Stream);
        assert!(
            proof.is_some_and(|binding| binding.is_bound()),
            "ACTIVE requires descriptor proof, not just runtime visibility"
        );
    }

    #[tokio::test]
    async fn install_rebinds_ability_deployment_control_plane_record() {
        let dir = tempfile::tempdir().unwrap();
        let store = AbilityDeploymentStore::open_at(dir.path().join("ability-deployments.json"));
        let (registrar, _, catalog) = wired_registrar(store);

        let state = registrar
            .install(host_stream_install(dir.path(), "/tmp/er-host.sock"))
            .await
            .unwrap();

        assert_eq!(state, InstallState::Active);
        let record = catalog
            .control_plane_record_for_mode("er.generate", DescriptorCallMode::Stream)
            .expect("ability deployment control-plane lookup is unambiguous")
            .expect("ability deployment control-plane record");
        assert_eq!(
            record.authority().scope().owner_projection(),
            "system-agent:ability-management"
        );
        assert!(record.authority().predicate().governs_advertise());
        assert!(record.authority().predicate().governs_invoke());
        assert_eq!(record.descriptor().call_mode(), DescriptorCallMode::Stream);
        assert_eq!(
            *record.implementation().source(),
            AbilityImplSource::DeviceDeploy
        );
        assert!(
            record
                .implementation()
                .runtime_env()
                .label()
                .contains("ability-deployment:host_stream"),
            "runtime env should pin the deployed exec kind"
        );
        let content_hash = crate::daemon::ability::builtins::device_control::ability_management::store::manifest_digest(
            &std::fs::read(dir.path().join("ability.json")).unwrap(),
        );
        assert_eq!(
            record.implementation().content_hash(),
            Some(content_hash.as_str())
        );
        let facts = catalog
            .runtime_binding_facts_for_mode("er.generate", DescriptorCallMode::Stream)
            .expect("runtime binding lookup is unambiguous")
            .expect("runtime binding facts");
        assert_eq!(facts.implementation_source, "device_deploy");
        assert_eq!(
            facts.implementation_content_hash.as_deref(),
            Some(content_hash.as_str())
        );
    }

    #[tokio::test]
    async fn install_and_uninstall_notify_dynamic_publication_hooks_after_commit() {
        let dir = tempfile::tempdir().unwrap();
        let store = AbilityDeploymentStore::open_at(dir.path().join("ability-deployments.json"));
        let (registrar, _, catalog) = wired_registrar(store);
        let notifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        catalog
            .register_dynamic_publication_participant(
                Arc::new(|_| Ok(())),
                Arc::new({
                    let notifications = Arc::clone(&notifications);
                    move || {
                        notifications.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                }),
            )
            .unwrap();

        let install = host_stream_install(dir.path(), "/tmp/er-host.sock");
        let ability_ura = install.ability_ura().to_string();
        let state = registrar.install(install).await.unwrap();

        assert_eq!(state, InstallState::Active);
        assert_eq!(
            notifications.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "ability.deploy must publish a fresh owner projection only after commit"
        );

        registrar
            .uninstall(AbilityDeploymentUninstall {
                ability_ura,
                install_id: None,
            })
            .await
            .unwrap();

        assert_eq!(
            notifications.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "ability.uninstall must publish the tombstoned owner projection after commit"
        );
    }

    #[tokio::test]
    async fn install_binds_runtime_proof_to_manifest_descriptor_version() {
        let dir = tempfile::tempdir().unwrap();
        let store = AbilityDeploymentStore::open_at(dir.path().join("ability-deployments.json"));
        let (registrar, rt, catalog) = wired_registrar(store);

        let state = registrar
            .install(host_stream_install_with_version(
                dir.path(),
                "/tmp/er-host.sock",
                Some("2.3.0"),
            ))
            .await
            .unwrap();

        assert_eq!(state, InstallState::Active);
        let runtime_desc = rt
            .ability_descriptor(er_generate_runtime_key())
            .await
            .expect("runtime descriptor");
        let control_plane_record = catalog
            .control_plane_record_for_version_mode(
                "er.generate",
                "2.3.0",
                DescriptorCallMode::Stream,
            )
            .expect("ability deployment control-plane lookup is unambiguous")
            .expect("ability deployment control-plane record");
        let proof = runtime_desc
            .options
            .proof_for_mode(AxonCallMode::Stream)
            .expect("active stream ability carries descriptor proof");
        assert_eq!(proof.descriptor_version, "2.3.0");
        assert_eq!(
            proof.schema_hash,
            control_plane_record.descriptor().schema_hash_bytes()
        );
        assert_eq!(
            proof.impl_hash,
            control_plane_record.implementation().impl_hash()
        );
    }

    #[tokio::test]
    async fn uninstall_removes_store_runtime_and_control_plane() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("ability-deployments.json");
        let (registrar, rt, catalog) =
            wired_registrar(AbilityDeploymentStore::open_at(store_path.clone()));

        let install = host_stream_install(dir.path(), "/tmp/er-host.sock");
        let ability_ura = install.ability_ura().to_string();
        registrar.install(install).await.unwrap();
        assert!(rt.has_ability(er_generate_runtime_key()).await);
        assert!(catalog
            .control_plane_record_for_mode("er.generate", DescriptorCallMode::Stream)
            .expect("ability deployment control-plane lookup is unambiguous")
            .is_some());
        assert_eq!(
            AbilityDeploymentStore::open_at(store_path.clone())
                .load()
                .unwrap()
                .len(),
            1
        );

        let outcome = registrar
            .uninstall(AbilityDeploymentUninstall {
                ability_ura,
                install_id: None,
            })
            .await
            .unwrap();

        assert_eq!(outcome.public_names, vec!["er.generate"]);
        assert_eq!(outcome.runtime_removed, 1);
        assert_eq!(outcome.control_plane_removed, 1);
        assert!(!rt.has_ability(er_generate_runtime_key()).await);
        assert!(catalog
            .control_plane_record_for_mode("er.generate", DescriptorCallMode::Stream)
            .expect("ability deployment control-plane lookup is unambiguous")
            .is_none());
        assert!(AbilityDeploymentStore::open_at(store_path)
            .load()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn uninstall_refuses_to_mutate_when_control_plane_catalog_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("ability-deployments.json");
        let (registrar, rt, catalog) =
            wired_registrar(AbilityDeploymentStore::open_at(store_path.clone()));

        let install = host_stream_install(dir.path(), "/tmp/er-host.sock");
        let ability_ura = install.ability_ura().to_string();
        registrar.install(install).await.unwrap();
        drop(catalog);

        let err = registrar
            .uninstall(AbilityDeploymentUninstall {
                ability_ura,
                install_id: None,
            })
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("control-plane catalog is not wired"),
            "{err}"
        );
        assert!(
            rt.has_ability(er_generate_runtime_key()).await,
            "runtime binding must remain when REMOVED cannot be verified"
        );
        assert_eq!(
            AbilityDeploymentStore::open_at(store_path)
                .load()
                .unwrap()
                .len(),
            1,
            "durable row must remain when control-plane verification is unavailable"
        );
    }

    /// Failure 4 (invariant 6): re-deploying the same manifest upserts
    /// one store row (stable install_id), not a duplicate; the runtime
    /// replace is idempotent.
    #[tokio::test]
    async fn repeat_deploy_is_idempotent_upsert() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("ability-deployments.json");
        let (registrar, rt, _catalog) =
            wired_registrar(AbilityDeploymentStore::open_at(store_path.clone()));

        registrar
            .install(host_stream_install(dir.path(), "/tmp/er-host.sock"))
            .await
            .unwrap();
        registrar
            .install(host_stream_install(dir.path(), "/tmp/er-host.sock"))
            .await
            .unwrap();

        let rows = AbilityDeploymentStore::open_at(store_path).load().unwrap();
        assert_eq!(rows.len(), 1, "same manifest must upsert, not duplicate");
        assert!(rt.has_ability(er_generate_runtime_key()).await);
    }

    #[tokio::test]
    async fn control_plane_rebind_failure_restores_previous_live_binding() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("ability-deployments.json");
        let (registrar, rt, catalog) =
            wired_registrar(AbilityDeploymentStore::open_at(store_path.clone()));

        registrar
            .install(host_stream_install(dir.path(), "/tmp/old-er-host.sock"))
            .await
            .unwrap();
        assert!(rt.has_ability(er_generate_runtime_key()).await);
        drop(catalog);

        let result = registrar
            .install(host_stream_install(dir.path(), "/tmp/new-er-host.sock"))
            .await;

        let err = result.unwrap_err();
        assert!(
            err.to_string()
                .contains("control-plane catalog is not wired"),
            "expired control-plane catalog must reject the deploy before mutation: {err}"
        );
        assert!(
            rt.has_ability(er_generate_runtime_key()).await,
            "failed redeploy must leave the previous live runtime binding untouched"
        );
        assert_eq!(
            AbilityDeploymentStore::open_at(store_path)
                .load()
                .unwrap()
                .len(),
            1,
            "durable store must roll back to the previous row"
        );
    }

    #[tokio::test]
    async fn runtime_replace_failure_restores_prior_control_plane_record() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("ability-deployments.json");
        let (registrar, rt, catalog) =
            wired_registrar(AbilityDeploymentStore::open_at(store_path.clone()));

        registrar
            .install(host_stream_install_with_version(
                dir.path(),
                "/tmp/old-er-host.sock",
                Some("1.0.0"),
            ))
            .await
            .unwrap();
        let old_record = stream_control_plane_record(&catalog);
        let old_descriptor_version = old_record.descriptor().version.clone();
        let old_impl_hash = old_record.implementation().impl_hash();

        registrar.fail_next_runtime_replace_for_test();
        let err = registrar
            .install(host_stream_install_with_version(
                dir.path(),
                "/tmp/new-er-host.sock",
                Some("2.0.0"),
            ))
            .await
            .expect_err("injected runtime replace failure must abort deploy");
        assert!(
            err.to_string().contains("injected runtime replace failure"),
            "{err}"
        );

        let restored_record = stream_control_plane_record(&catalog);
        assert_eq!(
            restored_record.descriptor().version.as_str(),
            old_descriptor_version,
            "failed redeploy must restore the previous descriptor version"
        );
        assert_eq!(
            restored_record.implementation().impl_hash(),
            old_impl_hash,
            "failed redeploy must restore the previous implementation proof"
        );
        assert!(rt.has_ability(er_generate_runtime_key()).await);
        let rows = AbilityDeploymentStore::open_at(store_path).load().unwrap();
        assert_eq!(rows.len(), 1, "failed redeploy must leave one durable row");
        assert_eq!(
            rows[0].manifest_hash(),
            old_record
                .implementation()
                .content_hash()
                .unwrap_or_default(),
            "durable row must remain the previous installed manifest"
        );
    }

    /// New durable rows replay from their embedded manifest snapshot:
    /// deleting or editing the original source bundle after deploy must
    /// not erase the installed ability.
    #[tokio::test]
    async fn boot_replay_uses_embedded_snapshot_when_source_manifest_drifts() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("ability-deployments.json");
        {
            let (registrar, _rt, _catalog) =
                wired_registrar(AbilityDeploymentStore::open_at(store_path.clone()));
            registrar
                .install(host_stream_install(dir.path(), "/tmp/er-host.sock"))
                .await
                .unwrap();
        }
        std::fs::write(dir.path().join("ability.json"), b"{\"name\":\"drifted\"}").unwrap();

        let registrar2 = AbilityDeploymentRegistrar::new_pending_with_store(
            AbilityDeploymentStore::open_at(store_path),
        );
        let rt2 = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let catalog2 = registrar_test_catalog(Arc::clone(&rt2));
        registrar2.set_runtime(Arc::clone(&rt2)).unwrap();
        registrar2
            .set_control_plane_catalog(Arc::downgrade(&catalog2))
            .unwrap();
        let report = registrar2.replay_from_store().await;

        assert_eq!(report.stale, 0);
        assert_eq!(report.registered, 1);
        assert_eq!(report.outcomes.len(), 1);
        assert_eq!(report.outcomes[0].public_name, "er.generate");
        assert_eq!(report.outcomes[0].status, ReplayOutcomeStatus::Registered);
        assert!(rt2.has_ability(er_generate_runtime_key()).await);
    }

    /// Boot replay re-registers an unchanged row (the happy replay path).
    #[tokio::test]
    async fn boot_replay_reregisters_unchanged_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("ability-deployments.json");
        {
            let (registrar, _rt, _catalog) =
                wired_registrar(AbilityDeploymentStore::open_at(store_path.clone()));
            registrar
                .install(host_stream_install(dir.path(), "/tmp/er-host.sock"))
                .await
                .unwrap();
        }
        let registrar2 = AbilityDeploymentRegistrar::new_pending_with_store(
            AbilityDeploymentStore::open_at(store_path),
        );
        let rt2 = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let catalog2 = registrar_test_catalog(Arc::clone(&rt2));
        registrar2.set_runtime(Arc::clone(&rt2)).unwrap();
        registrar2
            .set_control_plane_catalog(Arc::downgrade(&catalog2))
            .unwrap();
        let report = registrar2.replay_from_store().await;

        assert_eq!(report.registered, 1);
        assert_eq!(report.stale, 0);
        assert_eq!(report.outcomes.len(), 1);
        assert_eq!(report.outcomes[0].status, ReplayOutcomeStatus::Registered);
        assert!(rt2.has_ability(er_generate_runtime_key()).await);
    }

    #[tokio::test]
    async fn boot_replay_quarantines_previous_device_authority_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("ability-deployments.json");
        {
            let (registrar, _rt, _catalog) =
                wired_registrar(AbilityDeploymentStore::open_at(store_path.clone()));
            registrar
                .install(host_stream_install(dir.path(), "/tmp/current-host.sock"))
                .await
                .unwrap();
        }

        let previous_manifest = host_stream_manifest("/tmp/old-host.sock", "er.generate");
        let previous_manifest_bytes = serde_json::to_vec(&previous_manifest).unwrap();
        let previous_row = AbilityDeploymentRecord::new_with_manifest_bytes(
            "er.previous",
            "er",
            "easynet:///r/localhost/ability/device.old.er.previous",
            dir.path()
                .join("previous-ability.json")
                .to_string_lossy()
                .into_owned(),
            &previous_manifest_bytes,
            2,
            "easynet:///r/localhost/user/test-user",
            "test-previous-deploy",
        );
        AbilityDeploymentStore::open_at(store_path.clone())
            .upsert(previous_row.clone())
            .unwrap();

        let registrar2 = AbilityDeploymentRegistrar::new_pending_with_store(
            AbilityDeploymentStore::open_at(store_path.clone()),
        );
        let rt2 = crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
            crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
            None,
        );
        let catalog2 = registrar_test_catalog(Arc::clone(&rt2));
        registrar2.set_runtime(Arc::clone(&rt2)).unwrap();
        registrar2
            .set_control_plane_catalog(Arc::downgrade(&catalog2))
            .unwrap();
        let report = registrar2.replay_from_store().await;

        assert_eq!(report.registered, 1);
        assert_eq!(report.quarantined, 1);
        assert_eq!(report.errored, 0);
        assert!(report.outcomes.iter().any(|outcome| {
            outcome.install_id == previous_row.install_id()
                && outcome.status == ReplayOutcomeStatus::Quarantined
        }));
        assert!(rt2.has_ability(er_generate_runtime_key()).await);
        assert!(
            !rt2.has_ability("easynet:///r/localhost/ability/device.old.er.previous")
                .await,
            "foreign ability deployment must never be registered"
        );
        let replayable_rows = AbilityDeploymentStore::open_at(store_path).load().unwrap();
        assert!(
            replayable_rows
                .iter()
                .all(|row| row.install_id() != previous_row.install_id()),
            "quarantined rows must be hidden from boot replay"
        );
    }

    #[tokio::test]
    async fn leased_binding_expiry_removes_route_but_preserves_durable_descriptor() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("ability-deployments.json");
        let (registrar, runtime, catalog) =
            wired_registrar(AbilityDeploymentStore::open_at(store_path.clone()));
        let install = host_stream_install(dir.path(), "/tmp/leased-host.sock")
            .with_binding_lease_ms(Some(MIN_BINDING_LEASE_MS))
            .unwrap();

        registrar.install(install).await.unwrap();
        assert!(runtime.has_ability(er_generate_runtime_key()).await);
        assert_eq!(
            stream_control_plane_record(&catalog).ability(),
            "er.generate"
        );

        tokio::time::sleep(Duration::from_millis(MIN_BINDING_LEASE_MS + 150)).await;

        assert!(!runtime.has_ability(er_generate_runtime_key()).await);
        assert!(catalog
            .control_plane_record_for_mode("er.generate", DescriptorCallMode::Stream)
            .unwrap()
            .is_none());
        let rows = AbilityDeploymentStore::open_at(store_path).load().unwrap();
        assert_eq!(
            rows.len(),
            1,
            "lease expiry must not delete the descriptor install"
        );
        assert_eq!(rows[0].binding_lease_ms(), Some(MIN_BINDING_LEASE_MS));
    }

    #[tokio::test(start_paused = true)]
    async fn leased_binding_expiry_retries_publication_prepare_and_converges() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("ability-deployments.json");
        let (registrar, runtime, catalog) =
            wired_registrar(AbilityDeploymentStore::open_at(store_path.clone()));
        let fail_next_prepare = Arc::new(AtomicBool::new(true));
        let prepare_attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let committed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        registrar
            .install(
                host_stream_install(dir.path(), "/tmp/leased-host.sock")
                    .with_binding_lease_ms(Some(MIN_BINDING_LEASE_MS))
                    .unwrap(),
            )
            .await
            .unwrap();
        let install_id = AbilityDeploymentStore::open_at(store_path.clone())
            .load()
            .unwrap()[0]
            .install_id()
            .to_string();
        catalog
            .register_dynamic_publication_participant(
                Arc::new({
                    let fail_next_prepare = Arc::clone(&fail_next_prepare);
                    let prepare_attempts = Arc::clone(&prepare_attempts);
                    move |_| {
                        prepare_attempts.fetch_add(1, Ordering::SeqCst);
                        if fail_next_prepare.swap(false, Ordering::SeqCst) {
                            anyhow::bail!("injected transient publication prepare failure");
                        }
                        Ok(())
                    }
                }),
                Arc::new({
                    let committed = Arc::clone(&committed);
                    move || {
                        committed.fetch_add(1, Ordering::SeqCst);
                    }
                }),
            )
            .unwrap();

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(MIN_BINDING_LEASE_MS)).await;
        tokio::task::yield_now().await;

        assert_eq!(prepare_attempts.load(Ordering::SeqCst), 1);
        assert!(runtime.has_ability(er_generate_runtime_key()).await);
        assert!(stream_control_plane_record(&catalog).ability() == "er.generate");
        assert_eq!(
            AbilityDeploymentStore::open_at(store_path.clone())
                .load()
                .unwrap()
                .len(),
            1
        );
        assert!(registrar
            .active_leases
            .lock()
            .unwrap()
            .contains_key(&install_id));

        tokio::time::advance(BINDING_LEASE_RETRY_BASE_DELAY).await;
        tokio::task::yield_now().await;

        assert_eq!(prepare_attempts.load(Ordering::SeqCst), 2);
        assert_eq!(committed.load(Ordering::SeqCst), 1);
        assert!(!runtime.has_ability(er_generate_runtime_key()).await);
        assert!(catalog
            .control_plane_record_for_mode("er.generate", DescriptorCallMode::Stream)
            .unwrap()
            .is_none());
        assert_eq!(
            AbilityDeploymentStore::open_at(store_path)
                .load()
                .unwrap()
                .len(),
            1,
            "expiry preserves the durable descriptor"
        );
        assert!(!registrar
            .active_leases
            .lock()
            .unwrap()
            .contains_key(&install_id));
    }

    #[tokio::test(start_paused = true)]
    async fn renewed_generation_cancels_the_failed_expiry_retry() {
        let dir = tempfile::tempdir().unwrap();
        let (registrar, runtime, catalog) = wired_registrar(AbilityDeploymentStore::open_at(
            dir.path().join("ability-deployments.json"),
        ));
        let lease_install = || {
            host_stream_install(dir.path(), "/tmp/leased-host.sock")
                .with_binding_lease_ms(Some(MIN_BINDING_LEASE_MS))
                .unwrap()
        };
        registrar.install(lease_install()).await.unwrap();
        let install_id = registrar.store.load().unwrap()[0].install_id().to_string();
        let reject_prepare = Arc::new(AtomicBool::new(true));
        let prepare_attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        catalog
            .register_dynamic_publication_participant(
                Arc::new({
                    let reject_prepare = Arc::clone(&reject_prepare);
                    let prepare_attempts = Arc::clone(&prepare_attempts);
                    move |_| {
                        prepare_attempts.fetch_add(1, Ordering::SeqCst);
                        if reject_prepare.load(Ordering::SeqCst) {
                            anyhow::bail!("injected publication prepare outage");
                        }
                        Ok(())
                    }
                }),
                Arc::new(|| {}),
            )
            .unwrap();
        let first_generation = registrar.active_leases.lock().unwrap()[&install_id];

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(MIN_BINDING_LEASE_MS)).await;
        tokio::task::yield_now().await;
        assert_eq!(prepare_attempts.load(Ordering::SeqCst), 1);

        reject_prepare.store(false, Ordering::SeqCst);
        registrar.install(lease_install()).await.unwrap();
        let renewed_generation = registrar.active_leases.lock().unwrap()[&install_id];
        assert_ne!(first_generation, renewed_generation);

        tokio::task::yield_now().await;
        tokio::time::advance(BINDING_LEASE_RETRY_BASE_DELAY).await;
        tokio::task::yield_now().await;
        assert_eq!(
            prepare_attempts.load(Ordering::SeqCst),
            1,
            "the stale retry must exit before publication prepare"
        );
        assert!(runtime.has_ability(er_generate_runtime_key()).await);

        tokio::time::advance(
            Duration::from_millis(MIN_BINDING_LEASE_MS) - BINDING_LEASE_RETRY_BASE_DELAY,
        )
        .await;
        tokio::task::yield_now().await;
        assert!(!runtime.has_ability(er_generate_runtime_key()).await);
        assert_eq!(prepare_attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn uninstall_cancels_the_failed_expiry_retry() {
        let dir = tempfile::tempdir().unwrap();
        let (registrar, runtime, catalog) = wired_registrar(AbilityDeploymentStore::open_at(
            dir.path().join("ability-deployments.json"),
        ));
        let install = host_stream_install(dir.path(), "/tmp/leased-host.sock")
            .with_binding_lease_ms(Some(MIN_BINDING_LEASE_MS))
            .unwrap();
        let ability_ura = install.ability_ura().to_string();
        registrar.install(install).await.unwrap();
        let install_id = registrar.store.load().unwrap()[0].install_id().to_string();
        let reject_prepare = Arc::new(AtomicBool::new(true));
        let prepare_attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        catalog
            .register_dynamic_publication_participant(
                Arc::new({
                    let reject_prepare = Arc::clone(&reject_prepare);
                    let prepare_attempts = Arc::clone(&prepare_attempts);
                    move |_| {
                        prepare_attempts.fetch_add(1, Ordering::SeqCst);
                        if reject_prepare.load(Ordering::SeqCst) {
                            anyhow::bail!("injected publication prepare outage");
                        }
                        Ok(())
                    }
                }),
                Arc::new(|| {}),
            )
            .unwrap();

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(MIN_BINDING_LEASE_MS)).await;
        tokio::task::yield_now().await;
        assert_eq!(prepare_attempts.load(Ordering::SeqCst), 1);

        reject_prepare.store(false, Ordering::SeqCst);
        registrar
            .uninstall(AbilityDeploymentUninstall {
                ability_ura,
                install_id: Some(install_id.clone()),
            })
            .await
            .unwrap();
        let attempts_after_uninstall = prepare_attempts.load(Ordering::SeqCst);
        assert!(!registrar
            .active_leases
            .lock()
            .unwrap()
            .contains_key(&install_id));

        tokio::time::advance(BINDING_LEASE_RETRY_MAX_DELAY).await;
        tokio::task::yield_now().await;
        assert_eq!(
            prepare_attempts.load(Ordering::SeqCst),
            attempts_after_uninstall,
            "the uninstalled generation must not retry publication prepare"
        );
        assert!(!runtime.has_ability(er_generate_runtime_key()).await);
        assert!(registrar.store.load().unwrap().is_empty());
    }

    #[tokio::test]
    async fn redeploy_renews_lease_generation_without_premature_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let (registrar, runtime, _catalog) = wired_registrar(AbilityDeploymentStore::open_at(
            dir.path().join("ability-deployments.json"),
        ));
        let leased_install = || {
            host_stream_install(dir.path(), "/tmp/leased-host.sock")
                .with_binding_lease_ms(Some(MIN_BINDING_LEASE_MS))
                .unwrap()
        };

        registrar.install(leased_install()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(650)).await;
        registrar.install(leased_install()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;

        assert!(
            runtime.has_ability(er_generate_runtime_key()).await,
            "the superseded lease timer must not revoke a renewed binding"
        );
        tokio::time::sleep(Duration::from_millis(650)).await;
        assert!(!runtime.has_ability(er_generate_runtime_key()).await);
    }

    #[tokio::test]
    async fn boot_replay_keeps_overdue_leased_implementation_inactive_without_retry() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("ability-deployments.json");
        {
            let (registrar, _runtime, _catalog) =
                wired_registrar(AbilityDeploymentStore::open_at(store_path.clone()));
            registrar
                .install(
                    host_stream_install(dir.path(), "/tmp/leased-host.sock")
                        .with_binding_lease_ms(Some(MAX_BINDING_LEASE_MS))
                        .unwrap(),
                )
                .await
                .unwrap();
        }

        let (replayed, runtime, catalog) =
            wired_registrar(AbilityDeploymentStore::open_at(store_path));
        let report = replayed.replay_from_store().await;

        assert_eq!(report.registered, 0);
        assert_eq!(report.lease_pending, 1);
        assert_eq!(report.outcomes[0].status, ReplayOutcomeStatus::LeasePending);
        assert!(!runtime.has_ability(er_generate_runtime_key()).await);
        assert!(catalog.authority_ability_catalog_snapshot().is_empty());
        assert!(replayed.active_leases.lock().unwrap().is_empty());
    }

    #[test]
    fn shell_exec_requires_permission_broker() {
        let m = AbilityManifest::new("u", "unary", serde_json::json!({"type": "object"}))
            .unwrap()
            .with_exec(AbilityExec::Shell(ShellExec {
                argv: vec!["echo".to_string(), "hi".to_string()],
                stdout: None,
                sandbox: None,
            }))
            .unwrap();
        let err = match build_binding(&m) {
            Ok(_) => panic!("shell exec must require a permission broker"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("permission broker"), "{err}");
    }

    #[test]
    fn build_binding_rejects_no_exec() {
        let m = AbilityManifest::new("generate", "d", serde_json::json!({"type": "object"}))
            .expect("manifest");
        assert!(build_binding(&m).is_err(), "no [exec] must be rejected");
    }
}
