// EasyNet CLI — PR-N4 commit 5/N: cross-realm user binding e2e
// =================================================================
//
// File: tests/cross_realm_user_binding_e2e.rs
// Description: In-process integration test that drives the full
//              PR-N4 cross-realm user identity binding flow:
//              realm A's daemon issues a `UserBindingToken` via
//              `<self>.keyring.federate_user_identity_token`;
//              realm B's daemon consumes it via
//              `<self>.keyring.consume_federate_user_token`;
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

use easynet_cli::runtime::keyring::abilities::{
    handle_consume_federate_user_token, handle_create, handle_federate_user_identity_token,
};
use easynet_cli::runtime::keyring::federated_bindings::FederatedBindingsStore;
use easynet_cli::runtime::keyring::handle::KeyringHandle;
use easynet_cli::runtime::keyring::resolver::{
    FederatedUserOutcome, FederatedUserResolver,
};

/// Stand up realm A's daemon: keyring + agent_signing entry +
/// bound device subject. Returns the handle plus the user URI
/// so the test driver can pass it to the consumer side later.
fn boot_realm_a_daemon() -> (Arc<KeyringHandle>, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("realm_a_keyring.json");
    let h = Arc::new(KeyringHandle::open_or_create(path, "passphrase-a").unwrap());
    let user_uri = "easynet:///r/realm-a/agent/user-c".to_string();
    h.set_device_subject(user_uri.clone()).unwrap();
    handle_create(&h, json!({"purpose": "agent_signing"})).unwrap();
    (h, user_uri, dir)
}

#[test]
fn full_round_trip_realm_a_issues_realm_b_consumes_resolver_finds() {
    // ── Realm A: issue token ──
    let (a_keyring, source_user_uri, _a_dir) = boot_realm_a_daemon();
    let issued_at_ms: u64 = 1_714_500_000_000;

    let token_resp = handle_federate_user_identity_token(
        &a_keyring,
        json!({
            "target_realm": "realm-b",
            "issued_at_unix_ms": issued_at_ms,
        }),
    )
    .expect("realm A issues token");
    assert_eq!(token_resp["transport_hint"], json!("jwt-custom-claim"));
    let token_json = token_resp["token"].clone();
    assert_eq!(token_json["source_user_uri"], json!(source_user_uri));
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
    let outcome = resolver.resolve_user(&source_user_uri);
    assert_eq!(
        outcome,
        FederatedUserOutcome::BoundLocalUser(local_user_id_on_b.clone())
    );

    // Cross-check: a different user URI in realm A is NOT bound.
    let unbound_outcome = resolver.resolve_user("easynet:///r/realm-a/agent/user-other");
    assert_eq!(unbound_outcome, FederatedUserOutcome::NotBound);

    // Realm B's own URI is `Local` (no federated lookup
    // needed — INV-3).
    let local_outcome = resolver.resolve_user("easynet:///r/realm-b/agent/user-on-b");
    assert_eq!(local_outcome, FederatedUserOutcome::Local);
}

#[test]
fn replay_attempt_after_successful_consume_rejected() {
    // Same flow as above, but try to consume the SAME token
    // twice. Second attempt must reject with replay detected.
    let (a_keyring, _source_user_uri, _a_dir) = boot_realm_a_daemon();
    let issued_at_ms: u64 = 1_714_500_000_000;
    let token_resp = handle_federate_user_identity_token(
        &a_keyring,
        json!({
            "target_realm": "realm-b",
            "issued_at_unix_ms": issued_at_ms,
        }),
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
    let (a_keyring, _source_user_uri, _a_dir) = boot_realm_a_daemon();
    let token_resp = handle_federate_user_identity_token(
        &a_keyring,
        json!({
            "target_realm": "realm-b",
            "issued_at_unix_ms": 1_714_500_000_000_u64,
        }),
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
    let (a_keyring, source_user_uri, _a_dir) = boot_realm_a_daemon();
    let issued_at_ms: u64 = 1_714_500_000_000;
    let token = handle_federate_user_identity_token(
        &a_keyring,
        json!({
            "target_realm": "realm-b",
            "issued_at_unix_ms": issued_at_ms,
        }),
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
    let outcome = resolver.resolve_user(&source_user_uri);
    assert!(matches!(outcome, FederatedUserOutcome::BoundLocalUser(_)));
}
