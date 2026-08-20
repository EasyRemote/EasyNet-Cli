// EasyNet CLI — Pages reference system: ability registration
// ===========================================================
//
// File: src/daemon/ability/builtins/resources/pages/mod.rs
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
pub mod identity;
pub mod list_get_unpublish;
pub mod mime;
pub mod publish;
pub mod sandbox;
pub mod state;

use std::sync::Arc;

use anyhow::Context;

use crate::daemon::ability::dispatch::{AxonAbilityCatalog, LocalRpcHandler, OwnerKind};

pub use identity::{PagesIdentity, PagesUserRootIdentity};

/// Installation parameters for the Pages reference system.
///
/// `user` is the product-facing slug used in URLs and local project storage.
/// `owner_user_id` is the immutable product principal id used by Pages state,
/// admission, and the principal-scoped Pages Service owner. The Device still
/// hosts the directory handles and hot-registration implementation; it is not
/// the public Pages callee.
#[derive(Debug, Clone)]
pub struct PagesConfig {
    pub user: String,
    pub owner_user_id: String,
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
    // The realm label is an identity, not necessarily a reachable hostname
    // (dev realms are literally "localhost", where https://localhost/ has
    // no listener). Prefer the hub API base this device actually paired
    // against — the same origin that serves /web/ — and fall back to the
    // historical https://{realm} form only when no credentials exist
    // (production realms are DNS names).
    let base = crate::daemon::persistence::config::load_credentials_optional()
        .ok()
        .flatten()
        .map(|credentials| credentials.api_base());
    match base {
        Some(base) if !base.is_empty() => format!("{base}/web/{user}/{project_id}/"),
        _ => format!("https://{realm}/web/{user}/{project_id}/"),
    }
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
) -> anyhow::Result<()> {
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
        Err(err) => return Err(err).context("restore published Pages projects"),
    }
    register_restored_project_abilities(reg, &config.owner_user_id, &config.user)
        .context("register restored Pages project abilities")?;
    Ok(())
}

fn register_management_abilities(
    reg: &mut AxonAbilityCatalog,
    config: &PagesConfig,
    dispatch_handle: Arc<std::sync::OnceLock<Arc<AxonAbilityCatalog>>>,
) {
    let owner = pages_service_owner(&config.owner_user_id);

    let user = config.user.clone();
    let owner_user_id = config.owner_user_id.clone();
    let realm = config.realm.clone();
    let listener_port = config.listener_port;
    let publish_handle = Arc::clone(&dispatch_handle);
    let publish_handler: LocalRpcHandler = Arc::new(move |args| {
        let registry = publish_handle
            .get()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("pages registry handle not initialised"))?;
        publish::handle_publish(&owner_user_id, &user, listener_port, &realm, registry, args)
    });
    register_management_rpc(
        reg,
        "pages.publish",
        owner.clone(),
        manifest_for_verb("pages.publish"),
        publish_handler,
    );

    let user = config.user.clone();
    let unpublish_handle = Arc::clone(&dispatch_handle);
    let unpublish_handler: LocalRpcHandler = Arc::new(move |args| {
        let registry = unpublish_handle
            .get()
            .ok_or_else(|| anyhow::anyhow!("pages registry handle not initialised"))?;
        list_get_unpublish::handle_unpublish_with_registry(&user, registry.as_ref(), args)
    });
    register_management_rpc(
        reg,
        "pages.unpublish",
        owner.clone(),
        manifest_for_verb("pages.unpublish"),
        unpublish_handler,
    );

    let user = config.user.clone();
    let realm = config.realm.clone();
    let list_listener_port = config.listener_port;
    let list_handler: LocalRpcHandler = Arc::new(move |args| {
        list_get_unpublish::handle_list(&user, list_listener_port, &realm, args)
    });
    register_management_rpc(
        reg,
        "project_list",
        owner.clone(),
        manifest_for_verb("project_list"),
        list_handler,
    );

    let user = config.user.clone();
    let realm = config.realm.clone();
    let listener_port = config.listener_port;
    let get_handler: LocalRpcHandler =
        Arc::new(move |args| list_get_unpublish::handle_get(&user, listener_port, &realm, args));
    register_management_rpc(
        reg,
        "pages.get",
        owner.clone(),
        manifest_for_verb("pages.get"),
        get_handler,
    );

    let user = config.user.clone();
    let realm = config.realm.clone();
    let health_handle = Arc::clone(&dispatch_handle);
    let health_handler: LocalRpcHandler = Arc::new(move |args| {
        let registry = health_handle
            .get()
            .ok_or_else(|| anyhow::anyhow!("pages registry handle not initialised"))?;
        let owner_ura = registry
            .runtime_binding_facts_for_mode("pages.health", crate::daemon::ability::CallMode::Rpc)
            .map_err(|error| anyhow::anyhow!("resolve pages.health owner: {error}"))?
            .ok_or_else(|| anyhow::anyhow!("pages.health owner is not registered"))?
            .authority_root;
        list_get_unpublish::handle_health(&owner_ura, &user, &realm, args)
    });
    register_management_rpc(
        reg,
        "pages.health",
        owner,
        manifest_for_verb("pages.health"),
        health_handler,
    );
}

