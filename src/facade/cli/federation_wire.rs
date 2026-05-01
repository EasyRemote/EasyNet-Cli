// EasyNet CLI — federation_peers auto-wire on `easynet join`
// =============================================================
//
// File: src/facade/cli/federation_wire.rs
//
// On a successful `easynet join`, the device receives a
// `(tenant_id, hub_endpoint)` pair from the Hub. PR-N1 commits
// 6/N + 10/N already wire `[daemon.federated_peers]` to the
// running daemon (boot-time read + SIGHUP-aware reload). What
// was missing was the populate-on-join step: when this device
// pairs, the local hub-mode daemon (if any) needs that mapping
// so cross-hub `federation.forward_invoke` calls targeting the
// just-joined tenant resolve a hub URI.
//
// This module owns one helper: `auto_wire_federated_peer_from_
// credentials`. Called from `easynet join` after credentials
// are saved. Steps:
//
//   1. Read `~/.easynet/daemon-config.toml` via `toml_edit`
//      (preserves operator hand-formatting; we are touching one
//      table only).
//   2. Insert / update the `[daemon.federated_peers]` entry for
//      this tenant. Hub endpoint is normalised from the
//      `axon://host:port` shape the Hub returns to the
//      `https://host:port` shape the cross-hub dialer needs
//      (see `services/federation_client/cross_hub_dial.rs`).
//   3. Atomically rename-replace daemon-config.toml.
//   4. Best-effort SIGHUP the running daemon (Unix only) so the
//      `SharedFederatedPeers` cell picks up the new entry without
//      a daemon restart.
//
// Failure handling
// ----------------
// Every step is best-effort. If daemon-config.toml does not
// exist (this device has never run hub mode), or the file is
// not parseable, the helper logs a warning and returns — the
// `easynet join` flow is the user-facing hot path and must not
// fail because of a federated_peers wiring step. The user can
// always edit daemon-config.toml by hand later.
//
// Why CLI-side and not backend-side
// ---------------------------------
// The Hub's pairing endpoint hands the device a `tenant_id` +
// `hub_endpoint` JSON; the daemon-config.toml lives on the
// device's filesystem. Putting the file edit on the device
// (CLI) keeps the Hub stateless about which devices have
// hub-mode daemons and avoids a cross-repo coordination
// dependency.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::persistence::config::{self, Credentials};

/// Default daemon-config.toml location. Mirrors
/// `persistence::daemon_config::DEFAULT_DAEMON_CONFIG_PATH`. We
/// re-derive the path here rather than `pub use` the constant so
/// this module stays a leaf consumer that does not pull in the
/// `axon-pb`-feature-gated daemon_config module.
fn daemon_config_path() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".easynet/daemon-config.toml");
    }
    PathBuf::from(".easynet/daemon-config.toml")
}

/// Canonical hub-to-hub TLS port. Used as a fallback when the
/// operator did not pass `--peer-hub` and the Hub-supplied
/// endpoint carries the backend's inbound-from-device port (e.g.
/// `axon://...:50051`). Operators running on a non-default port
/// must pass `--peer-hub` explicitly; otherwise this guess is
/// emitted with a warning so they know to verify daemon-config.toml.
const CANONICAL_HUB_TO_HUB_PORT: u16 = 50443;

/// Outcome of resolving the federated_peers value to write. The
/// classification feeds the operator-facing warning so they know
/// whether the entry is `Confident` (operator-supplied or already
/// TLS-shaped) or `Guessed` (the helper picked the canonical
/// port for them).
#[derive(Debug)]
enum PeerHubResolution {
    /// Either the operator passed `--peer-hub`, or the Hub-
    /// supplied endpoint was already an `https://...` URL whose
    /// port the operator can be assumed to own. No warning needed.
    Confident(String),
    /// The endpoint shape required substitution to look like a
    /// daemon TLS listener. The operator gets a warning so they
    /// can verify the resulting `daemon-config.toml` matches
    /// their actual topology.
    Guessed { endpoint: String, source: String },
}

impl PeerHubResolution {
    fn endpoint(&self) -> &str {
        match self {
            Self::Confident(s) => s,
            Self::Guessed { endpoint, .. } => endpoint,
        }
    }
}

