// EasyNet CLI — context.* device abilities
// ==========================================
//
// File: src/daemon/ability/builtins/resources/context/ability.rs
// Description: Read/toggle surfaces over the Context store
//              (`persistence::context_store`) — the device-global
//              clipboard history, user-mapped project folders, and
//              favorites that back the Frontend "Context" page.
//
//   * `context.clipboard.list`   — newest-first unique clip entries +
//                                  duplicate counts + tracking flag
//   * `context.clipboard.get`    — one clip, with base64 PNG for images
//   * `context.clipboard.track`  — enable/disable capture (persisted;
//                                  the tracker thread re-reads per tick)
//   * `context.clipboard.remove` — delete one clip (and its PNG)
//   * `context.catalog`          — unified newest-first context picker
//                                  projection for the Frontend composer
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

use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::daemon::persistence::context_store;

pub const ABILITY_CLIPBOARD_LIST: &str =
    crate::daemon::ability::names::resources::CONTEXT_CLIPBOARD_LIST;
pub const ABILITY_CLIPBOARD_GET: &str =
    crate::daemon::ability::names::resources::CONTEXT_CLIPBOARD_GET;
pub const ABILITY_CLIPBOARD_TRACK: &str =
    crate::daemon::ability::names::resources::CONTEXT_CLIPBOARD_TRACK;
pub const ABILITY_CLIPBOARD_REMOVE: &str =
    crate::daemon::ability::names::resources::CONTEXT_CLIPBOARD_REMOVE;
pub const ABILITY_CATALOG: &str = crate::daemon::ability::names::resources::CONTEXT_CATALOG;
pub const ABILITY_FOLDERS_LIST: &str =
    crate::daemon::ability::names::resources::CONTEXT_FOLDERS_LIST;
pub const ABILITY_FS_LIST: &str = crate::daemon::ability::names::resources::CONTEXT_FS_LIST;
pub const ABILITY_FAVORITES_LIST: &str =
    crate::daemon::ability::names::resources::CONTEXT_FAVORITES_LIST;
pub const ABILITY_FAVORITES_ADD: &str =
    crate::daemon::ability::names::resources::CONTEXT_FAVORITES_ADD;
pub const ABILITY_FAVORITES_REMOVE: &str =
    crate::daemon::ability::names::resources::CONTEXT_FAVORITES_REMOVE;
pub const ABILITY_CAPTURES_LIST: &str =
    crate::daemon::ability::names::resources::CONTEXT_CAPTURES_LIST;
pub const ABILITY_CAPTURES_GET: &str =
    crate::daemon::ability::names::resources::CONTEXT_CAPTURES_GET;

/// Register every context ability. Called from
/// `daemon::ability::catalog::build_registry`.
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
        ABILITY_CLIPBOARD_REMOVE,
        OwnerKind::Device,
        std::sync::Arc::new(clipboard_remove_handler),
    );
    reg.register_rpc_with_owner(
        ABILITY_CATALOG,
        OwnerKind::Device,
        std::sync::Arc::new(catalog_handler),
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
    reg.register_rpc_with_owner(
        ABILITY_CAPTURES_LIST,
        OwnerKind::Device,
        std::sync::Arc::new(captures_list_handler),
    );
    reg.register_rpc_with_owner(
        ABILITY_CAPTURES_GET,
        OwnerKind::Device,
        std::sync::Arc::new(captures_get_handler),
    );
}

/// List persisted media artifacts, newest first. Optional `ability`
/// filter narrows to one folder; the response always carries the
/// distinct folder list so the Context page can render the per-device
/// directory level from one call.
fn captures_list_handler(args: Value) -> anyhow::Result<Value> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(100)
        .max(1) as usize;
    let ability = args.get("ability").and_then(Value::as_str);
    let entries: Vec<Value> = context_store::list_captures(ability, limit)
        .iter()
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();
    Ok(json!({
        "abilities": context_store::list_capture_abilities(),
        "entries": entries,
    }))
}

