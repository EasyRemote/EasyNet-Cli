// EasyNet CLI — Pages reference system: publish ability handler
// =============================================================
//
// File: src/daemon/ability/builtins/resources/pages/publish.rs
// Description: handler for `pages.publish` — the canonical
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
use serde_json::Value;

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::resources::projection::PagesPublishResponse;

use super::sandbox::open_directory;
use super::state::{
    persist_registry_for_user, PageVisibility, ProjectHandle, DEFAULT_FILE_SIZE_CAP,
    PUBLISHED_PROJECTS,
};

/// Handler invoked when a user calls `pages.publish`. The
/// handler is registered once at daemon boot under the user-prefixed
/// ability name; the dispatcher routes per-user calls to the same
/// closure with the calling daemon's explicit user identity.
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
///   "project_ura": "easynet:///r/<realm>/resource/<user>.<project_id>/",
///   "url_root":    "https://<realm>/web/<user>/<project_id>/"
/// }
/// ```
///
/// `url_root` is the production Hub URL — see
/// `pages_public_url_root` in `pages/mod.rs`. The daemon's in-process
/// HTTP listener URL (`http://<project>.<user>.pages.localhost:<port>/`)
/// is dev-only and is only surfaced by `pages.get` for
/// debugging.
pub fn handle_publish(
    owner_user_id: &str,
    user: &str,
    // The publish surface no longer depends on the daemon's
    // in-process listener — `url_root` is built from `realm` via
    // `pages_public_url_root`. The dev-only listener URL is reported
    // from `pages.get` (which still takes the port) when an
    // operator wants it for local curl.
    _listener_port: u16,
    realm: &str,
    registry: Arc<AxonAbilityCatalog>,
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
    let visibility = PageVisibility::parse(visibility_str)?;

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
             (call pages.unpublish first)"
        );
    }

    let folder_handle = open_directory(&canonical_root)?;

    let handle = Arc::new(ProjectHandle {
        user: user.to_string(),
        project_id: project_id.to_string(),
        folder_handle,
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

    // Register per-project fetch/API abilities into the live
    // daemon-hosted Axon runtime so Hub remote/session dispatch
    // can find them without any legacy resolver path.
    super::register_project_abilities(registry.as_ref(), owner_user_id, user, project_id)
        .context("register pages project abilities")?;

    let project_ura =
        crate::core::ura::resource_dot_ura(realm, &format!("{user}.{project_id}"), "/");
    let url_root = super::pages_public_url_root(realm, user, project_id);

    Ok(serde_json::to_value(PagesPublishResponse::success(
        project_ura,
        url_root,
        user,
        project_id,
        visibility.as_str(),
    ))?)
}

/// project_id grammar: URA-safe segment per RFC-006-B v0.6 §2.1.
/// `[a-zA-Z0-9_-]+`, max 64 chars. Forbid '.' so `<user>.<project>`
/// always has exactly two dot-separated components.
pub(super) fn validate_project_id(project_id: &str) -> anyhow::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ability::dispatch::{AbilityAuthorityContext, AxonAbilityCatalog};
    use serde_json::json;

    fn pages_registry(realm: &str, _user: &str) -> Arc<AxonAbilityCatalog> {
        let device_ura = crate::core::ura::device_ura(realm, "pages-publish-test-device");
        let authority_context = AbilityAuthorityContext::for_device_authority_root(device_ura)
            .expect("Pages publish test Device authority");
        Arc::new(AxonAbilityCatalog::new_with_runtime_and_authority_context(
            crate::daemon::axon_bridge::runtime_factory::build_local_runtime(
                crate::daemon::axon_bridge::runtime_factory::rejecting_test_key_resolver(),
                None,
            ),
            authority_context,
        ))
    }

    fn clear_registry_for_user(user: &str) {
        let keys: Vec<_> = PUBLISHED_PROJECTS
            .iter()
            .filter_map(|entry| {
                let key = entry.key();
                (key.0 == user).then(|| key.clone())
            })
            .collect();
        for key in keys {
            PUBLISHED_PROJECTS.remove(&key);
        }
    }

    #[test]
    fn handle_publish_returns_typed_payload_projection_shape() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let realm = "easynet.run";
        let user = "pages-publish-projection-user";
        let owner_user_id = "pages-publish-owner";
        let project_id = "docs-publish";
        clear_registry_for_user(user);
        let folder = tempfile::tempdir().expect("temp pages publish root");
        std::fs::write(folder.path().join("index.html"), "<h1>Hello</h1>")
            .expect("write test page");

        let published = handle_publish(
            owner_user_id,
            user,
            8787,
            realm,
            pages_registry(realm, user),
            json!({
                "folder": folder.path().display().to_string(),
                "project_id": project_id,
                "visibility": "public",
            }),
        )
        .expect("publish test project");
        clear_registry_for_user(user);

        assert_eq!(
            published["project_ura"],
            "easynet:///r/easynet.run/resource/pages-publish-projection-user.docs-publish"
        );
        assert_eq!(
            published["url_root"],
            "https://easynet.run/web/pages-publish-projection-user/docs-publish/"
        );
        assert_eq!(published["user"], user);
        assert_eq!(published["project_id"], project_id);
        assert_eq!(published["visibility"], "public");
        assert!(published.get("folder").is_none());
        assert!(published.get("canonical_root").is_none());
    }
}
