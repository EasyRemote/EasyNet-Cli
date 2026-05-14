// EasyNet CLI — `<self>.list_user_pubkeys` ability handler
// ==========================================================
//
// File: src/services/axon_serve/list_user_pubkeys.rs
// DEC-EU §multi-host-list. Read-only inventory of the realm-trust
// entries registered under a given user URI. Backend's list
// endpoint calls this instead of reading the TOML directly, so
// multi-host deployments (backend ↔ daemon on separate machines)
// stay consistent with what admission actually sees.
//
// Inputs
// ------
//   {"agent_ura": "easynet:///r/{realm}/user/{user_id}"}
//
// Output
// ------
//   {
//     "agent_ura": "...",
//     "keys": [{"public_key_b64": "...", "added_at_unix_ms": ...}, ...]
//   }
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026-2027 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use tonic::Status;

use crate::services::trust_anchor_cell::SharedTrustAnchor;

pub const ABILITY_SELF_LIST_USER_PUBKEYS: &str = "<self>.list_user_pubkeys";

#[derive(Debug, Deserialize)]
struct ListArgs {
    agent_ura: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListedUserKey {
    pub public_key_b64: String,
    pub added_at_unix_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListResponse {
    pub agent_ura: String,
    pub keys: Vec<ListedUserKey>,
}

pub fn handle(arguments: &[u8], cell: &SharedTrustAnchor) -> Result<Vec<u8>, Status> {
    let args: ListArgs = serde_json::from_slice(arguments).map_err(|err| {
        Status::invalid_argument(format!(
            "<self>.list_user_pubkeys: arguments JSON decode failed: {err}"
        ))
    })?;
    if args.agent_ura.is_empty() {
        return Err(Status::invalid_argument(
            "<self>.list_user_pubkeys: agent_ura is required",
        ));
    }

    let snapshot = cell.snapshot();
    let keys: Vec<ListedUserKey> = snapshot
        .lookup_user_all(&args.agent_ura)
        .iter()
        .map(|e| ListedUserKey {
            public_key_b64: e.public_key_b64.clone(),
            added_at_unix_ms: e.added_at_unix_ms,
        })
        .collect();

    serde_json::to_vec(&ListResponse {
        agent_ura: args.agent_ura,
        keys,
    })
    .map_err(|err| {
        Status::internal(format!(
            "<self>.list_user_pubkeys: response JSON encode failed: {err}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::realm_trust_anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
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
                    origin_tenant_id: None,
                    hub_uri: None,
                    tls_ca_pem_path: None,
                })
                .expect("append");
        }
        let cell = SharedTrustAnchor::new(Arc::new(anchor));
        let args =
            serde_json::to_vec(&json!({"agent_ura": "easynet:///r/realm/user/alice"})).unwrap();
        let body = handle(&args, &cell).expect("ok");
        let resp: ListResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(resp.agent_ura, "easynet:///r/realm/user/alice");
        assert_eq!(resp.keys.len(), 3);
    }

    #[test]
    fn list_returns_empty_for_unknown_user() {
        let cell = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
        let args =
            serde_json::to_vec(&json!({"agent_ura": "easynet:///r/realm/user/missing"})).unwrap();
        let body = handle(&args, &cell).expect("ok");
        let resp: ListResponse = serde_json::from_slice(&body).unwrap();
        assert!(resp.keys.is_empty());
    }
}
