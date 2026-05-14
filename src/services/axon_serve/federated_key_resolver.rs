// EasyNet CLI — FederatedKeyResolver (PR-N2 commit 1/N)
// =======================================================
//
// File: src/services/axon_serve/federated_key_resolver.rs
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
// at hub B) hit `unknown_agent_ura` and admission rejects with
// `caller_signature_invalid` regardless of whether the
// signature is correct, because hub B has no way to fetch
// hub A's pubkey for verification.
//
// PR-N2 closes that gap. FederatedKeyResolver:
//
//   1. Tries the local trust anchor first (INV-2 local-first).
//      Same-realm callers short-circuit before any network I/O.
//   2. On local miss, parses the caller URI's tenant and checks
//      the `realm-trust.toml` for a `[[trusted_agent]]` entry
//      whose `origin_tenant_id` matches that tenant (INV-1
//      federated trust gate). Operators explicitly opt into
//      cross-realm verification by adding such entries.
//   3. Calls `federation.resolve_key` on the peer hub via the
//      PR-N1 CrossHubDialer, returning the peer's pubkey for
//      the caller URI (INV-3 same Ed25519 4-step verify).
//   4. Dial failure surfaces as `unknown_agent_ura` so the
//      admission gate's reject path runs identically to a
//      local trust miss (INV-4 fail-closed).
//
// Sync trait shape
// ----------------
// `easynet_axon::invocation::axiom::KeyResolver::resolve` is a
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

#![cfg(feature = "axon-pb")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use easynet_axon::invocation::axiom::KeyResolver;
use easynet_axon::invocation::{AxonError, AxonErrorKind};
use ed25519_dalek::VerifyingKey;

#[cfg(test)]
use crate::pb::axon::v1::InvokeRequest;
use crate::services::federation_client::FederationClient;
use crate::services::realm_trust_anchor::RealmTrustAnchor;

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
}

/// Resolves an `agent_ura` to its Ed25519 verifying key, falling
/// through to a federated lookup when the local trust anchor has
/// no entry for the URI and the caller's tenant is one the
/// operator has marked as federated via DEC-N1 schema-B
/// `origin_tenant_id` on a `[[trusted_agent]]` entry.
pub struct FederatedKeyResolver {
    trust_anchor: Arc<RealmTrustAnchor>,
    federation_client: Option<Arc<dyn FederationClient>>,
    federated_peers: Arc<std::collections::BTreeMap<String, String>>,
    self_realm: Option<String>,
    /// 5-min TTL cache on cross-hub `federation.resolve_key`
    /// outcomes. Keyed by full `agent_ura`. Operators flush on
    /// trust-anchor SIGHUP via [`SharedFederatedKeyCache::flush`]
    /// so a key rotation propagates without a daemon restart.
    /// The mutex is held for the duration of one HashMap lookup
    /// / insert / drain — never across the cross-hub dial
    /// itself, so concurrent first-time resolves on disjoint
    /// URIs never serialize.
    ///
    /// The cache lives at AdmissionFacade scope (passed in via
    /// `with_cache`) so the per-admission-call resolver
    /// instance inherits the same in-process state. Without
    /// this share, the cache would reset to empty on every
    /// admission call and never deliver any savings.
    cache: SharedFederatedKeyCache,
    cache_ttl: Duration,
}

