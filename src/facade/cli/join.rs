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
use serde::{Deserialize, Serialize};

use crate::persistence::config;
use crate::support::{output, sysinfo};

#[derive(Debug, Deserialize)]
struct PairingPreflight {
    // URA v4.1.4 backend renamed the wire field `tenant_id` → `realm`
    // (PairingPreflightResp in backend/internal/types/types.go:314).
    // We deserialize from `realm`, fall back to the legacy
    // `tenant_id` for compat with pre-v4.1.4 hubs, and expose the
    // value through the existing `tenant_id` accessor so the rest
    // of join.rs (assertions, validate-pairing payload) keeps the
    // same shape — the v1 alias is the carrier on disk in
    // `credentials.json::tenant_id` until that schema is also
    // promoted (RFC follow-up).
    #[serde(rename = "realm", alias = "tenant_id")]
    tenant_id: String,
    node_id: String,
    /// Realm hub's Ed25519 pubkey (base64). The cold-start
    /// cross-machine fix: backend surfaces this here so the
    /// device can write the hub's `(uri, pubkey, role=hub)` row
    /// into its local `realm-trust.toml` during join, without
    /// needing on-host access to `~/.easynet-hub/<realm>/
    /// identity.json`. Empty on pre-v4.1.4 hubs (legacy fallback
    /// path reads the on-disk identity file when same-host).
    #[serde(default)]
    hub_public_key_b64: String,
    #[serde(default)]
    _hub_agent_uri: String,
}

#[derive(Debug, Serialize)]
struct ValidatePairingPayload {
    #[serde(flatten)]
    info: sysinfo::DeviceInfo,
    node_id: String,
    device_public_key: String,
}

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
    output::info("Preparing pairing...");
    let preflight = preflight_pairing_token(&token, &validate_base)?;
    output::info("Validating pairing token...");
    let mut creds = validate_pairing_token(&token, &validate_base, &preflight)?;
    backfill_credentials_username_from_auth_session(&mut creds);
    creds.hub_api_base = hub_api_override;
    config::save_credentials(&creds)?;

    // Ensure a minimal daemon-config.toml exists. Without it the
    // daemon's axon_serve sidecar refuses to bind the gRPC UDS
    // (no daemon-config = silent skip), so backend's
    // `daemon_grpc.Client` never finds the socket and
    // `axon: disconnected` pins forever — every `/api/v1/devices`
    // call returns the device as REMOVED no matter how alive
    // the device's `<self>.session` is on the hub.
    //
    // The minimal `device`-mode block is enough: realm + hub_endpoint
    // both come from credentials.json; uds_path defaults under
    // HOME via the daemon's own resolver. Idempotent — when a
    // daemon-config.toml already exists (operator wrote one or a
    // prior auto-wire ran) we leave it untouched.
    if let Err(e) = crate::persistence::daemon_config::ensure_minimal_device_config(&creds) {
        output::warn(&format!(
            "[easynet join] could not write default daemon-config.toml: {e}. \
             Backend will report this device as REMOVED until you write one by hand."
        ));
    }

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

    // URA v4.1.5 Phase 3C — push a fresh device keypair into the
    // local easynet-keyring vault. The vault is the load-bearing
    // signing surface for v4.1.5 production: backend (HubURI) and
    // daemon (DeviceURI) on this host both sign through the same
    // entry via role-overlay lookup. When the keyring daemon is
    // offline we fall back to v4.1.4's deterministic
    // derive_subject_keypair path (boot.rs:695) so the join
    // itself never fails on keyring availability — the warning
    // tells the operator the production posture has degraded.
    if let Err(e) = put_device_keypair_to_keyring(&creds) {
        output::warn(&format!(
            "[easynet join] keyring daemon offline ({e}); falling back to deterministic key derivation. Start `easynet-keyring` for production-grade secret isolation."
        ));
    }

    // LB-52 Gap 3 — mirror the device's own `(uri, pubkey,
    // role=Device)` self-entry into the local realm-trust.toml so
    // a co-located hub-mode daemon admits this device on
    // `<self>.session` without a separate
    // `<self>.register_device_pubkey` round-trip. Single-machine
    // demo / answer-sheet topologies that mock or skip the
    // backend hit this path; production deploys with a real
    // backend invoke `<self>.register_device_pubkey` and either
    // overwrite this entry or no-op against the matching pubkey
    // (idempotent). Failures log and keep going — same discipline
    // as the federated_peers auto-wire above.
    let _ = super::federation_wire::auto_wire_self_realm_trust_from_credentials(&creds);

    refresh_running_runtime_after_join(&creds);

    output::success("Paired successfully");
    output::detail("node_id", &creds.node_id);
    output::detail("hub_endpoint", &creds.hub_endpoint);
    output::detail("tenant_id", &creds.tenant_id);
    eprintln!();
    output::info("Run `easynet connect` to start the device agent.");
    Ok(())
}

