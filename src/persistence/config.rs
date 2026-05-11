// EasyNet CLI — Runtime Configuration
// ====================================
//
// File: src/persistence/config.rs
// Description: Persistence layer for all local device state under
//              `~/.easynet/`, plus the one-call bridge-opening helper
//              that composes loaded [`RuntimeState`] with the
//              state-free connector in `shared`.
//
// Three persistence domains
// -------------------------
//
//   1. `RuntimeState` (runtime.json) — ephemeral, one session:
//      `easynet start` → save()  |  `easynet *` → load()  |  `easynet stop` → remove()
//      Fields: endpoint (required), pid, hub, tenant, label, started_at.
//
//   2. `Credentials` (credentials.json) — long-lived, survives reboots:
//      `easynet join` → save_credentials()  |  `easynet reset` → delete_credentials()
//      Fields: node_id, credential_token, hub_endpoint, tenant_id, deploy_signature.
//      Unix permissions: 0o600 (contains credential_token and deploy_signature).
//
//   3. `DeviceSettings` (device_settings.json) — user-controlled knobs:
//      `easynet config` → save/load  |  consumed by start.rs at boot.
//      Fields: session_bridge_exec_enabled (default false).
//
// Where the bridge helper lives — and why here
// --------------------------------------------
//
// [`load_and_connect`] and [`RuntimeState::connect_bridge`] read the
// on-disk state and then open a [`DendriteBridge`] to its endpoint.
// They live in this module — not in `shared` — because the dependency
// only flows one way: **persistence composes transport**, never the
// reverse. `shared` remains a pure-plumbing leaf that takes an
// endpoint string in. An earlier draft had both helpers sitting at
// the top of `shared/mod.rs`, which made `shared` silently consume
// `persistence::config::load` — inverting the module layering in a
// way that only showed up as a paragraph in the doc comment. The
// placement here makes the layering visible at the use site:
//
//     // 1 argument → pure transport
//     let bridge = support::connect_bridge_to(endpoint)?;
//     // 0 arguments → reads state, then connects
//     let (bridge, state) = persistence::config::load_and_connect()?;
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
// Foundation layer consumed by every CLI command. No network
// dependencies on its own — `load_and_connect` sits at the seam
// where persistence pairs a loaded endpoint with the transport layer
// from `shared`.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
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
/// - **`chmod` before rename** closes the permissions race: if we
///   `rename` first and `chmod` after, another process can `open` the
///   file in the window between the two syscalls and read it with the
///   default (world-readable on most systems) mode. Applying the mode
///   to the temp file first means the file is *never* visible at the
///   target path with the wrong permissions.
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
) -> anyhow::Result<()> {
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
        let mut f = File::create(&tmp)?;
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
        return Err(e.into());
    }

    // Commit: swing the directory entry atomically onto `path`. On
    // failure (e.g. parent dir removed under us) the temp file is
    // cleaned up so we don't leak.
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// Convenience wrapper around [`atomic_write_with_permissions`] for the
/// common case of no special permissions. Preserved so the existing
/// ~20 call sites stay terse; new call sites that need owner-only
/// permissions should use the explicit form.
pub(crate) fn atomic_write(path: &Path, data: &[u8]) -> anyhow::Result<()> {
    atomic_write_with_permissions(path, data, WritePermissions::Default)
}

// ─── Platform-wide defaults ────────────────────────────────────────────────
// Single source of truth for default Hub/tenant/bind values.
// Consumed by start.rs, connect.rs, join.rs — never hardcode these elsewhere.

pub const DEFAULT_HUB: &str = "axon://easynet.run:50051";
pub const DEFAULT_HUB_HOST: &str = "easynet.run";
pub const DEFAULT_TENANT: &str = "easynet-platform";
pub const DEFAULT_BIND: &str = "0.0.0.0:50051";

