// EasyNet CLI — Files reference system: content-addressed blob store
// =====================================================================
//
// File: src/daemon/ability/builtins/resources/files_store/mod.rs
// Description: registration entry point for the user-scoped files
//              namespace, complement to Pages (RFC-006-B v0.6).
//              Registers owner-local `files.<verb>` abilities under the
//              daemon-native `agent/<user>.files` executor root.
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
//   $EASYNET_FILES_ROOT (default ~/.easynet/files)/<sha256>.metadata.json
//   Content-addressed. Same bytes → same hash → same on-disk path; producer
//   metadata is immutable for that hash.
//
// Three abilities registered:
//   files.put           — write {filename, bytes_b64, content_type}
//                         → {ura, sha256, size, content_type}
//   files.get           — read {sha256} or {ura}
//                         → {bytes_b64, content_type, sha256}
//   files.list          — read {} → {items: [{sha256, size, ...}]}
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

pub mod handlers;
pub mod state;

use std::path::PathBuf;
use std::sync::Arc;

use crate::daemon::ability::descriptors::AdmissionAction;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, LocalRpcHandler, OwnerKind};
use crate::daemon::ability::manifest::AbilityManifest;
use serde_json::{json, Value};

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

fn files_manifest(
    manifest_name: &str,
    description: &str,
    input_schema: Value,
    admission_action: AdmissionAction,
) -> AbilityManifest {
    AbilityManifest::new(manifest_name, description, input_schema)
        .and_then(|manifest| manifest.with_admission_action(admission_action.as_str()))
        .expect("files_store ability manifest must be valid")
}

fn put_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["filename", "bytes_b64", "content_type"],
        "properties": {
            "filename": {
                "type": "string",
                "minLength": 1,
                "description": "Producer-supplied display filename for the stored blob."
            },
            "bytes_b64": {
                "type": "string",
                "minLength": 1,
                "description": "Base64-encoded blob bytes supplied by the producer."
            },
            "content_type": {
                "type": "string",
                "minLength": 1,
                "description": "Producer-supplied payload media type. The file store never infers this from filename."
            }
        }
    })
}

fn get_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "oneOf": [
            {"required": ["sha256"]},
            {"required": ["ura"]}
        ],
        "properties": {
            "sha256": {
                "type": "string",
                "pattern": "^[0-9a-fA-F]{64}$",
                "description": "Content-addressed blob hash."
            },
            "ura": {
                "type": "string",
                "minLength": 1,
                "description": "Canonical files resource URA ending in the blob sha256."
            }
        }
    })
}

fn list_input_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    })
}

pub(crate) fn description_for(name: &str) -> Option<&'static str> {
    match name {
        "files.put" => Some("Write a content-addressed file blob for the user account."),
        "files.get" => Some("Read a content-addressed file blob for the user account."),
        "files.list" => Some("List content-addressed file blobs for the user account."),
        _ => None,
    }
}

pub(crate) fn input_schema_for(name: &str) -> Option<Value> {
    match name {
        "files.put" => Some(put_input_schema()),
        "files.get" => Some(get_input_schema()),
        "files.list" => Some(list_input_schema()),
        _ => None,
    }
}

fn register_files_rpc(
    reg: &mut AxonAbilityCatalog,
    ability: &'static str,
    owner: OwnerKind,
    manifest: AbilityManifest,
    handler: LocalRpcHandler,
) {
    reg.register_rpc_with_spec(ability, owner, manifest, handler)
}

/// Wire the Files reference system into the registry. Called
/// once at daemon boot from `build_registry_with_services`. The
/// blob storage root is resolved here from `EASYNET_FILES_ROOT`
/// (or the documented daemon files root); handlers receive the path
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
    let owner = OwnerKind::files_system();

    let user = config.user.clone();
    let realm = config.realm.clone();
    let root_for_put = Arc::clone(&root);
    let put_handler: LocalRpcHandler =
        Arc::new(move |args| handlers::handle_put(&user, &realm, &root_for_put, args));
    register_files_rpc(
        reg,
        "files.put",
        owner.clone(),
        files_manifest(
            "put",
            "Write a content-addressed file blob for the user account.",
            put_input_schema(),
            AdmissionAction::Manage,
        ),
        put_handler,
    );

    let root_for_get = Arc::clone(&root);
    let get_handler: LocalRpcHandler =
        Arc::new(move |args| handlers::handle_get(&root_for_get, args));
    register_files_rpc(
        reg,
        "files.get",
        owner.clone(),
        files_manifest(
            "get",
            "Read a content-addressed file blob for the user account.",
            get_input_schema(),
            AdmissionAction::Read,
        ),
        get_handler,
    );

    let user = config.user.clone();
    let realm = config.realm.clone();
    let root_for_list = Arc::clone(&root);
    let list_handler: LocalRpcHandler =
        Arc::new(move |args| handlers::handle_list(&user, &realm, &root_for_list, args));
    register_files_rpc(
        reg,
        "files.list",
        owner,
        files_manifest(
            "list",
            "List content-addressed file blobs for the user account.",
            list_input_schema(),
            AdmissionAction::Read,
        ),
        list_handler,
    );
}
