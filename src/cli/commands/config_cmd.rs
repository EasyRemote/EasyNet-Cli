// EasyNet CLI
// ===========
//
// File: src/cli/config_cmd.rs
// Description: `easynet config` — inspect and mutate device-local settings that control
//              runtime behavior (e.g., session_bridge exec permission).
//
// Protocol Responsibility:
// - Reads/writes ~/.easynet/device_settings.json, which is separate from credentials
//   and runtime state by design: settings are user-controlled knobs, not server-issued.
// - Changes take effect on next `easynet runtime start` (requires reconnect).
//
// Implementation Approach:
// - Subcommand dispatch: `show` (default) and `exec on|off`.
// - Named config_cmd.rs (not config.rs) to avoid collision with shared/config.rs.
//
// Usage Contract:
// - Safe to run while connected — settings are persisted but not hot-reloaded.
// - The `exec` toggle gates whether session_bridge allows one-shot command execution,
//   a security boundary: disabled by default to prevent unintended remote code execution.
//
// Architectural Position:
// - User-facing configuration surface. Decoupled from runtime lifecycle.
// - Consumed by start.rs at boot time via config::load_device_settings().
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::{Args, Subcommand};

use crate::daemon::persistence::config;
use crate::support::platform::output;

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub action: Option<ConfigAction>,
    /// Skip confirmation prompts (for non-interactive use)
    #[arg(long, short = 'y')]
    pub yes: bool,
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Show current device settings.
    Show,
    /// Enable or disable `session_bridge` exec (on|off).
    Exec {
        /// on or off
        value: String,
    },
}

pub fn run(args: ConfigArgs) -> anyhow::Result<()> {
    let auto_confirm = args.yes;
    match args.action.unwrap_or(ConfigAction::Show) {
        ConfigAction::Show => {
            let settings = config::load_device_settings();
            eprintln!(
                "session_bridge_exec_enabled: {}",
                settings.session_bridge_exec_enabled
            );
        }
        ConfigAction::Exec { value } => {
            let mut settings = config::load_device_settings();
            match value.as_str() {
                "on" | "true" | "1" | "enable" | "enabled" => {
                    if !settings.session_bridge_exec_enabled && !auto_confirm {
                        output::warn(
                            "This enables remote command execution on this device. \
                             Users in the same tenant with Hub access can run commands on it.",
                        );
                        if !output::confirm("Are you sure?")? {
                            output::info("Cancelled.");
                            return Ok(());
                        }
                    }
                    settings.session_bridge_exec_enabled = true;
                }
                "off" | "false" | "0" | "disable" | "disabled" => {
                    settings.session_bridge_exec_enabled = false;
                }
                _ => anyhow::bail!("invalid value {value:?} (expected on|off)"),
            }
            config::save_device_settings(&settings)?;
            output::success(&format!(
                "session_bridge_exec_enabled set to {}",
                settings.session_bridge_exec_enabled
            ));
            output::info("Note: reconnect required to apply. Run 'easynet runtime stop && easynet runtime start' or restart 'easynet runtime connect'.");
        }
    }
    Ok(())
}
