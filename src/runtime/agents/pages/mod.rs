// EasyNet CLI — Pages reference system: ability registration
// ===========================================================
//
// File: src/runtime/agents/pages/mod.rs
// Description: registration entry point for the Pages reference
//              system (RFC-006-B v0.6). Static management verbs are
//              registered directly into Axon's daemon-hosted
//              `LocalRuntime`; per-project fetch/API verbs are
//              hot-registered when projects are published or
//              restored.
//
// Conformance: RFC-006-B v0.6 §2 (the paradigm: HTTP becomes
//              invocation), §4 (the three transitions:
//              publish/fetch/unpublish), Phase B ontology
//              widening (ability owner kinds include `user` and
//              `resource`).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod api;
pub mod fetch;
pub mod list_get_unpublish;
pub mod mime;
pub mod publish;
pub mod sandbox;
pub mod state;

use std::sync::Arc;

use crate::runtime::ability_dispatch::{AxonAbilityCatalog, LocalRpcHandler, OwnerKind};

/// Installation parameters for the Pages reference system. Carry
/// the daemon's user identity (the `<user>` segment in every
/// pages-rooted URI), the realm, and the in-daemon Hub listener
/// port (only used to format the dev-only listener URL surfaced
/// from `pages.get`).
#[derive(Debug, Clone)]
pub struct PagesConfig {
    pub user: String,
    pub realm: String,
    pub listener_port: u16,
}

/// Production public URL where the Hub serves a published project.
///
/// Shape: `https://<realm>/web/<user>/<project_id>/`.
///
/// Authority: this is what `backend/internal/handler/pages_public/
/// serve.go` actually routes — every public fetch a browser does
/// against `easynet.run` resolves through `/web/<u>/<p>/` to the
/// owning daemon's `<user>.<project_id>.page.fetch` ability.
///
/// Earlier drafts of the publish handler returned a daemon-local
/// subdomain shape (`http://<project>.<user>.pages.localhost:<port>/`).
/// That was the URL of the daemon's *in-process* HTTP listener,
/// useful for `curl` against a local dev daemon but **never
/// reachable in production** — production has no
/// `*.*.pages.easynet.run` wildcard cert, and the daemon listener
/// is bound to 127.0.0.1 anyway. Surfacing it as the primary URL
/// from `pages publish` / `pages list` / `pages url` led to
/// operator confusion (silan's review: "对应hub的public的地址描述
/// 不准确"), so we now return the hub form here and demote the
/// daemon-local form to `pages_dev_listener_url_root`, surfaced
/// only by `pages.get` for debugging.
pub fn pages_public_url_root(realm: &str, user: &str, project_id: &str) -> String {
    format!("https://{realm}/web/{user}/{project_id}/")
}

/// Dev-only URL of the daemon's in-process HTTP listener for this
/// project. Only meaningful when `EASYNET_PAGES_PORT` is set and
/// the daemon spawned its local listener; in production this URL
/// resolves to nothing. Returned as a secondary field from
/// `pages.get` so an operator running `easynet pages show
/// <project>` can see both the production URL and the local
/// listener URL during dev.
pub fn pages_dev_listener_url_root(user: &str, project_id: &str, listener_port: u16) -> String {
    format!("http://{project_id}.{user}.pages.localhost:{listener_port}/")
}

/// Wire the Pages reference system into the registry. Called once
/// at daemon boot from `build_registry_with_services`.
///
/// `dispatch_handle` is the post-build OnceLock seam pointing at
/// the live registry. Publish/unpublish need it for post-boot
/// hot-registration, and the api `kind="ability"` branch reaches
/// through it to invoke any other ability the agent has deployed.
pub fn register(
    reg: &mut AxonAbilityCatalog,
    config: PagesConfig,
    dispatch_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
) {
    api::set_dispatch_handle(Arc::clone(&dispatch_handle));
    register_management_abilities(reg, &config, Arc::clone(&dispatch_handle));

    match state::restore_published_projects(&config.user) {
        Ok(summary) if summary.skipped > 0 => {
            let user_field = config.user.as_str();
            let restored = summary.restored;
            let skipped = summary.skipped;
            crate::op_event!(
                component = pages,
                kind = restore_partial,
                level = "warn",
                user = user_field,
                restored = restored,
                skipped = skipped,
            );
        }
        Ok(_) => {}
        Err(err) => {
            let user_field = config.user.as_str();
            let err_msg = format!("{err}");
            crate::op_event!(
                component = pages,
                kind = restore_failed,
                level = "warn",
                user = user_field,
                error = err_msg,
            );
        }
    }
    register_restored_project_abilities(reg, &config.user);
}

