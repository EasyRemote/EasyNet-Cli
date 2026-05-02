// EasyNet CLI — Keyring ability handlers (RFC-002 §3.3)
// =======================================================
//
// 10 abilities exposed by the daemon under `keyring.*` namespace
// (the daemon agent's own bundle):
//
//   keyring.create        — generate a fresh ed25519 entry
//   keyring.list          — enumerate entries (filter by purpose/status)
//   keyring.get_public    — fetch public key + fingerprint by key_id
//   keyring.sign          — sign a payload with a named entry
//   keyring.rotate        — retire active key, mint new with epoch+1
//   keyring.revoke        — mark a key revoked (cannot sign again)
//   keyring.expire_set    — schedule expiry timestamp on a key
//   keyring.bind_subject  — link a key to an AgentIdentity URA
//   keyring.peer_add      — TOFU-record a peer's public key
//   keyring.peer_list     — enumerate peer table
//
// All handlers are JSON-in / JSON-out. They register against the
// daemon's `LocalAbilityRegistry` and are invoked through the same
// dispatch path as any other ability.
//
// Author: Silan.Hu
// Email:  silan.hu@u.nus.edu
// Copyright (c) 2026-2027 easynet. All rights reserved.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::sync::Arc;

use super::federated_bindings::{FederatedBindingsStore, FederatedUserBinding};
use super::handle::KeyringHandle;
use super::store::{Entry, KeyStatus, PeerStatus};
use super::user_binding_chain::{
    verify_user_binding_signature, UserBindingError, UserBindingToken, ED25519_PUBKEY_LEN,
    USER_BINDING_FRESHNESS_MS, USER_BINDING_NONCE_LEN,
};
use crate::runtime::ability_dispatch::LocalAbilityRegistry;

fn b64_encode(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    STANDARD.encode(bytes)
}

fn b64_decode(s: &str) -> Result<Vec<u8>> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    STANDARD
        .decode(s)
        .map_err(|e| anyhow!("base64 decode: {e}"))
}

fn entry_view(e: &Entry, full: bool) -> Value {
    let mut v = json!({
        "key_id":         e.key_id,
        "algo":           e.algo,
        "purpose":        e.purpose,
        "status":         match e.status { KeyStatus::Active => "active", KeyStatus::Retired => "retired", KeyStatus::Revoked => "revoked" },
        "rotation_epoch": e.rotation_epoch,
        "bound_subject":  e.bound_subject,
        "rotated_from":   e.rotated_from,
        "created_unix_ms": e.created_unix_ms,
        "expires_unix_ms": e.expires_unix_ms,
        "revoked_unix_ms": e.revoked_unix_ms,
    });
    if full {
        v["public_key"] = json!(e.public_key_b64);
        // fingerprint is computed on demand
        if let Ok(fp) = super::store::entry_fingerprint(e) {
            v["fingerprint"] = json!(b64_encode(&fp));
        }
    }
    v
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("missing required string field `{key}`"))
}

pub fn handle_create(handle: &KeyringHandle, args: Value) -> Result<Value> {
    let purpose = require_str(&args, "purpose")?.to_string();
    let bound_subject = args
        .get("bound_subject")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let entry = handle.create_entry(&purpose, bound_subject)?;
    let fp = super::store::entry_fingerprint(&entry)?;
    Ok(json!({
        "key_id":         entry.key_id,
        "public_key":     entry.public_key_b64,
        "fingerprint":    b64_encode(&fp),
        "rotation_epoch": entry.rotation_epoch,
    }))
}

