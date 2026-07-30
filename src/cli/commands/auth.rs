// EasyNet CLI — auth: HTTP-driven backend session management
// =============================================================
//
// File: src/cli/commands/auth.rs
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

use crate::cli::presentation::identity::runtime_user_binding_display;
use crate::core::ura;
use crate::daemon::persistence::config::{
    self, atomic_write_with_permissions, state_dir, WritePermissions,
};
use crate::support::platform::output;

pub const DEFAULT_HUB_URL: &str = "http://127.0.0.1:8080";
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

/// Persisted auth session. Lives at `~/.easynet/auth.json` mode 0600.
/// Every auth-aware CLI command reads this to find the JWT bearer
/// token + the hub URL it was minted against.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthSession {
    /// JWT access token. Bearer-prefixed when sent.
    pub token: String,
    /// Refresh token returned by the backend. Keeping the refresh credential
    /// with the access credential lets every CLI HTTP operation use the same
    /// authenticated session instead of failing after the access JWT expires.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Hub HTTP URL the token was minted against. Defaults to
    /// `http://127.0.0.1:8080` when login is run with no `--hub`.
    pub hub_url: String,
    /// Email used to log in. Helps `whoami` print something
    /// useful without an extra round trip.
    pub email: String,
    /// User UUID from the JWT. Sourced from `/auth/login`'s
    /// `user.id` field; used by `whoami`.
    pub user_id: String,
    /// Display nickname returned by the backend.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    /// Stable username slug used in canonical user/agent URAs.
    pub username: String,
}

impl AuthSession {
    fn validated(self) -> anyhow::Result<Self> {
        validate_non_blank("token", &self.token)?;
        validate_non_blank("hub_url", &self.hub_url)?;
        validate_non_blank("email", &self.email)?;
        validate_non_blank("user_id", &self.user_id)?;
        if crate::core::identity::is_all_zero_principal_id(&self.user_id) {
            bail!("auth session carries all-zero user_id — run `easynet login <user>@<realm>`");
        }
        validate_non_blank("username", &self.username)?;
        Ok(self)
    }
}

fn validate_non_blank(field: &str, value: &str) -> anyhow::Result<()> {
    if value.trim().is_empty() {
        bail!("auth session {field} must not be blank");
    }
    Ok(())
}

pub(crate) fn auth_session_path() -> PathBuf {
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
    let session = session
        .validated()
        .with_context(|| format!("validate {}", path.display()))?;
    Ok(Some(session))
}

pub(crate) fn save_session(session: &AuthSession) -> anyhow::Result<()> {
    session
        .clone()
        .validated()
        .context("validate auth session before save")?;
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

pub(crate) fn clear_session() -> anyhow::Result<()> {
    let path = auth_session_path();
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => bail!("remove {}: {e}", path.display()),
    }
}

fn authenticated_session() -> anyhow::Result<AuthSession> {
    let session = load_session()?
        .ok_or_else(|| anyhow!("not logged in — run 'easynet auth login <email>' first"))?;
    if let Ok(credentials) = config::load_credentials() {
        if let Ok(device_user_id) = credentials.user_id() {
            let session_user_id = session.user_id.trim();
            if session_user_id != device_user_id {
                bail!(
                    "authenticated user {session_user_id} does not own paired device {} (owner {device_user_id}); log in as the paired owner or rejoin the device",
                    credentials.node_id
                );
            }
        }
    }
    Ok(session)
}

fn refresh_session(session: &mut AuthSession) -> anyhow::Result<()> {
    let refresh_token = session
        .refresh_token
        .as_deref()
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| anyhow!("access token expired and no refresh token is stored; run 'easynet auth login <email>'"))?;
    let url = format!("{}/api/v1/auth/refresh", session.hub_url);
    let auth: RefreshResp =
        http_post_json(&url, &serde_json::json!({"refresh_token": refresh_token}))?;
    session.token = auth.token;
    if auth.refresh_token.is_some() {
        session.refresh_token = auth.refresh_token;
    }
    save_session(session)
}

