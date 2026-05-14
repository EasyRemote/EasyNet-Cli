// EasyNet CLI — API key abilities
// =================================
//
// File: src/runtime/agents/api_key_ability.rs
// Description: ability family `<user>.api_key.{create, list, revoke}`
//              for OpenAI-compatibility-shaped API keys (RFC-006-C v0.1).
//
//              This is intentionally separate from the RFC-002
//              keyring vault: that vault holds Ed25519 keypairs
//              with a cryptographic admission protocol; API keys
//              here are bearer tokens — a long unguessable string
//              that maps to a `user/<username>` URA at the HTTP
//              boundary. Different threat model, different store.
//
// Persistence: TOML at `~/.easynet/api_keys.toml`. World-readable
//              by mode is acceptable for v0 because every line
//              records `token_hash = sha256(token)`, NOT the
//              token itself; the only place the raw token exists
//              after `create` is the response the caller sees
//              once. Lose it, mint a new one.
//
// URA shape:   `easynet:///r/<realm>/resource/api_key.<id>`
//              where `<id>` is the same unguessable string the
//              caller uses as Bearer (≥256 bits of entropy).
//
// Conformance: RFC-006-C v0.1 INV-2 (Capability-URI Key).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::runtime::ability_dispatch::{LocalAbilityRegistry, LocalRpcHandler};

/// Process-wide lock around the api_keys.toml read-modify-write
/// cycle. Without it, two concurrent `mint_api_key` invocations
/// can race: A loads, B loads, A writes, B writes — A's entry is
/// silently overwritten. The store is small enough that a single
/// global mutex is fine.
static STORE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    /// First 12 chars of the token — for `list` display so the
    /// operator can identify which key without printing the
    /// secret. Full token is never persisted.
    pub id_prefix: String,
    /// Full sha256(token) hex — what we compare against on auth.
    pub token_hash: String,
    /// User URA the bearer authenticates as.
    pub user_ura: String,
    pub label: Option<String>,
    pub created_at: u64,
    pub revoked_at: Option<u64>,
    pub last_used_at: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ApiKeyStore {
    #[serde(default)]
    pub keys: Vec<ApiKeyEntry>,
}

fn store_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".easynet").join("api_keys.toml")
}

fn load_store() -> ApiKeyStore {
    let path = store_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return ApiKeyStore::default();
    };
    toml::from_str(&text).unwrap_or_default()
}

