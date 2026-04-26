// EasyNet CLI
// ===========
//
// File: src/cli/start.rs
// Description: `easynet start` — spawns a local Axon runtime, joins Hub, registers node,
//              and maintains heartbeat. Supports both foreground and background modes.
//
// Lifecycle:
// - Ensures no runtime is already running (checks ~/.easynet/runtime.json).
// - Uses `ServerConfig` from the Axon SDK to auto-start a local runtime on a free port.
// - If credentials exist (from `easynet join`), registers the node and starts heartbeat.
// - In foreground mode: blocks on Ctrl-C, then gracefully deregisters + shuts down.
// - In background mode: forks a heartbeat daemon process, detaches the runtime.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::mem::ManuallyDrop;
use std::sync::OnceLock;

use anyhow::Context;
use clap::Args;
use easynet_axon::server::ServerConfig;

use super::heartbeat::{self, HeartbeatOutcome};
use crate::persistence::config;
use crate::support::{self, net, output, shutdown::ShutdownSignal};

/// Register a Ctrl-C handler that triggers `shutdown`. Safe to call multiple
/// times — only the first call installs the handler; subsequent calls are
/// no-ops.
///
/// `ctrlc::set_handler` only fails when an OS-level handler is already
/// installed for SIGINT, which `OnceLock::get_or_init` already prevents
/// for our own callers. The remaining failure mode is "another library in
/// the process has installed a handler first" — surface that as a warning
/// so the operator knows Ctrl-C will not gracefully shut us down.
fn install_ctrlc_handler(shutdown: &ShutdownSignal) {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    let s = shutdown.clone();
    INSTALLED.get_or_init(|| {
        if let Err(e) = ctrlc::set_handler(move || {
            eprintln!("\nShutting down...");
            s.trigger();
        }) {
            eprintln!(
                "[easynet warn] could not install Ctrl-C handler ({e}); \
                 the process will not respond to Ctrl-C with a graceful shutdown"
            );
        }
    });
}

#[derive(Debug, Args)]
pub struct StartArgs {
    /// Hub endpoint (e.g. `axon://easynet.run:50051`)
    #[arg(long, default_value = config::DEFAULT_HUB)]
    pub hub: String,
    /// Tenant ID
    #[arg(long, default_value = config::DEFAULT_TENANT)]
    pub tenant: String,
    /// Human-readable label for this device
    #[arg(long)]
    pub label: Option<String>,
    /// Pre-shared join token
    #[arg(long)]
    pub token: Option<String>,
    /// Run as a Hub (accept federation joins instead of joining a Hub)
    #[arg(long)]
    pub as_hub: bool,
    /// Bind address for Hub mode (default: 0.0.0.0:50051)
    #[arg(long, default_value = config::DEFAULT_BIND)]
    pub bind: String,
    /// Run in foreground (block until Ctrl-C)
    #[arg(long)]
    pub foreground: bool,
    /// Disable Hub-level MCP server on stdio (foreground only)
    #[arg(long)]
    pub no_mcp: bool,
    /// Allow insecure (plaintext) connections to the Hub
    #[arg(long)]
    pub insecure: bool,
}

impl StartArgs {
    /// Construct args suitable for `easynet connect` (foreground, minimal flags).
    /// Note: hub/tenant are placeholders — `run_device_mode()` overrides them from credentials.
    pub fn for_connect(no_mcp: bool) -> Self {
        Self {
            hub: config::DEFAULT_HUB.into(),
            tenant: config::DEFAULT_TENANT.into(),
            label: None,
            token: None,
            as_hub: false,
            bind: config::DEFAULT_BIND.into(),
            foreground: true,
            no_mcp,
            insecure: false,
        }
    }
}

/// Result of `verify_credential` — tracks whether the Hub was reachable.
#[derive(Debug, PartialEq, Eq)]
enum CredentialCheck {
    Valid,
    NetworkUnavailable,
    Revoked(String),
}

