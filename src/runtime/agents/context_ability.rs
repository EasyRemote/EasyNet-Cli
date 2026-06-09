// EasyNet CLI — context.* device abilities
// ==========================================
//
// File: src/runtime/agents/context_ability.rs
// Description: Read/toggle surfaces over the Context store
//              (`persistence::context_store`) — the device-global
//              clipboard history, user-mapped project folders, and
//              favorites that back the Frontend "Context" page.
//
//   * `context.clipboard.list`   — newest-first clip entries + tracking flag
//   * `context.clipboard.get`    — one clip, with base64 PNG for images
//   * `context.clipboard.track`  — enable/disable capture (persisted;
//                                  the tracker thread re-reads per tick)
//   * `context.folders.list`     — mapped project folders
//   * `context.fs.list`          — browse one level inside a mapping
//                                  (containment-checked)
//   * `context.favorites.list`   — favorites
//   * `context.favorites.add`    — star a clip / file / folder
//   * `context.favorites.remove` — unstar
//
// Owner is Device: the clipboard and the folder mappings are
// per-device state, captured/served by whichever daemon hosts them —
// the same ownership reasoning as `chat.history.*`.
//
// Folder mappings are ADDED/REMOVED via the `easynet context` CLI by
// design (the mapping grants filesystem read access, so it stays a
// local, operator-initiated act); the abilities only read.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use base64::Engine as _;
use serde_json::{json, Value};

use crate::persistence::context_store;
use crate::runtime::ability_dispatch::{AxonAbilityCatalog, OwnerKind};

pub const ABILITY_CLIPBOARD_LIST: &str = "context.clipboard.list";
pub const ABILITY_CLIPBOARD_GET: &str = "context.clipboard.get";
pub const ABILITY_CLIPBOARD_TRACK: &str = "context.clipboard.track";
pub const ABILITY_FOLDERS_LIST: &str = "context.folders.list";
pub const ABILITY_FS_LIST: &str = "context.fs.list";
pub const ABILITY_FAVORITES_LIST: &str = "context.favorites.list";
pub const ABILITY_FAVORITES_ADD: &str = "context.favorites.add";
pub const ABILITY_FAVORITES_REMOVE: &str = "context.favorites.remove";

/// Register every context ability. Called from
/// `runtime::agents::build_registry`.
pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner(
        ABILITY_CLIPBOARD_LIST,
        OwnerKind::Device,
        std::sync::Arc::new(clipboard_list_handler),
    );
    reg.register_rpc_with_owner(
        ABILITY_CLIPBOARD_GET,
        OwnerKind::Device,
        std::sync::Arc::new(clipboard_get_handler),
    );
    reg.register_rpc_with_owner(
        ABILITY_CLIPBOARD_TRACK,
        OwnerKind::Device,
        std::sync::Arc::new(clipboard_track_handler),
    );
    reg.register_rpc_with_owner(
        ABILITY_FOLDERS_LIST,
        OwnerKind::Device,
        std::sync::Arc::new(folders_list_handler),
    );
    reg.register_rpc_with_owner(
        ABILITY_FS_LIST,
        OwnerKind::Device,
        std::sync::Arc::new(fs_list_handler),
    );
    reg.register_rpc_with_owner(
        ABILITY_FAVORITES_LIST,
        OwnerKind::Device,
        std::sync::Arc::new(favorites_list_handler),
    );
    reg.register_rpc_with_owner(
        ABILITY_FAVORITES_ADD,
        OwnerKind::Device,
        std::sync::Arc::new(favorites_add_handler),
    );
    reg.register_rpc_with_owner(
        ABILITY_FAVORITES_REMOVE,
        OwnerKind::Device,
        std::sync::Arc::new(favorites_remove_handler),
    );
}

