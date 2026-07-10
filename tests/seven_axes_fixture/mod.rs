// EasyNet CLI — seven-axes e2e fixture (shared)
// ===============================================
//
// File: tests/seven_axes_fixture/mod.rs
// Description: The minimal-but-real daemon stack the seven-axes e2e
//              files boot (spec §3, W1/W2): a fresh HOME seeded
//              through the on-disk product files, plus a real tonic
//              `Invocation` server on a UDS reached through the
//              production client path (`EASYNET_DAEMON_GRPC_UDS`).
//
//              Split into `seed()` (process-global env + files, once
//              per test process) and `start_daemon()` (boot a server
//              against that HOME, restartable) so persistence-across-
//              restart assertions can bounce the daemon without
//              re-seeding.
//
//              Every e2e file using this fixture must hold to the
//              ONE-`#[test]`-per-file rule: the fixture owns process
//              env (`HOME`, `EASYNET_DAEMON_GRPC_UDS`); a second test
//              in the same binary would race it. Separate files are
//              separate processes — safe.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use easynet_axon::invocation::LocalRuntime;
use easynet_axon::pb::axon::v1::invocation_client::InvocationClient;
use easynet_axon::pb::axon::v1::invocation_server::InvocationServer;
use easynet_cli::daemon::ability::catalog::{
    build_registry_with_services_result, RegistryBuildConfig, RegistryBuildServices,
};
use easynet_cli::daemon::identity::self_identity::{SelfIdentity, SelfIdentityError};
use easynet_cli::daemon::invocation::admission::admission_facade::AdmissionFacade;
use easynet_cli::daemon::invocation::bidi::state::presence::PresenceRegistry;
use easynet_cli::daemon::invocation::dispatch::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::daemon::invocation::dispatch::invocation_wire::ProtoEnvelope;
use easynet_cli::daemon::keyring::{
    home_relative, vault_error_to_response, KeyringRequest, KeyringResponse, MasterKeySource,
    Vault, DEFAULT_VAULT_REL,
};
use easynet_cli::daemon::persistence::config::{self, RuntimeKind, RuntimeState};
use easynet_cli::daemon::trust::anchor::RealmTrustAnchor;
use easynet_cli::daemon::trust::cell::SharedTrustAnchor;
use easynet_cli::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver;
use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server, Uri};

const TEST_KEYRING_SEED_BYTE: u8 = 0x11;
pub const TESTBOT_ECHO_DESCRIPTOR_VERSION: &str = "2.3.0";
const FIXTURE_SYSTEM_DESCRIPTOR_VERSION: &str = "1.0.0";

/// A seeded HOME: env pointed, product files written. Keep it alive
/// for the whole test — dropping it deletes the tempdir.
pub struct SevenAxesHome {
    /// RAII keep-alive for the tempdir — never read, must not drop
    /// before the test ends. (Shared across test binaries that use
    /// different subsets of this struct; hence the allows.)
    #[allow(dead_code)]
    pub home: tempfile::TempDir,
    _keyring: TestKeyring,
    pub socket_path: PathBuf,
    pub trust_path: PathBuf,
    /// The CLI's unpaired loopback caller (`device_ura("cli","local")`).
    pub loopback_caller: String,
    /// Canonical URA minted for the seeded agent (RFC-005 §1.4).
    #[allow(dead_code)]
    pub testbot_ura: String,
    /// One ledger handle per HOME, shared across daemon restarts —
    /// redb is single-writer, and restart-persistence tests bounce
    /// the daemon while the database (like all state) belongs to the
    /// HOME, not to the process.
    pub ledger: Arc<easynet_axon::invocation::InvocationLedger>,
    /// Canonical URA of the second seeded agent (`zlearner` — named
    /// to sort AFTER `testbot` so ladder resolution and self-tier
    /// attribution in the W1 assertions stay put). Publishes no
    /// abilities of its own; exists to learn (W3 T3.3).
    #[allow(dead_code)]
    pub zlearner_ura: String,
}

