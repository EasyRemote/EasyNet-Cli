// EasyNet CLI — Persistence Layer — daemon configuration
// =======================================================
//
// File: src/daemon/persistence/daemon_config.rs
// Description: TOML-backed configuration for the `easynet-daemon`
//              binary's gRPC InvocationServer.
//
// Scope
// -----
// This module owns the on-disk format and validated representation of
// `~/.easynet/daemon-config.toml`. The file is the single source of
// truth for three things at daemon boot:
//
//   1. Which deployment mode the binary runs in (device / hub / both)
//   2. Where it listens (UDS path always; TCP socket only in hub
//      modes), and the TLS material it serves on TCP listeners
//   3. Which hub it dials outbound from (device mode only)
//
// What this module is NOT
// -----------------------
// - It is **not** the gRPC server. `daemon/invocation` owns that.
// - It is **not** the realm trust anchor (`realm-trust.toml`). That
//   file is read by `daemon/trust/anchor` and is authored by
//   the device-pairing flow shipping in PR-7. The daemon uses both
//   files at boot but they have distinct lifecycles.
// - It does **not** rebuild listeners or TLS state after boot. The
//   SIGHUP coordinator in `daemon::boot::invocation` deliberately
//   hot-reloads only cells whose runtime owners are built for
//   replacement: `federated_peers` and `[daemon.quota]`. Mode,
//   socket paths, TCP listeners, TLS cert/key paths, hub endpoint,
//   realm, and ledger path remain boot-time invariants and require a
//   daemon restart.
//
// Invariants enforced at load time
// --------------------------------
// `DaemonConfig::load` returns `Err(DaemonConfigError)` rather than a
// usable struct whenever any of the following holds — these are the
// invariants from `pr-drafts/PR-0-spec-daemon-invocation-server.md
// §1.2`:
//
// - Invariant 1 (attack surface): a device-mode daemon never binds a
//   TCP port. `mode = "device"` plus a `listen_tcp` field present is
//   a hard error, not a warning. Rationale: device hosts live behind
//   NAT and have no business serving inbound RPC; opening one would
//   either fail silently (firewall) or expose the device.
//
// - Invariant 2 (TLS on public TCP): if `listen_tcp` is set, both
//   `tls_cert_pem` and `tls_key_pem` MUST resolve to readable files
//   at boot. Plaintext gRPC on a public-bound endpoint is forbidden
//   even though application-layer envelopes are signed; transport
//   metadata (call-id, frame size, peer IP) leaks without TLS.
//
// - Invariant 3 (UDS owner) is enforced at the bind site, not at
//   load time, because it requires real `chmod`/`chown` calls. Load
//   time only validates the path syntax.
//
// Invariant 2.1 (cert reload) is a documentation-only invariant and
// imposes no code path here. Cert/key changes require a daemon
// restart; SIGHUP does not rebuild TLS listeners.
//
// Authorship and stability
// ------------------------
// This module's public API is consumed by `src/bin/easynet-daemon.rs`
// during the gRPC server bootstrap. Its shape is referenced from the
// PR-1 spec (§1.1, §1.2) and any backwards-incompatible change to
// `DaemonConfig` requires a spec amendment plus CTO re-ratification
// per spec §0.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// Default UDS path the daemon binds for backend cliipc clients and
/// for control-plane RPC from local processes. Operator override is
/// possible via `[daemon] uds_path = "..."` but the default is what
/// every consumer (backend, CLI, install.sh) hard-codes.
pub const DEFAULT_DAEMON_UDS_PATH: &str = "~/.easynet/daemon.sock";

/// Default location the daemon reads its config from. Operator
/// override is via the daemon binary's `--config <path>` argument
/// (handled in the binary, not here).
pub const DEFAULT_DAEMON_CONFIG_PATH: &str = "~/.easynet/daemon-config.toml";
pub const DEFAULT_LEDGER_DIR: &str = "~/.easynet/billing";

/// Expand a `~/...` path against the current process's HOME. Paths
/// without the prefix are returned unchanged. Kept in
/// `persistence::daemon_config` so every caller that needs the
/// daemon's default on-disk locations resolves them the same way.
pub fn expand_home_path(path: &Path) -> PathBuf {
    let Some(raw) = path.to_str() else {
        return path.to_path_buf();
    };
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}

/// Expanded location of `~/.easynet/daemon-config.toml`.
pub fn default_config_path() -> PathBuf {
    expand_home_path(Path::new(DEFAULT_DAEMON_CONFIG_PATH))
}

/// Expanded location of the daemon's gRPC UDS socket.
pub fn default_uds_path() -> PathBuf {
    #[cfg(windows)]
    {
        return PathBuf::from(crate::support::platform::named_pipe::scoped_pipe_name(
            "daemon-grpc",
        ));
    }

    #[cfg(not(windows))]
    expand_home_path(Path::new(DEFAULT_DAEMON_UDS_PATH))
}

