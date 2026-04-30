// EasyNet CLI — nonce replay store (admission §5.2 step 4)
// =========================================================
//
// File: src/services/nonce_replay_store.rs
//
// Thread-safe wrapper around `easynet_axon::invocation::admission::
// NonceReplayStore` so the daemon's `AdmissionFacade` can share one
// store across every concurrently-served gRPC RPC.
//
// Why a wrapper, not the SDK type directly
// ----------------------------------------
// The SDK's `NonceReplayStore` is `&mut self` for `check_and_record`
// — single-threaded by design. The daemon serves one InvokeRequest
// per tonic worker concurrently, so the store needs interior
// mutability + Send + Sync. `std::sync::Mutex` is sufficient: the
// admission gate is the only writer and writes are O(1) per call,
// so the lock surface is too narrow for contention to matter at
// realistic invoke rates.
//
// Per DEC-011 the store is in-memory only for RFC-003. The SDK's
// time-wheel implementation already provides bounded GC, monotonic-time
// clamping, and a 7-day default window — strictly more than DEC-011
// asks for. Re-using it (rather than rolling a fresh map) keeps the
// six-language admission semantics aligned, which matters for the
// PR-10 canary's cross-runtime conformance check.
//
// Author: Silan.Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::{Arc, Mutex};

use easynet_axon::invocation::admission::NonceReplayStore as AxonReplayStore;

/// Daemon-side replay store shared across every InvokeRequest. Cheap
/// to clone (it's an `Arc<Mutex<…>>`); production threads one shared
/// instance through `AdmissionFacade` at boot.
#[derive(Clone, Debug)]
pub struct SharedNonceReplayStore {
    inner: Arc<Mutex<AxonReplayStore>>,
}

impl SharedNonceReplayStore {
    /// Build a store with the SDK's default 7-day dedup window.
    /// Production callers want this; tests typically call
    /// [`Self::with_window_ms`] for tighter windows.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(AxonReplayStore::new())),
        }
    }

    /// Build a store with a custom dedup window. Width semantics
    /// match the SDK exactly — see
    /// [`AxonReplayStore::with_window_ms`].
    #[must_use]
    pub fn with_window_ms(window_ms: i64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AxonReplayStore::with_window_ms(window_ms))),
        }
    }

    /// Borrow the underlying `AxonReplayStore` for the duration of
    /// one admission check. Held only for the O(1) check-and-record
    /// write — the lock surface is too narrow for contention to
    /// matter on any realistic invoke rate.
    ///
    /// A poisoned lock means a previous admission panicked while
    /// holding it. We recover via `into_inner`-style poison handling
    /// (`into_inner` on a `PoisonError`) so a single buggy panic does
    /// not wedge the daemon's admission gate forever — the admission
    /// path is only `&mut self` for the O(1) hashmap insert, and the
    /// SDK store's invariants are upheld even after a panic.
    pub fn with_inner<R>(&self, f: impl FnOnce(&mut AxonReplayStore) -> R) -> R {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard)
    }

    /// Number of entries currently held. Test/observability only.
    #[must_use]
    pub fn len(&self) -> usize {
        self.with_inner(|s| s.len())
    }

    /// Whether the store has zero live entries. Test-only convenience.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.with_inner(|s| s.is_empty())
    }
}

impl Default for SharedNonceReplayStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use easynet_axon::invocation::admission::REASON_NONCE_REPLAY;

    #[test]
    fn fresh_nonce_admitted_replay_rejected() {
        let store = SharedNonceReplayStore::new();
        let nonce = [1u8; 16];

        store
            .with_inner(|s| s.check_and_record("caller", "ability", nonce, 1_000))
            .expect("first observation accepted");

        let err = store
            .with_inner(|s| s.check_and_record("caller", "ability", nonce, 2_000))
            .expect_err("second observation must be rejected");
        assert_eq!(err.reason, REASON_NONCE_REPLAY);
    }

    #[test]
    fn shared_store_serialises_concurrent_callers() {
        // The daemon runs one shared store across many tonic workers.
        // Verifying the Mutex actually serialises observers — a fresh
        // nonce admitted on thread A must reject on thread B.
        let store = SharedNonceReplayStore::new();
        let nonce = [42u8; 16];

        let handle_a = std::thread::spawn({
            let store = store.clone();
            move || store.with_inner(|s| s.check_and_record("caller", "ability", nonce, 1_000))
        });
        handle_a.join().unwrap().expect("first thread accepted");

        let err = store
            .with_inner(|s| s.check_and_record("caller", "ability", nonce, 2_000))
            .expect_err("second thread must reject");
        assert_eq!(err.reason, REASON_NONCE_REPLAY);
    }

    #[test]
    fn distinct_callers_share_store_without_collision() {
        let store = SharedNonceReplayStore::new();
        let nonce = [7u8; 16];

        store
            .with_inner(|s| s.check_and_record("alice", "ability", nonce, 1_000))
            .expect("alice accepted");
        store
            .with_inner(|s| s.check_and_record("bob", "ability", nonce, 2_000))
            .expect("bob accepted — different caller, same nonce ok");
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn len_and_is_empty_track_writes() {
        let store = SharedNonceReplayStore::new();
        assert!(store.is_empty());
        store
            .with_inner(|s| s.check_and_record("caller", "ability", [1u8; 16], 1_000))
            .expect("accepted");
        assert!(!store.is_empty());
        assert_eq!(store.len(), 1);
    }
}
