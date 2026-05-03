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

    let with_peer = match upsert_federated_peer_in_toml(&raw, &creds.tenant_id, peer_hub) {
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

    // Align `[daemon].hub_endpoint` to the freshly-paired hub.
    // A previous `device join` against a different hub (or a Docker
    // e2e session that pointed the daemon at a localhost listener)
    // leaves a stale value here that survives subsequent joins
    // — the device then dials the OLD hub and surfaces as
    // "PermissionDenied: caller URI is not in the realm trust
    // anchor" or as "transport error" against an unreachable host.
    //
    // CRITICAL: this MUST be `creds.hub_endpoint` (the device-to-
    // hub dial target the backend handed us at pairing time), NOT
    // `peer_hub` (the hub-to-hub TLS guess on port 50443). The
    // two are different protocols on different ports:
    //
    //   * `[daemon].hub_endpoint`        ← creds.hub_endpoint
    //     = device dials hub for `<self>.session` (gRPC plaintext
    //       on :50051 in dev / TLS axon:// in prod)
    //
    //   * `[daemon.federated_peers].<t>` ← peer_hub
    //     = THIS hub dials peer hubs for `forward_invoke`
    //       (always TLS on :50443)
    //
    // Conflating them broke single-host host-mode v4.1.5: the
    // 50443 guess overrode the working :50051 dial target, the
    // daemon's `<self>.session` bidi never connected, and the
    // device never showed ONLINE. Idempotent on a no-op.
    let updated = match upsert_daemon_hub_endpoint_in_toml(&with_peer, &creds.hub_endpoint) {
        Ok(s) => s,
        Err(err) => {
            eprintln!(
                "[easynet join] could not align [daemon].hub_endpoint ({err}); the \
                 federated_peers entry was written but a stale hub_endpoint may \
                 remain. Edit `[daemon].hub_endpoint` in daemon-config.toml manually \
                 if `easynet runtime start` dials the wrong host."
            );
            with_peer
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

/// LB-52 Gap 3 — mirror the just-paired device's own `(uri, pubkey,
/// role=Device)` entry into the local realm-trust.toml so a
/// co-located hub-mode daemon admits this device on
/// `<self>.session` without a separate
/// `<self>.register_device_pubkey` round-trip.
///
/// Why this exists
/// ---------------
/// The canonical writer for trust-anchor entries is the backend's
/// pairing flow calling `<self>.register_device_pubkey` (PR-7
/// commit 5/N). In single-machine demo / answer-sheet topologies,
/// the backend is mocked or absent, so the trust anchor stays
/// empty and the local daemon rejects its own paired device's
/// `<self>.session` admission. This helper closes that gap by
/// pre-populating the device's self-entry on `easynet join`,
/// derived deterministically from `(tenant_id, node_id)` via the
/// same `derive_owner_public_key_b64` the runtime publish path
/// uses. Production deploys with a real backend continue to use
/// the canonical pairing-flow writer; this helper is a no-op when
/// the entry is already present (idempotent).
///
/// Failure handling
/// ----------------
/// Mirrors `auto_wire_federated_peer_from_credentials`: every step
/// is best-effort; failures log and return Ok so the join hot
/// path never aborts on this step. Empty `tenant_id` or
/// `node_id` is a silent no-op (test fixtures with synthetic
/// credentials hit this).
///
/// Path resolution
/// ---------------
/// Honours `EASYNET_REALM_TRUST_PATH` for tests / demos
/// (matching `boot::trust_anchor_path_from_env_or_default`); falls
/// back to `~/.easynet/realm-trust.toml` so the operator-readable
/// home location is the canonical user-facing surface. The
/// production daemon path `/etc/easynet/realm-trust.toml` is
/// admin-owned and not writable by the unprivileged join flow;
/// operators on production deploys rely on the backend's
/// `<self>.register_device_pubkey` writer instead.
pub fn auto_wire_self_realm_trust_from_credentials(creds: &Credentials) -> anyhow::Result<()> {
    if creds.tenant_id.trim().is_empty() || creds.node_id.trim().is_empty() {
        return Ok(());
    }
    let path = realm_trust_path_for_join();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        if let Err(err) = fs::create_dir_all(parent) {
            eprintln!(
                "[easynet join] could not create {} ({err}); skipping realm-trust auto-wire",
                parent.display()
            );
            return Ok(());
        }
    }

    let raw = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            eprintln!(
                "[easynet join] could not read {} ({err}); skipping realm-trust auto-wire",
                path.display()
            );
            return Ok(());
        }
    };

    // URI v4.1.4 Phase 2F: device URA, not agent URA. The trust
    // anchor stores the daemon's device identity under the
    // `/device/` role segment; emitting the legacy `/agent/`
    // shape would land in a parallel namespace the parser
    // strict-rejects.
    let agent_uri = crate::uri::device_uri(creds.tenant_id.trim(), creds.node_id.trim());
    let public_key_b64 = crate::runtime::publish::derive_owner_public_key_b64(
        creds.tenant_id.trim(),
        creds.node_id.trim(),
    );
    let added_at_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    // Two entries: the device's own URI (role=device) AND the
    // local realm's hub URI (role=hub). The hub entry is what
    // admits backend's federation.* calls into the daemon's
    // admission gate; without it, every backend → daemon dispatch
    // 401s with "caller URI ... is not in the realm trust anchor"
    // and the device pins as REMOVED forever.
    //
    // The hub pubkey: backend's `LoadOrInitHubIdentity(realm)`
    // mints a fresh random seed at its first boot and stores it
    // in `~/.easynet-hub/<realm>/identity.json`. We read that file
    // to get the actual pubkey backend signs with — falling back
    // to `None` (skip the hub entry) when backend hasn't started
    // yet. In production: backend boots first, then the operator
    // joins; the file exists. In a fresh test rig where join runs
    // before backend ever booted, we skip the hub entry and emit
    // an INFO log; the operator just re-runs `easynet device join`
    // (or restarts the daemon) after backend's first boot.
    let hub_uri = crate::uri::hub_uri(creds.tenant_id.trim());
    let hub_pubkey_b64_opt = read_backend_hub_pubkey_b64(creds.tenant_id.trim());

    let after_device =
        match upsert_self_trusted_agent(&raw, &agent_uri, &public_key_b64, added_at_unix_ms) {
            Ok(s) => s,
            Err(err) => {
                eprintln!(
                    "[easynet join] could not edit realm-trust.toml ({err}); skipping \
                 realm-trust auto-wire. Add the device entry manually under \
                 `[[trusted_agent]]` if you want local hub-mode admission for \
                 this device."
                );
                return Ok(());
            }
        };
    let updated = match hub_pubkey_b64_opt {
        Some(hub_pubkey_b64) => match upsert_trusted_agent_inner(
            &after_device,
            &hub_uri,
            &hub_pubkey_b64,
            "hub",
            added_at_unix_ms,
        ) {
            Ok(s) => s,
            Err(err) => {
                eprintln!(
                    "[easynet join] could not append hub entry to realm-trust.toml ({err}); \
                     backend's federation.* calls will be admission-rejected until you \
                     add `[[trusted_agent]] role = \"hub\"` for {hub_uri} by hand."
                );
                after_device
            }
        },
        None => {
            eprintln!(
                "[easynet join] backend hub identity not yet on disk for realm \
                 {tenant} (no `~/.easynet-hub/{tenant}/identity.json` found); \
                 skipping hub trust-anchor entry. Re-run `easynet device join` \
                 after backend's first boot, or restart the daemon — backend \
                 will populate the trust set via `<self>.register_device_pubkey` \
                 once it can dial the daemon.",
                tenant = creds.tenant_id.trim(),
            );
            after_device
        }
    };

    if updated == raw {
        // Idempotent: both entries already match.
        return Ok(());
    }

    if let Err(err) = atomic_write(&path, updated.as_bytes()) {
        eprintln!(
            "[easynet join] could not write realm-trust.toml ({err}); skipping \
             realm-trust auto-wire"
        );
        return Ok(());
    }

    // Best-effort SIGHUP so a co-located hub-mode daemon picks up
    // the new entry without a restart (PR-7 commit 5/N
    // SharedTrustAnchor cell, same SIGHUP-aware reload path the
    // canonical `<self>.register_device_pubkey` writer uses). The
    // SIGHUP also reloads `[daemon.federated_peers]`; one signal
    // covers both files.
    if let Err(err) = sighup_running_daemon_best_effort() {
        eprintln!(
            "[easynet join] realm-trust.toml updated; SIGHUP to reload it failed ({err}). \
             The new self-entry will activate on the next daemon restart."
        );
    }

    Ok(())
}