/// A running in-process daemon; dropping it shuts the server down
/// and joins the thread.
pub struct TestDaemon {
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

struct TestKeyring {
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

struct SevenAxesSigner {
    signing_key: SigningKey,
    accepted_uras: Vec<String>,
}

impl SevenAxesSigner {
    fn for_caller(caller_ura: &str) -> Self {
        let mut accepted_uras = vec![caller_ura.to_string()];
        if let Ok(parsed) = easynet_cli::core::ura::parse_ura(caller_ura) {
            accepted_uras.push(easynet_cli::core::ura::hub_ura(&parsed.realm));
        }
        Self {
            signing_key: SigningKey::from_bytes(&test_keyring_seed()),
            accepted_uras,
        }
    }

    fn accepts(&self, self_ura: &str) -> bool {
        self.accepted_uras
            .iter()
            .any(|accepted| accepted == self_ura)
    }

    fn reject_unknown(&self, self_ura: &str) -> SelfIdentityError {
        SelfIdentityError::Rejected {
            kind: "seven_axes_fixture".to_string(),
            message: format!("unknown test signer URA: {self_ura}"),
        }
    }
}

impl SelfIdentity for SevenAxesSigner {
    fn sign(&self, self_ura: &str, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError> {
        if !self.accepts(self_ura) {
            return Err(self.reject_unknown(self_ura));
        }
        Ok(self.signing_key.sign(canonical_bytes))
    }

    fn public_key(&self, self_ura: &str) -> Result<VerifyingKey, SelfIdentityError> {
        if !self.accepts(self_ura) {
            return Err(self.reject_unknown(self_ura));
        }
        Ok(self.signing_key.verifying_key())
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for TestKeyring {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn start_test_keyring(primary_self: String) -> TestKeyring {
    let socket_path = easynet_cli::daemon::keyring::default_socket_path();
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent).expect("keyring socket parent");
    }
    let _ = std::fs::remove_file(&socket_path);

    let vault_path = home_relative(DEFAULT_VAULT_REL);
    if let Some(parent) = vault_path.parent() {
        std::fs::create_dir_all(parent).expect("keyring vault parent");
    }
    let source = MasterKeySource::Explicit("seven-axes-keyring-passphrase".to_string());
    let mut vault = Vault::open_or_init(&vault_path, &source).expect("open test keyring vault");
    let seed_hex = test_keyring_seed_hex();
    match vault.put(
        &primary_self,
        vec![easynet_cli::core::ura::hub_ura("cli")],
        seed_hex,
    ) {
        Ok(()) => vault.seal().expect("seal test keyring vault"),
        Err(easynet_cli::daemon::keyring::VaultError::AlreadyExists(_)) => {}
        Err(err) => panic!("seed test keyring entry: {err}"),
    }

    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test keyring runtime");
        rt.block_on(async move {
            let listener = UnixListener::bind(&socket_path).expect("bind test keyring socket");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))
                    .expect("chmod test keyring socket");
            }
            let _ = ready_tx.send(());
            let vault = Arc::new(tokio::sync::Mutex::new(vault));
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let (stream, _) = accepted.expect("accept test keyring client");
                        let vault = Arc::clone(&vault);
                        tokio::spawn(async move {
                            let _ = handle_test_keyring_connection(stream, vault).await;
                        });
                    }
                }
            }
        });
    });
    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("test keyring ready");
    TestKeyring {
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    }
}

fn test_keyring_seed() -> [u8; easynet_cli::daemon::keyring::ED25519_SEED_LEN] {
    [TEST_KEYRING_SEED_BYTE; easynet_cli::daemon::keyring::ED25519_SEED_LEN]
}

fn test_keyring_seed_hex() -> String {
    format!("{:02x}", TEST_KEYRING_SEED_BYTE).repeat(easynet_cli::daemon::keyring::ED25519_SEED_LEN)
}

fn test_keyring_public_key_b64() -> String {
    let signing_key = SigningKey::from_bytes(&test_keyring_seed());
    base64::engine::general_purpose::STANDARD.encode(signing_key.verifying_key().to_bytes())
}

