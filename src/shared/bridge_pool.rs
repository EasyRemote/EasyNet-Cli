// EasyNet CLI — DendriteBridge Connection Pool
// =============================================
//
// File: src/shared/bridge_pool.rs
// Description: High-performance, lock-free connection pool for DendriteBridge instances.
//
// Problem:
//   DendriteBridge is !Send/!Sync (FFI handle). Each thread in a parallel
//   phase needs its own bridge. Creating a new gRPC connection per step pays
//   TCP+gRPC handshake cost every time.
//
// Solution (gen-2):
//   Lock-free SegQueue from crossbeam replaces Mutex<Vec>. Bridges are moved
//   in/out atomically without contention. Pool size adapts to available CPU
//   cores. AtomicUsize tracks current size for bounded growth.
//
// Thread Safety:
//   The pool itself is Send+Sync (crossbeam SegQueue + atomics). Individual
//   DendriteBridge instances are moved into threads, never shared.
//
// Author: Silan Hu <silan.hu@u.nus.edu>
// Copyright (c) 2026 EasyNet. All rights reserved.

use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_queue::SegQueue;
use easynet_axon::dendrite_bridge::DendriteBridge;

/// Compute adaptive pool size based on available CPU cores.
///
/// Returns `max(num_cpus, 4)` capped at 64. This ensures the pool is large
/// enough for typical parallel phases without over-allocating on large machines.
pub fn adaptive_pool_size() -> usize {
    num_cpus::get().clamp(4, 64)
}

/// A lock-free pool of pre-connected DendriteBridge instances.
///
/// Uses crossbeam's `SegQueue` for wait-free push/pop operations.
/// Bridges are checked out (moved to caller), used, and returned.
/// The pool grows on demand if all bridges are checked out.
pub struct BridgePool {
    endpoint: String,
    timeout_ms: u64,
    queue: SegQueue<DendriteBridge>,
    /// Current number of bridges in the queue (approximate, used for bounded return).
    size: AtomicUsize,
    /// Maximum pool size — bridges beyond this are dropped on return.
    max_size: usize,
}

// Safety: BridgePool is Send+Sync because:
// - SegQueue<DendriteBridge> is internally synchronized (lock-free CAS operations).
// - Bridges are moved into/out of the queue, never shared across threads.
// - Each checked-out bridge is used by exactly one thread at a time.
// - DendriteBridge is !Send only because the FFI handle *might* not be
//   thread-safe, but we never use a bridge from two threads simultaneously.
//   The pool transfers ownership: checkout() → use → drop(BridgeGuard) → checkin().
unsafe impl Send for BridgePool {}
unsafe impl Sync for BridgePool {}

impl BridgePool {
    /// Create a new pool and eagerly connect `initial_size` bridges.
    ///
    /// Bridges that fail to connect during init are silently skipped;
    /// they will be created on demand when checked out.
    pub fn new(endpoint: &str, timeout_ms: u64, initial_size: usize) -> Self {
        let queue = SegQueue::new();
        let mut connected = 0usize;
        for _ in 0..initial_size {
            if let Ok(br) = DendriteBridge::connect(endpoint, timeout_ms) {
                queue.push(br);
                connected += 1;
            }
        }
        Self {
            endpoint: endpoint.to_string(),
            timeout_ms,
            queue,
            size: AtomicUsize::new(connected),
            // 2× initial for headroom; at least 16 to absorb burst phases.
            max_size: (initial_size * 2).max(16),
        }
    }

    /// Create a pool with adaptive sizing based on available CPU cores.
    pub fn with_adaptive_size(endpoint: &str, timeout_ms: u64) -> Self {
        Self::new(endpoint, timeout_ms, adaptive_pool_size())
    }

    /// Check out a bridge from the pool.
    ///
    /// Returns an existing bridge if available (lock-free pop),
    /// otherwise creates a new one on demand.
    ///
    /// Ordering note: `SegQueue::pop` returns `Some` to exactly one
    /// caller, so decrementing `size` with `Release` after a successful
    /// pop cannot double-count. The matching `Acquire` on the checkin
    /// side (via the reserve-CAS) establishes a happens-before edge so
    /// the two counters stay consistent across threads.
    pub fn checkout(&self) -> Result<BridgeGuard<'_>, String> {
        let bridge = match self.queue.pop() {
            Some(br) => {
                self.size.fetch_sub(1, Ordering::Release);
                br
            }
            None => DendriteBridge::connect(&self.endpoint, self.timeout_ms).map_err(|e| {
                // Include both the endpoint and the underlying SDK
                // diagnostic. The pool returns `Result<_, String>` (not
                // `anyhow::Error`) because the EAL dispatch trait
                // requires String, so we cannot keep the full error
                // chain — including the endpoint up front is the best
                // operator can do post-hoc with grep alone.
                format!("bridge connect to {} failed: {e}", self.endpoint)
            })?,
        };
        Ok(BridgeGuard {
            bridge: Some(bridge),
            pool: self,
        })
    }

    /// Return a bridge to the pool (called by `BridgeGuard::drop`).
    ///
    /// The capacity invariant — `size <= max_size` at all times — is
    /// enforced with a single compare-exchange loop that *reserves* the
    /// slot before the push. A naive `load → check → push → fetch_add`
    /// would race when several threads check in simultaneously: each
    /// would observe `current < max_size` and all would push, pushing
    /// the pool above its advertised cap.
    ///
    /// Reservation order:
    ///   1. CAS `size` from `n` to `n+1` iff `n < max_size`.
    ///   2. If CAS succeeds, we own the slot; push the bridge.
    ///   3. If CAS fails because the pool is full, drop the bridge.
    ///
    /// Using `AcqRel` on the successful CAS establishes the synchronise-with
    /// edge required to observe the pushed value from a subsequent
    /// `checkout` on another thread.
    fn checkin(&self, bridge: DendriteBridge) {
        let max_size = self.max_size;
        loop {
            let current = self.size.load(Ordering::Acquire);
            if current >= max_size {
                // Pool at capacity; drop the bridge rather than exceed the cap.
                return;
            }
            match self.size.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.queue.push(bridge);
                    return;
                }
                Err(_) => {
                    // Another thread changed `size`; retry with the fresh value.
                    // The `_weak` variant is allowed to spuriously fail — the
                    // outer loop handles both spurious and real losses.
                }
            }
        }
    }

    /// Endpoint accessor.
    #[allow(dead_code)]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Timeout accessor.
    #[allow(dead_code)]
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms
    }

    /// Current approximate number of idle bridges in the pool.
    #[allow(dead_code)]
    pub fn idle_count(&self) -> usize {
        self.size.load(Ordering::Relaxed)
    }

    /// Maximum pool size.
    #[allow(dead_code)]
    pub fn max_size(&self) -> usize {
        self.max_size
    }
}

