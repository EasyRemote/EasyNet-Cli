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

use crate::persistence::config;
use crate::support::{net, output, shutdown::ShutdownSignal};


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
    /// Hub endpoint (e.g. axon://easynet.run:50051).
    // No backticks: some terminals (iTerm2, Warp) auto-highlight
    // backtick-fenced text with an inverted background, producing
    // a visual "white block" in `--help`. The example URL is clear
    // without them.
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

    /// Construct args for the auto-start hop at the tail of
    /// `easynet join`. Background mode so the join command returns
    /// the user to their shell once the daemon is `Ready`, matching
    /// the historical "join then exit" UX. Hub/tenant are
    /// placeholders — `run_device_mode()` overrides them from the
    /// credentials we just wrote.
    pub fn for_join_autostart() -> Self {
        Self {
            hub: config::DEFAULT_HUB.into(),
            tenant: config::DEFAULT_TENANT.into(),
            label: None,
            token: None,
            as_hub: false,
            bind: config::DEFAULT_BIND.into(),
            foreground: false,
            no_mcp: false,
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
                "runtime already running (run 'easynet stop' first, or remove ~/.easynet/runtime.json)"
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

    // Credentials take precedence over CLI args for hub/tenant.
    let hub = creds.hub_endpoint.clone();
    let tenant = creds.tenant_id.clone();
    if args.hub != config::DEFAULT_HUB && args.hub != hub {
        output::warn(&format!(
            "--hub {} ignored; using {} from credentials. Run 'easynet reset' to un-pair first.",
            args.hub, hub
        ));
    }
    let label = args.label.clone().unwrap_or_else(|| creds.node_id.clone());
    let _ = (args.token.as_deref(), args.insecure);
    // EASYNET_PAGES_PORT is parsed by the daemon — it is the only
    // process that needs to validate the value and decide a default.
    // CLI just peeks at it for the progress UI's "fell back from N"
    // hint, treating any parse failure as "no hint available".
    let pages_start_hint = std::env::var("EASYNET_PAGES_PORT")
        .ok()
        .and_then(|raw| raw.parse::<u16>().ok())
        .filter(|p| *p > 0);

    crate::persistence::daemon_config::ensure_minimal_device_config(&creds)
        .context("ensure daemon-config.toml for device mode")?;
    let _ = super::federation_wire::auto_wire_self_realm_trust_from_credentials(&creds);

    let mut daemon_handle = spawn_easynet_daemon(&creds.node_id);
    let control_socket = crate::services::control::transport::default_socket_path();
    let boot = super::start_boot_watcher::wait_for_daemon_boot(
        &control_socket,
        daemon_handle.as_mut(),
        super::start_boot_watcher::BootContext {
            pages_start_port: pages_start_hint,
        },
    )?;
    // The daemon is the authoritative source for the bound port: it
    // either reported it via PortChosen, or wrote it to control.json
    // when the listener bound. The CLI never has a meaningful
    // fallback here — if neither is set, surface that as an error
    // (the listener boot stage would already have emitted Failed
    // and `wait_for_daemon_boot` would have returned before we got
    // here, so this branch is defence-in-depth only).
    let pages_listener_port = super::start_boot_watcher::final_pages_port(boot.pages_port)
        .ok_or_else(|| anyhow::anyhow!("daemon reported Ready without binding a pages port"))?;
    let sockets = DaemonSockets {
        control_socket,
        grpc_socket: crate::support::local_daemon_grpc::resolve_socket_path(),
    };
    let pid = discover_existing_daemon_pid();
    let endpoint = sockets.grpc_socket.display().to_string();

    let state = config::RuntimeState {
        endpoint: endpoint.clone(),
        runtime_kind: config::RuntimeKind::DaemonOnly,
        pid,
        hub: Some(hub.clone()),
        tenant: Some(tenant.clone()),
        label: Some(label.clone()),
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        credential_verified: Some(credential_verified),
    };
    config::save(&state)?;

    output::success("EasyNet daemon started");
    let control_socket = sockets.control_socket.display().to_string();
    let hub_api = creds.api_base();
    let pages_url_root = format!(
        "http://<project>.{user}.pages.localhost:{pages_listener_port}/",
        user = creds
            .username
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("<user>")
    );
    let pid_display = pid.map(|pid| pid.to_string());
    let mut rows = vec![
        ("daemon_socket", endpoint.as_str()),
        ("control_socket", control_socket.as_str()),
        ("hub_session", hub.as_str()),
        ("hub_api", hub_api.as_str()),
        ("realm", tenant.as_str()),
        ("pages_url_root", pages_url_root.as_str()),
    ];
    if let Some(ref pid) = pid_display {
        rows.push(("pid", pid.as_str()));
    }
    output::kv_section(&rows);

    // Welcome line — surface the human identity the daemon is now
    // operating under. `username` is optional pre-v4.1.4 so this
    // block only renders when we actually have a slug to address
    // the user by; otherwise the kv_section above already covers
    // the device-level info.
    if let Some(username) = creds
        .username
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        let user_ura = crate::ura::user_ura(&tenant, username);
        eprintln!();
        eprintln!(
            "{} {}",
            console::style("Welcome,").cyan().bold(),
            console::style(username).cyan().bold(),
        );
        eprintln!("  {}", console::style(user_ura).dim());
    }

    if !credential_verified {
        output::warn(&format!(
            "credential not verified: Hub API unreachable at {hub_api}"
        ));
        output::step(
            "Pairing is still present, but startup could not confirm it with the Hub API.",
        );
        output::step(
            "For Docker/local hubs, re-pair with --hub http://127.0.0.1:8080 or pass --hub-api.",
        );
    }

    if args.foreground {
        run_foreground_with_daemon(&creds, args.no_mcp)
    } else {
        output::info("Daemon running in background. Use 'easynet stop' to stop.");
        Ok(())
    }
}

