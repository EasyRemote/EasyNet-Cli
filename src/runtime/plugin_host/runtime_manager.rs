// EasyNet CLI — daemon plugin runtime manager
// ===========================================
//
// File: src/runtime/plugin_host/runtime_manager.rs
// Description: Owns the default plugin package/load/runtime pipeline.

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use crate::runtime::ability_dispatch::AxonAbilityCatalog;
use crate::runtime::ability_wire::AbilityWireRegistry;
use crate::runtime::plugin_host::errors::{PluginHostError, Result};
use crate::runtime::plugin_host::host_api::{PluginHotReloadReport, PluginRuntimeHost};
use crate::runtime::plugin_host::index::{PluginPackageIndex, PluginPackageIndexError};
use crate::runtime::plugin_host::load_plan::{PluginLoadPlan, PluginLoadPlanner};
use crate::runtime::plugin_host::surface::{PluginAbilitySurfaceRecord, PluginSurfaceProjector};

/// Snapshot of package index and load-plan state for one daemon profile.
///
/// Invariant 1: `load_plan` was produced from `index` by the same planner.
/// Invariant 2: descriptor projection reads `index`; invocation reads
/// `load_plan` plus `AxonAbilityCatalog`.
#[derive(Clone)]
pub struct PluginRuntimeState {
    index: PluginPackageIndex,
    load_plan: PluginLoadPlan,
    index_errors: Vec<PluginPackageIndexError>,
}

impl PluginRuntimeState {
    /// Construct a state snapshot from an already-loaded index.
    pub fn from_index(index: PluginPackageIndex) -> Self {
        let load_plan = PluginLoadPlanner::current().plan(&index);
        Self {
            index,
            load_plan,
            index_errors: Vec::new(),
        }
    }

    /// Construct a state snapshot from an already-loaded index and explicit
    /// planner.
    pub fn from_index_with_planner(index: PluginPackageIndex, planner: PluginLoadPlanner) -> Self {
        let load_plan = planner.plan(&index);
        Self {
            index,
            load_plan,
            index_errors: Vec::new(),
        }
    }

    /// Load the default builtin + installed package index. Builtin package
    /// failures remain hard errors; installed package failures are retained as
    /// operator-visible index errors so the daemon can still boot builtin
    /// plugin abilities.
    pub fn load_default() -> Result<Self> {
        let report = PluginPackageIndex::load_default_resilient()?;
        let (index, index_errors) = report.into_parts();
        let load_plan = PluginLoadPlanner::current().plan(&index);
        Ok(Self {
            index,
            load_plan,
            index_errors,
        })
    }

    /// Package index used for descriptor projection.
    pub fn index(&self) -> &PluginPackageIndex {
        &self.index
    }

    /// Runtime load plan used for registration.
    pub fn load_plan(&self) -> &PluginLoadPlan {
        &self.load_plan
    }

    /// Installed package rows that were skipped during index construction.
    pub fn index_errors(&self) -> &[PluginPackageIndexError] {
        &self.index_errors
    }
}

/// Daemon-owned plugin runtime manager.
///
/// What this is NOT: a plugin installer. Install/remove/update mutate the
/// package store; this manager observes that store, computes a load plan, and
/// reconciles registered abilities against the daemon runtime catalogue.
pub struct PluginRuntimeManager {
    state: RwLock<std::result::Result<PluginRuntimeState, String>>,
    runtime_host: PluginRuntimeHost,
    wire_registry: Arc<AbilityWireRegistry>,
}

impl PluginRuntimeManager {
    /// Construct an empty manager. State is loaded lazily so daemon metadata
    /// queries do not panic if package indexing fails.
    pub fn new() -> Self {
        let loaded = PluginRuntimeState::load_default();
        let wire_registry = loaded
            .as_ref()
            .map(AbilityWireRegistry::from_plugin_runtime_state)
            .unwrap_or_else(|_| AbilityWireRegistry::core());
        Self {
            state: RwLock::new(loaded.map_err(|err| err.to_string())),
            runtime_host: PluginRuntimeHost::new(),
            wire_registry: Arc::new(wire_registry),
        }
    }

    /// Construct a manager from an already-computed state snapshot.
    ///
    /// What this is NOT: the daemon default manager. Descriptor generation uses
    /// this constructor with an empty package index so system ability rendering
    /// cannot observe user-local installed plugin state under `$HOME`.
    pub fn from_state(state: PluginRuntimeState) -> Self {
        let wire_registry = AbilityWireRegistry::from_plugin_runtime_state(&state);
        Self {
            state: RwLock::new(Ok(state)),
            runtime_host: PluginRuntimeHost::new(),
            wire_registry: Arc::new(wire_registry),
        }
    }

    /// Register the current default load plan into the daemon catalog.
    pub fn register_default_plugins(&self, reg: &mut AxonAbilityCatalog) -> Result<()> {
        let state = PluginRuntimeState::load_default()?;
        self.runtime_host.register(state.load_plan(), reg)?;
        self.wire_registry.replace_from_plugin_runtime_state(&state);
        *self
            .state
            .write()
            .expect("plugin runtime manager state poisoned") = Ok(state);
        Ok(())
    }

    /// Register the manager's current load plan into a catalog.
    ///
    /// Unlike `register_default_plugins`, this does not reload `$HOME` package
    /// state. It is used by deterministic helpers that construct an explicit
    /// builtin-only state and then need the same runtime registration path as
    /// the daemon.
    pub fn register_current_plugins(&self, reg: &mut AxonAbilityCatalog) -> Result<()> {
        let state = self.state()?;
        self.runtime_host.register(state.load_plan(), reg)?;
        self.wire_registry.replace_from_plugin_runtime_state(&state);
        Ok(())
    }

    /// Reload package/load state and reconcile installed runtime abilities.
    pub fn reload_default_plugins(
        &self,
        reg: &AxonAbilityCatalog,
    ) -> Result<PluginHotReloadReport> {
        let state = PluginRuntimeState::load_default()?;
        let report = self.runtime_host.hot_reload(state.load_plan(), reg)?;
        self.wire_registry.replace_from_plugin_runtime_state(&state);
        *self
            .state
            .write()
            .expect("plugin runtime manager state poisoned") = Ok(state);
        Ok(report)
    }

    /// Return the latest known state snapshot.
    pub fn state(&self) -> Result<PluginRuntimeState> {
        self.state
            .read()
            .expect("plugin runtime manager state poisoned")
            .clone()
            .map_err(PluginHostError::DefaultIndexUnavailable)
    }

    /// Project plugin rows using actual daemon runtime catalog state.
    pub fn daemon_surface_rows(
        &self,
        reg: &AxonAbilityCatalog,
    ) -> Result<Vec<PluginAbilitySurfaceRecord>> {
        let state = self.state()?;
        let abilities = reg.list_abilities().into_iter().collect::<BTreeSet<_>>();
        Ok(PluginSurfaceProjector::project_with_daemon(
            state.index(),
            state.load_plan(),
            Some(&abilities),
            state.index_errors(),
        ))
    }

    /// Shared daemon-local bidi wire profile registry.
    ///
    /// The handle is stable for the daemon lifetime. Reload mutates its
    /// internal plugin snapshot so gRPC and `<self>.session` dispatchers observe
    /// the same plugin load state as `AxonAbilityCatalog` without restarting.
    pub fn ability_wire_registry(&self) -> Arc<AbilityWireRegistry> {
        Arc::clone(&self.wire_registry)
    }
}

impl Default for PluginRuntimeManager {
    fn default() -> Self {
        Self::new()
    }
}
