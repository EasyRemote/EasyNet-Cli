// EasyNet CLI — Baseline Locomotion Profile: filesystem abilities
// =================================================================
//
// File: src/daemon/ability/builtins/device_control/files.rs
// Description: `fs.read`, `fs.write`, `fs.stat`, `fs.list` — the
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
//   realisation. RFC-005 makes this a ResourceRef-only public
//   surface: callers do not send raw host paths.
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
// - No process-wide caching. Each call revalidates the supplied
//   ResourceRef, maps its virtual root plus relative path to a
//   local host path, then performs exactly one filesystem verb.
//   Sandboxing (chroot, OS capabilities) is deployment's
//   responsibility, not the ability's.
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
// - Path traversal: `..` is rejected inside ResourceRef paths.
//   Per-path ACLs are a higher-tier policy concern, but the v1
//   filesystem surface is still bounded by virtual roots.
//
// Architectural Position:
// - Sibling of `process_exec_ability.rs` and `http_request_ability.rs`
//   (later commits). Together they form the host-embodied
//   agent's `baseline-locomotion-v1` profile.
//
// Author: Silan.Hu
// Email: silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::ability::dispatch::OwnerKind;
pub use crate::daemon::resources::files::{
    resource_ref_for_local_path, FilesystemResourceCapability, ResolvedFilesystemPath,
};

use crate::daemon::resources::files as filesystem;
// ── Wire-name constants (cross-language pins) ─────────────────────

pub const ABILITY_FS_READ: &str = crate::daemon::ability::names::device_control::FS_READ;
pub const ABILITY_FS_WRITE: &str = crate::daemon::ability::names::device_control::FS_WRITE;
pub const ABILITY_FS_STAT: &str = crate::daemon::ability::names::device_control::FS_STAT;
pub const ABILITY_FS_LIST: &str = crate::daemon::ability::names::device_control::FS_LIST;

/// Profile membership marker. Receivers MAY surface this in
/// `agent.describe` so callers can confirm the profile contract
/// before invoking. Mirrors AXIOM §"Tier 2.5" `ability_profile_version`.
pub const PROFILE_VERSION: &str =
    crate::daemon::ability::names::device_control::BASELINE_LOCOMOTION_PROFILE_VERSION;

/// Default cap on `fs.read` size when the caller does not name a
/// `max_bytes` limit. 8 MiB is large enough to read most config
/// files, source files, and small datasets in one call; bigger
/// reads should use a payload-ref envelope or stream via
/// InvokeBidi.
const DEFAULT_READ_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// Hard upper bound on `max_bytes` per call. 100 MiB matches the
/// runner's hard cap for stdout/stderr so a single ability
/// receipt cannot exceed any other Tier 2.5 ability's. Callers
/// passing a larger value get a clear error rather than silently
/// reading "as much as available" — and we never allocate the
/// underlying `Vec<u8>` past this size, so a caller that asks
/// for `max_bytes = u64::MAX` cannot OOM the daemon.
const READ_MAX_BYTES_HARD_CAP: u64 = 100 * 1024 * 1024;

/// Paths that block forever, return non-deterministic content,
/// or alias the agent's own stdio. Each entry is matched against
/// the LITERAL path string the caller passed, after `canonicalize`
/// resolves symlinks and normalises `..`. Resolving to one of
/// these returns `fs.read: blocked device path` instead of
/// hanging the blocking-pool thread on `read(/dev/zero)`.
///
/// The list is intentionally narrow:
///
///   * Linux character-special devices that block on read or
///     produce unbounded streams (`/dev/zero`, `/dev/random`,
///     `/dev/urandom`, `/dev/null` excluded — null reads return
///     EOF immediately, not a hazard).
///   * Stdio aliases (`/dev/stdin`, `/dev/stdout`, `/dev/tty`,
///     `/proc/self/fd/0`, `/proc/self/fd/1`, `/proc/self/fd/2`).
///     Reading these would either steal the agent's own stdin
///     or echo its own stdout back into the receipt.
///
/// Per-path ACLs ("read /etc but not /var") remain a higher-tier
/// policy concern, NOT this list. This is purely the fail-closed
/// "do not hang the receiver thread on a special device".
const BLOCKED_READ_PATHS: &[&str] = &[
    "/dev/zero",
    "/dev/random",
    "/dev/urandom",
    "/dev/stdin",
    "/dev/stdout",
    "/dev/tty",
    "/proc/self/fd/0",
    "/proc/self/fd/1",
    "/proc/self/fd/2",
];

/// Default cap on `fs.list` entry count. A directory with more
/// entries than this is a misuse of `fs.list`; callers wanting
/// pagination should use higher-tier indexed-storage abilities.
const DEFAULT_LIST_MAX_ENTRIES: usize = 4096;

// ── Registration ───────────────────────────────────────────────────

