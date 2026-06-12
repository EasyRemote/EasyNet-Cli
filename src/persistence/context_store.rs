// EasyNet CLI — context store (clipboard history + folder mappings + favorites)
// ==============================================================================
//
// File: src/persistence/context_store.rs
// Description: On-disk state backing the Context surface (Frontend
//              "Context" page + `easynet context` CLI + the
//              `context.*` device abilities).
//
// Layout, all under `<state_dir>/context/`:
//
//   config.json      {"clipboard_tracking": bool}
//                    Read every tracker tick, so the CLI / ability can
//                    flip tracking without signalling the daemon.
//   clipboard.jsonl  One JSON object per captured clip, append-only
//                    (same durability reasoning as chat_sessions: an
//                    append can't lose prior entries to a partial
//                    rewrite). Newest entries are at the tail.
//   clips/<id>.png   Image payloads (screenshots etc.). The JSONL row
//                    carries the relative file name, not the bytes.
//   folders.json     [{"name","path","added_at"}] — user-mapped project
//                    folders (`easynet context add/remove`).
//   favorites.json   [{"id","kind","label","reference","added_at"}]
//
// Clipboard capture itself lives in `services::clipboard_tracker`;
// the `context.*` abilities in `runtime::agents::context_ability` are
// read/toggle surfaces over this store.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::persistence::config::{atomic_write_with_permissions, state_dir, WritePermissions};

/// Hard cap on entries returned by `list_clips` regardless of the
/// caller's `limit` — the JSONL is unbounded, responses must not be.
const LIST_CLIPS_MAX: usize = 200;

pub fn context_dir() -> PathBuf {
    state_dir().join("context")
}

fn config_path() -> PathBuf {
    context_dir().join("config.json")
}

fn clipboard_log_path() -> PathBuf {
    context_dir().join("clipboard.jsonl")
}

pub fn clips_dir() -> PathBuf {
    context_dir().join("clips")
}

fn folders_path() -> PathBuf {
    context_dir().join("folders.json")
}

fn favorites_path() -> PathBuf {
    context_dir().join("favorites.json")
}

// ── tracking config ─────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
struct ContextConfig {
    #[serde(default)]
    clipboard_tracking: bool,
}

fn load_config() -> ContextConfig {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Whether clipboard tracking is enabled. Read by the tracker every
/// tick (the file is tiny) so toggles take effect without IPC.
pub fn clipboard_tracking() -> bool {
    load_config().clipboard_tracking
}

pub fn set_clipboard_tracking(enabled: bool) -> anyhow::Result<()> {
    fs::create_dir_all(context_dir())?;
    let cfg = ContextConfig {
        clipboard_tracking: enabled,
    };
    let json = serde_json::to_string_pretty(&cfg)?;
    atomic_write_with_permissions(&config_path(), json.as_bytes(), WritePermissions::Default)?;
    Ok(())
}

// ── clipboard history ───────────────────────────────────────────────

/// One captured clipboard item. `image_file` is a file name inside
/// `clips_dir()` (not an absolute path) so the state dir can move.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipEntry {
    pub id: String,
    /// RFC3339 capture time.
    pub timestamp: String,
    /// Canonical device agent URA of the capturing device.
    pub device: String,
    /// "text" | "image"
    pub kind: String,
    /// Full text for text clips. Absent for images.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// File name under clips/ for image clips. Absent for text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_file: Option<String>,
    /// Short human preview (first line of text / "Screenshot WxH").
    pub preview: String,
}

pub fn append_clip(entry: &ClipEntry) -> anyhow::Result<()> {
    fs::create_dir_all(context_dir())?;
    let mut line = serde_json::to_string(entry)?;
    line.push('\n');
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(clipboard_log_path())?;
    f.write_all(line.as_bytes())?;
    Ok(())
}

/// Newest-first clip entries, capped at `min(limit, LIST_CLIPS_MAX)`.
pub fn list_clips(limit: usize) -> Vec<ClipEntry> {
    let cap = limit.clamp(1, LIST_CLIPS_MAX);
    let Ok(content) = fs::read_to_string(clipboard_log_path()) else {
        return Vec::new();
    };
    let mut entries: Vec<ClipEntry> = content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    entries.reverse();
    entries.truncate(cap);
    entries
}

