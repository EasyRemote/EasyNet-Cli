// EasyNet CLI — Plugin Group
// ===========================
//
// File: src/cli/groups/plugin.rs
// Description: `easynet plugin ...` package lifecycle and boot-state query.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use super::plugin_template::{init_hello_plugin, PluginTemplateInit, PluginTemplateLanguage};
use crate::daemon::plugins::index::default_plugin_root;
use crate::daemon::plugins::{
    DesktopCompanionManager, PluginInstaller, PluginKind, PluginKindView, PluginLoadPlanner,
    PluginPackageIndex, PluginPackageSurfaceRecord, PluginRealtimeActivationOutcome,
    PluginRealtimeActivationReport, PluginRealtimeKind, PluginRealtimeMode,
    PluginRealtimeOutcomeStatus, PluginRealtimePermissionStatus, PluginRealtimeTransport,
    PluginRealtimeTransportReadinessStatus, PluginSurfaceProjector, PluginSurfaceReport,
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
    /// Template language. Python is zero-build; Go generates compiled source.
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
    let report = match invoke_plugin_status()? {
        Some(report) => report,
        None => {
            output::warn("daemon is not running; showing offline planned plugin status");
            let index_report = PluginPackageIndex::load_default_resilient()?;
            let (index, index_errors) = index_report.into_parts();
            let plan = PluginLoadPlanner::current().plan(&index);
            PluginSurfaceProjector::project_report_with_daemon(&index, &plan, None, &index_errors)
        }
    };

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
            for package in report.packages {
                let realtime = realtime_label(&package);
                let companion = companion_field(&package, "projected_state");
                let supervisor = companion_field(&package, "supervisor_state");
                let observed = companion_field(&package, "observed_state");
                package_table.add_row([
                    package.package_id,
                    package.package_version,
                    plugin_kind_label(package.kind).to_string(),
                    package.planned_load_status,
                    package.daemon_runtime_status,
                    package.ability_count.to_string(),
                    bool_label(package.runtime_published),
                    bool_label(package.invokable),
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
            for row in report.abilities {
                ability_table.add_row([
                    row.package_id,
                    row.package_version,
                    row.ability,
                    plugin_kind_label(row.kind).to_string(),
                    format!("{:?}", row.call_mode).to_ascii_lowercase(),
                    row.planned_load_status,
                    row.daemon_runtime_status,
                    bool_label(row.descriptor_published),
                    bool_label(row.runtime_published),
                    bool_label(row.invokable),
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
        let local_status = DesktopCompanionManager::current().status_json(&package)?;
        let daemon_status = invoke_companion_status(&args.id, args.version.as_deref())?;
        let selected = select_companion_status(local_status, daemon_status);
        if let Some(warning) = selected.warning {
            output::warn(&warning);
        }
        let status = selected.status;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&status)?);
        } else {
            print_companion_status(&status);
        }
        return Ok(());
    }

    let report = offline_plugin_surface_report()?;
    let Some(row) = report.packages.into_iter().find(|row| {
        row.package_id == args.id
            && args
                .version
                .as_deref()
                .map(|version| row.package_version == version)
                .unwrap_or(true)
    }) else {
        anyhow::bail!("plugin package not found: {}", args.id);
    };
    if args.json {
        println!("{}", serde_json::to_string_pretty(&row)?);
    } else {
        output::kv_section(&[
            ("Package", row.package_id.as_str()),
            ("Version", row.package_version.as_str()),
            ("Kind", plugin_kind_label(row.kind)),
            ("Planned", row.planned_load_status.as_str()),
            ("Daemon", row.daemon_runtime_status.as_str()),
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
        OutputFormat::Table => print_activation_report(&report),
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

fn plugin_kind_label(kind: PluginKindView) -> &'static str {
    match kind {
        PluginKindView::Declarative => "declarative",
        PluginKindView::Sidecar => "sidecar",
        PluginKindView::Builtin => "builtin",
        PluginKindView::DesktopCompanion => "desktop_companion",
        PluginKindView::Unknown => "unknown",
    }
}

fn realtime_label(row: &PluginPackageSurfaceRecord) -> String {
    if row.realtime_activation_plans.is_empty() {
        return "-".to_string();
    }
    row.realtime_activation_plans
        .iter()
        .map(|plan| {
            let kind = format!("{:?}", plan.capability.kind()).to_ascii_lowercase();
            let modes = plan
                .capability
                .modes()
                .iter()
                .map(|mode| format!("{mode:?}").to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join("/");
            let quick = if plan.is_quick_add() { "+quick" } else { "" };
            let status = format!("{:?}", plan.status).to_ascii_lowercase();
            let missing = if plan.missing_abilities.is_empty() {
                String::new()
            } else {
                format!(" missing={}", plan.missing_abilities.join("/"))
            };
            format!("{kind}:{modes}{quick}+{status}{missing}")
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn companion_field(row: &PluginPackageSurfaceRecord, field: &str) -> String {
    row.companion
        .as_ref()
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

struct CompanionStatusSelection {
    status: serde_json::Value,
    warning: Option<String>,
}

fn select_companion_status(
    local_status: serde_json::Value,
    daemon_status: Option<serde_json::Value>,
) -> CompanionStatusSelection {
    match daemon_status {
        Some(daemon_status) if daemon_status == local_status => CompanionStatusSelection {
            status: daemon_status,
            warning: None,
        },
        Some(_) => CompanionStatusSelection {
            status: local_status,
            warning: Some(
                "daemon companion plugin state may be stale; showing local manager observation"
                    .to_string(),
            ),
        },
        None => CompanionStatusSelection {
            status: local_status,
            warning: None,
        },
    }
}

fn offline_plugin_surface_report() -> anyhow::Result<PluginSurfaceReport> {
    let index_report = PluginPackageIndex::load_default_resilient()?;
    let (index, index_errors) = index_report.into_parts();
    let plan = PluginLoadPlanner::current().plan(&index);
    Ok(PluginSurfaceProjector::project_report_with_daemon(
        &index,
        &plan,
        None,
        &index_errors,
    ))
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

fn print_activation_report(report: &PluginRealtimeActivationReport) {
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
    for outcome in &report.outcomes {
        table.add_row([
            outcome.package_id.clone(),
            outcome.package_version.clone(),
            activation_capability_label(outcome),
            activation_transport_label(outcome),
            outcome_status_label(outcome.status).to_string(),
            activation_resources_label(outcome),
            activation_abilities_label(outcome),
            activation_permissions_label(outcome),
            outcome.publish.realm_advertise.clone(),
        ]);
    }
    println!("{table}");
}

fn activation_transport_label(outcome: &PluginRealtimeActivationOutcome) -> String {
    let status = transport_readiness_status_label(outcome.transport.status);
    match outcome.transport.selected {
        Some(selected) => format!("{}:{status}", transport_label(selected)),
        None => status.to_string(),
    }
}

fn activation_capability_label(outcome: &PluginRealtimeActivationOutcome) -> String {
    let kind = realtime_kind_label(outcome.capability.kind());
    let modes = outcome
        .capability
        .modes()
        .iter()
        .map(|mode| realtime_mode_label(*mode))
        .collect::<Vec<_>>()
        .join("/");
    if modes.is_empty() {
        kind.to_string()
    } else {
        format!("{kind}:{modes}")
    }
}

fn activation_resources_label(outcome: &PluginRealtimeActivationOutcome) -> String {
    if !outcome.resources.missing.is_empty() {
        return format!("missing={}", outcome.resources.missing.join("/"));
    }
    let available = outcome
        .resources
        .available
        .iter()
        .map(|item| format!("{}={}", item.kind, item.count))
        .collect::<Vec<_>>();
    if available.is_empty() {
        "ready".to_string()
    } else {
        available.join("/")
    }
}

fn activation_abilities_label(outcome: &PluginRealtimeActivationOutcome) -> String {
    if !outcome.missing_abilities.is_empty() {
        return format!("missing={}", outcome.missing_abilities.join("/"));
    }
    if outcome.available_abilities.is_empty() {
        "-".to_string()
    } else {
        outcome.available_abilities.join("/")
    }
}

fn activation_permissions_label(outcome: &PluginRealtimeActivationOutcome) -> String {
    if outcome.permissions.required.is_empty() {
        return "not_required".to_string();
    }
    format!(
        "{}:{}",
        outcome.permissions.required.join("/"),
        permission_status_label(outcome.permissions.status)
    )
}

fn outcome_status_label(status: PluginRealtimeOutcomeStatus) -> &'static str {
    match status {
        PluginRealtimeOutcomeStatus::Ready => "ready",
        PluginRealtimeOutcomeStatus::Blocked => "blocked",
        PluginRealtimeOutcomeStatus::Partial => "partial",
        PluginRealtimeOutcomeStatus::Unsupported => "unsupported",
        PluginRealtimeOutcomeStatus::Unknown => "unknown",
    }
}

fn permission_status_label(status: PluginRealtimePermissionStatus) -> &'static str {
    match status {
        PluginRealtimePermissionStatus::NotRequired => "not_required",
        PluginRealtimePermissionStatus::StatusAbilityAvailable => "status_ability_available",
        PluginRealtimePermissionStatus::RequestAbilityAvailable => "request_ability_available",
        PluginRealtimePermissionStatus::Unknown => "unknown",
    }
}

fn transport_readiness_status_label(
    status: PluginRealtimeTransportReadinessStatus,
) -> &'static str {
    match status {
        PluginRealtimeTransportReadinessStatus::Unknown => "unknown",
        PluginRealtimeTransportReadinessStatus::Ready => "ready",
        PluginRealtimeTransportReadinessStatus::FallbackReady => "fallback_ready",
        PluginRealtimeTransportReadinessStatus::Blocked => "blocked",
    }
}

fn transport_label(transport: PluginRealtimeTransport) -> &'static str {
    match transport {
        PluginRealtimeTransport::InvokeStream => "invoke_stream",
        PluginRealtimeTransport::InvokeBidi => "invoke_bidi",
        PluginRealtimeTransport::Webrtc => "webrtc",
    }
}

fn realtime_kind_label(kind: PluginRealtimeKind) -> &'static str {
    match kind {
        PluginRealtimeKind::Camera => "camera",
        PluginRealtimeKind::Mic => "mic",
        PluginRealtimeKind::Screen => "screen",
        PluginRealtimeKind::Speaker => "speaker",
        PluginRealtimeKind::Voice => "voice",
    }
}

fn realtime_mode_label(mode: PluginRealtimeMode) -> &'static str {
    match mode {
        PluginRealtimeMode::Snapshot => "snapshot",
        PluginRealtimeMode::Subscribe => "subscribe",
        PluginRealtimeMode::Record => "record",
        PluginRealtimeMode::Publish => "publish",
        PluginRealtimeMode::Transcribe => "transcribe",
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
    invoke_plugin_control_ability(
        crate::daemon::ability::builtins::integrations::plugins::RELOAD_ABILITY,
        serde_json::json!({}),
    )
}

fn invoke_plugin_status() -> anyhow::Result<Option<PluginSurfaceReport>> {
    let Some(value) = invoke_plugin_control_ability(
        crate::daemon::ability::builtins::integrations::plugins::STATUS_ABILITY,
        serde_json::json!({}),
    )?
    else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_value(value)?))
}

fn invoke_plugin_activate_realtime(
    id: &str,
    version: Option<&str>,
) -> anyhow::Result<Option<PluginRealtimeActivationReport>> {
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
    Ok(Some(serde_json::from_value(value)?))
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
    let Some(subject) = plugin_control_subject_ura()? else {
        return Ok(None);
    };
    match crate::support::platform::local_invoke::invoke_local_ability_with_subject(
        ability,
        args,
        Some(subject),
    ) {
        Ok(value) => Ok(Some(value)),
        Err(err)
            if crate::support::platform::local_invoke::classify_invoke_error(&err)
                == crate::support::platform::local_invoke::LocalInvokeErrorKind::DaemonOffline =>
        {
            Ok(None)
        }
        Err(err) => Err(err),
    }
}

#[cfg(feature = "axon-pb")]
fn plugin_control_subject_ura() -> anyhow::Result<Option<String>> {
    match crate::daemon::persistence::config::load_credentials() {
        Ok(creds) => Ok(Some(crate::core::ura::device_ura(
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
    fn companion_status_selection_uses_daemon_when_equal() {
        let status = serde_json::json!({
            "kind": "desktop_companion_status",
            "package_id": "easynet.desktop.menubar",
            "projected_state": "running"
        });

        let selected = select_companion_status(status.clone(), Some(status.clone()));

        assert_eq!(selected.status, status);
        assert!(selected.warning.is_none());
    }

    #[test]
    fn companion_status_selection_warns_and_uses_local_when_daemon_differs() {
        let local_status = serde_json::json!({
            "kind": "desktop_companion_status",
            "package_id": "easynet.desktop.menubar",
            "projected_state": "running"
        });
        let daemon_status = serde_json::json!({
            "kind": "desktop_companion_status",
            "package_id": "easynet.desktop.menubar",
            "projected_state": "ready_stopped"
        });

        let selected = select_companion_status(local_status.clone(), Some(daemon_status));

        assert_eq!(selected.status, local_status);
        assert_eq!(
            selected.warning.as_deref(),
            Some("daemon companion plugin state may be stale; showing local manager observation")
        );
    }

    #[test]
    fn plugin_control_subject_is_unavailable_when_unpaired() {
        let _guard = crate::cli::commands::test_support::HomeGuard::new();

        assert_eq!(plugin_control_subject_ura().unwrap(), None);
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
            plugin_control_subject_ura().unwrap().as_deref(),
            Some("easynet:///r/acme/device/dev-a")
        );
    }
}
