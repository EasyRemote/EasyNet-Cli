// EasyNet CLI
// ===========
//
// File: src/cli/start.rs
// Description: `easynet start` — spawns a local Axon runtime and joins a Hub via federation.
//
// Lifecycle:
// - Ensures no runtime is already running (checks ~/.easynet/runtime.json).
// - Uses `ServerConfig` from the Axon SDK to auto-start a local runtime on a free port.
// - Discovers the spawned process PID via `lsof` (Unix) for later `easynet stop`.
// - Persists endpoint, PID, Hub URL, tenant, and label to ~/.easynet/runtime.json.
// - In foreground mode: blocks on Ctrl-C, then gracefully shuts down via ServerHandle::drop().
// - In background mode: `mem::forget(handle)` detaches the child process.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clap::Args;
use easynet_axon::server::ServerConfig;

use crate::shared::{config, output};

#[derive(Debug, Args)]
pub struct StartArgs {
    /// Hub endpoint (e.g. axon://hub.easynet.run:50084)
    #[arg(long)]
    pub hub: String,
    /// Tenant ID
    #[arg(long, default_value = "default")]
    pub tenant: String,
    /// Human-readable label for this device
    #[arg(long)]
    pub label: Option<String>,
    /// Pre-shared join token
    #[arg(long)]
    pub token: Option<String>,
    /// Run in foreground (block until Ctrl-C)
    #[arg(long)]
    pub foreground: bool,
}

pub fn run(args: StartArgs) -> anyhow::Result<()> {
    if config::load().is_ok() {
        anyhow::bail!(
            "runtime already running (run `easynet stop` first, or remove ~/.easynet/runtime.json)"
        );
    }

    // `ServerConfig` consults `EASYNET_AXON_ENDPOINT` to connect to an existing server.
    // `easynet start --hub ...` should always auto-start a local runtime and join the hub.
    std::env::remove_var("EASYNET_AXON_ENDPOINT");

    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    let label = args.label.as_deref().unwrap_or(&hostname);

    let mut cfg = ServerConfig::default()
        .hub(&args.hub)
        .hub_tenant(&args.tenant)
        .hub_label(label)
        .insecure(true);
    if let Some(ref t) = args.token {
        cfg = cfg.hub_join_token(t);
    }

    let srv = cfg
        .start()
        .map_err(|e| anyhow::anyhow!("start runtime: {e}"))?;

    let endpoint = srv.url().to_string();
    let pid = discover_pid_from_endpoint(&endpoint);

    let state = config::RuntimeState {
        endpoint: endpoint.clone(),
        pid,
        hub: Some(args.hub.clone()),
        tenant: Some(args.tenant.clone()),
        label: Some(label.to_string()),
        started_at: Some(chrono::Utc::now().to_rfc3339()),
    };
    config::save(&state)?;

    output::success(&format!("Axon runtime started on {endpoint}"));
    output::success(&format!("Joined {} as {label}", args.hub));
    output::info(&format!("  tenant: {}", args.tenant));
    if let Some(pid) = pid {
        output::info(&format!("  pid: {pid}"));
    }

    if args.foreground {
        output::info("Running in foreground (Ctrl-C to stop)...");
        let running = Arc::new(AtomicBool::new(true));
        let r = running.clone();
        ctrlc::set_handler(move || r.store(false, Ordering::SeqCst))?;
        while running.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        output::info("\nShutting down...");
        drop(srv);
        config::remove()?;
        output::success("Axon runtime stopped");
    } else {
        std::mem::forget(srv);
        output::info("Runtime running in background. Use `easynet stop` to stop.");
    }

    Ok(())
}

fn discover_pid_from_endpoint(endpoint: &str) -> Option<u32> {
    let port = parse_port_from_endpoint(endpoint)?;
    #[cfg(unix)]
    {
        find_listening_pid(port)
    }
    #[cfg(not(unix))]
    {
        let _ = port;
        None
    }
}

fn parse_port_from_endpoint(endpoint: &str) -> Option<u16> {
    let endpoint = endpoint.trim();
    let without_scheme = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .unwrap_or(endpoint);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    if authority.starts_with('[') {
        let end = authority.find(']')?;
        let rest = &authority[end + 1..];
        rest.strip_prefix(':')?.parse().ok()
    } else {
        let idx = authority.rfind(':')?;
        authority[idx + 1..].parse().ok()
    }
}

#[cfg(unix)]
fn find_listening_pid(port: u16) -> Option<u32> {
    use std::process::Command;

    let out = Command::new("lsof")
        .args([
            "-nP",
            &format!("-iTCP:{port}"),
            "-sTCP:LISTEN",
            "-t",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout.lines().next()?.trim().parse().ok()
}
