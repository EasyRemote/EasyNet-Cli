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

use crate::persistence::config;
use crate::support::{output, sysinfo};

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
    /// Peer hub's daemon TLS listen address, used to populate the
    /// local daemon's `[daemon.federated_peers]` entry for this
    /// tenant. Form: `https://host:port` (e.g. `https://hub-b.example:50443`).
    ///
    /// Why this is operator-supplied: the Hub's pairing response
    /// carries the **backend's** Axon endpoint (the inbound-from-
    /// device gRPC port), which is NOT the peer daemon's TLS
    /// listener. In a multi-hub deployment those addresses
    /// differ. Without this flag the auto-wire either writes the
    /// backend port (wrong for cross-hub dial) or assumes the
    /// canonical 50443 (wrong if the operator picked a different
    /// port). Pass `--peer-hub` when joining a tenant whose hub
    /// you intend to route cross-hub calls to.
    #[arg(long)]
    pub peer_hub: Option<String>,
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

    let hub_api_override = args
        .hub_api
        .as_ref()
        .map(|s| s.trim_end_matches('/').to_string());
    let validate_base = pick_validate_base(&args.hub, hub_api_override.as_deref());
    output::info("Validating pairing token...");
    let mut creds = validate_pairing_token(&token, &validate_base)?;
    creds.hub_api_base = hub_api_override;
    config::save_credentials(&creds)?;

    // Best-effort: if this device is also running a hub-mode
    // daemon (i.e. `~/.easynet/daemon-config.toml` exists), seed
    // the daemon's `[daemon.federated_peers]` table with the
    // tenant→hub mapping. When `--peer-hub` is set the operator
    // tells us the peer daemon's TLS listen address explicitly;
    // when absent, the helper falls back to the canonical-port
    // guess and warns the operator. SIGHUPs the running daemon
    // so the new entry activates without a restart. Failures
    // here log and keep going — the join itself has succeeded;
    // the operator can edit daemon-config.toml by hand later if
    // the auto-wire didn't fire (no daemon running, parser
    // hiccup, etc).
    let _ = super::federation_wire::auto_wire_federated_peer_from_credentials(
        &creds,
        args.peer_hub.as_deref(),
    );

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
        anyhow::bail!(
            "invalid pairing token: too short (minimum 8 characters, got {})",
            token.len()
        );
    }
    if token.len() > 256 {
        anyhow::bail!(
            "invalid pairing token: too long (maximum 256 characters, got {})",
            token.len()
        );
    }
    // Accept hex, alphanumeric, dashes, and underscores (covers hex tokens, UUIDs, base64url).
    if !token
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!("invalid pairing token: must contain only alphanumeric characters, dashes, or underscores");
    }
    Ok(())
}

fn pairing_status_error_message(code: u16, body: &str) -> String {
    match code {
        404 => "pairing token expired or already used — create a new token from the Hub dashboard"
            .into(),
        409 => "device already paired — run `easynet reset` first to un-pair, then retry".into(),
        _ => format!("Hub rejected pairing (HTTP {code}): {body}"),
    }
}

fn validate_pairing_response(creds: config::Credentials) -> anyhow::Result<config::Credentials> {
    if creds.node_id.is_empty() {
        anyhow::bail!("pairing response missing node_id");
    }
    Ok(creds)
}

/// Pick the REST-API base URL the pairing-token validation call
/// should hit. Operators commonly run a self-hosted Hub where the
/// user-facing portal (`--hub`) and the REST API (`--hub-api`)
/// live on different hosts/ports — e.g. portal at
/// `https://easynet.run`, REST API at `http://localhost:18080`.
/// Without preferring `--hub-api` when set, the validation call
/// hits the portal URL, gets a 404, and surfaces as "pairing
/// token expired or already used" — a misleading error mode.
fn pick_validate_base(hub: &str, hub_api_override: Option<&str>) -> String {
    hub_api_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| hub.to_string())
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
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            anyhow::bail!("{}", pairing_status_error_message(code, &body));
        }
        Err(ureq::Error::Transport(e)) => {
            anyhow::bail!(
                "cannot reach Hub at {base}: {e}\n  Check your network connection and Hub URL."
            );
        }
    };

    // The Hub's pairing endpoint is a versioned REST contract (see the
    // Hub's OpenAPI spec under /api/v1/devices/pairing). If `into_json`
    // fails, the bytes we got back are either not JSON at all (a proxy
    // inserted an HTML error page, a middlebox rewrote the response) or
    // the JSON shape no longer matches `config::Credentials` (the CLI
    // and Hub are on incompatible versions). Either way, the underlying
    // serde error is noise to an operator — they need to know *what to
    // do*, not which field's tag didn't match. We keep the raw cause in
    // the error chain via `context`, so `--verbose` / log scrapers still
    // surface the full detail, while the top-line stays operator-friendly.
    let creds: config::Credentials = resp.into_json().map_err(|e| {
        anyhow::Error::from(e).context(
            "Hub returned an unreadable pairing response — the Hub is likely on an \
             incompatible version, or a proxy rewrote the response. Verify the Hub URL \
             and that CLI + Hub versions match; re-run with a fresh pairing token if so.",
        )
    })?;

    validate_pairing_response(creds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn token_format_accepts_alnum_dash_underscore() {
        for token in ["abc12345", "A_B-C_99", "token_2026_04"] {
            assert!(
                validate_token_format(token).is_ok(),
                "expected valid token: {token}"
            );
        }
    }

    #[test]
    fn token_format_rejects_short_long_and_invalid_chars() {
        assert!(validate_token_format("short").is_err());
        assert!(validate_token_format(&"a".repeat(257)).is_err());
        assert!(validate_token_format("bad token").is_err());
        assert!(validate_token_format("bad/token").is_err());
    }

    #[test]
    fn pairing_status_error_message_maps_common_cases() {
        assert!(pairing_status_error_message(404, "x").contains("expired or already used"));
        assert!(pairing_status_error_message(409, "x").contains("device already paired"));
        assert_eq!(
            pairing_status_error_message(500, "oops"),
            "Hub rejected pairing (HTTP 500): oops"
        );
    }

    #[test]
    fn validate_pairing_response_rejects_empty_node_id() {
        let creds = config::Credentials {
            node_id: String::new(),
            credential_token: "cred".into(),
            hub_endpoint: "axon://easynet.run:50051".into(),
            tenant_id: "tenant".into(),
            deploy_signature: "sig".into(),
            hub_api_base: None,
        };
        let err = validate_pairing_response(creds).expect_err("missing node_id must fail");
        assert!(err.to_string().contains("missing node_id"));
    }

    #[test]
    fn pick_validate_base_prefers_hub_api_when_set() {
        let chosen = pick_validate_base("https://easynet.run", Some("http://localhost:18080"));
        assert_eq!(chosen, "http://localhost:18080");
    }

    #[test]
    fn pick_validate_base_falls_back_to_hub_when_api_unset() {
        let chosen = pick_validate_base("https://easynet.run", None);
        assert_eq!(chosen, "https://easynet.run");
    }

    #[test]
    fn validate_pairing_token_surfaces_transport_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let addr = listener.local_addr().expect("local_addr");
        drop(listener);
        let base = format!("http://{}", addr);
        let err = validate_pairing_token("token_1234", &base)
            .expect_err("transport failure should error");
        assert!(err.to_string().contains("cannot reach Hub"));
    }
}
