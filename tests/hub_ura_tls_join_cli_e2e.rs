//! Hub URA TLS join CLI E2E
//! ========================
//!
//! This test starts a real hub-mode `easynet-daemon` process with a private
//! TCP+TLS Invocation listener, bootstraps PrincipalLifecycle enrollment on the
//! Hub, then joins it from the `easynet` CLI binary by Hub URA. It proves the
//! backend-free path uses daemon-owned `federation.join`, Principal enrollment
//! proof admission and in-band `federation.resolve_key` rather than the staged
//! HTTP pairing facade.

#![cfg(all(feature = "axon-pb", unix))]

use std::fs::File;
use std::io::Read as _;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use easynet_cli::daemon::persistence::config::Credentials;
use easynet_cli::daemon::trust::anchor::{RealmTrustAnchor, TrustAnchorRole};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use serde_json::Value;

const REALM: &str = "localhost";
const HUB_URA: &str = "easynet:///r/localhost/authority";
const ADMIN_URA: &str = "easynet:///r/localhost/user/admin";
const ALICE_URA: &str = "easynet:///r/localhost/user/alice";
const BOB_URA: &str = "easynet:///r/localhost/user/bob";
const PRINCIPAL_DELETE: &str = "principal.lifecycle.delete";

#[test]
fn principal_bound_device_join_hub_ura_uses_real_tcp_tls_daemon_without_backend() {
    let hub_home = tempfile::tempdir().expect("hub HOME");
    let device_home = tempfile::tempdir().expect("device HOME");
    let port = free_local_port();
    let certs = write_test_ca_and_leaf(hub_home.path());
    write_hub_daemon_config(hub_home.path(), port, &certs.cert_pem, &certs.key_pem);

    let hub = HubDaemon::spawn(hub_home.path(), port);

    let admin = easynet_json(
        hub_home.path(),
        [
            "principal",
            "bootstrap",
            "--principal-ura",
            ADMIN_URA,
            "--proof-ref",
            "bootstrap-tls-admin",
            "--create-idempotency-key",
            "tls-admin-create",
            "--bind-idempotency-key",
            "tls-admin-bind",
            "--json",
        ],
        &hub,
    );
    assert_eq!(admin["principal"]["principal_ura"], ADMIN_URA);
    assert_eq!(admin["principal"]["state"], "active");
    let admin_binding_id = binding_id_at(&admin, 0);

    let enrollment = easynet_json(
        hub_home.path(),
        [
            "principal",
            "issue-enrollment",
            "--issuer-ura",
            ADMIN_URA,
            "--subject-principal-ura",
            ALICE_URA,
            "--proof-ref",
            &admin_binding_id,
            "--idempotency-key",
            "tls-alice-device-enrollment",
            "--expected-version",
            "2",
            "--json",
        ],
        &hub,
    );
    let enrollment_id = enrollment["principal"]["enrollments"][0]["enrollment_id"]
        .as_str()
        .expect("enrollment id")
        .to_string();

    run_easynet(
        device_home.path(),
        [
            "device",
            "join",
            HUB_URA,
            "--principal-ura",
            ALICE_URA,
            "--principal-enrollment-id",
            &enrollment_id,
            "--hub-ca",
            certs.ca_pem.to_str().expect("utf8 ca path"),
            "--hub-port",
            &port.to_string(),
            "--boot",
            "no",
            "--yes",
        ],
        &hub,
    );

    let credentials_path = device_home.path().join(".easynet/credentials.json");
    let credentials: Credentials =
        serde_json::from_slice(&std::fs::read(&credentials_path).expect("read device credentials"))
            .expect("decode credentials");
    assert_eq!(credentials.realm, REALM);
    assert_eq!(credentials.credential_token, "");
    assert_eq!(credentials.hub_api_base, None);
    assert!(
        credentials
            .join_receipt_hash
            .as_deref()
            .is_some_and(|hash| !hash.trim().is_empty()),
        "Hub URA join must persist membership lineage"
    );
    assert!(
        credentials
            .hub_pubkey_b64
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty()),
        "Hub public key must be imported through federation.resolve_key"
    );
    assert!(
        credentials
            .hub_tls_ca_pem_b64
            .as_deref()
            .is_some_and(|pem| !pem.trim().is_empty()),
        "Hub CA must be persisted for device-mode session bootstrap"
    );

    let trust_path = device_home.path().join(".easynet/realm-trust.toml");
    let trust = RealmTrustAnchor::try_load_strict(&trust_path).expect("load device trust anchor");
    let hub_row = trust
        .lookup(HUB_URA)
        .expect("device trust anchor must include Hub URA row");
    assert_eq!(hub_row.role, TrustAnchorRole::Hub);
    assert_eq!(hub_row.origin_realm.as_deref(), Some(REALM));
    assert!(
        hub_row.tls_ca_pem_path.as_deref().is_some(),
        "Hub trust row must point at the persisted pinned CA"
    );

    let device_ura = easynet_cli::core::ura::device_ura(REALM, &credentials.node_id);
    let device_row = trust
        .lookup(&device_ura)
        .expect("device trust anchor must include joined Device URA row");
    assert_eq!(device_row.role, TrustAnchorRole::Device);

    let hub_trust_path = hub_home.path().join(".easynet/realm-trust.toml");
    let hub_trust =
        RealmTrustAnchor::try_load_strict(&hub_trust_path).expect("load hub trust anchor");
    let owner = hub_trust
        .lookup_principal_owner(&device_ura)
        .expect("Hub RuntimeTrust must bind joined Device URA to Principal URA");
    assert_eq!(owner.principal_ura, device_ura);
    assert_eq!(owner.owner_ura, ALICE_URA);
    assert_eq!(owner.owner_user_id, "alice");

    let alice = easynet_json(
        hub_home.path(),
        [
            "principal",
            "enroll",
            "--principal-ura",
            ALICE_URA,
            "--enrollment-id",
            &enrollment_id,
            "--create-idempotency-key",
            "tls-alice-create",
            "--bind-idempotency-key",
            "tls-alice-bind",
            "--json",
        ],
        &hub,
    );
    assert_eq!(alice["principal"]["principal_ura"], ALICE_URA);
    assert_eq!(alice["principal"]["state"], "active");
    assert_eq!(active_binding_count(&alice), 1);
    assert!(alice["principal"].get("username").is_none());
    assert!(alice["principal"].get("user_id").is_none());
    let alice_initial_binding_id = binding_id_at(&alice, 0);
    let alice_initial_pubkey = public_key_at(&alice, 0);

    let bob_enrollment = easynet_json(
        hub_home.path(),
        [
            "principal",
            "issue-enrollment",
            "--issuer-ura",
            ADMIN_URA,
            "--subject-principal-ura",
            BOB_URA,
            "--proof-ref",
            &admin_binding_id,
            "--idempotency-key",
            "tls-bob-enrollment",
            "--expected-version",
            "4",
            "--json",
        ],
        &hub,
    );
    let bob_enrollment_id = newest_enrollment_id(&bob_enrollment);

    let bob = easynet_json(
        hub_home.path(),
        [
            "principal",
            "enroll",
            "--principal-ura",
            BOB_URA,
            "--enrollment-id",
            &bob_enrollment_id,
            "--create-idempotency-key",
            "tls-bob-create",
            "--bind-idempotency-key",
            "tls-bob-bind",
            "--json",
        ],
        &hub,
    );
    assert_eq!(bob["principal"]["principal_ura"], BOB_URA);
    assert_eq!(bob["principal"]["state"], "active");
    assert_eq!(active_binding_count(&bob), 1);
    let bob_initial_binding_id = binding_id_at(&bob, 0);

    let alice_phone = easynet_json(
        hub_home.path(),
        [
            "principal",
            "add-key",
            "--principal-ura",
            ALICE_URA,
            "--proof-ref",
            &alice_initial_binding_id,
            "--idempotency-key",
            "tls-alice-phone",
            "--expected-version",
            "2",
            "--json",
        ],
        &hub,
    );
    assert_eq!(active_binding_count(&alice_phone), 2);
    let alice_phone_binding_id = binding_id_at(&alice_phone, 1);
    let alice_phone_pubkey = public_key_at(&alice_phone, 1);

    let alice_rotated_pubkey = pubkey(42);
    let alice_rotated = easynet_json(
        hub_home.path(),
        [
            "principal",
            "rotate-key",
            "--principal-ura",
            ALICE_URA,
            "--binding-id",
            &alice_initial_binding_id,
            "--proof-ref",
            &alice_phone_binding_id,
            "--replacement-public-key-b64",
            &alice_rotated_pubkey,
            "--replacement-key-id",
            "tls-alice-laptop-rotated",
            "--idempotency-key",
            "tls-alice-rotate",
            "--expected-version",
            "3",
            "--json",
        ],
        &hub,
    );
    assert_eq!(active_binding_count(&alice_rotated), 2);
    let alice_rotated_binding_id = binding_id_at(&alice_rotated, 2);

    let alice_revoked_phone = easynet_json(
        hub_home.path(),
        [
            "principal",
            "revoke-key",
            "--principal-ura",
            ALICE_URA,
            "--binding-id",
            &alice_phone_binding_id,
            "--proof-ref",
            &alice_rotated_binding_id,
            "--idempotency-key",
            "tls-alice-revoke-phone",
            "--expected-version",
            "4",
            "--json",
        ],
        &hub,
    );
    assert_eq!(active_binding_count(&alice_revoked_phone), 1);

    easynet_json(
        hub_home.path(),
        [
            "principal",
            "configure-recovery",
            "--principal-ura",
            ALICE_URA,
            "--policy-ref",
            "recovery-policy:tls-alice",
            "--proof-ref",
            &alice_rotated_binding_id,
            "--idempotency-key",
            "tls-alice-recovery-policy",
            "--expected-version",
            "5",
            "--json",
        ],
        &hub,
    );

    let alice_recovery_pubkey = pubkey(43);
    let alice_recovered = easynet_json(
        hub_home.path(),
        [
            "principal",
            "recover",
            "--principal-ura",
            ALICE_URA,
            "--proof-ref",
            "recovery-policy:tls-alice",
            "--public-key-b64",
            &alice_recovery_pubkey,
            "--key-id",
            "tls-alice-recovery-key",
            "--idempotency-key",
            "tls-alice-recover",
            "--expected-version",
            "6",
            "--json",
        ],
        &hub,
    );
    assert_eq!(active_binding_count(&alice_recovered), 2);

    let alice_replay_recovery_pubkey = pubkey(44);
    let alice_replay_recovery_error = easynet_failure(
        hub_home.path(),
        [
            "principal",
            "recover",
            "--principal-ura",
            ALICE_URA,
            "--proof-ref",
            "recovery-policy:tls-alice",
            "--public-key-b64",
            &alice_replay_recovery_pubkey,
            "--key-id",
            "tls-alice-recovery-replay-key",
            "--idempotency-key",
            "tls-alice-recover-replay",
            "--expected-version",
            "7",
            "--json",
        ],
        &hub,
    );
    assert!(
        alice_replay_recovery_error.contains("already been consumed"),
        "Hub TCP+TLS recovery replay must surface daemon denial, got: {alice_replay_recovery_error}"
    );

    let alice_suspended = easynet_json(
        hub_home.path(),
        [
            "principal",
            "suspend",
            "--principal-ura",
            ALICE_URA,
            "--proof-kind",
            "active-key",
            "--proof-ref",
            &alice_rotated_binding_id,
            "--idempotency-key",
            "tls-alice-suspend",
            "--expected-version",
            "7",
            "--json",
        ],
        &hub,
    );
    assert_eq!(alice_suspended["principal"]["state"], "suspended");

    let alice_reactivated = easynet_json(
        hub_home.path(),
        [
            "principal",
            "reactivate",
            "--principal-ura",
            ALICE_URA,
            "--proof-kind",
            "recovery",
            "--proof-ref",
            "recovery-policy:tls-alice",
            "--idempotency-key",
            "tls-alice-reactivate",
            "--expected-version",
            "8",
            "--json",
        ],
        &hub,
    );
    assert_eq!(alice_reactivated["principal"]["state"], "active");

    let bob_phone = easynet_json(
        hub_home.path(),
        [
            "principal",
            "add-key",
            "--principal-ura",
            BOB_URA,
            "--proof-ref",
            &bob_initial_binding_id,
            "--idempotency-key",
            "tls-bob-phone",
            "--expected-version",
            "2",
            "--json",
        ],
        &hub,
    );
    assert_eq!(active_binding_count(&bob_phone), 2);

    easynet_json(
        hub_home.path(),
        [
            "principal",
            "configure-recovery",
            "--principal-ura",
            BOB_URA,
            "--policy-ref",
            "recovery-policy:tls-bob-deleted",
            "--proof-ref",
            &bob_initial_binding_id,
            "--idempotency-key",
            "tls-bob-deleted-recovery-policy",
            "--expected-version",
            "3",
            "--json",
        ],
        &hub,
    );

    let wrong_delete_grant = easynet_json(
        hub_home.path(),
        [
            "principal",
            "issue-grant",
            "--principal-ura",
            ADMIN_URA,
            "--action",
            "principal.lifecycle.add_key",
            "--proof-ref",
            &admin_binding_id,
            "--idempotency-key",
            "tls-admin-wrong-delete-grant",
            "--expected-version",
            "6",
            "--json",
        ],
        &hub,
    );
    let wrong_delete_grant_id = wrong_delete_grant["principal"]["grants"][0]["grant_id"]
        .as_str()
        .expect("wrong-action grant id")
        .to_string();

    let wrong_grant_delete_error = easynet_failure(
        hub_home.path(),
        [
            "principal",
            "delete",
            "--principal-ura",
            BOB_URA,
            "--actor-ura",
            ADMIN_URA,
            "--proof-kind",
            "grant",
            "--proof-ref",
            &wrong_delete_grant_id,
            "--idempotency-key",
            "tls-bob-delete-wrong-grant",
            "--expected-version",
            "4",
            "--json",
        ],
        &hub,
    );
    assert!(
        wrong_grant_delete_error.contains("grant proof reference"),
        "Hub TCP+TLS grant scope denial must surface daemon authorization failure, got: {wrong_grant_delete_error}"
    );
    let bob_after_wrong_grant_delete = easynet_json(
        hub_home.path(),
        ["principal", "get", "--principal-ura", BOB_URA, "--json"],
        &hub,
    );
    assert_eq!(
        bob_after_wrong_grant_delete["principal"]["state"], "active",
        "wrong-action grant must not mutate Bob before the valid delete grant"
    );

    let delete_grant = easynet_json(
        hub_home.path(),
        [
            "principal",
            "issue-grant",
            "--principal-ura",
            ADMIN_URA,
            "--action",
            PRINCIPAL_DELETE,
            "--proof-ref",
            &admin_binding_id,
            "--idempotency-key",
            "tls-admin-delete-grant",
            "--expected-version",
            "7",
            "--json",
        ],
        &hub,
    );
    let delete_grant_id = grant_id_for_action(&delete_grant, PRINCIPAL_DELETE);

    let bob_deleted = easynet_json(
        hub_home.path(),
        [
            "principal",
            "delete",
            "--principal-ura",
            BOB_URA,
            "--actor-ura",
            ADMIN_URA,
            "--proof-kind",
            "grant",
            "--proof-ref",
            &delete_grant_id,
            "--idempotency-key",
            "tls-bob-delete",
            "--expected-version",
            "4",
            "--json",
        ],
        &hub,
    );
    assert_eq!(bob_deleted["principal"]["state"], "deleted");

    let bob_deleted_recovery_pubkey = pubkey(45);
    let bob_deleted_recovery_error = easynet_failure(
        hub_home.path(),
        [
            "principal",
            "recover",
            "--principal-ura",
            BOB_URA,
            "--proof-ref",
            "recovery-policy:tls-bob-deleted",
            "--public-key-b64",
            &bob_deleted_recovery_pubkey,
            "--key-id",
            "tls-bob-deleted-recovery-key",
            "--idempotency-key",
            "tls-bob-recover-deleted",
            "--json",
        ],
        &hub,
    );
    assert!(
        bob_deleted_recovery_error.contains("principal must be active or suspended"),
        "Hub TCP+TLS deleted-principal recovery must surface daemon terminality, got: {bob_deleted_recovery_error}"
    );

    hub.shutdown_for_restart();
    let mut hub = HubDaemon::spawn(hub_home.path(), port);

    let alice_after_restart = easynet_json(
        hub_home.path(),
        ["principal", "get", "--principal-ura", ALICE_URA, "--json"],
        &hub,
    );
    assert_eq!(alice_after_restart["principal"]["state"], "active");
    assert_eq!(active_binding_count(&alice_after_restart), 2);

    let bob_after_restart = easynet_json(
        hub_home.path(),
        ["principal", "get", "--principal-ura", BOB_URA, "--json"],
        &hub,
    );
    assert_eq!(bob_after_restart["principal"]["state"], "deleted");

    let admin_after_restart = easynet_json(
        hub_home.path(),
        ["principal", "get", "--principal-ura", ADMIN_URA, "--json"],
        &hub,
    );
    assert_eq!(admin_after_restart["principal"]["state"], "active");
    assert!(
        admin_after_restart["principal"]["grants"]
            .as_array()
            .expect("admin grants")
            .iter()
            .any(|grant| grant["grant_id"] == delete_grant_id),
        "admin delete grant must persist across Hub restart"
    );

    let hub_trust =
        RealmTrustAnchor::try_load_strict(&hub_trust_path).expect("reload hub trust anchor");
    let owner = hub_trust
        .lookup_principal_owner(&device_ura)
        .expect("Hub owner binding must persist across restart");
    assert_eq!(owner.owner_ura, ALICE_URA);
    assert!(
        hub_trust
            .lookup_user_by_pubkey(ALICE_URA, &alice_initial_pubkey)
            .is_none(),
        "rotated Alice key must no longer be active in RuntimeTrust"
    );
    assert!(
        hub_trust
            .lookup_user_by_pubkey(ALICE_URA, &alice_phone_pubkey)
            .is_none(),
        "revoked Alice sibling key must no longer be active in RuntimeTrust"
    );
    assert!(
        hub_trust
            .lookup_user_by_pubkey(ALICE_URA, &alice_rotated_pubkey)
            .is_some(),
        "rotated Alice key must remain active after Hub restart"
    );
    assert!(
        hub_trust
            .lookup_user_by_pubkey(ALICE_URA, &alice_recovery_pubkey)
            .is_some(),
        "recovery key must remain active after Hub restart"
    );
    assert!(
        hub_trust
            .lookup_user_by_pubkey(ALICE_URA, &alice_replay_recovery_pubkey)
            .is_none(),
        "replayed recovery key must not be projected into RuntimeTrust"
    );
    assert!(
        hub_trust
            .lookup_user_by_pubkey(BOB_URA, &bob_deleted_recovery_pubkey)
            .is_none(),
        "deleted-principal recovery key must not be projected into RuntimeTrust"
    );

    hub.assert_still_running();
}

