// EasyNet CLI — DeviceAbilityRegistrar
// =================================================================
//
// File: src/daemon/ability/builtins/device_control/ability_management/registrar.rs
//
// The "runtime binding" leg of the `ability.deploy` transaction, and
// the boot-time replay of the durable catalog. Turns a deployed
// device-ability manifest into a LIVE, OwnerKind::Device row in the
// Axon `LocalRuntime`, then verifies it is actually routable before
// the deploy handler may report ACTIVE.
//
//     ability.deploy = manifest materialization     (device_ops_ability)
//                    + runtime binding               (THIS FILE)
//                    + durable catalog commit        (device_ability_store)
//
// Pending-runtime pattern: constructed at registry-build time without a
// runtime (catalog is built before the runtime exists), boot calls
// `set_runtime` once. Mirrors `HotAgentRegistrar`.
//
// Call mode is inferred from `exec.kind`, not a separate manifest
// field: `host_stream` is server-stream (the only external-process
// stream path); every other exec kind is unary RPC. There is no
// ambiguity to encode, so no `call_mode` field is introduced.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use axon_sdk::invocation::{
    AbilityCallModes, AbilityDescriptor, AbilityFn, AbilityOptions, CallMode as AxonCallMode,
    LocalRuntime,
};
use serde::Serialize;

