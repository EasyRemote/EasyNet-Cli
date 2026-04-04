// EasyNet CLI
// ===========
//
// File: src/cli/start.rs
// Description: `easynet start` — spawns a local Axon runtime, joins Hub, registers node,
//              and maintains heartbeat. Supports both foreground and background modes.
//
// Lifecycle:
// - Ensures no runtime is already running (checks ~/.easynet/runtime.json).
// - Uses `ServerConfig` from the Axon SDK to auto-start a local runtime on a free port.
// - If credentials exist (from `easynet join`), registers the node and starts heartbeat.
// - In foreground mode: blocks on Ctrl-C, then gracefully deregisters + shuts down.
// - In background mode: forks a heartbeat daemon process, detaches the runtime.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use anyhow::Context;
use clap::Args;
use easynet_axon::server::ServerConfig;

use crate::shared::{self, config, net, output, shutdown::ShutdownSignal};

const DEFAULT_HEARTBEAT_MS: u64 = 30_000;
const MAX_HEARTBEAT_FAILURES: u32 = 5;

#[derive(Debug, Args)]
pub struct StartArgs {
    /// Hub endpoint (e.g. axon://easynet.run:50051)
    #[arg(long, default_value = config::DEFAULT_HUB)]
    pub hub: String,
    /// Tenant ID
    #[arg(long, default_value = config::DEFAULT_TENANT)]
    pub tenant: String,
    /// Human-readable label for this device
    #[arg(long)]
    pub label: Option<String>,
    /// Pre-shared join token
    #[arg(long)]
    pub token: Option<String>,
    /// Run as a Hub (accept federation joins instead of joining a Hub)
    #[arg(long)]
    pub as_hub: bool,
    /// Bind address for Hub mode (default: 0.0.0.0:50051)
    #[arg(long, default_value = config::DEFAULT_BIND)]
    pub bind: String,
    /// Run in foreground (block until Ctrl-C)
    #[arg(long)]
    pub foreground: bool,
    /// Disable Hub-level MCP server on stdio (foreground only)
    #[arg(long)]
    pub no_mcp: bool,
}

impl StartArgs {
    /// Construct args suitable for `easynet connect` (foreground, minimal flags).
    /// Note: hub/tenant are placeholders — run_device_mode() overrides them from credentials.
    pub fn for_connect(no_mcp: bool) -> Self {
        Self {
            hub: config::DEFAULT_HUB.into(),
            tenant: config::DEFAULT_TENANT.into(),
            label: None,
            token: None,
            as_hub: false,
            bind: config::DEFAULT_BIND.into(),
            foreground: true,
            no_mcp,
        }
    }
}

/// Result of verify_credential — tracks whether the Hub was reachable.
enum CredentialCheck {
    Valid,
    NetworkUnavailable,
    Revoked(String),
}

pub fn run(args: StartArgs) -> anyhow::Result<()> {
    if let Ok(state) = config::load() {
        if state.pid.is_some_and(net::is_pid_alive) {
            anyhow::bail!(
                "runtime already running (run `easynet stop` first, or remove ~/.easynet/runtime.json)"
            );
        }
        output::info("Detected stale runtime state (process not running). Cleaning up...");
        config::remove().ok();
    }

    if args.as_hub {
        return run_as_hub(&args);
    }

    run_device_mode(&args)
}

// ── Device mode ─────────────────────────────────────────────────────────────