struct HubDaemon {
    child: Child,
    stdout_log: PathBuf,
    stderr_log: PathBuf,
    keyring_log: PathBuf,
    keyring_socket: PathBuf,
}

impl HubDaemon {
    fn spawn(home: &Path, port: u16) -> Self {
        let log_dir = home.join(".easynet/test-logs");
        std::fs::create_dir_all(&log_dir).expect("create hub log dir");
        let stdout_log = log_dir.join("hub-daemon.stdout.log");
        let stderr_log = log_dir.join("hub-daemon.stderr.log");
        let keyring_log = home.join(".easynet/logs/easynet-keyring.log");
        let keyring_socket = home.join(".easynet/keyring.sock");
        let child = Command::new(env!("CARGO_BIN_EXE_easynet-daemon"))
            .env("HOME", home)
            .env("EASYNET_BOOTSTRAP_MEDIA_RESOURCES", "0")
            .env("EASYNET_KEYRING_BIN", env!("CARGO_BIN_EXE_easynet-keyring"))
            .env_remove("EASYNET_DAEMON_GRPC_UDS")
            .env_remove("EASYNET_REALM_TRUST_PATH")
            .env_remove("EASYNET_KEYRING_SOCKET_PATH")
            .stdout(Stdio::from(
                File::create(&stdout_log).expect("hub stdout log"),
            ))
            .stderr(Stdio::from(
                File::create(&stderr_log).expect("hub stderr log"),
            ))
            .spawn()
            .expect("spawn hub daemon");
        let mut daemon = Self {
            child,
            stdout_log,
            stderr_log,
            keyring_log,
            keyring_socket,
        };
        daemon.wait_for_tcp_listener(port);
        daemon
    }

