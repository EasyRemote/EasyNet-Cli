// EasyNet CLI
// ===========
//
// File: src/cli/join.rs
// Description: `easynet join <token>` — pair this device with EasyNet Hub via a one-time
//              pairing token, establishing a persistent trust relationship.
//
// Protocol Responsibility:
// - Validates a one-time pairing token (32-64 hex chars) against the Hub REST API.
// - POST /api/v1/devices/pairing/{token}/validate with device sysinfo (hostname, OS, arch).
// - Receives and persists: node_id, credential_token, hub_endpoint, tenant_id, deploy_signature.
// - This is the ONLY command that creates ~/.easynet/credentials.json; all other commands consume it.
//
// Implementation Approach:
// - Synchronous HTTP via ureq with 30s timeout. No retry — pairing tokens are one-shot.
// - Token format validation before network call to fail fast on typos.
// - Supports --hub for self-hosted Hubs (defaults to https://easynet.run).
//
// Usage Contract:
// - Run once per device. Re-running overwrites existing credentials (re-pair).
// - Requires network access to Hub REST API (not the gRPC Axon endpoint).
// - After join, run `easynet connect` to start the device agent.
//
// Architectural Position:
// - Entry point of the device lifecycle: join → start → (heartbeat loop) → stop → reset.
// - Bridges the Hub's web-based pairing flow with the CLI's local credential store.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use crate::shared::{config, output, sysinfo};

#[derive(Debug, Args)]
pub struct JoinArgs {
    /// One-time pairing token (32-64 hex characters)
    pub token: String,
    /// Hub API base URL for self-hosted Hubs (default: `https://easynet.run`)
    #[arg(long, default_value_t = format!("https://{}", config::DEFAULT_HUB_HOST))]
    pub hub: String,
    /// Override Hub REST API base URL for credential verification (e.g. `http://localhost:8080`).
    /// Only needed for local dev when the REST API is on a different host/port than the Hub.
    #[arg(long)]
    pub hub_api: Option<String>,
    /// Skip confirmation prompts (for non-interactive use)
    #[arg(long, short = 'y')]
    pub yes: bool,
}

pub fn run(args: JoinArgs) -> anyhow::Result<()> {
    // Warn if already paired — prevent accidental overwrite.
    if let Ok(existing) = config::load_credentials() {
        output::warn(&format!(
            "Already paired as {} (hub: {})",
            existing.node_id, existing.hub_endpoint
        ));
        if !args.yes {
            output::info("This will overwrite existing credentials. Run `easynet reset` first to un-pair cleanly.");
            if !output::confirm("Continue?")? {
                output::info("Cancelled.");
                return Ok(());
            }
        }
    }

    let token = args.token.trim().to_string();
    validate_token_format(&token)?;

    output::info("Validating pairing token...");
    let mut creds = validate_pairing_token(&token, &args.hub)?;
    creds.hub_api_base = args.hub_api.map(|s| s.trim_end_matches('/').to_string());
    config::save_credentials(&creds)?;

    output::success("Paired successfully");
    output::detail("node_id", &creds.node_id);
    output::detail("hub_endpoint", &creds.hub_endpoint);
    output::detail("tenant_id", &creds.tenant_id);
    eprintln!();
    output::info("Run `easynet connect` to start the device agent.");
    Ok(())
}

fn validate_token_format(token: &str) -> anyhow::Result<()> {
    if token.len() < 8 {
        anyhow::bail!("invalid pairing token: too short (minimum 8 characters, got {})", token.len());
    }
    if token.len() > 256 {
        anyhow::bail!("invalid pairing token: too long (maximum 256 characters, got {})", token.len());
    }
    // Accept hex, alphanumeric, dashes, and underscores (covers hex tokens, UUIDs, base64url).
    if !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        anyhow::bail!("invalid pairing token: must contain only alphanumeric characters, dashes, or underscores");
    }
    Ok(())
}

fn validate_pairing_token(token: &str, hub_base: &str) -> anyhow::Result<config::Credentials> {
    let info = sysinfo::collect_system_info();
    let base = hub_base.trim_end_matches('/');
    let url = format!("{base}/api/v1/devices/pairing/{token}/validate");

    let resp = match ureq::post(&url)
        .timeout(std::time::Duration::from_secs(30))
        .send_json(&info)
    {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => {
            anyhow::bail!("pairing token expired or already used — create a new token from the Hub dashboard");
        }
        Err(ureq::Error::Status(409, _)) => {
            anyhow::bail!("device already paired — run `easynet reset` first to un-pair, then retry");
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            anyhow::bail!("Hub rejected pairing (HTTP {code}): {body}");
        }
        Err(ureq::Error::Transport(e)) => {
            anyhow::bail!("cannot reach Hub at {base}: {e}\n  Check your network connection and Hub URL.");
        }
    };

    let creds: config::Credentials = resp
        .into_json()
        .map_err(|e| anyhow::anyhow!("invalid pairing response: {e}"))?;

    if creds.node_id.is_empty() {
        anyhow::bail!("pairing response missing node_id");
    }
    Ok(creds)
}
