// EasyNet CLI — Files: put / get / list handlers
// ================================================
//
// Three RPC handlers:
//   put — write client-supplied bytes to the content-addressed
//         store; return the canonical URA + sha256.
//   get — read a blob by sha256 or canonical resource URA; return
//         base-64 bytes + content_type.
//   list — list every blob in the store with size + sha256.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

use super::state;

const BLOB_METADATA_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
struct BlobMetadata {
    schema_version: u32,
    sha256: String,
    filename: String,
    content_type: String,
    size: u64,
}

impl BlobMetadata {
    fn new(sha256: String, filename: String, content_type: String, size: usize) -> Self {
        Self {
            schema_version: BLOB_METADATA_SCHEMA_VERSION,
            sha256,
            filename,
            content_type,
            size: size as u64,
        }
    }
}

/// `files.put` — write client-supplied bytes to the
/// content-addressed store.
///
/// args: { filename: string, bytes_b64: string, content_type: string }
/// reply: { ura, sha256, size, content_type }
pub fn handle_put(user: &str, realm: &str, root: &Path, args: Value) -> anyhow::Result<Value> {
    let filename = args
        .get("filename")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("files.put: missing filename"))?
        .to_string();
    let bytes_b64 = args
        .get("bytes_b64")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("files.put: missing bytes_b64"))?;
    let content_type = args
        .get("content_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow::anyhow!("files.put: missing content_type"))?
        .to_string();

    let bytes = STANDARD
        .decode(bytes_b64.as_bytes())
        .map_err(|e| anyhow::anyhow!("files.put: bytes_b64 decode failed: {e}"))?;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha256_hex = hex::encode(hasher.finalize());

    state::ensure_root(root).ok();
    let path = state::blob_path(root, &sha256_hex)
        .map_err(|e| anyhow::anyhow!("files.put: resolve blob path: {e}"))?;
    let metadata = BlobMetadata::new(
        sha256_hex.clone(),
        filename.clone(),
        content_type.clone(),
        bytes.len(),
    );
    ensure_metadata_compatible(root, &metadata)?;
    if !path.exists() {
        let mut f = std::fs::File::create(&path)
            .map_err(|e| anyhow::anyhow!("files.put: create {path:?}: {e}"))?;
        f.write_all(&bytes)
            .map_err(|e| anyhow::anyhow!("files.put: write {path:?}: {e}"))?;
        f.sync_all().ok();
    }
    write_metadata_if_absent(root, &metadata)?;

    let ura = state::blob_ura(realm, user, &sha256_hex);

    Ok(json!({
        "ura": ura,
        "sha256": sha256_hex,
        "size": bytes.len(),
        "content_type": content_type,
        "filename": filename,
    }))
}

/// `files.get` — read a blob.
///
/// args (one of):
///   { sha256: "<hex>" }
///   { ura: "easynet:///r/<realm>/resource/<u>.files/<sha256>" }
///
/// reply: { bytes_b64, content_type, sha256, size }
pub fn handle_get(root: &Path, args: Value) -> anyhow::Result<Value> {
    let sha256_arg = args.get("sha256").and_then(Value::as_str);
    let ura_arg = args.get("ura").and_then(Value::as_str);
    let sha = match (sha256_arg, ura_arg) {
        (Some(s), None) => s.to_string(),
        (None, Some(ura)) => sha256_from_ura(ura)?,
        (Some(_), Some(_)) => anyhow::bail!("files.get: provide exactly one of {{sha256, ura}}"),
        (None, None) => anyhow::bail!("files.get: provide exactly one of {{sha256, ura}}"),
    };

    let blob_path = state::blob_path(root, &sha)
        .map_err(|e| anyhow::anyhow!("files.get: resolve {sha}: {e}"))?;
    let bytes = std::fs::read(&blob_path)
        .map_err(|e| anyhow::anyhow!("files.get: read {blob_path:?}: {e}"))?;
    let bytes_b64 = STANDARD.encode(&bytes);
    let metadata = read_metadata(root, &sha)?;

    Ok(json!({
        "bytes_b64": bytes_b64,
        "content_type": metadata.content_type,
        "filename": metadata.filename,
        "sha256": sha,
        "size": bytes.len(),
    }))
}

/// `files.list` — enumerate blobs in the store.
pub fn handle_list(user: &str, realm: &str, root: &Path, _args: Value) -> anyhow::Result<Value> {
    state::ensure_root(root).ok();
    let mut items = Vec::new();
    if let Ok(rd) = std::fs::read_dir(root) {
        for entry in rd.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.len() != 64 || !name.chars().all(|c| c.is_ascii_hexdigit()) {
                continue;
            }
            let metadata = read_metadata(root, name)?;
            items.push(json!({
                "sha256": name,
                "size": metadata.size,
                "filename": metadata.filename,
                "content_type": metadata.content_type,
                "ura": state::blob_ura(realm, user, name),
            }));
        }
    }
    Ok(json!({ "items": items }))
}

