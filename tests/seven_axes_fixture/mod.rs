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

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, RwLock};

use std::time::{Duration, Instant};

use axon_sdk::pb::axon::v1::invocation_client::InvocationClient;
use axon_sdk::pb::axon::v1::invocation_server::InvocationServer;
use easynet_cli::daemon::ability::catalog::{
    build_registry_with_services_result, RegistryBuildConfig, RegistryBuildServices,
};
use easynet_cli::daemon::identity::self_identity::{
    CanonicalSigner, KeyringClient, SelfIdentity, SelfIdentityError,
};
use easynet_cli::daemon::invocation::admission::admission_facade::AdmissionFacade;
use easynet_cli::daemon::invocation::bidi::state::presence::PresenceRegistry;
use easynet_cli::daemon::invocation::dispatch::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::daemon::invocation::dispatch::invocation_wire::{
    InvocationDerivationPolicy, ProtoEnvelope,
};
use easynet_cli::daemon::persistence::config::{self, RuntimeKind, RuntimeState};
use easynet_cli::daemon::trust::anchor::RealmTrustAnchor;
use easynet_cli::daemon::trust::cell::SharedTrustAnchor;
use easynet_cli::daemon::trust::key_resolver::RealmTrustAnchorKeyResolver;
use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::{json, Value};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::oneshot;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server, Uri};

pub const TESTBOT_ECHO_DESCRIPTOR_VERSION: &str = "2.3.0";

#[derive(Clone, Debug, PartialEq)]
struct FixtureDescriptorEntry {
    descriptor_ref: String,
    descriptor: easynet_cli::daemon::ability::descriptors::AbilityDescriptor,
}

type DescriptorRefIndex = Arc<RwLock<BTreeMap<(String, String), FixtureDescriptorEntry>>>;
const FIXTURE_HOSTED_AGENT_GENERATION: u64 = 1;

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
    /// Realm hub URA used for federation publication authority in this fixture.
    pub hub_ura: String,
    /// Canonical URA minted for the seeded agent (RFC-005 §1.4).
    #[allow(dead_code)]
    pub testbot_ura: String,
    /// One ledger handle per HOME, shared across daemon restarts —
    /// redb is single-writer, and restart-persistence tests bounce
    /// the daemon while the database (like all state) belongs to the
    /// HOME, not to the process.
    pub ledger: Arc<axon_sdk::invocation::InvocationLedger>,
    /// HOME-owned transport-boundary attempt ledger path. Production boot
    /// refuses to serve Invocation without this audit sidecar; the fixture
    /// constructs the service directly, so it must pass the same boot fact.
    attempt_ledger_path: PathBuf,
    /// Canonical URA of the second seeded agent (`zlearner` — named
    /// to sort AFTER `testbot` so ladder resolution and self-tier
    /// attribution in the W1 assertions stay put). Publishes no
    /// abilities of its own; exists to learn (W3 T3.3).
    #[allow(dead_code)]
    pub zlearner_ura: String,
    descriptor_refs: DescriptorRefIndex,
}

/// A running in-process daemon; dropping it shuts the server down
/// and joins the thread.
pub struct TestDaemon {
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

struct TestKeyring {
    child: Child,
}

struct FixtureCanonicalSigner {
    owner_ura: String,
    signing_owner_ura: String,
    public_key: VerifyingKey,
    provider: Arc<dyn SelfIdentity>,
}

#[async_trait::async_trait]
impl CanonicalSigner for FixtureCanonicalSigner {
    fn owner_ura(&self) -> &str {
        &self.owner_ura
    }

    async fn sign_canonical(&self, canonical_bytes: &[u8]) -> Result<Signature, SelfIdentityError> {
        let provider = Arc::clone(&self.provider);
        let signing_owner_ura = self.signing_owner_ura.clone();
        let public_key = self.public_key;
        let canonical_bytes = canonical_bytes.to_vec();
        tokio::task::spawn_blocking(move || {
            provider.sign_bound(&signing_owner_ura, &public_key, &canonical_bytes)
        })
        .await
        .map_err(|error| {
            SelfIdentityError::Transport(format!(
                "fixture key-service signing worker terminated unexpectedly: {error}"
            ))
        })?
    }

