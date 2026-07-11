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
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use easynet_cli::daemon::persistence::config::Credentials;
use easynet_cli::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgentRole};
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use serde_json::Value;

const REALM: &str = "localhost";
const HUB_URA: &str = "easynet:///r/localhost/hub";
const ADMIN_URA: &str = "easynet:///r/localhost/user/admin";
const ALICE_URA: &str = "easynet:///r/localhost/user/alice";

#[test]
fn principal_bound_device_join_hub_ura_uses_real_tcp_tls_daemon_without_backend() {
    let hub_home = tempfile::tempdir().expect("hub HOME");
    let device_home = tempfile::tempdir().expect("device HOME");
    let port = free_local_port();
    let certs = write_test_ca_and_leaf(hub_home.path());
    write_hub_daemon_config(hub_home.path(), port, &certs.cert_pem, &certs.key_pem);

    let mut hub = HubDaemon::spawn(hub_home.path(), port);

    let admin = easynet_json(
        hub_home.path(),
        [
            "principal",
            "bootstrap",
            "--principal-ura",
            ADMIN_URA,
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
    assert_eq!(hub_row.role, TrustedAgentRole::Hub);
    assert_eq!(hub_row.origin_realm.as_deref(), Some(REALM));
    assert!(
        hub_row.tls_ca_pem_path.as_deref().is_some(),
        "Hub trust row must point at the persisted pinned CA"
    );

    let device_ura = easynet_cli::core::ura::device_ura(REALM, &credentials.node_id);
    let device_row = trust
        .lookup(&device_ura)
        .expect("device trust anchor must include joined Device URA row");
    assert_eq!(device_row.role, TrustedAgentRole::Device);

    let hub_trust_path = hub_home.path().join(".easynet/realm-trust.toml");
    let hub_trust =
        RealmTrustAnchor::try_load_strict(&hub_trust_path).expect("load hub trust anchor");
    let owner = hub_trust
        .lookup_principal_owner(&device_ura)
        .expect("Hub RuntimeTrust must bind joined Device URA to Principal URA");
    assert_eq!(owner.principal_ura, device_ura);
    assert_eq!(owner.owner_ura, ALICE_URA);
    assert_eq!(owner.owner_user_id, "alice");

    hub.assert_still_running();
}

struct HubDaemon {
    child: Child,
    stdout_log: PathBuf,
    stderr_log: PathBuf,
}

impl HubDaemon {
    fn spawn(home: &Path, port: u16) -> Self {
        let log_dir = home.join(".easynet/test-logs");
        std::fs::create_dir_all(&log_dir).expect("create hub log dir");
        let stdout_log = log_dir.join("hub-daemon.stdout.log");
        let stderr_log = log_dir.join("hub-daemon.stderr.log");
        let child = Command::new(env!("CARGO_BIN_EXE_easynet-daemon"))
            .env("HOME", home)
            .env("EASYNET_BOOTSTRAP_MEDIA_RESOURCES", "0")
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
                    "hub daemon exited before TCP listener became ready: {status}\nstdout:\n{}\nstderr:\n{}",
                    read_to_string(&self.stdout_log),
                    read_to_string(&self.stderr_log),
                );
            }
            assert!(
                Instant::now() < deadline,
                "hub daemon TCP listener did not become ready at {addr}\nstdout:\n{}\nstderr:\n{}",
                read_to_string(&self.stdout_log),
                read_to_string(&self.stderr_log),
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn assert_still_running(&mut self) {
        assert!(
            self.child.try_wait().expect("poll hub daemon").is_none(),
            "hub daemon exited unexpectedly\nstdout:\n{}\nstderr:\n{}",
            read_to_string(&self.stdout_log),
            read_to_string(&self.stderr_log),
        );
    }
}

impl Drop for HubDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run_easynet<const N: usize>(home: &Path, args: [&str; N], hub: &HubDaemon) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_easynet"))
        .env("HOME", home)
        .env("EASYNET_BOOTSTRAP_MEDIA_RESOURCES", "0")
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

fn easynet_json<const N: usize>(home: &Path, args: [&str; N], hub: &HubDaemon) -> Value {
    let stdout = run_easynet(home, args, hub);
    serde_json::from_slice(&stdout).unwrap_or_else(|error| {
        panic!(
            "parse easynet JSON: {error}\nstdout:\n{}",
            String::from_utf8_lossy(&stdout)
        )
    })
}

fn binding_id_at(snapshot: &Value, index: usize) -> String {
    snapshot["principal"]["bindings"][index]["binding_id"]
        .as_str()
        .expect("binding id")
        .to_string()
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
