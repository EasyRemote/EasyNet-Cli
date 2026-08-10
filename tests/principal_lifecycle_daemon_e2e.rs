//! PrincipalLifecycle daemon-surface E2E
//! =====================================
//!
//! This test drives `principal.lifecycle.*` through the real daemon gRPC
//! descriptor-ref Invocation surface used by the existing seven-axes fixture.
//! It intentionally does not call the provider directly and does not introduce
//! Backend account, HTTP session or EasyRemote workflow state.

#![cfg(all(feature = "axon-pb", unix))]

mod seven_axes_fixture;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use easynet_cli::daemon::trust::anchor::RealmTrustAnchor;
use serde_json::{json, Value};
use seven_axes_fixture::SevenAxesHome;

const ADMIN: &str = "easynet:///r/cli/user/admin";
const BOB: &str = "easynet:///r/cli/user/bob";
const PRINCIPAL_CREATE: &str = "principal.lifecycle.create";
const PRINCIPAL_BIND_FIRST_KEY: &str = "principal.lifecycle.bind_first_key";
const PRINCIPAL_ADD_KEY: &str = "principal.lifecycle.add_key";
const PRINCIPAL_ROTATE_KEY: &str = "principal.lifecycle.rotate_key";
const PRINCIPAL_REVOKE_KEY: &str = "principal.lifecycle.revoke_key";
const PRINCIPAL_CONFIGURE_RECOVERY: &str = "principal.lifecycle.configure_recovery";
const PRINCIPAL_RECOVER: &str = "principal.lifecycle.recover";
const PRINCIPAL_DELETE: &str = "principal.lifecycle.delete";
const PRINCIPAL_ISSUE_ENROLLMENT: &str = "principal.lifecycle.issue_enrollment";
const PRINCIPAL_ISSUE_GRANT: &str = "principal.lifecycle.issue_grant";
const PRINCIPAL_GET: &str = "principal.lifecycle.get";

