// EasyNet CLI — signed two-daemon cross-Hub TLS integration test
// ===================================================================
//
// File: tests/cross_hub_two_daemon_real_tls_e2e.rs
// Description: Spawns two real Hub-mode `easynet-daemon` binaries on
//              ephemeral TCP+TLS ports, provisions their public identity
//              projections from their real key-service processes, exchanges
//              peer trust, and drives a descriptor-bound signed invocation
//              through Hub A to Hub B's local `meta.list_abilities` runtime.
//
// Why this test
// -------------
// PR-N1 commits 1-5/N ship the cross-hub federation wire surface;
// commit 6/N plugs `CrossHubDialer` into the daemon's boot path. The
// in-process e2e in commit 5/N (`cross_hub_forward_invoke_e2e_in_process`)
// exercises the routing chain through a `ForwardingPeerClient`
// fixture — it does NOT prove the binary's `start_daemon_invocation_transport`
// actually constructs and threads the dialer end-to-end.
//
// The success condition is deliberately end-to-end: Hub B must execute its
// own introspection ability and the catalog bytes must return through Hub A.
// Local admission denial, target-offline, or any transport error fails.
// - `rcgen` self-signed certs are generated at test time. CN/SAN
//   set to `localhost` + `127.0.0.1` so tonic's TLS hostname check
//   accepts the connection.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

#![cfg(feature = "axon-pb")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rcgen::{generate_simple_self_signed, CertifiedKey};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::time::{sleep, timeout};
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint};

use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;
use easynet_axon::pb::axon::v1::InvokeRequest;
use easynet_cli::daemon::ability::DEFAULT_ABILITY_DESCRIPTOR_VERSION;
use easynet_cli::daemon::identity::self_identity::KeyringClient;
use easynet_cli::daemon::invocation::admission::decision::{
    AccessAction, PrincipalKind, TokenClass,
};
use easynet_cli::daemon::invocation::admission::grant_matcher::{
    PermissionEffect, PermissionGrant, PermissionGrantLifetime, PermissionGrantState,
};
use easynet_cli::daemon::invocation::dispatch::federation_wrappers::{
    ForwardInvokeRequest, ForwardInvokeResponse, ABILITY_FEDERATION_FORWARD_INVOKE,
};
use easynet_cli::daemon::invocation::dispatch::invocation_wire::ProtoEnvelope;
use easynet_cli::daemon::persistence::access_control::AccessControlStore;

const TARGET_ABILITY: &str = "meta.list_abilities";
const OWNER_A: &str = "cross-hub-owner-a";
const OWNER_B: &str = "cross-hub-owner-b";
const CALL_ID: &str = "cross-hub-real-tls-call";

/// One daemon's filesystem layout for the test.
///
/// Owns the tempdir + child handle so dropping the harness kills
/// the daemon and removes its config.
struct DaemonHarness {
    /// Tempdir rooted as `HOME` for this daemon. The daemon writes
    /// `daemon-config.toml`, `realm-trust.toml`, and `daemon.sock`
    /// under here.
    home: tempfile::TempDir,
    child: Child,
    /// `https://127.0.0.1:<port>` — the TCP+TLS hub endpoint peers dial.
    hub_endpoint: String,
    /// Path to the leaf cert PEM. Used by the OTHER daemon's
    /// `realm-trust.toml` as the pinned CA the cross-hub dialer
    /// trusts. `Certificate::from_pem` accepts a self-signed
    /// cert as a "CA" for verification purposes.
    cert_pem_path: PathBuf,
    /// Canonical Hub identity owned by this daemon's key service.
    hub_ura: String,
    /// Realm used for peer-routing and trust projections.
    realm: String,
}

impl Drop for DaemonHarness {
    fn drop(&mut self) {
        // Best-effort kill. The OS reclaims the TCP+TLS port on
        // process exit; tempdir cleanup happens on TempDir drop.
        let _ = self.child.start_kill();
    }
}

/// Generate a self-signed cert + private key for `localhost` and
/// `127.0.0.1`. Returns the PEM-encoded strings.
fn make_self_signed_cert() -> (String, String) {
    let CertifiedKey { cert, key_pair } =
        generate_simple_self_signed(vec!["localhost".to_string(), "127.0.0.1".to_string()])
            .expect("rcgen self-signed cert");
    (cert.pem(), key_pair.serialize_pem())
}

