// EasyNet CLI — PR-N4 commit 5/N: cross-realm user binding e2e
// =================================================================
//
// File: tests/cross_realm_user_binding_e2e.rs
// Description: In-process integration test that drives the full
//              PR-N4 cross-realm user identity binding flow:
//              realm A's daemon issues a `UserBindingToken` via
//              `device.keyring.federate_user_identity_token`;
//              realm B's daemon consumes it via
//              `device.keyring.consume_federate_user_token`;
//              realm B's `FederatedUserResolver` then surfaces
//              the bound user identity.
//
// What this validates from PR-N4 spec §acceptance-gates:
// - e2e federate → consume round-trip with the token bytes
//   crossing the realm boundary verbatim.
// - Verify chain four checks all run in evaluation order
//   (target_realm, freshness, signature, replay).
// - FederatedUserResolver surfaces `BoundLocalUser` after
//   the consume succeeds.
//
// What this does NOT exercise (deferred):
// - Real TCP/TLS spawn (in-process keyrings + ability
//   registration; no daemon binary needed for the binding
//   path which is independent of the cross-hub transport).
// - Backend HTTP / JWT custom-claim transport (test passes
//   the JSON token directly between abilities; transport is
//   the spec §commit 2/N "transport_hint" detail, not part of
//   the binding semantics).
// - Full PR-N3 directory filter integration on top of the
//   resolver — that lands when commit N3-N4-bridge ships
//   (FederatedUserResolver consulted from
//   federation_directory's flatten/lookup paths).
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::Arc;

use serde_json::{json, Value};

use easynet_cli::daemon::keyring::abilities::{
    handle_consume_federate_user_token, handle_create, handle_federate_user_identity_token,
    ManagedSigningProvider,
};
use easynet_cli::daemon::keyring::federated_bindings::FederatedBindingsStore;
use easynet_cli::daemon::keyring::resolver::{FederatedUserOutcome, FederatedUserResolver};
use easynet_cli::daemon::keyring::{
    ManagedPeer, ManagedSigningKeyProjection, ManagedSigningStatus, MasterKeySource, Vault,
};

struct TestProvider(std::sync::Mutex<Vault>);

impl ManagedSigningProvider for TestProvider {
    fn create(
        &self,
        purpose: String,
        bound_subject: Option<String>,
    ) -> anyhow::Result<ManagedSigningKeyProjection> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .inventory_create(purpose, bound_subject)?)
    }
    fn list(
        &self,
        purpose: Option<String>,
        status: Option<ManagedSigningStatus>,
    ) -> anyhow::Result<Vec<ManagedSigningKeyProjection>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .inventory_list(purpose.as_deref(), status))
    }
    fn public_key(&self, key_id: &str) -> anyhow::Result<ManagedSigningKeyProjection> {
        Ok(self.0.lock().unwrap().inventory_public_key(key_id)?)
    }
    fn sign(
        &self,
        key_id: &str,
        canonical_bytes: &[u8],
    ) -> anyhow::Result<ed25519_dalek::Signature> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .inventory_sign(key_id, canonical_bytes)?)
    }
    fn rotate(&self, key_id: &str) -> anyhow::Result<ManagedSigningKeyProjection> {
        Ok(self.0.lock().unwrap().inventory_rotate(key_id)?)
    }
    fn revoke(&self, key_id: &str) -> anyhow::Result<i64> {
        Ok(self.0.lock().unwrap().inventory_revoke(key_id)?)
    }
    fn set_expiry(&self, key_id: &str, expires_unix_ms: i64) -> anyhow::Result<()> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .inventory_set_expiry(key_id, expires_unix_ms)?)
    }
    fn bind_subject(&self, key_id: &str, subject_ura: &str) -> anyhow::Result<()> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .inventory_bind_subject(key_id, subject_ura.to_string())?)
    }
    fn peer_add(
        &self,
        peer_ura: &str,
        public_key_b64: &str,
        via_hub: Option<String>,
    ) -> anyhow::Result<bool> {
        Ok(self.0.lock().unwrap().inventory_peer_add(
            peer_ura.to_string(),
            public_key_b64.to_string(),
            via_hub,
        )?)
    }
    fn peer_list(&self) -> anyhow::Result<Vec<ManagedPeer>> {
        Ok(self.0.lock().unwrap().inventory_peer_list())
    }
}

/// Stand up realm A's daemon: keyring + agent_signing entry +
/// bound user subject. Returns the provider plus the user URA
/// so the test driver can pass it to the consumer side later.
fn boot_realm_a_daemon() -> (
    Arc<dyn ManagedSigningProvider>,
    String,
    String,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open_or_init(
        &dir.path().join("realm-a-key-service.enc"),
        &MasterKeySource::Explicit("passphrase-a".into()),
    )
    .unwrap();
    let h: Arc<dyn ManagedSigningProvider> = Arc::new(TestProvider(std::sync::Mutex::new(vault)));
    let user_ura = "easynet:///r/realm-a/user/user-c".to_string();
    let created = handle_create(
        &h,
        json!({
            "purpose": "agent_signing",
            "bound_subject": user_ura,
        }),
    )
    .unwrap();
    (
        h,
        user_ura,
        created["key_id"].as_str().unwrap().to_string(),
        dir,
    )
}

fn issuer_args(source_user_ura: &str, key_id: &str, target_realm: &str, issued_at: u64) -> Value {
    json!({
        "source_user_ura": source_user_ura,
        "managed_key_id": key_id,
        "target_realm": target_realm,
        "issued_at_unix_ms": issued_at,
    })
}