pub fn run(args: StartArgs) -> anyhow::Result<()> {
    if let Ok(state) = config::load() {
        if state.pid.is_some_and(net::is_pid_alive) {
            anyhow::bail!(
                "runtime already running (run `easynet stop` first, or remove ~/.easynet/runtime.json)"
            );
        }
        output::info("Detected stale runtime state (process not running). Cleaning up...");
        config::remove().ok();
    }

    if args.as_hub {
        return run_as_hub(&args);
    }

    run_device_mode(&args)
}

// ── Device mode ─────────────────────────────────────────────────────────────

fn run_device_mode(args: &StartArgs) -> anyhow::Result<()> {
    // Load and verify credentials (from `easynet join`).
    let (creds, credential_verified) = load_and_verify_credentials()?;
    let settings = config::load_device_settings();

    // Configure env vars consumed by the Axon SDK. Must happen before any thread spawn.
    apply_env_patch(&env_patch_for_device(&creds, &settings));

    // Credentials take precedence over CLI args for hub/tenant.
    let hub = creds.hub_endpoint.clone();
    let tenant = creds.tenant_id.clone();
    if args.hub != config::DEFAULT_HUB && args.hub != hub {
        output::warn(&format!(
            "--hub {} ignored; using {} from credentials. Run `easynet reset` to un-pair first.",
            args.hub, hub
        ));
    }
    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    let label = args.label.clone().unwrap_or_else(|| creds.node_id.clone());

    let srv = start_runtime_for_device(
        &hub,
        &tenant,
        &label,
        &creds.node_id,
        args.token.as_deref(),
        args.insecure,
    )?;
    let endpoint = srv.url().to_string();
    let pid = net::discover_pid_from_endpoint(&endpoint);

    let state = config::RuntimeState {
        endpoint: endpoint.clone(),
        pid,
        hub: Some(hub.clone()),
        tenant: Some(tenant.clone()),
        label: Some(label.clone()),
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        credential_verified: Some(credential_verified),
    };
    config::save(&state)?;

    output::success(&format!("Axon runtime started on {endpoint}"));
    output::success(&format!("Joined {hub} as {label}"));
    output::detail("tenant", &tenant);
    if let Some(pid) = pid {
        output::detail("pid", &pid.to_string());
    }
    if !credential_verified {
        output::warn("credential not verified (Hub unreachable during startup)");
    }

    // Try connecting the bridge for node registration.
    let bridge = match support::connect_bridge_to(&endpoint) {
        Ok(b) => b,
        Err(e) => {
            output::warn(&format!("bridge connect failed: {e}"));
            output::info("Node registration skipped — runtime is still running.");
            output::info(
                "Hint: set EASYNET_DENDRITE_BRIDGE_LIB to the dendrite bridge library path.",
            );
            return run_foreground_or_detach(srv, args.foreground);
        }
    };

    // Build a2a.* labels so this node is discoverable as an A2A agent
    // across the Axon federation. The full encoding contract lives in
    // `registry::a2a_labels::build` — note it returns `Option<HashMap>` so
    // "no agents registered" maps cleanly to "omit labels on the wire"
    // (vs `Some({})` which would publish an empty map). If the agents
    // file is missing or malformed, we also omit labels rather than
    // registering with partial data.
    let labels = match crate::registry::agents::load_agents() {
        Ok(registry) => crate::registry::a2a_labels::build(&registry, &hostname),
        Err(e) => {
            output::warn(&format!(
                "failed to load ~/.easynet/agents.json ({e}); registering without a2a.* labels"
            ));
            None
        }
    };

    let reg_resp = bridge
        .register_node_with_options(
            &creds.tenant_id,
            &creds.node_id,
            &hostname,
            easynet_axon::dendrite_bridge::RegisterNodeOptions { labels, role: None },
        )
        .context("register node")?;

    let heartbeat_ms = reg_resp
        .get("heartbeat_interval_ms")
        .and_then(serde_json::Value::as_u64)
        .filter(|&v| v > 0)
        .unwrap_or(heartbeat::DEFAULT_HEARTBEAT_MS);

    output::success(&format!(
        "Node registered: {} (heartbeat every {}ms)",
        creds.node_id, heartbeat_ms
    ));

    // Re-publish every registered agent's manifests to the freshly
    // started axon-runtime. The runtime-local tool registry is
    // in-memory by design (see EasyNet-Axon
    // core/runtime-rs/src/state/runtime_tool.rs); a runtime restart
    // drops every register-tool entry. Without this re-publish step
    // the EasyNet frontend's Abilities catalog goes empty on every
    // `easynet runtime stop && start` even though the agent registry
    // (~/.easynet/agents.json) and the on-disk manifests are
    // unchanged.
    //
    // Best-effort: per-agent failures are logged but never abort
    // runtime startup. The runtime is up and serving — federation
    // discovery just degrades to "this node has no abilities yet"
    // until a successful re-publish lands. A subsequent
    // `easynet agent add` against a healthy runtime fixes it.
    republish_all_agents_best_effort(&bridge, &creds);

    // Publish device-level system abilities (system.ping,
    // system.skill.list, system.session.*, system.permission.*, etc.)
    // to the same axon-runtime. Per-agent abilities and system
    // abilities use the same `register_runtime_local_mcp_tool` slot;
    // without this call the system abilities live only in the daemon's
    // in-memory LocalAbilityRegistry — a Hub-mediated CallMcpTool can
    // never reach them, and the EasyNet frontend's Skills page
    // silently shows zero installs because backend listInstalledLogic
    // degrades a missing-ability response to an empty list.
    republish_system_abilities_best_effort(&bridge, &creds);

    if args.foreground {
        run_foreground_with_heartbeat(srv, &bridge, &creds, &endpoint, heartbeat_ms, args.no_mcp)
    } else {
        run_background_with_heartbeat(srv, &endpoint, heartbeat_ms)
    }
}

