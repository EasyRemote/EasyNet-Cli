// EasyNet CLI — Pages reference system: fetch ability handler
// ===========================================================
//
// File: src/runtime/agents/pages/fetch.rs
// Description: handler for `<user>.<project_id>.page.fetch` — the
//              read transition of the Resource Execution Model.
//              Reads bytes from the published folder through a
//              kernel-enforced sandbox and returns
//              `{ bytes_b64, content_type, size_bytes,
//                 force_attachment, sha256 }`.
//
// Conformance: RFC-006-B v0.6 §4.2 (fetch = read), INV-3 (Deterministic
//              Projection — output bytes are a deterministic function
//              of (resource identity, path)).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::io::Read;
use std::os::fd::AsFd;
use std::sync::Arc;

use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::runtime::ability_dispatch::LocalAbilityRegistry;

use super::mime::mime_from_path;
use super::sandbox::{open_beneath, validate_regular};
use super::state::PUBLISHED_PROJECTS;

/// args:
/// ```json
/// { "path": "/hello-world.html" }
/// ```
///
/// returns:
/// ```json
/// {
///   "bytes_b64":        "<base64>",
///   "content_type":     "text/html; charset=utf-8",
///   "size_bytes":       173,
///   "force_attachment": false,
///   "sha256":           "<hex>"
/// }
/// ```
///
/// The bytes are base64-encoded so JSON can carry them; the Hub
/// decodes for the HTTP response. `sha256` is computed over the
/// raw bytes and included so future receipt-verifiers can check
/// INV-3 (Deterministic Projection) without re-fetching.
pub fn handle_fetch(user: &str, project_id: &str, args: Value) -> anyhow::Result<Value> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing required arg: path"))?;

    let key = (user.to_string(), project_id.to_string());
    let handle = PUBLISHED_PROJECTS
        .get(&key)
        .ok_or_else(|| anyhow::anyhow!(
            "project not published: user={user} project_id={project_id}"
        ))?
        .clone();

    let mut file = open_beneath(
        handle.folder_fd.as_fd(),
        &handle.canonical_root,
        path,
    )?;
    let size = validate_regular(&file, handle.file_size_cap)?;

    let mut bytes = Vec::with_capacity(size as usize);
    file.read_to_end(&mut bytes)
        .map_err(|e| anyhow::anyhow!("read failed: {e}"))?;

    let mime = mime_from_path(path);
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha = format!("{:x}", hasher.finalize());

    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    Ok(json!({
        "bytes_b64":        b64,
        "content_type":     mime.content_type,
        "size_bytes":       bytes.len(),
        "force_attachment": mime.force_attachment,
        "sha256":           sha,
    }))
}

/// Register `ability/<user>.<project_id>.page.fetch` into the
/// LocalAbilityRegistry. Called by `publish.rs` at publish time.
///
/// Note: the registry is shared as `Arc<LocalAbilityRegistry>` from
/// `OnceLock` (set at daemon boot). To register dynamically we'd
/// need an interior-mutable handle. v0 MVP keeps the registry
/// static after boot; per-publish `page.fetch` registration is
/// implemented as a *resolver fallback* (see `register_resolver`)
/// so the registry doesn't need to be mutated after boot.
///
/// In other words: at boot we register a single fallback resolver
/// that, on lookup miss for any name matching
/// `<user>.<project_id>.page.fetch`, manufactures the handler on
/// the fly by looking up `(user, project_id)` in
/// `PUBLISHED_PROJECTS`. If the project exists, the handler is
/// returned; if not, the registry returns "no such ability" and
/// the dispatcher fails the call as it would for any unknown
/// ability.
pub fn register_fetch_ability(
    _registry: &Arc<LocalAbilityRegistry>,
    _user: &str,
    _project_id: &str,
) {
    // Intentionally a no-op in v0: registration is implicit through
    // the resolver fallback installed at boot
    // (see `super::register_resolver_fallback`). This function exists
    // so call sites read intentionally — when the registry grows a
    // mutable insertion path post-boot, this is where the eager
    // registration goes.
}
