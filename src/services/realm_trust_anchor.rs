// EasyNet CLI — Services Layer — Realm Trust Anchor
// ===================================================
//
// File: src/services/realm_trust_anchor.rs
// Description: TOML-backed loader for the per-host realm trust set
//              (`/etc/easynet/realm-trust.toml`). The daemon's
//              admission gate consults a `RealmTrustAnchor` to
//              answer "is this caller URI permitted to join this
//              realm".
//
// Where this fits in RFC-003
// --------------------------
// PR-7 (莫浩) authors `realm-trust.toml` via the device-pairing
// flow and the backend identity bootstrap. PR-1 (this commit, 7a/9)
// only reads the file: at daemon boot we either find it and parse
// every `[[trusted_agent]]` block or we fall back to an empty trust
// set.
//
// The empty-fallback is intentional and is what keeps PR-1 mergeable
// to `main` without PR-7 also landed: a daemon with an empty trust
// set rejects every external caller (admission strict-mode default),
// which is fine for tests and staging but never for production. PR-10
// production canary checklist gates on the file being non-empty
// before the binary swap (`pr-drafts/PR-0-spec-daemon-invocation-server.md
// §5.2 sequencing constraint`).
//
// What this module is
// -------------------
// - The TOML deserialisation surface (`[[trusted_agent]]` blocks)
// - The runtime representation (`RealmTrustAnchor` + `TrustedAgent`)
// - A loader that returns `Self` (empty if missing) plus a
//   `try_load_strict` variant tests use to assert presence
// - `lookup` to answer "do we have a public key for this URI"
//
// What this module is NOT
// -----------------------
// - The admission gate itself — that lives in `easynet-axon`'s
//   `invocation::admission` module and is consulted from
//   `services::axon_serve::daemon_invocation_service` (commit 7b/9)
// - The `realm-trust.toml` *writer* — pairing flow lives in PR-7
// - SIGHUP reload — PR-7 wires the reload signal handler; PR-1
//   reads once at boot
// - Strict-vs-permissive policy decisions — those run inside the
//   admission gate (also commit 7b/9), this module is a pure
//   read-side data surface
//
// File format
// -----------
// ```toml
// # /etc/easynet/realm-trust.toml
//
// [[trusted_agent]]
// agent_uri        = "easynet:///r/realm/agent/backend"
// public_key_b64   = "..."
// role             = "backend"      # or "device" / "hub"
// added_at_unix_ms = 1714492800000
//
// [[trusted_agent]]
// agent_uri        = "easynet:///r/realm/agent/laptop-1"
// public_key_b64   = "..."
// role             = "device"
// added_at_unix_ms = 1714492801234
// ```
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// Default location the daemon reads the realm trust anchor from.
/// Operators override via `[daemon] realm_trust_anchor_path` in
/// the config (PR-7 wires the override; PR-1 reads from the
/// default).
pub const DEFAULT_REALM_TRUST_PATH: &str = "/etc/easynet/realm-trust.toml";

/// Role a trusted agent plays in the realm. Used by audit log
/// formatters and by PR-7's pairing-flow validation; the admission
/// gate itself does not branch on role today.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustedAgentRole {
    /// EasyNet backend service running alongside the hub-mode daemon.
    Backend,
    /// Consumer device daemon dialing in over TLS.
    Device,
    /// Cross-realm hub federate. Out of RFC-003 scope; reserved for
    /// RFC-005.
    Hub,
}

/// One entry in `realm-trust.toml`. Public so the admission gate
/// facade (commit 7b/9) and PR-7's pairing flow can consume the
/// shape directly.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct TrustedAgent {
    /// Canonical agent URI per spec §5.1
    /// (`easynet:///r/{tenant_id}/agent/{node_id}`).
    pub agent_uri: String,
    /// Ed25519 verifying key, base64-encoded (32 raw bytes →
    /// 44 chars with padding). Validated by the admission gate
    /// when the entry is consulted, not at load time.
    pub public_key_b64: String,
    /// Role the agent plays in the realm.
    pub role: TrustedAgentRole,
    /// Timestamp the entry was added by the pairing flow (PR-7).
    /// Surface only — admission does not policy-check on age.
    pub added_at_unix_ms: u64,
}

/// Internal TOML shape; private so the public `RealmTrustAnchor`
/// owns its index data structure choice.
#[derive(Debug, Default, Deserialize)]
struct RawTrustAnchor {
    #[serde(default)]
    trusted_agent: Vec<TrustedAgent>,
}

