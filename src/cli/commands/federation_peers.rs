// EasyNet CLI — `easynet federation peers` subcommand
// =====================================================
//
// File: src/cli/federation_peers.rs
//
// Reads the local daemon's federation-peer config from disk and
// prints the canonical URAs an operator can pass to
// `easynet ability invoke --node <peer-ura> ...`. Closes the
// "I know cross-hub forwarding works but I have no idea what URA
// to put after `--node`" gap operators reported during PR-N1
// user-flow review.
//
// Sources enumerated
// ------------------
// 1. `~/.easynet/daemon-config.toml` `[daemon.federated_peers]`
//    table — the operator-curated `realm → hub_endpoint` map. Every
//    entry here is a realm the local daemon will route
//    cross-hub `Invocation::Invoke` calls to.
// 2. `<EASYNET_REALM_TRUST_PATH or /etc/easynet/realm-trust.toml>`
//    `[[trusted_agent]]` blocks with `role = "hub"`. These are
//    the peer hubs the cross-hub dialer's TLS gate accepts. The
//    schema-B fields (`origin_realm`, `hub_endpoint`,
//    `tls_ca_pem_path`) print alongside so the operator sees
//    the complete trust picture in one command.
//
// What this command does NOT do
// -----------------------------
// - Cross-realm directory federation enumeration (i.e. "list
//   devices on peer hub B for the realm I belong to"). That
//   requires `<agent>.discover` Tier 3 cross-hub merged view +
//   `federation.subscribe_directory_v2` cross-hub stream, both of
//   which are PR-N3 territory. Until PR-N3 lands, this command
//   surfaces only the static config the local daemon has on
//   disk.
// - Daemon roundtrip. The output is the operator's local
//   filesystem state — what the daemon would see at boot or
//   after the next SIGHUP reload. A daemon currently running
//   may have a different in-memory cell state if the operator
//   edited the files but did not yet SIGHUP; that race is
//   acknowledged in the printed footer.
//
// Wire shape
// ----------
// `easynet federation peers` (default plain output) prints a
// human-readable table. `--json` emits a structured payload for
// scripts:
//
//     {
//       "federated_peers": {"<realm>": "<hub_endpoint>", ...},
//       "trusted_hubs": [
//         {
//           "agent_ura": "...",
//           "origin_realm": "...",
//           "hub_endpoint": "...",
//           "tls_ca_pem_path": "..."
//         },
//         ...
//       ]
//     }
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;
use serde::Serialize;

use crate::daemon::trust::anchor::{
    RealmTrustAnchor, RealmTrustAnchorLoadState, TrustAnchorRole, TrustedAgent,
};
use crate::support::platform::output;

/// Default realm-trust.toml location, mirrors the daemon's
/// `daemon::boot::invocation::trust_anchor_path_from_env_or_default`.
/// The `EASYNET_REALM_TRUST_PATH` env override is the same one the
/// daemon honours so this subcommand and the daemon stay aligned in
/// test deployments. Host-mode installs usually cannot write
/// `/etc/easynet`, so they fall back to `~/.easynet/realm-trust.toml`.
fn realm_trust_path() -> anyhow::Result<PathBuf> {
    realm_trust_path_with_system_anchor(Path::new("/etc/easynet/realm-trust.toml"))
}

