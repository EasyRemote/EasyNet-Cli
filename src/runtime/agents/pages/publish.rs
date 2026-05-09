// EasyNet CLI — Pages reference system: publish ability handler
// =============================================================
//
// File: src/runtime/agents/pages/publish.rs
// Description: handler for `<user>.pages.publish` — the canonical
//              create transition of the Resource Execution Model
//              applied to a folder of static bytes.
//
//              Effect: opens the folder fd, mints the resource URA
//              `resource/<user>.<project_id>`, registers the
//              project's `<user>.<project_id>.page.fetch` ability
//              into the global registry, and inserts a
//              `ProjectHandle` into `PUBLISHED_PROJECTS`.
//
// Conformance: RFC-006-B v0.6 §4.1 (publish = create), INV-2 v0
//              transitional (project_id is user-defined).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Context;
use serde_json::{json, Value};

use crate::runtime::ability_dispatch::LocalAbilityRegistry;

use super::fetch::register_fetch_ability;
use super::sandbox::open_directory;
use super::state::{
    persist_registry_for_user, ProjectHandle, Visibility, DEFAULT_FILE_SIZE_CAP, PUBLISHED_PROJECTS,
};

/// Handler invoked when a user calls `<self>.pages.publish`. The
/// handler is registered once at daemon boot under the user-prefixed
/// ability name; the dispatcher routes per-user calls to the same
/// closure (`<self>` is the calling daemon's user identity).
///
/// args:
/// ```json
/// {
///   "folder":     "/absolute/path/to/folder",
///   "project_id": "papers",
///   "visibility": "public"
/// }
/// ```
///
/// returns:
/// ```json
/// {
///   "project_uri": "easynet:///r/<realm>/resource/<user>.<project_id>/",
///   "url_root":    "https://<realm>/web/<user>/<project_id>/"
/// }
/// ```
///
/// `url_root` is the production Hub URL — see
/// `pages_public_url_root` in `pages/mod.rs`. The daemon's in-process
/// HTTP listener URL (`http://<project>.<user>.pages.localhost:<port>/`)
/// is dev-only and is only surfaced by `<user>.pages.get` for
/// debugging.
pub fn handle_publish(
    user: &str,
    // Kept in the signature for ABI stability with the resolver
    // wiring in `pages/mod.rs`. The publish surface no longer
    // depends on the daemon's in-process listener — `url_root` is
    // built from `realm` via `pages_public_url_root`. The dev-only
    // listener URL is reported from `<user>.pages.get` (which still
    // takes the port) when an operator wants it for local curl.
    _listener_port: u16,
    realm: &str,
    registry: Arc<LocalAbilityRegistry>,
    args: Value,
) -> anyhow::Result<Value> {
    let folder_str = args
        .get("folder")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing required arg: folder"))?;
    let project_id = args
        .get("project_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing required arg: project_id"))?;
    let visibility_str = args
        .get("visibility")
        .and_then(Value::as_str)
        .unwrap_or("public");

    validate_project_id(project_id)?;
    let visibility = Visibility::parse(visibility_str)?;

    let folder = PathBuf::from(folder_str);
    if !folder.is_absolute() {
        anyhow::bail!("folder must be an absolute path: {folder_str}");
    }
    if !folder.exists() {
        anyhow::bail!("folder does not exist: {folder_str}");
    }
    if !folder.is_dir() {
        anyhow::bail!("folder is not a directory: {folder_str}");
    }
    let canonical_root =
        std::fs::canonicalize(&folder).map_err(|e| anyhow::anyhow!("canonicalize failed: {e}"))?;

    let key = (user.to_string(), project_id.to_string());
    if PUBLISHED_PROJECTS.contains_key(&key) {
        anyhow::bail!(
            "project already published: user={user} project_id={project_id} \
             (call <self>.pages.unpublish first)"
        );
    }

    let folder_fd = open_directory(&canonical_root)?;

    let handle = Arc::new(ProjectHandle {
        user: user.to_string(),
        project_id: project_id.to_string(),
        folder_fd,
        canonical_root: canonical_root.clone(),
        visibility,
        file_size_cap: DEFAULT_FILE_SIZE_CAP,
        started_at: SystemTime::now(),
    });

    PUBLISHED_PROJECTS.insert(key.clone(), handle.clone());
    if let Err(err) = persist_registry_for_user(user) {
        PUBLISHED_PROJECTS.remove(&key);
        return Err(err).context("persist pages publish registry");
    }

    // Register the per-project fetch ability into the live registry.
    // The registration function is owned by `fetch.rs` so the
    // ability-name format and the closure body live next to each
    // other.
    register_fetch_ability(&registry, user, project_id);

    let project_uri = format!("easynet:///r/{realm}/resource/{user}.{project_id}/");
    let url_root = super::pages_public_url_root(realm, user, project_id);

    Ok(json!({
        "project_uri": project_uri,
        "url_root":    url_root,
        "user":        user,
        "project_id":  project_id,
        "visibility":  visibility.as_str(),
    }))
}

/// project_id grammar: URA-safe segment per RFC-006-B v0.6 §2.1.
/// `[a-zA-Z0-9_-]+`, max 64 chars. Forbid '.' so `<user>.<project>`
/// always has exactly two dot-separated components.
fn validate_project_id(project_id: &str) -> anyhow::Result<()> {
    if project_id.is_empty() {
        anyhow::bail!("project_id is empty");
    }
    if project_id.len() > 64 {
        anyhow::bail!("project_id too long: {} > 64", project_id.len());
    }
    if !project_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        anyhow::bail!(
            "project_id contains invalid character; allowed: a-zA-Z0-9_- (got {project_id:?})"
        );
    }
    Ok(())
}
