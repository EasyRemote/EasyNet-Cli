//! On-miss trust sync for origin-caller claims.
//!
//! An executing device verifies a forwarded invocation's
//! the canonical invocation caller against its local realm trust anchor
//! (INV-1: admission is local-anchor-authoritative). But it cannot
//! pre-know every caller that may address it. Device keys are
//! registered at the realm hub (`register_device_pubkey` during
//! `device join`), and same-realm browser user keys are registered
//! there as `role = "user"`. This sync closes the gap with the same authority
//! direction as the session prelude: on a trust MISS for a syncable origin
//! caller, PULL the key from the hub (`federation.resolve_key`, routed through
//! this daemon's authenticated session channel). Device callers are imported
//! through the durable singleton trust policy. User/Authority caller keys
//! remain bounded, expiring Hub-attested runtime projections and never enter
//! this Device's local revocation ledger. Admission itself never consults the
//! network; the projection is warmed before the canonical admission retry.
//!
//! Hygiene: syncs are serialized (single-flight — a burst of frames
//! from one unknown caller triggers one resolve), and only an
//! authoritative hub "no keys" answer is remembered briefly so
//! repeated probes from an unregistered caller cannot turn into a
//! resolve storm. Transport, schema, and local import failures are
//! repairable state, not negative trust facts. Every failure leaves
//! the dispatch path to fail closed with the precise admission error.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::daemon::invocation::admission::federated_key_resolver::SharedHubAttestedCallerKeys;
use crate::daemon::invocation::admission::register_device_pubkey::RegisterPubkeyRequest;
use crate::daemon::invocation::bidi::session_escalation::SessionEscalationHandle;
use crate::daemon::invocation::bidi::session_wire::RequestOutcome;
use crate::daemon::trust::anchor::TrustAnchorRole;
use crate::daemon::trust::cell::SharedTrustAnchor;

/// How long a hub "unknown caller" answer suppresses re-resolving the
/// same URA. Long enough to bound storms, short enough that a freshly
/// joined device becomes admissible without operator action.
const NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(30);
const HUB_RESOLVE_KEY_TIMEOUT: Duration = Duration::from_secs(10);

pub struct DeviceTrustSync {
    daemon_realm: String,
    trust_anchor_path: PathBuf,
    cell: SharedTrustAnchor,
    /// Single-flight + authoritative negative cache. The lock only protects
    /// lifecycle state; hub resolve_key is always awaited outside the lock so
    /// session dispatch cannot self-deadlock while waiting for a reply carried
    /// by the same session.
    state: tokio::sync::Mutex<HashMap<String, TrustSyncState>>,
    /// Where hub-attested keys come from. Production uses the
    /// `session.open` escalation channel — the SAME authenticated
    /// hub channel the paired-user sync and hot-agent advertising
    /// use. A device-local `federation.resolve_key` invoke would be
    /// answered from THIS daemon's own anchor (the local ability
    /// shadows hub routing) and can never learn a new key.
    source: KeySource,
    /// Ephemeral trust projection consumed by the exact KeyResolver already
    /// installed in Axon's LocalRuntime. User and remote Authority keys are
    /// Hub-owned trust facts, so they are never written into this Device's
    /// durable local trust anchor.
    hub_attested_caller_keys: SharedHubAttestedCallerKeys,
}

enum KeySource {
    Session(Arc<SessionEscalationHandle>),
    /// Test seam: a pure function standing in for the hub.
    #[allow(dead_code)]
    Static(fn(&str) -> anyhow::Result<Vec<String>>),
}

enum TrustSyncState {
    Resolving(Arc<tokio::sync::Notify>),
    Negative(Instant),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DeviceTrustSyncStatus {
    NotSyncable,
    MalformedCaller(String),
    AlreadyTrusted,
    Synced,
    NegativeCached,
    HubReturnedNoKeys,
    ResolveFailed(String),
    ImportDidNotTrust,
    LocalAuthorityNotTrusted,
}

impl DeviceTrustSyncStatus {
    #[must_use]
    pub(crate) fn trusted(&self) -> bool {
        matches!(self, Self::AlreadyTrusted | Self::Synced)
    }

