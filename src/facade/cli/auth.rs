// EasyNet CLI — auth: HTTP-driven backend session management
// =============================================================
//
// File: src/facade/cli/auth.rs
//
// Talks to the EasyNet backend's `/api/v1/auth/*` and
// `/api/v1/devices/pairing` HTTP surface from a local CLI session
// — same shape as ssh / kubectl / gh: log in once, the token sits
// in `~/.easynet/auth.json`, every later auth-aware command picks
// it up automatically.
//
// What lives here:
//   * `LoginArgs::run`         — POST /api/v1/auth/register or /login
//   * `LogoutArgs::run`        — drop the saved token
//   * `WhoamiArgs::run`        — print current logged-in user
//   * `PairArgs::run`          — POST /api/v1/devices/pairing,
//                                returns a fresh pairing_token the
//                                operator can pipe into
//                                `easynet device join`
//   * On-disk `AuthSession`    — token + email + hub_url + uid,
//                                JSON file with file-mode 0600.
//
// Why a separate file from `groups/auth.rs`:
//   The `groups/` module owns Clap's user-facing arg structs +
//   subcommand surface. The `auth.rs` module owns the actual run
//   logic. This mirrors `join.rs` ↔ `groups/device.rs::Join` and
//   `start.rs` ↔ `groups/runtime.rs::Start`.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::persistence::config::{atomic_write_with_permissions, state_dir, WritePermissions};

const DEFAULT_HUB_URL: &str = "http://127.0.0.1:8080";
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Persisted auth session. Lives at `~/.easynet/auth.json` mode 0600.
/// Every auth-aware CLI command reads this to find the JWT bearer
/// token + the hub URL it was minted against.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    /// JWT access token. Bearer-prefixed when sent.
    pub token: String,
    /// Hub HTTP URL the token was minted against. Defaults to
    /// `http://127.0.0.1:8080` when login is run with no `--hub`.
    pub hub_url: String,
    /// Email used to log in. Helps `whoami` print something
    /// useful without an extra round trip.
    pub email: String,
    /// User UUID from the JWT. Sourced from `/auth/login`'s
    /// `user.id` field; used by `whoami`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Display nickname returned by the backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    /// Stable username slug used in canonical user/agent URIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

fn auth_session_path() -> PathBuf {
    state_dir().join("auth.json")
}

/// Load the persisted auth session. Returns Ok(None) when no
/// session is on disk; an error when the file exists but is
/// corrupt (caller should print a clear "your session is broken,
/// re-login" message).
pub fn load_session() -> anyhow::Result<Option<AuthSession>> {
    let path = auth_session_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => bail!("read {}: {e}", path.display()),
    };
    let session: AuthSession =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(session))
}

fn save_session(session: &AuthSession) -> anyhow::Result<()> {
    let dir = state_dir();
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(session)? + "\n";
    atomic_write_with_permissions(
        &auth_session_path(),
        json.as_bytes(),
        WritePermissions::OwnerReadWrite,
    )?;
    Ok(())
}

fn clear_session() -> anyhow::Result<()> {
    let path = auth_session_path();
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => bail!("remove {}: {e}", path.display()),
    }
}

// ── login ──────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Email address. Required.
    pub email: String,

    /// Password. If omitted, prompt interactively.
    #[arg(long)]
    pub password: Option<String>,

    /// Hub HTTP URL. Defaults to http://127.0.0.1:8080. Override
    /// for staging / production.
    #[arg(long, default_value = DEFAULT_HUB_URL)]
    pub hub: String,

    /// Register the user first if `/auth/register` accepts it,
    /// then log in. No-op when the email already exists. Useful
    /// for fresh dev rigs where the operator wants to avoid
    /// hand-running a separate register call.
    #[arg(long)]
    pub register_if_missing: bool,

    /// Nickname to use on register. Required when
    /// `--register-if-missing` is set and the email is new.
    /// Falls back to the local part of the email.
    #[arg(long)]
    pub nickname: Option<String>,
}

#[derive(Deserialize)]
struct AuthResp {
    token: String,
    #[serde(default)]
    user: Option<UserResp>,
}

#[derive(Deserialize)]
struct UserResp {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    nickname: Option<String>,
    #[serde(default)]
    username: Option<String>,
}

