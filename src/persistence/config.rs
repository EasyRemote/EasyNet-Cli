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
//     let bridge = shared::connect_bridge_to(endpoint)?;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub endpoint: String,
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

    /// Open a [`DendriteBridge`] to this runtime's endpoint, using the
    /// shared connect-timeout budget.
    ///
    /// Thin composition of [`crate::shared::connect_bridge_to`] —
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
    pub fn connect_bridge(
        &self,
    ) -> anyhow::Result<easynet_axon::dendrite_bridge::DendriteBridge> {
        crate::shared::connect_bridge_to(&self.endpoint)
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
/// `persistence::config` imports `shared::connect_bridge_to`, and
/// `shared` imports neither. An earlier draft inverted this and put
/// the helper in `shared/mod.rs`, which silently made the transport
/// layer depend on the persistence layer. The doc comment there
/// admitted the inversion in prose; the fix is to put the helper on
/// the correct side of the seam. See `src/shared/mod.rs`'s module
/// doc for the enforcement story.
///
/// [`DendriteBridge`]: easynet_axon::dendrite_bridge::DendriteBridge
pub fn load_and_connect() -> anyhow::Result<(
    easynet_axon::dendrite_bridge::DendriteBridge,
    RuntimeState,
)> {
    let state = load()?;
    let bridge = state.connect_bridge()?;
    Ok((bridge, state))
}

// ─── Device Credentials ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub node_id: String,
    pub credential_token: String,
    pub hub_endpoint: String,
    pub tenant_id: String,
    #[serde(default)]
    pub deploy_signature: String,
    /// Optional Hub REST API base URL (e.g. "http://localhost:8080") for local dev.
    /// When absent, derived from `hub_endpoint` by stripping scheme/port and using HTTPS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_api_base: Option<String>,
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

/// Extract the hostname from an endpoint URL for REST API calls.
///
/// For `axon://` endpoints, strips the gRPC port since the REST API uses HTTPS/443.
/// For `http://`/`https://` endpoints, preserves the authority (host:port) as-is.
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
            // http/https — preserve port if present.
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
        // http/https — preserve port for non-standard setups.
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
    fn credentials_api_base_prefers_override_and_trims_trailing_slash() {
        let creds = Credentials {
            node_id: "n".into(),
            credential_token: "t".into(),
            hub_endpoint: "axon://easynet.run:50051".into(),
            tenant_id: "tenant".into(),
            deploy_signature: String::new(),
            hub_api_base: Some("https://api.example.com/".into()),
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
        };
        assert_eq!(creds.api_base(), "https://my-hub.example.org");
    }
}
