// EasyNet CLI — Runtime Configuration
// ====================================
//
// File: src/daemon/persistence/config.rs
// Description: Persistence layer for local device state under
//              `~/.easynet/`.
//
// Three persistence domains
// -------------------------
//
//   1. `RuntimeState` (runtime.json) — ephemeral session projection:
//      `runtime start` → save()  |  lifecycle/status may load as metadata
//      `runtime stop` → remove() only after process facts are gone.
//      Fields: endpoint (required), pid, hub, tenant, label, started_at.
//
//   2. `Credentials` (credentials.json) — long-lived, survives reboots:
//      `easynet join` → save_credentials()  |  `easynet device reset` → delete_credentials()
//      Fields: node_id, credential_token/join_receipt_hash, hub_endpoint,
//      realm, user_id, username, deploy_signature.
//      Unix permissions: 0o600 (contains credential_token and deploy_signature).
//
//   3. `DeviceSettings` (device_settings.json) — user-controlled knobs:
//      `easynet config` → save/load  |  consumed by start.rs at boot.
//      Fields: session_bridge_exec_enabled (default false).
//
// Implementation notes
// --------------------
// - All files share `~/.easynet/` ([`state_dir`]).
// - JSON pretty-printed for human readability and git-friendliness.
// - Credentials separated from runtime state so server-issued secrets
//   never share a file with ephemeral session data.
//
// Architectural Position
// ----------------------
// Foundation layer consumed by every CLI command. It has no network
// dependency and does not infer transport state from persisted endpoints.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Permissions to apply to the staged temp file *before* rename.
///
/// Applied on Unix only. On other platforms the variant is accepted but
/// the chmod is a no-op — callers get the platform's default file mode.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) enum WritePermissions {
    /// The platform default — whatever `umask` produces.
    Default,
    /// Owner-only read/write (`0o600`). Used for credentials and any
    /// file that must never be world-readable.
    OwnerReadWrite,
}

/// Whether an atomic replacement is known to have crossed the rename commit
/// point.
///
/// Callers that maintain an in-memory projection must distinguish these two
/// states.  Before rename, rolling memory back is correct.  After rename, the
/// replacement is already visible and rolling memory back would create two
/// conflicting truths; the caller must retain the replacement and fail-stop
/// until the parent directory has been re-synchronised on restart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AtomicWriteCommitState {
    NotCommitted,
    ReplacementVisibleButDurabilityUncertain,
}

impl std::fmt::Display for AtomicWriteCommitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCommitted => f.write_str("not committed"),
            Self::ReplacementVisibleButDurabilityUncertain => {
                f.write_str("replacement visible but durability uncertain")
            }
        }
    }
}

/// Failure from the shared atomic-replacement primitive, including the exact
/// commit state at which it failed.
#[derive(Debug, thiserror::Error)]
#[error("atomic write {commit_state}: {source}")]
pub(crate) struct AtomicWriteError {
    commit_state: AtomicWriteCommitState,
    source: anyhow::Error,
}

impl AtomicWriteError {
    fn not_committed(source: impl Into<anyhow::Error>) -> Self {
        Self {
            commit_state: AtomicWriteCommitState::NotCommitted,
            source: source.into(),
        }
    }

    fn durability_uncertain(source: impl Into<anyhow::Error>) -> Self {
        Self {
            commit_state: AtomicWriteCommitState::ReplacementVisibleButDurabilityUncertain,
            source: source.into(),
        }
    }

    pub(crate) fn commit_state(&self) -> AtomicWriteCommitState {
        self.commit_state
    }
}

/// Atomic write: stage in a per-writer temp file, fsync, optionally chmod,
/// then rename onto the target. The function returns only after the new
/// contents are durably visible at `path`.
///
/// Why each step matters
/// ---------------------
///
/// - **Per-writer temp name** (`.<base>.<pid>.<seq>.tmp`) survives two
///   writers racing on the same target path from different terminals —
///   a shared `.<base>.tmp` would let writer B truncate writer A's
///   staging file between A's write and A's rename. The name is
///   derived from `std::process::id()` plus a process-local counter so
///   within one process the name is unique even for concurrent calls.
///
/// - **`sync_all` before rename** guarantees the file contents are on
///   stable storage before the directory entry is swapped. Without it
///   a power loss after `rename` can leave the filesystem pointing the
///   new name at unflushed (possibly empty) data.
///
/// - **Owner-only mode at `open(2)`** closes both permissions races: a
///   sensitive temp file is created as `0600`, then its mode is reasserted
///   before rename. Neither the staging name nor the target name is ever
///   visible with the platform's broader default mode.
///
/// - **`create_new` staging** prevents a stale or attacker-planted temp path
///   from being truncated or followed as a symlink. A collision fails closed
///   and is cleaned up; it never mutates the target.
///
/// - **Best-effort cleanup** removes the staged tmp if any step fails,
///   so a crash-loop doesn't gradually fill the directory with
///   `.runtime.json.1234.7.tmp` corpses.
///
/// `pub(crate)` so other shared/* modules that persist user-visible
/// state (the agent registry, future per-tenant config) can reuse the
/// same race-safe primitive instead of reimplementing it.
/// Reimplementations have already shipped and regressed once — see
/// iteration-1 audit notes.
pub(crate) fn atomic_write_with_permissions(
    path: &Path,
    data: &[u8],
    perms: WritePermissions,
) -> Result<(), AtomicWriteError> {
    atomic_write_with_permissions_and_sync(path, data, perms, sync_directory)
}