/// Which process shape the persisted runtime state refers to.
///
/// Old `runtime.json` files (pre-daemon-only device mode) carried only
/// an `endpoint` string and therefore deserialize as the historical
/// default: an Axon bridge endpoint.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// Historical local `axon-runtime` process reachable via the
    /// dendrite bridge (`state.endpoint` is a bridge endpoint).
    #[default]
    AxonBridge,
    /// Daemon-only device mode. `state.endpoint` names the daemon's
    /// local gRPC UDS socket and MUST NOT be treated as a bridge.
    DaemonOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub endpoint: String,
    #[serde(default)]
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
/// Returns the first available of `$HOME`, `$USERPROFILE`, or the OS-provided home.
pub fn home_dir() -> PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        return PathBuf::from(h);
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        return PathBuf::from(h);
    }
    // Last resort: platform home_dir (works on most systems).
    #[allow(deprecated)]
    std::env::home_dir().unwrap_or_else(|| {
        eprintln!("warning: cannot determine home directory; using current directory");
        PathBuf::from(".")
    })
}

pub fn state_dir() -> PathBuf {
    home_dir().join(".easynet")
}

/// Resolve the directory that holds per-agent root directories.
///
/// Returns the new convention (`~/.easynet/agents/`) when that path
/// exists on disk; otherwise returns the legacy path
/// (`~/.easynet/workspaces/`). Writers should still use the legacy
/// path until the full `agent.toml` + `AgentDirectory` machinery
/// lands (planned) — at that point writers flip to the new path and
/// a one-shot migration moves any remaining legacy-path agents.
///
/// Rationale for "read two, write one":
///
/// - The on-disk layout of a per-agent root is unchanged (runs/,
///   CLAUDE.md, AGENTS.md, .mcp.json, .codex/config.toml, .git/,
///   .agents/skills/). Only the parent directory name changes.
///   A read-side fallback is cheap and lets users who already have
///   agents under `workspaces/` keep working across the upgrade
///   without a flag day.
/// - Committing the final rename to the writer before the new
///   AgentDirectory model exists would leave us with the new path
///   on disk and the old code generating into it — two versions
///   of the same transition, confusing to debug.
///
/// Deprecation window: legacy fallback is kept until 1.9.0 (see
/// `docs/rfc/eal-control-flow-v1.md` is unrelated — the window
/// tracked in the top-level plan). A single `eprintln` warning is
/// emitted the first time a process observes a legacy-only agents
/// directory so operators know the rename is pending.
pub fn agents_root() -> PathBuf {
    let new = state_dir().join("agents");
    if new.exists() {
        return new;
    }
    let legacy = state_dir().join("workspaces");
    if legacy.exists() {
        warn_legacy_agents_root_once(&legacy);
        return legacy;
    }
    // Neither exists yet (fresh install, or `easynet start` hasn't
    // run). Fall through to the legacy path so the first writer
    // stays byte-compatible with the pre-rename shape. The new
    // AgentDirectory flip happens when that PR lands.
    legacy
}

/// Print the "workspaces is deprecated" warning at most once per
/// process. `state_dir()` is read on nearly every CLI invocation,
/// so a plain `eprintln` would spam the terminal. `OnceCell` gives
/// us the "once per process" semantic without a mutex.
fn warn_legacy_agents_root_once(path: &std::path::Path) {
    use std::sync::OnceLock;
    static WARNED: OnceLock<()> = OnceLock::new();
    if WARNED.set(()).is_ok() {
        eprintln!(
            "note: {} is deprecated — agents will move to {}/agents/ at 1.9.0",
            path.display(),
            state_dir().display(),
        );
    }
}

fn state_path() -> PathBuf {
    state_dir().join("runtime.json")
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

    pub fn uses_bridge(&self) -> bool {
        matches!(self.runtime_kind, RuntimeKind::AxonBridge)
    }

    /// Open a [`DendriteBridge`] to this runtime's endpoint, using the
    /// shared connect-timeout budget.
    ///
    /// Thin composition of [`crate::support::connect_bridge_to`] —
    /// lives here (not on the bridge side) so that consumers who
    /// already hold a `RuntimeState` can spell the call in a way that
    /// reads as "use this state to reach its runtime":
    ///
    /// ```ignore
    /// let state = persistence::config::load()?;
    /// let bridge = state.connect_bridge()?;
    /// ```
    ///
    /// If you don't already have a `RuntimeState`, prefer
    /// [`load_and_connect`] — it performs both steps in one call and
    /// the shorter name matches the common case.
    ///
    /// [`DendriteBridge`]: easynet_axon::dendrite_bridge::DendriteBridge
    pub fn connect_bridge(&self) -> anyhow::Result<easynet_axon::dendrite_bridge::DendriteBridge> {
        anyhow::ensure!(
            self.uses_bridge(),
            "local runtime is running in daemon-only mode; no axon bridge endpoint is available"
        );
        crate::support::connect_bridge_to(&self.endpoint)
    }
}