/// Push a fresh device keypair into the keyring under the
/// canonical self URI + hub-role overlay. Phase 3C bridge: when
/// the keyring is reachable, this is the production secret
/// When the operator paired AFTER starting the local runtime, the
/// initial boot missed the joined credentials and therefore never ran
/// the bootstrap/advertise/register sequence that requires realm +
/// node identity. Refresh that running runtime in place instead of
/// forcing a restart.
///
/// Best-effort by contract:
/// - no runtime metadata on disk => nothing is running, silently skip
/// - stale runtime metadata / failed bridge connect => warn, keep join success
/// - successful connect => reuse the exact same republish helper
///   `easynet runtime start` already uses so the bootstrap semantics
///   stay single-sourced
fn refresh_running_runtime_after_join(creds: &config::Credentials) {
    let state = match config::load() {
        Ok(state) => state,
        Err(_) => return,
    };
    if matches!(
        state.runtime_kind,
        crate::persistence::config::RuntimeKind::DaemonOnly
    ) {
        output::warn(
            "paired successfully, but a local easynet-daemon is already running. \
             Restart it with `easynet runtime stop && easynet runtime start` so it picks up the new credentials.",
        );
        return;
    }
    match state.connect_bridge() {
        Ok(bridge) => {
            output::detail(
                "runtime",
                "running runtime detected; refreshing identity + federation advertisement",
            );
            super::start::republish_via_federation_best_effort(&bridge, creds);
        }
        Err(e) => output::warn(&format!(
            "paired successfully, but could not refresh the running runtime at {}: {e}. \
             Restart it with `easynet runtime start` if cross-hub lookups keep failing.",
            state.endpoint
        )),
    }
}