pub fn run_login(args: LoginArgs) -> anyhow::Result<()> {
    let password = match args.password.clone() {
        Some(p) => p,
        None => rpassword::prompt_password("Password: ").context("read password")?,
    };

    let hub = args.hub.trim_end_matches('/').to_string();

    // Try login first. Fall through to register only when
    // `--register-if-missing` AND the failure is a 401/404 that
    // looks like "no such user".
    let resp = post_login(&hub, &args.email, &password);
    let auth = match resp {
        Ok(r) => r,
        Err(e) => {
            if !args.register_if_missing {
                return Err(e);
            }
            eprintln!("login failed ({e}); attempting register...");
            let nickname = args
                .nickname
                .clone()
                .unwrap_or_else(|| args.email.split('@').next().unwrap_or("user").to_string());
            post_register(&hub, &args.email, &password, &nickname)?
        }
    };

    let user_id = auth.user.as_ref().and_then(|u| u.id.clone());
    let nickname = auth.user.as_ref().and_then(|u| u.nickname.clone());
    let username = auth.user.as_ref().and_then(|u| u.username.clone());
    let session = AuthSession {
        token: auth.token,
        hub_url: hub,
        email: args.email,
        user_id,
        nickname,
        username,
    };
    save_session(&session)?;
    println!("✓ logged in as {}", session.email);
    if let Some(uid) = &session.user_id {
        println!("  user_id: {uid}");
    }
    println!("  hub:     {}", session.hub_url);
    println!("  saved to {}", auth_session_path().display());
    Ok(())
}

fn post_login(hub: &str, email: &str, password: &str) -> anyhow::Result<AuthResp> {
    let url = format!("{hub}/api/v1/auth/login");
    let body = serde_json::json!({"email": email, "password": password});
    http_post_json(&url, &body)
}

fn post_register(
    hub: &str,
    email: &str,
    password: &str,
    nickname: &str,
) -> anyhow::Result<AuthResp> {
    let url = format!("{hub}/api/v1/auth/register");
    let body = serde_json::json!({
        "email": email,
        "password": password,
        "nickname": nickname,
    });
    http_post_json(&url, &body)
}

fn http_post_json<T: for<'de> Deserialize<'de>>(
    url: &str,
    body: &serde_json::Value,
) -> anyhow::Result<T> {
    let resp = ureq::post(url)
        .timeout(HTTP_TIMEOUT)
        .set("Content-Type", "application/json")
        .send_json(body.clone())
        .map_err(|e| match e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                anyhow!("HTTP {code} from {url}: {body}")
            }
            ureq::Error::Transport(e) => anyhow!("transport to {url}: {e}"),
        })?;
    let parsed: T = resp.into_json().context("parse response JSON")?;
    Ok(parsed)
}

// ── logout ─────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct LogoutArgs;

pub fn run_logout(_args: LogoutArgs) -> anyhow::Result<()> {
    if load_session()?.is_none() {
        println!("(no active session)");
        return Ok(());
    }
    clear_session()?;
    println!("✓ logged out (cleared {})", auth_session_path().display());
    Ok(())
}

// ── whoami ─────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct WhoamiArgs;

pub fn run_whoami(_args: WhoamiArgs) -> anyhow::Result<()> {
    match load_session()? {
        None => {
            println!("(not logged in — run `easynet auth login <email>`)");
            Ok(())
        }
        Some(s) => {
            println!("email:    {}", s.email);
            if let Some(uid) = &s.user_id {
                println!("user_id:  {uid}");
            }
            if let Some(username) = &s.username {
                println!("username: {username}");
            }
            if let Some(nick) = &s.nickname {
                println!("nickname: {nick}");
            }
            println!("hub:      {}", s.hub_url);
            Ok(())
        }
    }
}

// ── pair (mint pairing token) ──────────────────────────────────

#[derive(Debug, Args)]
pub struct PairArgs {
    /// Print only the raw pairing_token (no other fields). Useful
    /// for piping: `easynet auth pair --quiet | xargs easynet device join`.
    #[arg(long)]
    pub quiet: bool,
}