/// Re-publish every Agent's directory entry + descriptor list to
/// the realm hub via `federation.advertise_*`. Replaces the two
/// pre-RFC-001 helpers (`republish_all_agents_best_effort` +
/// `republish_system_abilities_best_effort`) with the single
/// federation-shaped path.
///
/// Steps:
///   1. Build a `BootstrapPlan` from credentials + the loaded
///      AgentRegistry. Today every host enables consent; policy /
///      mcp default off until the [profiles] config wiring lands.
///   2. Hand the plan to `runtime::publish::republish_abilities_via_advertise`.
///   3. Render outcomes — one warn per failed advertise; one
///      info line summarising successes.
///
/// Spawn the easynet-daemon child process so its UDS listeners are
/// up before `runtime.register_local_tool` advertises their paths to
/// the runtime. Returns the spawned `Child` so the caller (or its
/// drop on shutdown) can terminate it; v1 leaves orphaning to the
/// process supervisor (operators usually run this in a session that
/// gets SIGTERMed on Ctrl-C, which propagates to the child).
///
/// Best-effort: a spawn failure is logged but never aborts startup.
/// In that degraded state, the runtime accepts register calls and
/// the registered endpoint is recorded, but every actual Invoke
/// falling back to `runtime_local_tools` will fail at the UDS
/// connect step until the operator manually starts the daemon.
/// Probe whether an `easynet-daemon` is accepting on the canonical
/// `~/.easynet/control.sock`. Returns `true` only if the path
/// exists AND a connect succeeds — a stale socket file (left after
/// a daemon crash) returns `false` because the connect refuses.
///
/// Unix-only; on non-Unix targets the daemon's IPC plane uses
/// Named Pipes, and the spawn-twice race that motivates this
/// probe is a Unix-specific failure mode.
fn probe_daemon_alive() -> bool {
    crate::support::local_daemon_grpc::probe_accepting(
        &crate::services::control::transport::default_socket_path(),
    )
}