pub fn default_ledger_dir() -> PathBuf {
    std::env::var_os("EASYNET_WORKSPACE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|workspace| workspace.join("billing"))
        .unwrap_or_else(|| expand_home_path(Path::new(DEFAULT_LEDGER_DIR)))
}

/// Resolve the daemon's gRPC UDS socket using the local config file when
/// present, else fall back to the canonical default path.
///
/// This mirrors the boot path's contract closely enough for CLI
/// readiness probes: a custom `uds_path` in `daemon-config.toml` wins;
/// a missing / malformed config falls back to the default path because
/// callers such as `easynet start` may be the very command that is about
/// to write the minimal config.
pub fn resolved_local_uds_path() -> PathBuf {
    match DaemonConfig::load(&default_config_path()) {
        Ok(cfg) => expand_home_path(cfg.uds_path()),
        Err(_) => default_uds_path(),
    }
}

/// CLI-side resolver: same as [`resolved_local_uds_path`] but lets an
/// `EASYNET_DAEMON_GRPC_UDS` env-var override the on-disk config so
/// integration tests can point CLI subcommands at a temp daemon
/// socket. On Windows the value is interpreted as a named-pipe name
/// instead of a filesystem path; on Unix, `~/`-prefixed values
/// expand against `$HOME` so a test harness can drop a path like
/// `~/.easynet-test/daemon.sock` without re-computing it.
///
/// **Why this lives here (not in `support/`):** `support/` is the
/// leaf layer per `src/support/mod.rs` and must not depend on
/// `persistence/`. The env-override-then-config-file recipe is one
/// step on top of [`resolved_local_uds_path`] and belongs with it.
/// Callers that need the CLI-resolved path import this function directly
/// instead of going through support-layer re-export shims.
pub fn resolved_local_uds_path_with_env_override() -> PathBuf {
    let raw = std::env::var("EASYNET_DAEMON_GRPC_UDS").ok();
    match raw {
        Some(raw) if !raw.trim().is_empty() => {
            #[cfg(windows)]
            {
                PathBuf::from(raw)
            }
            #[cfg(not(windows))]
            {
                if let Some(rest) = raw.strip_prefix("~/") {
                    if let Some(home) = std::env::var_os("HOME") {
                        return PathBuf::from(home).join(rest);
                    }
                }
                PathBuf::from(raw)
            }
        }
        _ => resolved_local_uds_path(),
    }
}

/// Ensure the local daemon has at least the minimal device-mode config
/// needed to boot its gRPC/session sidecar.
///
/// Idempotent: if the canonical config file already exists, the runtime role
/// and credential-derived identity fields (`mode`, `realm`, `hub_endpoint`)
/// are synchronized to the joined device epoch. Operator-authored generic
/// fields such as custom `uds_path`, `ledger_dir`, `federated_peers`, and quota
/// survive intact. Hub-only listener fields are removed because device mode
/// must not bind public TCP.
pub fn ensure_minimal_device_config(
    creds: &crate::daemon::persistence::config::Credentials,
) -> anyhow::Result<()> {
    let path = default_config_path();
    if path.exists() {
        return sync_existing_device_config_with_credentials(&path, creds);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let realm = if creds.realm.trim().is_empty() {
        "localhost"
    } else {
        creds.realm.trim()
    };
    let hub_endpoint = creds.hub_endpoint.trim();
    let body = format!(
        "# Auto-generated by EasyNet on {now}.\n\
         # Edit by hand to add `[daemon.federated_peers]`, hub-mode\n\
         # TLS pins, or override the UDS path.\n\
         [daemon]\n\
         mode = \"device\"\n\
         realm = \"{realm}\"\n\
         hub_endpoint = \"{hub_endpoint}\"\n",
        now = chrono::Utc::now().to_rfc3339(),
        realm = realm,
        hub_endpoint = hub_endpoint,
    );
    crate::daemon::persistence::config::atomic_write(&path, body.as_bytes())?;
    Ok(())
}

/// Ensure the local daemon has a hub-mode config so its gRPC/session
/// sidecar binds the public TCP+TLS Invocation listener.
///
/// Hub mode has no upstream hub of its own (RFC-003), so unlike the
/// device generator this writes no `hub_endpoint`. The TLS material is
/// mandatory: Invariant 2 (`check_invariant_2_tls_on_tcp`) rejects a
/// `listen_tcp` without both `tls_cert_pem` and `tls_key_pem`, so a hub
/// cannot boot in plaintext. Callers must resolve real cert/key paths
/// before calling; this function does not generate them.
///
/// Idempotent in the same spirit as `ensure_minimal_device_config`: if a
/// config already exists it is left untouched. An operator-authored hub
/// config (custom `uds_path`, `federated_peers`, quota) therefore
/// survives verbatim — we only ever *create* the minimal file when none
/// is present, never rewrite an existing one.
pub fn ensure_hub_config(bind: &str, realm: &str, cert: &Path, key: &Path) -> anyhow::Result<()> {
    let path = default_config_path();
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let realm = if realm.trim().is_empty() {
        "localhost"
    } else {
        realm.trim()
    };
    let body = format!(
        "# Auto-generated by EasyNet on {now}.\n\
         # Hub mode: binds a public TCP+TLS Invocation listener. Edit by\n\
         # hand to add `[daemon.federated_peers]` or override the UDS path.\n\
         [daemon]\n\
         mode = \"hub\"\n\
         realm = \"{realm}\"\n\
         listen_tcp = \"{bind}\"\n\
         tls_cert_pem = \"{cert}\"\n\
         tls_key_pem = \"{key}\"\n",
        now = chrono::Utc::now().to_rfc3339(),
        realm = realm,
        bind = bind,
        cert = cert.display(),
        key = key.display(),
    );
    crate::daemon::persistence::config::atomic_write(&path, body.as_bytes())?;
    Ok(())
}

fn sync_existing_device_config_with_credentials(
    path: &Path,
    creds: &crate::daemon::persistence::config::Credentials,
) -> anyhow::Result<()> {
    let raw = fs::read_to_string(path)?;
    let updated = sync_existing_device_config_toml(&raw, creds)?;
    if updated != raw {
        crate::daemon::persistence::config::atomic_write(path, updated.as_bytes())?;
    }
    Ok(())
}

fn sync_existing_device_config_toml(
    raw: &str,
    creds: &crate::daemon::persistence::config::Credentials,
) -> anyhow::Result<String> {
    use anyhow::Context as _;
    use toml_edit::{value, DocumentMut};

    let mut doc: DocumentMut = raw.parse().context("parse daemon-config.toml")?;
    let daemon_table = doc
        .as_table_mut()
        .get_mut("daemon")
        .ok_or_else(|| anyhow::anyhow!("[daemon] is required in existing daemon-config.toml"))?
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[daemon] is not a TOML table"))?;

    let mode_raw = daemon_table
        .get("mode")
        .and_then(|item| item.as_str())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "[daemon].mode is required in existing daemon-config.toml; refusing to infer device mode"
            )
        })?;
    let mode = DaemonMode::parse_config_value(mode_raw)
        .ok_or_else(|| anyhow::anyhow!("[daemon].mode has unsupported value {mode_raw:?}"))?;
    let realm = if creds.realm.trim().is_empty() {
        "localhost"
    } else {
        creds.realm.trim()
    };
    let hub_endpoint = creds.hub_endpoint.trim();
    let mut changed = false;

    if mode != DaemonMode::Device {
        daemon_table.insert("mode", value(DaemonMode::Device.as_str()));
        changed = true;
    }
    if daemon_table.get("realm").and_then(|item| item.as_str()) != Some(realm) {
        daemon_table.insert("realm", value(realm));
        changed = true;
    }
    if daemon_table
        .get("hub_endpoint")
        .and_then(|item| item.as_str())
        != Some(hub_endpoint)
    {
        daemon_table.insert("hub_endpoint", value(hub_endpoint));
        changed = true;
    }
    for hub_only_field in ["listen_tcp", "tls_cert_pem", "tls_key_pem"] {
        if daemon_table.remove(hub_only_field).is_some() {
            changed = true;
        }
    }

    if changed {
        Ok(doc.to_string())
    } else {
        Ok(raw.to_string())
    }
}