/// Load the persisted [`RuntimeState`] and open a [`DendriteBridge`]
/// to its endpoint in one call.
///
/// Returns `(bridge, state)` so callers can access tenant, label, or
/// started-at metadata without a second load. This is the canonical
/// entry point for CLI commands that just need "connect me to
/// whatever runtime is running here".
///
/// # Why this composition lives in `persistence`
///
/// The function spans two layers — it reads a file (`persistence`)
/// and opens a socket (`shared`) — so placing it requires a
/// convention. We place it in the layer that *consumes* the other:
/// `persistence::config` imports `support::connect_bridge_to`, and
/// `shared` imports neither. An earlier draft inverted this and put
/// the helper in `shared/mod.rs`, which silently made the transport
/// layer depend on the persistence layer. The doc comment there
/// admitted the inversion in prose; the fix is to put the helper on
/// the correct side of the seam. See `src/shared/mod.rs`'s module
/// doc for the enforcement story.
///
/// [`DendriteBridge`]: easynet_axon::dendrite_bridge::DendriteBridge
pub fn load_and_connect(
) -> anyhow::Result<(easynet_axon::dendrite_bridge::DendriteBridge, RuntimeState)> {
    let state = load()?;
    let bridge = state.connect_bridge()?;
    Ok((bridge, state))
}

// ─── Device Credentials ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    pub node_id: String,
    pub credential_token: String,
    pub hub_endpoint: String,
    // URI v4.1.4 backend renamed the wire field `tenant_id` → `realm`
    // for every device-pairing response (CreatePairingResp,
    // PairingPreflightResp, DeviceResp).
    //
    // The Rust struct keeps `tenant_id` as the in-memory field name
    // (~15 callsites depend on it) but accepts EITHER `realm` (v4.1.4)
    // or `tenant_id` (legacy + on-disk v1 credentials.json) on the
    // wire via serde alias. `default` lets the v1 form decode when
    // only `tenant_id` is present and the v4.1.4 form when only
    // `realm` is present. Output side: `serialize` always writes
    // `tenant_id` (the field name) for backward-compat with any
    // tooling that still reads credentials.json by hand. A future
    // amendment can flip serialization to write `realm` once nothing
    // else reads the file path.
    #[serde(default, alias = "realm")]
    pub tenant_id: String,
    #[serde(default)]
    pub deploy_signature: String,
    /// Optional Hub REST API base URL (e.g. "http://localhost:8080") for local dev.
    /// When absent, derived from `hub_endpoint` by stripping scheme/port and using HTTPS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_api_base: Option<String>,
    /// URI v2 username — stable slug for the user this device is
    /// paired to. Optional during the migration window; populated
    /// by the Phase 14 backend in validate-pairing responses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Realm hub's Ed25519 pubkey (base64), captured during pairing
    /// preflight. Cross-machine cold-start fix (hub in US, CLI in
    /// SG): the device's `auto_wire_self_realm_trust` step needs
    /// this to write the hub's `(uri, pubkey, role=hub)` row into
    /// `realm-trust.toml` without needing on-host access to the
    /// hub's `~/.easynet-hub/<realm>/identity.json`. Empty when
    /// paired against a pre-v4.1.4 hub (legacy fallback path reads
    /// identity.json directly when same-host).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_pubkey_b64: Option<String>,
    /// Optional base64-encoded PEM trust anchor for the hub's
    /// public TLS listener. Self-hosted hubs with private/self-
    /// signed CAs populate this during pairing preflight so the
    /// join flow can persist a local CA pin before the daemon
    /// opens `<self>.session`. Publicly-trusted hubs leave it
    /// empty and runtime dials fall back to native OS roots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_tls_ca_pem_b64: Option<String>,
}