async fn handle_test_keyring_connection<S>(
    mut stream: S,
    vault: Arc<tokio::sync::Mutex<Vault>>,
) -> anyhow::Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e.into()),
        }
        let frame_len = u32::from_be_bytes(len_buf) as usize;
        if frame_len > 1024 * 1024 {
            let resp = KeyringResponse::err("frame_too_large", "request frame exceeds 1MiB");
            write_test_keyring_response(&mut stream, &resp).await?;
            return Ok(());
        }
        let mut buf = vec![0u8; frame_len];
        stream.read_exact(&mut buf).await?;
        let resp = match serde_json::from_slice::<KeyringRequest>(&buf) {
            Ok(req) => dispatch_test_keyring(req, &vault).await,
            Err(e) => KeyringResponse::err("parse", format!("bad request: {e}")),
        };
        write_test_keyring_response(&mut stream, &resp).await?;
    }
}

async fn dispatch_test_keyring(
    req: KeyringRequest,
    vault: &Arc<tokio::sync::Mutex<Vault>>,
) -> KeyringResponse {
    match req {
        KeyringRequest::Sign {
            self_ura,
            canonical_bytes_b64,
        } => {
            let bytes = match base64::engine::general_purpose::STANDARD.decode(canonical_bytes_b64)
            {
                Ok(bytes) => bytes,
                Err(e) => {
                    return KeyringResponse::err("base64", format!("canonical_bytes_b64: {e}"));
                }
            };
            let guard = vault.lock().await;
            match guard.sign(&self_ura, &bytes) {
                Ok(sig) => KeyringResponse::Signature {
                    signature_b64: base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()),
                },
                Err(e) => vault_error_to_response(e),
            }
        }
        KeyringRequest::DerivePubkey { self_ura } => {
            let guard = vault.lock().await;
            match guard.derive_pubkey(&self_ura) {
                Ok(pk) => KeyringResponse::PublicKey {
                    public_key_b64: base64::engine::general_purpose::STANDARD.encode(pk.to_bytes()),
                },
                Err(e) => vault_error_to_response(e),
            }
        }
        KeyringRequest::List => {
            let guard = vault.lock().await;
            KeyringResponse::List {
                entries: guard.list(),
            }
        }
        KeyringRequest::Forget { primary_self } => {
            let mut guard = vault.lock().await;
            match guard.mutate_and_seal(|vault| {
                vault.forget(&primary_self);
                Ok(())
            }) {
                Ok(()) => KeyringResponse::Ok,
                Err(e) => vault_error_to_response(e),
            }
        }
        _ => KeyringResponse::err("unsupported", "test fixture does not implement request"),
    }
}

async fn write_test_keyring_response<S>(
    stream: &mut S,
    resp: &KeyringResponse,
) -> anyhow::Result<()>
where
    S: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(resp)?;
    stream.write_all(&(body.len() as u32).to_be_bytes()).await?;
    stream.write_all(&body).await?;
    stream.flush().await?;
    Ok(())
}

