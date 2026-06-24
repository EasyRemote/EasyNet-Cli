// EasyNet CLI — plugin runtime host API
// =====================================
//
// File: src/runtime/plugin_host/host_api.rs
// Description: Register loaded plugin abilities into AxonAbilityCatalog.

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use serde::Serialize;
use serde_json::Value;

use crate::core::ability_spec::{EalExec, McpExec};
use crate::runtime::ability::{AbilityImplSource, RuntimeEnv};
use crate::runtime::ability_dispatch::{
    AxonAbilityCatalog, ControlPlaneImplementation, EnvelopeContext, LocalBidiHandlerWithEnvelope,
    LocalRpcHandlerWithEnvelope, LocalStreamHandlerWithEnvelope, OwnerKind,
};
use crate::runtime::context::ParentInvocationContext;
use crate::runtime::plugin_host::errors::{PluginHostError, Result};
use crate::runtime::plugin_host::load_plan::PluginLoadPlan;
use crate::runtime::plugin_host::manifest::{PluginCallMode, PluginDeclarativeBinding, PluginKind};
use crate::runtime::plugin_host::sidecar::{
    sidecar_invocation_from_context, SidecarCommand, SidecarRuntimeHost,
};

/// Runtime host for daemon plugin packages.
///
/// What this is NOT: an installer or package index. It consumes a load plan and
/// registers only packages that the planner marked as loaded.
#[derive(Clone, Default)]
pub struct PluginRuntimeHost {
    tracked_runtime_abilities: Arc<RwLock<BTreeSet<String>>>,
}

/// Result of a daemon plugin runtime refresh.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PluginHotReloadReport {
    pub loaded_packages: Vec<String>,
    pub registered_abilities: Vec<String>,
    pub unregistered_abilities: Vec<String>,
}

impl PluginRuntimeHost {
    /// Construct a plugin runtime host.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register every loaded package into the ability catalog.
    pub fn register(&self, load_plan: &PluginLoadPlan, reg: &mut AxonAbilityCatalog) -> Result<()> {
        let existing: BTreeSet<String> = reg.list_abilities().into_iter().collect();
        for entry in load_plan.entries() {
            if !entry.is_loaded() {
                continue;
            }
            let package = entry.package();
            reject_catalog_collisions(package, &existing)?;
            match package.manifest().kind() {
                PluginKind::Builtin => {
                    let binding = package.builtin_binding().ok_or_else(|| {
                        PluginHostError::MissingBuiltinBinding(package.id().as_str().to_string())
                    })?;
                    (binding.register)(reg, package.manifest().limits());
                }
                PluginKind::Sidecar => {
                    let command = SidecarCommand::from_package(package);
                    let mut sink = CatalogRegistrationSink::dynamic(
                        reg,
                        package,
                        AbilityImplSource::SidecarPlugin,
                    );
                    register_json_frame_process_package(package, &mut sink, command)?;
                    self.remember_runtime_plugin_package(package);
                }
                PluginKind::Declarative => {
                    let impl_source =
                        declarative_impl_source(package.manifest().declarative_binding());
                    let mut sink = CatalogRegistrationSink::dynamic(reg, package, impl_source);
                    register_declarative_package(package, &mut sink)?;
                    self.remember_runtime_plugin_package(package);
                }
            }
        }
        Ok(())
    }