#[derive(Debug, Clone)]
struct DaemonSockets {
    control_socket: std::path::PathBuf,
    grpc_socket: std::path::PathBuf,
}

fn discover_existing_daemon_pid() -> Option<u32> {
    let pid_path = crate::persistence::config::easynet_daemon_pid_path();
    if let Some(pid) = std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|pid| net::is_pid_alive(*pid))
    {
        return Some(pid);
    }

    #[cfg(windows)]
    {
        return None;
    }

    #[cfg(not(windows))]
    {
        let output = std::process::Command::new("pgrep")
            .args(["-f", "easynet-daemon"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
            .find(|pid| *pid != std::process::id() && net::is_pid_alive(*pid))
    }
}

fn spawn_easynet_daemon(node_id: &str) -> Option<std::process::Child> {
    // Liveness probe: if a daemon is already accepting on the
    // canonical control.sock, do not spawn a second one. A second
    // spawn races against the first for the runtime-dispatch
    // socket bind (one-process), and the loser exits silently —
    // leaving a "half-broken" setup that swallows dispatches. The
    // pidfile alone does not catch this case (an operator who
    // hand-spawned the daemon for a test never wrote one). Probe
    // the actual UDS so any healthy responder, however spawned,
    // counts as "already running".
    if probe_daemon_alive() {
        let sock = crate::services::control::transport::default_socket_path();
        output::info(&format!(
            "easynet-daemon already accepting on {} — leaving it in place",
            sock.display()
        ));
        return None;
    }

    // Resolve the daemon binary: env override > sibling of current
    // exe > PATH. The env override exists because the test stack
    // uses an out-of-tree build; production installers drop both
    // binaries into /usr/local/bin so the sibling lookup wins.
    let bin_path = std::env::var_os("EASYNET_DAEMON_BIN")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("easynet-daemon")))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("easynet-daemon"));

    let mut cmd = std::process::Command::new(&bin_path);
    cmd.env("EASYNET_NODE_ID", node_id);
    // EASYNET_PAGES_PORT is inherited from this process's environment
    // (Command's default). The daemon parses it; the CLI does not.
    // Daemon's IPC + dispatch logs go to a known file so operators
    // can tail without guessing where stderr landed.
    let log_dir = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join(".easynet")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("easynet-daemon.log");
    if let Ok(f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        if let Ok(f2) = f.try_clone() {
            cmd.stdout(std::process::Stdio::from(f2));
        }
        cmd.stderr(std::process::Stdio::from(f));
    }
    match cmd.spawn() {
        Ok(child) => {
            // Record the daemon's pid so `easynet runtime stop` can
            // signal it deterministically. Best-effort: a write
            // failure means stop will fall back to the (correct)
            // pgrep-style sweep, but we want the pidfile to be the
            // authoritative path so a second `runtime start` doesn't
            // race with a still-alive ghost daemon (load-bearing —
            // the runtime-dispatch socket bind is one-process and a
            // second daemon's responder exits silently, leaving a
            // half-broken setup that swallows chat connections).
            let pid = child.id();
            let pid_path = crate::persistence::config::easynet_daemon_pid_path();
            if let Some(parent) = pid_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&pid_path, pid.to_string()) {
                output::warn(&format!(
                    "could not write daemon pidfile {}: {e}; \
                     `runtime stop` will fall back to pgrep",
                    pid_path.display()
                ));
            }
            output::detail_dim(
                "daemon",
                &format!(
                    "spawned {} (pid {pid}; log: {})",
                    bin_path.display(),
                    log_path.display()
                ),
            );
            Some(child)
        }
        Err(e) => {
            output::warn(&format!(
                "failed to spawn easynet-daemon ({}): {e}; control.sock + runtime-dispatch.sock will not be available",
                bin_path.display()
            ));
            None
        }
    }
}