/// Parse the trailing path segment of a v4.1.5 resource URA as
/// the sha256-hex blob id. pub(crate): the chat ability's URA
/// attachments resolve store blobs through this same parse.
pub(crate) fn sha256_from_ura(ura: &str) -> anyhow::Result<String> {
    // easynet:///r/<realm>/resource/<u>.files/<sha256>
    let parsed = crate::core::ura::parse_ura(ura)
        .map_err(|e| anyhow::anyhow!("files: invalid URA `{ura}`: {e}"))?;
    if !matches!(parsed.kind, crate::core::ura::URAKind::Resource) {
        anyhow::bail!("files: URA `{ura}` is not a resource URA");
    }
    let path = parsed
        .resource_path()
        .unwrap_or_default()
        .trim_start_matches('/');
    if path.len() != 64 || !path.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("files: URA path is not a 64-hex sha256: `{path}`");
    }
    Ok(path.to_string())
}

fn ensure_metadata_compatible(root: &Path, next: &BlobMetadata) -> anyhow::Result<()> {
    let metadata_path = state::metadata_path(root, &next.sha256)
        .map_err(|e| anyhow::anyhow!("files.put: resolve metadata path: {e}"))?;
    if !metadata_path.exists() {
        return Ok(());
    }
    let existing = read_metadata(root, &next.sha256)?;
    if existing == *next {
        return Ok(());
    }
    anyhow::bail!(
        "files.put: blob {} already exists with different producer metadata \
         (filename={:?}, content_type={:?})",
        next.sha256,
        existing.filename,
        existing.content_type
    )
}

fn write_metadata_if_absent(root: &Path, metadata: &BlobMetadata) -> anyhow::Result<()> {
    let metadata_path = state::metadata_path(root, &metadata.sha256)
        .map_err(|e| anyhow::anyhow!("files.put: resolve metadata path: {e}"))?;
    if metadata_path.exists() {
        return Ok(());
    }
    let bytes = serde_json::to_vec_pretty(metadata)
        .map_err(|e| anyhow::anyhow!("files.put: encode metadata: {e}"))?;
    let mut f = std::fs::File::create(&metadata_path)
        .map_err(|e| anyhow::anyhow!("files.put: create {metadata_path:?}: {e}"))?;
    f.write_all(&bytes)
        .map_err(|e| anyhow::anyhow!("files.put: write {metadata_path:?}: {e}"))?;
    f.sync_all().ok();
    Ok(())
}

