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

use crate::facade::cli::ability_catalog_row::AbilityCatalogueRow;
use crate::persistence::config::{
    self, atomic_write_with_permissions, state_dir, WritePermissions,
};
use crate::support::output;
use crate::ura;

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
    /// Stable username slug used in canonical user/agent URAs.
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

    /// Register the user first if '/auth/register' accepts it,
    /// then log in. No-op when the email already exists. Useful
    /// for fresh dev rigs where the operator wants to avoid
    /// hand-running a separate register call.
    #[arg(long)]
    pub register_if_missing: bool,

    /// Nickname to use on register. Required when
    /// '--register-if-missing' is set and the email is new.
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

/// `whoami` answers "who am I to EasyNet right now?". There are
/// two layers of identity:
///
/// - **User session** (`~/.easynet/auth.json`) — the human-level
///   identity, established by `easynet auth login <email>`. Carries
///   email / user_id / username / nickname.
/// - **Device pairing** (`~/.easynet/credentials.json`) — the
///   machine-level identity, established by `easynet device join
///   <token>`. Carries node_id, realm, and (post-Phase 14)
///   `username` of the user this device is paired to.
///
/// Per RFC-001 §3.2, a device is a first-class agent; a paired
/// device implicitly carries its owner's identity. So a host that
/// has run `device join` but not `auth login` still has a usable
/// identity for most operations — `easynet runtime start` / `easynet ability
/// invoke` etc. don't require an interactive auth session, only
/// the device credentials.
///
/// The reporting precedence:
/// 1. Auth session present → render user-level identity.
/// 2. No session but paired → render device-level identity (device
///    URA, hub URA, paired username if known).
/// 3. Neither → tell the user what to do.
pub fn run_whoami(_args: WhoamiArgs) -> anyhow::Result<()> {
    let session = load_session()?;
    let creds = config::load_credentials().ok();

    match (session, creds) {
        (Some(s), creds) => {
            // Layer 1 — full user session. We still surface the
            // device URA when paired so the user knows which
            // machine identity is wired to this account from this
            // host. Rendered through `kv_section_stdout` for the
            // same bold-cyan label / vertically-aligned look as
            // banner and `runtime status`.
            let device_ura = creds
                .as_ref()
                .map(|c| ura::device_ura(c.realm_str(), &c.node_id));
            let mut rows: Vec<(&str, &str)> = vec![("email", s.email.as_str())];
            if let Some(uid) = s.user_id.as_deref() {
                rows.push(("user_id", uid));
            }
            if let Some(username) = s.username.as_deref() {
                rows.push(("username", username));
            }
            if let Some(nick) = s.nickname.as_deref() {
                rows.push(("nickname", nick));
            }
            rows.push(("hub", s.hub_url.as_str()));
            if let Some(d) = device_ura.as_deref() {
                rows.push(("device", d));
            }
            output::kv_section_stdout(&rows);
            Ok(())
        }
        (None, Some(c)) => {
            // Layer 2 — paired but no interactive auth session.
            // This is the common state on a freshly-joined device:
            // the user typed `easynet device join <token>` and
            // never bothered with `auth login`. They DO have an
            // identity — the device URA — and most CLI commands
            // accept it. Saying "not logged in" without explaining
            // the device pairing was confusing (silan flagged this
            // when `easynet runtime start` worked despite `whoami` saying
            // not logged in).
            let realm = c.realm_str().to_string();
            let hub_ura = ura::hub_ura(&realm);
            let device_ura = ura::device_ura(&realm, &c.node_id);
            let user_ura = c
                .username
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|u| ura::user_ura(&realm, u));
            println!("(no interactive auth session on this host)");
            println!("paired as a device:");
            let mut rows: Vec<(&str, &str)> = vec![("Hub", hub_ura.as_str())];
            if let Some(ref u) = user_ura {
                rows.push(("Current user", u.as_str()));
            }
            rows.push(("Current device", device_ura.as_str()));
            rows.push(("Realm", realm.as_str()));
            output::kv_section_stdout(&rows);
            println!();
            println!(
                "Most commands work with the device pairing. \
                 Run 'easynet auth login <email>' to attach a \
                 user-level session for ops that require it."
            );
            Ok(())
        }
        (None, None) => {
            println!("(not logged in and not paired)");
            println!("  • run 'easynet auth login <email>' for a user session, OR");
            println!("  • run 'easynet device join <token>' to pair this device.");
            Ok(())
        }
    }
}