/// Fetch one artifact's bytes inline (base64). Mirrors the
/// clipboard.get shape so the frontend's lazy-loader pattern reuses.
fn captures_get_handler(args: Value) -> anyhow::Result<Value> {
    let id = require_str(&args, "id", "context.captures.get")?;
    let (path, entry) = context_store::capture_abs_path(&id)
        .ok_or_else(|| anyhow::anyhow!("context.captures.get: no capture {id}"))?;
    let bytes = std::fs::read(path)?;
    let mut out = serde_json::to_value(&entry)?;
    out["data_base64"] = json!(base64::engine::general_purpose::STANDARD.encode(bytes));
    Ok(out)
}

fn clipboard_list_handler(args: Value) -> anyhow::Result<Value> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(100)
        .max(1) as usize;
    let entries: Vec<Value> = context_store::list_clip_summaries(limit)
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

fn clipboard_remove_handler(args: Value) -> anyhow::Result<Value> {
    let id = require_str(&args, "id", "context.clipboard.remove")?;
    let removed = context_store::remove_clip(&id)?;
    Ok(serde_json::to_value(removed)?)
}

fn catalog_handler(args: Value) -> anyhow::Result<Value> {
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50)
        .clamp(1, 200) as usize;
    let mut items = Vec::new();

    for favorite in context_store::list_favorites() {
        items.push(json!({
            "id": format!("favorite:{}", favorite.id),
            "kind": "favorite",
            "label": favorite.label,
            "detail": favorite.kind,
            "reference": favorite.reference,
            "at": favorite.added_at,
            "source": {
                "favorite_id": favorite.id,
                "kind": favorite.kind,
            },
        }));
    }
    for clip in context_store::list_clip_summaries(limit) {
        items.push(json!({
            "id": format!("clipboard:{}", clip.entry.id),
            "kind": "clipboard",
            "label": if clip.entry.preview.trim().is_empty() {
                format!("{} clipboard entry", clip.entry.kind)
            } else {
                clip.entry.preview.clone()
            },
            "detail": clip.entry.kind,
            "reference": context_reference(
                "context.clipboard.get",
                json!({"id": clip.entry.id}),
            ),
            "at": clip.entry.timestamp,
            "source": {
                "device": clip.entry.device,
                "occurrence_count": clip.occurrence_count,
                "duplicate_count": clip.duplicate_count,
            },
        }));
    }
    for capture in context_store::list_captures(None, limit) {
        items.push(json!({
            "id": format!("capture:{}", capture.id),
            "kind": "capture",
            "label": if capture.preview.trim().is_empty() {
                capture.file.clone()
            } else {
                capture.preview.clone()
            },
            "detail": format!("{} · {}", capture.ability, capture.content_type),
            "reference": context_reference(
                "context.captures.get",
                json!({"id": capture.id}),
            ),
            "at": capture.timestamp,
            "source": {
                "device": capture.device,
                "ability": capture.ability,
                "file": capture.file,
                "byte_size": capture.byte_size,
            },
        }));
    }
    for folder in context_store::list_folders() {
        let exists = std::path::Path::new(&folder.path).is_dir();
        items.push(json!({
            "id": format!("folder:{}", folder.name),
            "kind": "folder",
            "label": folder.name,
            "detail": if exists {
                folder.path.clone()
            } else {
                format!("{} · missing", folder.path)
            },
            "reference": context_reference(
                "context.fs.list",
                json!({"folder": folder.name, "path": ""}),
            ),
            "at": folder.added_at,
            "source": {
                "path": folder.path,
                "exists": exists,
            },
        }));
    }

    items.sort_by(compare_catalog_items);
    items.truncate(limit);
    Ok(json!({ "items": items }))
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

fn context_reference(action: &str, args: Value) -> String {
    format!(
        "easynet-context://{}?args={}",
        action,
        urlencoding::encode(&args.to_string())
    )
}