fn start_stdio_mcp_server(creds: &config::Credentials) {
    let config = crate::runtime::agents::profiles::mcp::StdioServerConfig {
        server_name: "easynet-device".into(),
        tenant_id: creds.tenant_id.clone(),
        agent_name: None,
    };
    let configured = crate::runtime::agents::profiles::mcp::build_stdio_server(&config);
    let descriptor_count = configured.descriptor_count();
    std::thread::spawn(move || {
        let server = easynet_axon::mcp::StdioMcpServer::new(configured.provider)
            .with_server_name(configured.server_name)
            .with_server_version(env!("CARGO_PKG_VERSION"));
        if let Err(e) = server.run(std::io::stdin().lock(), &mut std::io::stdout()) {
            eprintln!("mcp server exited: {e}");
        }
    });
    output::success(&format!(
        "MCP server started on stdio ({descriptor_count} tools advertised)"
    ));
}

fn run_foreground_with_daemon(creds: &config::Credentials, no_mcp: bool) -> anyhow::Result<()> {
    if !no_mcp {
        start_stdio_mcp_server(creds);
    }

    let shutdown = ShutdownSignal::new();
    install_ctrlc_handler(&shutdown);
    output::info("Running in foreground (Ctrl-C to stop)...");
    shutdown.wait();
    super::stop::run(super::stop::StopArgs {})
}

/// Best-effort by contract — daemon startup completes regardless
/// of advertise failures so heartbeat, federation join, and other
/// surfaces stay reachable. The directory just degrades until a
/// later boot or operator-initiated re-advertise catches up.
pub(crate) fn republish_via_federation_best_effort(
    bridge: &easynet_axon::dendrite_bridge::DendriteBridge,
    creds: &config::Credentials,
) {
    let plan = match build_bootstrap_plan(creds) {
        Ok(p) => p,
        Err(e) => {
            output::warn(&format!("bootstrap plan: {e}"));
            return;
        }
    };
    // Pin the caller URI for hub-shaped federation calls so the
    // bridge stamps `envelope.caller.uri` to a canonical URA —
    // `plan.host_device_ura` already carries that shape (see
    // `build_bootstrap_plan_from`).
    let invoker = crate::runtime::advertise::BridgeAbilityInvoker::with_caller_ura(
        bridge,
        plan.host_device_ura.clone(),
    );

    // Bootstrap self-identity FIRST. Every subsequent signed Invoke
    // (federation.advertise_*, runtime.register_local_tool, anything)
    // carries an `easynet.public_key` derived from the daemon's
    // identity; the runtime rejects them with
    // AXON_EASYNET_SUBJECT_KEY_UNREGISTERED until that key is
    // recorded in `state.identity.node_keys` for this node. Calling
    // bootstrap_self_identity here populates that table once per
    // runtime lifetime; it is a no-op on subsequent calls (the
    // first-writer-wins guard returns replaced_prior=true).
    if !plan.realm.is_empty() {
        let identity_outcome = crate::runtime::publish::bootstrap_self_identity_via_runtime(
            &invoker,
            &creds.tenant_id,
            &plan.realm,
            &creds.node_id,
        );
        match &identity_outcome.result {
            Ok(_) => output::detail(
                "runtime-identity",
                &format!("bootstrapped trusted-key material for {}", creds.node_id),
            ),
            Err(msg) => output::warn(&format!(
                "runtime.bootstrap_self_identity failed: {msg}; signed Invokes will fail until \
                 the runtime accepts this node's key (federation.advertise_* + every \
                 frontend Invoke depend on this)"
            )),
        }
    }

    let outcomes = crate::runtime::publish::republish_abilities_via_advertise(
        &invoker,
        &creds.tenant_id,
        &plan,
    );

    let mut ok = 0usize;
    let mut total = 0usize;
    let mut skipped = false;
    for o in &outcomes {
        if o.label == "skipped" {
            skipped = true;
            continue;
        }
        total += 1;
        match &o.result {
            Ok(_) => ok += 1,
            Err(msg) => {
                output::warn(&format!("advertise {} failed: {msg}", o.label));
            }
        }
    }
    if skipped {
        output::detail(
            "directory",
            "advertise deferred — daemon has no realm yet (run easynet join)",
        );
    } else if total > 0 {
        output::detail(
            "directory",
            &format!(
                "{ok}/{total} federation.advertise_* calls succeeded — entries visible to peers"
            ),
        );
    }

    // Step-3 register: tell axon-runtime that *this daemon* is the
    // implementation behind every daemon-owned ability. Without this
    // an InvokeAbility from the EasyNet frontend reaches the runtime,
    // resolves no SessionRegistry binding, and falls through to
    // NoBinding — even though the daemon is right there listening on
    // its dispatch UDS. Best-effort: a register failure leaves the
    // dispatch path degraded but keeps boot moving.
    if !plan.realm.is_empty() && !plan.host_device_ura.is_empty() {
        let dispatch_endpoint = crate::services::control::runtime_dispatch::dispatch_endpoint_uri();
        let reg_outcomes = crate::runtime::publish::register_local_tools_via_runtime(
            &invoker,
            &creds.tenant_id,
            &plan.realm,
            &creds.node_id,
            &dispatch_endpoint,
        );
        let mut reg_ok = 0usize;
        let mut reg_total = 0usize;
        for o in &reg_outcomes {
            reg_total += 1;
            match &o.result {
                Ok(_) => reg_ok += 1,
                Err(msg) => {
                    output::warn(&format!(
                        "runtime.register_local_tool {} failed: {msg}",
                        o.label
                    ));
                }
            }
        }
        if reg_total > 0 {
            output::detail(
                "runtime-dispatch",
                &format!(
                    "{reg_ok}/{reg_total} runtime.register_local_tool calls succeeded — \
                     daemon-owned abilities are reachable via runtime"
                ),
            );
        }
    }
}

