// EasyNet CLI — Plugin Group
// ===========================
//
// File: src/cli/groups/plugin.rs
// Description: `easynet plugin ...` package lifecycle and boot-state query.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde_json::Value;

use super::plugin_template::{init_hello_plugin, PluginTemplateInit, PluginTemplateLanguage};
use crate::daemon::plugins::index::default_plugin_root;
use crate::daemon::plugins::{
    DesktopCompanionManager, PluginInstaller, PluginKind, PluginPackageIndex,
};
use crate::support::platform::output::{self, OutputFormat};

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
    /// Create a Hello World plugin project.
    Init(InitArgs),
    /// List indexed plugin abilities and their descriptor/runtime/invocation surfaces.
    List(ListArgs),
    /// Install an unpacked plugin package directory transactionally.
    Install(PackageSourceArgs),
    /// Install a replacement package version transactionally.
    Update(PackageSourceArgs),
    /// Remove one installed package version transactionally.
    Remove(RemoveArgs),
    /// Enable a desktop companion plugin.
    Enable(CompanionPackageArgs),
    /// Disable a desktop companion plugin.
    Disable(CompanionPackageArgs),
    /// Show one plugin package status.
    Status(PluginStatusArgs),
    /// Check whether a package's realtime capability can be activated now.
    ActivateRealtime(ActivateRealtimeArgs),
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Target directory for the generated plugin project.
    pub path: PathBuf,
    /// Plugin package id. Defaults to local.<directory_slug>.
    #[arg(long)]
    pub id: Option<String>,
    /// Public plugin ability name. Defaults to <directory_slug>.echo.
    #[arg(long)]
    pub ability: Option<String>,
    /// Plugin package version.
    #[arg(long, default_value = "0.1.0")]
    pub package_version: String,
    /// Governed AbilityDescriptor version.
    #[arg(long, default_value = "1.0.0")]
    pub descriptor_version: String,
    /// Template language. Python and Node are script-backed; Go, Rust, and Java generate compiled source.
    #[arg(long, value_enum, default_value_t = PluginTemplateLanguage::Python)]
    pub language: PluginTemplateLanguage,
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

#[derive(Debug, Args)]
pub struct CompanionPackageArgs {
    /// Plugin package id.
    pub id: String,
    /// Optional plugin package version.
    #[arg(long)]
    pub version: Option<String>,
}