impl SevenAxesHome {
    /// Point `HOME`/`EASYNET_DAEMON_GRPC_UDS` at a fresh tempdir and
    /// seed it the way the product writes it: `agents.json` (registry
    /// row), `agent.toml` (workspace spec), one flat-layout ability
    /// manifest, `local-agents.json` (hosted-URA mint — without it
    /// the discover ladder rightly projects nothing), and a realm
    /// trust toml admitting the loopback caller. URAs come from the
    /// Axon builders, never hand-written literals (spec §0.1-8).
    pub fn seed() -> Self {
        let home = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", home.path());
        let socket_path = home.path().join("daemon.sock");
        std::env::set_var("EASYNET_DAEMON_GRPC_UDS", &socket_path);

        let agent_root = home.path().join("agents/testbot");
        std::fs::create_dir_all(&agent_root).expect("agent root");
        std::fs::write(
            agent_root.join("agent.toml"),
            "name = \"testbot\"\nruntime = \"claude-code\"\n",
        )
        .expect("write minimal agent.toml");

        let learner_root = home.path().join("agents/zlearner");
        std::fs::create_dir_all(learner_root.join("abilities")).expect("learner root");
        std::fs::write(
            learner_root.join("agent.toml"),
            "name = \"zlearner\"\nruntime = \"claude-code\"\n",
        )
        .expect("write learner agent.toml");

        let abilities_dir = agent_root.join("abilities");
        std::fs::create_dir_all(&abilities_dir).expect("abilities dir");
        std::fs::write(
            abilities_dir.join("weather-probe.ability.toml"),
            "name = \"weather-probe\"\n\
             description = \"fetch the local weather forecast\"\n\
             \n\
             [input_schema]\n\
             type = \"object\"\n",
        )
        .expect("write weather-probe.ability.toml");
        std::fs::write(
            abilities_dir.join("echo.ability.toml"),
            format!(
                "schema_version = \"1\"\n\
             descriptor_version = \"{TESTBOT_ECHO_DESCRIPTOR_VERSION}\"\n\
             name = \"echo\"\n\
             description = \"echo one short string for deterministic mission tests\"\n\
             timeout_seconds = 5\n\
             \n\
             [input_schema]\n\
             type = \"object\"\n\
             \n\
             [input_schema.properties.message]\n\
             type = \"string\"\n\
             \n\
             [exec]\n\
             kind = \"shell\"\n\
             argv = [\"/bin/echo\", \"{{{{ message }}}}\"]\n"
            ),
        )
        .expect("write echo.ability.toml");

        let state_dir = home.path().join(".easynet");
        std::fs::create_dir_all(&state_dir).expect("state dir");
        std::fs::write(
            state_dir.join("agents.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "agents": {
                    "testbot": {
                        "schema_version": 2,
                        "agent_type": "claude-code",
                        "root_path": agent_root,
                    },
                    "zlearner": {
                        "schema_version": 2,
                        "agent_type": "claude-code",
                        "root_path": learner_root,
                    }
                }
            }))
            .expect("encode agents.json"),
        )
        .expect("write agents.json");

        let loopback_caller = easynet_cli::core::ura::device_ura("cli", "local");
        let testbot_ura = easynet_cli::core::ura::agent_ura("cli", "local", "testbot");
        let zlearner_ura = easynet_cli::core::ura::agent_ura("cli", "local", "zlearner");
        std::fs::write(
            state_dir.join("local-agents.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "host_device_agent_ura": loopback_caller,
                "hosted_agents": [{
                    "profile": "llm",
                    "name": "testbot",
                    "agent_ura": testbot_ura,
                    "signing_authority": format!("hosted_by:{loopback_caller}"),
                    "first_seen_at": "2026-06-13T00:00:00+00:00",
                }, {
                    "profile": "llm",
                    "name": "zlearner",
                    "agent_ura": zlearner_ura,
                    "signing_authority": format!("hosted_by:{loopback_caller}"),
                    "first_seen_at": "2026-06-13T00:00:00+00:00",
                }],
            }))
            .expect("encode local-agents.json"),
        )
        .expect("write local-agents.json");

        let trust_path = home.path().join("realm-trust.toml");
        let trusted_public_key_b64 = test_keyring_public_key_b64();
        let mut f = std::fs::File::create(&trust_path).expect("create trust toml");
        write!(
            f,
            r#"
[[trusted_agent]]
agent_ura = "{loopback_caller}"
public_key_b64 = "{trusted_public_key_b64}"
role = "device"
added_at_unix_ms = 0
"#
        )
        .expect("write trust toml");
        drop(f);

        let ledger_path = easynet_cli::daemon::persistence::daemon_config::default_ledger_dir()
            .join("invocations.redb");
        let ledger = Arc::new(
            easynet_axon::invocation::InvocationLedger::open(&ledger_path)
                .expect("open test ledger"),
        );
        config::save(&RuntimeState {
            endpoint: socket_path.display().to_string(),
            runtime_kind: RuntimeKind::DaemonOnly,
            pid: None,
            hub: None,
            tenant: Some("cli".to_string()),
            label: Some("seven-axes-fixture".to_string()),
            started_at: None,
            credential_verified: None,
        })
        .expect("write runtime state for mission runner");
        config::save_credentials(&config::Credentials {
            node_id: "local".to_string(),
            credential_token: "seven-axes-token".to_string(),
            hub_endpoint: socket_path.display().to_string(),
            realm: "cli".to_string(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("local".to_string()),
            user_id: Some("user-local".to_string()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        })
        .expect("write credentials for federation-backed discover scope");
        let keyring = start_test_keyring(loopback_caller.clone());

        SevenAxesHome {
            home,
            _keyring: keyring,
            socket_path,
            trust_path,
            loopback_caller,
            testbot_ura,
            zlearner_ura,
            ledger,
        }
    }

    /// Canonical URA of the seeded `testbot.weather-probe` ability —
    /// minted through the same owner projection the ladder uses.
    #[allow(dead_code)]
    pub fn source_descriptor_ura(&self) -> String {
        easynet_cli::core::ura::owner_ability_ura(&self.testbot_ura, "weather-probe")
            .expect("mint source descriptor URA")
    }

    /// Drive a real `testbot.echo` unary invocation through the daemon
    /// gRPC surface, then poll the shared ledger by request id and
    /// return the persisted invocation metadata the watch/usage e2e
    /// tests need. This stays in the fixture instead of making
    /// crate-internal CLI helpers public just for integration tests.
    #[allow(dead_code)]
    pub fn invoke_testbot_echo_with_meta(&self, message: &str) -> Value {
        let callee = self.testbot_ura.clone();
        let descriptor_ref =
            fixture_descriptor_ref(&self.testbot_ura, "echo", TESTBOT_ECHO_DESCRIPTOR_VERSION);
        let (_value, request_id, terminal_receipt) = invoke_daemon_ability(
            &self.socket_path,
            &self.loopback_caller,
            &callee,
            &self.loopback_caller,
            "echo",
            descriptor_ref.as_str(),
            json!({ "message": message }),
        );
        assert!(
            !request_id.is_empty(),
            "daemon response must carry a request_id"
        );

        let mut record = Value::Null;
        let history_descriptor_ref = fixture_descriptor_ref(
            &self.loopback_caller,
            "invocation.history.get",
            FIXTURE_SYSTEM_DESCRIPTOR_VERSION,
        );
        for _ in 0..10 {
            let (history, _, _) = invoke_daemon_ability(
                &self.socket_path,
                &self.loopback_caller,
                &self.loopback_caller,
                &self.loopback_caller,
                "invocation.history.get",
                history_descriptor_ref.as_str(),
                json!({ "key": { "request_id": request_id } }),
            );
            record = history.get("record").cloned().unwrap_or(Value::Null);
            if !record.is_null() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            !record.is_null(),
            "ledger must contain echo invocation for request_id {request_id}"
        );

        json!({
            "request_id": request_id,
            "trace_id": record.get("trace_id").cloned().unwrap_or(Value::Null),
            "invocation_ura": record.get("invocation_ura").cloned().unwrap_or(Value::Null),
            "caller_ura": record.get("caller_ura").cloned().unwrap_or(Value::Null),
            "callee_ura": record.get("callee_ura").cloned().unwrap_or(Value::Null),
            "subject_ura": record.get("subject_ura").cloned().unwrap_or(Value::Null),
            "ability": "echo",
            "ledger_state": record.get("state").cloned().unwrap_or(Value::Null),
            "receipt_proof_facts": terminal_receipt
                .as_ref()
                .map(receipt_proof_facts_value)
                .unwrap_or(Value::Null),
        })
    }

    /// Boot the real daemon surface against this HOME: full system
    /// catalogue (agents read from disk through the production
    /// loader) materialised into a `LocalRuntime`, served by
    /// `DaemonInvocationService` over tonic — the same stack
    /// `easynet runtime start` wires, minus hub/plugin/session
    /// concerns these tests do not exercise. Restartable: state lives
    /// in the seeded HOME, not in the daemon.
    pub fn start_daemon(&self) -> TestDaemon {
        start_daemon_at(
            &self.socket_path,
            &self.trust_path,
            self.loopback_caller.clone(),
            Arc::clone(&self.ledger),
        )
    }
}

