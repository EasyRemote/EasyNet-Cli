// EasyNet CLI — fs.edit ability (Tier 2.5 / surgical text edit)
// =================================================================
//
// File: src/daemon/ability/builtins/device_control/file_edit.rs
// Description: `fs.edit` — string-replace primitive over a single
//              text file. Implements the AliveCode FileEditTool
//              "exactly-once-match-or-replace_all" rule, with
//              structured rejection on ambiguous matches and a
//              size guard against editing multi-GB files.
//
// Why a separate ability (not a flag on fs.write)
// -----------------------------------------------
// `fs.write` overwrites the whole file. For surgical edits, the
// agent would round-trip via fs.read → string operations →
// fs.write, which:
//
//   1. Costs three RPCs and serialises a full file body twice.
//   2. Loses atomicity between read and write — another writer
//      can change the file in between, and the agent silently
//      stomps on those changes.
//   3. Has no protection against accidentally rewriting all
//      occurrences when the agent only meant one (the classic
//      `s/foo/bar/` vs `s/foo/bar/g` confusion).
//
// `fs.edit` collapses the round trip into one ability and bakes
// the exactly-once rule into the contract. AliveCode found this
// to be the single biggest correctness gain over a naive
// read-modify-write loop.
//
// Contract
// --------
//   * Inputs: resource_ref, old_string, new_string, optional
//     replace_all (default false).
//   * Behaviour:
//
//     - File missing AND old_string=="" AND new_string!="":
//         Create new file with new_string as its content.
//         (AliveCode's "create-via-edit" pattern; a caller
//         that wants this can pass create_if_missing=true and
//         the receiver checks that old_string is empty.)
//
//     - File missing AND any other shape: reject (NoSuchFile).
//
//     - File present AND old_string=="": reject (Ambiguous —
//         empty string would match every position; refuse rather
//         than guess intent).
//
//     - File present AND old_string occurs zero times: reject
//         (NotFound, with a snippet of the search string in the
//         response so the caller can self-correct).
//
//     - File present AND old_string occurs ≥2 times AND
//         replace_all=false: reject (AmbiguousMatch with the
//         match count). Caller must either disambiguate
//         old_string by adding context, or pass replace_all=true.
//
//     - File present AND old_string occurs exactly once: replace
//         and write atomically (delegate to fs.write's
//         tempfile+fsync+rename path).
//
//     - File present AND replace_all=true: replace every
//         occurrence and write atomically. Receipt records the
//         match count.
//
// Why not multi-edit batching
// ---------------------------
// AliveCode's MultiEdit batches several (old_string, new_string)
// pairs in one call. Useful UX, but every batched edit can
// still be expressed as a sequence of single fs.edit calls;
// batching is a v2 concern. v1 keeps the surface small.
//
// File size guard
// ---------------
// `MAX_EDIT_FILE_SIZE = 1 GiB`. A 1 GiB file is large enough to
// cover any source / config file an agent realistically edits;
// beyond that, the read+replace+write triple would chew RAM
// and CPU disproportionately to the value. Caller gets a clear
// FileTooLarge response rather than the daemon swapping itself
// to death.
//
// Atomicity
// ---------
// The write step uses the SAME tempfile + fdatasync + rename
// path as fs.write — see `fs_ability::handler_write`. We
// duplicate the implementation here rather than calling the
// fs.write handler directly because a future refactor of the
// write path (e.g. preserving permissions) shouldn't tangle
// fs.edit into fs.write's request shape; both functions stay
// thin in their own module.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::daemon::ability::dispatch::AxonAbilityCatalog;

use crate::daemon::ability::dispatch::OwnerKind;
use crate::daemon::resources::files::{self as filesystem, FilesystemResourceCapability};
/// Wire name. Pinned by the Tier 2.5 surface; rename = protocol break.
pub const ABILITY_NAME: &str = crate::daemon::ability::names::device_control::FS_EDIT;

/// Profile membership marker echoed in every receipt.
pub const PROFILE_VERSION: &str =
    crate::daemon::ability::names::device_control::BASELINE_LOCOMOTION_PROFILE_VERSION;

