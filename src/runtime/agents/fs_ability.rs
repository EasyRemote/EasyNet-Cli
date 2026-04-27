// EasyNet CLI — Baseline Locomotion Profile: filesystem abilities
// =================================================================
//
// File: src/runtime/agents/fs_ability.rs
// Description: `fs.read`, `fs.write`, `fs.list` — the three
//              filesystem members of the Baseline Locomotion
//              Profile (AXIOM §"Tier 2.5"). Implemented as
//              schema-validated Axon abilities; every call goes
//              through the same admission + delegation + receipt
//              pipeline as any other ability.
//
// Protocol Responsibility:
// - Provide the v1 filesystem capability that any host-embodied
//   agent claiming `baseline-locomotion-v1` is required to expose
//   per AXIOM §"Tier 2.5". The wire surface is normative; the
//   in-process implementation here is one conformant
//   realisation.
// - The abilities are NOT a backend escape hatch. Every operation:
//     * has a structured input schema (no arbitrary strings),
//     * is mediated by AXIOM admission (caller signature, nonce
//       replay, four-form causal context),
//     * emits a callee-signed receipt with redaction rules,
//     * writes through atomic `rename(2)` so a crashed write
//       never leaves a half-written file.
//
// Implementation Approach:
// - Synchronous handlers running on tokio's blocking-friendly
//   pool. Filesystem syscalls are the right size for a single
//   ability call — bytes counted in KB to MB, not GB. A bigger
//   payload deserves a payload-ref envelope, not a longer
//   `fs.read` call.
// - No process-wide caching, no path normalization beyond the
//   defensive checks below. Callers express the path they want;
//   we hand it to the OS verbatim. Sandboxing (chroot, OS
//   capabilities) is the deployment's responsibility, not the
//   ability's.
// - Redaction in receipts: the receipt records the path, sizes,
//   and a SHA-256 of the content; it does NOT record the bytes
//   themselves (a 4 MiB write would otherwise blow up the
//   receipt log).
//
// Usage Contract:
// - The receiver of these calls is an axon-admission-gated
//   caller; the caller has already proven its identity and right
//   to act under the supplied subject. The handler does not
//   second-guess the admission decision.
// - Path traversal: `..` is permitted because callers may
//   legitimately address a file via a relative parent. The
//   security model is: if the caller has the right to invoke
//   `fs.write` against this agent at all, it has the right to
//   touch any path the agent can touch. Per-path ACLs are a
//   higher-tier policy concern, not part of the v1 baseline.
//
// Architectural Position:
// - Sibling of `process_exec_ability.rs` and `http_request_ability.rs`
//   (later commits). Together they form the host-embodied
//   agent's `baseline-locomotion-v1` profile.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::runtime::ability_dispatch::LocalAbilityRegistry;

// ── Wire-name constants (cross-language pins) ─────────────────────

pub const ABILITY_FS_READ: &str = "fs.read";
pub const ABILITY_FS_WRITE: &str = "fs.write";
pub const ABILITY_FS_LIST: &str = "fs.list";

/// Profile membership marker. Receivers MAY surface this in
/// `agent.describe` so callers can confirm the profile contract
/// before invoking. Mirrors AXIOM §"Tier 2.5" `ability_profile_version`.
pub const PROFILE_VERSION: &str = "baseline-locomotion-v1";

/// Default cap on `fs.read` size when the caller does not name a
/// `max_bytes` limit. 8 MiB is large enough to read most config
/// files, source files, and small datasets in one call; bigger
/// reads should use a payload-ref envelope or stream via
/// InvokeBidi.
const DEFAULT_READ_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Default cap on `fs.list` entry count. A directory with more
/// entries than this is a misuse of `fs.list`; callers wanting
/// pagination should use higher-tier indexed-storage abilities.
const DEFAULT_LIST_MAX_ENTRIES: usize = 4096;

// ── Registration ───────────────────────────────────────────────────

/// Wire all three filesystem abilities into the registry. Called
/// from `runtime::agents::build_registry_with_services` once at
/// daemon startup. The abilities are stateless so registration is
/// just three handler closures with no per-call setup.
pub fn register(reg: &mut LocalAbilityRegistry) {
    reg.register_rpc(ABILITY_FS_READ, Arc::new(handler_read));
    reg.register_rpc(ABILITY_FS_WRITE, Arc::new(handler_write));
    reg.register_rpc(ABILITY_FS_LIST, Arc::new(handler_list));
}