/// Three deployment modes recognised by the daemon binary. Each
/// implies a distinct listener invariant set (see module docs).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DaemonMode {
    /// Consumer device behind NAT. Outbound `session.open` to a
    /// hub; never binds TCP. Backend cliipc connects to UDS.
    Device,

    /// Public-internet rendezvous. Binds both UDS (for backend
    /// cliipc) and TCP+TLS (for inbound `session.open` from
    /// remote devices). Has no upstream hub of its own under
    /// RFC-003; cross-realm hub-to-hub is RFC-005, out of scope.
    Hub,

    /// Production server colocating a hub and the EasyNet backend
    /// service. Listener invariants identical to Hub; the backend
    /// process runs as a sibling and dials the same UDS.
    Both,
}

impl DaemonMode {
    /// Stable lowercase representation used in discovery files and
    /// operator-facing diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            DaemonMode::Device => "device",
            DaemonMode::Hub => "hub",
            DaemonMode::Both => "both",
        }
    }

    fn parse_config_value(raw: &str) -> Option<Self> {
        match raw.trim() {
            "device" => Some(DaemonMode::Device),
            "hub" => Some(DaemonMode::Hub),
            "both" => Some(DaemonMode::Both),
            _ => None,
        }
    }
}

/// Parsed and invariant-validated representation of
/// `~/.easynet/daemon-config.toml`. Construct via `DaemonConfig::load`;
/// the `Deserialize`-derived shape used internally is private to
/// guarantee the invariants below.
///
/// Fields in this struct are `pub(crate)` rather than `pub` because
/// callers should access them through the methods below (so a future
/// invariant can be added without breaking call sites). The `pub(crate)`
/// keeps test code in the same crate able to construct mock values
/// directly while shielding external embedders from the field layout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonConfig {
    mode: DaemonMode,
    realm: String,
    hub_endpoint: Option<String>,
    listen_tcp: Option<SocketAddr>,
    tls_cert_pem: Option<PathBuf>,
    tls_key_pem: Option<PathBuf>,
    uds_path: PathBuf,
    ledger_dir: PathBuf,
    /// **PR-N1 commit 3a/N**. Operator-curated `realm → hub_endpoint`
    /// map the federation dispatcher consults when `federation.
    /// canonical_invoke` targets a realm that is not local.
    /// Empty = no cross-realm dispatch endpoint is configured. The
    /// route resolver then returns a typed no-route/offline answer for
    /// cross-realm invocation targets. Federated-directory snapshots are
    /// observability read models and do not synthesize dispatch endpoints;
    /// the map remains the operator's manual statement of "these are the
    /// peer realms I federate with".
    ///
    /// `BTreeMap` over `HashMap` for stable iteration order (TOML
    /// dump in operator audit + `cargo test` byte-stable
    /// expectation).
    federated_peers: BTreeMap<String, String>,
    /// #185: per-consumer invocation quota policy (caps applied per
    /// ability per window — see [`QuotaConfig`]). `None` = the feature
    /// is off and every caller is unmetered. `Some` even with no caps
    /// still means "metering wired" — the admission gate consults the
    /// store but a `0` cap leaves callers unthrottled.
    quota: Option<QuotaConfig>,
}

impl DaemonConfig {
    /// Load and validate a daemon config from a TOML file at `path`.
    ///
    /// On success, the returned `DaemonConfig` satisfies every
    /// invariant from spec §1.2 that can be checked at load time
    /// (Invariants 1, 2). UDS-owner enforcement (Invariant 3) is the
    /// caller's responsibility at bind time.
    ///
    /// `path` is expanded for `~/` prefix resolution; relative paths
    /// resolve against the daemon process's working directory at the
    /// time of call.
    pub fn load(path: &Path) -> Result<Self, DaemonConfigError> {
        let raw = fs::read_to_string(path).map_err(|source| DaemonConfigError::ReadFailed {
            path: path.to_path_buf(),
            source,
        })?;

        let parsed: RawDaemonConfig =
            toml::from_str(&raw).map_err(|source| DaemonConfigError::ParseFailed {
                path: path.to_path_buf(),
                source,
            })?;

        Self::from_raw(parsed)
    }

    /// Construct a config from already-deserialised data. Public
    /// within the crate so unit tests can exercise the invariant
    /// checks without writing TOML to disk.
    pub(crate) fn from_raw(raw: RawDaemonConfig) -> Result<Self, DaemonConfigError> {
        let RawDaemonConfig { daemon } = raw;
        let RawDaemonSection {
            mode,
            realm,
            hub_endpoint,
            listen_tcp,
            tls_cert_pem,
            tls_key_pem,
            uds_path,
            ledger_dir,
            federated_peers,
            quota,
        } = daemon;

        if realm.trim().is_empty() {
            return Err(DaemonConfigError::EmptyRealm);
        }

        Self::check_invariant_1_device_no_tcp(mode, listen_tcp.as_deref())?;
        Self::check_invariant_2_tls_on_tcp(
            listen_tcp.as_deref(),
            tls_cert_pem.as_deref(),
            tls_key_pem.as_deref(),
        )?;

        let listen_tcp_parsed = listen_tcp
            .map(|s| {
                s.parse::<SocketAddr>()
                    .map_err(|_| DaemonConfigError::InvalidListenTcp(s))
            })
            .transpose()?;

        let uds_path = uds_path.map(PathBuf::from).unwrap_or_else(default_uds_path);
        let ledger_dir = ledger_dir
            .map(PathBuf::from)
            .unwrap_or_else(default_ledger_dir);

        if matches!(mode, DaemonMode::Device) && hub_endpoint.is_none() {
            return Err(DaemonConfigError::DeviceMissingHubEndpoint);
        }

        Ok(Self {
            mode,
            realm,
            hub_endpoint,
            listen_tcp: listen_tcp_parsed,
            tls_cert_pem: tls_cert_pem.map(PathBuf::from),
            tls_key_pem: tls_key_pem.map(PathBuf::from),
            uds_path,
            ledger_dir,
            federated_peers: federated_peers.unwrap_or_default(),
            quota: quota.map(QuotaConfig::from),
        })
    }