fn clipboard_list_handler(args: Value) -> anyhow::Result<Value> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(100)
        .max(1) as usize;
    let entries: Vec<Value> = context_store::list_clips(limit)
        .iter()
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();
    Ok(json!({
        "tracking": context_store::clipboard_tracking(),
        "entries": entries,
    }))
}

fn clipboard_get_handler(args: Value) -> anyhow::Result<Value> {
    let id = require_str(&args, "id", "context.clipboard.get")?;
    let entry = context_store::list_clips(200)
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| anyhow::anyhow!("context.clipboard.get: no clip {id}"))?;
    let mut out = serde_json::to_value(&entry)?;
    if entry.kind == "image" {
        let path = context_store::clip_image_abs_path(&id)
            .ok_or_else(|| anyhow::anyhow!("context.clipboard.get: image file missing"))?;
        let bytes = std::fs::read(path)?;
        out["data_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(bytes));
        out["content_type"] = json!("image/png");
    }
    Ok(out)
}

fn clipboard_track_handler(args: Value) -> anyhow::Result<Value> {
    let enabled = args
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| anyhow::anyhow!("context.clipboard.track: `enabled` (bool) required"))?;
    context_store::set_clipboard_tracking(enabled)?;
    Ok(json!({ "tracking": enabled }))
}

fn folders_list_handler(_args: Value) -> anyhow::Result<Value> {
    let folders: Vec<Value> = context_store::list_folders()
        .iter()
        .map(|f| {
            json!({
                "name": f.name,
                "path": f.path,
                "added_at": f.added_at,
                "exists": std::path::Path::new(&f.path).is_dir(),
            })
        })
        .collect();
    Ok(json!({ "folders": folders }))
}

fn fs_list_handler(args: Value) -> anyhow::Result<Value> {
    let folder = require_str(&args, "folder", "context.fs.list")?;
    let rel = args
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    context_store::list_folder_entries(&folder, &rel)
}

fn favorites_list_handler(_args: Value) -> anyhow::Result<Value> {
    let favorites: Vec<Value> = context_store::list_favorites()
        .iter()
        .map(|f| serde_json::to_value(f).unwrap_or(Value::Null))
        .collect();
    Ok(json!({ "favorites": favorites }))
}

fn favorites_add_handler(args: Value) -> anyhow::Result<Value> {
    let kind = require_str(&args, "kind", "context.favorites.add")?;
    let label = require_str(&args, "label", "context.favorites.add")?;
    let reference = require_str(&args, "reference", "context.favorites.add")?;
    let fav = context_store::add_favorite(&kind, &label, &reference)?;
    Ok(serde_json::to_value(fav)?)
}

fn favorites_remove_handler(args: Value) -> anyhow::Result<Value> {
    let id = require_str(&args, "id", "context.favorites.remove")?;
    let removed = context_store::remove_favorite(&id)?;
    Ok(serde_json::to_value(removed)?)
}

fn require_str(args: &Value, key: &str, ability: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("{ability}: `{key}` required"))
}

// ── descriptions + schemas (wired in agents/mod.rs) ─────────────────

pub fn description_for(name: &str) -> Option<&'static str> {
    Some(match name {
        ABILITY_CLIPBOARD_LIST => {
            "List captured clipboard history (newest first) with the tracking flag."
        }
        ABILITY_CLIPBOARD_GET => {
            "Read one clipboard entry; image entries include base64 PNG data."
        }
        ABILITY_CLIPBOARD_TRACK => "Enable or disable clipboard history tracking on this device.",
        ABILITY_FOLDERS_LIST => "List the project folders mapped via `easynet context add`.",
        ABILITY_FS_LIST => "Browse one directory level inside a mapped project folder.",
        ABILITY_FAVORITES_LIST => "List favorites (starred clips, files, folders).",
        ABILITY_FAVORITES_ADD => "Star a clipboard entry, file, or folder.",
        ABILITY_FAVORITES_REMOVE => "Remove a favorite by id.",
        _ => return None,
    })
}

