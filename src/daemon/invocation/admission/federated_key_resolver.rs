// EasyNet CLI — FederatedKeyResolver (PR-N2 commit 1/N)
// =======================================================
//
// File: src/daemon/invocation/federated_key_resolver.rs
// Description: KeyResolver wrapping the local TrustAnchorKeyResolver
//              with cross-realm fall-through to a peer hub's
//              `federation.resolve_key` ability via the PR-N1
//              CrossHubDialer.
//
// Why this resolver exists
// ------------------------
// PR-7 commit 4/N's TrustAnchorKeyResolver only knows about
// agents in the local realm's `realm-trust.toml`. Cross-realm
// callers (a device in realm A signing an envelope that lands
// at hub B) hit `CALLER_KEY_NOT_FOUND` and admission rejects with
// key-resolution failure regardless of whether the
// signature is correct, because hub B has no way to fetch
// hub A's pubkey for verification.
//
// PR-N2 closes that gap. FederatedKeyResolver:
//
//   1. Tries the local trust anchor first (INV-2 local-first).
//      Same-realm callers short-circuit before any network I/O.
//   2. On local miss, parses the caller URA's realm and checks
//      the `realm-trust.toml` for a `[[trusted_agent]]` entry
//      whose `origin_realm` matches that realm (INV-1
//      federated trust gate). Operators explicitly opt into
//      cross-realm verification by adding such entries.
//   3. Calls `federation.resolve_key` on the peer hub via the
//      PR-N1 CrossHubDialer, returning the peer's pubkey for
//      the caller URA (INV-3 same Ed25519 4-step verify).
//   4. Dial failure surfaces as `CALLER_KEY_NOT_FOUND` so the
//      admission gate's reject path runs identically to a
//      local trust miss (INV-4 fail-closed).
//
// Sync trait shape
// ----------------
// `axon_sdk::invocation::axiom::KeyResolver::resolve` is a
// sync method (called from inside the sync `run_admission`
// pipeline). The cross-hub dial is async (tonic). We bridge via
// `tokio::task::block_in_place` + `Handle::current().block_on`
// — safe because the admission gate runs on a `tokio::spawn`
// task in the gRPC server, and `block_in_place` is the
// canonical pattern when a sync call needs to await inside an
// async runtime.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use axon_sdk::invocation::axiom::KeyResolver;
use axon_sdk::invocation::{AxonError, AxonErrorKind, ErrorCode, ErrorStage, SecurityClass};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ed25519_dalek::VerifyingKey;

use crate::core::ura::{parse_ura, URAKind};
use crate::daemon::federation::client::FederationClient;
use crate::daemon::federation::peers::SharedFederatedPeers;
use crate::daemon::identity::self_identity::CanonicalSigner;
use crate::daemon::invocation::admission::peer_envelope_signer::{
    PeerInvocationSubject, PeerInvokeRequest,
};
use crate::daemon::invocation::admission::principal_lifecycle::PrincipalLifecycleReader;
use crate::daemon::trust::cell::SharedTrustAnchor;
#[cfg(test)]
use axon_sdk::pb::axon::v1::InvokeRequest;

/// Default TTL for a federated-resolve cache entry. 5 minutes
/// trims a hot signed-call path from O(N) cross-hub round-trips
/// to ~O(N/window) without making key-rotation observability
/// worse than the SIGHUP cadence operators already rely on for
/// trust-anchor reloads. Tunable via
/// [`FederatedKeyResolver::with_cache_ttl`] for tests.
pub const DEFAULT_FEDERATED_RESOLVE_CACHE_TTL: Duration = Duration::from_secs(300);

/// Per-entry record in the federated-resolve cache. Stores the
/// resolved verifying key plus the deadline after which the
/// entry is considered stale.
#[derive(Clone)]
struct CachedKey {
    key: VerifyingKey,
    expires_at: Instant,
}

/// Shared TTL cache handle. Lives at AdmissionFacade scope so a
/// new `FederatedKeyResolver` constructed per admission call
/// inherits the same in-process cache state. Cloning is cheap
/// (one `Arc::clone`); mutations go through the inner `Mutex`.
#[derive(Clone, Default)]
pub struct SharedFederatedKeyCache {
    inner: Arc<Mutex<HashMap<String, CachedKey>>>,
}

impl SharedFederatedKeyCache {
    /// Construct an empty shared cache. Same shape as
    /// `Default::default()` but explicit at boot sites.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Drop every cached entry. Operator SIGHUP entry point —
    /// trust-anchor reload calls this so a key rotation
    /// propagates without waiting for the per-entry TTL.
    pub fn flush(&self) {
        match self.inner.lock() {
            Ok(mut g) => g.clear(),
            Err(poisoned) => poisoned.into_inner().clear(),
        }
    }

    /// Test-only: total cached entries.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        match self.inner.lock() {
            Ok(g) => g.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    /// Test-only: whether the cache currently holds no entries.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        match self.inner.lock() {
            Ok(g) => g.is_empty(),
            Err(poisoned) => poisoned.into_inner().is_empty(),
        }
    }
}