#[test]
fn principal_lifecycle_runs_through_real_daemon_and_survives_restart() {
    let home = SevenAxesHome::seed();
    let daemon = home.start_daemon_without_hosted_projection();

    home.invoke_device_hosted_system_ability(
        PRINCIPAL_CREATE,
        request(json!({
            "command": command("admin-create", "bootstrap", "proof:admin-create", None, ADMIN),
            "principal_ura": ADMIN
        })),
    );
    let admin_bound = home.invoke_device_hosted_system_ability(
        PRINCIPAL_BIND_FIRST_KEY,
        request(json!({
            "command": command("admin-bind", "bootstrap", "proof:admin-create", Some(1), ADMIN),
            "principal_ura": ADMIN,
            "public_key_b64": pubkey(20)
        })),
    );
    let admin_binding_id = binding_id_at(&admin_bound, 0);
    home.invoke_device_hosted_system_ability(
        PRINCIPAL_ADD_KEY,
        request(json!({
            "command": command("admin-backup", "active_key", &admin_binding_id, Some(2), ADMIN),
            "principal_ura": ADMIN,
            "public_key_b64": pubkey(25)
        })),
    );
    let joined_admin_device = easynet_cli::core::ura::device_ura("cli", "joined-admin-phone");
    let join_receipt = home.invoke_federation_join_with_principal_proof(
        &joined_admin_device,
        ADMIN,
        "active_key",
        &admin_binding_id,
    );
    assert_eq!(join_receipt["membership_ura"], joined_admin_device);
    assert_eq!(join_receipt["realm"], "cli");

    let enrollment = home.invoke_device_hosted_system_ability(
        PRINCIPAL_ISSUE_ENROLLMENT,
        request(json!({
            "command": command("bob-enroll", "active_key", &admin_binding_id, Some(3), ADMIN),
            "principal_ura": ADMIN,
            "subject_principal_ura": BOB
        })),
    );
    let enrollment_id = enrollment["principal"]["enrollments"][0]["enrollment_id"]
        .as_str()
        .expect("enrollment id")
        .to_string();

    home.invoke_device_hosted_system_ability(
        PRINCIPAL_CREATE,
        request(json!({
            "command": command("bob-create", "enrollment", &enrollment_id, None, BOB),
            "principal_ura": BOB
        })),
    );
    let bob_bound = home.invoke_device_hosted_system_ability(
        PRINCIPAL_BIND_FIRST_KEY,
        request(json!({
            "command": command("bob-laptop", "enrollment", &enrollment_id, Some(1), BOB),
            "principal_ura": BOB,
            "public_key_b64": pubkey(21)
        })),
    );
    let bob_laptop_binding_id = binding_id_at(&bob_bound, 0);

    let bob_with_phone = home.invoke_device_hosted_system_ability(
        PRINCIPAL_ADD_KEY,
        request(json!({
            "command": command("bob-phone", "active_key", &bob_laptop_binding_id, Some(2), BOB),
            "principal_ura": BOB,
            "public_key_b64": pubkey(23)
        })),
    );
    let bob_phone_binding_id = binding_id_at(&bob_with_phone, 1);

    let bob_rotated = home.invoke_device_hosted_system_ability(
        PRINCIPAL_ROTATE_KEY,
        request(json!({
            "command": command("bob-rotate", "active_key", &bob_phone_binding_id, Some(3), BOB),
            "principal_ura": BOB,
            "binding_id": bob_laptop_binding_id,
            "replacement": {
                "command": command("ignored-replacement", "active_key", &bob_phone_binding_id, None, BOB),
                "principal_ura": BOB,
                "public_key_b64": pubkey(22)
            }
        })),
    );
    let bob_rotated_laptop_binding_id = binding_id_at(&bob_rotated, 2);

    home.invoke_device_hosted_system_ability(
        PRINCIPAL_REVOKE_KEY,
        request(json!({
            "command": command("bob-revoke-phone", "active_key", &bob_rotated_laptop_binding_id, Some(4), BOB),
            "principal_ura": BOB,
            "binding_id": bob_phone_binding_id
        })),
    );
    home.invoke_device_hosted_system_ability(
        PRINCIPAL_CONFIGURE_RECOVERY,
        request(json!({
            "command": command("bob-recovery-policy", "active_key", &bob_rotated_laptop_binding_id, Some(5), BOB),
            "principal_ura": BOB,
            "policy_ref": "recovery-policy:bob"
        })),
    );
    home.invoke_device_hosted_system_ability(
        PRINCIPAL_RECOVER,
        request(json!({
            "command": command("bob-recover", "recovery", "recovery-policy:bob", Some(6), BOB),
            "principal_ura": BOB,
            "replacement_key": {
                "command": command("ignored-recovery-child", "recovery", "recovery-policy:bob", None, BOB),
                "principal_ura": BOB,
                "public_key_b64": pubkey(24)
            }
        })),
    );

    let grant = home.invoke_device_hosted_system_ability(
        PRINCIPAL_ISSUE_GRANT,
        request(json!({
            "command": command("admin-delete-grant", "active_key", &admin_binding_id, Some(5), ADMIN),
            "principal_ura": ADMIN,
            "actions": [PRINCIPAL_DELETE]
        })),
    );
    let delete_grant_id = grant["principal"]["grants"][0]["grant_id"]
        .as_str()
        .expect("grant id")
        .to_string();

    let deleted = home.invoke_device_hosted_system_ability(
        PRINCIPAL_DELETE,
        request(json!({
            "command": command("bob-delete", "grant", &delete_grant_id, Some(7), ADMIN),
            "principal_ura": BOB
        })),
    );
    assert_eq!(deleted["principal"]["state"], "deleted");
    drop(daemon);

    let restarted = home.start_daemon_without_hosted_projection();
    let after_restart = home.invoke_device_hosted_system_ability(
        PRINCIPAL_GET,
        json!({
            "principal_ura": BOB
        }),
    );
    assert_eq!(after_restart["principal"]["state"], "deleted");
    assert_eq!(
        after_restart["principal"]["bindings"]
            .as_array()
            .expect("bindings")
            .iter()
            .filter(|binding| binding["state"] == "active")
            .count(),
        2
    );
    drop(restarted);

    let persisted_trust = RealmTrustAnchor::try_load_strict(&home.trust_path).expect("load trust");
    assert!(persisted_trust
        .lookup_user_by_pubkey(BOB, &pubkey(21))
        .is_none());
    assert!(persisted_trust
        .lookup_user_by_pubkey(BOB, &pubkey(23))
        .is_none());
    assert!(persisted_trust
        .lookup_user_by_pubkey(BOB, &pubkey(22))
        .is_some());
    assert!(persisted_trust
        .lookup_user_by_pubkey(BOB, &pubkey(24))
        .is_some());
    let joined_owner = persisted_trust
        .lookup_principal_owner(&joined_admin_device)
        .expect("joined device has persisted principal owner binding");
    assert_eq!(joined_owner.owner_ura, ADMIN);
    assert_eq!(joined_owner.owner_user_id, "admin");
}

fn request(request: Value) -> Value {
    json!({ "request": request })
}

fn command(
    idempotency_key: &str,
    proof_kind: &str,
    proof_ref: &str,
    expected_version: Option<u64>,
    actor_ura: &str,
) -> Value {
    let mut value = json!({
        "actor_ura": actor_ura,
        "idempotency_key": idempotency_key,
        "proof": {
            "kind": proof_kind,
            "reference": proof_ref
        }
    });
    if let Some(expected) = expected_version {
        value["expected_version"] = json!(expected);
    }
    value
}

fn pubkey(seed: u8) -> String {
    let signing = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
    B64.encode(signing.verifying_key().to_bytes())
}

fn binding_id_at(snapshot: &Value, index: usize) -> String {
    snapshot["principal"]["bindings"][index]["binding_id"]
        .as_str()
        .expect("binding id")
        .to_string()
}