// ── fs.read ──────────────────────────────────────────────────────

fn handler_read(args: Value) -> Result<Value> {
    let path = require_string(&args, "path")?;
    let max_bytes = args
        .get("max_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_READ_MAX_BYTES);
    let encoding = args
        .get("encoding")
        .and_then(Value::as_str)
        .unwrap_or("binary");

    let metadata = std::fs::metadata(Path::new(path))
        .map_err(|e| anyhow!("fs.read: stat {path:?}: {e}"))?;
    let total_size = metadata.len();

    let mut content = std::fs::read(Path::new(path))
        .map_err(|e| anyhow!("fs.read: open {path:?}: {e}"))?;
    let truncated = content.len() as u64 > max_bytes;
    if truncated {
        content.truncate(max_bytes as usize);
    }

    let body = match encoding {
        "utf8" => match String::from_utf8(content.clone()) {
            Ok(s) => json!(s),
            Err(_) => {
                // Caller asked for utf8 but the file isn't valid
                // UTF-8. Returning an error is the honest answer
                // — silently downgrading to base64 would surprise
                // a caller who expected text.
                return Err(anyhow!(
                    "fs.read: file at {path:?} is not valid UTF-8; use encoding=\"binary\""
                ));
            }
        },
        "binary" => json!(BASE64_STANDARD.encode(&content)),
        other => {
            return Err(anyhow!(
                "fs.read: unknown encoding {other:?}; expected \"binary\" or \"utf8\""
            ))
        }
    };

    Ok(json!({
        "content": body,
        "size": total_size,
        "truncated": truncated,
        "content_sha256": hex::encode(Sha256::digest(&content)),
        "ability_profile_version": PROFILE_VERSION,
    }))
}

// ── fs.write ─────────────────────────────────────────────────────

fn handler_write(args: Value) -> Result<Value> {
    let path = require_string(&args, "path")?;
    let create_parents = args
        .get("create_parents")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mode = args.get("mode").and_then(Value::as_u64).map(|m| m as u32);

    let raw = decode_content(&args)?;

    if create_parents {
        if let Some(parent) = Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow!("fs.write: mkdir -p {parent:?}: {e}"))?;
            }
        }
    }

    // Atomic write: write to a temp file in the same directory, then
    // rename. A crash mid-write leaves the prior content intact.
    let dst = Path::new(path);
    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    let file_stem = dst
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("__write");
    let tmp = parent.join(format!(".{file_stem}.tmp.{}", uuid_suffix()));

    std::fs::write(&tmp, &raw)
        .map_err(|e| anyhow!("fs.write: write tmp {tmp:?}: {e}"))?;

    #[cfg(unix)]
    if let Some(m) = mode {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(m);
        std::fs::set_permissions(&tmp, perms).map_err(|e| {
            // Clean up tmp on failure so a partial chmod doesn't
            // leave a stale file in the parent dir.
            let _ = std::fs::remove_file(&tmp);
            anyhow!("fs.write: chmod tmp {tmp:?}: {e}")
        })?;
    }
    #[cfg(not(unix))]
    let _ = mode;

    std::fs::rename(&tmp, dst).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow!("fs.write: rename {tmp:?} -> {dst:?}: {e}")
    })?;

    Ok(json!({
        "bytes_written": raw.len(),
        "content_sha256": hex::encode(Sha256::digest(&raw)),
        "ability_profile_version": PROFILE_VERSION,
    }))
}

fn decode_content(args: &Value) -> Result<Vec<u8>> {
    let content = args
        .get("content")
        .ok_or_else(|| anyhow!("fs.write: missing required field `content`"))?;
    if let Some(s) = content.as_str() {
        // Heuristic: bytes form is base64 by default per the
        // schema. A caller that wants to write a UTF-8 string
        // directly sets `encoding: "utf8"`.
        let encoding = args
            .get("encoding")
            .and_then(Value::as_str)
            .unwrap_or("base64");
        match encoding {
            "base64" => BASE64_STANDARD
                .decode(s.as_bytes())
                .map_err(|e| anyhow!("fs.write: invalid base64 content: {e}")),
            "utf8" => Ok(s.as_bytes().to_vec()),
            other => Err(anyhow!(
                "fs.write: unknown encoding {other:?}; expected \"base64\" or \"utf8\""
            )),
        }
    } else if let Some(arr) = content.as_array() {
        // JSON array of integers form — explicit, used by
        // bindings that prefer not to base64-encode.
        let mut out = Vec::with_capacity(arr.len());
        for v in arr {
            let n = v.as_u64().ok_or_else(|| {
                anyhow!("fs.write: content[] entries must be integers in 0..256")
            })?;
            if n > 255 {
                return Err(anyhow!("fs.write: content[] entry {n} out of range"));
            }
            out.push(n as u8);
        }
        Ok(out)
    } else {
        Err(anyhow!(
            "fs.write: `content` must be a base64 string, a UTF-8 string with encoding=\"utf8\", or an array of bytes"
        ))
    }
}