/// Resolve the realm-trust.toml path the join helper should write
/// to. Honours `EASYNET_REALM_TRUST_PATH` (test/demo override the
/// daemon also honours via `boot::trust_anchor_path_from_env_or_
/// default`) so a single env var rebases both the writer and the
/// reader. Falls back to `~/.easynet/realm-trust.toml` — the
/// home-rooted operator-visible default. The production
/// `/etc/easynet/realm-trust.toml` location is intentionally NOT
/// the join-time fallback: it requires root and operators on
/// production deploys go through the backend's
/// `<self>.register_device_pubkey` writer, not this helper.
/// Read backend's hub identity file at `~/.easynet-hub/<realm>/identity.json`
/// and project its private-key seed → ed25519 pubkey base64. Returns
/// `None` when the file is absent, malformed, or the seed is the
/// wrong length — every callsite treats `None` as "skip the hub
/// trust entry; backend hasn't booted yet" rather than failing the
/// join.
///
/// The file shape mirrors backend's `runtime/subject_context.go::
/// backendIdentityRecord`:
///   {
///     "private_key_seed_hex": "<64-hex>",
///     "agent_uri": "easynet:///r/<realm>/hub",
///     "created_at_unix_ms": <int>
///   }
fn read_backend_hub_pubkey_b64(realm: &str) -> Option<String> {
    use base64::Engine as _;
    use ed25519_dalek::SigningKey;

    let home = std::env::var_os("HOME")?;
    let path = std::path::Path::new(&home)
        .join(".easynet-hub")
        .join(realm)
        .join("identity.json");
    let raw = std::fs::read_to_string(&path).ok()?;

    #[derive(serde::Deserialize)]
    struct BackendIdentityRecord {
        private_key_seed_hex: String,
    }
    let parsed: BackendIdentityRecord = serde_json::from_str(&raw).ok()?;
    let seed_bytes = hex::decode(parsed.private_key_seed_hex.trim()).ok()?;
    if seed_bytes.len() != 32 {
        return None;
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&seed_bytes);
    let signing = SigningKey::from_bytes(&seed);
    Some(base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes()))
}