    fn check_invariant_1_device_no_tcp(
        mode: DaemonMode,
        listen_tcp: Option<&str>,
    ) -> Result<(), DaemonConfigError> {
        if matches!(mode, DaemonMode::Device) && listen_tcp.is_some() {
            return Err(DaemonConfigError::DeviceModeBoundTcp);
        }
        Ok(())
    }

    fn check_invariant_2_tls_on_tcp(
        listen_tcp: Option<&str>,
        cert: Option<&str>,
        key: Option<&str>,
    ) -> Result<(), DaemonConfigError> {
        match (listen_tcp, cert, key) {
            (Some(_), Some(_), Some(_)) => Ok(()),
            (Some(_), _, _) => Err(DaemonConfigError::TcpWithoutTls),
            (None, _, _) => Ok(()),
        }
    }

    /// The deployment mode this daemon is configured for.
    pub fn mode(&self) -> DaemonMode {
        self.mode
    }

    /// The realm this daemon and every device it serves belong to.
    /// Used as the `realm` component when minting URAs and when
    /// deriving `join_receipt_hash` (spec §5.1).
    pub fn realm(&self) -> &str {
        &self.realm
    }

    /// Outbound hub endpoint; only present in device mode (and
    /// guaranteed present in device mode by Invariant
    /// `DeviceMissingHubEndpoint`). Returns `None` for hub / both
    /// modes.
    pub fn hub_endpoint(&self) -> Option<&str> {
        self.hub_endpoint.as_deref()
    }

    /// TCP listen address; `None` in device mode by Invariant 1.
    pub fn listen_tcp(&self) -> Option<SocketAddr> {
        self.listen_tcp
    }

    /// Path to the TLS certificate PEM; required iff `listen_tcp`
    /// is set, by Invariant 2.
    pub fn tls_cert_pem(&self) -> Option<&Path> {
        self.tls_cert_pem.as_deref()
    }

    /// Path to the TLS private key PEM; same constraint as
    /// `tls_cert_pem`.
    pub fn tls_key_pem(&self) -> Option<&Path> {
        self.tls_key_pem.as_deref()
    }

    /// Path to the daemon's UDS socket. Always present (defaulted
    /// from `DEFAULT_DAEMON_UDS_PATH` when the operator does not
    /// override).
    pub fn uds_path(&self) -> &Path {
        &self.uds_path
    }

    pub fn ledger_dir(&self) -> &Path {
        &self.ledger_dir
    }

    /// **PR-N1 commit 3a/N**. Operator-curated cross-realm
    /// dispatch map. Empty when the operator did not configure any
    /// federation peers — the federation dispatcher then returns typed
    /// no-route/offline for cross-realm `Invocation::Invoke` calls.
    pub fn federated_peers(&self) -> &BTreeMap<String, String> {
        &self.federated_peers
    }

    /// The per-consumer invocation quota policy (#185), or `None` when
    /// the operator did not configure a `[daemon.quota]` table (the
    /// feature is off; every caller is unmetered).
    #[must_use]
    pub fn quota(&self) -> Option<&QuotaConfig> {
        self.quota.as_ref()
    }
}

/// Internal deserialisation shape. Pub-within-crate only so unit
/// tests can build instances without round-tripping through TOML.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RawDaemonConfig {
    pub(crate) daemon: RawDaemonSection,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawDaemonSection {
    pub(crate) mode: DaemonMode,
    pub(crate) realm: String,
    #[serde(default)]
    pub(crate) hub_endpoint: Option<String>,
    #[serde(default)]
    pub(crate) listen_tcp: Option<String>,
    #[serde(default)]
    pub(crate) tls_cert_pem: Option<String>,
    #[serde(default)]
    pub(crate) tls_key_pem: Option<String>,
    #[serde(default)]
    pub(crate) uds_path: Option<String>,
    #[serde(default)]
    pub(crate) ledger_dir: Option<String>,
    /// PR-N1 commit 3a/N: operator-curated `realm → hub_endpoint`
    /// map for cross-realm `Invocation::Invoke` routing.
    /// `#[serde(default)]` so configs that omit federation routing
    /// policy load with an empty map.
    #[serde(default)]
    pub(crate) federated_peers: Option<BTreeMap<String, String>>,
    /// #185: per-consumer invocation quota. Absent = the whole
    /// feature is off (every caller unmetered). `#[serde(default)]`
    /// so existing configs load unchanged.
    #[serde(default)]
    pub(crate) quota: Option<RawQuotaSection>,
}

/// Raw `[daemon.quota]` sub-table. All fields optional so an empty
/// `[daemon.quota]` table is valid (and equivalent to "metering on
/// with no caps").
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RawQuotaSection {
    #[serde(default)]
    pub(crate) default_cap_per_window: Option<i32>,
    #[serde(default)]
    pub(crate) window_ms: Option<i64>,
    #[serde(default)]
    pub(crate) per_consumer: Option<BTreeMap<String, i32>>,
}

