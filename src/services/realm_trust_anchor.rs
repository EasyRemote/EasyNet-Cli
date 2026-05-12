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
// PR-7 authors `realm-trust.toml` via the device-pairing
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
// agent_uri        = "easynet:///r/realm/hub"
// public_key_b64   = "..."
// role             = "backend"      # or "device" / "hub"
// added_at_unix_ms = 1714492800000
//
// [[trusted_agent]]
// agent_uri        = "easynet:///r/realm/device/laptop-1"
// public_key_b64   = "..."
// role             = "device"
// added_at_unix_ms = 1714492801234
// ```
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default location the daemon reads the realm trust anchor from.
/// Operators override via `[daemon] realm_trust_anchor_path` in
/// the config (PR-7 wires the override; PR-1 reads from the
/// default).
pub const DEFAULT_REALM_TRUST_PATH: &str = "/etc/easynet/realm-trust.toml";

/// Role a trusted agent plays in the realm. Used by audit log
/// formatters and by PR-7's pairing-flow validation; the admission
/// gate itself does not branch on role today.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustedAgentRole {
    /// EasyNet backend service running alongside the hub-mode daemon.
    Backend,
    /// Consumer device daemon dialing in over TLS.
    Device,
    /// Cross-realm hub federate. RFC-N PR-N1 cross-hub dial gate
    /// requires `role == Hub` AND `origin_tenant_id.is_some()`.
    /// DEC-N1 schema-B `origin_tenant_id` field added in PR-N1
    /// commit 2/N below; PR-N2 fills in cross-realm admission key
    /// resolution against the same entry.
    Hub,
    /// End-user signing as a first-class Caller. DEC-EU
    /// (RFC-001 amendment "user-as-first-class-caller"): user
    /// holds an Ed25519 keypair and signs mutating envelopes
    /// directly instead of being a Subject under hub-as-Caller.
    ///
    /// DEC-EU step 1 (this commit): single-keypair-per-user.
    /// Multi-device (a user logged in on browser + phone, each
    /// with its own non-exportable keypair) is a known followup;
    /// it requires either a per-device user URI suffix or a
    /// multimap relaxation of Invariant 1. Both paths are RFC-001
    /// amendments — see DEC-EU §multi-device.
    User,
}

/// One entry in `realm-trust.toml`. Public so the admission gate
/// facade (commit 7b/9) and PR-7's pairing flow can consume the
/// shape directly.
///
/// PR-N1 commit 2/N adds three optional fields used only by the
/// cross-hub federation dialer (`role = Hub` entries):
/// `origin_tenant_id`, `hub_uri`, `tls_ca_pem_path`. Backend /
/// Device entries leave them `None`; missing fields in older TOML
/// files deserialize to `None` via `#[serde(default)]` so PR-1+
/// trust files load unchanged on a PR-N1 daemon (DEC-N1 schema-B
/// backwards-compat).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct TrustedAgent {
    /// Canonical caller URI per spec §5.1. The expected role→shape
    /// mapping is:
    /// - Backend => `easynet:///r/{realm}/hub`
    /// - Device  => `easynet:///r/{realm}/device/{device_id}`
    /// - Hub     => `easynet:///r/{realm}/hub`
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
    /// **PR-N1 schema-B**. Tenant id this peer hub serves, in the
    /// form embedded in the peer's canonical hub URI. Set
    /// only on `role = Hub` entries; the admission gate uses this
    /// to resolve `caller.uri.tenant() → peer hub URI` when an
    /// invoke targets a tenant outside the local realm. `None` on
    /// Backend/Device entries and on legacy Hub entries written
    /// before PR-N1; the dialer fail-closes when this is `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_tenant_id: Option<String>,
    /// **PR-N1 schema-B**. Concrete dial URL for the peer hub,
    /// e.g. `"https://hub-b.example.com:50443"`. `Endpoint::
    /// from_shared(hub_uri)` is the only place this string is
    /// parsed — keep it operator-pasteable, not a structured URI.
    /// `None` ⇒ peer is not dial-eligible (not a federation peer
    /// or a legacy entry); the dialer surfaces `PeerNotTrusted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_uri: Option<String>,
    /// **PR-N1 schema-B**. Filesystem path the cross-hub dialer
    /// reads to obtain the operator-pinned CA certificate that
    /// must sign the peer's TLS leaf. The path is read once per
    /// dial-cache miss; the contents are passed verbatim to
    /// `tonic::transport::Certificate::from_pem`. `None` ⇒ no
    /// pinning configured; the dialer refuses to dial without an
    /// explicit pin (no system-CA fallback by design — DEC-N1's
    /// "operator-controlled trust set" rule).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls_ca_pem_path: Option<PathBuf>,
}

/// Internal TOML shape; private so the public `RealmTrustAnchor`
/// owns its index data structure choice.
#[derive(Debug, Default, Deserialize, Serialize)]
struct RawTrustAnchor {
    #[serde(default)]
    trusted_agent: Vec<TrustedAgent>,
}