/// Re-publish every registered agent's ability manifests to the freshly
/// started axon-runtime. The runtime-local tool registry is in-memory
/// by design (see EasyNet-Axon `core/runtime-rs/src/state/runtime_tool.rs`),
/// so a runtime restart drops every register-tool entry. Without this
/// step the EasyNet frontend's Abilities catalog goes empty after every
/// `easynet runtime stop && start` even though the agent registry and
/// the on-disk manifests are unchanged.
///
/// Best-effort. We log per-agent outcomes but do not propagate errors —
/// runtime startup must complete so other surfaces (heartbeat,
/// federation join) keep working. A registry that is briefly missing
/// abilities is recoverable; a runtime that refused to start because
/// one agent's directory is unreadable is not.
fn republish_all_agents_best_effort(
    bridge: &easynet_axon::dendrite_bridge::DendriteBridge,
    creds: &config::Credentials,
) {
    let registry = match crate::registry::agents::load_agents() {
        Ok(r) => r,
        Err(e) => {
            output::warn(&format!(
                "could not load agent registry for re-publish: {e}"
            ));
            return;
        }
    };
    if registry.agents.is_empty() {
        // Brand-new install with no agents yet — nothing to re-publish.
        return;
    }
    let socket_path = config::state_dir().join("control.sock");
    let dispatch_endpoint = format!("ipc://{}", socket_path.display());

    let mut total_ok = 0usize;
    let mut total_published = 0usize;
    for (agent_name, entry) in &registry.agents {
        let Some(root) = entry.root_path.as_ref() else {
            continue;
        };
        let directory = match crate::runtime::directory::AgentDirectory::open(root) {
            Ok(d) => d,
            Err(e) => {
                output::warn(&format!(
                    "re-publish {agent_name}: open agent directory failed: {e}"
                ));
                continue;
            }
        };
        let outcomes = crate::runtime::publish::publish_agent_to_local_runtime(
            bridge,
            &creds.tenant_id,
            &creds.node_id,
            agent_name,
            &directory,
            &dispatch_endpoint,
        );
        for o in &outcomes {
            total_published += 1;
            match &o.result {
                Ok(_) => total_ok += 1,
                Err(msg) => {
                    output::warn(&format!("re-publish {} failed: {msg}", o.tool_name));
                }
            }
        }
    }
    if total_published > 0 {
        output::detail(
            "abilities",
            &format!(
                "{}/{} re-published — visible in EasyNet Abilities",
                total_ok, total_published
            ),
        );
    }
}

