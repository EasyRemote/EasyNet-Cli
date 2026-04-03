// EasyNet CLI
// ===========
//
// File: src/cli/stop.rs
// Description: `easynet stop` — terminates the locally running Axon runtime.
//
// Shutdown Strategy:
// - Reads PID from runtime.json, or discovers it via `lsof` on the endpoint port.
// - Sends SIGTERM and waits up to 5 seconds for graceful exit.
// - Always clears ~/.easynet/runtime.json regardless of signal delivery outcome.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use clap::Args;

use crate::shared::{config, output};

#[derive(Debug, Args)]
pub struct StopArgs {}

pub fn run(_args: StopArgs) -> anyhow::Result<()> {
    let state = config::load()?;
    output::info(&format!("Stopping runtime at {}...", state.endpoint));

    let pid = state.pid.or_else(|| discover_pid_from_endpoint(&state.endpoint));
    if let Some(pid) = pid {
        #[cfg(unix)]
        {
            stop_pid(pid);
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            anyhow::bail!("stop is not supported on this platform (no unix signals)");
        }
    } else {
        output::info("warning: could not determine runtime pid; clearing state file only");
    }

    config::remove()?;
    output::success("Axon runtime stopped");
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

#[cfg(unix)]
fn stop_pid(pid: u32) {
    // Best-effort: SIGTERM + short wait. (Stop will still clear runtime.json.)
    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        let alive = unsafe { libc::kill(pid as i32, 0) == 0 };
        if !alive {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