fn realm_trust_path_with_system_anchor(system_anchor: &Path) -> anyhow::Result<PathBuf> {
    if let Some(p) = std::env::var_os("EASYNET_REALM_TRUST_PATH") {
        if p.to_string_lossy().trim().is_empty() {
            anyhow::bail!("EASYNET_REALM_TRUST_PATH must not be empty");
        }
        return Ok(PathBuf::from(p));
    }
    let etc = system_anchor.to_path_buf();
    if let Ok(meta) = std::fs::metadata(&etc) {
        if meta.is_file() && meta.len() > 0 {
            return Ok(etc);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        if home.to_string_lossy().trim().is_empty() {
            anyhow::bail!("HOME is required for realm-trust inspection path");
        }
        return Ok(PathBuf::from(home).join(".easynet/realm-trust.toml"));
    }
    anyhow::bail!("HOME is required for realm-trust inspection path")
}

#[derive(Debug, Args)]
pub struct PeersArgs {
    /// Emit JSON for scripts instead of a plain-text table.
    #[arg(long)]
    pub json: bool,
}

/// JSON shape `easynet federation peers --json` emits. Scripts
/// can pipe to `jq` and pluck whatever subset they need.
#[derive(Debug, Serialize)]
struct PeersOutput {
    federated_peers: BTreeMap<String, String>,
    trusted_hubs: Vec<TrustedHubEntry>,
}

#[derive(Debug, Serialize)]
struct TrustedHubEntry {
    agent_ura: String,
    origin_realm: String,
    hub_endpoint: String,
    tls_ca_pem_path: String,
}

pub fn run(args: PeersArgs) -> anyhow::Result<()> {
    let federated_peers = read_federated_peers()?;
    let trusted_hubs = read_trusted_hubs()?;

    if args.json {
        let out = PeersOutput {
            federated_peers,
            trusted_hubs,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    print_plain(&federated_peers, &trusted_hubs);
    Ok(())
}

fn print_plain(federated_peers: &BTreeMap<String, String>, trusted_hubs: &[TrustedHubEntry]) {
    output::info("federated_peers (operator-curated realm → hub_endpoint map)");
    if federated_peers.is_empty() {
        output::detail(
            "(empty)",
            "no [daemon.federated_peers] entries; cross-realm routing is disabled",
        );
    } else {
        for (realm, hub_endpoint) in federated_peers {
            output::detail(realm, hub_endpoint);
        }
    }
    eprintln!();

    output::info("trusted_hubs (realm-trust.toml [[trusted_agent]] role=\"hub\")");
    if trusted_hubs.is_empty() {
        output::detail(
            "(empty)",
            "no hub-role entries; cross-hub dialer cannot reach any peer",
        );
    } else {
        for hub in trusted_hubs {
            output::detail("agent_ura", &hub.agent_ura);
            output::detail("  origin_realm", &hub.origin_realm);
            output::detail("  hub_endpoint", &hub.hub_endpoint);
            output::detail("  tls_ca_pem_path", &hub.tls_ca_pem_path);
        }
    }
    eprintln!();

    output::info("To invoke an ability against a peer device, pass --node:");
    output::info(
        "  easynet ability invoke <ability-ura> --node easynet:///r/<realm>/device/<node>",
    );
    output::info(
        "where <realm> appears in 'federated_peers' above and <node> is the peer device's node_id.",
    );
    output::info(
        "Cross-realm device enumeration (auto-discovering <node>) requires PR-N3 directory federation.",
    );
}

fn read_federated_peers() -> anyhow::Result<BTreeMap<String, String>> {
    let path = crate::cli::commands::federation_paths::daemon_config_path("inspection")?;
    read_federated_peers_from_path(&path)
}

fn read_federated_peers_from_path(path: &Path) -> anyhow::Result<BTreeMap<String, String>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw = std::fs::read_to_string(path).with_context(|| {
        format!(
            "could not read daemon federation peer config {}",
            path.display()
        )
    })?;
    parse_federated_peers_from(&raw)
        .with_context(|| format!("invalid daemon federation peer config {}", path.display()))
}

fn parse_federated_peers_from(raw: &str) -> anyhow::Result<BTreeMap<String, String>> {
    use toml_edit::DocumentMut;

    let doc: DocumentMut = raw.parse()?;
    let mut out = BTreeMap::new();
    if let Some(daemon) = doc.get("daemon").and_then(|i| i.as_table()) {
        if let Some(peers) = daemon.get("federated_peers").and_then(|i| i.as_table()) {
            for (k, v) in peers.iter() {
                let realm = k.trim();
                if realm.is_empty() {
                    anyhow::bail!("[daemon.federated_peers] contains an empty realm key");
                }
                let Some(endpoint) = v.as_str().map(str::trim).filter(|value| !value.is_empty())
                else {
                    anyhow::bail!(
                        "[daemon.federated_peers].{k} must be a non-empty hub endpoint string"
                    );
                };
                out.insert(realm.to_string(), endpoint.to_string());
            }
        } else if daemon.get("federated_peers").is_some() {
            anyhow::bail!("[daemon.federated_peers] must be a TOML table");
        }
    } else if doc.get("daemon").is_some() {
        anyhow::bail!("[daemon] must be a TOML table");
    }
    Ok(out)
}

fn read_trusted_hubs() -> anyhow::Result<Vec<TrustedHubEntry>> {
    let path = realm_trust_path()?;
    read_trusted_hubs_from_path(&path)
}

fn read_trusted_hubs_from_path(path: &Path) -> anyhow::Result<Vec<TrustedHubEntry>> {
    let anchor = match RealmTrustAnchor::load_with_state(path)
        .with_context(|| format!("invalid realm trust config {}", path.display()))?
    {
        RealmTrustAnchorLoadState::Loaded(anchor) => anchor,
        RealmTrustAnchorLoadState::Missing { .. } => return Ok(Vec::new()),
    };
    trusted_hubs_from_anchor(anchor)
        .with_context(|| format!("invalid realm trust config {}", path.display()))
}

fn trusted_hubs_from_anchor(anchor: RealmTrustAnchor) -> anyhow::Result<Vec<TrustedHubEntry>> {
    anchor
        .entries_sorted()
        .into_iter()
        .filter(|entry| entry.role == TrustAnchorRole::Hub)
        .map(trusted_hub_entry)
        .collect()
}

fn trusted_hub_entry(entry: TrustedAgent) -> anyhow::Result<TrustedHubEntry> {
    let origin_realm = required_hub_field(&entry.agent_ura, "origin_realm", entry.origin_realm)?;
    let hub_endpoint = required_hub_field(&entry.agent_ura, "hub_endpoint", entry.hub_endpoint)?;
    let tls_ca_pem_path = entry
        .tls_ca_pem_path
        .map(|path| path.display().to_string())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "hub trusted_agent entry {} requires non-empty tls_ca_pem_path",
                entry.agent_ura
            )
        })?;

    Ok(TrustedHubEntry {
        agent_ura: entry.agent_ura,
        origin_realm,
        hub_endpoint,
        tls_ca_pem_path,
    })
}