/// Wire all three filesystem abilities into the registry. Called
/// from `daemon::ability::catalog::build_registry_with_services` once at
/// daemon startup. The abilities are stateless so registration is
/// just three handler closures with no per-call setup.
pub fn register(reg: &mut AxonAbilityCatalog) {
    reg.register_rpc_with_owner("fs.read", OwnerKind::Device, Arc::new(handler_read));
    reg.register_rpc_with_owner("fs.write", OwnerKind::Device, Arc::new(handler_write));
    reg.register_rpc_with_owner("fs.stat", OwnerKind::Device, Arc::new(handler_stat));
    reg.register_rpc_with_owner("fs.list", OwnerKind::Device, Arc::new(handler_list));
}

// ── fs.read ──────────────────────────────────────────────────────

fn handler_read(args: Value) -> Result<Value> {
    let max_bytes = args
        .get("max_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_READ_MAX_BYTES);
    if max_bytes > READ_MAX_BYTES_HARD_CAP {
        return Err(anyhow!(
            "fs.read: max_bytes {max_bytes} exceeds hard cap {READ_MAX_BYTES_HARD_CAP}"
        ));
    }
    let encoding = args
        .get("encoding")
        .and_then(Value::as_str)
        .unwrap_or("binary");
    if !matches!(encoding, "binary" | "utf8") {
        return Err(anyhow!(
            "fs.read: unknown encoding {encoding:?}; expected \"binary\" or \"utf8\""
        ));
    }

    // Optional line-mode parameters. Both default to "no
    // line-mode" (use byte cap only). When either is set we
    // require encoding=utf8 — slicing UTF-8 bytes by lines is
    // only well-defined on text content. Numbering is 1-based
    // for `offset_lines` (matches `head -n N`, `tail +N`, and
    // every editor's "go to line" prompt).
    let offset_lines = args.get("offset_lines").and_then(Value::as_u64);
    let limit_lines = args.get("limit_lines").and_then(Value::as_u64);
    if (offset_lines.is_some() || limit_lines.is_some()) && encoding != "utf8" {
        return Err(anyhow!(
            "fs.read: offset_lines/limit_lines require encoding=\"utf8\""
        ));
    }

    let resolved = filesystem::resolve_filesystem_path(&args, FilesystemResourceCapability::Read)?;
    let path = resolved.local_path.as_path();
    let path_label = resolved.display_path.as_str();

    if is_blocked_read_path(path) {
        return Err(anyhow!(
            "fs.read: {path_label:?} is on the blocked-device path list"
        ));
    }

    let metadata =
        std::fs::metadata(path).map_err(|e| anyhow!("fs.read: stat {path_label:?}: {e}"))?;
    let total_size = metadata.len();

    // Stream up to max_bytes + 1, so we can tell "exactly at the
    // cap" from "the file is bigger than the cap" without ever
    // allocating past the cap. `Read::take` enforces the bound
    // at the syscall level — a multi-GB special file or a
    // misreported metadata.len cannot OOM us.
    let file =
        std::fs::File::open(path).map_err(|e| anyhow!("fs.read: open {path_label:?}: {e}"))?;
    let mut limited = file.take(max_bytes.saturating_add(1));
    let mut content: Vec<u8> = Vec::with_capacity((max_bytes.min(64 * 1024)) as usize);
    limited
        .read_to_end(&mut content)
        .map_err(|e| anyhow!("fs.read: read {path:?}: {e}"))?;
    let truncated = content.len() as u64 > max_bytes;
    if truncated {
        content.truncate(max_bytes as usize);
    }

    // Apply line-mode slicing on the captured bytes (already
    // capped). Slicing AFTER the byte cap means a 50 MiB log
    // file with offset_lines=9000 still reads 8 MiB and walks
    // its lines once; we don't promise to find the offset past
    // the byte cap (a future ranged-read variant could).
    let body = match encoding {
        "utf8" => {
            let text = std::str::from_utf8(&content).map_err(|_| {
                anyhow!(
                    "fs.read: file at {path_label:?} is not valid UTF-8; use encoding=\"binary\""
                )
            })?;
            if offset_lines.is_some() || limit_lines.is_some() {
                let sliced = slice_lines(text, offset_lines.unwrap_or(1), limit_lines);
                json!(sliced)
            } else {
                json!(text)
            }
        }
        "binary" => json!(BASE64_STANDARD.encode(&content)),
        _ => unreachable!("fs.read encoding was validated before filesystem resolution"),
    };

    Ok(json!({
        "content": body,
        "size": total_size,
        "truncated": truncated,
        "content_sha256": hex::encode(Sha256::digest(&content)),
        // Surface mtime so a caller doing a read → modify → write
        // round trip can pass it as `expected_mtime_ms` to fs.write
        // / fs.edit and detect concurrent modifications. None when
        // the underlying filesystem doesn't track mtime.
        "mtime_ms": file_mtime_ms(&metadata),
        "ability_profile_version": PROFILE_VERSION,
    }))
}

/// Is `path` (as the caller passed it) on the blocked-device
/// list, OR does its canonical resolution land on the list?
/// The double check defends against `/proc/self/cwd/../dev/zero`
/// or symlinks pointing at `/dev/zero`.
fn is_blocked_read_path(path: &Path) -> bool {
    if let Some(path) = path.to_str() {
        if BLOCKED_READ_PATHS.contains(&path) {
            return true;
        }
    }
    if let Ok(canon) = std::fs::canonicalize(path) {
        if let Some(s) = canon.to_str() {
            return BLOCKED_READ_PATHS.contains(&s);
        }
    }
    false
}

fn is_blocked_read_path_str(path: &str) -> bool {
    if BLOCKED_READ_PATHS.contains(&path) {
        return true;
    }
    if let Ok(canon) = std::fs::canonicalize(Path::new(path)) {
        if let Some(s) = canon.to_str() {
            return BLOCKED_READ_PATHS.contains(&s);
        }
    }
    false
}

/// Public wrapper exposing the blocked-path check for sibling
/// abilities (chat attachments) that need the same defence in
/// depth. Kept named distinctly so a search for `is_blocked_read_path`
/// still surfaces only the in-module call site.
pub(crate) fn is_blocked_read_path_for_chat(path: &str) -> bool {
    is_blocked_read_path_str(path)
}

/// Return the substring of `text` from line `offset_lines`
/// (1-based) inclusive, capped at `limit_lines` lines if set.
/// Trailing newline preserved when present in the source.
/// Out-of-range `offset_lines` returns "".
fn slice_lines(text: &str, offset_lines: u64, limit_lines: Option<u64>) -> String {
    if offset_lines == 0 {
        // 0-based callers are bugs, not "from the start" — bash
        // tools (head -n, sed -n) all use 1-based, so a 0 is a
        // clear caller error. Return "" deterministically rather
        // than aliasing it to 1.
        return String::new();
    }
    let mut out = String::new();
    let mut count_taken: u64 = 0;
    for (idx_zero_based, line) in text.split_inclusive('\n').enumerate() {
        let line_num = (idx_zero_based as u64).saturating_add(1);
        if line_num < offset_lines {
            continue;
        }
        if let Some(limit) = limit_lines {
            if count_taken >= limit {
                break;
            }
        }
        out.push_str(line);
        count_taken += 1;
    }
    out
}

// ── fs.write ─────────────────────────────────────────────────────

fn handler_write(args: Value) -> Result<Value> {
    let resolved = filesystem::resolve_filesystem_path(&args, FilesystemResourceCapability::Write)?;
    let path = resolved.local_path;
    let path_label = resolved.display_path;
    let create_parents = args
        .get("create_parents")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let explicit_mode = args.get("mode").and_then(Value::as_u64).map(|m| m as u32);
    let expected_mtime_ms = args.get("expected_mtime_ms").and_then(Value::as_u64);

    let raw = decode_content(&args)?;

    if create_parents {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| anyhow!("fs.write: mkdir -p {parent:?}: {e}"))?;
            }
        }
    }
    if let Some(root) = resolved.virtual_root_path.as_deref() {
        filesystem::ensure_write_parent_under_root(&path, root)?;
    }

    // Resolve a single layer of symlink so that:
    //
    //   1. The temp file lives next to the REAL file (so
    //      rename(2) stays on one filesystem). Otherwise a
    //      cross-device rename fails with EXDEV.
    //   2. We replace the symlink's TARGET, not the symlink
    //      itself — so `rename(tmp, /etc/foo)` where `/etc/foo`
    //      is a symlink to `/real/foo` ends up updating
    //      `/real/foo`'s inode and leaves the symlink at
    //      `/etc/foo` pointing where it always did.
    //
    // We deliberately resolve only ONE level. Recursive
    // resolution would be a different (and surprising) write
    // semantics; tests need to match what the operator sees with
    // `ls -l`. POSIX rename(2) on a symlink path replaces the
    // symlink itself — not what we want when an agent writes to
    // a config that happens to be a symlink.
    let written_path = resolve_symlink_one_level(&path);
    let dst: &Path = &written_path;
    if let Some(root) = resolved.virtual_root_path.as_deref() {
        if dst.exists() {
            filesystem::ensure_path_under_root(dst, root).map_err(|e| {
                anyhow!("fs.write: resolved target escapes resource virtual root: {e}")
            })?;
        } else {
            filesystem::ensure_write_parent_under_root(dst, root)?;
        }
    }

    // Inspect the existing target to capture its mode (for
    // permission preservation) and its mtime (for the
    // expected_mtime_ms guard). One stat covers both.
    let existing_meta = std::fs::symlink_metadata(dst).ok();
    // Note: symlink_metadata on a regular file behaves the same
    // as metadata — it only differs when the path itself IS a
    // symlink, which we've already resolved away above.
    if let Some(expected) = expected_mtime_ms {
        match &existing_meta {
            Some(m) => {
                let actual = file_mtime_ms(m);
                if actual != Some(expected) {
                    return Err(anyhow!(
                        "fs.write: expected_mtime_ms {expected} != actual {actual:?} \
                         (file modified since the caller's last read; refuse rather \
                         than clobber concurrent edits)"
                    ));
                }
            }
            None => {
                // Caller passed expected_mtime_ms against a file
                // that doesn't exist. Treat as a strict
                // staleness assertion: no existing file means
                // the caller's mental model is wrong.
                return Err(anyhow!(
                    "fs.write: expected_mtime_ms set but file does not exist"
                ));
            }
        }
    }

    // Permission decision (Unix only):
    //   * Caller passed `mode` → use it (caller wins).
    //   * Otherwise, target exists → preserve its mode.
    //   * Otherwise, fall back to umask default.
    #[cfg(unix)]
    let target_mode: Option<u32> = explicit_mode.or_else(|| {
        use std::os::unix::fs::PermissionsExt;
        existing_meta
            .as_ref()
            .map(|m| m.permissions().mode() & 0o7777)
    });
    #[cfg(not(unix))]
    let target_mode: Option<u32> = explicit_mode;

    // Atomic write: write to a temp file in the same directory
    // as the resolved target, fsync it, set its mode, then
    // rename. A crash mid-write leaves the prior content intact.
    // The fsync before rename is the load-bearing step often
    // missed in "tempfile + rename = atomic" lore: ext4/XFS are
    // within their rights to present a zero-byte file on power
    // loss between rename(2) and the deferred data flush. The
    // chmod-before-rename means the published file has its
    // final permissions atomically — there is no instant when
    // the file exists with a wrong mode.
    use std::io::Write as _;
    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    let file_stem = dst
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("__write");
    let tmp = parent.join(format!(".{file_stem}.tmp.{}", uuid_suffix()));

    {
        let mut f = std::fs::File::create(&tmp)
            .map_err(|e| anyhow!("fs.write: create tmp {tmp:?}: {e}"))?;
        f.write_all(&raw)
            .map_err(|e| anyhow!("fs.write: write tmp {tmp:?}: {e}"))?;
        f.sync_data()
            .map_err(|e| anyhow!("fs.write: fdatasync {tmp:?}: {e}"))?;
    }

    #[cfg(unix)]
    if let Some(m) = target_mode {
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
    let _ = target_mode;

    std::fs::rename(&tmp, dst).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        anyhow!("fs.write: rename {tmp:?} -> {dst:?}: {e}")
    })?;

    let mode_preserved = explicit_mode.is_none() && existing_meta.is_some();
    let mut resp = json!({
        "bytes_written": raw.len(),
        "content_sha256": hex::encode(Sha256::digest(&raw)),
        "mode_preserved": mode_preserved,
        "ability_profile_version": PROFILE_VERSION,
    });
    if path != dst {
        // We followed a symlink. Surface the resolved path so
        // the caller (and the receipt) record what was actually
        // written. The original symlink path is implicit in the
        // request.
        resp["resolved_target"] = json!(dst.to_string_lossy());
    }
    resp["resource_ref_revalidated"] = json!(true);
    resp["display_path"] = json!(path_label);
    Ok(resp)
}

