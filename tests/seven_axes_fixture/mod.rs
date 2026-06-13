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

use easynet_axon::invocation::LocalRuntime;
use easynet_axon::pb::axon::v1::invocation_server::InvocationServer;
use easynet_cli::runtime::agents::{
    build_registry_with_services, RegistryBuildConfig, RegistryBuildServices,
};
use easynet_cli::services::invocation_transport::admission_facade::AdmissionFacade;
use easynet_cli::services::invocation_transport::daemon_invocation_service::DaemonInvocationService;
use easynet_cli::services::presence_registry::PresenceRegistry;
use easynet_cli::services::realm_trust_anchor::RealmTrustAnchor;
use tokio::net::UnixListener;
use tokio::sync::oneshot;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::Server;

/// A seeded HOME: env pointed, product files written. Keep it alive
/// for the whole test — dropping it deletes the tempdir.
pub struct SevenAxesHome {
    /// RAII keep-alive for the tempdir — never read, must not drop
    /// before the test ends. (Shared across test binaries that use
    /// different subsets of this struct; hence the allows.)
    #[allow(dead_code)]
    pub home: tempfile::TempDir,
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

        let loopback_caller = easynet_cli::ura::device_ura("cli", "local");
        let testbot_ura = easynet_cli::ura::agent_ura("cli", "local", "testbot");
        let zlearner_ura = easynet_cli::ura::agent_ura("cli", "local", "zlearner");
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
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "device"
added_at_unix_ms = 0
"#
        )
        .expect("write trust toml");
        drop(f);

        let ledger_path =
            easynet_cli::persistence::daemon_config::default_ledger_dir().join("invocations.redb");
        let ledger = Arc::new(
            easynet_axon::invocation::InvocationLedger::open(&ledger_path)
                .expect("open test ledger"),
        );

        SevenAxesHome {
            home,
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
    pub fn taught_ability_ura(&self) -> String {
        easynet_cli::ura::owner_ability_ura(&self.testbot_ura, "weather-probe")
            .expect("mint taught ability URA")
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

fn start_daemon_at(
    socket_path: &Path,
    trust_path: &Path,
    daemon_ura: String,
    ledger: Arc<easynet_axon::invocation::InvocationLedger>,
) -> TestDaemon {
    let agents = easynet_cli::registry::agents::load_agents().expect("load seeded agents.json");
    assert!(
        agents.agents.contains_key("testbot"),
        "fixture must load the seeded agent through the production path"
    );

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
    easynet_cli::runtime::axon_bridge::runtime_factory::configure_local_runtime(
        &runtime,
        None,
        Some(Arc::clone(&ledger)),
    );
    let mut config = RegistryBuildConfig::new(RegistryBuildServices::fresh(), &agents);
    config.local_runtime = Some(Arc::clone(&runtime));
    config.invocation_ledger = Some(Arc::clone(&ledger));
    let _catalog = build_registry_with_services(config);

    let trust_anchor =
        RealmTrustAnchor::try_load_strict(trust_path).expect("load test trust anchor");
    let admission = AdmissionFacade::new(Arc::new(trust_anchor), Some(daemon_ura.clone()));
    let presence = Arc::new(PresenceRegistry::new());
    let service = DaemonInvocationService::new(Arc::clone(&presence), admission)
        .with_local_runtime(runtime)
        .with_invocation_ledger(ledger);

    // A restart binds the same UDS path again; remove the stale node.
    let _ = std::fs::remove_file(socket_path);

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let socket = socket_path.to_path_buf();
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
                easynet_cli::services::presence_registry::DISPATCH_CHANNEL_CAPACITY,
            );
            tokio::spawn(async move { while noop_rx.recv().await.is_some() {} });
            presence.insert(daemon_ura, noop_tx);

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

    TestDaemon {
        shutdown: Some(shutdown_tx),
        thread: Some(thread),
    }
}
