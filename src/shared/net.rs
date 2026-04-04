// EasyNet CLI — Network & Process Utilities
// ==========================================
//
// File: src/shared/net.rs
// Description: Shared network and process discovery functions used by start.rs and stop.rs.
//
// Extracted from start.rs and stop.rs to eliminate duplication:
//   - parse_port_from_endpoint(): extract port from http/https/axon endpoint strings
//   - discover_pid_from_endpoint(): find PID listening on the endpoint's port
//   - find_listening_pid(): Unix lsof-based PID lookup
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

/// Extract port number from an endpoint URL (supports http://, https://, axon://, and IPv6).
pub fn parse_port_from_endpoint(endpoint: &str) -> Option<u16> {
    let endpoint = endpoint.trim();
    let without_scheme = endpoint
        .strip_prefix("http://")
        .or_else(|| endpoint.strip_prefix("https://"))
        .or_else(|| endpoint.strip_prefix("axon://"))
        .unwrap_or(endpoint);
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    if authority.starts_with('[') {
        // IPv6: [::1]:50051
        let end = authority.find(']')?;
        let rest = &authority[end + 1..];
        rest.strip_prefix(':')?.parse().ok()
    } else {
        let idx = authority.rfind(':')?;
        authority[idx + 1..].parse().ok()
    }
}

/// Discover the PID of the process listening on the endpoint's port.
pub fn discover_pid_from_endpoint(endpoint: &str) -> Option<u32> {
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

/// Check whether a process with the given PID is still alive.
#[cfg(unix)]
pub fn is_pid_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    unsafe { libc::kill(pid, 0) == 0 }
}

#[cfg(not(unix))]
pub fn is_pid_alive(_pid: u32) -> bool {
    // Cannot check on non-unix; assume alive to be safe.
    true
}

/// Find the PID of a process listening on the given TCP port (Unix only, via lsof).
#[cfg(unix)]
pub fn find_listening_pid(port: u16) -> Option<u32> {
    use std::process::Command;

    let out = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout.lines().next()?.trim().parse().ok()
}