use crate::daemon::ability::builtins::agents::chat::build_host_stream_handler;
use crate::daemon::ability::builtins::device_control::ability_management::store::{
    manifest_digest, DeviceAbilityRecord, DeviceAbilityStore,
};
use crate::daemon::ability::dispatch::{
    stream_env_ability_with_options, AxonAbilityCatalog, ControlPlaneAuthorityModeTxn,
    ControlPlaneAuthorityRebind, ControlPlaneImplementation,
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

/// One installed device ability, ready to register.
pub struct DeviceAbilityInstall {
    /// Wire dispatch key (e.g. `er.generate`).
    key: String,
    namespace: String,
    ability_ura: String,
    manifest_path: String,
    manifest_bytes: Vec<u8>,
    manifest: AbilityManifest,
    /// Caller-supplied install timestamp (runtime forbids ambient clock).
    installed_at_unix_ms: u64,
}

impl DeviceAbilityInstall {
    pub fn new(
        key: impl Into<String>,
        namespace: impl Into<String>,
        ability_ura: impl Into<String>,
        manifest_path: impl Into<String>,
        manifest_bytes: Vec<u8>,
        manifest: AbilityManifest,
        installed_at_unix_ms: u64,
    ) -> anyhow::Result<Self> {
        let key = key.into();
        let namespace = namespace.into();
        let ability_ura = ability_ura.into();
        let manifest_path = manifest_path.into();
        Self::validate(
            &key,
            &namespace,
            &ability_ura,
            &manifest_path,
            &manifest_bytes,
            &manifest,
        )?;
        Ok(Self {
            key,
            namespace,
            ability_ura,
            manifest_path,
            manifest_bytes,
            manifest,
            installed_at_unix_ms,
        })
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
        if selector.owner_kind() != "device" {
            anyhow::bail!(
                "ability.deploy: device install requires a device-owned Ability URA, got owner kind {:?}",
                selector.owner_kind()
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAbilityUninstall {
    pub ability_ura: String,
    pub install_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAbilityUninstallOutcome {
    pub public_names: Vec<String>,
    pub install_ids: Vec<String>,
    pub runtime_removed: usize,
    pub control_plane_removed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceAbilityUninstallStep {
    Planned,
    DurableTombstoned,
    RuntimeCleared,
    ControlPlaneCleared,
    StoreCommitted,
}

struct DeviceAbilityUninstallTransaction {
    step: DeviceAbilityUninstallStep,
    removed: Vec<DeviceAbilityRecord>,
    public_names: Vec<String>,
    install_ids: Vec<String>,
    runtime_removed: usize,
    control_plane_removed: usize,
    resumed_tombstone: bool,
}

impl DeviceAbilityUninstallTransaction {
    fn new(removed: Vec<DeviceAbilityRecord>, resumed_tombstone: bool) -> Self {
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
            step: DeviceAbilityUninstallStep::Planned,
            removed,
            public_names,
            install_ids,
            runtime_removed: 0,
            control_plane_removed: 0,
            resumed_tombstone,
        }
    }

    fn advance(&mut self, step: DeviceAbilityUninstallStep) {
        self.step = step;
    }

    fn outcome(self) -> DeviceAbilityUninstallOutcome {
        DeviceAbilityUninstallOutcome {
            public_names: self.public_names,
            install_ids: self.install_ids,
            runtime_removed: self.runtime_removed,
            control_plane_removed: self.control_plane_removed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceAbilityControlPlaneKey {
    authority_root: String,
    public_name: String,
    call_mode: DescriptorCallMode,
}

impl DeviceAbilityControlPlaneKey {
    fn from_install(
        install: &DeviceAbilityInstall,
        call_mode: DescriptorCallMode,
    ) -> anyhow::Result<Self> {
        Self::from_ability_ura(install.ability_ura(), install.key(), call_mode)
    }

    fn from_record(
        record: &DeviceAbilityRecord,
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
        if selector.owner_kind() != "device" {
            anyhow::bail!(
                "device ability control-plane key requires device owner, got {:?} in {}",
                selector.owner_kind(),
                ability_ura
            );
        }
        if selector.public_name() != expected_public_name {
            anyhow::bail!(
                "device ability control-plane key public name drift: URA has {:?}, record has {:?}",
                selector.public_name(),
                expected_public_name
            );
        }
        Ok(Self {
            authority_root: selector.owner_ura().to_string(),
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
        AuthorityScope::new("device", self.authority_root.clone()).map_err(|error| {
            anyhow::anyhow!(
                "device ability control-plane authority scope rejected for {}: {error}",
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
        install: &DeviceAbilityInstall,
        record: &AbilityControlPlaneRecord,
    ) -> anyhow::Result<Self> {
        Self::from_manifest(install.ability_ura(), install.manifest(), record)
    }

    fn from_record(
        row: &DeviceAbilityRecord,
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
        let call_mode = descriptor_call_mode_for_modes(options.modes);
        if record.descriptor().call_mode() != call_mode {
            anyhow::bail!(
                "device ability runtime binding mode drift for {:?}: manifest implies {:?}, \
                 control-plane record has {:?}",
                record.ability(),
                call_mode,
                record.descriptor().call_mode()
            );
        }
        if record.descriptor().version.as_str() != manifest.descriptor_version() {
            anyhow::bail!(
                "device ability runtime binding version drift for {:?}: manifest has {}, \
                 control-plane record has {}",
                record.ability(),
                manifest.descriptor_version(),
                record.descriptor().version.as_str()
            );
        }
        let axon_call_mode = axon_call_mode_for_descriptor_mode(call_mode);
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

/// Constructed pending at registry-build time; boot injects the runtime
/// via [`DeviceAbilityRegistrar::set_runtime`]. Owns the durable store.
pub struct DeviceAbilityRegistrar {
    runtime: OnceLock<Arc<LocalRuntime>>,
    control_plane_catalog: OnceLock<Weak<AxonAbilityCatalog>>,
    store: DeviceAbilityStore,
    #[cfg(test)]
    fail_next_runtime_replace: AtomicBool,
}

impl DeviceAbilityRegistrar {
    #[must_use]
    pub fn new_pending() -> Arc<Self> {
        Arc::new(Self {
            runtime: OnceLock::new(),
            control_plane_catalog: OnceLock::new(),
            store: DeviceAbilityStore::open_default(),
            #[cfg(test)]
            fail_next_runtime_replace: AtomicBool::new(false),
        })
    }

    /// Test seam: explicit store path.
    #[must_use]
    pub fn new_pending_with_store(store: DeviceAbilityStore) -> Arc<Self> {
        Arc::new(Self {
            runtime: OnceLock::new(),
            control_plane_catalog: OnceLock::new(),
            store,
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
            .map_err(|_| anyhow::anyhow!("device ability registrar: runtime already wired"))
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
            anyhow::anyhow!("device ability registrar: control-plane catalog already wired")
        })
    }

    fn runtime(&self) -> anyhow::Result<&Arc<LocalRuntime>> {
        self.runtime
            .get()
            .ok_or_else(|| anyhow::anyhow!("device ability registrar: runtime not wired yet"))
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
    pub async fn install(&self, install: DeviceAbilityInstall) -> anyhow::Result<InstallState> {
        let runtime = self.runtime()?.clone();
        let catalog = self.control_plane_catalog("ability.deploy")?;
        let key = install.key().to_string();
        if catalog.has_static_ability(&key) {
            anyhow::bail!(
                "ability.deploy: refusing to shadow boot-time ability {key:?}; \
                 choose a distinct device ability name"
            );
        }

        let call_mode = descriptor_call_mode_for_manifest(install.manifest())?;
        let control_plane_key = DeviceAbilityControlPlaneKey::from_install(&install, call_mode)?;

        // ── durable installing intent (hidden from boot replay) ─────
        let record = DeviceAbilityRecord::new_installing_with_manifest_bytes(
            key.clone(),
            install.namespace().to_string(),
            install.ability_ura().to_string(),
            install.manifest_path().to_string(),
            install.manifest_bytes(),
            install.installed_at_unix_ms(),
        );
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
                    "restore durable device ability store",
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
                        "restore durable device ability store and prior live runtime binding",
                    ),
                    control_plane_restore,
                    "restore prior control-plane records",
                ));
            }
        };
        let want = binding.modes();
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
                    "restore durable device ability store and prior live runtime binding",
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
                    "restore durable device ability store and prior live runtime binding",
                ),
                control_plane_restore,
                "restore prior control-plane records",
            ));
        }
        control_plane_txn.commit();
        catalog.notify_dynamic_publication_hooks();
        Ok(state)
    }

    /// Remove a deployed device ability from durable store, live
    /// LocalRuntime, and the daemon control-plane registry. This is the
    /// inverse of `install`; `ability.uninstall` must never report
    /// REMOVED while any of those three legs still advertises the
    /// binding.
    pub async fn uninstall(
        &self,
        uninstall: DeviceAbilityUninstall,
    ) -> anyhow::Result<DeviceAbilityUninstallOutcome> {
        let runtime = self.runtime()?.clone();
        let catalog = self.control_plane_catalog("ability.uninstall")?;
        let removal_plan = self
            .store
            .stage_remove_by_ability(&uninstall.ability_ura, uninstall.install_id.as_deref())?;
        if removal_plan.is_empty() {
            match uninstall.install_id.as_deref() {
                Some(id) => anyhow::bail!(
                    "ability.uninstall: no installed device ability matched ability_ura {:?} \
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
        let mut transaction =
            DeviceAbilityUninstallTransaction::new(removal_plan.into_records(), resumed_tombstone);
        transaction.advance(DeviceAbilityUninstallStep::DurableTombstoned);
        let control_plane_removals = match Self::control_plane_removal_keys(&transaction.removed) {
            Ok(keys) => keys,
            Err(err) => {
                if let Err(restore_err) = self.store.restore_records(transaction.removed.clone()) {
                    anyhow::bail!(
                        "ability.uninstall: failed to infer control-plane modes: {err}; \
                             additionally failed to restore durable store rows: {restore_err}"
                    );
                }
                anyhow::bail!(
                        "ability.uninstall: failed to infer control-plane modes: {err}; durable store rows restored"
                    );
            }
        };
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
        transaction.advance(DeviceAbilityUninstallStep::RuntimeCleared);

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
        transaction.advance(DeviceAbilityUninstallStep::ControlPlaneCleared);

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
        transaction.advance(DeviceAbilityUninstallStep::StoreCommitted);
        catalog.notify_dynamic_publication_hooks();

        Ok(transaction.outcome())
    }

    async fn restore_live_records(&self, records: &[DeviceAbilityRecord]) -> anyhow::Result<usize> {
        let runtime = self.runtime()?.clone();
        let catalog = self.control_plane_catalog("device ability restore")?;
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
            let control_plane_key = DeviceAbilityControlPlaneKey::from_record(
                row,
                descriptor_call_mode_for_manifest(&manifest)?,
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
        records: &[DeviceAbilityRecord],
    ) -> anyhow::Result<Vec<DeviceAbilityControlPlaneKey>> {
        let mut keys = Vec::new();
        for row in records {
            let bytes = row.manifest_bytes().map_err(|e| {
                anyhow::anyhow!(
                    "infer control-plane mode for {}: read manifest: {e}",
                    row.public_name()
                )
            })?;
            let manifest = AbilityManifest::from_json_slice(&bytes).map_err(|e| {
                anyhow::anyhow!(
                    "infer control-plane mode for {}: parse manifest: {e}",
                    row.public_name()
                )
            })?;
            let key = DeviceAbilityControlPlaneKey::from_record(
                row,
                descriptor_call_mode_for_manifest(&manifest)?,
            )?;
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        Ok(keys)
    }

    /// Boot replay (invariant 7): recover uncommitted install intents,
    /// then re-register every committed durable row into the live runtime
    /// from its embedded manifest snapshot.
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
        let catalog = match self.control_plane_catalog("device ability replay") {
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
                    "device ability owner {owner} is not hosted by current daemon authority {hosted_device_authority_root}; row hidden from boot replay"
                ),
            );
        }
        for row in rows {
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
            let descriptor_call_mode = match descriptor_call_mode_for_manifest(&manifest) {
                Ok(mode) => mode,
                Err(err) => {
                    report.push_errored(&row, format!("infer descriptor call mode: {err}"));
                    continue;
                }
            };
            let control_plane_key =
                match DeviceAbilityControlPlaneKey::from_record(&row, descriptor_call_mode) {
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
        key: &DeviceAbilityControlPlaneKey,
    ) -> ControlPlaneAuthorityModeTxn<'a> {
        catalog.begin_control_plane_authority_mode_transaction(
            key.authority_root(),
            key.public_name(),
            key.call_mode(),
        )
    }

    fn rebind_control_plane_record(
        catalog: &AxonAbilityCatalog,
        key: &DeviceAbilityControlPlaneKey,
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
                RuntimeEnv::device_ability(deployed_exec_kind(manifest)),
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
    fn push_registered(&mut self, row: &DeviceAbilityRecord) {
        self.registered += 1;
        self.outcomes.push(ReplayOutcome::new(
            row,
            ReplayOutcomeStatus::Registered,
            "registered into LocalRuntime and control-plane catalog",
        ));
    }

    fn push_stale(&mut self, row: &DeviceAbilityRecord, detail: impl Into<String>) {
        self.stale += 1;
        self.outcomes
            .push(ReplayOutcome::new(row, ReplayOutcomeStatus::Stale, detail));
    }

    fn push_quarantined(&mut self, row: &DeviceAbilityRecord, detail: impl Into<String>) {
        self.quarantined += 1;
        self.outcomes.push(ReplayOutcome::new(
            row,
            ReplayOutcomeStatus::Quarantined,
            detail,
        ));
    }

    fn push_errored(&mut self, row: &DeviceAbilityRecord, detail: impl Into<String>) {
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
        row: &DeviceAbilityRecord,
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
    Stale,
    Quarantined,
    Errored,
}

/// Build the `(AbilityFn, AbilityOptions)` for a device ability from its
/// manifest's exec kind. host_stream → stream-mode. Shell exec is
/// rejected until a permission broker and receipt-audited operator
/// approval path exist; `ability.deploy` must not become an arbitrary
/// host-command surface without that broker.
/// The single deployable device exec kind, with the deployability policy and
/// its operator-facing rejection strings defined in exactly one place.
///
/// `build_binding` (handler + options) and `descriptor_call_mode_for_manifest`
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
                "ability.deploy: device ability exec kind {other:?} is not deployable \
                 (only host_stream is supported on the device deploy path)"
            )),
            None => Err(anyhow::anyhow!(
                "ability.deploy: device ability manifest has no [exec] binding"
            )),
        }
    }

    fn descriptor_call_mode(&self) -> DescriptorCallMode {
        match self {
            Self::HostStream(_) => DescriptorCallMode::Stream,
        }
    }
}

fn build_binding(manifest: &AbilityManifest) -> anyhow::Result<(AbilityFn, AbilityOptions)> {
    match DeployableExec::classify(manifest)? {
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

fn descriptor_call_mode_for_modes(modes: AbilityCallModes) -> DescriptorCallMode {
    if modes.bidi {
        DescriptorCallMode::Bidi
    } else if modes.stream {
        DescriptorCallMode::Stream
    } else {
        DescriptorCallMode::Rpc
    }
}

fn descriptor_call_mode_for_manifest(
    manifest: &AbilityManifest,
) -> anyhow::Result<DescriptorCallMode> {
    Ok(DeployableExec::classify(manifest)?.descriptor_call_mode())
}

fn axon_call_mode_for_descriptor_mode(mode: DescriptorCallMode) -> AxonCallMode {
    match mode {
        DescriptorCallMode::Rpc => AxonCallMode::Rpc,
        DescriptorCallMode::Stream => AxonCallMode::Stream,
        DescriptorCallMode::Bidi => AxonCallMode::Bidi,
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
        let registrar = DeviceAbilityRegistrar::new_pending();
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
        let registrar = DeviceAbilityRegistrar::new_pending();
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
        AbilityManifest::new(
            "generate",
            "stream gen",
            serde_json::json!({"type": "object"}),
        )
        .unwrap()
        .with_admission_action("stream")
        .unwrap()
        .with_exec(AbilityExec::HostStream(HostStreamExec {
            host_socket: socket.to_string(),
            function: function.to_string(),
        }))
        .unwrap()
    }

    fn host_stream_install(store_dir: &std::path::Path, socket: &str) -> DeviceAbilityInstall {
        host_stream_install_with_version(store_dir, socket, None)
    }

    fn host_stream_install_with_version(
        store_dir: &std::path::Path,
        socket: &str,
        descriptor_version: Option<&str>,
    ) -> DeviceAbilityInstall {
        let mut manifest = host_stream_manifest(socket, "er.generate");
        if let Some(descriptor_version) = descriptor_version {
            manifest = manifest
                .with_descriptor_version(descriptor_version)
                .expect("descriptor version must validate");
        }
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let manifest_path = store_dir.join("ability.json");
        std::fs::write(&manifest_path, &manifest_bytes).unwrap();
        DeviceAbilityInstall::new(
            "er.generate",
            "er",
            "easynet:///r/localhost/ability/device.d1.er.generate",
            manifest_path.to_string_lossy().into_owned(),
            manifest_bytes,
            manifest,
            1,
        )
        .unwrap()
    }

    fn er_generate_runtime_key() -> &'static str {
        "easynet:///r/localhost/ability/device.d1.er.generate"
    }

    fn wired_registrar(
        store: DeviceAbilityStore,
    ) -> (
        Arc<DeviceAbilityRegistrar>,
        Arc<LocalRuntime>,
        Arc<AxonAbilityCatalog>,
    ) {
        let registrar = DeviceAbilityRegistrar::new_pending_with_store(store);
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

    fn stream_control_plane_record(catalog: &AxonAbilityCatalog) -> AbilityControlPlaneRecord {
        catalog
            .control_plane_record_for_mode("er.generate", DescriptorCallMode::Stream)
            .expect("device ability control-plane lookup is unambiguous")
            .expect("device ability control-plane record")
    }

    // ── Negative test matrix (plan §"必测四个失败态") ──────────────

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
        let store = DeviceAbilityStore::open_at(bogus_parent.join("device-abilities.json"));
        let registrar = DeviceAbilityRegistrar::new_pending_with_store(store);
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
        let store_path = dir.path().join("device-abilities.json");
        let store = DeviceAbilityStore::open_at(store_path.clone());
        let registrar = DeviceAbilityRegistrar::new_pending_with_store(store);
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
        let result = DeviceAbilityInstall::new(
            "",
            install.namespace(),
            install.ability_ura(),
            install.manifest_path(),
            install.manifest_bytes().to_vec(),
            install.manifest().clone(),
            install.installed_at_unix_ms(),
        );

        assert!(
            result.is_err(),
            "invalid runtime key must fail construction"
        );
        let rows = DeviceAbilityStore::open_at(store_path).load().unwrap();
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
        let store = DeviceAbilityStore::open_at(dir.path().join("device-abilities.json"));
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
    async fn install_rebinds_device_ability_control_plane_record() {
        let dir = tempfile::tempdir().unwrap();
        let store = DeviceAbilityStore::open_at(dir.path().join("device-abilities.json"));
        let (registrar, _, catalog) = wired_registrar(store);

        let state = registrar
            .install(host_stream_install(dir.path(), "/tmp/er-host.sock"))
            .await
            .unwrap();

        assert_eq!(state, InstallState::Active);
        let record = catalog
            .control_plane_record_for_mode("er.generate", DescriptorCallMode::Stream)
            .expect("device ability control-plane lookup is unambiguous")
            .expect("device ability control-plane record");
        assert_eq!(record.authority().scope().owner_projection(), "device");
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
                .contains("device-ability:host_stream"),
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
        let store = DeviceAbilityStore::open_at(dir.path().join("device-abilities.json"));
        let (registrar, _, catalog) = wired_registrar(store);
        let notifications = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        catalog.register_dynamic_publication_hook(Arc::new({
            let notifications = Arc::clone(&notifications);
            move || {
                notifications.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }));

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
            .uninstall(DeviceAbilityUninstall {
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
        let store = DeviceAbilityStore::open_at(dir.path().join("device-abilities.json"));
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
            .expect("device ability control-plane lookup is unambiguous")
            .expect("device ability control-plane record");
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
        let store_path = dir.path().join("device-abilities.json");
        let (registrar, rt, catalog) =
            wired_registrar(DeviceAbilityStore::open_at(store_path.clone()));

        let install = host_stream_install(dir.path(), "/tmp/er-host.sock");
        let ability_ura = install.ability_ura().to_string();
        registrar.install(install).await.unwrap();
        assert!(rt.has_ability(er_generate_runtime_key()).await);
        assert!(catalog
            .control_plane_record_for_mode("er.generate", DescriptorCallMode::Stream)
            .expect("device ability control-plane lookup is unambiguous")
            .is_some());
        assert_eq!(
            DeviceAbilityStore::open_at(store_path.clone())
                .load()
                .unwrap()
                .len(),
            1
        );

        let outcome = registrar
            .uninstall(DeviceAbilityUninstall {
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
            .expect("device ability control-plane lookup is unambiguous")
            .is_none());
        assert!(DeviceAbilityStore::open_at(store_path)
            .load()
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn uninstall_refuses_to_mutate_when_control_plane_catalog_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("device-abilities.json");
        let (registrar, rt, catalog) =
            wired_registrar(DeviceAbilityStore::open_at(store_path.clone()));

        let install = host_stream_install(dir.path(), "/tmp/er-host.sock");
        let ability_ura = install.ability_ura().to_string();
        registrar.install(install).await.unwrap();
        drop(catalog);

        let err = registrar
            .uninstall(DeviceAbilityUninstall {
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
            DeviceAbilityStore::open_at(store_path)
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
        let store_path = dir.path().join("device-abilities.json");
        let (registrar, rt, _catalog) =
            wired_registrar(DeviceAbilityStore::open_at(store_path.clone()));

        registrar
            .install(host_stream_install(dir.path(), "/tmp/er-host.sock"))
            .await
            .unwrap();
        registrar
            .install(host_stream_install(dir.path(), "/tmp/er-host.sock"))
            .await
            .unwrap();

        let rows = DeviceAbilityStore::open_at(store_path).load().unwrap();
        assert_eq!(rows.len(), 1, "same manifest must upsert, not duplicate");
        assert!(rt.has_ability(er_generate_runtime_key()).await);
    }

    #[tokio::test]
    async fn control_plane_rebind_failure_restores_previous_live_binding() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("device-abilities.json");
        let (registrar, rt, catalog) =
            wired_registrar(DeviceAbilityStore::open_at(store_path.clone()));

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
            DeviceAbilityStore::open_at(store_path)
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
        let store_path = dir.path().join("device-abilities.json");
        let (registrar, rt, catalog) =
            wired_registrar(DeviceAbilityStore::open_at(store_path.clone()));

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
        let rows = DeviceAbilityStore::open_at(store_path).load().unwrap();
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
        let store_path = dir.path().join("device-abilities.json");
        {
            let (registrar, _rt, _catalog) =
                wired_registrar(DeviceAbilityStore::open_at(store_path.clone()));
            registrar
                .install(host_stream_install(dir.path(), "/tmp/er-host.sock"))
                .await
                .unwrap();
        }
        std::fs::write(dir.path().join("ability.json"), b"{\"name\":\"drifted\"}").unwrap();

        let registrar2 =
            DeviceAbilityRegistrar::new_pending_with_store(DeviceAbilityStore::open_at(store_path));
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
        let store_path = dir.path().join("device-abilities.json");
        {
            let (registrar, _rt, _catalog) =
                wired_registrar(DeviceAbilityStore::open_at(store_path.clone()));
            registrar
                .install(host_stream_install(dir.path(), "/tmp/er-host.sock"))
                .await
                .unwrap();
        }
        let registrar2 =
            DeviceAbilityRegistrar::new_pending_with_store(DeviceAbilityStore::open_at(store_path));
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
        let store_path = dir.path().join("device-abilities.json");
        {
            let (registrar, _rt, _catalog) =
                wired_registrar(DeviceAbilityStore::open_at(store_path.clone()));
            registrar
                .install(host_stream_install(dir.path(), "/tmp/current-host.sock"))
                .await
                .unwrap();
        }

        let previous_manifest = host_stream_manifest("/tmp/old-host.sock", "er.generate");
        let previous_manifest_bytes = serde_json::to_vec(&previous_manifest).unwrap();
        let previous_row = DeviceAbilityRecord::new_with_manifest_bytes(
            "er.previous",
            "er",
            "easynet:///r/localhost/ability/device.old.er.previous",
            dir.path()
                .join("previous-ability.json")
                .to_string_lossy()
                .into_owned(),
            &previous_manifest_bytes,
            2,
        );
        DeviceAbilityStore::open_at(store_path.clone())
            .upsert(previous_row.clone())
            .unwrap();

        let registrar2 = DeviceAbilityRegistrar::new_pending_with_store(
            DeviceAbilityStore::open_at(store_path.clone()),
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
            "foreign device ability must never be registered"
        );
        let replayable_rows = DeviceAbilityStore::open_at(store_path).load().unwrap();
        assert!(
            replayable_rows
                .iter()
                .all(|row| row.install_id() != previous_row.install_id()),
            "quarantined rows must be hidden from boot replay"
        );
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
