// EasyNet CLI — Plugin Group
// ===========================
//
// File: src/facade/cli/groups/plugin.rs
// Description: `easynet plugin ...` package lifecycle and boot-state query.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::runtime::plugin_host::index::default_plugin_root;
use crate::runtime::plugin_host::{
    PluginAbilitySurfaceRecord, PluginInstaller, PluginLoadPlanner, PluginPackageIndex,
    PluginSurfaceProjector,
};
use crate::support::output::{self, OutputFormat};

/// Plugin package lifecycle and daemon boot-state inspection.
///
/// What this is NOT: ability invocation. Installed plugin abilities still enter
/// the daemon only through `PluginPackageIndex -> PluginLoadPlan ->
/// PluginRuntimeHost -> AxonAbilityCatalog`.
#[derive(Debug, Args)]
pub struct PluginArgs {
    #[command(subcommand)]
    pub action: PluginAction,
}

#[derive(Debug, Subcommand)]
pub enum PluginAction {
    /// List indexed plugin abilities and their descriptor/runtime/invocation surfaces.
    List(ListArgs),
    /// Install an unpacked plugin package directory transactionally.
    Install(PackageSourceArgs),
    /// Install a replacement package version transactionally.
    Update(PackageSourceArgs),
    /// Remove one installed package version transactionally.
    Remove(RemoveArgs),
}

#[derive(Debug, Args)]
pub struct ListArgs {
    /// Output format. 'table' is operator-facing; 'json' is stable for scripts.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct PackageSourceArgs {
    /// Unpacked package root containing plugin.toml.
    pub path: PathBuf,
}

#[derive(Debug, Args)]
pub struct RemoveArgs {
    /// Plugin package id.
    pub id: String,
    /// Plugin package version.
    pub version: String,
}

pub fn run(args: PluginArgs) -> anyhow::Result<()> {
    match args.action {
        PluginAction::List(args) => run_list(args),
        PluginAction::Install(args) => run_install(args),
        PluginAction::Update(args) => run_update(args),
        PluginAction::Remove(args) => run_remove(args),
    }
}

fn run_list(args: ListArgs) -> anyhow::Result<()> {
    let rows = match invoke_plugin_status()? {
        Some(rows) => rows,
        None => {
            output::warn("daemon is not running; showing offline planned plugin status");
            let report = PluginPackageIndex::load_default_resilient()?;
            let (index, index_errors) = report.into_parts();
            let plan = PluginLoadPlanner::current().plan(&index);
            PluginSurfaceProjector::project_with_daemon(&index, &plan, None, &index_errors)
        }
    };

    match args.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        OutputFormat::Table => {
            let mut table = output::table(&[
                "package",
                "version",
                "ability",
                "kind",
                "mode",
                "planned",
                "daemon",
                "descriptor",
                "runtime",
                "invoke",
            ]);
            for row in rows {
                table.add_row([
                    row.package_id,
                    row.package_version,
                    row.ability,
                    format!("{:?}", row.kind).to_ascii_lowercase(),
                    format!("{:?}", row.call_mode).to_ascii_lowercase(),
                    row.planned_load_status,
                    row.daemon_runtime_status,
                    bool_label(row.descriptor_published),
                    bool_label(row.runtime_published),
                    bool_label(row.invokable),
                ]);
            }
            println!("{table}");
        }
    }
    Ok(())
}

fn run_install(args: PackageSourceArgs) -> anyhow::Result<()> {
    let installer = PluginInstaller::new(default_plugin_root());
    let record = installer.install(&args.path)?;
    output::success(&format!(
        "installed plugin {}@{}",
        record.id, record.version
    ));
    output::detail("hash", &record.hash);
    notify_daemon_reload()?;
    Ok(())
}

