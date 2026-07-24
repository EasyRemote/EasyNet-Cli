// EasyNet CLI — API key abilities
// =================================
//
// File: src/daemon/ability/builtins/governance/api_key.rs
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
// Conformance: RFC-006-C v0.1 INV-2 (Capability-URA Key).
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::io::ErrorKind;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use once_cell::sync::Lazy;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::daemon::ability::descriptors::AdmissionAction;
use crate::daemon::ability::dispatch::{AxonAbilityCatalog, LocalRpcHandler};

use super::api_key_projection::{ApiKeyCreateResponse, ApiKeyListResponse, ApiKeyRevokeResponse};

/// Process-wide lock around the api_keys.toml read-modify-write
/// cycle. Without it, two concurrent `mint_api_key` invocations
/// can race: A loads, B loads, A writes, B writes — A's entry is
/// silently overwritten. The store is small enough that a single
/// global mutex is fine.
static STORE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct ApiKeyStore {
    pub keys: Vec<ApiKeyEntry>,
}

fn store_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".easynet").join("api_keys.toml")
}

fn local_default_token_path() -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME unset for local API key cache")?;
    Ok(PathBuf::from(home)
        .join(".easynet")
        .join("api_keys.local.toml"))
}

fn load_store() -> anyhow::Result<ApiKeyStore> {
    let path = store_path();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ApiKeyStore::default());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("read API key store {}", path.display()));
        }
    };
    toml::from_str(&text).with_context(|| format!("parse API key store {}", path.display()))
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

/// Mint a fresh API key. Returns the bearer token ONCE — the
/// caller must persist it client-side.
pub fn handle_create(user: &str, realm: &str, args: Value) -> anyhow::Result<Value> {
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
        user_ura: crate::core::ura::user_ura(realm, user),
        label: label.clone(),
        created_at: now_secs(),
        revoked_at: None,
        last_used_at: None,
    };

    let key_ura = {
        let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let mut store = load_store()?;
        store.keys.push(entry.clone());
        save_store(&store)?;
        // INV-2 capability URA: the addressable resource id is the
        // FULL 64-hex token id (256 bits of entropy), not the
        // 12-char display prefix. The display prefix is for
        // operator-visible listing only; the URA must carry the
        // full unguessable id so revocation by URA cannot collide
        // and so the URA itself functions as the capability.
        crate::core::ura::resource_dot_ura(realm, &format!("api_key.{id}"), "")
    };

    Ok(serde_json::to_value(ApiKeyCreateResponse::one_time_token(
        token, key_ura, &entry,
    ))?)
}

/// List keys (without exposing tokens).
pub fn handle_list(user: &str, realm: &str, _args: Value) -> anyhow::Result<Value> {
    let store = load_store()?;
    let user_ura = crate::core::ura::user_ura(realm, user);
    Ok(serde_json::to_value(ApiKeyListResponse::from_entries(
        store.keys.iter().filter(|key| key.user_ura == user_ura),
    ))?)
}

/// Revoke a key by id_prefix.
pub fn handle_revoke(user: &str, realm: &str, args: Value) -> anyhow::Result<Value> {
    let id_prefix = args
        .get("id_prefix")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing id_prefix"))?;
    let user_ura = crate::core::ura::user_ura(realm, user);

    let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut store = load_store()?;
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
    Ok(serde_json::to_value(ApiKeyRevokeResponse::revoked(
        id_prefix,
    ))?)
}

/// Resolve a Bearer token to a user URA, mutating last_used_at.
/// Returns Err if token unknown or revoked.
pub fn resolve_token(token: &str) -> anyhow::Result<(String, String)> {
    let hash = hash_token(token);
    let _guard = STORE_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut store = load_store()?;
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
pub fn read_local_default_token() -> anyhow::Result<Option<String>> {
    let path = local_default_token_path()?;
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("read local API key cache {}", path.display()));
        }
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct LocalTokens {
        default_token: String,
    }
    let parsed: LocalTokens = toml::from_str(&text)
        .with_context(|| format!("parse local API key cache {}", path.display()))?;
    let token = parsed.default_token.trim();
    if token.is_empty() {
        anyhow::bail!(
            "local API key cache {} has blank default_token",
            path.display()
        );
    }
    Ok(Some(token.to_string()))
}

/// Write the raw token to the local cache file so subsequent
/// CLI calls can find it without --key. Operator-side
/// convenience; never sent over the wire.
pub fn write_local_default_token(token: &str) -> anyhow::Result<()> {
    let path = local_default_token_path()?;
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("local API key cache path has no parent"))?;
    fs::create_dir_all(&dir)?;
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