/// Per-consumer invocation quota policy (#185). The daemon's
/// admission gate consults this AFTER permission is granted, to meter
/// an already-admitted caller. A consumer's cap is its
/// `per_consumer` entry if present, else `default_cap_per_window`. A
/// cap `<= 0` means that consumer is unmetered.
///
/// **Granularity (read before setting a cap).** The cap is keyed by
/// consumer URA but applies *per ability, per window*: the enforcement
/// counter ([`crate::daemon::invocation::admission::usage_quota`]) windows on
/// `(consumer_ura, ability)`, so a cap of `N` admits up to `N` calls
/// to *each distinct ability* per window, not `N` calls total across
/// all abilities. This is deliberate — one hot ability cannot starve a
/// consumer's budget for the rest — but an operator sizing a cap must
/// reason per ability, not per consumer-aggregate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuotaConfig {
    /// Per-ability, per-window request cap for any consumer without a
    /// `per_consumer` override. `0` (the default) = unmetered. See the
    /// struct-level granularity note: this bounds calls to each
    /// distinct ability independently, not the consumer's total.
    default_cap_per_window: i32,
    /// Tumbling-window width in millis. Defaults to 60_000 (1 min).
    window_ms: i64,
    /// Per-consumer-URA cap overrides. Each value is itself a
    /// per-ability, per-window cap (see the struct-level granularity
    /// note). `BTreeMap` for stable audit / test ordering.
    per_consumer: BTreeMap<String, i32>,
}

impl QuotaConfig {
    /// Default window width when the operator omits `window_ms`.
    pub const DEFAULT_WINDOW_MS: i64 = 60_000;

    /// Construct a policy directly. Used by the owner-facing
    /// `quota` CLI verb (which builds and persists a policy) and by
    /// tests. `window_ms <= 0` is clamped to [`Self::DEFAULT_WINDOW_MS`]
    /// so the window arithmetic always has a positive width.
    #[must_use]
    pub fn new(
        default_cap_per_window: i32,
        window_ms: i64,
        per_consumer: BTreeMap<String, i32>,
    ) -> Self {
        Self {
            default_cap_per_window,
            window_ms: if window_ms > 0 {
                window_ms
            } else {
                Self::DEFAULT_WINDOW_MS
            },
            per_consumer,
        }
    }

    /// The per-ability, per-window cap that applies to `consumer_ura`:
    /// its override if present, otherwise the default cap. A return
    /// `<= 0` means unmetered. The returned cap bounds each distinct
    /// ability independently (see the struct-level granularity note).
    #[must_use]
    pub fn cap_for(&self, consumer_ura: &str) -> i32 {
        self.per_consumer
            .get(consumer_ura)
            .copied()
            .unwrap_or(self.default_cap_per_window)
    }

    /// Tumbling-window width in millis.
    #[must_use]
    pub fn window_ms(&self) -> i64 {
        self.window_ms
    }

    /// The default per-window cap (for consumers without an override).
    #[must_use]
    pub fn default_cap_per_window(&self) -> i32 {
        self.default_cap_per_window
    }

    /// Read-only view of the per-consumer overrides.
    #[must_use]
    pub fn per_consumer(&self) -> &BTreeMap<String, i32> {
        &self.per_consumer
    }
}

impl From<RawQuotaSection> for QuotaConfig {
    fn from(raw: RawQuotaSection) -> Self {
        Self::new(
            raw.default_cap_per_window.unwrap_or(0),
            raw.window_ms.unwrap_or(Self::DEFAULT_WINDOW_MS),
            raw.per_consumer.unwrap_or_default(),
        )
    }
}

/// Every way `DaemonConfig::load` can fail. Each variant maps to a
/// distinct operator-fixable mistake; the daemon binary surfaces the
/// `Display` form to stderr at boot and exits non-zero.
#[derive(Debug, Error)]
pub enum DaemonConfigError {
    #[error("failed to read daemon config at {path}: {source}")]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse daemon config at {path}: {source}")]
    ParseFailed {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("[daemon] realm is empty; pick a non-empty string identifying your federation")]
    EmptyRealm,

