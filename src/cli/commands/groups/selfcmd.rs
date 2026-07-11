// EasyNet CLI — Self Management
// ==============================
//
// File: src/cli/groups/selfcmd.rs
// Description: `easynet self …` — check for updates, update in-place, and
//              fully uninstall the CLI + runtime + configuration.
//
// Update mechanism:
//   Fetches the latest version tag from the GitHub release API, compares
//   with the compiled-in version, and re-runs the install script to
//   upgrade in-place.
//
// Uninstall mechanism:
//   Removes binaries (easynet, axon-runtime), native libraries
//   (dendrite bridge), ~/.easynet data directory, and the
//   EASYNET_DENDRITE_BRIDGE_LIB env var from the shell profile.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::{Args, Subcommand};

use crate::cli::commands::stop::{self, StopOptions};
use crate::daemon::persistence::config;
use crate::support::platform::output;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const HEALTH_API: &str = "https://easynet.run/api/v1/health";
const INSTALL_SCRIPT_URL: &str = "https://easynet.run/install";

#[derive(Debug, Args)]
pub struct SelfArgs {
    #[command(subcommand)]
    pub action: SelfAction,
}

#[derive(Debug, Subcommand)]
pub enum SelfAction {
    /// Check if a newer version is available.
    Check,
    /// Update to the latest version.
    Update,
    /// Completely uninstall EasyNet CLI, runtime, and configuration.
    Uninstall(UninstallArgs),
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
    /// Skip the interactive confirmation (non-interactive / CI use).
    #[arg(long, short = 'y')]
    pub yes: bool,
}

pub fn run(args: SelfArgs) -> anyhow::Result<()> {
    match args.action {
        SelfAction::Check => run_check(),
        SelfAction::Update => run_update(),
        SelfAction::Uninstall(a) => run_uninstall(a),
    }
}

// ── Check ──────────────────────────────────────────────────────────────

fn fetch_latest_version() -> anyhow::Result<String> {
    let output = std::process::Command::new("curl")
        .args(["-sSfL", HEALTH_API])
        .output()
        .context("failed to reach easynet.run")?;

    if !output.status.success() {
        anyhow::bail!("easynet.run health check failed (status {})", output.status);
    }

    let body: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("failed to parse health response")?;

    let version = body["cli_version"].as_str().unwrap_or("").to_string();

    if version.is_empty() {
        anyhow::bail!("easynet.run did not return cli_version in health response");
    }
    Ok(version)
}

/// Compare two semver-ish version strings (e.g. "1.0.1" vs "0.1.5").
/// Returns true if `remote` is strictly newer than `local`.
fn is_newer(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> { s.split('.').filter_map(|p| p.parse().ok()).collect() };
    let r = parse(remote);
    let l = parse(local);
    for i in 0..r.len().max(l.len()) {
        let rv = r.get(i).copied().unwrap_or(0);
        let lv = l.get(i).copied().unwrap_or(0);
        if rv > lv {
            return true;
        }
        if rv < lv {
            return false;
        }
    }
    false
}

fn run_check() -> anyhow::Result<()> {
    output::step(&format!("Current version: {CURRENT_VERSION}"));
    output::step("Checking for updates...");

    let latest = fetch_latest_version()?;

    if is_newer(&latest, CURRENT_VERSION) {
        output::step(&format!("New version available: {latest}"));
        output::step("Run 'easynet self update' to install it.");
    } else {
        output::success(&format!(
            "You are on the latest version ({CURRENT_VERSION})"
        ));
    }
    Ok(())
}

// ── Update ─────────────────────────────────────────────────────────────

fn run_update() -> anyhow::Result<()> {
    output::step(&format!("Current version: {CURRENT_VERSION}"));
    output::step("Checking for updates...");

    let latest = fetch_latest_version()?;

    if !is_newer(&latest, CURRENT_VERSION) {
        output::success(&format!("Already up to date ({CURRENT_VERSION})"));
        return Ok(());
    }

    output::step(&format!("Updating {CURRENT_VERSION} → {latest}..."));

    // Re-run the install script which handles platform detection,
    // binary download, env setup, and stale binary cleanup.
    let status = if cfg!(target_os = "windows") {
        std::process::Command::new("powershell")
            .args([
                "-Command",
                &format!(
                    "irm {} | iex",
                    INSTALL_SCRIPT_URL.replace("/install", "/install.ps1")
                ),
            ])
            .status()
            .context("failed to run PowerShell install script")?
    } else {
        std::process::Command::new("sh")
            .args(["-c", &format!("curl -sSf {} | sh", INSTALL_SCRIPT_URL)])
            .status()
            .context("failed to run install script")?
    };

    if status.success() {
        output::success(&format!("Updated to {latest}"));
    } else {
        anyhow::bail!("update failed (exit code {:?})", status.code());
    }
    Ok(())
}