/// Resolves an `agent_ura` to its Ed25519 verifying key, falling
/// through to a federated lookup when the local trust anchor has
/// no entry for the URA and the caller's realm is one the
/// operator has marked as federated via DEC-N1 schema-B
/// `origin_realm` on a `[[trusted_agent]]` entry.
#[derive(Clone)]
pub struct FederatedKeyResolver {
    trust_anchor: SharedTrustAnchor,
    federation_client: Option<Arc<dyn FederationClient>>,
    federated_peers: SharedFederatedPeers,
    self_realm: Option<String>,
    /// Public key presented by the envelope being admitted, encoded as
    /// standard base64. User URAs are 1:N, so cross-realm
    /// `federation.resolve_key` must forward this pin to the peer hub.
    presented_pubkey_b64: Option<String>,
    /// 5-min TTL cache on cross-hub `federation.resolve_key`
    /// outcomes. Keyed by full `agent_ura`. Operators flush on
    /// trust-anchor SIGHUP via [`SharedFederatedKeyCache::flush`]
    /// so a key rotation propagates without a daemon restart.
    /// The mutex is held for the duration of one HashMap lookup
    /// / insert / drain — never across the cross-hub dial
    /// itself, so concurrent first-time resolves on disjoint
    /// URAs never serialize.
    ///
    /// The cache lives at AdmissionFacade scope (passed in via
    /// `with_cache`) so the per-admission-call resolver
    /// instance inherits the same in-process state. Without
    /// this share, the cache would reset to empty on every
    /// admission call and never deliver any savings.
    cache: SharedFederatedKeyCache,
    cache_ttl: Duration,
    hub_signer: Option<Arc<dyn CanonicalSigner>>,
    /// Late-bound read model for same-realm User principal keys. Daemon boot
    /// constructs the Axon LocalRuntime before the transport layer derives the
    /// trust-anchor-backed lifecycle store path, so the resolver owns the
    /// stable provider graph and the transport installs this read model once
    /// the path is known. Runtime key admission and `federation.resolve_key`
    /// therefore consume the same PrincipalLifecycle aggregate.
    principal_lifecycle: Arc<RwLock<Option<PrincipalLifecycleReader>>>,
}

enum LocalKeyResolutionError {
    Missing,
    InvalidAuthority(AxonError),
}

impl FederatedKeyResolver {
    /// Construct a resolver that always tries the local trust
    /// anchor first, then federation. The federation client is
    /// optional — daemons booted without one (device mode, or
    /// hub-mode pre-PR-N1-commit-6) get a local-only resolver
    /// equivalent to PR-7's `TrustAnchorKeyResolver`.
    #[must_use]
    pub fn new(
        trust_anchor: SharedTrustAnchor,
        federation_client: Option<Arc<dyn FederationClient>>,
        federated_peers: SharedFederatedPeers,
        self_realm: Option<String>,
    ) -> Self {
        Self {
            trust_anchor,
            federation_client,
            federated_peers,
            self_realm,
            presented_pubkey_b64: None,
            cache: SharedFederatedKeyCache::new(),
            cache_ttl: DEFAULT_FEDERATED_RESOLVE_CACHE_TTL,
            hub_signer: None,
            principal_lifecycle: Arc::new(RwLock::new(None)),
        }
    }