/// If `path` is a symlink, return the absolute path of its
/// immediate target; otherwise return `path` unchanged. Only
/// one level — recursive resolution would be a different
/// semantics (matches `readlink`, not `realpath`).
///
/// `pub(super)` so the sibling fs.edit ability gets the same
/// symlink-aware atomic-write semantics without re-implementing
/// the helper.
pub(super) fn resolve_symlink_one_level(path: &Path) -> std::path::PathBuf {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => match std::fs::read_link(path) {
            Ok(link) if link.is_absolute() => link,
            Ok(link) => {
                let parent = path.parent().unwrap_or_else(|| Path::new("."));
                parent.join(link)
            }
            Err(_) => path.to_path_buf(),
        },
        _ => path.to_path_buf(),
    }
}

/// Convert `Metadata::modified()` to milliseconds-since-epoch.
/// Returns None for filesystems that don't track mtime.
/// Shared with the fs.edit ability for the expected_mtime_ms
/// staleness guard.
pub(super) fn file_mtime_ms(m: &std::fs::Metadata) -> Option<u64> {
    let modified = m.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_millis() as u64)
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
            let n = v
                .as_u64()
                .ok_or_else(|| anyhow!("fs.write: content[] entries must be integers in 0..256"))?;
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

fn handler_stat(args: Value) -> Result<Value> {
    let resolved = filesystem::resolve_filesystem_path(&args, FilesystemResourceCapability::Stat)?;
    let mut obj = describe_entry(&resolved.local_path, &resolved.display_path)?;
    obj["ability_profile_version"] = json!(PROFILE_VERSION);
    obj["resource_ref_revalidated"] = json!(true);
    obj["display_path"] = json!(resolved.display_path);
    Ok(obj)
}

