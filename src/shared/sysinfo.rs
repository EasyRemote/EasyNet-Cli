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

pub fn collect_system_info() -> DeviceInfo {
    let hostname = gethostname::gethostname()
        .to_string_lossy()
        .into_owned();

    DeviceInfo {
        display_name: hostname.clone(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        hostname,
    }
}