async fn connect_to_daemon(socket_path: &Path) -> Channel {
    let path = socket_path.to_path_buf();
    Endpoint::try_from("http://[::]:50051")
        .expect("valid endpoint")
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let path = path.clone();
            async move {
                let stream = UnixStream::connect(path).await?;
                Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
            }
        }))
        .await
        .expect("connect to daemon")
}

fn invoke_daemon_ability(
    socket_path: &Path,
    caller_ura: &str,
    callee_ura: &str,
    subject_ura: &str,
    function_name: &str,
    descriptor_ability_ref: &str,
    args: Value,
) -> (
    Value,
    String,
    Option<easynet_axon::pb::axon::v1::InvocationReceipt>,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    rt.block_on(async {
        let mut client = InvocationClient::new(connect_to_daemon(socket_path).await);
        let arguments = serde_json::to_vec(&args).expect("encode daemon invoke args");
        let signer = SevenAxesSigner::for_caller(caller_ura);
        let envelope = ProtoEnvelope::targeted(caller_ura, callee_ura, subject_ura)
            .expect("valid seven-axes invoke envelope");
        let request = envelope
            .signed_descriptor_ref_invoke_request(
                function_name,
                descriptor_ability_ref,
                arguments,
                &signer,
            )
            .expect("valid seven-axes descriptor-ref signed invoke request");
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            client.invoke(tonic::Request::new(request)),
        )
        .await
        .expect("daemon invoke must not hang")
        .expect("daemon invoke must succeed")
        .into_inner();
        let request_id = response
            .header
            .as_ref()
            .map(|header| header.request_id.clone())
            .unwrap_or_default();
        let terminal_receipt = response.terminal_receipt.clone();
        let value = if response.result.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&response.result).expect("daemon result must be JSON")
        };
        (value, request_id, terminal_receipt)
    })
}