pub fn register(reg: &mut AxonAbilityCatalog, user: &str, realm: &str) {
    use crate::daemon::ability::dispatch::OwnerKind;
    let owner = OwnerKind::User(user.to_string());
    let user_owned = user.to_string();
    let realm_owned = realm.to_string();
    let u1 = user_owned.clone();
    let r1 = realm_owned.clone();
    let create_handler: LocalRpcHandler = Arc::new(move |args| handle_create(&u1, &r1, args));
    reg.register_rpc_with_owner_and_action(
        format!("{user}.api_key.create"),
        owner.clone(),
        AdmissionAction::Manage,
        create_handler,
    );

    let u2 = user_owned.clone();
    let r2 = realm_owned.clone();
    let list_handler: LocalRpcHandler = Arc::new(move |args| handle_list(&u2, &r2, args));
    reg.register_rpc_with_owner_and_action(
        format!("{user}.api_key.list"),
        owner.clone(),
        AdmissionAction::Read,
        list_handler,
    );

    let u3 = user_owned.clone();
    let r3 = realm_owned;
    let revoke_handler: LocalRpcHandler = Arc::new(move |args| handle_revoke(&u3, &r3, args));
    reg.register_rpc_with_owner_and_action(
        format!("{user}.api_key.revoke"),
        owner,
        AdmissionAction::Manage,
        revoke_handler,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;
    use serde_json::json;

    fn seed_malformed_store() {
        let path = store_path();
        std::fs::create_dir_all(path.parent().expect("api key store parent"))
            .expect("create isolated api key state dir");
        std::fs::write(path, "keys = [").expect("write malformed api key store");
    }

    fn write_store_body(body: &str) {
        let path = store_path();
        std::fs::create_dir_all(path.parent().expect("api key store parent"))
            .expect("create isolated api key state dir");
        std::fs::write(path, body).expect("write api key store");
    }

    fn write_local_default_cache(body: &str) {
        let path = local_default_token_path().expect("local default token path");
        std::fs::create_dir_all(path.parent().expect("local cache parent"))
            .expect("create isolated local cache dir");
        std::fs::write(path, body).expect("write local default token cache");
    }

    fn assert_store_parse_failure(error: anyhow::Error) {
        let message = format!("{error:#}");
        assert!(
            message.contains("parse API key store"),
            "unexpected error: {message}"
        );
    }

    fn assert_store_parse_failure_contains(error: anyhow::Error, expected: &str) {
        let message = format!("{error:#}");
        assert!(
            message.contains("parse API key store"),
            "unexpected error: {message}"
        );
        assert!(
            message.contains(expected),
            "expected {expected:?} in error: {message}"
        );
    }

    #[test]
    fn missing_store_is_fresh_install_empty_state() {
        let _home = HomeGuard::new();

        let listed =
            handle_list("alice", "example", json!({})).expect("missing store lists as empty");

        assert_eq!(listed["keys"].as_array().expect("keys array").len(), 0);
    }

    #[test]
    fn api_key_store_rejects_existing_file_without_keys() {
        let _home = HomeGuard::new();
        write_store_body("# legacy empty authority file\n");

        let error = load_store().expect_err("existing API key store without keys must fail");

        assert_store_parse_failure_contains(error, "missing field `keys`");
    }

    #[test]
    fn api_key_store_rejects_unknown_top_level_fields() {
        let _home = HomeGuard::new();
        write_store_body("keys = []\nlegacy = true\n");

        let error = load_store().expect_err("unknown top-level API key store fields must fail");

        assert_store_parse_failure_contains(error, "unknown field `legacy`");
    }

    #[test]
    fn api_key_store_rejects_unknown_entry_fields() {
        let _home = HomeGuard::new();
        write_store_body(
            r#"
[[keys]]
id_prefix = "abc123"
token_hash = "hash"
user_ura = "easynet:///r/example/user/alice"
created_at = 1
legacy_scope = "all"
"#,
        );

        let error = load_store().expect_err("unknown API key entry fields must fail");

        assert_store_parse_failure_contains(error, "unknown field `legacy_scope`");
    }

    #[test]
    fn list_rejects_malformed_store_instead_of_empty_projection() {
        let _home = HomeGuard::new();
        seed_malformed_store();

        let error =
            handle_list("alice", "example", json!({})).expect_err("malformed store must fail list");

        assert_store_parse_failure(error);
    }

    #[test]
    fn create_rejects_malformed_store_instead_of_overwriting_authority() {
        let _home = HomeGuard::new();
        seed_malformed_store();

        let error = handle_create("alice", "example", json!({"label": "new"}))
            .expect_err("malformed store must fail create");

        assert_store_parse_failure(error);
        let body = std::fs::read_to_string(store_path()).expect("malformed store still present");
        assert_eq!(body, "keys = [");
    }

    #[test]
    fn create_stamps_registered_realm_without_product_default_lookup() {
        let _home = HomeGuard::new();

        let created = handle_create("alice", "custom-realm", json!({"label": "new"}))
            .expect("create API key");

        assert_eq!(
            created["user_ura"].as_str(),
            Some("easynet:///r/custom-realm/user/alice")
        );
        assert!(
            created["key_ura"].as_str().expect("key ura").starts_with(
                &crate::core::ura::resource_dot_ura("custom-realm", "api_key.", "")
            ),
            "key URA should use registered realm: {created}"
        );
    }

    #[test]
    fn create_list_revoke_return_typed_public_shapes_without_secret_leaks() {
        let _home = HomeGuard::new();

        let created =
            handle_create("alice", "example", json!({"label": "dev"})).expect("create API key");
        let id_prefix = created["id_prefix"]
            .as_str()
            .expect("created id_prefix")
            .to_string();

        assert!(created["token"]
            .as_str()
            .expect("one-time token")
            .starts_with("easynet-sk-"));
        assert_eq!(created["label"], "dev");
        assert!(created.get("token_hash").is_none());

        let listed = handle_list("alice", "example", json!({})).expect("list API keys");
        assert_eq!(listed["keys"].as_array().expect("keys array").len(), 1);
        assert_eq!(listed["keys"][0]["id_prefix"], id_prefix);
        assert_eq!(listed["keys"][0]["label"], "dev");
        assert_eq!(listed["keys"][0]["revoked"], false);
        assert!(listed["keys"][0].get("token").is_none());
        assert!(listed["keys"][0].get("token_hash").is_none());
        assert!(listed["keys"][0].get("user_ura").is_none());

        let revoked = handle_revoke("alice", "example", json!({"id_prefix": id_prefix}))
            .expect("revoke API key");
        assert_eq!(revoked["revoked"], created["id_prefix"]);
        assert!(revoked.get("user_ura").is_none());

        let listed_after_revoke =
            handle_list("alice", "example", json!({})).expect("list revoked API key");
        assert_eq!(listed_after_revoke["keys"][0]["revoked"], true);
        assert!(listed_after_revoke["keys"][0]["revoked_at"].is_number());
    }

    #[test]
    fn missing_local_default_token_cache_is_no_default_token_state() {
        let _home = HomeGuard::new();

        let token = read_local_default_token().expect("missing cache should be readable state");

        assert_eq!(token, None);
    }

    #[test]
    fn local_default_token_cache_reads_written_token() {
        let _home = HomeGuard::new();

        write_local_default_token("sk-test-token").expect("write local default token");

        let token = read_local_default_token().expect("read local default token");
        assert_eq!(token.as_deref(), Some("sk-test-token"));
    }

    #[test]
    fn local_default_token_cache_rejects_malformed_toml() {
        let _home = HomeGuard::new();
        write_local_default_cache("default_token = ");

        let error =
            read_local_default_token().expect_err("malformed local default token must fail");

        let message = format!("{error:#}");
        assert!(
            message.contains("parse local API key cache"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn local_default_token_cache_rejects_unknown_fields() {
        let _home = HomeGuard::new();
        write_local_default_cache("default_token = \"sk-test-token\"\nlegacy = true\n");

        let error =
            read_local_default_token().expect_err("unknown local default token fields must fail");

        let message = format!("{error:#}");
        assert!(
            message.contains("parse local API key cache"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn local_default_token_cache_rejects_blank_token() {
        let _home = HomeGuard::new();
        write_local_default_cache("default_token = \"  \"\n");

        let error = read_local_default_token().expect_err("blank local default token must fail");

        let message = format!("{error:#}");
        assert!(
            message.contains("blank default_token"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn bearer_resolution_rejects_malformed_store_instead_of_unknown_token() {
        let _home = HomeGuard::new();
        seed_malformed_store();

        let error = resolve_token("easynet-sk-test").expect_err("malformed store must fail auth");
        let message = format!("{error:#}");

        assert!(
            message.contains("parse API key store"),
            "unexpected error: {message}"
        );
        assert!(
            !message.contains("api key not recognized"),
            "malformed credential authority must not be projected as unknown token: {message}"
        );
    }
}