/// Build a `BootstrapPlan` from credentials + the loaded agent
/// registry. Pure function so the test below can exercise it
/// without a real bridge.
fn build_bootstrap_plan(
    creds: &config::Credentials,
) -> anyhow::Result<crate::runtime::agents::profiles::bootstrap::BootstrapPlan> {
    let username = bootstrap_username_for(creds);
    build_bootstrap_plan_from(&creds.tenant_id, &creds.node_id, &username)
}

/// Resolve the hosted-agent owner slug used in canonical
/// `agent/<user>.<id>` URIs.
///
/// Primary source is `credentials.json.username`, which the pairing
/// flow persists once the backend returns the stable username slug.
/// During the migration window older credentials files may still miss
/// it even though the operator has a valid auth session; in that case
/// fall back to `auth.json.username`. We deliberately do NOT fall back
/// to JWT `user_id` because backend visibility filters anchor on the
/// stable username slug, and swapping in the UUID would mint another
/// invisible-but-plausible URI instead of surfacing the missing data.
pub(crate) fn bootstrap_username_for(creds: &config::Credentials) -> String {
    if let Some(username) = creds
        .username
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return username.to_string();
    }
    match crate::facade::cli::auth::load_session() {
        Ok(Some(session)) => session
            .username
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .unwrap_or("")
            .to_string(),
        Ok(None) | Err(_) => String::new(),
    }
}