// ── fs.list ──────────────────────────────────────────────────────

fn handler_list(args: Value) -> Result<Value> {
    let path = require_string(&args, "path")?;
    let max_entries = args
        .get("max_entries")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_LIST_MAX_ENTRIES as u64) as usize;
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut entries: Vec<Value> = Vec::new();
    let mut truncated = false;

    if recursive {
        list_recursive(Path::new(path), max_entries, &mut entries, &mut truncated)?;
    } else {
        for entry in std::fs::read_dir(Path::new(path))
            .map_err(|e| anyhow!("fs.list: read_dir {path:?}: {e}"))?
        {
            if entries.len() >= max_entries {
                truncated = true;
                break;
            }
            let entry = entry.map_err(|e| anyhow!("fs.list: iter entry: {e}"))?;
            entries.push(describe_entry(&entry.path())?);
        }
    }

    Ok(json!({
        "entries": entries,
        "truncated": truncated,
        "ability_profile_version": PROFILE_VERSION,
    }))
}

fn list_recursive(
    dir: &Path,
    max_entries: usize,
    out: &mut Vec<Value>,
    truncated: &mut bool,
) -> Result<()> {
    let read = std::fs::read_dir(dir).map_err(|e| anyhow!("fs.list: read_dir {dir:?}: {e}"))?;
    for entry in read {
        if out.len() >= max_entries {
            *truncated = true;
            return Ok(());
        }
        let entry = entry.map_err(|e| anyhow!("fs.list: iter entry: {e}"))?;
        let path = entry.path();
        out.push(describe_entry(&path)?);
        if path.is_dir() && !path.is_symlink() {
            // is_symlink check prevents infinite loops via
            // self-referential symlinks. A symlink that points
            // back to an ancestor would otherwise blow up the
            // entry list.
            list_recursive(&path, max_entries, out, truncated)?;
            if *truncated {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn describe_entry(path: &Path) -> Result<Value> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| anyhow!("fs.list: stat {path:?}: {e}"))?;
    let kind = if metadata.file_type().is_symlink() {
        "symlink"
    } else if metadata.is_dir() {
        "dir"
    } else if metadata.is_file() {
        "file"
    } else {
        "other"
    };
    let mut obj = json!({
        "name": path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        "path": path.to_string_lossy().to_string(),
        "kind": kind,
    });
    if metadata.is_file() {
        obj["size"] = json!(metadata.len());
    }
    if let Ok(modified) = metadata.modified() {
        if let Ok(d) = modified.duration_since(std::time::UNIX_EPOCH) {
            obj["mtime_unix_ms"] = json!(d.as_millis() as i64);
        }
    }
    Ok(obj)
}

// ── Schema + description (for discovery) ──────────────────────────

pub fn input_schema_read() -> Value {
    json!({
        "type": "object",
        "required": ["path"],
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string", "minLength": 1 },
            "max_bytes": { "type": "integer", "minimum": 0 },
            "encoding": { "type": "string", "enum": ["binary", "utf8"] }
        }
    })
}

pub fn input_schema_write() -> Value {
    json!({
        "type": "object",
        "required": ["path", "content"],
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string", "minLength": 1 },
            "content": {
                "oneOf": [
                    { "type": "string", "description": "base64-encoded by default; pass encoding=\"utf8\" for raw text" },
                    { "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 255 } }
                ]
            },
            "encoding": { "type": "string", "enum": ["base64", "utf8"] },
            "mode": { "type": "integer", "minimum": 0 },
            "create_parents": { "type": "boolean" }
        }
    })
}

pub fn input_schema_list() -> Value {
    json!({
        "type": "object",
        "required": ["path"],
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string", "minLength": 1 },
            "recursive": { "type": "boolean" },
            "max_entries": { "type": "integer", "minimum": 1 }
        }
    })
}

