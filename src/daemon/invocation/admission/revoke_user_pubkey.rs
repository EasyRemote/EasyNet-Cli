// EasyNet CLI — `identity.revoke_user_pubkey` ability handler
// ============================================================
//
// File: src/daemon/invocation/revoke_user_pubkey.rs
// DEC-EU §revocation. Removes a (user_ura, public_key_b64) entry
// from the daemon's `realm-trust.toml` and re-publishes the shared
// trust-anchor cell so subsequent admission calls cannot use the
// revoked key.
//
// Sister surface to `identity.register_pubkey`. Where register
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

use serde::Deserialize;
use tonic::Status;

use crate::daemon::invocation::admission::runtime_trust::RuntimeTrust;
use crate::daemon::trust::cell::SharedTrustAnchor;

/// Canonical daemon identity/trust ability name.
pub const ABILITY_IDENTITY_REVOKE_USER_PUBKEY: &str =
    crate::daemon::ability::names::federation::IDENTITY_REVOKE_USER_PUBKEY;

#[derive(Debug, Deserialize)]
struct RevokeArgs {
    agent_ura: String,
    public_key_b64: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RevokeUserPubkeyIntent {
    agent_ura: String,
    public_key_b64: String,
}

impl RevokeUserPubkeyIntent {
    pub(crate) fn agent_ura(&self) -> &str {
        &self.agent_ura
    }

    pub(crate) fn public_key_b64(&self) -> &str {
        &self.public_key_b64
    }

    #[cfg(test)]
    pub(crate) fn for_test(agent_ura: String, public_key_b64: String) -> Self {
        Self {
            agent_ura,
            public_key_b64,
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RevokeResponse {
    pub ok: bool,
    pub removed: bool,
}

pub(crate) struct RevokeWriteOutcome {
    pub(crate) body: Vec<u8>,
    pub(crate) removed: bool,
}

pub fn handle(
    arguments: &[u8],
    daemon_realm: &str,
    trust_anchor_path: &Path,
    cell: &SharedTrustAnchor,
) -> Result<Vec<u8>, Status> {
    Ok(handle_with_outcome(arguments, daemon_realm, trust_anchor_path, cell)?.body)
}

pub(crate) fn handle_with_outcome(
    arguments: &[u8],
    daemon_realm: &str,
    trust_anchor_path: &Path,
    cell: &SharedTrustAnchor,
) -> Result<RevokeWriteOutcome, Status> {
    let args: RevokeArgs = serde_json::from_slice(arguments).map_err(|err| {
        Status::invalid_argument(format!(
            "identity.revoke_user_pubkey: arguments JSON decode failed: {err}"
        ))
    })?;

    if args.agent_ura.is_empty() {
        return Err(Status::invalid_argument(
            "identity.revoke_user_pubkey: agent_ura is required",
        ));
    }
    if args.public_key_b64.is_empty() {
        return Err(Status::invalid_argument(
            "identity.revoke_user_pubkey: public_key_b64 is required",
        ));
    }
    let removed = RuntimeTrust::new(daemon_realm, trust_anchor_path, cell)
        .revoke_user_pubkey(&args.agent_ura, &args.public_key_b64)?;

    let body = serde_json::to_vec(&RevokeResponse { ok: true, removed }).map_err(|err| {
        Status::internal(format!(
            "identity.revoke_user_pubkey: response JSON encode failed: {err}"
        ))
    })?;
    Ok(RevokeWriteOutcome { body, removed })
}

pub(crate) fn parse_revoke_user_pubkey_intent(
    arguments: &[u8],
) -> Result<RevokeUserPubkeyIntent, Status> {
    let args: RevokeArgs = serde_json::from_slice(arguments).map_err(|err| {
        Status::invalid_argument(format!(
            "identity.revoke_user_pubkey: arguments JSON decode failed: {err}"
        ))
    })?;
    if args.agent_ura.is_empty() {
        return Err(Status::invalid_argument(
            "identity.revoke_user_pubkey: agent_ura is required",
        ));
    }
    if args.public_key_b64.is_empty() {
        return Err(Status::invalid_argument(
            "identity.revoke_user_pubkey: public_key_b64 is required",
        ));
    }
    Ok(RevokeUserPubkeyIntent {
        agent_ura: args.agent_ura,
        public_key_b64: args.public_key_b64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
    use crate::daemon::trust::cell::SharedTrustAnchor;
    use base64::prelude::*;
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
                origin_realm: None,
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
