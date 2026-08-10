// EasyNet CLI — Pages reference system: list / get / unpublish handlers
// =====================================================================
//
// File: src/daemon/ability/builtins/resources/pages/list_get_unpublish.rs
// Description: three remaining ability handlers for the Pages
//              reference system.
//
//                project_list       (operational, read)
//                pages.get        (operational, read)
//                pages.unpublish  (canonical,    delete)
//
// Conformance: RFC-006-B v0.6 §4.3 (unpublish = delete);
//              list/get are introspection abilities not formally
//              part of the three-transition core but required
//              by the CLI surface.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::Value;

use anyhow::Context;

use crate::daemon::ability::dispatch::AxonAbilityCatalog;
use crate::daemon::resources::projection::{
    PagesHealthCheck, PagesHealthResponse, PagesProjectDetailResponse, PagesProjectListItem,
    PagesProjectListResponse, PagesUnpublishResponse,
};

use super::state::{persist_registry_for_user, ProjectHandle, PUBLISHED_PROJECTS};

/// `project_list` — return every project the daemon
/// currently hosts under this user. `url_root` is the production
/// Hub URL; `dev_listener_url_root` is the daemon's local listener
/// URL, which is the one that actually opens during local dev.
pub fn handle_list(
    user: &str,
    listener_port: u16,
    realm: &str,
    _args: Value,
) -> anyhow::Result<Value> {
    let mut entries = Vec::new();
    for entry in PUBLISHED_PROJECTS.iter() {
        let (k_user, project_id) = entry.key();
        if k_user != user {
            continue;
        }
        entries.push(project_list_item(
            k_user,
            project_id,
            entry.value(),
            listener_port,
            realm,
        ));
    }
    Ok(serde_json::to_value(
        PagesProjectListResponse::from_projects(entries),
    )?)
}

/// `pages.get` — return one project's detail.
pub fn handle_get(
    user: &str,
    listener_port: u16,
    realm: &str,
    args: Value,
) -> anyhow::Result<Value> {
    let project_id = args
        .get("project_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing required arg: project_id"))?;
    let key = (user.to_string(), project_id.to_string());
    let h = PUBLISHED_PROJECTS
        .get(&key)
        .ok_or_else(|| anyhow::anyhow!("project not found: user={user} project_id={project_id}"))?
        .clone();

    Ok(serde_json::to_value(project_detail_response(
        user,
        project_id,
        &h,
        listener_port,
        realm,
    ))?)
}

/// `pages.health` — report daemon-owned Pages registry readiness.
///
/// This is deliberately narrower than backend/public-route health: it proves
/// the daemon pages management ability family is loaded and, when requested,
/// that one published project is present in the local registry.
pub fn handle_health(
    owner_user_id: &str,
    user: &str,
    realm: &str,
    args: Value,
) -> anyhow::Result<Value> {
    let surface_ref = args.get("surface_ref").and_then(Value::as_str);
    let project_id = args
        .get("project_id")
        .and_then(Value::as_str)
        .or_else(|| surface_ref.and_then(|value| project_id_from_surface_ref(user, value)));
    if surface_ref.is_some() && project_id.is_none() {
        anyhow::bail!("surface_ref does not target this user's pages resource");
    }
    if let Some(project_id) = project_id {
        super::publish::validate_project_id(project_id)?;
    }
    let mut page_count = 0usize;
    let mut project_found = project_id.is_none();
    for entry in PUBLISHED_PROJECTS.iter() {
        let (entry_user, entry_project_id) = entry.key();
        if entry_user != user {
            continue;
        }
        page_count += 1;
        if project_id.is_some_and(|target| target == entry_project_id) {
            project_found = true;
        }
    }
    let ready = project_found;
    let state = if ready { "ready" } else { "degraded" };
    let owner_ura = super::management_agent_ura(realm, owner_user_id);
    let surface_ref = project_id
        .map(|project_id| {
            crate::core::ura::resource_dot_ura(realm, &format!("{user}.{project_id}"), "/")
        })
        .unwrap_or_else(|| {
            crate::core::ura::resource_dot_ura(realm, &format!("{user}.pages"), "/")
        });
    Ok(serde_json::to_value(PagesHealthResponse::new(
        state,
        ready,
        owner_ura,
        surface_ref,
        page_count,
        vec![
            PagesHealthCheck::pages_registry(),
            PagesHealthCheck::project(project_id, project_found),
        ],
    ))?)
}