fn receipt_proof_facts_value(receipt: &easynet_axon::pb::axon::v1::InvocationReceipt) -> Value {
    json!({
        "descriptor_version": receipt.descriptor_version.as_str(),
        "schema_hash": hex::encode(&receipt.schema_hash),
        "impl_hash": hex::encode(&receipt.impl_hash),
        "runtime_env": receipt.runtime_env.as_str(),
        "input_hash": hex::encode(&receipt.input_hash),
        "output_hash": hex::encode(&receipt.output_hash),
        "ability_binding": receipt.ability_binding.as_str(),
        "has_authority_proof": receipt.authority_proof.is_some(),
        "has_callee_signature": receipt.callee_signature.is_some(),
    })
}

fn fixture_descriptor_ref(callee_ura: &str, function_name: &str, version: &str) -> String {
    format!(
        "{}@{version}",
        easynet_cli::core::ura::owner_ability_ura(callee_ura, function_name)
            .expect("fixture ability URA")
    )
}

fn seed_hosted_agent_projection(
    socket_path: &Path,
    caller_ura: &str,
    host_device_ura: &str,
    agent_ura: &str,
) {
    let advertise_agent_descriptor_ref = fixture_descriptor_ref(
        host_device_ura,
        "federation.advertise_agent",
        FIXTURE_SYSTEM_DESCRIPTOR_VERSION,
    );
    let (agent_resp, _, _) = invoke_daemon_ability(
        socket_path,
        caller_ura,
        host_device_ura,
        caller_ura,
        "federation.advertise_agent",
        advertise_agent_descriptor_ref.as_str(),
        json!({
            "agent_ura": agent_ura,
            "public_key_hex": "",
            "host_node_id": "local",
            "signing_authority": {
                "kind": "hosted_by",
                "host_ura": host_device_ura,
            }
        }),
    );
    assert_eq!(
        agent_resp["ack"], true,
        "fixture hosted-agent advertise must ack: {agent_resp}"
    );

    let public_name = easynet_cli::core::ura::owner_local_ability_name(agent_ura, "echo");
    let ability_ura = easynet_cli::core::ura::owner_ability_ura(agent_ura, &public_name)
        .expect("fixture echo ability URA");
    let route_summary_ref = format!("route-ref::{ability_ura}");
    let (namespace, local_name) = public_name
        .rsplit_once('.')
        .map(|(namespace, local)| (namespace.to_string(), local.to_string()))
        .unwrap_or_else(|| (String::new(), public_name.clone()));
    let advertise_abilities_descriptor_ref = fixture_descriptor_ref(
        host_device_ura,
        "federation.advertise_abilities",
        FIXTURE_SYSTEM_DESCRIPTOR_VERSION,
    );
    let (abilities_resp, _, _) = invoke_daemon_ability(
        socket_path,
        caller_ura,
        host_device_ura,
        caller_ura,
        "federation.advertise_abilities",
        advertise_abilities_descriptor_ref.as_str(),
        json!({
            "agent_ura": agent_ura,
            "owner_ura": agent_ura,
            "host_device_ura": host_device_ura,
            "projection_revision": 1,
            "projection_digest": "sha256:seven-axes-fixture-echo",
            "lease_expires_unix_ms": 0,
            "ability_summaries": [{
                "ability_ura": ability_ura,
                "owner_ura": agent_ura,
                "namespace": namespace,
                "local_name": local_name,
                "descriptor_revision": "sha256:seven-axes-fixture-echo-descriptor",
                "schema_ref": null,
                "schema_hash": null,
                "policy_ref": "visibility:private",
                "route_summary_ref": route_summary_ref,
                "tags": ["class:query", "source:seven-axes-fixture"],
                "callable_summary": {
                    "public_name": public_name,
                    "description": "echo one short string for deterministic mission tests",
                    "ability_class": "query",
                    "input_fields": [{
                        "name": "message",
                        "required": false,
                        "value_type": "string"
                    }],
                    "flags": {
                        "read_only": false,
                        "destructive": false,
                        "idempotent": true,
                        "streaming_only": false,
                        "bidi_only": false
                    }
                }
            }]
        }),
    );
    assert_eq!(
        abilities_resp["ack"], true,
        "fixture ability projection advertise must ack: {abilities_resp}"
    );
    assert_eq!(
        abilities_resp["count"], 1,
        "fixture must publish exactly one executable echo route: {abilities_resp}"
    );
}

