// EasyNet CLI
// ===========
//
// File: src/cli/commands/start.rs
// Description: `easynet runtime start` — starts the EasyNet product daemon,
//              joins Hub, registers node, and opens the product session.
//              Supports both foreground and background modes.
//
// Lifecycle:
// - Reads daemon process facts first, then treats runtime.json as a projection.
// - Both device and hub modes start `easynet-daemon` (mode=device / mode=hub) — the single product policy owner — not a raw Axon runtime.
// - If credentials exist (from `easynet device join`), registers the node and opens the session path.
// - In foreground mode: blocks on Ctrl-C, then gracefully deregisters + shuts down.
// - In background mode: detaches the daemon and leaves stop ownership to `runtime stop`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Context;
use clap::Args;

use crate::daemon::boot::join_connection_state::{
    classify_boot_failure, record_snapshot, JoinConnectionSnapshot, JoinConnectionState,
    JoinFailureCode, JoinTransition,
};
use crate::daemon::lifecycle::{
    RuntimeLifecycleService, RuntimeStartPreflightAction, RuntimeStartRequest,
};
use crate::daemon::persistence::config;
use crate::support::platform::{output, shutdown::ShutdownSignal};

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
    /// TLS certificate (PEM) for Hub mode's public TCP listener.
    /// Required the first time a hub starts without a daemon-config.toml;
    /// used to scaffold one. A public hub cannot run without TLS.
    #[arg(long)]
    pub cert: Option<std::path::PathBuf>,
    /// TLS private key (PEM) for Hub mode's public TCP listener.
    /// Pairs with --cert.
    #[arg(long)]
    pub key: Option<std::path::PathBuf>,
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
            cert: None,
            key: None,
            foreground: true,
            no_mcp,
            insecure: false,
        }
    }

    /// Construct args for the auto-start hop at the tail of
    /// `easynet device join`. Background mode so the join command returns
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
            cert: None,
            key: None,
            foreground: false,
            no_mcp: false,
            insecure: false,
        }
    }
}

/// Result of `verify_credential` — tracks whether the Hub explicitly
/// accepted the paired credential before daemon startup.
#[derive(Debug, PartialEq, Eq)]
enum CredentialCheck {
    Valid,
    NetworkUnavailable,
    Revoked(String),
}

/// Device-mode preflight budget for the Hub session socket.
///
/// This is intentionally a plain TCP reachability probe, not a federation
/// session connect. `runtime start` is about to spawn
/// `easynet-daemon`, and the daemon owns the real Axon Invocation /
/// session handshake. The CLI only needs to reject a clearly absent
/// listener before forking a background process; requiring the
/// optional dendrite bridge dynamic library here makes successful
/// pairing depend on a local SDK artifact that device mode does not
/// need.
const HUB_SESSION_ENDPOINT_CONNECT_TIMEOUT_MS: u64 = 5_000;

pub fn run(args: StartArgs) -> anyhow::Result<()> {
    if args.as_hub {
        return run_as_hub(&args);
    }

    run_device_mode(&args)
}

fn preflight_runtime_start(request: &RuntimeStartRequest) -> anyhow::Result<()> {
    let report = RuntimeLifecycleService::new().preflight_start(request)?;
    match report.action() {
        RuntimeStartPreflightAction::CleanStart => Ok(()),
        RuntimeStartPreflightAction::RemovedStaleProjection => {
            output::info("Detected stale runtime projection (process not running). Cleaning up...");
            Ok(())
        }
        RuntimeStartPreflightAction::AttachAndRebuildProjection => {
            output::warn(
                "Detected live daemon without runtime.json; attaching and rebuilding runtime projection.",
            );
            Ok(())
        }
        RuntimeStartPreflightAction::AlreadyRunning => Ok(()),
    }
}

fn save_runtime_projection_after_ready(
    handle: &mut crate::daemon::DaemonHandle,
    state: &config::RuntimeState,
) -> anyhow::Result<()> {
    RuntimeLifecycleService::new()
        .save_projection_after_ready(handle, state)
        .context("persist runtime projection after daemon Ready")
}

fn ensure_desktop_companions_after_ready() {
    let Ok(state) = crate::daemon::plugins::default_state() else {
        return;
    };
    let failures = crate::daemon::plugins::DesktopCompanionManager::current()
        .ensure_running_after_daemon_ready(state.index().packages());
    for failure in failures {
        crate::op_event!(
            component = desktop_companion,
            kind = post_ready_reconcile_failed,
            package_id = failure.package_id,
            package_version = failure.package_version,
            action = failure.action,
            code = failure.code,
            reason = failure.reason,
        );
        output::warn(&format!("desktop companion reconcile warning: {failure}"));
    }
}

