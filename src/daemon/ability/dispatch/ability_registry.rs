//! Authority-keyed registry for executable daemon ability bindings.
//!
//! Descriptor metadata remains in the control-plane registry and invocation,
//! admission, and receipt finalization remain in their owning runtime layers.
//! This object owns only handler slots, external runtime mode bindings, and
//! static/dynamic lifecycle origin.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    AbilityCallModes, ControlPlaneAbilityKey, DescriptorCallMode, DynamicAbilitySnapshot,
    DynamicRegistration, HandlerSlotKind, LocalBidiHandler, LocalBidiHandlerWithEnvelope,
    LocalRpcHandler, LocalRpcHandlerWithEnvelope, LocalStreamHandler,
    LocalStreamHandlerWithEnvelope, RuntimeHandlerSet, StaticRegistrationHandler,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ExecutionOrigin {
    Static,
    Dynamic,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ExecutionIndexCounts {
    pub(super) rpc: usize,
    pub(super) stream: usize,
    pub(super) bidi: usize,
    pub(super) rpc_with_env: usize,
    pub(super) stream_with_env: usize,
    pub(super) bidi_with_env: usize,
}

impl ExecutionIndexCounts {
    pub(super) fn total(self) -> usize {
        self.rpc
            + self.stream
            + self.bidi
            + self.rpc_with_env
            + self.stream_with_env
            + self.bidi_with_env
    }
}

#[derive(Clone)]
struct ExecutionIndexEntry {
    origin: ExecutionOrigin,
    handlers: RuntimeHandlerSet,
    external_runtime_modes: AbilityCallModes,
}

/// Authority-keyed execution binding index for daemon ability implementations.
///
/// The index is deliberately not a descriptor metadata table. Local ability
/// handlers store executable closures and their lifecycle origin. Daemon
/// Invocation exact routes store only an external runtime mode bit: their
/// execution closure lives in `DaemonInvocationService`, but the catalog still
/// needs an explicit implementation binding fact so publication reads do not
/// report executable authority abilities as `unbound`.
///
/// The authority key mirrors the Axon runtime binding boundary: one
/// `(authority_root, ability)` row can carry RPC, stream, and bidi bindings,
/// while the control-plane registry remains the source for descriptor version,
/// schema hash, owner, and call-mode proofs.
#[derive(Default)]
pub(super) struct AbilityRegistry {
    entries: BTreeMap<ControlPlaneAbilityKey, ExecutionIndexEntry>,
}

fn empty_runtime_modes() -> AbilityCallModes {
    AbilityCallModes {
        rpc: false,
        stream: false,
        bidi: false,
    }
}

fn runtime_modes_contain(modes: AbilityCallModes, call_mode: DescriptorCallMode) -> bool {
    match call_mode {
        DescriptorCallMode::Rpc => modes.rpc,
        DescriptorCallMode::Stream => modes.stream,
        DescriptorCallMode::Bidi => modes.bidi,
    }
}

fn runtime_modes_insert(modes: &mut AbilityCallModes, call_mode: DescriptorCallMode) {
    match call_mode {
        DescriptorCallMode::Rpc => modes.rpc = true,
        DescriptorCallMode::Stream => modes.stream = true,
        DescriptorCallMode::Bidi => modes.bidi = true,
    }
}

fn runtime_modes_remove(modes: &mut AbilityCallModes, call_mode: DescriptorCallMode) {
    match call_mode {
        DescriptorCallMode::Rpc => modes.rpc = false,
        DescriptorCallMode::Stream => modes.stream = false,
        DescriptorCallMode::Bidi => modes.bidi = false,
    }
}

fn runtime_modes_any(modes: AbilityCallModes) -> bool {
    modes.rpc || modes.stream || modes.bidi
}

impl ExecutionIndexEntry {
    pub(super) fn has_handlers(&self) -> bool {
        !self.handlers.is_empty()
    }

    pub(super) fn bound_modes(&self) -> AbilityCallModes {
        let handler_modes = self.handlers.modes();
        AbilityCallModes {
            rpc: handler_modes.rpc || self.external_runtime_modes.rpc,
            stream: handler_modes.stream || self.external_runtime_modes.stream,
            bidi: handler_modes.bidi || self.external_runtime_modes.bidi,
        }
    }

    pub(super) fn is_runtime_bound_for_mode(&self, call_mode: DescriptorCallMode) -> bool {
        runtime_modes_contain(self.bound_modes(), call_mode)
    }
}

impl AbilityRegistry {
    pub(super) fn counts(&self, origin: ExecutionOrigin) -> ExecutionIndexCounts {
        let mut counts = ExecutionIndexCounts::default();
        for entry in self.entries.values().filter(|entry| entry.origin == origin) {
            let entry_counts = entry.handlers.counts();
            counts.rpc += entry_counts.rpc;
            counts.stream += entry_counts.stream;
            counts.bidi += entry_counts.bidi;
            counts.rpc_with_env += entry_counts.rpc_with_env;
            counts.stream_with_env += entry_counts.stream_with_env;
            counts.bidi_with_env += entry_counts.bidi_with_env;
        }
        counts
    }

    pub(super) fn dynamic_snapshot(&self, key: &ControlPlaneAbilityKey) -> DynamicAbilitySnapshot {
        self.entries
            .get(key)
            .filter(|entry| entry.origin == ExecutionOrigin::Dynamic)
            .map(|entry| DynamicAbilitySnapshot::from_handlers(entry.handlers.clone()))
            .unwrap_or_default()
    }

    pub(super) fn restore_dynamic(
        &mut self,
        key: ControlPlaneAbilityKey,
        snapshot: DynamicAbilitySnapshot,
    ) {
        self.drain_dynamic(&key);
        if snapshot.has_handlers() {
            self.entries.insert(
                key,
                ExecutionIndexEntry {
                    origin: ExecutionOrigin::Dynamic,
                    handlers: snapshot.into_handlers(),
                    external_runtime_modes: empty_runtime_modes(),
                },
            );
        }
    }

    pub(super) fn install_static(
        &mut self,
        key: ControlPlaneAbilityKey,
        handler: StaticRegistrationHandler,
    ) {
        let entry = self
            .entries
            .entry(key)
            .or_insert_with(|| ExecutionIndexEntry {
                origin: ExecutionOrigin::Static,
                handlers: RuntimeHandlerSet::default(),
                external_runtime_modes: empty_runtime_modes(),
            });
        assert_eq!(
            entry.origin,
            ExecutionOrigin::Static,
            "static registration attempted to overwrite a dynamic execution row"
        );
        entry.handlers.install_static(handler);
    }

    pub(super) fn install_external_static_binding(
        &mut self,
        key: ControlPlaneAbilityKey,
        call_mode: DescriptorCallMode,
    ) {
        let entry = self
            .entries
            .entry(key)
            .or_insert_with(|| ExecutionIndexEntry {
                origin: ExecutionOrigin::Static,
                handlers: RuntimeHandlerSet::default(),
                external_runtime_modes: empty_runtime_modes(),
            });
        assert_eq!(
            entry.origin,
            ExecutionOrigin::Static,
            "external daemon-invocation binding attempted to overwrite a dynamic execution row"
        );
        runtime_modes_insert(&mut entry.external_runtime_modes, call_mode);
    }

    pub(super) fn install_external_dynamic_binding(
        &mut self,
        key: ControlPlaneAbilityKey,
        call_mode: DescriptorCallMode,
    ) -> anyhow::Result<()> {
        let entry = self
            .entries
            .entry(key)
            .or_insert_with(|| ExecutionIndexEntry {
                origin: ExecutionOrigin::Dynamic,
                handlers: RuntimeHandlerSet::default(),
                external_runtime_modes: empty_runtime_modes(),
            });
        if entry.origin != ExecutionOrigin::Dynamic {
            anyhow::bail!(
                "external dynamic runtime binding attempted to overwrite a static execution row"
            );
        }
        if entry.has_handlers() {
            anyhow::bail!(
                "external dynamic runtime binding conflicts with a catalog-owned dynamic handler"
            );
        }
        runtime_modes_insert(&mut entry.external_runtime_modes, call_mode);
        Ok(())
    }

    pub(super) fn remove_external_dynamic_binding(
        &mut self,
        key: &ControlPlaneAbilityKey,
        call_mode: DescriptorCallMode,
    ) -> bool {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        if entry.origin != ExecutionOrigin::Dynamic
            || !runtime_modes_contain(entry.external_runtime_modes, call_mode)
        {
            return false;
        }
        runtime_modes_remove(&mut entry.external_runtime_modes, call_mode);
        if !entry.has_handlers() && !runtime_modes_any(entry.external_runtime_modes) {
            self.entries.remove(key);
        }
        true
    }

    pub(super) fn has_external_dynamic_binding(
        &self,
        key: &ControlPlaneAbilityKey,
        call_mode: DescriptorCallMode,
    ) -> bool {
        self.entries.get(key).is_some_and(|entry| {
            entry.origin == ExecutionOrigin::Dynamic
                && runtime_modes_contain(entry.external_runtime_modes, call_mode)
        })
    }

    pub(super) fn install_dynamic(
        &mut self,
        key: ControlPlaneAbilityKey,
        registration: DynamicRegistration,
    ) {
        let DynamicRegistration {
            ability: _,
            owner: _,
            authority_scope: _,
            manifest: _,
            receipt_semantics: _,
            implementation: _,
            handler,
        } = registration;
        let call_mode = handler.call_mode();
        let entry = self
            .entries
            .entry(key)
            .or_insert_with(|| ExecutionIndexEntry {
                origin: ExecutionOrigin::Dynamic,
                handlers: RuntimeHandlerSet::default(),
                external_runtime_modes: empty_runtime_modes(),
            });
        assert_eq!(
            entry.origin,
            ExecutionOrigin::Dynamic,
            "dynamic registration attempted to overwrite a static execution row"
        );
        entry.handlers.remove_mode(call_mode);
        entry.handlers.install_dynamic(handler);
    }

    pub(super) fn drain_static(&mut self, key: &ControlPlaneAbilityKey) -> bool {
        self.drain_origin(key, ExecutionOrigin::Static)
    }

    pub(super) fn drain_dynamic(&mut self, key: &ControlPlaneAbilityKey) -> bool {
        self.drain_origin(key, ExecutionOrigin::Dynamic)
    }

    pub(super) fn drain_origin(
        &mut self,
        key: &ControlPlaneAbilityKey,
        origin: ExecutionOrigin,
    ) -> bool {
        let present = self
            .entries
            .get(key)
            .map(|entry| entry.origin == origin && entry.has_handlers())
            .unwrap_or(false);
        if present {
            self.entries.remove(key);
        }
        present
    }

    pub(super) fn contains_origin_handler_by_name(
        &self,
        ability: &str,
        origin: ExecutionOrigin,
    ) -> bool {
        self.entries.iter().any(|(key, entry)| {
            key.ability() == ability && entry.origin == origin && entry.has_handlers()
        })
    }

    pub(super) fn origin_key_by_ability(
        &self,
        ability: &str,
        origin: ExecutionOrigin,
    ) -> anyhow::Result<Option<ControlPlaneAbilityKey>> {
        let keys = self
            .entries
            .iter()
            .filter(|(key, entry)| {
                key.ability() == ability && entry.origin == origin && entry.has_handlers()
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        match keys.as_slice() {
            [] => Ok(None),
            [key] => Ok(Some(key.clone())),
            _ => anyhow::bail!(
                "ability {ability:?} has multiple {origin:?} execution authority keys {keys:?}"
            ),
        }
    }

    pub(super) fn slots(&self, key: &ControlPlaneAbilityKey) -> Vec<HandlerSlotKind> {
        self.entries
            .get(key)
            .map(|entry| entry.handlers.slots())
            .unwrap_or_default()
    }

    pub(super) fn names(&self, origin: ExecutionOrigin) -> Vec<String> {
        let mut names = BTreeSet::new();
        for (key, entry) in &self.entries {
            if entry.origin == origin && entry.has_handlers() {
                names.insert(key.ability().to_string());
            }
        }
        names.into_iter().collect()
    }

    pub(super) fn static_rows_for_ability(
        &self,
        ability: &str,
    ) -> Vec<(ControlPlaneAbilityKey, RuntimeHandlerSet)> {
        self.entries
            .iter()
            .filter(|(key, entry)| {
                key.ability() == ability
                    && entry.origin == ExecutionOrigin::Static
                    && entry.has_handlers()
            })
            .map(|(key, entry)| (key.clone(), entry.handlers.clone()))
            .collect()
    }

    pub(super) fn handlers_for_key(&self, key: &ControlPlaneAbilityKey) -> RuntimeHandlerSet {
        self.entries
            .get(key)
            .map(|entry| entry.handlers.clone())
            .unwrap_or_default()
    }

    pub(super) fn resolve_rpc_for_key(
        &self,
        key: &ControlPlaneAbilityKey,
    ) -> Option<LocalRpcHandler> {
        self.entries
            .get(key)
            .and_then(|entry| entry.handlers.resolve_rpc())
    }

    pub(super) fn unique_handler_slot<T>(
        &self,
        ability: &str,
        extract: impl Fn(&RuntimeHandlerSet) -> Option<T>,
    ) -> Option<T> {
        let mut matches = self
            .entries
            .iter()
            .filter(|(key, entry)| key.ability() == ability && entry.has_handlers())
            .filter_map(|(_, entry)| extract(&entry.handlers));
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first)
    }

    pub(super) fn unique_mode_registered(
        &self,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> bool {
        let mut matches = self
            .entries
            .iter()
            .filter(|(key, entry)| key.ability() == ability && entry.has_handlers())
            .filter(|(_, entry)| runtime_modes_contain(entry.handlers.modes(), call_mode));
        matches.next().is_some() && matches.next().is_none()
    }

    pub(super) fn has_mode(&self, ability: &str, call_mode: DescriptorCallMode) -> bool {
        self.unique_mode_registered(ability, call_mode)
    }

    pub(super) fn has_mode_for_authority(
        &self,
        authority_root: &str,
        ability: &str,
        call_mode: DescriptorCallMode,
    ) -> bool {
        let key = ControlPlaneAbilityKey::new(authority_root, ability);
        self.entries
            .get(&key)
            .is_some_and(|entry| entry.is_runtime_bound_for_mode(call_mode))
    }

    pub(super) fn has_any_handler(&self, ability: &str) -> bool {
        self.entries
            .iter()
            .any(|(key, entry)| key.ability() == ability && entry.has_handlers())
    }

    pub(super) fn resolve_rpc(&self, ability: &str) -> Option<LocalRpcHandler> {
        self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_rpc)
    }

    pub(super) fn resolve_stream(&self, ability: &str) -> Option<LocalStreamHandler> {
        self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_stream)
    }

    pub(super) fn resolve_stream_with_env(
        &self,
        ability: &str,
    ) -> Option<LocalStreamHandlerWithEnvelope> {
        self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_stream_with_env)
    }

    pub(super) fn resolve_bidi(&self, ability: &str) -> Option<LocalBidiHandler> {
        self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_bidi)
    }

    pub(super) fn resolve_bidi_with_env(
        &self,
        ability: &str,
    ) -> Option<LocalBidiHandlerWithEnvelope> {
        self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_bidi_with_env)
    }

    pub(super) fn resolve_rpc_with_env(
        &self,
        ability: &str,
    ) -> Option<LocalRpcHandlerWithEnvelope> {
        self.unique_handler_slot(ability, RuntimeHandlerSet::resolve_rpc_with_env)
    }
}