/// Remove one clip by id: its JSONL line is dropped (every other line
/// is kept byte-for-byte) and an image clip's PNG is deleted from
/// `clips_dir()` best-effort. Returns the removed entry.
pub fn remove_clip(id: &str) -> anyhow::Result<ClipEntry> {
    let content = fs::read_to_string(clipboard_log_path()).unwrap_or_default();
    let mut removed: Option<ClipEntry> = None;
    let mut kept = String::with_capacity(content.len());
    for line in content.lines() {
        match serde_json::from_str::<ClipEntry>(line) {
            Ok(e) if removed.is_none() && e.id == id => removed = Some(e),
            _ => {
                kept.push_str(line);
                kept.push('\n');
            }
        }
    }
    let entry = removed.ok_or_else(|| anyhow::anyhow!("context clipboard: no clip {id}"))?;
    atomic_write_with_permissions(
        &clipboard_log_path(),
        kept.as_bytes(),
        WritePermissions::Default,
    )?;
    if let Some(file) = &entry.image_file {
        if !(file.contains('/') || file.contains('\\') || file.contains("..")) {
            let _ = fs::remove_file(clips_dir().join(file));
        }
    }
    Ok(entry)
}

/// Absolute path of a stored clip image, if the entry exists and is an
/// image. Resolves strictly inside `clips_dir()` — the id is ours, but
/// the lookup still refuses separators so a crafted id can't traverse.
pub fn clip_image_abs_path(id: &str) -> Option<PathBuf> {
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return None;
    }
    let entry = list_clips(LIST_CLIPS_MAX)
        .into_iter()
        .find(|e| e.id == id)?;
    let file = entry.image_file?;
    if file.contains('/') || file.contains('\\') || file.contains("..") {
        return None;
    }
    let p = clips_dir().join(file);
    p.is_file().then_some(p)
}

// ── folder mappings ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderMapping {
    /// Display name (defaults to the directory's file name).
    pub name: String,
    /// Absolute path on this device.
    pub path: String,
    pub added_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FoldersFile {
    #[serde(default)]
    folders: Vec<FolderMapping>,
}

pub fn list_folders() -> Vec<FolderMapping> {
    fs::read_to_string(folders_path())
        .ok()
        .and_then(|s| serde_json::from_str::<FoldersFile>(&s).ok())
        .unwrap_or_default()
        .folders
}

fn save_folders(folders: Vec<FolderMapping>) -> anyhow::Result<()> {
    fs::create_dir_all(context_dir())?;
    let json = serde_json::to_string_pretty(&FoldersFile { folders })?;
    atomic_write_with_permissions(&folders_path(), json.as_bytes(), WritePermissions::Default)?;
    Ok(())
}

/// Map a folder. The path must exist and be a directory; it is stored
/// canonicalized so the browse ability's containment check can't be
/// defeated by symlinked aliases of the mapping itself.
pub fn add_folder(path: &str, name: Option<&str>) -> anyhow::Result<FolderMapping> {
    let canon = fs::canonicalize(path)
        .map_err(|e| anyhow::anyhow!("context add: cannot resolve {path}: {e}"))?;
    if !canon.is_dir() {
        anyhow::bail!("context add: {path} is not a directory");
    }
    let canon_str = canon.to_string_lossy().to_string();
    let mut folders = list_folders();
    if folders.iter().any(|f| f.path == canon_str) {
        anyhow::bail!("context add: {canon_str} is already mapped");
    }
    let display = name
        .map(str::to_string)
        .or_else(|| canon.file_name().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| canon_str.clone());
    let mapping = FolderMapping {
        name: display,
        path: canon_str,
        added_at: chrono::Utc::now().to_rfc3339(),
    };
    folders.push(mapping.clone());
    save_folders(folders)?;
    Ok(mapping)
}

/// Remove by name or path. Returns the removed mapping.
pub fn remove_folder(key: &str) -> anyhow::Result<FolderMapping> {
    let mut folders = list_folders();
    let pos = folders
        .iter()
        .position(|f| f.name == key || f.path == key)
        .ok_or_else(|| anyhow::anyhow!("context remove: no mapped folder named {key}"))?;
    let removed = folders.remove(pos);
    save_folders(folders)?;
    Ok(removed)
}