    /// Override the federated-resolve cache TTL. Tests use a
    /// short TTL to exercise the expiry path without sleeping
    /// 5 minutes; production paths use the default (300s).
    #[must_use]
    pub fn with_cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }

    /// Use the supplied `SharedFederatedKeyCache` instead of
    /// building a fresh one. AdmissionFacade calls this so every
    /// per-admission resolver shares one in-process cache; the
    /// SIGHUP handler at boot scope holds a clone too so a
    /// trust-anchor reload can flush all cached entries
    /// atomically.
    #[must_use]
    pub fn with_cache(mut self, cache: SharedFederatedKeyCache) -> Self {
        self.cache = cache;
        self
    }

    /// Attach the public key presented by the envelope currently being
    /// admitted. Empty input remains absent on the wire; the
    /// `ResolveKeyRequest` DTO owns the request projection.
    #[must_use]
    pub fn with_presented_pubkey_b64(mut self, presented_pubkey_b64: impl Into<String>) -> Self {
        let trimmed = presented_pubkey_b64.into().trim().to_string();
        if !trimmed.is_empty() {
            self.presented_pubkey_b64 = Some(trimmed);
        }
        self
    }

    #[must_use]
    pub fn request_scoped_with_presented_pubkey_b64(
        &self,
        presented_pubkey_b64: impl Into<String>,
    ) -> Self {
        self.clone().with_presented_pubkey_b64(presented_pubkey_b64)
    }

    /// Attach the owner-bound signing capability used for outbound cross-hub
    /// `federation.resolve_key` requests. The resolver never receives or
    /// derives private key material.
    #[must_use]
    pub fn with_hub_signer(mut self, signer: Arc<dyn CanonicalSigner>) -> Self {
        self.hub_signer = Some(signer);
        self
    }

    pub(crate) fn attach_principal_lifecycle_reader(&self, reader: PrincipalLifecycleReader) {
        match self.principal_lifecycle.write() {
            Ok(mut guard) => {
                *guard = Some(reader);
            }
            Err(poisoned) => {
                *poisoned.into_inner() = Some(reader);
            }
        }
    }

    /// Drop every cached entry. Re-exported on the resolver for
    /// call-site convenience; identical to
    /// `self.cache.clone().flush()`.
    pub fn flush_cache(&self) {
        self.cache.flush();
    }

    #[must_use]
    pub fn is_configured_federated_caller(&self, caller_ura: &str) -> bool {
        if self.federation_client.is_none() {
            return false;
        }
        let Some(caller_realm) = crate::core::ura::realm_from_ura(caller_ura) else {
            return false;
        };
        if self.self_realm.as_deref() == Some(caller_realm.as_str()) {
            return false;
        }
        self.federated_peers.snapshot().contains_key(&caller_realm)
    }

    /// Test-only: total cached entries.
    #[cfg(test)]
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Lookup `agent_ura` in the cache. Returns the cached key
    /// only if it has not expired; expired entries are removed
    /// inline so the next caller misses cleanly. Mutex held
    /// for one HashMap operation; never across the cross-hub
    /// dial.
    fn cache_lookup(&self, agent_ura: &str) -> Option<VerifyingKey> {
        let cache_key = self.cache_key(agent_ura);
        let mut guard = match self.cache.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        match guard.get(&cache_key) {
            Some(entry) if entry.expires_at > Instant::now() => Some(entry.key),
            Some(_expired) => {
                guard.remove(&cache_key);
                None
            }
            None => None,
        }
    }

    /// Insert a freshly-resolved key into the cache with the
    /// configured TTL. Subsequent lookups on the same URA inside
    /// the window short-circuit before any cross-hub dial.
    fn cache_insert(&self, agent_ura: &str, key: VerifyingKey) {
        let cache_key = self.cache_key(agent_ura);
        let mut guard = match self.cache.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.insert(
            cache_key,
            CachedKey {
                key,
                expires_at: Instant::now() + self.cache_ttl,
            },
        );
    }

    fn cache_key(&self, agent_ura: &str) -> String {
        // `\x1f` (ASCII unit separator) joins the URA and pubkey. It
        // cannot appear in either an `easynet://` URA or standard
        // base64, so no `(ura, pk)` pair can collide with a different
        // pair under naive concatenation — which is the whole point of
        // keying by pubkey: one 1:N user URA must not share a cache
        // slot across its registered device keys.
        match self.presented_pubkey_b64.as_deref() {
            Some(pk) => format!("{agent_ura}\x1f{pk}"),
            None => agent_ura.to_string(),
        }
    }

    /// Local-first lookup. Mirrors `TrustAnchorKeyResolver` shape
    /// so existing single-realm setups behave identically.
    fn resolve_local(&self, agent_ura: &str) -> Result<VerifyingKey, LocalKeyResolutionError> {
        let trust_anchor = self.trust_anchor.snapshot();
        if let Some(entry) = match self.presented_pubkey_b64.as_deref() {
            Some(pk) => trust_anchor.lookup_user_by_pubkey(agent_ura, pk),
            None => trust_anchor.lookup(agent_ura),
        } {
            return Self::decode_local_public_key_b64(
                agent_ura,
                &entry.public_key_b64,
                "local_trust_anchor",
            );
        }
        if let Some(key) = self.resolve_principal_lifecycle_local_key(agent_ura)? {
            return Ok(key);
        }
        Err(LocalKeyResolutionError::Missing)
    }

    fn resolve_principal_lifecycle_local_key(
        &self,
        agent_ura: &str,
    ) -> Result<Option<VerifyingKey>, LocalKeyResolutionError> {
        let Some(self_realm) = self.self_realm.as_deref() else {
            return Ok(None);
        };
        let Ok(agent) = parse_ura(agent_ura) else {
            return Ok(None);
        };
        if agent.kind != URAKind::User || agent.realm != self_realm {
            return Ok(None);
        }
        let reader = match self.principal_lifecycle.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };
        let Some(reader) = reader else {
            return Ok(None);
        };
        let public_key_b64 = reader
            .active_public_keys_b64(agent_ura, self.presented_pubkey_b64.as_deref())
            .map_err(|status| {
                LocalKeyResolutionError::InvalidAuthority(
                    caller_key_not_found(
                        agent_ura,
                        &format!("principal_lifecycle_read_failed:{}", status.message()),
                    )
                    .with_context("authority_source", "principal_lifecycle"),
                )
            })?
            .into_iter()
            .next();
        let Some(public_key_b64) = public_key_b64 else {
            return Ok(None);
        };
        crate::op_event!(
            component = daemon_invocation,
            kind = principal_lifecycle_resolve_key_succeeded,
            agent_ura = agent_ura,
        );
        Self::decode_local_public_key_b64(agent_ura, &public_key_b64, "principal_lifecycle")
            .map(Some)
    }

    fn decode_local_public_key_b64(
        agent_ura: &str,
        public_key_b64: &str,
        authority_source: &'static str,
    ) -> Result<VerifyingKey, LocalKeyResolutionError> {
        let invalid_authority = |detail: String| {
            LocalKeyResolutionError::InvalidAuthority(
                caller_key_not_found(agent_ura, detail.as_str())
                    .with_context("authority_source", authority_source),
            )
        };
        let raw = BASE64_STANDARD
            .decode(public_key_b64)
            .map_err(|error| invalid_authority(format!("public_key_b64_decode_failed:{error}")))?;
        let arr: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
            invalid_authority(format!(
                "public_key_wrong_length:expected_32_got_{}",
                raw.len()
            ))
        })?;
        VerifyingKey::from_bytes(&arr)
            .map_err(|error| invalid_authority(format!("public_key_parse_failed:{error}")))
    }

    /// Cross-realm fall-through. Decision tree per spec §commit 2/N:
    ///
    /// - `caller realm == self_realm` → local-only; do NOT dial
    ///   federated. Local miss is final.
    /// - operator did NOT mark the caller realm as federated (no
    ///   `[[trusted_agent]] origin_realm = "<realm>"` entry
    ///   in `realm-trust.toml`) → local-only.
    /// - `federated_peers` map has no entry mapping
    ///   `caller realm → hub_endpoint` → cannot dial; return
    ///   CALLER_KEY_NOT_FOUND.
    /// - dial fails → CALLER_KEY_NOT_FOUND (INV-4 fail-closed).
    ///
    /// Returns `Ok(VerifyingKey)` only when the cross-hub resolve
    /// returns a valid base64 Ed25519 pubkey for the caller.
    fn resolve_federated(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        // Cache short-circuit. A hot signed-call path with the
        // same caller URA repeated within the TTL window skips
        // the cross-hub dial entirely. Cache-miss paths
        // (expired, never-resolved, post-flush) fall through to
        // a real dial. Cache failure modes are NEVER considered
        // — `CALLER_KEY_NOT_FOUND` flows from the federated dial
        // chain itself, not from the cache; we never cache a
        // negative result so a transient peer-hub outage cannot
        // poison the cache.
        if let Some(cached) = self.cache_lookup(agent_ura) {
            return Ok(cached);
        }

        let Some(client) = self.federation_client.as_ref() else {
            return Err(caller_key_not_found(agent_ura, "no_federation_client"));
        };

        let caller_realm = crate::core::ura::realm_from_ura(agent_ura)
            .ok_or_else(|| caller_key_not_found(agent_ura, "malformed_ura"))?;

        // INV-1 federated trust gate: same-realm caller's local
        // miss is final. Returning CALLER_KEY_NOT_FOUND here is the
        // same surface as a normal trust-anchor miss for a local
        // URA — the admission gate emits
        // CALLER_KEY_NOT_FOUND, which is the right
        // operator signal ("the URA is not trusted in this
        // realm").
        if let Some(self_realm) = self.self_realm.as_deref() {
            if caller_realm == self_realm {
                return Err(caller_key_not_found(agent_ura, "same_realm_local_miss"));
            }
        }

        // INV-1 second clause: the operator must have explicitly
        // marked the caller's realm as federated. This requires
        // EITHER a `[[trusted_agent]]` entry whose
        // `origin_realm` matches the caller's realm (the
        // canonical schema-B path) OR a `[daemon.federated_peers]`
        // entry for that realm. Both signal "I trust this
        // realm's hub". We accept both because the operator
        // workflow may have populated only one (e.g. via
        // `easynet join` auto-wire, which writes to
        // federated_peers but not to realm-trust.toml).
        let trust_entry_marked = self
            .trust_anchor
            .snapshot()
            .entries_sorted()
            .into_iter()
            .any(|e| e.origin_realm.as_deref() == Some(caller_realm.as_str()));
        let peers = self.federated_peers.snapshot();
        let peer_entry = peers.get(&caller_realm);
        if !trust_entry_marked && peer_entry.is_none() {
            return Err(caller_key_not_found(agent_ura, "realm_not_federated"));
        }

        let Some(peer_hub_endpoint) = peer_entry else {
            return Err(caller_key_not_found(agent_ura, "no_hub_endpoint_for_realm"));
        };

        let mut resolve_key_request =
            crate::daemon::federation::wire_contract::ResolveKeyRequest::new(agent_ura);
        if let Some(presented_pubkey_b64) = self.presented_pubkey_b64.as_deref() {
            resolve_key_request =
                resolve_key_request.with_presented_pubkey_b64(presented_pubkey_b64);
        }
        let args_bytes = resolve_key_request.to_arguments_bytes().map_err(|e| {
            AxonError::new(AxonErrorKind::Internal)
                .with_reason("resolve_key_args_encode")
                .with_message(format!("agent_ura:{agent_ura}:{e}"))
        })?;
        let Some(self_realm) = self.self_realm.as_deref() else {
            return Err(caller_key_not_found(agent_ura, "missing_self_realm"));
        };
        let peer_hub_ura = crate::core::ura::hub_ura(&caller_realm);
        let ability =
            crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY;
        let subject_ura =
            crate::core::ura::owner_ability_ura(&peer_hub_ura, ability).ok_or_else(|| {
                AxonError::new(AxonErrorKind::Internal)
                    .with_reason("resolve_key_subject_build")
                    .with_message(format!("peer_hub_ura:{peer_hub_ura}:ability:{ability}"))
            })?;
        let request_builder = PeerInvokeRequest::new(
            PeerInvocationSubject::ExplicitSubject(&subject_ura),
            &subject_ura,
            ability,
            args_bytes,
            Some(self_realm),
            self.hub_signer.as_deref(),
        );
        // `KeyResolver` is a synchronous Axon port. Enter Tokio's explicit
        // blocking region before awaiting the async signer so key-service UDS
        // I/O remains on `spawn_blocking`, never on an admission worker.
        let request = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async { request_builder.into_invoke_request().await })
        })
        .map_err(|status| {
            AxonError::new(AxonErrorKind::Internal)
                .with_reason("resolve_key_peer_request_build")
                .with_message(format!(
                    "agent_ura:{agent_ura}:code={:?}:{}",
                    status.code(),
                    status.message()
                ))
        })?;

        // Bridge sync trait → async tonic call.
        let target_hub_endpoint = peer_hub_endpoint.clone();
        let client_clone = Arc::clone(client);
        let response = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { client_clone.invoke(&target_hub_endpoint, request).await })
        })
        .map_err(|err| caller_key_not_found(agent_ura, &format!("dial_failed:{err}")))?;

        let parsed: serde_json::Value = serde_json::from_slice(&response.result).map_err(|e| {
            caller_key_not_found(agent_ura, &format!("resolve_key_response_parse:{e}"))
        })?;
        let pk_b64 = parsed
            .get("public_key_b64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                caller_key_not_found(agent_ura, "resolve_key_response_missing_pubkey")
            })?;
        let raw = BASE64_STANDARD.decode(pk_b64).map_err(|e| {
            caller_key_not_found(agent_ura, &format!("resolve_key_pubkey_b64_decode:{e}"))
        })?;
        let arr: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
            caller_key_not_found(
                agent_ura,
                &format!("resolve_key_pubkey_wrong_length:{}", raw.len()),
            )
        })?;
        let verifying_key = VerifyingKey::from_bytes(&arr).map_err(|e| {
            caller_key_not_found(agent_ura, &format!("resolve_key_pubkey_parse:{e}"))
        })?;
        // Fail-closed pin check. When we forwarded a
        // `presented_pubkey_b64`, the peer hub's job is to confirm that
        // exact key is registered under the caller URA and echo it
        // back; the key it returns MUST byte-equal the one we pinned.
        // A divergent key means either a misbehaving peer or a 1:N user
        // URA the peer disambiguated differently than the envelope —
        // both unsafe to admit, so we reject rather than trust the
        // peer's substitution. Compared on decoded bytes (not the
        // base64 strings) so padding / encoding variance never masks a
        // real mismatch.
        if let Some(pinned) = self.presented_pubkey_b64.as_deref() {
            let pinned_bytes = BASE64_STANDARD.decode(pinned).map_err(|e| {
                caller_key_not_found(agent_ura, &format!("presented_pubkey_b64_decode:{e}"))
            })?;
            if pinned_bytes != arr {
                return Err(caller_key_not_found(
                    agent_ura,
                    "resolve_key_response_pubkey_mismatch",
                ));
            }
        }
        // Cache success only. A failed dial / parse leaves the
        // cache untouched so a recoverable peer-hub outage does
        // not poison resolution for the configured TTL.
        self.cache_insert(agent_ura, verifying_key);
        Ok(verifying_key)
    }
}