    #[error(
        "[daemon] mode = \"device\" but listen_tcp is set; device-mode daemons must not bind \
         a TCP port (Invariant 1, spec §1.2)"
    )]
    DeviceModeBoundTcp,

    #[error(
        "[daemon] listen_tcp is set but tls_cert_pem or tls_key_pem is missing; plaintext \
         gRPC on a public TCP port is forbidden (Invariant 2, spec §1.2)"
    )]
    TcpWithoutTls,

    #[error("[daemon] listen_tcp = \"{0}\" is not a valid host:port socket address")]
    InvalidListenTcp(String),

    #[error(
        "[daemon] mode = \"device\" but hub_endpoint is missing; device-mode daemons must \
         dial a hub for outbound `session.open` (spec §1.3)"
    )]
    DeviceMissingHubEndpoint,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::commands::test_support::HomeGuard;

    fn raw(
        mode: DaemonMode,
        realm: &str,
        hub: Option<&str>,
        listen: Option<&str>,
        cert: Option<&str>,
        key: Option<&str>,
    ) -> RawDaemonConfig {
        RawDaemonConfig {
            daemon: RawDaemonSection {
                mode,
                realm: realm.to_string(),
                hub_endpoint: hub.map(str::to_string),
                listen_tcp: listen.map(str::to_string),
                tls_cert_pem: cert.map(str::to_string),
                tls_key_pem: key.map(str::to_string),
                uds_path: None,
                ledger_dir: None,
                federated_peers: None,
                quota: None,
            },
        }
    }

    #[test]
    fn device_mode_with_hub_endpoint_is_valid() {
        let cfg = DaemonConfig::from_raw(raw(
            DaemonMode::Device,
            "easynet.run",
            Some("https://hub.example.com:50051"),
            None,
            None,
            None,
        ))
        .expect("valid device config");

        assert_eq!(cfg.mode(), DaemonMode::Device);
        assert_eq!(cfg.realm(), "easynet.run");
        assert_eq!(cfg.hub_endpoint(), Some("https://hub.example.com:50051"));
        assert!(cfg.listen_tcp().is_none());
    }

    #[test]
    fn quota_absent_means_feature_off() {
        let cfg = DaemonConfig::from_raw(raw(
            DaemonMode::Device,
            "easynet.run",
            Some("https://hub.example.com:50051"),
            None,
            None,
            None,
        ))
        .expect("default config");
        assert!(
            cfg.quota().is_none(),
            "no [daemon.quota] table → metering off"
        );
    }

    #[test]
    fn quota_table_parses_caps_window_and_overrides() {
        let parsed: RawDaemonConfig = toml::from_str(
            r#"
            [daemon]
            mode = "device"
            realm = "easynet.run"
            hub_endpoint = "https://hub.example.com:50051"

            [daemon.quota]
            default_cap_per_window = 100
            window_ms = 30000

            [daemon.quota.per_consumer]
            "easynet:///r/easynet.run/user/alice" = 5
            "#,
        )
        .expect("quota TOML parses");
        let cfg = DaemonConfig::from_raw(parsed).expect("valid config");
        let quota = cfg.quota().expect("quota configured");

        assert_eq!(quota.default_cap_per_window(), 100);
        assert_eq!(quota.window_ms(), 30_000);
        // Override beats the default; an unlisted consumer falls back.
        assert_eq!(quota.cap_for("easynet:///r/easynet.run/user/alice"), 5);
        assert_eq!(quota.cap_for("easynet:///r/easynet.run/user/bob"), 100);
    }

    #[test]
    fn empty_quota_table_means_metering_on_with_default_window() {
        let parsed: RawDaemonConfig = toml::from_str(
            r#"
            [daemon]
            mode = "device"
            realm = "easynet.run"
            hub_endpoint = "https://hub.example.com:50051"

            [daemon.quota]
            "#,
        )
        .expect("empty quota table parses");
        let cfg = DaemonConfig::from_raw(parsed).expect("valid config");
        let quota = cfg.quota().expect("an empty table still wires metering");
        assert_eq!(quota.default_cap_per_window(), 0, "no cap → unmetered caps");
        assert_eq!(quota.window_ms(), QuotaConfig::DEFAULT_WINDOW_MS);
    }

    #[test]
    fn quota_non_positive_window_clamps_to_default() {
        let parsed: RawDaemonConfig = toml::from_str(
            r#"
            [daemon]
            mode = "device"
            realm = "easynet.run"
            hub_endpoint = "https://hub.example.com:50051"

            [daemon.quota]
            default_cap_per_window = 10
            window_ms = -1
            "#,
        )
        .expect("quota TOML parses");
        let cfg = DaemonConfig::from_raw(parsed).expect("valid config");
        assert_eq!(
            cfg.quota().expect("quota configured").window_ms(),
            QuotaConfig::DEFAULT_WINDOW_MS,
            "hand-edited non-positive windows must not collapse to a 1ms quota window"
        );
    }

    #[test]
    fn ledger_dir_defaults_and_can_be_configured() {
        let mut raw = raw(
            DaemonMode::Device,
            "easynet.run",
            Some("https://hub.example.com:50051"),
            None,
            None,
            None,
        );
        let defaulted = DaemonConfig::from_raw(raw.clone()).expect("default config");
        assert!(defaulted.ledger_dir().ends_with(".easynet/billing"));

        raw.daemon.ledger_dir = Some("/tmp/easynet-workspace/billing".to_string());
        let configured = DaemonConfig::from_raw(raw).expect("configured ledger dir");
        assert_eq!(
            configured.ledger_dir(),
            Path::new("/tmp/easynet-workspace/billing")
        );
    }

    #[test]
    fn retired_billing_dir_key_is_rejected() {
        let err = toml::from_str::<RawDaemonConfig>(
            r#"
            [daemon]
            mode = "device"
            realm = "easynet.run"
            hub_endpoint = "https://hub.example.com:50051"
            billing_dir = "/tmp/easynet-workspace/billing"
            "#,
        )
        .expect_err("retired billing_dir must not deserialize as ledger_dir");
        assert!(
            err.to_string().contains("billing_dir"),
            "error should name retired billing_dir field: {err}"
        );
    }

    #[test]
    fn retired_directory_auto_route_key_is_rejected() {
        let err = toml::from_str::<RawDaemonConfig>(
            r#"
            [daemon]
            mode = "device"
            realm = "easynet.run"
            hub_endpoint = "https://hub.example.com:50051"
            allow_directory_auto_route = true
            "#,
        )
        .expect_err("retired directory auto-route switch must not deserialize");
        assert!(
            err.to_string().contains("allow_directory_auto_route"),
            "error should name retired allow_directory_auto_route field: {err}"
        );
    }

    #[test]
    fn invariant_1_device_with_tcp_is_rejected() {
        let err = DaemonConfig::from_raw(raw(
            DaemonMode::Device,
            "easynet.run",
            Some("https://hub.example.com:50051"),
            Some("0.0.0.0:50051"),
            None,
            None,
        ))
        .expect_err("device + listen_tcp must be rejected");

        assert!(matches!(err, DaemonConfigError::DeviceModeBoundTcp));
    }

    #[test]
    fn invariant_2_tcp_without_cert_is_rejected() {
        let err = DaemonConfig::from_raw(raw(
            DaemonMode::Hub,
            "easynet.run",
            None,
            Some("0.0.0.0:50051"),
            None,
            None,
        ))
        .expect_err("hub + listen_tcp without tls must be rejected");

        assert!(matches!(err, DaemonConfigError::TcpWithoutTls));
    }

    #[test]
    fn invariant_2_tcp_with_cert_only_is_rejected() {
        let err = DaemonConfig::from_raw(raw(
            DaemonMode::Hub,
            "easynet.run",
            None,
            Some("0.0.0.0:50051"),
            Some("/etc/easynet/tls/cert.pem"),
            None,
        ))
        .expect_err("hub + listen_tcp with cert but no key must be rejected");

        assert!(matches!(err, DaemonConfigError::TcpWithoutTls));
    }

    #[test]
    fn hub_mode_with_full_tls_is_valid() {
        let cfg = DaemonConfig::from_raw(raw(
            DaemonMode::Hub,
            "easynet.run",
            None,
            Some("0.0.0.0:50051"),
            Some("/etc/easynet/tls/cert.pem"),
            Some("/etc/easynet/tls/key.pem"),
        ))
        .expect("valid hub config");

        assert_eq!(cfg.mode(), DaemonMode::Hub);
        assert_eq!(
            cfg.listen_tcp(),
            Some("0.0.0.0:50051".parse::<SocketAddr>().unwrap())
        );
        assert_eq!(
            cfg.tls_cert_pem().and_then(Path::to_str),
            Some("/etc/easynet/tls/cert.pem")
        );
    }

    #[test]
    fn both_mode_with_full_tls_is_valid() {
        let cfg = DaemonConfig::from_raw(raw(
            DaemonMode::Both,
            "easynet.run",
            None,
            Some("0.0.0.0:50051"),
            Some("/etc/easynet/tls/cert.pem"),
            Some("/etc/easynet/tls/key.pem"),
        ))
        .expect("valid both-mode config");

        assert_eq!(cfg.mode(), DaemonMode::Both);
    }

    #[test]
    fn empty_realm_is_rejected() {
        let err = DaemonConfig::from_raw(raw(
            DaemonMode::Hub,
            "",
            None,
            Some("0.0.0.0:50051"),
            Some("/etc/easynet/tls/cert.pem"),
            Some("/etc/easynet/tls/key.pem"),
        ))
        .expect_err("empty realm must be rejected");

        assert!(matches!(err, DaemonConfigError::EmptyRealm));
    }

    #[test]
    fn whitespace_only_realm_is_rejected() {
        let err = DaemonConfig::from_raw(raw(
            DaemonMode::Hub,
            "   ",
            None,
            Some("0.0.0.0:50051"),
            Some("/etc/easynet/tls/cert.pem"),
            Some("/etc/easynet/tls/key.pem"),
        ))
        .expect_err("whitespace-only realm must be rejected");

        assert!(matches!(err, DaemonConfigError::EmptyRealm));
    }

    #[test]
    fn invalid_listen_tcp_is_rejected() {
        let err = DaemonConfig::from_raw(raw(
            DaemonMode::Hub,
            "easynet.run",
            None,
            Some("not a socket address"),
            Some("/etc/easynet/tls/cert.pem"),
            Some("/etc/easynet/tls/key.pem"),
        ))
        .expect_err("garbage listen_tcp must be rejected");

        assert!(matches!(err, DaemonConfigError::InvalidListenTcp(_)));
    }

    #[test]
    fn device_mode_without_hub_endpoint_is_rejected() {
        let err = DaemonConfig::from_raw(raw(
            DaemonMode::Device,
            "easynet.run",
            None,
            None,
            None,
            None,
        ))
        .expect_err("device mode with no hub_endpoint must be rejected");

        assert!(matches!(err, DaemonConfigError::DeviceMissingHubEndpoint));
    }

    #[test]
    fn default_uds_path_is_used_when_omitted() {
        let _g = HomeGuard::new();
        let cfg = DaemonConfig::from_raw(raw(
            DaemonMode::Hub,
            "easynet.run",
            None,
            Some("0.0.0.0:50051"),
            Some("/etc/easynet/tls/cert.pem"),
            Some("/etc/easynet/tls/key.pem"),
        ))
        .expect("valid hub config");

        #[cfg(windows)]
        {
            let actual = cfg.uds_path().display().to_string();
            assert!(
                actual.starts_with(r"\\.\pipe\easynet-daemon-grpc-"),
                "unexpected windows daemon pipe: {actual}"
            );
            return;
        }

        // The default UDS path constant carries a literal `~`; the
        // factory expands `$HOME` through `default_uds_path()` so
        // downstream `bind(2)` calls receive a real absolute path.
        // Assert the expanded shape rather than the constant — pinning
        // the literal would lock the test to a pre-expansion code
        // path that no longer exists.
        let actual = cfg.uds_path();
        assert!(
            actual.is_absolute(),
            "default uds path must expand to an absolute path, got {}",
            actual.display()
        );
        assert!(
            actual.ends_with(".easynet/daemon.sock"),
            "default uds path must terminate at .easynet/daemon.sock, got {}",
            actual.display()
        );
    }

    #[test]
    fn default_path_helpers_expand_home() {
        let _g = HomeGuard::new();
        assert!(
            default_config_path().ends_with(".easynet/daemon-config.toml"),
            "unexpected default config path: {}",
            default_config_path().display()
        );

        #[cfg(windows)]
        {
            let actual = default_uds_path().display().to_string();
            assert!(
                actual.starts_with(r"\\.\pipe\easynet-daemon-grpc-"),
                "unexpected windows daemon pipe path: {actual}"
            );
            return;
        }

        assert!(
            default_uds_path().ends_with(".easynet/daemon.sock"),
            "unexpected default uds path: {}",
            default_uds_path().display()
        );
    }

    #[test]
    fn ensure_minimal_device_config_writes_default_file_and_syncs_device_fields() {
        let _g = HomeGuard::new();
        let mut creds = crate::daemon::persistence::config::Credentials {
            node_id: "node-1".into(),
            credential_token: "token".into(),
            hub_endpoint: "https://hub.example:50443".into(),
            realm: "tenant-a".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };

        ensure_minimal_device_config(&creds).expect("write minimal config");
        let path = default_config_path();
        let body = std::fs::read_to_string(&path).expect("read minimal config");
        assert!(body.contains("mode = \"device\""));
        assert!(body.contains("realm = \"tenant-a\""));
        assert!(body.contains("hub_endpoint = \"https://hub.example:50443\""));

        std::fs::write(
            &path,
            r#"[daemon]
mode = "device"
realm = "old-tenant"
hub_endpoint = "https://hub:50443"
uds_path = "/tmp/custom.sock"

[daemon.federated_peers]
"tenant-b" = "https://hub-b:50443"
"#,
        )
        .expect("overwrite config");
        creds.hub_endpoint = "https://127.0.0.1:50443".into();
        ensure_minimal_device_config(&creds).expect("sync device config");
        let synced = std::fs::read_to_string(&path).expect("read synced config");
        assert!(synced.contains("realm = \"tenant-a\""));
        assert!(synced.contains("hub_endpoint = \"https://127.0.0.1:50443\""));
        assert!(synced.contains("uds_path = \"/tmp/custom.sock\""));
        assert!(synced.contains("\"tenant-b\" = \"https://hub-b:50443\""));
    }

    #[test]
    fn ensure_minimal_device_config_converges_hub_mode_config_to_device_epoch() {
        let _g = HomeGuard::new();
        let creds = crate::daemon::persistence::config::Credentials {
            node_id: "node-1".into(),
            credential_token: "token".into(),
            hub_endpoint: "https://127.0.0.1:50443".into(),
            realm: "tenant-a".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        let path = default_config_path();
        std::fs::create_dir_all(path.parent().expect("config parent")).expect("mkdir");
        let raw = r#"[daemon]
mode = "both"
realm = "hub-realm"
listen_tcp = "0.0.0.0:50443"
tls_cert_pem = "/tmp/cert.pem"
tls_key_pem = "/tmp/key.pem"
uds_path = "/tmp/operator.sock"

[daemon.federated_peers]
"tenant-b" = "https://hub-b:50443"
"#;
        std::fs::write(&path, raw).expect("write hub config");

        ensure_minimal_device_config(&creds).expect("hub config converged to device");
        let converged = std::fs::read_to_string(&path).expect("read converged config");
        assert!(converged.contains("mode = \"device\""));
        assert!(converged.contains("realm = \"tenant-a\""));
        assert!(converged.contains("hub_endpoint = \"https://127.0.0.1:50443\""));
        assert!(converged.contains("uds_path = \"/tmp/operator.sock\""));
        assert!(converged.contains("\"tenant-b\" = \"https://hub-b:50443\""));
        assert!(!converged.contains("listen_tcp"));
        assert!(!converged.contains("tls_cert_pem"));
        assert!(!converged.contains("tls_key_pem"));

        let cfg = DaemonConfig::load(&path).expect("converged device config must load");
        assert_eq!(cfg.mode(), DaemonMode::Device);
        assert!(cfg.listen_tcp().is_none());
    }

    #[test]
    fn ensure_minimal_device_config_rejects_existing_config_without_explicit_mode() {
        let _g = HomeGuard::new();
        let creds = crate::daemon::persistence::config::Credentials {
            node_id: "node-1".into(),
            credential_token: "token".into(),
            hub_endpoint: "https://127.0.0.1:50443".into(),
            realm: "tenant-a".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        let path = default_config_path();
        std::fs::create_dir_all(path.parent().expect("config parent")).expect("mkdir");
        let raw = r#"[daemon]
realm = "old-tenant"
hub_endpoint = "https://hub:50443"
"#;
        std::fs::write(&path, raw).expect("write malformed config");

        let error = ensure_minimal_device_config(&creds)
            .expect_err("existing config without mode must fail closed");
        assert!(
            error.to_string().contains("[daemon].mode is required"),
            "unexpected error: {error:#}"
        );
        let unchanged = std::fs::read_to_string(&path).expect("read config");
        assert_eq!(unchanged, raw);
    }

    #[test]
    fn ensure_minimal_device_config_rejects_existing_config_with_unknown_mode() {
        let _g = HomeGuard::new();
        let creds = crate::daemon::persistence::config::Credentials {
            node_id: "node-1".into(),
            credential_token: "token".into(),
            hub_endpoint: "https://127.0.0.1:50443".into(),
            realm: "tenant-a".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: Some("alice".into()),
            user_id: Some("user-alice".into()),
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
            join_receipt_hash: None,
        };
        let path = default_config_path();
        std::fs::create_dir_all(path.parent().expect("config parent")).expect("mkdir");
        let raw = r#"[daemon]
mode = "controller"
realm = "old-tenant"
hub_endpoint = "https://hub:50443"
"#;
        std::fs::write(&path, raw).expect("write malformed config");

        let error = ensure_minimal_device_config(&creds)
            .expect_err("existing config with unknown mode must fail closed");
        assert!(
            error
                .to_string()
                .contains("[daemon].mode has unsupported value"),
            "unexpected error: {error:#}"
        );
        let unchanged = std::fs::read_to_string(&path).expect("read config");
        assert_eq!(unchanged, raw);
    }

    #[test]
    fn ensure_hub_config_writes_tls_listener_config_that_loads() {
        let _g = HomeGuard::new();
        let cert = PathBuf::from("/tmp/hub-cert.pem");
        let key = PathBuf::from("/tmp/hub-key.pem");

        ensure_hub_config("0.0.0.0:50051", "tenant-a", &cert, &key).expect("write hub config");
        let path = default_config_path();
        let body = std::fs::read_to_string(&path).expect("read hub config");
        assert!(body.contains("mode = \"hub\""));
        assert!(body.contains("realm = \"tenant-a\""));
        assert!(body.contains("listen_tcp = \"0.0.0.0:50051\""));
        assert!(body.contains("tls_cert_pem = \"/tmp/hub-cert.pem\""));
        assert!(body.contains("tls_key_pem = \"/tmp/hub-key.pem\""));
        // A hub has no upstream hub of its own.
        assert!(!body.contains("hub_endpoint"));

        // The generated file must satisfy every load-time invariant,
        // including Invariant 2 (TLS material present on a TCP listener).
        let cfg = DaemonConfig::load(&path).expect("generated hub config must load");
        assert_eq!(cfg.mode(), DaemonMode::Hub);
        assert!(cfg.hub_endpoint().is_none());
    }

    #[test]
    fn ensure_hub_config_empty_realm_falls_back_to_localhost() {
        let _g = HomeGuard::new();
        ensure_hub_config(
            "0.0.0.0:50051",
            "   ",
            &PathBuf::from("/tmp/c.pem"),
            &PathBuf::from("/tmp/k.pem"),
        )
        .expect("write hub config");
        let body = std::fs::read_to_string(default_config_path()).expect("read hub config");
        assert!(body.contains("realm = \"localhost\""));
    }

    #[test]
    fn ensure_hub_config_does_not_overwrite_existing_config() {
        let _g = HomeGuard::new();
        let path = default_config_path();
        std::fs::create_dir_all(path.parent().expect("config parent")).expect("mkdir");
        let raw = r#"[daemon]
mode = "both"
realm = "operator-realm"
listen_tcp = "0.0.0.0:9443"
tls_cert_pem = "/etc/easynet/cert.pem"
tls_key_pem = "/etc/easynet/key.pem"
"#;
        std::fs::write(&path, raw).expect("write operator config");

        ensure_hub_config(
            "0.0.0.0:50051",
            "tenant-a",
            &PathBuf::from("/tmp/c.pem"),
            &PathBuf::from("/tmp/k.pem"),
        )
        .expect("existing config left alone");
        let unchanged = std::fs::read_to_string(&path).expect("read config");
        assert_eq!(unchanged, raw);
    }
}