// ── pair (mint pairing token) ──────────────────────────────────

#[derive(Debug, Args)]
pub struct PairArgs {
    /// Print only the raw pairing_token (no other fields). Useful
    /// for piping: 'easynet auth pair --quiet | xargs easynet device join'.
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
        .ok_or_else(|| anyhow!("not logged in — run 'easynet auth login <email>' first"))?;

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
// daemon UDS via `node.list` (device-mode CLI: "what does
// THIS device see in its hub federation?"). Operator-mode HTTP
// is a different lens entirely: "what does the BACKEND know about
// the realm, viewed as the logged-in user?". Keeping the two
// surfaces separate avoids overloading verbs that already have a
// well-known meaning.

fn auth_get_json<T: for<'de> Deserialize<'de>>(path: &str) -> anyhow::Result<T> {
    let session = load_session()?
        .ok_or_else(|| anyhow!("not logged in — run 'easynet auth login <email>' first"))?;
    let url = format!("{}{}", session.hub_url, path);
    let resp = ureq::get(&url)
        .timeout(HTTP_TIMEOUT)
        .set("Authorization", &format!("Bearer {}", session.token))
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(401, _) => anyhow!(
                "HTTP 401 — token expired or invalid. Run 'easynet auth login <email>' to refresh."
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
        println!("(no devices — 'easynet auth pair | xargs easynet device join' to attach one)");
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
    ability_ura: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

pub fn run_abilities(args: AbilitiesArgs) -> anyhow::Result<()> {
    let session = load_session()?
        .ok_or_else(|| anyhow!("not logged in — run 'easynet auth login <email>' first"))?;
    let path = format!("/api/v1/devices/{}/abilities", args.node_id);
    let url = format!("{}{}", session.hub_url, path);
    let resp: AbilityListResp = match ureq::get(&url)
        .timeout(HTTP_TIMEOUT)
        .set("Authorization", &format!("Bearer {}", session.token))
        .call()
    {
        Ok(resp) => resp.into_json().context("parse response JSON")?,
        Err(ureq::Error::Status(401, _)) => bail!(
            "HTTP 401 — token expired or invalid. Run 'easynet auth login <email>' to refresh."
        ),
        Err(ureq::Error::Status(404, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            fallback_device_abilities_from_local_daemon(&args.node_id).map_err(|fallback_err| {
                anyhow!(
                    "HTTP 404 from {url}: {body}\nlocal federation fallback failed: {fallback_err}"
                )
            })?
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            bail!("HTTP {code} from {url}: {body}");
        }
        Err(ureq::Error::Transport(e)) => bail!("transport to {url}: {e}"),
    };
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "items": resp.items.iter().map(|a| serde_json::json!({
                    "name": a.name,
                    "ability_ura": a.ability_ura,
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
        "{:<24} {:<64} {:<10} {:<10}",
        "LABEL", "ABILITY_URA", "VERSION", "STATE"
    );
    for a in &resp.items {
        println!(
            "{:<24} {:<64} {:<10} {:<10}",
            a.name.as_deref().unwrap_or("-"),
            a.ability_ura.as_deref().unwrap_or("-"),
            a.version.as_deref().unwrap_or("-"),
            a.state.as_deref().unwrap_or("-"),
        );
    }
    Ok(())
}

/// Joint-plan unified path: when the backend HTTP API can't find a
/// device (typically cross-hub), fall back to the daemon's
/// `node.describe` ability — the same surface
/// `easynet device show` uses post-phase-1.2.
///
/// Routing matches the rest of the CLI:
///   * `node_id` matches this device's own node id → invoke
///     `node.describe` locally over the control socket.
///   * Otherwise → resolve the target as a canonical device URA:
///     cross-hub directory hit first, local-realm fallback second,
///     then forward_invoke `node.describe`.
///     The backend HTTP API surface only exposes bare uuid today,
///     but a canonical URA still lands here correctly if passed by
///     a future caller.
///
/// The legacy `node.describe` path handled both arms server-
/// side; the daemon-side handler is on the phase 4 cull list. This
/// helper preserves the operator-visible behaviour while the
/// dependency moves.
fn fallback_device_abilities_from_local_daemon(node_id: &str) -> anyhow::Result<AbilityListResp> {
    let node = describe_node_via_unified_path(node_id)
        .with_context(|| format!("invoke node.describe for node {node_id}"))?;
    let abilities = node
        .get("abilities")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("node.describe returned no 'abilities' array"))?;
    let items = abilities
        .iter()
        .map(ability_item_from_descriptor)
        .collect::<Vec<_>>();
    Ok(AbilityListResp { items })
}

fn describe_node_via_unified_path(node_id: &str) -> anyhow::Result<serde_json::Value> {
    let trimmed = node_id.trim();
    let creds = crate::persistence::config::load_credentials().ok();
    let local_node = creds
        .as_ref()
        .map(|c| c.node_id.clone())
        .unwrap_or_default();
    let local_tenant = creds.as_ref().map(|c| c.realm.clone()).unwrap_or_default();

    let is_local = !local_node.is_empty() && trimmed == local_node;
    if is_local {
        return crate::support::local_invoke::invoke_local_ability(
            "node.describe",
            serde_json::json!({"node_id": "local"}),
        )
        .context("invoke node.describe (local)");
    }

    describe_node_remote(trimmed, &local_tenant)
}

#[cfg(feature = "axon-pb")]
fn describe_node_remote(node: &str, local_tenant: &str) -> anyhow::Result<serde_json::Value> {
    let _ = local_tenant;
    let target_ura = crate::support::remote_device::resolve_target_device_ura(node)?;
    let caller_ura = crate::support::remote_device::caller_device_ura_from_credentials();
    let target_call = crate::services::invocation_transport::federation_invoke::RemoteAbilityInvocationTarget::for_target_owned_selector(
        &target_ura,
        "node.describe",
    )?;
    crate::services::invocation_transport::federation_invoke::invoke_via_federation_forward_target(
        &target_call,
        serde_json::json!({"node_id": "local"}),
        caller_ura.as_deref(),
    )
    .with_context(|| format!("forward node.describe to target={target_ura}"))
}

#[cfg(not(feature = "axon-pb"))]
fn describe_node_remote(node: &str, _local_tenant: &str) -> anyhow::Result<serde_json::Value> {
    Err(crate::support::local_invoke::federation_not_wired_error(
        &format!("describing remote device {node:?}"),
    ))
}

fn ability_item_from_descriptor(value: &serde_json::Value) -> AbilityItem {
    let row = AbilityCatalogueRow::from_value(value);
    AbilityItem {
        name: Some(row.label().to_string()),
        ability_ura: row.ability_ura().map(str::to_string),
        version: row.version().map(str::to_string),
        state: Some(row.state().to_string()),
    }
}

#[derive(Debug, Args)]
pub struct ExecArgs {
    /// Device node_id to run the command on.
    pub node_id: String,
    /// Shell command line. Wrap in quotes for whole shell strings:
    ///   easynet auth exec <node> -- "ls /tmp | head -5"
    /// Or pass tokens after '--':
    ///   easynet auth exec <node> -- ls /tmp
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub cmd: Vec<String>,
    /// Canonical advertised ability tool name to invoke. Default
    /// 'shell.run'. Override to 'process.exec'
    /// (typed argv) or any other registered tool.
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
    let tool_name = canonical_auth_exec_tool_name(&args.tool)?.to_string();
    let session = load_session()?
        .ok_or_else(|| anyhow!("not logged in — run 'easynet auth login <email>' first"))?;
    // Backend's POST /api/v1/abilities/invoke is what the frontend
    // uses for ad-hoc exec — `node_id` selects the target device,
    // `tool_name` picks the canonical ability advertised by the
    // device's daemon.
    let url = format!("{}/api/v1/abilities/invoke", session.hub_url);
    let arguments = match tool_name.as_str() {
        // shell.run / process.exec take a full command string.
        "shell.run" | "process.exec" => {
            serde_json::json!({"command": args.cmd.join(" ")})
        }
        // Anything else: pass cmd tokens as an argv array; the
        // ability handler can interpret as it sees fit.
        _ => serde_json::json!({"argv": args.cmd}),
    };
    let body = serde_json::json!({
        "tool_name": tool_name,
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
                anyhow!("HTTP 401 — token expired. Run 'easynet auth login <email>' to refresh.")
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

fn canonical_auth_exec_tool_name(raw: &str) -> anyhow::Result<&str> {
    let tool = raw.trim();
    if tool.is_empty() {
        bail!("--tool must be a canonical advertised ability name, e.g. shell.run");
    }
    if tool.starts_with("device.") {
        bail!("--tool must use the public advertised ability name; use shell.run or process.exec");
    }
    Ok(tool)
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
    // top-level `ura` or `status` fields. The pre-fix renderer
    // looked for `ura` / `status` and printed `-` for every row,
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
            .or_else(|| a.get("ura"))
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
        .ok_or_else(|| anyhow!("not logged in — run 'easynet auth login <email>' first"))?;

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
                anyhow!("HTTP 401 — token expired. Run 'easynet auth login <email>' to refresh.")
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
        .ok_or_else(|| anyhow!("not logged in — run 'easynet auth login <email>' first"))?;

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
                anyhow!("HTTP 401 — token expired. Run 'easynet auth login <email>' to refresh.")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ability_item_projection_maps_ura_descriptor_shape() {
        let item = ability_item_from_descriptor(&serde_json::json!({
            "name": "shell.run",
            "ability_ura": "easynet:///r/test/ability/device.dev-1.shell.run",
            "ability_version": "1",
        }));
        assert_eq!(item.name.as_deref(), Some("shell.run"));
        assert_eq!(
            item.ability_ura.as_deref(),
            Some("easynet:///r/test/ability/device.dev-1.shell.run")
        );
        assert_eq!(item.version.as_deref(), Some("1"));
        assert_eq!(item.state.as_deref(), Some("ACTIVE"));
    }

    #[test]
    fn auth_exec_tool_name_accepts_canonical_device_tool() {
        assert_eq!(
            canonical_auth_exec_tool_name(" shell.run ").unwrap(),
            "shell.run"
        );
        assert_eq!(
            canonical_auth_exec_tool_name("process.exec").unwrap(),
            "process.exec"
        );
    }

    #[test]
    fn auth_exec_tool_name_rejects_retired_device_owner_prefix() {
        let err = canonical_auth_exec_tool_name("device.shell.run").unwrap_err();
        assert!(
            err.to_string().contains("shell.run"),
            "error should name the canonical shell tool: {err}"
        );

        let err = canonical_auth_exec_tool_name("device.process.exec").unwrap_err();
        assert!(
            err.to_string().contains("process.exec"),
            "error should name the canonical process tool: {err}"
        );
    }

    #[test]
    fn auth_exec_tool_name_rejects_empty_tool_name() {
        let err = canonical_auth_exec_tool_name("   ").unwrap_err();
        assert!(
            err.to_string()
                .contains("canonical advertised ability name"),
            "error should explain the canonical tool-name contract: {err}"
        );
    }
}
