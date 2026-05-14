// EasyNet CLI — Files: put / get / list handlers
// ================================================
//
// Three RPC handlers:
//   put — write client-supplied bytes to the content-addressed
//         store; return the canonical URA + sha256.
//   get — read a blob by sha256; return base-64 bytes +
//         content_type. Accepts a v4.1.5 URA in `uri` field as
//         shorthand for the sha256 (parses out the trailing path
//         segment).
//   list — list every blob in the store with size + sha256.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;

use super::state;

/// Heuristic content-type from filename extension. Matches the
/// MIME table Pages uses (`pages/mime.rs`); duplicated here to
/// keep the modules independent. Defaults to
/// `application/octet-stream` for anything not in the allow-list.
fn mime_from_filename(filename: &str) -> &'static str {
    let lower = filename.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain; charset=utf-8",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// `<user>.files.put` — write client-supplied bytes to the
/// content-addressed store.
///
/// args: { filename: string, bytes_b64: string, content_type?: string }
/// reply: { uri, sha256, size, content_type }
pub fn handle_put(user: &str, realm: &str, root: &Path, args: Value) -> anyhow::Result<Value> {
    let filename = args
        .get("filename")
        .and_then(Value::as_str)
        .unwrap_or("blob")
        .to_string();
    let bytes_b64 = args
        .get("bytes_b64")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("files.put: missing bytes_b64"))?;
    let provided_ct = args
        .get("content_type")
        .and_then(Value::as_str)
        .map(str::to_string);

    let bytes = STANDARD
        .decode(bytes_b64.as_bytes())
        .map_err(|e| anyhow::anyhow!("files.put: bytes_b64 decode failed: {e}"))?;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha256_hex = hex::encode(hasher.finalize());

    state::ensure_root(root).ok();
    let path = state::blob_path(root, &sha256_hex)
        .map_err(|e| anyhow::anyhow!("files.put: resolve blob path: {e}"))?;
    if !path.exists() {
        let mut f = std::fs::File::create(&path)
            .map_err(|e| anyhow::anyhow!("files.put: create {path:?}: {e}"))?;
        f.write_all(&bytes)
            .map_err(|e| anyhow::anyhow!("files.put: write {path:?}: {e}"))?;
        f.sync_all().ok();
    }

    let content_type = provided_ct.unwrap_or_else(|| mime_from_filename(&filename).to_string());
    let uri = state::blob_uri(realm, user, &sha256_hex);

    Ok(json!({
        "uri": uri,
        "sha256": sha256_hex,
        "size": bytes.len(),
        "content_type": content_type,
        "filename": filename,
    }))
}

/// `<user>.files.get` — read a blob.
///
/// args (one of):
///   { sha256: "<hex>" }
///   { uri: "easynet:///r/<realm>/resource/<u>.files/<sha256>" }
///   { path: "<sha256>" }     // RFC-006-B page.fetch shape compat
///
/// reply: { bytes_b64, content_type, sha256, size }
pub fn handle_get(root: &Path, args: Value) -> anyhow::Result<Value> {
    let sha = if let Some(s) = args.get("sha256").and_then(Value::as_str) {
        s.to_string()
    } else if let Some(uri) = args.get("uri").and_then(Value::as_str) {
        sha256_from_uri(uri)?
    } else if let Some(path) = args.get("path").and_then(Value::as_str) {
        // page.fetch-shape compat: leading slash optional, trailing
        // path becomes the sha256.
        path.trim_start_matches('/').to_string()
    } else {
        anyhow::bail!("files.get: provide one of {{sha256, uri, path}}");
    };

    let blob_path = state::blob_path(root, &sha)
        .map_err(|e| anyhow::anyhow!("files.get: resolve {sha}: {e}"))?;
    let bytes = std::fs::read(&blob_path)
        .map_err(|e| anyhow::anyhow!("files.get: read {blob_path:?}: {e}"))?;
    let bytes_b64 = STANDARD.encode(&bytes);

    // Sniff content-type from leading bytes (very narrow set; the
    // put-side already records the operator-supplied type, but
    // content-addressed storage doesn't preserve it across blobs
    // with differing names).
    let content_type = sniff_content_type(&bytes);

    Ok(json!({
        "bytes_b64": bytes_b64,
        "content_type": content_type,
        "sha256": sha,
        "size": bytes.len(),
    }))
}

