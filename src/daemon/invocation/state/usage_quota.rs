// EasyNet CLI - daemon Invocation usage quota gate
// =============================================================
//
// File: src/daemon/invocation/state/usage_quota.rs
//
// Per-(consumer-URA, ability) invocation quota for unary invokes.
// The admission gate consults this module only AFTER identity,
// signature, and replay checks have already admitted the caller.
// Quota is a governance refinement; it is not an authentication or
// authorization substitute.
//
// Scope: unary invoke only
// ------------------------
// Metering is wired into the unary `invoke` RPC, where the daemon can
// reject before dispatch and attach Axon's `RateLimitInfo` to the
// single response. Streaming and bidi calls are not metered here.
//
// Runtime ownership
// -----------------
// `SharedUsageQuotaGate` is the owner object. It holds a reloadable
// `QuotaConfig` plus a bounded in-memory counter store. Boot clones the
// same gate into the `AdmissionFacade` and into the SIGHUP reload task,
// so editing `[daemon.quota]` and sending SIGHUP changes the next
// admission without rebuilding the gRPC service.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use crate::persistence::daemon_config::QuotaConfig;
use sha2::{Digest, Sha256};

/// Maximum distinct `(consumer_ura, ability)` windows retained in one
/// daemon process.
///
/// Justification: quota keys are attacker-influenced after admission
/// because `function_name` participates in the key. The daemon must
/// never let an admitted caller allocate unbounded counters by probing
/// random ability names. 16K keys is well above the expected product
/// envelope (hundreds of users/devices times tens of active abilities)
/// while keeping the map bounded to a few MiB.
pub const MAX_QUOTA_TRACKED_KEYS: usize = 16 * 1024;

/// Maximum accepted caller URA bytes for quota-key material.
///
/// Justification: quota runs after admission, but the caller URA still
/// arrives from the wire envelope. A normal user/device/hub URA is well
/// below this bound; rejecting larger keys prevents an admitted caller
/// from turning the quota map into an unbounded string-retention sink.
pub const MAX_QUOTA_CONSUMER_URA_BYTES: usize = 512;

/// Maximum accepted ability-name bytes for quota-key material.
///
/// Justification: ability names are protocol identifiers, not payloads.
/// The largest product-owned names are short dotted strings. Keeping a
/// hard byte ceiling preserves the "few MiB" store bound even when a
/// caller probes unknown ability names.
pub const MAX_QUOTA_ABILITY_NAME_BYTES: usize = 256;

/// Why a metered call was denied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuotaDenyReason {
    /// The caller consumed every request in its current window.
    BudgetExhausted,
    /// The daemon's bounded counter table is full even after expired
    /// windows were pruned.
    StoreSaturated,
    /// The caller supplied quota-key material that is too large to
    /// retain or hash under the daemon's quota contract.
    KeyTooLarge,
}

/// Outcome of one quota check. Mirrors the integer fields Axon's
/// `RateLimitInfo` carries, with an internal denial reason so the
/// admission facade can produce an operator-accurate gRPC message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuotaDecision {
    /// Whether the call is allowed.
    pub allowed: bool,
    /// Requests still available in the current window after this call.
    pub quota_remaining: i32,
    /// Total requests allowed per window for this key.
    pub quota_limit: i32,
    /// Wall-clock millis at which the current window resets.
    pub reset_at_unix_ms: i64,
    /// `0` when not throttled; otherwise the millis the caller should
    /// wait before retrying.
    pub retry_after_ms: i32,
    /// Present only when `allowed == false`.
    pub deny_reason: Option<QuotaDenyReason>,
}

impl QuotaDecision {
    /// The decision for an unmetered key: always allowed, no window,
    /// no throttle. Used when `cap <= 0`.
    #[must_use]
    fn unmetered() -> Self {
        Self {
            allowed: true,
            quota_remaining: 0,
            quota_limit: 0,
            reset_at_unix_ms: 0,
            retry_after_ms: 0,
            deny_reason: None,
        }
    }
}

/// One key's live window state.
#[derive(Clone, Copy, Debug)]
struct WindowState {
    /// Wall-clock millis when the current window opened.
    window_start_ms: i64,
    /// Requests already consumed in the current window.
    used: i32,
}

/// Fixed-width key for one `(consumer_ura, ability)` quota window.
///
/// The store never retains attacker-controlled key strings. It hashes
/// length-prefixed byte slices into a 32-byte digest, so memory growth is
/// bounded by [`MAX_QUOTA_TRACKED_KEYS`] rather than by caller-chosen
/// string length.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct QuotaKey([u8; 32]);