impl FederatedKeyResolver {
    /// Construct a resolver that always tries the local trust
    /// anchor first, then federation. The federation client is
    /// optional — daemons booted without one (device mode, or
    /// hub-mode pre-PR-N1-commit-6) get a local-only resolver
    /// equivalent to PR-7's `TrustAnchorKeyResolver`.
    #[must_use]
    pub fn new(
        trust_anchor: Arc<RealmTrustAnchor>,
        federation_client: Option<Arc<dyn FederationClient>>,
        federated_peers: Arc<std::collections::BTreeMap<String, String>>,
        self_realm: Option<String>,
    ) -> Self {
        Self {
            trust_anchor,
            federation_client,
            federated_peers,
            self_realm,
            cache: SharedFederatedKeyCache::new(),
            cache_ttl: DEFAULT_FEDERATED_RESOLVE_CACHE_TTL,
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

    /// Drop every cached entry. Re-exported on the resolver for
    /// call-site convenience; identical to
    /// `self.cache.clone().flush()`.
    pub fn flush_cache(&self) {
        self.cache.flush();
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
        let mut guard = match self.cache.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        match guard.get(agent_ura) {
            Some(entry) if entry.expires_at > Instant::now() => Some(entry.key),
            Some(_expired) => {
                guard.remove(agent_ura);
                None
            }
            None => None,
        }
    }

    /// Insert a freshly-resolved key into the cache with the
    /// configured TTL. Subsequent lookups on the same URI inside
    /// the window short-circuit before any cross-hub dial.
    fn cache_insert(&self, agent_ura: &str, key: VerifyingKey) {
        let mut guard = match self.cache.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.insert(
            agent_ura.to_string(),
            CachedKey {
                key,
                expires_at: Instant::now() + self.cache_ttl,
            },
        );
    }

    /// Local-first lookup. Mirrors `TrustAnchorKeyResolver` shape
    /// so existing single-realm setups behave identically.
    fn resolve_local(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        let entry = self.trust_anchor.lookup(agent_ura).ok_or_else(|| {
            AxonError::new(AxonErrorKind::InvalidArgument)
                .with_reason("unknown_agent_ura")
                .with_message(format!("agent_ura:{agent_ura}"))
        })?;
        let raw = BASE64_STANDARD.decode(&entry.public_key_b64).map_err(|e| {
            AxonError::new(AxonErrorKind::InvalidArgument)
                .with_reason("public_key_b64_decode_failed")
                .with_message(format!("agent_ura:{agent_ura}:{e}"))
        })?;
        let arr: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
            AxonError::new(AxonErrorKind::InvalidArgument)
                .with_reason("public_key_wrong_length")
                .with_message(format!(
                    "agent_ura:{agent_ura}:expected_32_got_{}",
                    raw.len()
                ))
        })?;
        VerifyingKey::from_bytes(&arr).map_err(|e| {
            AxonError::new(AxonErrorKind::InvalidArgument)
                .with_reason("public_key_parse_failed")
                .with_message(format!("agent_ura:{agent_ura}:{e}"))
        })
    }