/// `pages.unpublish` — remove the project. Drops the
/// `ProjectHandle` (releasing the folder fd) and removes the
/// entry from `PUBLISHED_PROJECTS`, then rewrites the user's
/// restart snapshot. Ability-path unpublish also removes the
/// project's hot-registered fetch/API abilities from LocalRuntime.
pub fn handle_unpublish(user: &str, args: Value) -> anyhow::Result<Value> {
    handle_unpublish_inner(user, None, args)
}

pub fn handle_unpublish_with_registry(
    user: &str,
    registry: &AxonAbilityCatalog,
    args: Value,
) -> anyhow::Result<Value> {
    handle_unpublish_inner(user, Some(registry), args)
}

fn handle_unpublish_inner(
    user: &str,
    registry: Option<&AxonAbilityCatalog>,
    args: Value,
) -> anyhow::Result<Value> {
    let project_id = args
        .get("project_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing required arg: project_id"))?;
    let key = (user.to_string(), project_id.to_string());
    let ability_names = registry
        .map(|registry| super::registered_project_ability_names(registry, user, project_id))
        .unwrap_or_default();
    let removed = PUBLISHED_PROJECTS.remove(&key);
    let Some((_removed_key, removed_handle)) = removed else {
        anyhow::bail!("project not found: user={user} project_id={project_id}");
    };
    if let Err(err) = persist_registry_for_user(user) {
        PUBLISHED_PROJECTS.insert(key, removed_handle);
        return Err(err).context("persist pages publish registry");
    }
    if let Some(registry) = registry {
        super::unregister_project_abilities(registry, ability_names)
            .context("unregister pages project abilities")?;
    }
    Ok(serde_json::to_value(PagesUnpublishResponse::success(
        user, project_id,
    ))?)
}