impl QuotaKey {
    fn new(consumer_ura: &str, ability: &str) -> Result<Self, QuotaDenyReason> {
        if consumer_ura.len() > MAX_QUOTA_CONSUMER_URA_BYTES
            || ability.len() > MAX_QUOTA_ABILITY_NAME_BYTES
        {
            return Err(QuotaDenyReason::KeyTooLarge);
        }

        let mut hasher = Sha256::new();
        hasher.update((consumer_ura.len() as u64).to_le_bytes());
        hasher.update(consumer_ura.as_bytes());
        hasher.update((ability.len() as u64).to_le_bytes());
        hasher.update(ability.as_bytes());
        Ok(Self(hasher.finalize().into()))
    }
}

/// Reloadable quota policy plus the daemon-local counter store.
///
/// Invariants:
/// 1. `policy == None` means quota is fully disabled and no new
///    counters are allocated.
/// 2. The counter map is bounded by [`MAX_QUOTA_TRACKED_KEYS`].
/// 3. Policy replacement is visible to the next admission check; a
///    daemon restart is not required after SIGHUP.
#[derive(Clone, Debug)]
pub struct SharedUsageQuotaGate {
    policy: Arc<RwLock<Option<QuotaConfig>>>,
    store: SharedUsageQuotaStore,
}

impl SharedUsageQuotaGate {
    /// Construct a disabled gate. A later SIGHUP may publish
    /// `Some(QuotaConfig)` into it.
    #[must_use]
    pub fn disabled() -> Self {
        Self::from_policy(None)
    }

    /// Construct a gate from the boot-time config policy.
    #[must_use]
    pub fn from_policy(policy: Option<QuotaConfig>) -> Self {
        Self {
            policy: Arc::new(RwLock::new(policy)),
            store: SharedUsageQuotaStore::new(),
        }
    }

    /// Replace the live policy after a daemon-config reload. Replacing
    /// with `None` disables quota and clears retained windows so turning
    /// the feature off immediately frees memory.
    pub fn replace_policy(&self, next: Option<QuotaConfig>) {
        let disabled = next.is_none();
        match self.policy.write() {
            Ok(mut guard) => *guard = next,
            Err(poisoned) => *poisoned.into_inner() = next,
        }
        if disabled {
            self.store.clear();
        }
    }

    /// Snapshot the live policy. Observability/test helper; admission
    /// paths should call [`Self::check_and_record`] so they only copy
    /// the scalar fields needed for this request, not the whole
    /// per-consumer override table.
    #[must_use]
    pub fn policy(&self) -> Option<QuotaConfig> {
        match self.policy.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Meter one already-admitted `(consumer_ura, ability)` pair.
    ///
    /// Returns `None` when quota is disabled or the caller's effective
    /// cap is `<= 0` (unmetered). Returns `Some(decision)` only when
    /// the call is actively metered.
    pub fn check_and_record(
        &self,
        consumer_ura: &str,
        ability: &str,
        now_ms: i64,
    ) -> Option<QuotaDecision> {
        let (cap, window_ms) = match self.policy.read() {
            Ok(guard) => {
                let policy = guard.as_ref()?;
                (policy.cap_for(consumer_ura), policy.window_ms())
            }
            Err(poisoned) => {
                let guard = poisoned.into_inner();
                let policy = guard.as_ref()?;
                (policy.cap_for(consumer_ura), policy.window_ms())
            }
        };
        if cap <= 0 {
            return None;
        }
        Some(
            self.store
                .check_and_record(consumer_ura, ability, cap, window_ms, now_ms),
        )
    }

    /// Number of currently retained `(consumer, ability)` windows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.store.len()
    }

    /// Whether no metered windows are currently retained.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }
}

/// Daemon-side usage counter shared across every InvokeRequest. Cheap
/// to clone; production owns it through [`SharedUsageQuotaGate`].
#[derive(Clone, Debug, Default)]
pub struct SharedUsageQuotaStore {
    inner: Arc<Mutex<HashMap<QuotaKey, WindowState>>>,
}