    fn signing_public_key(&self) -> Result<VerifyingKey, SelfIdentityError> {
        Ok(self.public_key)
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
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_test_keyring(primary_self: String) -> (TestKeyring, String) {
    let socket_path = easynet_cli::daemon::keyring::default_socket_path();
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent).expect("keyring socket parent");
    }
    let _ = std::fs::remove_file(&socket_path);

    let child = Command::new(env!("CARGO_BIN_EXE_easynet-keyring"))
        .env("EASYNET_KEYRING_SOCKET_PATH", &socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn test key service");
    let client = KeyringClient::new(&socket_path);
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.health().is_err() {
        assert!(
            Instant::now() < deadline,
            "test key service did not become ready"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let public_key = client
        .ensure(&primary_self)
        .expect("ensure test runtime identity");
    let trusted_public_key_b64 = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(public_key.to_bytes())
    };
    (TestKeyring { child }, trusted_public_key_b64)
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
            "schema_version = \"1\"\nname = \"testbot\"\nruntime = \"claude-code\"\n",
        )
        .expect("write minimal agent.toml");

        let learner_root = home.path().join("agents/zlearner");
        std::fs::create_dir_all(learner_root.join("abilities")).expect("learner root");
        std::fs::write(
            learner_root.join("agent.toml"),
            "schema_version = \"1\"\nname = \"zlearner\"\nruntime = \"claude-code\"\n",
        )
        .expect("write learner agent.toml");

        let abilities_dir = agent_root.join("abilities");
        std::fs::create_dir_all(&abilities_dir).expect("abilities dir");
        std::fs::write(
            abilities_dir.join("weather-probe.ability.toml"),
            "schema_version = \"1\"\n\
             name = \"weather-probe\"\n\
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
                    "default/testbot": {
                        "schema_version": 2,
                        "agent_type": "claude-code",
                        "root_path": agent_root,
                    },
                    "default/zlearner": {
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
        let hub_ura = easynet_cli::core::ura::hub_ura("cli");
        let testbot_ura = easynet_cli::core::ura::agent_ura("cli", "local", "testbot");
        let zlearner_ura = easynet_cli::core::ura::agent_ura("cli", "local", "zlearner");
        let descriptor_refs = Arc::new(RwLock::new(BTreeMap::new()));
        let (keyring, trusted_public_key_b64) = start_test_keyring(loopback_caller.clone());
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
        let mut f = std::fs::File::create(&trust_path).expect("create trust toml");
        write!(
            f,
            r#"
[[trusted_agent]]
agent_ura = "{loopback_caller}"
public_key_b64 = "{trusted_public_key_b64}"
role = "device"
added_at_unix_ms = 0

[[trusted_principal_owner]]
principal_ura = "{loopback_caller}"
owner_user_id = "user-local"
owner_ura = "easynet:///r/cli/user/user-local"
owner_username = "local"
added_at_unix_ms = 0
"#
        )
        .expect("write trust toml");
        drop(f);

        let ledger_path = easynet_cli::daemon::persistence::daemon_config::default_ledger_dir()
            .join("invocations.redb");
        let attempt_ledger_path =
            easynet_cli::daemon::persistence::daemon_config::default_ledger_dir()
                .join("invocation-attempts.jsonl");
        let ledger = Arc::new(
            axon_sdk::invocation::InvocationLedger::open(&ledger_path).expect("open test ledger"),
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
        let user_ura = easynet_cli::core::ura::user_ura("cli", "user-local");
        easynet_cli::daemon::identity::self_identity::ensure_user_runtime_signing_identity(
            &KeyringClient::default_path(),
            &user_ura,
        )
        .unwrap_or_else(|error| panic!("seed fixture paired User signer `{user_ura}`: {error}"));
        easynet_cli::daemon::control::discovery::write(
            &easynet_cli::daemon::control::discovery::default_path(),
            &easynet_cli::daemon::control::discovery::ControlDiscovery {
                socket_path: None,
                pipe_name: None,
                invocation_endpoint: Some(socket_path.clone()),
                daemon_identity: Some(easynet_cli::daemon::control::discovery::DaemonIdentity {
                    mode: "device".to_string(),
                    realm: "cli".to_string(),
                    node_id: Some("local".to_string()),
                }),
                pid: std::process::id(),
                daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                supported_ipc_versions:
                    easynet_cli::daemon::control::discovery::IpcVersionRange::single(
                        easynet_cli::daemon::control::discovery::IPC_VERSION_V1,
                    ),
                capability_flags: vec![
                    easynet_cli::daemon::control::discovery::flags::BOOT_STATUS.to_string(),
                    easynet_cli::daemon::control::discovery::flags::CONTROL_DIAGNOSTICS.to_string(),
                    easynet_cli::daemon::control::discovery::flags::PAIRED_USER_RUNTIME_SIGNER
                        .to_string(),
                ],
                pages_port: None,
            },
        )
        .expect("write production-shaped daemon Ready discovery");
        SevenAxesHome {
            home,
            _keyring: keyring,
            socket_path,
            trust_path,
            loopback_caller,
            hub_ura,
            testbot_ura,
            zlearner_ura,
            ledger,
            attempt_ledger_path,
            descriptor_refs,
        }
    }

    /// Canonical URA of the seeded `testbot.weather-probe` ability —
    /// minted through the same owner projection the ladder uses.
    #[allow(dead_code)]
    pub fn source_descriptor_ura(&self) -> String {
        easynet_cli::core::ura::owner_ability_ura(&self.testbot_ura, "weather-probe")
            .expect("mint source descriptor URA")
    }

    #[allow(dead_code)]
    pub fn testbot_echo_descriptor_ref(&self) -> String {
        require_descriptor_ref(&self.descriptor_refs, &self.testbot_ura, "echo")
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
            require_descriptor_ref(&self.descriptor_refs, &self.testbot_ura, "echo");
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
        let history_descriptor_ref = require_descriptor_ref(
            &self.descriptor_refs,
            &self.loopback_caller,
            "invocation.record.get",
        );
        for _ in 0..10 {
            let (history, _, _) = invoke_daemon_ability(
                &self.socket_path,
                &self.loopback_caller,
                &self.loopback_caller,
                &self.loopback_caller,
                "invocation.record.get",
                history_descriptor_ref.as_str(),
                json!({ "request_id": request_id }),
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

    /// Invoke one Device-owned system ability through the same descriptor-ref
    /// gRPC surface as production callers.
    #[allow(dead_code)]
    pub fn invoke_device_system_ability(&self, function_name: &str, args: Value) -> Value {
        let descriptor_ref =
            require_descriptor_ref(&self.descriptor_refs, &self.loopback_caller, function_name);
        let (value, _, _) = invoke_daemon_ability(
            &self.socket_path,
            &self.loopback_caller,
            &self.loopback_caller,
            &self.loopback_caller,
            function_name,
            descriptor_ref.as_str(),
            args,
        );
        value
    }

    /// Publish one hosted Agent identity and its complete ability projection
    /// through the live daemon descriptors. The generation is shared by both
    /// writes because they describe one durable Agent incarnation.
    #[allow(dead_code)]
    pub fn advertise_hosted_agent_projection(
        &self,
        host_device_ura: &str,
        owner_ura: &str,
        projection_revision: u64,
        ability_summaries: Vec<Value>,
    ) {
        advertise_hosted_agent_projection(
            &self.socket_path,
            &self.loopback_caller,
            &self.hub_ura,
            host_device_ura,
            owner_ura,
            FIXTURE_HOSTED_AGENT_GENERATION,
            projection_revision,
            ability_summaries,
            &self.descriptor_refs,
        );
    }

    /// Invoke `federation.join` through the real daemon gRPC surface with the
    /// protocol's provisional genesis envelope. This intentionally bypasses the
    /// signed descriptor-ref helper because the join caller is not a member yet.
    #[allow(dead_code)]
    pub fn invoke_federation_join_with_principal_proof(
        &self,
        membership_ura: &str,
        principal_ura: &str,
        proof_kind: &str,
        proof_ref: &str,
    ) -> Value {
        invoke_federation_join_with_principal_proof(
            &self.socket_path,
            &self.hub_ura,
            membership_ura,
            principal_ura,
            proof_kind,
            proof_ref,
            &self.descriptor_refs,
        )
    }

    /// Boot the real daemon surface against this HOME: full system
    /// catalogue (agents read from disk through the production
    /// loader) materialised into a `LocalRuntime`, served by
    /// `DaemonInvocationService` over tonic — the same stack
    /// `easynet runtime start` wires, minus hub/plugin/session
    /// concerns these tests do not exercise. Restartable: state lives
    /// in the seeded HOME, not in the daemon.
    #[allow(dead_code)]
    pub fn start_daemon(&self) -> TestDaemon {
        start_daemon_at(
            &self.socket_path,
            &self.trust_path,
            self.loopback_caller.clone(),
            self.hub_ura.clone(),
            vec![self.testbot_ura.clone(), self.zlearner_ura.clone()],
            Arc::clone(&self.descriptor_refs),
            Arc::clone(&self.ledger),
            self.attempt_ledger_path.clone(),
            true,
        )
    }

    /// Boot the same real daemon surface without publishing the hosted-agent
    /// projection. System-provider E2E tests use this when they only need
    /// daemon-owned abilities and should not depend on federation publication
    /// policy.
    #[allow(dead_code)]
    pub fn start_daemon_without_hosted_projection(&self) -> TestDaemon {
        start_daemon_at(
            &self.socket_path,
            &self.trust_path,
            self.loopback_caller.clone(),
            self.hub_ura.clone(),
            vec![self.testbot_ura.clone(), self.zlearner_ura.clone()],
            Arc::clone(&self.descriptor_refs),
            Arc::clone(&self.ledger),
            self.attempt_ledger_path.clone(),
            false,
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
    Option<axon_sdk::pb::axon::v1::InvocationReceipt>,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    rt.block_on(async {
        let mut client = InvocationClient::new(connect_to_daemon(socket_path).await);
        let arguments = serde_json::to_vec(&args).expect("encode daemon invoke args");
        let signer = KeyringClient::default_path();
        let envelope = ProtoEnvelope::from_target(
            caller_ura,
            callee_ura,
            subject_ura,
            InvocationDerivationPolicy::FreshRoot,
        )
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
        .unwrap_or_else(|status| {
            panic!(
                "daemon invoke must succeed: function_name={function_name} callee={callee_ura} subject={subject_ura} descriptor_ref={descriptor_ability_ref}: {status:?}"
            )
        })
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

fn receipt_proof_facts_value(receipt: &axon_sdk::pb::axon::v1::InvocationReceipt) -> Value {
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

fn invoke_federation_join_with_principal_proof(
    socket_path: &Path,
    hub_ura: &str,
    membership_ura: &str,
    principal_ura: &str,
    proof_kind: &str,
    proof_ref: &str,
    descriptor_refs: &DescriptorRefIndex,
) -> Value {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    rt.block_on(async {
        let mut client = InvocationClient::new(connect_to_daemon(socket_path).await);
        let key_service = Arc::new(KeyringClient::default_path());
        let public_key = key_service
            .ensure(membership_ura)
            .expect("ensure joined member identity inside key service");
        let public_key_bytes = public_key.to_bytes();
        let public_key_hex = hex::encode(public_key_bytes);
        let realm = easynet_cli::core::ura::parse_ura(hub_ura)
            .expect("fixture hub URA parses")
            .realm;
        let arguments = serde_json::to_vec(&json!({
            "membership_ura": membership_ura,
            "realm": realm,
            "public_key_hex": &public_key_hex,
            "principal_enrollment": {
                "principal_ura": principal_ura,
                "proof": {
                    "kind": proof_kind,
                    "reference": proof_ref
                }
            }
        }))
        .expect("encode federation.join args");
        let descriptor_ref = require_descriptor_ref(descriptor_refs, hub_ura, "federation.join");
        let signer = FixtureCanonicalSigner {
            owner_ura: membership_ura.to_string(),
            signing_owner_ura: membership_ura.to_string(),
            public_key,
            provider: key_service,
        };
        let request = ProtoEnvelope::federation_join_bootstrap(
            hub_ura,
            membership_ura,
            InvocationDerivationPolicy::FreshRoot,
        )
        .expect("valid federation.join bootstrap envelope")
        .signed_descriptor_ref_invoke_request_with_signer(
            easynet_cli::daemon::ability::conformance::ABILITY_FEDERATION_JOIN,
            descriptor_ref,
            arguments,
            &signer,
        )
        .await
        .expect("valid federation.join invoke request");
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            client.invoke(tonic::Request::new(request)),
        )
        .await
        .expect("federation.join must not hang")
        .expect("federation.join must succeed")
        .into_inner();
        serde_json::from_slice(&response.result).expect("decode federation.join result")
    })
}

fn live_rpc_descriptor_refs(
    catalog: &easynet_cli::daemon::ability::dispatch::AxonAbilityCatalog,
) -> BTreeMap<(String, String), FixtureDescriptorEntry> {
    let mut refs = BTreeMap::new();
    for row in catalog
        .authority_ability_catalog_snapshot()
        .into_iter()
        .filter(|row| row.descriptor.call_mode() == easynet_cli::daemon::ability::CallMode::Rpc)
    {
        let ability_ura = row
            .descriptor
            .canonical_ability_ura()
            .expect("live descriptor must have a canonical Ability URA");
        let selector = easynet_cli::core::ura::AbilitySelector::parse(&ability_ura)
            .expect("live descriptor Ability URA must expose its owner");
        let descriptor_ref = axon_sdk::invocation::canonical_ability_descriptor_ref(&format!(
            "{}@{}#{}!{}",
            ability_ura,
            row.descriptor.version,
            hex::encode(row.descriptor.descriptor_hash_bytes()),
            row.descriptor.admission_action().as_str(),
        ))
        .expect("live descriptor must form a canonical descriptor ref");
        let public_name = row.descriptor.public_name();
        let entry = FixtureDescriptorEntry {
            descriptor_ref,
            descriptor: row.descriptor,
        };
        for name in [row.name.clone(), public_name, ability_ura.clone()] {
            let key = (selector.owner_ura().to_string(), name);
            if let Some(existing) = refs.insert(key.clone(), entry.clone()) {
                assert_eq!(
                    existing, entry,
                    "live RPC descriptor index is ambiguous for {key:?}"
                );
            }
        }
    }
    refs
}

fn require_descriptor_ref(
    descriptor_refs: &DescriptorRefIndex,
    callee_ura: &str,
    function_name: &str,
) -> String {
    descriptor_refs
        .read()
        .expect("seven-axes descriptor-ref index lock")
        .get(&(callee_ura.to_string(), function_name.to_string()))
        .unwrap_or_else(|| {
            panic!(
                "live catalog has no RPC descriptor ref for `{function_name}` under `{callee_ura}`"
            )
        })
        .descriptor_ref
        .clone()
}

fn require_projection_summary(
    descriptor_refs: &DescriptorRefIndex,
    callee_ura: &str,
    function_name: &str,
) -> Value {
    let descriptor = descriptor_refs
        .read()
        .expect("seven-axes descriptor-ref index lock")
        .get(&(callee_ura.to_string(), function_name.to_string()))
        .unwrap_or_else(|| {
            panic!(
                "live catalog has no projection summary for `{function_name}` under `{callee_ura}`"
            )
        })
        .descriptor
        .clone();
    easynet_cli::daemon::federation::read_model::owner_projection::
        canonical_summary_values_from_descriptors(callee_ura, &[descriptor])
        .expect("selected hosted-agent descriptor must project canonically")
        .into_iter()
        .next()
        .expect("one selected descriptor must produce one projection summary")
}

fn seed_hosted_agent_projection(
    socket_path: &Path,
    caller_ura: &str,
    hub_ura: &str,
    host_device_ura: &str,
    agent_ura: &str,
    descriptor_refs: &DescriptorRefIndex,
) {
    let ability_summary = require_projection_summary(descriptor_refs, agent_ura, "echo");
    advertise_hosted_agent_projection(
        socket_path,
        caller_ura,
        hub_ura,
        host_device_ura,
        agent_ura,
        FIXTURE_HOSTED_AGENT_GENERATION,
        1,
        vec![ability_summary],
        descriptor_refs,
    );
}

#[allow(clippy::too_many_arguments)]
fn advertise_hosted_agent_projection(
    socket_path: &Path,
    caller_ura: &str,
    hub_ura: &str,
    host_device_ura: &str,
    agent_ura: &str,
    generation: u64,
    projection_revision: u64,
    ability_summaries: Vec<Value>,
    descriptor_refs: &DescriptorRefIndex,
) {
    let advertise_agent_descriptor_ref =
        require_descriptor_ref(descriptor_refs, hub_ura, "federation.advertise_agent");
    let (agent_resp, _, _) = invoke_daemon_ability(
        socket_path,
        caller_ura,
        hub_ura,
        agent_ura,
        "federation.advertise_agent",
        advertise_agent_descriptor_ref.as_str(),
        json!({
            "agent_ura": agent_ura,
            "generation": generation,
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

    let projection_digest =
        easynet_cli::daemon::federation::read_model::owner_projection::
            canonical_projection_digest_from_values(
                agent_ura,
                host_device_ura,
                generation,
                projection_revision,
                0,
                &ability_summaries,
            )
            .expect("fixture projection summaries must be canonical");
    let advertise_abilities_descriptor_ref =
        require_descriptor_ref(descriptor_refs, hub_ura, "federation.advertise_abilities");
    let (abilities_resp, _, _) = invoke_daemon_ability(
        socket_path,
        caller_ura,
        hub_ura,
        agent_ura,
        "federation.advertise_abilities",
        advertise_abilities_descriptor_ref.as_str(),
        json!({
            "agent_ura": agent_ura,
            "owner_ura": agent_ura,
            "host_device_ura": host_device_ura,
            "generation": generation,
            "projection_revision": projection_revision,
            "projection_digest": projection_digest,
            "lease_expires_unix_ms": 0,
            "ability_summaries": ability_summaries
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
    hub_ura: String,
    hosted_agent_uras: Vec<String>,
    descriptor_refs: DescriptorRefIndex,
    ledger: Arc<axon_sdk::invocation::InvocationLedger>,
    attempt_ledger_path: PathBuf,
    publish_hosted_projection: bool,
) -> TestDaemon {
    let agents = easynet_cli::daemon::persistence::agent_registry::load_agents()
        .expect("load seeded agents.json");
    assert!(
        agents.agents.contains_key("default/testbot"),
        "fixture must load the seeded agent through the production path"
    );

    let trust_anchor =
        Arc::new(RealmTrustAnchor::try_load_strict(trust_path).expect("load test trust anchor"));
    let shared_trust_anchor = SharedTrustAnchor::new(Arc::clone(&trust_anchor));
    let authority_context =
        easynet_cli::daemon::ability::dispatch::AbilityAuthorityContext::for_combined_authority_roots_with_hosted_agents(
            daemon_ura.clone(),
            hosted_agent_uras,
        )
        .expect("seven-axes fixture authority roots must be canonical");
    let hosted_inventory = authority_context
        .hosted_agent_signing_inventory()
        .expect("combined fixture authority must own hosted-Agent signing inventory");
    let key_service = KeyringClient::default_path();
    easynet_cli::daemon::identity::self_identity::ensure_daemon_local_system_identity(&key_service)
        .expect("seed fixture daemon-local signing owner");
    for owner_ura in [daemon_ura.as_str(), hub_ura.as_str()] {
        key_service
            .ensure(owner_ura)
            .unwrap_or_else(|error| panic!("seed fixture signing owner `{owner_ura}`: {error}"));
    }
    let receipt_authority_config =
        easynet_cli::daemon::axon_bridge::runtime_factory::ProductionReceiptAuthorityConfig::new([
            daemon_ura.clone(),
            hub_ura.clone(),
        ])
        .with_hosted_agent_inventory(daemon_ura.clone(), hosted_inventory);

    // ONE ledger handle (owned by the HOME fixture) for both halves:
    // the service WRITES unary records through it, and the registry's
    // `invocation.history.*` / `invocation.trace.get` abilities READ
    // through the same Arc — redb is single-writer; a second open of
    // the same file would split the truth (and a daemon restart would
    // deadlock on the lock).
    let daemon_runtime =
        easynet_cli::daemon::axon_bridge::runtime_factory::build_production_local_runtime(
            receipt_authority_config,
            Arc::new(RealmTrustAnchorKeyResolver::new(
                shared_trust_anchor.clone(),
            )),
        )
        .expect("build seven-axes owner-bound daemon runtime");
    let runtime = daemon_runtime.runtime();
    easynet_cli::daemon::axon_bridge::runtime_factory::install_ledger_sink(
        &runtime,
        Some(Arc::clone(&ledger)),
    );
    let presence = Arc::new(PresenceRegistry::new());
    let advertised_agents = Arc::new(
        easynet_cli::daemon::federation::read_model::advertised_agents::AdvertisedAgentStore::new(),
    );
    let ability_catalog = Arc::new(
        easynet_cli::daemon::federation::read_model::ability_catalog::AbilityCatalogStore::new(),
    );
    let local_catalog_cell = Arc::new(std::sync::OnceLock::new());
    let discover_resolver = Arc::new(
        easynet_cli::daemon::ability::builtins::agents::discover::LocalDirectoryDiscoverFederationResolver::new(
            Arc::clone(&presence),
            Arc::clone(&advertised_agents),
            Arc::clone(&ability_catalog),
            Arc::clone(&local_catalog_cell),
        ),
    );
    let services =
        RegistryBuildServices::fresh().with_discover_federation_resolver(discover_resolver);
    let mut config =
        RegistryBuildConfig::new_with_authority_context(services, &agents, authority_context);
    let hot_agent_registrar_cell: Arc<
        easynet_cli::daemon::ability::builtins::agents::lifecycle::SharedHotRegistrarCell,
    > = Arc::new(std::sync::OnceLock::new());
    config.hot_agent_registrar_cell = Arc::clone(&hot_agent_registrar_cell);
    config.local_runtime = Some(Arc::clone(&runtime));
    config.invocation_ledger = Some(Arc::clone(&ledger));
    let built_registry =
        build_registry_with_services_result(config).expect("assemble seven-axes fixture catalog");
    let catalog = Arc::clone(&built_registry.catalog);
    *descriptor_refs
        .write()
        .expect("seven-axes descriptor-ref index lock") =
        live_rpc_descriptor_refs(catalog.as_ref());
    local_catalog_cell
        .set(Arc::clone(&catalog))
        .expect("fixture local catalog cell has one writer");
    hot_agent_registrar_cell
        .get()
        .expect("catalog assembly wires hot-Agent registrar")
        .require_ready()
        .expect("hot-Agent registrar ready after catalog assembly");
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

    let admission =
        AdmissionFacade::with_trust_anchor_cell(shared_trust_anchor.clone(), Some(hub_ura.clone()))
            .with_ability_catalog(Arc::clone(&catalog));
    daemon_runtime
        .bind_derived_invocation_admission(catalog.as_ref(), admission.clone())
        .expect("bind seven-axes derived Invocation product admission");
    let service = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_directory_read_models(advertised_agents, ability_catalog)
        .with_local_ability_catalog(Arc::clone(&catalog))
        .with_daemon_runtime(daemon_runtime)
        .with_invocation_ledger(ledger)
        .with_invocation_attempt_ledger_path(attempt_ledger_path)
        .expect("wire seven-axes invocation attempt audit ledger")
        .with_register_pubkey("cli", trust_path, shared_trust_anchor);
    futures::executor::block_on(
        service.register_daemon_unary_routes_for_owners(&[daemon_ura.clone(), hub_ura.clone()]),
    )
    .expect("register seven-axes daemon exact unary routes for both authority roots");
    futures::executor::block_on(service.register_daemon_stream_routes(&hub_ura))
        .expect("register seven-axes daemon exact stream routes");

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
            presence
                .insert(daemon_ura_for_presence, noop_tx)
                .expect("canonical presence key");

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
    if publish_hosted_projection {
        seed_hosted_agent_projection(
            socket_path,
            &daemon_ura,
            &hub_ura,
            &daemon_ura,
            &easynet_cli::core::ura::agent_ura("cli", "local", "testbot"),
            &descriptor_refs,
        );
    }

    TestDaemon {
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    }
}