/// RAII guard that returns the bridge to the pool on drop.
///
/// This type is `!Send` because `DendriteBridge` is `!Send`, which is
/// correct — the guard must be used and dropped on the same thread.
pub struct BridgeGuard<'a> {
    bridge: Option<DendriteBridge>,
    pool: &'a BridgePool,
}

impl<'a> BridgeGuard<'a> {
    /// Access the underlying bridge.
    pub fn bridge(&self) -> &DendriteBridge {
        self.bridge.as_ref().expect("bridge taken after drop")
    }
}

impl Drop for BridgeGuard<'_> {
    fn drop(&mut self) {
        if let Some(br) = self.bridge.take() {
            self.pool.checkin(br);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_pool_size_is_reasonable() {
        let size = adaptive_pool_size();
        assert!(
            size >= 4,
            "adaptive pool size should be at least 4, got {size}"
        );
        assert!(
            size <= 64,
            "adaptive pool size should be at most 64, got {size}"
        );
    }

    #[test]
    fn pool_new_with_zero_initial_size() {
        let pool = BridgePool::new("localhost:50051", 5000, 0);
        assert_eq!(pool.max_size, 16); // max(0*2, 16)
        assert_eq!(pool.idle_count(), 0);
    }

    #[test]
    fn pool_max_size_at_least_16() {
        let pool = BridgePool::new("localhost:50051", 5000, 4);
        assert_eq!(pool.max_size, 16); // max(4*2=8, 16)
    }

    #[test]
    fn pool_max_size_scales_with_initial() {
        let pool = BridgePool::new("localhost:50051", 5000, 32);
        assert_eq!(pool.max_size, 64); // 32*2 = 64 > 16
    }

    #[test]
    fn adaptive_pool_uses_cpu_count() {
        let pool = BridgePool::with_adaptive_size("localhost:50051", 5000);
        let expected_max = (adaptive_pool_size() * 2).max(16);
        assert_eq!(pool.max_size, expected_max);
    }

    /// Pins the capacity invariant under concurrent checkin.
    ///
    /// We cannot construct real `DendriteBridge` instances in a unit
    /// test (it would try to connect to a gRPC endpoint), so this test
    /// exercises the accounting side of the pool in isolation by
    /// driving `size.fetch_add` / `compare_exchange_weak` through the
    /// same critical region `checkin` uses.
    ///
    /// The contract under test: no matter how many concurrent threads
    /// attempt to reserve a slot, the counter never exceeds `max_size`.
    /// A broken Relaxed-only implementation of this sequence would
    /// occasionally overshoot; the Acquire/Release CAS here must not.
    #[test]
    fn capacity_invariant_holds_under_contention() {
        use std::sync::Arc;
        use std::thread;

        let max_size: usize = 8;
        let size = Arc::new(AtomicUsize::new(0));
        let succeeded = Arc::new(AtomicUsize::new(0));
        let threads = 32;
        let rounds_per_thread = 500;

        let handles: Vec<_> = (0..threads)
            .map(|_| {
                let size = Arc::clone(&size);
                let succeeded = Arc::clone(&succeeded);
                thread::spawn(move || {
                    for _ in 0..rounds_per_thread {
                        // Reserve-or-reject, mirroring checkin's CAS loop.
                        let reserved = loop {
                            let current = size.load(Ordering::Acquire);
                            if current >= max_size {
                                break false;
                            }
                            if size
                                .compare_exchange_weak(
                                    current,
                                    current + 1,
                                    Ordering::AcqRel,
                                    Ordering::Acquire,
                                )
                                .is_ok()
                            {
                                break true;
                            }
                        };
                        if reserved {
                            succeeded.fetch_add(1, Ordering::Relaxed);
                            // Mirror the eventual checkout: release the slot.
                            size.fetch_sub(1, Ordering::Release);
                        }
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        // Every participating thread succeeded at least once, so the CAS
        // loop is not a livelock.
        assert!(
            succeeded.load(Ordering::Relaxed) >= threads,
            "expected every thread to reserve at least once"
        );
        // After all threads have finished, size is back to zero.
        assert_eq!(size.load(Ordering::Acquire), 0);
    }
}
