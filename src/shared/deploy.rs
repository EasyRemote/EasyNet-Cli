// EasyNet CLI — Deploy Pipeline
// Shared three-phase ability deployment: Publish -> Install -> Activate.

use anyhow::Context;
use easynet_axon::dendrite_bridge::DendriteBridge;

pub struct DeployParams<'a> {
    pub tenant: &'a str,
    pub node_id: &'a str,
    pub tool_name: &'a str,
    pub ability_name: &'a str,
    pub version: &'a str,
    pub description: &'a str,
    pub command: &'a str,
    pub signature: Option<&'a str>,
    pub digest: &'a str,
    pub payload_bytes: Option<usize>,
    pub payload_b64: Option<&'a str>,
}

pub struct DeployResult {
    pub install_id: String,
}

/// Execute the three-phase deploy pipeline: publish -> install -> activate.
/// `signature` falls back to ephemeral placeholder if None or empty.
pub fn run_pipeline(br: &DendriteBridge, p: &DeployParams) -> anyhow::Result<DeployResult> {
    let sig = p
        .signature
        .filter(|s| !s.is_empty())
        .unwrap_or("__AXON_EPHEMERAL_DO_NOT_USE_IN_PROD__");

    let metadata = serde_json::json!({
        "mcp.tool_name": p.tool_name,
        "mcp.description": p.description,
        "axon.exec.command": p.command,
    });

    // Phase 1: Publish
    br.publish_capability(
        p.tenant,
        p.tool_name,
        p.ability_name,
        p.version,
        p.digest,
        Some(sig),
        &[],
        metadata,
        None,
        p.payload_bytes.map(|n| n.try_into().unwrap_or(i64::MAX)),
        p.payload_b64,
        None,
        None,
    )
    .context("publish")?;

    // Phase 2: Install
    let install_result = br
        .install_capability(p.tenant, p.node_id, p.tool_name, p.version, "", false, "host", 30)
        .context("install")?;

    let install_id = install_result
        .get("install_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Phase 3: Activate
    br.activate_capability(p.tenant, p.node_id, &install_id)
        .context("activate")?;

    Ok(DeployResult { install_id })
}