    /// Cross-realm fall-through. Decision tree per spec §commit 2/N:
    ///
    /// - `caller_tenant == self_realm` → local-only; do NOT dial
    ///   federated. Local miss is final.
    /// - operator did NOT mark caller_tenant as federated (no
    ///   `[[trusted_agent]] origin_tenant_id = "<tenant>"` entry
    ///   in `realm-trust.toml`) → local-only.
    /// - `federated_peers` map has no entry mapping
    ///   `caller_tenant → hub_uri` → cannot dial; return
    ///   unknown_agent_ura.
    /// - dial fails → unknown_agent_ura (INV-4 fail-closed).
    ///
    /// Returns `Ok(VerifyingKey)` only when the cross-hub resolve
    /// returns a valid base64 Ed25519 pubkey for the caller.
    fn resolve_federated(&self, agent_ura: &str) -> Result<VerifyingKey, AxonError> {
        // Cache short-circuit. A hot signed-call path with the
        // same caller URI repeated within the TTL window skips
        // the cross-hub dial entirely. Cache-miss paths
        // (expired, never-resolved, post-flush) fall through to
        // a real dial. Cache failure modes are NEVER considered
        // — `unknown_agent_ura` flows from the federated dial
        // chain itself, not from the cache; we never cache a
        // negative result so a transient peer-hub outage cannot
        // poison the cache.
        if let Some(cached) = self.cache_lookup(agent_ura) {
            return Ok(cached);
        }

        let Some(client) = self.federation_client.as_ref() else {
            return Err(unknown_agent_ura(agent_ura, "no_federation_client"));
        };

        let caller_tenant =
            crate::services::axon_serve::daemon_invocation_service::parse_tenant_from_uri(
                agent_ura,
            )
            .ok_or_else(|| unknown_agent_ura(agent_ura, "malformed_uri"))?;

        // INV-1 federated trust gate: same-realm caller's local
        // miss is final. Returning unknown_agent_ura here is the
        // same surface as a normal trust-anchor miss for a local
        // URI — the admission gate emits
        // AXON_CALLER_SIGNATURE_INVALID, which is the right
        // operator signal ("the URI is not trusted in this
        // realm").
        if let Some(self_realm) = self.self_realm.as_deref() {
            if caller_tenant == self_realm {
                return Err(unknown_agent_ura(agent_ura, "same_realm_local_miss"));
            }
        }

        // INV-1 second clause: the operator must have explicitly
        // marked the caller's tenant as federated. This requires
        // EITHER a `[[trusted_agent]]` entry whose
        // `origin_tenant_id` matches the caller's tenant (the
        // canonical schema-B path) OR a `[daemon.federated_peers]`
        // entry for that tenant. Both signal "I trust this
        // tenant's hub". We accept both because the operator
        // workflow may have populated only one (e.g. via
        // `easynet join` auto-wire, which writes to
        // federated_peers but not to realm-trust.toml).
        let trust_entry_marked = self
            .trust_anchor
            .entries_sorted()
            .into_iter()
            .any(|e| e.origin_tenant_id.as_deref() == Some(caller_tenant.as_str()));
        let peer_entry = self.federated_peers.get(&caller_tenant);
        if !trust_entry_marked && peer_entry.is_none() {
            return Err(unknown_agent_ura(agent_ura, "tenant_not_federated"));
        }

        let Some(peer_hub_uri) = peer_entry else {
            return Err(unknown_agent_ura(agent_ura, "no_hub_uri_for_tenant"));
        };

        // Build the cross-hub `federation.resolve_key` request.
        // The peer-side ability handler is a thin RFC-002 wrap
        // around its local trust anchor; we forward the caller
        // URI verbatim and parse the response as a JSON
        // `{"public_key_b64": "<base64-32-bytes>"}` shape.
        let args = serde_json::json!({ "agent_ura": agent_ura });
        let args_bytes = serde_json::to_vec(&args).map_err(|e| {
            AxonError::new(AxonErrorKind::Internal)
                .with_reason("resolve_key_args_encode")
                .with_message(format!("agent_ura:{agent_ura}:{e}"))
        })?;
        let Some(self_realm) = self.self_realm.as_deref() else {
            return Err(unknown_agent_ura(agent_ura, "missing_self_realm"));
        };
        let local_hub_uri = crate::ura::hub_ura(self_realm);
        let request = crate::services::axon_serve::ProtoEnvelope::caller_only(local_hub_uri)
            .and_then(|env| {
                env.invoke_request(
                    crate::services::axon_serve::federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY,
                    args_bytes,
                )
            })
            .map_err(|e| {
                AxonError::new(AxonErrorKind::Internal)
                    .with_reason("resolve_key_envelope_build")
                    .with_message(format!("agent_ura:{agent_ura}:{e}"))
            })?;

        // Bridge sync trait → async tonic call.
        let target_hub = peer_hub_uri.clone();
        let client_clone = Arc::clone(client);
        let response = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async move { client_clone.forward_invoke(&target_hub, request).await })
        })
        .map_err(|err| unknown_agent_ura(agent_ura, &format!("dial_failed:{err}")))?;

        let parsed: serde_json::Value = serde_json::from_slice(&response.result).map_err(|e| {
            unknown_agent_ura(agent_ura, &format!("resolve_key_response_parse:{e}"))
        })?;
        let pk_b64 = parsed
            .get("public_key_b64")
            .and_then(|v| v.as_str())
            .ok_or_else(|| unknown_agent_ura(agent_ura, "resolve_key_response_missing_pubkey"))?;
        let raw = BASE64_STANDARD.decode(pk_b64).map_err(|e| {
            unknown_agent_ura(agent_ura, &format!("resolve_key_pubkey_b64_decode:{e}"))
        })?;
        let arr: [u8; 32] = raw.as_slice().try_into().map_err(|_| {
            unknown_agent_ura(
                agent_ura,
                &format!("resolve_key_pubkey_wrong_length:{}", raw.len()),
            )
        })?;
        let verifying_key = VerifyingKey::from_bytes(&arr)
            .map_err(|e| unknown_agent_ura(agent_ura, &format!("resolve_key_pubkey_parse:{e}")))?;
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
            Err(_) => self.resolve_federated(agent_ura),
        }
    }
}