/// List one directory level inside a mapped folder. `rel` is a path
/// relative to the mapping root ("" = the root itself). Containment:
/// the canonicalized target must stay under the canonicalized mapping
/// root, so `..` and symlinks out of the tree are refused.
pub fn list_folder_entries(folder_key: &str, rel: &str) -> anyhow::Result<Value> {
    let mapping = list_folders()
        .into_iter()
        .find(|f| f.name == folder_key || f.path == folder_key)
        .ok_or_else(|| anyhow::anyhow!("context.fs.list: unknown folder {folder_key}"))?;
    let root = fs::canonicalize(&mapping.path)
        .map_err(|e| anyhow::anyhow!("context.fs.list: mapping root vanished: {e}"))?;
    let target = if rel.is_empty() {
        root.clone()
    } else {
        fs::canonicalize(root.join(rel))
            .map_err(|e| anyhow::anyhow!("context.fs.list: cannot resolve {rel}: {e}"))?
    };
    if !target.starts_with(&root) {
        anyhow::bail!("context.fs.list: {rel} escapes the mapped folder");
    }
    let mut entries: Vec<Value> = Vec::new();
    for ent in fs::read_dir(&target)? {
        let Ok(ent) = ent else { continue };
        let meta = match ent.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let name = ent.file_name().to_string_lossy().to_string();
        // Skip dotfiles — Finder hides them too, and they're noise in
        // a context-browsing surface.
        if name.starts_with('.') {
            continue;
        }
        let modified = meta
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from)
            .map(|t| t.to_rfc3339())
            .unwrap_or_default();
        entries.push(json!({
            "name": name,
            "kind": if meta.is_dir() { "dir" } else { "file" },
            "size": if meta.is_dir() { Value::Null } else { json!(meta.len()) },
            "modified": modified,
        }));
    }
    // Directories first, then files, each alphabetical — Finder order.
    entries.sort_by(|a, b| {
        let ad = a["kind"] == "dir";
        let bd = b["kind"] == "dir";
        bd.cmp(&ad).then_with(|| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
        })
    });
    Ok(json!({
        "folder": mapping.name,
        "root": mapping.path,
        "path": rel,
        "entries": entries,
    }))
}

// ── favorites ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Favorite {
    pub id: String,
    /// "clipboard" | "file" | "folder"
    pub kind: String,
    /// Human label shown in the Favorites list.
    pub label: String,
    /// What it points at: clip id, or absolute/mapped path.
    pub reference: String,
    pub added_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FavoritesFile {
    #[serde(default)]
    favorites: Vec<Favorite>,
}

pub fn list_favorites() -> Vec<Favorite> {
    fs::read_to_string(favorites_path())
        .ok()
        .and_then(|s| serde_json::from_str::<FavoritesFile>(&s).ok())
        .unwrap_or_default()
        .favorites
}

fn save_favorites(favorites: Vec<Favorite>) -> anyhow::Result<()> {
    fs::create_dir_all(context_dir())?;
    let json = serde_json::to_string_pretty(&FavoritesFile { favorites })?;
    atomic_write_with_permissions(
        &favorites_path(),
        json.as_bytes(),
        WritePermissions::Default,
    )?;
    Ok(())
}

pub fn add_favorite(kind: &str, label: &str, reference: &str) -> anyhow::Result<Favorite> {
    let mut favorites = list_favorites();
    if favorites
        .iter()
        .any(|f| f.kind == kind && f.reference == reference)
    {
        anyhow::bail!("favorite already exists for {reference}");
    }
    let fav = Favorite {
        id: uuid::Uuid::new_v4().to_string(),
        kind: kind.to_string(),
        label: label.to_string(),
        reference: reference.to_string(),
        added_at: chrono::Utc::now().to_rfc3339(),
    };
    favorites.push(fav.clone());
    save_favorites(favorites)?;
    Ok(fav)
}

pub fn remove_favorite(id: &str) -> anyhow::Result<Favorite> {
    let mut favorites = list_favorites();
    let pos = favorites
        .iter()
        .position(|f| f.id == id)
        .ok_or_else(|| anyhow::anyhow!("no favorite with id {id}"))?;
    let removed = favorites.remove(pos);
    save_favorites(favorites)?;
    Ok(removed)
}

// ── ability capture artifacts ───────────────────────────────────────
//
// Media abilities (screen.snapshot / camera.snapshot / mic.subscribe)
// persist their products here so the Context page can browse them as
// `<device>/<ability>/<artifact>`. Same durability shape as the
// clipboard: an append-only `captures.jsonl` index plus payload files
// under `captures/<ability>/`. The ability name doubles as the folder
// name on disk and in the UI.