    fn wait_for_tcp_listener(&mut self, port: u16) {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if TcpStream::connect(addr).is_ok() {
                return;
            }
            if let Some(status) = self.child.try_wait().expect("poll hub daemon") {
                panic!(
                    "hub daemon exited before TCP listener became ready: {status}\nstdout:\n{}\nstderr:\n{}\nkey-service:\n{}",
                    read_to_string(&self.stdout_log),
                    read_to_string(&self.stderr_log),
                    read_to_string(&self.keyring_log),
                );
            }
            assert!(
                Instant::now() < deadline,
                "hub daemon TCP listener did not become ready at {addr}\nstdout:\n{}\nstderr:\n{}\nkey-service:\n{}",
                read_to_string(&self.stdout_log),
                read_to_string(&self.stderr_log),
                read_to_string(&self.keyring_log),
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn assert_still_running(&mut self) {
        assert!(
            self.child.try_wait().expect("poll hub daemon").is_none(),
            "hub daemon exited unexpectedly\nstdout:\n{}\nstderr:\n{}\nkey-service:\n{}",
            read_to_string(&self.stdout_log),
            read_to_string(&self.stderr_log),
            read_to_string(&self.keyring_log),
        );
    }

    fn shutdown_for_restart(mut self) {
        self.stop_child();
        assert!(
            self.wait_for_keyring_endpoint_closed(Duration::from_secs(10)),
            "Hub key service endpoint stayed reachable at {} after daemon shutdown\nkey-service:\n{}",
            self.keyring_socket.display(),
            read_to_string(&self.keyring_log),
        );
    }

    fn stop_child(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }

    fn wait_for_keyring_endpoint_closed(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while UnixStream::connect(&self.keyring_socket).is_ok() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        true
    }
}

impl Drop for HubDaemon {
    fn drop(&mut self) {
        self.stop_child();
        let _ = self.wait_for_keyring_endpoint_closed(Duration::from_secs(10));
    }
}

fn run_easynet<const N: usize>(home: &Path, args: [&str; N], hub: &HubDaemon) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_easynet"))
        .env("HOME", home)
        .env("EASYNET_BOOTSTRAP_MEDIA_RESOURCES", "0")
        .env("EASYNET_KEYRING_BIN", env!("CARGO_BIN_EXE_easynet-keyring"))
        .env_remove("EASYNET_DAEMON_GRPC_UDS")
        .env_remove("EASYNET_REALM_TRUST_PATH")
        .env_remove("EASYNET_KEYRING_SOCKET_PATH")
        .args(args)
        .output()
        .expect("run easynet");

    assert!(
        output.status.success(),
        "easynet failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}\nhub stdout:\n{}\nhub stderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        read_to_string(&hub.stdout_log),
        read_to_string(&hub.stderr_log),
    );
    output.stdout
}

fn easynet_failure<const N: usize>(home: &Path, args: [&str; N], hub: &HubDaemon) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_easynet"))
        .env("HOME", home)
        .env("EASYNET_BOOTSTRAP_MEDIA_RESOURCES", "0")
        .env("EASYNET_KEYRING_BIN", env!("CARGO_BIN_EXE_easynet-keyring"))
        .env_remove("EASYNET_DAEMON_GRPC_UDS")
        .env_remove("EASYNET_REALM_TRUST_PATH")
        .env_remove("EASYNET_KEYRING_SOCKET_PATH")
        .args(args)
        .output()
        .expect("run easynet");

