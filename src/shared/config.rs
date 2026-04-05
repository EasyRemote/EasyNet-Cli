// EasyNet CLI — Runtime Configuration
// ====================================
//
// File: src/shared/config.rs
// Description: Persistence layer for all local device state under ~/.easynet/.
//
// Protocol Responsibility:
// - Manages three orthogonal persistence domains, each with distinct lifecycles:
//
//   1. RuntimeState (runtime.json) — ephemeral, one session:
//      `easynet start` → save()  |  `easynet *` → load()  |  `easynet stop` → remove()
//      Fields: endpoint (required), pid, hub, tenant, label, started_at.
//
//   2. Credentials (credentials.json) — long-lived, survives reboots:
//      `easynet join` → save_credentials()  |  `easynet reset` → delete_credentials()
//      Fields: node_id, credential_token, hub_endpoint, tenant_id, deploy_signature.
//      Unix permissions: 0o600 (contains credential_token and deploy_signature).
//
//   3. DeviceSettings (device_settings.json) — user-controlled knobs:
//      `easynet config` → save/load  |  consumed by start.rs at boot.
//      Fields: session_bridge_exec_enabled (default false).
//
// Implementation Approach:
// - All files share ~/.easynet/ directory (state_dir()).
// - JSON pretty-printed for human readability and git-friendliness.
// - Credentials separated from runtime state to avoid mixing server-issued secrets
//   with ephemeral session data.
//
// Architectural Position:
// - Foundation layer consumed by every CLI command. No network dependencies.
// - Single source of truth for "where is the runtime" and "who am I on the Hub."
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Atomic write: write to a temp file in the same directory, then rename.
/// Prevents corruption if the process crashes mid-write.
fn atomic_write(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("config")
    ));
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

// ─── Platform-wide defaults ────────────────────────────────────────────────
// Single source of truth for default Hub/tenant/bind values.
// Consumed by start.rs, connect.rs, join.rs — never hardcode these elsewhere.

pub const DEFAULT_HUB: &str = "axon://easynet.run:50051";
pub const DEFAULT_HUB_HOST: &str = "easynet.run";
pub const DEFAULT_TENANT: &str = "easynet-platform";
pub const DEFAULT_BIND: &str = "0.0.0.0:50051";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub endpoint: String,
    pub pid: Option<u32>,
    pub hub: Option<String>,
    pub tenant: Option<String>,
    pub label: Option<String>,
    pub started_at: Option<String>,
    /// Whether the device credential was verified with the Hub at startup.
    /// `None` = not applicable (hub mode), `Some(false)` = Hub unreachable, `Some(true)` = verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_verified: Option<bool>,
}

/// Resolve the user's home directory.
/// Returns the first available of `$HOME`, `$USERPROFILE`, or the OS-provided home.
pub fn home_dir() -> PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        return PathBuf::from(h);
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        return PathBuf::from(h);
    }
    // Last resort: platform home_dir (works on most systems).
    #[allow(deprecated)]
    std::env::home_dir().unwrap_or_else(|| {
        eprintln!("warning: cannot determine home directory; using current directory");
        PathBuf::from(".")
    })
}

pub fn state_dir() -> PathBuf {
    home_dir().join(".easynet")
}

fn state_path() -> PathBuf {
    state_dir().join("runtime.json")
}

pub fn save(state: &RuntimeState) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(state)?;
    atomic_write(&state_path(), json.as_bytes())?;
    Ok(())
}

pub fn load() -> anyhow::Result<RuntimeState> {
    let path = state_path();
    let data = fs::read_to_string(&path)
        .map_err(|_| anyhow::anyhow!("no running runtime — run `easynet start` first"))?;
    let state: RuntimeState = serde_json::from_str(&data)?;
    Ok(state)
}

