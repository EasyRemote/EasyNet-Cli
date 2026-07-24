// EasyNet CLI — plugin runtime host API
// =====================================
//
// File: src/daemon/plugins/host_api.rs
// Description: Collect loaded plugin AbilityImpl contributions for daemon binding.

use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

use serde::Serialize;
use serde_json::Value;

use crate::daemon::ability::dispatch::{
    EnvelopeContext, LocalBidiHandlerWithEnvelope, LocalRpcHandlerWithEnvelope,
    LocalStreamHandlerWithEnvelope,
};
use crate::daemon::ability::manifest::{EalExec, McpExec};
use crate::daemon::ability::{AbilityImplSource, RuntimeEnv};
use crate::daemon::plugins::contribution::{
    PluginContributionBuilder, PluginContributionSet, PluginRequirementSet,
};
use crate::daemon::plugins::errors::{PluginHostError, Result};
use crate::daemon::plugins::load_plan::PluginLoadPlan;
use crate::daemon::plugins::manifest::{CallMode, PluginDeclarativeBinding, PluginKind};
use crate::daemon::plugins::realtime::PluginRealtimeActivationPlan;
use crate::daemon::plugins::sidecar::{
    sidecar_invocation_from_context, SidecarCommand, SidecarRuntimeHost,
};
use crate::daemon::plugins::PluginRealtimeCapability;

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
    pub realtime_activation_hints: Vec<PluginRealtimeActivationHint>,
    pub realtime_activation_plans: Vec<PluginRealtimeActivationPlan>,
}

/// Quick-add realtime capability hint returned after plugin reload.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PluginRealtimeActivationHint {
    pub package_id: String,
    pub package_version: String,
    pub capabilities: Vec<PluginRealtimeCapability>,
    pub activation_plans: Vec<PluginRealtimeActivationPlan>,
}

/// Boot-time plugin contribution split.
///
/// Builtin packages are compiled into the daemon and bind into the static
/// execution index. Installed sidecar/declarative packages bind into the
/// dynamic execution index so reload/remove can reconcile them post-boot.
#[derive(Clone, Default)]
pub struct PluginBootContributionSet {
    builtin: PluginContributionSet,
    runtime: PluginContributionSet,
}

impl PluginBootContributionSet {
    pub fn builtin(&self) -> &PluginContributionSet {
        &self.builtin
    }

    pub fn runtime(&self) -> &PluginContributionSet {
        &self.runtime
    }
}

impl PluginRuntimeHost {
    /// Construct a plugin runtime host.
    pub fn new() -> Self {
        Self::default()
    }

    /// Collect boot-time contributions without applying daemon authority
    /// policy or mutating the Axon runtime.
    pub fn collect_boot_contributions(
        &self,
        load_plan: &PluginLoadPlan,
    ) -> Result<PluginBootContributionSet> {
        Ok(PluginBootContributionSet {
            builtin: collect_plugin_contributions(load_plan, BuiltinContribution::Only)?,
            runtime: collect_plugin_contributions(load_plan, BuiltinContribution::Skip)?,
        })
    }

    /// Collect installed runtime contributions for post-boot binding/reload.
    pub fn collect_runtime_contributions(
        &self,
        load_plan: &PluginLoadPlan,
    ) -> Result<PluginContributionSet> {
        collect_plugin_contributions(load_plan, BuiltinContribution::Skip)
    }

    pub fn runtime_ability_names(&self, load_plan: &PluginLoadPlan) -> BTreeSet<String> {
        loaded_runtime_plugin_ability_names(load_plan)
    }

    pub fn tracked_runtime_abilities(&self) -> BTreeSet<String> {
        self.tracked_runtime_abilities
            .read()
            .expect("plugin runtime ability tracker poisoned")
            .clone()
    }