fn run_device_mode(args: &StartArgs) -> anyhow::Result<()> {
    // Load and verify credentials (from `easynet join`).
    let (creds, credential_verified) = load_and_verify_credentials()?;
    let settings = config::load_device_settings();

    // Configure env vars consumed by the Axon SDK. Must happen before any thread spawn.
    apply_env_patch(&env_patch_for_device(&creds, &settings));

    // Credentials take precedence over CLI args for hub/tenant.
    let hub = creds.hub_endpoint.clone();
    let tenant = creds.tenant_id.clone();
    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    let label = args.label.clone().unwrap_or_else(|| creds.node_id.clone());

    let srv = start_runtime_for_device(&hub, &tenant, &label, &creds.node_id, args.token.as_deref())?;
    let endpoint = srv.url().to_string();
    let pid = net::discover_pid_from_endpoint(&endpoint);

    let state = config::RuntimeState {
        endpoint: endpoint.clone(),
        pid,
        hub: Some(hub.clone()),
        tenant: Some(tenant.clone()),
        label: Some(label.clone()),
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        credential_verified: Some(credential_verified),
    };
    config::save(&state)?;

    output::success(&format!("Axon runtime started on {endpoint}"));
    output::success(&format!("Joined {hub} as {label}"));
    output::detail("tenant", &tenant);
    if let Some(pid) = pid {
        output::detail("pid", &pid.to_string());
    }
    if !credential_verified {
        output::info("  warning: credential not verified (Hub unreachable during startup)");
    }

    // Try connecting the bridge for node registration.
    let bridge = match shared::connect_bridge_to(&endpoint) {
        Ok(b) => b,
        Err(e) => {
            output::info(&format!("warning: bridge connect failed: {e}"));
            output::info("Node registration skipped — runtime is still running.");
            output::info("Hint: set EASYNET_DENDRITE_BRIDGE_LIB to the dendrite bridge library path.");
            return run_foreground_or_detach(srv, args.foreground);
        }
    };

    let reg_resp = bridge
        .register_node(&creds.tenant_id, &creds.node_id, &hostname)
        .context("register node")?;

    let heartbeat_ms = reg_resp
        .get("heartbeat_interval_ms")
        .and_then(serde_json::Value::as_u64)
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_HEARTBEAT_MS);

    output::success(&format!(
        "Node registered: {} (heartbeat every {}ms)",
        creds.node_id, heartbeat_ms
    ));

    if args.foreground {
        run_foreground_with_heartbeat(srv, &bridge, &creds, &endpoint, heartbeat_ms, args.no_mcp)
    } else {
        run_background_with_heartbeat(srv, &creds, &endpoint, heartbeat_ms)
    }
}

/// Load credentials and verify against Hub. Returns error on revoked/missing credentials.
fn load_and_verify_credentials() -> anyhow::Result<(config::Credentials, bool)> {
    let Ok(creds) = config::load_credentials() else {
        output::info("No credentials found.");
        output::info("Visit https://easynet.run or your Hub to create a pairing token,");
        output::info("then run `easynet join <token>` to pair this device.");
        anyhow::bail!("no credentials — cannot start device agent");
    };

    match verify_credential(&creds) {
        CredentialCheck::Valid => Ok((creds, true)),
        CredentialCheck::NetworkUnavailable => Ok((creds, false)),
        CredentialCheck::Revoked(msg) => {
            eprintln!("{} {msg}", console::style("✗").red().bold());
            eprintln!("  node_id: {}", creds.node_id);
            eprintln!("  hub:     {}", creds.hub_endpoint);
            eprintln!("  Credential revoked or device removed from account.");
            eprintln!();
            config::delete_credentials().ok();
            eprintln!("  Stale credentials cleaned up.");
            eprintln!("  Visit https://easynet.run or your Hub to create a new pairing token,");
            eprintln!("  then run `easynet join <token>`.");
            anyhow::bail!("credential revoked");
        }
    }
}

/// Build and start the Axon runtime for device mode.
fn start_runtime_for_device(
    hub: &str,
    tenant: &str,
    label: &str,
    runtime_id: &str,
    join_token: Option<&str>,
) -> anyhow::Result<easynet_axon::server::ServerHandle> {
    let mut cfg = ServerConfig::default()
        .hub(hub)
        .hub_tenant(tenant)
        .hub_label(label)
        .hub_runtime_id(runtime_id)
        .insecure(true);
    if let Some(t) = join_token {
        cfg = cfg.hub_join_token(t);
    }
    cfg.start().context("start runtime")
}