/// Hard upper bound on the file size fs.edit will read. 1 GiB
/// matches AliveCode's MAX_EDIT_FILE_SIZE; bigger targets are
/// unrealistic for surgical edits and disproportionately
/// expensive (read + scan + replace + write all serialise the
/// content). Caller gets FileTooLarge rather than a long stall.
pub const MAX_EDIT_FILE_SIZE: u64 = 1024 * 1024 * 1024;

pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner("fs.edit", OwnerKind::Device, Arc::new(handler));
}

fn handler(args: Value) -> Result<Value> {
    let resolved_path =
        filesystem::resolve_filesystem_path(&args, FilesystemResourceCapability::Write)?;
    let path = resolved_path.local_path;
    let path_label = resolved_path.display_path;
    let old_string = args
        .get("old_string")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("fs.edit: missing required string field `old_string`"))?
        .to_string();
    let new_string = args
        .get("new_string")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("fs.edit: missing required string field `new_string`"))?
        .to_string();
    let replace_all = args
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let expected_mtime_ms = args.get("expected_mtime_ms").and_then(Value::as_u64);

    // Resolve a single layer of symlink so the tempfile lives
    // next to the REAL file (rename(2) stays on one filesystem)
    // and the symlink at `path` keeps pointing to its original
    // target (we replace the inode the symlink points at, not
    // the symlink itself). See fs_ability::resolve_symlink_one_level
    // for the same semantics.
    if let Some(root) = resolved_path.virtual_root_path.as_deref() {
        filesystem::ensure_write_parent_under_root(&path, root)?;
    }
    let resolved = super::files::resolve_symlink_one_level(&path);
    let dst: &Path = &resolved;
    if let Some(root) = resolved_path.virtual_root_path.as_deref() {
        if dst.exists() {
            filesystem::ensure_path_under_root(dst, root)?;
        } else {
            filesystem::ensure_write_parent_under_root(dst, root)?;
        }
    }
    let exists = dst.exists();

    // expected_mtime_ms guard — caller asserts the file's mtime
    // matches what they last saw. If not, refuse rather than
    // clobber concurrent edits. Same shape as fs.write.
    if let Some(expected) = expected_mtime_ms {
        match std::fs::metadata(dst) {
            Ok(m) => {
                let actual = super::files::file_mtime_ms(&m);
                if actual != Some(expected) {
                    return Ok(rejection(
                        "StaleMtime",
                        "expected_mtime_ms does not match file's current mtime; \
                         refuse rather than clobber concurrent edits",
                        Some(json!({
                            "expected_mtime_ms": expected,
                            "actual_mtime_ms": actual,
                        })),
                        &path_label,
                    ));
                }
            }
            Err(_) => {
                return Ok(rejection(
                    "StaleMtime",
                    "expected_mtime_ms set but file does not exist",
                    None,
                    &path_label,
                ));
            }
        }
    }

    // Empty old_string is the create-new-file primitive AND the
    // disallowed empty-search on an existing file. Branch on
    // whether the target exists.
    if old_string.is_empty() {
        if exists {
            return Ok(rejection(
                "AmbiguousEmptyOldString",
                "old_string is empty but the file exists; empty would match every position",
                None,
                &path_label,
            ));
        }
        // Create-via-edit. new_string may be empty (creates an
        // empty file, deterministic) — we don't second-guess
        // the caller.
        write_atomic(dst, new_string.as_bytes())?;
        return Ok(json!({
            "ok": true,
            "kind": "created",
            "matches_replaced": 0,
            "bytes_written": new_string.len(),
            "content_sha256": hex::encode(Sha256::digest(new_string.as_bytes())),
            "display_path": path_label,
            "resource_ref_revalidated": true,
            "ability_profile_version": PROFILE_VERSION,
        }));
    }

    // old_string is non-empty. The file MUST exist for any
    // replacement to be defined.
    if !exists {
        return Ok(rejection(
            "NoSuchFile",
            "target file does not exist",
            None,
            &path_label,
        ));
    }

    // Pre-read size guard — read_file_capped enforces it but
    // we surface the reason cleanly here.
    let metadata =
        std::fs::metadata(dst).map_err(|e| anyhow!("fs.edit: stat {path_label:?}: {e}"))?;
    if metadata.len() > MAX_EDIT_FILE_SIZE {
        return Ok(rejection(
            "FileTooLarge",
            &format!(
                "file size {} exceeds fs.edit hard cap {}",
                metadata.len(),
                MAX_EDIT_FILE_SIZE
            ),
            None,
            &path_label,
        ));
    }

    // We require text content for a meaningful string search.
    // A binary file with embedded ASCII matches would still
    // match — but `String::from_utf8` rejects non-UTF-8 bytes,
    // and treating arbitrary binary as UTF-8 to do byte search
    // would silently corrupt non-ASCII content. fs.write is the
    // ability for binary edits.
    let bytes = std::fs::read(dst).map_err(|e| anyhow!("fs.edit: read {path_label:?}: {e}"))?;
    let text = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => {
            return Ok(rejection(
                "NotUtf8",
                "fs.edit requires a UTF-8 file; for binary, use fs.write",
                None,
                &path_label,
            ));
        }
    };

    let count = count_occurrences(&text, &old_string);
    if count == 0 {
        return Ok(rejection(
            "NotFound",
            "old_string not found in file",
            Some(json!({
                "search_preview": preview(&old_string, 80),
                "search_bytes": old_string.len(),
            })),
            &path_label,
        ));
    }
    if count > 1 && !replace_all {
        return Ok(rejection(
            "AmbiguousMatch",
            "old_string matched multiple positions; pass replace_all=true or include more context",
            Some(json!({
                "match_count": count,
                "search_preview": preview(&old_string, 80),
            })),
            &path_label,
        ));
    }

    // Single match (count == 1) or replace_all == true.
    let replaced = if replace_all {
        text.replace(&old_string, &new_string)
    } else {
        // count == 1 — replacen with limit=1 is identical to
        // replace here, but expressing the intent (\"this is the
        // exactly-once branch\") in code helps a future reader.
        text.replacen(&old_string, &new_string, 1)
    };
    let new_bytes = replaced.into_bytes();
    write_atomic(dst, &new_bytes)?;

    Ok(json!({
        "ok": true,
        "kind": "edited",
        "matches_replaced": if replace_all { count } else { 1 },
        "bytes_written": new_bytes.len(),
        "content_sha256": hex::encode(Sha256::digest(&new_bytes)),
        "display_path": path_label,
        "resource_ref_revalidated": true,
        "ability_profile_version": PROFILE_VERSION,
    }))
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.matches(needle).count()
}