#[derive(Debug, Args)]
pub struct PluginStatusArgs {
    /// Plugin package id.
    pub id: String,
    /// Optional plugin package version.
    #[arg(long)]
    pub version: Option<String>,
    /// Output JSON instead of an operator table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ActivateRealtimeArgs {
    /// Plugin package id.
    pub id: String,
    /// Optional plugin package version.
    #[arg(long)]
    pub version: Option<String>,
    /// Output format. 'table' is operator-facing; 'json' is stable for scripts.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

pub fn run(args: PluginArgs) -> anyhow::Result<()> {
    match args.action {
        PluginAction::Init(args) => run_init(args),
        PluginAction::List(args) => run_list(args),
        PluginAction::Install(args) => run_install(args),
        PluginAction::Update(args) => run_update(args),
        PluginAction::Remove(args) => run_remove(args),
        PluginAction::Enable(args) => run_enable(args),
        PluginAction::Disable(args) => run_disable(args),
        PluginAction::Status(args) => run_status(args),
        PluginAction::ActivateRealtime(args) => run_activate_realtime(args),
    }
}

fn run_init(args: InitArgs) -> anyhow::Result<()> {
    let project = init_hello_plugin(PluginTemplateInit {
        path: args.path,
        package_id: args.id,
        ability_name: args.ability,
        package_version: args.package_version,
        descriptor_version: args.descriptor_version,
        language: args.language,
    })?;
    output::success(&format!(
        "created {} Hello World plugin {}@{}",
        project.language.label(),
        project.package_id,
        project.package_version
    ));
    output::detail("path", &project.path.display().to_string());
    output::detail("ability", &project.ability_name);
    let next = match project.language {
        PluginTemplateLanguage::Python => {
            format!("easynet plugin install '{}'", project.path.display())
        }
        PluginTemplateLanguage::Go => {
            format!(
                "cd '{}' && make build && easynet plugin install .",
                project.path.display()
            )
        }
        PluginTemplateLanguage::Rust => {
            format!(
                "cd '{}' && make build && easynet plugin install .",
                project.path.display()
            )
        }
        PluginTemplateLanguage::Java => {
            format!(
                "cd '{}' && make build && easynet plugin install .",
                project.path.display()
            )
        }
        PluginTemplateLanguage::Node => {
            format!(
                "cd '{}' && npm install && easynet plugin install .",
                project.path.display()
            )
        }
    };
    output::detail("next", &next);
    Ok(())
}

fn run_list(args: ListArgs) -> anyhow::Result<()> {
    let report = require_plugin_control_value(invoke_plugin_status()?, "plugin list")?;

    match args.format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        OutputFormat::Table => {
            let mut package_table = output::table(&[
                "package",
                "version",
                "kind",
                "planned",
                "daemon",
                "abilities",
                "runtime",
                "invoke",
                "realtime",
                "companion",
                "supervisor",
                "observed",
            ]);
            for package in array_json(&report, "packages")? {
                let realtime = realtime_label(package);
                let companion = companion_field(package, "projected_state");
                let supervisor = companion_field(package, "supervisor_state");
                let observed = companion_field(package, "observed_state");
                package_table.add_row([
                    string_json(package, "package_id"),
                    string_json(package, "package_version"),
                    string_json(package, "kind"),
                    string_json(package, "planned_load_status"),
                    string_json(package, "daemon_runtime_status"),
                    usize_json(package, "ability_count").to_string(),
                    bool_label(bool_json(package, "runtime_published")),
                    bool_label(bool_json(package, "invokable")),
                    realtime,
                    companion,
                    supervisor,
                    observed,
                ]);
            }
            println!("{package_table}");

            let mut ability_table = output::table(&[
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
            for row in array_json(&report, "abilities")? {
                ability_table.add_row([
                    string_json(row, "package_id"),
                    string_json(row, "package_version"),
                    string_json(row, "ability"),
                    string_json(row, "kind"),
                    string_json(row, "call_mode"),
                    string_json(row, "planned_load_status"),
                    string_json(row, "daemon_runtime_status"),
                    bool_label(bool_json(row, "descriptor_published")),
                    bool_label(bool_json(row, "runtime_published")),
                    bool_label(bool_json(row, "invokable")),
                ]);
            }
            println!("{ability_table}");
        }
    }
    Ok(())
}

fn run_install(args: PackageSourceArgs) -> anyhow::Result<()> {
    let installer = PluginInstaller::new(default_plugin_root());
    let companion_manager = DesktopCompanionManager::current();
    let record = installer.install_with_companion_manager(&args.path, &companion_manager)?;
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
    let companion_manager = DesktopCompanionManager::current();
    let record = installer.update_with_companion_manager(&args.path, &companion_manager)?;
    output::success(&format!("updated plugin {}@{}", record.id, record.version));
    output::detail("hash", &record.hash);
    notify_daemon_reload()?;
    Ok(())
}

fn run_remove(args: RemoveArgs) -> anyhow::Result<()> {
    let installer = PluginInstaller::new(default_plugin_root());
    let companion_manager = DesktopCompanionManager::current();
    installer.remove_with_companion_manager(&args.id, &args.version, &companion_manager)?;
    output::success(&format!("removed plugin {}@{}", args.id, args.version));
    notify_daemon_reload()?;
    Ok(())
}

fn run_enable(args: CompanionPackageArgs) -> anyhow::Result<()> {
    let package = resolve_package(&args.id, args.version.as_deref())?;
    let result = DesktopCompanionManager::current().enable(&package)?;
    output::success(&format!(
        "enabled companion {}@{}",
        package.id().as_str(),
        package.version().as_str()
    ));
    output::detail("result", &result.to_string());
    notify_daemon_reload()?;
    Ok(())
}

fn run_disable(args: CompanionPackageArgs) -> anyhow::Result<()> {
    let package = resolve_package(&args.id, args.version.as_deref())?;
    let result = DesktopCompanionManager::current().disable(&package)?;
    output::success(&format!(
        "disabled companion {}@{}",
        package.id().as_str(),
        package.version().as_str()
    ));
    output::detail("result", &result.to_string());
    notify_daemon_reload()?;
    Ok(())
}

fn run_status(args: PluginStatusArgs) -> anyhow::Result<()> {
    let package = resolve_package(&args.id, args.version.as_deref())?;
    if package.manifest().kind() == PluginKind::DesktopCompanion {
        let status = require_plugin_control_value(
            invoke_companion_status(&args.id, args.version.as_deref())?,
            "plugin status",
        )?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&status)?);
        } else {
            print_companion_status(&status);
        }
        return Ok(());
    }

    let report = require_plugin_control_value(invoke_plugin_status()?, "plugin status")?;
    let Some(row) = array_json(&report, "packages")?.iter().find(|row| {
        string_json(row, "package_id") == args.id
            && args
                .version
                .as_deref()
                .map(|version| string_json(row, "package_version") == version)
                .unwrap_or(true)
    }) else {
        anyhow::bail!("plugin package not found: {}", args.id);
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&row)?);
    } else {
        let package = string_json(row, "package_id");
        let version = string_json(row, "package_version");
        let kind = string_json(row, "kind");
        let planned = string_json(row, "planned_load_status");
        let daemon = string_json(row, "daemon_runtime_status");
        output::kv_section(&[
            ("Package", package.as_str()),
            ("Version", version.as_str()),
            ("Kind", kind.as_str()),
            ("Planned", planned.as_str()),
            ("Daemon", daemon.as_str()),
        ]);
    }
    Ok(())
}