/// Run in foreground with heartbeat + optional MCP server.
fn run_foreground_with_heartbeat(
    srv: easynet_axon::server::ServerHandle,
    bridge: &easynet_axon::dendrite_bridge::DendriteBridge,
    creds: &config::Credentials,
    endpoint: &str,
    heartbeat_ms: u64,
    no_mcp: bool,
) -> anyhow::Result<()> {
    if !no_mcp {
        let ep = endpoint.to_string();
        let t = creds.tenant_id.clone();
        std::thread::spawn(move || {
            let kit = crate::mcp::hub_kit::HubCaseKit::new(ep, t);
            let server = easynet_axon::mcp::StdioMcpServer::new(kit)
                .with_server_name("easynet-device")
                .with_server_version(env!("CARGO_PKG_VERSION"));
            if let Err(e) = server.run(std::io::stdin().lock(), &mut std::io::stdout()) {
                eprintln!("mcp server exited: {e}");
            }
        });
        output::success("MCP server started on stdio");
    }

    let shutdown = ShutdownSignal::new();
    let s = shutdown.clone();
    ctrlc::set_handler(move || {
        eprintln!("\nShutting down...");
        s.trigger();
    })?;

    let outcome =
        heartbeat_loop(bridge, &creds.tenant_id, &creds.node_id, heartbeat_ms, &shutdown);

    let reason = match outcome {
        HeartbeatOutcome::FailuresExhausted => "heartbeat lost",
        HeartbeatOutcome::HubRejected => "hub rejected",
        HeartbeatOutcome::NodeRejected => "node rejected",
        HeartbeatOutcome::Shutdown => "device shutdown",
    };
    let _ = bridge.deregister_node(&creds.tenant_id, &creds.node_id, reason);
    drop(srv);
    config::remove()?;
    match outcome {
        HeartbeatOutcome::FailuresExhausted => {
            output::success("Axon runtime stopped (heartbeat lost after consecutive failures)");
        }
        HeartbeatOutcome::HubRejected => {
            output::success("Axon runtime stopped (Hub rejected this member)");
        }
        HeartbeatOutcome::NodeRejected => {
            // Device was administratively removed — clean up local credentials
            // so the user cannot reconnect with a revoked identity.
            config::delete_credentials().ok();
            output::success("Axon runtime stopped (device removed by admin)");
            output::info("  Local credentials have been removed.");
            output::info("  To reconnect, create a new pairing token and run `easynet join <token>`.");
        }
        HeartbeatOutcome::Shutdown => {
            output::success("Axon runtime stopped");
        }
    }
    Ok(())
}

/// Detach runtime to background and spawn heartbeat daemon.
fn run_background_with_heartbeat(
    srv: easynet_axon::server::ServerHandle,
    creds: &config::Credentials,
    endpoint: &str,
    heartbeat_ms: u64,
) -> anyhow::Result<()> {
    spawn_heartbeat_daemon(endpoint, &creds.tenant_id, &creds.node_id, heartbeat_ms)?;
    // Intentionally leak the handle so the runtime keeps running after this process exits.
    std::mem::forget(srv);
    output::info("Runtime running in background. Use `easynet stop` to stop.");
    Ok(())
}

/// Either block in foreground until Ctrl-C, or detach to background.
/// Used when bridge connect fails but runtime is still running.
fn run_foreground_or_detach(
    srv: easynet_axon::server::ServerHandle,
    foreground: bool,
) -> anyhow::Result<()> {
    if foreground {
        let shutdown = ShutdownSignal::new();
        let s = shutdown.clone();
        ctrlc::set_handler(move || s.trigger())?;
        output::info("Running in foreground (Ctrl-C to stop)...");
        shutdown.wait();
        drop(srv);
        config::remove()?;
        output::success("Axon runtime stopped");
    } else {
        std::mem::forget(srv);
        output::info("Runtime running in background. Use `easynet stop` to stop.");
    }
    Ok(())
}