/// Re-publish every `system.*` ability to the freshly started
/// axon-runtime. Mirrors `republish_all_agents_best_effort` but for
/// the device-level handlers that have no on-disk manifest — they
/// live in the daemon's `LocalAbilityRegistry` and would otherwise be
/// invisible to the Hub-mediated `CallMcpTool` path.
///
/// Same best-effort policy: per-tool failures are warned but never
/// abort startup. The runtime is up regardless; the federation
/// discovery surface for system abilities just degrades until a
/// healthy daemon restart catches up.
fn republish_system_abilities_best_effort(
    bridge: &easynet_axon::dendrite_bridge::DendriteBridge,
    creds: &config::Credentials,
) {
    let socket_path = config::state_dir().join("control.sock");
    let dispatch_endpoint = format!("ipc://{}", socket_path.display());

    let outcomes = crate::runtime::publish::publish_system_abilities_to_local_runtime(
        bridge,
        &creds.tenant_id,
        &creds.node_id,
        &dispatch_endpoint,
    );
    let total_published = outcomes.len();
    let mut total_ok = 0usize;
    for o in &outcomes {
        match &o.result {
            Ok(_) => total_ok += 1,
            Err(msg) => {
                output::warn(&format!("re-publish {} failed: {msg}", o.tool_name));
            }
        }
    }
    if total_published > 0 {
        output::detail(
            "system abilities",
            &format!(
                "{}/{} re-published — Skills + system.* visible to backend",
                total_ok, total_published
            ),
        );
    }
}

/// Load credentials and verify against Hub. Returns error on revoked/missing credentials.
fn load_and_verify_credentials() -> anyhow::Result<(config::Credentials, bool)> {
    load_and_verify_credentials_with(verify_credential)
}

fn load_and_verify_credentials_with<F>(verify: F) -> anyhow::Result<(config::Credentials, bool)>
where
    F: Fn(&config::Credentials) -> CredentialCheck,
{
    let Ok(creds) = config::load_credentials() else {
        output::info("No credentials found.");
        output::info("Visit https://easynet.run or your Hub to create a pairing token,");
        output::info("then run `easynet join <token>` to pair this device.");
        output::info("If you're running a Hub, use `easynet start --as-hub` instead.");
        anyhow::bail!("no credentials — cannot start device agent");
    };

    match verify(&creds) {
        CredentialCheck::Valid => Ok((creds, true)),
        CredentialCheck::NetworkUnavailable => Ok((creds, false)),
        CredentialCheck::Revoked(msg) => {
            eprintln!("{} {msg}", console::style("✗").red().bold());
            eprintln!("  node_id: {}", creds.node_id);
            eprintln!("  hub:     {}", creds.hub_endpoint);
            eprintln!("  Credential revoked or device removed from account.");
            eprintln!();
            config::delete_credentials().ok();
            eprintln!("  Stale credentials cleaned up.");
            eprintln!("  Visit https://easynet.run or your Hub to create a new pairing token,");
            eprintln!("  then run `easynet join <token>`.");
            anyhow::bail!("credential revoked");
        }
    }
}

/// Build and start the Axon runtime for device mode.
fn start_runtime_for_device(
    hub: &str,
    tenant: &str,
    label: &str,
    runtime_id: &str,
    join_token: Option<&str>,
    insecure: bool,
) -> anyhow::Result<easynet_axon::server::ServerHandle> {
    // Local runtime does not need mTLS — no TLS cert provisioning flow exists yet.
    // The `insecure` CLI flag controls Hub gRPC transport, not local runtime mTLS.
    let _ = insecure;
    let mut cfg = ServerConfig::default()
        .hub(hub)
        .hub_tenant(tenant)
        .hub_label(label)
        .hub_runtime_id(runtime_id);
    if let Some(t) = join_token {
        cfg = cfg.hub_join_token(t);
    }
    cfg.start().context("start runtime")
}