/// Hard cap on entries returned by `list_captures` — the JSONL is
/// unbounded, responses must not be.
const LIST_CAPTURES_MAX: usize = 200;

pub fn captures_dir() -> PathBuf {
    context_dir().join("captures")
}

fn captures_log_path() -> PathBuf {
    context_dir().join("captures.jsonl")
}

/// One persisted media artifact. `file` is a file name inside
/// `captures/<ability>/` (never a path) so the state dir can move.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureEntry {
    pub id: String,
    /// RFC3339 capture time.
    pub timestamp: String,
    /// Canonical device agent URA of the producing device.
    pub device: String,
    /// Producing ability — also the on-disk folder name
    /// ("screen.snapshot" | "camera.snapshot" | "mic.subscribe").
    pub ability: String,
    /// File name under captures/<ability>/.
    pub file: String,
    pub content_type: String,
    pub byte_size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Short human preview ("Screenshot 2940x1912", "Recording 12s").
    pub preview: String,
}

/// Ability names come from our own registries, but they become path
/// segments — refuse anything that isn't a bare `[a-z0-9._-]` token.
fn safe_path_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        && !segment.contains("..")
}

/// Persist one media artifact: payload to `captures/<ability>/`,
/// index row appended to `captures.jsonl`. Returns the entry.
pub struct CaptureRecord<'a> {
    pub device: &'a str,
    pub ability: &'a str,
    pub ext: &'a str,
    pub bytes: &'a [u8],
    pub content_type: &'a str,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub preview: String,
}

