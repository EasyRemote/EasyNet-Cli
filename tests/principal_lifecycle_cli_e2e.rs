//! PrincipalLifecycle CLI facade E2E
//! =================================
//!
//! This test executes the `easynet` CLI binary against the real daemon UDS
//! fixture and daemon key-service. It proves the product-neutral CLI facades
//! lower to the daemon-owned PrincipalLifecycle aggregate without Backend
//! account state, HTTP session state or private-key material.

#![cfg(all(feature = "axon-pb", unix))]

mod seven_axes_fixture;

use easynet_cli::daemon::trust::anchor::RealmTrustAnchor;
use serde_json::Value;
use seven_axes_fixture::SevenAxesHome;
use std::process::Command;

const ADMIN: &str = "easynet:///r/cli/user/admin";
const BOB: &str = "easynet:///r/cli/user/bob";

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

    let bob_snapshot = easynet_json(["principal", "get", "--principal-ura", BOB, "--json"]);
    assert_eq!(bob_snapshot["principal"]["state"], "active");
    assert!(bob_snapshot["principal"].get("username").is_none());
    assert!(bob_snapshot["principal"].get("user_id").is_none());

    let trust = RealmTrustAnchor::load_or_empty(&home.trust_path).expect("load trust");
    let bob_public_key = bob["principal"]["bindings"][0]["public_key_b64"]
        .as_str()
        .expect("bob public key");
    assert!(
        trust.lookup_user_by_pubkey(BOB, bob_public_key).is_some(),
        "CLI-enrolled principal public key must be projected into RuntimeTrust"
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

fn binding_id_at(snapshot: &Value, index: usize) -> String {
    snapshot["principal"]["bindings"][index]["binding_id"]
        .as_str()
        .expect("binding id")
        .to_string()
}