fn register_management_rpc(
    reg: &mut AxonAbilityCatalog,
    ability: &'static str,
    owner: OwnerKind,
    manifest: crate::daemon::ability::manifest::AbilityManifest,
    handler: LocalRpcHandler,
) {
    reg.register_rpc_with_spec(ability, owner, manifest, handler);
}

/// Build the `AbilityManifest` for a `pages.<verb>` from the shared
/// spec list. The manifest `name` is the bare verb (`get`), since `.`
/// is the agent/verb separator AbilityManifest rejects.
fn manifest_for_verb(relative_name: &str) -> crate::daemon::ability::manifest::AbilityManifest {
    let spec = management_ability_specs()
        .into_iter()
        .find(|s| s.relative_name == relative_name)
        .unwrap_or_else(|| panic!("no pages spec for {relative_name}"));
    pages_manifest(
        pages_verb_tail(spec.relative_name),
        spec.description,
        spec.input_schema,
    )
    .with_admission_action(pages_admission_action(spec.relative_name))
    .expect("static pages manifest admission action is well-formed")
}

fn pages_admission_action(relative_name: &str) -> &'static str {
    match relative_name {
        "pages.publish" | "pages.unpublish" => "invoke",
        _ => "read",
    }
}

/// Build an `AbilityManifest` for a pages verb. Panics only on a
/// programmer error (a malformed literal schema), which a unit test
/// catches at build time — never at runtime with author input.
fn pages_manifest(
    name: &str,
    description: &str,
    input_schema: serde_json::Value,
) -> crate::daemon::ability::manifest::AbilityManifest {
    crate::daemon::ability::manifest::AbilityManifest::new(name, description, input_schema)
        .expect("static pages manifest is well-formed")
}

/// The `{ project_id }` input schema shared by `pages.get` and
/// `pages.unpublish` — both require the caller to name the project.
fn pages_project_id_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "required": ["project_id"],
        "properties": {
            "project_id": { "type": "string", "description": "The project id to act on." }
        }
    })
}

/// One management verb's advertise-facing spec: the registry-relative
/// ability name (`project_list`), a human description, and the input
/// schema. Single source of truth shared by the local registration
/// (manifests) AND the session-prelude advertise descriptor builder,
/// so the Frontend InvokeAbilityDialog renders the right form whether
/// it reads the local manifest or the advertised descriptor.
pub(crate) struct PagesAbilitySpec {
    pub relative_name: &'static str,
    pub description: &'static str,
    pub input_schema: serde_json::Value,
}

fn pages_health_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "project_id": {
                "type": "string",
                "description": "Optional project id to check."
            },
            "surface_ref": {
                "type": "string",
                "description": "Optional project resource URA to check."
            }
        }
    })
}