/// Atomic replacement with an injectable parent-directory synchroniser.
///
/// The injection seam is deliberately below all callers and above the commit
/// classification so tests can prove post-rename behaviour without a second,
/// subtly different writer implementation.
pub(crate) fn atomic_write_with_permissions_and_sync<F>(
    path: &Path,
    data: &[u8],
    perms: WritePermissions,
    sync: F,
) -> Result<(), AtomicWriteError>
where
    F: FnOnce(&Path) -> anyhow::Result<()>,
{
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or(Path::new("."));
    let base = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config");
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".{base}.{}.{seq}.tmp", std::process::id()));

    // Stage: open the temp file, write the payload, fsync, apply
    // permissions. Any failure on this path removes the temp file.
    let staged = (|| -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if matches!(perms, WritePermissions::OwnerReadWrite) {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut f = options.open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        #[cfg(unix)]
        if matches!(perms, WritePermissions::OwnerReadWrite) {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(not(unix))]
        let _ = perms;
        Ok(())
    })();
    if let Err(e) = staged {
        let _ = fs::remove_file(&tmp);
        return Err(AtomicWriteError::not_committed(e));
    }

    // Commit: swing the directory entry atomically onto `path`. On
    // failure (e.g. parent dir removed under us) the temp file is
    // cleaned up so we don't leak.
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(AtomicWriteError::not_committed(e));
    }
    sync(dir).map_err(AtomicWriteError::durability_uncertain)
}

/// Sync a directory after a metadata mutation such as `rename` or
/// `hard_link`. POSIX filesystems need this to make the directory entry
/// durable across power loss. Windows does not support opening a
/// directory as a regular `File`, and NTFS makes the metadata update
/// durable through the operation itself, so the step is intentionally a
/// no-op there.
pub(crate) fn sync_directory(dir: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        let dir_handle = File::open(dir)
            .map_err(|e| anyhow::anyhow!("open dir {} for fsync: {e}", dir.display()))?;
        dir_handle
            .sync_all()
            .map_err(|e| anyhow::anyhow!("fsync dir {}: {e}", dir.display()))?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Sync the parent directory for a path after a file was created,
/// removed, linked, or renamed. See [`sync_directory`] for platform
/// semantics.
pub(crate) fn sync_parent_dir(path: &Path) -> anyhow::Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    sync_directory(parent)
}

/// Convenience wrapper around [`atomic_write_with_permissions`] for the
/// common case of no special permissions. Preserved so the existing
/// ~20 call sites stay terse; new call sites that need owner-only
/// permissions should use the explicit form.
pub(crate) fn atomic_write(path: &Path, data: &[u8]) -> Result<(), AtomicWriteError> {
    atomic_write_with_permissions(path, data, WritePermissions::Default)
}

// ─── Platform-wide defaults ────────────────────────────────────────────────
// Single source of truth for default Hub/tenant/bind values.
// Consumed by start.rs, connect.rs, join.rs — never hardcode these elsewhere.

pub const DEFAULT_HUB: &str = "axon://easynet.run:50051";
pub const DEFAULT_HUB_HOST: &str = "easynet.run";
pub const DEFAULT_TENANT: &str = "easynet-platform";
pub const DEFAULT_BIND: &str = "0.0.0.0:50051";

/// Process shape recorded by the runtime session projection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// Unified EasyNet daemon. `state.endpoint` is the local Invocation UDS.
    DaemonOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub endpoint: String,
    pub runtime_kind: RuntimeKind,
    pub pid: Option<u32>,
    pub hub: Option<String>,
    pub tenant: Option<String>,
    pub label: Option<String>,
    pub started_at: Option<String>,
    /// Whether the device credential was verified with the Hub at startup.
    /// `None` = not applicable (hub mode), `Some(false)` = Hub unreachable, `Some(true)` = verified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_verified: Option<bool>,
}

/// Resolve the user's home directory.
/// Returns the first available absolute path from `$HOME`, `$USERPROFILE`, or
/// the OS-provided home.
pub fn home_dir() -> PathBuf {
    try_home_dir().unwrap_or_else(|error| panic!("{error}"))
}

pub fn try_home_dir() -> anyhow::Result<PathBuf> {
    resolve_home_dir(
        std::env::var("HOME").ok(),
        std::env::var("USERPROFILE").ok(),
        platform_home_dir(),
    )
}

#[allow(deprecated)]
fn platform_home_dir() -> Option<PathBuf> {
    #[allow(deprecated)]
    std::env::home_dir()
}

fn resolve_home_dir(
    home: Option<String>,
    user_profile: Option<String>,
    os_home: Option<PathBuf>,
) -> anyhow::Result<PathBuf> {
    if let Some(path) = env_home_path("HOME", home)? {
        return Ok(path);
    }
    if let Some(path) = env_home_path("USERPROFILE", user_profile)? {
        return Ok(path);
    }
    if let Some(path) = os_home {
        return validate_home_path("OS home directory", path);
    }
    anyhow::bail!(
        "cannot determine home directory; refusing to use current directory as EasyNet state root"
    )
}

fn env_home_path(name: &'static str, value: Option<String>) -> anyhow::Result<Option<PathBuf>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(None);
    }
    validate_home_path(name, PathBuf::from(value)).map(Some)
}

fn validate_home_path(source: &'static str, path: PathBuf) -> anyhow::Result<PathBuf> {
    if path.as_os_str().is_empty() {
        anyhow::bail!("{source} resolved to an empty path");
    }
    if !path.is_absolute() {
        anyhow::bail!("{source} must resolve to an absolute path, got {path:?}");
    }
    Ok(path)
}

pub fn state_dir() -> PathBuf {
    home_dir().join(".easynet")
}