    /// Hot-register loaded plugin packages into the catalogue's dynamic side.
    ///
    /// This is the daemon reload path after an install/update transaction. It
    /// never mutates the static boot maps; static abilities continue to win on
    /// name collision, and dynamic rows can be removed with
    /// [`AxonAbilityCatalog::hot_unregister`].
    pub fn hot_register(&self, load_plan: &PluginLoadPlan, reg: &AxonAbilityCatalog) -> Result<()> {
        for entry in load_plan.entries() {
            if !entry.is_loaded() {
                continue;
            }
            let package = entry.package();
            match package.manifest().kind() {
                PluginKind::Builtin => {}
                PluginKind::Sidecar => {
                    reject_static_catalog_collisions(package, reg)?;
                    let command = SidecarCommand::from_package(package);
                    let mut sink = CatalogRegistrationSink::dynamic(
                        reg,
                        package,
                        AbilityImplSource::SidecarPlugin,
                    );
                    register_json_frame_process_package(package, &mut sink, command)?;
                    self.remember_runtime_plugin_package(package);
                }
                PluginKind::Declarative => {
                    reject_static_catalog_collisions(package, reg)?;
                    let impl_source =
                        declarative_impl_source(package.manifest().declarative_binding());
                    let mut sink = CatalogRegistrationSink::dynamic(reg, package, impl_source);
                    register_declarative_package(package, &mut sink)?;
                    self.remember_runtime_plugin_package(package);
                }
            }
        }
        Ok(())
    }

    /// Reconcile the daemon runtime with a freshly computed plugin load plan.
    ///
    /// Builtin plugins are intentionally skipped: they are compiled into the
    /// daemon and cannot be installed or removed by package transactions. This
    /// method owns only installed sidecar/declarative packages.
    pub fn hot_reload(
        &self,
        load_plan: &PluginLoadPlan,
        reg: &AxonAbilityCatalog,
    ) -> Result<PluginHotReloadReport> {
        let mut report = PluginHotReloadReport::default();
        let current = loaded_runtime_plugin_ability_names(load_plan);

        for entry in load_plan.entries() {
            if !entry.is_loaded() || entry.package().builtin_binding().is_some() {
                continue;
            }
            let package = entry.package();
            report.loaded_packages.push(format!(
                "{}@{}",
                package.id().as_str(),
                package.version().as_str()
            ));
            match package.manifest().kind() {
                PluginKind::Builtin => {}
                PluginKind::Sidecar => {
                    reject_static_catalog_collisions(package, reg)?;
                    let command = SidecarCommand::from_package(package);
                    let mut sink = CatalogRegistrationSink::dynamic(
                        reg,
                        package,
                        AbilityImplSource::SidecarPlugin,
                    );
                    register_json_frame_process_package(package, &mut sink, command)?;
                }
                PluginKind::Declarative => {
                    reject_static_catalog_collisions(package, reg)?;
                    let impl_source =
                        declarative_impl_source(package.manifest().declarative_binding());
                    let mut sink = CatalogRegistrationSink::dynamic(reg, package, impl_source);
                    register_declarative_package(package, &mut sink)?;
                }
            }
        }

        let mut tracked = self
            .tracked_runtime_abilities
            .write()
            .expect("plugin runtime ability tracker poisoned");
        let stale = tracked
            .difference(&current)
            .cloned()
            .collect::<Vec<String>>();
        for ability in stale {
            if reg.hot_remove_runtime_ability(&ability).map_err(|error| {
                PluginHostError::ControlPlaneRegistrationFailed {
                    ability: ability.clone(),
                    reason: error.to_string(),
                }
            })? {
                report.unregistered_abilities.push(ability);
            }
        }
        *tracked = current;
        report.registered_abilities = tracked.iter().cloned().collect();
        report.loaded_packages.sort();
        report.registered_abilities.sort();
        report.unregistered_abilities.sort();
        Ok(report)
    }

    fn remember_runtime_plugin_package(
        &self,
        package: &crate::runtime::plugin_host::package::SharedPluginPackage,
    ) {
        let mut tracked = self
            .tracked_runtime_abilities
            .write()
            .expect("plugin runtime ability tracker poisoned");
        tracked.extend(
            package
                .manifest()
                .abilities()
                .iter()
                .map(|ability| ability.name().to_string()),
        );
    }
}

fn loaded_runtime_plugin_ability_names(load_plan: &PluginLoadPlan) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for entry in load_plan.entries() {
        if !entry.is_loaded() || entry.package().builtin_binding().is_some() {
            continue;
        }
        names.extend(
            entry
                .package()
                .manifest()
                .abilities()
                .iter()
                .map(|ability| ability.name().to_string()),
        );
    }
    names
}