/// Pick a free TCP port by binding `127.0.0.1:0` and reading back
/// the assigned port. The socket is dropped before this returns,
/// so the port is technically free-then-reusable; tonic re-binds
/// it on the daemon side. This is the standard pattern for ephemeral-
/// port integration tests; the inherent TOCTOU window is small
/// enough in practice.
fn pick_free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    listener.local_addr().expect("local_addr").port()
}

/// Wait until `127.0.0.1:port` accepts TCP connections, or fail
/// after `timeout_duration`. Used to gate the test on both daemons
/// having bound their TCP+TLS listeners.
async fn wait_for_tcp_bind(port: u16, timeout_duration: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout_duration;
    while tokio::time::Instant::now() < deadline {
        if let Ok(Ok(_)) = timeout(
            Duration::from_millis(100),
            TcpStream::connect(("127.0.0.1", port)),
        )
        .await
        {
            return true;
        }
        sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Stream the daemon's stderr to the test process's stderr with a
/// per-line `[daemon-A]` / `[daemon-B]` prefix so a failing test
/// can be debugged from `cargo test --nocapture` output.
fn pipe_daemon_stderr(child: &mut Child, label: &'static str) {
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                eprintln!("[{label}] {line}");
            }
        });
    }
}

/// Build a `daemon-config.toml` for a hub-mode daemon. Returns the
/// TOML body the caller writes under `<home>/.easynet/`.
fn daemon_config_body(
    realm: &str,
    listen_tcp: &str,
    cert_pem_path: &Path,
    key_pem_path: &Path,
    federated_peers: &[(String, String)],
) -> String {
    let mut body = format!(
        r#"
[daemon]
mode = "hub"
realm = "{realm}"
listen_tcp = "{listen_tcp}"
tls_cert_pem = {cert:?}
tls_key_pem = {key:?}

[daemon.federated_peers]
"#,
        cert = cert_pem_path.to_string_lossy(),
        key = key_pem_path.to_string_lossy(),
    );
    for (realm, hub_endpoint) in federated_peers {
        body.push_str(&format!("{realm:?} = {hub_endpoint:?}\n"));
    }
    body
}

#[derive(Debug)]
struct TrustedRuntimeFixture {
    agent_ura: String,
    public_key_b64: String,
    role: &'static str,
    origin_realm: Option<String>,
    hub_endpoint: Option<String>,
    tls_ca_pem_path: Option<PathBuf>,
}

#[derive(Debug)]
struct PrincipalOwnerFixture {
    principal_ura: String,
    owner_user_id: String,
    owner_ura: String,
}

/// Build the exact trust document consumed by both admission and the
/// cross-Hub dial gate. Runtime rows always carry key-service projections;
/// there are no sentinel keys or role aliases in this fixture.
fn realm_trust_body(
    runtimes: &[TrustedRuntimeFixture],
    owners: &[PrincipalOwnerFixture],
) -> String {
    let mut body = String::new();
    for (i, runtime) in runtimes.iter().enumerate() {
        let origin_realm = runtime
            .origin_realm
            .as_ref()
            .map_or_else(String::new, |realm| format!("origin_realm = {realm:?}\n"));
        let hub_endpoint = runtime
            .hub_endpoint
            .as_ref()
            .map_or_else(String::new, |endpoint| {
                format!("hub_endpoint = {endpoint:?}\n")
            });
        let tls_ca_pem_path = runtime
            .tls_ca_pem_path
            .as_ref()
            .map_or_else(String::new, |path| {
                format!("tls_ca_pem_path = {:?}\n", path.to_string_lossy())
            });
        body.push_str(&format!(
            r#"
[[trusted_agent]]
agent_ura = {agent_ura:?}
public_key_b64 = {public_key_b64:?}
role = {role:?}
added_at_unix_ms = {ts}
{origin_realm}{hub_endpoint}{tls_ca_pem_path}
"#,
            agent_ura = runtime.agent_ura,
            public_key_b64 = runtime.public_key_b64,
            role = runtime.role,
            ts = 1_714_492_800_000_u64 + (i as u64),
        ));
    }
    for (i, owner) in owners.iter().enumerate() {
        body.push_str(&format!(
            r#"
[[trusted_principal_owner]]
principal_ura = {principal_ura:?}
owner_user_id = {owner_user_id:?}
owner_ura = {owner_ura:?}
added_at_unix_ms = {ts}
"#,
            principal_ura = owner.principal_ura,
            owner_user_id = owner.owner_user_id,
            owner_ura = owner.owner_ura,
            ts = 1_714_492_900_000_u64 + (i as u64),
        ));
    }
    body
}

