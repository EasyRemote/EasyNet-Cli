// EasyNet CLI — Files reference system: content-addressed blob store
// =====================================================================
//
// File: src/runtime/system_abilities/resources/files_store/mod.rs
// Description: registration entry point for the user-rooted files
//              namespace, complement to Pages (RFC-006-B v0.6).
//              Registers `<user>.files.<verb>` directly into the
//              daemon-hosted Axon LocalRuntime.
//
// What this is for:
//   `/v1/chat/completions` accepts OpenAI-shape multimodal messages
//   carrying `image_url.url` / `file.url` fields. When that field
//   carries a v4.1.5 resource URA `easynet:///r/<realm>/resource/<u>.files/<sha256>`,
//   the daemon-side adapter dereferences it into base-64 bytes
//   inline before forwarding to `<agent>.chat`. Files this surface
//   serves are the files the chat-base ability can read; agents
//   write outputs through the same surface so the reply can carry
//   URAs the client renders.
//
// Wire shape per v4.1.5 §A.URA-7:
//   easynet:///r/<realm>/resource/<userID>.files/<sha256>
//
// Owner segment is `<userID>.files` (dot-id-part); the slash-path-
// part is the content-addressed blob hash. Content-addressed
// because (a) dedup is free, (b) URA → bytes mapping is verifiable
// without the daemon having to store any naming convention beyond
// the hash, (c) replays + caching are safe.
//
// Storage:
//   $EASYNET_FILES_ROOT (default ~/.easynet/files)/<sha256>
//   Content-addressed. Same bytes → same hash → same on-disk path.
//
// Three abilities registered:
//   <user>.files.put    — write {filename, bytes_b64, content_type?}
//                         → {ura, sha256, size, content_type}
//   <user>.files.get    — read {sha256} or {path: "<sha256>"}
//                         → {bytes_b64, content_type, sha256}
//   <user>.files.list   — read {} → {items: [{sha256, size, ...}]}
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod handlers;
pub mod state;

use std::path::PathBuf;
use std::sync::Arc;

use crate::runtime::ability_dispatch::{AxonAbilityCatalog, LocalRpcHandler, OwnerKind};

/// Installation parameters for the Files reference system. Mirror
/// of `PagesConfig`; the daemon's user identity is the only field
/// that varies per-deployment. `root` is the boot-time-resolved
/// blob storage path (env-read happens once in `register`, handlers
/// take it by arg so unit tests are parallel-safe per memory
/// `feedback_no_lock_around_parallel_tests.md`).
#[derive(Debug, Clone)]
pub struct FilesConfig {
    pub user: String,
    pub realm: String,
}

/// Wire the Files reference system into the registry. Called
/// once at daemon boot from `build_registry_with_services`. The
/// blob storage root is resolved here from `EASYNET_FILES_ROOT`
/// (or `~/.easynet/files` fallback); handlers receive the path
/// by Arc, no further env-reads at invoke time.
pub fn register(reg: &mut AxonAbilityCatalog, config: FilesConfig) {
    let root: Arc<PathBuf> = match state::root_from_env() {
        Ok(p) => Arc::new(p),
        Err(err) => {
            eprintln!(
                "[files] could not resolve EASYNET_FILES_ROOT (`{err}`); \
                 files surface disabled this boot"
            );
            return;
        }
    };
    let owner = OwnerKind::User(config.user.clone());

    let user = config.user.clone();
    let realm = config.realm.clone();
    let root_for_put = Arc::clone(&root);
    let put_handler: LocalRpcHandler =
        Arc::new(move |args| handlers::handle_put(&user, &realm, &root_for_put, args));
    reg.register_rpc_with_owner(
        format!("{}.files.put", config.user),
        owner.clone(),
        put_handler,
    );

    let root_for_get = Arc::clone(&root);
    let get_handler: LocalRpcHandler =
        Arc::new(move |args| handlers::handle_get(&root_for_get, args));
    reg.register_rpc_with_owner(
        format!("{}.files.get", config.user),
        owner.clone(),
        get_handler,
    );

    let user = config.user.clone();
    let realm = config.realm.clone();
    let root_for_list = Arc::clone(&root);
    let list_handler: LocalRpcHandler =
        Arc::new(move |args| handlers::handle_list(&user, &realm, &root_for_list, args));
    reg.register_rpc_with_owner(format!("{}.files.list", config.user), owner, list_handler);
}
