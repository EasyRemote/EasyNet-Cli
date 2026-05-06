// EasyNet CLI — Pages reference system: list / get / unpublish handlers
// =====================================================================
//
// File: src/runtime/agents/pages/list_get_unpublish.rs
// Description: three remaining ability handlers for the Pages
//              reference system.
//
//                <user>.pages.list       (operational, read)
//                <user>.pages.get        (operational, read)
//                <user>.pages.unpublish  (canonical,    delete)
//
// Conformance: RFC-006-B v0.6 §4.3 (unpublish = delete);
//              list/get are introspection abilities not formally
//              part of the three-transition core but required
//              by the CLI surface.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde_json::{json, Value};

use super::state::PUBLISHED_PROJECTS;

/// `<user>.pages.list` — return every project the daemon
/// currently hosts under this user.
pub fn handle_list(user: &str, listener_port: u16, _args: Value) -> anyhow::Result<Value> {
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
            "user":           k_user,
            "project_id":     project_id,
            "folder":         h.canonical_root.display().to_string(),
            "visibility":     h.visibility.as_str(),
            "started_at_ms":  started_at_ms,
            "url_root":       format!(
                "http://{project_id}.{k_user}.pages.localhost:{listener_port}/"
            ),
        }));
    }
    Ok(json!({ "projects": entries }))
}

/// `<user>.pages.get` — return one project's detail.
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
    let url_root = format!("http://{project_id}.{user}.pages.localhost:{listener_port}/");
    let project_uri = format!("easynet:///r/{realm}/resource/{user}.{project_id}/",);

    Ok(json!({
        "user":           user,
        "project_id":     project_id,
        "project_uri":    project_uri,
        "folder":         h.canonical_root.display().to_string(),
        "visibility":     h.visibility.as_str(),
        "started_at_ms":  started_at_ms,
        "url_root":       url_root,
        "file_size_cap":  h.file_size_cap,
    }))
}

/// `<user>.pages.unpublish` — remove the project. Drops the
/// `ProjectHandle` (releasing the folder fd) and removes the
/// entry from `PUBLISHED_PROJECTS`. Subsequent fetch calls fail
/// at the resolver because the entry is gone.
pub fn handle_unpublish(user: &str, args: Value) -> anyhow::Result<Value> {
    let project_id = args
        .get("project_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing required arg: project_id"))?;
    let key = (user.to_string(), project_id.to_string());
    if PUBLISHED_PROJECTS.remove(&key).is_none() {
        anyhow::bail!("project not found: user={user} project_id={project_id}");
    }
    Ok(json!({
        "user":       user,
        "project_id": project_id,
        "removed":    true,
    }))
}