#[derive(Deserialize)]
struct PairingResp {
    pairing_token: String,
    #[serde(default)]
    realm: Option<String>,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    node_id: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
}

pub fn run_pair(args: PairArgs) -> anyhow::Result<()> {
    let session = load_session()?
        .ok_or_else(|| anyhow!("not logged in — run `easynet auth login <email>` first"))?;

    let url = format!("{}/api/v1/devices/pairing", session.hub_url);
    let resp: PairingResp = ureq::post(&url)
        .timeout(HTTP_TIMEOUT)
        .set("Authorization", &format!("Bearer {}", session.token))
        .set("Content-Type", "application/json")
        .send_json(serde_json::json!({}))
        .map_err(|e| match e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                if code == 401 {
                    anyhow!(
                        "HTTP 401 from {url}: token expired or invalid — run \
                         `easynet auth login <email>` to refresh.\nBody: {body}"
                    )
                } else {
                    anyhow!("HTTP {code} from {url}: {body}")
                }
            }
            ureq::Error::Transport(e) => anyhow!("transport to {url}: {e}"),
        })?
        .into_json()
        .context("parse pairing response")?;

    if args.quiet {
        println!("{}", resp.pairing_token);
        return Ok(());
    }

    println!("✓ pairing token minted");
    println!("  pairing_token: {}", resp.pairing_token);
    if let Some(realm) = &resp.realm {
        println!("  realm:         {realm}");
    }
    if let Some(ep) = &resp.endpoint {
        println!("  endpoint:      {ep}");
    }
    if let Some(node) = &resp.node_id {
        println!("  node_id:       {node}");
    }
    if let Some(exp) = resp.expires_in {
        println!("  expires_in:    {exp}s");
    }
    println!();
    println!("Next:");
    println!("  easynet device join {}", resp.pairing_token);
    Ok(())
}

// ── operator-mode HTTP commands ───────────────────────────────
//
// These mirror what the frontend's Devices / DeviceDetail pages
// fetch from /api/v1. They use the JWT cached at ~/.easynet/auth.json
// — same model as `gh repo list` after `gh auth login`.
//
// Why not under `easynet device` (the existing group)?
// `easynet device list` already exists and talks to the LOCAL
// daemon UDS via `fleet.list_nodes` (device-mode CLI: "what does
// THIS device see in its hub federation?"). Operator-mode HTTP
// is a different lens entirely: "what does the BACKEND know about
// the realm, viewed as the logged-in user?". Keeping the two
// surfaces separate avoids overloading verbs that already have a
// well-known meaning.

fn auth_get_json<T: for<'de> Deserialize<'de>>(path: &str) -> anyhow::Result<T> {
    let session = load_session()?
        .ok_or_else(|| anyhow!("not logged in — run `easynet auth login <email>` first"))?;
    let url = format!("{}{}", session.hub_url, path);
    let resp = ureq::get(&url)
        .timeout(HTTP_TIMEOUT)
        .set("Authorization", &format!("Bearer {}", session.token))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(401, _) => anyhow!(
                "HTTP 401 — token expired or invalid. Run `easynet auth login <email>` to refresh."
            ),
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                anyhow!("HTTP {code} from {url}: {body}")
            }
            ureq::Error::Transport(e) => anyhow!("transport to {url}: {e}"),
        })?;
    let parsed: T = resp.into_json().context("parse response JSON")?;
    Ok(parsed)
}

#[derive(Debug, Args)]
pub struct DevicesArgs {
    /// Output as JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Deserialize)]
struct DeviceListResp {
    items: Vec<DeviceItem>,
}

#[derive(Deserialize, Debug)]
struct DeviceItem {
    node_id: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    os: Option<String>,
    #[serde(default)]
    arch: Option<String>,
    #[serde(default)]
    realm: Option<String>,
}

pub fn run_devices(args: DevicesArgs) -> anyhow::Result<()> {
    let resp: DeviceListResp = auth_get_json("/api/v1/devices")?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "items": resp.items.iter().map(|d| serde_json::json!({
                    "node_id": d.node_id,
                    "display_name": d.display_name,
                    "state": d.state,
                    "os": d.os,
                    "arch": d.arch,
                    "realm": d.realm,
                })).collect::<Vec<_>>()
            }))?
        );
        return Ok(());
    }
    if resp.items.is_empty() {
        println!("(no devices — `easynet auth pair | xargs easynet device join` to attach one)");
        return Ok(());
    }
    println!(
        "{:<38} {:<10} {:<8} {:<10} {:<24}",
        "NODE_ID", "STATE", "OS", "ARCH", "DISPLAY_NAME"
    );
    for d in &resp.items {
        println!(
            "{:<38} {:<10} {:<8} {:<10} {:<24}",
            d.node_id,
            d.state.as_deref().unwrap_or("-"),
            d.os.as_deref().unwrap_or("-"),
            d.arch.as_deref().unwrap_or("-"),
            d.display_name.as_deref().unwrap_or("-"),
        );
    }
    Ok(())
}

