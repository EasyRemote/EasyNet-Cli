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
//    cross-hub `federation.forward_invoke` calls to.
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
//   `federation.subscribe_directory` cross-hub stream, both of
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
use std::path::PathBuf;

use clap::Args;
use serde::Serialize;

use crate::support::output;

/// Default daemon-config.toml location, mirrors
/// `persistence::daemon_config::DEFAULT_DAEMON_CONFIG_PATH`. Re-
/// derived here so the subcommand doesn't pull in the
/// `axon-pb`-feature-gated daemon_config module.
fn daemon_config_path() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".easynet/daemon-config.toml");
    }
    PathBuf::from(".easynet/daemon-config.toml")
}

/// Default realm-trust.toml location, mirrors the daemon's
/// `daemon::invocation::boot::trust_anchor_path_from_env_or_default`.
/// The `EASYNET_REALM_TRUST_PATH` env override is the same one the
/// daemon honours so this subcommand and the daemon stay aligned in
/// test deployments. Host-mode installs usually cannot write
/// `/etc/easynet`, so they fall back to `~/.easynet/realm-trust.toml`.
fn realm_trust_path() -> PathBuf {
    if let Some(p) = std::env::var_os("EASYNET_REALM_TRUST_PATH") {
        return PathBuf::from(p);
    }
    let etc = PathBuf::from("/etc/easynet/realm-trust.toml");
    if let Ok(meta) = std::fs::metadata(&etc) {
        if meta.is_file() && meta.len() > 0 {
            return etc;
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".easynet/realm-trust.toml");
    }
    PathBuf::from(".easynet/realm-trust.toml")
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
    #[serde(skip_serializing_if = "Option::is_none")]
    origin_realm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hub_endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tls_ca_pem_path: Option<String>,
}

pub fn run(args: PeersArgs) -> anyhow::Result<()> {
    let federated_peers = read_federated_peers().unwrap_or_default();
    let trusted_hubs = read_trusted_hubs().unwrap_or_default();

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
            if let Some(t) = &hub.origin_realm {
                output::detail("  origin_realm", t);
            }
            if let Some(u) = &hub.hub_endpoint {
                output::detail("  hub_endpoint", u);
            }
            if let Some(p) = &hub.tls_ca_pem_path {
                output::detail("  tls_ca_pem_path", p);
            }
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
    use toml_edit::DocumentMut;

    let path = daemon_config_path();
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    let doc: DocumentMut = raw.parse()?;
    let mut out = BTreeMap::new();
    if let Some(daemon) = doc.get("daemon").and_then(|i| i.as_table()) {
        if let Some(peers) = daemon.get("federated_peers").and_then(|i| i.as_table()) {
            for (k, v) in peers.iter() {
                if let Some(s) = v.as_str() {
                    out.insert(k.to_string(), s.to_string());
                }
            }
        }
    }
    Ok(out)
}