/// The user-scoped pages management verbs and their schemas.
pub(crate) fn management_ability_specs() -> Vec<PagesAbilitySpec> {
    vec![
        PagesAbilitySpec {
            relative_name: "project_list",
            description: "List the page projects this user currently publishes on this daemon.",
            input_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {}
            }),
        },
        PagesAbilitySpec {
            relative_name: "pages.publish",
            description: "Publish a folder of static bytes as a website under this user.",
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["project_id", "folder"],
                "properties": {
                    "project_id": { "type": "string", "description": "URA-safe project id (alnum + _ + -, max 64)." },
                    "folder": { "type": "string", "description": "Absolute path to the folder to publish." },
                    "visibility": { "type": "string", "description": "Visibility; only `public` is supported in MVP.", "default": "public" }
                }
            }),
        },
        PagesAbilitySpec {
            relative_name: "pages.get",
            description: "Return one published project's detail (folder, visibility, URLs).",
            input_schema: pages_project_id_schema(),
        },
        PagesAbilitySpec {
            relative_name: "pages.unpublish",
            description:
                "Unpublish a project: release the folder fd and unregister the fetch ability.",
            input_schema: pages_project_id_schema(),
        },
        PagesAbilitySpec {
            relative_name: "pages.health",
            description: "Report daemon Pages registry readiness for this user or one project.",
            input_schema: pages_health_schema(),
        },
    ]
}

/// The verb tail of a `pages.<verb>` relative name (the bare manifest
/// name AbilityManifest requires — `.` is the agent/verb separator).
pub(crate) fn pages_verb_tail(relative_name: &str) -> &str {
    relative_name
        .strip_prefix("pages.")
        .unwrap_or(relative_name)
}

pub(crate) fn register_project_abilities(
    reg: &AxonAbilityCatalog,
    owner_user_id: &str,
    user: &str,
    project_id: &str,
) -> anyhow::Result<usize> {
    fetch::register_fetch_ability(reg, owner_user_id, user, project_id)?;
    Ok(1 + api::register_api_abilities_for_project(reg, owner_user_id, user, project_id)?)
}

pub(crate) fn pages_service_owner(owner_user_id: &str) -> OwnerKind {
    OwnerKind::Service {
        principal_id: owner_user_id.to_string(),
        service_id: "pages".to_string(),
    }
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

pub(crate) fn unregister_project_abilities(
    reg: &AxonAbilityCatalog,
    names: Vec<String>,
) -> anyhow::Result<()> {
    for name in names {
        reg.hot_unregister(&name)?;
    }
    Ok(())
}

fn register_restored_project_abilities(
    reg: &AxonAbilityCatalog,
    owner_user_id: &str,
    user: &str,
) -> anyhow::Result<usize> {
    let mut project_ids: Vec<String> = state::PUBLISHED_PROJECTS
        .iter()
        .filter_map(|entry| {
            let (entry_user, project_id) = entry.key();
            (entry_user == user).then(|| project_id.clone())
        })
        .collect();
    project_ids.sort();
    let mut registered = 0;
    for project_id in project_ids {
        registered += register_project_abilities(reg, owner_user_id, user, &project_id)
            .with_context(|| format!("register restored Pages project {user}/{project_id}"))?;
    }
    Ok(registered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_management_manifests_are_well_formed_and_declare_required_args() {
        // The `.expect()` in pages_manifest must never fire at runtime.
        // Also pin that get/unpublish require project_id (the bug that
        // showed "No input required" and 400'd on empty invoke).
        let id = pages_project_id_schema();
        assert_eq!(id["required"][0], "project_id");

        let get = pages_manifest("get", "d", pages_project_id_schema());
        assert_eq!(get.input_schema()["required"][0], "project_id");

        let publish = pages_manifest(
            "publish",
            "d",
            serde_json::json!({
                "type": "object",
                "required": ["project_id", "folder"],
                "properties": { "project_id": {"type": "string"}, "folder": {"type": "string"} }
            }),
        );
        let req = publish.input_schema()["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "folder"));
        assert!(req.iter().any(|v| v == "project_id"));

        let specs = management_ability_specs();
        assert!(
            specs
                .iter()
                .any(|spec| spec.relative_name == "pages.health"),
            "pages.health must be advertised with the pages management family"
        );
        let health = pages_manifest("health", "d", pages_health_schema());
        assert!(health.input_schema()["properties"]
            .get("surface_ref")
            .is_some());
    }
}