/// Resolve the value to write into `[daemon.federated_peers]`.
///
/// Precedence:
///   1. Operator-supplied `--peer-hub` wins outright. The operator
///      knows the peer daemon's TLS listen address; we trust it
///      and pass through.
///   2. `https://...` Hub endpoint passes through (rare in
///      practice — backends emit `axon://` or `http://` — but
///      preserves operator-set TLS endpoints when they appear).
///   3. `axon://host[:port]` → `https://host:50443` with a
///      warning. The Hub's `Axon.Endpoint` carries the inbound-
///      from-device gRPC port; the daemon's TLS port is by
///      convention 50443. Guessing keeps the auto-wire working
///      for the canonical deployment but warns so non-default
///      ports get caught.
///   4. `http://host[:port]` → `https://host:50443` with a
///      warning. Same shape as (3) but a different scheme; the
///      backend port (commonly 50051 in local-dev) must NOT be
///      written verbatim, since that would target the backend's
///      gRPC listener instead of the peer daemon's TLS listener.
///   5. Anything else → prepend `https://`, no port substitution
///      (we have no signal to know what port the operator wants).
fn resolve_peer_hub_endpoint(
    operator_override: Option<&str>,
    creds_hub_endpoint: &str,
) -> PeerHubResolution {
    if let Some(raw) = operator_override.map(str::trim).filter(|s| !s.is_empty()) {
        return PeerHubResolution::Confident(raw.to_string());
    }

    let trimmed = creds_hub_endpoint.trim();

    if trimmed.starts_with("https://") {
        return PeerHubResolution::Confident(trimmed.to_string());
    }

    if let Some(rest) = trimmed.strip_prefix("axon://") {
        let host = rest.split(':').next().unwrap_or(rest);
        return PeerHubResolution::Guessed {
            endpoint: format!("https://{host}:{CANONICAL_HUB_TO_HUB_PORT}"),
            source: trimmed.to_string(),
        };
    }

    if let Some(rest) = trimmed.strip_prefix("http://") {
        let host = rest.split(':').next().unwrap_or(rest);
        return PeerHubResolution::Guessed {
            endpoint: format!("https://{host}:{CANONICAL_HUB_TO_HUB_PORT}"),
            source: trimmed.to_string(),
        };
    }

    PeerHubResolution::Guessed {
        endpoint: format!("https://{trimmed}"),
        source: trimmed.to_string(),
    }
}

/// Auto-wire the `(tenant_id, peer_hub)` mapping from a
/// successful `easynet join` into the local daemon's
/// `[daemon.federated_peers]` table. Best-effort: every error
/// logs a warning and returns Ok(()) so the user-facing join
/// flow never fails on this step.
///
/// `operator_peer_hub` is the optional `--peer-hub` flag. When
/// set it overrides the credentials-derived endpoint; when
/// absent the helper falls back to a port-50443 guess off the
/// Hub-supplied endpoint and emits an operator warning so
/// non-default deployments are caught.
pub fn auto_wire_federated_peer_from_credentials(
    creds: &Credentials,
    operator_peer_hub: Option<&str>,
) -> anyhow::Result<()> {
    if creds.tenant_id.trim().is_empty() {
        return Ok(());
    }
    let path = daemon_config_path();
    if !path.exists() {
        // No daemon-config means no hub-mode daemon on this
        // device. Nothing to wire; return silently.
        return Ok(());
    }

    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(err) => {
            eprintln!(
                "[easynet join] could not read {} ({err}); skipping federated_peers auto-wire",
                path.display()
            );
            return Ok(());
        }
    };

    let resolution = resolve_peer_hub_endpoint(operator_peer_hub, &creds.hub_endpoint);
    if let PeerHubResolution::Guessed { endpoint, source } = &resolution {
        eprintln!(
            "[easynet join] peer hub endpoint not supplied; guessing `{endpoint}` from \
             `{source}` (canonical hub-to-hub port {CANONICAL_HUB_TO_HUB_PORT}). \
             If your peer daemon's TLS listener is on a different host or port, \
             re-run join with `--peer-hub <https://host:port>` or edit \
             `[daemon.federated_peers]` in daemon-config.toml directly."
        );
    }
    let peer_hub = resolution.endpoint();

    let updated = match upsert_federated_peer_in_toml(&raw, &creds.tenant_id, peer_hub) {
        Ok(s) => s,
        Err(err) => {
            eprintln!(
                "[easynet join] could not edit daemon-config.toml ({err}); skipping \
                 federated_peers auto-wire. Add the entry manually under \
                 `[daemon.federated_peers]` if you want cross-hub routing for this tenant."
            );
            return Ok(());
        }
    };

    if updated == raw {
        // Idempotent: existing entry already matches.
        return Ok(());
    }

    if let Err(err) = atomic_write(&path, updated.as_bytes()) {
        eprintln!(
            "[easynet join] could not write daemon-config.toml ({err}); skipping \
             federated_peers auto-wire"
        );
        return Ok(());
    }

    // Best-effort SIGHUP so the running daemon picks up the new
    // peer entry without a restart (PR-N1 commit 10/N
    // SharedFederatedPeers cell). Failure here is benign — the
    // operator can SIGHUP / restart manually.
    if let Err(err) = sighup_running_daemon_best_effort() {
        eprintln!(
            "[easynet join] daemon-config.toml updated; SIGHUP to reload it failed ({err}). \
             The new federated_peers entry will activate on the next daemon restart."
        );
    }

    Ok(())
}