impl Credentials {
    /// Returns the v4.1.4 realm. After the alias-based serde change
    /// (Phase 2B'), the value lives in `tenant_id` regardless of
    /// whether the wire payload used the new `realm` field name or
    /// the legacy `tenant_id` field name. Callers should still go
    /// through this helper rather than reading `.tenant_id` directly
    /// — that way a future field rename will only need one edit.
    pub fn realm_str(&self) -> &str {
        &self.tenant_id
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
                    if DAEMON_TLS_PORTS.iter().any(|p| *p == port) {
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
            if DAEMON_TLS_PORTS.iter().any(|p| *p == port) {
                return host.to_string();
            }
        }
        authority.to_string()
    }
}

fn credentials_path() -> PathBuf {
    state_dir().join("credentials.json")
}

/// Path to the heartbeat daemon PID file.
/// Used by start.rs (write) and stop.rs (read + cleanup).
pub fn heartbeat_pid_path() -> PathBuf {
    state_dir().join("heartbeat.pid")
}

/// Path to the easynet-daemon PID file. Same shape and lifecycle as
/// `heartbeat_pid_path`: start.rs writes it after spawn, stop.rs
/// reads + signals + removes. Without it, `easynet runtime stop`
/// could only kill the heartbeat daemon and the axon runtime;
/// the easynet-daemon child stayed alive across restarts and a
/// fresh `runtime start` would spawn a SECOND daemon that loses
/// the runtime-dispatch socket bind to the older one ("another
/// process already accepts on …/runtime-dispatch.sock — refusing
/// to overwrite"). The newer daemon's responder exits and from
/// that point control.sock chat dispatches succeed exactly once
/// (whichever daemon's tokio runtime gets the connection) before
/// returning "daemon closed the connection". Pidfile + signal-on-
/// stop fixes that ghost-daemon class entirely.
pub fn easynet_daemon_pid_path() -> PathBuf {
    state_dir().join("easynet-daemon.pid")
}