fn save_store(store: &ApiKeyStore) -> anyhow::Result<()> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(store)?;
    // atomic via temp file in same dir, then rename
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.flush()?;
    }
    fs::rename(&tmp, &path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn hash_token(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    format!("{:x}", h.finalize())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn realm() -> String {
    std::env::var("EASYNET_PAGES_REALM").unwrap_or_else(|_| crate::ura::REALM_EASYNET.to_string())
}

/// Mint a fresh API key. Returns the bearer token ONCE — the
/// caller must persist it client-side.
pub fn handle_create(user: &str, args: Value) -> anyhow::Result<Value> {
    let label = args.get("label").and_then(Value::as_str).map(String::from);

    // 32 random bytes → 64-hex token. Prefix with `easynet-sk-`
    // so secret-scanners (GitHub, gitleaks) can identify it
    // distinctly from OpenAI / Anthropic keys.
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    let id = hex::encode(buf);
    let token = format!("easynet-sk-{id}");

    let entry = ApiKeyEntry {
        id_prefix: id[..12].to_string(),
        token_hash: hash_token(&token),
        user_ura: crate::ura::user_ura(&realm(), user),
        label: label.clone(),
        created_at: now_secs(),
        revoked_at: None,
        last_used_at: None,
    };

    let key_uri = {
        let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut store = load_store();
        store.keys.push(entry.clone());
        save_store(&store)?;
        // INV-2 capability URA: the addressable resource id is the
        // FULL 64-hex token id (256 bits of entropy), not the
        // 12-char display prefix. The display prefix is for
        // operator-visible listing only; the URA must carry the
        // full unguessable id so revocation by URA cannot collide
        // and so the URA itself functions as the capability.
        crate::ura::resource_dot_ura(&realm(), &format!("api_key.{id}"), "")
    };

    Ok(json!({
        "token":          token,                          // ONLY returned here
        "key_uri":        key_uri,
        "id_prefix":      entry.id_prefix,
        "user_ura":       entry.user_ura,
        "label":          entry.label,
        "created_at":     entry.created_at,
        "warning":        "Save the token now. It is the only time we will show it.",
    }))
}

/// List keys (without exposing tokens).
pub fn handle_list(user: &str, _args: Value) -> anyhow::Result<Value> {
    let store = load_store();
    let user_ura = crate::ura::user_ura(&realm(), user);
    let mine: Vec<_> = store
        .keys
        .iter()
        .filter(|k| k.user_ura == user_ura)
        .map(|k| {
            json!({
                "id_prefix":     k.id_prefix,
                "label":         k.label,
                "created_at":    k.created_at,
                "last_used_at":  k.last_used_at,
                "revoked":       k.revoked_at.is_some(),
                "revoked_at":    k.revoked_at,
            })
        })
        .collect();
    Ok(json!({ "keys": mine }))
}

/// Revoke a key by id_prefix.
pub fn handle_revoke(user: &str, args: Value) -> anyhow::Result<Value> {
    let id_prefix = args
        .get("id_prefix")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing id_prefix"))?;
    let user_ura = crate::ura::user_ura(&realm(), user);

    let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut store = load_store();
    let mut found = false;
    for k in store.keys.iter_mut() {
        if k.id_prefix == id_prefix && k.user_ura == user_ura {
            if k.revoked_at.is_some() {
                anyhow::bail!("key {id_prefix} already revoked");
            }
            k.revoked_at = Some(now_secs());
            found = true;
            break;
        }
    }
    if !found {
        anyhow::bail!("key {id_prefix} not found for user {user}");
    }
    save_store(&store)?;
    Ok(json!({ "revoked": id_prefix }))
}

/// Resolve a Bearer token to a user URA, mutating last_used_at.
/// Returns Err if token unknown or revoked.
pub fn resolve_token(token: &str) -> anyhow::Result<(String, String)> {
    let hash = hash_token(token);
    let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut store = load_store();
    for k in store.keys.iter_mut() {
        if k.token_hash == hash {
            if k.revoked_at.is_some() {
                anyhow::bail!("api key revoked");
            }
            k.last_used_at = Some(now_secs());
            let user_ura = k.user_ura.clone();
            let id_prefix = k.id_prefix.clone();
            // best-effort save (last_used update); failure is non-fatal
            let _ = save_store(&store);
            return Ok((user_ura, id_prefix));
        }
    }
    anyhow::bail!("api key not recognized");
}

/// Read the operator-side default token from
/// `~/.easynet/api_keys.local.toml` (`default_token = "..."`).
/// Used by `easynet llm-api` and `easynet api-key` family as
/// the default when the caller did not pass `--key` or set the
/// `EASYNET_API_KEY` env var.
///
/// Important: the raw token is NEVER stored in `api_keys.toml`
/// (only its sha256 hash is). The local cache file is a separate
/// operator convenience — `easynet api-key create` writes it
/// once at mint time, mode 0600. Lose the file → mint a new key.
pub fn read_local_default_token() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home)
        .join(".easynet")
        .join("api_keys.local.toml");
    let text = fs::read_to_string(path).ok()?;
    #[derive(Deserialize)]
    struct LocalTokens {
        #[serde(default)]
        default_token: Option<String>,
    }
    let parsed: LocalTokens = toml::from_str(&text).ok()?;
    parsed.default_token
}

/// Write the raw token to the local cache file so subsequent
/// CLI calls can find it without --key. Operator-side
/// convenience; never sent over the wire.
pub fn write_local_default_token(token: &str) -> anyhow::Result<()> {
    let home = std::env::var("HOME").map_err(|_| anyhow::anyhow!("HOME unset"))?;
    let dir = PathBuf::from(home).join(".easynet");
    fs::create_dir_all(&dir)?;
    let path = dir.join("api_keys.local.toml");
    let text = format!("default_token = \"{token}\"\n");
    fs::write(&path, text)?;
    // tighten perms — best effort
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn register(reg: &mut LocalAbilityRegistry, user: &str) {
    use crate::runtime::ability_dispatch::OwnerKind;
    let owner = OwnerKind::User(user.to_string());
    let user_owned = user.to_string();
    let u1 = user_owned.clone();
    let create_handler: LocalRpcHandler = Arc::new(move |args| handle_create(&u1, args));
    reg.register_rpc_with_owner(
        format!("{user}.api_key.create"),
        owner.clone(),
        create_handler,
    );

    let u2 = user_owned.clone();
    let list_handler: LocalRpcHandler = Arc::new(move |args| handle_list(&u2, args));
    reg.register_rpc_with_owner(format!("{user}.api_key.list"), owner.clone(), list_handler);

    let u3 = user_owned.clone();
    let revoke_handler: LocalRpcHandler = Arc::new(move |args| handle_revoke(&u3, args));
    reg.register_rpc_with_owner(format!("{user}.api_key.revoke"), owner, revoke_handler);
}