impl SharedUsageQuotaStore {
    /// Build an empty counter store. The window width is supplied per
    /// check from the live quota policy so SIGHUP can change
    /// `[daemon.quota].window_ms` without rebuilding this store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Record one admitted invocation for `(consumer_ura, ability)`
    /// against `cap` requests-per-window.
    pub fn check_and_record(
        &self,
        consumer_ura: &str,
        ability: &str,
        cap: i32,
        window_ms: i64,
        now_ms: i64,
    ) -> QuotaDecision {
        if cap <= 0 {
            return QuotaDecision::unmetered();
        }
        let window_ms = window_ms.max(1);
        let key = match QuotaKey::new(consumer_ura, ability) {
            Ok(key) => key,
            Err(reason) => {
                return QuotaDecision {
                    allowed: false,
                    quota_remaining: 0,
                    quota_limit: cap,
                    reset_at_unix_ms: now_ms.saturating_add(window_ms),
                    retry_after_ms: 0,
                    deny_reason: Some(reason),
                };
            }
        };

        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if !guard.contains_key(&key) && guard.len() >= MAX_QUOTA_TRACKED_KEYS {
            prune_expired_windows(&mut guard, now_ms, window_ms);
            if guard.len() >= MAX_QUOTA_TRACKED_KEYS {
                return QuotaDecision {
                    allowed: false,
                    quota_remaining: 0,
                    quota_limit: cap,
                    reset_at_unix_ms: now_ms.saturating_add(window_ms),
                    retry_after_ms: i32::try_from(window_ms).unwrap_or(i32::MAX),
                    deny_reason: Some(QuotaDenyReason::StoreSaturated),
                };
            }
        }

        let state = guard.entry(key).or_insert(WindowState {
            window_start_ms: now_ms,
            used: 0,
        });

        if now_ms.saturating_sub(state.window_start_ms) >= window_ms {
            state.window_start_ms = now_ms;
            state.used = 0;
        }

        let reset_at_unix_ms = state.window_start_ms.saturating_add(window_ms);
        if state.used >= cap {
            let retry_after_ms = reset_at_unix_ms.saturating_sub(now_ms).max(0);
            return QuotaDecision {
                allowed: false,
                quota_remaining: 0,
                quota_limit: cap,
                reset_at_unix_ms,
                retry_after_ms: i32::try_from(retry_after_ms).unwrap_or(i32::MAX),
                deny_reason: Some(QuotaDenyReason::BudgetExhausted),
            };
        }

        state.used += 1;
        QuotaDecision {
            allowed: true,
            quota_remaining: cap - state.used,
            quota_limit: cap,
            reset_at_unix_ms,
            retry_after_ms: 0,
            deny_reason: None,
        }
    }

    /// Drop every retained counter.
    pub fn clear(&self) {
        match self.inner.lock() {
            Ok(mut guard) => guard.clear(),
            Err(poisoned) => poisoned.into_inner().clear(),
        }
    }

    /// Number of keys currently tracked.
    #[must_use]
    pub fn len(&self) -> usize {
        match self.inner.lock() {
            Ok(guard) => guard.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        }
    }

    /// Whether the store tracks zero keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn prune_expired_windows(guard: &mut HashMap<QuotaKey, WindowState>, now_ms: i64, window_ms: i64) {
    guard.retain(|_, state| now_ms.saturating_sub(state.window_start_ms) < window_ms);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn unmetered_cap_always_allows_and_records_nothing() {
        let store = SharedUsageQuotaStore::new();
        let d = store.check_and_record("alice", "echo", 0, 60_000, 1_000);
        assert!(d.allowed);
        assert_eq!(d.quota_limit, 0);
        assert!(store.is_empty(), "unmetered keys must not allocate state");
    }

    #[test]
    fn decrements_until_exhausted_then_throttles() {
        let store = SharedUsageQuotaStore::new();
        let first = store.check_and_record("alice", "echo", 2, 10_000, 1_000);
        assert!(first.allowed);
        assert_eq!(first.quota_remaining, 1);
        assert_eq!(first.reset_at_unix_ms, 11_000);

        let second = store.check_and_record("alice", "echo", 2, 10_000, 1_500);
        assert!(second.allowed);
        assert_eq!(second.quota_remaining, 0);

        let third = store.check_and_record("alice", "echo", 2, 10_000, 2_000);
        assert!(!third.allowed, "third call exceeds cap of 2");
        assert_eq!(third.quota_remaining, 0);
        assert_eq!(third.deny_reason, Some(QuotaDenyReason::BudgetExhausted));
        assert_eq!(third.retry_after_ms, 9_000, "time left until window reset");
    }

    #[test]
    fn window_reset_restores_budget() {
        let store = SharedUsageQuotaStore::new();
        store.check_and_record("alice", "echo", 1, 10_000, 1_000);
        let throttled = store.check_and_record("alice", "echo", 1, 10_000, 2_000);
        assert!(!throttled.allowed);

        let after = store.check_and_record("alice", "echo", 1, 10_000, 11_000);
        assert!(after.allowed, "window elapsed -> budget reset");
        assert_eq!(after.quota_remaining, 0);
        assert_eq!(after.reset_at_unix_ms, 21_000);
    }