fn run_activate_realtime(args: ActivateRealtimeArgs) -> anyhow::Result<()> {
    let Some(report) = invoke_plugin_activate_realtime(&args.id, args.version.as_deref())? else {
        anyhow::bail!(
            "daemon is not running or local invoke is unavailable; start the daemon and retry"
        );
    };
    match args.format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        OutputFormat::Table => print_activation_report(&report)?,
    }
    Ok(())
}

fn bool_label(value: bool) -> String {
    if value {
        "yes".to_string()
    } else {
        "no".to_string()
    }
}

fn realtime_label(row: &Value) -> String {
    let plans = row
        .get("realtime_activation_plans")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if plans.is_empty() {
        return "-".to_string();
    }
    plans
        .iter()
        .map(|plan| {
            let kind = plan
                .pointer("/capability/kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let modes = plan
                .pointer("/capability/modes")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("/");
            let quick = if plan
                .pointer("/capability/quick_add")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                "+quick"
            } else {
                ""
            };
            let status = plan
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let missing_abilities = plan
                .get("missing_abilities")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let missing = if missing_abilities.is_empty() {
                String::new()
            } else {
                format!(" missing={}", string_array_label(missing_abilities))
            };
            format!("{kind}:{modes}{quick}+{status}{missing}")
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn companion_field(row: &Value, field: &str) -> String {
    row.get("companion")
        .and_then(|value| value.get(field))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-")
        .to_string()
}

fn print_companion_status(status: &serde_json::Value) {
    let rows: Vec<(&str, String)> = vec![
        ("Package", string_json(status, "package_id")),
        ("Version", string_json(status, "package_version")),
        ("Display", string_json(status, "display_name")),
        ("Platform", string_json(status, "platform")),
        ("State", string_json(status, "projected_state")),
        ("Desired", string_json(status, "desired_state")),
        ("Supervisor", string_json(status, "supervisor_state")),
        ("Observed", string_json(status, "observed_state")),
    ];
    let kv = rows
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect::<Vec<_>>();
    output::kv_section(&kv);
}

fn string_json(value: &serde_json::Value, field: &str) -> String {
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-")
        .to_string()
}

fn bool_json(value: &Value, field: &str) -> bool {
    value.get(field).and_then(Value::as_bool).unwrap_or(false)
}

fn usize_json(value: &Value, field: &str) -> usize {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

fn array_json<'a>(value: &'a Value, field: &str) -> anyhow::Result<&'a Vec<Value>> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("plugin control response missing `{field}` array"))
}

fn string_array_label(values: &[Value]) -> String {
    values
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("/")
}

fn require_plugin_control_value<T>(value: Option<T>, command: &str) -> anyhow::Result<T> {
    value.ok_or_else(|| {
        anyhow::anyhow!(
            "`easynet {command}` requires the daemon plugin control ability; \
             start the daemon and retry"
        )
    })
}

fn resolve_package(
    id: &str,
    version: Option<&str>,
) -> anyhow::Result<crate::daemon::plugins::package::SharedPluginPackage> {
    let index_report = PluginPackageIndex::load_default_resilient()?;
    let (index, _) = index_report.into_parts();
    let matches = index
        .packages()
        .iter()
        .filter(|package| {
            package.id().as_str() == id
                && version
                    .map(|version| package.version().as_str() == version)
                    .unwrap_or(true)
        })
        .cloned()
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [package] => Ok(package.clone()),
        [] => anyhow::bail!("plugin package not found: {id}"),
        _ => anyhow::bail!("multiple plugin versions found for {id}; pass --version"),
    }
}

fn print_activation_report(report: &Value) -> anyhow::Result<()> {
    let mut table = output::table(&[
        "package",
        "version",
        "capability",
        "transport",
        "status",
        "resources",
        "abilities",
        "permissions",
        "publish",
    ]);
    for outcome in array_json(report, "outcomes")? {
        table.add_row([
            string_json(outcome, "package_id"),
            string_json(outcome, "package_version"),
            activation_capability_label(outcome),
            activation_transport_label(outcome),
            string_json(outcome, "status"),
            activation_resources_label(outcome),
            activation_abilities_label(outcome),
            activation_permissions_label(outcome),
            outcome
                .get("publish")
                .map(|publish| string_json(publish, "realm_advertise"))
                .unwrap_or_else(|| "-".to_string()),
        ]);
    }
    println!("{table}");
    Ok(())
}

fn activation_transport_label(outcome: &Value) -> String {
    let Some(transport) = outcome.get("transport") else {
        return "-".to_string();
    };
    let status = string_json(transport, "status");
    match transport.get("selected").and_then(Value::as_str) {
        Some(selected) => format!("{selected}:{status}"),
        None => status,
    }
}

fn activation_capability_label(outcome: &Value) -> String {
    let Some(capability) = outcome.get("capability") else {
        return "-".to_string();
    };
    let kind = string_json(capability, "kind");
    let modes = capability
        .get("modes")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("/");
    if modes.is_empty() {
        kind.to_string()
    } else {
        format!("{kind}:{modes}")
    }
}

fn activation_resources_label(outcome: &Value) -> String {
    let Some(resources) = outcome.get("resources") else {
        return "-".to_string();
    };
    let missing = resources
        .get("missing")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if !missing.is_empty() {
        return format!("missing={}", string_array_label(missing));
    }
    let available = resources
        .get("available")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|item| {
            format!(
                "{}={}",
                string_json(item, "kind"),
                item.get("count").and_then(Value::as_u64).unwrap_or(0)
            )
        })
        .collect::<Vec<_>>();
    if available.is_empty() {
        "ready".to_string()
    } else {
        available.join("/")
    }
}

