// EasyNet CLI - daemon trust anchor
// ===================================================
//
// File: src/daemon/trust/anchor.rs
// Description: TOML-backed loader for the per-host realm trust set
//              (`/etc/easynet/realm-trust.toml`). The daemon's
//              admission gate consults a `RealmTrustAnchor` to
//              answer "is this caller URA permitted to join this
//              realm".
//
// Where this fits in RFC-003
// --------------------------
// PR-7 authors `realm-trust.toml` via the device-pairing
// flow and the backend identity bootstrap. PR-1 (this commit, 7a/9)
// only reads the file: at daemon boot we either find it and parse
// every `[[trusted_agent]]` block or we report an explicit missing
// storage state to the boot policy boundary. The block name and
// `agent_ura` key are persisted compatibility names; values may be
// canonical caller/principal URAs such as Authority, Device, User, or
// Agent URAs. They do not mean every trusted signer is an Agent.
//
// What this module is
// -------------------
// - The TOML deserialisation surface (`[[trusted_agent]]` blocks)
// - The runtime representation (`RealmTrustAnchor` + `TrustedAgent`)
// - A loader that reports explicit storage state plus a
//   `try_load_strict` variant for paths that must exist
// - `lookup` to answer "do we have a public key for this URA"
//
// What this module is NOT
// -----------------------
// - The admission gate itself — that lives in `axon-sdk`'s
//   `invocation::admission` module and is consulted from
//   `daemon::invocation::dispatch::daemon_invocation_service` (commit 7b/9)
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
// agent_ura        = "easynet:///r/realm/authority"
// public_key_b64   = "..."
// role             = "backend"      # or "device" / "hub"
// added_at_unix_ms = 1714492800000
//
// [[trusted_agent]]
// agent_ura        = "easynet:///r/realm/device/laptop-1"
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

/// Resolve the realm trust anchor path for both daemon boot and
/// local inspection commands.
///
/// Resolution order:
/// 1. `EASYNET_REALM_TRUST_PATH` test/operator override.
/// 2. `/etc/easynet/realm-trust.toml` when it exists and is non-empty.
/// 3. `$HOME/.easynet/realm-trust.toml` for unprivileged host-mode installs.
///
/// This belongs with the anchor data model, not with the Axon gRPC
/// transport. `--no-default-features` builds still need to inspect
/// local trust state even though they cannot host or dial the
/// invocation transport.
pub fn trust_anchor_path_from_env_or_default() -> PathBuf {
    if let Some(override_path) = std::env::var_os("EASYNET_REALM_TRUST_PATH") {
        return expand_home(override_path.to_string_lossy().as_ref());
    }
    let etc = expand_home(DEFAULT_REALM_TRUST_PATH);
    if let Ok(meta) = std::fs::metadata(&etc) {
        if meta.is_file() && meta.len() > 0 {
            return etc;
        }
    }
    expand_home("~/.easynet/realm-trust.toml")
}

fn expand_home(raw: &str) -> PathBuf {
    if raw == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(raw)
}

/// Persisted role label for a realm trust-anchor entry. Used by audit log
/// formatters and pairing-flow validation.
///
/// The serialized trust file still uses `[[trusted_agent]]` and `agent_ura`
/// for compatibility, but this enum is not an Agent ontology. Admission lowers
/// this storage role into `TrustedCallerPath` before policy derives the logical
/// `PrincipalKind`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustAnchorRole {
    /// EasyNet backend service running alongside the hub-mode daemon.
    Backend,
    /// Consumer device daemon dialing in over TLS.
    Device,
    /// Cross-realm hub federate. RFC-N PR-N1 cross-hub dial gate
    /// requires `role == Hub` AND `origin_realm.is_some()`.
    /// DEC-N1 schema-B `origin_realm` field added in PR-N1
    /// commit 2/N below; PR-N2 fills in cross-realm admission key
    /// resolution against the same entry.
    Hub,
    /// End-user signing as a first-class Caller. DEC-EU
    /// (RFC-001 amendment "user-as-first-class-caller"): user
    /// holds an Ed25519 keypair and signs mutating envelopes
    /// directly instead of being a Subject under hub-as-Caller.
    ///
    /// DEC-EU multi-device: one user URA may own multiple
    /// non-exportable signing keys. Admission and key resolution
    /// must therefore bind user trust by `(user_ura, pubkey)`;
    /// a bare user URA is intentionally not sufficient to select
    /// signing material.
    User,
}

/// One entry in `realm-trust.toml`. Public so the admission gate
/// facade (commit 7b/9) and PR-7's pairing flow can consume the
/// shape directly.
///
/// The serialized table and field names stay `[[trusted_agent]]` and
/// `agent_ura` for trust-file compatibility. Semantically the field stores the
/// runtime caller/principal URA admitted by this trust anchor; it is not an
/// assertion that User, Device, or Authority identities are Agents.
///
/// PR-N1 commit 2/N adds three optional fields used only by the
/// cross-hub federation dialer (`role = Hub` entries):
/// `origin_realm`, `hub_endpoint`, `tls_ca_pem_path`. Backend /
/// Device entries leave them `None`; missing schema-B fields
/// deserialize to `None` via `#[serde(default)]`, while unknown
/// fields are rejected so stale aliases and operator typos cannot
/// silently affect trust-anchor behavior.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedAgent {
    /// Canonical runtime caller/principal URA per spec §5.1. The field name is
    /// a storage/wire compatibility name inherited from the original trust
    /// anchor file. Runtime code must treat the value as selected by `role`, not
    /// as proof that the principal is an Agent. The expected role→shape mapping
    /// is:
    /// - Backend => `easynet:///r/{realm}/authority`
    /// - Device  => `easynet:///r/{realm}/device/{device_id}`
    /// - Hub     => `easynet:///r/{realm}/authority`
    /// - User    => `easynet:///r/{realm}/user/{user_id}`
    pub agent_ura: String,
    /// Ed25519 verifying key, base64-encoded (32 raw bytes →
    /// 44 chars with padding). Validated by the admission gate
    /// when the entry is consulted, not at load time.
    pub public_key_b64: String,
    /// Storage trust role this caller/principal plays in the realm.
    pub role: TrustAnchorRole,
    /// Timestamp the entry was added by the pairing flow (PR-7).
    /// Surface only — admission does not policy-check on age.
    pub added_at_unix_ms: u64,
    /// **PR-N1 schema-B**. Realm this peer hub serves, in the form
    /// embedded in the peer's canonical hub URA. Set
    /// only on `role = Hub` entries; the admission gate uses this
    /// to resolve the caller URA realm into a peer hub URA when an
    /// invoke targets a tenant outside the local realm. `None` on
    /// Backend/Device entries and on schema-A Hub entries written
    /// before PR-N1; the dialer fail-closes when this is `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_realm: Option<String>,
    /// **PR-N1 schema-B**. Concrete dial URL for the peer hub,
    /// e.g. `"https://authority-b.example.com:50443"`. `Endpoint::
    /// from_shared(hub_endpoint)` is the only place this string is
    /// parsed — keep it operator-pasteable, not a structured URA.
    /// `None` ⇒ peer is not dial-eligible (not a federation peer
    /// or a schema-A entry); the dialer surfaces `PeerNotTrusted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hub_endpoint: Option<String>,
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

/// Ownership fact for a trusted runtime principal.
///
/// `TrustedAgent` answers "which key may sign for this principal URA".
/// This row answers "which user owns this principal URA" for RFC-014 policy
/// owner resolution. The canonical owner identity is the immutable User URA
/// and user id; product display aliases are intentionally excluded from this
/// authority fact.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedPrincipalOwner {
    /// Principal URA whose owner is known, e.g. a device URA.
    pub principal_ura: String,
    /// Canonical owner user id used by RFC-014 grant storage.
    pub owner_user_id: String,
    /// Canonical owner user URA.
    pub owner_ura: String,
    /// Timestamp the owner fact was written.
    pub added_at_unix_ms: u64,
}