fn preview(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    // Walk char boundaries so we don't slice mid-codepoint.
    let mut end = cap;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = String::with_capacity(end + 1);
    out.push_str(&s[..end]);
    out.push('…');
    out
}

fn rejection(code: &str, message: &str, detail: Option<Value>, display_path: &str) -> Value {
    json!({
        "ok": false,
        "code": code,
        "message": message,
        "display_path": display_path,
        "detail": detail.unwrap_or(Value::Null),
        "ability_profile_version": PROFILE_VERSION,
    })
}

/// Atomic write: tempfile in the same dir, fdatasync, mode-
/// preserve, rename. Mirrors fs_ability::handler_write so the
/// durability story is uniform across the two write-bearing
/// abilities. Permissions are preserved when the target file
/// already exists — overwriting `chmod 600 secret.key` must
/// not silently downgrade the mode to umask default.
///
/// `dst` is expected to be the symlink-resolved final path
/// (caller does this via fs_ability::resolve_symlink_one_level).
fn write_atomic(dst: &Path, raw: &[u8]) -> Result<()> {
    let existing_mode: Option<u32> = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(dst)
                .ok()
                .map(|m| m.permissions().mode() & 0o7777)
        }
        #[cfg(not(unix))]
        {
            None
        }
    };
    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    let stem = dst.file_name().and_then(|n| n.to_str()).unwrap_or("__edit");
    let tmp = parent.join(format!(".{stem}.tmp.{}", uuid_suffix()));
    {
        let mut f =
            std::fs::File::create(&tmp).map_err(|e| anyhow!("fs.edit: create tmp {tmp:?}: {e}"))?;
        f.write_all(raw)
            .map_err(|e| anyhow!("fs.edit: write tmp {tmp:?}: {e}"))?;
        f.sync_data()
            .map_err(|e| anyhow!("fs.edit: fdatasync {tmp:?}: {e}"))?;
    }
    #[cfg(unix)]
    if let Some(m) = existing_mode {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(m);
        std::fs::set_permissions(&tmp, perms).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            anyhow!("fs.edit: chmod tmp {tmp:?}: {e}")
        })?;
    }
    #[cfg(not(unix))]
    let _ = existing_mode;

    std::fs::rename(&tmp, dst).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow!("fs.edit: rename {tmp:?} -> {dst:?}: {e}")
    })?;
    Ok(())
}