/// Spawn an `easynet-daemon` binary with the given filesystem
/// layout. Returns a `DaemonHarness` that owns the child process
/// and tempdir. The daemon's `HOME` is rooted at the tempdir so
/// the test does not pollute the developer's `~/.easynet/`.
async fn spawn_daemon(
    label: &'static str,
    realm: &str,
    listen_tcp_port: u16,
    federated_peers: Vec<(String, String)>, // (peer_realm, peer_hub_endpoint)
) -> DaemonHarness {
    let home = tempfile::tempdir().expect("daemon home tempdir");
    let easynet_dir = home.path().join(".easynet");
    std::fs::create_dir_all(&easynet_dir).expect("mkdir .easynet");

    // Generate self-signed cert + key for THIS daemon's TCP+TLS
    // listener. Other daemons that want to dial us pin against
    // this cert.
    let (cert_pem, key_pem) = make_self_signed_cert();
    let cert_path = easynet_dir.join("tls-cert.pem");
    let key_path = easynet_dir.join("tls-key.pem");
    std::fs::write(&cert_path, &cert_pem).expect("write cert pem");
    std::fs::write(&key_path, &key_pem).expect("write key pem");

    let listen_tcp = format!("127.0.0.1:{listen_tcp_port}");
    let config_body =
        daemon_config_body(realm, &listen_tcp, &cert_path, &key_path, &federated_peers);
    std::fs::write(easynet_dir.join("daemon-config.toml"), config_body)
        .expect("write daemon-config.toml");

    // Boot with an empty external trust set. Hub boot writes only its own
    // public projection; once both real key services are ready, the test
    // replaces this document with self + peer projections and asks the daemon
    // to reload it.
    let realm_trust_path = easynet_dir.join("realm-trust.toml");
    std::fs::write(&realm_trust_path, realm_trust_body(&[], &[])).expect("write realm-trust.toml");

    let bin = env!("CARGO_BIN_EXE_easynet-daemon");
    let mut command = Command::new(bin);
    command
        .env("HOME", home.path())
        .env("EASYNET_REALM_TRUST_PATH", &realm_trust_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().expect("spawn easynet-daemon");
    pipe_daemon_stderr(&mut child, label);

    DaemonHarness {
        home,
        child,
        hub_endpoint: format!("https://127.0.0.1:{listen_tcp_port}"),
        cert_pem_path: cert_path,
        hub_ura: easynet_cli::core::ura::hub_ura(realm),
        realm: realm.to_string(),
    }
}

/// Re-write a daemon's `realm-trust.toml` to point each peer's
/// `tls_ca_pem_path` at the supplied paths. Used after both
/// daemons have spawned so each side's trust anchor knows the
/// other's cert path.
fn rewrite_realm_trust(
    home: &Path,
    runtimes: &[TrustedRuntimeFixture],
    owners: &[PrincipalOwnerFixture],
) -> std::io::Result<()> {
    let path = home.join(".easynet").join("realm-trust.toml");
    std::fs::write(path, realm_trust_body(runtimes, owners))
}

fn key_service_client(daemon: &DaemonHarness) -> KeyringClient {
    KeyringClient::new(daemon.home.path().join(".easynet/keyring.sock"))
}

fn ensure_runtime_public_key_b64(daemon: &DaemonHarness, owner_ura: &str) -> String {
    let public_key = key_service_client(daemon)
        .ensure(owner_ura)
        .unwrap_or_else(|err| panic!("ensure key-service identity `{owner_ura}`: {err}"));
    BASE64_STANDARD.encode(public_key.to_bytes())
}

/// Persist Hub B's explicit owner grant for the carried child invocation.
/// The peer carrier proves Hub A's identity; this grant separately proves
/// that the Hub-link token may read Hub B's introspection descriptor.
fn grant_peer_introspection_read(
    daemon_b: &DaemonHarness,
    peer_hub_ura: &str,
    target_ability_ura: &str,
) {
    let owner_ura = easynet_cli::core::ura::user_ura(&daemon_b.realm, OWNER_B);
    let root = daemon_b
        .home
        .path()
        .join(".easynet/access-control")
        .join(OWNER_B);
    let mut store = AccessControlStore::open_or_create_at(root, OWNER_B)
        .expect("open Hub B owner access-control store");
    store
        .create_grant(
            PermissionGrant {
                grant_id: "cross-hub-introspection-read".to_string(),
                owner_user_id: OWNER_B.to_string(),
                principal_kind: PrincipalKind::Token,
                principal_id: peer_hub_ura.to_string(),
                token_id: Some(peer_hub_ura.to_string()),
                token_class: Some(TokenClass::HubLink),
                callee_ura: Some(daemon_b.hub_ura.clone()),
                subject_ura_pattern: Some(target_ability_ura.to_string()),
                ability_ura_pattern: Some(target_ability_ura.to_string()),
                actions: vec![AccessAction::Read],
                constraints: None,
                effect: PermissionEffect::Allow,
                lifetime: PermissionGrantLifetime::Session,
                state: PermissionGrantState::Active,
                expires_at: None,
                review_required_after: None,
                last_reviewed_at: None,
                last_used_at: None,
                created_by: owner_ura.clone(),
                created_at: "2026-07-11T00:00:00Z".to_string(),
                updated_at: None,
                revoked_at: None,
                reason: Some("signed two-Hub TLS integration fixture".to_string()),
            },
            &owner_ura,
        )
        .expect("create Hub B peer introspection-read grant");
}

/// SIGHUP the daemon so it reloads its trust anchor. Used after
/// rewriting realm-trust.toml. The daemon's reload task picks up
/// the new entry without a restart.
#[cfg(unix)]
fn sighup_daemon(child: &Child) -> std::io::Result<()> {
    if let Some(pid) = child.id() {
        // Use libc directly to avoid pulling another dep.
        let result = unsafe { libc::kill(pid as libc::pid_t, libc::SIGHUP) };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "child has no pid (already exited?)",
        ))
    }
}