    #[must_use]
    pub(crate) fn diagnostic(&self) -> Option<String> {
        match self {
            Self::NotSyncable | Self::AlreadyTrusted | Self::Synced => None,
            Self::MalformedCaller(err) => Some(format!("caller URA is malformed: {err}")),
            Self::NegativeCached => Some("negative cache is active".to_string()),
            Self::HubReturnedNoKeys => Some("hub returned no public keys".to_string()),
            Self::ResolveFailed(err) => Some(format!("hub resolve_key failed: {err}")),
            Self::ImportDidNotTrust => Some(
                "hub-attested key projection completed but exact caller trust is still missing"
                    .to_string(),
            ),
            Self::LocalAuthorityNotTrusted => Some(
                "same-realm Authority key is absent from or does not match the local trust anchor"
                    .to_string(),
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SyncableCaller {
    Device {
        presented_pubkey_b64: Option<String>,
    },
    LocalUser {
        presented_pubkey_b64: String,
    },
    /// The current realm Authority is the session peer that forwards the
    /// invocation. Its key is provisioned in the durable realm trust anchor.
    /// Resolving it over that same session would wait on its own reply path.
    LocalAuthority {
        presented_pubkey_b64: Option<String>,
    },
    ExternalCaller {
        presented_pubkey_b64: String,
    },
}

impl SyncableCaller {
    fn register_role(&self) -> &'static str {
        match self {
            Self::Device { .. } => "device",
            Self::LocalUser { .. } => "user",
            Self::LocalAuthority { .. } => "authority",
            Self::ExternalCaller { .. } => "external",
        }
    }

    fn persisted_role(&self) -> Option<TrustAnchorRole> {
        match self {
            Self::Device { .. } => Some(TrustAnchorRole::Device),
            Self::LocalUser { .. } | Self::LocalAuthority { .. } | Self::ExternalCaller { .. } => {
                None
            }
        }
    }

    fn cache_key(&self, caller_ura: &str) -> String {
        match self {
            Self::Device {
                presented_pubkey_b64,
            } => match presented_pubkey_b64 {
                Some(pk) => format!("device:{caller_ura}:{pk}"),
                None => format!("device:{caller_ura}:*"),
            },
            Self::LocalUser {
                presented_pubkey_b64,
            } => format!("user:{caller_ura}:{presented_pubkey_b64}"),
            Self::LocalAuthority {
                presented_pubkey_b64,
            } => match presented_pubkey_b64 {
                Some(pk) => format!("authority:{caller_ura}:{pk}"),
                None => format!("authority:{caller_ura}:*"),
            },
            Self::ExternalCaller {
                presented_pubkey_b64,
            } => format!("external-caller:{caller_ura}:{presented_pubkey_b64}"),
        }
    }

    fn presented_pubkey_b64(&self) -> Option<&str> {
        match self {
            Self::Device {
                presented_pubkey_b64,
            } => presented_pubkey_b64.as_deref(),
            Self::LocalUser {
                presented_pubkey_b64,
            } => Some(presented_pubkey_b64),
            Self::LocalAuthority {
                presented_pubkey_b64,
            } => presented_pubkey_b64.as_deref(),
            Self::ExternalCaller {
                presented_pubkey_b64,
            } => Some(presented_pubkey_b64),
        }
    }
}

impl DeviceTrustSync {
    #[must_use]
    pub(crate) fn new(
        daemon_realm: String,
        trust_anchor_path: PathBuf,
        cell: SharedTrustAnchor,
        escalation: Arc<SessionEscalationHandle>,
        hub_attested_caller_keys: SharedHubAttestedCallerKeys,
    ) -> Self {
        Self::with_source(
            daemon_realm,
            trust_anchor_path,
            cell,
            KeySource::Session(escalation),
            hub_attested_caller_keys,
        )
    }

    fn with_source(
        daemon_realm: String,
        trust_anchor_path: PathBuf,
        cell: SharedTrustAnchor,
        source: KeySource,
        hub_attested_caller_keys: SharedHubAttestedCallerKeys,
    ) -> Self {
        Self {
            daemon_realm,
            trust_anchor_path,
            cell,
            state: tokio::sync::Mutex::new(HashMap::new()),
            source,
            hub_attested_caller_keys,
        }
    }

    async fn resolve_from_hub(
        &self,
        agent_ura: &str,
        presented_pubkey_b64: Option<&str>,
    ) -> anyhow::Result<ResolvedCallerTrust> {
        match &self.source {
            KeySource::Session(handle) => {
                let mut args_value = serde_json::json!({ "agent_ura": agent_ura });
                if let Some(pk) = presented_pubkey_b64.filter(|pk| !pk.is_empty()) {
                    args_value["presented_pubkey_b64"] = serde_json::Value::String(pk.to_string());
                }
                let args = serde_json::to_vec(&args_value)?;
                let ability =
                    crate::daemon::invocation::dispatch::federation_wrappers::ABILITY_FEDERATION_RESOLVE_KEY;
                match handle.escalate(ability.to_string(), args).await {
                    RequestOutcome::Ok { result_bytes } => {
                        parse_resolved_caller_trust(&result_bytes)
                    }
                    RequestOutcome::Err { error } => {
                        anyhow::bail!("hub resolve_key failed: {error:?}")
                    }
                }
            }
            KeySource::Static(resolver) => resolver(agent_ura).map(ResolvedCallerTrust::keys_only),
        }
    }

    /// Make a single-key device caller admissible if the realm hub attests it.
    pub async fn ensure_caller_key(&self, caller_ura: &str) -> bool {
        self.ensure_caller_key_with_presented_pubkey(caller_ura, None)
            .await
    }

    /// Make `caller_ura`'s key admissible if the realm hub attests it.
    /// Returns whether the anchor holds the required entry afterwards;
    /// `false` simply lets the claim dispatch fail closed downstream.
    ///
    /// For user callers, `presented_pubkey_b64` pins the check to the
    /// exact browser key carried by the signed origin-caller claim.
    pub async fn ensure_caller_key_with_presented_pubkey(
        &self,
        caller_ura: &str,
        presented_pubkey_b64: Option<&str>,
    ) -> bool {
        self.ensure_caller_key_status(caller_ura, presented_pubkey_b64)
            .await
            .trusted()
    }

    pub(crate) async fn ensure_caller_key_status(
        &self,
        caller_ura: &str,
        presented_pubkey_b64: Option<&str>,
    ) -> DeviceTrustSyncStatus {
        let role = match self.syncable_caller(caller_ura, presented_pubkey_b64) {
            Ok(Some(role)) => role,
            Ok(None) => return DeviceTrustSyncStatus::NotSyncable,
            Err(err) => return DeviceTrustSyncStatus::MalformedCaller(err),
        };
        if self.anchor_has_caller_key(caller_ura, &role) {
            return DeviceTrustSyncStatus::AlreadyTrusted;
        }
        if matches!(role, SyncableCaller::LocalAuthority { .. }) {
            return DeviceTrustSyncStatus::LocalAuthorityNotTrusted;
        }
        let cache_key = role.cache_key(caller_ura);

        loop {
            let wait_for = {
                let mut state = self.state.lock().await;
                // Double-check under the lock: a concurrent miss may have
                // synced while this one waited.
                if self.anchor_has_caller_key(caller_ura, &role) {
                    return DeviceTrustSyncStatus::AlreadyTrusted;
                }
                match state.get(&cache_key) {
                    Some(TrustSyncState::Negative(failed_at))
                        if failed_at.elapsed() < NEGATIVE_CACHE_TTL =>
                    {
                        return DeviceTrustSyncStatus::NegativeCached;
                    }
                    Some(TrustSyncState::Negative(_)) => {
                        state.remove(&cache_key);
                        None
                    }
                    Some(TrustSyncState::Resolving(notify)) => Some(Arc::clone(notify)),
                    None => {
                        state.insert(
                            cache_key.clone(),
                            TrustSyncState::Resolving(Arc::new(tokio::sync::Notify::new())),
                        );
                        None
                    }
                }
            };
            match wait_for {
                Some(notify) => notify.notified().await,
                None => break,
            }
        }

        let resolve_result = tokio::time::timeout(
            HUB_RESOLVE_KEY_TIMEOUT,
            self.resolve_from_hub(caller_ura, role.presented_pubkey_b64()),
        )
        .await
        .map_err(|_| anyhow::anyhow!("hub resolve_key timed out after {HUB_RESOLVE_KEY_TIMEOUT:?}"))
        .and_then(|result| result);

        let resolved = match resolve_result {
            Ok(resolved) if !resolved.public_keys_b64.is_empty() => resolved,
            Ok(_) => {
                self.finish_sync_state(&cache_key, Some(TrustSyncState::Negative(Instant::now())))
                    .await;
                return DeviceTrustSyncStatus::HubReturnedNoKeys;
            }
            Err(err) => {
                let diagnostic = err.to_string();
                self.finish_sync_state(&cache_key, None).await;
                crate::op_event!(
                    component = device_trust_sync,
                    kind = resolve_failed,
                    caller_ura = caller_ura,
                    error = diagnostic,
                );
                return DeviceTrustSyncStatus::ResolveFailed(diagnostic);
            }
        };

        let imported = self.import_caller_trust(caller_ura, &resolved, &role);
        if imported {
            self.finish_sync_state(&cache_key, None).await;
            crate::op_event!(
                component = device_trust_sync,
                kind = caller_key_synced,
                caller_ura = caller_ura,
                role = role.register_role(),
            );
            DeviceTrustSyncStatus::Synced
        } else {
            self.finish_sync_state(&cache_key, None).await;
            DeviceTrustSyncStatus::ImportDidNotTrust
        }
    }

    async fn finish_sync_state(&self, cache_key: &str, replacement: Option<TrustSyncState>) {
        let notify = {
            let mut state = self.state.lock().await;
            let notify = match state.remove(cache_key) {
                Some(TrustSyncState::Resolving(notify)) => Some(notify),
                _ => None,
            };
            if let Some(replacement) = replacement {
                state.insert(cache_key.to_string(), replacement);
            }
            notify
        };
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
    }

    fn syncable_caller(
        &self,
        caller_ura: &str,
        presented_pubkey_b64: Option<&str>,
    ) -> Result<Option<SyncableCaller>, String> {
        let parsed = crate::core::ura::parse_ura(caller_ura)
            .map_err(|error| format!("invalid caller_ura `{caller_ura}`: {error}"))?;
        match parsed.kind {
            crate::core::ura::URAKind::Device => Ok(Some(SyncableCaller::Device {
                presented_pubkey_b64: presented_pubkey_b64
                    .filter(|pk| !pk.is_empty())
                    .map(str::to_string),
            })),
            crate::core::ura::URAKind::User if parsed.realm == self.daemon_realm => {
                let presented_pubkey_b64 = presented_pubkey_b64
                    .map(str::trim)
                    .filter(|pk| !pk.is_empty())
                    .ok_or_else(|| {
                        "User caller requires an exact presented public key".to_string()
                    })?;
                Ok(Some(SyncableCaller::LocalUser {
                    presented_pubkey_b64: presented_pubkey_b64.to_string(),
                }))
            }
            crate::core::ura::URAKind::Authority if parsed.realm == self.daemon_realm => {
                Ok(Some(SyncableCaller::LocalAuthority {
                    presented_pubkey_b64: presented_pubkey_b64
                        .filter(|pk| !pk.is_empty())
                        .map(str::to_string),
                }))
            }
            crate::core::ura::URAKind::User | crate::core::ura::URAKind::Authority => {
                let presented_pubkey_b64 = presented_pubkey_b64
                    .map(str::trim)
                    .filter(|pk| !pk.is_empty())
                    .ok_or_else(|| {
                        "external User/Authority caller requires an exact presented public key"
                            .to_string()
                    })?;
                Ok(Some(SyncableCaller::ExternalCaller {
                    presented_pubkey_b64: presented_pubkey_b64.to_string(),
                }))
            }
            _ => Ok(None),
        }
    }

    fn anchor_has_caller_key(&self, caller_ura: &str, role: &SyncableCaller) -> bool {
        let anchor = self.cell.snapshot();
        match role {
            SyncableCaller::Device {
                presented_pubkey_b64: Some(pk),
            } => anchor
                .lookup(caller_ura)
                .map(|entry| entry.public_key_b64 == *pk)
                .unwrap_or(false),
            SyncableCaller::Device {
                presented_pubkey_b64: None,
            } => anchor.lookup(caller_ura).is_some(),
            SyncableCaller::LocalUser {
                presented_pubkey_b64,
            } => self
                .hub_attested_caller_keys
                .contains(caller_ura, presented_pubkey_b64),
            SyncableCaller::LocalAuthority {
                presented_pubkey_b64: Some(pk),
            } => anchor
                .lookup(caller_ura)
                .map(|entry| entry.public_key_b64 == *pk)
                .unwrap_or(false),
            SyncableCaller::LocalAuthority {
                presented_pubkey_b64: None,
            } => anchor.lookup(caller_ura).is_some(),
            SyncableCaller::ExternalCaller {
                presented_pubkey_b64,
            } => self
                .hub_attested_caller_keys
                .contains(caller_ura, presented_pubkey_b64),
        }
    }

    /// Project Hub-owned caller keys into the bounded runtime cache, or import
    /// Device keys through the durable singleton write policy. These are two
    /// distinct authority planes: a Device must never create permanent User
    /// revocation facts from cache eviction.
    fn import_caller_trust(
        &self,
        caller_ura: &str,
        resolved: &ResolvedCallerTrust,
        role: &SyncableCaller,
    ) -> bool {
        if let SyncableCaller::LocalUser {
            presented_pubkey_b64,
        }
        | SyncableCaller::ExternalCaller {
            presented_pubkey_b64,
        } = role
        {
            if let Err(error) = self.hub_attested_caller_keys.attest_caller_key(
                caller_ura,
                presented_pubkey_b64,
                &resolved.public_keys_b64,
            ) {
                crate::op_event!(
                    component = device_trust_sync,
                    kind = hub_attested_caller_key_rejected,
                    caller_ura = caller_ura,
                    error = error.to_string(),
                );
                return false;
            }
            return self.anchor_has_caller_key(caller_ura, role);
        }
        let persisted_role = role
            .persisted_role()
            .expect("only Device caller trust reaches durable import");
        for public_key_b64 in &resolved.public_keys_b64 {
            let request = match resolved.principal_owner_ura.as_deref() {
                Some(owner_ura) => {
                    RegisterPubkeyRequest::new(caller_ura, public_key_b64, persisted_role)
                        .with_principal_owner(owner_ura)
                }
                None => RegisterPubkeyRequest::new(caller_ura, public_key_b64, persisted_role),
            };
            let register_args = match request.to_arguments_bytes() {
                Ok(v) => v,
                Err(_) => continue,
            };
            match crate::daemon::invocation::admission::register_device_pubkey::handle(
                &register_args,
                &self.daemon_realm,
                &self.trust_anchor_path,
                &self.cell,
            ) {
                Ok(_) => {}
                Err(status) if status.code() == tonic::Code::AlreadyExists => {}
                Err(status) => {
                    crate::op_event!(
                        component = device_trust_sync,
                        kind = import_rejected,
                        caller_ura = caller_ura,
                        role = role.register_role(),
                        error = status.message(),
                    );
                }
            }
        }
        self.anchor_has_caller_key(caller_ura, role)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResolvedCallerTrust {
    public_keys_b64: Vec<String>,
    principal_owner_ura: Option<String>,
}

impl ResolvedCallerTrust {
    fn keys_only(public_keys_b64: Vec<String>) -> Self {
        Self {
            public_keys_b64,
            principal_owner_ura: None,
        }
    }
}

/// Parse the hub's resolve_key reply.
///
/// The hub response is schema-bound trust evidence. New hubs return
/// `public_keys_b64` for both single-key and multi-key principals; a missing
/// field, non-array field, or malformed row is a corrupt authority response,
/// not a signal to repair from legacy single-key fields. An empty array remains
/// the explicit "hub has no keys" answer used by negative caching.
fn parse_resolved_caller_trust(result_bytes: &[u8]) -> anyhow::Result<ResolvedCallerTrust> {
    let response: serde_json::Value = serde_json::from_slice(result_bytes)?;
    let keys = parse_public_keys_b64_field(&response)?;
    Ok(ResolvedCallerTrust {
        public_keys_b64: keys,
        principal_owner_ura: response
            .get("principal_owner_ura")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
    })
}

fn parse_public_keys_b64_field(response: &serde_json::Value) -> anyhow::Result<Vec<String>> {
    let public_keys = response
        .get("public_keys_b64")
        .ok_or_else(|| anyhow::anyhow!("resolve_key_response_missing_public_keys_b64"))?
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("resolve_key_response_public_keys_b64_not_array"))?;
    public_keys
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let key = value.as_str().ok_or_else(|| {
                anyhow::anyhow!("resolve_key_response_public_keys_b64[{index}]_not_string")
            })?;
            let key = key.trim();
            if key.is_empty() {
                anyhow::bail!("resolve_key_response_public_keys_b64[{index}]_empty");
            }
            Ok(key.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::daemon::trust::anchor::{
        RealmTrustAnchor, RevokedUserPubkey, TrustAnchorRole, TrustedAgent,
    };

    fn empty_cell() -> SharedTrustAnchor {
        SharedTrustAnchor::new(Arc::new(
            RealmTrustAnchor::from_entries(vec![]).expect("empty anchor"),
        ))
    }

    fn cell_with_user_key(user_ura: &str, public_key_b64: &str) -> SharedTrustAnchor {
        SharedTrustAnchor::new(Arc::new(
            RealmTrustAnchor::from_entries(vec![TrustedAgent {
                agent_ura: user_ura.to_string(),
                public_key_b64: public_key_b64.to_string(),
                role: TrustAnchorRole::User,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            }])
            .expect("user anchor"),
        ))
    }

    fn sync_with(
        resolver: fn(&str) -> anyhow::Result<Vec<String>>,
        dir: &tempfile::TempDir,
    ) -> DeviceTrustSync {
        DeviceTrustSync::with_source(
            "test-realm".into(),
            dir.path().join("realm-trust.toml"),
            empty_cell(),
            KeySource::Static(resolver),
            SharedHubAttestedCallerKeys::new(),
        )
    }

    fn test_key_b64() -> String {
        B64.encode(
            SigningKey::from_bytes(&[0x42; 32])
                .verifying_key()
                .to_bytes(),
        )
    }

    #[test]
    fn parse_resolved_caller_trust_requires_schema_bound_public_keys() {
        let legacy_single_key = serde_json::json!({
            "public_key_b64": test_key_b64()
        });

        let err = parse_resolved_caller_trust(
            &serde_json::to_vec(&legacy_single_key).expect("json serializes"),
        )
        .expect_err("legacy single-key resolve_key response must not be repaired");

        assert!(
            err.to_string()
                .contains("resolve_key_response_missing_public_keys_b64"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_resolved_caller_trust_rejects_malformed_public_key_rows() {
        let malformed = serde_json::json!({
            "public_keys_b64": [test_key_b64(), 7]
        });

        let err =
            parse_resolved_caller_trust(&serde_json::to_vec(&malformed).expect("json serializes"))
                .expect_err("malformed key row must not be skipped");

        assert!(
            err.to_string()
                .contains("resolve_key_response_public_keys_b64[1]_not_string"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_resolved_caller_trust_accepts_empty_public_key_array_as_hub_miss() {
        let miss = serde_json::json!({
            "public_keys_b64": []
        });

        let resolved =
            parse_resolved_caller_trust(&serde_json::to_vec(&miss).expect("json serializes"))
                .expect("empty array is an explicit hub miss");

        assert!(resolved.public_keys_b64.is_empty());
    }

    #[tokio::test]
    async fn miss_resolves_imports_and_admits() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![B64.encode(
                SigningKey::from_bytes(&[0x42; 32])
                    .verifying_key()
                    .to_bytes(),
            )])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        let ura = "easynet:///r/test-realm/device/node-a";

        assert!(sync.ensure_caller_key(ura).await, "synced key must admit");
        assert!(sync.cell.snapshot().lookup(ura).is_some());
        // Second call is an anchor hit (no resolve needed to prove —
        // returns true immediately).
        assert!(sync.ensure_caller_key(ura).await);
        let _ = test_key_b64();
    }

    #[test]
    fn import_caller_trust_persists_key_and_principal_owner_fact() {
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(|_| Ok(Vec::new()), &dir);
        let ura = "easynet:///r/test-realm/device/node-a";
        let key = test_key_b64();
        let resolved = ResolvedCallerTrust {
            public_keys_b64: vec![key.clone()],
            principal_owner_ura: Some("easynet:///r/test-realm/user/alice".to_string()),
        };

        assert!(sync.import_caller_trust(
            ura,
            &resolved,
            &SyncableCaller::Device {
                presented_pubkey_b64: Some(key.clone()),
            },
        ));
        let anchor = sync.cell.snapshot();
        assert_eq!(
            anchor
                .lookup(ura)
                .map(|entry| entry.public_key_b64.as_str()),
            Some(key.as_str())
        );
        let owner = anchor
            .lookup_principal_owner(ura)
            .expect("principal owner imported");
        assert_eq!(owner.owner_user_id, "alice");
        assert_eq!(owner.owner_ura, "easynet:///r/test-realm/user/alice");
    }

    #[tokio::test]
    async fn cross_realm_device_miss_resolves_imports_and_admits() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![B64.encode(
                SigningKey::from_bytes(&[0x43; 32])
                    .verifying_key()
                    .to_bytes(),
            )])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        let ura = "easynet:///r/peer-realm/device/node-a";

        assert!(
            sync.ensure_caller_key(ura).await,
            "federated device caller keys are hub-attested and locally warmed"
        );
        assert!(sync.cell.snapshot().lookup(ura).is_some());
    }

    #[tokio::test]
    async fn hub_unknown_fails_closed_and_is_negative_cached() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        let ura = "easynet:///r/test-realm/device/unknown";

        assert!(!sync.ensure_caller_key(ura).await);
        assert!(
            sync.state
                .lock()
                .await
                .contains_key(&format!("device:{ura}:*")),
            "unknown caller must be negative-cached"
        );
        assert!(!sync.ensure_caller_key(ura).await);
    }

    #[tokio::test]
    async fn resolve_failure_fails_closed_without_negative_cache() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            anyhow::bail!("temporary hub channel unavailable")
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        let ura = "easynet:///r/test-realm/device/transient";

        let status = sync.ensure_caller_key_status(ura, None).await;

        match status {
            DeviceTrustSyncStatus::ResolveFailed(message) => {
                assert!(
                    message.contains("temporary hub channel unavailable"),
                    "resolve diagnostic must preserve root error: {message}"
                );
            }
            other => panic!("transient resolve failure must stay typed, got {other:?}"),
        }
        assert!(
            !sync
                .state
                .lock()
                .await
                .contains_key(&format!("device:{ura}:*")),
            "transient resolve failures must not activate negative cache"
        );
    }

    #[tokio::test]
    async fn import_rejection_fails_closed_without_negative_cache() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec!["not-base64".to_string()])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        let ura = "easynet:///r/test-realm/device/bad-key";

        let status = sync.ensure_caller_key_status(ura, None).await;

        assert_eq!(status, DeviceTrustSyncStatus::ImportDidNotTrust);
        assert!(
            !sync
                .state
                .lock()
                .await
                .contains_key(&format!("device:{ura}:*")),
            "corrupt/import-rejected authority data must not activate negative cache"
        );
    }

    /// Deterministic User keys for the Hub-attested runtime projection tests.
    fn tombstoned_key_test_b64() -> String {
        B64.encode(
            SigningKey::from_bytes(&[0x71; 32])
                .verifying_key()
                .to_bytes(),
        )
    }

    #[tokio::test]
    async fn same_realm_user_hub_attestation_does_not_enter_local_revocation_ledger() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![tombstoned_key_test_b64()])
        }
        let user_ura = "easynet:///r/test-realm/user/alice";
        let tombstoned = tombstoned_key_test_b64();
        let dir = tempfile::tempdir().expect("tmp");
        let cell = SharedTrustAnchor::new(Arc::new(
            RealmTrustAnchor::from_parts(
                vec![],
                vec![RevokedUserPubkey {
                    agent_ura: user_ura.to_string(),
                    public_key_b64: tombstoned.clone(),
                    revoked_at_unix_ms: 1_700_000_000_000,
                    rotation_epoch: 1,
                }],
            )
            .expect("anchor with tombstone"),
        ));
        let hub_attested = SharedHubAttestedCallerKeys::new();
        let sync = DeviceTrustSync::with_source(
            "test-realm".into(),
            dir.path().join("realm-trust.toml"),
            cell,
            KeySource::Static(resolver),
            hub_attested.clone(),
        );

        let status = sync
            .ensure_caller_key_status(user_ura, Some(&tombstoned))
            .await;

        assert_eq!(status, DeviceTrustSyncStatus::Synced);
        let snapshot = sync.cell.snapshot();
        assert!(
            snapshot
                .lookup_user_by_pubkey(user_ura, &tombstoned)
                .is_none(),
            "Hub-owned caller proof must not resurrect or mutate local durable User trust"
        );
        assert!(snapshot.is_user_pubkey_revoked(user_ura, &tombstoned));
        assert!(
            hub_attested.contains(user_ura, &tombstoned),
            "exact Hub proof must remain available to the runtime resolver"
        );
    }