#[derive(Debug, Args)]
pub struct AbilitiesArgs {
    /// Device node_id whose abilities to list.
    pub node_id: String,
    /// Output as JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Deserialize)]
struct AbilityListResp {
    items: Vec<AbilityItem>,
}

#[derive(Deserialize)]
struct AbilityItem {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

pub fn run_abilities(args: AbilitiesArgs) -> anyhow::Result<()> {
    let path = format!("/api/v1/devices/{}/abilities", args.node_id);
    let resp: AbilityListResp = auth_get_json(&path)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "items": resp.items.iter().map(|a| serde_json::json!({
                    "name": a.name,
                    "tool_name": a.tool_name,
                    "version": a.version,
                    "state": a.state,
                })).collect::<Vec<_>>()
            }))?
        );
        return Ok(());
    }
    if resp.items.is_empty() {
        println!(
            "(no abilities advertised by {} — daemon may be offline or no agents joined)",
            args.node_id
        );
        return Ok(());
    }
    println!(
        "{:<24} {:<24} {:<10} {:<10}",
        "NAME", "TOOL", "VERSION", "STATE"
    );
    for a in &resp.items {
        println!(
            "{:<24} {:<24} {:<10} {:<10}",
            a.name.as_deref().unwrap_or("-"),
            a.tool_name.as_deref().unwrap_or("-"),
            a.version.as_deref().unwrap_or("-"),
            a.state.as_deref().unwrap_or("-"),
        );
    }
    Ok(())
}

#[derive(Debug, Args)]
pub struct ExecArgs {
    /// Device node_id to run the command on.
    pub node_id: String,
    /// Shell command line. Wrap in quotes for whole shell strings:
    ///   easynet auth exec <node> -- "ls /tmp | head -5"
    /// Or pass tokens after `--`:
    ///   easynet auth exec <node> -- ls /tmp
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub cmd: Vec<String>,
    /// Ability tool name to invoke. Default `shell.run`. Override
    /// to `process.exec` (typed argv) or any other registered tool.
    #[arg(long, default_value = "shell.run")]
    pub tool: String,
    /// Timeout in milliseconds (default 30s).
    #[arg(long, default_value_t = 30_000)]
    pub timeout_ms: u32,
    /// Output as JSON (full backend receipt) instead of stdout / stderr.
    #[arg(long)]
    pub json: bool,
}