#[test]
fn full_round_trip_realm_a_issues_realm_b_consumes_resolver_finds() {
    // ── Realm A: issue token ──
    let (a_keyring, source_user_ura, key_id, _a_dir) = boot_realm_a_daemon();
    let issued_at_ms: u64 = 1_714_500_000_000;

    let token_resp = handle_federate_user_identity_token(
        &a_keyring,
        issuer_args(&source_user_ura, &key_id, "realm-b", issued_at_ms),
    )
    .expect("realm A issues token");
    assert_eq!(token_resp["transport_hint"], json!("jwt-custom-claim"));
    let token_json = token_resp["token"].clone();
    assert_eq!(token_json["source_user_ura"], json!(source_user_ura));
    assert_eq!(token_json["target_realm"], json!("realm-b"));

    // ── Token bytes cross the realm boundary (in-process: just
    // round-trip through serde so we exercise the wire shape).
    let serialised: Value =
        serde_json::from_slice(&serde_json::to_vec(&token_json).unwrap()).unwrap();

    // ── Realm B: consume token ──
    let bindings = Arc::new(FederatedBindingsStore::in_memory());
    let local_user_id_on_b = "user-c-on-realm-b".to_string();
    let consume_resp = handle_consume_federate_user_token(
        &bindings,
        json!({
            "token": serialised,
            "self_realm": "realm-b",
            "local_user_id": local_user_id_on_b,
            "now_unix_ms": issued_at_ms + 1_000,
        }),
    )
    .expect("realm B consumes token");
    assert_eq!(consume_resp["binding_recorded"], json!(true));
    assert_eq!(consume_resp["source_realm"], json!("realm-a"));
    assert_eq!(consume_resp["local_user_id"], json!(local_user_id_on_b));

    // ── Resolver: realm B can now look up the bound user ──
    let resolver = FederatedUserResolver::new("realm-b", bindings.clone());
    let outcome = resolver.resolve_user(&source_user_ura);
    assert_eq!(
        outcome,
        FederatedUserOutcome::BoundLocalUser(local_user_id_on_b.clone())
    );

    // Cross-check: a different user URA in realm A is NOT bound.
    let unbound_outcome = resolver.resolve_user("easynet:///r/realm-a/user/user-other");
    assert_eq!(unbound_outcome, FederatedUserOutcome::NotBound);

    // Realm B's own URA is `Local` (no federated lookup
    // needed — INV-3).
    let local_outcome = resolver.resolve_user("easynet:///r/realm-b/user/user-on-b");
    assert_eq!(local_outcome, FederatedUserOutcome::Local);
}

#[test]
fn replay_attempt_after_successful_consume_rejected() {
    // Same flow as above, but try to consume the SAME token
    // twice. Second attempt must reject with replay detected.
    let (a_keyring, source_user_ura, key_id, _a_dir) = boot_realm_a_daemon();
    let issued_at_ms: u64 = 1_714_500_000_000;
    let token_resp = handle_federate_user_identity_token(
        &a_keyring,
        issuer_args(&source_user_ura, &key_id, "realm-b", issued_at_ms),
    )
    .unwrap();
    let token = token_resp["token"].clone();

    let bindings = Arc::new(FederatedBindingsStore::in_memory());
    let consume_args = json!({
        "token": token,
        "self_realm": "realm-b",
        "local_user_id": "user-c-on-realm-b",
        "now_unix_ms": issued_at_ms + 1_000,
    });
    handle_consume_federate_user_token(&bindings, consume_args.clone())
        .expect("first consume succeeds");
    let err = handle_consume_federate_user_token(&bindings, consume_args)
        .expect_err("second consume must reject with replay");
    assert!(
        err.to_string().contains("replay detected"),
        "expected replay rejection, got: {err}"
    );
}

#[test]
fn token_for_wrong_realm_rejected_at_target_check() {
    // Realm A issues a token targeting realm B; realm C tries
    // to consume — must reject at target_realm check before
    // any expensive crypto runs.
    let (a_keyring, source_user_ura, key_id, _a_dir) = boot_realm_a_daemon();
    let token_resp = handle_federate_user_identity_token(
        &a_keyring,
        issuer_args(&source_user_ura, &key_id, "realm-b", 1_714_500_000_000),
    )
    .unwrap();

    let bindings = Arc::new(FederatedBindingsStore::in_memory());
    let err = handle_consume_federate_user_token(
        &bindings,
        json!({
            "token": token_resp["token"],
            "self_realm": "realm-c", // = NOT what the token targets
            "local_user_id": "user-on-c",
            "now_unix_ms": 1_714_500_000_001_u64,
        }),
    )
    .expect_err("consumed at wrong realm must reject");
    assert!(
        err.to_string().contains("wrong target_realm"),
        "rejection must be the target-realm check, got: {err}"
    );
}

#[test]
fn binding_persists_across_resolver_construction() {
    // Once written to the store, the binding survives
    // constructing a fresh resolver — readers don't need to be
    // alive at consume time.
    let (a_keyring, source_user_ura, key_id, _a_dir) = boot_realm_a_daemon();
    let issued_at_ms: u64 = 1_714_500_000_000;
    let token = handle_federate_user_identity_token(
        &a_keyring,
        issuer_args(&source_user_ura, &key_id, "realm-b", issued_at_ms),
    )
    .unwrap()["token"]
        .clone();

    let bindings = Arc::new(FederatedBindingsStore::in_memory());
    handle_consume_federate_user_token(
        &bindings,
        json!({
            "token": token,
            "self_realm": "realm-b",
            "local_user_id": "u",
            "now_unix_ms": issued_at_ms + 1,
        }),
    )
    .unwrap();

    // Construct the resolver AFTER the consume.
    let resolver = FederatedUserResolver::new("realm-b", bindings);
    let outcome = resolver.resolve_user(&source_user_ura);
    assert!(matches!(outcome, FederatedUserOutcome::BoundLocalUser(_)));
}