/// Canonical directory that owns all per-agent roots.
pub fn agents_root() -> PathBuf {
    state_dir().join("agents")
}

fn state_path() -> PathBuf {
    state_dir().join("runtime.json")
}

#[cfg(test)]
pub(crate) fn runtime_state_path() -> PathBuf {
    state_path()
}

pub fn save(state: &RuntimeState) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(state)?;
    atomic_write(&state_path(), json.as_bytes())?;
    Ok(())
}

pub fn load() -> anyhow::Result<RuntimeState> {
    let path = state_path();
    let data = fs::read_to_string(&path)
        .map_err(|_| anyhow::anyhow!("no running runtime — run `easynet start` first"))?;
    let state: RuntimeState = serde_json::from_str(&data)?;
    Ok(state)
}

pub(crate) fn load_optional_runtime_state() -> anyhow::Result<Option<RuntimeState>> {
    let path = state_path();
    let data = match fs::read_to_string(&path) {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(anyhow::anyhow!(
                "read runtime projection {}: {error}",
                path.display()
            ));
        }
    };
    let state: RuntimeState = serde_json::from_str(&data)
        .map_err(|error| anyhow::anyhow!("parse runtime projection {}: {error}", path.display()))?;
    Ok(Some(state))
}

pub fn remove() -> anyhow::Result<()> {
    let path = state_path();
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

impl RuntimeState {
    pub fn tenant_or_default(&self) -> &str {
        self.tenant.as_deref().unwrap_or(DEFAULT_TENANT)
    }
}

// ─── Device Credentials ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub node_id: String,
    pub credential_token: String,
    pub hub_endpoint: String,
    /// Realm this device is paired into.
    pub realm: String,
    #[serde(default)]
    pub deploy_signature: String,
    /// Optional Hub REST API base URL (e.g. "http://localhost:8080") for local dev.
    /// When absent, derived from `hub_endpoint` by stripping scheme/port and using HTTPS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_api_base: Option<String>,
    /// Stable username slug for the user this device is paired to.
    /// Required for display and current agent/resource owner slugs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Immutable product user id (`users.id`) for runtime user-subject URAs.
    /// This is the canonical anchor for `identity.*_user_pubkey` and
    /// user-as-caller trust paths; username must not be used as that subject.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// Realm hub's Ed25519 pubkey (base64), captured during pairing
    /// preflight. Cross-machine cold-start fix (hub in US, CLI in
    /// SG): the device's `auto_wire_self_realm_trust` step needs
    /// this to write the hub's `(ura, pubkey, role=hub)` row into
    /// `realm-trust.toml` without needing on-host access to the hub
    /// runtime keyring. Empty pairing responses are rejected by join.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_pubkey_b64: Option<String>,
    /// Optional base64-encoded PEM trust anchor for the hub's
    /// public TLS listener. Self-hosted hubs with private/self-
    /// signed CAs populate this during pairing preflight so the
    /// join flow can persist a local CA pin before the daemon
    /// opens `session.open`. Publicly-trusted hubs leave it
    /// empty and runtime dials fall back to native OS roots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_tls_ca_pem_b64: Option<String>,
    /// Federation-native join lineage root returned by
    /// `federation.join`. Token-pairing credentials leave this
    /// empty; URA join credentials use it as their completeness
    /// anchor because no backend pairing token/user row is involved
    /// in Phase 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub join_receipt_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeUserBinding {
    Bound { user_ura: String },
    Unbound { reason: &'static str },
}

impl Credentials {
    /// Returns the paired realm.
    pub fn realm_str(&self) -> &str {
        &self.realm
    }
}

impl Credentials {
    /// Resolve the Hub REST API base URL.
    /// Uses `hub_api_base` if set, otherwise derives from `hub_endpoint`.
    pub fn api_base(&self) -> String {
        if let Some(ref base) = self.hub_api_base {
            return base.trim_end_matches('/').to_string();
        }
        let host = extract_api_host(&self.hub_endpoint);
        format!("https://{host}")
    }

    /// Return the stable username slug carried by device credentials.
    ///
    /// Credentials without this slug are structurally incomplete: the
    /// runtime cannot derive canonical user-rooted or agent-rooted
    /// URAs from the credential file alone.
    pub fn username_slug(&self) -> anyhow::Result<&str> {
        self.username
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "credentials file is missing username — run `easynet join <token>` to re-pair"
                )
            })
    }

    /// Return the immutable product user id carried by device credentials.
    pub fn user_id(&self) -> anyhow::Result<&str> {
        let user_id = self
            .user_id
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "credentials file is missing user_id — run `easynet join <token>` to re-pair"
                )
            })?;
        if crate::core::identity::is_all_zero_principal_id(user_id) {
            anyhow::bail!(
                "credentials file carries all-zero user_id — run `easynet join <token>` to re-pair"
            );
        }
        Ok(user_id)
    }

    /// Return the canonical runtime user-subject URA for this credential.
    pub fn user_ura(&self) -> anyhow::Result<String> {
        Ok(crate::core::ura::user_ura(
            self.realm_str(),
            self.user_id()?,
        ))
    }

    /// Project the paired credential into an explicit runtime User binding
    /// state. Token-paired devices must carry a concrete user identity; the
    /// federation-native join lineage may intentionally be device-only until a
    /// downstream product binds a user principal.
    pub fn runtime_user_binding(&self) -> anyhow::Result<RuntimeUserBinding> {
        if self
            .user_id
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Ok(RuntimeUserBinding::Bound {
                user_ura: self.user_ura()?,
            });
        }
        if self.join_receipt_hash().is_some() && self.user_id.is_none() {
            return Ok(RuntimeUserBinding::Unbound {
                reason: "not bound (federation-native device credential)",
            });
        }
        Ok(RuntimeUserBinding::Bound {
            user_ura: self.user_ura()?,
        })
    }

    pub fn join_receipt_hash(&self) -> Option<&str> {
        self.join_receipt_hash
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    fn validate_complete(&self) -> anyhow::Result<()> {
        if self.node_id.is_empty() || self.hub_endpoint.is_empty() || self.realm.is_empty() {
            anyhow::bail!("credentials file is incomplete — run `easynet join <token>` to re-pair");
        }
        if self.credential_token.trim().is_empty() && self.join_receipt_hash().is_none() {
            anyhow::bail!(
                "credentials file is missing credential_token or join_receipt_hash — run `easynet join <token>` to re-pair"
            );
        }
        if self.join_receipt_hash().is_none() {
            self.username_slug()?;
            self.user_id()?;
        } else if self.user_id.is_some() {
            self.user_id()?;
        }
        Ok(())
    }
}

