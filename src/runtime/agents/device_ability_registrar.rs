// EasyNet CLI — DeviceAbilityRegistrar
// =================================================================
//
// File: src/runtime/agents/device_ability_registrar.rs
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

use std::sync::{Arc, OnceLock, Weak};

use easynet_axon::invocation::{
    AbilityCallModes, AbilityDescriptor, AbilityFn, AbilityOptions, LocalRuntime,
};
use serde::Serialize;

use crate::core::ability_spec::{AbilityExec, AbilityManifest};
use crate::runtime::ability::{
    AbilityDescriptorKey, AbilityImplSource, AuthorityScope, CallMode as DescriptorCallMode,
    RuntimeEnv,
};
use crate::runtime::ability_dispatch::{
    stream_env_ability_with_options, AxonAbilityCatalog, ControlPlaneAuthorityRebind,
    ControlPlaneImplementation,
};
use crate::runtime::agents::chat_ability::build_host_stream_handler;
use crate::runtime::agents::device_ability_store::{
    manifest_digest, DeviceAbilityRecord, DeviceAbilityStore,
};

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
        let selector = crate::ura::AbilitySelector::parse(ability_ura)?;
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
        let selector = crate::ura::AbilitySelector::parse(ability_ura)?;
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

/// Constructed pending at registry-build time; boot injects the runtime
/// via [`DeviceAbilityRegistrar::set_runtime`]. Owns the durable store.
pub struct DeviceAbilityRegistrar {
    runtime: OnceLock<Arc<LocalRuntime>>,
    control_plane_catalog: OnceLock<Weak<AxonAbilityCatalog>>,
    store: DeviceAbilityStore,
}

impl DeviceAbilityRegistrar {
    #[must_use]
    pub fn new_pending() -> Arc<Self> {
        Arc::new(Self {
            runtime: OnceLock::new(),
            control_plane_catalog: OnceLock::new(),
            store: DeviceAbilityStore::open_default(),
        })
    }

    /// Test seam: explicit store path.
    #[must_use]
    pub fn new_pending_with_store(store: DeviceAbilityStore) -> Arc<Self> {
        Arc::new(Self {
            runtime: OnceLock::new(),
            control_plane_catalog: OnceLock::new(),
            store,
        })
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

        // ── build handler + options from exec kind ──────────────────
        let (ability_fn, options) = build_binding(install.manifest())?;
        let want = options.modes;
        let control_plane_key = DeviceAbilityControlPlaneKey::from_install(
            &install,
            descriptor_call_mode_for_modes(want),
        )?;
        let runtime_key = install.ability_ura().to_string();

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

        // ── replace_ability (binding); rollback staged row on failure ─
        if let Err(e) = runtime
            .replace_ability(runtime_key.clone(), ability_fn, options)
            .await
        {
            let rollback = self
                .store
                .rollback_install(record.install_id(), overwritten);
            return Err(append_cleanup_error(
                anyhow::anyhow!(
                    "ability.deploy: replace_ability({runtime_key}) for {key} failed: {e}"
                ),
                rollback,
                "restore durable device ability store",
            ));
        }

        // ── route + mode visibility (invariant 3, the danger point) ─
        // ACTIVE iff the runtime descriptor can see exactly this key AND
        // its registered modes match the call mode we intended to bind.
        // `has_ability` alone is NOT enough.
        let state = match runtime.ability_descriptor(&runtime_key).await {
            Some(desc) if runtime_descriptor_matches(&desc, &runtime_key, want) => {
                InstallState::Active
            }
            _ => InstallState::Installed,
        };

        if let Err(e) = Self::rebind_control_plane_record(
            &catalog,
            &control_plane_key,
            install.manifest(),
            record.manifest_hash(),
        ) {
            let _ = runtime.unregister_ability(&runtime_key).await;
            let overwritten_for_restore = overwritten.clone();
            let rollback = self
                .store
                .rollback_install(record.install_id(), overwritten);
            let live_restore = self.restore_live_records(&overwritten_for_restore).await;
            return Err(append_cleanup_error(
                anyhow::anyhow!("ability.deploy: control-plane rebind({key}) failed: {e}"),
                rollback.and(live_restore.map(|_| ())),
                "restore durable device ability store and prior live runtime binding",
            ));
        }
        if let Err(e) = self.store.commit_installed(record.install_id()) {
            let _ = runtime.unregister_ability(&runtime_key).await;
            catalog.remove_control_plane_record_for_authority_mode(
                control_plane_key.authority_root(),
                control_plane_key.public_name(),
                control_plane_key.call_mode(),
            );
            let overwritten_for_restore = overwritten.clone();
            let rollback = self
                .store
                .rollback_install(record.install_id(), overwritten);
            let live_restore = self.restore_live_records(&overwritten_for_restore).await;
            return Err(append_cleanup_error(
                anyhow::anyhow!("ability.deploy: commit installed({key}) failed: {e}"),
                rollback.and(live_restore.map(|_| ())),
                "restore durable device ability store and prior live runtime binding",
            ));
        }
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

        Ok(transaction.outcome())
    }