fn reject_catalog_collisions(
    package: &crate::runtime::plugin_host::package::SharedPluginPackage,
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
    package: &crate::runtime::plugin_host::package::SharedPluginPackage,
    reg: &AxonAbilityCatalog,
) -> Result<()> {
    let second = format!("{}@{}", package.id().as_str(), package.version().as_str());
    for ability in package.manifest().abilities() {
        if reg.has_static_ability(ability.name()) {
            return Err(PluginHostError::DuplicateAbilityOwner {
                ability: ability.name().to_string(),
                first: "daemon-static-catalog".to_string(),
                second: second.clone(),
            });
        }
    }
    Ok(())
}

fn declarative_impl_source(binding: Option<&PluginDeclarativeBinding>) -> AbilityImplSource {
    match binding {
        Some(PluginDeclarativeBinding::Eal { .. }) => AbilityImplSource::Eal,
        Some(PluginDeclarativeBinding::Mcp { .. }) => AbilityImplSource::Mcp,
        Some(PluginDeclarativeBinding::Exec { .. }) | None => AbilityImplSource::DeclarativePlugin,
    }
}

struct CatalogRegistrationSink<'a> {
    reg: &'a AxonAbilityCatalog,
    impl_source: AbilityImplSource,
    runtime_env: RuntimeEnv,
}

impl<'a> CatalogRegistrationSink<'a> {
    fn dynamic(
        reg: &'a AxonAbilityCatalog,
        package: &crate::runtime::plugin_host::package::SharedPluginPackage,
        impl_source: AbilityImplSource,
    ) -> Self {
        Self {
            reg,
            impl_source,
            runtime_env: RuntimeEnv::plugin_sidecar(
                package.id().as_str(),
                package.version().as_str(),
            ),
        }
    }

    fn register_rpc(
        &mut self,
        ability: String,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
    ) -> Result<()> {
        self.reg
            .hot_register_rpc_with_envelope_and_spec_and_impl(
                ability.clone(),
                OwnerKind::Device,
                manifest,
                handler,
                ControlPlaneImplementation::new(self.impl_source.clone(), self.runtime_env.clone()),
            )
            .map_err(|error| PluginHostError::ControlPlaneRegistrationFailed {
                ability,
                reason: error.to_string(),
            })?;
        Ok(())
    }

    fn register_stream(
        &mut self,
        ability: String,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalStreamHandlerWithEnvelope,
    ) -> Result<()> {
        self.reg
            .hot_register_stream_with_envelope_and_spec_and_impl(
                ability.clone(),
                OwnerKind::Device,
                manifest,
                handler,
                ControlPlaneImplementation::new(self.impl_source.clone(), self.runtime_env.clone()),
            )
            .map_err(|error| PluginHostError::ControlPlaneRegistrationFailed {
                ability,
                reason: error.to_string(),
            })?;
        Ok(())
    }

    fn register_bidi(
        &mut self,
        ability: String,
        manifest: crate::core::ability_spec::AbilityManifest,
        handler: LocalBidiHandlerWithEnvelope,
    ) -> Result<()> {
        self.reg
            .hot_register_bidi_with_envelope_and_spec_and_impl(
                ability.clone(),
                OwnerKind::Device,
                manifest,
                handler,
                ControlPlaneImplementation::new(self.impl_source.clone(), self.runtime_env.clone()),
            )
            .map_err(|error| PluginHostError::ControlPlaneRegistrationFailed {
                ability,
                reason: error.to_string(),
            })?;
        Ok(())
    }
}

fn register_declarative_package(
    package: &crate::runtime::plugin_host::package::SharedPluginPackage,
    sink: &mut CatalogRegistrationSink<'_>,
) -> Result<()> {
    match package.manifest().declarative_binding() {
        Some(PluginDeclarativeBinding::Exec { argv }) => {
            let command = exec_declarative_command(package, argv)?;
            register_json_frame_process_package(package, sink, command)
        }
        Some(PluginDeclarativeBinding::Eal {
            program,
            result_binding,
        }) => register_eal_declarative_package(package, sink, program, result_binding.clone()),
        Some(PluginDeclarativeBinding::Mcp { server, tool }) => {
            register_mcp_declarative_package(package, sink, server, tool)
        }
        None => Ok(()),
    }
}