/// Daemon TLS gRPC ports we recognise as transport-plane endpoints
/// — when `hub_endpoint` carries one of these on `https://`, the
/// REST API base needs to be stripped back to bare `:443` because
/// these ports speak HTTP/2 + gRPC, not HTTP/1.1 REST.
///
/// 50443 is the canonical hub-side TLS listen port (`Daemon`
/// `mode = both` in production); 50543 is the answer-sheet demo's
/// hub-B port; both surface as `extract_api_host` inputs whenever
/// the operator's `Axon.PublicEndpoint` resolves to a daemon TLS
/// listener (the post-LB-65 production yaml hard-codes
/// `https://easynet.run:50443` so this list is the production
/// hot path).
const DAEMON_TLS_PORTS: &[&str] = &["50443", "50543"];

/// Extract the hostname authority an REST/HTTP API call should target,
/// given the bidi/transport endpoint persisted in `creds.hub_endpoint`.
///
/// Conventions:
/// * `axon://host:<grpc-port>` → strip the port (REST is on HTTPS/443).
/// * `https://host:<daemon-TLS-port>` → strip the port for the same
///   reason — those ports serve gRPC, not REST. The set of recognised
///   ports lives in `DAEMON_TLS_PORTS`; `--hub-api` overrides this
///   for operators running REST on a non-standard port.
/// * `https://host:<other-port>` / `http://host:<port>` → preserve the
///   authority verbatim (operator-set non-default REST endpoint).
fn extract_api_host(endpoint: &str) -> String {
    let endpoint = endpoint.trim();
    let (is_axon, without_scheme) = if let Some(rest) = endpoint.strip_prefix("axon://") {
        (true, rest)
    } else if let Some(rest) = endpoint.strip_prefix("https://") {
        (false, rest)
    } else if let Some(rest) = endpoint.strip_prefix("http://") {
        (false, rest)
    } else {
        (false, endpoint)
    };
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);
    if authority.is_empty() {
        return DEFAULT_HUB_HOST.to_string();
    }
    // IPv6 bracketed address: [::1]:port or [::1]
    if authority.starts_with('[') {
        if let Some(bracket_end) = authority.find(']') {
            let host_part = &authority[..=bracket_end]; // includes brackets
            if is_axon {
                // Strip port for axon:// — REST API uses HTTPS/443.
                return host_part.to_string();
            }
            // https://[::1]:50443 → strip daemon-TLS port back to bare host
            if let Some(rest) = authority.strip_prefix(host_part) {
                if let Some(port) = rest.strip_prefix(':') {
                    if DAEMON_TLS_PORTS.contains(&port) {
                        return host_part.to_string();
                    }
                }
            }
            return authority.to_string();
        }
    }
    if is_axon {
        // axon:// uses gRPC port — strip it, REST API is on HTTPS/443.
        authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host)
            .to_string()
    } else {
        // https://host:<daemon-TLS-port> → strip the port (gRPC, not
        // REST). Preserve every other authority verbatim so an
        // operator running REST on a non-standard port keeps working.
        if let Some((host, port)) = authority.rsplit_once(':') {
            if DAEMON_TLS_PORTS.contains(&port) {
                return host.to_string();
            }
        }
        authority.to_string()
    }
}

fn credentials_path() -> PathBuf {
    state_dir().join("credentials.json")
}

/// Path to the unified EasyNet daemon PID file.
///
/// Start writes it after spawning `easynet-daemon`; stop validates,
/// signals, and removes it. Discovery remains an independent repair
/// path when this projection is missing or stale.
pub fn easynet_daemon_pid_path() -> PathBuf {
    state_dir().join("easynet-daemon.pid")
}

pub fn save_credentials(creds: &Credentials) -> anyhow::Result<()> {
    creds.validate_complete()?;
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(creds)? + "\n";
    // Owner-only mode is applied to the staged temp file *before* the
    // rename, so the credentials file is never briefly world-readable
    // at its final path. A post-rename chmod leaves that window open —
    // see `atomic_write_with_permissions` for the full argument.
    atomic_write_with_permissions(
        &credentials_path(),
        json.as_bytes(),
        WritePermissions::OwnerReadWrite,
    )?;
    Ok(())
}

pub fn load_credentials() -> anyhow::Result<Credentials> {
    load_credentials_optional()?
        .ok_or_else(|| anyhow::anyhow!("no credentials found — run `easynet join <token>` first"))
}