fn required_hub_field(
    agent_ura: &str,
    field: &'static str,
    value: Option<String>,
) -> anyhow::Result<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("hub trusted_agent entry {agent_ura} requires non-empty {field}")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    struct RealmTrustEnvGuard {
        previous_path: Option<OsString>,
        previous_home: Option<OsString>,
    }

    impl RealmTrustEnvGuard {
        fn capture() -> Self {
            Self {
                previous_path: std::env::var_os("EASYNET_REALM_TRUST_PATH"),
                previous_home: std::env::var_os("HOME"),
            }
        }
    }

    impl Drop for RealmTrustEnvGuard {
        fn drop(&mut self) {
            match self.previous_path.take() {
                Some(value) => std::env::set_var("EASYNET_REALM_TRUST_PATH", value),
                None => std::env::remove_var("EASYNET_REALM_TRUST_PATH"),
            }
            match self.previous_home.take() {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    #[test]
    fn empty_daemon_config_yields_empty_federated_peers() {
        let raw = r#"
[daemon]
mode = "device"
realm = "r1"
"#;
        let peers = parse_federated_peers_from(raw).expect("valid daemon config");
        assert!(peers.is_empty());
    }

    #[test]
    fn populated_federated_peers_table_round_trips() {
        let raw = r#"
[daemon]
mode = "hub"
realm = "r1"

[daemon.federated_peers]
"user-a" = "https://hub-a:50443"
"user-b" = "https://hub-b:50443"
"#;
        let peers = parse_federated_peers_from(raw).expect("valid daemon config");
        assert_eq!(peers.len(), 2);
        assert_eq!(
            peers.get("user-a").map(String::as_str),
            Some("https://hub-a:50443")
        );
        assert_eq!(
            peers.get("user-b").map(String::as_str),
            Some("https://hub-b:50443")
        );
    }

    #[test]
    fn realm_trust_inspection_path_rejects_missing_home_before_cwd_fallback() {
        let _lock = crate::cli::commands::test_support::env_lock();
        let _guard = RealmTrustEnvGuard::capture();
        let dir = tempfile::tempdir().expect("temp dir");
        let missing_system_anchor = dir.path().join("missing-system-realm-trust.toml");
        std::env::remove_var("EASYNET_REALM_TRUST_PATH");
        std::env::remove_var("HOME");

        let error = realm_trust_path_with_system_anchor(&missing_system_anchor)
            .expect_err("missing HOME must not resolve under cwd");

        assert!(
            error
                .to_string()
                .contains("HOME is required for realm-trust inspection path"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn realm_trust_inspection_path_rejects_blank_home_before_relative_state_path() {
        let _lock = crate::cli::commands::test_support::env_lock();
        let _guard = RealmTrustEnvGuard::capture();
        let dir = tempfile::tempdir().expect("temp dir");
        let missing_system_anchor = dir.path().join("missing-system-realm-trust.toml");
        std::env::remove_var("EASYNET_REALM_TRUST_PATH");
        std::env::set_var("HOME", " ");

        let error = realm_trust_path_with_system_anchor(&missing_system_anchor)
            .expect_err("blank HOME must not resolve under cwd");

        assert!(
            error
                .to_string()
                .contains("HOME is required for realm-trust inspection path"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn realm_trust_filters_to_hub_role_only() {
        let local_hub = crate::core::ura::hub_ura("realm");
        let peer_hub = crate::core::ura::hub_ura("peer-realm");
        let raw = format!(
            r#"
[[trusted_agent]]
agent_ura = "{local_hub}"
public_key_b64 = "AAAA"
role = "backend"
added_at_unix_ms = 1700000000000

[[trusted_agent]]
agent_ura = "easynet:///r/realm/device/laptop"
public_key_b64 = "BBBB"
role = "device"
added_at_unix_ms = 1700000000001

[[trusted_agent]]
agent_ura = "{peer_hub}"
public_key_b64 = "CCCC"
role = "hub"
added_at_unix_ms = 1700000000002
origin_realm = "peer-realm"
hub_endpoint = "https://peer-hub.example:50443"
tls_ca_pem_path = "/etc/easynet/peer-ca.pem"
"#
        );
        let hubs =
            read_trusted_hubs_from_path(&write_temp_trust(&raw)).expect("valid realm trust config");
        assert_eq!(hubs.len(), 1);
        assert_eq!(hubs[0].agent_ura, peer_hub);
        assert_eq!(hubs[0].origin_realm, "peer-realm");
        assert_eq!(hubs[0].hub_endpoint, "https://peer-hub.example:50443");
        assert_eq!(hubs[0].tls_ca_pem_path, "/etc/easynet/peer-ca.pem");
    }

    #[test]
    fn realm_trust_with_no_hub_entries_yields_empty_list() {
        let local_hub = crate::core::ura::hub_ura("realm");
        let raw = format!(
            r#"
[[trusted_agent]]
agent_ura = "{local_hub}"
public_key_b64 = "AAAA"
role = "backend"
added_at_unix_ms = 1700000000000
"#
        );
        let hubs =
            read_trusted_hubs_from_path(&write_temp_trust(&raw)).expect("valid realm trust config");
        assert!(hubs.is_empty());
    }

    #[test]
    fn realm_trust_hub_entry_missing_schema_b_fields_fails_closed() {
        let peer_hub = crate::core::ura::hub_ura("peer-realm");
        let raw = format!(
            r#"
[[trusted_agent]]
agent_ura = "{peer_hub}"
public_key_b64 = "CCCC"
role = "hub"
added_at_unix_ms = 1700000000002
"#
        );
        let err = read_trusted_hubs_from_path(&write_temp_trust(&raw))
            .expect_err("schema-incomplete hub trust row must fail");
        assert!(format!("{err:#}").contains("requires non-empty origin_realm"));
    }

    #[test]
    fn missing_files_are_fresh_empty_state() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing_daemon_config = dir.path().join("missing-daemon-config.toml");
        let missing_realm_trust = dir.path().join("missing-realm-trust.toml");

        assert!(read_federated_peers_from_path(&missing_daemon_config)
            .expect("missing daemon config should be empty")
            .is_empty());
        assert!(read_trusted_hubs_from_path(&missing_realm_trust)
            .expect("missing realm trust should be empty")
            .is_empty());
    }

    #[test]
    fn malformed_daemon_config_fails_closed() {
        let err = parse_federated_peers_from(
            r#"
[daemon]
mode = "hub"

[daemon.federated_peers]
"peer" = 42
"#,
        )
        .expect_err("malformed peer endpoint must fail");

        assert!(err
            .to_string()
            .contains("[daemon.federated_peers].peer must be a non-empty hub endpoint string"));
    }

    #[test]
    fn malformed_realm_trust_hub_entry_fails_closed() {
        let err = read_trusted_hubs_from_path(&write_temp_trust(
            r#"
[[trusted_agent]]
role = "hub"
public_key_b64 = "CCCC"
added_at_unix_ms = 1700000000002
"#,
        ))
        .expect_err("hub entry without agent_ura must fail");

        assert!(format!("{err:#}").contains("missing field `agent_ura`"));
    }

    #[test]
    fn malformed_realm_trust_missing_role_fails_closed() {
        let err = read_trusted_hubs_from_path(&write_temp_trust(
            r#"
[[trusted_agent]]
agent_ura = "easynet:///r/peer-realm/authority"
public_key_b64 = "CCCC"
added_at_unix_ms = 1700000000002
origin_realm = "peer-realm"
hub_endpoint = "https://peer-hub.example:50443"
tls_ca_pem_path = "/etc/easynet/peer-ca.pem"
"#,
        ))
        .expect_err("trusted_agent without role must fail");

        assert!(format!("{err:#}").contains("missing field `role`"));
    }

    #[test]
    fn malformed_realm_trust_hub_agent_ura_fails_closed() {
        let err = read_trusted_hubs_from_path(&write_temp_trust(
            r#"
[[trusted_agent]]
agent_ura = "not-a-ura"
public_key_b64 = "CCCC"
role = "hub"
added_at_unix_ms = 1700000000002
origin_realm = "peer-realm"
hub_endpoint = "https://peer-hub.example:50443"
tls_ca_pem_path = "/etc/easynet/peer-ca.pem"
"#,
        ))
        .expect_err("hub entry with non-canonical agent_ura must fail");

        let err = format!("{err:#}");
        assert!(err.contains("expected the peer hub URA"));
        assert!(err.contains("parse failed"));
    }

    fn write_temp_trust(raw: &str) -> PathBuf {
        let file = tempfile::NamedTempFile::new().expect("temp trust file");
        std::fs::write(file.path(), raw).expect("write trust file");
        file.into_temp_path()
            .keep()
            .expect("persist temp trust path")
    }
}