pub fn save_credentials(creds: &Credentials) -> anyhow::Result<()> {
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
    let path = credentials_path();
    let data = fs::read_to_string(&path)
        .map_err(|_| anyhow::anyhow!("no credentials found — run `easynet join <token>` first"))?;
    let creds: Credentials = serde_json::from_str(&data)?;
    if creds.node_id.is_empty()
        || creds.credential_token.is_empty()
        || creds.hub_endpoint.is_empty()
        || creds.tenant_id.is_empty()
    {
        anyhow::bail!("credentials file is incomplete — run `easynet join <token>` to re-pair");
    }
    Ok(creds)
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
pub struct DeviceSettings {
    #[serde(default)]
    pub session_bridge_exec_enabled: bool,
}

fn device_settings_path() -> PathBuf {
    state_dir().join("device_settings.json")
}

pub fn load_device_settings() -> DeviceSettings {
    let path = device_settings_path();
    fs::read_to_string(&path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).ok())
        .unwrap_or_default()
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
            tenant_id: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: Some("https://api.example.com/".into()),
            username: None,
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
        };
        assert_eq!(creds.api_base(), "https://api.example.com");
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
            tenant_id: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: None,
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
        };
        assert_eq!(creds.api_base(), "https://my-hub.example.org");
    }

    #[test]
    fn runtime_state_defaults_to_axon_bridge_when_kind_missing() {
        let state: RuntimeState = serde_json::from_str(
            r#"{
                "endpoint": "axon://127.0.0.1:50051",
                "pid": 7,
                "tenant": "tenant-a"
            }"#,
        )
        .expect("deserialize legacy runtime state");
        assert_eq!(state.runtime_kind, RuntimeKind::AxonBridge);
        assert!(state.uses_bridge());
    }

    #[test]
    fn daemon_only_runtime_state_rejects_bridge_connect() {
        let state = RuntimeState {
            endpoint: "/tmp/easynet.sock".into(),
            runtime_kind: RuntimeKind::DaemonOnly,
            pid: Some(9),
            hub: None,
            tenant: Some("tenant-a".into()),
            label: None,
            started_at: None,
            credential_verified: None,
        };
        let err = match state.connect_bridge() {
            Ok(_) => panic!("daemon-only state must not bridge-connect"),
            Err(err) => err,
        };
        assert!(
            err.to_string().contains("daemon-only mode"),
            "unexpected error: {err}"
        );
    }

    // ── agents_root() migration read ─────────────────────────────────────

    // All of these tests use `HomeGuard` so they never touch the
    // developer's real `~/.easynet/` tree. `HomeGuard` serializes
    // concurrent tests via a global mutex, which is load-bearing because
    // `agents_root()` reads `HOME`.

    use crate::facade::cli::test_support::HomeGuard;

    #[test]
    fn agents_root_prefers_new_layout_when_only_new_exists() {
        let _g = HomeGuard::new();
        let new_path = state_dir().join("agents");
        fs::create_dir_all(&new_path).unwrap();

        assert_eq!(agents_root(), new_path);
    }

    #[test]
    fn agents_root_falls_back_to_legacy_when_only_legacy_exists() {
        let _g = HomeGuard::new();
        let legacy = state_dir().join("workspaces");
        fs::create_dir_all(&legacy).unwrap();

        assert_eq!(agents_root(), legacy);
    }

    #[test]
    fn agents_root_prefers_new_when_both_exist() {
        // Double-presence is a real-world shape: a user who created
        // agents on an old binary and upgrades to a newer one that has
        // begun writing to the new path will briefly hold both trees.
        // The helper must resolve the ambiguity toward the new path so
        // the rest of the codebase reads a single consistent layout.
        let _g = HomeGuard::new();
        let legacy = state_dir().join("workspaces");
        let new_path = state_dir().join("agents");
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&new_path).unwrap();

        assert_eq!(agents_root(), new_path);
    }

    #[test]
    fn agents_root_returns_legacy_path_when_neither_exists() {
        // Fresh install: no layout on disk. The helper returns the
        // legacy path so the first writer lands under
        // `~/.easynet/workspaces/`, byte-compatible with every
        // pre-rename deployment. Flipping this default is reserved
        // for the PR that introduces AgentDirectory writes.
        let _g = HomeGuard::new();
        let legacy = state_dir().join("workspaces");
        assert_eq!(agents_root(), legacy);
    }

    #[cfg(unix)]
    #[test]
    fn agents_root_survives_an_unreadable_legacy_path() {
        // A legacy `workspaces/` that the user cannot list (e.g.
        // wrong ownership after a chown) must not panic the resolver
        // or crash on every CLI call. We verify the helper still
        // returns a path — either the new layout (preferred) or the
        // legacy one — without aborting.
        use std::os::unix::fs::PermissionsExt;

        let _g = HomeGuard::new();
        let legacy = state_dir().join("workspaces");
        fs::create_dir_all(&legacy).unwrap();
        fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o000)).unwrap();

        // The helper uses `Path::exists()` internally, which only
        // checks metadata reachability — it tolerates unreadable
        // targets as long as the path resolves. Either return path
        // is acceptable here; the critical property is "no panic".
        let got = agents_root();

        // Restore permissions so `HomeGuard::drop` can clean up.
        fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o700)).unwrap();

        assert!(got.starts_with(state_dir()));
    }

    #[test]
    fn agents_root_emits_deprecation_notice_at_most_once() {
        // The deprecation notice runs through a `OnceLock`, so the
        // first call that hits the legacy-only branch prints, and
        // every subsequent call in the same process is silent. We
        // can't capture stderr directly from an integration-style
        // test, but we can still verify the helper is idempotent
        // under repeated calls: the returned path must be stable
        // and the process must not crash.
        let _g = HomeGuard::new();
        let legacy = state_dir().join("workspaces");
        fs::create_dir_all(&legacy).unwrap();

        let first = agents_root();
        let second = agents_root();
        let third = agents_root();
        assert_eq!(first, legacy);
        assert_eq!(second, legacy);
        assert_eq!(third, legacy);
    }
}