pub fn load_credentials_optional() -> anyhow::Result<Option<Credentials>> {
    let path = credentials_path();
    let data = match fs::read_to_string(&path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(anyhow::anyhow!(
                "read credentials from {}: {err}",
                path.display()
            ));
        }
    };
    let creds: Credentials = serde_json::from_str(&data)
        .map_err(|err| anyhow::anyhow!("parse credentials from {}: {err}", path.display()))?;
    creds
        .validate_complete()
        .map_err(|err| anyhow::anyhow!("validate credentials from {}: {err}", path.display()))?;
    Ok(Some(creds))
}

pub fn delete_credentials() -> anyhow::Result<()> {
    let path = credentials_path();
    if path.exists() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

// ─── Device Settings ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DeviceSettings {
    #[serde(default)]
    pub session_bridge_exec_enabled: bool,
    /// Stable per-machine install id. Generated once and persisted in
    /// `device_settings.json` (which survives `easynet device reset`, unlike
    /// `credentials.json`), so the hub can recognise a returning machine on
    /// re-pair and reuse its `node_id` + keypair + trust row instead of
    /// minting a fresh identity each time (device-id churn). `None` until the
    /// first `load_or_create_install_id()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_id: Option<String>,
}

fn device_settings_path() -> PathBuf {
    state_dir().join("device_settings.json")
}

/// Return this machine's stable install id, generating and persisting one on
/// first use. Survives `easynet device reset` (it lives in `device_settings.json`,
/// not `credentials.json`), so re-pairing the same host presents the same
/// install id and the hub can reuse the prior `node_id`.
pub fn load_or_create_install_id() -> anyhow::Result<String> {
    let mut settings = load_device_settings()?;
    if let Some(id) = settings
        .install_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(id.to_string());
    }
    let id = uuid::Uuid::new_v4().to_string();
    settings.install_id = Some(id.clone());
    save_device_settings(&settings)?;
    Ok(id)
}

pub fn load_device_settings() -> anyhow::Result<DeviceSettings> {
    let path = device_settings_path();
    let data = match fs::read_to_string(&path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DeviceSettings::default());
        }
        Err(err) => {
            return Err(anyhow::anyhow!(
                "read device settings from {}: {err}",
                path.display()
            ));
        }
    };
    serde_json::from_str(&data)
        .map_err(|err| anyhow::anyhow!("parse device settings from {}: {err}", path.display()))
}

pub fn save_device_settings(settings: &DeviceSettings) -> anyhow::Result<()> {
    let dir = state_dir();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(settings)? + "\n";
    atomic_write(&device_settings_path(), json.as_bytes())?;
    Ok(())
}