/// Run in foreground with heartbeat + optional MCP server.
fn run_foreground_with_heartbeat(
    srv: easynet_axon::server::ServerHandle,
    bridge: &easynet_axon::dendrite_bridge::DendriteBridge,
    creds: &config::Credentials,
    endpoint: &str,
    heartbeat_ms: u64,
    no_mcp: bool,
) -> anyhow::Result<()> {
    if !no_mcp {
        let ep = endpoint.to_string();
        let t = creds.tenant_id.clone();
        // Snapshot the agent registry once and build the adapter in the
        // calling thread. Registry reads are I/O; doing them here (before
        // the spawn) surfaces any filesystem error in the parent's error
        // stream rather than silently in the background MCP thread. If
        // the registry is missing or malformed, advertise zero abilities
        // rather than refuse to start the MCP server — this matches the
        // a2a-label path's degraded-but-running policy (see `a2a_labels`
        // module doc).
        let agent_abilities = match crate::registry::agents::load_agents() {
            Ok(registry) => {
                // Build a LocalAbilityRegistry with chat handlers so
                // AgentDispatchAdapter::handle routes through the same
                // unified handler the daemon uses. Without this the MCP
                // adapter would have nothing to dispatch to and surface
                // an Internal "boot ordering" error on every chat call.
                let mut local = crate::runtime::ability_dispatch::LocalAbilityRegistry::new();
                crate::runtime::system::chat_ability::register(
                    &mut local,
                    &registry,
                    std::sync::Arc::new(Vec::new()),
                );
                crate::facade::mcp::agent_dispatch::AgentDispatchAdapter::build(
                    &registry,
                    std::sync::Arc::new(local),
                    creds.tenant_id.clone(),
                )
            }
            Err(e) => {
                output::warn(&format!(
                    "failed to load ~/.easynet/agents.json ({e}); \
                     MCP server will advertise no agent abilities"
                ));
                crate::facade::mcp::agent_dispatch::AgentDispatchAdapter::empty(
                    creds.tenant_id.clone(),
                )
            }
        };
        std::thread::spawn(move || {
            let kit = crate::facade::mcp::provider::HubMcpProvider::new(ep, t)
                .with_agent_abilities(agent_abilities);
            let server = easynet_axon::mcp::StdioMcpServer::new(kit)
                .with_server_name("easynet-device")
                .with_server_version(env!("CARGO_PKG_VERSION"));
            if let Err(e) = server.run(std::io::stdin().lock(), &mut std::io::stdout()) {
                eprintln!("mcp server exited: {e}");
            }
        });
        output::success("MCP server started on stdio");
    }

    let shutdown = ShutdownSignal::new();
    install_ctrlc_handler(&shutdown);

    // Foreground mode uses the bridge the parent process already opened
    // and registered against — no reconnect layer here. If the transport
    // drops under foreground `easynet start`, the operator is watching and
    // can restart. The daemon path takes the reconnecting transport; see
    // `heartbeat::run_daemon`.
    let mut transport = heartbeat::DirectBridge::new(bridge);
    let outcome = heartbeat::heartbeat_loop(
        &mut transport,
        &creds.tenant_id,
        &creds.node_id,
        heartbeat_ms,
        &shutdown,
    );

    if let Err(e) = bridge.deregister_node(&creds.tenant_id, &creds.node_id, outcome.reason()) {
        // Failing to deregister leaves a phantom node in the Hub view
        // until the next stale-node sweep. The local shutdown still
        // proceeds, but the operator should know the federation is in
        // an inconsistent state.
        output::warn(&format!(
            "deregister_node failed for {} ({}): {e}",
            creds.node_id,
            outcome.reason(),
        ));
    }
    drop(srv);
    config::remove()?;
    match outcome {
        HeartbeatOutcome::FailuresExhausted => {
            output::success("Axon runtime stopped (heartbeat lost after consecutive failures)");
        }
        HeartbeatOutcome::HubRejected => {
            output::success("Axon runtime stopped (Hub rejected this member)");
        }
        HeartbeatOutcome::NodeRejected => {
            // Device was administratively removed — clean up local credentials
            // so the user cannot reconnect with a revoked identity.
            config::delete_credentials().ok();
            output::success("Axon runtime stopped (device removed by admin)");
            output::step("Local credentials have been removed.");
            output::step(
                "To reconnect, create a new pairing token and run `easynet join <token>`.",
            );
        }
        HeartbeatOutcome::Shutdown => {
            output::success("Axon runtime stopped");
        }
    }
    Ok(())
}