fn project_started_at_ms(handle: &ProjectHandle) -> u64 {
    handle
        .started_at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn project_list_item(
    user: &str,
    project_id: &str,
    handle: &ProjectHandle,
    listener_port: u16,
    realm: &str,
) -> PagesProjectListItem {
    PagesProjectListItem::new(
        user,
        project_id,
        handle.canonical_root.display().to_string(),
        handle.visibility.as_str(),
        project_started_at_ms(handle),
        super::pages_public_url_root(realm, user, project_id),
        super::pages_dev_listener_url_root(user, project_id, listener_port),
    )
}

fn project_detail_response(
    user: &str,
    project_id: &str,
    handle: &ProjectHandle,
    listener_port: u16,
    realm: &str,
) -> PagesProjectDetailResponse {
    PagesProjectDetailResponse::success(
        user,
        project_id,
        crate::core::ura::resource_dot_ura(realm, &format!("{user}.{project_id}"), "/"),
        handle.canonical_root.display().to_string(),
        handle.visibility.as_str(),
        project_started_at_ms(handle),
        super::pages_public_url_root(realm, user, project_id),
        super::pages_dev_listener_url_root(user, project_id, listener_port),
        handle.file_size_cap,
    )
}

fn project_id_from_surface_ref<'a>(user: &str, raw: &'a str) -> Option<&'a str> {
    let marker = format!("resource/{user}.");
    let (_, tail) = raw.split_once(&marker)?;
    let project = tail.split('/').next().unwrap_or(tail);
    (!project.is_empty() && !project.contains('.')).then_some(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};

    fn publish_test_project(user: &str, project_id: &str) -> (tempfile::TempDir, (String, String)) {
        let root = tempfile::tempdir().expect("temp pages root");
        let canonical_root = std::fs::canonicalize(root.path()).expect("canonical pages root");
        let folder_handle =
            crate::daemon::ability::builtins::resources::pages::sandbox::open_directory(
                &canonical_root,
            )
            .expect("open test pages root");
        let key = (user.to_string(), project_id.to_string());
        PUBLISHED_PROJECTS.insert(
            key.clone(),
            Arc::new(ProjectHandle {
                user: user.to_string(),
                project_id: project_id.to_string(),
                folder_handle,
                canonical_root,
                visibility: super::super::state::PageVisibility::Public,
                file_size_cap: super::super::state::DEFAULT_FILE_SIZE_CAP,
                started_at: UNIX_EPOCH + Duration::from_millis(123),
            }),
        );
        (root, key)
    }

    fn remove_test_project(key: &(String, String)) {
        PUBLISHED_PROJECTS.remove(key);
    }

    #[test]
    fn handle_list_returns_typed_project_projection_shape() {
        let user = "pages-list-projection-user";
        let (_root, key) = publish_test_project(user, "docs-list");

        let listed = handle_list(user, 8787, "example", json!({})).unwrap();
        remove_test_project(&key);

        let projects = listed["projects"].as_array().expect("projects array");
        let project = projects
            .iter()
            .find(|project| project["project_id"] == "docs-list")
            .expect("test project listed");
        assert_eq!(project["user"], user);
        assert_eq!(project["visibility"], "public");
        assert_eq!(project["started_at_ms"], 123);
        assert_eq!(
            project["url_root"],
            "https://example/web/pages-list-projection-user/docs-list/"
        );
        assert_eq!(
            project["dev_listener_url_root"],
            "http://docs-list.pages-list-projection-user.pages.localhost:8787/"
        );
        assert!(project.get("file_size_cap").is_none());
        assert!(project.get("project_ura").is_none());
    }

    #[test]
    fn handle_get_returns_typed_project_detail_shape() {
        let user = "pages-get-projection-user";
        let (_root, key) = publish_test_project(user, "docs-get");

        let detail = handle_get(user, 8787, "example", json!({"project_id": "docs-get"})).unwrap();
        remove_test_project(&key);

        assert_eq!(detail["user"], user);
        assert_eq!(detail["project_id"], "docs-get");
        assert_eq!(
            detail["project_ura"],
            "easynet:///r/example/resource/pages-get-projection-user.docs-get"
        );
        assert_eq!(detail["visibility"], "public");
        assert_eq!(detail["started_at_ms"], 123);
        assert_eq!(
            detail["url_root"],
            "https://example/web/pages-get-projection-user/docs-get/"
        );
        assert_eq!(
            detail["dev_listener_url_root"],
            "http://docs-get.pages-get-projection-user.pages.localhost:8787/"
        );
        assert_eq!(
            detail["file_size_cap"],
            super::super::state::DEFAULT_FILE_SIZE_CAP
        );
    }

    #[test]
    fn handle_unpublish_returns_typed_receipt_and_removes_project() {
        let _home = crate::cli::commands::test_support::HomeGuard::new();
        let user = "pages-unpublish-projection-user";
        let (_root, key) = publish_test_project(user, "docs-unpublish");

        let removed = handle_unpublish(user, json!({"project_id": "docs-unpublish"})).unwrap();

        assert_eq!(removed["user"], user);
        assert_eq!(removed["project_id"], "docs-unpublish");
        assert_eq!(removed["removed"], true);
        assert!(
            !PUBLISHED_PROJECTS.contains_key(&key),
            "unpublish must remove published project"
        );
    }

    #[test]
    fn handle_health_reports_aggregate_ready_without_projects() {
        let health = handle_health(
            "health-aggregate-owner",
            "health-aggregate-user",
            "example",
            json!({}),
        )
        .unwrap();

        assert_eq!(health["state"], "ready");
        assert_eq!(health["ready"], true);
        assert_eq!(
            health["owner_ura"],
            "easynet:///r/example/agent/health-aggregate-owner.pages"
        );
    }

    #[test]
    fn handle_health_reports_missing_project_as_degraded() {
        let health = handle_health(
            "health-missing-owner",
            "health-missing-user",
            "example",
            json!({"project_id": "docs"}),
        )
        .unwrap();

        assert_eq!(health["state"], "degraded");
        assert_eq!(health["ready"], false);
        assert_eq!(
            health["surface_ref"],
            "easynet:///r/example/resource/health-missing-user.docs"
        );
    }

    #[test]
    fn handle_health_reports_project_present_as_ready_projection() {
        let user = "health-ready-user";
        let (_root, key) = publish_test_project(user, "docs-health");

        let health = handle_health(
            "health-ready-owner",
            user,
            "example",
            json!({"project_id": "docs-health"}),
        )
        .unwrap();
        remove_test_project(&key);

        assert_eq!(health["state"], "ready");
        assert_eq!(health["ready"], true);
        assert_eq!(health["page_count"], 1);
        assert_eq!(
            health["surface_ref"],
            "easynet:///r/example/resource/health-ready-user.docs-health"
        );
        assert_eq!(health["checks"][0]["name"], "pages_registry");
        assert_eq!(health["checks"][1]["name"], "project");
        assert_eq!(health["checks"][1]["state"], "ready");
        assert_eq!(health["checks"][1]["ready"], true);
        assert_eq!(health["checks"][1]["message"], Value::Null);
        assert_eq!(health["checks"][1]["metadata"]["project_id"], "docs-health");
        assert_eq!(health["checks"][1]["metadata"]["requested"], true);
    }

    #[test]
    fn handle_health_rejects_foreign_surface_ref() {
        let err = handle_health(
            "alice-owner",
            "alice",
            "example",
            json!({"surface_ref": "easynet:///r/example/resource/bob.docs/"}),
        )
        .unwrap_err();

        assert!(
            err.to_string().contains("surface_ref"),
            "wrong error: {err}"
        );
    }
}