/// Trust set the daemon consults at admission time. Built once at
/// boot from the on-disk TOML; PR-7 wires SIGHUP-triggered reload
/// against the same constructor.
///
/// **Invariant 1 (singleton-URI uniqueness)**: each `agent_uri`
/// with role Backend / Device / Hub appears at most once. A
/// duplicate-URI file is a configuration error — we reject at
/// load time so a typo never silently shadows an earlier entry.
///
/// **Invariant 1' (user multi-pubkey)**: DEC-EU lifts the strict
/// URI-uniqueness rule for `role = "user"`. RFC-001 "identity ≠
/// key" requires a user to retain the same identity URI across
/// devices while each device holds its own non-exportable
/// keypair. We therefore admit multiple `[[trusted_agent]]`
/// blocks sharing one user URI, gated by a composite uniqueness
/// rule: **(agent_uri, public_key_b64) is unique**. The pairing
/// flow rejects re-registering the same pubkey under the same
/// user URI; different pubkeys under the same user URI are the
/// expected multi-device shape.
///
/// **Invariant 2 (lookup is borrow)**: `lookup` returns a borrowed
/// `&TrustedAgent` so call sites do not clone the whole entry per
/// admission check. The admission gate copies only the public key
/// when it needs to. User admission goes through
/// [`lookup_user_by_pubkey`](#method.lookup_user_by_pubkey)
/// because a bare URI lookup is ambiguous when a user has
/// registered N devices.
///
/// **Empty-fallback semantics**: a missing file maps to an empty
/// `RealmTrustAnchor`. The dispatcher logs a WARN at boot when the
/// trust set is empty (the operator runbook in `docs/daemon-config.md`
/// covers this). Admission strict-mode against an empty trust set
/// rejects every external caller, which is the safe default before
/// PR-7 + PR-10 land.
#[derive(Debug, Default)]
pub struct RealmTrustAnchor {
    /// Hub / Backend / Device entries — single value per URI.
    by_uri: HashMap<String, TrustedAgent>,
    /// User entries — DEC-EU multi-pubkey-per-URI. Each Vec is
    /// kept short (a typical user has 2-5 devices); a linear
    /// pubkey scan during admission is fine.
    users: HashMap<String, Vec<TrustedAgent>>,
}

fn parse_legacy_bare_agent_uri(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("easynet:///r/")?;
    let (realm, after_realm) = rest.split_once('/')?;
    let token = after_realm.strip_prefix("agent/")?;
    if realm.is_empty() || token.is_empty() || token.contains('/') || token.contains('.') {
        return None;
    }
    Some((realm, token))
}

fn canonical_uri_for_role(
    agent_uri: &str,
    role: TrustedAgentRole,
) -> Result<String, RealmTrustError> {
    if let Ok(parsed) = crate::uri::parse_ura(agent_uri) {
        return match (role, parsed.kind) {
            (TrustedAgentRole::Device, crate::uri::URAKind::Device) => Ok(agent_uri.to_string()),
            (TrustedAgentRole::Backend | TrustedAgentRole::Hub, crate::uri::URAKind::Hub) => {
                Ok(agent_uri.to_string())
            }
            (TrustedAgentRole::User, crate::uri::URAKind::User) => Ok(agent_uri.to_string()),
            (TrustedAgentRole::Device, kind) => Err(RealmTrustError::InvalidUriForRole {
                agent_uri: agent_uri.to_string(),
                role: "device".to_string(),
                detail: format!("expected a canonical device URI, got {kind:?}"),
            }),
            (TrustedAgentRole::Backend, kind) => Err(RealmTrustError::InvalidUriForRole {
                agent_uri: agent_uri.to_string(),
                role: "backend".to_string(),
                detail: format!("expected the realm hub URI, got {kind:?}"),
            }),
            (TrustedAgentRole::Hub, kind) => Err(RealmTrustError::InvalidUriForRole {
                agent_uri: agent_uri.to_string(),
                role: "hub".to_string(),
                detail: format!("expected the peer hub URI, got {kind:?}"),
            }),
            (TrustedAgentRole::User, kind) => Err(RealmTrustError::InvalidUriForRole {
                agent_uri: agent_uri.to_string(),
                role: "user".to_string(),
                detail: format!("expected a canonical user URI, got {kind:?}"),
            }),
        };
    }

    let Some((realm, _token)) = parse_legacy_bare_agent_uri(agent_uri) else {
        return Err(RealmTrustError::InvalidUriForRole {
            agent_uri: agent_uri.to_string(),
            role: match role {
                TrustedAgentRole::Backend => "backend".to_string(),
                TrustedAgentRole::Device => "device".to_string(),
                TrustedAgentRole::Hub => "hub".to_string(),
                TrustedAgentRole::User => "user".to_string(),
            },
            detail: "URI is neither canonical nor the supported legacy bare-agent fallback"
                .to_string(),
        });
    };
    Ok(match role {
        TrustedAgentRole::Device => {
            crate::uri::device_uri(realm, crate::uri::strip_v1_agent_prefix(agent_uri).as_str())
        }
        TrustedAgentRole::Backend | TrustedAgentRole::Hub => crate::uri::hub_uri(realm),
        TrustedAgentRole::User => {
            // No legacy bare-agent → user URI fallback. User URIs
            // arrived with v4.1.4; any legacy-shaped trust entry
            // claiming role="user" is operator error.
            return Err(RealmTrustError::InvalidUriForRole {
                agent_uri: agent_uri.to_string(),
                role: "user".to_string(),
                detail: "user role requires a canonical user URI; legacy bare-agent shape has no \
                         user-URI lift"
                    .to_string(),
            });
        }
    })
}

