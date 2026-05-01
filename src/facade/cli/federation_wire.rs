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

/// Translate a Hub-supplied `axon://host:port` endpoint into the
/// `https://host:50443` shape the cross-hub dialer accepts.
///
/// The Hub returns endpoints like `axon://easynet.run:50051`
/// (the inbound-from-device gRPC endpoint). The cross-hub dialer
/// reads `[[trusted_agent]] hub_uri` as a TLS endpoint per
/// `tonic::transport::Endpoint::from_shared`, which expects an
/// `https://` scheme. By default the inbound-from-device port
/// (50051) is not the hub-to-hub port (50443); operators
/// running the standard deployment can edit daemon-config.toml
/// after the fact if their topology differs. The helper applies
/// the canonical port mapping but does NOT replace
/// already-`https://`-shaped endpoints.
fn normalise_hub_endpoint(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        return trimmed.to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("axon://") {
        // Strip any port and re-append the canonical hub-to-hub
        // port. A future commit may surface the cross-hub port
        // separately on the credentials shape so this
        // hard-coded substitution is removable.
        let host = rest.split(':').next().unwrap_or(rest);
        return format!("https://{host}:50443");
    }
    // Fallback: prepend https:// and let the operator edit if
    // the resulting URI does not match their topology.
    format!("https://{trimmed}")
}

/// Auto-wire the `(tenant_id, hub_endpoint)` mapping from a
/// successful `easynet join` into the local daemon's
/// `[daemon.federated_peers]` table. Best-effort: every error
/// logs a warning and returns Ok(()) so the user-facing join
/// flow never fails on this step.
pub fn auto_wire_federated_peer_from_credentials(creds: &Credentials) -> anyhow::Result<()> {
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

    let normalised_hub = normalise_hub_endpoint(&creds.hub_endpoint);

    let updated = match upsert_federated_peer_in_toml(&raw, &creds.tenant_id, &normalised_hub) {
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

    #[test]
    fn normalise_passes_https_endpoint_through() {
        assert_eq!(
            normalise_hub_endpoint("https://hub.example:50443"),
            "https://hub.example:50443"
        );
    }

    #[test]
    fn normalise_rewrites_axon_scheme_with_canonical_port() {
        assert_eq!(
            normalise_hub_endpoint("axon://easynet.run:50051"),
            "https://easynet.run:50443"
        );
    }

    #[test]
    fn normalise_handles_axon_without_port() {
        assert_eq!(
            normalise_hub_endpoint("axon://hub.local"),
            "https://hub.local:50443"
        );
    }

    #[test]
    fn normalise_falls_back_to_https_prefix_on_unknown_scheme() {
        assert_eq!(
            normalise_hub_endpoint("hub.example:50443"),
            "https://hub.example:50443"
        );
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
        auto_wire_federated_peer_from_credentials(&creds).expect("empty tenant is no-op");
    }
}