fn exec_declarative_command(
    package: &crate::runtime::plugin_host::package::SharedPluginPackage,
    argv: &[String],
) -> Result<SidecarCommand> {
    let Some(program) = argv.first() else {
        return Err(PluginHostError::InvalidDeclarativeBinding {
            id: package.id().as_str().to_string(),
            reason: "exec binding must declare argv[0]".to_string(),
        });
    };
    let program = if std::path::Path::new(program).is_absolute() {
        std::path::PathBuf::from(program)
    } else {
        package.root().join(program)
    };
    let args = argv.iter().skip(1).cloned().collect::<Vec<_>>();
    Ok(SidecarCommand::with_args(program, args))
}

fn register_eal_declarative_package(
    package: &crate::runtime::plugin_host::package::SharedPluginPackage,
    sink: &mut CatalogRegistrationSink<'_>,
    program: &str,
    result_binding: Option<String>,
) -> Result<()> {
    ensure_declarative_rpc_only(package, "eal")?;
    let spec = EalExec {
        source: program.to_string(),
        result_binding,
    };
    for ability in package.manifest().abilities() {
        let ability_name = ability.name().to_string();
        sink.register_rpc(
            ability_name.clone(),
            package.ability_registry_manifest(&ability_name)?,
            eal_rpc_handler(spec.clone()),
        )?;
    }
    Ok(())
}

fn register_mcp_declarative_package(
    package: &crate::runtime::plugin_host::package::SharedPluginPackage,
    sink: &mut CatalogRegistrationSink<'_>,
    server: &str,
    tool: &str,
) -> Result<()> {
    ensure_declarative_rpc_only(package, "mcp")?;
    let spec = McpExec {
        server: server.to_string(),
        tool: tool.to_string(),
    };
    for ability in package.manifest().abilities() {
        let ability_name = ability.name().to_string();
        sink.register_rpc(
            ability_name.clone(),
            package.ability_registry_manifest(&ability_name)?,
            mcp_rpc_handler(spec.clone()),
        )?;
    }
    Ok(())
}

fn ensure_declarative_rpc_only(
    package: &crate::runtime::plugin_host::package::SharedPluginPackage,
    label: &'static str,
) -> Result<()> {
    for ability in package.manifest().abilities() {
        if ability.call_mode() != PluginCallMode::Rpc {
            return Err(PluginHostError::InvalidDeclarativeBinding {
                id: package.id().as_str().to_string(),
                reason: format!(
                    "{label} declarative binding only supports rpc abilities in this release; \
                     ability {} declares {:?}",
                    ability.name(),
                    ability.call_mode()
                ),
            });
        }
    }
    Ok(())
}

fn register_json_frame_process_package(
    package: &crate::runtime::plugin_host::package::SharedPluginPackage,
    sink: &mut CatalogRegistrationSink<'_>,
    command: SidecarCommand,
) -> Result<()> {
    for ability in package.manifest().abilities() {
        let ability_name = ability.name().to_string();
        let manifest = package.ability_registry_manifest(&ability_name)?;
        match ability.call_mode() {
            PluginCallMode::Rpc => {
                sink.register_rpc(
                    ability_name.clone(),
                    manifest,
                    rpc_process_handler(command.clone(), ability_name),
                )?;
            }
            PluginCallMode::Stream => {
                sink.register_stream(
                    ability_name.clone(),
                    manifest,
                    stream_process_handler(command.clone(), ability_name),
                )?;
            }
            PluginCallMode::Bidi => {
                sink.register_bidi(
                    ability_name.clone(),
                    manifest,
                    bidi_process_handler(command.clone(), ability_name),
                )?;
            }
        }
    }
    Ok(())
}