// ── Hub mode ────────────────────────────────────────────────────────────────

fn run_as_hub(args: &StartArgs) -> anyhow::Result<()> {
    // Clear env vars to prevent SDK from connecting to an existing endpoint or hub.
    apply_env_patch(&env_patch_for_hub());

    let tenant = args.tenant.clone();

    let mut cfg = ServerConfig::default()
        .endpoint(&args.bind)
        .insecure(true);
    if let Some(ref t) = args.token {
        cfg = cfg.hub_join_token(t);
    }

    let srv = cfg
        .start()
        .context("start hub")?;

    let endpoint = srv.url().to_string();
    let pid = net::discover_pid_from_endpoint(&endpoint);

    let state = config::RuntimeState {
        endpoint: endpoint.clone(),
        pid,
        hub: None,
        tenant: Some(tenant),
        label: Some("hub".to_string()),
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        credential_verified: None, // Not applicable in hub mode.
    };
    config::save(&state)?;

    output::success(&format!("Hub started on {endpoint}"));
    if let Some(pid) = pid {
        output::detail("pid", &pid.to_string());
    }
    output::step("Devices can join with: easynet start --hub axon://<this-ip>:50051");

    if args.foreground {
        let shutdown = ShutdownSignal::new();
        let s = shutdown.clone();
        ctrlc::set_handler(move || {
            eprintln!("\nShutting down...");
            s.trigger();
        })?;
        output::info("Running in foreground (Ctrl-C to stop)...");
        shutdown.wait();
        drop(srv);
        config::remove()?;
        output::success("Hub stopped");
    } else {
        std::mem::forget(srv);
        output::info("Hub running in background. Use `easynet stop` to stop.");
    }
    Ok(())
}

// ── Environment setup (pre-thread) ─────────────────────────────────────────

/// Environment variables the Axon SDK needs, grouped for `init_env_vars()`.
struct EnvPatch {
    sets: Vec<(&'static str, String)>,
    removes: Vec<&'static str>,
}

/// Apply environment variable patch.
///
/// # Safety
/// Must be called on the main thread before any `std::thread::spawn` or `ServerConfig::start`.
/// Caller is responsible for ensuring single-threaded context.
fn apply_env_patch(patch: &EnvPatch) {
    // SAFETY: guaranteed single-threaded by caller (main thread, before runtime start).
    unsafe {
        for (k, v) in &patch.sets {
            std::env::set_var(k, v);
        }
        for k in &patch.removes {
            std::env::remove_var(k);
        }
    }
}

fn env_patch_for_device(creds: &config::Credentials, settings: &config::DeviceSettings) -> EnvPatch {
    let mut sets = Vec::new();
    if !creds.deploy_signature.is_empty() {
        sets.push(("AXON_DEPLOY_SIGNATURE_BASE64", creds.deploy_signature.clone()));
    }
    let exec_enabled = std::env::var("EASYNET_SESSION_BRIDGE_EXEC_ENABLED")
        .map(|v| v == "1")
        .unwrap_or(settings.session_bridge_exec_enabled);
    if exec_enabled {
        sets.push(("EASYNET_SESSION_BRIDGE_EXEC_ENABLED", "1".into()));
    }
    EnvPatch {
        sets,
        removes: vec!["EASYNET_AXON_ENDPOINT"],
    }
}

fn env_patch_for_hub() -> EnvPatch {
    EnvPatch {
        sets: Vec::new(),
        removes: vec!["EASYNET_AXON_ENDPOINT", "AXON_HUB"],
    }
}

// ── Heartbeat ───────────────────────────────────────────────────────────────

/// Outcome of the heartbeat loop — signals why it exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeartbeatOutcome {
    /// User requested shutdown (Ctrl-C / SIGTERM).
    Shutdown,
    /// Too many consecutive heartbeat failures.
    FailuresExhausted,
    /// Hub sent a permanent rejection (evicted).
    HubRejected,
    /// This node was administratively removed by the Hub.
    NodeRejected,
}