fn compare_catalog_items(left: &Value, right: &Value) -> std::cmp::Ordering {
    let left_at = left.get("at").and_then(Value::as_str).unwrap_or("");
    let right_at = right.get("at").and_then(Value::as_str).unwrap_or("");
    right_at.cmp(left_at).then_with(|| {
        left.get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(right.get("id").and_then(Value::as_str).unwrap_or(""))
    })
}

// ── descriptions + schemas (wired in agents/mod.rs) ─────────────────

pub fn description_for(name: &str) -> Option<&'static str> {
    Some(match name {
        ABILITY_CLIPBOARD_LIST => {
            "List captured clipboard history as newest-first unique entries with duplicate counts and the tracking flag."
        }
        ABILITY_CLIPBOARD_GET => "Read one clipboard entry; image entries include base64 PNG data.",
        ABILITY_CLIPBOARD_TRACK => "Enable or disable clipboard history tracking on this device.",
        ABILITY_CLIPBOARD_REMOVE => "Delete one clipboard entry (and its stored image, if any).",
        ABILITY_CATALOG => {
            "List a unified newest-first context catalog for the Frontend composer picker."
        }
        ABILITY_FOLDERS_LIST => "List the project folders mapped via `easynet context add`.",
        ABILITY_FS_LIST => "Browse one directory level inside a mapped project folder.",
        ABILITY_FAVORITES_LIST => "List favorites (starred clips, files, folders).",
        ABILITY_FAVORITES_ADD => "Star a clipboard entry, file, or folder.",
        ABILITY_FAVORITES_REMOVE => "Remove a favorite by id.",
        ABILITY_CAPTURES_LIST => {
            "List media artifacts persisted by abilities (screenshots, photos, recordings), \
             newest first, with the distinct ability folder names."
        }
        ABILITY_CAPTURES_GET => {
            "Read one persisted media artifact inline (base64) with its content type."
        }
        _ => return None,
    })
}

