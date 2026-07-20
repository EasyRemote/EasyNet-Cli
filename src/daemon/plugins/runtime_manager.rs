// EasyNet CLI — daemon plugin runtime manager
// ===========================================
//
// File: src/daemon/plugins/runtime_manager.rs
// Description: Owns the default plugin package/load/runtime pipeline.

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::ability::wire::AbilityWireRegistry;
use crate::daemon::plugins::contribution::DaemonPluginBinder;
use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::host_api::{
    PluginHotReloadReport, PluginRealtimeActivationHint, PluginRuntimeHost,
};
use crate::daemon::plugins::index::{PluginPackageIndex, PluginPackageIndexError};
use crate::daemon::plugins::load_plan::{PluginLoadPlan, PluginLoadPlanner};
use crate::daemon::plugins::realtime::activation_plans_for_manifest;
use crate::daemon::plugins::surface::{PluginSurfaceProjector, PluginSurfaceReport};

/// Snapshot of package index and load-plan state for one daemon profile.
///
/// Invariant 1: `load_plan` was produced from `index` by the same planner.
/// Invariant 2: descriptor projection reads `index`; runtime binding reads
/// `load_plan` and applies contributions through the daemon binder.
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
    /// Construct the daemon default manager from the canonical plugin runtime
    /// snapshot.
    ///
    /// Reads through the shared default-state snapshot (F-050): on a cold
    /// process this is the one boot-time disk read that also primes the
    /// snapshot for every later catalog reader.
    ///
    /// Failure is terminal for daemon assembly. The runtime catalog and bidi
    /// wire registry are two projections of the same plugin state; booting with
    /// a core-only wire registry after package-state failure makes product
    /// routes disappear as late `ABILITY_NOT_FOUND`/`No route visible` errors.
    pub fn new() -> Result<Self> {
        let loaded = super::default_state().map(|state| (*state).clone())?;
        let wire_registry = AbilityWireRegistry::from_plugin_runtime_state(&loaded);
        Ok(Self {
            state: RwLock::new(Ok(loaded)),
            runtime_host: PluginRuntimeHost::new(),
            wire_registry: Arc::new(wire_registry),
        })
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
        self.register_state_plugins(&state, reg)?;
        self.wire_registry.replace_from_plugin_runtime_state(&state);
        super::publish_default_state(&state);
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
        self.register_state_plugins(&state, reg)?;
        self.wire_registry.replace_from_plugin_runtime_state(&state);
        Ok(())
    }

    /// Reload package/load state and reconcile installed runtime abilities.
    pub fn reload_default_plugins(
        &self,
        reg: &AxonAbilityCatalog,
    ) -> Result<PluginHotReloadReport> {
        let state = PluginRuntimeState::load_default()?;
        let report = self.reload_plugins_from_state(state.clone(), reg)?;
        self.wire_registry.replace_from_plugin_runtime_state(&state);
        super::publish_default_state(&state);
        *self
            .state
            .write()
            .expect("plugin runtime manager state poisoned") = Ok(state);
        Ok(report)
    }

    pub(crate) fn reload_plugins_from_state(
        &self,
        state: PluginRuntimeState,
        reg: &AxonAbilityCatalog,
    ) -> Result<PluginHotReloadReport> {
        let report = self.reconcile_runtime_plugins(&state, reg)?;
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

    /// Project plugin packages and abilities using actual daemon runtime state.
    pub fn daemon_surface_report(&self, reg: &AxonAbilityCatalog) -> Result<PluginSurfaceReport> {
        let state = self.state()?;
        let abilities = reg.list_abilities().into_iter().collect::<BTreeSet<_>>();
        Ok(PluginSurfaceProjector::project_report_with_daemon(
            state.index(),
            state.load_plan(),
            Some(&abilities),
            state.index_errors(),
        ))
    }

    /// Shared daemon-local bidi wire profile registry.
    ///
    /// The handle is stable for the daemon lifetime. Reload mutates its
    /// internal plugin snapshot so gRPC and `session.open` dispatchers observe
    /// the same plugin load state as `AxonAbilityCatalog` without restarting.
    pub fn ability_wire_registry(&self) -> Arc<AbilityWireRegistry> {
        Arc::clone(&self.wire_registry)
    }

    fn register_state_plugins(
        &self,
        state: &PluginRuntimeState,
        reg: &mut AxonAbilityCatalog,
    ) -> Result<()> {
        let existing = reg.list_abilities().into_iter().collect::<BTreeSet<_>>();
        for entry in state.load_plan().entries() {
            if entry.is_loaded() {
                reject_catalog_collisions(entry.package(), &existing)?;
            }
        }

        let contributions = self
            .runtime_host
            .collect_boot_contributions(state.load_plan())?;
        {
            DaemonPluginBinder::static_catalog(reg).bind_set(contributions.builtin())?;
        }
        DaemonPluginBinder::dynamic_catalog(reg).bind_set(contributions.runtime())?;
        self.runtime_host.replace_tracked_runtime_abilities(
            self.runtime_host.runtime_ability_names(state.load_plan()),
        );
        Ok(())
    }

    fn reconcile_runtime_plugins(
        &self,
        state: &PluginRuntimeState,
        reg: &AxonAbilityCatalog,
    ) -> Result<PluginHotReloadReport> {
        let mut report = PluginHotReloadReport::default();
        let current = self.runtime_host.runtime_ability_names(state.load_plan());
        let contributions = self
            .runtime_host
            .collect_runtime_contributions(state.load_plan())?;
        let static_abilities = reg
            .static_ability_names()
            .into_iter()
            .collect::<BTreeSet<_>>();

        for entry in state.load_plan().entries() {
            if !entry.is_loaded() || entry.package().builtin_binding().is_some() {
                continue;
            }
            reject_static_catalog_collisions(entry.package(), &static_abilities)?;
        }

        DaemonPluginBinder::dynamic_catalog(reg).bind_set(&contributions)?;
        let daemon_abilities = reg.list_abilities().into_iter().collect::<BTreeSet<_>>();

        for entry in state.load_plan().entries() {
            if !entry.is_loaded() || entry.package().builtin_binding().is_some() {
                continue;
            }
            let package = entry.package();
            report.loaded_packages.push(format!(
                "{}@{}",
                package.id().as_str(),
                package.version().as_str()
            ));
            let activation_plans = activation_plans_for_manifest(
                package.id().as_str(),
                package.version().as_str(),
                package.manifest(),
                Some(&daemon_abilities),
            );
            let quick_add_plans = activation_plans
                .iter()
                .filter(|plan| plan.is_quick_add())
                .cloned()
                .collect::<Vec<_>>();
            if !quick_add_plans.is_empty() {
                let quick_add_capabilities = quick_add_plans
                    .iter()
                    .map(|plan| plan.capability.clone())
                    .collect::<Vec<_>>();
                report
                    .realtime_activation_hints
                    .push(PluginRealtimeActivationHint {
                        package_id: package.id().as_str().to_string(),
                        package_version: package.version().as_str().to_string(),
                        capabilities: quick_add_capabilities,
                        activation_plans: quick_add_plans,
                    });
            }
            report.realtime_activation_plans.extend(activation_plans);
        }

        let stale = self
            .runtime_host
            .tracked_runtime_abilities()
            .difference(&current)
            .cloned()
            .collect::<Vec<String>>();
        for ability in stale {
            if reg.hot_unregister(&ability).map_err(|error| {
                PluginHostError::ControlPlaneRegistrationFailed {
                    ability: ability.clone(),
                    reason: error.to_string(),
                }
            })? {
                report.unregistered_abilities.push(ability);
            }
        }
        self.runtime_host
            .replace_tracked_runtime_abilities(current.clone());
        report.registered_abilities = current.into_iter().collect();
        sort_hot_reload_report(&mut report);
        Ok(report)
    }
}

