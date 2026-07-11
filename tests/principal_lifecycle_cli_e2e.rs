//! PrincipalLifecycle CLI facade E2E
//! =================================
//!
//! This test executes the `easynet` CLI binary against the real daemon UDS
//! fixture and daemon key-service. It proves the product-neutral CLI facades
//! lower to the daemon-owned PrincipalLifecycle aggregate without Backend
//! account state, HTTP session state or private-key material.

#![cfg(all(feature = "axon-pb", unix))]

mod seven_axes_fixture;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use easynet_cli::daemon::trust::anchor::RealmTrustAnchor;
use serde_json::Value;
use seven_axes_fixture::SevenAxesHome;
use std::process::Command;

const ADMIN: &str = "easynet:///r/cli/user/admin";
const BOB: &str = "easynet:///r/cli/user/bob";
const PRINCIPAL_DELETE: &str = "principal.lifecycle.delete";

#[test]
fn principal_cli_facades_run_through_real_daemon() {
    let home = SevenAxesHome::seed();
    let _daemon = home.start_daemon_without_hosted_projection();

    let admin = easynet_json([
        "principal",
        "bootstrap",
        "--principal-ura",
        ADMIN,
        "--create-idempotency-key",
        "cli-admin-create",
        "--bind-idempotency-key",
        "cli-admin-bind",
        "--json",
    ]);
    assert_eq!(admin["principal"]["principal_ura"], ADMIN);
    assert_eq!(admin["principal"]["state"], "active");
    let admin_binding_id = binding_id_at(&admin, 0);

    let enrollment = easynet_json([
        "principal",
        "issue-enrollment",
        "--issuer-ura",
        ADMIN,
        "--subject-principal-ura",
        BOB,
        "--proof-ref",
        &admin_binding_id,
        "--idempotency-key",
        "cli-bob-enrollment",
        "--expected-version",
        "2",
        "--json",
    ]);
    let enrollment_id = enrollment["principal"]["enrollments"][0]["enrollment_id"]
        .as_str()
        .expect("enrollment id")
        .to_string();

    let bob = easynet_json([
        "principal",
        "enroll",
        "--principal-ura",
        BOB,
        "--enrollment-id",
        &enrollment_id,
        "--create-idempotency-key",
        "cli-bob-create",
        "--bind-idempotency-key",
        "cli-bob-bind",
        "--json",
    ]);
    assert_eq!(bob["principal"]["principal_ura"], BOB);
    assert_eq!(bob["principal"]["state"], "active");
    assert_eq!(bob["principal"]["bindings"].as_array().unwrap().len(), 1);
    let bob_laptop_binding_id = binding_id_at(&bob, 0);

    let bob_phone = easynet_json([
        "principal",
        "add-key",
        "--principal-ura",
        BOB,
        "--proof-ref",
        &bob_laptop_binding_id,
        "--idempotency-key",
        "cli-bob-phone",
        "--expected-version",
        "2",
        "--json",
    ]);
    assert_eq!(active_binding_count(&bob_phone), 2);
    let bob_phone_binding_id = binding_id_at(&bob_phone, 1);

    let rotated_pubkey = pubkey(42);
    let bob_rotated = easynet_json([
        "principal",
        "rotate-key",
        "--principal-ura",
        BOB,
        "--binding-id",
        &bob_laptop_binding_id,
        "--proof-ref",
        &bob_phone_binding_id,
        "--replacement-public-key-b64",
        &rotated_pubkey,
        "--replacement-key-id",
        "cli-bob-laptop-rotated",
        "--idempotency-key",
        "cli-bob-rotate",
        "--expected-version",
        "3",
        "--json",
    ]);
    assert_eq!(active_binding_count(&bob_rotated), 2);
    let bob_rotated_laptop_binding_id = binding_id_at(&bob_rotated, 2);

    let bob_revoked_phone = easynet_json([
        "principal",
        "revoke-key",
        "--principal-ura",
        BOB,
        "--binding-id",
        &bob_phone_binding_id,
        "--proof-ref",
        &bob_rotated_laptop_binding_id,
        "--idempotency-key",
        "cli-bob-revoke-phone",
        "--expected-version",
        "4",
        "--json",
    ]);
    assert_eq!(active_binding_count(&bob_revoked_phone), 1);

    easynet_json([
        "principal",
        "configure-recovery",
        "--principal-ura",
        BOB,
        "--policy-ref",
        "recovery-policy:cli-bob",
        "--proof-ref",
        &bob_rotated_laptop_binding_id,
        "--idempotency-key",
        "cli-bob-recovery-policy",
        "--expected-version",
        "5",
        "--json",
    ]);

    let recovery_pubkey = pubkey(43);
    let bob_recovered = easynet_json([
        "principal",
        "recover",
        "--principal-ura",
        BOB,
        "--proof-ref",
        "recovery-policy:cli-bob",
        "--public-key-b64",
        &recovery_pubkey,
        "--key-id",
        "cli-bob-recovery-key",
        "--idempotency-key",
        "cli-bob-recover",
        "--expected-version",
        "6",
        "--json",
    ]);
    assert_eq!(active_binding_count(&bob_recovered), 2);

    let replay_recovery_pubkey = pubkey(44);
    let replay_recovery_error = easynet_failure([
        "principal",
        "recover",
        "--principal-ura",
        BOB,
        "--proof-ref",
        "recovery-policy:cli-bob",
        "--public-key-b64",
        &replay_recovery_pubkey,
        "--key-id",
        "cli-bob-recovery-replay-key",
        "--idempotency-key",
        "cli-bob-recover-replay",
        "--expected-version",
        "7",
        "--json",
    ]);
    assert!(
        replay_recovery_error.contains("already been consumed"),
        "CLI must surface daemon recovery replay denial, got: {replay_recovery_error}"
    );
    let bob_after_replay = easynet_json(["principal", "get", "--principal-ura", BOB, "--json"]);
    assert_eq!(active_binding_count(&bob_after_replay), 2);

    let bob_suspended = easynet_json([
        "principal",
        "suspend",
        "--principal-ura",
        BOB,
        "--proof-kind",
        "active-key",
        "--proof-ref",
        &bob_rotated_laptop_binding_id,
        "--idempotency-key",
        "cli-bob-suspend",
        "--expected-version",
        "7",
        "--json",
    ]);
    assert_eq!(bob_suspended["principal"]["state"], "suspended");

    let bob_reactivated = easynet_json([
        "principal",
        "reactivate",
        "--principal-ura",
        BOB,
        "--proof-kind",
        "recovery",
        "--proof-ref",
        "recovery-policy:cli-bob",
        "--idempotency-key",
        "cli-bob-reactivate",
        "--expected-version",
        "8",
        "--json",
    ]);
    assert_eq!(bob_reactivated["principal"]["state"], "active");

    easynet_json([
        "principal",
        "configure-recovery",
        "--principal-ura",
        BOB,
        "--policy-ref",
        "recovery-policy:cli-bob-deleted",
        "--proof-ref",
        &bob_rotated_laptop_binding_id,
        "--idempotency-key",
        "cli-bob-deleted-recovery-policy",
        "--expected-version",
        "9",
        "--json",
    ]);

    let grant = easynet_json([
        "principal",
        "issue-grant",
        "--principal-ura",
        ADMIN,
        "--action",
        PRINCIPAL_DELETE,
        "--proof-ref",
        &admin_binding_id,
        "--idempotency-key",
        "cli-admin-delete-grant",
        "--expected-version",
        "4",
        "--json",
    ]);
    let delete_grant_id = grant["principal"]["grants"][0]["grant_id"]
        .as_str()
        .expect("grant id")
        .to_string();

    let bob_deleted = easynet_json([
        "principal",
        "delete",
        "--principal-ura",
        BOB,
        "--actor-ura",
        ADMIN,
        "--proof-kind",
        "grant",
        "--proof-ref",
        &delete_grant_id,
        "--idempotency-key",
        "cli-bob-delete",
        "--expected-version",
        "10",
        "--json",
    ]);
    assert_eq!(bob_deleted["principal"]["state"], "deleted");

    let deleted_recovery_pubkey = pubkey(45);
    let deleted_recovery_error = easynet_failure([
        "principal",
        "recover",
        "--principal-ura",
        BOB,
        "--proof-ref",
        "recovery-policy:cli-bob-deleted",
        "--public-key-b64",
        &deleted_recovery_pubkey,
        "--key-id",
        "cli-bob-deleted-recovery-key",
        "--idempotency-key",
        "cli-bob-recover-deleted",
        "--expected-version",
        "11",
        "--json",
    ]);
    assert!(
        deleted_recovery_error.contains("principal must be active or suspended"),
        "CLI must surface daemon deleted-principal terminality, got: {deleted_recovery_error}"
    );

    let bob_snapshot = easynet_json(["principal", "get", "--principal-ura", BOB, "--json"]);
    assert_eq!(bob_snapshot["principal"]["state"], "deleted");
    assert!(bob_snapshot["principal"].get("username").is_none());
    assert!(bob_snapshot["principal"].get("user_id").is_none());

    let trust = RealmTrustAnchor::load_or_empty(&home.trust_path).expect("load trust");
    let bob_public_key = bob["principal"]["bindings"][0]["public_key_b64"]
        .as_str()
        .expect("bob public key");
    assert!(
        trust.lookup_user_by_pubkey(BOB, bob_public_key).is_none(),
        "rotated CLI-enrolled key must no longer be active in RuntimeTrust"
    );
    assert!(
        trust.lookup_user_by_pubkey(BOB, &rotated_pubkey).is_some(),
        "rotated public key must remain active in RuntimeTrust"
    );
    assert!(
        trust.lookup_user_by_pubkey(BOB, &recovery_pubkey).is_some(),
        "recovery public key must be projected into RuntimeTrust"
    );
    assert!(
        trust
            .lookup_user_by_pubkey(BOB, &replay_recovery_pubkey)
            .is_none(),
        "replayed recovery key must not be projected into RuntimeTrust"
    );
    assert!(
        trust
            .lookup_user_by_pubkey(BOB, &deleted_recovery_pubkey)
            .is_none(),
        "deleted-principal recovery key must not be projected into RuntimeTrust"
    );
}

fn easynet_json<const N: usize>(args: [&str; N]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_easynet"))
        .args(args)
        .output()
        .expect("run easynet");
    assert!(
        output.status.success(),
        "easynet failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "parse easynet JSON: {error}\nstdout:\n{}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn easynet_failure<const N: usize>(args: [&str; N]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_easynet"))
        .args(args)
        .output()
        .expect("run easynet");
    assert!(
        !output.status.success(),
        "easynet unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn binding_id_at(snapshot: &Value, index: usize) -> String {
    snapshot["principal"]["bindings"][index]["binding_id"]
        .as_str()
        .expect("binding id")
        .to_string()
}

fn active_binding_count(snapshot: &Value) -> usize {
    snapshot["principal"]["bindings"]
        .as_array()
        .expect("bindings")
        .iter()
        .filter(|binding| binding["state"] == "active")
        .count()
}

fn pubkey(seed: u8) -> String {
    let signing = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
    B64.encode(signing.verifying_key().to_bytes())
}