fn handler_list(args: Value) -> Result<Value> {
    let resolved = filesystem::resolve_filesystem_path(&args, FilesystemResourceCapability::List)?;
    let path = resolved.local_path;
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
        list_recursive(
            &path,
            resolved.display_path.as_str(),
            max_entries,
            &mut entries,
            &mut truncated,
        )?;
    } else {
        for entry in
            std::fs::read_dir(&path).map_err(|e| anyhow!("fs.list: read_dir {path:?}: {e}"))?
        {
            if entries.len() >= max_entries {
                truncated = true;
                break;
            }
            let entry = entry.map_err(|e| anyhow!("fs.list: iter entry: {e}"))?;
            let entry_path = entry.path();
            let display_path = entry_display_path(
                resolved.display_path.as_str(),
                entry_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(""),
            );
            entries.push(describe_entry(&entry_path, &display_path)?);
        }
    }

    let mut response = json!({
        "entries": entries,
        "truncated": truncated,
        "ability_profile_version": PROFILE_VERSION,
    });
    response["resource_ref_revalidated"] = json!(true);
    response["display_path"] = json!(resolved.display_path);
    Ok(response)
}

fn list_recursive(
    dir: &Path,
    dir_display_path: &str,
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
        let display_path = entry_display_path(
            dir_display_path,
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(""),
        );
        out.push(describe_entry(&path, &display_path)?);
        if path.is_dir() && !path.is_symlink() {
            // is_symlink check prevents infinite loops via
            // self-referential symlinks. A symlink that points
            // back to an ancestor would otherwise blow up the
            // entry list.
            list_recursive(&path, &display_path, max_entries, out, truncated)?;
            if *truncated {
                return Ok(());
            }
        }
    }
    Ok(())
}