pub fn handle_list(handle: &KeyringHandle, args: Value) -> Result<Value> {
    let purpose_filter = args
        .get("filter")
        .and_then(|f| f.get("purpose"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let status_filter = args
        .get("filter")
        .and_then(|f| f.get("status"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let entries: Vec<Value> = handle
        .list_entries()
        .into_iter()
        .filter(|e| {
            purpose_filter
                .as_deref()
                .map(|p| e.purpose == p)
                .unwrap_or(true)
        })
        .filter(|e| {
            status_filter
                .as_deref()
                .map(|s| match (s, &e.status) {
                    ("active", KeyStatus::Active) => true,
                    ("retired", KeyStatus::Retired) => true,
                    ("revoked", KeyStatus::Revoked) => true,
                    _ => false,
                })
                .unwrap_or(true)
        })
        .map(|e| entry_view(&e, false))
        .collect();
    Ok(json!({ "entries": entries }))
}

pub fn handle_get_public(handle: &KeyringHandle, args: Value) -> Result<Value> {
    let key_id = require_str(&args, "key_id")?;
    let entry = handle
        .find_entry_by_id(key_id)
        .ok_or_else(|| anyhow!("key_id {key_id} not found"))?;
    let fp = super::store::entry_fingerprint(&entry)?;
    Ok(json!({
        "public_key":     entry.public_key_b64,
        "fingerprint":    b64_encode(&fp),
        "status":         match entry.status { KeyStatus::Active => "active", KeyStatus::Retired => "retired", KeyStatus::Revoked => "revoked" },
        "rotation_epoch": entry.rotation_epoch,
    }))
}

pub fn handle_sign(handle: &KeyringHandle, args: Value) -> Result<Value> {
    let key_id = require_str(&args, "key_id")?;
    let payload_b64 = require_str(&args, "payload_b64")?;
    let payload = b64_decode(payload_b64)?;
    let sig = handle.sign(key_id, &payload)?;
    Ok(json!({ "signature_b64": b64_encode(&sig) }))
}

/// **PR-N4 commit 2/N**. `<self>.keyring.federate_user_identity_token`
/// ability handler. Realm A's daemon raises a `UserBindingToken`
/// destined for `target_realm` (= the realm B the user wants to be
/// recognised on). The token's `source_realm` + `source_user_uri`
/// are taken from the daemon's bound device-subject; the
/// `source_user_pubkey` is the daemon's `agent_signing` entry's
/// public key under the RFC-002 single-key model. The token is
/// signed with that same entry's private key.
///
/// JSON wire shape:
/// ```text
/// args:    { "target_realm": "<realm-b>", "issued_at_unix_ms": <u64> }
/// returns: { "token": <json UserBindingToken>,
///            "transport_hint": "jwt-custom-claim" }
/// ```
///
/// `issued_at_unix_ms` is caller-supplied so the test harness can
/// pin a fixed timestamp; production callers pass the current
/// epoch-ms. The handler MAY round to seconds in a future commit
/// for backend JWT-encoding compatibility, but v1 keeps ms.
pub fn handle_federate_user_identity_token(handle: &KeyringHandle, args: Value) -> Result<Value> {
    let target_realm = require_str(&args, "target_realm")?.to_string();
    if target_realm.is_empty() {
        return Err(anyhow!("target_realm must be non-empty"));
    }
    let issued_at_ms = args
        .get("issued_at_unix_ms")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("missing required u64 field `issued_at_unix_ms`"))?;

    // Source identity comes from the daemon's bound device-
    // subject. INV-1 is enforced by construction: the signing
    // key (purpose=agent_signing) is the daemon's own backend
    // identity, and that's the same key the consuming realm
    // resolves via FederatedKeyResolver / federation.resolve_key.
    let source_user_uri = handle.device_subject().ok_or_else(|| {
        anyhow!(
            "daemon has no bound device-subject; \
             call <self>.keyring.bind_subject before raising user binding tokens"
        )
    })?;
    let source_realm = parse_realm_from_uri(&source_user_uri).ok_or_else(|| {
        anyhow!(
            "device-subject {source_user_uri:?} is not a canonical \
             easynet:///r/<realm>/agent/<id> URI"
        )
    })?;
    if source_realm == target_realm {
        return Err(anyhow!(
            "target_realm equals source_realm (`{source_realm}`); \
             a token issued for the daemon's own realm has no federated meaning"
        ));
    }

    // Pick the active agent_signing entry — the daemon's backend
    // identity. RFC-002 single-key model means there's exactly
    // one such active entry per ring.
    let signing_entry = handle
        .list_entries()
        .into_iter()
        .find(|e| e.purpose == "agent_signing" && e.status == KeyStatus::Active)
        .ok_or_else(|| {
            anyhow!(
                "no active agent_signing entry in keyring; \
             call <self>.keyring.create with purpose=agent_signing first"
            )
        })?;
    let pubkey_raw = b64_decode(&signing_entry.public_key_b64)?;
    let mut source_user_pubkey = [0u8; ED25519_PUBKEY_LEN];
    if pubkey_raw.len() != ED25519_PUBKEY_LEN {
        return Err(anyhow!(
            "agent_signing entry has wrong-length pubkey: {} bytes (expected {})",
            pubkey_raw.len(),
            ED25519_PUBKEY_LEN
        ));
    }
    source_user_pubkey.copy_from_slice(&pubkey_raw);

    // Generate a fresh CSPRNG nonce so two tokens issued with
    // identical timestamps still distinguish on the consumer's
    // replay store.
    let mut nonce = [0u8; USER_BINDING_NONCE_LEN];
    use rand::RngCore;
    rand::thread_rng().fill_bytes(&mut nonce);

    let mut token = UserBindingToken::new_unsigned(
        source_realm.to_string(),
        source_user_uri.clone(),
        source_user_pubkey,
        target_realm,
        issued_at_ms,
        nonce,
    );
    let canonical = super::user_binding_chain::canonical_user_binding_bytes(&token);
    let sig = handle.sign(&signing_entry.key_id, &canonical)?;
    token.signature = sig.to_vec();

    Ok(json!({
        "token": token,
        "transport_hint": "jwt-custom-claim",
    }))
}

/// Parse the realm slice from a canonical EasyNet URI
/// (`easynet:///r/<realm>/agent/<id>`). Returns `None` for any
/// malformed shape. Inlined here rather than imported from
/// `services::axon_serve` to keep the keyring layer free of an
/// `axon-pb` feature dependency.
fn parse_realm_from_uri(uri: &str) -> Option<&str> {
    let rest = uri.strip_prefix("easynet:///r/")?;
    let (realm, _tail) = rest.split_once("/agent/")?;
    if realm.is_empty() {
        return None;
    }
    Some(realm)
}

/// **PR-N4 commit 3/N**. `<self>.keyring.consume_federate_user_token`
/// ability handler. Realm B's user (already authenticated on
/// realm B with `local_user_id`) consumes a `UserBindingToken`
/// issued by realm A. On success, a `FederatedUserBinding` row
/// is written to the daemon's federated bindings store —
/// subsequent cross-realm discovery / device-listing surfaces
/// can then match `(source_realm, source_user_uri) →
/// local_user_id` to filter the user's devices across realms.
///
/// Four-check verify chain per spec §commit 3/N (in evaluation
/// order — fastest checks first to short-circuit attacks):
///   1. `target_realm == self_realm` — INV-3 unidirectional;
///       a token issued for realm C cannot be replayed at us.
///   2. `issued_at_ms` is within `USER_BINDING_FRESHNESS_MS`
///       of `now_ms` — bounds the replay window even if the
///       per-nonce store loses state.
///   3. token signature verifies via the embedded
///       `source_user_pubkey` (and the canonical bytes shape
///       from commit 1/N). The full PR-N2 cross-realm-pubkey-
///       belongs-to-source-realm-backend check is added in
///       commit 4/N's `FederatedUserResolver` layer; the
///       structural verify here proves the bytes were signed
///       by whoever holds the private key matching the
///       embedded source_user_pubkey, which combined with
///       INV-2 (consumer is in an authenticated session) +
///       replay defence is meaningful at v1.
///   4. nonce is not in the consumer's replay store for this
///       source_realm — INV-3 dedup.
///
/// JSON wire shape:
/// ```text
/// args: {
///   "token": <UserBindingToken JSON>,
///   "self_realm": "<realm-b>",
///   "local_user_id": "<consumer's session user id>",
///   "now_unix_ms": <u64>,                  // caller-supplied for testability
/// }
/// returns: {
///   "binding_recorded": true,
///   "source_realm": "<realm-a>",
///   "source_user_uri": "<...>",
///   "local_user_id": "<...>",
/// }
/// ```
///
/// The `now_unix_ms` is caller-supplied so tests can pin a
/// deterministic clock; production callers (the consume bridge
/// or the backend's HTTP path) pass the current epoch-ms.
pub fn handle_consume_federate_user_token(
    bindings: &FederatedBindingsStore,
    args: Value,
) -> Result<Value> {
    let self_realm = require_str(&args, "self_realm")?.to_string();
    if self_realm.is_empty() {
        return Err(anyhow!("self_realm must be non-empty"));
    }
    let local_user_id = require_str(&args, "local_user_id")?.to_string();
    if local_user_id.is_empty() {
        return Err(anyhow!("local_user_id must be non-empty"));
    }
    let now_ms = args
        .get("now_unix_ms")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("missing required u64 field `now_unix_ms`"))?;

    let token: UserBindingToken = serde_json::from_value(
        args.get("token")
            .cloned()
            .ok_or_else(|| anyhow!("missing required field `token`"))?,
    )
    .map_err(|err| anyhow!("token JSON shape: {err}"))?;

    // ── Check 1: target_realm matches us ──
    if token.target_realm != self_realm {
        let err = UserBindingError::WrongTargetRealm {
            expected: self_realm.clone(),
            actual: token.target_realm.clone(),
        };
        return Err(anyhow!("{}", err));
    }

    // ── Check 2: freshness window ──
    if now_ms.saturating_sub(token.issued_at_ms) > USER_BINDING_FRESHNESS_MS {
        let err = UserBindingError::ExpiredToken {
            issued_at_ms: token.issued_at_ms,
            now_ms,
        };
        return Err(anyhow!("{}", err));
    }
    // Future-dated tokens are also rejected; an issuer with a
    // wildly skewed clock cannot extend the window arbitrarily.
    if token.issued_at_ms > now_ms.saturating_add(USER_BINDING_FRESHNESS_MS) {
        let err = UserBindingError::ExpiredToken {
            issued_at_ms: token.issued_at_ms,
            now_ms,
        };
        return Err(anyhow!("future-dated token: {}", err));
    }

    // ── Check 3: signature verifies ──
    verify_user_binding_signature(&token).map_err(|err| anyhow!("{}", err))?;

    // ── Check 4: replay ──
    let nonce_b64 = b64_encode(&token.nonce);
    if bindings.nonce_seen(&token.source_realm, &nonce_b64) {
        return Err(anyhow!("{}", UserBindingError::ReplayDetected));
    }

    // All checks passed — record.
    let binding = FederatedUserBinding {
        source_realm: token.source_realm.clone(),
        source_user_uri: token.source_user_uri.clone(),
        source_user_pubkey_b64: b64_encode(&token.source_user_pubkey),
        local_user_id: local_user_id.clone(),
        bound_at_unix_ms: i64::try_from(now_ms).unwrap_or(i64::MAX),
    };
    bindings.record_binding(binding, nonce_b64)?;

    Ok(json!({
        "binding_recorded": true,
        "source_realm":     token.source_realm,
        "source_user_uri":  token.source_user_uri,
        "local_user_id":    local_user_id,
    }))
}

pub fn handle_rotate(handle: &KeyringHandle, args: Value) -> Result<Value> {
    let key_id = require_str(&args, "key_id")?;
    let (new_id, retired_id, epoch) = handle.rotate(key_id)?;
    Ok(json!({
        "new_key_id":     new_id,
        "retired_key_id": retired_id,
        "rotation_epoch": epoch,
    }))
}

pub fn handle_revoke(handle: &KeyringHandle, args: Value) -> Result<Value> {
    let key_id = require_str(&args, "key_id")?;
    let reason = args.get("reason").and_then(|v| v.as_str()).unwrap_or("");
    let ts = handle.revoke(key_id, reason)?;
    Ok(json!({ "tombstone_unix_ms": ts }))
}

pub fn handle_expire_set(handle: &KeyringHandle, args: Value) -> Result<Value> {
    let key_id = require_str(&args, "key_id")?;
    let expires_unix_ms = args
        .get("expires_unix_ms")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| anyhow!("missing required i64 field `expires_unix_ms`"))?;
    handle.set_expiry(key_id, expires_unix_ms)?;
    Ok(json!({ "ok": true }))
}

pub fn handle_bind_subject(handle: &KeyringHandle, args: Value) -> Result<Value> {
    let key_id = require_str(&args, "key_id")?;
    let subject_id = require_str(&args, "subject_id")?;
    handle.bind_subject(key_id, subject_id)?;
    Ok(json!({ "ok": true }))
}

pub fn handle_peer_add(handle: &KeyringHandle, args: Value) -> Result<Value> {
    let peer_uri = require_str(&args, "peer_uri")?;
    let public_key = require_str(&args, "public_key")?;
    let fingerprint = args.get("fingerprint").and_then(|v| v.as_str());
    let via_hub = args.get("via_hub").and_then(|v| v.as_str());
    let added = handle.peer_add(peer_uri, public_key, fingerprint, via_hub)?;
    Ok(json!({ "added": added }))
}

pub fn handle_peer_list(handle: &KeyringHandle, _args: Value) -> Result<Value> {
    let peers: Vec<Value> = handle
        .list_peers()
        .into_iter()
        .map(|p| {
            json!({
                "peer_uri":       p.peer_uri,
                "fingerprint":    p.fingerprint_b64,
                "public_key":     p.public_key_b64,
                "status":         match p.status { PeerStatus::Trusted => "trusted", PeerStatus::Suspended => "suspended", PeerStatus::Revoked => "revoked" },
                "via_hub":        p.via_hub,
                "added_unix_ms":  p.added_unix_ms,
                "last_seen_unix_ms": p.last_seen_unix_ms,
            })
        })
        .collect();
    Ok(json!({ "peers": peers }))
}

/// Register all 10 keyring abilities under `<owner>.keyring.<verb>`.
///
/// `owner` is the agent name they publish under (typically `"<self>"`
/// for the daemon's self-bundle).
pub fn register_for_owner(reg: &mut LocalAbilityRegistry, owner: &str, handle: Arc<KeyringHandle>) {
    let name = |verb: &str| format!("{owner}.keyring.{verb}");

    let h = handle.clone();
    reg.register_rpc(
        &name("create"),
        Arc::new(move |args| handle_create(&h, args)),
    );
    let h = handle.clone();
    reg.register_rpc(&name("list"), Arc::new(move |args| handle_list(&h, args)));
    let h = handle.clone();
    reg.register_rpc(
        &name("get_public"),
        Arc::new(move |args| handle_get_public(&h, args)),
    );
    let h = handle.clone();
    reg.register_rpc(&name("sign"), Arc::new(move |args| handle_sign(&h, args)));
    let h = handle.clone();
    reg.register_rpc(
        &name("rotate"),
        Arc::new(move |args| handle_rotate(&h, args)),
    );
    let h = handle.clone();
    reg.register_rpc(
        &name("revoke"),
        Arc::new(move |args| handle_revoke(&h, args)),
    );
    let h = handle.clone();
    reg.register_rpc(
        &name("expire_set"),
        Arc::new(move |args| handle_expire_set(&h, args)),
    );
    let h = handle.clone();
    reg.register_rpc(
        &name("bind_subject"),
        Arc::new(move |args| handle_bind_subject(&h, args)),
    );
    let h = handle.clone();
    reg.register_rpc(
        &name("peer_add"),
        Arc::new(move |args| handle_peer_add(&h, args)),
    );
    let h = handle.clone();
    reg.register_rpc(
        &name("peer_list"),
        Arc::new(move |args| handle_peer_list(&h, args)),
    );
    let h = handle.clone();
    reg.register_rpc(
        &name("federate_user_identity_token"),
        Arc::new(move |args| handle_federate_user_identity_token(&h, args)),
    );
}

/// **PR-N4 commit 3/N**. Register the consumer-side
/// `<self>.keyring.consume_federate_user_token` ability under
/// `owner`. Kept as a separate registration function rather
/// than folding into `register_for_owner` because the bindings
/// store has a different lifecycle than the keyring handle —
/// production daemons construct one bindings store per process
/// from a path, while the keyring handle is per-ring.
pub fn register_federated_consume_for_owner(
    reg: &mut LocalAbilityRegistry,
    owner: &str,
    bindings: Arc<FederatedBindingsStore>,
) {
    let name = format!("{owner}.keyring.consume_federate_user_token");
    let b = bindings.clone();
    reg.register_rpc(
        &name,
        Arc::new(move |args| handle_consume_federate_user_token(&b, args)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle() -> (Arc<KeyringHandle>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keyring.json");
        let h = Arc::new(KeyringHandle::open_or_create(path, "p").unwrap());
        (h, dir)
    }

    #[test]
    fn create_then_list_then_get_public() {
        let (h, _d) = handle();
        let created = handle_create(&h, json!({"purpose": "agent_signing"})).unwrap();
        let key_id = created["key_id"].as_str().unwrap().to_string();

        let listed = handle_list(&h, json!({})).unwrap();
        let entries = listed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["key_id"], json!(key_id));

        let pub_view = handle_get_public(&h, json!({"key_id": key_id})).unwrap();
        assert_eq!(pub_view["status"], json!("active"));
        assert!(pub_view["public_key"].as_str().unwrap().len() > 0);
    }

    #[test]
    fn sign_then_externally_verify_via_handler() {
        let (h, _d) = handle();
        let created = handle_create(&h, json!({"purpose": "agent_signing"})).unwrap();
        let key_id = created["key_id"].as_str().unwrap();
        let pk_b64 = created["public_key"].as_str().unwrap();
        let payload_b64 = b64_encode(b"federation envelope bytes");
        let signed =
            handle_sign(&h, json!({"key_id": key_id, "payload_b64": payload_b64})).unwrap();
        let sig_b64 = signed["signature_b64"].as_str().unwrap();

        let pk = b64_decode(pk_b64).unwrap();
        let sig = b64_decode(sig_b64).unwrap();
        let pk_arr: [u8; 32] = pk.try_into().unwrap();
        let sig_arr: [u8; 64] = sig.try_into().unwrap();
        use ed25519_dalek::{Verifier, VerifyingKey};
        let vk = VerifyingKey::from_bytes(&pk_arr).unwrap();
        let sig_obj = ed25519_dalek::Signature::from_bytes(&sig_arr);
        assert!(vk.verify(b"federation envelope bytes", &sig_obj).is_ok());
    }

    #[test]
    fn rotate_then_revoke_round_trip() {
        let (h, _d) = handle();
        let c = handle_create(&h, json!({"purpose": "x"})).unwrap();
        let k1 = c["key_id"].as_str().unwrap().to_string();
        let r = handle_rotate(&h, json!({"key_id": k1})).unwrap();
        let k2 = r["new_key_id"].as_str().unwrap().to_string();
        assert_eq!(r["rotation_epoch"], json!(1));
        // Revoke the new one too:
        let rev = handle_revoke(&h, json!({"key_id": k2, "reason": "compromise"})).unwrap();
        assert!(rev["tombstone_unix_ms"].as_i64().unwrap() > 0);
        // Cannot sign with revoked.
        let payload_b64 = b64_encode(b"x");
        assert!(handle_sign(&h, json!({"key_id": k2, "payload_b64": payload_b64}),).is_err());
    }

    #[test]
    fn peer_add_then_list_round_trip() {
        let (h, _d) = handle();
        let entry = handle_create(&h, json!({"purpose": "x"})).unwrap();
        let pk = entry["public_key"].as_str().unwrap().to_string();
        let added = handle_peer_add(
            &h,
            json!({
                "peer_uri": "easynet:///r/org/reg/agent.alice",
                "public_key": pk,
                "via_hub": "easynet:///r/org/reg/agent.alice-hub"
            }),
        )
        .unwrap();
        assert_eq!(added["added"], json!(true));
        let listed = handle_peer_list(&h, json!({})).unwrap();
        let peers = listed["peers"].as_array().unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(
            peers[0]["peer_uri"],
            json!("easynet:///r/org/reg/agent.alice")
        );
        assert_eq!(peers[0]["status"], json!("trusted"));
    }

    #[test]
    fn bind_subject_filters_list() {
        let (h, _d) = handle();
        handle_create(&h, json!({"purpose": "p1"})).unwrap();
        handle_create(&h, json!({"purpose": "p2"})).unwrap();
        let listed = handle_list(&h, json!({"filter": {"purpose": "p1"}})).unwrap();
        let entries = listed["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["purpose"], json!("p1"));
    }

    #[test]
    fn expire_set_and_bind_subject_persist() {
        let (h, _d) = handle();
        let created = handle_create(&h, json!({"purpose": "x"})).unwrap();
        let key_id = created["key_id"].as_str().unwrap();
        handle_expire_set(
            &h,
            json!({"key_id": key_id, "expires_unix_ms": 9_999_999_999i64}),
        )
        .unwrap();
        handle_bind_subject(
            &h,
            json!({"key_id": key_id, "subject_id": "easynet:///r/prv/reg/agent.foo"}),
        )
        .unwrap();
        let listed = handle_list(&h, json!({})).unwrap();
        let e = &listed["entries"][0];
        assert_eq!(e["expires_unix_ms"], json!(9_999_999_999i64));
        assert_eq!(e["bound_subject"], json!("easynet:///r/prv/reg/agent.foo"));
    }

    // ── PR-N4 commit 2/N — federate_user_identity_token ──────

    fn handle_with_subject_and_signing_key() -> (Arc<KeyringHandle>, tempfile::TempDir) {
        let (h, d) = handle();
        h.set_device_subject("easynet:///r/realm-a/agent/user-c".to_string())
            .unwrap();
        // Create the agent_signing entry the federate token
        // handler picks up.
        handle_create(&h, json!({"purpose": "agent_signing"})).unwrap();
        (h, d)
    }

    #[test]
    fn federate_user_identity_token_happy_path() {
        let (h, _d) = handle_with_subject_and_signing_key();
        let resp = handle_federate_user_identity_token(
            &h,
            json!({
                "target_realm": "realm-b",
                "issued_at_unix_ms": 1_714_500_000_000_u64,
            }),
        )
        .expect("token issued");
        assert_eq!(resp["transport_hint"], json!("jwt-custom-claim"));
        let token = &resp["token"];
        assert_eq!(token["source_realm"], json!("realm-a"));
        assert_eq!(
            token["source_user_uri"],
            json!("easynet:///r/realm-a/agent/user-c")
        );
        assert_eq!(token["target_realm"], json!("realm-b"));
        assert_eq!(token["issued_at_ms"], json!(1_714_500_000_000_u64));
        // Source pubkey + signature length sanity.
        assert_eq!(
            token["source_user_pubkey"].as_array().unwrap().len(),
            32,
            "source_user_pubkey must be 32 bytes"
        );
        assert_eq!(
            token["signature"].as_array().unwrap().len(),
            64,
            "signature must be 64 bytes"
        );
        assert_eq!(
            token["nonce"].as_array().unwrap().len(),
            32,
            "nonce must be 32 random bytes"
        );
    }

    #[test]
    fn federate_user_identity_token_signature_verifies_via_chain() {
        // The token issued by this handler must verify with the
        // user_binding_chain::verify_user_binding_signature
        // function — the consuming realm's gate.
        let (h, _d) = handle_with_subject_and_signing_key();
        let resp = handle_federate_user_identity_token(
            &h,
            json!({
                "target_realm": "realm-b",
                "issued_at_unix_ms": 1_714_500_000_000_u64,
            }),
        )
        .unwrap();
        let token: super::super::user_binding_chain::UserBindingToken =
            serde_json::from_value(resp["token"].clone()).expect("deserialise token");
        super::super::user_binding_chain::verify_user_binding_signature(&token)
            .expect("issued token signature must verify");
    }

    #[test]
    fn federate_user_identity_token_two_calls_have_distinct_nonces() {
        // INV-3 / replay defence: two tokens issued back-to-back
        // with the same target_realm + issued_at MUST have
        // distinct nonces, so the consuming realm's replay
        // store can dedup them as separate calls.
        let (h, _d) = handle_with_subject_and_signing_key();
        let args = json!({
            "target_realm": "realm-b",
            "issued_at_unix_ms": 1_714_500_000_000_u64,
        });
        let r1 = handle_federate_user_identity_token(&h, args.clone()).unwrap();
        let r2 = handle_federate_user_identity_token(&h, args).unwrap();
        assert_ne!(r1["token"]["nonce"], r2["token"]["nonce"]);
    }

    #[test]
    fn federate_user_identity_token_rejects_self_target_realm() {
        // INV-3 unidirectional: the source realm cannot issue a
        // binding for itself; that's not a federated assertion,
        // just self-loop noise. Reject early.
        let (h, _d) = handle_with_subject_and_signing_key();
        let err = handle_federate_user_identity_token(
            &h,
            json!({
                "target_realm": "realm-a", // = source realm
                "issued_at_unix_ms": 1_714_500_000_000_u64,
            }),
        )
        .expect_err("must reject self-realm target");
        assert!(
            err.to_string().contains("source_realm"),
            "rejection must explain why; got: {err}"
        );
    }

    #[test]
    fn federate_user_identity_token_requires_bound_subject() {
        // Without `set_device_subject`, the daemon has no
        // identity to issue tokens about. Reject.
        let (h, _d) = handle();
        handle_create(&h, json!({"purpose": "agent_signing"})).unwrap();
        let err = handle_federate_user_identity_token(
            &h,
            json!({
                "target_realm": "realm-b",
                "issued_at_unix_ms": 1_714_500_000_000_u64,
            }),
        )
        .expect_err("must reject without device-subject");
        assert!(
            err.to_string().contains("bound device-subject"),
            "rejection must explain missing identity; got: {err}"
        );
    }

    #[test]
    fn federate_user_identity_token_requires_active_signing_entry() {
        // Subject set but no agent_signing entry. Reject.
        let (h, _d) = handle();
        h.set_device_subject("easynet:///r/realm-a/agent/user-c".to_string())
            .unwrap();
        let err = handle_federate_user_identity_token(
            &h,
            json!({
                "target_realm": "realm-b",
                "issued_at_unix_ms": 1_714_500_000_000_u64,
            }),
        )
        .expect_err("must reject without agent_signing entry");
        assert!(
            err.to_string().contains("agent_signing"),
            "rejection must explain missing entry; got: {err}"
        );
    }

    #[test]
    fn federate_user_identity_token_requires_canonical_uri() {
        // Subject set to a non-canonical URI. Reject.
        let (h, _d) = handle();
        h.set_device_subject("not-a-canonical-uri".to_string())
            .unwrap();
        handle_create(&h, json!({"purpose": "agent_signing"})).unwrap();
        let err = handle_federate_user_identity_token(
            &h,
            json!({
                "target_realm": "realm-b",
                "issued_at_unix_ms": 1_714_500_000_000_u64,
            }),
        )
        .expect_err("must reject malformed device-subject");
        assert!(err.to_string().contains("canonical"));
    }

    #[test]
    fn federate_user_identity_token_rejects_empty_target_realm() {
        let (h, _d) = handle_with_subject_and_signing_key();
        let err = handle_federate_user_identity_token(
            &h,
            json!({
                "target_realm": "",
                "issued_at_unix_ms": 1_714_500_000_000_u64,
            }),
        )
        .expect_err("must reject empty target_realm");
        assert!(err.to_string().contains("non-empty"));
    }

    // ── PR-N4 commit 3/N — consume_federate_user_token ───────

    /// Issue a token from realm A (using a fresh keyring) and
    /// return both the JSON token + the source-realm-pubkey so
    /// the test driver can wire up the consumer side.
    fn issue_token_from_realm_a(target_realm: &str, issued_at_ms: u64) -> Value {
        let (h, _d) = handle();
        h.set_device_subject("easynet:///r/realm-a/agent/user-c".to_string())
            .unwrap();
        handle_create(&h, json!({"purpose": "agent_signing"})).unwrap();
        let resp = handle_federate_user_identity_token(
            &h,
            json!({
                "target_realm": target_realm,
                "issued_at_unix_ms": issued_at_ms,
            }),
        )
        .unwrap();
        resp["token"].clone()
    }

    #[test]
    fn consume_federate_user_token_happy_path() {
        let token = issue_token_from_realm_a("realm-b", 1_714_500_000_000);
        let bindings = FederatedBindingsStore::in_memory();
        let resp = handle_consume_federate_user_token(
            &bindings,
            json!({
                "token": token,
                "self_realm": "realm-b",
                "local_user_id": "user-c-on-realm-b",
                "now_unix_ms": 1_714_500_000_000_u64 + 1_000,
            }),
        )
        .expect("consume happy path");
        assert_eq!(resp["binding_recorded"], json!(true));
        assert_eq!(resp["source_realm"], json!("realm-a"));
        assert_eq!(resp["local_user_id"], json!("user-c-on-realm-b"));
        // Binding was actually written.
        let bound = bindings
            .find_local_user("realm-a", "easynet:///r/realm-a/agent/user-c")
            .expect("binding present");
        assert_eq!(bound, "user-c-on-realm-b");
    }

    #[test]
    fn consume_federate_user_token_rejects_wrong_target_realm() {
        let token = issue_token_from_realm_a("realm-b", 1_714_500_000_000);
        let bindings = FederatedBindingsStore::in_memory();
        let err = handle_consume_federate_user_token(
            &bindings,
            json!({
                "token": token,
                "self_realm": "realm-c", // = NOT what the token targets
                "local_user_id": "user",
                "now_unix_ms": 1_714_500_000_000_u64 + 1_000,
            }),
        )
        .expect_err("must reject wrong target_realm");
        assert!(err.to_string().contains("wrong target_realm"));
    }

    #[test]
    fn consume_federate_user_token_rejects_expired_token() {
        let token = issue_token_from_realm_a("realm-b", 1_714_500_000_000);
        let bindings = FederatedBindingsStore::in_memory();
        // now is well past issued_at + freshness window (24h).
        let err = handle_consume_federate_user_token(
            &bindings,
            json!({
                "token": token,
                "self_realm": "realm-b",
                "local_user_id": "u",
                "now_unix_ms": 1_714_500_000_000_u64 + 25 * 60 * 60 * 1000,
            }),
        )
        .expect_err("must reject expired token");
        assert!(err.to_string().contains("expired token"));
    }

    #[test]
    fn consume_federate_user_token_rejects_future_dated_token() {
        // Token issued far in the "future" relative to consumer
        // clock. Reject — an issuer with skewed clock cannot
        // extend the freshness window arbitrarily.
        let issued_ahead = 1_714_500_000_000_u64 + 100 * 60 * 60 * 1000;
        let token = issue_token_from_realm_a("realm-b", issued_ahead);
        let bindings = FederatedBindingsStore::in_memory();
        let err = handle_consume_federate_user_token(
            &bindings,
            json!({
                "token": token,
                "self_realm": "realm-b",
                "local_user_id": "u",
                "now_unix_ms": 1_714_500_000_000_u64,
            }),
        )
        .expect_err("must reject future-dated token");
        assert!(err.to_string().contains("future-dated"));
    }

    #[test]
    fn consume_federate_user_token_rejects_tampered_signature() {
        let mut token = issue_token_from_realm_a("realm-b", 1_714_500_000_000);
        // Flip the first byte of the signature.
        let sig = token["signature"].as_array_mut().unwrap();
        let first = sig[0].as_u64().unwrap();
        sig[0] = json!((first ^ 0x01) as u8);
        let bindings = FederatedBindingsStore::in_memory();
        let err = handle_consume_federate_user_token(
            &bindings,
            json!({
                "token": token,
                "self_realm": "realm-b",
                "local_user_id": "u",
                "now_unix_ms": 1_714_500_000_000_u64 + 1_000,
            }),
        )
        .expect_err("tampered signature must reject");
        assert!(err.to_string().contains("invalid signature"));
    }

    #[test]
    fn consume_federate_user_token_rejects_replay() {
        let token = issue_token_from_realm_a("realm-b", 1_714_500_000_000);
        let bindings = FederatedBindingsStore::in_memory();
        let args = json!({
            "token": token,
            "self_realm": "realm-b",
            "local_user_id": "u",
            "now_unix_ms": 1_714_500_000_000_u64 + 1_000,
        });
        // First consume succeeds.
        handle_consume_federate_user_token(&bindings, args.clone()).unwrap();
        // Second consume of the SAME token (same nonce) is replay.
        let err =
            handle_consume_federate_user_token(&bindings, args).expect_err("replay must reject");
        assert!(err.to_string().contains("replay detected"));
    }

    #[test]
    fn consume_federate_user_token_rejects_empty_self_realm() {
        let token = issue_token_from_realm_a("realm-b", 1_714_500_000_000);
        let bindings = FederatedBindingsStore::in_memory();
        let err = handle_consume_federate_user_token(
            &bindings,
            json!({
                "token": token,
                "self_realm": "",
                "local_user_id": "u",
                "now_unix_ms": 1_714_500_000_000_u64,
            }),
        )
        .expect_err("empty self_realm must reject");
        assert!(err.to_string().contains("non-empty"));
    }

    #[test]
    fn consume_federate_user_token_rejects_empty_local_user_id() {
        let token = issue_token_from_realm_a("realm-b", 1_714_500_000_000);
        let bindings = FederatedBindingsStore::in_memory();
        let err = handle_consume_federate_user_token(
            &bindings,
            json!({
                "token": token,
                "self_realm": "realm-b",
                "local_user_id": "",
                "now_unix_ms": 1_714_500_000_000_u64,
            }),
        )
        .expect_err("empty local_user_id must reject");
        assert!(err.to_string().contains("non-empty"));
    }

    #[test]
    fn consume_federate_user_token_full_round_trip_realm_a_to_realm_b() {
        // End-to-end: realm A daemon issues, realm B daemon
        // consumes. This is the cross-realm path — the token's
        // bytes leave realm A and the consumer's keyring on B
        // never had the source pubkey before.
        let token = issue_token_from_realm_a("realm-b", 1_714_500_000_000);
        // Build a fresh realm B store (no prior knowledge of A).
        let bindings = FederatedBindingsStore::in_memory();
        let resp = handle_consume_federate_user_token(
            &bindings,
            json!({
                "token": token,
                "self_realm": "realm-b",
                "local_user_id": "user-c-realm-b-id",
                "now_unix_ms": 1_714_500_000_001_u64,
            }),
        )
        .unwrap();
        assert_eq!(resp["binding_recorded"], json!(true));
        // Realm B can now look up the cross-realm user.
        let local_id = bindings
            .find_local_user("realm-a", "easynet:///r/realm-a/agent/user-c")
            .expect("binding present");
        assert_eq!(local_id, "user-c-realm-b-id");
    }
}