fn run_update(args: PackageSourceArgs) -> anyhow::Result<()> {
    let installer = PluginInstaller::new(default_plugin_root());
    let record = installer.update(&args.path)?;
    output::success(&format!("updated plugin {}@{}", record.id, record.version));
    output::detail("hash", &record.hash);
    notify_daemon_reload()?;
    Ok(())
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    let installer = PluginInstaller::new(default_plugin_root());
    installer.remove(&args.id, &args.version)?;
    output::success(&format!("removed plugin {}@{}", args.id, args.version));
    notify_daemon_reload()?;
    Ok(())
}

fn bool_label(value: bool) -> String {
    if value {
        "yes".to_string()
    } else {
        "no".to_string()
    }
}

fn notify_daemon_reload() -> anyhow::Result<()> {
    match invoke_plugin_reload() {
        Ok(Some(value)) => {
            output::detail("daemon_reload", &value.to_string());
            Ok(())
        }
        Ok(None) => {
            output::warn(
                "daemon is not running; plugin change will take effect on next daemon boot",
            );
            Ok(())
        }
        Err(err) => Err(err),
    }
}

fn invoke_plugin_reload() -> anyhow::Result<Option<serde_json::Value>> {
    invoke_plugin_control_ability(crate::runtime::agents::plugin_lifecycle_ability::RELOAD_ABILITY)
}

fn invoke_plugin_status() -> anyhow::Result<Option<Vec<PluginAbilitySurfaceRecord>>> {
    let Some(value) = invoke_plugin_control_ability(
        crate::runtime::agents::plugin_lifecycle_ability::STATUS_ABILITY,
    )?
    else {
        return Ok(None);
    };
    let rows = serde_json::from_value(
        value
            .get("abilities")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )?;
    Ok(Some(rows))
}

fn invoke_plugin_control_ability(
    ability: &'static str,
) -> anyhow::Result<Option<serde_json::Value>> {
    #[cfg(feature = "axon-pb")]
    {
        invoke_plugin_control_ability_via_daemon(ability)
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = ability;
        Ok(None)
    }
}

#[cfg(feature = "axon-pb")]
fn invoke_plugin_control_ability_via_daemon(
    ability: &'static str,
) -> anyhow::Result<Option<serde_json::Value>> {
    let Some(_subject) = plugin_control_subject_ura()? else {
        return Ok(None);
    };
    match crate::support::local_invoke::invoke_local_ability(ability, serde_json::json!({})) {
        Ok(value) => Ok(Some(value)),
        Err(err)
            if crate::support::local_invoke::classify_invoke_error(&err)
                == crate::support::local_invoke::LocalInvokeErrorKind::DaemonOffline =>
        {
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

#[cfg(feature = "axon-pb")]
fn plugin_control_subject_ura() -> anyhow::Result<Option<String>> {
    match crate::persistence::config::load_credentials() {
        Ok(creds) => Ok(Some(crate::ura::device_ura(
            creds.realm.trim(),
            creds.node_id.trim(),
        ))),
        Err(err) => {
            if is_missing_or_incomplete_credentials(&err) {
                Ok(None)
            } else {
                Err(err)
            }
        }
    }
}

#[cfg(feature = "axon-pb")]
fn is_missing_or_incomplete_credentials(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("no credentials found") || msg.contains("credentials file is incomplete")
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;

    #[test]
    fn plugin_control_subject_is_unavailable_when_unpaired() {
        let _guard = crate::facade::cli::test_support::HomeGuard::new();

        assert_eq!(plugin_control_subject_ura().unwrap(), None);
    }

    #[test]
    fn plugin_control_subject_uses_paired_device_ura() {
        let _guard = crate::facade::cli::test_support::HomeGuard::new();
        crate::persistence::config::save_credentials(&crate::persistence::config::Credentials {
            node_id: "dev-a".to_string(),
            credential_token: "token".to_string(),
            realm: "acme".to_string(),
            hub_endpoint: "axon://hub.example:50051".to_string(),
            username: Some("alice".to_string()),
            ..Default::default()
        })
        .expect("write test credentials");

        assert_eq!(
            plugin_control_subject_ura().unwrap().as_deref(),
            Some("easynet:///r/acme/device/dev-a")
        );
    }
}
