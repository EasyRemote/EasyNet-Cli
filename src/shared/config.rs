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
use std::path::PathBuf;

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

/// Resolve the user's home directory. Falls back to "." if neither HOME nor USERPROFILE is set.
pub fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map_or_else(|_| PathBuf::from("."), PathBuf::from)
}

fn state_dir() -> PathBuf {
    home_dir().join(".easynet")
}

fn state_path() -> PathBuf {
    state_dir().join("runtime.json")
}

pub fn save(state: &RuntimeState) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(state)?;
    fs::write(state_path(), json)?;
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
        self.tenant.as_deref().unwrap_or("default")
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
}

fn credentials_path() -> PathBuf {
    state_dir().join("credentials.json")
}

pub fn save_credentials(creds: &Credentials) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(creds)? + "\n";
    let path = credentials_path();
    fs::write(&path, json)?;
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
    if creds.node_id.is_empty() || creds.hub_endpoint.is_empty() || creds.tenant_id.is_empty() {
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
    fs::write(device_settings_path(), json)?;
    Ok(())
}