pub fn run_exec(args: ExecArgs) -> anyhow::Result<()> {
    if args.cmd.is_empty() {
        bail!("no command — usage: easynet auth exec <node_id> -- <cmd> [args ...]");
    }
    let session = load_session()?
        .ok_or_else(|| anyhow!("not logged in — run `easynet auth login <email>` first"))?;
    // Backend's POST /api/v1/abilities/invoke is what the frontend
    // uses for ad-hoc exec — `node_id` selects the target device,
    // `tool_name` picks the ability (shell.run / process.exec /
    // anything advertised by the device's daemon).
    let url = format!("{}/api/v1/abilities/invoke", session.hub_url);
    let arguments = match args.tool.as_str() {
        // shell.run takes a full command string.
        "shell.run" | "process.exec" => {
            serde_json::json!({"command": args.cmd.join(" ")})
        }
        // Anything else: pass cmd tokens as an argv array; the
        // ability handler can interpret as it sees fit.
        _ => serde_json::json!({"argv": args.cmd}),
    };
    let body = serde_json::json!({
        "tool_name": args.tool,
        "node_id": args.node_id,
        "arguments": arguments,
        "timeout_ms": args.timeout_ms,
    });
    let resp: serde_json::Value = ureq::post(&url)
        .timeout(Duration::from_millis(args.timeout_ms as u64 + 5_000))
        .set("Authorization", &format!("Bearer {}", session.token))
        .set("Content-Type", "application/json")
        .send_json(body)
        .map_err(|e| match e {
            ureq::Error::Status(401, _) => {
                anyhow!("HTTP 401 — token expired. Run `easynet auth login <email>` to refresh.")
            }
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                anyhow!("HTTP {code} from {url}: {body}")
            }
            ureq::Error::Transport(e) => anyhow!("transport to {url}: {e}"),
        })?
        .into_json()
        .context("parse invoke response")?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }
    // Human view: the shell.run / process.exec contract returns
    // {stdout, stderr, exit_code} inside `result`. Other tools
    // return whatever shape they want; for them, fall back to
    // pretty JSON since we have no contract.
    let result = resp
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let is_error = resp
        .get("is_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // shell.run / process.exec encode stdout/stderr as base64 in
    // their receipt body so binary output round-trips through JSON
    // safely. The CLI is a TTY consumer, so decode back to bytes
    // and stream raw — same shape the operator sees from a local
    // shell. Fall back to printing the raw string when the field
    // is something the encoder didn't base64 (best-effort: real
    // shell.run always emits b64).
    let stdout_raw = result.get("stdout").and_then(|v| v.as_str()).unwrap_or("");
    let stderr_raw = result.get("stderr").and_then(|v| v.as_str()).unwrap_or("");
    let stdout_bytes = B64
        .decode(stdout_raw.as_bytes())
        .unwrap_or_else(|_| stdout_raw.as_bytes().to_vec());
    let stderr_bytes = B64
        .decode(stderr_raw.as_bytes())
        .unwrap_or_else(|_| stderr_raw.as_bytes().to_vec());
    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let stderr = String::from_utf8_lossy(&stderr_bytes);
    let exit_code = result.get("exit_code").and_then(|v| v.as_i64());

    if !stdout.is_empty() {
        print!("{stdout}");
        if !stdout.ends_with('\n') {
            println!();
        }
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
        if !stderr.ends_with('\n') {
            eprintln!();
        }
    }
    if stdout.is_empty() && stderr.is_empty() && !result.is_null() {
        // Tool's payload doesn't follow shell.run contract; print pretty JSON.
        println!("{}", serde_json::to_string_pretty(&result)?);
    }
    if is_error {
        if let Some(err_obj) = resp.get("error") {
            eprintln!(
                "ability error: {}",
                serde_json::to_string(err_obj).unwrap_or_default()
            );
        }
    }
    if let Some(code) = exit_code {
        if code != 0 {
            std::process::exit(code as i32);
        }
    }
    Ok(())
}

// ── agents ─────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct AgentsArgs {
    /// Output as JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Deserialize)]
struct AgentListResp {
    items: Vec<serde_json::Value>,
}

pub fn run_agents(args: AgentsArgs) -> anyhow::Result<()> {
    let resp: AgentListResp = auth_get_json("/api/v1/agents")?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "items": resp.items,
            }))?
        );
        return Ok(());
    }
    if resp.items.is_empty() {
        println!("(no agents — daemon may be offline, or no hosted agents joined)");
        return Ok(());
    }
    // Backend's `/api/v1/agents` shape (listAgentsLogic.go) is
    // {agent_id, display_name, node_id, tags, skills:[...]} — no
    // top-level `uri` or `status` fields. The pre-fix renderer
    // looked for `uri` / `status` and printed `-` for every row,
    // which made every agent look offline / unidentified even when
    // the response carried real data. Render the device that hosts
    // each agent (NODE_ID) and the skill count so the operator
    // sees both identity and "what can this agent do".
    println!(
        "{:<60} {:<28} {:<38} {:>6}",
        "AGENT_ID", "DISPLAY_NAME", "NODE_ID", "SKILLS"
    );
    for a in &resp.items {
        let agent_id = a
            .get("agent_id")
            .or_else(|| a.get("uri"))
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let name = a
            .get("display_name")
            .or_else(|| a.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        let node_id = a.get("node_id").and_then(|v| v.as_str()).unwrap_or("-");
        let skills = a
            .get("skills")
            .and_then(|v| v.as_array())
            .map(|s| s.len())
            .unwrap_or(0);
        println!(
            "{:<60} {:<28} {:<38} {:>6}",
            agent_id, name, node_id, skills
        );
    }
    Ok(())
}