fn realm_trust_path_for_join() -> PathBuf {
    if let Some(override_path) = std::env::var_os("EASYNET_REALM_TRUST_PATH") {
        return PathBuf::from(override_path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".easynet/realm-trust.toml");
    }
    PathBuf::from(".easynet/realm-trust.toml")
}

/// TOML edit: insert-or-update a `[[trusted_agent]]` row whose
/// `agent_uri` matches the joining device. Preserves every other
/// row + comment via `toml_edit`. Idempotent when the row already
/// has the same `public_key_b64` (the deterministic derivation
/// from `(tenant_id, node_id)` should always produce the same
/// pubkey for the same identity, so a re-run of `easynet join`
/// against the same credentials is a no-op).
fn upsert_self_trusted_agent(
    raw: &str,
    agent_uri: &str,
    public_key_b64: &str,
    added_at_unix_ms: u64,
) -> anyhow::Result<String> {
    upsert_trusted_agent_inner(raw, agent_uri, public_key_b64, "device", added_at_unix_ms)
}

/// Generic [[trusted_agent]] upsert. `role` ∈ {"device", "hub", "agent"}.
/// Idempotent on (agent_uri); skips entirely when a row with the
/// same URI is already present (preserves operator-edited fields).
fn upsert_trusted_agent_inner(
    raw: &str,
    agent_uri: &str,
    public_key_b64: &str,
    role: &str,
    added_at_unix_ms: u64,
) -> anyhow::Result<String> {
    use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

    let mut doc: DocumentMut = if raw.trim().is_empty() {
        DocumentMut::new()
    } else {
        raw.parse().context("parse realm-trust.toml")?
    };

    // The on-disk shape is `[[trusted_agent]]` (array of tables).
    // toml_edit represents that as `Item::ArrayOfTables`.
    let agents_item = doc
        .as_table_mut()
        .entry("trusted_agent")
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    let agents = agents_item
        .as_array_of_tables_mut()
        .ok_or_else(|| anyhow::anyhow!("`trusted_agent` is not a TOML array of tables"))?;

    // Idempotent path: an existing entry with our agent_uri
    // means the canonical writer (or a previous join run) already
    // populated this row. Leave it untouched so we preserve any
    // operator-edited fields (e.g. role override for an admin-
    // promoted device, or a pubkey explicitly rotated through the
    // canonical `<self>.register_device_pubkey` writer). The
    // pubkey-mismatch case is treated identically: the existing
    // entry is authoritative; a re-pair that legitimately rotates
    // the key should go through `easynet reset` first.
    let already_present = agents.iter().any(|existing| {
        existing
            .get("agent_uri")
            .and_then(|i| i.as_str())
            .map(|s| s == agent_uri)
            .unwrap_or(false)
    });
    if already_present {
        return Ok(doc.to_string());
    }

    // No existing entry: append a fresh row.
    let mut row = Table::new();
    row.insert("agent_uri", value(agent_uri));
    row.insert("public_key_b64", value(public_key_b64));
    row.insert("role", value(role));
    row.insert("added_at_unix_ms", value(added_at_unix_ms as i64));
    // `origin_tenant_id`, `hub_uri`, `tls_ca_pem_path` are
    // Hub-role-only fields specific to peer-hub trust entries
    // (cross-realm TLS dial); they're omitted here since the
    // local hub trust entry is dialed via the local UDS and
    // peer-hub entries are written by docker-e2e-cross-hub.sh
    // or operator hand.
    agents.push(row);

    Ok(doc.to_string())
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
    let peers_item = daemon_table.entry("federated_peers").or_insert_with(|| {
        Item::Table({
            let mut t = Table::new();
            t.set_implicit(false);
            t
        })
    });
    let peers_table = peers_item
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[daemon.federated_peers] is not a TOML table"))?;
    peers_table.set_implicit(false);
    peers_table.insert(tenant_id, value(hub_uri));

    Ok(doc.to_string())
}