/// Variant that takes the inputs directly. Public so `agent.rs`'s
/// publish path can construct the plan from a `(tenant_id,
/// node_id, username)` triple already in scope without re-loading
/// credentials. The third argument is the stable username slug the
/// backend resolves for this user and anchors under `user/` / `agent/`
/// URIs; pass empty when the device is not yet joined or the slug is
/// genuinely unavailable.
pub(crate) fn build_bootstrap_plan_from(
    tenant_id: &str,
    node_id: &str,
    username: &str,
) -> anyhow::Result<crate::runtime::agents::profiles::bootstrap::BootstrapPlan> {
    use crate::runtime::agents::profiles::bootstrap::{BootstrapPlan, LlmSubAgent};

    let registry = crate::registry::agents::load_agents()
        .map_err(|e| anyhow::anyhow!("load agent registry: {e}"))?;

    let llm_sub_agents: Vec<LlmSubAgent> = registry
        .agents
        .iter()
        .map(|(name, entry)| LlmSubAgent {
            name: name.clone(),
            agent_type_display: entry.agent_type.to_string(),
            model: entry.model.clone(),
        })
        .collect();

    Ok(BootstrapPlan {
        // The credentials' realm field maps to the tenant for now;
        // a future config split will separate them.
        realm: tenant_id.to_string(),
        user_id: username.to_string(),
        // node_id from credentials is the local node identifier
        // (`en-...`). Wrap it in the canonical URA shape so every
        // downstream consumer (advertise_self_signed_device,
        // BridgeAbilityInvoker::with_caller_ura, hub
        // self-signed-must-equal-caller check) sees one form. The
        // bare node_id remains accessible separately via
        // `creds.node_id` when an entry path needs it.
        host_device_ura: crate::ura::device_ura(tenant_id, node_id),
        // Defaults match plan §1's "default-on consent on
        // interactive hosts"; policy + mcp default off until
        // [profiles] config wiring lands.
        consent: true,
        policy: false,
        mcp: false,
        llm_sub_agents,
    })
}

