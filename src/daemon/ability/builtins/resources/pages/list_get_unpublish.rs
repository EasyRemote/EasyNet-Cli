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

use serde_json::{json, Value};

use anyhow::Context;

use crate::daemon::ability::dispatch::AxonAbilityCatalog;

use super::state::{persist_registry_for_user, PUBLISHED_PROJECTS};

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
        let h = entry.value();
        let started_at_ms = h
            .started_at
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        entries.push(json!({
            "user":                  k_user,
            "project_id":            project_id,
            "folder":                h.canonical_root.display().to_string(),
            "visibility":            h.visibility.as_str(),
            "started_at_ms":         started_at_ms,
            "url_root":              super::pages_public_url_root(realm, k_user, project_id),
            "dev_listener_url_root": super::pages_dev_listener_url_root(k_user, project_id, listener_port),
        }));
    }
    Ok(json!({ "projects": entries }))
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

    let started_at_ms = h
        .started_at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let url_root = super::pages_public_url_root(realm, user, project_id);
    let dev_listener_url_root = super::pages_dev_listener_url_root(user, project_id, listener_port);
    let project_ura =
        crate::core::ura::resource_dot_ura(realm, &format!("{user}.{project_id}"), "/");

    Ok(json!({
        "user":                      user,
        "project_id":                project_id,
        "project_ura":               project_ura,
        "folder":                    h.canonical_root.display().to_string(),
        "visibility":                h.visibility.as_str(),
        "started_at_ms":             started_at_ms,
        // Production URL — what a browser hits at easynet.run.
        "url_root":                  url_root,
        // Dev-only daemon-local listener URL. Only reachable when
        // EASYNET_PAGES_PORT is set and the daemon spawned its
        // in-process HTTP listener; null in production daemons.
        // CLI `pages show` renders both; `pages url` prints only
        // `url_root`.
        "dev_listener_url_root":     dev_listener_url_root,
        "file_size_cap":             h.file_size_cap,
    }))
}

/// `pages.health` — report daemon-owned Pages registry readiness.
///
/// This is deliberately narrower than backend/public-route health: it proves
/// the daemon pages management ability family is loaded and, when requested,
/// that one published project is present in the local registry.
pub fn handle_health(user: &str, realm: &str, args: Value) -> anyhow::Result<Value> {
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
    let owner_ura = crate::core::ura::agent_ura(realm, user, "pages");
    let surface_ref = project_id
        .map(|project_id| {
            crate::core::ura::resource_dot_ura(realm, &format!("{user}.{project_id}"), "/")
        })
        .unwrap_or_else(|| {
            crate::core::ura::resource_dot_ura(realm, &format!("{user}.pages"), "/")
        });
    Ok(json!({
        "state": state,
        "ready": ready,
        "owner_ura": owner_ura,
        "surface_ref": surface_ref,
        "page_count": page_count,
        "checks": [
            {
                "name": "pages_registry",
                "state": "ready",
                "ready": true,
                "message": null,
                "latency_ms": 0,
                "metadata": {"source": "PUBLISHED_PROJECTS"}
            },
            {
                "name": "project",
                "state": if project_found { "ready" } else { "missing" },
                "ready": project_found,
                "message": if project_found { Value::Null } else { json!("project is not published") },
                "latency_ms": 0,
                "metadata": {
                    "project_id": project_id,
                    "requested": project_id.is_some()
                }
            }
        ]
    }))
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
    Ok(json!({
        "user":       user,
        "project_id": project_id,
        "removed":    true,
    }))
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

    #[test]
    fn handle_health_reports_aggregate_ready_without_projects() {
        let health = handle_health("health-aggregate-user", "example", json!({})).unwrap();

        assert_eq!(health["state"], "ready");
        assert_eq!(health["ready"], true);
        assert_eq!(
            health["owner_ura"],
            "easynet:///r/example/agent/health-aggregate-user.pages"
        );
    }

    #[test]
    fn handle_health_reports_missing_project_as_degraded() {
        let health = handle_health(
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
    fn handle_health_rejects_foreign_surface_ref() {
        let err = handle_health(
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
