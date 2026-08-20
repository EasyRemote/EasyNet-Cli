// EasyNet CLI
// ===========
//
// File: src/cli/bridge_lib.rs
// Description: Single source of truth for locating the native dendrite
//   bridge dynamic library (`libaxon_dendrite_bridge.{dylib,so,dll}`).
//
// Why this module exists:
// - `easynet device join` stages the bridge lib into
//   `~/.easynet/dendrite-bridge/native/`, but the Axon SDK loader
//   (`resolve_native_lib`) does not search that path. It only checks
//   `EASYNET_DENDRITE_BRIDGE_LIB` / `_HOME`, a gated local-source build,
//   and a crate-relative embedded path.
// - The resolver chain below knows every place the CLI may have put the
//   lib. Previously it lived inside `mcp_install.rs` and only the
//   MCP-install flow benefited; the daemon-start flow inherited the
//   ambient env and failed with "dendrite bridge library not found"
//   even though the lib was staged on disk. This module hoists the
//   chain so both the MCP server and the daemon resolve the lib
//   identically.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use toml_edit::DocumentMut;

use crate::daemon::persistence::config;

/// Resolve the native dendrite bridge library path.
///
/// Precedence (first existing hit wins):
/// 1. `explicit` — a caller-supplied path (e.g. `--bridge-lib`). A
///    non-empty value that does not exist is a hard error; an empty
///    value means "no explicit override".
/// 2. `EASYNET_DENDRITE_BRIDGE_LIB` from this process's environment.
/// 3. `~/.easynet/dendrite-bridge/native/<libname>` — the data-dir
///    location `easynet device join` stages into.
/// 4. The lib already wired into `~/.claude/settings.json`.
/// 5. The lib already wired into `~/.codex/config.toml`.
/// 6. A local repo build (`EasyNet-Axon/core/runtime-rs/dendrite-bridge`).
///
/// Returns `Ok(None)` when nothing is found — callers decide whether
/// that is fatal.
pub(crate) fn resolve_bridge_lib(explicit: Option<&str>) -> anyhow::Result<Option<String>> {
    if let Some(raw) = explicit {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        let path = PathBuf::from(trimmed);
        anyhow::ensure!(path.exists(), "bridge lib not found at {}", path.display());
        return Ok(Some(trimmed.to_string()));
    }

    if let Ok(raw) = std::env::var("EASYNET_DENDRITE_BRIDGE_LIB") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() && PathBuf::from(trimmed).exists() {
            return Ok(Some(trimmed.to_string()));
        }
    }

    if let Some(v) = bridge_lib_from_easynet_home() {
        return Ok(Some(v));
    }
    if let Some(v) = bridge_lib_from_claude_settings()? {
        return Ok(Some(v));
    }
    if let Some(v) = bridge_lib_from_codex_config()? {
        return Ok(Some(v));
    }
    if let Some(v) = bridge_lib_from_local_repos() {
        return Ok(Some(v));
    }
    Ok(None)
}

pub(crate) fn default_bridge_lib_filename() -> &'static str {
    if cfg!(target_os = "macos") {
        "libaxon_dendrite_bridge.dylib"
    } else if cfg!(target_os = "windows") {
        "axon_dendrite_bridge.dll"
    } else {
        "libaxon_dendrite_bridge.so"
    }
}

fn bridge_lib_from_easynet_home() -> Option<String> {
    let home = config::home_dir();
    let candidate = home
        .join(".easynet")
        .join("dendrite-bridge")
        .join("native")
        .join(default_bridge_lib_filename());
    if candidate.exists() {
        return Some(candidate.to_string_lossy().into_owned());
    }
    None
}

fn bridge_lib_from_claude_settings() -> anyhow::Result<Option<String>> {
    let path = config::home_dir().join(".claude").join("settings.json");
    let Some(data) = read_optional_text(&path, "Claude MCP settings")? else {
        return Ok(None);
    };
    let v: Value = serde_json::from_str(&data).map_err(|error| {
        anyhow::anyhow!("parse Claude MCP settings {}: {error}", path.display())
    })?;
    let Some(servers) = v.get("mcpServers").and_then(|value| value.as_object()) else {
        return Ok(None);
    };
    for (_, server) in servers {
        let env = server.get("env").and_then(|v| v.as_object());
        let Some(env) = env else { continue };
        let lib = env
            .get("EASYNET_DENDRITE_BRIDGE_LIB")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(lib) = lib else { continue };
        if PathBuf::from(lib).exists() {
            return Ok(Some(lib.to_string()));
        }
    }
    Ok(None)
}

