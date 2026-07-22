// EasyNet CLI — `identity.list_user_pubkeys` ability handler
// ===========================================================
//
// File: src/daemon/invocation/list_user_pubkeys.rs
// DEC-EU §multi-host-list. Read-only inventory of the realm-trust
// entries registered under a given user URA. Backend's list
// endpoint calls this instead of reading the TOML directly, so
// multi-host deployments (backend ↔ daemon on separate machines)
// stay consistent with what admission actually sees.
//
// Inputs
// ------
//   {"user_ura": "easynet:///r/{realm}/user/{user_id}"}
//
// Output
// ------
//   {
//     "user_ura": "...",
//     "keys": [{"public_key_b64": "...", "added_at_unix_ms": ...}, ...],
//     "rotation_epoch": 1,
//     "revoked_key_count": 1
//   }
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026-2027 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use tonic::Status;

use crate::daemon::invocation::admission::runtime_trust::RuntimeTrustReader;

pub const ABILITY_IDENTITY_LIST_USER_PUBKEYS: &str =
    crate::daemon::ability::names::federation::IDENTITY_LIST_USER_PUBKEYS;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    user_ura: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListedUserKey {
    pub public_key_b64: String,
    pub added_at_unix_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListResponse {
    pub user_ura: String,
    pub keys: Vec<ListedUserKey>,
    pub rotation_epoch: u64,
    pub revoked_key_count: usize,
}

pub(crate) fn handle(
    arguments: &[u8],
    runtime_trust: RuntimeTrustReader<'_>,
) -> Result<Vec<u8>, Status> {
    let args: ListArgs = serde_json::from_slice(arguments).map_err(|err| {
        Status::invalid_argument(format!(
            "identity.list_user_pubkeys: arguments JSON decode failed: {err}"
        ))
    })?;
    let user_ura = required_user_ura(&args.user_ura)?;

    let snapshot = runtime_trust.user_snapshot(&user_ura);
    let keys: Vec<ListedUserKey> = snapshot
        .keys
        .into_iter()
        .map(|key| ListedUserKey {
            public_key_b64: key.public_key_b64,
            added_at_unix_ms: key.added_at_unix_ms,
        })
        .collect();

    serde_json::to_vec(&ListResponse {
        user_ura: snapshot.user_ura,
        keys,
        rotation_epoch: snapshot.rotation_epoch,
        revoked_key_count: snapshot.revoked_key_count,
    })
    .map_err(|err| {
        Status::internal(format!(
            "identity.list_user_pubkeys: response JSON encode failed: {err}"
        ))
    })
}

fn required_user_ura(raw: &str) -> Result<String, Status> {
    let user_ura = raw.trim();
    if user_ura.is_empty() {
        return Err(Status::invalid_argument(
            "identity.list_user_pubkeys: user_ura is required",
        ));
    }
    let parsed = crate::core::ura::parse_ura(user_ura).map_err(|error| {
        Status::invalid_argument(format!(
            "identity.list_user_pubkeys: user_ura must be a canonical User URA: {error}"
        ))
    })?;
    if parsed.kind != crate::core::ura::URAKind::User {
        return Err(Status::invalid_argument(format!(
            "identity.list_user_pubkeys: user_ura must identify a User, got {:?}",
            parsed.kind
        )));
    }
    Ok(user_ura.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::invocation::admission::runtime_trust::RuntimeTrustReader;
    use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
    use crate::daemon::trust::cell::SharedTrustAnchor;
    use base64::prelude::*;
    use serde_json::json;
    use std::sync::Arc;

    fn b64_pubkey(seed: u8) -> String {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        BASE64_STANDARD.encode(sk.verifying_key().to_bytes())
    }

    #[test]
    fn list_returns_every_pubkey_for_user() {
        let mut anchor = RealmTrustAnchor::default();
        for seed in [1u8, 2, 3] {
            anchor
                .append_agent(TrustedAgent {
                    agent_ura: "easynet:///r/realm/user/alice".to_string(),
                    public_key_b64: b64_pubkey(seed),
                    role: TrustedAgentRole::User,
                    added_at_unix_ms: 1_700_000_000_000 + u64::from(seed),
                    origin_realm: None,
                    hub_endpoint: None,
                    tls_ca_pem_path: None,
                })
                .expect("append");
        }
        let cell = SharedTrustAnchor::new(Arc::new(anchor));
        let args =
            serde_json::to_vec(&json!({"user_ura": "easynet:///r/realm/user/alice"})).unwrap();
        let body = handle(&args, RuntimeTrustReader::new(&cell)).expect("ok");
        let resp: ListResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.user_ura, "easynet:///r/realm/user/alice");
        assert_eq!(resp.keys.len(), 3);
        assert_eq!(resp.rotation_epoch, 0);
        assert_eq!(resp.revoked_key_count, 0);
    }

    #[test]
    fn list_returns_empty_for_unknown_user() {
        let cell = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
        let args =
            serde_json::to_vec(&json!({"user_ura": "easynet:///r/realm/user/missing"})).unwrap();
        let body = handle(&args, RuntimeTrustReader::new(&cell)).expect("ok");
        let resp: ListResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.keys.is_empty());
        assert_eq!(resp.rotation_epoch, 0);
        assert_eq!(resp.revoked_key_count, 0);
    }

    #[test]
    fn list_includes_revocation_snapshot() {
        let user_ura = "easynet:///r/realm/user/alice";
        let revoked_key = b64_pubkey(1);
        let active_key = b64_pubkey(2);
        let mut anchor = RealmTrustAnchor::default();
        for key in [&revoked_key, &active_key] {
            anchor
                .append_agent(TrustedAgent {
                    agent_ura: user_ura.to_string(),
                    public_key_b64: key.to_string(),
                    role: TrustedAgentRole::User,
                    added_at_unix_ms: 1_700_000_000_000,
                    origin_realm: None,
                    hub_endpoint: None,
                    tls_ca_pem_path: None,
                })
                .expect("append");
        }
        anchor
            .revoke_user_pubkey(user_ura, &revoked_key, 1_700_000_100_000)
            .expect("revoke")
            .expect("tombstone");

        let cell = SharedTrustAnchor::new(Arc::new(anchor));
        let args = serde_json::to_vec(&json!({"user_ura": user_ura})).unwrap();
        let body = handle(&args, RuntimeTrustReader::new(&cell)).expect("ok");
        let resp: ListResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(resp.keys.len(), 1);
        assert_eq!(resp.keys[0].public_key_b64, active_key);
        assert_eq!(resp.rotation_epoch, 1);
        assert_eq!(resp.revoked_key_count, 1);
    }

    #[test]
    fn list_rejects_retired_agent_ura_request_field() {
        let cell = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
        let args =
            serde_json::to_vec(&json!({"agent_ura": "easynet:///r/realm/user/alice"})).unwrap();

        let error = handle(&args, RuntimeTrustReader::new(&cell))
            .expect_err("retired agent_ura input must fail");
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
        assert!(error.message().contains("unknown field `agent_ura`"));
    }

    #[test]
    fn list_rejects_non_user_ura_scope() {
        let cell = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
        let args =
            serde_json::to_vec(&json!({"user_ura": "easynet:///r/realm/device/dev-a"})).unwrap();

        let error = handle(&args, RuntimeTrustReader::new(&cell))
            .expect_err("device URA must not query user key inventory");
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
        assert!(error.message().contains("user_ura must identify a User"));
    }
}
