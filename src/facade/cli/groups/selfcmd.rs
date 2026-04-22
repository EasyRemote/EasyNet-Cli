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

use anyhow::Context;
use clap::{Args, Subcommand};

use crate::support::output;

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

    // Stop runtime if running.
    output::step("Stopping runtime...");
    let _ = std::process::Command::new("easynet")
        .args(["runtime", "stop"])
        .status();

    // Remove binaries.
    for bin in ["easynet", "axon-runtime"] {
        for dir in ["/usr/local/bin", "/usr/bin"] {
            let path = format!("{dir}/{bin}");
            if std::path::Path::new(&path).exists() {
                output::step(&format!("Removing {path}"));
                let _ = std::fs::remove_file(&path).or_else(|_| {
                    std::process::Command::new("sudo")
                        .args(["rm", "-f", &path])
                        .status()
                        .map(|_| ())
                });
            }
        }
    }

    // Remove ~/.easynet directory.
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default();
    let easynet_home = format!("{home}/.easynet");
    if std::path::Path::new(&easynet_home).exists() {
        output::step(&format!("Removing {easynet_home}"));
        let _ = std::fs::remove_dir_all(&easynet_home);
    }

    // Clean env var from shell profile.
    clean_shell_profile(&home);

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

fn clean_shell_profile(home: &str) {
    let profiles = [
        format!("{home}/.zshrc"),
        format!("{home}/.bashrc"),
        format!("{home}/.bash_profile"),
        format!("{home}/.profile"),
    ];

    for profile_path in &profiles {
        let path = std::path::Path::new(profile_path);
        if !path.exists() {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
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

        if std::fs::write(path, &out).is_ok() {
            output::step(&format!(
                "Cleaned EASYNET_DENDRITE_BRIDGE_LIB from {profile_path}"
            ));
        }
    }
}