fn activation_abilities_label(outcome: &Value) -> String {
    let missing = outcome
        .get("missing_abilities")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if !missing.is_empty() {
        return format!("missing={}", string_array_label(missing));
    }
    let available = outcome
        .get("available_abilities")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if available.is_empty() {
        "-".to_string()
    } else {
        string_array_label(available)
    }
}

fn activation_permissions_label(outcome: &Value) -> String {
    let Some(permissions) = outcome.get("permissions") else {
        return "-".to_string();
    };
    let required = permissions
        .get("required")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if required.is_empty() {
        return "not_required".to_string();
    }
    format!(
        "{}:{}",
        string_array_label(required),
        string_json(permissions, "status")
    )
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
    invoke_plugin_control_ability(
        crate::daemon::ability::builtins::integrations::plugins::RELOAD_ABILITY,
        serde_json::json!({}),
    )
}

fn invoke_plugin_status() -> anyhow::Result<Option<Value>> {
    let Some(value) = invoke_plugin_control_ability(
        crate::daemon::ability::builtins::integrations::plugins::STATUS_ABILITY,
        serde_json::json!({}),
    )?
    else {
        return Ok(None);
    };
    Ok(Some(value))
}

fn invoke_plugin_activate_realtime(
    id: &str,
    version: Option<&str>,
) -> anyhow::Result<Option<Value>> {
    let mut body = serde_json::json!({ "package_id": id });
    if let Some(version) = version {
        body["package_version"] = serde_json::json!(version);
    }
    let Some(value) = invoke_plugin_control_ability(
        crate::daemon::ability::builtins::integrations::plugins::ACTIVATE_REALTIME_ABILITY,
        body,
    )?
    else {
        return Ok(None);
    };
    Ok(Some(value))
}

fn invoke_companion_status(
    id: &str,
    version: Option<&str>,
) -> anyhow::Result<Option<serde_json::Value>> {
    let mut body = serde_json::json!({ "package_id": id });
    if let Some(version) = version {
        body["package_version"] = serde_json::json!(version);
    }
    invoke_plugin_control_ability(
        crate::daemon::ability::builtins::integrations::plugins::COMPANION_STATUS_ABILITY,
        body,
    )
}

fn invoke_plugin_control_ability(
    ability: &'static str,
    args: serde_json::Value,
) -> anyhow::Result<Option<serde_json::Value>> {
    #[cfg(feature = "axon-pb")]
    {
        invoke_plugin_control_ability_via_daemon(ability, args)
    }

    #[cfg(not(feature = "axon-pb"))]
    {
        let _ = ability;
        let _ = args;
        Ok(None)
    }
}

