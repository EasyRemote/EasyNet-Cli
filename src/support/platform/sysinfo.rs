// EasyNet CLI — System Info
// =========================
//
// File: src/shared/sysinfo.rs
// Description: Collects local device fingerprint (hostname, OS, architecture) for pairing
//              and node registration payloads.
//
// Protocol Responsibility:
// - Produces the device metadata sent to Hub during pairing (join.rs) and node registration.
// - Fields match the Hub's DevicePairing schema: display_name, os, arch, hostname.
// - Uses compile-time constants (std::env::consts) for OS/arch — no runtime detection overhead.
//
// Architectural Position:
// - Leaf utility with no dependencies on config or network. Used by join.rs.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::Serialize;

/// Device fingerprint sent to Hub during pairing and registration.
#[derive(Debug, Serialize)]
pub struct DeviceInfo {
    pub display_name: String,
    pub os: &'static str,
    pub arch: &'static str,
    pub hostname: String,
}

/// Sentinel hostname used when the OS reports an empty value. The Hub's
/// pairing schema requires a non-empty `hostname`/`display_name`, and an
/// empty string would render as a blank row in `easynet device list`.
const UNKNOWN_HOSTNAME: &str = "unknown-host";

pub fn collect_system_info() -> DeviceInfo {
    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    // `gethostname` returns an empty `OsString` on misconfigured hosts
    // (e.g. containers with no /etc/hostname). Fall back to a sentinel
    // rather than emit an empty `hostname` field — the Hub side trims and
    // would otherwise reject the registration with a non-obvious error.
    let hostname = if hostname.trim().is_empty() {
        UNKNOWN_HOSTNAME.to_string()
    } else {
        hostname
    };

    DeviceInfo {
        display_name: hostname.clone(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        hostname,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collected_hostname_is_never_empty() {
        // The Hub pairing schema requires non-empty hostname/display_name;
        // pinning the contract here means a future refactor of the
        // `gethostname` fallback can't silently regress to an empty
        // string.
        let info = collect_system_info();
        assert!(!info.hostname.is_empty(), "hostname must not be empty");
        assert!(
            !info.display_name.is_empty(),
            "display_name must not be empty"
        );
    }
}