    pub fn replace_tracked_runtime_abilities(&self, abilities: BTreeSet<String>) {
        let mut tracked = self
            .tracked_runtime_abilities
            .write()
            .expect("plugin runtime ability tracker poisoned");
        *tracked = abilities;
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

fn declarative_impl_source(binding: Option<&PluginDeclarativeBinding>) -> AbilityImplSource {
    match binding {
        Some(PluginDeclarativeBinding::Eal { .. }) => AbilityImplSource::Eal,
        Some(PluginDeclarativeBinding::Mcp { .. }) => AbilityImplSource::Mcp,
        Some(PluginDeclarativeBinding::Exec { .. }) | None => AbilityImplSource::DeclarativePlugin,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BuiltinContribution {
    Only,
    Skip,
}

fn collect_plugin_contributions(
    load_plan: &PluginLoadPlan,
    builtin: BuiltinContribution,
) -> Result<PluginContributionSet> {
    let mut contributions = PluginContributionSet::default();
    for entry in load_plan.entries() {
        if !entry.is_loaded() {
            continue;
        }
        let package = entry.package();
        let is_builtin = package.builtin_binding().is_some();
        match builtin {
            BuiltinContribution::Only if !is_builtin => continue,
            BuiltinContribution::Skip if is_builtin => continue,
            _ => {}
        }
        contributions.push(collect_package_contribution(package)?);
    }
    Ok(contributions)
}

fn collect_package_contribution(
    package: &crate::daemon::plugins::package::SharedPluginPackage,
) -> Result<crate::daemon::plugins::contribution::PluginPackageContribution> {
    let manifest = package.manifest();
    let mut builder = PluginContributionBuilder::new(
        package.id().as_str().to_string(),
        package.version().as_str().to_string(),
        manifest.kind(),
        manifest.limits(),
        PluginRequirementSet::new(
            manifest.permissions().to_vec(),
            manifest.resources().to_vec(),
        ),
        manifest.realtime_capabilities().to_vec(),
    );

    match manifest.kind() {
        PluginKind::Builtin => {
            let binding = package.builtin_binding().ok_or_else(|| {
                PluginHostError::MissingBuiltinBinding(package.id().as_str().to_string())
            })?;
            binding.contribute(&mut builder, manifest.limits())?;
        }
        PluginKind::Sidecar => {
            let command = SidecarCommand::from_package(package);
            let mut sink =
                ContributionRegistrationSink::new(&mut builder, AbilityImplSource::SidecarPlugin);
            contribute_json_frame_process_package(package, &mut sink, command)?;
        }
        PluginKind::Declarative => {
            let impl_source = declarative_impl_source(manifest.declarative_binding());
            let mut sink = ContributionRegistrationSink::new(&mut builder, impl_source);
            contribute_declarative_package(package, &mut sink)?;
        }
        PluginKind::DesktopCompanion => {
            return Err(PluginHostError::InvalidContribution {
                package: format!("{}@{}", package.id().as_str(), package.version().as_str()),
                ability: "<package>".to_string(),
                reason: "desktop companion packages do not contribute ability implementations"
                    .to_string(),
            });
        }
    }

    builder.finish()
}

struct ContributionRegistrationSink<'a> {
    builder: &'a mut PluginContributionBuilder,
    impl_source: AbilityImplSource,
    runtime_env: RuntimeEnv,
}

impl<'a> ContributionRegistrationSink<'a> {
    fn new(builder: &'a mut PluginContributionBuilder, impl_source: AbilityImplSource) -> Self {
        let runtime_env = builder.plugin_runtime_env();
        Self {
            builder,
            impl_source,
            runtime_env,
        }
    }

    fn contribute_rpc(
        &mut self,
        ability: String,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalRpcHandlerWithEnvelope,
    ) -> Result<()> {
        self.builder.rpc(
            ability,
            manifest,
            self.impl_source.clone(),
            self.runtime_env.clone(),
            handler,
        )
    }

    fn contribute_stream(
        &mut self,
        ability: String,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalStreamHandlerWithEnvelope,
    ) -> Result<()> {
        self.builder.stream(
            ability,
            manifest,
            self.impl_source.clone(),
            self.runtime_env.clone(),
            handler,
        )
    }

    fn contribute_bidi(
        &mut self,
        ability: String,
        manifest: crate::daemon::ability::manifest::AbilityManifest,
        handler: LocalBidiHandlerWithEnvelope,
    ) -> Result<()> {
        self.builder.bidi(
            ability,
            manifest,
            self.impl_source.clone(),
            self.runtime_env.clone(),
            handler,
        )
    }
}

fn contribute_declarative_package(
    package: &crate::daemon::plugins::package::SharedPluginPackage,
    sink: &mut ContributionRegistrationSink<'_>,
) -> Result<()> {
    match package.manifest().declarative_binding() {
        Some(PluginDeclarativeBinding::Exec { argv }) => {
            let command = exec_declarative_command(package, argv)?;
            contribute_json_frame_process_package(package, sink, command)
        }
        Some(PluginDeclarativeBinding::Eal {
            program,
            result_binding,
        }) => contribute_eal_declarative_package(package, sink, program, result_binding.clone()),
        Some(PluginDeclarativeBinding::Mcp { server, tool }) => {
            contribute_mcp_declarative_package(package, sink, server, tool)
        }
        None => Ok(()),
    }
}

fn exec_declarative_command(
    package: &crate::daemon::plugins::package::SharedPluginPackage,
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

fn contribute_eal_declarative_package(
    package: &crate::daemon::plugins::package::SharedPluginPackage,
    sink: &mut ContributionRegistrationSink<'_>,
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
        sink.contribute_rpc(
            ability_name.clone(),
            package.ability_registry_manifest(&ability_name)?,
            eal_rpc_handler(spec.clone()),
        )?;
    }
    Ok(())
}

fn contribute_mcp_declarative_package(
    package: &crate::daemon::plugins::package::SharedPluginPackage,
    sink: &mut ContributionRegistrationSink<'_>,
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
        sink.contribute_rpc(
            ability_name.clone(),
            package.ability_registry_manifest(&ability_name)?,
            mcp_rpc_handler(spec.clone()),
        )?;
    }
    Ok(())
}

fn ensure_declarative_rpc_only(
    package: &crate::daemon::plugins::package::SharedPluginPackage,
    label: &'static str,
) -> Result<()> {
    for ability in package.manifest().abilities() {
        if ability.call_mode() != CallMode::Rpc {
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

fn contribute_json_frame_process_package(
    package: &crate::daemon::plugins::package::SharedPluginPackage,
    sink: &mut ContributionRegistrationSink<'_>,
    command: SidecarCommand,
) -> Result<()> {
    for ability in package.manifest().abilities() {
        let ability_name = ability.name().to_string();
        let manifest = package.ability_registry_manifest(&ability_name)?;
        match ability.call_mode() {
            CallMode::Rpc => {
                sink.contribute_rpc(
                    ability_name.clone(),
                    manifest,
                    rpc_process_handler(command.clone(), ability_name),
                )?;
            }
            CallMode::Stream => {
                sink.contribute_stream(
                    ability_name.clone(),
                    manifest,
                    stream_process_handler(command.clone(), ability_name),
                )?;
            }
            CallMode::Bidi => {
                sink.contribute_bidi(
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
        let gateway = Arc::new(PluginEalInvocationGateway::new(env));
        crate::daemon::execution::mission::executors::eal::run_eal_exec_with_gateway(
            &spec, &args, gateway, None,
        )
    })
}

#[derive(Clone)]
struct PluginEalInvocationGateway {
    admitted_parent: EnvelopeContext,
}

impl PluginEalInvocationGateway {
    fn new(admitted_parent: EnvelopeContext) -> Self {
        Self { admitted_parent }
    }
}

impl crate::daemon::execution::mission::invocation_gateway::MissionInvocationGateway
    for PluginEalInvocationGateway
{
    fn invoke(
        &self,
        request: crate::daemon::execution::mission::invocation_gateway::MissionInvocationRequest,
    ) -> anyhow::Result<crate::daemon::execution::child_invocation::ChildInvocationOutcome> {
        let gateway =
            crate::daemon::execution::mission::invocation_gateway::DaemonMissionInvocationGateway::from_admitted_envelope(
                &self.admitted_parent,
            )?;
        crate::daemon::execution::mission::invocation_gateway::MissionInvocationGateway::invoke(
            &gateway, request,
        )
    }
}

fn mcp_rpc_handler(spec: McpExec) -> LocalRpcHandlerWithEnvelope {
    Arc::new(move |env, args: Value| {
        let invocation_context = invocation_observation_from_envelope(&env);
        crate::daemon::ability::builtins::integrations::mcp::executor::run_mcp_exec_with_invocation_context(
            &spec,
            &args,
            Some(invocation_context),
        )
    })
}

fn invocation_observation_from_envelope(env: &EnvelopeContext) -> Value {
    serde_json::json!({
        "caller_ura": env.caller(),
        "callee_ura": env.callee(),
        "ability_ura": env.ability(),
        "subject_ura": env.subject(),
        "invocation_nonce": env.invocation_nonce(),
        "causal_context": env.causal_context(),
    })
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
    use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
    use crate::daemon::ability::CallMode as DescriptorCallMode;
    use crate::daemon::invocation::routing::target::CallMode;
    use crate::daemon::plugins::package::PluginPackage;
    use crate::daemon::plugins::{
        PluginLoadPlanner, PluginPackageIndex, PluginRuntimeManager, PluginRuntimeState,
    };

    fn executable_test_catalog() -> AxonAbilityCatalog {
        AxonAbilityCatalog::new_test_runtime_for_device_authority(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            "easynet:///r/acme/device/test-plugin-host",
        )
    }

    #[test]
    fn plugin_runtime_host_registers_exec_declarative_rpc() {
        let root = tempfile::tempdir().expect("root");
        write_exec_declarative_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let mut catalog = executable_test_catalog();

        manager_from_index(index)
            .register_current_plugins(&mut catalog)
            .expect("register declarative exec");

        assert!(catalog.has_rpc("test.declarative_echo"));
        let record = catalog
            .control_plane_record_for_mode("test.declarative_echo", DescriptorCallMode::Rpc)
            .expect("plugin control-plane lookup is unambiguous")
            .expect("plugin control-plane record");
        assert_eq!(
            record.descriptor().description,
            "test descriptor for test.declarative_echo"
        );
        assert_eq!(record.descriptor().input_schema()["type"], "object");
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
            .execute_rpc(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                "test.declarative_echo",
                json!({"message": "hello"}),
                CallMode::Rpc,
                "easynet:///r/acme/resource/test",
            ))
            .expect("declarative exec rpc");
        assert_eq!(result, json!({"ok": true, "message": "hello"}));
    }

    #[test]
    fn plugin_runtime_host_hot_registers_exec_declarative_rpc() {
        let root = tempfile::tempdir().expect("root");
        write_exec_declarative_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let mut catalog = executable_test_catalog();

        manager_from_index(index)
            .register_current_plugins(&mut catalog)
            .expect("hot register declarative exec");

        assert!(catalog.has_dynamic("test.declarative_echo"));
        let record = catalog
            .control_plane_record_for_mode("test.declarative_echo", DescriptorCallMode::Rpc)
            .expect("plugin control-plane lookup is unambiguous")
            .expect("hot registered plugin canonical descriptor");
        assert_eq!(
            record.descriptor().description,
            "test descriptor for test.declarative_echo"
        );
        assert_eq!(record.descriptor().input_schema()["type"], "object");
        let result = catalog
            .execute_rpc(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                "test.declarative_echo",
                json!({"message": "hot"}),
                CallMode::Rpc,
                "easynet:///r/acme/resource/test",
            ))
            .expect("hot declarative exec rpc");
        assert_eq!(result, json!({"ok": true, "message": "hot"}));

        assert!(catalog
            .hot_unregister("test.declarative_echo")
            .expect("hot declarative unregister"));
        assert!(!catalog.has_dynamic("test.declarative_echo"));
        assert!(!catalog.has_rpc("test.declarative_echo"));
        catalog
            .execute_rpc(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                "test.declarative_echo",
                json!({"message": "after"}),
                CallMode::Rpc,
                "easynet:///r/acme/resource/test",
            ))
            .expect_err("hot-unregistered plugin ability must not remain invokable");
    }

    #[test]
    fn plugin_runtime_host_hot_reload_rejects_static_ability_collision() {
        let root = tempfile::tempdir().expect("root");
        write_sidecar_package(root.path(), "fs.read");
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let mut catalog = executable_test_catalog();
        catalog.register_rpc_with_owner(
            "fs.read",
            OwnerKind::Device,
            Arc::new(|_args| Ok(json!({"from": "static-system"}))),
        );

        let manager = PluginRuntimeManager::from_state(empty_state());
        let err = manager
            .reload_plugins_from_state(planned_state(index), &catalog)
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
            .invoke_rpc_target_json(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root(
                "fs.read",
                json!({}),
                CallMode::Rpc,
            ))
            .expect("static system handler remains invokable after rejected reload");
        assert_eq!(out, json!({"from": "static-system"}));
    }

    #[test]
    fn plugin_eal_gateway_rejects_child_dispatch_without_daemon_runtime_admission() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let mut catalog = executable_test_catalog();
        catalog.register_rpc_with_envelope_and_owner(
            "observe.health",
            OwnerKind::Device,
            Arc::new(|env, _args| {
                let gateway = PluginEalInvocationGateway::new(env);
                let request =
                    crate::daemon::execution::mission::invocation_gateway::MissionInvocationRequest::system(
                        "observe.health",
                        json!({}),
                    );
                crate::daemon::execution::mission::invocation_gateway::MissionInvocationGateway::invoke(
                    &gateway, request,
                )
                .map(|_| json!({"unexpected": "child dispatch admitted"}))
            }),
        );

        let err = catalog
            .execute_rpc(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                "observe.health",
                json!({}),
                CallMode::Rpc,
                "easynet:///r/acme/resource/test",
            ))
            .expect_err("canonical-only runtime must not synthesize daemon runtime admission");
        let msg = format!("{err:#}");
        assert!(
            msg.contains(
                "Mission child dispatch requires the admitting daemon runtime-admission capability"
            ),
            "wrong error: {msg}"
        );
    }

    #[test]
    fn plugin_runtime_host_registers_eal_declarative_rpc() {
        let mut catalog = executable_test_catalog();
        register_eal_declarative_plugin(&mut catalog);

        assert!(catalog.has_rpc("test.declarative_eal"));
        let record = catalog
            .control_plane_record_for_mode("test.declarative_eal", DescriptorCallMode::Rpc)
            .expect("EAL plugin control-plane lookup is unambiguous")
            .expect("EAL plugin control-plane record");
        assert_eq!(*record.implementation().source(), AbilityImplSource::Eal);
        let err = catalog
            .execute_rpc(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                "test.declarative_eal",
                json!({}),
                CallMode::Rpc,
                "easynet:///r/acme/resource/test",
            ))
            .expect_err("missing EAL template argument should surface through handler");
        let msg = format!("{err}");
        assert!(msg.contains("eal executor"), "wrong error: {msg}");
        assert!(msg.contains("name"), "wrong error: {msg}");
    }

    fn register_eal_declarative_plugin(catalog: &mut AxonAbilityCatalog) {
        let root = tempfile::tempdir().expect("root");
        write_eal_declarative_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        manager_from_index(index)
            .register_current_plugins(catalog)
            .expect("register declarative eal");
    }

    #[test]
    fn plugin_runtime_host_hot_registers_mcp_declarative_rpc() {
        let root = tempfile::tempdir().expect("root");
        write_mcp_declarative_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let mut catalog = executable_test_catalog();

        manager_from_index(index)
            .register_current_plugins(&mut catalog)
            .expect("hot register declarative mcp");

        assert!(catalog.has_dynamic("test.declarative_mcp"));
        let record = catalog
            .control_plane_record_for_mode("test.declarative_mcp", DescriptorCallMode::Rpc)
            .expect("MCP plugin control-plane lookup is unambiguous")
            .expect("MCP plugin control-plane record");
        assert_eq!(*record.implementation().source(), AbilityImplSource::Mcp);
        assert_eq!(
            record.descriptor().description,
            "test descriptor for test.declarative_mcp"
        );
        assert_eq!(record.descriptor().input_schema()["type"], "object");
        let err = catalog
            .execute_rpc(crate::daemon::invocation::routing::target::SystemInvocationTargetIssuer::local_root_for_subject(
                "test.declarative_mcp",
                json!([1, 2]),
                CallMode::Rpc,
                "easynet:///r/acme/resource/test",
            ))
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
        let mut catalog = AxonAbilityCatalog::new();
        let manager = manager_from_index(index);

        manager
            .register_current_plugins(&mut catalog)
            .expect("boot register sidecar");
        assert!(catalog.has_rpc("device.test.hot_reload_remove"));

        let empty = PluginPackageIndex::from_packages(Vec::new()).expect("empty index");
        let report = manager
            .reload_plugins_from_state(planned_state(empty), &catalog)
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

    #[test]
    fn plugin_runtime_host_hot_reload_reports_realtime_activation_hints() {
        let root = tempfile::tempdir().expect("root");
        write_realtime_sidecar_package(root.path());
        let package = Arc::new(PluginPackage::from_installed(root.path(), None).expect("package"));
        let index = PluginPackageIndex::from_packages(vec![package]).expect("index");
        let catalog = AxonAbilityCatalog::new();

        let report = PluginRuntimeManager::from_state(empty_state())
            .reload_plugins_from_state(planned_state(index), &catalog)
            .expect("hot reload realtime sidecar");

        assert_eq!(report.realtime_activation_hints.len(), 1);
        let hint = &report.realtime_activation_hints[0];
        assert_eq!(hint.package_id, "test.sidecar.realtime");
        assert_eq!(hint.capabilities.len(), 1);
        assert!(hint.capabilities[0].quick_add());
        assert_eq!(hint.activation_plans.len(), 1);
        assert_eq!(
            hint.activation_plans[0].status,
            crate::daemon::plugins::PluginRealtimeActivationStatus::Ready
        );
        assert_eq!(
            hint.activation_plans[0].available_abilities,
            vec!["test.camera".to_string()]
        );
        assert_eq!(report.realtime_activation_plans.len(), 1);
    }

    fn planned_state(index: PluginPackageIndex) -> PluginRuntimeState {
        PluginRuntimeState::from_index_with_planner(index, PluginLoadPlanner::new("macos"))
    }

    fn empty_state() -> PluginRuntimeState {
        planned_state(PluginPackageIndex::from_packages(Vec::new()).expect("empty plugin index"))
    }

    fn manager_from_index(index: PluginPackageIndex) -> PluginRuntimeManager {
        PluginRuntimeManager::from_state(planned_state(index))
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

    fn write_realtime_sidecar_package(root: &std::path::Path) {
        fs::create_dir_all(root.join("abilities")).expect("abilities dir");
        fs::create_dir_all(root.join("bin")).expect("bin dir");
        fs::write(
            root.join("plugin.toml"),
            r#"
schema_version = "1"
id = "test.sidecar.realtime"
version = "0.1.0"
kind = "sidecar"
entrypoint = "bin/sidecar"
abilities = ["abilities/*.ability.toml"]
permissions = ["camera"]
resources = ["camera"]
platforms = []

[limits]
max_sessions = 1
max_frame_queue = 1

[[ability_metadata]]
name = "test.camera"
layer = "operational"
call_mode = "bidi"
bidi_wire_kind = "json_frames"

[[realtime_capability]]
kind = "camera"
modes = ["snapshot", "subscribe", "record"]
transport = "invoke_bidi"
activation_abilities = ["test.camera"]
permissions = ["camera"]
resources = ["camera"]
quick_add = true
"#,
        )
        .expect("manifest");
        fs::write(
            root.join("abilities/test.camera.ability.toml"),
            test_descriptor("test.camera"),
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
            r#"schema_version = "2"
	name = "{ability}"
	description = "test descriptor for {ability}"
	admission_action = "invoke"

	[input_schema]
	type = "object"
additionalProperties = false
"#
        )
    }
}
