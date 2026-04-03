// EasyNet CLI
// ===========
//
// File: src/cli/deploy.rs
// Description: `easynet deploy <path> --to <node>` — three-phase ability deployment.
//
// Pipeline: Publish → Install → Activate (forward-recovery saga).
// - Reads ability.json from the given directory for metadata (name, version, command).
// - Publishes to the registry with ephemeral signature.
// - Installs on target node, returns install_id.
// - Activates to make the ability callable.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;
use console::style;

use crate::shared::{self, config, output};

#[derive(Debug, Args)]
pub struct DeployArgs {
    /// Path to ability/skill directory
    pub path: String,
    /// Target node ID
    #[arg(long)]
    pub to: String,
}

pub fn run(args: DeployArgs) -> anyhow::Result<()> {
    let state = config::load()?;
    let br = shared::connect_bridge()?;
    let tenant = state.tenant_or_default();

    let dir = std::path::Path::new(&args.path);
    anyhow::ensure!(dir.is_dir(), "{} is not a directory", args.path);

    let desc_path = dir.join("ability.json");
    anyhow::ensure!(desc_path.exists(), "no ability.json in {}", args.path);

    let desc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&desc_path)?)?;
    let name = desc.get("name").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("missing 'name'"))?;
    let version = desc.get("version").and_then(|v| v.as_str()).unwrap_or("1.0.0");
    let tool_name = desc.get("tool_name").and_then(|v| v.as_str()).unwrap_or(name);
    let description = desc.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let command = desc.get("command").and_then(|v| v.as_str()).ok_or_else(|| anyhow::anyhow!("missing 'command'"))?;

    // Publish
    eprint!("  publishing {}@{} ... ", style(name).cyan(), version);
    let metadata = serde_json::json!({
        "mcp.tool_name": tool_name,
        "mcp.description": description,
        "axon.exec.command": command,
    });
    br.publish_capability(
        tenant, tool_name, name, version, "",
        Some("__AXON_EPHEMERAL_DO_NOT_USE_IN_PROD__"),
        &[], metadata, None, None, None, None, None,
    ).map_err(|e| anyhow::anyhow!("publish: {e}"))?;
    eprintln!("{}", style("✓").green());

    // Install
    eprint!("  installing on {} ... ", style(&args.to).cyan());
    let install_result = br
        .install_capability(tenant, &args.to, tool_name, version, "", false, "host", 30)
        .map_err(|e| anyhow::anyhow!("install: {e}"))?;
    let install_id = install_result.get("install_id").and_then(|v| v.as_str()).unwrap_or("?");
    eprintln!("{}", style("✓").green());
    output::step(&format!("installed (install_id: {install_id})"));

    // Activate
    br.activate_capability(tenant, &args.to, install_id)
        .map_err(|e| anyhow::anyhow!("activate: {e}"))?;
    output::success(&format!("activated — {tool_name} is live"));
    Ok(())
}