fn start_daemon_at(
    socket_path: &Path,
    trust_path: &Path,
    daemon_ura: String,
    ledger: Arc<easynet_axon::invocation::InvocationLedger>,
) -> TestDaemon {
    let agents = easynet_cli::daemon::persistence::agent_registry::load_agents()
        .expect("load seeded agents.json");
    assert!(
        agents.agents.contains_key("testbot"),
        "fixture must load the seeded agent through the production path"
    );

    let trust_anchor =
        Arc::new(RealmTrustAnchor::try_load_strict(trust_path).expect("load test trust anchor"));
    let shared_trust_anchor = SharedTrustAnchor::new(Arc::clone(&trust_anchor));

    // ONE ledger handle (owned by the HOME fixture) for both halves:
    // the service WRITES unary records through it, and the registry's
    // `invocation.history.*` / `invocation.trace.get` abilities READ
    // through the same Arc — redb is single-writer; a second open of
    // the same file would split the truth (and a daemon restart would
    // deadlock on the lock).
    let runtime = LocalRuntime::new();
    // Production sink wiring (`configure_local_runtime`, same as
    // daemon boot): Axon-routed unary invokes persist their terminal
    // records through the SDK-canonical `LedgerSink` — one writer.
    easynet_cli::daemon::axon_bridge::runtime_factory::configure_local_runtime(
        &runtime,
        Some(Arc::new(RealmTrustAnchorKeyResolver::new(
            shared_trust_anchor,
        ))),
        Some(Arc::clone(&ledger)),
    );
    let presence = Arc::new(PresenceRegistry::new());
    let advertised_agents = Arc::new(
        easynet_cli::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore::new(),
    );
    let ability_catalog = Arc::new(
        easynet_cli::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new(),
    );
    let discover_resolver = Arc::new(
        easynet_cli::daemon::ability::builtins::agents::discover::LocalDirectoryDiscoverFederationResolver::new(
            Arc::clone(&presence),
            Arc::clone(&advertised_agents),
            Arc::clone(&ability_catalog),
            Some(daemon_ura.clone()),
        ),
    );
    let services =
        RegistryBuildServices::fresh().with_discover_federation_resolver(discover_resolver);
    let mut config = RegistryBuildConfig::new(services, &agents);
    let hot_agent_registrar_cell: Arc<
        easynet_cli::daemon::ability::builtins::agents::lifecycle::SharedHotRegistrarCell,
    > = Arc::new(std::sync::OnceLock::new());
    config.hot_agent_registrar_cell = Arc::clone(&hot_agent_registrar_cell);
    config.local_runtime = Some(Arc::clone(&runtime));
    config.invocation_ledger = Some(Arc::clone(&ledger));
    let built_registry = build_registry_with_services_result(config);
    let catalog = Arc::clone(&built_registry.catalog);
    if let Some(hot_registrar) = hot_agent_registrar_cell.get() {
        hot_registrar.set_runtime(Arc::clone(&runtime));
    }
    if let Some(device_registrar) = built_registry.device_registrar_cell.get() {
        device_registrar
            .set_control_plane_catalog(Arc::downgrade(&catalog))
            .expect("wire device registrar control-plane catalog");
        device_registrar
            .set_runtime(Arc::clone(&runtime))
            .expect("wire device registrar runtime");
        let replay = futures::executor::block_on(device_registrar.replay_from_store());
        assert!(
            !replay.runtime_not_ready && !replay.store_unreadable,
            "device ability replay must be readable in fixture: {replay:?}"
        );
    }

    let admission = AdmissionFacade::new(trust_anchor, Some(daemon_ura.clone()));
    let service = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_directory_read_models(advertised_agents, ability_catalog)
        .with_local_runtime(runtime)
        .with_invocation_ledger(ledger);

    // A restart binds the same UDS path again; remove the stale node.
    let _ = std::fs::remove_file(socket_path);

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let socket = socket_path.to_path_buf();
    let daemon_ura_for_presence = daemon_ura.clone();
    let thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("daemon test runtime");
        rt.block_on(async move {
            // Self-presence seed, mirroring the boot path
            // (`seed_device_mode_self_presence`): device-local route
            // resolution answers "am I online?" against this
            // registry; self-targeted invokes run inline through the
            // LocalRuntime — the drain task only swallows defensive
            // out-of-path frames.
            let (noop_tx, mut noop_rx) = tokio::sync::mpsc::channel(
                easynet_cli::daemon::invocation::bidi::state::presence::DISPATCH_CHANNEL_CAPACITY,
            );
            tokio::spawn(async move { while noop_rx.recv().await.is_some() {} });
            presence.insert(daemon_ura_for_presence, noop_tx);

            let listener = UnixListener::bind(&socket).expect("bind test UDS");
            let incoming = UnixListenerStream::new(listener);
            let _ = Server::builder()
                .add_service(InvocationServer::new(service))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });
    });

    // Wait for the listener to accept before the client probes it.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if std::os::unix::net::UnixStream::connect(socket_path).is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "daemon UDS never came up at {}",
            socket_path.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    seed_hosted_agent_projection(
        socket_path,
        &daemon_ura,
        &daemon_ura,
        &easynet_cli::core::ura::agent_ura("cli", "local", "testbot"),
    );

    TestDaemon {
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    }
}