#[cfg(feature = "axon-pb")]
fn invoke_plugin_control_ability_via_daemon(
    ability: &'static str,
    args: serde_json::Value,
) -> anyhow::Result<Option<serde_json::Value>> {
    let subject = match PluginControlSubject::resolve()? {
        PluginControlSubject::Available(subject) => subject,
        PluginControlSubject::Unpaired => return Ok(None),
    };
    match crate::support::platform::local_invoke::LocalDaemonSystemAbilityIssuer::invoke_root_for_subject(
        ability, args, &subject,
    ) {
        Ok(value) => Ok(Some(value)),
        Err(err)
            if crate::support::platform::local_invoke::classify_invoke_failure(&err)
                == crate::support::platform::local_invoke::LocalInvokeFailureClass::DaemonOffline =>
        {
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

#[cfg(feature = "axon-pb")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum PluginControlSubject {
    Available(String),
    Unpaired,
}

#[cfg(feature = "axon-pb")]
impl PluginControlSubject {
    fn resolve() -> anyhow::Result<Self> {
        let Some(creds) = crate::daemon::persistence::config::load_credentials_optional()? else {
            return Ok(Self::Unpaired);
        };
        Ok(Self::Available(crate::core::ura::device_ura(
            creds.realm.trim(),
            creds.node_id.trim(),
        )))
    }
}

#[cfg(all(test, feature = "axon-pb"))]
mod tests {
    use super::*;

    #[test]
    fn plugin_control_value_accepts_daemon_authority_value() {
        let status = serde_json::json!({
            "kind": "desktop_companion_status",
            "package_id": "easynet.desktop.menubar",
            "projected_state": "running"
        });

        let selected = require_plugin_control_value(Some(status.clone()), "plugin status")
            .expect("daemon value should be accepted");

        assert_eq!(selected, status);
    }

    #[test]
    fn plugin_control_value_rejects_missing_daemon_authority() {
        let err = require_plugin_control_value::<serde_json::Value>(None, "plugin status")
            .expect_err("missing daemon value must fail closed");

        assert!(err
            .to_string()
            .contains("requires the daemon plugin control ability"));
    }

    #[test]
    fn plugin_control_subject_is_unavailable_when_unpaired() {
        let _guard = crate::cli::commands::test_support::HomeGuard::new();

        assert_eq!(
            PluginControlSubject::resolve().unwrap(),
            PluginControlSubject::Unpaired
        );
    }

    #[test]
    fn plugin_control_subject_rejects_malformed_credentials() {
        let _guard = crate::cli::commands::test_support::HomeGuard::new();
        std::fs::create_dir_all(crate::daemon::persistence::config::state_dir())
            .expect("state dir");
        std::fs::write(
            crate::daemon::persistence::config::state_dir().join("credentials.json"),
            b"{",
        )
        .expect("write malformed credentials");

        let err = PluginControlSubject::resolve()
            .expect_err("malformed credentials must not look like unpaired state");

        assert!(
            err.to_string().contains("parse credentials"),
            "wrong error: {err}"
        );
    }

    #[test]
    fn plugin_control_subject_rejects_incomplete_credentials() {
        let _guard = crate::cli::commands::test_support::HomeGuard::new();
        std::fs::create_dir_all(crate::daemon::persistence::config::state_dir())
            .expect("state dir");
        std::fs::write(
            crate::daemon::persistence::config::state_dir().join("credentials.json"),
            r#"{
  "node_id": "",
  "credential_token": "token",
  "hub_endpoint": "axon://hub.example:7700",
  "realm": "acme",
  "username": "alice",
  "user_id": "user-alice"
}
"#,
        )
        .expect("write incomplete credentials");

        let err = PluginControlSubject::resolve()
            .expect_err("incomplete credentials must not look like unpaired state");

        assert!(
            err.to_string().contains("validate credentials"),
            "wrong error: {err}"
        );
    }

    #[test]
    fn plugin_control_subject_uses_paired_device_ura() {
        let _guard = crate::cli::commands::test_support::HomeGuard::new();
        crate::daemon::persistence::config::save_credentials(
            &crate::daemon::persistence::config::Credentials {
                node_id: "dev-a".to_string(),
                credential_token: "token".to_string(),
                realm: "acme".to_string(),
                hub_endpoint: "axon://hub.example:50051".to_string(),
                username: Some("alice".to_string()),
                user_id: Some("user-alice".to_string()),
                ..Default::default()
            },
        )
        .expect("write test credentials");

        assert_eq!(
            PluginControlSubject::resolve().unwrap(),
            PluginControlSubject::Available("easynet:///r/acme/device/dev-a".to_string())
        );
    }
}
