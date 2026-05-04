// EasyNet CLI — Files reference system: content-addressed blob store
// =====================================================================
//
// File: src/runtime/agents/files/mod.rs
// Description: registration entry point for the user-rooted files
//              namespace, complement to Pages (RFC-006-B v0.6).
//              Installs a fallback resolver matching
//              `<user>.files.<verb>` on lookup miss and
//              synthesises put/get/list handlers.
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
//                         → {uri, sha256, size, content_type}
//   <user>.files.get    — read {sha256} or {path: "<sha256>"}
//                         → {bytes_b64, content_type, sha256}
//   <user>.files.list   — read {} → {items: [{sha256, size, ...}]}
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod state;
pub mod handlers;

use std::path::PathBuf;
use std::sync::Arc;

use crate::runtime::ability_dispatch::{LocalAbilityRegistry, LocalFallbackResolver, LocalRpcHandler};

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
pub fn register(reg: &mut LocalAbilityRegistry, config: FilesConfig) {
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
    let cfg = Arc::new(config);
    let resolver: LocalFallbackResolver = {
        let cfg = Arc::clone(&cfg);
        let root = Arc::clone(&root);
        Arc::new(move |name: &str| -> Option<LocalRpcHandler> {
            let prefix = format!("{}.files.", cfg.user);
            let rest = name.strip_prefix(&prefix)?;
            let cfg = Arc::clone(&cfg);
            let root = Arc::clone(&root);
            match rest {
                "put" => Some(Arc::new(move |args| {
                    handlers::handle_put(&cfg.user, &cfg.realm, &root, args)
                })),
                "get" => Some(Arc::new(move |args| handlers::handle_get(&root, args))),
                "list" => Some(Arc::new(move |args| {
                    handlers::handle_list(&cfg.user, &cfg.realm, &root, args)
                })),
                _ => None,
            }
        })
    };
    reg.chain_rpc_fallback(resolver);
}