fn bridge_lib_from_codex_config() -> anyhow::Result<Option<String>> {
    let path = config::home_dir().join(".codex").join("config.toml");
    let Some(data) = read_optional_text(&path, "Codex MCP config")? else {
        return Ok(None);
    };
    let doc = data
        .parse::<DocumentMut>()
        .map_err(|error| anyhow::anyhow!("parse Codex MCP config {}: {error}", path.display()))?;
    let Some(servers) = doc.get("mcp_servers").and_then(|item| item.as_table()) else {
        return Ok(None);
    };
    for (_, item) in servers.iter() {
        let Some(server) = item.as_table() else {
            continue;
        };
        let Some(env) = server.get("env").and_then(|item| item.as_table()) else {
            continue;
        };
        let lib = env
            .get("EASYNET_DENDRITE_BRIDGE_LIB")
            .and_then(|i| i.as_value())
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(lib) = lib else { continue };
        if PathBuf::from(lib).exists() {
            return Ok(Some(lib.to_string()));
        }
    }
    Ok(None)
}

fn read_optional_text(path: &Path, label: &str) -> anyhow::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(data) => Ok(Some(data)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(anyhow::anyhow!("read {label} {}: {error}", path.display())),
    }
}

fn bridge_lib_from_local_repos() -> Option<String> {
    let filename = default_bridge_lib_filename();
    let mut cur = std::env::current_dir().ok()?;

    for _ in 0..8 {
        let direct_release = cur
            .join("core")
            .join("runtime-rs")
            .join("dendrite-bridge")
            .join("target")
            .join("release")
            .join(filename);
        if direct_release.exists() {
            return Some(direct_release.to_string_lossy().into_owned());
        }

        let direct_debug = cur
            .join("core")
            .join("runtime-rs")
            .join("dendrite-bridge")
            .join("target")
            .join("debug")
            .join(filename);
        if direct_debug.exists() {
            return Some(direct_debug.to_string_lossy().into_owned());
        }

        let sibling_release = cur
            .join("EasyNet-Axon")
            .join("core")
            .join("runtime-rs")
            .join("dendrite-bridge")
            .join("target")
            .join("release")
            .join(filename);
        if sibling_release.exists() {
            return Some(sibling_release.to_string_lossy().into_owned());
        }

        let sibling_debug = cur
            .join("EasyNet-Axon")
            .join("core")
            .join("runtime-rs")
            .join("dendrite-bridge")
            .join("target")
            .join("debug")
            .join(filename);
        if sibling_debug.exists() {
            return Some(sibling_debug.to_string_lossy().into_owned());
        }

        cur = cur.parent()?.to_path_buf();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_home_file(relative: &[&str], contents: &str) -> PathBuf {
        let path = relative
            .iter()
            .fold(config::home_dir(), |base, segment| base.join(segment));
        std::fs::create_dir_all(path.parent().expect("home fixture parent"))
            .expect("create home fixture parent");
        std::fs::write(&path, contents).expect("write home fixture");
        path
    }

    #[test]
    fn missing_bridge_configs_remain_absent() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();

        assert!(bridge_lib_from_claude_settings()
            .expect("missing Claude settings is absence")
            .is_none());
        assert!(bridge_lib_from_codex_config()
            .expect("missing Codex config is absence")
            .is_none());
    }

    #[test]
    fn claude_bridge_config_rejects_malformed_existing_json() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let path = write_home_file(&[".claude", "settings.json"], "{");

        let error = bridge_lib_from_claude_settings()
            .expect_err("malformed existing Claude settings must fail closed");
        let message = error.to_string();
        assert!(
            message.contains("parse Claude MCP settings"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains(&path.display().to_string()),
            "error must identify source path: {message}"
        );
    }

    #[test]
    fn codex_bridge_config_rejects_malformed_existing_toml() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let path = write_home_file(&[".codex", "config.toml"], "[mcp_servers");

        let error = bridge_lib_from_codex_config()
            .expect_err("malformed existing Codex config must fail closed");
        let message = error.to_string();
        assert!(
            message.contains("parse Codex MCP config"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains(&path.display().to_string()),
            "error must identify source path: {message}"
        );
    }
}