impl KeyResolver for FederatedKeyResolver {
    fn resolve(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        // Local-first per INV-2.
        match self.resolve_local(agent_ura) {
            Ok(key) => Ok(key),
            Err(LocalKeyResolutionError::Missing) => self.resolve_federated(agent_ura),
            Err(LocalKeyResolutionError::InvalidAuthority(error)) => Err(error),
        }
    }
}

/// Wrap a federated-resolve failure as a caller key-resolution
/// failure. This is deliberately distinct from
/// `CALLER_SIGNATURE_INVALID`: a missing public key and a bad
/// signature require different operator action.
fn caller_key_not_found(agent_ura: &str, detail: &str) -> AxonError {
    AxonError::new(AxonErrorKind::InvalidArgument)
        .with_code(ErrorCode::CallerKeyNotFound)
        .with_stage(ErrorStage::CallerAuthentication)
        .with_security_class(SecurityClass::Identity)
        .with_message(format!("agent_ura:{agent_ura}:{detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::identity::self_identity::TestCanonicalSigner;
    use crate::daemon::trust::anchor::{RealmTrustAnchor, TrustedAgent, TrustedAgentRole};
    use ed25519_dalek::SigningKey;
    use serde_json::json;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn test_resolver(
        trust_anchor: Arc<RealmTrustAnchor>,
        federation_client: Option<Arc<dyn FederationClient>>,
        federated_peers: Arc<BTreeMap<String, String>>,
        self_realm: Option<String>,
    ) -> FederatedKeyResolver {
        FederatedKeyResolver::new(
            SharedTrustAnchor::new(trust_anchor),
            federation_client,
            SharedFederatedPeers::new(federated_peers.as_ref().clone()),
            self_realm,
        )
    }

    fn test_hub_signer(realm: &str) -> Arc<dyn CanonicalSigner> {
        Arc::new(TestCanonicalSigner::new(
            crate::core::ura::hub_ura(realm),
            [0x31; 32],
        ))
    }

    fn ed25519_pubkey_b64() -> (SigningKey, String) {
        // Deterministic test key (zero seed). Acceptable because
        // these tests verify resolution wire shape, not
        // cryptographic strength.
        let signing = SigningKey::from_bytes(&[1u8; 32]);
        let pk_bytes = signing.verifying_key().to_bytes();
        (signing, BASE64_STANDARD.encode(pk_bytes))
    }

    fn local_entry(ura: &str, pk_b64: &str) -> TrustedAgent {
        TrustedAgent {
            agent_ura: ura.to_string(),
            public_key_b64: pk_b64.to_string(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }
    }

    fn user_entry(ura: &str, pk_b64: &str) -> TrustedAgent {
        TrustedAgent {
            agent_ura: ura.to_string(),
            public_key_b64: pk_b64.to_string(),
            role: TrustedAgentRole::User,
            added_at_unix_ms: 1_700_000_000_000,
            origin_realm: None,
            hub_endpoint: None,
            tls_ca_pem_path: None,
        }
    }

    /// Test-only `FederationClient` that returns a canned JSON
    /// response. The dispatcher under test calls
    /// `federation.resolve_key` against this client and parses
    /// the result the same way it parses a real peer response.
    struct CannedFederationClient {
        canned_response: Vec<u8>,
    }

    #[async_trait::async_trait]
    impl FederationClient for CannedFederationClient {
        async fn invoke(
            &self,
            _target_hub_endpoint: &crate::daemon::federation::client::HubEndpoint,
            _request: InvokeRequest,
        ) -> Result<
            axon_sdk::pb::axon::v1::InvokeResponse,
            crate::daemon::federation::client::FederationClientError,
        > {
            Ok(axon_sdk::pb::axon::v1::InvokeResponse {
                result: self.canned_response.clone(),
                ..Default::default()
            })
        }
    }

    /// Test-only client that always errors out, simulating a
    /// peer hub that's down or breaker-open.
    struct DialFailedClient;

    #[async_trait::async_trait]
    impl FederationClient for DialFailedClient {
        async fn invoke(
            &self,
            target_hub_endpoint: &crate::daemon::federation::client::HubEndpoint,
            _request: InvokeRequest,
        ) -> Result<
            axon_sdk::pb::axon::v1::InvokeResponse,
            crate::daemon::federation::client::FederationClientError,
        > {
            Err(
                crate::daemon::federation::client::FederationClientError::DialFailed {
                    endpoint: target_hub_endpoint.clone(),
                    detail: "test-injected failure".to_string(),
                },
            )
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_hit_short_circuits_before_federated_dial() {
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let local_ura = "easynet:///r/realm-a/device/local-device";
        let anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![local_entry(local_ura, &pk_b64)]).unwrap(),
        );

        // Wire a dial-failed client; if the resolver tried to
        // dial we'd get an error. The local-first short-circuit
        // means the dial must NOT happen.
        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = test_resolver(
            anchor,
            Some(client),
            Arc::new(BTreeMap::new()),
            Some("realm-a".to_string()),
        );

        let key = resolver.resolve(local_ura).expect("local hit");
        assert_eq!(key.to_bytes().len(), 32);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_resolver_observes_live_trust_and_peer_replacements() {
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let local_ura = "easynet:///r/realm-a/device/local-device";
        let cross_ura = "easynet:///r/realm-b/device/peer-device";
        let trust = SharedTrustAnchor::new(Arc::new(RealmTrustAnchor::default()));
        let peers = SharedFederatedPeers::default();
        let response_json = serde_json::json!({ "public_key_b64": pk_b64 });
        let client: Arc<dyn FederationClient> = Arc::new(CannedFederationClient {
            canned_response: serde_json::to_vec(&response_json).unwrap(),
        });
        let resolver = FederatedKeyResolver::new(
            trust.clone(),
            Some(client),
            peers.clone(),
            Some("realm-a".to_string()),
        )
        .with_hub_signer(test_hub_signer("realm-a"));

        resolver
            .resolve(local_ura)
            .expect_err("unpublished local key must fail");
        trust.replace(Arc::new(
            RealmTrustAnchor::from_entries(vec![local_entry(local_ura, &pk_b64)]).unwrap(),
        ));
        resolver
            .resolve(local_ura)
            .expect("same resolver observes trust replacement");

        resolver
            .resolve(cross_ura)
            .expect_err("unpublished peer route must fail");
        peers.replace(BTreeMap::from([(
            "realm-b".to_string(),
            "https://hub-b:50443".to_string(),
        )]));
        resolver
            .resolve(cross_ura)
            .expect("same resolver observes peer replacement");
    }

    #[test]
    fn same_realm_principal_lifecycle_key_resolves_local_miss_without_dial() {
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let dir = tempdir().expect("tempdir");
        let user = "easynet:///r/realm-a/user/alice";
        let mut principals = serde_json::Map::new();
        principals.insert(
            user.to_string(),
            json!({
                "principal_ura": user,
                "state": "active",
                "version": 2,
                "bindings": [{
                    "binding_id": "bind-alice",
                    "principal_ura": user,
                    "key_id": "key-alice",
                    "public_key_b64": pk_b64,
                    "state": "active",
                    "created_unix_ms": 1
                }],
                "enrollment_proof": {
                    "kind": "bootstrap",
                    "reference": "proof:create"
                },
                "consumed_recovery_proofs": {},
                "enrollments": [],
                "grants": [],
                "created_unix_ms": 1,
                "updated_unix_ms": 1,
                "command_log": {"create": 1}
            }),
        );
        let store_path = dir.path().join("principal-lifecycle.json");
        std::fs::write(
            &store_path,
            serde_json::to_vec(&json!({ "principals": principals })).expect("store json"),
        )
        .expect("write lifecycle store");

        let resolver = test_resolver(
            Arc::new(RealmTrustAnchor::default()),
            Some(Arc::new(DialFailedClient)),
            Arc::new(BTreeMap::new()),
            Some("realm-a".to_string()),
        )
        .with_presented_pubkey_b64(pk_b64.clone());
        resolver.attach_principal_lifecycle_reader(PrincipalLifecycleReader::new(store_path));

        let resolved = resolver
            .resolve(user)
            .expect("same-realm User key should resolve from PrincipalLifecycle");
        assert_eq!(BASE64_STANDARD.encode(resolved.to_bytes()), pk_b64);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_realm_with_federated_peers_entry_resolves_via_dial() {
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let cross_ura = "easynet:///r/realm-b/device/peer-device";

        // Local trust anchor has NO entry for cross_ura.
        let anchor = Arc::new(RealmTrustAnchor::default());

        // Operator wired a federated_peers entry for realm-b.
        // This is the post-`easynet join` shape (Track 3).
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        // Peer hub returns the canned pubkey.
        let response_json = serde_json::json!({
            "public_key_b64": pk_b64,
        });
        let client: Arc<dyn FederationClient> = Arc::new(CannedFederationClient {
            canned_response: serde_json::to_vec(&response_json).unwrap(),
        });

        let resolver = test_resolver(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        )
        .with_hub_signer(test_hub_signer("realm-a"));

        let key = resolver.resolve(cross_ura).expect("federated hit");
        assert_eq!(key.to_bytes().len(), 32);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_realm_resolution_fails_closed_without_hub_signer() {
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let cross_ura = "easynet:///r/realm-b/device/peer-device";
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());
        let client: Arc<dyn FederationClient> = Arc::new(CannedFederationClient {
            canned_response: serde_json::to_vec(&serde_json::json!({
                "public_key_b64": pk_b64,
            }))
            .unwrap(),
        });
        let resolver = test_resolver(
            Arc::new(RealmTrustAnchor::default()),
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        );

        let error = resolver
            .resolve(cross_ura)
            .expect_err("cross-hub key resolution must require a hub signer");
        assert_eq!(error.reason, "resolve_key_peer_request_build");
        assert!(error.message.contains("configured hub signer"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_realm_without_federation_marker_returns_unknown() {
        let cross_ura = "easynet:///r/realm-b/device/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());

        // No federated_peers entry, no origin_realm-marked
        // trust entry. The resolver MUST NOT dial — operator did
        // not opt into cross-realm resolution for realm-b.
        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = test_resolver(
            anchor,
            Some(client),
            Arc::new(BTreeMap::new()),
            Some("realm-a".to_string()),
        );

        let err = resolver.resolve(cross_ura).expect_err("unmarked");
        assert!(
            err.reason == ErrorCode::CallerKeyNotFound.as_str(),
            "expected CALLER_KEY_NOT_FOUND, got {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_realm_dial_failure_surfaces_as_unknown() {
        let cross_ura = "easynet:///r/realm-b/device/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = test_resolver(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        )
        .with_hub_signer(test_hub_signer("realm-a"));

        // INV-4 fail-closed: dial failure → CALLER_KEY_NOT_FOUND,
        // NOT a silent local fall-through.
        let err = resolver.resolve(cross_ura).expect_err("dial fail");
        assert!(
            err.reason == ErrorCode::CallerKeyNotFound.as_str(),
            "expected CALLER_KEY_NOT_FOUND, got {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn same_realm_local_miss_does_not_dial_federated() {
        let same_realm_ura = "easynet:///r/realm-a/device/missing-device";
        let anchor = Arc::new(RealmTrustAnchor::default());

        // Federated peer wired, but the missing URA is in the
        // SAME realm as `self_realm` — INV-1 says local miss is
        // final, do not federate.
        let mut peers = BTreeMap::new();
        peers.insert("realm-a".to_string(), "https://hub-a:50443".to_string());

        // DialFailedClient ensures any accidental federated dial
        // would surface as a different error variant.
        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = test_resolver(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        )
        .with_hub_signer(test_hub_signer("realm-a"));

        let err = resolver
            .resolve(same_realm_ura)
            .expect_err("same-realm miss");
        let err_str = format!("{err:?}");
        assert_eq!(err.reason, ErrorCode::CallerKeyNotFound.as_str());
        // The dial must NOT have fired; the failure detail
        // should reflect a same-realm-local-miss, not
        // dial_failed.
        assert!(
            err_str.contains("same_realm_local_miss"),
            "expected same_realm_local_miss in detail, got {err_str}"
        );
    }

    // ── TTL cache (C3b) tests ──────────────────────────────────

    /// `CountingFederationClient` records every cross-hub dial
    /// and returns the same canned pubkey response. Lets the
    /// cache tests assert exactly how many real dials fired.
    struct CountingFederationClient {
        canned_response: Vec<u8>,
        dial_count: std::sync::atomic::AtomicUsize,
    }

    impl CountingFederationClient {
        fn new(canned_response: Vec<u8>) -> Self {
            Self {
                canned_response,
                dial_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        fn dials(&self) -> usize {
            self.dial_count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl FederationClient for CountingFederationClient {
        async fn invoke(
            &self,
            _target_hub_endpoint: &crate::daemon::federation::client::HubEndpoint,
            _request: InvokeRequest,
        ) -> Result<
            axon_sdk::pb::axon::v1::InvokeResponse,
            crate::daemon::federation::client::FederationClientError,
        > {
            self.dial_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(axon_sdk::pb::axon::v1::InvokeResponse {
                result: self.canned_response.clone(),
                ..Default::default()
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn invalid_local_authority_does_not_fall_through_to_federation() {
        let caller_ura = "easynet:///r/realm-b/device/local-corrupt";
        let anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![local_entry(caller_ura, "not-base64")])
                .expect("trust inventory accepts key material for admission-time validation"),
        );
        let (_, federated_key) = ed25519_pubkey_b64();
        let client = Arc::new(CountingFederationClient::new(
            serde_json::to_vec(&serde_json::json!({
                "public_key_b64": federated_key,
            }))
            .unwrap(),
        ));
        let federation_client: Arc<dyn FederationClient> = client.clone();
        let resolver = test_resolver(
            anchor,
            Some(federation_client),
            Arc::new(BTreeMap::from([(
                "realm-b".to_string(),
                "https://hub-b:50443".to_string(),
            )])),
            Some("realm-a".to_string()),
        )
        .with_hub_signer(test_hub_signer("realm-a"));

        let error = resolver
            .resolve(caller_ura)
            .expect_err("a corrupt local authority row must fail closed");
        assert_eq!(error.code, ErrorCode::CallerKeyNotFound);
        assert_eq!(
            error.context.get("authority_source").map(String::as_str),
            Some("local_trust_anchor")
        );
        assert_eq!(
            client.dials(),
            0,
            "an invalid claimed local authority must not be replaced by a federated key"
        );
    }

    struct EchoPresentedPubkeyClient {
        dial_count: std::sync::atomic::AtomicUsize,
        seen_args: Mutex<Vec<serde_json::Value>>,
    }

    impl EchoPresentedPubkeyClient {
        fn new() -> Self {
            Self {
                dial_count: std::sync::atomic::AtomicUsize::new(0),
                seen_args: Mutex::new(Vec::new()),
            }
        }

        fn dials(&self) -> usize {
            self.dial_count.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn seen_args(&self) -> Vec<serde_json::Value> {
            self.seen_args.lock().expect("seen args mutex").clone()
        }
    }

    #[async_trait::async_trait]
    impl FederationClient for EchoPresentedPubkeyClient {
        async fn invoke(
            &self,
            _target_hub_endpoint: &crate::daemon::federation::client::HubEndpoint,
            request: InvokeRequest,
        ) -> Result<
            axon_sdk::pb::axon::v1::InvokeResponse,
            crate::daemon::federation::client::FederationClientError,
        > {
            self.dial_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let args: serde_json::Value =
                serde_json::from_slice(&request.arguments).expect("resolve_key args JSON");
            self.seen_args
                .lock()
                .expect("seen args mutex")
                .push(args.clone());
            let pk_b64 = args
                .get("presented_pubkey_b64")
                .and_then(|v| v.as_str())
                .expect("presented pubkey must be forwarded");
            Ok(axon_sdk::pb::axon::v1::InvokeResponse {
                result: serde_json::to_vec(&serde_json::json!({
                    "public_key_b64": pk_b64,
                }))
                .unwrap(),
                ..Default::default()
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ttl_cache_hits_avoid_repeat_dial_within_window() {
        // Two consecutive resolves on the same URA within the
        // TTL window: second hits cache, peer hub is dialed
        // exactly once.
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let cross_ura = "easynet:///r/realm-b/device/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        let response_json = serde_json::json!({ "public_key_b64": pk_b64 });
        let counting = Arc::new(CountingFederationClient::new(
            serde_json::to_vec(&response_json).unwrap(),
        ));
        let client: Arc<dyn FederationClient> = counting.clone();

        let resolver = test_resolver(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        )
        .with_hub_signer(test_hub_signer("realm-a"));

        let k1 = resolver.resolve(cross_ura).expect("dial 1");
        let k2 = resolver.resolve(cross_ura).expect("cache hit");
        assert_eq!(k1.to_bytes(), k2.to_bytes());
        assert_eq!(counting.dials(), 1, "second resolve must hit cache");
        assert_eq!(resolver.cache_len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ttl_cache_expires_after_window_and_redials() {
        // Short TTL forces expiry; the second resolve sees an
        // expired entry, evicts it, and dials again.
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let cross_ura = "easynet:///r/realm-b/device/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        let response_json = serde_json::json!({ "public_key_b64": pk_b64 });
        let counting = Arc::new(CountingFederationClient::new(
            serde_json::to_vec(&response_json).unwrap(),
        ));
        let client: Arc<dyn FederationClient> = counting.clone();

        let resolver = test_resolver(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        )
        .with_hub_signer(test_hub_signer("realm-a"))
        .with_cache_ttl(Duration::from_millis(50));

        let _ = resolver.resolve(cross_ura).expect("dial 1");
        std::thread::sleep(Duration::from_millis(80));
        let _ = resolver.resolve(cross_ura).expect("dial 2 post-expiry");
        assert_eq!(counting.dials(), 2, "expired cache entry must redial");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flush_cache_clears_all_entries() {
        // Operator SIGHUP / key-rotation entry point. Simulate
        // by populating the cache, then calling flush_cache,
        // and asserting the next resolve hits the dial again.
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let cross_ura = "easynet:///r/realm-b/device/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        let response_json = serde_json::json!({ "public_key_b64": pk_b64 });
        let counting = Arc::new(CountingFederationClient::new(
            serde_json::to_vec(&response_json).unwrap(),
        ));
        let client: Arc<dyn FederationClient> = counting.clone();

        let resolver = test_resolver(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        )
        .with_hub_signer(test_hub_signer("realm-a"));

        let _ = resolver.resolve(cross_ura).expect("dial 1");
        assert_eq!(resolver.cache_len(), 1);
        resolver.flush_cache();
        assert_eq!(resolver.cache_len(), 0, "flush drops all entries");
        let _ = resolver.resolve(cross_ura).expect("dial 2 post-flush");
        assert_eq!(counting.dials(), 2, "post-flush resolve dials again");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dial_failure_does_not_poison_cache() {
        // A failed cross-hub dial must NOT cache the failure as
        // a negative entry — a recoverable peer-hub outage
        // would otherwise keep resolving as `CALLER_KEY_NOT_FOUND`
        // for the entire TTL even after the peer comes back.
        let cross_ura = "easynet:///r/realm-b/device/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = test_resolver(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        )
        .with_hub_signer(test_hub_signer("realm-a"));

        let _ = resolver.resolve(cross_ura).expect_err("dial fails");
        assert_eq!(
            resolver.cache_len(),
            0,
            "negative outcomes never poison the cache"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_user_resolution_pins_presented_pubkey_without_dial() {
        let (_a, pk_a) = ed25519_pubkey_b64();
        let key_b = SigningKey::from_bytes(&[2u8; 32]);
        let pk_b = BASE64_STANDARD.encode(key_b.verifying_key().to_bytes());
        let user_ura = "easynet:///r/realm-a/user/alice";
        let anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![
                user_entry(user_ura, &pk_a),
                user_entry(user_ura, &pk_b),
            ])
            .unwrap(),
        );

        let client = Arc::new(EchoPresentedPubkeyClient::new());
        let resolver = test_resolver(
            anchor,
            Some(client.clone()),
            Arc::new(BTreeMap::new()),
            Some("realm-a".to_string()),
        )
        .with_presented_pubkey_b64(pk_b.clone());

        let key = resolver.resolve(user_ura).expect("local user pin");
        assert_eq!(BASE64_STANDARD.encode(key.to_bytes()), pk_b);
        assert_eq!(client.dials(), 0, "same-realm local pin must not dial");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_realm_user_resolution_forwards_presented_pubkey_and_keys_cache_by_pubkey() {
        let (_a, pk_a) = ed25519_pubkey_b64();
        let key_b = SigningKey::from_bytes(&[2u8; 32]);
        let pk_b = BASE64_STANDARD.encode(key_b.verifying_key().to_bytes());
        let user_ura = "easynet:///r/realm-b/user/alice";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());
        let peers = Arc::new(peers);
        let cache = SharedFederatedKeyCache::new();
        let client = Arc::new(EchoPresentedPubkeyClient::new());

        let client_dyn: Arc<dyn FederationClient> = client.clone();
        let key_a = test_resolver(
            Arc::clone(&anchor),
            Some(client_dyn),
            Arc::clone(&peers),
            Some("realm-a".to_string()),
        )
        .with_hub_signer(test_hub_signer("realm-a"))
        .with_cache(cache.clone())
        .with_presented_pubkey_b64(pk_a.clone())
        .resolve(user_ura)
        .expect("federated user key a");

        let client_dyn: Arc<dyn FederationClient> = client.clone();
        let key_b = test_resolver(anchor, Some(client_dyn), peers, Some("realm-a".to_string()))
            .with_hub_signer(test_hub_signer("realm-a"))
            .with_cache(cache)
            .with_presented_pubkey_b64(pk_b.clone())
            .resolve(user_ura)
            .expect("federated user key b");

        assert_eq!(BASE64_STANDARD.encode(key_a.to_bytes()), pk_a);
        assert_eq!(BASE64_STANDARD.encode(key_b.to_bytes()), pk_b);
        assert_ne!(key_a.to_bytes(), key_b.to_bytes());
        assert_eq!(
            client.dials(),
            2,
            "same user URA with two presented keys must not share one cache entry"
        );
        let seen = client.seen_args();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["agent_ura"], user_ura);
        assert_eq!(seen[0]["presented_pubkey_b64"], pk_a);
        assert_eq!(seen[1]["agent_ura"], user_ura);
        assert_eq!(seen[1]["presented_pubkey_b64"], pk_b);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_realm_rejects_peer_key_that_diverges_from_presented_pin() {
        // A peer hub that returns a key other than the one we pinned is
        // either misbehaving or disambiguating a 1:N user URA to a
        // different device than the envelope signed with. Either way the
        // resolve must fail closed and never cache the substituted key.
        let (_a, pk_a) = ed25519_pubkey_b64();
        let key_b = SigningKey::from_bytes(&[2u8; 32]);
        let pk_b = BASE64_STANDARD.encode(key_b.verifying_key().to_bytes());
        let user_ura = "easynet:///r/realm-b/user/alice";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        // Peer echoes key_a regardless of what we present.
        let client: Arc<dyn FederationClient> = Arc::new(CannedFederationClient {
            canned_response: serde_json::to_vec(&serde_json::json!({
                "public_key_b64": pk_a,
            }))
            .unwrap(),
        });
        let resolver = test_resolver(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        )
        .with_hub_signer(test_hub_signer("realm-a"))
        // We present key_b; the peer's key_a must be rejected.
        .with_presented_pubkey_b64(pk_b);

        let err = resolver
            .resolve(user_ura)
            .expect_err("divergent peer key must fail closed");
        assert!(
            err.message.contains("resolve_key_response_pubkey_mismatch"),
            "expected pin-mismatch reject; got: {}",
            err.message
        );
        assert_eq!(
            resolver.cache_len(),
            0,
            "a rejected substitution must never poison the cache"
        );
    }
}