/// `<user>.files.list` — enumerate blobs in the store.
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
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            items.push(json!({
                "sha256": name,
                "size": size,
                "uri": state::blob_uri(realm, user, name),
            }));
        }
    }
    Ok(json!({ "items": items }))
}

/// Parse the trailing path segment of a v4.1.5 resource URA as
/// the sha256-hex blob id.
fn sha256_from_uri(uri: &str) -> anyhow::Result<String> {
    // easynet:///r/<realm>/resource/<u>.files/<sha256>
    let parsed = crate::ura::parse_ura(uri)
        .map_err(|e| anyhow::anyhow!("files: invalid URA `{uri}`: {e}"))?;
    if !matches!(parsed.kind, crate::ura::URAKind::Resource) {
        anyhow::bail!("files: URA `{uri}` is not a resource URA");
    }
    let path = parsed.path.trim_start_matches('/');
    if path.len() != 64 || !path.chars().all(|c| c.is_ascii_hexdigit()) {
        anyhow::bail!("files: URA path is not a 64-hex sha256: `{path}`");
    }
    Ok(path.to_string())
}

/// Tiny magic-byte sniffer covering the MIME types most likely to
/// flow through `image_url` / `file` chat content blocks. Falls
/// back to `application/octet-stream` for anything else.
fn sniff_content_type(bytes: &[u8]) -> &'static str {
    if bytes.len() >= 4 && bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return "image/png";
    }
    if bytes.len() >= 3 && bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "image/jpeg";
    }
    if bytes.len() >= 6 && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a") {
        return "image/gif";
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp";
    }
    if bytes.len() >= 4 && bytes.starts_with(b"%PDF") {
        return "application/pdf";
    }
    if bytes.starts_with(b"<svg") || bytes.starts_with(b"<?xml") {
        return "image/svg+xml";
    }
    "application/octet-stream"
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
            }),
        )
        .unwrap();
        let sha = put["sha256"].as_str().unwrap();
        assert_eq!(sha.len(), 64);
        assert_eq!(
            put["uri"].as_str().unwrap(),
            crate::ura::resource_dot_ura("test.local", "alice.files", sha)
        );

        let got = handle_get(root.path(), json!({ "sha256": sha })).unwrap();
        assert_eq!(got["sha256"].as_str().unwrap(), sha);
        let decoded = STANDARD.decode(got["bytes_b64"].as_str().unwrap()).unwrap();
        assert_eq!(decoded, body);
    }

    #[test]
    fn get_accepts_uri_form() {
        let root = fresh_root();
        let body = b"hello via uri".to_vec();
        let put = handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({
                "filename": "x.bin",
                "bytes_b64": STANDARD.encode(&body),
            }),
        )
        .unwrap();
        let uri = put["uri"].as_str().unwrap();

        let got = handle_get(root.path(), json!({ "uri": uri })).unwrap();
        let decoded = STANDARD.decode(got["bytes_b64"].as_str().unwrap()).unwrap();
        assert_eq!(decoded, body);
    }

    #[test]
    fn put_dedupes_identical_bytes() {
        let root = fresh_root();
        let body = b"dedupe me".to_vec();
        let p1 = handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({"filename": "a", "bytes_b64": STANDARD.encode(&body)}),
        )
        .unwrap();
        let p2 = handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({"filename": "b", "bytes_b64": STANDARD.encode(&body)}),
        )
        .unwrap();
        assert_eq!(p1["sha256"], p2["sha256"]);
    }

    #[test]
    fn list_returns_items_after_put() {
        let root = fresh_root();
        handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({"filename": "a", "bytes_b64": STANDARD.encode(b"a")}),
        )
        .unwrap();
        handle_put(
            "alice",
            "test.local",
            root.path(),
            json!({"filename": "b", "bytes_b64": STANDARD.encode(b"b")}),
        )
        .unwrap();
        let listed = handle_list("alice", "test.local", root.path(), json!({})).unwrap();
        let items = listed["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn sniff_png_jpeg_pdf() {
        assert_eq!(
            sniff_content_type(&[0x89, b'P', b'N', b'G', 0, 0, 0, 0]),
            "image/png"
        );
        assert_eq!(sniff_content_type(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        assert_eq!(sniff_content_type(b"%PDF-1.4"), "application/pdf");
        assert_eq!(sniff_content_type(b"hello"), "application/octet-stream");
    }
}