// ── device remove (HTTP-side) ─────────────────────────────────

#[derive(Debug, Args)]
pub struct DeviceRemoveArgs {
    /// Device node_id to remove.
    pub node_id: String,
    /// Skip the interactive confirmation.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

pub fn run_device_remove(args: DeviceRemoveArgs) -> anyhow::Result<()> {
    let session = load_session()?
        .ok_or_else(|| anyhow!("not logged in — run `easynet auth login <email>` first"))?;

    if !args.yes {
        eprint!("remove device {} from this realm? [y/N] ", args.node_id);
        use std::io::{self, BufRead, Write};
        io::stderr().flush().ok();
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line).ok();
        let trimmed = line.trim().to_lowercase();
        if trimmed != "y" && trimmed != "yes" {
            println!("(aborted)");
            return Ok(());
        }
    }

    let url = format!(
        "{}/api/v1/devices/{}",
        session.hub_url,
        urlencoding::encode(&args.node_id)
    );
    let resp_str = ureq::delete(&url)
        .timeout(HTTP_TIMEOUT)
        .set("Authorization", &format!("Bearer {}", session.token))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(401, _) => {
                anyhow!("HTTP 401 — token expired. Run `easynet auth login <email>` to refresh.")
            }
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                anyhow!("HTTP {code} from {url}: {body}")
            }
            ureq::Error::Transport(e) => anyhow!("transport to {url}: {e}"),
        })?
        .into_string()
        .unwrap_or_default();
    println!("✓ removed {}", args.node_id);
    if !resp_str.is_empty() {
        println!("  {resp_str}");
    }
    Ok(())
}

// ── events (SSE tail) ──────────────────────────────────────────

#[derive(Debug, Args)]
pub struct EventsArgs {
    /// Optional device node_id filter — show only events for this device.
    #[arg(long)]
    pub node: Option<String>,

    /// Stop after this many events. 0 = stream forever.
    #[arg(long, default_value_t = 0)]
    pub limit: usize,
}

pub fn run_events(args: EventsArgs) -> anyhow::Result<()> {
    let session = load_session()?
        .ok_or_else(|| anyhow!("not logged in — run `easynet auth login <email>` first"))?;

    // Backend's /api/v1/events accepts the JWT as a query param
    // (matches the frontend's EventSource which can't set headers).
    // SSE wire shape: lines of `data: <json>\n\n`, one event per
    // double-newline. We use ureq with a streaming reader rather
    // than a real SSE client because the dependency footprint is
    // smaller and we only need the happy-path tail.
    let url = format!("{}/api/v1/events?token={}", session.hub_url, session.token);
    // No request timeout: SSE stream is long-lived; rely on
    // transport defaults for connect.
    let resp = ureq::get(&url)
        .set("Accept", "text/event-stream")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(401, _) => {
                anyhow!("HTTP 401 — token expired. Run `easynet auth login <email>` to refresh.")
            }
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                anyhow!("HTTP {code} from {url}: {body}")
            }
            ureq::Error::Transport(e) => anyhow!("transport to {url}: {e}"),
        })?;

    use std::io::BufRead;
    let reader = std::io::BufReader::new(resp.into_reader());
    let mut count = 0usize;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("(stream read error: {e})");
                break;
            }
        };
        // SSE frames: `data: <payload>` lines, blank line ends event.
        // We just print the data payload as-is.
        if let Some(payload) = line.strip_prefix("data: ") {
            // Optional node filter.
            if let Some(want) = &args.node {
                if !payload.contains(want) {
                    continue;
                }
            }
            println!("{payload}");
            count += 1;
            if args.limit > 0 && count >= args.limit {
                break;
            }
        }
    }
    Ok(())
}