    #[tokio::test]
    async fn same_realm_device_existing_different_key_syncs_presented_key() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![B64.encode(
                SigningKey::from_bytes(&[0x62; 32])
                    .verifying_key()
                    .to_bytes(),
            )])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let ura = "easynet:///r/test-realm/device/node-a";
        let stale = B64.encode(
            SigningKey::from_bytes(&[0x61; 32])
                .verifying_key()
                .to_bytes(),
        );
        let presented = B64.encode(
            SigningKey::from_bytes(&[0x62; 32])
                .verifying_key()
                .to_bytes(),
        );
        let cell = SharedTrustAnchor::new(Arc::new(
            RealmTrustAnchor::from_entries(vec![TrustedAgent {
                agent_ura: ura.to_string(),
                public_key_b64: stale,
                role: TrustAnchorRole::Device,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            }])
            .expect("stale device anchor"),
        ));
        let sync = DeviceTrustSync::with_source(
            "test-realm".into(),
            dir.path().join("realm-trust.toml"),
            cell,
            KeySource::Static(resolver),
            SharedHubAttestedCallerKeys::new(),
        );

        assert!(
            sync.ensure_caller_key_with_presented_pubkey(ura, Some(&presented))
                .await,
            "presented device key should replace the stale same-URA key"
        );
        assert_eq!(
            sync.cell
                .snapshot()
                .lookup(ura)
                .map(|entry| entry.public_key_b64.as_str()),
            Some(presented.as_str())
        );
    }

    #[tokio::test]
    async fn same_realm_user_miss_projects_presented_key_ephemerally() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![B64.encode(
                SigningKey::from_bytes(&[0x51; 32])
                    .verifying_key()
                    .to_bytes(),
            )])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        let user_ura = "easynet:///r/test-realm/user/alice";
        let presented = B64.encode(
            SigningKey::from_bytes(&[0x51; 32])
                .verifying_key()
                .to_bytes(),
        );

        assert!(
            sync.ensure_caller_key_with_presented_pubkey(user_ura, Some(&presented))
                .await,
            "hub-attested user key must be projected before admission"
        );
        assert!(sync.cell.snapshot().lookup_user_all(user_ura).is_empty());
        assert!(sync.hub_attested_caller_keys.contains(user_ura, &presented));
    }

    #[tokio::test]
    async fn same_realm_user_without_presented_key_fails_before_hub_resolution() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![B64.encode(
                SigningKey::from_bytes(&[0x52; 32])
                    .verifying_key()
                    .to_bytes(),
            )])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        let user_ura = "easynet:///r/test-realm/user/alice";

        let status = sync.ensure_caller_key_status(user_ura, None).await;

        assert!(matches!(
            status,
            DeviceTrustSyncStatus::MalformedCaller(message)
                if message.contains("exact presented public key")
        ));
        assert!(sync.state.lock().await.is_empty());
    }

    #[tokio::test]
    async fn same_realm_user_existing_different_key_still_syncs_presented_key() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![B64.encode(
                SigningKey::from_bytes(&[0x62; 32])
                    .verifying_key()
                    .to_bytes(),
            )])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let user_ura = "easynet:///r/test-realm/user/alice";
        let old_key = B64.encode(
            SigningKey::from_bytes(&[0x61; 32])
                .verifying_key()
                .to_bytes(),
        );
        let presented = B64.encode(
            SigningKey::from_bytes(&[0x62; 32])
                .verifying_key()
                .to_bytes(),
        );
        let sync = DeviceTrustSync::with_source(
            "test-realm".into(),
            dir.path().join("realm-trust.toml"),
            cell_with_user_key(user_ura, &old_key),
            KeySource::Static(resolver),
            SharedHubAttestedCallerKeys::new(),
        );

        assert!(
            sync.ensure_caller_key_with_presented_pubkey(user_ura, Some(&presented))
                .await,
            "presented browser key must not be hidden by another key under the same user URA"
        );
        let anchor = sync.cell.snapshot();
        assert_eq!(anchor.lookup_user_all(user_ura).len(), 1);
        assert!(anchor.lookup_user_by_pubkey(user_ura, &old_key).is_some());
        assert!(anchor.lookup_user_by_pubkey(user_ura, &presented).is_none());
        assert!(sync.hub_attested_caller_keys.contains(user_ura, &presented));
    }

    #[tokio::test]
    async fn same_realm_authority_uses_exact_local_anchor_without_session_resolution() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            panic!("same-realm Authority must not resolve over its own session");
        }
        let dir = tempfile::tempdir().expect("tmp");
        let authority_ura = "easynet:///r/test-realm/authority";
        let presented = B64.encode(resolver_key_bytes(0x75));
        let cell = SharedTrustAnchor::new(Arc::new(
            RealmTrustAnchor::from_entries(vec![TrustedAgent {
                agent_ura: authority_ura.to_string(),
                public_key_b64: presented.clone(),
                role: TrustAnchorRole::Hub,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            }])
            .expect("same-realm Authority anchor"),
        ));
        let sync = DeviceTrustSync::with_source(
            "test-realm".into(),
            dir.path().join("realm-trust.toml"),
            cell,
            KeySource::Static(resolver),
            SharedHubAttestedCallerKeys::new(),
        );

        let status = sync
            .ensure_caller_key_status(authority_ura, Some(&presented))
            .await;

        assert_eq!(status, DeviceTrustSyncStatus::AlreadyTrusted);
    }

    #[tokio::test]
    async fn same_realm_authority_key_mismatch_fails_closed_without_session_resolution() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            panic!("same-realm Authority mismatch must fail locally");
        }
        let dir = tempfile::tempdir().expect("tmp");
        let authority_ura = "easynet:///r/test-realm/authority";
        let anchored = B64.encode(resolver_key_bytes(0x75));
        let presented = B64.encode(resolver_key_bytes(0x76));
        let cell = SharedTrustAnchor::new(Arc::new(
            RealmTrustAnchor::from_entries(vec![TrustedAgent {
                agent_ura: authority_ura.to_string(),
                public_key_b64: anchored,
                role: TrustAnchorRole::Hub,
                added_at_unix_ms: 1_700_000_000_000,
                origin_realm: None,
                hub_endpoint: None,
                tls_ca_pem_path: None,
            }])
            .expect("same-realm Authority anchor"),
        ));
        let sync = DeviceTrustSync::with_source(
            "test-realm".into(),
            dir.path().join("realm-trust.toml"),
            cell,
            KeySource::Static(resolver),
            SharedHubAttestedCallerKeys::new(),
        );

        let status = sync
            .ensure_caller_key_status(authority_ura, Some(&presented))
            .await;

        assert_eq!(status, DeviceTrustSyncStatus::LocalAuthorityNotTrusted);
    }

    #[tokio::test]
    async fn cross_realm_user_uses_ephemeral_hub_attestation_without_local_registration() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![B64.encode(
                SigningKey::from_bytes(&[0x73; 32])
                    .verifying_key()
                    .to_bytes(),
            )])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let cell = empty_cell();
        let canonical_resolver =
            crate::daemon::invocation::admission::federated_key_resolver::FederatedKeyResolver::new(
                cell.clone(),
                None,
                crate::daemon::federation::peers::SharedFederatedPeers::default(),
                Some("test-realm".to_string()),
            );
        let hub_attested = canonical_resolver.hub_attested_caller_keys();
        let sync = DeviceTrustSync::with_source(
            "test-realm".into(),
            dir.path().join("realm-trust.toml"),
            cell.clone(),
            KeySource::Static(resolver),
            hub_attested,
        );
        let user_ura = "easynet:///r/peer-realm/user/alice";
        let presented = B64.encode(
            SigningKey::from_bytes(&[0x73; 32])
                .verifying_key()
                .to_bytes(),
        );

        let status = sync
            .ensure_caller_key_status(user_ura, Some(&presented))
            .await;

        assert_eq!(status, DeviceTrustSyncStatus::Synced);
        assert!(
            cell.snapshot().lookup_user_all(user_ura).is_empty(),
            "external User key must not become destination-realm durable trust"
        );
        let resolved =
            axon_sdk::invocation::KeyResolver::resolve_all(&canonical_resolver, user_ura)
                .expect("runtime resolver sees the Hub-attested projection");
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].to_bytes(), resolver_key_bytes(0x73));
    }

    #[tokio::test]
    async fn origin_authority_uses_ephemeral_hub_attestation_for_peer_runtime_admission() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            Ok(vec![B64.encode(resolver_key_bytes(0x74))])
        }
        let dir = tempfile::tempdir().expect("tmp");
        let cell = empty_cell();
        let canonical_resolver =
            crate::daemon::invocation::admission::federated_key_resolver::FederatedKeyResolver::new(
                cell.clone(),
                None,
                crate::daemon::federation::peers::SharedFederatedPeers::default(),
                Some("test-realm".to_string()),
            );
        let sync = DeviceTrustSync::with_source(
            "test-realm".into(),
            dir.path().join("realm-trust.toml"),
            cell.clone(),
            KeySource::Static(resolver),
            canonical_resolver.hub_attested_caller_keys(),
        );
        let authority_ura = "easynet:///r/peer-realm/authority";
        let presented = B64.encode(resolver_key_bytes(0x74));

        let status = sync
            .ensure_caller_key_status(authority_ura, Some(&presented))
            .await;

        assert_eq!(status, DeviceTrustSyncStatus::Synced);
        assert!(
            cell.snapshot().lookup(authority_ura).is_none(),
            "origin Authority must not become a durable local trust-anchor row"
        );
        let resolved =
            axon_sdk::invocation::KeyResolver::resolve_all(&canonical_resolver, authority_ura)
                .expect("runtime resolver sees the Hub-attested origin Authority");
        assert_eq!(resolved[0].to_bytes(), resolver_key_bytes(0x74));
    }

    fn resolver_key_bytes(seed: u8) -> [u8; 32] {
        SigningKey::from_bytes(&[seed; 32])
            .verifying_key()
            .to_bytes()
    }

    #[tokio::test]
    async fn non_syncable_callers_never_sync() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            panic!("resolver must not run for non-device callers");
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);
        assert!(
            !sync
                .ensure_caller_key("easynet:///r/test-realm/authority")
                .await
        );
        let status = sync
            .ensure_caller_key_status("easynet:///r/other-realm/user/alice", None)
            .await;
        assert!(matches!(status, DeviceTrustSyncStatus::MalformedCaller(_)));
    }

    #[tokio::test]
    async fn malformed_caller_ura_is_not_reported_as_non_syncable() {
        fn resolver(_ura: &str) -> anyhow::Result<Vec<String>> {
            panic!("resolver must not run for malformed callers");
        }
        let dir = tempfile::tempdir().expect("tmp");
        let sync = sync_with(resolver, &dir);

        let status = sync
            .ensure_caller_key_status("not-a-canonical-ura", Some(&test_key_b64()))
            .await;

        match status {
            DeviceTrustSyncStatus::MalformedCaller(message) => {
                assert!(
                    message.contains("invalid caller_ura"),
                    "malformed status must preserve parse context: {message}"
                );
            }
            other => panic!("malformed caller must be typed separately, got {other:?}"),
        }
        assert!(
            !sync.ensure_caller_key("not-a-canonical-ura").await,
            "public bool helper must still fail closed"
        );
    }
}