/// Detach runtime to background and spawn heartbeat daemon.
fn run_background_with_heartbeat(
    srv: easynet_axon::server::ServerHandle,
    endpoint: &str,
    heartbeat_ms: u64,
) -> anyhow::Result<()> {
    heartbeat::spawn_daemon(endpoint, heartbeat_ms)?;
    // Intentionally leak the handle so the runtime keeps running after this process exits.
    let _ = ManuallyDrop::new(srv);
    output::info("Runtime running in background. Use `easynet stop` to stop.");
    Ok(())
}

/// Either block in foreground until Ctrl-C, or detach to background.
/// Used when bridge connect fails but runtime is still running.
fn run_foreground_or_detach(
    srv: easynet_axon::server::ServerHandle,
    foreground: bool,
) -> anyhow::Result<()> {
    if foreground {
        let shutdown = ShutdownSignal::new();
        install_ctrlc_handler(&shutdown);
        output::info("Running in foreground (Ctrl-C to stop)...");
        shutdown.wait();
        drop(srv);
        config::remove()?;
        output::success("Axon runtime stopped");
    } else {
        let _ = ManuallyDrop::new(srv);
        output::info("Runtime running in background. Use `easynet stop` to stop.");
    }
    Ok(())
}

// ── Hub mode ────────────────────────────────────────────────────────────────

fn run_as_hub(args: &StartArgs) -> anyhow::Result<()> {
    // Clear env vars to prevent SDK from connecting to an existing endpoint or hub.
    apply_env_patch(&env_patch_for_hub());

    let tenant = args.tenant.clone();

    let mut cfg = ServerConfig::default()
        .endpoint(&args.bind)
        .insecure(args.insecure);
    if let Some(ref t) = args.token {
        cfg = cfg.hub_join_token(t);
    }

    let srv = cfg.start().context("start hub")?;

    let endpoint = srv.url().to_string();
    let pid = net::discover_pid_from_endpoint(&endpoint);

    let state = config::RuntimeState {
        endpoint: endpoint.clone(),
        pid,
        hub: None,
        tenant: Some(tenant),
        label: Some("hub".to_string()),
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        credential_verified: None, // Not applicable in hub mode.
    };
    config::save(&state)?;

    output::success(&format!("Hub started on {endpoint}"));
    if let Some(pid) = pid {
        output::detail("pid", &pid.to_string());
    }
    output::step("Devices can join with: easynet start --hub axon://<this-ip>:50051");

    if args.foreground {
        let shutdown = ShutdownSignal::new();
        install_ctrlc_handler(&shutdown);
        output::info("Running in foreground (Ctrl-C to stop)...");
        shutdown.wait();
        drop(srv);
        config::remove()?;
        output::success("Hub stopped");
    } else {
        let _ = ManuallyDrop::new(srv);
        output::info("Hub running in background. Use `easynet stop` to stop.");
    }
    Ok(())
}

// ── Environment setup (pre-thread) ─────────────────────────────────────────