// Agent registry types moved to shared/agents.rs to preserve this file's
// three-domain contract (runtime state / credentials / device settings).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_resolver_rejects_missing_sources_before_current_directory_fallback() {
        let error = resolve_home_dir(None, None, None)
            .expect_err("missing home sources must not fall back to cwd");

        assert!(
            error
                .to_string()
                .contains("refusing to use current directory"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn home_resolver_rejects_relative_env_source_before_state_root_derivation() {
        let error = resolve_home_dir(Some("relative-home".to_string()), None, None)
            .expect_err("relative HOME must not derive a relative state root");

        assert!(
            error
                .to_string()
                .contains("HOME must resolve to an absolute path"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn home_resolver_uses_first_absolute_available_source() {
        let user_profile = std::env::temp_dir().join("easynet-userprofile-home");
        let os_home = std::env::temp_dir().join("easynet-os-home");

        let resolved = resolve_home_dir(
            Some(" ".to_string()),
            Some(user_profile.display().to_string()),
            Some(os_home),
        )
        .expect("blank HOME is unavailable; USERPROFILE is the first absolute source");

        assert_eq!(resolved, user_profile);
    }

    #[test]
    fn extract_api_host_strips_port_for_axon_scheme() {
        assert_eq!(extract_api_host("axon://easynet.run:50051"), "easynet.run");
        assert_eq!(extract_api_host("axon://10.0.0.1:50051"), "10.0.0.1");
        assert_eq!(extract_api_host("axon://[::1]:50051"), "[::1]");
    }

    #[test]
    fn extract_api_host_preserves_http_authority() {
        assert_eq!(
            extract_api_host("https://hub.example.com:8443"),
            "hub.example.com:8443"
        );
        assert_eq!(extract_api_host("http://127.0.0.1:8080"), "127.0.0.1:8080");
        assert_eq!(extract_api_host("https://[::1]:8080"), "[::1]:8080");
    }

    #[test]
    fn extract_api_host_strips_daemon_tls_port_for_https_scheme() {
        // Production yaml hard-codes PublicEndpoint=https://easynet.run:50443
        // (the daemon TLS gRPC port). REST calls have to land on :443,
        // not on the gRPC listener — historic bug.
        assert_eq!(extract_api_host("https://easynet.run:50443"), "easynet.run");
        assert_eq!(extract_api_host("https://10.0.0.1:50443"), "10.0.0.1");
        assert_eq!(extract_api_host("https://[::1]:50443"), "[::1]");
        // demo's hub-B port follows the same posture.
        assert_eq!(extract_api_host("https://hub-b:50543"), "hub-b");
    }

    #[test]
    fn credentials_api_base_prefers_override_and_trims_trailing_slash() {
        let creds = Credentials {
            node_id: "n".into(),
            credential_token: "t".into(),
            hub_endpoint: "axon://easynet.run:50051".into(),
            realm: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: Some("https://api.example.com/".into()),
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        assert_eq!(creds.api_base(), "https://api.example.com");
    }

    #[test]
    fn runtime_user_binding_projects_bound_user_ura() {
        let creds = Credentials {
            node_id: "n".into(),
            credential_token: "t".into(),
            hub_endpoint: "axon://easynet.run:50051".into(),
            realm: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };

        assert_eq!(
            creds.runtime_user_binding().expect("runtime user binding"),
            RuntimeUserBinding::Bound {
                user_ura: "easynet:///r/tenant/user/user-alice".into(),
            }
        );
    }

    #[test]
    fn runtime_user_binding_makes_federation_native_device_only_explicit() {
        let creds = Credentials {
            node_id: "n".into(),
            credential_token: String::new(),
            hub_endpoint: "https://hub.example:50443".into(),
            realm: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: None,
            user_id: None,
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: Some("a".repeat(64)),
        };

        assert_eq!(
            creds
                .runtime_user_binding()
                .expect("device-only federation binding"),
            RuntimeUserBinding::Unbound {
                reason: "not bound (federation-native device credential)",
            }
        );
    }

    #[test]
    fn runtime_user_binding_rejects_blank_join_user_id_instead_of_hiding_it() {
        let creds = Credentials {
            node_id: "n".into(),
            credential_token: String::new(),
            hub_endpoint: "https://hub.example:50443".into(),
            realm: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: None,
            user_id: Some("   ".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: Some("a".repeat(64)),
        };

        let err = creds
            .runtime_user_binding()
            .expect_err("blank explicit user_id is invalid");
        assert!(
            err.to_string().contains("missing user_id"),
            "explicit blank user_id must not become unbound: {err}"
        );
    }

    #[test]
    fn atomic_write_survives_concurrent_writers() {
        // Regression guard: two threads writing to the same path must not
        // corrupt it, even under contention. A shared tmp name would let
        // thread B overwrite thread A's staging file between A's write
        // and rename, producing a garbled target.
        use std::sync::Arc;
        use std::thread;

        let dir = std::env::temp_dir().join(format!(
            "easynet-config-atomic-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        fs::create_dir_all(&dir).unwrap();
        let target = Arc::new(dir.join("runtime.json"));

        let workers: Vec<_> = (0..8)
            .map(|i| {
                let t = Arc::clone(&target);
                thread::spawn(move || {
                    // Each worker loops a handful of times so the tmp-file
                    // window is actually overlapping between threads.
                    for j in 0..16 {
                        let payload = format!(r#"{{"writer":{i},"iter":{j}}}"#);
                        atomic_write(&t, payload.as_bytes()).expect("atomic_write");
                    }
                })
            })
            .collect();
        for w in workers {
            w.join().unwrap();
        }

        // After the storm, the file must contain one of the payloads
        // verbatim — never an interleaved byte salad.
        let contents = fs::read_to_string(&*target).unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&contents).expect("final state must be valid JSON");
        assert!(v.get("writer").and_then(|x| x.as_u64()).is_some());
        assert!(v.get("iter").and_then(|x| x.as_u64()).is_some());

        // No stray .tmp files should remain in the directory.
        let stragglers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(".tmp")))
            .collect();
        assert!(
            stragglers.is_empty(),
            "leftover tmp files after concurrent writes: {stragglers:?}"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn atomic_write_classifies_post_rename_directory_sync_failure() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("state.json");
        fs::write(&target, b"old").unwrap();

        let error = atomic_write_with_permissions_and_sync(
            &target,
            b"new",
            WritePermissions::OwnerReadWrite,
            |_| anyhow::bail!("injected parent-directory fsync failure"),
        )
        .unwrap_err();

        assert_eq!(
            error.commit_state(),
            AtomicWriteCommitState::ReplacementVisibleButDurabilityUncertain
        );
        assert_eq!(fs::read(&target).unwrap(), b"new");
    }

    /// On Unix, owner-only permissions must be visible to a reader the
    /// instant the target path exists — i.e. permissions are baked into
    /// the file *before* the rename. A naive `rename`-then-`chmod`
    /// would momentarily expose the file with the default mode.
    ///
    /// This test reads `st_mode` immediately after `atomic_write_with_
    /// permissions` returns and asserts the owner-only bits are set.
    /// Because the rename is atomic with respect to the final path, any
    /// state observable at that path must already carry the mode.
    #[cfg(unix)]
    #[test]
    fn atomic_write_applies_owner_only_permissions_before_rename() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "easynet-config-perms-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("secret.json");

        atomic_write_with_permissions(
            &target,
            b"{\"token\":\"sensitive\"}",
            WritePermissions::OwnerReadWrite,
        )
        .expect("write");

        let mode = fs::metadata(&target).unwrap().permissions().mode();
        // Low nine bits encode owner/group/other rwx. `0o600` = owner
        // rw, no group/other access.
        assert_eq!(
            mode & 0o777,
            0o600,
            "credentials file mode must be 0o600 immediately after rename, got {:o}",
            mode & 0o777
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn credentials_api_base_derives_from_hub_endpoint_when_override_absent() {
        let creds = Credentials {
            node_id: "n".into(),
            credential_token: "t".into(),
            hub_endpoint: "axon://my-hub.example.org:50051".into(),
            realm: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        assert_eq!(creds.api_base(), "https://my-hub.example.org");
    }

    #[test]
    fn runtime_state_rejects_missing_runtime_kind() {
        let err = serde_json::from_str::<RuntimeState>(
            r#"{
                "endpoint": "/tmp/easynet-daemon.sock",
                "pid": 7,
                "tenant": "tenant-a"
            }"#,
        )
        .expect_err("runtime projection must carry explicit runtime_kind");
        assert!(
            err.to_string().contains("missing field `runtime_kind`"),
            "unexpected runtime projection error: {err}"
        );
    }

    #[test]
    fn save_credentials_rejects_missing_username() {
        let _g = HomeGuard::new();
        let creds = Credentials {
            node_id: "node".into(),
            credential_token: "token".into(),
            hub_endpoint: "axon://hub.example:7700".into(),
            realm: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: None,
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };

        let err = save_credentials(&creds).expect_err("missing username must not persist");
        assert!(
            err.to_string().contains("missing username"),
            "error should name the missing username contract: {err}"
        );
    }

    #[test]
    fn save_credentials_rejects_missing_user_id() {
        let _g = HomeGuard::new();
        let creds = Credentials {
            node_id: "node".into(),
            credential_token: "token".into(),
            hub_endpoint: "axon://hub.example:7700".into(),
            realm: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: None,
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };

        let err = save_credentials(&creds).expect_err("missing user_id must not persist");
        assert!(
            err.to_string().contains("missing user_id"),
            "error should name the missing user_id contract: {err}"
        );
    }

    #[test]
    fn save_credentials_accepts_federation_join_receipt_without_user_binding() {
        let _g = HomeGuard::new();
        let creds = Credentials {
            node_id: "node".into(),
            credential_token: String::new(),
            hub_endpoint: "https://hub.example:50443".into(),
            realm: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: None,
            user_id: None,
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: Some("a".repeat(64)),
        };

        save_credentials(&creds).expect("federation-native credentials save");
    }

    #[test]
    fn save_credentials_rejects_join_receipt_with_all_zero_user_id() {
        let _g = HomeGuard::new();
        let creds = Credentials {
            node_id: "node".into(),
            credential_token: String::new(),
            hub_endpoint: "https://hub.example:50443".into(),
            realm: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: None,
            user_id: Some("00000000-0000-0000-0000-000000000000".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: Some("a".repeat(64)),
        };

        let err = save_credentials(&creds).expect_err("all-zero user_id must not persist");
        assert!(
            err.to_string().contains("all-zero user_id"),
            "error should name the all-zero user_id contract: {err}"
        );
    }

    #[test]
    fn save_credentials_writes_realm_field() {
        let _g = HomeGuard::new();
        let creds = Credentials {
            node_id: "node".into(),
            credential_token: "token".into(),
            hub_endpoint: "axon://hub.example:7700".into(),
            realm: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };

        save_credentials(&creds).expect("save credentials");
        let saved = fs::read_to_string(credentials_path()).expect("read credentials");
        let value: serde_json::Value = serde_json::from_str(&saved).expect("credentials json");
        assert_eq!(value["realm"], "tenant");
        assert!(
            value.get("tenant_id").is_none(),
            "credentials.json must not write retired tenant_id: {saved}"
        );
    }

    #[test]
    fn load_credentials_rejects_file_without_username() {
        let _g = HomeGuard::new();
        fs::create_dir_all(state_dir()).expect("create state dir");
        fs::write(
            credentials_path(),
            r#"{
  "node_id": "node",
  "credential_token": "token",
  "hub_endpoint": "axon://hub.example:7700",
  "realm": "tenant",
  "user_id": "user-alice",
  "deploy_signature": ""
}
"#,
        )
        .expect("write incomplete credentials");

        let err = load_credentials().expect_err("missing username must fail on load");
        assert!(
            err.to_string().contains("missing username"),
            "error should name the missing username contract: {err}"
        );
    }

    #[test]
    fn load_credentials_optional_returns_none_for_missing_file() {
        let _g = HomeGuard::new();

        let creds = load_credentials_optional().expect("missing credentials is optional absence");

        assert!(creds.is_none());
    }

    #[test]
    fn load_credentials_optional_rejects_malformed_existing_file() {
        let _g = HomeGuard::new();
        fs::create_dir_all(state_dir()).expect("create state dir");
        fs::write(credentials_path(), b"{").expect("write malformed credentials");

        let err = load_credentials_optional().expect_err("malformed credentials must fail");

        assert!(
            err.to_string().contains("parse credentials"),
            "error should name credential parse failure: {err}"
        );
    }

    #[test]
    fn load_credentials_rejects_file_without_user_id() {
        let _g = HomeGuard::new();
        fs::create_dir_all(state_dir()).expect("create state dir");
        fs::write(
            credentials_path(),
            r#"{
  "node_id": "node",
  "credential_token": "token",
  "hub_endpoint": "axon://hub.example:7700",
  "realm": "tenant",
  "deploy_signature": "",
  "username": "alice"
}
"#,
        )
        .expect("write incomplete credentials");

        let err = load_credentials().expect_err("missing user_id must fail on load");
        assert!(
            err.to_string().contains("missing user_id"),
            "error should name the missing user_id contract: {err}"
        );
    }

    #[test]
    fn load_credentials_rejects_all_zero_user_id() {
        let _g = HomeGuard::new();
        fs::create_dir_all(state_dir()).expect("create state dir");
        fs::write(
            credentials_path(),
            r#"{
  "node_id": "node",
  "credential_token": "token",
  "hub_endpoint": "axon://hub.example:7700",
  "realm": "tenant",
  "deploy_signature": "",
  "username": "alice",
  "user_id": "00000000-0000-0000-0000-000000000000"
}
"#,
        )
        .expect("write placeholder credentials");

        let err = load_credentials().expect_err("all-zero user_id must fail on load");
        assert!(
            err.to_string().contains("all-zero user_id"),
            "error should name the all-zero user_id contract: {err}"
        );
    }

    #[test]
    fn load_credentials_rejects_join_receipt_with_all_zero_user_id() {
        let _g = HomeGuard::new();
        fs::create_dir_all(state_dir()).expect("create state dir");
        fs::write(
            credentials_path(),
            r#"{
  "node_id": "node",
  "credential_token": "",
  "hub_endpoint": "https://hub.example:50443",
  "realm": "tenant",
  "deploy_signature": "",
  "user_id": "00000000-0000-0000-0000-000000000000",
  "join_receipt_hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
}
"#,
        )
        .expect("write receipt credentials with placeholder user id");

        let err = load_credentials().expect_err("all-zero user_id must fail on load");
        assert!(
            err.to_string().contains("all-zero user_id"),
            "error should name the all-zero user_id contract: {err}"
        );
    }

    #[test]
    fn load_credentials_rejects_retired_tenant_id_field() {
        let _g = HomeGuard::new();
        fs::create_dir_all(state_dir()).expect("create state dir");
        fs::write(
            credentials_path(),
            r#"{
  "node_id": "node",
  "credential_token": "token",
  "hub_endpoint": "axon://hub.example:7700",
  "tenant_id": "tenant",
  "deploy_signature": "",
  "username": "alice",
  "user_id": "user-alice"
}
"#,
        )
        .expect("write retired credentials");

        let err = load_credentials().expect_err("retired tenant_id must fail on load");
        assert!(
            err.to_string().contains("tenant_id"),
            "error should name the retired field: {err}"
        );
    }

    #[test]
    fn load_optional_runtime_state_returns_none_when_projection_missing() {
        let _g = HomeGuard::new();

        assert!(
            load_optional_runtime_state()
                .expect("missing projection")
                .is_none(),
            "missing runtime.json is a valid absent projection state"
        );
    }

    #[test]
    fn load_optional_runtime_state_rejects_malformed_existing_projection() {
        let _g = HomeGuard::new();
        fs::create_dir_all(state_dir()).expect("create state dir");
        fs::write(runtime_state_path(), "{ not json").expect("write malformed runtime projection");

        let error = load_optional_runtime_state()
            .expect_err("malformed existing runtime.json must fail closed");

        assert!(
            error.to_string().contains("parse runtime projection"),
            "wrong error: {error:#}"
        );
    }

    // ── Agent directory layout migration ─────────────────────────────────

    // All of these tests use `HomeGuard` so they never touch the
    // developer's real `~/.easynet/` tree. `HomeGuard` serializes
    // concurrent tests via a global mutex, which is load-bearing because
    // the layout owner reads `HOME`.

    use crate::cli::commands::test_support::HomeGuard;

    #[test]
    fn agents_root_is_canonical_even_before_the_directory_exists() {
        let _g = HomeGuard::new();
        assert_eq!(agents_root(), state_dir().join("agents"));
    }

    #[test]
    fn install_id_is_generated_once_and_stable_across_calls_and_reset() {
        let _g = HomeGuard::new();

        // First call generates and persists.
        let first = load_or_create_install_id().expect("first install id");
        assert!(!first.is_empty());
        assert!(
            load_device_settings()
                .expect("load device settings")
                .install_id
                .as_deref()
                == Some(first.as_str())
        );

        // Second call returns the SAME id (idempotent), not a new uuid.
        let second = load_or_create_install_id().expect("second install id");
        assert_eq!(first, second, "install id must be stable across calls");

        // `reset` only deletes credentials.json — device_settings.json (and thus
        // the install id) survives, so a re-pair presents the same id.
        let _ = delete_credentials();
        let after_reset = load_or_create_install_id().expect("install id after reset");
        assert_eq!(
            first, after_reset,
            "install id must survive reset (lives in device_settings.json)"
        );
    }

    #[test]
    fn load_device_settings_missing_file_returns_default() {
        let _g = HomeGuard::new();

        let settings = load_device_settings().expect("missing settings uses default");

        assert!(!settings.session_bridge_exec_enabled);
        assert_eq!(settings.install_id, None);
    }

    #[test]
    fn load_device_settings_rejects_malformed_existing_file() {
        let _g = HomeGuard::new();
        fs::create_dir_all(state_dir()).expect("create state dir");
        fs::write(device_settings_path(), b"{").expect("write malformed settings");

        let err = load_device_settings().expect_err("malformed settings must fail");

        assert!(
            err.to_string().contains("parse device settings"),
            "error should name parse failure: {err}"
        );
    }

    #[test]
    fn load_device_settings_rejects_unknown_fields() {
        let _g = HomeGuard::new();
        fs::create_dir_all(state_dir()).expect("create state dir");
        fs::write(
            device_settings_path(),
            br#"{"session_bridge_exec_enabled":false,"legacy":true}"#,
        )
        .expect("write unknown settings field");

        let err = load_device_settings().expect_err("unknown settings field must fail");

        assert!(
            err.to_string().contains("unknown field"),
            "error should reject unknown fields: {err}"
        );
    }

    #[test]
    fn load_or_create_install_id_rejects_malformed_settings_without_rewriting() {
        let _g = HomeGuard::new();
        fs::create_dir_all(state_dir()).expect("create state dir");
        fs::write(device_settings_path(), b"{").expect("write malformed settings");

        let err = load_or_create_install_id().expect_err("malformed settings must fail");

        assert!(
            err.to_string().contains("parse device settings"),
            "error should surface malformed settings: {err}"
        );
        assert_eq!(
            fs::read(device_settings_path()).expect("settings preserved"),
            b"{",
            "malformed settings must not be rewritten as default state"
        );
    }
}
