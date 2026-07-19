// EasyNet CLI — Hub diagnostics
// =============================
//
// File: src/cli/commands/hub.rs
// Description: Read-only Hub endpoint inspection for operator diagnostics.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>

use anyhow::bail;
use clap::{Args, Subcommand};

use crate::support::platform::output;

#[derive(Debug, Args)]
pub struct HubArgs {
    #[command(subcommand)]
    pub action: HubAction,
}

#[derive(Debug, Subcommand)]
pub enum HubAction {
    /// Inspect an explicit Hub/Auth endpoint without changing local trust.
    Inspect(HubInspectArgs),
}

#[derive(Debug, Args)]
pub struct HubInspectArgs {
    /// Hub/Auth endpoint URL.
    pub endpoint: String,
}

pub fn run(args: HubArgs) -> anyhow::Result<()> {
    match args.action {
        HubAction::Inspect(args) => run_inspect(args),
    }
}

fn run_inspect(args: HubInspectArgs) -> anyhow::Result<()> {
    let endpoint = args.endpoint.trim();
    if !(endpoint.starts_with("https://") || endpoint.starts_with("http://")) {
        bail!("Hub endpoint must start with http:// or https://");
    }
    let without_scheme = endpoint
        .strip_prefix("https://")
        .or_else(|| endpoint.strip_prefix("http://"))
        .unwrap_or(endpoint);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    let tls = if endpoint.starts_with("https://") {
        "enabled"
    } else {
        "disabled"
    };
    output::kv_section_stdout(&[
        ("endpoint", endpoint),
        ("authority", authority),
        ("tls", tls),
        ("trust_write", "no"),
    ]);
    Ok(())
}