/// Tombstone for a user public key that was explicitly revoked.
///
/// Revocation is persisted alongside active trust rows instead of being
/// inferred from "missing from `[[trusted_agent]]`". That gives the runtime
/// trust aggregate a durable read model for security-sensitive surfaces:
/// `rotation_epoch`, revoked-key count, and "this exact key may not be
/// re-admitted after revocation".
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RevokedUserPubkey {
    /// Canonical user URA whose key was revoked.
    pub(crate) agent_ura: String,
    /// Base64 Ed25519 verifying key that has been tombstoned.
    pub(crate) public_key_b64: String,
    /// Wall-clock timestamp when the local daemon recorded the revocation.
    pub(crate) revoked_at_unix_ms: u64,
    /// Monotonic per-user revocation epoch. The next successful revoke for
    /// the same user stores `max(existing rotation_epoch) + 1`.
    pub(crate) rotation_epoch: u64,
}

/// Internal TOML shape; private so the public `RealmTrustAnchor`
/// owns its index data structure choice.
#[derive(Debug, Default, Deserialize, Serialize)]
struct RawTrustAnchor {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    trusted_agent: Vec<TrustedAgent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    trusted_principal_owner: Vec<TrustedPrincipalOwner>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    revoked_user_pubkey: Vec<RevokedUserPubkey>,
}

/// Trust set the daemon consults at admission time. Built once at
/// boot from the on-disk TOML; PR-7 wires SIGHUP-triggered reload
/// against the same constructor.
///
/// **Invariant 1 (singleton-URA uniqueness)**: each runtime principal/caller URA
/// stored in the wire-compatible `agent_ura` field with role Backend / Device /
/// Hub appears at most once. A
/// duplicate-URA file is a configuration error — we reject at
/// load time so a typo never silently shadows an earlier entry.
///
/// **Invariant 1' (user multi-pubkey)**: DEC-EU lifts the strict
/// URA-uniqueness rule for `role = "user"`. RFC-001 "identity ≠
/// key" requires a user to retain the same identity URA across
/// devices while each device holds its own non-exportable
/// keypair. We therefore admit multiple `[[trusted_agent]]`
/// blocks sharing one user URA, gated by a composite uniqueness
/// rule: **(agent_ura, public_key_b64) is unique**. The pairing
/// flow rejects re-registering the same pubkey under the same
/// User principal URA; different pubkeys under the same User URA are the
/// expected multi-device shape.
///
/// **Invariant 2 (lookup is borrow)**: `lookup` returns a borrowed
/// `&TrustedAgent` so call sites do not clone the whole entry per
/// admission check. The admission gate copies only the public key
/// when it needs to. User admission goes through
/// [`lookup_user_by_pubkey`](#method.lookup_user_by_pubkey)
/// because a bare URA lookup is ambiguous when a user has
/// registered N devices.
///
/// **Load-state semantics**: missing storage is represented explicitly by
/// `RealmTrustAnchorLoadState::Missing`. The storage model never collapses
/// that state into an empty trust set; daemon boot, reload, CLI display, and
/// receipt verification each own their own policy boundary.
#[derive(Debug, Default)]
pub struct RealmTrustAnchor {
    /// Hub / Backend / Device entries — single value per URA.
    by_ura: HashMap<String, TrustedAgent>,
    /// User entries — DEC-EU multi-pubkey-per-URA. Each Vec is
    /// kept short (a typical user has 2-5 devices); a linear
    /// pubkey scan during admission is fine.
    users: HashMap<String, Vec<TrustedAgent>>,
    /// Persisted revocation tombstones for user-role pubkeys. This is
    /// separate from active `users` so a missing active key can be
    /// distinguished from an explicitly revoked key.
    revoked_users: HashMap<String, Vec<RevokedUserPubkey>>,
    /// RFC-014 owner facts for trusted runtime principals.
    principal_owners: HashMap<String, TrustedPrincipalOwner>,
}

#[derive(Debug)]
pub enum RealmTrustAnchorLoadState {
    Loaded(RealmTrustAnchor),
    Missing { path: PathBuf },
}

fn role_label(role: TrustAnchorRole) -> &'static str {
    match role {
        TrustAnchorRole::Backend => "backend",
        TrustAnchorRole::Device => "device",
        TrustAnchorRole::Hub => "hub",
        TrustAnchorRole::User => "user",
    }
}

fn canonical_ura_expectation(role: TrustAnchorRole) -> &'static str {
    match role {
        TrustAnchorRole::Backend => "expected the realm hub URA",
        TrustAnchorRole::Device => "expected a canonical device URA",
        TrustAnchorRole::Hub => "expected the peer hub URA",
        TrustAnchorRole::User => "expected a canonical user URA",
    }
}

fn canonical_ura_for_role(
    runtime_principal_ura: &str,
    role: TrustAnchorRole,
) -> Result<String, RealmTrustError> {
    let identity = crate::core::identity::RuntimeIdentityUra::parse(runtime_principal_ura)
        .map_err(|err| RealmTrustError::InvalidUraForRole {
            agent_ura: runtime_principal_ura.to_string(),
            role: role_label(role).to_string(),
            detail: format!("{}; parse failed: {err}", canonical_ura_expectation(role)),
        })?;

    match (role, identity.kind()) {
        (TrustAnchorRole::Device, crate::core::ura::URAKind::Device)
        | (TrustAnchorRole::Backend | TrustAnchorRole::Hub, crate::core::ura::URAKind::Authority)
        | (TrustAnchorRole::User, crate::core::ura::URAKind::User) => Ok(identity.into_string()),
        (_, kind) => Err(RealmTrustError::InvalidUraForRole {
            agent_ura: runtime_principal_ura.to_string(),
            role: role_label(role).to_string(),
            detail: format!("{}; got {kind:?}", canonical_ura_expectation(role)),
        }),
    }
}

fn canonicalize_entry(mut entry: TrustedAgent) -> Result<TrustedAgent, RealmTrustError> {
    entry.agent_ura = canonical_ura_for_role(&entry.agent_ura, entry.role)?;
    Ok(entry)
}

fn canonicalize_principal_owner(
    mut owner: TrustedPrincipalOwner,
) -> Result<TrustedPrincipalOwner, RealmTrustError> {
    owner.principal_ura = canonical_ura_for_runtime_principal(&owner.principal_ura)?;
    owner.owner_ura = canonical_ura_for_role(&owner.owner_ura, TrustAnchorRole::User)?;
    if owner.owner_user_id.trim().is_empty() {
        return Err(RealmTrustError::InvalidPrincipalOwner {
            principal_ura: owner.principal_ura,
            detail: "owner_user_id is required".to_string(),
        });
    }
    owner.owner_user_id = owner.owner_user_id.trim().to_string();
    let owner_ura = crate::core::ura::parse_ura(&owner.owner_ura).map_err(|err| {
        RealmTrustError::InvalidPrincipalOwner {
            principal_ura: owner.principal_ura.clone(),
            detail: format!("owner_ura parse failed after canonicalization: {err}"),
        }
    })?;
    if owner_ura.user_id() != Some(owner.owner_user_id.as_str()) {
        return Err(RealmTrustError::InvalidPrincipalOwner {
            principal_ura: owner.principal_ura,
            detail: "owner_ura user id must equal owner_user_id".to_string(),
        });
    }
    Ok(owner)
}

fn canonical_ura_for_runtime_principal(
    runtime_principal_ura: &str,
) -> Result<String, RealmTrustError> {
    let identity = crate::core::identity::RuntimeIdentityUra::parse(runtime_principal_ura)
        .map_err(|err| RealmTrustError::InvalidPrincipalOwner {
            principal_ura: runtime_principal_ura.to_string(),
            detail: err.to_string(),
        })?;
    match identity.kind() {
        crate::core::ura::URAKind::Agent
        | crate::core::ura::URAKind::Device
        | crate::core::ura::URAKind::Authority
        | crate::core::ura::URAKind::User => Ok(identity.into_string()),
        kind => Err(RealmTrustError::InvalidPrincipalOwner {
            principal_ura: runtime_principal_ura.to_string(),
            detail: format!("expected agent, device, hub, or user URA, got {kind:?}"),
        }),
    }
}