fn rpc_process_handler(command: SidecarCommand, ability: String) -> LocalRpcHandlerWithEnvelope {
    Arc::new(move |env, args: Value| {
        let invocation = sidecar_invocation_from_context(env, &ability, args)
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        SidecarRuntimeHost::new(command.clone())
            .invoke_rpc(uuid::Uuid::new_v4().to_string(), invocation)
            .map_err(|err| anyhow::anyhow!("{err}"))
    })
}

fn eal_rpc_handler(spec: EalExec) -> LocalRpcHandlerWithEnvelope {
    Arc::new(move |env, args: Value| {
        let invocation_context = invocation_context_from_envelope(&env).to_json_value();
        crate::runtime::agents::eal_executor::run_eal_exec_with_invocation_context(
            &spec,
            &args,
            Some(invocation_context),
            None,
        )
    })
}

fn mcp_rpc_handler(spec: McpExec) -> LocalRpcHandlerWithEnvelope {
    Arc::new(move |env, args: Value| {
        let invocation_context = invocation_context_from_envelope(&env).to_json_value();
        crate::runtime::agents::mcp_executor::run_mcp_exec_with_invocation_context(
            &spec,
            &args,
            Some(invocation_context),
        )
    })
}

fn invocation_context_from_envelope(env: &EnvelopeContext) -> ParentInvocationContext {
    ParentInvocationContext {
        caller: Some(env.caller().to_string()),
        callee: Some(env.callee().to_string()),
        ability: Some(env.ability().to_string()),
        subject: Some(env.subject().to_string()),
        invocation_nonce: Some(env.invocation_nonce().to_vec()),
        causal_context: Some(env.causal_context().clone()),
    }
}

fn stream_process_handler(
    command: SidecarCommand,
    ability: String,
) -> LocalStreamHandlerWithEnvelope {
    Arc::new(move |env, args: Value| {
        let invocation = sidecar_invocation_from_context(env, &ability, args)
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        SidecarRuntimeHost::new(command.clone())
            .open_stream(uuid::Uuid::new_v4().to_string(), invocation)
            .map_err(|err| anyhow::anyhow!("{err}"))
    })
}

