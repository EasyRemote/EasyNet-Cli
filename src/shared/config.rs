// EasyNet CLI — Runtime Configuration
// ====================================
//
// File: src/shared/config.rs
// Description: Persistent runtime state at ~/.easynet/runtime.json.
//
// Lifecycle:
//   `easynet start` → save(RuntimeState)
//   `easynet *`     → load() to discover endpoint
//   `easynet stop`  → remove()
//
// Fields: endpoint (required), pid, hub, tenant, label, started_at (all optional).
// The `tenant_or_default()` helper returns "default" when tenant is unset.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub endpoint: String,
    pub pid: Option<u32>,
    pub hub: Option<String>,
    pub tenant: Option<String>,
    pub label: Option<String>,
    pub started_at: Option<String>,
}

fn state_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".easynet")
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
