// EasyNet CLI
// ===========
//
// File: src/cli/deploy.rs
// Description: `easynet deploy <path> --to <node>` — three-phase ability deployment pipeline.
//
// Protocol Responsibility:
// - Implements a forward-recovery saga: Publish → Install → Activate.
//   Phase 1 (Publish): Registers package metadata + bytes in Hub registry.
//   Phase 2 (Install): Materializes the ability on target node, returns install_id.
//   Phase 3 (Activate): Enables invocation — ability appears in `easynet abilities`.
// - Each phase is idempotent-safe; partial failures leave the system in a recoverable state.
//
// Implementation Approach:
// - Reads ability.json for metadata: name, version, tool_name, description, command.
// - Packages ability.json as base64 payload with SHA-256 digest for integrity.
// - Uses ephemeral signature (__AXON_EPHEMERAL_DO_NOT_USE_IN_PROD__) for development;
//   requires AXON_ALLOW_PLACEHOLDER_DEPLOY_SIGNATURE=1 env var.
//
// Usage Contract:
// - The target node (--to) must be online and registered in the federation.
// - ability.json must contain at minimum: "name" and "command" fields.
// - After deployment, the ability is callable via `easynet invoke` or MCP tool calls.
//
// Architectural Position:
// - Write path of the ability lifecycle. Read path is abilities.rs.
// - Mirrors the MCP handler deploy_ability in mcp/handlers.rs (same three-phase flow).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use base64::Engine;
use clap::Args;
use console::style;
use sha2::Digest;

use crate::shared::{self, config, deploy, output};

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
    let br = shared::connect_bridge_to(&state.endpoint)?;
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

    // Package the ability.json as base64 for publishing.
    let raw = std::fs::read(&desc_path)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
    let digest = format!("sha256:{:x}", sha2::Sha256::digest(&raw));

    // Prefer real deploy signature from credentials; fall back to ephemeral for dev.
    let signature = config::load_credentials()
        .ok()
        .map(|c| c.deploy_signature)
        .filter(|s| !s.is_empty());

    if signature.is_none() {
        output::info("warning: no deploy signature found — using ephemeral placeholder (dev only)");
    }

    eprint!("  deploying {}@{} to {} ... ", style(name).cyan(), version, style(&args.to).cyan());
    let result = deploy::run_pipeline(&br, &deploy::DeployParams {
        tenant,
        node_id: &args.to,
        tool_name,
        ability_name: name,
        version,
        description,
        command,
        signature: signature.as_deref(),
        digest: &digest,
        payload_bytes: Some(raw.len()),
        payload_b64: Some(&b64),
    })?;
    eprintln!("{}", style("✓").green());

    output::step(&format!("install_id: {}", result.install_id));
    output::success(&format!("activated — {tool_name} is live"));
    Ok(())
}