/// TOML edit step: insert-or-update `[daemon.federated_peers]
/// <tenant_id> = <hub_uri>` while preserving every other field
/// in the file. Operator-authored daemon-config.toml files often
/// carry hand-formatted comments; using `toml_edit` (rather than
/// `toml::to_string` round-trip) keeps that formatting intact.
fn upsert_federated_peer_in_toml(
    raw: &str,
    tenant_id: &str,
    hub_uri: &str,
) -> anyhow::Result<String> {
    use toml_edit::{value, DocumentMut, Item, Table};

    let mut doc: DocumentMut = raw.parse().context("parse daemon-config.toml")?;

    let daemon_item = doc
        .as_table_mut()
        .entry("daemon")
        .or_insert_with(|| Item::Table(Table::new()));
    let daemon_table = daemon_item
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[daemon] is not a TOML table"))?;
    let peers_item = daemon_table
        .entry("federated_peers")
        .or_insert_with(|| Item::Table({
            let mut t = Table::new();
            t.set_implicit(false);
            t
        }));
    let peers_table = peers_item
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[daemon.federated_peers] is not a TOML table"))?;
    peers_table.set_implicit(false);
    peers_table.insert(tenant_id, value(hub_uri));

    Ok(doc.to_string())
}

/// Atomic write: write to a sibling tempfile, fsync, then
/// `rename(2)` on top of the existing file. POSIX guarantees
/// rename is atomic for same-filesystem replacements; mirrors
/// the discipline `realm_trust_anchor::save` uses for the trust
/// anchor file.
fn atomic_write(path: &Path, body: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_name = match path.file_name() {
        Some(name) => {
            let mut s = name.to_os_string();
            s.push(".tmp");
            s
        }
        None => anyhow::bail!("daemon-config path has no file name component"),
    };
    let tmp_path = parent.join(tmp_name);

    {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)
            .with_context(|| format!("open {} for write", tmp_path.display()))?;
        file.write_all(body)
            .with_context(|| format!("write {}", tmp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("fsync {}", tmp_path.display()))?;
    }
    fs::rename(&tmp_path, path).with_context(|| {
        let _ = fs::remove_file(&tmp_path);
        format!(
            "atomic rename {} → {}",
            tmp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}

/// Send SIGHUP to the running easynet-daemon, if any. Returns
/// `Ok(())` even if the pidfile is absent (no daemon running →
/// nothing to reload). Errors only when the pidfile names a PID
/// the OS rejects.
#[cfg(unix)]
fn sighup_running_daemon_best_effort() -> anyhow::Result<()> {
    let pid_path = config::easynet_daemon_pid_path();
    if !pid_path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(&pid_path)
        .with_context(|| format!("read {}", pid_path.display()))?;
    let pid: i32 = raw
        .trim()
        .parse()
        .with_context(|| format!("parse pid from {}", pid_path.display()))?;
    let result = unsafe { libc::kill(pid, libc::SIGHUP) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error()).with_context(|| format!("kill -HUP {pid}"))
    }
}

#[cfg(not(unix))]
fn sighup_running_daemon_best_effort() -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint_of(r: &PeerHubResolution) -> &str {
        r.endpoint()
    }

    #[test]
    fn operator_peer_hub_override_wins_over_creds() {
        let r = resolve_peer_hub_endpoint(Some("https://peer-b.example:50443"), "axon://hub:50051");
        assert!(matches!(r, PeerHubResolution::Confident(_)));
        assert_eq!(endpoint_of(&r), "https://peer-b.example:50443");
    }

    #[test]
    fn empty_operator_override_falls_through_to_creds() {
        let r = resolve_peer_hub_endpoint(Some("   "), "https://hub.example:50443");
        assert!(matches!(r, PeerHubResolution::Confident(_)));
        assert_eq!(endpoint_of(&r), "https://hub.example:50443");
    }

    #[test]
    fn https_creds_endpoint_passes_through_confident() {
        let r = resolve_peer_hub_endpoint(None, "https://hub.example:50443");
        assert!(matches!(r, PeerHubResolution::Confident(_)));
        assert_eq!(endpoint_of(&r), "https://hub.example:50443");
    }

    #[test]
    fn axon_scheme_creds_endpoint_guesses_canonical_port() {
        let r = resolve_peer_hub_endpoint(None, "axon://easynet.run:50051");
        match &r {
            PeerHubResolution::Guessed { endpoint, source } => {
                assert_eq!(endpoint, "https://easynet.run:50443");
                assert_eq!(source, "axon://easynet.run:50051");
            }
            other => panic!("expected Guessed, got {other:?}"),
        }
    }

    #[test]
    fn http_scheme_creds_endpoint_guesses_canonical_port_not_backend_port() {
        // Real production failure mode: backend Axon.Endpoint
        // arrives as `http://localhost:50051`. Writing that
        // verbatim into [daemon.federated_peers] would point the
        // cross-hub dialer at the backend's gRPC port, which is
        // not the peer daemon's TLS listener. The resolver must
        // substitute the canonical hub-to-hub port and flag the
        // outcome as Guessed so the operator gets a warning.
        let r = resolve_peer_hub_endpoint(None, "http://localhost:50051");
        match &r {
            PeerHubResolution::Guessed { endpoint, source } => {
                assert_eq!(endpoint, "https://localhost:50443");
                assert_eq!(source, "http://localhost:50051");
            }
            other => panic!("expected Guessed, got {other:?}"),
        }
    }

    #[test]
    fn axon_without_port_guesses_canonical_port() {
        let r = resolve_peer_hub_endpoint(None, "axon://hub.local");
        assert_eq!(endpoint_of(&r), "https://hub.local:50443");
        assert!(matches!(r, PeerHubResolution::Guessed { .. }));
    }

    #[test]
    fn unknown_scheme_falls_back_to_https_prefix_no_port_substitution() {
        let r = resolve_peer_hub_endpoint(None, "hub.example:50443");
        assert_eq!(endpoint_of(&r), "https://hub.example:50443");
        assert!(matches!(r, PeerHubResolution::Guessed { .. }));
    }

    #[test]
    fn upsert_inserts_new_tenant_into_empty_config() {
        let raw = r#"
[daemon]
mode = "hub"
realm = "r1"
"#;
        let updated =
            upsert_federated_peer_in_toml(raw, "user-a", "https://hub-a:50443").expect("upsert");
        assert!(updated.contains("[daemon.federated_peers]"));
        assert!(updated.contains("user-a = \"https://hub-a:50443\""));
        // Pre-existing fields preserved.
        assert!(updated.contains("mode = \"hub\""));
        assert!(updated.contains("realm = \"r1\""));
    }

    #[test]
    fn upsert_updates_existing_tenant_entry() {
        let raw = r#"
[daemon]
mode = "hub"
realm = "r1"

[daemon.federated_peers]
"user-a" = "https://OLD:50443"
"user-b" = "https://b:50443"
"#;
        let updated =
            upsert_federated_peer_in_toml(raw, "user-a", "https://NEW:50443").expect("upsert");
        // toml_edit may emit the key quoted or unquoted depending
        // on whether the original used quotes; the value update
        // is what we pin.
        assert!(
            updated.contains("\"https://NEW:50443\""),
            "updated value missing; got:\n{updated}"
        );
        assert!(
            !updated.contains("https://OLD:50443"),
            "old value still present; got:\n{updated}"
        );
        // user-b untouched.
        assert!(updated.contains("\"https://b:50443\""));
    }

    #[test]
    fn upsert_preserves_unrelated_fields_and_comments() {
        let raw = r#"
# operator note: this file is read at boot
[daemon]
mode = "hub"
# realm picks the URI prefix
realm = "production"
listen_tcp = "127.0.0.1:50443"
"#;
        let updated =
            upsert_federated_peer_in_toml(raw, "tenant-x", "https://x:50443").expect("upsert");
        assert!(updated.contains("# operator note"));
        assert!(updated.contains("# realm picks"));
        assert!(updated.contains("listen_tcp = \"127.0.0.1:50443\""));
        assert!(updated.contains("tenant-x = \"https://x:50443\""));
    }

    #[test]
    fn auto_wire_returns_ok_when_daemon_config_absent() {
        // Empty tenant_id is the no-op branch the helper short-
        // circuits on; this test pins that contract so a Hub
        // that briefly returns empty tenant_id (or the test
        // fixture with cleared HOME) does not fail join.
        let creds = Credentials {
            node_id: "n1".into(),
            credential_token: "tok".into(),
            hub_endpoint: "axon://hub:50051".into(),
            tenant_id: String::new(),
            deploy_signature: "sig".into(),
            hub_api_base: None,
        };
        auto_wire_federated_peer_from_credentials(&creds, None).expect("empty tenant is no-op");
    }
}