/// Extract the realm segment from a canonical Agent URA. Returns
/// `None` if the URA shape is not the expected
/// `easynet:///r/<realm>/agent/<id>` form. Used by `agent remove`
/// to find which hub to send `federation.revoke` to.
pub(crate) fn realm_from_agent_ura(uri: &str) -> Option<String> {
    crate::ura::parse_ura(uri).ok().map(|parsed| parsed.realm)
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
        output::info("then run 'easynet join <token>' to pair this device.");
        output::info("If you're running a Hub, use 'easynet start --as-hub' instead.");
        anyhow::bail!("no credentials — cannot start device agent");
    };

    match verify(&creds) {
        CredentialCheck::Valid => Ok((creds, true)),
        CredentialCheck::NetworkUnavailable => Ok((creds, false)),
        CredentialCheck::Revoked(msg) => {
            eprintln!("{} {msg}", console::style("✗").red().bold());
            eprintln!("  node_id:     {}", creds.node_id);
            eprintln!("  hub_session: {}", creds.hub_endpoint);
            eprintln!("  hub_api:     {}", creds.api_base());
            eprintln!("  Credential revoked or device removed from account.");
            eprintln!();
            config::delete_credentials().ok();
            eprintln!("  Stale credentials cleaned up.");
            eprintln!("  Visit https://easynet.run or your Hub to create a new pairing token,");
            eprintln!("  then run 'easynet join <token>'.");
            anyhow::bail!("credential revoked");
        }
    }
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
        runtime_kind: config::RuntimeKind::AxonBridge,
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
        output::info("Hub running in background. Use 'easynet stop' to stop.");
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
                "could not verify credential via {base} (session endpoint: {}): {e}; continuing anyway",
                creds.hub_endpoint
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
            username: None,
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
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

    #[test]
    fn build_bootstrap_plan_threads_credentials_into_plan() {
        let _g = HomeGuard::new();
        // Empty registry: load_agents returns Default. The plan
        // should still build with consent=true (default-on per
        // plan §1) and an empty llm list.
        let creds = test_creds();
        let plan = build_bootstrap_plan(&creds).expect("plan must build");
        assert_eq!(plan.realm, "tenant-test");
        assert_eq!(
            plan.host_device_ura,
            "easynet:///r/tenant-test/device/node-test"
        );
        assert!(plan.consent, "consent default-on per plan §1");
        assert!(!plan.policy);
        assert!(!plan.mcp);
        assert!(plan.llm_sub_agents.is_empty());
    }

    #[test]
    fn build_bootstrap_plan_falls_back_to_auth_session_username() {
        let _g = HomeGuard::new();
        let state_dir = crate::persistence::config::state_dir();
        std::fs::create_dir_all(&state_dir).expect("create state dir");
        let session = crate::facade::cli::auth::AuthSession {
            token: "token".into(),
            hub_url: "http://127.0.0.1:8080".into(),
            email: "alice@example.com".into(),
            user_id: Some("user-uuid".into()),
            nickname: Some("Alice".into()),
            username: Some("alice".into()),
        };
        std::fs::write(
            state_dir.join("auth.json"),
            serde_json::to_vec(&session).expect("serialize session"),
        )
        .expect("write auth.json");

        let mut creds = test_creds();
        creds.username = None;
        let plan = build_bootstrap_plan(&creds).expect("plan must build");
        assert_eq!(plan.user_id, "alice");
    }

    #[test]
    fn build_bootstrap_plan_from_separates_inputs_for_callers_with_existing_creds() {
        let _g = HomeGuard::new();
        // build_bootstrap_plan_from is what agent.rs uses — it
        // takes pre-extracted (tenant, node) pair so callers don't
        // re-load credentials. Behavior must match build_bootstrap_plan
        // for the same inputs. Plan wraps the raw node id into the
        // canonical `easynet:///r/<realm>/device/<node>` resource URA
        // for downstream Hub-tier signing — that wrapping is exactly
        // what the federation Invoke surface consumes, so the test
        // pins the wrapped form rather than the raw bare id.
        let plan = build_bootstrap_plan_from("tenant-test", "node-test", "user-test")
            .expect("plan must build");
        assert_eq!(plan.realm, "tenant-test");
        assert_eq!(plan.user_id, "user-test");
        assert_eq!(
            plan.host_device_ura,
            "easynet:///r/tenant-test/device/node-test"
        );
    }

    #[test]
    fn realm_from_agent_ura_extracts_segment() {
        // URI v4.1.4: hub is realm-singleton (no sub-id); device-id
        // is bare UUID. Function is shape-agnostic — only the
        // `<realm>/<rest>` boundary matters.
        assert_eq!(
            realm_from_agent_ura("easynet:///r/acme/hub"),
            Some("acme".to_string())
        );
        assert_eq!(
            realm_from_agent_ura(
                "easynet:///r/contoso/device/4065c47a-ec6f-4330-87a5-0d69787709b8"
            ),
            Some("contoso".to_string())
        );
    }

    #[test]
    fn realm_from_agent_ura_returns_none_for_malformed_uris() {
        assert_eq!(realm_from_agent_ura(""), None);
        assert_eq!(realm_from_agent_ura("not-an-uri"), None);
        assert_eq!(realm_from_agent_ura("easynet:///r/"), None);
        assert_eq!(realm_from_agent_ura("easynet:///r/acme"), None);
        assert_eq!(
            realm_from_agent_ura("http://example.com/r/acme/agent/X"),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_uds_alive_false_when_path_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("control.sock");
        assert!(!sock.exists());
        assert!(!crate::support::local_daemon_grpc::probe_accepting(&sock));
    }

    #[cfg(unix)]
    #[test]
    fn probe_uds_alive_false_for_stale_socket_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("control.sock");
        std::fs::write(&sock, b"").expect("write empty file");
        assert!(sock.exists());
        assert!(!crate::support::local_daemon_grpc::probe_accepting(&sock));
    }

    #[cfg(unix)]
    #[test]
    fn probe_uds_alive_true_when_listener_accepts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("control.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind probe");
        assert!(crate::support::local_daemon_grpc::probe_accepting(&sock));
    }

}