fn canonicalize_revoked_user_key(
    mut entry: RevokedUserPubkey,
) -> Result<RevokedUserPubkey, RealmTrustError> {
    entry.agent_ura = canonical_ura_for_role(&entry.agent_ura, TrustAnchorRole::User)?;
    Ok(entry)
}

impl RealmTrustAnchor {
    /// Load from `path` while preserving the exact storage state.
    ///
    /// Missing storage is not an error at this layer and is never projected
    /// into an empty trust set here. Callers must make that policy decision at
    /// their own boundary.
    pub fn load_with_state(path: &Path) -> Result<RealmTrustAnchorLoadState, RealmTrustError> {
        match fs::read_to_string(path) {
            Ok(raw) => Self::parse(&raw, path).map(RealmTrustAnchorLoadState::Loaded),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Ok(RealmTrustAnchorLoadState::Missing {
                    path: path.to_path_buf(),
                })
            }
            Err(source) => Err(RealmTrustError::ReadFailed {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Load from `path` and return an error if the file is missing.
    pub fn try_load_strict(path: &Path) -> Result<Self, RealmTrustError> {
        let raw = fs::read_to_string(path).map_err(|source| RealmTrustError::ReadFailed {
            path: path.to_path_buf(),
            source,
        })?;
        Self::parse(&raw, path)
    }

    /// Construct directly from already-deserialised entries. Public
    /// within crate tests so fixture-heavy admission and federation
    /// suites can build small anchors without TOML round-trips.
    #[cfg(test)]
    pub(crate) fn from_entries(entries: Vec<TrustedAgent>) -> Result<Self, RealmTrustError> {
        Self::from_parts(entries, Vec::new())
    }

    /// Construct from both active trust rows and persisted revocation
    /// tombstones. Mutation code uses this to rebuild a complete aggregate
    /// from a snapshot before applying one transaction.
    #[cfg(test)]
    pub(crate) fn from_parts(
        entries: Vec<TrustedAgent>,
        revoked_user_pubkeys: Vec<RevokedUserPubkey>,
    ) -> Result<Self, RealmTrustError> {
        Self::from_parts_with_principal_owners(entries, Vec::new(), revoked_user_pubkeys)
    }

    pub(crate) fn from_parts_with_principal_owners(
        entries: Vec<TrustedAgent>,
        principal_owners: Vec<TrustedPrincipalOwner>,
        revoked_user_pubkeys: Vec<RevokedUserPubkey>,
    ) -> Result<Self, RealmTrustError> {
        let mut anchor = Self::default();
        for revoked in revoked_user_pubkeys {
            let revoked = canonicalize_revoked_user_key(revoked)?;
            anchor.insert_revoked_canonicalized(revoked)?;
        }
        for entry in entries {
            let entry = canonicalize_entry(entry)?;
            anchor.insert_canonicalized(entry)?;
        }
        for owner in principal_owners {
            let owner = canonicalize_principal_owner(owner)?;
            anchor.upsert_principal_owner_canonicalized(owner)?;
        }
        Ok(anchor)
    }

    /// Insert an already-canonicalised entry. Splits the user
    /// multi-pubkey path from the singleton-URA path so both
    /// `from_entries` and `append_agent` go through the same
    /// invariant check.
    fn insert_canonicalized(&mut self, entry: TrustedAgent) -> Result<(), RealmTrustError> {
        match entry.role {
            TrustAnchorRole::User => {
                if self.is_user_pubkey_revoked(&entry.agent_ura, &entry.public_key_b64) {
                    return Err(RealmTrustError::RevokedUserPubkey {
                        agent_ura: entry.agent_ura,
                    });
                }
                let bucket = self.users.entry(entry.agent_ura.clone()).or_default();
                // (URA, pubkey) composite uniqueness: same key
                // registered twice under one user URA is operator
                // error; different keys are the multi-device
                // expected shape.
                if bucket
                    .iter()
                    .any(|e| e.public_key_b64 == entry.public_key_b64)
                {
                    return Err(RealmTrustError::DuplicateUserPubkey {
                        agent_ura: entry.agent_ura,
                    });
                }
                bucket.push(entry);
            }
            TrustAnchorRole::Backend | TrustAnchorRole::Device | TrustAnchorRole::Hub => {
                if self.by_ura.contains_key(&entry.agent_ura) {
                    return Err(RealmTrustError::DuplicateUra {
                        agent_ura: entry.agent_ura,
                    });
                }
                self.by_ura.insert(entry.agent_ura.clone(), entry);
            }
        }
        Ok(())
    }

    fn insert_revoked_canonicalized(
        &mut self,
        entry: RevokedUserPubkey,
    ) -> Result<(), RealmTrustError> {
        let bucket = self
            .revoked_users
            .entry(entry.agent_ura.clone())
            .or_default();
        if bucket
            .iter()
            .any(|e| e.public_key_b64 == entry.public_key_b64)
        {
            return Err(RealmTrustError::DuplicateRevokedUserPubkey {
                agent_ura: entry.agent_ura,
            });
        }
        bucket.push(entry);
        Ok(())
    }

    fn upsert_principal_owner_canonicalized(
        &mut self,
        mut owner: TrustedPrincipalOwner,
    ) -> Result<(), RealmTrustError> {
        if let Some(existing) = self.principal_owners.get(&owner.principal_ura) {
            if existing.owner_user_id != owner.owner_user_id
                || existing.owner_ura != owner.owner_ura
            {
                return Err(RealmTrustError::PrincipalOwnerConflict {
                    principal_ura: owner.principal_ura,
                });
            }
            owner.added_at_unix_ms = existing.added_at_unix_ms;
        }
        self.principal_owners
            .insert(owner.principal_ura.clone(), owner);
        Ok(())
    }

    fn parse(raw: &str, path: &Path) -> Result<Self, RealmTrustError> {
        let parsed: RawTrustAnchor =
            toml::from_str(raw).map_err(|source| RealmTrustError::ParseFailed {
                path: path.to_path_buf(),
                source,
            })?;
        Self::from_parts_with_principal_owners(
            parsed.trusted_agent,
            parsed.trusted_principal_owner,
            parsed.revoked_user_pubkey,
        )
    }

    /// Look up the singleton trust entry for a runtime caller/principal URA.
    ///
    /// This method intentionally excludes user entries. User URAs
    /// are 1:N under DEC-EU multi-device and therefore require
    /// explicit pubkey binding through
    /// [`lookup_user_by_pubkey`](#method.lookup_user_by_pubkey) or
    /// explicit enumeration through [`lookup_user_all`](#method.lookup_user_all).
    /// A bare user URA must not synthesize or select signing
    /// material.
    #[must_use]
    pub fn lookup(&self, runtime_principal_ura: &str) -> Option<&TrustedAgent> {
        self.by_ura.get(runtime_principal_ura)
    }

    #[must_use]
    pub fn lookup_principal_owner(&self, principal_ura: &str) -> Option<&TrustedPrincipalOwner> {
        self.principal_owners.get(principal_ura)
    }

    /// DEC-EU: resolve a user envelope's caller against the
    /// (URA, pubkey) composite key. Returns the matching trust
    /// entry or `None` if either the URA is unknown or the
    /// presented pubkey is not registered under that URA.
    ///
    /// `presented_pubkey_b64` is the public key the caller's
    /// signature material claims to belong to; the admission
    /// gate is responsible for separately verifying that the
    /// signature is valid for that key. This method only answers
    /// "is this (URA, key) pair in the trust set".
    #[must_use]
    pub fn lookup_user_by_pubkey(
        &self,
        user_ura: &str,
        presented_pubkey_b64: &str,
    ) -> Option<&TrustedAgent> {
        let bucket = self.users.get(user_ura)?;
        bucket
            .iter()
            .find(|e| e.public_key_b64 == presented_pubkey_b64)
    }

    /// All trust entries registered under a user URA, regardless
    /// of pubkey. Used by audit / admin surfaces ("list alice's
    /// registered devices"); admission MUST use
    /// `lookup_user_by_pubkey` instead.
    #[must_use]
    pub fn lookup_user_all(&self, user_ura: &str) -> &[TrustedAgent] {
        self.users
            .get(user_ura)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Current revocation epoch for a user. Epoch starts at 0 and advances
    /// only on successful user-key revocations.
    #[must_use]
    pub(crate) fn user_rotation_epoch(&self, user_ura: &str) -> u64 {
        self.revoked_users
            .get(user_ura)
            .and_then(|bucket| bucket.iter().map(|e| e.rotation_epoch).max())
            .unwrap_or(0)
    }

    /// Number of tombstoned keys for a user. Read models expose this as
    /// evidence that a missing key was explicitly revoked, not simply absent.
    #[must_use]
    pub(crate) fn revoked_user_pubkey_count(&self, user_ura: &str) -> usize {
        self.revoked_users.get(user_ura).map_or(0, Vec::len)
    }

    /// Returns true when a (user URA, pubkey) pair has a persisted
    /// revocation tombstone.
    #[must_use]
    pub(crate) fn is_user_pubkey_revoked(&self, user_ura: &str, public_key_b64: &str) -> bool {
        self.revoked_users
            .get(user_ura)
            .is_some_and(|bucket| bucket.iter().any(|e| e.public_key_b64 == public_key_b64))
    }

    /// PR-N1 commit 2/N: cross-hub dialer peer lookup. Returns the
    /// `TrustedAgent` whose `hub_endpoint == target_hub_endpoint` AND whose
    /// `role == Hub` AND whose `origin_realm.is_some()`. The
    /// triple gate is the federation peer trust contract from
    /// DEC-N1 schema-B + PR-N1 spec §commit 2/N: the dialer never
    /// dials a peer that is not all three of operator-pinned,
    /// federation-roled, and realm-tagged.
    ///
    /// Linear scan over the trust set. The federation peer
    /// population is operator-curated (tens of entries, not
    /// thousands), so a secondary index would be over-engineering.
    /// Re-evaluate if the scan ever shows up in admission profiles.
    #[must_use]
    pub fn lookup_peer_hub(&self, target_hub_endpoint: &str) -> Option<&TrustedAgent> {
        self.by_ura.values().find(|a| {
            a.role == TrustAnchorRole::Hub
                && a.origin_realm.is_some()
                && a.hub_endpoint.as_deref() == Some(target_hub_endpoint)
        })
    }

    /// Whether this anchor contains an operator-pinned federation peer
    /// serving `origin_realm`.
    ///
    /// This is the admission-side counterpart to [`lookup_peer_hub`]: the
    /// dialer resolves by concrete endpoint after routing, while the policy
    /// gate only needs to know whether a remote realm is an explicit peer
    /// before it treats the local daemon as a forwarder rather than the
    /// remote resource owner.
    #[must_use]
    pub fn has_federation_peer_for_realm(&self, origin_realm: &str) -> bool {
        let origin_realm = origin_realm.trim();
        !origin_realm.is_empty()
            && self.by_ura.values().any(|a| {
                a.role == TrustAnchorRole::Hub
                    && a.origin_realm.as_deref() == Some(origin_realm)
                    && a.hub_endpoint
                        .as_deref()
                        .is_some_and(|endpoint| !endpoint.trim().is_empty())
                    && a.tls_ca_pem_path.is_some()
            })
    }

    /// Number of trusted agents in the anchor. Used by the daemon
    /// boot log and by PR-10 canary checklist verification. Counts
    /// every entry, including each user-pubkey row separately
    /// (a user with 3 devices contributes 3 to `len()`).
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_ura.len() + self.users.values().map(Vec::len).sum::<usize>()
    }

    /// Whether the anchor is empty. Empty is allowed by PR-1
    /// (logged as WARN) but rejected by PR-10 canary's pre-swap
    /// verification.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_ura.is_empty() && self.users.is_empty()
    }

    /// Append a single trusted agent entry. Per Invariant 1
    /// (URA uniqueness) the same `agent_ura` cannot appear twice;
    /// a duplicate returns `RealmTrustError::DuplicateUra` so the
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

    pub fn upsert_principal_owner(
        &mut self,
        owner: TrustedPrincipalOwner,
    ) -> Result<(), RealmTrustError> {
        let owner = canonicalize_principal_owner(owner)?;
        self.upsert_principal_owner_canonicalized(owner)
    }

    /// Insert or replace one singleton-role trust entry.
    ///
    /// This is intentionally narrower than `append_agent`: it is
    /// only for roles whose URA owns exactly one active key at a
    /// time (`Backend`, `Hub`, and `Device`). It lets daemon
    /// bootstrap and hub-attested device trust sync repair a stale
    /// trust file after the runtime signing projection rotates
    /// without weakening User multi-pubkey semantics.
    #[cfg(feature = "axon-pb")]
    pub(crate) fn upsert_singleton_agent(
        &mut self,
        entry: TrustedAgent,
    ) -> Result<(), RealmTrustError> {
        let mut entry = canonicalize_entry(entry)?;
        match entry.role {
            TrustAnchorRole::Backend | TrustAnchorRole::Hub | TrustAnchorRole::Device => {
                if let Some(existing) = self.by_ura.get(&entry.agent_ura) {
                    if matches!(
                        existing.role,
                        TrustAnchorRole::Backend | TrustAnchorRole::Hub | TrustAnchorRole::Device
                    ) {
                        if existing.role == TrustAnchorRole::Hub {
                            entry.role = TrustAnchorRole::Hub;
                        }
                        if entry.origin_realm.is_none() {
                            entry.origin_realm = existing.origin_realm.clone();
                        }
                        if entry.hub_endpoint.is_none() {
                            entry.hub_endpoint = existing.hub_endpoint.clone();
                        }
                        if entry.tls_ca_pem_path.is_none() {
                            entry.tls_ca_pem_path = existing.tls_ca_pem_path.clone();
                        }
                    }
                }
                self.by_ura.insert(entry.agent_ura.clone(), entry);
                Ok(())
            }
            TrustAnchorRole::User => self.insert_canonicalized(entry),
        }
    }

    /// DEC-EU §revocation. Remove the active (user_ura, pubkey) entry and
    /// append a persisted tombstone in the same aggregate mutation. Returns
    /// `Ok(Some(tombstone))` when an active key was revoked, `Ok(None)` when
    /// no matching active row existed (idempotent revoke for clients that
    /// retry after a partial failure).
    ///
    /// Only user-role buckets are mutable through this API; removing hub /
    /// backend / device entries requires a different surface
    /// (operator-curated by hand), since those are realm-shaping decisions,
    /// not user-managed credentials.
    pub(crate) fn revoke_user_pubkey(
        &mut self,
        user_ura: &str,
        public_key_b64: &str,
        revoked_at_unix_ms: u64,
    ) -> Result<Option<RevokedUserPubkey>, RealmTrustError> {
        // Validate through the same canonical user-URA gate as
        // append_agent. Revocation is keyed by the exact user URA;
        // aliases are rejected instead of repaired.
        let canonical = canonical_ura_for_role(user_ura, TrustAnchorRole::User)?;
        let next_epoch = self.user_rotation_epoch(&canonical).saturating_add(1);
        let bucket = match self.users.get_mut(&canonical) {
            Some(bucket) => bucket,
            None => return Ok(None),
        };
        let before = bucket.len();
        bucket.retain(|e| e.public_key_b64 != public_key_b64);
        let removed = bucket.len() != before;
        if bucket.is_empty() {
            self.users.remove(&canonical);
        }
        if !removed {
            return Ok(None);
        }
        let tombstone = RevokedUserPubkey {
            agent_ura: canonical,
            public_key_b64: public_key_b64.to_string(),
            revoked_at_unix_ms,
            rotation_epoch: next_epoch,
        };
        self.insert_revoked_canonicalized(tombstone.clone())?;
        Ok(Some(tombstone))
    }

    /// Snapshot of the trust set as a sorted slice. Sort order is
    /// `(agent_ura, public_key_b64)` lexicographic so
    /// [`save`](#method.save) writes a stable file across
    /// restarts even when one user URA carries multiple pubkeys
    /// (DEC-EU). A hash-map iteration order would diff every
    /// save and defeat operator review.
    #[must_use]
    pub fn entries_sorted(&self) -> Vec<TrustedAgent> {
        let mut out: Vec<TrustedAgent> = self.by_ura.values().cloned().collect();
        for bucket in self.users.values() {
            out.extend(bucket.iter().cloned());
        }
        out.sort_by(|a, b| {
            a.agent_ura
                .cmp(&b.agent_ura)
                .then_with(|| a.public_key_b64.cmp(&b.public_key_b64))
        });
        out
    }

    #[must_use]
    pub fn principal_owners_sorted(&self) -> Vec<TrustedPrincipalOwner> {
        let mut out: Vec<TrustedPrincipalOwner> = self.principal_owners.values().cloned().collect();
        out.sort_by(|a, b| {
            a.principal_ura
                .cmp(&b.principal_ura)
                .then_with(|| a.owner_ura.cmp(&b.owner_ura))
        });
        out
    }

    /// Snapshot of persisted user-key revocations as a stable sorted list.
    /// Sort order mirrors active entries and then preserves epoch order for
    /// easier operator review.
    #[must_use]
    pub(crate) fn revoked_user_pubkeys_sorted(&self) -> Vec<RevokedUserPubkey> {
        let mut out = Vec::new();
        for bucket in self.revoked_users.values() {
            out.extend(bucket.iter().cloned());
        }
        out.sort_by(|a, b| {
            a.agent_ura
                .cmp(&b.agent_ura)
                .then_with(|| a.rotation_epoch.cmp(&b.rotation_epoch))
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
    /// PR-7 commit 5/N's `identity.register_pubkey` ability
    /// calls `save` after each successful `append_agent` and then
    /// signals SIGHUP to the daemon to trigger reload (the daemon
    /// boot loop's signal handler re-runs `load_with_state` against
    /// the same path).
    ///
    /// Per `RawTrustAnchor`'s sort discipline (entries_sorted), the
    /// resulting TOML is byte-stable across saves with the same
    /// content — operator diffing actually shows real changes.
    pub fn save(&self, path: &Path) -> Result<(), RealmTrustError> {
        let raw = RawTrustAnchor {
            trusted_agent: self.entries_sorted(),
            trusted_principal_owner: self.principal_owners_sorted(),
            revoked_user_pubkey: self.revoked_user_pubkeys_sorted(),
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
        if !path.is_absolute() {
            return Err(RealmTrustError::WriteFailed {
                path: path.to_path_buf(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "realm trust anchor save path must be absolute",
                ),
            });
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| RealmTrustError::WriteFailed {
                path: path.to_path_buf(),
                source: std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "path has no parent directory component",
                ),
            })?;
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
/// other than NotFound (NotFound is an explicit load state);
/// `ParseFailed` covers TOML syntax errors; `DuplicateUra` covers
/// the URA-uniqueness invariant.
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
        "realm trust anchor invariant 1 violated: agent_ura `{agent_ura}` appears more than \
         once. PR-7 pairing-flow writes must enforce uniqueness."
    )]
    DuplicateUra { agent_ura: String },

    #[error(
        "realm trust anchor invariant 1' violated: user `{agent_ura}` is already registered \
         with this exact public key. Different pubkeys under one user URA are allowed (multi-\
         device); the same pubkey twice is operator error."
    )]
    DuplicateUserPubkey { agent_ura: String },

    #[error(
        "realm trust anchor invariant 1'' violated: user `{agent_ura}` already has a revocation \
         tombstone for this public key."
    )]
    DuplicateRevokedUserPubkey { agent_ura: String },