    assert!(
        !output.status.success(),
        "easynet unexpectedly succeeded\nstdout:\n{}\nstderr:\n{}\nhub stdout:\n{}\nhub stderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        read_to_string(&hub.stdout_log),
        read_to_string(&hub.stderr_log),
    );
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn easynet_json<const N: usize>(home: &Path, args: [&str; N], hub: &HubDaemon) -> Value {
    let stdout = run_easynet(home, args, hub);
    let value: Value = serde_json::from_slice(&stdout).unwrap_or_else(|error| {
        panic!(
            "parse easynet JSON: {error}\nstdout:\n{}",
            String::from_utf8_lossy(&stdout)
        )
    });
    assert_no_private_key_material(&value, "$");
    value
}

fn binding_id_at(snapshot: &Value, index: usize) -> String {
    snapshot["principal"]["bindings"][index]["binding_id"]
        .as_str()
        .expect("binding id")
        .to_string()
}

fn newest_enrollment_id(snapshot: &Value) -> String {
    snapshot["principal"]["enrollments"]
        .as_array()
        .expect("enrollments")
        .last()
        .expect("newest enrollment")
        .get("enrollment_id")
        .and_then(Value::as_str)
        .expect("enrollment id")
        .to_string()
}

fn grant_id_for_action(snapshot: &Value, action: &str) -> String {
    snapshot["principal"]["grants"]
        .as_array()
        .expect("grants")
        .iter()
        .find(|grant| {
            grant["actions"]
                .as_array()
                .expect("grant actions")
                .iter()
                .any(|item| item.as_str() == Some(action))
        })
        .and_then(|grant| grant["grant_id"].as_str())
        .expect("grant id for action")
        .to_string()
}

fn public_key_at(snapshot: &Value, index: usize) -> String {
    snapshot["principal"]["bindings"][index]["public_key_b64"]
        .as_str()
        .expect("public key")
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

fn assert_no_private_key_material(value: &Value, path: &str) {
    const FORBIDDEN_FIELD_TOKENS: &[&str] = &[
        "seed",
        "private",
        "secret",
        "vault",
        "passphrase",
        "master_key",
        "ciphertext",
        "keyring",
        "storage_path",
    ];
    const FORBIDDEN_VALUE_MARKERS: &[&str] = &["BEGIN PRIVATE KEY", "PRIVATE KEY-----"];

    match value {
        Value::Object(object) => {
            for (field, nested) in object {
                let normalized = field.to_ascii_lowercase();
                assert!(
                    !FORBIDDEN_FIELD_TOKENS
                        .iter()
                        .any(|token| normalized.contains(token)),
                    "CLI JSON output leaked private-key custody field at {path}.{field}"
                );
                assert_no_private_key_material(nested, &format!("{path}.{field}"));
            }
        }
        Value::Array(items) => {
            for (index, nested) in items.iter().enumerate() {
                assert_no_private_key_material(nested, &format!("{path}[{index}]"));
            }
        }
        Value::String(text) => {
            assert!(
                !FORBIDDEN_VALUE_MARKERS
                    .iter()
                    .any(|marker| text.contains(marker)),
                "CLI JSON output leaked private-key material marker at {path}"
            );
        }
        _ => {}
    }
}

struct TestCerts {
    ca_pem: PathBuf,
    cert_pem: PathBuf,
    key_pem: PathBuf,
}

fn write_test_ca_and_leaf(home: &Path) -> TestCerts {
    let cert_dir = home.join(".easynet/tls");
    std::fs::create_dir_all(&cert_dir).expect("create cert dir");
    let ca_pem = cert_dir.join("ca.pem");
    let cert_pem = cert_dir.join("cert.pem");
    let key_pem = cert_dir.join("key.pem");

    let (ca, ca_key) = new_ca();
    let (leaf, leaf_key) = new_leaf(&ca, &ca_key);
    std::fs::write(&ca_pem, ca.pem()).expect("write ca pem");
    std::fs::write(&cert_pem, format!("{}{}", leaf.pem(), ca.pem())).expect("write cert chain");
    std::fs::write(&key_pem, leaf_key.serialize_pem()).expect("write key pem");

    TestCerts {
        ca_pem,
        cert_pem,
        key_pem,
    }
}

fn new_ca() -> (Certificate, KeyPair) {
    let mut params = CertificateParams::new(Vec::<String>::new()).expect("ca params");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(DnType::CommonName, "EasyNet test Hub CA");
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params.key_usages.push(KeyUsagePurpose::KeyCertSign);
    params.key_usages.push(KeyUsagePurpose::CrlSign);
    let key = KeyPair::generate().expect("ca key");
    let cert = params.self_signed(&key).expect("ca cert");
    (cert, key)
}

fn new_leaf(ca: &Certificate, ca_key: &KeyPair) -> (Certificate, KeyPair) {
    let mut params = CertificateParams::new(vec!["127.0.0.1".to_string()]).expect("leaf params");
    params
        .distinguished_name
        .push(DnType::CommonName, "127.0.0.1");
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params
        .extended_key_usages
        .push(ExtendedKeyUsagePurpose::ServerAuth);
    let key = KeyPair::generate().expect("leaf key");
    let cert = params.signed_by(&key, ca, ca_key).expect("leaf cert");
    (cert, key)
}

fn write_hub_daemon_config(home: &Path, port: u16, cert: &Path, key: &Path) {
    let state_dir = home.join(".easynet");
    std::fs::create_dir_all(&state_dir).expect("create hub state dir");
    let body = format!(
        r#"[daemon]
mode = "hub"
realm = "{REALM}"
listen_tcp = "127.0.0.1:{port}"
tls_cert_pem = "{}"
tls_key_pem = "{}"
"#,
        cert.display(),
        key.display(),
    );
    std::fs::write(state_dir.join("daemon-config.toml"), body).expect("write hub daemon config");
}

fn free_local_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral local port")
        .local_addr()
        .expect("read local addr")
        .port()
}

fn read_to_string(path: &Path) -> String {
    let mut out = String::new();
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let _ = file.read_to_string(&mut out);
    out
}