    async fn restore_live_records(&self, records: &[DeviceAbilityRecord]) -> anyhow::Result<usize> {
        let runtime = self.runtime()?.clone();
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
            let (ability_fn, options) = build_binding(&manifest).map_err(|e| {
                anyhow::anyhow!("restore {}: build binding: {e}", row.public_name())
            })?;
            let modes = options.modes;
            runtime
                .replace_ability(row.ability_ura().to_string(), ability_fn, options)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "restore {}: replace runtime ability: {e}",
                        row.public_name()
                    )
                })?;
            let control_plane_key = DeviceAbilityControlPlaneKey::from_record(
                row,
                descriptor_call_mode_for_modes(modes),
            )?;
            self.rebind_control_plane(&control_plane_key, &manifest, row.manifest_hash())?;
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
            let (_, options) = build_binding(&manifest).map_err(|e| {
                anyhow::anyhow!(
                    "infer control-plane mode for {}: build binding: {e}",
                    row.public_name()
                )
            })?;
            let key = DeviceAbilityControlPlaneKey::from_record(
                row,
                descriptor_call_mode_for_modes(options.modes),
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

        let mut report = ReplayReport {
            recovered_installing,
            ..ReplayReport::default()
        };
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
            let (ability_fn, options) = match build_binding(&manifest) {
                Ok(pair) => pair,
                Err(err) => {
                    report.push_errored(&row, format!("build runtime binding: {err}"));
                    continue;
                }
            };
            let descriptor_call_mode = descriptor_call_mode_for_modes(options.modes);
            match runtime
                .replace_ability(row.ability_ura().to_string(), ability_fn, options)
                .await
            {
                Ok(_) => {
                    match DeviceAbilityControlPlaneKey::from_record(&row, descriptor_call_mode)
                        .and_then(|control_plane_key| {
                            self.rebind_control_plane(
                                &control_plane_key,
                                &manifest,
                                row.manifest_hash(),
                            )
                        }) {
                        Ok(()) => {
                            report.push_registered(&row);
                        }
                        Err(err) => {
                            let _ = runtime.unregister_ability(row.ability_ura()).await;
                            report.push_errored(
                                &row,
                                format!("rebind control-plane descriptor: {err}"),
                            );
                        }
                    }
                }
                Err(err) => {
                    report.push_errored(&row, format!("replace runtime ability: {err}"));
                }
            }
        }
        report
    }

    fn rebind_control_plane(
        &self,
        key: &DeviceAbilityControlPlaneKey,
        manifest: &AbilityManifest,
        impl_content_hash: &str,
    ) -> anyhow::Result<()> {
        let catalog = self.control_plane_catalog("device ability rebind")?;
        Self::rebind_control_plane_record(&catalog, key, manifest, impl_content_hash)
    }

    fn rebind_control_plane_record(
        catalog: &AxonAbilityCatalog,
        key: &DeviceAbilityControlPlaneKey,
        manifest: &AbilityManifest,
        impl_content_hash: &str,
    ) -> anyhow::Result<()> {
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
    Errored,
}

