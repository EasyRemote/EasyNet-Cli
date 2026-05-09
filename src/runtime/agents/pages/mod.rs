// EasyNet CLI — Pages reference system: ability registration
// ===========================================================
//
// File: src/runtime/agents/pages/mod.rs
// Description: registration entry point for the Pages reference
//              system (RFC-006-B v0.6). Installs a single fallback
//              resolver into the daemon's `LocalAbilityRegistry`
//              that pattern-matches `<user>.pages.<verb>` and
//              `<user>.<project_id>.page.fetch` on lookup miss
//              and synthesises the appropriate handler.
//
// Why a fallback resolver, not eager `register_rpc`:
//   The `<user>.<project_id>.page.fetch` ability name is
//   per-publish — at boot we don't know which projects exist, and
//   the registry is frozen behind `Arc` after boot. The fallback
//   resolver pattern (already used by the dispatcher) lets us
//   synthesise the handler at lookup time based on
//   `PUBLISHED_PROJECTS`. The cost is one extra hash check per
//   lookup miss; the benefit is no mutable-registry plumbing.
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

use serde_json::Value;

use crate::runtime::ability_dispatch::{LocalAbilityRegistry, LocalRpcHandler};

/// Installation parameters for the Pages reference system. Carry
/// the daemon's user identity (the `<user>` segment in every
/// pages-rooted URI), the realm, and the in-daemon Hub listener
/// port (only used to format the dev-only listener URL surfaced
/// from `<user>.pages.get`).
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
/// only by `<user>.pages.get` for debugging.
pub fn pages_public_url_root(realm: &str, user: &str, project_id: &str) -> String {
    format!("https://{realm}/web/{user}/{project_id}/")
}

/// Dev-only URL of the daemon's in-process HTTP listener for this
/// project. Only meaningful when `EASYNET_PAGES_PORT` is set and
/// the daemon spawned its local listener; in production this URL
/// resolves to nothing. Returned as a secondary field from
/// `<user>.pages.get` so an operator running `easynet pages show
/// <project>` can see both the production URL and the local
/// listener URL during dev.
pub fn pages_dev_listener_url_root(user: &str, project_id: &str, listener_port: u16) -> String {
    format!("http://{project_id}.{user}.pages.localhost:{listener_port}/")
}

/// Wire the Pages reference system into the registry. Called
/// once at daemon boot from `build_registry_with_services`.
///
/// Installs a single fallback resolver. The resolver consults
/// `PUBLISHED_PROJECTS` for `<user>.<project>.page.fetch` requests
/// and dispatches to fixed handlers for `<user>.pages.{publish,
/// unpublish, list, get}`.
///
/// `dispatch_handle` is the post-build OnceLock seam pointing at
/// the live registry. The api `kind="ability"` branch reaches
/// through it to invoke any other ability the agent has deployed.
pub fn register(
    reg: &mut LocalAbilityRegistry,
    config: PagesConfig,
    dispatch_handle: Arc<std::sync::OnceLock<Arc<LocalAbilityRegistry>>>,
) {
    let resolver_config = config.clone();
    api::set_dispatch_handle(dispatch_handle);

    // Build a placeholder Arc for the publish handler (which needs
    // a registry handle for `register_fetch_ability`). v0 keeps
    // the eager registration a no-op (see fetch::register_fetch_ability),
    // so the publish handler doesn't actually need this. We leave
    // the field for the day a non-fallback registration path lands.
    let publish_registry: Arc<LocalAbilityRegistry> = Arc::new(LocalAbilityRegistry::new());

    let resolver: crate::runtime::ability_dispatch::LocalFallbackResolver =
        Arc::new(move |name: &str| -> Option<LocalRpcHandler> {
            let cfg = resolver_config.clone();
            let publish_reg = publish_registry.clone();

            // Pattern 1: <user>.pages.<verb>  where verb is fixed.
            if let Some(rest) = name.strip_prefix(&format!("{}.pages.", cfg.user)) {
                match rest {
                    "publish" => {
                        let cfg2 = cfg.clone();
                        let reg2 = publish_reg.clone();
                        return Some(Arc::new(move |args: Value| {
                            publish::handle_publish(
                                &cfg2.user,
                                cfg2.listener_port,
                                &cfg2.realm,
                                reg2.clone(),
                                args,
                            )
                        }));
                    }
                    "unpublish" => {
                        let user = cfg.user.clone();
                        return Some(Arc::new(move |args: Value| {
                            list_get_unpublish::handle_unpublish(&user, args)
                        }));
                    }
                    "list" => {
                        let user = cfg.user.clone();
                        let realm = cfg.realm.clone();
                        return Some(Arc::new(move |args: Value| {
                            list_get_unpublish::handle_list(&user, &realm, args)
                        }));
                    }
                    "get" => {
                        let user = cfg.user.clone();
                        let port = cfg.listener_port;
                        let realm = cfg.realm.clone();
                        return Some(Arc::new(move |args: Value| {
                            list_get_unpublish::handle_get(&user, port, &realm, args)
                        }));
                    }
                    _ => return None,
                }
            }

            // Pattern 2: <user>.<project>.page.fetch
            if let Some(rest) = name.strip_prefix(&format!("{}.", cfg.user)) {
                if let Some(project_id) = rest.strip_suffix(".page.fetch") {
                    // project_id may not contain '.' (RFC-006-B §publish.validate_project_id);
                    // refuse if it does to avoid ambiguous parses against
                    // sub-namespaced ability names.
                    if !project_id.contains('.') && !project_id.is_empty() {
                        let user = cfg.user.clone();
                        let pid = project_id.to_string();
                        return Some(Arc::new(move |args: Value| {
                            fetch::handle_fetch(&user, &pid, args)
                        }));
                    }
                }
            }

            // Pattern 3: <user>.<project>.api.<verb>
            // Dynamic-backend surface — the project author drops a TOML
            // manifest at <project>/api/<verb>.toml; the daemon evaluates
            // it per request. RFC-006-B v0.6 §10 "API surface" (post-MVP).
            // Subject is the project resource, ability tail is
            // `.api.<verb>`. Non-deterministic by declaration (INV-3 does
            // not bind here).
            if let Some(rest) = name.strip_prefix(&format!("{}.", cfg.user)) {
                // Walk: rest = "<project>.api.<verb>" — must contain
                // ".api." with a project_id (no '.') in front and a
                // verb (no '.', single-segment) after.
                if let Some((project_id, verb)) = rest.split_once(".api.") {
                    if !project_id.is_empty()
                        && !project_id.contains('.')
                        && !verb.is_empty()
                        && !verb.contains('.')
                    {
                        let user = cfg.user.clone();
                        let pid = project_id.to_string();
                        let v = verb.to_string();
                        return Some(Arc::new(move |args: Value| {
                            api::handle_api(&user, &pid, &v, args)
                        }));
                    }
                }
            }

            // Pattern 4: hub-rooted serve ability. The hub-as-agent's
            // `01HUB.pages.serve` ability is registered by the hub
            // module separately; the resolver does not synthesise it
            // here. (See src/runtime/hub/pages_serve_ability.rs.)
            None
        });

    reg.chain_rpc_fallback(resolver);
    match state::restore_published_projects(&config.user) {
        Ok(summary) if summary.skipped > 0 => {
            eprintln!(
                "warning: restored {} pages project(s) for user {}; skipped {} stale snapshot entrie(s)",
                summary.restored, config.user, summary.skipped
            );
        }
        Ok(_) => {}
        Err(err) => {
            eprintln!(
                "warning: failed to restore pages projects for user {}: {err}",
                config.user
            );
        }
    }
}