pub fn remove() -> anyhow::Result<()> {
    let path = state_path();
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

impl RuntimeState {
    pub fn tenant_or_default(&self) -> &str {
        self.tenant.as_deref().unwrap_or(DEFAULT_TENANT)
    }
}

// ─── Device Credentials ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub node_id: String,
    pub credential_token: String,
    pub hub_endpoint: String,
    pub tenant_id: String,
    #[serde(default)]
    pub deploy_signature: String,
    /// Optional Hub REST API base URL (e.g. "http://localhost:8080") for local dev.
    /// When absent, derived from `hub_endpoint` by stripping scheme/port and using HTTPS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_api_base: Option<String>,
}

impl Credentials {
    /// Resolve the Hub REST API base URL.
    /// Uses `hub_api_base` if set, otherwise derives from `hub_endpoint`.
    pub fn api_base(&self) -> String {
        if let Some(ref base) = self.hub_api_base {
            return base.trim_end_matches('/').to_string();
        }
        let host = extract_api_host(&self.hub_endpoint);
        format!("https://{host}")
    }
}

/// Extract the hostname from an endpoint URL for REST API calls.
///
/// For `axon://` endpoints, strips the gRPC port since the REST API uses HTTPS/443.
/// For `http://`/`https://` endpoints, preserves the authority (host:port) as-is.
fn extract_api_host(endpoint: &str) -> String {
    let endpoint = endpoint.trim();
    let (is_axon, without_scheme) = if let Some(rest) = endpoint.strip_prefix("axon://") {
        (true, rest)
    } else if let Some(rest) = endpoint.strip_prefix("https://") {
        (false, rest)
    } else if let Some(rest) = endpoint.strip_prefix("http://") {
        (false, rest)
    } else {
        (false, endpoint)
    };
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    if authority.is_empty() {
        return DEFAULT_HUB_HOST.to_string();
    }
    // IPv6 bracketed address: [::1]:port or [::1]
    if authority.starts_with('[') {
        if let Some(bracket_end) = authority.find(']') {
            let host_part = &authority[..=bracket_end]; // includes brackets
            if is_axon {
                // Strip port for axon:// — REST API uses HTTPS/443.
                return host_part.to_string();
            }
            // http/https — preserve port if present.
            return authority.to_string();
        }
    }
    if is_axon {
        // axon:// uses gRPC port — strip it, REST API is on HTTPS/443.
        authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host)
            .to_string()
    } else {
        // http/https — preserve port for non-standard setups.
        authority.to_string()
    }
}

fn credentials_path() -> PathBuf {
    state_dir().join("credentials.json")
}

/// Path to the heartbeat daemon PID file.
/// Used by start.rs (write) and stop.rs (read + cleanup).
pub fn heartbeat_pid_path() -> PathBuf {
    state_dir().join("heartbeat.pid")
}

pub fn save_credentials(creds: &Credentials) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(creds)? + "\n";
    let path = credentials_path();
    atomic_write(&path, json.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn load_credentials() -> anyhow::Result<Credentials> {
    let path = credentials_path();
    let data = fs::read_to_string(&path)
        .map_err(|_| anyhow::anyhow!("no credentials found — run `easynet join <token>` first"))?;
    let creds: Credentials = serde_json::from_str(&data)?;
    if creds.node_id.is_empty() || creds.credential_token.is_empty()
        || creds.hub_endpoint.is_empty() || creds.tenant_id.is_empty()
    {
        anyhow::bail!("credentials file is incomplete — run `easynet join <token>` to re-pair");
    }
    Ok(creds)
}

pub fn delete_credentials() -> anyhow::Result<()> {
    let path = credentials_path();
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

// ─── Device Settings ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceSettings {
    #[serde(default)]
    pub session_bridge_exec_enabled: bool,
}

fn device_settings_path() -> PathBuf {
    state_dir().join("device_settings.json")
}

pub fn load_device_settings() -> DeviceSettings {
    let path = device_settings_path();
    fs::read_to_string(&path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
}

pub fn save_device_settings(settings: &DeviceSettings) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(settings)? + "\n";
    atomic_write(&device_settings_path(), json.as_bytes())?;
    Ok(())
}

// Agent registry types moved to shared/agents.rs to preserve this file's
// three-domain contract (runtime state / credentials / device settings).