// Requires the `axon-pb` feature (the transport plane is feature-gated) and
// the built `easynet-daemon` binary, which cargo provides via CARGO_BIN_EXE.
// Runs by default under `cargo test --features axon-pb`.
#[tokio::test]
async fn cross_hub_two_daemon_real_tls_round_trip() {
    // ── 1. Pick ports and boot both real Hub daemons ───────
    let port_a = pick_free_port();
    let port_b = pick_free_port();
    let realm_a = "realm-a";
    let realm_b = "realm-b";
    let hub_a_ura = easynet_cli::core::ura::hub_ura(realm_a);
    let hub_b_ura = easynet_cli::core::ura::hub_ura(realm_b);
    let caller_device_ura = easynet_cli::core::ura::device_ura(realm_a, "signed-client-a");
    let hub_a_endpoint = format!("https://127.0.0.1:{port_a}");
    let hub_b_endpoint = format!("https://127.0.0.1:{port_b}");

    let daemon_b = spawn_daemon(
        "daemon-B",
        realm_b,
        port_b,
        vec![(realm_a.to_string(), hub_a_endpoint.clone())],
    )
    .await;

    let daemon_a = spawn_daemon(
        "daemon-A",
        realm_a,
        port_a,
        vec![(realm_b.to_string(), hub_b_endpoint.clone())],
    )
    .await;

    // ── 2. Wait for both daemons' TCP+TLS listeners to bind ──
    //
    // This MUST happen before we SIGHUP. The daemon installs its
    // SIGHUP reload handler during the transport-boot stage; a SIGHUP
    // delivered before that handler is installed hits the default
    // disposition (terminate) and silently kills the daemon mid-boot.
    // Binding the TCP listener is downstream of handler install, so a
    // successful bind proves the daemon is ready to receive SIGHUP.
    assert!(
        wait_for_tcp_bind(port_a, Duration::from_secs(10)).await,
        "daemon A failed to bind TCP+TLS on port {port_a}",
    );
    assert!(
        wait_for_tcp_bind(port_b, Duration::from_secs(10)).await,
        "daemon B failed to bind TCP+TLS on port {port_b}",
    );

    // ── 3. Project real identities and publish reciprocal trust ──
    let hub_a_public_key_b64 = ensure_runtime_public_key_b64(&daemon_a, &hub_a_ura);
    let hub_b_public_key_b64 = ensure_runtime_public_key_b64(&daemon_b, &hub_b_ura);
    let caller_device_public_key_b64 = ensure_runtime_public_key_b64(&daemon_a, &caller_device_ura);
    let owner_a_ura = easynet_cli::core::ura::user_ura(realm_a, OWNER_A);
    let owner_b_ura = easynet_cli::core::ura::user_ura(realm_b, OWNER_B);

    rewrite_realm_trust(
        daemon_a.home.path(),
        &[
            TrustedRuntimeFixture {
                agent_ura: hub_a_ura.clone(),
                public_key_b64: hub_a_public_key_b64.clone(),
                role: "hub",
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            },
            TrustedRuntimeFixture {
                agent_ura: caller_device_ura.clone(),
                public_key_b64: caller_device_public_key_b64,
                role: "device",
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            },
            TrustedRuntimeFixture {
                agent_ura: hub_b_ura.clone(),
                public_key_b64: hub_b_public_key_b64.clone(),
                role: "hub",
                origin_realm: Some(realm_b.to_string()),
                hub_endpoint: Some(hub_b_endpoint.clone()),
                tls_ca_pem_path: Some(daemon_b.cert_pem_path.clone()),
            },
        ],
        &[
            PrincipalOwnerFixture {
                principal_ura: hub_a_ura.clone(),
                owner_user_id: OWNER_A.to_string(),
                owner_ura: owner_a_ura.clone(),
            },
            PrincipalOwnerFixture {
                principal_ura: caller_device_ura.clone(),
                owner_user_id: OWNER_A.to_string(),
                owner_ura: owner_a_ura,
            },
        ],
    )
    .expect("rewrite A's trust");
    rewrite_realm_trust(
        daemon_b.home.path(),
        &[
            TrustedRuntimeFixture {
                agent_ura: hub_b_ura.clone(),
                public_key_b64: hub_b_public_key_b64,
                role: "hub",
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            },
            TrustedRuntimeFixture {
                agent_ura: hub_a_ura.clone(),
                public_key_b64: hub_a_public_key_b64,
                role: "hub",
                origin_realm: Some(realm_a.to_string()),
                hub_endpoint: Some(hub_a_endpoint.clone()),
                tls_ca_pem_path: Some(daemon_a.cert_pem_path.clone()),
            },
        ],
        &[PrincipalOwnerFixture {
            principal_ura: hub_b_ura.clone(),
            owner_user_id: OWNER_B.to_string(),
            owner_ura: owner_b_ura,
        }],
    )
    .expect("rewrite B's trust");

    #[cfg(unix)]
    {
        sighup_daemon(&daemon_a.child).expect("SIGHUP A");
        sighup_daemon(&daemon_b.child).expect("SIGHUP B");
    }

    // Peer membership does not grant child authority. Hub B's owner grants
    // the authenticated Hub A token one exact Hub-introspection capability.
    let target_ability_ura = easynet_cli::core::ura::owner_ability_ura(&hub_b_ura, TARGET_ABILITY)
        .expect("Hub B introspection ability URA");
    grant_peer_introspection_read(&daemon_b, &hub_a_ura, &target_ability_ura);

    // Give the coordinated reload task one scheduling turn.
    sleep(Duration::from_millis(200)).await;

    // ── 4. Build a TLS gRPC client for daemon A ──────────────
    let ca_pem = std::fs::read(&daemon_a.cert_pem_path).expect("read A cert");
    let ca = Certificate::from_pem(&ca_pem);
    let tls = ClientTlsConfig::new()
        .ca_certificate(ca)
        .domain_name("localhost");
    let endpoint = Endpoint::from_shared(daemon_a.hub_endpoint.clone())
        .expect("endpoint")
        .tls_config(tls)
        .expect("tls config")
        .timeout(Duration::from_secs(10));
    let channel = endpoint.connect_lazy();
    let mut client = InvocationClient::new(channel);

    // ── 5. Sign the complete request and require Hub B's result ──
    let inner_envelope = serde_json::json!({
        "ability_ura": target_ability_ura,
        "subject_ura": target_ability_ura,
        "args": {},
        "call_id": CALL_ID,
    });
    let forward_request = ForwardInvokeRequest {
        target_ura: hub_b_ura.clone(),
        inner_envelope_b64: BASE64_STANDARD.encode(
            serde_json::to_vec(&inner_envelope).expect("encode inner introspection invocation"),
        ),
        causal_context_bytes: Vec::new(),
        forward_deadline_ms: 10_000,
        origin_caller: None,
    };
    let request_args = serde_json::to_vec(&forward_request).expect("encode forward request");
    let forward_ability_ura =
        easynet_cli::core::ura::owner_ability_ura(&hub_a_ura, ABILITY_FEDERATION_FORWARD_INVOKE)
            .expect("Hub A forward ability URA");
    let descriptor_ref = format!("{forward_ability_ura}@{DEFAULT_ABILITY_DESCRIPTOR_VERSION}");
    let signer = key_service_client(&daemon_a);
    let request: InvokeRequest =
        ProtoEnvelope::targeted(&caller_device_ura, &hub_a_ura, &forward_ability_ura)
            .expect("canonical outer forward envelope")
            .signed_descriptor_ref_invoke_request(
                ABILITY_FEDERATION_FORWARD_INVOKE,
                descriptor_ref,
                request_args,
                &signer,
            )
            .expect("descriptor-bound signed outer request");

    let body = timeout(Duration::from_secs(10), client.invoke(request))
        .await
        .expect("signed cross-Hub invocation must terminate")
        .expect("signed cross-Hub invocation must succeed")
        .into_inner();
    let parsed: ForwardInvokeResponse =
        serde_json::from_slice(&body.result).expect("response body is ForwardInvokeResponse JSON");
    assert_eq!(parsed.correlation_call_id, CALL_ID);
    let peer_catalog: serde_json::Value = serde_json::from_slice(&parsed.result_bytes)
        .expect("Hub B meta.list_abilities result is JSON");
    let abilities = peer_catalog
        .get("abilities")
        .and_then(serde_json::Value::as_array)
        .expect("Hub B result carries its ability catalog");
    for ability in abilities {
        let owner_ura = ability
            .get("owner_ura")
            .and_then(serde_json::Value::as_str)
            .expect("every Hub B descriptor carries owner_ura");
        assert_ne!(
            owner_ura, "self",
            "schema template identity must never escape onto the wire: {ability}"
        );
        assert_eq!(
            owner_ura, hub_b_ura,
            "Hub callee must expose only Hub B authority rows: {ability}"
        );
        let name = ability
            .get("name")
            .and_then(serde_json::Value::as_str)
            .expect("every Hub B descriptor carries a public name");
        let ability_ura = ability
            .get("ability_ura")
            .and_then(serde_json::Value::as_str)
            .expect("every Hub B descriptor carries ability_ura");
        easynet_cli::core::ura::parse_ura(ability_ura)
            .unwrap_or_else(|error| panic!("non-canonical Ability URA `{ability_ura}`: {error}"));
        assert_eq!(
            easynet_cli::core::ura::owner_ability_ura(owner_ura, name).as_deref(),
            Some(ability_ura),
            "descriptor owner/name must reproduce its canonical Ability URA: {ability}"
        );
    }
    assert!(
        abilities.iter().any(|ability| {
            ability.get("name").and_then(serde_json::Value::as_str) == Some(TARGET_ABILITY)
                && ability
                    .get("ability_ura")
                    .and_then(serde_json::Value::as_str)
                    == Some(target_ability_ura.as_str())
        }),
        "successful return must contain Hub B's canonical introspection descriptor, not local admission or delivery acceptance: {peer_catalog}"
    );
}