/// Build the `(AbilityFn, AbilityOptions)` for a device ability from its
/// manifest's exec kind. host_stream → stream-mode. Shell exec is
/// rejected until a permission broker and receipt-audited operator
/// approval path exist; `ability.deploy` must not become an arbitrary
/// host-command surface without that broker.
fn build_binding(manifest: &AbilityManifest) -> anyhow::Result<(AbilityFn, AbilityOptions)> {
    match manifest.exec() {
        Some(AbilityExec::HostStream(spec)) => {
            let handler = build_host_stream_handler(spec.clone());
            Ok(stream_env_ability_with_options(handler))
        }
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

#[cfg(test)]
fn rpc_only() -> AbilityCallModes {
    AbilityCallModes {
        rpc: true,
        stream: false,
        bidi: false,
    }
}

fn append_cleanup_error(
    primary: anyhow::Error,
    cleanup: anyhow::Result<()>,
    cleanup_action: &'static str,
) -> anyhow::Error {
    match cleanup {
        Ok(()) => primary,
        Err(cleanup_err) => {
            anyhow::anyhow!("{primary}; additionally failed to {cleanup_action}: {cleanup_err}")
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ability_dispatch::rpc_handler_to_ability_fn;

    #[test]
    fn install_state_wire_strings() {
        assert_eq!(InstallState::Active.as_wire(), "ACTIVE");
        assert_eq!(InstallState::Installed.as_wire(), "INSTALLED");
    }

    #[test]
    fn registrar_rejects_duplicate_runtime_wiring() {
        let registrar = DeviceAbilityRegistrar::new_pending();
        let first = LocalRuntime::new();
        let second = LocalRuntime::new();

        registrar.set_runtime(first).unwrap();
        let err = registrar.set_runtime(second).unwrap_err();

        assert!(err.to_string().contains("runtime already wired"), "{err}");
    }

    #[test]
    fn registrar_rejects_duplicate_control_plane_wiring() {
        let registrar = DeviceAbilityRegistrar::new_pending();
        let rt = LocalRuntime::new();
        let first = Arc::new(AxonAbilityCatalog::new_with_runtime(Arc::clone(&rt)));
        let second = Arc::new(AxonAbilityCatalog::new_with_runtime(rt));

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

    use crate::core::ability_spec::{AbilityExec, HostStreamExec, ShellExec};

    fn host_stream_manifest(socket: &str, function: &str) -> AbilityManifest {
        AbilityManifest::new(
            "generate",
            "stream gen",
            serde_json::json!({"type": "object"}),
        )
        .unwrap()
        .with_exec(AbilityExec::HostStream(HostStreamExec {
            host_socket: socket.to_string(),
            function: function.to_string(),
        }))
        .unwrap()
    }

    fn host_stream_install(store_dir: &std::path::Path, socket: &str) -> DeviceAbilityInstall {
        let manifest = host_stream_manifest(socket, "er.generate");
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
        let rt = LocalRuntime::new();
        let catalog = Arc::new(AxonAbilityCatalog::new_with_runtime(Arc::clone(&rt)));
        registrar.set_runtime(Arc::clone(&rt)).unwrap();
        registrar
            .set_control_plane_catalog(Arc::downgrade(&catalog))
            .unwrap();
        (registrar, rt, catalog)
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
        let rt = LocalRuntime::new();
        let catalog = Arc::new(AxonAbilityCatalog::new_with_runtime(Arc::clone(&rt)));
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
        let rt = LocalRuntime::new();
        let catalog = Arc::new(AxonAbilityCatalog::new_with_runtime(Arc::clone(&rt)));
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
        let content_hash = crate::runtime::agents::device_ability_store::manifest_digest(
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
        let rt2 = LocalRuntime::new();
        let catalog2 = Arc::new(AxonAbilityCatalog::new_with_runtime(Arc::clone(&rt2)));
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
        let rt2 = LocalRuntime::new();
        let catalog2 = Arc::new(AxonAbilityCatalog::new_with_runtime(Arc::clone(&rt2)));
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