    #[test]
    fn distinct_keys_have_independent_budgets() {
        let store = SharedUsageQuotaStore::new();
        assert!(
            store
                .check_and_record("alice", "echo", 1, 10_000, 1_000)
                .allowed
        );
        assert!(
            !store
                .check_and_record("alice", "echo", 1, 10_000, 1_100)
                .allowed
        );
        assert!(
            store
                .check_and_record("alice", "stat", 1, 10_000, 1_200)
                .allowed,
            "a different ability has its own budget"
        );
        assert!(
            store
                .check_and_record("bob", "echo", 1, 10_000, 1_300)
                .allowed
        );
        assert_eq!(store.len(), 3);
    }

    #[test]
    fn throttle_does_not_consume_extra_budget() {
        let store = SharedUsageQuotaStore::new();
        store.check_and_record("alice", "echo", 1, 10_000, 1_000);
        for t in 2_000..2_005 {
            assert!(
                !store
                    .check_and_record("alice", "echo", 1, 10_000, t)
                    .allowed
            );
        }
        let after = store.check_and_record("alice", "echo", 1, 10_000, 11_001);
        assert!(after.allowed, "budget recovers exactly one slot per window");
    }

    #[test]
    fn shared_store_serialises_concurrent_callers() {
        let store = SharedUsageQuotaStore::new();
        let handle = std::thread::spawn({
            let store = store.clone();
            move || store.check_and_record("alice", "echo", 1, 10_000, 1_000)
        });
        assert!(handle.join().unwrap().allowed);
        let second = store.check_and_record("alice", "echo", 1, 10_000, 1_100);
        assert!(!second.allowed, "shared budget exhausted across threads");
    }

    #[test]
    fn saturated_store_prunes_expired_windows_before_failing_closed() {
        let store = SharedUsageQuotaStore::new();
        for i in 0..MAX_QUOTA_TRACKED_KEYS {
            let consumer = format!("consumer-{i}");
            assert!(
                store
                    .check_and_record(&consumer, "echo", 1, 10_000, 1_000)
                    .allowed
            );
        }
        assert_eq!(store.len(), MAX_QUOTA_TRACKED_KEYS);

        let after_prune = store.check_and_record("fresh", "echo", 1, 10_000, 20_000);
        assert!(
            after_prune.allowed,
            "expired windows must be pruned before the hard capacity bound"
        );
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn saturated_store_fails_closed_when_no_window_can_be_pruned() {
        let store = SharedUsageQuotaStore::new();
        for i in 0..MAX_QUOTA_TRACKED_KEYS {
            let consumer = format!("consumer-{i}");
            store.check_and_record(&consumer, "echo", 1, 10_000, 1_000);
        }

        let denied = store.check_and_record("fresh", "echo", 1, 10_000, 2_000);
        assert!(!denied.allowed);
        assert_eq!(denied.deny_reason, Some(QuotaDenyReason::StoreSaturated));
        assert_eq!(
            store.len(),
            MAX_QUOTA_TRACKED_KEYS,
            "capacity reject must not allocate the fresh key"
        );
    }

    #[test]
    fn oversized_key_fails_without_allocating_window() {
        let store = SharedUsageQuotaStore::new();
        let ability = "a".repeat(MAX_QUOTA_ABILITY_NAME_BYTES + 1);

        let denied = store.check_and_record("alice", &ability, 1, 10_000, 1_000);

        assert!(!denied.allowed);
        assert_eq!(denied.deny_reason, Some(QuotaDenyReason::KeyTooLarge));
        assert!(
            store.is_empty(),
            "oversized quota key must not allocate retained state"
        );
    }

    #[test]
    fn gate_replaces_policy_without_rebuilding_store() {
        let gate =
            SharedUsageQuotaGate::from_policy(Some(QuotaConfig::new(1, 10_000, BTreeMap::new())));
        assert!(
            gate.check_and_record("alice", "echo", 1_000)
                .expect("metered")
                .allowed
        );
        assert!(
            !gate
                .check_and_record("alice", "echo", 1_500)
                .expect("metered")
                .allowed
        );

        gate.replace_policy(Some(QuotaConfig::new(2, 10_000, BTreeMap::new())));
        assert!(
            gate.check_and_record("alice", "echo", 2_000)
                .expect("still metered")
                .allowed,
            "raising the cap via SIGHUP must affect the next admission"
        );

        gate.replace_policy(None);
        assert_eq!(
            gate.check_and_record("alice", "echo", 2_500),
            None,
            "disabling quota via SIGHUP must stop metering immediately"
        );
        assert!(gate.is_empty(), "disabling quota clears retained windows");
    }
}