pub fn record_capture(record: CaptureRecord<'_>) -> anyhow::Result<CaptureEntry> {
    let CaptureRecord {
        device,
        ability,
        ext,
        bytes,
        content_type,
        width,
        height,
        duration_ms,
        preview,
    } = record;

    if !safe_path_segment(ability) {
        anyhow::bail!("record_capture: ability {ability:?} is not a safe folder name");
    }
    if !safe_path_segment(ext) {
        anyhow::bail!("record_capture: extension {ext:?} is not a safe file suffix");
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now();
    // Timestamp prefix keeps `ls` of the folder chronologically sorted
    // — the folder itself is a user-facing surface (Context page).
    let file = format!("{}-{}.{}", now.format("%Y%m%dT%H%M%S"), &id[..8], ext);
    let dir = captures_dir().join(ability);
    fs::create_dir_all(&dir)?;
    fs::write(dir.join(&file), bytes)?;
    let entry = CaptureEntry {
        id,
        timestamp: now.to_rfc3339(),
        device: device.to_string(),
        ability: ability.to_string(),
        file,
        content_type: content_type.to_string(),
        byte_size: bytes.len() as u64,
        width,
        height,
        duration_ms,
        preview,
    };
    let mut line = serde_json::to_string(&entry)?;
    line.push('\n');
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(captures_log_path())?;
    f.write_all(line.as_bytes())?;
    Ok(entry)
}

/// Newest-first capture entries, optionally filtered to one ability,
/// capped at `min(limit, LIST_CAPTURES_MAX)`.
pub fn list_captures(ability: Option<&str>, limit: usize) -> Vec<CaptureEntry> {
    let cap = limit.clamp(1, LIST_CAPTURES_MAX);
    let Ok(content) = fs::read_to_string(captures_log_path()) else {
        return Vec::new();
    };
    let mut entries: Vec<CaptureEntry> = content
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .filter(|e: &CaptureEntry| ability.is_none_or(|a| e.ability == a))
        .collect();
    entries.reverse();
    entries.truncate(cap);
    entries
}

/// Distinct ability folder names present in the captures index,
/// alphabetical. Drives the Context page's per-device folder list.
pub fn list_capture_abilities() -> Vec<String> {
    let Ok(content) = fs::read_to_string(captures_log_path()) else {
        return Vec::new();
    };
    let mut abilities: Vec<String> = content
        .lines()
        .filter_map(|l| serde_json::from_str::<CaptureEntry>(l).ok())
        .map(|e| e.ability)
        .collect();
    abilities.sort();
    abilities.dedup();
    abilities
}

/// Absolute path + entry for a stored capture. Same traversal posture
/// as `clip_image_abs_path`: ids are ours, but the lookup still
/// refuses separators so a crafted id can't escape.
pub fn capture_abs_path(id: &str) -> Option<(PathBuf, CaptureEntry)> {
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return None;
    }
    let entry = list_captures(None, LIST_CAPTURES_MAX)
        .into_iter()
        .find(|e| e.id == id)?;
    if !safe_path_segment(&entry.ability) || entry.file.contains('/') || entry.file.contains("..") {
        return None;
    }
    let p = captures_dir().join(&entry.ability).join(&entry.file);
    p.is_file().then_some((p, entry))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_tracking_round_trip() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        assert!(!clipboard_tracking(), "default is off");
        set_clipboard_tracking(true).unwrap();
        assert!(clipboard_tracking());
        set_clipboard_tracking(false).unwrap();
        assert!(!clipboard_tracking());
    }

    #[test]
    fn clips_append_and_list_newest_first() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        for i in 0..3 {
            append_clip(&ClipEntry {
                id: format!("c{i}"),
                timestamp: format!("2026-06-10T00:0{i}:00Z"),
                device: "easynet:///r/localhost/device/d1".into(),
                kind: "text".into(),
                text: Some(format!("clip {i}")),
                image_file: None,
                preview: format!("clip {i}"),
            })
            .unwrap();
        }
        let clips = list_clips(2);
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].id, "c2", "newest first");
        assert_eq!(clips[1].id, "c1");
    }

    #[test]
    fn folder_mapping_add_list_remove_and_containment() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/a.txt"), b"x").unwrap();
        std::fs::write(dir.path().join("top.txt"), b"y").unwrap();

        let mapping = add_folder(dir.path().to_str().unwrap(), Some("proj")).unwrap();
        assert_eq!(mapping.name, "proj");
        assert_eq!(list_folders().len(), 1);

        // root listing: dirs before files, dotfiles hidden
        let root = list_folder_entries("proj", "").unwrap();
        let names: Vec<&str> = root["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["sub", "top.txt"]);

        // descend
        let sub = list_folder_entries("proj", "sub").unwrap();
        assert_eq!(sub["entries"][0]["name"], "a.txt");

        // traversal refused
        assert!(list_folder_entries("proj", "../").is_err());

        remove_folder("proj").unwrap();
        assert!(list_folders().is_empty());
    }

    #[test]
    fn captures_record_list_get_round_trip() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let entry = record_capture(CaptureRecord {
            device: "easynet:///r/localhost/device/d1",
            ability: "screen.snapshot",
            ext: "jpg",
            bytes: b"\xff\xd8fakejpeg",
            content_type: "image/jpeg",
            width: Some(2940),
            height: Some(1912),
            duration_ms: None,
            preview: "Screenshot 2940x1912".into(),
        })
        .unwrap();
        record_capture(CaptureRecord {
            device: "easynet:///r/localhost/device/d1",
            ability: "mic.subscribe",
            ext: "wav",
            bytes: b"RIFFfakewav",
            content_type: "audio/wav",
            width: None,
            height: None,
            duration_ms: Some(1500),
            preview: "Recording 1.5s".into(),
        })
        .unwrap();

        // newest-first, ability filter works
        let all = list_captures(None, 10);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].ability, "mic.subscribe", "newest first");
        let screens = list_captures(Some("screen.snapshot"), 10);
        assert_eq!(screens.len(), 1);
        assert_eq!(screens[0].id, entry.id);

        // distinct folder names, alphabetical
        assert_eq!(
            list_capture_abilities(),
            vec!["mic.subscribe".to_string(), "screen.snapshot".to_string()]
        );

        // payload resolvable, traversal refused
        let (path, got) = capture_abs_path(&entry.id).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"\xff\xd8fakejpeg");
        assert_eq!(got.content_type, "image/jpeg");
        assert!(capture_abs_path("../evil").is_none());

        // unsafe ability folder refused
        assert!(record_capture(CaptureRecord {
            device: "d",
            ability: "../escape",
            ext: "jpg",
            bytes: b"x",
            content_type: "image/jpeg",
            width: None,
            height: None,
            duration_ms: None,
            preview: "p".into(),
        })
        .is_err());
    }

    #[test]
    fn favorites_round_trip() {
        let _g = crate::facade::cli::test_support::HomeGuard::new();
        let f = add_favorite("clipboard", "snippet", "c1").unwrap();
        assert_eq!(list_favorites().len(), 1);
        // duplicate refused
        assert!(add_favorite("clipboard", "again", "c1").is_err());
        remove_favorite(&f.id).unwrap();
        assert!(list_favorites().is_empty());
    }
}