fn register_management_abilities(
    reg: &mut AxonAbilityCatalog,
    config: &PagesConfig,
    dispatch_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
) {
    let owner = OwnerKind::User(config.user.clone());

    let user = config.user.clone();
    let realm = config.realm.clone();
    let listener_port = config.listener_port;
    let publish_handle = Arc::clone(&dispatch_handle);
    let publish_handler: LocalRpcHandler = Arc::new(move |args| {
        let registry = publish_handle
            .get()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("pages registry handle not initialised"))?;
        publish::handle_publish(&user, listener_port, &realm, registry, args)
    });
    reg.register_rpc_with_owner("pages.publish", owner.clone(), publish_handler);

    let user = config.user.clone();
    let unpublish_handle = Arc::clone(&dispatch_handle);
    let unpublish_handler: LocalRpcHandler = Arc::new(move |args| {
        let registry = unpublish_handle
            .get()
            .ok_or_else(|| anyhow::anyhow!("pages registry handle not initialised"))?;
        list_get_unpublish::handle_unpublish_with_registry(&user, registry.as_ref(), args)
    });
    reg.register_rpc_with_owner("pages.unpublish", owner.clone(), unpublish_handler);

    let user = config.user.clone();
    let realm = config.realm.clone();
    let list_handler: LocalRpcHandler =
        Arc::new(move |args| list_get_unpublish::handle_list(&user, &realm, args));
    reg.register_rpc_with_owner("pages.list", owner.clone(), list_handler);

    let user = config.user.clone();
    let realm = config.realm.clone();
    let listener_port = config.listener_port;
    let get_handler: LocalRpcHandler =
        Arc::new(move |args| list_get_unpublish::handle_get(&user, listener_port, &realm, args));
    reg.register_rpc_with_owner("pages.get", owner, get_handler);
}

pub(crate) fn register_project_abilities(
    reg: &AxonAbilityCatalog,
    user: &str,
    project_id: &str,
) -> usize {
    fetch::register_fetch_ability(reg, user, project_id);
    1 + api::register_api_abilities_for_project(reg, user, project_id)
}

pub(crate) fn registered_project_ability_names(
    reg: &AxonAbilityCatalog,
    user: &str,
    project_id: &str,
) -> Vec<String> {
    let fetch_name = fetch::fetch_ability_name(user, project_id);
    let api_prefix = format!("{user}.{project_id}.api.");
    let mut names: Vec<String> = reg
        .list_abilities()
        .into_iter()
        .filter(|name| name == &fetch_name || name.starts_with(&api_prefix))
        .collect();
    if !names.iter().any(|name| name == &fetch_name) {
        names.push(fetch_name);
    }
    names.sort();
    names.dedup();
    names
}

pub(crate) fn unregister_project_abilities(reg: &AxonAbilityCatalog, names: Vec<String>) {
    for name in names {
        reg.hot_unregister(&name);
    }
}

fn register_restored_project_abilities(reg: &AxonAbilityCatalog, user: &str) {
    let mut project_ids: Vec<String> = state::PUBLISHED_PROJECTS
        .iter()
        .filter_map(|entry| {
            let (entry_user, project_id) = entry.key();
            (entry_user == user).then(|| project_id.clone())
        })
        .collect();
    project_ids.sort();
    for project_id in project_ids {
        register_project_abilities(reg, user, &project_id);
    }
}