/// Blocking heartbeat loop — runs until shutdown is signaled, failures exhaust,
/// or the Hub rejects this member/node.
fn heartbeat_loop(
    bridge: &easynet_axon::dendrite_bridge::DendriteBridge,
    tenant: &str,
    node_id: &str,
    interval_ms: u64,
    shutdown: &ShutdownSignal,
) -> HeartbeatOutcome {
    let interval = std::time::Duration::from_millis(interval_ms);
    let mut failures = 0u32;
    while !shutdown.is_triggered() {
        // Sleep for the heartbeat interval, waking immediately if shutdown is signaled.
        let timed_out = shutdown.wait_timeout(interval);
        if !timed_out {
            break; // Shutdown was signaled during wait.
        }
        match bridge.node_heartbeat(tenant, node_id) {
            Ok(resp) => {
                if failures > 0 {
                    eprintln!("heartbeat recovered after {failures} failures");
                    failures = 0;
                }
                // Check for permanent rejection — Hub has evicted this member.
                if resp.get("permanent").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                    let status = resp.get("status").and_then(serde_json::Value::as_str).unwrap_or("unknown");
                    eprintln!("heartbeat permanently rejected by hub (status: {status}), disconnecting");
                    return HeartbeatOutcome::HubRejected;
                }
                // Check if this device's node was administratively removed.
                let self_rejected = resp
                    .get("rejected_nodes")
                    .and_then(|v| v.as_array())
                    .is_some_and(|arr| {
                        arr.iter()
                            .filter_map(|v| v.get("node_id").and_then(|n| n.as_str()))
                            .any(|id| id == node_id)
                    });
                if self_rejected {
                    eprintln!("this node ({node_id}) was rejected by hub, disconnecting");
                    return HeartbeatOutcome::NodeRejected;
                }
            }
            Err(e) => {
                failures += 1;
                eprintln!("heartbeat failed ({failures}/{MAX_HEARTBEAT_FAILURES}): {e}");
                if failures >= MAX_HEARTBEAT_FAILURES {
                    eprintln!("heartbeat lost — initiating graceful shutdown");
                    return HeartbeatOutcome::FailuresExhausted;
                }
            }
        }
    }
    HeartbeatOutcome::Shutdown
}

/// Fork a background daemon that handles heartbeat + deregister on SIGTERM.
fn spawn_heartbeat_daemon(
    endpoint: &str,
    tenant: &str,
    node_id: &str,
    heartbeat_ms: u64,
) -> anyhow::Result<()> {
    let exe = std::env::current_exe()
        .context("resolve exe path")?;

    let log_dir = config::home_dir().join(".easynet").join("logs");
    std::fs::create_dir_all(&log_dir)?;
    let log_path = log_dir.join("heartbeat.log");
    let log_fh = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)?;
    let log_err = log_fh.try_clone()?;

    let child = std::process::Command::new(exe)
        .arg("_heartbeat-daemon")
        .env("_EASYNET_HB_ENDPOINT", endpoint)
        .env("_EASYNET_HB_TENANT", tenant)
        .env("_EASYNET_HB_NODE_ID", node_id)
        .env("_EASYNET_HB_INTERVAL_MS", heartbeat_ms.to_string())
        .stdout(log_fh)
        .stderr(log_err)
        .spawn()
        .context("spawn heartbeat daemon")?;

    let hb_pid_path = config::home_dir().join(".easynet").join("heartbeat.pid");
    std::fs::write(&hb_pid_path, child.id().to_string())?;

    output::detail("heartbeat daemon", &format!("pid {} (log: {})", child.id(), log_path.display()));

    Ok(())
}