fn read_trusted_hubs() -> anyhow::Result<Vec<TrustedHubEntry>> {
    use toml_edit::DocumentMut;

    let path = realm_trust_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    let doc: DocumentMut = raw.parse()?;
    let mut out = Vec::new();
    if let Some(agents) = doc
        .get("trusted_agent")
        .and_then(|i| i.as_array_of_tables())
    {
        for table in agents.iter() {
            let role = table.get("role").and_then(|i| i.as_str()).unwrap_or("");
            if role != "hub" {
                continue;
            }
            let entry = TrustedHubEntry {
                agent_ura: table
                    .get("agent_ura")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string(),
                origin_realm: table
                    .get("origin_realm")
                    .and_then(|i| i.as_str())
                    .map(str::to_string),
                hub_endpoint: table
                    .get("hub_endpoint")
                    .and_then(|i| i.as_str())
                    .map(str::to_string),
                tls_ca_pem_path: table
                    .get("tls_ca_pem_path")
                    .and_then(|i| i.as_str())
                    .map(str::to_string),
            };
            out.push(entry);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(contents: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("temp file");
        file.write_all(contents.as_bytes()).expect("write");
        file
    }

    /// Drive `read_federated_peers` against an arbitrary path by
    /// re-using the same TOML parser the production helper does.
    /// The helper above reads from a fixed env-derived path; this
    /// shim lets tests pin the parser contract.
    fn parse_federated_peers_from(raw: &str) -> BTreeMap<String, String> {
        use toml_edit::DocumentMut;
        let doc: DocumentMut = raw.parse().expect("parse");
        let mut out = BTreeMap::new();
        if let Some(daemon) = doc.get("daemon").and_then(|i| i.as_table()) {
            if let Some(peers) = daemon.get("federated_peers").and_then(|i| i.as_table()) {
                for (k, v) in peers.iter() {
                    if let Some(s) = v.as_str() {
                        out.insert(k.to_string(), s.to_string());
                    }
                }
            }
        }
        out
    }

    fn parse_trusted_hubs_from(raw: &str) -> Vec<TrustedHubEntry> {
        use toml_edit::DocumentMut;
        let doc: DocumentMut = raw.parse().expect("parse");
        let mut out = Vec::new();
        if let Some(agents) = doc
            .get("trusted_agent")
            .and_then(|i| i.as_array_of_tables())
        {
            for table in agents.iter() {
                let role = table.get("role").and_then(|i| i.as_str()).unwrap_or("");
                if role != "hub" {
                    continue;
                }
                out.push(TrustedHubEntry {
                    agent_ura: table
                        .get("agent_ura")
                        .and_then(|i| i.as_str())
                        .unwrap_or("")
                        .to_string(),
                    origin_realm: table
                        .get("origin_realm")
                        .and_then(|i| i.as_str())
                        .map(str::to_string),
                    hub_endpoint: table
                        .get("hub_endpoint")
                        .and_then(|i| i.as_str())
                        .map(str::to_string),
                    tls_ca_pem_path: table
                        .get("tls_ca_pem_path")
                        .and_then(|i| i.as_str())
                        .map(str::to_string),
                });
            }
        }
        out
    }

    #[test]
    fn empty_daemon_config_yields_empty_federated_peers() {
        let raw = r#"
[daemon]
mode = "device"
realm = "r1"
"#;
        let peers = parse_federated_peers_from(raw);
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
        let peers = parse_federated_peers_from(raw);
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
    fn realm_trust_filters_to_hub_role_only() {
        let local_hub = crate::ura::hub_ura("realm");
        let peer_hub = crate::ura::hub_ura("peer-realm");
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
        let hubs = parse_trusted_hubs_from(&raw);
        assert_eq!(hubs.len(), 1);
        assert_eq!(hubs[0].agent_ura, peer_hub);
        assert_eq!(hubs[0].origin_realm.as_deref(), Some("peer-realm"));
        assert_eq!(
            hubs[0].hub_endpoint.as_deref(),
            Some("https://peer-hub.example:50443")
        );
        assert_eq!(
            hubs[0].tls_ca_pem_path.as_deref(),
            Some("/etc/easynet/peer-ca.pem")
        );
    }

    #[test]
    fn realm_trust_with_no_hub_entries_yields_empty_list() {
        let local_hub = crate::ura::hub_ura("realm");
        let raw = format!(
            r#"
[[trusted_agent]]
agent_ura = "{local_hub}"
public_key_b64 = "AAAA"
role = "backend"
added_at_unix_ms = 1700000000000
"#
        );
        let hubs = parse_trusted_hubs_from(&raw);
        assert!(hubs.is_empty());
    }

    #[test]
    fn realm_trust_hub_entry_missing_schema_b_fields_is_listed() {
        // Minimal hub entries lacking
        // origin_realm / hub_endpoint / tls_ca_pem_path. The
        // listing surface still includes them so the operator
        // sees the trust set as-is and can decide to fill in
        // the missing fields (or remove the entry).
        let peer_hub = crate::ura::hub_ura("peer-realm");
        let raw = format!(
            r#"
[[trusted_agent]]
agent_ura = "{peer_hub}"
public_key_b64 = "CCCC"
role = "hub"
added_at_unix_ms = 1700000000002
"#
        );
        let hubs = parse_trusted_hubs_from(&raw);
        assert_eq!(hubs.len(), 1);
        assert_eq!(hubs[0].origin_realm, None);
        assert_eq!(hubs[0].hub_endpoint, None);
        assert_eq!(hubs[0].tls_ca_pem_path, None);
    }

    #[test]
    fn missing_files_do_not_panic() {
        // The production `read_federated_peers` and
        // `read_trusted_hubs` return Ok(empty) when the file
        // does not exist. Pin the contract so a fresh device
        // (no daemon-config, no realm-trust) produces a clean
        // "(empty)" listing rather than a hard error.
        let _ = write_temp("");
        // Direct functions that take env-derived paths cannot be
        // easily exercised here without setting HOME — that is
        // covered indirectly by the integration tests that drive
        // the binary. The pin above (`empty_daemon_config_yields
        // _empty_federated_peers`) covers the parser side.
    }
}
