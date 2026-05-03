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
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::persistence::config::{
    atomic_write_with_permissions, state_dir, WritePermissions,
};

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
    let session: AuthSession = serde_json::from_str(&raw)
        .with_context(|| format!("parse {}", path.display()))?;
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
}

pub fn run_login(args: LoginArgs) -> anyhow::Result<()> {
    let password = match args.password.clone() {
        Some(p) => p,
        None => rpassword::prompt_password("Password: ")
            .context("read password")?,
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
                .unwrap_or_else(|| {
                    args.email
                        .split('@')
                        .next()
                        .unwrap_or("user")
                        .to_string()
                });
            post_register(&hub, &args.email, &password, &nickname)?
        }
    };

    let session = AuthSession {
        token: auth.token,
        hub_url: hub,
        email: args.email,
        user_id: auth.user.as_ref().and_then(|u| u.id.clone()),
        nickname: auth.user.and_then(|u| u.nickname),
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