/// Trust set the daemon consults at admission time. Built once at
/// boot from the on-disk TOML; PR-7 wires SIGHUP-triggered reload
/// against the same constructor.
///
/// **Invariant 1 (URI uniqueness)**: each `agent_uri` appears at
/// most once. A duplicate-URI file is a configuration error — we
/// reject at load time so a typo never silently shadows an earlier
/// entry. The pairing flow in PR-7 must enforce uniqueness on the
/// write side.
///
/// **Invariant 2 (lookup is borrow)**: `lookup` returns a borrowed
/// `&TrustedAgent` so call sites do not clone the whole entry per
/// admission check. The admission gate copies only the public key
/// when it needs to.
///
/// **Empty-fallback semantics**: a missing file maps to an empty
/// `RealmTrustAnchor`. The dispatcher logs a WARN at boot when the
/// trust set is empty (the operator runbook in `docs/daemon-config.md`
/// covers this). Admission strict-mode against an empty trust set
/// rejects every external caller, which is the safe default before
/// PR-7 + PR-10 land.
#[derive(Debug, Default)]
pub struct RealmTrustAnchor {
    by_uri: HashMap<String, TrustedAgent>,
}

impl RealmTrustAnchor {
    /// Load from `path` and return an empty anchor if the file is
    /// missing. Use this at daemon boot — staging environments
    /// commonly lack the file and a missing trust set is a logged
    /// warning, not a fatal boot error.
    pub fn load_or_empty(path: &Path) -> Result<Self, RealmTrustError> {
        match fs::read_to_string(path) {
            Ok(raw) => Self::parse(&raw, path),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(RealmTrustError::ReadFailed {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Load from `path` and return an error if the file is missing.
    /// Test seam — production daemons use `load_or_empty`.
    pub fn try_load_strict(path: &Path) -> Result<Self, RealmTrustError> {
        let raw = fs::read_to_string(path).map_err(|source| RealmTrustError::ReadFailed {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&raw, path)
    }

    /// Construct directly from already-deserialised entries. Public
    /// within the crate so PR-7's pairing flow can build an anchor
    /// from in-memory entries during a write-and-reload cycle and
    /// tests can build small fixtures.
    pub(crate) fn from_entries(entries: Vec<TrustedAgent>) -> Result<Self, RealmTrustError> {
        let mut by_uri = HashMap::with_capacity(entries.len());
        for entry in entries {
            if let Some(prior) = by_uri.insert(entry.agent_uri.clone(), entry.clone()) {
                return Err(RealmTrustError::DuplicateUri {
                    agent_uri: prior.agent_uri,
                });
            }
        }
        Ok(Self { by_uri })
    }

    fn parse(raw: &str, path: &Path) -> Result<Self, RealmTrustError> {
        let parsed: RawTrustAnchor =
            toml::from_str(raw).map_err(|source| RealmTrustError::ParseFailed {
                path: path.to_path_buf(),
                source,
            })?;
        Self::from_entries(parsed.trusted_agent)
    }

    /// Look up the trust entry for `agent_uri`. Returns `None` if
    /// the URI is not in the trust set (admission gate rejects in
    /// that case; this module is intentionally a pure data surface).
    #[must_use]
    pub fn lookup(&self, agent_uri: &str) -> Option<&TrustedAgent> {
        self.by_uri.get(agent_uri)
    }

    /// Number of trusted agents in the anchor. Used by the daemon
    /// boot log and by PR-10 canary checklist verification.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_uri.len()
    }

    /// Whether the anchor is empty. Empty is allowed by PR-1
    /// (logged as WARN) but rejected by PR-10 canary's pre-swap
    /// verification.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_uri.is_empty()
    }
}

/// Every way the loader can fail. `ReadFailed` covers I/O errors
/// other than NotFound (NotFound maps to `Self::default()` via
/// `load_or_empty`); `ParseFailed` covers TOML syntax errors;
/// `DuplicateUri` covers the URI-uniqueness invariant.
#[derive(Debug, Error)]
pub enum RealmTrustError {
    #[error("failed to read realm trust anchor at {path}: {source}")]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse realm trust anchor at {path}: {source}")]
    ParseFailed {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error(
        "realm trust anchor invariant 1 violated: agent_uri `{agent_uri}` appears more than \
         once. PR-7 pairing-flow writes must enforce uniqueness."
    )]
    DuplicateUri { agent_uri: String },
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

    fn entry(uri: &str) -> TrustedAgent {
        TrustedAgent {
            agent_uri: uri.to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_714_492_800_000,
        }
    }

    #[test]
    fn missing_file_yields_empty_anchor() {
        let nonexistent = PathBuf::from("/tmp/easynet-realm-trust-test-does-not-exist");
        let anchor = RealmTrustAnchor::load_or_empty(&nonexistent).expect("load_or_empty Ok");
        assert!(anchor.is_empty());
        assert_eq!(anchor.len(), 0);
    }

    #[test]
    fn try_load_strict_on_missing_file_returns_read_error() {
        let nonexistent = PathBuf::from("/tmp/easynet-realm-trust-test-does-not-exist");
        match RealmTrustAnchor::try_load_strict(&nonexistent) {
            Err(RealmTrustError::ReadFailed { .. }) => {}
            other => panic!("expected ReadFailed, got {other:?}"),
        }
    }

    #[test]
    fn empty_file_yields_empty_anchor() {
        let file = write_temp("");
        let anchor = RealmTrustAnchor::load_or_empty(file.path()).expect("Ok");
        assert!(anchor.is_empty());
    }

    #[test]
    fn single_entry_loads_and_lookups() {
        let toml_content = r#"
[[trusted_agent]]
agent_uri = "easynet:///r/realm/agent/backend"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "backend"
added_at_unix_ms = 1714492800000
        "#;

        let file = write_temp(toml_content);
        let anchor = RealmTrustAnchor::load_or_empty(file.path()).expect("Ok");
        assert_eq!(anchor.len(), 1);

        let entry = anchor
            .lookup("easynet:///r/realm/agent/backend")
            .expect("present");
        assert_eq!(entry.role, TrustedAgentRole::Backend);
        assert_eq!(entry.added_at_unix_ms, 1_714_492_800_000);
    }

    #[test]
    fn multiple_entries_with_distinct_uris_load() {
        let toml_content = r#"
[[trusted_agent]]
agent_uri = "easynet:///r/realm/agent/backend"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "backend"
added_at_unix_ms = 1714492800000

[[trusted_agent]]
agent_uri = "easynet:///r/realm/agent/laptop-1"
public_key_b64 = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB="
role = "device"
added_at_unix_ms = 1714492801234
        "#;

        let file = write_temp(toml_content);
        let anchor = RealmTrustAnchor::load_or_empty(file.path()).expect("Ok");
        assert_eq!(anchor.len(), 2);
        assert!(anchor.lookup("easynet:///r/realm/agent/backend").is_some());
        assert!(anchor.lookup("easynet:///r/realm/agent/laptop-1").is_some());
        assert!(anchor.lookup("easynet:///r/realm/agent/missing").is_none());
    }

    #[test]
    fn duplicate_uri_is_rejected() {
        let entries = vec![
            entry("easynet:///r/realm/agent/n1"),
            entry("easynet:///r/realm/agent/n1"),
        ];
        match RealmTrustAnchor::from_entries(entries) {
            Err(RealmTrustError::DuplicateUri { agent_uri }) => {
                assert_eq!(agent_uri, "easynet:///r/realm/agent/n1");
            }
            other => panic!("expected DuplicateUri, got {other:?}"),
        }
    }

    #[test]
    fn malformed_toml_is_rejected() {
        let file = write_temp("this is not valid TOML {{{");
        match RealmTrustAnchor::load_or_empty(file.path()) {
            Err(RealmTrustError::ParseFailed { .. }) => {}
            other => panic!("expected ParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn unknown_role_value_is_rejected_at_parse() {
        let toml_content = r#"
[[trusted_agent]]
agent_uri = "easynet:///r/realm/agent/n1"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "unknown"
added_at_unix_ms = 1714492800000
        "#;
        let file = write_temp(toml_content);
        match RealmTrustAnchor::load_or_empty(file.path()) {
            Err(RealmTrustError::ParseFailed { .. }) => {}
            other => panic!("expected ParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn from_entries_preserves_lookup() {
        let anchor = RealmTrustAnchor::from_entries(vec![
            entry("easynet:///r/realm/agent/a"),
            entry("easynet:///r/realm/agent/b"),
        ])
        .expect("Ok");
        assert_eq!(anchor.len(), 2);
        assert!(anchor.lookup("easynet:///r/realm/agent/a").is_some());
        assert!(anchor.lookup("easynet:///r/realm/agent/b").is_some());
    }
}
