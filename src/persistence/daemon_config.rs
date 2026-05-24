// EasyNet CLI — Persistence Layer — daemon configuration
// =======================================================
//
// File: src/persistence/daemon_config.rs
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
// - It is **not** the gRPC server. `services/axon_serve` owns that.
// - It is **not** the realm trust anchor (`realm-trust.toml`). That
//   file is read by `services/realm_trust_anchor` and is authored by
//   the device-pairing flow shipping in PR-7. The daemon uses both
//   files at boot but they have distinct lifecycles.
// - It does **not** hot-reload. The TOML is parsed once at boot.
//   Operator workflow for cert renewal under Let's Encrypt is
//   `systemctl restart easynet-daemon` after `certbot renew`; see
//   `docs/daemon-config.md` (PR-1 deliverable). File-watch reload is
//   a future RFC, explicitly out of RFC-003 scope.
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
// imposes no code path here. PR-1 explicitly does not implement
// cert hot-reload.
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
pub const DEFAULT_BILLING_DIR: &str = "~/.easynet/billing";

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
        return PathBuf::from(crate::support::named_pipe::scoped_pipe_name("daemon-grpc"));
    }

    #[cfg(not(windows))]
    expand_home_path(Path::new(DEFAULT_DAEMON_UDS_PATH))
}

