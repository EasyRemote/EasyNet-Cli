// EasyNet CLI — `<self>.revoke_user_pubkey` ability handler
// ===========================================================
//
// File: src/services/axon_serve/revoke_user_pubkey.rs
// DEC-EU §revocation. Removes a (user_ura, public_key_b64) entry
// from the daemon's `realm-trust.toml` and re-publishes the shared
// trust-anchor cell so subsequent admission calls cannot use the
// revoked key.
//
// Sister surface to `<self>.register_device_pubkey`. Where register
// admits multiple roles (device / backend / hub / user), revoke is
// intentionally user-only — operator-curated hub / device entries
// are managed by hand in the TOML, never through a daemon ability.
//
// Inputs
// ------
//   {
//     "agent_ura":      "easynet:///r/{realm}/user/{user_id}",
//     "public_key_b64": "<base64 standard, 32-byte ed25519 vk>"
//   }
//
// Output
// ------
//   {"ok": true,  "removed": true}   — entry existed and was removed
//   {"ok": true,  "removed": false}  — no matching entry (idempotent
//                                      retry; treat as success on the
//                                      caller side)
//
// Realm scope
// -----------
// User entries currently must register in their home realm
// (`register_device_pubkey` enforces this in DEC-EU phase 1).
// Revocation enforces the same scope: `agent_ura.realm == daemon_realm`.
// Cross-realm user roaming is the DEC-EU §multi-realm followup
// (阶段 3.3); when that lands, this check relaxes in lockstep.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026-2027 EasyNet. All rights reserved.

use std::path::Path;
use std::sync::Arc;

use base64::prelude::*;
use ed25519_dalek::VerifyingKey;
use serde::Deserialize;
use tonic::Status;

use crate::services::realm_trust_anchor::{RealmTrustAnchor, RealmTrustError, TrustedAgent};
use crate::services::trust_anchor_cell::SharedTrustAnchor;

/// Wire-stable ability name. The backend pins this verbatim.
pub const ABILITY_SELF_REVOKE_USER_PUBKEY: &str = "<self>.revoke_user_pubkey";

