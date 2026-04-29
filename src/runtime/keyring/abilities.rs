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

use super::handle::KeyringHandle;
use super::store::{Entry, KeyStatus, PeerStatus};
use crate::runtime::ability_dispatch::LocalAbilityRegistry;

fn b64_encode(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    STANDARD.encode(bytes)
}

fn b64_decode(s: &str) -> Result<Vec<u8>> {
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;
    STANDARD.decode(s).map_err(|e| anyhow!("base64 decode: {e}"))
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
pub fn register_for_owner(
    reg: &mut LocalAbilityRegistry,
    owner: &str,
    handle: Arc<KeyringHandle>,
) {
    let name = |verb: &str| format!("{owner}.keyring.{verb}");

    let h = handle.clone();
    reg.register_rpc(
        &name("create"),
        Arc::new(move |args| handle_create(&h, args)),
    );
    let h = handle.clone();
    reg.register_rpc(
        &name("list"),
        Arc::new(move |args| handle_list(&h, args)),
    );
    let h = handle.clone();
    reg.register_rpc(
        &name("get_public"),
        Arc::new(move |args| handle_get_public(&h, args)),
    );
    let h = handle.clone();
    reg.register_rpc(
        &name("sign"),
        Arc::new(move |args| handle_sign(&h, args)),
    );
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
        let created =
            handle_create(&h, json!({"purpose": "agent_signing"})).unwrap();
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
        let created =
            handle_create(&h, json!({"purpose": "agent_signing"})).unwrap();
        let key_id = created["key_id"].as_str().unwrap();
        let pk_b64 = created["public_key"].as_str().unwrap();
        let payload_b64 = b64_encode(b"federation envelope bytes");
        let signed = handle_sign(
            &h,
            json!({"key_id": key_id, "payload_b64": payload_b64}),
        )
        .unwrap();
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
        assert!(handle_sign(
            &h,
            json!({"key_id": k2, "payload_b64": payload_b64}),
        )
        .is_err());
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
        assert_eq!(peers[0]["peer_uri"], json!("easynet:///r/org/reg/agent.alice"));
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
}