// ── Device mode ─────────────────────────────────────────────────────────────

fn run_device_mode(args: &StartArgs) -> anyhow::Result<()> {
    // Load and verify credentials (from `easynet device join`).
    let (creds, credential_verified) = load_and_verify_credentials()?;
    record_snapshot(JoinConnectionSnapshot::from_credentials(
        JoinConnectionState::HubCredentialVerified,
        Some(JoinTransition::VerifyCredential),
        &creds,
        "cli.start",
    ));
    if let Err(err) = verify_hub_session_endpoint(&creds) {
        record_snapshot(JoinConnectionSnapshot::failed_from_credentials(
            JoinFailureCode::StartFailedSessionEndpoint,
            JoinTransition::ConnectSessionEndpoint,
            &creds,
            err.to_string(),
            true,
            "cli.start",
        ));
        return Err(err);
    }
    record_snapshot(JoinConnectionSnapshot::from_credentials(
        JoinConnectionState::HubSessionEndpointReachable,
        Some(JoinTransition::ConnectSessionEndpoint),
        &creds,
        "cli.start",
    ));

    // Credentials take precedence over CLI args for hub/tenant.
    let hub = creds.hub_endpoint.clone();
    let tenant = creds.realm.clone();
    if args.hub != config::DEFAULT_HUB && args.hub != hub {
        output::warn(&format!(
            "--hub {} ignored; using {} from credentials. Run 'easynet reset' to un-pair first.",
            args.hub, hub
        ));
    }
    let label = args.label.clone().unwrap_or_else(|| creds.node_id.clone());
    let _ = (args.token.as_deref(), args.insecure);
    preflight_runtime_start(&RuntimeStartRequest::device(&tenant, &creds.node_id))?;
    // EASYNET_PAGES_PORT is parsed by the daemon — it is the only
    // process that needs to validate the value and decide a default.
    // CLI just peeks at it for the progress UI's "fell back from N"
    // hint, treating any parse failure as "no hint available".
    let pages_start_hint = std::env::var("EASYNET_PAGES_PORT")
        .ok()
        .and_then(|raw| raw.parse::<u16>().ok())
        .filter(|p| *p > 0);

    crate::daemon::persistence::daemon_config::ensure_minimal_device_config(&creds)
        .context("ensure daemon-config.toml for device mode")?;
    super::federation_wire::auto_wire_self_realm_trust_from_credentials(&creds)
        .context("wire local realm trust for device mode")?;
    bootstrap_local_agent_projection(&creds).context("sync local agent owner projection")?;

    record_snapshot(JoinConnectionSnapshot::from_credentials(
        JoinConnectionState::RuntimeStarting,
        Some(JoinTransition::BootDaemon),
        &creds,
        "cli.start",
    ));
    let mut daemon_handle = match crate::daemon::DaemonStartConfig::device(&creds.node_id)
        .map(|cfg| cfg.with_realm(creds.realm_str()))
        .map(with_bridge_lib_env)
        .and_then(|cfg| cfg.start())
    {
        Ok(handle) => handle,
        Err(err) => {
            record_snapshot(JoinConnectionSnapshot::failed_from_credentials(
                JoinFailureCode::StartFailedBootStage,
                JoinTransition::BootDaemon,
                &creds,
                err.to_string(),
                true,
                "cli.start",
            ));
            return Err(err).context("start easynet-daemon");
        }
    };
    let attached_existing_daemon = daemon_handle.child_mut().is_none();
    record_snapshot(JoinConnectionSnapshot::from_credentials(
        JoinConnectionState::DaemonBooting,
        Some(JoinTransition::BootDaemon),
        &creds,
        "cli.start",
    ));
    // From here the daemon process owns the asynchronous `session.open`
    // lifecycle. Publish the pending state before waiting for Ready so a fast
    // session contract cannot be overwritten later by the CLI's older view.
    // Boot failures still record a failed snapshot below; successful contracts
    // promote this to J800 from the frame loop.
    record_snapshot(JoinConnectionSnapshot::from_credentials(
        JoinConnectionState::SelfSessionAdmissionPending,
        Some(JoinTransition::OpenSelfSession),
        &creds,
        "cli.start",
    ));
    let control_socket = daemon_handle.control_endpoint().to_path_buf();
    let boot = match super::start_boot_watcher::wait_for_daemon_boot(
        &control_socket,
        daemon_handle.child_mut(),
        super::start_boot_watcher::BootContext {
            pages_start_port: pages_start_hint,
        },
    ) {
        Ok(boot) => boot,
        Err(err) => {
            let message = err.to_string();
            let (failure, transition, retryable) = classify_boot_failure(&message);
            record_snapshot(JoinConnectionSnapshot::failed_from_credentials(
                failure,
                transition,
                &creds,
                message,
                retryable,
                "cli.start",
            ));
            return Err(err);
        }
    };
    // The daemon is the authoritative source for the bound port: it
    // either reported it via PortChosen, or wrote it to control.json
    // when the listener bound. The CLI never has a meaningful
    // fallback here — if neither is set, surface that as an error
    // (the listener boot stage would already have emitted Failed
    // and `wait_for_daemon_boot` would have returned before we got
    // here, so this branch is defence-in-depth only).
    let pages_listener_port = super::start_boot_watcher::final_pages_port(boot.pages_port)
        .ok_or_else(|| anyhow::anyhow!("daemon reported Ready without binding a pages port"))?;
    let pid = daemon_handle.pid();
    let endpoint = daemon_handle.invocation_endpoint().display().to_string();

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
    save_runtime_projection_after_ready(&mut daemon_handle, &state)?;
    ensure_desktop_companions_after_ready();

    if attached_existing_daemon {
        output::success("EasyNet daemon attached");
    } else {
        output::success("EasyNet daemon started");
    }
    let control_socket = daemon_handle.control_endpoint().display().to_string();
    let hub_api = creds.api_base();
    let pages_url_root = format!(
        "http://<project>.{user}.pages.localhost:{pages_listener_port}/",
        user = creds.username_slug()?
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

    // Welcome line — surface the human-readable paired account while the
    // canonical user URA below stays anchored on credentials.user_id.
    let username = creds.username_slug()?;
    let user_ura = creds.user_ura()?;
    eprintln!();
    eprintln!(
        "{} {}",
        console::style("Welcome,").cyan().bold(),
        console::style(username).cyan().bold(),
    );
    eprintln!("  {}", console::style(user_ura).dim());

    if args.foreground {
        run_foreground_with_daemon(&creds, args.no_mcp)
    } else {
        output::info("Daemon running in background. Use 'easynet runtime stop' to stop.");
        Ok(())
    }
}

/// Inject `EASYNET_DENDRITE_BRIDGE_LIB` into the daemon's environment.
///
/// `easynet device join` stages the native bridge lib into
/// `~/.easynet/dendrite-bridge/native/`, but the Axon SDK loader the
/// daemon links against does not search that path — it only honours the
/// env var (plus a gated local-source build and a crate-relative
/// embedded copy). Without this, a fresh `join` → `runtime start`
/// fails with "dendrite bridge library not found" even though the lib
/// is on disk. `bridge_lib::resolve_bridge_lib` is the same chain the
/// MCP-install flow uses, so the daemon and the MCP server resolve the
/// lib identically.
///
/// Best-effort: if the var is already in this process's environment the
/// daemon inherits it (no override), and if resolution finds nothing we
/// leave the env untouched so the SDK loader's own tiers still apply.
fn with_bridge_lib_env(cfg: crate::daemon::DaemonStartConfig) -> crate::daemon::DaemonStartConfig {
    if std::env::var("EASYNET_DENDRITE_BRIDGE_LIB").is_ok_and(|v| !v.trim().is_empty()) {
        return cfg;
    }
    match crate::cli::daemon_client::bridge_lib::resolve_bridge_lib(None) {
        Ok(Some(lib)) => cfg.with_env("EASYNET_DENDRITE_BRIDGE_LIB", lib),
        _ => cfg,
    }
}

fn start_stdio_mcp_server(creds: &config::Credentials) -> anyhow::Result<()> {
    let config = crate::daemon::ability::catalog::profiles::mcp::StdioServerConfig {
        server_name: "easynet-device".into(),
        tenant_id: creds.realm.clone(),
        agent_name: None,
    };
    let configured = crate::daemon::ability::catalog::profiles::mcp::build_stdio_server(&config)?;
    let descriptor_count = configured.descriptor_count();
    std::thread::spawn(move || {
        let server = crate::daemon::execution::mcp::stdio::StdioMcpServer::new(configured.provider)
            .with_server_name(configured.server_name)
            .with_server_version(env!("CARGO_PKG_VERSION"));
        if let Err(e) = server.run(std::io::stdin().lock(), &mut std::io::stdout()) {
            eprintln!("mcp server exited: {e}");
        }
    });
    output::success(&format!(
        "MCP server started on stdio ({descriptor_count} tools advertised)"
    ));
    Ok(())
}

fn run_foreground_with_daemon(creds: &config::Credentials, no_mcp: bool) -> anyhow::Result<()> {
    if !no_mcp {
        start_stdio_mcp_server(creds)?;
    }

    let shutdown = ShutdownSignal::new();
    install_ctrlc_handler(&shutdown);
    output::info("Running in foreground (Ctrl-C to stop)...");
    shutdown.wait();
    super::stop::run(super::stop::StopArgs {})
}

/// Build a `BootstrapPlan` from credentials + the loaded agent
/// registry. Pure function so the test below can exercise it
/// without a real bridge.
fn build_bootstrap_plan(
    creds: &config::Credentials,
) -> anyhow::Result<crate::daemon::ability::catalog::profiles::bootstrap::BootstrapPlan> {
    let user_id = creds.user_id()?;
    let username = creds.username_slug()?;
    build_bootstrap_plan_from(&creds.realm, &creds.node_id, user_id, username)
}

/// Variant that takes the inputs directly. Public so `agent.rs`'s
/// publish path can construct the plan from a `(realm, node_id,
/// user_id, username)` tuple already in scope without re-loading
/// credentials. `user_id` (UUID) is the immutable subject anchor for
/// `user/` trust URAs; `username` (slug) is the owner-prefix for
/// `agent/<username>.<id>` URAs (§15.1-3 dual grammar).
pub(crate) fn build_bootstrap_plan_from(
    realm: &str,
    node_id: &str,
    user_id: &str,
    username: &str,
) -> anyhow::Result<crate::daemon::ability::catalog::profiles::bootstrap::BootstrapPlan> {
    crate::daemon::ability::catalog::profiles::bootstrap::build_plan_from_registry(
        realm, node_id, user_id, username,
    )
}

fn bootstrap_local_agent_projection(
    creds: &config::Credentials,
) -> anyhow::Result<Vec<crate::daemon::ability::catalog::profiles::bootstrap::BootstrapOutcome>> {
    let plan = build_bootstrap_plan(creds)?;
    crate::daemon::ability::builtins::agents::lifecycle::bootstrap_local_agent_projection(&plan)
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
        output::info("then run 'easynet device join <token>' to pair this device.");
        output::info("If you're running a Hub, use 'easynet runtime start --as-hub' instead.");
        anyhow::bail!("no credentials — cannot start device agent");
    };

    if has_daemon_native_join_lineage(&creds) {
        output::info(
            "Hub URA join lineage detected; skipping backend HTTP credential verification.",
        );
        return Ok((creds, true));
    }

    match verify(&creds) {
        CredentialCheck::Valid => Ok((creds, true)),
        CredentialCheck::NetworkUnavailable => {
            record_snapshot(JoinConnectionSnapshot::failed_from_credentials(
                JoinFailureCode::StartFailedCredentialVerify,
                JoinTransition::VerifyCredential,
                &creds,
                "Hub credential verification is unavailable",
                true,
                "cli.start",
            ));
            eprintln!(
                "{} Hub credential verification is unavailable.",
                console::style("✗").red().bold()
            );
            eprintln!("  node_id:     {}", creds.node_id);
            eprintln!("  hub_session: {}", creds.hub_endpoint);
            eprintln!("  hub_api:     {}", creds.api_base());
            eprintln!();
            eprintln!(
                "  Refusing to start the device daemon because Hub reachability is required \
                 before PresenceRegistry/session state can be trusted."
            );
            eprintln!(
                "  For Docker/local hubs, re-pair with --hub http://127.0.0.1:8080 \
                 or pass --hub-api so verification hits the correct backend."
            );
            anyhow::bail!("hub credential verification unavailable")
        }
        CredentialCheck::Revoked(msg) => {
            record_snapshot(JoinConnectionSnapshot::failed_from_credentials(
                JoinFailureCode::StartFailedCredentialVerify,
                JoinTransition::VerifyCredential,
                &creds,
                msg.clone(),
                false,
                "cli.start",
            ));
            eprintln!("{} {msg}", console::style("✗").red().bold());
            eprintln!("  node_id:     {}", creds.node_id);
            eprintln!("  hub_session: {}", creds.hub_endpoint);
            eprintln!("  hub_api:     {}", creds.api_base());
            eprintln!("  Credential revoked or device removed from account.");
            eprintln!();
            config::delete_credentials().ok();
            eprintln!("  Stale credentials cleaned up.");
            eprintln!("  Visit https://easynet.run or your Hub to create a new pairing token,");
            eprintln!("  then run 'easynet device join <token>'.");
            anyhow::bail!("credential revoked");
        }
    }
}