fn bidi_process_handler(command: SidecarCommand, ability: String) -> LocalBidiHandlerWithEnvelope {
    Arc::new(move |env, args: Value| {
        let invocation = sidecar_invocation_from_context(env, &ability, args)
            .map_err(|err| anyhow::anyhow!("{err}"))?;
        SidecarRuntimeHost::new(command.clone())
            .open_bidi(uuid::Uuid::new_v4().to_string(), invocation)
            .map_err(|err| anyhow::anyhow!("{err}"))
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::runtime::ability::CallMode as DescriptorCallMode;
    use crate::runtime::invocation_target::{CallMode, InvocationTarget, TargetScope};
    use crate::runtime::plugin_host::package::PluginPackage;
    use crate::runtime::plugin_host::{PluginLoadPlanner, PluginPackageIndex};

    #[test]
    fn plugin_runtime_host_registers_exec_declarative_rpc() {
        let root = tempfile::tempdir().expect("root");
        write_exec_declarative_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let plan = PluginLoadPlanner::new("macos").plan(&index);
        let mut catalog = AxonAbilityCatalog::new();

        PluginRuntimeHost::new()
            .register(&plan, &mut catalog)
            .expect("register declarative exec");

        assert!(catalog.has_rpc("test.declarative_echo"));
        let manifest = catalog
            .manifest_for_dynamic("test.declarative_echo")
            .expect("registered plugin ability manifest");
        assert_eq!(
            manifest.description(),
            "test descriptor for test.declarative_echo"
        );
        assert_eq!(manifest.input_schema()["type"], "object");
        let record = catalog
            .control_plane_record_for_mode("test.declarative_echo", DescriptorCallMode::Rpc)
            .expect("plugin control-plane lookup is unambiguous")
            .expect("plugin control-plane record");
        assert_eq!(
            *record.implementation().source(),
            AbilityImplSource::DeclarativePlugin
        );
        assert!(record
            .implementation()
            .runtime_env()
            .label()
            .contains("plugin:"));
        assert_eq!(record.authority().scope().owner_projection(), "device");
        let result = catalog
            .execute_rpc(InvocationTarget {
                scope: TargetScope::Local,
                ability: "test.declarative_echo".to_string(),
                normalized_args: json!({"message": "hello"}),
                call_mode: CallMode::Rpc,
                subject: Some("easynet:///r/acme/resource/test".to_string()),
                causal_context: None,
            })
            .expect("declarative exec rpc");
        assert_eq!(result, json!({"ok": true, "message": "hello"}));
    }

    #[test]
    fn plugin_runtime_host_hot_registers_exec_declarative_rpc() {
        let root = tempfile::tempdir().expect("root");
        write_exec_declarative_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let plan = PluginLoadPlanner::new("macos").plan(&index);
        let catalog = AxonAbilityCatalog::new();

        PluginRuntimeHost::new()
            .hot_register(&plan, &catalog)
            .expect("hot register declarative exec");

        assert!(catalog.has_dynamic("test.declarative_echo"));
        let manifest = catalog
            .manifest_for_dynamic("test.declarative_echo")
            .expect("hot registered plugin ability manifest");
        assert_eq!(
            manifest.description(),
            "test descriptor for test.declarative_echo"
        );
        assert_eq!(manifest.input_schema()["type"], "object");
        let result = catalog
            .execute_rpc(InvocationTarget {
                scope: TargetScope::Local,
                ability: "test.declarative_echo".to_string(),
                normalized_args: json!({"message": "hot"}),
                call_mode: CallMode::Rpc,
                subject: Some("easynet:///r/acme/resource/test".to_string()),
                causal_context: None,
            })
            .expect("hot declarative exec rpc");
        assert_eq!(result, json!({"ok": true, "message": "hot"}));

        assert!(catalog
            .hot_unregister("test.declarative_echo")
            .expect("hot declarative unregister"));
        assert!(!catalog.has_dynamic("test.declarative_echo"));
        assert!(!catalog.has_rpc("test.declarative_echo"));
        catalog
            .execute_rpc(InvocationTarget {
                scope: TargetScope::Local,
                ability: "test.declarative_echo".to_string(),
                normalized_args: json!({"message": "after"}),
                call_mode: CallMode::Rpc,
                subject: Some("easynet:///r/acme/resource/test".to_string()),
                causal_context: None,
            })
            .expect_err("hot-unregistered plugin ability must not remain invokable");
    }

    #[test]
    fn plugin_runtime_host_hot_reload_rejects_static_ability_collision() {
        let root = tempfile::tempdir().expect("root");
        write_sidecar_package(root.path(), "fs.read");
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let plan = PluginLoadPlanner::new("macos").plan(&index);
        let mut catalog = AxonAbilityCatalog::new();
        catalog.register_rpc_with_owner(
            "fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(json!({"from": "static-system"}))),
        );

        let err = PluginRuntimeHost::new()
            .hot_reload(&plan, &catalog)
            .expect_err("hot reload must reject plugins that shadow system abilities");
        assert!(
            matches!(
                err,
                PluginHostError::DuplicateAbilityOwner {
                    ref ability,
                    ref first,
                    ..
                } if ability == "fs.read" && first == "daemon-static-catalog"
            ),
            "wrong hot reload error: {err}"
        );
        assert!(
            !catalog.has_dynamic("fs.read"),
            "rejected plugin must not leave a dynamic handler behind"
        );
        let out = catalog
            .invoke_rpc_json("fs.read", json!({}))
            .expect("static system handler remains invokable after rejected reload");
        assert_eq!(out, json!({"from": "static-system"}));
    }

    #[test]
    fn plugin_runtime_host_registers_eal_declarative_rpc() {
        let root = tempfile::tempdir().expect("root");
        write_eal_declarative_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let plan = PluginLoadPlanner::new("macos").plan(&index);
        let mut catalog = AxonAbilityCatalog::new();

        PluginRuntimeHost::new()
            .register(&plan, &mut catalog)
            .expect("register declarative eal");

        assert!(catalog.has_rpc("test.declarative_eal"));
        let record = catalog
            .control_plane_record_for_mode("test.declarative_eal", DescriptorCallMode::Rpc)
            .expect("EAL plugin control-plane lookup is unambiguous")
            .expect("EAL plugin control-plane record");
        assert_eq!(*record.implementation().source(), AbilityImplSource::Eal);
        let err = catalog
            .execute_rpc(InvocationTarget {
                scope: TargetScope::Local,
                ability: "test.declarative_eal".to_string(),
                normalized_args: json!({}),
                call_mode: CallMode::Rpc,
                subject: Some("easynet:///r/acme/resource/test".to_string()),
                causal_context: None,
            })
            .expect_err("missing EAL template argument should surface through handler");
        let msg = format!("{err}");
        assert!(msg.contains("eal executor"), "wrong error: {msg}");
        assert!(msg.contains("name"), "wrong error: {msg}");
    }

    #[test]
    fn plugin_runtime_host_hot_registers_mcp_declarative_rpc() {
        let root = tempfile::tempdir().expect("root");
        write_mcp_declarative_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let plan = PluginLoadPlanner::new("macos").plan(&index);
        let catalog = AxonAbilityCatalog::new();

        PluginRuntimeHost::new()
            .hot_register(&plan, &catalog)
            .expect("hot register declarative mcp");

        assert!(catalog.has_dynamic("test.declarative_mcp"));
        let record = catalog
            .control_plane_record_for_mode("test.declarative_mcp", DescriptorCallMode::Rpc)
            .expect("MCP plugin control-plane lookup is unambiguous")
            .expect("MCP plugin control-plane record");
        assert_eq!(*record.implementation().source(), AbilityImplSource::Mcp);
        let manifest = catalog
            .manifest_for_dynamic("test.declarative_mcp")
            .expect("hot registered MCP plugin ability manifest");
        assert_eq!(
            manifest.description(),
            "test descriptor for test.declarative_mcp"
        );
        assert_eq!(manifest.input_schema()["type"], "object");
        let err = catalog
            .execute_rpc(InvocationTarget {
                scope: TargetScope::Local,
                ability: "test.declarative_mcp".to_string(),
                normalized_args: json!([1, 2]),
                call_mode: CallMode::Rpc,
                subject: Some("easynet:///r/acme/resource/test".to_string()),
                causal_context: None,
            })
            .expect_err("non-object MCP args should be rejected by mcp executor");
        let msg = format!("{err}");
        assert!(
            msg.contains("mcp executor") && msg.contains("must be a JSON object"),
            "wrong error: {msg}"
        );
    }

    #[test]
    fn plugin_runtime_host_hot_reload_removes_boot_registered_installed_plugin() {
        let root = tempfile::tempdir().expect("root");
        write_sidecar_package(root.path(), "device.test.hot_reload_remove");
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let plan = PluginLoadPlanner::new("macos").plan(&index);
        let mut catalog = AxonAbilityCatalog::new();
        let host = PluginRuntimeHost::new();

        host.register(&plan, &mut catalog)
            .expect("boot register sidecar");
        assert!(catalog.has_rpc("device.test.hot_reload_remove"));

        let empty = PluginPackageIndex::from_packages(Vec::new()).expect("empty index");
        let empty_plan = PluginLoadPlanner::new("macos").plan(&empty);
        let report = host
            .hot_reload(&empty_plan, &catalog)
            .expect("hot reload empty plugin index");

        assert!(report
            .unregistered_abilities
            .iter()
            .any(|ability| ability == "device.test.hot_reload_remove"));
        assert!(
            !catalog.has_rpc("device.test.hot_reload_remove"),
            "removed plugin ability must not remain invokable through LocalRuntime"
        );
    }

    fn write_exec_declarative_package(root: &std::path::Path) {
        fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        fs::create_dir_all(root.join("bin")).expect("bin dir");
        fs::write(
            root.join("plugin.toml"),
            r#"
schema_version = "1"
id = "test.declarative"
version = "0.1.0"
kind = "declarative"
entrypoint = "declarative.exec"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[declarative]
kind = "exec"
argv = ["bin/exec-plugin"]

[[ability_metadata]]
name = "test.declarative_echo"
layer = "control"
"#,
        )
        .expect("manifest");
        fs::write(
            root.join("abilities/test.declarative_echo.ability.toml"),
            test_descriptor("test.declarative_echo"),
        )
        .expect("descriptor");
        let script = root.join("bin/exec-plugin");
        fs::write(
            &script,
            r#"#!/bin/sh
read frame
call_id=$(printf '%s\n' "$frame" | sed -n 's/.*"call_id":"\([^"]*\)".*/\1/p')
message=$(printf '%s\n' "$frame" | sed -n 's/.*"message":"\([^"]*\)".*/\1/p')
printf '%s\n' "{\"type\":\"result\",\"call_id\":\"$call_id\",\"value\":{\"ok\":true,\"message\":\"$message\"}}"
"#,
        )
        .expect("exec script");
        let mut permissions = fs::metadata(&script).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).expect("chmod script");
    }

    fn write_eal_declarative_package(root: &std::path::Path) {
        fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        fs::write(
            root.join("plugin.toml"),
            r#"
schema_version = "1"
id = "test.declarative.eal"
version = "0.1.0"
kind = "declarative"
entrypoint = "declarative.eal"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[declarative]
kind = "eal"
program = "mission \"{{ name }}\" {}"

[[ability_metadata]]
name = "test.declarative_eal"
layer = "control"
"#,
        )
        .expect("manifest");
        fs::write(
            root.join("abilities/test.declarative_eal.ability.toml"),
            test_descriptor("test.declarative_eal"),
        )
        .expect("descriptor");
    }

    fn write_mcp_declarative_package(root: &std::path::Path) {
        fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        fs::write(
            root.join("plugin.toml"),
            r#"
schema_version = "1"
id = "test.declarative.mcp"
version = "0.1.0"
kind = "declarative"
entrypoint = "declarative.mcp"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[declarative]
kind = "mcp"
server = "test-server"
tool = "test-tool"

[[ability_metadata]]
name = "test.declarative_mcp"
layer = "control"
"#,
        )
        .expect("manifest");
        fs::write(
            root.join("abilities/test.declarative_mcp.ability.toml"),
            test_descriptor("test.declarative_mcp"),
        )
        .expect("descriptor");
    }

    fn write_sidecar_package(root: &std::path::Path, ability: &str) {
        fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        fs::create_dir_all(root.join("bin")).expect("bin dir");
        fs::write(
            root.join("plugin.toml"),
            format!(
                r#"
schema_version = "1"
id = "test.sidecar.hot_reload"
version = "0.1.0"
kind = "sidecar"
entrypoint = "bin/sidecar"
abilities = ["abilities/*.ability.toml"]
permissions = []
resources = []
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "{ability}"
layer = "control"
"#
            ),
        )
        .expect("manifest");
        fs::write(
            root.join(format!("abilities/{ability}.ability.toml")),
            test_descriptor(ability),
        )
        .expect("descriptor");
        let sidecar = root.join("bin/sidecar");
        fs::write(&sidecar, "#!/bin/sh\n").expect("sidecar bin");
        let mut permissions = fs::metadata(&sidecar)
            .expect("sidecar metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&sidecar, permissions).expect("chmod sidecar");
    }

    fn test_descriptor(ability: &str) -> String {
        format!(
            r#"schema_version = "1"
name = "{ability}"
description = "test descriptor for {ability}"

[input_schema]
type = "object"
additionalProperties = false
"#
        )
    }
}