fn read_metadata(root: &Path, sha: &str) -> anyhow::Result<BlobMetadata> {
    let metadata_path = state::metadata_path(root, sha)
        .map_err(|e| anyhow::anyhow!("files.metadata: resolve metadata path: {e}"))?;
    let bytes = std::fs::read(&metadata_path)
        .map_err(|e| anyhow::anyhow!("files.metadata: read {metadata_path:?}: {e}"))?;
    let metadata: BlobMetadata = serde_json::from_slice(&bytes)
        .map_err(|e| anyhow::anyhow!("files.metadata: decode {metadata_path:?}: {e}"))?;
    if metadata.schema_version != BLOB_METADATA_SCHEMA_VERSION {
        anyhow::bail!(
            "files.metadata: unsupported schema_version {} for {}",
            metadata.schema_version,
            sha
        );
    }
    if metadata.sha256 != sha {
        anyhow::bail!(
            "files.metadata: sha256 mismatch for {}: metadata names {}",
            sha,
            metadata.sha256
        );
    }
    if metadata.filename.trim().is_empty() {
        anyhow::bail!("files.metadata: filename is empty for {sha}");
    }
    if metadata.content_type.trim().is_empty() {
        anyhow::bail!("files.metadata: content_type is empty for {sha}");
    }
    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Per-test root — every test gets its own tempdir, no env
    /// mutation, no shared global. Safe under cargo's default
    /// parallel test runner.
    fn fresh_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn put_then_get_round_trips_bytes() {
        let root = fresh_root();
        let body = b"hello files".to_vec();
        let body_b64 = STANDARD.encode(&body);

        let put = handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({
                "filename": "hello.txt",
                "bytes_b64": body_b64,
                "content_type": "text/plain; charset=utf-8",
            }),
        )
        .unwrap();
        let sha = put["sha256"].as_str().unwrap();
        assert_eq!(sha.len(), 64);
        assert_eq!(
            put["ura"].as_str().unwrap(),
            crate::core::ura::resource_dot_ura("test.local", "alice.files", sha)
        );

        let got = handle_get(root.path(), json!({ "sha256": sha })).unwrap();
        assert_eq!(got["sha256"].as_str().unwrap(), sha);
        assert_eq!(got["filename"].as_str(), Some("hello.txt"));
        assert_eq!(
            got["content_type"].as_str(),
            Some("text/plain; charset=utf-8")
        );
        let decoded = STANDARD.decode(got["bytes_b64"].as_str().unwrap()).unwrap();
        assert_eq!(decoded, body);
    }

    #[test]
    fn get_accepts_ura_form() {
        let root = fresh_root();
        let body = b"hello via ura".to_vec();
        let put = handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({
                "filename": "x.bin",
                "bytes_b64": STANDARD.encode(&body),
                "content_type": "application/octet-stream",
            }),
        )
        .unwrap();
        let ura = put["ura"].as_str().unwrap();

        let got = handle_get(root.path(), json!({ "ura": ura })).unwrap();
        let decoded = STANDARD.decode(got["bytes_b64"].as_str().unwrap()).unwrap();
        assert_eq!(decoded, body);
    }

    #[test]
    fn put_is_idempotent_for_identical_metadata() {
        let root = fresh_root();
        let body = b"dedupe me".to_vec();
        let p1 = handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({
                "filename": "a.bin",
                "bytes_b64": STANDARD.encode(&body),
                "content_type": "application/octet-stream",
            }),
        )
        .unwrap();
        let p2 = handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({
                "filename": "a.bin",
                "bytes_b64": STANDARD.encode(&body),
                "content_type": "application/octet-stream",
            }),
        )
        .unwrap();
        assert_eq!(p1["sha256"], p2["sha256"]);
    }

    #[test]
    fn put_rejects_metadata_conflict_for_existing_blob() {
        let root = fresh_root();
        let body = b"dedupe me".to_vec();
        handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({
                "filename": "a.bin",
                "bytes_b64": STANDARD.encode(&body),
                "content_type": "application/octet-stream",
            }),
        )
        .unwrap();

        let error = handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({
                "filename": "b.bin",
                "bytes_b64": STANDARD.encode(&body),
                "content_type": "application/octet-stream",
            }),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("different producer metadata"));
    }

    #[test]
    fn list_returns_items_after_put() {
        let root = fresh_root();
        handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({
                "filename": "a",
                "bytes_b64": STANDARD.encode(b"a"),
                "content_type": "application/octet-stream",
            }),
        )
        .unwrap();
        handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({
                "filename": "b",
                "bytes_b64": STANDARD.encode(b"b"),
                "content_type": "application/octet-stream",
            }),
        )
        .unwrap();
        let listed = handle_list("alice", "test.local", root.path(), json!({})).unwrap();
        let items = listed["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.get("filename").is_some()));
        assert!(items.iter().all(|item| item.get("content_type").is_some()));
    }

    #[test]
    fn put_requires_producer_content_type() {
        let root = fresh_root();
        let error = handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({
                "filename": "hello.txt",
                "bytes_b64": STANDARD.encode(b"hello"),
            }),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("missing content_type"));
    }

    #[test]
    fn put_requires_producer_filename() {
        let root = fresh_root();
        let error = handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({
                "bytes_b64": STANDARD.encode(b"hello"),
                "content_type": "text/plain",
            }),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("missing filename"));
    }

    #[test]
    fn get_rejects_retired_path_selector() {
        let root = fresh_root();
        let error = handle_get(root.path(), json!({ "path": "a".repeat(64) })).unwrap_err();

        assert!(format!("{error:#}").contains("sha256, ura"));
    }

    #[test]
    fn get_requires_one_canonical_selector() {
        let root = fresh_root();
        let error = handle_get(
            root.path(),
            json!({
                "sha256": "a".repeat(64),
                "ura": crate::core::ura::resource_dot_ura(
                    "test.local",
                    "alice.files",
                    &"a".repeat(64)
                ),
            }),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("exactly one"));
    }

    #[test]
    fn get_requires_persisted_metadata() {
        let root = fresh_root();
        let sha = hex::encode(Sha256::digest(b"orphan"));
        std::fs::write(state::blob_path(root.path(), &sha).unwrap(), b"orphan").unwrap();

        let error = handle_get(root.path(), json!({ "sha256": sha })).unwrap_err();

        assert!(format!("{error:#}").contains("files.metadata"));
    }

    #[test]
    fn list_requires_persisted_metadata() {
        let root = fresh_root();
        let sha = hex::encode(Sha256::digest(b"orphan"));
        std::fs::write(state::blob_path(root.path(), &sha).unwrap(), b"orphan").unwrap();

        let error = handle_list("alice", "test.local", root.path(), json!({})).unwrap_err();

        assert!(format!("{error:#}").contains("files.metadata"));
    }
}