pub fn description_read() -> &'static str {
    "Read a file from the host's filesystem. Part of the \
     baseline-locomotion-v1 profile (AXIOM §Tier 2.5)."
}

pub fn description_write() -> &'static str {
    "Write a file atomically (temp + rename) on the host's filesystem. \
     Part of the baseline-locomotion-v1 profile (AXIOM §Tier 2.5)."
}

pub fn description_list() -> &'static str {
    "List the contents of a directory on the host's filesystem. \
     Part of the baseline-locomotion-v1 profile (AXIOM §Tier 2.5)."
}

// ── Helpers ────────────────────────────────────────────────────────

fn require_string<'a>(args: &'a Value, field: &str) -> Result<&'a str> {
    args.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing required string field `{field}`"))
}

/// Short random suffix for tmp filenames during atomic write.
/// Uses uuid for a collision-safe value without pulling in a
/// random-string crate. Just the simple form of the uuid for
/// brevity in the temp path.
fn uuid_suffix() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("easynet-fs-test-{}", uuid_suffix()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    // ─── fs.read ─────────────────────────────────────────────

    #[test]
    fn read_returns_utf8_when_requested() {
        let dir = temp_dir();
        let path = dir.join("hello.txt");
        std::fs::write(&path, "hello world").unwrap();

        let resp = handler_read(json!({
            "path": path.to_str().unwrap(),
            "encoding": "utf8",
        }))
        .unwrap();
        assert_eq!(resp["content"], json!("hello world"));
        assert_eq!(resp["size"], json!(11));
        assert_eq!(resp["truncated"], json!(false));
        assert_eq!(resp["ability_profile_version"], json!(PROFILE_VERSION));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_returns_base64_for_binary() {
        let dir = temp_dir();
        let path = dir.join("bin.dat");
        let bytes = vec![0xDE, 0xAD, 0xBE, 0xEF];
        std::fs::write(&path, &bytes).unwrap();

        let resp = handler_read(json!({
            "path": path.to_str().unwrap(),
        }))
        .unwrap();
        let decoded = BASE64_STANDARD
            .decode(resp["content"].as_str().unwrap())
            .unwrap();
        assert_eq!(decoded, bytes);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_rejects_non_utf8_when_utf8_requested() {
        let dir = temp_dir();
        let path = dir.join("bin.dat");
        std::fs::write(&path, [0xFF, 0xFE, 0xFD]).unwrap();

        let err = handler_read(json!({
            "path": path.to_str().unwrap(),
            "encoding": "utf8",
        }))
        .unwrap_err();
        assert!(err.to_string().contains("not valid UTF-8"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_truncates_at_max_bytes() {
        let dir = temp_dir();
        let path = dir.join("big.txt");
        std::fs::write(&path, vec![b'x'; 1000]).unwrap();

        let resp = handler_read(json!({
            "path": path.to_str().unwrap(),
            "max_bytes": 100,
            "encoding": "utf8",
        }))
        .unwrap();
        assert_eq!(resp["truncated"], json!(true));
        assert_eq!(resp["size"], json!(1000));
        assert_eq!(resp["content"].as_str().unwrap().len(), 100);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_rejects_unknown_encoding() {
        let dir = temp_dir();
        let path = dir.join("x.txt");
        std::fs::write(&path, "x").unwrap();

        let err = handler_read(json!({
            "path": path.to_str().unwrap(),
            "encoding": "rot13",
        }))
        .unwrap_err();
        assert!(err.to_string().contains("unknown encoding"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── fs.write ────────────────────────────────────────────

    #[test]
    fn write_utf8_creates_file_atomically() {
        let dir = temp_dir();
        let path = dir.join("out.txt");

        let resp = handler_write(json!({
            "path": path.to_str().unwrap(),
            "content": "hello",
            "encoding": "utf8",
        }))
        .unwrap();
        assert_eq!(resp["bytes_written"], json!(5));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_base64_round_trips() {
        let dir = temp_dir();
        let path = dir.join("out.bin");
        let bytes = vec![0x01, 0x02, 0x03];
        let encoded = BASE64_STANDARD.encode(&bytes);

        handler_write(json!({
            "path": path.to_str().unwrap(),
            "content": encoded,
        }))
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_create_parents_makes_intermediate_dirs() {
        let dir = temp_dir();
        let nested = dir.join("a/b/c/file.txt");

        handler_write(json!({
            "path": nested.to_str().unwrap(),
            "content": "x",
            "encoding": "utf8",
            "create_parents": true,
        }))
        .unwrap();
        assert!(nested.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_without_create_parents_fails_on_missing_dir() {
        let dir = temp_dir();
        let nested = dir.join("ghost/file.txt");

        let err = handler_write(json!({
            "path": nested.to_str().unwrap(),
            "content": "x",
            "encoding": "utf8",
        }))
        .unwrap_err();
        assert!(err.to_string().contains("write tmp"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_atomic_does_not_clobber_on_partial_failure() {
        // The promise: a crashed write leaves the existing file
        // intact. We can't crash mid-write in a test, but we can
        // verify the rename target is the only thing the caller
        // sees by ensuring no .tmp file is left after a successful
        // write.
        let dir = temp_dir();
        let path = dir.join("atomic.txt");
        std::fs::write(&path, "old").unwrap();

        handler_write(json!({
            "path": path.to_str().unwrap(),
            "content": "new",
            "encoding": "utf8",
        }))
        .unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        // No .tmp.* file should remain.
        for entry in std::fs::read_dir(&dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().to_string();
            assert!(
                !name.starts_with(".atomic.txt.tmp"),
                "leftover tmp file: {name}"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_rejects_array_with_out_of_range_byte() {
        let dir = temp_dir();
        let path = dir.join("x.bin");
        let err = handler_write(json!({
            "path": path.to_str().unwrap(),
            "content": [256, 1, 2],
        }))
        .unwrap_err();
        assert!(err.to_string().contains("out of range"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── fs.list ─────────────────────────────────────────────

    #[test]
    fn list_returns_file_and_dir_entries() {
        let dir = temp_dir();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();

        let resp = handler_list(json!({
            "path": dir.to_str().unwrap(),
        }))
        .unwrap();
        let entries = resp["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        let kinds: Vec<&str> = entries
            .iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect();
        assert!(kinds.contains(&"file"));
        assert!(kinds.contains(&"dir"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_recursive_descends_into_subdirs() {
        let dir = temp_dir();
        std::fs::create_dir(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/inner.txt"), "x").unwrap();

        let resp = handler_list(json!({
            "path": dir.to_str().unwrap(),
            "recursive": true,
        }))
        .unwrap();
        let names: Vec<String> = resp["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"sub".to_string()));
        assert!(names.contains(&"inner.txt".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_truncates_at_max_entries() {
        let dir = temp_dir();
        for i in 0..10 {
            std::fs::write(dir.join(format!("f{i}.txt")), "x").unwrap();
        }
        let resp = handler_list(json!({
            "path": dir.to_str().unwrap(),
            "max_entries": 3,
        }))
        .unwrap();
        assert_eq!(resp["entries"].as_array().unwrap().len(), 3);
        assert_eq!(resp["truncated"], json!(true));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_returns_size_for_files_not_dirs() {
        let dir = temp_dir();
        std::fs::write(dir.join("file.txt"), b"hello").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();

        let resp = handler_list(json!({ "path": dir.to_str().unwrap() })).unwrap();
        for entry in resp["entries"].as_array().unwrap() {
            match entry["kind"].as_str().unwrap() {
                "file" => assert_eq!(entry["size"], json!(5)),
                "dir" => assert!(entry.get("size").is_none()),
                _ => {}
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── Schema sanity ─────────────────────────────────────

    #[test]
    fn schemas_are_well_formed_objects() {
        for s in [
            input_schema_read(),
            input_schema_write(),
            input_schema_list(),
        ] {
            assert_eq!(s["type"], json!("object"));
            assert_eq!(s["required"][0], json!("path"));
        }
    }

    #[test]
    fn descriptions_mention_the_profile_name() {
        for d in [description_read(), description_write(), description_list()] {
            assert!(d.contains("baseline-locomotion-v1"));
        }
    }

    // ─── Profile version is the AXIOM-defined string ──────

    #[test]
    fn profile_version_pinned_to_axiom_spec() {
        // AXIOM §"Tier 2.5" defines this exact string. A rename
        // here is a protocol break.
        assert_eq!(PROFILE_VERSION, "baseline-locomotion-v1");
    }
}