fn entry_display_path(parent_display_path: &str, entry_name: &str) -> String {
    let parent = parent_display_path.trim_end_matches('/');
    if parent.is_empty() {
        entry_name.to_string()
    } else if entry_name.is_empty() {
        parent.to_string()
    } else {
        format!("{parent}/{entry_name}")
    }
}

fn describe_entry(path: &Path, display_path: &str) -> Result<Value> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|e| anyhow!("fs.list: stat {path:?}: {e}"))?;
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
        "display_path": display_path,
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
        "required": ["resource_ref"],
        "additionalProperties": false,
        "properties": {
            "resource_ref": filesystem::resource_ref_schema(),
            "max_bytes": {
                "type": "integer",
                "minimum": 0,
                "maximum": READ_MAX_BYTES_HARD_CAP,
                "description": "Per-call read cap. Default 8 MiB, hard cap 100 MiB. Read is streamed up to this cap; multi-GB paths cannot OOM the receiver."
            },
            "encoding": { "type": "string", "enum": ["binary", "utf8"] },
            "offset_lines": {
                "type": "integer",
                "minimum": 0,
                "description": "1-based starting line. Requires encoding=\"utf8\". 0 returns empty deterministically."
            },
            "limit_lines": {
                "type": "integer",
                "minimum": 0,
                "description": "Maximum lines to return after offset_lines. Requires encoding=\"utf8\"."
            }
        }
    })
}

pub fn input_schema_write() -> Value {
    json!({
        "type": "object",
        "required": ["resource_ref", "content"],
        "additionalProperties": false,
        "properties": {
            "resource_ref": filesystem::resource_ref_schema(),
            "content": {
                "oneOf": [
                    { "type": "string", "description": "base64-encoded by default; pass encoding=\"utf8\" for raw text" },
                    { "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 255 } }
                ]
            },
            "encoding": { "type": "string", "enum": ["base64", "utf8"] },
            "mode": {
                "type": "integer",
                "minimum": 0,
                "description": "Unix permission bits. When set, the caller wins. When unset and the target file already exists, the receiver preserves the target's existing mode. When unset and the target is new, falls back to umask default."
            },
            "create_parents": { "type": "boolean" },
            "expected_mtime_ms": {
                "type": "integer",
                "minimum": 0,
                "description": "Optional staleness guard. Caller asserts the target's mtime (milliseconds since UNIX epoch) matches the value last seen. Receiver stats the file before writing and rejects if the actual mtime differs (or the file does not exist). Stateless equivalent of FileWriteTool's read-before-write check; the caller passes the mtime they captured from the most recent fs.read of the same path."
            }
        }
    })
}

pub fn input_schema_stat() -> Value {
    json!({
        "type": "object",
        "required": ["resource_ref"],
        "additionalProperties": false,
        "properties": {
            "resource_ref": filesystem::resource_ref_schema()
        }
    })
}

pub fn input_schema_list() -> Value {
    json!({
        "type": "object",
        "required": ["resource_ref"],
        "additionalProperties": false,
        "properties": {
            "resource_ref": filesystem::resource_ref_schema(),
            "recursive": { "type": "boolean" },
            "max_entries": { "type": "integer", "minimum": 1 }
        }
    })
}

pub fn description_read() -> &'static str {
    "Read a file through a revalidated filesystem ResourceRef (streamed up to \
     max_bytes, default 8 MiB, hard cap 100 MiB). With \
     encoding=\"utf8\" the optional offset_lines / limit_lines \
     pair selects a 1-based line window. Blocked-device paths \
     (/dev/{zero,random,urandom,stdin,stdout,tty}, \
     /proc/self/fd/{0,1,2}) reject. Part of the \
     baseline-locomotion-v1 profile (AXIOM §Tier 2.5)."
}