/// Entry point for the heartbeat daemon subprocess (hidden subcommand).
pub fn run_heartbeat_daemon() -> anyhow::Result<()> {
    let endpoint = std::env::var("_EASYNET_HB_ENDPOINT")
        .map_err(|_| anyhow::anyhow!("missing _EASYNET_HB_ENDPOINT"))?;
    let tenant = std::env::var("_EASYNET_HB_TENANT")
        .map_err(|_| anyhow::anyhow!("missing _EASYNET_HB_TENANT"))?;
    let node_id = std::env::var("_EASYNET_HB_NODE_ID")
        .map_err(|_| anyhow::anyhow!("missing _EASYNET_HB_NODE_ID"))?;
    let interval_ms: u64 = std::env::var("_EASYNET_HB_INTERVAL_MS")
        .unwrap_or_else(|_| DEFAULT_HEARTBEAT_MS.to_string())
        .parse()?;

    let bridge = shared::connect_bridge_to(&endpoint)?;

    let shutdown = ShutdownSignal::new();
    let s = shutdown.clone();
    ctrlc::set_handler(move || {
        s.trigger();
    })?;

    let outcome = heartbeat_loop(&bridge, &tenant, &node_id, interval_ms, &shutdown);

    let reason = match outcome {
        HeartbeatOutcome::FailuresExhausted => "heartbeat lost",
        HeartbeatOutcome::HubRejected => "hub rejected",
        HeartbeatOutcome::NodeRejected => "node rejected",
        HeartbeatOutcome::Shutdown => "device shutdown",
    };
    let _ = bridge.deregister_node(&tenant, &node_id, reason);
    if outcome == HeartbeatOutcome::NodeRejected {
        config::delete_credentials().ok();
        eprintln!("heartbeat daemon: device removed by admin — credentials cleaned up");
    }
    eprintln!("heartbeat daemon: deregistered {node_id} ({reason}), exiting");
    Ok(())
}

// ── Credential verification ─────────────────────────────────────────────────

/// Verify device credentials with the backend API.
/// Extracts the HTTPS host from the hub endpoint (supports axon://, http://, https://).
fn verify_credential(creds: &config::Credentials) -> CredentialCheck {
    let host = extract_host(&creds.hub_endpoint);
    let url = format!("https://{host}/api/v1/devices/verify-credential");

    let resp = ureq::post(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send_json(serde_json::json!({
            "node_id": creds.node_id,
            "credential_token": creds.credential_token,
        }));

    match resp {
        Ok(r) if (200..300).contains(&r.status()) => CredentialCheck::Valid,
        Ok(r) => {
            let body = r.into_string().unwrap_or_default();
            CredentialCheck::Revoked(format!("credential rejected: {body}"))
        }
        Err(ureq::Error::Status(code @ (401 | 403), _)) => {
            CredentialCheck::Revoked(format!("credential revoked or device removed (HTTP {code})"))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            CredentialCheck::Revoked(format!("Hub rejected credential (HTTP {code}): {body}"))
        }
        Err(e) => {
            eprintln!("  warning: could not verify credential ({e}), continuing anyway");
            CredentialCheck::NetworkUnavailable
        }
    }
}

/// Extract the hostname from an endpoint URL.
/// Handles axon://host:port, http://host:port, https://host:port, and bare host:port.
fn extract_host(endpoint: &str) -> &str {
    let endpoint = endpoint.trim();
    let without_scheme = endpoint
        .strip_prefix("axon://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .or_else(|| endpoint.strip_prefix("http://"))
        .unwrap_or(endpoint);
    // Take everything before the first ':' or '/' (whichever comes first).
    let end = without_scheme
        .find([':', '/'])
        .unwrap_or(without_scheme.len());
    let host = &without_scheme[..end];
    if host.is_empty() {
        config::DEFAULT_HUB_HOST
    } else {
        host
    }
}