#[derive(Debug, Deserialize)]
struct RevokeArgs {
    agent_ura: String,
    public_key_b64: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RevokeResponse {
    pub ok: bool,
    pub removed: bool,
}

pub fn handle(
    arguments: &[u8],
    daemon_realm: &str,
    trust_anchor_path: &Path,
    cell: &SharedTrustAnchor,
) -> Result<Vec<u8>, Status> {
    let args: RevokeArgs = serde_json::from_slice(arguments).map_err(|err| {
        Status::invalid_argument(format!(
            "<self>.revoke_user_pubkey: arguments JSON decode failed: {err}"
        ))
    })?;

    if args.agent_ura.is_empty() {
        return Err(Status::invalid_argument(
            "<self>.revoke_user_pubkey: agent_ura is required",
        ));
    }
    if args.public_key_b64.is_empty() {
        return Err(Status::invalid_argument(
            "<self>.revoke_user_pubkey: public_key_b64 is required",
        ));
    }
    validate_public_key_b64(&args.public_key_b64)?;

    let parsed_realm = parse_realm_from_ura(&args.agent_ura).ok_or_else(|| {
        Status::invalid_argument(format!(
            "<self>.revoke_user_pubkey: agent_ura `{}` does not match the canonical user URA",
            args.agent_ura,
        ))
    })?;
    if parsed_realm != daemon_realm {
        return Err(Status::permission_denied(format!(
            "<self>.revoke_user_pubkey: agent_ura realm `{parsed_realm}` must match daemon \
             realm `{daemon_realm}` (cross-realm user roaming is DEC-EU §multi-realm followup)",
        )));
    }

    // Snapshot, mutate, atomic save, publish — same pattern as
    // register_device_pubkey so a SIGHUP-triggered reload sees the
    // same authoritative state.
    let snapshot = cell.snapshot();
    let mut next_entries: Vec<TrustedAgent> = snapshot.entries_sorted();
    let mut next_anchor =
        RealmTrustAnchor::from_entries(next_entries.split_off(0)).map_err(realm_error_to_status)?;
    let removed = next_anchor
        .remove_user_pubkey(&args.agent_ura, &args.public_key_b64)
        .map_err(realm_error_to_status)?;

    next_anchor
        .save(trust_anchor_path)
        .map_err(realm_error_to_status)?;
    cell.replace(Arc::new(next_anchor));

    serde_json::to_vec(&RevokeResponse { ok: true, removed }).map_err(|err| {
        Status::internal(format!(
            "<self>.revoke_user_pubkey: response JSON encode failed: {err}"
        ))
    })
}

fn validate_public_key_b64(raw: &str) -> Result<(), Status> {
    let decoded = BASE64_STANDARD.decode(raw).map_err(|err| {
        Status::invalid_argument(format!(
            "<self>.revoke_user_pubkey: public_key_b64 is not valid base64: {err}"
        ))
    })?;
    let bytes: [u8; 32] = decoded.as_slice().try_into().map_err(|_| {
        Status::invalid_argument(format!(
            "<self>.revoke_user_pubkey: public_key_b64 must decode to exactly 32 bytes, got {}",
            decoded.len()
        ))
    })?;
    VerifyingKey::from_bytes(&bytes).map_err(|err| {
        Status::invalid_argument(format!(
            "<self>.revoke_user_pubkey: public_key_b64 is not a valid Ed25519 verifying key: {err}"
        ))
    })?;
    Ok(())
}

/// Strip the canonical `easynet:///r/{realm}/...` prefix and return
/// the realm component. Mirrors register_device_pubkey's parser so
/// the two surfaces accept the same URA shape.
fn parse_realm_from_ura(ura: &str) -> Option<String> {
    crate::ura::parse_ura(ura).ok().map(|parsed| parsed.realm)
}

fn realm_error_to_status(err: RealmTrustError) -> Status {
    match err {
        RealmTrustError::InvalidUraForRole {
            agent_ura,
            role,
            detail,
        } => Status::invalid_argument(format!(
            "<self>.revoke_user_pubkey: {role} URA `{agent_ura}` invalid: {detail}"
        )),
        RealmTrustError::WriteFailed { path, source } => Status::internal(format!(
            "<self>.revoke_user_pubkey: write {path:?}: {source}"
        )),
        RealmTrustError::SerializeFailed { path, source } => Status::internal(format!(
            "<self>.revoke_user_pubkey: serialize {path:?}: {source}"
        )),
        other => Status::internal(format!("<self>.revoke_user_pubkey: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::realm_trust_anchor::TrustedAgentRole;
    use crate::services::trust_anchor_cell::SharedTrustAnchor;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn b64_pubkey() -> String {
        // Deterministic test pubkey — any 32 bytes that form a valid
        // Ed25519 point. Reusing the same vk across tests is fine; we
        // verify shape, not key derivation.
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        BASE64_STANDARD.encode(sk.verifying_key().to_bytes())
    }

    #[test]
    fn revoke_removes_existing_user_pubkey() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");

        let mut anchor = RealmTrustAnchor::default();
        let pubkey = b64_pubkey();
        anchor
            .append_agent(TrustedAgent {
                agent_ura: "easynet:///r/realm/user/alice".to_string(),
                public_key_b64: pubkey.clone(),
                role: TrustedAgentRole::User,
                added_at_unix_ms: 1_714_000_000_000,
                origin_tenant_id: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            })
            .expect("seed user entry");
        anchor.save(&path).expect("save");
        let cell = SharedTrustAnchor::new(Arc::new(anchor));

        let args = serde_json::to_vec(&json!({
            "agent_ura": "easynet:///r/realm/user/alice",
            "public_key_b64": pubkey,
        }))
        .unwrap();
        let body = handle(&args, "realm", &path, &cell).expect("ok");
        let resp: RevokeResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.ok);
        assert!(resp.removed);
        assert!(cell
            .snapshot()
            .lookup_user_by_pubkey("easynet:///r/realm/user/alice", &pubkey)
            .is_none());
    }

    #[test]
    fn revoke_is_idempotent_for_unknown_pubkey() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");
        let cell = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));

        let args = serde_json::to_vec(&json!({
            "agent_ura": "easynet:///r/realm/user/alice",
            "public_key_b64": b64_pubkey(),
        }))
        .unwrap();
        let body = handle(&args, "realm", &path, &cell).expect("ok");
        let resp: RevokeResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.ok);
        assert!(!resp.removed);
    }

    #[test]
    fn revoke_rejects_wrong_realm() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");
        let cell = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));

        let args = serde_json::to_vec(&json!({
            "agent_ura": "easynet:///r/other-realm/user/alice",
            "public_key_b64": b64_pubkey(),
        }))
        .unwrap();
        let err = handle(&args, "realm", &path, &cell).expect_err("expected reject");
        assert_eq!(err.code(), tonic::Code::PermissionDenied);
    }

    #[test]
    fn revoke_rejects_bad_pubkey_shape() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");
        let cell = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));

        let args = serde_json::to_vec(&json!({
            "agent_ura": "easynet:///r/realm/user/alice",
            "public_key_b64": "not-base64",
        }))
        .unwrap();
        let err = handle(&args, "realm", &path, &cell).expect_err("expected reject");
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
    }
}
