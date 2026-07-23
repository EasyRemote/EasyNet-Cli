// EasyNet CLI — Pages reference system: fetch ability handler
// ===========================================================
//
// File: src/daemon/ability/builtins/resources/pages/fetch.rs
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
use std::sync::Arc;

use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::daemon::ability::dispatch::{AxonAbilityCatalog, OwnerKind};
use crate::daemon::ability::AuthorityScope;
use crate::daemon::resources::projection::PagesFetchResponse;

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
        .ok_or_else(|| {
            anyhow::anyhow!("project not published: user={user} project_id={project_id}")
        })?
        .clone();

    let mut file = open_beneath(&handle.folder_handle, &handle.canonical_root, path)?;
    let size = validate_regular(&file, handle.file_size_cap)?;

    let mut bytes = Vec::with_capacity(size as usize);
    file.read_to_end(&mut bytes)
        .map_err(|e| anyhow::anyhow!("read failed: {e}"))?;

    let mime = mime_from_path(path);
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let sha = format!("{:x}", hasher.finalize());

    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

    Ok(serde_json::to_value(PagesFetchResponse::success(
        b64,
        mime.content_type,
        bytes.len(),
        mime.force_attachment,
        sha,
    ))?)
}

pub(crate) fn fetch_ability_name(user: &str, project_id: &str) -> String {
    format!("{user}.{project_id}.page.fetch")
}

fn fetch_ability_manifest() -> crate::daemon::ability::manifest::AbilityManifest {
    crate::daemon::ability::manifest::AbilityManifest::new(
        "fetch",
        "Fetch a published Pages project asset by sandboxed path.",
        json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {"type": "string"}
            },
            "additionalProperties": false
        }),
    )
    .and_then(|manifest| manifest.with_admission_action("read"))
    .expect("dynamic Pages fetch manifest is well-formed")
}

/// Register `<user>.<project_id>.page.fetch` into the daemon-hosted
/// Axon runtime. Called by `publish.rs` at publish time and by
/// `pages::register` after restart restore.
pub fn register_fetch_ability(
    registry: &AxonAbilityCatalog,
    user: &str,
    project_id: &str,
    authority_scope: AuthorityScope,
) -> anyhow::Result<()> {
    let ability = fetch_ability_name(user, project_id);
    let owner = OwnerKind::User(user.to_string());
    let user = user.to_string();
    let project_id = project_id.to_string();
    registry.hot_register_rpc_with_spec_and_authority_scope(
        ability,
        owner,
        authority_scope,
        fetch_ability_manifest(),
        Arc::new(move |args| handle_fetch(&user, &project_id, args)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::SystemTime;

    fn publish_fetch_test_project(
        user: &str,
        project_id: &str,
    ) -> (tempfile::TempDir, (String, String)) {
        let root = tempfile::tempdir().expect("temp pages fetch root");
        let canonical_root = std::fs::canonicalize(root.path()).expect("canonical pages root");
        let folder_handle =
            crate::daemon::ability::builtins::resources::pages::sandbox::open_directory(
                &canonical_root,
            )
            .expect("open pages fetch test root");
        let key = (user.to_string(), project_id.to_string());
        PUBLISHED_PROJECTS.insert(
            key.clone(),
            Arc::new(super::super::state::ProjectHandle {
                user: user.to_string(),
                project_id: project_id.to_string(),
                folder_handle,
                canonical_root,
                visibility: super::super::state::PageVisibility::Public,
                file_size_cap: super::super::state::DEFAULT_FILE_SIZE_CAP,
                started_at: SystemTime::UNIX_EPOCH,
            }),
        );
        (root, key)
    }

    #[test]
    fn handle_fetch_returns_typed_payload_projection_shape() {
        let user = "pages-fetch-projection-user";
        let project_id = "docs-fetch";
        let (root, key) = publish_fetch_test_project(user, project_id);
        std::fs::write(root.path().join("index.html"), "<h1>Hello</h1>").expect("write test page");

        let fetched = handle_fetch(user, project_id, json!({"path": "/index.html"})).unwrap();
        PUBLISHED_PROJECTS.remove(&key);

        assert_eq!(fetched["content_type"], "text/html; charset=utf-8");
        assert_eq!(fetched["size_bytes"], 14);
        assert_eq!(fetched["force_attachment"], false);
        assert_eq!(
            fetched["sha256"],
            "e2c6c0ea7c7900c31f953e48d30d5e839801ab90630d751e7c8426ed5859da47"
        );
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(fetched["bytes_b64"].as_str().expect("bytes_b64 string"))
            .expect("decode response bytes");
        assert_eq!(decoded, b"<h1>Hello</h1>");
        assert!(fetched.get("local_path").is_none());
    }
}