pub fn description_write() -> &'static str {
    "Write a revalidated filesystem ResourceRef atomically (temp + rename). \
     Part of the baseline-locomotion-v1 profile (AXIOM §Tier 2.5)."
}

pub fn description_stat() -> &'static str {
    "Stat a file or directory through a revalidated RFC-005 ResourceRef. Part of the \
     baseline-locomotion-v1 profile (AXIOM §Tier 2.5)."
}

pub fn description_list() -> &'static str {
    "List the contents of a directory through a revalidated filesystem ResourceRef. \
     Part of the baseline-locomotion-v1 profile (AXIOM §Tier 2.5)."
}

// ── Helpers ────────────────────────────────────────────────────────

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

    fn local_ref(path: &Path, capability: FilesystemResourceCapability) -> Value {
        filesystem::resource_ref_for_local_path(path, capability).unwrap()
    }

    // ─── fs.read ─────────────────────────────────────────────

    #[test]
    fn read_returns_utf8_when_requested() {
        let dir = temp_dir();
        let path = dir.join("hello.txt");
        std::fs::write(&path, "hello world").unwrap();

        let resp = handler_read(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Read),
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
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Read),
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
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Read),
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
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Read),
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
        let err = handler_read(json!({
            "encoding": "rot13",
        }))
        .unwrap_err();
        assert!(err.to_string().contains("unknown encoding"));
    }

    // ─── fs.read hardening (slice 10a) ───────────────────────

    #[test]
    fn read_rejects_max_bytes_over_hard_cap() {
        let dir = temp_dir();
        let path = dir.join("x.txt");
        std::fs::write(&path, "x").unwrap();

        let err = handler_read(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Read),
            "max_bytes": READ_MAX_BYTES_HARD_CAP + 1,
        }))
        .unwrap_err();
        assert!(err.to_string().contains("hard cap"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_does_not_oom_on_overstated_max_bytes() {
        // Pre-fix behavior: `fs::read(path)` then `truncate` would
        // allocate a Vec sized to the file's actual length BEFORE
        // truncating, ignoring max_bytes for the allocation step.
        // After the fix, `Read::take(max_bytes + 1)` enforces the
        // bound at the syscall level. Use a 10 KiB file with a
        // generous max_bytes; the test passes if the read returns
        // and the response shape is correct.
        let dir = temp_dir();
        let path = dir.join("ten_k.bin");
        std::fs::write(&path, vec![b'.'; 10_240]).unwrap();
        let resp = handler_read(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Read),
            "max_bytes": 1_000_000,
        }))
        .unwrap();
        assert_eq!(resp["size"], json!(10_240));
        assert_eq!(resp["truncated"], json!(false));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_line_offset_returns_specified_window() {
        let dir = temp_dir();
        let path = dir.join("lines.txt");
        let body = (1..=10)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, &body).unwrap();

        let resp = handler_read(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Read),
            "encoding": "utf8",
            "offset_lines": 3,
            "limit_lines": 2,
        }))
        .unwrap();
        let content = resp["content"].as_str().unwrap();
        assert!(content.contains("line 3"));
        assert!(content.contains("line 4"));
        assert!(!content.contains("line 5"));
        assert!(!content.contains("line 1"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_line_offset_zero_returns_empty() {
        let dir = temp_dir();
        let path = dir.join("any.txt");
        std::fs::write(&path, "x\ny\n").unwrap();
        let resp = handler_read(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Read),
            "encoding": "utf8",
            "offset_lines": 0,
        }))
        .unwrap();
        assert_eq!(resp["content"].as_str().unwrap(), "");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_line_offset_past_eof_returns_empty() {
        let dir = temp_dir();
        let path = dir.join("two.txt");
        std::fs::write(&path, "a\nb\n").unwrap();
        let resp = handler_read(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Read),
            "encoding": "utf8",
            "offset_lines": 100,
        }))
        .unwrap();
        assert_eq!(resp["content"].as_str().unwrap(), "");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_line_mode_requires_utf8_encoding() {
        let dir = temp_dir();
        let path = dir.join("any.bin");
        std::fs::write(&path, "x").unwrap();
        let err = handler_read(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Read),
            "offset_lines": 1,
        }))
        .unwrap_err();
        assert!(err.to_string().contains("offset_lines"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn slice_lines_helper_basic() {
        assert_eq!(slice_lines("a\nb\nc\n", 1, None), "a\nb\nc\n");
        assert_eq!(slice_lines("a\nb\nc\n", 2, None), "b\nc\n");
        assert_eq!(slice_lines("a\nb\nc\n", 1, Some(2)), "a\nb\n");
        assert_eq!(slice_lines("a\nb\nc\n", 2, Some(1)), "b\n");
        assert_eq!(slice_lines("a\nb\nc\n", 100, None), "");
        assert_eq!(slice_lines("a\nb\nc\n", 0, None), "");
    }

    #[test]
    fn is_blocked_read_path_recognises_known_devices() {
        if std::path::Path::new("/dev/zero").exists() {
            assert!(is_blocked_read_path_str("/dev/zero"));
        }
        // Not on the list — even if the file exists, it must
        // not be flagged.
        let dir = temp_dir();
        let path = dir.join("safe.txt");
        std::fs::write(&path, "x").unwrap();
        assert!(!is_blocked_read_path_str(path.to_str().unwrap()));
        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── fs.write ────────────────────────────────────────────

    #[test]
    fn write_utf8_creates_file_atomically() {
        let dir = temp_dir();
        let path = dir.join("out.txt");

        let resp = handler_write(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Write),
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
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Write),
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
            "resource_ref": local_ref(&nested, FilesystemResourceCapability::Write),
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
            "resource_ref": local_ref(&nested, FilesystemResourceCapability::Write),
            "content": "x",
            "encoding": "utf8",
        }))
        .unwrap_err();
        // The tmp file's parent dir doesn't exist, so File::create
        // fails — error mentions either "create tmp" or the
        // missing path. Either is fine; the test is about
        // *some* error happening before the rename clobbers
        // anything.
        let msg = err.to_string();
        assert!(
            msg.contains("create tmp") || msg.contains("write tmp"),
            "expected create/write tmp error, got: {msg}"
        );
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
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Write),
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
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Write),
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
            "resource_ref": local_ref(&dir, FilesystemResourceCapability::List),
        }))
        .unwrap();
        let entries = resp["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        for entry in entries {
            assert!(
                entry.get("path").is_none(),
                "fs.list entries must not expose daemon host paths: {entry:?}"
            );
            assert!(
                entry["display_path"].as_str().is_some(),
                "fs.list entries must expose ResourceRef display paths: {entry:?}"
            );
        }
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
            "resource_ref": local_ref(&dir, FilesystemResourceCapability::List),
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
        let display_paths: Vec<String> = resp["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["display_path"].as_str().unwrap().to_string())
            .collect();
        assert!(
            display_paths.iter().any(|path| path.ends_with("/sub")),
            "recursive list missing sub display path: {display_paths:?}"
        );
        assert!(
            display_paths
                .iter()
                .any(|path| path.ends_with("/sub/inner.txt")),
            "recursive list missing inner display path: {display_paths:?}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn list_truncates_at_max_entries() {
        let dir = temp_dir();
        for i in 0..10 {
            std::fs::write(dir.join(format!("f{i}.txt")), "x").unwrap();
        }
        let resp = handler_list(json!({
            "resource_ref": local_ref(&dir, FilesystemResourceCapability::List),
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

        let resp = handler_list(json!({
            "resource_ref": local_ref(&dir, FilesystemResourceCapability::List),
        }))
        .unwrap();
        for entry in resp["entries"].as_array().unwrap() {
            match entry["kind"].as_str().unwrap() {
                "file" => assert_eq!(entry["size"], json!(5)),
                "dir" => assert!(entry.get("size").is_none()),
                _ => {}
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn public_fs_handlers_require_resource_ref() {
        for err in [
            handler_read(json!({ "encoding": "utf8" })).unwrap_err(),
            handler_write(json!({ "content": "x", "encoding": "utf8" })).unwrap_err(),
            handler_stat(json!({})).unwrap_err(),
            handler_list(json!({})).unwrap_err(),
        ] {
            assert!(
                err.to_string()
                    .contains("resource_ref: missing required object"),
                "expected ResourceRef-required error, got: {err}"
            );
        }
    }

    // ─── Schema sanity ─────────────────────────────────────

    #[test]
    fn schemas_are_well_formed_objects() {
        for s in [
            input_schema_read(),
            input_schema_write(),
            input_schema_stat(),
            input_schema_list(),
        ] {
            assert_eq!(s["type"], json!("object"));
            assert!(s["properties"]["resource_ref"].is_object());
            assert!(s["required"]
                .as_array()
                .unwrap()
                .contains(&json!("resource_ref")));
            assert!(s["properties"].get("path").is_none());
        }
    }

    #[test]
    fn descriptions_mention_the_profile_name() {
        for d in [
            description_read(),
            description_write(),
            description_stat(),
            description_list(),
        ] {
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

    // ─── fs.write hardening (post-AliveCode audit) ──────────

    #[cfg(unix)]
    #[test]
    fn write_preserves_existing_file_mode() {
        // Caller does NOT pass `mode`. Existing file is 0o600.
        // After overwrite, mode must STILL be 0o600 — overwriting
        // a chmod-600 secret with default-umask 0o644 is the
        // bug we're fixing.
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir();
        let path = dir.join("secret.key");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let resp = handler_write(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Write),
            "content": "new",
            "encoding": "utf8",
        }))
        .unwrap();
        assert_eq!(resp["mode_preserved"], json!(true));

        let final_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(
            final_mode, 0o600,
            "expected 0o600 preserved, got 0o{final_mode:o}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn write_explicit_mode_wins_over_existing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir();
        let path = dir.join("file");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let resp = handler_write(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Write),
            "content": "new",
            "encoding": "utf8",
            "mode": 0o644,
        }))
        .unwrap();
        assert_eq!(resp["mode_preserved"], json!(false));

        let final_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(final_mode, 0o644);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_through_symlink_keeps_link_intact() {
        // Setup: real file + symlink pointing at it. Write
        // through the symlink path. The symlink must SURVIVE,
        // and its target file must be the one updated.
        let dir = temp_dir();
        let real = dir.join("real.txt");
        let link = dir.join("link.txt");
        std::fs::write(&real, "old").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real, &link).unwrap();
        }
        #[cfg(not(unix))]
        {
            // Non-Unix: skip the symlink test — symlink creation
            // requires elevated privileges on Windows.
            std::fs::remove_dir_all(&dir).ok();
            return;
        }

        let resp = handler_write(json!({
            "resource_ref": local_ref(&link, FilesystemResourceCapability::Write),
            "content": "new",
            "encoding": "utf8",
        }))
        .unwrap();

        // Receipt should report the resolved target.
        assert!(resp["resolved_target"].is_string());

        // Real file got the new content.
        let real_content = std::fs::read_to_string(&real).unwrap();
        assert_eq!(real_content, "new");

        // Symlink still exists and still points at real.
        let link_meta = std::fs::symlink_metadata(&link).unwrap();
        assert!(
            link_meta.file_type().is_symlink(),
            "link.txt must remain a symlink after write-through"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_with_matching_expected_mtime_succeeds() {
        let dir = temp_dir();
        let path = dir.join("file.txt");
        std::fs::write(&path, "first").unwrap();
        let mtime = file_mtime_ms(&std::fs::metadata(&path).unwrap()).unwrap();

        let resp = handler_write(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Write),
            "content": "second",
            "encoding": "utf8",
            "expected_mtime_ms": mtime,
        }))
        .unwrap();
        assert_eq!(resp["bytes_written"], json!(6));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_with_stale_expected_mtime_rejects() {
        let dir = temp_dir();
        let path = dir.join("file.txt");
        std::fs::write(&path, "first").unwrap();

        let err = handler_write(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Write),
            "content": "second",
            "encoding": "utf8",
            "expected_mtime_ms": 1u64, // far in the past, will not match
        }))
        .unwrap_err();
        assert!(
            err.to_string().contains("expected_mtime_ms"),
            "expected staleness error, got: {err}"
        );
        // File MUST be unchanged.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_with_expected_mtime_on_missing_file_rejects() {
        let dir = temp_dir();
        let path = dir.join("nope.txt");
        let err = handler_write(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Write),
            "content": "x",
            "encoding": "utf8",
            "expected_mtime_ms": 12345u64,
        }))
        .unwrap_err();
        assert!(err.to_string().contains("does not exist"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_response_carries_mtime_ms() {
        let dir = temp_dir();
        let path = dir.join("file.txt");
        std::fs::write(&path, "x").unwrap();
        let resp = handler_read(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Read),
            "encoding": "utf8",
        }))
        .unwrap();
        // mtime_ms is a non-null integer on every filesystem we
        // run tests on.
        let mtime = resp["mtime_ms"].as_u64();
        assert!(mtime.is_some(), "fs.read should expose mtime_ms");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_then_write_mtime_round_trip() {
        // End-to-end: fs.read captures mtime, fs.write asserts
        // it. The captured value must be the one that satisfies
        // the guard.
        let dir = temp_dir();
        let path = dir.join("file.txt");
        std::fs::write(&path, "v1").unwrap();

        let read = handler_read(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Read),
            "encoding": "utf8",
        }))
        .unwrap();
        let mtime = read["mtime_ms"].as_u64().expect("mtime present");

        let write = handler_write(json!({
            "resource_ref": local_ref(&path, FilesystemResourceCapability::Write),
            "content": "v2",
            "encoding": "utf8",
            "expected_mtime_ms": mtime,
        }))
        .unwrap();
        assert_eq!(write["bytes_written"], json!(2));
        std::fs::remove_dir_all(&dir).ok();
    }

    // ─── helper ────────────────────────────────────────

    #[test]
    fn resolve_symlink_one_level_returns_target_for_symlink() {
        let dir = temp_dir();
        let real = dir.join("r");
        let link = dir.join("l");
        std::fs::write(&real, "x").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&real, &link).unwrap();
            let resolved = resolve_symlink_one_level(&link);
            assert_eq!(resolved, real);
        }
        let resolved_real = resolve_symlink_one_level(&real);
        assert_eq!(resolved_real, real);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn resolve_symlink_one_level_passes_through_non_symlink() {
        let p = std::path::PathBuf::from("/nonexistent/path/x");
        assert_eq!(resolve_symlink_one_level(&p), p);
    }
}