pub fn input_schema_for(name: &str) -> Option<Value> {
    let schema = match name {
        ABILITY_CLIPBOARD_LIST => json!({
            "type": "object",
            "properties": {
                "limit": {"type": "integer", "description": "Max unique entries to return (default 100, cap 200)."}
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
        ABILITY_CLIPBOARD_REMOVE => json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Clip id from context.clipboard.list."}
            },
            "required": ["id"],
            "additionalProperties": false,
        }),
        ABILITY_CATALOG => json!({
            "type": "object",
            "properties": {
                "limit": {"type": "integer", "description": "Max unified context items to return (default 50, cap 200)."}
            },
            "additionalProperties": false,
        }),
        ABILITY_CAPTURES_LIST => json!({
            "type": "object",
            "properties": {
                "ability": {"type": "string", "description": "Filter to one producing ability (folder name, e.g. screen.snapshot)."},
                "limit": {"type": "integer", "description": "Max entries to return (default 100, cap 200)."}
            },
            "additionalProperties": false,
        }),
        ABILITY_CAPTURES_GET => json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "Capture id from context.captures.list."}
            },
            "required": ["id"],
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
pub const ALL: [&str; 12] = [
    ABILITY_CLIPBOARD_LIST,
    ABILITY_CLIPBOARD_GET,
    ABILITY_CLIPBOARD_TRACK,
    ABILITY_CLIPBOARD_REMOVE,
    ABILITY_CATALOG,
    ABILITY_FOLDERS_LIST,
    ABILITY_FS_LIST,
    ABILITY_FAVORITES_LIST,
    ABILITY_FAVORITES_ADD,
    ABILITY_FAVORITES_REMOVE,
    ABILITY_CAPTURES_LIST,
    ABILITY_CAPTURES_GET,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_toggle_then_list_reflects_state() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        clipboard_track_handler(json!({"enabled": true})).unwrap();
        let out = clipboard_list_handler(json!({})).unwrap();
        assert_eq!(out["tracking"], true);
        assert_eq!(out["entries"].as_array().unwrap().len(), 0);
        clipboard_track_handler(json!({"enabled": false})).unwrap();
        assert_eq!(
            clipboard_list_handler(json!({})).unwrap()["tracking"],
            false
        );
    }

    #[test]
    fn track_requires_enabled_bool() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        assert!(clipboard_track_handler(json!({})).is_err());
        assert!(clipboard_track_handler(json!({"enabled": "yes"})).is_err());
    }

    #[test]
    fn fs_list_requires_folder_and_descriptions_schemas_cover_all() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        assert!(fs_list_handler(json!({})).is_err());
        for name in ALL {
            assert!(description_for(name).is_some(), "{name} description");
            assert!(input_schema_for(name).is_some(), "{name} schema");
        }
    }

    #[test]
    fn clipboard_remove_deletes_entry_and_errors_on_unknown_id() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        let entry = context_store::ClipEntry {
            id: "clip-1".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            device: "easynet:///r/localhost/device/d1".into(),
            kind: "text".into(),
            text: Some("hello".into()),
            image_file: None,
            preview: "hello".into(),
        };
        context_store::append_clip(&entry).unwrap();
        assert_eq!(
            clipboard_list_handler(json!({})).unwrap()["entries"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        let removed = clipboard_remove_handler(json!({"id": "clip-1"})).unwrap();
        assert_eq!(removed["id"], "clip-1");
        assert_eq!(
            clipboard_list_handler(json!({})).unwrap()["entries"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
        assert!(clipboard_remove_handler(json!({"id": "clip-1"})).is_err());
        assert!(clipboard_remove_handler(json!({})).is_err());
    }

    #[test]
    fn clipboard_list_collapses_duplicates_and_marks_counts() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        for (id, text) in [
            ("clip-old", "same text"),
            ("clip-other", "other text"),
            ("clip-new", "same text"),
        ] {
            context_store::append_clip(&context_store::ClipEntry {
                id: id.into(),
                timestamp: "2026-01-01T00:00:00Z".into(),
                device: "easynet:///r/localhost/device/d1".into(),
                kind: "text".into(),
                text: Some(text.into()),
                image_file: None,
                preview: text.into(),
            })
            .unwrap();
        }

        let out = clipboard_list_handler(json!({"limit": 10})).unwrap();
        let entries = out["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["id"], "clip-new");
        assert_eq!(entries[0]["occurrence_count"], 2);
        assert_eq!(entries[0]["duplicate_count"], 1);
        assert_eq!(entries[1]["id"], "clip-other");
        assert_eq!(entries[1]["occurrence_count"], 1);
        assert_eq!(entries[1]["duplicate_count"], 0);
    }

    #[test]
    fn favorites_add_validates_and_round_trips() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
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

    #[test]
    fn catalog_merges_context_sources_for_frontend_picker() {
        let _g = crate::cli::commands::test_support::HomeGuard::new();
        context_store::append_clip(&context_store::ClipEntry {
            id: "clip-1".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            device: "easynet:///r/localhost/device/d1".into(),
            kind: "text".into(),
            text: Some("hello".into()),
            image_file: None,
            preview: "hello".into(),
        })
        .unwrap();
        let favorite = favorites_add_handler(
            json!({"kind": "clipboard", "label": "snippet", "reference": "clip-1"}),
        )
        .unwrap();

        let out = catalog_handler(json!({"limit": 10})).unwrap();
        let items = out["items"].as_array().unwrap();
        assert!(items.iter().any(|item| item["id"] == "clipboard:clip-1"));
        assert!(items
            .iter()
            .any(|item| item["id"] == format!("favorite:{}", favorite["id"].as_str().unwrap())));
        let clip = items
            .iter()
            .find(|item| item["id"] == "clipboard:clip-1")
            .unwrap();
        assert_eq!(clip["kind"], "clipboard");
        assert_eq!(clip["label"], "hello");
        assert!(clip["reference"]
            .as_str()
            .unwrap()
            .starts_with("easynet-context://context.clipboard.get?args="));
    }
}