/// Set `[daemon].hub_endpoint = <hub_uri>` in the daemon-config TOML
/// document, preserving every other field. Mirrors the `toml_edit`
/// discipline of `upsert_federated_peer_in_toml` so operator hand-
/// formatting / comments survive intact. Idempotent: if the existing
/// value already matches, the returned string is byte-identical to
/// the input (the caller can skip the atomic write).
fn upsert_daemon_hub_endpoint_in_toml(raw: &str, hub_uri: &str) -> anyhow::Result<String> {
    use toml_edit::{value, DocumentMut, Item, Table};

    let mut doc: DocumentMut = raw.parse().context("parse daemon-config.toml")?;

    let daemon_item = doc
        .as_table_mut()
        .entry("daemon")
        .or_insert_with(|| Item::Table(Table::new()));
    let daemon_table = daemon_item
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("[daemon] is not a TOML table"))?;
    daemon_table.insert("hub_endpoint", value(hub_uri));

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
        format!("atomic rename {} → {}", tmp_path.display(), path.display())
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
    let raw =
        fs::read_to_string(&pid_path).with_context(|| format!("read {}", pid_path.display()))?;
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
    fn auto_wire_writes_creds_hub_endpoint_not_peer_hub_guess() {
        // Regression pin for the v4.1.5 host-mode ONLINE bug:
        // join used to overwrite [daemon].hub_endpoint with
        // peer_hub (the :50443 TLS guess), which broke the
        // device-mode dial target. The fix is to write
        // creds.hub_endpoint into [daemon].hub_endpoint and
        // reserve peer_hub for [daemon.federated_peers] only.
        //
        // Two-protocol invariant:
        //   * [daemon].hub_endpoint        ← creds.hub_endpoint  (gRPC plaintext / axon://, :50051 in dev)
        //   * [daemon.federated_peers].T   ← peer_hub            (TLS, :50443)
        //
        // This test creates a daemon-config under HOME, runs the
        // auto-wire, and asserts that [daemon].hub_endpoint is the
        // creds value, NOT the 50443 guess.
        use std::io::Write;
        let tmp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("HOME", tmp.path());

        let cfg_path = tmp.path().join(".easynet").join("daemon-config.toml");
        std::fs::create_dir_all(cfg_path.parent().unwrap()).unwrap();
        let mut f = std::fs::File::create(&cfg_path).unwrap();
        writeln!(
            f,
            "[daemon]\nmode = \"device\"\nrealm = \"localhost\"\nhub_endpoint = \"http://stale-host:50051\"\nuds_path = \"/tmp/x.sock\"\n"
        )
        .unwrap();
        drop(f);

        let creds = Credentials {
            node_id: "n1".into(),
            credential_token: "tok".into(),
            hub_endpoint: "http://127.0.0.1:50051".into(),
            tenant_id: "localhost".into(),
            deploy_signature: "sig".into(),
            hub_api_base: None,
            username: None,
        };
        auto_wire_federated_peer_from_credentials(&creds, None).expect("auto-wire");

        let updated = std::fs::read_to_string(&cfg_path).unwrap();
        assert!(
            updated.contains("hub_endpoint = \"http://127.0.0.1:50051\""),
            "[daemon].hub_endpoint must equal creds.hub_endpoint; got:\n{updated}"
        );
        assert!(
            !updated.contains("hub_endpoint = \"https://127.0.0.1:50443\""),
            "[daemon].hub_endpoint regressed to the 50443 TLS guess; got:\n{updated}"
        );
        // Federated_peers entry SHOULD be the 50443 guess (cross-
        // hub dial target is TLS).
        assert!(
            updated.contains(":50443") && updated.contains("[daemon.federated_peers]"),
            "[daemon.federated_peers] missing or wrong; got:\n{updated}"
        );
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
            username: None,
        };
        auto_wire_federated_peer_from_credentials(&creds, None).expect("empty tenant is no-op");
    }

    // ── LB-52 Gap 3 — realm-trust auto-wire on join ────────────────

    #[test]
    fn upsert_self_trusted_agent_appends_to_empty_doc() {
        let raw = "";
        let updated = upsert_self_trusted_agent(
            raw,
            "easynet:///r/tenant-a/device/dev-1",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            1_700_000_000_000,
        )
        .expect("upsert into empty doc");
        // Round-trip parse to verify we wrote a valid TOML AOT entry.
        let parsed: toml::Value = updated.parse().expect("output parses as TOML");
        let arr = parsed
            .get("trusted_agent")
            .and_then(|v| v.as_array())
            .expect("trusted_agent array present");
        assert_eq!(arr.len(), 1, "exactly one entry written");
        let row = arr[0].as_table().expect("row is a table");
        assert_eq!(
            row.get("agent_uri").and_then(|v| v.as_str()),
            Some("easynet:///r/tenant-a/device/dev-1"),
        );
        assert_eq!(row.get("role").and_then(|v| v.as_str()), Some("device"));
        assert_eq!(
            row.get("public_key_b64").and_then(|v| v.as_str()),
            Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
        );
        assert_eq!(
            row.get("added_at_unix_ms").and_then(|v| v.as_integer()),
            Some(1_700_000_000_000),
        );
    }

    #[test]
    fn upsert_self_trusted_agent_idempotent_when_uri_already_present() {
        // An existing row with our URI is left untouched even if
        // the pubkey differs — the canonical
        // `<self>.register_device_pubkey` writer (or an operator
        // edit) is authoritative.
        let raw = r#"
[[trusted_agent]]
agent_uri = "easynet:///r/tenant-a/device/dev-1"
public_key_b64 = "OPERATOR-WRITTEN-VALUE"
role = "device"
added_at_unix_ms = 100
"#;
        let updated = upsert_self_trusted_agent(
            raw,
            "easynet:///r/tenant-a/device/dev-1",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            999_999_999_999,
        )
        .expect("idempotent path");
        // Pubkey untouched, no second row appended.
        assert!(updated.contains("OPERATOR-WRITTEN-VALUE"));
        assert!(!updated.contains("AAAAAAAA"));
        let arr_count = updated.matches("[[trusted_agent]]").count();
        assert_eq!(arr_count, 1, "no duplicate row appended");
    }

    #[test]
    fn upsert_self_trusted_agent_preserves_existing_unrelated_rows() {
        let raw = r#"
[[trusted_agent]]
agent_uri = "easynet:///r/tenant-a/agent/other-device"
public_key_b64 = "OTHER-KEY"
role = "device"
added_at_unix_ms = 1
"#;
        let updated = upsert_self_trusted_agent(
            raw,
            "easynet:///r/tenant-a/device/dev-1",
            "MY-KEY",
            1_700_000_000_000,
        )
        .expect("upsert");
        // Both rows present.
        assert!(updated.contains("other-device"));
        assert!(updated.contains("dev-1"));
        let parsed: toml::Value = updated.parse().expect("parses");
        let arr = parsed
            .get("trusted_agent")
            .and_then(|v| v.as_array())
            .expect("trusted_agent array");
        assert_eq!(arr.len(), 2, "preserves existing row + appends new");
    }

    #[test]
    fn auto_wire_self_realm_trust_short_circuits_on_empty_node_id() {
        let creds = Credentials {
            node_id: String::new(),
            credential_token: "tok".into(),
            hub_endpoint: "axon://hub:50051".into(),
            tenant_id: "tenant-a".into(),
            deploy_signature: "sig".into(),
            hub_api_base: None,
            username: None,
        };
        auto_wire_self_realm_trust_from_credentials(&creds)
            .expect("empty node_id is a no-op (no panic, no write)");
    }

    #[test]
    fn auto_wire_self_realm_trust_writes_entry_under_env_override() {
        // Drive the helper end-to-end by pointing
        // EASYNET_REALM_TRUST_PATH at a tempdir-rooted path AND
        // staging a backend identity.json under the same tempdir's
        // ~/.easynet-hub/<realm>/ subtree so the helper finds the
        // hub pubkey. Asserts the file contains BOTH:
        //   - device entry (deterministic pubkey derivation)
        //   - hub entry (pubkey derived from staged seed)
        let tmp = tempfile::tempdir().expect("tempdir");
        let trust_path = tmp.path().join("realm-trust.toml");

        // Stage a backend identity at <HOME>/.easynet-hub/tenant-a/identity.json
        // with a known seed so we can reverse-derive the pubkey for
        // assertion. Using a deterministic 32-byte seed keeps the
        // assertion stable.
        let staged_seed: [u8; 32] = [42; 32];
        let staged_seed_hex = staged_seed
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        let identity_dir = tmp.path().join(".easynet-hub").join("tenant-a");
        std::fs::create_dir_all(&identity_dir).expect("create identity dir");
        std::fs::write(
            identity_dir.join("identity.json"),
            format!(
                r#"{{"private_key_seed_hex":"{staged_seed_hex}","agent_uri":"easynet:///r/tenant-a/hub","created_at_unix_ms":1}}"#,
            ),
        )
        .expect("write identity.json");

        // Override HOME so read_backend_hub_pubkey_b64 finds the
        // staged identity.json — same tempdir as the trust file.
        let prev = std::env::var_os("EASYNET_REALM_TRUST_PATH");
        let prev_home = std::env::var_os("HOME");
        std::env::set_var("EASYNET_REALM_TRUST_PATH", &trust_path);
        std::env::set_var("HOME", tmp.path());
        struct EnvGuard(Option<std::ffi::OsString>, Option<std::ffi::OsString>);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(v) => std::env::set_var("EASYNET_REALM_TRUST_PATH", v),
                    None => std::env::remove_var("EASYNET_REALM_TRUST_PATH"),
                }
                match self.1.take() {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
        let _guard = EnvGuard(prev, prev_home);

        let creds = Credentials {
            node_id: "dev-1".into(),
            credential_token: "tok".into(),
            hub_endpoint: "axon://hub:50051".into(),
            tenant_id: "tenant-a".into(),
            deploy_signature: "sig".into(),
            hub_api_base: None,
            username: None,
        };
        auto_wire_self_realm_trust_from_credentials(&creds).expect("auto-wire ok");

        let body = std::fs::read_to_string(&trust_path).expect("file exists");
        let parsed: toml::Value = body.parse().expect("parses");
        let arr = parsed
            .get("trusted_agent")
            .and_then(|v| v.as_array())
            .expect("trusted_agent array");
        assert_eq!(
            arr.len(),
            2,
            "auto-wire writes both the device entry AND the local hub entry: device+hub = 2"
        );

        // Find each entry by role — order is "device first, then hub"
        // per the upsert call sequence, but we lookup by role to
        // stay decoupled from append order.
        let device_row = arr
            .iter()
            .find(|v| v.get("role").and_then(|r| r.as_str()) == Some("device"))
            .and_then(|v| v.as_table())
            .expect("device entry present");
        assert_eq!(
            device_row.get("agent_uri").and_then(|v| v.as_str()),
            Some("easynet:///r/tenant-a/device/dev-1"),
        );
        let expected_dev_pk =
            crate::runtime::publish::derive_owner_public_key_b64("tenant-a", "dev-1");
        assert_eq!(
            device_row.get("public_key_b64").and_then(|v| v.as_str()),
            Some(expected_dev_pk.as_str()),
            "device pubkey must be the deterministic derivation (matches what \
             <self>.register_device_pubkey would write)"
        );

        let hub_row = arr
            .iter()
            .find(|v| v.get("role").and_then(|r| r.as_str()) == Some("hub"))
            .and_then(|v| v.as_table())
            .expect("hub entry present");
        assert_eq!(
            hub_row.get("agent_uri").and_then(|v| v.as_str()),
            Some("easynet:///r/tenant-a/hub"),
            "hub URI is the realm-singleton shape; admits backend's federation.* dispatches",
        );

        // The hub pubkey we wrote came from the staged seed
        // (read_backend_hub_pubkey_b64 hex-decoded private_key_seed_hex
        // and projected through ed25519_dalek). Reverse-derive here.
        use base64::Engine as _;
        use ed25519_dalek::SigningKey;
        let signing = SigningKey::from_bytes(&staged_seed);
        let expected_hub_pk =
            base64::engine::general_purpose::STANDARD.encode(signing.verifying_key().to_bytes());
        assert_eq!(
            hub_row.get("public_key_b64").and_then(|v| v.as_str()),
            Some(expected_hub_pk.as_str()),
            "hub pubkey must be the ed25519 projection of the staged identity.json's seed",
        );

        // Re-running is idempotent: file size unchanged.
        let body_before = body;
        auto_wire_self_realm_trust_from_credentials(&creds).expect("second auto-wire is a no-op");
        let body_after = std::fs::read_to_string(&trust_path).expect("file exists");
        assert_eq!(body_after, body_before, "second run is byte-identical");
    }
}