/// surface; when offline, the caller logs + continues, and the
/// daemon falls back to deterministic key derivation per
/// `boot.rs::load_daemon_identity`.
///
/// Returns `Ok(())` when the put landed (or when the entry
/// already existed — pairing the same node twice is a noop, the
/// pre-existing entry stays). Errors only on transport faults
/// the operator should see.
fn put_device_keypair_to_keyring(creds: &config::Credentials) -> anyhow::Result<()> {
    use crate::services::self_identity::{
        canonical_self_uris, fresh_seed_hex, KeyringClient, SelfIdentityError,
    };

    let realm = creds.tenant_id.trim();
    let node_id = creds.node_id.trim();
    if realm.is_empty() || node_id.is_empty() {
        anyhow::bail!("credentials missing realm or node_id");
    }
    let (primary_self, role_overlays) = canonical_self_uris(realm, node_id);

    let client = KeyringClient::default_path();
    // Probe reachability with a lightweight `list` first so the
    // operator-facing error is "keyring daemon offline" not
    // "keyring rejected put". Avoids confusing log lines when the
    // daemon is just not running.
    client
        .list()
        .map_err(|e| anyhow::anyhow!("keyring daemon ping: {e}"))?;

    match client.put(&primary_self, role_overlays, fresh_seed_hex()) {
        Ok(()) => Ok(()),
        // already_exists is benign — re-pairing the same device
        // keeps the existing keypair. Any other error is real.
        Err(SelfIdentityError::Rejected { kind, .. }) if kind == "already_exists" => Ok(()),
        Err(e) => Err(anyhow::anyhow!("keyring put: {e}")),
    }
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

/// Bridge the migration window where the backend may not yet return
/// `username` from validate-pairing but the operator already holds a
/// logged-in auth session that does know it. This keeps
/// `credentials.json` rich enough for hosted-agent bootstrap on the
/// first post-join runtime boot, instead of persisting `<unjoined>`
/// placeholder URIs until a later manual repair.
fn backfill_credentials_username_from_auth_session(creds: &mut config::Credentials) {
    if creds
        .username
        .as_deref()
        .is_some_and(|v| !v.trim().is_empty())
    {
        return;
    }
    let Ok(Some(session)) = crate::facade::cli::auth::load_session() else {
        return;
    };
    let Some(username) = session.username else {
        return;
    };
    let username = username.trim();
    if username.is_empty() {
        return;
    }
    creds.username = Some(username.to_string());
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

fn preflight_pairing_token(token: &str, hub_base: &str) -> anyhow::Result<PairingPreflight> {
    let base = hub_base.trim_end_matches('/');
    let url = format!("{base}/api/v1/devices/pairing/{token}/preflight");

    let resp = match ureq::get(&url)
        .timeout(std::time::Duration::from_secs(30))
        .call()
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

    let preflight: PairingPreflight = resp.into_json().map_err(|e| {
        anyhow::Error::from(e).context(
            "Hub returned an unreadable pairing preflight response — the Hub is likely on an \
             incompatible version, or a proxy rewrote the response. Verify the Hub URL and \
             that CLI + Hub versions match; re-run with a fresh pairing token if so.",
        )
    })?;
    if preflight.tenant_id.is_empty() {
        anyhow::bail!("pairing preflight response missing tenant_id");
    }
    if preflight.node_id.is_empty() {
        anyhow::bail!("pairing preflight response missing node_id");
    }
    Ok(preflight)
}

fn validate_pairing_token(
    token: &str,
    hub_base: &str,
    preflight: &PairingPreflight,
) -> anyhow::Result<config::Credentials> {
    let payload = build_validate_pairing_payload(preflight)?;
    let base = hub_base.trim_end_matches('/');
    let url = format!("{base}/api/v1/devices/pairing/{token}/validate");

    let resp = match ureq::post(&url)
        .timeout(std::time::Duration::from_secs(30))
        .send_json(&payload)
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

    let creds = validate_pairing_response(creds)?;
    if creds.node_id != preflight.node_id {
        anyhow::bail!(
            "Hub returned node_id {} but pairing preflight reserved {}; aborting to avoid \
             booting with mismatched identity",
            creds.node_id,
            preflight.node_id
        );
    }
    // URA v4.1.4: realm_str() picks `realm` first, falls back to
    // legacy `tenant_id`. Both v4.1.4 and pre-v4.1.4 hubs round-trip.
    let creds_realm = creds.realm_str();
    if creds_realm != preflight.tenant_id {
        anyhow::bail!(
            "Hub returned realm {} but pairing preflight reserved {}; aborting to avoid \
             deriving credentials under the wrong realm",
            creds_realm,
            preflight.tenant_id
        );
    }
    // Cross-machine cold-start fix: stash the hub pubkey from
    // preflight onto the in-memory + on-disk credentials so that
    // `auto_wire_self_realm_trust_from_credentials` (called by the
    // join entry point right after this function returns) can
    // populate the device's `realm-trust.toml` without needing
    // on-host access to the hub's identity.json file. Empty when
    // paired against a pre-v4.1.4 hub — the legacy file lookup
    // path stays as the same-host fallback.
    let mut creds = creds;
    if !preflight.hub_public_key_b64.trim().is_empty() {
        creds.hub_pubkey_b64 = Some(preflight.hub_public_key_b64.trim().to_string());
    }
    Ok(creds)
}

fn build_validate_pairing_payload(
    preflight: &PairingPreflight,
) -> anyhow::Result<ValidatePairingPayload> {
    Ok(ValidatePairingPayload {
        info: sysinfo::collect_system_info(),
        node_id: preflight.node_id.clone(),
        device_public_key: derive_device_public_key_hex(&preflight.tenant_id, &preflight.node_id)?,
    })
}

fn derive_device_public_key_hex(tenant_id: &str, node_id: &str) -> anyhow::Result<String> {
    use anyhow::Context as _;
    use base64::Engine as _;

    let subject_id = format!("easynet:prv:reg:agent.{node_id}");
    let (_seed, public_key_b64) =
        crate::runtime::publish::derive_subject_keypair(tenant_id, &subject_id);
    let public_key = base64::engine::general_purpose::STANDARD
        .decode(public_key_b64.as_bytes())
        .context("decode derived device public key")?;
    Ok(hex::encode(public_key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
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
            username: None,
            hub_pubkey_b64: None,
        };
        let err = validate_pairing_response(creds).expect_err("missing node_id must fail");
        assert!(err.to_string().contains("missing node_id"));
    }

    #[test]
    fn backfill_credentials_username_uses_auth_session_when_pairing_response_omits_it() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let state_dir = config::state_dir();
        std::fs::create_dir_all(&state_dir).expect("create state dir");
        let session = crate::facade::cli::auth::AuthSession {
            token: "token".into(),
            hub_url: "http://127.0.0.1:8080".into(),
            email: "alice@example.com".into(),
            user_id: Some("user-uuid".into()),
            nickname: Some("Alice".into()),
            username: Some("alice".into()),
        };
        std::fs::write(
            state_dir.join("auth.json"),
            serde_json::to_vec(&session).expect("serialize session"),
        )
        .expect("write auth.json");

        let mut creds = config::Credentials {
            node_id: "node".into(),
            credential_token: "cred".into(),
            hub_endpoint: "axon://hub.example:50051".into(),
            tenant_id: "tenant".into(),
            deploy_signature: "sig".into(),
            hub_api_base: None,
            username: None,
            hub_pubkey_b64: None,
        };
        backfill_credentials_username_from_auth_session(&mut creds);
        assert_eq!(creds.username.as_deref(), Some("alice"));
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
    fn derive_device_public_key_hex_matches_runtime_derivation() {
        let tenant_id = "tenant-a";
        let node_id = "en-test-node";
        let got = derive_device_public_key_hex(tenant_id, node_id).expect("derive hex");
        let want_b64 = crate::runtime::publish::derive_owner_public_key_b64(tenant_id, node_id);
        let want = hex::encode(
            base64::engine::general_purpose::STANDARD
                .decode(want_b64.as_bytes())
                .expect("decode owner b64"),
        );
        assert_eq!(got, want);
    }

    #[test]
    fn build_validate_pairing_payload_carries_reserved_identity() {
        let preflight = PairingPreflight {
            tenant_id: "tenant-a".into(),
            node_id: "en-test-node".into(),
            hub_public_key_b64: String::new(),
            _hub_agent_uri: String::new(),
        };
        let payload = build_validate_pairing_payload(&preflight).expect("build payload");
        assert_eq!(payload.node_id, "en-test-node");
        assert_eq!(payload.device_public_key.len(), 64);
    }

    #[test]
    fn preflight_pairing_token_surfaces_transport_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let addr = listener.local_addr().expect("local_addr");
        drop(listener);
        let base = format!("http://{}", addr);
        let err = preflight_pairing_token("token_1234", &base)
            .expect_err("transport failure should error");
        assert!(err.to_string().contains("cannot reach Hub"));
    }

    #[test]
    fn validate_pairing_token_surfaces_transport_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind probe");
        let addr = listener.local_addr().expect("local_addr");
        drop(listener);
        let base = format!("http://{}", addr);
        let preflight = PairingPreflight {
            tenant_id: "tenant-a".into(),
            node_id: "en-test-node".into(),
            hub_public_key_b64: String::new(),
            _hub_agent_uri: String::new(),
        };
        let err = validate_pairing_token("token_1234", &base, &preflight)
            .expect_err("transport failure should error");
        assert!(err.to_string().contains("cannot reach Hub"));
    }
}