// ── Uninstall ──────────────────────────────────────────────────────────

fn run_uninstall(args: UninstallArgs) -> anyhow::Result<()> {
    if !args.yes {
        eprintln!("This will remove:");
        eprintln!("  - easynet and axon-runtime binaries");
        eprintln!("  - Dendrite bridge native library");
        eprintln!("  - ~/.easynet directory (config, logs, runtime data)");
        eprintln!("  - EASYNET_DENDRITE_BRIDGE_LIB from your shell profile");
        eprintln!();
        eprint!("Continue? [y/N] ");

        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            output::step("Cancelled.");
            return Ok(());
        }
    }

    let mut env = ProductionUninstallEnvironment;
    SelfUninstallPlan.execute(&mut env)?;

    output::success("EasyNet CLI uninstalled");
    eprintln!();
    eprintln!("  To complete removal, restart your terminal or run:");
    if cfg!(target_os = "windows") {
        eprintln!("    refreshenv");
    } else {
        // Detect which profile we should suggest sourcing.
        let shell = std::env::var("SHELL").unwrap_or_default();
        if shell.ends_with("zsh") {
            eprintln!("    source ~/.zshrc");
        } else {
            eprintln!("    source ~/.bashrc");
        }
    }
    eprintln!();
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceIdentity {
    device_ura: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HubRemovalReport {
    Reported,
    Skipped(String),
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelfUninstallStage {
    CaptureIdentity,
    ReportHubRemoval,
    StopRuntime,
    RemoveDesktopCompanions,
    RemoveBinaries,
    RemoveData,
    CleanShellProfile,
    Complete,
}

trait UninstallEnvironment {
    fn record_stage(&mut self, _stage: SelfUninstallStage) {}

    fn capture_device_identity(&mut self) -> Option<DeviceIdentity>;

    fn report_hub_removal(&mut self, identity: &DeviceIdentity) -> HubRemovalReport;

    fn stop_runtime_without_revoke(&mut self) -> Result<(), String>;

    fn remove_desktop_companions(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn binary_paths(&self) -> Vec<PathBuf>;

    fn remove_binary_if_exists(&mut self, path: &Path) -> bool;

    fn home_dir(&self) -> Option<PathBuf>;

    fn remove_data_dir_if_exists(&mut self, path: &Path) -> bool;

    fn clean_shell_profile(&mut self, home: &Path);
}

#[derive(Debug, Default)]
struct SelfUninstallPlan;

impl SelfUninstallPlan {
    fn execute<E: UninstallEnvironment>(&self, env: &mut E) -> anyhow::Result<()> {
        env.record_stage(SelfUninstallStage::CaptureIdentity);
        let identity = env.capture_device_identity();

        env.record_stage(SelfUninstallStage::ReportHubRemoval);
        match identity.as_ref() {
            Some(identity) => match env.report_hub_removal(identity) {
                HubRemovalReport::Reported => output::step("Reported device removal to hub"),
                HubRemovalReport::Skipped(reason) => {
                    output::step(&format!("Hub removal report skipped ({reason})"))
                }
                HubRemovalReport::Failed(err) => output::warn(&format!(
                    "Hub removal report failed (continuing uninstall): {err}"
                )),
            },
            None => output::step("Hub removal report skipped (no local device credentials)"),
        }

        env.record_stage(SelfUninstallStage::StopRuntime);
        output::step("Stopping runtime...");
        if let Err(err) = env.stop_runtime_without_revoke() {
            output::warn(&format!(
                "Runtime stop failed (continuing uninstall): {err}"
            ));
        }

        env.record_stage(SelfUninstallStage::RemoveDesktopCompanions);
        output::step("Removing desktop companions...");
        if let Err(err) = env.remove_desktop_companions() {
            output::warn(&format!(
                "Desktop companion cleanup failed (continuing uninstall): {err}"
            ));
        }

        env.record_stage(SelfUninstallStage::RemoveBinaries);
        for path in env.binary_paths() {
            if env.remove_binary_if_exists(&path) {
                output::step(&format!("Removing {}", path.display()));
            }
        }

        let home = env.home_dir();
        env.record_stage(SelfUninstallStage::RemoveData);
        match home.as_ref() {
            Some(home) => {
                let easynet_home = home.join(".easynet");
                if env.remove_data_dir_if_exists(&easynet_home) {
                    output::step(&format!("Removing {}", easynet_home.display()));
                }
            }
            None => output::step("Skipping ~/.easynet removal (home directory unavailable)"),
        }

        env.record_stage(SelfUninstallStage::CleanShellProfile);
        if let Some(home) = home.as_ref() {
            env.clean_shell_profile(home);
        } else {
            output::step("Skipping shell profile cleanup (home directory unavailable)");
        }

        env.record_stage(SelfUninstallStage::Complete);
        Ok(())
    }
}

struct ProductionUninstallEnvironment;

impl UninstallEnvironment for ProductionUninstallEnvironment {
    fn capture_device_identity(&mut self) -> Option<DeviceIdentity> {
        let creds = config::load_credentials().ok()?;
        Some(DeviceIdentity {
            device_ura: crate::core::ura::device_ura(&creds.realm, &creds.node_id),
        })
    }

    fn report_hub_removal(&mut self, identity: &DeviceIdentity) -> HubRemovalReport {
        if identity.device_ura.trim().is_empty() {
            return HubRemovalReport::Skipped("empty device identity".to_string());
        }

        #[cfg(feature = "axon-pb")]
        {
            match crate::daemon::invocation::routing::remote_invoke::invoke_federation_revoke(
                &identity.device_ura,
                "self uninstall",
            ) {
                Ok(()) => HubRemovalReport::Reported,
                Err(err) => HubRemovalReport::Failed(err.to_string()),
            }
        }

        #[cfg(not(feature = "axon-pb"))]
        {
            let _ = identity;
            HubRemovalReport::Skipped("axon-pb feature disabled".to_string())
        }
    }

    fn stop_runtime_without_revoke(&mut self) -> Result<(), String> {
        stop::run_with_options(stop::StopArgs {}, StopOptions { skip_revoke: true })
            .map_err(|err| err.to_string())
    }

    fn remove_desktop_companions(&mut self) -> Result<(), String> {
        let state = crate::daemon::plugins::default_state().map_err(|err| err.to_string())?;
        let manager = crate::daemon::plugins::DesktopCompanionManager::current();
        let warnings = manager.cleanup_for_self_uninstall(state.index().packages());
        if warnings.is_empty() {
            Ok(())
        } else {
            Err(warnings.join("; "))
        }
    }

    fn binary_paths(&self) -> Vec<PathBuf> {
        ["easynet", "axon-runtime"]
            .into_iter()
            .flat_map(|bin| {
                ["/usr/local/bin", "/usr/bin"]
                    .into_iter()
                    .map(move |dir| Path::new(dir).join(bin))
            })
            .collect()
    }

    fn remove_binary_if_exists(&mut self, path: &Path) -> bool {
        if !path.exists() {
            return false;
        }
        let removed = std::fs::remove_file(path).or_else(|_| {
            std::process::Command::new("sudo")
                .arg("rm")
                .arg("-f")
                .arg(path)
                .status()
                .map(|_| ())
        });
        removed.is_ok()
    }

    fn home_dir(&self) -> Option<PathBuf> {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
    }

    fn remove_data_dir_if_exists(&mut self, path: &Path) -> bool {
        if !path.exists() {
            return false;
        }
        std::fs::remove_dir_all(path).is_ok()
    }

    fn clean_shell_profile(&mut self, home: &Path) {
        clean_shell_profile(home);
    }
}

fn clean_shell_profile(home: &Path) {
    let profiles = [
        home.join(".zshrc"),
        home.join(".bashrc"),
        home.join(".bash_profile"),
        home.join(".profile"),
    ];

    for profile_path in &profiles {
        if !profile_path.exists() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(profile_path) else {
            continue;
        };
        if !content.contains("EASYNET_DENDRITE_BRIDGE_LIB") {
            continue;
        }

        // Remove the EasyNet env lines.
        let cleaned: Vec<&str> = content
            .lines()
            .filter(|line| {
                !line.contains("EASYNET_DENDRITE_BRIDGE_LIB")
                    && !line.contains("# EasyNet dendrite bridge")
            })
            .collect();

        // Trim trailing blank lines left behind.
        let mut result: Vec<&str> = cleaned.into_iter().collect();
        while result.last().is_some_and(|l| l.trim().is_empty()) {
            result.pop();
        }
        // Ensure file ends with newline.
        let mut out = result.join("\n");
        if !out.ends_with('\n') {
            out.push('\n');
        }

        if std::fs::write(profile_path, &out).is_ok() {
            output::step(&format!(
                "Cleaned EASYNET_DENDRITE_BRIDGE_LIB from {}",
                profile_path.display()
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FakeUninstallEnvironment {
        stages: Vec<SelfUninstallStage>,
        identity: Option<DeviceIdentity>,
        report_result: HubRemovalReport,
        stop_result: Result<(), String>,
        binary_paths: Vec<PathBuf>,
        removed_binaries: Vec<PathBuf>,
        home: Option<PathBuf>,
        removed_data_dirs: Vec<PathBuf>,
        cleaned_profiles: Vec<PathBuf>,
        report_targets: Vec<String>,
        stop_calls: usize,
    }

    impl FakeUninstallEnvironment {
        fn with_identity() -> Self {
            Self {
                stages: Vec::new(),
                identity: Some(DeviceIdentity {
                    device_ura: "easynet:///r/acme/device/01DEV".to_string(),
                }),
                report_result: HubRemovalReport::Reported,
                stop_result: Ok(()),
                binary_paths: vec![
                    PathBuf::from("/usr/local/bin/easynet"),
                    PathBuf::from("/usr/local/bin/axon-runtime"),
                ],
                removed_binaries: Vec::new(),
                home: Some(PathBuf::from("/home/alice")),
                removed_data_dirs: Vec::new(),
                cleaned_profiles: Vec::new(),
                report_targets: Vec::new(),
                stop_calls: 0,
            }
        }
    }

    impl UninstallEnvironment for FakeUninstallEnvironment {
        fn record_stage(&mut self, stage: SelfUninstallStage) {
            self.stages.push(stage);
        }

        fn capture_device_identity(&mut self) -> Option<DeviceIdentity> {
            self.identity.clone()
        }

        fn report_hub_removal(&mut self, identity: &DeviceIdentity) -> HubRemovalReport {
            self.report_targets.push(identity.device_ura.clone());
            self.report_result.clone()
        }

        fn stop_runtime_without_revoke(&mut self) -> Result<(), String> {
            self.stop_calls += 1;
            self.stop_result.clone()
        }

        fn binary_paths(&self) -> Vec<PathBuf> {
            self.binary_paths.clone()
        }

        fn remove_binary_if_exists(&mut self, path: &Path) -> bool {
            self.removed_binaries.push(path.to_path_buf());
            true
        }

        fn home_dir(&self) -> Option<PathBuf> {
            self.home.clone()
        }

        fn remove_data_dir_if_exists(&mut self, path: &Path) -> bool {
            self.removed_data_dirs.push(path.to_path_buf());
            true
        }

        fn clean_shell_profile(&mut self, home: &Path) {
            self.cleaned_profiles.push(home.to_path_buf());
        }
    }

    #[test]
    fn self_uninstall_reports_hub_removal_before_stopping_runtime() {
        let mut env = FakeUninstallEnvironment::with_identity();

        SelfUninstallPlan.execute(&mut env).expect("plan succeeds");

        assert_eq!(
            env.stages,
            vec![
                SelfUninstallStage::CaptureIdentity,
                SelfUninstallStage::ReportHubRemoval,
                SelfUninstallStage::StopRuntime,
                SelfUninstallStage::RemoveDesktopCompanions,
                SelfUninstallStage::RemoveBinaries,
                SelfUninstallStage::RemoveData,
                SelfUninstallStage::CleanShellProfile,
                SelfUninstallStage::Complete,
            ]
        );
        assert_eq!(
            env.report_targets,
            vec!["easynet:///r/acme/device/01DEV".to_string()]
        );
        assert_eq!(env.stop_calls, 1);
        assert_eq!(
            env.removed_data_dirs,
            vec![PathBuf::from("/home/alice/.easynet")]
        );
        assert_eq!(env.cleaned_profiles, vec![PathBuf::from("/home/alice")]);
    }

    #[test]
    fn self_uninstall_continues_when_hub_removal_report_fails() {
        let mut env = FakeUninstallEnvironment::with_identity();
        env.report_result = HubRemovalReport::Failed("daemon unavailable".to_string());

        SelfUninstallPlan
            .execute(&mut env)
            .expect("plan remains best-effort");

        assert_eq!(env.report_targets.len(), 1);
        assert_eq!(env.stop_calls, 1);
        assert_eq!(env.removed_binaries.len(), 2);
        assert_eq!(
            env.removed_data_dirs,
            vec![PathBuf::from("/home/alice/.easynet")]
        );
    }

    #[test]
    fn self_uninstall_skips_hub_report_without_credentials() {
        let mut env = FakeUninstallEnvironment::with_identity();
        env.identity = None;

        SelfUninstallPlan
            .execute(&mut env)
            .expect("missing credentials is non-fatal");

        assert!(env.report_targets.is_empty());
        assert_eq!(env.stop_calls, 1);
        assert_eq!(
            &env.stages[..3],
            &[
                SelfUninstallStage::CaptureIdentity,
                SelfUninstallStage::ReportHubRemoval,
                SelfUninstallStage::StopRuntime,
            ]
        );
    }

    #[test]
    fn self_uninstall_continues_when_runtime_stop_fails() {
        let mut env = FakeUninstallEnvironment::with_identity();
        env.stop_result = Err("pid did not exit".to_string());

        SelfUninstallPlan
            .execute(&mut env)
            .expect("stop failure is non-fatal");

        assert_eq!(env.stop_calls, 1);
        assert_eq!(env.removed_binaries.len(), 2);
        assert_eq!(
            env.removed_data_dirs,
            vec![PathBuf::from("/home/alice/.easynet")]
        );
    }
}