/// Environment variables the Axon SDK needs, grouped for `init_env_vars()`.
struct EnvPatch {
    sets: Vec<(&'static str, String)>,
    removes: Vec<&'static str>,
}

/// Apply environment variable patch.
///
/// # Safety
/// Must be called on the main thread before any `std::thread::spawn` or `ServerConfig::start`.
/// `assert_single_threaded()` guards against accidental misuse.
fn apply_env_patch(patch: &EnvPatch) {
    assert_single_threaded();
    // SAFETY: assert_single_threaded() verified no other threads exist.
    unsafe {
        for (k, v) in &patch.sets {
            std::env::set_var(k, v);
        }
        for k in &patch.removes {
            std::env::remove_var(k);
        }
    }
}

/// Panic if other threads are running. Guards `apply_env_patch` against UB.
///
/// On Linux, counts `/proc/self/task` entries.
/// On macOS, uses `pthread_num_threads_np()` from libc.
/// On other platforms, this is a no-op — call-site discipline required.
fn assert_single_threaded() {
    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = std::fs::read_dir("/proc/self/task") {
            let count = entries.count();
            assert!(
                count <= 1,
                "apply_env_patch called with {count} threads running — must be single-threaded"
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::mem::MaybeUninit;
        extern "C" {
            fn task_threads(
                target_task: libc::c_uint,
                act_list: *mut *mut libc::c_uint,
                act_list_cnt: *mut libc::c_uint,
            ) -> libc::c_int;
            fn mach_task_self() -> libc::c_uint;
            fn vm_deallocate(target_task: libc::c_uint, address: usize, size: usize)
                -> libc::c_int;
        }
        let mut thread_list = MaybeUninit::<*mut libc::c_uint>::uninit();
        let mut thread_count = MaybeUninit::<libc::c_uint>::uninit();
        // SAFETY: Mach kernel call to enumerate threads in the current task.
        let kr = unsafe {
            task_threads(
                mach_task_self(),
                thread_list.as_mut_ptr(),
                thread_count.as_mut_ptr(),
            )
        };
        if kr == 0 {
            let count = unsafe { thread_count.assume_init() };
            let list = unsafe { thread_list.assume_init() };
            // Free the Mach-allocated thread list.
            unsafe {
                vm_deallocate(
                    mach_task_self(),
                    list as usize,
                    count as usize * std::mem::size_of::<libc::c_uint>(),
                );
            }
            assert!(
                count <= 1,
                "apply_env_patch called with {count} threads running — must be single-threaded"
            );
        }
    }
}

fn env_patch_for_device(
    creds: &config::Credentials,
    settings: &config::DeviceSettings,
) -> EnvPatch {
    let mut sets = Vec::new();
    if creds.deploy_signature.is_empty() {
        // Allow ephemeral/placeholder deploy signatures in dev mode when no real
        // signature is available. Must be set here (single-threaded init) because
        // env mutation from handler threads is UB.
        sets.push(("AXON_ALLOW_PLACEHOLDER_DEPLOY_SIGNATURE", "1".into()));
    } else {
        sets.push((
            "AXON_DEPLOY_SIGNATURE_BASE64",
            creds.deploy_signature.clone(),
        ));
    }
    let exec_enabled = std::env::var("EASYNET_SESSION_BRIDGE_EXEC_ENABLED")
        .map(|v| v == "1")
        .unwrap_or(settings.session_bridge_exec_enabled);
    if exec_enabled {
        sets.push(("EASYNET_SESSION_BRIDGE_EXEC_ENABLED", "1".into()));
    }
    EnvPatch {
        sets,
        removes: vec!["EASYNET_AXON_ENDPOINT"],
    }
}

fn env_patch_for_hub() -> EnvPatch {
    EnvPatch {
        sets: Vec::new(),
        removes: vec!["EASYNET_AXON_ENDPOINT", "AXON_HUB"],
    }
}

// ── Credential verification ─────────────────────────────────────────────────

fn classify_credential_status_code(code: u16) -> CredentialCheck {
    if (400..500).contains(&code) {
        match code {
            // 404 = endpoint not found (Hub version mismatch), 429 = rate limited — transient.
            404 | 429 => {
                output::warn(&format!("Hub returned HTTP {code}, continuing anyway"));
                CredentialCheck::NetworkUnavailable
            }
            // 401/403 = credential explicitly rejected.
            // Other 4xx (400, 422, etc.) = client-side error, likely a bad credential.
            _ => CredentialCheck::Revoked(format!("credential rejected by Hub (HTTP {code})")),
        }
    } else if code >= 500 {
        output::warn(&format!(
            "Hub returned server error (HTTP {code}), continuing anyway"
        ));
        CredentialCheck::NetworkUnavailable
    } else {
        output::warn(&format!(
            "unexpected Hub response (HTTP {code}) during credential check, continuing anyway"
        ));
        CredentialCheck::NetworkUnavailable
    }
}

/// Verify device credentials with the backend API.
fn verify_credential(creds: &config::Credentials) -> CredentialCheck {
    let base = creds.api_base();
    let url = format!("{base}/api/v1/devices/verify-credential");

    let resp = ureq::post(&url)
        .timeout(std::time::Duration::from_secs(5))
        .send_json(serde_json::json!({
            "node_id": creds.node_id,
            "credential_token": creds.credential_token,
        }));

    match resp {
        Ok(r) if (200..300).contains(&r.status()) => CredentialCheck::Valid,
        Ok(r) => {
            let body = r.into_string().unwrap_or_default();
            CredentialCheck::Revoked(format!("credential rejected: {body}"))
        }
        Err(ureq::Error::Status(code, _)) => classify_credential_status_code(code),
        Err(e) => {
            output::warn(&format!(
                "could not verify credential ({e}), continuing anyway"
            ));
            CredentialCheck::NetworkUnavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::cli::test_support::HomeGuard;

    fn test_creds() -> config::Credentials {
        config::Credentials {
            node_id: "node-test".into(),
            credential_token: "token-test".into(),
            hub_endpoint: "axon://easynet.run:50051".into(),
            tenant_id: "tenant-test".into(),
            deploy_signature: "sig".into(),
            hub_api_base: Some("https://api.example.com".into()),
        }
    }

    #[test]
    fn classify_credential_status_code_maps_revoked_vs_transient() {
        assert_eq!(
            classify_credential_status_code(401),
            CredentialCheck::Revoked("credential rejected by Hub (HTTP 401)".into())
        );
        assert_eq!(
            classify_credential_status_code(404),
            CredentialCheck::NetworkUnavailable
        );
        assert_eq!(
            classify_credential_status_code(429),
            CredentialCheck::NetworkUnavailable
        );
        assert_eq!(
            classify_credential_status_code(503),
            CredentialCheck::NetworkUnavailable
        );
    }

    #[test]
    fn load_and_verify_credentials_returns_verified_when_valid() {
        let _g = HomeGuard::new();
        let creds = test_creds();
        config::save_credentials(&creds).expect("save test credentials");

        let (loaded, verified) = load_and_verify_credentials_with(|_| CredentialCheck::Valid)
            .expect("valid credentials must pass");
        assert!(verified);
        assert_eq!(loaded.node_id, "node-test");
        assert!(
            config::load_credentials().is_ok(),
            "credentials should stay on disk"
        );
    }

    #[test]
    fn load_and_verify_credentials_keeps_unverified_credentials_when_hub_unavailable() {
        let _g = HomeGuard::new();
        let creds = test_creds();
        config::save_credentials(&creds).expect("save test credentials");

        let (loaded, verified) =
            load_and_verify_credentials_with(|_| CredentialCheck::NetworkUnavailable)
                .expect("must continue on transient outage");
        assert!(!verified);
        assert_eq!(loaded.node_id, "node-test");
        assert_eq!(loaded.api_base(), "https://api.example.com");
        assert!(
            config::load_credentials().is_ok(),
            "credentials should remain on transient outage"
        );
    }

    #[test]
    fn load_and_verify_credentials_deletes_revoked_credentials() {
        let _g = HomeGuard::new();
        let creds = test_creds();
        config::save_credentials(&creds).expect("save test credentials");

        let err =
            load_and_verify_credentials_with(|_| CredentialCheck::Revoked("bad token".into()))
                .expect_err("revoked credentials must error");
        assert!(err.to_string().contains("credential revoked"));
        assert!(
            config::load_credentials().is_err(),
            "credentials must be deleted after revocation"
        );
    }

    #[test]
    fn load_and_verify_credentials_errors_when_missing() {
        let _g = HomeGuard::new();
        let err = load_and_verify_credentials().expect_err("missing credentials must fail");
        assert!(err.to_string().contains("no credentials"));
    }
}