/// Wrap a federated-resolve failure as the same wire-shape the
/// local trust-miss path emits, so the admission gate's reject
/// reason is `AXON_CALLER_SIGNATURE_INVALID` regardless of
/// whether the URI was unknown locally or unreachable cross-
/// realm. Operators reading the reject log see the failure
/// detail in the AxonError message field.
fn unknown_agent_ura(agent_ura: &str, detail: &str) -> AxonError {
    AxonError::new(AxonErrorKind::InvalidArgument)
        .with_reason("unknown_agent_ura")
        .with_message(format!("agent_ura:{agent_ura}:{detail}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::realm_trust_anchor::{TrustedAgent, TrustedAgentRole};
    use ed25519_dalek::SigningKey;
    use std::collections::BTreeMap;

    fn ed25519_pubkey_b64() -> (SigningKey, String) {
        // Deterministic test key (zero seed). Acceptable because
        // these tests verify resolution wire shape, not
        // cryptographic strength.
        let signing = SigningKey::from_bytes(&[1u8; 32]);
        let pk_bytes = signing.verifying_key().to_bytes();
        (signing, BASE64_STANDARD.encode(pk_bytes))
    }

    fn local_entry(uri: &str, pk_b64: &str) -> TrustedAgent {
        TrustedAgent {
            agent_ura: uri.to_string(),
            public_key_b64: pk_b64.to_string(),
            role: TrustedAgentRole::Device,
            added_at_unix_ms: 1_700_000_000_000,
            origin_tenant_id: None,
            hub_uri: None,
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
        async fn forward_invoke(
            &self,
            _target_hub: &crate::services::federation_client::HubUri,
            _request: InvokeRequest,
        ) -> Result<
            crate::pb::axon::v1::InvokeResponse,
            crate::services::federation_client::FederationClientError,
        > {
            Ok(crate::pb::axon::v1::InvokeResponse {
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
        async fn forward_invoke(
            &self,
            target_hub: &crate::services::federation_client::HubUri,
            _request: InvokeRequest,
        ) -> Result<
            crate::pb::axon::v1::InvokeResponse,
            crate::services::federation_client::FederationClientError,
        > {
            Err(
                crate::services::federation_client::FederationClientError::DialFailed {
                    hub: target_hub.clone(),
                    detail: "test-injected failure".to_string(),
                },
            )
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_hit_short_circuits_before_federated_dial() {
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let local_uri = "easynet:///r/realm-a/device/local-device";
        let anchor = Arc::new(
            RealmTrustAnchor::from_entries(vec![local_entry(local_uri, &pk_b64)]).unwrap(),
        );

        // Wire a dial-failed client; if the resolver tried to
        // dial we'd get an error. The local-first short-circuit
        // means the dial must NOT happen.
        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(BTreeMap::new()),
            Some("realm-a".to_string()),
        );

        let key = resolver.resolve(local_uri).expect("local hit");
        assert_eq!(key.to_bytes().len(), 32);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_realm_with_federated_peers_entry_resolves_via_dial() {
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let cross_uri = "easynet:///r/realm-b/device/peer-device";

        // Local trust anchor has NO entry for cross_uri.
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

        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        );

        let key = resolver.resolve(cross_uri).expect("federated hit");
        assert_eq!(key.to_bytes().len(), 32);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_realm_without_federation_marker_returns_unknown() {
        let cross_uri = "easynet:///r/realm-b/device/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());

        // No federated_peers entry, no origin_tenant_id-marked
        // trust entry. The resolver MUST NOT dial — operator did
        // not opt into cross-realm resolution for realm-b.
        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(BTreeMap::new()),
            Some("realm-a".to_string()),
        );

        let err = resolver.resolve(cross_uri).expect_err("unmarked");
        assert!(
            format!("{err:?}").contains("unknown_agent_ura"),
            "expected unknown_agent_ura, got {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cross_realm_dial_failure_surfaces_as_unknown() {
        let cross_uri = "easynet:///r/realm-b/device/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        );

        // INV-4 fail-closed: dial failure → unknown_agent_ura,
        // NOT a silent local fall-through.
        let err = resolver.resolve(cross_uri).expect_err("dial fail");
        assert!(
            format!("{err:?}").contains("unknown_agent_ura"),
            "expected unknown_agent_ura, got {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn same_realm_local_miss_does_not_dial_federated() {
        let same_realm_uri = "easynet:///r/realm-a/device/missing-device";
        let anchor = Arc::new(RealmTrustAnchor::default());

        // Federated peer wired, but the missing URI is in the
        // SAME realm as `self_realm` — INV-1 says local miss is
        // final, do not federate.
        let mut peers = BTreeMap::new();
        peers.insert("realm-a".to_string(), "https://hub-a:50443".to_string());

        // DialFailedClient ensures any accidental federated dial
        // would surface as a different error variant.
        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        );

        let err = resolver
            .resolve(same_realm_uri)
            .expect_err("same-realm miss");
        let err_str = format!("{err:?}");
        assert!(err_str.contains("unknown_agent_ura"));
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
        async fn forward_invoke(
            &self,
            _target_hub: &crate::services::federation_client::HubUri,
            _request: InvokeRequest,
        ) -> Result<
            crate::pb::axon::v1::InvokeResponse,
            crate::services::federation_client::FederationClientError,
        > {
            self.dial_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(crate::pb::axon::v1::InvokeResponse {
                result: self.canned_response.clone(),
                ..Default::default()
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ttl_cache_hits_avoid_repeat_dial_within_window() {
        // Two consecutive resolves on the same URI within the
        // TTL window: second hits cache, peer hub is dialed
        // exactly once.
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let cross_uri = "easynet:///r/realm-b/device/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        let response_json = serde_json::json!({ "public_key_b64": pk_b64 });
        let counting = Arc::new(CountingFederationClient::new(
            serde_json::to_vec(&response_json).unwrap(),
        ));
        let client: Arc<dyn FederationClient> = counting.clone();

        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        );

        let k1 = resolver.resolve(cross_uri).expect("dial 1");
        let k2 = resolver.resolve(cross_uri).expect("cache hit");
        assert_eq!(k1.to_bytes(), k2.to_bytes());
        assert_eq!(counting.dials(), 1, "second resolve must hit cache");
        assert_eq!(resolver.cache_len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ttl_cache_expires_after_window_and_redials() {
        // Short TTL forces expiry; the second resolve sees an
        // expired entry, evicts it, and dials again.
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let cross_uri = "easynet:///r/realm-b/device/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        let response_json = serde_json::json!({ "public_key_b64": pk_b64 });
        let counting = Arc::new(CountingFederationClient::new(
            serde_json::to_vec(&response_json).unwrap(),
        ));
        let client: Arc<dyn FederationClient> = counting.clone();

        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        )
        .with_cache_ttl(Duration::from_millis(50));

        let _ = resolver.resolve(cross_uri).expect("dial 1");
        std::thread::sleep(Duration::from_millis(80));
        let _ = resolver.resolve(cross_uri).expect("dial 2 post-expiry");
        assert_eq!(counting.dials(), 2, "expired cache entry must redial");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn flush_cache_clears_all_entries() {
        // Operator SIGHUP / key-rotation entry point. Simulate
        // by populating the cache, then calling flush_cache,
        // and asserting the next resolve hits the dial again.
        let (_signing, pk_b64) = ed25519_pubkey_b64();
        let cross_uri = "easynet:///r/realm-b/device/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        let response_json = serde_json::json!({ "public_key_b64": pk_b64 });
        let counting = Arc::new(CountingFederationClient::new(
            serde_json::to_vec(&response_json).unwrap(),
        ));
        let client: Arc<dyn FederationClient> = counting.clone();

        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        );

        let _ = resolver.resolve(cross_uri).expect("dial 1");
        assert_eq!(resolver.cache_len(), 1);
        resolver.flush_cache();
        assert_eq!(resolver.cache_len(), 0, "flush drops all entries");
        let _ = resolver.resolve(cross_uri).expect("dial 2 post-flush");
        assert_eq!(counting.dials(), 2, "post-flush resolve dials again");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dial_failure_does_not_poison_cache() {
        // A failed cross-hub dial must NOT cache the failure as
        // a negative entry — a recoverable peer-hub outage
        // would otherwise keep resolving as `unknown_agent_ura`
        // for the entire TTL even after the peer comes back.
        let cross_uri = "easynet:///r/realm-b/device/peer-device";
        let anchor = Arc::new(RealmTrustAnchor::default());
        let mut peers = BTreeMap::new();
        peers.insert("realm-b".to_string(), "https://hub-b:50443".to_string());

        let client: Arc<dyn FederationClient> = Arc::new(DialFailedClient);
        let resolver = FederatedKeyResolver::new(
            anchor,
            Some(client),
            Arc::new(peers),
            Some("realm-a".to_string()),
        );

        let _ = resolver.resolve(cross_uri).expect_err("dial fails");
        assert_eq!(
            resolver.cache_len(),
            0,
            "negative outcomes never poison the cache"
        );
    }
}