pub fn input_schema_for(name: &str) -> Option<Value> {
    let schema = match name {
        ABILITY_CLIPBOARD_LIST => json!({
            "type": "object",
            "properties": {
                "limit": {"type": "integer", "description": "Max entries to return (default 100, cap 200)."}
            },
            "additionalProperties": false,
        }),
        ABILITY_CLIPBOARD_GET => json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Clip id from context.clipboard.list."}
            },
            "required": ["id"],
            "additionalProperties": false,
        }),
        ABILITY_CLIPBOARD_TRACK => json!({
            "type": "object",
            "properties": {
                "enabled": {"type": "boolean", "description": "true to start capturing, false to stop."}
            },
            "required": ["enabled"],
            "additionalProperties": false,
        }),
        ABILITY_FOLDERS_LIST => json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false,
        }),
        ABILITY_FS_LIST => json!({
            "type": "object",
            "properties": {
                "folder": {"type": "string", "description": "Mapped folder name (or its path)."},
                "path": {"type": "string", "description": "Relative path inside the folder; empty = root."}
            },
            "required": ["folder"],
            "additionalProperties": false,
        }),
        ABILITY_FAVORITES_LIST => json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false,
        }),
        ABILITY_FAVORITES_ADD => json!({
            "type": "object",
            "properties": {
                "kind": {"type": "string", "description": "clipboard | file | folder"},
                "label": {"type": "string", "description": "Display label."},
                "reference": {"type": "string", "description": "Clip id or path the favorite points at."}
            },
            "required": ["kind", "label", "reference"],
            "additionalProperties": false,
        }),
        ABILITY_FAVORITES_REMOVE => json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Favorite id from context.favorites.list."}
            },
            "required": ["id"],
            "additionalProperties": false,
        }),
        _ => return None,
    };
    Some(schema)
}

/// Every context ability name, for registration loops/tests.
pub const ALL: [&str; 8] = [
    ABILITY_CLIPBOARD_LIST,
    ABILITY_CLIPBOARD_GET,
    ABILITY_CLIPBOARD_TRACK,
    ABILITY_FOLDERS_LIST,
    ABILITY_FS_LIST,
    ABILITY_FAVORITES_LIST,
    ABILITY_FAVORITES_ADD,
    ABILITY_FAVORITES_REMOVE,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_toggle_then_list_reflects_state() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        clipboard_track_handler(json!({"enabled": true})).unwrap();
        let out = clipboard_list_handler(json!({})).unwrap();
        assert_eq!(out["tracking"], true);
        assert_eq!(out["entries"].as_array().unwrap().len(), 0);
        clipboard_track_handler(json!({"enabled": false})).unwrap();
        assert_eq!(clipboard_list_handler(json!({})).unwrap()["tracking"], false);
    }

    #[test]
    fn track_requires_enabled_bool() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        assert!(clipboard_track_handler(json!({})).is_err());
        assert!(clipboard_track_handler(json!({"enabled": "yes"})).is_err());
    }

    #[test]
    fn fs_list_requires_folder_and_descriptions_schemas_cover_all() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        assert!(fs_list_handler(json!({})).is_err());
        for name in ALL {
            assert!(description_for(name).is_some(), "{name} description");
            assert!(input_schema_for(name).is_some(), "{name} schema");
        }
    }

    #[test]
    fn favorites_add_validates_and_round_trips() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        assert!(favorites_add_handler(json!({"kind": "clipboard"})).is_err());
        let fav = favorites_add_handler(
            json!({"kind": "clipboard", "label": "snippet", "reference": "c1"}),
        )
        .unwrap();
        let id = fav["id"].as_str().unwrap().to_string();
        let listed = favorites_list_handler(json!({})).unwrap();
        assert_eq!(listed["favorites"].as_array().unwrap().len(), 1);
        favorites_remove_handler(json!({"id": id})).unwrap();
        assert_eq!(
            favorites_list_handler(json!({})).unwrap()["favorites"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }
}