fn reject_catalog_collisions(
    package: &crate::daemon::plugins::package::SharedPluginPackage,
    existing: &BTreeSet<String>,
) -> Result<()> {
    let second = format!("{}@{}", package.id().as_str(), package.version().as_str());
    for ability in package.manifest().abilities() {
        if existing.contains(ability.name()) {
            return Err(PluginHostError::DuplicateAbilityOwner {
                ability: ability.name().to_string(),
                first: "daemon-catalog".to_string(),
                second: second.clone(),
            });
        }
    }
    Ok(())
}

fn reject_static_catalog_collisions(
    package: &crate::daemon::plugins::package::SharedPluginPackage,
    static_abilities: &BTreeSet<String>,
) -> Result<()> {
    let second = format!("{}@{}", package.id().as_str(), package.version().as_str());
    for ability in package.manifest().abilities() {
        if static_abilities.contains(ability.name()) {
            return Err(PluginHostError::DuplicateAbilityOwner {
                ability: ability.name().to_string(),
                first: "daemon-static-catalog".to_string(),
                second: second.clone(),
            });
        }
    }
    Ok(())
}

fn sort_hot_reload_report(report: &mut PluginHotReloadReport) {
    report.loaded_packages.sort();
    report.registered_abilities.sort();
    report.unregistered_abilities.sort();
    report.realtime_activation_hints.sort_by(|a, b| {
        a.package_id
            .cmp(&b.package_id)
            .then(a.package_version.cmp(&b.package_version))
    });
    report.realtime_activation_plans.sort_by(|a, b| {
        a.package_id
            .cmp(&b.package_id)
            .then(a.package_version.cmp(&b.package_version))
            .then(format!("{:?}", a.capability.kind()).cmp(&format!("{:?}", b.capability.kind())))
    });
}