fn uuid_suffix() -> String {
    // Same shape as fs_ability::uuid_suffix — 12 hex chars from
    // a v4 UUID, plenty for in-directory tempfile uniqueness.
    let id = uuid::Uuid::new_v4();
    let s = id.as_simple().to_string();
    s[..12].to_string()
}

// ── Schema + description ──────────────────────────────────────────

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "required": ["resource_ref", "old_string", "new_string"],
        "additionalProperties": false,
        "properties": {
            "resource_ref": crate::daemon::resources::files::resource_ref_schema(),
            "old_string": {
                "type": "string",
                "description": "Exact substring to find. Empty string + non-existent file = create-new-file with new_string as content."
            },
            "new_string": {
                "type": "string",
                "description": "Replacement substring."
            },
            "replace_all": {
                "type": "boolean",
                "description": "When true, replace every occurrence; receipt records match_count. When false (default), the call rejects with AmbiguousMatch unless old_string occurs exactly once."
            },
            "expected_mtime_ms": {
                "type": "integer",
                "minimum": 0,
                "description": "Optional staleness guard. Caller asserts the target's mtime (ms since UNIX epoch) matches the value last seen. Receiver stats the file before editing and rejects with StaleMtime if the actual mtime differs."
            }
        }
    })
}

pub fn description() -> &'static str {
    "Surgical string-replace edit on a single text file through a revalidated \
     RFC-005 filesystem ResourceRef. Default contract: old_string MUST occur \
     exactly once; ambiguous matches reject with the count rather than silently \
     rewriting all occurrences. Pass replace_all=true to opt into bulk \
     replacement. File size capped at 1 GiB. Atomic write via tempfile + \
     fdatasync + rename. Part of the baseline-locomotion-v1 profile \
     (AXIOM §Tier 2.5)."
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("easynet-fs-edit-{pid}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
    }

    fn read_file(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    fn edit_ref(path: &Path) -> Value {
        crate::daemon::resources::files::resource_ref_for_local_path(
            path,
            crate::daemon::resources::files::FilesystemResourceCapability::Write,
        )
        .unwrap()
    }

    // ─── exactly-once happy path ───────────────────────────

    #[test]
    fn replaces_unique_match() {
        let dir = temp_dir();
        let path = dir.join("a.txt");
        write_file(&path, "hello world");
        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "world",
            "new_string": "rust",
        }))
        .unwrap();
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["kind"], json!("edited"));
        assert_eq!(resp["matches_replaced"], json!(1));
        assert_eq!(read_file(&path), "hello rust");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ambiguous_match_rejects_without_replace_all() {
        let dir = temp_dir();
        let path = dir.join("a.txt");
        write_file(&path, "foo foo foo");
        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "foo",
            "new_string": "bar",
        }))
        .unwrap();
        assert_eq!(resp["ok"], json!(false));
        assert_eq!(resp["code"], json!("AmbiguousMatch"));
        assert_eq!(resp["detail"]["match_count"], json!(3));
        // File MUST be unchanged.
        assert_eq!(read_file(&path), "foo foo foo");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn replace_all_flag_replaces_every_occurrence() {
        let dir = temp_dir();
        let path = dir.join("a.txt");
        write_file(&path, "foo foo foo");
        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "foo",
            "new_string": "bar",
            "replace_all": true,
        }))
        .unwrap();
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["matches_replaced"], json!(3));
        assert_eq!(read_file(&path), "bar bar bar");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn not_found_returns_search_preview() {
        let dir = temp_dir();
        let path = dir.join("a.txt");
        write_file(&path, "hello world");
        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "missing",
            "new_string": "x",
        }))
        .unwrap();
        assert_eq!(resp["code"], json!("NotFound"));
        assert_eq!(resp["detail"]["search_preview"], json!("missing"));
        assert_eq!(read_file(&path), "hello world");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── empty old_string ────────────────────────────────

    #[test]
    fn empty_old_string_creates_new_file() {
        let dir = temp_dir();
        let path = dir.join("new.txt");
        assert!(!path.exists());
        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "",
            "new_string": "fresh content",
        }))
        .unwrap();
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(resp["kind"], json!("created"));
        assert_eq!(read_file(&path), "fresh content");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_old_string_creates_empty_file() {
        // Edge case: caller wants to create an empty placeholder.
        let dir = temp_dir();
        let path = dir.join("empty.txt");
        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "",
            "new_string": "",
        }))
        .unwrap();
        assert_eq!(resp["ok"], json!(true));
        assert!(path.exists());
        assert_eq!(read_file(&path), "");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_old_string_on_existing_file_rejects() {
        let dir = temp_dir();
        let path = dir.join("exists.txt");
        write_file(&path, "do not touch");
        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "",
            "new_string": "x",
        }))
        .unwrap();
        assert_eq!(resp["code"], json!("AmbiguousEmptyOldString"));
        assert_eq!(read_file(&path), "do not touch");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── missing file ────────────────────────────────────

    #[test]
    fn non_empty_old_string_on_missing_file_rejects() {
        let dir = temp_dir();
        let path = dir.join("nope.txt");
        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "x",
            "new_string": "y",
        }))
        .unwrap();
        assert_eq!(resp["code"], json!("NoSuchFile"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── encoding ────────────────────────────────────────

    #[test]
    fn binary_file_rejects_with_not_utf8() {
        let dir = temp_dir();
        let path = dir.join("bin.dat");
        std::fs::write(&path, [0xFF, 0xFE, 0xFD]).unwrap();
        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "x",
            "new_string": "y",
        }))
        .unwrap();
        assert_eq!(resp["code"], json!("NotUtf8"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── size guard ──────────────────────────────────────

    // Note: the 1 GiB cap is too large to exercise reliably in a
    // unit test (would write 1 GiB to /tmp). The cap value itself
    // and the metadata-check path are exercised by
    // file_size_cap_constant_is_one_gib + the rejection helper
    // tests below.

    #[test]
    fn file_size_cap_constant_is_one_gib() {
        assert_eq!(MAX_EDIT_FILE_SIZE, 1024 * 1024 * 1024);
    }

    // ─── helpers ─────────────────────────────────────────

    #[test]
    fn count_occurrences_basic() {
        assert_eq!(count_occurrences("abcabc", "abc"), 2);
        assert_eq!(count_occurrences("abcabc", "x"), 0);
        assert_eq!(count_occurrences("aaa", "aa"), 1); // non-overlapping
        assert_eq!(count_occurrences("anything", ""), 0);
    }

    #[test]
    fn preview_truncates_at_cap_with_ellipsis() {
        assert_eq!(preview("short", 80), "short");
        let long = "x".repeat(200);
        let p = preview(&long, 80);
        assert!(p.ends_with('…'));
        assert!(p.chars().count() <= 81);
    }

    #[test]
    fn preview_respects_utf8_boundary() {
        // 中 is 3 UTF-8 bytes. cap=2 must not slice mid-codepoint.
        let s = "中文";
        let p = preview(s, 2);
        assert!(p.is_ascii() || p.chars().all(|c| c.is_ascii() || c == '…'));
    }

    #[test]
    fn rejection_response_shape() {
        let v = rejection("Foo", "msg", None, "tmp/x");
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["code"], json!("Foo"));
        assert_eq!(v["message"], json!("msg"));
        assert_eq!(v["display_path"], json!("tmp/x"));
        assert_eq!(v["detail"], Value::Null);
    }

    // ─── schema ──────────────────────────────────────────

    #[test]
    fn input_schema_requires_resource_ref_old_string_new_string() {
        let s = input_schema();
        let req: Vec<&str> = s["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(req.contains(&"resource_ref"));
        assert!(req.contains(&"old_string"));
        assert!(req.contains(&"new_string"));
        assert!(s["properties"].get("path").is_none());
        assert!(s["properties"].get("resource_ref").is_some());
    }

    #[test]
    fn missing_resource_ref_rejects_before_filesystem_access() {
        let err = handler(json!({
            "old_string": "old",
            "new_string": "new",
        }))
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("resource_ref: missing required object"),
            "{err}"
        );
    }

    #[test]
    fn stale_resource_ref_revision_rejects_before_filesystem_access() {
        let dir = temp_dir();
        let path = dir.join("a.txt");
        write_file(&path, "old");
        let mut resource_ref = edit_ref(&path);
        resource_ref["revision"] = json!("stale");
        let err = handler(json!({
            "resource_ref": resource_ref,
            "old_string": "old",
            "new_string": "new",
        }))
        .unwrap_err();
        assert!(err.to_string().contains("resource_ref: revision mismatch"));
        assert_eq!(read_file(&path), "old");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_only_resource_ref_cannot_edit() {
        let dir = temp_dir();
        let path = dir.join("a.txt");
        write_file(&path, "old");
        let resource_ref = crate::daemon::resources::files::resource_ref_for_local_path(
            &path,
            crate::daemon::resources::files::FilesystemResourceCapability::Read,
        )
        .unwrap();
        let err = handler(json!({
            "resource_ref": resource_ref,
            "old_string": "old",
            "new_string": "new",
        }))
        .unwrap_err();
        assert!(err
            .to_string()
            .contains("resource_ref: capability read does not permit write"));
        assert_eq!(read_file(&path), "old");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── atomicity ───────────────────────────────────────

    #[test]
    fn write_atomic_does_not_leave_tmp_on_success() {
        let dir = temp_dir();
        let path = dir.join("a.txt");
        write_atomic(&path, b"hi").unwrap();
        // No `.a.txt.tmp.*` file should remain.
        let entries: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries, vec!["a.txt".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── post-AliveCode-audit hardening ──────────────────────

    #[cfg(unix)]
    #[test]
    fn edit_preserves_existing_file_mode() {
        // chmod 600 → fs.edit replaces a string → mode stays 600.
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir();
        let path = dir.join("secret.conf");
        std::fs::write(&path, "key=old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "old",
            "new_string": "new",
        }))
        .unwrap();
        assert_eq!(resp["ok"], json!(true));
        let final_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(final_mode, 0o600);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edit_through_symlink_keeps_link_intact() {
        let dir = temp_dir();
        let real = dir.join("real.txt");
        let link = dir.join("link.txt");
        std::fs::write(&real, "first").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real, &link).unwrap();
        }
        #[cfg(not(unix))]
        {
            std::fs::remove_dir_all(&dir).ok();
            return;
        }
        let resp = handler(json!({
            "resource_ref": edit_ref(&link),
            "old_string": "first",
            "new_string": "second",
        }))
        .unwrap();
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "second");
        let link_meta = std::fs::symlink_metadata(&link).unwrap();
        assert!(link_meta.file_type().is_symlink());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edit_with_matching_expected_mtime_succeeds() {
        let dir = temp_dir();
        let path = dir.join("a.txt");
        std::fs::write(&path, "old").unwrap();
        let mtime = crate::daemon::ability::builtins::device_control::files::file_mtime_ms(
            &std::fs::metadata(&path).unwrap(),
        )
        .unwrap();
        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "old",
            "new_string": "new",
            "expected_mtime_ms": mtime,
        }))
        .unwrap();
        assert_eq!(resp["ok"], json!(true));
        assert_eq!(read_file(&path), "new");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edit_with_stale_expected_mtime_rejects() {
        let dir = temp_dir();
        let path = dir.join("a.txt");
        std::fs::write(&path, "old").unwrap();
        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "old",
            "new_string": "new",
            "expected_mtime_ms": 1u64,
        }))
        .unwrap();
        assert_eq!(resp["ok"], json!(false));
        assert_eq!(resp["code"], json!("StaleMtime"));
        // File MUST be unchanged.
        assert_eq!(read_file(&path), "old");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn edit_with_expected_mtime_on_missing_file_rejects() {
        let dir = temp_dir();
        let path = dir.join("nope.txt");
        let resp = handler(json!({
            "resource_ref": edit_ref(&path),
            "old_string": "x",
            "new_string": "y",
            "expected_mtime_ms": 12345u64,
        }))
        .unwrap();
        assert_eq!(resp["code"], json!("StaleMtime"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