fn auth_post_json<T: for<'de> Deserialize<'de>>(
    path: &str,
    body: &serde_json::Value,
    timeout: Duration,
) -> anyhow::Result<T> {
    let mut session = authenticated_session()?;
    let url = format!("{}{}", session.hub_url, path);
    let mut refreshed = false;
    loop {
        let result = ureq::post(&url)
            .timeout(timeout)
            .set("Authorization", &format!("Bearer {}", session.token))
            .set("Content-Type", "application/json")
            .send_json(body.clone());
        match result {
            Ok(response) => return response.into_json().context("parse response JSON"),
            Err(ureq::Error::Status(401, response)) if !refreshed => {
                let _ = response.into_string();
                refresh_session(&mut session)?;
                refreshed = true;
            }
            Err(ureq::Error::Status(401, response)) => {
                let body = response.into_string().unwrap_or_default();
                bail!("HTTP 401 from {url}: token expired or invalid: {body}");
            }
            Err(ureq::Error::Status(code, response)) => {
                let body = response.into_string().unwrap_or_default();
                bail!("HTTP {code} from {url}: {body}");
            }
            Err(ureq::Error::Transport(error)) => bail!("transport to {url}: {error}"),
        }
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

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthResp {
    token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    user: UserResp,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UserResp {
    id: String,
    #[serde(default)]
    account_key: Option<String>,
    #[serde(default)]
    ura: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    phone: Option<String>,
    #[serde(default)]
    nickname: Option<String>,
    username: String,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default)]
    passkey_public_key_count: Option<i64>,
    #[serde(default)]
    account_public_keys: Option<Vec<AccountPublicKeyResp>>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountPublicKeyResp {
    id: String,
    name: String,
    credential_id: String,
    public_key: String,
    fingerprint: String,
    backed_up: bool,
    created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RefreshResp {
    token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

pub fn run_login(args: LoginArgs) -> anyhow::Result<()> {
    let session = login_and_save(args)?;
    render_login_success(&session);
    Ok(())
}

pub(crate) fn login_and_save(args: LoginArgs) -> anyhow::Result<AuthSession> {
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

    let session = AuthSession {
        token: auth.token,
        refresh_token: auth.refresh_token,
        hub_url: hub,
        email: args.email,
        user_id: auth.user.id,
        nickname: auth.user.nickname,
        username: auth.user.username,
    };
    save_session(&session)?;
    Ok(session)
}

pub(crate) fn render_login_success(session: &AuthSession) {
    println!("✓ logged in as {}", session.email);
    println!("  user_id: {}", session.user_id);
    println!("  hub:     {}", session.hub_url);
    println!("  saved to {}", auth_session_path().display());
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
    let creds = config::load_credentials_optional()
        .context("load device credentials for identity projection")?;

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
            let mut rows: Vec<(&str, &str)> = vec![
                ("email", s.email.as_str()),
                ("user_id", s.user_id.as_str()),
                ("username", s.username.as_str()),
            ];
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
            println!("(no interactive auth session on this host)");
            println!("paired as a device:");
            let mut rows: Vec<(&str, &str)> = vec![("Hub", hub_ura.as_str())];
            let user_binding = runtime_user_binding_display(&c);
            rows.push(("Current user", user_binding.value()));
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PairingResp {
    pub pairing_token: String,
    #[serde(default)]
    pub realm: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
}

pub(crate) fn mint_pairing_token() -> anyhow::Result<PairingResp> {
    auth_post_json(
        "/api/v1/devices/pairing",
        &serde_json::json!({}),
        HTTP_TIMEOUT,
    )
}

pub fn run_pair(args: PairArgs) -> anyhow::Result<()> {
    let resp = mint_pairing_token()?;

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
// daemon through the canonical federation discovery path
// (device-mode CLI: "what does THIS device see in its hub federation?").
// Operator-mode HTTP
// is a different lens entirely: "what does the BACKEND know about
// the realm, viewed as the logged-in user?". Keeping the two
// surfaces separate avoids overloading verbs that already have a
// well-known meaning.

fn auth_get_json<T: for<'de> Deserialize<'de>>(path: &str) -> anyhow::Result<T> {
    let mut session = authenticated_session()?;
    let url = format!("{}{}", session.hub_url, path);
    let mut refreshed = false;
    loop {
        match ureq::get(&url)
            .timeout(HTTP_TIMEOUT)
            .set("Authorization", &format!("Bearer {}", session.token))
            .call()
        {
            Ok(response) => return response.into_json().context("parse response JSON"),
            Err(ureq::Error::Status(401, response)) if !refreshed => {
                let _ = response.into_string();
                refresh_session(&mut session)?;
                refreshed = true;
            }
            Err(ureq::Error::Status(401, response)) => {
                let body = response.into_string().unwrap_or_default();
                bail!("HTTP 401 from {url}: token expired or invalid: {body}");
            }
            Err(ureq::Error::Status(code, response)) => {
                let body = response.into_string().unwrap_or_default();
                bail!("HTTP {code} from {url}: {body}");
            }
            Err(ureq::Error::Transport(error)) => bail!("transport to {url}: {error}"),
        }
    }
}

#[derive(Debug, Args)]
pub struct DevicesArgs {
    /// Output as JSON instead of a table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeviceListResp {
    items: Vec<DeviceItem>,
    #[serde(default)]
    resolve_unavailable: Vec<ResolveUnavailable>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    trust_level: Option<String>,
    #[serde(default)]
    device_group: Option<String>,
    #[serde(default)]
    auth_binding: Option<String>,
    #[serde(default)]
    credential_provisioned: Option<bool>,
    #[serde(default)]
    public_key_registered: Option<bool>,
    #[serde(default)]
    device_public_key: Option<String>,
    #[serde(default)]
    device_public_key_fingerprint: Option<String>,
    #[serde(default)]
    credential_token: Option<String>,
    #[serde(default)]
    hub_endpoint: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    user_id: Option<String>,
    #[serde(default)]
    deploy_signature: Option<String>,
    #[serde(default)]
    federated_peers: Vec<FederatedPeerEntry>,
    #[serde(default)]
    ura: Option<String>,
    #[serde(default)]
    last_seen_unix_ms: Option<i64>,
    #[serde(default)]
    resolve_unavailable: Vec<ResolveUnavailable>,
    #[serde(default)]
    state_code: Option<String>,
    #[serde(default)]
    transition_id: Option<String>,
    #[serde(default)]
    interrupted_transition: Option<String>,
    #[serde(default)]
    failure: Option<ConnectionFailure>,
}

#[derive(Deserialize, Debug, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct FederatedPeerEntry {
    realm: String,
    peer_hub_url: String,
    #[serde(default)]
    peer_hub_pubkey: Option<String>,
}

#[derive(Deserialize, Debug, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ResolveUnavailable {
    source: String,
    reason: String,
    #[serde(default)]
    query_name: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    stage: Option<String>,
    #[serde(default)]
    retryable: Option<bool>,
    #[serde(default)]
    retry_after_unix_ms: Option<i64>,
}

#[derive(Deserialize, Debug, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ConnectionFailure {
    code: String,
    message: String,
    stage: String,
    retryable: bool,
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
                    "trust_level": d.trust_level,
                    "device_group": d.device_group,
                    "os": d.os,
                    "arch": d.arch,
                    "realm": d.realm,
                    "auth_binding": d.auth_binding,
                    "credential_provisioned": d.credential_provisioned,
                    "public_key_registered": d.public_key_registered,
                    "device_public_key": d.device_public_key,
                    "device_public_key_fingerprint": d.device_public_key_fingerprint,
                    "credential_token": d.credential_token,
                    "hub_endpoint": d.hub_endpoint,
                    "username": d.username,
                    "user_id": d.user_id,
                    "deploy_signature": d.deploy_signature,
                    "ura": d.ura,
                    "last_seen_unix_ms": d.last_seen_unix_ms,
                    "state_code": d.state_code,
                    "transition_id": d.transition_id,
                    "interrupted_transition": d.interrupted_transition,
                    "failure": d.failure,
                    "resolve_unavailable": d.resolve_unavailable,
                    "federated_peers": d.federated_peers,
                })).collect::<Vec<_>>(),
                "resolve_unavailable": resp.resolve_unavailable,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AbilityListResp {
    items: Vec<AbilityItem>,
    #[serde(default)]
    resolve_unavailable: Vec<ResolveUnavailable>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AbilityItem {
    #[serde(default)]
    ura: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    ability_ura: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    install_id: Option<String>,
    #[serde(default)]
    owner_ura: Option<String>,
}

pub fn run_abilities(args: AbilitiesArgs) -> anyhow::Result<()> {
    let path = format!("/api/v1/devices/{}/abilities", args.node_id);
    let resp: AbilityListResp = auth_get_json(&path)?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "items": resp.items.iter().map(|a| serde_json::json!({
                    "ura": a.ura,
                    "name": a.name,
                    "tool_name": a.tool_name,
                    "ability_ura": a.ability_ura,
                    "version": a.version,
                    "category": a.category,
                    "description": a.description,
                    "state": a.state,
                    "install_id": a.install_id,
                    "owner_ura": a.owner_ura,
                })).collect::<Vec<_>>(),
                "resolve_unavailable": resp.resolve_unavailable,
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
    // Backend's POST /api/v1/abilities/invoke is what the frontend
    // uses for ad-hoc exec — `node_id` selects the target device,
    // `tool_name` picks the canonical ability advertised by the
    // device's daemon.
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
    let resp: serde_json::Value = auth_post_json(
        "/api/v1/abilities/invoke",
        &body,
        Duration::from_millis(args.timeout_ms as u64 + 5_000),
    )?;
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentListResp {
    items: Vec<AgentItem>,
    #[serde(default)]
    resolve_unavailable: Vec<ResolveUnavailable>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct AgentItem {
    agent_id: String,
    display_name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    runtime: Option<String>,
    #[serde(default)]
    base_runtime: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    base_model: Option<String>,
    tags: Vec<String>,
    node_id: String,
    #[serde(default)]
    host_device_ura: Option<String>,
    #[serde(default)]
    skills: Vec<SkillInfo>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct SkillInfo {
    skill_id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentTableProjection {
    agent_id: String,
    display_name: String,
    node_id: String,
    skill_count: usize,
}

impl AgentTableProjection {
    fn from_backend_row(row: &AgentItem) -> Self {
        Self {
            agent_id: row.agent_id.clone(),
            display_name: row.display_name.clone(),
            node_id: row.node_id.clone(),
            skill_count: row.skills.len(),
        }
    }
}

pub fn run_agents(args: AgentsArgs) -> anyhow::Result<()> {
    let resp: AgentListResp = auth_get_json("/api/v1/agents")?;
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "items": resp.items,
                "resolve_unavailable": resp.resolve_unavailable,
            }))?
        );
        return Ok(());
    }
    if resp.items.is_empty() {
        println!("(no agents — daemon may be offline, or no hosted agents joined)");
        return Ok(());
    }
    // Backend's `/api/v1/agents` shape (listAgentsLogic.go) is the
    // canonical table contract: {agent_id, display_name, node_id, tags,
    // skills:[...]}. The CLI table projects that shape directly and does not
    // repair retired row aliases into identity facts.
    println!(
        "{:<60} {:<28} {:<38} {:>6}",
        "AGENT_ID", "DISPLAY_NAME", "NODE_ID", "SKILLS"
    );
    for a in &resp.items {
        let row = AgentTableProjection::from_backend_row(a);
        println!(
            "{:<60} {:<28} {:<38} {:>6}",
            row.agent_id, row.display_name, row.node_id, row.skill_count
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

// ── user signing-key register ──────────────────────────────────
//
// Wrapper→CLI thesis: a user signing key is a long-lived identity
// credential, so the DAEMON (the runtime trust source) owns it — not the
// browser and not the product backend. This command is the CLI facade for
// that: it asks the daemon key service to create or project a managed user
// signing key and registers that public projection through
// the daemon's `identity.register_pubkey` ability directly
// over the local UDS socket. The backend's `POST /me/signing-keys` is the
// browser's equivalent facade onto the same daemon ability — neither one
// is the source of truth.
//
// The local UDS invoke is admitted as the loopback `_system.local` caller
// (not a device URA), which the daemon's identity-write gate exempts, so a
// `role:"user"` write is authorized without a delegation proof.
//
// A user may hold MULTIPLE signing keys (the trust anchor keys on
// `(user_ura, public_key_b64)`), so the CLI host's key coexists with the
// browser's IndexedDB key. This command therefore enables CLI-driven
// user-as-caller invocation; it does NOT retro-register the browser's key.

#[derive(Debug, Args)]
pub struct SigningKeyRegisterArgs {
    /// Print the registered public key (base64) on success.
    #[arg(long)]
    pub show_pubkey: bool,
}

pub fn run_signing_key_register(args: SigningKeyRegisterArgs) -> anyhow::Result<()> {
    // The user URA's realm MUST equal the daemon realm (the trust-anchor
    // writer pins user-row realm to the daemon realm; only device rows may
    // be cross-realm). The credentials realm is the realm this host paired
    // into, i.e. the co-located daemon's realm.
    let creds = config::load_credentials()
        .context("load device credentials (run `easynet device join <token>` first)")?;
    let user_ura = creds.user_ura()?;

    let outcome = super::user_signing_identity::reconcile_local_user_signing_identity(&user_ura)?;

    output::success(&format!("Registered user signing key for {user_ura}"));
    if args.show_pubkey {
        println!("{}", outcome.public_key_b64);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;

    fn write_auth_session(body: &str) {
        std::fs::create_dir_all(config::state_dir()).expect("state dir");
        std::fs::write(auth_session_path(), body).expect("write auth session");
    }

    fn assert_auth_session_failure_contains(error: anyhow::Error, expected: &str) {
        let message = format!("{error:#}");
        assert!(
            message.contains(expected),
            "expected {expected:?} in auth session error: {message}"
        );
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

    #[test]
    fn auth_agents_table_uses_canonical_backend_fields() {
        let response = serde_json::from_value::<AgentListResp>(serde_json::json!({
            "items": [{
            "agent_id": "agent-1",
            "display_name": "Agent One",
            "tags": [],
            "node_id": "device-1",
            "skills": [
                    { "skill_id": "skill-1", "name": "shell.run" },
                    { "skill_id": "skill-2", "name": "terminal.create" }
                ]
            }]
        }))
        .expect("agent list row should decode through the typed backend contract");
        let row = AgentTableProjection::from_backend_row(&response.items[0]);

        assert_eq!(
            row,
            AgentTableProjection {
                agent_id: "agent-1".to_string(),
                display_name: "Agent One".to_string(),
                node_id: "device-1".to_string(),
                skill_count: 2,
            }
        );
    }

    #[test]
    fn auth_agents_table_rejects_legacy_row_aliases() {
        let error = serde_json::from_value::<AgentListResp>(serde_json::json!({
            "items": [{
                "ura": "easynet:///r/test/agent/legacy",
                "name": "Legacy Agent",
                "tags": [],
                "node_id": "device-1",
                "skills": []
            }]
        }))
        .expect_err("agent list rows must reject retired aliases at schema ingress");

        assert!(
            error.to_string().contains("ura"),
            "schema error should name the retired alias: {error}"
        );
    }

    #[test]
    fn whoami_rejects_malformed_credentials_instead_of_rendering_unpaired() {
        let _home = HomeGuard::new();
        std::fs::create_dir_all(config::state_dir()).expect("state dir");
        std::fs::write(config::state_dir().join("credentials.json"), "{ not json")
            .expect("malformed credentials");

        let error = run_whoami(WhoamiArgs {}).expect_err("malformed credentials must fail closed");

        assert!(
            error.to_string().contains("load device credentials"),
            "whoami must expose credential projection failure: {error}"
        );
    }

    #[test]
    fn missing_auth_session_is_logged_out_state() {
        let _home = HomeGuard::new();

        let session = load_session().expect("missing auth session should be readable state");

        assert!(session.is_none());
    }

    #[test]
    fn auth_session_rejects_missing_user_id_owner_fact() {
        let _home = HomeGuard::new();
        write_auth_session(
            r#"{
  "token": "token",
  "hub_url": "https://hub.example",
  "email": "alice@example.test",
  "username": "alice"
}"#,
        );

        let error = load_session().expect_err("auth session without user_id must fail");

        assert_auth_session_failure_contains(error, "missing field `user_id`");
    }

    #[test]
    fn auth_session_rejects_missing_username_owner_fact() {
        let _home = HomeGuard::new();
        write_auth_session(
            r#"{
  "token": "token",
  "hub_url": "https://hub.example",
  "email": "alice@example.test",
  "user_id": "user-alice"
}"#,
        );

        let error = load_session().expect_err("auth session without username must fail");

        assert_auth_session_failure_contains(error, "missing field `username`");
    }

    #[test]
    fn auth_session_rejects_all_zero_user_id_owner_fact() {
        let _home = HomeGuard::new();
        write_auth_session(
            r#"{
  "token": "token",
  "hub_url": "https://hub.example",
  "email": "alice@example.test",
  "user_id": "00000000-0000-0000-0000-000000000000",
  "username": "alice"
}"#,
        );

        let error = load_session().expect_err("auth session all-zero user_id must fail");

        assert_auth_session_failure_contains(error, "all-zero user_id");
    }

    #[test]
    fn auth_session_rejects_unknown_legacy_fields() {
        let _home = HomeGuard::new();
        write_auth_session(
            r#"{
  "token": "token",
  "hub_url": "https://hub.example",
  "email": "alice@example.test",
  "user_id": "user-alice",
  "username": "alice",
  "legacy_subject": "alice"
}"#,
        );

        let error = load_session().expect_err("unknown auth session fields must fail");

        assert_auth_session_failure_contains(error, "unknown field `legacy_subject`");
    }

    #[test]
    fn login_response_requires_user_owner_facts() {
        let missing_user = serde_json::from_value::<AuthResp>(serde_json::json!({
            "token": "token"
        }))
        .expect_err("login response without user must fail");
        assert!(
            missing_user.to_string().contains("missing field `user`"),
            "unexpected error: {missing_user}"
        );

        let missing_username = serde_json::from_value::<AuthResp>(serde_json::json!({
            "token": "token",
            "user": { "id": "user-alice" }
        }))
        .expect_err("login response without username must fail");
        assert!(
            missing_username
                .to_string()
                .contains("missing field `username`"),
            "unexpected error: {missing_username}"
        );
    }

    #[test]
    fn login_response_accepts_backend_public_user_projection() {
        let response = serde_json::from_value::<AuthResp>(serde_json::json!({
            "token": "token",
            "refresh_token": "refresh",
            "expires_in": 3600,
            "user": {
                "id": "user-alice",
                "account_key": "user-alice",
                "ura": "easynet:///r/acme/user/user-alice",
                "username": "alice",
                "email": "alice@example.test",
                "phone": "",
                "nickname": "Alice",
                "avatar": "",
                "passkey_public_key_count": 0,
                "account_public_keys": []
            }
        }))
        .expect("CLI auth response DTO must accept the backend public UserResp contract");

        assert_eq!(response.user.id, "user-alice");
        assert_eq!(response.user.username, "alice");
        assert_eq!(response.user.nickname.as_deref(), Some("Alice"));
    }

    #[test]
    fn login_response_rejects_unknown_product_fields() {
        let top_level = serde_json::from_value::<AuthResp>(serde_json::json!({
            "token": "token",
            "refresh_token": "refresh",
            "user": {
                "id": "user-alice",
                "username": "alice"
            },
            "state_code": "J200"
        }))
        .expect_err("login response envelope must reject read-model drift");
        assert!(
            top_level.to_string().contains("state_code"),
            "schema error should name the noncanonical field: {top_level}"
        );

        let nested = serde_json::from_value::<AuthResp>(serde_json::json!({
            "token": "token",
            "user": {
                "id": "user-alice",
                "username": "alice",
                "user_ura": "easynet:///r/acme/user/user-alice"
            }
        }))
        .expect_err("login user projection must reject retired owner aliases");
        assert!(
            nested.to_string().contains("user_ura"),
            "schema error should name the retired alias: {nested}"
        );
    }

    #[test]
    fn refresh_response_does_not_require_user_owner_facts() {
        let response = serde_json::from_value::<RefreshResp>(serde_json::json!({
            "token": "new-token"
        }))
        .expect("refresh response is token-only");

        assert_eq!(response.token, "new-token");
        assert!(response.refresh_token.is_none());
    }

    #[test]
    fn refresh_response_rejects_unknown_product_fields() {
        let error = serde_json::from_value::<RefreshResp>(serde_json::json!({
            "token": "new-token",
            "state_code": "J200"
        }))
        .expect_err("refresh response must reject read-model drift");

        assert!(
            error.to_string().contains("state_code"),
            "schema error should name the noncanonical field: {error}"
        );
    }

    #[test]
    fn pairing_token_response_rejects_unknown_product_fields() {
        let error = serde_json::from_value::<PairingResp>(serde_json::json!({
            "pairing_token": "token_123",
            "realm": "acme",
            "state_code": "J200"
        }))
        .expect_err("pairing token response must reject read-model drift");

        assert!(
            error.to_string().contains("state_code"),
            "schema error should name the noncanonical field: {error}"
        );
    }

    #[test]
    fn device_list_response_accepts_backend_read_model_contract() {
        let response = serde_json::from_value::<DeviceListResp>(serde_json::json!({
            "items": [{
                "node_id": "dev-1",
                "display_name": "Device",
                "state": "online",
                "trust_level": "",
                "device_group": "",
                "os": "darwin",
                "arch": "arm64",
                "realm": "acme",
                "ura": "easynet:///r/acme/device/dev-1",
                "last_seen_unix_ms": 42,
                "resolve_unavailable": [],
                "state_code": "J800",
                "transition_id": "T11_REFETCH_READ_MODEL",
                "interrupted_transition": "T11_REFETCH_READ_MODEL",
                "failure": {
                    "code": "RESOLVE_UNAVAILABLE",
                    "message": "namespace resolver unavailable",
                    "stage": "resolve",
                    "retryable": true
                }
            }],
            "resolve_unavailable": [{
                "source": "backend_namespace_resolve",
                "reason": "NOT_FOUND",
                "query_name": "easynet:///r/acme/device/dev-1",
                "message": "owner is not online",
                "code": "ROUTE_NEGATIVE",
                "stage": "resolve",
                "retryable": true,
                "retry_after_unix_ms": 1000
            }]
        }))
        .expect(
            "operator-mode device list must accept backend public DeviceResp read-model fields",
        );

        assert_eq!(response.items[0].state_code.as_deref(), Some("J800"));
        assert_eq!(
            response.resolve_unavailable[0].source,
            "backend_namespace_resolve"
        );
    }

    #[test]
    fn device_list_response_rejects_uncontracted_fields() {
        let top_level = serde_json::from_value::<DeviceListResp>(serde_json::json!({
            "items": [],
            "cursor": "legacy"
        }))
        .expect_err("device list envelope must reject uncontracted fields");
        assert!(
            top_level.to_string().contains("cursor"),
            "schema error should name the noncanonical field: {top_level}"
        );

        let item = serde_json::from_value::<DeviceListResp>(serde_json::json!({
            "items": [{
                "node_id": "dev-1",
                "display_name": "Device",
                "state": "online",
                "legacy_state_code": "J200"
            }]
        }))
        .expect_err("device list rows must reject uncontracted drift");
        assert!(
            item.to_string().contains("legacy_state_code"),
            "schema error should name the noncanonical field: {item}"
        );
    }

    #[test]
    fn ability_list_response_accepts_backend_public_contract() {
        let response = serde_json::from_value::<AbilityListResp>(serde_json::json!({
            "items": [{
                "ura": "easynet:///r/acme/ability/device.dev-1.browser.open_session",
                "name": "browser.open_session",
                "tool_name": "browser.open_session",
                "ability_ura": "easynet:///r/acme/ability/device.dev-1.browser.open_session",
                "version": "1.0.0",
                "category": "browser",
                "description": "Open a browser session",
                "state": "available",
                "install_id": "install-1",
                "owner_ura": "easynet:///r/acme/device/dev-1"
            }],
            "resolve_unavailable": []
        }))
        .expect("operator-mode ability list must accept backend public AbilityResp fields");

        assert_eq!(
            response.items[0].owner_ura.as_deref(),
            Some("easynet:///r/acme/device/dev-1")
        );
    }

    #[test]
    fn ability_list_response_rejects_uncontracted_fields() {
        let error = serde_json::from_value::<AbilityListResp>(serde_json::json!({
            "items": [{
                "name": "browser.open_session",
                "ability_ura": "easynet:///r/acme/ability/device.dev-1.browser.open_session",
                "version": "1.0.0",
                "state": "available",
                "descriptor_ref": "legacy"
            }]
        }))
        .expect_err("ability list rows must reject descriptor projection drift");

        assert!(
            error.to_string().contains("descriptor_ref"),
            "schema error should name the noncanonical field: {error}"
        );
    }

    #[test]
    fn agent_list_response_rejects_unknown_envelope_fields() {
        let error = serde_json::from_value::<AgentListResp>(serde_json::json!({
            "items": [],
            "state_code": "J200"
        }))
        .expect_err("agent list envelope must reject read-model drift");

        assert!(
            error.to_string().contains("state_code"),
            "schema error should name the noncanonical field: {error}"
        );
    }

    #[test]
    fn agent_list_response_accepts_backend_public_contract() {
        let response = serde_json::from_value::<AgentListResp>(serde_json::json!({
            "items": [{
                "agent_id": "easynet:///r/acme/agent/alice.claude",
                "display_name": "Claude",
                "description": "Agent roster row",
                "runtime": "claude-code",
                "base_runtime": "claude-code",
                "model": "sonnet",
                "base_model": "sonnet",
                "tags": [],
                "node_id": "dev-1",
                "host_device_ura": "easynet:///r/acme/device/dev-1",
                "skills": [{
                    "skill_id": "skill-1",
                    "name": "read",
                    "description": "Read files",
                    "tags": ["files"],
                    "state": "enabled"
                }]
            }],
            "resolve_unavailable": []
        }))
        .expect("operator-mode agent list must accept backend public AgentResp fields");

        let row = AgentTableProjection::from_backend_row(&response.items[0]);
        assert_eq!(row.agent_id, "easynet:///r/acme/agent/alice.claude");
        assert_eq!(row.skill_count, 1);
    }

    #[test]
    fn agent_list_response_rejects_uncontracted_row_fields() {
        let error = serde_json::from_value::<AgentListResp>(serde_json::json!({
            "items": [{
                "agent_id": "easynet:///r/acme/agent/alice.claude",
                "display_name": "Claude",
                "tags": [],
                "node_id": "dev-1",
                "legacy_agent_id": "alice"
            }]
        }))
        .expect_err("agent list rows must reject uncontracted drift");

        assert!(
            error.to_string().contains("legacy_agent_id"),
            "schema error should name the noncanonical field: {error}"
        );
    }
}