pub fn default_billing_dir() -> PathBuf {
    std::env::var_os("EASYNET_WORKSPACE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|workspace| workspace.join("billing"))
        .unwrap_or_else(|| expand_home_path(Path::new(DEFAULT_BILLING_DIR)))
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

/// Ensure the local daemon has at least the minimal device-mode config
/// needed to boot its gRPC/session sidecar.
///
/// Idempotent: if the canonical config file already exists in device
/// mode, only the credential-derived identity fields (`realm`,
/// `hub_endpoint`) are synchronized. Operator-authored fields such as
/// custom `uds_path` and `federated_peers` survive intact. Hub/both
/// mode configs are left untouched because they describe a different
/// deployment topology.
pub fn ensure_minimal_device_config(
    creds: &crate::persistence::config::Credentials,
) -> anyhow::Result<()> {
    let path = default_config_path();
    if path.exists() {
        return sync_existing_device_config_with_credentials(&path, creds);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let realm = if creds.tenant_id.trim().is_empty() {
        "localhost"
    } else {
        creds.tenant_id.trim()
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
    crate::persistence::config::atomic_write(&path, body.as_bytes())?;
    Ok(())
}

fn sync_existing_device_config_with_credentials(
    path: &Path,
    creds: &crate::persistence::config::Credentials,
) -> anyhow::Result<()> {
    let raw = fs::read_to_string(path)?;
    let updated = sync_existing_device_config_toml(&raw, creds)?;
    if updated != raw {
        crate::persistence::config::atomic_write(path, updated.as_bytes())?;
    }
    Ok(())
}

fn sync_existing_device_config_toml(
    raw: &str,
    creds: &crate::persistence::config::Credentials,
) -> anyhow::Result<String> {
    use anyhow::Context as _;
    use toml_edit::{value, DocumentMut, Item, Table};

    let mut doc: DocumentMut = raw.parse().context("parse daemon-config.toml")?;
    let daemon_item = doc
        .as_table_mut()
        .entry("daemon")
        .or_insert_with(|| Item::Table(Table::new()));
    let daemon_table = daemon_item
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[daemon] is not a TOML table"))?;

    let mode = daemon_table
        .get("mode")
        .and_then(|item| item.as_str())
        .unwrap_or("device");
    if mode != "device" {
        return Ok(raw.to_string());
    }

    let realm = if creds.tenant_id.trim().is_empty() {
        "localhost"
    } else {
        creds.tenant_id.trim()
    };
    let hub_endpoint = creds.hub_endpoint.trim();
    let mut changed = false;

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
    /// Consumer device behind NAT. Outbound `<self>.session` to a
    /// hub; never binds TCP. Backend cliipc connects to UDS.
    Device,

    /// Public-internet rendezvous. Binds both UDS (for backend
    /// cliipc) and TCP+TLS (for inbound `<self>.session` from
    /// remote devices). Has no upstream hub of its own under
    /// RFC-003; cross-realm hub-to-hub is RFC-005, out of scope.
    Hub,

    /// Production server colocating a hub and the EasyNet backend
    /// service. Listener invariants identical to Hub; the backend
    /// process runs as a sibling and dials the same UDS.
    Both,
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
    billing_dir: PathBuf,
    /// **PR-N1 commit 3a/N**. Operator-curated `tenant → hub_uri`
    /// map the federation dispatcher consults when `federation.
    /// forward_invoke` targets a tenant whose realm is not local.
    /// Empty = no cross-tenant routing configured (legacy
    /// `target_online: false` fallback). PR-N3 will replace this
    /// hand-curated map with the auto-discovered cross-realm
    /// directory; until then the map is the operator's manual
    /// statement of "these are the peer realms I federate with".
    ///
    /// `BTreeMap` over `HashMap` for stable iteration order (TOML
    /// dump in operator audit + `cargo test` byte-stable
    /// expectation).
    federated_peers: BTreeMap<String, String>,
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
            billing_dir,
            federated_peers,
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
        let billing_dir = billing_dir
            .map(PathBuf::from)
            .unwrap_or_else(default_billing_dir);

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
            billing_dir,
            federated_peers: federated_peers.unwrap_or_default(),
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
    /// Used as the `realm` component when minting URIs and when
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

    pub fn billing_dir(&self) -> &Path {
        &self.billing_dir
    }

    /// **PR-N1 commit 3a/N**. Operator-curated cross-tenant
    /// dispatch map. Empty when the operator did not configure
    /// any federation peers — the federation dispatcher then
    /// falls back to the legacy `target_online: false` shape
    /// for cross-tenant `federation.forward_invoke` calls.
    pub fn federated_peers(&self) -> &BTreeMap<String, String> {
        &self.federated_peers
    }
}

/// Internal deserialisation shape. Pub-within-crate only so unit
/// tests can build instances without round-tripping through TOML.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RawDaemonConfig {
    pub(crate) daemon: RawDaemonSection,
}

#[derive(Clone, Debug, Deserialize)]
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
    pub(crate) billing_dir: Option<String>,
    /// PR-N1 commit 3a/N: operator-curated `tenant → hub_uri`
    /// map for cross-tenant `federation.forward_invoke` routing.
    /// `#[serde(default)]` so legacy daemon-config.toml files
    /// load unchanged (empty map).
    #[serde(default)]
    pub(crate) federated_peers: Option<BTreeMap<String, String>>,
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
         dial a hub for outbound `<self>.session` (spec §1.3)"
    )]
    DeviceMissingHubEndpoint,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facade::cli::test_support::HomeGuard;

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
                billing_dir: None,
                federated_peers: None,
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
    fn billing_dir_defaults_and_can_be_configured() {
        let mut raw = raw(
            DaemonMode::Device,
            "easynet.run",
            Some("https://hub.example.com:50051"),
            None,
            None,
            None,
        );
        let defaulted = DaemonConfig::from_raw(raw.clone()).expect("default config");
        assert!(defaulted.billing_dir().ends_with(".easynet/billing"));

        raw.daemon.billing_dir = Some("/tmp/easynet-workspace/billing".to_string());
        let configured = DaemonConfig::from_raw(raw).expect("configured billing dir");
        assert_eq!(
            configured.billing_dir(),
            Path::new("/tmp/easynet-workspace/billing")
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
            cfg.tls_cert_pem().map(Path::to_str).flatten(),
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
        let mut creds = crate::persistence::config::Credentials {
            node_id: "node-1".into(),
            credential_token: "token".into(),
            hub_endpoint: "https://hub.example:50443".into(),
            tenant_id: "tenant-a".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: None,
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
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
    fn ensure_minimal_device_config_does_not_rewrite_hub_mode_config() {
        let _g = HomeGuard::new();
        let creds = crate::persistence::config::Credentials {
            node_id: "node-1".into(),
            credential_token: "token".into(),
            hub_endpoint: "https://127.0.0.1:50443".into(),
            tenant_id: "tenant-a".into(),
            deploy_signature: String::new(),
            hub_api_base: None,
            username: None,
            hub_pubkey_b64: None,
            hub_tls_ca_pem_b64: None,
        };
        let path = default_config_path();
        std::fs::create_dir_all(path.parent().expect("config parent")).expect("mkdir");
        let raw = r#"[daemon]
mode = "both"
realm = "hub-realm"
listen_tcp = "0.0.0.0:50443"
tls_cert_pem = "/tmp/cert.pem"
tls_key_pem = "/tmp/key.pem"
"#;
        std::fs::write(&path, raw).expect("write hub config");

        ensure_minimal_device_config(&creds).expect("hub mode left alone");
        let unchanged = std::fs::read_to_string(&path).expect("read hub config");
        assert_eq!(unchanged, raw);
    }
}