fn has_daemon_native_join_lineage(creds: &config::Credentials) -> bool {
    creds.credential_token.trim().is_empty()
        && creds
            .join_receipt_hash
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        && creds
            .hub_pubkey_b64
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

fn verify_hub_session_endpoint(creds: &config::Credentials) -> anyhow::Result<()> {
    verify_hub_session_endpoint_with(creds, tcp_connect_hub_session_endpoint)
}

fn verify_hub_session_endpoint_with<F>(
    creds: &config::Credentials,
    connect: F,
) -> anyhow::Result<()>
where
    F: FnOnce(&str) -> anyhow::Result<()>,
{
    let endpoint = creds.hub_endpoint.trim();
    if endpoint.is_empty() {
        anyhow::bail!("credentials missing hub_endpoint");
    }
    connect(endpoint).with_context(|| {
        format!(
            "connect Hub session endpoint {endpoint}; refusing to start daemon until the Hub session endpoint is reachable"
        )
    })
}

fn tcp_connect_hub_session_endpoint(endpoint: &str) -> anyhow::Result<()> {
    let addrs = endpoint_socket_addrs(endpoint)?;
    let mut last_err = None;
    for addr in addrs {
        match TcpStream::connect_timeout(
            &addr,
            Duration::from_millis(HUB_SESSION_ENDPOINT_CONNECT_TIMEOUT_MS),
        ) {
            Ok(_) => return Ok(()),
            Err(err) => last_err = Some(err),
        }
    }
    match last_err {
        Some(err) => Err(err).context("tcp connect to Hub session endpoint"),
        None => anyhow::bail!("Hub session endpoint resolved to no socket addresses"),
    }
}

fn endpoint_socket_addrs(endpoint: &str) -> anyhow::Result<Vec<std::net::SocketAddr>> {
    let (host, port) = parse_endpoint_host_port(endpoint)?;
    let addrs: Vec<_> = (host.as_str(), port)
        .to_socket_addrs()
        .with_context(|| format!("resolve Hub session endpoint host {host}:{port}"))?
        .collect();
    Ok(addrs)
}

fn parse_endpoint_host_port(endpoint: &str) -> anyhow::Result<(String, u16)> {
    let trimmed = endpoint.trim();
    anyhow::ensure!(!trimmed.is_empty(), "Hub session endpoint is empty");

    let (scheme, rest) = trimmed
        .split_once("://")
        .map(|(scheme, rest)| (Some(scheme), rest))
        .unwrap_or((None, trimmed));
    let default_port = match scheme {
        Some("axon") => Some(50051),
        Some("https") => Some(443),
        Some("http") => Some(80),
        _ => None,
    };
    let authority_with_userinfo = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    let authority = authority_with_userinfo
        .rsplit_once('@')
        .map(|(_, authority)| authority)
        .unwrap_or(authority_with_userinfo);
    anyhow::ensure!(
        !authority.is_empty(),
        "Hub session endpoint missing authority: {endpoint}"
    );

    if let Some(bracketed) = authority.strip_prefix('[') {
        let (host, rest) = bracketed
            .split_once(']')
            .with_context(|| format!("invalid bracketed Hub endpoint host: {endpoint}"))?;
        anyhow::ensure!(
            !host.is_empty(),
            "Hub session endpoint missing host: {endpoint}"
        );
        anyhow::ensure!(
            rest.is_empty() || rest.starts_with(':'),
            "invalid bracketed Hub endpoint authority: {endpoint}"
        );
        let port = match rest.strip_prefix(':') {
            Some(raw) => raw
                .parse::<u16>()
                .with_context(|| format!("invalid Hub session endpoint port: {endpoint}"))?,
            None => default_port
                .with_context(|| format!("Hub session endpoint missing port: {endpoint}"))?,
        };
        return Ok((host.to_string(), port));
    }

    let (host, port_raw) = match authority.rsplit_once(':') {
        Some((host, port_raw)) => (host, Some(port_raw)),
        None => (authority, None),
    };
    anyhow::ensure!(
        !host.is_empty(),
        "Hub session endpoint missing host: {endpoint}"
    );
    anyhow::ensure!(
        !host.contains(':'),
        "IPv6 Hub session endpoint host must be bracketed: {endpoint}"
    );
    let port = match port_raw {
        Some(raw) => raw
            .parse::<u16>()
            .with_context(|| format!("invalid Hub session endpoint port: {endpoint}"))?,
        None => default_port
            .with_context(|| format!("Hub session endpoint missing port: {endpoint}"))?,
    };
    Ok((host.to_string(), port))
}

// ── Hub mode ────────────────────────────────────────────────────────────────

/// Resolve the hub-mode daemon config, scaffolding it from `--cert` /
/// `--key` when none exists. Fails fast (no daemon spawn, no raw
/// axon-runtime) when the config is absent without TLS material or is
/// not a hub/both-mode config. Split out from `run_as_hub` so the
/// "hub never starts a raw runtime" exit criterion is unit-testable
/// without spawning a daemon.
fn resolve_hub_config(
    args: &StartArgs,
) -> anyhow::Result<crate::daemon::persistence::daemon_config::DaemonConfig> {
    use crate::daemon::persistence::daemon_config::{self, DaemonConfig, DaemonMode};

    let config_path = daemon_config::default_config_path();
    if !config_path.exists() {
        match (&args.cert, &args.key) {
            (Some(cert), Some(key)) => {
                daemon_config::ensure_hub_config(&args.bind, &args.tenant, cert, key)
                    .context("scaffold hub daemon-config.toml")?;
            }
            _ => anyhow::bail!(
                "hub mode needs a TLS-bearing daemon config at {}.\n\
                 Either pass --cert <cert.pem> --key <key.pem> to scaffold one, \
                 or author it by hand:\n  \
                 [daemon]\n  mode = \"hub\"\n  realm = \"{}\"\n  \
                 listen_tcp = \"{}\"\n  tls_cert_pem = \"<path/to/cert.pem>\"\n  \
                 tls_key_pem = \"<path/to/key.pem>\"\n\
                 A public hub cannot run without TLS.",
                config_path.display(),
                args.tenant,
                args.bind,
            ),
        }
    }
    let cfg = DaemonConfig::load(&config_path)
        .with_context(|| format!("load hub daemon config at {}", config_path.display()))?;
    if !matches!(cfg.mode(), DaemonMode::Hub | DaemonMode::Both) {
        anyhow::bail!(
            "daemon config at {} is mode={:?}, not hub/both — \
             set `mode = \"hub\"` to start as a hub",
            config_path.display(),
            cfg.mode(),
        );
    }
    Ok(cfg)
}

fn run_as_hub(args: &StartArgs) -> anyhow::Result<()> {
    // Hub mode runs through `easynet-daemon` (mode=hub), the same policy
    // owner as device mode — not a raw axon-runtime. A hub binds a
    // public TCP+TLS Invocation listener, so its config MUST carry
    // `listen_tcp` + TLS material; the daemon refuses to bind TCP in
    // plaintext (load Invariant 2).
    let cfg = resolve_hub_config(args)?;
    let realm = cfg.realm().to_string();
    preflight_runtime_start(&RuntimeStartRequest::hub(&realm))?;

    let start_cfg = with_bridge_lib_env(crate::daemon::DaemonStartConfig::hub().with_realm(&realm));
    let mut daemon_handle = start_cfg.start().context("start hub easynet-daemon")?;
    let attached_existing_daemon = daemon_handle.child_mut().is_none();
    let control_socket = daemon_handle.control_endpoint().to_path_buf();
    super::start_boot_watcher::wait_for_daemon_boot(
        &control_socket,
        daemon_handle.child_mut(),
        super::start_boot_watcher::BootContext {
            pages_start_port: None,
        },
    )?;
    let pid = daemon_handle.pid();
    let endpoint = daemon_handle.invocation_endpoint().display().to_string();

    let state = config::RuntimeState {
        endpoint: endpoint.clone(),
        runtime_kind: config::RuntimeKind::DaemonOnly,
        pid,
        hub: None,
        tenant: Some(realm),
        label: Some("hub".to_string()),
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        credential_verified: None, // Not applicable in hub mode.
    };
    save_runtime_projection_after_ready(&mut daemon_handle, &state)?;
    ensure_desktop_companions_after_ready();

    if attached_existing_daemon {
        output::success("EasyNet hub attached");
    } else {
        output::success("EasyNet hub started");
    }
    let control_socket = daemon_handle.control_endpoint().display().to_string();
    let listen_tcp = cfg
        .listen_tcp()
        .map(|a| a.to_string())
        .unwrap_or_else(|| args.bind.clone());
    let mut rows = vec![
        ("daemon_socket", endpoint.as_str()),
        ("control_socket", control_socket.as_str()),
        ("listen_tcp", listen_tcp.as_str()),
        ("realm", cfg.realm()),
    ];
    let pid_display = pid.map(|pid| pid.to_string());
    if let Some(ref pid) = pid_display {
        rows.push(("pid", pid.as_str()));
    }
    output::kv_section(&rows);
    output::step(&format!(
        "Devices can join with: easynet runtime start --hub axon://<this-ip>:{}",
        listen_tcp.rsplit(':').next().unwrap_or("50051"),
    ));

    if args.foreground {
        run_foreground_with_daemon_hub()
    } else {
        output::info("Hub running in background. Use 'easynet runtime stop' to stop.");
        Ok(())
    }
}

/// Foreground hub: block until Ctrl-C, then leave the daemon running
/// (the same lifecycle contract as background — `easynet runtime stop` owns
/// teardown). The daemon is a detached child; we only hold the console.
fn run_foreground_with_daemon_hub() -> anyhow::Result<()> {
    let shutdown = ShutdownSignal::new();
    install_ctrlc_handler(&shutdown);
    output::info("Running in foreground (Ctrl-C to detach)...");
    shutdown.wait();
    output::info("Detached. Hub still running — use 'easynet runtime stop' to stop it.");
    Ok(())
}

// ── Credential verification ─────────────────────────────────────────────────

fn classify_credential_status_code(code: u16) -> CredentialCheck {
    if (400..500).contains(&code) {
        match code {
            // 404 = endpoint not found (Hub version mismatch), 429 = rate limited — transient.
            404 | 429 => {
                output::warn(&format!("Hub returned HTTP {code}; refusing to start"));
                CredentialCheck::NetworkUnavailable
            }
            // 401/403 = credential explicitly rejected.
            // Other 4xx (400, 422, etc.) = client-side error, likely a bad credential.
            _ => CredentialCheck::Revoked(format!("credential rejected by Hub (HTTP {code})")),
        }
    } else if code >= 500 {
        output::warn(&format!(
            "Hub returned server error (HTTP {code}); refusing to start"
        ));
        CredentialCheck::NetworkUnavailable
    } else {
        output::warn(&format!(
            "unexpected Hub response (HTTP {code}) during credential check; refusing to start"
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
                "could not verify credential via {base} (session endpoint: {}): {e}; refusing to start",
                creds.hub_endpoint
            ));
            CredentialCheck::NetworkUnavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;

    fn test_creds() -> config::Credentials {
        config::Credentials {
            node_id: "node-test".into(),
            credential_token: "token-test".into(),
            hub_endpoint: "axon://easynet.run:50051".into(),
            realm: "tenant-test".into(),
            deploy_signature: "sig".into(),
            hub_api_base: Some("https://api.example.com".into()),
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        }
    }

    fn hub_args(cert: Option<&str>, key: Option<&str>) -> StartArgs {
        StartArgs {
            hub: config::DEFAULT_HUB.into(),
            tenant: "tenant-hub".into(),
            label: None,
            token: None,
            as_hub: true,
            bind: "0.0.0.0:50051".into(),
            cert: cert.map(std::path::PathBuf::from),
            key: key.map(std::path::PathBuf::from),
            foreground: false,
            no_mcp: false,
            insecure: false,
        }
    }

    #[test]
    fn hub_without_config_or_tls_flags_fails_fast() {
        // Exit criterion: hub must NOT start a raw axon-runtime. With no
        // config and no --cert/--key, resolution fails before any spawn.
        let _g = HomeGuard::new();
        let err = resolve_hub_config(&hub_args(None, None))
            .expect_err("hub without TLS config must fail fast");
        let msg = err.to_string();
        assert!(msg.contains("TLS-bearing daemon config"), "got: {msg}");
        assert!(msg.contains("--cert"), "must guide operator: {msg}");
    }

    #[test]
    fn hub_with_tls_flags_scaffolds_hub_mode_config() {
        // --cert/--key scaffold a mode=hub config that resolves to the
        // daemon-owned hub path (DaemonMode::Hub), never a bridge.
        use crate::daemon::persistence::daemon_config::DaemonMode;
        let _g = HomeGuard::new();
        let cfg = resolve_hub_config(&hub_args(Some("/tmp/c.pem"), Some("/tmp/k.pem")))
            .expect("hub with TLS flags must resolve");
        assert_eq!(cfg.mode(), DaemonMode::Hub);
        assert_eq!(cfg.realm(), "tenant-hub");
        assert!(
            cfg.hub_endpoint().is_none(),
            "a hub has no upstream hub of its own"
        );
    }

    #[test]
    fn hub_rejects_device_mode_config() {
        // An existing device-mode config must not be silently started as
        // a hub — fail fast with mode guidance.
        let _g = HomeGuard::new();
        crate::daemon::persistence::daemon_config::ensure_minimal_device_config(&test_creds())
            .expect("seed device config");
        let err = resolve_hub_config(&hub_args(Some("/tmp/c.pem"), Some("/tmp/k.pem")))
            .expect_err("device-mode config must be rejected for hub start");
        assert!(err.to_string().contains("not hub/both"), "got: {err}");
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
    fn load_and_verify_credentials_fails_but_keeps_credentials_when_hub_unavailable() {
        let _g = HomeGuard::new();
        let creds = test_creds();
        config::save_credentials(&creds).expect("save test credentials");

        let err = load_and_verify_credentials_with(|_| CredentialCheck::NetworkUnavailable)
            .expect_err("unreachable Hub must stop daemon startup");
        assert!(
            err.to_string()
                .contains("hub credential verification unavailable"),
            "got: {err}"
        );
        assert!(
            config::load_credentials().is_ok(),
            "credentials should remain on transient outage; only revocation deletes them"
        );
    }

    #[test]
    fn load_and_verify_credentials_skips_backend_for_hub_ura_join_lineage() {
        let _g = HomeGuard::new();
        let mut creds = test_creds();
        creds.credential_token.clear();
        creds.join_receipt_hash = Some("sha256:test-join-receipt".into());
        creds.hub_pubkey_b64 = Some("hub-pubkey".into());
        config::save_credentials(&creds).expect("save daemon-native credentials");

        let (loaded, verified) = load_and_verify_credentials_with(|_| {
            panic!("backend verify must not be called for Hub URA join credentials")
        })
        .expect("daemon-native credentials should pass without backend HTTP");

        assert!(verified);
        assert_eq!(loaded.node_id, "node-test");
    }

    #[test]
    fn verify_hub_session_endpoint_fails_before_daemon_start_when_unreachable() {
        let creds = test_creds();
        let err = verify_hub_session_endpoint_with(&creds, |endpoint| {
            anyhow::bail!("dial refused at {endpoint}")
        })
        .expect_err("unreachable Hub session endpoint must fail");
        let msg = err.to_string();
        assert!(
            msg.contains("connect Hub session endpoint axon://easynet.run:50051"),
            "got: {msg}"
        );
        assert!(
            msg.contains("refusing to start daemon"),
            "operator needs explicit no-start reason: {msg}"
        );
    }

    #[test]
    fn verify_hub_session_endpoint_uses_plain_tcp_probe() {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind local Hub session stand-in");
        let endpoint = format!("https://{}", listener.local_addr().unwrap());
        let mut creds = test_creds();
        creds.hub_endpoint = endpoint;

        verify_hub_session_endpoint(&creds).expect("plain TCP listener should pass preflight");
    }

    #[test]
    fn parse_endpoint_host_port_accepts_runtime_endpoint_forms() {
        assert_eq!(
            parse_endpoint_host_port("https://127.0.0.1:50443").unwrap(),
            ("127.0.0.1".into(), 50443)
        );
        assert_eq!(
            parse_endpoint_host_port("axon://easynet.run:50051").unwrap(),
            ("easynet.run".into(), 50051)
        );
        assert_eq!(
            parse_endpoint_host_port("axon://easynet.run").unwrap(),
            ("easynet.run".into(), 50051)
        );
        assert_eq!(
            parse_endpoint_host_port("https://hub.example").unwrap(),
            ("hub.example".into(), 443)
        );
        assert_eq!(
            parse_endpoint_host_port("https://[::1]:50443/path").unwrap(),
            ("::1".into(), 50443)
        );
        assert_eq!(
            parse_endpoint_host_port("https://[::1]/path").unwrap(),
            ("::1".into(), 443)
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
        assert!(!plan.mcp);
        assert!(plan.llm_sub_agents.is_empty());
    }

    #[test]
    fn build_bootstrap_plan_rejects_credentials_without_user_id() {
        let mut creds = test_creds();
        creds.user_id = None;
        let err = build_bootstrap_plan(&creds).expect_err("user_id is required");
        assert!(
            err.to_string().contains("missing user_id"),
            "error should surface the credential contract: {err}"
        );
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
        let plan = build_bootstrap_plan_from("tenant-test", "node-test", "user-test", "alice")
            .expect("plan must build");
        assert_eq!(plan.realm, "tenant-test");
        assert_eq!(plan.user_id, "user-test");
        assert_eq!(plan.username, "alice");
        assert_eq!(
            plan.host_device_ura,
            "easynet:///r/tenant-test/device/node-test"
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_uds_alive_false_when_path_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("control.sock");
        assert!(!sock.exists());
        assert!(!crate::support::platform::local_daemon_grpc::probe_accepting(&sock));
    }

    #[cfg(unix)]
    #[test]
    fn probe_uds_alive_false_for_stale_socket_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("control.sock");
        std::fs::write(&sock, b"").expect("write empty file");
        assert!(sock.exists());
        assert!(!crate::support::platform::local_daemon_grpc::probe_accepting(&sock));
    }

    #[cfg(unix)]
    #[test]
    fn probe_uds_alive_true_when_listener_accepts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let sock = dir.path().join("control.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind probe");
        assert!(crate::support::platform::local_daemon_grpc::probe_accepting(&sock));
    }
}