    #[error(
        "realm trust anchor invariant 3 violated: user `{agent_ura}` cannot re-register a \
         public key that has a persisted revocation tombstone."
    )]
    RevokedUserPubkey { agent_ura: String },

    #[error("trusted {role} URA `{agent_ura}` is invalid: {detail}")]
    InvalidUraForRole {
        agent_ura: String,
        role: String,
        detail: String,
    },

    #[error("trusted principal owner `{principal_ura}` is invalid: {detail}")]
    InvalidPrincipalOwner {
        principal_ura: String,
        detail: String,
    },

    #[error(
        "trusted principal owner `{principal_ura}` conflicts with the existing canonical owner binding"
    )]
    PrincipalOwnerConflict { principal_ura: String },

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

    fn load_existing(path: &Path) -> RealmTrustAnchor {
        match RealmTrustAnchor::load_with_state(path).expect("load state") {
            RealmTrustAnchorLoadState::Loaded(anchor) => anchor,
            RealmTrustAnchorLoadState::Missing { path } => {
                panic!("expected existing trust anchor at {}", path.display())
            }
        }
    }

    fn entry(ura: &str) -> TrustedAgent {
        TrustedAgent {
            agent_ura: ura.to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustAnchorRole::Device,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }
    }

    #[test]
    fn missing_file_projects_explicit_load_state() {
        let nonexistent = PathBuf::from("/tmp/easynet-realm-trust-test-does-not-exist");
        match RealmTrustAnchor::load_with_state(&nonexistent).expect("load state") {
            RealmTrustAnchorLoadState::Missing { path } => assert_eq!(path, nonexistent),
            RealmTrustAnchorLoadState::Loaded(anchor) => {
                panic!("missing storage must not become empty anchor: {anchor:?}")
            }
        }
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
        let anchor = load_existing(file.path());
        assert!(anchor.is_empty());
    }

    #[test]
    fn single_entry_loads_and_lookups() {
        let toml_content = r#"
[[trusted_agent]]
agent_ura = "easynet:///r/realm/authority"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "backend"
added_at_unix_ms = 1714492800000
        "#;

        let file = write_temp(toml_content);
        let anchor = load_existing(file.path());
        assert_eq!(anchor.len(), 1);

        let entry = anchor
            .lookup("easynet:///r/realm/authority")
            .expect("present");
        assert_eq!(entry.role, TrustAnchorRole::Backend);
        assert_eq!(entry.added_at_unix_ms, 1_714_492_800_000);
    }

    #[test]
    fn multiple_entries_with_distinct_uras_load() {
        let toml_content = r#"
[[trusted_agent]]
agent_ura = "easynet:///r/realm/authority"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "backend"
added_at_unix_ms = 1714492800000

[[trusted_agent]]
agent_ura = "easynet:///r/realm/device/laptop-1"
public_key_b64 = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB="
role = "device"
added_at_unix_ms = 1714492801234
        "#;

        let file = write_temp(toml_content);
        let anchor = load_existing(file.path());
        assert_eq!(anchor.len(), 2);
        assert!(anchor.lookup("easynet:///r/realm/authority").is_some());
        assert!(anchor
            .lookup("easynet:///r/realm/device/laptop-1")
            .is_some());
        assert!(anchor.lookup("easynet:///r/realm/device/missing").is_none());
    }

    #[test]
    fn duplicate_ura_is_rejected() {
        let entries = vec![
            entry("easynet:///r/realm/device/n1"),
            entry("easynet:///r/realm/device/n1"),
        ];
        match RealmTrustAnchor::from_entries(entries) {
            Err(RealmTrustError::DuplicateUra { agent_ura }) => {
                assert_eq!(agent_ura, "easynet:///r/realm/device/n1");
            }
            other => panic!("expected DuplicateUra, got {other:?}"),
        }
    }

    #[test]
    fn malformed_toml_is_rejected() {
        let file = write_temp("this is not valid TOML {{{");
        match RealmTrustAnchor::load_with_state(file.path()) {
            Err(RealmTrustError::ParseFailed { .. }) => {}
            other => panic!("expected ParseFailed, got {other:?}"),
        }
    }

    #[test]
    fn unknown_role_value_is_rejected_at_parse() {
        let toml_content = r#"
[[trusted_agent]]
agent_ura = "easynet:///r/realm/device/n1"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "unknown"
        added_at_unix_ms = 1714492800000
        "#;
        let file = write_temp(toml_content);
        match RealmTrustAnchor::load_with_state(file.path()) {
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

    #[test]
    fn lookup_does_not_repair_bare_device_agent_alias() {
        let anchor = RealmTrustAnchor::from_entries(vec![entry("easynet:///r/realm/device/01ABC")])
            .expect("canonical device entry loads");

        assert!(anchor.lookup("easynet:///r/realm/device/01ABC").is_some());
        assert!(anchor.lookup("easynet:///r/realm/agent/01ABC").is_none());
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
    fn append_agent_rejects_duplicate_ura() {
        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(entry("easynet:///r/realm/device/n1"))
            .expect("first append Ok");

        match anchor.append_agent(entry("easynet:///r/realm/device/n1")) {
            Err(RealmTrustError::DuplicateUra { agent_ura }) => {
                assert_eq!(agent_ura, "easynet:///r/realm/device/n1");
            }
            other => panic!("expected DuplicateUra, got {other:?}"),
        }
        // The map's first entry must still be present unchanged —
        // a failed append doesn't pollute the trust set.
        assert_eq!(anchor.len(), 1);
    }

    fn assert_absolute_save_path_precondition(error: RealmTrustError) {
        match error {
            RealmTrustError::WriteFailed { source, .. } => {
                assert_eq!(
                    source.kind(),
                    std::io::ErrorKind::InvalidInput,
                    "save path precondition must fail before filesystem mutation"
                );
                assert!(
                    source.to_string().contains("must be absolute"),
                    "unexpected save path error: {source}"
                );
            }
            other => panic!("expected WriteFailed, got {other:?}"),
        }
    }

    #[test]
    fn save_rejects_relative_path_before_cwd_tmp_fallback() {
        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(entry("easynet:///r/realm/device/n1"))
            .expect("append Ok");
        let path = Path::new("target/realm-trust-relative-save-test.toml");

        let error = anchor
            .save(path)
            .expect_err("relative save path must not depend on cwd");

        assert_absolute_save_path_precondition(error);
    }

    #[test]
    fn save_rejects_dot_relative_path_before_cwd_tmp_fallback() {
        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(entry("easynet:///r/realm/device/n1"))
            .expect("append Ok");
        let path = Path::new("./target/realm-trust-dot-relative-save-test.toml");

        let error = anchor
            .save(path)
            .expect_err("dot-relative save path must not depend on cwd");

        assert_absolute_save_path_precondition(error);
    }

    #[test]
    fn save_then_load_round_trip_preserves_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_ura: "easynet:///r/realm/authority".to_string(),
                public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                role: TrustAnchorRole::Backend,
                added_at_unix_ms: 1_714_492_800_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            })
            .expect("append backend");
        anchor
            .append_agent(TrustedAgent {
                agent_ura: "easynet:///r/realm/device/laptop-1".to_string(),
                public_key_b64: "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=".to_string(),
                role: TrustAnchorRole::Device,
                added_at_unix_ms: 1_714_492_801_234,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            })
            .expect("append laptop-1");

        anchor.save(&path).expect("save Ok");

        let loaded = RealmTrustAnchor::try_load_strict(&path).expect("strict load Ok");
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded
                .lookup("easynet:///r/realm/authority")
                .map(|e| e.role),
            Some(TrustAnchorRole::Backend),
        );
        assert_eq!(
            loaded
                .lookup("easynet:///r/realm/device/laptop-1")
                .map(|e| e.role),
            Some(TrustAnchorRole::Device),
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
        // sorts on agent_ura. Operator diffing depends on this.
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
    fn schema_a_toml_without_schema_b_fields_loads() {
        // A `realm-trust.toml` written by a PR-1..PR-7 daemon
        // does not carry `origin_realm` / `hub_endpoint` /
        // `tls_ca_pem_path`. PR-N1 daemons must load it
        // unchanged (DEC-N1 schema-B backwards-compat). Asserts
        // both the deserialise path AND that the schema-B fields
        // default to `None`.
        let toml_content = r#"
[[trusted_agent]]
agent_ura = "easynet:///r/realm/authority"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "backend"
        added_at_unix_ms = 1714492800000
"#;
        let file = write_temp(toml_content);
        let anchor = load_existing(file.path());
        let entry = anchor
            .lookup("easynet:///r/realm/authority")
            .expect("schema-A entry present");
        assert_eq!(entry.origin_realm, None);
        assert_eq!(entry.hub_endpoint, None);
        assert_eq!(entry.tls_ca_pem_path, None);
    }

    #[test]
    fn schema_b_hub_entry_loads_with_all_three_fields() {
        let toml_content = r#"
[[trusted_agent]]
agent_ura = "easynet:///r/peer-realm/authority"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "hub"
added_at_unix_ms = 1714492800000
origin_realm = "peer-realm"
hub_endpoint = "https://peer-hub.example:50443"
tls_ca_pem_path = "/etc/easynet/peer-hub-ca.pem"
"#;
        let file = write_temp(toml_content);
        let anchor = load_existing(file.path());
        let entry = anchor
            .lookup("easynet:///r/peer-realm/authority")
            .expect("schema-B entry present");
        assert_eq!(entry.origin_realm.as_deref(), Some("peer-realm"));
        assert_eq!(
            entry.hub_endpoint.as_deref(),
            Some("https://peer-hub.example:50443")
        );
        assert_eq!(
            entry.tls_ca_pem_path.as_deref(),
            Some(Path::new("/etc/easynet/peer-hub-ca.pem")),
        );
    }

    #[test]
    fn schema_b_rejects_unknown_hub_endpoint_field_alias() {
        let toml_content = r#"
[[trusted_agent]]
agent_ura = "easynet:///r/peer-realm/authority"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "hub"
added_at_unix_ms = 1714492800000
origin_realm = "peer-realm"
hub_endpoint_alias = "https://peer-hub.example:50443"
tls_ca_pem_path = "/etc/easynet/peer-hub-ca.pem"
        "#;
        let file = write_temp(toml_content);
        let err = RealmTrustAnchor::try_load_strict(file.path())
            .expect_err("unknown hub endpoint alias must not deserialize as hub_endpoint");
        assert!(
            matches!(err, RealmTrustError::ParseFailed { .. }),
            "unknown hub endpoint alias should be rejected at schema parse: {err:?}"
        );
        let err_text = err.to_string();
        assert!(
            err_text.contains("hub_endpoint_alias") || err_text.contains("hub_endpoint"),
            "parse error should name the unknown or canonical hub endpoint field: {err_text}"
        );
    }

    #[test]
    fn schema_b_rejects_retired_origin_tenant_id_field() {
        let toml_content = r#"
[[trusted_agent]]
agent_ura = "easynet:///r/peer-realm/authority"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "hub"
added_at_unix_ms = 1714492800000
origin_tenant_id = "peer-realm"
hub_endpoint = "https://peer-hub.example:50443"
tls_ca_pem_path = "/etc/easynet/peer-hub-ca.pem"
"#;
        let file = write_temp(toml_content);
        let err = RealmTrustAnchor::try_load_strict(file.path())
            .expect_err("retired origin_tenant_id must not deserialize as origin_realm");
        assert!(
            matches!(err, RealmTrustError::ParseFailed { .. }),
            "retired origin_tenant_id should be rejected at schema parse: {err:?}"
        );
        let err_text = err.to_string();
        assert!(
            err_text.contains("origin_tenant_id") || err_text.contains("origin_realm"),
            "parse error should name the retired or canonical origin realm field: {err_text}"
        );
    }

    #[test]
    fn lookup_peer_hub_finds_matching_federation_entry() {
        let target_hub_endpoint = "https://peer-hub.example:50443";
        let mut entry = entry("easynet:///r/peer-realm/authority");
        entry.role = TrustAnchorRole::Hub;
        entry.origin_realm = Some("peer-realm".to_string());
        entry.hub_endpoint = Some(target_hub_endpoint.to_string());
        entry.tls_ca_pem_path = Some(PathBuf::from("/tmp/peer-ca.pem"));

        let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
        let found = anchor
            .lookup_peer_hub(target_hub_endpoint)
            .expect("peer found");
        assert_eq!(found.role, TrustAnchorRole::Hub);
        assert_eq!(found.origin_realm.as_deref(), Some("peer-realm"));
    }

    #[test]
    fn lookup_peer_hub_skips_non_hub_role() {
        let target_hub_endpoint = "https://peer-hub.example:50443";
        let mut entry = entry("easynet:///r/peer-realm/authority");
        // Backend role with a hub_endpoint set — operator typo. Must
        // not be returned by `lookup_peer_hub`.
        entry.role = TrustAnchorRole::Backend;
        entry.origin_realm = Some("peer-realm".to_string());
        entry.hub_endpoint = Some(target_hub_endpoint.to_string());
        entry.tls_ca_pem_path = Some(PathBuf::from("/tmp/peer-ca.pem"));

        let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
        assert!(anchor.lookup_peer_hub(target_hub_endpoint).is_none());
    }

    #[test]
    fn lookup_peer_hub_skips_entry_missing_origin_realm() {
        let target_hub_endpoint = "https://peer-hub.example:50443";
        let mut entry = entry("easynet:///r/peer-realm/authority");
        entry.role = TrustAnchorRole::Hub;
        entry.origin_realm = None;
        entry.hub_endpoint = Some(target_hub_endpoint.to_string());
        entry.tls_ca_pem_path = Some(PathBuf::from("/tmp/peer-ca.pem"));

        let anchor = RealmTrustAnchor::from_entries(vec![entry]).expect("anchor");
        assert!(anchor.lookup_peer_hub(target_hub_endpoint).is_none());
    }

    #[test]
    fn lookup_peer_hub_returns_none_when_hub_endpoint_does_not_match() {
        let mut entry = entry("easynet:///r/peer-realm/authority");
        entry.role = TrustAnchorRole::Hub;
        entry.origin_realm = Some("peer-realm".to_string());
        entry.hub_endpoint = Some("https://peer-hub.example:50443".to_string());
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
        // canonical user URAs survive round-trip without
        // canonicalisation rewriting them.
        let toml_content = r#"
[[trusted_agent]]
agent_ura = "easynet:///r/realm/user/alice"
public_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
role = "user"
added_at_unix_ms = 1714492800000
"#;
        let file = write_temp(toml_content);
        let anchor = load_existing(file.path());
        let entry = anchor
            .lookup_user_by_pubkey(
                "easynet:///r/realm/user/alice",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            )
            .expect("user entry present");
        assert_eq!(entry.role, TrustAnchorRole::User);
        assert!(
            anchor.lookup("easynet:///r/realm/user/alice").is_none(),
            "bare user URA lookup must not select signing material"
        );
    }

    #[test]
    fn user_role_with_hub_endpoint_is_rejected() {
        // A user trust entry pointing at a hub URA is operator
        // error; canonicalisation must refuse so it never lands
        // in the trust set silently.
        let bad = TrustedAgent {
            agent_ura: "easynet:///r/realm/authority".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustAnchorRole::User,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };
        match RealmTrustAnchor::from_entries(vec![bad]) {
            Err(RealmTrustError::InvalidUraForRole { role, .. }) => {
                assert_eq!(role, "user");
            }
            other => panic!("expected InvalidUraForRole for user role, got {other:?}"),
        }
    }

    #[test]
    fn user_role_rejects_all_zero_principal_before_trust_projection() {
        let bad = TrustedAgent {
            agent_ura: "easynet:///r/realm/user/00000000-0000-0000-0000-000000000000".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustAnchorRole::User,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };
        match RealmTrustAnchor::from_entries(vec![bad]) {
            Err(RealmTrustError::InvalidUraForRole { role, detail, .. }) => {
                assert_eq!(role, "user");
                assert!(
                    detail.contains("all-zero principal placeholder"),
                    "wrong trust projection error: {detail}"
                );
            }
            other => panic!("expected InvalidUraForRole for all-zero User, got {other:?}"),
        }
    }

    #[test]
    fn user_role_rejects_bare_device_agent_alias() {
        // No alias lift for user URAs; any bare-device agent alias
        // tagged role="user" is a typo.
        let bad = TrustedAgent {
            agent_ura: "easynet:///r/realm/agent/01ABC".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustAnchorRole::User,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };
        match RealmTrustAnchor::from_entries(vec![bad]) {
            Err(RealmTrustError::InvalidUraForRole { role, detail, .. }) => {
                assert_eq!(role, "user");
                assert!(
                    detail.contains("expected a canonical user URA"),
                    "detail should explain the no-user-alias rule: {detail}",
                );
            }
            other => panic!("expected InvalidUraForRole, got {other:?}"),
        }
    }

    #[test]
    fn device_role_rejects_bare_device_agent_alias() {
        let bad = TrustedAgent {
            agent_ura: "easynet:///r/realm/agent/01ABC".to_string(),
            public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            role: TrustAnchorRole::Device,
            added_at_unix_ms: 1_714_492_800_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        };

        match RealmTrustAnchor::from_entries(vec![bad]) {
            Err(RealmTrustError::InvalidUraForRole { role, detail, .. }) => {
                assert_eq!(role, "device");
                assert!(
                    detail.contains("expected a canonical device URA"),
                    "detail should explain the canonical device requirement: {detail}",
                );
            }
            other => panic!("expected InvalidUraForRole for device role, got {other:?}"),
        }
    }

    #[test]
    fn user_multi_pubkey_under_same_ura_is_admitted() {
        // DEC-EU §multi-device: one user URA, multiple pubkeys
        // (one per device). Both entries must coexist in the
        // trust set; lookup_user_by_pubkey selects by the
        // presented key.
        let pk_laptop = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let pk_phone = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=";
        let alice = "easynet:///r/realm/user/alice";

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_ura: alice.to_string(),
                public_key_b64: pk_laptop.to_string(),
                role: TrustAnchorRole::User,
                added_at_unix_ms: 1_714_492_800_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            })
            .expect("first user keypair");
        anchor
            .append_agent(TrustedAgent {
                agent_ura: alice.to_string(),
                public_key_b64: pk_phone.to_string(),
                role: TrustAnchorRole::User,
                added_at_unix_ms: 1_714_492_900_000,
                origin_realm: None,
                hub_endpoint: None,
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
        // Composite uniqueness: (URA, pubkey) is unique. The
        // pairing flow's "device already paired" surface depends
        // on this returning a structured error.
        let alice = "easynet:///r/realm/user/alice";
        let pk = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_ura: alice.to_string(),
                public_key_b64: pk.to_string(),
                role: TrustAnchorRole::User,
                added_at_unix_ms: 1_714_492_800_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            })
            .expect("first append");

        match anchor.append_agent(TrustedAgent {
            agent_ura: alice.to_string(),
            public_key_b64: pk.to_string(),
            role: TrustAnchorRole::User,
            added_at_unix_ms: 1_714_492_900_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }) {
            Err(RealmTrustError::DuplicateUserPubkey { agent_ura }) => {
                assert_eq!(agent_ura, alice);
            }
            other => panic!("expected DuplicateUserPubkey, got {other:?}"),
        }

        assert_eq!(anchor.lookup_user_all(alice).len(), 1);
    }

    #[test]
    fn revoke_user_pubkey_drops_only_the_named_key_and_records_tombstone() {
        let alice = "easynet:///r/realm/user/alice";
        let pk_a = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let pk_b = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=";

        let mut anchor = RealmTrustAnchor::default();
        for pk in [pk_a, pk_b] {
            anchor
                .append_agent(TrustedAgent {
                    agent_ura: alice.to_string(),
                    public_key_b64: pk.to_string(),
                    role: TrustAnchorRole::User,
                    added_at_unix_ms: 1_714_492_800_000,
                    origin_realm: None,
                    hub_endpoint: None,
                    tls_ca_pem_path: None,
                })
                .expect("append");
        }
        assert_eq!(anchor.lookup_user_all(alice).len(), 2);

        let tombstone = anchor
            .revoke_user_pubkey(alice, pk_a, 1_714_493_000_000)
            .expect("revoke pk_a")
            .expect("active key revoked");
        assert_eq!(anchor.lookup_user_all(alice).len(), 1);
        assert!(anchor.lookup_user_by_pubkey(alice, pk_a).is_none());
        assert!(anchor.lookup_user_by_pubkey(alice, pk_b).is_some());
        assert_eq!(tombstone.public_key_b64, pk_a);
        assert_eq!(tombstone.revoked_at_unix_ms, 1_714_493_000_000);
        assert_eq!(tombstone.rotation_epoch, 1);
        assert_eq!(anchor.user_rotation_epoch(alice), 1);
        assert_eq!(anchor.revoked_user_pubkey_count(alice), 1);
        assert!(anchor.is_user_pubkey_revoked(alice, pk_a));
    }

    #[test]
    fn revoke_user_pubkey_collapses_bucket_when_last_key_revoked() {
        let alice = "easynet:///r/realm/user/alice";
        let pk = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_ura: alice.to_string(),
                public_key_b64: pk.to_string(),
                role: TrustAnchorRole::User,
                added_at_unix_ms: 1_714_492_800_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            })
            .expect("append");
        assert!(anchor
            .revoke_user_pubkey(alice, pk, 1_714_493_000_000)
            .expect("remove")
            .is_some());
        // Bucket gone; subsequent revokes return Ok(None) instead of an error
        // and do not append another tombstone (idempotent retry contract).
        assert!(anchor
            .revoke_user_pubkey(alice, pk, 1_714_493_100_000)
            .expect("re-remove")
            .is_none());
        assert_eq!(anchor.lookup_user_all(alice).len(), 0);
        assert_eq!(anchor.revoked_user_pubkey_count(alice), 1);
        assert!(anchor.is_empty());
    }

    #[test]
    fn revoke_user_pubkey_unknown_ura_is_noop() {
        let mut anchor = RealmTrustAnchor::default();
        let ok = anchor
            .revoke_user_pubkey("easynet:///r/realm/user/missing", "AAAA", 1_714_493_000_000)
            .expect("noop ok");
        assert!(ok.is_none());
    }

    #[test]
    fn user_multi_pubkey_lookup_requires_presented_pubkey() {
        // User trust is a composite `(user_ura, pubkey)` binding.
        // A bare URA lookup must fail closed instead of selecting
        // a deterministic but semantically arbitrary key.
        let alice = "easynet:///r/realm/user/alice";
        let pk_a = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let pk_b = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBA=";
        let mut anchor = RealmTrustAnchor::default();
        for pk in [pk_b, pk_a] {
            anchor
                .append_agent(TrustedAgent {
                    agent_ura: alice.to_string(),
                    public_key_b64: pk.to_string(),
                    role: TrustAnchorRole::User,
                    added_at_unix_ms: 1_714_000_000_000,
                    origin_realm: None,
                    hub_endpoint: None,
                    tls_ca_pem_path: None,
                })
                .expect("append");
        }
        assert!(anchor.lookup(alice).is_none());
        assert_eq!(
            anchor
                .lookup_user_by_pubkey(alice, pk_a)
                .expect("explicit key resolves")
                .public_key_b64,
            pk_a
        );
        assert_eq!(
            anchor
                .lookup_user_by_pubkey(alice, pk_b)
                .expect("explicit key resolves")
                .public_key_b64,
            pk_b
        );
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
                agent_ura: alice.to_string(),
                public_key_b64: pk_a.to_string(),
                role: TrustAnchorRole::User,
                added_at_unix_ms: 1_714_492_800_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            })
            .expect("pk_a");
        anchor
            .append_agent(TrustedAgent {
                agent_ura: alice.to_string(),
                public_key_b64: pk_b.to_string(),
                role: TrustAnchorRole::User,
                added_at_unix_ms: 1_714_492_900_000,
                origin_realm: None,
                hub_endpoint: None,
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
    fn revoked_user_pubkey_round_trips_through_save_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");
        let alice = "easynet:///r/realm/user/alice";
        let pk = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_ura: alice.to_string(),
                public_key_b64: pk.to_string(),
                role: TrustAnchorRole::User,
                added_at_unix_ms: 1_714_492_800_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            })
            .expect("append");
        anchor
            .revoke_user_pubkey(alice, pk, 1_714_493_000_000)
            .expect("revoke")
            .expect("tombstone");

        anchor.save(&path).expect("save");
        let loaded = RealmTrustAnchor::try_load_strict(&path).expect("load");

        assert_eq!(loaded.lookup_user_all(alice).len(), 0);
        assert_eq!(loaded.revoked_user_pubkey_count(alice), 1);
        assert_eq!(loaded.user_rotation_epoch(alice), 1);
        assert!(loaded.is_user_pubkey_revoked(alice, pk));
    }

    #[test]
    fn revoked_user_pubkey_cannot_be_registered_again() {
        let alice = "easynet:///r/realm/user/alice";
        let pk = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_ura: alice.to_string(),
                public_key_b64: pk.to_string(),
                role: TrustAnchorRole::User,
                added_at_unix_ms: 1_714_492_800_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            })
            .expect("append");
        anchor
            .revoke_user_pubkey(alice, pk, 1_714_493_000_000)
            .expect("revoke")
            .expect("tombstone");

        match anchor.append_agent(TrustedAgent {
            agent_ura: alice.to_string(),
            public_key_b64: pk.to_string(),
            role: TrustAnchorRole::User,
            added_at_unix_ms: 1_714_493_100_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }) {
            Err(RealmTrustError::RevokedUserPubkey { agent_ura }) => assert_eq!(agent_ura, alice),
            other => panic!("expected RevokedUserPubkey, got {other:?}"),
        }
    }

    #[test]
    fn user_role_save_load_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_ura: "easynet:///r/realm/user/alice".to_string(),
                public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                role: TrustAnchorRole::User,
                added_at_unix_ms: 1_714_492_800_000,
                origin_realm: None,
                hub_endpoint: None,
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
        assert_eq!(entry.role, TrustAnchorRole::User);
    }

    #[test]
    fn save_round_trips_schema_b_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("realm-trust.toml");

        let mut anchor = RealmTrustAnchor::default();
        anchor
            .append_agent(TrustedAgent {
                agent_ura: "easynet:///r/peer-realm/authority".to_string(),
                public_key_b64: "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
                role: TrustAnchorRole::Hub,
                added_at_unix_ms: 1_714_492_800_000,
                origin_realm: Some("peer-realm".to_string()),
                hub_endpoint: Some("https://peer-hub.example:50443".to_string()),
                tls_ca_pem_path: Some(PathBuf::from("/etc/easynet/peer-hub-ca.pem")),
            })
            .expect("append hub entry");

        anchor.save(&path).expect("save Ok");
        let loaded = RealmTrustAnchor::try_load_strict(&path).expect("load Ok");
        let entry = loaded
            .lookup("easynet:///r/peer-realm/authority")
            .expect("hub entry present");
        assert_eq!(entry.origin_realm.as_deref(), Some("peer-realm"));
        assert_eq!(
            entry.hub_endpoint.as_deref(),
            Some("https://peer-hub.example:50443")
        );
        assert_eq!(
            entry.tls_ca_pem_path.as_deref(),
            Some(Path::new("/etc/easynet/peer-hub-ca.pem")),
        );
    }
}