fn canonicalize_entry(mut entry: TrustedAgent) -> Result<TrustedAgent, RealmTrustError> {
    entry.agent_uri = canonical_uri_for_role(&entry.agent_uri, entry.role)?;
    Ok(entry)
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
        let mut anchor = Self::default();
        for entry in entries {
            let entry = canonicalize_entry(entry)?;
            anchor.insert_canonicalized(entry)?;
        }
        Ok(anchor)
    }

    /// Insert an already-canonicalised entry. Splits the user
    /// multi-pubkey path from the singleton-URI path so both
    /// `from_entries` and `append_agent` go through the same
    /// invariant check.
    fn insert_canonicalized(&mut self, entry: TrustedAgent) -> Result<(), RealmTrustError> {
        match entry.role {
            TrustedAgentRole::User => {
                let bucket = self.users.entry(entry.agent_uri.clone()).or_default();
                // (URI, pubkey) composite uniqueness: same key
                // registered twice under one user URI is operator
                // error; different keys are the multi-device
                // expected shape.
                if bucket.iter().any(|e| e.public_key_b64 == entry.public_key_b64) {
                    return Err(RealmTrustError::DuplicateUserPubkey {
                        agent_uri: entry.agent_uri,
                    });
                }
                bucket.push(entry);
            }
            TrustedAgentRole::Backend
            | TrustedAgentRole::Device
            | TrustedAgentRole::Hub => {
                if let Some(prior) = self.by_uri.insert(entry.agent_uri.clone(), entry.clone()) {
                    return Err(RealmTrustError::DuplicateUri {
                        agent_uri: prior.agent_uri,
                    });
                }
            }
        }
        Ok(())
    }

    fn parse(raw: &str, path: &Path) -> Result<Self, RealmTrustError> {
        let parsed: RawTrustAnchor =
            toml::from_str(raw).map_err(|source| RealmTrustError::ParseFailed {
                path: path.to_path_buf(),
                source,
            })?;
        Self::from_entries(parsed.trusted_agent)
    }

    /// Look up the trust entry for an agent URI.
    ///
    /// For hub / backend / device URIs (1:1 mapping) this is the
    /// authoritative resolution.
    ///
    /// For user URIs (1:N — DEC-EU multi-device) this returns the
    /// FIRST registered pubkey in lex order. Callers that need to
    /// verify against the exact pubkey the envelope presented must
    /// use [`lookup_user_by_pubkey`](#method.lookup_user_by_pubkey)
    /// instead. The single-value fallback exists so the existing
    /// `KeyResolver` trait (which takes only `caller_uri`) keeps
    /// working for the same-realm single-keypair common case; a
    /// full multi-pubkey resolver extension is tracked under
    /// DEC-EU §multi-realm.
    #[must_use]
    pub fn lookup(&self, agent_uri: &str) -> Option<&TrustedAgent> {
        if let Some(entry) = self.by_uri.get(agent_uri) {
            return Some(entry);
        }
        let canonical = crate::uri::canonicalize_presence_key(agent_uri);
        if canonical != agent_uri {
            if let Some(entry) = self.by_uri.get(&canonical) {
                return Some(entry);
            }
        }
        // User bucket fallback. Pick the lex-smallest pubkey so the
        // choice is deterministic across daemon restarts; the
        // single-pubkey trait shape is the caller's constraint, not
        // an ontological one.
        self.users.get(agent_uri).and_then(|bucket| {
            let mut sorted: Vec<&TrustedAgent> = bucket.iter().collect();
            sorted.sort_by(|a, b| a.public_key_b64.cmp(&b.public_key_b64));
            sorted.into_iter().next()
        })
    }

    /// DEC-EU: resolve a user envelope's caller against the
    /// (URI, pubkey) composite key. Returns the matching trust
    /// entry or `None` if either the URI is unknown or the
    /// presented pubkey is not registered under that URI.
    ///
    /// `presented_pubkey_b64` is the public key the caller's
    /// signature material claims to belong to; the admission
    /// gate is responsible for separately verifying that the
    /// signature is valid for that key. This method only answers
    /// "is this (URI, key) pair in the trust set".
    #[must_use]
    pub fn lookup_user_by_pubkey(
        &self,
        user_uri: &str,
        presented_pubkey_b64: &str,
    ) -> Option<&TrustedAgent> {
        let bucket = self.users.get(user_uri)?;
        bucket
            .iter()
            .find(|e| e.public_key_b64 == presented_pubkey_b64)
    }

    /// All trust entries registered under a user URI, regardless
    /// of pubkey. Used by audit / admin surfaces ("list alice's
    /// registered devices"); admission MUST use
    /// `lookup_user_by_pubkey` instead.
    #[must_use]
    pub fn lookup_user_all(&self, user_uri: &str) -> &[TrustedAgent] {
        self.users
            .get(user_uri)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// PR-N1 commit 2/N: cross-hub dialer peer lookup. Returns the
    /// `TrustedAgent` whose `hub_uri == target_hub` AND whose
    /// `role == Hub` AND whose `origin_tenant_id.is_some()`. The
    /// triple gate is the federation peer trust contract from
    /// DEC-N1 schema-B + PR-N1 spec §commit 2/N: the dialer never
    /// dials a peer that is not all three of operator-pinned,
    /// federation-roled, and tenant-tagged.
    ///
    /// Linear scan over the trust set. The federation peer
    /// population is operator-curated (tens of entries, not
    /// thousands), so a secondary index would be over-engineering.
    /// Re-evaluate if the scan ever shows up in admission profiles.
    #[must_use]
    pub fn lookup_peer_hub(&self, target_hub: &str) -> Option<&TrustedAgent> {
        self.by_uri.values().find(|a| {
            a.role == TrustedAgentRole::Hub
                && a.origin_tenant_id.is_some()
                && a.hub_uri.as_deref() == Some(target_hub)
        })
    }

    /// Number of trusted agents in the anchor. Used by the daemon
    /// boot log and by PR-10 canary checklist verification. Counts
    /// every entry, including each user-pubkey row separately
    /// (a user with 3 devices contributes 3 to `len()`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_uri.len() + self.users.values().map(Vec::len).sum::<usize>()
    }

    /// Whether the anchor is empty. Empty is allowed by PR-1
    /// (logged as WARN) but rejected by PR-10 canary's pre-swap
    /// verification.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_uri.is_empty() && self.users.is_empty()
    }

    /// Append a single trusted agent entry. Per Invariant 1
    /// (URI uniqueness) the same `agent_uri` cannot appear twice;
    /// a duplicate returns `RealmTrustError::DuplicateUri` so the
    /// caller (PR-7's pairing flow) can surface a structured
    /// "device already paired" error.
    ///
    /// `append_agent` is the in-memory mutation; persist with
    /// [`save`](#method.save) after the append succeeds. The two
    /// steps are split so the pairing flow can validate +
    /// transactionally roll the in-memory state forward (a partial
    /// disk write would be a recovery problem; the mutation +
    /// atomic-rename pair below addresses that).
    pub fn append_agent(&mut self, entry: TrustedAgent) -> Result<(), RealmTrustError> {
        let entry = canonicalize_entry(entry)?;
        self.insert_canonicalized(entry)
    }

    /// DEC-EU §revocation. Remove the (user_uri, pubkey) entry from
    /// the user bucket. Returns `Ok(true)` when an entry was
    /// removed, `Ok(false)` when no matching row existed (idempotent
    /// revoke for browsers that retry after a partial failure).
    ///
    /// Only user-role buckets are mutable through this API; removing
    /// hub / backend / device entries requires a different surface
    /// (operator-curated by hand), since those are realm-shaping
    /// decisions, not user-managed credentials.
    pub fn remove_user_pubkey(
        &mut self,
        user_uri: &str,
        public_key_b64: &str,
    ) -> Result<bool, RealmTrustError> {
        // Canonicalise the URI the same way append_agent does so a
        // caller that passes a legacy-shaped agent URI gets the same
        // resolution; for User this is a no-op today (no legacy
        // lift) but is symmetric with the write path.
        let canonical = canonical_uri_for_role(user_uri, TrustedAgentRole::User)?;
        let bucket = match self.users.get_mut(&canonical) {
            Some(b) => b,
            None => return Ok(false),
        };
        let before = bucket.len();
        bucket.retain(|e| e.public_key_b64 != public_key_b64);
        let removed = bucket.len() != before;
        if bucket.is_empty() {
            self.users.remove(&canonical);
        }
        Ok(removed)
    }

    /// Snapshot of the trust set as a sorted slice. Sort order is
    /// `(agent_uri, public_key_b64)` lexicographic so
    /// [`save`](#method.save) writes a stable file across
    /// restarts even when one user URI carries multiple pubkeys
    /// (DEC-EU). A hash-map iteration order would diff every
    /// save and defeat operator review.
    #[must_use]
    pub fn entries_sorted(&self) -> Vec<TrustedAgent> {
        let mut out: Vec<TrustedAgent> = self.by_uri.values().cloned().collect();
        for bucket in self.users.values() {
            out.extend(bucket.iter().cloned());
        }
        out.sort_by(|a, b| {
            a.agent_uri
                .cmp(&b.agent_uri)
                .then_with(|| a.public_key_b64.cmp(&b.public_key_b64))
        });
        out
    }

    /// Persist the trust anchor to `path` atomically: write to a
    /// sibling tempfile (`<path>.tmp`), fsync, then `rename(2)` on
    /// top of the existing file. POSIX guarantees rename is atomic
    /// for same-filesystem replacements, so a power failure mid-
    /// write leaves either the prior file or the new file —
    /// never a partial truncation.
    ///
    /// PR-7 commit 5/N's `<self>.register_device_pubkey` ability
    /// calls `save` after each successful `append_agent` and then
    /// signals SIGHUP to the daemon to trigger reload (the daemon
    /// boot loop's signal handler re-runs `load_or_empty` against
    /// the same path).
    ///
    /// Per `RawTrustAnchor`'s sort discipline (entries_sorted), the
    /// resulting TOML is byte-stable across saves with the same
    /// content — operator diffing actually shows real changes.
    pub fn save(&self, path: &Path) -> Result<(), RealmTrustError> {
        let raw = RawTrustAnchor {
            trusted_agent: self.entries_sorted(),
        };
        let body =
            toml::to_string_pretty(&raw).map_err(|source| RealmTrustError::SerializeFailed {
                path: path.to_path_buf(),
                source,
            })?;

        // Write to <path>.tmp in the same directory so `rename` is
        // a same-filesystem atomic operation. Different temp dirs
        // (e.g. /tmp vs /etc) cross filesystems, which downgrades
        // the rename to copy-then-unlink and loses atomicity.
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let tmp_name = match path.file_name() {
            Some(name) => {
                let mut s = name.to_os_string();
                s.push(".tmp");
                s
            }
            None => {
                return Err(RealmTrustError::WriteFailed {
                    path: path.to_path_buf(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "path has no file name component",
                    ),
                });
            }
        };
        let tmp_path = parent.join(tmp_name);

        // Open with O_CREAT | O_TRUNC | O_WRONLY semantics. Mode
        // 0600 on Unix — trust anchor contains public keys (not
        // secrets), but the file is admin-owned and operator
        // convention for /etc/easynet/* is owner-only.
        // Windows builds skip the mode bit (file ACL is the
        // platform's analog; daemon doesn't ship there yet).
        {
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            opts.mode(0o600);
            let mut file = opts
                .open(&tmp_path)
                .map_err(|source| RealmTrustError::WriteFailed {
                    path: tmp_path.clone(),
                    source,
                })?;
            file.write_all(body.as_bytes())
                .map_err(|source| RealmTrustError::WriteFailed {
                    path: tmp_path.clone(),
                    source,
                })?;
            file.sync_all()
                .map_err(|source| RealmTrustError::WriteFailed {
                    path: tmp_path.clone(),
                    source,
                })?;
        }
        fs::rename(&tmp_path, path).map_err(|source| {
            // Best-effort cleanup of the tmpfile; if rename failed,
            // leaving an orphan is the lesser evil (operators can
            // grep for `.tmp` and clean up).
            let _ = fs::remove_file(&tmp_path);
            RealmTrustError::WriteFailed {
                path: path.to_path_buf(),
                source,
            }
        })?;
        Ok(())
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

    #[error(
        "realm trust anchor invariant 1' violated: user `{agent_uri}` is already registered \
         with this exact public key. Different pubkeys under one user URI are allowed (multi-\
         device); the same pubkey twice is operator error."
    )]
    DuplicateUserPubkey { agent_uri: String },

    #[error("trusted {role} URI `{agent_uri}` is invalid: {detail}")]
    InvalidUriForRole {
        agent_uri: String,
        role: String,
        detail: String,
    },

    #[error("failed to write realm trust anchor at {path}: {source}")]
    WriteFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to serialize realm trust anchor for {path}: {source}")]
    SerializeFailed {
        path: PathBuf,
        #[source]
        source: toml::ser::Error,
    },
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
            origin_tenant_id: None,
            hub_uri: None,
            tls_ca_pem_path: None,
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
agent_uri = "easynet:///r/realm/hub"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "backend"
added_at_unix_ms = 1714492800000
        "#;

        let file = write_temp(toml_content);
        let anchor = RealmTrustAnchor::load_or_empty(file.path()).expect("Ok");
        assert_eq!(anchor.len(), 1);

        let entry = anchor.lookup("easynet:///r/realm/hub").expect("present");
        assert_eq!(entry.role, TrustedAgentRole::Backend);
        assert_eq!(entry.added_at_unix_ms, 1_714_492_800_000);
    }

    #[test]
    fn multiple_entries_with_distinct_uris_load() {
        let toml_content = r#"
[[trusted_agent]]
agent_uri = "easynet:///r/realm/hub"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "backend"
added_at_unix_ms = 1714492800000

[[trusted_agent]]
agent_uri = "easynet:///r/realm/device/laptop-1"
public_key_b64 = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB="
role = "device"
added_at_unix_ms = 1714492801234
        "#;

        let file = write_temp(toml_content);
        let anchor = RealmTrustAnchor::load_or_empty(file.path()).expect("Ok");
        assert_eq!(anchor.len(), 2);
        assert!(anchor.lookup("easynet:///r/realm/hub").is_some());
        assert!(anchor
            .lookup("easynet:///r/realm/device/laptop-1")
            .is_some());
        assert!(anchor.lookup("easynet:///r/realm/device/missing").is_none());
    }

    #[test]
    fn duplicate_uri_is_rejected() {
        let entries = vec![
            entry("easynet:///r/realm/device/n1"),
            entry("easynet:///r/realm/device/n1"),
        ];
        match RealmTrustAnchor::from_entries(entries) {
            Err(RealmTrustError::DuplicateUri { agent_uri }) => {
                assert_eq!(agent_uri, "easynet:///r/realm/device/n1");
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
agent_uri = "easynet:///r/realm/device/n1"
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
            entry("easynet:///r/realm/device/a"),
            entry("easynet:///r/realm/device/b"),
        ])
        .expect("Ok");
        assert_eq!(anchor.len(), 2);
        assert!(anchor.lookup("easynet:///r/realm/device/a").is_some());
        assert!(anchor.lookup("easynet:///r/realm/device/b").is_some());
    }

    // ── PR-7 commit 3/N: write-side tests ──────────────────────

    #[test]
    fn append_agent_adds_new_entry() {
        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(entry("easynet:///r/realm/device/n1"))
            .expect("append Ok");
        assert_eq!(anchor.len(), 1);
        assert!(anchor.lookup("easynet:///r/realm/device/n1").is_some());
    }

    #[test]
    fn append_agent_rejects_duplicate_uri() {
        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(entry("easynet:///r/realm/device/n1"))
            .expect("first append Ok");

        match anchor.append_agent(entry("easynet:///r/realm/device/n1")) {
            Err(RealmTrustError::DuplicateUri { agent_uri }) => {
                assert_eq!(agent_uri, "easynet:///r/realm/device/n1");
            }
            other => panic!("expected DuplicateUri, got {other:?}"),
        }
        // The map's first entry must still be present unchanged —
        // a failed append doesn't pollute the trust set.
        assert_eq!(anchor.len(), 1);
    }

    #[test]
    fn save_then_load_round_trip_preserves_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_uri: "easynet:///r/realm/hub".to_string(),
                public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                role: TrustedAgentRole::Backend,
                added_at_unix_ms: 1_714_492_800_000,
                origin_tenant_id: None,
                hub_uri: None,
                tls_ca_pem_path: None,
            })
            .expect("append backend");
        anchor
            .append_agent(TrustedAgent {
                agent_uri: "easynet:///r/realm/device/laptop-1".to_string(),
                public_key_b64: "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=".to_string(),
                role: TrustedAgentRole::Device,
                added_at_unix_ms: 1_714_492_801_234,
                origin_tenant_id: None,
                hub_uri: None,
                tls_ca_pem_path: None,
            })
            .expect("append laptop-1");

        anchor.save(&path).expect("save Ok");

        let loaded = RealmTrustAnchor::try_load_strict(&path).expect("strict load Ok");
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded.lookup("easynet:///r/realm/hub").map(|e| e.role),
            Some(TrustedAgentRole::Backend),
        );
        assert_eq!(
            loaded
                .lookup("easynet:///r/realm/device/laptop-1")
                .map(|e| e.role),
            Some(TrustedAgentRole::Device),
        );
    }

    #[test]
    fn save_is_atomic_no_partial_file_on_disk() {
        // After save() returns Ok, the target path exists and is
        // fully formed; the sibling .tmp file is gone.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");
        let tmp = dir.path().join("realm-trust.toml.tmp");

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(entry("easynet:///r/realm/device/n1"))
            .expect("append Ok");
        anchor.save(&path).expect("save Ok");

        assert!(path.exists(), "target file should exist after save");
        assert!(
            !tmp.exists(),
            ".tmp file must not be left behind on success"
        );
    }

    #[test]
    fn save_produces_stable_byte_output_under_same_content() {
        // Append the same entries in different insertion orders;
        // saved bytes must be identical because entries_sorted()
        // sorts on agent_uri. Operator diffing depends on this.
        let dir = tempfile::tempdir().expect("tempdir");
        let path_a = dir.path().join("a.toml");
        let path_b = dir.path().join("b.toml");

        let mut anchor_a = RealmTrustAnchor::default();
        anchor_a
            .append_agent(entry("easynet:///r/realm/device/a"))
            .expect("a");
        anchor_a
            .append_agent(entry("easynet:///r/realm/device/b"))
            .expect("b");

        let mut anchor_b = RealmTrustAnchor::default();
        anchor_b
            .append_agent(entry("easynet:///r/realm/device/b"))
            .expect("b");
        anchor_b
            .append_agent(entry("easynet:///r/realm/device/a"))
            .expect("a");

        anchor_a.save(&path_a).expect("save a");
        anchor_b.save(&path_b).expect("save b");

        let bytes_a = fs::read(&path_a).expect("read a");
        let bytes_b = fs::read(&path_b).expect("read b");
        assert_eq!(
            bytes_a, bytes_b,
            "save() must be order-stable so operator diffs surface real changes only",
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_sets_owner_only_mode_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(entry("easynet:///r/realm/device/n1"))
            .expect("append Ok");
        anchor.save(&path).expect("save Ok");

        let metadata = fs::metadata(&path).expect("stat");
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "trust anchor file must be 0600 on disk");
    }

    // ── PR-N1 commit 2/N: schema-B federation peer lookup ─────

    #[test]
    fn legacy_toml_without_schema_b_fields_loads() {
        // A `realm-trust.toml` written by a PR-1..PR-7 daemon
        // does not carry `origin_tenant_id` / `hub_uri` /
        // `tls_ca_pem_path`. PR-N1 daemons must load it
        // unchanged (DEC-N1 schema-B backwards-compat). Asserts
        // both the deserialise path AND that the schema-B fields
        // default to `None`.
        let toml_content = r#"
[[trusted_agent]]
agent_uri = "easynet:///r/realm/hub"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "backend"
added_at_unix_ms = 1714492800000
"#;
        let file = write_temp(toml_content);
        let anchor = RealmTrustAnchor::load_or_empty(file.path())
            .expect("legacy toml must load on a PR-N1 daemon");
        let entry = anchor
            .lookup("easynet:///r/realm/hub")
            .expect("legacy entry present");
        assert_eq!(entry.origin_tenant_id, None);
        assert_eq!(entry.hub_uri, None);
        assert_eq!(entry.tls_ca_pem_path, None);
    }

    #[test]
    fn schema_b_hub_entry_loads_with_all_three_fields() {
        let toml_content = r#"
[[trusted_agent]]
agent_uri = "easynet:///r/peer-realm/hub"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "hub"
added_at_unix_ms = 1714492800000
origin_tenant_id = "peer-realm"
hub_uri = "https://peer-hub.example:50443"
tls_ca_pem_path = "/etc/easynet/peer-hub-ca.pem"
"#;
        let file = write_temp(toml_content);
        let anchor = RealmTrustAnchor::load_or_empty(file.path()).expect("schema-B loads");
        let entry = anchor
            .lookup("easynet:///r/peer-realm/hub")
            .expect("schema-B entry present");
        assert_eq!(entry.origin_tenant_id.as_deref(), Some("peer-realm"));
        assert_eq!(
            entry.hub_uri.as_deref(),
            Some("https://peer-hub.example:50443")
        );
        assert_eq!(
            entry.tls_ca_pem_path.as_deref(),
            Some(Path::new("/etc/easynet/peer-hub-ca.pem")),
        );
    }

    #[test]
    fn lookup_peer_hub_finds_matching_federation_entry() {
        let target_hub = "https://peer-hub.example:50443";
        let mut entry = entry("easynet:///r/peer-realm/hub");
        entry.role = TrustedAgentRole::Hub;
        entry.origin_tenant_id = Some("peer-realm".to_string());
        entry.hub_uri = Some(target_hub.to_string());
        entry.tls_ca_pem_path = Some(PathBuf::from("/tmp/peer-ca.pem"));

        let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
        let found = anchor.lookup_peer_hub(target_hub).expect("peer found");
        assert_eq!(found.role, TrustedAgentRole::Hub);
        assert_eq!(found.origin_tenant_id.as_deref(), Some("peer-realm"));
    }

    #[test]
    fn lookup_peer_hub_skips_non_hub_role() {
        let target_hub = "https://peer-hub.example:50443";
        let mut entry = entry("easynet:///r/peer-realm/hub");
        // Backend role with a hub_uri set — operator typo. Must
        // not be returned by `lookup_peer_hub`.
        entry.role = TrustedAgentRole::Backend;
        entry.origin_tenant_id = Some("peer-realm".to_string());
        entry.hub_uri = Some(target_hub.to_string());
        entry.tls_ca_pem_path = Some(PathBuf::from("/tmp/peer-ca.pem"));

        let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
        assert!(anchor.lookup_peer_hub(target_hub).is_none());
    }

    #[test]
    fn lookup_peer_hub_skips_entry_missing_origin_tenant_id() {
        let target_hub = "https://peer-hub.example:50443";
        let mut entry = entry("easynet:///r/peer-realm/hub");
        entry.role = TrustedAgentRole::Hub;
        entry.origin_tenant_id = None;
        entry.hub_uri = Some(target_hub.to_string());
        entry.tls_ca_pem_path = Some(PathBuf::from("/tmp/peer-ca.pem"));

        let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
        assert!(anchor.lookup_peer_hub(target_hub).is_none());
    }

    #[test]
    fn lookup_peer_hub_returns_none_when_hub_uri_does_not_match() {
        let mut entry = entry("easynet:///r/peer-realm/hub");
        entry.role = TrustedAgentRole::Hub;
        entry.origin_tenant_id = Some("peer-realm".to_string());
        entry.hub_uri = Some("https://peer-hub.example:50443".to_string());
        entry.tls_ca_pem_path = Some(PathBuf::from("/tmp/peer-ca.pem"));

        let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
        assert!(anchor
            .lookup_peer_hub("https://different-hub.example:50443")
            .is_none());
    }

    // ── DEC-EU: user-as-first-class-caller ────────────────────

    #[test]
    fn user_role_round_trips_through_toml() {
        // The on-disk string MUST be lower-case "user" — Go-side
        // dev-init-trust writes role = "user" for every registered
        // user keypair. Asserts the serde lower-case rule and that
        // canonical user URIs survive round-trip without
        // canonicalisation rewriting them.
        let toml_content = r#"
[[trusted_agent]]
agent_uri = "easynet:///r/realm/user/alice"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "user"
added_at_unix_ms = 1714492800000
"#;
        let file = write_temp(toml_content);
        let anchor = RealmTrustAnchor::load_or_empty(file.path()).expect("user role loads");
        let entry = anchor
            .lookup_user_by_pubkey(
                "easynet:///r/realm/user/alice",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            )
            .expect("user entry present");
        assert_eq!(entry.role, TrustedAgentRole::User);
        // Single-keypair fallback path: lookup() returns the
        // user's only registered pubkey when no presented_pubkey
        // can disambiguate. Multi-device test
        // (user_multi_pubkey_lookup_returns_deterministic_first)
        // covers the deterministic ordering invariant.
        let any = anchor
            .lookup("easynet:///r/realm/user/alice")
            .expect("user single-keypair lookup returns the registered entry");
        assert_eq!(any.role, TrustedAgentRole::User);
    }

    #[test]
    fn user_role_with_hub_uri_is_rejected() {
        // A user trust entry pointing at a hub URI is operator
        // error; canonicalisation must refuse so it never lands
        // in the trust set silently.
        let bad = TrustedAgent {
            agent_uri: "easynet:///r/realm/hub".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::User,
            added_at_unix_ms: 1_714_492_800_000,
            origin_tenant_id: None,
            hub_uri: None,
            tls_ca_pem_path: None,
        };
        match RealmTrustAnchor::from_entries(vec![bad]) {
            Err(RealmTrustError::InvalidUriForRole { role, .. }) => {
                assert_eq!(role, "user");
            }
            other => panic!("expected InvalidUriForRole for user role, got {other:?}"),
        }
    }

    #[test]
    fn user_role_rejects_legacy_bare_agent_uri() {
        // No legacy lift for user URIs — v4.1.4 was the first
        // version to expose them; any pre-v4.1.4 entry tagged
        // role="user" is a typo.
        let bad = TrustedAgent {
            agent_uri: "easynet:///r/realm/agent/01ABC".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustedAgentRole::User,
            added_at_unix_ms: 1_714_492_800_000,
            origin_tenant_id: None,
            hub_uri: None,
            tls_ca_pem_path: None,
        };
        match RealmTrustAnchor::from_entries(vec![bad]) {
            Err(RealmTrustError::InvalidUriForRole { role, detail, .. }) => {
                assert_eq!(role, "user");
                assert!(
                    detail.contains("user-URI lift") || detail.contains("canonical"),
                    "detail should explain the no-legacy-user rule: {detail}",
                );
            }
            other => panic!("expected InvalidUriForRole, got {other:?}"),
        }
    }

    #[test]
    fn user_multi_pubkey_under_same_uri_is_admitted() {
        // DEC-EU §multi-device: one user URI, multiple pubkeys
        // (one per device). Both entries must coexist in the
        // trust set; lookup_user_by_pubkey selects by the
        // presented key.
        let pk_laptop = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let pk_phone = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=";
        let alice = "easynet:///r/realm/user/alice";

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_uri: alice.to_string(),
                public_key_b64: pk_laptop.to_string(),
                role: TrustedAgentRole::User,
                added_at_unix_ms: 1_714_492_800_000,
                origin_tenant_id: None,
                hub_uri: None,
                tls_ca_pem_path: None,
            })
            .expect("first user keypair");
        anchor
            .append_agent(TrustedAgent {
                agent_uri: alice.to_string(),
                public_key_b64: pk_phone.to_string(),
                role: TrustedAgentRole::User,
                added_at_unix_ms: 1_714_492_900_000,
                origin_tenant_id: None,
                hub_uri: None,
                tls_ca_pem_path: None,
            })
            .expect("second user keypair");

        assert_eq!(anchor.lookup_user_all(alice).len(), 2);
        assert_eq!(anchor.len(), 2);

        let laptop_entry = anchor
            .lookup_user_by_pubkey(alice, pk_laptop)
            .expect("laptop key resolves");
        assert_eq!(laptop_entry.public_key_b64, pk_laptop);

        let phone_entry = anchor
            .lookup_user_by_pubkey(alice, pk_phone)
            .expect("phone key resolves");
        assert_eq!(phone_entry.public_key_b64, pk_phone);

        assert!(anchor
            .lookup_user_by_pubkey(alice, "CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC=")
            .is_none());
    }

    #[test]
    fn user_same_pubkey_twice_is_rejected() {
        // Composite uniqueness: (URI, pubkey) is unique. The
        // pairing flow's "device already paired" surface depends
        // on this returning a structured error.
        let alice = "easynet:///r/realm/user/alice";
        let pk = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_uri: alice.to_string(),
                public_key_b64: pk.to_string(),
                role: TrustedAgentRole::User,
                added_at_unix_ms: 1_714_492_800_000,
                origin_tenant_id: None,
                hub_uri: None,
                tls_ca_pem_path: None,
            })
            .expect("first append");

        match anchor.append_agent(TrustedAgent {
            agent_uri: alice.to_string(),
            public_key_b64: pk.to_string(),
            role: TrustedAgentRole::User,
            added_at_unix_ms: 1_714_492_900_000,
            origin_tenant_id: None,
            hub_uri: None,
            tls_ca_pem_path: None,
        }) {
            Err(RealmTrustError::DuplicateUserPubkey { agent_uri }) => {
                assert_eq!(agent_uri, alice);
            }
            other => panic!("expected DuplicateUserPubkey, got {other:?}"),
        }

        assert_eq!(anchor.lookup_user_all(alice).len(), 1);
    }

    #[test]
    fn remove_user_pubkey_drops_only_the_named_key() {
        let alice = "easynet:///r/realm/user/alice";
        let pk_a = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let pk_b = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=";

        let mut anchor = RealmTrustAnchor::default();
        for pk in [pk_a, pk_b] {
            anchor
                .append_agent(TrustedAgent {
                    agent_uri: alice.to_string(),
                    public_key_b64: pk.to_string(),
                    role: TrustedAgentRole::User,
                    added_at_unix_ms: 1_714_492_800_000,
                    origin_tenant_id: None,
                    hub_uri: None,
                    tls_ca_pem_path: None,
                })
                .expect("append");
        }
        assert_eq!(anchor.lookup_user_all(alice).len(), 2);

        let removed = anchor
            .remove_user_pubkey(alice, pk_a)
            .expect("remove pk_a");
        assert!(removed);
        assert_eq!(anchor.lookup_user_all(alice).len(), 1);
        assert!(anchor.lookup_user_by_pubkey(alice, pk_a).is_none());
        assert!(anchor.lookup_user_by_pubkey(alice, pk_b).is_some());
    }

    #[test]
    fn remove_user_pubkey_collapses_bucket_when_last_key_revoked() {
        let alice = "easynet:///r/realm/user/alice";
        let pk = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_uri: alice.to_string(),
                public_key_b64: pk.to_string(),
                role: TrustedAgentRole::User,
                added_at_unix_ms: 1_714_492_800_000,
                origin_tenant_id: None,
                hub_uri: None,
                tls_ca_pem_path: None,
            })
            .expect("append");
        assert!(anchor.remove_user_pubkey(alice, pk).expect("remove"));
        // Bucket gone; subsequent removes return Ok(false) instead
        // of an error (idempotent retry contract).
        assert!(!anchor.remove_user_pubkey(alice, pk).expect("re-remove"));
        assert_eq!(anchor.lookup_user_all(alice).len(), 0);
        assert!(anchor.is_empty());
    }

    #[test]
    fn remove_user_pubkey_unknown_uri_is_noop() {
        let mut anchor = RealmTrustAnchor::default();
        let ok = anchor
            .remove_user_pubkey("easynet:///r/realm/user/missing", "AAAA")
            .expect("noop ok");
        assert!(!ok);
    }

    #[test]
    fn user_multi_pubkey_lookup_returns_deterministic_first() {
        // KeyResolver trait takes only the URI; for multi-device
        // users it gets the lex-smallest pubkey. Determinism is
        // load-bearing — admission must give the same answer
        // across restarts and across daemons.
        let alice = "easynet:///r/realm/user/alice";
        let pk_a = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let pk_b = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=";
        let mut anchor = RealmTrustAnchor::default();
        // Insert in reverse-lex order; lookup() must still return
        // pk_a (the lex-smallest).
        for pk in [pk_b, pk_a] {
            anchor
                .append_agent(TrustedAgent {
                    agent_uri: alice.to_string(),
                    public_key_b64: pk.to_string(),
                    role: TrustedAgentRole::User,
                    added_at_unix_ms: 1_714_000_000_000,
                    origin_tenant_id: None,
                    hub_uri: None,
                    tls_ca_pem_path: None,
                })
                .expect("append");
        }
        let resolved = anchor.lookup(alice).expect("resolves");
        assert_eq!(resolved.public_key_b64, pk_a);
    }

    #[test]
    fn user_multi_pubkey_round_trips_through_save_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");
        let alice = "easynet:///r/realm/user/alice";
        let pk_a = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let pk_b = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=";

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_uri: alice.to_string(),
                public_key_b64: pk_a.to_string(),
                role: TrustedAgentRole::User,
                added_at_unix_ms: 1_714_492_800_000,
                origin_tenant_id: None,
                hub_uri: None,
                tls_ca_pem_path: None,
            })
            .expect("pk_a");
        anchor
            .append_agent(TrustedAgent {
                agent_uri: alice.to_string(),
                public_key_b64: pk_b.to_string(),
                role: TrustedAgentRole::User,
                added_at_unix_ms: 1_714_492_900_000,
                origin_tenant_id: None,
                hub_uri: None,
                tls_ca_pem_path: None,
            })
            .expect("pk_b");

        anchor.save(&path).expect("save");
        let loaded = RealmTrustAnchor::try_load_strict(&path).expect("load");

        assert_eq!(loaded.lookup_user_all(alice).len(), 2);
        assert!(loaded.lookup_user_by_pubkey(alice, pk_a).is_some());
        assert!(loaded.lookup_user_by_pubkey(alice, pk_b).is_some());
    }

    #[test]
    fn user_role_save_load_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_uri: "easynet:///r/realm/user/alice".to_string(),
                public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                role: TrustedAgentRole::User,
                added_at_unix_ms: 1_714_492_800_000,
                origin_tenant_id: None,
                hub_uri: None,
                tls_ca_pem_path: None,
            })
            .expect("append user");
        anchor.save(&path).expect("save Ok");

        let loaded = RealmTrustAnchor::try_load_strict(&path).expect("load Ok");
        let entry = loaded
            .lookup_user_by_pubkey(
                "easynet:///r/realm/user/alice",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            )
            .expect("user present");
        assert_eq!(entry.role, TrustedAgentRole::User);
    }

    #[test]
    fn save_round_trips_schema_b_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_uri: "easynet:///r/peer-realm/hub".to_string(),
                public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                role: TrustedAgentRole::Hub,
                added_at_unix_ms: 1_714_492_800_000,
                origin_tenant_id: Some("peer-realm".to_string()),
                hub_uri: Some("https://peer-hub.example:50443".to_string()),
                tls_ca_pem_path: Some(PathBuf::from("/etc/easynet/peer-hub-ca.pem")),
            })
            .expect("append hub entry");

        anchor.save(&path).expect("save Ok");
        let loaded = RealmTrustAnchor::try_load_strict(&path).expect("load Ok");
        let entry = loaded
            .lookup("easynet:///r/peer-realm/hub")
            .expect("hub entry present");
        assert_eq!(entry.origin_tenant_id.as_deref(), Some("peer-realm"));
        assert_eq!(
            entry.hub_uri.as_deref(),
            Some("https://peer-hub.example:50443")
        );
        assert_eq!(
            entry.tls_ca_pem_path.as_deref(),
            Some(Path::new("/etc/easynet/peer-hub-ca.pem")),
        );
    }
}
